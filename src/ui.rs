use crate::hotkey;
use crate::{
    benchmark::{self, Bench},
    breadcrumb,
    chrome::*,
    home_page::{Activity, HomePage, Icon, Item},
    preview::{self, Content},
};
use better_explorer::{
    model::{
        from_wide, read_settings, safe_name, wide, write_settings, GroupBy, History, Location,
        ModifiedDateFormat, Settings, ThemePreference,
    },
    namespace::Namespace,
    shell::{self, Operation},
    work::{self, Column, DateGroup, DateGroups, Entry, Request, ScanEvent, Scanner, Sort},
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashSet, VecDeque},
    ffi::{OsStr, OsString},
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
    ptr::{copy_nonoverlapping, null, null_mut},
    sync::{
        mpsc::{sync_channel, SyncSender},
        Arc, Mutex,
    },
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::{
        LibraryLoader::GetModuleHandleW, SystemServices::SS_NOPREFIX, Threading::CreateMutexW,
    },
    UI::{
        Controls::*, HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::*, Shell::*,
        WindowsAndMessaging::*,
    },
};

const WAKE: u32 = WM_APP + 1;
const SHOW_MENU: u32 = WM_APP + 2;
const SELECTION_CHANGED: u32 = WM_APP + 3;
const CHANGE: u32 = WM_APP + 4;
const HOVER: u32 = WM_APP + 5;
const BENCH_NAVIGATE: u32 = WM_APP + 6;
const BENCH_PRESENT: u32 = WM_APP + 7;
const BENCH_STARTED: u32 = WM_APP + 8;
const REFRESH_TIMER: usize = 1;
const SETTINGS_TIMER: usize = 2;
const GROUP_TIMER: usize = 3;
const RESIZE_EDGE: i32 = 7;

#[derive(Clone, Copy)]
struct ControlColors {
    text: u32,
    muted: u32,
    surfaces: [u32; 4],
    brushes: [HBRUSH; 4],
}

thread_local! {
    // Controls can request their brushes during synchronous layout and text changes.
    static CONTROL_COLORS: Cell<Option<ControlColors>> = const { Cell::new(None) };
}

const LIST: u16 = 100;
const ADDRESS: u16 = 101;
const SEARCH: u16 = 102;
const BACK: u16 = 103;
const FORWARD: u16 = 104;
const UP: u16 = 105;
const REFRESH: u16 = 106;
const PATH: u16 = 107;
const NEW: u16 = 108;
const CUT: u16 = 109;
const COPY: u16 = 110;
const PASTE: u16 = 111;
const RENAME: u16 = 112;
const DELETE: u16 = 113;
const SORT: u16 = 114;
const VIEW: u16 = 115;
const FILTER: u16 = 116;
const MORE: u16 = 117;
const DETAILS: u16 = 118;
const NEW_TAB: u16 = 119;
const CLOSE_TAB: u16 = 120;
const OPEN: u16 = 121;
const OPEN_TAB: u16 = 122;
const OPEN_WINDOW: u16 = 123;
const OPEN_WITH: u16 = 124;
const PROPERTIES: u16 = 125;
const COPY_PATH: u16 = 126;
const REVEAL: u16 = 127;
const PIN: u16 = 128;
const SHORTCUT: u16 = 129;
const NEW_FOLDER: u16 = 130;
const NEW_TEXT: u16 = 131;
const PERMANENT: u16 = 132;
const ENTER_PATH: u16 = 133;
const ENTER_SEARCH: u16 = 134;
const ACCEPT: u16 = 135;
const CANCEL: u16 = 136;
const NAME: u16 = 137;
const STATUS: u16 = 138;
const STOP_SEARCH: u16 = 139;
const SELECT_ALL: u16 = 140;
const DETAILS_VIEW: u16 = 141;
const LIST_VIEW: u16 = 142;
const ICON_VIEW: u16 = 143;
const SHOW_HIDDEN: u16 = 144;
const SHOW_EXTENSIONS: u16 = 145;
const SORT_NAME: u16 = 146;
const SORT_MODIFIED: u16 = 147;
const SORT_KIND: u16 = 148;
const SORT_SIZE: u16 = 149;
const SORT_REVERSE: u16 = 150;
const FILTER_ALL: u16 = 151;
const FILTER_FOLDERS: u16 = 152;
const FILTER_FILES: u16 = 153;
const THEME_SYSTEM: u16 = 154;
const THEME_DARK: u16 = 155;
const THEME_LIGHT: u16 = 156;
const RESTORE_TABS: u16 = 157;
const MOVE_TAB_LEFT: u16 = 158;
const MOVE_TAB_RIGHT: u16 = 159;
const NEW_WINDOW: u16 = 160;
const NEXT_TAB: u16 = 161;
const PREVIOUS_TAB: u16 = 162;
const CONTEXT: u16 = 163;
const EDIT_ADDRESS: u16 = 164;
const CARDS_VIEW: u16 = 184;
const SMALL_ICONS: u16 = 185;
const LARGE_ICONS: u16 = 186;
const XLARGE_ICONS: u16 = 187;
const COMPRESS: u16 = 188;
const EXTRACT: u16 = 189;
const HOME: u16 = 170;
const COMPUTER: u16 = 171;
const NETWORK: u16 = 180;
const RECYCLE: u16 = 181;
const LIBRARIES: u16 = 182;
const OPEN_SETTINGS: u16 = 190;
const WIN_E_WINDOWS: u16 = 191;
const WIN_E_BETTER: u16 = 192;
const NAVIGATION: u16 = 193;
const SUMMARY: u16 = 205;
const DATE_FORMAT_DAY_FIRST: u16 = 206;
const DATE_FORMAT_MONTH_FIRST: u16 = 207;
const RECYCLE_RESTORE: u16 = 208;
const RECYCLE_EMPTY: u16 = 209;
const GROUP_NONE: u16 = 214;
const GROUP_MODIFIED: u16 = 215;
const OPEN_TERMINAL: u16 = 210;
const FOLDER_PROPERTIES: u16 = 211;
const FOLDER_COPY_PATH: u16 = 212;
const GROUP_SUBMENU: u16 = 213;
const HOME_RECENT: u16 = 194;
const HOME_FAVORITES: u16 = 195;
const HOME_TILES: u16 = 196;
const HOME_CONTENT: u16 = 197;
const FAVORITE: u16 = 198;
const FILTER_TYPES: [(u16, Filter, &str); 6] = [
    (199, Filter::Word, "Word"),
    (200, Filter::Excel, "Excel"),
    (201, Filter::PowerPoint, "PowerPoint"),
    (202, Filter::Pictures, "Pictures"),
    (203, Filter::Videos, "Videos"),
    (204, Filter::Pdf, "PDF"),
];
const HOME_QUICK_START: u16 = 600;
const HOME_ACTIVITY_START: u16 = 700;
const TAB_START: u16 = 300;
const BREADCRUMB_START: u16 = 1000;
const TAB_LIMIT: usize = 10;
const FRAME_MIN: u16 = 165;
const FRAME_MAX: u16 = 166;
const FRAME_CLOSE: u16 = 167;
const QUICK_DETAILS: u16 = 168;
const QUICK_ICONS: u16 = 169;
const CRUMB_START: u16 = 520;
const CRUMB_LIMIT: usize = 32;
const DRIVE_START: u16 = 560;
const DRIVE_LIMIT: usize = 26;
const VIEW_CARDS: u32 = 10;
const VIEW_LARGE: u32 = 6;
const VIEW_SMALL: u32 = 7;
const VIEW_XLARGE: u32 = 8;
const COLUMNS: [(&str, i32); 4] = [
    ("Name", 325),
    ("Date modified", 143),
    ("Type", 115),
    ("Size", 88),
];

const COMMANDS: &[(u16, &str, i32)] = &[
    (NEW, "New ▾", 84),
    (CUT, "Cut", 43),
    (COPY, "Copy", 43),
    (PASTE, "Paste", 43),
    (RENAME, "Rename", 43),
    (DELETE, "Delete", 43),
    (SORT, "Sort  ▾", 82),
    (VIEW, "View  ▾", 84),
    (FILTER, "Filter  ▾", 87),
    (MORE, "More options", 45),
    (DETAILS, "Details", 85),
    (OPEN_SETTINGS, "Settings", 91),
];
const PLACES: [&str; 6] = [
    "Desktop",
    "Downloads",
    "Documents",
    "Pictures",
    "Music",
    "Videos",
];
const MENU_IDS: [u16; 7] = [NEW, SORT, VIEW, FILTER, MORE, PATH, CONTEXT];

enum Event {
    Scan(ScanEvent),
    Preview(u64, Option<Content>),
    Boot(Settings, Vec<(PathBuf, &'static str)>),
    SettingsError(String),
    Hotkey {
        status: Option<hotkey::Status>,
        error: Option<String>,
    },
    Operation {
        result: Result<String, String>,
        refresh: bool,
        opened: Option<PathBuf>,
    },
}

type Events = Arc<Mutex<VecDeque<Event>>>;

fn publish(queue: &Events, window: usize, event: Event) {
    let mut queue = queue.lock().unwrap();
    let signal = queue.is_empty();
    queue.push_back(event);
    if signal {
        unsafe {
            PostMessageW(window as HWND, WAKE, 0, 0);
        }
    }
}

struct Tab {
    history: History,
    sort: Sort,
    filter: Filter,
    view: u32,
}

struct ListSelection {
    paths: HashSet<PathBuf>,
    focused: Option<PathBuf>,
    anchor: Option<PathBuf>,
}

impl Tab {
    fn new(location: Location) -> Self {
        let view = if matches!(location, Location::Computer) {
            VIEW_CARDS
        } else {
            LV_VIEW_DETAILS
        };
        Self {
            history: History::new(location),
            sort: Sort::default(),
            filter: Filter::All,
            view,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Folders,
    Files,
    Word,
    Excel,
    PowerPoint,
    Pictures,
    Videos,
    Pdf,
}

impl Filter {
    fn shows(self, item: &Entry) -> bool {
        let path = item
            .shell_item
            .as_ref()
            .map_or(item.path.as_path(), |shell| Path::new(&shell.name));
        self.matches(path, item.directory)
    }

    fn matches(self, path: &Path, directory: bool) -> bool {
        let extensions: &[&str] = match self {
            Self::All => return true,
            Self::Folders => return directory,
            Self::Files => return !directory,
            Self::Word => &["doc", "docx", "docm", "dot", "dotx", "odt", "rtf"],
            Self::Excel => &["xls", "xlsx", "xlsm", "xlsb", "csv", "ods"],
            Self::PowerPoint => &["ppt", "pptx", "pptm", "ppsx", "odp"],
            Self::Pictures => &[
                "png", "jpg", "jpeg", "gif", "bmp", "webp", "tiff", "heic", "svg", "ico", "avif",
            ],
            Self::Videos => &["mp4", "mov", "mkv", "webm", "avi", "wmv", "m4v", "mpeg"],
            Self::Pdf => &["pdf"],
        };
        !directory
            && path.extension().is_some_and(|extension| {
                extensions
                    .iter()
                    .any(|kind| extension.eq_ignore_ascii_case(kind))
            })
    }
}

enum Edit {
    Create(PathBuf, bool),
    Rename(PathBuf, Option<OsString>),
    Delete(Vec<PathBuf>),
    Shortcut(PathBuf, PathBuf),
    Compress(Vec<PathBuf>, PathBuf),
    Extract(PathBuf, PathBuf),
}

#[derive(Clone, Copy)]
struct SettingsControls {
    heading: HWND,
    description: HWND,
    state: HWND,
    date_format_heading: HWND,
}

#[derive(Clone, Copy)]
enum HotkeyRequest {
    Read,
    Change(bool),
}

#[derive(PartialEq, Eq)]
struct NavRow {
    label: String,
    target: Option<Location>,
    command: u16,
    icon: u16,
    pinned: bool,
}

struct App {
    window: HWND,
    buttons: Vec<(u16, HWND)>,
    list: HWND,
    navigation: HWND,
    nav_rows: Vec<NavRow>,
    home: Option<HomePage>,
    home_activity: Activity,
    home_filter: Filter,
    popup_items: Vec<MenuItem>,
    home_quick_paths: Vec<PathBuf>,
    home_activity_paths: Vec<PathBuf>,
    address: HWND,
    search: HWND,
    title: HWND,
    summary: HWND,
    detail_title: HWND,
    detail_body: HWND,
    status: HWND,
    edit_label: HWND,
    edit_name: HWND,
    images: [HIMAGELIST; 2],
    icon_size: i32,
    fitted_columns: Option<[i32; 4]>,
    fonts: [HFONT; 2],
    brushes: [HBRUSH; 4],
    dpi: u32,
    colors: Palette,
    tabs: Vec<Tab>,
    active: usize,
    settings: Settings,
    settings_controls: Option<SettingsControls>,
    hotkey_status: Option<hotkey::Status>,
    hotkey_request: Option<HotkeyRequest>,
    hotkey_error: Option<String>,
    places: Vec<(PathBuf, &'static str)>,
    crumbs: Vec<PathBuf>,
    crumb_slots: Vec<Option<Location>>,
    crumb_segments: Vec<(String, Location)>,
    crumb_widths: Vec<i32>,
    editing_address: bool,
    header: HWND,
    hover: HWND,
    bench: Option<Bench>,
    preview: Option<Content>,
    preview_worker: Option<preview::Worker>,
    preview_key: Option<(PathBuf, u64, u64)>,
    preview_id: u64,
    rows: Vec<Arc<Entry>>,
    visible: Vec<usize>,
    group_headers: Vec<(usize, DateGroup)>,
    edit: Option<Edit>,
    error: Option<String>,
    note: Option<String>,
    scanning: bool,
    busy: usize,
    id: u64,
    events: Events,
    scanner: Option<Scanner>,
    operations: Option<SyncSender<Operation>>,
    clipboard_write: Option<shell::ClipboardWriteReceipt>,
    keep_rows: bool,
    saved: Option<SyncSender<Settings>>,
    save_worker: Option<std::thread::JoinHandle<()>>,
    save_enabled: bool,
    watcher: Option<work::Watcher>,
    booted: bool,
    launch_path: Option<PathBuf>,
    selection_event: bool,
}

impl App {
    fn create(window: HWND, launch_path: Option<PathBuf>, save_enabled: bool) -> Self {
        let mut buttons = Vec::new();
        for (id, label) in [
            (NEW_TAB, "New tab"),
            (CLOSE_TAB, "Close tab"),
            (BACK, "Back"),
            (FORWARD, "Forward"),
            (UP, "Up"),
            (REFRESH, "Refresh"),
            (PATH, "Path choices"),
            (ACCEPT, "Apply"),
            (CANCEL, "Cancel"),
            (STOP_SEARCH, "Stop search"),
            (FRAME_MIN, "Minimize"),
            (FRAME_MAX, "Maximize or restore"),
            (FRAME_CLOSE, "Close window"),
            (QUICK_DETAILS, "Details view"),
            (QUICK_ICONS, "Icon view"),
        ] {
            buttons.push((id, button(window, label, id)));
        }
        for &(id, label, _) in COMMANDS {
            buttons.push((id, button(window, label, id)));
        }
        let label = |id| new_control(window, "Static", "", id, SS_NOPREFIX, 0);
        let input = |id| {
            new_control(
                window,
                "Edit",
                "",
                id,
                WS_TABSTOP | if id == NAME { WS_BORDER } else { 0 } | ES_AUTOHSCROLL as u32,
                0,
            )
        };
        let style = LVS_REPORT | LVS_OWNERDATA | LVS_SHOWSELALWAYS | LVS_SHAREIMAGELISTS;
        let list = new_control(
            window,
            "SysListView32",
            "Files",
            LIST,
            WS_TABSTOP | style,
            0,
        );
        let navigation = new_control(
            window,
            "ListBox",
            "Navigation",
            NAVIGATION,
            WS_TABSTOP
                | WS_VSCROLL
                | (LBS_NOTIFY | LBS_OWNERDRAWFIXED | LBS_HASSTRINGS | LBS_NOINTEGRALHEIGHT) as u32,
            0,
        );
        let dpi = unsafe { GetDpiForWindow(window) }.max(96);
        let colors = Palette::read(ThemePreference::System);
        let mut app = Self {
            window,
            buttons,
            list,
            navigation,
            nav_rows: Vec::new(),
            home: HomePage::create(window),
            home_activity: Activity::Recent,
            home_filter: Filter::All,
            popup_items: Vec::new(),
            home_quick_paths: Vec::new(),
            home_activity_paths: Vec::new(),
            address: input(ADDRESS),
            search: input(SEARCH),
            title: label(0),
            summary: label(SUMMARY),
            detail_title: label(0),
            detail_body: label(0),
            status: label(STATUS),
            edit_label: label(0),
            edit_name: input(NAME),
            images: [0; 2],
            icon_size: 52,
            fitted_columns: None,
            fonts: [null_mut(); 2],
            brushes: [null_mut(); 4],
            dpi,
            colors,
            tabs: vec![Tab::new(
                launch_path
                    .clone()
                    .map(Location::Folder)
                    .unwrap_or_default(),
            )],
            active: 0,
            settings: Settings::default(),
            settings_controls: None,
            hotkey_status: None,
            hotkey_request: None,
            hotkey_error: None,
            places: Vec::new(),
            crumbs: Vec::new(),
            crumb_slots: Vec::new(),
            crumb_segments: Vec::new(),
            crumb_widths: Vec::new(),
            editing_address: false,
            header: send(list, LVM_GETHEADER, 0, 0) as HWND,
            hover: null_mut(),
            bench: Bench::configured(),
            preview: None,
            preview_worker: None,
            preview_key: None,
            preview_id: 0,
            rows: Vec::new(),
            visible: Vec::new(),
            group_headers: Vec::new(),
            edit: None,
            note: None,
            error: None,
            scanning: true,
            busy: 0,
            id: 0,
            events: Arc::new(Mutex::new(VecDeque::new())),
            scanner: None,
            operations: None,
            clipboard_write: None,
            keep_rows: false,
            saved: None,
            save_worker: None,
            save_enabled,
            watcher: None,
            booted: false,
            launch_path,
            selection_event: false,
        };
        unsafe {
            better_explorer::scroll::install(list);
        }
        send(list, CCM_SETUNICODEFORMAT, 1, 0);
        let styles =
            LVS_EX_DOUBLEBUFFER | LVS_EX_FULLROWSELECT | LVS_EX_HEADERDRAGDROP | LVS_EX_LABELTIP;
        send(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            styles as usize,
            styles as isize,
        );
        for (index, (title, width)) in [
            ("Name", 325),
            ("Date modified", 143),
            ("Type", 115),
            ("Size", 88),
        ]
        .into_iter()
        .enumerate()
        {
            let mut title = wide(title);
            let column = LVCOLUMNW {
                mask: LVCF_TEXT | LVCF_WIDTH | LVCF_FMT,
                fmt: if index == 3 {
                    LVCFMT_RIGHT
                } else {
                    LVCFMT_LEFT
                },
                pszText: title.as_mut_ptr(),
                cx: scale(width, dpi),
                ..unsafe { zeroed() }
            };
            send(list, LVM_INSERTCOLUMNW, index, &column as *const _ as isize);
        }
        app.apply_dpi(dpi);
        app.apply_theme();
        let cue = wide("Search this folder");
        send(app.search, EM_SETCUEBANNER, 1, cue.as_ptr() as isize);
        unsafe {
            DragAcceptFiles(window, 1);
        }
        app.update_sidebar();
        app.update_navigation();
        app.status();
        app.layout();
        app
    }

    fn start(&mut self) {
        let owner = self.window as usize;
        let scan_queue = Arc::clone(&self.events);
        self.scanner = Some(Scanner::start(move |event| {
            publish(&scan_queue, owner, Event::Scan(event))
        }));
        let (operations, receiver) = sync_channel::<Operation>(8);
        let operation_queue = Arc::clone(&self.events);
        std::thread::Builder::new()
            .name("Windows file operations".into())
            .spawn(move || {
                for operation in receiver {
                    let refresh = operation.updates_listing();
                    let opened = if let Operation::Open(path) = &operation {
                        Some(path.clone())
                    } else {
                        None
                    };
                    let result = shell::run(owner, operation);
                    let opened = opened.filter(|_| result.is_ok());
                    publish(
                        &operation_queue,
                        owner,
                        Event::Operation {
                            result,
                            refresh,
                            opened,
                        },
                    );
                }
            })
            .expect("start Windows operation worker");
        self.operations = Some(operations);
        let (saved, receiver) = sync_channel::<Settings>(2);
        let settings_queue = Arc::clone(&self.events);
        self.save_worker = Some(
            std::thread::Builder::new()
                .name("Explorer settings".into())
                .spawn(move || {
                    publish(
                        &settings_queue,
                        owner,
                        Event::Boot(read_settings(), work::known_places()),
                    );
                    while let Ok(settings) = receiver.recv() {
                        if let Err(error) = write_settings(&settings) {
                            publish(
                                &settings_queue,
                                owner,
                                Event::SettingsError(format!("Can't save settings: {error}")),
                            );
                        }
                    }
                })
                .expect("start settings worker"),
        );
        self.saved = Some(saved);
        self.watcher = work::Watcher::start(move |id| unsafe {
            PostMessageW(owner as HWND, CHANGE, id as usize, 0);
        });
        self.request_scan();
        self.focus_content();
    }

    fn focus_content(&self) {
        if let Some(home) = self
            .home
            .as_ref()
            .filter(|_| *self.location() == Location::Home)
        {
            home.focus();
        } else {
            unsafe {
                SetFocus(self.list);
            }
        }
    }

    fn button(&self, id: u16) -> HWND {
        self.buttons
            .iter()
            .find(|(key, _)| *key == id)
            .map_or(null_mut(), |(_, handle)| *handle)
    }

    fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }
    fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }
    fn location(&self) -> &Location {
        self.tab().history.current()
    }
    fn location_dir(&self) -> Option<PathBuf> {
        self.location().directory().map(Path::to_path_buf)
    }

    fn dest(&self) -> Option<PathBuf> {
        match self.location() {
            Location::Folder(path) => Some(path.clone()),
            _ => None,
        }
    }

    fn sort(&self) -> Sort {
        match self.location() {
            Location::Folder(path) => self.settings.folder_order(path).sort,
            _ => self.tab().sort,
        }
    }

    fn group_by(&self) -> GroupBy {
        match self.location() {
            Location::Folder(path) if self.tab().view == LV_VIEW_DETAILS => {
                self.settings.folder_order(path).group
            }
            _ => GroupBy::None,
        }
    }

    fn group_menu(&self) -> Vec<MenuItem> {
        if !matches!(self.location(), Location::Folder(_)) || self.tab().view != LV_VIEW_DETAILS {
            return Vec::new();
        }
        let group = self.group_by();
        vec![
            MenuItem::new(GROUP_NONE, "None").checked(group == GroupBy::None),
            MenuItem::new(GROUP_MODIFIED, "Date modified").checked(group == GroupBy::DateModified),
        ]
    }

    fn apply_dpi(&mut self, dpi: u32) {
        self.dpi = dpi;
        let old = std::mem::replace(&mut self.fonts, [font(dpi, false), font(dpi, true)]);
        for (_, control) in self.buttons.iter().copied() {
            send(control, WM_SETFONT, self.fonts[0] as usize, 1);
        }
        for control in [
            self.list,
            self.navigation,
            self.address,
            self.search,
            self.summary,
            self.status,
            self.edit_name,
            self.edit_label,
            self.detail_body,
        ] {
            send(control, WM_SETFONT, self.fonts[0] as usize, 1);
        }
        for control in [self.title, self.detail_title, self.header] {
            send(control, WM_SETFONT, self.fonts[1] as usize, 1);
        }
        if let Some(controls) = self.settings_controls {
            for control in [controls.heading, controls.date_format_heading] {
                send(control, WM_SETFONT, self.fonts[1] as usize, 1);
            }
            for control in [controls.state, controls.description] {
                send(control, WM_SETFONT, self.fonts[0] as usize, 1);
            }
        }
        for handle in old {
            if !handle.is_null() {
                unsafe {
                    DeleteObject(handle);
                }
            }
        }
        self.measure_breadcrumbs();
        if let Some(home) = &self.home {
            home.set_style(&self.colors, self.dpi);
        }
        send(
            self.navigation,
            LB_SETITEMHEIGHT,
            0,
            scale(31, dpi) as isize,
        );
        self.fitted_columns = None;
        for (index, (kind, side)) in [(LVSIL_SMALL, 22), (LVSIL_NORMAL, self.icon_size)]
            .into_iter()
            .enumerate()
        {
            let images = list_images(dpi, side);
            send(self.list, LVM_SETIMAGELIST, kind as usize, images as isize);
            let old = std::mem::replace(&mut self.images[index], images);
            if old != 0 {
                unsafe {
                    ImageList_Destroy(old);
                }
            }
        }
    }

    fn apply_theme(&mut self) {
        self.colors = Palette::read(self.settings.theme);
        let old = std::mem::replace(&mut self.brushes, unsafe {
            [
                CreateSolidBrush(self.colors.chrome),
                CreateSolidBrush(self.colors.content),
                CreateSolidBrush(self.colors.sidebar),
                CreateSolidBrush(self.colors.field),
            ]
        });
        CONTROL_COLORS.set(Some(ControlColors {
            text: self.colors.text,
            muted: self.colors.muted,
            surfaces: [
                self.colors.chrome,
                self.colors.content,
                self.colors.sidebar,
                self.colors.field,
            ],
            brushes: self.brushes,
        }));
        set_theme(self.window, self.list, &self.colors);
        if let Some(home) = &self.home {
            home.set_style(&self.colors, self.dpi);
        }
        let theme = wide(if self.colors.dark {
            "DarkMode_Explorer"
        } else {
            "Explorer"
        });
        unsafe {
            SetWindowTheme(
                self.navigation,
                if self.colors.contrast {
                    null()
                } else {
                    theme.as_ptr()
                },
                null(),
            );
        }
        for handle in old {
            if !handle.is_null() {
                unsafe {
                    DeleteObject(handle);
                }
            }
        }
    }

    fn set_dynamic_button(&mut self, id: u16, label: &str) {
        let mut control = self.button(id);
        if control.is_null() {
            control = button(self.window, label, id);
            if control.is_null() {
                return;
            }
            send(control, WM_SETFONT, self.fonts[0] as usize, 0);
            unsafe {
                SetWindowSubclass(control, Some(button_proc), 1, 0);
            }
            self.buttons.push((id, control));
        } else {
            text(control, label);
        }
    }

    fn update_drive_buttons(&mut self) {
        if !matches!(self.location(), Location::Computer) {
            return;
        }
        for index in 0..self.visible.len().min(DRIVE_LIMIT) {
            let Some(entry) = self.entry(index) else {
                continue;
            };
            let label = if let Some(capacity) = &entry.capacity {
                format!("{}, {}", entry.label(), capacity.label())
            } else {
                entry.label()
            };
            self.set_dynamic_button(DRIVE_START + index as u16, &label);
        }
    }

    fn open_settings(&mut self) {
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| *tab.history.current() == Location::Settings)
        {
            self.switch(index);
        } else if self.tabs.len() == TAB_LIMIT {
            self.go(Location::Settings);
        } else {
            self.new_tab(Location::Settings);
        }
        let focus = if self
            .hotkey_status
            .as_ref()
            .is_some_and(|status| status.configured || status.running)
        {
            WIN_E_BETTER
        } else {
            WIN_E_WINDOWS
        };
        let focus = if self.hotkey_request.is_some() {
            self.button(OPEN_SETTINGS)
        } else {
            self.button(focus)
        };
        unsafe {
            SetFocus(focus);
        }
    }

    fn ensure_settings_controls(&mut self) {
        if self.settings_controls.is_some() {
            return;
        }
        let label = |caption| new_control(self.window, "Static", caption, 0, SS_NOPREFIX, 0);
        let controls = SettingsControls {
            heading: label("What opens when you press Win+E?"),
            description: label(
                "BetterExplorer uses a background shortcut helper that starts at sign-in.",
            ),
            state: label(""),
            date_format_heading: label("Date modified format"),
        };
        for (control, font) in [
            (controls.heading, self.fonts[1]),
            (controls.description, self.fonts[0]),
            (controls.state, self.fonts[0]),
            (controls.date_format_heading, self.fonts[1]),
        ] {
            send(control, WM_SETFONT, font as usize, 0);
        }
        self.set_dynamic_button(WIN_E_WINDOWS, "Windows File Explorer");
        self.set_dynamic_button(WIN_E_BETTER, "BetterExplorer");
        self.set_dynamic_button(DATE_FORMAT_DAY_FIRST, "DD.MM.YYYY");
        self.set_dynamic_button(DATE_FORMAT_MONTH_FIRST, "MM/DD/YYYY");
        self.settings_controls = Some(controls);
        self.update_modified_date_controls();
    }

    fn update_modified_date_controls(&self) {
        for id in [DATE_FORMAT_DAY_FIRST, DATE_FORMAT_MONTH_FIRST] {
            let control = self.button(id);
            if !control.is_null() {
                set_enabled(control, self.booted);
                unsafe {
                    InvalidateRect(control, null(), 0);
                }
            }
        }
    }

    fn request_hotkey(&mut self, request: HotkeyRequest) {
        if self.hotkey_request.is_some() {
            return;
        }
        self.hotkey_request = Some(request);
        self.hotkey_error = None;
        self.update_shortcut_controls();
        let events = Arc::clone(&self.events);
        let owner = self.window as usize;
        let work = std::thread::Builder::new()
            .name("Win+E settings".into())
            .spawn(move || {
                let change = match request {
                    HotkeyRequest::Read => Ok(()),
                    HotkeyRequest::Change(enabled) => hotkey::set_enabled(enabled),
                };
                let status = hotkey::status();
                let error = change
                    .err()
                    .map(|err| err.to_string())
                    .or_else(|| status.as_ref().err().map(|err| err.to_string()));
                publish(
                    &events,
                    owner,
                    Event::Hotkey {
                        status: status.ok(),
                        error,
                    },
                );
            });
        if let Err(error) = work {
            self.hotkey_request = None;
            self.hotkey_error = Some(error.to_string());
            self.update_shortcut_controls();
        }
    }

    fn update_shortcut_controls(&self) {
        let Some(controls) = self.settings_controls else {
            return;
        };
        let message = if let Some(error) = &self.hotkey_error {
            format!("Win+E setting: {error}")
        } else {
            match (self.hotkey_request, self.hotkey_status.as_ref()) {
                (Some(HotkeyRequest::Read), _) => "Checking the current Win+E setting…".into(),
                (Some(HotkeyRequest::Change(_)), _) => "Updating Win+E…".into(),
                (_, Some(status)) if status.another_copy => {
                    "Win+E is managed by another BetterExplorer copy.".into()
                }
                (_, Some(status)) if status.configured && status.running => {
                    "Active in this session. Windows Startup Apps controls launch at sign-in."
                        .into()
                }
                (_, Some(status)) if status.configured => {
                    "Registered for sign-in. Select BetterExplorer to start the helper now.".into()
                }
                (_, Some(status)) if status.running => {
                    "The BetterExplorer shortcut helper is running.".into()
                }
                (_, Some(_)) => "Windows File Explorer handles Win+E.".into(),
                _ => String::new(),
            }
        };
        text(controls.state, message);
        let available = self.hotkey_request.is_none();
        for id in [WIN_E_WINDOWS, WIN_E_BETTER] {
            let control = self.button(id);
            set_enabled(control, available);
            unsafe {
                InvalidateRect(control, null(), 0);
            }
        }
    }

    fn drive_cards(&self) -> bool {
        if !matches!(self.location(), Location::Computer) || self.tab().view != VIEW_CARDS {
            return false;
        }
        let bounds = rect(self.window);
        let d = |value| scale(value, self.dpi);
        let right = bounds.right - if self.settings.details { d(226) } else { 0 };
        let columns = ((right - self.sidebar_width(bounds.right) - d(12)) / d(255)).max(1) as usize;
        let lines = ((bounds.bottom - d(186)) / d(64)).max(1) as usize;
        self.visible.len() <= DRIVE_LIMIT.min(columns * lines)
    }

    fn address_bounds(&self, width: i32) -> (i32, i32, i32) {
        let d = |value| scale(value, self.dpi);
        let left = d(170);
        let search = if *self.location() == Location::Settings {
            0
        } else {
            (width / 4).clamp(d(122), d(280))
        };
        let right = width - d(10) - if search == 0 { 0 } else { search + d(8) };
        (left, right, search)
    }

    fn sidebar_width(&self, width: i32) -> i32 {
        (width / 6).clamp(scale(151, self.dpi), scale(184, self.dpi))
    }

    fn compact_toolbar(&self, width: i32) -> bool {
        width < scale(866, self.dpi)
    }

    fn layout(&mut self) {
        let bounds = rect(self.window);
        let w = bounds.right;
        let h = bounds.bottom;
        if w == 0 || h == 0 {
            return;
        }
        let dpi = self.dpi;
        let d = |value| scale(value, dpi);
        let pos = |id, x, y, width, height| move_to(self.button(id), x, y, width, height);
        let margin = d(10);
        let settings_tab = *self.location() == Location::Settings;
        let home_tab = self.home.is_some() && *self.location() == Location::Home;
        let tab_width = ((w - d(193)) / self.tabs.len() as i32 - d(2)).clamp(d(32), d(221));
        let mut x = margin;
        for offset in 0..TAB_LIMIT {
            let id = TAB_START + offset as u16;
            let control = self.button(id);
            if control.is_null() {
                continue;
            }
            let shown = offset < self.tabs.len();
            set_visible(control, shown);
            if shown {
                let close = if offset == self.active { d(25) } else { 0 };
                pos(id, x, d(RESIZE_EDGE), tab_width - close, d(27));
                if close != 0 {
                    pos(
                        CLOSE_TAB,
                        x + tab_width - close,
                        d(RESIZE_EDGE),
                        close,
                        d(27),
                    );
                }
                x += tab_width + d(2);
            }
        }
        pos(NEW_TAB, x + d(4), d(RESIZE_EDGE), d(32), d(27));
        let frame_inset = if unsafe { IsZoomed(self.window) } != 0 {
            0
        } else {
            d(RESIZE_EDGE)
        };
        for (index, id) in [FRAME_CLOSE, FRAME_MAX, FRAME_MIN].into_iter().enumerate() {
            pos(
                id,
                w - frame_inset - d(44 * (index as i32 + 1)),
                frame_inset,
                d(44),
                d(35) - frame_inset,
            );
        }

        x = margin;
        for id in [BACK, FORWARD, UP, REFRESH] {
            pos(id, x, d(43), d(36), d(30));
            x += d(39);
        }
        let (address_x, address_right, search_width) = self.address_bounds(w);
        let address_width = address_right - address_x;
        pos(PATH, address_x, d(44), d(32), d(28));
        move_to(
            self.address,
            address_x + d(35),
            d(46),
            address_width - d(42),
            d(22),
        );
        set_visible(self.address, self.editing_address && !settings_tab);
        set_visible(self.search, !settings_tab);
        set_visible(self.button(REFRESH), !settings_tab);
        move_to(
            self.search,
            w - search_width - margin + d(10),
            d(49),
            search_width - d(39),
            d(20),
        );
        let compact = self.compact_toolbar(w);
        let narrow = w < d(650);
        x = margin;
        let mut right = w - margin;
        for id in [OPEN_SETTINGS, DETAILS] {
            let width = d(if id == OPEN_SETTINGS {
                103
            } else if compact {
                36
            } else {
                88
            });
            right -= width;
            pos(id, right, d(83), width, d(31));
            right -= d(4);
            set_visible(self.button(id), id == OPEN_SETTINGS || !settings_tab);
        }
        for &(id, _, width) in COMMANDS {
            if matches!(id, OPEN_SETTINGS | DETAILS) {
                continue;
            }
            let shown = !settings_tab
                && !(compact && matches!(id, RENAME | DELETE | FILTER))
                && !(narrow && matches!(id, CUT | COPY | PASTE));
            set_visible(self.button(id), shown);
            if shown {
                if matches!(id, CUT | SORT | MORE) {
                    x += d(5);
                }
                pos(id, x, d(83), d(width), d(31));
                x += d(width + 2);
            }
        }

        let sidebar_width = self.sidebar_width(w);
        let status_y = if settings_tab { h } else { h - d(27) };
        move_to(
            self.navigation,
            d(RESIZE_EDGE),
            d(126),
            sidebar_width - d(RESIZE_EDGE + 5),
            status_y - d(130),
        );
        let listing_right = if self.settings.details && !settings_tab {
            w - d(226)
        } else {
            w
        };
        let content_x = sidebar_width + d(12);
        let list_width = listing_right - content_x - d(6);
        let drive_mode = self.drive_cards();
        let section = !matches!(self.location(), Location::Folder(_));
        let list_y = if section { 155 } else { 123 };
        set_visible(self.title, section && !home_tab);
        set_visible(self.summary, section && !settings_tab && !home_tab);
        if let Some(home) = &self.home {
            home.set_visible(home_tab);
            if home_tab {
                home.set_bounds(
                    content_x - d(6),
                    d(123),
                    listing_right - content_x + d(2),
                    status_y - d(123),
                );
            }
        }
        move_to(
            self.title,
            content_x,
            d(131),
            (listing_right - content_x - d(160)).max(d(100)),
            d(23),
        );
        move_to(self.summary, listing_right - d(116), d(133), d(100), d(22));
        set_visible(self.list, !home_tab && !drive_mode && !settings_tab);
        move_to(
            self.list,
            content_x - d(3),
            d(list_y),
            list_width,
            status_y
                - d(if self.edit.is_some() {
                    list_y + 48
                } else {
                    list_y
                }),
        );
        let columns = ((listing_right - content_x) / d(255)).max(1) as usize;
        for index in 0..DRIVE_LIMIT {
            let id = DRIVE_START + index as u16;
            let shown = drive_mode && index < self.visible.len();
            let control = self.button(id);
            if control.is_null() {
                continue;
            }
            set_visible(control, shown);
            if shown {
                let col = (index % columns) as i32;
                let row = (index / columns) as i32;
                pos(
                    id,
                    content_x + col * d(255),
                    d(155) + row * d(64),
                    d(245).min(listing_right - content_x - d(5)),
                    d(57),
                );
            }
        }
        move_to(
            self.detail_title,
            listing_right + d(14),
            d(133),
            d(195),
            d(40),
        );
        let detail_top = if matches!(self.preview, Some(Content::Bitmap(_))) {
            320
        } else {
            181
        };
        move_to(
            self.detail_body,
            listing_right + d(14),
            d(detail_top),
            d(196),
            status_y - d(detail_top + 11),
        );
        set_visible(self.detail_title, self.settings.details && !settings_tab);
        set_visible(self.detail_body, self.settings.details && !settings_tab);
        set_visible(self.status, !settings_tab);
        set_visible(self.button(QUICK_DETAILS), !settings_tab && !home_tab);
        set_visible(self.button(QUICK_ICONS), !settings_tab && !home_tab);
        if let Some(controls) = self.settings_controls {
            for control in [
                controls.heading,
                controls.description,
                controls.state,
                self.button(WIN_E_WINDOWS),
                self.button(WIN_E_BETTER),
                controls.date_format_heading,
                self.button(DATE_FORMAT_DAY_FIRST),
                self.button(DATE_FORMAT_MONTH_FIRST),
            ] {
                set_visible(control, settings_tab);
            }
            if settings_tab {
                let x = content_x + d(13);
                let span = (w - x - d(20)).min(d(690));
                move_to(controls.heading, x, d(184), span, d(27));
                let gap = d(10);
                let windows_width = d(221).min((span - gap) * 11 / 20);
                pos(WIN_E_WINDOWS, x, d(218), windows_width, d(36));
                pos(
                    WIN_E_BETTER,
                    x + windows_width + gap,
                    d(218),
                    d(188).min(span - windows_width - gap),
                    d(36),
                );
                move_to(controls.state, x, d(266), span, d(49));
                move_to(controls.description, x, d(319), span, d(48));
                move_to(controls.date_format_heading, x, d(373), span, d(27));
                let format_width = d(136).min((span - gap) / 2);
                pos(DATE_FORMAT_DAY_FIRST, x, d(402), format_width, d(36));
                pos(
                    DATE_FORMAT_MONTH_FIRST,
                    x + format_width + gap,
                    d(402),
                    format_width,
                    d(36),
                );
            }
        }
        move_to(self.status, margin, status_y + d(5), w - d(198), d(20));
        pos(
            QUICK_DETAILS,
            w - frame_inset - d(61),
            status_y + d(1),
            d(30),
            d(25) - frame_inset,
        );
        pos(
            QUICK_ICONS,
            w - frame_inset - d(30),
            status_y + d(1),
            d(30),
            d(25) - frame_inset,
        );
        pos(STOP_SEARCH, w - d(192), status_y + d(1), d(115), d(24));
        set_visible(
            self.button(STOP_SEARCH),
            self.scanning && matches!(self.location(), Location::Search { .. }),
        );
        for control in [
            self.edit_label,
            self.edit_name,
            self.button(ACCEPT),
            self.button(CANCEL),
        ] {
            set_visible(control, self.edit.is_some());
        }
        if self.edit.is_some() {
            let y = status_y - d(39);
            move_to(self.edit_label, content_x, y + d(5), d(154), d(24));
            move_to(
                self.edit_name,
                content_x + d(155),
                y,
                w - content_x - d(315),
                d(27),
            );
            pos(ACCEPT, w - d(147), y, d(63), d(28));
            pos(CANCEL, w - d(76), y, d(63), d(28));
        }
        if self.tab().view == LV_VIEW_DETAILS || self.tab().view == VIEW_CARDS {
            let available = (list_width - d(19)).max(d(342));
            let modified = d(143).min(available / 3);
            let kind = if self.recycling() {
                d(265).min(available / 3)
            } else {
                d(104).min(available / 5)
            };
            let size = d(78).min(available / 6);
            let widths = [
                (available - modified - kind - size).min(d(COLUMNS[0].1)),
                modified,
                kind,
                size,
            ];
            if self.fitted_columns != Some(widths) {
                for (index, width) in widths.into_iter().enumerate() {
                    send(self.list, LVM_SETCOLUMNWIDTH, index, width as isize);
                }
                self.fitted_columns = Some(widths);
            }
        }
        let mut crumb_x = address_x + d(33);
        let placements = if self.editing_address {
            Vec::new()
        } else {
            breadcrumb::fit(
                &self.crumb_widths,
                address_right - crumb_x - d(4),
                d(32),
                d(72),
            )
        };
        let placements = if placements.len() > CRUMB_LIMIT {
            let tail = &placements[placements.len() - (CRUMB_LIMIT - 1)..];
            let mut limited = Vec::with_capacity(CRUMB_LIMIT);
            limited.push(breadcrumb::Placement {
                index: None,
                width: d(32),
            });
            limited.extend(tail.iter().map(|placement| breadcrumb::Placement {
                index: placement.index,
                width: placement.width,
            }));
            limited
        } else {
            placements
        };
        let new_slots: Vec<_> = placements
            .iter()
            .take(CRUMB_LIMIT)
            .map(|placement| {
                placement
                    .index
                    .map(|index| self.crumb_segments[index].1.clone())
            })
            .collect();
        for (offset, placement) in placements.iter().take(CRUMB_LIMIT).enumerate() {
            let id = CRUMB_START + offset as u16;
            if self.crumb_slots.get(offset) != new_slots.get(offset) {
                let label = placement
                    .index
                    .map_or("\u{2026}", |index| &self.crumb_segments[index].0)
                    .to_owned();
                self.set_dynamic_button(id, &label);
            }
            move_to(self.button(id), crumb_x, d(44), placement.width, d(28));
            crumb_x += placement.width;
        }
        self.crumb_slots = new_slots;
        for offset in 0..CRUMB_LIMIT {
            let control = self.button(CRUMB_START + offset as u16);
            if !control.is_null() {
                set_visible(control, offset < self.crumb_slots.len());
            }
        }
        unsafe {
            InvalidateRect(self.window, null(), 0);
            InvalidateRect(self.header, null(), 0);
            RedrawWindow(
                self.window,
                null(),
                null_mut(),
                RDW_INVALIDATE | RDW_ALLCHILDREN,
            );
        }
    }

    fn address_focus(&mut self, editing: bool) {
        if editing && *self.location() == Location::Settings {
            return;
        }
        if self.editing_address != editing {
            self.editing_address = editing;
            if !editing {
                let value = self
                    .location_dir()
                    .map(|p| p.into_os_string())
                    .unwrap_or_else(|| self.location().caption().into());
                text(self.address, value);
            }
            self.layout();
        }
        if editing {
            unsafe {
                SetFocus(self.address);
            }
            send(self.address, EM_SETSEL, 0, -1);
        }
    }

    fn paint(&self) {
        let bounds = rect(self.window);
        let d = |value| scale(value, self.dpi);
        let footer_height = if *self.location() == Location::Settings {
            0
        } else {
            d(28)
        };
        let mut paint = unsafe { zeroed() };
        let dc = unsafe { BeginPaint(self.window, &mut paint) };
        fill(dc, &bounds, self.colors.content);
        fill(
            dc,
            &RECT {
                bottom: d(122),
                ..bounds
            },
            self.colors.chrome,
        );
        fill(
            dc,
            &RECT {
                top: d(122),
                right: self.sidebar_width(bounds.right),
                bottom: bounds.bottom - footer_height,
                ..bounds
            },
            self.colors.sidebar,
        );
        fill(
            dc,
            &RECT {
                top: bounds.bottom - footer_height,
                ..bounds
            },
            self.colors.chrome,
        );
        fill(
            dc,
            &RECT {
                bottom: d(79),
                ..bounds
            },
            self.colors.title,
        );
        let (left, right, search_width) = self.address_bounds(bounds.right);
        rounded_fill(
            dc,
            &RECT {
                left,
                right,
                top: d(42),
                bottom: d(75),
            },
            d(6),
            self.colors.field,
        );
        if search_width != 0 {
            let search_right = bounds.right - d(10);
            rounded_fill(
                dc,
                &RECT {
                    left: right + d(8),
                    top: d(42),
                    right: search_right,
                    bottom: d(75),
                },
                d(6),
                self.colors.field,
            );
            draw_glyph(
                dc,
                Glyph::Search,
                search_right - d(24),
                d(51),
                d(15),
                self.colors.muted,
            );
        }
        for y in [37, 78, 121] {
            fill(
                dc,
                &RECT {
                    top: d(y),
                    bottom: d(y) + 1,
                    ..bounds
                },
                self.colors.edge,
            );
        }
        let right = if self.settings.details && *self.location() != Location::Settings {
            bounds.right - d(226)
        } else {
            0
        };
        for x in [self.sidebar_width(bounds.right), right] {
            if x != 0 {
                fill(
                    dc,
                    &RECT {
                        top: d(122),
                        left: x,
                        right: x + 1,
                        bottom: bounds.bottom - footer_height,
                    },
                    self.colors.edge,
                );
            }
        }
        if right > 0 {
            if let Some(Content::Bitmap(image)) = &self.preview {
                image.draw(
                    dc,
                    RECT {
                        left: right + d(14),
                        right: bounds.right - d(14),
                        top: d(172),
                        bottom: d(311),
                    },
                );
            }
        }
        unsafe {
            EndPaint(self.window, &paint);
        }
    }

    fn update_home(&mut self) {
        let Some(home) = &self.home else {
            return;
        };
        if *self.location() != Location::Home {
            return;
        }
        let mut quick = Vec::new();
        self.home_quick_paths.clear();
        for (path, label) in &self.places {
            let (icon, title) = match *label {
                "Desktop" => (Icon::Desktop, (*label).into()),
                "Downloads" => (Icon::Downloads, (*label).into()),
                "Documents" => (Icon::Documents, (*label).into()),
                "Pictures" => (Icon::Pictures, (*label).into()),
                "Music" => (Icon::Music, (*label).into()),
                "Videos" => (Icon::Videos, (*label).into()),
                _ => (
                    Icon::Folder,
                    path.file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                        .into_owned(),
                ),
            };
            quick.push(Item {
                title,
                subtitle: "Local folder".into(),
                id: HOME_QUICK_START + self.home_quick_paths.len() as u16,
                icon,
            });
            self.home_quick_paths.push(path.clone());
        }
        for path in self.settings.pins.iter().take(75) {
            if self.home_quick_paths.contains(path) {
                continue;
            }
            quick.push(Item {
                title: path
                    .file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
                    .into_owned(),
                subtitle: path.parent().unwrap_or(path).display().to_string(),
                id: HOME_QUICK_START + self.home_quick_paths.len() as u16,
                icon: Icon::Folder,
            });
            self.home_quick_paths.push(path.clone());
        }
        let activity_paths = match self.home_activity {
            Activity::Recent => &self.settings.recent,
            Activity::Favorites => &self.settings.favorites,
        };
        let filter = self.home_filter;
        let paths: Vec<_> = activity_paths
            .iter()
            .filter(|path| filter.matches(path, false))
            .take(75)
            .cloned()
            .collect();
        let activity = paths
            .iter()
            .enumerate()
            .map(|(index, path)| Item {
                title: (if self.settings.hide_extensions {
                    path.file_stem()
                } else {
                    path.file_name()
                })
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned(),
                subtitle: path.parent().unwrap_or(path).display().to_string(),
                id: HOME_ACTIVITY_START + index as u16,
                icon: Icon::File,
            })
            .collect();
        self.home_activity_paths = paths;
        home.set_compact(self.settings.home_compact);
        home.set_content(
            quick,
            activity,
            HOME_RECENT,
            HOME_FAVORITES,
            self.home_activity,
        );
    }

    fn update_sidebar(&mut self) {
        let mut rows = vec![NavRow {
            label: "Home".into(),
            target: Some(Location::Home),
            command: HOME,
            icon: 5,
            pinned: false,
        }];
        for (index, name) in PLACES.iter().enumerate() {
            if let Some((path, _)) = self.places.iter().find(|(_, label)| label == name) {
                rows.push(NavRow {
                    label: (*name).into(),
                    target: Some(Location::Folder(path.clone())),
                    command: 0,
                    icon: 6 + index as u16,
                    pinned: true,
                });
            }
        }
        for path in &self.settings.pins {
            let target = Location::Folder(path.clone());
            if !rows.iter().any(|row| row.target.as_ref() == Some(&target)) {
                rows.push(NavRow {
                    label: target.caption(),
                    target: Some(target),
                    command: 0,
                    icon: 2,
                    pinned: true,
                });
            }
        }
        for (label, command, icon, target) in [
            ("This PC", COMPUTER, 4, Some(Location::Computer)),
            (
                "Network",
                NETWORK,
                13,
                Some(Location::namespace(Namespace::Network)),
            ),
            (
                "Recycle Bin",
                RECYCLE,
                14,
                Some(Location::namespace(Namespace::RecycleBin)),
            ),
            (
                "Libraries",
                LIBRARIES,
                15,
                Some(Location::namespace(Namespace::Libraries)),
            ),
        ] {
            rows.push(NavRow {
                label: label.into(),
                command,
                icon,
                target,
                pinned: false,
            });
        }
        if self.nav_rows != rows {
            let top = send(self.navigation, LB_GETTOPINDEX, 0, 0);
            send(self.navigation, WM_SETREDRAW, 0, 0);
            send(self.navigation, LB_RESETCONTENT, 0, 0);
            self.nav_rows = rows;
            for row in &self.nav_rows {
                let value = wide(&row.label);
                send(self.navigation, LB_ADDSTRING, 0, value.as_ptr() as LPARAM);
            }
            send(self.navigation, LB_SETTOPINDEX, top.max(0) as usize, 0);
            send(self.navigation, WM_SETREDRAW, 1, 0);
        }
        unsafe {
            InvalidateRect(self.navigation, null(), 1);
        }
    }

    fn select_sidebar(&mut self) {
        let selected = send(self.navigation, LB_GETCURSEL, 0, 0);
        let Some(row) = usize::try_from(selected)
            .ok()
            .and_then(|index| self.nav_rows.get(index))
        else {
            return;
        };
        if let Some(location) = row.target.clone() {
            if self.location() != &location {
                self.go(location);
            }
        } else {
            self.command(row.command);
        }
        unsafe {
            InvalidateRect(self.navigation, null(), 0);
        }
    }

    fn measure_menu(&self, data: &mut MEASUREITEMSTRUCT) {
        let Some(item) = self.popup_items.get(data.itemData) else {
            return;
        };
        let d = |value| scale(value, self.dpi);
        if item.id == 0 {
            data.itemHeight = d(9) as u32;
            data.itemWidth = 0;
            return;
        }
        data.itemHeight = d(31) as u32;
        let dc = unsafe { GetDC(self.window) };
        if dc.is_null() {
            data.itemWidth = d(205) as u32;
            return;
        }
        let (label, shortcut) = item.label.split_once('\t').unwrap_or((&item.label, ""));
        let width = |value: &str| {
            let text = wide(value.replace("&&", "&"));
            let mut extent = unsafe { zeroed() };
            unsafe {
                GetTextExtentPoint32W(dc, text.as_ptr(), text.len() as i32 - 1, &mut extent);
            }
            extent.cx
        };
        let previous = unsafe { SelectObject(dc, self.fonts[0]) };
        let shortcut = if shortcut.is_empty() {
            0
        } else {
            width(shortcut) + d(23)
        };
        data.itemWidth = (width(label) + d(60) + shortcut).clamp(d(190), d(482)) as u32;
        unsafe {
            SelectObject(dc, previous);
            ReleaseDC(self.window, dc);
        }
    }

    fn draw_menu(&self, draw: &DRAWITEMSTRUCT) {
        let Some(item) = self.popup_items.get(draw.itemData) else {
            return;
        };
        let r = draw.rcItem;
        let dc = draw.hDC;
        let d = |value| scale(value, self.dpi);
        fill(dc, &r, self.colors.chrome);
        if item.id == 0 {
            let y = (r.top + r.bottom) / 2;
            fill(
                dc,
                &RECT {
                    left: r.left + d(10),
                    right: r.right - d(10),
                    top: y,
                    bottom: y + 1,
                },
                self.colors.edge,
            );
            return;
        }
        if draw.itemState & ODS_SELECTED != 0 && item.enabled {
            rounded_fill(
                dc,
                &RECT {
                    left: r.left + d(3),
                    right: r.right - d(3),
                    top: r.top + d(1),
                    bottom: r.bottom - d(1),
                },
                d(4),
                self.colors.active,
            );
        }
        let (label, shortcut) = item.label.split_once('\t').unwrap_or((&item.label, ""));
        let color = if item.enabled {
            self.colors.text
        } else {
            self.colors.muted
        };
        let flags = DT_SINGLELINE | DT_VCENTER;
        let scale_icon = d(16);
        let icon_y = (r.top + r.bottom - scale_icon) / 2;
        if let Some(glyph) = if item.checked {
            Some(Glyph::Check)
        } else {
            item.glyph()
        } {
            let icon_color = if !item.enabled {
                self.colors.muted
            } else if matches!(
                item.id,
                VIEW | SORT | GROUP_SUBMENU | NEW | RENAME | REFRESH | RECYCLE_RESTORE
            ) || item.checked
            {
                self.colors.accent
            } else {
                self.colors.text
            };
            draw_glyph(dc, glyph, r.left + d(11), icon_y, scale_icon, icon_color);
        }
        if item.submenu && self.colors.dark {
            let middle = (r.top + r.bottom) / 2;
            rounded_fill(
                dc,
                &RECT {
                    left: r.right - d(16),
                    right: r.right - d(5),
                    top: middle - d(6),
                    bottom: middle + d(6),
                },
                d(3),
                rgb(131, 128, 145),
            );
        }
        let text = RECT {
            left: r.left + d(38),
            right: r.right - d(15),
            ..r
        };
        text_at(
            dc,
            &label.replace("&&", "&"),
            RECT {
                right: text.right - if shortcut.is_empty() { 0 } else { d(87) },
                ..text
            },
            self.fonts[0],
            color,
            flags,
        );
        if !shortcut.is_empty() {
            text_at(
                dc,
                shortcut,
                text,
                self.fonts[0],
                self.colors.muted,
                flags | DT_RIGHT,
            );
        }
    }

    fn draw_sidebar(&self, draw: &DRAWITEMSTRUCT) {
        let dc = draw.hDC;
        let bounds = draw.rcItem;
        fill(dc, &bounds, self.colors.sidebar);
        let Some(row) = self.nav_rows.get(draw.itemID as usize) else {
            return;
        };
        let d = |value| scale(value, self.dpi);
        let active = row.target.as_ref() == Some(self.location());
        if active || draw.itemState & ODS_SELECTED != 0 && unsafe { GetFocus() } == self.navigation
        {
            fill(
                dc,
                &RECT {
                    top: bounds.top + d(1),
                    bottom: bounds.bottom - d(1),
                    ..bounds
                },
                self.colors.active,
            );
        }
        let side = d(18);
        let icon_x = bounds.left + d(8);
        let icon_y = bounds.top + (bounds.bottom - bounds.top - side) / 2;
        if self.colors.contrast || !draw_resource_icon(dc, row.icon, icon_x, icon_y, side) {
            let glyph = match row.target.as_ref() {
                Some(Location::Home) => Glyph::Home,
                Some(Location::Computer) => Glyph::Drive,
                Some(Location::Namespace {
                    kind: Namespace::Network,
                    ..
                }) => Glyph::Network,
                Some(Location::Namespace {
                    kind: Namespace::RecycleBin,
                    ..
                }) => Glyph::Recycle,
                Some(Location::Namespace {
                    kind: Namespace::Libraries,
                    ..
                }) => Glyph::Library,
                _ => Glyph::Folder,
            };
            draw_glyph(dc, glyph, icon_x, icon_y, side, self.colors.muted);
        }
        if row.pinned {
            draw_glyph(
                dc,
                Glyph::Pin,
                bounds.right - d(17),
                icon_y + d(4),
                d(10),
                self.colors.muted,
            );
        }
        text_at(
            dc,
            &row.label,
            RECT {
                left: bounds.left + d(30),
                right: bounds.right - d(if row.pinned { 21 } else { 5 }),
                ..bounds
            },
            self.fonts[0],
            self.colors.text,
            DT_SINGLELINE | DT_VCENTER | DT_LEFT,
        );
        if draw.itemState & ODS_FOCUS != 0 && unsafe { GetFocus() } == self.navigation {
            unsafe {
                DrawFocusRect(dc, &bounds);
            }
        }
    }

    fn update_navigation(&mut self) {
        let top = send(self.navigation, LB_GETTOPINDEX, 0, 0);
        let selection = self
            .nav_rows
            .iter()
            .position(|row| row.target.as_ref() == Some(self.location()));
        send(
            self.navigation,
            LB_SETCURSEL,
            selection.unwrap_or(usize::MAX),
            0,
        );
        send(self.navigation, LB_SETTOPINDEX, top.max(0) as usize, 0);
        unsafe {
            InvalidateRect(self.navigation, null(), 0);
        }
        self.update_home();
        self.sync_column_labels();
        for offset in 0..self.tabs.len() {
            let label = self.tabs[offset].history.current().caption();
            self.set_dynamic_button(TAB_START + offset as u16, &label);
        }
        let caption = self.location().caption();
        text(self.window, format!("{caption} | BetterExplorer"));
        text(
            self.title,
            if matches!(self.location(), Location::Computer) {
                "Devices and drives"
            } else {
                &caption
            },
        );
        text(
            self.address,
            self.location_dir()
                .map(|path| path.into_os_string())
                .unwrap_or_else(|| OsString::from(caption)),
        );
        text(
            self.search,
            match self.location() {
                Location::Search { query, .. } => query,
                _ => "",
            },
        );
        self.crumbs = self
            .location_dir()
            .map(|path| {
                path.ancestors()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .take((u16::MAX - BREADCRUMB_START) as usize)
                    .map(Path::to_path_buf)
                    .collect()
            })
            .unwrap_or_default();
        self.editing_address = false;
        self.crumb_slots.clear();
        self.crumb_segments = breadcrumb::segments(
            self.location(),
            std::env::var("SystemDrive").ok().as_deref(),
        );
        self.measure_breadcrumbs();
        let hint = wide(if self.location_dir().is_some() {
            format!("Search {}", self.location().caption())
        } else {
            "Open a folder to search".into()
        });
        send(self.search, EM_SETCUEBANNER, 1, hint.as_ptr() as isize);
        set_enabled(self.button(BACK), self.tab().history.can_back());
        set_enabled(self.button(FORWARD), self.tab().history.can_forward());
        set_enabled(
            self.button(UP),
            self.location_dir().is_some()
                || matches!(self.location(), Location::Namespace { child: Some(_), .. }),
        );
        set_enabled(self.button(NEW), self.dest().is_some());
        set_enabled(self.button(SORT), *self.location() != Location::Home);
        set_enabled(self.button(PASTE), self.dest().is_some());
        if self.settings_controls.is_some() {
            for &(id, control) in &self.buttons {
                if id == HOME
                    || id == COMPUTER
                    || id == OPEN_SETTINGS
                    || (TAB_START..TAB_START + TAB_LIMIT as u16).contains(&id)
                    || (CRUMB_START..CRUMB_START + CRUMB_LIMIT as u16).contains(&id)
                {
                    unsafe {
                        InvalidateRect(control, null(), 0);
                    }
                }
            }
        }
    }

    fn measure_breadcrumbs(&mut self) {
        let dc = unsafe { GetDC(self.window) };
        if dc.is_null() {
            return;
        }
        let font = unsafe { SelectObject(dc, self.fonts[0]) };
        self.crumb_widths = self
            .crumb_segments
            .iter()
            .enumerate()
            .map(|(index, (label, _))| {
                let label = wide(label);
                let mut size = unsafe { zeroed() };
                unsafe {
                    GetTextExtentPoint32W(dc, label.as_ptr(), label.len() as i32 - 1, &mut size);
                }
                size.cx + scale(if index == 0 { 20 } else { 40 }, self.dpi)
            })
            .collect();
        unsafe {
            SelectObject(dc, font);
            ReleaseDC(self.window, dc);
        }
    }

    fn apply_view(&mut self, view: u32) {
        self.tab_mut().view = view;
        let side = match view {
            VIEW_SMALL => 30,
            VIEW_LARGE => 78,
            VIEW_XLARGE => 112,
            _ => 52,
        };
        if side != self.icon_size {
            self.icon_size = side;
            let images = list_images(self.dpi, side);
            send(self.list, LVM_SETIMAGELIST, LVSIL_NORMAL as usize, images);
            let old = std::mem::replace(&mut self.images[1], images);
            if old != 0 {
                unsafe {
                    ImageList_Destroy(old);
                }
            }
        }
        let native = match view {
            VIEW_SMALL | VIEW_LARGE | VIEW_XLARGE => LV_VIEW_ICON,
            VIEW_CARDS => LV_VIEW_DETAILS,
            _ => view,
        };
        send(self.list, LVM_SETVIEW, native as usize, 0);
        if native == LV_VIEW_ICON {
            let width = scale(side + 34, self.dpi);
            let height = scale(side + 38, self.dpi);
            send(
                self.list,
                LVM_SETICONSPACING,
                0,
                ((height << 16) | width) as isize,
            );
        }
    }

    fn update_view(&mut self) {
        let default = if matches!(self.location(), Location::Computer) {
            VIEW_CARDS
        } else {
            LV_VIEW_DETAILS
        };
        let view = self
            .dest()
            .and_then(|path| self.settings.views.get(&path).copied())
            .filter(|view| {
                matches!(
                    *view,
                    LV_VIEW_DETAILS
                        | LV_VIEW_LIST
                        | LV_VIEW_ICON
                        | VIEW_SMALL
                        | VIEW_LARGE
                        | VIEW_XLARGE
                )
            })
            .unwrap_or(default);
        self.apply_view(view);
    }

    fn go(&mut self, location: Location) {
        self.tab_mut().history.open(location);
        self.update_view();
        self.refresh();
        self.schedule_save();
    }

    fn refresh(&mut self) {
        self.reload(false);
    }

    fn reload(&mut self, keep_rows: bool) {
        if keep_rows && self.edit.is_some() {
            unsafe {
                SetTimer(self.window, REFRESH_TIMER, 250, None);
            }
            return;
        }
        self.keep_rows = keep_rows;
        if !keep_rows {
            self.clear_selection();
            self.update_preview();
            self.rows.clear();
            self.visible.clear();
            self.group_headers.clear();
            unsafe { KillTimer(self.window, GROUP_TIMER) };
            self.set_count();
            self.edit = None;
            self.update_navigation();
        }
        self.error = None;
        self.note = None;
        let settings_tab = *self.location() == Location::Settings;
        self.scanning = !settings_tab;
        if settings_tab {
            self.ensure_settings_controls();
        }
        self.layout();
        self.status();
        self.request_scan();
    }

    fn request_scan(&mut self) {
        self.id = self.id.wrapping_add(1);
        if *self.location() == Location::Settings {
            if let Some(scanner) = &self.scanner {
                scanner.cancel(self.id);
            }
            if let Some(watcher) = &self.watcher {
                watcher.watch(self.id, None);
            }
            self.request_hotkey(HotkeyRequest::Read);
            return;
        }
        if let Some(scanner) = &self.scanner {
            scanner.request(Request {
                id: self.id,
                location: self.location().clone(),
                hidden: self.settings.hidden,
                sort: self.sort(),
                pins: self.settings.pins.clone(),
            });
        }
        if let Some(watcher) = &self.watcher {
            watcher.watch(self.id, self.dest());
        }
        unsafe {
            KillTimer(self.window, REFRESH_TIMER);
        }
    }

    fn set_count(&self) {
        send(
            self.list,
            LVM_SETITEMCOUNT,
            self.visible.len(),
            (LVSICF_NOINVALIDATEALL | LVSICF_NOSCROLL) as isize,
        );
        unsafe {
            InvalidateRect(self.list, null(), 0);
        }
    }

    fn clear_selection(&self) {
        let state = LVITEMW {
            stateMask: LVIS_SELECTED | LVIS_FOCUSED,
            ..unsafe { zeroed() }
        };
        send(
            self.list,
            LVM_SETITEMSTATE,
            usize::MAX,
            &state as *const _ as isize,
        );
    }

    fn events(&mut self) {
        let events = std::mem::take(&mut *self.events.lock().unwrap());
        for event in events {
            match event {
                Event::Scan(ScanEvent::Batch(id, rows)) if id == self.id && !self.keep_rows => {
                    let filter = self.tab().filter;
                    for row in rows {
                        if filter.shows(&row) {
                            self.visible.push(self.rows.len());
                        }
                        self.rows.push(row);
                    }
                    self.set_count();
                    self.status();
                }
                Event::Scan(ScanEvent::Done(id, rows, error)) if id == self.id => {
                    let selection = self.list_selection();
                    self.clear_selection();
                    self.rows = rows;
                    self.error = error;
                    self.scanning = false;
                    self.keep_rows = false;
                    self.rebuild_visible();
                    self.update_drive_buttons();
                    self.restore_list_selection(selection);
                    self.update_preview();
                    self.status();
                    self.layout();
                }
                Event::Scan(_) => (),
                Event::Preview(id, content) if id == self.preview_id => {
                    self.preview = content;
                    self.layout();
                    self.status();
                }
                Event::Preview(..) => (),
                Event::Boot(settings, places) => {
                    let listing_changed = match self.location() {
                        Location::Home => self.settings.pins != settings.pins,
                        Location::Folder(path) => {
                            self.settings.hidden != settings.hidden
                                || self.settings.folder_order(path) != settings.folder_order(path)
                        }
                        Location::Search { .. } => self.settings.hidden != settings.hidden,
                        Location::Computer | Location::Settings | Location::Namespace { .. } => {
                            false
                        }
                    };
                    let view_changed = self.dest().is_some_and(|path| {
                        self.settings.views.get(&path) != settings.views.get(&path)
                    });
                    let details_changed = self.settings.details != settings.details;
                    let date_format_changed =
                        self.settings.modified_date_format != settings.modified_date_format;
                    self.places = places;
                    self.settings = settings;
                    self.update_sidebar();
                    self.update_home();
                    let restore = self.launch_path.is_none()
                        && !std::env::args_os().any(|arg| arg == "--new-window")
                        && self.id == 1
                        && self.tabs.len() == 1
                        && matches!(self.location(), Location::Home)
                        && !self.editing_address
                        && self.settings.restore_tabs
                        && !self.settings.tabs.is_empty();
                    self.booted = true;
                    self.apply_theme();
                    self.update_modified_date_controls();
                    if date_format_changed {
                        unsafe {
                            InvalidateRect(self.list, null(), 0);
                        }
                    }
                    if restore {
                        self.tabs = self
                            .settings
                            .tabs
                            .iter()
                            .take(TAB_LIMIT)
                            .cloned()
                            .map(Tab::new)
                            .collect();
                        self.active = self.settings.active.min(self.tabs.len() - 1);
                        self.update_view();
                        self.refresh();
                    } else {
                        if view_changed {
                            self.update_view();
                            self.rebuild_grouping();
                        }
                        if listing_changed {
                            self.reload(true);
                        } else {
                            if view_changed || details_changed {
                                self.layout();
                            }
                            self.update_preview();
                            self.status();
                        }
                    }
                }
                Event::Hotkey { status, error } => {
                    self.hotkey_request = None;
                    self.hotkey_status = status;
                    self.hotkey_error = error;
                    self.update_shortcut_controls();
                }
                Event::SettingsError(error) => {
                    self.note = Some(error);
                    self.status();
                }
                Event::Operation {
                    result,
                    refresh,
                    opened,
                } => {
                    if let Some(path) = opened {
                        self.settings.record_recent(path);
                        self.update_home();
                        self.schedule_save();
                    }
                    self.busy = self.busy.saturating_sub(1);
                    if refresh {
                        self.reload(true);
                    }
                    match result {
                        Ok(message) if !message.is_empty() => self.note = Some(message),
                        Err(error) => self.note = Some(error),
                        _ if refresh => self.note = Some("File operation finished.".into()),
                        _ => (),
                    }
                    self.status();
                }
            }
        }
    }

    fn rebuild_visible(&mut self) {
        let filter = self.tab().filter;
        self.visible.clear();
        self.group_headers.clear();
        unsafe { KillTimer(self.window, GROUP_TIMER) };
        let rows = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| filter.shows(row));
        let groups = if !self.scanning && self.group_by() == GroupBy::DateModified {
            DateGroups::current()
        } else {
            None
        };
        if let Some(groups) = groups {
            unsafe { SetTimer(self.window, GROUP_TIMER, groups.next_day_ms(), None) };
            let mut buckets: [_; DateGroup::ALL.len()] = std::array::from_fn(|_| Vec::new());
            for (index, row) in rows {
                buckets[groups.group(row.modified) as usize].push(index);
            }
            let mut order = DateGroup::ALL;
            let sort = self.sort();
            if sort.column == Column::Modified && !sort.descending {
                order[..DateGroup::Unknown as usize].reverse();
            }
            for group in order {
                let bucket = &buckets[group as usize];
                if !bucket.is_empty() {
                    self.group_headers.push((self.visible.len(), group));
                    self.visible.push(usize::MAX);
                    self.visible.extend_from_slice(bucket);
                }
            }
        } else {
            self.visible.extend(rows.map(|(index, _)| index));
        }
        self.set_count();
    }

    fn rebuild_grouping(&mut self) {
        let selection = self.list_selection();
        self.clear_selection();
        self.rebuild_visible();
        self.restore_list_selection(selection);
    }

    fn group_header(&self, index: usize) -> Option<DateGroup> {
        self.group_headers
            .binary_search_by_key(&index, |&(row, _)| row)
            .ok()
            .map(|header| self.group_headers[header].1)
    }

    fn clear_group_selection(&self) {
        if self.group_headers.is_empty() {
            return;
        }
        let state = LVITEMW {
            stateMask: LVIS_SELECTED | LVIS_FOCUSED,
            ..unsafe { zeroed() }
        };
        for &(index, _) in &self.group_headers {
            if send(self.list, LVM_GETITEMSTATE, index, state.stateMask as isize) != 0 {
                send(
                    self.list,
                    LVM_SETITEMSTATE,
                    index,
                    &state as *const _ as LPARAM,
                );
            }
        }
        if self
            .group_header(send(self.list, LVM_GETSELECTIONMARK, 0, 0) as usize)
            .is_some()
        {
            let focused = send(
                self.list,
                LVM_GETNEXTITEM,
                usize::MAX,
                LVNI_FOCUSED as isize,
            );
            send(self.list, LVM_SETSELECTIONMARK, 0, focused);
        }
    }

    fn visible_count(&self) -> usize {
        self.visible.len() - self.group_headers.len()
    }

    fn entry(&self, index: usize) -> Option<&Entry> {
        self.visible
            .get(index)
            .and_then(|&row| self.rows.get(row))
            .map(Arc::as_ref)
    }

    fn selected_indices(&self) -> Vec<usize> {
        let mut selected = Vec::new();
        let mut start = usize::MAX;
        while let Some(index) = self.next_selected_index(start) {
            selected.push(index);
            start = index;
        }
        selected
    }

    fn selection_count(&self) -> usize {
        let count = send(self.list, LVM_GETSELECTEDCOUNT, 0, 0).max(0) as usize;
        if count == 0 {
            return 0;
        }
        let headers = self.group_headers.iter().filter(|&&(index, _)| {
            send(self.list, LVM_GETITEMSTATE, index, LVIS_SELECTED as isize) != 0
        });
        count.saturating_sub(headers.count())
    }

    fn next_selected_index(&self, mut start: usize) -> Option<usize> {
        loop {
            let index = usize::try_from(send(
                self.list,
                LVM_GETNEXTITEM,
                start,
                LVNI_SELECTED as isize,
            ))
            .ok()?;
            if self.group_header(index).is_none() {
                return Some(index);
            }
            start = index;
        }
    }

    fn list_selection(&self) -> ListSelection {
        let path_at = |row| {
            usize::try_from(row)
                .ok()
                .and_then(|index| self.entry(index))
                .map(|entry| entry.path.clone())
        };
        ListSelection {
            paths: self
                .selected_indices()
                .into_iter()
                .filter_map(|index| self.entry(index))
                .map(|entry| entry.path.clone())
                .collect(),
            focused: path_at(send(
                self.list,
                LVM_GETNEXTITEM,
                usize::MAX,
                LVNI_FOCUSED as isize,
            )),
            anchor: path_at(send(self.list, LVM_GETSELECTIONMARK, 0, 0)),
        }
    }

    fn restore_list_selection(&self, selection: ListSelection) {
        let mut anchor = -1;
        if selection.focused.is_some() || selection.anchor.is_some() || !selection.paths.is_empty()
        {
            for (index, &row) in self.visible.iter().enumerate() {
                let Some(entry) = self.rows.get(row) else {
                    continue;
                };
                let path = &entry.path;
                if selection.anchor.as_ref() == Some(path) {
                    anchor = index as isize;
                }
                let mut flags = 0;
                if selection.paths.contains(path) {
                    flags |= LVIS_SELECTED;
                }
                if selection.focused.as_ref() == Some(path) {
                    flags |= LVIS_FOCUSED;
                }
                if flags != 0 {
                    let state = LVITEMW {
                        state: flags,
                        stateMask: flags,
                        ..unsafe { zeroed() }
                    };
                    send(
                        self.list,
                        LVM_SETITEMSTATE,
                        index,
                        &state as *const _ as isize,
                    );
                }
            }
        }
        send(self.list, LVM_SETSELECTIONMARK, 0, anchor);
    }

    fn first_selected_index(&self) -> Option<usize> {
        self.next_selected_index(usize::MAX)
    }

    fn first_selected(&self) -> Option<&Entry> {
        self.entry(self.first_selected_index()?)
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected_indices()
            .into_iter()
            .filter_map(|index| self.entry(index))
            .filter(|entry| !entry.location)
            .map(|entry| entry.path.clone())
            .collect()
    }

    fn update_preview(&mut self) {
        let requested = if self.settings.details && self.selection_count() == 1 {
            self.first_selected()
                .filter(|entry| !entry.directory && !entry.location)
                .map(|entry| (entry.path.clone(), entry.modified, entry.size))
        } else {
            None
        };
        if self.preview_key == requested {
            return;
        }
        self.preview_key = requested;
        self.preview_id = self.preview_id.wrapping_add(1).max(1);
        self.preview = None;
        if let Some((path, _, _)) = &self.preview_key {
            if self.preview_worker.is_none() {
                let events = Arc::clone(&self.events);
                let owner = self.window as usize;
                self.preview_worker = preview::Worker::start(move |id, content| {
                    publish(&events, owner, Event::Preview(id, content));
                });
            }
            if let Some(worker) = &self.preview_worker {
                worker.request(self.preview_id, path.clone());
            }
        }
        self.layout();
    }

    fn status(&self) {
        let home = self.home.is_some() && *self.location() == Location::Home;
        let items = if home {
            self.home_quick_paths.len() + self.home_activity_paths.len()
        } else {
            self.visible_count()
        };
        let count = if home { 0 } else { self.selection_count() };
        let summary = if self.scanning {
            "Loading…".into()
        } else if items == 1 {
            "1 item".into()
        } else {
            format!("{items} items")
        };
        text(self.summary, &summary);
        let status = if let Some(message) = self.note.as_ref().or(self.error.as_ref()) {
            message.clone()
        } else if self.busy != 0 {
            "Windows is processing the file operation…".into()
        } else if self.scanning {
            format!("{items} found · Reading folder…")
        } else if count != 0 {
            format!("{items} items  ·  {count} selected")
        } else if items == 0 {
            match self.location() {
                Location::Namespace {
                    kind: Namespace::Network,
                    ..
                } => "No network devices or shares found.".into(),
                Location::Namespace {
                    kind: Namespace::RecycleBin,
                    ..
                } => "Recycle Bin is empty.".into(),
                Location::Namespace {
                    kind: Namespace::Libraries,
                    ..
                } => "No libraries or library folders found.".into(),
                _ => "No items to show.".into(),
            }
        } else {
            summary
        };
        text(self.status, status);
        let selected = self.first_selected();
        let actionable = !home && selected.is_some_and(|entry| !entry.location);
        for id in [CUT, COPY] {
            set_enabled(self.button(id), actionable);
        }
        set_enabled(
            self.button(DELETE),
            actionable || self.recycling() && selected.is_some(),
        );
        set_enabled(self.button(RENAME), actionable && count == 1);
        if self.settings.details {
            match selected {
                Some(entry) => {
                    text(self.detail_title, entry.label());
                    let mut body = format!(
                        "{}\n\nSize: {}\n{}: {}\n\n{}",
                        entry.kind(),
                        entry.display_size(),
                        if self.recycling() {
                            "Deleted"
                        } else {
                            "Modified"
                        },
                        entry.date(self.settings.modified_date_format),
                        entry.shell_item.as_ref().map_or_else(
                            || entry.path.display().to_string(),
                            |item| item
                                .original_location
                                .as_ref()
                                .or(item.target.as_ref())
                                .map_or_else(String::new, |path| path.display().to_string()),
                        )
                    );
                    if let Some(Content::Text(preview)) = &self.preview {
                        body.push_str("\n\nPreview\n");
                        body.push_str(preview);
                    }
                    text(self.detail_body, body);
                }
                None => {
                    text(self.detail_title, "Details");
                    text(self.detail_body, "Select an item to see its details.");
                }
            }
        }
    }

    fn show_selected(&mut self, mode: u16) {
        if let Some(index) = self.first_selected_index() {
            self.activate_row(mode, index);
        }
    }

    fn activate_row(&mut self, mode: u16, index: usize) {
        let Some(entry) = self.entry(index) else {
            return;
        };
        if let Some(item) = &entry.shell_item {
            let Location::Namespace { kind, .. } = self.location() else {
                return;
            };
            if *kind == Namespace::RecycleBin {
                return;
            }
            if !matches!(mode, OPEN | OPEN_TAB | OPEN_WINDOW) {
                return;
            }
            let next = if let Some(path) = &item.target {
                let is_share_or_folder = path.components().count() > 1;
                if is_share_or_folder
                    && path
                        .extension()
                        .is_none_or(|extension| !extension.eq_ignore_ascii_case("library-ms"))
                {
                    Some(Location::Folder(path.clone()))
                } else {
                    None
                }
            } else {
                None
            };
            let next = next.unwrap_or_else(|| Location::Namespace {
                kind: *kind,
                child: Some(item.key.clone()),
                title: Some(item.name.clone()),
            });
            if mode == OPEN_TAB {
                self.new_tab(next);
            } else {
                self.go(next);
                self.focus_content();
            }
            return;
        }
        self.show_entry(mode, entry.path.clone(), entry.directory);
    }

    fn show_entry(&mut self, mode: u16, path: PathBuf, directory: bool) {
        match mode {
            OPEN if directory => {
                self.go(Location::Folder(path));
                unsafe {
                    SetFocus(self.list);
                }
            }
            OPEN => {
                self.operation(Operation::Open(path));
            }
            OPEN_TAB if directory => self.new_tab(Location::Folder(path)),
            OPEN_WINDOW if directory => self.window_at(Some(path.as_os_str())),
            OPEN_WITH if !directory => {
                self.operation(Operation::OpenWith(path));
            }
            PROPERTIES => {
                self.operation(Operation::Properties(path));
            }
            COPY_PATH => {
                if self.operation(Operation::CopyPath(path)) {
                    self.clipboard_write = None;
                }
            }
            REVEAL => {
                self.operation(Operation::Reveal(path));
            }
            _ => (),
        }
    }

    fn operation(&mut self, operation: Operation) -> bool {
        let Some(sender) = &self.operations else {
            return false;
        };
        let queued = sender.try_send(operation).is_ok();
        if queued {
            self.note = None;
            self.busy += 1;
        } else {
            self.note =
                Some("The file operation queue is full. Try again when Windows finishes.".into());
        }
        self.status();
        queued
    }

    fn new_tab(&mut self, location: Location) {
        if self.tabs.len() == TAB_LIMIT {
            self.note = Some(format!("This window already has {TAB_LIMIT} tabs."));
            self.status();
            return;
        }
        self.tabs.push(Tab::new(location));
        self.switch(self.tabs.len() - 1);
    }

    fn switch(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
            if self.dest().is_some() {
                self.update_view();
            } else {
                self.apply_view(self.tab().view);
            }
            self.refresh();
            self.schedule_save();
        }
    }

    fn close_tab(&mut self) {
        if self.tabs.len() == 1 {
            unsafe {
                PostMessageW(self.window, WM_CLOSE, 0, 0);
            }
        } else {
            self.tabs.remove(self.active);
            self.switch(self.active.min(self.tabs.len() - 1));
        }
    }

    fn window_at(&mut self, path: Option<&OsStr>) {
        let result = std::env::current_exe().and_then(|exe| {
            let mut command = std::process::Command::new(exe);
            command.arg("--new-window");
            if let Some(path) = path {
                command.arg(path);
            }
            command.spawn()
        });
        if let Err(error) = result {
            self.note = Some(format!("Can't open a new window: {error}"));
            self.status();
        }
    }

    fn schedule_save(&self) {
        if self.booted && self.save_enabled && better_explorer::model::settings_path().is_some() {
            unsafe {
                SetTimer(self.window, SETTINGS_TIMER, 240, None);
            }
        }
    }

    fn settings_snapshot(&self) -> Settings {
        let mut settings = self.settings.clone();
        if self.save_enabled {
            settings.remember_tabs(
                self.tabs.iter().map(|tab| tab.history.current().clone()),
                self.active,
            );
        }
        settings
    }

    fn save(&self) {
        unsafe {
            KillTimer(self.window, SETTINGS_TIMER);
        }
        if self.booted && self.save_enabled {
            if let Some(sender) = &self.saved {
                if sender.try_send(self.settings_snapshot()).is_err() {
                    self.schedule_save();
                }
            }
        }
    }

    fn begin_edit(&mut self, edit: Edit, prompt: &str, value: impl AsRef<OsStr>) {
        let deletion = matches!(edit, Edit::Delete(_));
        self.edit = Some(edit);
        text(self.edit_label, prompt);
        text(self.edit_name, value);
        text(
            self.button(ACCEPT),
            if deletion { "Delete" } else { "Apply" },
        );
        self.layout();
        if deletion {
            unsafe {
                SetFocus(self.button(CANCEL));
            }
        } else {
            unsafe {
                SetFocus(self.edit_name);
            }
            send(self.edit_name, EM_SETSEL, 0, -1);
        }
    }

    fn accept_edit(&mut self) {
        let Some(edit) = self.edit.take() else { return };
        let mut name = read(self.edit_name).to_string_lossy().into_owned();
        let operation = match edit {
            Edit::Delete(paths) => Operation::Delete {
                paths,
                permanent: true,
            },
            edit => {
                if let Edit::Rename(_, Some(extension)) = &edit {
                    name.push('.');
                    name.push_str(&extension.to_string_lossy());
                }
                if !safe_name(&name) {
                    self.edit = Some(edit);
                    self.note = Some("Enter a valid Windows file name.".into());
                    self.status();
                    return;
                }
                match edit {
                    Edit::Create(target, directory) => Operation::Create {
                        target,
                        name,
                        directory,
                    },
                    Edit::Rename(path, _) => Operation::Rename { path, name },
                    Edit::Shortcut(source, target) => {
                        if !name.to_ascii_lowercase().ends_with(".lnk") {
                            name.push_str(".lnk");
                        }
                        Operation::Shortcut {
                            source,
                            target: target.join(name),
                        }
                    }
                    Edit::Compress(sources, folder) => {
                        if !name.to_ascii_lowercase().ends_with(".zip") {
                            name.push_str(".zip");
                        }
                        Operation::Compress {
                            sources,
                            output: folder.join(name),
                        }
                    }
                    Edit::Extract(source, folder) => Operation::Extract {
                        source,
                        output: folder.join(name),
                    },
                    Edit::Delete(_) => unreachable!(),
                }
            }
        };
        self.layout();
        self.operation(operation);
        unsafe {
            SetFocus(self.list);
        }
    }

    fn go_address(&mut self) {
        let value = read(self.address);
        if value.is_empty() {
            return;
        }
        let input = value.to_string_lossy();
        if input.eq_ignore_ascii_case("Home") {
            self.go(Location::Home);
        } else if input.eq_ignore_ascii_case("This PC") {
            self.go(Location::Computer);
        } else if input.eq_ignore_ascii_case("Network") {
            self.go(Location::namespace(Namespace::Network));
        } else if input.eq_ignore_ascii_case("Recycle Bin") {
            self.go(Location::namespace(Namespace::RecycleBin));
        } else if input.eq_ignore_ascii_case("Libraries") {
            self.go(Location::namespace(Namespace::Libraries));
        } else if let Some(server) = input
            .strip_prefix(r"\\")
            .map(|name| name.trim_end_matches('\\'))
            .filter(|name| {
                !name.is_empty() && !name.contains(['\\', '/']) && !matches!(*name, "." | "?")
            })
        {
            self.go(Location::Namespace {
                kind: Namespace::Network,
                child: Some(format!(r"\\{server}")),
                title: Some(server.into()),
            });
        } else {
            let path = work::input_path(value, self.location_dir().as_deref());
            if !path.is_absolute() {
                self.note = Some("Enter a full folder path.".into());
                self.status();
                return;
            }
            self.go(Location::Folder(path));
        }
        unsafe {
            SetFocus(self.list);
        }
    }

    fn search(&mut self) {
        let Some(root) = self.location_dir() else {
            self.note = Some("Open a folder before searching.".into());
            self.status();
            return;
        };
        let query = read(self.search).to_string_lossy().trim().to_owned();
        let location = if query.is_empty() {
            Location::Folder(root)
        } else {
            Location::Search { root, query }
        };
        self.go(location);
        unsafe {
            SetFocus(self.list);
        }
    }

    fn command(&mut self, id: u16) {
        if self.edit.is_some() && !matches!(id, ACCEPT | CANCEL) {
            self.edit = None;
            self.layout();
        }
        match id {
            BACK | FORWARD => {
                let history = &mut self.tab_mut().history;
                if if id == BACK {
                    history.back()
                } else {
                    history.forward()
                }
                .is_some()
                {
                    self.update_view();
                    self.refresh();
                    self.schedule_save();
                }
            }
            UP => {
                let location = match self.location() {
                    Location::Search { root, .. } => Location::Folder(root.clone()),
                    Location::Namespace {
                        kind,
                        child: Some(_),
                        ..
                    } => Location::namespace(*kind),
                    Location::Folder(path) => path
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .map(|p| Location::Folder(p.into()))
                        .unwrap_or(Location::Computer),
                    _ => return,
                };
                self.go(location);
            }
            REFRESH => self.refresh(),
            ENTER_PATH => self.go_address(),
            EDIT_ADDRESS => self.address_focus(true),
            FRAME_MIN | FRAME_MAX => unsafe {
                let command = if id == FRAME_MIN {
                    SC_MINIMIZE
                } else if IsZoomed(self.window) != 0 {
                    SC_RESTORE
                } else {
                    SC_MAXIMIZE
                };
                PostMessageW(self.window, WM_SYSCOMMAND, command as usize, 0);
            },
            FRAME_CLOSE => unsafe {
                PostMessageW(self.window, WM_CLOSE, 0, 0);
            },
            ENTER_SEARCH => self.search(),
            OPEN_SETTINGS => self.open_settings(),
            WIN_E_WINDOWS | WIN_E_BETTER if *self.location() == Location::Settings => {
                self.request_hotkey(HotkeyRequest::Change(id == WIN_E_BETTER));
            }
            DATE_FORMAT_DAY_FIRST | DATE_FORMAT_MONTH_FIRST
                if self.booted && *self.location() == Location::Settings =>
            {
                let format = if id == DATE_FORMAT_DAY_FIRST {
                    ModifiedDateFormat::DayMonthYear
                } else {
                    ModifiedDateFormat::MonthDayYear
                };
                if self.settings.modified_date_format != format {
                    self.settings.modified_date_format = format;
                    self.update_modified_date_controls();
                    unsafe {
                        InvalidateRect(self.list, null(), 0);
                    }
                    self.status();
                    self.schedule_save();
                }
            }
            HOME => self.go(Location::Home),
            COMPUTER => self.go(Location::Computer),
            NETWORK => self.go(Location::namespace(Namespace::Network)),
            RECYCLE => self.go(Location::namespace(Namespace::RecycleBin)),
            LIBRARIES => self.go(Location::namespace(Namespace::Libraries)),
            NEW_TAB => self.new_tab(Location::Home),
            CLOSE_TAB => self.close_tab(),
            NEXT_TAB => self.switch((self.active + 1) % self.tabs.len()),
            PREVIOUS_TAB => self.switch((self.active + self.tabs.len() - 1) % self.tabs.len()),
            MOVE_TAB_LEFT | MOVE_TAB_RIGHT => {
                let index = if id == MOVE_TAB_LEFT {
                    self.active.saturating_sub(1)
                } else {
                    (self.active + 1).min(self.tabs.len() - 1)
                };
                self.tabs.swap(self.active, index);
                self.switch(index);
            }
            NEW_WINDOW => self.window_at(self.location_dir().as_deref().map(Path::as_os_str)),
            OPEN | OPEN_TAB | OPEN_WINDOW | OPEN_WITH | PROPERTIES | COPY_PATH | REVEAL => {
                self.show_selected(id)
            }
            OPEN_TERMINAL => {
                let path = self
                    .first_selected()
                    .filter(|row| row.directory && row.shell_item.is_none())
                    .map(|row| row.path.clone())
                    .or_else(|| self.dest());
                if let Some(path) = path {
                    self.operation(Operation::Terminal(path));
                }
            }
            FOLDER_PROPERTIES | FOLDER_COPY_PATH => {
                if let Some(folder) = self.dest() {
                    self.show_entry(
                        if id == FOLDER_PROPERTIES {
                            PROPERTIES
                        } else {
                            COPY_PATH
                        },
                        folder,
                        true,
                    );
                }
            }
            CUT | COPY => {
                let paths = self.selected_paths();
                if !paths.is_empty() {
                    let receipt = shell::ClipboardWriteReceipt::capture();
                    if self.operation(Operation::Clipboard {
                        paths,
                        cut: id == CUT,
                        receipt: receipt.clone(),
                    }) {
                        self.clipboard_write = Some(receipt);
                    }
                }
            }
            PASTE => {
                if let Some(target) = self.dest() {
                    let receipt = shell::ClipboardPasteReceipt::capture(
                        self.window as usize,
                        self.clipboard_write.clone(),
                    );
                    self.operation(Operation::Paste { target, receipt });
                }
            }
            RECYCLE_RESTORE | DELETE | PERMANENT if self.recycling() => {
                let keys: Vec<_> = self
                    .selected_indices()
                    .into_iter()
                    .filter_map(|index| self.entry(index)?.shell_item.as_ref())
                    .map(|item| item.key.clone())
                    .collect();
                if !keys.is_empty() {
                    self.operation(Operation::Recycle {
                        keys,
                        restore: id == RECYCLE_RESTORE,
                    });
                }
            }
            RECYCLE_EMPTY if self.recycling() && !self.rows.is_empty() => {
                self.operation(Operation::EmptyRecycle);
            }
            DELETE | PERMANENT => {
                let paths = self.selected_paths();
                if paths.is_empty() {
                    return;
                }
                if id == DELETE {
                    self.operation(Operation::Delete {
                        paths,
                        permanent: false,
                    });
                } else {
                    let count = paths.len();
                    self.begin_edit(
                        Edit::Delete(paths),
                        "Permanently delete?",
                        format!("{count} item(s) will be removed."),
                    );
                    send(self.edit_name, EM_SETREADONLY, 1, 0);
                }
            }
            NEW_FOLDER | NEW_TEXT => {
                if let Some(target) = self.dest() {
                    let directory = id == NEW_FOLDER;
                    self.begin_edit(
                        Edit::Create(target, directory),
                        if directory {
                            "New folder name"
                        } else {
                            "New document name"
                        },
                        if directory {
                            "New folder"
                        } else {
                            "New Text Document.txt"
                        },
                    );
                    send(self.edit_name, EM_SETREADONLY, 0, 0);
                }
            }
            RENAME => {
                if self.selection_count() != 1 {
                    return;
                }
                if let Some(entry) = self.first_selected().filter(|entry| !entry.location) {
                    let path = entry.path.clone();
                    let extension = if self.settings.hide_extensions && !entry.directory {
                        path.extension().map(OsStr::to_owned)
                    } else {
                        None
                    };
                    let value = from_wide(entry.display_name(self.settings.hide_extensions));
                    self.begin_edit(Edit::Rename(path, extension), "Rename item", value);
                    send(self.edit_name, EM_SETREADONLY, 0, 0);
                }
            }
            SHORTCUT => {
                if let (Some(entry), Some(target)) = (self.first_selected(), self.dest()) {
                    let source = entry.path.clone();
                    let label = format!("{} - Shortcut.lnk", entry.label());
                    self.begin_edit(Edit::Shortcut(source, target), "Shortcut name", label);
                    send(self.edit_name, EM_SETREADONLY, 0, 0);
                }
            }
            COMPRESS => {
                let sources = self.selected_paths();
                if let (Some(first), Some(target)) = (sources.first(), self.dest()) {
                    let default = if sources.len() == 1 {
                        let base = first.file_stem().unwrap_or(OsStr::new("Archive"));
                        format!("{}.zip", base.to_string_lossy())
                    } else {
                        "Archive.zip".into()
                    };
                    self.begin_edit(Edit::Compress(sources, target), "ZIP archive name", default);
                    send(self.edit_name, EM_SETREADONLY, 0, 0);
                }
            }
            EXTRACT => {
                if let Some(entry) = self
                    .first_selected()
                    .filter(|item| is_zip(&item.path) && !item.directory)
                {
                    let source = entry.path.clone();
                    if let Some(parent) = source.parent() {
                        let default = source
                            .file_stem()
                            .unwrap_or(OsStr::new("Extracted"))
                            .to_owned();
                        self.begin_edit(
                            Edit::Extract(source.clone(), parent.into()),
                            "Extract to folder",
                            default,
                        );
                        send(self.edit_name, EM_SETREADONLY, 0, 0);
                    }
                }
            }
            ACCEPT => self.accept_edit(),
            CANCEL => {
                self.edit = None;
                self.layout();
                unsafe {
                    SetFocus(self.list);
                }
            }
            STOP_SEARCH => {
                if let Location::Search { root, .. } = self.location() {
                    self.go(Location::Folder(root.clone()));
                }
            }
            SELECT_ALL => {
                let state = LVITEMW {
                    state: LVIS_SELECTED,
                    stateMask: LVIS_SELECTED,
                    ..unsafe { zeroed() }
                };
                send(
                    self.list,
                    LVM_SETITEMSTATE,
                    usize::MAX,
                    &state as *const _ as isize,
                );
                self.clear_group_selection();
                self.status();
            }
            DETAILS => {
                self.settings.details = !self.settings.details;
                self.update_preview();
                self.layout();
                self.status();
                self.schedule_save();
            }
            DETAILS_VIEW | LIST_VIEW | ICON_VIEW | QUICK_DETAILS | QUICK_ICONS | CARDS_VIEW
            | SMALL_ICONS | LARGE_ICONS | XLARGE_ICONS => {
                let mode = match id {
                    LIST_VIEW => LV_VIEW_LIST,
                    ICON_VIEW | QUICK_ICONS => LV_VIEW_ICON,
                    CARDS_VIEW => VIEW_CARDS,
                    SMALL_ICONS => VIEW_SMALL,
                    LARGE_ICONS => VIEW_LARGE,
                    XLARGE_ICONS => VIEW_XLARGE,
                    _ => LV_VIEW_DETAILS,
                };
                if let Some(path) = self.dest() {
                    self.settings.views.insert(path, mode);
                    if self.settings.views.len() > 400 {
                        let key = self
                            .settings
                            .views
                            .keys()
                            .find(|key| Some(key.as_path()) != self.location().directory())
                            .cloned();
                        if let Some(key) = key {
                            self.settings.views.remove(&key);
                        }
                    }
                }
                let previous_group = self.group_by();
                self.apply_view(mode);
                if previous_group != self.group_by() {
                    self.rebuild_grouping();
                }
                self.layout();
                self.schedule_save();
            }
            SHOW_HIDDEN => {
                self.settings.hidden = !self.settings.hidden;
                self.refresh();
                self.schedule_save();
            }
            SHOW_EXTENSIONS => {
                self.settings.hide_extensions = !self.settings.hide_extensions;
                self.update_home();
                unsafe {
                    InvalidateRect(self.list, null(), 0);
                }
                self.schedule_save();
            }
            SORT_NAME | SORT_MODIFIED | SORT_KIND | SORT_SIZE | SORT_REVERSE => {
                let mut sort = self.sort();
                let column = match id {
                    SORT_NAME => Column::Name,
                    SORT_MODIFIED => Column::Modified,
                    SORT_KIND => Column::Kind,
                    SORT_SIZE => Column::Size,
                    _ => sort.column,
                };
                if id == SORT_REVERSE || sort.column == column {
                    sort.descending = !sort.descending;
                } else {
                    sort = Sort {
                        column,
                        descending: column == Column::Modified,
                    };
                }
                if let Some(path) = self.dest() {
                    self.settings.folder_orders.entry(path).or_default().sort = sort;
                    self.schedule_save();
                } else {
                    self.tab_mut().sort = sort;
                }
                unsafe { InvalidateRect(self.header, null(), 0) };
                self.reload(true);
            }
            GROUP_NONE | GROUP_MODIFIED => {
                if let Some(path) = self.dest() {
                    self.settings.folder_orders.entry(path).or_default().group = if id == GROUP_NONE
                    {
                        GroupBy::None
                    } else {
                        GroupBy::DateModified
                    };
                    self.rebuild_grouping();
                    self.status();
                    self.schedule_save();
                }
            }
            HOME_RECENT | HOME_FAVORITES => {
                self.home_activity = if id == HOME_RECENT {
                    Activity::Recent
                } else {
                    Activity::Favorites
                };
                self.update_home();
                self.status();
            }
            HOME_TILES | HOME_CONTENT => {
                self.settings.home_compact = id == HOME_CONTENT;
                self.update_home();
                self.schedule_save();
            }
            FAVORITE => {
                if let Some(path) = self
                    .first_selected()
                    .filter(|row| !row.directory)
                    .map(|row| row.path.clone())
                {
                    if let Some(index) = self
                        .settings
                        .favorites
                        .iter()
                        .position(|favorite| favorite == &path)
                    {
                        self.settings.favorites.remove(index);
                        self.note = Some("File removed from Favorites.".into());
                    } else if self.settings.favorites.len() < 75 {
                        self.settings.favorites.push(path);
                        self.note = Some("File added to Favorites.".into());
                    } else {
                        self.note = Some("75 files are already in Favorites.".into());
                    }
                    self.update_home();
                    self.status();
                    self.schedule_save();
                }
            }
            id if (HOME_QUICK_START..HOME_ACTIVITY_START).contains(&id) => {
                if let Some(path) = self
                    .home_quick_paths
                    .get((id - HOME_QUICK_START) as usize)
                    .cloned()
                {
                    self.show_entry(OPEN, path, true);
                }
            }
            id if (HOME_ACTIVITY_START..HOME_ACTIVITY_START + 75).contains(&id) => {
                if let Some(path) = self
                    .home_activity_paths
                    .get((id - HOME_ACTIVITY_START) as usize)
                    .cloned()
                {
                    self.show_entry(OPEN, path, false);
                }
            }
            FILTER_ALL | FILTER_FOLDERS | FILTER_FILES | 199..=204 => {
                let filter = match id {
                    FILTER_FOLDERS => Filter::Folders,
                    FILTER_FILES => Filter::Files,
                    _ => FILTER_TYPES
                        .iter()
                        .find(|(command, _, _)| *command == id)
                        .map_or(Filter::All, |(_, filter, _)| *filter),
                };
                if *self.location() == Location::Home {
                    self.home_filter = filter;
                    self.update_home();
                    self.status();
                    return;
                }
                self.tab_mut().filter = filter;
                self.clear_selection();
                self.rebuild_visible();
                self.update_drive_buttons();
                self.layout();
                self.status();
            }
            THEME_SYSTEM | THEME_DARK | THEME_LIGHT => {
                self.settings.theme = match id {
                    THEME_DARK => ThemePreference::Dark,
                    THEME_LIGHT => ThemePreference::Light,
                    _ => ThemePreference::System,
                };
                self.apply_theme();
                self.schedule_save();
            }
            RESTORE_TABS => {
                self.settings.restore_tabs = !self.settings.restore_tabs;
                self.schedule_save();
            }
            PIN => {
                let path = self
                    .first_selected()
                    .filter(|entry| entry.directory && entry.shell_item.is_none())
                    .map(|entry| entry.path.clone())
                    .or_else(|| self.dest());
                if let Some(path) = path {
                    if let Some(index) = self.settings.pins.iter().position(|pin| *pin == path) {
                        self.settings.pins.remove(index);
                        self.note = Some("Folder unpinned.".into());
                    } else if self.settings.pins.len() < 75 {
                        self.settings.pins.push(path);
                        self.note = Some("Folder pinned.".into());
                    } else {
                        self.note = Some("75 folders are already pinned.".into());
                    }
                    let note = self.note.clone();
                    self.update_sidebar();
                    if matches!(self.location(), Location::Home) {
                        self.refresh();
                    }
                    self.note = note;
                    self.status();
                    self.schedule_save();
                }
            }
            id if (TAB_START..TAB_START + TAB_LIMIT as u16).contains(&id) => {
                self.switch((id - TAB_START) as usize);
            }
            id if (CRUMB_START..CRUMB_START + CRUMB_LIMIT as u16).contains(&id) => {
                if let Some(location) = self
                    .crumb_slots
                    .get((id - CRUMB_START) as usize)
                    .cloned()
                    .flatten()
                {
                    if self.location() == &location {
                        self.address_focus(true);
                    } else {
                        self.go(location);
                    }
                } else {
                    unsafe {
                        PostMessageW(
                            self.window,
                            SHOW_MENU,
                            PATH as usize,
                            self.button(id) as LPARAM,
                        );
                    }
                }
            }
            id if (DRIVE_START..DRIVE_START + DRIVE_LIMIT as u16).contains(&id) => {
                if self.drive_cards() {
                    if let Some(path) = self
                        .entry((id - DRIVE_START) as usize)
                        .map(|row| row.path.clone())
                    {
                        self.go(Location::Folder(path));
                        unsafe {
                            SetFocus(self.list);
                        }
                    }
                }
            }
            id if id >= BREADCRUMB_START => {
                if let Some(path) = self.crumbs.get((id - BREADCRUMB_START) as usize) {
                    self.go(Location::Folder(path.clone()));
                }
            }
            _ => (),
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            for image in self.images {
                if image != 0 {
                    ImageList_Destroy(image);
                }
            }
            for object in self.fonts.into_iter().chain(self.brushes) {
                if !object.is_null() {
                    DeleteObject(object);
                }
            }
        }
    }
}

struct MenuItem {
    id: u16,
    label: String,
    enabled: bool,
    checked: bool,
    submenu: bool,
    children: Vec<MenuItem>,
}

impl MenuItem {
    fn new(id: u16, label: &str) -> Self {
        Self {
            id,
            label: label.into(),
            enabled: true,
            checked: false,
            submenu: false,
            children: Vec::new(),
        }
    }
    fn checked(mut self, value: bool) -> Self {
        self.checked = value;
        self
    }
    fn enabled(mut self, value: bool) -> Self {
        self.enabled = value;
        self
    }
    fn separator() -> Self {
        Self::new(0, "")
    }

    fn submenu(id: u16, label: &str, children: Vec<Self>) -> Self {
        let mut item = Self::new(id, label);
        item.submenu = !children.is_empty();
        item.children = children;
        item
    }

    fn glyph(&self) -> Option<Glyph> {
        Some(match self.id {
            VIEW => Glyph::View,
            SORT => Glyph::Sort,
            GROUP_SUBMENU => Glyph::Details,
            REFRESH => Glyph::Refresh,
            NEW => Glyph::Add,
            NEW_FOLDER => Glyph::Folder,
            NEW_TEXT => Glyph::Document,
            OPEN => Glyph::Forward,
            OPEN_TAB => Glyph::Add,
            OPEN_WINDOW | REVEAL => Glyph::Restore,
            OPEN_WITH => Glyph::Document,
            OPEN_TERMINAL => Glyph::Terminal,
            CUT => Glyph::Cut,
            COPY => Glyph::Copy,
            PASTE => Glyph::Paste,
            RENAME => Glyph::Rename,
            DELETE | RECYCLE_EMPTY => Glyph::Delete,
            SHORTCUT | COPY_PATH | FOLDER_COPY_PATH => Glyph::Link,
            PIN | FAVORITE => Glyph::Pin,
            COMPRESS | EXTRACT => Glyph::Archive,
            PROPERTIES | FOLDER_PROPERTIES => Glyph::Settings,
            RECYCLE_RESTORE => Glyph::Restore,
            _ => return None,
        })
    }
}

impl App {
    fn menu(&mut self, id: u16) -> Vec<MenuItem> {
        let item = MenuItem::new;
        let line = MenuItem::separator;
        match id {
            NEW => vec![
                item(NEW_FOLDER, "Folder\tCtrl+Shift+N"),
                item(NEW_TEXT, "Text Document"),
            ],
            SORT => {
                let sort = self.sort();
                vec![
                    item(SORT_NAME, "Name").checked(sort.column == Column::Name),
                    item(SORT_MODIFIED, self.column_label(1))
                        .checked(sort.column == Column::Modified),
                    item(SORT_KIND, self.column_label(2)).checked(sort.column == Column::Kind),
                    item(SORT_SIZE, "Size").checked(sort.column == Column::Size),
                    line(),
                    item(SORT_REVERSE, "Descending").checked(sort.descending),
                ]
            }
            VIEW => {
                let view = self.tab().view;
                let mut items = vec![];
                if self.home.is_some() && *self.location() == Location::Home {
                    return vec![
                        item(HOME_TILES, "Tiles").checked(!self.settings.home_compact),
                        item(HOME_CONTENT, "Content").checked(self.settings.home_compact),
                        line(),
                        item(DETAILS, "Details pane").checked(self.settings.details),
                        item(SHOW_HIDDEN, "Hidden items").checked(self.settings.hidden),
                        item(SHOW_EXTENSIONS, "File name extensions")
                            .checked(!self.settings.hide_extensions),
                    ];
                }
                if matches!(self.location(), Location::Computer) {
                    items.push(item(CARDS_VIEW, "Drive cards").checked(view == VIEW_CARDS));
                }
                items.extend([
                    item(XLARGE_ICONS, "Extra large icons").checked(view == VIEW_XLARGE),
                    item(LARGE_ICONS, "Large icons").checked(view == VIEW_LARGE),
                    item(ICON_VIEW, "Medium icons").checked(view == LV_VIEW_ICON),
                    item(SMALL_ICONS, "Small icons").checked(view == VIEW_SMALL),
                    item(LIST_VIEW, "List").checked(view == LV_VIEW_LIST),
                    item(DETAILS_VIEW, "Details").checked(view == LV_VIEW_DETAILS),
                    line(),
                    item(SHOW_HIDDEN, "Hidden items").checked(self.settings.hidden),
                    item(SHOW_EXTENSIONS, "File name extensions")
                        .checked(!self.settings.hide_extensions),
                    item(DETAILS, "Details pane").checked(self.settings.details),
                ]);
                items
            }
            FILTER => {
                let filter = if *self.location() == Location::Home {
                    self.home_filter
                } else {
                    self.tab().filter
                };
                let mut items =
                    vec![item(FILTER_ALL, "All file types").checked(filter == Filter::All)];
                if *self.location() != Location::Home {
                    items.extend([
                        item(FILTER_FOLDERS, "Folders only").checked(filter == Filter::Folders),
                        item(FILTER_FILES, "Files only").checked(filter == Filter::Files),
                    ]);
                }
                items.extend(
                    FILTER_TYPES.map(|(id, value, label)| item(id, label).checked(filter == value)),
                );
                items
            }
            MORE => {
                let width = rect(self.window).right;
                let selection = self.first_selected();
                let mut commands = vec![item(OPEN_SETTINGS, "Settings\tCtrl+,")];
                if self.recycling() {
                    commands.extend([
                        line(),
                        item(RECYCLE_RESTORE, "Restore selected items")
                            .enabled(selection.is_some()),
                        item(RECYCLE_EMPTY, "Empty Recycle Bin").enabled(!self.rows.is_empty()),
                    ]);
                }
                if self.compact_toolbar(width) {
                    commands.push(line());
                    if width < scale(650, self.dpi) {
                        commands.extend([
                            item(CUT, "Cut\tCtrl+X")
                                .enabled(selection.is_some_and(|row| !row.location)),
                            item(COPY, "Copy\tCtrl+C")
                                .enabled(selection.is_some_and(|row| !row.location)),
                            item(PASTE, "Paste\tCtrl+V").enabled(self.dest().is_some()),
                        ]);
                    }
                    commands.extend([
                        item(RENAME, "Rename\tF2").enabled(
                            self.selection_count() == 1
                                && selection.is_some_and(|entry| !entry.location),
                        ),
                        item(DELETE, "Delete\tDel")
                            .enabled(selection.is_some_and(|entry| !entry.location)),
                    ]);
                    commands.extend(self.menu(FILTER));
                }
                commands.extend([
                    line(),
                    item(NEW_WINDOW, "New window\tCtrl+N"),
                    item(CLOSE_TAB, "Close tab\tCtrl+W"),
                    item(MOVE_TAB_LEFT, "Move tab left").enabled(self.active > 0),
                    item(MOVE_TAB_RIGHT, "Move tab right")
                        .enabled(self.active + 1 < self.tabs.len()),
                    line(),
                    item(PIN, "Pin or unpin folder").enabled(
                        self.location_dir().is_some()
                            || self
                                .first_selected()
                                .is_some_and(|row| row.directory && row.shell_item.is_none()),
                    ),
                    item(SELECT_ALL, "Select all\tCtrl+A")
                        .enabled(*self.location() != Location::Home && !self.visible.is_empty()),
                    line(),
                    item(RESTORE_TABS, "Restore tabs on startup")
                        .checked(self.settings.restore_tabs),
                    line(),
                    item(THEME_SYSTEM, "Use system theme")
                        .checked(matches!(self.settings.theme, ThemePreference::System)),
                    item(THEME_DARK, "Dark theme")
                        .checked(matches!(self.settings.theme, ThemePreference::Dark)),
                    item(THEME_LIGHT, "Light theme")
                        .checked(matches!(self.settings.theme, ThemePreference::Light)),
                ]);
                commands
            }
            PATH => {
                let mut items = vec![
                    item(EDIT_ADDRESS, "Edit address\tCtrl+L"),
                    line(),
                    item(HOME, "Home"),
                    item(COMPUTER, "This PC"),
                ];
                if !self.crumbs.is_empty() {
                    items.push(line());
                }
                for (index, path) in self.crumbs.iter().enumerate().rev() {
                    let label = path.to_string_lossy().replace('&', "&&");
                    items.push(item(BREADCRUMB_START + index as u16, &label).checked(index == 0));
                }
                items
            }
            CONTEXT => {
                if self.recycling() && self.first_selected().is_none() {
                    return vec![
                        item(RECYCLE_EMPTY, "Empty Recycle Bin").enabled(!self.rows.is_empty()),
                        item(REFRESH, "Refresh\tF5"),
                    ];
                }
                if let Some(entry) = self.first_selected() {
                    let count = self.selection_count();
                    if entry.shell_item.is_some() {
                        return if matches!(
                            self.location(),
                            Location::Namespace {
                                kind: Namespace::RecycleBin,
                                ..
                            }
                        ) {
                            vec![
                                item(RECYCLE_RESTORE, "Restore"),
                                item(DELETE, "Delete permanently"),
                                line(),
                                item(RECYCLE_EMPTY, "Empty Recycle Bin"),
                                item(REFRESH, "Refresh\tF5"),
                            ]
                        } else {
                            vec![
                                item(OPEN, "Open\tEnter"),
                                item(OPEN_TAB, "Open in new tab"),
                                line(),
                                item(REFRESH, "Refresh\tF5"),
                            ]
                        };
                    }
                    let mut items = vec![item(OPEN, "Open\tEnter").enabled(count == 1)];
                    if entry.directory {
                        items.push(item(OPEN_TAB, "Open in new tab").enabled(count == 1));
                        items.push(item(OPEN_WINDOW, "Open in new window").enabled(count == 1));
                        items.push(item(OPEN_TERMINAL, "Open in Terminal").enabled(count == 1));
                    } else {
                        items.push(item(OPEN_WITH, "Open with…").enabled(count == 1));
                    }
                    items.extend([
                        line(),
                        item(CUT, "Cut\tCtrl+X").enabled(!entry.location),
                        item(COPY, "Copy\tCtrl+C").enabled(!entry.location),
                        item(RENAME, "Rename\tF2").enabled(!entry.location && count == 1),
                        item(DELETE, "Delete\tDel").enabled(!entry.location),
                        item(COPY_PATH, "Copy as path").enabled(count == 1),
                        line(),
                    ]);
                    if entry.directory {
                        items.push(
                            item(
                                PIN,
                                if self.settings.pins.contains(&entry.path) {
                                    "Unpin folder"
                                } else {
                                    "Pin folder"
                                },
                            )
                            .enabled(count == 1),
                        );
                    } else if count == 1 {
                        items.push(item(
                            FAVORITE,
                            if self.settings.favorites.contains(&entry.path) {
                                "Remove from Favorites"
                            } else {
                                "Add to Favorites"
                            },
                        ));
                    }
                    if !entry.location && self.dest().is_some() {
                        items.push(item(COMPRESS, "Compress to ZIP…"));
                    }
                    if !entry.directory && count == 1 && is_zip(&entry.path) {
                        items.push(item(EXTRACT, "Extract all…"));
                    }
                    items.extend([
                        item(SHORTCUT, "Create shortcut…")
                            .enabled(count == 1 && self.dest().is_some()),
                        line(),
                        item(REVEAL, "Show in Windows File Explorer").enabled(count == 1),
                        item(PROPERTIES, "Properties\tAlt+Enter").enabled(count == 1),
                    ]);
                    items
                } else {
                    let destination = self.dest().is_some();
                    let paste = destination && shell::has_file_clipboard();
                    let mut items = vec![
                        MenuItem::submenu(VIEW, "View", self.menu(VIEW)),
                        MenuItem::submenu(SORT, "Sort by", self.menu(SORT)),
                    ];
                    let groups = self.group_menu();
                    if !groups.is_empty() {
                        items.push(MenuItem::submenu(GROUP_SUBMENU, "Group by", groups));
                    }
                    items.extend([
                        item(REFRESH, "Refresh\tF5"),
                        line(),
                        item(PASTE, "Paste\tCtrl+V").enabled(paste),
                        MenuItem::submenu(NEW, "New", self.menu(NEW)).enabled(destination),
                    ]);
                    if destination {
                        items.extend([
                            line(),
                            item(OPEN_TERMINAL, "Open in Terminal"),
                            item(FOLDER_COPY_PATH, "Copy folder path"),
                            item(PIN, "Pin or unpin current folder"),
                            line(),
                            item(FOLDER_PROPERTIES, "Properties"),
                        ]);
                    }
                    items
                }
            }
            _ => Vec::new(),
        }
    }

    fn button_style(&self, id: u16, control: HWND) -> ButtonStyle {
        let tab = (TAB_START..TAB_START + TAB_LIMIT as u16).contains(&id);
        let crumb = (CRUMB_START..CRUMB_START + CRUMB_LIMIT as u16).contains(&id);
        let glyph = match id {
            BACK => Some(Glyph::Back),
            FORWARD => Some(Glyph::Forward),
            UP => Some(Glyph::Up),
            REFRESH => Some(Glyph::Refresh),
            NEW | NEW_TAB => Some(Glyph::Add),
            CLOSE_TAB | FRAME_CLOSE => Some(Glyph::Close),
            CUT => Some(Glyph::Cut),
            COPY => Some(Glyph::Copy),
            PASTE => Some(Glyph::Paste),
            RENAME => Some(Glyph::Rename),
            DELETE => Some(Glyph::Delete),
            SORT => Some(Glyph::Sort),
            VIEW | QUICK_DETAILS => Some(Glyph::View),
            FILTER => Some(Glyph::Filter),
            MORE => Some(Glyph::More),
            DETAILS | QUICK_ICONS => Some(Glyph::Details),
            OPEN_SETTINGS => Some(Glyph::Settings),
            WIN_E_WINDOWS => Some(Glyph::Drive),
            WIN_E_BETTER => Some(Glyph::Folder),
            PATH => Some(match self.location() {
                Location::Home => Glyph::Home,
                Location::Settings => Glyph::Settings,
                Location::Namespace {
                    kind: Namespace::Network,
                    ..
                } => Glyph::Network,
                Location::Namespace {
                    kind: Namespace::RecycleBin,
                    ..
                } => Glyph::Recycle,
                Location::Namespace {
                    kind: Namespace::Libraries,
                    ..
                } => Glyph::Library,
                _ => Glyph::Drive,
            }),
            COMPUTER => Some(Glyph::Drive),
            HOME => Some(Glyph::Home),
            NETWORK => Some(Glyph::Network),
            RECYCLE => Some(Glyph::Recycle),
            LIBRARIES => Some(Glyph::Library),
            FRAME_MIN => Some(Glyph::Minimize),
            FRAME_MAX => Some(if unsafe { IsZoomed(self.window) } != 0 {
                Glyph::Restore
            } else {
                Glyph::Maximize
            }),
            id if crumb
                && id != CRUMB_START
                && self
                    .crumb_slots
                    .get((id - CRUMB_START) as usize)
                    .is_some_and(Option::is_some) =>
            {
                Some(Glyph::Chevron)
            }
            _ if tab => Some(
                match self
                    .tabs
                    .get((id - TAB_START) as usize)
                    .map(|tab| tab.history.current())
                {
                    Some(Location::Home) => Glyph::Home,
                    Some(Location::Computer) => Glyph::Drive,
                    Some(Location::Settings) => Glyph::Settings,
                    Some(Location::Search { .. }) => Glyph::Search,
                    Some(Location::Namespace {
                        kind: Namespace::Network,
                        ..
                    }) => Glyph::Network,
                    Some(Location::Namespace {
                        kind: Namespace::RecycleBin,
                        ..
                    }) => Glyph::Recycle,
                    Some(Location::Namespace {
                        kind: Namespace::Libraries,
                        ..
                    }) => Glyph::Library,
                    _ => Glyph::Folder,
                },
            ),
            _ => None,
        };
        let title = tab
            || matches!(
                id,
                NEW_TAB
                    | CLOSE_TAB
                    | FRAME_MIN
                    | FRAME_MAX
                    | FRAME_CLOSE
                    | BACK
                    | FORWARD
                    | UP
                    | REFRESH
            );
        let surface = if title {
            self.colors.title
        } else if crumb || id == PATH {
            self.colors.field
        } else {
            self.colors.chrome
        };
        let accent = tab || matches!(id, SORT | VIEW | FILTER | NEW);
        let icon_color = if accent {
            self.colors.accent
        } else {
            self.colors.text
        };
        ButtonStyle {
            glyph,
            icon_color,
            surface,
            label: (id != DETAILS || !self.compact_toolbar(rect(self.window).right))
                && (crumb
                    || tab
                    || matches!(
                        id,
                        NEW | SORT
                            | VIEW
                            | FILTER
                            | DETAILS
                            | OPEN_SETTINGS
                            | WIN_E_WINDOWS
                            | WIN_E_BETTER
                    )
                    || glyph.is_none()),
            left: crumb || tab || matches!(id, WIN_E_WINDOWS | WIN_E_BETTER),
            active: (tab && id == TAB_START + self.active as u16)
                || (id == OPEN_SETTINGS && *self.location() == Location::Settings)
                || (matches!(id, WIN_E_WINDOWS | WIN_E_BETTER)
                    && self.hotkey_status.as_ref().is_some_and(|status| {
                        !status.another_copy
                            && (status.configured || status.running) == (id == WIN_E_BETTER)
                    }))
                || (id == DATE_FORMAT_DAY_FIRST
                    && self.settings.modified_date_format == ModifiedDateFormat::DayMonthYear)
                || (id == DATE_FORMAT_MONTH_FIRST
                    && self.settings.modified_date_format == ModifiedDateFormat::MonthDayYear)
                || (id == DETAILS && self.settings.details)
                || (id == HOME && matches!(self.location(), Location::Home))
                || (id == COMPUTER && matches!(self.location(), Location::Computer))
                || (id == QUICK_DETAILS && self.tab().view == LV_VIEW_DETAILS)
                || (id == QUICK_ICONS
                    && matches!(
                        self.tab().view,
                        LV_VIEW_ICON | VIEW_LARGE | VIEW_XLARGE | VIEW_SMALL
                    )),
            danger: id == FRAME_CLOSE,
            hover: self.hover == control,
        }
    }

    fn draw_drive(&self, draw: &DRAWITEMSTRUCT, index: usize) {
        let Some(entry) = self.entry(index) else {
            return;
        };
        let dc = draw.hDC;
        let r = draw.rcItem;
        let d = |v| scale(v, self.dpi);
        let active =
            draw.itemState & (ODS_SELECTED | ODS_FOCUS) != 0 || self.hover == draw.hwndItem;
        fill(
            dc,
            &r,
            if active {
                self.colors.active
            } else {
                self.colors.content
            },
        );
        let outline = if active {
            self.colors.accent
        } else {
            self.colors.edge
        };
        unsafe {
            let brush = CreateSolidBrush(outline);
            FrameRect(dc, &r, brush);
            DeleteObject(brush);
        }
        draw_glyph(
            dc,
            Glyph::Drive,
            r.left + d(11),
            r.top + d(13),
            d(28),
            self.colors.accent,
        );
        let x = r.left + d(50);
        let w = r.right - d(10);
        let row = |top, bottom| RECT {
            left: x,
            right: w,
            top: r.top + d(top),
            bottom: r.top + d(bottom),
        };
        text_at(
            dc,
            &entry.label(),
            row(4, 23),
            self.fonts[1],
            self.colors.text,
            DT_SINGLELINE | DT_VCENTER,
        );
        if let Some(capacity) = &entry.capacity {
            let bar = row(26, 33);
            fill(dc, &bar, self.colors.edge);
            let used = (capacity.total - capacity.free) as f64 / capacity.total as f64;
            let color = if used > 0.93 && !self.colors.contrast {
                rgb(205, 71, 52)
            } else {
                self.colors.accent
            };
            fill(
                dc,
                &RECT {
                    right: x + ((w - x) as f64 * used) as i32,
                    ..bar
                },
                color,
            );
            text_at(
                dc,
                &capacity.label(),
                row(34, 54),
                self.fonts[0],
                self.colors.muted,
                DT_SINGLELINE | DT_VCENTER,
            );
        } else {
            text_at(
                dc,
                "Storage details unavailable",
                row(29, 52),
                self.fonts[0],
                self.colors.muted,
                DT_SINGLELINE | DT_VCENTER,
            );
        }
    }

    fn recycling(&self) -> bool {
        matches!(
            self.location(),
            Location::Namespace {
                kind: Namespace::RecycleBin,
                ..
            }
        )
    }

    fn column_label(&self, index: usize) -> &'static str {
        if self.recycling() {
            match index {
                1 => return "Date deleted",
                2 => return "Original location",
                _ => {}
            }
        }
        COLUMNS[index].0
    }

    fn sync_column_labels(&self) {
        for index in [1, 2] {
            let mut label = wide(self.column_label(index));
            let column = LVCOLUMNW {
                mask: LVCF_TEXT,
                pszText: label.as_mut_ptr(),
                ..unsafe { zeroed() }
            };
            send(
                self.list,
                LVM_SETCOLUMNW,
                index,
                &column as *const _ as LPARAM,
            );
        }
    }

    fn draw_header(&self) {
        let header = self.header;
        let mut paint = unsafe { zeroed() };
        let dc = unsafe { BeginPaint(header, &mut paint) };
        let bounds = rect(header);
        let d = |v| scale(v, self.dpi);
        fill(dc, &bounds, self.colors.sidebar);
        let sort = self.sort();
        for index in 0..COLUMNS.len() {
            let label = self.column_label(index);
            let mut r: RECT = unsafe { zeroed() };
            if send(header, HDM_GETITEMRECT, index, &mut r as *mut _ as LPARAM) == 0 {
                continue;
            }
            fill(
                dc,
                &RECT {
                    left: r.right - 1,
                    ..r
                },
                self.colors.edge,
            );
            let sorted = index == sort.column as usize;
            if sorted {
                fill(
                    dc,
                    &RECT {
                        top: r.bottom - 2,
                        ..r
                    },
                    self.colors.accent,
                );
            }
            let style = if index == 3 { DT_RIGHT } else { DT_LEFT };
            text_at(
                dc,
                label,
                RECT {
                    left: r.left + d(8),
                    right: r.right - d(8),
                    ..r
                },
                self.fonts[usize::from(sorted)],
                self.colors.text,
                DT_SINGLELINE | DT_VCENTER | style,
            );
        }
        unsafe {
            EndPaint(header, &paint);
        }
    }

    fn hit_test(&self, coordinates: LPARAM) -> LRESULT {
        let x = coordinates as i16 as i32;
        let y = (coordinates >> 16) as i16 as i32;
        let mut r: RECT = unsafe { zeroed() };
        unsafe {
            GetWindowRect(self.window, &mut r);
        }
        if unsafe { IsZoomed(self.window) } == 0 {
            let edge = scale(RESIZE_EDGE, self.dpi);
            let left = x < r.left + edge;
            let right = x >= r.right - edge;
            let top = y < r.top + edge;
            let bottom = y >= r.bottom - edge;
            let resize = match (left, right, top, bottom) {
                (true, _, true, _) => HTTOPLEFT,
                (_, true, true, _) => HTTOPRIGHT,
                (true, _, _, true) => HTBOTTOMLEFT,
                (_, true, _, true) => HTBOTTOMRIGHT,
                (true, _, _, _) => HTLEFT,
                (_, true, _, _) => HTRIGHT,
                (_, _, true, _) => HTTOP,
                (_, _, _, true) => HTBOTTOM,
                _ => HTCLIENT,
            };
            if resize != HTCLIENT {
                return resize as isize;
            }
        }
        if y < r.top + scale(38, self.dpi) {
            HTCAPTION as isize
        } else {
            HTCLIENT as isize
        }
    }

    fn on_notify(&mut self, notification: LPARAM) -> LRESULT {
        if notification == 0 {
            return 0;
        }
        // The list-view notification is valid for the duration of WM_NOTIFY.
        let header = unsafe { &*(notification as *const NMHDR) };
        if header.hwndFrom != self.list {
            return 0;
        }
        match header.code {
            NM_CUSTOMDRAW if !self.group_headers.is_empty() => {
                let draw = unsafe { &*(notification as *const NMLVCUSTOMDRAW) };
                if draw.nmcd.dwDrawStage == CDDS_PREPAINT {
                    return CDRF_NOTIFYITEMDRAW as LRESULT;
                }
                if draw.nmcd.dwDrawStage == CDDS_ITEMPREPAINT {
                    if let Some(group) = self.group_header(draw.nmcd.dwItemSpec) {
                        let mut bounds: RECT = unsafe { zeroed() };
                        if send(
                            self.list,
                            LVM_GETITEMRECT,
                            draw.nmcd.dwItemSpec,
                            &mut bounds as *mut _ as LPARAM,
                        ) != 0
                        {
                            let width = rect(self.list).right;
                            bounds.left = scale(8, self.dpi);
                            bounds.right = width - bounds.left;
                            let row = RECT {
                                left: 0,
                                right: width,
                                ..bounds
                            };
                            let dc = draw.nmcd.hdc;
                            fill(dc, &row, self.colors.content);
                            fill(
                                dc,
                                &RECT {
                                    top: bounds.bottom - 1,
                                    ..bounds
                                },
                                self.colors.edge,
                            );
                            let color = if self.colors.contrast {
                                self.colors.text
                            } else {
                                self.colors.accent
                            };
                            text_at(
                                dc,
                                group.label(),
                                bounds,
                                self.fonts[1],
                                color,
                                DT_SINGLELINE | DT_VCENTER | DT_LEFT,
                            );
                        }
                        return CDRF_SKIPDEFAULT as LRESULT;
                    }
                }
            }
            LVN_GETDISPINFOW => {
                let display = unsafe { &mut *(notification as *mut NMLVDISPINFOW) };
                let item = &mut display.item;
                if let Some(entry) = self.entry(item.iItem as usize) {
                    if item.mask & LVIF_IMAGE != 0 {
                        item.iImage = if matches!(self.location(), Location::Computer) {
                            2
                        } else if entry.directory {
                            0
                        } else {
                            1
                        };
                    }
                    if item.mask & LVIF_TEXT != 0 && !item.pszText.is_null() && item.cchTextMax > 0
                    {
                        let other;
                        let value = match item.iSubItem {
                            0 => entry.display_name(self.settings.hide_extensions),
                            col => {
                                let field = match col {
                                    1 => entry.date(self.settings.modified_date_format),
                                    2 if self.recycling() => entry
                                        .shell_item
                                        .as_ref()
                                        .and_then(|item| item.original_location.as_ref())
                                        .map_or_else(String::new, |path| {
                                            path.display().to_string()
                                        }),
                                    2 => entry.kind(),
                                    3 => entry.display_size(),
                                    _ => String::new(),
                                };
                                other = wide(field);
                                &other[..other.len() - 1]
                            }
                        };
                        let count = value.len().min(item.cchTextMax as usize - 1);
                        unsafe {
                            copy_nonoverlapping(value.as_ptr(), item.pszText, count);
                            *item.pszText.add(count) = 0;
                        }
                    }
                } else if let Some(group) = self.group_header(item.iItem as usize) {
                    if item.mask & LVIF_IMAGE != 0 {
                        item.iImage = I_IMAGENONE;
                    }
                    if item.mask & LVIF_TEXT != 0 && !item.pszText.is_null() && item.cchTextMax > 0
                    {
                        let label = wide(if item.iSubItem == 0 {
                            group.label()
                        } else {
                            ""
                        });
                        let count = (label.len() - 1).min(item.cchTextMax as usize - 1);
                        unsafe {
                            copy_nonoverlapping(label.as_ptr(), item.pszText, count);
                            *item.pszText.add(count) = 0;
                        }
                    }
                }
            }
            LVN_ITEMCHANGED | LVN_ODSTATECHANGED => {
                if !self.selection_event {
                    self.selection_event = true;
                    unsafe {
                        PostMessageW(self.window, SELECTION_CHANGED, 0, 0);
                    }
                }
            }
            NM_DBLCLK => {
                let event = unsafe { &*(notification as *const NMITEMACTIVATE) };
                if let Ok(index) = usize::try_from(event.iItem) {
                    self.activate_row(OPEN, index);
                }
            }
            NM_RCLICK => {
                let event = unsafe { &*(notification as *const NMITEMACTIVATE) };
                if self.entry(event.iItem as usize).is_none() {
                    self.clear_selection();
                } else {
                    let index = event.iItem as usize;
                    if send(self.list, LVM_GETITEMSTATE, index, LVIS_SELECTED as isize) == 0 {
                        self.clear_selection();
                        let selected = LVITEMW {
                            state: LVIS_SELECTED | LVIS_FOCUSED,
                            stateMask: LVIS_SELECTED | LVIS_FOCUSED,
                            ..unsafe { zeroed() }
                        };
                        send(
                            self.list,
                            LVM_SETITEMSTATE,
                            index,
                            &selected as *const _ as LPARAM,
                        );
                    }
                }
                let mut point = event.ptAction;
                unsafe {
                    ClientToScreen(self.list, &mut point);
                }
                let packed = ((point.y as u16 as usize) << 16) | point.x as u16 as usize;
                unsafe {
                    PostMessageW(self.window, SHOW_MENU, CONTEXT as usize, packed as isize);
                }
            }
            LVN_COLUMNCLICK => {
                let event = unsafe { &*(notification as *const NMLISTVIEW) };
                if let Some(id) =
                    [SORT_NAME, SORT_MODIFIED, SORT_KIND, SORT_SIZE].get(event.iSubItem as usize)
                {
                    self.command(*id);
                }
            }
            _ => (),
        }
        0
    }
}

struct PopupMenu(HMENU);

impl PopupMenu {
    fn new() -> Option<Self> {
        let handle = unsafe { CreatePopupMenu() };
        (!handle.is_null()).then_some(Self(handle))
    }

    fn append(&self, items: Vec<MenuItem>, painted: bool, rows: &mut Vec<MenuItem>) {
        for (index, mut item) in items.into_iter().enumerate() {
            let children = std::mem::take(&mut item.children);
            let child = if children.is_empty() {
                None
            } else {
                let Some(menu) = Self::new() else {
                    continue;
                };
                menu.append(children, painted, rows);
                Some(menu)
            };
            let mut label = wide(&item.label);
            let info = MENUITEMINFOW {
                cbSize: size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_FTYPE
                    | MIIM_STATE
                    | MIIM_STRING
                    | MIIM_ID
                    | MIIM_DATA
                    | if child.is_some() { MIIM_SUBMENU } else { 0 },
                fType: (if painted { MFT_OWNERDRAW } else { MFT_STRING })
                    | if item.id == 0 { MFT_SEPARATOR } else { 0 },
                fState: (if item.enabled && item.id != 0 {
                    MFS_ENABLED
                } else {
                    MFS_DISABLED
                }) | if item.checked {
                    MFS_CHECKED
                } else {
                    MFS_UNCHECKED
                },
                wID: item.id as u32,
                dwItemData: rows.len(),
                dwTypeData: label.as_mut_ptr(),
                hSubMenu: child.as_ref().map_or(null_mut(), |menu| menu.0),
                ..unsafe { zeroed() }
            };
            if unsafe { InsertMenuItemW(self.0, index as u32, 1, &info) } != 0 {
                if let Some(menu) = child {
                    // The parent now owns the submenu.
                    std::mem::forget(menu);
                }
                rows.push(item);
            }
        }
    }
}

impl Drop for PopupMenu {
    fn drop(&mut self) {
        unsafe {
            DestroyMenu(self.0);
        }
    }
}

#[cfg(test)]
mod context_menu_tests {
    use super::*;

    #[test]
    fn native_menu_keeps_direct_commands_and_nested_commands_addressable() {
        for painted in [false, true] {
            let menu = PopupMenu::new().expect("create native test menu");
            let mut rows = Vec::new();
            menu.append(
                vec![
                    MenuItem::new(RENAME, "Rename\tF2"),
                    MenuItem::separator(),
                    MenuItem::submenu(NEW, "New", vec![MenuItem::new(NEW_FOLDER, "Folder")]),
                ],
                painted,
                &mut rows,
            );
            unsafe {
                assert_eq!(GetMenuItemCount(menu.0), 3);
                assert_eq!(GetMenuItemID(menu.0, 0), RENAME as u32);
                let mut separator = MENUITEMINFOW {
                    cbSize: size_of::<MENUITEMINFOW>() as u32,
                    fMask: MIIM_FTYPE,
                    ..zeroed()
                };
                assert_ne!(GetMenuItemInfoW(menu.0, 1, 1, &mut separator), 0);
                assert_ne!(separator.fType & MFT_SEPARATOR, 0);
                let new_menu = GetSubMenu(menu.0, 2);
                assert!(!new_menu.is_null());
                assert_eq!(GetMenuItemID(new_menu, 0), NEW_FOLDER as u32);
                let mut info = MENUITEMINFOW {
                    cbSize: size_of::<MENUITEMINFOW>() as u32,
                    fMask: MIIM_DATA | MIIM_FTYPE,
                    ..zeroed()
                };
                assert_ne!(GetMenuItemInfoW(new_menu, 0, 1, &mut info), 0);
                assert_eq!(rows[info.dwItemData].id, NEW_FOLDER);
                assert_eq!(info.fType, if painted { MFT_OWNERDRAW } else { MFT_STRING });
            }
        }
    }
}

fn show_menu(app: &RefCell<App>, id: u16, coordinates: LPARAM) {
    let Ok(mut state) = app.try_borrow_mut() else {
        return;
    };
    let items = state.menu(id);
    let painted = !state.colors.contrast;
    let menu_color = state.colors.chrome;
    let dark_border = (state.colors.dark && painted).then_some(state.colors.edge);
    let window = state.window;
    let selected_targets = |state: &App| {
        state
            .selected_indices()
            .into_iter()
            .filter_map(|index| state.entry(index).map(|row| row.path.clone()))
            .collect::<std::collections::HashSet<_>>()
    };
    let target = (id == CONTEXT).then(|| (state.location().clone(), selected_targets(&state)));
    let mut anchor: POINT = unsafe { zeroed() };
    unsafe {
        if id == CONTEXT {
            if coordinates == -1 {
                let mut box_: RECT = zeroed();
                let row = send(
                    state.list,
                    LVM_GETNEXTITEM,
                    usize::MAX,
                    LVNI_FOCUSED as isize,
                );
                if row >= 0 {
                    send(
                        state.list,
                        LVM_GETITEMRECT,
                        row as usize,
                        &mut box_ as *mut _ as LPARAM,
                    );
                }
                anchor.x = box_.left + 12;
                anchor.y = box_.bottom;
                ClientToScreen(state.list, &mut anchor);
            } else {
                anchor.x = coordinates as i16 as i32;
                anchor.y = (coordinates >> 16) as i16 as i32;
            }
        } else {
            let mut bounds = zeroed();
            let source = if id == PATH && coordinates != 0 {
                coordinates as HWND
            } else {
                state.button(id)
            };
            GetWindowRect(source, &mut bounds);
            anchor.x = bounds.left;
            anchor.y = bounds.bottom;
        }
    }
    drop(state);
    if items.is_empty() {
        return;
    }
    unsafe {
        let Some(menu) = PopupMenu::new() else {
            return;
        };
        let brush = if painted {
            CreateSolidBrush(menu_color)
        } else {
            null_mut()
        };
        if !brush.is_null() {
            let info = MENUINFO {
                cbSize: size_of::<MENUINFO>() as u32,
                fMask: MIM_BACKGROUND | MIM_APPLYTOSUBMENUS,
                hbrBack: brush,
                ..zeroed()
            };
            let mut rows = Vec::new();
            menu.append(items, painted, &mut rows);
            SetMenuInfo(menu.0, &info);
            app.borrow_mut().popup_items = rows;
        } else {
            let mut rows = Vec::new();
            menu.append(items, false, &mut rows);
        }
        let border = dark_border.map(PopupBorder::apply);
        let selected = TrackPopupMenu(
            menu.0,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            anchor.x,
            anchor.y,
            0,
            window,
            null(),
        );
        drop(border);
        drop(menu);
        if painted {
            app.borrow_mut().popup_items.clear();
        }
        if !brush.is_null() {
            DeleteObject(brush);
        }
        if selected > 0 {
            let mut state = app.borrow_mut();
            let unchanged = target.is_none_or(|(location, paths)| {
                state.location() == &location && selected_targets(&state) == paths
            });
            if unchanged {
                state.command(selected as u16);
            }
        }
    }
}

fn is_zip(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
}

fn key(app: &RefCell<App>, message: &MSG) -> bool {
    if !matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN) {
        return false;
    }
    let Ok(mut app) = app.try_borrow_mut() else {
        return false;
    };
    let focus = unsafe { GetFocus() };
    let edit = if app.edit.is_some() {
        app.edit_name
    } else {
        null_mut()
    };
    let typing = focus == app.address || focus == app.search || focus == edit;
    let down = |key: u16| unsafe { GetKeyState(key as i32) < 0 };
    let ctrl = down(VK_CONTROL);
    let alt = down(VK_MENU);
    let shift = down(VK_SHIFT);
    let vk = message.wParam as u16;
    if vk == VK_TAB && !ctrl && !alt {
        let next = unsafe { GetNextDlgTabItem(app.window, focus, i32::from(shift)) };
        if !next.is_null() {
            unsafe {
                SetFocus(next);
            }
        }
        return true;
    }
    let shortcut = match (vk, ctrl, alt, shift) {
        (VK_OEM_COMMA, true, false, _) => OPEN_SETTINGS,
        (VK_RETURN, false, false, _) if focus == app.navigation => {
            app.select_sidebar();
            return true;
        }
        (VK_RETURN, false, false, _) if focus == app.button(WIN_E_WINDOWS) => WIN_E_WINDOWS,
        (VK_RETURN, false, false, _) if focus == app.button(WIN_E_BETTER) => WIN_E_BETTER,
        (VK_RETURN, false, false, _) if focus == app.button(DATE_FORMAT_DAY_FIRST) => {
            DATE_FORMAT_DAY_FIRST
        }
        (VK_RETURN, false, false, _) if focus == app.button(DATE_FORMAT_MONTH_FIRST) => {
            DATE_FORMAT_MONTH_FIRST
        }
        (0x4C, true, false, _) | (VK_D, false, true, _) => {
            app.address_focus(true);
            return true;
        }
        (VK_ESCAPE, false, false, _) if focus == app.address => {
            app.address_focus(false);
            unsafe {
                SetFocus(app.list);
            }
            return true;
        }
        (0x46, true, false, _) | (VK_F3, false, false, _)
            if *app.location() != Location::Settings =>
        {
            unsafe {
                SetFocus(app.search);
            }
            send(app.search, EM_SETSEL, 0, -1);
            return true;
        }
        (VK_P, false, true, true) => DETAILS,
        (VK_LEFT, false, true, _) => BACK,
        (VK_RIGHT, false, true, _) => FORWARD,
        (VK_UP, false, true, _) => UP,
        (VK_F5, _, false, _) => REFRESH,
        (VK_TAB, true, false, false) => NEXT_TAB,
        (VK_TAB, true, false, true) => PREVIOUS_TAB,
        (0x54, true, false, false) => NEW_TAB,
        (0x57, true, false, false) => CLOSE_TAB,
        (0x4E, true, false, false) => NEW_WINDOW,
        (0x4E, true, false, true) => NEW_FOLDER,
        (VK_RETURN, false, false, _) if focus == app.address => ENTER_PATH,
        (VK_RETURN, false, false, _) if focus == app.search => ENTER_SEARCH,
        (VK_RETURN, false, false, _) if focus == edit || focus == app.button(ACCEPT) => ACCEPT,
        (VK_RETURN, false, false, _) if focus == app.button(CANCEL) => CANCEL,
        (VK_RETURN, false, false, _) if focus == app.list => OPEN,
        (VK_RETURN, false, true, _) if focus == app.list => PROPERTIES,
        (VK_ESCAPE, false, false, _) if app.edit.is_some() => CANCEL,
        (VK_ESCAPE, false, false, _)
            if app.scanning && matches!(app.location(), Location::Search { .. }) =>
        {
            STOP_SEARCH
        }
        (0x41, true, false, false) if !typing && *app.location() != Location::Home => SELECT_ALL,
        (0x58, true, false, false) if !typing => CUT,
        (0x43, true, false, false) if !typing => COPY,
        (0x56, true, false, false) if !typing => PASTE,
        (VK_DELETE, false, false, false) if focus == app.list => DELETE,
        (VK_DELETE, false, false, true) if focus == app.list => PERMANENT,
        (VK_F2, false, false, _) if focus == app.list => RENAME,
        (VK_BACK, false, false, false) if focus == app.list => BACK,
        (VK_APPS, false, false, _) | (VK_F10, false, false, true) if focus == app.list => {
            unsafe {
                PostMessageW(app.window, SHOW_MENU, CONTEXT as usize, -1);
            }
            return true;
        }
        _ => return false,
    };
    app.command(shortcut);
    true
}

// GWLP_USERDATA borrows the RefCell owned by run(). It outlives the message pump.
unsafe extern "system" fn procedure(window: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if matches!(message, WM_NCCALCSIZE | WM_NCPAINT) {
        return 0;
    }
    if message == WM_NCACTIVATE {
        return unsafe { DefWindowProcW(window, message, wp, -1) };
    }
    if matches!(
        message,
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX
    ) {
        if let Some(colors) = CONTROL_COLORS.get() {
            let id = unsafe { GetDlgCtrlID(lp as HWND) } as u16;
            let surface = match id {
                STATUS => 0,
                NAVIGATION => 2,
                ADDRESS | SEARCH => 3,
                _ => 1,
            };
            unsafe {
                SetTextColor(
                    wp as HDC,
                    if matches!(id, STATUS | SUMMARY) {
                        colors.muted
                    } else {
                        colors.text
                    },
                );
                SetBkColor(wp as HDC, colors.surfaces[surface]);
            }
            return colors.brushes[surface] as LRESULT;
        }
    }
    let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *const RefCell<App>;
    if pointer.is_null() {
        return unsafe { DefWindowProcW(window, message, wp, lp) };
    }
    let cell = unsafe { &*pointer };
    if message == BENCH_STARTED {
        let mut bench = cell.borrow_mut().bench.take();
        if let Some(bench) = &mut bench {
            bench.started(window);
            if bench.done() {
                unsafe {
                    PostMessageW(window, WM_CLOSE, 0, 0);
                }
            }
        }
        cell.borrow_mut().bench = bench;
        return 0;
    }
    if message == BENCH_PRESENT {
        let should_paint = cell
            .try_borrow()
            .is_ok_and(|app| app.bench.as_ref().is_some_and(Bench::is_running));
        if should_paint {
            benchmark::present(window);
            if let Ok(mut app) = cell.try_borrow_mut() {
                let (rows, complete) = (app.visible_count(), !app.scanning);
                if let Some(bench) = &mut app.bench {
                    bench.painted(rows, complete);
                    if complete {
                        unsafe {
                            PostMessageW(
                                window,
                                if bench.done() {
                                    WM_CLOSE
                                } else {
                                    BENCH_NAVIGATE
                                },
                                0,
                                0,
                            );
                        }
                    }
                }
            }
        }
        return 0;
    }
    if message == SHOW_MENU {
        show_menu(cell, wp as u16, lp);
        return 0;
    }
    if message == WM_DESTROY {
        unsafe {
            PostQuitMessage(0);
        }
        return 0;
    }
    if message == WM_NCDESTROY {
        unsafe {
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
        }
        return unsafe { DefWindowProcW(window, message, wp, lp) };
    }
    let Ok(mut app) = cell.try_borrow_mut() else {
        return unsafe { DefWindowProcW(window, message, wp, lp) };
    };
    match message {
        WM_COMMAND => {
            if wp as u16 == NAVIGATION && wp >> 16 == LBN_SELCHANGE as usize {
                app.select_sidebar();
                return 0;
            }
            if wp as u16 == ADDRESS && wp >> 16 == EN_KILLFOCUS as usize {
                app.address_focus(false);
                return 0;
            }
            if wp >> 16 != 0 {
                return 0;
            }
            let id = wp as u16;
            if MENU_IDS.contains(&id) {
                unsafe {
                    PostMessageW(window, SHOW_MENU, id as usize, lp);
                }
            } else {
                app.command(id);
            }
            0
        }
        BENCH_NAVIGATE => {
            if let Some(path) = app.bench.as_mut().and_then(Bench::begin_navigation) {
                app.go(Location::Folder(path));
            }
            0
        }
        WAKE => {
            let began = app
                .bench
                .as_ref()
                .filter(|bench| bench.is_running())
                .map(|_| std::time::Instant::now());
            app.events();
            if let Some(bench) = &mut app.bench {
                if let Some(began) = began {
                    bench.dispatch_elapsed(began);
                }
            }
            if let Some(bench) = &app.bench {
                if bench.is_running()
                    && (!app.scanning || (!app.visible.is_empty() && bench.needs_first_paint()))
                {
                    unsafe {
                        PostMessageW(window, BENCH_PRESENT, 0, 0);
                    }
                } else if !bench.is_running() && !bench.done() && app.booted && !app.scanning {
                    unsafe {
                        PostMessageW(window, BENCH_NAVIGATE, 0, 0);
                    }
                }
            }
            0
        }
        SELECTION_CHANGED => {
            app.selection_event = false;
            app.clear_group_selection();
            app.note = None;
            app.update_preview();
            app.status();
            0
        }
        CHANGE if wp == app.id as usize => {
            unsafe {
                SetTimer(window, REFRESH_TIMER, 250, None);
            }
            0
        }
        WM_TIMER if wp == REFRESH_TIMER => {
            if !app.scanning && app.edit.is_none() {
                app.reload(true);
            }
            0
        }
        WM_TIMER if wp == SETTINGS_TIMER => {
            app.save();
            0
        }
        WM_TIMER if wp == GROUP_TIMER => {
            unsafe { KillTimer(window, GROUP_TIMER) };
            if !app.scanning && app.group_by() == GroupBy::DateModified {
                app.rebuild_grouping();
            }
            0
        }
        WM_TIMECHANGE => {
            if !app.scanning && app.group_by() == GroupBy::DateModified {
                app.rebuild_grouping();
            }
            0
        }
        WM_PAINT => {
            app.paint();
            0
        }
        WM_ERASEBKGND => 1,
        WM_NCHITTEST => app.hit_test(lp),
        WM_LBUTTONDOWN => {
            let x = lp as i16 as i32;
            let y = (lp >> 16) as i16 as i32;
            let d = |v| scale(v, app.dpi);
            let width = rect(window).right;
            let (left, right, _) = app.address_bounds(width);
            if (left..right).contains(&x) && (d(43)..d(74)).contains(&y) {
                app.address_focus(true);
            }
            0
        }
        HOVER => {
            let control = wp as HWND;
            if lp != 0 && app.hover == control {
                app.hover = null_mut();
                unsafe {
                    InvalidateRect(control, null(), 0);
                }
            } else if lp == 0 && app.hover != control {
                let old = std::mem::replace(&mut app.hover, control);
                unsafe {
                    if !old.is_null() {
                        InvalidateRect(old, null(), 0);
                    }
                    InvalidateRect(control, null(), 0);
                }
            }
            0
        }
        WM_SIZE | WM_EXITSIZEMOVE => {
            app.layout();
            0
        }
        WM_THEMECHANGED | WM_SETTINGCHANGE | WM_SYSCOLORCHANGE => {
            app.apply_theme();
            0
        }
        WM_DPICHANGED => {
            app.apply_dpi(wp as u16 as u32);
            let bounds = unsafe { &*(lp as *const RECT) };
            unsafe {
                SetWindowPos(
                    window,
                    null_mut(),
                    bounds.left,
                    bounds.top,
                    bounds.right - bounds.left,
                    bounds.bottom - bounds.top,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
            app.layout();
            0
        }
        WM_GETMINMAXINFO => {
            let info = unsafe { &mut *(lp as *mut MINMAXINFO) };
            info.ptMinTrackSize.x = scale(552, app.dpi);
            info.ptMinTrackSize.y = scale(450, app.dpi);
            unsafe {
                let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
                let mut bounds: MONITORINFO = zeroed();
                bounds.cbSize = size_of::<MONITORINFO>() as u32;
                if GetMonitorInfoW(monitor, &mut bounds) != 0 {
                    info.ptMaxPosition.x = bounds.rcWork.left - bounds.rcMonitor.left;
                    info.ptMaxPosition.y = bounds.rcWork.top - bounds.rcMonitor.top;
                    info.ptMaxSize.x = bounds.rcWork.right - bounds.rcWork.left;
                    info.ptMaxSize.y = bounds.rcWork.bottom - bounds.rcWork.top;
                }
            }
            0
        }
        WM_NOTIFY => app.on_notify(lp),
        WM_MEASUREITEM => {
            let measure = unsafe { &mut *(lp as *mut MEASUREITEMSTRUCT) };
            if measure.CtlType == ODT_MENU {
                app.measure_menu(measure);
                1
            } else {
                0
            }
        }
        WM_DRAWITEM => {
            let draw = unsafe { &*(lp as *const DRAWITEMSTRUCT) };
            let id = draw.CtlID as u16;
            if draw.CtlType == ODT_MENU {
                app.draw_menu(draw);
            } else if id == NAVIGATION {
                app.draw_sidebar(draw);
            } else if (DRIVE_START..DRIVE_START + DRIVE_LIMIT as u16).contains(&id) {
                app.draw_drive(draw, (id - DRIVE_START) as usize);
            } else {
                let style = app.button_style(id, draw.hwndItem);
                draw_button(
                    draw,
                    &app.colors,
                    app.dpi,
                    app.fonts[usize::from(style.active)],
                    style,
                );
            }
            1
        }
        WM_DROPFILES => {
            let handle = wp as HDROP;
            let paths = unsafe { shell::dropped_paths(handle) };
            unsafe {
                DragFinish(handle);
            }
            if let Some(target) = app.dest() {
                if !paths.is_empty() {
                    app.operation(Operation::Transfer {
                        paths,
                        target,
                        moving: false,
                    });
                }
            } else {
                app.note = Some("Open a destination folder before dropping files.".into());
                app.status();
            }
            0
        }
        WM_CLOSE => {
            if matches!(app.hotkey_request, Some(HotkeyRequest::Change(_))) {
                return 0;
            }
            if app.busy > 0 {
                let message = "Wait for Windows to finish the file operation before closing.";
                if *app.location() == Location::Settings {
                    if let Some(controls) = app.settings_controls {
                        text(controls.state, message);
                    }
                } else {
                    app.note = Some(message.into());
                    app.status();
                }
                return 0;
            }
            unsafe {
                KillTimer(window, SETTINGS_TIMER);
                DestroyWindow(window);
            }
            0
        }
        _ => {
            // Default handling can synchronously dispatch size and command messages.
            drop(app);
            unsafe { DefWindowProcW(window, message, wp, lp) }
        }
    }
}

unsafe extern "system" fn header_proc(
    window: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if msg == WM_PAINT {
        let list = unsafe { GetParent(window) };
        let main = unsafe { GetParent(list) };
        let pointer = unsafe { GetWindowLongPtrW(main, GWLP_USERDATA) } as *const RefCell<App>;
        if !pointer.is_null() {
            if let Ok(app) = unsafe { &*pointer }.try_borrow() {
                if !app.colors.contrast {
                    app.draw_header();
                    return 0;
                }
            }
        }
    }
    unsafe { DefSubclassProc(window, msg, wp, lp) }
}

unsafe extern "system" fn button_proc(
    window: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_MOUSEMOVE {
            let parent = GetParent(window);
            let pointer = GetWindowLongPtrW(parent, GWLP_USERDATA) as *const RefCell<App>;
            if !pointer.is_null()
                && (&*pointer)
                    .try_borrow()
                    .is_ok_and(|app| app.hover != window)
            {
                let mut tracking: TRACKMOUSEEVENT = zeroed();
                tracking.cbSize = size_of::<TRACKMOUSEEVENT>() as u32;
                tracking.dwFlags = TME_LEAVE;
                tracking.hwndTrack = window;
                TrackMouseEvent(&mut tracking);
                PostMessageW(parent, HOVER, window as usize, 0);
            }
        } else if msg == WM_MOUSELEAVE {
            PostMessageW(GetParent(window), HOVER, window as usize, 1);
        }
        DefSubclassProc(window, msg, wp, lp)
    }
}

unsafe extern "system" fn group_list_proc(
    window: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        let key = wp as u16;
        let move_focus = msg == WM_KEYDOWN
            && matches!(key, VK_UP | VK_DOWN | VK_HOME | VK_END | VK_PRIOR | VK_NEXT);
        let click = matches!(msg, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK);
        if !move_focus && !click {
            return DefSubclassProc(window, msg, wp, lp);
        }
        let pointer = GetWindowLongPtrW(GetParent(window), GWLP_USERDATA) as *const RefCell<App>;
        if pointer.is_null() {
            return DefSubclassProc(window, msg, wp, lp);
        }
        let app = &*pointer;
        let grouped = app
            .try_borrow()
            .is_ok_and(|app| !app.group_headers.is_empty());
        if !grouped {
            return DefSubclassProc(window, msg, wp, lp);
        }
        if click {
            let mut hit = LVHITTESTINFO {
                pt: POINT {
                    x: lp as i16 as i32,
                    y: (lp >> 16) as i16 as i32,
                },
                ..zeroed()
            };
            let row = send(window, LVM_SUBITEMHITTEST, 0, &mut hit as *mut _ as LPARAM);
            if app
                .try_borrow()
                .is_ok_and(|app| app.group_header(row as usize).is_some())
            {
                SetFocus(window);
                return 0;
            }
        }
        let result = DefSubclassProc(window, msg, wp, lp);
        if move_focus {
            let row = send(window, LVM_GETNEXTITEM, usize::MAX, LVNI_FOCUSED as isize);
            if app
                .try_borrow()
                .is_ok_and(|app| app.group_header(row as usize).is_some())
            {
                let next = if row == 0 || matches!(key, VK_DOWN | VK_NEXT | VK_HOME) {
                    VK_DOWN
                } else {
                    VK_UP
                };
                DefSubclassProc(window, WM_KEYDOWN, next as WPARAM, lp);
            }
            if let Ok(app) = app.try_borrow() {
                app.clear_group_selection();
            }
        }
        result
    }
}

struct SettingsLease(windows_sys::Win32::Foundation::HANDLE);

impl SettingsLease {
    fn acquire() -> Option<Self> {
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
        if better_explorer::model::settings_path().is_none()
            || std::env::args_os().any(|arg| arg == "--new-window")
        {
            return None;
        }
        let name = wide(r"Local\BetterExplorer.Settings.1");
        unsafe {
            let handle = CreateMutexW(null(), 0, name.as_ptr());
            if handle.is_null() {
                return None;
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(handle);
                return None;
            }
            Some(Self(handle))
        }
    }
}

impl Drop for SettingsLease {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

pub fn run(path: Option<PathBuf>) -> Result<(), std::io::Error> {
    unsafe {
        let settings = SettingsLease::acquire();
        let controls = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES | ICC_STANDARD_CLASSES,
        };
        InitCommonControlsEx(&controls);
        let module = GetModuleHandleW(null());
        let class = wide("BetterExplorerWindow");
        let icon = LoadImageW(
            module,
            std::ptr::without_provenance(1),
            IMAGE_ICON,
            32,
            32,
            LR_SHARED,
        );
        let small_icon = LoadImageW(
            module,
            std::ptr::without_provenance(1),
            IMAGE_ICON,
            16,
            16,
            LR_SHARED,
        );
        let info = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(procedure),
            hInstance: module,
            hIcon: icon,
            hIconSm: small_icon,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        if RegisterClassExW(&info) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let window = CreateWindowExW(
            WS_EX_APPWINDOW,
            class.as_ptr(),
            wide("BetterExplorer").as_ptr(),
            WS_POPUP
                | WS_THICKFRAME
                | WS_SYSMENU
                | WS_MINIMIZEBOX
                | WS_MAXIMIZEBOX
                | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1136,
            732,
            null_mut(),
            null_mut(),
            module,
            null(),
        );
        if window.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let app = Box::new(RefCell::new(App::create(window, path, settings.is_some())));
        SetWindowLongPtrW(
            window,
            GWLP_USERDATA,
            (&*app as *const RefCell<App>) as isize,
        );
        {
            let app = app.borrow();
            if !app.header.is_null() {
                SetWindowSubclass(app.header, Some(header_proc), 1, 0);
            }
            SetWindowSubclass(app.list, Some(group_list_proc), 2, 0);
            for &(_, button) in &app.buttons {
                SetWindowSubclass(button, Some(button_proc), 1, 0);
            }
        }
        let without_activation = std::env::var_os("BETTEREXPLORER_BENCH_NOACTIVATE").is_some();
        ShowWindow(
            window,
            if without_activation {
                SW_SHOWNOACTIVATE
            } else {
                SW_SHOWNORMAL
            },
        );
        UpdateWindow(window);
        if app.borrow().bench.is_some() {
            PostMessageW(window, BENCH_STARTED, 0, 0);
        }
        app.borrow_mut().start();
        let mut message = zeroed();
        loop {
            let code = GetMessageW(&mut message, null_mut(), 0, 0);
            if code == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if code == 0 {
                break;
            }
            if !key(&app, &message) {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let mut app = app.into_inner();
        app.saved.take();
        if let Some(worker) = app.save_worker.take() {
            let _ = worker.join();
        }
        if let Some(bench) = &app.bench {
            bench.save()?;
        }
        if app.booted && app.save_enabled {
            let _ = write_settings(&app.settings_snapshot());
        }
        Ok(())
    }
}

#[cfg(test)]
mod group_tests {
    use super::*;
    use windows_sys::Win32::{
        Foundation::FILETIME, System::SystemInformation::GetSystemTimeAsFileTime,
    };

    #[test]
    fn keyboard_moves_across_date_headings_without_selecting_them() {
        unsafe {
            let controls = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_LISTVIEW_CLASSES | ICC_STANDARD_CLASSES,
            };
            assert_ne!(InitCommonControlsEx(&controls), 0);
            let window = CreateWindowExW(
                0,
                wide("Static").as_ptr(),
                null(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                900,
                700,
                null_mut(),
                null_mut(),
                null_mut(),
                null(),
            );
            assert!(!window.is_null());
            let app = Box::new(RefCell::new(App::create(
                window,
                Some(r"C:\Example".into()),
                false,
            )));
            SetWindowLongPtrW(
                window,
                GWLP_USERDATA,
                (&*app as *const RefCell<App>) as isize,
            );
            let list = app.borrow().list;
            assert_ne!(SetWindowSubclass(list, Some(group_list_proc), 2, 0), 0);

            let mut now: FILETIME = zeroed();
            GetSystemTimeAsFileTime(&mut now);
            let now = u64::from(now.dwHighDateTime) << 32 | u64::from(now.dwLowDateTime);
            let entry = |name: &str, directory, modified| {
                Arc::new(Entry {
                    path: PathBuf::from(name),
                    name: wide(name),
                    extension: Vec::new(),
                    directory,
                    size: 0,
                    modified,
                    location: false,
                    capacity: None,
                    shell_item: None,
                })
            };
            {
                let mut app = app.borrow_mut();
                app.scanning = false;
                app.rows = vec![
                    entry("newer.txt", false, now),
                    entry("older-folder", true, now - 1_728_000_000_000),
                ];
                app.rebuild_visible();
                assert_eq!(app.visible_count(), 2);
                assert_eq!(app.group_header(0), Some(DateGroup::Today));
                assert!(app.group_header(2).is_some());
            }
            let selected = LVITEMW {
                state: LVIS_SELECTED | LVIS_FOCUSED,
                stateMask: LVIS_SELECTED | LVIS_FOCUSED,
                ..zeroed()
            };
            send(list, LVM_SETITEMSTATE, 1, &selected as *const _ as LPARAM);
            send(list, WM_KEYDOWN, VK_DOWN as WPARAM, 1);
            {
                let mut app = app.borrow_mut();
                assert_eq!(app.first_selected().unwrap().label(), "older-folder");
                assert_eq!(
                    send(list, LVM_GETITEMSTATE, 2, selected.stateMask as LPARAM),
                    0
                );
                app.command(GROUP_NONE);
                assert_eq!(app.visible.len(), 2);
                assert_eq!(app.first_selected().unwrap().label(), "older-folder");
            }
            DestroyWindow(window);
        }
    }
}
