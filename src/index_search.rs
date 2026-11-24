use std::{
    ffi::OsString,
    mem::{offset_of, size_of},
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    ptr::{null, null_mut},
};
use windows::Win32::System::{
    Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    },
    Search::{
        IAccessor, ICommandText, IDBCreateCommand, IDBCreateSession, IDBInitialize,
        IDataInitialize, IRowset, DBACCESSOR_ROWDATA, DBBINDING, DBMEMOWNER_CLIENTOWNED,
        DBPARAMIO_NOTPARAM, DBPART_LENGTH, DBPART_STATUS, DBPART_VALUE, DBSTATUS_S_OK,
        DBTYPE_FILETIME, DBTYPE_UI4, DBTYPE_UI8, DBTYPE_WSTR, HACCESSOR, MSDAINITIALIZE,
    },
};
use windows_core::{w, Interface, GUID, PCWSTR};
use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;

const SQL_DIALECT: GUID = GUID::from_u128(0xc8b521fb_5cf3_11ce_ade5_00aa0044773d);
const PATH_CAPACITY: usize = 32_768;
const FIRST_BATCH: usize = 16;
const BATCH: usize = 256;
const VALUE_OK: u32 = DBSTATUS_S_OK.0 as u32;
const QUERY_COLUMNS: &str =
    "System.ItemUrl, System.FileAttributes, System.Size, System.DateModified";

pub struct IndexedFile {
    pub path: PathBuf,
    pub name: OsString,
    pub directory: bool,
    pub attributes: u32,
    pub size: u64,
    pub modified: u64,
}

pub fn search(
    query: &str,
    stale: impl Fn() -> bool,
    mut publish: impl FnMut(Vec<IndexedFile>),
) -> Result<(), String> {
    let query = query.trim();
    if stale()
        || query.is_empty()
        || query
            .chars()
            .any(|character| character < ' ' || r#"\/:*?"<>|"#.contains(character))
    {
        return Ok(());
    }
    let _com = ComApartment::initialize().map_err(index_unavailable)?;
    let source = open_catalog().map_err(index_unavailable)?;
    if stale() {
        return Ok(());
    }
    if let Some(sql) = prefix_query(query) {
        let lowercase = query.to_lowercase();
        return stream(&source, &sql, &stale, |mut batch| {
            batch.retain(|file| {
                file.name
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&lowercase)
            });
            if !batch.is_empty() {
                publish(batch);
            }
        })
        .map_err(query_failed);
    }
    if stale() {
        return Ok(());
    }
    let sql = format!(
        "SELECT {QUERY_COLUMNS} FROM SYSTEMINDEX WHERE SCOPE='file:' AND System.FileName LIKE '{}'",
        filename_pattern(query),
    );
    stream(&source, &sql, &stale, publish).map_err(query_failed)
}

fn prefix_query(query: &str) -> Option<String> {
    if !query
        .chars()
        .all(|character| character.is_alphanumeric() || character == ' ')
    {
        return None;
    }
    let predicate = if query.len() == 1 && query.as_bytes()[0].is_ascii_alphabetic() {
        format!("System.FileName LIKE '{query}%'")
    } else {
        format!(r#"CONTAINS(System.FileName, '"{query}*"')"#)
    };
    Some(format!(
        "SELECT {QUERY_COLUMNS} FROM SYSTEMINDEX WHERE SCOPE='file:' AND {predicate}"
    ))
}

fn stream(
    source: &IDBInitialize,
    sql: &str,
    stale: &impl Fn() -> bool,
    mut publish: impl FnMut(Vec<IndexedFile>),
) -> windows_core::Result<()> {
    let rowset = execute(source, sql)?;
    if stale() {
        return Ok(());
    }
    let accessor = RowAccessor::new(&rowset)?;
    let mut storage = RowData::new();
    let mut capacity = FIRST_BATCH;
    while !stale() {
        let rows = Rows::fetch(&rowset, capacity)?;
        if rows.count == 0 || stale() {
            break;
        }
        let mut batch = Vec::with_capacity(rows.count);
        for &handle in &rows.handles[..rows.count] {
            if stale() {
                return Ok(());
            }
            // The accessor describes this exact buffer layout. The handle belongs to the rowset.
            unsafe {
                rowset.GetData(
                    handle,
                    accessor.handle,
                    (&mut storage as *mut RowData).cast(),
                )
            }?;
            if let Some(file) = storage.indexed_file() {
                batch.push(file);
            }
        }
        drop(rows);
        if !batch.is_empty() && !stale() {
            publish(batch);
            capacity = BATCH;
        }
    }
    Ok(())
}

fn filename_pattern(query: &str) -> String {
    let mut pattern = String::with_capacity(query.len() + 2);
    pattern.push('%');
    for character in query.chars() {
        match character {
            '\'' => pattern.push_str("''"),
            '%' => pattern.push_str("[%]"),
            '_' => pattern.push_str("[_]"),
            '[' => pattern.push_str("[[]"),
            _ => pattern.push(character),
        }
    }
    pattern.push('%');
    pattern
}

fn indexed_path(url: &[u16]) -> Option<PathBuf> {
    const FILE_PREFIX: &[u16] = &[
        b'f' as u16,
        b'i' as u16,
        b'l' as u16,
        b'e' as u16,
        b':' as u16,
    ];
    const SLASH: u16 = b'/' as u16;
    const BACKSLASH: u16 = b'\\' as u16;
    let path = url.strip_prefix(FILE_PREFIX)?;
    let path = path.strip_prefix(&[SLASH, SLASH, SLASH]).unwrap_or(path);
    let &[drive, colon, slash, ..] = path else {
        return None;
    };
    if !u8::try_from(drive).is_ok_and(|drive| drive.is_ascii_alphabetic())
        || colon != b':' as u16
        || !matches!(slash, SLASH | BACKSLASH)
    {
        return None;
    }
    // The catalog stores a raw filesystem path, including literal percent sequences.
    let path: Vec<_> = path
        .iter()
        .map(|&unit| if unit == SLASH { BACKSLASH } else { unit })
        .collect();
    Some(OsString::from_wide(&path).into())
}

fn index_unavailable(error: windows_core::Error) -> String {
    format!("The Windows Search index is unavailable ({error}).")
}

fn query_failed(error: windows_core::Error) -> String {
    format!("Windows Search could not finish the query ({error}).")
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> windows_core::Result<Self> {
        // The search runs on a dedicated worker. COM interfaces are kept on that thread.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn open_catalog() -> windows_core::Result<IDBInitialize> {
    unsafe {
        let provider: IDataInitialize =
            CoCreateInstance(&MSDAINITIALIZE, None, CLSCTX_INPROC_SERVER)?;
        let mut source = None;
        provider.GetDataSource(
            None,
            CLSCTX_INPROC_SERVER.0,
            w!("Provider=Search.CollatorDSO.1;Extended Properties=\"Application=Windows\";"),
            &IDBInitialize::IID,
            &mut source,
        )?;
        let source: IDBInitialize = source.ok_or_else(unexpected_provider_result)?.cast()?;
        source.Initialize()?;
        Ok(source)
    }
}

fn execute(source: &IDBInitialize, query: &str) -> windows_core::Result<IRowset> {
    let query: Vec<u16> = query.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let session: IDBCreateCommand = source
            .cast::<IDBCreateSession>()?
            .CreateSession(None, &IDBCreateCommand::IID)?
            .cast()?;
        let command: ICommandText = session.CreateCommand(None, &ICommandText::IID)?.cast()?;
        command.SetCommandText(&SQL_DIALECT, PCWSTR(query.as_ptr()))?;
        let mut rows = None;
        command.Execute(None, &IRowset::IID, None, None, Some(&mut rows))?;
        rows.ok_or_else(unexpected_provider_result)?.cast()
    }
}

fn unexpected_provider_result() -> windows_core::Error {
    windows_core::Error::new(
        windows::Win32::Foundation::E_UNEXPECTED,
        "The catalog provider returned an incomplete result",
    )
}

#[repr(C)]
struct RowData {
    url_status: u32,
    attributes_status: u32,
    size_status: u32,
    modified_status: u32,
    url_bytes: usize,
    attributes: u32,
    size: u64,
    modified: u64,
    url: [u16; PATH_CAPACITY],
}

impl RowData {
    fn new() -> Self {
        Self {
            url_status: 0,
            attributes_status: 0,
            size_status: 0,
            modified_status: 0,
            url_bytes: 0,
            attributes: 0,
            size: 0,
            modified: 0,
            url: [0; PATH_CAPACITY],
        }
    }

    fn indexed_file(&self) -> Option<IndexedFile> {
        if self.url_status != VALUE_OK || self.attributes_status != VALUE_OK {
            return None;
        }
        let path = indexed_path(self.url.get(..self.url_bytes / size_of::<u16>())?)?;
        let name = path.file_name()?.to_owned();
        Some(IndexedFile {
            path,
            name,
            directory: self.attributes & FILE_ATTRIBUTE_DIRECTORY != 0,
            attributes: self.attributes,
            size: if self.size_status == VALUE_OK {
                self.size
            } else {
                0
            },
            modified: if self.modified_status == VALUE_OK {
                self.modified
            } else {
                0
            },
        })
    }
}

struct RowAccessor {
    accessor: IAccessor,
    handle: HACCESSOR,
}

impl RowAccessor {
    fn new(rowset: &IRowset) -> windows_core::Result<Self> {
        let binding = |ordinal, value, status, kind, size| DBBINDING {
            iOrdinal: ordinal,
            obValue: value,
            obStatus: status,
            dwPart: (DBPART_VALUE.0 | DBPART_STATUS.0) as u32,
            dwMemOwner: DBMEMOWNER_CLIENTOWNED.0 as u32,
            eParamIO: DBPARAMIO_NOTPARAM.0 as u32,
            cbMaxLen: size,
            wType: kind,
            ..DBBINDING::default()
        };
        let mut bindings = [
            binding(
                1,
                offset_of!(RowData, url),
                offset_of!(RowData, url_status),
                DBTYPE_WSTR.0 as u16,
                size_of::<[u16; PATH_CAPACITY]>(),
            ),
            binding(
                2,
                offset_of!(RowData, attributes),
                offset_of!(RowData, attributes_status),
                DBTYPE_UI4.0 as u16,
                size_of::<u32>(),
            ),
            binding(
                3,
                offset_of!(RowData, size),
                offset_of!(RowData, size_status),
                DBTYPE_UI8.0 as u16,
                size_of::<u64>(),
            ),
            binding(
                4,
                offset_of!(RowData, modified),
                offset_of!(RowData, modified_status),
                DBTYPE_FILETIME.0 as u16,
                size_of::<u64>(),
            ),
        ];
        bindings[0].obLength = offset_of!(RowData, url_bytes);
        bindings[0].dwPart |= DBPART_LENGTH.0 as u32;
        let accessor: IAccessor = rowset.cast()?;
        let mut handle = HACCESSOR::default();
        unsafe {
            accessor.CreateAccessor(
                DBACCESSOR_ROWDATA.0 as u32,
                bindings.len(),
                bindings.as_ptr(),
                size_of::<RowData>(),
                &mut handle,
                None,
            )?;
        }
        Ok(Self { accessor, handle })
    }
}

impl Drop for RowAccessor {
    fn drop(&mut self) {
        let _ = unsafe { self.accessor.ReleaseAccessor(self.handle, None) };
    }
}

struct Rows<'a> {
    rowset: &'a IRowset,
    handles: [usize; BATCH],
    count: usize,
}

impl<'a> Rows<'a> {
    fn fetch(rowset: &'a IRowset, capacity: usize) -> windows_core::Result<Self> {
        let mut rows = Self {
            rowset,
            handles: [0; BATCH],
            count: 0,
        };
        let mut handles = rows.handles.as_mut_ptr();
        // OLE DB takes a pointer to the caller's handle-buffer pointer.
        unsafe {
            (rowset.vtable().GetNextRows)(
                rowset.as_raw(),
                0,
                0,
                capacity as isize,
                &mut rows.count,
                &mut handles,
            )
            .ok()?;
        }
        Ok(rows)
    }
}

impl Drop for Rows<'_> {
    fn drop(&mut self) {
        if self.count != 0 {
            let _ = unsafe {
                self.rowset.ReleaseRows(
                    self.count,
                    self.handles.as_ptr(),
                    null(),
                    null_mut(),
                    null_mut(),
                )
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_literal_search_syntax() {
        assert_eq!(filename_pattern("50%_[x]'s"), "%50[%][_][[]x]''s%");
    }

    #[test]
    fn converts_local_urls_without_decoding_filename() {
        let url: Vec<_> = "file:D:/Ordner/Österreich%2F [1].txt"
            .encode_utf16()
            .collect();
        assert_eq!(
            indexed_path(&url),
            Some(PathBuf::from(r"D:\Ordner\Österreich%2F [1].txt"))
        );
        let remote: Vec<_> = "file://server/share/example.txt".encode_utf16().collect();
        assert!(indexed_path(&remote).is_none());
    }
}
