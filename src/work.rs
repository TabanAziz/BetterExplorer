use crate::{
    model::{from_wide, wide, Location, ModifiedDateFormat},
    namespace,
};
use std::{
    cmp::Ordering as Order,
    ffi::{OsStr, OsString},
    fs,
    os::windows::fs::MetadataExt,
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
    Foundation::{FILETIME, SYSTEMTIME},
    Storage::FileSystem::{
        FileTimeToLocalFileTime, GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDriveStringsW,
        GetVolumeInformationW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SYSTEM,
    },
    System::Time::FileTimeToSystemTime,
    UI::Shell::StrCmpLogicalW,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Column {
    #[default]
    Name,
    Modified,
    Kind,
    Size,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Sort {
    pub column: Column,
    pub descending: bool,
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
        if hide_extensions && !self.directory && !self.extension.is_empty() {
            &self.name[..self.name.len().saturating_sub(self.extension.len() + 1)]
        } else {
            &self.name[..self.name.len().saturating_sub(1)]
        }
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
        let utc = FILETIME {
            dwLowDateTime: self.modified as u32,
            dwHighDateTime: (self.modified >> 32) as u32,
        };
        let mut local = unsafe { std::mem::zeroed() };
        let mut date: SYSTEMTIME = unsafe { std::mem::zeroed() };
        if unsafe {
            FileTimeToLocalFileTime(&utc, &mut local) != 0
                && FileTimeToSystemTime(&local, &mut date) != 0
        } {
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
    Done(u64, Vec<Arc<Entry>>, Option<String>),
}

pub fn sort_entries(entries: &mut [Arc<Entry>], sort: Sort) {
    entries.sort_unstable_by(|left, right| {
        let name_order =
            || unsafe { StrCmpLogicalW(left.name.as_ptr(), right.name.as_ptr()) }.cmp(&0);
        let field = match sort.column {
            Column::Name => name_order(),
            Column::Modified => left.modified.cmp(&right.modified),
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
        let field = if sort.descending {
            field.reverse()
        } else {
            field
        };
        match right.directory.cmp(&left.directory) {
            Order::Equal => field,
            order => order,
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

pub fn scan(request: Request, latest: &AtomicU64, mut publish: impl FnMut(ScanEvent)) {
    let Request {
        id,
        location,
        hidden,
        sort,
        pins,
    } = request;
    let stale = || latest.load(Ordering::Relaxed) != id;
    if stale() {
        return;
    }
    let mut entries = Vec::new();
    let mut error = None;
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
                    Arc::new(Entry {
                        path: PathBuf::from(&item.key),
                        name: wide(&item.name),
                        extension: Vec::new(),
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
        location => {
            let (root, query) = match location {
                Location::Folder(root) => (root, None),
                Location::Search { root, query } => (root, Some(query.to_lowercase())),
                _ => unreachable!(),
            };
            enumerate(
                root,
                query,
                hidden,
                &stale,
                &mut entries,
                &mut error,
                |rows| publish(ScanEvent::Batch(id, rows)),
            );
        }
    }
    if stale() {
        return;
    }
    sort_entries(&mut entries, sort);
    if !stale() {
        publish(ScanEvent::Done(id, entries, error));
    }
}

fn enumerate(
    root: PathBuf,
    query: Option<String>,
    hidden: bool,
    stale: &impl Fn() -> bool,
    entries: &mut Vec<Arc<Entry>>,
    error: &mut Option<String>,
    mut batch: impl FnMut(Vec<Arc<Entry>>),
) {
    let mut pending = vec![root.clone()];
    let mut delivered = 0;
    let mut last_batch = Instant::now();
    let mut unreadable = 0usize;
    while let Some(folder) = pending.pop() {
        if stale() {
            return;
        }
        let read = match fs::read_dir(&folder) {
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
        for item in read {
            if stale() {
                return;
            }
            let Ok(item) = item else {
                unreadable += 1;
                continue;
            };
            let Ok(metadata) = item.metadata() else {
                unreadable += 1;
                continue;
            };
            let attributes = metadata.file_attributes();
            if !hidden && attributes & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0 {
                continue;
            }
            let directory = attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
            let name = item.file_name();
            if query.is_some() && directory && attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
                pending.push(item.path());
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
                    path: item.path(),
                    name: wide(&name),
                    extension,
                    directory,
                    size: metadata.file_size(),
                    modified: metadata.last_write_time(),
                    location: false,
                    capacity: None,
                    shell_item: None,
                }));
            }
            if entries.len() > delivered
                && (delivered == 0 || last_batch.elapsed() >= Duration::from_millis(45))
            {
                let cutoff = entries.len();
                batch(entries[delivered..cutoff].to_vec());
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

type Lane = Arc<(Mutex<Option<Request>>, Condvar)>;

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
                    let work = request.take().unwrap();
                    drop(request);
                    scan(work, &latest, |event| publish(event));
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
        self.cancel(request.id);
        let lane = if matches!(
            request.location,
            Location::Namespace {
                kind: namespace::Namespace::Network,
                ..
            }
        ) || request.location.directory().is_some_and(network_path)
        {
            &self.remote
        } else {
            &self.local
        };
        *lane.0.lock().unwrap() = Some(request);
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
