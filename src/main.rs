#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod benchmark;
#[cfg(windows)]
mod breadcrumb;
#[cfg(windows)]
mod chrome;
#[cfg(windows)]
mod file_icon_cache;
#[cfg(windows)]
mod file_icons;
#[cfg(windows)]
mod home_page;
#[cfg(windows)]
mod hotkey;
#[cfg(windows)]
mod preview;
#[cfg(windows)]
mod settings_page;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod window_geometry;

#[cfg(windows)]
fn main() {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if let Some(exit) = first.as_deref().and_then(hotkey::cli) {
        std::process::exit(exit);
    }
    let path = first
        .into_iter()
        .chain(args)
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
