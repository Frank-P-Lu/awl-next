//! PURE fold-logic laws (no rope / render / globals). Every rule the actions and
//! render lean on is pinned here at the purest reachable seam.

use super::*;
use std::collections::BTreeSet;

fn folds(items: &[usize]) -> BTreeSet<usize> {
    items.iter().copied().collect()
}

// A small document with a nested outline:
//   0  # A
//   1  a body
//   2  ## A.1
//   3  a1 body
//   4  ## A.2
//   5  a2 body
//   6  # B
//   7  b body
const OUTLINE: &str = "# A\na body\n## A.1\na1 body\n## A.2\na2 body\n# B\nb body";

#[test]
fn heading_level_counts_the_leading_hash_run() {
    assert_eq!(heading_level("# H", true), 1);
    assert_eq!(heading_level("### deep", true), 3);
    assert_eq!(heading_level("#nospace", true), 1); // matches render SIZE half
    assert_eq!(heading_level("  ## indented", true), 2);
    assert_eq!(heading_level("body # not", true), 0);
    assert_eq!(heading_level("plain", true), 0);
}

#[test]
fn non_markdown_has_no_headings() {
    assert_eq!(heading_level("# H", false), 0);
    let levels = heading_levels("# H\nbody", false);
    assert_eq!(levels, vec![0, 0]);
}

#[test]
fn section_range_stops_at_a_sibling_or_shallower_heading() {
    let levels = heading_levels(OUTLINE, true);
    // # A (level 1) hides everything up to # B at line 6.
    assert_eq!(section_range(&levels, 0), (1, 6));
    // ## A.1 (level 2) hides only its own body up to ## A.2 at line 4.
    assert_eq!(section_range(&levels, 2), (3, 4));
    // # B hides to EOF.
    assert_eq!(section_range(&levels, 6), (7, 8));
}

#[test]
fn folding_a_parent_hides_child_headings_whole() {
    let levels = heading_levels(OUTLINE, true);
    let hidden = hidden_lines(&levels, &folds(&[0])); // fold # A
    // The heading line itself stays visible; its whole subtree (lines 1..=5) hides.
    assert_eq!(
        hidden,
        vec![false, true, true, true, true, true, false, false]
    );
}

#[test]
fn hidden_lines_is_empty_without_folds() {
    let levels = heading_levels(OUTLINE, true);
    let hidden = hidden_lines(&levels, &BTreeSet::new());
    assert!(hidden.iter().all(|&h| !h), "nothing hidden with no folds");
}

#[test]
fn hidden_count_is_the_section_length() {
    let levels = heading_levels(OUTLINE, true);
    assert_eq!(hidden_count(&levels, 0), 5); // lines 1..=5
    assert_eq!(hidden_count(&levels, 2), 1); // line 3 only
}

#[test]
fn fold_tails_reports_visible_headings_with_their_hidden_counts() {
    let levels = heading_levels(OUTLINE, true);
    // Fold ## A.1 (line 2, hides line 3) and # B (line 6, hides line 7): both
    // headings are still visible, so both get a tail with their own count.
    let tails = fold_tails(&levels, &folds(&[2, 6]));
    assert_eq!(tails, vec![(2, 1), (6, 1)]);
    // Nothing folded → no tails.
    assert!(fold_tails(&levels, &BTreeSet::new()).is_empty());
}

#[test]
fn fold_tails_suppresses_a_heading_hidden_by_a_folded_parent() {
    let levels = heading_levels(OUTLINE, true);
    // Fold # A (line 0, hides lines 1..=5) AND its child ## A.1 (line 2). The child
    // is itself HIDDEN inside # A's section, so it contributes NO tail — only the
    // visible parent's tail shows, counting its whole subtree (5 lines).
    let tails = fold_tails(&levels, &folds(&[0, 2]));
    assert_eq!(tails, vec![(0, 5)], "the nested child's tail is suppressed");
}

#[test]
fn chevron_reveals_only_on_the_caret_or_hovered_heading() {
    // The caret ON the heading's row reveals its chevron; a different caret row does not.
    assert!(chevron_revealed(2, 2, None));
    assert!(!chevron_revealed(2, 5, None));
    // HOVER (live only) reveals it; hovering a different row does not.
    assert!(chevron_revealed(2, 5, Some(2)));
    assert!(!chevron_revealed(2, 5, Some(3)));
    // Either arm alone suffices.
    assert!(chevron_revealed(2, 2, Some(9)));
}

#[test]
fn enclosing_heading_reads_the_innermost_section() {
    let levels = heading_levels(OUTLINE, true);
    assert_eq!(enclosing_heading(&levels, 0), Some(0)); // on # A itself
    assert_eq!(enclosing_heading(&levels, 1), Some(0)); // body under # A
    assert_eq!(enclosing_heading(&levels, 3), Some(2)); // body under ## A.1
    assert_eq!(enclosing_heading(&levels, 5), Some(4)); // body under ## A.2
    assert_eq!(enclosing_heading(&levels, 7), Some(6)); // body under # B
}

#[test]
fn enclosing_heading_is_none_before_the_first_heading() {
    let levels = heading_levels("preamble\nmore\n# A\nbody", true);
    assert_eq!(enclosing_heading(&levels, 0), None);
    assert_eq!(enclosing_heading(&levels, 1), None);
    assert_eq!(enclosing_heading(&levels, 3), Some(2));
}

#[test]
fn toggle_folds_then_unfolds_the_enclosing_heading() {
    let levels = heading_levels(OUTLINE, true);
    let mut f = BTreeSet::new();
    // Caret in A.1 body -> toggles ## A.1 (line 2).
    assert_eq!(toggle_at(&levels, &mut f, 3), Some(2));
    assert!(f.contains(&2));
    // Toggle again from the same spot -> unfolds.
    assert_eq!(toggle_at(&levels, &mut f, 3), Some(2));
    assert!(f.is_empty());
    // No enclosing heading -> None, no change.
    let mut g = BTreeSet::new();
    assert_eq!(toggle_at(&levels, &mut g, 100), enclosing_heading(&levels, 100));
}

#[test]
fn collapse_others_keeps_the_caret_chain_and_subtree_open() {
    let levels = heading_levels(OUTLINE, true);
    // Caret in ## A.1 body: keep # A (ancestor) + ## A.1 (self); fold ## A.2 and # B.
    let f = collapse_others(&levels, 3);
    assert_eq!(f, folds(&[4, 6]));
    // Nothing in the kept chain is hidden.
    let hidden = hidden_lines(&levels, &f);
    assert!(!hidden[0] && !hidden[2], "the caret's chain stays visible");
    assert!(hidden[5] && hidden[7], "sibling + unrelated sections collapse");
}

#[test]
fn collapse_others_on_a_top_heading_keeps_its_whole_subtree() {
    let levels = heading_levels(OUTLINE, true);
    // Caret on # A: keep # A and everything nested (## A.1, ## A.2); fold only # B.
    let f = collapse_others(&levels, 0);
    assert_eq!(f, folds(&[6]));
}

#[test]
fn collapse_others_before_the_first_heading_folds_everything() {
    let levels = heading_levels("intro\n# A\nx\n# B\ny", true);
    let f = collapse_others(&levels, 0);
    assert_eq!(f, folds(&[1, 3]));
}

#[test]
fn expand_containing_reveals_a_line_hidden_by_nested_folds() {
    let levels = heading_levels(OUTLINE, true);
    // Fold both # A and ## A.1: line 3 is hidden by both.
    let mut f = folds(&[0, 2]);
    assert!(hidden_lines(&levels, &f)[3]);
    let changed = expand_containing(&levels, &mut f, 3);
    assert!(changed);
    // Both folds that hid line 3 are gone; line 3 is now visible.
    assert!(!hidden_lines(&levels, &f)[3]);
    assert!(f.is_empty());
}

#[test]
fn expand_containing_leaves_a_fold_you_are_sitting_on() {
    let levels = heading_levels(OUTLINE, true);
    let mut f = folds(&[0]); // fold # A
    // Caret ON the folded heading line 0 (never hidden) — the fold stays.
    let changed = expand_containing(&levels, &mut f, 0);
    assert!(!changed);
    assert!(f.contains(&0));
}

#[test]
fn expand_range_reveals_a_fold_the_selection_would_span_invisibly() {
    let levels = heading_levels(OUTLINE, true);
    let mut f = folds(&[2]); // ## A.1 hides line 3
    // A selection from line 1 to line 5 spans the hidden line 3.
    let changed = expand_range(&levels, &mut f, 1, 5);
    assert!(changed);
    assert!(f.is_empty());
}

#[test]
fn expand_range_leaves_a_fold_the_selection_does_not_touch() {
    let levels = heading_levels(OUTLINE, true);
    let mut f = folds(&[6]); // # B hides line 7
    // Selection entirely inside the A subtree does not touch B's hidden line.
    let changed = expand_range(&levels, &mut f, 0, 5);
    assert!(!changed);
    assert!(f.contains(&6));
}

#[test]
fn filter_drops_hidden_lines_and_remaps_visible_ones() {
    let text = "# A\na1\na2\n# B\nb1";
    let hidden = [false, true, true, false, false];
    let f = Filter::new(text, &hidden);
    assert!(f.any_hidden());
    assert_eq!(f.text, "# A\n# B\nb1");
    // Visible full lines remap to their filtered index.
    assert_eq!(f.line(0), 0);
    assert_eq!(f.line(3), 1); // # B: two hidden lines removed before it
    assert_eq!(f.line(4), 2);
    assert!(f.visible(0) && !f.visible(1) && f.visible(3));
}

#[test]
fn filter_is_identity_when_nothing_is_hidden() {
    let text = "# A\na1\n# B";
    let f = Filter::new(text, &[false, false, false]);
    assert!(!f.any_hidden());
    assert_eq!(f.text, text);
    assert_eq!(f.line(2), 2);
}

#[test]
fn prune_stale_drops_a_fold_whose_heading_was_edited_away() {
    // Same lines as OUTLINE but line 2 is no longer a heading.
    let edited = "# A\na body\nA.1 plain\na1 body\n## A.2\na2 body\n# B\nb body";
    let levels = heading_levels(edited, true);
    let mut f = folds(&[0, 2]);
    let changed = prune_stale(&levels, &mut f);
    assert!(changed);
    assert_eq!(f, folds(&[0]), "the non-heading fold entry is pruned");
}
