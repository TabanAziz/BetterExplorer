use crate::{
    model::{from_wide, wide, Location, ModifiedDateFormat},
    namespace,
};
use std::{
    ffi::{OsStr, OsString},
    fs, io,
    os::windows::{ffi::OsStringExt, fs::MetadataExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};
use windows::{
    core::GUID,
    Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{
            FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music,
            FOLDERID_Pictures, FOLDERID_Profile, FOLDERID_Videos, SHGetKnownFolderPath,
            KF_FLAG_DONT_VERIFY,
        },
    },
};
use windows_sys::Win32::{
    Foundation::{ERROR_NO_MORE_FILES, FILETIME, HANDLE, INVALID_HANDLE_VALUE, SYSTEMTIME},
    Globalization::{CompareStringOrdinal, CSTR_EQUAL},
    Storage::FileSystem::{
        FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW,
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDriveStringsW, GetVolumeInformationW,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_ATTRIBUTE_SYSTEM, FIND_FIRST_EX_LARGE_FETCH, WIN32_FIND_DATAW,
    },
    System::{
        SystemInformation::{GetLocalTime, GetSystemTimeAsFileTime},
        Time::{
            FileTimeToSystemTime, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTimeEx,
            TzSpecificLocalTimeToSystemTimeEx,
        },
    },
    UI::Shell::StrCmpLogicalW,
};

pub use crate::model::{Column, Sort};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateGroup {
    Future,
    Today,
    Yesterday,
    EarlierThisWeek,
    LastWeek,
    EarlierThisMonth,
    LastMonth,
    EarlierThisYear,
    LongAgo,
    Unknown,
}

impl DateGroup {
    pub const ALL: [Self; 10] = [
        Self::Future,
        Self::Today,
        Self::Yesterday,
        Self::EarlierThisWeek,
        Self::LastWeek,
        Self::EarlierThisMonth,
        Self::LastMonth,
        Self::EarlierThisYear,
        Self::LongAgo,
        Self::Unknown,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Future => "Future",
            Self::Today => "Today",
            Self::Yesterday => "Yesterday",
            Self::EarlierThisWeek => "Earlier this week",
            Self::LastWeek => "Last week",
            Self::EarlierThisMonth => "Earlier this month",
            Self::LastMonth => "Last month",
            Self::EarlierThisYear => "Earlier this year",
            Self::LongAgo => "A long time ago",
            Self::Unknown => "Unspecified",
        }
    }
}

const DAY_TICKS: u64 = 864_000_000_000;

fn system_time_ticks(time: &SYSTEMTIME) -> Option<u64> {
    let mut file_time: FILETIME = unsafe { std::mem::zeroed() };
    (unsafe { SystemTimeToFileTime(time, &mut file_time) } != 0)
        .then_some(u64::from(file_time.dwHighDateTime) << 32 | u64::from(file_time.dwLowDateTime))
}

fn ticks_system_time(ticks: u64) -> Option<SYSTEMTIME> {
    let mut time = unsafe { std::mem::zeroed() };
    let file_time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    (unsafe { FileTimeToSystemTime(&file_time, &mut time) } != 0).then_some(time)
}

pub struct DateGroups([u64; 8]);

impl DateGroups {
    pub fn current() -> Option<Self> {
        let mut now = unsafe { std::mem::zeroed() };
        unsafe { GetLocalTime(&mut now) };
        Self::for_local_date(now.wYear, now.wMonth, now.wDay)
    }

    fn for_local_date(year: u16, month: u16, day: u16) -> Option<Self> {
        let local_date = |year, month, day| {
            system_time_ticks(&SYSTEMTIME {
                wYear: year,
                wMonth: month,
                wDay: day,
                ..unsafe { std::mem::zeroed() }
            })
        };
        let today = local_date(year, month, day)?;
        // Windows FILETIME begins on a Monday. Work in local calendar days until
        // converting each boundary to UTC, so daylight saving never shifts a day.
        let monday = today - (today / DAY_TICKS % 7) * DAY_TICKS;
        let (prior_year, prior_month) = if month == 1 {
            (year - 1, 12)
        } else {
            (year, month - 1)
        };
        let boundaries = [
            today + DAY_TICKS,
            today,
            today.saturating_sub(DAY_TICKS),
            monday,
            monday.saturating_sub(7 * DAY_TICKS),
            local_date(year, month, 1)?,
            local_date(prior_year, prior_month, 1)?,
            local_date(year, 1, 1)?,
        ];
        let mut starts = [0; 8];
        for (start, boundary) in starts.iter_mut().zip(boundaries) {
            let local = ticks_system_time(boundary)?;
            let mut utc = unsafe { std::mem::zeroed() };
            if unsafe { TzSpecificLocalTimeToSystemTimeEx(std::ptr::null(), &local, &mut utc) } == 0
            {
                return None;
            }
            *start = system_time_ticks(&utc)?;
        }
        Some(Self(starts))
    }

    pub fn group(&self, modified: u64) -> DateGroup {
        if modified == 0 {
            return DateGroup::Unknown;
        }
        self.0
            .iter()
            .position(|&start| modified >= start)
            .map_or(DateGroup::LongAgo, |index| DateGroup::ALL[index])
    }

    pub fn next_day_ms(&self) -> u32 {
        let mut now: FILETIME = unsafe { std::mem::zeroed() };
        unsafe { GetSystemTimeAsFileTime(&mut now) };
        let now = u64::from(now.dwHighDateTime) << 32 | u64::from(now.dwLowDateTime);
        self.0[0]
            .saturating_sub(now)
            .div_ceil(10_000)
            .clamp(1_000, u64::from(u32::MAX)) as u32
    }
}

#[derive(Debug)]
pub struct Capacity {
    pub total: u64,
    pub free: u64,
}

impl Capacity {
    pub fn label(&self) -> String {
        let gib = 1_073_741_824.0;
        format!(
            "{:.1} GB free of {:.1} GB",
            self.free as f64 / gib,
            self.total as f64 / gib
        )
    }
}

#[derive(Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: Vec<u16>,
    pub extension: Vec<u16>,
    pub directory: bool,
    pub size: u64,
    pub modified: u64,
    pub location: bool,
    pub capacity: Option<Capacity>,
    pub shell_item: Option<namespace::Item>,
}

impl Entry {
    fn place(path: PathBuf, label: impl AsRef<OsStr>) -> Self {
        Self {
            path,
            name: wide(label),
            extension: Vec::new(),
            directory: true,
            size: 0,
            modified: 0,
            location: true,
            capacity: None,
            shell_item: None,
        }
    }
    pub fn label(&self) -> String {
        from_wide(&self.name).to_string_lossy().into()
    }
    pub fn display_name(&self, hide_extensions: bool) -> &[u16] {
        let name = &self.name[..self.name.len().saturating_sub(1)];
        if hide_extensions && !self.directory && !self.extension.is_empty() {
            return name
                .strip_suffix(&self.extension[..self.extension.len() - 1])
                .and_then(|stem| stem.strip_suffix(&[u16::from(b'.')]))
                .unwrap_or(name);
        }
        name
    }
    pub fn kind(&self) -> String {
        if let Some(item) = &self.shell_item {
            item.kind.clone()
        } else if self.location {
            "Location".into()
        } else if self.directory {
            "File folder".into()
        } else if self.extension.is_empty() {
            "File".into()
        } else {
            format!(
                "{} file",
                from_wide(&self.extension).to_string_lossy().to_uppercase()
            )
        }
    }
    pub fn date(&self, format: ModifiedDateFormat) -> String {
        if self.modified == 0 {
            return String::new();
        }
        let Some(utc) = ticks_system_time(self.modified) else {
            return String::new();
        };
        let mut date: SYSTEMTIME = unsafe { std::mem::zeroed() };
        if unsafe { SystemTimeToTzSpecificLocalTimeEx(std::ptr::null(), &utc, &mut date) } != 0 {
            let SYSTEMTIME {
                wYear: year,
                wMonth: month,
                wDay: day,
                wHour: hour,
                wMinute: minute,
                ..
            } = date;
            match format {
                ModifiedDateFormat::DayMonthYear => {
                    format!("{day:02}.{month:02}.{year:04} {hour:02}:{minute:02}")
                }
                ModifiedDateFormat::MonthDayYear => {
                    format!("{month:02}/{day:02}/{year:04} {hour:02}:{minute:02}")
                }
            }
        } else {
            String::new()
        }
    }
    pub fn display_size(&self) -> String {
        if self.directory {
            String::new()
        } else if self.size < 1024 {
            format!("{} B", self.size)
        } else if self.size < 1024 * 1024 {
            format!("{:.1} KB", self.size as f64 / 1024.0)
        } else if self.size < 1024 * 1024 * 1024 {
            format!("{:.1} MB", self.size as f64 / 1048576.0)
        } else {
            format!("{:.1} GB", self.size as f64 / 1073741824.0)
        }
    }
}

pub struct Request {
    pub id: u64,
    pub location: Location,
    pub hidden: bool,
    pub sort: Sort,
    pub pins: Vec<PathBuf>,
}

pub enum ScanEvent {
    Batch(u64, Vec<Arc<Entry>>),
    FirstRowTiming(u64, [Instant; 4]),
    Done(u64, Vec<Arc<Entry>>, Option<String>),
}

pub fn sort_entries(entries: &mut [Arc<Entry>], sort: Sort) {
    let folders_first = sort.column != Column::Modified;
    sort_entries_with_folders(entries, sort, folders_first);
}

fn sort_entries_with_folders(entries: &mut [Arc<Entry>], sort: Sort, folders_first: bool) {
    entries.sort_unstable_by(|left, right| {
        if folders_first && left.directory != right.directory {
            return right.directory.cmp(&left.directory);
        }
        let name_order =
            || unsafe { StrCmpLogicalW(left.name.as_ptr(), right.name.as_ptr()) }.cmp(&0);
        let field = match sort.column {
            Column::Name => name_order(),
            Column::Modified => left.modified.cmp(&right.modified),
            Column::Folder => left.path.parent().cmp(&right.path.parent()),
            Column::Kind => match (&left.shell_item, &right.shell_item) {
                (Some(left), Some(right))
                    if left.original_location.is_some() || right.original_location.is_some() =>
                {
                    left.original_location.cmp(&right.original_location)
                }
                (Some(left), Some(right)) => left.kind.cmp(&right.kind),
                _ => left.extension.cmp(&right.extension),
            },
            Column::Size => left.size.cmp(&right.size),
        }
        .then_with(name_order);
        if sort.descending {
            field.reverse()
        } else {
            field
        }
    });
}

pub fn known_places() -> Vec<(PathBuf, &'static str)> {
    let folders: [(&GUID, &str); 7] = [
        (&FOLDERID_Desktop, "Desktop"),
        (&FOLDERID_Downloads, "Downloads"),
        (&FOLDERID_Documents, "Documents"),
        (&FOLDERID_Pictures, "Pictures"),
        (&FOLDERID_Music, "Music"),
        (&FOLDERID_Videos, "Videos"),
        (&FOLDERID_Profile, "User folder"),
    ];
    folders
        .iter()
        .filter_map(|(id, label)| {
            // The returned string is allocated by the Shell.
            let raw = unsafe { SHGetKnownFolderPath(*id, KF_FLAG_DONT_VERIFY, None).ok()? };
            let path = unsafe { PathBuf::from(from_wide(raw.as_wide())) };
            unsafe { CoTaskMemFree(Some(raw.0.cast())) };
            Some((path, *label))
        })
        .collect()
}

fn drives() -> Vec<PathBuf> {
    let mut buffer = vec![0; 256];
    let mut count =
        unsafe { GetLogicalDriveStringsW(buffer.len() as u32, buffer.as_mut_ptr()) } as usize;
    if count > buffer.len() {
        buffer.resize(count, 0);
        count =
            unsafe { GetLogicalDriveStringsW(buffer.len() as u32, buffer.as_mut_ptr()) } as usize;
    }
    buffer
        .get(..count)
        .unwrap_or(&[])
        .split(|&unit| unit == 0)
        .filter(|part| !part.is_empty())
        .map(|part| PathBuf::from(from_wide(part)))
        .collect()
}

fn drive_entry(path: PathBuf) -> Entry {
    let root = wide(&path);
    let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
    let fallback = match kind {
        2 => "Removable drive",
        4 => "Network drive",
        5 => "Optical drive",
        6 => "RAM drive",
        _ => "Local drive",
    };
    let mut label = [0u16; 261];
    let mut total = 0;
    let mut free = 0;
    let mut named = fallback.to_owned();
    let mut capacity = None;
    // Query fixed disks only. An offline remote or removable volume can stall its driver.
    if matches!(kind, 3 | 6) {
        if unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        } != 0
            && label[0] != 0
        {
            named = from_wide(&label).to_string_lossy().into_owned();
        }
        if unsafe {
            GetDiskFreeSpaceExW(root.as_ptr(), std::ptr::null_mut(), &mut total, &mut free)
        } != 0
            && total != 0
        {
            capacity = Some(Capacity {
                total,
                free: free.min(total),
            });
        }
    }
    let label = format!(
        "{named} ({})",
        path.to_string_lossy().trim_end_matches('\\')
    );
    let mut entry = Entry::place(path, label);
    entry.capacity = capacity;
    entry
}

pub fn scan(request: Request, latest: &AtomicU64, publish: impl FnMut(ScanEvent)) {
    scan_revealing(request, None, latest, publish);
}

fn scan_revealing(
    request: Request,
    reveal: Option<PathBuf>,
    latest: &AtomicU64,
    mut publish: impl FnMut(ScanEvent),
) {
    let Request {
        id,
        location,
        hidden,
        mut sort,
        pins,
    } = request;
    let stale = || latest.load(Ordering::Relaxed) != id;
    if stale() {
        return;
    }
    let mut entries = Vec::new();
    let mut error = None;
    let file_system = matches!(location, Location::Folder(_)) || location.is_search();
    if matches!(location, Location::PcSearch { .. }) && sort.column == Column::Kind {
        sort.column = Column::Folder;
    }
    match location {
        Location::Settings => (),
        Location::Namespace { kind, child, .. } => {
            let result = if let Some(child) = child {
                namespace::enumerate_child(kind, &child, stale)
            } else {
                namespace::enumerate(kind, stale)
            };
            match result {
                Ok(items) => entries.extend(items.into_iter().map(|item| {
                    let extension = if kind == namespace::Namespace::RecycleBin && !item.directory {
                        item.extension
                            .as_deref()
                            .or_else(|| Path::new(&item.name).extension())
                            .map(wide)
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    Arc::new(Entry {
                        path: PathBuf::from(&item.key),
                        name: wide(&item.name),
                        extension,
                        directory: item.directory,
                        size: item.size,
                        modified: if kind == namespace::Namespace::RecycleBin {
                            item.deleted
                        } else {
                            item.modified
                        },
                        location: true,
                        capacity: None,
                        shell_item: Some(item),
                    })
                })),
                Err(message) => error = Some(message),
            }
        }
        Location::Home => {
            let places = known_places();
            entries.extend(
                places
                    .iter()
                    .map(|(path, label)| Arc::new(Entry::place(path.clone(), label))),
            );
            for path in pins {
                if places.iter().any(|(known, _)| known == &path) {
                    continue;
                }
                let label = path.file_name().unwrap_or(path.as_os_str()).to_owned();
                entries.push(Arc::new(Entry::place(path, label)));
            }
        }
        Location::Computer => {
            for path in drives() {
                if stale() {
                    return;
                }
                entries.push(Arc::new(drive_entry(path)));
            }
        }
        Location::PcSearch {
            query, all_drives, ..
        } => {
            if all_drives {
                let query = query.to_lowercase();
                for drive in drives() {
                    if stale() {
                        return;
                    }
                    // Include fixed and removable local volumes.
                    if matches!(unsafe { GetDriveTypeW(wide(&drive).as_ptr()) }, 2 | 3) {
                        enumerate(
                            drive,
                            Some(query.clone()),
                            Visibility {
                                hidden,
                                reveal: None,
                            },
                            &stale,
                            &mut entries,
                            &mut error,
                            |rows, _| publish(ScanEvent::Batch(id, rows)),
                        );
                    }
                }
            } else {
                let result = crate::index_search::search(&query, stale, |batch| {
                    let rows: Vec<_> = batch
                        .into_iter()
                        .filter(|file| {
                            hidden
                                || file.attributes & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM)
                                    == 0
                        })
                        .map(|file| {
                            Arc::new(Entry {
                                extension: if file.directory {
                                    Vec::new()
                                } else {
                                    file.path.extension().map(wide).unwrap_or_default()
                                },
                                path: file.path,
                                name: wide(file.name),
                                directory: file.directory,
                                size: file.size,
                                modified: file.modified,
                                location: false,
                                capacity: None,
                                shell_item: None,
                            })
                        })
                        .collect();
                    if !rows.is_empty() {
                        entries.extend(rows.iter().cloned());
                        publish(ScanEvent::Batch(id, rows));
                    }
                });
                error = result.err();
            }
        }
        location => {
            let (root, query) = match location {
                Location::Folder(root) => (root, None),
                Location::Search { root, query } => (root, Some(query.to_lowercase())),
                _ => unreachable!(),
            };
            let reveal = reveal
                .as_deref()
                .filter(|path| query.is_none() && path.parent() == Some(root.as_path()))
                .and_then(Path::file_name)
                .map(wide);
            enumerate(
                root,
                query,
                Visibility { hidden, reveal },
                &stale,
                &mut entries,
                &mut error,
                |rows, timing| {
                    if let Some(timing) = timing {
                        publish(ScanEvent::FirstRowTiming(id, timing));
                    }
                    publish(ScanEvent::Batch(id, rows));
                },
            );
        }
    }
    if stale() {
        return;
    }
    if file_system {
        sort_entries(&mut entries, sort);
    } else {
        sort_entries_with_folders(&mut entries, sort, true);
    }
    if !stale() {
        publish(ScanEvent::Done(id, entries, error));
    }
}

struct Visibility {
    hidden: bool,
    reveal: Option<Vec<u16>>,
}

impl Visibility {
    fn includes(&self, attributes: u32, name: &OsStr) -> bool {
        self.hidden
            || attributes & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) == 0
            || self.reveal.as_ref().is_some_and(|target| {
                let name = wide(name);
                unsafe {
                    CompareStringOrdinal(name.as_ptr(), -1, target.as_ptr(), -1, 1) == CSTR_EQUAL
                }
            })
    }
}

struct DirectoryRow {
    name: OsString,
    attributes: u32,
    size: u64,
    modified: u64,
}

struct BulkDirectory {
    handle: HANDLE,
    data: WIN32_FIND_DATAW,
    first: bool,
    done: bool,
}

impl BulkDirectory {
    fn open(folder: &Path) -> Option<Self> {
        // Keep Windows' standard path semantics. The standard-library path remains
        // available for UNC, extended paths, long paths and drivers rejecting bulk fetch.
        if !matches!(folder.components().next(), Some(std::path::Component::Prefix(prefix))
            if matches!(prefix.kind(), std::path::Prefix::Disk(_)))
            || !folder.is_absolute()
            || network_path(folder)
        {
            return None;
        }
        let query = wide(folder.join("*"));
        if query.len() >= 260 {
            return None;
        }
        let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
        let handle = unsafe {
            FindFirstFileExW(
                query.as_ptr(),
                FindExInfoBasic,
                (&mut data as *mut WIN32_FIND_DATAW).cast(),
                FindExSearchNameMatch,
                std::ptr::null(),
                FIND_FIRST_EX_LARGE_FETCH,
            )
        };
        (handle != INVALID_HANDLE_VALUE).then_some(Self {
            handle,
            data,
            first: true,
            done: false,
        })
    }
}

impl Drop for BulkDirectory {
    fn drop(&mut self) {
        if self.handle != INVALID_HANDLE_VALUE {
            unsafe { FindClose(self.handle) };
        }
    }
}

impl Iterator for BulkDirectory {
    type Item = io::Result<DirectoryRow>;

    fn next(&mut self) -> Option<Self::Item> {
        while !self.done {
            if !std::mem::take(&mut self.first)
                && unsafe { FindNextFileW(self.handle, &mut self.data) } == 0
            {
                self.done = true;
                let e = io::Error::last_os_error();
                return (e.raw_os_error() != Some(ERROR_NO_MORE_FILES as i32)).then_some(Err(e));
            }
            let data = &self.data;
            let name = &data.cFileName[..data
                .cFileName
                .iter()
                .position(|&unit| unit == 0)
                .unwrap_or(data.cFileName.len())];
            if matches!(name, [46] | [46, 46]) {
                continue;
            }
            return Some(Ok(DirectoryRow {
                name: OsString::from_wide(name),
                attributes: data.dwFileAttributes,
                size: (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow),
                modified: (u64::from(data.ftLastWriteTime.dwHighDateTime) << 32)
                    | u64::from(data.ftLastWriteTime.dwLowDateTime),
            }));
        }
        None
    }
}

enum DirectoryRows {
    Bulk(BulkDirectory),
    Standard(fs::ReadDir),
}

impl DirectoryRows {
    fn open(folder: &Path, bulk: bool) -> io::Result<Self> {
        if bulk {
            if let Some(read) = BulkDirectory::open(folder) {
                return Ok(Self::Bulk(read));
            }
        }
        fs::read_dir(folder).map(Self::Standard)
    }
}

impl Iterator for DirectoryRows {
    type Item = io::Result<DirectoryRow>;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Bulk(read) => read.next(),
            Self::Standard(read) => read.next().map(|item| {
                let item = item?;
                let m = item.metadata()?;
                Ok(DirectoryRow {
                    name: item.file_name(),
                    attributes: m.file_attributes(),
                    size: m.file_size(),
                    modified: m.last_write_time(),
                })
            }),
        }
    }
}

#[cfg(test)]
#[test]
fn native_listing_matches_standard_unicode_metadata() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("Größe 10.txt"), b"content")?;
    fs::create_dir(root.path().join("Ordner 2"))?;
    let entries = |bulk| -> io::Result<Vec<_>> {
        let mut rows = DirectoryRows::open(root.path(), bulk)?
            .map(|row| row.map(|row| (row.name, row.attributes, row.size, row.modified)))
            .collect::<io::Result<Vec<_>>>()?;
        rows.sort();
        Ok(rows)
    };
    assert_eq!(entries(false)?, entries(true)?);
    Ok(())
}

fn enumerate(
    root: PathBuf,
    query: Option<String>,
    visibility: Visibility,
    stale: &impl Fn() -> bool,
    entries: &mut Vec<Arc<Entry>>,
    error: &mut Option<String>,
    mut batch: impl FnMut(Vec<Arc<Entry>>, Option<[Instant; 4]>),
) {
    let probe = query.is_none() && std::env::var_os("BETTEREXPLORER_BENCH_PROBE").is_some();
    let bulk =
        query.is_none() && !(probe && std::env::var_os("BETTEREXPLORER_BENCH_STANDARD").is_some());
    let mut pending = vec![root.clone()];
    let mut delivered = entries.len();
    let mut last_batch = Instant::now();
    let mut unreadable = 0usize;
    while let Some(folder) = pending.pop() {
        if stale() {
            return;
        }
        let opening = probe.then(Instant::now);
        let read = match DirectoryRows::open(&folder, bulk) {
            Ok(read) => read,
            Err(failure) => {
                if folder == root {
                    *error = Some(format!("Can't open {}: {failure}", root.display()));
                } else {
                    unreadable += 1;
                }
                continue;
            }
        };
        let opened = probe.then(Instant::now);
        for item in read {
            if stale() {
                return;
            }
            let Ok(item) = item else {
                unreadable += 1;
                continue;
            };
            let attributes = item.attributes;
            let name = item.name;
            if !visibility.includes(attributes, &name) {
                continue;
            }
            let matched = (probe && delivered == 0).then(Instant::now);
            let directory = attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
            if query.is_some() && directory && attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
                pending.push(folder.join(&name));
            }
            if !query
                .as_ref()
                .is_some_and(|query| !name.to_string_lossy().to_lowercase().contains(query))
            {
                let extension = if directory {
                    Vec::new()
                } else {
                    Path::new(&name).extension().map(wide).unwrap_or_default()
                };
                entries.push(Arc::new(Entry {
                    path: folder.join(&name),
                    name: wide(&name),
                    extension,
                    directory,
                    size: item.size,
                    modified: item.modified,
                    location: false,
                    capacity: None,
                    shell_item: None,
                }));
            }
            let count = entries.len() - delivered;
            if count != 0
                && (delivered == 0
                    || (count.is_multiple_of(32)
                        && last_batch.elapsed() >= Duration::from_millis(45)))
            {
                let cutoff = entries.len();
                let rows = entries[delivered..cutoff].to_vec();
                let timing = matched
                    .zip(opening.zip(opened))
                    .map(|(matched, (opening, opened))| [opening, opened, matched, Instant::now()]);
                batch(rows, timing);
                delivered = cutoff;
                last_batch = Instant::now();
            }
        }
    }
    if unreadable != 0 && error.is_none() {
        *error = Some(format!(
            "{unreadable} entries or folders could not be read."
        ));
    }
}

type Lane = Arc<(Mutex<Option<(Request, Option<PathBuf>)>>, Condvar)>;

pub struct Scanner {
    latest: Arc<AtomicU64>,
    local: Lane,
    remote: Lane,
}

impl Scanner {
    pub fn start(publish: impl Fn(ScanEvent) + Send + Sync + 'static) -> Self {
        let latest = Arc::new(AtomicU64::new(0));
        let publish = Arc::new(publish);
        let lane = |label: &str| {
            let mailbox: Lane = Arc::new((Mutex::new(None), Condvar::new()));
            let worker_mailbox = Arc::clone(&mailbox);
            let latest = Arc::clone(&latest);
            let publish = Arc::clone(&publish);
            std::thread::Builder::new()
                .name(label.into())
                .spawn(move || loop {
                    let (lock, ready) = &*worker_mailbox;
                    let mut request = ready
                        .wait_while(lock.lock().unwrap(), |request| request.is_none())
                        .unwrap();
                    let (work, reveal) = request.take().unwrap();
                    drop(request);
                    scan_revealing(work, reveal, &latest, |event| publish(event));
                })
                .expect("start directory worker");
            mailbox
        };
        Self {
            local: lane("local files"),
            remote: lane("network files"),
            latest,
        }
    }

    pub fn cancel(&self, id: u64) {
        self.latest.store(id, Ordering::Relaxed);
    }

    pub fn request(&self, request: Request) {
        self.request_revealing(request, None);
    }

    pub fn request_revealing(&self, request: Request, reveal: Option<PathBuf>) {
        self.cancel(request.id);
        let lane = if matches!(
            request.location,
            Location::Namespace {
                kind: namespace::Namespace::Network,
                ..
            } | Location::PcSearch { .. }
        ) || request.location.directory().is_some_and(network_path)
        {
            &self.remote
        } else {
            &self.local
        };
        *lane.0.lock().unwrap() = Some((request, reveal));
        lane.1.notify_one();
    }
}

pub fn network_path(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return false;
    };
    match prefix.kind() {
        Prefix::UNC(..) | Prefix::VerbatimUNC(..) => true,
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
            let root = [letter as u16, b':' as u16, b'\\' as u16, 0];
            unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(root.as_ptr()) == 4 }
        }
        _ => false,
    }
}

pub fn input_path(value: OsString, current: Option<&Path>) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else if let Some(current) = current {
        current.join(path)
    } else {
        path
    }
}

struct WatchRequest {
    id: u64,
    path: Option<PathBuf>,
}

struct WatchSignal {
    handle: usize,
    request: Mutex<Option<WatchRequest>>,
    done: std::sync::atomic::AtomicBool,
}

impl Drop for WatchSignal {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle as _);
        }
    }
}

pub struct Watcher(Arc<WatchSignal>);

impl Watcher {
    pub fn start(changed: impl Fn(u64) + Send + 'static) -> Option<Self> {
        use windows_sys::Win32::System::Threading::CreateEventW;
        let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
        if handle.is_null() {
            return None;
        }
        let signal = Arc::new(WatchSignal {
            handle: handle as usize,
            request: Mutex::new(None),
            done: std::sync::atomic::AtomicBool::new(false),
        });
        let worker = Arc::clone(&signal);
        std::thread::Builder::new()
            .name("folder changes".into())
            .spawn(move || watch_loop(worker, changed))
            .ok()?;
        Some(Self(signal))
    }

    pub fn watch(&self, id: u64, path: Option<PathBuf>) {
        *self.0.request.lock().unwrap() = Some(WatchRequest { id, path });
        unsafe {
            windows_sys::Win32::System::Threading::SetEvent(self.0.handle as _);
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.0.done.store(true, Ordering::Release);
        unsafe {
            windows_sys::Win32::System::Threading::SetEvent(self.0.handle as _);
        }
    }
}

fn watch_loop(signal: Arc<WatchSignal>, changed: impl Fn(u64)) {
    use windows_sys::Win32::{
        Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0},
        Storage::FileSystem::*,
        System::Threading::{WaitForMultipleObjects, INFINITE},
    };
    let mut directory: HANDLE = INVALID_HANDLE_VALUE;
    let mut id = 0;
    loop {
        let handles = [signal.handle as HANDLE, directory];
        let state = unsafe {
            WaitForMultipleObjects(
                if directory == INVALID_HANDLE_VALUE {
                    1
                } else {
                    2
                },
                handles.as_ptr(),
                0,
                INFINITE,
            )
        };
        if state == WAIT_OBJECT_0 {
            if signal.done.load(Ordering::Acquire) {
                break;
            }
            let Some(request) = signal.request.lock().unwrap().take() else {
                continue;
            };
            if directory != INVALID_HANDLE_VALUE {
                unsafe {
                    FindCloseChangeNotification(directory);
                }
                directory = INVALID_HANDLE_VALUE;
            }
            id = request.id;
            if let Some(path) = request.path.filter(|path| !network_path(path)) {
                directory = unsafe {
                    FindFirstChangeNotificationW(
                        wide(path).as_ptr(),
                        0,
                        FILE_NOTIFY_CHANGE_FILE_NAME
                            | FILE_NOTIFY_CHANGE_DIR_NAME
                            | FILE_NOTIFY_CHANGE_LAST_WRITE
                            | FILE_NOTIFY_CHANGE_SIZE
                            | FILE_NOTIFY_CHANGE_ATTRIBUTES,
                    )
                };
            }
        } else if state == WAIT_OBJECT_0 + 1 {
            changed(id);
            if unsafe { FindNextChangeNotification(directory) } == 0 {
                unsafe {
                    FindCloseChangeNotification(directory);
                }
                directory = INVALID_HANDLE_VALUE;
            }
        } else {
            break;
        }
    }
    if directory != INVALID_HANDLE_VALUE {
        unsafe {
            FindCloseChangeNotification(directory);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_display_name_can_already_hide_the_file_extension() {
        let mut entry = Entry::place(PathBuf::new(), "Report.pdf");
        entry.directory = false;
        entry.extension = wide("pdf");
        assert_eq!(from_wide(entry.display_name(true)), "Report");
        assert_eq!(from_wide(entry.display_name(false)), "Report.pdf");

        entry.name = wide("Report");
        assert_eq!(from_wide(entry.display_name(true)), "Report");
    }

    #[test]
    fn modified_groups_follow_local_calendar_boundaries() {
        let groups = DateGroups::for_local_date(2026, 11, 24).unwrap();
        let modified = |month, day, hour, minute| {
            let local = SYSTEMTIME {
                wYear: 2026,
                wMonth: month,
                wDay: day,
                wHour: hour,
                wMinute: minute,
                ..unsafe { std::mem::zeroed() }
            };
            let mut utc = unsafe { std::mem::zeroed() };
            assert_ne!(
                unsafe { TzSpecificLocalTimeToSystemTimeEx(std::ptr::null(), &local, &mut utc) },
                0
            );
            system_time_ticks(&utc).unwrap()
        };
        assert_eq!(groups.group(modified(11, 24, 0, 0)), DateGroup::Today);
        assert_eq!(groups.group(modified(11, 23, 23, 59)), DateGroup::Yesterday);
        assert_eq!(
            groups.group(modified(11, 11, 12, 0)),
            DateGroup::EarlierThisMonth
        );
        assert_eq!(groups.group(modified(10, 21, 12, 0)), DateGroup::LastMonth);
        assert_eq!(groups.group(0), DateGroup::Unknown);
    }

    #[test]
    fn date_modified_sort_interleaves_folders_and_files() {
        let entry = |name: &str, directory, modified| {
            let mut entry = Entry::place(PathBuf::from(name), name);
            entry.location = false;
            entry.directory = directory;
            entry.modified = modified;
            Arc::new(entry)
        };
        let mut entries = [
            entry("older.txt", false, 1),
            entry("folder", true, 2),
            entry("newer.txt", false, 3),
        ];

        sort_entries(
            &mut entries,
            Sort {
                column: Column::Modified,
                descending: true,
            },
        );

        assert_eq!(
            entries.map(|entry| entry.label()),
            ["newer.txt", "folder", "older.txt"]
        );
    }
}
