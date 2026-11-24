use better_explorer::model::{from_wide, wide, ThemePreference};
use std::{
    ffi::OsString,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, RECT, WPARAM},
    Graphics::{
        Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE},
        Gdi::*,
    },
    System::{
        LibraryLoader::GetModuleHandleW,
        Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD},
    },
    UI::{
        Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
        Controls::*,
        WindowsAndMessaging::*,
    },
};

pub const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | g << 8 | b << 16
}

pub struct Palette {
    pub dark: bool,
    pub contrast: bool,
    pub chrome: u32,
    pub title: u32,
    pub sidebar: u32,
    pub content: u32,
    pub text: u32,
    pub muted: u32,
    pub edge: u32,
    pub active: u32,
    pub accent: u32,
}

impl Palette {
    pub fn read(preference: ThemePreference) -> Self {
        let mut high: HIGHCONTRASTW = unsafe { zeroed() };
        high.cbSize = size_of::<HIGHCONTRASTW>() as u32;
        let contrast = unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                high.cbSize,
                (&mut high as *mut HIGHCONTRASTW).cast(),
                0,
            );
            high.dwFlags & HCF_HIGHCONTRASTON != 0
        };
        if contrast {
            let color = |index| unsafe { GetSysColor(index) };
            return Self {
                dark: false,
                contrast,
                chrome: color(COLOR_BTNFACE),
                title: color(COLOR_BTNFACE),
                sidebar: color(COLOR_WINDOW),
                content: color(COLOR_WINDOW),
                text: color(COLOR_WINDOWTEXT),
                muted: color(COLOR_WINDOWTEXT),
                edge: color(COLOR_WINDOWTEXT),
                active: color(COLOR_WINDOW),
                accent: color(COLOR_HIGHLIGHT),
            };
        }
        let dark = match preference {
            ThemePreference::System => system_dark(),
            ThemePreference::Dark => true,
            ThemePreference::Light => false,
        };
        if dark {
            Self {
                dark,
                contrast,
                chrome: rgb(32, 30, 30),
                title: rgb(44, 34, 32),
                sidebar: rgb(32, 31, 30),
                content: rgb(26, 25, 25),
                text: rgb(242, 240, 238),
                muted: rgb(184, 179, 176),
                edge: rgb(63, 58, 55),
                active: rgb(68, 49, 39),
                accent: rgb(255, 155, 82),
            }
        } else {
            Self {
                dark,
                contrast,
                chrome: rgb(249, 246, 243),
                title: rgb(242, 229, 220),
                sidebar: rgb(251, 248, 246),
                content: rgb(255, 255, 255),
                text: rgb(32, 30, 28),
                muted: rgb(94, 86, 82),
                edge: rgb(229, 218, 211),
                active: rgb(250, 231, 217),
                accent: rgb(163, 68, 24),
            }
        }
    }
}

fn system_dark() -> bool {
    let mut value = 1u32;
    let mut len = size_of::<u32>() as u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize").as_ptr(),
            wide("AppsUseLightTheme").as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut value as *mut u32).cast(),
            &mut len,
        );
    }
    value == 0
}

pub fn set_theme(window: HWND, list: HWND, palette: &Palette) {
    let dark = i32::from(palette.dark);
    let style = if palette.contrast {
        Vec::new()
    } else {
        wide(if palette.dark {
            "DarkMode_ItemsView"
        } else {
            "Explorer"
        })
    };
    let style_ptr = if style.is_empty() {
        null()
    } else {
        style.as_ptr()
    };
    unsafe {
        DwmSetWindowAttribute(
            window,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&dark as *const i32).cast(),
            size_of::<i32>() as u32,
        );
        SetWindowTheme(list, style_ptr, null());
        let header = send(list, LVM_GETHEADER, 0, 0) as HWND;
        if !header.is_null() {
            SetWindowTheme(header, style_ptr, null());
        }
        send(list, LVM_SETBKCOLOR, 0, palette.content as LPARAM);
        send(list, LVM_SETTEXTBKCOLOR, 0, palette.content as LPARAM);
        send(list, LVM_SETTEXTCOLOR, 0, palette.text as LPARAM);
        InvalidateRect(window, null(), 1);
        RedrawWindow(window, null(), null_mut(), RDW_INVALIDATE | RDW_ALLCHILDREN);
    }
}

pub fn new_control(
    parent: HWND,
    class: &str,
    label: &str,
    id: u16,
    style: u32,
    extended: u32,
) -> HWND {
    unsafe {
        CreateWindowExW(
            extended,
            wide(class).as_ptr(),
            wide(label).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            0,
            0,
            parent,
            id as usize as HMENU,
            GetModuleHandleW(null()),
            null(),
        )
    }
}

pub fn button(parent: HWND, label: &str, id: u16) -> HWND {
    new_control(
        parent,
        "Button",
        label,
        id,
        WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
    )
}

pub fn text(window: HWND, value: impl AsRef<std::ffi::OsStr>) {
    unsafe {
        SetWindowTextW(window, wide(value).as_ptr());
    }
}

pub fn read(window: HWND) -> OsString {
    unsafe {
        let count = GetWindowTextLengthW(window);
        let mut buffer = vec![0u16; count.max(0) as usize + 1];
        GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32);
        from_wide(&buffer)
    }
}

pub fn rect(window: HWND) -> RECT {
    let mut bounds = unsafe { zeroed() };
    unsafe {
        GetClientRect(window, &mut bounds);
    }
    bounds
}

pub fn send(window: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> isize {
    unsafe { SendMessageW(window, message, wp, lp) }
}

pub fn move_to(window: HWND, x: i32, y: i32, width: i32, height: i32) {
    unsafe {
        MoveWindow(window, x, y, width.max(0), height.max(0), 1);
    }
}

pub fn set_visible(window: HWND, visible: bool) {
    unsafe {
        ShowWindow(window, if visible { SW_SHOW } else { SW_HIDE });
    }
}

pub fn set_enabled(window: HWND, enabled: bool) {
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow(window, enabled as i32);
    }
}

pub fn font(dpi: u32, semibold: bool) -> HFONT {
    unsafe {
        CreateFontW(
            -scale(13, dpi),
            0,
            0,
            0,
            if semibold { 600 } else { 400 },
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
    }
}

pub fn scale(logical: i32, dpi: u32) -> i32 {
    (logical * dpi as i32 + 48) / 96
}

pub fn fill(dc: HDC, rect: &RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(dc, rect, brush);
        DeleteObject(brush);
    }
}

pub fn draw_button(
    draw: &DRAWITEMSTRUCT,
    palette: &Palette,
    dpi: u32,
    font: HFONT,
    style: ButtonStyle,
) {
    let dc = draw.hDC;
    let bounds = draw.rcItem;
    let selected = draw.itemState & ODS_SELECTED != 0;
    let disabled = draw.itemState & ODS_DISABLED != 0;
    let hover = style.hover && !disabled;
    let inset = scale(2, dpi);
    fill(dc, &bounds, style.surface);
    if style.active || selected || hover {
        let r = RECT {
            left: bounds.left + inset,
            top: bounds.top + inset,
            right: bounds.right - inset,
            bottom: bounds.bottom - inset,
        };
        let color = if style.danger && hover && !palette.contrast {
            rgb(150, 39, 34)
        } else {
            palette.active
        };
        fill(dc, &r, color);
        if style.active {
            fill(
                dc,
                &RECT {
                    top: r.bottom - inset,
                    ..r
                },
                palette.accent,
            );
        }
    }
    let label = read(draw.hwndItem).to_string_lossy().into_owned();
    let height = bounds.bottom - bounds.top;
    let icon_size = scale(16, dpi);
    let icon_only = style.glyph.is_some() && !style.label;
    let mut text_bounds = bounds;
    text_bounds.left += scale(if style.left { 9 } else { 7 }, dpi);
    text_bounds.right -= scale(7, dpi);
    if let Some(glyph) = style.glyph {
        let x = if icon_only {
            (bounds.left + bounds.right - icon_size) / 2
        } else {
            text_bounds.left
        };
        let color = if disabled {
            palette.muted
        } else {
            style.icon_color
        };
        draw_glyph(
            dc,
            glyph,
            x,
            bounds.top + (height - icon_size) / 2,
            icon_size,
            color,
        );
        text_bounds.left += icon_size + scale(6, dpi);
    }
    if !icon_only {
        let color = if disabled {
            palette.muted
        } else {
            palette.text
        };
        let flags = DT_VCENTER
            | DT_SINGLELINE
            | if style.left || style.glyph.is_some() {
                DT_LEFT
            } else {
                DT_CENTER
            };
        text_at(dc, &label, text_bounds, font, color, flags);
    }
    if draw.itemState & ODS_FOCUS != 0 {
        unsafe {
            let mut focus = bounds;
            InflateRect(&mut focus, -2, -2);
            DrawFocusRect(dc, &focus);
        }
    }
}

pub struct ButtonStyle {
    pub glyph: Option<Glyph>,
    pub label: bool,
    pub left: bool,
    pub active: bool,
    pub danger: bool,
    pub hover: bool,
    pub surface: u32,
    pub icon_color: u32,
}

pub fn list_images(dpi: u32, side: i32) -> HIMAGELIST {
    let side = scale(side, dpi);
    unsafe {
        let list = ImageList_Create(side, side, ILC_COLOR32 | ILC_MASK, 3, 0);
        for resource in 2..=4 {
            let icon = LoadImageW(
                GetModuleHandleW(null()),
                resource as usize as *const u16,
                IMAGE_ICON,
                side,
                side,
                0,
            );
            if !icon.is_null() {
                ImageList_ReplaceIcon(list, -1, icon);
                DestroyIcon(icon);
            }
        }
        list
    }
}

#[derive(Clone, Copy)]
pub enum Glyph {
    Back,
    Forward,
    Up,
    Refresh,
    Add,
    Close,
    Cut,
    Copy,
    Paste,
    Rename,
    Delete,
    Sort,
    View,
    Filter,
    More,
    Details,
    Search,
    Folder,
    Drive,
    Home,
    Pin,
    Network,
    Library,
    Recycle,
    Minimize,
    Maximize,
    Restore,
    Chevron,
    Settings,
}

pub fn draw_glyph(dc: HDC, glyph: Glyph, x: i32, y: i32, size: i32, color: u32) {
    use windows_sys::Win32::Foundation::POINT;

    let stroke = |points: &[(i32, i32)]| {
        let points: Vec<POINT> = points
            .iter()
            .map(|&(px, py)| POINT {
                x: x + px * size / 16,
                y: y + py * size / 16,
            })
            .collect();
        unsafe {
            Polyline(dc, points.as_ptr(), points.len() as i32);
        }
    };
    let box_ = |left, top, right, bottom| {
        stroke(&[
            (left, top),
            (right, top),
            (right, bottom),
            (left, bottom),
            (left, top),
        ])
    };
    let dot = |px, py| stroke(&[(px, py), (px + 2, py)]);
    unsafe {
        let pen = CreatePen(PS_SOLID, (size / 13).max(1), color);
        let old = SelectObject(dc, pen);
        match glyph {
            Glyph::Back => {
                stroke(&[(13, 8), (3, 8)]);
                stroke(&[(7, 4), (3, 8), (7, 12)]);
            }
            Glyph::Forward => {
                stroke(&[(3, 8), (13, 8)]);
                stroke(&[(9, 4), (13, 8), (9, 12)]);
            }
            Glyph::Up => {
                stroke(&[(8, 14), (8, 3)]);
                stroke(&[(3, 8), (8, 3), (13, 8)]);
            }
            Glyph::Refresh => {
                stroke(&[
                    (13, 6),
                    (11, 3),
                    (6, 2),
                    (3, 5),
                    (2, 9),
                    (5, 13),
                    (10, 14),
                    (13, 11),
                ]);
                stroke(&[(10, 6), (14, 6), (14, 2)]);
            }
            Glyph::Add => {
                stroke(&[(3, 8), (13, 8)]);
                stroke(&[(8, 3), (8, 13)]);
            }
            Glyph::Close => {
                stroke(&[(4, 4), (12, 12)]);
                stroke(&[(4, 12), (12, 4)]);
            }
            Glyph::Cut => {
                stroke(&[
                    (4, 3),
                    (10, 10),
                    (13, 11),
                    (13, 14),
                    (10, 14),
                    (9, 11),
                    (12, 3),
                ]);
                stroke(&[(7, 9), (4, 10), (3, 13), (5, 15), (8, 13), (8, 10)]);
            }
            Glyph::Copy => {
                box_(6, 5, 14, 14);
                stroke(&[(4, 11), (2, 11), (2, 2), (10, 2), (10, 4)]);
            }
            Glyph::Paste => {
                stroke(&[(5, 4), (2, 4), (2, 14), (13, 14), (13, 4), (11, 4)]);
                box_(5, 2, 10, 6);
                stroke(&[(5, 9), (10, 9)]);
            }
            Glyph::Rename => {
                stroke(&[(2, 13), (3, 9), (11, 1), (14, 4), (6, 12), (2, 13)]);
                stroke(&[(8, 14), (15, 14)]);
            }
            Glyph::Delete | Glyph::Recycle => {
                stroke(&[(2, 4), (14, 4)]);
                stroke(&[(4, 5), (5, 14), (11, 14), (12, 5)]);
                stroke(&[(6, 3), (6, 2), (10, 2), (10, 3)]);
                stroke(&[(7, 6), (7, 11)]);
                stroke(&[(9, 6), (9, 11)]);
            }
            Glyph::Sort => {
                stroke(&[(1, 6), (4, 3), (7, 6)]);
                stroke(&[(4, 3), (4, 13)]);
                stroke(&[(9, 10), (12, 13), (15, 10)]);
                stroke(&[(12, 3), (12, 13)]);
            }
            Glyph::View => {
                for row in [4, 8, 12] {
                    dot(2, row);
                    stroke(&[(6, row), (14, row)]);
                }
            }
            Glyph::Filter => stroke(&[(2, 3), (14, 3), (9, 9), (9, 13), (7, 14), (7, 9), (2, 3)]),
            Glyph::More => {
                dot(2, 8);
                dot(7, 8);
                dot(12, 8);
            }
            Glyph::Details => {
                box_(2, 3, 14, 13);
                stroke(&[(9, 3), (9, 13)]);
                dot(10, 6);
                dot(10, 9);
            }
            Glyph::Search => {
                stroke(&[
                    (3, 3),
                    (8, 2),
                    (11, 5),
                    (11, 8),
                    (8, 11),
                    (4, 11),
                    (2, 8),
                    (2, 5),
                    (3, 3),
                ]);
                stroke(&[(10, 10), (14, 14)]);
            }
            Glyph::Folder => stroke(&[(2, 4), (7, 4), (8, 6), (14, 6), (14, 13), (2, 13), (2, 4)]),
            Glyph::Drive => {
                box_(2, 3, 14, 11);
                stroke(&[(8, 11), (8, 14)]);
                stroke(&[(4, 14), (12, 14)]);
            }
            Glyph::Settings => {
                for (row, handle) in [(3, 5), (8, 11), (13, 7)] {
                    stroke(&[(2, row), (14, row)]);
                    stroke(&[(handle, row - 2), (handle, row + 2)]);
                }
            }
            Glyph::Home => {
                stroke(&[(1, 7), (8, 2), (15, 7)]);
                stroke(&[
                    (3, 6),
                    (3, 14),
                    (7, 14),
                    (7, 10),
                    (10, 10),
                    (10, 14),
                    (13, 14),
                    (13, 6),
                ]);
            }
            Glyph::Pin => stroke(&[
                (8, 2),
                (10, 6),
                (14, 6),
                (11, 9),
                (12, 14),
                (8, 11),
                (4, 14),
                (5, 9),
                (2, 6),
                (6, 6),
                (8, 2),
            ]),
            Glyph::Network => {
                box_(6, 2, 10, 6);
                box_(1, 10, 5, 14);
                box_(11, 10, 15, 14);
                stroke(&[(3, 10), (3, 8), (13, 8), (13, 10)]);
                stroke(&[(8, 6), (8, 8)]);
            }
            Glyph::Library => {
                box_(2, 3, 5, 13);
                box_(6, 3, 9, 13);
                stroke(&[(10, 4), (12, 3), (15, 13), (13, 14), (10, 4)]);
            }
            Glyph::Minimize => stroke(&[(3, 8), (13, 8)]),
            Glyph::Maximize => box_(3, 3, 13, 13),
            Glyph::Restore => {
                box_(3, 5, 11, 13);
                stroke(&[(5, 4), (5, 3), (13, 3), (13, 11), (12, 11)]);
            }
            Glyph::Chevron => stroke(&[(6, 4), (10, 8), (6, 12)]),
        }
        SelectObject(dc, old);
        DeleteObject(pen);
    }
}

pub fn text_at(dc: HDC, value: &str, mut bounds: RECT, font: HFONT, color: u32, flags: u32) {
    let mut value = wide(value);
    unsafe {
        let old = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, color);
        DrawTextW(
            dc,
            value.as_mut_ptr(),
            -1,
            &mut bounds,
            flags | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        SelectObject(dc, old);
    }
}
