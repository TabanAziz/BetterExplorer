#![cfg(windows)]

use better_explorer::{
    model::{from_wide, wide},
    shell::{run, Operation},
};
use std::{fs, path::PathBuf};
use windows::{
    core::{Interface, PCWSTR},
    Win32::{
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile,
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ,
        },
        UI::Shell::{IShellLinkW, SHGetPathFromIDListEx, ShellLink, GPFIDL_DEFAULT},
    },
};

#[test]
fn shell_creates_and_renames_unicode_items() {
    let fixture = tempfile::tempdir().expect("create fixture");
    let root = fixture.path();
    let folder = root.join("Öffentliche 資料 📁");
    run(
        0,
        Operation::Create {
            target: root.to_owned(),
            name: "Öffentliche 資料 📁".into(),
            directory: true,
        },
    )
    .expect("create Unicode folder through the Shell");
    assert!(folder.is_dir());

    let original = folder.join("Entwurf résumé.txt");
    run(
        0,
        Operation::Create {
            target: folder.clone(),
            name: "Entwurf résumé.txt".into(),
            directory: false,
        },
    )
    .expect("create Unicode file through the Shell");
    assert!(original.is_file());
    assert_eq!(fs::metadata(&original).unwrap().len(), 0);

    let payload = b"draft\0\xFF\r\n";
    fs::write(&original, payload).unwrap();
    run(
        0,
        Operation::Rename {
            path: original.clone(),
            name: "Fertig ✓.txt".into(),
        },
    )
    .expect("rename Unicode file through the Shell");
    assert!(!original.try_exists().unwrap());
    assert_eq!(fs::read(folder.join("Fertig ✓.txt")).unwrap(), payload);
    fixture.close().expect("remove fixture");
}

#[test]
fn shell_copy_and_move_preserve_unicode_paths_and_file_bytes() {
    let fixture = tempfile::tempdir().expect("create fixture");
    let source = fixture.path().join("Original 🌍.bin");
    let copy_folder = fixture.path().join("Kopien 日本語");
    let move_folder = fixture.path().join("Verschoben 📂");
    fs::create_dir(&copy_folder).unwrap();
    fs::create_dir(&move_folder).unwrap();
    let payload = [0, 1, 24, 255, 0, 128, 254, 13].repeat(4096);
    fs::write(&source, &payload).unwrap();

    run(
        0,
        Operation::Transfer {
            paths: vec![source.clone()],
            target: copy_folder.clone(),
            moving: false,
        },
    )
    .expect("copy file through the Shell");
    let copied = copy_folder.join(source.file_name().unwrap());
    assert_eq!(fs::read(&source).unwrap(), payload);
    assert_eq!(fs::read(&copied).unwrap(), payload);

    run(
        0,
        Operation::Transfer {
            paths: vec![source.clone()],
            target: move_folder.clone(),
            moving: true,
        },
    )
    .expect("move file through the Shell");
    let moved = move_folder.join(source.file_name().unwrap());
    assert!(!source.try_exists().unwrap());
    assert_eq!(fs::read(&moved).unwrap(), payload);
    assert_eq!(fs::read(&copied).unwrap(), payload);
    fixture.close().expect("remove fixture");
}

#[test]
fn shell_shortcut_loads_with_its_original_unicode_target() {
    let fixture = tempfile::tempdir().expect("create fixture");
    let source = fixture.path().join("Überblick 数据 📄.txt");
    let shortcut = fixture.path().join("Verknüpfung.lnk");
    fs::write(&source, b"fixture").unwrap();
    run(
        0,
        Operation::Shortcut {
            source: source.clone(),
            target: shortcut.clone(),
        },
    )
    .expect("save shortcut through the Shell");

    let shortcut_wide = wide(&shortcut);
    let mut target_path = vec![0; 32_768];
    let resolved = unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
        let result = (|| -> windows::core::Result<_> {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.cast::<IPersistFile>()?
                .Load(PCWSTR(shortcut_wide.as_ptr()), STGM_READ)?;
            let id = link.GetIDList()?;
            let resolved = !id.is_null()
                && SHGetPathFromIDListEx(id, &mut target_path, GPFIDL_DEFAULT).as_bool();
            CoTaskMemFree(Some(id.cast()));
            Ok(resolved)
        })();
        CoUninitialize();
        result.expect("reload saved shortcut through the Shell")
    };
    assert!(resolved, "saved shortcut has no filesystem target");
    let resolved = PathBuf::from(from_wide(&target_path));
    assert_eq!(
        fs::canonicalize(resolved).unwrap(),
        fs::canonicalize(source).unwrap()
    );
    fixture.close().expect("remove fixture");
}
