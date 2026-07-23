//! Caret geometry, scroll-follow, trail/morph anchoring, zoom clamp, and the
//! copy-pulse decay math -- split out of the former monolithic `render::tests`
//! (2026-07 code-organization pass). See `caret_block` for the BLOCK-caret
//! sizing/ink-box tests specifically.

use super::super::*;
use super::{H, headless_pipeline, view};

#[test]
fn visible_lines_count() {
    assert_eq!(visible_lines(H), 24);
}

#[test]
fn no_scroll_when_cursor_visible() {
    // cursor on line 5, no scroll -> stays 0.
    assert_eq!(clamp_scroll(0, 5, H), 0);
}

#[test]
fn scroll_down_to_follow_cursor() {
    // cursor on line 30 with 24 visible rows -> scroll so line 30 is last
    // visible: scroll = 30 + 1 - 24 = 7.
    assert_eq!(clamp_scroll(0, 30, H), 7);
}

#[test]
fn scroll_up_when_cursor_above_view() {
    // currently scrolled to 10, cursor moves to line 3 -> scroll up to 3.
    assert_eq!(clamp_scroll(10, 3, H), 3);
}

#[test]
fn scroll_unchanged_when_cursor_within_window() {
    // scrolled to 10, cursor at line 20 (10..34 visible) -> unchanged.
    assert_eq!(clamp_scroll(10, 20, H), 10);
}

#[test]
fn metrics_scale_with_zoom() {
    let m1 = Metrics::new(1.0);
    assert_eq!(m1.font_size, FONT_SIZE);
    assert_eq!(m1.line_height, LINE_HEIGHT);
    assert_eq!(m1.char_width, CHAR_WIDTH);

    let m2 = Metrics::new(2.0);
    assert!((m2.font_size - FONT_SIZE * 2.0).abs() < 1e-3);
    assert!((m2.line_height - LINE_HEIGHT * 2.0).abs() < 1e-3);
    assert!((m2.char_width - CHAR_WIDTH * 2.0).abs() < 1e-3);
    assert!((m2.caret_w - CARET_W * 2.0).abs() < 1e-3);
    assert!((m2.caret_h - CARET_H * 2.0).abs() < 1e-3);
    // The caret-shape metrics (resting square height, motion streak thickness,
    // streak length clamps + velocity scale) also scale linearly with zoom.
    assert!((m2.caret_block_h - CARET_BLOCK_H * 2.0).abs() < 1e-3);
    assert!((m2.caret_streak_h - CARET_STREAK_H * 2.0).abs() < 1e-3);
    assert!((m2.caret_streak_min_len - CARET_STREAK_MIN_LEN * 2.0).abs() < 1e-3);
    assert!((m2.caret_streak_max_len - CARET_STREAK_MAX_LEN * 2.0).abs() < 1e-3);
    assert!((m2.caret_streak_vel_full - CARET_STREAK_VEL_FULL * 2.0).abs() < 1e-3);
    assert!(
        (m2.caret_streak_gap - crate::caret::CARET_STREAK_GAP * 2.0).abs() < 1e-3
    );
}

/// The motion morph: the trailing-streak length grows monotonically with the
/// caret's horizontal speed and is clamped to the [min, max] band. This is the
/// "faster ⇒ longer streak" mapping that makes the moving caret read as a
/// velocity-scaled comet trail rather than a fixed bar.
#[test]
fn streak_length_grows_with_speed_and_clamps() {
    let m = Metrics::new(1.0);
    // At rest (speed 0) the streak is at its minimum length...
    assert!((m.streak_len_for_speed(0.0) - CARET_STREAK_MIN_LEN).abs() < 1e-3);
    // ...at the full-length velocity it reaches the maximum...
    assert!((m.streak_len_for_speed(CARET_STREAK_VEL_FULL) - CARET_STREAK_MAX_LEN).abs() < 1e-3);
    // ...and faster than that it stays clamped at the maximum (no runaway).
    assert!((m.streak_len_for_speed(CARET_STREAK_VEL_FULL * 4.0) - CARET_STREAK_MAX_LEN).abs() < 1e-3);
    // Monotonic non-decreasing across the band, and always within [min, max].
    let mut prev = m.streak_len_for_speed(0.0);
    for i in 0..=20 {
        let speed = CARET_STREAK_VEL_FULL * (i as f32) / 10.0; // up to 2x full
        let len = m.streak_len_for_speed(speed);
        assert!(len >= prev - 1e-4, "streak length must be non-decreasing");
        assert!(
            (CARET_STREAK_MIN_LEN..=CARET_STREAK_MAX_LEN).contains(&len),
            "streak length {len} out of band"
        );
        prev = len;
    }
    // The mapping scales with zoom (a 2x zoom doubles both ends of the band).
    let m2 = Metrics::new(2.0);
    assert!((m2.streak_len_for_speed(0.0) - CARET_STREAK_MIN_LEN * 2.0).abs() < 1e-3);
}

#[test]
fn caret_geometry_orients_trail_along_travel_axis() {
    // Caret x/y geometry folds the page globals (wrap width + text_left);
    // hold the page lock so a parallel page write can't move it (page.rs:95-99).
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping caret_geometry_orients_trail_along_travel_axis: no wgpu adapter");
        return;
    };
    let text = "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\neta\ntheta\niota";
    p.set_view(&view(text, 0, 0));

    // The single quad morphs in its OWN frame (w = length along travel, h =
    // thickness across) and is ROTATED onto the travel axis. So in BOTH the
    // horizontal and vertical cases the streak is long-and-thin (w > h); the
    // direction is carried by the returned axis, not by swapping w/h.

    // HORIZONTAL glide: axis ≈ +x, a long thin streak through the TEXT optical
    // centre — `pos.y` dropped by `caret_trail_drop` to the x-height middle (so
    // it runs through the letters, NOT a baseline underline and NOT slightly
    // above the text). Fully in motion here (settle ~0 ⇒ the full drop applies).
    p.inject_motion_demo();
    let (_cx, cy_h, w_h, h_h, _c, ax_h, ay_h) = p.caret_geometry();
    assert!(w_h > h_h, "motion streak must be long-and-thin: w={w_h} h={h_h}");
    assert!(
        ax_h.abs() > 0.9 && ay_h.abs() < 0.1,
        "horizontal trail axis must be ~+x: ({ax_h}, {ay_h})"
    );
    let want_cy = p.caret.pos.y + p.metrics.caret_trail_drop;
    assert!(
        (cy_h - want_cy).abs() < 1e-3,
        "horizontal trail must run through the TEXT centre (pos.y + trail drop): \
         cy={cy_h} want={want_cy} pos.y={} drop={}",
        p.caret.pos.y,
        p.metrics.caret_trail_drop
    );
    assert!(
        h_h < p.metrics.caret_block_h * 0.5,
        "streak must be thin, h={h_h}"
    );

    // VERTICAL glide: axis ≈ +y (the trail points DOWN the lines), still
    // long-and-thin in its own frame.
    p.inject_motion_demo_vertical();
    let (_cx, _cy, w_v, h_v, _c, ax_v, ay_v) = p.caret_geometry();
    assert!(w_v > h_v, "motion streak must be long-and-thin: w={w_v} h={h_v}");
    assert!(
        ay_v.abs() > 0.9 && ax_v.abs() < 0.1,
        "vertical trail axis must be ~+y: ({ax_v}, {ay_v})"
    );
}

/// FIX 3: the BLOCK caret's descender-aware bottom drops ONLY for glyphs whose
/// real rasterized ink dips below the baseline. A non-dipping `a` measures zero
/// descender (block unchanged); a dipping `g` measures a positive depth (block
/// bottom extends to wrap it). Font-correct (read from the swash placement box),
/// not a hardcoded letter list.
#[test]
fn block_descender_extends_only_for_dippers() {
    // The descender reads the caret's ANCHOR cell, which is MODE-KEYED (Morph
    // anchors one char back); pin BLOCK under the caret lock so the anchor is
    // the cursor cell this test addresses.
    let _c = crate::testlock::serial();
    crate::caret::set_mode(CaretMode::Block);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping block_descender_extends_only_for_dippers: no wgpu adapter");
        return;
    };
    let text = "ag"; // col 0 = 'a' (sits on the baseline), col 1 = 'g' (descender)
    p.set_view(&view(text, 0, 0));
    let a = p.cursor_glyph_descender();
    p.set_view(&view(text, 0, 1));
    let g = p.cursor_glyph_descender();
    assert!(a < 1.5, "non-dipping 'a' must have ~zero descender, got {a}");
    assert!(g > 2.0, "dipping 'g' must extend below the baseline, got {g}");
    assert!(g > a + 2.0, "'g' must dip further than 'a': g={g} a={a}");
}

/// FIX 2: the cosmetic | trail anchors on the SAME x the active caret look uses.
/// In Block mode it centres on the cell (offset = half the block width); in I-beam
/// mode it sits on the thin insertion bar (offset = IBEAM_W/2 ≈ the cell's left
/// edge). A vertical trail (constant column) makes the streak's x equal to that
/// anchor, so the two modes' anchor x must differ by exactly the offset gap.
#[test]
fn cosmetic_trail_anchor_is_mode_aware() {
    // The anchor x's fold the page globals (text_left); mutates the process-
    // global caret mode. Hold BOTH shared test locks (page → caret, the
    // suite-wide order) so neither a page write nor a caret-mode test races this.
    let _p = crate::testlock::serial();
    let _g = crate::testlock::serial();
    // Pin a cursor-cell-anchored look BEFORE the set_view latch (the anchor is
    // mode-keyed: Morph would shift the cell one back).
    crate::caret::set_mode(CaretMode::Block);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping cosmetic_trail_anchor_is_mode_aware: no wgpu adapter");
        return;
    };
    let text = "alpha\nbeta\ngamma\ndelta";
    p.set_view(&view(text, 1, 2));
    let (tx, ty) = p.caret_target_xy();
    // A VERTICAL kick (same column, two rows up→down) so the | always shows.
    let from = Sample { x: tx, y: ty - 2.0 * p.metrics.line_height };
    let to = Sample { x: tx, y: ty };

    // The streak draws on over the sweep window, so nudge it past zero length.
    // The trail anchor reads the PER-FRAME latched look (`caret_look`), so a mode
    // switch must be followed by a frame (`set_view`) to take effect — exactly as
    // the live app re-latches every prepared frame. Re-`set_view` at the same
    // position after each `set_mode` so the latch tracks the global under test.
    crate::caret::set_mode(CaretMode::Block);
    p.set_view(&view(text, 1, 2));
    p.caret.kick_trail(from, to, false);
    p.caret.step_trail(0.03);
    let (block_x, ..) = p.caret_trail_geometry().expect("block trail active");

    crate::caret::set_mode(CaretMode::Ibeam);
    p.set_view(&view(text, 1, 2));
    p.caret.kick_trail(from, to, false);
    p.caret.step_trail(0.03);
    let (ibeam_x, ..) = p.caret_trail_geometry().expect("ibeam trail active");

    // Block | sits at the cell centre; I-beam | sits on the bar near pos.x.
    let want_block = tx + p.caret_block_w() * 0.5;
    let want_ibeam = tx + IBEAM_W * p.metrics.zoom * 0.5;
    assert!((block_x - want_block).abs() < 1e-3, "block | centred: {block_x} vs {want_block}");
    assert!((ibeam_x - want_ibeam).abs() < 1e-3, "ibeam | on the bar: {ibeam_x} vs {want_ibeam}");
    assert!(
        block_x > ibeam_x + 1.0,
        "block | must sit right of the i-beam |: block={block_x} ibeam={ibeam_x}"
    );
    crate::caret::set_mode(CaretMode::Block);
}

/// The I-beam caret: at REST a steady thin/tall bar pinned at the insertion
/// point (`pos.x + thin/2`); under motion the comet stretches along the travel
/// axis (width grows + height collapses on a horizontal glide; height grows on
/// a vertical glide). ~90 lines of branchy geometry with no direct test before.
#[test]
fn ibeam_geometry_rest_and_motion() {
    // Caret x geometry folds the page globals; hold the page lock (page.rs:95-99).
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping ibeam_geometry_rest_and_motion: no wgpu adapter");
        return;
    };
    let text = "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\neta\ntheta\niota";
    p.set_view(&view(text, 0, 2));
    p.settle_caret();
    let thin = IBEAM_W * p.metrics.zoom;
    let tall = p.metrics.caret_h * p.cursor_scale();
    // AT REST (settle_factor 1, motion 0): the steady thin/tall insertion bar.
    let (cx, _cy, w, h, _c) = p.caret_ibeam_geometry();
    assert!((w - thin).abs() < 1e-3, "rest width == IBEAM_W*zoom: w={w} thin={thin}");
    assert!((h - tall).abs() < 1e-3, "rest height == caret_h*scale: h={h} tall={tall}");
    assert!(
        (cx - (p.caret.pos.x + thin * 0.5)).abs() < 1e-3,
        "rest cx pins the | on the insertion bar: cx={cx} want={}",
        p.caret.pos.x + thin * 0.5
    );

    // HORIZONTAL motion: the comet width GROWS past the thin bar while the
    // height COLLAPSES from tall toward thin.
    p.inject_motion_demo();
    let (.., w_h, h_h, _) = p.caret_ibeam_geometry();
    assert!(w_h > thin, "horizontal comet width grows: w={w_h} thin={thin}");
    assert!(h_h < tall, "horizontal comet height collapses: h={h_h} tall={tall}");

    // VERTICAL motion: the comet HEIGHT grows past the tall bar; width stays
    // thin. Inject a fast downward glide directly (the height floors at the cell
    // height, so it only visibly grows once the speed-driven streak exceeds it).
    p.cursor_line = 3;
    p.cursor_col = 0;
    p.set_caret_target(false, false);
    let (tx, ty) = p.caret_target_xy();
    let target = Sample { x: tx, y: ty };
    let pos = Sample { x: tx, y: ty - 3.0 * p.metrics.line_height };
    let vel = Sample { x: 0.0, y: 6000.0 };
    p.caret.inject_motion(target, pos, vel);
    let (.., w_v, h_v, _) = p.caret_ibeam_geometry();
    assert!(h_v > tall, "vertical comet height grows: h={h_v} tall={tall}");
    assert!((w_v - thin).abs() < 1e-3, "vertical comet stays thin: w={w_v} thin={thin}");
}

/// The morph caret's SPACE-BAR geometry on a glyphless ANCHOR cell centres the
/// thin bar on the cell MIDPOINT (`pos.x + advance/2`), not pinned to the cell's
/// left edge — the specific bug the function's doc warns about. Under the
/// one-back MORPH anchor the glyphless cell is the SPACE the caret just passed:
/// a cursor at col 2 of `a b` anchors the space at col 1.
#[test]
fn space_bar_caret_centers_on_cell_advance() {
    // Caret x geometry folds the page globals AND the mode-keyed anchor; hold
    // page → caret (the suite-wide order) and pin MORPH (the space bar is a
    // Morph look), restoring Block after.
    let _g = crate::testlock::serial();
    let _cl = crate::testlock::serial();
    crate::caret::set_mode(CaretMode::Morph);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping space_bar_caret_centers_on_cell_advance: no wgpu adapter");
        crate::caret::set_mode(CaretMode::Block);
        return;
    };
    let text = "a b"; // cursor past the space: the ANCHOR (col 1) is the glyphless space cell
    p.set_view(&view(text, 0, 2));
    p.settle_caret();
    assert_eq!(p.caret_anchor_col(), 1, "morph anchors the just-passed space");
    let (cx, _cy, w, _h, _corner) = p.caret_space_bar_geometry();
    let want_cx = p.caret.pos.x + p.caret_target_w() * 0.5;
    assert!(
        (cx - want_cx).abs() < 1e-3,
        "space-bar | centres on the cell midpoint: cx={cx} want={want_cx}"
    );
    assert!(
        (w - CARET_SPACE_BAR_W * p.metrics.zoom).abs() < 1e-3,
        "space-bar width == CARET_SPACE_BAR_W*zoom: w={w}"
    );
    crate::caret::set_mode(CaretMode::Block);
}

/// THE MORPH ANCHOR (the living caret rides the last-typed glyph): MORPH's
/// caret cell is ONE char BACK of the insertion point — typing `abc|` anchors
/// (and silhouettes) the `c` — while BLOCK and I-BEAM keep the cell AFTER the
/// insertion point, unchanged. FALLBACKS: col 0 (a line start, incl. the
/// fresh line after Enter) and an empty line have no previous glyph on the
/// line, so the GEOMETRY anchor stays the cursor cell (its left edge is the
/// insertion x — never the previous line's last char) but the caret INHABITS
/// nothing there: the silhouette key empties (`caret_inhabited_key` — the
/// glyph AHEAD must not light) and the caret degrades to the thin insertion
/// bar.
#[test]
fn morph_caret_anchors_one_char_back_with_line_start_fallback() {
    // Caret x folds the page globals AND the mode-keyed anchor; hold
    // page → caret (the suite-wide order), pin each look explicitly, and
    // restore Block.
    let _g = crate::testlock::serial();
    let _cl = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!(
            "skipping morph_caret_anchors_one_char_back_with_line_start_fallback: no wgpu adapter"
        );
        return;
    };
    let text = "abc\n\nxyz";

    // BLOCK at end-of-line: the caret cell is the (glyphless) cell AFTER 'c'.
    crate::caret::set_mode(CaretMode::Block);
    p.set_view(&view(text, 0, 3));
    assert_eq!(p.caret_anchor_col(), 3, "block anchors the insertion cell");
    let xs = p.line_glyph_xs(0);
    let (bx, by) = p.caret_target_xy();
    assert!(
        (bx - (p.text_left() + xs[3])).abs() < 1e-3,
        "block sits on the end-of-line cell: {bx} vs {}",
        p.text_left() + xs[3]
    );

    // I-BEAM: the same insertion-point anchor as Block (unchanged).
    crate::caret::set_mode(CaretMode::Ibeam);
    p.set_view(&view(text, 0, 3));
    assert_eq!(p.caret_anchor_col(), 3, "ibeam anchors the insertion cell");
    assert!((p.caret_target_xy().0 - bx).abs() < 1e-3, "ibeam x == block x");

    // MORPH at end-of-line: ONE back — the caret inhabits the just-typed 'c',
    // whose glyph is the silhouette mask key.
    crate::caret::set_mode(CaretMode::Morph);
    p.set_view(&view(text, 0, 3));
    assert_eq!(p.caret_anchor_col(), 2, "morph anchors the previous glyph");
    let (mx, my) = p.caret_target_xy();
    assert!(
        (mx - (p.text_left() + xs[2])).abs() < 1e-3,
        "morph sits on the 'c' cell: {mx} vs {}",
        p.text_left() + xs[2]
    );
    assert!((my - by).abs() < 1e-3, "same row: only x steps back");
    assert!(
        p.cursor_glyph_key_at(0, p.caret_anchor_col()).is_some(),
        "the anchored 'c' rasterizes a silhouette"
    );
    assert!(
        p.caret_inhabited_key().is_some(),
        "mid-line the caret INHABITS the anchored glyph (silhouette on)"
    );

    // FALLBACK line start (col 0 of "xyz", the line-after-Enter shape): the
    // GEOMETRY anchor stays the cursor cell — identical to Block, never the
    // previous line's last char — but the caret inhabits NO glyph: the 'x'
    // AHEAD of the cursor must not light, so the silhouette key empties and
    // the caret degrades to the thin insertion bar.
    p.set_view(&view(text, 2, 0));
    assert_eq!(p.caret_anchor_col(), 0, "line start falls back to the cursor cell");
    assert!(
        p.cursor_glyph_key_at(2, 0).is_some(),
        "sanity: the col-0 cell DOES hold a rasterizable 'x' — it is the degrade"
    );
    assert!(
        p.caret_inhabited_key().is_none(),
        "line start inhabits NOTHING (the 'x' ahead stays unlit; bar degrade)"
    );
    let (m0x, m0y) = p.caret_target_xy();
    crate::caret::set_mode(CaretMode::Block);
    p.set_view(&view(text, 2, 0));
    let (b0x, b0y) = p.caret_target_xy();
    assert!(
        (m0x - b0x).abs() < 1e-3 && (m0y - b0y).abs() < 1e-3,
        "line start: morph == block (fallback), morph=({m0x},{m0y}) block=({b0x},{b0y})"
    );
    assert!(
        p.caret_inhabited_key().is_some(),
        "BLOCK at the same col 0 keeps inhabiting the cursor cell (unchanged)"
    );

    // FALLBACK empty line: anchor col 0, glyphless — the insertion-bar path.
    crate::caret::set_mode(CaretMode::Morph);
    p.set_view(&view(text, 1, 0));
    assert_eq!(p.caret_anchor_col(), 0, "empty line falls back to the cursor cell");
    assert!(
        p.cursor_glyph_key_at(1, p.caret_anchor_col()).is_none(),
        "an empty line stays glyphless"
    );
    assert!(
        p.caret_inhabited_key().is_none(),
        "an empty line inhabits nothing (insertion-bar degrade)"
    );

    crate::caret::set_mode(CaretMode::Block);
}

/// The MORPH LINE-START DEGRADE draws the I-BEAM'S bar — same behavior, same
/// code (`ibeam_bar_dims` is the one owner of the bar's constants): at a
/// settled col 0 the morph's `caret_linestart_bar_geometry` and the I-beam's
/// resting `caret_ibeam_geometry` return the IDENTICAL tuple — thin
/// `IBEAM_W*zoom` bar pinned at the insertion x (`pos.x + thin/2`), the full
/// row-scaled `caret_h` tall, centred on the cell-box centre. The melt-to-bar
/// is not a lookalike of the I-beam; it IS the I-beam's bar.
#[test]
fn morph_linestart_bar_is_the_ibeam_rest_bar() {
    // Caret x geometry folds the page globals AND the mode-keyed anchor; hold
    // page → caret (the suite-wide order), pin Morph, restore Block.
    let _g = crate::testlock::serial();
    let _cl = crate::testlock::serial();
    crate::caret::set_mode(CaretMode::Morph);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping morph_linestart_bar_is_the_ibeam_rest_bar: no wgpu adapter");
        crate::caret::set_mode(CaretMode::Block);
        return;
    };
    let text = "abc\n\nxyz";
    p.set_view(&view(text, 0, 0)); // "Iabc": the line-start degrade
    p.settle_caret();

    let (mx, my, mw, mh, mc) = p.caret_linestart_bar_geometry();
    // The I-beam's rest pose at the same settled spring state (its geometry
    // reads only the spring + metrics, not the mode global). Morph's col-0
    // anchor is the cursor cell, so the two share the same spring target too.
    let (ix, iy, iw, ih, ic) = p.caret_ibeam_geometry();
    assert!((mx - ix).abs() < 1e-6, "same insertion x: {mx} vs {ix}");
    assert!((my - iy).abs() < 1e-6, "same centre y: {my} vs {iy}");
    assert!((mw - iw).abs() < 1e-6, "same thin width: {mw} vs {iw}");
    assert!((mh - ih).abs() < 1e-6, "same tall height: {mh} vs {ih}");
    assert!((mc - ic).abs() < 1e-6, "same corner: {mc} vs {ic}");

    // And the shared dims are really the I-beam constants: IBEAM_W across,
    // the full row-scaled glyph cell box tall, pinned at the insertion x.
    let thin = IBEAM_W * p.metrics.zoom;
    assert!((mw - thin).abs() < 1e-3, "bar width == IBEAM_W*zoom: {mw}");
    assert!(
        (mh - p.metrics.caret_h * p.cursor_scale()).abs() < 1e-3,
        "bar height == caret_h*scale: {mh}"
    );
    assert!(
        (mx - (p.caret.pos.x + thin * 0.5)).abs() < 1e-3,
        "bar pinned at the insertion point x: {mx}"
    );
    // An EMPTY line degrades to the same bar form (only the row differs).
    p.set_view(&view(text, 1, 0));
    p.settle_caret();
    let (_ex, _ey, ew, eh, ec) = p.caret_linestart_bar_geometry();
    assert!((ew - mw).abs() < 1e-6 && (eh - mh).abs() < 1e-6 && (ec - mc).abs() < 1e-6);

    crate::caret::set_mode(CaretMode::Block);
}

/// At a SOFT-WRAP boundary the MORPH anchor (col-1) belongs to the PREVIOUS
/// visual row — the row that owns the collapsed wrap-boundary space — so the
/// morph caret rides that row while Block sits on the continuation row below,
/// and the collapsed space's DEGENERATE cell is rescued to a visible default
/// cell (the caret-sliver fix in `col_x_and_advance`).
#[test]
fn morph_anchor_at_wrap_boundary_rides_the_previous_row() {
    // Wrap geometry folds the page globals; the anchor is mode-keyed. Hold
    // page → caret, pin the looks explicitly, restore Block.
    let _g = crate::testlock::serial();
    let _cl = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping morph_anchor_at_wrap_boundary_rides_the_previous_row: no wgpu adapter");
        return;
    };
    let long = "word ".repeat(80); // wraps on the 1200px canvas

    crate::caret::set_mode(CaretMode::Block);
    p.set_view(&view(&long, 0, 0));
    let rows = p.visual_rows(0);
    assert!(rows.len() >= 2, "long line should wrap ({} rows)", rows.len());
    let wrap_col = rows[1].start_col; // the first char of visual row 2
    assert_eq!(
        long.chars().nth(wrap_col - 1),
        Some(' '),
        "the char before the wrap boundary is the collapsed space"
    );
    // BLOCK at the wrap boundary: the cursor cell, on the SECOND visual row.
    p.set_view(&view(&long, 0, wrap_col));
    assert_eq!(p.caret_anchor_col(), wrap_col);
    let (_bx, by) = p.caret_target_xy();

    // MORPH: one back — the collapsed wrap-boundary space, owned by the
    // PREVIOUS visual row (pick_row's half-open span), one row ABOVE Block.
    crate::caret::set_mode(CaretMode::Morph);
    p.set_view(&view(&long, 0, wrap_col));
    assert_eq!(p.caret_anchor_col(), wrap_col - 1);
    let (_mx, my) = p.caret_target_xy();
    assert!(
        my < by - 1.0,
        "morph rides the PREVIOUS visual row at the wrap boundary: morph_y={my} block_y={by}"
    );
    // The collapsed space's degenerate cell is rescued to a visible cell, so
    // the slim-bar fallback there never draws a ~1px sliver.
    assert!(
        p.caret_target_w() >= p.metrics.char_width * 0.5,
        "degenerate wrap-space cell rescued: w={}",
        p.caret_target_w()
    );

    crate::caret::set_mode(CaretMode::Block);
}

/// A FULL-WIDTH CJK previous char keeps its full-width cell as the MORPH
/// anchor: at end-of-line after `日本` the anchor is the `本` cell — the real
/// full-width advance and a real silhouette glyph — where Block keeps the
/// default-width end-of-line cell. col-1 is a CHAR column, so the multi-byte
/// glyph is exactly one column back.
#[test]
fn morph_anchor_cjk_full_width_cell() {
    // Caret x/w fold the page globals; the anchor is mode-keyed. Hold
    // page → caret, pin Morph, restore Block.
    let _g = crate::testlock::serial();
    let _cl = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping morph_anchor_cjk_full_width_cell: no wgpu adapter");
        return;
    };
    crate::caret::set_mode(CaretMode::Morph);
    let text = "日本";
    p.set_view(&view(text, 0, 2)); // cursor at end-of-line, after 本
    assert_eq!(p.caret_anchor_col(), 1, "morph anchors the full-width 本");
    let xs = p.line_glyph_xs(0);
    let full = xs[2] - xs[1]; // 本's real shaped advance
    assert!(
        full > p.metrics.char_width * 1.2,
        "sanity: the CJK glyph shapes full-width (adv={full}, cell={})",
        p.metrics.char_width
    );
    assert!(
        (p.caret_target_w() - full).abs() < 1e-3,
        "the morph cell is the full-width advance: {} vs {full}",
        p.caret_target_w()
    );
    assert!(
        (p.caret_target_xy().0 - (p.text_left() + xs[1])).abs() < 1e-3,
        "the morph caret sits on 本's left edge"
    );
    assert!(
        p.cursor_glyph_key_at(0, 1).is_some(),
        "本 rasterizes a full-width silhouette"
    );
    crate::caret::set_mode(CaretMode::Block);
}

/// The morph FROM/TO cross-fade captures are ANCHOR-CONSISTENT: on a cursor
/// move the "from" key latches the glyph at the OLD anchor (one back of the
/// old cursor), so a glide morphs previously-inhabited → newly-inhabited
/// glyph. In Block the latch keeps reading the old cursor cell (unchanged).
#[test]
fn morph_from_key_latches_the_old_anchor() {
    // The latch is mode-keyed; hold page → caret, pin looks, restore Block.
    let _g = crate::testlock::serial();
    let _cl = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping morph_from_key_latches_the_old_anchor: no wgpu adapter");
        return;
    };
    let text = "abcd";

    crate::caret::set_mode(CaretMode::Morph);
    p.set_view(&view(text, 0, 2)); // the caret inhabits 'b' (anchor col 1)
    let key_b = p.cursor_glyph_key_at(0, 1);
    assert!(key_b.is_some(), "'b' has a glyph");
    p.set_view(&view(text, 0, 4)); // move: now inhabits 'd'; from = the OLD anchor 'b'
    assert_eq!(
        p.caret_from_key, key_b,
        "morph latches the OLD ANCHOR glyph (the previously-inhabited 'b')"
    );

    // LINE-START DEPARTURE: at col 0 the morph caret was the thin insertion
    // BAR — it inhabited NO glyph — so leaving col 0 latches from = None and
    // the newly-inhabited glyph fades in from nothing, never from the
    // un-inhabited 'a' that sat AHEAD of the cursor.
    p.set_view(&view(text, 0, 0)); // land on the line start (the bar)
    p.set_view(&view(text, 0, 2)); // leave it: now inhabits 'b'
    assert_eq!(
        p.caret_from_key, None,
        "leaving a line start fades in from NOTHING (the bar inhabited no glyph)"
    );

    // BLOCK: the latch keeps reading the old CURSOR cell itself (unchanged).
    crate::caret::set_mode(CaretMode::Block);
    p.set_view(&view(text, 0, 2)); // re-latch the Block look
    let key_c = p.cursor_glyph_key_at(0, 2);
    assert!(key_c.is_some(), "'c' has a glyph");
    p.set_view(&view(text, 0, 3));
    assert_eq!(
        p.caret_from_key, key_c,
        "block latches the old cursor cell (unchanged behavior)"
    );
}

/// set_caret_target's SPRING-AIM decision on `is_edit_move` (the one seam all
/// three looks share): EVERY EDIT MOVE SNAPS — cross-row (Enter / paste
/// reflow, the "caret lags on Enter" fix) AND same-row (typing along a line,
/// the "typing slides the caret" fix: attention is already at the insertion
/// point; a glide's job is carrying the eye across DISTANCE, which typing
/// does not have — zero translation frames, pos == target, velocity zeroed).
/// The aliveness under a keystroke stays with the typing-impact / squash
/// juice, which rides ON TOP of the snap (kick sets the spring animating
/// again). NAVIGATION keeps the zip-distance gate: a small move snaps, a big
/// jump glides.
#[test]
fn edit_moves_snap_while_navigation_keeps_the_zip_gate() {
    // Row/col caret targets fold the page wrap globals; hold the page lock.
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping edit_moves_snap_while_navigation_keeps_the_zip_gate: no wgpu adapter");
        return;
    };
    let text = "alpha\nbeta\ngamma\ndelta";

    // CROSS-ROW edit (e.g. Enter / a multi-line paste): snaps instantly.
    p.set_view(&view(text, 0, 0));
    p.settle_caret();
    p.cursor_line = 1;
    p.cursor_col = 0;
    p.set_caret_target(true, false);
    let (pos, target, _sf, animating) = p.caret_snapshot();
    assert!(!animating, "cross-row edit must snap (no glide)");
    assert!(
        (pos.0 - target.0).abs() < 1e-3 && (pos.1 - target.1).abs() < 1e-3,
        "snap leaves pos == target: pos={pos:?} target={target:?}"
    );

    // SAME-ROW edit (typing along a line): snaps too — zero mid-glide
    // displacement, fully settled the same frame the char lands.
    p.set_view(&view(text, 1, 0));
    p.settle_caret();
    p.cursor_col = 3;
    p.set_caret_target(true, false);
    let (pos, target, sf, animating) = p.caret_snapshot();
    assert!(!animating, "same-row edit must snap (no typing slide)");
    assert!(
        (pos.0 - target.0).abs() < 1e-3 && (pos.1 - target.1).abs() < 1e-3,
        "typing leaves pos == target: pos={pos:?} target={target:?}"
    );
    assert!((sf - 1.0).abs() < 1e-6, "typed caret is fully settled (resting shape)");

    // The typing-impact JUICE still rides on top of the snap: the back-kick
    // re-animates the spring (the flinch plays out) around the SAME target.
    p.caret_type_impact();
    let (_pos, target2, _sf, animating) = p.caret_snapshot();
    assert!(animating, "the impact kick re-animates the spring (flinch juice)");
    assert!(
        (target2.0 - target.0).abs() < 1e-6 && (target2.1 - target.1).abs() < 1e-6,
        "the flinch never moves the target — it settles back to the same rest"
    );

    // NAVIGATION: a one-char hop is under the zip gate -> snaps.
    p.set_view(&view(text, 1, 0));
    p.settle_caret();
    p.cursor_col = 1;
    p.set_caret_target(false, false);
    assert!(!p.caret_snapshot().3, "small nav move snaps");

    // NAVIGATION: a multi-row jump is past the gate -> animates.
    p.set_view(&view(text, 0, 0));
    p.settle_caret();
    p.cursor_line = 3;
    p.cursor_col = 4;
    p.set_caret_target(false, false);
    assert!(p.caret_snapshot().3, "large nav move animates");
}

#[test]
fn zoom_clamps_to_range() {
    assert!((clamp_zoom(10.0) - ZOOM_MAX).abs() < 1e-3);
    assert!((clamp_zoom(0.01) - ZOOM_MIN).abs() < 1e-3);
    // rounds to the nearest step
    assert!((clamp_zoom(1.63) - 1.6).abs() < 1e-3);
    assert!((clamp_zoom(1.0) - 1.0).abs() < 1e-3);
}

#[test]
fn copy_pulse_ease_is_a_clamped_smoothstep() {
    // 0 = just kicked (full brighten), 1 = settled (no boost) — exact endpoints.
    assert_eq!(copy_pulse_ease(0.0), 0.0);
    assert_eq!(copy_pulse_ease(1.0), 1.0);
    // Symmetric about the midpoint, like `CaretAnim::pop_scale`'s own smoothstep.
    assert!((copy_pulse_ease(0.5) - 0.5).abs() < 1e-6);
    // Monotonically non-decreasing across the range (no bounce/overshoot).
    let mut prev = copy_pulse_ease(0.0);
    let mut t = 0.0;
    while t <= 1.0 {
        let v = copy_pulse_ease(t);
        assert!(v >= prev - 1e-6, "copy_pulse_ease must not decrease ({t} -> {v} < {prev})");
        prev = v;
        t += 0.05;
    }
    // Out-of-range input clamps first (defensive — callers already clamp
    // `copy_pulse_t`, but the free fn stays total).
    assert_eq!(copy_pulse_ease(-1.0), 0.0);
    assert_eq!(copy_pulse_ease(2.0), 1.0);
}

#[test]
fn copy_pulse_settles_at_construction_then_kicks_and_decays_back() {
    // A freshly-built pipeline (and every headless capture, which never calls
    // `copy_pulse`) sits permanently at the settled fraction (1.0) — the
    // selection quad draws its plain theme tint, byte-identical to before this
    // round existed. Kicking it drops to 0 (full brighten); running the live
    // clock out settles it back to exactly 1.0 (byte-identical to the pre-kick
    // rendering) — the LIVE-ONLY animation's "decays to exactly the pre-copy
    // rendering" contract, exercised without a GPU present/draw.
    let got = pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl copy-pulse test device"),
                ..Default::default()
            })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let p = TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
        Some(p)
    });
    let Some(mut p) = got else {
        eprintln!("skipping copy_pulse_settles_at_construction_then_kicks_and_decays_back: no wgpu adapter");
        return;
    };

    assert_eq!(p.copy_pulse_settle(), 1.0, "a fresh pipeline starts settled");
    // advance() with no pulse ever kicked must never move it off 1.0.
    p.advance(1.0 / 60.0);
    assert_eq!(p.copy_pulse_settle(), 1.0, "advancing with no kick stays settled");

    p.copy_pulse();
    assert_eq!(p.copy_pulse_settle(), 0.0, "the kick starts fully brightened");
    let mut frames = 0;
    while p.advance(1.0 / 120.0) && frames < 10_000 {
        frames += 1;
    }
    assert!(frames > 0, "the pulse must animate for at least one frame");
    assert_eq!(p.copy_pulse_settle(), 1.0, "the pulse decays back to fully settled");
}

// --- ACCESSIBILITY TIER 1: reduce-motion settles every `advance()` seam -----
// instantly to its EXACT final state (same position/color) in ONE step,
// rather than easing over many. `advance()`'s three OR-folded callees
// (`step_caret`, `step_copy_pulse`, `step_caret_preview`) are the whole gate
// surface — see `motion.rs`'s module doc for why a future 4th animator must
// join this same seam.

#[test]
fn reduced_motion_settles_the_caret_spring_in_one_step() {
    let _g = crate::testlock::serial();
    let saved = crate::motion::reduced();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping reduced_motion_settles_the_caret_spring_in_one_step: no wgpu adapter");
        return;
    };
    let text = "one\ntwo\nthree\nfour\nfive\n";
    p.set_view(&view(text, 0, 0));
    p.settle_caret();
    crate::motion::set_reduced(true);
    // A large nav jump that would normally GLIDE (see `edit_moves_snap_while_
    // navigation_keeps_the_zip_gate`'s own "large nav move animates" case).
    p.cursor_line = 3;
    p.cursor_col = 4;
    p.set_caret_target(false, false);
    let (_, target_before, _, _) = p.caret_snapshot();
    // ONE `advance()` call must fully settle it — no glide frames in between.
    let still_animating = p.advance(1.0 / 60.0);
    let (pos, target, sf, animating) = p.caret_snapshot();
    assert!(!still_animating, "advance() reports settled after one reduced-motion step");
    assert!(!animating, "the spring itself is no longer animating");
    assert_eq!(target, target_before, "reduce-motion never changes WHERE the caret lands");
    assert!(
        (pos.0 - target.0).abs() < 1e-3 && (pos.1 - target.1).abs() < 1e-3,
        "pos == target instantly: pos={pos:?} target={target:?}"
    );
    assert!((sf - 1.0).abs() < 1e-6, "fully settled (resting shape), same as a headless capture");
    crate::motion::set_reduced(saved);
}

#[test]
fn reduced_motion_settles_the_copy_pulse_in_one_step() {
    let _g = crate::testlock::serial();
    let saved = crate::motion::reduced();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping reduced_motion_settles_the_copy_pulse_in_one_step: no wgpu adapter");
        return;
    };
    crate::motion::set_reduced(true);
    p.copy_pulse();
    assert_eq!(p.copy_pulse_settle(), 0.0, "the kick still starts fully brightened");
    let still_animating = p.advance(1.0 / 60.0);
    assert!(!still_animating, "advance() reports settled after one reduced-motion step");
    assert_eq!(
        p.copy_pulse_settle(),
        1.0,
        "the selection brighten settles to its EXACT resting tint in one step"
    );
    crate::motion::set_reduced(saved);
}

#[test]
fn reduced_motion_settles_the_caret_style_preview_loop_instantly() {
    let _g = crate::testlock::serial();
    let saved = crate::motion::reduced();
    let Some(mut p) = headless_pipeline() else {
        eprintln!(
            "skipping reduced_motion_settles_the_caret_style_preview_loop_instantly: no wgpu adapter"
        );
        return;
    };
    crate::motion::set_reduced(true);
    // Open the caret-style picker's preview (mirrors `set_view`'s own
    // `caret_preview` wiring) without a full `set_view` reshape.
    p.caret_preview = Some(CaretMode::Block);
    let still_animating = p.advance(1.0 / 60.0);
    assert!(
        !still_animating,
        "the choreographed demo settles instead of looping under reduce-motion"
    );
    assert_eq!(
        p.caret_demo.text(),
        crate::caret::SAMPLE,
        "settle() types the full sample line at once — the SAME settled state a headless capture renders"
    );
    crate::motion::set_reduced(saved);
}

/// ITEM 57 — the caret's per-frame glyph lookup is POSITION-INDEPENDENT: on a
/// document of many IDENTICAL lines, the lookup at the TOP, MIDDLE, and TAIL
/// resolves the SAME glyph, reports the SAME line-local visited work, and lands the
/// SAME within-row baseline offset — even though the number of shaped runs BEFORE
/// the cursor line grows from a handful to the whole document. The old lookups
/// filtered the whole `layout_runs()` stream (cost ∝ position); the target-line-local
/// record reads only the cursor line's own `layout_opt()`, so equal lines do equal
/// work wherever they sit.
#[test]
fn caret_lookup_position_independent() {
    // Block look is the deterministic anchor (cursor column). Pin it under the
    // reentrant serial guard the caret-mode global shares.
    let _g = crate::testlock::serial();
    let _cl = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping caret_lookup_position_independent: no wgpu adapter");
        return;
    };
    let saved = crate::caret::mode();
    crate::caret::set_mode(CaretMode::Block);

    // Many IDENTICAL non-wrapping content lines: one shaped run each, so the prefix
    // before the cursor line grows 1:1 with the line index.
    const N: usize = 200;
    let line = "abcdefghij";
    let text = std::iter::repeat(line).take(N).collect::<Vec<_>>().join("\n");
    let col = 3;

    let sample = |p: &mut TextPipeline, li: usize| {
        p.set_view(&view(&text, li, col));
        let key = p.cursor_glyph_key_at(li, col);
        let glyphs = p.caret_line_glyph_count();
        // Within-row baseline offset: the absolute baseline minus the visual row's
        // top. For identical lines this is a pure font fact, independent of WHERE the
        // row sits in the document — so it exposes any position-dependent baseline bug.
        let off = p.caret_baseline_y() - p.visual_row_top(li, col);
        // The prefix a whole-doc walk would touch (grows with position) — proves the
        // doc really does put a long prefix in front of the tail case (non-vacuous).
        let mut prefix = 0usize;
        for run in p.buffer.layout_runs() {
            if run.line_i > li {
                break;
            }
            prefix += 1;
        }
        (key, glyphs, off, prefix)
    };

    let (k_top, g_top, o_top, pre_top) = sample(&mut p, 1);
    let (k_mid, g_mid, o_mid, pre_mid) = sample(&mut p, N / 2);
    let (k_tail, g_tail, o_tail, pre_tail) = sample(&mut p, N - 1);

    // Non-vacuous: the prefix really grows top → middle → tail.
    assert!(
        pre_top < pre_mid && pre_mid < pre_tail,
        "sanity: the shaped-run prefix must grow with position (top={pre_top}, \
         mid={pre_mid}, tail={pre_tail}) or the test proves nothing"
    );

    // Work ran (never "measured" 0 work) and is CONSTANT across positions.
    assert!(g_top > 0, "the line-local lookup visited real glyph work");
    assert_eq!(
        (g_top, g_mid, g_tail),
        (g_top, g_top, g_top),
        "identical lines do identical line-local work regardless of position \
         (top={g_top}, mid={g_mid}, tail={g_tail})"
    );

    // The resolved glyph is IDENTICAL — same shaped 'd' at col 3, wherever the line
    // sits. (byte-identical to the old whole-doc walk, which resolved the same glyph.)
    assert!(k_top.is_some(), "col 3 of a content line has a glyph");
    assert_eq!(k_top, k_mid, "top and middle resolve the SAME glyph key");
    assert_eq!(k_top, k_tail, "top and tail resolve the SAME glyph key");

    // The within-row baseline offset is IDENTICAL — the baseline reconstruction is a
    // function of the row's own layout, not its document position.
    assert!(
        (o_top - o_mid).abs() < 1e-3 && (o_top - o_tail).abs() < 1e-3,
        "the within-row baseline offset is position-independent \
         (top={o_top}, mid={o_mid}, tail={o_tail})"
    );

    crate::caret::set_mode(saved);
}

/// ITEM 57 GREP-LAW — the caret render module (`src/render/caret.rs`) must NOT walk
/// the whole document's `layout_runs()` stream: the per-frame caret glyph lookups
/// read the cursor line's OWN `layout_opt()` (the target-line-local record) so their
/// cost is independent of the caret's document position. This structurally bans a
/// future consumer from quietly reintroducing the O(prefix) whole-doc walk. (The
/// prefix-run WITNESS legitimately walks `layout_runs()` — it lives in
/// `caretbench.rs`, a bench driver, exempt like the other `--bench-*` harnesses.)
#[test]
fn caret_no_whole_doc_walk_law() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/render/caret.rs");
    let text = std::fs::read_to_string(&path).expect("read caret.rs");
    let mut hits = Vec::new();
    for (i, line) in text.lines().enumerate() {
        // Consider only the CODE before any `//` comment, so the doc comments (which
        // discuss `layout_runs()` by name) don't trip the ban.
        let code = line.split("//").next().unwrap_or("");
        if code.contains("layout_runs") {
            hits.push((i + 1, line.trim().to_string()));
        }
    }
    assert!(
        hits.is_empty(),
        "src/render/caret.rs must not call `layout_runs()` — the caret glyph lookups \
         read the cursor line's own `layout_opt()` (target-line-local, item 57). \
         Offending lines:\n{}",
        hits.iter()
            .map(|(l, s)| format!("  caret.rs:{l}: {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
