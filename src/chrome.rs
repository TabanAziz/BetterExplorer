use better_explorer::model::{from_wide, wide, ThemePreference};
use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::OsString,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, RECT, WPARAM},
    Graphics::{
        Dwm::{
            DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
            DWMWCP_ROUND,
        },
        Gdi::*,
    },
    System::{
        LibraryLoader::GetModuleHandleW,
        Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD},
        Threading::{GetCurrentProcessId, GetCurrentThreadId},
    },
    UI::{
        Accessibility::{
            SetWinEventHook, UnhookWinEvent, HCF_HIGHCONTRASTON, HIGHCONTRASTW, HWINEVENTHOOK,
        },
        Controls::*,
        HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
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
    pub field: u32,
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
                field: color(COLOR_WINDOW),
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
                chrome: rgb(33, 34, 39),
                title: rgb(36, 32, 48),
                sidebar: rgb(30, 31, 35),
                field: rgb(47, 44, 57),
                content: rgb(26, 27, 30),
                text: rgb(244, 243, 248),
                muted: rgb(184, 182, 196),
                edge: rgb(59, 58, 70),
                active: rgb(55, 52, 68),
                accent: rgb(255, 174, 92),
            }
        } else {
            Self {
                dark,
                contrast,
                chrome: rgb(248, 248, 252),
                title: rgb(237, 234, 248),
                sidebar: rgb(246, 246, 251),
                field: rgb(248, 246, 252),
                content: rgb(255, 255, 255),
                text: rgb(32, 31, 42),
                muted: rgb(94, 92, 111),
                edge: rgb(221, 219, 231),
                active: rgb(230, 225, 247),
                accent: rgb(158, 75, 21),
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
            "DarkMode_Explorer"
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
        let corners = DWMWCP_ROUND;
        DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&corners as *const i32).cast(),
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
        MoveWindow(window, x, y, width.max(0), height.max(0), 0);
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
    font_sized(dpi, 13, semibold)
}

pub fn font_sized(dpi: u32, size: i32, semibold: bool) -> HFONT {
    unsafe {
        CreateFontW(
            -scale(size, dpi),
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
        rounded_fill(dc, &r, scale(5, dpi), color);
        if style.active {
            fill(
                dc,
                &RECT {
                    left: r.left + inset,
                    right: r.right - inset,
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

struct ResourceIcon(HICON);

impl Drop for ResourceIcon {
    fn drop(&mut self) {
        unsafe {
            DestroyIcon(self.0);
        }
    }
}

thread_local! {
    static RESOURCE_ICONS: RefCell<HashMap<(u16, i32), ResourceIcon>> = RefCell::new(HashMap::new());
}

pub fn draw_resource_icon(dc: HDC, resource: u16, x: i32, y: i32, side: i32) -> bool {
    RESOURCE_ICONS.with_borrow_mut(|icons| {
        let key = (resource, side);
        if icons.len() >= 96 && !icons.contains_key(&key) {
            icons.clear();
        }
        if let std::collections::hash_map::Entry::Vacant(entry) = icons.entry(key) {
            let icon = unsafe {
                LoadImageW(
                    GetModuleHandleW(null()),
                    resource as usize as *const u16,
                    IMAGE_ICON,
                    side,
                    side,
                    0,
                )
            };
            if !icon.is_null() {
                entry.insert(ResourceIcon(icon));
            }
        }
        icons.get(&key).is_some_and(|icon| unsafe {
            DrawIconEx(dc, x, y, icon.0, side, side, 0, null_mut(), DI_NORMAL) != 0
        })
    })
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

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Glyph {
    Back,
    Forward,
    Up,
    Refresh,
    Add,
    NewTab,
    Close,
    Cut,
    Copy,
    Paste,
    Rename,
    Delete,
    Undo,
    Redo,
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
    Pushpin,
    Network,
    Library,
    Recycle,
    Minimize,
    Maximize,
    Restore,
    Chevron,
    Settings,
    Terminal,
    Link,
    Shortcut,
    Document,
    Archive,
    Check,
}

pub fn rounded_fill(dc: HDC, bounds: &RECT, radius: i32, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
        RoundRect(
            dc,
            bounds.left,
            bounds.top,
            bounds.right,
            bounds.bottom,
            radius * 2,
            radius * 2,
        );
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        DeleteObject(brush);
    }
}

pub struct PopupBorder {
    hook: HWINEVENTHOOK,
}

impl PopupBorder {
    pub fn apply() -> Self {
        let hook = unsafe {
            SetWinEventHook(
                EVENT_OBJECT_SHOW,
                EVENT_OBJECT_SHOW,
                null_mut(),
                Some(shape_popup),
                GetCurrentProcessId(),
                GetCurrentThreadId(),
                WINEVENT_OUTOFCONTEXT,
            )
        };
        Self { hook }
    }
}

impl Drop for PopupBorder {
    fn drop(&mut self) {
        if !self.hook.is_null() {
            unsafe { UnhookWinEvent(self.hook) };
        }
    }
}

unsafe extern "system" fn shape_popup(
    _: HWINEVENTHOOK,
    _: u32,
    window: HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    if object != OBJID_WINDOW || child != CHILDID_SELF as i32 || window.is_null() {
        return;
    }
    let mut class = [0u16; 8];
    let length = unsafe { GetClassNameW(window, class.as_mut_ptr(), class.len() as i32) };
    if length == 6
        && class[..6].iter().copied().eq("#32768".encode_utf16())
        && unsafe { IsWindowVisible(window) } != 0
    {
        clip_popup_frame(window);
    }
}

// Windows paints the frame in its native menu loop. Clip it on show, including submenus.
fn clip_popup_frame(window: HWND) {
    let mut bounds: RECT = unsafe { zeroed() };
    if unsafe { GetWindowRect(window, &mut bounds) } == 0 {
        return;
    }
    let dpi = unsafe { GetDpiForWindow(window) }.max(96);
    let x_edge = unsafe { GetSystemMetricsForDpi(SM_CXBORDER, dpi) }.max(1);
    let y_edge = unsafe { GetSystemMetricsForDpi(SM_CYBORDER, dpi) }.max(1);
    let width = bounds.right - bounds.left - 2 * x_edge;
    let height = bounds.bottom - bounds.top - 2 * y_edge;
    let diameter = scale(16, dpi).min(width).min(height);
    if diameter <= 1 {
        return;
    }
    // GDI leaves an extra pixel off the right and bottom of a rounded region.
    let region = unsafe {
        CreateRoundRectRgn(
            x_edge,
            y_edge,
            x_edge + width + 1,
            y_edge + height + 1,
            diameter,
            diameter,
        )
    };
    if !region.is_null() && unsafe { SetWindowRgn(window, region, 1) } == 0 {
        unsafe { DeleteObject(region) };
    }
}

struct IconBitmap {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    pixels: *mut u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum GlyphPaint {
    Plain(u32),
    Menu { dark: bool },
}

impl GlyphPaint {
    fn draw(self, dc: HDC, glyph: Glyph, x: i32, y: i32, size: i32) {
        match self {
            Self::Plain(color) => draw_glyph_shape(dc, glyph, x, y, size, color),
            Self::Menu { dark } => draw_menu_glyph_shape(dc, glyph, x, y, size, dark),
        }
    }
}

impl IconBitmap {
    fn new(dc: HDC, size: i32) -> Option<Self> {
        unsafe {
            let mut info: BITMAPINFO = zeroed();
            info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = size;
            info.bmiHeader.biHeight = -size;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            let mut pixels = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
            if bitmap.is_null() {
                return None;
            }
            let dc = CreateCompatibleDC(dc);
            if dc.is_null() {
                DeleteObject(bitmap);
                return None;
            }
            let previous = SelectObject(dc, bitmap);
            Some(Self {
                dc,
                bitmap,
                previous,
                pixels: pixels.cast(),
            })
        }
    }

    fn rasterize(dc: HDC, glyph: Glyph, size: i32, paint: GlyphPaint) -> Option<Self> {
        const SAMPLES: usize = 4;
        let source_size = size as usize * SAMPLES;
        let source = Self::new(dc, source_size as i32)?;
        let destination = Self::new(dc, size)?;
        unsafe {
            std::ptr::write_bytes(source.pixels, 0, source_size * source_size);
        }
        // Menu swatches are nonblack. Use white coverage for single-color glyphs so
        // system foreground colors such as pure black can still be drawn.
        let source_paint = match paint {
            GlyphPaint::Plain(_) => GlyphPaint::Plain(rgb(255, 255, 255)),
            menu => menu,
        };
        source_paint.draw(source.dc, glyph, 0, 0, source_size as i32);
        unsafe {
            GdiFlush();
        }
        let pixels =
            unsafe { std::slice::from_raw_parts(source.pixels, source_size * source_size) };
        let size = size as usize;
        for y in 0..size {
            for x in 0..size {
                let (mut red, mut green, mut blue, mut covered) = (0, 0, 0, 0);
                for dy in 0..SAMPLES {
                    let offset = (y * SAMPLES + dy) * source_size + x * SAMPLES;
                    for &pixel in &pixels[offset..offset + SAMPLES] {
                        if pixel & 0x00ff_ffff != 0 {
                            covered += 1;
                            red += (pixel >> 16) & 255;
                            green += (pixel >> 8) & 255;
                            blue += pixel & 255;
                        }
                    }
                }
                if let GlyphPaint::Plain(color) = paint {
                    red = (color & 255) * covered;
                    green = ((color >> 8) & 255) * covered;
                    blue = ((color >> 16) & 255) * covered;
                }
                let samples = (SAMPLES * SAMPLES) as u32;
                let average = |channel| (channel + samples / 2) / samples;
                let pixel = average(covered * 255) << 24
                    | average(red) << 16
                    | average(green) << 8
                    | average(blue);
                unsafe {
                    destination.pixels.add(y * size + x).write(pixel);
                }
            }
        }
        Some(destination)
    }
}

impl Drop for IconBitmap {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

type IconCache = HashMap<(Glyph, i32, GlyphPaint), IconBitmap>;

thread_local! {
    static ICONS: RefCell<IconCache> = RefCell::new(HashMap::new());
}

pub fn draw_glyph(dc: HDC, glyph: Glyph, x: i32, y: i32, size: i32, color: u32) {
    draw_cached_glyph(dc, glyph, x, y, size, GlyphPaint::Plain(color));
}

pub fn draw_menu_glyph(dc: HDC, glyph: Glyph, x: i32, y: i32, size: i32, dark: bool) {
    draw_cached_glyph(dc, glyph, x, y, size, GlyphPaint::Menu { dark });
}

fn draw_cached_glyph(dc: HDC, glyph: Glyph, x: i32, y: i32, size: i32, paint: GlyphPaint) {
    if !(6..=64).contains(&size) {
        paint.draw(dc, glyph, x, y, size);
        return;
    }
    let painted = ICONS.with_borrow_mut(|icons| {
        let key = (glyph, size, paint);
        if icons.len() >= 192 && !icons.contains_key(&key) {
            icons.clear();
        }
        if let std::collections::hash_map::Entry::Vacant(entry) = icons.entry(key) {
            if let Some(icon) = IconBitmap::rasterize(dc, glyph, size, paint) {
                entry.insert(icon);
            }
        }
        icons.get(&key).is_some_and(|icon| unsafe {
            GdiAlphaBlend(
                dc,
                x,
                y,
                size,
                size,
                icon.dc,
                0,
                0,
                size,
                size,
                BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                },
            ) != 0
        })
    });
    if !painted {
        paint.draw(dc, glyph, x, y, size);
    }
}

fn menu_glyph_colors(glyph: Glyph, dark: bool) -> (u32, u32) {
    let (dark_ink, dark_fill, light_ink, light_fill) = match glyph {
        Glyph::Cut | Glyph::Details | Glyph::Settings => (
            rgb(197, 172, 255),
            rgb(68, 52, 101),
            rgb(104, 57, 167),
            rgb(236, 225, 253),
        ),
        Glyph::Delete | Glyph::Recycle => (
            rgb(255, 139, 158),
            rgb(100, 45, 61),
            rgb(172, 45, 72),
            rgb(255, 220, 229),
        ),
        Glyph::Rename => (
            rgb(255, 177, 107),
            rgb(107, 64, 44),
            rgb(167, 68, 19),
            rgb(255, 230, 206),
        ),
        Glyph::Archive | Glyph::Folder | Glyph::Paste | Glyph::Pin | Glyph::Pushpin => (
            rgb(251, 207, 115),
            rgb(101, 76, 38),
            rgb(130, 82, 13),
            rgb(255, 240, 207),
        ),
        Glyph::Add | Glyph::Check | Glyph::Refresh => (
            rgb(124, 225, 165),
            rgb(36, 84, 62),
            rgb(26, 116, 70),
            rgb(219, 247, 231),
        ),
        Glyph::Link | Glyph::Shortcut | Glyph::Terminal | Glyph::Sort => (
            rgb(105, 221, 232),
            rgb(31, 78, 90),
            rgb(13, 111, 123),
            rgb(214, 247, 249),
        ),
        _ => (
            rgb(131, 188, 255),
            rgb(44, 72, 109),
            rgb(35, 88, 166),
            rgb(216, 233, 255),
        ),
    };
    if dark {
        (dark_ink, dark_fill)
    } else {
        (light_ink, light_fill)
    }
}

fn draw_menu_glyph_shape(dc: HDC, glyph: Glyph, x: i32, y: i32, size: i32, dark: bool) {
    use windows_sys::Win32::Foundation::POINT;

    let (ink, tint) = menu_glyph_colors(glyph, dark);
    let s = |value: i32| value * size / 16;
    let rect = |l, t, r, b| RECT {
        left: x + s(l),
        top: y + s(t),
        right: x + s(r),
        bottom: y + s(b),
    };
    let panel = |l, t, r, b, radius, color| {
        rounded_fill(dc, &rect(l, t, r, b), s(radius).max(1), color);
    };
    let points = |points: &[(i32, i32)]| -> Vec<POINT> {
        points
            .iter()
            .map(|&(px, py)| POINT {
                x: x + s(px),
                y: y + s(py),
            })
            .collect()
    };
    unsafe {
        let pen = CreatePen(PS_SOLID, s(1).max(1), ink);
        let brush = CreateSolidBrush(tint);
        let old_pen = SelectObject(dc, pen);
        let old_brush = SelectObject(dc, brush);
        let line = |vertices: &[(i32, i32)]| {
            let vertices = points(vertices);
            Polyline(dc, vertices.as_ptr(), vertices.len() as i32);
        };
        let polygon = |vertices: &[(i32, i32)]| {
            let vertices = points(vertices);
            Polygon(dc, vertices.as_ptr(), vertices.len() as i32);
        };
        match glyph {
            Glyph::Document => {
                polygon(&[(3, 2), (9, 2), (13, 6), (13, 14), (3, 14)]);
                line(&[(9, 2), (9, 6), (13, 6)]);
                line(&[(5, 9), (11, 9)]);
                line(&[(5, 11), (10, 11)]);
            }
            Glyph::Copy => {
                panel(1, 1, 11, 12, 2, ink);
                panel(2, 2, 10, 11, 1, tint);
                panel(5, 4, 15, 15, 2, ink);
                panel(6, 5, 14, 14, 1, tint);
                line(&[(8, 8), (12, 8)]);
                line(&[(8, 10), (12, 10)]);
            }
            Glyph::Cut => {
                line(&[(4, 2), (10, 10)]);
                line(&[(12, 2), (6, 10)]);
                Ellipse(dc, x + s(2), y + s(9), x + s(8), y + s(15));
                Ellipse(dc, x + s(8), y + s(9), x + s(14), y + s(15));
            }
            Glyph::Paste => {
                panel(2, 3, 14, 15, 2, ink);
                panel(3, 4, 13, 14, 1, tint);
                panel(5, 1, 11, 6, 1, ink);
                line(&[(5, 9), (11, 9)]);
                line(&[(5, 11), (10, 11)]);
            }
            Glyph::Rename => {
                polygon(&[(2, 13), (3, 9), (11, 1), (14, 4), (6, 12)]);
                line(&[(9, 3), (12, 6)]);
                line(&[(3, 9), (6, 12)]);
                line(&[(8, 14), (15, 14)]);
            }
            Glyph::Delete | Glyph::Recycle => {
                polygon(&[(4, 5), (12, 5), (11, 14), (5, 14)]);
                panel(2, 3, 14, 5, 1, ink);
                line(&[(6, 3), (6, 2), (10, 2), (10, 3)]);
                line(&[(7, 7), (7, 11)]);
                line(&[(9, 7), (9, 11)]);
            }
            Glyph::Archive => {
                panel(2, 5, 14, 15, 2, ink);
                panel(3, 6, 13, 14, 1, tint);
                panel(1, 2, 15, 6, 1, ink);
                line(&[(6, 9), (10, 9)]);
            }
            Glyph::Folder => {
                polygon(&[(2, 4), (6, 4), (8, 6), (14, 6), (14, 13), (2, 13)]);
                line(&[(2, 7), (14, 7)]);
            }
            Glyph::Pin => {
                polygon(&[
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
                ]);
            }
            Glyph::Pushpin => {
                polygon(&[
                    (6, 2),
                    (14, 10),
                    (12, 10),
                    (9, 8),
                    (8, 12),
                    (4, 8),
                    (8, 7),
                    (6, 4),
                ]);
                line(&[(6, 10), (2, 14)]);
            }
            Glyph::Restore => {
                panel(5, 2, 15, 11, 2, ink);
                panel(6, 3, 14, 10, 1, tint);
                panel(1, 5, 12, 14, 2, ink);
                panel(2, 7, 11, 13, 1, tint);
            }
            Glyph::Terminal | Glyph::View | Glyph::Details | Glyph::NewTab => {
                panel(1, 2, 15, 14, 2, tint);
                draw_glyph_shape(dc, glyph, x + s(1), y + s(1), s(14), ink);
            }
            _ => {
                panel(1, 1, 15, 15, 7, tint);
                draw_glyph_shape(dc, glyph, x + s(2), y + s(2), s(12), ink);
            }
        }
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        DeleteObject(brush);
        DeleteObject(pen);
    }
}

fn draw_glyph_shape(dc: HDC, glyph: Glyph, x: i32, y: i32, size: i32, color: u32) {
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
        let pen = CreatePen(PS_SOLID, (size / 12).max(1), color);
        let old = SelectObject(dc, pen);
        let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
        let round = |l, t, r, b| {
            Ellipse(
                dc,
                x + l * size / 16,
                y + t * size / 16,
                x + r * size / 16,
                y + b * size / 16,
            )
        };
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
            Glyph::NewTab => {
                box_(2, 3, 14, 13);
                stroke(&[(2, 6), (14, 6)]);
                stroke(&[(6, 9), (10, 9)]);
                stroke(&[(8, 7), (8, 11)]);
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
            Glyph::Undo => {
                stroke(&[(2, 7), (10, 7), (13, 9), (13, 11), (11, 13), (7, 13)]);
                stroke(&[(6, 3), (2, 7), (6, 11)]);
            }
            Glyph::Redo => {
                stroke(&[(14, 7), (6, 7), (3, 9), (3, 11), (5, 13), (9, 13)]);
                stroke(&[(10, 3), (14, 7), (10, 11)]);
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
                round(2, 2, 11, 11);
                stroke(&[(10, 10), (14, 14)]);
            }
            Glyph::Folder => stroke(&[(2, 4), (7, 4), (8, 6), (14, 6), (14, 13), (2, 13), (2, 4)]),
            Glyph::Drive => {
                box_(2, 3, 14, 11);
                stroke(&[(8, 11), (8, 14)]);
                stroke(&[(4, 14), (12, 14)]);
            }
            Glyph::Settings => {
                stroke(&[
                    (6, 2),
                    (10, 2),
                    (10, 4),
                    (11, 5),
                    (13, 4),
                    (15, 7),
                    (13, 8),
                    (13, 9),
                    (15, 10),
                    (13, 13),
                    (11, 12),
                    (10, 13),
                    (10, 15),
                    (6, 15),
                    (6, 13),
                    (5, 12),
                    (3, 13),
                    (1, 10),
                    (3, 9),
                    (3, 8),
                    (1, 7),
                    (3, 4),
                    (5, 5),
                    (6, 4),
                    (6, 2),
                ]);
                round(6, 6, 11, 11);
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
            Glyph::Pushpin => {
                stroke(&[
                    (6, 2),
                    (14, 10),
                    (12, 10),
                    (9, 8),
                    (8, 12),
                    (4, 8),
                    (8, 7),
                    (6, 4),
                    (6, 2),
                ]);
                stroke(&[(6, 10), (2, 14)]);
            }
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
            Glyph::Terminal => {
                box_(1, 3, 15, 13);
                stroke(&[(4, 6), (6, 8), (4, 10)]);
                stroke(&[(8, 10), (12, 10)]);
            }
            Glyph::Link => {
                stroke(&[(6, 5), (9, 2), (12, 2), (14, 4), (14, 7), (11, 10)]);
                stroke(&[(10, 11), (7, 14), (4, 14), (2, 12), (2, 9), (5, 6)]);
                stroke(&[(5, 11), (11, 5)]);
            }
            Glyph::Shortcut => {
                stroke(&[(8, 3), (3, 3), (3, 13), (13, 13), (13, 8)]);
                stroke(&[(7, 9), (13, 3)]);
                stroke(&[(9, 3), (13, 3), (13, 7)]);
            }
            Glyph::Document => {
                stroke(&[(4, 2), (10, 2), (13, 5), (13, 14), (4, 14), (4, 2)]);
                stroke(&[(10, 2), (10, 5), (13, 5)]);
                stroke(&[(6, 8), (11, 8)]);
                stroke(&[(6, 11), (11, 11)]);
            }
            Glyph::Archive => {
                box_(2, 6, 14, 14);
                box_(1, 2, 15, 6);
                stroke(&[(6, 9), (10, 9)]);
            }
            Glyph::Check => stroke(&[(3, 8), (7, 12), (14, 4)]),
        }
        SelectObject(dc, old_brush);
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
