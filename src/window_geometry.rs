use better_explorer::model::settings_path;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{fs, io, mem};
use windows_sys::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MonitorFromRect, MonitorFromWindow, MONITORINFO,
        MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL,
    },
    UI::{
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::{
            GetCursorPos, GetSystemMetrics, GetWindowRect, IsIconic, IsZoomed, SM_CXSCREEN,
            SM_CYSCREEN,
        },
    },
};

const SIZE_FILE: &str = "window-size.json";
const POSITION_FILE: &str = "window-position.json";

#[derive(Clone, Copy, Deserialize, Serialize)]
pub struct WindowPosition {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl WindowPosition {
    pub fn load() -> Option<Self> {
        load::<Self>(POSITION_FILE)
            .filter(|position| position.right > position.left && position.bottom > position.top)
    }

    pub fn save(self) -> io::Result<()> {
        save(POSITION_FILE, self)
    }

    pub fn normal(window: HWND) -> Option<Self> {
        normal_bounds(window).map(|bounds| Self {
            left: bounds.left,
            top: bounds.top,
            right: bounds.right,
            bottom: bounds.bottom,
        })
    }

    fn bounds(self) -> RECT {
        RECT {
            left: self.left,
            top: self.top,
            right: self.right,
            bottom: self.bottom,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSize {
    width: i32,
    height: i32,
}

impl WindowSize {
    const DEFAULT: Self = Self {
        width: 968,
        height: 608,
    };
    const MINIMUM: Self = Self {
        width: 552,
        height: 450,
    };

    pub fn load() -> Self {
        load::<Self>(SIZE_FILE)
            .filter(|size| size.width > 0 && size.height > 0)
            .unwrap_or(Self::DEFAULT)
    }

    pub fn save(self) -> io::Result<()> {
        save(SIZE_FILE, self)
    }

    pub fn normal(window: HWND) -> Option<Self> {
        let dpi = unsafe { GetDpiForWindow(window) };
        if dpi == 0 {
            return None;
        }
        normal_bounds(window).map(|bounds| Self {
            width: scaled(bounds.right - bounds.left, 96, dpi),
            height: scaled(bounds.bottom - bounds.top, 96, dpi),
        })
    }

    pub fn minimum(dpi: u32, work: RECT) -> POINT {
        Self::MINIMUM.fit(dpi, work)
    }

    fn fit(self, dpi: u32, work: RECT) -> POINT {
        POINT {
            x: scaled(self.width.max(Self::MINIMUM.width), dpi, 96).min(work.right - work.left),
            y: scaled(self.height.max(Self::MINIMUM.height), dpi, 96).min(work.bottom - work.top),
        }
    }

    pub fn placed(self, dpi: u32, work: RECT, position: Option<WindowPosition>) -> RECT {
        let size = self.fit(dpi, work);
        let center = center(work);
        let left = position.map_or(center.x - size.x / 2, |position| position.left);
        let top = position.map_or(center.y - size.y / 2, |position| position.top);
        let left = left.clamp(work.left, work.right - size.x);
        let top = top.clamp(work.top, work.bottom - size.y);
        RECT {
            left,
            top,
            right: left + size.x,
            bottom: top + size.y,
        }
    }
}

fn load<T: DeserializeOwned>(name: &str) -> Option<T> {
    let path = settings_path()?.with_file_name(name);
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save(name: &str, value: impl Serialize) -> io::Result<()> {
    let Some(path) = settings_path().map(|path| path.with_file_name(name)) else {
        return Ok(());
    };
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(temp.as_file_mut(), &value)?;
    temp.persist(path)?;
    Ok(())
}

fn normal_bounds(window: HWND) -> Option<RECT> {
    let mut bounds = unsafe { mem::zeroed::<RECT>() };
    (unsafe {
        IsZoomed(window) == 0 && IsIconic(window) == 0 && GetWindowRect(window, &mut bounds) != 0
    })
    .then_some(bounds)
}

fn scaled(value: i32, numerator: u32, denominator: u32) -> i32 {
    let denominator = i64::from(denominator);
    ((i64::from(value) * i64::from(numerator) + denominator / 2) / denominator)
        .min(i64::from(i32::MAX)) as i32
}

pub fn center(bounds: RECT) -> POINT {
    POINT {
        x: bounds.left + (bounds.right - bounds.left) / 2,
        y: bounds.top + (bounds.bottom - bounds.top) / 2,
    }
}

pub fn startup_work_area(position: Option<WindowPosition>) -> (RECT, Option<WindowPosition>) {
    if let Some(position) = position {
        let monitor = unsafe { MonitorFromRect(&position.bounds(), MONITOR_DEFAULTTONULL) };
        if let Some(info) = monitor_info(monitor) {
            return (info.rcWork, Some(position));
        }
    }
    let mut cursor = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut cursor) };
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    let work = monitor_info(monitor).map_or_else(
        || RECT {
            left: 0,
            top: 0,
            right: unsafe { GetSystemMetrics(SM_CXSCREEN) },
            bottom: unsafe { GetSystemMetrics(SM_CYSCREEN) },
        },
        |info| info.rcWork,
    );
    (work, None)
}

pub fn window_monitor_info(window: HWND) -> Option<MONITORINFO> {
    monitor_info(unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) })
}

fn monitor_info(monitor: windows_sys::Win32::Graphics::Gdi::HMONITOR) -> Option<MONITORINFO> {
    let mut info = MONITORINFO {
        cbSize: mem::size_of::<MONITORINFO>() as u32,
        ..unsafe { mem::zeroed() }
    };
    (unsafe { GetMonitorInfoW(monitor, &mut info) } != 0).then_some(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_window_dimensions_scale_and_fit_the_destination_monitor() {
        let work_area = RECT {
            left: -1280,
            top: 30,
            right: 0,
            bottom: 790,
        };
        let saved = WindowSize {
            width: 811,
            height: 579,
        };
        let bounds = saved.placed(144, work_area, None);
        assert_eq!(
            (bounds.left, bounds.top, bounds.right, bounds.bottom),
            (-1248, 30, -31, 790)
        );

        let reopened_at_lower_dpi = saved.placed(96, work_area, None);
        assert_eq!(
            reopened_at_lower_dpi.right - reopened_at_lower_dpi.left,
            811
        );
        assert_eq!(
            reopened_at_lower_dpi.bottom - reopened_at_lower_dpi.top,
            579
        );
    }

    #[test]
    fn saved_window_position_fits_the_current_work_area() {
        let work = RECT {
            left: -1280,
            top: 30,
            right: 0,
            bottom: 790,
        };
        let position = WindowPosition {
            left: -420,
            top: -8,
            right: 110,
            bottom: 561,
        };
        let bounds = WindowSize::DEFAULT.placed(96, work, Some(position));
        assert_eq!(
            (bounds.left, bounds.top, bounds.right, bounds.bottom),
            (-968, 30, 0, 638)
        );
    }
}
