//! The pure spring-physics settle math (critically/under-damped, epsilon
//! settling, the pop-kick ease) -- split out of the former monolithic
//! `caret::tests` (2026-07 code-organization pass).

use super::super::*;
use super::{settle};

#[test]
fn first_target_snaps_no_glide() {
    let mut a = CaretAnim::new();
    a.set_target(100.0, 200.0);
    assert!(!a.is_animating(), "first target must snap, not animate");
    assert_eq!(a.pos, Sample { x: 100.0, y: 200.0 });
}

#[test]
fn timeline_injected_dt_progresses_and_is_deterministic() {
    // Models the `--capture-timeline` virtual clock: prime at the ORIGIN, glide
    // toward the DESTINATION, then advance by an INJECTED cumulative-ms
    // sequence. The animated x must progress MONOTONICALLY from near the origin
    // toward the destination, and stepping the same sequence twice must be
    // byte-identical (no clock, no RNG).
    let origin = Sample { x: 16.0, y: 200.0 };
    let dest = Sample { x: 600.0, y: 200.0 };
    // Cumulative ms since the move started; dt for step i is t[i]-t[i-1].
    let steps_ms: [u32; 5] = [0, 16, 50, 150, 400];

    let run = || -> Vec<f32> {
        let mut a = CaretAnim::new();
        a.set_target(origin.x, origin.y); // prime (snaps at origin)
        a.set_target(dest.x, dest.y); // start the glide
        let mut prev_ms = 0u32;
        let mut xs = Vec::new();
        for &t in &steps_ms {
            let dt = (t.saturating_sub(prev_ms)) as f32 / 1000.0;
            prev_ms = t;
            a.step(dt);
            xs.push(a.pos.x);
        }
        xs
    };

    let xs = run();
    // t0: no step taken yet -> still at the origin.
    assert!((xs[0] - origin.x).abs() < 1e-6, "t0 must be at origin: {}", xs[0]);
    // Strictly progressing toward the destination across the early/mid steps.
    for w in xs.windows(2).take(3) {
        assert!(w[1] > w[0], "caret x must progress toward target: {w:?}");
    }
    // Mid-glide is genuinely BETWEEN origin and destination (a real trajectory,
    // not an instant snap).
    assert!(
        xs[1] > origin.x && xs[1] < dest.x,
        "t16 must be mid-glide: {}",
        xs[1]
    );
    // Late in the sequence the caret has effectively arrived at the line end.
    let last = *xs.last().unwrap();
    assert!((last - dest.x).abs() < POS_EPSILON, "late step must settle at target: {last}");

    // Determinism: the injected-dt sequence is byte-identical across runs.
    assert_eq!(xs, run(), "injected-dt timeline must be deterministic");
}

#[test]
fn spring_settles_and_stops() {
    // Glide from y=300 up to y=20 at 60 fps.
    let (frames, _overshot, final_y) = settle(
        Sample { x: 16.0, y: 20.0 },
        Sample { x: 16.0, y: 300.0 },
        1.0 / 60.0,
    );
    // Must come to rest exactly on target and stop animating.
    assert!((final_y - 20.0).abs() < 1.0, "did not settle on target: {final_y}");
    // ~140-160 ms at 60 fps is ~9-11 frames; allow slack but bound it so a
    // runaway/never-settling spring fails the test.
    assert!(frames > 3 && frames < 60, "settle frames out of range: {frames}");
}

#[test]
fn spring_is_underdamped_overshoots() {
    // A lightly underdamped spring should overshoot the target slightly.
    let (_frames, overshot, _final_y) = settle(
        Sample { x: 16.0, y: 20.0 },
        Sample { x: 16.0, y: 400.0 },
        1.0 / 120.0,
    );
    assert!(overshot, "expected a small overshoot (underdamped feel)");
}

#[test]
fn settles_within_epsilon() {
    let mut a = CaretAnim::new();
    a.set_target(0.0, 0.0);
    a.set_target(50.0, 50.0);
    while a.is_animating() {
        a.step(1.0 / 60.0);
    }
    let dx = (a.pos.x - a.target.x).abs();
    let dy = (a.pos.y - a.target.y).abs();
    assert!(dx <= POS_EPSILON && dy <= POS_EPSILON);
    assert_eq!(a.vel.x, 0.0);
    assert_eq!(a.vel.y, 0.0);
}

#[test]
fn pop_kicks_below_one_then_eases_back_with_pos_pinned() {
    let mut a = CaretAnim::new();
    // Prime on a glyph (snaps; no pop, settled at scale 1.0).
    a.set_target(100.0, 50.0);
    assert!((a.pop_scale() - 1.0).abs() < 1e-6, "prime must not pop");

    // A SMALL navigation move (one glyph advance right): the position SNAPS to
    // target instantly (pinned), and the cosmetic pop kicks.
    a.nav_to(100.0 + crate::render::CHAR_WIDTH, 50.0);
    let target = a.target;
    assert_eq!(a.pos.x, target.x, "small move must pin pos.x to target at t0");
    assert_eq!(a.pos.y, target.y, "small move must pin pos.y to target at t0");
    assert!(!a.is_animating(), "a small move snaps: the spring must not animate");

    // The pop is squashed below 1 (down to ~CARET_POP_SCALE) right after the kick.
    let s0 = a.pop_scale();
    assert!(s0 < 1.0, "pop must squash the drawn scale below 1: {s0}");
    assert!(s0 >= CARET_POP_SCALE - 1e-6, "pop must not squash past CARET_POP_SCALE: {s0}");

    // Step the LIVE clock: the scale eases monotonically back to 1.0 while the
    // caret POSITION stays pinned to target the whole time (the pop never moves it).
    let mut prev = s0;
    let mut popping = true;
    let mut frames = 0;
    while popping && frames < 1000 {
        popping = a.step_pop(1.0 / 120.0);
        assert_eq!(a.pos.x, target.x, "pop must not move pos.x");
        assert_eq!(a.pos.y, target.y, "pop must not move pos.y");
        assert!(!a.is_animating(), "pop must never animate the spring/position");
        let s = a.pop_scale();
        assert!(s + 1e-6 >= prev, "pop scale must ease back monotonically: {prev} -> {s}");
        assert!(s <= 1.0 + 1e-6, "pop scale must never exceed 1.0: {s}");
        prev = s;
        frames += 1;
    }
    assert!((a.pop_scale() - 1.0).abs() < 1e-6, "pop must settle exactly at scale 1.0");
    // ~90ms at 120fps is ~11 frames; bound it so a never-settling pop fails.
    assert!(frames > 3 && frames < 60, "pop settle frames out of range: {frames}");

    // RE-KICK (a held repeat) restarts the squash with the position still pinned.
    a.kick_pop();
    assert!(a.pop_scale() < 1.0, "re-kick must squash again (interruptible)");
    assert_eq!(a.pos.x, target.x);
    assert_eq!(a.pos.y, target.y);
}

#[test]
fn snap_to_target_settles_the_pop() {
    // The deterministic capture path snaps (settle) AFTER a move may have kicked
    // the pop on the prime/settle sequence; the frozen frame must be full-scale.
    let mut a = CaretAnim::new();
    a.set_target(0.0, 0.0);
    a.nav_to(80.0, 0.0); // kicks the pop
    assert!(a.pop_scale() < 1.0);
    a.snap_to_target();
    assert!((a.pop_scale() - 1.0).abs() < 1e-6, "snap_to_target must settle the pop");
}
