//! UNIT TESTS for the caret spring / morph / juice / preview machinery,
//! split by feature area (2026-07 code-organization pass) out of the former
//! monolithic `caret::tests` module into this `caret/tests/` directory --
//! every test's NAME is unchanged, only its module path grew one segment
//! (`caret::tests::foo` -> `caret::tests::<area>::foo`). NOTE: two areas
//! are named `spring_settle`/`impact` rather than the shorter `spring`/`juice`
//! -- `caret.rs` already declares real `spring`/`juice` submodules, and a
//! same-named test module would shadow them under a blanket glob (the same
//! trap `render/tests/` hit with `theme`/`geometry`). This file owns every
//! shared fixture helper (`settle`, `primed_caret`, `trail_head_y`,
//! `drawn_streak_len`, `gate_streak_len`) since several are called from tests
//! in more than one area; each child re-derives `caret` root access via its
//! own `use super::super::*;` plus a targeted `use super::{..};` for
//! whichever helpers it actually calls.

use super::*;

mod impact;
mod mode;
mod spring_settle;
mod streak_underline;
mod trail;

/// Helper: run the spring to rest from a downward jump and report frames +
/// whether it overshot the target.
pub(super) fn settle(target: Sample, start: Sample, dt: f32) -> (usize, bool, f32) {
    let mut a = CaretAnim::new();
    // Prime at start so the next set_target glides.
    a.set_target(start.x, start.y);
    a.set_target(target.x, target.y);
    let mut frames = 0;
    let mut overshot = false;
    // The caret starts at `start` and glides UP to `target` (target.y < start.y).
    while a.is_animating() && frames < 2000 {
        a.step(dt);
        frames += 1;
        // Overshoot = pos goes past target in the direction of travel.
        if start.y > target.y && a.pos.y < target.y - 0.5 {
            overshot = true;
        }
    }
    (frames, overshot, a.pos.y)
}

/// Prime a caret on a glyph with the default zoom-1 yardsticks so the trail gate
/// measures moves in real chars/lines.
pub(super) fn primed_caret() -> CaretAnim {
    let mut a = CaretAnim::new();
    a.set_glyph_advance(crate::render::CHAR_WIDTH);
    a.set_line_height(crate::render::LINE_HEIGHT);
    a.set_target(200.0, 200.0); // prime (snaps; no trail)
    assert!(!a.trail_active(), "a fresh prime must draw no trail");
    a
}

/// The leading-edge HEAD y of the cosmetic streak, as the renderer/sidecar read it
/// (head endpoint = center + axis*half_along). Zero text-drop so it's the bare span.
pub(super) fn trail_head_y(a: &CaretAnim) -> f32 {
    let (c, half, _across, axis) = a.trail_geometry(3.0, CARET_STREAK_GAP, 0.0, 0.0);
    c.y + axis.1 * half
}

/// The DRAWN trailing-streak length (px) the renderer would emit for the
/// caret's current state, computed through the exact production path
/// (`streak_length` → `motion_geometry`) so the held-trail tests assert on
/// what actually paints, not a re-derived approximation.
pub(super) fn drawn_streak_len(a: &CaretAnim, m: &crate::render::Metrics) -> f32 {
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

/// The DRAWN streak length helper (same as the held-trail tests) so the gate
/// tests assert on what actually paints.
pub(super) fn gate_streak_len(a: &CaretAnim, m: &crate::render::Metrics) -> f32 {
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
