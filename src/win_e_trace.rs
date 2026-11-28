// Compiled only for a dedicated diagnostic binary; never enabled in release packaging.
use std::{
    fs::{self, File},
    io::{self, Write},
    path::PathBuf,
    sync::mpsc::{self, SyncSender},
    thread,
};
use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

pub(crate) fn stamp() -> i64 {
    let mut ticks = 0;
    unsafe { QueryPerformanceCounter(&mut ticks) };
    ticks
}

fn output(kind: &str) -> io::Result<File> {
    let directory =
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is unavailable")
        })?)
        .join("BetterExplorerWinETrace");
    fs::create_dir_all(&directory)?;
    let mut file =
        File::create(directory.join(format!("{kind}-{}-{}.tsv", std::process::id(), stamp())))?;
    let mut frequency = 0;
    unsafe { QueryPerformanceFrequency(&mut frequency) };
    writeln!(file, "# qpc_frequency_hz={frequency}")?;
    writeln!(file, "qpc\trequest\tphase\tvalue")?;
    Ok(file)
}

pub(crate) struct Event {
    ticks: i64,
    request: i64,
    phase: &'static str,
    value: i64,
}

impl Event {
    pub(crate) fn new(request: i64, phase: &'static str, value: i64) -> Self {
        Self {
            ticks: stamp(),
            request,
            phase,
            value,
        }
    }

    fn write(&self, file: &mut File) -> io::Result<()> {
        writeln!(
            file,
            "{}\t{}\t{}\t{}",
            self.ticks, self.request, self.phase, self.value
        )
    }
}

pub(crate) type Sender = SyncSender<Event>;

pub(crate) fn listener() -> io::Result<Sender> {
    let (tx, rx) = mpsc::sync_channel::<Event>(256);
    thread::Builder::new().spawn(move || {
        let Ok(mut file) = output("listener") else {
            return;
        };
        while let Ok(event) = rx.recv() {
            if event.write(&mut file).is_err() {
                break;
            }
        }
    })?;
    Ok(tx)
}

pub(crate) fn mark(sender: Option<&Sender>, request: i64, phase: &'static str, value: i64) {
    if let Some(sender) = sender {
        // Never block the low-level hook on I/O or a full diagnostic queue.
        let _ = sender.try_send(Event::new(request, phase, value));
    }
}

pub(crate) fn app_shown(start: Event) {
    let shown = Event::new(0, "window_shown", i64::from(std::process::id()));
    let _ = thread::Builder::new().spawn(move || {
        if let Ok(mut file) = output("window") {
            let _ = start.write(&mut file).and_then(|()| shown.write(&mut file));
        }
    });
}
