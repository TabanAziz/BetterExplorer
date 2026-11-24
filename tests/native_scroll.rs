#![cfg(windows)]

use better_explorer::scroll::{self, TIMER};
use std::{
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    thread,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{HWND, POINT, RECT},
    UI::{Controls::*, WindowsAndMessaging::*},
};

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[test]
fn icon_scroll_animates_up_and_restores_the_entire_first_row() {
    unsafe {
        let controls = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES,
        };
        assert_ne!(InitCommonControlsEx(&controls), 0);
        let parent = CreateWindowExW(
            0,
            wide("Static").as_ptr(),
            wide("").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            850,
            480,
            null_mut(),
            null_mut(),
            null_mut(),
            null(),
        );
        assert!(!parent.is_null());
        let list = CreateWindowExW(
            0,
            wide("SysListView32").as_ptr(),
            wide("").as_ptr(),
            WS_CHILD | WS_VISIBLE | LVS_ICON | LVS_OWNERDATA | LVS_SHAREIMAGELISTS,
            0,
            0,
            850,
            438,
            parent,
            null_mut(),
            null_mut(),
            null(),
        );
        assert!(!list.is_null());
        let images = ImageList_Create(52, 52, ILC_COLOR32, 0, 1);
        SendMessageW(list, LVM_SETIMAGELIST, LVSIL_NORMAL as usize, images);
        assert!(scroll::install(list));
        assert_eq!(SendMessageW(list, LVM_SETVIEW, LV_VIEW_ICON as usize, 0), 1);
        SendMessageW(list, LVM_SETICONSPACING, 0, (90 << 16) | 86);
        SendMessageW(list, LVM_SETITEMCOUNT, 10_000, 0);

        let first = item_bounds(list);
        assert!(first.top >= 0 && first.bottom <= 438);
        SendMessageW(list, LVM_SCROLL, 0, 117);
        assert_eq!(origin(list), 117);
        let mut lines = 3u32;
        SystemParametersInfoW(
            SPI_GETWHEELSCROLLLINES,
            0,
            (&mut lines as *mut u32).cast(),
            0,
        );
        if lines != 0 {
            SendMessageW(list, WM_MOUSEWHEEL, 480usize << 16, 0);
            let next = origin(list);
            assert!(
                (1..117).contains(&next),
                "first visible motion was {next}px"
            );
            thread::sleep(Duration::from_millis(140));
            SendMessageW(list, WM_TIMER, TIMER, 0);
            assert_eq!(origin(list), 0);
            let restored = item_bounds(list);
            assert_eq!((restored.top, restored.bottom), (first.top, first.bottom));
        }

        SendMessageW(list, LVM_SCROLL, 0, 117);
        SendMessageW(list, LVM_SETITEMCOUNT, 0, 0);
        SendMessageW(list, LVM_SETITEMCOUNT, 100, 0);
        assert_eq!(origin(list), 0);
        DestroyWindow(parent);
        ImageList_Destroy(images);
    }
}

unsafe fn origin(list: HWND) -> i32 {
    let mut point: POINT = unsafe { zeroed() };
    assert_ne!(
        unsafe { SendMessageW(list, LVM_GETORIGIN, 0, (&mut point as *mut POINT) as isize) },
        0
    );
    point.y
}

unsafe fn item_bounds(list: HWND) -> RECT {
    let mut bounds: RECT = unsafe { zeroed() };
    assert_ne!(
        unsafe {
            SendMessageW(
                list,
                LVM_GETITEMRECT,
                0,
                (&mut bounds as *mut RECT) as isize,
            )
        },
        0
    );
    bounds
}
