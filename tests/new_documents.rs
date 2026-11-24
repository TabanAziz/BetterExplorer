#![cfg(windows)]

use better_explorer::{
    model::wide,
    new_document::{available_documents, load_catalog, DocumentKind},
    shell::{History, Operation},
};
use std::{
    fs::{self, File},
    ptr::null_mut,
};
use windows_sys::Win32::{
    Graphics::Gdi::DeleteObject,
    UI::WindowsAndMessaging::{LoadImageW, IMAGE_BITMAP, LR_CREATEDIBSECTION, LR_LOADFROMFILE},
};
use zip::ZipArchive;

#[test]
fn creates_the_selected_document_formats() {
    let fixture = tempfile::tempdir().unwrap();
    let mut history = History::default();
    let formats = [
        (DocumentKind::Bitmap, "BM"),
        (DocumentKind::Word, "PK"),
        (DocumentKind::PowerPoint, "PK"),
        (DocumentKind::Python, ""),
        (DocumentKind::RichText, "{\\rtf"),
        (DocumentKind::Text, ""),
        (DocumentKind::Excel, "PK"),
        (DocumentKind::Zip, "PK"),
    ];
    for (kind, magic) in formats {
        assert!(available_documents().any(|available| available == kind));
        let name = format!("Blank{}", kind.extension());
        history
            .run(
                0,
                Operation::CreateDocument {
                    target: fixture.path().to_owned(),
                    name: name.clone(),
                    template: kind,
                },
            )
            .unwrap();
        let path = fixture.path().join(name);
        assert!(fs::read(&path).unwrap().starts_with(magic.as_bytes()));
        let package_part = match kind {
            DocumentKind::Word => "word/document.xml",
            DocumentKind::PowerPoint => "ppt/presentation.xml",
            DocumentKind::Excel => "xl/workbook.xml",
            DocumentKind::Zip => {
                ZipArchive::new(File::open(path).unwrap()).unwrap();
                continue;
            }
            DocumentKind::Bitmap => {
                let path = wide(path);
                let bitmap = unsafe {
                    LoadImageW(
                        null_mut(),
                        path.as_ptr(),
                        IMAGE_BITMAP,
                        0,
                        0,
                        LR_LOADFROMFILE | LR_CREATEDIBSECTION,
                    )
                };
                assert!(!bitmap.is_null());
                unsafe { DeleteObject(bitmap) };
                continue;
            }
            _ => continue,
        };
        let mut zip = ZipArchive::new(File::open(path).unwrap()).unwrap();
        assert!(zip.by_name(package_part).unwrap().size() > 0);
    }
}

#[test]
fn installed_access_templates_produce_access_databases() {
    load_catalog();
    let fixture = tempfile::tempdir().unwrap();
    let mut history = History::default();
    for kind in available_documents() {
        let signature = match kind {
            DocumentKind::Access => b"Standard ACE DB",
            DocumentKind::AccessLegacy => b"Standard Jet DB",
            _ => continue,
        };
        let name = format!("Blank{}", kind.extension());
        history
            .run(
                0,
                Operation::CreateDocument {
                    target: fixture.path().to_owned(),
                    name: name.clone(),
                    template: kind,
                },
            )
            .unwrap();
        let bytes = fs::read(fixture.path().join(name)).unwrap();
        assert!(bytes[4..].starts_with(signature));
    }
}

#[test]
fn document_creation_preserves_existing_files_and_records_undo_and_redo() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("Note.rtf");
    let mut history = History::default();
    let create = || Operation::CreateDocument {
        target: fixture.path().to_owned(),
        name: "Note.rtf".into(),
        template: DocumentKind::RichText,
    };
    fs::write(&path, b"existing").unwrap();
    assert!(history.run(0, create()).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"existing");
    assert!(history.undo_label().is_none());
    fs::remove_file(&path).unwrap();
    history.run(0, create()).unwrap();
    let contents = fs::read(&path).unwrap();
    assert_eq!(history.undo_label(), Some("Create"));
    history.run(0, Operation::Undo).unwrap();
    assert!(!path.exists());
    history.run(0, Operation::Redo).unwrap();
    assert_eq!(fs::read(path).unwrap(), contents);
}
