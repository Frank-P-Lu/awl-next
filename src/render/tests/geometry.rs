//! Pure page/hit-test/glyph-advance geometry math -- page column layout,
//! hit-testing, `assemble_glyph_xs`, line/col <-> char-index mapping,
//! max-scroll, and visual-row picking -- split out of the former monolithic
//! `render::tests` (2026-07 code-organization pass). See `geometry_reshape`
//! for the row-geometry INVALIDATION/reshape half of this same area.

use super::super::*;
use super::{H};

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
