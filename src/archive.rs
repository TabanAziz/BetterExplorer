use crate::model::{safe_name, wide};
use sevenz_rust2::{
    encoder_options::Lzma2Options, ArchiveEntry, ArchiveReader, ArchiveWriter,
    EncoderConfiguration, EncoderMethod, Password,
};
use std::cmp::Ordering;
use std::collections::{btree_map::Entry, BTreeMap};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, Write};
use std::mem::{offset_of, size_of, size_of_val};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, PathBuf};
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
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_STREAM_INFO,
};
use windows_sys::Win32::System::SystemServices::FILE_NAMED_STREAMS;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipReadOptions, ZipWriter};

const ZONE_IDENTIFIER: &str = ":Zone.Identifier:$DATA";
const BUFFER_SIZE: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Zip,
    SevenZip,
    Tar,
}

impl Format {
    pub const ALL: [Self; 3] = [Self::Zip, Self::SevenZip, Self::Tar];

    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?;
        Self::ALL
            .into_iter()
            .find(|format| extension.eq_ignore_ascii_case(format.extension()))
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::SevenZip => "7z",
            Self::Tar => "tar",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Zip => "ZIP",
            Self::SevenZip => "7z",
            Self::Tar => "TAR",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Level {
    #[default]
    Fast,
    Balanced,
    Smaller,
}

impl Level {
    pub const ALL: [Self; 3] = [Self::Fast, Self::Balanced, Self::Smaller];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Fast => "Fast",
            Self::Balanced => "Balanced",
            Self::Smaller => "Smaller",
        }
    }

    fn zip_level(self) -> i64 {
        match self {
            Self::Fast => 1,
            Self::Balanced => 6,
            Self::Smaller => 9,
        }
    }

    fn lzma_level(self) -> u32 {
        match self {
            Self::Fast => 1,
            Self::Balanced => 3,
            Self::Smaller => 6,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Compression {
    pub format: Format,
    pub level: Level,
}

pub fn suggested_name(stem: &str, extension: &str) -> String {
    fitted_name(stem, 1, extension).unwrap_or_else(|| format!("Archive{extension}"))
}

pub fn compress(sources: &[PathBuf], output: &Path) -> Result<(), String> {
    let options = Compression {
        format: Format::from_path(output).unwrap_or_default(),
        level: Level::Balanced,
    };
    compress_with(sources, output, options, false, |_| {}).map(|_| ())
}

pub fn compress_with(
    sources: &[PathBuf],
    output: &Path,
    options: Compression,
    automatic: bool,
    before_publish: impl FnMut(&Path),
) -> Result<PathBuf, String> {
    if sources.is_empty() {
        return Err("Select at least one item to compress.".into());
    }
    let destination = Destination::new(output, automatic, true)?;
    let staged =
        tempfile::tempdir_in(&destination.parent).map_err(|err| at(&destination.parent, err))?;
    let (staging_guard, _) = open_source(staged.path())?;
    let file = staged.path().join("archive");
    let writer = BufWriter::with_capacity(BUFFER_SIZE, new_file(&file)?);
    let mut writer = Writer::new(writer, options).map_err(|err| at(output, err))?;
    let mut paths = ArchivePaths::default();

    for path in sources {
        let name = checked_name(path.file_name(), path)?;
        let (source, metadata) = open_source(path)?;
        if metadata.is_dir() {
            let canonical = fs::canonicalize(path).map_err(|err| at(path, err))?;
            let canonical = WindowsPath::new(canonical.as_os_str());
            if destination
                .parent
                .ancestors()
                .any(|p| WindowsPath::new(p.as_os_str()) == canonical)
            {
                return Err(at(
                    path,
                    "Choose an archive location outside the source folder.",
                ));
            }
        }
        add_source(&mut writer, &mut paths, path, name, source, metadata)?;
    }

    writer.finish().map_err(|err| at(output, err))?;
    let result = destination.publish(&file, before_publish);
    drop(staging_guard);
    result
}

pub fn extract(source: &Path, output: &Path) -> Result<(), String> {
    extract_with(source, output, false, |_| {}).map(|_| ())
}

pub fn extract_with(
    source: &Path,
    output: &Path,
    automatic: bool,
    before_publish: impl FnMut(&Path),
) -> Result<PathBuf, String> {
    let format =
        Format::from_path(source).ok_or_else(|| at(source, "Select a ZIP, 7z or TAR file."))?;
    let destination = Destination::new(output, automatic, false)?;
    let (input, metadata) = open_source(source)?;
    if !metadata.is_file() {
        return Err(at(source, "Select an archive file."));
    }
    let zone = open_zone_identifier(source, &input)?;
    let staged =
        tempfile::tempdir_in(&destination.parent).map_err(|err| at(&destination.parent, err))?;
    let (staging_guard, _) = open_source(staged.path())?;
    let root = staged.path().join("files");
    fs::create_dir(&root).map_err(|err| at(&root, err))?;
    let (root_guard, _) = open_source(&root)?;
    let mut extraction = Extraction {
        root: &root,
        paths: ArchivePaths::default(),
        zone,
    };
    let input = BufReader::with_capacity(BUFFER_SIZE, input);
    match format {
        Format::Zip => extract_zip(input, source, &mut extraction)?,
        Format::SevenZip => extract_7z(input, source, &mut extraction)?,
        Format::Tar => extract_tar(input, source, &mut extraction)?,
    }
    drop(root_guard);
    let result = destination.publish(&root, before_publish);
    drop(staging_guard);
    result
}

fn extract_zip(
    input: BufReader<File>,
    source: &Path,
    extraction: &mut Extraction<'_>,
) -> Result<(), String> {
    let mut zip = ZipArchive::new(input).map_err(|err| at(source, err))?;
    if zip.has_duplicate_names() {
        return Err(at(source, "The ZIP contains duplicate names."));
    }
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
        let size = entry.size();
        extraction.write(&name, directory, size, &mut entry)?;
    }
    Ok(())
}

fn extract_7z(
    input: BufReader<File>,
    source: &Path,
    extraction: &mut Extraction<'_>,
) -> Result<(), String> {
    const UNIX_ATTRIBUTES: u32 = 0x8000;
    let mut archive = ArchiveReader::new_with_memory_limit(input, Password::empty(), 256 * 1024)
        .map_err(|err| at(source, err))?;
    archive.set_thread_count(codec_threads(4));
    archive
        .for_each_entries(|entry, reader| {
            let attributes = entry.windows_attributes();
            let unix_type = (attributes >> 16) & 0o170000;
            if entry.is_anti_item()
                || attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
                || (attributes & UNIX_ATTRIBUTES != 0
                    && !matches!(unix_type, 0 | 0o040000 | 0o100000))
            {
                return Err(sevenz_rust2::Error::Other(
                    format!("Unsupported 7z entry type for {:?}.", entry.name()).into(),
                ));
            }
            let name = entry.name().replace('\\', "/");
            extraction
                .write(&name, entry.is_directory(), entry.size(), reader)
                .map_err(|err| sevenz_rust2::Error::Other(err.into()))?;
            Ok(true)
        })
        .map_err(|err| at(source, err))
}

fn extract_tar(
    input: BufReader<File>,
    source: &Path,
    extraction: &mut Extraction<'_>,
) -> Result<(), String> {
    let mut archive = tar::Archive::new(input);
    for entry in archive.entries().map_err(|err| at(source, err))? {
        let mut entry = entry.map_err(|err| at(source, err))?;
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions() {
            continue;
        }
        let name = std::str::from_utf8(&entry.path_bytes())
            .map_err(|err| at(source, err))?
            .replace('\\', "/");
        if !(kind.is_dir() || kind.is_file() || kind.is_contiguous() || kind.is_gnu_sparse()) {
            return Err(format!("Unsupported TAR entry type for {name:?}."));
        }
        let name = name.trim_start_matches("./");
        let size = entry.size();
        if kind.is_dir() && matches!(name, "" | ".") && size == 0 {
            continue;
        }
        extraction.write(name, kind.is_dir(), size, &mut entry)?;
    }
    Ok(())
}

struct Extraction<'a> {
    root: &'a Path,
    paths: ArchivePaths,
    zone: Option<File>,
}

impl Extraction<'_> {
    fn write(
        &mut self,
        name: &str,
        directory: bool,
        size: u64,
        reader: &mut dyn Read,
    ) -> Result<(), String> {
        if directory && (size != 0 || reader.read(&mut [0]).map_err(|err| at(self.root, err))? != 0)
        {
            return Err(format!("Archive directory {name:?} contains file data."));
        }
        let (target, _pins) = extract_path(self.root, &mut self.paths, name, directory)?;
        if directory {
            return Ok(());
        }
        let mut file = new_file(&target)?;
        if let Some(zone) = self.zone.as_mut() {
            let stream = zone_identifier_path(&target);
            write_zone_identifier(&file, &stream, zone).map_err(|err| at(&stream, err))?;
        }
        let mut writer = BufWriter::with_capacity(BUFFER_SIZE, &mut file);
        let written = io::copy(reader, &mut writer).map_err(|err| at(&target, err))?;
        writer.flush().map_err(|err| at(&target, err))?;
        if written != size {
            return Err(format!("Archive entry {name:?} has an incorrect size."));
        }
        Ok(())
    }
}

struct Destination {
    parent: PathBuf,
    _parent_guard: File,
    stem: String,
    extension: String,
    automatic: bool,
}

impl Destination {
    fn new(output: &Path, automatic: bool, is_archive: bool) -> Result<Self, String> {
        let name = checked_name(output.file_name(), output)?;
        let parent = output
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = fs::canonicalize(parent).map_err(|err| at(parent, err))?;
        // Windows opens the destination directory for writes during the final rename.
        // Allow child operations while keeping the directory itself pinned against deletion.
        let (parent_guard, metadata) = open_checked(&parent, FILE_SHARE_READ | FILE_SHARE_WRITE)?;
        if !metadata.is_dir() {
            return Err(at(&parent, "Choose an existing destination folder."));
        }
        let (stem, extension) = if is_archive {
            name.rsplit_once('.')
                .map_or((name, String::new()), |(stem, extension)| {
                    (stem, format!(".{extension}"))
                })
        } else {
            (name, String::new())
        };
        let destination = Self {
            parent,
            _parent_guard: parent_guard,
            stem: stem.to_owned(),
            extension,
            automatic,
        };
        if !automatic && destination.occupied(&destination.parent.join(name))? {
            return Err(at(output, "The destination already exists."));
        }
        Ok(destination)
    }

    fn occupied(&self, path: &Path) -> Result<bool, String> {
        match fs::symlink_metadata(path) {
            Ok(_) => Ok(true),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(at(path, err)),
        }
    }

    fn publish(
        &self,
        staged: &Path,
        mut before_publish: impl FnMut(&Path),
    ) -> Result<PathBuf, String> {
        let staged = wide(staged);
        let reported_parent = conventional_path(&self.parent);
        for number in 1.. {
            let name = fitted_name(&self.stem, number, &self.extension)
                .ok_or_else(|| at(&self.parent, "Invalid output name."))?;
            let output = self.parent.join(&name);
            if self.automatic && self.occupied(&output)? {
                continue;
            }
            let output_path = wide(&output);
            let reported = reported_parent.join(name);
            before_publish(&reported);
            // Without MOVEFILE_REPLACE_EXISTING, publication cannot replace a competing file.
            if unsafe { MoveFileExW(staged.as_ptr(), output_path.as_ptr(), 0) } != 0 {
                return Ok(reported);
            }
            let error = io::Error::last_os_error();
            if !self.automatic || !self.occupied(&output)? {
                return Err(at(&output, error));
            }
        }
        unreachable!()
    }
}

fn conventional_path(path: &Path) -> PathBuf {
    // The Shell's Recycle Bin API needs ordinary paths for Undo. Keep verbatim
    // paths when a component cannot be addressed by its conventional spelling.
    if path.components().any(
        |part| matches!(part, Component::Normal(name) if !name.to_str().is_some_and(safe_name)),
    ) {
        return path.to_owned();
    }
    if let Some(path) = path.to_str() {
        if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{unc}"));
        }
        if let Some(disk) = path.strip_prefix(r"\\?\") {
            if disk.as_bytes().get(1) == Some(&b':') {
                return PathBuf::from(disk);
            }
        }
    }
    path.to_owned()
}

fn fitted_name(stem: &str, number: u64, extension: &str) -> Option<String> {
    let suffix = if number == 1 {
        String::new()
    } else {
        format!(" ({number})")
    };
    let limit = 255usize.checked_sub(suffix.len() + extension.encode_utf16().count())?;
    let mut length = 0;
    let truncated: String = stem
        .chars()
        .take_while(|c| {
            length += c.len_utf16();
            length <= limit
        })
        .collect();
    let stem = if number == 1 && truncated == stem {
        stem
    } else {
        truncated.trim_end_matches([' ', '.'])
    };
    let name = format!("{stem}{suffix}{extension}");
    safe_name(&name).then_some(name)
}

fn new_file(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .map_err(|err| at(path, err))
}

fn open_source(path: &Path) -> Result<(File, fs::Metadata), String> {
    open_checked(path, FILE_SHARE_READ)
}

fn open_checked(path: &Path, share: u32) -> Result<(File, fs::Metadata), String> {
    // Open the entry itself and keep it in place while reading it.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .share_mode(share)
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

enum Writer {
    Zip(Box<ZipWriter<BufWriter<File>>>, Level),
    SevenZip(Box<ArchiveWriter<BufWriter<File>>>, Level),
    Tar(tar::Builder<BufWriter<File>>),
}

impl Writer {
    fn new(output: BufWriter<File>, options: Compression) -> Result<Self, String> {
        Ok(match options.format {
            Format::Zip => Self::Zip(Box::new(ZipWriter::new(output)), options.level),
            Format::SevenZip => Self::SevenZip(
                Box::new(ArchiveWriter::new(output).map_err(|err| err.to_string())?),
                options.level,
            ),
            Format::Tar => Self::Tar(tar::Builder::new(output)),
        })
    }

    fn add(
        &mut self,
        name: &str,
        source: &mut File,
        metadata: &fs::Metadata,
    ) -> Result<(), String> {
        let directory = metadata.is_dir();
        let size = metadata.len();
        match self {
            Self::Zip(writer, level) => {
                let options = SimpleFileOptions::default()
                    .compression_method(CompressionMethod::Deflated)
                    .compression_level(Some(level.zip_level()))
                    // Leave room for the Deflate overhead as input approaches the ZIP32 limit.
                    .large_file(!directory && size >= u64::from(u32::MAX) - (16 << 20));
                if directory {
                    writer
                        .add_directory(name, options)
                        .map_err(|err| err.to_string())?;
                } else {
                    writer
                        .start_file(name, options)
                        .map_err(|err| err.to_string())?;
                    io::copy(source, writer.as_mut()).map_err(|err| err.to_string())?;
                }
            }
            Self::SevenZip(writer, level) => {
                let entry = if directory {
                    ArchiveEntry::new_directory(name)
                } else {
                    ArchiveEntry::new_file(name)
                };
                let input = if directory || size == 0 {
                    None
                } else {
                    writer.set_content_methods(vec![seven_zip_encoder(*level, size)]);
                    Some(BufReader::with_capacity(BUFFER_SIZE, source))
                };
                writer
                    .push_archive_entry(entry, input)
                    .map_err(|err| err.to_string())?;
            }
            Self::Tar(writer) => {
                let mut header = tar::Header::new_ustar();
                let name = if name.is_ascii() && header.set_path(name).is_ok() {
                    name
                } else {
                    // PAX paths declare UTF-8, which preserves names in Windows tar.
                    writer
                        .append_pax_extensions([("path", name.as_bytes())])
                        .map_err(|err| err.to_string())?;
                    "PaxItem"
                };
                if directory {
                    header.set_mode(0o755);
                    header.set_size(0);
                    header.set_entry_type(tar::EntryType::Directory);
                    writer
                        .append_data(&mut header, name, io::empty())
                        .map_err(|err| err.to_string())?;
                } else {
                    header.set_mode(0o644);
                    header.set_size(size);
                    header.set_entry_type(tar::EntryType::Regular);
                    writer
                        .append_data(&mut header, name, source)
                        .map_err(|err| err.to_string())?;
                }
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<(), String> {
        let mut output = match self {
            Self::Zip(writer, _) => writer.finish().map_err(|err| err.to_string())?,
            Self::SevenZip(writer, _) => writer.finish().map_err(|err| err.to_string())?,
            Self::Tar(writer) => writer.into_inner().map_err(|err| err.to_string())?,
        };
        output.flush().map_err(|err| err.to_string())
    }
}

fn codec_threads(maximum: u32) -> u32 {
    std::thread::available_parallelism()
        .map_or(1, |cores| cores.get() as u32)
        .min(maximum)
}

fn seven_zip_encoder(level: Level, size: u64) -> EncoderConfiguration {
    if level == Level::Fast && size < 1024 {
        return EncoderMethod::COPY.into();
    }
    let threads = codec_threads(if level == Level::Smaller { 2 } else { 4 });
    let mut options = if threads > 1 && size >= 64 * 1024 * 1024 {
        Lzma2Options::from_level_mt(level.lzma_level(), threads, 8 * 1024 * 1024)
    } else {
        Lzma2Options::from_level(level.lzma_level())
    };
    let dictionary = match level {
        Level::Fast => 1 << 20,
        Level::Balanced => 4 << 20,
        Level::Smaller => 8 << 20,
    };
    options.set_dictionary_size(size.min(dictionary).max(4096).next_power_of_two() as u32);
    options.into()
}

fn add_source(
    writer: &mut Writer,
    paths: &mut ArchivePaths,
    path: &Path,
    name: &str,
    mut source: File,
    metadata: fs::Metadata,
) -> Result<(), String> {
    if metadata.is_dir() {
        paths.insert(name, EntryType::Directory)?;
        writer
            .add(name, &mut source, &metadata)
            .map_err(|err| at(path, err))?;
        for entry in fs::read_dir(path).map_err(|err| at(path, err))? {
            let entry = entry.map_err(|err| at(path, err))?;
            let child = entry.path();
            let component = checked_name(child.file_name(), &child)?;
            let (file, metadata) = open_source(&child)?;
            add_source(
                writer,
                paths,
                &child,
                &format!("{name}/{component}"),
                file,
                metadata,
            )?;
        }
    } else if metadata.is_file() {
        paths.insert(name, EntryType::File)?;
        writer
            .add(name, &mut source, &metadata)
            .map_err(|err| at(path, err))?;
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
) -> Result<(PathBuf, Vec<File>), String> {
    let name = name.strip_suffix('/').unwrap_or(name);
    let mut components = name.split('/').peekable();
    let mut relative = String::new();
    let mut target = root.to_path_buf();
    let mut pins = Vec::new();
    while let Some(component) = components.next() {
        if !safe_name(component) {
            return Err(format!("Unsafe archive entry path {name:?}."));
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
        let create = paths.insert(&relative, kind)?;
        if kind != EntryType::File {
            if create {
                fs::create_dir(&target).map_err(|err| at(&target, err))?;
            }
            let (directory, metadata) = open_source(&target)?;
            if !metadata.is_dir() {
                return Err(at(&target, "An archive folder was replaced."));
            }
            pins.push(directory);
        }
    }
    Ok((target, pins))
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
                Err(format!(
                    "Conflicting archive paths {existing:?} and {path:?}."
                ))
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
