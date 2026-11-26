use std::{
    cell::{Cell, RefCell},
    env,
    ffi::{OsStr, OsString},
    io,
    mem::size_of,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::Command,
    ptr::{null, null_mut},
    sync::mpsc::{self, SyncSender},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
        HANDLE, HWND, LPARAM, LRESULT, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM,
    },
    Storage::FileSystem::SYNCHRONIZE,
    System::{
        LibraryLoader::GetModuleHandleW,
        Registry::{
            RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
            RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
        },
        Threading::{
            CreateMutexW, OpenProcess, QueryFullProcessImageNameW, ReleaseMutex,
            WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
            KEYEVENTF_KEYUP, VK_E, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_RCONTROL,
            VK_RMENU, VK_RSHIFT, VK_RWIN,
        },
        WindowsAndMessaging::{
            CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
            FindWindowExW, GetMessageW, GetWindowTextW, GetWindowThreadProcessId, PostMessageW,
            PostQuitMessage, RegisterClassW, SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION,
            HHOOK, HWND_MESSAGE, KBDLLHOOKSTRUCT, LLKHF_ALTDOWN, LLKHF_INJECTED,
            LLKHF_LOWER_IL_INJECTED, MSG, WH_KEYBOARD_LL, WM_CLOSE, WM_DESTROY, WM_KEYDOWN,
            WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASSW,
        },
    },
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "BetterExplorerWinE";
const LISTENER_ARG: &str = "--win-e-listener";
const LISTENER_MUTEX: &str = r"Local\BetterExplorer.WinE.Listener.1";
const SETTINGS_MUTEX: &str = r"Local\BetterExplorer.WinE.Settings.1";
const LISTENER_CLASS: &str = "BetterExplorerWinEListener";
const STARTUP_WAIT_MS: u32 = 4000;
const POLL_INTERVAL: Duration = Duration::from_millis(25);

pub struct Status {
    pub configured: bool,
    pub running: bool,
    pub another_copy: bool,
}

pub fn status() -> io::Result<Status> {
    let exe = env::current_exe()?;
    let startup = startup_state(&exe)?;
    let listener = find_listener(&exe);
    Ok(Status {
        configured: startup == StartupState::Ours,
        running: matches!(listener, Listener::Ours { .. }),
        another_copy: startup == StartupState::Other || matches!(listener, Listener::Other),
    })
}

pub fn set_enabled(enabled: bool) -> io::Result<()> {
    let exe = env::current_exe()?;
    let _settings = settings_lock()?;
    let startup = startup_state(&exe)?;
    if startup == StartupState::Other {
        return Err(other_copy());
    }
    if enabled {
        if matches!(find_listener(&exe), Listener::Other) {
            return Err(other_copy());
        }
        if startup == StartupState::Missing {
            write_startup(&exe)?;
        }
        if let Err(error) = start_listener(&exe) {
            if startup == StartupState::Missing {
                remove_own_startup(&exe)?;
            }
            return Err(error);
        }
        Ok(())
    } else {
        remove_own_startup(&exe)?;
        stop_listener(&exe)
    }
}

pub fn resume_on_launch() {
    if env::var_os("BETTEREXPLORER_EPHEMERAL").is_some() {
        return;
    }
    let _ = thread::Builder::new().spawn(|| {
        if let Ok(exe) = env::current_exe() {
            if startup_state(&exe).ok() == Some(StartupState::Ours)
                && matches!(find_listener(&exe), Listener::Missing)
            {
                let _ = resume_configured_listener();
            }
        }
    });
}

fn resume_configured_listener() -> io::Result<()> {
    with_settings_lock(|exe| {
        if startup_state(exe)? == StartupState::Ours {
            start_listener(exe)?;
        }
        Ok(())
    })
}

pub fn cli(arg: &OsStr) -> Option<i32> {
    let result = match arg.to_str()? {
        "--win-e-enabled" => {
            return Some(
                match env::current_exe().and_then(|exe| startup_state(&exe)) {
                    Ok(StartupState::Ours) => 0,
                    Ok(_) => 2,
                    Err(_) => 1,
                },
            );
        }
        "--win-e-on" => set_enabled(true),
        "--win-e-off" => set_enabled(false),
        "--win-e-pause" => with_settings_lock(stop_listener),
        "--win-e-resume" => resume_configured_listener(),
        LISTENER_ARG => env::current_exe().and_then(run_listener),
        _ => return None,
    };
    Some(i32::from(result.is_err()))
}

fn with_settings_lock(operation: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()> {
    let exe = env::current_exe()?;
    let _settings = settings_lock()?;
    operation(&exe)
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

fn startup_command(exe: &Path) -> Vec<u16> {
    let mut command = OsString::from("\"");
    command.push(exe.as_os_str());
    command.push(format!("\" {LISTENER_ARG}"));
    wide(command)
}

#[derive(PartialEq, Eq)]
enum StartupState {
    Missing,
    Ours,
    Other,
}

struct RunKey(HKEY);

impl RunKey {
    fn state(&self, expected: &[u16]) -> io::Result<StartupState> {
        let name = wide(RUN_VALUE);
        let mut data = vec![0u16; expected.len()];
        let mut bytes = std::mem::size_of_val(data.as_slice()) as u32;
        let mut value_type = 0;
        let result = unsafe {
            RegQueryValueExW(
                self.0,
                name.as_ptr(),
                null(),
                &mut value_type,
                data.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        match result {
            ERROR_FILE_NOT_FOUND => Ok(StartupState::Missing),
            ERROR_MORE_DATA => Ok(StartupState::Other),
            ERROR_SUCCESS => {
                let is_ours = value_type == REG_SZ
                    && bytes as usize == std::mem::size_of_val(expected)
                    && data == expected;
                Ok(if is_ours {
                    StartupState::Ours
                } else {
                    StartupState::Other
                })
            }
            error => Err(io::Error::from_raw_os_error(error as i32)),
        }
    }
}

impl Drop for RunKey {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

fn open_run_key(access: u32) -> io::Result<Option<RunKey>> {
    let mut key = null_mut();
    let path = wide(RUN_KEY);
    match unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, access, &mut key) } {
        ERROR_SUCCESS => Ok(Some(RunKey(key))),
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(None),
        error => Err(io::Error::from_raw_os_error(error as i32)),
    }
}

fn startup_state(exe: &Path) -> io::Result<StartupState> {
    match open_run_key(KEY_QUERY_VALUE)? {
        Some(key) => key.state(&startup_command(exe)),
        None => Ok(StartupState::Missing),
    }
}

fn write_startup(exe: &Path) -> io::Result<()> {
    let command = startup_command(exe);
    if command.len() > 261 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows startup commands can contain at most 260 characters.",
        ));
    }
    let path = wide(RUN_KEY);
    let mut key = null_mut();
    registry_result(unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            null(),
            0,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            null(),
            &mut key,
            null_mut(),
        )
    })?;
    let key = RunKey(key);
    match key.state(&command)? {
        StartupState::Ours => return Ok(()),
        StartupState::Other => return Err(other_copy()),
        StartupState::Missing => {}
    }
    let name = wide(RUN_VALUE);
    registry_result(unsafe {
        RegSetValueExW(
            key.0,
            name.as_ptr(),
            0,
            REG_SZ,
            command.as_ptr().cast(),
            std::mem::size_of_val(command.as_slice()) as u32,
        )
    })
}

fn remove_own_startup(exe: &Path) -> io::Result<()> {
    if startup_state(exe)? == StartupState::Missing {
        return Ok(());
    }
    let Some(key) = open_run_key(KEY_QUERY_VALUE | KEY_SET_VALUE)? else {
        return Ok(());
    };
    match key.state(&startup_command(exe))? {
        StartupState::Missing => Ok(()),
        StartupState::Other => Err(other_copy()),
        StartupState::Ours => {
            let name = wide(RUN_VALUE);
            let result = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
            if result == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                registry_result(result)
            }
        }
    }
}

fn registry_result(result: u32) -> io::Result<()> {
    match result {
        ERROR_SUCCESS => Ok(()),
        error => Err(io::Error::from_raw_os_error(error as i32)),
    }
}

fn other_copy() -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Another copy of BetterExplorer has configured or started the Win+E listener.",
    )
}

fn timed_out(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, message)
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

struct MutexGuard(Handle);

impl Drop for MutexGuard {
    fn drop(&mut self) {
        unsafe { ReleaseMutex(self.0 .0) };
    }
}

fn lock(name: &str, timeout_ms: u32) -> io::Result<Option<MutexGuard>> {
    let name = wide(name);
    let handle = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let handle = Handle(handle);
    match unsafe { WaitForSingleObject(handle.0, timeout_ms) } {
        WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Some(MutexGuard(handle))),
        WAIT_TIMEOUT => Ok(None),
        _ => Err(io::Error::last_os_error()),
    }
}

fn settings_lock() -> io::Result<MutexGuard> {
    lock(SETTINGS_MUTEX, STARTUP_WAIT_MS)?
        .ok_or_else(|| timed_out("The Win+E setting is being updated."))
}

enum Listener {
    Missing,
    Ours { window: HWND, process: Handle },
    Other,
}

fn find_listener(exe: &Path) -> Listener {
    find_listener_for_class(exe, LISTENER_CLASS)
}

fn find_listener_for_class(exe: &Path, class: &str) -> Listener {
    let class = wide(class);
    let window = unsafe { FindWindowExW(HWND_MESSAGE, null_mut(), class.as_ptr(), null()) };
    if window.is_null() {
        return Listener::Missing;
    }
    let expected = wide(exe);
    let mut actual = vec![0u16; expected.len() + 1];
    let length = unsafe { GetWindowTextW(window, actual.as_mut_ptr(), actual.len() as i32) };
    if length as usize + 1 != expected.len() || actual[..expected.len()] != expected {
        return Listener::Other;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid) };
    if process.is_null() {
        return Listener::Other;
    }
    let process = Handle(process);
    let mut length = actual.len() as u32;
    let found =
        unsafe { QueryFullProcessImageNameW(process.0, 0, actual.as_mut_ptr(), &mut length) };
    if found == 0 || length as usize + 1 != expected.len() || actual[..expected.len()] != expected {
        return Listener::Other;
    }
    let mut current_pid = 0;
    unsafe { GetWindowThreadProcessId(window, &mut current_pid) };
    if pid != current_pid {
        return Listener::Missing;
    }
    Listener::Ours { window, process }
}

fn start_listener(exe: &Path) -> io::Result<()> {
    match find_listener(exe) {
        Listener::Ours { .. } => return Ok(()),
        Listener::Other => return Err(other_copy()),
        Listener::Missing => {}
    }
    let mut child = Command::new(exe).arg(LISTENER_ARG).spawn()?;
    let deadline = Instant::now() + Duration::from_millis(STARTUP_WAIT_MS.into());
    loop {
        match find_listener(exe) {
            Listener::Ours { .. } => return Ok(()),
            Listener::Other => return Err(other_copy()),
            Listener::Missing => {}
        }
        if let Some(status) = child.try_wait()? {
            return Err(io::Error::other(format!(
                "The Win+E listener exited before it was ready ({status})."
            )));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(timed_out("The Win+E listener did not start."));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn stop_listener(exe: &Path) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_millis(STARTUP_WAIT_MS.into());
    loop {
        match find_listener(exe) {
            Listener::Other => return Ok(()),
            Listener::Ours { window, process } => {
                let posted = unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
                let timeout = if posted == 0 { 0 } else { STARTUP_WAIT_MS };
                let post_error = (posted == 0).then(io::Error::last_os_error);
                return match unsafe { WaitForSingleObject(process.0, timeout) } {
                    WAIT_OBJECT_0 => Ok(()),
                    WAIT_TIMEOUT => {
                        Err(post_error
                            .unwrap_or_else(|| timed_out("The Win+E listener did not stop.")))
                    }
                    _ => Err(io::Error::last_os_error()),
                };
            }
            Listener::Missing => {
                if lock(LISTENER_MUTEX, 0)?.is_some() {
                    return Ok(());
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(timed_out("The Win+E listener did not respond."));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

const MODIFIERS: [u16; 8] = [
    VK_LWIN,
    VK_RWIN,
    VK_LSHIFT,
    VK_RSHIFT,
    VK_LCONTROL,
    VK_RCONTROL,
    VK_LMENU,
    VK_RMENU,
];
const WINDOWS_KEYS: u8 = 0b11;

#[derive(Clone, Copy, Default)]
struct Chord {
    modifiers: u8,
    e_down: bool,
    suppress_e: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Pass,
    Suppress,
    Open,
}

impl Chord {
    fn update_modifiers(&mut self, logical: u8) {
        let previously_pressed_win = self.modifiers & WINDOWS_KEYS;
        self.modifiers &= logical;
        if previously_pressed_win != 0 && self.modifiers & WINDOWS_KEYS == 0 {
            self.e_down = false;
            self.suppress_e = false;
        }
    }

    fn key(&mut self, key: u16, down: bool, other_modifier: bool) -> Action {
        if let Some(index) = MODIFIERS.iter().position(|&modifier| modifier == key) {
            let bit = 1 << index;
            self.modifiers &= !bit;
            if down {
                self.modifiers |= bit;
            }
        }
        if key != VK_E {
            return Action::Pass;
        }

        let repeat = self.e_down && down;
        self.e_down = down;
        if self.suppress_e {
            self.suppress_e = down;
            return Action::Suppress;
        }
        if down
            && !repeat
            && !other_modifier
            && self.modifiers & WINDOWS_KEYS != 0
            && self.modifiers & !WINDOWS_KEYS == 0
        {
            Action::Open
        } else {
            Action::Pass
        }
    }
}

thread_local! {
    static CHORD: Cell<Chord> = const {
        Cell::new(Chord { modifiers: 0, e_down: false, suppress_e: false })
    };
    static OPEN_REQUESTS: RefCell<Option<SyncSender<()>>> = const { RefCell::new(None) };
}

unsafe extern "system" fn keyboard(code: i32, message: WPARAM, detail: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let event = &*(detail as *const KBDLLHOOKSTRUCT);
        if event.flags & (LLKHF_INJECTED | LLKHF_LOWER_IL_INJECTED) == 0 {
            let down = match message as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => true,
                WM_KEYUP | WM_SYSKEYUP => false,
                _ => return CallNextHookEx(null_mut(), code, message, detail),
            };
            let action = CHORD.with(|state| {
                let mut chord = state.get();
                let key = event.vkCode as u16;
                let mut other_modifier = event.flags & LLKHF_ALTDOWN != 0;
                if key == VK_E && down && chord.modifiers & WINDOWS_KEYS != 0 {
                    // Win+L and other desktop switches can hide a modifier's release from this
                    // hook. Query only modifiers here: this E event has yet to update its state.
                    let logical = MODIFIERS.iter().enumerate().fold(0, |mask, (index, &key)| {
                        if GetAsyncKeyState(key.into()) < 0 {
                            mask | (1 << index)
                        } else {
                            mask
                        }
                    });
                    chord.update_modifiers(logical);
                    other_modifier |= logical & !WINDOWS_KEYS != 0;
                }
                let action = chord.key(key, down, other_modifier);
                state.set(chord);
                action
            });
            if action == Action::Suppress {
                return 1;
            }
            if action == Action::Open && request_open() {
                CHORD.with(|state| {
                    let mut chord = state.get();
                    chord.suppress_e = true;
                    state.set(chord);
                });
                return 1;
            }
        }
    }
    CallNextHookEx(null_mut(), code, message, detail)
}

fn request_open() -> bool {
    OPEN_REQUESTS.with(|requests| {
        let requests = requests.borrow();
        let Some(sender) = requests.as_ref() else {
            return false;
        };
        // Explorer otherwise sees a bare Windows-key press and can open Start on release.
        // An unassigned key marks the Windows key as used without typing a character.
        const MENU_MASK_KEY: u16 = 0xe8;
        let inputs = [0, KEYEVENTF_KEYUP].map(|flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: MENU_MASK_KEY,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
        let sent = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                size_of::<INPUT>() as i32,
            )
        };
        if sent == 1 {
            unsafe { SendInput(1, &inputs[1], size_of::<INPUT>() as i32) };
        }
        sent == inputs.len() as u32 && sender.try_send(()).is_ok()
    })
}

struct Hook(HHOOK);

impl Hook {
    fn install() -> io::Result<Self> {
        CHORD.set(Chord {
            e_down: unsafe { GetAsyncKeyState(VK_E.into()) } < 0,
            ..Chord::default()
        });
        let hook = unsafe {
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), GetModuleHandleW(null()), 0)
        };
        if hook.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(hook))
        }
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        unsafe { UnhookWindowsHookEx(self.0) };
    }
}

unsafe extern "system" fn listener_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CLOSE => {
            DestroyWindow(window);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

fn make_message_window(class: &str, title: &OsStr) -> io::Result<HWND> {
    let class = wide(class);
    let title = wide(title);
    unsafe {
        let module = GetModuleHandleW(null());
        let mut info: WNDCLASSW = std::mem::zeroed();
        info.lpfnWndProc = Some(listener_procedure);
        info.hInstance = module;
        info.lpszClassName = class.as_ptr();
        if RegisterClassW(&info) == 0 {
            return Err(io::Error::last_os_error());
        }
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            module,
            null(),
        );
        if window.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(window)
        }
    }
}

fn message_loop() -> io::Result<()> {
    let mut message: MSG = unsafe { std::mem::zeroed() };
    loop {
        match unsafe { GetMessageW(&mut message, null_mut(), 0, 0) } {
            -1 => return Err(io::Error::last_os_error()),
            0 => return Ok(()),
            _ => unsafe { DispatchMessageW(&message) },
        };
    }
}

fn run_listener(exe: PathBuf) -> io::Result<()> {
    let Some(_listener) = lock(LISTENER_MUTEX, 0)? else {
        return Ok(());
    };
    if startup_state(&exe)? != StartupState::Ours {
        return Ok(());
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    let launch_exe = exe.clone();
    thread::Builder::new().spawn(move || {
        while receiver.recv().is_ok() {
            let _ = Command::new(&launch_exe).spawn();
        }
    })?;
    OPEN_REQUESTS.set(Some(sender));
    let _hook = Hook::install()?;
    make_message_window(LISTENER_CLASS, exe.as_os_str())?;
    message_loop()
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_R;

    #[test]
    fn recognizes_e_once_per_press_with_either_windows_key() {
        for win in [VK_LWIN, VK_RWIN] {
            let mut chord = Chord::default();
            assert_eq!(chord.key(win, true, false), Action::Pass);
            assert_eq!(chord.key(VK_E, true, false), Action::Open);
            chord.suppress_e = true;
            assert_eq!(chord.key(VK_E, true, false), Action::Suppress);
            assert_eq!(chord.key(win, false, false), Action::Pass);
            assert_eq!(chord.key(VK_E, false, false), Action::Suppress);
            assert_eq!(chord.key(VK_E, true, false), Action::Pass);
        }
    }

    #[test]
    fn lets_other_shortcuts_and_modifiers_through() {
        let mut chord = Chord::default();
        assert_eq!(chord.key(VK_LWIN, true, false), Action::Pass);
        assert_eq!(chord.key(VK_RWIN, true, false), Action::Pass);
        assert_eq!(chord.key(VK_LWIN, false, false), Action::Pass);
        assert_eq!(chord.key(VK_R, true, false), Action::Pass);
        assert_eq!(chord.key(VK_R, false, false), Action::Pass);
        for modifier in [
            VK_LSHIFT,
            VK_RSHIFT,
            VK_LCONTROL,
            VK_RCONTROL,
            VK_LMENU,
            VK_RMENU,
        ] {
            assert_eq!(chord.key(modifier, true, false), Action::Pass);
            assert_eq!(chord.key(VK_E, true, false), Action::Pass);
            assert_eq!(chord.key(VK_E, false, false), Action::Pass);
            assert_eq!(chord.key(modifier, false, false), Action::Pass);
        }
        assert_eq!(chord.key(VK_E, true, true), Action::Pass);
        assert_eq!(chord.key(VK_E, false, false), Action::Pass);
        assert_eq!(chord.key(VK_E, true, false), Action::Open);
    }

    #[test]
    fn does_not_turn_a_passed_through_press_into_a_shortcut_on_repeat() {
        let mut chord = Chord::default();
        assert_eq!(chord.key(VK_E, true, false), Action::Pass);
        assert_eq!(chord.key(VK_LWIN, true, false), Action::Pass);
        assert_eq!(chord.key(VK_E, true, false), Action::Pass);
        assert_eq!(chord.key(VK_E, false, false), Action::Pass);
        assert_eq!(chord.key(VK_E, true, false), Action::Open);
        assert_eq!(chord.key(VK_E, true, false), Action::Pass);
        assert_eq!(chord.key(VK_E, false, false), Action::Pass);
        assert_eq!(chord.key(VK_E, true, false), Action::Open);
    }

    #[test]
    fn recovers_after_a_release_on_a_different_desktop() {
        let mut chord = Chord::default();
        chord.key(VK_LWIN, true, false);
        assert_eq!(chord.key(VK_E, true, false), Action::Open);
        chord.suppress_e = true;
        chord.update_modifiers(0);
        assert_eq!(chord.key(VK_E, true, false), Action::Pass);
        chord.key(VK_E, false, false);
        chord.key(VK_LWIN, true, false);
        assert_eq!(chord.key(VK_E, true, false), Action::Open);
    }

    #[test]
    fn passive_listener_lifecycle() -> io::Result<()> {
        const PROBE_CLASS: &str = "BETTER_EXPLORER_TEST_WIN_E_CLASS";
        let exe = env::current_exe()?;
        if let Ok(class) = env::var(PROBE_CLASS) {
            let _hook = Hook::install()?;
            make_message_window(&class, exe.as_os_str())?;
            return message_loop();
        }

        let class = format!("BetterExplorerWinEProbe-{}", std::process::id());
        let mut child = Command::new(&exe)
            .args(["--exact", "hotkey::tests::passive_listener_lifecycle"])
            .env(PROBE_CLASS, &class)
            .stdout(std::process::Stdio::null())
            .spawn()?;
        let result = (|| {
            let deadline = Instant::now() + Duration::from_millis(STARTUP_WAIT_MS.into());
            loop {
                match find_listener_for_class(&exe, &class) {
                    Listener::Ours { window, process } => {
                        if unsafe { PostMessageW(window, WM_CLOSE, 0, 0) } == 0 {
                            return Err(io::Error::last_os_error());
                        }
                        return match unsafe { WaitForSingleObject(process.0, STARTUP_WAIT_MS) } {
                            WAIT_OBJECT_0 => Ok(()),
                            WAIT_TIMEOUT => Err(timed_out("The passive hook did not stop.")),
                            _ => Err(io::Error::last_os_error()),
                        };
                    }
                    Listener::Other => return Err(other_copy()),
                    Listener::Missing => {}
                }
                if let Some(status) = child.try_wait()? {
                    return Err(io::Error::other(format!(
                        "The passive hook exited ({status})."
                    )));
                }
                if Instant::now() >= deadline {
                    return Err(timed_out("The passive hook did not start."));
                }
                thread::sleep(POLL_INTERVAL);
            }
        })();
        if result.is_err() {
            let _ = child.kill();
        }
        let status = child.wait()?;
        result?;
        assert!(status.success());
        assert!(matches!(
            find_listener_for_class(&exe, &class),
            Listener::Missing
        ));
        Ok(())
    }
}
