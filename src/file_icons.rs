use std::{
    cell::OnceCell,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{sync_channel, SyncSender},
        Arc,
    },
    thread,
};
use windows_core::{IUnknown, Interface};
use windows_sys::{
    core::GUID,
    Win32::{
        Graphics::Gdi::{
            CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, BITMAPINFO, BITMAPINFOHEADER,
            BI_RGB, DIB_RGB_COLORS,
        },
        Storage::FileSystem::FILE_ATTRIBUTE_NORMAL,
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
        UI::{
            Controls::{ImageList_GetIcon, ImageList_GetIconSize, HIMAGELIST, ILD_TRANSPARENT},
            Shell::{
                SHGetFileInfoW, SHGetImageList, SHGetStockIconInfo, SHFILEINFOW, SHGFI_FLAGS,
                SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SHELLICONSIZE, SHGFI_SMALLICON,
                SHGFI_SYSICONINDEX, SHGFI_USEFILEATTRIBUTES, SHGSI_SYSICONINDEX, SHIL_EXTRALARGE,
                SHIL_JUMBO, SHIL_LARGE, SHIL_SMALL, SHSTOCKICONINFO, SIID_DOCNOASSOC,
            },
            WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO},
        },
    },
};

const REQUEST_CAPACITY: usize = 64;
const IID_IMAGE_LIST: GUID = GUID::from_u128(0x46eb5926_582e_4017_9fdf_e8998daa0950);

pub struct Request {
    pub slot: usize,
    pub generation: u64,
    pub key: Vec<u16>,
    pub side: i32,
}

pub struct Result {
    pub request: Request,
    pub icon: Option<Icon>,
}

// The copied icon can be used and destroyed on a different thread.
pub struct Icon(usize);

impl Icon {
    fn from_handle(handle: HICON) -> Option<Self> {
        (!handle.is_null()).then_some(Self(handle as usize))
    }

    pub fn handle(&self) -> HICON {
        self.0 as HICON
    }

    fn has_large_artwork(&self, side: i32) -> bool {
        let Some(bitmaps) = IconBitmaps::get(self.handle()) else {
            return false;
        };
        if bitmaps.0.hbmColor.is_null() {
            return false;
        }

        let mut data: BITMAPINFO = unsafe { zeroed() };
        data.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        data.bmiHeader.biWidth = side;
        data.bmiHeader.biHeight = -side;
        data.bmiHeader.biPlanes = 1;
        data.bmiHeader.biBitCount = 32;
        data.bmiHeader.biCompression = BI_RGB;
        let mut pixels = vec![0u32; side as usize * side as usize];
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            if dc.is_null() {
                return false;
            }
            let rows = GetDIBits(
                dc,
                bitmaps.0.hbmColor,
                0,
                side as u32,
                pixels.as_mut_ptr().cast(),
                &mut data,
                DIB_RGB_COLORS,
            );
            DeleteDC(dc);
            rows == side && covers_half(&pixels, side as usize)
        }
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        unsafe { DestroyIcon(self.handle()) };
    }
}

struct IconBitmaps(ICONINFO);

impl IconBitmaps {
    fn get(icon: HICON) -> Option<Self> {
        let mut bitmaps = Self(unsafe { zeroed() });
        (unsafe { GetIconInfo(icon, &mut bitmaps.0) } != 0).then_some(bitmaps)
    }
}

impl Drop for IconBitmaps {
    fn drop(&mut self) {
        unsafe {
            if !self.0.hbmColor.is_null() {
                DeleteObject(self.0.hbmColor);
            }
            if !self.0.hbmMask.is_null() {
                DeleteObject(self.0.hbmMask);
            }
        }
    }
}

fn covers_half(pixels: &[u32], side: usize) -> bool {
    let (mut left, mut top, mut right) = (side, side, 0);
    for (y, row) in pixels.chunks_exact(side).enumerate() {
        for (x, pixel) in row.iter().enumerate() {
            if pixel >> 24 >= 16 {
                left = left.min(x);
                right = right.max(x + 1);
                top = top.min(y);
                if right - left >= side / 2 || y + 1 - top >= side / 2 {
                    return true;
                }
            }
        }
    }
    false
}

pub struct Worker {
    sender: SyncSender<Request>,
    stopped: Arc<AtomicBool>,
}

impl Worker {
    pub fn start(publish: impl Fn(Result) + Send + 'static) -> Option<Self> {
        let (sender, receiver) = sync_channel::<Request>(REQUEST_CAPACITY);
        let stopped = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::clone(&stopped);
        thread::Builder::new()
            .name("file icons".into())
            .spawn(move || {
                let apartment = Apartment::start();
                let mut shell = None;
                while let Ok(request) = receiver.recv() {
                    if cancelled.load(Ordering::Relaxed) {
                        break;
                    }
                    let icon = apartment
                        .as_ref()
                        .and_then(|_| shell.get_or_insert_with(ShellIcons::new).lookup(&request));
                    if !cancelled.load(Ordering::Relaxed) {
                        publish(Result { request, icon });
                    }
                }
            })
            .ok()?;
        Some(Self { sender, stopped })
    }

    pub fn request(&self, request: Request) -> bool {
        self.sender.try_send(request).is_ok()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
}

struct Apartment;

impl Apartment {
    fn start() -> Option<Self> {
        if unsafe { CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32) } < 0 {
            return None;
        }
        Some(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct ImageList {
    interface: IUnknown,
    side: i32,
}

impl ImageList {
    fn get(kind: u32) -> Option<Self> {
        let mut raw = null_mut();
        unsafe {
            if SHGetImageList(kind as i32, &IID_IMAGE_LIST, &mut raw) < 0 || raw.is_null() {
                return None;
            }
            // SHGetImageList returns an owning IImageList reference and supports the HIMAGELIST cast.
            let interface = IUnknown::from_raw(raw);
            let (mut width, mut height) = (0, 0);
            if ImageList_GetIconSize(raw as HIMAGELIST, &mut width, &mut height) == 0
                || width <= 0
                || height <= 0
            {
                return None;
            }
            Some(Self {
                interface,
                side: width.min(height),
            })
        }
    }

    fn icon(&self, index: i32) -> Option<Icon> {
        Icon::from_handle(unsafe {
            ImageList_GetIcon(
                self.interface.as_raw() as HIMAGELIST,
                index,
                ILD_TRANSPARENT,
            )
        })
    }
}

struct ShellIcons {
    lists: Vec<ImageList>,
    jumbo: OnceCell<Option<ImageList>>,
    unassociated: Option<i32>,
    extensionless: Option<i32>,
}

impl ShellIcons {
    fn new() -> Self {
        let extensionless =
            file_info(&synthetic_name(&[]), SHGFI_SYSICONINDEX).map(|info| info.iIcon);
        let mut stock: SHSTOCKICONINFO = unsafe { zeroed() };
        stock.cbSize = size_of::<SHSTOCKICONINFO>() as u32;
        let unassociated =
            (unsafe { SHGetStockIconInfo(SIID_DOCNOASSOC, SHGSI_SYSICONINDEX, &mut stock) } >= 0)
                .then_some(stock.iSysImageIndex);
        let lists = [SHIL_SMALL, SHIL_LARGE, SHIL_EXTRALARGE]
            .into_iter()
            .filter_map(ImageList::get)
            .collect();
        Self {
            lists,
            jumbo: OnceCell::new(),
            unassociated,
            extensionless,
        }
    }

    fn lookup(&self, request: &Request) -> Option<Icon> {
        let name = synthetic_name(&request.key);
        let index = file_info(&name, SHGFI_SYSICONINDEX)?.iIcon;
        if request.key.is_empty() || [self.unassociated, self.extensionless].contains(&Some(index))
        {
            return None;
        }

        let regular = self
            .lists
            .iter()
            .min_by_key(|list| (list.side < request.side, list.side.abs_diff(request.side)));
        if let Some(regular) = regular.filter(|list| request.side > list.side + list.side / 4) {
            let jumbo = self.jumbo.get_or_init(|| ImageList::get(SHIL_JUMBO));
            if let Some(jumbo) = jumbo.as_ref().filter(|list| list.side > regular.side) {
                // Some Windows associations put a small icon in one corner of the Jumbo canvas.
                if let Some(icon) = jumbo
                    .icon(index)
                    .filter(|icon| icon.has_large_artwork(jumbo.side))
                {
                    return Some(icon);
                }
            }
        }
        if let Some(icon) = regular.and_then(|list| list.icon(index)) {
            return Some(icon);
        }

        let size_flag = if request.side <= 24 {
            SHGFI_SMALLICON
        } else {
            SHGFI_LARGEICON
        };
        file_info(&name, SHGFI_ICON | SHGFI_SHELLICONSIZE | size_flag)
            .and_then(|info| Icon::from_handle(info.hIcon))
    }
}

fn synthetic_name(key: &[u16]) -> Vec<u16> {
    let mut name: Vec<_> = "BetterExplorerFileType".encode_utf16().collect();
    if !key.is_empty() {
        name.push(b'.' as u16);
        name.extend_from_slice(key);
    }
    name.push(0);
    name
}

fn file_info(name: &[u16], flags: SHGFI_FLAGS) -> Option<SHFILEINFOW> {
    let mut info: SHFILEINFOW = unsafe { zeroed() };
    let result = unsafe {
        SHGetFileInfoW(
            name.as_ptr(),
            FILE_ATTRIBUTE_NORMAL,
            &mut info,
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_USEFILEATTRIBUTES | flags,
        )
    };
    (result != 0).then_some(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};
    use windows_sys::Win32::Graphics::Gdi::{GetObjectW, BITMAP};

    #[test]
    fn jumbo_coverage_distinguishes_full_artwork_from_a_padded_small_icon() {
        let side = 256;
        let mut pixels = vec![0u32; side * side];
        for row in pixels.chunks_exact_mut(side).take(48) {
            row[..48].fill(0xff808080);
        }
        assert!(!covers_half(&pixels, side));

        pixels.fill(0);
        for row in pixels.chunks_exact_mut(side).skip(28).take(200) {
            row[80..176].fill(0xff808080);
        }
        assert!(covers_half(&pixels, side));
    }

    #[test]
    fn worker_resolves_synthetic_file_types_and_preserves_request_identity() {
        let _apartment = Apartment::start().unwrap();
        let shell = ShellIcons::new();
        let small = ImageList::get(SHIL_SMALL).unwrap().side;
        let medium = ImageList::get(SHIL_LARGE).unwrap().side;
        let large = ImageList::get(SHIL_EXTRALARGE).unwrap().side;
        assert!(large > small);
        let (publish, results) = mpsc::channel();
        let worker = Worker::start(move |result| publish.send(result).unwrap()).unwrap();
        let types = [
            "",
            "bexunregistered8492acd5",
            "txt",
            "zip",
            "pdf",
            "md",
            "svg",
            "docx",
            "mwb",
            "mcdx",
        ];

        for (slot, extension) in types.into_iter().enumerate() {
            let key: Vec<_> = extension.encode_utf16().collect();
            let index = file_info(&synthetic_name(&key), SHGFI_SYSICONINDEX).map(|info| info.iIcon);
            let has_icon =
                index.is_some() && index != shell.extensionless && index != shell.unassociated;
            for (generation, side, expected) in [
                (42, small, small),
                (43, small + 1, medium),
                (44, large, large),
                (45, large + 4, large),
            ] {
                assert!(worker.request(Request {
                    slot,
                    generation,
                    key: key.clone(),
                    side
                }));
                let result = results.recv_timeout(Duration::from_secs(10)).unwrap();
                assert_eq!(result.request.slot, slot);
                assert_eq!(result.request.generation, generation);
                assert_eq!(result.request.key, key);
                assert_eq!(result.request.side, side);
                assert_eq!(result.icon.is_some(), slot >= 2 && has_icon, "{extension}");
                if let Some(icon) = result.icon {
                    let (width, height) = dimensions(icon.handle());
                    assert_eq!(
                        width.min(height),
                        expected,
                        "{extension}: {width} x {height}"
                    );
                }
            }
        }
    }

    fn dimensions(icon: HICON) -> (i32, i32) {
        let bitmaps = IconBitmaps::get(icon).unwrap();
        unsafe {
            let info = &bitmaps.0;
            let monochrome = info.hbmColor.is_null();
            let bitmap = if monochrome {
                info.hbmMask
            } else {
                info.hbmColor
            };
            let mut data: BITMAP = zeroed();
            let read = GetObjectW(
                bitmap,
                size_of::<BITMAP>() as i32,
                (&mut data as *mut BITMAP).cast(),
            );
            assert_ne!(read, 0);
            (data.bmWidth, data.bmHeight / if monochrome { 2 } else { 1 })
        }
    }
}
