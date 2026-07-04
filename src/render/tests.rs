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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _p = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
            overlay_bindings: Vec::new(),
            overlay_times: Vec::new(),
            overlay_selected: 0,
            overlay_scroll: 0,
            overlay_hint: String::new(),
        overlay_lens: Vec::new(),
        overlay_sections: Vec::new(),
            caret_preview: None,
            gutter_name: String::new(),
            gutter_project: String::new(),
            is_markdown: false,
            syn_lang: None,
            overlay_spell: None,
            notice: String::new(),
        }
    }

    #[test]
    fn selection_rects_multiline_geometry_and_eol_pad() {
        // Selection x geometry folds the page globals (text_left + wrap width);
        // hold the page lock so a parallel page write can't move it (page.rs:95-99).
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    #[test]
    fn oracle_visual_motion_follows_wrapped_rows() {
        // The visual-line LAYOUT ORACLE on the GPU pipeline: visual up/down step
        // through WRAPPED rows of one logical line and cross into adjacent logical
        // lines, all from the shaped geometry. (GPU-backed; skips with no adapter.)
        use crate::actions::LayoutOracle;
        // Soft-wrap geometry folds the page globals (column width); hold the page
        // lock so a parallel page write can't re-wrap the rows mid-test.
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        // Pin the default world (Tawny) so the ornament set is the shared defaults:
        // `---` → ❧, `***` → ⁂, `___` → ❦.
        theme::set_active(theme::DEFAULT_THEME);
        // Three DIFFERENT break syntaxes, each alone on its own line (blank-separated):
        // line 2 = `---`, line 4 = `***`, line 6 = `___`.
        let text = "intro\n\n---\n\n***\n\n___\n\nmore\n";

        // CARET OFF every break (line 0): all three ornaments draw, each the glyph its
        // OWN syntax picked — ❧, ⁂, ❦ in document order (⁂ is the three-star asterism
        // for the three asterisks). This is the whole feature: the mark tracks the type.
        let mut off = view(text, 0, 0);
        off.is_markdown = true;
        p.set_view(&off);
        let marks: Vec<char> = p.rule_marks().into_iter().map(|(_, c)| c).collect();
        assert_eq!(
            marks,
            vec!['❧', '⁂', '❦'],
            "--- ⁄ *** ⁄ ___ must pick ❧ ⁄ ⁂ ⁄ ❦ respectively: {marks:?}"
        );

        // REVEAL-ON-CURSOR still holds PER LINE: put the caret on the `***` line (4).
        // Its ornament yields (the raw *** reveal for editing) while the OTHER two
        // breaks keep their distinct ornaments — ❧ and ❦, the ⁂ dropped.
        let mut on_star = view(text, 4, 0);
        on_star.is_markdown = true;
        p.set_view(&on_star);
        let revealed: Vec<char> = p.rule_marks().into_iter().map(|(_, c)| c).collect();
        assert_eq!(
            revealed,
            vec!['❧', '❦'],
            "caret on the *** line suppresses only its ⁂; ❧ and ❦ remain: {revealed:?}"
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

        // CARET OFF every list line (on the trailing blank line 3): each bullet draws
        // its depth glyph • ◦ ▪ and its raw marker is concealed (transparent ink).
        let mut off = view(text, 3, 0);
        off.is_markdown = true;
        p.set_view(&off);
        assert_eq!(
            p.bullet_glyphs(),
            vec!['•', '◦', '▪'],
            "depth 0/1/2 => • ◦ ▪ regardless of the -,*,+ typed: {:?}",
            p.bullet_glyphs()
        );
        for li in 0..3 {
            assert!(
                p.bullet_marker_concealed(li),
                "caret off => the raw marker on line {li} is concealed"
            );
        }

        // CARET ON the middle bullet (line 1): its raw `*` REVEALS (editable) and no
        // glyph draws for it; the other two keep their • and ▪.
        let mut on = view(text, 1, 3);
        on.is_markdown = true;
        p.set_view(&on);
        assert_eq!(
            p.bullet_glyphs(),
            vec!['•', '▪'],
            "caret on the mid bullet suppresses only its ◦: {:?}",
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let (comments, strings) = p.wash_rects();
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
        let (c2, s2) = p.wash_rects();
        assert_eq!(p.reshape_count, reshapes + 1, "the edit reshapes once");
        assert_ne!(p.wash_cache_version(), Some(key), "an edit rebuilds the wash protos");
        assert_eq!((c2.len(), s2.len()), (1, 1));

        // PROSE (no syn_lang, not markdown): zero rects — byte-identical render.
        p.set_view(&view("plain prose here\n", 0, 0));
        let (c3, s3) = p.wash_rects();
        assert!(c3.is_empty() && s3.is_empty(), "prose buffers carry no washes");
    }

    /// WASH O(visible): on a TALL code doc the per-frame wash pass emits only the
    /// visible band's quads (proto cull) — never one per document line.
    #[test]
    fn wash_rects_cull_to_visible_band() {
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let (comments, _) = p.wash_rects();
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping markdown_fence_inherits_washes: no wgpu adapter");
            return;
        };
        let text = "prose before\n```rust\n// a calm fence note\nlet s = \"hi\";\n```\nprose after\n";
        let mut v = view(text, 0, 0);
        v.is_markdown = true;
        p.set_view(&v);
        let (comments, strings) = p.wash_rects();
        assert_eq!(comments.len(), 1, "the fence's prose comment washes: {comments:?}");
        assert_eq!(strings.len(), 1, "the fence's string washes: {strings:?}");

        // Markdown with NO fence: no washes at all (prose byte-identity).
        let mut v2 = view("# title\nplain prose paragraph\n", 0, 0);
        v2.is_markdown = true;
        p.set_view(&v2);
        let (c, s) = p.wash_rects();
        assert!(c.is_empty() && s.is_empty(), "fence-less markdown carries no washes");
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    fn heading_rows_are_taller_and_gated_to_markdown() {
        // The row-count assertion assumes NOTHING wraps, which folds the page
        // globals (column width); hold the page lock (page.rs:95-99).
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    fn variable_height_scroll_reaches_the_last_row() {
        // Visual-row totals fold the page wrap globals; hold the page lock.
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    fn focus_typewriter_centers_the_cursor_row() {
        // Visual-row totals + scroll targets fold the page wrap globals; hold the
        // page lock so a parallel page write can't re-wrap the doc mid-test.
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping focus_typewriter_centers_the_cursor_row: no wgpu adapter");
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
        // Focus OFF (minimal-adjust): only nudge enough to reveal the row near the
        // viewport BOTTOM — a SMALL scroll from the top.
        let minimal = p.scroll_to_show_row(row, 0, 800.0);
        // Focus ON (typewriter): CENTER the row — scroll much further down.
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
    fn focus_paragraph_colors_only_the_active_unit() {
        let _g = crate::focus::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping focus_paragraph_colors_only_the_active_unit: no wgpu adapter");
            return;
        };
        // Two paragraphs (lines 0-1) and (lines 3-4), split by a blank line 2.
        let text = "Para one a.\nPara one b.\n\nPara two a.\nPara two b.";
        crate::focus::set_mode(crate::focus::FocusMode::Paragraph);
        // Cursor in the SECOND paragraph (line 3).
        p.set_view(&view(text, 3, 2));
        p.settle_focus();
        // The active paragraph (lines 3,4) must carry explicit full-ink color spans;
        // the FIRST paragraph + the title line ride the dim default (no span). The
        // pipeline tracks exactly the lines it colored.
        let mut colored = p.focus_lines.clone();
        colored.sort_unstable();
        assert_eq!(
            colored,
            vec![3, 4],
            "only the cursor's paragraph lines should be full-ink; outside is dimmed"
        );
        // The reported active range matches the second paragraph.
        let (mode, range) = p.focus_report();
        assert_eq!(mode, "paragraph");
        let start = "Para one a.\nPara one b.\n\n".chars().count();
        assert_eq!(range, Some((start, text.chars().count())));
        // Turning focus OFF clears every colored line (all text returns to full ink).
        crate::focus::set_mode(crate::focus::FocusMode::Off);
        p.set_view(&view(text, 3, 2));
        assert!(
            p.focus_lines.is_empty(),
            "focus off must clear all per-line color spans"
        );
    }

    #[test]
    fn focus_in_unit_edit_does_not_rekick_fade() {
        let _g = crate::focus::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping focus_in_unit_edit_does_not_rekick_fade: no wgpu adapter");
            return;
        };
        crate::focus::set_mode(crate::focus::FocusMode::Paragraph);
        // Settle on the SECOND paragraph (the first application snaps; settle pins it).
        let text = "Para one a.\nPara one b.\n\nPara two a.\nPara two b.";
        p.set_view(&view(text, 3, 2));
        p.settle_focus();
        assert_eq!(p.focus_t, 1.0, "first application snaps settled");
        assert_eq!(p.focus_prev, None, "nothing fading out after the snap");

        // TYPE inside the same paragraph: line 3 grows by one char, so the active
        // unit's END index shifts (+1) even though the cursor never left the unit.
        // This is the per-keystroke flash trigger; an edit must NOT re-kick the fade.
        let edited = "Para one a.\nPara one b.\n\nPaxra two a.\nPara two b.";
        let mut typed = view(edited, 3, 3);
        typed.is_edit_move = true;
        p.set_view(&typed);
        assert_eq!(
            p.focus_t, 1.0,
            "an in-unit edit must leave the focus fade settled (no per-keystroke flash)"
        );
        assert_eq!(
            p.focus_prev, None,
            "an in-unit edit must not start a crossfade-out of the same unit"
        );
        // The range still tracks the (now longer) paragraph at full ink.
        let start = "Para one a.\nPara one b.\n\n".chars().count();
        assert_eq!(p.focus_report().1, Some((start, edited.chars().count())));

        // A genuine cursor MOVE into a DIFFERENT (disjoint) paragraph MUST still kick
        // the calm crossfade: the prior unit fades out, the new fade restarts at 0.
        let prev_range = p.focus_cur;
        p.set_view(&view(edited, 0, 0)); // is_edit_move = false (pure navigation)
        assert_eq!(
            p.focus_t, 0.0,
            "moving to a different unit must restart the crossfade"
        );
        assert_eq!(
            p.focus_prev, prev_range,
            "the just-left unit fades out as focus_prev"
        );
        crate::focus::set_mode(crate::focus::FocusMode::Off);
    }

    /// CRASH REGRESSION (user report + trace): focus mode held stale line indices
    /// in `focus_lines` from a PRIOR coloring pass; a select-all + type (or any big
    /// delete) shrinks the buffer, and the next `clear_focus_spans` indexed
    /// `self.buffer.lines[li]` RAW with one of those now-out-of-range indices —
    /// "len is 1 but the index is 757" inside `key_down`, an unwinding-unsafe
    /// panic → hard process abort. This drives the EXACT shape live-App code
    /// takes: focus on, a multi-paragraph doc so `focus_lines` colors HIGH indices,
    /// then a same-frame edit that collapses the whole document to one short line
    /// (the select-all + type shape) with `is_edit_move = true` (so the fade logic
    /// takes the silent-adopt path, not the jump/kick path — matching a real
    /// keystroke, not a cursor move).
    #[test]
    fn focus_survives_buffer_shrink_below_colored_lines() {
        let _g = crate::focus::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping focus_survives_buffer_shrink_below_colored_lines: no wgpu adapter");
            return;
        };
        crate::focus::set_mode(crate::focus::FocusMode::Paragraph);
        // Five paragraphs; cursor lands in the LAST one (lines 8, 9), so
        // `focus_lines` colors high indices that will be well past the end of the
        // shrunk buffer.
        let text = "Para one.\n\nPara two.\n\nPara three.\n\nPara four.\n\nPara five a.\nPara five b.";
        p.set_view(&view(text, 8, 2));
        assert!(
            p.focus_lines.contains(&9),
            "sanity: the active paragraph's high line indices are colored: {:?}",
            p.focus_lines
        );

        // SELECT-ALL + TYPE: the whole document collapses to a single short line,
        // as one atomic edit (mirrors the real apply_core path: delete-selection +
        // insert-char land as one text replacement, is_edit_move = true).
        let mut shrunk = view("x", 0, 1);
        shrunk.is_edit_move = true;
        p.set_view(&shrunk); // must not panic

        // The stale high-index lines are gone; focus re-colors within the new,
        // much shorter document.
        for &li in &p.focus_lines {
            assert!(
                li < p.line_count(),
                "focus_lines must never outlive the buffer: {li} >= {}",
                p.line_count()
            );
        }

        // A second edit on the now-tiny buffer must also stay clean (the cleared
        // stale lines don't leave the pipeline in a half-updated state).
        let mut typed_more = view("xy", 0, 2);
        typed_more.is_edit_move = true;
        p.set_view(&typed_more);
        crate::focus::set_mode(crate::focus::FocusMode::Off);
    }

    /// DIRECT UNIT at the exact panic site: `focus_lines` holding an index past the
    /// buffer's end must not panic `clear_focus_spans` — it must silently skip the
    /// vanished line (mirrors the `.get_mut(li)` guard already beside it).
    #[test]
    fn clear_focus_spans_skips_out_of_range_stale_index() {
        let _g = crate::focus::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping clear_focus_spans_skips_out_of_range_stale_index: no wgpu adapter");
            return;
        };
        p.set_view(&view("only one line", 0, 0));
        assert_eq!(p.line_count(), 1);
        // Plant a stale index far beyond the buffer's single line, as if a prior
        // coloring pass ran against a much larger document.
        p.focus_lines = vec![0, 999];
        p.clear_focus_spans(); // must not panic
        assert!(
            p.focus_lines.is_empty(),
            "clear_focus_spans always drains focus_lines, in-range or not"
        );
    }

    #[test]
    fn theme_font_switch_reshapes_document() {
        // The caret-x reads below fold BOTH globals: the theme font (the shaped
        // advances) AND the page state (`column_width()` folds `page_on()` /
        // `measure()` — geometry.rs — into the wrap width + text_left every x is
        // measured from). Other tests flip the page globals under page::TEST_LOCK
        // (measure 15/40/50…), so reading them here with only the theme lock raced
        // a parallel page write — the historical parallel-run flake of this very
        // test. Hold both, in the suite-wide theme → page order (page.rs:95-99).
        // The caret x is also ANCHOR-keyed (Morph shifts one cell back, and with no
        // override the mode DEFAULTS off the active theme's font — proportional
        // Gumtree would flip it to Morph mid-test); hold the caret lock and pin
        // BLOCK so the x reads stay on the cursor cell across the world switches.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

        // Switching to a SAME-font world (Quokka and Kingfisher are both IBM Plex
        // Sans) need not reshape: the document is already shaped in that family.
        theme::set_active_by_name("Quokka").unwrap();
        p.sync_theme();
        let n = p.reshape_count;
        theme::set_active_by_name("Kingfisher").unwrap(); // also IBM Plex Sans
        p.sync_theme();
        assert_eq!(
            p.reshape_count, n,
            "a same-font theme switch must NOT reshape the document"
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        // font change is visible via `needs_font_reshape`.
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
            p.needs_font_reshape(),
            "the deferred font change is pending (Quokka is IBM Plex Sans)"
        );

        // SETTLE: the one deferred reshape lands. Exactly one reshape, and the
        // shaped state is identical to the synchronous `sync_theme` route.
        p.sync_theme_font();
        assert_eq!(p.reshape_count, n + 1, "the settle pays exactly ONE reshape");
        assert_eq!(p.shaped_font, "IBM Plex Sans");
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
        assert!(p.needs_font_reshape(), "a deferral is pending toward EB Garamond");
        let m = p.reshape_count;
        theme::set_active_by_name("Quokka").unwrap(); // the world the picker opened on
        p.sync_theme(); // retint_theme_now: full, synchronous
        assert_eq!(p.reshape_count, m, "reverting to the shaped face reshapes nothing");
        p.sync_theme_font(); // the stray late fire
        assert_eq!(
            p.reshape_count, m,
            "a stray deferred reshape after the revert must be a strict no-op"
        );
        assert_eq!(p.shaped_font, "IBM Plex Sans");

        // Restore the default world so other tests see a clean global.
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// PER-WORLD CODE MONO across SHARED-DISPLAY worlds: `sync_theme` compares the
    /// EFFECTIVE shaped face (`doc_family` — the world's mono on a CODE buffer,
    /// else its display font; render.rs), so on a code buffer a switch between two
    /// worlds sharing ONE display sans but naming DIFFERENT monos (Quokka →
    /// Kingfisher: both IBM Plex Sans; IBM Plex Mono vs JetBrains Mono) MUST
    /// reshape and retrack `shaped_font` — while two worlds sharing the MONO
    /// (Kingfisher → Currawong, both JetBrains Mono) must NOT. The PROSE half of
    /// the same compare (a
    /// shared display font skips the reshape) is pinned by
    /// `theme_font_switch_reshapes_document` next door; this is the code half.
    #[test]
    fn code_mono_switch_reshapes_across_shared_display_worlds() {
        // Shaping folds the theme font AND the page wrap globals; hold both locks
        // (theme → page order, page.rs:95-99) so a parallel mutator can't flip
        // either between the reshape-count reads.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping code_mono_switch_reshapes_across_shared_display_worlds: no wgpu adapter");
            return;
        };

        // A CODE buffer on Quokka shapes in the world's mono companion.
        theme::set_active_by_name("Quokka").unwrap();
        p.sync_theme();
        assert_eq!(theme::active().font, "IBM Plex Sans");
        assert_eq!(theme::active().mono, "IBM Plex Mono");
        let mut code = view("fn main() { let x = 1; }", 0, 0);
        code.syn_lang = Some(crate::syntax::Lang::Rust);
        p.set_view(&code);
        assert_eq!(
            p.shaped_font, "IBM Plex Mono",
            "a code buffer shapes in the world's mono, not its display sans"
        );
        let n = p.reshape_count;

        // Quokka → Kingfisher: the SAME display sans, a DIFFERENT mono. The
        // display-font compare alone would skip; the effective-face compare must
        // see the mono change and reshape the code buffer.
        theme::set_active_by_name("Kingfisher").unwrap();
        p.sync_theme();
        assert_eq!(theme::active().font, "IBM Plex Sans", "the display sans is shared");
        assert_eq!(theme::active().mono, "JetBrains Mono");
        assert!(
            p.reshape_count > n,
            "a mono change must reshape a code buffer even when the display font is shared"
        );
        assert_eq!(
            p.shaped_font, "JetBrains Mono",
            "shaped_font tracks the NEW mono after the switch"
        );

        // Kingfisher → Currawong: DIFFERENT display faces (IBM Plex Sans vs
        // JetBrains Mono) but the SAME code mono — the converse case. A prose
        // buffer would reshape here; the code buffer is already shaped in the
        // shared mono, so it must NOT (no reshape, shaped_font unchanged).
        let m = p.reshape_count;
        theme::set_active_by_name("Currawong").unwrap();
        p.sync_theme();
        assert_ne!(
            theme::active().font,
            "IBM Plex Sans",
            "Currawong's display face differs from Kingfisher's"
        );
        assert_eq!(theme::active().mono, "JetBrains Mono", "Currawong shares Kingfisher's mono");
        assert_eq!(
            p.reshape_count, m,
            "two worlds sharing a mono must NOT reshape a code buffer"
        );
        assert_eq!(p.shaped_font, "JetBrains Mono");

        // Restore the default world so other tests see a clean global.
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    #[test]
    fn heading_size_survives_theme_switch() {
        // Shaping folds the theme font AND the page wrap globals; hold both
        // (theme → page order, page.rs:95-99).
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    /// (d) selection (composited over `base_100` at its authored alpha) is a
    ///     QUIET highlight: ΔL in [0.05, 0.35] (measured 0.086–0.231) — visible
    ///     enough to see, never so opaque it reads as a solid paint fill;
    ///     redmean vs `base_100` ≥ 150 (measured worst 204.3, Undertow) so it is
    ///     never accidentally near-invisible.
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

            // (d) Selection is a quiet, never-invisible highlight.
            let sel_opaque = theme::Srgb::rgb(th.selection.r, th.selection.g, th.selection.b);
            let svb = redmean(sel_opaque, th.base_100);
            assert!(svb >= 150.0, "{}: selection vs base_100 redmean {svb:.1} < 150", th.name);
            let eff = composite(th.selection, th.base_100);
            let dl = (eff.to_hsl().2 - l_bg).abs();
            assert!(
                (0.05..=0.35).contains(&dl),
                "{}: selection composited ΔL {dl:.3} outside quiet-highlight band [0.05, 0.35]",
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
    /// at the FULL content ink (decision 2 made render-real), the focus
    /// `color_override` still wins uniformly, and a commented-out statement keeps
    /// the muted grey.
    #[test]
    fn syn_attrs_comment_tiers_and_focus_override() {
        use crate::syntax::SynKind;
        let _g = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        theme::set_active_by_name("Tawny").unwrap();
        let base = Attrs::new();
        let th = theme::active();
        assert_eq!(
            syn_attrs(&base, SynKind::Comment, None).color_opt,
            Some(th.base_content.to_glyphon()),
            "prose comment shapes at FULL content ink"
        );
        assert_eq!(
            syn_attrs(&base, SynKind::CommentCode, None).color_opt,
            Some(th.muted.to_glyphon()),
            "commented-out code keeps the muted grey"
        );
        // The FOCUS override replaces any role color uniformly (the existing seam).
        let focus = glyphon::Color::rgb(1, 2, 3);
        for k in [SynKind::Comment, SynKind::CommentCode, SynKind::Str,
                  SynKind::Constant, SynKind::Definition] {
            assert_eq!(syn_attrs(&base, k, Some(focus)).color_opt, Some(focus));
        }
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    /// CRLF LINE-MODEL AGREEMENT (the render half): on a Windows-ended document
    /// ("a\r\nb\r\nc") the [`Buffer`] (ropey: CRLF is ONE line break) and the
    /// pipeline (splits the pushed text on '\n' — text.rs) must agree on the
    /// LOGICAL LINE COUNT, or every line-indexed seam between them (cursor
    /// mirroring, squiggle line lookup, scroll follow) is off.
    ///
    /// CHARACTERIZES THE CURRENT DIVERGENCE (do not "fix" it here — CRLF handling
    /// belongs to the LOADING seam, which is characterized buffer-side): the two
    /// models DO agree on the count (3), but the pipeline's `split('\n')` RETAINS
    /// the '\r' at the end of every non-final line, so each shaped line carries a
    /// PHANTOM trailing column ("a\r" = 2 chars → 3 x-boundaries where the buffer
    /// line's content is the 1-char "a"). End-of-line caret/selection geometry on
    /// a CRLF file therefore includes one extra (usually zero-width) cell.
    #[test]
    fn crlf_buffer_and_pipeline_line_models_agree_on_count() {
        use crate::buffer::Buffer;
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping crlf_buffer_and_pipeline_line_models_agree_on_count: no wgpu adapter");
            return;
        };
        let text = "a\r\nb\r\nc";
        let buf = Buffer::from_str(text);
        assert_eq!(buf.line_count(), 3, "ropey treats CRLF as a single line break");
        // The pipeline shapes the SAME text the live sync pushes (buffer.text()
        // round-trips the CRs verbatim).
        assert_eq!(buf.text(), text, "the rope preserves the \\r bytes");
        p.set_view(&view(&buf.text(), 0, 0));
        assert_eq!(
            p.line_count(),
            buf.line_count(),
            "buffer and pipeline must agree on the logical line count of a CRLF doc"
        );
        // THE DIVERGENCE, pinned: the '\r' rides into each shaped line as a
        // phantom trailing char column (2 chars on line 0, not 1).
        assert_eq!(
            p.buffer.lines[0].text(),
            "a\r",
            "current behavior: the pipeline line retains the CR (phantom column)"
        );
        assert_eq!(
            p.line_glyph_xs(0).len(),
            3,
            "current behavior: 2 chars ('a' + the CR) => 3 x-boundaries on line 0"
        );
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        // The walk assumes STABLE wrap geometry + focus-off coloring; hold the
        // global test locks so a parallel theme/page/focus mutator can't reshape
        // the document mid-walk.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _f = crate::focus::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
            assert!(
                warm_row > prev_row,
                "the scroll-follow row did not descend on step {step}: {prev_row} -> {warm_row}"
            );
            prev_row = warm_row;
        }
    }
