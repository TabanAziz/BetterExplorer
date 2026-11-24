use crate::{
    model::{from_wide, wide},
    shell::{dropped_paths, file_drop_bytes, GlobalBlock},
    work::network_path,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    os::windows::ffi::OsStrExt,
    path::{Component, Components, Path, PathBuf, Prefix},
    ptr::null_mut,
    rc::Rc,
};
use windows::{
    core::{implement, Ref, BOOL, HRESULT},
    Win32::{
        Foundation::{
            DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, HWND, POINTL, S_OK,
        },
        System::{
            Com::{
                IDataObject, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
            },
            Ole::{
                DoDragDrop, IDropSource, IDropSource_Impl, IDropTarget, IDropTarget_Impl,
                OleInitialize, OleUninitialize, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop,
                CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_NONE,
            },
            SystemServices::{MK_CONTROL, MK_LBUTTON, MK_SHIFT, MODIFIERKEYS_FLAGS},
        },
        UI::Shell::{SHCreateDataObject, CFSTR_PERFORMEDDROPEFFECT},
    },
};
use windows_sys::Win32::{
    Globalization::{CompareStringOrdinal, CSTR_EQUAL},
    Storage::FileSystem::{GetVolumeNameForVolumeMountPointW, GetVolumePathNameW},
    System::DataExchange::RegisterClipboardFormatW,
};

/// Starts a native left-button file drag on the UI thread.
pub fn drag_paths(paths: &[PathBuf]) -> Result<(), String> {
    let _ole = OleApartment::initialize()?;
    let data = file_data_object(paths)?;
    let source: IDropSource = FileDropSource.into();
    let mut effect = DROPEFFECT::default();
    // The filesystem drop target performs the complete transfer.
    unsafe {
        DoDragDrop(
            &data,
            &source,
            DROPEFFECT_COPY | DROPEFFECT_MOVE,
            &mut effect,
        )
    }
    .ok()
    .map_err(|error| format!("Windows could not start the file drag: {error}"))
}

/// Filters legacy file drops, which support only copying.
pub fn legacy_copy_paths(paths: Vec<PathBuf>, target: &Path) -> Vec<PathBuf> {
    DragFiles::new(paths)
        .plan(target)
        .map_or_else(Vec::new, |plan| plan.paths)
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

/// Owns the OLE registration on the UI thread. Drop it before destroying the window.
pub struct WindowDropTarget {
    window: HWND,
    _ole: OleApartment,
}

impl WindowDropTarget {
    pub fn register(
        window: HWND,
        folder_at: impl Fn(POINTL) -> Option<PathBuf> + 'static,
        queue_transfer: impl Fn(Vec<PathBuf>, PathBuf, bool) -> bool + 'static,
    ) -> Result<Self, String> {
        let ole = OleApartment::initialize()?;
        let target: IDropTarget = FileDropTarget {
            folder_at: Box::new(folder_at),
            queue_transfer: Box::new(queue_transfer),
            files: RefCell::new(None),
        }
        .into();
        unsafe { RegisterDragDrop(window, &target) }
            .map_err(|error| format!("Windows could not enable file drops: {error}"))?;
        Ok(Self { window, _ole: ole })
    }
}

impl Drop for WindowDropTarget {
    fn drop(&mut self) {
        let _ = unsafe { RevokeDragDrop(self.window) };
    }
}

#[implement(IDropTarget)]
struct FileDropTarget {
    folder_at: Box<dyn Fn(POINTL) -> Option<PathBuf>>,
    queue_transfer: Box<dyn Fn(Vec<PathBuf>, PathBuf, bool) -> bool>,
    files: RefCell<Option<DragFiles>>,
}

impl FileDropTarget_Impl {
    fn preview(&self, point: POINTL, keys: MODIFIERKEYS_FLAGS, allowed: DROPEFFECT) -> DROPEFFECT {
        let Some(target) = (self.folder_at)(point) else {
            return DROPEFFECT_NONE;
        };
        self.files
            .try_borrow_mut()
            .ok()
            .and_then(|mut files| files.as_mut()?.preview(target, keys, allowed))
            .unwrap_or(DROPEFFECT_NONE)
    }
}

impl IDropTarget_Impl for FileDropTarget_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let paths = data.as_ref().map_or_else(Vec::new, paths_from_data);
        if let Ok(mut files) = self.files.try_borrow_mut() {
            *files = Some(DragFiles::new(paths));
        }
        self.DragOver(keys, point, effect)
    }

    fn DragOver(
        &self,
        keys: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        if let Some(effect) = unsafe { effect.as_mut() } {
            *effect = self.preview(*point, keys, *effect);
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        if let Ok(mut files) = self.files.try_borrow_mut() {
            files.take();
        }
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let files = self
            .files
            .try_borrow_mut()
            .ok()
            .and_then(|mut files| files.take());
        let Some(effect) = (unsafe { effect.as_mut() }) else {
            return Ok(());
        };
        let allowed = std::mem::replace(effect, DROPEFFECT_NONE);
        let Some((files, target)) = files.zip((self.folder_at)(*point)) else {
            return Ok(());
        };
        let Some(plan) = files.plan(&target) else {
            return Ok(());
        };
        let chosen = plan.effect(keys, allowed);
        let moving = chosen == DROPEFFECT_MOVE;
        if chosen == DROPEFFECT_NONE || !(self.queue_transfer)(plan.paths, target, moving) {
            return Ok(());
        }
        if moving {
            // This target queues the entire move. The source must leave the originals alone.
            if let Some(data) = data.as_ref() {
                let format = unsafe { RegisterClipboardFormatW(CFSTR_PERFORMEDDROPEFFECT.0) };
                if format != 0 {
                    let _ = set_global_data(data, format as u16, &DROPEFFECT_NONE.0.to_le_bytes());
                }
            }
        } else {
            *effect = chosen;
        }
        Ok(())
    }
}

fn paths_from_data(data: &IDataObject) -> Vec<PathBuf> {
    let Ok(mut medium) = (unsafe { data.GetData(&global_format(CF_HDROP.0)) }) else {
        return Vec::new();
    };
    let paths = if medium.tymed == TYMED_HGLOBAL.0 as u32 {
        unsafe { dropped_paths(medium.u.hGlobal.0) }
    } else {
        Vec::new()
    };
    unsafe { ReleaseStgMedium(&mut medium) };
    paths
}

struct DragFiles {
    sources: Vec<(PathBuf, Rc<DropFolder>)>,
    hover: Option<(PathBuf, Option<DropPlan>)>,
}

struct DropFolder {
    resolved: PathBuf,
    volume: Option<OsString>,
}

impl DropFolder {
    fn resolve(path: &Path) -> Self {
        let local = !network_path(path);
        let canonical = local.then(|| std::fs::canonicalize(path).ok()).flatten();
        let resolved = canonical.as_deref().unwrap_or(path);
        Self {
            resolved: drop_path_key(resolved),
            volume: (local && !network_path(resolved))
                .then(|| volume_name(resolved))
                .flatten(),
        }
    }
}

struct DropPlan {
    paths: Vec<PathBuf>,
    same_volume: bool,
}

impl DragFiles {
    fn new(paths: Vec<PathBuf>) -> Self {
        let mut source_folders = HashMap::new();
        let mut seen = HashSet::new();
        let sources = paths
            .into_iter()
            .filter_map(|path| {
                let parent = path.parent().filter(|parent| parent.is_absolute())?;
                let folder = source_folders
                    .entry(parent.to_path_buf())
                    .or_insert_with(|| Rc::new(DropFolder::resolve(parent)));
                let resolved = folder.resolved.join(path.file_name()?);
                seen.insert(resolved).then(|| (path, Rc::clone(folder)))
            })
            .collect();
        Self {
            sources,
            hover: None,
        }
    }

    fn plan(&self, target: &Path) -> Option<DropPlan> {
        if self.sources.is_empty() {
            return None;
        }
        let target = DropFolder::resolve(target);
        let mut paths = Vec::new();
        let mut same_volume = target.volume.is_some();
        for (path, parent) in &self.sources {
            if let Some(mut remaining) = strip_drop_prefix(&target.resolved, &parent.resolved) {
                let Some(child) = remaining.next() else {
                    continue;
                };
                if windows_name_eq(child.as_os_str(), path.file_name()?) {
                    return None;
                }
            }
            same_volume &= parent.volume == target.volume;
            paths.push(path.clone());
        }
        (!paths.is_empty()).then_some(DropPlan { paths, same_volume })
    }

    fn preview(
        &mut self,
        target: PathBuf,
        keys: MODIFIERKEYS_FLAGS,
        allowed: DROPEFFECT,
    ) -> Option<DROPEFFECT> {
        if self.hover.as_ref().is_none_or(|(last, _)| last != &target) {
            let plan = self.plan(&target);
            self.hover = Some((target, plan));
        }
        Some(self.hover.as_ref()?.1.as_ref()?.effect(keys, allowed))
    }
}

impl DropPlan {
    fn effect(&self, keys: MODIFIERKEYS_FLAGS, allowed: DROPEFFECT) -> DROPEFFECT {
        let control = keys.contains(MK_CONTROL);
        let shift = keys.contains(MK_SHIFT);
        if control && shift {
            // Ctrl+Shift is the Shell's link gesture.
            return DROPEFFECT_NONE;
        }
        let preferred = if shift || (!control && self.same_volume) {
            DROPEFFECT_MOVE
        } else {
            DROPEFFECT_COPY
        };
        if allowed.contains(preferred) {
            preferred
        } else if !control && !shift && self.same_volume {
            allowed & DROPEFFECT_COPY
        } else {
            DROPEFFECT_NONE
        }
    }
}

fn drop_path_key(path: &Path) -> PathBuf {
    let mut parts = path.components();
    let Some(Component::Prefix(prefix)) = parts.next() else {
        return path.to_path_buf();
    };
    let mut key = match prefix.kind() {
        Prefix::VerbatimUNC(server, share) => {
            let mut name = OsString::from(r"\\");
            name.push(server);
            name.push(r"\");
            name.push(share);
            PathBuf::from(name)
        }
        Prefix::VerbatimDisk(letter) => PathBuf::from(format!("{}:", char::from(letter))),
        _ => PathBuf::from(prefix.as_os_str()),
    };
    for part in parts {
        match part {
            Component::ParentDir => {
                key.pop();
            }
            Component::CurDir => (),
            part => key.push(part.as_os_str()),
        }
    }
    key
}

fn strip_drop_prefix<'a>(path: &'a Path, parent: &Path) -> Option<Components<'a>> {
    if let Ok(path) = path.strip_prefix(parent) {
        return Some(path.components());
    }
    let mut parts = path.components();
    for part in parent.components() {
        let candidate = parts.next()?;
        if !windows_name_eq(candidate.as_os_str(), part.as_os_str()) {
            return None;
        }
    }
    Some(parts)
}

fn windows_name_eq(left: &OsStr, right: &OsStr) -> bool {
    left == right || {
        let (left, right) = (wide(left), wide(right));
        unsafe { CompareStringOrdinal(left.as_ptr(), -1, right.as_ptr(), -1, 1) == CSTR_EQUAL }
    }
}

fn volume_name(path: &Path) -> Option<OsString> {
    let path = wide(path);
    let mut root = [0u16; 1024];
    if unsafe { GetVolumePathNameW(path.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return None;
    }
    let mut name = [0u16; 64];
    if unsafe {
        GetVolumeNameForVolumeMountPointW(root.as_ptr(), name.as_mut_ptr(), name.len() as u32)
    } != 0
    {
        Some(from_wide(&name))
    } else {
        Some(from_wide(&root))
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
    set_global_data(&data, CF_HDROP.0, &file_drop_bytes(paths))?;
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

    #[test]
    fn file_drop_routes_multiple_files_using_the_target_and_modifiers() {
        let _ole = OleApartment::initialize().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let source = scratch.path().join("source");
        let target = source.join("destination");
        std::fs::create_dir_all(&target).unwrap();
        let paths = [source.join("first.txt"), source.join("Grüße.txt")];
        for (index, path) in paths.iter().enumerate() {
            std::fs::write(path, [index as u8]).unwrap();
        }

        let folder = target.clone();
        let data = file_data_object(&paths).unwrap();
        let target_interface: IDropTarget = FileDropTarget {
            folder_at: Box::new(move |point| {
                Some(if point.x == 0 {
                    source.clone()
                } else {
                    folder.clone()
                })
            }),
            queue_transfer: Box::new({
                let history = RefCell::new(crate::shell::History::default());
                move |paths, target, moving| {
                    history
                        .borrow_mut()
                        .run(
                            0,
                            crate::shell::Operation::Transfer {
                                paths,
                                target,
                                moving,
                            },
                        )
                        .is_ok()
                }
            }),
            files: RefCell::new(None),
        }
        .into();

        let allowed = DROPEFFECT_COPY | DROPEFFECT_MOVE;
        let mut effect = allowed;
        let background = POINTL { x: 0, y: 0 };
        let folder_row = POINTL { x: 1, y: 0 };
        unsafe { target_interface.DragEnter(&data, MK_LBUTTON, background, &mut effect) }.unwrap();
        assert_eq!(effect, DROPEFFECT_NONE);
        effect = allowed;
        unsafe { target_interface.Drop(&data, MK_CONTROL, background, &mut effect) }.unwrap();
        assert_eq!(effect, DROPEFFECT_NONE);
        assert_eq!(
            std::fs::read_dir(paths[0].parent().unwrap())
                .unwrap()
                .count(),
            3
        );
        effect = allowed;
        unsafe { target_interface.DragEnter(&data, MK_LBUTTON, background, &mut effect) }.unwrap();
        effect = allowed;
        unsafe { target_interface.DragOver(MK_LBUTTON, folder_row, &mut effect) }.unwrap();
        assert_eq!(effect, DROPEFFECT_MOVE);
        effect = allowed;
        unsafe { target_interface.DragOver(MK_LBUTTON | MK_CONTROL, folder_row, &mut effect) }
            .unwrap();
        assert_eq!(effect, DROPEFFECT_COPY);
        effect = allowed;
        unsafe { target_interface.Drop(&data, MK_SHIFT, folder_row, &mut effect) }.unwrap();
        assert_eq!(effect, DROPEFFECT_NONE); // The whole move is handled by the target.
        for (index, path) in paths.iter().enumerate() {
            assert!(!path.exists());
            assert_eq!(
                std::fs::read(target.join(path.file_name().unwrap())).unwrap(),
                [index as u8]
            );
        }
    }

    #[test]
    fn file_drop_defaults_to_copy_on_another_volume() {
        let plan = DropPlan {
            paths: Vec::new(),
            same_volume: false,
        };
        let allowed = DROPEFFECT_COPY | DROPEFFECT_MOVE;
        assert_eq!(plan.effect(MK_LBUTTON, allowed), DROPEFFECT_COPY);
        assert_eq!(plan.effect(MK_CONTROL, allowed), DROPEFFECT_COPY);
        assert_eq!(plan.effect(MK_SHIFT, allowed), DROPEFFECT_MOVE);
        assert_eq!(plan.effect(MK_SHIFT, DROPEFFECT_COPY), DROPEFFECT_NONE);
        assert_eq!(plan.effect(MK_LBUTTON, DROPEFFECT_MOVE), DROPEFFECT_NONE);
    }

    #[test]
    fn network_drops_compare_lexical_folder_paths_without_volume_probes() {
        let file = PathBuf::from(r"\\server\share\source\file.txt");
        let files = DragFiles::new(vec![file.clone()]);
        assert!(files
            .plan(Path::new(r"\\?\UNC\SERVER\SHARE\SOURCE"))
            .is_none());
        let sibling = files
            .plan(Path::new(r"\\server\share\destination"))
            .unwrap();
        assert_eq!(sibling.paths, [file]);
        assert!(!sibling.same_volume);

        let folder = PathBuf::from(r"\\server\share\source\Child");
        let files = DragFiles::new(vec![folder]);
        assert!(files
            .plan(Path::new(r"\\?\UNC\SERVER\share\source\child\nested"))
            .is_none());
    }

    #[test]
    fn file_drag_data_exposes_unicode_paths() {
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
        windows_folder_drop(MK_CONTROL, DROPEFFECT_COPY);
    }

    #[test]
    fn windows_folder_drop_moves_files_on_the_same_volume_by_default() {
        windows_folder_drop(MODIFIERKEYS_FLAGS(0), DROPEFFECT_MOVE);
    }

    fn windows_folder_drop(keys: MODIFIERKEYS_FLAGS, expected_effect: DROPEFFECT) {
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
        let allowed = DROPEFFECT_COPY | DROPEFFECT_MOVE;
        let mut effect = allowed;
        let point = POINTL::default();
        unsafe { target.DragEnter(&data, MK_LBUTTON | keys, point, &mut effect) }.unwrap();
        assert_eq!(effect, expected_effect);
        effect = allowed;
        unsafe { target.Drop(&data, keys, point, &mut effect) }.unwrap();
        for (index, path) in paths.iter().enumerate() {
            if expected_effect == DROPEFFECT_COPY {
                assert_eq!(std::fs::read(path).unwrap(), [index as u8]);
            } else {
                assert!(!path.exists());
            }
            assert_eq!(
                std::fs::read(destination.join(path.file_name().unwrap())).unwrap(),
                [index as u8]
            );
        }
    }
}
