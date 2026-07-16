//! Bounded real-surface GPU robustness probe (`--soak-gpu`).
//!
//! This module owns the deterministic schedule, process-memory sampling, and
//! report arithmetic. The live App applies each declarative [`Stimulus`] through
//! its ordinary paths and reports actual surface outcomes back here; the probe
//! is deliberately not a second renderer or a headless surface substitute.

#![cfg(not(target_arch = "wasm32"))]

use std::time::{Duration, Instant};

mod report;
pub(crate) use report::Report;

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
    Skipped(SkipKind),
}

/// The reason a frame did not present, mirrored from the live GPU skip/fault
/// taxonomy so the probe can count each cause SEPARATELY. Collapsing these into
/// a single `skipped` total is exactly what hid the zero-drawable
/// investigation: 30s of pure `Occluded` skips read identically to a timeout
/// storm. Per-kind counters make that self-diagnosing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SkipKind {
    Timeout,
    Occluded,
    SurfaceReconfigured,
    SurfaceRecreated,
    PrepareFailed,
    /// A classified fault (OOM / device-lost / surface-lost / validation) whose
    /// frame is dropped while App-owned recovery runs.
    Fault,
}

impl SkipKind {
    /// Every variant, in the fixed order the report prints — the array's length
    /// IS the counter table's width (a new variant fails to compile the
    /// `[u64; SkipKind::ALL.len()]` counter until it is added here).
    pub(crate) const ALL: [SkipKind; 6] = [
        Self::Timeout,
        Self::Occluded,
        Self::SurfaceReconfigured,
        Self::SurfaceRecreated,
        Self::PrepareFailed,
        Self::Fault,
    ];

    pub(crate) fn index(self) -> usize {
        match self {
            Self::Timeout => 0,
            Self::Occluded => 1,
            Self::SurfaceReconfigured => 2,
            Self::SurfaceRecreated => 3,
            Self::PrepareFailed => 4,
            Self::Fault => 5,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Occluded => "occluded",
            Self::SurfaceReconfigured => "reconfigured",
            Self::SurfaceRecreated => "recreated",
            Self::PrepareFailed => "prepare_failed",
            Self::Fault => "fault",
        }
    }
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
    /// Per-cause skip tally, indexed by [`SkipKind::index`]. The `skipped` total
    /// above is their sum; keeping both lets the report print the breakdown
    /// without losing the one-number headline.
    skipped_by_kind: [u64; SkipKind::ALL.len()],
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
        // Interleave the three periodic stimuli by NORMALIZED LAG rather than
        // draining one kind before the next. The old drain order let resize
        // demand monopolize the schedule: the live loop emits at most one
        // resize per tick (the batch breaks on it), so as `fraction` crept up
        // resize was perpetually "due" and re-selected first every tick, and
        // themes/overlays stayed at zero (themes=0/overlays=0 at 6s). Picking
        // the kind furthest behind its own quota (deficit ÷ its target) keeps
        // all three progressing together; ties resolve resize→theme→overlay.
        let periodic = [
            (self.scheduled_resizes, due(RESIZE_TARGET, fraction), RESIZE_TARGET),
            (self.scheduled_themes, due(THEME_TARGET, fraction), THEME_TARGET),
            (
                self.scheduled_overlay_toggles,
                due(OVERLAY_TARGET * 2, fraction),
                OVERLAY_TARGET * 2,
            ),
        ];
        let mut choice: Option<usize> = None;
        let mut best_lag = 0.0_f64;
        for (i, &(scheduled, want, target)) in periodic.iter().enumerate() {
            if scheduled < want {
                let lag = f64::from(want - scheduled) / f64::from(target);
                if choice.is_none() || lag > best_lag {
                    choice = Some(i);
                    best_lag = lag;
                }
            }
        }
        match choice {
            Some(0) => {
                let i = self.scheduled_resizes;
                self.scheduled_resizes += 1;
                let sizes = [(960, 640), (1280, 800), (1100, 720), (1440, 900)];
                let (width, height) = sizes[i as usize % sizes.len()];
                Some(Stimulus::Resize { width, height })
            }
            Some(1) => {
                self.scheduled_themes += 1;
                Some(Stimulus::ThemeNext)
            }
            Some(2) => {
                self.scheduled_overlay_toggles += 1;
                self.overlay_open = !self.overlay_open;
                Some(Stimulus::Overlay {
                    open: self.overlay_open,
                })
            }
            _ => None,
        }
    }

    pub(crate) fn observe_backend(&mut self, backend: impl Into<String>) {
        self.backend = Some(backend.into());
    }

    pub(crate) fn observe_frame(&mut self, outcome: FrameOutcome, acquired: bool) {
        self.counts.frames += 1;
        self.counts.acquires += u64::from(acquired);
        match outcome {
            FrameOutcome::Presented => self.counts.presents += 1,
            FrameOutcome::Skipped(kind) => {
                self.counts.skipped += 1;
                self.counts.skipped_by_kind[kind.index()] += 1;
            }
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
            rss: report::summarize(&rss),
            metal: report::summarize(&metal),
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

    /// Walking real time in small ticks — at most one resize/inject emitted per
    /// tick, exactly like the live `drive_gpu_soak` batch — every periodic kind
    /// must be making progress well before the end. The OLD drain order left
    /// themes and overlays at ZERO here (resize was perpetually "due" and won
    /// every tick); the normalized-lag interleave keeps all three moving.
    #[test]
    fn interleaved_schedule_never_starves_themes_or_overlays() {
        let start = Instant::now();
        let dur = Duration::from_secs(600);
        let mut c = Controller::new_at(SoakConfig { duration: dur }, start);
        assert_eq!(c.next_stimulus(start), Some(Stimulus::SetLavaTheme));
        let steps = 2000u32;
        for step in 1..=steps {
            let now = start + dur.mul_f64(f64::from(step) / f64::from(steps))
                - Duration::from_millis(1);
            for _ in 0..32 {
                let Some(s) = c.next_stimulus(now) else { break };
                if matches!(s, Stimulus::Resize { .. } | Stimulus::Inject(_)) {
                    break;
                }
            }
            if step == steps * 2 / 5 {
                assert!(c.scheduled_resizes > 0, "resizes starved");
                assert!(c.scheduled_themes > 0, "themes starved");
                assert!(c.scheduled_overlay_toggles > 0, "overlays starved");
            }
        }
    }

    /// A skip is tallied both in the headline total and in its own per-kind
    /// bucket, so 30s of pure occlusion never masquerades as a timeout storm.
    #[test]
    fn observe_frame_tracks_skip_kind_separately() {
        let mut c = Controller::new_at(SoakConfig::default(), Instant::now());
        c.observe_frame(FrameOutcome::Skipped(SkipKind::Occluded), false);
        c.observe_frame(FrameOutcome::Skipped(SkipKind::Occluded), false);
        c.observe_frame(FrameOutcome::Skipped(SkipKind::Timeout), false);
        c.observe_frame(FrameOutcome::Presented, true);
        assert_eq!(c.counts.skipped, 3);
        assert_eq!(c.counts.presents, 1);
        assert_eq!(c.counts.skipped_by_kind[SkipKind::Occluded.index()], 2);
        assert_eq!(c.counts.skipped_by_kind[SkipKind::Timeout.index()], 1);
        assert_eq!(c.counts.skipped_by_kind[SkipKind::Fault.index()], 0);
    }
}
