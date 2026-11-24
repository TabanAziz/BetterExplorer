#![cfg(windows)]

use better_explorer::shell::{History, Operation};
use std::fs;

#[test]
fn rename_undo_redo_preserves_file_and_handles_case_changes() {
    let fixture = tempfile::tempdir().unwrap();
    let original = fixture.path().join("Résumé.txt");
    let renamed = fixture.path().join("RÉSUMÉ.txt");
    fs::write(&original, b"same file").unwrap();
    let mut history = History::default();
    history
        .run(
            0,
            Operation::Rename {
                path: original.clone(),
                name: "RÉSUMÉ.txt".into(),
            },
        )
        .unwrap();
    assert_eq!(history.undo_label(), Some("Rename"));
    assert_eq!(
        fs::read_dir(fixture.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path(),
        renamed
    );
    history.run(0, Operation::Undo).unwrap();
    assert_eq!(history.redo_label(), Some("Rename"));
    assert_eq!(
        fs::read_dir(fixture.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path(),
        original
    );
    history.run(0, Operation::Redo).unwrap();
    assert_eq!(
        fs::read_dir(fixture.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path(),
        renamed
    );
    assert_eq!(fs::read(renamed).unwrap(), b"same file");
}

#[test]
fn undo_move_refuses_occupied_destination_and_can_be_retried() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source.txt");
    let target = fixture.path().join("target");
    fs::create_dir(&target).unwrap();
    let moved = target.join("source.txt");
    fs::write(&source, b"original").unwrap();
    let mut history = History::default();
    history
        .run(
            0,
            Operation::Transfer {
                paths: vec![source.clone()],
                target,
                moving: true,
            },
        )
        .unwrap();
    assert_eq!(history.undo_label(), Some("Move"));
    fs::write(&source, b"different file").unwrap();
    assert!(history.run(0, Operation::Undo).is_err());
    assert_eq!(fs::read(&source).unwrap(), b"different file");
    assert_eq!(fs::read(&moved).unwrap(), b"original");
    assert_eq!(history.undo_label(), Some("Move"));

    fs::remove_file(&source).unwrap();
    history.run(0, Operation::Undo).unwrap();
    assert_eq!(fs::read(&source).unwrap(), b"original");
    assert!(!moved.exists());
    history.run(0, Operation::Redo).unwrap();
    assert_eq!(fs::read(&moved).unwrap(), b"original");
    assert!(!source.exists());
}

#[test]
fn create_and_copy_redo_restore_the_original_outputs() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let mut history = History::default();
    let folder = root.join("New folder");
    history
        .run(
            0,
            Operation::Create {
                target: root.to_owned(),
                name: "New folder".into(),
                directory: true,
            },
        )
        .unwrap();
    assert!(folder.is_dir());
    history.run(0, Operation::Undo).unwrap();
    assert!(!folder.exists());
    history.run(0, Operation::Redo).unwrap();
    assert!(folder.is_dir());

    let source = root.join("source.txt");
    let copied = folder.join("source.txt");
    fs::write(&source, b"copied payload").unwrap();
    history
        .run(
            0,
            Operation::Transfer {
                paths: vec![source.clone()],
                target: folder,
                moving: false,
            },
        )
        .unwrap();
    history.run(0, Operation::Undo).unwrap();
    assert!(!copied.exists());
    fs::write(&source, b"source changed").unwrap();
    history.run(0, Operation::Redo).unwrap();
    assert_eq!(fs::read(&copied).unwrap(), b"copied payload");
    assert_eq!(fs::read(&source).unwrap(), b"source changed");
}

#[test]
fn recycle_undo_redo_targets_the_exact_entry_and_refuses_a_replacement() {
    let fixture = tempfile::tempdir().unwrap();
    let file = fixture.path().join("Delete me 🗑.txt");
    fs::write(&file, b"historical file").unwrap();
    let mut history = History::default();
    history
        .run(
            0,
            Operation::Delete {
                paths: vec![file.clone()],
                permanent: false,
            },
        )
        .unwrap();
    assert!(!file.exists());
    assert_eq!(history.undo_label(), Some("Delete"));
    history.run(0, Operation::Undo).unwrap();
    assert_eq!(fs::read(&file).unwrap(), b"historical file");
    history.run(0, Operation::Redo).unwrap();
    assert!(!file.exists());
    history.run(0, Operation::Undo).unwrap();
    assert_eq!(fs::read(&file).unwrap(), b"historical file");

    let former = fixture.path().join("Moved aside.txt");
    fs::rename(&file, &former).unwrap();
    fs::write(&file, b"replacement").unwrap();
    assert!(history.run(0, Operation::Redo).is_err());
    assert_eq!(fs::read(&file).unwrap(), b"replacement");
    assert_eq!(fs::read(&former).unwrap(), b"historical file");
}

#[test]
fn undo_keeps_a_created_file_that_has_since_been_edited() {
    let fixture = tempfile::tempdir().unwrap();
    let original = fixture.path().join("New file.txt");
    let mut history = History::default();
    history
        .run(
            0,
            Operation::Create {
                target: fixture.path().to_owned(),
                name: "New file.txt".into(),
                directory: false,
            },
        )
        .unwrap();
    fs::write(&original, b"new contents").unwrap();
    assert!(history.run(0, Operation::Undo).is_err());
    assert_eq!(fs::read(&original).unwrap(), b"new contents");
    assert!(history.redo_label().is_none());
}

#[test]
fn recycle_undo_preserves_each_side_of_a_partially_blocked_batch() {
    let fixture = tempfile::tempdir().unwrap();
    let first = fixture.path().join("first.txt");
    let second = fixture.path().join("second.txt");
    fs::write(&first, b"first").unwrap();
    fs::write(&second, b"second").unwrap();
    let mut history = History::default();
    history
        .run(
            0,
            Operation::Delete {
                paths: vec![first.clone(), second.clone()],
                permanent: false,
            },
        )
        .unwrap();
    fs::write(&first, b"unrelated").unwrap();
    assert!(history.run(0, Operation::Undo).is_err());
    assert_eq!(fs::read(&first).unwrap(), b"unrelated");
    assert_eq!(fs::read(&second).unwrap(), b"second");
    assert_eq!(history.undo_label(), Some("Delete"));
    assert_eq!(history.redo_label(), Some("Delete"));

    history.run(0, Operation::Redo).unwrap();
    assert!(!second.exists());
    fs::remove_file(&first).unwrap();
    history.run(0, Operation::Undo).unwrap();
    history.run(0, Operation::Undo).unwrap();
    assert_eq!(fs::read(&first).unwrap(), b"first");
    assert_eq!(fs::read(&second).unwrap(), b"second");
}

#[test]
fn same_folder_copy_undo_uses_the_name_chosen_by_windows() {
    let fixture = tempfile::tempdir().unwrap();
    let original = fixture.path().join("Same folder.txt");
    fs::write(&original, b"source").unwrap();
    let mut history = History::default();
    history
        .run(
            0,
            Operation::Transfer {
                paths: vec![original.clone()],
                target: fixture.path().to_owned(),
                moving: false,
            },
        )
        .unwrap();
    let copy = fs::read_dir(fixture.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|item| item != &original)
        .unwrap();
    history.run(0, Operation::Undo).unwrap();
    assert!(!copy.exists());
    assert_eq!(fs::read(&original).unwrap(), b"source");
    history.run(0, Operation::Redo).unwrap();
    assert_eq!(fs::read(&copy).unwrap(), b"source");
}
