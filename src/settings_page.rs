use crate::chrome::{
    blend_color, button, draw_glyph, fill, font_sized, move_to, new_control, rounded_fill, scale,
    send, set_visible, text, text_at, Glyph, Palette,
};
use crate::theme_visuals;
use better_explorer::model::{wide, ThemePreference};
use std::{
    cell::Cell,
    ffi::OsStr,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, SystemServices::SS_NOPREFIX},
    UI::{
        Controls::{
            SetScrollInfo, SetWindowTheme, DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED,
        },
        HiDpi::GetSystemMetricsForDpi,
        Input::KeyboardAndMouse::SetFocus,
        WindowsAndMessaging::*,
    },
};

pub const WIN_E_WINDOWS: u16 = 191;
pub const WIN_E_BETTER: u16 = 192;
pub const DATE_FORMAT_DAY_FIRST: u16 = 206;
pub const DATE_FORMAT_MONTH_FIRST: u16 = 207;
pub const FOLDER_DEFAULT_WINDOWS: u16 = 219;
pub const FOLDER_DEFAULT_BETTER: u16 = 220;
pub const CONTENT_TOP: i32 = 79;
const PAGE_TITLE: u16 = 221;
const SHORTCUT_HEADING: u16 = 222;
const SHORTCUT_STATUS: u16 = 224;
const DATE_HEADING: u16 = 225;
const DATE_HINT: u16 = 226;
const FOLDER_DEFAULT_HEADING: u16 = 227;
const FOLDER_DEFAULT_STATUS: u16 = 228;
const THEME_HEADING: u16 = 229;
pub const THEME_START: u16 = 230;
const GALLERY_COLUMNS: i32 = 3;
const GALLERY_GAP: i32 = 8;
const MIN_THEME_WIDTH: i32 = 111;
const THEME_HEIGHT: i32 = 82;
const THEME_CHECK_SIZE: i32 = 12;
const THEME_CHECK_GAP: i32 = 3;

pub fn theme(id: u16) -> Option<ThemePreference> {
    ThemePreference::ALL
        .get(id.checked_sub(THEME_START)? as usize)
        .copied()
}

pub fn is_button(id: u16) -> bool {
    theme(id).is_some() || choice(id).is_some()
}

pub fn accessible_name(label: &str, selected: bool) -> String {
    if selected {
        format!("{label}, selected")
    } else {
        label.to_owned()
    }
}

pub fn is_muted_label(id: u16) -> bool {
    matches!(id, SHORTCUT_STATUS | DATE_HINT | FOLDER_DEFAULT_STATUS)
}

#[derive(Clone, Copy)]
pub struct Choice {
    pub id: u16,
    pub caption: &'static str,
    example: Option<&'static str>,
}

impl Choice {
    pub fn accessible_name(self, selected: bool) -> String {
        let mut name = self.caption.to_owned();
        if let Some(example) = self.example {
            name.push_str(", ");
            name.push_str(example);
        }
        if selected {
            name.push_str(", selected");
        }
        name
    }
}

pub const SHORTCUT_CHOICES: [Choice; 2] = [
    Choice {
        id: WIN_E_WINDOWS,
        caption: "File Explorer",
        example: None,
    },
    Choice {
        id: WIN_E_BETTER,
        caption: "BetterExplorer",
        example: None,
    },
];

pub const DATE_CHOICES: [Choice; 2] = [
    Choice {
        id: DATE_FORMAT_DAY_FIRST,
        caption: "Day first",
        example: Some("13.11.2026 14:35"),
    },
    Choice {
        id: DATE_FORMAT_MONTH_FIRST,
        caption: "Month first",
        example: Some("11/13/2026 14:35"),
    },
];

pub const FOLDER_DEFAULT_CHOICES: [Choice; 2] = [
    Choice {
        id: FOLDER_DEFAULT_WINDOWS,
        caption: "Windows default",
        example: None,
    },
    Choice {
        id: FOLDER_DEFAULT_BETTER,
        caption: "BetterExplorer",
        example: None,
    },
];

pub fn choice(id: u16) -> Option<Choice> {
    SHORTCUT_CHOICES
        .into_iter()
        .chain(FOLDER_DEFAULT_CHOICES)
        .chain(DATE_CHOICES)
        .find(|choice| choice.id == id)
}

pub struct Page {
    window: HWND,
    scrollbar: HWND,
    fonts: [HFONT; 2],
    title: HWND,
    shortcut_heading: HWND,
    shortcut_status: HWND,
    folder_default_heading: HWND,
    folder_default_status: HWND,
    date_heading: HWND,
    date_hint: HWND,
    theme_heading: HWND,
    layout: Cell<Option<Layout>>,
    scroll: Cell<i32>,
    wheel_fraction: Cell<i32>,
}

impl Page {
    pub fn create(parent: HWND, dpi: u32, fonts: [HFONT; 2], procedure: WNDPROC) -> Option<Self> {
        let window = unsafe {
            let module = GetModuleHandleW(null());
            let class = wide("BetterExplorerSettings");
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: procedure,
                hInstance: module,
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                lpszClassName: class.as_ptr(),
                ..zeroed()
            });
            CreateWindowExW(
                WS_EX_CONTROLPARENT,
                class.as_ptr(),
                null(),
                WS_CHILD | WS_CLIPCHILDREN,
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
        let label = |id, caption| new_control(window, "Static", caption, id, SS_NOPREFIX, 0);
        let mut page = Self {
            window,
            scrollbar: new_control(window, "ScrollBar", "", 0, SBS_VERT as u32, 0),
            fonts: [null_mut(); 2],
            title: label(PAGE_TITLE, "Settings"),
            shortcut_heading: label(SHORTCUT_HEADING, "Win+E shortcut"),
            shortcut_status: label(SHORTCUT_STATUS, ""),
            folder_default_heading: label(FOLDER_DEFAULT_HEADING, "Default folder manager"),
            folder_default_status: label(FOLDER_DEFAULT_STATUS, ""),
            date_heading: label(DATE_HEADING, "Date modified format"),
            date_hint: label(DATE_HINT, "Used in file lists and item details."),
            theme_heading: label(THEME_HEADING, "Themes"),
            layout: Cell::new(None),
            scroll: Cell::new(0),
            wheel_fraction: Cell::new(0),
        };
        let failed = SHORTCUT_CHOICES
            .into_iter()
            .chain(FOLDER_DEFAULT_CHOICES)
            .chain(DATE_CHOICES)
            .map(|choice| (choice.id, choice.caption))
            .chain(
                ThemePreference::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(index, theme)| (THEME_START + index as u16, theme.label())),
            )
            .any(|(id, label)| button(window, label, id).is_null());
        if failed
            || [
                page.scrollbar,
                page.title,
                page.shortcut_heading,
                page.shortcut_status,
                page.folder_default_heading,
                page.folder_default_status,
                page.date_heading,
                page.date_hint,
                page.theme_heading,
            ]
            .into_iter()
            .any(|control| control.is_null())
        {
            unsafe { DestroyWindow(window) };
            return None;
        }
        page.set_fonts(dpi, fonts);
        Some(page)
    }

    pub fn button(&self, id: u16) -> HWND {
        unsafe { GetDlgItem(self.window, id.into()) }
    }

    pub fn buttons(&self) -> impl Iterator<Item = HWND> + '_ {
        SHORTCUT_CHOICES
            .into_iter()
            .chain(FOLDER_DEFAULT_CHOICES)
            .chain(DATE_CHOICES)
            .map(|choice| choice.id)
            .chain(THEME_START..THEME_START + ThemePreference::ALL.len() as u16)
            .map(|id| self.button(id))
    }

    pub fn set_fonts(&mut self, dpi: u32, fonts: [HFONT; 2]) {
        let old = std::mem::replace(
            &mut self.fonts,
            [font_sized(dpi, 32, true), font_sized(dpi, 16, true)],
        );
        for (window, font) in [
            (self.title, self.fonts[0]),
            (self.shortcut_heading, self.fonts[1]),
            (self.shortcut_status, fonts[0]),
            (self.folder_default_heading, self.fonts[1]),
            (self.folder_default_status, fonts[0]),
            (self.date_heading, self.fonts[1]),
            (self.date_hint, fonts[0]),
            (self.theme_heading, self.fonts[1]),
        ] {
            send(window, WM_SETFONT, font as usize, 0);
        }
        for button in self.buttons() {
            send(button, WM_SETFONT, fonts[0] as usize, 0);
        }
        for font in old {
            if !font.is_null() {
                unsafe { DeleteObject(font) };
            }
        }
    }

    pub fn set_style(&self, palette: &Palette) {
        let theme = wide(if palette.dark {
            "DarkMode_Explorer"
        } else {
            "Explorer"
        });
        unsafe {
            SetWindowTheme(
                self.scrollbar,
                if palette.contrast {
                    null()
                } else {
                    theme.as_ptr()
                },
                null(),
            );
        }
    }

    pub fn set_status(&self, value: impl AsRef<OsStr>) {
        text(self.shortcut_status, value);
    }

    pub fn set_folder_default_status(&self, value: impl AsRef<OsStr>) {
        text(self.folder_default_status, value);
    }

    pub fn layout(&self, sidebar: i32, width: i32, height: i32, dpi: u32) {
        let top = scale(CONTENT_TOP, dpi);
        let width = (width - sidebar).max(0);
        let height = (height - top).max(0);
        let full = Layout::new(width, height, dpi);
        let scroll_width = if full.max_scroll() > 0 {
            scrollbar_width(dpi).min(width)
        } else {
            0
        };
        let layout = Layout::new(width - scroll_width, height, dpi);
        if let Some(previous) = self.layout.replace(Some(layout)) {
            if previous.dpi != dpi {
                let old_dpi = previous.dpi as i32;
                self.scroll
                    .set((self.scroll.get() * dpi as i32 + old_dpi / 2) / old_dpi);
            }
        }
        let offset = self.scroll.get().clamp(0, layout.max_scroll());
        self.scroll.set(offset);
        move_to(self.window, sidebar, top, width, height);
        self.place_controls(layout, offset);
        if scroll_width > 0 {
            move_to(
                self.scrollbar,
                width - scroll_width,
                0,
                scroll_width,
                height,
            );
            unsafe {
                SetScrollInfo(
                    self.scrollbar,
                    SB_CTL,
                    &SCROLLINFO {
                        cbSize: size_of::<SCROLLINFO>() as u32,
                        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                        nMin: 0,
                        nMax: layout.content_height() - 1,
                        nPage: height as u32,
                        nPos: offset,
                        nTrackPos: 0,
                    },
                    1,
                );
            }
        }
        set_visible(self.scrollbar, scroll_width > 0);
        set_visible(self.window, true);
    }

    pub fn hide(&self) {
        set_visible(self.window, false);
    }

    pub fn focus_first(&self) {
        self.scroll_to(0);
        unsafe {
            let first = GetNextDlgTabItem(self.window, null_mut(), 0);
            if !first.is_null() {
                SetFocus(first);
            }
        }
    }

    pub fn is_under_pointer(&self, lp: LPARAM) -> bool {
        let point = POINT {
            x: lp as i16 as i32,
            y: (lp >> 16) as i16 as i32,
        };
        let mut bounds = unsafe { zeroed() };
        unsafe { GetWindowRect(self.window, &mut bounds) != 0 && PtInRect(&bounds, point) != 0 }
    }

    pub fn reveal(&self, target: HWND) {
        let Some(layout) = self.layout.get() else {
            return;
        };
        if unsafe { GetParent(target) } != self.window {
            return;
        }
        let mut bounds = crate::chrome::rect(target);
        unsafe { MapWindowPoints(target, self.window, (&mut bounds as *mut RECT).cast(), 2) };
        let inset = scale(4, layout.dpi);
        let position = self.scroll.get();
        if bounds.top < inset {
            self.scroll_to(position + bounds.top - inset);
        } else if bounds.bottom + inset > layout.height {
            self.scroll_to(position + bounds.bottom + inset - layout.height);
        }
    }

    pub fn scroll_bar(&self, wp: WPARAM) {
        let Some(layout) = self.layout.get() else {
            return;
        };
        let offset = self.scroll.get();
        let line = scale(26, layout.dpi);
        let next = match wp as u16 as i32 {
            SB_TOP => 0,
            SB_BOTTOM => layout.max_scroll(),
            SB_LINEUP => offset - line,
            SB_LINEDOWN => offset + line,
            SB_PAGEUP => offset - layout.height,
            SB_PAGEDOWN => offset + layout.height,
            SB_THUMBTRACK => {
                let mut info = SCROLLINFO {
                    cbSize: size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_TRACKPOS,
                    ..unsafe { zeroed() }
                };
                if unsafe { GetScrollInfo(self.scrollbar, SB_CTL, &mut info) } == 0 {
                    return;
                }
                info.nTrackPos
            }
            SB_THUMBPOSITION => (wp >> 16) as u16 as i32,
            _ => return,
        };
        self.scroll_to(next);
    }

    pub fn wheel(&self, delta: i16) {
        let Some(layout) = self.layout.get() else {
            return;
        };
        let mut lines = 3u32;
        unsafe {
            SystemParametersInfoW(
                SPI_GETWHEELSCROLLLINES,
                0,
                (&mut lines as *mut u32).cast(),
                0,
            );
        }
        let distance = if lines == u32::MAX {
            layout.height
        } else {
            (lines as i32).saturating_mul(scale(26, layout.dpi))
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

    fn scroll_to(&self, position: i32) {
        let Some(layout) = self.layout.get() else {
            return;
        };
        let offset = position.clamp(0, layout.max_scroll());
        if self.scroll.replace(offset) == offset {
            return;
        }
        self.place_controls(layout, offset);
        unsafe {
            SetScrollInfo(
                self.scrollbar,
                SB_CTL,
                &SCROLLINFO {
                    cbSize: size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_POS,
                    nPos: offset,
                    ..zeroed()
                },
                1,
            );
            RedrawWindow(
                self.window,
                null(),
                null_mut(),
                RDW_INVALIDATE | RDW_ALLCHILDREN,
            );
        }
    }

    fn place_controls(&self, layout: Layout, offset: i32) {
        let d = |value| scale(value, layout.dpi);
        let position = |control, bounds: RECT| {
            move_to(
                control,
                bounds.left,
                bounds.top - offset,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
            );
        };
        let label = |top, height: i32, right| RECT {
            left: layout.left,
            right,
            top,
            bottom: top + d(height),
        };
        position(self.title, label(d(20), 44, layout.right));
        position(
            self.theme_heading,
            label(layout.theme_heading, 24, layout.right),
        );
        for (row, heading, hint, hint_height, choices) in [
            (
                layout.shortcut,
                self.shortcut_heading,
                self.shortcut_status,
                36,
                SHORTCUT_CHOICES,
            ),
            (
                layout.folder_default,
                self.folder_default_heading,
                self.folder_default_status,
                36,
                FOLDER_DEFAULT_CHOICES,
            ),
            (
                layout.dates,
                self.date_heading,
                self.date_hint,
                20,
                DATE_CHOICES,
            ),
        ] {
            position(heading, label(row.top, 24, layout.label_right));
            position(
                hint,
                label(row.top + d(25), hint_height, layout.label_right),
            );
            let inner = inset(row.track, d(4));
            let gap = d(3);
            let first = (inner.right - inner.left - gap) / 2;
            position(
                self.button(choices[0].id),
                RECT {
                    right: inner.left + first,
                    ..inner
                },
            );
            position(
                self.button(choices[1].id),
                RECT {
                    left: inner.left + first + gap,
                    ..inner
                },
            );
        }
        for index in 0..ThemePreference::ALL.len() {
            position(
                self.button(THEME_START + index as u16),
                layout.theme_bounds(index),
            );
        }
    }

    pub fn paint(&self, dc: HDC, update: &RECT, colors: &Palette) {
        fill(dc, update, colors.content);
        if let Some(layout) = self.layout.get() {
            layout.paint(dc, self.scroll.get(), colors);
        }
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        for font in self.fonts {
            if !font.is_null() {
                unsafe { DeleteObject(font) };
            }
        }
    }
}

fn scrollbar_width(dpi: u32) -> i32 {
    unsafe { GetSystemMetricsForDpi(SM_CXVSCROLL, dpi) }.max(scale(12, dpi))
}

#[derive(Clone, Copy)]
struct Layout {
    left: i32,
    right: i32,
    height: i32,
    label_right: i32,
    dividers: [i32; 3],
    shortcut: Row,
    folder_default: Row,
    dates: Row,
    theme_heading: i32,
    gallery: i32,
    columns: usize,
    dpi: u32,
}

#[derive(Clone, Copy)]
struct Row {
    top: i32,
    track: RECT,
}

impl Layout {
    fn new(width: i32, height: i32, dpi: u32) -> Self {
        let d = |value| scale(value, dpi);
        let gutter = d(if width >= d(680) { 40 } else { 12 });
        let span = (width - 2 * gutter).clamp(0, d(720));
        let left = (width - span) / 2;
        let right = left + span;
        let gap = d(GALLERY_GAP);
        let columns = ((span + gap) / (d(MIN_THEME_WIDTH) + gap)).clamp(1, GALLERY_COLUMNS);
        let side_by_side = span >= d(590);
        let track_width = if side_by_side { d(304) } else { span };
        let track_left = right - track_width;
        let label_right = if side_by_side {
            track_left - d(24)
        } else {
            right
        };
        let row = |top, offset, height| Row {
            top: d(top),
            track: RECT {
                left: track_left,
                right,
                top: d(top + offset),
                bottom: d(top + offset + height),
            },
        };
        let (shortcut, folder_default, dates, dividers, theme_heading) = if side_by_side {
            (
                row(72, 2, 42),
                row(148, 2, 42),
                row(224, 0, 54),
                [137, 213, 291],
                304,
            )
        } else {
            (
                row(72, 63, 42),
                row(199, 63, 42),
                row(326, 46, 54),
                [188, 315, 437],
                449,
            )
        };
        Self {
            left,
            right,
            height,
            label_right,
            dividers: dividers.map(d),
            shortcut,
            folder_default,
            dates,
            theme_heading: d(theme_heading),
            gallery: d(theme_heading + 24),
            columns: columns as usize,
            dpi,
        }
    }

    fn theme_bounds(self, index: usize) -> RECT {
        let d = |value| scale(value, self.dpi);
        let gap = d(GALLERY_GAP);
        let columns = self.columns as i32;
        let column = (index % self.columns) as i32;
        let row = (index / self.columns) as i32;
        let width = (self.right - self.left - (columns - 1) * gap).max(0);
        let left = self.left + column * gap;
        let row_top = row * (THEME_HEIGHT + GALLERY_GAP);
        RECT {
            left: left + width * column / columns,
            right: left + width * (column + 1) / columns,
            top: self.gallery + d(row_top),
            bottom: self.gallery + d(row_top + THEME_HEIGHT),
        }
    }

    fn content_height(self) -> i32 {
        self.theme_bounds(ThemePreference::ALL.len() - 1).bottom + scale(7, self.dpi)
    }

    fn max_scroll(self) -> i32 {
        (self.content_height() - self.height).max(0)
    }

    fn paint(self, dc: HDC, offset: i32, colors: &Palette) {
        let d = |value| scale(value, self.dpi);
        for y in self.dividers {
            fill(
                dc,
                &RECT {
                    left: self.left,
                    right: self.right,
                    top: y - offset,
                    bottom: y - offset + 1,
                },
                colors.edge,
            );
        }
        for track in [
            self.shortcut.track,
            self.folder_default.track,
            self.dates.track,
        ] {
            let track = RECT {
                top: track.top - offset,
                bottom: track.bottom - offset,
                ..track
            };
            rounded_fill(dc, &track, d(10), track_color(colors));
        }
    }
}

fn track_color(colors: &Palette) -> u32 {
    if colors.contrast {
        return colors.chrome;
    }
    let weight = if colors.dark { 42 } else { 50 };
    blend_color(colors.content, colors.edge, weight)
}

fn inset(mut bounds: RECT, gap: i32) -> RECT {
    unsafe { InflateRect(&mut bounds, -gap, -gap) };
    bounds
}

pub fn draw_theme(
    draw: &DRAWITEMSTRUCT,
    theme: ThemePreference,
    colors: &Palette,
    dpi: u32,
    fonts: [HFONT; 2],
    selected: bool,
    hovered: bool,
) {
    let dc = draw.hDC;
    let bounds = draw.rcItem;
    let d = |value| scale(value, dpi);
    let focus = draw.itemState & ODS_FOCUS != 0;
    let pressed = draw.itemState & ODS_SELECTED != 0;
    fill(dc, &bounds, colors.content);
    if focus {
        rounded_fill(dc, &bounds, d(8), colors.text);
        rounded_fill(dc, &inset(bounds, d(2)), d(6), colors.content);
    }
    let face = inset(bounds, d(2));
    let surface = if selected {
        if colors.contrast {
            colors.accent
        } else {
            track_color(colors)
        }
    } else if hovered || pressed {
        colors.field
    } else {
        colors.content
    };
    if selected {
        rounded_fill(
            dc,
            &face,
            d(6),
            if colors.contrast {
                colors.accent
            } else {
                colors.edge
            },
        );
        rounded_fill(dc, &inset(face, d(1)), d(5), surface);
    } else if surface != colors.content {
        rounded_fill(dc, &face, d(6), surface);
    }
    let (preview, mut caption) = theme_content(bounds, dpi);
    if preview.right > preview.left && preview.bottom > preview.top {
        rounded_fill(dc, &preview, d(5), colors.edge);
        theme_visuals::paint_preview(dc, inset(preview, d(1)), theme, dpi);
    }
    let color = if selected && colors.contrast {
        unsafe { GetSysColor(COLOR_HIGHLIGHTTEXT) }
    } else {
        colors.text
    };
    if selected {
        let size = d(THEME_CHECK_SIZE);
        draw_glyph(
            dc,
            Glyph::Check,
            caption.right - size,
            (caption.top + caption.bottom - size) / 2,
            size,
            color,
        );
        caption.right -= size + d(THEME_CHECK_GAP);
    }
    text_at(
        dc,
        theme.label(),
        caption,
        fonts[usize::from(selected)],
        color,
        DT_SINGLELINE | DT_VCENTER | DT_LEFT,
    );
}

fn theme_content(bounds: RECT, dpi: u32) -> (RECT, RECT) {
    let d = |value| scale(value, dpi);
    let preview = RECT {
        left: bounds.left + d(6),
        top: bounds.top + d(6),
        right: bounds.right - d(6),
        bottom: bounds.bottom - d(25),
    };
    let caption = RECT {
        left: preview.left,
        right: preview.right,
        top: preview.bottom + d(1),
        bottom: bounds.bottom - d(3),
    };
    (preview, caption)
}

pub fn draw_choice(
    draw: &DRAWITEMSTRUCT,
    choice: Choice,
    colors: &Palette,
    dpi: u32,
    fonts: [HFONT; 2],
    selected: bool,
    hovered: bool,
) {
    let dc = draw.hDC;
    let bounds = draw.rcItem;
    let d = |value| scale(value, dpi);
    let enabled = draw.itemState & ODS_DISABLED == 0;
    let interactive = enabled && (hovered || draw.itemState & ODS_SELECTED != 0);
    let focus = enabled && draw.itemState & ODS_FOCUS != 0;
    let track = track_color(colors);
    let surface = if selected {
        if colors.contrast {
            colors.accent
        } else if colors.dark {
            colors.active
        } else {
            colors.content
        }
    } else if interactive && !colors.contrast {
        if colors.dark {
            colors.field
        } else {
            colors.active
        }
    } else {
        track
    };
    fill(dc, &bounds, track);
    if focus {
        rounded_fill(dc, &bounds, d(7), colors.muted);
        rounded_fill(dc, &inset(bounds, d(1)), d(6), surface);
    } else if surface != track {
        rounded_fill(dc, &bounds, d(7), surface);
    }
    let primary = if colors.contrast && selected {
        unsafe { GetSysColor(COLOR_HIGHLIGHTTEXT) }
    } else if (enabled && selected) || focus || interactive {
        colors.text
    } else {
        colors.muted
    };
    let label = inset(bounds, d(4));
    let font = fonts[usize::from(selected)];
    let flags = DT_SINGLELINE | DT_VCENTER | DT_CENTER;
    let caption = |mut row: RECT| {
        if selected {
            let check_size = d(13);
            draw_glyph(
                dc,
                Glyph::Check,
                row.left + d(5),
                (row.top + row.bottom - check_size) / 2,
                check_size,
                primary,
            );
            row.left += d(12);
        }
        text_at(dc, choice.caption, row, font, primary, flags);
    };
    if let Some(example) = choice.example {
        let middle = (label.top + label.bottom) / 2;
        caption(RECT {
            bottom: middle + d(2),
            ..label
        });
        text_at(
            dc,
            example,
            RECT {
                top: middle - d(1),
                ..label
            },
            fonts[0],
            if colors.contrast {
                primary
            } else {
                colors.muted
            },
            flags,
        );
    } else {
        caption(label);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::font;
    use windows_sys::Win32::Foundation::SIZE;

    #[test]
    fn full_size_settings_stack_choices_and_scroll_to_all_themes() {
        for dpi in [96, 120, 144, 168, 192] {
            let d = |value| scale(value, dpi);
            let layout = Layout::new(
                d(968) - d(161) - 1 - d(7),
                d(608) - d(CONTENT_TOP) - d(7),
                dpi,
            );
            assert!(layout.shortcut.track.bottom < layout.folder_default.top);
            assert!(layout.folder_default.track.bottom < layout.dates.top);
            assert!(layout.dates.track.bottom < layout.theme_heading);
            assert!(layout.theme_heading < layout.theme_bounds(0).top);
            assert!(layout.max_scroll() > 0, "at {dpi} DPI");
            assert!(layout.theme_bounds(8).bottom - layout.max_scroll() <= layout.height);
        }
    }

    #[test]
    fn theme_previews_and_selected_labels_fit_at_minimum_window_size() {
        for (window_width, dpi) in [552, 512]
            .into_iter()
            .flat_map(|width| [96, 120, 144, 192].into_iter().map(move |dpi| (width, dpi)))
        {
            let d = |value| scale(value, dpi);
            let width = d(window_width) - d(151) - 1 - d(7) - scrollbar_width(dpi);
            let height = d(450) - d(CONTENT_TOP) - d(7);
            let layout = Layout::new(width, height, dpi);
            assert!(layout.max_scroll() > 0);
            let labels = unsafe {
                let dc = CreateCompatibleDC(std::ptr::null_mut());
                assert!(!dc.is_null());
                let font = font(dpi, true);
                let previous = SelectObject(dc, font);
                let labels = ThemePreference::ALL.map(|theme| {
                    let label = wide(theme.label());
                    let mut size = SIZE { cx: 0, cy: 0 };
                    assert_ne!(
                        GetTextExtentPoint32W(
                            dc,
                            label.as_ptr(),
                            label.len() as i32 - 1,
                            &mut size
                        ),
                        0
                    );
                    size
                });
                SelectObject(dc, previous);
                DeleteObject(font);
                DeleteDC(dc);
                labels
            };
            for (index, size) in labels.into_iter().enumerate() {
                let tile = layout.theme_bounds(index);
                assert!(tile.right <= width && tile.bottom - layout.max_scroll() <= height);
                let (preview, caption) = theme_content(tile, dpi);
                assert!(preview.right - preview.left >= d(65));
                assert!(preview.bottom - preview.top >= d(29));
                let label_width =
                    caption.right - caption.left - d(THEME_CHECK_SIZE) - d(THEME_CHECK_GAP);
                assert!(
                    size.cx <= label_width && size.cy <= caption.bottom - caption.top,
                    "{} at {window_width}px and {dpi} DPI, label width {}, available {label_width}",
                    ThemePreference::ALL[index].label(),
                    size.cx,
                );
            }
        }
    }
}
