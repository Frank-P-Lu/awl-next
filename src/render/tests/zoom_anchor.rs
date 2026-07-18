//! ZOOM ANCHORING law tests — the pure-owner seam
//! ([`TextPipeline::zoom_anchor_scroll`] + [`TextPipeline::char_screen_top`]).
//!
//! The LIVE app anchors a keyboard ⌘± on the CARET and a wheel ⌘-scroll on the
//! POINTER: the anchored document point holds its SCREEN position across the zoom
//! step instead of the viewport drifting from the top. The transition itself is
//! live-only (it needs the OLD-zoom capture + the deferred reflow), so these tests
//! drive the exact same MATH the app does — reshape a real pipeline from zoom A to
//! zoom B and assert the owner keeps the anchor point fixed. Every case skips on a
//! GPU-less machine (`headless_pipeline() -> None`). The critical case is the
//! HEADING-heavy fixture: variable row heights are where a naive linear scale of the
//! old geometry (or the old top-anchored scroll) drifts, and the owner does not.

use super::super::*;
use super::{headless_pipeline, view, view_md};

/// A tall body-text fixture: `n` short lines that never wrap, so every visual row is
/// a uniform `LINE_HEIGHT`.
fn body(n: usize) -> String {
    (0..n).map(|i| format!("line {i:02} body text")).collect::<Vec<_>>().join("\n")
}

/// A heading-heavy markdown fixture: a `#`/`##` heading every few lines, so the
/// visual-row table has NON-uniform heights (the case naive math fails).
fn headings(n: usize) -> String {
    (0..n)
        .map(|i| match i % 4 {
            0 => format!("# Heading {i:02}"),
            2 => format!("## Sub {i:02}"),
            _ => format!("body line {i:02} here"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Reshape `p` to `zoom` (+ `scroll`) with the given (markdown?) fixture, returning
/// the pipeline shaped at that zoom. Mirrors the live `set_view` reshape the deferred
/// zoom reflow performs.
fn reshape(p: &mut TextPipeline, text: &str, cl: usize, cc: usize, zoom: f32, scroll: usize, md: bool) {
    let mut v = if md { view_md(text, cl, cc) } else { view(text, cl, cc) };
    v.zoom = zoom;
    v.scroll_lines = scroll;
    p.set_view(&v);
}

/// Edge-to-edge (page OFF) so short lines never wrap and the row grid is clean; the
/// two `serial()` takes bar a concurrent page/theme flip from shifting geometry
/// between the two reshapes (CLAUDE.md's flake note; reentrant, so double-take is fine).
fn with_page_off(f: impl FnOnce()) {
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let was_on = crate::page::page_on();
    let was_measure = crate::page::measure();
    crate::page::set_page_on(false);
    f();
    crate::page::set_page_on(was_on);
    crate::page::set_measure(was_measure);
}

/// KEYBOARD path, UNIFORM rows: the caret's screen y is invariant across a zoom-in.
/// With no anchoring the caret would drift proportionally to its distance from the
/// top; the owner holds it within a sub-row tolerance.
#[test]
fn caret_screen_y_holds_across_zoom_in_uniform() {
    with_page_off(|| {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skip caret_screen_y_holds_across_zoom_in_uniform: no wgpu adapter");
            return;
        };
        let text = body(50);
        let (cl, cc) = (30, 0);
        // Zoom 1.0, caret mid-viewport at scroll 20.
        reshape(&mut p, &text, cl, cc, 1.0, 20, false);
        let caret_y_old = p.char_screen_top(cl, cc, 20);
        assert!(caret_y_old > TEXT_TOP && caret_y_old < 800.0, "caret must start on screen: {caret_y_old}");
        // Reshape to zoom 1.2, then anchor.
        reshape(&mut p, &text, cl, cc, 1.2, 20, false);
        let new_scroll = p.zoom_anchor_scroll(cl, cc, caret_y_old, 800.0);
        let caret_y_new = p.char_screen_top(cl, cc, new_scroll);
        let tol = p.row_height_px(p.visual_row_of(cl, cc));
        assert!(
            (caret_y_new - caret_y_old).abs() < tol,
            "anchored caret drifted {} px (tol {tol}); old {caret_y_old} new {caret_y_new}",
            (caret_y_new - caret_y_old).abs()
        );
        // The anchor did REAL work: the naive top-anchored scroll (scroll unchanged)
        // would leave the caret far off its old y.
        let caret_y_naive = p.char_screen_top(cl, cc, 20);
        assert!(
            (caret_y_naive - caret_y_old).abs() > tol,
            "test is vacuous — top-anchor drift {} within tol {tol}",
            (caret_y_naive - caret_y_old).abs()
        );
    });
}

/// KEYBOARD path, HEADING-heavy (VARIABLE rows): the caret's screen y is invariant
/// across a zoom step even when the rows above it are non-uniform heading heights —
/// the case a linear scale of the old geometry gets wrong.
#[test]
fn caret_screen_y_holds_across_zoom_variable_rows() {
    with_page_off(|| {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skip caret_screen_y_holds_across_zoom_variable_rows: no wgpu adapter");
            return;
        };
        let text = headings(60);
        let (cl, cc) = (33, 0);
        reshape(&mut p, &text, cl, cc, 1.0, 18, true);
        let caret_y_old = p.char_screen_top(cl, cc, 18);
        assert!(caret_y_old > TEXT_TOP && caret_y_old < 800.0, "caret on screen: {caret_y_old}");
        reshape(&mut p, &text, cl, cc, 1.3, 18, true);
        let new_scroll = p.zoom_anchor_scroll(cl, cc, caret_y_old, 800.0);
        let caret_y_new = p.char_screen_top(cl, cc, new_scroll);
        // Tolerance = the tallest row that could sit at the boundary (the anchor is
        // quantised to whole variable-height rows).
        let tol = p.row_height_px(p.visual_row_of(cl, cc)).max(p.row_height_px(new_scroll));
        assert!(
            (caret_y_new - caret_y_old).abs() < tol,
            "variable-row anchor drift {} px (tol {tol}); old {caret_y_old} new {caret_y_new}",
            (caret_y_new - caret_y_old).abs()
        );
    });
}

/// ZoomOut ROUND-TRIP returns the scroll to (≈) where it started: zoom in anchoring
/// the caret, then zoom back out anchoring the caret at its held y, and the scroll
/// lands within one row of the original.
#[test]
fn zoom_round_trip_returns_scroll() {
    with_page_off(|| {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skip zoom_round_trip_returns_scroll: no wgpu adapter");
            return;
        };
        let text = headings(60);
        let (cl, cc) = (28, 0);
        let scroll0 = 16usize;
        reshape(&mut p, &text, cl, cc, 1.0, scroll0, true);
        let caret_y0 = p.char_screen_top(cl, cc, scroll0);
        // In to 1.3.
        reshape(&mut p, &text, cl, cc, 1.3, scroll0, true);
        let scroll1 = p.zoom_anchor_scroll(cl, cc, caret_y0, 800.0);
        let caret_y1 = p.char_screen_top(cl, cc, scroll1);
        // Back out to 1.0, anchoring the caret at the y it now holds.
        reshape(&mut p, &text, cl, cc, 1.0, scroll1, true);
        let scroll2 = p.zoom_anchor_scroll(cl, cc, caret_y1, 800.0);
        assert!(
            scroll2.abs_diff(scroll0) <= 1,
            "round-trip scroll {scroll2} vs start {scroll0} (>1 row apart)"
        );
    });
}

/// CARET-OFF-SCREEN fallback: when the caret is scrolled far off-screen the app
/// anchors the VIEWPORT CENTRE instead. This drives the pieces the app composes —
/// the off-screen predicate (`char_screen_top` below the top) and a centre anchor —
/// and asserts the centre document point is invariant AND the view did NOT jump to
/// the caret.
#[test]
fn off_screen_caret_anchors_viewport_center() {
    with_page_off(|| {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skip off_screen_caret_anchors_viewport_center: no wgpu adapter");
            return;
        };
        let text = body(80);
        let (cl, cc) = (2, 0); // caret near the top of the document
        let scroll = 40usize; // scrolled far below it
        reshape(&mut p, &text, cl, cc, 1.0, scroll, false);
        // Predicate the app's fallback branch reads: the caret is off-screen (above).
        let caret_y = p.char_screen_top(cl, cc, scroll);
        assert!(caret_y < TEXT_TOP, "caret should be off-screen above: {caret_y}");
        // Fallback: anchor whatever sits at the viewport centre.
        let center_y = (TEXT_TOP + 800.0) * 0.5;
        let (center_x, _) = (600.0f32, center_y);
        let (aline, acol) = p.hit_test(center_x, center_y, scroll);
        let center_top_old = p.char_screen_top(aline, acol, scroll);
        reshape(&mut p, &text, cl, cc, 1.25, scroll, false);
        let new_scroll = p.zoom_anchor_scroll(aline, acol, center_y, 800.0);
        let center_top_new = p.char_screen_top(aline, acol, new_scroll);
        let tol = p.row_height_px(p.visual_row_of(aline, acol));
        // The centre document point holds its screen y (within a row).
        assert!(
            (center_top_new - center_top_old).abs() < 2.0 * tol,
            "centre anchor drift {} px (tol {}); old {center_top_old} new {center_top_new}",
            (center_top_new - center_top_old).abs(),
            2.0 * tol
        );
        // The view did NOT jump to the caret: the caret is still off-screen above.
        let caret_y_after = p.char_screen_top(cl, cc, new_scroll);
        assert!(
            caret_y_after < TEXT_TOP,
            "view jumped toward the caret (now at {caret_y_after}); should stay put"
        );
    });
}

/// POINTER path (the wheel's owner is the SAME `zoom_anchor_scroll`): a synthetic
/// pointer anchor keeps the document point under the cursor at the pointer's screen
/// y across a zoom step.
#[test]
fn pointer_anchor_holds_doc_point_under_cursor() {
    with_page_off(|| {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skip pointer_anchor_holds_doc_point_under_cursor: no wgpu adapter");
            return;
        };
        let text = headings(60);
        let scroll = 20usize;
        reshape(&mut p, &text, 0, 0, 1.0, scroll, true);
        let pointer_y = 300.0f32;
        let (aline, acol) = p.hit_test(600.0, pointer_y, scroll);
        reshape(&mut p, &text, 0, 0, 1.35, scroll, true);
        let new_scroll = p.zoom_anchor_scroll(aline, acol, pointer_y, 800.0);
        let anchor_top_new = p.char_screen_top(aline, acol, new_scroll);
        let tol = p.row_height_px(p.visual_row_of(aline, acol));
        assert!(
            (anchor_top_new - pointer_y).abs() < tol,
            "pointer anchor row-top {anchor_top_new} vs pointer {pointer_y} (>1 row apart, tol {tol})"
        );
    });
}

/// BOUNDARY clamps: zooming OUT at the document top can't scroll above 0 (the anchor
/// yields to scroll 0), and zooming IN with the caret near the bottom clamps at
/// `max_scroll_rows` (never past the document end).
#[test]
fn zoom_anchor_clamps_at_document_bounds() {
    with_page_off(|| {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skip zoom_anchor_clamps_at_document_bounds: no wgpu adapter");
            return;
        };
        let text = body(80);
        // TOP: caret on line 1, scroll 0, zoom OUT. The anchor would want scroll < 0.
        let (cl, cc) = (1, 0);
        reshape(&mut p, &text, cl, cc, 1.0, 0, false);
        let caret_y = p.char_screen_top(cl, cc, 0);
        reshape(&mut p, &text, cl, cc, 0.7, 0, false);
        let top_scroll = p.zoom_anchor_scroll(cl, cc, caret_y, 800.0);
        assert_eq!(top_scroll, 0, "zoom-out at the top must clamp scroll to 0");

        // BOTTOM: caret on the last line, scroll at max, zoom IN. The taller document
        // pushes the anchor target past the end, so it clamps at max_scroll_rows.
        let last = 79usize;
        reshape(&mut p, &text, last, 0, 1.0, 0, false);
        let max0 = p.max_scroll_rows(800.0);
        reshape(&mut p, &text, last, 0, 1.0, max0, false);
        let caret_y_bot = p.char_screen_top(last, 0, max0);
        reshape(&mut p, &text, last, 0, 1.5, max0, false);
        let bot_scroll = p.zoom_anchor_scroll(last, 0, caret_y_bot, 800.0);
        let max1 = p.max_scroll_rows(800.0);
        assert_eq!(bot_scroll, max1, "zoom-in at the bottom must clamp at max_scroll_rows");
    });
}
