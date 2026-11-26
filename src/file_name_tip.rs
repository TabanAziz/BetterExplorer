use crate::chrome::{scale, send, Palette};
use std::{
    cell::{Cell, RefCell},
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
    Graphics::Gdi::*,
    UI::{
        Controls::*,
        Input::KeyboardAndMouse::{
            TrackMouseEvent, TME_CANCEL, TME_HOVER, TME_LEAVE, TRACKMOUSEEVENT,
        },
        Shell::{DefSubclassProc, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

static EMPTY_TEXT: [u16; 1] = [0];

pub struct FileNameTip {
    list: HWND,
    window: Cell<HWND>,
    row: Cell<Option<usize>>,
    text: RefCell<Vec<u16>>,
}

impl FileNameTip {
    pub fn new(list: HWND) -> Self {
        Self {
            list,
            window: Cell::new(null_mut()),
            row: Cell::new(None),
            text: RefCell::default(),
        }
    }

    pub fn hover(&self, row: Option<usize>) {
        if self.row.replace(row) == row {
            return;
        }
        let tip = self.window.get();
        if !tip.is_null() {
            send(
                tip,
                TTM_TRACKACTIVATE,
                0,
                &self.tool() as *const _ as LPARAM,
            );
        }
        let mut tracking = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: if row.is_some() {
                TME_HOVER | TME_LEAVE
            } else {
                TME_CANCEL | TME_HOVER
            },
            hwndTrack: self.list,
            dwHoverTime: HOVER_DEFAULT,
        };
        unsafe { TrackMouseEvent(&mut tracking) };
    }

    pub fn clear(&self) {
        self.hover(None);
    }

    pub fn row(&self) -> Option<usize> {
        self.row.get()
    }

    pub fn set_font(&self, font: HFONT) {
        let tip = self.window.get();
        if !tip.is_null() {
            send(tip, WM_SETFONT, font as usize, 0);
        }
    }

    pub fn destroy(&self) {
        let tip = self.window.replace(null_mut());
        if !tip.is_null() {
            unsafe { DestroyWindow(tip) };
        }
    }

    fn tool(&self) -> TTTOOLINFOW {
        TTTOOLINFOW {
            cbSize: size_of::<TTTOOLINFOW>() as u32,
            uFlags: TTF_IDISHWND | TTF_TRACK | TTF_ABSOLUTE,
            hwnd: self.list,
            uId: self.list as usize,
            ..unsafe { zeroed() }
        }
    }

    fn window(&self) -> HWND {
        let existing = self.window.get();
        if !existing.is_null() {
            return existing;
        }
        unsafe {
            let tip = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                TOOLTIPS_CLASSW,
                null(),
                WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX | TTS_NOANIMATE | TTS_NOFADE,
                0,
                0,
                0,
                0,
                self.list,
                null_mut(),
                null_mut(),
                null(),
            );
            if tip.is_null() {
                return tip;
            }
            let mut tool = self.tool();
            tool.lpszText = EMPTY_TEXT.as_ptr().cast_mut();
            if SetWindowSubclass(tip, Some(transparent_tip), 1, 0) == 0
                || send(tip, TTM_ADDTOOLW, 0, &tool as *const _ as LPARAM) == 0
            {
                DestroyWindow(tip);
                return null_mut();
            }
            // Tooltip colors only take effect when this window uses classic rendering.
            SetWindowTheme(tip, EMPTY_TEXT.as_ptr(), EMPTY_TEXT.as_ptr());
            self.window.set(tip);
            tip
        }
    }

    pub fn show(
        &self,
        name: &[u16],
        anchor: RECT,
        mut pointer: POINT,
        font: HFONT,
        colors: &Palette,
        dpi: u32,
    ) {
        unsafe {
            let mut top = POINT {
                x: anchor.left,
                y: anchor.top,
            };
            let mut bottom = POINT {
                x: anchor.left,
                y: anchor.bottom,
            };
            if ClientToScreen(self.list, &mut top) == 0
                || ClientToScreen(self.list, &mut bottom) == 0
                || ClientToScreen(self.list, &mut pointer) == 0
            {
                return;
            }
            let mut info: MONITORINFO = zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            let monitor = MonitorFromPoint(pointer, MONITOR_DEFAULTTONULL);
            let work = (!monitor.is_null() && GetMonitorInfoW(monitor, &mut info) != 0)
                .then_some(info.rcWork);
            let tip = self.window();
            if tip.is_null() {
                return;
            }
            let pad = scale(8, dpi);
            let margin = RECT {
                left: pad,
                right: pad,
                top: scale(4, dpi),
                bottom: scale(4, dpi),
            };
            let max_width = work.map_or(scale(560, dpi), |work| {
                scale(560, dpi).min((work.right - work.left - 2 * pad - 4).max(1))
            });
            send(tip, WM_SETFONT, font as usize, 0);
            send(tip, TTM_SETTIPBKCOLOR, colors.field as usize, 0);
            send(tip, TTM_SETTIPTEXTCOLOR, colors.text as usize, 0);
            send(tip, TTM_SETMARGIN, 0, &margin as *const _ as LPARAM);
            send(tip, TTM_SETMAXTIPWIDTH, 0, max_width as LPARAM);

            let mut text = self.text.borrow_mut();
            *text = wrapped_text(self.list, font, name, max_width);
            let mut tool = self.tool();
            tool.lpszText = text.as_mut_ptr();
            let tool = &tool as *const _ as LPARAM;
            send(tip, TTM_UPDATETIPTEXTW, 0, tool);
            let size = send(tip, TTM_GETBUBBLESIZE, 0, tool) as u32;
            let width = i32::from(size as u16);
            let height = i32::from((size >> 16) as u16);
            let gap = scale(3, dpi);
            let mut x = bottom.x;
            let mut y = bottom.y + gap;
            if let Some(work) = work {
                x = x.clamp(work.left, (work.right - width).max(work.left));
                if y + height > work.bottom {
                    y = (top.y - gap - height).max(work.top);
                }
            }
            let position = ((y as u16 as u32) << 16) | x as u16 as u32;
            send(tip, TTM_TRACKPOSITION, 0, position as LPARAM);
            send(tip, TTM_TRACKACTIVATE, 1, tool);
        }
    }
}

fn wrapped_text(list: HWND, font: HFONT, mut name: &[u16], width: i32) -> Vec<u16> {
    let mut text = Vec::with_capacity(name.len() + 1);
    unsafe {
        let dc = GetDC(list);
        if !dc.is_null() {
            let old = SelectObject(dc, font);
            while name.len() > 1 {
                let mut fit = 0;
                let mut size: SIZE = zeroed();
                if GetTextExtentExPointW(
                    dc,
                    name.as_ptr(),
                    name.len() as i32,
                    width,
                    &mut fit,
                    null_mut(),
                    &mut size,
                ) == 0
                    || fit as usize >= name.len()
                {
                    break;
                }
                let mut split = (fit as usize).max(1);
                if (0xd800..=0xdbff).contains(&name[split - 1])
                    && (0xdc00..=0xdfff).contains(&name[split])
                {
                    split = if split > 1 { split - 1 } else { 2 };
                }
                if split == name.len() {
                    break;
                }
                text.extend_from_slice(&name[..split]);
                text.extend([13, 10]);
                name = &name[split..];
            }
            SelectObject(dc, old);
            ReleaseDC(list, dc);
        }
    }
    text.extend_from_slice(name);
    text.push(0);
    text
}

unsafe extern "system" fn transparent_tip(
    window: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if message == WM_NCHITTEST {
        return HTTRANSPARENT as LRESULT;
    }
    unsafe { DefSubclassProc(window, message, wp, lp) }
}
