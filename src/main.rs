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
mod file_name_tip;
#[cfg(windows)]
mod folder_command;
#[cfg(windows)]
mod folder_manager;
#[cfg(windows)]
mod home_page;
#[cfg(windows)]
mod hotkey;
#[cfg(windows)]
mod preview;
#[cfg(windows)]
mod settings_page;
#[cfg(windows)]
mod theme_visuals;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod win_e_task;
#[cfg(windows)]
mod window_geometry;

#[cfg(windows)]
fn main() {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_deref() == Some(std::ffi::OsStr::new(folder_command::SERVER_ARG)) {
        if let Err(error) = folder_command::run() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(exit) = first.as_deref().and_then(|arg| hotkey::cli(arg, &mut args)) {
        std::process::exit(exit);
    }
    if let Some(exit) = first
        .as_deref()
        .and_then(|arg| folder_manager::cli(arg, &mut args))
    {
        std::process::exit(exit);
    }
    let (path, selection) = launch_paths(first.into_iter().chain(args));
    hotkey::resume_on_launch();
    if let Err(error) = ui::run(path, selection) {
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

#[cfg(windows)]
fn launch_paths(
    args: impl Iterator<Item = std::ffi::OsString>,
) -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
    let mut args = args;
    let first = args.find(|arg| arg != "--new-window");
    let absolute = |arg| std::path::absolute(std::path::PathBuf::from(arg)).ok();
    if first.as_deref() == Some(std::ffi::OsStr::new("--select")) {
        let selection = args.next().and_then(absolute);
        let parent = selection
            .as_deref()
            .and_then(std::path::Path::parent)
            .map(std::path::Path::to_path_buf);
        return if parent.is_some() {
            (parent, selection)
        } else {
            (selection, None)
        };
    }
    let path = if first.as_deref() == Some(std::ffi::OsStr::new("--open-folder")) {
        args.next()
    } else {
        first
    };
    (path.and_then(absolute), None)
}

#[cfg(all(windows, test))]
mod tests {
    use super::launch_paths;
    use std::path::PathBuf;

    #[test]
    fn launches_containing_folder_for_selection_and_accepts_shell_drive_paths() {
        let parse = |args: &[&str]| launch_paths(args.iter().map(|arg| (*arg).into()));
        assert_eq!(
            parse(&["--select", r"C:\Some Folder\report.pdf"]),
            (
                Some(PathBuf::from(r"C:\Some Folder")),
                Some(PathBuf::from(r"C:\Some Folder\report.pdf")),
            )
        );
        assert_eq!(
            parse(&["--open-folder", r"C:\\."]),
            (Some(PathBuf::from(r"C:\")), None)
        );
        assert_eq!(
            parse(&["--new-window", r"C:\Some Folder"]),
            (Some(PathBuf::from(r"C:\Some Folder")), None)
        );
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("BetterExplorer runs on Windows.");
}
