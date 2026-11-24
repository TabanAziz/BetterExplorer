use super::{delete_with_shell, item, run_recorded, Apartment, Operation, ResultSink};
use crate::model::{from_wide, wide};
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io,
    mem::{offset_of, size_of},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
    ptr::copy_nonoverlapping,
    sync::{Arc, Mutex},
};
use windows::{
    core::{HRESULT, PCWSTR},
    Win32::{
        Foundation::{E_NOINTERFACE, E_NOTIMPL, HWND, S_OK},
        System::Com::CoTaskMemFree,
        UI::Shell::{
            BHID_Transfer, FOLDERID_RecycleBinFolder, IFileOperationProgressSink, ILGetSize,
            IShellItem, ITransferSource, SHChangeNotify, SHCreateItemFromIDList,
            SHGetIDListFromObject, SHGetKnownFolderItem, COPYENGINE_E_RECYCLE_BIN_NOT_FOUND,
            COPYENGINE_E_RECYCLE_FORCE_NUKE, COPYENGINE_E_RECYCLE_PATH_TOO_LONG,
            COPYENGINE_E_RECYCLE_SIZE_TOO_BIG, COPYENGINE_S_DONT_PROCESS_CHILDREN,
            COPYENGINE_S_KEEP_BOTH, KF_FLAG_DEFAULT, SHCNE_RENAMEFOLDER, SHCNE_RENAMEITEM,
            SHCNF_PATHW, SIGDN_FILESYSPATH, TSF_FAIL_EXIST,
        },
    },
};
use windows_sys::Win32::{
    Foundation::{ERROR_CALL_NOT_IMPLEMENTED, ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED},
    Globalization::{CompareStringOrdinal, CSTR_EQUAL},
    Storage::FileSystem::{
        FileIdInfo, FileRenameInfo, GetFileInformationByHandleEx, SetFileInformationByHandle,
        DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
        FILE_READ_ATTRIBUTES, FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE,
    },
};

const HISTORY_LIMIT: usize = 32;

#[derive(Default)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
}

impl History {
    pub fn undo_label(&self) -> Option<&'static str> {
        self.undo.last().map(|step| step.label)
    }

    pub fn redo_label(&self) -> Option<&'static str> {
        self.redo.last().map(|step| step.label)
    }

    pub fn run(&mut self, owner: usize, operation: Operation) -> Result<String, String> {
        match operation {
            Operation::Undo => self.replay(false),
            Operation::Redo => self.replay(true),
            _ => {
                let changes_listing = operation.updates_listing();
                let mut recording = Recording::default();
                let result = run_recorded(owner, operation, Some(&mut recording));
                let attempted = recording.attempted;
                let step = recording.finish();
                if step.is_some() || changes_listing && result.is_ok() {
                    self.redo.clear();
                }
                if let Some(step) = step {
                    let completed = step.changes.len();
                    push(&mut self.undo, step);
                    return match result {
                        Ok(message) if attempted > completed => Ok(append_status(
                            message,
                            &format!("Undo is available for {completed} of {attempted} items."),
                        )),
                        Ok(message) => Ok(message),
                        Err(error) => Err(format!(
                            "{error} Undo is available for {completed} item(s)."
                        )),
                    };
                }
                result.map(|message| {
                    if changes_listing {
                        self.undo.clear();
                        append_status(message, "Undo is unavailable for this action.")
                    } else {
                        message
                    }
                })
            }
        }
    }

    fn replay(&mut self, redo: bool) -> Result<String, String> {
        let _apartment = Apartment::start().map_err(|error| error.to_string())?;
        let (source, destination) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        let Some(mut step) = source.pop() else {
            return Err(format!(
                "Nothing to {}.",
                if redo { "redo" } else { "undo" }
            ));
        };
        let label = step.label;
        let mut inverse = Step {
            label,
            changes: Vec::with_capacity(step.changes.len()),
        };
        while let Some(change) = step.changes.last() {
            match change.apply() {
                Ok(opposite) => {
                    step.changes.pop();
                    inverse.changes.push(opposite);
                }
                Err(error) => {
                    source.push(step);
                    if inverse.changes.is_empty() {
                        return Err(error);
                    }
                    let count = inverse.changes.len();
                    push(destination, inverse);
                    return Err(format!("{count} item(s) completed. {error}"));
                }
            }
        }
        push(destination, inverse);
        Ok(format!(
            "{}: {label}.",
            if redo { "Redid" } else { "Undid" }
        ))
    }
}

fn append_status(message: String, status: &str) -> String {
    if message.is_empty() {
        status.into()
    } else {
        format!("{message} {status}")
    }
}

fn push(stack: &mut Vec<Step>, step: Step) {
    if stack.len() == HISTORY_LIMIT {
        stack.remove(0);
    }
    stack.push(step);
}

struct Step {
    label: &'static str,
    changes: Vec<Change>,
}

#[derive(Default)]
pub(super) struct Recording {
    label: Option<&'static str>,
    attempted: usize,
    changes: Arc<Mutex<Vec<Change>>>,
}

impl Recording {
    pub(super) fn track(
        &mut self,
        label: &'static str,
        capture: Option<Capture>,
    ) -> Option<IFileOperationProgressSink> {
        self.label = Some(label);
        self.attempted += 1;
        let capture = capture?;
        Some(
            ResultSink {
                failure: None,
                capture: Some(Observer {
                    capture,
                    changes: Arc::clone(&self.changes),
                }),
            }
            .into(),
        )
    }

    fn finish(self) -> Option<Step> {
        let label = self.label?;
        let changes = std::mem::take(&mut *self.changes.lock().unwrap());
        (!changes.is_empty()).then_some(Step { label, changes })
    }

    pub(super) fn recycle(&mut self, owner: HWND, paths: &[PathBuf]) -> Result<(), String> {
        self.label = Some("Delete");
        for path in paths {
            self.attempted += 1;
            let live = match Tracked::capture(path) {
                Ok(live) => live,
                Err(error)
                    if error
                        .raw_os_error()
                        .is_some_and(|code| UNSUPPORTED_FILE_SYSTEM.contains(&(code as u32))) =>
                {
                    delete_with_shell(owner, std::slice::from_ref(path), false)?;
                    continue;
                }
                Err(error) => return Err(format!("Can't open {}: {error}", path.display())),
            };
            match live.recycle(None) {
                Ok(change) => self.changes.lock().unwrap().push(change),
                Err(RecycleError::Unavailable(_)) => {
                    let _file = live.verified(0)?;
                    // Keep the native confirmation for a new Delete when the Bin is
                    // unavailable. History replay only calls RecycleItem.
                    delete_with_shell(owner, std::slice::from_ref(path), false)?;
                }
                Err(RecycleError::Failed(error)) => return Err(error),
            }
        }
        Ok(())
    }
}

const UNSUPPORTED_FILE_SYSTEM: [u32; 3] = [
    ERROR_CALL_NOT_IMPLEMENTED,
    ERROR_INVALID_FUNCTION,
    ERROR_NOT_SUPPORTED,
];

enum RecycleError {
    Unavailable(String),
    Failed(String),
}

impl From<String> for RecycleError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

impl RecycleError {
    fn into_message(self) -> String {
        match self {
            Self::Unavailable(message) | Self::Failed(message) => message,
        }
    }
}

pub(super) struct Observer {
    capture: Capture,
    changes: Arc<Mutex<Vec<Change>>>,
}

impl Observer {
    pub(super) fn completed(
        &self,
        result: HRESULT,
        source: Option<&IShellItem>,
        output: Option<&IShellItem>,
    ) {
        // Other success codes can mean a merge, skip or incomplete operation.
        if !matches!(
            result,
            S_OK | COPYENGINE_S_KEEP_BOTH | COPYENGINE_S_DONT_PROCESS_CHILDREN
        ) {
            return;
        }
        if let Some(change) = output.and_then(|output| self.capture.completed(source, output)) {
            self.changes.lock().unwrap().push(change);
        }
    }
}

pub(super) enum Capture {
    Created(Location),
    Copy {
        source: PathBuf,
        target: Location,
        occupied: bool,
    },
    Move {
        source: Option<PathBuf>,
        before: Tracked,
        target: Location,
        occupied: bool,
    },
}

impl Capture {
    pub(super) fn created(path: &Path) -> Option<Self> {
        let location = Location::capture(path).ok()?;
        vacant(&location.path).ok()?;
        Some(Self::Created(location))
    }

    pub(super) fn renamed(path: &Path, name: &str) -> Option<Self> {
        let before = Tracked::capture(path).ok()?;
        let target = before.location.with_name(OsStr::new(name))?;
        if before.location.path == target.path {
            return None;
        }
        let occupied = before.can_move_to(&target).is_err();
        Some(Self::Move {
            source: None,
            before,
            target,
            occupied,
        })
    }

    pub(super) fn transferred(
        path: &Path,
        source: &IShellItem,
        folder: &Path,
        moving: bool,
    ) -> Option<Self> {
        let target = Location::capture(&folder.join(path.file_name()?)).ok()?;
        let occupied = vacant(&target.path).is_err();
        let source = file_path(source)?;
        if moving {
            Some(Self::Move {
                before: Tracked::capture(path).ok()?,
                source: Some(source),
                target,
                occupied,
            })
        } else {
            Some(Self::Copy {
                source,
                target,
                occupied,
            })
        }
    }

    pub(super) fn publish_created(
        self,
        recording: &mut Recording,
        label: &'static str,
        path: &Path,
    ) {
        if let Self::Created(location) = self {
            recording.label = Some(label);
            recording.attempted += 1;
            if let Some(change) = location
                .created_output(path, false)
                .and_then(Change::generated)
            {
                recording.changes.lock().unwrap().push(change);
            }
        }
    }

    fn completed(&self, source: Option<&IShellItem>, output: &IShellItem) -> Option<Change> {
        match self {
            Self::Created(target) => {
                Change::generated(target.created_output(&file_path(output)?, false)?)
            }
            Self::Copy {
                source: expected,
                target,
                occupied,
            } => {
                (file_path(source?)? == *expected).then_some(())?;
                Change::generated(target.created_output(&file_path(output)?, *occupied)?)
            }
            Self::Move {
                source: expected,
                before,
                target,
                occupied,
            } => {
                if let Some(expected) = expected {
                    (file_path(source?)? == *expected).then_some(())?;
                }
                let output = file_path(output)?;
                let after = target.created_output(&output, *occupied)?;
                // Only record a move as reversible if the Shell moved the same object.
                (after.id == before.id && after.location.path != before.location.path).then_some(
                    Change::Move {
                        from: after,
                        to: before.location.clone(),
                    },
                )
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct Location {
    path: PathBuf,
    parent: Identity,
}

impl Location {
    fn capture(path: &Path) -> io::Result<Self> {
        let path = std::path::absolute(path)?;
        let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
        Ok(Self {
            parent: identity(&open(parent, 0, false)?)?,
            path,
        })
    }

    fn with_name(&self, name: &OsStr) -> Option<Self> {
        Some(Self {
            path: self.path.parent()?.join(name),
            parent: self.parent,
        })
    }

    fn verified_parent(&self) -> Result<&Path, String> {
        let path = self
            .path
            .parent()
            .ok_or("The original folder is unavailable.")?;
        let current = open(path, 0, false).and_then(|handle| identity(&handle));
        if current.is_ok_and(|id| id == self.parent) {
            Ok(path)
        } else {
            Err(format!(
                "The folder {} no longer matches the history.",
                path.display()
            ))
        }
    }

    fn created_output(&self, path: &Path, occupied: bool) -> Option<Tracked> {
        let output = Self::capture(path).ok()?;
        if self.parent != output.parent || (occupied && equivalent_name(&self.path, &output.path)) {
            return None;
        }
        let id = identity_at(&output.path).ok()?;
        // Preserve the Shell's actual output name, including names chosen to keep both items.
        Some(Tracked {
            location: output,
            id,
        })
    }
}

#[derive(Clone)]
pub(super) struct Tracked {
    location: Location,
    id: Identity,
}

impl Tracked {
    fn capture(path: &Path) -> io::Result<Self> {
        let location = Location::capture(path)?;
        let id = identity_at(&location.path)?;
        Ok(Self { location, id })
    }

    fn verified(&self, access: u32) -> Result<File, String> {
        self.location.verified_parent()?;
        let file = open(&self.location.path, access, true)
            .map_err(|error| format!("Can't open {}: {error}", self.location.path.display()))?;
        if identity(&file).is_ok_and(|id| id == self.id) {
            Ok(file)
        } else {
            Err(format!(
                "{} no longer matches the history.",
                self.location.path.display()
            ))
        }
    }

    fn can_move_to(&self, to: &Location) -> Result<(), String> {
        to.verified_parent()?;
        if self.location.parent == to.parent
            && equivalent_name(&self.location.path, &to.path)
            && identity_at(&to.path).is_ok_and(|id| id == self.id)
        {
            return Ok(());
        }
        vacant(&to.path)
    }

    fn recycle(&self, expected: Option<OutputStamp>) -> Result<Change, RecycleError> {
        let path = &self.location.path;
        let failure = |error: windows::core::Error| {
            let message = format!("Can't move {} to the Recycle Bin: {error}", path.display());
            let code = error.code();
            if matches!(
                code,
                E_NOINTERFACE
                    | E_NOTIMPL
                    | COPYENGINE_E_RECYCLE_BIN_NOT_FOUND
                    | COPYENGINE_E_RECYCLE_FORCE_NUKE
                    | COPYENGINE_E_RECYCLE_PATH_TOO_LONG
                    | COPYENGINE_E_RECYCLE_SIZE_TOO_BIG
            ) || UNSUPPORTED_FILE_SYSTEM
                .into_iter()
                .any(|win32| HRESULT::from_win32(win32) == code)
            {
                RecycleError::Unavailable(message)
            } else {
                RecycleError::Failed(message)
            }
        };
        let prepare = || -> windows::core::Result<_> {
            let source = item(path)?;
            let parent = unsafe { source.GetParent()? };
            let transfer: ITransferSource = unsafe { parent.BindToHandler(None, &BHID_Transfer)? };
            let bin: IShellItem =
                unsafe { SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None)? };
            Ok((source, transfer, bin))
        };
        let (source, transfer, bin) = prepare().map_err(failure)?;
        let original = self.verified(0)?;
        if let Some(expected) = expected {
            expected.verify(&original, path)?;
        }
        // RecycleItem has no permanent-delete fallback. Keep the verified file open until
        // the Shell returns its exact new location, then validate the recycled identity.
        let output = unsafe { transfer.RecycleItem(&source, &bin, 0) }.map_err(failure)?;
        let recycled_path = file_path(&output).ok_or_else(|| {
            format!(
                "Windows did not return a usable Recycle Bin entry for {}.",
                path.display()
            )
        })?;
        if let Ok(current) = identity_at(&recycled_path) {
            if current != self.id {
                let recovery = restore_recycled(&output, &self.location);
                let message = format!("{} changed during the operation.", path.display());
                return Err(RecycleError::Failed(match recovery {
                    Ok(()) => format!("{message} The changed item was restored."),
                    Err(error) => {
                        format!("{message} The changed item is in the Recycle Bin. {error}")
                    }
                }));
            }
        }
        let recycled = Recycled::capture(&output, recycled_path).ok_or_else(|| {
            format!(
                "Windows did not return a usable Recycle Bin entry for {}.",
                path.display()
            )
        })?;
        Ok(Change::Restore {
            to: self.clone(),
            recycled,
            generated: expected.is_some(),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct OutputStamp {
    last_write: u64,
    size: u64,
    directory: bool,
}

impl OutputStamp {
    fn capture(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        Ok(Self {
            last_write: metadata.last_write_time(),
            size: metadata.file_size(),
            directory: metadata.is_dir(),
        })
    }

    fn verify(self, file: &File, path: &Path) -> Result<(), String> {
        if Self::capture(file).is_ok_and(|current| current == self)
            || self.directory
                && fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none())
        {
            Ok(())
        } else {
            Err(format!(
                "{} has changed since the operation.",
                path.display()
            ))
        }
    }
}

enum Change {
    Move {
        from: Tracked,
        to: Location,
    },
    Trash {
        item: Tracked,
        expected: Option<OutputStamp>,
    },
    Restore {
        to: Tracked,
        recycled: Recycled,
        generated: bool,
    },
}

impl Change {
    fn generated(item: Tracked) -> Option<Self> {
        let expected = OutputStamp::capture(&item.verified(0).ok()?).ok()?;
        Some(Self::Trash {
            item,
            expected: Some(expected),
        })
    }

    fn apply(&self) -> Result<Self, String> {
        match self {
            Self::Move { from, to } => {
                from.can_move_to(to)?;
                let handle = from.verified(DELETE)?;
                rename_by_handle(&handle, &to.path)
                    .map_err(|error| format!("Can't move to {}: {error}", to.path.display()))?;
                notify_move(&handle, &from.location.path, &to.path);
                Ok(Self::Move {
                    from: Tracked {
                        location: to.clone(),
                        id: from.id,
                    },
                    to: from.location.clone(),
                })
            }
            Self::Trash { item, expected } => {
                item.recycle(*expected).map_err(RecycleError::into_message)
            }
            Self::Restore {
                to,
                recycled,
                generated,
            } => {
                let source = recycled.item(to.id)?;
                restore_recycled(&source, &to.location)?;
                let restored = to.verified(0)?;
                let expected = generated
                    .then(|| OutputStamp::capture(&restored).ok())
                    .flatten();
                Ok(Self::Trash {
                    item: to.clone(),
                    expected,
                })
            }
        }
    }
}

struct Recycled {
    pidl: Vec<u16>,
    path: PathBuf,
}

impl Recycled {
    fn capture(item: &IShellItem, path: PathBuf) -> Option<Self> {
        let raw = unsafe { SHGetIDListFromObject(item).ok()? };
        let length = unsafe { ILGetSize(Some(raw)) } as usize;
        let mut pidl = vec![0u16; length.div_ceil(2)];
        unsafe {
            copy_nonoverlapping(raw.cast::<u8>(), pidl.as_mut_ptr().cast(), length);
            CoTaskMemFree(Some(raw.cast()));
        }
        (!pidl.is_empty()).then_some(Self { pidl, path })
    }

    fn item(&self, id: Identity) -> Result<IShellItem, String> {
        if !identity_at(&self.path).is_ok_and(|current| current == id) {
            return Err("The original item is no longer available in the Recycle Bin.".into());
        }
        unsafe { SHCreateItemFromIDList(self.pidl.as_ptr().cast()) }
            .map_err(|error| error.to_string())
    }
}

fn restore_recycled(source: &IShellItem, to: &Location) -> Result<(), String> {
    let parent = to.verified_parent()?;
    vacant(&to.path)?;
    let name = wide(
        to.path
            .file_name()
            .ok_or("The original name is unavailable.")?,
    );
    let restore = || -> windows::core::Result<IShellItem> {
        let bin = unsafe { source.GetParent()? };
        let transfer: ITransferSource = unsafe { bin.BindToHandler(None, &BHID_Transfer)? };
        unsafe {
            transfer.MoveItem(
                source,
                &item(parent)?,
                PCWSTR(name.as_ptr()),
                TSF_FAIL_EXIST.0 as u32,
            )
        }
    };
    restore()
        .map(|_| ())
        .map_err(|error| format!("Can't restore {}: {error}", to.path.display()))
}

fn file_path(item: &IShellItem) -> Option<PathBuf> {
    let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH).ok()? };
    let path = PathBuf::from(from_wide(unsafe { name.as_wide() }));
    unsafe { CoTaskMemFree(Some(name.0.cast())) };
    Some(path)
}

pub(super) fn same_source_folder(paths: &[PathBuf], target: &Path) -> bool {
    let target = open(target, 0, false).and_then(|folder| identity(&folder));
    target.is_ok_and(|target| {
        paths.iter().all(|path| {
            path.parent().is_some_and(|parent| {
                open(parent, 0, false)
                    .and_then(|folder| identity(&folder))
                    .is_ok_and(|id| id == target)
            })
        })
    })
}

fn equivalent_name(left: &Path, right: &Path) -> bool {
    let (Some(left), Some(right)) = (left.file_name(), right.file_name()) else {
        return false;
    };
    let (left, right) = (wide(left), wide(right));
    unsafe { CompareStringOrdinal(left.as_ptr(), -1, right.as_ptr(), -1, 1) == CSTR_EQUAL }
}

fn vacant(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Can't check {}: {error}", path.display())),
        Ok(_) => Err(format!("{} already exists.", path.display())),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
    volume: u64,
    file: [u8; 16],
}

fn open(path: &Path, access: u32, reparse_point: bool) -> io::Result<File> {
    let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
    if reparse_point {
        flags |= FILE_FLAG_OPEN_REPARSE_POINT;
    }
    OpenOptions::new()
        .access_mode(access | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(flags)
        .open(path)
}

fn identity_at(path: &Path) -> io::Result<Identity> {
    identity(&open(path, 0, true)?)
}

fn identity(file: &File) -> io::Result<Identity> {
    let mut information: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut information as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(Identity {
        volume: information.VolumeSerialNumber,
        file: information.FileId.Identifier,
    })
}

fn rename_by_handle(file: &File, to: &Path) -> io::Result<()> {
    let mut name = wide(to);
    name.pop();
    let name_size = name.len() * size_of::<u16>();
    let length = size_of::<FILE_RENAME_INFO>() + name_size;
    let mut storage = vec![0usize; length.div_ceil(size_of::<usize>())];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        // Zeroed flags prohibit replacement. The checked source handle binds this rename
        // to the original file even if another process changes its old path concurrently.
        (*info).FileNameLength = name_size as u32;
        copy_nonoverlapping(
            name.as_ptr(),
            storage
                .as_mut_ptr()
                .cast::<u8>()
                .add(offset_of!(FILE_RENAME_INFO, FileName))
                .cast(),
            name.len(),
        );
        if SetFileInformationByHandle(
            file.as_raw_handle(),
            FileRenameInfo,
            info.cast(),
            length as u32,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn notify_move(file: &File, from: &Path, to: &Path) {
    let event = if file.metadata().is_ok_and(|metadata| metadata.is_dir()) {
        SHCNE_RENAMEFOLDER
    } else {
        SHCNE_RENAMEITEM
    };
    let (from, to) = (wide(from), wide(to));
    unsafe {
        SHChangeNotify(
            event,
            SHCNF_PATHW,
            Some(from.as_ptr().cast()),
            Some(to.as_ptr().cast()),
        )
    };
}
