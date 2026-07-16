//! `--soak-gpu` report arithmetic + the CLI deliverable print. Split out of the
//! probe's `mod.rs` to keep each file under the ~500-line ceiling: this half is
//! the pure memory/recovery summary math (`summarize`/`median`/slope) and the
//! [`Report`] the live [`super::Controller`] hands back at the end of a run. All
//! print sites live HERE, so the println-audit's `soak_gpu/*` accounting has one
//! home.

use super::{Counts, SkipKind};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Summary {
    pub min_bytes: u64,
    pub median_bytes: u64,
    pub peak_bytes: u64,
    pub end_median_bytes: u64,
    pub slope_bytes_per_min: f64,
}

pub(super) fn summarize(samples: &[(f64, u64)]) -> Option<Summary> {
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
    pub(super) elapsed: Duration,
    pub(super) backend: Option<String>,
    pub(super) counts: Counts,
    pub(super) rss: Option<Summary>,
    pub(super) metal: Option<Summary>,
    pub(super) recovery_ms: [Option<f64>; 3],
    pub(super) required_cycles_met: bool,
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
        // Per-cause breakdown of the `skipped` total: this line is what turns
        // "820 skipped, 0 presented" into "820 occluded" — the difference
        // between a mystery and a self-diagnosis.
        let mut breakdown = String::from("skipped_by_kind");
        for kind in SkipKind::ALL {
            breakdown.push_str(&format!(
                " {}={}",
                kind.label(),
                self.counts.skipped_by_kind[kind.index()]
            ));
        }
        println!("{breakdown}");
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
    fn report_refuses_missing_real_surface_proof() {
        let complete = Counts {
            acquires: 1,
            presents: 1,
            resizes: super::super::RESIZE_TARGET,
            themes: super::super::THEME_TARGET,
            overlays: super::super::OVERLAY_TARGET,
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
