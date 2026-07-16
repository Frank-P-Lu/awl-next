//! Bounded real-surface GPU robustness probe (`--soak-gpu`).
//!
//! This module owns the deterministic schedule, process-memory sampling, and
//! report arithmetic. The live App applies each declarative [`Stimulus`] through
//! its ordinary paths and reports actual surface outcomes back here; the probe
//! is deliberately not a second renderer or a headless surface substitute.

#![cfg(not(target_arch = "wasm32"))]

use std::time::{Duration, Instant};

pub(crate) const DEFAULT_DURATION: Duration = Duration::from_secs(15 * 60);
pub(crate) const RESIZE_TARGET: u32 = 300;
pub(crate) const THEME_TARGET: u32 = 300;
pub(crate) const OVERLAY_TARGET: u32 = 150;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SoakConfig {
    pub duration: Duration,
}

impl Default for SoakConfig {
    fn default() -> Self {
        Self {
            duration: DEFAULT_DURATION,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FaultKind {
    OutOfMemory,
    SurfaceLost,
    DeviceLost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stimulus {
    SetLavaTheme,
    Resize { width: u32, height: u32 },
    ThemeNext,
    Overlay { open: bool },
    Inject(FaultKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameOutcome {
    Presented,
    Skipped,
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    elapsed_secs: f64,
    rss_bytes: u64,
    metal_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Counts {
    frames: u64,
    acquires: u64,
    presents: u64,
    skipped: u64,
    resizes: u32,
    themes: u32,
    overlays: u32,
    faults: u32,
}

/// App-owned state for one probe. The event loop asks for at most one stimulus
/// at a time and remains the sole owner of renderer mutation and recovery.
pub(crate) struct Controller {
    config: SoakConfig,
    started: Instant,
    last_sample: Option<Instant>,
    lava_seeded: bool,
    overlay_open: bool,
    injected: [bool; 3],
    scheduled_resizes: u32,
    scheduled_themes: u32,
    scheduled_overlay_toggles: u32,
    counts: Counts,
    samples: Vec<Sample>,
    recovery_started: [Option<Instant>; 3],
    recovery_ms: [Option<f64>; 3],
    backend: Option<String>,
}

impl Controller {
    pub(crate) fn new(config: SoakConfig) -> Self {
        Self::new_at(config, Instant::now())
    }

    fn new_at(config: SoakConfig, started: Instant) -> Self {
        Self {
            config,
            started,
            last_sample: None,
            lava_seeded: false,
            overlay_open: false,
            injected: [false; 3],
            scheduled_resizes: 0,
            scheduled_themes: 0,
            scheduled_overlay_toggles: 0,
            counts: Counts::default(),
            samples: Vec::new(),
            recovery_started: [None; 3],
            recovery_ms: [None; 3],
            backend: None,
        }
    }

    pub(crate) fn finished(&self, now: Instant) -> bool {
        now.duration_since(self.started) >= self.config.duration
    }

    /// Spread the full roster uniformly over the post-warmup interval. A short
    /// developer run accelerates the roster instead of silently reducing it.
    pub(crate) fn next_stimulus(&mut self, now: Instant) -> Option<Stimulus> {
        if !self.lava_seeded {
            self.lava_seeded = true;
            return Some(Stimulus::SetLavaTheme);
        }
        let elapsed = now.duration_since(self.started);
        let warmup = Duration::from_secs(5).min(self.config.duration / 10);
        if elapsed < warmup || self.finished(now) {
            return None;
        }
        let active = elapsed.saturating_sub(warmup);
        let span = self
            .config
            .duration
            .saturating_sub(warmup)
            .max(Duration::from_millis(1));
        let fraction = (active.as_secs_f64() / span.as_secs_f64()).clamp(0.0, 1.0);

        for (index, threshold, kind) in [
            (0, 0.20, FaultKind::OutOfMemory),
            (1, 0.50, FaultKind::SurfaceLost),
            (2, 0.80, FaultKind::DeviceLost),
        ] {
            if !self.injected[index] && fraction >= threshold {
                self.injected[index] = true;
                self.counts.faults += 1;
                self.recovery_started[index] = Some(now);
                return Some(Stimulus::Inject(kind));
            }
        }
        let want_resize = due(RESIZE_TARGET, fraction);
        if self.scheduled_resizes < want_resize {
            let i = self.scheduled_resizes;
            self.scheduled_resizes += 1;
            let sizes = [(960, 640), (1280, 800), (1100, 720), (1440, 900)];
            let (width, height) = sizes[i as usize % sizes.len()];
            return Some(Stimulus::Resize { width, height });
        }
        let want_theme = due(THEME_TARGET, fraction);
        if self.scheduled_themes < want_theme {
            self.scheduled_themes += 1;
            return Some(Stimulus::ThemeNext);
        }
        let want_toggles = due(OVERLAY_TARGET * 2, fraction);
        if self.scheduled_overlay_toggles < want_toggles {
            self.scheduled_overlay_toggles += 1;
            self.overlay_open = !self.overlay_open;
            return Some(Stimulus::Overlay {
                open: self.overlay_open,
            });
        }
        None
    }

    pub(crate) fn observe_backend(&mut self, backend: impl Into<String>) {
        self.backend = Some(backend.into());
    }

    pub(crate) fn observe_frame(&mut self, outcome: FrameOutcome, acquired: bool) {
        self.counts.frames += 1;
        self.counts.acquires += u64::from(acquired);
        match outcome {
            FrameOutcome::Presented => self.counts.presents += 1,
            FrameOutcome::Skipped => self.counts.skipped += 1,
        }
    }

    /// Record completed work, separately from scheduling it. In particular a
    /// requested resize counts only when winit delivers a changed size and the
    /// live surface has actually been reconfigured.
    pub(crate) fn observe_resize(&mut self) {
        self.counts.resizes += 1;
    }

    pub(crate) fn observe_theme_switch(&mut self) {
        self.counts.themes += 1;
    }

    pub(crate) fn observe_overlay_cycle(&mut self) {
        self.counts.overlays += 1;
    }

    pub(crate) fn observe_recovered(&mut self, kind: FaultKind, now: Instant) {
        let i = fault_index(kind);
        if let Some(start) = self.recovery_started[i].take() {
            self.recovery_ms[i] = Some(now.duration_since(start).as_secs_f64() * 1000.0);
        }
    }

    /// Sample once per second. `metal_bytes` comes from the live raw Metal
    /// device query. Missing samples make the final macOS report fail clearly.
    pub(crate) fn sample_if_due(&mut self, now: Instant, metal_bytes: Option<u64>) {
        if self
            .last_sample
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(1))
        {
            return;
        }
        self.last_sample = Some(now);
        if let Some(rss_bytes) = process_rss_bytes() {
            self.samples.push(Sample {
                elapsed_secs: now.duration_since(self.started).as_secs_f64(),
                rss_bytes,
                metal_bytes,
            });
        }
    }

    pub(crate) fn report(&self, now: Instant) -> Report {
        let rss: Vec<_> = self
            .samples
            .iter()
            .map(|s| (s.elapsed_secs, s.rss_bytes))
            .collect();
        let metal: Vec<_> = self
            .samples
            .iter()
            .filter_map(|s| s.metal_bytes.map(|b| (s.elapsed_secs, b)))
            .collect();
        Report {
            elapsed: now.duration_since(self.started),
            backend: self.backend.clone(),
            counts: self.counts,
            rss: summarize(&rss),
            metal: summarize(&metal),
            recovery_ms: self.recovery_ms,
            required_cycles_met: self.counts.resizes >= RESIZE_TARGET
                && self.counts.themes >= THEME_TARGET
                && self.counts.overlays >= OVERLAY_TARGET
                && self.counts.faults == 3,
        }
    }
}

fn due(target: u32, fraction: f64) -> u32 {
    ((target as f64 * fraction).ceil() as u32).min(target)
}

fn fault_index(kind: FaultKind) -> usize {
    match kind {
        FaultKind::OutOfMemory => 0,
        FaultKind::SurfaceLost => 1,
        FaultKind::DeviceLost => 2,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Summary {
    pub min_bytes: u64,
    pub median_bytes: u64,
    pub peak_bytes: u64,
    pub end_median_bytes: u64,
    pub slope_bytes_per_min: f64,
}

fn summarize(samples: &[(f64, u64)]) -> Option<Summary> {
    if samples.is_empty() {
        return None;
    }
    let values: Vec<_> = samples.iter().map(|(_, v)| *v).collect();
    let tail_len = values.len().div_ceil(10).max(1);
    Some(Summary {
        min_bytes: *values.iter().min().unwrap(),
        median_bytes: median(&values),
        peak_bytes: *values.iter().max().unwrap(),
        end_median_bytes: median(&values[values.len() - tail_len..]),
        slope_bytes_per_min: least_squares_slope(samples) * 60.0,
    })
}

fn median(values: &[u64]) -> u64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        sorted[mid - 1] / 2 + sorted[mid] / 2 + (sorted[mid - 1] % 2 + sorted[mid] % 2) / 2
    } else {
        sorted[mid]
    }
}

fn least_squares_slope(samples: &[(f64, u64)]) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let n = samples.len() as f64;
    let mean_x = samples.iter().map(|(x, _)| x).sum::<f64>() / n;
    let mean_y = samples.iter().map(|(_, y)| *y as f64).sum::<f64>() / n;
    let numerator = samples
        .iter()
        .map(|(x, y)| (x - mean_x) * (*y as f64 - mean_y))
        .sum::<f64>();
    let denominator = samples
        .iter()
        .map(|(x, _)| (x - mean_x).powi(2))
        .sum::<f64>();
    if denominator == 0.0 {
        0.0
    } else {
        numerator / denominator
    }
}

pub(crate) struct Report {
    elapsed: Duration,
    backend: Option<String>,
    counts: Counts,
    rss: Option<Summary>,
    metal: Option<Summary>,
    recovery_ms: [Option<f64>; 3],
    required_cycles_met: bool,
}

impl Report {
    pub(crate) fn passed(&self) -> bool {
        self.backend.is_some()
            && self.counts.acquires > 0
            && self.counts.presents > 0
            && self.required_cycles_met
            && self.recovery_ms.iter().all(Option::is_some)
            && self.rss.is_some()
            && (!cfg!(target_os = "macos") || self.metal.is_some())
    }

    pub(crate) fn print(&self) {
        println!(
            "soak-gpu result: {}",
            if self.passed() { "PASS" } else { "FAIL" }
        );
        println!(
            "elapsed_s={:.3} backend={} frames={} acquires={} presents={} skipped={} resizes={} themes={} overlay_cycles={} faults={}",
            self.elapsed.as_secs_f64(),
            self.backend.as_deref().unwrap_or("absent"),
            self.counts.frames,
            self.counts.acquires,
            self.counts.presents,
            self.counts.skipped,
            self.counts.resizes,
            self.counts.themes,
            self.counts.overlays,
            self.counts.faults
        );
        print_summary("rss", self.rss);
        print_summary("metal", self.metal);
        println!(
            "recovery_ms oom={} surface_lost={} device_lost={}",
            fmt_optional(self.recovery_ms[0]),
            fmt_optional(self.recovery_ms[1]),
            fmt_optional(self.recovery_ms[2])
        );
        if self.counts.acquires == 0 || self.counts.presents == 0 {
            println!("defect: real native surface acquire+present path was not proven");
        }
        if cfg!(target_os = "macos") && self.metal.is_none() {
            println!("defect: Metal currentAllocatedSize samples were absent");
        }
    }
}

fn print_summary(label: &str, summary: Option<Summary>) {
    match summary {
        Some(s) => println!(
            "{label}_bytes min={} median={} peak={} end_median={} slope_per_min={:.3}",
            s.min_bytes, s.median_bytes, s.peak_bytes, s.end_median_bytes, s.slope_bytes_per_min
        ),
        None => println!("{label}_bytes absent"),
    }
}

fn fmt_optional(value: Option<f64>) -> String {
    value.map_or_else(|| "absent".to_string(), |v| format!("{v:.3}"))
}

#[cfg(target_os = "macos")]
fn process_rss_bytes() -> Option<u64> {
    use std::ffi::{c_int, c_void};
    #[repr(C)]
    #[derive(Default)]
    struct ProcTaskInfo {
        virtual_size: u64,
        resident_size: u64,
        total_user: u64,
        total_system: u64,
        threads_user: u64,
        threads_system: u64,
        policy: i32,
        faults: i32,
        pageins: i32,
        cow_faults: i32,
        messages_sent: i32,
        messages_received: i32,
        syscalls_mach: i32,
        syscalls_unix: i32,
        csw: i32,
        threadnum: i32,
        numrunning: i32,
        priority: i32,
    }
    unsafe extern "C" {
        fn getpid() -> c_int;
        fn proc_pidinfo(
            pid: c_int,
            flavor: c_int,
            arg: u64,
            buffer: *mut c_void,
            size: c_int,
        ) -> c_int;
    }
    let mut info = ProcTaskInfo::default();
    // SAFETY: `info` is a writable, correctly-sized PROC_PIDTASKINFO buffer.
    let read = unsafe {
        proc_pidinfo(
            getpid(),
            4,
            0,
            (&mut info as *mut ProcTaskInfo).cast(),
            std::mem::size_of::<ProcTaskInfo>() as c_int,
        )
    };
    (read as usize == std::mem::size_of::<ProcTaskInfo>()).then_some(info.resident_size)
}

#[cfg(target_os = "linux")]
fn process_rss_bytes() -> Option<u64> {
    use std::ffi::c_int;
    unsafe extern "C" {
        fn getpagesize() -> c_int;
    }
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: getpagesize has no arguments and returns a process-global constant.
    let page_size = unsafe { getpagesize() };
    (page_size > 0).then_some(pages.saturating_mul(page_size as u64))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn process_rss_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_math_pins_median_tail_and_slope() {
        let s = summarize(&[(0.0, 100), (1.0, 110), (2.0, 120), (3.0, 130), (4.0, 140)]).unwrap();
        assert_eq!(
            (
                s.min_bytes,
                s.median_bytes,
                s.peak_bytes,
                s.end_median_bytes
            ),
            (100, 120, 140, 140)
        );
        assert!((s.slope_bytes_per_min - 600.0).abs() < 1e-9);
        assert_eq!(median(&[u64::MAX - 1, u64::MAX]), u64::MAX - 1);
    }

    #[test]
    fn accelerated_schedule_reaches_every_cycle_and_fault() {
        let start = Instant::now();
        let mut c = Controller::new_at(
            SoakConfig {
                duration: Duration::from_secs(2),
            },
            start,
        );
        let end = start + Duration::from_secs(2) - Duration::from_nanos(1);
        while c.next_stimulus(end).is_some() {}
        assert_eq!(
            (
                c.scheduled_resizes,
                c.scheduled_themes,
                c.scheduled_overlay_toggles / 2,
                c.counts.faults
            ),
            (RESIZE_TARGET, THEME_TARGET, OVERLAY_TARGET, 3)
        );
    }

    #[test]
    fn report_refuses_missing_real_surface_proof() {
        let complete = Counts {
            acquires: 1,
            presents: 1,
            resizes: RESIZE_TARGET,
            themes: THEME_TARGET,
            overlays: OVERLAY_TARGET,
            faults: 3,
            ..Counts::default()
        };
        let memory = Summary {
            min_bytes: 1,
            median_bytes: 1,
            peak_bytes: 1,
            end_median_bytes: 1,
            slope_bytes_per_min: 0.0,
        };
        let mut report = Report {
            elapsed: Duration::from_secs(1),
            backend: Some("Metal".to_string()),
            counts: complete,
            rss: Some(memory),
            metal: Some(memory),
            recovery_ms: [Some(1.0); 3],
            required_cycles_met: true,
        };
        assert!(report.passed());
        report.counts.presents = 0;
        assert!(!report.passed());
        report.counts.presents = 1;
        report.backend = None;
        assert!(!report.passed());
    }
}
