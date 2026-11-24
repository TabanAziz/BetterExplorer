use crate::{
    archive,
    model::{from_wide, safe_name, wide},
};
use std::{
    ffi::OsStr,
    io,
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
    ptr::{copy_nonoverlapping, null},
    sync::{Arc, Mutex},
};
use windows::{
    core::{implement, Interface, Ref, HRESULT, PCWSTR},
    Win32::{
        Foundation::HWND,
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        },
        UI::Shell::{
            FileOperation, IFileOperation, IFileOperationProgressSink,
            IFileOperationProgressSink_Impl, IShellItem, IShellLinkW, SHCreateItemFromParsingName,
            SHOpenWithDialog, ShellLink, COPYENGINE_S_USER_IGNORED, FOFX_ADDUNDORECORD,
            FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE, FOFX_SHOWELEVATIONPROMPT, FOF_ALLOWUNDO,
            FOF_WANTNUKEWARNING, OAIF_EXEC, OPENASINFO,
        },
    },
};
use windows_sys::Win32::{
    Foundation::GlobalFree,
    Storage::FileSystem::{MoveFileExW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL},
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
            GetOpenClipboardWindow, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
        },
        Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
        Ole::{CF_HDROP, CF_UNICODETEXT},
    },
    UI::{
        Shell::{
            DragQueryFileW, ShellExecuteExW, DROPFILES, HDROP, SEE_MASK_INVOKEIDLIST,
            SHELLEXECUTEINFOW,
        },
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

pub enum Operation {
    Open(PathBuf),
    OpenWith(PathBuf),
    Properties(PathBuf),
    Reveal(PathBuf),
    Recycle {
        keys: Vec<String>,
        restore: bool,
    },
    EmptyRecycle,
    Clipboard {
        paths: Vec<PathBuf>,
        cut: bool,
        receipt: ClipboardWriteReceipt,
    },
    CopyPath(PathBuf),
    Paste {
        target: PathBuf,
        receipt: ClipboardPasteReceipt,
    },
    Transfer {
        paths: Vec<PathBuf>,
        target: PathBuf,
        moving: bool,
    },
    Create {
        target: PathBuf,
        name: String,
        directory: bool,
    },
    Rename {
        path: PathBuf,
        name: String,
    },
    Delete {
        paths: Vec<PathBuf>,
        permanent: bool,
    },
    Shortcut {
        source: PathBuf,
        target: PathBuf,
    },
    Compress {
        sources: Vec<PathBuf>,
        output: PathBuf,
    },
    Extract {
        source: PathBuf,
        output: PathBuf,
    },
}

impl Operation {
    pub fn updates_listing(&self) -> bool {
        matches!(
            self,
            Self::Paste { .. }
                | Self::Transfer { .. }
                | Self::Create { .. }
                | Self::Rename { .. }
                | Self::Delete { .. }
                | Self::Recycle { .. }
                | Self::EmptyRecycle
                | Self::Shortcut { .. }
                | Self::Compress { .. }
                | Self::Extract { .. }
        )
    }
}

struct Apartment;

impl Apartment {
    fn start() -> windows::core::Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}

pub fn run(owner: usize, operation: Operation) -> Result<String, String> {
    let _apartment = Apartment::start().map_err(|e| e.to_string())?;
    let owner = HWND(owner as _);
    let result = match operation {
        Operation::Open(path) => execute(owner, "open", path.as_os_str(), None),
        Operation::OpenWith(path) => {
            let path = wide(path);
            let options = OPENASINFO {
                pcszFile: PCWSTR(path.as_ptr()),
                pcszClass: PCWSTR::null(),
                oaifInFlags: OAIF_EXEC,
            };
            unsafe { SHOpenWithDialog(Some(owner), &options) }.map_err(|e| e.to_string())
        }
        Operation::Properties(path) => execute(owner, "properties", path.as_os_str(), None),
        Operation::Reveal(path) => reveal(owner, &path),
        Operation::Recycle { keys, restore } => {
            crate::namespace::recycle_action(&keys, restore, owner.0 as usize)
        }
        Operation::EmptyRecycle => unsafe {
            windows::Win32::UI::Shell::SHEmptyRecycleBinW(Some(owner), PCWSTR::null(), 0)
                .map_err(|error| error.to_string())
        },
        Operation::Clipboard {
            paths,
            cut,
            receipt,
        } => {
            if let Err(error) = put_files(owner, &paths, cut, &receipt) {
                *receipt.state.lock().unwrap() = ClipboardWriteState::Failed;
                return Err(error);
            }
            return Ok(format!(
                "{} {} item(s).",
                if cut { "Cut" } else { "Copied" },
                paths.len()
            ));
        }
        Operation::CopyPath(path) => {
            let data = wide(path);
            write_clipboard(owner, &[(CF_UNICODETEXT as u32, as_bytes(&data))], None)?;
            return Ok("Copied path.".into());
        }
        Operation::Paste { target, receipt } => {
            let (paths, moving, sequence) = read_clipboard(owner, &receipt)?;
            transfer(owner, &paths, &target, moving)?;
            if moving {
                clear_clipboard_if_unchanged(owner, sequence);
            }
            return Ok(if moving {
                "Items moved."
            } else {
                "Items copied."
            }
            .into());
        }
        Operation::Transfer {
            paths,
            target,
            moving,
        } => {
            transfer(owner, &paths, &target, moving)?;
            return Ok(if moving {
                "Items moved."
            } else {
                "Items copied."
            }
            .into());
        }
        Operation::Create {
            target,
            name,
            directory,
        } => create(owner, &target, &name, directory),
        Operation::Rename { path, name } => rename(owner, &path, &name),
        Operation::Delete { paths, permanent } => delete(owner, &paths, permanent),
        Operation::Shortcut { source, target } => shortcut(&source, &target),
        Operation::Compress { sources, output } => {
            archive::compress(&sources, &output)?;
            return Ok("ZIP archive created.".into());
        }
        Operation::Extract { source, output } => {
            archive::extract(&source, &output)?;
            return Ok("ZIP archive extracted.".into());
        }
    };
    result.map(|()| String::new())
}

fn execute(owner: HWND, verb: &str, path: &OsStr, params: Option<&OsStr>) -> Result<(), String> {
    let verb = wide(verb);
    let path = wide(path);
    let params = params.map(wide);
    let mut info: SHELLEXECUTEINFOW = unsafe { zeroed() };
    info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_INVOKEIDLIST;
    info.hwnd = owner.0;
    info.lpVerb = verb.as_ptr();
    info.lpFile = path.as_ptr();
    info.lpParameters = params.as_ref().map_or(null(), |value| value.as_ptr());
    info.nShow = SW_SHOWNORMAL;
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    Ok(())
}

fn reveal(owner: HWND, path: &Path) -> Result<(), String> {
    let mut parameters = std::ffi::OsString::from("/select,\"");
    parameters.push(path);
    parameters.push("\"");
    let executable = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .ok_or_else(|| "Windows directory is unavailable.".to_owned())?
        .join("explorer.exe");
    execute(owner, "open", executable.as_os_str(), Some(&parameters))
}

fn item(path: &Path) -> windows::core::Result<IShellItem> {
    let path = wide(path);
    unsafe { SHCreateItemFromParsingName(PCWSTR(path.as_ptr()), None) }
}

fn perform(
    owner: HWND,
    recycle: Option<bool>,
    queue: impl FnOnce(&IFileOperation) -> windows::core::Result<()>,
) -> Result<(), String> {
    let run = || -> windows::core::Result<_> {
        let failure = Arc::new(Mutex::new(None));
        let sink: IFileOperationProgressSink = ResultSink(Arc::clone(&failure)).into();
        // The COM apartment and all Shell objects stay on the operation thread.
        let aborted = unsafe {
            let operation: IFileOperation =
                CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)?;
            if !owner.0.is_null() {
                operation.SetOwnerWindow(owner)?;
            }
            let base = FOFX_SHOWELEVATIONPROMPT | FOFX_EARLYFAILURE;
            let flags = match recycle {
                Some(false) => base,
                Some(true) => {
                    base | FOF_ALLOWUNDO
                        | FOFX_ADDUNDORECORD
                        | FOFX_RECYCLEONDELETE
                        | FOF_WANTNUKEWARNING
                }
                None => base | FOF_ALLOWUNDO | FOFX_ADDUNDORECORD,
            };
            operation.SetOperationFlags(flags)?;
            queue(&operation)?;
            let cookie = operation.Advise(&sink)?;
            let result = operation.PerformOperations();
            let _ = operation.Unadvise(cookie);
            result?;
            operation.GetAnyOperationsAborted()?.as_bool()
        };
        let failure = *failure.lock().unwrap();
        Ok((aborted, failure))
    };
    match run().map_err(|error| error.to_string())? {
        (_, Some(COPYENGINE_S_USER_IGNORED)) => {
            Err("Windows skipped one or more items. The operation did not finish.".into())
        }
        (_, Some(code)) => Err(windows::core::Error::from_hresult(code).to_string()),
        (true, _) => Err("Windows stopped the operation before it finished.".into()),
        _ => Ok(()),
    }
}

fn transfer(owner: HWND, paths: &[PathBuf], target: &Path, moving: bool) -> Result<(), String> {
    perform(owner, None, |operation| {
        let destination = item(target)?;
        for path in paths {
            let source = item(path)?;
            unsafe {
                if moving {
                    operation.MoveItem(&source, &destination, PCWSTR::null(), None)?;
                } else {
                    operation.CopyItem(&source, &destination, PCWSTR::null(), None)?;
                }
            }
        }
        Ok(())
    })
}

fn create(owner: HWND, target: &Path, name: &str, directory: bool) -> Result<(), String> {
    if !safe_name(name) {
        return Err("Enter a valid Windows file name.".into());
    }
    let name = wide(name);
    perform(owner, None, |operation| unsafe {
        operation.NewItem(
            &item(target)?,
            if directory {
                FILE_ATTRIBUTE_DIRECTORY
            } else {
                FILE_ATTRIBUTE_NORMAL
            },
            PCWSTR(name.as_ptr()),
            PCWSTR::null(),
            None,
        )
    })
}

fn rename(owner: HWND, path: &Path, name: &str) -> Result<(), String> {
    if !safe_name(name) {
        return Err("Enter a valid Windows file name.".into());
    }
    let name = wide(name);
    perform(owner, None, |operation| unsafe {
        operation.RenameItem(&item(path)?, PCWSTR(name.as_ptr()), None)
    })
}

fn delete(owner: HWND, paths: &[PathBuf], permanent: bool) -> Result<(), String> {
    perform(owner, Some(!permanent), |operation| {
        for path in paths {
            unsafe {
                operation.DeleteItem(&item(path)?, None)?;
            }
        }
        Ok(())
    })
}

fn shortcut(source: &Path, target: &Path) -> Result<(), String> {
    let parent = target.parent().ok_or("Choose a destination folder.")?;
    let staged = tempfile::tempdir_in(parent).map_err(|error| error.to_string())?;
    let staged_file = wide(staged.path().join("target.lnk"));
    let source = wide(source);
    let target = wide(target);
    let create = || -> windows::core::Result<()> {
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(PCWSTR(source.as_ptr()))?;
            link.cast::<IPersistFile>()?
                .Save(PCWSTR(staged_file.as_ptr()), true)
        }
    };
    create().map_err(|error| error.to_string())?;
    // Publish without replacing an existing shortcut.
    if unsafe { MoveFileExW(staged_file.as_ptr(), target.as_ptr(), 0) } == 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    Ok(())
}

#[implement(IFileOperationProgressSink)]
struct ResultSink(Arc<Mutex<Option<HRESULT>>>);

impl ResultSink_Impl {
    fn record(&self, code: HRESULT) {
        if code.is_err() || code == COPYENGINE_S_USER_IGNORED {
            let mut failure = self.0.lock().unwrap();
            if failure.is_none() || *failure == Some(COPYENGINE_S_USER_IGNORED) {
                *failure = Some(code);
            }
        }
    }
}

#[allow(non_snake_case)]
impl IFileOperationProgressSink_Impl for ResultSink_Impl {
    fn StartOperations(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn FinishOperations(&self, result: HRESULT) -> windows::core::Result<()> {
        self.record(result);
        Ok(())
    }
    fn PreRenameItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostRenameItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        result: HRESULT,
        _: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(result);
        Ok(())
    }
    fn PreMoveItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: Ref<IShellItem>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostMoveItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        result: HRESULT,
        _: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(result);
        Ok(())
    }
    fn PreCopyItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: Ref<IShellItem>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostCopyItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        result: HRESULT,
        _: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(result);
        Ok(())
    }
    fn PreDeleteItem(&self, _: u32, _: Ref<IShellItem>) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostDeleteItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        result: HRESULT,
        _: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(result);
        Ok(())
    }
    fn PreNewItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostNewItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        _: &PCWSTR,
        _: u32,
        result: HRESULT,
        _: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(result);
        Ok(())
    }
    fn UpdateProgress(&self, _: u32, _: u32) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResetTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct ClipboardWriteReceipt {
    state: Arc<Mutex<ClipboardWriteState>>,
}

enum ClipboardWriteState {
    Queued(u32),
    Writing(u32),
    Written { previous: u32, sequence: u32 },
    Failed,
}

impl ClipboardWriteReceipt {
    pub fn capture() -> Self {
        Self {
            state: Arc::new(Mutex::new(ClipboardWriteState::Queued(unsafe {
                GetClipboardSequenceNumber()
            }))),
        }
    }
}

pub struct ClipboardPasteReceipt {
    sequence: u32,
    queued_write: Option<ClipboardWriteReceipt>,
}

impl ClipboardPasteReceipt {
    pub fn capture(owner: usize, prior_write: Option<ClipboardWriteReceipt>) -> Self {
        let Some(write) = prior_write else {
            return Self {
                sequence: unsafe { GetClipboardSequenceNumber() },
                queued_write: None,
            };
        };
        let state = write.state.lock().unwrap();
        let sequence = unsafe { GetClipboardSequenceNumber() };
        // A paste may arrive while Copy or Cut is publishing its clipboard formats.
        let queued_sequence = match *state {
            ClipboardWriteState::Queued(previous) if previous == sequence => Some(previous),
            ClipboardWriteState::Writing(previous)
                if owner != 0 && unsafe { GetOpenClipboardWindow() } == owner as _ =>
            {
                Some(previous)
            }
            _ => None,
        };
        drop(state);
        Self {
            sequence: queued_sequence.unwrap_or(sequence),
            queued_write: queued_sequence.map(|_| write),
        }
    }

    fn expected_sequence(&self) -> Result<u32, String> {
        let sequence = if let Some(write) = &self.queued_write {
            match *write.state.lock().unwrap() {
                ClipboardWriteState::Written { previous, sequence }
                    if previous == self.sequence =>
                {
                    sequence
                }
                ClipboardWriteState::Written { .. } => return Err(CLIPBOARD_CHANGED.into()),
                _ => return Err("The queued Copy or Cut did not update the clipboard.".into()),
            }
        } else {
            self.sequence
        };
        if sequence == 0 {
            return Err(
                "Windows could not identify the clipboard contents. Try Paste again.".into(),
            );
        }
        Ok(sequence)
    }
}

const CLIPBOARD_CHANGED: &str = "The clipboard changed. Paste again to use its current contents.";

struct Clipboard;
impl Clipboard {
    fn open(owner: HWND) -> Result<Self, String> {
        if unsafe { OpenClipboard(owner.0) } == 0 {
            Err(format!(
                "Can't open the clipboard: {}",
                io::Error::last_os_error()
            ))
        } else {
            Ok(Self)
        }
    }
}
impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

fn drop_effect_format() -> u32 {
    unsafe { RegisterClipboardFormatW(wide("Preferred DropEffect").as_ptr()) }
}

fn as_bytes(units: &[u16]) -> &[u8] {
    // A u16 buffer can always be read as a byte slice of twice the length.
    unsafe { std::slice::from_raw_parts(units.as_ptr().cast(), std::mem::size_of_val(units)) }
}

fn put_files(
    owner: HWND,
    paths: &[PathBuf],
    cut: bool,
    receipt: &ClipboardWriteReceipt,
) -> Result<(), String> {
    let mut list = Vec::new();
    for path in paths {
        list.extend(wide(path));
    }
    list.push(0);
    let mut storage = vec![0; size_of::<DROPFILES>()];
    storage[..4].copy_from_slice(&(size_of::<DROPFILES>() as u32).to_le_bytes());
    storage[16..20].copy_from_slice(&1u32.to_le_bytes());
    storage.extend_from_slice(as_bytes(&list));
    let effect = (if cut { 2u32 } else { 1u32 }).to_le_bytes();
    write_clipboard(
        owner,
        &[(CF_HDROP as u32, &storage), (drop_effect_format(), &effect)],
        Some(receipt),
    )
}

fn write_clipboard(
    owner: HWND,
    items: &[(u32, &[u8])],
    receipt: Option<&ClipboardWriteReceipt>,
) -> Result<(), String> {
    let mut blocks = Vec::with_capacity(items.len());
    for &(format, bytes) in items {
        if format == 0 {
            return Err("Windows could not register the clipboard format.".into());
        }
        let block = GlobalBlock(unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) });
        if block.0.is_null() {
            return Err(io::Error::last_os_error().to_string());
        }
        unsafe {
            let data = GlobalLock(block.0);
            if data.is_null() {
                return Err(io::Error::last_os_error().to_string());
            }
            copy_nonoverlapping(bytes.as_ptr(), data.cast(), bytes.len());
            GlobalUnlock(block.0);
        }
        blocks.push((format, block));
    }
    let _clipboard = Clipboard::open(owner)?;
    let previous = receipt.map(|receipt| {
        let previous = unsafe { GetClipboardSequenceNumber() };
        *receipt.state.lock().unwrap() = ClipboardWriteState::Writing(previous);
        previous
    });
    if unsafe { EmptyClipboard() } == 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    // Store the effect first so a failed cut never puts a copy-only payload on the clipboard.
    for (format, mut block) in blocks.into_iter().rev() {
        if unsafe { SetClipboardData(format, block.0) }.is_null() {
            return Err(io::Error::last_os_error().to_string());
        }
        block.0 = std::ptr::null_mut();
    }
    if let Some((receipt, previous)) = receipt.zip(previous) {
        // Capture the published version before another window can open the clipboard.
        *receipt.state.lock().unwrap() = ClipboardWriteState::Written {
            previous,
            sequence: unsafe { GetClipboardSequenceNumber() },
        };
    }
    Ok(())
}

struct GlobalBlock(windows_sys::Win32::Foundation::HGLOBAL);
impl Drop for GlobalBlock {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                GlobalFree(self.0);
            }
        }
    }
}

/// Obtain all file paths from a live Windows drop handle.
///
/// # Safety
/// The caller must keep the HDROP handle valid for the duration of this call.
pub unsafe fn dropped_paths(handle: HDROP) -> Vec<PathBuf> {
    let count = unsafe { DragQueryFileW(handle, u32::MAX, std::ptr::null_mut(), 0) };
    let mut paths = Vec::new();
    for index in 0..count {
        let len = unsafe { DragQueryFileW(handle, index, std::ptr::null_mut(), 0) };
        if len == 0 {
            continue;
        }
        let mut path = vec![0u16; len as usize + 1];
        if unsafe { DragQueryFileW(handle, index, path.as_mut_ptr(), len + 1) } != 0 {
            paths.push(PathBuf::from(from_wide(&path)));
        }
    }
    paths
}

fn read_clipboard(
    owner: HWND,
    receipt: &ClipboardPasteReceipt,
) -> Result<(Vec<PathBuf>, bool, u32), String> {
    let _clipboard = Clipboard::open(owner)?;
    if unsafe { GetClipboardSequenceNumber() } != receipt.expected_sequence()? {
        return Err(CLIPBOARD_CHANGED.into());
    }
    let data = unsafe { GetClipboardData(CF_HDROP as u32) };
    if data.is_null() {
        return Err("The clipboard does not contain files.".into());
    }
    let paths = unsafe { dropped_paths(data) };
    if paths.is_empty() {
        return Err("The clipboard does not contain any readable file paths.".into());
    }
    let effect = unsafe { GetClipboardData(drop_effect_format()) };
    let moving = unsafe {
        if effect.is_null() || GlobalSize(effect) < size_of::<u32>() {
            false
        } else {
            let data = GlobalLock(effect).cast::<u32>();
            if data.is_null() {
                false
            } else {
                let value = data.read_unaligned();
                GlobalUnlock(effect);
                value & 3 == 2
            }
        }
    };
    // Delayed rendering can advance the sequence while we read an external owner's formats.
    Ok((paths, moving, unsafe { GetClipboardSequenceNumber() }))
}

fn clear_clipboard_if_unchanged(owner: HWND, sequence: u32) {
    if let Ok(_clipboard) = Clipboard::open(owner) {
        unsafe {
            if GetClipboardSequenceNumber() == sequence {
                EmptyClipboard();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_records_an_ignored_move_even_when_the_batch_succeeds() {
        let recorded = Arc::new(Mutex::new(None));
        let sink: IFileOperationProgressSink = ResultSink(Arc::clone(&recorded)).into();
        unsafe {
            sink.PostMoveItem(
                0,
                None::<&IShellItem>,
                None::<&IShellItem>,
                PCWSTR::null(),
                COPYENGINE_S_USER_IGNORED,
                None::<&IShellItem>,
            )
            .unwrap();
            sink.FinishOperations(HRESULT(0)).unwrap();
        }
        assert_eq!(*recorded.lock().unwrap(), Some(COPYENGINE_S_USER_IGNORED));
    }
}
