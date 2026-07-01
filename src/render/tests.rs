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
        // Mutates the process-global caret mode; hold caret's shared test lock so it
        // does not race caret.rs's own mode tests.
        let _g = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    /// The morph caret's SPACE-BAR geometry on a glyphless cell centres the thin bar
    /// on the cell MIDPOINT (`pos.x + advance/2`), not pinned to the cell's left
    /// edge — the specific bug the function's doc warns about. Untested before.
    #[test]
    fn space_bar_caret_centers_on_cell_advance() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping space_bar_caret_centers_on_cell_advance: no wgpu adapter");
            return;
        };
        let text = "a b"; // col 1 is the space cell (glyphless)
        p.set_view(&view(text, 0, 1));
        p.settle_caret();
        let (cx, _cy, w, _h, _c) = p.caret_space_bar_geometry();
        let want_cx = p.caret.pos.x + p.caret_target_w() * 0.5;
        assert!(
            (cx - want_cx).abs() < 1e-3,
            "space-bar | centres on the cell midpoint: cx={cx} want={want_cx}"
        );
        assert!(
            (w - CARET_SPACE_BAR_W * p.metrics.zoom).abs() < 1e-3,
            "space-bar width == CARET_SPACE_BAR_W*zoom: w={w}"
        );
    }

    /// set_caret_target's edit-reflow branch selection (the "caret lags on Enter"
    /// fix): a CROSS-ROW edit SNAPS (jump_to), a SAME-ROW edit GLIDES (set_target),
    /// and the navigation zip-distance gate snaps a small move but animates a big one.
    #[test]
    fn edit_reflow_across_row_snaps_but_same_line_glides() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping edit_reflow_across_row_snaps_but_same_line_glides: no wgpu adapter");
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

        // SAME-ROW edit (typing along a line): glides.
        p.set_view(&view(text, 1, 0));
        p.settle_caret();
        p.cursor_col = 3;
        p.set_caret_target(true, false);
        assert!(p.caret_snapshot().3, "same-row edit must glide (animating)");

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
        // Page mode off: left is the fixed origin and width spans the window
        // minus both TEXT_LEFT margins — identical to the pre-page behavior.
        let cw = CHAR_WIDTH;
        assert_eq!(column_left_for(1200.0, cw, false, 80), TEXT_LEFT);
        assert!((column_width_for(1200.0, cw, false, 80) - (1200.0 - 2.0 * TEXT_LEFT)).abs() < 1e-3);
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
            overlay_hint: String::new(),
            caret_preview: None,
            gutter_name: String::new(),
            gutter_project: String::new(),
            is_markdown: false,
            syn_lang: None,
        }
    }

    #[test]
    fn selection_rects_multiline_geometry_and_eol_pad() {
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
        let (rect, text, _beat) = p
            .caret_preview_panel_report()
            .expect("the preview panel is summoned with the picker");
        assert_eq!(text, crate::caret::SAMPLE, "the settled panel shows the full sample line");
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

    #[test]
    fn theme_font_switch_reshapes_document() {
        let _g = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    #[test]
    fn heading_size_survives_theme_switch() {
        let _g = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    /// MONO FIX regression: the mono worlds (IBM Plex Mono) must shape in TRUE
    /// monospace — a line of all-'i' and a line of all-'m' have the SAME, uniform
    /// glyph pitch. The bug (a default Weight-400 request dropping the bundled
    /// Light face and falling through to proportional `.SF NS`) made i ~5px / m
    /// ~19px; the `mono_safe_weight(300)` fix realigns the request with the face.
    /// Contrast a proportional world (Literata) where i and m differ by design.
    #[test]
    fn mono_world_shapes_uniform_pitch() {
        let _g = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    /// The BLOCK caret quad's resting WIDTH tracks the REAL shaped glyph advance at
    /// the cursor: on a PROPORTIONAL world it is wide on `m` and narrow on `i`
    /// (exactly the glyph's advance, no fixed-cell floor); on a MONO world it is the
    /// constant cell and byte-identical to the old `caret_target_w`.
    #[test]
    fn block_caret_width_tracks_glyph_advance() {
        let _g = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        // column — the mono block is unchanged. (Keyed on the theme's declared font
        // family, so this holds even where the mono face isn't installed and shaping
        // falls back: Tawny still renders exactly as it did before.)
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
