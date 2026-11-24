use crate::chrome::{
    draw_glyph, fill, font_sized, move_to, new_control, rgb, rounded_fill, scale, send,
    set_visible, text, text_at, Glyph, Palette,
};
use std::ffi::OsStr;
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Gdi::*,
    System::SystemServices::SS_NOPREFIX,
    UI::{
        Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED},
        WindowsAndMessaging::*,
    },
};

pub const WIN_E_WINDOWS: u16 = 191;
pub const WIN_E_BETTER: u16 = 192;
pub const DATE_FORMAT_DAY_FIRST: u16 = 206;
pub const DATE_FORMAT_MONTH_FIRST: u16 = 207;
pub const CONTENT_TOP: i32 = 79;
const PAGE_TITLE: u16 = 221;
const SHORTCUT_HEADING: u16 = 222;
const SHORTCUT_STATUS: u16 = 224;
const DATE_HEADING: u16 = 225;
const DATE_HINT: u16 = 226;

pub fn is_muted_label(id: u16) -> bool {
    matches!(id, SHORTCUT_STATUS | DATE_HINT)
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

pub fn choice(id: u16) -> Option<Choice> {
    SHORTCUT_CHOICES
        .into_iter()
        .chain(DATE_CHOICES)
        .find(|choice| choice.id == id)
}

pub struct Page {
    fonts: [HFONT; 2],
    title: HWND,
    shortcut_heading: HWND,
    shortcut_status: HWND,
    date_heading: HWND,
    date_hint: HWND,
}

impl Page {
    pub fn create(window: HWND, dpi: u32, fonts: [HFONT; 2]) -> Self {
        let label = |id, caption| new_control(window, "Static", caption, id, SS_NOPREFIX, 0);
        let mut page = Self {
            fonts: [std::ptr::null_mut(); 2],
            title: label(PAGE_TITLE, "Settings"),
            shortcut_heading: label(SHORTCUT_HEADING, "Win+E shortcut"),
            shortcut_status: label(SHORTCUT_STATUS, ""),
            date_heading: label(DATE_HEADING, "Date modified format"),
            date_hint: label(DATE_HINT, "Used in file lists and item details."),
        };
        page.set_fonts(dpi, fonts);
        page
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
            (self.date_heading, self.fonts[1]),
            (self.date_hint, fonts[0]),
        ] {
            send(window, WM_SETFONT, font as usize, 0);
        }
        for font in old {
            if !font.is_null() {
                unsafe { DeleteObject(font) };
            }
        }
    }

    pub fn set_status(&self, value: impl AsRef<OsStr>) {
        text(self.shortcut_status, value);
    }

    pub fn layout(&self, layout: Option<Layout>, button: impl Fn(u16) -> HWND) {
        for window in [
            self.title,
            self.shortcut_heading,
            self.shortcut_status,
            self.date_heading,
            self.date_hint,
        ]
        .into_iter()
        .chain(
            SHORTCUT_CHOICES
                .into_iter()
                .chain(DATE_CHOICES)
                .map(|choice| button(choice.id)),
        ) {
            set_visible(window, layout.is_some());
        }
        let Some(layout) = layout else {
            return;
        };
        let d = |value| scale(value, layout.dpi);
        move_to(
            self.title,
            layout.left,
            d(108),
            layout.right - layout.left,
            d(46),
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
                layout.dates,
                self.date_heading,
                self.date_hint,
                20,
                DATE_CHOICES,
            ),
        ] {
            let width = layout.label_right - layout.left;
            move_to(heading, layout.left, row.top, width, d(24));
            move_to(hint, layout.left, row.top + d(26), width, d(hint_height));
            let inner = inset(row.track, d(4));
            let gap = d(3);
            let first = (inner.right - inner.left - gap) / 2;
            let y = inner.top;
            let height = inner.bottom - y;
            move_to(button(choices[0].id), inner.left, y, first, height);
            let second_x = inner.left + first + gap;
            move_to(
                button(choices[1].id),
                second_x,
                y,
                inner.right - second_x,
                height,
            );
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

#[derive(Clone, Copy)]
pub struct Layout {
    left: i32,
    right: i32,
    label_right: i32,
    divider: i32,
    shortcut: Row,
    dates: Row,
    dpi: u32,
}

#[derive(Clone, Copy)]
struct Row {
    top: i32,
    track: RECT,
}

impl Layout {
    pub fn new(sidebar: i32, width: i32, dpi: u32) -> Self {
        let d = |value| scale(value, dpi);
        let available = width - sidebar;
        let gutter = d(if available >= d(680) { 40 } else { 24 });
        let span = (available - 2 * gutter).clamp(0, d(720));
        let left = sidebar + (available - span) / 2;
        let right = left + span;
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
        let (shortcut, dates, divider) = if side_by_side {
            (row(180, 4, 42), row(274, 0, 54), 251)
        } else {
            (row(168, 62, 42), row(309, 54, 54), 292)
        };
        Self {
            left,
            right,
            label_right,
            divider: d(divider),
            shortcut,
            dates,
            dpi,
        }
    }

    pub fn paint(self, dc: HDC, colors: &Palette) {
        let d = |value| scale(value, self.dpi);
        fill(
            dc,
            &RECT {
                left: self.left,
                right: self.right,
                top: self.divider,
                bottom: self.divider + 1,
            },
            colors.edge,
        );
        for track in [self.shortcut.track, self.dates.track] {
            rounded_fill(dc, &track, d(10), track_color(colors));
        }
    }
}

fn track_color(colors: &Palette) -> u32 {
    if colors.contrast {
        colors.chrome
    } else if colors.dark {
        rgb(40, 40, 47)
    } else {
        rgb(238, 237, 244)
    }
}

fn inset(mut bounds: RECT, gap: i32) -> RECT {
    unsafe { InflateRect(&mut bounds, -gap, -gap) };
    bounds
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
            rgb(67, 64, 81)
        } else {
            colors.content
        }
    } else if interactive && !colors.contrast {
        if colors.dark {
            rgb(51, 50, 62)
        } else {
            rgb(229, 226, 239)
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
