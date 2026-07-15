//! UNIFIED BENCH SUITE (hidden `--bench-suite` flag) — the measured matrix the
//! five scattered `--bench-*` flags never were: corpus TIERS (S/M/L/XPARA/
//! XMD/CODE, generated deterministically from a fixed seed — see [`corpus`])
//! crossed with interaction SCENARIOS (cold open, typing burst, scroll +
//! jump-to-end, search, palette open, zoom burst, theme burst, wrap/resize —
//! see [`scenarios`]), every cell reporting min/median/p90 wall times AND the
//! witness counters that prove the work happened (the bench-witness law:
//! a cell that can silently measure nothing is a defect, enforced with
//! `ensure!`s, not comments).
//!
//! Output is the printed table plus a machine-readable `bench.json` written
//! beside the invocation (the capture-sidecar idiom; shape `awl-bench/N`, see
//! [`report`]). With `--bench-baseline <path>` the run then diffs itself
//! against the checked-in `benches/baseline.json` — machine-keyed, warn at
//! ~20% per cell, nonzero exit on regression — which is what
//! `scripts/bench.sh` runs on merge-train days.
//!
//! The five legacy flags (`--bench-typing`/`-perf`/`-frame`/`-theme-burst`/
//! `-zoom-burst`) are left intact and untouched: they answer DEEP per-stage
//! questions (a 24-stage frame split, cold/warm atlas laps) the matrix
//! deliberately doesn't re-implement, while the suite owns the BREADTH +
//! baseline story. A child of `render` (like [`super::perfbench`] /
//! [`super::framebench`]) so scenarios reach the private pipeline fields +
//! `pub(super)` seams directly. RELEASE-ONLY numbers: a debug-profile run
//! prints a loud warning, stamps `"profile": "debug"` into bench.json, and a
//! release baseline refuses to diff against it.

mod corpus;
mod cx;
mod report;
mod scenarios;

use std::path::PathBuf;

use anyhow::{ensure, Result};
use glyphon::Cache;

use crate::clock::Instant;
use crate::config::Config;

use corpus::Tier;
use scenarios::Scenario;

/// The report canvas: the same 2910x1720 @2x physical geometry the frame
/// profiler uses (a real laptop window), page mode ON, debug panel OFF.
const WIDTH: u32 = 2910;
const HEIGHT: u32 = 1720;
const DPI: f32 = 2.0;
/// The dt a steady 60fps live loop feeds `advance`.
const DT: f32 = 1.0 / 60.0;

/// Run the full suite; `baseline` (from `--bench-baseline`) additionally
/// diffs the fresh run against a checked-in baseline and exits nonzero on a
/// regression (see [`report::diff_against`] for the exact exit policy).
pub fn run(baseline: Option<PathBuf>) -> Result<()> {
    pollster::block_on(run_async(baseline))
}

async fn run_async(baseline: Option<PathBuf>) -> Result<()> {
    let wall0 = Instant::now();
    let (device, queue) = crate::capture::gpu::headless_device().await?;
    let cache = Cache::new(&device);

    // The pinned suite posture: page ON (the product's default), debug OFF,
    // the default world. Scenarios that move a global (theme) restore it.
    crate::debug::set_debug_on(false);
    crate::page::set_page_on(true);
    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let config = Config::load(PathBuf::new()); // pure defaults, no file

    println!("bench suite — {WIDTH}x{HEIGHT} @{DPI}x · page ON · debug OFF · schema {}", report::SCHEMA);
    println!("(headless: submit+poll SERIALIZES GPU cost; witnesses are hard failures, not notes)");
    if cfg!(debug_assertions) {
        println!("WARNING: DEV PROFILE — timings are 10-20x off; never baseline or diff this run");
    }

    let spell = crate::spell::SpellChecker::new(crate::spell::DictVariant::EnUs)
        .map_err(|e| anyhow::anyhow!("spell checker failed to load: {e}"))?;

    let mut cells: Vec<report::CellRec> = Vec::new();
    let mut skips: Vec<report::SkipRec> = Vec::new();
    println!();
    println!(
        "{:>6} | {:>10} | {:>7} | {:>10} | {:>10} | {:>10} | witness",
        "tier", "scenario", "samples", "min ms", "median ms", "p90 ms"
    );
    println!(
        "{:->6}-+-{:->10}-+-{:->7}-+-{:->10}-+-{:->10}-+-{:->10}-+--------",
        "", "", "", "", "", ""
    );
    for tier in Tier::ALL {
        // DETERMINISM (the spec's own assert): the same seed must yield a
        // byte-identical corpus — generate twice and compare outright.
        let text = corpus::text(tier);
        ensure!(
            text == corpus::text(tier),
            "corpus tier {} must regenerate byte-identically",
            tier.name()
        );
        // Reset the per-tier posture: default world, the tier's own page measure.
        crate::theme::set_active(crate::theme::DEFAULT_THEME);
        crate::page::set_measure(config.measure_for(tier.class()));
        let misspelled = spell.misspellings_for(&text, tier.syn_lang());
        let mut cx = cx::Cx::new(&device, &queue, &cache, &config, tier, text, misspelled)?;
        for sc in Scenario::ALL {
            if let Some(reason) = scenarios::skip_reason(tier, sc) {
                skips.push(report::SkipRec {
                    tier: tier.name().to_string(),
                    scenario: sc.name().to_string(),
                    reason: reason.to_string(),
                });
                continue;
            }
            let out = scenarios::run_scenario(sc, &mut cx)?;
            let samples = out.samples_ms.len() as u64;
            let (min, median, p90) = stats(out.samples_ms);
            let witness = out
                .witness
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ");
            println!(
                "{:>6} | {:>10} | {:>7} | {:>10.3} | {:>10.3} | {:>10.3} | {}",
                tier.name(),
                sc.name(),
                samples,
                min,
                median,
                p90,
                witness
            );
            cells.push(report::CellRec {
                tier: tier.name().to_string(),
                scenario: sc.name().to_string(),
                samples,
                min_ms: min,
                median_ms: median,
                p90_ms: p90,
                witness: out
                    .witness
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
            });
        }
    }
    println!();
    for k in &skips {
        println!("skipped {} x {} — {}", k.tier, k.scenario, k.reason);
    }
    let wall_s = wall0.elapsed().as_secs_f64();
    println!("total wall time: {wall_s:.1}s over {} cells (+{} documented skips)", cells.len(), skips.len());

    let doc = report::BenchDoc::gather(wall_s, cells, skips);
    let out_path = PathBuf::from("bench.json");
    // Through the ONE atomic-write door (`fs::write_atomic`, the durable-write
    // law) — a half-written bench.json would poison the next baseline diff.
    crate::fs::write_atomic(&out_path, doc.to_json().as_bytes())
        .map_err(|e| anyhow::anyhow!("failed to write {}: {e}", out_path.display()))?;
    println!("wrote {} ({} · {} · {})", out_path.display(), doc.host, doc.arch, doc.profile);

    if let Some(bp) = baseline {
        println!();
        report::diff_against(&bp, &doc)?;
    }
    Ok(())
}

/// Min + median + p90 over a sample set (sorted copy; p90 = the ceil(0.9n)-th
/// value, honest for the small per-cell sample counts the suite runs). The
/// MIN is the baseline-diff statistic — see `report`'s WARN_RATIO doc.
fn stats(mut v: Vec<f64>) -> (f64, f64, f64) {
    assert!(!v.is_empty(), "a cell must have samples");
    v.sort_by(|a, b| a.partial_cmp(b).expect("bench samples are finite"));
    let median = v[v.len() / 2];
    let p90_idx = ((v.len() as f64 * 0.9).ceil() as usize).clamp(1, v.len()) - 1;
    (v[0], median, v[p90_idx])
}

// ============================================================================
// WORK-COUNT TRIPWIRES — deterministic, timing-free invariants of the render
// pipeline's work accounting, dev-profile safe, run under `cargo test`. Each
// one is a real regression guard for the accidental-O(doc) class the suite's
// witnesses lean on; if one of these breaks, the suite's cells start measuring
// a different workload (and the baseline silently lies).
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{TextPipeline, ViewState};

    /// Headless (device, queue, pipeline) at the suite canvas, shaped on the
    /// SMALL corpus tier (dev-profile friendly). None when no wgpu adapter.
    fn shaped_pipeline() -> Option<(wgpu::Device, wgpu::Queue, TextPipeline, ViewState)> {
        let (device, queue) = pollster::block_on(crate::capture::gpu::headless_device()).ok()?;
        let cache = glyphon::Cache::new(&device);
        let mut p = TextPipeline::new(&device, &queue, &cache, crate::capture::FORMAT);
        p.set_size(WIDTH as f32, HEIGHT as f32);
        p.set_dpi(DPI);
        let view = ViewState {
            text: corpus::text(Tier::S),
            is_markdown: true,
            ..ViewState::base()
        };
        p.set_view(&view);
        Some((device, queue, p, view))
    }

    /// TRIPWIRE 1: a PURE SCROLL step (only `scroll_lines` changes) schedules
    /// ZERO reshapes and leaves the shaped row geometry untouched — scrolling
    /// must stay O(visible), never O(doc).
    #[test]
    fn pure_scroll_step_schedules_zero_reshapes() {
        let _g = crate::testlock::serial();
        let Some((_d, _q, mut p, mut view)) = shaped_pipeline() else {
            eprintln!("skipping pure_scroll_step_schedules_zero_reshapes: no wgpu adapter");
            return;
        };
        let reshapes = p.reshape_count;
        let geom_gen = p.row_geom.generation();
        for scroll in [1usize, 5, 20] {
            view.scroll_lines = scroll;
            p.set_view(&view);
        }
        assert_eq!(p.reshape_count, reshapes, "a pure scroll must never reshape");
        assert_eq!(
            p.row_geom.generation(),
            geom_gen,
            "a pure scroll must not invalidate the shaped row geometry"
        );
    }

    /// TRIPWIRE 2: pushing an IDENTICAL view twice is reshape-free — the
    /// composed-text compare must keep a redraw-without-change free.
    #[test]
    fn identical_set_view_is_reshape_free() {
        let _g = crate::testlock::serial();
        let Some((_d, _q, mut p, view)) = shaped_pipeline() else {
            eprintln!("skipping identical_set_view_is_reshape_free: no wgpu adapter");
            return;
        };
        let reshapes = p.reshape_count;
        p.set_view(&view);
        p.set_view(&view);
        assert_eq!(
            p.reshape_count, reshapes,
            "an unchanged view must not reshape"
        );
    }

    /// TRIPWIRE 3: ONE zoom change reshapes EXACTLY once — the seam the
    /// zoom-burst coalescing (latest-wins at the present boundary) relies on;
    /// a second reshape here would double every zoom step's cost silently.
    #[test]
    fn one_zoom_change_reshapes_exactly_once() {
        let _g = crate::testlock::serial();
        let Some((_d, _q, mut p, mut view)) = shaped_pipeline() else {
            eprintln!("skipping one_zoom_change_reshapes_exactly_once: no wgpu adapter");
            return;
        };
        let reshapes = p.reshape_count;
        view.zoom = 1.1;
        p.set_view(&view);
        assert_eq!(
            p.reshape_count,
            reshapes + 1,
            "one zoom change must reshape exactly once"
        );
    }

    /// TRIPWIRE 4: `sync_theme` reshapes IFF the effective face/palette really
    /// changed — a same-world re-sync stays free (the idle re-preview), and a
    /// different-face switch genuinely pays (the old `--bench-perf` THEME
    /// stage once measured a no-op; this pins both directions).
    #[test]
    fn sync_theme_reshapes_iff_the_face_changes() {
        let _g = crate::testlock::serial();
        let Some((_d, _q, mut p, _view)) = shaped_pipeline() else {
            eprintln!("skipping sync_theme_reshapes_iff_the_face_changes: no wgpu adapter");
            return;
        };
        crate::theme::set_active_by_name("Gumtree").expect("Gumtree exists");
        p.sync_theme();
        let reshapes = p.reshape_count;
        // Same world again: free.
        p.sync_theme();
        assert_eq!(p.reshape_count, reshapes, "a same-world sync_theme must not reshape");
        // A different-face world: must pay a real reshape.
        crate::theme::set_active_by_name("Tawny").expect("Tawny exists");
        p.sync_theme();
        assert!(
            p.reshape_count > reshapes,
            "a different-face theme switch must actually reshape"
        );
        crate::theme::set_active(crate::theme::DEFAULT_THEME);
    }

    /// The scroll cell's OUTCOME witness leans on `row_top_px(scroll_lines)`
    /// as its oracle; this pins the oracle itself — exactly 0 at the top,
    /// strictly positive once the viewport really moves — so a pinned
    /// viewport (the vacuous-witness sabotage the repair round fixed: a
    /// no-op'd scroll passing on caret animation alone) can never satisfy
    /// the cell's per-step ensure.
    #[test]
    fn scroll_offset_oracle_moves_with_the_viewport() {
        let _g = crate::testlock::serial();
        let Some((_d, _q, mut p, mut view)) = shaped_pipeline() else {
            eprintln!("skipping scroll_offset_oracle_moves_with_the_viewport: no wgpu adapter");
            return;
        };
        assert_eq!(
            p.row_top_px(p.scroll_lines),
            0.0,
            "the top of the document must resolve offset 0"
        );
        let rows = p.total_visual_rows();
        assert!(rows > 1, "the S corpus must shape more than one visual row");
        view.scroll_lines = rows - 1;
        p.set_view(&view);
        assert!(
            p.row_top_px(p.scroll_lines) > 0.0,
            "a scrolled viewport must resolve a strictly positive offset"
        );
    }

    /// The matrix has NO silent holes: every tier x scenario cell is either
    /// run or carries a documented skip reason (today: exactly two skips —
    /// CODE x zoom by taste, XPARA x resize banked on the shape-budget gap).
    #[test]
    fn every_matrix_hole_is_documented() {
        let mut skips = 0usize;
        for tier in Tier::ALL {
            for sc in Scenario::ALL {
                if scenarios::skip_reason(tier, sc).is_some() {
                    skips += 1;
                }
            }
        }
        assert_eq!(skips, 2, "every skip must be deliberate and documented");
        assert!(
            scenarios::skip_reason(Tier::Code, Scenario::Zoom).is_some(),
            "the CODE x zoom skip is documented"
        );
        assert!(
            scenarios::skip_reason(Tier::XPara, Scenario::Resize).is_some(),
            "the XPARA x resize skip is documented (the shape-budget gap)"
        );
    }

    #[test]
    fn stats_are_order_statistics() {
        let (lo, m, p) = stats(vec![5.0, 1.0, 3.0, 2.0, 4.0]);
        assert_eq!((lo, m, p), (1.0, 3.0, 5.0));
        let (lo, m, p) = stats(vec![7.0]);
        assert_eq!((lo, m, p), (7.0, 7.0, 7.0));
    }
}
