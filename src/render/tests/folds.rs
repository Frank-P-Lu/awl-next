//! FOLD RENDER LAW: a collapsed section's hidden lines are dropped from the shaped
//! text, so they contribute ZERO visual rows (and zero height) — the row simply is
//! not laid out. Driven through the SAME `fold::apply_to_view` seam the live
//! `sync_view` and the headless capture use, then shaped by a real headless
//! pipeline so the geometry is the true one.

use super::{headless_pipeline, view_md};

// Two sibling sections, no soft-wrap:
//   0 # A / 1 a1 / 2 a2 / 3 # B / 4 b1
const DOC: &str = "# A\na1\na2\n# B\nb1";

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
    let hidden = vec![false, true, true, false, false];
    let mut folded = view_md(DOC, 0, 0);
    crate::fold::apply_to_view(&mut folded, &hidden);
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
