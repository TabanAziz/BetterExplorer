use crate::{
    benchmark::{self, Bench},
    chrome::*,
    preview::{self, Content},
};
use better_explorer::{
    model::{
        from_wide, read_settings, safe_name, wide, write_settings, History, Location, Settings,
        ThemePreference,
    },
    shell::{self, Operation},
    work::{self, Column, Entry, Request, ScanEvent, Scanner, Sort},
};
use std::{
    cell::RefCell,
    collections::VecDeque,
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
const PLACE_START: u16 = 172;
const PINNED: u16 = 179;
const NETWORK: u16 = 180;
const RECYCLE: u16 = 181;
const LIBRARIES: u16 = 182;
const TAB_START: u16 = 300;
const BREADCRUMB_START: u16 = 340;
const PIN_START: u16 = 430;
const TAB_LIMIT: usize = 10;
const FRAME_MIN: u16 = 165;
const FRAME_MAX: u16 = 166;
const FRAME_CLOSE: u16 = 167;
const QUICK_DETAILS: u16 = 168;
const QUICK_ICONS: u16 = 169;
const CRUMB_START: u16 = 520;
const CRUMB_LIMIT: usize = 7;
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
    (NEW, "New  ▾", 76),
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
];
const PLACES: [&str; 6] = [
    "Desktop",
    "Downloads",
    "Documents",
    "Pictures",
    "Music",
    "Videos",
];
const MENU_IDS: [u16; 8] = [NEW, SORT, VIEW, FILTER, MORE, PATH, PINNED, CONTEXT];

enum Event {
    Scan(ScanEvent),
    Preview(u64, Option<Content>),
    Boot(Settings, Vec<(PathBuf, &'static str)>),
    SettingsError(String),
    Operation {
        result: Result<String, String>,
        refresh: bool,
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
}

impl Filter {
    fn shows(self, item: &Entry) -> bool {
        match self {
            Self::All => true,
            Self::Folders => item.directory,
            Self::Files => !item.directory,
        }
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

struct App {
    window: HWND,
    buttons: Vec<(u16, HWND)>,
    list: HWND,
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
    fonts: [HFONT; 2],
    brushes: [HBRUSH; 2],
    dpi: u32,
    colors: Palette,
    tabs: Vec<Tab>,
    active: usize,
    settings: Settings,
    places: Vec<(PathBuf, &'static str)>,
    pinned: Vec<PathBuf>,
    crumbs: Vec<PathBuf>,
    crumb_slots: Vec<Location>,
    crumb_display: Vec<(String, i32)>,
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
            (HOME, "Home"),
            (COMPUTER, "This PC"),
            (PINNED, "Pinned folders  ▾"),
            (NETWORK, "Network"),
            (RECYCLE, "Recycle Bin"),
            (LIBRARIES, "Libraries"),
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
        for (offset, place) in PLACES.iter().enumerate() {
            let id = PLACE_START + offset as u16;
            buttons.push((id, button(window, place, id)));
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
                WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
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
        let dpi = unsafe { GetDpiForWindow(window) }.max(96);
        let colors = Palette::read(ThemePreference::System);
        let mut app = Self {
            window,
            buttons,
            list,
            address: input(ADDRESS),
            search: input(SEARCH),
            title: label(0),
            summary: label(0),
            detail_title: label(0),
            detail_body: label(0),
            status: label(STATUS),
            edit_label: label(0),
            edit_name: input(NAME),
            images: [0; 2],
            icon_size: 52,
            fonts: [null_mut(); 2],
            brushes: [null_mut(); 2],
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
            places: Vec::new(),
            pinned: Vec::new(),
            crumbs: Vec::new(),
            crumb_slots: Vec::new(),
            crumb_display: Vec::new(),
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
                    let result = shell::run(owner, operation);
                    publish(
                        &operation_queue,
                        owner,
                        Event::Operation { result, refresh },
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
        unsafe {
            SetFocus(self.list);
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

    fn apply_dpi(&mut self, dpi: u32) {
        self.dpi = dpi;
        let old = std::mem::replace(&mut self.fonts, [font(dpi, false), font(dpi, true)]);
        for (_, control) in self.buttons.iter().copied() {
            send(control, WM_SETFONT, self.fonts[0] as usize, 1);
        }
        for control in [
            self.list,
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
        for handle in old {
            if !handle.is_null() {
                unsafe {
                    DeleteObject(handle);
                }
            }
        }
        for (index, width) in [325, 143, 115, 88].into_iter().enumerate() {
            send(
                self.list,
                LVM_SETCOLUMNWIDTH,
                index,
                scale(width, dpi) as isize,
            );
        }
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
            ]
        });
        set_theme(self.window, self.list, &self.colors);
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

    fn drive_cards(&self) -> bool {
        if !matches!(self.location(), Location::Computer) || self.tab().view != VIEW_CARDS {
            return false;
        }
        let bounds = rect(self.window);
        let d = |value| scale(value, self.dpi);
        let right = bounds.right - if self.settings.details { d(226) } else { 0 };
        let columns = ((right - d(174)) / d(255)).max(1) as usize;
        let lines = ((bounds.bottom - d(186)) / d(64)).max(1) as usize;
        self.visible.len() <= DRIVE_LIMIT.min(columns * lines)
    }

    fn layout(&self) {
        let bounds = rect(self.window);
        let w = bounds.right;
        let h = bounds.bottom;
        let d = |value| scale(value, self.dpi);
        let pos = |id, x, y, width, height| move_to(self.button(id), x, y, width, height);
        let margin = d(10);
        let tab_width = ((w - d(205)) / self.tabs.len() as i32).clamp(d(38), d(205));
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
                pos(id, x, d(5), tab_width - close, d(29));
                if close != 0 {
                    pos(CLOSE_TAB, x + tab_width - close, d(5), close, d(29));
                }
                x += tab_width + d(2);
            }
        }
        pos(NEW_TAB, x + d(4), d(5), d(32), d(29));
        for (index, id) in [FRAME_CLOSE, FRAME_MAX, FRAME_MIN].into_iter().enumerate() {
            pos(id, w - d(44 * (index as i32 + 1)), 0, d(44), d(35));
        }

        x = margin;
        for id in [BACK, FORWARD, UP, REFRESH] {
            pos(id, x, d(43), d(36), d(30));
            x += d(39);
        }
        let search_width = (w / 4).clamp(d(158), d(280));
        let address_x = x + d(4);
        let address_width = w - address_x - search_width - d(28);
        let address_right = address_x + address_width;
        pos(PATH, address_x, d(44), d(32), d(28));
        move_to(
            self.address,
            address_x + d(35),
            d(46),
            address_width - d(42),
            d(24),
        );
        set_visible(self.address, self.editing_address);
        let mut crumb_x = address_x + d(33);
        let last = self.crumb_display.len().saturating_sub(1);
        let current_width = self
            .crumb_display
            .get(last)
            .map_or(0, |(_, width)| d((*width).min(158)));
        for offset in 0..CRUMB_LIMIT {
            let id = CRUMB_START + offset as u16;
            let control = self.button(id);
            if control.is_null() {
                continue;
            }
            let available = address_right - crumb_x - d(8);
            let width = self
                .crumb_display
                .get(offset)
                .map(|(_, width)| d((*width).min(158)))
                .unwrap_or(0);
            let keep = offset == last || available >= width + current_width;
            let shown = !self.editing_address && width != 0 && keep && available >= d(46);
            set_visible(control, shown);
            if shown {
                let width = width.min(available);
                pos(id, crumb_x, d(44), width, d(28));
                crumb_x += width;
            }
        }
        move_to(
            self.search,
            w - search_width - margin,
            d(46),
            search_width - d(9),
            d(25),
        );
        x = margin;
        for &(id, _, width) in COMMANDS {
            if id == DETAILS {
                pos(id, w - d(width + 10), d(83), d(width), d(31));
            } else {
                if matches!(id, CUT | SORT | MORE) {
                    x += d(9);
                }
                pos(id, x, d(83), d(width), d(31));
                x += d(width + 3);
            }
        }

        let sidebar_width = d(166);
        let mut y = d(123);
        for id in [
            HOME,
            COMPUTER,
            PLACE_START,
            PLACE_START + 1,
            PLACE_START + 2,
            PLACE_START + 3,
            PLACE_START + 4,
            PLACE_START + 5,
            PINNED,
            NETWORK,
            RECYCLE,
            LIBRARIES,
        ] {
            if matches!(id, PLACE_START | NETWORK) {
                y += d(13);
            }
            pos(id, d(6), y, sidebar_width - d(12), d(29));
            y += d(29);
        }
        let status_y = h - d(27);
        let listing_right = if self.settings.details { w - d(226) } else { w };
        let content_x = sidebar_width + d(12);
        let drive_mode = self.drive_cards();
        let section = !matches!(self.location(), Location::Folder(_));
        let list_y = if section { 155 } else { 123 };
        set_visible(self.title, section);
        set_visible(self.summary, section);
        move_to(
            self.title,
            content_x,
            d(131),
            (listing_right - content_x - d(160)).max(d(100)),
            d(23),
        );
        move_to(self.summary, listing_right - d(116), d(133), d(100), d(22));
        set_visible(self.list, !drive_mode);
        move_to(
            self.list,
            content_x - d(3),
            d(list_y),
            listing_right - content_x - d(6),
            status_y
                - d(if self.edit.is_some() {
                    list_y + 48
                } else {
                    list_y
                }),
        );
        let columns = ((listing_right - d(174)) / d(255)).max(1) as usize;
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
                    d(245),
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
        set_visible(self.detail_title, self.settings.details);
        set_visible(self.detail_body, self.settings.details);
        move_to(self.status, margin, status_y + d(6), w - d(221), d(19));
        pos(QUICK_DETAILS, w - d(65), status_y + d(1), d(30), d(24));
        pos(QUICK_ICONS, w - d(34), status_y + d(1), d(30), d(24));
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
        unsafe {
            InvalidateRect(self.window, null(), 0);
            InvalidateRect(self.header, null(), 0);
        }
    }

    fn address_focus(&mut self, editing: bool) {
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
                right: d(166),
                bottom: bounds.bottom - d(28),
                ..bounds
            },
            self.colors.sidebar,
        );
        fill(
            dc,
            &RECT {
                top: bounds.bottom - d(28),
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
        let search_width = (bounds.right / 4).clamp(d(158), d(280));
        let left = d(169);
        let right = bounds.right - search_width - d(18);
        fill(
            dc,
            &RECT {
                left,
                right,
                top: d(43),
                bottom: d(74),
            },
            self.colors.chrome,
        );
        fill(
            dc,
            &RECT {
                left: bounds.right - search_width - d(13),
                top: d(43),
                right: bounds.right - d(7),
                bottom: d(74),
            },
            self.colors.chrome,
        );
        for y in [d(190), d(381)] {
            fill(
                dc,
                &RECT {
                    left: d(12),
                    top: y,
                    right: d(153),
                    bottom: y + 1,
                },
                self.colors.edge,
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
        let right = if self.settings.details {
            bounds.right - d(226)
        } else {
            0
        };
        for x in [d(166), right] {
            if x != 0 {
                fill(
                    dc,
                    &RECT {
                        top: d(122),
                        left: x,
                        right: x + 1,
                        bottom: bounds.bottom - d(28),
                    },
                    self.colors.edge,
                );
            }
        }
        if self.settings.details {
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

    fn update_navigation(&mut self) {
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
                    .take(80)
                    .map(Path::to_path_buf)
                    .collect()
            })
            .unwrap_or_default();
        self.editing_address = false;
        self.crumb_slots.clear();
        self.crumb_display.clear();
        if self.crumbs.is_empty() {
            self.crumb_slots.push(self.location().clone());
            self.crumb_display.push((self.location().caption(), 126));
        } else {
            self.crumb_slots.push(Location::Computer);
            self.crumb_display.push(("This PC".into(), 78));
            for path in self.crumbs.iter().take(CRUMB_LIMIT - 1).rev() {
                let label = if let Some(name) = path.file_name() {
                    name.to_string_lossy().into_owned()
                } else {
                    path.display().to_string()
                };
                let width = 30 + (label.chars().count() as i32 * 7).clamp(16, 135);
                self.crumb_slots.push(Location::Folder(path.clone()));
                self.crumb_display.push((label, width));
            }
        }
        for index in 0..self.crumb_display.len() {
            let label = self.crumb_display[index].0.clone();
            self.set_dynamic_button(CRUMB_START + index as u16, &label);
        }
        let hint = wide(if self.location_dir().is_some() {
            format!("Search {}", self.location().caption())
        } else {
            "Open a folder to search".into()
        });
        send(self.search, EM_SETCUEBANNER, 1, hint.as_ptr() as isize);
        set_enabled(self.button(BACK), self.tab().history.can_back());
        set_enabled(self.button(FORWARD), self.tab().history.can_forward());
        set_enabled(self.button(UP), self.location_dir().is_some());
        set_enabled(self.button(NEW), self.dest().is_some());
        set_enabled(self.button(PASTE), self.dest().is_some());
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
            self.set_count();
            self.edit = None;
            self.update_navigation();
        }
        self.error = None;
        self.note = None;
        self.scanning = true;
        self.layout();
        self.status();
        self.request_scan();
    }

    fn request_scan(&mut self) {
        self.id = self.id.wrapping_add(1);
        if let Some(scanner) = &self.scanner {
            scanner.request(Request {
                id: self.id,
                location: self.location().clone(),
                hidden: self.settings.hidden,
                sort: self.tab().sort,
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
                    let selected = self
                        .selected_indices()
                        .into_iter()
                        .filter_map(|index| self.entry(index))
                        .map(|entry| entry.path.clone())
                        .collect::<std::collections::HashSet<_>>();
                    let focused = usize::try_from(send(
                        self.list,
                        LVM_GETNEXTITEM,
                        usize::MAX,
                        LVNI_FOCUSED as isize,
                    ))
                    .ok()
                    .and_then(|index| self.entry(index))
                    .map(|entry| entry.path.clone());
                    self.clear_selection();
                    self.rows = rows;
                    self.error = error;
                    self.scanning = false;
                    self.keep_rows = false;
                    self.rebuild_visible();
                    self.update_drive_buttons();
                    if focused.is_some() || !selected.is_empty() {
                        for (index, &row) in self.visible.iter().enumerate() {
                            let path = &self.rows[row].path;
                            let mut flags = 0;
                            if selected.contains(path) {
                                flags |= LVIS_SELECTED;
                            }
                            if focused.as_ref() == Some(path) {
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
                        Location::Folder(_) | Location::Search { .. } => {
                            self.settings.hidden != settings.hidden
                        }
                        Location::Computer => false,
                    };
                    let view_changed = self.dest().is_some_and(|path| {
                        self.settings.views.get(&path) != settings.views.get(&path)
                    });
                    let details_changed = self.settings.details != settings.details;
                    self.places = places;
                    self.settings = settings;
                    let restore = self.launch_path.is_none()
                        && self.id == 1
                        && self.tabs.len() == 1
                        && matches!(self.location(), Location::Home)
                        && !self.editing_address
                        && self.settings.restore_tabs
                        && !self.settings.tabs.is_empty();
                    self.booted = true;
                    self.apply_theme();
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
                Event::SettingsError(error) => {
                    self.note = Some(error);
                    self.status();
                }
                Event::Operation { result, refresh } => {
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
        self.visible.extend(
            self.rows
                .iter()
                .enumerate()
                .filter(|(_, row)| filter.shows(row))
                .map(|(index, _)| index),
        );
        self.set_count();
    }

    fn entry(&self, index: usize) -> Option<&Entry> {
        self.visible
            .get(index)
            .and_then(|&row| self.rows.get(row))
            .map(Arc::as_ref)
    }

    fn selected_indices(&self) -> Vec<usize> {
        let mut selected = Vec::new();
        let mut index = -1;
        loop {
            index = send(
                self.list,
                LVM_GETNEXTITEM,
                index as usize,
                LVNI_SELECTED as isize,
            );
            if index < 0 {
                return selected;
            }
            selected.push(index as usize);
        }
    }

    fn selection_count(&self) -> usize {
        send(self.list, LVM_GETSELECTEDCOUNT, 0, 0).max(0) as usize
    }

    fn first_selected(&self) -> Option<&Entry> {
        let index = send(
            self.list,
            LVM_GETNEXTITEM,
            usize::MAX,
            LVNI_SELECTED as isize,
        );
        self.entry(usize::try_from(index).ok()?)
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
        let items = self.visible.len();
        let count = self.selection_count();
        let summary = if self.scanning {
            "Loading…".into()
        } else if items == 1 {
            "1 item".into()
        } else {
            format!("{items} items")
        };
        text(self.summary, summary);
        let status = if let Some(message) = self.note.as_ref().or(self.error.as_ref()) {
            message.clone()
        } else if self.busy != 0 {
            "Windows is processing the file operation…".into()
        } else if self.scanning {
            format!("{items} found · Reading folder…")
        } else if count != 0 {
            format!("{items} items  ·  {count} selected")
        } else if items == 0 {
            "No items to show.".into()
        } else {
            format!("{items} items")
        };
        text(self.status, status);
        let selected = self.first_selected();
        let actionable = selected.is_some_and(|entry| !entry.location);
        for id in [CUT, COPY, DELETE] {
            set_enabled(self.button(id), actionable);
        }
        set_enabled(self.button(RENAME), actionable && count == 1);
        if self.settings.details {
            match selected {
                Some(entry) => {
                    text(self.detail_title, entry.label());
                    let mut body = format!(
                        "{}\n\nSize: {}\nModified: {}\n\n{}",
                        entry.kind(),
                        entry.display_size(),
                        entry.date(),
                        entry.path.display()
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
        let Some(entry) = self.first_selected() else {
            return;
        };
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
            settings.tabs = self
                .tabs
                .iter()
                .map(|tab| tab.history.current().clone())
                .collect();
            settings.active = self.active;
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
            FRAME_MIN => unsafe {
                ShowWindow(self.window, SW_MINIMIZE);
            },
            FRAME_MAX => unsafe {
                ShowWindow(
                    self.window,
                    if IsZoomed(self.window) != 0 {
                        SW_RESTORE
                    } else {
                        SW_MAXIMIZE
                    },
                );
            },
            FRAME_CLOSE => unsafe {
                PostMessageW(self.window, WM_CLOSE, 0, 0);
            },
            ENTER_SEARCH => self.search(),
            HOME => self.go(Location::Home),
            COMPUTER => self.go(Location::Computer),
            NETWORK => {
                self.operation(Operation::Namespace("shell:NetworkPlacesFolder"));
            }
            RECYCLE => {
                self.operation(Operation::Namespace("shell:RecycleBinFolder"));
            }
            LIBRARIES => {
                self.operation(Operation::Namespace("shell:Libraries"));
            }
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
                self.apply_view(mode);
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
                unsafe {
                    InvalidateRect(self.list, null(), 0);
                }
                self.schedule_save();
            }
            SORT_NAME | SORT_MODIFIED | SORT_KIND | SORT_SIZE | SORT_REVERSE => {
                let sort = &mut self.tab_mut().sort;
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
                    *sort = Sort {
                        column,
                        descending: false,
                    };
                }
                self.refresh();
            }
            FILTER_ALL | FILTER_FOLDERS | FILTER_FILES => {
                self.tab_mut().filter = match id {
                    FILTER_FOLDERS => Filter::Folders,
                    FILTER_FILES => Filter::Files,
                    _ => Filter::All,
                };
                self.clear_selection();
                self.rebuild_visible();
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
                    .filter(|entry| entry.directory)
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
                    if matches!(self.location(), Location::Home) {
                        self.refresh();
                    }
                    self.note = note;
                    self.status();
                    self.schedule_save();
                }
            }
            id if (CRUMB_START..CRUMB_START + CRUMB_LIMIT as u16).contains(&id) => {
                if let Some(location) = self.crumb_slots.get((id - CRUMB_START) as usize).cloned() {
                    if self.location() == &location {
                        self.address_focus(true);
                    } else {
                        self.go(location);
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
            id if (PLACE_START..PLACE_START + PLACES.len() as u16).contains(&id) => {
                let title = PLACES[(id - PLACE_START) as usize];
                if let Some((path, _)) = self.places.iter().find(|(_, label)| *label == title) {
                    self.go(Location::Folder(path.clone()));
                } else {
                    self.note = Some(format!("{title} is unavailable."));
                    self.status();
                }
            }
            id if (TAB_START..TAB_START + TAB_LIMIT as u16).contains(&id) => {
                self.switch((id - TAB_START) as usize)
            }
            id if (BREADCRUMB_START..BREADCRUMB_START + 80).contains(&id) => {
                if let Some(path) = self.crumbs.get((id - BREADCRUMB_START) as usize) {
                    self.go(Location::Folder(path.clone()));
                }
            }
            id if (PIN_START..PIN_START + 75).contains(&id) => {
                if let Some(path) = self.pinned.get((id - PIN_START) as usize) {
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
}

impl MenuItem {
    fn new(id: u16, label: &str) -> Self {
        Self {
            id,
            label: label.into(),
            enabled: true,
            checked: false,
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
                let sort = self.tab().sort;
                vec![
                    item(SORT_NAME, "Name").checked(sort.column == Column::Name),
                    item(SORT_MODIFIED, "Date modified").checked(sort.column == Column::Modified),
                    item(SORT_KIND, "Type").checked(sort.column == Column::Kind),
                    item(SORT_SIZE, "Size").checked(sort.column == Column::Size),
                    line(),
                    item(SORT_REVERSE, "Descending").checked(sort.descending),
                ]
            }
            VIEW => {
                let view = self.tab().view;
                let mut items = vec![];
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
                let filter = self.tab().filter;
                vec![
                    item(FILTER_ALL, "All items").checked(filter == Filter::All),
                    item(FILTER_FOLDERS, "Folders only").checked(filter == Filter::Folders),
                    item(FILTER_FILES, "Files only").checked(filter == Filter::Files),
                ]
            }
            MORE => vec![
                item(NEW_WINDOW, "New window\tCtrl+N"),
                item(CLOSE_TAB, "Close tab\tCtrl+W"),
                item(MOVE_TAB_LEFT, "Move tab left").enabled(self.active > 0),
                item(MOVE_TAB_RIGHT, "Move tab right").enabled(self.active + 1 < self.tabs.len()),
                line(),
                item(PIN, "Pin or unpin folder").enabled(
                    self.location_dir().is_some()
                        || self.first_selected().is_some_and(|row| row.directory),
                ),
                item(SELECT_ALL, "Select all\tCtrl+A").enabled(!self.visible.is_empty()),
                line(),
                item(RESTORE_TABS, "Restore tabs on startup").checked(self.settings.restore_tabs),
                line(),
                item(THEME_SYSTEM, "Use system theme")
                    .checked(matches!(self.settings.theme, ThemePreference::System)),
                item(THEME_DARK, "Dark theme")
                    .checked(matches!(self.settings.theme, ThemePreference::Dark)),
                item(THEME_LIGHT, "Light theme")
                    .checked(matches!(self.settings.theme, ThemePreference::Light)),
            ],
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
            PINNED => {
                self.pinned = self.settings.pins.clone();
                let mut items: Vec<_> = self
                    .pinned
                    .iter()
                    .enumerate()
                    .map(|(index, path)| {
                        item(
                            PIN_START + index as u16,
                            &path.to_string_lossy().replace('&', "&&"),
                        )
                    })
                    .collect();
                if !items.is_empty() {
                    items.push(line());
                }
                items.push(
                    item(PIN, "Pin or unpin current folder").enabled(self.location_dir().is_some()),
                );
                items
            }
            CONTEXT => {
                if let Some(entry) = self.first_selected() {
                    let count = self.selection_count();
                    let mut items = vec![item(OPEN, "Open\tEnter")];
                    if entry.directory {
                        items.push(item(OPEN_TAB, "Open in new tab"));
                        items.push(item(OPEN_WINDOW, "Open in new window"));
                        items.push(item(
                            PIN,
                            if self.settings.pins.contains(&entry.path) {
                                "Unpin folder"
                            } else {
                                "Pin folder"
                            },
                        ));
                    } else {
                        items.push(item(OPEN_WITH, "Open with"));
                    }
                    if !entry.location && self.dest().is_some() {
                        items.push(item(COMPRESS, "Compress to ZIP…"));
                    }
                    if !entry.directory && count == 1 && is_zip(&entry.path) {
                        items.push(item(EXTRACT, "Extract all…"));
                    }
                    items.extend([
                        line(),
                        item(CUT, "Cut\tCtrl+X").enabled(!entry.location),
                        item(COPY, "Copy\tCtrl+C").enabled(!entry.location),
                        item(COPY_PATH, "Copy as path"),
                        item(PASTE, "Paste\tCtrl+V").enabled(self.dest().is_some()),
                        item(RENAME, "Rename\tF2").enabled(!entry.location && count == 1),
                        item(DELETE, "Delete\tDel").enabled(!entry.location),
                        line(),
                        item(SHORTCUT, "Create shortcut…")
                            .enabled(count == 1 && self.dest().is_some()),
                        item(REVEAL, "Show in Windows File Explorer"),
                        item(PROPERTIES, "Properties\tAlt+Enter"),
                    ]);
                    items
                } else {
                    let folder = self.dest().is_some();
                    vec![
                        item(NEW_FOLDER, "New folder").enabled(folder),
                        item(NEW_TEXT, "New text document").enabled(folder),
                        item(PASTE, "Paste\tCtrl+V").enabled(folder),
                        item(REFRESH, "Refresh\tF5"),
                        item(PIN, "Pin or unpin current folder").enabled(folder),
                    ]
                }
            }
            _ => Vec::new(),
        }
    }

    fn button_style(&self, id: u16, control: HWND) -> ButtonStyle {
        let tab = (TAB_START..TAB_START + TAB_LIMIT as u16).contains(&id);
        let crumb = (CRUMB_START..CRUMB_START + CRUMB_LIMIT as u16).contains(&id);
        let place = (PLACE_START..PLACE_START + PLACES.len() as u16).contains(&id);
        let sidebar = (HOME..=LIBRARIES).contains(&id);
        let active_place = place
            && self.places.iter().any(|(path, label)| {
                *label == PLACES[(id - PLACE_START) as usize]
                    && self.location().directory() == Some(path.as_path())
            });
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
            PATH | COMPUTER => Some(Glyph::Drive),
            HOME => Some(Glyph::Home),
            PINNED => Some(Glyph::Pin),
            NETWORK => Some(Glyph::Network),
            RECYCLE => Some(Glyph::Recycle),
            LIBRARIES => Some(Glyph::Library),
            FRAME_MIN => Some(Glyph::Minimize),
            FRAME_MAX => Some(if unsafe { IsZoomed(self.window) } != 0 {
                Glyph::Restore
            } else {
                Glyph::Maximize
            }),
            id if crumb && id != CRUMB_START => Some(Glyph::Chevron),
            _ if place => Some(Glyph::Folder),
            _ if tab => Some(
                match self
                    .tabs
                    .get((id - TAB_START) as usize)
                    .map(|tab| tab.history.current())
                {
                    Some(Location::Home) => Glyph::Home,
                    Some(Location::Computer) => Glyph::Drive,
                    Some(Location::Search { .. }) => Glyph::Search,
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
        let surface = if sidebar {
            self.colors.sidebar
        } else if title {
            self.colors.title
        } else {
            self.colors.chrome
        };
        let accent = place
            || tab
            || matches!(
                id,
                HOME | SORT | VIEW | FILTER | NEW | PINNED | RECYCLE | LIBRARIES
            );
        let icon_color = if accent {
            self.colors.accent
        } else {
            self.colors.text
        };
        ButtonStyle {
            glyph,
            icon_color,
            surface,
            label: sidebar
                || crumb
                || tab
                || matches!(id, NEW | SORT | VIEW | FILTER | DETAILS)
                || glyph.is_none(),
            left: sidebar || crumb || tab,
            active: (tab && id == TAB_START + self.active as u16)
                || active_place
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

    fn draw_header(&self) {
        let header = self.header;
        let mut paint = unsafe { zeroed() };
        let dc = unsafe { BeginPaint(header, &mut paint) };
        let bounds = rect(header);
        let d = |v| scale(v, self.dpi);
        fill(dc, &bounds, self.colors.sidebar);
        let sort = self.tab().sort;
        for (index, (label, _)) in COLUMNS.into_iter().enumerate() {
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
            let edge = scale(6, self.dpi);
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
                                    1 => entry.date(),
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
                if let Some(entry) = usize::try_from(event.iItem)
                    .ok()
                    .and_then(|index| self.entry(index))
                {
                    self.show_entry(OPEN, entry.path.clone(), entry.directory);
                }
            }
            NM_RCLICK => {
                let event = unsafe { &*(notification as *const NMITEMACTIVATE) };
                if event.iItem < 0 {
                    self.clear_selection();
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

fn show_menu(app: &RefCell<App>, id: u16, coordinates: LPARAM) {
    let Ok(mut state) = app.try_borrow_mut() else {
        return;
    };
    let items = state.menu(id);
    let window = state.window;
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
            GetWindowRect(state.button(id), &mut bounds);
            anchor.x = bounds.left;
            anchor.y = bounds.bottom;
        }
    }
    drop(state);
    if items.is_empty() {
        return;
    }
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        for item in items {
            let flags = if item.id == 0 {
                MF_SEPARATOR
            } else {
                MF_STRING
                    | if !item.enabled { MF_GRAYED } else { 0 }
                    | if item.checked { MF_CHECKED } else { 0 }
            };
            AppendMenuW(menu, flags, item.id as usize, wide(item.label).as_ptr());
        }
        let selected = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            anchor.x,
            anchor.y,
            0,
            window,
            null(),
        );
        DestroyMenu(menu);
        if selected > 0 {
            PostMessageW(window, WM_COMMAND, selected as usize, 0);
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
        (0x46, true, false, _) | (VK_F3, false, false, _) => {
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
        (0x41, true, false, false) if !typing => SELECT_ALL,
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
    if message == WM_NCCALCSIZE && wp != 0 {
        return 0;
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
                let (rows, complete) = (app.visible.len(), !app.scanning);
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
            if (d(169)..width - (width / 4).clamp(d(158), d(280)) - d(16)).contains(&x)
                && (d(43)..d(74)).contains(&y)
            {
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
        WM_SIZE => {
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
            info.ptMinTrackSize.x = scale(834, app.dpi);
            info.ptMinTrackSize.y = scale(590, app.dpi);
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
        WM_DRAWITEM => {
            let draw = unsafe { &*(lp as *const DRAWITEMSTRUCT) };
            let id = draw.CtlID as u16;
            if (DRIVE_START..DRIVE_START + DRIVE_LIMIT as u16).contains(&id) {
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
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
            let dc = wp as HDC;
            let control = lp as HWND;
            let chrome = control == app.status;
            unsafe {
                SetTextColor(
                    dc,
                    if control == app.status || control == app.summary {
                        app.colors.muted
                    } else {
                        app.colors.text
                    },
                );
                SetBkColor(
                    dc,
                    if chrome {
                        app.colors.chrome
                    } else {
                        app.colors.content
                    },
                );
            }
            app.brushes[usize::from(!chrome)] as isize
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
            if app.busy > 0 {
                app.note =
                    Some("Wait for Windows to finish the file operation before closing.".into());
                app.status();
                return 0;
            }
            unsafe {
                KillTimer(window, SETTINGS_TIMER);
                DestroyWindow(window);
            }
            0
        }
        _ => unsafe { DefWindowProcW(window, message, wp, lp) },
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
            0,
            class.as_ptr(),
            wide("BetterExplorer").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
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
