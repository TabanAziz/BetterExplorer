use crate::model::wide;
use std::{
    borrow::Cow,
    ffi::OsString,
    fs,
    io::{Cursor, Read, Write},
    os::windows::ffi::OsStringExt,
    path::{Component, Path, PathBuf, Prefix},
    ptr::null_mut,
    sync::OnceLock,
};
use windows_sys::Win32::{
    Foundation::ERROR_SUCCESS,
    Storage::FileSystem::GetDriveTypeW,
    System::Registry::{
        RegGetValueW, HKEY_CLASSES_ROOT, REG_BINARY, RRF_RT_ANY, RRF_RT_REG_BINARY, RRF_RT_REG_SZ,
    },
};
use zip::ZipArchive;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum DocumentKind {
    Access,
    AccessLegacy,
    Bitmap,
    Word,
    PowerPoint,
    Python,
    RichText,
    Text,
    Excel,
    Zip,
}

const DOCUMENT_KINDS: [DocumentKind; 10] = [
    DocumentKind::Access,
    DocumentKind::AccessLegacy,
    DocumentKind::Bitmap,
    DocumentKind::Word,
    DocumentKind::PowerPoint,
    DocumentKind::Python,
    DocumentKind::RichText,
    DocumentKind::Text,
    DocumentKind::Excel,
    DocumentKind::Zip,
];

const BITMAP: &[u8] = &[
    // Bitmap file header followed by a BITMAPINFOHEADER.
    b'B', b'M', 58, 0, 0, 0, 0, 0, 0, 0, 54, 0, 0, 0, 40, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 24,
    0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    // One white pixel and scanline padding.
    255, 255, 255, 0,
];

const EMPTY_ZIP: &[u8] = b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
const DRIVE_REMOTE: u32 = 4;
const MAX_TEMPLATE_BYTES: u64 = 32 * 1024 * 1024;

struct DocumentFormat {
    title: &'static str,
    extension: &'static str,
    fallback: Option<&'static [u8]>,
}

impl DocumentKind {
    fn format(self) -> DocumentFormat {
        let (title, extension, fallback): (&str, &str, Option<&[u8]>) = match self {
            Self::Access => ("Access Database", ".accdb", None),
            Self::AccessLegacy => ("Access 2000-2003 Database", ".mdb", None),
            Self::Bitmap => ("Bitmap Image", ".bmp", Some(BITMAP)),
            Self::Word => (
                "Word Document",
                ".docx",
                Some(include_bytes!("../resources/new/blank.docx")),
            ),
            Self::PowerPoint => (
                "PowerPoint Presentation",
                ".pptx",
                Some(include_bytes!("../resources/new/blank.pptx")),
            ),
            Self::Python => ("Python File", ".py", Some(b"")),
            Self::RichText => ("Rich Text Format", ".rtf", Some(b"{\\rtf1\\ansi\r\n}")),
            Self::Text => ("Text Document", ".txt", Some(b"")),
            Self::Excel => (
                "Excel Worksheet",
                ".xlsx",
                Some(include_bytes!("../resources/new/blank.xlsx")),
            ),
            Self::Zip => ("Compressed (zipped) Folder", ".zip", Some(EMPTY_ZIP)),
        };
        DocumentFormat {
            title,
            extension,
            fallback,
        }
    }

    pub fn title(self) -> &'static str {
        self.format().title
    }

    pub fn extension(self) -> &'static str {
        self.format().extension
    }

    fn accepts(self, bytes: &[u8]) -> bool {
        match self {
            Self::Access => bytes.len() >= 4096 && bytes[4..].starts_with(b"Standard ACE DB\0"),
            Self::AccessLegacy => {
                bytes.len() >= 2048 && bytes[4..].starts_with(b"Standard Jet DB\0")
            }
            Self::Bitmap => bytes.len() >= 26 && bytes.starts_with(b"BM"),
            Self::RichText => bytes.starts_with(b"{\\rtf"),
            Self::Text | Self::Python => true,
            Self::Zip => ZipArchive::new(Cursor::new(bytes)).is_ok(),
            Self::Word => has_package_parts(bytes, "word/document.xml"),
            Self::PowerPoint => has_package_parts(bytes, "ppt/presentation.xml"),
            Self::Excel => has_package_parts(bytes, "xl/workbook.xml"),
        }
    }
}

type Templates = Vec<(DocumentKind, Cow<'static, [u8]>)>;
static TEMPLATES: OnceLock<Templates> = OnceLock::new();

fn templates() -> &'static Templates {
    TEMPLATES.get_or_init(|| {
        DOCUMENT_KINDS
            .into_iter()
            .filter_map(|kind| {
                let bytes = registered_template(kind)
                    .map(Cow::Owned)
                    .or_else(|| kind.format().fallback.map(Cow::Borrowed))?;
                Some((kind, bytes))
            })
            .collect()
    })
}

pub fn load_catalog() {
    let _ = templates();
}

pub fn available_documents() -> impl Iterator<Item = DocumentKind> {
    let cached = TEMPLATES.get();
    DOCUMENT_KINDS.into_iter().filter(move |kind| {
        kind.format().fallback.is_some()
            || cached.is_some_and(|templates| templates.iter().any(|(entry, _)| entry == kind))
    })
}

pub(crate) fn create(output: &Path, kind: DocumentKind) -> Result<(), String> {
    let extension = kind.extension();
    if !output
        .extension()
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(&extension[1..]))
    {
        return Err(format!(
            "Enter a valid Windows file name ending in {extension}."
        ));
    }
    let parent = output.parent().ok_or("Choose a destination folder.")?;
    let (_, bytes) = templates()
        .iter()
        .find(|(candidate, _)| *candidate == kind)
        .ok_or("Windows does not have a template for this document type.")?;
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|err| err.to_string())?;
    staged.write_all(bytes).map_err(|err| err.to_string())?;
    staged
        .persist_noclobber(output)
        .map_err(|err| err.error.to_string())?;
    Ok(())
}

fn has_package_parts(bytes: &[u8], main_part: &str) -> bool {
    let Ok(mut package) = ZipArchive::new(Cursor::new(bytes)) else {
        return false;
    };
    ["[Content_Types].xml", "_rels/.rels", main_part]
        .into_iter()
        .all(|part| package.by_name(part).is_ok_and(|entry| entry.size() != 0))
}

fn registered_template(kind: DocumentKind) -> Option<Vec<u8>> {
    let extension = kind.extension();
    let prog_id = registry_string(extension, "").and_then(|id| id.into_string().ok());
    let candidates = [
        Some(format!(r"{extension}\ShellNew")),
        prog_id
            .as_ref()
            .map(|id| format!(r"{extension}\{id}\ShellNew")),
        prog_id.as_ref().map(|id| format!(r"{id}\ShellNew")),
    ];
    candidates
        .iter()
        .flatten()
        .find_map(|key| {
            registry_string(key, "FileName")
                .and_then(|name| read_template_file(Path::new(&name), kind))
                .or_else(|| {
                    let (value_type, data) =
                        registry_value(key, "Data", RRF_RT_REG_BINARY | RRF_RT_REG_SZ)?;
                    let data = if value_type == REG_BINARY {
                        data
                    } else {
                        utf16_string(&data)?.into_string().ok()?.into_bytes()
                    };
                    kind.accepts(&data).then_some(data)
                })
                .or_else(|| {
                    registry_value(key, "NullFile", RRF_RT_ANY)
                        .filter(|_| kind.accepts(b""))
                        .map(|_| Vec::new())
                })
        })
        .or_else(|| {
            // Access can register the older format through a command. Supply a blank Jet 4
            // database for that registration without invoking its command.
            if kind == DocumentKind::AccessLegacy
                && candidates
                    .iter()
                    .flatten()
                    .any(|key| registry_value(key, "Command", RRF_RT_REG_SZ).is_some())
            {
                let archive = include_bytes!("../resources/new/blank.mdb.zip");
                let mut archive = ZipArchive::new(Cursor::new(archive)).ok()?;
                let mut bytes = Vec::new();
                archive
                    .by_name("blank.mdb")
                    .ok()?
                    .read_to_end(&mut bytes)
                    .ok()?;
                return kind.accepts(&bytes).then_some(bytes);
            }
            None
        })
}

fn read_template_file(name: &Path, kind: DocumentKind) -> Option<Vec<u8>> {
    let read = |path: PathBuf| {
        if !is_local_template_path(&path) {
            return None;
        }
        let file = fs::File::open(path).ok()?;
        let size = file.metadata().ok()?.len();
        if size > MAX_TEMPLATE_BYTES {
            return None;
        }
        let mut bytes = Vec::with_capacity(size as usize);
        file.take(MAX_TEMPLATE_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > MAX_TEMPLATE_BYTES {
            return None;
        }
        kind.accepts(&bytes).then_some(bytes)
    };
    if name.is_absolute() {
        return read(name.to_owned());
    }
    if name.as_os_str().is_empty()
        || !name
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return None;
    }
    [
        ("APPDATA", r"Microsoft\Windows\Templates"),
        ("ProgramData", r"Microsoft\Windows\Templates"),
        ("WINDIR", "ShellNew"),
    ]
    .into_iter()
    .find_map(|(variable, folder)| {
        let base = PathBuf::from(std::env::var_os(variable)?);
        read(base.join(folder).join(name))
    })
}

fn is_local_template_path(path: &Path) -> bool {
    let mut parts = path.components();
    let (Some(Component::Prefix(prefix)), Some(Component::RootDir)) = (parts.next(), parts.next())
    else {
        return false;
    };
    let (Prefix::Disk(drive) | Prefix::VerbatimDisk(drive)) = prefix.kind() else {
        return false;
    };
    let root = [drive.into(), b':'.into(), b'\\'.into(), 0];
    unsafe { GetDriveTypeW(root.as_ptr()) != DRIVE_REMOTE }
}

fn registry_string(key: &str, name: &str) -> Option<OsString> {
    // RegGetValueW expands REG_EXPAND_SZ and returns it as REG_SZ.
    let (_, bytes) = registry_value(key, name, RRF_RT_REG_SZ)?;
    utf16_string(&bytes)
}

fn utf16_string(bytes: &[u8]) -> Option<OsString> {
    let (pairs, _) = bytes.as_chunks::<2>();
    let units = pairs
        .iter()
        .copied()
        .map(u16::from_le_bytes)
        .take_while(|unit| *unit != 0)
        .collect::<Vec<_>>();
    (!units.is_empty()).then(|| OsString::from_wide(&units))
}

fn registry_value(key: &str, name: &str, flags: u32) -> Option<(u32, Vec<u8>)> {
    let key = wide(key);
    let name = wide(name);
    let mut size = 0;
    let mut kind = 0;
    let result = unsafe {
        RegGetValueW(
            HKEY_CLASSES_ROOT,
            key.as_ptr(),
            name.as_ptr(),
            flags,
            &mut kind,
            null_mut(),
            &mut size,
        )
    };
    if result != ERROR_SUCCESS || u64::from(size) > MAX_TEMPLATE_BYTES {
        return None;
    }
    let mut data = vec![0; size as usize];
    if size > 0 {
        let result = unsafe {
            RegGetValueW(
                HKEY_CLASSES_ROOT,
                key.as_ptr(),
                name.as_ptr(),
                flags,
                &mut kind,
                data.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if result != ERROR_SUCCESS {
            return None;
        }
        data.truncate(size as usize);
    }
    Some((kind, data))
}
