use crate::shell::{drop_effect_format, file_drop_bytes, GlobalBlock};
use std::{os::windows::ffi::OsStrExt, path::PathBuf, ptr::null_mut};
use windows::{
    core::{implement, BOOL, HRESULT},
    Win32::{
        Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK},
        System::{
            Com::{
                IDataObject, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
            },
            Ole::{
                DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize, OleUninitialize,
                CF_HDROP, DROPEFFECT, DROPEFFECT_COPY,
            },
            SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS},
        },
        UI::Shell::SHCreateDataObject,
    },
};

/// Starts a native left-button file drag on the UI thread.
pub fn copy_paths(paths: &[PathBuf]) -> Result<(), String> {
    let _ole = OleApartment::initialize()?;
    let data = file_data_object(paths)?;
    let source: IDropSource = FileDropSource.into();
    let mut effect = DROPEFFECT::default();
    // The drop target handles the copy. The source never performs a file operation.
    unsafe { DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect) }
        .ok()
        .map_err(|error| format!("Windows could not start the file drag: {error}"))
}

struct OleApartment;

impl OleApartment {
    fn initialize() -> Result<Self, String> {
        unsafe { OleInitialize(None) }
            .map_err(|error| format!("Windows could not initialize file dragging: {error}"))?;
        Ok(Self)
    }
}

impl Drop for OleApartment {
    fn drop(&mut self) {
        unsafe { OleUninitialize() };
    }
}

fn file_data_object(paths: &[PathBuf]) -> Result<IDataObject, String> {
    if paths.is_empty()
        || paths
            .iter()
            .any(|path| !path.is_absolute() || path.as_os_str().encode_wide().any(|unit| unit == 0))
    {
        return Err("Dragging files requires absolute file paths.".into());
    }

    // A generic Shell data object can carry paths from different source folders.
    let data = unsafe { SHCreateDataObject(None, None, None::<&IDataObject>) }
        .map_err(|error| format!("Windows could not create file drop data: {error}"))?;
    let preferred_effect = u16::try_from(drop_effect_format())
        .ok()
        .filter(|format| *format != 0)
        .ok_or("Windows could not register the preferred drop effect.")?;
    set_global_data(&data, CF_HDROP.0, &file_drop_bytes(paths))?;
    set_global_data(&data, preferred_effect, &DROPEFFECT_COPY.0.to_le_bytes())?;
    Ok(data)
}

fn global_format(format: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: format,
        ptd: null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

fn set_global_data(data: &IDataObject, format: u16, bytes: &[u8]) -> Result<(), String> {
    let block = GlobalBlock::from_bytes(bytes)?;
    let medium = STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 {
            hGlobal: windows::Win32::Foundation::HGLOBAL(block.as_raw()),
        },
        ..Default::default()
    };
    unsafe { data.SetData(&global_format(format), &medium, true) }
        .map_err(|error| format!("Windows could not write file drop data: {error}"))?;
    // SetData takes ownership only after succeeding.
    block.into_raw();
    Ok(())
}

#[implement(IDropSource)]
struct FileDropSource;

impl IDropSource_Impl for FileDropSource_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if !keys.contains(MK_LBUTTON) {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Ole::ReleaseStgMedium;
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};

    #[test]
    fn file_drag_data_exposes_unicode_paths_and_copy_effect() {
        let _ole = OleApartment::initialize().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let nested = directory.path().join("anderer Ordner");
        std::fs::create_dir(&nested).unwrap();
        let paths = [
            directory.path().join("Grüße.txt"),
            nested.join("東京🚀.txt"),
        ];
        for path in &paths {
            std::fs::write(path, b"drag test").unwrap();
        }
        let data = file_data_object(&paths).unwrap();

        let format = global_format(CF_HDROP.0);
        let mut medium = unsafe { data.GetData(&format) }.unwrap();
        let actual = unsafe { crate::shell::dropped_paths(medium.u.hGlobal.0) };
        unsafe { ReleaseStgMedium(&mut medium) };
        assert_eq!(actual, paths);

        let format = global_format(drop_effect_format() as u16);
        let mut medium = unsafe { data.GetData(&format) }.unwrap();
        let effect = unsafe {
            let handle = medium.u.hGlobal.0;
            let value = GlobalLock(handle).cast::<u32>();
            let effect = value.as_ref().copied();
            if effect.is_some() {
                GlobalUnlock(handle);
            }
            ReleaseStgMedium(&mut medium);
            effect
        };
        assert_eq!(effect, Some(DROPEFFECT_COPY.0));
    }

    #[test]
    fn file_drag_rejects_missing_relative_and_nul_paths() {
        assert!(file_data_object(&[]).is_err());
        assert!(file_data_object(&[PathBuf::from("relative.txt")]).is_err());
        assert!(file_data_object(&[PathBuf::from("C:\\bad\0name")]).is_err());
    }

    #[test]
    fn file_drag_uses_left_button_and_can_be_cancelled() {
        let source: IDropSource = FileDropSource.into();
        unsafe {
            assert_eq!(source.QueryContinueDrag(false, MK_LBUTTON), S_OK);
            assert_eq!(
                source.QueryContinueDrag(false, MODIFIERKEYS_FLAGS(0)),
                DRAGDROP_S_DROP
            );
            assert_eq!(
                source.QueryContinueDrag(true, MK_LBUTTON),
                DRAGDROP_S_CANCEL
            );
            assert_eq!(
                source.GiveFeedback(DROPEFFECT_COPY),
                DRAGDROP_S_USEDEFAULTCURSORS
            );
        }
    }

    #[test]
    fn windows_folder_drop_copies_files_from_different_source_folders() {
        use crate::model::wide;
        use windows::{
            core::PCWSTR,
            Win32::{
                Foundation::{HWND, POINTL},
                System::{Com::IBindCtx, Ole::IDropTarget},
                UI::Shell::{BHID_SFObject, IShellFolder, IShellItem, SHCreateItemFromParsingName},
            },
        };

        let _ole = OleApartment::initialize().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("target");
        let nested = directory.path().join("other");
        std::fs::create_dir(&destination).unwrap();
        std::fs::create_dir(&nested).unwrap();
        let paths = [
            directory.path().join("Grüße.txt"),
            nested.join("東京🚀.txt"),
        ];
        for (index, path) in paths.iter().enumerate() {
            std::fs::write(path, [index as u8]).unwrap();
        }
        let data = file_data_object(&paths).unwrap();
        let destination_wide = wide(&destination);
        let item: IShellItem = unsafe {
            SHCreateItemFromParsingName(PCWSTR(destination_wide.as_ptr()), None::<&IBindCtx>)
        }
        .unwrap();
        let folder: IShellFolder =
            unsafe { item.BindToHandler(None::<&IBindCtx>, &BHID_SFObject) }.unwrap();
        let target: IDropTarget = unsafe { folder.CreateViewObject(HWND::default()) }.unwrap();
        let mut effect = DROPEFFECT_COPY;
        let point = POINTL::default();
        unsafe { target.DragEnter(&data, MK_LBUTTON, point, &mut effect) }.unwrap();
        assert_eq!(effect, DROPEFFECT_COPY);
        unsafe { target.Drop(&data, MODIFIERKEYS_FLAGS(0), point, &mut effect) }.unwrap();
        assert_eq!(effect, DROPEFFECT_COPY);
        for (index, path) in paths.iter().enumerate() {
            assert_eq!(std::fs::read(path).unwrap(), [index as u8]);
            assert_eq!(
                std::fs::read(destination.join(path.file_name().unwrap())).unwrap(),
                [index as u8]
            );
        }
    }
}
