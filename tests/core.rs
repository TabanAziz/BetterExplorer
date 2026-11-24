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
