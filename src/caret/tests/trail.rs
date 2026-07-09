//! The comet TRAIL -- pos-pinned show/fade over horizontal/vertical/held
//! moves, the settle-factor curve, and hop/jump overshoot damping -- split
//! out of the former monolithic `caret::tests` (2026-07 code-organization
//! pass).

use super::super::*;
use super::{primed_caret, trail_head_y, gate_streak_len};

#[test]
fn small_horizontal_move_shows_no_trail_and_pins_pos() {
    let mut a = primed_caret();
    // One glyph-advance right: under CARET_TRAIL_MIN_CHARS -> NO streak, and the
    // small move SNAPS so the position is pinned to target.
    a.nav_to(200.0 + crate::render::CHAR_WIDTH, 200.0);
    assert!(!a.trail_active(), "a 1-char hop must show no cosmetic trail");
    assert!((a.trail_alpha()).abs() < 1e-6, "no trail -> zero alpha");
    assert_eq!(a.pos, a.target, "small move must pin pos to target");
    assert!(!a.is_animating(), "small move snaps: spring must not animate");
}

#[test]
fn vertical_move_shows_trail_and_pins_pos() {
    let mut a = primed_caret();
    // One line down: ANY row change shows the | , and a single line still SNAPS
    // (under the zip-rows gate) so the position is pinned.
    a.nav_to(200.0, 200.0 + crate::render::LINE_HEIGHT);
    assert!(a.trail_active(), "a vertical move must show the | trail");
    assert!(a.is_trail_vertical(), "a row change is a VERTICAL streak");
    assert!(a.trail_alpha() > 0.0, "an active trail has positive alpha");
    assert_eq!(a.pos, a.target, "vertical move must pin pos to target");
    assert!(!a.is_animating(), "single-line move snaps: spring must not animate");
}

#[test]
fn big_horizontal_move_shows_trail_with_pos_pinned() {
    let mut a = primed_caret();
    // Three chars right: past CARET_TRAIL_MIN_CHARS (2) so the streak shows, but
    // under the zip gate (CARET_ZIP_CHARS = 4) so the move still SNAPS -> pinned.
    a.nav_to(200.0 + 3.0 * crate::render::CHAR_WIDTH, 200.0);
    assert!(a.trail_active(), "a >2-char horizontal move must show the streak");
    assert!(!a.is_trail_vertical(), "a same-row jump is a HORIZONTAL streak");
    assert_eq!(a.pos, a.target, "a sub-zip horizontal move must pin pos to target");
    assert!(!a.is_animating(), "a 3-char move snaps: spring must not animate");
}

#[test]
fn trail_fades_out_with_pos_pinned_the_whole_time() {
    let mut a = primed_caret();
    a.nav_to(200.0, 200.0 + crate::render::LINE_HEIGHT);
    let target = a.target;
    let mut prev = a.trail_alpha();
    assert!(prev > 0.0);
    let mut fading = true;
    let mut frames = 0;
    while fading && frames < 1000 {
        fading = a.step_trail(1.0 / 120.0);
        assert_eq!(a.pos, target, "the cosmetic trail must never move the caret");
        let al = a.trail_alpha();
        assert!(al <= prev + 1e-6, "trail alpha must ease DOWN monotonically: {prev} -> {al}");
        prev = al;
        frames += 1;
    }
    assert!(!a.trail_active(), "the trail must fully fade out");
    assert!((a.trail_alpha()).abs() < 1e-6);
    // ~200ms at 120fps is ~24 frames; bound it so a never-fading trail fails.
    assert!(frames > 5 && frames < 120, "trail fade frames out of range: {frames}");
}

#[test]
fn held_repeat_keeps_trail_topped_up_steady() {
    // A held DOWN auto-repeat: re-kick each ~30ms step. The trail must be present
    // and near peak alpha EVERY step (a steady, continuous | — never a strobe).
    let mut a = primed_caret();
    let mut y = 200.0;
    let mut alphas = Vec::new();
    for _ in 0..8 {
        y += crate::render::LINE_HEIGHT;
        a.set_held(true);
        a.nav_to(200.0, y);
        a.step_trail(30.0 / 1000.0);
        assert!(a.trail_active(), "held DOWN must keep the | present each step");
        assert!(a.is_trail_held(), "a held re-kick must be flagged held");
        assert_eq!(a.pos, a.target, "held trail must keep the caret pinned");
        alphas.push(a.trail_alpha());
    }
    // Steady: every step sits near peak (a 30ms slice of a 200ms fade barely dips),
    // so the spread is a small fraction of the peak — no strobe.
    let max = alphas.iter().cloned().fold(f32::MIN, f32::max);
    let min = alphas.iter().cloned().fold(f32::MAX, f32::min);
    assert!(min > 0.0, "held | must never blink out");
    assert!(
        (max - min) <= 0.25 * CARET_TRAIL_ALPHA,
        "held | alpha must be steady: spread {} too large",
        max - min
    );
}

#[test]
fn held_right_one_char_shows_no_trail() {
    // A held RIGHT auto-repeat: one char per step is under the horizontal gate, so
    // NO streak draws on any step (plain snappy cursor), matching | on vertical only.
    let mut a = primed_caret();
    let mut x = 200.0;
    for _ in 0..6 {
        x += crate::render::CHAR_WIDTH;
        a.set_held(true);
        a.nav_to(x, 200.0);
        a.step_trail(30.0 / 1000.0);
        assert!(!a.trail_active(), "held RIGHT 1-char hops must show no trail");
        assert_eq!(a.pos, a.target, "held right keeps the caret pinned");
    }
}

#[test]
fn vertical_trail_sweeps_head_old_to_new_then_fades_pos_pinned() {
    let mut a = primed_caret();
    let from_y = a.pos.y;
    // One line down: a single-line move SNAPS (pos pinned) yet draws the | .
    let to_y = from_y + crate::render::LINE_HEIGHT;
    a.nav_to(200.0, to_y);
    let target = a.target;
    assert_eq!(a.pos, target, "vertical move snaps: pos pinned at t0");

    // At the kick the leading edge sits at the OLD position; the sweep has not run.
    assert!(a.trail_sweep_p() < 1e-3, "sweep starts at 0 (edge at old)");
    assert!(
        (trail_head_y(&a) - from_y).abs() < 1e-3,
        "the streak head starts at the OLD caret y"
    );

    // Over the SWEEP window the head whips DOWN (old→new), monotonically, while the
    // caret position stays pinned the whole time.
    let mut prev_head = trail_head_y(&a);
    let mut prev_sweep = a.trail_sweep_p();
    let mut t = 0.0f32;
    let sweep_s = CARET_TRAIL_SWEEP_MS / 1000.0;
    while t < sweep_s - 1e-4 {
        a.step_trail(1.0 / 240.0);
        t += 1.0 / 240.0;
        assert_eq!(a.pos, target, "the sweep must never move the caret");
        let head = trail_head_y(&a);
        let sweep = a.trail_sweep_p();
        assert!(head >= prev_head - 1e-3, "head must sweep DOWN old→new: {prev_head}->{head}");
        assert!(sweep >= prev_sweep - 1e-6, "sweep progress must advance: {prev_sweep}->{sweep}");
        prev_head = head;
        prev_sweep = sweep;
    }
    // Sweep complete: the head has arrived on the NEW caret y (full old→new span),
    // and the alpha is still at peak (the fade only begins after the sweep).
    assert!(a.trail_sweep_p() > 0.999, "sweep completes within its window");
    assert!(
        (trail_head_y(&a) - to_y).abs() < 0.5,
        "the streak head arrives at the NEW caret y"
    );
    let full_alpha = a.trail_alpha();
    assert!(
        (full_alpha - CARET_TRAIL_ALPHA).abs() < 1e-3,
        "alpha held at peak through the sweep: {full_alpha}"
    );

    // After the sweep it FADES (alpha drops) while the head stays put on the caret.
    let head_settled = trail_head_y(&a);
    a.step_trail(40.0 / 1000.0);
    assert!(a.trail_alpha() < full_alpha, "after the sweep the trail fades");
    assert_eq!(a.pos, target, "the fade must never move the caret");
    assert!(
        (trail_head_y(&a) - head_settled).abs() < 1e-2,
        "after the sweep the head rests on the caret"
    );
}

#[test]
fn held_down_sweep_is_pinned_full_and_steady() {
    // A held DOWN auto-repeat re-kicks the sweep each step, but a held run PINS the
    // sweep to its full span so the drawn length never strobes mid-draw-on: every
    // step the head is on the NEW caret (sweep == 1) with the caret pinned.
    let mut a = primed_caret();
    let mut y = a.pos.y;
    for _ in 0..8 {
        y += crate::render::LINE_HEIGHT;
        a.set_held(true);
        a.nav_to(200.0, y);
        // Even immediately after the re-kick (sweep_t == 0) the HELD sweep reads 1.0.
        assert!(a.is_trail_held(), "held re-kick must be flagged held");
        assert!(
            (a.trail_sweep_p() - 1.0).abs() < 1e-6,
            "held sweep is pinned to the full span (steady, no strobe)"
        );
        assert_eq!(a.pos, a.target, "held sweep keeps the caret pinned");
        a.step_trail(30.0 / 1000.0);
    }
}

#[test]
fn settle_factor_is_one_at_rest() {
    // At rest exactly on target: settle_factor == 1.0 (full underline).
    let mut a = CaretAnim::new();
    a.set_target(100.0, 200.0); // snaps; pos == target, vel == 0
    assert!(!a.is_animating());
    assert!((a.settle_factor() - 1.0).abs() < 1e-6, "rest must be full underline");
}

#[test]
fn settle_factor_collapses_when_moving_fast() {
    // A caret far from target AND moving fast must collapse toward the dot
    // (settle_factor near 0).
    let mut a = CaretAnim::new();
    a.inject_motion(
        Sample { x: 0.0, y: 0.0 },
        Sample { x: 0.0, y: 300.0 },
        Sample { x: 0.0, y: -1500.0 },
    );
    let s = a.settle_factor();
    assert!(s < 0.05, "fast mid-glide must collapse to a dot, got {s}");
}

#[test]
fn settle_factor_monotone_reforms_as_it_arrives() {
    // As the caret nears the target and decelerates, the settle factor must
    // rise monotonically toward 1.0 over the final stretch of a glide. We
    // sample it at the very end of a glide and assert it is climbing.
    let mut a = CaretAnim::new();
    a.set_target(16.0, 300.0);
    a.set_target(16.0, 20.0);
    let mut last = a.settle_factor();
    let mut climbed_to_full = false;
    let mut min_seen = 1.0f32;
    while a.is_animating() {
        a.step(1.0 / 120.0);
        let s = a.settle_factor();
        min_seen = min_seen.min(s);
        last = s;
    }
    // Mid-glide it dipped low (was a dot)...
    assert!(min_seen < 0.2, "should have collapsed mid-glide, min={min_seen}");
    // ...and by the time it settled it is the full underline.
    if (last - 1.0).abs() < 1e-3 {
        climbed_to_full = true;
    }
    assert!(climbed_to_full, "must re-form to full underline at rest, last={last}");
}

#[test]
fn settle_factor_in_unit_range() {
    // For arbitrary injected states the factor stays within [0,1].
    for (px, py, vx, vy) in [
        (0.0, 0.0, 0.0, 0.0),
        (5.0, 5.0, 100.0, 100.0),
        (200.0, 0.0, -3000.0, 0.0),
        (1.0, 1.0, 10.0, -10.0),
    ] {
        let mut a = CaretAnim::new();
        a.inject_motion(
            Sample { x: 0.0, y: 0.0 },
            Sample { x: px, y: py },
            Sample { x: vx, y: vy },
        );
        let s = a.settle_factor();
        assert!((0.0..=1.0).contains(&s), "settle factor out of [0,1]: {s}");
    }
}

#[test]
fn injected_motion_animates() {
    let mut a = CaretAnim::new();
    a.inject_motion(
        Sample { x: 16.0, y: 16.0 },
        Sample { x: 16.0, y: 120.0 },
        Sample { x: 0.0, y: -300.0 },
    );
    assert!(a.is_animating());
}

#[test]
fn one_glyph_hop_never_overshoots() {
    // A single-character hop (~1 glyph-advance) is near-critically damped, so
    // it must settle WITHOUT overshooting — rapid typing reads as calm.
    let adv = crate::render::CHAR_WIDTH;
    let mut a = CaretAnim::new();
    a.set_glyph_advance(adv);
    a.set_target(0.0, 0.0); // prime / snap
    a.set_target(adv, 0.0); // one-glyph hop to the right
    let mut overshot = false;
    let mut frames = 0;
    while a.is_animating() && frames < 2000 {
        a.step(1.0 / 120.0);
        frames += 1;
        if a.pos.x > adv + 0.5 {
            overshot = true;
        }
    }
    assert!(!overshot, "a one-glyph hop must not overshoot, x={}", a.pos.x);
}

#[test]
fn large_jump_still_overshoots() {
    // A big jump (~42 advances) stays springy and keeps its overshoot.
    let adv = crate::render::CHAR_WIDTH;
    let mut a = CaretAnim::new();
    a.set_glyph_advance(adv);
    a.set_target(0.0, 0.0); // prime / snap
    a.set_target(0.0, 600.0); // 600px jump down
    let mut overshot = false;
    let mut frames = 0;
    while a.is_animating() && frames < 2000 {
        a.step(1.0 / 120.0);
        frames += 1;
        if a.pos.y > 600.0 + 0.5 {
            overshot = true;
        }
    }
    assert!(overshot, "a 600px jump must keep its springy overshoot");
}

#[test]
fn move_damping_monotonic_in_distance() {
    // Damping must be monotonically NON-INCREASING in distance: tiny hops are
    // the most damped, big jumps the springiest.
    let mut a = CaretAnim::new();
    a.set_glyph_advance(crate::render::CHAR_WIDTH);
    let mut prev = a.move_damping(0.0);
    let mut i = 1;
    while i <= 200 {
        let dist = i as f32 * 2.0;
        let d = a.move_damping(dist);
        assert!(
            d <= prev + 1e-4,
            "damping increased with distance: {d} > {prev} at dist={dist}"
        );
        prev = d;
        i += 1;
    }
    // Endpoints land on the documented band.
    assert!(
        (a.move_damping(0.0) - SMALL_MOVE_DAMPING).abs() < 1e-3,
        "tiny move must use SMALL_MOVE_DAMPING"
    );
    let far = crate::render::CHAR_WIDTH * (LARGE_MOVE_ADV + 4.0);
    assert!(
        (a.move_damping(far) - DAMPING).abs() < 1e-3,
        "far move must use springy DAMPING"
    );
}

#[test]
fn damping_zoom_invariant_for_one_glyph_move() {
    // A one-glyph move must yield the SAME damping at any zoom: the glyph
    // advance scales with zoom and so does the pixel distance, so the move
    // measured in advances (and thus the damping) is unchanged.
    let adv1 = crate::render::CHAR_WIDTH;
    let adv2 = crate::render::CHAR_WIDTH * 2.0;
    let mut a1 = CaretAnim::new();
    a1.set_glyph_advance(adv1);
    let mut a2 = CaretAnim::new();
    a2.set_glyph_advance(adv2);
    let d1 = a1.move_damping(adv1); // one glyph at zoom 1
    let d2 = a2.move_damping(adv2); // one glyph at zoom 2
    assert!(
        (d1 - d2).abs() < 1e-4,
        "one-glyph damping must be zoom-invariant: {d1} vs {d2}"
    );
}

#[test]
fn trail_follows_true_vector_and_is_always_centre_anchored() {
    // Representative zoomed metric scalars (exact values don't matter; the
    // geometry is scale-free in what we assert).
    let (block_w, block_h, thin, streak) = (14.0_f32, 22.0_f32, 2.8_f32, 60.0_f32);
    // A non-zero tail gap (≈1.5 chars): the tail pulls in but the head stays on
    // the caret, so every head-glue / anchor assertion below is unchanged.
    let gap = 20.0_f32;
    // The in-motion trail anchors at the TEXT optical centre = `pos.y` + this
    // drop (these injected states are fully in motion, settle ~0 ⇒ motion ~1, so
    // the full drop applies). A few px DOWN from the line-box centre.
    let drop = 3.0_f32;

    // DIAGONAL jump (different ROW and COLUMN, e.g. an isearch hop between two
    // matches): fast velocity along (target - source) at 45°. The trail must be
    // a true slant — BOTH components clearly non-zero AND parallel to the move —
    // not collapsed onto the vertical axis (the old mirror-onto-axis bug).
    let mut d = CaretAnim::new();
    d.set_line_height(crate::render::LINE_HEIGHT);
    d.inject_motion(
        Sample { x: 400.0, y: 400.0 }, // target (down-right)
        Sample { x: 100.0, y: 100.0 }, // pos (source, mid-glide)
        Sample { x: 3000.0, y: 3000.0 }, // fast: settle_factor ~ 0
    );
    let (tail, head) = d.trail_endpoints(block_w, block_h, thin, streak, gap, drop);
    let (tx, ty) = (head.x - tail.x, head.y - tail.y);
    assert!(
        tx.abs() > 1.0 && ty.abs() > 1.0,
        "a diagonal trail must slant on BOTH axes, got ({tx}, {ty})"
    );
    assert!(
        (tx - ty).abs() < 0.05 * tx.abs().max(ty.abs()),
        "trail must run along the true 45° vector, got ({tx}, {ty})"
    );
    // The diagonal trail anchors at the TEXT optical centre: the head (leading
    // edge, glued to the caret in x) sits at `pos.y` + the text-centre drop.
    assert!(
        (head.y - (d.pos.y + drop)).abs() < 1.0,
        "a diagonal trail's head must sit at the text centre {}, got {}",
        d.pos.y + drop,
        head.y
    );

    // VERTICAL jump (down one+ rows, same column): the trail is a straight line
    // through the caret CENTRE — its head (leading) endpoint sits at the centre.
    let mut v = CaretAnim::new();
    v.set_line_height(crate::render::LINE_HEIGHT);
    v.inject_motion(
        Sample { x: 200.0, y: 400.0 }, // target (below)
        Sample { x: 200.0, y: 100.0 }, // pos (source, above)
        Sample { x: 0.0, y: 3000.0 },  // fast down: settle_factor ~ 0
    );
    let (vt, vh) = v.trail_endpoints(block_w, block_h, thin, streak, gap, drop);
    assert!(
        (vt.x - vh.x).abs() < 1e-3,
        "a vertical trail must run straight down one column (shared x)"
    );

    // HORIZONTAL jump: fast +x velocity. The trail is now CENTRE-anchored too —
    // both endpoints share the caret's vertical CENTRE `pos.y` (a centred sweep
    // THROUGH the line centre), NOT dropped below to a baseline underline.
    let mut h = CaretAnim::new();
    h.set_line_height(crate::render::LINE_HEIGHT);
    h.inject_motion(
        Sample { x: 400.0, y: 100.0 },
        Sample { x: 100.0, y: 100.0 },
        Sample { x: 3000.0, y: 0.0 },
    );
    let (ht, hh) = h.trail_endpoints(block_w, block_h, thin, streak, gap, drop);
    assert!(
        (ht.y - hh.y).abs() < 1e-3,
        "a horizontal trail must lie on a single y (a straight sweep)"
    );
    assert!(
        (hh.x - ht.x).abs() > 1.0,
        "a horizontal trail must have length along its axis"
    );
    // TEXT-centre-anchored: both endpoints sit at `pos.y` + the text-centre drop
    // (the x-height middle), NOT dropped all the way to a baseline underline. The
    // small drop runs the centred sweep THROUGH the letters, not above them.
    let center_y = h.pos.y + drop;
    assert!(
        (ht.y - center_y).abs() < 1e-3 && (hh.y - center_y).abs() < 1e-3,
        "a horizontal trail must run through the TEXT centre {} (no baseline drop), got {} / {}",
        center_y,
        ht.y,
        hh.y
    );
}

#[test]
fn streak_tail_inset_from_origin_head_stays_on_caret() {
    // Representative zoomed scalars; the geometry is scale-free in what we assert.
    let (block_w, block_h, thin, streak) =
        (14.0_f32, 22.0_f32, 2.8_f32, 60.0_f32);
    let gap = 20.0_f32;
    // A representative text-centre drop; it only translates the trail, so the
    // gap/head-glue differences below are invariant to it (passed consistently).
    let drop = 3.0_f32;

    // HORIZONTAL move (right -> left, like a delete): the caret travels along -x.
    // Inject a fast, far glide so settle_factor == 0 (fully in motion).
    let mut h = CaretAnim::new();
    h.set_line_height(crate::render::LINE_HEIGHT);
    h.inject_motion(
        Sample { x: 0.0, y: 100.0 },    // target (left)
        Sample { x: 300.0, y: 100.0 },  // pos (caret, mid-glide)
        Sample { x: -3000.0, y: 0.0 },  // fast left: settle_factor ~ 0
    );
    // The HEAD (leading edge, AT the caret) is unchanged by the gap, and sits at
    // the caret's cell-centre x = pos.x + block_w/2 (the caret's leading edge).
    let (h_tail_g, h_head_g) = h.trail_endpoints(block_w, block_h, thin, streak, gap, drop);
    let (h_tail_0, h_head_0) = h.trail_endpoints(block_w, block_h, thin, streak, 0.0, drop);
    let caret_lead = h.pos.x + block_w * 0.5;
    assert!(
        (h_head_g.x - caret_lead).abs() < 1e-3,
        "HEAD must stay glued to the caret leading edge {caret_lead}, got {}",
        h_head_g.x
    );
    // Gap must NOT move the head (no detaching from the caret).
    assert!(
        (h_head_g.x - h_head_0.x).abs() < 1e-3 && (h_head_g.y - h_head_0.y).abs() < 1e-3,
        "the gap must not move the HEAD (it stays on the caret)"
    );
    // The TAIL (origin side) is inset by ~gap ALONG the travel vector: it pulls
    // in TOWARD the head, so the trail length shrinks by exactly the gap (the
    // head is fixed). Direction-agnostic: the tail moves along the line, never off
    // it. Here travel is -x, so the tail (the right/origin end) slides left.
    let h_len_0 = (h_head_0.x - h_tail_0.x).hypot(h_head_0.y - h_tail_0.y);
    let h_len_g = (h_head_g.x - h_tail_g.x).hypot(h_head_g.y - h_tail_g.y);
    assert!(
        (h_len_0 - h_len_g - gap).abs() < 1e-3 && h_len_g < h_len_0,
        "the TAIL must inset toward the head by ~gap ({gap}): len {h_len_0} -> {h_len_g}"
    );
    // The origin-side tail is the RIGHT end (travel is leftward); it moved left.
    assert!(
        (h_tail_g.x - (h_tail_0.x - gap)).abs() < 1e-3,
        "horizontal tail must slide toward the head (left) by the gap"
    );

    // VERTICAL move (down): travel along +y; same head-glue / tail-inset rule.
    let mut v = CaretAnim::new();
    v.set_line_height(crate::render::LINE_HEIGHT);
    v.inject_motion(
        Sample { x: 200.0, y: 400.0 }, // target (below)
        Sample { x: 200.0, y: 100.0 }, // pos (caret)
        Sample { x: 0.0, y: 3000.0 },  // fast down: settle_factor ~ 0
    );
    let (v_tail_g, v_head_g) = v.trail_endpoints(block_w, block_h, thin, streak, gap, drop);
    let (v_tail_0, v_head_0) = v.trail_endpoints(block_w, block_h, thin, streak, 0.0, drop);
    assert!(
        (v_head_g.x - v_head_0.x).abs() < 1e-3 && (v_head_g.y - v_head_0.y).abs() < 1e-3,
        "vertical: the gap must not move the HEAD"
    );
    // Travel is +y (down), so the origin-side tail (the UPPER end) insets DOWN
    // toward the head; the trail length shrinks by exactly the gap.
    let v_len_0 = (v_head_0.x - v_tail_0.x).hypot(v_head_0.y - v_tail_0.y);
    let v_len_g = (v_head_g.x - v_tail_g.x).hypot(v_head_g.y - v_tail_g.y);
    assert!(
        (v_len_0 - v_len_g - gap).abs() < 1e-3 && v_len_g < v_len_0,
        "vertical TAIL must inset toward the head by ~gap ({gap}): len {v_len_0} -> {v_len_g}"
    );
    let dy = v_tail_g.y - v_tail_0.y;
    assert!(
        (dy - gap).abs() < 1e-3 && dy > 0.0,
        "vertical tail (upper/origin end) must slide DOWN toward the head by the gap, moved {dy}"
    );
}

#[test]
fn streak_shorter_than_gap_draws_nothing() {
    let (block_w, block_h, thin) = (14.0_f32, 22.0_f32, 2.8_f32);
    let gap = 20.0_f32;
    // A streak whose full in-motion length is SHORTER than the gap: the gap
    // swallows it, so the clamped length is 0 → no visible streak.
    let short_streak = 8.0_f32;
    let mut a = CaretAnim::new();
    a.set_line_height(crate::render::LINE_HEIGHT);
    a.inject_motion(
        Sample { x: 0.0, y: 100.0 },
        Sample { x: 300.0, y: 100.0 },
        Sample { x: -3000.0, y: 0.0 }, // fully in motion (settle 0)
    );
    let (_c, half_along, _half_across, _axis) =
        a.motion_geometry(block_w, block_h, thin, short_streak, gap, 3.0);
    assert!(
        half_along < 1e-6,
        "a move shorter than the gap must draw NO streak, got half-length {half_along}"
    );
    let (tail, head) = a.trail_endpoints(block_w, block_h, thin, short_streak, gap, 3.0);
    let len = ((head.x - tail.x).powi(2) + (head.y - tail.y).powi(2)).sqrt();
    assert!(len < 1e-6, "zero-length streak expected, got {len}");
}

#[test]
fn is_zip_move_gates_on_distance_not_action() {
    let adv = crate::render::CHAR_WIDTH;
    let lh = crate::render::LINE_HEIGHT;
    let mut a = CaretAnim::new();
    a.set_glyph_advance(adv);
    a.set_line_height(lh);
    a.set_target(100.0, 100.0); // prime / rest
    // Single char (C-f / held right): SMALL.
    assert!(!a.is_zip_move(100.0 + adv, 100.0), "one-char hop is not a zip");
    // A few chars (C-e near the end): still SMALL (< CARET_ZIP_CHARS).
    assert!(
        !a.is_zip_move(100.0 + (CARET_ZIP_CHARS - 1.0) * adv, 100.0),
        "a short C-e (within the gate) snaps"
    );
    // Long C-e across a line: BIG.
    assert!(
        a.is_zip_move(100.0 + (CARET_ZIP_CHARS + 4.0) * adv, 100.0),
        "a long C-e zips"
    );
    // Single line (C-n / held down): SMALL.
    assert!(!a.is_zip_move(100.0, 100.0 + lh), "one-line hop is not a zip");
    // Single line with a big goal-column x clamp: still SMALL (one row).
    assert!(
        !a.is_zip_move(40.0, 100.0 + lh),
        "one-line hop with a small x clamp still snaps"
    );
    // Multi-line / page jump: BIG.
    assert!(a.is_zip_move(100.0, 100.0 + 3.0 * lh), "a page jump zips");
}

#[test]
fn small_nav_move_snaps_instantly_with_no_trail() {
    // A single-char nav hop (incl. held L/R) and a single-line hop must SNAP via
    // nav_to: pos == target immediately, settled, not animating, NO trail.
    let m = crate::render::Metrics::new(1.0);
    let adv = m.char_width;
    let lh = m.line_height;
    let gap = m.caret_streak_gap;

    let mut a = CaretAnim::new();
    a.set_glyph_advance(adv);
    a.set_line_height(lh);
    a.set_target(100.0, 100.0); // prime / rest
    a.nav_to(100.0 + adv, 100.0); // one char right
    assert!(!a.is_animating(), "a small nav move must snap, not animate");
    assert_eq!(a.pos, a.target, "snapped caret sits exactly on target");
    assert!(
        (a.settle_factor() - 1.0).abs() < 1e-6,
        "snapped caret is fully settled (resting shape)"
    );
    assert!(
        gate_streak_len(&a, &m) < gap,
        "a snapped small move draws NO trail past the gap ({gap})"
    );

    // HELD right is the SAME small move — it must snap with no trail too.
    let mut h = CaretAnim::new();
    h.set_glyph_advance(adv);
    h.set_line_height(lh);
    h.set_target(100.0, 100.0); // prime
    h.set_held(true); // OS auto-repeat
    h.nav_to(100.0 + adv, 100.0); // one char right, held
    assert!(!h.is_animating(), "a held one-char hop must snap");
    assert_eq!(h.pos, h.target);
    assert!(!h.is_holding(), "a snapped held hop drops the holding latch");
    assert!(
        gate_streak_len(&h, &m) < gap,
        "held one-char hop draws NO trail (small move snaps)"
    );

    // Single line down (C-n / held down): snaps too.
    let mut v = CaretAnim::new();
    v.set_glyph_advance(adv);
    v.set_line_height(lh);
    v.set_target(100.0, 100.0); // prime
    v.nav_to(100.0, 100.0 + lh); // one line down
    assert!(!v.is_animating(), "a one-line nav move must snap");
    assert_eq!(v.pos, v.target);
}

#[test]
fn big_nav_move_glides_and_trails() {
    // A long horizontal jump (C-e across a long line) must ANIMATE: pos != target
    // right after nav_to, the spring is still travelling, and mid-glide the
    // trailing streak blooms past the gap.
    let m = crate::render::Metrics::new(1.0);
    let adv = m.char_width;
    let lh = m.line_height;
    let gap = m.caret_streak_gap;

    let mut a = CaretAnim::new();
    a.set_glyph_advance(adv);
    a.set_line_height(lh);
    a.set_target(16.0, 100.0); // prime / rest
    let dest_x = 16.0 + 40.0 * adv; // long C-e across a line
    a.nav_to(dest_x, 100.0);
    assert!(a.is_animating(), "a big nav move must glide");
    assert!(
        (a.pos.x - a.target.x).abs() > POS_EPSILON,
        "big-move caret is still travelling, not at target"
    );
    // Mid-glide the streak blooms past the gap (the zip flourish).
    let mut max_streak = 0.0_f32;
    let mut min_s = a.settle_factor();
    let mut frames = 0;
    while a.is_animating() && frames < 2000 {
        a.step(1.0 / 120.0);
        max_streak = max_streak.max(gate_streak_len(&a, &m));
        min_s = min_s.min(a.settle_factor());
        frames += 1;
    }
    assert!(min_s < 0.2, "a big nav move must collapse to the streak, min={min_s}");
    assert!(
        max_streak > gap,
        "a big nav move must draw a trail past the gap ({gap}), max={max_streak}"
    );
}
