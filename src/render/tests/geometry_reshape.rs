//! Row-geometry invalidation + reshape-on-change tests (set_size rewrap,
//! measure-only invalidation, row_geom generation bump, incremental vs full
//! shape agreement, typewriter-scroll centering/pin, and the CRLF buffer vs
//! pipeline line-count agreement) -- split out of the former monolithic
//! `render::tests` (2026-07 code-organization pass). See `geometry` for the
//! pure page/hit-test math half of this same area.

use super::super::*;
use super::{headless_pipeline, view};

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
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
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
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
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

/// LOCKOUT REGRESSION (bug, 2026-07-15): with the page widened past the window's
/// capacity the column COLLAPSES to the `PAGE_MIN_PAD` margins, and the resize
/// affordance must STILL arm at both drawn edges so the user can drag the width back
/// inward. Drives the REAL `page_resize_edge_at` (the same method hover, the ColResize
/// cursor, the press-arm, and the double-click reset all ride) through the live page
/// globals + adaptive `column_left` — the earlier `left <= PAGE_MIN_PAD + 1.0 → None`
/// guard returned `None` here and locked the user out. GPU-backed; skips with no
/// adapter. Holds both TEST_LOCKs like every page-global-mutating test.
#[test]
fn collapsed_page_still_arms_the_resize_affordance() {
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping collapsed_page_still_arms_the_resize_affordance: no wgpu adapter");
        return;
    };
    let was_on = crate::page::page_on();
    let was_measure = crate::page::measure();
    crate::page::set_page_on(true);
    // A narrow window with the measure pinned to MAX: the column can't fit, so it
    // collapses to the small pad on both sides (left == PAGE_MIN_PAD).
    p.set_size(600.0, 800.0);
    crate::page::set_measure(crate::page::MAX_MEASURE);
    let left = p.column_left();
    let width = p.column_width();
    assert!(
        left <= PAGE_MIN_PAD + 1.0,
        "fixture must actually collapse (left={left}) or it can't exercise the old guard",
    );
    // The pre-fix guard killed BOTH edges; both must now arm.
    assert_eq!(
        p.page_resize_edge_at(left),
        Some(ResizeEdge::Left),
        "collapsed left edge must arm the resize (lockout fix)",
    );
    assert_eq!(
        p.page_resize_edge_at(left + width),
        Some(ResizeEdge::Right),
        "collapsed right edge must arm the resize (lockout fix)",
    );
    assert!(p.page_resize_hover(left), "hover must report the collapsed edge too");
    // And a drag inward from the collapsed right edge narrows the measure below MAX.
    // The gesture anchors the OPPOSITE (left) edge at press time, exactly like the live
    // `begin_page_resize_if_hovering`; dragging the right edge 200px inward of its press
    // position must shrink the width, hence the measure.
    let narrowed = p.page_resize_measure_at(left + width - 200.0, ResizeEdge::Right, left);
    assert!(
        narrowed < crate::page::MAX_MEASURE,
        "dragging the collapsed edge inward must narrow the measure (got {narrowed})",
    );
    crate::page::set_page_on(was_on);
    crate::page::set_measure(was_measure);
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
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
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
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
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
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
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
fn typewriter_centers_the_cursor_row() {
    // Visual-row totals + scroll targets fold the page wrap globals; hold the
    // page lock so a parallel page write can't re-wrap the doc mid-test.
    let _g = crate::testlock::serial();
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
    let _g = crate::testlock::serial();
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
fn variable_height_scroll_reaches_the_last_row() {
    // Visual-row totals fold the page wrap globals; hold the page lock.
    let _g = crate::testlock::serial();
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
