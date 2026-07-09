//! The caret-feedback IMPACT/juice triggers -- kick, recoil, type-impact
//! squash, delete-squash, gulp, line-land, and the copy-pulse (deliberately
//! not velocity-damped) -- split out of the former monolithic
//! `caret::tests` (2026-07 code-organization pass).

use super::super::*;

#[test]
fn kick_adds_signed_velocity_and_animates() {
    let mut a = CaretAnim::new();
    a.set_target(100.0, 50.0); // prime / snap (vel 0, not animating)
    assert!(!a.is_animating());
    a.kick(220.0, 0.0); // InsertChar: recoil right
    assert!(a.is_animating(), "a kick must re-arm the spring");
    assert_eq!(a.vel.x, 220.0);
    a.kick(-220.0, 0.0); // additive: a left flinch cancels it
    assert!((a.vel.x).abs() < 1e-6, "kicks are additive on velocity");
    a.kick(0.0, 300.0); // Newline: a downward drop
    assert_eq!(a.vel.y, 300.0);
}

#[test]
fn recoil_kicks_the_impulse_in_the_named_direction_then_settles() {
    // Each RecoilDir injects CARET_RECOIL_IMPULSE along its axis (y grows DOWN),
    // re-arms the spring, and — being a pure velocity kick — leaves `pos`/`target`
    // untouched so the spring decays back to the SAME resting caret.
    for (dir, ex, ey) in [
        (RecoilDir::Left, -CARET_RECOIL_IMPULSE, 0.0),
        (RecoilDir::Right, CARET_RECOIL_IMPULSE, 0.0),
        (RecoilDir::Up, 0.0, -CARET_RECOIL_IMPULSE),
        (RecoilDir::Down, 0.0, CARET_RECOIL_IMPULSE),
    ] {
        let mut a = CaretAnim::new();
        a.set_target(100.0, 50.0); // prime / rest (vel 0, not animating)
        assert!(!a.is_animating());
        a.recoil(dir);
        assert!(a.is_animating(), "a recoil must re-arm the spring");
        assert_eq!((a.vel.x, a.vel.y), (ex, ey), "{dir:?} impulse vector");
        assert_eq!(a.pos, a.target, "recoil never moves the logical target");
        // Run the spring out: it must settle back exactly on target (byte-identical
        // resting caret), proving a settled capture is unaffected.
        for _ in 0..600 {
            a.step(1.0 / 120.0);
        }
        assert!(!a.is_animating(), "the recoil decays to rest");
        assert_eq!(a.pos, a.target, "settled caret is back on target");
    }
}

#[test]
fn type_impact_squashes_and_back_kicks_then_settles() {
    // A DELIBERATE typed char (caret at rest): the cosmetic pop squashes to
    // CARET_TYPE_IMPACT_SCALE AND a velocity BACK-KICK fires AGAINST the forward
    // type direction (leftward, -x) — the outward flinch — while the logical
    // target is untouched, so the spring decays back to the SAME resting caret.
    let mut a = CaretAnim::new();
    a.set_target(100.0, 50.0); // prime / rest (vel 0, scale 1.0, not animating)
    assert!((a.pop_scale() - 1.0).abs() < 1e-6);
    a.type_impact();
    assert!(
        (a.pop_scale() - CARET_TYPE_IMPACT_SCALE).abs() < 1e-6,
        "a deliberate keystroke squashes to the full impact floor"
    );
    assert!(a.vel.x < -1.0, "the back-kick recoils against forward typing (−x)");
    assert_eq!(a.vel.y, 0.0, "typing impact is horizontal only");
    assert_eq!(a.pos, a.target, "impact rides the VISUAL caret; target untouched");
    // Run the live clock out: the spring AND the pop both settle back to rest.
    for _ in 0..600 {
        a.step(1.0 / 120.0);
        a.step_pop(1.0 / 120.0);
    }
    assert!(!a.is_animating(), "the back-kick decays to rest");
    assert_eq!(a.pos, a.target, "settled caret is back on target (byte-identical)");
    assert!((a.pop_scale() - 1.0).abs() < 1e-6, "the squash-pop settles to scale 1.0");
}

#[test]
fn delete_squash_is_inward_only_no_velocity() {
    // A backspace / C-d INWARD squash: a PURE scale collapse (to
    // CARET_DELETE_SQUASH) with NO velocity kick — the opposite of typing's
    // outward flinch. The logical target is untouched.
    let mut a = CaretAnim::new();
    a.set_target(100.0, 50.0);
    a.delete_squash();
    assert!(
        (a.pop_scale() - CARET_DELETE_SQUASH).abs() < 1e-6,
        "delete squashes to its floor"
    );
    assert_eq!((a.vel.x, a.vel.y), (0.0, 0.0), "deletion is a pure squash, no kick");
    assert_eq!(a.pos, a.target, "squash never moves the caret position");
}

#[test]
fn gulp_is_a_deeper_longer_pulse_than_a_char_delete() {
    // Kill-line GULP: a deeper squash (past the single-char delete) over the
    // longer CARET_GULP_MS — a bigger, satisfying swallow.
    assert!(
        CARET_GULP_SCALE < CARET_DELETE_SQUASH,
        "the gulp must dip deeper than a single-char delete squash"
    );
    assert!(CARET_GULP_MS > CARET_POP_MS, "the gulp must run longer than the snappy pop");

    let mut a = CaretAnim::new();
    a.set_target(100.0, 50.0);
    a.gulp();
    assert!((a.pop_scale() - CARET_GULP_SCALE).abs() < 1e-6, "gulp squashes to its floor");
    assert_eq!((a.vel.x, a.vel.y), (0.0, 0.0), "a gulp is a pure scale pulse, no kick");
    // It settles back to rest like every flinch (byte-identical settled capture).
    let mut frames = 0;
    while a.step_pop(1.0 / 120.0) && frames < 1000 {
        frames += 1;
    }
    assert!((a.pop_scale() - 1.0).abs() < 1e-6, "the gulp settles to scale 1.0");
}

#[test]
fn line_land_is_a_pure_squash_no_velocity_kick() {
    // ENTER JUICE — LINE LANDING (PHASE 3): a caret-level touchdown squash as
    // Enter takes the new line — a PURE scale collapse (to
    // CARET_LINE_LAND_SCALE) with NO velocity kick: a kick on this axis would
    // re-displace the caret off the new line for a few frames, reintroducing
    // the caret-lags-on-Enter lag `jump_to` was built to remove.
    let mut a = CaretAnim::new();
    a.set_target(100.0, 50.0);
    a.line_land();
    assert!(
        (a.pop_scale() - CARET_LINE_LAND_SCALE).abs() < 1e-6,
        "line-land squashes to its floor"
    );
    assert_eq!((a.vel.x, a.vel.y), (0.0, 0.0), "line-land is a pure squash, no kick");
    assert_eq!(a.pos, a.target, "squash never moves the caret position");
    // It settles back to rest like every flinch (byte-identical settled capture).
    let mut frames = 0;
    while a.step_pop(1.0 / 120.0) && frames < 1000 {
        frames += 1;
    }
    assert!((a.pop_scale() - 1.0).abs() < 1e-6, "line-land settles to scale 1.0");
}

#[test]
fn copy_pulse_is_the_gentlest_pure_squash_no_velocity_kick() {
    // COPY PULSE: a successful M-w/Cmd-C copy gives the caret a gentle
    // confirmation kick — a PURE scale collapse (to `CARET_COPY_PULSE_SCALE`)
    // with NO velocity kick, exactly like line-land/gulp/delete. "Obvious and
    // understated": it must be the GENTLEST floor of the whole set (closest to
    // 1.0), since nothing was actually edited.
    assert!(
        CARET_COPY_PULSE_SCALE > CARET_DELETE_SQUASH
            && CARET_COPY_PULSE_SCALE > CARET_GULP_SCALE
            && CARET_COPY_PULSE_SCALE > CARET_LINE_LAND_SCALE
            && CARET_COPY_PULSE_SCALE > CARET_TYPE_IMPACT_SCALE
            && CARET_COPY_PULSE_SCALE > CARET_POP_SCALE,
        "the copy pulse must read gentler than every other flinch/bounce"
    );
    assert!(CARET_COPY_PULSE_SCALE < 1.0, "it must still be a visible dip");

    let mut a = CaretAnim::new();
    a.set_target(100.0, 50.0);
    a.copy_pulse();
    assert!(
        (a.pop_scale() - CARET_COPY_PULSE_SCALE).abs() < 1e-6,
        "copy pulse squashes to its (gentle) floor"
    );
    assert_eq!((a.vel.x, a.vel.y), (0.0, 0.0), "copy pulse is a pure squash, no kick");
    assert_eq!(a.pos, a.target, "squash never moves the caret position");
    // It settles back to rest like every flinch (byte-identical settled capture).
    let mut frames = 0;
    while a.step_pop(1.0 / 120.0) && frames < 1000 {
        frames += 1;
    }
    assert!((a.pop_scale() - 1.0).abs() < 1e-6, "copy pulse settles to scale 1.0");
}

#[test]
fn edit_flinch_is_velocity_damped_in_a_fast_burst() {
    // The KEY anti-strobe rule: a flinch is scaled by the caret's CURRENT spring
    // speed. A DELIBERATE keystroke (caret at rest) lands the FULL thunk; a fast
    // BURST (the spring already racing ≥ CARET_TYPE_IMPACT_DAMP_VEL from the prior
    // keystroke) is SUPPRESSED — the squash flattens to ~1.0 and the back-kick to
    // ~0, so the caret smooths into a slide instead of strobing.

    // Deliberate: at rest, full impact.
    let mut rest = CaretAnim::new();
    rest.set_target(100.0, 50.0);
    rest.type_impact();
    let full_kick = rest.vel.x;
    assert!((rest.pop_scale() - CARET_TYPE_IMPACT_SCALE).abs() < 1e-6, "rest = full squash");
    assert!(full_kick < -1.0, "rest = full back-kick");

    // Burst: the spring is already racing past the damp threshold. The flinch is
    // suppressed — the floor is ~1.0 (no squash) and the added velocity is ~0.
    let mut burst = CaretAnim::new();
    burst.set_target(100.0, 50.0);
    burst.kick(CARET_TYPE_IMPACT_DAMP_VEL + 50.0, 0.0); // race the spring
    let vel_before = burst.vel.x;
    burst.type_impact();
    assert!(
        (burst.pop_scale() - 1.0).abs() < 1e-3,
        "a fast burst must NOT squash (no strobe): {}",
        burst.pop_scale()
    );
    assert!(
        (burst.vel.x - vel_before).abs() < 1e-3,
        "a fast burst must add ~no back-kick velocity (smooth slide)"
    );

    // A delete in a burst is likewise suppressed (held backspace never strobes).
    let mut held = CaretAnim::new();
    held.set_target(100.0, 50.0);
    held.kick(-(CARET_TYPE_IMPACT_DAMP_VEL + 50.0), 0.0);
    held.delete_squash();
    assert!(
        (held.pop_scale() - 1.0).abs() < 1e-3,
        "held backspace must not squash-strobe"
    );

    // A held-Enter burst is likewise suppressed (mashed Enter never strobes).
    let mut held_enter = CaretAnim::new();
    held_enter.set_target(100.0, 50.0);
    held_enter.kick(CARET_TYPE_IMPACT_DAMP_VEL + 50.0, 0.0);
    held_enter.line_land();
    assert!(
        (held_enter.pop_scale() - 1.0).abs() < 1e-3,
        "held Enter must not squash-strobe"
    );
}

#[test]
fn copy_pulse_is_deliberately_not_velocity_damped() {
    // Unlike every edit flinch above, `copy_pulse` does NOT read `impact_damp`:
    // copy is a one-shot, deliberate action rather than a fast-repeat one (you
    // can't "hold down copy"), so a plain kick reads calmer here than a damped
    // one would. Even with the spring already racing past the edit-flinch damp
    // threshold, the pulse still squashes to its full floor.
    let mut burst = CaretAnim::new();
    burst.set_target(100.0, 50.0);
    burst.kick(CARET_TYPE_IMPACT_DAMP_VEL + 50.0, 0.0); // race the spring
    burst.copy_pulse();
    assert!(
        (burst.pop_scale() - CARET_COPY_PULSE_SCALE).abs() < 1e-6,
        "copy pulse squashes to its FULL floor even mid-glide (not velocity-damped)"
    );
}

#[test]
fn edit_reflow_move_snaps_while_navigation_glides() {
    let adv = crate::render::CHAR_WIDTH;
    let lh = crate::render::LINE_HEIGHT;

    // EDIT that crosses a row (Enter at a line start): the edit-apply path snaps
    // via jump_to, so the caret is AT the new line INSTANTLY — pos == target,
    // settled, not animating, full resting shape (no lag of the insertion point).
    let mut e = CaretAnim::new();
    e.set_glyph_advance(adv);
    e.set_line_height(lh);
    e.set_target(16.0, 100.0); // prime / rest
    assert!(e.crosses_row(100.0 + lh), "down-one-line is a row crossing");
    e.jump_to(16.0, 100.0 + lh); // edit-driven reflow ⇒ snap
    assert!(!e.is_animating(), "an edit reflow must snap, not animate");
    assert_eq!(e.pos, e.target, "snapped caret sits exactly on target");
    assert!(
        (e.settle_factor() - 1.0).abs() < 1e-6,
        "snapped caret is fully settled (resting shape)"
    );

    // NAVIGATION of the SAME distance (down-arrow one line): still mid-glide —
    // the spring keeps its personality on a motion move.
    let mut n = CaretAnim::new();
    n.set_glyph_advance(adv);
    n.set_line_height(lh);
    n.set_target(16.0, 100.0); // prime / rest
    n.set_target(16.0, 100.0 + lh); // navigation down one line
    assert!(n.is_animating(), "a navigation move must glide");
    assert!(
        (n.pos.y - n.target.y).abs() > POS_EPSILON,
        "navigation caret is still travelling, not at target"
    );
}
