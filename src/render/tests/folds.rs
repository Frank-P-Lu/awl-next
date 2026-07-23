//! FOLD RENDER LAW: a collapsed section's hidden lines are dropped from the shaped
//! text, so they contribute ZERO visual rows (and zero height) — the row simply is
//! not laid out. Driven through the SAME `fold::apply_to_view` seam the live
//! `sync_view` and the headless capture use, then shaped by a real headless
//! pipeline so the geometry is the true one.

use super::{headless_pipeline, view_md};
use crate::render::FoldTail;
use std::collections::BTreeSet;

// Two sibling sections, no soft-wrap:
//   0 # A / 1 a1 / 2 a2 / 3 # B / 4 b1
const DOC: &str = "# A\na1\na2\n# B\nb1";

/// Fold the given heading lines of `DOC` and return the `(hidden mask, tails)` the
/// live `sync_view` / capture builders feed [`crate::fold::apply_to_view`].
fn fold(headings: &[usize]) -> (Vec<bool>, Vec<(usize, usize)>) {
    let levels = crate::fold::heading_levels(DOC, true);
    let folds: BTreeSet<usize> = headings.iter().copied().collect();
    (
        crate::fold::hidden_lines(&levels, &folds),
        crate::fold::fold_tails(&levels, &folds),
    )
}

#[test]
fn a_folded_section_contributes_zero_visual_rows() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("no GPU adapter; skipping fold render law");
        return;
    };

    // UNFOLDED: all five logical lines shape to five visual rows (no wrap).
    let unfolded = view_md(DOC, 0, 0);
    p.set_view(&unfolded);
    let rows_unfolded = p.total_visual_rows();
    assert_eq!(rows_unfolded, 5, "five lines, five visual rows unfolded");

    // FOLD # A (line 0): its section is lines 1..=2 (a1, a2). Feed the hidden mask
    // through the shared fold seam — exactly what the app/capture builders do.
    let (hidden, tails) = fold(&[0]);
    let mut folded = view_md(DOC, 0, 0);
    crate::fold::apply_to_view(&mut folded, &hidden, &tails);
    // The two hidden lines are gone from the shaped text (so they cannot lay out).
    assert_eq!(folded.text, "# A\n# B\nb1");
    p.set_view(&folded);
    let rows_folded = p.total_visual_rows();
    assert_eq!(
        rows_folded, 3,
        "the two hidden lines contribute ZERO visual rows"
    );
    assert_eq!(
        rows_unfolded - rows_folded,
        2,
        "exactly the folded section's line count is removed from the layout"
    );
}

// ITEM 47a — the quiet "… N lines" TAIL on a collapsed heading: it carries the
// CORRECT hidden count, rides the heading's OWN row (adds no row / never disturbs
// the zero-height hidden-row law), and hangs to the RIGHT of the heading text.
#[test]
fn fold_tail_rides_the_heading_row_with_the_correct_count() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("no GPU adapter; skipping fold-tail law");
        return;
    };

    // FOLD # A (line 0): hides a1, a2 → the tail reads "… 2 lines" on the heading.
    let (hidden, tails) = fold(&[0]);
    let mut folded = view_md(DOC, 0, 0);
    crate::fold::apply_to_view(&mut folded, &hidden, &tails);
    // The view records the tail on the heading's FILTERED row (0) with count 2.
    assert_eq!(folded.fold_tails, vec![FoldTail { line: 0, hidden: 2 }]);

    p.set_view(&folded);
    // The tail is an ORNAMENT, not a shaped line: the folded doc is still exactly 3
    // visual rows — the tail added NONE (the zero-height hidden-row law is intact).
    assert_eq!(
        p.total_visual_rows(),
        3,
        "the tail rides the heading row; it adds no visual row"
    );

    // ONE mark, on the heading's own row, with the right N, past the heading text.
    let marks = p.fold_tail_marks();
    assert_eq!(marks.len(), 1, "one tail for the one collapsed heading");
    let (baseline, left, n, line) = marks[0];
    assert_eq!(n, 2, "the tail's N is the section's hidden-line count");
    assert_eq!(line, 0, "the tail hangs on the filtered heading row");
    // item 65: the mark's `f32` slot is the heading's REAL shaped BASELINE (the
    // draw pass then subtracts the tail's OWN shaped `line_y` from this), not the
    // row's top — baseline-aligned, not merely centered in the tall heading row.
    assert_eq!(
        baseline,
        p.line_ornament_baseline(0),
        "the tail's placement baseline is the heading row's own REAL shaped baseline"
    );
    assert!(
        left > p.text_left(),
        "the tail sits to the RIGHT of the heading text, not over it"
    );
}

// The count tracks the ACTUAL hidden extent: folding the deeper section # B (which
// hides only its single body line) reads "… 1 line", and a nested fold's tail is
// suppressed while its parent is folded (the parent's count already covers it).
#[test]
fn fold_tail_count_tracks_the_hidden_extent() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("no GPU adapter; skipping fold-tail count law");
        return;
    };

    // FOLD # B (line 3): its section is line 4 (b1) only → "… 1 line". Only that one
    // line below it hides, so # B keeps its full-doc row 3 in the filtered document.
    let (hidden, tails) = fold(&[3]);
    let mut folded = view_md(DOC, 0, 0);
    crate::fold::apply_to_view(&mut folded, &hidden, &tails);
    assert_eq!(folded.fold_tails, vec![FoldTail { line: 3, hidden: 1 }]);
    p.set_view(&folded);
    let marks = p.fold_tail_marks();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].2, 1, "# B hides exactly one line");
    assert_eq!(marks[0].3, 3, "# B renders on filtered row 3");
}

// item 65 GALLERY-FOUND REGRESSION: a collapsed heading long enough to visually
// WRAP used to hang its tail off the FLATTENED end-of-line x
// (`line_glyph_xs(line).last()`, which deliberately offsets each wrapped row's
// glyphs to continue past the previous one for callers that don't care which row
// a column lands on) — landing the tail comfortably past the actual column, off
// in the page's right margin, utterly disconnected from the heading it annotates.
// The fix reads the FIRST VISUAL ROW's own row-LOCAL end x
// (`visual_rows(line)[0]`, never offset across rows), matching where the tail's
// BASELINE already sits (always the first row's — see `fold_tail_marks`'s doc).
#[test]
fn fold_tail_hangs_after_the_first_visual_row_when_the_heading_wraps() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("no GPU adapter; skipping fold-tail wrap regression law");
        return;
    };
    // A single H1 line long enough to wrap at the default 1200px canvas (H1's
    // scaled-up glyphs make even a fairly ordinary sentence wrap).
    let doc = "# A rather long section heading that keeps going for quite a while indeed so it wraps here\nbody one\nbody two\n";
    let levels = crate::fold::heading_levels(doc, true);
    let folds: BTreeSet<usize> = [0].into_iter().collect();
    let hidden = crate::fold::hidden_lines(&levels, &folds);
    let tails = crate::fold::fold_tails(&levels, &folds);

    let mut view = view_md(doc, 0, 0);
    crate::fold::apply_to_view(&mut view, &hidden, &tails);
    p.set_view(&view);

    // Fixture self-check: the heading genuinely wraps to more than one visual row
    // (else this test cannot witness the bug it guards against).
    let rows = p.visual_rows(0);
    assert!(
        rows.len() > 1,
        "fixture must actually wrap the heading to >1 visual row, got {}",
        rows.len()
    );
    // The OLD (buggy) placement's `end` — the flattened, cumulative-across-rows x —
    // is what the fix must NOT use: it lands far past the actual column.
    let flattened_end = p.line_glyph_xs(0).last().copied().unwrap_or(0.0);
    let buggy_left = p.text_left() + flattened_end;

    let marks = p.fold_tail_marks();
    assert_eq!(marks.len(), 1);
    let (_, left, _, _) = marks[0];

    // FIX: the tail stays within a first-row-sized budget — comfortably inside the
    // actual wrap width, nowhere near the buggy flattened placement.
    let ceiling = p.text_left() + p.text_wrap_width();
    assert!(
        left <= ceiling,
        "the tail must hang within the actual column, not past it: left={left} ceiling={ceiling}"
    );
    assert!(
        left < buggy_left - p.metrics.char_width,
        "the tail must NOT land at the old flattened (cumulative-across-wrapped-rows) x: \
         left={left} buggy_flattened_left={buggy_left}"
    );
}

// ITEM 47b (item 65 taste correction) — the expand CHEVRON is a SUMMONED
// affordance: shown only when the caret is on the collapsed heading (the
// headlessly-reachable arm; hover is live-only). It now hangs IMMEDIATELY LEFT of
// the heading — OUTSIDE the editable text advance, in the writing column's own
// leading pad — never sharing the tail's (unmoved, right-of-text) slot. The tail
// never hides.
#[test]
fn fold_chevron_reveals_only_when_the_caret_is_on_the_collapsed_heading() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("no GPU adapter; skipping fold-chevron law");
        return;
    };
    // PAGE MODE globals are process-wide; serialize with every other page test and
    // restore the default so a later test never inherits this one's setting.
    let _g = crate::testlock::serial();
    crate::page::set_page_on(true);
    let (hidden, tails) = fold(&[0]); // fold # A; its tail hangs on filtered row 0

    // Caret ON the collapsed heading (# A, full line 0 → filtered row 0, where folding
    // parks it): the chevron reveals on that row, LEFT of the heading text.
    let mut on = view_md(DOC, 0, 0);
    crate::fold::apply_to_view(&mut on, &hidden, &tails);
    assert_eq!(on.cursor_line, 0, "caret on the folded heading's filtered row");
    p.set_view(&on);
    let ch = p.fold_chevron_marks();
    assert_eq!(ch.len(), 1, "the caret-on-heading chevron reveals");
    assert_eq!(ch[0].2, 0, "on the heading's own filtered row");
    assert_eq!(
        ch[0].0,
        p.line_ornament_baseline(0),
        "the chevron's placement baseline is the heading row's own REAL shaped baseline"
    );
    assert!(
        ch[0].1 < p.text_left(),
        "the chevron sits OUTSIDE the editable text advance, strictly left of text_left \
         (got {} vs text_left {})",
        ch[0].1,
        p.text_left()
    );
    let tail = p.fold_tail_marks();
    assert!(
        ch[0].1 < tail[0].1,
        "the chevron (left margin) sits LEFT of the tail (right of the heading text)"
    );

    // Caret OFF the heading (on b1, full line 4 → a different filtered row): NO chevron,
    // but the tail is still shown — the tail is unconditional, the chevron summoned.
    let mut off = view_md(DOC, 4, 0);
    crate::fold::apply_to_view(&mut off, &hidden, &tails);
    assert_ne!(off.cursor_line, 0, "caret is not on the collapsed heading");
    p.set_view(&off);
    assert!(
        p.fold_chevron_marks().is_empty(),
        "no chevron when the caret is not on the heading (and no pointer to hover) — \
         rest state shows no chevron"
    );
    assert_eq!(
        p.fold_tail_marks().len(),
        1,
        "the tail is always shown, chevron or not"
    );
    crate::page::set_page_on(true);
}

// item 65 NO-OVERLAP PIXEL LAW: the chevron is a SEPARATE ornament — never part of
// the shaped document glyph run — so revealing it must never shift the heading's
// own glyph x-positions vs the no-chevron REST state. Isolated via HOVER (not the
// caret): landing the CARET on a heading ALSO reveals its raw WYSIWYG markdown
// markup (PHILOSOPHY.md's "any line shows raw markdown while the caret is on
// it"), which genuinely DOES change that line's glyph advances (CLAUDE.md's own
// tripwire: "Conceal reveal changes glyph advances, not just color") — a real,
// unrelated effect that would otherwise contaminate this law. Hover triggers the
// SAME `chevron_revealed` arm without touching conceal state, so comparing
// rest-vs-hover (the pixel law's own phrasing) isolates the chevron alone.
// Compared via the SAME real-shaped-glyph source the caret/hit-test/selection all
// read ([`TextPipeline::line_glyph_xs`]), not re-derived pixels.
#[test]
fn fold_chevron_reveal_never_shifts_the_heading_glyph_positions() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("no GPU adapter; skipping fold-chevron no-overlap law");
        return;
    };
    let _g = crate::testlock::serial();
    crate::page::set_page_on(true);
    let (hidden, tails) = fold(&[0]); // fold # A
    // Caret parked away from the heading for BOTH states (line 4, "b1") — WYSIWYG
    // conceal on the heading line is therefore identical in both; only hover varies.
    let mut view = view_md(DOC, 4, 0);
    crate::fold::apply_to_view(&mut view, &hidden, &tails);

    // REST: no hover, chevron absent.
    p.set_view(&view);
    p.set_hover_line(None);
    assert!(p.fold_chevron_marks().is_empty(), "rest state: no chevron");
    let xs_rest = p.line_glyph_xs(0);
    let top_rest = p.line_ornament_top(0);
    let rows_rest = p.total_visual_rows();

    // HOVER on the collapsed heading's row: chevron revealed.
    p.set_hover_line(Some(0));
    assert_eq!(p.fold_chevron_marks().len(), 1, "chevron revealed: hovering the heading");
    let xs_reveal = p.line_glyph_xs(0);
    let top_reveal = p.line_ornament_top(0);
    let rows_reveal = p.total_visual_rows();

    assert_eq!(
        xs_rest, xs_reveal,
        "the heading's own shaped glyph x-boundaries must be IDENTICAL whether or \
         not the chevron is revealed (it lives outside the text advance entirely)"
    );
    assert_eq!(top_rest, top_reveal, "the heading row's top never moves either");
    assert_eq!(rows_rest, rows_reveal, "revealing the chevron adds no visual row");
    crate::page::set_page_on(true);
}

// item 65 graceful-hide: the chevron needs room in the writing column's own
// leading pad ([`TextPipeline::text_left`] minus [`TextPipeline::column_left`]).
// Edge-to-edge (page mode off) that pad is exactly zero, so the chevron would
// otherwise land ON TOP of the heading's own first glyph — instead it hides
// entirely, mirroring the outline's / gutter's own no-room floors. The tail is
// UNAFFECTED (it hangs to the right, in the always-available wrap width), so the
// collapsed state stays legible either way.
#[test]
fn fold_chevron_hides_gracefully_with_no_room_edge_to_edge() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("no GPU adapter; skipping fold-chevron no-room law");
        return;
    };
    let _g = crate::testlock::serial();
    crate::page::set_page_on(false); // edge-to-edge: text_left == column_left, zero pad
    let (hidden, tails) = fold(&[0]);
    let mut on = view_md(DOC, 0, 0);
    crate::fold::apply_to_view(&mut on, &hidden, &tails);
    p.set_view(&on);
    assert!(
        (p.text_left() - p.column_left()).abs() < 0.01,
        "edge-to-edge has no writing-column leading pad to hang the chevron in"
    );
    assert!(
        p.fold_chevron_marks().is_empty(),
        "no room => the chevron gracefully hides even with the caret on the heading"
    );
    assert_eq!(
        p.fold_tail_marks().len(),
        1,
        "the tail is unaffected by the chevron's room gate — still shows the count"
    );
    crate::page::set_page_on(true);
}
