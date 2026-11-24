use crate::model::{safe_name, wide};
use std::cmp::Ordering;
use std::collections::{btree_map::Entry, BTreeMap};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Seek};
use std::mem::{offset_of, size_of, size_of_val};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{
    ERROR_HANDLE_EOF, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER,
    ERROR_MORE_DATA, ERROR_NOT_SUPPORTED,
};
use windows_sys::Win32::Globalization::{
    CompareStringOrdinal, CSTR_EQUAL, CSTR_GREATER_THAN, CSTR_LESS_THAN,
};
use windows_sys::Win32::Storage::FileSystem::{
    FileIdInfo, FileStreamInfo, GetFileInformationByHandleEx, GetVolumeInformationByHandleW,
    MoveFileExW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_SHARE_READ, FILE_STREAM_INFO,
};
use windows_sys::Win32::System::SystemServices::FILE_NAMED_STREAMS;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipReadOptions, ZipWriter};

const ZONE_IDENTIFIER: &str = ":Zone.Identifier:$DATA";

pub fn compress(sources: &[PathBuf], output: &Path) -> Result<(), String> {
    if sources.is_empty() {
        return Err("Select at least one item to compress.".into());
    }
    let (parent, output) = new_destination(output)?;
    let mut staged = tempfile::NamedTempFile::new_in(&parent).map_err(|err| at(&parent, err))?;
    let mut zip = ZipWriter::new(staged.as_file_mut());
    let mut paths = ArchivePaths::default();

    for path in sources {
        let name = checked_name(path.file_name(), path)?;
        let (source, metadata) = open_source(path)?;
        if metadata.is_dir() {
            let canonical = fs::canonicalize(path).map_err(|err| at(path, err))?;
            let canonical = WindowsPath::new(canonical.as_os_str());
            if parent
                .ancestors()
                .any(|p| WindowsPath::new(p.as_os_str()) == canonical)
            {
                return Err(at(
                    path,
                    "Choose an archive location outside the source folder.",
                ));
            }
        }
        add_source(&mut zip, &mut paths, path, name, source, metadata)?;
    }

    zip.finish().map_err(|err| at(&output, err))?;
    staged
        .persist_noclobber(&output)
        .map_err(|err| at(&output, err.error))?;
    Ok(())
}

pub fn extract(source: &Path, output: &Path) -> Result<(), String> {
    let (parent, output) = new_destination(output)?;
    let (input, metadata) = open_source(source)?;
    if !metadata.is_file() {
        return Err(at(source, "Select a ZIP file."));
    }
    let mut zone = open_zone_identifier(source, &input)?;
    let mut zip = ZipArchive::new(BufReader::new(input)).map_err(|err| at(source, err))?;
    if zip.has_duplicate_names() {
        return Err(at(source, "The ZIP contains duplicate names."));
    }
    let staged = tempfile::tempdir_in(&parent).map_err(|err| at(&parent, err))?;
    let mut paths = ArchivePaths::default();
    for index in 0..zip.len() {
        let options = ZipReadOptions::new().enforce_header_consistency(true);
        let mut entry = zip
            .by_index_with_options(index, options)
            .map_err(|err| at(source, err))?;
        let name = entry
            .name()
            .map_err(|err| at(source, err))?
            .replace('\\', "/");
        let file_type = entry.unix_mode().unwrap_or(0) & 0o170000;
        if !matches!(file_type, 0 | 0o040000 | 0o100000)
            || entry.external_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err(format!("Unsupported ZIP entry type for {name:?}."));
        }
        let directory = entry.is_dir() || name.ends_with('/') || file_type == 0o040000;
        let target = extract_path(staged.path(), &mut paths, &name, directory)?;
        if directory {
            if entry.size() != 0 || entry.read(&mut [0]).map_err(|err| at(&target, err))? != 0 {
                return Err(format!("ZIP directory {name:?} contains file data."));
            }
        } else {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .share_mode(FILE_SHARE_READ)
                .open(&target)
                .map_err(|err| at(&target, err))?;
            if let Some(zone) = zone.as_mut() {
                let stream = zone_identifier_path(&target);
                write_zone_identifier(&file, &stream, zone).map_err(|err| at(&stream, err))?;
            }
            let written = io::copy(&mut entry, &mut file).map_err(|err| at(&target, err))?;
            if written != entry.size() {
                return Err(format!("ZIP entry {name:?} has an incorrect size."));
            }
        }
    }

    let staged_path = wide(staged.path());
    let output_path = wide(&output);
    // With zero flags, MoveFileExW refuses to replace an existing destination.
    if unsafe { MoveFileExW(staged_path.as_ptr(), output_path.as_ptr(), 0) } == 0 {
        return Err(at(&output, io::Error::last_os_error()));
    }
    let _ = staged.keep();
    Ok(())
}

fn new_destination(output: &Path) -> Result<(PathBuf, PathBuf), String> {
    let name = checked_name(output.file_name(), output)?;
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|err| at(parent, err))?;
    let output = parent.join(name);
    match fs::symlink_metadata(&output) {
        Ok(_) => Err(at(&output, "The destination already exists.")),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok((parent, output)),
        Err(err) => Err(at(&output, err)),
    }
}

fn open_source(path: &Path) -> Result<(File, fs::Metadata), String> {
    // Open the entry itself and keep it in place while reading it.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .map_err(|err| at(path, err))?;
    let metadata = file.metadata().map_err(|err| at(path, err))?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(at(
            path,
            "Links and reparse points cannot be archived or extracted.",
        ));
    }
    Ok((file, metadata))
}

fn open_zone_identifier(source: &Path, input: &File) -> Result<Option<File>, String> {
    // Check the archive handle so a different file at the same path cannot hide its mark.
    if !has_zone_identifier(input).map_err(|err| at(source, err))? {
        return Ok(None);
    }
    let path = zone_identifier_path(source);
    let (stream, _) = open_source(&path)?;
    check_stream_file(input, &stream).map_err(|err| at(&path, err))?;
    Ok(Some(stream))
}

fn has_zone_identifier(file: &File) -> io::Result<bool> {
    // FILE_STREAM_INFO requires an eight-byte aligned buffer.
    let mut buffer = vec![0u64; 128];
    loop {
        let size = u32::try_from(size_of_val(buffer.as_slice())).map_err(io::Error::other)?;
        let result = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileStreamInfo,
                buffer.as_mut_ptr().cast(),
                size,
            )
        };
        if result != 0 {
            // The initialized u64 allocation is also valid to read as bytes.
            let bytes =
                unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), size as usize) };
            return stream_info_has_zone_identifier(bytes);
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error().map(|code| code as u32) {
            Some(ERROR_HANDLE_EOF) => return Ok(false),
            Some(ERROR_INVALID_FUNCTION | ERROR_INVALID_PARAMETER | ERROR_NOT_SUPPORTED)
                if volume_has_no_streams(file) =>
            {
                return Ok(false);
            }
            Some(ERROR_MORE_DATA | ERROR_INSUFFICIENT_BUFFER) => {
                buffer
                    .try_reserve_exact(buffer.len())
                    .map_err(io::Error::other)?;
                buffer.resize(buffer.len() * 2, 0);
            }
            _ => return Err(error),
        }
    }
}

fn stream_info_has_zone_identifier(mut bytes: &[u8]) -> io::Result<bool> {
    const NAME_OFFSET: usize = offset_of!(FILE_STREAM_INFO, StreamName);
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid file stream information.",
        )
    };
    loop {
        let header = bytes.get(..NAME_OFFSET).ok_or_else(invalid)?;
        let next = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
        let name_len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        let name = bytes[NAME_OFFSET..].get(..name_len).ok_or_else(invalid)?;
        if name.len() == ZONE_IDENTIFIER.len() * 2
            && name
                .as_chunks::<2>()
                .0
                .iter()
                .zip(ZONE_IDENTIFIER.bytes())
                .all(|(unit, expected)| unit[1] == 0 && unit[0].eq_ignore_ascii_case(&expected))
        {
            return Ok(true);
        }
        if next == 0 {
            return Ok(false);
        }
        if next < NAME_OFFSET + name_len {
            return Err(invalid());
        }
        bytes = bytes.get(next..).ok_or_else(invalid)?;
    }
}

fn volume_has_no_streams(file: &File) -> bool {
    let mut flags = 0;
    let result = unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            null_mut(),
            0,
            null_mut(),
            null_mut(),
            &mut flags,
            null_mut(),
            0,
        )
    };
    result != 0 && flags & FILE_NAMED_STREAMS == 0
}

fn write_zone_identifier(file: &File, path: &Path, zone: &mut File) -> io::Result<()> {
    let mut stream = OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .share_mode(FILE_SHARE_READ)
        .open(path)?;
    check_stream_file(file, &stream)?;
    zone.rewind()?;
    io::copy(zone, &mut stream)?;
    Ok(())
}

fn check_stream_file(file: &File, stream: &File) -> io::Result<()> {
    let file = file_id(file)?;
    let stream = file_id(stream)?;
    if file.VolumeSerialNumber != stream.VolumeSerialNumber
        || file.FileId.Identifier != stream.FileId.Identifier
    {
        return Err(io::Error::other(
            "The origin stream does not belong to the expected file.",
        ));
    }
    Ok(())
}

fn file_id(file: &File) -> io::Result<FILE_ID_INFO> {
    let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    let result = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

fn zone_identifier_path(file: &Path) -> PathBuf {
    let mut stream = file.as_os_str().to_owned();
    stream.push(ZONE_IDENTIFIER);
    stream.into()
}

fn checked_name<'a>(name: Option<&'a OsStr>, path: &Path) -> Result<&'a str, String> {
    name.and_then(OsStr::to_str)
        .filter(|name| safe_name(name))
        .ok_or_else(|| at(path, "Invalid Windows file name."))
}

fn add_source(
    zip: &mut ZipWriter<&mut File>,
    paths: &mut ArchivePaths,
    path: &Path,
    name: &str,
    mut source: File,
    metadata: fs::Metadata,
) -> Result<(), String> {
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    if metadata.is_dir() {
        paths.insert(name, EntryType::Directory)?;
        zip.add_directory(name, options)
            .map_err(|err| at(path, err))?;
        for entry in fs::read_dir(path).map_err(|err| at(path, err))? {
            let entry = entry.map_err(|err| at(path, err))?;
            let child = entry.path();
            let component = checked_name(child.file_name(), &child)?;
            let (file, metadata) = open_source(&child)?;
            add_source(
                zip,
                paths,
                &child,
                &format!("{name}/{component}"),
                file,
                metadata,
            )?;
        }
    } else if metadata.is_file() {
        paths.insert(name, EntryType::File)?;
        // Allow room for Deflate overhead as the input approaches the ZIP32 limit.
        let options = options.large_file(metadata.len() >= u64::from(u32::MAX) - (16 << 20));
        zip.start_file(name, options).map_err(|err| at(path, err))?;
        io::copy(&mut source, zip).map_err(|err| at(path, err))?;
    } else {
        return Err(at(path, "This type of file cannot be archived."));
    }
    Ok(())
}

fn extract_path(
    root: &Path,
    paths: &mut ArchivePaths,
    name: &str,
    directory: bool,
) -> Result<PathBuf, String> {
    let name = name.strip_suffix('/').unwrap_or(name);
    let mut components = name.split('/').peekable();
    let mut relative = String::new();
    let mut target = root.to_path_buf();
    while let Some(component) = components.next() {
        if !safe_name(component) {
            return Err(format!("Unsafe ZIP entry path {name:?}."));
        }
        if !relative.is_empty() {
            relative.push('/');
        }
        relative.push_str(component);
        target.push(component);
        let kind = match (components.peek().is_some(), directory) {
            (true, _) => EntryType::ImplicitDirectory,
            (false, true) => EntryType::Directory,
            (false, false) => EntryType::File,
        };
        if paths.insert(&relative, kind)? && kind != EntryType::File {
            fs::create_dir(&target).map_err(|err| at(&target, err))?;
        }
    }
    Ok(target)
}

fn at(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EntryType {
    ImplicitDirectory,
    Directory,
    File,
}

#[derive(Default)]
struct ArchivePaths(BTreeMap<WindowsPath, (String, EntryType)>);

impl ArchivePaths {
    fn insert(&mut self, path: &str, kind: EntryType) -> Result<bool, String> {
        match self.0.entry(WindowsPath::new(OsStr::new(path))) {
            Entry::Vacant(entry) => {
                entry.insert((path.to_owned(), kind));
                Ok(true)
            }
            Entry::Occupied(mut entry) => {
                let (existing, existing_kind) = entry.get_mut();
                if existing == path {
                    match (*existing_kind, kind) {
                        (
                            EntryType::Directory | EntryType::ImplicitDirectory,
                            EntryType::ImplicitDirectory,
                        ) => {
                            return Ok(false);
                        }
                        (EntryType::ImplicitDirectory, EntryType::Directory) => {
                            *existing_kind = kind;
                            return Ok(false);
                        }
                        _ => {}
                    }
                }
                Err(format!("Conflicting ZIP paths {existing:?} and {path:?}."))
            }
        }
    }
}

#[derive(Eq)]
struct WindowsPath(Vec<u16>);

impl WindowsPath {
    fn new(path: &OsStr) -> Self {
        Self(wide(path))
    }
}

impl Ord for WindowsPath {
    fn cmp(&self, other: &Self) -> Ordering {
        match unsafe { CompareStringOrdinal(self.0.as_ptr(), -1, other.0.as_ptr(), -1, 1) } {
            CSTR_LESS_THAN => Ordering::Less,
            CSTR_EQUAL => Ordering::Equal,
            CSTR_GREATER_THAN => Ordering::Greater,
            _ => self.0.cmp(&other.0),
        }
    }
}

impl PartialOrd for WindowsPath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for WindowsPath {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
