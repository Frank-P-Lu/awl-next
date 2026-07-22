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
    let (top, left, n, line) = marks[0];
    assert_eq!(n, 2, "the tail's N is the section's hidden-line count");
    assert_eq!(line, 0, "the tail hangs on the filtered heading row");
    assert_eq!(
        top,
        p.line_ornament_top(0),
        "the tail's top is the heading row's own top (rides that row)"
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

// ITEM 47b — the expand CHEVRON is a SUMMONED affordance: shown only when the caret
// is on the collapsed heading (the headlessly-reachable arm; hover is live-only). It
// rides the heading's own row, LEFT of the always-present tail. The tail never hides.
#[test]
fn fold_chevron_reveals_only_when_the_caret_is_on_the_collapsed_heading() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("no GPU adapter; skipping fold-chevron law");
        return;
    };
    let (hidden, tails) = fold(&[0]); // fold # A; its tail hangs on filtered row 0

    // Caret ON the collapsed heading (# A, full line 0 → filtered row 0, where folding
    // parks it): the chevron reveals on that row, LEFT of the tail.
    let mut on = view_md(DOC, 0, 0);
    crate::fold::apply_to_view(&mut on, &hidden, &tails);
    assert_eq!(on.cursor_line, 0, "caret on the folded heading's filtered row");
    p.set_view(&on);
    let ch = p.fold_chevron_marks();
    assert_eq!(ch.len(), 1, "the caret-on-heading chevron reveals");
    assert_eq!(ch[0].2, 0, "on the heading's own filtered row");
    assert_eq!(ch[0].0, p.line_ornament_top(0), "riding the heading row");
    let tail = p.fold_tail_marks();
    assert!(
        ch[0].1 < tail[0].1,
        "the chevron sits LEFT of the tail (its reserved slot)"
    );

    // Caret OFF the heading (on b1, full line 4 → a different filtered row): NO chevron,
    // but the tail is still shown — the tail is unconditional, the chevron summoned.
    let mut off = view_md(DOC, 4, 0);
    crate::fold::apply_to_view(&mut off, &hidden, &tails);
    assert_ne!(off.cursor_line, 0, "caret is not on the collapsed heading");
    p.set_view(&off);
    assert!(
        p.fold_chevron_marks().is_empty(),
        "no chevron when the caret is not on the heading (and no pointer to hover)"
    );
    assert_eq!(
        p.fold_tail_marks().len(),
        1,
        "the tail is always shown, chevron or not"
    );
}
