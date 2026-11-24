use better_explorer::model::wide;
use std::{
    ffi::{OsStr, OsString},
    io,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
        HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0,
    },
    System::{
        Registry::{
            RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegDeleteValueW, RegOpenKeyExW,
            RegQueryInfoKeyW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
            HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_BINARY, REG_SZ,
        },
        Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
    },
    UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_FLUSHNOWAIT, SHCNF_IDLIST},
};

const VERB: &str = "BetterExplorer.Open";
const OWNER: &str = "BetterExplorerExecutable";
const PREVIOUS: &str = "BetterExplorerPreviousDefault";
const LABEL: &str = "Open in BetterExplorer";
const CLASSES: [&str; 2] = [r"Directory\shell", r"Drive\shell"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Inactive,
    Active,
    Partial,
    Conflict,
}

pub fn status() -> io::Result<Status> {
    Registration::new(&Scope::live(), &std::env::current_exe()?)?.status()
}

pub fn set_enabled(enabled: bool) -> io::Result<()> {
    Registration::new(&Scope::live(), &std::env::current_exe()?)?.change(enabled)
}

pub fn cli(arg: &OsStr, args: impl Iterator<Item = OsString>) -> Option<i32> {
    let enabled = match arg.to_str()? {
        "--folder-manager-on" => Some(true),
        "--folder-manager-off" => Some(false),
        "--folder-manager-state" => None,
        _ => return None,
    };
    let run = || -> io::Result<i32> {
        let mut owner = std::env::current_exe()?;
        let mut scope = Scope::live();
        let mut args = args;
        while let Some(option) = args.next() {
            let value = args.next().ok_or_else(invalid_options)?;
            match option.to_str() {
                Some("--owner") => owner = PathBuf::from(value),
                Some("--test-root") => scope = Scope::test(Path::new(&value))?,
                _ => return Err(invalid_options()),
            }
        }
        let registration = Registration::new(&scope, &owner)?;
        if let Some(enabled) = enabled {
            registration.change(enabled)?;
            return Ok(0);
        }
        Ok(match registration.status()? {
            Status::Active => 0,
            Status::Inactive => 2,
            Status::Partial => 3,
            Status::Conflict => 4,
        })
    };
    Some(match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    })
}

fn invalid_options() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "Invalid folder manager options.",
    )
}

struct Scope {
    root: String,
    testing: bool,
}

impl Scope {
    fn live() -> Self {
        Self {
            root: r"Software\Classes".into(),
            testing: false,
        }
    }

    fn test(path: &Path) -> io::Result<Self> {
        let path = path.canonicalize()?;
        let temp = std::env::temp_dir().canonicalize()?;
        if path.parent() != Some(temp.as_path()) || !path.is_dir() {
            return Err(invalid_options());
        }
        Ok(Self {
            root: format!(
                r"Software\BetterExplorer.SetupTests\{}",
                path.file_name().unwrap().to_string_lossy()
            ),
            testing: true,
        })
    }
}

struct Registration<'a> {
    scope: &'a Scope,
    keys: [String; 2],
    owner: Value,
    command: Value,
}

impl<'a> Registration<'a> {
    fn new(scope: &'a Scope, owner: &Path) -> io::Result<Self> {
        if !owner.is_absolute() || owner.as_os_str().encode_wide().any(|c| c == b'"' as u16) {
            return Err(invalid_options());
        }
        let mut command = OsString::from("\"");
        command.push(owner.as_os_str());
        // The suffix keeps a drive root's backslash away from the closing quote.
        command.push(r#"" --open-folder "%1\.""#);
        Ok(Self {
            scope,
            keys: CLASSES.map(|class| format!(r"{}\{class}", scope.root)),
            owner: Value::text(owner),
            command: Value::text(command),
        })
    }

    fn snapshots(&self) -> io::Result<[Snapshot; 2]> {
        Ok([self.snapshot(&self.keys[0])?, self.snapshot(&self.keys[1])?])
    }

    fn snapshot(&self, key: &str) -> io::Result<Snapshot> {
        let default = read(key, "")?;
        let container = Container::read(key)?;
        let verb = verb_key(key);
        let command_key = command_key(key);
        let owner = read(&verb, OWNER)?;
        let label = read(&verb, "")?;
        let previous = read(&verb, PREVIOUS)?;
        let command = read(&command_key, "")?;
        let active = default.as_ref().is_some_and(|value| value.text_eq(VERB));
        let verb_counts = counts(&verb)?;
        let command_counts = counts(&command_key)?;
        let machine_verb = !self.scope.testing && counts_in(HKEY_LOCAL_MACHINE, &verb)?.is_some();
        let owned = owner.as_ref() == Some(&self.owner)
            && label.as_ref().is_none_or(|value| value.text_eq(LABEL))
            && command.as_ref().is_none_or(|value| value == &self.command)
            && previous.as_ref().map_or(!active, Value::valid_backup)
            && verb_counts
                == Some((
                    u32::from(command_counts.is_some()),
                    1 + u32::from(previous.is_some()) + u32::from(label.is_some()),
                ))
            && command_counts.is_none_or(|value| value == (0, u32::from(command.is_some())));
        let foreign = !owned
            && (verb_counts.is_some_and(|counts| counts != (0, 0)) || active || machine_verb);
        Ok(Snapshot {
            container,
            default,
            owner,
            label,
            previous,
            command,
            owned,
            active,
            foreign,
        })
    }

    fn status(&self) -> io::Result<Status> {
        let _lock = Lock::acquire()?;
        let states = self.snapshots()?;
        Ok(
            if states
                .iter()
                .all(|state| state.owned && state.active && state.command.is_some())
            {
                Status::Active
            } else if states.iter().any(|state| state.foreign) {
                Status::Conflict
            } else if states.iter().any(|state| state.owned) {
                Status::Partial
            } else {
                Status::Inactive
            },
        )
    }

    fn change(&self, enabled: bool) -> io::Result<()> {
        let _lock = Lock::acquire()?;
        let old = self.snapshots()?;
        if enabled && old.iter().any(|state| state.foreign) {
            return Err(io::Error::other(
                "Another BetterExplorer copy or an existing shell registration uses this folder command.",
            ));
        }
        if !enabled
            && old.iter().any(|state| {
                state.foreign && state.active && state.owner.as_ref() == Some(&self.owner)
            })
        {
            return Err(io::Error::other(
                "The BetterExplorer folder command was changed outside the app and needs to be restored before it can be removed.",
            ));
        }
        if !enabled && !old.iter().any(|state| state.owned) {
            return Ok(());
        }
        let mut changed = false;
        for (index, (key, state)) in self.keys.iter().zip(&old).enumerate() {
            let result = if enabled {
                let previous = if state.owned && state.active {
                    state.previous.clone().unwrap()
                } else {
                    let container = state
                        .previous
                        .as_ref()
                        .map_or(state.container, Value::container);
                    Value::backup(state.default.as_ref(), container)
                };
                changed = true;
                self.install(key, &previous)
            } else if state.owned {
                changed = true;
                self.remove(key, state)
            } else {
                continue;
            };
            if let Err(error) = result {
                let mut recovery = Ok(());
                for (key, state) in self.keys[..=index].iter().zip(&old[..=index]).rev() {
                    if (enabled || state.owned) && !state.foreign {
                        if let Err(error) = state.restore(key) {
                            recovery = Err(error);
                        }
                    }
                }
                self.notify();
                return Err(match recovery {
                    Ok(()) => error,
                    Err(recovery) => io::Error::other(format!("{error} Rollback: {recovery}")),
                });
            }
        }
        if changed {
            self.notify();
        }
        Ok(())
    }

    fn install(&self, key: &str, previous: &Value) -> io::Result<()> {
        let verb = verb_key(key);
        write(&verb, OWNER, Some(&self.owner))?;
        write(&verb, PREVIOUS, Some(previous))?;
        write(&verb, "", Some(&Value::text(LABEL)))?;
        write(&command_key(key), "", Some(&self.command))?;
        write(key, "", Some(&Value::text(VERB)))
    }

    fn remove(&self, key: &str, state: &Snapshot) -> io::Result<()> {
        if state.active {
            let previous = state.previous.as_ref().unwrap().original();
            write(key, "", previous.as_ref())?;
        }
        clear_verb(key)?;
        if let Some(previous) = &state.previous {
            previous.container().cleanup(key)?;
        }
        Ok(())
    }

    fn notify(&self) {
        if !self.scope.testing {
            unsafe {
                SHChangeNotify(
                    SHCNE_ASSOCCHANGED as i32,
                    SHCNF_IDLIST | SHCNF_FLUSHNOWAIT,
                    null(),
                    null(),
                )
            }
        }
    }
}

fn verb_key(key: &str) -> String {
    format!(r"{key}\{VERB}")
}

fn command_key(key: &str) -> String {
    format!(r"{key}\{VERB}\command")
}

fn clear_verb(key: &str) -> io::Result<()> {
    let command = command_key(key);
    let verb = verb_key(key);
    write(&command, "", None)?;
    delete_empty_key(&command)?;
    for name in ["", PREVIOUS, OWNER] {
        write(&verb, name, None)?;
    }
    delete_empty_key(&verb)
}

struct Snapshot {
    container: Container,
    default: Option<Value>,
    owner: Option<Value>,
    label: Option<Value>,
    previous: Option<Value>,
    command: Option<Value>,
    owned: bool,
    active: bool,
    foreign: bool,
}

impl Snapshot {
    fn restore(&self, key: &str) -> io::Result<()> {
        if !self.owned {
            write(key, "", self.default.as_ref())?;
            clear_verb(key)?;
            return self.container.cleanup(key);
        }
        let verb = verb_key(key);
        write(&verb, PREVIOUS, self.previous.as_ref())?;
        write(&verb, OWNER, self.owner.as_ref())?;
        write(&verb, "", self.label.as_ref())?;
        write(&command_key(key), "", self.command.as_ref())?;
        write(key, "", self.default.as_ref())
    }
}

#[derive(Clone, Copy)]
struct Container {
    class: bool,
    shell: bool,
}

impl Container {
    fn read(key: &str) -> io::Result<Self> {
        Ok(Self {
            class: counts(class_key(key))?.is_some(),
            shell: counts(key)?.is_some(),
        })
    }

    fn cleanup(self, key: &str) -> io::Result<()> {
        if !self.shell {
            delete_empty_key(key)?;
        }
        if !self.class {
            delete_empty_key(class_key(key))?;
        }
        Ok(())
    }
}

fn class_key(key: &str) -> &str {
    key.rsplit_once('\\').unwrap().0
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Value {
    kind: u32,
    data: Vec<u8>,
}

impl Value {
    fn text(text: impl AsRef<OsStr>) -> Self {
        Self {
            kind: REG_SZ,
            data: wide(text).into_iter().flat_map(u16::to_le_bytes).collect(),
        }
    }

    fn text_eq(&self, text: &str) -> bool {
        let (pairs, remainder) = self.data.as_chunks::<2>();
        if self.kind != REG_SZ || !remainder.is_empty() {
            return false;
        }
        let units: Vec<_> = pairs
            .iter()
            .copied()
            .map(u16::from_le_bytes)
            .take_while(|unit| *unit != 0)
            .collect();
        String::from_utf16_lossy(&units).eq_ignore_ascii_case(text)
    }

    fn backup(original: Option<&Self>, container: Container) -> Self {
        let mut data = vec![u8::from(container.class) | (u8::from(container.shell) << 1)];
        if let Some(value) = original {
            data.extend(value.kind.to_le_bytes());
            data.extend(&value.data);
        }
        Self {
            kind: REG_BINARY,
            data,
        }
    }

    fn valid_backup(&self) -> bool {
        self.kind == REG_BINARY
            && (self.data.len() == 1 || self.data.len() >= 5)
            && matches!(self.data[0], 0 | 1 | 3)
    }

    fn container(&self) -> Container {
        Container {
            class: self.data[0] & 1 != 0,
            shell: self.data[0] & 2 != 0,
        }
    }

    fn original(&self) -> Option<Self> {
        Some(Self {
            kind: u32::from_le_bytes(self.data.get(1..5)?.try_into().unwrap()),
            data: self.data[5..].into(),
        })
    }
}

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

fn result(code: u32) -> io::Result<()> {
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code as i32))
    }
}

fn open(path: &str, access: u32) -> io::Result<Option<Key>> {
    open_in(HKEY_CURRENT_USER, path, access)
}

fn open_in(hive: HKEY, path: &str, access: u32) -> io::Result<Option<Key>> {
    let mut key = null_mut();
    match unsafe { RegOpenKeyExW(hive, wide(path).as_ptr(), 0, access, &mut key) } {
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(None),
        code => result(code).map(|_| Some(Key(key))),
    }
}

fn read(path: &str, name: &str) -> io::Result<Option<Value>> {
    let Some(key) = open(path, KEY_QUERY_VALUE)? else {
        return Ok(None);
    };
    let name = wide(name);
    let mut kind = 0;
    let mut data = Vec::new();
    loop {
        let mut length = data.len() as u32;
        let buffer = if data.is_empty() {
            null_mut()
        } else {
            data.as_mut_ptr()
        };
        match unsafe {
            RegQueryValueExW(key.0, name.as_ptr(), null(), &mut kind, buffer, &mut length)
        } {
            ERROR_FILE_NOT_FOUND => return Ok(None),
            ERROR_MORE_DATA => data.resize(length as usize, 0),
            ERROR_SUCCESS if length as usize > data.len() => data.resize(length as usize, 0),
            code => {
                result(code)?;
                data.truncate(length as usize);
                return Ok(Some(Value { kind, data }));
            }
        }
    }
}

fn write(path: &str, name: &str, value: Option<&Value>) -> io::Result<()> {
    let name = wide(name);
    if let Some(value) = value {
        let mut key = null_mut();
        result(unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide(path).as_ptr(),
                0,
                null(),
                0,
                KEY_SET_VALUE,
                null(),
                &mut key,
                null_mut(),
            )
        })?;
        let key = Key(key);
        result(unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                value.kind,
                value.data.as_ptr(),
                value.data.len() as u32,
            )
        })
    } else if let Some(key) = open(path, KEY_SET_VALUE)? {
        match unsafe { RegDeleteValueW(key.0, name.as_ptr()) } {
            ERROR_FILE_NOT_FOUND => Ok(()),
            code => result(code),
        }
    } else {
        Ok(())
    }
}

fn counts(path: &str) -> io::Result<Option<(u32, u32)>> {
    counts_in(HKEY_CURRENT_USER, path)
}

fn counts_in(hive: HKEY, path: &str) -> io::Result<Option<(u32, u32)>> {
    let Some(key) = open_in(hive, path, KEY_QUERY_VALUE)? else {
        return Ok(None);
    };
    let mut keys = 0;
    let mut values = 0;
    result(unsafe {
        RegQueryInfoKeyW(
            key.0,
            null_mut(),
            null_mut(),
            null(),
            &mut keys,
            null_mut(),
            null_mut(),
            &mut values,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
        )
    })?;
    Ok(Some((keys, values)))
}

fn delete_empty_key(path: &str) -> io::Result<()> {
    if counts(path)? == Some((0, 0)) {
        match unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, wide(path).as_ptr()) } {
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => (),
            code => result(code)?,
        }
    }
    Ok(())
}

struct Lock(HANDLE);

impl Lock {
    fn acquire() -> io::Result<Self> {
        let handle = unsafe {
            CreateMutexW(
                null(),
                0,
                wide(r"Local\BetterExplorer.FolderManager").as_ptr(),
            )
        };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        match unsafe { WaitForSingleObject(handle, 2000) } {
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self(handle)),
            _ => {
                unsafe { CloseHandle(handle) };
                Err(io::Error::other(
                    "The folder manager setting is busy. Try again.",
                ))
            }
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Registry::{RegDeleteTreeW, REG_EXPAND_SZ};

    struct TestScope(Scope);

    impl TestScope {
        fn new(path: &Path) -> io::Result<Self> {
            Scope::test(path).map(Self)
        }
    }

    impl Drop for TestScope {
        fn drop(&mut self) {
            unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, wide(&self.0.root).as_ptr()) };
        }
    }

    #[test]
    fn restores_previous_defaults_and_preserves_third_party_changes() -> io::Result<()> {
        let test_root = tempfile::tempdir()?;
        let scope = TestScope::new(test_root.path())?;
        let fake_exe = test_root.path().join("BetterExplorer.exe");
        let registration = Registration::new(&scope.0, &fake_exe)?;
        let directory = &registration.keys[0];
        let drive = &registration.keys[1];
        let previous = Value {
            kind: REG_EXPAND_SZ,
            data: Value::text("%OtherFileManager%").data,
        };
        write(directory, "", Some(&previous))?;

        assert_eq!(registration.status()?, Status::Inactive);
        registration.change(true)?;
        assert_eq!(registration.status()?, Status::Active);
        registration.change(true)?;
        assert_eq!(read(directory, "")?, Some(Value::text(VERB)));

        let newer = Value::text("OtherDriveManager");
        write(drive, "", Some(&newer))?;
        assert_eq!(registration.status()?, Status::Partial);
        registration.change(false)?;
        assert_eq!(read(directory, "")?, Some(previous));
        assert_eq!(read(drive, "")?, Some(newer));
        assert_eq!(read(&verb_key(directory), OWNER)?, None);
        assert_eq!(read(&verb_key(drive), OWNER)?, None);
        assert_eq!(registration.status()?, Status::Inactive);
        Ok(())
    }

    #[test]
    fn another_copy_cannot_take_over_or_remove_the_registration() -> io::Result<()> {
        let test_root = tempfile::tempdir()?;
        let scope = TestScope::new(test_root.path())?;
        let first_path = test_root.path().join("First.exe");
        let second_path = test_root.path().join("Second.exe");
        let first = Registration::new(&scope.0, &first_path)?;
        let second = Registration::new(&scope.0, &second_path)?;
        first.change(true)?;
        assert_eq!(second.status()?, Status::Conflict);
        assert!(second.change(true).is_err());
        second.change(false)?;
        assert_eq!(first.status()?, Status::Active);
        first.change(false)?;
        assert_eq!(first.status()?, Status::Inactive);
        Ok(())
    }

    #[test]
    fn refuses_to_uninstall_an_active_owned_verb_with_a_modified_command() -> io::Result<()> {
        let test_root = tempfile::tempdir()?;
        let scope = TestScope::new(test_root.path())?;
        let path = test_root.path().join("BetterExplorer.exe");
        let registration = Registration::new(&scope.0, &path)?;
        registration.change(true)?;

        let key = &registration.keys[0];
        let command = Value::text(format!(r#""{}" "%1""#, path.display()));
        write(&command_key(key), "", Some(&command))?;
        let result = registration.change(false);
        let default_after_attempt = read(key, "")?;
        let command_after_attempt = read(&command_key(key), "")?;
        write(&command_key(key), "", Some(&registration.command))?;
        registration.change(false)?;

        assert!(result.is_err());
        assert_eq!(default_after_attempt, Some(Value::text(VERB)));
        assert_eq!(command_after_attempt, Some(command));
        assert_eq!(registration.status()?, Status::Inactive);
        Ok(())
    }
}
