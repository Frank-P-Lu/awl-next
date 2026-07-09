//! UNIT TESTS for the `TextPipeline` GPU aggregation root, kept as one
//! `render::tests` module (relocated VERBATIM out of `render.rs` to keep the root
//! a focused pipeline + method module). `use super::*` resolves to the `render`
//! root, so the child module's access to its ancestor's private items is
//! unchanged — the same 727-green suite, byte-for-byte.

    use super::*;

    // 800px tall, TEXT_TOP 16, LINE_HEIGHT 32 -> floor((800-16)/32) = 24 rows.
    const H: f32 = 800.0;

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

    // --- Zoom metric scaling ----------------------------------------------

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
        let _g = crate::page::test_lock();
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
        let _c = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _p = crate::page::test_lock();
        let _g = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        crate::caret::set_mode(CaretMode::Block);
        p.caret.kick_trail(from, to, false);
        p.caret.step_trail(0.03);
        let (block_x, ..) = p.caret_trail_geometry().expect("block trail active");

        crate::caret::set_mode(CaretMode::Ibeam);
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
        let _g = crate::page::test_lock();
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
        let _g = crate::page::test_lock();
        let _cl = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::test_lock();
        let _cl = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::test_lock();
        let _cl = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::test_lock();
        let _cl = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::test_lock();
        let _cl = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::test_lock();
        let _cl = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::test_lock();
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

    // --- COPY PULSE: the pure decay math -----------------------------------

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

    // --- PAGE MODE centered-column geometry -------------------------------

    #[test]
    fn page_off_is_edge_to_edge() {
        // Page mode off: left is the fixed NONPAGE_INSET origin and width spans the
        // window minus both plain side insets.
        let cw = CHAR_WIDTH;
        assert_eq!(column_left_for(1200.0, cw, false, 80), NONPAGE_INSET);
        assert!((column_width_for(1200.0, cw, false, 80) - (1200.0 - 2.0 * NONPAGE_INSET)).abs() < 1e-3);
    }

    #[test]
    fn page_on_centers_capped_column() {
        // Wide window, narrow measure: the column caps at measure*char_width and
        // is centered, so left == (window - width)/2 and margins are symmetric.
        let cw = CHAR_WIDTH; // 14.4
        let w = column_width_for(1200.0, cw, true, 40);
        assert!((w - 40.0 * cw).abs() < 1e-3, "width should be measure*advance, got {w}");
        let left = column_left_for(1200.0, cw, true, 40);
        assert!((left - (1200.0 - w) * 0.5).abs() < 1e-3, "column must be centered, left={left}");
        // Symmetric margins: right margin == left margin.
        let right_margin = 1200.0 - (left + w);
        assert!((right_margin - left).abs() < 1e-3, "margins must match: l={left} r={right_margin}");
    }

    #[test]
    fn page_on_narrow_window_fills_minus_small_pad() {
        // Window narrower than the 80-char measure: the RESPONSIVE column fills the
        // width minus only the SMALL uniform pad (PAGE_MIN_PAD) on each side — the
        // generous margin collapses, so the text runs effectively edge-to-edge
        // instead of being strangled into a sliver. Never overflows, stays centered.
        let cw = CHAR_WIDTH;
        let narrow = 400.0;
        let w = column_width_for(narrow, cw, true, 80);
        let left = column_left_for(narrow, cw, true, 80);
        let right = narrow - (left + w);
        // Fills the width minus the small pad on each side (margins collapse to ~0).
        assert!((w - (narrow - 2.0 * PAGE_MIN_PAD)).abs() < 1e-3, "narrow column must fill minus pad: w={w}");
        assert!(w <= narrow - 2.0 * PAGE_MIN_PAD + 1e-3, "must not overflow: w={w}");
        assert!((left - PAGE_MIN_PAD).abs() < 1e-3, "left collapses to the small pad, got {left}");
        assert!((left - right).abs() < 1e-3, "margins must stay symmetric: l={left} r={right}");
    }

    #[test]
    fn page_on_near_full_measure_binds_at_measure() {
        // At the 1200px capture width the 80-char measure (≈1152px) very nearly fills
        // the window: the responsive margin collapses from the generous band to the
        // small leftover, the column sits at its TARGET MEASURE (1152, not capped down
        // to 960), and the ~24px leftover splits symmetrically as the margin.
        let cw = CHAR_WIDTH; // 14.4 -> measure_px 1152
        let win = 1200.0;
        let measure_px = 80.0 * cw; // 1152
        let w = column_width_for(win, cw, true, 80);
        let left = column_left_for(win, cw, true, 80);
        let right = win - (left + w);
        assert!((w - measure_px).abs() < 1e-3, "column must sit at the measure, got {w}");
        assert!((left - right).abs() < 1e-3, "margins must be symmetric: l={left} r={right}");
        assert!((left - (win - measure_px) * 0.5).abs() < 1e-3, "leftover splits as the margin, left={left}");
        // The leftover margin is the small ~24px, well under the old generous 120px.
        assert!(left >= PAGE_MIN_PAD - 1e-3 && left < page_min_margin(win), "margin collapsed to the leftover: {left}");
    }

    #[test]
    fn page_column_proportion_is_dpi_invariant() {
        // The live window width arrives in PHYSICAL pixels and the glyph advance now
        // scales by the SAME display DPI (`Metrics::with_dpi`), so a page column whose
        // MEASURE binds (column == measure*advance) keeps the same FRACTION of the
        // window — centered, symmetric, each margin >= the small PAGE_MIN_PAD floor —
        // at any monitor scale. Before the DPI fold the advance stayed at its 1:1 size
        // while the window doubled, so the column filled only ~1/dpi of the screen.
        // A 40-char measure binds across all the widths below, so the proportion is
        // exact (in the fill regime a fixed-pixel pad would make it dpi-dependent).
        for &logical_w in &[900.0_f32, 1200.0, 1600.0] {
            for &zoom in &[1.0_f32, 1.18, 1.5] {
                let cw1 = Metrics::with_dpi(zoom, 1.0).char_width;
                let frac1 = column_width_for(logical_w, cw1, true, 40) / logical_w;
                for &dpi in &[1.0_f32, 2.0, 2.5] {
                    let phys_w = logical_w * dpi;
                    let cw = Metrics::with_dpi(zoom, dpi).char_width;
                    let w = column_width_for(phys_w, cw, true, 40);
                    let left = column_left_for(phys_w, cw, true, 40);
                    let right = phys_w - (left + w);
                    assert!((left - right).abs() < 1e-2, "asymmetric margins l={left} r={right}");
                    assert!(
                        (left - (phys_w - w) * 0.5).abs() < 1e-2,
                        "column must be centered, left={left}"
                    );
                    assert!(left >= PAGE_MIN_PAD - 1e-2, "left {left} < PAGE_MIN_PAD");
                    let frac = w / phys_w;
                    assert!(
                        (frac - frac1).abs() < 1e-3,
                        "proportion drifted with dpi: {frac} vs {frac1} (w={logical_w} zoom={zoom} dpi={dpi})"
                    );
                }
            }
        }
    }

    // --- Mouse hit-testing round trips ------------------------------------

    #[test]
    fn hit_test_top_left_is_origin() {
        let m = Metrics::new(1.0);
        // A click in the first cell maps to (line 0, col 0).
        assert_eq!(hit_test(TEXT_LEFT + 1.0, TEXT_TOP + 1.0, 0, &m, TEXT_LEFT), (0, 0));
    }

    #[test]
    fn hit_test_roundtrips_cell_centers() {
        // Click inside the LEFT portion of each glyph cell (col + 0.25, clearly
        // within the glyph, away from the rounding boundary at +0.5) and confirm
        // we recover that col, at zoom 1.0 and 1.6, with and without scroll.
        // round() snaps a click past the half-glyph to the next gap (correct
        // caret placement), which the +0.25 offset deliberately avoids.
        for zoom in [1.0f32, 1.6] {
            let m = Metrics::new(zoom);
            for scroll in [0usize, 5] {
                for line in 0..4usize {
                    for col in 0..8usize {
                        let px = TEXT_LEFT + (col as f32 + 0.25) * m.char_width;
                        let py = TEXT_TOP + ((line as f32) + 0.5) * m.line_height;
                        let (hl, hc) = hit_test(px, py, scroll, &m, TEXT_LEFT);
                        assert_eq!(hl, scroll + line, "line z={zoom} s={scroll}");
                        assert_eq!(hc, col, "col z={zoom} s={scroll} line={line}");
                    }
                }
            }
        }
    }

    #[test]
    fn hit_test_rounds_to_nearest_gap() {
        let m = Metrics::new(1.0);
        // Just past the right edge of col 0's glyph (>0.5 width) snaps to col 1.
        let px = TEXT_LEFT + 0.6 * m.char_width;
        assert_eq!(hit_test(px, TEXT_TOP + 1.0, 0, &m, TEXT_LEFT).1, 1);
        // Just inside the left part snaps to col 0.
        let px = TEXT_LEFT + 0.4 * m.char_width;
        assert_eq!(hit_test(px, TEXT_TOP + 1.0, 0, &m, TEXT_LEFT).1, 0);
    }

    #[test]
    fn hit_test_above_text_clamps_to_first_visible() {
        let m = Metrics::new(1.0);
        // Click in the top margin (py < TEXT_TOP) clamps to the first visible
        // line (= scroll) and col 0.
        assert_eq!(hit_test(0.0, 0.0, 7, &m, TEXT_LEFT), (7, 0));
    }

    // --- Free-scroll clamping ---------------------------------------------

    // --- Advance-aware glyph-x assembly (char<->byte + real advances) ------

    #[test]
    fn assemble_xs_latin_uses_real_advances() {
        // "ab": two 1-byte chars, each advance 14.4 (mono). Clusters carry BYTE
        // ranges; xs must be the per-char boundaries plus the end.
        let clusters = [(0usize, 1usize, 0.0f32, 14.4f32), (1, 2, 14.4, 28.8)];
        let xs = assemble_glyph_xs("ab", &clusters, CHAR_WIDTH);
        assert_eq!(xs.len(), 3);
        assert!((xs[0] - 0.0).abs() < 1e-3);
        assert!((xs[1] - 14.4).abs() < 1e-3);
        assert!((xs[2] - 28.8).abs() < 1e-3, "end-of-line = right of last glyph");
    }

    #[test]
    fn assemble_xs_cjk_full_width_and_byte_mapping() {
        // "日本" : two 3-byte kanji, each full-width advance 24.0. The cluster
        // byte ranges are 0..3 and 3..6, but the CHAR columns must be 0,1,2 — this
        // is the critical char<->byte mapping for multi-byte CJK.
        let clusters = [(0usize, 3usize, 0.0f32, 24.0f32), (3, 6, 24.0, 48.0)];
        let xs = assemble_glyph_xs("日本", &clusters, CHAR_WIDTH);
        assert_eq!(xs.len(), 3, "2 chars -> 3 boundaries");
        assert!((xs[0] - 0.0).abs() < 1e-3);
        assert!((xs[1] - 24.0).abs() < 1e-3, "second char starts at full-width offset");
        assert!((xs[2] - 48.0).abs() < 1e-3);
        // The advance of char 0 is the full-width cell, not CHAR_WIDTH.
        assert!((xs[1] - xs[0] - 24.0).abs() < 1e-3);
    }

    #[test]
    fn assemble_xs_mixed_latin_then_cjk() {
        // "a日": 'a' (1 byte, adv 14.4) then '日' (bytes 1..4, full-width 24.0).
        let clusters = [(0usize, 1usize, 0.0f32, 14.4f32), (1, 4, 14.4, 38.4)];
        let xs = assemble_glyph_xs("a日", &clusters, CHAR_WIDTH);
        assert_eq!(xs.len(), 3);
        assert!((xs[1] - 14.4).abs() < 1e-3, "CJK starts after the Latin glyph");
        assert!((xs[2] - 38.4).abs() < 1e-3, "end after full-width CJK");
    }

    #[test]
    fn assemble_xs_empty_line_falls_back_to_char_width() {
        // No clusters: a single end boundary at 0 (caret sits at line start).
        let xs = assemble_glyph_xs("", &[], CHAR_WIDTH);
        assert_eq!(xs, vec![0.0]);
    }

    #[test]
    fn assemble_xs_texture_healed_ligature_splits_at_the_interior() {
        // THE MONASPACE TEXTURE-HEALING CASE (M = N): "=>" shapes to TWO glyphs
        // that BOTH carry the same cluster span (bytes 0..2), each advancing one
        // cell W. The combined cluster advance is 2W, so the interior boundary
        // (between '=' and '>') must land at exactly x = W and the end at 2W —
        // NOT the half-pitch (W/2, W) the old "first glyph's advance wins" logic
        // produced. This is the caret/selection/hit-test correctness fix.
        let w = 14.4f32;
        let clusters = [(0usize, 2usize, 0.0f32, w), (0, 2, w, 2.0 * w)];
        let xs = assemble_glyph_xs("=>", &clusters, CHAR_WIDTH);
        assert_eq!(xs.len(), 3, "2 chars -> 3 boundaries");
        assert!((xs[0] - 0.0).abs() < 1e-3, "first char at 0");
        assert!((xs[1] - w).abs() < 1e-3, "interior split at the FULL first cell, not half");
        assert!((xs[2] - 2.0 * w).abs() < 1e-3, "end at the combined advance");
        // The line is UNIFORM PITCH: both per-char deltas equal W (maxdev ~0).
        assert!((xs[1] - xs[0] - w).abs() < 1e-3 && (xs[2] - xs[1] - w).abs() < 1e-3);
    }

    #[test]
    fn assemble_xs_three_char_shared_cluster_splits_into_even_thirds() {
        // GENERAL N-char / M-glyph shared cluster (M = N = 3, e.g. a "::>"-style
        // texture-healed operator run): three glyphs all stamped with the span
        // bytes 0..3, each advancing W. The combined advance 3W distributes into
        // three EVEN columns at 0, W, 2W with the end at 3W.
        let w = 12.0f32;
        let clusters = [
            (0usize, 3usize, 0.0f32, w),
            (0, 3, w, 2.0 * w),
            (0, 3, 2.0 * w, 3.0 * w),
        ];
        let xs = assemble_glyph_xs("::>", &clusters, CHAR_WIDTH);
        assert_eq!(xs.len(), 4, "3 chars -> 4 boundaries");
        for k in 0..=3 {
            assert!(
                (xs[k] - k as f32 * w).abs() < 1e-3,
                "column {k} must be an even third at {}",
                k as f32 * w
            );
        }
    }

    #[test]
    fn assemble_xs_true_ligature_one_glyph_splits_advance_fairly() {
        // M < N: a TRUE ligature (prose "fi" → ONE glyph covering bytes 0..2 with
        // a single advance W). The two source chars split that advance fairly:
        // char 0 at 0, char 1 at W/2, end at W — the same interpolation rule, so
        // standard prose ligatures get a correct (fair) per-char caret grid too.
        let w = 14.4f32;
        let clusters = [(0usize, 2usize, 0.0f32, w)];
        let xs = assemble_glyph_xs("fi", &clusters, CHAR_WIDTH);
        assert_eq!(xs.len(), 3);
        assert!((xs[0] - 0.0).abs() < 1e-3);
        assert!((xs[1] - w * 0.5).abs() < 1e-3, "single glyph splits fairly at half");
        assert!((xs[2] - w).abs() < 1e-3);
    }

    #[test]
    fn assemble_xs_non_ligature_1to1_is_unchanged() {
        // REGRESSION GUARD: the common 1-glyph-per-1-char case is byte-identical
        // to before the grouping fix — each char sits at its own real advance,
        // even when advances DIFFER (a proportional face). A "healed" ligature
        // followed by a plain char also keeps the plain char at its true x.
        let clusters = [(0usize, 1usize, 0.0f32, 5.0f32), (1, 2, 5.0, 24.0)];
        let xs = assemble_glyph_xs("im", &clusters, CHAR_WIDTH);
        assert_eq!(xs, vec![0.0, 5.0, 24.0]);
        // A shared 2-char cluster (advance 2W) followed by a normal char at 2W.
        let w = 10.0f32;
        let clusters2 = [
            (0usize, 2usize, 0.0f32, w),
            (0, 2, w, 2.0 * w),
            (2, 3, 2.0 * w, 3.0 * w),
        ];
        let xs2 = assemble_glyph_xs("=>x", &clusters2, CHAR_WIDTH);
        assert_eq!(xs2.len(), 4);
        // The plain 'x' boundary is its true right (3W), and the shared span's
        // OWN end boundary is the combined 2W (not overwritten by the old bug).
        assert!((xs2[2] - 2.0 * w).abs() < 1e-3, "shared span end at combined 2W");
        assert!((xs2[3] - 3.0 * w).abs() < 1e-3, "plain char end at its true advance");
        assert!(
            xs2.windows(2).all(|d| (d[1] - d[0] - w).abs() < 1e-3),
            "the whole line stays uniform pitch W"
        );
    }

    // --- IME preedit splice position (line/col -> char index) --------------

    #[test]
    fn line_col_to_char_index_basic() {
        let t = "hello\nworld";
        assert_eq!(line_col_to_char_index(t, 0, 0), 0);
        assert_eq!(line_col_to_char_index(t, 0, 5), 5); // end of "hello"
        assert_eq!(line_col_to_char_index(t, 1, 0), 6); // start of "world"
        assert_eq!(line_col_to_char_index(t, 1, 5), 11); // end of buffer
    }

    #[test]
    fn line_col_to_char_index_clamps_col() {
        let t = "hi\nlonger";
        // col past end of line 0 clamps to just before the newline (char idx 2).
        assert_eq!(line_col_to_char_index(t, 0, 99), 2);
    }

    #[test]
    fn line_col_to_char_index_multibyte_cjk() {
        // "日本\nx": each kanji is one CHAR (3 bytes). Splice index is in CHARS,
        // so col 1 on line 0 is char index 1 (byte 3), col 2 is char index 2.
        let t = "日本\nx";
        assert_eq!(line_col_to_char_index(t, 0, 0), 0);
        assert_eq!(line_col_to_char_index(t, 0, 1), 1);
        assert_eq!(line_col_to_char_index(t, 0, 2), 2);
        assert_eq!(line_col_to_char_index(t, 1, 0), 3); // after the '\n'
        // And the byte offset of char index 1 is 3 (one full-width kanji in).
        assert_eq!(t.char_indices().nth(1).map(|(b, _)| b), Some(3));
    }

    #[test]
    fn max_scroll_accounts_for_viewport() {
        // `max_scroll`'s first arg is the TOTAL VISUAL ROW count (the scroll unit).
        // A doc taller than the viewport now gets ~one screenful of "scroll past
        // end" headroom: the max lets the LAST row rise to the top of the viewport,
        // i.e. `total - OVERSCROLL_KEEP_ROWS`.
        let visible = visible_lines_z(H, LINE_HEIGHT);
        // A doc taller than the viewport scrolls until its last row reaches the top.
        assert_eq!(
            max_scroll(visible + 30, H, LINE_HEIGHT),
            visible + 30 - OVERSCROLL_KEEP_ROWS
        );
        // A doc that fits entirely (or is shorter) cannot scroll into the void.
        assert_eq!(max_scroll(visible, H, LINE_HEIGHT), 0);
        assert_eq!(max_scroll(visible.saturating_sub(3), H, LINE_HEIGHT), 0);
        assert_eq!(max_scroll(1, H, LINE_HEIGHT), 0);
        assert_eq!(max_scroll(0, H, LINE_HEIGHT), 0);
    }

    #[test]
    fn max_scroll_reaches_last_visual_row_of_wrapped_doc() {
        // A WRAPPED document has MORE visual rows than logical lines, and
        // max_scroll must let the LAST visual row reach the bottom. Say 50 logical
        // lines each wrap into ~3 rows -> ~150 visual rows. With `visible` rows on
        // screen, the max scroll is total_rows - visible, NOT (logical - visible).
        let visible = visible_lines_z(H, LINE_HEIGHT);
        let logical = 50usize;
        let total_visual = logical * 3; // each line wraps to 3 rows
        let m = max_scroll(total_visual, H, LINE_HEIGHT);
        // With "scroll past end" the max lets the last row reach the TOP, so the
        // ceiling is `total - OVERSCROLL_KEEP_ROWS`, ~one screenful past the old
        // bottom-pinned `total - visible`.
        assert!(m > total_visual - visible, "overscroll must exceed the bottom pin");
        assert_eq!(m, total_visual - OVERSCROLL_KEEP_ROWS);
        // The bug this fixes: a logical-line max would stop far too early. Prove
        // the visual-row max is strictly larger than the old logical-line max
        // would have been, so the previously-unreachable last rows are reachable.
        let old_logical_max = max_scroll(logical, H, LINE_HEIGHT);
        assert!(m > old_logical_max, "visual-row max must exceed logical-line max");
        // At max scroll the window is [m, m+visible); the last visual row index
        // (total_visual-1) now sits at the TOP of that window: m == total_visual-1.
        assert_eq!(m, total_visual - 1);
    }

    #[test]
    fn max_scroll_overscrolls_past_end_but_stays_bounded() {
        // "Scroll past end": a buffer TALLER than the viewport can now scroll until
        // its last row rises to ~the TOP of the viewport, ~one screenful of extra
        // headroom past where the last row pins to the bottom — and no further.
        let visible = visible_lines_z(H, LINE_HEIGHT);
        let total = visible + 50; // taller than the viewport
        let m = max_scroll(total, H, LINE_HEIGHT);

        // The OLD max pinned the last row to the bottom: total - visible.
        let old_max = total - visible;
        // The new max is strictly GREATER (it allows overscroll past the end)...
        assert!(m > old_max, "new max ({m}) must exceed old bottom-pinned max ({old_max})");
        // ...and lets the last row reach ~the top: total - 1 (a small margin away
        // from the absolute top is allowed via OVERSCROLL_KEEP_ROWS).
        assert_eq!(m, total - OVERSCROLL_KEEP_ROWS);
        assert!(m <= total - 1, "must not scroll the last row off the top");

        // BOUNDED: the overscroll past the old max is at most ONE screenful, never
        // an unbounded blank void.
        let overscroll = m - old_max;
        assert!(
            overscroll <= visible,
            "overscroll ({overscroll}) must be capped to ~one screenful ({visible})"
        );

        // Scrolling UP still clamps at the top, and a doc that fits can't scroll.
        assert_eq!(max_scroll(visible, H, LINE_HEIGHT), 0);
    }

    #[test]
    fn non_wrap_visual_rows_equal_logical_lines_invariant() {
        // INVARIANT: when nothing wraps, total visual rows == logical line count,
        // so max_scroll (and therefore every scroll computation built on it) is
        // byte-for-byte the old logical-line behavior. We model "nothing wraps" by
        // total_visual_rows == line_count and assert the two max_scroll values
        // agree for a spread of document sizes.
        let visible = visible_lines_z(H, LINE_HEIGHT);
        for line_count in [0usize, 1, 5, visible, visible + 1, visible + 40, 200] {
            let total_visual = line_count; // no wrap => 1 visual row per line
            // Expected = base (last row to bottom) + one-screenful overscroll, with
            // a doc that fits getting no overscroll. Same formula whether you feed
            // it logical lines or (equal) visual rows -> the non-wrap invariant.
            let expected = if line_count > visible {
                line_count - OVERSCROLL_KEEP_ROWS
            } else {
                0
            };
            assert_eq!(
                max_scroll(total_visual, H, LINE_HEIGHT),
                expected,
                "non-wrap max_scroll must equal logical-line max for {line_count} lines"
            );
        }
    }

    #[test]
    fn visual_row_of_position_uses_run_line_top_over_line_height() {
        // `visual_row_of` maps a (line, col) to round(run.line_top / line_height).
        // Verify the pure arithmetic with synthetic rows: a non-wrapped line is one
        // row at line_top 0 -> row index 0; a wrapped line's continuation at
        // line_top == 2*line_height -> row index 2, regardless of how `pick_row`
        // chose it. (This mirrors the GPU path which reads real run.line_top.)
        let lh = LINE_HEIGHT;
        // Row at top 0 -> index 0.
        assert_eq!((0.0f32 / lh).round() as usize, 0);
        // Row at top 2*lh -> index 2 (a continuation two rows down).
        assert_eq!((2.0 * lh / lh).round() as usize, 2);
        // Rounding tolerates tiny float drift from centering offsets.
        assert_eq!(((3.0 * lh + 0.3) / lh).round() as usize, 3);
        assert_eq!(((3.0 * lh - 0.3) / lh).round() as usize, 3);
    }

    // --- Wrap-aware vertical positioning (visual rows) --------------------

    #[test]
    fn byte_col_maps_byte_to_char_column() {
        // ASCII: byte == col.
        assert_eq!(byte_col("hello", 0), 0);
        assert_eq!(byte_col("hello", 3), 3);
        assert_eq!(byte_col("hello", 5), 5); // end of line == char count
        assert_eq!(byte_col("hello", 99), 5); // past end clamps to char count
        // Multibyte CJK: each kanji is 3 bytes but 1 char column.
        assert_eq!(byte_col("日本語", 0), 0);
        assert_eq!(byte_col("日本語", 3), 1); // second kanji starts at byte 3
        assert_eq!(byte_col("日本語", 6), 2);
        assert_eq!(byte_col("日本語", 9), 3); // end (3 chars)
    }

    /// Build a synthetic visual row with a uniform 1px-per-col x map over its
    /// columns, for testing `pick_row` / `col_in_row` without a GPU.
    fn row(line_top: f32, start_col: usize, end_col: usize, total_cols: usize) -> VisualRow {
        let xs: Vec<f32> = (0..=total_cols).map(|c| c as f32).collect();
        VisualRow {
            line_top,
            line_height: LINE_HEIGHT,
            byte_start: start_col,
            byte_end: end_col,
            start_col,
            end_col,
            xs,
        }
    }

    #[test]
    fn pick_row_single_row_is_uniform_top() {
        // A non-wrapped logical line is one row at line_top 0 (relative to buffer
        // top). For ANY column, pick_row returns that row -> its top is exactly
        // the uniform top. This is the invariant that guarantees non-wrapped
        // content is unchanged: visual_row_top == doc_top() + 0 == uniform.
        let rows = vec![row(0.0, 0, 5, 5)];
        for col in 0..=6 {
            assert_eq!(pick_row(&rows, col).line_top, 0.0, "col {col}");
        }
    }

    #[test]
    fn pick_row_wrapped_picks_the_owning_row() {
        // One logical line wrapped into two rows: cols 0..6 on row A (top 0), cols
        // 6..12 on row B (top 32). At the wrap boundary (col 6) the LOWER row wins.
        let lh = LINE_HEIGHT;
        let rows = vec![row(0.0, 0, 6, 12), row(lh, 6, 12, 12)];
        assert_eq!(pick_row(&rows, 0).line_top, 0.0);
        assert_eq!(pick_row(&rows, 5).line_top, 0.0);
        // Boundary: col 6 is the start of row B -> caret lands on the lower row.
        assert_eq!(pick_row(&rows, 6).line_top, lh, "wrap boundary -> lower row");
        assert_eq!(pick_row(&rows, 9).line_top, lh);
        // End of line (col 12) stays on the last row.
        assert_eq!(pick_row(&rows, 12).line_top, lh);
        // Past end-of-line clamps to the last row.
        assert_eq!(pick_row(&rows, 99).line_top, lh);
    }

    #[test]
    fn pick_row_index_matches_pick_row() {
        // `pick_row_index` is the index form of `pick_row` (same wrap-boundary
        // bias), so the visual-motion oracle can step to the adjacent row.
        let rows = vec![row(0.0, 0, 6, 12), row(LINE_HEIGHT, 6, 12, 12)];
        assert_eq!(pick_row_index(&rows, 0), 0);
        assert_eq!(pick_row_index(&rows, 5), 0);
        // Boundary col 6 -> the LOWER row (index 1), matching pick_row.
        assert_eq!(pick_row_index(&rows, 6), 1);
        assert_eq!(pick_row_index(&rows, 12), 1); // end of line -> last row
        assert_eq!(pick_row_index(&rows, 99), 1); // past end -> last row
    }

    #[test]
    fn col_in_row_hit_maps_x_to_column_on_that_row() {
        // Row B owns cols 6..12 with xs[c] == c. A click x within the row maps to
        // the right GLOBAL column (not a row-local one), snapping past midpoints.
        let rows = vec![row(0.0, 0, 6, 12), row(LINE_HEIGHT, 6, 12, 12)];
        let b = &rows[1];
        // x just inside col 7's cell (7.2) -> col 7.
        assert_eq!(TextPipeline::col_in_row(b, 7.2), 7);
        // x past col 7's midpoint (7.6) -> snaps to col 8.
        assert_eq!(TextPipeline::col_in_row(b, 7.6), 8);
        // x past the row's last glyph -> row end col (12).
        assert_eq!(TextPipeline::col_in_row(b, 99.0), 12);
        // x before the row's first owned col still snaps within the row.
        assert_eq!(TextPipeline::col_in_row(b, 6.1), 6);
    }

    // --- Incremental-shaping / reshape-skip invariants (GPU-backed) --------
    //
    // These build a real headless `TextPipeline` (the shaping path needs a wgpu
    // device). On a machine with no adapter they skip gracefully rather than
    // failing, so the suite still passes in a GPU-less CI.

    /// Build a headless pipeline, or `None` if no wgpu adapter is available.
    fn headless_pipeline() -> Option<TextPipeline> {
        pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .ok()?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("awl test device"),
                    ..Default::default()
                })
                .await
                .ok()?;
            let cache = Cache::new(&device);
            let mut p = TextPipeline::new(
                &device,
                &queue,
                &cache,
                wgpu::TextureFormat::Rgba8UnormSrgb,
            );
            p.set_size(1200.0, 800.0);
            Some(p)
        })
    }

    fn view(text: &str, line: usize, col: usize) -> ViewState {
        ViewState {
            text: text.to_string(),
            cursor_line: line,
            cursor_col: col,
            scroll_lines: 0,
            zoom: 1.0,
            selection: None,
            preedit: String::new(),
            misspelled: Vec::new(),
            is_edit_move: false,
            held: false,
            search_matches: Vec::new(),
            search_current: None,
            search_query: String::new(),
            search_active: false,
            search_case_sensitive: false,
            search_replace_active: false,
            search_replacement: String::new(),
            search_editing_replacement: false,
            overlay_active: false,
            overlay_crisp: false,
            overlay_query: String::new(),
            overlay_items: Vec::new(),
            overlay_empty: None,
            overlay_bindings: Vec::new(),
            overlay_times: Vec::new(),
            overlay_git: Vec::new(),
            overlay_selected: 0,
            overlay_scroll: 0,
            overlay_window_rows: 12,
            overlay_hint: String::new(),
        overlay_lens: Vec::new(),
        overlay_sections: Vec::new(),
            caret_preview: None,
            gutter_name: String::new(),
            gutter_project: String::new(),
            is_markdown: false,
            doc_dir: None,
            syn_lang: None,
            overlay_spell: None,
            notice: String::new(),
            cjk_priority: crate::frontmatter::DEFAULT_CJK_PRIORITY.to_vec(),
            eol: crate::buffer::Eol::Lf,
        }
    }

    /// A markdown [`view`] — same as [`view`] but with `is_markdown` set, so the
    /// styling + outline passes run (used by the margin-outline tests).
    fn view_md(text: &str, line: usize, col: usize) -> ViewState {
        let mut v = view(text, line, col);
        v.is_markdown = true;
        v
    }

    #[test]
    fn selection_rects_multiline_geometry_and_eol_pad() {
        // Selection x geometry folds the page globals (text_left + wrap width);
        // hold the page lock so a parallel page write can't move it (page.rs:95-99).
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping selection_rects_multiline_geometry_and_eol_pad: no wgpu adapter");
            return;
        };
        // A 3-line buffer, selection from line0 col2 through line2 col3: line0 is a
        // partial first line (col2..eol), line1 a full middle line, line2 a partial
        // last line (0..col3).
        let text = "alpha\nbeta\ngamma";
        let mut v = view(text, 2, 3);
        v.selection = Some(((0, 2), (2, 3)));
        p.set_view(&v);

        let rects = p.selection_rects();
        assert_eq!(rects.len(), 3, "one rect per logical line: {rects:?}");

        let m = &p.metrics;
        let eol_pad = m.char_width * 0.5;
        let doc_top = p.doc_top();
        let left = p.text_left();

        // The middle + last lines start at the writing-column left; the first line is
        // inset by its start column.
        assert!((rects[1][0] - left).abs() < 1e-3, "middle line starts at left");
        assert!((rects[2][0] - left).abs() < 1e-3, "last line starts at left");
        assert!(rects[0][0] > left + 1e-3, "first line is inset by its start col");

        // Rows descend in order by one line_height each (uniform, non-heading).
        assert!(rects[0][1] < rects[1][1] && rects[1][1] < rects[2][1], "rows descend");
        assert!(
            (rects[1][1] - rects[0][1] - m.line_height).abs() < 1e-3,
            "row spacing == line_height"
        );
        // Row 0 sits at doc_top centered within its line height.
        let want_y0 = doc_top + (m.line_height - m.caret_h) * 0.5;
        assert!((rects[0][1] - want_y0).abs() < 1e-3, "row0 y centered: {} vs {}", rects[0][1], want_y0);
        // Each rect is one (unscaled) caret-height band.
        for r in &rects {
            assert!((r[3] - m.caret_h).abs() < 1e-3, "rect height == caret_h: {r:?}");
        }

        // The EOL pad: the full middle line equals a no-EOL full selection of the
        // same line PLUS the trailing-newline sliver.
        let mid_no_eol = p.range_rects((1, 0), (1, 4));
        assert_eq!(mid_no_eol.len(), 1, "single-line full selection: {mid_no_eol:?}");
        assert!(
            (rects[1][2] - (mid_no_eol[0][2] + eol_pad)).abs() < 1e-3,
            "middle width == full line + eol_pad: {} vs {}+{}",
            rects[1][2], mid_no_eol[0][2], eol_pad
        );
        // The last line has NO eol pad (it stops at the cursor column).
        let last_only = p.range_rects((2, 0), (2, 3));
        assert!(
            (rects[2][2] - last_only[0][2]).abs() < 1e-3,
            "last line width has no eol pad: {} vs {}",
            rects[2][2], last_only[0][2]
        );
    }

    /// PERF O(visible): `range_rects` (selection / search) over a Select-All in a
    /// TALL doc scrolled to the MIDDLE emits only the visible band's rects — never
    /// one per document line — AND resolves the geometry through the BATCHED
    /// `visual_rows_for_lines`, so it never clobbers the single-slot cursor-line row
    /// memo. The pre-fix per-line `line_glyph_xs` + `visual_rows` walk did BOTH: an
    /// O(doc^2)-per-frame Select-All and a memo stomp on the last selected line. This
    /// WITNESSES THE WORK (the memo survives) rather than just the bounded return.
    #[test]
    fn range_rects_selection_is_visible_bounded_and_memo_safe() {
        // Selection x/y geometry folds the page globals; hold the page lock so a
        // parallel page write can't move the writing column mid-test.
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping range_rects_selection_is_visible_bounded_and_memo_safe: no wgpu adapter");
            return;
        };
        // 2000 short single-row lines: every line is a Select-All member, but only
        // the on-screen band can paint. Scroll to the middle so the lines above sit
        // off the top of the viewport and the tail below it.
        const N: usize = 2000;
        let text: String = (0..N).map(|i| format!("line {i}\n")).collect();
        let cursor_line = N / 2;
        let mut v = view(&text, cursor_line, 0);
        v.scroll_lines = cursor_line - 5; // put the cursor line near the view top
        p.set_view(&v);

        // WARM the single-slot cursor-line memo, then prove Select-All leaves it
        // intact — a per-line `visual_rows` walk (the retired path) would have
        // overwritten it with the LAST selected line's rows.
        let _ = p.visual_rows(cursor_line);
        assert!(
            p.row_geom.cached_rows(cursor_line).is_some(),
            "precondition: the cursor-line row memo is warm"
        );

        let last_col = format!("line {}", N - 1).chars().count();
        let rects = p.range_rects((0, 0), (N - 1, last_col));

        // O(visible): the emitted rects are bounded by the visible band + margin, NOT
        // one per document line (2000).
        assert!(!rects.is_empty(), "the visible selection must produce rects");
        assert!(
            rects.len() < 200,
            "Select-All must emit only the visible band's rects, got {} of {N}",
            rects.len()
        );

        // WITNESS THE WORK: the batched resolve left the cursor-line memo warm.
        assert!(
            p.row_geom.cached_rows(cursor_line).is_some(),
            "range_rects must resolve via the batched path and NOT clobber the cursor-line memo"
        );

        // The cull is exact per row: every emitted rect lands within the viewport +
        // the generous ornament margin (the same band `proto_visible` gates on).
        let margin = p.metrics.line_height * 8.0;
        for r in &rects {
            let (y, h) = (r[1], r[3]);
            assert!(
                y + h > -margin && y < p.window_h + margin,
                "every emitted rect is within the visible band: {r:?}"
            );
        }
    }

    #[test]
    fn oracle_visual_motion_follows_wrapped_rows() {
        // The visual-line LAYOUT ORACLE on the GPU pipeline: visual up/down step
        // through WRAPPED rows of one logical line and cross into adjacent logical
        // lines, all from the shaped geometry. (GPU-backed; skips with no adapter.)
        use crate::actions::LayoutOracle;
        // Soft-wrap geometry folds the page globals (column width); hold the page
        // lock so a parallel page write can't re-wrap the rows mid-test.
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping oracle_visual_motion_follows_wrapped_rows: no wgpu adapter");
            return;
        };
        // A single long logical line that soft-wraps into several visual rows on
        // the 1200px canvas.
        let long = "word ".repeat(80); // 400 chars, wraps
        p.set_view(&view(&long, 0, 0));
        let rows = p.visual_rows(0);
        assert!(rows.len() >= 2, "long line should wrap: {} rows", rows.len());

        // DOWN from the very start (goal-x at the left edge) lands on the FIRST
        // column of the SECOND visual row — SAME logical line, different visual row.
        let gx = p.visual_x_of(0, 0);
        let (dl, dc) = p.visual_line_down(0, 0, gx);
        assert_eq!(dl, 0, "down stays in the same wrapped logical line");
        assert_eq!(dc, rows[1].start_col, "down lands at the next visual row's start");
        // UP from there returns to the first visual row's start (col 0).
        assert_eq!(p.visual_line_up(dl, dc, gx), (0, 0), "up returns to the top row");
        // visual_line_start/end bracket the SECOND visual row's column span.
        assert_eq!(p.visual_line_start(0, dc), (0, rows[1].start_col));
        assert_eq!(p.visual_line_end(0, dc), (0, rows[1].end_col));

        // Crossing LOGICAL lines: a short two-line buffer, down from line 0 to
        // line 1 and back up.
        p.set_view(&view("abc\ndefgh", 0, 1));
        let gx2 = p.visual_x_of(0, 1);
        let (l, c) = p.visual_line_down(0, 1, gx2);
        assert_eq!(l, 1, "down crosses into the next logical line");
        assert_eq!(p.visual_line_up(l, c, gx2).0, 0, "up crosses back to line 0");
    }

    /// FULL VERTICAL-MOTION SWEEP over the real CAPTURE.md (wrapped paragraphs,
    /// headings, lists, inline `code`): for EVERY logical line, a spread of goal_x
    /// (left edge, each row's own end-x + mid-x, far right) and EVERY start column,
    /// one `visual_line_down` step must land STRICTLY BELOW its input (a lower
    /// GROUND-TRUTH visual row from the whole-doc `visual_rows` partition) until the
    /// true LAST visual row, and one `visual_line_up` step STRICTLY ABOVE until the
    /// first. A step that returns the SAME (line,col) is a FIXED POINT — the
    /// "moving straight down gets stuck" bug. GPU-backed; skips with no adapter.
    #[test]
    fn oracle_vertical_sweep_capture_md_strictly_monotonic() {
        use crate::actions::LayoutOracle;
        // Soft-wrap geometry folds the page globals (column width); hold the page
        // lock so a parallel page write can't re-wrap the rows mid-sweep.
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping oracle_vertical_sweep_capture_md: no wgpu adapter");
            return;
        };
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/CAPTURE.md"))
            .expect("CAPTURE.md present at crate root");
        let mut v = view(&text, 0, 0);
        v.is_markdown = true;
        p.set_view(&v);

        let n = p.line_count();
        // GROUND-TRUTH partition: the whole-doc `visual_rows` for every line, plus a
        // prefix sum so any (line,col) maps to ONE global visual-row index. This is
        // the known-correct row partition the oracle's `line_rows_local` must match.
        let all_rows: Vec<Vec<VisualRow>> = (0..n).map(|l| p.visual_rows(l)).collect();
        let mut cum = vec![0usize; n + 1];
        for l in 0..n {
            cum[l + 1] = cum[l] + all_rows[l].len();
        }
        let total = cum[n];
        let gvrow =
            |line: usize, col: usize| -> usize { cum[line] + pick_row_index(&all_rows[line], col) };

        let mut fixed_points: Vec<String> = Vec::new();
        let mut non_descend: Vec<String> = Vec::new();
        let mut non_ascend: Vec<String> = Vec::new();

        for line in 0..n {
            let rows = &all_rows[line];
            let char_count = rows.last().map(|r| r.end_col).unwrap_or(0);
            // goal_x spread: the left edge, each row's own start/end/mid x (the
            // wrap-boundary x's are the interesting ones), and a far-right x.
            let mut gxs: Vec<f32> = vec![0.0, 100_000.0];
            for r in rows {
                let sx = r.xs.get(r.start_col).copied().unwrap_or(0.0);
                let ex = r.xs.get(r.end_col).copied().unwrap_or(0.0);
                gxs.push(sx);
                gxs.push(ex);
                gxs.push((sx + ex) * 0.5);
            }
            for &gx in &gxs {
                for col in 0..=char_count {
                    let g0 = gvrow(line, col);
                    // DOWN: strictly below unless already at the doc's last visual row.
                    let (dl, dc) = p.visual_line_down(line, col, gx);
                    if (dl, dc) == (line, col) {
                        if g0 + 1 != total {
                            fixed_points.push(format!(
                                "DOWN fixed point line={line} col={col} gx={gx:.1} \
                                 (gvrow {g0} of last {})",
                                total - 1
                            ));
                        }
                    } else if gvrow(dl, dc) <= g0 {
                        non_descend.push(format!(
                            "DOWN line={line} col={col} gx={gx:.1}: g{g0} -> ({dl},{dc}) g{}",
                            gvrow(dl, dc)
                        ));
                    }
                    // UP: strictly above unless already at the doc's first visual row.
                    let (ul, uc) = p.visual_line_up(line, col, gx);
                    if (ul, uc) == (line, col) {
                        if g0 != 0 {
                            fixed_points.push(format!(
                                "UP fixed point line={line} col={col} gx={gx:.1} (gvrow {g0})"
                            ));
                        }
                    } else if gvrow(ul, uc) >= g0 {
                        non_ascend.push(format!(
                            "UP line={line} col={col} gx={gx:.1}: g{g0} -> ({ul},{uc}) g{}",
                            gvrow(ul, uc)
                        ));
                    }
                }
            }
        }

        let dump = |label: &str, v: &[String]| {
            if !v.is_empty() {
                eprintln!("=== {label}: {} cases (first 25) ===", v.len());
                for s in v.iter().take(25) {
                    eprintln!("  {s}");
                }
            }
        };
        dump("FIXED POINTS", &fixed_points);
        dump("NON-DESCENDING DOWN", &non_descend);
        dump("NON-ASCENDING UP", &non_ascend);
        assert!(
            fixed_points.is_empty() && non_descend.is_empty() && non_ascend.is_empty(),
            "vertical-motion sweep: {} fixed points, {} non-descending downs, {} non-ascending ups \
             (total visual rows {total})",
            fixed_points.len(),
            non_descend.len(),
            non_ascend.len(),
        );
    }

    /// The user's exact complaint, END TO END: arrowing straight through the real
    /// CAPTURE.md must REACH the far edge and never STICK, for ANY sticky goal_x.
    /// Faithfully replays `actions::motion::vertical_motion` — a real [`Buffer`], a
    /// goal_x seeded ONCE and kept across the run (`set_cursor_visual`), each landing
    /// round-tripped through `line_col_to_char` — then walks a full DOWN from the top
    /// and a full UP from the bottom for a spread of goal_x (incl. the far-right x
    /// that used to wedge on line 471's shared table-wrap boundary). Every walk must
    /// terminate at the last / first visual row, never on a fixed point midway.
    #[test]
    fn oracle_full_vertical_walk_reaches_extremes_capture_md() {
        use crate::actions::LayoutOracle;
        use crate::buffer::Buffer;
        // Soft-wrap geometry folds the page globals (column width); hold the page
        // lock so a parallel page write can't re-wrap the rows mid-walk.
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping oracle_full_vertical_walk: no wgpu adapter");
            return;
        };
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/CAPTURE.md"))
            .expect("CAPTURE.md present at crate root");
        let mut v = view(&text, 0, 0);
        v.is_markdown = true;
        p.set_view(&v);
        let total = p.total_visual_rows();
        let last_line = p.line_count() - 1;

        // Walk one direction with a fixed sticky goal_x; return the number of steps
        // and the final (line,col), stopping on a NO-MOVE (a fixed point / stuck).
        let walk = |p: &TextPipeline, down: bool, seed: (usize, usize), goal: f32| -> (usize, (usize, usize)) {
            let mut buf = Buffer::from_str(&text);
            let seed_idx = buf.line_col_to_char(seed.0, seed.1);
            buf.set_cursor_visual(seed_idx, goal);
            let mut steps = 0usize;
            loop {
                let (line, col) = buf.cursor_line_col();
                let goal_x = buf.goal_x().unwrap_or_else(|| p.visual_x_of(line, col));
                let (nl, nc) = if down {
                    p.visual_line_down(line, col, goal_x)
                } else {
                    p.visual_line_up(line, col, goal_x)
                };
                let before = buf.cursor_char();
                buf.set_cursor_visual(buf.line_col_to_char(nl, nc), goal_x);
                if buf.cursor_char() == before {
                    return (steps, buf.cursor_line_col()); // reached an edge OR stuck
                }
                steps += 1;
                assert!(steps <= total + 50, "runaway walk (down={down}, goal_x={goal})");
            }
        };

        // The four goal_x cover the left edge, mid, and the far-right x's (>= a table
        // row's end) that triggered the pre-fix UP fixed point at line 471 col 416.
        for &goal in &[0.0f32, 500.0, 1050.0, 2000.0] {
            let (_steps, (fl, _fc)) = walk(&p, true, (0, 0), goal);
            assert_eq!(
                fl, last_line,
                "DOWN from the top with goal_x={goal} must reach the LAST logical line, stopped at {fl}"
            );
            let (_steps, (fl, _fc)) = walk(&p, false, (last_line, 0), goal);
            assert_eq!(
                fl, 0,
                "UP from the bottom with goal_x={goal} must reach line 0 (no wrap-boundary stick), stopped at {fl}"
            );
        }
    }

    #[test]
    fn outline_headings_stashed_and_current_is_nearest_at_or_above_caret() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping outline_headings_stashed: no wgpu adapter");
            return;
        };
        // "# Title" (line 0), "## Section A" (line 4), "### Deep" (line 8).
        let text = "# Title\n\nsome prose\n\n## Section A\n\nbody\n\n### Deep\n";

        // A NON-markdown buffer stashes NO outline headings (gated on md_enabled).
        let mut plain = view(text, 0, 0);
        plain.is_markdown = false;
        p.set_view(&plain);
        let (_on, headings, current) = p.outline_report();
        assert!(headings.is_empty(), "non-markdown buffer has no outline: {headings:?}");
        assert_eq!(current, None);

        // A MARKDOWN buffer distills the three headings (riding the md parse).
        let mut md = view(text, 0, 0);
        md.is_markdown = true;
        p.set_view(&md);
        let (_on, headings, current) = p.outline_report();
        assert_eq!(
            headings,
            vec![("Title", 1u8, 0usize), ("Section A", 2, 4), ("Deep", 3, 8)],
            "three headings in document order"
        );
        // Caret on line 0 (the first heading): current is that heading.
        assert_eq!(current, Some(0));

        // Caret on line 2 (prose under the first heading): still the first heading —
        // the nearest AT or ABOVE the caret line.
        p.set_view(&view_md(text, 2, 0));
        assert_eq!(p.outline_current(), Some(0));

        // Caret on line 4 (the second heading's own line): that heading.
        p.set_view(&view_md(text, 4, 0));
        assert_eq!(p.outline_current(), Some(1));

        // Caret on line 6 (body under the second heading): still the second.
        p.set_view(&view_md(text, 6, 0));
        assert_eq!(p.outline_current(), Some(1));

        // Caret on the deepest heading's line 8: the third heading.
        p.set_view(&view_md(text, 8, 0));
        assert_eq!(p.outline_current(), Some(2));
    }

    #[test]
    fn outline_current_is_none_above_the_first_heading() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping outline_current_none_above: no wgpu adapter");
            return;
        };
        // Prose BEFORE the first heading: a caret up there has no heading at/above it.
        let text = "intro line\nmore intro\n\n# First\n\nbody\n";
        p.set_view(&view_md(text, 0, 0));
        assert_eq!(p.outline_current(), None, "caret above the first heading");
        // Move onto the heading line (line 3): now the first heading is current.
        p.set_view(&view_md(text, 3, 0));
        assert_eq!(p.outline_current(), Some(0));
    }

    /// THE MARGIN OUTLINE RENDER: it draws its heading list ONLY when on + page mode +
    /// markdown + a wide-enough margin, and hides gracefully otherwise (off / edge-to-edge
    /// / non-markdown / heading-free). The CURRENT heading (nearest at/above the caret)
    /// is the one CONTENT (dark) row among the FAINT rest — asserted here via the
    /// drawn-lines report, the SAME `outline_layout` owner the pixels shape from.
    #[test]
    fn outline_draws_on_page_md_and_the_current_row_is_flagged() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping outline_draws_on_page_md: no wgpu adapter");
            return;
        };
        let _o = crate::outline::TEST_LOCK.lock().unwrap();
        let _g = crate::page::test_lock();
        crate::outline::set_outline_on(true);
        crate::page::set_measure(40);
        crate::page::set_page_on(true);
        // A WIDE window so the left margin comfortably clears the OUTLINE_MIN_CHARS floor
        // (at the 1200px default the page margin is too narrow — see the floor test).
        p.set_size(1900.0, 900.0);
        // Three headings; caret on the first (line 0).
        let text = "# Title\n\nprose\n\n## Section A\n\nbody\n\n### Deep\n";
        p.set_view(&view_md(text, 0, 0));

        use chrome::{OutlineRow, OutlineRung};
        // `line` is the source heading's 0-based document line (the click-to-jump
        // target): "# Title" is line 0, "## Section A" line 4, "### Deep" line 8.
        // These cases are fully visible (nothing clips), so every row is un-`faded`.
        let row = |label: &str, rung: OutlineRung, current: bool, gap_before: bool, line: usize| {
            OutlineRow { label: label.to_string(), rung, faded: false, current, gap_before, line }
        };

        let lines = p
            .outline_draw_report(900)
            .expect("page + md + on + a wide margin => the outline is drawn");
        // The per-level indent rides `Heading::label()` (h1 flush, h2/h3 indented).
        // TWO-STATE ink: caret on line 0 lights ONLY the current H1 (Content); every
        // other heading is Faint (depth reads from the indent, not ink). A half-row
        // group gap precedes the H2 (a later top-level section), never the H3.
        assert_eq!(
            lines,
            vec![
                row("Title", OutlineRung::Content, true, false, 0),
                row("  Section A", OutlineRung::Faint, false, true, 4),
                row("    Deep", OutlineRung::Faint, false, false, 8),
            ],
            "current H1 = Content; every other heading Faint; a group gap before the H2"
        );

        // The current row FOLLOWS the caret: move onto the second heading's line (4).
        // Now ONLY the H2 is current (Content); the H1 — an ancestor, but ancestry no
        // longer lifts — drops back to Faint alongside the H3.
        p.set_view(&view_md(text, 4, 0));
        let lines = p.outline_draw_report(900).unwrap();
        assert_eq!(
            lines,
            vec![
                row("Title", OutlineRung::Faint, false, false, 0),
                row("  Section A", OutlineRung::Content, true, true, 4),
                row("    Deep", OutlineRung::Faint, false, false, 8),
            ],
            "only the caret's current H2 is Content; the H1 ancestor is Faint (no lift)"
        );

        // OFF => hidden (None), so a default (off) frame is byte-identical.
        crate::outline::set_outline_on(false);
        assert_eq!(p.outline_draw_report(900), None, "outline off hides it");
        crate::outline::set_outline_on(true);

        // EDGE-TO-EDGE (page off): no margin, so the outline hides.
        crate::page::set_page_on(false);
        p.set_view(&view_md(text, 0, 0));
        assert_eq!(p.outline_draw_report(900), None, "edge-to-edge hides the outline");
        crate::page::set_page_on(true);

        // NON-MARKDOWN: no headings distilled, so the outline hides.
        let mut plain = view(text, 0, 0);
        plain.is_markdown = false;
        p.set_view(&plain);
        assert_eq!(p.outline_draw_report(900), None, "a non-markdown buffer has no outline");

        // A markdown buffer with NO headings hides too.
        p.set_view(&view_md("just prose, no headings here\n", 0, 0));
        assert_eq!(p.outline_draw_report(900), None, "a heading-free doc hides the outline");

        crate::outline::set_outline_on(false);
        crate::page::set_page_on(false);
        crate::page::set_measure(80);
    }

    /// GRACEFUL HIDE: below the [`rowlayout::OUTLINE_MIN_CHARS`] margin floor the whole
    /// outline vanishes rather than draw a useless sliver — exactly as the gutter
    /// collapses on a narrow margin. The fixture derives the char budget from the same
    /// pure geometry the pipeline uses, so a future constant tweak can't make it stale.
    #[test]
    fn outline_hides_below_the_narrow_margin_floor() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping outline_hides_below_the_narrow_margin_floor: no wgpu adapter");
            return;
        };
        let _o = crate::outline::TEST_LOCK.lock().unwrap();
        let _g = crate::page::test_lock();
        crate::outline::set_outline_on(true);
        let measure = 70usize;
        crate::page::set_measure(measure);
        crate::page::set_page_on(true);
        // The 1200px default width: the page margin is genuinely narrow here.
        let window_w = 1200.0;
        p.set_size(window_w, 800.0);
        let text = "# Title\n\n## Section\n";
        p.set_view(&view_md(text, 0, 0));

        // Self-check the fixture lands BELOW the floor (derived, not guessed) — the
        // outline's band is `[TEXT_LEFT, column_left - gap)`, one pad narrower than the
        // gutter's, at the LABEL scale it renders at.
        let col_left = column_left_for(window_w, CHAR_WIDTH, true, measure);
        let gap = CHAR_WIDTH * 1.5;
        let avail = col_left - gap - TEXT_LEFT;
        let label_char_w = CHAR_WIDTH * crate::markdown::type_scale::LABEL;
        let avail_chars = (avail / label_char_w).floor().max(0.0) as usize;
        assert!(
            avail_chars < rowlayout::OUTLINE_MIN_CHARS,
            "fixture must land the margin BELOW the outline floor, got avail_chars={avail_chars}"
        );
        assert_eq!(
            p.outline_draw_report(800),
            None,
            "a margin below the floor hides the outline (graceful collapse)"
        );

        crate::outline::set_outline_on(false);
        crate::page::set_page_on(false);
        crate::page::set_measure(80);
    }

    /// LONG-DOC FOLLOW (the chosen default): when the headings outnumber the rows the
    /// margin can hold, the visible window SLIDES to keep the CURRENT heading on screen —
    /// the section you are in never scrolls off. Uses a SHORT canvas height so only a
    /// few rows fit, with the caret deep in the document.
    #[test]
    fn outline_follow_keeps_the_current_heading_visible_on_a_long_doc() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping outline_follow_keeps_current_visible: no wgpu adapter");
            return;
        };
        let _o = crate::outline::TEST_LOCK.lock().unwrap();
        let _g = crate::page::test_lock();
        crate::outline::set_outline_on(true);
        crate::page::set_measure(40);
        crate::page::set_page_on(true);
        // Wide enough for the char floor, SHORT enough that only a few rows fit.
        let height = 220u32;
        p.set_size(1900.0, height as f32);
        // 40 headings, each block "# Hi\n\nbody\n\n" => heading i sits on line 4*i.
        let mut text = String::new();
        for i in 0..40 {
            text.push_str(&format!("# H{i}\n\nbody\n\n"));
        }
        let last = 39usize;
        p.set_view(&view_md(&text, 4 * last, 0));

        let lines = p
            .outline_draw_report(height)
            .expect("a wide margin + real headings => the outline draws");
        // The margin holds FEWER rows than there are headings (the follow is exercised).
        assert!(
            lines.len() < 40,
            "the short canvas must hold fewer rows than headings, got {}",
            lines.len()
        );
        // EXACTLY one row is the current one, and it is the LAST heading — the caret's
        // section, kept visible by the follow rather than scrolled off the top.
        let current: Vec<&chrome::OutlineRow> = lines.iter().filter(|r| r.current).collect();
        assert_eq!(current.len(), 1, "the current section is always in the followed window");
        assert_eq!(current[0].label, "H39", "the followed window keeps the caret's heading");

        crate::outline::set_outline_on(false);
        crate::page::set_page_on(false);
        crate::page::set_measure(80);
    }

    /// EDGE FADE: when the follow-window CLIPS (more headings than fit), the clipped
    /// first / last visible row is marked `faded` — its Faint ink drops toward the
    /// ground via ALPHA, a quiet "more above / more below" — while the current row
    /// (Content, pinned to the bottom edge by the follow) is NEVER faded. A fully-
    /// visible outline fades nothing.
    #[test]
    fn outline_edge_fade_dims_the_clipped_rows_but_not_the_current() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping outline_edge_fade: no wgpu adapter");
            return;
        };
        let _o = crate::outline::TEST_LOCK.lock().unwrap();
        let _g = crate::page::test_lock();
        crate::outline::set_outline_on(true);
        crate::page::set_measure(40);
        crate::page::set_page_on(true);
        use chrome::OutlineRung;

        // CLIPPING: 40 top-level headings, a SHORT canvas, caret mid-doc (heading 20).
        // The follow pins heading 20 to the bottom edge (clips below) and clips above.
        let height = 220u32;
        p.set_size(1900.0, height as f32);
        let mut text = String::new();
        for i in 0..40 {
            text.push_str(&format!("# H{i}\n\nbody\n\n"));
        }
        p.set_view(&view_md(&text, 4 * 20, 0));
        let lines = p.outline_draw_report(height).expect("outline draws");
        assert!(lines.len() < 40 && lines.len() >= 3, "the window clips, got {}", lines.len());
        // The clipped FIRST row is a non-current heading (Faint) marked `faded` — the
        // "more above" whisper rides ALPHA now (no rung below Faint to step down to).
        assert!(!lines[0].current, "the first clipped row is not the current heading");
        assert_eq!(lines[0].rung, OutlineRung::Faint, "every non-current row is Faint");
        assert!(lines[0].faded, "the clipped top row fades toward the ground (alpha)");
        // The LAST row is the current heading, pinned to the bottom edge — Content and
        // NEVER faded despite the below-clip (the you-are-here row wins over the hint).
        let last = lines.last().unwrap();
        assert!(last.current, "the follow pins the current heading to the bottom edge");
        assert_eq!(last.rung, OutlineRung::Content, "the current row is Content");
        assert!(!last.faded, "the current row is never faded by the edge hint");
        // Interior non-current rows are Faint and NOT faded (only the clipped edges are).
        assert!(
            lines[1..lines.len() - 1].iter().any(|r| !r.current && !r.faded && r.rung == OutlineRung::Faint),
            "interior rows are un-faded Faint"
        );

        // FULLY VISIBLE: 3 headings on a tall canvas, caret on the LAST — nothing clips,
        // so the first (non-current) row is plain un-faded Faint.
        p.set_size(1900.0, 900.0);
        let short = "# One\n\nbody\n\n# Two\n\nbody\n\n# Three\n";
        p.set_view(&view_md(short, 16, 0)); // caret on "# Three"
        let lines = p.outline_draw_report(900).expect("outline draws");
        assert_eq!(lines.len(), 3, "all headings visible");
        assert!(!lines[0].current);
        assert_eq!(lines[0].rung, OutlineRung::Faint, "a non-current row is Faint");
        assert!(!lines[0].faded, "a fully-visible outline fades no edge");

        crate::outline::set_outline_on(false);
        crate::page::set_page_on(false);
        crate::page::set_measure(80);
    }

    #[test]
    fn markdown_styling_gated_and_composed() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping markdown_styling_gated_and_composed: no wgpu adapter");
            return;
        };
        let text = "# Title\n\nsome **bold** words\n";
        // NON-markdown buffer: NO md spans at all (byte-identical render).
        let mut plain = view(text, 0, 0);
        plain.is_markdown = false;
        p.set_view(&plain);
        assert!(
            p.md_report().is_empty(),
            "a non-markdown buffer must yield NO md spans"
        );
        // MARKDOWN buffer: the heading hashes dim to `markup`, the title is `h1`,
        // and the `**bold**` run yields a `bold` span with dim `**` markers.
        let mut md = view(text, 0, 0);
        md.is_markdown = true;
        p.set_view(&md);
        let spans = p.md_report();
        assert!(
            spans.iter().any(|(s, e, t)| *s == 0 && *e == 2 && *t == "markup"),
            "leading '# ' should be a markup span: {spans:?}"
        );
        assert!(
            spans.iter().any(|(s, e, t)| *s == 2 && *e == 7 && *t == "h1"),
            "title 'Title' should be an h1 span: {spans:?}"
        );
        // "some " starts at byte 9; "**bold**" → ** at 14..16, bold 16..20, ** 20..22.
        assert!(
            spans.iter().any(|(_, _, t)| *t == "bold"),
            "a **bold** run should yield a bold span: {spans:?}"
        );
        let bold = spans.iter().find(|(_, _, t)| *t == "bold").unwrap();
        assert!(
            spans
                .iter()
                .any(|(_s, e, t)| *t == "markup" && *e == bold.0),
            "the '**' before a bold run should be a markup span: {spans:?}"
        );
    }

    #[test]
    fn symbol_runs_isolate_modifier_and_ornament_glyphs() {
        // The macOS modifier glyphs + the ornaments are SYMBOLS; ASCII / letters are
        // not, so a chord like "⌘⇧O" yields ONE run over the two leading glyphs and
        // leaves the "O" to the display face.
        assert!(is_symbol('\u{2318}') && is_symbol('\u{21E7}')); // ⌘ ⇧
        assert!(is_symbol('❧') && is_symbol('❦')); // the hr + end ornaments
        assert!(!is_symbol('O') && !is_symbol('-') && !is_symbol('A'));
        let s = "\u{2318}\u{21E7}O"; // ⌘⇧O
        let runs = symbol_runs(s);
        assert_eq!(runs.len(), 1, "the two modifier glyphs form one run: {runs:?}");
        assert_eq!(&s[runs[0].clone()], "\u{2318}\u{21E7}", "run covers ⌘⇧ only");
        // Mid-text section sign: an isolated symbol run between plain text.
        let t = "a \u{00A7}3 b"; // "a §3 b"
        let r2 = symbol_runs(t);
        assert_eq!(r2.len(), 1);
        assert_eq!(&t[r2[0].clone()], "\u{00A7}");
        // A symbol-free line yields no runs (so its render stays byte-identical).
        assert!(symbol_runs("plain ascii line").is_empty());
    }

    #[test]
    fn symbol_face_registered_under_private_family() {
        let Some(p) = headless_pipeline() else {
            eprintln!("skipping symbol_face_registered_under_private_family: no wgpu adapter");
            return;
        };
        // The bundled subset registers under the private SYMBOL_FAMILY name (named
        // only via per-run family spans, never as a display face), so the modifier
        // glyphs + ornaments have a home face to resolve to instead of tofu.
        let registered = p
            .font_system
            .db()
            .faces()
            .any(|f| f.families.iter().any(|(n, _)| n == SYMBOL_FAMILY));
        assert!(registered, "the bundled symbol face must register under {SYMBOL_FAMILY:?}");
    }

    #[test]
    fn horizontal_rule_ornament_gated_and_centered() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping horizontal_rule_ornament_gated_and_centered: no wgpu adapter");
            return;
        };
        // A `---` alone (blank lines around it) is a thematic break on line 2.
        let text = "intro\n\n---\n\nmore\n";

        // MARKDOWN: exactly one section-break ornament (the centered fleuron that
        // REPLACES the old thin rule line), placed on the `---` row; the sidecar
        // still tags the line `rule`.
        let mut md = view(text, 0, 0);
        md.is_markdown = true;
        p.set_view(&md);
        let tops = p.rule_tops();
        assert_eq!(tops.len(), 1, "one --- line => one rule ornament: {tops:?}");
        assert!(
            p.md_report().iter().any(|(_, _, t)| *t == "rule"),
            "the rule line should be tagged `rule` in the sidecar"
        );

        // NON-markdown: the SAME text yields NO rule ornament (gated like every md
        // effect); `prepare_ornaments` uploads no areas, so nothing draws.
        let mut plain = view(text, 0, 0);
        plain.is_markdown = false;
        p.set_view(&plain);
        assert!(
            p.rule_tops().is_empty(),
            "a non-markdown buffer must draw no rule ornaments"
        );
    }

    #[test]
    fn horizontal_rule_conceals_dashes_until_the_caret_lands() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping horizontal_rule_conceals_dashes_until_the_caret_lands: no wgpu adapter");
            return;
        };
        // A `---` thematic break alone on logical line 2 (blank lines around it).
        let text = "intro\n\n---\n\nmore\n";

        // CARET OFF the hr (line 0): an hr is pure markup, so the raw `---` CONCEAL
        // (transparent ink) and the centered fleuron is the only mark — exactly one
        // rule ornament, on the `---` row. The sidecar still tags the line `rule`.
        let mut off = view(text, 0, 0);
        off.is_markdown = true;
        p.set_view(&off);
        assert_eq!(
            p.rule_tops().len(),
            1,
            "caret off the hr => the fleuron draws on the --- row: {:?}",
            p.rule_tops()
        );
        assert!(
            p.rule_line_concealed(2),
            "caret off the hr => the raw --- are concealed (transparent)"
        );
        assert!(
            p.md_report().iter().any(|(_, _, t)| *t == "rule"),
            "the rule line stays tagged `rule` in the sidecar even when concealed"
        );

        // CARET ON the hr line (line 2): the dashes REVEAL (visible, editable) and the
        // fleuron is SUPPRESSED so editing the rule is unobstructed.
        let mut on = view(text, 2, 0);
        on.is_markdown = true;
        p.set_view(&on);
        assert!(
            p.rule_tops().is_empty(),
            "caret on the hr => the fleuron yields to the revealed dashes: {:?}",
            p.rule_tops()
        );
        assert!(
            !p.rule_line_concealed(2),
            "caret on the hr => the raw --- reveal (not transparent)"
        );

        // Moving the caret back OFF re-conceals (the toggle is live, both directions).
        p.set_view(&off);
        assert!(p.rule_line_concealed(2), "caret leaves => --- re-conceal");
        assert_eq!(p.rule_tops().len(), 1, "caret leaves => the fleuron returns");
    }

    #[test]
    fn thematic_break_ornament_tracks_the_syntax_per_line() {
        // This test WRITES the process-global active theme (the pin below); hold
        // the theme lock so it can't yank the world out from under a theme test.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping thematic_break_ornament_tracks_the_syntax_per_line: no wgpu adapter");
            return;
        };
        // Pin the default world (Tawny) so the ornament set is its own trio; read the
        // three glyphs from the world itself so this test tracks a future re-pick.
        theme::set_active(theme::DEFAULT_THEME);
        let orn = theme::active().ornaments;
        let (dash, star, under) = (orn.dash, orn.star, orn.underscore);
        // Three DISTINCT glyphs (the design-table contract) — otherwise the
        // reveal-on-cursor half below can't tell which mark dropped.
        assert!(dash != star && star != under && dash != under);
        // Three DIFFERENT break syntaxes, each alone on its own line (blank-separated):
        // line 2 = `---`, line 4 = `***`, line 6 = `___`.
        let text = "intro\n\n---\n\n***\n\n___\n\nmore\n";

        // CARET OFF every break (line 0): all three ornaments draw, each the glyph its
        // OWN syntax picked — dash / star / underscore in document order. This is the
        // whole feature: the mark tracks the type the author wrote.
        let mut off = view(text, 0, 0);
        off.is_markdown = true;
        p.set_view(&off);
        let marks: Vec<char> = p.rule_marks().into_iter().map(|(_, c)| c).collect();
        assert_eq!(
            marks,
            vec![dash, star, under],
            "--- ⁄ *** ⁄ ___ must pick the world's dash ⁄ star ⁄ underscore: {marks:?}"
        );

        // REVEAL-ON-CURSOR still holds PER LINE: put the caret on the `***` line (4).
        // Its ornament yields (the raw *** reveal for editing) while the OTHER two
        // breaks keep their distinct ornaments — dash and underscore, the star dropped.
        let mut on_star = view(text, 4, 0);
        on_star.is_markdown = true;
        p.set_view(&on_star);
        let revealed: Vec<char> = p.rule_marks().into_iter().map(|(_, c)| c).collect();
        assert_eq!(
            revealed,
            vec![dash, under],
            "caret on the *** line suppresses only its star; dash and underscore remain: {revealed:?}"
        );
    }

    #[test]
    fn nested_bullets_cycle_by_depth_and_reveal_on_cursor() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping nested_bullets_cycle_by_depth_and_reveal_on_cursor: no wgpu adapter");
            return;
        };
        // Three nested bullets at depth 0/1/2 (0/2/4 leading spaces), typed with MIXED
        // markers (-, *, +) to prove the glyph is DEPTH-derived, not char-derived.
        let text = "- top\n  * mid\n    + deep\n";

        // Default world (Tawny) → the plain `•`/`◦` pair, cycling every TWO levels.
        // CARET OFF every list line (on the trailing blank line 3): each bullet draws
        // its depth glyph • ◦ • and its raw marker is concealed (transparent ink).
        let mut off = view(text, 3, 0);
        off.is_markdown = true;
        p.set_view(&off);
        assert_eq!(
            p.bullet_glyphs(),
            vec!['•', '◦', '•'],
            "depth 0/1/2 => • ◦ • (pair cycles) regardless of the -,*,+ typed: {:?}",
            p.bullet_glyphs()
        );
        for li in 0..3 {
            assert!(
                p.bullet_marker_concealed(li),
                "caret off => the raw marker on line {li} is concealed"
            );
        }

        // CARET ON the middle bullet (line 1): its raw `*` REVEALS (editable) and no
        // glyph draws for it; the other two keep their depth-0/2 glyph (both •).
        let mut on = view(text, 1, 3);
        on.is_markdown = true;
        p.set_view(&on);
        assert_eq!(
            p.bullet_glyphs(),
            vec!['•', '•'],
            "caret on the mid bullet suppresses only its ◦ (lines 0 and 2 keep •): {:?}",
            p.bullet_glyphs()
        );
        assert!(!p.bullet_marker_concealed(1), "caret on => the mid `*` reveals");
        assert!(
            p.bullet_marker_concealed(0) && p.bullet_marker_concealed(2),
            "the other bullets stay concealed"
        );

        // An ORDERED item keeps its number (no bullet glyph).
        let mut ord = view("1. one\n2. two\n", 2, 0);
        ord.is_markdown = true;
        p.set_view(&ord);
        assert!(p.bullet_glyphs().is_empty(), "ordered lists get no bullet glyph");

        // NON-markdown buffer: no bullets at all (a `.rs` file with `- x` is
        // byte-identical — the glyph is gated on `md_enabled`).
        let mut plain = view(text, 3, 0);
        plain.is_markdown = false;
        p.set_view(&plain);
        assert!(p.bullet_glyphs().is_empty(), "non-markdown => no bullet glyphs");
    }

    /// PER-WORLD BULLETS: the depth-derived glyph swaps to the ACTIVE world's own
    /// [`theme::Theme::bullets`] pair (drawn in its ornament face) — a technical
    /// world keeps `•`/`◦`, a literary serif draws its characterful pair, and
    /// Undertow the manicule. Reveal-on-cursor is unchanged (off-caret only). Proves
    /// the glyph is theme-DATA, not a fixed geometric triple.
    #[test]
    fn bullet_glyphs_swap_per_world() {
        // set_active_by_name mutates the theme global; bullet_marks folds page
        // geometry → hold theme then page (the documented theme→…→page order).
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping bullet_glyphs_swap_per_world: no wgpu adapter");
            return;
        };
        // Two nested bullets at depth 0/1, caret parked off both list lines (line 2).
        let text = "- top\n  - sub\n";
        let cases = [
            ("Tawny", ('•', '◦')),       // geometric world: plain, byte-identical
            ("Undertow", ('☞', '❧')),    // the manicule showpiece + hedera
            ("Gumtree", ('❧', '☙')),     // Junicode botanical hederas
            ("Bilby", ('❧', '❦')),       // Garamond Renaissance fleurons
            ("Mopoke", ('⁑', '❦')),      // the quiet utilitarian Junicode mark
        ];
        for (world, (g0, g1)) in cases {
            theme::set_active_by_name(world).unwrap();
            let mut off = view(text, 2, 0);
            off.is_markdown = true;
            p.set_view(&off);
            assert_eq!(
                p.bullet_glyphs(),
                vec![g0, g1],
                "{world}: depth 0/1 draws its per-world pair {:?}",
                (g0, g1)
            );
            // Reveal-on-cursor still holds: caret on the top bullet (line 0) drops
            // its glyph, leaving only the depth-1 glyph.
            let mut on = view(text, 0, 2);
            on.is_markdown = true;
            p.set_view(&on);
            assert_eq!(
                p.bullet_glyphs(),
                vec![g1],
                "{world}: caret on the top bullet reveals its raw marker (no glyph)"
            );
        }
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// NEVER-TOFU (per-world LIST BULLETS): both glyphs of every world's
    /// [`theme::Theme::bullets`] pair resolve to a REAL glyph in that world's
    /// [`theme::Theme::ornament_face`] — the font-DB half of the structural
    /// `theme::tests::every_world_has_a_bullet_pair` law, mirroring
    /// `ornament_glyphs_resolve_in_each_worlds_assigned_face` for the section trio.
    /// This is what proves the manicule ☞ actually lives in EB Garamond and every
    /// Junicode hedera in the bundled ornament subset.
    #[test]
    fn bullet_glyphs_resolve_in_each_worlds_assigned_face() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping bullet_glyphs_resolve_in_each_worlds_assigned_face: no wgpu adapter");
            return;
        };
        for t in theme::THEMES.iter() {
            let id = p
                .font_system
                .db()
                .faces()
                .find(|f| f.families.iter().any(|(n, _)| n == t.ornament_face))
                .map(|f| f.id)
                .unwrap_or_else(|| panic!("{}: ornament face {:?} is registered", t.name, t.ornament_face));
            let font = p
                .font_system
                .get_font(id, glyphon::cosmic_text::fontdb::Weight::NORMAL)
                .unwrap_or_else(|| panic!("{}: ornament face {:?} loads", t.name, t.ornament_face));
            let charmap = font.as_swash().charmap();
            for (level, ch) in [("level-1", t.bullets.0), ("level-2", t.bullets.1)] {
                assert!(
                    charmap.map(ch) != 0,
                    "{}: {} bullet {:?} (U+{:04X}) is NOT in its ornament face {:?} — tofu",
                    t.name,
                    level,
                    ch,
                    ch as u32,
                    t.ornament_face
                );
            }
        }
    }

    /// PERF O(visible): `bullet_marks` places each visible bullet's glyph WITHOUT the
    /// retired per-line O(li) `line_glyph_xs` walk (an O(doc) `layout_runs` walk from
    /// doc start, per bullet — O(visible_bullets × scroll) each frame, breaking the
    /// O(visible) law its sibling `rule_marks` honours by reading cached row geometry).
    /// An UNINDENTED bullet needs no walk at all (its marker sits at column 0); an
    /// INDENTED bullet resolves through the BATCHED, memo-safe `visual_rows_for_lines`,
    /// never a per-line `visual_rows` (which would clobber the single-slot cursor-line
    /// row memo). Placement stays byte-identical to the retired `line_glyph_xs`-based x.
    /// Mirrors `range_rects_selection_is_visible_bounded_and_memo_safe`.
    #[test]
    fn bullet_marks_placement_unchanged_and_geometry_is_o_visible() {
        // Bullet x folds the page globals (writing-column left); hold the page lock so
        // a parallel page write can't move the column mid-test.
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping bullet_marks_placement_unchanged_and_geometry_is_o_visible: no wgpu adapter");
            return;
        };

        // PART A — PLACEMENT UNCHANGED. A small doc mixing an UNINDENTED bullet
        // (indent 0, my x==0 branch) and an INDENTED one (indent 2, the batched path),
        // caret on the trailing blank line so every bullet is placed. Each mark's x
        // must equal the retired `line_glyph_xs(li)[indent]`-based x, byte-for-byte.
        let mut small = view("- a\n  - b\n- c\n\n", 3, 0);
        small.is_markdown = true;
        p.set_view(&small);
        let text_left = p.text_left();
        let marks = p.bullet_marks(); // ascending line order: lines 0, 1, 2
        assert_eq!(marks.len(), 3, "all three bullets placed: {marks:?}");
        let expect = |li: usize, indent: usize| -> f32 {
            text_left + p.line_glyph_xs(li).get(indent).copied().unwrap_or(0.0)
        };
        for (mark, (li, indent)) in marks.iter().zip([(0, 0), (1, 2), (2, 0)]) {
            let want = expect(li, indent);
            assert!(
                (mark.1 - want).abs() < 0.01,
                "bullet x on line {li} (indent {indent}) changed: {} vs {want}",
                mark.1
            );
        }
        // Sanity: the indented bullet really sits right of the unindented ones (so the
        // batched path is exercised on a genuinely offset marker, not a vacuous 0).
        assert!(
            marks[1].1 > marks[0].1 + 0.5,
            "the indented bullet's marker must sit right of column 0: {marks:?}"
        );

        // PART B — O(visible) + memo-safe. A TALL doc (many bullets, every 3rd
        // INDENTED so the visible band always contains some) scrolled to the middle:
        // only the on-screen band's bullets are placed, and the batched resolve leaves
        // the warm cursor-line row memo intact.
        const N: usize = 400;
        let text: String = (0..N)
            .map(|i| if i % 3 == 0 { "  - x\n" } else { "- y\n" })
            .collect();
        let cursor_line = N / 2;
        let mut tall = view(&text, cursor_line, 0);
        tall.is_markdown = true;
        tall.scroll_lines = cursor_line - 5; // put the caret near the view top
        p.set_view(&tall);

        // WARM the single-slot cursor-line memo, then prove `bullet_marks` leaves it
        // intact — a per-line `visual_rows` walk (the wrong fix) would stomp it.
        let _ = p.visual_rows(cursor_line);
        assert!(
            p.row_geom.cached_rows(cursor_line).is_some(),
            "precondition: the cursor-line row memo is warm"
        );

        let tall_marks = p.bullet_marks();
        assert!(!tall_marks.is_empty(), "the visible bullets must be placed");
        assert!(
            tall_marks.len() < 100,
            "only the visible band's bullets, got {} of {N}",
            tall_marks.len()
        );
        // WITNESS THE WORK: an INDENTED bullet is in the visible band (some x sits
        // right of column 0), so `visual_rows_for_lines` genuinely ran — and it left
        // the cursor-line memo warm (the batched, memo-safe path, not per-line
        // `visual_rows`).
        assert!(
            tall_marks.iter().any(|m| m.1 > text_left + 0.5),
            "an indented bullet must be visible so the batched geometry path runs"
        );
        assert!(
            p.row_geom.cached_rows(cursor_line).is_some(),
            "bullet_marks must resolve indented bullets via the batched (memo-safe) path"
        );
    }

    #[test]
    fn wordcount_readout_gated_to_markdown() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping wordcount_readout_gated_to_markdown: no wgpu adapter");
            return;
        };
        let text = "one two three four five\n"; // 5 words

        // MARKDOWN: the readout reports the word count + a (rounded-up) reading time.
        let mut md = view(text, 0, 0);
        md.is_markdown = true;
        p.set_view(&md);
        assert_eq!(
            p.readout_report(),
            Some((5, 1)),
            "5 words => `5 words · 1 min`"
        );

        // NON-markdown: NO readout (gated, so a plain buffer stays byte-identical).
        let mut plain = view(text, 0, 0);
        plain.is_markdown = false;
        p.set_view(&plain);
        assert_eq!(p.readout_report(), None, "non-markdown => no readout");

        // An empty markdown buffer has nothing to read.
        let mut blank = view("", 0, 0);
        blank.is_markdown = true;
        p.set_view(&blank);
        assert_eq!(p.readout_report(), None, "a wordless buffer => no readout");
    }

    /// i18n: a leading frontmatter block is METADATA, not manuscript — its
    /// `lang:`/etc. lines never inflate the word-count/reading-time readout.
    #[test]
    fn readout_excludes_frontmatter_block() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping readout_excludes_frontmatter_block: no wgpu adapter");
            return;
        };
        // Frontmatter contributes "lang ja" (2 words) which must NOT count;
        // only the 5-word body should.
        let text = "---\nlang: ja\n---\none two three four five\n";
        let mut md = view(text, 0, 0);
        md.is_markdown = true;
        p.set_view(&md);
        assert_eq!(
            p.readout_report(),
            Some((5, 1)),
            "the frontmatter's own words must not count toward the readout"
        );

        // A document that is FRONTMATTER ONLY (no body) reads as wordless.
        let fm_only = "---\nlang: ja\ntitle: x\n---\n";
        let mut md2 = view(fm_only, 0, 0);
        md2.is_markdown = true;
        p.set_view(&md2);
        assert_eq!(p.readout_report(), None, "a frontmatter-only doc has nothing to read");
    }

    #[test]
    fn notice_parked_offscreen_when_empty() {
        // The CALM NOTICE mirrors the ViewState field and defaults EMPTY — the
        // empty string routes through the shared corner-label body's park-off-
        // screen arm (the same gate the wordcount/gutter byte-identity rides),
        // so every capture (which can never carry a notice — autosave is
        // live-only) draws nothing. A live notice lands in the mirror verbatim
        // and clears back to empty when the view drops it.
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping notice_parked_offscreen_when_empty: no wgpu adapter");
            return;
        };
        let v = view("hello\n", 0, 0);
        p.set_view(&v);
        assert!(p.notice.is_empty(), "default view carries no notice");
        let mut warned = view("hello\n", 0, 0);
        warned.notice = "changed on disk outside awl — autosave held".to_string();
        p.set_view(&warned);
        assert_eq!(
            p.notice, "changed on disk outside awl — autosave held",
            "a live notice mirrors into the pipeline"
        );
        p.set_view(&v);
        assert!(p.notice.is_empty(), "the notice clears when the view drops it");
    }

    #[test]
    fn gutter_visible_only_in_page_mode_and_dim_overlay_tracks_takeover() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping gutter_visible_only_in_page_mode: no wgpu adapter");
            return;
        };
        let _g = crate::page::test_lock();
        // A named buffer + a NARROW measure so the left margin is wide enough to hold
        // the gutter (the gate also requires a min margin width).
        crate::page::set_measure(40);
        crate::page::set_page_on(true);
        let mut v = view("hello world\n", 0, 0);
        v.gutter_name = "notes.md".to_string();
        v.gutter_project = "awl".to_string();
        p.set_view(&v);
        assert_eq!(
            p.gutter_report(),
            Some(("notes.md".to_string(), "awl".to_string())),
            "page mode + a name + a wide margin => the gutter is drawn"
        );

        // EDGE-TO-EDGE (page off): no margin, so the gutter hides.
        crate::page::set_page_on(false);
        p.set_view(&v);
        assert_eq!(p.gutter_report(), None, "edge-to-edge hides the gutter");

        // An UNNAMED buffer hides the gutter even in page mode.
        crate::page::set_page_on(true);
        let mut blank = view("", 0, 0);
        blank.gutter_name = String::new();
        p.set_view(&blank);
        assert_eq!(p.gutter_report(), None, "no name => no gutter");

        // DIM-OVERLAY tracks a FULL-takeover overlay (not the search split panel).
        let mut over = view("hello\n", 0, 0);
        over.overlay_active = true;
        p.set_view(&over);
        assert!(p.dims_doc(), "a full overlay dims the document behind it");
        let mut peek = view("hello\n", 0, 0);
        peek.search_active = true; // the SPLIT search panel, not a takeover
        p.set_view(&peek);
        assert!(!p.dims_doc(), "the search split panel keeps the document bright");

        crate::page::set_page_on(false);
        crate::page::set_measure(80);
    }

    /// OVERLAY IS INSTANT (no summon/dismiss motion): a summoned card appears at its
    /// settled resting geometry immediately, and a close drops it the same frame the
    /// view clears `overlay_active` — no rise-in offset, no retained sink-out. Guards
    /// the removal of the old overlay-motion round.
    #[test]
    fn overlay_appears_and_closes_instantly_no_motion() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping overlay_appears_and_closes_instantly_no_motion: no wgpu adapter");
            return;
        };
        let _g = crate::page::test_lock();
        let mut over = view("hello\n", 0, 0);
        over.overlay_active = true;
        over.overlay_items = vec!["Alpha".into(), "Beta".into(), "Gamma".into()];
        p.set_view(&over);

        // OPEN: the card is present at its resting geometry immediately, and advancing
        // the live clock never moves it (nothing is animating the overlay).
        let rest = p.overlay_card_rect().expect("overlay card present");
        assert!(p.dims_doc(), "the overlay is open");
        assert!(
            !p.advance(1.0 / 60.0),
            "an open overlay schedules no motion frames"
        );
        assert_eq!(
            p.overlay_card_rect().unwrap(),
            rest,
            "the card never moves — it appears at its settled position"
        );

        // CLOSE: syncing a view with the overlay logically gone drops the card the SAME
        // frame — no retained sink-out.
        let mut closed = view("hello\n", 0, 0);
        closed.overlay_active = false;
        p.set_view(&closed);
        assert!(!p.dims_doc(), "the overlay closes instantly");
        assert!(p.overlay_card_rect().is_none(), "the card is gone the same frame");
    }

    /// THE BUG (user screenshot): at a narrow page-column width the gutter used to
    /// lay the raw filename into a fixed-width wrapping box, so a long name
    /// WRAPPED mid-word ("DESIGN.md" -> "DESIG" / "N.md") and the fixed-height box
    /// clipped the project line right off underneath it. THE FIX (corrected by a
    /// taste pass over the first landing): the gutter pre-fits BOTH the filename
    /// AND the project line to ONE line EACH through the shared `rowlayout`
    /// elision door, sharing the same column-width budget — but fit
    /// INDEPENDENTLY. Neither line yields to the other from width pressure; only
    /// the hard floor hides the whole gutter.
    #[test]
    fn narrow_gutter_never_wraps_and_both_lines_elide_independently() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!(
                "skipping narrow_gutter_never_wraps_and_both_lines_elide_independently: no wgpu adapter"
            );
            return;
        };
        let _g = crate::page::test_lock();

        // A window/measure combo landing the margin comfortably BETWEEN the small
        // collapse floor and the generous ceiling — a real but TIGHT margin, not a
        // degenerate one. Derived from the same pure geometry the pipeline itself
        // uses (not hand-guessed), so a future constant tweak can't silently make
        // this fixture meaningless.
        let window_w = 1700.0;
        let measure = 96usize;
        crate::page::set_measure(measure);
        crate::page::set_page_on(true);
        p.set_size(window_w, 800.0);

        let long_name = "a-fairly-long-descriptive-note-title.md";
        let project = "awl-next";
        let mut v = view("hello world\n", 0, 0);
        v.gutter_name = long_name.to_string();
        v.gutter_project = project.to_string();
        p.set_view(&v);

        // The SAME budget math `gutter_layout` derives, computed here from the
        // pure free functions so the fixture is self-checking.
        let col_left = column_left_for(window_w, CHAR_WIDTH, true, measure);
        let gap = CHAR_WIDTH * 1.5;
        let avail = col_left - gap;
        let label_char_w = CHAR_WIDTH * crate::markdown::type_scale::LABEL;
        let avail_chars = (avail / label_char_w).floor().max(0.0) as usize;
        assert!(
            avail_chars > rowlayout::GUTTER_MIN_NAME_CHARS && avail_chars < long_name.chars().count(),
            "fixture must land the gutter in the ELIDING band (hard floor < avail < name), \
             got avail_chars={avail_chars} name_chars={}",
            long_name.chars().count()
        );
        assert!(
            project.chars().count() <= avail_chars,
            "fixture project must be short enough to stay whole at this avail, \
             got avail_chars={avail_chars} project_chars={}",
            project.chars().count()
        );

        let (name, reported_project) =
            p.gutter_report().expect("a tight-but-real margin still shows the gutter");
        // (1) THE FIX: the filename is ALWAYS one line — never mid-word wrapped —
        // and the sidecar reports EXACTLY what was drawn.
        assert!(!name.contains('\n'), "the filename must render on ONE line, got {name:?}");
        assert!(
            name.chars().count() <= avail_chars,
            "the reported name must fit the same budget the pixels draw at, got {name:?} (budget {avail_chars})"
        );
        assert_ne!(name, long_name, "a name this long in this margin must actually elide");
        assert!(name.ends_with(".md"), "elision preserves the extension: {name:?}");
        // (2) THE CORRECTION: the project line does NOT yield just because the
        // filename is eliding — it stays visible, fit independently against the
        // SAME budget. Here it's short enough to still show whole.
        assert_eq!(
            reported_project, project,
            "the project must keep showing (fit independently) alongside an eliding filename"
        );

        // A SHORT name at this SAME narrow margin is never elided (elision is the
        // last resort) — the fixture isn't just "narrow enough to hide everything".
        let mut short = view("hello world\n", 0, 0);
        short.gutter_name = "short.md".to_string();
        short.gutter_project = project.to_string();
        p.set_view(&short);
        let (short_name, short_project) =
            p.gutter_report().expect("a short name always fits this margin");
        assert_eq!(short_name, "short.md", "a short name is never elided");
        assert_eq!(short_project, project, "a short name leaves plenty of room for the project too");

        // The SYMMETRIC case: a genuinely long PROJECT elides independently too,
        // while a short filename stays whole right alongside it — proving the
        // correction isn't just "name always wins."
        let long_project = "a-fairly-long-project-directory-name";
        assert!(
            avail_chars < long_project.chars().count(),
            "fixture must also land the project in its own eliding band, \
             got avail_chars={avail_chars} project_chars={}",
            long_project.chars().count()
        );
        let mut swapped = view("hello world\n", 0, 0);
        swapped.gutter_name = "short.md".to_string();
        swapped.gutter_project = long_project.to_string();
        p.set_view(&swapped);
        let (swapped_name, elided_project) =
            p.gutter_report().expect("a tight-but-real margin still shows the gutter");
        assert_eq!(swapped_name, "short.md", "the short name is unaffected by the project eliding");
        assert_ne!(elided_project, long_project, "a project this long in this margin must actually elide");
        assert!(elided_project.chars().count() <= avail_chars);
        assert!(!elided_project.contains('\n'), "the project must render on ONE line too");

        crate::page::set_page_on(false);
        crate::page::set_measure(80);
    }

    /// FIX: `blur_signature` must invalidate on a PAGE/WRAP geometry change — a page
    /// drag, `C-x {`/`}`, or a page-mode toggle re-wraps the document (`set_size` /
    /// `sync_wrap_width`) WITHOUT bumping `reshape_count` (that only fires on a text
    /// reshape), so before this fix the cached frosted backdrop stayed stale, showing
    /// the OLD column behind a freshly-reopened overlay. `row_geom.generation()` is
    /// bumped by `RowGeom::invalidate` exactly when the shaped runs actually re-wrap,
    /// and `page::page_on()`/`page::measure()` cover the rare case where the page
    /// flags flip without the wrap width itself changing.
    #[test]
    fn blur_signature_invalidates_on_page_geometry_change_not_on_a_no_op_frame() {
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!(
                "skipping blur_signature_invalidates_on_page_geometry_change: no wgpu adapter"
            );
            return;
        };
        crate::page::set_page_on(false);
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
        p.set_size(1200.0, 800.0);
        let sig_edge_to_edge = p.blur_signature(1200, 800);

        // A NO-OP frame (same size, same page state, no text edit): the signature
        // must NOT change — this is the "settled overlay-open frame re-blurs
        // nothing" guarantee (a caret spring alone must never invalidate it).
        p.set_size(1200.0, 800.0);
        let sig_no_op = p.blur_signature(1200, 800);
        assert_eq!(
            sig_edge_to_edge, sig_no_op,
            "an unchanged page/wrap state must not perturb the blur signature"
        );

        // PAGE-MODE TOGGLE + a narrower measure re-wraps the document at a new
        // column width: the signature must invalidate.
        crate::page::set_page_on(true);
        crate::page::set_measure(40);
        p.set_size(1200.0, 800.0);
        let sig_page_on_narrow = p.blur_signature(1200, 800);
        assert_ne!(
            sig_edge_to_edge, sig_page_on_narrow,
            "toggling page mode (a real wrap-width change) must invalidate the blur signature"
        );

        // A MEASURE-ONLY change (still in page mode) re-wraps again: must invalidate
        // once more.
        crate::page::set_measure(60);
        p.set_size(1200.0, 800.0);
        let sig_measure_wider = p.blur_signature(1200, 800);
        assert_ne!(
            sig_page_on_narrow, sig_measure_wider,
            "a measure-only change must also invalidate the blur signature"
        );

        crate::page::set_page_on(false);
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
    }

    /// The CARET-STYLE preview PANEL: it appears BELOW the picker (a floating card with
    /// the settled sample line + an animated caret) while the caret-style picker is
    /// open, and PARKS (nothing drawn, demo reset) the instant it closes — the panel
    /// primitive's elevation quads and the demo caret all go empty (DESIGN §6 idle).
    #[test]
    fn caret_preview_panel_appears_below_picker_and_stops_on_close() {
        // Build a headless pipeline but KEEP the device/queue so we can drive `prepare`
        // (the elevation-quad instance counts are only set during prepare).
        let got = pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .ok()?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("awl caret-preview test device"),
                    ..Default::default()
                })
                .await
                .ok()?;
            let cache = Cache::new(&device);
            let mut p =
                TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
            p.set_size(1200.0, 800.0);
            Some((device, queue, p))
        });
        let Some((device, queue, mut p)) = got else {
            eprintln!("skipping caret_preview_panel_appears_below_picker_and_stops_on_close: no wgpu adapter");
            return;
        };

        // OPEN the caret-style picker (the familiar Block/Morph/I-beam list), Block row
        // highlighted. Headless: pin the deterministic SETTLED end-state (the loop is
        // live-only), then prepare the frame.
        let mut v = view("hello world\n", 0, 0);
        v.overlay_active = true;
        v.overlay_crisp = true;
        v.overlay_items = vec!["Block".into(), "Morph".into(), "I-beam".into()];
        v.overlay_selected = 0;
        v.overlay_hint = "Enter apply".to_string();
        v.caret_preview = Some(crate::caret::CaretMode::Block);
        p.set_view(&v);
        p.settle_caret_preview();
        p.prepare(&device, &queue, 1200, 800).unwrap();

        // The panel is present, holds the FULL sample line (settled), is a non-degenerate
        // ~2-line box, and hangs clearly BELOW the picker card (whose top is y≈52).
        let (rect, text, _beat, silhouette) = p
            .caret_preview_panel_report()
            .expect("the preview panel is summoned with the picker");
        assert_eq!(text, crate::caret::SAMPLE, "the settled panel shows the full sample line");
        assert!(!silhouette, "Block never paints the Morph silhouette");
        assert!(rect[2] > 300.0, "the panel spans the picker width: {rect:?}");
        assert!(rect[3] > p.metrics.line_height, "a two-line-tall box: {rect:?}");
        assert!(
            rect[1] > 52.0 + 3.0 * p.metrics.line_height,
            "the panel floats below the picker card: {rect:?}"
        );
        // The panel primitive's three elevation quads + the demo caret are all drawn.
        assert_eq!(p.float_card.instance_count(), 1, "the float card is summoned");
        assert_eq!(p.float_shadow.instance_count(), 1, "with a drop shadow");
        assert_eq!(p.float_border.instance_count(), 1, "and a crisp raised edge");
        assert!(p.caret_preview_pipeline.is_drawn(), "the demo caret rides the sample line");

        // CLOSE the picker: the panel + caret park (nothing drawn), the demo resets.
        let closed = view("hello world\n", 0, 0);
        p.set_view(&closed);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        assert!(
            p.caret_preview_panel_report().is_none(),
            "no panel once the picker is closed"
        );
        assert_eq!(p.float_card.instance_count(), 0, "float card parked on close");
        assert_eq!(p.float_shadow.instance_count(), 0, "shadow parked on close");
        assert_eq!(p.float_border.instance_count(), 0, "border parked on close");
        assert!(!p.caret_preview_pipeline.is_drawn(), "preview caret parked on close");
    }

    /// TABLE COLUMN ALLOCATION (the CSS auto-table shape — the fix for the
    /// "Da wn"/"Tim e" mid-word-break bug): a wide GFM table's TOKEN columns
    /// (single-word cells — World / Time / Register) hold a rigid min-content
    /// floor and NEVER shrink as the writing column narrows, while its PHRASE
    /// columns (Ground / Ornament) absorb the whole squeeze by word-wrapping.
    /// Driven end-to-end through the REAL font: `prepare_table_grid` measures the
    /// per-column min/max content and lays them out, and the deterministic
    /// `tables_report()` carries the laid widths. The distinctive signature vs the
    /// retired proportional-shrink clamp is that the token columns are
    /// BYTE-IDENTICAL across two very different measures (the old clamp scaled
    /// EVERY column, so they would have differed).
    #[test]
    fn table_allocation_holds_token_columns_rigid_across_widths() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        crate::markdown::set_wysiwyg_on(true);
        crate::page::set_page_on(true);
        let got = pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .ok()?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("awl table-alloc test device"),
                    ..Default::default()
                })
                .await
                .ok()?;
            let cache = Cache::new(&device);
            let mut p =
                TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
            p.set_size(1200.0, 800.0);
            Some((device, queue, p))
        });
        let Some((device, queue, mut p)) = got else {
            eprintln!("skipping table_allocation_holds_token_columns_rigid_across_widths: no wgpu adapter");
            return;
        };
        // A WORLDS.md-style wide table: token columns 0/4/5 (World/Time/Register)
        // are single words; phrase columns 1/3 (Ground/Ornament) carry multi-word
        // phrases. The caret sits on the trailing prose (line 5), off the table, so
        // the grid draws off-cursor and its widths are measured + reported.
        let text = "\
| World      | Ground                | Display     | Ornament                          | Time  | Register |\n\
|------------|-----------------------|-------------|-----------------------------------|-------|----------|\n\
| Gumtree    | pale eucalyptus-green | Literata    | Junicode botanical sprig fleur    | Day   | Refined  |\n\
| Kingfisher | midnight-navy         | IBM Sans    | Awl Marks pinwheel star lozenge   | Night | Everyday |\n\
\n\
prose after\n";

        let widths_at = |p: &mut TextPipeline, device: &wgpu::Device, queue: &wgpu::Queue, measure: usize| -> Vec<f32> {
            crate::page::set_measure(measure);
            let mut v = view(text, 5, 0);
            v.is_markdown = true;
            p.set_view(&v);
            p.prepare(device, queue, 1200, 800).unwrap();
            let rep = p.tables_report();
            assert_eq!(rep.len(), 1, "one table laid out at measure {measure}");
            assert_eq!(rep[0].cols, 6, "six columns at measure {measure}");
            rep[0].col_widths.clone()
        };

        // A NARROW measure (squeeze/overflow) and a WIDE one (the phrases fit at
        // max-content). The token columns must be byte-identical between them.
        let narrow = widths_at(&mut p, &device, &queue, 44);
        let wide = widths_at(&mut p, &device, &queue, 90);

        for c in [0usize, 4, 5] {
            assert!(
                (narrow[c] - wide[c]).abs() < 0.01,
                "token column {c} is rigid across widths (never shrinks below its word): \
                 narrow={:?} wide={:?}",
                narrow, wide
            );
        }
        // The phrase columns absorbed the extra room at the wide measure — they
        // GREW (word-wrapping at the narrow one) rather than the token columns
        // shrinking.
        for c in [1usize, 3] {
            assert!(
                wide[c] > narrow[c] + 1.0,
                "phrase column {c} absorbs the squeeze (grows with room): \
                 narrow={:?} wide={:?}",
                narrow, wide
            );
        }

        crate::markdown::set_wysiwyg_on(true);
        crate::page::set_page_on(false);
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
    }

    /// THE X-RAY pure caret redirect + pan-to-caret (`xray_col_x` /
    /// `xray_pan_for_caret`): the caret on a concealed table row rides the FLOATED
    /// source's own glyph advances (minus the pan), and the pan keeps the caret
    /// column inside the padded viewport window (the find-field single-line pan),
    /// clamped to the row's scrollable range. Pure — no GPU, no font.
    #[test]
    fn xray_caret_redirect_and_pan_are_pure_and_clamped() {
        let x = crate::render::XrayRow {
            line: 3,
            source: "abc".into(),
            glyph_xs: vec![0.0, 10.0, 25.0, 40.0], // 3 chars, row ends at 40
            top: 0.0,
            height: 20.0,
            pan: 5.0,
        };
        // Redirect: x = glyph_xs[col] − pan; advance = next − this.
        let (gx, adv) = super::geometry::xray_col_x(&x, 0, 8.0);
        assert!((gx + 5.0).abs() < 1e-3 && (adv - 10.0).abs() < 1e-3, "col 0: {gx} {adv}");
        let (gx, adv) = super::geometry::xray_col_x(&x, 2, 8.0);
        assert!((gx - 20.0).abs() < 1e-3 && (adv - 15.0).abs() < 1e-3, "col 2: {gx} {adv}");
        // End of row (col == n) falls back to a default char cell.
        let (gx, adv) = super::geometry::xray_col_x(&x, 3, 8.0);
        assert!((gx - 35.0).abs() < 1e-3 && (adv - 8.0).abs() < 1e-3, "end col: {gx} {adv}");
        // Past the end clamps to n (never panics / reads OOB).
        let (gx, _) = super::geometry::xray_col_x(&x, 99, 8.0);
        assert!((gx - 35.0).abs() < 1e-3, "past-end clamps to n: {gx}");

        use super::geometry::xray_pan_for_caret as pan;
        // A row that fits never pans.
        assert_eq!(pan(50.0, 100.0, 200.0, 8.0, 0.0), 0.0);
        // Caret past the right of the window nudges the pan so the caret sits a pad
        // shy of the right edge (clamped to the scrollable max = content − view).
        let p = pan(480.0, 500.0, 200.0, 10.0, 0.0);
        assert!((p - 290.0).abs() < 1e-3, "right-nudge: {p}");
        // Caret already comfortably in the window keeps the previous pan (no jitter).
        let p = pan(150.0, 500.0, 200.0, 10.0, 50.0);
        assert!((p - 50.0).abs() < 1e-3, "in-window keeps prev: {p}");
        // Caret left of the window nudges the pan left to a pad shy of the caret.
        let p = pan(20.0, 500.0, 200.0, 10.0, 100.0);
        assert!((p - 10.0).abs() < 1e-3, "left-nudge: {p}");
        // The pan never exceeds the scrollable max, whatever the caret asks.
        let p = pan(9999.0, 500.0, 200.0, 10.0, 0.0);
        assert!((p - 300.0).abs() < 1e-3, "clamped to content − view: {p}");
    }

    /// PARK-ON-CLOSE: a CLOSED summoned overlay must leave ZERO stale overlay
    /// pixels for the next frame — the exact live repro is OPEN palette → Esc →
    /// HOLD Cmd-I (the stats HUD), where the HUD forces the frosted-blur backdrop
    /// path that draws the overlay card UNCONDITIONALLY. So after the overlay
    /// closes the text renderer must carry no glyphs and every overlay quad must
    /// be parked (0 instances), regardless of HUD state.
    #[test]
    fn closed_overlay_parks_text_and_quads_even_while_the_hud_is_held() {
        let _g = crate::page::test_lock();
        let got = pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .ok()?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("awl overlay-park test device"),
                    ..Default::default()
                })
                .await
                .ok()?;
            let cache = Cache::new(&device);
            let mut p =
                TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
            p.set_size(1200.0, 800.0);
            Some((device, queue, p))
        });
        let Some((device, queue, mut p)) = got else {
            eprintln!("skipping closed_overlay_parks_text_and_quads_even_while_the_hud_is_held: no wgpu adapter");
            return;
        };

        // OPEN a command-palette-style overlay with a few rows, one selected.
        let mut v = view("hello world\n", 0, 0);
        v.overlay_active = true;
        v.overlay_items = vec![
            "Go to file…".into(),
            "Switch project…".into(),
            "Finish file".into(),
        ];
        v.overlay_selected = 0;
        v.overlay_hint = "↵ run  ←/→ lens".to_string();
        p.set_view(&v);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        // The overlay is drawn: the card + a selected-row band + real glyphs.
        assert_eq!(p.panel_card.instance_count(), 1, "the overlay card is drawn while open");
        assert_eq!(p.overlay_rows.instance_count(), 1, "the selected-row band is drawn");
        assert!(
            p.overlay_text_glyph_count() > 0,
            "the overlay text carries the palette rows while open"
        );

        // CLOSE the overlay AND hold the stats HUD — the exact live repro that
        // forces the frosted-blur path (which draws the overlay card
        // unconditionally). The overlay must now be fully parked anyway.
        crate::hud::set_held(true);
        let closed = view("hello world\n", 0, 0);
        p.set_view(&closed);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        crate::hud::set_held(false);

        assert_eq!(
            p.overlay_text_glyph_count(),
            0,
            "the closed overlay's text renderer carries no stale palette glyphs"
        );
        assert_eq!(p.panel_card.instance_count(), 0, "the card quad is parked on close");
        assert_eq!(p.overlay_rows.instance_count(), 0, "the row band is parked on close");
        assert_eq!(
            p.overlay_lens_underline.instance_count(),
            0,
            "the theme-lens underline is parked on close"
        );
        assert!(!p.panel_caret.is_drawn(), "the amber query caret is parked on close");
    }

    /// EMPTY STATE (pass 3): a picker with NO candidate rows draws ONE dim message
    /// row (the shared `overlay_empty` text) in the candidate area — the card grows a
    /// row for it, the shaped panel actually carries the message glyphs, and NO
    /// selected-row highlight band is drawn (the message is not selectable). A picker
    /// WITH rows reserves no such row (regression guard).
    #[test]
    fn overlay_empty_state_draws_a_dim_message_row() {
        let _g = crate::page::test_lock();
        let got = pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .ok()?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("awl empty-state test device"),
                    ..Default::default()
                })
                .await
                .ok()?;
            let cache = Cache::new(&device);
            let mut p =
                TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
            p.set_size(1200.0, 800.0);
            Some((device, queue, p))
        });
        let Some((device, queue, mut p)) = got else {
            eprintln!("skipping overlay_empty_state_draws_a_dim_message_row: no wgpu adapter");
            return;
        };

        // A go-to picker with a query but NO matching rows → the shared "no matches".
        let mut v = view("hello\n", 0, 0);
        v.overlay_active = true;
        v.overlay_crisp = true;
        v.overlay_items = Vec::new();
        v.overlay_query = "zzz".into();
        v.overlay_empty = Some("no matches".to_string());
        p.set_view(&v);
        p.prepare(&device, &queue, 1200, 800).unwrap();

        // The card reserves a candidate row for the message (query + 1 message row,
        // no hint set here) and the shaped panel carries the message text.
        let joined: String = p
            .panel_buffer
            .lines
            .iter()
            .map(|l| l.text().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("no matches"), "shaped panel shows the message: {joined:?}");
        // No selected-row highlight band: the empty-state message is not selectable.
        assert_eq!(
            p.overlay_rows.instance_count(),
            0,
            "no highlight band over an empty-state message"
        );

        // Regression: a picker WITH rows draws no empty-state message.
        let mut v2 = view("hello\n", 0, 0);
        v2.overlay_active = true;
        v2.overlay_crisp = true;
        v2.overlay_items = vec!["alpha.md".into()];
        v2.overlay_empty = None;
        p.set_view(&v2);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        let joined2: String = p
            .panel_buffer
            .lines
            .iter()
            .map(|l| l.text().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!joined2.contains("no matches"), "no message row when there are rows");
    }

    /// The CARET-STYLE preview PANEL, MORPH highlighted: the settled demo caret
    /// actually paints the glyph-SILHOUETTE (the preview's OWN `CaretGlyphPipeline`,
    /// never the document's), not a permanent thin bar — the picker's one job is to
    /// demonstrate what the highlighted look does to real text, and Morph's whole
    /// point is the recolored letter, not a bar. Closing the picker parks it too.
    #[test]
    fn caret_preview_panel_morph_paints_the_glyph_silhouette() {
        let got = pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .ok()?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("awl caret-preview-morph test device"),
                    ..Default::default()
                })
                .await
                .ok()?;
            let cache = Cache::new(&device);
            let mut p =
                TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
            p.set_size(1200.0, 800.0);
            Some((device, queue, p))
        });
        let Some((device, queue, mut p)) = got else {
            eprintln!("skipping caret_preview_panel_morph_paints_the_glyph_silhouette: no wgpu adapter");
            return;
        };

        // OPEN the caret-style picker with MORPH highlighted; settle (headless: the
        // choreography loop is live-only) to the fully-typed sample line at rest.
        let mut v = view("hello world\n", 0, 0);
        v.overlay_active = true;
        v.overlay_crisp = true;
        v.overlay_items = vec!["Block".into(), "Morph".into(), "I-beam".into()];
        v.overlay_selected = 1;
        v.overlay_hint = "Enter apply".to_string();
        v.caret_preview = Some(crate::caret::CaretMode::Morph);
        p.set_view(&v);
        p.settle_caret_preview();
        p.prepare(&device, &queue, 1200, 800).unwrap();

        let (_rect, text, _beat, silhouette) = p
            .caret_preview_panel_report()
            .expect("the preview panel is summoned with the picker");
        assert_eq!(text, crate::caret::SAMPLE, "settled: the full sample line, caret at rest");
        // Settled at rest on a real letter (the sample ends "...morph", a real glyph
        // one back of the insertion point): the SILHOUETTE pipeline paints (reported
        // straight from the sidecar-facing seam), and the plain block/bar pipeline is
        // suppressed so the two never double-draw.
        assert!(
            silhouette,
            "Morph, settled on a real glyph, must paint the preview's own silhouette"
        );
        assert!(
            p.caret_preview_glyph_pipeline.is_drawn(),
            "the pipeline behind the report is genuinely holding an instance"
        );
        assert!(
            !p.caret_preview_pipeline.is_drawn(),
            "the block/bar pipeline is suppressed while the silhouette paints"
        );

        // CLOSE the picker: both preview caret pipelines park.
        let closed = view("hello world\n", 0, 0);
        p.set_view(&closed);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        assert!(
            !p.caret_preview_glyph_pipeline.is_drawn(),
            "silhouette parked once the picker closes"
        );
        assert!(!p.caret_preview_pipeline.is_drawn(), "block/bar caret parked too");
    }

    /// The CONTEXTUAL SPELL PANEL: the spell overlay renders as a SMALL floating panel
    /// anchored AT the misspelled word (its left edge at the word start, hanging just
    /// below the word's row), on the reusable float primitive with NO scrim/blur — NOT
    /// the centered takeover card the other pickers use. Contrasted against a centered
    /// overlay to prove the geometry actually differs.
    #[test]
    fn spell_panel_floats_at_the_word_not_center_screen() {
        let got = pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .ok()?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("awl spell-panel test device"),
                    ..Default::default()
                })
                .await
                .ok()?;
            let cache = Cache::new(&device);
            let mut p =
                TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
            p.set_size(1200.0, 800.0);
            Some((device, queue, p))
        });
        let Some((device, queue, mut p)) = got else {
            eprintln!("skipping spell_panel_floats_at_the_word_not_center_screen: no wgpu adapter");
            return;
        };
        // The card anchors to the word via text_left, which folds the page
        // globals; hold the page lock so the anchor can't move between the
        // prepare and the assertion reads (page.rs:95-99).
        let _g = crate::page::test_lock();

        // The spell overlay: "teh" is the misspelled word at line 0, cols [0, 3); the
        // panel is anchored at that span and lists the corrections as rows.
        let mut v = view("teh quick brown fox\n", 0, 0);
        v.overlay_active = true;
        v.overlay_items = vec!["the".into(), "tea".into(), "ten".into()];
        v.overlay_selected = 0;
        v.overlay_spell = Some((0, 0, 3));
        p.set_view(&v);
        p.prepare(&device, &queue, 1200, 800).unwrap();

        // It recedes NOTHING (no frosted blur, no scrim) — it's a small popup, not a
        // takeover.
        assert!(!p.dims_doc(), "the contextual spell panel keeps the document crisp");
        // The card floats AT the word: its left edge sits at the word start (text_left,
        // since "teh" begins at col 0) and it is SMALL — nowhere near a centered ~half-
        // canvas card. And it hangs BELOW the word's row (top past the first line).
        let word_left = p.text_left();
        let [x, y, w, _h] = p.overlay_card_rect().expect("the spell overlay has a card");
        assert!((x - word_left).abs() < 2.0, "card left edge anchors to the word start: {x} vs {word_left}");
        assert!(w <= 360.0, "the panel is a small popup, not a wide takeover: w={w}");
        assert!(x + w < 500.0, "the panel stays over the word, not centered: x={x} w={w}");
        assert!(y > p.metrics.line_height, "the panel hangs below the word's row: y={y}");
        // It rides the FLOAT primitive (shadow + border + card), and the flat centered
        // card + the amber query caret are BOTH parked.
        assert_eq!(p.float_card.instance_count(), 1, "the spell panel is a floating card");
        assert_eq!(p.float_shadow.instance_count(), 1, "with a drop shadow");
        assert_eq!(p.float_border.instance_count(), 1, "and a raised border edge");
        assert_eq!(p.panel_card.instance_count(), 0, "no flat centered card for the spell panel");
        assert!(!p.panel_caret.is_drawn(), "no amber query caret on the spell panel");

        // CONTRAST: a centered overlay (no spell target) is a wide card near screen
        // center, on the flat panel card — NOT the float primitive.
        let mut c = view("teh quick brown fox\n", 0, 0);
        c.overlay_active = true;
        c.overlay_items = vec!["the".into(), "tea".into(), "ten".into()];
        p.set_view(&c);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        let [cx, _cy, cw, _ch] = p.overlay_card_rect().expect("the centered overlay has a card");
        assert!(cw >= 360.0, "a centered overlay is a wide card: w={cw}");
        assert!((cx - (1200.0 - cw) * 0.5).abs() < 2.0, "the centered card is horizontally centered: x={cx}");
        assert_eq!(p.float_card.instance_count(), 0, "a centered overlay parks the float card");
        assert_eq!(p.panel_card.instance_count(), 1, "a centered overlay uses the flat card");
    }

    /// SPELL PANEL WIDTH is CONTENT-driven, not word-driven: the card sizes to the
    /// widest suggestion ROW's shaped width + padding (with a calm MIN), so a SHORT
    /// misspelled word can't make a narrow card the longer corrections overflow. The
    /// same short word yields a WIDER card when its suggestions are longer — proof the
    /// width tracks the content, not the (fixed) anchor word.
    #[test]
    fn spell_panel_width_fits_longest_suggestion_not_the_word() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping spell_panel_width_fits_longest_suggestion_not_the_word: no wgpu adapter");
            return;
        };
        let pad = 10.0_f32; // the spell panel's inner padding (spell_overlay_geometry)
        let margin = 8.0_f32;
        let canvas = 1200.0_f32;

        // The SAME short misspelled word ("teh"), once with a LONG suggestion.
        let mut long = view("teh quick brown fox\n", 0, 0);
        long.overlay_active = true;
        long.overlay_items = vec!["the".into(), "thoroughgoingly".into(), "ten".into()];
        long.overlay_selected = 0;
        long.overlay_spell = Some((0, 0, 3));
        p.set_view(&long);
        // The measured content width == the widest shaped suggestion row.
        let content = p.measure_spell_content_w();
        assert!(content > 0.0, "a shaped suggestion has a positive width");
        let [_lx, _ly, w_long, _lh] = p.overlay_card_rect().expect("the spell overlay has a card");
        // The card width follows the formula: content + padding, floored at the calm
        // MIN (140) and capped small (360), kept on-canvas — NOT the word's width.
        let expect = (content + 2.0 * pad).clamp(140.0, 360.0).min(canvas - 2.0 * margin);
        assert!(
            (w_long - expect).abs() < 0.5,
            "card width is content-driven (max-row + pad, min 140, cap 360): got {w_long}, expected {expect} (content {content})"
        );
        // The long suggestion pushed the card PAST the min floor (so this case is
        // meaningful) and its inner text column FITS the suggestion — no overflow.
        assert!(w_long > 140.0, "the long suggestion widens the card past the min: {w_long}");
        assert!(
            w_long - 2.0 * pad >= content - 0.5,
            "the card's text column ({}) fits the longest suggestion ({content})",
            w_long - 2.0 * pad
        );
        assert!(w_long <= 360.0, "still a small popup, not a takeover: {w_long}");

        // The SAME word with only SHORT suggestions → a NARROWER card, clamped to the
        // calm MIN. Width tracks the content, not the (identical) word.
        let mut short = view("teh quick brown fox\n", 0, 0);
        short.overlay_active = true;
        short.overlay_items = vec!["the".into(), "ten".into(), "tea".into()];
        short.overlay_selected = 0;
        short.overlay_spell = Some((0, 0, 3));
        p.set_view(&short);
        let [_sx, _sy, w_short, _sh] = p.overlay_card_rect().expect("the spell overlay has a card");
        assert!(w_short >= 140.0, "a short suggestion set still respects the min width: {w_short}");
        assert!(
            w_short < w_long,
            "the longer suggestions make a WIDER card ({w_long}) than the short set ({w_short}) at the SAME word — content-driven, not word-driven"
        );
    }

    /// THE REPLACE-FIELD CARET rides the reserved cell shaped right after the
    /// REPLACEMENT text on its OWN row (line 1), exactly the way the find caret sits
    /// after the query on row 0. The regression: the reserved cell's byte offset was
    /// computed BUFFER-GLOBAL (`row0_len + "\n" + "replace " + replacement`), but
    /// cosmic-text's `LayoutGlyph::start` is LINE-relative (resets to 0 after every
    /// `\n`), so that offset matched NO line-1 glyph and the caret dropped onto the
    /// hardcoded char-pitch fallback — floating mid-panel on a proportional world.
    /// The caret-x is a PURE function of the shaped layout, so we drive
    /// `panel_shape_text` + `panel_layout` directly and compare against the
    /// INDEPENDENTLY-scanned x of the reserved glyph on line 1.
    #[test]
    fn replace_caret_rides_the_reserved_cell_after_the_replacement_text() {
        // A PROPORTIONAL world (Literata) so the shaped advance genuinely differs from
        // the char-pitch fallback — the bug is invisible on a mono grid where the two
        // coincide. set_active_by_name mutates the theme global → hold the theme lock.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::theme::set_active_by_name("Gumtree").unwrap();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping replace_caret_rides_the_reserved_cell_after_the_replacement_text: no wgpu adapter");
            return;
        };
        let width = 1200u32;
        const REPLACE_LABEL: &str = "replace "; // must match panel.rs's label

        // The reserved-cell glyph's x on line 1, scanned INDEPENDENTLY of the
        // caret-offset math under test — the ground truth the caret must land on.
        let reserved_x = |p: &TextPipeline, text_left: f32, replacement: &str| -> f32 {
            let cell = REPLACE_LABEL.len() + replacement.len();
            for run in p.panel_buffer.layout_runs() {
                if run.line_i != 1 {
                    continue;
                }
                for g in run.glyphs.iter() {
                    if g.start == cell {
                        return text_left + g.x;
                    }
                }
            }
            panic!("no reserved-cell glyph on the replace row for {replacement:?}");
        };

        for replacement in ["world", ""] {
            let mut v = view("hello\nhello\n", 0, 0);
            v.search_active = true;
            v.search_query = "hello".into();
            v.search_matches = vec![((0, 0), (0, 5)), ((1, 0), (1, 5))];
            v.search_current = Some(0);
            v.search_replace_active = true;
            v.search_replacement = replacement.into();
            v.search_editing_replacement = true; // focus on the REPLACE field
            p.set_view(&v);

            let shape = p.panel_shape_text(width);
            assert_eq!(shape.caret_row, 1.0, "replace focus targets row 1");
            // The offset is LINE-relative: the label + replacement WITHIN line 1 only —
            // no find-row bytes, no `\n`.
            assert_eq!(
                shape.caret_byte,
                REPLACE_LABEL.len() + replacement.len(),
                "reserved-cell byte is line-relative for {replacement:?}"
            );
            let (_card, text_left, _top, caret_x) =
                p.panel_layout(width, shape.caret_byte, shape.caret_fallback_chars, shape.caret_row);

            let expected = reserved_x(&p, text_left, replacement);
            assert!(
                (caret_x - expected).abs() < 0.5,
                "replace caret rides the shaped reserved cell (x={caret_x}, expected {expected}) for {replacement:?}"
            );
            // And it is the SHAPED advance, not the hardcoded char-pitch fallback
            // (the old bug's landing spot) — proof we resolved a real line-1 glyph.
            let fallback = text_left + p.metrics.char_width * shape.caret_fallback_chars as f32;
            assert!(
                (caret_x - fallback).abs() > 0.5,
                "on a proportional world the caret is NOT the char-pitch fallback \
                 (x={caret_x}, fallback {fallback}) for {replacement:?}"
            );
        }

        // REGRESSION: with the SAME replace panel up but focus on the FIND field, the
        // caret returns to row 0 riding the query end — the row filter must not have
        // stranded the find caret.
        let mut v = view("hello\nhello\n", 0, 0);
        v.search_active = true;
        v.search_query = "hello".into();
        v.search_matches = vec![((0, 0), (0, 5)), ((1, 0), (1, 5))];
        v.search_current = Some(0);
        v.search_replace_active = true;
        v.search_replacement = "world".into();
        v.search_editing_replacement = false; // focus on the FIND field
        p.set_view(&v);
        let shape = p.panel_shape_text(width);
        assert_eq!(shape.caret_row, 0.0, "find focus targets row 0");
        let (_card, text_left, _top, caret_x) =
            p.panel_layout(width, shape.caret_byte, shape.caret_fallback_chars, shape.caret_row);
        // Ground truth: the reserved gap glyph on line 0 sits at byte "find "+query.
        let cell = "find    ".len() + "hello".len();
        let mut find_expected = None;
        for run in p.panel_buffer.layout_runs() {
            if run.line_i != 0 {
                continue;
            }
            for g in run.glyphs.iter() {
                if g.start == cell {
                    find_expected = Some(text_left + g.x);
                }
            }
        }
        let find_expected = find_expected.expect("reserved gap glyph on the find row");
        assert!(
            (caret_x - find_expected).abs() < 0.5,
            "find caret still rides the query end on row 0 (x={caret_x}, expected {find_expected})"
        );
    }

    /// CLICK-TO-SWITCH-FIELD: the pure `panel_hit` maps a physical pointer to the
    /// find/replace field it lands on, from the SAME `panel_layout` the fields draw
    /// from (no parallel geometry). Row 0 = find, row 1 = replace (present only once
    /// revealed); inside the card but off a row = `Elsewhere` (a swallowed no-op);
    /// off the card / panel down = `None` (falls through to the document). This is
    /// the purest seam of `App::panel_click`'s find↔replace decision.
    #[test]
    fn panel_hit_maps_the_pointer_to_the_find_or_replace_field() {
        // The top-right panel card is anchored to the window's right edge, not the
        // page-mode writing column, so no page-global geometry is folded (no lock).
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping panel_hit_maps_the_pointer_to_the_find_or_replace_field: no wgpu adapter");
            return;
        };
        let width = p.window_w as u32;

        // Replace REVEALED: three panel rows (find / replace / key-hint).
        let mut v = view("hello\nhello\n", 0, 0);
        v.search_active = true;
        v.search_query = "hello".into();
        v.search_matches = vec![((0, 0), (0, 5)), ((1, 0), (1, 5))];
        v.search_current = Some(0);
        v.search_replace_active = true;
        v.search_replacement = "world".into();
        v.search_editing_replacement = false;
        p.set_view(&v);
        // Shape the panel so panel_layout has real rows to measure.
        let shape = p.panel_shape_text(width);
        let ([card_x, card_y, card_w, card_h], _tl, text_top, _cx) =
            p.panel_layout(width, shape.caret_byte, shape.caret_fallback_chars, shape.caret_row);
        let lh = p.metrics.line_height;
        let mid = card_x + card_w * 0.5; // safely inside the card horizontally

        assert_eq!(p.panel_hit(mid, text_top + 0.5 * lh), Some(PanelHit::Find));
        assert_eq!(p.panel_hit(mid, text_top + 1.5 * lh), Some(PanelHit::Replace));
        // The key-hint line (row 2) is inside the card but not editable -> Elsewhere.
        assert_eq!(p.panel_hit(mid, text_top + 2.5 * lh), Some(PanelHit::Elsewhere));
        // Off the card (far left / above / below) -> None: the press falls through.
        assert_eq!(p.panel_hit(card_x - 20.0, text_top + 0.5 * lh), None);
        assert_eq!(p.panel_hit(mid, card_y - 5.0), None);
        assert_eq!(p.panel_hit(mid, card_y + card_h + 5.0), None);

        // Replace NOT revealed: a single find row. Row 0 -> Find; below the one row
        // is off the (1-row) card -> None; the replace band never resolves.
        let mut v1 = view("hello\nhello\n", 0, 0);
        v1.search_active = true;
        v1.search_query = "hello".into();
        v1.search_matches = vec![((0, 0), (0, 5)), ((1, 0), (1, 5))];
        v1.search_current = Some(0);
        v1.search_replace_active = false;
        p.set_view(&v1);
        let shape1 = p.panel_shape_text(width);
        let ([cx1, _cy1, cw1, ch1], _t1, top1, _c1) = p.panel_layout(
            width,
            shape1.caret_byte,
            shape1.caret_fallback_chars,
            shape1.caret_row,
        );
        let mid1 = cx1 + cw1 * 0.5;
        assert_eq!(p.panel_hit(mid1, top1 + 0.5 * lh), Some(PanelHit::Find));
        // The would-be replace band sits below the one-row card -> off card -> None.
        assert!(top1 + 1.5 * lh > _cy1 + ch1, "replace band is below the 1-row card");
        assert_eq!(p.panel_hit(mid1, top1 + 1.5 * lh), None);

        // Panel DOWN -> always None (the press falls through to the document).
        let v2 = view("hello\nhello\n", 0, 0); // search_active defaults false
        p.set_view(&v2);
        assert_eq!(p.panel_hit(mid1, top1 + 0.5 * lh), None);
    }

    /// CLICK-AWAY on a summoned overlay: the three pointer regions `input.rs` resolves
    /// from the SAME `overlay_card_rect` + `overlay_row_at` geometry — ON a candidate
    /// row (→ select+accept), OUTSIDE the card (→ dismiss via `Action::Cancel`, the
    /// close Esc uses; see `actions::overlay_nav` tests), and INSIDE-but-off-a-row (→
    /// swallowed, stays modal). This is the kind-agnostic geometry every overlay shares.
    #[test]
    fn overlay_click_regions_select_inside_row_and_dismiss_outside() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping overlay_click_regions_select_inside_row_and_dismiss_outside: no wgpu adapter");
            return;
        };
        // A centered picker: a query line on top, three candidate rows, a foot hint.
        let mut v = view("hello world\n", 0, 0);
        v.overlay_active = true;
        v.overlay_items = vec!["Alpha".into(), "Beta".into(), "Gamma".into()];
        v.overlay_selected = 0;
        v.overlay_hint = "\u{21B5} run".into();
        p.set_view(&v);

        let [cx, cy, cw, ch] = p.overlay_card_rect().expect("the overlay has a card");
        let lh = p.metrics.line_height;
        let pad = 12.0_f32; // centered-overlay inner padding (overlay_geometry)
        let text_top = cy + pad;
        // The exact predicate input.rs uses for "inside the card".
        let inside = |px: f32, py: f32| px >= cx && px <= cx + cw && py >= cy && py <= cy + ch;

        // ON the first candidate row (one line below the query row): hit-tests to row 0
        // → input.rs selects + accepts it.
        let row_x = cx + cw * 0.5;
        let row0_y = text_top + 1.5 * lh;
        assert_eq!(p.overlay_row_at(row_x, row0_y), Some(0), "a click on the first candidate row selects it");
        assert!(inside(row_x, row0_y), "the row is inside the card");

        // OUTSIDE the card entirely: no row hit AND outside the rect → input.rs routes
        // this to Action::Cancel (dismiss), the same close Esc uses.
        let out_x = cx - 40.0;
        let out_y = cy - 40.0;
        assert_eq!(p.overlay_row_at(out_x, out_y), None, "a click off the card hits no row");
        assert!(!inside(out_x, out_y), "the point is outside the card → dismiss");

        // INSIDE the card but on the QUERY line (not a candidate row): no row hit, yet
        // inside the rect → swallowed, the picker stays modal (no dismiss).
        let query_y = text_top + 0.5 * lh;
        assert_eq!(p.overlay_row_at(row_x, query_y), None, "the query line is not a candidate row");
        assert!(inside(row_x, query_y), "but it is inside the card → swallowed, not dismissed");

        // CURSOR-SHAPE flag sources on this NON-spell picker (the pointing-hand
        // generalization + the query-input I-beam): a candidate row lights the
        // clickable-row flag (→ Pointer) but NOT the query flag; the query line
        // lights the query flag (→ I-beam) but NOT the row flag; off the card
        // lights neither.
        assert!(p.overlay_row_at(row_x, row0_y).is_some(), "row → clickable-overlay-row flag (hand)");
        assert!(!p.over_overlay_query(row_x, row0_y), "a candidate row is not the query field");
        assert!(p.over_overlay_query(row_x, query_y), "the query line → query-input flag (I-beam)");
        assert_eq!(p.overlay_row_at(row_x, query_y), None, "the query line lights no row flag");
        assert!(!p.over_overlay_query(out_x, out_y), "off the card → no query field");
    }

    /// CLICKABLE LENS STRIP: `overlay_lens_at` is the pure x/y → facet-STRIP-INDEX
    /// hit-test `overlay_click` (input.rs) and the cursor-shape hover flag both ride
    /// (one owner — the same geometry the strip SHAPER laid out, read back from the
    /// shaped glyphs). A click/hover on a facet label resolves to its own strip
    /// index regardless of which lens is currently active; off the strip row (the
    /// query line, a candidate row, off the card) resolves to `None`.
    #[test]
    fn overlay_lens_at_resolves_facet_labels_by_their_own_strip_index() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let got = pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .ok()?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("awl test device"),
                    ..Default::default()
                })
                .await
                .ok()?;
            let cache = Cache::new(&device);
            let mut p =
                TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
            p.set_size(1200.0, 800.0);
            Some((device, queue, p))
        });
        let Some((device, queue, mut p)) = got else {
            eprintln!("skipping overlay_lens_at_resolves_facet_labels_by_their_own_strip_index: no wgpu adapter");
            return;
        };

        // A faceted picker shaped like the theme picker: five strip lenses (All,
        // Time, Register, Voice, Temperature — All never drawn), Time active.
        let strip = |active: usize| -> Vec<(String, bool)> {
            ["All", "Time", "Register", "Voice", "Temperature"]
                .iter()
                .enumerate()
                .map(|(i, l)| (l.to_string(), i == active))
                .collect()
        };
        let mut v = view("hello\n", 0, 0);
        v.overlay_active = true;
        v.overlay_items = vec!["Alpha".into(), "Beta".into(), "Gamma".into()];
        v.overlay_selected = 0;
        v.overlay_lens = strip(1); // Time active
        p.set_view(&v);
        p.prepare(&device, &queue, 1200, 800).unwrap();

        let lh = p.overlay_lh();
        let [cx, cy, _cw, _ch] = p.overlay_card_rect().expect("the faceted overlay has a card");
        let pad = 12.0_f32; // centered-overlay inner padding (overlay_geometry)
        let text_top = cy + pad;
        let strip_y = text_top + 1.5 * lh; // mid strip row (display line 1)
        let query_y = text_top + 0.5 * lh; // the query line — not the strip
        let row_y = text_top + 2.5 * lh; // a candidate item row — below the strip

        // The ACTIVE facet's own recorded underline rect pinpoints its shaped x-span —
        // a click in its middle resolves to ITS OWN strip index (1, Time).
        let [ux, uy, uw, _uh] = p.overlay_theme_underline.expect("Time is active, so it is underlined");
        assert!(
            uy >= text_top + lh - 5.0 && uy <= text_top + 2.0 * lh + 5.0,
            "underline sits on the strip row (line 1)"
        );
        let time_mid_x = ux + uw * 0.5;
        assert_eq!(p.overlay_lens_at(time_mid_x, strip_y), Some(1), "a click on Time resolves to strip index 1");

        // Off the strip row entirely (query line, a candidate row) never hits a lens,
        // even at the exact same x as a real facet label.
        assert_eq!(p.overlay_lens_at(time_mid_x, query_y), None, "the query line is not the strip");
        assert_eq!(p.overlay_lens_at(time_mid_x, row_y), None, "a candidate row is not the strip");

        // Off the card entirely (far outside its rect) never hits a lens.
        assert_eq!(p.overlay_lens_at(cx - 200.0, cy - 200.0), None, "off the card hits no lens");

        // Re-shape with Register (index 2) active instead — the SAME x position that
        // hit "Time" above still resolves to strip index 1 (Time's label metrics never
        // move: only its COLOR changes with which lens is active, never its width), and
        // Register's own new underline resolves to its own index (2), not Time's.
        v.overlay_lens = strip(2); // Register active
        p.set_view(&v);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        assert_eq!(
            p.overlay_lens_at(time_mid_x, strip_y),
            Some(1),
            "Time's own x-span still resolves to index 1 even while Register is active"
        );
        let [rx, _ry, rw, _rh] = p.overlay_theme_underline.expect("Register is now active");
        let register_mid_x = rx + rw * 0.5;
        assert_eq!(
            p.overlay_lens_at(register_mid_x, strip_y),
            Some(2),
            "a click on Register resolves to strip index 2"
        );
    }

    /// THE NO-OVERLAP LAW at the pipeline level (rowlayout end-to-end): a row's name
    /// and its dim right column share ONE budget — when the shaped pixels say both
    /// cannot fit, the RIGHT column YIELDS (dropped whole) and the short names stay
    /// crisp (never elided); when both genuinely fit — even at the minimum window —
    /// both show. This is the caret-picker regression: its long descriptions used to
    /// collapse the name budget to a 4-char floor ("Block" → "B…ck") and then paint
    /// straight over the munched names.
    #[test]
    fn overlay_right_column_yields_before_names_elide() {
        // Shaped pixel widths fold the active THEME font and prepare reads the PAGE
        // globals — hold both test locks (theme → page order, page.rs:95-99).
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let got = pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .ok()?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("awl test device"),
                    ..Default::default()
                })
                .await
                .ok()?;
            let cache = Cache::new(&device);
            let mut p =
                TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
            p.set_size(464.0, 600.0);
            Some((device, queue, p))
        });
        let Some((device, queue, mut p)) = got else {
            eprintln!("skipping overlay_right_column_yields_before_names_elide: no wgpu adapter");
            return;
        };

        // A caret-picker-shaped view: SHORT names beside one enormous description no
        // face can fit beside them at the minimum window width.
        let long_desc = "a deliberately enormous description line that no world face could \
                         ever fit beside a candidate name at the minimum window width";
        let mut v = view("hello\n", 0, 0);
        v.overlay_active = true;
        v.overlay_items = vec!["Block".into(), "Morph".into(), "I-beam".into()];
        v.overlay_bindings = vec![long_desc.into(), "short".into(), "also short".into()];
        v.overlay_selected = 0;
        p.set_view(&v);
        p.prepare(&device, &queue, 464, 600).unwrap();
        assert!(
            !p.overlay_right_shown,
            "narrow + oversized right column: the right column must YIELD"
        );
        let line = |p: &TextPipeline, i: usize| p.panel_buffer.lines[i].text().to_string();
        assert_eq!(line(&p, 1), "Block", "a 5-char name is NEVER elided");
        assert_eq!(line(&p, 2), "Morph");
        assert_eq!(line(&p, 3), "I-beam");

        // The SAME names beside SHORT labels at the SAME minimum window: both cells
        // genuinely fit, so the right column shows and the names stay whole —
        // disclosure follows the measured fit, not the window size alone.
        v.overlay_bindings = vec!["hi".into(), "yo".into(), "ok".into()];
        p.set_view(&v);
        p.prepare(&device, &queue, 464, 600).unwrap();
        assert!(
            p.overlay_right_shown,
            "narrow + short right column: both cells fit, the column shows"
        );
        assert_eq!(line(&p, 1), "Block", "names stay whole beside a granted column");

        // And the oversized description yields even at the DEFAULT canvas — the rule
        // is one budget, not a narrow-window special case.
        v.overlay_bindings = vec![long_desc.into(), "short".into(), "also short".into()];
        p.set_view(&v);
        p.set_size(1200.0, 800.0);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        assert!(
            !p.overlay_right_shown,
            "an oversized right column yields at any width"
        );
        assert_eq!(line(&p, 1), "Block", "…and the names still never pay for it");
    }

    /// RESPONSIVE CARD: at the minimum window width the centered picker card spans
    /// nearly the full window (window − 2·margin), mirroring the responsive page
    /// column, instead of the old fixed 360 that starved the text column; at the
    /// default 1200 canvas it stays the familiar 600 (wide captures byte-identical).
    #[test]
    fn overlay_card_spans_nearly_the_full_narrow_window() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping overlay_card_spans_nearly_the_full_narrow_window: no wgpu adapter");
            return;
        };
        let mut v = view("hello\n", 0, 0);
        v.overlay_active = true;
        v.overlay_items = vec!["Alpha".into(), "Beta".into()];
        p.set_view(&v);

        // Minimum window (≈ 30 columns + insets): the card spans window − 24.
        p.set_size(464.0, 600.0);
        let [x, _y, w, _h] = p.overlay_card_rect().expect("overlay card");
        assert!((w - 440.0).abs() < 0.5, "narrow card spans nearly the window: w={w}");
        assert!((x - 12.0).abs() < 0.5, "with the calm 12px margin: x={x}");

        // Default canvas: the same half-window card as ever.
        p.set_size(1200.0, 800.0);
        let [_x, _y, w, _h] = p.overlay_card_rect().expect("overlay card");
        assert!((w - 600.0).abs() < 0.5, "wide card is unchanged: w={w}");
    }

    /// KEY-HINT KEYCAPS: ↵ (Return) and ⇥ (Tab) are classified as SYMBOLS (so the hint
    /// lines shape them from the bundled SYMBOL_FAMILY face like ⌘/⌥, not tofu) AND the
    /// bundled AwlSymbols face actually COVERS both codepoints.
    #[test]
    fn keycap_glyphs_are_symbols_and_bundled() {
        // Classification: both keycaps are symbols; a plain letter is not.
        assert!(is_symbol('\u{21B5}'), "↵ Return is a symbol keycap");
        assert!(is_symbol('\u{21E5}'), "⇥ Tab is a symbol keycap");
        assert!(!is_symbol('r') && !is_symbol('t'), "plain letters are not symbols");
        // A hint fragment isolates the leading glyph run from the plain text remainder.
        let s = "\u{21B5} restore";
        let runs = symbol_runs(s);
        assert_eq!(runs.len(), 1, "one run over the ↵ keycap: {runs:?}");
        assert_eq!(&s[runs[0].clone()], "\u{21B5}", "the run covers ↵ only");

        // Font coverage: the bundled AwlSymbols face resolves both keycaps.
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping keycap_glyphs_are_symbols_and_bundled font-coverage half: no wgpu adapter");
            return;
        };
        let id = p
            .font_system
            .db()
            .faces()
            .find(|f| f.families.iter().any(|(n, _)| n == SYMBOL_FAMILY))
            .map(|f| f.id)
            .expect("the bundled symbol face is registered");
        let font = p
            .font_system
            .get_font(id, glyphon::cosmic_text::fontdb::Weight::NORMAL)
            .expect("the symbol face loads");
        // A nonzero glyph id in the face's charmap means the codepoint resolves to a
        // real glyph (not .notdef / tofu).
        let charmap = font.as_swash().charmap();
        assert!(charmap.map('\u{21B5}') != 0, "AwlSymbols must cover ↵ (U+21B5) — else it renders as tofu");
        assert!(charmap.map('\u{21E5}') != 0, "AwlSymbols must cover ⇥ (U+21E5) — else it renders as tofu");
        // Sanity: the pre-existing ⌘ still resolves, and an uncovered codepoint does not.
        assert!(charmap.map('\u{2318}') != 0, "the ⌘ glyph still resolves");
        assert!(charmap.map('Z') == 0, "a plain letter is NOT in the symbol face");
    }

    /// WRITING NITS: the muted STRAIGHT underline geometry flags exactly the three
    /// mechanical typos (double space, space-before-punct, trailing whitespace) and
    /// NOT the stylistic ones (`!!!`, a 2-space Markdown hard break) — and the whole
    /// layer parks empty when the toggle is off (so a nits-off frame is byte-identical
    /// to no nits). Also proves the underline is FLAT (amplitude 0), the shape that
    /// distinguishes it from the wavy spell squiggle.
    #[test]
    fn nit_underlines_flag_mechanical_typos_straight_and_gate_on_the_toggle() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping nit_underlines_flag_mechanical_typos_straight_and_gate_on_the_toggle: no wgpu adapter");
            return;
        };
        let _g = crate::nits::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // line0: double space (nit). line1: space-before-comma (nit). line2: one
        // trailing space (nit). line3: repeated punctuation (NOT a nit). line4: a
        // 2-space Markdown hard break (NOT a nit). Cursor parked on line3 (the
        // clean, nit-free line) — REVEAL-ON-CURSOR suppresses nits on the CARET's
        // own line, so the fixture avoids that line entirely.
        let text = "a  b\nhi ,x\ntrail \nwow!!!\nbreak  \n";
        let v = view(text, 3, 0);
        p.set_view(&v);

        crate::nits::set_nits_on(true);
        let ul = p.nit_underlines();
        assert_eq!(
            ul.len(),
            3,
            "exactly the double-space, space-before-comma, and trailing-space nits"
        );
        // Every nit underline is STRAIGHT (amp 0) — a flat muted line, NOT a squiggle.
        assert!(
            ul.iter().all(|s| s.amp == 0.0 && s.thickness > 0.0 && s.w > 0.0),
            "nit underlines are straight (amp 0), stroked, and non-empty"
        );

        // Toggled OFF: the layer builds NOTHING (byte-identical to no nits at all).
        crate::nits::set_nits_on(false);
        assert!(
            p.nit_underlines().is_empty(),
            "the nits toggle hides every underline"
        );
        crate::nits::set_nits_on(true);
    }

    /// GFM-TABLE nit exemption: a markdown TABLE row's column-alignment double
    /// spaces (`| Name  | Value |`) must NOT nit — the parsed table spans mark those
    /// lines as rows, and `ensure_nit_protos` picks `line_nits_table_row` for them
    /// (the multi-space rule suppressed). A real prose double space OUTSIDE the table
    /// still flags, proving the exemption is scoped to table rows, not blanket.
    #[test]
    fn nit_underlines_exempt_table_row_column_alignment() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping nit_underlines_exempt_table_row_column_alignment: no wgpu adapter");
            return;
        };
        let _g = crate::nits::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::nits::set_nits_on(true);
        // A table whose header + body rows use column-alignment double spaces, then a
        // prose paragraph carrying a GENUINE double space. Caret parked on the prose
        // line's sibling (line 5) so reveal-on-cursor never masks the assertions.
        let text = "| Name  | Value |\n|-------|-------|\n| foo   | 1     |\n\nreal  slip\n\n";
        let mut v = view(text, 5, 0);
        v.is_markdown = true;
        p.set_view(&v);
        let ul = p.nit_underlines();
        // EXACTLY one nit — the prose "real  slip" double space; every table row's
        // alignment run is exempt.
        assert_eq!(
            ul.len(),
            1,
            "only the prose double space nits; table alignment is exempt: {} nits",
            ul.len()
        );

        // Sanity: with the SAME text rendered as PLAIN (non-markdown) — no table
        // spans — the alignment double spaces DO nit, proving the exemption rides the
        // parsed table markup, not the buffer text.
        let mut plain = view(text, 5, 0);
        plain.is_markdown = false;
        p.set_view(&plain);
        assert!(
            p.nit_underlines().len() > 1,
            "without table markup the alignment runs flag as ordinary double spaces"
        );
        crate::nits::set_nits_on(true);
    }

    /// REVEAL-ON-CURSOR (nits): the CARET's own line never nit-flags, no matter how
    /// many mechanical typos it holds — "typing 'word  ' flags instantly" is
    /// exactly the mid-thought flicker this suppresses. Move the caret to the
    /// OTHER line and that line's nit appears, while the (now caret-owned) line's
    /// own nit vanishes — a pure per-frame READ, not a cache rebuild (no reshape
    /// between the two reads).
    #[test]
    fn nit_underlines_suppress_the_entire_caret_line_only() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping nit_underlines_suppress_the_entire_caret_line_only: no wgpu adapter");
            return;
        };
        let _g = crate::nits::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::nits::set_nits_on(true);
        // line0 and line1 each carry one double-space nit.
        let text = "a  b\nc  d";
        let mut v = view(text, 0, 0); // caret ON line0
        p.set_view(&v);
        let reshapes = p.reshape_count;
        let ul = p.nit_underlines();
        assert_eq!(ul.len(), 1, "only line1's nit survives while the caret sits on line0");

        v.cursor_line = 1; // caret moves to line1 — a pure cursor move, no reshape
        v.cursor_col = 0;
        p.set_view(&v);
        assert_eq!(p.reshape_count, reshapes, "a pure cursor move must not reshape");
        let ul2 = p.nit_underlines();
        assert_eq!(ul2.len(), 1, "line0's nit now shows; line1's (caret's) is suppressed");
        assert!(
            (ul2[0].x - ul[0].x).abs() > 1.0 || (ul2[0].y - ul[0].y).abs() > 1.0,
            "the surviving nit is the OTHER line's, not the same geometry replayed"
        );
        crate::nits::set_nits_on(true);
    }

    /// REVEAL-ON-CURSOR (spell): suppresses ONLY the word the caret sits on/next
    /// to, NOT the whole line — a DIFFERENT misspelling on the SAME line still
    /// squiggles (the taste call the queue flagged explicitly).
    #[test]
    fn spell_squiggles_suppress_only_the_caret_word_not_the_whole_line() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping spell_squiggles_suppress_only_the_caret_word_not_the_whole_line: no wgpu adapter");
            return;
        };
        // "helo" cols 0..4, "wrld" cols 5..9, both misspelled on the SAME line.
        let text = "helo wrld";
        let mis = vec![
            crate::spell::Misspelling { line: 0, start_col: 0, end_col: 4 },
            crate::spell::Misspelling { line: 0, start_col: 5, end_col: 9 },
        ];
        // Caret ON "helo" (col 0, the word's start — inclusive adjacency).
        let mut v = view(text, 0, 0);
        v.misspelled = mis.clone();
        p.set_view(&v);
        let s = p.spell_squiggles();
        assert_eq!(s.len(), 1, "only 'wrld' squiggles; 'helo' (under the caret) yields");

        // Caret moves to "wrld" (col 5): now "helo" squiggles, "wrld" yields.
        v.cursor_col = 5;
        v.misspelled = mis.clone();
        p.set_view(&v);
        let s2 = p.spell_squiggles();
        assert_eq!(s2.len(), 1, "the OTHER word now squiggles");
        assert!(
            (s2[0].x - s[0].x).abs() > 1.0,
            "the surviving squiggle moved to the other word (helo x={}, wrld x={})",
            s[0].x,
            s2[0].x
        );

        // Caret parked well away from BOTH words: both squiggle.
        v.cursor_col = 100;
        v.misspelled = mis;
        p.set_view(&v);
        assert_eq!(p.spell_squiggles().len(), 2, "no word under the caret => both flag");
    }

    /// CODE-BUFFER SCOPE (nits): a recognized code buffer restricts nits to the
    /// lexer's PROSE regions (comment + string), mirroring spell's scoping — a
    /// code-side alignment double-space never nits, while the SAME shape inside a
    /// prose comment still does.
    #[test]
    fn nit_underlines_scope_to_prose_spans_in_a_code_buffer() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping nit_underlines_scope_to_prose_spans_in_a_code_buffer: no wgpu adapter");
            return;
        };
        let _g = crate::nits::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::nits::set_nits_on(true);
        // line0: `let x  = 5; // ok  now` — a CODE-side double space ("x  =", cols
        // 5..7, alignment-shaped) and a COMMENT-side double space ("ok  now", cols
        // 17..19, genuine prose). line1 is an untouched parking spot for the caret
        // (reveal-on-cursor must not be the thing suppressing line0's nit here).
        let text = "let x  = 5; // ok  now\nzzz";
        let mut v = view(text, 1, 0);
        v.syn_lang = Some(crate::syntax::Lang::Rust);
        p.set_view(&v);
        let ul = p.nit_underlines();
        assert_eq!(
            ul.len(),
            1,
            "only the comment's prose double-space nits; the code alignment space doesn't"
        );

        // The SAME text with NO recognized language (prose/plain buffer): both
        // double-spaces are eligible (the pre-existing, unscoped behavior).
        let mut v2 = view(text, 1, 0);
        v2.syn_lang = None;
        p.set_view(&v2);
        assert_eq!(
            p.nit_underlines().len(),
            2,
            "a non-code buffer is unscoped: both double-spaces nit"
        );
        crate::nits::set_nits_on(true);
    }

    /// i18n: a leading frontmatter block's lines never nit — metadata, not
    /// manuscript, mirroring the word-count/spell exclusions exactly.
    #[test]
    fn nit_underlines_exclude_frontmatter_block() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping nit_underlines_exclude_frontmatter_block: no wgpu adapter");
            return;
        };
        let _g = crate::nits::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::nits::set_nits_on(true);
        // The frontmatter's own line has a mechanical double-space nit shape
        // (mid-line, not the 2-trailing-spaces hard-break exception); the body
        // has a genuine one too.
        let text = "---\nlang:  ja\n---\nbody  here\nmore\n";
        let v = view(text, 4, 0); // caret on "more" (nit-free), body's own nit stays eligible
        let mut vmd = v;
        vmd.is_markdown = true;
        p.set_view(&vmd);
        let ul = p.nit_underlines();
        assert_eq!(
            ul.len(),
            1,
            "only the body's double-space nits; the frontmatter's own trailing space never does"
        );

        // The SAME text as NON-markdown: frontmatter detection never even runs
        // (it's a markdown-only concept), so BOTH nits are eligible.
        let mut vplain = view(text, 4, 0);
        vplain.is_markdown = false;
        p.set_view(&vplain);
        assert_eq!(
            p.nit_underlines().len(),
            2,
            "a non-markdown buffer never parses frontmatter, so nothing is excluded"
        );
        crate::nits::set_nits_on(true);
    }

    // --- UnderlineCache / proto invalidation (rects.rs) --------------------
    //
    // The spell-squiggle and nit-underline bands are served from CACHED,
    // scroll-independent protos keyed on (RowGeom generation, spell generation)
    // and (RowGeom generation, reshape count) respectively — the perf seam in
    // rects.rs. These tests pin every key half: a stale cache would keep serving
    // the OLD pixels through an edit / zoom / font switch, or mis-cull on scroll.

    /// SYNTAX WASH CACHE + GEOMETRY: a code buffer's PROSE comment and STRING
    /// spans produce wash quads; commented-out code (CommentCode) produces NONE;
    /// a cursor move / scroll keeps the proto cache WARM (version unchanged, no
    /// reshape — the squiggle-cache invalidation contract); an EDIT rebuilds it;
    /// and a prose buffer yields zero rects (byte-identical render).
    #[test]
    fn wash_cache_and_geometry_contract() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping wash_cache_and_geometry_contract: no wgpu adapter");
            return;
        };
        // A rust buffer: a prose comment (washed), a commented-out statement
        // (NOT washed) and a string literal (washed on this dark default world).
        let text = "// a calm prose note\n// let x = foo(bar);\nlet s = \"hi\";\n";
        let mut v = view(text, 0, 0);
        v.syn_lang = Some(crate::syntax::Lang::Rust);
        p.set_view(&v);
        let (comments, strings, highlights) = p.wash_rects();
        assert!(highlights.is_empty(), "a code buffer never has highlight washes");
        assert_eq!(
            comments.len(), 1,
            "one prose comment => one wash band (the commented-out statement gets none): {comments:?}"
        );
        assert_eq!(strings.len(), 1, "one string literal => one string wash band");
        let key = p.wash_cache_version().expect("protos built");
        let reshapes = p.reshape_count;

        // A CURSOR MOVE keeps the cache warm (no reshape, no rebuild).
        let mut v2 = view(text, 2, 3);
        v2.syn_lang = Some(crate::syntax::Lang::Rust);
        p.set_view(&v2);
        let _ = p.wash_rects();
        assert_eq!(p.reshape_count, reshapes, "a cursor move must not reshape");
        assert_eq!(p.wash_cache_version(), Some(key), "a cursor move keeps the wash protos warm");

        // A SCROLL keeps it warm too (scroll only shifts the per-frame offset).
        let mut v3 = view(text, 2, 3);
        v3.syn_lang = Some(crate::syntax::Lang::Rust);
        v3.scroll_lines = 1;
        p.set_view(&v3);
        let _ = p.wash_rects();
        assert_eq!(p.wash_cache_version(), Some(key), "a scroll keeps the wash protos warm");

        // An EDIT reshapes once and rebuilds the protos (new version key).
        let edited = "// a calm prose note!!\n// let x = foo(bar);\nlet s = \"hi\";\n";
        let mut v4 = view(edited, 0, 0);
        v4.syn_lang = Some(crate::syntax::Lang::Rust);
        p.set_view(&v4);
        let (c2, s2, _h2) = p.wash_rects();
        assert_eq!(p.reshape_count, reshapes + 1, "the edit reshapes once");
        assert_ne!(p.wash_cache_version(), Some(key), "an edit rebuilds the wash protos");
        assert_eq!((c2.len(), s2.len()), (1, 1));

        // PROSE (no syn_lang, not markdown): zero rects — byte-identical render.
        p.set_view(&view("plain prose here\n", 0, 0));
        let (c3, s3, _h3) = p.wash_rects();
        assert!(c3.is_empty() && s3.is_empty(), "prose buffers carry no washes");
    }

    /// WASH O(visible): on a TALL code doc the per-frame wash pass emits only the
    /// visible band's quads (proto cull) — never one per document line.
    #[test]
    fn wash_rects_cull_to_visible_band() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping wash_rects_cull_to_visible_band: no wgpu adapter");
            return;
        };
        // 600 prose-comment lines: every line carries a wash PROTO, but the frame
        // must emit only the visible band (canvas rows + the generous margin).
        let text: String = (0..600).map(|i| format!("// prose note number {i}\n")).collect();
        let mut v = view(&text, 0, 0);
        v.syn_lang = Some(crate::syntax::Lang::Rust);
        p.set_view(&v);
        let (comments, _, _) = p.wash_rects();
        assert!(!comments.is_empty(), "the visible comments must wash");
        assert!(
            comments.len() < 150,
            "emitted wash quads must be bounded by the visible band, got {} of 600",
            comments.len()
        );
    }

    /// MARKDOWN FENCES inherit the washes through the SAME seam (decision 4):
    /// a ```rust fence's prose comment + string wash; markdown WITHOUT fences
    /// (and the fence's own surrounding prose) yields zero wash quads.
    #[test]
    fn markdown_fence_inherits_washes() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping markdown_fence_inherits_washes: no wgpu adapter");
            return;
        };
        let text = "prose before\n```rust\n// a calm fence note\nlet s = \"hi\";\n```\nprose after\n";
        let mut v = view(text, 0, 0);
        v.is_markdown = true;
        p.set_view(&v);
        let (comments, strings, highlights) = p.wash_rects();
        assert_eq!(comments.len(), 1, "the fence's prose comment washes: {comments:?}");
        assert_eq!(strings.len(), 1, "the fence's string washes: {strings:?}");
        assert!(highlights.is_empty(), "a fenced code block carries no highlight washes");

        // Markdown with NO fence: no washes at all (prose byte-identity).
        let mut v2 = view("# title\nplain prose paragraph\n", 0, 0);
        v2.is_markdown = true;
        p.set_view(&v2);
        let (c, s, h) = p.wash_rects();
        assert!(c.is_empty() && s.is_empty() && h.is_empty(), "fence-less markdown carries no washes");
    }

    /// MARKDOWN `==highlight==`: the marked text carries an `MdKind::Highlight`
    /// span (reported as `"highlight"` in the sidecar) and its wash quad rides
    /// its OWN dedicated HIGHLIGHT bucket + violet pipeline — DECOUPLED from the
    /// prose-comment wash (a deliberate, narrow break of the one-warm-wash owner
    /// so a highlighter POPS): the highlight produces exactly one quad in the
    /// third `wash_rects` slot and ZERO in the comment/string buckets. A
    /// `.rs`-style CODE buffer (`syn_lang` set, `is_markdown` false) with the
    /// identical `==` bytes — a comparison operator, never a highlight — carries
    /// NEITHER an `md_spans` entry nor an extra wash quad, because
    /// `markdown::spans` is never invoked at all off the `is_markdown` gate
    /// (`parse_doc_spans`).
    #[test]
    fn markdown_highlight_inherits_wash_and_code_buffers_never_match() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!(
                "skipping markdown_highlight_inherits_wash_and_code_buffers_never_match: no wgpu adapter"
            );
            return;
        };
        let text = "prose before ==marked text== prose after\n";
        let mut v = view(text, 0, 0);
        v.is_markdown = true;
        p.set_view(&v);
        let spans = p.md_report();
        assert!(
            spans.iter().any(|(s, e, t)| *s == 15 && *e == 26 && *t == "highlight"),
            "'marked text' (15..26) should be a highlight span: {spans:?}"
        );
        assert!(
            spans.iter().any(|(s, e, t)| *s == 13 && *e == 15 && *t == "markup"),
            "the opening '==' dims to markup: {spans:?}"
        );
        assert!(
            spans.iter().any(|(s, e, t)| *s == 26 && *e == 28 && *t == "markup"),
            "the closing '==' dims to markup: {spans:?}"
        );
        let (comments, strings, highlights) = p.wash_rects();
        assert_eq!(
            highlights.len(), 1,
            "the highlight rides its OWN dedicated highlight-wash bucket: {highlights:?}"
        );
        assert!(
            comments.is_empty(),
            "a highlight is DECOUPLED from the comment wash, never in its bucket: {comments:?}"
        );
        assert!(strings.is_empty(), "a highlight never touches the string bucket");

        // The IDENTICAL `==` bytes in a CODE buffer (a comparison operator, not a
        // highlight): no md spans at all, and consequently no extra wash quad.
        let code_text = "let ok = a ==marked text== b;\n";
        let mut vc = view(code_text, 0, 0);
        vc.is_markdown = false;
        vc.syn_lang = Some(crate::syntax::Lang::Rust);
        p.set_view(&vc);
        assert!(
            p.md_report().is_empty(),
            "a code buffer must never run the markdown highlight pass: {:?}",
            p.md_report()
        );
    }

    /// WYSIWYG (the PHILOSOPHY.md amendment): the four LINE-scoped conceal kinds
    /// — heading, emphasis, inline code, highlight — each conceal (transparent
    /// ink) when the caret is on a DIFFERENT line, and reveal independently the
    /// instant the caret lands on their own line, exactly mirroring the
    /// pre-existing hr/bullet reveal-on-cursor toggle.
    #[test]
    fn wysiwyg_conceals_each_line_scoped_kind_off_cursor_and_reveals_on() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(true);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping wysiwyg_conceals_each_line_scoped_kind_off_cursor_and_reveals_on: no wgpu adapter");
            return;
        };
        // Line 0: heading '#' at byte 0. Line 1: emphasis '**' at byte 0. Line 2:
        // inline-code backtick at byte 0. Line 3: highlight '==' at byte 0. Line 4
        // is a blank line the caret can sit on with NOTHING concealable on it.
        let text = "# Title\n**bold**\n`code`\n==mark==\n";
        let mut off = view(text, 4, 0);
        off.is_markdown = true;
        p.set_view(&off);
        assert!(p.concealed_at(0, 0), "heading '#' concealed off its own line");
        assert!(p.concealed_at(1, 0), "emphasis '**' concealed off its own line");
        assert!(p.concealed_at(2, 0), "inline-code backtick concealed off its own line");
        assert!(p.concealed_at(3, 0), "highlight '==' concealed off its own line");

        // Caret on the HEADING line: only it reveals; the other three stay concealed.
        let mut on0 = view(text, 0, 0);
        on0.is_markdown = true;
        p.set_view(&on0);
        assert!(!p.concealed_at(0, 0), "caret on the heading line reveals its '#'");
        assert!(p.concealed_at(1, 0), "emphasis stays concealed (caret elsewhere)");
        assert!(p.concealed_at(2, 0), "code stays concealed (caret elsewhere)");
        assert!(p.concealed_at(3, 0), "highlight stays concealed (caret elsewhere)");

        // Caret on the EMPHASIS line: only it reveals now; the heading re-conceals.
        let mut on1 = view(text, 1, 0);
        on1.is_markdown = true;
        p.set_view(&on1);
        assert!(p.concealed_at(0, 0), "heading re-conceals once the caret leaves");
        assert!(!p.concealed_at(1, 0), "caret on the emphasis line reveals its '**'");
        assert!(p.concealed_at(2, 0), "code stays concealed");
        assert!(p.concealed_at(3, 0), "highlight stays concealed");

        crate::markdown::set_wysiwyg_on(true);
    }

    /// WYSIWYG FENCE (BLOCK-scoped): a fenced code block's marker lines (the
    /// info-string line + the closing fence) conceal when the caret is OUTSIDE
    /// the whole block, and reveal together the instant the caret lands
    /// ANYWHERE inside it — including on a BODY line, which itself is NEVER
    /// concealed regardless of caret position (it carries its own `Code`
    /// coloring, never blanked).
    #[test]
    fn wysiwyg_fence_markers_are_block_scoped_body_never_conceals() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(true);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping wysiwyg_fence_markers_are_block_scoped_body_never_conceals: no wgpu adapter");
            return;
        };
        // line0 "prose", line1 "```rust" (open+info), line2 body, line3 "```"
        // (close), line4 "more".
        let text = "prose\n```rust\nlet x = 1;\n```\nmore\n";
        let mut outside = view(text, 0, 0);
        outside.is_markdown = true;
        p.set_view(&outside);
        assert!(p.concealed_at(1, 0), "fence open+info concealed with caret outside the block");
        assert!(p.concealed_at(3, 0), "fence close concealed with caret outside the block");
        assert!(!p.concealed_at(2, 0), "a body line must NEVER conceal");

        // Caret on the BODY line (line 2, inside the block): BOTH marker lines
        // reveal together, and the body line still never conceals.
        let mut inside_body = view(text, 2, 0);
        inside_body.is_markdown = true;
        p.set_view(&inside_body);
        assert!(!p.concealed_at(1, 0), "fence open+info reveals: caret is inside the block");
        assert!(!p.concealed_at(3, 0), "fence close reveals: caret is inside the block");
        assert!(!p.concealed_at(2, 0), "the body line still never conceals");

        // Caret AFTER the block (line 4): both markers re-conceal.
        let mut after = view(text, 4, 0);
        after.is_markdown = true;
        p.set_view(&after);
        assert!(p.concealed_at(1, 0), "fence open+info re-conceals once the caret leaves the block");
        assert!(p.concealed_at(3, 0), "fence close re-conceals once the caret leaves the block");

        crate::markdown::set_wysiwyg_on(true);
    }

    /// WYSIWYG FRONTMATTER (BLOCK-scoped, reuses the Fence seam verbatim): a
    /// `---`-delimited frontmatter block conceals wholesale when the caret is
    /// OUTSIDE it and reveals wholesale the instant the caret lands ANYWHERE
    /// inside it — no per-line body carve-out (unlike Fence, a frontmatter
    /// block has no highlighted body, so the whole thing is markup).
    #[test]
    fn wysiwyg_frontmatter_is_block_scoped_like_fence() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(true);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping wysiwyg_frontmatter_is_block_scoped_like_fence: no wgpu adapter");
            return;
        };
        // line0 "---", line1 "lang: ja", line2 "---", line3 "# Title", line4 body.
        let text = "---\nlang: ja\n---\n# Title\nbody\n";
        let mut outside = view(text, 3, 0);
        outside.is_markdown = true;
        p.set_view(&outside);
        assert!(p.concealed_at(0, 0), "opening --- concealed with caret outside the block");
        assert!(p.concealed_at(1, 0), "lang: ja concealed with caret outside the block");
        assert!(p.concealed_at(2, 0), "closing --- concealed with caret outside the block");

        // Caret INSIDE the block (line 1): the whole block reveals together.
        let mut inside = view(text, 1, 0);
        inside.is_markdown = true;
        p.set_view(&inside);
        assert!(!p.concealed_at(0, 0), "opening --- reveals: caret is inside the block");
        assert!(!p.concealed_at(1, 0), "lang: ja reveals: caret is inside the block");
        assert!(!p.concealed_at(2, 0), "closing --- reveals: caret is inside the block");

        // Caret back outside (line 4, the body): re-conceals.
        let mut after = view(text, 4, 0);
        after.is_markdown = true;
        p.set_view(&after);
        assert!(p.concealed_at(0, 0), "re-conceals once the caret leaves the block");
        assert!(p.concealed_at(2, 0), "re-conceals once the caret leaves the block");

        crate::markdown::set_wysiwyg_on(true);
    }

    /// WYSIWYG OFF (`wysiwyg = false`): a total no-op — every concealable span
    /// stays REVEALED (plain dim `Markup`-like styling, exactly the pre-round
    /// always-visible markup) regardless of the caret, and the value-step
    /// PANEL/PILL washes upload zero geometry, reproducing today's rendering
    /// byte-identically.
    #[test]
    fn wysiwyg_off_never_conceals_and_uploads_no_wash_geometry() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(false);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping wysiwyg_off_never_conceals_and_uploads_no_wash_geometry: no wgpu adapter");
            return;
        };
        let text = "# Title\n**bold**\n`code`\n==mark==\nprose\n```rust\nlet x = 1;\n```\nmore\n";
        // Caret nowhere near any concealable line — with WYSIWYG on this would
        // conceal everything; with it OFF, nothing ever conceals.
        let mut v = view(text, 4, 0);
        v.is_markdown = true;
        p.set_view(&v);
        assert!(!p.concealed_at(0, 0), "wysiwyg=false: heading never conceals");
        assert!(!p.concealed_at(1, 0), "wysiwyg=false: emphasis never conceals");
        assert!(!p.concealed_at(2, 0), "wysiwyg=false: inline code never conceals");
        assert!(!p.concealed_at(3, 0), "wysiwyg=false: highlight never conceals");
        assert!(!p.concealed_at(5, 0), "wysiwyg=false: fence open never conceals");
        assert!(!p.concealed_at(7, 0), "wysiwyg=false: fence close never conceals");
        assert!(p.code_pill_rects().is_empty(), "wysiwyg=false: no inline-code pill geometry");
        assert!(p.fence_panel_rects().is_empty(), "wysiwyg=false: no fence-panel geometry");

        crate::markdown::set_wysiwyg_on(true);
    }

    /// WYSIWYG WASH GEOMETRY: the inline-code PILL and the fenced-code PANEL each
    /// upload non-empty geometry when WYSIWYG is on and the buffer has the
    /// matching construct — the panel spans EVERY visual row of the block
    /// (fence lines AND body), MERGED into ONE continuous quad from block top
    /// to block bottom (`merge_row_bands` — the live-review fix for the panel
    /// reading as separate striped rows; see its doc comment for the shader
    /// seam antialiasing reason a per-row panel looked broken even though the
    /// underlying row geometry was already mathematically contiguous).
    #[test]
    fn wysiwyg_pill_and_panel_rects_present_when_on() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(true);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping wysiwyg_pill_and_panel_rects_present_when_on: no wgpu adapter");
            return;
        };
        let text = "prose with `inline code` here\n\n```rust\nlet x = 1;\nlet y = 2;\n```\n";
        let mut v = view(text, 0, 0);
        v.is_markdown = true;
        p.set_view(&v);
        let pills = p.code_pill_rects();
        assert_eq!(pills.len(), 1, "one inline-code span => one pill quad: {pills:?}");
        let panels = p.fence_panel_rects();
        // 4 visual rows in the block (the open+info line, the two body lines,
        // and the closing fence line) MERGE into exactly one continuous card —
        // no internal seam between rows.
        assert_eq!(panels.len(), 1, "the whole block merges into one panel quad: {panels:?}");
        let expected_h = 4.0 * p.metrics.line_height;
        assert!(
            (panels[0][3] - expected_h).abs() < 1.0,
            "the merged panel spans all 4 rows' combined height: {panels:?} vs {expected_h}"
        );

        crate::markdown::set_wysiwyg_on(true);
    }

    // --- INLINE IMAGES: parse + layout (the markdown/layout phase) ------------

    /// The pure reveal decision for an IMAGE conceal is LINE-scoped, exactly like
    /// heading/emphasis: reveal (show source) iff the caret is on the image's own
    /// line; conceal (draw image) otherwise.
    #[test]
    fn wysiwyg_reveals_image_is_line_scoped() {
        use crate::markdown::ConcealKind;
        let range = 5..30;
        // off-cursor (caret on a DIFFERENT line) -> conceal the source.
        assert!(!super::spans::wysiwyg_reveals(ConcealKind::Image, true, 0, &range));
        // on-cursor (caret on THIS line) -> reveal the raw `![alt](path)` source.
        assert!(super::spans::wysiwyg_reveals(ConcealKind::Image, false, 10, &range));
    }

    /// A link's `[`/`](url)` plumbing is LINE-scoped, exactly like emphasis /
    /// headings / images: concealed off its own line, revealed on it.
    #[test]
    fn wysiwyg_reveals_link_is_line_scoped() {
        use crate::markdown::ConcealKind;
        let range = 4..25;
        assert!(!super::spans::wysiwyg_reveals(ConcealKind::Link, true, 0, &range));
        assert!(super::spans::wysiwyg_reveals(ConcealKind::Link, false, 10, &range));
    }

    /// END-TO-END WYSIWYG links: off the caret's line the `[`/`](url)` plumbing
    /// conceals to transparent (zero-width) ink while the link TEXT stays visible
    /// content ink — so `see [the essay](http://x) now` reads as `see the essay
    /// now`; on the caret's own line the whole source reveals for editing. Asserted
    /// through the shared `concealed_at` conceal-state reader.
    #[test]
    fn wysiwyg_link_plumbing_conceals_off_cursor_text_stays_visible() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(true);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping wysiwyg_link_plumbing_conceals: no wgpu adapter");
            return;
        };
        // Line 0: `see [the essay](http://x) now`. Byte 4 = `[`, bytes 5..14 =
        // `the essay` (link text), bytes 14..25 = `](http://x)` tail, 16 in the url.
        let text = "see [the essay](http://x) now\nprose\n";
        // Caret on line 1 (prose): line 0's link plumbing conceals.
        let mut off = view(text, 1, 0);
        off.is_markdown = true;
        p.set_view(&off);
        assert!(p.concealed_at(0, 4), "opening '[' concealed off the line");
        assert!(p.concealed_at(0, 16), "the url inside the tail concealed off the line");
        assert!(
            !p.concealed_at(0, 8),
            "the link TEXT stays visible (never concealed)"
        );

        // Caret ON line 0: the whole `[text](url)` source reveals for editing.
        let mut on = view(text, 0, 0);
        on.is_markdown = true;
        p.set_view(&on);
        assert!(!p.concealed_at(0, 4), "caret on the link line reveals '['");
        assert!(!p.concealed_at(0, 16), "caret on the link line reveals the url");

        crate::markdown::set_wysiwyg_on(true);
    }

    /// The pure fit-to-column display-size math: never wider than the column,
    /// aspect preserved, an optional width hint replacing the intrinsic width.
    /// `max_h = 0.0` disables the viewport-height cap (see the dedicated
    /// `image_display_size_caps_at_the_viewport_height` test below for that half).
    #[test]
    fn image_display_size_fits_to_column_and_preserves_aspect() {
        // 120x48 (aspect 2.5), wide column -> full intrinsic, height = 120/2.5 = 48.
        let (w, h) = super::spans::image_display_size(120, 48, None, 1000.0, 0.0);
        assert!((w - 120.0).abs() < 0.1 && (h - 48.0).abs() < 0.1, "{w}x{h}");
        // Narrow column clamps width AND scales height with it.
        let (w2, h2) = super::spans::image_display_size(120, 48, None, 60.0, 0.0);
        assert!((w2 - 60.0).abs() < 0.1 && (h2 - 24.0).abs() < 0.1, "{w2}x{h2}");
        // A `|300` hint upsizes toward 300 but stays clamped to the column.
        let (w3, _) = super::spans::image_display_size(120, 48, Some(300), 1000.0, 0.0);
        assert!((w3 - 300.0).abs() < 0.1, "hint sets width: {w3}");
        let (w4, _) = super::spans::image_display_size(120, 48, Some(300), 200.0, 0.0);
        assert!((w4 - 200.0).abs() < 0.1, "hint still clamped to column: {w4}");
    }

    /// The viewport-height cap: a huge-native-size (retina-paste-shaped) image's
    /// display HEIGHT never exceeds `max_h`, and its width shrinks PROPORTIONALLY
    /// (the aspect never distorts) — the "full-bleed wall" fix.
    #[test]
    fn image_display_size_caps_at_the_viewport_height() {
        // A tall retina paste: 2241x4000 (aspect ~0.56), a generous wide column so
        // fit-to-column alone would draw it near-full native size.
        let (w, h) = super::spans::image_display_size(2241, 4000, None, 2000.0, 500.0);
        assert!((h - 500.0).abs() < 0.1, "height pinned to the cap: {h}");
        // Width follows the SAME scale factor the height was cut by (500/4000).
        let expected_w = 2241.0 * (500.0 / 4000.0);
        assert!((w - expected_w).abs() < 0.5, "width scales proportionally: {w} vs {expected_w}");
        // A short-and-wide image well under the cap is untouched by it.
        let (w2, h2) = super::spans::image_display_size(1200, 480, None, 2000.0, 500.0);
        assert!((w2 - 1200.0).abs() < 0.1 && (h2 - 480.0).abs() < 0.1, "under the cap, unchanged: {w2}x{h2}");
        // A non-positive max_h disables the cap outright (the "window height not
        // known yet" escape hatch).
        let (w3, h3) = super::spans::image_display_size(2241, 4000, None, 2000.0, 0.0);
        assert!((w3 - 2000.0).abs() < 0.1 && (h3 - 3570.7).abs() < 1.0, "cap disabled: {w3}x{h3}");
    }

    /// END-TO-END: an `![alt](img.png)` line reserves a TALL row equal to the
    /// bundled fixture's fit-to-column DISPLAY height (120x48 -> 48px) via the
    /// same variable-row-height machinery headings use; off the caret's line the
    /// source CONCEALS (zero-width) and on the caret's line it REVEALS at full
    /// width. CAPTION MODEL (re-decided 2026-07-09): the caret's own image row
    /// height is UNCHANGED on reveal (stays the image height `h` = 48) — ZERO
    /// reflow — and the revealed body-size source renders CENTRED OVER the
    /// still-drawn, dimmed image. Fixture: `samples/tiny.png`.
    #[test]
    fn inline_image_reserves_tall_row_and_reveals_source_on_cursor() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _pg = crate::page::test_lock();
        let prev = crate::markdown::inline_images_on();
        crate::markdown::set_inline_images_on(true);
        crate::markdown::set_wysiwyg_on(true);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping inline_image_reserves_tall_row: no wgpu adapter");
            crate::markdown::set_inline_images_on(prev);
            return;
        };
        // `doc_dir` is None (the `view` helper), so the relative path resolves
        // against the test cwd (the crate root) — `samples/tiny.png` is 120x48.
        let text = "![pic](samples/tiny.png)\nprose here\n";
        // Caret on line 1 (prose): line 0's image source conceals, the tall row shows.
        let mut v = view(text, 1, 0);
        v.is_markdown = true;
        p.set_view(&v);
        let rows0 = p.visual_rows(0);
        let h = rows0[0].line_height;
        assert!((h - 48.0).abs() < 2.0, "image row reserves the 48px display height: {h}");
        let xs = &rows0[0].xs;
        let total = xs.last().copied().unwrap_or(0.0) - xs.first().copied().unwrap_or(0.0);
        assert!(total < 2.0, "off-cursor image source collapses to ~0 width: {total} ({xs:?})");
        let report = p.images_report();
        assert_eq!(report.len(), 1, "one image reported: {report:?}");
        assert!(!report[0].missing, "the bundled fixture reads: {report:?}");
        assert!(!report[0].revealed, "caret off the image line: {report:?}");
        assert!(
            (report[0].display_h - 48.0).abs() < 1.0 && (report[0].display_w - 120.0).abs() < 1.0,
            "report carries the fit-to-column size: {report:?}"
        );

        // Caret ON line 0: the source reveals at full width, but the row height is
        // UNCHANGED (still 48, the image height) — the caption model reflows
        // nothing; the source just renders centred over the dimmed image.
        let mut v0 = view(text, 0, 0);
        v0.is_markdown = true;
        p.set_view(&v0);
        let rows0b = p.visual_rows(0);
        assert!(
            (rows0b[0].line_height - 48.0).abs() < 2.0,
            "CAPTION MODEL: the revealed image row height is UNCHANGED (still 48, no grow): {}",
            rows0b[0].line_height
        );
        let xs2 = &rows0b[0].xs;
        let total2 = xs2.last().copied().unwrap_or(0.0) - xs2.first().copied().unwrap_or(0.0);
        assert!(total2 > 20.0, "on-cursor the image source reveals at full width: {total2}");
        assert!(p.images_report()[0].revealed, "caret on the image line reveals it");
        // CARET SIZE: the caret sizes to the body-size SOURCE (scale 1.0), NOT the
        // tall reserved row — a row-scaled caret balloons to the whole image row.
        // `caret_cell_top` centres the body-height caret in the h-tall row, exactly
        // where cosmic-text centres the source glyphs, so it lands on the caption.
        assert!(
            (p.cursor_scale() - 1.0).abs() < 1e-6,
            "caret on an image line is body-size (scale 1.0), never the tall row: {}",
            p.cursor_scale()
        );

        crate::markdown::set_inline_images_on(prev);
    }

    /// FIX (2026-07-09): selecting chars on a REVEALED image line must draw a
    /// BODY-height selection band — the SAME height the caret draws there — NOT a
    /// char-wide × whole-image-height PILLAR (the reported selection bug). The caret
    /// was already pinned to the caption text (`cursor_scale` → 1.0 on an image
    /// line) but the selection / squiggle row-bands still sized to the tall image
    /// row; both now share the ONE owner [`TextPipeline::caret_band_scale`]. Mirrors
    /// the caret test above. Fixture: `samples/tiny.png` (120×48 → a 48px row).
    #[test]
    fn selection_on_image_line_is_body_height_not_the_image_pillar() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _pg = crate::page::test_lock();
        let prev = crate::markdown::inline_images_on();
        crate::markdown::set_inline_images_on(true);
        crate::markdown::set_wysiwyg_on(true);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping selection_on_image_line_is_body_height_not_the_image_pillar: no wgpu adapter");
            crate::markdown::set_inline_images_on(prev);
            return;
        };
        let text = "![pic](samples/tiny.png)\nprose here\n";
        // Caret ON the image line so its source reveals at full width; select 4 chars.
        let mut v = view(text, 0, 4);
        v.is_markdown = true;
        v.selection = Some(((0, 0), (0, 4)));
        p.set_view(&v);
        let img_h = p.visual_rows(0)[0].line_height;
        assert!(img_h > 30.0, "image row reserves the tall display height: {img_h}");
        let sel = p.selection_rects();
        assert!(!sel.is_empty(), "selection on the revealed image line produces a rect: {sel:?}");
        let band_h = sel[0][3];
        let caret_h = p.metrics.caret_h;
        // BODY height (the caret's own band), never the tall image row => no pillar.
        assert!(
            (band_h - caret_h).abs() < 0.5,
            "image-line selection band is body caret height ({caret_h}), not the image pillar: {band_h} (row {img_h})"
        );
        assert!(
            band_h < img_h * 0.6,
            "selection band is far shorter than the image row (no pillar): {band_h} vs {img_h}"
        );
        // And it matches a PROSE line's selection band exactly (the same body anchor).
        let mut vp = view(text, 1, 4);
        vp.is_markdown = true;
        vp.selection = Some(((1, 0), (1, 4)));
        p.set_view(&vp);
        let prose = p.selection_rects();
        assert!(!prose.is_empty(), "prose-line selection produces a rect: {prose:?}");
        assert!(
            (prose[0][3] - band_h).abs() < 0.5,
            "image-line band == prose-line band (both body caret height): {} vs {band_h}",
            prose[0][3]
        );
        crate::markdown::set_inline_images_on(prev);
    }

    /// CAPTION MODEL (settled `df773ba`): the image is DRAWN on every line now —
    /// caret-on-line only floats the raw source as a caption overlay, it no longer
    /// hides the drawn image — so the resize handles must arm REGARDLESS of caret
    /// position. This supersedes the old images-v2 reveal-hides-the-image model's
    /// `im.revealed` exclusion in `image_hit_rects` (dead code once the caption
    /// model landed, since a revealed image is a drawn image too). Fixture:
    /// `samples/tiny.png`.
    #[test]
    fn revealed_images_still_arm_resize_handles() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _pg = crate::page::test_lock();
        let prev = crate::markdown::inline_images_on();
        crate::markdown::set_inline_images_on(true);
        crate::markdown::set_wysiwyg_on(true);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping revealed_images_still_arm_resize_handles: no wgpu adapter");
            crate::markdown::set_inline_images_on(prev);
            return;
        };
        let text = "![pic](samples/tiny.png)\nprose here\n";
        // Caret OFF the image line: exactly one hit rect, as expected off-reveal.
        let mut v_off = view(text, 1, 0);
        v_off.is_markdown = true;
        p.set_view(&v_off);
        let rects_off = p.image_hit_rects();
        assert_eq!(rects_off.len(), 1, "off-cursor: the drawn image arms a handle target: {rects_off:?}");

        // Caret ON the image line (the image REVEALS its source as a caption): the
        // handle target is STILL present — same byte range, same on-screen rect —
        // since the image itself is still drawn underneath the caption.
        let mut v_on = view(text, 0, 0);
        v_on.is_markdown = true;
        p.set_view(&v_on);
        assert!(p.images_report()[0].revealed, "caret on the image line reveals it");
        let rects_on = p.image_hit_rects();
        assert_eq!(
            rects_on.len(),
            1,
            "REVEALED: the handle target survives caret-on-line (the caption model draws the image regardless): {rects_on:?}"
        );
        assert_eq!(rects_off[0].0, rects_on[0].0, "same image byte range either way");
        crate::markdown::set_inline_images_on(prev);
    }

    /// IMAGES OFF: the `![alt](path)` line keeps a NORMAL-height row, emits no
    /// image report, and its source renders as plain full-width text — byte-
    /// identical to the pre-feature editor.
    #[test]
    fn inline_images_off_keeps_normal_row_and_no_report() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _pg = crate::page::test_lock();
        let prev = crate::markdown::inline_images_on();
        crate::markdown::set_inline_images_on(false);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping inline_images_off: no wgpu adapter");
            crate::markdown::set_inline_images_on(prev);
            return;
        };
        let text = "![pic](samples/tiny.png)\nprose\n";
        let mut v = view(text, 1, 0);
        v.is_markdown = true;
        p.set_view(&v);
        let rows0 = p.visual_rows(0);
        assert!(
            (rows0[0].line_height - p.metrics.line_height).abs() < 1.0,
            "images OFF: the image line keeps a normal-height row: {}",
            rows0[0].line_height
        );
        assert!(p.images_report().is_empty(), "images OFF: nothing reported");
        let xs = &rows0[0].xs;
        let total = xs.last().copied().unwrap_or(0.0) - xs.first().copied().unwrap_or(0.0);
        assert!(total > 20.0, "images OFF: source renders as plain full-width text: {total}");
        crate::markdown::set_inline_images_on(prev);
    }

    /// WYSIWYG OFF byte-identity guard: with inline images ON but WYSIWYG OFF there
    /// is no reveal/conceal model at all — the image row is exactly the image height
    /// `h` (48, same as the caption model) AND the source shows UNCONCEALED at full
    /// width whether or not the caret is on it. The caption model never grows the
    /// row, so `h` matches on-caret too; the distinguishing off-state fact is the
    /// unconcealed source. Fixture: `samples/tiny.png`.
    #[test]
    fn wysiwyg_off_image_line_does_not_grow_on_reveal() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _pg = crate::page::test_lock();
        let prev = crate::markdown::inline_images_on();
        let prevw = crate::markdown::wysiwyg_on();
        crate::markdown::set_inline_images_on(true);
        crate::markdown::set_wysiwyg_on(false);
        let restore = || {
            crate::markdown::set_inline_images_on(prev);
            crate::markdown::set_wysiwyg_on(prevw);
        };
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping wysiwyg_off_image_line: no wgpu adapter");
            restore();
            return;
        };
        let text = "![pic](samples/tiny.png)\nprose\n";
        // Caret OFF the image line (line 1): with WYSIWYG off the source is NOT
        // concealed — it shows at full width even though the caret is elsewhere.
        let mut voff = view(text, 1, 0);
        voff.is_markdown = true;
        p.set_view(&voff);
        let rows_off = p.visual_rows(0);
        assert!(
            (rows_off[0].line_height - 48.0).abs() < 2.0,
            "WYSIWYG off: the image row is h (48): {}",
            rows_off[0].line_height
        );
        let xs_off = &rows_off[0].xs;
        let total_off = xs_off.last().copied().unwrap_or(0.0) - xs_off.first().copied().unwrap_or(0.0);
        assert!(
            total_off > 20.0,
            "WYSIWYG off: the source shows UNCONCEALED (full width) off the caret line: {total_off}"
        );
        // Caret ON the image line 0 — the row is still h (48), never grows.
        let mut v = view(text, 0, 0);
        v.is_markdown = true;
        p.set_view(&v);
        let rows0 = p.visual_rows(0);
        assert!(
            (rows0[0].line_height - 48.0).abs() < 2.0,
            "WYSIWYG off: the caret's image row stays h (48), never grows: {}",
            rows0[0].line_height
        );
        restore();
    }

    /// HIT-TEST across a REVEALED image row (the `h`-tall row, source shown at body
    /// size CENTRED in it — the caption model): a full-width x sweep at the row's
    /// vertical centre always resolves to logical line 0 and an in-bounds column,
    /// AND the sweep still discriminates (more than one distinct column), so the
    /// revealed caption stays clickable. Fixture: `samples/tiny.png`.
    #[test]
    fn revealed_image_row_hit_test_stays_in_bounds() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _pg = crate::page::test_lock();
        let prev = crate::markdown::inline_images_on();
        let prevw = crate::markdown::wysiwyg_on();
        crate::markdown::set_inline_images_on(true);
        crate::markdown::set_wysiwyg_on(true);
        let restore = || {
            crate::markdown::set_inline_images_on(prev);
            crate::markdown::set_wysiwyg_on(prevw);
        };
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping revealed_image_row_hit_test: no wgpu adapter");
            restore();
            return;
        };
        let src = "![pic](samples/tiny.png)";
        let text = format!("{src}\nprose here\n");
        let char_count = src.chars().count();
        // Caret ON line 0: the source reveals in the grown row.
        let mut v0 = view(&text, 0, 0);
        v0.is_markdown = true;
        p.set_view(&v0);
        let rows0 = p.visual_rows(0);
        let row_h = rows0[0].line_height;
        // Vertical centre of the revealed row (where cosmic-text centres the source).
        let py = p.line_ornament_top(0) + row_h * 0.5;
        let left = p.text_left();
        let wrap = p.text_wrap_width();
        let mut cols = std::collections::BTreeSet::new();
        let steps = 48;
        for i in 0..=steps {
            let px = left + wrap * (i as f32 / steps as f32);
            let (line, col) = p.hit_test(px, py, 0);
            assert_eq!(line, 0, "every click on the revealed image row lands on line 0");
            assert!(
                col <= char_count,
                "hit column {col} stays within the source's {char_count} chars"
            );
            cols.insert(col);
        }
        assert!(
            cols.len() > 3,
            "the x sweep discriminates columns on the revealed source: {cols:?}"
        );
        restore();
    }

    /// A headless pipeline PLUS its device/queue, so a test can drive the full
    /// `prepare` frame (the image draw's instance counts are only set there). `None`
    /// on a GPU-less machine (skip).
    fn headless_pipeline_dq() -> Option<(wgpu::Device, wgpu::Queue, TextPipeline)> {
        pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .ok()?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("awl image-draw test device"),
                    ..Default::default()
                })
                .await
                .ok()?;
            let cache = Cache::new(&device);
            let mut p =
                TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
            p.set_size(1200.0, 800.0);
            Some((device, queue, p))
        })
    }

    /// GPU DRAW: an OFF-CURSOR image on a visible line decodes the bundled fixture
    /// and draws exactly ONE image quad (no placeholder) and NO caption scrim;
    /// moving the caret ONTO the image line REVEALS the source but the image STAYS
    /// DRAWN (dimmed, UNMOVED — the caption model, source centred over it) and a
    /// caption SCRIM band appears behind the revealed source. Fixture:
    /// `samples/tiny.png`.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn inline_image_off_cursor_draws_one_quad_and_stays_drawn_when_revealed() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _pg = crate::page::test_lock();
        if std::fs::metadata("samples/tiny.png").is_err() {
            eprintln!("skipping: samples/tiny.png fixture not present");
            return;
        }
        let prev = crate::markdown::inline_images_on();
        let prevw = crate::markdown::wysiwyg_on();
        crate::markdown::set_inline_images_on(true);
        crate::markdown::set_wysiwyg_on(true);
        let restore = || {
            crate::markdown::set_inline_images_on(prev);
            crate::markdown::set_wysiwyg_on(prevw);
        };
        let Some((device, queue, mut p)) = headless_pipeline_dq() else {
            eprintln!("skipping inline_image_off_cursor_draws_one_quad: no wgpu adapter");
            restore();
            return;
        };
        let text = "![pic](samples/tiny.png)\nprose here\n";
        // Caret on line 1 (prose) — the image on line 0 is off-cursor + visible.
        let mut v = view(text, 1, 0);
        v.is_markdown = true;
        p.set_view(&v);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        assert_eq!(p.image_pipeline.instance_count(), 1, "one image quad drawn off-cursor");
        assert_eq!(
            p.image_placeholder_pipeline.instance_count(),
            0,
            "a readable fixture draws NO placeholder"
        );
        assert_eq!(
            p.image_scrim_pipeline.instance_count(),
            0,
            "off-cursor: no caption scrim (the source is concealed)"
        );

        // Caret ON the image line — the source reveals, but the image STAYS DRAWN
        // (dimmed, UNMOVED — the caption model): still one quad. A caption SCRIM
        // band now backs the revealed source (at least one band; a wrapped source
        // could produce more).
        let mut v0 = view(text, 0, 0);
        v0.is_markdown = true;
        p.set_view(&v0);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        assert_eq!(
            p.image_pipeline.instance_count(),
            1,
            "the image stays drawn (dimmed) when its source line is revealed"
        );
        assert!(
            p.image_scrim_pipeline.instance_count() >= 1,
            "revealed: a caption scrim band backs the source: {}",
            p.image_scrim_pipeline.instance_count()
        );
        restore();
    }

    /// GPU DRAW: a MISSING-file image draws the calm rounded PLACEHOLDER quad (one),
    /// and NO image quad — a missing image is a calm state, never an error.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn inline_image_missing_file_draws_placeholder_not_quad() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _pg = crate::page::test_lock();
        let prev = crate::markdown::inline_images_on();
        let prevw = crate::markdown::wysiwyg_on();
        crate::markdown::set_inline_images_on(true);
        crate::markdown::set_wysiwyg_on(true);
        let restore = || {
            crate::markdown::set_inline_images_on(prev);
            crate::markdown::set_wysiwyg_on(prevw);
        };
        let Some((device, queue, mut p)) = headless_pipeline_dq() else {
            eprintln!("skipping inline_image_missing_file_draws_placeholder: no wgpu adapter");
            restore();
            return;
        };
        let text = "![a caption](does-not-exist-awl.png)\nprose\n";
        let mut v = view(text, 1, 0);
        v.is_markdown = true;
        p.set_view(&v);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        let report = p.images_report();
        assert_eq!(report.len(), 1, "one image reported: {report:?}");
        assert!(report[0].missing, "the absent file is reported missing: {report:?}");
        assert_eq!(
            p.image_placeholder_pipeline.instance_count(),
            1,
            "the missing image draws exactly one placeholder card"
        );
        assert_eq!(
            p.image_pipeline.instance_count(),
            0,
            "a missing image draws NO textured quad"
        );
        restore();
    }

    /// A NON-IMAGE markdown buffer draws neither an image quad nor a placeholder —
    /// byte-identical to the pre-feature editor at the GPU layer.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn no_image_buffer_draws_neither_quad_nor_placeholder() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _pg = crate::page::test_lock();
        let prev = crate::markdown::inline_images_on();
        crate::markdown::set_inline_images_on(true);
        let Some((device, queue, mut p)) = headless_pipeline_dq() else {
            eprintln!("skipping no_image_buffer_draws_neither: no wgpu adapter");
            crate::markdown::set_inline_images_on(prev);
            return;
        };
        let mut v = view("# heading\n\nplain prose only\n", 0, 0);
        v.is_markdown = true;
        p.set_view(&v);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        assert_eq!(p.image_pipeline.instance_count(), 0, "no images: no quad");
        assert_eq!(
            p.image_placeholder_pipeline.instance_count(),
            0,
            "no images: no placeholder"
        );
        crate::markdown::set_inline_images_on(prev);
    }

    // --- WYSIWYG v1.1: TRUE ZERO-WIDTH conceal (the live-review headline fix) --

    /// GHOST SPACING is gone: a concealed heading's `"# "` collapses to ~0
    /// advance, so the title starts FLUSH at the column edge (not indented by
    /// the markup's natural width), and a concealed emphasis pair collapses to
    /// a SINGLE normal word-space between the words on either side — not the
    /// "almost  italics" double-gap v1 shipped. Compares the concealed line's
    /// `VisualRow::xs` (per-char pixel boundaries) against a PLAIN reference
    /// buffer carrying the identical visible characters with no markup at all;
    /// zero-width conceal must make the two indistinguishable.
    #[test]
    fn wysiwyg_zero_width_conceal_collapses_heading_indent_and_emphasis_gap() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(true);
        let Some(mut p) = headless_pipeline() else {
            eprintln!(
                "skipping wysiwyg_zero_width_conceal_collapses_heading_indent_and_emphasis_gap: no wgpu adapter"
            );
            return;
        };

        // --- Heading: "# Title" with the caret on a DIFFERENT line (line 2),
        // so line 0's "# " markup conceals. ---
        let heading_text = "# Title\nprose\nmore prose\n";
        let mut v = view(heading_text, 2, 0);
        v.is_markdown = true;
        p.set_view(&v);
        let rows = p.visual_rows(0);
        let xs = &rows[0].xs;
        // "T" (byte/char col 2, right after the concealed "# ") sits at ~0 —
        // flush at the column edge, not indented by the hash+space's natural
        // width (which would be several pixels).
        assert!(
            xs[2] < 1.0,
            "concealed '# ' collapses to near-zero advance, title starts flush: xs={xs:?}"
        );

        // --- Emphasis: "almost *italics* end" concealed vs the IDENTICAL
        // visible text with no markup at all — the gap between "almost" and
        // "italics" must match a plain single space exactly. ---
        let concealed_text = "almost *italics* end\nprose\n";
        let mut vc = view(concealed_text, 1, 0); // caret on line 1: line 0 conceals
        vc.is_markdown = true;
        p.set_view(&vc);
        let rows_c = p.visual_rows(0);
        let xs_c = &rows_c[0].xs;
        // col 6 = end of "almost" (before the space); col 8 = start of "italics"
        // (right after the concealed '*' at col 7).
        let concealed_gap = xs_c[8] - xs_c[6];

        let plain_text = "almost italics end\nprose\n";
        let mut vp = view(plain_text, 1, 0);
        vp.is_markdown = true;
        p.set_view(&vp);
        let rows_p = p.visual_rows(0);
        let xs_p = &rows_p[0].xs;
        // col 6 = end of "almost"; col 7 = start of "italics" (one real space
        // apart, no markup at all).
        let plain_gap = xs_p[7] - xs_p[6];

        assert!(
            (concealed_gap - plain_gap).abs() < 1.0,
            "concealed '*' collapses so the word-gap matches a plain single space: \
             concealed={concealed_gap} plain={plain_gap} (xs_c={xs_c:?} xs_p={xs_p:?})"
        );

        crate::markdown::set_wysiwyg_on(true);
    }

    /// The accepted REVEAL-REFLOW cost: the instant the caret enters a
    /// concealed line, its markup reveals at FULL width again (the Obsidian
    /// behavior this round's spec explicitly accepted) — proving the
    /// zero-width collapse is reveal-gated, not a permanent layout change.
    #[test]
    fn wysiwyg_zero_width_conceal_reveals_full_width_when_caret_enters_line() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(true);
        let Some(mut p) = headless_pipeline() else {
            eprintln!(
                "skipping wysiwyg_zero_width_conceal_reveals_full_width_when_caret_enters_line: no wgpu adapter"
            );
            return;
        };
        let text = "# Title\nprose\n";

        // Caret elsewhere: concealed, title flush at ~0.
        let mut off = view(text, 1, 0);
        off.is_markdown = true;
        p.set_view(&off);
        let xs_off = p.visual_rows(0)[0].xs.clone();
        assert!(xs_off[2] < 1.0, "concealed off-cursor: flush: {xs_off:?}");

        // Caret ON the heading line: reveals at full (real) width — "# " keeps
        // its natural several-pixel advance again.
        let mut on = view(text, 0, 0);
        on.is_markdown = true;
        p.set_view(&on);
        let xs_on = p.visual_rows(0)[0].xs.clone();
        assert!(
            xs_on[2] > 5.0,
            "revealed on-cursor: '# ' keeps its real advance (reflow accepted): {xs_on:?}"
        );

        crate::markdown::set_wysiwyg_on(true);
    }

    /// HIT-TEST + CARET SANITY on a concealed line: several near-coincident
    /// zero-width x boundaries must never panic and must always resolve to a
    /// column within the line's valid range — the risk area this round's spec
    /// called out explicitly. Sweeps a click across the FULL row width of a
    /// concealed heading line (including squarely inside the collapsed "# "
    /// run) and asserts every result is in-bounds; also confirms two adjacent
    /// concealed byte positions can resolve to DIFFERENT columns without
    /// panicking (sequential linear scan over degenerate/duplicate x's).
    #[test]
    fn wysiwyg_zero_width_conceal_hit_test_stays_in_bounds() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(true);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping wysiwyg_zero_width_conceal_hit_test_stays_in_bounds: no wgpu adapter");
            return;
        };
        // Line 0 conceals ("# Title"); caret sits on line 1.
        let text = "# Title\nprose\n";
        let mut v = view(text, 1, 0);
        v.is_markdown = true;
        p.set_view(&v);
        let line_char_count = "# Title".chars().count();

        let doc_top = p.doc_top();
        let text_left = p.text_left();
        let py = doc_top + p.metrics.line_height * 0.5;
        // Sweep x from well left of the column through well past the last
        // glyph, including right where the collapsed "# " used to occupy space.
        let mut cols_seen = std::collections::BTreeSet::new();
        for i in -5..40 {
            let px = text_left + i as f32 * 2.0;
            let (line, col) = p.hit_test(px, py, 0);
            assert_eq!(line, 0, "click on row 0's band must resolve to line 0");
            assert!(
                col <= line_char_count,
                "column must stay within the line's char range: col={col} max={line_char_count}"
            );
            cols_seen.insert(col);
        }
        // The sweep must resolve to MORE than one column (not every click
        // collapsing to a single degenerate point) — proves the sequential
        // walk still discriminates real content despite the concealed run's
        // near-coincident x boundaries.
        assert!(
            cols_seen.len() > 1,
            "hit-test sweep should resolve multiple distinct columns: {cols_seen:?}"
        );

        crate::markdown::set_wysiwyg_on(true);
    }

    /// REGRESSION GUARD: `wysiwyg = false` stays a total no-op for the
    /// zero-width mechanism too — a concealable span is never given the
    /// near-zero-font-size metrics override (it's only ever plain-dimmed, byte-
    /// identical to the pre-WYSIWYG-round rendering), so a heading's `"# "` and
    /// an emphasis pair's `"*"` keep their REAL advances regardless of caret
    /// position.
    #[test]
    fn wysiwyg_off_keeps_real_advances_never_zero_width() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(false);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping wysiwyg_off_keeps_real_advances_never_zero_width: no wgpu adapter");
            return;
        };
        let text = "# Title\nprose\n";
        let mut v = view(text, 1, 0); // caret elsewhere: would conceal if wysiwyg were on
        v.is_markdown = true;
        p.set_view(&v);
        let xs = p.visual_rows(0)[0].xs.clone();
        assert!(
            xs[2] > 5.0,
            "wysiwyg=false: '# ' keeps its real advance even off-cursor: {xs:?}"
        );

        crate::markdown::set_wysiwyg_on(true);
    }

    /// REGRESSION GUARD: a non-markdown buffer never runs the WYSIWYG conceal
    /// pass at all (no `md_spans`, so `add_wysiwyg_conceal_spans` no-ops
    /// trivially) — a `.rs`-style line containing literal `# ` / `*` characters
    /// renders at their real advances, byte-identical to before this round.
    #[test]
    fn wysiwyg_non_markdown_buffer_untouched_by_zero_width_conceal() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(true);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping wysiwyg_non_markdown_buffer_untouched_by_zero_width_conceal: no wgpu adapter");
            return;
        };
        // A CODE-shaped line with literal '#'/'*' characters, is_markdown=false.
        let text = "# not a heading\nlet y = 2;\n";
        let mut v = view(text, 1, 0);
        v.is_markdown = false;
        p.set_view(&v);
        let xs = p.visual_rows(0)[0].xs.clone();
        assert!(
            xs[2] > 5.0,
            "non-markdown buffer: '# ' is plain text at its real advance: {xs:?}"
        );

        crate::markdown::set_wysiwyg_on(true);
    }

    // --- Blockquote WYSIWYG: conceal the `>` marker + margin pull-quote mark ---

    /// The blockquote `>` marker CONCEALS off the caret's line (collapses to
    /// near-zero advance, so the quote text starts flush at the column edge) and
    /// REVEALS at its real advance when the caret lands on the line — the same
    /// reveal-on-cursor contract as the heading/emphasis conceal, now generalized
    /// to `ConcealKind::Blockquote`.
    #[test]
    fn blockquote_marker_conceals_off_caret_and_reveals_on_caret() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(true);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping blockquote_marker_conceals_off_caret_and_reveals_on_caret: no wgpu adapter");
            return;
        };
        // "> quoted": the "> " marker is chars 0..2; "quoted" starts at char col 2.
        let text = "> quoted\nprose\n";

        // Caret on line 1 (a DIFFERENT line): line 0's "> " conceals to near-zero,
        // so "quoted" starts flush at ~0.
        let mut off = view(text, 1, 0);
        off.is_markdown = true;
        p.set_view(&off);
        let xs_off = p.visual_rows(0)[0].xs.clone();
        assert!(
            xs_off[2] < 1.0,
            "concealed '> ' collapses, quote text starts flush off-cursor: {xs_off:?}"
        );

        // Caret ON the blockquote line: the "> " reveals at its real advance.
        let mut on = view(text, 0, 0);
        on.is_markdown = true;
        p.set_view(&on);
        let xs_on = p.visual_rows(0)[0].xs.clone();
        assert!(
            xs_on[2] > 5.0,
            "revealed on-cursor: '> ' keeps its real advance (reflow accepted): {xs_on:?}"
        );

        crate::markdown::set_wysiwyg_on(true);
    }

    /// ONE hanging pull-quote mark per contiguous blockquote BLOCK — not per line.
    /// Two separate blockquotes yield two blocks; a nested `>>` line stays part of
    /// its contiguous block (the markers coalesce), so it never spawns a second
    /// mark. Asserted via the page/scroll-independent `quote_block_lines` cache.
    #[test]
    fn blockquote_hanging_mark_is_one_per_block_nested_coalesces() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(true);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping blockquote_hanging_mark_is_one_per_block_nested_coalesces: no wgpu adapter");
            return;
        };
        // Block A: lines 0-1. A blank + a paragraph break the run. Block B: lines
        // 5-6, whose line 6 is a NESTED `>>` (still one contiguous block).
        //  0: "> a"   1: "> b"   2: ""   3: "para"   4: ""   5: "> c"   6: ">> d"
        let text = "> a\n> b\n\npara\n\n> c\n>> d\n";
        let mut v = view(text, 3, 0); // caret on the plain paragraph
        v.is_markdown = true;
        p.set_view(&v);
        assert_eq!(
            p.quote_block_lines(),
            vec![0, 5],
            "one block starting at line 0 (a,b) and one at line 5 (c + nested d)"
        );
    }

    /// The margin PULL-QUOTE mark is PAGE-MODE only (the left margin exists only in
    /// page mode) — `quote_marks` yields a top per visible block in page mode and
    /// NOTHING edge-to-edge (the documented non-page fallback: the concealed marker
    /// alone). Also present regardless of the caret (a block affordance, not
    /// reveal-on-cursor).
    #[test]
    fn blockquote_pull_quote_mark_page_mode_only() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        crate::markdown::set_wysiwyg_on(true);
        let was_page = crate::page::page_on();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping blockquote_pull_quote_mark_page_mode_only: no wgpu adapter");
            crate::page::set_page_on(was_page);
            return;
        };
        let text = "> a\n> b\n\npara\n\n> c\n";
        let mut v = view(text, 0, 0); // caret INSIDE block A — mark still present
        v.is_markdown = true;
        p.set_view(&v);

        crate::page::set_page_on(true);
        assert_eq!(
            p.quote_marks().len(),
            2,
            "page mode: one hanging mark per visible block, present even with the caret in a block"
        );

        crate::page::set_page_on(false);
        assert!(
            p.quote_marks().is_empty(),
            "edge-to-edge (non-page): no margin, so no hanging mark (concealed marker only)"
        );

        crate::page::set_page_on(was_page);
    }

    /// DETERMINISM GUARD: a doc with no blockquote produces NO pull-quote marks and
    /// NO blockquote conceal spans — nothing here touches a non-blockquote render.
    #[test]
    fn non_blockquote_doc_has_no_quote_marks() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        crate::markdown::set_wysiwyg_on(true);
        let was_page = crate::page::page_on();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping non_blockquote_doc_has_no_quote_marks: no wgpu adapter");
            crate::page::set_page_on(was_page);
            return;
        };
        let text = "# Title\nplain prose with a > not-a-quote inline\n";
        let mut v = view(text, 0, 0);
        v.is_markdown = true;
        p.set_view(&v);
        crate::page::set_page_on(true);
        assert!(p.quote_block_lines().is_empty(), "no blockquote blocks in a plain doc");
        assert!(p.quote_marks().is_empty(), "no pull-quote marks in a plain doc");
        crate::page::set_page_on(was_page);
    }

    /// FIX (2026-07-09): the hanging pull-quote DROP-CAP mark must live INSIDE the
    /// writing column (in the quote block's own left text-pad gutter), NOT out in the
    /// left margin where it collided with the now-default-on OUTLINE. The pure
    /// placement law (`super::geometry::pull_quote_left`): the mark's RIGHT edge
    /// clears the quote text's left edge, and its LEFT edge never spills back out of
    /// the page into the margin.
    #[test]
    fn pull_quote_hangs_in_the_column_gutter_never_the_margin() {
        use super::geometry::pull_quote_left;
        // Typical page-mode geometry: page column at 240, text inset to 280, a small
        // clearance gap, a narrow mark that fits the gutter.
        let (column_left, text_left, gap, mark_w) = (240.0_f32, 280.0_f32, 4.0_f32, 22.0_f32);
        let x = pull_quote_left(column_left, text_left, gap, mark_w);
        assert!(
            x >= column_left - 1e-4,
            "mark left never past the page edge into the outline's margin: {x} < {column_left}"
        );
        assert!(
            x + mark_w <= text_left - gap + 1e-4,
            "mark right edge clears the quote text (a `gap` shy of `text_left`): {} vs {text_left}",
            x + mark_w
        );
        assert!(
            x > column_left + 1e-4,
            "a mark that fits the gutter hangs shy of the text, not flush at the page edge: {x}"
        );
        // An OVER-WIDE mark (wider than the gutter) clamps to `column_left` — it stays
        // INSIDE the page (out of the margin) rather than spilling left into the
        // outline; the accepted cost is a slight overlap with the text, never a
        // collision with the margin.
        let wide = pull_quote_left(column_left, text_left, gap, 100.0);
        assert!(
            (wide - column_left).abs() < 1e-4,
            "an over-wide mark clamps to the page edge, never the margin: {wide}"
        );
    }

    // --- Fence-panel / wash SEAM merge (`merge_row_bands`) ------------------

    /// `merge_row_bands` PURE UNIT CONTRACT: vertically-contiguous same-x
    /// bands collapse to one quad spanning their union; a variable-width run
    /// merges to the UNION x-range; two bands on the SAME row (equal y) never
    /// merge into each other; a real vertical GAP (an intervening unlisted row)
    /// keeps bands separate.
    #[test]
    fn merge_row_bands_contract() {
        use super::rects::merge_row_bands;
        // Three contiguous same-width rows (a uniform "panel") -> one quad.
        let uniform = vec![[10.0, 0.0, 100.0, 32.0], [10.0, 32.0, 100.0, 32.0], [10.0, 64.0, 100.0, 32.0]];
        let merged = merge_row_bands(uniform);
        assert_eq!(merged.len(), 1, "three contiguous rows merge to one: {merged:?}");
        assert!((merged[0][1] - 0.0).abs() < 1e-3, "merged top == first row's top");
        assert!((merged[0][3] - 96.0).abs() < 1e-3, "merged height == sum of all three: {merged:?}");
        assert!((merged[0][0] - 10.0).abs() < 1e-3 && (merged[0][2] - 100.0).abs() < 1e-3);

        // Variable-width contiguous rows (a wrapped prose wash) -> ONE quad at
        // the UNION x-range.
        let variable = vec![[20.0, 0.0, 30.0, 32.0], [5.0, 32.0, 80.0, 32.0]];
        let merged_v = merge_row_bands(variable);
        assert_eq!(merged_v.len(), 1, "variable-width contiguous rows still merge: {merged_v:?}");
        assert!((merged_v[0][0] - 5.0).abs() < 1e-3, "union left == the wider row's left");
        assert!((merged_v[0][2] - 80.0).abs() < 1e-3, "union width == max(20+30, 5+80) - 5 = 80: {merged_v:?}");
        assert!((merged_v[0][3] - 64.0).abs() < 1e-3);

        // Two bands on the SAME row (equal y, disjoint x) never merge into
        // each other.
        let same_row = vec![[0.0, 0.0, 10.0, 32.0], [50.0, 0.0, 10.0, 32.0]];
        let merged_s = merge_row_bands(same_row);
        assert_eq!(merged_s.len(), 2, "same-row bands stay separate: {merged_s:?}");

        // A real vertical GAP (row 2 skipped entirely) keeps the two runs apart.
        let gapped = vec![[0.0, 0.0, 10.0, 32.0], [0.0, 64.0, 10.0, 32.0]];
        let merged_g = merge_row_bands(gapped);
        assert_eq!(merged_g.len(), 2, "a real gap keeps bands separate: {merged_g:?}");

        // Empty / single input pass through untouched.
        assert!(merge_row_bands(Vec::new()).is_empty());
        let one = vec![[1.0, 2.0, 3.0, 4.0]];
        assert_eq!(merge_row_bands(one.clone()), one);
    }

    /// MULTI-ROW WASH SEAM: a multi-line `/* ... */` block comment (three
    /// contiguous visual rows, same bucket) merges into ONE continuous quad —
    /// the live-review's "python docstring wash striping" report. Compares
    /// against the merged height so a future regression (e.g. reintroducing
    /// per-row emission without the merge) is caught directly.
    #[test]
    fn multiline_comment_wash_merges_into_one_continuous_band() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping multiline_comment_wash_merges_into_one_continuous_band: no wgpu adapter");
            return;
        };
        let text = "/* line one\n   line two\n   line three */\nlet x = 1;\n";
        let mut v = view(text, 3, 0);
        v.syn_lang = Some(crate::syntax::Lang::Rust);
        p.set_view(&v);
        let (comments, _strings, _highlights) = p.wash_rects();
        assert_eq!(
            comments.len(), 1,
            "a 3-row block comment merges into one continuous wash band: {comments:?}"
        );
        let expected_h = 3.0 * p.metrics.line_height;
        assert!(
            (comments[0][3] - expected_h).abs() < 1.0,
            "merged band spans all 3 rows: {comments:?} vs {expected_h}"
        );
    }

    /// FENCE-PANEL CACHE contract, mirroring `wash_cache_and_geometry_contract`:
    /// a cursor move / scroll keeps the proto cache warm (no rebuild); an edit
    /// reshapes once and rebuilds it (a new version key).
    #[test]
    fn fence_panel_cache_stays_warm_across_cursor_and_scroll_rebuilds_on_edit() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(true);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping fence_panel_cache_stays_warm_across_cursor_and_scroll_rebuilds_on_edit: no wgpu adapter");
            return;
        };
        let text = "```rust\nlet x = 1;\n```\n";
        let mut v = view(text, 0, 0);
        v.is_markdown = true;
        p.set_view(&v);
        let _ = p.fence_panel_rects();
        let key = p.fence_panel_cache_version().expect("protos built");
        let reshapes = p.reshape_count;

        // A cursor move (revealing the fence) keeps the cache warm.
        let mut v2 = view(text, 1, 0);
        v2.is_markdown = true;
        p.set_view(&v2);
        let _ = p.fence_panel_rects();
        assert_eq!(p.reshape_count, reshapes, "a cursor move must not reshape");
        assert_eq!(
            p.fence_panel_cache_version(), Some(key),
            "a cursor move keeps the fence-panel protos warm"
        );

        // An edit reshapes once and rebuilds the protos (new version key).
        let edited = "```rust\nlet x = 2;\n```\n";
        let mut v3 = view(edited, 0, 0);
        v3.is_markdown = true;
        p.set_view(&v3);
        let _ = p.fence_panel_rects();
        assert_eq!(p.reshape_count, reshapes + 1, "the edit reshapes once");
        assert_ne!(
            p.fence_panel_cache_version(), Some(key),
            "an edit rebuilds the fence-panel protos"
        );

        crate::markdown::set_wysiwyg_on(true);
    }

    /// WYSIWYG rides the WASH + FENCE-PANEL cache KEYS: both caches BUILD a
    /// WYSIWYG-gated bucket (the inline-code pill; the whole fence panel), and
    /// `wysiwyg_on()` is a process-global that can flip WITHOUT a reshape. So a
    /// runtime flip must REKEY each cache and force a rebuild — never serve the
    /// stale on-state protos. Pre-fix the key was only `(generation, reshape_count)`,
    /// so a flip left the key unchanged and the stale pill/panel kept drawing.
    #[test]
    fn wysiwyg_flip_rekeys_wash_and_fence_panel_caches() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping wysiwyg_flip_rekeys_wash_and_fence_panel_caches: no wgpu adapter");
            return;
        };
        // A markdown buffer carrying BOTH a WYSIWYG-gated inline-code pill and a
        // WYSIWYG-gated fenced-block panel. The md_spans (inline `Code` + the
        // `ConcealMarkup(Fence)` range) are parsed at set_view time and do NOT depend
        // on the wysiwyg global — only the PROTO build does — so flipping the global
        // afterward isolates the cache-key contract.
        let text = "an `inline` bit\n\n```rust\nlet x = 1;\n```\n";
        let mut v = view(text, 0, 0);
        v.is_markdown = true;

        // WYSIWYG ON: the pill + panel are present; capture each cache's key.
        crate::markdown::set_wysiwyg_on(true);
        p.set_view(&v);
        assert!(!p.code_pill_rects().is_empty(), "wysiwyg on: the inline-code pill draws");
        assert!(!p.fence_panel_rects().is_empty(), "wysiwyg on: the fence panel draws");
        let wash_key_on = p.wash_cache_version().expect("wash protos built");
        let panel_key_on = p.fence_panel_cache_version().expect("panel protos built");

        // Flip WYSIWYG OFF with NO reshape / geometry change (same buffer, same
        // view) — only the process-global toggled. Both caches must REKEY (the
        // wysiwyg half of the key flips) and rebuild to the empty buckets. A stale
        // (generation, reshape_count)-only key would still serve the on-state pill /
        // panel here.
        crate::markdown::set_wysiwyg_on(false);
        assert!(
            p.code_pill_rects().is_empty(),
            "wysiwyg off: no pill (a stale wash bucket would still draw one)"
        );
        assert!(
            p.fence_panel_rects().is_empty(),
            "wysiwyg off: no panel (a stale fence-panel bucket would still draw one)"
        );
        assert_ne!(
            p.wash_cache_version(), Some(wash_key_on),
            "flipping wysiwyg rekeys the wash cache"
        );
        assert_ne!(
            p.fence_panel_cache_version(), Some(panel_key_on),
            "flipping wysiwyg rekeys the fence-panel cache"
        );

        // restore the sticky default for any later test on this thread
        crate::markdown::set_wysiwyg_on(true);
    }

    /// ROWGEOM GENERATION: every `invalidate()` bumps the shaped-geometry
    /// generation the derived proto caches key on. Pure cache mechanics — no GPU.
    #[test]
    fn row_geom_invalidate_bumps_generation() {
        let rg = rowgeom::RowGeom::new();
        let g0 = rg.generation();
        rg.invalidate();
        assert_eq!(rg.generation(), g0 + 1, "one invalidate = one generation step");
        rg.invalidate();
        rg.invalidate();
        assert_eq!(rg.generation(), g0 + 3, "the generation is monotonic per invalidate");
    }

    /// SPELL-GEN + EDIT INVALIDATION: (a) a NEW spell list over the SAME text —
    /// only the spell generation moves, NO reshape — must re-place the squiggle
    /// under the newly-flagged word; (b) an EDIT that shifts the flagged word
    /// right must move BOTH the squiggle and the nit underline (the reshape bumps
    /// the RowGeom generation both caches key on). GPU-backed; skips w/o adapter.
    #[test]
    fn underline_cache_rebuilds_on_spell_list_and_edit() {
        // Squiggle x-positions fold the theme advances + the page wrap globals;
        // nits also read their process toggle. Hold all three (theme → page →
        // nits) so no parallel mutator moves the geometry between reads.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let _n = crate::nits::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping underline_cache_rebuilds_on_spell_list_and_edit: no wgpu adapter");
            return;
        };
        crate::nits::set_nits_on(true);
        // "helo" (cols 0..4) and "wrld" (cols 5..9) on line0; the double space at
        // cols 9..11 is the nit. A second, untouched line1 gives the cursor
        // somewhere to sit OFF line0 — REVEAL-ON-CURSOR suppresses every nit (and
        // the caret's own word) on the caret's line, which would otherwise
        // swallow this fixture's line0-only spans.
        let text = "helo wrld  x\nzzz";
        let span = |s: usize, e: usize| crate::spell::Misspelling { line: 0, start_col: s, end_col: e };
        let mut v = view(text, 1, 0);
        v.misspelled = vec![span(0, 4)];
        p.set_view(&v);
        let reshapes = p.reshape_count;
        let s1 = p.spell_squiggles();
        assert_eq!(s1.len(), 1, "one misspelling => one squiggle");
        let n1 = p.nit_underlines();
        assert_eq!(n1.len(), 1, "the double space => one nit underline");

        // (a) SAME text, the OTHER word flagged: no reshape (no generation bump),
        // only the spell list generation — the squiggle must still move right.
        let mut v2 = view(text, 1, 0);
        v2.misspelled = vec![span(5, 9)];
        p.set_view(&v2);
        assert_eq!(p.reshape_count, reshapes, "a spell-list-only push must not reshape");
        let s2 = p.spell_squiggles();
        assert_eq!(s2.len(), 1);
        assert!(
            s2[0].x > s1[0].x + 1.0,
            "a new spell list over unchanged text must re-place the squiggle \
             (old x={}, new x={})",
            s1[0].x,
            s2[0].x
        );

        // (b) EDIT: prefix "zz " shifts every flagged span right by 3 columns.
        // The reshape bumps the RowGeom generation, so BOTH proto caches rebuild.
        let edited = "zz helo wrld  x\nzzz";
        let mut v3 = view(edited, 1, 0);
        v3.misspelled = vec![span(3, 7)];
        p.set_view(&v3);
        assert_eq!(p.reshape_count, reshapes + 1, "the edit reshapes once");
        let s3 = p.spell_squiggles();
        assert_eq!(s3.len(), 1);
        assert!(
            s3[0].x > s1[0].x + 1.0,
            "the squiggle must follow the shifted word (old x={}, new x={})",
            s1[0].x,
            s3[0].x
        );
        let n3 = p.nit_underlines();
        assert_eq!(n3.len(), 1);
        assert!(
            n3[0].x > n1[0].x + 1.0,
            "the nit underline must follow the shifted double space \
             (old x={}, new x={})",
            n1[0].x,
            n3[0].x
        );
        crate::nits::set_nits_on(true);
    }

    /// ZOOM INVALIDATION: a zoom change re-shapes at the new metrics and bumps the
    /// RowGeom GENERATION; the cached squiggle/nit protos keyed on it must rebuild
    /// so the bands scale with the glyphs instead of replaying zoom-1 pixels.
    #[test]
    fn underline_cache_rebuilds_on_zoom_change() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let _n = crate::nits::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping underline_cache_rebuilds_on_zoom_change: no wgpu adapter");
            return;
        };
        crate::nits::set_nits_on(true);
        // Double space at cols 2..4 (the nit), "helo" at cols 7..11 (the squiggle):
        // both sit past col 0 so their x carries the zoom-scaled advances. A
        // second, untouched line1 gives the cursor somewhere to sit OFF line0
        // (REVEAL-ON-CURSOR would otherwise suppress both fixtures on the caret's
        // own line).
        let text = "aa  bb helo\nzzz";
        let mis = vec![crate::spell::Misspelling { line: 0, start_col: 7, end_col: 11 }];
        let mut v1 = view(text, 1, 0);
        v1.misspelled = mis.clone();
        p.set_view(&v1);
        let s1 = p.spell_squiggles();
        let n1 = p.nit_underlines();
        assert_eq!((s1.len(), n1.len()), (1, 1));

        let mut v2 = view(text, 1, 0);
        v2.misspelled = mis;
        v2.zoom = 1.6;
        p.set_view(&v2);
        let s2 = p.spell_squiggles();
        let n2 = p.nit_underlines();
        assert_eq!((s2.len(), n2.len()), (1, 1));
        // The word starts 7 zoomed advances in: x must move right with the scale.
        assert!(
            s2[0].x > s1[0].x + 1.0,
            "zoom must re-place the squiggle on the scaled advances \
             (z1 x={}, z1.6 x={})",
            s1[0].x,
            s2[0].x
        );
        assert!(
            s2[0].w > s1[0].w + 1.0,
            "the squiggle band must widen with the zoomed glyphs \
             (z1 w={}, z1.6 w={})",
            s1[0].w,
            s2[0].w
        );
        assert!(
            (s2[0].amp - s1[0].amp * 1.6).abs() < 1e-3,
            "the wave amplitude scales with zoom"
        );
        assert!(
            n2[0].x > n1[0].x + 1.0,
            "zoom must re-place the nit underline too (z1 x={}, z1.6 x={})",
            n1[0].x,
            n2[0].x
        );
        crate::nits::set_nits_on(true);
    }

    /// THEME-FONT-SWITCH INVALIDATION: a display-face switch reshapes
    /// (`sync_theme` → `restyle_all_lines` → RowGeom invalidate), so the squiggle
    /// protos rebuild against the NEW advances — the band under "brown" must
    /// follow the proportional x-range, not replay the mono cell grid.
    #[test]
    fn underline_cache_rebuilds_on_theme_font_switch() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping underline_cache_rebuilds_on_theme_font_switch: no wgpu adapter");
            return;
        };
        theme::set_active_by_name("Tawny").unwrap(); // mono grid
        p.sync_theme();
        let text = "The quick brown fox";
        let mut v = view(text, 0, 0);
        v.misspelled = vec![crate::spell::Misspelling { line: 0, start_col: 10, end_col: 15 }];
        p.set_view(&v);
        let s1 = p.spell_squiggles();
        assert_eq!(s1.len(), 1);

        theme::set_active_by_name("Gumtree").unwrap(); // proportional Literata
        p.sync_theme();
        let s2 = p.spell_squiggles();
        assert_eq!(s2.len(), 1, "the squiggle survives the font switch");
        // The prefix "The quick " and the word "brown" both shape to different
        // advances on the proportional face, so the band's x-range must move.
        assert!(
            (s2[0].x - s1[0].x).abs() > 1.0 || (s2[0].w - s1[0].w).abs() > 1.0,
            "a font switch must rebuild the squiggle on the new advances \
             (mono x={} w={}, serif x={} w={})",
            s1[0].x,
            s1[0].w,
            s2[0].x,
            s2[0].w
        );

        // Restore the default world so other tests see a clean global.
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// SCROLL CULL + REVEAL: the protos are scroll-INDEPENDENT — each frame just
    /// adds the current `doc_top` and culls bands outside the viewport plus the
    /// generous 8-line margin. A squiggle far below the canvas must emit NOTHING
    /// at scroll 0, then appear (inside the canvas) once scrolled into view — all
    /// WITHOUT a reshape, so both frames are served by the SAME cached protos.
    #[test]
    fn squiggle_scroll_culls_offscreen_and_reveals_on_scroll() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping squiggle_scroll_culls_offscreen_and_reveals_on_scroll: no wgpu adapter");
            return;
        };
        // 100 short lines; "helo" misspelled on line 60 — ~1900px below the top
        // of the 800px canvas, far past the 8-line-height cull margin.
        let mut text = String::new();
        for i in 0..100 {
            if i == 60 {
                text.push_str("helo\n");
            } else {
                text.push_str(&format!("line {i}\n"));
            }
        }
        let mis = vec![crate::spell::Misspelling { line: 60, start_col: 0, end_col: 4 }];
        let mut v = view(&text, 0, 0);
        v.misspelled = mis.clone();
        p.set_view(&v);
        let reshapes = p.reshape_count;
        assert!(
            p.spell_squiggles().is_empty(),
            "a squiggle far below the viewport is culled (would rasterize nothing)"
        );

        // Scroll the word's row into view: a scroll-only push (no reshape) — the
        // cached proto must now emit a band inside the canvas.
        let mut v2 = view(&text, 0, 0);
        v2.misspelled = mis;
        v2.scroll_lines = 55;
        p.set_view(&v2);
        assert_eq!(p.reshape_count, reshapes, "a scroll-only push must not reshape");
        let s = p.spell_squiggles();
        assert_eq!(s.len(), 1, "scrolled into view: the cached proto now emits");
        assert!(
            s[0].y > 0.0 && s[0].y < p.window_h,
            "the revealed band sits inside the canvas: y={}",
            s[0].y
        );
    }

    #[test]
    fn hud_report_figures_and_held_tracks_the_global() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping hud_report_figures_and_held_tracks_the_global: no wgpu adapter");
            return;
        };
        let _g = crate::hud::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // A markdown buffer, cursor at the very start => 0% through the doc. The HUD is
        // now TRIMMED to the two WRITER figures — word count + %-through-doc — with no
        // file-created / session-time fields at all.
        let mut v = view("# Title\n\nsome prose with five words\n", 0, 0);
        v.is_markdown = true;
        p.set_view(&v);
        let r = p.hud_report();
        assert_eq!(r.percent, 0, "cursor at the start => 0%");
        assert!(r.words.is_some(), "a markdown buffer reports a word count");
        // The LIFETIME-ODOMETER fields moved to the summoned Lifetime stats card's
        // report: they default to the "—" placeholder since the pipeline's
        // `hud_stats` is `None` until the live App pushes a snapshot (never in a
        // headless pipeline), so every odometer row reads as unknown.
        let l = p.lifetime_report();
        assert!(!l.open, "the Lifetime card global is off by default");
        for f in [&l.chars, &l.writing, &l.files, &l.caret_travel, &l.world] {
            assert_eq!(f, crate::hud::PLACEHOLDER, "odometer field defaults to placeholder");
        }
        // After a snapshot is pushed, the Lifetime card's fields format the real figures.
        p.set_hud_stats(Some(crate::hud::HudStats {
            chars_typed: 1_234,
            active_writing_ms: 12 * 60_000,
            files_touched: 7,
            caret_distance_px: 820.0 * crate::hud::CARET_PX_PER_METRE,
            world: Some("Tawny".to_string()),
        }));
        let l2 = p.lifetime_report();
        assert_eq!(l2.chars, "1,234");
        assert_eq!(l2.writing, "12m");
        assert_eq!(l2.files, "7");
        assert_eq!(l2.caret_travel, "820 m");
        assert_eq!(l2.world, "Tawny");
        p.set_hud_stats(None);
        // LINE ENDINGS: the report carries the view's EOL — a pure buffer fact,
        // deterministic (unlike the dropped clock/fs fields). The `view()` helper
        // defaults to LF; a CRLF view flips the reported ending + its "LF"/"CRLF" label.
        assert_eq!(r.eol, crate::buffer::Eol::Lf, "default view is LF");
        assert_eq!(r.eol.label(), "LF");
        let mut crlf = view("# Title\n\nsome prose\n", 0, 0);
        crlf.is_markdown = true;
        crlf.eol = crate::buffer::Eol::Crlf;
        p.set_view(&crlf);
        assert_eq!(p.hud_report().eol, crate::buffer::Eol::Crlf, "CRLF view reports CRLF");
        assert_eq!(p.hud_report().eol.label(), "CRLF");
        p.set_view(&v);

        // `held` mirrors the process-global both ways.
        crate::hud::set_held(false);
        assert!(!p.hud_report().held);
        crate::hud::set_held(true);
        assert!(p.hud_report().held);
        crate::hud::set_held(false);

        // A non-markdown buffer OMITS the word count (writer-only stat).
        let mut code = view("fn main() {}\n", 0, 0);
        code.is_markdown = false;
        p.set_view(&code);
        assert_eq!(p.hud_report().words, None, "non-markdown omits the word count");

        // %-through-doc advances with the cursor: near the document end it is a high
        // fraction (and never exceeds 100). Cursor on the last content line's end.
        let mut endv = view("abcd\nefgh\n", 1, 4);
        endv.is_markdown = true;
        p.set_view(&endv);
        let pct = p.hud_report().percent;
        assert!((80..=100).contains(&pct), "cursor near the end => high percent, got {pct}");
    }

    /// The held stats HUD and a full summoned overlay are MUTUALLY EXCLUSIVE (the
    /// overlay wins). `hud_showing()` — the ONE owner both the blur gate and the
    /// `prepare_hud` layout gate route through — is TRUE only when the key is held
    /// AND no overlay is open, so a still-held Cmd-I never draws its card over an
    /// open theme picker nor forces the frost that would defeat the picker's crisp
    /// live-color preview. (Regression for the "HUD renders on top of the picker"
    /// live bug.)
    #[test]
    fn hud_showing_yields_to_an_open_overlay() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping hud_showing_yields_to_an_open_overlay: no wgpu adapter");
            return;
        };
        let _g = crate::hud::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // HUD held, NO overlay => the HUD draws.
        crate::hud::set_held(true);
        let mut plain = view("hello world\n", 0, 0);
        plain.overlay_active = false;
        p.set_view(&plain);
        assert!(p.hud_showing(), "held + no overlay => the HUD shows");

        // HUD still held, but a CRISP overlay (the theme picker) is open => the HUD
        // yields: nothing HUD-shaped draws, and it contributes NO backdrop blur, so
        // the picker keeps its crisp live-color preview.
        let mut over = view("hello world\n", 0, 0);
        over.overlay_active = true;
        over.overlay_crisp = true;
        p.set_view(&over);
        assert!(!p.hud_showing(), "held + overlay open => the HUD is suppressed");
        assert!(
            !p.backdrop_blur(),
            "a crisp overlay + a suppressed HUD leaves the frame unblurred (crisp preview intact)"
        );

        // Close the overlay while the key is STILL held => the HUD reappears.
        p.set_view(&plain);
        assert!(p.hud_showing(), "overlay closed while held => the HUD returns");

        // Releasing the key stops it regardless of overlay state.
        crate::hud::set_held(false);
        assert!(!p.hud_showing(), "released => never showing");
        crate::hud::set_held(false);
    }

    /// THE HOLD-⌘ PEEK's held-card report + draw gate: `peek_report().rows` folds an
    /// EMPTY push to the curated starter six (the capture / fresh-install fallback) and
    /// reflects a personalized push verbatim; `peek_showing()` (the ONE owner the blur
    /// gate + `prepare_hud` route through) is true only while open AND no overlay is up,
    /// so the peek never draws over a picker — same yield contract as the held HUD.
    #[test]
    fn peek_report_folds_empty_to_starter_and_yields_to_an_open_overlay() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping peek_report_folds_empty_to_starter_and_yields_to_an_open_overlay: no wgpu adapter");
            return;
        };
        let _g = crate::peek::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // No pushed rows (a capture / fresh install) => the report folds to the starter six.
        p.set_peek_rows(Vec::new());
        assert_eq!(
            p.peek_report().rows,
            crate::peek::starter_rows(),
            "empty push => the curated starter six renders"
        );
        // A personalized push (the live ledger's candidates) wins verbatim.
        let learned = vec![crate::peek::PeekRow {
            chord: "⌘;".into(),
            name: "Spell suggestions".into(),
        }];
        p.set_peek_rows(learned.clone());
        assert_eq!(p.peek_report().rows, learned, "pushed rows shown verbatim");

        // The draw gate: open + no overlay => showing; an open overlay suppresses it.
        crate::peek::set_open(true);
        let mut plain = view("hello\n", 0, 0);
        plain.overlay_active = false;
        p.set_view(&plain);
        assert!(p.peek_showing(), "open + no overlay => the peek shows");
        assert!(p.peek_report().open, "report mirrors the process-global");
        let mut over = view("hello\n", 0, 0);
        over.overlay_active = true;
        p.set_view(&over);
        assert!(!p.peek_showing(), "open + overlay => the peek is suppressed");
        crate::peek::set_open(false);
        assert!(!p.peek_showing(), "closed => never showing");
        crate::peek::set_open(false);
    }

    /// THE KEYBINDINGS TIPS FOOTER grows the card by exactly its rows: a flat overlay
    /// with N tips pushed is `N + 1` rows (the tips + one blank separator) taller than
    /// the same overlay with none — the chrome-below-the-list threading. Empty tips
    /// (every non-Keybindings picker, and every capture) leave the card unchanged, so a
    /// Keybindings capture is byte-identical.
    #[test]
    fn keybindings_tips_footer_grows_the_card_by_its_rows() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping keybindings_tips_footer_grows_the_card_by_its_rows: no wgpu adapter");
            return;
        };
        let mut v = view("hello\n", 0, 0);
        v.overlay_active = true;
        v.overlay_items = vec!["Go to file".into(), "Save".into(), "Undo".into()];

        // No tips: baseline card height (the footer is hidden — capture-identical).
        p.set_keybindings_tips(Vec::new());
        p.set_view(&v);
        let (_, _, _, base_h, _) = p.overlay_window_report().expect("overlay open");

        // Three tips: the card grows by 3 tip rows + 1 blank separator = 4 rows.
        p.set_keybindings_tips(vec![
            "⌘O  Go to file".into(),
            "⌘T  Switch theme".into(),
            "⌘S  Save".into(),
        ]);
        p.set_view(&v);
        let (_, _, _, tips_h, _) = p.overlay_window_report().expect("overlay open");
        let grew = tips_h - base_h;
        let lh = p.overlay_lh();
        assert!(
            (grew - 4.0 * lh).abs() < 0.5,
            "footer added 3 tips + 1 separator = 4 rows (grew {grew}, lh {lh})"
        );
    }

    #[test]
    fn md_line_scale_keys_off_leading_hash_count() {
        use crate::markdown::heading_scale;
        // Non-markdown buffer: always body size, whatever the text.
        assert_eq!(md_line_scale("# heading", false), 1.0);
        // Size by the leading-hash COUNT (valid ATX or not).
        assert_eq!(md_line_scale("# h1", true), heading_scale(1));
        assert_eq!(md_line_scale("## h2", true), heading_scale(2));
        assert_eq!(md_line_scale("### h3", true), heading_scale(3));
        assert_eq!(md_line_scale("###### deep", true), heading_scale(3)); // 4+ clamps
        // Grows the instant you type `#`, before the space + title.
        assert_eq!(md_line_scale("#", true), heading_scale(1));
        assert_eq!(md_line_scale("#nospace", true), heading_scale(1));
        assert_eq!(md_line_scale("  ## indented", true), heading_scale(2));
        // A `#` that is NOT the line's leading run is ignored (body size).
        assert_eq!(md_line_scale("not a #heading", true), 1.0);
        assert_eq!(md_line_scale("plain prose", true), 1.0);
    }

    #[test]
    fn md_line_scale_grows_thematic_break_rows_to_the_active_worlds_ornament_scale() {
        // A thematic break grows its row to the ACTIVE WORLD'S per-world ornament scale
        // (no longer a single global rung), so the tall row centers the bigger fleuron
        // — and by the SAME value `prepare_ornaments` shapes the glyph at. md_line_scale
        // reads `theme::active().ornament_scale`, so hold the theme lock while flipping.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // A GEOMETRIC world (Currawong → 1.5): every break syntax grows to ITS scale.
        crate::theme::set_active_by_name("Currawong").unwrap();
        let geo = crate::theme::active().ornament_scale;
        assert_eq!(geo, crate::theme::ORNAMENT_SCALE_GEOMETRIC);
        assert_eq!(md_line_scale("---", true), geo);
        assert_eq!(md_line_scale("***", true), geo);
        assert_eq!(md_line_scale("___", true), geo);
        assert_eq!(md_line_scale("- - -", true), geo);

        // An ORNATE world (Mopoke → 2.2): the SAME break lines now grow to the LARGER
        // scale — proof the row height is per-world, not a fixed rung.
        crate::theme::set_active_by_name("Mopoke").unwrap();
        let ornate = crate::theme::active().ornament_scale;
        assert_eq!(ornate, crate::theme::ORNAMENT_SCALE_ORNATE);
        assert!(ornate > geo, "the ornate world grows the break row more than a geometric one");
        assert_eq!(md_line_scale("---", true), ornate);
        assert_eq!(md_line_scale("***", true), ornate);

        // Gated to markdown; a non-md buffer keeps the break at body size (per-world
        // scale never applies), and a dash LIST item (not a break) stays body size.
        assert_eq!(md_line_scale("---", false), 1.0);
        assert_eq!(md_line_scale("- item", true), 1.0);

        crate::theme::set_active(crate::theme::DEFAULT_THEME);
    }

    #[test]
    fn heading_rows_are_taller_and_gated_to_markdown() {
        // The row-count assertion assumes NOTHING wraps, which folds the page
        // globals (column width); hold the page lock (page.rs:95-99).
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping heading_rows_are_taller_and_gated_to_markdown: no wgpu adapter");
            return;
        };
        // line0 = h1, line1 blank, line2/3 body, line4 trailing empty.
        let text = "# Big\n\nbody one\nbody two\n";

        // MARKDOWN: the heading row (row 0) is taller than a body row (row 2) by
        // ~heading_scale(1), while the body rows stay uniform.
        let mut md = view(text, 0, 0);
        md.is_markdown = true;
        p.set_view(&md);
        assert_eq!(p.total_visual_rows(), 5, "no wrap => one row per logical line");
        let h1 = p.row_height_px(0);
        let body = p.row_height_px(2);
        assert!(body > 0.0);
        let ratio = h1 / body;
        let want = crate::markdown::heading_scale(1);
        assert!(
            (ratio - want).abs() < 0.05,
            "h1 row should be ~{want}x a body row, got {ratio} ({h1}/{body})"
        );
        // Body rows are uniform among themselves.
        assert!((p.row_height_px(2) - p.row_height_px(3)).abs() < 0.01);
        let md_doc_h = p.total_doc_height();

        // NON-MARKDOWN: the SAME text shapes with uniform rows (no heading growth),
        // proving the size is gated like every other md effect.
        let mut plain = view(text, 0, 0);
        plain.is_markdown = false;
        p.set_view(&plain);
        assert!(
            (p.row_height_px(0) - p.row_height_px(2)).abs() < 0.01,
            "a non-markdown buffer must keep every row a uniform height"
        );
        assert!(
            md_doc_h > p.total_doc_height(),
            "the heading must make the markdown document taller in pixels"
        );

        // Non-wrapped: visual_row_of still equals the logical line, so cursor-follow
        // is unchanged when nothing wraps even though rows differ in height.
        p.set_view(&md);
        assert_eq!(p.visual_row_of(2, 0), 2);
    }

    #[test]
    fn thematic_break_row_grows_by_the_active_worlds_ornament_scale_and_refits_on_theme_switch() {
        // Row-height math folds the page wrap globals AND reads the active theme's
        // per-world ornament scale — hold both locks (order: theme, then page).
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping thematic_break_row_ornament_scale: no wgpu adapter");
            return;
        };
        // A thematic break (row 0) over a plain body line (row 2).
        let text = "---\n\nbody line\n";
        let mut md = view(text, 2, 0); // caret on the body line (logical line 2), NOT the break
        md.is_markdown = true;

        // GEOMETRIC world (Currawong → 1.5): the break row grows to ~1.5x a body row.
        crate::theme::set_active_by_name("Currawong").unwrap();
        p.set_view(&md);
        let body = p.row_height_px(2);
        assert!(body > 0.0);
        let geo_break = p.row_height_px(0);
        let geo_ratio = geo_break / body;
        assert!(
            (geo_ratio - crate::theme::ORNAMENT_SCALE_GEOMETRIC).abs() < 0.05,
            "Currawong break row should be ~{}x a body row, got {geo_ratio}",
            crate::theme::ORNAMENT_SCALE_GEOMETRIC
        );

        // Switch to an ORNATE world (Mopoke → 2.2) and RESHAPE via the same theme-font
        // seam a live theme switch rides: the break row must RE-FIT to the larger scale
        // (proof the row-height ↔ glyph-box coupling is per-world, picked up on switch).
        crate::theme::set_active_by_name("Mopoke").unwrap();
        p.sync_theme_font();
        let body2 = p.row_height_px(2);
        let ornate_break = p.row_height_px(0);
        let ornate_ratio = ornate_break / body2;
        assert!(
            (ornate_ratio - crate::theme::ORNAMENT_SCALE_ORNATE).abs() < 0.05,
            "Mopoke break row should be ~{}x a body row, got {ornate_ratio}",
            crate::theme::ORNAMENT_SCALE_ORNATE
        );
        assert!(
            ornate_break > geo_break + 0.5,
            "the ornate world must grow the break row taller than the geometric one \
             ({ornate_break} vs {geo_break})"
        );

        crate::theme::set_active(crate::theme::DEFAULT_THEME);
    }

    #[test]
    fn variable_height_scroll_reaches_the_last_row() {
        // Visual-row totals fold the page wrap globals; hold the page lock.
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping variable_height_scroll_reaches_the_last_row: no wgpu adapter");
            return;
        };
        // A document taller than the 800px viewport, with big headings interleaved.
        let mut text = String::new();
        for i in 0..10 {
            text.push_str(&format!("# Heading {i}\n\nbody line for section {i}\n\n"));
        }
        text.push_str("THE LAST LINE\n");
        let mut md = view(&text, 0, 0);
        md.is_markdown = true;
        p.set_view(&md);

        let total = p.total_visual_rows();
        let last = total - 1;
        // The doc overflows, so it must be scrollable, and following the last row
        // from the top yields a NON-zero scroll that keeps the last row reachable
        // (bounded by the pixel-accurate max).
        let max = p.max_scroll_rows(800.0);
        assert!(max > 0, "a doc taller than the viewport must be scrollable");
        let follow = p.scroll_to_show_row(last, 0, 800.0);
        assert!(follow > 0, "cursor-follow to the last row must scroll down");
        assert!(follow <= max, "follow scroll must stay within max_scroll");
        // At that scroll the last row's bottom fits inside the text viewport.
        let bottom = p.row_top_px(follow) + (p.total_doc_height() - p.row_top_px(last));
        let _ = bottom; // (sanity: row_top monotonic)
        assert!(
            p.total_doc_height() - p.row_top_px(follow) <= 800.0 - TEXT_TOP + 0.5,
            "from the follow scroll, the remaining document must fit the viewport"
        );
    }

    #[test]
    fn typewriter_centers_the_cursor_row() {
        // Visual-row totals + scroll targets fold the page wrap globals; hold the
        // page lock so a parallel page write can't re-wrap the doc mid-test.
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping typewriter_centers_the_cursor_row: no wgpu adapter");
            return;
        };
        // A plain (non-markdown) doc much taller than the 800px viewport: uniform
        // rows, so cursor-follow is purely about vertical placement.
        let mut text = String::new();
        for i in 0..40 {
            text.push_str(&format!("line {i}\n"));
        }
        p.set_view(&view(&text, 25, 0));
        let total = p.total_visual_rows();
        assert!(total >= 40, "the doc must overflow the viewport");
        let max = p.max_scroll_rows(800.0);
        assert!(max > 0, "a doc taller than the viewport must be scrollable");

        let row = p.visual_row_of(25, 0);
        // Typewriter OFF (minimal-adjust): only nudge enough to reveal the row near
        // the viewport BOTTOM — a SMALL scroll from the top.
        let minimal = p.scroll_to_show_row(row, 0, 800.0);
        // Typewriter ON: CENTER the row — scroll much further down.
        let centered = p.scroll_to_center_row(row, 800.0);
        assert!(
            centered > minimal,
            "centering must scroll further than the minimal-adjust (centered={centered}, minimal={minimal})"
        );
        assert!(centered <= max, "centered scroll must stay within max_scroll");

        // At the centered scroll, the cursor row's vertical CENTER sits within one
        // row height of the viewport's vertical center (closest integer-row centering).
        let avail = 800.0 - TEXT_TOP;
        let viewport_center = TEXT_TOP + avail / 2.0;
        let doc_top = TEXT_TOP - p.row_top_px(centered);
        let row_center = doc_top + p.row_top_px(row) + p.row_height_px(row) / 2.0;
        assert!(
            (row_center - viewport_center).abs() <= p.row_height_px(row),
            "typewriter must center the cursor row (row_center={row_center}, viewport_center={viewport_center})"
        );

        // Near the document TOP there is no content above to center against, so
        // centering pins at row 0 — matching the minimal-adjust there exactly.
        assert_eq!(p.scroll_to_center_row(0, 800.0), 0);
        assert_eq!(p.scroll_to_center_row(p.visual_row_of(1, 0), 800.0), 0);
        assert_eq!(p.scroll_to_show_row(0, 0, 800.0), 0);
    }

    #[test]
    fn typewriter_pin_clamps_at_document_edges() {
        // The TYPEWRITER pin is `scroll_to_center_row` geometry composed with the
        // caller's `.min(max_scroll_rows())` clamp (the exact
        // composition in `app::viewstate::sync_view` + `capture::modes`). Prove the
        // edges: TOP pins at 0 (no content above), BODY centers strictly inside the
        // range, and the pin NEVER exceeds max_scroll (the safety clamp holds for
        // every row, including the last — centering can't pull the tail off-screen).
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping typewriter_pin_clamps_at_document_edges: no wgpu adapter");
            return;
        };
        let mut text = String::new();
        for i in 0..60 {
            text.push_str(&format!("line {i}\n"));
        }
        p.set_view(&view(&text, 0, 0));
        let total = p.total_visual_rows();
        assert!(total >= 60, "the doc must overflow the viewport");
        let max = p.max_scroll_rows(800.0);
        assert!(max > 0, "a doc taller than the viewport must be scrollable");

        // The pin the caller actually applies: center, then clamp to max_scroll.
        let pin = |row: usize| p.scroll_to_center_row(row, 800.0).min(max);

        // TOP: the first row pins at 0 (no content above to center against) — the
        // caret rides near the top edge naturally.
        assert_eq!(pin(0), 0, "a caret at row 0 pins to the document top");

        // BODY: a mid-document caret centers strictly inside (0, max), and the pin
        // never exceeds max_scroll.
        let mid_row = p.visual_row_of(30, 0);
        let mid = pin(mid_row);
        assert!(mid > 0 && mid < max, "a body caret centers between the edges (pin={mid}, max={max})");

        // The pin is MONOTONIC + BOUNDED across the whole document: moving the caret
        // down never scrolls up, and no row's pin ever exceeds max_scroll (the
        // `.min(max)` safety net holds even for the last row, so centering can never
        // strand the document tail past its bottom).
        let last = total - 1;
        let last_pin = pin(last);
        assert!(last_pin <= max, "the last row's pin stays within max_scroll");
        assert!(
            last_pin >= mid,
            "moving toward the bottom scrolls further down, never up (last={last_pin}, mid={mid})"
        );
        let mut prev = 0usize;
        for row in 0..total {
            let s = pin(row);
            assert!(s >= prev, "pin is monotonic non-decreasing in the row");
            assert!(s <= max, "pin never exceeds max_scroll at row {row}");
            prev = s;
        }
    }

    #[test]
    fn cursor_move_does_not_reshape() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping cursor_move_does_not_reshape: no wgpu adapter");
            return;
        };
        let text = "alpha\nbeta\ngamma\ndelta";
        // First push of this text reshapes once.
        p.set_view(&view(text, 0, 0));
        let after_first = p.reshape_count;
        // Move the cursor around the SAME text: no reshape may happen.
        p.set_view(&view(text, 1, 2));
        p.set_view(&view(text, 3, 0));
        p.set_view(&view(text, 2, 5));
        assert_eq!(
            p.reshape_count, after_first,
            "cursor-only changes must NOT trigger a reshape"
        );
        // A SCROLL-only change (different scroll_lines, same text) also must not.
        let mut scrolled = view(text, 2, 5);
        scrolled.scroll_lines = 1;
        p.set_view(&scrolled);
        assert_eq!(
            p.reshape_count, after_first,
            "scroll-only changes must NOT trigger a reshape"
        );
        // A SELECTION-only change must not reshape either.
        let mut selected = view(text, 2, 5);
        selected.selection = Some(((0, 0), (1, 2)));
        p.set_view(&selected);
        assert_eq!(
            p.reshape_count, after_first,
            "selection-only changes must NOT trigger a reshape"
        );
    }

    #[test]
    fn theme_font_switch_reshapes_document() {
        // The caret-x reads below fold BOTH globals: the theme font (the shaped
        // advances) AND the page state (`column_width()` folds `page_on()` /
        // `measure()` — geometry.rs — into the wrap width + text_left every x is
        // measured from). Other tests flip the page globals under page::test_lock()
        // (measure 15/40/50…), so reading them here with only the theme lock raced
        // a parallel page write — the historical parallel-run flake of this very
        // test. Hold both, in the suite-wide theme → page order (see page::test_lock()'s doc).
        // The caret x is also ANCHOR-keyed (Morph shifts one cell back, and with no
        // override the mode DEFAULTS off the active theme's font — proportional
        // Gumtree would flip it to Morph mid-test); hold the caret lock and pin
        // BLOCK so the x reads stay on the cursor cell across the world switches.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let _c = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::caret::set_mode(CaretMode::Block);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping theme_font_switch_reshapes_document: no wgpu adapter");
            return;
        };
        // Start on a MONO world (IBM Plex Mono) so the caret x is on a fixed cell.
        theme::set_active_by_name("Tawny").unwrap();
        p.sync_theme();
        let text = "The quick brown fox";
        // Place the caret 10 chars in (on the 'b' of "brown").
        p.set_view(&view(text, 0, 10));
        let mono_x = p.caret_target_xy().0;
        let reshapes_before = p.reshape_count;

        // Switch to a PROPORTIONAL serif world (Literata). sync_theme must reshape
        // the document in the new family (text + zoom unchanged) so the glyph shapes
        // — and the real advances — change.
        theme::set_active_by_name("Gumtree").unwrap();
        p.sync_theme();
        assert!(
            p.reshape_count > reshapes_before,
            "a theme font switch must reshape the document"
        );
        // The caret x is derived from the REAL shaped advances; on a proportional
        // face the cumulative advance to col 10 differs from the mono cell grid, so
        // the caret tracked the new advances rather than staying on the mono cell.
        let serif_x = p.caret_target_xy().0;
        assert!(
            (serif_x - mono_x).abs() > 1.0,
            "caret x must follow the proportional advances after a font switch \
             (mono={mono_x}, serif={serif_x})"
        );

        // A switch that leaves the SHAPED face unchanged must NOT reshape: the
        // document is already shaped in that family. With the taste-review face
        // swaps every world now names a UNIQUE display face, so the former
        // distinct-world-same-font pair (Quokka + Kingfisher, both IBM Plex Sans)
        // no longer exists; the realizable instance is a redundant switch to the
        // same world — sync_theme keys the reshape on the shaped face, not the call.
        theme::set_active_by_name("Quokka").unwrap();
        p.sync_theme();
        let n = p.reshape_count;
        theme::set_active_by_name("Quokka").unwrap(); // same world, already shaped
        p.sync_theme();
        assert_eq!(
            p.reshape_count, n,
            "a switch that leaves the shaped face unchanged must NOT reshape"
        );

        // Restore the default world so other tests see a clean global.
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// THE PREVIEW DEBOUNCE SPLIT (`sync_theme` = `sync_theme_colors` +
    /// `sync_theme_font`): the live theme-picker preview re-colors instantly per
    /// arrow and DEFERS the font reshape until the selection settles, so an arrow
    /// burst must cost ZERO reshapes until the one deferred `sync_theme_font` —
    /// which must land the IDENTICAL shaped state the synchronous `sync_theme`
    /// produces (the settled frame is byte-identical; the debounce only re-orders
    /// WHEN the reshape happens, never what it shapes). And the Esc-revert path
    /// (`retint_theme_now` = a full `sync_theme` on the restored world) must leave
    /// NOTHING for a stray deferred fire to do — a late `sync_theme_font` after
    /// the revert is a strict no-op.
    #[test]
    fn theme_preview_color_split_defers_reshape_and_revert_leaves_none() {
        // Shaping folds the theme font AND the page wrap globals; hold both locks
        // (theme → page order, page.rs:95-99). The caret-x equality below is also
        // ANCHOR-keyed (with no override the mode defaults off the active theme's
        // font — proportional Quokka would latch Morph and shift the x one cell);
        // hold the caret lock and pin BLOCK so both pipelines anchor identically.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let _c = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::caret::set_mode(CaretMode::Block);
        let Some(mut p) = headless_pipeline() else {
            eprintln!(
                "skipping theme_preview_color_split_defers_reshape_and_revert_leaves_none: no wgpu adapter"
            );
            return;
        };
        let text = "The quick brown fox";

        // Open on a MONO world; the doc shapes in IBM Plex Mono.
        theme::set_active_by_name("Tawny").unwrap();
        p.sync_theme();
        p.set_view(&view(text, 0, 10));
        let n = p.reshape_count;

        // ARROW BURST (the live preview path): colors only, per hop. No hop may
        // reshape; the doc stays shaped in the opening face while the pending
        // font change is visible via `needs_theme_reshape`.
        for world in ["Gumtree", "Bilby", "Saltpan", "Quokka"] {
            theme::set_active_by_name(world).unwrap();
            p.sync_theme_colors();
        }
        assert_eq!(
            p.reshape_count, n,
            "a color-only preview burst must not reshape the document"
        );
        assert_eq!(p.shaped_font, "IBM Plex Mono", "still shaped in the opening face");
        assert!(
            p.needs_theme_reshape(),
            "the deferred font change is pending (Quokka is Fira Sans)"
        );

        // SETTLE: the one deferred reshape lands. Exactly one reshape, and the
        // shaped state is identical to the synchronous `sync_theme` route.
        p.sync_theme_font();
        assert_eq!(p.reshape_count, n + 1, "the settle pays exactly ONE reshape");
        assert_eq!(p.shaped_font, "Fira Sans");
        let deferred_x = p.caret_target_xy().0;
        let Some(mut q) = headless_pipeline() else { return };
        q.sync_theme(); // synchronous full switch to the same (Quokka) world
        q.set_view(&view(text, 0, 10));
        assert_eq!(
            deferred_x,
            q.caret_target_xy().0,
            "the deferred reshape must land the same settled geometry as a synchronous sync_theme"
        );

        // ESC-REVERT with a pending deferral: previews colored ahead to Undertow,
        // then the revert applies the ORIGINAL world fully + synchronously (the
        // `retint_theme_now` path). The doc is already shaped in that face, so the
        // revert itself reshapes nothing — and a STRAY deferred fire afterwards
        // (the case the App cancels; harmless even if it raced through) no-ops.
        theme::set_active_by_name("Undertow").unwrap();
        p.sync_theme_colors();
        assert!(p.needs_theme_reshape(), "a deferral is pending toward EB Garamond");
        let m = p.reshape_count;
        theme::set_active_by_name("Quokka").unwrap(); // the world the picker opened on
        p.sync_theme(); // retint_theme_now: full, synchronous
        assert_eq!(p.reshape_count, m, "reverting to the shaped face reshapes nothing");
        p.sync_theme_font(); // the stray late fire
        assert_eq!(
            p.reshape_count, m,
            "a stray deferred reshape after the revert must be a strict no-op"
        );
        assert_eq!(p.shaped_font, "Fira Sans");

        // Restore the default world so other tests see a clean global.
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// PER-WORLD CODE MONO — `sync_theme` tracks the EFFECTIVE shaped face
    /// (`doc_family` — the world's mono on a CODE buffer, else its display font;
    /// render.rs), NOT the display font. On a code buffer a switch whose MONO
    /// changes (Quokka → Kingfisher: IBM Plex Mono vs JetBrains Mono) MUST retrack
    /// `shaped_font` to the new mono. The stronger, converse isolation: two worlds
    /// with DIFFERENT display faces but the SAME mono (Kingfisher → Mangrove — IBM
    /// Plex Sans vs JetBrains Mono display, both JetBrains Mono code) leave
    /// `shaped_font` UNCHANGED (the effective face didn't move) even though the
    /// display changed — proving the reshape/track gate keys on the MONO, not the
    /// display; the world switch still reshapes to re-bake the per-span syntax
    /// COLORS (`shaped_theme`, the same-face recolor path). (The taste-review face
    /// swaps left every world with a UNIQUE display face, so the former
    /// shared-display isolation — two worlds sharing ONE display sans — is no
    /// longer expressible; the same-mono / different-display leg carries the gate
    /// proof.) The PROSE reshape half is pinned by
    /// `theme_font_switch_reshapes_document` next door; this is the code half.
    #[test]
    fn code_mono_switch_reshapes_effective_face() {
        // Shaping folds the theme font AND the page wrap globals; hold both locks
        // (theme → page order, page.rs:95-99) so a parallel mutator can't flip
        // either between the reshape-count reads.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping code_mono_switch_reshapes_effective_face: no wgpu adapter");
            return;
        };

        // A CODE buffer on Quokka shapes in the world's mono companion.
        theme::set_active_by_name("Quokka").unwrap();
        p.sync_theme();
        assert_eq!(theme::active().font, "Fira Sans");
        assert_eq!(theme::active().mono, "IBM Plex Mono");
        let mut code = view("fn main() { let x = 1; }", 0, 0);
        code.syn_lang = Some(crate::syntax::Lang::Rust);
        p.set_view(&code);
        assert_eq!(
            p.shaped_font, "IBM Plex Mono",
            "a code buffer shapes in the world's mono, not its display sans"
        );
        let n = p.reshape_count;

        // Quokka → Kingfisher: the code MONO changes (IBM Plex Mono → JetBrains
        // Mono). The effective-face compare must see the mono change and reshape
        // the code buffer, retracking `shaped_font` to the new mono.
        theme::set_active_by_name("Kingfisher").unwrap();
        p.sync_theme();
        assert_eq!(theme::active().font, "IBM Plex Sans");
        assert_eq!(theme::active().mono, "JetBrains Mono");
        assert!(
            p.reshape_count > n,
            "a mono change must reshape a code buffer"
        );
        assert_eq!(
            p.shaped_font, "JetBrains Mono",
            "shaped_font tracks the NEW mono after the switch"
        );

        // Kingfisher → Mangrove: DIFFERENT display faces (IBM Plex Sans vs
        // JetBrains Mono) but the SAME code mono (both JetBrains Mono) — the
        // converse case. The code buffer is already shaped in the shared mono, so
        // the effective FACE is unchanged and `shaped_font` must NOT move even
        // though the display font did — proving the gate keys on the mono, not the
        // display. The WORLD (palette) DID change, so the switch still reshapes
        // once to re-bake the per-span syntax colors (`shaped_theme` — the
        // Magpie→Undertow stale-color fix), landing back on the same shared mono.
        let m = p.reshape_count;
        theme::set_active_by_name("Mangrove").unwrap();
        p.sync_theme();
        assert_ne!(
            theme::active().font,
            "IBM Plex Sans",
            "Mangrove's display face differs from Kingfisher's"
        );
        assert_eq!(theme::active().mono, "JetBrains Mono", "Mangrove shares Kingfisher's mono");
        assert!(
            p.reshape_count > m,
            "a world switch re-bakes span colors even when the code mono is shared"
        );
        assert_eq!(
            p.shaped_font, "JetBrains Mono",
            "the shared mono means the effective FACE is unchanged across the re-bake"
        );

        // Restore the default world so other tests see a clean global.
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// STALE SPAN-COLOR fix: per-span syntax/markdown/focus colors are BAKED into
    /// the buffer `AttrsList` at shape time, so a theme switch that keeps the SAME
    /// effective face (Magpie -> Undertow, both Monaspace Xenon, on a code buffer)
    /// used to skip the re-bake and leave those spans colored for the OLD world's
    /// derivation on the NEW ground. `sync_theme_font` now compares `shaped_theme`
    /// alongside `shaped_font`, so a same-face palette change still restyles and the
    /// baked color tracks the NEW world's `role_style_for`. Also pins the same-world
    /// no-op guard (a redundant `sync_theme` must not restyle).
    #[test]
    fn theme_switch_rebakes_span_colors_across_shared_effective_face() {
        // Shaping folds the theme font AND the page wrap globals; hold both locks
        // (theme → page order, page.rs:95-99).
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!(
                "skipping theme_switch_rebakes_span_colors_across_shared_effective_face: no wgpu adapter"
            );
            return;
        };

        // Magpie (light) and Undertow (dark) BOTH shape code in Monaspace Xenon, so a
        // code buffer's EFFECTIVE face is identical across the switch — the font
        // tracker alone would skip the reshape. Their palettes differ sharply (light
        // vs dark ink ladder), so the baked syntax colors MUST change.
        theme::set_active_by_name("Magpie").unwrap();
        p.sync_theme();
        assert_eq!(theme::active().mono, "Monaspace Xenon");
        let text = "let x = 42;";
        let mut code = view(text, 0, 0);
        code.syn_lang = Some(crate::syntax::Lang::Rust);
        p.set_view(&code);

        // Find a byte whose span carries a baked syntax COLOR (a role fg tint); the
        // exact offset doesn't matter, only that the SAME byte is re-read after the
        // switch (same text + lexer -> same role at that byte, only the derivation
        // moves).
        let colored_byte = (0..text.len())
            .find(|&b| {
                p.buffer.lines[0].attrs_list().get_span(b).color_opt.is_some()
            })
            .expect("a rust code buffer bakes at least one colored syntax span");
        let magpie_color = p.buffer.lines[0].attrs_list().get_span(colored_byte).color_opt;
        assert!(magpie_color.is_some());
        let n = p.reshape_count;

        // Switch to a SAME-effective-face world (Undertow, also Monaspace Xenon).
        theme::set_active_by_name("Undertow").unwrap();
        assert_eq!(
            theme::active().mono,
            "Monaspace Xenon",
            "the two worlds share the code face, so the font tracker alone would skip"
        );
        p.sync_theme();
        assert!(
            p.reshape_count > n,
            "a same-face world switch must still restyle to re-bake the span colors"
        );
        assert_eq!(
            p.shaped_font, "Monaspace Xenon",
            "the effective face is unchanged across the color re-bake"
        );
        let undertow_color = p.buffer.lines[0].attrs_list().get_span(colored_byte).color_opt;
        assert!(undertow_color.is_some());
        assert_ne!(
            magpie_color, undertow_color,
            "the baked syntax color must reflect the NEW world's role_style_for, not the old"
        );

        // SAME-world, same-face: a redundant `sync_theme` is a strict no-op (the
        // `shaped_theme == active_index()` guard mirrors the `shaped_font` one).
        let m = p.reshape_count;
        p.sync_theme();
        assert_eq!(p.reshape_count, m, "re-syncing the SAME world must not restyle");
        assert_eq!(
            p.buffer.lines[0].attrs_list().get_span(colored_byte).color_opt,
            undertow_color,
            "an idempotent re-sync leaves the baked color untouched"
        );

        // Restore the default world so other tests see a clean global.
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    #[test]
    fn heading_size_survives_theme_switch() {
        // Shaping folds the theme font AND the page wrap globals; hold both
        // (theme → page order, page.rs:95-99).
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping heading_size_survives_theme_switch: no wgpu adapter");
            return;
        };
        theme::set_active_by_name("Tawny").unwrap();
        p.sync_theme();
        let text = "# Big\n\nbody one\nbody two\n";
        let mut md = view(text, 0, 0);
        md.is_markdown = true;
        p.set_view(&md);
        let ratio_before = p.row_height_px(0) / p.row_height_px(2);
        assert!(ratio_before > 1.4, "sanity: heading taller before switch ({ratio_before})");

        // Switch to a DIFFERENT-font world: the heading must STAY bigger. The bug was
        // `sync_theme` rebuilding CJK-only attrs, which dropped the markdown styling
        // and shrank headings back to body size on a live theme switch.
        theme::set_active_by_name("Gumtree").unwrap();
        p.sync_theme();
        let ratio_after = p.row_height_px(0) / p.row_height_px(2);
        assert!(
            ratio_after > 1.4,
            "heading must stay larger than body after a theme/font switch ({ratio_after})"
        );

        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// BUG regression (user screenshot 2026-07-04): zooming with the caret ON a
    /// heading line left the amber block caret floating ~half a row above the
    /// glyphs while the text itself re-laid correctly. Root cause: `set_view`
    /// called `set_caret_target` (which reads the cursor's row geometry via
    /// `cursor_row_height`/`caret_cell_top`) BEFORE the zoom-triggered
    /// `restyle_all_lines` — so on a doc with headings, a zoom step reshaped body
    /// text at the new metrics while the heading line's ABSOLUTE per-span pixel
    /// metrics (set by the PREVIOUS restyle) were still stale until
    /// `restyle_all_lines` ran, moments later, with no caret-target recompute
    /// after it. The caret spring latched a target built from the transient,
    /// pre-restyle row geometry — and nothing ever asked it to recompute once the
    /// geometry settled.
    #[test]
    fn zoom_on_heading_line_keeps_caret_target_aligned() {
        // Shaping folds the theme font AND the page wrap globals; hold both
        // (theme -> page order, page.rs:95-99).
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping zoom_on_heading_line_keeps_caret_target_aligned: no wgpu adapter");
            return;
        };
        let text = "## h2\n\nbody one\nbody two\n";

        // 1) Open the markdown doc with the caret on a BODY line at zoom 1.0. The
        // md-flip restyle fires here, but the cursor's own row is a body row
        // (unaffected by heading scale), so this establishes a clean baseline.
        let mut v = view(text, 2, 0);
        v.is_markdown = true;
        v.zoom = 1.0;
        p.set_view(&v);

        // 2) Move the caret ONTO the heading line, zoom unchanged: a plain
        // cursor-move target update against already-settled heading geometry.
        let mut v2 = view(text, 0, 3);
        v2.is_markdown = true;
        v2.zoom = 1.0;
        p.set_view(&v2);
        let (_, target_before_zoom, _, _) = p.caret_snapshot();

        // 3) Zoom, caret still on the heading line. This is the exact repro: the
        // zoom step both rescales body metrics AND (because the doc has a
        // heading) triggers `restyle_all_lines` to rescale the heading's
        // absolute pixel metrics to match.
        let row0_h_before = p.row_height_px(0);
        let mut v3 = view(text, 0, 3);
        v3.is_markdown = true;
        v3.zoom = 1.6;
        p.set_view(&v3);
        let (_, target_after_zoom, _, _) = p.caret_snapshot();

        // Sanity: the heading row itself really did grow with the zoom (the
        // "text re-lays correctly" half of the bug report) — read fresh from the
        // settled row-geometry table, not the caret.
        let row0_h_after = p.row_height_px(0);
        assert!(
            row0_h_after > row0_h_before * 1.3,
            "sanity: a 1.6x zoom must actually grow the heading row's height \
             (before={row0_h_before} after={row0_h_after})"
        );
        let _ = target_before_zoom;

        // The pipeline's state is fully settled after `set_view` returns (the
        // conditional restyle, if any, has already run), so a FRESH read of the
        // pure `caret_target_xy()` reflects the true, post-restyle geometry —
        // independent of whatever order `set_view` computed things in. The
        // caret's LATCHED spring target must agree with it.
        let (correct_x, correct_y) = p.caret_target_xy();
        assert!(
            (target_after_zoom.0 - correct_x).abs() < 0.5,
                "caret target x must match the settled heading-row geometry \
             (latched={:?}, correct=({correct_x}, {correct_y}))",
            target_after_zoom
        );
        assert!(
            (target_after_zoom.1 - correct_y).abs() < 0.5,
            "caret target y must match the settled heading-row geometry, not a \
             stale pre-restyle row height (latched={:?}, correct=({correct_x}, {correct_y}))",
            target_after_zoom
        );
    }

    /// MONO FIX regression: the mono worlds (IBM Plex Mono) must shape in TRUE
    /// monospace — a line of all-'i' and a line of all-'m' have the SAME, uniform
    /// glyph pitch. The bug (a default Weight-400 request dropping the bundled
    /// Light face and falling through to proportional `.SF NS`) made i ~5px / m
    /// ~19px; the `mono_safe_weight(300)` fix realigns the request with the face.
    /// Contrast a proportional world (Literata) where i and m differ by design.
    #[test]
    fn mono_world_shapes_uniform_pitch() {
        // Pitch reads fold the theme font AND the page wrap globals (a mid-test
        // measure write would re-wrap the lines); hold both (theme → page).
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping mono_world_shapes_uniform_pitch: no wgpu adapter");
            return;
        };
        // Advance between consecutive glyph xs (the per-column pitch). A line of N
        // identical chars yields N+1 xs (the last is the end-of-line caret slot).
        let pitch = |xs: &[f32]| -> f32 {
            assert!(xs.len() >= 3, "need a few glyphs to measure pitch");
            xs[1] - xs[0]
        };
        let uniform = |xs: &[f32]| -> bool {
            let p0 = xs[1] - xs[0];
            xs.windows(2).all(|w| (w[1] - w[0] - p0).abs() < 0.5)
        };

        // MONO world: i-pitch == m-pitch, and each line is internally uniform.
        theme::set_active_by_name("Tawny").unwrap();
        p.sync_theme();
        p.set_view(&view("iiiiiiiiii", 0, 0));
        let xs_i = p.line_glyph_xs(0);
        p.set_view(&view("mmmmmmmmmm", 0, 0));
        let xs_m = p.line_glyph_xs(0);
        let (pi, pm) = (pitch(&xs_i), pitch(&xs_m));
        assert!(
            uniform(&xs_i) && uniform(&xs_m),
            "mono world: each line must have uniform internal pitch (i={pi}, m={pm})"
        );
        assert!(
            (pi - pm).abs() < 0.5,
            "mono world must shape i and m at the SAME pitch (i={pi}, m={pm}); \
             a proportional fallback would give i<<m"
        );

        // PROPORTIONAL world (Literata): i and m have visibly different advances —
        // proves the test actually discriminates mono from proportional shaping.
        theme::set_active_by_name("Gumtree").unwrap();
        p.sync_theme();
        p.set_view(&view("iiiiiiiiii", 0, 0));
        let pi2 = pitch(&p.line_glyph_xs(0));
        p.set_view(&view("mmmmmmmmmm", 0, 0));
        let pm2 = pitch(&p.line_glyph_xs(0));
        assert!(
            (pi2 - pm2).abs() > 1.0,
            "proportional world should give i != m (i={pi2}, m={pm2})"
        );

        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// THE NEVER-TOFU LAW (font-DB half — complements `theme::tests::
    /// every_font_id_has_a_nonempty_candidate_ladder_on_every_world`'s
    /// structural check): `FontId::Latin` and `FontId::Ja` resolve to a
    /// CONCRETELY-REGISTERED face via the real font DB on EVERY world, in a
    /// normal build — the guaranteed floor. Both ladders' first candidate is
    /// always a bundled embedded face (the world's own `Theme::font` for
    /// Latin; bundled Noto Serif/Sans JP for Ja — see `theme::CJK_MINCHO`/
    /// `CJK_GOTHIC`), so this never depends on what's installed on the
    /// machine running the test. zh-Hans/zh-Hant/ko are NOT asserted here —
    /// v1 ships no bundled asset for them, so whether they resolve is
    /// genuinely machine-dependent (the documented degenerate path: `None` ->
    /// no span added -> cosmic-text's neutral fallback, never a panic).
    #[test]
    fn latin_and_ja_always_resolve_to_an_embedded_face() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping latin_and_ja_always_resolve_to_an_embedded_face: no wgpu adapter");
            return;
        };
        for t in theme::THEMES.iter() {
            theme::set_active_by_name(t.name).unwrap();
            p.sync_theme();
            assert!(
                p.resolve_font_id(theme::FontId::Latin).is_some(),
                "{}: Latin must always resolve (its own embedded display face)",
                t.name
            );
            assert!(
                p.resolve_font_id(theme::FontId::Ja).is_some(),
                "{}: Ja must always resolve (bundled Noto Serif/Sans JP)",
                t.name
            );
        }
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// NEVER-TOFU (per-world ORNAMENT FACE): every world's three section-break
    /// glyphs (`Ornaments::dash`/`star`/`underscore`) resolve to a REAL glyph in
    /// that world's assigned [`theme::Theme::ornament_face`] — no world can ship a
    /// fleuron its own ornament face lacks (the ⁂/❡/❥-not-in-EB-Garamond trap). The
    /// font-DB half of the structural `theme::tests::
    /// every_world_ornament_face_is_a_registered_ornament_face` law. Also pins the
    /// design-table contract that the three glyphs are DISTINCT per world (dash /
    /// star / underscore each read as their own symbol, never a shared fallback).
    #[test]
    fn ornament_glyphs_resolve_in_each_worlds_assigned_face() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping ornament_glyphs_resolve_in_each_worlds_assigned_face: no wgpu adapter");
            return;
        };
        for t in theme::THEMES.iter() {
            let (d, s, u) = (t.ornaments.dash, t.ornaments.star, t.ornaments.underscore);
            assert!(
                d != s && s != u && d != u,
                "{}: ornament trio must be THREE DISTINCT glyphs, got dash={:?} star={:?} underscore={:?}",
                t.name,
                d,
                s,
                u
            );
            let id = p
                .font_system
                .db()
                .faces()
                .find(|f| f.families.iter().any(|(n, _)| n == t.ornament_face))
                .map(|f| f.id)
                .unwrap_or_else(|| panic!("{}: ornament face {:?} is registered", t.name, t.ornament_face));
            let font = p
                .font_system
                .get_font(id, glyphon::cosmic_text::fontdb::Weight::NORMAL)
                .unwrap_or_else(|| panic!("{}: ornament face {:?} loads", t.name, t.ornament_face));
            let charmap = font.as_swash().charmap();
            for (label, ch) in [
                ("dash `---`", t.ornaments.dash),
                ("star `***`", t.ornaments.star),
                ("underscore `___`", t.ornaments.underscore),
            ] {
                assert!(
                    charmap.map(ch) != 0,
                    "{}: {} glyph {:?} (U+{:04X}) is NOT in its ornament face {:?} — renders as tofu",
                    t.name,
                    label,
                    ch,
                    ch as u32,
                    t.ornament_face
                );
            }
        }
    }

    /// THE CHINESE ROUND extends the never-tofu floor to `ZhHans`/`Ko`: since
    /// both now bundle a face too (Noto Serif/Sans SC + LXGW WenKai for
    /// zh-Hans; Noto Sans KR for ko — `render::FONT_ZH_KO_FACES`), they
    /// resolve on EVERY world in a normal build, exactly like Latin/Ja.
    /// `ZhHant` is deliberately NOT asserted here — it still ships no bundled
    /// asset this round (Big5 subsetting is banked), so whether it resolves
    /// stays genuinely machine-dependent (the documented degenerate path).
    #[test]
    fn zh_hans_and_ko_always_resolve_to_an_embedded_face() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping zh_hans_and_ko_always_resolve_to_an_embedded_face: no wgpu adapter");
            return;
        };
        for t in theme::THEMES.iter() {
            theme::set_active_by_name(t.name).unwrap();
            p.sync_theme();
            assert!(
                p.resolve_font_id(theme::FontId::ZhHans).is_some(),
                "{}: ZhHans must always resolve (bundled Noto Serif/Sans SC or LXGW WenKai)",
                t.name
            );
            assert!(
                p.resolve_font_id(theme::FontId::Ko).is_some(),
                "{}: Ko must always resolve (bundled Gowun Batang on serif worlds, else Noto Sans KR)",
                t.name
            );
        }
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// PER-FACE registration: each of the Chinese round's four bundled faces
    /// registers in the font DB under its exact expected family name (the
    /// same "verified through fontdb" guarantee `FONT_CJK_FACES`'s JP pair
    /// already carries) — a subsetting/instancing mistake that silently
    /// renamed or corrupted a face would fail this immediately rather than
    /// surfacing as a confusing tofu box downstream.
    #[test]
    fn zh_ko_faces_register_under_their_expected_family_names() {
        let Some(p) = headless_pipeline() else {
            eprintln!("skipping zh_ko_faces_register_under_their_expected_family_names: no wgpu adapter");
            return;
        };
        for expected in ["Noto Serif SC", "Noto Sans SC", "Noto Sans KR", "LXGW WenKai"] {
            let registered = p
                .font_system
                .db()
                .faces()
                .any(|f| f.families.iter().any(|(n, _)| n == expected));
            assert!(registered, "{expected:?} must be registered in the font DB");
        }
    }

    /// PER-FACE registration (Phase 2 "JP face variety" round): each of the
    /// three new bundled JP faces ([`render::FONT_JA_VARIETY_FACES`]) registers
    /// under its exact expected family name — the same "verified through fontdb"
    /// guarantee the Noto/Chinese faces carry. A subsetting mistake that renamed
    /// or corrupted a face fails HERE, not as a downstream tofu box.
    #[test]
    fn ja_variety_faces_register_under_their_expected_family_names() {
        let Some(p) = headless_pipeline() else {
            eprintln!("skipping ja_variety_faces_register_under_their_expected_family_names: no wgpu adapter");
            return;
        };
        for expected in ["Shippori Mincho", "Zen Maru Gothic", "Klee One"] {
            let registered = p
                .font_system
                .db()
                .faces()
                .any(|f| f.families.iter().any(|(n, _)| n == expected));
            assert!(registered, "{expected:?} must be registered in the font DB");
        }
    }

    /// Phase 2 "JP face variety": each reassigned world's `FontId::Ja` resolves
    /// to its NEW bundled face on the real font DB (machine-independent, since
    /// each ladder names the bundled face FIRST) — the font-DB half of the
    /// `theme::tests::cjk_fallback_matches_world_character` structural law. This
    /// is the fact the capture test asserts through the sidecar, proven here at
    /// the purest reachable seam.
    #[test]
    fn ja_variety_worlds_resolve_their_new_bundled_face() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping ja_variety_worlds_resolve_their_new_bundled_face: no wgpu adapter");
            return;
        };
        // (world, expected FontId::Ja family) — one per new ladder, both members.
        let cases = [
            ("Gumtree", "Shippori Mincho"),
            ("Bilby", "Shippori Mincho"),
            ("Undertow", "Shippori Mincho"),
            ("Galah", "Zen Maru Gothic"),
            ("Kingfisher", "Zen Maru Gothic"),
            ("Mopoke", "Klee One"),
            ("Quokka", "Klee One"),
            // Two worlds this round left ALONE keep the neutral Noto face.
            ("Saltpan", "Noto Serif JP"),
            ("Currawong", "Noto Sans JP"),
        ];
        for (world, want) in cases {
            theme::set_active_by_name(world).unwrap();
            p.sync_theme();
            let (fam, _) = p
                .resolve_font_id(theme::FontId::Ja)
                .unwrap_or_else(|| panic!("{world}: Ja must resolve"));
            assert_eq!(fam, want, "{world}: Ja should resolve to {want}");
        }
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// PER-FACE registration ("CJK companions" round): the one bundled Korean
    /// serif companion ([`render::FONT_CJK_COMPANION_FACES`]) registers under its
    /// exact expected family name — the same "verified through fontdb" guarantee
    /// the JP/ZH faces carry. A subsetting/rename mistake fails HERE, not as a
    /// downstream tofu box.
    #[test]
    fn ko_companion_face_registers_under_its_family_name() {
        let Some(p) = headless_pipeline() else {
            eprintln!("skipping ko_companion_face_registers_under_its_family_name: no wgpu adapter");
            return;
        };
        let registered = p
            .font_system
            .db()
            .faces()
            .any(|f| f.families.iter().any(|(n, _)| n == "Gowun Batang"));
        assert!(registered, "\"Gowun Batang\" must be registered in the font DB");
    }

    /// "CJK companions" round: each SERIF world's `FontId::Ko` resolves to the
    /// bundled Gowun Batang on the real font DB (machine-independent — the serif
    /// ko ladder names it FIRST), while a SANS/MONO world's `Ko` stays the
    /// neutral Noto Sans KR floor. The font-DB half of the
    /// `theme::tests::zh_hant_uniform_ko_splits_serif_from_sans` structural law,
    /// proven at the purest reachable seam (mirrors
    /// `ja_variety_worlds_resolve_their_new_bundled_face`).
    #[test]
    fn ko_serif_worlds_resolve_gowun_batang() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping ko_serif_worlds_resolve_gowun_batang: no wgpu adapter");
            return;
        };
        // (world, expected FontId::Ko family) — serif worlds get Gowun Batang;
        // two sans/mono controls keep the Noto Sans KR floor.
        let cases = [
            ("Gumtree", "Gowun Batang"),
            ("Bilby", "Gowun Batang"),
            ("Undertow", "Gowun Batang"),
            ("Saltpan", "Gowun Batang"),
            ("Outback", "Gowun Batang"),
            ("Magpie", "Gowun Batang"),
            // Sans/mono controls — the neutral bundled floor, never Gowun Batang.
            ("Currawong", "Noto Sans KR"),
            ("Kingfisher", "Noto Sans KR"),
        ];
        for (world, want) in cases {
            theme::set_active_by_name(world).unwrap();
            p.sync_theme();
            let (fam, _) = p
                .resolve_font_id(theme::FontId::Ko)
                .unwrap_or_else(|| panic!("{world}: Ko must resolve"));
            assert_eq!(fam, want, "{world}: Ko should resolve to {want}");
        }
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// The 10 proportional display families each ship in `render::FONT_THEME_BOLD_FACES`,
    /// but a bold face only fixes the `weight_diff == 0` fallback trap if it registers
    /// under the SAME family name its Regular uses AND declares usWeightClass 700 — a
    /// subsetting/name-fixup mistake (the exact failure the CJK round guards against for
    /// its faces) that renamed the face or left it at weight 400 would silently keep the
    /// mono-fallback bug. This asserts the font-DB fact directly for all 10 (including
    /// Fira Sans / Bitter, which are registered but not yet assigned to any world, so the
    /// resolution test below can't reach them through a theme switch).
    #[test]
    fn bold_display_faces_register_under_their_family_names_at_weight_700() {
        let Some(p) = headless_pipeline() else {
            eprintln!("skipping bold_display_faces_register_under_their_family_names_at_weight_700: no wgpu adapter");
            return;
        };
        for expected in [
            "Literata",
            "Newsreader 16pt 16pt",
            "IBM Plex Sans",
            "Zilla Slab",
            "Figtree",
            "iA Writer Quattro S",
            "Fraunces 9pt",
            "EB Garamond",
            "Fira Sans",
            "Bitter",
        ] {
            let has_bold = p.font_system.db().faces().any(|f| {
                f.weight.0 == 700 && f.families.iter().any(|(n, _)| n == expected)
            });
            assert!(
                has_bold,
                "a weight-700 face must be registered under {expected:?} (the family its \
                 Regular uses) — else a `**bold**` request trips the weight_diff==0 mono trap"
            );
        }
    }

    /// THE `**bold**` REGRESSION, resolved through the REAL font system: shaping bold
    /// markdown on a world whose display face is one of the 10 bundled bolds must
    /// resolve the bold content glyphs to a WEIGHT-700, NON-MONOSPACE face — never
    /// cosmic-text's mono fallback (the shipping bug: with only the 400 Regular present,
    /// a `Weight::BOLD` request drops the proportional face via `|400-700| == 300` and
    /// lands in Menlo/Monaspace). Iterates every world whose `Theme::font` is a bundled
    /// bold family and inspects the shaped `layout_runs`, mapping each bold-content
    /// glyph's `font_id` back to its `FaceInfo`. `!monospaced` is the load-bearing
    /// assertion — the mono fallback is the exact failure signature.
    #[test]
    fn markdown_bold_resolves_to_a_real_bold_face_never_the_mono_fallback() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping markdown_bold_resolves_to_a_real_bold_face_never_the_mono_fallback: no wgpu adapter");
            return;
        };
        let bold_families = [
            "Literata",
            "Newsreader 16pt 16pt",
            "IBM Plex Sans",
            "Zilla Slab",
            "Figtree",
            "iA Writer Quattro S",
            "Fraunces 9pt",
            "EB Garamond",
        ];
        let mut checked = 0usize;
        for t in theme::THEMES.iter() {
            if !bold_families.contains(&t.font) {
                continue; // mono worlds stay Regular-only; unassigned faces covered above
            }
            theme::set_active_by_name(t.name).unwrap();
            p.sync_theme();
            // Bold on line 1 (line 0 blank), caret parked on line 0 — off the bold
            // line, so WYSIWYG conceal is inert; weight applies regardless. Content
            // "bold" is line-relative bytes 2..6 of "**bold**".
            p.set_view(&view_md("\n**bold**", 0, 0));
            let mut saw_glyph = false;
            for run in p.buffer.layout_runs() {
                if run.line_i != 1 {
                    continue;
                }
                for g in run.glyphs.iter() {
                    if g.start < 2 || g.start >= 6 {
                        continue; // only the "bold" content, not the `**` delimiters
                    }
                    let face = p
                        .font_system
                        .db()
                        .face(g.font_id)
                        .expect("shaped glyph maps to a registered face");
                    assert_eq!(
                        face.families[0].0, t.font,
                        "{}: bold content glyph resolved to {:?}, not the world face {:?}",
                        t.name, face.families[0].0, t.font
                    );
                    assert_eq!(
                        face.weight.0, 700,
                        "{}: bold content glyph resolved to weight {}, not 700",
                        t.name, face.weight.0
                    );
                    assert!(
                        !face.monospaced,
                        "{}: bold content glyph fell to a MONOSPACE face — the weight_diff==0 mono-fallback bug",
                        t.name
                    );
                    saw_glyph = true;
                }
            }
            assert!(saw_glyph, "{}: found no bold content glyph to check", t.name);
            checked += 1;
        }
        assert!(checked >= 8, "expected to check all 8 assigned bold worlds, checked {checked}");
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// THE bold/italic-breaks-Japanese REGRESSION, resolved through the REAL font
    /// system: shaping `**bold**` / `*italic*` / `***bold-italic***` Japanese must
    /// resolve every CJK content glyph to the world's BUNDLED JP face at its
    /// registered Weight 400 / Normal style — NEVER a heavier / slanted / mono /
    /// system fallback. The failure signature the fix guards against: a markdown
    /// emphasis span sets `Weight(700)` / `Style::Italic`, and without the
    /// script-span layer's weight+style PIN (see `spans::add_script_spans`) that
    /// request drops the 400/Normal-only bundled face (`weight_diff != 0` +
    /// style-mismatch) and tofu/system-falls mid-sentence. Checks a serif world
    /// (Undertow → Shippori Mincho, its Phase-2 ja override) and a sans world
    /// (Currawong → Noto Sans JP) — `want_fam` is read dynamically from the
    /// resolver, so it tracks each world's assigned face rather than a literal;
    /// caret parked on the blank line 0, so the styled lines are OFF-cursor (their
    /// `**`/`*` markers conceal — the emphasis weight/style still applies to the
    /// content, which is exactly the run under test).
    #[test]
    fn markdown_emphasis_keeps_the_bundled_cjk_face_never_a_fallback() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping markdown_emphasis_keeps_the_bundled_cjk_face_never_a_fallback: no wgpu adapter");
            return;
        };
        for world in ["Undertow", "Currawong"] {
            theme::set_active_by_name(world).unwrap();
            p.sync_theme();
            let (want_fam, _) = p
                .resolve_font_id(theme::FontId::Ja)
                .expect("Ja must resolve to a bundled face");
            // line 0 blank (caret here); line 1 bold, line 2 italic, line 3 bold-italic.
            let text = "\n**太字**\n*斜体*\n***両方***";
            p.set_view(&view_md(text, 0, 0));
            let lines: Vec<String> =
                p.buffer.lines.iter().map(|l| l.text().to_string()).collect();
            let mut checked = 0usize;
            for run in p.buffer.layout_runs() {
                if run.line_i == 0 {
                    continue;
                }
                let lt = &lines[run.line_i];
                for g in run.glyphs.iter() {
                    let ch = lt.get(g.start..g.end).unwrap_or("");
                    if !ch.chars().next().map(super::spans::is_cjk).unwrap_or(false) {
                        continue; // skip the `**`/`*` delimiter glyphs, only CJK content
                    }
                    let face = p
                        .font_system
                        .db()
                        .face(g.font_id)
                        .expect("shaped glyph maps to a registered face");
                    assert_eq!(
                        face.families[0].0, want_fam,
                        "{world}: emphasized CJK glyph {ch:?} resolved to {:?}, not the bundled JP face {want_fam:?}",
                        face.families[0].0
                    );
                    assert_eq!(
                        face.weight.0, 400,
                        "{world}: emphasized CJK glyph {ch:?} resolved to weight {} — the bold(700) leaked past the pin",
                        face.weight.0
                    );
                    assert!(
                        matches!(face.style, glyphon::cosmic_text::fontdb::Style::Normal),
                        "{world}: emphasized CJK glyph {ch:?} resolved to a slanted style {:?} — the italic leaked past the pin",
                        face.style
                    );
                    assert!(
                        !face.monospaced,
                        "{world}: emphasized CJK glyph {ch:?} fell to a MONOSPACE fallback",
                    );
                    checked += 1;
                }
            }
            assert!(
                checked >= 6,
                "{world}: expected the 6 emphasized CJK content glyphs, checked {checked}"
            );
        }
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// The bundled text + ornament faces (Fira Sans, Iosevka, Bitter, Junicode)
    /// and the rebuilt symbol face (Awl Marks) must each resolve under their
    /// expected registered family name — so they are addressable via `Family::Name`
    /// (the section-break fleuron / About end-mark now names Junicode this way), and
    /// a renamed/corrupted face fails here rather than surfacing as downstream tofu.
    /// (Vollkorn-Ornaments was dropped — it shipped no classic fleurons, so no world
    /// could use it for a section break.)
    #[test]
    fn bundled_text_and_ornament_faces_register_under_their_family_names() {
        let Some(p) = headless_pipeline() else {
            eprintln!("skipping bundled_text_and_ornament_faces_register_under_their_family_names: no wgpu adapter");
            return;
        };
        for expected in ["Fira Sans", "Iosevka", "Bitter", "Junicode", "Awl Marks"] {
            let registered = p
                .font_system
                .db()
                .faces()
                .any(|f| f.families.iter().any(|(n, _)| n == expected));
            assert!(registered, "{expected:?} must be registered in the font DB");
        }
    }

    // ── i18n render resolution ladder (`add_script_spans` / `ScriptFonts`) ────
    //
    // Pure-function tests over a fabricated `ScriptFonts` (no real font DB / GPU
    // needed — `add_script_spans` does zero font-DB work itself, just the
    // per-run ladder + span laying), inspecting the resulting `AttrsList` via
    // `get_span` (the same introspection `rects.rs`'s conceal tests already use).

    fn family_name(al: &glyphon::cosmic_text::AttrsList, byte: usize) -> Option<String> {
        match al.get_span(byte).family {
            Family::Name(n) => Some(n.to_string()),
            _ => None,
        }
    }

    #[test]
    fn add_script_spans_ja_tagged_doc_with_hangul_run_uses_ko_not_ja() {
        // THE task-spec example verbatim: a ja-tagged doc with an embedded
        // hangul run. Step (a) has no ko mapping for a `ja` tag -> falls to
        // step (b): the run's OWN script (hangul -> ko).
        let fonts = super::text::ScriptFonts {
            ja: Some(("JaFace", glyphon::Weight(400))),
            zh_hans: None,
            zh_hant: None,
            ko: Some(("KoFace", glyphon::Weight(400))),
        };
        let base = Attrs::new();
        let text = "한글"; // pure hangul
        let mut al = glyphon::cosmic_text::AttrsList::new(&base);
        add_script_spans(
            &mut al, text, &base, Some(crate::frontmatter::Lang::Ja),
            &crate::frontmatter::DEFAULT_CJK_PRIORITY, &fonts,
        );
        assert_eq!(family_name(&al, 0), Some("KoFace".to_string()));
    }

    #[test]
    fn add_script_spans_ja_tagged_doc_with_han_run_uses_ja() {
        // A ja tag DOES map Han (kanji) -> its own step (a) mapping wins.
        let fonts = super::text::ScriptFonts {
            ja: Some(("JaFace", glyphon::Weight(400))),
            zh_hans: Some(("ZhHansFace", glyphon::Weight(400))),
            zh_hant: None,
            ko: None,
        };
        let base = Attrs::new();
        let text = "日本語"; // pure han (kanji)
        let mut al = glyphon::cosmic_text::AttrsList::new(&base);
        add_script_spans(
            &mut al, text, &base, Some(crate::frontmatter::Lang::Ja),
            &crate::frontmatter::DEFAULT_CJK_PRIORITY, &fonts,
        );
        assert_eq!(family_name(&al, 0), Some("JaFace".to_string()));
    }

    #[test]
    fn add_script_spans_untagged_han_uses_cjk_priority_tiebreak() {
        // No doc tag at all: an untagged Han-only run falls to (c), the
        // cjk_priority ladder — here configured zh-Hans-first.
        let fonts = super::text::ScriptFonts {
            ja: Some(("JaFace", glyphon::Weight(400))),
            zh_hans: Some(("ZhHansFace", glyphon::Weight(400))),
            zh_hant: None,
            ko: None,
        };
        let base = Attrs::new();
        let text = "汉字";
        let priority = [
            crate::frontmatter::Lang::ZhHans,
            crate::frontmatter::Lang::Ja,
            crate::frontmatter::Lang::ZhHant,
            crate::frontmatter::Lang::Ko,
        ];
        let mut al = glyphon::cosmic_text::AttrsList::new(&base);
        add_script_spans(&mut al, text, &base, None, &priority, &fonts);
        assert_eq!(family_name(&al, 0), Some("ZhHansFace".to_string()));
    }

    #[test]
    fn add_script_spans_mixed_run_each_script_resolves_independently() {
        // "hi漢字ですは" -- latin "hi" (untouched), han "漢字" (-> ja tag),
        // kana "ですは" (-> ja, unambiguous) — every script resolves per-run.
        let fonts = super::text::ScriptFonts {
            ja: Some(("JaFace", glyphon::Weight(400))),
            zh_hans: None,
            zh_hant: None,
            ko: None,
        };
        let base = Attrs::new();
        let text = "hi漢字ですは";
        let mut al = glyphon::cosmic_text::AttrsList::new(&base);
        add_script_spans(
            &mut al, text, &base, Some(crate::frontmatter::Lang::Ja),
            &crate::frontmatter::DEFAULT_CJK_PRIORITY, &fonts,
        );
        // "hi" (bytes 0..2): no override -> base family (no Name span).
        assert_eq!(family_name(&al, 0), None, "the latin run must not be overridden");
        // "漢" starts at byte 2 (han).
        assert_eq!(family_name(&al, 2), Some("JaFace".to_string()));
        // "で" starts after "漢字" (2 kanji, 3 bytes each = byte 8) (kana).
        assert_eq!(family_name(&al, 8), Some("JaFace".to_string()));
    }

    #[test]
    fn add_script_spans_unresolved_script_leaves_base_face() {
        // zh-Hans has NO candidate resolved on this machine (`None`) — the
        // documented degenerate case: no override span, base face wins.
        let fonts = super::text::ScriptFonts { ja: None, zh_hans: None, zh_hant: None, ko: None };
        let base = Attrs::new();
        let text = "汉字";
        let mut al = glyphon::cosmic_text::AttrsList::new(&base);
        add_script_spans(&mut al, text, &base, None, &crate::frontmatter::DEFAULT_CJK_PRIORITY, &fonts);
        assert_eq!(family_name(&al, 0), None, "no candidate resolved -> no override span");
    }

    #[test]
    fn add_script_spans_pins_weight_and_style_over_bold_italic_base() {
        // THE bold/italic-breaks-Japanese fix at its purest seam: a CJK run must
        // resolve to its face's REGISTERED weight+style (400/Normal for every
        // bundled CJK face — no bold/italic CJK cut exists in v1), NEVER a
        // `**bold**`(700) / `*italic*` emphasis leaking onto it. Model the worst
        // case explicitly — a base ALREADY carrying Weight::BOLD + Style::Italic
        // (as if an emphasis span sat under the run) — and assert the script span
        // overwrites BOTH. Pre-fix the weight was pinned but the STYLE was
        // inherited from the base, so the italic leaked; the `.style(Normal)` pin
        // closes it.
        let fonts = super::text::ScriptFonts {
            ja: Some(("JaFace", glyphon::Weight(400))),
            zh_hans: None,
            zh_hant: None,
            ko: None,
        };
        let base = Attrs::new()
            .weight(glyphon::Weight::BOLD)
            .style(glyphon::Style::Italic);
        let text = "太字"; // pure kanji
        let mut al = glyphon::cosmic_text::AttrsList::new(&base);
        add_script_spans(
            &mut al,
            text,
            &base,
            Some(crate::frontmatter::Lang::Ja),
            &crate::frontmatter::DEFAULT_CJK_PRIORITY,
            &fonts,
        );
        let a = al.get_span(0);
        assert_eq!(family_name(&al, 0), Some("JaFace".to_string()), "CJK run keeps its resolved face");
        assert_eq!(a.weight, glyphon::Weight(400), "weight pinned to the resolved face's 400, not the bold 700");
        assert_eq!(a.style, glyphon::Style::Normal, "style pinned to Normal, not the italic base");
    }

    // --- Table GRID cells render INLINE markdown (bold/italic/code), markers
    // gone --- (`spans::cell_inline_attrs`, the tables-v1 styled off-cursor cell).
    // Pure `AttrsList` inspection via `get_span`, mirroring the add_script_spans
    // tests above. A concealed marker byte reads as transparent ink (alpha 0) —
    // the same "concealed?" idiom `rects.rs` uses — proving the raw `*`/`` ` ``
    // no longer draw.

    #[test]
    fn table_cell_bold_marker_conceals_and_content_is_bold() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(true);
        let base = Attrs::new();
        // "**bold**": `*`=0,`*`=1, "bold"=2..6, `*`=6,`*`=7.
        let al = cell_inline_attrs(&base, 20.0, "**bold**");
        // Content shapes in the real BOLD weight (the world's bundled 700 face).
        assert_eq!(al.get_span(2).weight.0, 700, "the cell content is bold weight");
        // The `**` delimiters are concealed (transparent ink) — no literal asterisks.
        assert!(
            matches!(al.get_span(0).color_opt, Some(c) if c.a() == 0),
            "leading `**` marker is concealed (transparent)"
        );
        assert!(
            matches!(al.get_span(7).color_opt, Some(c) if c.a() == 0),
            "trailing `**` marker is concealed (transparent)"
        );
        crate::markdown::set_wysiwyg_on(true);
    }

    #[test]
    fn table_cell_italic_marker_conceals_and_content_is_italic() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(true);
        let base = Attrs::new();
        // "*x*": `*`=0, "x"=1, `*`=2.
        let al = cell_inline_attrs(&base, 20.0, "*x*");
        assert!(
            matches!(al.get_span(1).style, glyphon::Style::Italic),
            "the cell content is italic"
        );
        assert!(
            matches!(al.get_span(0).color_opt, Some(c) if c.a() == 0),
            "the `*` marker is concealed (transparent) — no literal asterisk"
        );
        crate::markdown::set_wysiwyg_on(true);
    }

    #[test]
    fn table_cell_code_marker_conceals_and_content_is_mono() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(true);
        let base = Attrs::new();
        // "`x`": backtick=0, "x"=1, backtick=2 (inline code arrives via Event::Code).
        let al = cell_inline_attrs(&base, 20.0, "`x`");
        assert!(
            matches!(al.get_span(1).family, Family::Monospace),
            "the cell content shapes in the mono family"
        );
        assert!(
            matches!(al.get_span(0).color_opt, Some(c) if c.a() == 0),
            "the backtick delimiter is concealed (transparent) — no literal backtick"
        );
        crate::markdown::set_wysiwyg_on(true);
    }

    #[test]
    fn table_cell_plain_text_is_unchanged_from_base() {
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(true);
        let base = Attrs::new();
        // No inline markup -> `markdown::spans` is empty -> the list is `base`
        // alone: byte-identical to the pre-styling `set_text(cell, base)`.
        let al = cell_inline_attrs(&base, 20.0, "Monaspace Xenon");
        let s = al.get_span(0);
        assert_eq!(s.weight.0, 400, "plain cell keeps the normal weight");
        assert!(matches!(s.style, glyphon::Style::Normal), "plain cell is not italic");
        assert!(!matches!(s.family, Family::Monospace), "plain cell is not mono");
        assert!(s.color_opt.is_none(), "plain cell has no conceal / tint override");
        assert!(s.metrics_opt.is_none(), "plain cell has no zero-width metrics override");
        crate::markdown::set_wysiwyg_on(true);
    }

    /// WRAP-NOT-CLIP: a too-wide GFM table row wraps its long cell and RESERVES a
    /// tall document row (`compute_table_layout` → the shared `image_heights`
    /// slot), while a row that fits on one line reserves nothing. This is the
    /// mechanism that grows the row so the drawn grid never overlaps the following
    /// content — the alternative to the old hard-clip. Drives the real
    /// `compute_table_layout` seam over a headless pipeline.
    #[test]
    fn wide_table_wraps_and_reserves_a_tall_row_while_a_short_row_does_not() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::markdown::set_wysiwyg_on(true);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping wide_table_wraps...: no wgpu adapter");
            return;
        };
        // A long cell whose natural width far exceeds its column, so it MUST wrap to
        // several lines; the sibling cells + header/short rows all fit on one line.
        let long = "pale eucalyptus-green with a very long description that keeps \
                    going well past any single column width so it is forced to wrap \
                    onto several lines inside its own narrow column";
        let text = format!(
            "| World | Ground |\n|-------|--------|\n| Short | {long} |\n| Tiny | ok |\n"
        );
        let md_spans = crate::markdown::spans(&text);
        // set_view configures md_enabled + metrics + wrap width for the pipeline.
        p.set_view(&view_md(&text, 0, 0));
        let lh = p.metrics.line_height;
        let heights = p.compute_table_layout(&text, &md_spans);
        // Body row carrying the long cell (doc line 2) reserves a MULTI-line row.
        let wide = heights[2].expect("the wrapping table row reserves a tall row");
        assert!(
            wide > lh * 1.5,
            "the wrapped row grows to several line-heights (got {wide}, lh {lh})"
        );
        // The long cell wraps to MORE lines than any short cell, so its row is the
        // tallest reserved row (a proportionally-squeezed header column may itself
        // wrap a little — that is correct wrap-not-clip too — but never as tall).
        for (li, h) in heights.iter().enumerate() {
            if li != 2 {
                if let Some(other) = h {
                    assert!(wide > *other, "the long row (got {wide}) is tallest (line {li}: {other})");
                }
            }
        }
        // The separator (doc line 1) is never a grid row → never a reservation.
        assert!(heights[1].is_none(), "the separator row is not a grid row");

        // CONTROL — a table whose columns all fit reserves NOTHING (byte-identical
        // single-line rows, exactly the pre-round layout).
        let fits = "| a | b |\n|---|---|\n| c | d |\n";
        let fits_spans = crate::markdown::spans(fits);
        p.set_view(&view_md(fits, 0, 0));
        let fh = p.compute_table_layout(fits, &fits_spans);
        assert!(
            fh.iter().all(|h| h.is_none()),
            "a table that fits reserves no tall row (got {fh:?})"
        );
        crate::markdown::set_wysiwyg_on(true);
    }

    /// PER-WORLD CODE MONO: a CODE buffer (`syn_lang == Some`) shapes in the world's
    /// monospace companion (`Theme::mono`) even on a SERIF world, so its columns have
    /// a uniform fixed pitch — while a PROSE buffer in the SAME world keeps the
    /// proportional display face (i and m differ). Gumtree is a Literata (serif)
    /// world whose `mono` is Monaspace Xenon, so it exercises the mono/prose split.
    #[test]
    fn code_buffer_shapes_in_world_mono_while_prose_stays_display() {
        // Pitch reads fold the theme font AND the page wrap globals; hold both
        // (theme → page order, page.rs:95-99).
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping code_buffer_shapes_in_world_mono...: no wgpu adapter");
            return;
        };
        let pitch = |xs: &[f32]| -> f32 { xs[1] - xs[0] };
        let uniform = |xs: &[f32]| -> bool {
            let p0 = xs[1] - xs[0];
            xs.windows(2).all(|w| (w[1] - w[0] - p0).abs() < 0.5)
        };

        // SERIF world whose code face is a mono (Gumtree: Literata display / Monaspace
        // Xenon mono).
        theme::set_active_by_name("Gumtree").unwrap();
        p.sync_theme();
        assert_eq!(theme::active().font, "Literata");
        assert_eq!(theme::active().mono, "Monaspace Xenon");

        // A CODE buffer: mark it as Rust so the mono face is selected.
        let mut code = view("iiiiiiiiii", 0, 0);
        code.syn_lang = Some(crate::syntax::Lang::Rust);
        p.set_view(&code);
        let xs_i = p.line_glyph_xs(0);
        let mut code_m = view("mmmmmmmmmm", 0, 0);
        code_m.syn_lang = Some(crate::syntax::Lang::Rust);
        p.set_view(&code_m);
        let xs_m = p.line_glyph_xs(0);
        let (pi, pm) = (pitch(&xs_i), pitch(&xs_m));
        assert!(
            uniform(&xs_i) && uniform(&xs_m),
            "a code buffer must shape monospace (uniform pitch) even on a serif world (i={pi}, m={pm})"
        );
        assert!(
            (pi - pm).abs() < 0.5,
            "code buffer must shape i and m at the SAME mono pitch (i={pi}, m={pm})"
        );

        // A PROSE buffer (no syn_lang, not markdown) in the SAME world keeps the
        // proportional serif face: i and m differ.
        p.set_view(&view("iiiiiiiiii", 0, 0));
        let pi2 = pitch(&p.line_glyph_xs(0));
        p.set_view(&view("mmmmmmmmmm", 0, 0));
        let pm2 = pitch(&p.line_glyph_xs(0));
        assert!(
            (pi2 - pm2).abs() > 1.0,
            "prose in a serif world must stay proportional (i={pi2}, m={pm2})"
        );

        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// THE PURE FONT-FEATURE OWNER (`text::font_features`) returns the right
    /// three-way ligature split per (is_code, face, code_ligatures) — a pure fn,
    /// no GPU / locks. Covers: prose → standard on + discretionary off (NOT gated
    /// by the toggle); a pitch-safe code mono (JBM/Iosevka) → programming
    /// ligatures via `calt`; an unsafe/inert mono (Monaspace/IBM Plex) → the
    /// ligature-free set (`calt`+`rclt`+`ccmp` off); the toggle OFF → the
    /// ligature-free set for a safe mono too; and DISCRETIONARY off in EVERY case.
    #[test]
    fn font_features_owner_is_the_three_way_ligature_split() {
        use glyphon::cosmic_text::{FeatureTag, FontFeatures};
        let ff = |is_code: bool, face: &str, code_ligs: bool| -> FontFeatures {
            super::text::font_features(is_code, face, code_ligs)
        };
        // Last-set value of a tag (None = untouched → the font default applies).
        let val = |f: &FontFeatures, tag: FeatureTag| -> Option<u32> {
            f.features.iter().rev().find(|x| x.tag == tag).map(|x| x.value)
        };
        let liga = FeatureTag::STANDARD_LIGATURES;
        let clig = FeatureTag::CONTEXTUAL_LIGATURES;
        let calt = FeatureTag::CONTEXTUAL_ALTERNATES;
        let dlig = FeatureTag::DISCRETIONARY_LIGATURES;
        let rclt = FeatureTag::new(b"rclt");
        let ccmp = FeatureTag::new(b"ccmp");

        // PROSE (proportional display face): standard + contextual ON, discretionary
        // OFF — and NOT gated by the code_ligatures toggle (both toggle states equal).
        // `calt` is explicitly OFF too, on EVERY face (incl. a mono display face) —
        // the prose-ligature-leak fix: `calt` is Monaspace's programming-ligature
        // engine, and prose must never inherit a font's own default `calt` state.
        for code_ligs in [true, false] {
            for face in ["Literata", "Monaspace Xenon", "JetBrains Mono"] {
                let f = ff(false, face, code_ligs);
                assert_eq!(val(&f, liga), Some(1), "{face}: prose standard ligatures ON");
                assert_eq!(val(&f, clig), Some(1), "{face}: prose contextual ligatures ON");
                assert_eq!(val(&f, dlig), Some(0), "{face}: prose discretionary OFF");
                assert_eq!(val(&f, calt), Some(0), "{face}: prose calt OFF (no ligature leak)");
            }
        }

        // CODE on a PITCH-SAFE mono, toggle ON: programming ligatures via calt;
        // standard/contextual OFF; discretionary OFF.
        for face in ["JetBrains Mono", "Iosevka"] {
            let f = ff(true, face, true);
            assert_eq!(val(&f, calt), Some(1), "{face}: programming ligatures via calt ON");
            assert_eq!(val(&f, liga), Some(0), "{face}: standard OFF");
            assert_eq!(val(&f, clig), Some(0), "{face}: contextual-lig OFF");
            assert_eq!(val(&f, dlig), Some(0), "{face}: discretionary OFF");
        }

        // CODE on the SAME safe mono, toggle OFF: the ligature-free code set — calt
        // OFF (no programming ligatures), rclt+ccmp OFF. This is the "back to the
        // current no-ligature code behaviour" branch.
        let f = ff(true, "JetBrains Mono", false);
        assert_eq!(val(&f, calt), Some(0), "toggle off: calt OFF (no code ligatures)");
        assert_eq!(val(&f, rclt), Some(0), "toggle off: rclt OFF");
        assert_eq!(val(&f, ccmp), Some(0), "toggle off: ccmp OFF");
        assert_eq!(val(&f, dlig), Some(0), "toggle off: discretionary OFF");

        // CODE on an UNSAFE mono (Monaspace), toggle ON: STILL ligature-free — its
        // rclt+ccmp texture-healing must be disabled to keep uniform pitch (no safe
        // ligature option). Same for the INERT IBM Plex Mono.
        for face in ["Monaspace Xenon", "IBM Plex Mono"] {
            let f = ff(true, face, true);
            assert_eq!(val(&f, calt), Some(0), "{face}: calt OFF (unsafe/inert)");
            assert_eq!(val(&f, rclt), Some(0), "{face}: rclt OFF (stop cluster merge)");
            assert_eq!(val(&f, ccmp), Some(0), "{face}: ccmp OFF (stop cluster merge)");
            assert_eq!(val(&f, dlig), Some(0), "{face}: discretionary OFF");
        }

        // PROSE LIGATURE LEAK regression: no face should NOT restore calt — even
        // an unclassified/unknown display face stays explicitly OFF in prose,
        // since the prose branch returns before `mono_is_pitch_safe` is ever
        // consulted (calt has no legitimate prose role, safe mono or not).
        let f = ff(false, "Some Future Mono", true);
        assert_eq!(val(&f, calt), Some(0), "prose on an unknown face: calt still OFF");

        // An UNKNOWN mono defaults to the conservative ligature-free set.
        let f = ff(true, "Some Future Mono", true);
        assert_eq!(val(&f, calt), Some(0), "unknown mono: conservative ligature-free");
        assert_eq!(val(&f, rclt), Some(0), "unknown mono: rclt OFF");

        // The per-mono safety classifier itself: only the measured-safe monos.
        assert!(super::text::mono_is_pitch_safe("JetBrains Mono"));
        assert!(super::text::mono_is_pitch_safe("Iosevka"));
        assert!(!super::text::mono_is_pitch_safe("Monaspace Xenon"));
        assert!(!super::text::mono_is_pitch_safe("IBM Plex Mono"));
        assert!(!super::text::mono_is_pitch_safe("Some Future Mono"));
    }

    /// THE REPORTED PROSE LIGATURE LEAK, shaped for REAL (not the pure
    /// `font_features` unit above): a markdown line `==x!!==` on Mangrove
    /// (JetBrains Mono — a PITCH-SAFE mono display world, per
    /// `mono_is_pitch_safe`, whose programming ligatures ride `calt` while
    /// keeping exactly 1 glyph per source char — the exact mechanism the
    /// leak rides, and empirically confirmed live on this bundled face: with
    /// `calt` forced back on, the trailing `!` right before the highlight's
    /// closing `==` picks up a DIFFERENT contextual glyph purely because it
    /// sits next to `=`, even though `!!` is unrelated prose content, never a
    /// code construct — the reported `==foo!!==` → `==foo≠=`-reading fusion).
    /// Before this round's fix, the prose branch of `font_features` never
    /// touched `calt` at all, so it inherited the font's own (on) default.
    #[test]
    fn prose_calt_off_keeps_highlight_delimiters_as_separate_glyphs() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!(
                "skipping prose_calt_off_keeps_highlight_delimiters_as_separate_glyphs: no wgpu adapter"
            );
            return;
        };
        theme::set_active_by_name("Mangrove").unwrap();
        assert_eq!(theme::active().font, "JetBrains Mono");
        p.sync_theme();
        let line_text = "==x!!==";

        // THE REAL PRODUCTION PATH: a markdown (prose) buffer's own doc_attrs
        // — `calt` OFF, this round's fix.
        p.set_view(&view_md(line_text, 0, 0));
        let glyph_at = |p: &TextPipeline, byte: usize| -> u16 {
            p.buffer
                .layout_runs()
                .find(|r| r.line_i == 0)
                .and_then(|r| r.glyphs.iter().find(|g| g.start == byte))
                .map(|g| g.glyph_id)
                .expect("a glyph must start at this byte")
        };
        // Byte 4 is the SECOND `!` of `!!` — the char immediately before the
        // trailing `==` delimiter, i.e. the exact `!`+`=` adjacency reported.
        let prose_bang = glyph_at(&p, 4);

        // THE COUNTERFACTUAL, shaped directly (not through `font_features`):
        // the SAME text + face with `calt` forcibly RE-ENABLED, proving the
        // mechanism is real on this bundled font, independent of this test's
        // own assertions about the fix.
        let mut ff_calt_on = glyphon::cosmic_text::FontFeatures::new();
        ff_calt_on.disable(glyphon::cosmic_text::FeatureTag::DISCRETIONARY_LIGATURES);
        ff_calt_on.enable(glyphon::cosmic_text::FeatureTag::STANDARD_LIGATURES);
        ff_calt_on.enable(glyphon::cosmic_text::FeatureTag::CONTEXTUAL_LIGATURES);
        ff_calt_on.enable(glyphon::cosmic_text::FeatureTag::CONTEXTUAL_ALTERNATES);
        let attrs = Attrs::new()
            .family(Family::Name("JetBrains Mono"))
            .weight(mono_safe_weight("JetBrains Mono"))
            .font_features(ff_calt_on);
        p.buffer.set_text(&mut p.font_system, line_text, &attrs, Shaping::Advanced, None);
        let calt_on_bang = glyph_at(&p, 4);

        assert_ne!(
            prose_bang, calt_on_bang,
            "sanity: JetBrains Mono's `calt` must actually change this glyph, or \
             this test can't discriminate the fix (both gid={prose_bang})"
        );

        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// The per-mono probe's exact ligature-dense content — arrows, comparisons,
    /// path-sep, pipe. All ASCII, so on a TRUE mono every cell is one pitch; a
    /// cluster merge (a ligature spanning >1 source char as one glyph) makes the
    /// per-char `line_glyph_xs` non-uniform on these operators.
    const LIG_CONTENT: &str = "-> => != >= <= == :: |>";

    /// Uniform-pitch predicate over a `line_glyph_xs`: every consecutive delta
    /// equals the first (within 0.5px).
    fn xs_uniform(xs: &[f32]) -> bool {
        assert!(xs.len() >= 3, "need a few glyphs to measure pitch");
        let p0 = xs[1] - xs[0];
        xs.windows(2).all(|w| (w[1] - w[0] - p0).abs() < 0.5)
    }

    /// THE CODE-LIGATURE PITCH GUARD (the critical regression the three-way split
    /// must not break, and the exact gap the probe flagged): with the code-ligature
    /// features applied, every FONT-FEATURE-CONTROLLABLE mono (JetBrains Mono,
    /// Iosevka, IBM Plex Mono) STILL shapes real programming-ligature content
    /// (`-> => != >= <= == :: |>`) at STRICT uniform pitch — the per-char
    /// `line_glyph_xs` stay evenly spaced, so caret/hit-test/selection column math
    /// is honest. `font_features` keeps this uniform (calt for JBM/Iosevka's
    /// GSUB programming ligatures, 1 glyph per source char; ligature-free for the
    /// inert IBM Plex Mono).
    ///
    /// Monaspace Xenon is EXCLUDED here and covered by the characterization test
    /// below: its ligatures are AAT/`morx`-driven and CANNOT be suppressed via
    /// OpenType feature tags in this shaper (cosmic-text 0.18.2 / harfrust 0.5.2 —
    /// `rclt` isn't even in harfrust's AAT feature-mapping table), so it remains
    /// non-uniform on operator sequences. The pre-existing `mono_world_shapes_
    /// uniform_pitch` only shaped `iiii`/`mmmm` (no ligature triggers), so it
    /// MISSED this entirely — this pair of tests pins both the fixed monos and the
    /// known-unfixed one.
    #[test]
    fn code_ligature_content_stays_uniform_pitch_on_feature_controllable_monos() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!(
                "skipping code_ligature_content_stays_uniform_pitch_on_feature_controllable_monos: no wgpu adapter"
            );
            return;
        };
        let mut covered = std::collections::HashSet::new();
        for t in theme::THEMES.iter() {
            // Monaspace's AAT ligatures resist OT-feature suppression (see below).
            if t.mono == "Monaspace Xenon" {
                continue;
            }
            theme::set_active_by_name(t.name).unwrap();
            p.sync_theme();
            let mut code = view(LIG_CONTENT, 0, 0);
            code.syn_lang = Some(crate::syntax::Lang::Rust);
            p.set_view(&code);
            let xs = p.line_glyph_xs(0);
            assert!(
                xs_uniform(&xs),
                "{} (mono {}): code ligatures must keep uniform pitch on `{}` — xs={:?}",
                t.name,
                t.mono,
                LIG_CONTENT,
                xs
            );
            covered.insert(t.mono);
        }
        // Sanity: the three controllable monos were actually exercised (a mis-rename
        // of a mono face would otherwise silently shrink this guard to nothing).
        for m in ["JetBrains Mono", "Iosevka", "IBM Plex Mono"] {
            assert!(covered.contains(m), "expected a world with mono {m} to be tested");
        }
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// THE MONASPACE CLUSTER-FIX REGRESSION GUARD (flipped from the old
    /// characterization test — see its history below). Monaspace Xenon's
    /// programming ligatures are AAT/`morx`-driven "texture-healing": `-> => !=
    /// :: …` shape to one glyph PER source char but all carry the SAME cluster
    /// span, and they CANNOT be suppressed via OpenType feature tags in this
    /// shaper (cosmic-text 0.18.2 / harfrust 0.5.2 — `rclt` isn't even in
    /// harfrust's AAT feature table). The font-feature path could never make
    /// these uniform; the DEEPER fix did — `assemble_glyph_xs` now groups the
    /// glyphs sharing a span and spreads the source chars EVENLY over the
    /// group's combined advance, so the per-char `line_glyph_xs` are uniform
    /// again and the caret / selection / hit-test column math on a Monaspace
    /// code line is honest. Shapes BOTH the mixed letters-and-operators content
    /// the round named AND the pure-operator `LIG_CONTENT` the guard above uses,
    /// asserting strict uniform pitch (maxdev < 0.5px) on each.
    ///
    /// (History: this test used to assert the OPPOSITE — that Monaspace stayed
    /// non-uniform, a documented AAT limitation — with a note that its assertion
    /// should flip the day the `assemble_glyph_xs` cluster fix landed. It has.)
    #[test]
    fn monaspace_ligatures_shape_uniform_pitch_after_the_cluster_fix() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!(
                "skipping monaspace_ligatures_shape_uniform_pitch_after_the_cluster_fix: no wgpu adapter"
            );
            return;
        };
        // Potoroo's mono is Monaspace Xenon (asserted, so a reassignment surfaces here).
        theme::set_active_by_name("Potoroo").unwrap();
        assert_eq!(theme::active().mono, "Monaspace Xenon");
        p.sync_theme();
        // Mixed letters + texture-healed operators (the round's named fixture) AND
        // the pure-operator sequence — both must land on a strict uniform grid.
        for content in ["a => b != c :: d", LIG_CONTENT] {
            let mut code = view(content, 0, 0);
            code.syn_lang = Some(crate::syntax::Lang::Rust);
            p.set_view(&code);
            let xs = p.line_glyph_xs(0);
            assert!(
                xs_uniform(&xs),
                "Monaspace texture-healed ligatures must now shape UNIFORM pitch on \
                 `{content}` (the assemble_glyph_xs cluster fix) — xs={xs:?}"
            );
        }
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// CARET / SELECTION / HIT-TEST INSIDE A PROGRAMMING-LIGATURE CLUSTER (the
    /// subtle-bug zone the ligature-policy round had to clear): with code
    /// ligatures ON, a pitch-safe mono (JetBrains Mono / Iosevka) substitutes the
    /// `=>` / `!=` glyph SHAPES via `calt` while keeping 1 glyph per source char,
    /// so the per-char column model stays exact even though the on-screen glyph
    /// reads as one arrow. This drives the REAL pipeline (not the pure seam) and
    /// asserts all three consumers of `line_glyph_xs` agree per-char:
    ///   * CARET: the caret x BETWEEN the two ligature chars (col of `>`) sits one
    ///     full pitch past the `=` — i.e. `col_x_and_advance` gives an exact
    ///     per-char boundary, never the whole-cluster width.
    ///   * SELECTION: a per-column advance across the cluster equals one pitch each
    ///     (a selection of just `=` covers exactly one cell, not the whole `=>`).
    ///   * HIT-TEST: a click in the first quarter of a char's cell resolves to that
    ///     char, the last quarter to the next — round-tripping every column,
    ///     including the two chars fused into the arrow glyph.
    #[test]
    fn caret_and_hit_test_are_per_char_inside_a_programming_ligature_cluster() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let saved_lig = crate::render::code_ligatures_on();
        crate::render::set_code_ligatures_on(true);
        let Some(mut p) = headless_pipeline() else {
            eprintln!(
                "skipping caret_and_hit_test_are_per_char_inside_a_programming_ligature_cluster: no wgpu adapter"
            );
            crate::render::set_code_ligatures_on(saved_lig);
            return;
        };
        // `a => b != c` — the ligature clusters `=>` (cols 2-3) and `!=` (cols 7-8)
        // sit mid-line with plain chars on either side, so a per-char boundary is
        // measurable against its neighbours.
        let content = "a => b != c";
        // The world PAIRS: Currawong=Iosevka, Mangrove=JetBrains Mono.
        for world in ["Currawong", "Mangrove"] {
            theme::set_active_by_name(world).unwrap();
            assert!(
                super::text::mono_is_pitch_safe(theme::active().mono),
                "{world}: expected a pitch-safe programming-ligature mono (mono={})",
                theme::active().mono
            );
            p.sync_theme();
            let mut code = view(content, 0, 0);
            code.syn_lang = Some(crate::syntax::Lang::Rust);
            p.set_view(&code);

            let xs = p.line_glyph_xs(0);
            let n = content.chars().count();
            assert_eq!(xs.len(), n + 1, "{world}: one x boundary per char + end");
            let pitch = xs[1] - xs[0];

            // CARET: every column boundary is an EXACT per-char multiple of the
            // pitch — the `>` of `=>` lands one pitch past the `=`, never fused.
            for c in 0..=n {
                let (x, _adv) = p.col_x_and_advance(0, c);
                let expect = c as f32 * pitch;
                assert!(
                    (x - expect).abs() < 0.5,
                    "{world}: caret x at col {c} must be per-char ({expect}), got {x} (xs={xs:?})"
                );
            }
            // SELECTION: the advance of each interior column is one pitch — a
            // one-char selection over the `=` (col 2) or `!` (col 7) is one cell.
            for c in [2usize, 3, 7, 8] {
                let (_x, adv) = p.col_x_and_advance(0, c);
                assert!(
                    (adv - pitch).abs() < 0.5,
                    "{world}: col {c} advance must be one pitch ({pitch}), got {adv}"
                );
            }
            // HIT-TEST: a click in the first quarter of each char's cell resolves
            // to that char; the last quarter to the next gap — round-trips every
            // column, including the two chars fused into an arrow glyph.
            let text_left = p.text_left();
            let py = p.doc_top() + p.metrics.line_height * 0.5;
            for c in 0..n {
                let cell = xs[c + 1] - xs[c];
                let (_l, col_lo) = p.hit_test(text_left + xs[c] + cell * 0.25, py, 0);
                assert_eq!(col_lo, c, "{world}: click in the near quarter of col {c} → col {c}");
                let (_l, col_hi) = p.hit_test(text_left + xs[c] + cell * 0.75, py, 0);
                assert_eq!(col_hi, c + 1, "{world}: click in the far quarter of col {c} → col {}", c + 1);
            }
        }
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
        crate::render::set_code_ligatures_on(saved_lig);
    }

    /// MEASURED redmean RGB distance (u8 scale) — the perceptual-ish weighting the
    /// role-style law thresholds were calibrated against.
    fn redmean(a: theme::Srgb, b: theme::Srgb) -> f32 {
        let rbar = (a.r as f32 + b.r as f32) * 0.5;
        let dr = a.r as f32 - b.r as f32;
        let dg = a.g as f32 - b.g as f32;
        let db = a.b as f32 - b.b as f32;
        ((2.0 + rbar / 256.0) * dr * dr
            + 4.0 * dg * dg
            + (2.0 + (255.0 - rbar) / 256.0) * db * db)
            .sqrt()
    }

    /// A translucent wash quad composited over an opaque ground — what the eye
    /// actually sees behind a washed span (straight alpha, u8 rounding).
    fn composite(wash: theme::Srgb, ground: theme::Srgb) -> theme::Srgb {
        let a = wash.a as f32 / 255.0;
        let ch = |w: u8, g: u8| (g as f32 + (w as f32 - g as f32) * a).round() as u8;
        theme::Srgb::rgb(ch(wash.r, ground.r), ch(wash.g, ground.g), ch(wash.b, ground.b))
    }

    /// Circular hue distance in degrees.
    fn hue_dist(a: f32, b: f32) -> f32 {
        let d = (a - b).rem_euclid(360.0);
        d.min(360.0 - d)
    }

    /// THE ROLE-STYLE LAW TEST — sweeps EVERY world in `theme::THEMES` (a future
    /// world is enrolled automatically) × every syntax role, and asserts the laws
    /// on the EFFECTIVE style (`role_style_for`, overrides included). LOCK-FREE:
    /// `role_style_for` takes `&Theme`, never the process-global active theme.
    ///
    /// The laws (thresholds calibrated from the measured 14-world table):
    /// (a) every pair among {Definition, Constant, Str, CommentCode(=muted)} is
    ///     redmean ≥ 40 apart (measured floor 51.6, Tawny Def–Const);
    /// (b) prose-Comment fg == `base_content` EXACTLY (comments are the prose in
    ///     the code — decision 2) and CommentCode fg == `muted` EXACTLY;
    /// (c) the comment wash composited over `base_100` is a WHISPER: ΔL in
    ///     [0.03, 0.12] and redmean ≥ 35 (measured 0.063–0.11 / 51–89) — a wash
    ///     is structurally incapable of reading as the accent;
    /// (d) dark worlds wash strings too (comment-wash vs string-wash effective
    ///     redmean ≥ 20, measured 28–29); light worlds carry NO string wash;
    ///     Definition / Constant / CommentCode are NEVER washed;
    /// (e) AMBER GUARD: every derived fg tint with sat > 0.15 sits ≥ 30° of hue
    ///     from the world's `primary` AND at sat ≤ 0.50 (the comment tiers are
    ///     the existing inks — exempt by identity, never equal to primary);
    /// (f) presence ordering is monotone per mode: Definition sits closest to
    ///     the full ink, then Constant, then Str;
    /// (g) PERCEPTIBILITY FLOOR: every tinted role's fg (Definition, Constant,
    ///     Str) sits redmean ≥ 70 from `base_content` on EVERY world — a floor
    ///     picked from the measured 14-world table: dark `Definition` at its old
    ///     t=0.12/sat=0.32 measured 36.4–65.2 (Currawong screenshot-confirmed as
    ///     plain-looking ink, the bug this law exists to catch structurally);
    ///     every OTHER role/world combination already measured ≥ 76 (worst:
    ///     dark Constant at 76.2, Undertow). 70 sits safely below that 76.2 floor
    ///     (room for future re-tuning) while sitting well above the old broken
    ///     Definition range, so a future regression of this exact shape — a role
    ///     tint that clears the pairwise ≥40 law but reads as invisible against
    ///     the page's own ink — fails this test immediately instead of needing a
    ///     human screenshot to notice;
    /// (h) LUMINANCE FLOOR: every tinted role's fg sits WCAG relative-luminance
    ///     ΔY ≥ 0.05 from `base_content` on every world — redmean ALONE is proven
    ///     insufficient by (g)'s own history: light `Definition` cleared redmean
    ///     70+ (Saltpan measured 148) while carrying almost all of that distance
    ///     in the BLUE channel, which Rec.709 luminance weighs at only 0.0722 (vs
    ///     0.7152 on green) — the eye resolves LUMINANCE first (sparse S-cones),
    ///     so a color can be "far" in redmean and still read as plain ink. Floor
    ///     picked from the retuned 14-world table (`measure_role_luminance`, an
    ///     ignored scratch test): worst case (post the round-2 ground-contrast
    ///     retune below) is light Definition/Constant at ΔY 0.056 (Gumtree); 0.05
    ///     sits with margin below every measured value and comfortably above the
    ///     old broken range (ΔY 0.027–0.042) — so a future regression of this
    ///     exact shape (redmean-passing, luminance-invisible) fails structurally
    ///     instead of needing a screenshot.
    /// (i) GROUND-CONTRAST FLOOR: every tinted role's fg clears a WCAG contrast
    ///     RATIO of ≥ 4.5:1 against `base_100` (the page's own background) on
    ///     every world — the axis (h) does not cover. (h) only measures distance
    ///     from the INK; a fix that satisfies (h) by pushing a role's lightness
    ///     toward `muted` can, on a light world, push it most of the way toward
    ///     the pale GROUND instead — distinct-from-ink is not the same claim as
    ///     readable-on-page. This is exactly what happened: the round-1 light
    ///     retune (`T_LIGHT = [0.84, 0.90, 0.94]`) cleared (g)/(h) on every world
    ///     yet a live taste-gate verdict on Saltpan called the result "too hard
    ///     to read" — strings/constants/definitions as washed-out pastels on the
    ///     pale ground. Measured: Saltpan `Str` at the round-1 rungs contrasted
    ///     only 4.62:1 against `base_100` (Quokka worse, 3.66:1) — well under
    ///     body-text-grade legibility (WCAG AA normal text = 4.5:1) despite
    ///     clearing every prior law. 4.5:1 is the standard body-text floor (not
    ///     loosened for glyph-scale mono/serif — the user's own complaint was
    ///     about reading code prose, i.e. body text). Dark worlds were ALREADY
    ///     clearing this floor by a wide margin (measured 9.4–13.5:1 — a dark
    ///     ground is far from every usable tint) and are asserted here
    ///     unchanged, never retuned. Round 2's retune (`T_LIGHT = [0.76, 0.78,
    ///     0.80]`, `S_FG_LIGHT = 0.18`, found by `sweep_light_ladder` now
    ///     searching for BOTH floors (h) and (i) simultaneously) measures
    ///     worst-case ground contrast 4.84:1 (Quokka `Str`) while keeping (h)'s
    ///     worst-case ΔY at 0.056 — both floors clear with margin on every
    ///     light world.
    #[test]
    fn role_style_laws_hold_for_every_world() {
        use crate::syntax::SynKind;
        // The explicit role roster, backed by a NO-WILDCARD match: a future
        // SynKind variant fails to compile here until it is enrolled in the sweep.
        const ROLES: [SynKind; 5] = [
            SynKind::Comment,
            SynKind::CommentCode,
            SynKind::Str,
            SynKind::Constant,
            SynKind::Definition,
        ];
        fn enrolled(k: SynKind) -> usize {
            match k {
                SynKind::Comment => 0,
                SynKind::CommentCode => 1,
                SynKind::Str => 2,
                SynKind::Constant => 3,
                SynKind::Definition => 4,
            }
        }
        for (i, k) in ROLES.iter().enumerate() {
            assert_eq!(enrolled(*k), i, "ROLES roster out of sync with SynKind");
        }

        for th in theme::THEMES.iter() {
            let style = |k: SynKind| role_style_for(th, k);

            // (b) The two comment tiers ARE the existing inks, exactly.
            assert_eq!(style(SynKind::Comment).fg, th.base_content,
                "{}: prose comments render at FULL content ink", th.name);
            assert_eq!(style(SynKind::CommentCode).fg, th.muted,
                "{}: commented-out code stays the muted grey", th.name);

            // (a) Pairwise distinguishability of the four ink-distinct roles.
            let four = [SynKind::Definition, SynKind::Constant, SynKind::Str, SynKind::CommentCode];
            for i in 0..four.len() {
                for j in i + 1..four.len() {
                    let d = redmean(style(four[i]).fg, style(four[j]).fg);
                    assert!(
                        d >= 40.0,
                        "{}: {:?} vs {:?} fg redmean {d:.1} < 40 (memory test fails)",
                        th.name, four[i], four[j]
                    );
                }
            }

            // (c) The comment wash: present on every world, a value whisper.
            let cw = style(SynKind::Comment).wash
                .unwrap_or_else(|| panic!("{}: every world carries the comment wash", th.name));
            let ceff = composite(cw, th.base_100);
            let dl = (ceff.to_hsl().2 - th.base_100.to_hsl().2).abs();
            assert!(
                (0.03..=0.12).contains(&dl),
                "{}: comment-wash ΔL {dl:.3} outside the whisper band [0.03, 0.12]",
                th.name
            );
            assert!(
                redmean(ceff, th.base_100) >= 35.0,
                "{}: comment wash too faint (redmean {:.1} < 35)",
                th.name, redmean(ceff, th.base_100)
            );

            // (d) Strings: washed on dark worlds (distinct from the comment wash),
            // fg-tint-only on light; Definition/Constant/CommentCode never washed.
            if th.dark {
                let sw = style(SynKind::Str).wash
                    .unwrap_or_else(|| panic!("{}: dark worlds wash strings", th.name));
                let seff = composite(sw, th.base_100);
                let sdl = (seff.to_hsl().2 - th.base_100.to_hsl().2).abs();
                assert!(
                    (0.03..=0.12).contains(&sdl),
                    "{}: string-wash ΔL {sdl:.3} outside [0.03, 0.12]", th.name
                );
                assert!(
                    redmean(ceff, seff) >= 20.0,
                    "{}: comment vs string wash effective redmean {:.1} < 20",
                    th.name, redmean(ceff, seff)
                );
            } else {
                assert!(style(SynKind::Str).wash.is_none(),
                    "{}: light worlds carry NO string wash", th.name);
            }
            assert!(style(SynKind::Definition).wash.is_none()
                && style(SynKind::Constant).wash.is_none()
                && style(SynKind::CommentCode).wash.is_none(),
                "{}: only prose comments (+ dark strings) are washed", th.name);

            // (e) AMBER GUARD over every enrolled role's effective fg.
            let (ph, _, _) = th.primary.to_hsl();
            for k in ROLES {
                let fg = style(k).fg;
                assert_ne!(fg, th.primary, "{}: {k:?} must never BE the accent", th.name);
                if fg == th.base_content || fg == th.muted {
                    continue; // the comment tiers ride the existing inks (exempt by identity)
                }
                let (h, s, _) = fg.to_hsl();
                assert!(s <= 0.5, "{}: {k:?} fg sat {s:.2} > 0.50 (too loud)", th.name);
                if s > 0.15 {
                    let d = hue_dist(h, ph);
                    assert!(
                        d >= 30.0,
                        "{}: {k:?} fg hue {h:.0}° only {d:.0}° from primary {ph:.0}°",
                        th.name
                    );
                }
            }

            // (f) Presence ordering: Definition closest to full ink, then Constant,
            // then Str — monotone in BOTH modes (lightness distance from base_content).
            let lf = th.base_content.to_hsl().2;
            let dist_l = |k: SynKind| (style(k).fg.to_hsl().2 - lf).abs();
            assert!(
                dist_l(SynKind::Definition) < dist_l(SynKind::Constant),
                "{}: Definition must be more present than Constant", th.name
            );
            assert!(
                dist_l(SynKind::Constant) < dist_l(SynKind::Str),
                "{}: Constant must be more present than Str", th.name
            );

            // (g) PERCEPTIBILITY FLOOR — every tinted role's fg must read as
            // clearly distinct from the page's own ink, not just from its
            // sibling roles (the bug this law exists to catch: Definition
            // cleared the pairwise ≥40 floor at redmean ~43 vs base_content on
            // Currawong yet read as plain white in a live screenshot).
            const PERCEPTIBILITY_FLOOR: f32 = 70.0;
            for k in [SynKind::Definition, SynKind::Constant, SynKind::Str] {
                let d = redmean(style(k).fg, th.base_content);
                assert!(
                    d >= PERCEPTIBILITY_FLOOR,
                    "{}: {k:?} fg redmean {d:.1} vs base_content < floor {PERCEPTIBILITY_FLOOR} (imperceptible tint)",
                    th.name
                );
            }

            // (h) LUMINANCE FLOOR — redmean alone passed the exact bug this law
            // exists to catch (light Definition, almost all its redmean distance
            // sitting in the low-luminance-weight blue channel). Every tinted
            // role's fg must clear a WCAG relative-luminance ΔY from `base_content`.
            const LUMINANCE_FLOOR: f32 = 0.05;
            let y0 = rel_luminance(th.base_content);
            for k in [SynKind::Definition, SynKind::Constant, SynKind::Str] {
                let dy = (rel_luminance(style(k).fg) - y0).abs();
                assert!(
                    dy >= LUMINANCE_FLOOR,
                    "{}: {k:?} fg relative-luminance ΔY {dy:.3} vs base_content < floor {LUMINANCE_FLOOR} (redmean-passing, luminance-invisible)",
                    th.name
                );
            }

            // (i) GROUND-CONTRAST FLOOR — (h) alone passed the exact bug this law
            // exists to catch (a light-world fix that satisfies "distinct from
            // ink" by pushing lightness toward `muted`, which is itself already
            // most of the way toward the pale `base_100` ground — camouflage
            // against the PAGE, not the ink). Every tinted role's fg must clear
            // a WCAG contrast RATIO against `base_100` — body-text grade.
            const GROUND_CONTRAST_FLOOR: f32 = 4.5;
            for k in [SynKind::Definition, SynKind::Constant, SynKind::Str] {
                let cr = contrast_ratio(style(k).fg, th.base_100);
                assert!(
                    cr >= GROUND_CONTRAST_FLOOR,
                    "{}: {k:?} fg contrast-vs-ground {cr:.2}:1 < floor {GROUND_CONTRAST_FLOOR}:1 (luminance-distinct-from-ink but camouflaged against the page)",
                    th.name
                );
            }
        }
    }

    /// THE HIGHLIGHT-WASH LAW TEST — sweeps EVERY world and asserts the dedicated
    /// markdown `==highlight==` wash ([`highlight_wash`]) obeys its own contract,
    /// distinct from the comment wash's whisper contract above. The `==highlight==`
    /// band was DECOUPLED from the warm comment wash (a deliberate, narrow break of
    /// the one-warm-wash owner — a highlighter and a comment wash are different
    /// intents): the old shared cream read MUDDY on the cool pale light grounds
    /// (Gumtree pale-green, Bilby pale-cyan, Saltpan ecru), a faint warm-over-cool
    /// blend with almost no hue contrast, so a highlighter that should POP nearly
    /// vanished. THIS ROUND made the hue PER-WORLD — derived from each world's own
    /// accent (`hue(primary) + 165°`, a split-complementary), superseding the fixed
    /// foreign violet (which read as un-native, the same on every world). The laws,
    /// all on the EFFECTIVE `highlight_wash` (lock-free — it takes `&Theme`, never
    /// the process-global active theme):
    /// - (a) DISTINCT FROM THE COMMENT WASH: the highlight quad rgba is never equal
    ///   to the world's comment wash — the whole point of the decouple.
    /// - (b) AMBER GUARD (DESIGN §3): the per-world hue sits ≥ 30° off that world's
    ///   `primary` (the 165° split-complement rotation makes it exactly 165° on
    ///   every world — the caret's amber stays its own).
    /// - (c) IT POPS: composited over `base_100` it clears a redmean floor (70) far
    ///   above the comment wash's own 35 floor, AND out-pops the comment wash on
    ///   EVERY world (highlight composited redmean > comment composited redmean) —
    ///   the direct proof it reads louder than the whisper it replaced.
    /// - (d) STILL CALM: the composited VALUE step (ΔL vs `base_100`) stays under a
    ///   ceiling (0.20) — a wash, not a neon slab. (No ΔL FLOOR: on a cool ground
    ///   the pop is entirely HUE-driven, so its value step is deliberately modest —
    ///   redmean, not ΔL, is the pop axis for a hue-shift highlight.)
    /// - (e) PER-WORLD HUE (the point of this round): the highlight hue VARIES
    ///   across the worlds — at least 8 distinct hues among the 14 (proof it is no
    ///   longer a single fixed value) — while each stays ≥ 15° off its OWN world's
    ///   ground hue (`base_100`), so no world's highlight muddies against its page.
    #[test]
    fn highlight_wash_laws_hold_for_every_world() {
        // Pop floor — a highlight composited over the page must clear this, far
        // above the comment wash's own faint-floor of 35 (law (c) above).
        const HIGHLIGHT_POP_FLOOR: f32 = 70.0;
        // Calm ceiling — the composited value step stays a wash, not a slab.
        const HIGHLIGHT_CALM_DL_CEIL: f32 = 0.20;
        // Ground-separation floor — the per-world hue never lands on its own page's
        // hue (measured worst 20.8°, Bilby); anything under this reads muddy.
        const HIGHLIGHT_GROUND_HUE_FLOOR: f32 = 15.0;
        // Per-world variation floor — proof the hue is derived, not fixed.
        const HIGHLIGHT_MIN_DISTINCT_HUES: usize = 8;
        let mut distinct_hues = std::collections::HashSet::new();
        for th in theme::THEMES.iter() {
            let hw = highlight_wash(th);
            assert!(hw.a > 0, "{}: the highlight wash is always present", th.name);

            // (a) distinct from the comment wash — the decouple.
            let cw = role_style_for(th, crate::syntax::SynKind::Comment)
                .wash
                .unwrap_or_else(|| panic!("{}: every world carries the comment wash", th.name));
            assert_ne!(
                hw.rgba_bytes(), cw.rgba_bytes(),
                "{}: the highlight wash must be DECOUPLED from (never equal to) the comment wash",
                th.name
            );

            // (b) amber guard: the per-world hue sits ≥ 30° off primary.
            let (hh, hs, _) = hw.to_hsl();
            let (ph, _, _) = th.primary.to_hsl();
            assert!(hs > 0.15, "{}: highlight wash should carry real chroma", th.name);
            let d = hue_dist(hh, ph);
            assert!(
                d >= 30.0,
                "{}: highlight wash hue {hh:.0}° only {d:.0}° from primary {ph:.0}°",
                th.name
            );

            // (c) it POPS: composited over the page it clears the pop floor AND
            // out-pops the comment wash on this world.
            let heff = composite(hw, th.base_100);
            let ceff = composite(cw, th.base_100);
            let h_pop = redmean(heff, th.base_100);
            let c_pop = redmean(ceff, th.base_100);
            assert!(
                h_pop >= HIGHLIGHT_POP_FLOOR,
                "{}: highlight wash too faint (composited redmean {h_pop:.1} < floor {HIGHLIGHT_POP_FLOOR})",
                th.name
            );
            assert!(
                h_pop > c_pop,
                "{}: the highlight wash must out-pop the comment whisper (highlight redmean {h_pop:.1} <= comment {c_pop:.1})",
                th.name
            );

            // (d) still calm: the composited value step stays under the ceiling.
            let dl = (heff.to_hsl().2 - th.base_100.to_hsl().2).abs();
            assert!(
                dl <= HIGHLIGHT_CALM_DL_CEIL,
                "{}: highlight wash ΔL {dl:.3} over the calm ceiling {HIGHLIGHT_CALM_DL_CEIL} (reads as a slab, not a wash)",
                th.name
            );

            // (e) per-world: the hue never muddies against this world's OWN ground.
            let (gh, _, _) = th.base_100.to_hsl();
            let dg = hue_dist(hh, gh);
            assert!(
                dg >= HIGHLIGHT_GROUND_HUE_FLOOR,
                "{}: highlight wash hue {hh:.0}° only {dg:.0}° from its ground {gh:.0}° (muddy)",
                th.name
            );
            distinct_hues.insert(hh.round() as i32);
        }
        // (e) per-world variation: the hue is derived, not a single fixed value.
        assert!(
            distinct_hues.len() >= HIGHLIGHT_MIN_DISTINCT_HUES,
            "highlight hue must VARY per world: only {} distinct hues across {} worlds (< {})",
            distinct_hues.len(), theme::THEMES.len(), HIGHLIGHT_MIN_DISTINCT_HUES
        );
    }

    /// SCRATCH measurement harness (not a law): prints redmean + relative-luminance
    /// distance from `base_content` for every tinted role on every world, to
    /// calibrate the luminance floor. Run with
    /// `cargo test measure_role_luminance -- --nocapture --ignored`.
    #[test]
    #[ignore]
    fn measure_role_luminance() {
        use crate::syntax::SynKind;
        for th in theme::THEMES.iter() {
            let y0 = rel_luminance(th.base_content);
            let ym = rel_luminance(th.muted);
            eprintln!("{:10} dark={:5} MUTED dY={:.4}", th.name, th.dark, (ym - y0).abs());
            for k in [SynKind::Definition, SynKind::Constant, SynKind::Str] {
                let style = role_style_for(th, k);
                let d = redmean(style.fg, th.base_content);
                let dy = (rel_luminance(style.fg) - y0).abs();
                eprintln!(
                    "{:10} dark={:5} {:10?} redmean={:6.1} dY={:.4} fg={:?}",
                    th.name, th.dark, k, d, dy, style.fg
                );
            }
        }
    }

    /// Relative luminance per WCAG (gamma-decoded, Rec.709 weights). Alpha ignored.
    /// SCRATCH helper for `measure_role_luminance` / `sweep_light_ladder`.
    fn rel_luminance(c: theme::Srgb) -> f32 {
        let lin = |v: u8| {
            let x = v as f32 / 255.0;
            if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    /// WCAG contrast RATIO between two colors ((L1+0.05)/(L2+0.05), L1 the
    /// lighter). SCRATCH helper for `measure_ground_contrast` / `sweep_light_ladder`.
    fn contrast_ratio(a: theme::Srgb, b: theme::Srgb) -> f32 {
        let (ya, yb) = (rel_luminance(a), rel_luminance(b));
        let (hi, lo) = if ya > yb { (ya, yb) } else { (yb, ya) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// SCRATCH measurement (not a law): WCAG contrast ratio of every tinted role's
    /// fg against `base_100` (the GROUND, not the ink) on every world — the axis
    /// the pre-(i) law suite never checked (ink-distance alone permits
    /// background-camouflage; see THEMES.md). Run with `cargo test
    /// measure_ground_contrast -- --nocapture --ignored`.
    #[test]
    #[ignore]
    fn measure_ground_contrast() {
        use crate::syntax::SynKind;
        for th in theme::THEMES.iter() {
            for k in [SynKind::Definition, SynKind::Constant, SynKind::Str] {
                let style = role_style_for(th, k);
                let cr = contrast_ratio(style.fg, th.base_100);
                eprintln!("{:10} dark={:5} {:10?} contrast-vs-ground={:5.2}:1", th.name, th.dark, k, cr);
            }
        }
    }

    /// SCRATCH param sweep (not a law): tries a grid of `(t_def, t_const, t_str, s)`
    /// light-ladder candidates directly against `role_style_for`'s formula (mirrored
    /// here since the constants aren't parameterized). Round 2 (the ground-contrast
    /// retune): a candidate must clear EVERY existing law (pairwise ≥40,
    /// perceptibility ≥70, ink-luminance ΔY ≥0.05) PLUS the new ground-contrast
    /// floor (≥4.5:1 vs `base_100`) simultaneously — reports the winner ranked by
    /// worst-case ground contrast (the axis round 1 never searched for; see
    /// THEMES.md and the `T_LIGHT` doc comment in `render/spans.rs`). Run with
    /// `cargo test sweep_light_ladder -- --nocapture --ignored`.
    #[test]
    #[ignore]
    fn sweep_light_ladder() {
        const HUE_DEF: f32 = 220.0;
        const HUE_CONST: f32 = 290.0;
        const HUE_STR: f32 = 140.0;
        const GROUND_FLOOR: f32 = 4.5; // WCAG body-text-grade contrast ratio vs base_100
        const LUM_FLOOR: f32 = 0.05;
        let light_worlds: Vec<_> = theme::THEMES.iter().filter(|t| !t.dark).collect();

        let mut best: Option<(f32, (f32, f32, f32, f32))> = None;
        let mut t_def = 0.20;
        while t_def <= 0.85 {
            let mut t_const = t_def + 0.01;
            while t_const <= 0.90 {
                let mut t_str = t_const + 0.01;
                while t_str <= 0.95 {
                    let mut s = 0.15;
                    while s <= 0.50 {
                        let mut ok = true;
                        let mut worst_ground = f32::INFINITY;
                        for th in &light_worlds {
                            let (_, _, l_full) = th.base_content.to_hsl();
                            let (_, _, l_dim) = th.muted.to_hsl();
                            let fg_at = |anchor: f32, ti: f32| {
                                theme::Srgb::from_hsl(anchor, s, l_full + (l_dim - l_full) * ti)
                            };
                            let def = fg_at(HUE_DEF, t_def);
                            let cst = fg_at(HUE_CONST, t_const);
                            let st = fg_at(HUE_STR, t_str);
                            let muted = th.muted;
                            let base = th.base_content;
                            let pairs = [
                                redmean(def, cst), redmean(def, st), redmean(def, muted),
                                redmean(cst, st), redmean(cst, muted), redmean(st, muted),
                            ];
                            if pairs.iter().any(|d| *d < 40.0) { ok = false; break; }
                            let floors = [redmean(def, base), redmean(cst, base), redmean(st, base)];
                            if floors.iter().any(|d| *d < 70.0) { ok = false; break; }
                            let y0 = rel_luminance(base);
                            let dys = [
                                (rel_luminance(def) - y0).abs(),
                                (rel_luminance(cst) - y0).abs(),
                                (rel_luminance(st) - y0).abs(),
                            ];
                            if dys.iter().any(|d| *d < LUM_FLOOR) { ok = false; break; }
                            let grounds = [
                                contrast_ratio(def, th.base_100),
                                contrast_ratio(cst, th.base_100),
                                contrast_ratio(st, th.base_100),
                            ];
                            for g in grounds {
                                worst_ground = worst_ground.min(g);
                            }
                        }
                        if ok && worst_ground >= GROUND_FLOOR {
                            if best.map(|(b, _)| worst_ground > b).unwrap_or(true) {
                                best = Some((worst_ground, (t_def, t_const, t_str, s)));
                            }
                        }
                        s += 0.01;
                    }
                    t_str += 0.01;
                }
                t_const += 0.01;
            }
            t_def += 0.01;
        }
        eprintln!("BEST (worst-case ground contrast, subject to every law): {:?}", best);
        eprintln!("SHIPPED (rounded, chosen for margin on BOTH the luminance and ground floors): \
            T_LIGHT=[0.76,0.78,0.80] S_FG_LIGHT=0.18 — worst ground 4.84:1 (Quokka Str), worst ink dY 0.056 (Gumtree Def/Const)");
    }

    /// THE INK-LADDER + SELECTION LAW TEST — sweeps every world in `theme::THEMES`
    /// and asserts the non-role-tint half of the audit: the ink ladder
    /// (`base_content` → `muted` → `faint`) steps monotonically toward the
    /// background and each step stays perceptibly distinct, `faint` (the dimmest
    /// UI-metadata rung — gutter line numbers, debug panel, stats HUD captions)
    /// stays legible against its own `base_100`, and `selection` is a QUIET
    /// highlight — visible but never reading as a paint bucket. Thresholds
    /// calibrated from the measured 14-world table (`measure_ink_ladder`, an
    /// ignored scratch test):
    /// (a) `base_content`→`muted` redmean ≥ 100 (worst measured 201.9, Gumtree)
    ///     and `muted`→`faint` redmean ≥ 80 (worst measured 116.7, Potoroo) —
    ///     each ladder rung reads as its own distinct step, not a copy of its
    ///     neighbor;
    /// (b) monotone LIGHTNESS: `faint` sits strictly between `muted` and
    ///     `base_100` in HSL lightness (further toward the background than
    ///     `muted`, but not AT the background) on every world — the ladder never
    ///     reverses or collapses;
    /// (c) `faint` vs `base_100` redmean ≥ 100 (worst measured 166.6, Mopoke) —
    ///     the faintest rung still reads as present ink, not invisible;
    /// (d) selection COMPOSITED over `base_100` at its authored alpha (what the eye
    ///     actually sees — NOT the opaque tint, which flattered a sub-glance
    ///     highlight) clears a CONTRAST FLOOR: composited-vs-ground redmean ≥ 35 AND
    ///     ΔL ≥ 0.10, so a selection can never read as "you can't tell it's
    ///     highlighted" (the reported Undertow/Mangrove bug: those two composited to
    ///     only ΔL 0.090 / 0.076, invisible enough to fail this law before their
    ///     tints were lifted in-hue). Still CALM: ΔL ≤ 0.35 (a quiet highlight, never
    ///     a solid paint fill — worst 0.231, Outback). Floor calibrated to fail the
    ///     two worst offenders; every world now clears ΔL 0.118 (Currawong).
    #[test]
    fn ink_ladder_and_selection_laws_hold_for_every_world() {
        for th in theme::THEMES.iter() {
            // (a) Distinct steps.
            let step1 = redmean(th.base_content, th.muted);
            assert!(step1 >= 100.0, "{}: content->muted redmean {step1:.1} < 100", th.name);
            let step2 = redmean(th.muted, th.faint);
            assert!(step2 >= 80.0, "{}: muted->faint redmean {step2:.1} < 80", th.name);

            // (b) Monotone lightness: faint strictly between muted and base_100.
            let l_muted = th.muted.to_hsl().2;
            let l_faint = th.faint.to_hsl().2;
            let l_bg = th.base_100.to_hsl().2;
            if th.dark {
                // Dark world: ink lightens toward background as it dims... no —
                // background is DARKEST, ink is light; faint recedes TOWARD the
                // dark background, so l_faint sits between l_bg and l_muted.
                assert!(
                    l_faint < l_muted && l_faint > l_bg,
                    "{}: faint lightness {l_faint:.3} not between bg {l_bg:.3} and muted {l_muted:.3}",
                    th.name
                );
            } else {
                assert!(
                    l_faint > l_muted && l_faint < l_bg,
                    "{}: faint lightness {l_faint:.3} not between muted {l_muted:.3} and bg {l_bg:.3}",
                    th.name
                );
            }

            // (c) Faint stays legible against its own background.
            let fvb = redmean(th.faint, th.base_100);
            assert!(fvb >= 100.0, "{}: faint vs base_100 redmean {fvb:.1} < 100 (too faint to read)", th.name);

            // (d) Selection COMPOSITED over the ground is a quiet, GLANCEABLE
            // highlight — measured on what the eye sees, not the opaque tint.
            let eff = composite(th.selection, th.base_100);
            let svb = redmean(eff, th.base_100);
            assert!(
                svb >= 35.0,
                "{}: selection composited vs base_100 redmean {svb:.1} < 35 (near-invisible)",
                th.name
            );
            let dl = (eff.to_hsl().2 - l_bg).abs();
            assert!(
                dl >= 0.10,
                "{}: selection composited ΔL {dl:.3} < 0.10 — sub-glance, you can't tell it's highlighted",
                th.name
            );
            assert!(
                dl <= 0.35,
                "{}: selection composited ΔL {dl:.3} > 0.35 — reads as a solid paint fill, not a calm highlight",
                th.name
            );
        }
    }

    /// SCRATCH measurement (not a law): ink-ladder step sizes (content->muted,
    /// muted->faint) and faint-vs-background legibility, plus selection-vs-
    /// background distance, for every world. Informs the audit's ladder/selection
    /// laws. Run with `cargo test measure_ink_ladder -- --nocapture --ignored`.
    #[test]
    #[ignore]
    fn measure_ink_ladder() {
        for th in theme::THEMES.iter() {
            let y = |c: theme::Srgb| rel_luminance(c);
            eprintln!(
                "{:10} dark={:5} content->muted redmean={:6.1} dY={:.3} | muted->faint redmean={:6.1} dY={:.3} | faint-vs-bg redmean={:6.1} dY={:.3} | selection-vs-bg redmean={:6.1}",
                th.name, th.dark,
                redmean(th.base_content, th.muted), (y(th.base_content) - y(th.muted)).abs(),
                redmean(th.muted, th.faint), (y(th.muted) - y(th.faint)).abs(),
                redmean(th.faint, th.base_100), (y(th.faint) - y(th.base_100)).abs(),
                redmean(theme::Srgb::rgb(th.selection.r, th.selection.g, th.selection.b), th.base_100),
            );
            let sel_eff = composite(th.selection, th.base_100);
            let dl = (sel_eff.to_hsl().2 - th.base_100.to_hsl().2).abs();
            eprintln!("{:10} selection composited ΔL={:.3}", th.name, dl);
        }
    }

    /// COMMENT PROMINENCE at the attrs seam: a code buffer's prose comment shapes
    /// at the FULL content ink (decision 2 made render-real), and a commented-out
    /// statement keeps the muted grey.
    #[test]
    fn syn_attrs_comment_tiers() {
        use crate::syntax::SynKind;
        let _g = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        theme::set_active_by_name("Tawny").unwrap();
        let base = Attrs::new();
        let th = theme::active();
        assert_eq!(
            syn_attrs(&base, SynKind::Comment).color_opt,
            Some(th.base_content.to_glyphon()),
            "prose comment shapes at FULL content ink"
        );
        assert_eq!(
            syn_attrs(&base, SynKind::CommentCode).color_opt,
            Some(th.muted.to_glyphon()),
            "commented-out code keeps the muted grey"
        );
        theme::set_active(theme::DEFAULT_THEME);
    }

    #[test]
    fn editing_text_reshapes_exactly_once_per_change() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping editing_text_reshapes_exactly_once_per_change: no wgpu adapter");
            return;
        };
        p.set_view(&view("alpha\nbeta", 0, 0));
        let base = p.reshape_count;
        // Append a char on line 1 (a keystroke): exactly one reshape.
        p.set_view(&view("alpha\nbetax", 1, 5));
        assert_eq!(p.reshape_count, base + 1, "one edit => one reshape");
        // Re-pushing the IDENTICAL text (e.g. the cursor-follow second push) must
        // not reshape again.
        p.set_view(&view("alpha\nbetax", 1, 5));
        assert_eq!(
            p.reshape_count,
            base + 1,
            "re-pushing identical text must not reshape"
        );
    }

    #[test]
    fn incremental_matches_full_shape_geometry() {
        // The incremental path must produce the SAME shaped geometry (total visual
        // rows + caret target) as the old whole-buffer reshape, on a doc that wraps.
        // Both pipelines wrap at the live `column_width()`, which folds BOTH the
        // global theme font (char width) and the global page state (measure). Hold
        // both locks so neither a concurrent theme switch nor a page toggle can flip
        // the wrap width between the two shapes and split the row counts.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p_incr) = headless_pipeline() else {
            eprintln!("skipping incremental_matches_full_shape_geometry: no wgpu adapter");
            return;
        };
        let Some(mut p_full) = headless_pipeline() else {
            return;
        };
        // A few long lines so soft-wrap produces multiple visual rows per line.
        let long = "wrap ".repeat(60);
        let text = format!("{long}\nshort\n{long}\nend");
        p_incr.set_view(&view(&text, 0, 0));
        p_full.set_text_full(&text);
        assert_eq!(
            p_incr.total_visual_rows(),
            p_full.total_visual_rows(),
            "incremental + full reshape must agree on total visual rows"
        );
        // Now EDIT line 1 incrementally and compare against a fresh full reshape of
        // the edited text: the per-line cache reuse must not drift the geometry.
        let edited = format!("{long}\nshorter!!\n{long}\nend");
        p_incr.set_view(&view(&edited, 1, 9));
        let mut p_full2 = headless_pipeline().unwrap();
        p_full2.set_text_full(&edited);
        assert_eq!(
            p_incr.total_visual_rows(),
            p_full2.total_visual_rows(),
            "after an incremental edit, geometry must match a full reshape"
        );
    }

    #[test]
    fn total_visual_rows_is_cached_between_reads() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping total_visual_rows_is_cached_between_reads: no wgpu adapter");
            return;
        };
        p.set_view(&view("a\nb\nc", 0, 0));
        let r1 = p.total_visual_rows();
        // A cursor-only change must NOT reshape, so the cached row count is reused
        // and still correct.
        p.set_view(&view("a\nb\nc", 2, 1));
        assert_eq!(p.total_visual_rows(), r1);
        // A real edit (add a line) must refresh the count.
        p.set_view(&view("a\nb\nc\nd", 3, 1));
        assert_eq!(p.total_visual_rows(), r1 + 1);
    }

    /// CRLF LINE-MODEL AGREEMENT (the render half): RESOLVED (was the pinned
    /// divergence). A Windows-ended document is now NORMALIZED on load
    /// (`Buffer::from_file` strips every '\r\n' to '\n' — the VS Code model), so
    /// the [`Buffer`] (ropey, LF-only counting) and the pipeline (splits the pushed
    /// text on '\n') agree on the logical line count AND on every shaped line's
    /// content — there is no leftover '\r' to ride in as a phantom trailing column.
    /// Loading through the real `from_file` seam (over an `InMemoryFs`) is what
    /// exercises the normalization; a raw `from_str("a\r\nb")` would keep the CR as
    /// content (characterized buffer-side).
    #[test]
    fn crlf_buffer_and_pipeline_line_models_agree_on_count() {
        use crate::buffer::{Buffer, Eol};
        use std::sync::Arc;
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping crlf_buffer_and_pipeline_line_models_agree_on_count: no wgpu adapter");
            return;
        };
        let path = std::path::PathBuf::from("/docs/win.md");
        let mem = crate::fs::InMemoryFs::new().with_file(&path, "a\r\nb\r\nc");
        crate::fs::with_fs(Arc::new(mem), || {
            let buf = Buffer::from_file(&path);
            assert_eq!(buf.eol(), Eol::Crlf, "detected CRLF");
            // The rope is PURELY '\n' — no CR survives the load.
            assert_eq!(buf.text(), "a\nb\nc", "CRLF normalized to LF on load");
            assert_eq!(buf.line_count(), 3);
            p.set_view(&view(&buf.text(), 0, 0));
            assert_eq!(
                p.line_count(),
                buf.line_count(),
                "buffer and pipeline agree on the logical line count of a CRLF doc"
            );
            // RESOLVED: the shaped line carries NO phantom '\r' — line 0 is exactly
            // "a" (1 char → 2 x-boundaries), matching the buffer's own content.
            assert_eq!(
                p.buffer.lines[0].text(),
                "a",
                "the pipeline line no longer retains a CR (no phantom column)"
            );
            assert_eq!(
                p.line_glyph_xs(0).len(),
                2,
                "1 char ('a') => 2 x-boundaries on line 0"
            );
        });
    }

    /// The BLOCK caret quad's resting WIDTH tracks the REAL shaped glyph advance at
    /// the cursor: on a PROPORTIONAL world it is wide on `m` and narrow on `i`
    /// (exactly the glyph's advance, no fixed-cell floor); on a MONO world it is the
    /// constant cell and byte-identical to the old `caret_target_w`.
    #[test]
    fn block_caret_width_tracks_glyph_advance() {
        // Advance reads fold the theme font AND the page wrap globals; hold both
        // (theme → page order, page.rs:95-99). The block width is read at the
        // mode-keyed ANCHOR cell (Morph shifts one back, and the no-override
        // default follows the active font — proportional Gumtree would latch
        // Morph); hold the caret lock and pin BLOCK, the look under test.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let _c = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::caret::set_mode(CaretMode::Block);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping block_caret_width_tracks_glyph_advance: no wgpu adapter");
            return;
        };
        let text = "milk"; // col 0 = 'm' (wide), col 1 = 'i' (narrow)

        // PROPORTIONAL (Gumtree = Literata): the block width is the REAL glyph
        // advance, so the wide 'm' yields a wider block than the narrow 'i' and the
        // narrow glyph drops BELOW the fixed cell — the old `.max(caret_w)` floor,
        // which pinned every cell to caret_w, is gone on proportional faces.
        theme::set_active_by_name("Gumtree").unwrap();
        p.sync_theme();
        p.set_view(&view(text, 0, 0)); // on 'm'
        let w_m = p.caret_block_w();
        let (_x, adv_m) = p.col_x_and_advance(0, 0);
        p.set_view(&view(text, 0, 1)); // on 'i'
        let w_i = p.caret_block_w();
        let (_x, adv_i) = p.col_x_and_advance(0, 1);
        assert!(
            w_m > w_i + 1.0,
            "proportional block must be wider on 'm' than 'i' (m={w_m}, i={w_i})"
        );
        // The block is EXACTLY the real glyph advance (no floor) on each glyph.
        assert!((w_m - adv_m).abs() < 1e-3, "block 'm' == real advance ({w_m} vs {adv_m})");
        assert!((w_i - adv_i).abs() < 1e-3, "block 'i' == real advance ({w_i} vs {adv_i})");
        // ...and the narrow glyph is thinner than the old fixed cell — proof the
        // floor that made the block too wide on thin glyphs is gone.
        assert!(
            w_i < p.metrics.caret_w,
            "narrow 'i' block must be thinner than the fixed cell (i={w_i}, cell={})",
            p.metrics.caret_w
        );

        // MONO (Tawny = IBM Plex Mono): the historical `.max(caret_w)` floor is kept,
        // so the BLOCK width is byte-identical to the old `caret_target_w` at every
        // column — the mono block is unchanged. (Keyed on the EFFECTIVE shaped
        // family — the declared doc family, not the resolved face — so this holds
        // even where the mono face isn't installed and shaping falls back: Tawny
        // still renders exactly as it did before.)
        theme::set_active_by_name("Tawny").unwrap();
        p.sync_theme();
        for col in 0..text.chars().count() {
            p.set_view(&view(text, 0, col));
            assert!(
                (p.caret_block_w() - p.caret_target_w()).abs() < 1e-6,
                "mono block must equal the old caret_target_w at col {col} (unchanged)"
            );
            // On a glyph at/above the cell the floor is a no-op (block == advance);
            // a narrow glyph is floored UP to the fixed cell — exactly the old block.
            assert!(
                p.caret_block_w() >= p.metrics.caret_w - 1e-3,
                "mono block never drops below the fixed cell at col {col}"
            );
        }

        // Restore the default world so other tests see a clean global.
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// REGRESSION (the wrap-boundary SLIVER): the SPACE where a long line
    /// soft-wraps gets NO visible glyph in its row — cosmic-text collapses the
    /// trailing whitespace at the break, so its two x boundaries coincide at the
    /// row's right edge and the raw cell width is ~0. The block caret drawn from
    /// that advance rendered as a ~1px sliver (reported on Mangrove = JetBrains
    /// Mono). `col_x_and_advance` must rescue such a DEGENERATE cell to the
    /// default cell width, so the block caret keeps a full visible cell there —
    /// on mono AND proportional worlds alike — while genuinely narrow glyphs
    /// (`i`, `l`) keep their real advance (no too-wide floor reintroduced; see
    /// `block_caret_width_tracks_glyph_advance`).
    #[test]
    fn block_caret_full_cell_on_wrap_boundary_space() {
        // The wrap boundary IS the fixture: it folds the theme font AND the page
        // wrap globals, so hold both (theme → page order, page.rs:95-99). The block
        // width is read at the mode-keyed ANCHOR cell and the no-override default
        // follows the active font (proportional Gumtree would latch Morph); hold
        // the caret lock and pin BLOCK, the look under test.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let _c = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::caret::set_mode(CaretMode::Block);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping block_caret_full_cell_on_wrap_boundary_space: no wgpu adapter");
            return;
        };
        let long = "word ".repeat(80); // 400 chars, wraps on the 1200px canvas

        // A world for each shaping family: the reported mono world (Mangrove =
        // JetBrains Mono) and a proportional world (Gumtree = Literata) — the
        // degenerate-cell rescue must hold on both.
        for world in ["Mangrove", "Gumtree"] {
            theme::set_active_by_name(world).unwrap();
            p.sync_theme();
            p.set_view(&view(&long, 0, 0));
            let rows = p.visual_rows(0);
            assert!(rows.len() >= 2, "{world}: long line should wrap ({} rows)", rows.len());
            // The wrap-boundary SPACE: the char just before the second row's
            // start. It belongs to the FIRST row (pick_row's half-open span), at
            // the row's right edge, where its collapsed cell is the degenerate one.
            let space_col = rows[1].start_col - 1;
            assert_eq!(
                long.chars().nth(space_col),
                Some(' '),
                "{world}: the wrap boundary lands on the collapsed space"
            );
            // Prove the setup reproduces the degenerate cell: the RAW x delta of
            // the collapsed space is a sliver, far below a real glyph advance.
            let row = &rows[0];
            let raw = row.xs[space_col + 1] - row.xs[space_col];
            assert!(
                raw < p.metrics.char_width * 0.2,
                "{world}: wrap-boundary space cell should be collapsed (raw={raw})"
            );
            // The rescued advance is a full default cell...
            let (_x, adv) = p.col_x_and_advance(0, space_col);
            assert!(
                (adv - p.metrics.char_width).abs() < 1e-3,
                "{world}: degenerate cell advance rescued to char_width (adv={adv})"
            );
            // ...and the BLOCK caret quad drawn there is a visible full cell, not
            // the ~1px sliver.
            p.set_view(&view(&long, 0, space_col));
            let w = p.caret_block_w();
            assert!(
                w >= p.metrics.char_width * 0.5,
                "{world}: block caret at the wrap-boundary space must be a visible \
                 cell, not a sliver (w={w}, cell={})",
                p.metrics.char_width
            );
        }

        // Restore the default world so other tests see a clean global.
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// THE KERNED-GLYPH CARET FIX: on a PROPORTIONAL world, when the caret's
    /// anchor column maps ONE-TO-ONE onto a single shaped glyph, the BLOCK
    /// caret's settled rest quad must sit EXACTLY on that glyph's own swash ink
    /// box (what MORPH already recolours) — never the naive advance CELL.
    /// Reproduces the reported bug: on "awl" (Mopoke = iA Writer Quattro S) the
    /// middle 'w' has a nonzero ink left-bearing AND an ink width narrower than
    /// its advance cell, so the OLD cell-only block quad sat visibly offset +
    /// narrow against the real glyph while Morph (which already samples the
    /// glyph) did not.
    #[test]
    fn block_caret_ink_aligns_on_kerned_glyph() {
        // Ink-box lookup rides the theme font AND the page wrap globals; the
        // anchor is mode-keyed. Hold theme -> page -> caret (the suite-wide
        // order), pin BLOCK, restore both globals after.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let _c = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::caret::set_mode(CaretMode::Block);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping block_caret_ink_aligns_on_kerned_glyph: no wgpu adapter");
            return;
        };
        theme::set_active_by_name("Mopoke").unwrap(); // proportional (iA Writer Quattro S)
        p.sync_theme();
        let text = "awl"; // col 1 = 'w', kerned between 'a' and 'l'
        p.set_view(&view(text, 0, 1));
        p.settle_caret();

        // The naive CELL box (the pre-fix geometry): the advance-derived width.
        let (_cell_x, cell_adv) = p.col_x_and_advance(0, 1);

        // The glyph's real ink box — the SAME swash lookup MORPH's silhouette reads.
        let (ink_left, ink_w) = p
            .caret_anchor_ink_box()
            .expect("a single 'w' glyph on a proportional world must yield an ink box");

        // Fixture sanity: this glyph's ink really DOES diverge from its advance
        // cell (a nonzero left bearing and/or a width mismatch) — otherwise this
        // test would pass even with the old, unfixed cell-only geometry.
        assert!(
            ink_left.abs() > 0.5 || (ink_w - cell_adv).abs() > 0.5,
            "fixture must reproduce a real cell/ink divergence: left={ink_left} ink_w={ink_w} cell_adv={cell_adv}"
        );

        // The settled BLOCK quad must sit EXACTLY on the glyph's ink box, not
        // the naive cell.
        let pen_x = p.caret.pos.x;
        let (cx, _cy, w, _h, _corner, _ax, _ay) = p.caret_geometry();
        let got_left = cx - w * 0.5;
        assert!(
            (got_left - (pen_x + ink_left)).abs() < 1e-2,
            "block left edge must equal the glyph's ink left: got {got_left} want {}",
            pen_x + ink_left
        );
        assert!(
            (w - ink_w).abs() < 1e-2,
            "block width must equal the glyph's ink width: got {w} want {ink_w}"
        );

        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
        crate::caret::set_mode(CaretMode::Block);
    }

    /// The ink-box override is DELIBERATELY scoped OFF in two cases, both
    /// asserted here:
    ///   * a MONO world (the default, Tawny): a monospace display wants a
    ///     perfectly uniform caret grid, not per-glyph ink wobble, so
    ///     `caret_anchor_ink_box` must return `None` at every column and the
    ///     existing `.max(caret_w)`-floored cell math
    ///     (`block_caret_width_tracks_glyph_advance`) stays untouched.
    ///   * a LIGATURE cluster (`cluster_span_at` reports a char span > 1): the
    ///     guard is exercised directly at the pure free-function seam with a
    ///     SYNTHETIC 2-char cluster, mirroring how `assemble_glyph_xs` is
    ///     unit-tested without a GPU. (Since the ligature-policy round enabled
    ///     `liga` on the proportional prose faces, a real prose "fi"/"ffi" now
    ///     DOES merge to one glyph — the empirical "never ligates" note that used
    ///     to sit here was true only while ligatures were globally disabled; the
    ///     real-cluster caret/hit-test path is now covered on the code monos by
    ///     `caret_and_hit_test_are_per_char_inside_a_programming_ligature_cluster`.)
    #[test]
    fn caret_ink_box_off_for_mono_and_ligature_cluster() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let _c = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::caret::set_mode(CaretMode::Block);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping caret_ink_box_off_for_mono_and_ligature_cluster: no wgpu adapter");
            return;
        };

        // MONO world (Tawny, the default).
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
        let text = "awl";
        for col in 0..text.chars().count() {
            p.set_view(&view(text, 0, col));
            assert!(
                p.caret_anchor_ink_box().is_none(),
                "mono world must never ink-align (col {col})"
            );
        }

        // LIGATURE fallback: the pure cluster-span seam, synthetic multi-char
        // cluster (as if "fi" shaped to a single glyph).
        assert_eq!(
            cluster_span_at("fi", &[(0, 2)], 0),
            Some(2),
            "a 2-char ligature cluster spans 2 chars"
        );
        assert_eq!(
            cluster_span_at("fi", &[(0, 1), (1, 2)], 0),
            Some(1),
            "one glyph per char spans 1 (the common case)"
        );
        assert_eq!(
            cluster_span_at("fi", &[(0, 2)], 5),
            None,
            "no cluster owns an out-of-range byte"
        );

        crate::caret::set_mode(CaretMode::Block);
    }

    /// FIX 2 (MORPH LINE-START DEGRADE): the cosmetic | trail must anchor on the
    /// caret's CURRENT FORM, not the raw caret-mode global. When Morph melts to
    /// the line-start insertion BAR (col 0 / a fresh line / an empty line —
    /// [`crate::caret::morph_line_start`]) the trail must anchor at the bar's
    /// LEFT-EDGE x, exactly like the real I-beam look's trail — NOT the
    /// glyph-cell centre a settled/space-bar Morph caret uses. Extends
    /// `cosmetic_trail_anchor_is_mode_aware` (FIX 1) with the Morph-specific
    /// bar-form case.
    #[test]
    fn cosmetic_trail_anchor_follows_morph_linestart_bar() {
        // The anchor x's fold the page globals; mutates the process-global
        // caret mode. Hold both shared test locks (page -> caret, the suite-wide
        // order).
        let _p = crate::page::test_lock();
        let _g = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::caret::set_mode(CaretMode::Morph);
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping cosmetic_trail_anchor_follows_morph_linestart_bar: no wgpu adapter");
            return;
        };
        let text = "abc\ndef";
        // Cursor at col 0 of line 1 ("def"): the Morph LINE-START degrade — no
        // produced glyph before the insertion point, so the caret melts to the
        // I-beam's thin insertion bar.
        p.set_view(&view(text, 1, 0));
        assert!(
            crate::caret::morph_line_start(p.cursor_col),
            "fixture must sit at a Morph line-start degrade"
        );
        let (tx, ty) = p.caret_target_xy();

        // A VERTICAL kick (same column, one row up->down) so the | always shows.
        let from = Sample { x: tx, y: ty - p.metrics.line_height };
        let to = Sample { x: tx, y: ty };
        p.caret.kick_trail(from, to, false);
        p.caret.step_trail(0.03);
        let (morph_bar_x, ..) = p.caret_trail_geometry().expect("morph line-start trail active");

        // The SAME anchor the real I-beam bar uses at the SAME insertion x.
        let want_bar = tx + IBEAM_W * p.metrics.zoom * 0.5;
        assert!(
            (morph_bar_x - want_bar).abs() < 1e-3,
            "morph line-start | must anchor on the bar: got {morph_bar_x} want {want_bar}"
        );

        // Contrast: a MID-LINE Morph caret (settled on a real glyph, cell-form)
        // keeps anchoring on the CELL centre, unchanged.
        p.set_view(&view(text, 0, 2)); // "ab|c": anchors the 'b' glyph, cell-form
        assert!(
            !crate::caret::morph_line_start(p.cursor_col),
            "fixture must NOT be a line start"
        );
        let (tx2, ty2) = p.caret_target_xy();
        let from2 = Sample { x: tx2, y: ty2 - p.metrics.line_height };
        let to2 = Sample { x: tx2, y: ty2 };
        p.caret.kick_trail(from2, to2, false);
        p.caret.step_trail(0.03);
        let (morph_cell_x, ..) = p.caret_trail_geometry().expect("morph cell-form trail active");
        let want_cell = tx2 + p.caret_block_w() * 0.5;
        assert!(
            (morph_cell_x - want_cell).abs() < 1e-3,
            "morph cell-form | must still anchor on the cell centre: got {morph_cell_x} want {want_cell}"
        );

        crate::caret::set_mode(CaretMode::Block);
    }

    /// INVARIANT: the document buffer's soft-wrap width must equal the live page
    /// COLUMN width after EVERY frame, so the centered page floats with a styled
    /// margin on BOTH sides at any window size / DPI — never running off the right
    /// edge. Drives the precise live failure mode (a page-state flip that does not
    /// re-wrap, then non-reshaping frames) and asserts `prepare`'s per-frame
    /// `sync_wrap_width` heals it. Regression guard for the LEFT-aligned / clipped
    /// right-margin bug.
    #[test]
    fn page_buffer_wrap_always_equals_column_width() {
        // `column_width()` folds BOTH the global theme font (char width) and the
        // global page state (measure); this test reads it repeatedly and asserts it
        // stays self-consistent across a frame, so hold both locks to bar a concurrent
        // theme switch or page toggle from flipping it between the heal and the assert.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping page_buffer_wrap_always_equals_column_width: no wgpu adapter");
            return;
        };
        let text = "the quick brown fox jumps over the lazy dog\nsecond line of prose here";
        let assert_synced = |p: &mut TextPipeline, tag: &str| {
            // `prepare` enforces the invariant once per frame; re-derive + compare.
            // The buffer wraps at the inset TEXT width (column minus the writing pad
            // on both sides), not the full surface column.
            let want = p.text_wrap_width();
            let have = p.buffer.size().0.unwrap_or(f32::NAN);
            assert!(
                (have - want).abs() <= 0.5,
                "{tag}: buffer wrap {have} != text_wrap_width {want} (page would clip right)"
            );
            // The centered column must leave a margin on BOTH sides.
            let right_margin = p.window_w - (p.column_left() + p.column_width());
            assert!(
                right_margin >= 0.0,
                "{tag}: right margin {right_margin} < 0 (no right margin)"
            );
        };

        // Retina-like startup: set_size at physical BEFORE set_dpi (Gpu::new order).
        // Reads the process-global page state without MUTATING it, so this test is
        // parallel-safe with the other render tests.
        p.set_size(2400.0, 1600.0);
        p.set_dpi(2.0);
        p.set_view(&view(text, 0, 0));
        p.sync_wrap_width();
        assert_synced(&mut p, "startup-retina");

        // The precise failure mode, reproduced WITHOUT touching any global: force the
        // buffer to a STALE, too-wide wrap (as a wider prior window / edge-to-edge
        // wrap would leave it), exactly as the live bug does when a page-state change
        // doesn't re-wrap and only non-reshaping frames follow. `sync_wrap_width` (run
        // by `prepare` every frame) must heal it back to the centered column width.
        let stale_wide = p.window_w + 400.0; // wider than the window -> overflows right
        let shape_h = p.full_shape_height();
        p.buffer
            .set_size(&mut p.font_system, Some(stale_wide), Some(shape_h));
        // A cursor-only set_view does NOT reshape, so it must NOT itself heal — proving
        // the heal comes from the per-frame `sync_wrap_width`, not the edit path.
        p.set_view(&view(text, 0, 1));
        p.sync_wrap_width();
        assert_synced(&mut p, "after-stale-wide-wrap");

        // And again after a no-text-change re-push (settled idle frame stays synced).
        p.set_view(&view(text, 0, 1));
        p.sync_wrap_width();
        assert_synced(&mut p, "settled-frame");
    }

    /// CURSOR SHAPE: `TextPipeline::over_writing_column` must agree with the SAME
    /// `column_left`/`column_width` the page-resize hover test reads — a click
    /// clearly inside the column reads `true`, a click clearly out past the margin
    /// (page mode on, with real margin room) reads `false`. Holds both TEST_LOCKs
    /// like every other test reading page-folding geometry (CLAUDE.md's flake note).
    #[test]
    fn over_writing_column_agrees_with_the_page_column_bounds() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping over_writing_column_agrees_with_the_page_column_bounds: no wgpu adapter");
            return;
        };
        p.set_size(1200.0, 800.0);
        let was_on = crate::page::page_on();
        let was_measure = crate::page::measure();
        crate::page::set_page_on(true);
        crate::page::set_measure(40);
        let left = p.column_left();
        let width = p.column_width();
        assert!(p.over_writing_column(left + width * 0.5), "column center is over the writing column");
        assert!(!p.over_writing_column(left - 20.0), "well past the left margin is not");
        assert!(!p.over_writing_column(left + width + 20.0), "well past the right margin is not");
        crate::page::set_page_on(was_on);
        crate::page::set_measure(was_measure);
    }

    /// The vertical-motion sweep body shared by the CLAUDE.md width-grid test and
    /// the bullet+bold fixture test: for the CURRENTLY-shaped document, assert that
    /// ONE `visual_line_down` / `visual_line_up` step from EVERY (line, col, goal_x)
    /// is STRICTLY monotonic in the whole-doc visual-row partition (no fixed point,
    /// no backward step), then that FULL hold-down / hold-up walks (the user's
    /// held-arrow gesture, `vertical_motion`-faithful: sticky goal_x + the buffer
    /// round-trip) reach the far document edge without wedging.
    ///
    /// The (col, goal_x) loops enumerate REPRESENTATIVES instead of every value,
    /// with no loss of coverage: a step's landing depends only on the START ROW
    /// (`pick_row_index(col)`) and `goal_x` — never on which of that row's columns
    /// the caret held — so per row its `start_col` (strict owner) and its `end_col`
    /// (the wrap-boundary column — owned by the NEXT row at a shared boundary, by
    /// THIS row at a gapped/EOL one) cover both ownership regimes; and the landing
    /// is a step function of `goal_x` whose breakpoints are the TARGET row's own
    /// cell boundaries, so that row's start/mid/end x + the two extremes sample
    /// every landing regime (incl. the past-content default that lands on the
    /// shared wrap-boundary column — the historical stick). `walks_only` keeps just
    /// the held-arrow walks — the cheap mode the wide width-grid points use.
    fn assert_vertical_sweep_clean(p: &TextPipeline, text: &str, label: &str, walks_only: bool) {
        use crate::actions::LayoutOracle;
        use crate::buffer::Buffer;
        let n = p.line_count();
        let all_rows: Vec<Vec<VisualRow>> = (0..n).map(|l| p.line_rows_local(l)).collect();
        let mut cum = vec![0usize; n + 1];
        for l in 0..n {
            cum[l + 1] = cum[l] + all_rows[l].len();
        }
        let total = cum[n];
        let gvrow =
            |line: usize, col: usize| -> usize { cum[line] + pick_row_index(&all_rows[line], col) };

        // goal_x spread for stepping INTO `target`: the landing is a step function
        // of goal_x whose breakpoints are that row's own cell boundaries, so its
        // start/mid/end x + the two extremes sample every landing regime (incl. the
        // past-content default that lands on the wrap-boundary column).
        let gxs_for = |target: &VisualRow| -> [f32; 5] {
            let sx = target.xs.get(target.start_col).copied().unwrap_or(0.0);
            let ex = target.xs.get(target.end_col).copied().unwrap_or(0.0);
            [0.0, sx, (sx + ex) * 0.5, ex, 100_000.0]
        };
        let mut bad: Vec<String> = Vec::new();
        let sweep_lines = if walks_only { 0 } else { n };
        for line in 0..sweep_lines {
            let rows = &all_rows[line];
            for (idx, row) in rows.iter().enumerate() {
                // Representative columns of THIS row: start + wrap-boundary end.
                let cols = [row.start_col, row.end_col];
                // The DOWN step's target row: the next row of this line, else the
                // NEXT line's first row (None at the document bottom).
                let down_target: Option<&VisualRow> = rows
                    .get(idx + 1)
                    .or_else(|| all_rows.get(line + 1).and_then(|r| r.first()));
                // The UP step's target: the previous row, else the PREVIOUS line's
                // last row (None at the document top).
                let up_target: Option<&VisualRow> = idx
                    .checked_sub(1)
                    .and_then(|i| rows.get(i))
                    .or_else(|| line.checked_sub(1).and_then(|l| all_rows[l].last()));
                for &col in cols.iter().take(if cols[0] == cols[1] { 1 } else { 2 }) {
                    let g0 = gvrow(line, col);
                    if let Some(t) = down_target {
                        for gx in gxs_for(t) {
                            let (dl, dc) = p.visual_line_down(line, col, gx);
                            if (dl, dc) == (line, col) {
                                if g0 + 1 != total {
                                    bad.push(format!(
                                        "{label}: DOWN fixed point line={line} col={col} gx={gx:.1}"
                                    ));
                                }
                            } else if gvrow(dl, dc) <= g0 {
                                bad.push(format!(
                                    "{label}: DOWN non-descending line={line} col={col} \
                                     gx={gx:.1} g{g0} -> ({dl},{dc}) g{}",
                                    gvrow(dl, dc)
                                ));
                            }
                        }
                    }
                    if let Some(t) = up_target {
                        for gx in gxs_for(t) {
                            let (ul, uc) = p.visual_line_up(line, col, gx);
                            if (ul, uc) == (line, col) {
                                if g0 != 0 {
                                    bad.push(format!(
                                        "{label}: UP fixed point line={line} col={col} gx={gx:.1}"
                                    ));
                                }
                            } else if gvrow(ul, uc) >= g0 {
                                bad.push(format!(
                                    "{label}: UP non-ascending line={line} col={col} \
                                     gx={gx:.1} g{g0} -> ({ul},{uc}) g{}",
                                    gvrow(ul, uc)
                                ));
                            }
                        }
                    }
                }
            }
        }
        for s in bad.iter().take(25) {
            eprintln!("  {s}");
        }
        assert!(bad.is_empty(), "{label}: {} sweep violations (total rows {total})", bad.len());

        // FULL WALKS — the exact held-arrow gesture, vertical_motion-faithful.
        let last_line = n - 1;
        for &goal in &[0.0f32, 700.0, 100_000.0] {
            for &down in &[true, false] {
                let mut buf = Buffer::from_str(text);
                let seed = if down { (0usize, 0usize) } else { (last_line, 0usize) };
                buf.set_cursor_visual(buf.line_col_to_char(seed.0, seed.1), goal);
                let mut steps = 0usize;
                loop {
                    let (line, col) = buf.cursor_line_col();
                    let gx = buf.goal_x().unwrap_or_else(|| p.visual_x_of(line, col));
                    let (nl, nc) = if down {
                        p.visual_line_down(line, col, gx)
                    } else {
                        p.visual_line_up(line, col, gx)
                    };
                    let before = buf.cursor_char();
                    buf.set_cursor_visual(buf.line_col_to_char(nl, nc), gx);
                    if buf.cursor_char() == before {
                        let (fl, _fc) = buf.cursor_line_col();
                        let want = if down { last_line } else { 0 };
                        assert_eq!(
                            fl, want,
                            "{label}: {} walk (goal_x={goal}) STUCK at line {fl} after {steps} steps",
                            if down { "DOWN" } else { "UP" }
                        );
                        break;
                    }
                    steps += 1;
                    assert!(
                        steps <= total + 50,
                        "{label}: runaway walk (down={down}, goal_x={goal})"
                    );
                }
            }
        }
    }

    /// The "holding arrow-down gets stuck" hunt, PINNED over the repo's own
    /// CLAUDE.md (markdown bullets with **bold** spans wrapping across rows — the
    /// reported stick was line 11's `- **PHILOSOPHY.md** — …` bullet) at a GRID of
    /// wrap widths + a HiDPI point: the live window is an arbitrary size, so a
    /// wrap-boundary seam can exist at widths the default 1200px canvas never
    /// shapes. The default width runs the full strict-monotonicity sweep; the other
    /// grid points (and the dpi-2 Retina point) run the held-arrow walks — the
    /// user's exact gesture — to keep the suite fast. GPU-backed; skips with no
    /// adapter.
    #[test]
    fn oracle_vertical_sweep_claude_md_across_widths() {
        // Wrap geometry reads the page/theme globals; hold their test locks so a
        // parallel mutator can't re-wrap the document mid-sweep.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping oracle_vertical_sweep_claude_md_across_widths: no wgpu adapter");
            return;
        };
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/CLAUDE.md"))
            .expect("CLAUDE.md present at crate root");
        let mut v = view(&text, 0, 0);
        v.is_markdown = true;
        p.set_view(&v);
        assert_vertical_sweep_clean(&p, &text, "CLAUDE.md w=1200", false);
        for w in [560.0f32, 900.0, 1620.0] {
            p.set_size(w, 800.0);
            assert_vertical_sweep_clean(&p, &text, &format!("CLAUDE.md w={w}"), true);
        }
        // HiDPI: the live Retina window (dpi 2) shapes at doubled metrics — walk
        // one doubled-width point so the scaled advances get the same guarantee.
        p.set_dpi(2.0);
        p.set_size(2400.0, 1600.0);
        assert_vertical_sweep_clean(&p, &text, "CLAUDE.md dpi=2 w=2400", true);
        p.set_dpi(1.0);
        p.set_size(1200.0, 800.0);
    }

    /// The reported stick's LINE SHAPE, synthetically: markdown BULLET lines whose
    /// **bold** span (shaped in the bold-fallback face, so its advances differ from
    /// the body) sits right in the wrap band, plus em-dashes and long wrapping
    /// prose — `- **Word.md** — long prose that wraps…`. Swept over several widths
    /// so the bold-run boundary crosses the wrap edge somewhere in the grid.
    /// GPU-backed; skips with no adapter.
    #[test]
    fn oracle_vertical_sweep_bullet_bold_fixture() {
        // Wrap geometry reads the page/theme globals; hold their test locks so a
        // parallel mutator can't re-wrap the document mid-sweep.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping oracle_vertical_sweep_bullet_bold_fixture: no wgpu adapter");
            return;
        };
        let mut text = String::from("# Fixture — contract docs\n\ncontract docs:\n");
        for i in 0..8 {
            text.push_str(&format!(
                "- **DOC{i}.md** — why the fixture is the way it is; the design principles; \
                 the root doc; a further clause so the bullet line wraps across several \
                 visual rows at every width in the grid, keeping the bold span near an edge.\n"
            ));
        }
        text.push_str("\ntrailing prose after the list, long enough to wrap as well when the \
                       column narrows to the smallest width in the sweep grid below.\n");
        let mut v = view(&text, 0, 0);
        v.is_markdown = true;
        p.set_view(&v);
        for w in [480.0f32, 620.0, 760.0, 900.0, 1040.0, 1200.0, 1400.0, 1680.0] {
            p.set_size(w, 800.0);
            assert_vertical_sweep_clean(&p, &text, &format!("fixture w={w}"), false);
        }
        p.set_size(1200.0, 800.0);
    }

    /// `set_size` must INVALIDATE the row-geometry caches when it actually re-wraps:
    /// the live window-resize / page-mode-toggle / page-width paths all re-wrap
    /// through it, and the following `prepare`'s `sync_wrap_width` sees the width
    /// already in sync (skipping its own invalidate) — so a stale cache here left
    /// every post-resize scroll / caret-row / hit-test answering from the PRE-resize
    /// geometry until the next text edit (a live-only de-sync no capture replays,
    /// since captures size the pipeline before the text). GPU-backed; skips with no
    /// adapter.
    #[test]
    fn set_size_rewrap_invalidates_row_geometry() {
        // Wrap geometry reads the page/theme globals; hold their test locks so a
        // parallel mutator can't change the wrap width under the comparison.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping set_size_rewrap_invalidates_row_geometry: no wgpu adapter");
            return;
        };
        let text = "word ".repeat(300); // one long soft-wrapping line
        p.set_view(&view(&text, 0, 0));
        let total_wide = p.total_visual_rows();
        let rows0_wide = p.visual_rows(0).len(); // warms the single-slot memo

        p.set_size(600.0, 800.0); // live resize: the buffer re-wraps ~2x as tall
        let total_after = p.total_visual_rows();
        let rows0_after = p.visual_rows(0).len();
        let top_after = p.row_top_px(total_after - 1);

        // Ground truth: drop every cache and recompute from the shaped runs.
        p.row_geom.invalidate();
        assert_eq!(
            total_after,
            p.total_visual_rows(),
            "total_visual_rows must be re-derived after a re-wrapping set_size"
        );
        assert_eq!(
            rows0_after,
            p.visual_rows(0).len(),
            "the cursor-line VisualRow memo must be dropped by a re-wrapping set_size"
        );
        assert!(
            (top_after - p.row_top_px(p.total_visual_rows() - 1)).abs() < 0.5,
            "row tops must be re-derived after a re-wrapping set_size"
        );
        // And the narrower wrap really did change the geometry (the test bites).
        assert!(
            total_after > total_wide && rows0_after > rows0_wide,
            "narrower wrap must yield more rows: {total_wide} -> {total_after}"
        );
    }

    /// `App::sync_page_measure` (the prose/code page-width split's buffer-switch
    /// resync) re-applies `page::set_measure` then calls `set_size` with the
    /// SAME window dimensions as before — no resize, just a measure change. This
    /// proves `set_size` still detects THAT re-wrap and invalidates row geometry
    /// even when the window itself hasn't moved (the exact mechanism the App-level
    /// switch depends on to answer FRESH geometry the very next frame, not stale
    /// pre-switch layout — the "mind RowGeom invalidation" seam this round leans on
    /// rather than reinventing).
    #[test]
    fn measure_change_alone_invalidates_row_geometry_on_the_next_set_size() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping measure_change_alone_invalidates_row_geometry_on_the_next_set_size: no wgpu adapter");
            return;
        };
        crate::page::set_page_on(true);
        crate::page::set_measure(crate::page::DEFAULT_MEASURE); // 70: a prose-width column
        let text = "word ".repeat(300); // one long soft-wrapping line
        p.set_view(&view(&text, 0, 0));
        p.set_size(1200.0, 800.0); // re-derive wrap at the prose measure
        let total_prose = p.total_visual_rows();
        let rows0_prose = p.visual_rows(0).len(); // warms the single-slot memo

        // Switch measure only (mirrors a buffer switch to a CODE file) — SAME
        // window size as before, so any staleness here is measure-caused alone.
        crate::page::set_measure(crate::page::DEFAULT_MEASURE_CODE); // 100: wider
        p.set_size(1200.0, 800.0);
        let total_code = p.total_visual_rows();
        let rows0_code = p.visual_rows(0).len();
        let top_code = p.row_top_px(total_code - 1);

        // Ground truth: drop every cache and recompute from the shaped runs.
        p.row_geom.invalidate();
        assert_eq!(
            total_code,
            p.total_visual_rows(),
            "total_visual_rows must be re-derived after a measure-only set_size"
        );
        assert_eq!(
            rows0_code,
            p.visual_rows(0).len(),
            "the cursor-line VisualRow memo must be dropped by a measure-only set_size"
        );
        assert!(
            (top_code - p.row_top_px(p.total_visual_rows() - 1)).abs() < 0.5,
            "row tops must be re-derived after a measure-only set_size"
        );
        // The WIDER code measure really did change the geometry (fewer, wider rows).
        assert!(
            total_code < total_prose && rows0_code < rows0_prose,
            "a wider measure must yield fewer wrapped rows: {total_prose} -> {total_code}"
        );
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
    }

    /// The LIVE held-arrow seam, pipeline-side: `App::sync_view` pushes a
    /// CURSOR-ONLY `ViewState` per OS auto-repeat (same text, same zoom — the
    /// reshape short-circuit skips all shaping). Walk the caret down a wrapped
    /// markdown doc exactly that way and assert, after EVERY push, that nothing the
    /// skip left behind is stale: no reshape ran, the pipeline mirrors the pushed
    /// cursor, the caret spring TARGET equals the position computed from a
    /// freshly-invalidated row geometry (warm caches == cold truth), and the
    /// cursor's visual row (the scroll-follow input) strictly descends. A cursor
    /// that advances internally while the RENDERED caret/scroll reads stale would
    /// fail here — the live "held-down stuck" de-sync shape that captures (which
    /// rebuild fully) can never see. GPU-backed; skips with no adapter.
    #[test]
    fn held_cursor_only_view_pushes_stay_fresh() {
        use crate::actions::LayoutOracle;
        // The walk assumes STABLE wrap geometry; hold the global test locks so a
        // parallel theme/page mutator can't reshape the document mid-walk.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::test_lock();
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping held_cursor_only_view_pushes_stay_fresh: no wgpu adapter");
            return;
        };
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/CLAUDE.md"))
            .expect("CLAUDE.md present at crate root");
        let mut v = view(&text, 0, 0);
        v.is_markdown = true;
        v.held = true;
        p.set_view(&v);
        let reshapes = p.reshape_count;
        let mut goal: Option<f32> = None;
        let mut prev_row = p.visual_row_of(0, 0);
        let (mut line, mut col) = (0usize, 0usize);
        for step in 0..200 {
            // One held C-n, exactly as actions::motion::vertical_motion steps it.
            let gx = goal.unwrap_or_else(|| p.visual_x_of(line, col));
            goal = Some(gx);
            let (nl, nc) = p.visual_line_down(line, col, gx);
            assert_ne!((nl, nc), (line, col), "stuck at ({line},{col}) on step {step}");
            (line, col) = (nl, nc);
            // The cursor-only re-push sync_view does on the auto-repeat.
            let mut vs = view(&text, line, col);
            vs.is_markdown = true;
            vs.held = true;
            p.set_view(&vs);
            assert_eq!(p.reshape_count, reshapes, "a cursor-only push must not reshape");
            assert_eq!(
                (p.cursor_line, p.cursor_col),
                (line, col),
                "pipeline cursor mirror lagged the push on step {step}"
            );
            // WARM caret target (what the frame will draw toward) vs COLD truth.
            let warm_xy = p.caret_target_xy();
            let warm_row = p.visual_row_of(line, col);
            let (_, warm_target, _, _) = {
                let s = p.caret_snapshot();
                (s.0, s.1, s.2, s.3)
            };
            p.row_geom.invalidate();
            let cold_xy = p.caret_target_xy();
            let cold_row = p.visual_row_of(line, col);
            assert!(
                (warm_xy.0 - cold_xy.0).abs() < 0.01 && (warm_xy.1 - cold_xy.1).abs() < 0.01,
                "caret target from warm caches diverged from cold truth on step {step}: \
                 warm {warm_xy:?} cold {cold_xy:?}"
            );
            assert_eq!(warm_row, cold_row, "visual_row_of diverged on step {step}");
            assert!(
                (warm_target.0 - warm_xy.0).abs() < 0.01
                    && (warm_target.1 - warm_xy.1).abs() < 0.01,
                "the spring target was not re-aimed at the pushed cursor on step {step}"
            );
            // WYSIWYG v1.1 exception (documented, not a regression): a line
            // carrying `**bold**`/`*italic*` markup can WRAP into a different
            // number of visual rows depending on whether the caret is currently
            // ON it (real advances, revealed) or has just LEFT it (near-zero
            // advances, concealed) — the accepted "line re-wraps on reveal" cost
            // (CLAUDE.md's WYSIWYG section) cascades to every row index below it.
            // Stepping DOWN off such a line can therefore hold the global row
            // flat for exactly this one step (the line just shed a row as it
            // re-concealed) — never regress, only plateau. A full 500-step sweep
            // of this very file confirms no ACTUAL decrease ever occurs, only
            // occasional equality, so `>=` (not the old strict `>`) is the
            // correct invariant post-WYSIWYG; strict monotonicity is preserved
            // pre-WYSIWYG (color-only conceal never changed wrap counts).
            assert!(
                warm_row >= prev_row,
                "the scroll-follow row regressed on step {step}: {prev_row} -> {warm_row}"
            );
            prev_row = warm_row;
        }
    }

