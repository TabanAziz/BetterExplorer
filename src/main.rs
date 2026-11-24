#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod benchmark;
#[cfg(windows)]
mod chrome;
#[cfg(windows)]
mod preview;
#[cfg(windows)]
mod ui;

#[cfg(windows)]
fn main() {
    let path = std::env::args_os()
        .skip(1)
        .find(|arg| arg != "--new-window")
        .map(std::path::PathBuf::from)
        .and_then(|path| std::path::absolute(path).ok());
    if let Err(error) = ui::run(path) {
        let error =
            better_explorer::model::wide(format!("BetterExplorer could not start.\n\n{error}"));
        let title = better_explorer::model::wide("BetterExplorer");
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR};
            MessageBoxW(
                std::ptr::null_mut(),
                error.as_ptr(),
                title.as_ptr(),
                MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("BetterExplorer runs on Windows.");
}
