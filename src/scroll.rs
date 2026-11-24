use std::{
    cell::RefCell,
    mem::size_of,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    UI::{
        Controls::*,
        HiDpi::GetDpiForWindow,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const SUBCLASS: usize = 1;
pub const TIMER: usize = 0x4655;
const FRAME_MS: u32 = 15;
const WHEEL_PAGESCROLL: u32 = u32::MAX;
const DURATION: Duration = Duration::from_millis(120);

#[derive(Clone, Copy)]
struct Track {
    origin: i32,
    min: i32,
    max: i32,
    page: i32,
    pixels_per_unit: i32,
    units_per_line: i32,
    horizontal: bool,
    icons: bool,
}

fn track(window: HWND, view: u32) -> Option<Track> {
    let icons = matches!(view, LV_VIEW_ICON | LV_VIEW_SMALLICON);
    let horizontal = view == LV_VIEW_LIST;
    let mut info = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_ALL,
        ..unsafe { std::mem::zeroed() }
    };
    let bar = if horizontal { SB_HORZ } else { SB_VERT };
    if unsafe { GetScrollInfo(window, bar, &mut info) } == 0 {
        return None;
    }
    let page = i32::try_from(info.nPage).unwrap_or(i32::MAX).max(1);
    let max = info.nMax.saturating_sub(page - 1);
    if max <= info.nMin {
        return None;
    }
    let pixels_per_unit = if icons {
        1
    } else if horizontal {
        (unsafe { SendMessageW(window, LVM_GETCOLUMNWIDTH, 0, 0) } as i32).max(1)
    } else {
        let mut bounds: RECT = unsafe { std::mem::zeroed() };
        unsafe {
            SendMessageW(
                window,
                LVM_GETITEMRECT,
                0,
                (&mut bounds as *mut RECT) as LPARAM,
            );
        }
        (bounds.bottom - bounds.top).max(1)
    };
    let dpi = unsafe { GetDpiForWindow(window) }.max(96);
    Some(Track {
        origin: info.nPos.clamp(info.nMin, max),
        min: info.nMin,
        max,
        page,
        pixels_per_unit,
        units_per_line: if icons { (42 * dpi / 96) as i32 } else { 1 },
        horizontal,
        icons,
    })
}

#[derive(Clone, Copy)]
struct Motion {
    start: i32,
    end: i32,
    initiated: Instant,
}

impl Motion {
    fn position(self, elapsed: Duration) -> i32 {
        let fraction = (elapsed.as_secs_f64() / DURATION.as_secs_f64()).clamp(0.0, 1.0);
        let progress = 1.0 - (1.0 - fraction).powi(3);
        self.start + ((self.end - self.start) as f64 * progress).round() as i32
    }
}

struct Wheel {
    view: u32,
    motion: Option<Motion>,
    fraction: f64,
    scrolling: bool,
}

impl Default for Wheel {
    fn default() -> Self {
        Self {
            view: LV_VIEW_DETAILS,
            motion: None,
            fraction: 0.0,
            scrolling: false,
        }
    }
}

impl Wheel {
    fn retarget(&mut self, track: Track, wheel_delta: i16, lines: u32) {
        let distance = if lines == WHEEL_PAGESCROLL {
            track.page
        } else {
            track
                .units_per_line
                .saturating_mul(i32::try_from(lines).unwrap_or(i32::MAX))
        };
        let movement = self.fraction - wheel_delta as f64 * distance as f64 / 120.0;
        let increment = movement.trunc() as i32;
        self.fraction = movement.fract();
        if increment == 0 {
            return;
        }
        let base = self
            .motion
            .filter(|motion| (motion.end - track.origin).signum() == increment.signum())
            .map_or(track.origin, |motion| motion.end);
        let end = base.saturating_add(increment).clamp(track.min, track.max);
        if end == track.min || end == track.max {
            self.fraction = 0.0;
        }
        self.motion = Some(Motion {
            start: track.origin,
            end,
            initiated: Instant::now(),
        });
    }

    fn stop(&mut self) {
        self.motion = None;
        self.fraction = 0.0;
    }
}

fn advance(window: HWND, wheel: &RefCell<Wheel>, immediate: bool) {
    let (view, motion) = {
        let state = wheel.borrow();
        let Some(motion) = state.motion else { return };
        (state.view, motion)
    };
    let Some(track) = track(window, view) else {
        wheel.borrow_mut().stop();
        unsafe {
            KillTimer(window, TIMER);
        }
        return;
    };
    let elapsed = motion.initiated.elapsed()
        + Duration::from_millis(if immediate { FRAME_MS.into() } else { 0 });
    let destination = motion.end.clamp(track.min, track.max);
    let position = motion.position(elapsed).clamp(track.min, track.max);
    let delta = (position - track.origin).saturating_mul(track.pixels_per_unit);
    let finished = elapsed >= DURATION || position == destination;
    if delta == 0 && !finished {
        return;
    }
    wheel.borrow_mut().scrolling = true;
    unsafe {
        if delta != 0 {
            let (dx, dy) = if track.horizontal {
                (delta as usize, 0)
            } else {
                (0, delta as isize)
            };
            SendMessageW(window, LVM_SCROLL, dx, dy);
        }
        if finished && destination == track.min && track.icons {
            let mut origin: POINT = std::mem::zeroed();
            if SendMessageW(
                window,
                LVM_GETORIGIN,
                0,
                (&mut origin as *mut POINT) as LPARAM,
            ) != 0
                && origin.y != 0
            {
                SendMessageW(window, WM_VSCROLL, SB_TOP as usize, 0);
            }
        }
    }
    let mut state = wheel.borrow_mut();
    state.scrolling = false;
    if finished {
        state.motion = None;
        unsafe {
            KillTimer(window, TIMER);
        }
    }
}

/// Installs smooth wheel scrolling on a list view on the calling GUI thread.
///
/// # Safety
/// The caller must supply a live list-view HWND owned by the calling GUI thread.
pub unsafe fn install(window: HWND) -> bool {
    let pointer = Box::into_raw(Box::new(RefCell::new(Wheel::default())));
    if unsafe { SetWindowSubclass(window, Some(procedure), SUBCLASS, pointer as usize) } == 0 {
        unsafe {
            drop(Box::from_raw(pointer));
        }
        return false;
    }
    true
}

unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    let state = data as *mut RefCell<Wheel>;
    let wheel = unsafe { &*state };
    match message {
        WM_MOUSEWHEEL if wp & 0x000c == 0 => {
            let mut lines = 3u32;
            unsafe {
                SystemParametersInfoW(
                    SPI_GETWHEELSCROLLLINES,
                    0,
                    (&mut lines as *mut u32).cast(),
                    0,
                );
            }
            let view = wheel.borrow().view;
            if let Some(track) = track(window, view) {
                wheel.borrow_mut().retarget(track, (wp >> 16) as i16, lines);
                if wheel.borrow().motion.is_some() {
                    unsafe {
                        SetTimer(window, TIMER, FRAME_MS, None);
                    }
                    advance(window, wheel, true);
                }
            }
            return 0;
        }
        WM_TIMER if wp == TIMER => {
            advance(window, wheel, false);
            return 0;
        }
        LVM_SETVIEW => {
            let mut state = wheel.borrow_mut();
            state.stop();
            state.view = wp as u32;
            unsafe {
                KillTimer(window, TIMER);
            }
        }
        WM_VSCROLL | WM_HSCROLL | WM_KEYDOWN | LVM_SETICONSPACING => {
            let mut state = wheel.borrow_mut();
            if !state.scrolling {
                state.stop();
                unsafe {
                    KillTimer(window, TIMER);
                }
            }
        }
        LVM_SETITEMCOUNT if wp == 0 => {
            wheel.borrow_mut().stop();
            unsafe {
                KillTimer(window, TIMER);
                SendMessageW(window, WM_VSCROLL, SB_TOP as usize, 0);
                SendMessageW(window, WM_HSCROLL, SB_LEFT as usize, 0);
            }
        }
        WM_NCDESTROY => unsafe {
            KillTimer(window, TIMER);
            RemoveWindowSubclass(window, Some(procedure), SUBCLASS);
            drop(Box::from_raw(state));
        },
        _ => {}
    }
    unsafe { DefSubclassProc(window, message, wp, lp) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon_track(origin: i32) -> Track {
        Track {
            origin,
            min: 0,
            max: 900,
            page: 400,
            pixels_per_unit: 1,
            units_per_line: 42,
            horizontal: false,
            icons: true,
        }
    }

    #[test]
    fn upward_motion_eases_and_lands_on_origin() {
        let mut wheel = Wheel::default();
        wheel.retarget(icon_track(117), 120, 3);
        let motion = wheel.motion.unwrap();
        let first = motion.position(Duration::from_millis(15));
        let middle = motion.position(Duration::from_millis(45));
        assert!((0..117).contains(&first));
        assert!(middle < first);
        assert_eq!(motion.position(DURATION), 0);
    }

    #[test]
    fn bursts_accumulate_and_reversals_drop_the_old_direction() {
        let mut wheel = Wheel::default();
        for _ in 0..2 {
            wheel.retarget(icon_track(450), -120, 3);
        }
        assert_eq!(wheel.motion.unwrap().end, 702);
        wheel.retarget(icon_track(450), 120, 3);
        assert_eq!(wheel.motion.unwrap().end, 324);
    }
}
