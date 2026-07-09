//! The held-key STREAK + its underline-suppression rules (typing vs
//! navigation, edit-move vs held-arrow, per-frame dx reporting) -- split out
//! of the former monolithic `caret::tests` (2026-07 code-organization
//! pass).

use super::super::*;
use super::{drawn_streak_len};

#[test]
fn typing_hop_shows_no_underline() {
    // A single-character advance is an EDIT (the renderer flags it from the
    // bumped buffer version), so it must NOT drop to the underline:
    // settle_factor stays pinned at 1.0 for the whole slide.
    let adv = crate::render::CHAR_WIDTH;
    let mut a = CaretAnim::new();
    a.set_glyph_advance(adv);
    a.set_target(100.0, 50.0); // prime / snap
    a.set_edit_move(true); // typing one char is an edit
    a.set_target(100.0 + adv, 50.0);
    let mut min_s = a.settle_factor();
    let mut frames = 0;
    while a.is_animating() && frames < 2000 {
        a.step(1.0 / 120.0);
        min_s = min_s.min(a.settle_factor());
        frames += 1;
    }
    assert!(
        min_s > 0.999,
        "a typing hop must not show the underline, min settle={min_s}"
    );
}

#[test]
fn mashing_keys_shows_no_underline() {
    // Type so fast (one char EVERY frame) the spring can't catch up and falls
    // several advances behind. Because each keystroke is an EDIT, the underline
    // stays suppressed however far behind the spring lags.
    let adv = crate::render::CHAR_WIDTH;
    let mut a = CaretAnim::new();
    a.set_glyph_advance(adv);
    a.set_target(100.0, 50.0); // prime
    let mut tx = 100.0_f32;
    let mut min_s = a.settle_factor();
    let mut max_lag = 0.0_f32;
    for _ in 0..30 {
        tx += adv; // one-char advance per frame
        a.set_edit_move(true); // every keystroke is an edit
        a.set_target(tx, 50.0);
        a.step(1.0 / 60.0);
        min_s = min_s.min(a.settle_factor());
        max_lag = max_lag.max((a.target.x - a.pos.x).abs());
    }
    while a.is_animating() {
        a.step(1.0 / 60.0);
        min_s = min_s.min(a.settle_factor());
    }
    // The burst really did outrun the spring (else the test proves nothing).
    assert!(
        max_lag > 1.5 * adv,
        "test must drive the spring past the threshold, lag={} adv",
        max_lag / adv
    );
    // ...yet no underline ever appeared.
    assert!(min_s > 0.999, "mashing keys must not show the underline, min settle={min_s}");
}

#[test]
fn held_arrow_navigation_shows_underline() {
    // Holding left/right is NAVIGATION (not an edit), a burst of one-char
    // steps. As the caret races ahead and the spring falls behind, the streak
    // must bloom — the motion feedback that was wrongly muted by the old
    // per-keystroke distance gate.
    let adv = crate::render::CHAR_WIDTH;
    let mut a = CaretAnim::new();
    a.set_glyph_advance(adv);
    a.set_target(100.0, 50.0); // prime
    let mut tx = 100.0_f32;
    let mut min_s = a.settle_factor();
    // One char per frame at 60fps (key-repeat), NOT flagged as an edit.
    for _ in 0..30 {
        tx += adv;
        a.set_target(tx, 50.0); // edit_move stays false
        a.step(1.0 / 60.0);
        min_s = min_s.min(a.settle_factor());
    }
    // The underline appeared (and on the horizontal axis).
    assert!(min_s < 0.5, "held-arrow navigation must show the underline, min settle={min_s}");
    assert!(!a.is_vertical_move(), "horizontal nav must use the horizontal axis");
}

#[test]
fn held_horizontal_motion_draws_continuous_streak_over_gap() {
    // Holding LEFT/RIGHT is a CONTINUOUS chain of one-char hops (OS auto-repeat
    // ⇒ `set_held(true)`). The spring must stay springy and LAG, so the trail
    // spans the accumulated travel and draws a stable streak comfortably past
    // the gap on EVERY hop — never collapsing to nothing (the "held L/R trail
    // vanishes" regression).
    let m = crate::render::Metrics::new(1.0);
    let adv = m.char_width;
    let gap = m.caret_streak_gap;
    let mut a = CaretAnim::new();
    a.set_glyph_advance(adv);
    a.set_line_height(m.line_height);
    a.set_target(100.0, 50.0); // prime / snap (the initial PRESS, not a repeat)
    let mut tx = 100.0_f32;
    let mut min_streak = f32::INFINITY;
    let mut max_streak = 0.0_f32;
    let mut sampled = 0;
    for i in 0..24 {
        tx += adv;
        a.set_held(true); // every subsequent event is an OS auto-repeat
        a.set_target(tx, 50.0); // one-char navigation hop
        a.step(1.0 / 60.0);
        if i >= 6 {
            // ...once the lagging trail has established.
            let len = drawn_streak_len(&a, &m);
            min_streak = min_streak.min(len);
            max_streak = max_streak.max(len);
            sampled += 1;
        }
    }
    assert!(sampled > 0);
    assert!(a.is_holding(), "a held burst must latch the holding state");
    assert!(!a.is_vertical_move(), "held L/R must stay on the horizontal axis");
    assert!(
        min_streak > gap,
        "held L/R must draw a continuous streak over the gap ({gap}), min={min_streak}"
    );
    // STEADY: the held length is a constant, not a per-repeat pulse, so the
    // min/max spread across the run is negligible.
    assert!(
        (max_streak - min_streak) <= 0.10 * min_streak,
        "held L/R streak must be steady, spread={} (min={min_streak}, max={max_streak})",
        max_streak - min_streak
    );
}

#[test]
fn held_vertical_motion_does_not_strobe() {
    // Holding UP/DOWN: each line-hop must SUSTAIN a stable trail across
    // consecutive repeats — never flicking to a zero-length streak between hops
    // (the "held U/D strobes" regression). We assert the drawn streak is BOTH
    // non-zero on every established hop AND always past the gap.
    let m = crate::render::Metrics::new(1.0);
    let lh = m.line_height;
    let gap = m.caret_streak_gap;
    let mut a = CaretAnim::new();
    a.set_glyph_advance(m.char_width);
    a.set_line_height(lh);
    a.set_target(100.0, 100.0); // prime / snap
    let mut ty = 100.0_f32;
    let mut min_streak = f32::INFINITY;
    let mut max_streak = 0.0_f32;
    let mut strobed_to_zero = false;
    let mut sampled = 0;
    for i in 0..18 {
        ty += lh;
        a.set_held(true);
        a.set_target(100.0, ty); // one-line held hop down
        a.step(1.0 / 60.0);
        if i >= 5 {
            let len = drawn_streak_len(&a, &m);
            if len < 1.0 {
                strobed_to_zero = true;
            }
            min_streak = min_streak.min(len);
            max_streak = max_streak.max(len);
            sampled += 1;
        }
    }
    assert!(sampled > 0);
    assert!(a.is_vertical_move(), "held down must latch the vertical axis");
    assert!(!strobed_to_zero, "held U/D trail must not strobe to a zero-length streak");
    assert!(
        min_streak > gap,
        "held U/D must keep a stable streak over the gap ({gap}), min={min_streak}"
    );
    // STEADY: a constant held length, so the run's min/max spread is negligible
    // (no per-repeat pulse).
    assert!(
        (max_streak - min_streak) <= 0.10 * min_streak,
        "held U/D streak must be steady, spread={} (min={min_streak}, max={max_streak})",
        max_streak - min_streak
    );
}

#[test]
fn lone_short_hop_draws_no_trail() {
    // A SINGLE discrete tap (one arrow press, then stop ⇒ `held` stays false)
    // is a lone one-char hop. The full gap must suppress it: the caret never
    // extends a trailing streak past the gap — it stays within the resting
    // block and re-forms — so a tap reads clean (no stray streak).
    let m = crate::render::Metrics::new(1.0);
    let adv = m.char_width;
    let gap = m.caret_streak_gap;
    let mut a = CaretAnim::new();
    a.set_glyph_advance(adv);
    a.set_line_height(m.line_height);
    a.set_target(100.0, 50.0); // prime / snap
    a.set_target(100.0 + adv, 50.0); // ONE navigation hop (held stays false)
    let mut max_streak = 0.0_f32;
    let mut frames = 0;
    while a.is_animating() && frames < 2000 {
        a.step(1.0 / 120.0);
        max_streak = max_streak.max(drawn_streak_len(&a, &m));
        frames += 1;
    }
    assert!(!a.is_holding(), "a lone tap must not latch the holding state");
    assert!(
        max_streak < gap,
        "a lone short hop must draw NO trail past the gap ({gap}), max={max_streak}"
    );
}

#[test]
fn move_axis_is_latched_per_move() {
    // The travel axis is decided per move from the logical move delta, so a
    // vertical move is vertical and a horizontal move is horizontal —
    // regardless of momentary velocity. (Stops the up/down shape flicker.)
    let mut a = CaretAnim::new();
    a.set_glyph_advance(crate::render::CHAR_WIDTH);
    a.set_target(100.0, 100.0); // prime
    a.set_target(100.0, 300.0); // straight down
    assert!(a.is_vertical_move(), "a downward move must latch the vertical axis");
    a.set_target(300.0, 300.0); // straight right
    assert!(!a.is_vertical_move(), "a rightward move must latch the horizontal axis");
}

#[test]
fn vertical_move_stays_vertical_despite_big_column_jump() {
    // Down-arrow from a mid-row column into a short line: y advances one line
    // but the goal-column clamp jumps x a long way left. The move must still be
    // VERTICAL (row-crossing), so the streak doesn't flicker to a horizontal
    // underline mid-row — the bug the |dy|>|dx| test had.
    let mut a = CaretAnim::new();
    a.set_glyph_advance(crate::render::CHAR_WIDTH);
    a.set_line_height(crate::render::LINE_HEIGHT);
    a.set_target(300.0, 100.0); // prime: a mid-row column on a long line
    // Down ONE line (dy = LINE_HEIGHT) while x jumps left far more than that.
    a.set_target(40.0, 100.0 + crate::render::LINE_HEIGHT);
    assert!(
        a.is_vertical_move(),
        "a down move must stay vertical despite a big column/x jump"
    );
}

#[test]
fn edit_move_suppresses_underline_even_when_large() {
    // An edit can move the caret a long way in one step (Enter to a far
    // column, a wide/CJK glyph, a paste), but it's still typing — no
    // underline, however large the jump.
    let mut a = CaretAnim::new();
    a.set_glyph_advance(crate::render::CHAR_WIDTH);
    a.set_target(16.0, 40.0); // prime
    a.set_edit_move(true);
    a.set_target(200.0, 90.0); // big move, but flagged as an edit
    let mut min_s = a.settle_factor();
    while a.is_animating() {
        a.step(1.0 / 120.0);
        min_s = min_s.min(a.settle_factor());
    }
    assert!(min_s > 0.999, "an edit move must not streak even when large, min={min_s}");
}

#[test]
fn navigation_jump_still_shows_underline() {
    // A real jump (here a full-line Ctrl-E style glide) must still collapse
    // to the streak mid-flight — suppression is only for typing-sized hops.
    let mut a = CaretAnim::new();
    a.set_glyph_advance(crate::render::CHAR_WIDTH);
    a.set_target(16.0, 40.0); // prime / snap
    a.set_target(600.0, 40.0); // long horizontal jump
    let mut min_s = a.settle_factor();
    while a.is_animating() {
        a.step(1.0 / 120.0);
        min_s = min_s.min(a.settle_factor());
    }
    assert!(min_s < 0.2, "a navigation jump must still show the underline, min={min_s}");
}

#[test]
fn frame_dx_reports_large_per_frame_advance_mid_glide() {
    // A fast full-line glide moves farther than the streak clamp in a single
    // 60fps frame; frame_dx() must report that large advance so the renderer
    // can bridge the streak across it.
    let mut a = CaretAnim::new();
    a.set_glyph_advance(crate::render::CHAR_WIDTH);
    a.set_target(0.0, 0.0); // prime / snap
    a.set_target(1200.0, 0.0); // fast cross-screen jump
    a.step(1.0 / 60.0);
    assert!(
        a.frame_dx().abs() > 64.0,
        "fast glide must move more than the streak clamp in one frame, got {}",
        a.frame_dx()
    );

    // The deterministic injected-motion screenshot path leaves frame_dx at 0.
    let mut b = CaretAnim::new();
    b.inject_motion(
        Sample { x: 1000.0, y: 0.0 },
        Sample { x: 200.0, y: 0.0 },
        Sample { x: 1900.0, y: 0.0 },
    );
    assert_eq!(b.frame_dx(), 0.0, "injected motion must keep frame_dx == 0");
}

#[test]
fn single_line_vertical_move_is_near_critical() {
    let adv = crate::render::CHAR_WIDTH;
    let lh = crate::render::LINE_HEIGHT;

    // A single DOWN-one-line hop must use the near-critical SMALL_MOVE_DAMPING
    // (no overshoot), matching a single left/right hop — NOT the springy band
    // the old euclidean dist/glyph_advance classification put it in.
    let mut a = CaretAnim::new();
    a.set_glyph_advance(adv);
    a.set_line_height(lh);
    a.set_target(100.0, 100.0); // prime
    a.set_target(100.0, 100.0 + lh); // down one line
    assert!(
        (a.damping - SMALL_MOVE_DAMPING).abs() < 1e-3,
        "single vertical hop must be near-critical, got {}",
        a.damping
    );

    // Even when the goal-column clamps x a long way (down-arrow into a short
    // line), it is still the one-ROW hop, so it stays near-critical.
    let mut b = CaretAnim::new();
    b.set_glyph_advance(adv);
    b.set_line_height(lh);
    b.set_target(400.0, 100.0); // prime: a far-right column
    b.set_target(40.0, 100.0 + lh); // down one line, x clamps far left
    assert!(
        (b.damping - SMALL_MOVE_DAMPING).abs() < 1e-3,
        "vertical hop with a big x clamp must stay near-critical, got {}",
        b.damping
    );

    // A LONG multi-line jump must keep its springy DAMPING (life preserved).
    let mut c = CaretAnim::new();
    c.set_glyph_advance(adv);
    c.set_line_height(lh);
    c.set_target(100.0, 100.0); // prime
    c.set_target(100.0, 100.0 + 10.0 * lh); // ten lines down
    assert!(
        (c.damping - DAMPING).abs() < 1e-3,
        "a ten-line vertical jump must stay springy, got {}",
        c.damping
    );

    // Horizontal single hop is unchanged (still near-critical).
    let mut d = CaretAnim::new();
    d.set_glyph_advance(adv);
    d.set_line_height(lh);
    d.set_target(100.0, 50.0); // prime
    d.set_target(100.0 + adv, 50.0); // one glyph right
    assert!(
        (d.damping - SMALL_MOVE_DAMPING).abs() < 1e-3,
        "a single left/right hop must remain near-critical, got {}",
        d.damping
    );
}
