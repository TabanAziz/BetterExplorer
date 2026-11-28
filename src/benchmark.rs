use serde::Serialize;
use std::{io, mem::zeroed, path::PathBuf, time::Instant};
use windows_sys::Win32::{
    Foundation::{FILETIME, HWND},
    Graphics::{
        Dwm::DwmFlush,
        Gdi::{RedrawWindow, RDW_ALLCHILDREN, RDW_INVALIDATE, RDW_UPDATENOW},
    },
    System::{
        SystemInformation::GetSystemTimePreciseAsFileTime,
        Threading::{GetCurrentProcess, GetProcessTimes},
    },
};

pub fn present(window: HWND) {
    unsafe {
        RedrawWindow(
            window,
            std::ptr::null(),
            std::ptr::null_mut(),
            RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW,
        );
        DwmFlush();
    }
}

fn startup_ms() -> f64 {
    let to_ticks =
        |time: FILETIME| ((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64;
    unsafe {
        let mut created = zeroed();
        let mut exit = zeroed();
        let mut kernel = zeroed();
        let mut user = zeroed();
        let mut now = zeroed();
        GetSystemTimePreciseAsFileTime(&mut now);
        if GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exit,
            &mut kernel,
            &mut user,
        ) == 0
        {
            return 0.0;
        }
        to_ticks(now).saturating_sub(to_ticks(created)) as f64 / 10_000.0
    }
}

#[derive(Serialize)]
struct Sample {
    first_ms: f64,
    complete_ms: f64,
    rows: usize,
    max_dispatch_ms: f64,
    first_row_stages_ms: Option<[f64; 5]>,
}

#[derive(Serialize)]
struct Measurements {
    startup_ms: f64,
    navigation: Vec<Sample>,
}

struct Running {
    started: Instant,
    first_ms: Option<f64>,
    max_dispatch_ms: f64,
    first_row_stages_ms: Option<[f64; 5]>,
}

pub struct Bench {
    path: PathBuf,
    target: Option<PathBuf>,
    count: usize,
    warmup_finished: bool,
    running: Option<Running>,
    measurements: Measurements,
}

impl Bench {
    pub fn configured() -> Option<Self> {
        let path = PathBuf::from(std::env::var_os("BETTEREXPLORER_BENCH_REPORT")?);
        let target = std::env::var_os("BETTEREXPLORER_BENCH_TARGET").map(PathBuf::from);
        let count = if target.is_some() {
            std::env::var("BETTEREXPLORER_BENCH_RUNS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0)
        } else {
            0
        };
        Some(Self {
            path,
            target,
            count,
            warmup_finished: false,
            running: None,
            measurements: Measurements {
                startup_ms: 0.0,
                navigation: Vec::new(),
            },
        })
    }

    pub fn started(&mut self, window: HWND) {
        present(window);
        self.measurements.startup_ms = startup_ms();
    }

    pub fn done(&self) -> bool {
        self.measurements.navigation.len() >= self.count
    }

    pub fn begin_navigation(&mut self) -> Option<PathBuf> {
        let target = self.target.clone()?;
        if self.running.is_some() || self.done() {
            return None;
        }
        self.running = Some(Running {
            started: Instant::now(),
            first_ms: None,
            max_dispatch_ms: 0.0,
            first_row_stages_ms: None,
        });
        Some(target)
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    pub fn needs_first_paint(&self) -> bool {
        self.running
            .as_ref()
            .is_some_and(|running| running.first_ms.is_none())
    }

    pub fn note_scan(&mut self, stages: [Instant; 4]) {
        if let Some(running) = &mut self.running {
            let elapsed =
                |now: Instant| now.duration_since(running.started).as_secs_f64() * 1_000.0;
            running.first_row_stages_ms = Some([
                elapsed(stages[0]),
                elapsed(stages[1]),
                elapsed(stages[2]),
                elapsed(stages[3]),
                elapsed(Instant::now()),
            ]);
        }
    }

    pub fn dispatch_elapsed(&mut self, began: Instant) {
        if let Some(running) = &mut self.running {
            running.max_dispatch_ms = running
                .max_dispatch_ms
                .max(began.elapsed().as_secs_f64() * 1_000.0);
        }
    }

    pub fn painted(&mut self, rows: usize, complete: bool) {
        let Some(running) = &mut self.running else {
            return;
        };
        let elapsed = running.started.elapsed().as_secs_f64() * 1_000.0;
        if rows > 0 && running.first_ms.is_none() {
            running.first_ms = Some(elapsed);
        }
        if !complete {
            return;
        }
        let sample = Sample {
            first_ms: running.first_ms.unwrap_or(elapsed),
            complete_ms: elapsed,
            rows,
            max_dispatch_ms: running.max_dispatch_ms,
            first_row_stages_ms: running.first_row_stages_ms,
        };
        self.running = None;
        if self.warmup_finished {
            self.measurements.navigation.push(sample);
        } else {
            self.warmup_finished = true;
        }
    }

    pub fn save(&self) -> io::Result<()> {
        std::fs::write(&self.path, serde_json::to_vec_pretty(&self.measurements)?)
    }
}
