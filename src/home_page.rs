use crate::chrome::{
    button, draw_glyph, draw_resource_icon, fill, font, rect, rgb, scale, text_at, Glyph, Palette,
};
use better_explorer::model::wide;
use std::{
    cell::{Cell, RefCell},
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    rc::Rc,
    sync::OnceLock,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{
            SetScrollInfo, SetWindowTheme, DRAWITEMSTRUCT, ODS_FOCUS, ODS_SELECTED, WM_MOUSELEAVE,
        },
        HiDpi::GetSystemMetricsForDpi,
        Input::KeyboardAndMouse::{
            SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_END, VK_HOME, VK_NEXT,
            VK_PRIOR, VK_RETURN,
        },
        Shell::{DefSubclassProc, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const GAP: i32 = 9;
const TILE_HEIGHT: i32 = 62;
const ROW_HEIGHT: i32 = 46;
const TAB_HEIGHT: i32 = 33;
const REVEAL: u32 = WM_APP + 1;

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Icon {
    Desktop,
    Downloads,
    Documents,
    Pictures,
    Music,
    Videos,
    Folder,
    File,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Activity {
    Recent,
    Favorites,
}

#[derive(Eq, PartialEq)]
pub struct Item {
    pub title: String,
    pub subtitle: String,
    pub id: u16,
    pub icon: Icon,
}

pub struct HomePage(Rc<State>);

impl HomePage {
    pub fn create(parent: HWND) -> Option<Self> {
        static REGISTERED: OnceLock<bool> = OnceLock::new();
        let class = wide("BetterExplorerHome");
        let module = unsafe { GetModuleHandleW(null()) };
        if !REGISTERED.get_or_init(|| unsafe {
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(page_proc),
                hInstance: module,
                lpszClassName: class.as_ptr(),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                ..zeroed()
            }) != 0
        }) {
            return None;
        }
        let window = unsafe {
            CreateWindowExW(
                WS_EX_CONTROLPARENT,
                class.as_ptr(),
                wide("Home").as_ptr(),
                WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                0,
                0,
                parent,
                null_mut(),
                module,
                null(),
            )
        };
        if window.is_null() {
            return None;
        }
        let scrollbar = unsafe {
            CreateWindowExW(
                0,
                wide("ScrollBar").as_ptr(),
                null(),
                WS_CHILD | SBS_VERT as u32,
                0,
                0,
                0,
                0,
                window,
                null_mut(),
                module,
                null(),
            )
        };
        let tabs = [
            create_button(window, "Recent", 0),
            create_button(window, "Favorites", 0),
        ];
        if scrollbar.is_null() || tabs.iter().any(|tab| tab.is_null()) {
            unsafe { DestroyWindow(window) };
            return None;
        }
        let page = Self(Rc::new(State {
            window: Cell::new(window),
            colors: Cell::new(Colors::default()),
            dpi: Cell::new(96),
            fonts: Cell::new(Fonts::new(96)),
            tabs,
            selected: Cell::new(Activity::Recent),
            quick: RefCell::new(Vec::new()),
            activity: RefCell::new(Vec::new()),
            scrollbar,
            scroll: Cell::new(0),
            wheel_fraction: Cell::new(0),
            compact: Cell::new(false),
            layout: Cell::new(Layout::default()),
        }));
        unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, Rc::into_raw(page.0.clone()) as isize) };
        Some(page)
    }

    pub fn set_bounds(&self, x: i32, y: i32, width: i32, height: i32) {
        unsafe { MoveWindow(self.0.window.get(), x, y, width.max(0), height.max(0), 1) };
    }

    pub fn set_visible(&self, visible: bool) {
        unsafe { ShowWindow(self.0.window.get(), if visible { SW_SHOW } else { SW_HIDE }) };
    }

    pub fn set_compact(&self, compact: bool) {
        if self.0.compact.replace(compact) != compact {
            self.0.arrange();
        }
    }

    pub fn focus(&self) {
        let target = self.0.quick.borrow().first().map_or(
            self.0.tabs[usize::from(self.0.selected.get() == Activity::Favorites)],
            |entry| entry.window,
        );
        unsafe { SetFocus(target) };
    }

    pub fn set_style(&self, palette: &Palette, dpi: u32) {
        let state = &self.0;
        state.colors.set(Colors::from_palette(palette));
        if state.dpi.replace(dpi) != dpi {
            state.fonts.replace(Fonts::new(dpi)).delete();
        }
        let theme = wide(if palette.dark {
            "DarkMode_Explorer"
        } else {
            "Explorer"
        });
        unsafe {
            SetWindowTheme(
                state.scrollbar,
                if palette.contrast {
                    null()
                } else {
                    theme.as_ptr()
                },
                null(),
            );
        }
        state.arrange();
    }

    pub fn set_content(
        &self,
        quick: Vec<Item>,
        activity: Vec<Item>,
        recent_id: u16,
        favorites_id: u16,
        selected: Activity,
    ) {
        let state = &self.0;
        let window = state.window.get();
        let quick_changed = update_entries(window, &state.quick, quick);
        let activity_changed = update_entries(window, &state.activity, activity);
        let changed_tab = state.selected.replace(selected) != selected;
        for (control, id) in state.tabs.into_iter().zip([recent_id, favorites_id]) {
            unsafe { SetWindowLongPtrW(control, GWLP_ID, id as isize) };
        }
        if quick_changed || activity_changed {
            let quick = state.quick.borrow();
            let activity = state.activity.borrow();
            let tab_order = quick
                .iter()
                .map(|entry| entry.window)
                .chain(state.tabs)
                .chain(activity.iter().map(|entry| entry.window));
            for control in tab_order {
                unsafe {
                    SetWindowPos(
                        control,
                        HWND_BOTTOM,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
            }
        }
        if changed_tab {
            let current = state.layout.get();
            state.scroll.set(state.scroll.get().min(current.tabs_y));
            state.wheel_fraction.set(0);
        }
        if quick_changed || activity_changed || changed_tab {
            state.arrange();
        }
    }
}

impl Drop for HomePage {
    fn drop(&mut self) {
        let window = self.0.window.get();
        if !window.is_null() {
            unsafe { DestroyWindow(window) };
        }
    }
}

struct Control {
    window: HWND,
    item: Item,
}

fn create_button(parent: HWND, label: &str, id: u16) -> HWND {
    let window = button(parent, label, id);
    if !window.is_null() {
        unsafe { SetWindowSubclass(window, Some(button_proc), 1, 0) };
    }
    window
}

fn update_entries(window: HWND, slot: &RefCell<Vec<Control>>, items: Vec<Item>) -> bool {
    if slot.borrow().iter().map(|entry| &entry.item).eq(&items) {
        return false;
    }
    let mut controls = slot.take().into_iter();
    let entries = items
        .into_iter()
        .filter_map(|item| {
            let control = if let Some(old) = controls.next() {
                if old.item != item {
                    unsafe {
                        SetWindowLongPtrW(old.window, GWLP_ID, item.id as isize);
                        SetWindowTextW(old.window, wide(&item.title).as_ptr());
                    }
                }
                old.window
            } else {
                create_button(window, &item.title, item.id)
            };
            (!control.is_null()).then_some(Control {
                window: control,
                item,
            })
        })
        .collect();
    slot.replace(entries);
    for leftover in controls {
        unsafe { DestroyWindow(leftover.window) };
    }
    true
}

#[derive(Clone, Copy)]
struct Colors {
    content: u32,
    tile: u32,
    text: u32,
    muted: u32,
    edge: u32,
    accent: u32,
    active: u32,
    contrast: bool,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            content: rgb(255, 255, 255),
            tile: rgb(251, 248, 246),
            text: rgb(32, 30, 28),
            muted: rgb(94, 86, 82),
            edge: rgb(229, 218, 211),
            active: rgb(250, 231, 217),
            accent: rgb(163, 68, 24),
            contrast: false,
        }
    }
}

impl Colors {
    fn from_palette(palette: &Palette) -> Self {
        Self {
            content: palette.content,
            tile: palette.sidebar,
            text: palette.text,
            muted: palette.muted,
            edge: palette.edge,
            accent: palette.accent,
            active: palette.active,
            contrast: palette.contrast,
        }
    }

    fn highlight_text(self) -> u32 {
        if self.contrast {
            unsafe { GetSysColor(COLOR_HIGHLIGHTTEXT) }
        } else {
            self.text
        }
    }

    fn highlight(self) -> u32 {
        if self.contrast {
            self.accent
        } else {
            self.active
        }
    }
}

#[derive(Clone, Copy)]
struct Fonts {
    regular: HFONT,
    label: HFONT,
    heading: HFONT,
}

impl Fonts {
    fn new(dpi: u32) -> Self {
        Self {
            regular: font(dpi, false),
            label: font(dpi, true),
            heading: unsafe {
                CreateFontW(
                    -scale(21, dpi),
                    0,
                    0,
                    0,
                    600,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET as u32,
                    OUT_DEFAULT_PRECIS as u32,
                    CLIP_DEFAULT_PRECIS as u32,
                    CLEARTYPE_QUALITY as u32,
                    0,
                    wide("Segoe UI").as_ptr(),
                )
            },
        }
    }

    fn delete(self) {
        for font in [self.regular, self.label, self.heading] {
            unsafe { DeleteObject(font) };
        }
    }
}

fn grid_columns(width: i32, count: usize, dpi: u32) -> usize {
    let gap = scale(GAP, dpi);
    let minimum = scale(204, dpi);
    let capacity = ((width + gap) / (minimum + gap)).clamp(1, 3) as usize;
    capacity.min(count.max(1))
}

#[derive(Clone, Copy, Default)]
struct Layout {
    x: i32,
    width: i32,
    viewport: i32,
    columns: usize,
    tiles_y: i32,
    tabs_y: i32,
    activity_y: i32,
    total: i32,
}

impl Layout {
    fn new(
        width: i32,
        height: i32,
        quick: usize,
        activity: usize,
        dpi: u32,
        compact: bool,
    ) -> Self {
        let d = |value| scale(value, dpi);
        let x = d(24).min((width / 10).max(0));
        let width = (width - 2 * x).clamp(0, d(1100));
        let columns = if compact {
            1
        } else {
            grid_columns(width, quick, dpi)
        };
        let rows = quick.div_ceil(columns) as i32;
        let tiles_y = d(90);
        let tabs_y = tiles_y + rows * (d(TILE_HEIGHT) + d(GAP)) + d(14);
        let activity_y = tabs_y + d(TAB_HEIGHT + 9);
        let total = activity_y + activity.max(1) as i32 * d(ROW_HEIGHT) + d(18);
        Self {
            x,
            width,
            viewport: height,
            columns,
            tiles_y,
            tabs_y,
            activity_y,
            total,
        }
    }

    fn max_scroll(self) -> i32 {
        (self.total - self.viewport).max(0)
    }
}

struct State {
    window: Cell<HWND>,
    dpi: Cell<u32>,
    colors: Cell<Colors>,
    fonts: Cell<Fonts>,
    tabs: [HWND; 2],
    selected: Cell<Activity>,
    quick: RefCell<Vec<Control>>,
    activity: RefCell<Vec<Control>>,
    scrollbar: HWND,
    scroll: Cell<i32>,
    wheel_fraction: Cell<i32>,
    compact: Cell<bool>,
    layout: Cell<Layout>,
}

impl Drop for State {
    fn drop(&mut self) {
        self.fonts.get().delete();
    }
}

impl State {
    fn arrange(&self) {
        let window = self.window.get();
        let bounds = rect(window);
        let dpi = self.dpi.get();
        let d = |value| scale(value, dpi);
        let quick = self.quick.borrow();
        let activity = self.activity.borrow();
        let layout_for = |width| {
            Layout::new(
                width,
                bounds.bottom,
                quick.len(),
                activity.len(),
                dpi,
                self.compact.get(),
            )
        };
        let full_layout = layout_for(bounds.right);
        let bar = bounds.bottom > 0 && full_layout.max_scroll() > 0;
        let scroll_width = unsafe { GetSystemMetricsForDpi(SM_CXVSCROLL, dpi) }.max(d(12));
        let layout = if bar {
            layout_for((bounds.right - scroll_width).max(0))
        } else {
            full_layout
        };
        self.layout.set(layout);
        let offset = self.scroll.get().clamp(0, layout.max_scroll());
        self.scroll.set(offset);
        let place = |control, x, y, width: i32, height| unsafe {
            SetWindowPos(
                control,
                null_mut(),
                x,
                y - offset,
                width.max(0),
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        };
        let gap = d(GAP);
        let columns = layout.columns as i32;
        let tile_space = (layout.width - (columns - 1) * gap).max(0);
        for (index, entry) in quick.iter().enumerate() {
            let index = index as i32;
            let column = index % columns;
            let start = tile_space * column / columns;
            let end = tile_space * (column + 1) / columns;
            place(
                entry.window,
                layout.x + start + column * gap,
                layout.tiles_y + index / columns * (d(TILE_HEIGHT) + gap),
                end - start,
                d(TILE_HEIGHT),
            );
        }
        let tab_gap = d(4);
        let tab_width = ((layout.width - tab_gap) / 2).clamp(0, d(98));
        for (index, tab) in self.tabs.into_iter().enumerate() {
            place(
                tab,
                layout.x + index as i32 * (tab_width + tab_gap),
                layout.tabs_y,
                tab_width,
                d(TAB_HEIGHT),
            );
        }
        for (index, entry) in activity.iter().enumerate() {
            place(
                entry.window,
                layout.x,
                layout.activity_y + index as i32 * d(ROW_HEIGHT),
                layout.width,
                d(ROW_HEIGHT),
            );
        }
        unsafe {
            if bar {
                SetWindowPos(
                    self.scrollbar,
                    null_mut(),
                    bounds.right - scroll_width,
                    0,
                    scroll_width,
                    bounds.bottom,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                SetScrollInfo(
                    self.scrollbar,
                    SB_CTL,
                    &SCROLLINFO {
                        cbSize: size_of::<SCROLLINFO>() as u32,
                        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                        nMin: 0,
                        nMax: layout.total - 1,
                        nPage: layout.viewport as u32,
                        nPos: offset,
                        nTrackPos: 0,
                    },
                    1,
                );
            }
            ShowWindow(self.scrollbar, if bar { SW_SHOW } else { SW_HIDE });
            RedrawWindow(window, null(), null_mut(), RDW_INVALIDATE | RDW_ALLCHILDREN);
        }
    }

    fn scroll_to(&self, position: i32) {
        let next = position.clamp(0, self.layout.get().max_scroll());
        if self.scroll.replace(next) != next {
            self.arrange();
        }
    }

    fn scroll_bar(&self, command: u16) {
        let offset = self.scroll.get();
        let layout = self.layout.get();
        let line = scale(26, self.dpi.get());
        let next = match command as i32 {
            SB_TOP => 0,
            SB_BOTTOM => layout.max_scroll(),
            SB_LINEUP => offset - line,
            SB_LINEDOWN => offset + line,
            SB_PAGEUP => offset - layout.viewport,
            SB_PAGEDOWN => offset + layout.viewport,
            SB_THUMBTRACK | SB_THUMBPOSITION => {
                let mut info = SCROLLINFO {
                    cbSize: size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_TRACKPOS,
                    ..unsafe { zeroed() }
                };
                unsafe { GetScrollInfo(self.scrollbar, SB_CTL, &mut info) };
                info.nTrackPos
            }
            _ => return,
        };
        self.scroll_to(next);
    }

    fn wheel(&self, delta: i16) {
        let mut lines = 3u32;
        unsafe {
            SystemParametersInfoW(
                SPI_GETWHEELSCROLLLINES,
                0,
                (&mut lines as *mut u32).cast(),
                0,
            )
        };
        let distance = if lines == u32::MAX {
            self.layout.get().viewport
        } else {
            (lines as i32).saturating_mul(scale(26, self.dpi.get()))
        };
        let movement = self
            .wheel_fraction
            .get()
            .saturating_sub(i32::from(delta).saturating_mul(distance));
        self.wheel_fraction.set(movement % WHEEL_DELTA as i32);
        self.scroll_to(
            self.scroll
                .get()
                .saturating_add(movement / WHEEL_DELTA as i32),
        );
    }

    fn reveal(&self, child: HWND) {
        let mut bounds = unsafe { zeroed() };
        let mut origin = POINT { x: 0, y: 0 };
        let window = self.window.get();
        unsafe {
            GetClientRect(child, &mut bounds);
            MapWindowPoints(child, window, &mut origin, 1);
        }
        let height = self.layout.get().viewport;
        let inset = scale(4, self.dpi.get());
        if origin.y < inset {
            self.scroll_to(self.scroll.get() + origin.y - inset);
        } else if origin.y + bounds.bottom + inset > height {
            self.scroll_to(self.scroll.get() + origin.y + bounds.bottom + inset - height);
        }
    }

    fn paint(&self, dc: HDC, update: &RECT) {
        let colors = self.colors.get();
        let fonts = self.fonts.get();
        let layout = self.layout.get();
        let offset = self.scroll.get();
        let d = |value| scale(value, self.dpi.get());
        let line = |top, height| area(layout.x, top - offset, layout.width, height);
        let flags = DT_SINGLELINE | DT_VCENTER;
        fill(dc, update, colors.content);
        text_at(
            dc,
            "Home",
            line(d(16), d(31)),
            fonts.heading,
            colors.text,
            flags,
        );
        text_at(
            dc,
            "Quick access",
            line(d(58), d(22)),
            fonts.label,
            colors.text,
            flags,
        );
        let separator = line(layout.tabs_y + d(TAB_HEIGHT + 4), d(1));
        fill(dc, &separator, colors.edge);
        if self.activity.borrow().is_empty() {
            let label = match self.selected.get() {
                Activity::Recent => "No recent items.",
                Activity::Favorites => "No favorites yet.",
            };
            text_at(
                dc,
                label,
                line(layout.activity_y, d(ROW_HEIGHT)),
                fonts.regular,
                colors.muted,
                flags,
            );
        }
    }

    fn draw_button(&self, draw: &DRAWITEMSTRUCT) {
        let mut point = POINT { x: 0, y: 0 };
        let hovered =
            unsafe { GetCursorPos(&mut point) != 0 && WindowFromPoint(point) == draw.hwndItem };
        let focused = draw.itemState & ODS_FOCUS != 0;
        let hot = hovered || draw.itemState & ODS_SELECTED != 0;
        for (index, control) in self.tabs.into_iter().enumerate() {
            if draw.hwndItem == control {
                self.draw_tab(draw, index, hot, focused);
                return;
            }
        }
        for (section, tile) in [(&self.quick, true), (&self.activity, false)] {
            if let Some(entry) = section
                .borrow()
                .iter()
                .find(|entry| entry.window == draw.hwndItem)
            {
                self.draw_entry(draw, &entry.item, tile, hot, focused);
                return;
            }
        }
    }

    fn draw_tab(&self, draw: &DRAWITEMSTRUCT, index: usize, hot: bool, focused: bool) {
        let colors = self.colors.get();
        let fonts = self.fonts.get();
        let bounds = draw.rcItem;
        let d = |value| scale(value, self.dpi.get());
        let tab = if index == 0 {
            Activity::Recent
        } else {
            Activity::Favorites
        };
        let selected = self.selected.get() == tab;
        let highlighted = hot || selected;
        fill(draw.hDC, &bounds, colors.content);
        if highlighted || focused {
            round_box(
                draw.hDC,
                &bounds,
                d(8),
                if highlighted {
                    colors.highlight()
                } else {
                    colors.content
                },
                if focused && !(highlighted && colors.contrast) {
                    colors.accent
                } else if highlighted {
                    colors.highlight()
                } else {
                    colors.content
                },
            );
        }
        let label = if tab == Activity::Recent {
            "Recent"
        } else {
            "Favorites"
        };
        text_at(
            draw.hDC,
            label,
            bounds,
            if selected { fonts.label } else { fonts.regular },
            if highlighted {
                colors.highlight_text()
            } else {
                colors.text
            },
            DT_SINGLELINE | DT_VCENTER | DT_CENTER,
        );
        if selected && !colors.contrast {
            let mark = area(
                (bounds.right - d(16)) / 2,
                bounds.bottom - d(4),
                d(16),
                d(2),
            );
            fill(draw.hDC, &mark, colors.accent);
        }
        if focused && colors.contrast {
            let focus = area(
                bounds.left + d(2),
                bounds.top + d(2),
                bounds.right - bounds.left - d(4),
                bounds.bottom - bounds.top - d(4),
            );
            unsafe { DrawFocusRect(draw.hDC, &focus) };
        }
    }

    fn draw_entry(&self, draw: &DRAWITEMSTRUCT, item: &Item, tile: bool, hot: bool, focused: bool) {
        let colors = self.colors.get();
        let fonts = self.fonts.get();
        let dc = draw.hDC;
        let bounds = draw.rcItem;
        let d = |value| scale(value, self.dpi.get());
        let background = if hot {
            colors.highlight()
        } else if tile {
            colors.tile
        } else {
            colors.content
        };
        let accent = if focused || (hot && tile) {
            colors.accent
        } else if tile {
            colors.edge
        } else {
            background
        };
        let foreground = if hot {
            colors.highlight_text()
        } else {
            colors.text
        };
        let subtitle = if hot && colors.contrast {
            foreground
        } else {
            colors.muted
        };
        fill(dc, &bounds, colors.content);
        if tile || hot || focused {
            round_box(dc, &bounds, d(8), background, accent);
        }
        if !tile && !hot && !focused {
            let divider = area(
                bounds.left,
                bounds.bottom - d(1),
                bounds.right - bounds.left,
                d(1),
            );
            fill(dc, &divider, colors.edge);
        }
        let size = d(if tile { 34 } else { 26 });
        let icon_x = bounds.left + d(if tile { 12 } else { 6 });
        let center = (bounds.top + bounds.bottom) / 2;
        let label_x = icon_x + size + d(10);
        let width = bounds.right - label_x - d(10);
        draw_icon(
            dc,
            item.icon,
            icon_x,
            center - size / 2,
            size,
            colors,
            foreground,
        );
        let line = d(18);
        let has_subtitle = !item.subtitle.is_empty();
        text_at(
            dc,
            &item.title,
            area(
                label_x,
                center - if has_subtitle { line } else { line / 2 },
                width,
                line,
            ),
            fonts.label,
            foreground,
            DT_SINGLELINE | DT_VCENTER,
        );
        if has_subtitle {
            text_at(
                dc,
                &item.subtitle,
                area(label_x, center, width, line),
                fonts.regular,
                subtitle,
                DT_SINGLELINE | DT_VCENTER,
            );
        }
        if focused && colors.contrast {
            let focus = area(
                bounds.left + d(2),
                bounds.top + d(2),
                bounds.right - bounds.left - d(4),
                bounds.bottom - bounds.top - d(4),
            );
            unsafe { DrawFocusRect(dc, &focus) };
        }
    }
}

unsafe extern "system" fn page_proc(window: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *const State;
    let Some(state) = (unsafe { pointer.as_ref() }) else {
        return unsafe { DefWindowProcW(window, message, wp, lp) };
    };
    match message {
        WM_NCDESTROY => unsafe {
            state.window.set(null_mut());
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            drop(Rc::from_raw(pointer));
            DefWindowProcW(window, message, wp, lp)
        },
        WM_SIZE => {
            state.arrange();
            0
        }
        WM_PAINT => unsafe {
            let mut paint = zeroed();
            let dc = BeginPaint(window, &mut paint);
            state.paint(dc, &paint.rcPaint);
            EndPaint(window, &paint);
            0
        },
        WM_ERASEBKGND => 1,
        WM_DRAWITEM if lp != 0 => unsafe {
            state.draw_button(&*(lp as *const DRAWITEMSTRUCT));
            1
        },
        WM_COMMAND if lp != 0 && wp >> 16 == BN_CLICKED as usize => unsafe {
            PostMessageW(GetParent(window), message, wp, lp);
            0
        },
        WM_VSCROLL => {
            state.scroll_bar(wp as u16);
            0
        }
        WM_MOUSEWHEEL if wp & 0x000c == 0 => {
            state.wheel((wp >> 16) as i16);
            0
        }
        REVEAL => {
            state.reveal(wp as HWND);
            0
        }
        _ => unsafe { DefWindowProcW(window, message, wp, lp) },
    }
}

unsafe extern "system" fn button_proc(
    window: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    match message {
        WM_MOUSEMOVE => unsafe {
            TrackMouseEvent(&mut TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: window,
                dwHoverTime: 0,
            });
            InvalidateRect(window, null(), 0);
        },
        WM_MOUSELEAVE => unsafe {
            InvalidateRect(window, null(), 0);
        },
        WM_SETFOCUS => unsafe {
            SendMessageW(GetParent(window), REVEAL, window as usize, 0);
        },
        WM_KEYDOWN if wp as u16 == VK_RETURN => unsafe {
            SendMessageW(window, BM_CLICK, 0, 0);
            return 0;
        },
        WM_KEYDOWN if matches!(wp as u16, VK_HOME | VK_END | VK_PRIOR | VK_NEXT) => unsafe {
            let command = match wp as u16 {
                VK_HOME => SB_TOP,
                VK_END => SB_BOTTOM,
                VK_PRIOR => SB_PAGEUP,
                _ => SB_PAGEDOWN,
            };
            SendMessageW(GetParent(window), WM_VSCROLL, command as usize, 0);
            return 0;
        },
        _ => {}
    }
    unsafe { DefSubclassProc(window, message, wp, lp) }
}

fn area(x: i32, y: i32, width: i32, height: i32) -> RECT {
    RECT {
        left: x,
        top: y,
        right: x + width.max(0),
        bottom: y + height.max(0),
    }
}

fn round_box(dc: HDC, bounds: &RECT, radius: i32, surface: u32, edge: u32) {
    unsafe {
        let brush = CreateSolidBrush(surface);
        let pen = CreatePen(PS_SOLID, 1, edge);
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, pen);
        RoundRect(
            dc,
            bounds.left,
            bounds.top,
            bounds.right,
            bounds.bottom,
            radius,
            radius,
        );
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        DeleteObject(pen);
        DeleteObject(brush);
    }
}

fn draw_icon(dc: HDC, icon: Icon, x: i32, y: i32, size: i32, colors: Colors, foreground: u32) {
    let resource = match icon {
        Icon::Desktop => 6,
        Icon::Downloads => 7,
        Icon::Documents => 8,
        Icon::Pictures => 9,
        Icon::Music => 10,
        Icon::Videos => 11,
        Icon::Folder => 2,
        Icon::File => 3,
    };
    if !colors.contrast && draw_resource_icon(dc, resource, x, y, size) {
        return;
    }
    let glyph = match icon {
        Icon::File | Icon::Documents => Glyph::Library,
        _ => Glyph::Folder,
    };
    draw_glyph(dc, glyph, x, y, size, foreground);
}

#[cfg(test)]
mod tests {
    use super::{grid_columns, Layout};

    #[test]
    fn quick_access_columns_follow_the_available_width_and_scale() {
        assert_eq!(grid_columns(320, 6, 96), 1);
        assert_eq!(grid_columns(560, 6, 96), 2);
        assert_eq!(grid_columns(780, 6, 96), 3);
        assert_eq!(grid_columns(840, 6, 144), 2);
        assert_eq!(grid_columns(1170, 6, 144), 3);
        assert_eq!(grid_columns(780, 2, 96), 2);
        assert_eq!(grid_columns(780, 0, 96), 1);
    }

    #[test]
    fn content_mode_wraps_to_one_column_and_keeps_activity_reachable() {
        let desktop = Layout::new(880, 400, 6, 2, 96, false);
        let content = Layout::new(880, 400, 6, 2, 96, true);

        assert_eq!(desktop.columns, 3);
        assert_eq!(content.columns, 1);
        assert_eq!(desktop.max_scroll(), 0);
        assert!(content.activity_y - content.max_scroll() < content.viewport);
        assert!(content.max_scroll() > 0);
    }
}
