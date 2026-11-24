#![cfg(windows)]

use better_explorer::archive::{
    compress, compress_with, extract, extract_with, Compression, Format, Level,
};
use better_explorer::shell::{History, Operation};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

#[test]
fn recorded_quick_archive_actions_undo_the_actual_nonconflicting_outputs() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let source = root.join("Draft.v2.txt");
    let occupied_archive = root.join("Draft.v2.tar");
    let occupied_folder = root.join("Draft.v2 (2)");
    let archive = root.join("Draft.v2 (2).tar");
    let extracted = root.join("Draft.v2 (2) (2)");
    fs::write(&source, b"source text").unwrap();
    fs::write(&occupied_archive, b"older archive").unwrap();
    fs::create_dir(&occupied_folder).unwrap();
    fs::write(occupied_folder.join("keep.txt"), b"older folder").unwrap();

    let mut history = History::default();
    history
        .run(
            0,
            Operation::Compress {
                sources: vec![source],
                output: occupied_archive.clone(),
                options: Compression {
                    format: Format::Tar,
                    level: Level::Fast,
                },
                automatic: true,
            },
        )
        .unwrap();
    assert!(archive.is_file());
    assert_eq!(history.undo_label(), Some("Compress"));
    history
        .run(
            0,
            Operation::Extract {
                source: archive.clone(),
                output: occupied_folder.clone(),
                automatic: true,
            },
        )
        .unwrap();
    assert_eq!(
        fs::read(extracted.join("Draft.v2.txt")).unwrap(),
        b"source text"
    );
    assert_eq!(history.undo_label(), Some("Extract"));

    history.run(0, Operation::Undo).unwrap();
    assert!(!extracted.exists());
    history.run(0, Operation::Undo).unwrap();
    assert!(!archive.exists());
    assert_eq!(fs::read(occupied_archive).unwrap(), b"older archive");
    assert_eq!(
        fs::read(occupied_folder.join("keep.txt")).unwrap(),
        b"older folder"
    );
    history.run(0, Operation::Redo).unwrap();
    history.run(0, Operation::Redo).unwrap();
    assert_eq!(
        fs::read(extracted.join("Draft.v2.txt")).unwrap(),
        b"source text"
    );
}

#[test]
fn quick_formats_preserve_contents_and_pick_conflict_free_names() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    let source = root.join("project.v1");
    fs::create_dir_all(source.join("empty.dir")).unwrap();
    fs::write(source.join("Überblick 📁.txt"), b"text").unwrap();
    fs::write(source.join("empty.txt"), b"").unwrap();

    for format in Format::ALL {
        let output = root.join(format!("project.v1.{}", format.extension()));
        fs::write(&output, b"keep archive").unwrap();
        let options = Compression {
            format,
            level: Level::Fast,
        };
        let archive = compress_with(
            std::slice::from_ref(&source),
            &output,
            options,
            true,
            |_| {},
        )
        .unwrap();
        assert_eq!(
            archive.file_name().unwrap(),
            format!("project.v1 (2).{}", format.extension()).as_str()
        );
        assert_eq!(fs::read(&output).unwrap(), b"keep archive");

        let extracted = extract_with(&archive, &source, true, |_| {}).unwrap();
        assert_ne!(extracted.file_name(), source.file_name());
        let extracted = extracted.join("project.v1");
        assert_eq!(
            fs::read(extracted.join("Überblick 📁.txt")).unwrap(),
            b"text"
        );
        assert_eq!(fs::read(extracted.join("empty.txt")).unwrap(), b"");
        assert!(extracted.join("empty.dir").is_dir());
    }
}

#[test]
fn quick_archive_retries_a_destination_claimed_during_publication() {
    let workspace = tempdir().unwrap();
    let file = workspace.path().join("report.txt");
    let output = workspace.path().join("report.zip");
    fs::write(&file, b"report").unwrap();
    let mut attempts = Vec::new();
    let archive = compress_with(&[file], &output, Compression::default(), true, |path| {
        attempts.push(path.file_name().unwrap().to_owned());
        if attempts.len() == 1 {
            fs::write(path, b"keep me").unwrap();
        }
    })
    .unwrap();
    assert_eq!(attempts, ["report.zip", "report (2).zip"]);
    assert_eq!(fs::read(&output).unwrap(), b"keep me");
    assert!(archive.is_file());
}

#[test]
fn zip_round_trip_keeps_selected_roots_contents_and_empty_directories() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    let folder = root.join("Unterlagen");
    let sibling = root.join("notiz.txt");
    let archive = root.join("Gesammelt.zip");
    let output = root.join("Entpackt");
    let contents = [0, 1, 2, 255];

    fs::create_dir_all(folder.join("Leer")).unwrap();
    fs::write(folder.join("Überblick 📁.bin"), contents).unwrap();
    fs::write(&sibling, b"Text").unwrap();
    compress(&[folder, sibling], &archive).unwrap();

    let mut stored = ZipArchive::new(File::open(&archive).unwrap()).unwrap();
    assert_eq!(
        stored.by_name("notiz.txt").unwrap().compression(),
        CompressionMethod::Deflated
    );

    extract(&archive, &output).unwrap();
    assert_eq!(
        fs::read(output.join("Unterlagen").join("Überblick 📁.bin")).unwrap(),
        contents
    );
    assert_eq!(fs::read(output.join("notiz.txt")).unwrap(), b"Text");
    assert_eq!(
        fs::read(zone_stream(&output.join("notiz.txt")))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::NotFound
    );
    assert_eq!(
        fs::read_dir(output.join("Unterlagen").join("Leer"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn extract_propagates_the_archive_zone_identifier_to_every_file() {
    let workspace = tempdir().unwrap();
    let archive = workspace.path().join("Heruntergeladen 📁.zip");
    let output = workspace.path().join("output");
    let mark = b"[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://download.example/fixture.zip\r\nReferrerUrl=https://download.example/\r\n";
    let payload = b"@echo fixture";
    write_archive(&archive, &[("file.cmd", payload), ("nested/leer.txt", b"")]);
    let original = fs::read(&archive).unwrap();
    let mixed_case_mark = named_stream(&archive, ":zone.IDENTIFIER:$DATA");
    fs::write(&mixed_case_mark, mark).unwrap();
    let other_stream = named_stream(&archive, ":other:$DATA");
    fs::write(&other_stream, b"unrelated").unwrap();

    extract(&archive, &output).unwrap();
    for (name, content) in [("file.cmd", &payload[..]), ("nested/leer.txt", &b""[..])] {
        let file = output.join(name);
        assert_eq!(fs::read(&file).unwrap(), content);
        assert_eq!(fs::read(zone_stream(&file)).unwrap(), mark);
        assert!(!named_stream(&file, ":other:$DATA").exists());
    }
    assert_eq!(fs::read(&archive).unwrap(), original);
    assert_eq!(fs::read(&mixed_case_mark).unwrap(), mark);
    assert_eq!(fs::read(other_stream).unwrap(), b"unrelated");
}

#[test]
fn extract_aborts_when_the_zone_identifier_cannot_be_read() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    let archive = root.join("download.zip");
    let output = root.join("output");
    write_archive(&archive, &[("file.cmd", b"@echo fixture")]);
    let stream = zone_stream(&archive);
    fs::write(&stream, b"[ZoneTransfer]\r\nZoneId=3\r\n").unwrap();
    let writer = File::options().write(true).open(&stream).unwrap();

    assert!(extract(&archive, &output).is_err());
    assert!(!output.exists());
    assert_eq!(fs::read_dir(root).unwrap().count(), 1);
    drop(writer);
    extract(&archive, &output).unwrap();
    assert_eq!(
        fs::read(zone_stream(&output.join("file.cmd"))).unwrap(),
        b"[ZoneTransfer]\r\nZoneId=3\r\n"
    );
}

#[test]
fn extract_accepts_windows_separators_and_directory_entries_after_their_children() {
    let workspace = tempdir().unwrap();
    let archive = workspace.path().join("fixture.zip");
    let output = workspace.path().join("output");
    let mut writer = ZipWriter::new(File::create(&archive).unwrap());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    writer.start_file(r"nested\item.txt", options).unwrap();
    writer.write_all(b"fixture").unwrap();
    writer.add_directory("nested/", options).unwrap();
    writer.finish().unwrap();

    extract(&archive, &output).unwrap();
    assert_eq!(
        fs::read(output.join("nested").join("item.txt")).unwrap(),
        b"fixture"
    );
}

#[test]
fn extract_rejects_unsafe_names_and_cleans_staging() {
    for name in [
        "../../outside.txt",
        "nested/../outside.txt",
        "/absolute.txt",
        r"C:\absolute.txt",
        "folder/item.txt:stream",
        "folder/CON.txt",
        "folder/trailing.",
        "folder//item.txt",
    ] {
        let workspace = tempdir().unwrap();
        let root = workspace.path();
        let archive = root.join("fixture.zip");
        let output = root.join("output");
        write_archive(&archive, &[("valid.txt", b"before"), (name, b"invalid")]);

        assert!(extract(&archive, &output).is_err(), "{name}");
        assert!(!output.exists(), "{name}");
        assert_eq!(fs::read_dir(root).unwrap().count(), 1, "{name}");
    }
}

#[test]
fn extract_rejects_file_folder_and_windows_name_collisions() {
    for (first, second) in [
        ("report.txt", "REPORT.TXT"),
        ("nested/file.txt", "NESTED/other.txt"),
        ("flat", "flat/file.txt"),
        ("flat/file.txt", "flat"),
    ] {
        let workspace = tempdir().unwrap();
        let root = workspace.path();
        let archive = root.join("fixture.zip");
        let output = root.join("output");
        write_archive(&archive, &[(first, b"a"), (second, b"b")]);

        assert!(extract(&archive, &output).is_err(), "{first} / {second}");
        assert!(!output.exists());
        assert_eq!(fs::read_dir(root).unwrap().count(), 1);
    }
}

#[test]
fn extract_rejects_zip_links() {
    let workspace = tempdir().unwrap();
    let archive = workspace.path().join("fixture.zip");
    let output = workspace.path().join("output");
    let mut writer = ZipWriter::new(File::create(&archive).unwrap());
    writer
        .add_symlink("link", "outside", SimpleFileOptions::default())
        .unwrap();
    writer.finish().unwrap();

    assert!(extract(&archive, &output).is_err());
    assert!(!output.exists());
    assert_eq!(fs::read_dir(workspace.path()).unwrap().count(), 1);
}

#[test]
fn extract_rejects_exact_duplicate_names_in_the_zip_directory() {
    let workspace = tempdir().unwrap();
    let archive = workspace.path().join("fixture.zip");
    let output = workspace.path().join("output");
    write_archive(&archive, &[("one.txt", b"first"), ("two.txt", b"second")]);
    let mut bytes = fs::read(&archive).unwrap();
    let positions: Vec<_> = bytes
        .windows(b"two.txt".len())
        .enumerate()
        .filter_map(|(offset, window)| (window == b"two.txt").then_some(offset))
        .collect();
    assert_eq!(positions.len(), 2);
    for offset in positions {
        bytes[offset..offset + b"one.txt".len()].copy_from_slice(b"one.txt");
    }
    fs::write(&archive, bytes).unwrap();

    assert!(extract(&archive, &output).is_err());
    assert!(!output.exists());
    assert_eq!(fs::read_dir(workspace.path()).unwrap().count(), 1);
}

#[test]
fn extract_discards_staging_when_entry_contents_fail_crc_check() {
    let workspace = tempdir().unwrap();
    let archive = workspace.path().join("fixture.zip");
    let output = workspace.path().join("output");
    let payload = b"distinctive entry contents";
    write_archive(&archive, &[("valid.txt", b"before"), ("bad.txt", payload)]);
    let mut bytes = fs::read(&archive).unwrap();
    let payload_start = bytes
        .windows(payload.len())
        .position(|window| window == payload)
        .unwrap();
    bytes[payload_start] ^= 1;
    fs::write(&archive, bytes).unwrap();

    assert!(extract(&archive, &output).is_err());
    assert!(!output.exists());
    assert_eq!(fs::read_dir(workspace.path()).unwrap().count(), 1);
}

#[test]
fn archive_operations_preserve_existing_outputs() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    let source = root.join("source.txt");
    let archive = root.join("existing.zip");
    let output = root.join("existing");
    fs::write(&source, b"new").unwrap();
    write_archive(&archive, &[("same.txt", b"in archive")]);
    fs::create_dir(&output).unwrap();
    fs::write(output.join("same.txt"), b"keep").unwrap();
    let previous_archive = fs::read(&archive).unwrap();

    assert!(compress(&[source], &archive).is_err());
    assert_eq!(fs::read(&archive).unwrap(), previous_archive);
    assert!(extract(&archive, &output).is_err());
    assert_eq!(fs::read(output.join("same.txt")).unwrap(), b"keep");
}

#[test]
fn compress_rejects_junctions_inside_selected_folders() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    let folder = root.join("selected");
    let outside = root.join("outside");
    let archive = root.join("fixture.zip");
    fs::create_dir(&folder).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("fixture.txt"), b"outside the selection").unwrap();
    let command = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$ErrorActionPreference = 'Stop'; New-Item -ItemType Junction -Path $env:BEX_ZIP_TEST_LINK -Target $env:BEX_ZIP_TEST_TARGET | Out-Null",
        ])
        .env("BEX_ZIP_TEST_LINK", folder.join("link"))
        .env("BEX_ZIP_TEST_TARGET", &outside)
        .output()
        .unwrap();
    assert!(
        command.status.success(),
        "{}",
        String::from_utf8_lossy(&command.stderr)
    );

    assert!(compress(&[folder], &archive).is_err());
    assert!(!archive.exists());
    assert_eq!(fs::read_dir(root).unwrap().count(), 2);
}

#[test]
fn compress_rejects_duplicate_root_names_and_cleans_staging() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    let first = root.join("first");
    let second = root.join("second");
    let archive = root.join("fixture.zip");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&second).unwrap();
    fs::write(first.join("same.txt"), b"first").unwrap();
    fs::write(second.join("SAME.TXT"), b"second").unwrap();

    assert!(compress(&[first.join("same.txt"), second.join("SAME.TXT")], &archive).is_err());
    assert!(!archive.exists());
    assert_eq!(fs::read_dir(root).unwrap().count(), 2);
}

fn write_archive(path: &Path, entries: &[(&str, &[u8])]) {
    let mut writer = ZipWriter::new(File::create(path).unwrap());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, contents) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(contents).unwrap();
    }
    writer.finish().unwrap();
}

fn zone_stream(path: &Path) -> PathBuf {
    named_stream(path, ":Zone.Identifier:$DATA")
}

fn named_stream(path: &Path, name: &str) -> PathBuf {
    let mut stream = path.as_os_str().to_owned();
    stream.push(name);
    stream.into()
}
