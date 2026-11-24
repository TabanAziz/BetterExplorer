use better_explorer::model::wide;
use std::{
    fs::OpenOptions,
    io::Read,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::SIZE,
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
        UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_THUMBNAILONLY},
    },
};
use windows_sys::Win32::{
    Foundation::RECT,
    Graphics::Gdi::*,
    Storage::FileSystem::{
        FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_READ,
    },
};

pub enum Content {
    Text(String),
    Bitmap(Bitmap),
}

pub struct Bitmap(usize);

impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.0 as HBITMAP);
        }
    }
}

impl Bitmap {
    pub fn draw(&self, dc: HDC, bounds: RECT) {
        unsafe {
            let mut bitmap: BITMAP = std::mem::zeroed();
            let handle = self.0 as HBITMAP;
            if GetObjectW(
                handle,
                std::mem::size_of::<BITMAP>() as i32,
                (&mut bitmap as *mut BITMAP).cast(),
            ) == 0
            {
                return;
            }
            if bitmap.bmWidth < 1 || bitmap.bmHeight < 1 {
                return;
            }
            let width = bounds.right - bounds.left;
            let height = bounds.bottom - bounds.top;
            let scale = (width as f64 / bitmap.bmWidth as f64)
                .min(height as f64 / bitmap.bmHeight as f64)
                .min(1.0);
            let w = (bitmap.bmWidth as f64 * scale).round() as i32;
            let h = (bitmap.bmHeight as f64 * scale).round() as i32;
            let source = CreateCompatibleDC(dc);
            if source.is_null() {
                return;
            }
            let old = SelectObject(source, handle);
            let old_mode = SetStretchBltMode(dc, HALFTONE);
            let x = bounds.left + (width - w) / 2;
            let y = bounds.top + (height - h) / 2;
            StretchBlt(
                dc,
                x,
                y,
                w,
                h,
                source,
                0,
                0,
                bitmap.bmWidth,
                bitmap.bmHeight,
                SRCCOPY,
            );
            SetStretchBltMode(dc, old_mode);
            SelectObject(source, old);
            DeleteDC(source);
        }
    }
}

type Pending = (Mutex<Option<(u64, PathBuf)>>, Condvar);

pub struct Worker(Arc<Pending>);

impl Worker {
    pub fn start(publish: impl Fn(u64, Option<Content>) + Send + 'static) -> Option<Self> {
        let requests: Arc<Pending> = Arc::new((Mutex::new(None), Condvar::new()));
        let reader = Arc::clone(&requests);
        std::thread::Builder::new()
            .name("file preview".into())
            .spawn(move || {
                let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() };
                loop {
                    let (latest, ready) = &*reader;
                    let mut request = ready
                        .wait_while(latest.lock().unwrap(), |work| work.is_none())
                        .unwrap();
                    let (id, path) = request.take().unwrap();
                    drop(request);
                    if id == 0 {
                        break;
                    }
                    publish(id, load(&path, initialized));
                }
                if initialized {
                    unsafe {
                        CoUninitialize();
                    }
                }
            })
            .ok()?;
        Some(Self(requests))
    }

    pub fn request(&self, id: u64, path: PathBuf) {
        *self.0 .0.lock().unwrap() = Some((id, path));
        self.0 .1.notify_one();
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.request(0, PathBuf::new());
    }
}

fn load(path: &Path, shell_ready: bool) -> Option<Content> {
    if better_explorer::work::network_path(path) {
        return None;
    }
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let text = matches!(
        ext.as_str(),
        "txt" | "md" | "log" | "csv" | "json" | "ini" | "xml" | "rs" | "toml"
    );
    let image = matches!(
        ext.as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tif" | "tiff"
    );
    if !text && (!image || !shell_ready) {
        return None;
    }

    let metadata = std::fs::symlink_metadata(path).ok()?;
    let skip = FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_REPARSE_POINT | 0x0004_0000 | 0x0040_0000;
    if !metadata.is_file() || metadata.file_attributes() & skip != 0 {
        return None;
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .ok()?;
    if file.metadata().ok()?.file_attributes() & skip != 0 {
        return None;
    }

    if text {
        let mut data = Vec::new();
        file.take(12_288).read_to_end(&mut data).ok()?;
        let value = String::from_utf8_lossy(&data);
        return Some(Content::Text(
            value
                .trim_start_matches('\u{feff}')
                .chars()
                .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\r' | '\t'))
                .take(600)
                .collect(),
        ));
    }
    let path = wide(path);
    unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(PCWSTR(path.as_ptr()), None).ok()?;
        let bitmap = factory
            .GetImage(SIZE { cx: 256, cy: 256 }, SIIGBF_THUMBNAILONLY)
            .ok()?;
        Some(Content::Bitmap(Bitmap(bitmap.0 as usize)))
    }
}
