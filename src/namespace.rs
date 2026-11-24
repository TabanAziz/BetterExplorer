use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet, ffi::OsString, mem::size_of, os::windows::ffi::OsStringExt, path::PathBuf,
};
use windows::{
    core::{Interface, GUID, PCSTR, PCWSTR, PSTR, PWSTR},
    Win32::{
        Foundation::{FILETIME, HWND, PROPERTYKEY},
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ,
            },
            SystemServices::SFGAO_FOLDER,
        },
        UI::{
            Shell::{
                BHID_EnumItems, BHID_SFObject, Common::ITEMIDLIST, FOLDERID_Libraries,
                FOLDERID_NetworkFolder, FOLDERID_RecycleBinFolder, IContextMenu, IEnumShellItems,
                ILFindLastID, IShellFolder, IShellItem, IShellItem2, IShellItemArray,
                IShellLibrary, SHCreateItemFromParsingName, SHGetIDListFromObject,
                SHGetKnownFolderItem, ShellLibrary, CMF_NORMAL, CMINVOKECOMMANDINFO, GCS_VERBA,
                GCS_VERBW, KF_FLAG_DEFAULT, LFF_FORCEFILESYSTEM, SEE_MASK_NOASYNC, SIGDN,
                SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_FILESYSPATH, SIGDN_NORMALDISPLAY,
            },
            WindowsAndMessaging::{
                CreatePopupMenu, DestroyMenu, GetMenuState, HMENU, MF_BYCOMMAND, MF_DISABLED,
                MF_GRAYED, SW_SHOWNORMAL,
            },
        },
    },
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum Namespace {
    Network,
    RecycleBin,
    Libraries,
}

#[derive(Debug)]
pub struct Item {
    pub name: String,
    pub extension: Option<OsString>,
    pub target: Option<PathBuf>,
    pub directory: bool,
    pub key: String,
    pub kind: String,
    pub modified: u64,
    pub deleted: u64,
    pub size: u64,
    pub original_location: Option<PathBuf>,
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
        unsafe { CoUninitialize() };
    }
}

pub fn enumerate(kind: Namespace, stale: impl Fn() -> bool) -> Result<Vec<Item>, String> {
    if stale() {
        return Ok(Vec::new());
    }
    let result = || {
        let _apartment = Apartment::start()?;
        let folder = root(kind)?;
        shell_entries(&folder, kind, &stale)
    };
    result().map_err(|error| error.to_string())
}

pub fn enumerate_child(
    kind: Namespace,
    key: &str,
    stale: impl Fn() -> bool,
) -> Result<Vec<Item>, String> {
    if stale() {
        return Ok(Vec::new());
    }
    if kind == Namespace::RecycleBin {
        return Err("Recycle Bin items cannot be browsed.".into());
    }
    let result = || {
        let _apartment = Apartment::start()?;
        let folder = parse_item(key)?;
        if kind == Namespace::Libraries {
            library_folders(&folder, &stale)
        } else {
            shell_entries(&folder, kind, &stale)
        }
    };
    result().map_err(|error| error.to_string())
}

fn root(kind: Namespace) -> windows::core::Result<IShellItem> {
    let id = match kind {
        Namespace::Network => &FOLDERID_NetworkFolder,
        Namespace::RecycleBin => &FOLDERID_RecycleBinFolder,
        Namespace::Libraries => &FOLDERID_Libraries,
    };
    unsafe { SHGetKnownFolderItem(id, KF_FLAG_DEFAULT, None) }
}

fn parse_item(key: &str) -> windows::core::Result<IShellItem> {
    let key: Vec<_> = key.encode_utf16().chain(Some(0)).collect();
    unsafe { SHCreateItemFromParsingName(PCWSTR(key.as_ptr()), None) }
}

fn shell_entries(
    folder: &IShellItem,
    kind: Namespace,
    stale: &impl Fn() -> bool,
) -> windows::core::Result<Vec<Item>> {
    let enumerator: IEnumShellItems = unsafe { folder.BindToHandler(None, &BHID_EnumItems)? };
    let mut entries = Vec::new();
    while !stale() {
        let Some(item) = next_item(&enumerator)? else {
            break;
        };
        if let Some(entry) = shell_entry(&item, kind) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

fn next_item(enumerator: &IEnumShellItems) -> windows::core::Result<Option<IShellItem>> {
    let mut next = [None];
    unsafe { enumerator.Next(&mut next, None)? };
    let [item] = next;
    Ok(item)
}

fn item_key(item: &IShellItem) -> Option<String> {
    let name =
        name(item, SIGDN_DESKTOPABSOLUTEPARSING).or_else(|| name(item, SIGDN_FILESYSPATH))?;
    Some(name.to_string_lossy().into_owned())
}

fn shell_entry(item: &IShellItem, namespace: Namespace) -> Option<Item> {
    let key = item_key(item)?;
    let display_name = name(item, SIGDN_NORMALDISPLAY)?
        .to_string_lossy()
        .into_owned();
    let directory = unsafe { item.GetAttributes(SFGAO_FOLDER) }
        .is_ok_and(|attributes| attributes.contains(SFGAO_FOLDER));
    let (target, directory, kind) = match namespace {
        // A server alone has no filesystem path. Its shares are enumerated through the Shell.
        Namespace::Network => (
            name(item, SIGDN_FILESYSPATH)
                .map(PathBuf::from)
                .filter(|value| value.is_absolute())
                .or_else(|| {
                    let unc_path = PathBuf::from(&key);
                    (key.starts_with(r"\\") && unc_path.is_absolute()).then_some(unc_path)
                }),
            directory,
            if directory {
                "Network folder"
            } else {
                "Network device"
            },
        ),
        Namespace::Libraries => (None, true, "Library"),
        Namespace::RecycleBin => (
            None,
            directory,
            if directory { "File folder" } else { "File" },
        ),
    };
    let mut entry = Item {
        name: display_name,
        extension: None,
        target,
        directory,
        key,
        kind: kind.into(),
        modified: 0,
        deleted: 0,
        size: 0,
        original_location: None,
    };
    if namespace == Namespace::RecycleBin {
        if let Ok(properties) = item.cast::<IShellItem2>() {
            recycle_metadata(&mut entry, &properties);
        }
    }
    Some(entry)
}

struct Pidl(*mut ITEMIDLIST);

impl Drop for Pidl {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0.cast())) };
    }
}

struct PopupMenu(HMENU);

impl Drop for PopupMenu {
    fn drop(&mut self) {
        let _ = unsafe { DestroyMenu(self.0) };
    }
}

pub fn recycle_action(keys: &[String], restore: bool, owner: usize) -> Result<(), String> {
    let keys: HashSet<_> = keys.iter().map(String::as_str).collect();
    if keys.is_empty() {
        return Err("Select at least one Recycle Bin item.".into());
    }
    let owner = HWND(owner as _);
    let _apartment = Apartment::start().map_err(|error| error.to_string())?;
    let folder = root(Namespace::RecycleBin).map_err(|error| error.to_string())?;
    let selected = recycle_selection(&folder, &keys).map_err(|error| error.to_string())?;
    if selected.len() != keys.len() {
        return Err("One or more selected items are no longer in the Recycle Bin.".into());
    }
    let context = recycle_menu(&folder, &selected, owner).map_err(|error| error.to_string())?;
    let verb = if restore { "undelete" } else { "delete" };
    let (_popup, command) = available_command(&context, verb)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Windows does not offer that action for the selected items.".to_owned())?;
    let invocation = CMINVOKECOMMANDINFO {
        cbSize: size_of::<CMINVOKECOMMANDINFO>() as _,
        fMask: SEE_MASK_NOASYNC,
        hwnd: owner,
        lpVerb: PCSTR(command as usize as *const u8),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { context.InvokeCommand(&invocation) }.map_err(|error| error.to_string())?;

    // Some handlers return success when their confirmation dialog is cancelled.
    let folder = root(Namespace::RecycleBin).map_err(|error| error.to_string())?;
    let remaining = recycle_selection(&folder, &keys).map_err(|error| error.to_string())?;
    if !remaining.is_empty() {
        return Err(format!(
            "{} selected item(s) are still in the Recycle Bin.",
            remaining.len()
        ));
    }
    Ok(())
}

fn recycle_selection(
    folder: &IShellItem,
    keys: &HashSet<&str>,
) -> windows::core::Result<Vec<Pidl>> {
    let contents: IEnumShellItems = unsafe { folder.BindToHandler(None, &BHID_EnumItems)? };
    let mut remaining = keys.clone();
    let mut selected = Vec::with_capacity(remaining.len());
    while !remaining.is_empty() {
        let Some(item) = next_item(&contents)? else {
            break;
        };
        if item_key(&item).is_some_and(|key| remaining.remove(key.as_str())) {
            selected.push(Pidl(unsafe { SHGetIDListFromObject(&item)? }));
        }
    }
    Ok(selected)
}

fn recycle_menu(
    folder: &IShellItem,
    selected: &[Pidl],
    owner: HWND,
) -> windows::core::Result<IContextMenu> {
    let children: Vec<*const ITEMIDLIST> = selected
        .iter()
        .map(|pidl| unsafe { ILFindLastID(pidl.0).cast_const() })
        .collect();
    let parent: IShellFolder = unsafe { folder.BindToHandler(None, &BHID_SFObject)? };
    unsafe { parent.GetUIObjectOf(owner, &children, None) }
}

fn available_command(
    context: &IContextMenu,
    verb: &str,
) -> windows::core::Result<Option<(PopupMenu, u32)>> {
    let menu = PopupMenu(unsafe { CreatePopupMenu()? });
    let first = 1;
    let status = unsafe { context.QueryContextMenu(menu.0, 0, first, u16::MAX.into(), CMF_NORMAL) };
    status.ok()?;
    let count = status.0 as u32 & u32::from(u16::MAX);
    let command = (0..count).find(|command| {
        let state = unsafe { GetMenuState(menu.0, first + command, MF_BYCOMMAND) };
        state != u32::MAX
            && state & (MF_DISABLED.0 | MF_GRAYED.0) == 0
            && matches_verb(context, *command, verb)
    });
    Ok(command.map(|command| (menu, command)))
}

fn matches_verb(context: &IContextMenu, command: u32, expected: &str) -> bool {
    let mut wide = [0u16; 64];
    let mut ansi = [0u8; 64];
    unsafe {
        if context
            .GetCommandString(
                command as usize,
                GCS_VERBW,
                None,
                PSTR(wide.as_mut_ptr().cast()),
                wide.len() as u32,
            )
            .is_ok()
        {
            let written = wide
                .iter()
                .position(|&unit| unit == 0)
                .unwrap_or(wide.len());
            if String::from_utf16_lossy(&wide[..written]).eq_ignore_ascii_case(expected) {
                return true;
            }
        }
        if context
            .GetCommandString(
                command as usize,
                GCS_VERBA,
                None,
                PSTR(ansi.as_mut_ptr()),
                ansi.len() as u32,
            )
            .is_err()
        {
            return false;
        }
    }
    let written = ansi
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(ansi.len());
    ansi[..written].eq_ignore_ascii_case(expected.as_bytes())
}

fn library_folders(
    folder: &IShellItem,
    stale: &impl Fn() -> bool,
) -> windows::core::Result<Vec<Item>> {
    unsafe {
        let library: IShellLibrary = CoCreateInstance(&ShellLibrary, None, CLSCTX_INPROC_SERVER)?;
        library.LoadLibraryFromItem(folder, STGM_READ.0)?;
        let folders: IShellItemArray = library.GetFolders(LFF_FORCEFILESYSTEM)?;
        let mut entries = Vec::new();
        for index in 0..folders.GetCount()? {
            if stale() {
                break;
            }
            let item = folders.GetItemAt(index)?;
            let Some(path) = name(&item, SIGDN_FILESYSPATH).map(PathBuf::from) else {
                continue;
            };
            let display = name(&item, SIGDN_NORMALDISPLAY)
                .or_else(|| path.file_name().map(|value| value.to_owned()))
                .unwrap_or_else(|| path.as_os_str().to_owned());
            entries.push(Item {
                name: display.to_string_lossy().into_owned(),
                extension: None,
                key: path.to_string_lossy().into_owned(),
                target: Some(path),
                directory: true,
                kind: "File folder".into(),
                modified: 0,
                deleted: 0,
                size: 0,
                original_location: None,
            });
        }
        Ok(entries)
    }
}

fn name(item: &IShellItem, format: SIGDN) -> Option<OsString> {
    read_shell_string(unsafe { item.GetDisplayName(format).ok()? })
}

fn read_shell_string(value: PWSTR) -> Option<OsString> {
    if value.is_null() {
        return None;
    }
    // Shell display names and property strings are allocated through the COM task allocator.
    let string = unsafe { OsString::from_wide(value.as_wide()) };
    unsafe { CoTaskMemFree(Some(value.0.cast())) };
    (!string.is_empty()).then_some(string)
}

// These property IDs are defined by propkey.h and ShlGuid.h in the Windows SDK.
const DOCUMENT: GUID = GUID::from_u128(0xb725f130_47ef_101a_a5f1_02608c9eebac);
const DISPLACED: GUID = GUID::from_u128(0x9b174b33_40ff_11d2_a27e_00c04fc30871);
const ITEM_TYPE_TEXT: PROPERTYKEY = PROPERTYKEY {
    fmtid: DOCUMENT,
    pid: 4,
};
const FILE_EXTENSION: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xe4f10a3c_49e6_405d_8288_a23bd4eeaa6c),
    pid: 100,
};
const SIZE: PROPERTYKEY = PROPERTYKEY {
    fmtid: DOCUMENT,
    pid: 12,
};
const DATE_MODIFIED: PROPERTYKEY = PROPERTYKEY {
    fmtid: DOCUMENT,
    pid: 14,
};
const DELETED_FROM: PROPERTYKEY = PROPERTYKEY {
    fmtid: DISPLACED,
    pid: 2,
};
const DATE_DELETED: PROPERTYKEY = PROPERTYKEY {
    fmtid: DISPLACED,
    pid: 3,
};

fn filetime(value: FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}

fn recycle_metadata(entry: &mut Item, item: &IShellItem2) {
    unsafe {
        if !entry.directory {
            entry.extension = item
                .GetString(&FILE_EXTENSION)
                .ok()
                .and_then(read_shell_string)
                .and_then(|extension| {
                    extension
                        .to_str()?
                        .strip_prefix('.')
                        .filter(|extension| !extension.is_empty())
                        .map(OsString::from)
                });
        }
        entry.modified = item.GetFileTime(&DATE_MODIFIED).map_or(0, filetime);
        entry.deleted = item.GetFileTime(&DATE_DELETED).map_or(0, filetime);
        entry.size = item.GetUInt64(&SIZE).unwrap_or(0);
        entry.original_location = item
            .GetString(&DELETED_FROM)
            .ok()
            .and_then(read_shell_string)
            .map(PathBuf::from);
        if let Some(description) = item
            .GetString(&ITEM_TYPE_TEXT)
            .ok()
            .and_then(read_shell_string)
        {
            entry.kind = description.to_string_lossy().into_owned();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{core::w, Win32::UI::Shell::LSF_FAILIFTHERE};

    #[test]
    fn recycle_bin_can_be_listed_without_an_explorer_window() {
        let entries = enumerate(Namespace::RecycleBin, || false).unwrap();
        assert!(entries
            .iter()
            .all(|entry| !entry.key.is_empty() && entry.target.is_none()));
    }

    #[test]
    #[ignore = "Requires two items already present in the local Windows Recycle Bin."]
    fn recycle_menu_offers_commands_for_a_multi_selection() {
        let entries = enumerate(Namespace::RecycleBin, || false).unwrap();
        assert!(entries.len() >= 2);
        let keys: HashSet<_> = entries.iter().map(|entry| &entry.key).collect();
        assert_eq!(keys.len(), entries.len());
        println!(
            "Listed {} items, with original location/deletion time on {}/{}.",
            entries.len(),
            entries
                .iter()
                .filter(|entry| entry.original_location.is_some())
                .count(),
            entries.iter().filter(|entry| entry.deleted != 0).count(),
        );
        let selected: HashSet<_> = entries
            .iter()
            .take(2)
            .map(|entry| entry.key.as_str())
            .collect();
        let _apartment = Apartment::start().unwrap();
        let folder = root(Namespace::RecycleBin).unwrap();
        let selected = recycle_selection(&folder, &selected).unwrap();
        assert_eq!(selected.len(), 2);
        let menu = recycle_menu(&folder, &selected, HWND::default()).unwrap();
        assert!(available_command(&menu, "undelete").unwrap().is_some());
        assert!(available_command(&menu, "delete").unwrap().is_some());
    }

    #[test]
    fn recycle_action_rejects_a_missing_selection_before_invoking_a_command() {
        let keys = vec![String::from(
            "this is not a native Recycle Bin parsing name",
        )];
        let error = recycle_action(&keys, true, 0).unwrap_err();
        assert_eq!(
            error,
            "One or more selected items are no longer in the Recycle Bin."
        );
    }

    #[test]
    fn opening_a_library_lists_its_configured_folders() {
        let fixture = tempfile::tempdir().unwrap();
        let first = fixture.path().join("First folder");
        let second = fixture.path().join("Second folder");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();

        let _apartment = Apartment::start().unwrap();
        let key = unsafe {
            let library: IShellLibrary =
                CoCreateInstance(&ShellLibrary, None, CLSCTX_INPROC_SERVER).unwrap();
            for folder in [&first, &second] {
                library
                    .AddFolder(&parse_item(&folder.to_string_lossy()).unwrap())
                    .unwrap();
            }
            let parent = parse_item(&fixture.path().to_string_lossy()).unwrap();
            let item = library
                .Save(&parent, w!("Test library"), LSF_FAILIFTHERE)
                .unwrap();
            name(&item, SIGDN_DESKTOPABSOLUTEPARSING).unwrap()
        };
        let entries =
            enumerate_child(Namespace::Libraries, &key.to_string_lossy(), || false).unwrap();
        let expected = [
            first.canonicalize().unwrap(),
            second.canonicalize().unwrap(),
        ];
        let actual: Vec<_> = entries
            .iter()
            .map(|entry| entry.target.as_ref().unwrap().canonicalize().unwrap())
            .collect();
        assert_eq!(actual, expected);
    }
}
