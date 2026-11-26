use crate::folder_command;
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
const LABEL: &str = "Open folder";
const LEGACY_LABEL: &str = "Open in BetterExplorer";
const DELEGATE: &str = "DelegateExecute";
const MULTI_SELECT: &str = "MultiSelectModel";
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
    let operation = if enabled {
        Change::Enable
    } else {
        Change::Disable
    };
    Registration::new(&Scope::live(), &std::env::current_exe()?)?.change(operation)
}

pub fn cli(arg: &OsStr, args: impl Iterator<Item = OsString>) -> Option<i32> {
    let operation = match arg.to_str()? {
        "--folder-manager-on" => Some(Change::Enable),
        "--folder-manager-off" => Some(Change::Disable),
        "--folder-manager-upgrade" => Some(Change::Upgrade),
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
        Ok(match operation {
            Some(operation) => {
                registration.change(operation)?;
                0
            }
            None => match registration.status()? {
                Status::Active => 0,
                Status::Inactive => 2,
                Status::Partial => 3,
                Status::Conflict => 4,
            },
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
    server: ServerRegistration,
}

#[derive(Clone, Copy)]
enum Change {
    Enable,
    Disable,
    Upgrade,
}

impl<'a> Registration<'a> {
    fn new(scope: &'a Scope, owner: &Path) -> io::Result<Self> {
        if !owner.is_absolute() || owner.as_os_str().encode_wide().any(|c| c == b'"' as u16) {
            return Err(invalid_options());
        }
        let mut executable = OsString::from("\"");
        executable.push(owner.as_os_str());
        executable.push("\"");
        // The suffix keeps a drive root's backslash away from the closing quote.
        let mut command = executable.clone();
        command.push(r#" --open-folder "%1\.""#);
        let mut server_command = executable;
        server_command.push(" ");
        server_command.push(folder_command::SERVER_ARG);
        let class_id = format!("{{{:?}}}", folder_command::CLSID);
        let class = format!(r"{}\CLSID\{class_id}", scope.root);
        Ok(Self {
            scope,
            keys: CLASSES.map(|class| format!(r"{}\{class}", scope.root)),
            owner: Value::text(owner),
            command: Value::text(command),
            server: ServerRegistration {
                delegate: Value::text(class_id),
                command_key: format!(r"{class}\LocalServer32"),
                class,
                command: Value::text(server_command),
            },
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
        let multi_select = read(&verb, MULTI_SELECT)?;
        let command = read(&command_key, "")?;
        let delegate = read(&command_key, DELEGATE)?;
        let active = default.as_ref().is_some_and(|value| value.text_eq(VERB));
        let verb_counts = counts(&verb)?;
        let command_counts = counts(&command_key)?;
        let machine_verb = !self.scope.testing && counts_in(HKEY_LOCAL_MACHINE, &verb)?.is_some();
        let owned = owner.as_ref() == Some(&self.owner)
            && label
                .as_ref()
                .is_none_or(|value| value.text_eq(LABEL) || value.text_eq(LEGACY_LABEL))
            && multi_select
                .as_ref()
                .is_none_or(|value| value.text_eq("Document"))
            && command.as_ref().is_none_or(|value| value == &self.command)
            && delegate
                .as_ref()
                .is_none_or(|value| value == &self.server.delegate)
            && previous.as_ref().map_or(!active, Value::valid_backup)
            && verb_counts
                == Some((
                    u32::from(command_counts.is_some()),
                    1 + u32::from(previous.is_some())
                        + u32::from(label.is_some())
                        + u32::from(multi_select.is_some()),
                ))
            && command_counts.is_none_or(|value| {
                value
                    == (
                        0,
                        u32::from(command.is_some()) + u32::from(delegate.is_some()),
                    )
            });
        let foreign = !owned
            && (verb_counts.is_some_and(|counts| counts != (0, 0)) || active || machine_verb);
        Ok(Snapshot {
            container,
            default,
            owner,
            label,
            previous,
            multi_select,
            command,
            delegate,
            owned,
            active,
            foreign,
        })
    }

    fn status(&self) -> io::Result<Status> {
        let _lock = Lock::acquire()?;
        let states = self.snapshots()?;
        let server = self.server.snapshot(self.scope, &self.owner)?;
        let legacy = states.iter().all(|state| state.delegate.is_none());
        let current = server.command.is_some() && states.iter().all(Snapshot::current_command);
        Ok(
            if server.foreign || states.iter().any(|state| state.foreign) {
                Status::Conflict
            } else if states.iter().all(Snapshot::active_command) && (legacy || current) {
                Status::Active
            } else if server.owned || states.iter().any(|state| state.owned) {
                Status::Partial
            } else {
                Status::Inactive
            },
        )
    }

    fn change(&self, operation: Change) -> io::Result<()> {
        let _lock = Lock::acquire()?;
        let old = self.snapshots()?;
        let server = self.server.snapshot(self.scope, &self.owner)?;
        if matches!(operation, Change::Upgrade)
            && (server.foreign
                || !old.iter().all(Snapshot::active_command)
                || (server.command.is_some() && old.iter().all(Snapshot::current_command)))
        {
            return Ok(());
        }
        let enabled = !matches!(operation, Change::Disable);
        if enabled && (server.foreign || old.iter().any(|state| state.foreign)) {
            return Err(io::Error::other(
                "Another BetterExplorer copy or an existing shell registration uses this folder command.",
            ));
        }
        if !enabled
            && ((server.foreign && server.command.as_ref() == Some(&self.server.command))
                || old.iter().any(|state| {
                    state.foreign && state.active && state.owner.as_ref() == Some(&self.owner)
                        || server.owned
                            && !state.owned
                            && state.delegate.as_ref() == Some(&self.server.delegate)
                }))
        {
            return Err(io::Error::other(
                "The BetterExplorer folder command was changed outside the app and needs to be restored before it can be removed.",
            ));
        }
        let change_server = enabled || server.owned;
        if !change_server && !old.iter().any(|state| state.owned) {
            return Ok(());
        }
        let mut tried_server = false;
        let mut tried_verbs = 0;
        let mut apply = || -> io::Result<()> {
            if enabled {
                tried_server = true;
                self.server.install(&self.owner)?;
            }
            for (index, (key, state)) in self.keys.iter().zip(&old).enumerate() {
                tried_verbs = index + 1;
                if enabled {
                    let previous = if state.owned && state.active {
                        state.previous.clone().unwrap()
                    } else {
                        let container = state
                            .previous
                            .as_ref()
                            .map_or(state.container, Value::container);
                        Value::backup(state.default.as_ref(), container)
                    };
                    self.install(key, &previous)?;
                } else if state.owned {
                    self.remove(key, state)?;
                }
            }
            if !enabled && change_server {
                tried_server = true;
                self.server.clear()?;
            }
            Ok(())
        };
        let result = apply();
        if let Err(error) = result {
            let mut recovery = Ok(());
            if tried_server && !enabled {
                recovery = self.server.restore(&server);
            }
            for (key, state) in self.keys[..tried_verbs].iter().zip(&old).rev() {
                if (enabled || state.owned) && !state.foreign {
                    recovery = recovery.and(state.restore(key));
                }
            }
            if tried_server && enabled {
                recovery = recovery.and(self.server.restore(&server));
            }
            self.notify();
            return Err(match recovery {
                Ok(()) => error,
                Err(recovery) => io::Error::other(format!("{error} Rollback: {recovery}")),
            });
        }
        self.notify();
        Ok(())
    }

    fn install(&self, key: &str, previous: &Value) -> io::Result<()> {
        let verb = verb_key(key);
        write(&verb, OWNER, Some(&self.owner))?;
        write(&verb, PREVIOUS, Some(previous))?;
        write(&verb, "", Some(&Value::text(LABEL)))?;
        write(&verb, MULTI_SELECT, Some(&Value::text("Document")))?;
        let command = command_key(key);
        write(&command, "", Some(&self.command))?;
        write(&command, DELEGATE, Some(&self.server.delegate))?;
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

struct ServerRegistration {
    class: String,
    command_key: String,
    command: Value,
    delegate: Value,
}

impl ServerRegistration {
    fn snapshot(&self, scope: &Scope, expected_owner: &Value) -> io::Result<ServerSnapshot> {
        let class_counts = counts(&self.class)?;
        let server_counts = counts(&self.command_key)?;
        let owner = read(&self.class, OWNER)?;
        let command = read(&self.command_key, "")?;
        let owned = owner.as_ref() == Some(expected_owner)
            && class_counts == Some((u32::from(server_counts.is_some()), 1))
            && server_counts.is_none_or(|counts| counts == (0, u32::from(command.is_some())))
            && command.as_ref().is_none_or(|value| value == &self.command);
        let machine_class = !scope.testing && counts_in(HKEY_LOCAL_MACHINE, &self.class)?.is_some();
        Ok(ServerSnapshot {
            owner,
            command,
            has_server_key: server_counts.is_some(),
            owned,
            foreign: machine_class || (!owned && class_counts.is_some()),
        })
    }

    fn install(&self, owner: &Value) -> io::Result<()> {
        write(&self.class, OWNER, Some(owner))?;
        write(&self.command_key, "", Some(&self.command))
    }

    fn clear(&self) -> io::Result<()> {
        write(&self.command_key, "", None)?;
        delete_empty_key(&self.command_key)?;
        write(&self.class, OWNER, None)?;
        delete_empty_key(&self.class)
    }

    fn restore(&self, snapshot: &ServerSnapshot) -> io::Result<()> {
        if !snapshot.owned {
            return self.clear();
        }
        write(&self.class, OWNER, snapshot.owner.as_ref())?;
        write(&self.command_key, "", snapshot.command.as_ref())?;
        if snapshot.has_server_key {
            create_for_write(&self.command_key)?;
        } else {
            delete_empty_key(&self.command_key)?;
        }
        Ok(())
    }
}

struct ServerSnapshot {
    owner: Option<Value>,
    command: Option<Value>,
    has_server_key: bool,
    owned: bool,
    foreign: bool,
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
    write(&command, DELEGATE, None)?;
    delete_empty_key(&command)?;
    for name in ["", PREVIOUS, OWNER, MULTI_SELECT] {
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
    multi_select: Option<Value>,
    command: Option<Value>,
    delegate: Option<Value>,
    owned: bool,
    active: bool,
    foreign: bool,
}

impl Snapshot {
    fn active_command(&self) -> bool {
        self.owned && self.active && self.command.is_some()
    }

    fn current_command(&self) -> bool {
        self.delegate.is_some() && self.multi_select.is_some()
    }

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
        write(&verb, MULTI_SELECT, self.multi_select.as_ref())?;
        let command = command_key(key);
        write(&command, "", self.command.as_ref())?;
        write(&command, DELEGATE, self.delegate.as_ref())?;
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
        let key = create_for_write(path)?;
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

fn create_for_write(path: &str) -> io::Result<Key> {
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
    Ok(Key(key))
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
    fn upgrades_an_active_legacy_registration_without_changing_its_backups() -> io::Result<()> {
        let test_root = tempfile::tempdir()?;
        let scope = TestScope::new(test_root.path())?;
        let path = test_root.path().join("BetterExplorer.exe");
        let registration = Registration::new(&scope.0, &path)?;
        registration.change(Change::Upgrade)?;
        assert_eq!(registration.status()?, Status::Inactive);
        let legacy_label = Value::text("Open in BetterExplorer");
        let previous = Value::text("OtherFolderManager");
        let directory = &registration.keys[0];
        write(directory, "", Some(&previous))?;
        for key in &registration.keys {
            let backup = Value::backup(read(key, "")?.as_ref(), Container::read(key)?);
            let verb = verb_key(key);
            write(&verb, OWNER, Some(&registration.owner))?;
            write(&verb, PREVIOUS, Some(&backup))?;
            write(&verb, "", Some(&legacy_label))?;
            write(&command_key(key), "", Some(&registration.command))?;
            write(key, "", Some(&Value::text(VERB)))?;
        }
        assert_eq!(registration.status()?, Status::Active);

        registration.change(Change::Upgrade)?;

        let clsid = "{801E1D22-334A-4B1F-9F3B-A99DC4A38721}";
        let class = format!(r"{}\CLSID\{clsid}", scope.0.root);
        let server = format!(r"{class}\LocalServer32");
        assert_eq!(
            read(&server, "")?,
            Some(Value::text(format!(
                r#""{}" --folder-command-server"#,
                path.display()
            )))
        );
        for key in &registration.keys {
            assert!(read(&command_key(key), "DelegateExecute")?
                .is_some_and(|value| value.text_eq(clsid)));
            assert_eq!(
                read(&verb_key(key), MULTI_SELECT)?,
                Some(Value::text("Document"))
            );
        }
        assert_eq!(registration.status()?, Status::Active);
        registration.change(Change::Upgrade)?;

        registration.change(Change::Disable)?;
        assert_eq!(read(directory, "")?, Some(previous));
        assert_eq!(read(&registration.keys[1], "")?, None);
        assert_eq!(counts(&class)?, None);
        assert_eq!(registration.status()?, Status::Inactive);
        Ok(())
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
        registration.change(Change::Enable)?;
        assert_eq!(registration.status()?, Status::Active);
        registration.change(Change::Enable)?;
        assert_eq!(read(directory, "")?, Some(Value::text(VERB)));

        let newer = Value::text("OtherDriveManager");
        write(drive, "", Some(&newer))?;
        assert_eq!(registration.status()?, Status::Partial);
        registration.change(Change::Upgrade)?;
        assert_eq!(read(drive, "")?, Some(newer.clone()));
        registration.change(Change::Disable)?;
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
        first.change(Change::Enable)?;
        assert_eq!(second.status()?, Status::Conflict);
        assert!(second.change(Change::Enable).is_err());
        second.change(Change::Disable)?;
        assert_eq!(first.status()?, Status::Active);
        first.change(Change::Disable)?;
        assert_eq!(first.status()?, Status::Inactive);
        Ok(())
    }

    #[test]
    fn refuses_to_uninstall_an_active_owned_verb_with_a_modified_command() -> io::Result<()> {
        let test_root = tempfile::tempdir()?;
        let scope = TestScope::new(test_root.path())?;
        let path = test_root.path().join("BetterExplorer.exe");
        let registration = Registration::new(&scope.0, &path)?;
        registration.change(Change::Enable)?;

        let key = &registration.keys[0];
        let command = Value::text(format!(r#""{}" "%1""#, path.display()));
        write(&command_key(key), "", Some(&command))?;
        let result = registration.change(Change::Disable);
        let default_after_attempt = read(key, "")?;
        let command_after_attempt = read(&command_key(key), "")?;
        write(&command_key(key), "", Some(&registration.command))?;
        registration.change(Change::Disable)?;

        assert!(result.is_err());
        assert_eq!(default_after_attempt, Some(Value::text(VERB)));
        assert_eq!(command_after_attempt, Some(command));
        assert_eq!(registration.status()?, Status::Inactive);
        Ok(())
    }

    #[test]
    fn preserves_a_modified_server_registration_and_refuses_to_uninstall_its_executable(
    ) -> io::Result<()> {
        let test_root = tempfile::tempdir()?;
        let scope = TestScope::new(test_root.path())?;
        let registration =
            Registration::new(&scope.0, &test_root.path().join("BetterExplorer.exe"))?;
        registration.change(Change::Enable)?;
        let extra = Value::text("Other configuration");
        write(&registration.server.class, "Extra", Some(&extra))?;

        assert_eq!(registration.status()?, Status::Conflict);
        assert!(registration.change(Change::Disable).is_err());
        assert_eq!(read(&registration.server.class, "Extra")?, Some(extra));
        assert_eq!(
            read(&registration.server.command_key, "")?,
            Some(registration.server.command.clone())
        );
        for key in &registration.keys {
            assert_eq!(read(key, "")?, Some(Value::text(VERB)));
        }
        Ok(())
    }
}
