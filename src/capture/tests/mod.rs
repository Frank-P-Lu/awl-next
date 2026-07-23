//! Capture-path tests, split by feature area (2026-07 code-organization
//! pass) out of the former monolithic `capture::tests` module into this
//! `capture/tests/` directory -- every test's NAME is unchanged, only its
//! module path grew one segment (`capture::tests::foo` ->
//! `capture::tests::<area>::foo`). This file owns the shared drive-helpers
//! (`adapter_available`, `num_after`, `drawn_streak_len`, the held-key run
//! fixtures) since more than one area calls them; each child re-derives
//! `capture` root access via its own `use super::super::*;` plus a targeted
//! `use super::{..};` for whichever helpers it actually calls.

use super::animated::step_held;
use super::*;

use crate::caret::CaretAnim;
use crate::render;

mod caret_streak;
mod folds;
mod i18n_fixtures;
mod panels;
mod pickers_faceted;
mod schema_chrome;

/// Re-derive the DRAWN streak length (px) for the caret's current spring state
/// through the exact production path (`streak_length` → `motion_geometry`),
/// mirroring the renderer's `caret_geometry`/`caret_trail_report`.
pub(super) fn drawn_streak_len(a: &CaretAnim, m: &render::Metrics) -> f32 {
    let speed = (a.vel.x * a.vel.x + a.vel.y * a.vel.y).sqrt();
    let streak_len = a.streak_length(
        m.streak_len_for_speed(speed),
        m.caret_streak_max_len,
        m.caret_held_len,
    );
    let (_c, half_along, _half_across, _axis) = a.motion_geometry(
        m.caret_w,
        m.caret_block_h,
        m.caret_streak_h,
        streak_len,
        m.caret_streak_gap,
        m.caret_trail_drop,
    );
    half_along * 2.0
}

/// Drive the SAME deterministic re-targeting the held-capture harness uses
/// (`step_held` one char/line per virtual-clock step, `held=true`), and assert
/// the DRAWN trail across the sustained held run is (a) always clear of the gap
/// (never flickering out) AND (b) STEADY — a low-variance, near-constant length,
/// not the per-repeat pulse the instantaneous-velocity length used to draw. This
/// is the harness-level guarantee a human reads off the per-step sidecar
/// `caret.trail.length`.
pub(super) fn held_run_keeps_steady_streak(dir: HeldDir, lens: &[usize], origin: (usize, usize)) {
    let m = render::Metrics::new(1.0);
    let adv = m.char_width;
    let lh = m.line_height;
    let gap = m.caret_streak_gap;
    let last = lens.len() - 1;
    // Cumulative-ms steps like the smoke run (0,30,60,...,210): one held
    // re-target + one injected-dt advance per entry.
    let steps: [u32; 8] = [0, 30, 60, 90, 120, 150, 180, 210];

    let mut a = CaretAnim::new();
    a.set_glyph_advance(adv);
    a.set_line_height(lh);
    // Prime AT REST on the origin (the initial press).
    let to_px = |(l, c): (usize, usize)| (c as f32 * adv + 100.0, l as f32 * lh + 100.0);
    let (ox, oy) = to_px(origin);
    a.set_target(ox, oy);
    a.snap_to_target();

    let mut cur = origin;
    let mut prev_ms = 0u32;
    let mut lengths: Vec<f32> = Vec::new();
    for (i, &t_ms) in steps.iter().enumerate() {
        cur = step_held(cur, dir, lens, last);
        let (x, y) = to_px(cur);
        a.set_held(true);
        a.set_target(x, y);
        let dt = (t_ms.saturating_sub(prev_ms)) as f32 / 1000.0;
        prev_ms = t_ms;
        a.step(dt);
        // Skip the dt=0 priming entry (step 0): no time advanced, so the spring
        // has not yet lagged. From the first real advance on, the trail must be
        // present + steady every step.
        if i >= 1 {
            assert!(a.is_holding(), "held run must stay latched at step {i}");
            lengths.push(drawn_streak_len(&a, &m));
        }
    }
    assert!(!lengths.is_empty());
    // (a) every held step clears the gap — the streak never flickers out.
    for (k, &len) in lengths.iter().enumerate() {
        assert!(
            len > gap,
            "held {:?} step {k} streak {len} must clear the gap {gap}",
            dir as u8
        );
    }
    // (b) the held trail is STEADY: the spread across the run is a small
    // fraction of the mean, not the per-repeat pulse (~13px on ~29px) it was.
    let mean = lengths.iter().sum::<f32>() / lengths.len() as f32;
    let max = lengths.iter().cloned().fold(f32::MIN, f32::max);
    let min = lengths.iter().cloned().fold(f32::MAX, f32::min);
    assert!(
        (max - min) <= 0.10 * mean,
        "held {:?} streak must be steady: spread {} ({min}..{max}) exceeds 10% of mean {mean}",
        dir as u8,
        max - min
    );
}

/// True when a wgpu adapter is present, so the GPU-dependent capture tests can
/// skip gracefully on a headless/CI box (mirrors `render::tests::headless_pipeline`).
pub(super) fn adapter_available() -> bool {
    pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .is_ok()
    })
}

/// Extract the integer/float that follows `"key":` AFTER the first occurrence of
/// `anchor` in the sidecar JSON. Scoped by `anchor` so `page.column.left` /
/// `canvas.width` don't collide with same-named keys elsewhere.
pub(super) fn num_after(json: &str, anchor: &str, key: &str) -> f64 {
    let from = json.find(anchor).expect("anchor present");
    let rest = &json[from..];
    let kpos = rest.find(key).expect("key present after anchor");
    let after = &rest[kpos + key.len()..];
    // Skip `": ` and read the leading numeric token.
    let token: String = after
        .chars()
        .skip_while(|c| !(c.is_ascii_digit() || *c == '-' || *c == '+'))
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
        .collect();
    token.parse().unwrap_or_else(|_| panic!("bad number for {key:?}: {token:?}"))
}
