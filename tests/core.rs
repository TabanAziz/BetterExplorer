#![cfg(windows)]

use better_explorer::model::{safe_name, History, Location};
use better_explorer::work::{scan, Request, ScanEvent, Sort};
use std::sync::atomic::AtomicU64;

#[test]
fn history_discards_forward_locations_after_new_navigation() {
    let mut history = History::new(Location::Home);
    history.open(Location::path(r"C:\one"));
    history.open(Location::path(r"C:\two"));
    assert_eq!(history.back(), Some(&Location::path(r"C:\one")));
    history.open(Location::path(r"C:\three"));
    assert_eq!(history.forward(), None);
    assert_eq!(history.current(), &Location::path(r"C:\three"));
}

#[test]
fn new_names_cannot_escape_the_destination() {
    for invalid in [
        "",
        ".",
        "..",
        "a/b",
        "a\\b",
        "a:stream",
        "trailing.",
        "trailing ",
        "CON.txt",
        "LPT¹",
    ] {
        assert!(!safe_name(invalid), "{invalid}");
    }
    assert!(safe_name("Überblick 📁.txt"));
}

#[test]
fn scan_yields_progress_and_returns_unicode_names_in_natural_order() {
    let path = std::env::temp_dir().join(format!("BetterExplorer-core-{}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let _cleanup = Cleanup(path.clone());
    for name in ["Datei 10.txt", "Datei 2.txt", "Öffnen 📁.txt"] {
        std::fs::write(path.join(name), b"fixture").unwrap();
    }
    let request = Request {
        id: 1,
        location: Location::Folder(path),
        hidden: false,
        sort: Sort::default(),
        pins: Vec::new(),
    };
    let mut events = Vec::new();
    scan(request, &AtomicU64::new(1), |event| events.push(event));
    assert!(matches!(events.first(), Some(ScanEvent::Batch(..))));
    let Some(ScanEvent::Done(_, entries, None)) = events.last() else {
        panic!("no sorted result")
    };
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.label())
            .collect::<Vec<_>>(),
        ["Datei 2.txt", "Datei 10.txt", "Öffnen 📁.txt"]
    );
}

struct Cleanup(std::path::PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn settings_tab_is_not_saved_as_a_folder_to_restore() {
    use better_explorer::model::Settings;
    let first = Location::path(r"C:\Projects");
    let second = Location::path(r"C:\Photos");
    let mut settings = Settings::default();

    settings.remember_tabs([first.clone(), Location::Settings, second.clone()], 1);
    assert_eq!(settings.tabs, [first, second.clone()]);
    assert_eq!(settings.active, 0);

    settings.remember_tabs([Location::Settings, second.clone()], 1);
    assert_eq!(settings.tabs, [second]);
    assert_eq!(settings.active, 0);

    settings.remember_tabs([Location::Settings], 0);
    assert!(settings.tabs.is_empty());
    assert_eq!(settings.active, 0);
}

fn entry_at_local_date(month: u16, day: u16) -> better_explorer::work::Entry {
    use better_explorer::model::wide;
    use better_explorer::work::Entry;
    use windows_sys::Win32::{
        Foundation::{FILETIME, SYSTEMTIME},
        System::Time::{SystemTimeToFileTime, TzSpecificLocalTimeToSystemTimeEx},
    };
    let date = SYSTEMTIME {
        wYear: 2026,
        wMonth: month,
        wDay: day,
        wHour: 17,
        wMinute: 5,
        ..unsafe { std::mem::zeroed() }
    };
    let mut converted: SYSTEMTIME = unsafe { std::mem::zeroed() };
    let mut utc: FILETIME = unsafe { std::mem::zeroed() };
    assert_ne!(
        unsafe { TzSpecificLocalTimeToSystemTimeEx(std::ptr::null(), &date, &mut converted) },
        0
    );
    assert_ne!(unsafe { SystemTimeToFileTime(&converted, &mut utc) }, 0);
    Entry {
        path: "example.txt".into(),
        name: wide("example.txt"),
        extension: wide("txt"),
        directory: false,
        size: 0,
        modified: u64::from(utc.dwHighDateTime) << 32 | u64::from(utc.dwLowDateTime),
        location: false,
        capacity: None,
        shell_item: None,
    }
}

#[test]
fn date_modified_formats_local_time_day_first_by_default() {
    use better_explorer::model::Settings;
    let entry = entry_at_local_date(2, 14);
    assert_eq!(
        entry.date(Settings::default().modified_date_format),
        "14.02.2026 17:05"
    );
}

#[test]
fn existing_settings_without_a_date_format_use_day_first() {
    use better_explorer::model::{ModifiedDateFormat, Settings};
    let settings: Settings = serde_json::from_str(r#"{"details":true}"#).unwrap();
    assert!(settings.details);
    assert_eq!(
        settings.modified_date_format,
        ModifiedDateFormat::DayMonthYear
    );
}

#[test]
fn date_modified_formats_local_time_month_first_using_saved_choice() {
    use better_explorer::model::{ModifiedDateFormat, Settings};
    let settings = Settings {
        modified_date_format: ModifiedDateFormat::MonthDayYear,
        ..Settings::default()
    };
    let saved = serde_json::to_vec(&settings).unwrap();
    let restored: Settings = serde_json::from_slice(&saved).unwrap();
    assert_eq!(
        entry_at_local_date(2, 14).date(restored.modified_date_format),
        "02/14/2026 17:05"
    );
}

#[test]
fn date_modified_sort_uses_timestamps_across_month_boundaries() {
    use better_explorer::model::ModifiedDateFormat;
    use better_explorer::work::{sort_entries, Column};
    use std::sync::Arc;
    let mut entries = [
        Arc::new(entry_at_local_date(2, 14)),
        Arc::new(entry_at_local_date(1, 28)),
    ];
    sort_entries(
        &mut entries,
        Sort {
            column: Column::Modified,
            descending: false,
        },
    );
    assert_eq!(
        entries.map(|entry| entry.date(ModifiedDateFormat::DayMonthYear)),
        ["28.01.2026 17:05", "14.02.2026 17:05"]
    );
}

#[test]
fn regular_folders_group_by_modified_date_until_overridden() {
    use better_explorer::model::{FolderOrder, GroupBy, Settings};
    use better_explorer::work::Column;
    use std::path::Path;

    let downloads = Path::new(r"C:\Users\Example\Downloads");
    let projects = Path::new(r"C:\Projects");
    let mut settings: Settings = serde_json::from_str(r#"{"details":true}"#).unwrap();
    let projects_default = settings.folder_order(projects);
    assert_eq!(projects_default.sort.column, Column::Modified);
    assert!(projects_default.sort.descending);
    assert_eq!(projects_default.group, GroupBy::DateModified);
    assert_eq!(settings.folder_order(downloads), projects_default);

    settings.folder_orders.insert(
        downloads.into(),
        FolderOrder {
            sort: Sort {
                column: Column::Name,
                descending: false,
            },
            group: GroupBy::None,
        },
    );
    let restored: Settings =
        serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
    assert_eq!(restored.folder_order(downloads).sort, Sort::default());
    assert_eq!(restored.folder_order(downloads).group, GroupBy::None);
    assert_eq!(restored.folder_order(projects), projects_default);
}
