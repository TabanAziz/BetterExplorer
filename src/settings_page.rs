use crate::chrome::{
    blend_color, draw_glyph, fill, font_sized, move_to, new_control, rounded_fill, scale, send,
    set_visible, text, text_at, Glyph, Palette,
};
use crate::theme_visuals;
use better_explorer::model::ThemePreference;
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
pub const SECTIONS: [(u16, Section); 2] = [(227, Section::General), (228, Section::Appearance)];
pub const THEME_START: u16 = 230;
const SECTION_TOP: i32 = 150;
const SECTION_BOTTOM: i32 = 184;
const GALLERY_TOP: i32 = 197;
const GALLERY_COLUMNS: usize = 3;
const THEME_CHECK_SIZE: i32 = 12;
const THEME_CHECK_GAP: i32 = 3;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    General,
    Appearance,
}

impl Section {
    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
        }
    }
}

pub fn section(id: u16) -> Option<Section> {
    SECTIONS
        .iter()
        .find_map(|&(key, page)| (id == key).then_some(page))
}

pub fn theme(id: u16) -> Option<ThemePreference> {
    ThemePreference::ALL
        .get(id.checked_sub(THEME_START)? as usize)
        .copied()
}

pub fn is_button(id: u16) -> bool {
    section(id).is_some() || theme(id).is_some() || choice(id).is_some()
}

pub fn accessible_name(label: &str, selected: bool) -> String {
    if selected {
        format!("{label}, selected")
    } else {
        label.to_owned()
    }
}

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
        let general = layout.is_some_and(|layout| layout.section == Section::General);
        let appearance = layout.is_some_and(|layout| layout.section == Section::Appearance);
        let general_controls = [
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
        );
        for window in general_controls {
            set_visible(window, general);
        }
        for index in 0..ThemePreference::ALL.len() {
            set_visible(button(THEME_START + index as u16), appearance);
        }
        for (id, _) in SECTIONS {
            set_visible(button(id), layout.is_some());
        }
        set_visible(self.title, layout.is_some());
        let Some(layout) = layout else {
            return;
        };
        let d = |value| scale(value, layout.dpi);
        move_to(
            self.title,
            layout.left,
            d(104),
            layout.right - layout.left,
            d(44),
        );
        let position = |id, bounds: RECT| {
            move_to(
                button(id),
                bounds.left,
                bounds.top,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
            );
        };
        for (index, (id, _)) in SECTIONS.into_iter().enumerate() {
            position(id, layout.section_bounds(index));
        }
        if appearance {
            for index in 0..ThemePreference::ALL.len() {
                position(THEME_START + index as u16, layout.theme_bounds(index));
            }
            return;
        }
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
    section: Section,
    left: i32,
    right: i32,
    height: i32,
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
    pub fn new(sidebar: i32, width: i32, height: i32, dpi: u32, section: Section) -> Self {
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
            (row(210, 4, 42), row(305, 0, 54), 281)
        } else {
            (row(194, 62, 42), row(318, 48, 54), 308)
        };
        Self {
            section,
            left,
            right,
            height,
            label_right,
            divider: d(divider),
            shortcut,
            dates,
            dpi,
        }
    }

    fn section_bounds(self, index: usize) -> RECT {
        let d = |value| scale(value, self.dpi);
        let (left, right) = [(0, 77), (85, 191)][index];
        RECT {
            left: self.left + d(left),
            right: self.left + d(right),
            top: d(SECTION_TOP),
            bottom: d(SECTION_BOTTOM) + 1,
        }
    }

    fn theme_bounds(self, index: usize) -> RECT {
        let d = |value| scale(value, self.dpi);
        let gap = d(9);
        let columns = GALLERY_COLUMNS as i32;
        let rows = ThemePreference::ALL.len().div_ceil(GALLERY_COLUMNS) as i32;
        let column = (index % GALLERY_COLUMNS) as i32;
        let row = (index / GALLERY_COLUMNS) as i32;
        let width = (self.right - self.left - (columns - 1) * gap).max(0);
        let height =
            ((self.height - d(GALLERY_TOP + 12) - (rows - 1) * gap) / rows).clamp(0, d(110));
        let left = self.left + column * gap;
        let top = d(GALLERY_TOP) + row * (height + gap);
        RECT {
            left: left + width * column / columns,
            right: left + width * (column + 1) / columns,
            top,
            bottom: top + height,
        }
    }

    pub fn paint(self, dc: HDC, colors: &Palette) {
        let d = |value| scale(value, self.dpi);
        let line = |y| RECT {
            left: self.left,
            right: self.right,
            top: y,
            bottom: y + 1,
        };
        fill(dc, &line(d(SECTION_BOTTOM)), colors.edge);
        if self.section == Section::Appearance {
            return;
        }
        fill(dc, &line(self.divider), colors.edge);
        for track in [self.shortcut.track, self.dates.track] {
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

pub fn draw_section(
    draw: &DRAWITEMSTRUCT,
    section: Section,
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
    fill(
        dc,
        &RECT {
            top: bounds.bottom - 1,
            ..bounds
        },
        colors.edge,
    );
    let face = RECT {
        bottom: bounds.bottom - d(4),
        ..bounds
    };
    let surface = if selected && colors.contrast {
        colors.accent
    } else if selected {
        track_color(colors)
    } else if hovered || pressed {
        colors.field
    } else {
        colors.content
    };
    if focus {
        rounded_fill(dc, &face, d(6), colors.text);
        rounded_fill(dc, &inset(face, d(1)), d(5), surface);
    } else if surface != colors.content {
        rounded_fill(dc, &face, d(6), surface);
    }
    text_at(
        dc,
        section.label(),
        face,
        fonts[usize::from(selected)],
        if selected && colors.contrast {
            unsafe { GetSysColor(COLOR_HIGHLIGHTTEXT) }
        } else if selected || focus || hovered {
            colors.text
        } else {
            colors.muted
        },
        DT_SINGLELINE | DT_VCENTER | DT_CENTER,
    );
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
        left: preview.left + d(1),
        right: preview.right - d(1),
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
    use better_explorer::model::wide;
    use windows_sys::Win32::Foundation::SIZE;

    #[test]
    fn theme_previews_and_selected_labels_fit_at_minimum_window_size() {
        for dpi in [96, 144] {
            let d = |value| scale(value, dpi);
            let layout = Layout::new(d(151), d(552), d(450), dpi, Section::Appearance);
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
                assert!(tile.right <= d(552) && tile.bottom <= d(450));
                let (preview, caption) = theme_content(tile, dpi);
                assert!(preview.right - preview.left >= d(65));
                assert!(preview.bottom - preview.top >= d(29));
                let label_width =
                    caption.right - caption.left - d(THEME_CHECK_SIZE) - d(THEME_CHECK_GAP);
                assert!(size.cx <= label_width && size.cy <= caption.bottom - caption.top);
            }
        }
    }
}
