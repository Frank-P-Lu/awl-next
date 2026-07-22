//! FOLD action-seam laws: fold/unfold + collapse-others through `apply_core`, and
//! the AUTO-EXPAND rules (an edit / motion landing inside a fold reveals it; a
//! selection never spans a fold invisibly; folds are never on the undo timeline).
//! Folds are VIEW state on the buffer, so they drive through the shared core
//! exactly like the `--keys` replay.

use super::super::*;
use super::{drive_act, drive_act_effect};

// The nested outline the pure `fold::tests` also use:
//   0 # A / 1 a body / 2 ## A.1 / 3 a1 body / 4 ## A.2 / 5 a2 body / 6 # B / 7 b body
const OUTLINE: &str = "# A\na body\n## A.1\na1 body\n## A.2\na2 body\n# B\nb body";

#[test]
fn toggle_fold_collapses_then_expands_the_section_under_the_caret() {
    let mut buffer = Buffer::from_str(OUTLINE);
    // Caret in ## A.1's body (line 3) -> folds ## A.1 (line 2).
    buffer.set_cursor(buffer.line_col_to_char(3, 1));
    drive_act(&mut buffer, &Action::ToggleFold);
    assert!(buffer.folds().contains(&2), "the enclosing heading folded");
    assert!(buffer.has_folds());
    // The rope is untouched — a fold is view state, never file content.
    assert_eq!(buffer.text(), OUTLINE);
    // Caret is still on the visible line 3 (never hidden by its own toggle), so a
    // second toggle re-expands.
    drive_act(&mut buffer, &Action::ToggleFold);
    assert!(buffer.folds().is_empty(), "toggling again unfolds");
}

#[test]
fn fold_is_never_on_the_undo_timeline() {
    let mut buffer = Buffer::from_str(OUTLINE);
    buffer.set_cursor(0);
    drive_act(&mut buffer, &Action::ToggleFold);
    assert!(buffer.folds().contains(&0));
    assert!(
        !buffer.can_undo(),
        "a fold pushes no undo group — it is pure view state"
    );
}

#[test]
fn collapse_other_sections_keeps_only_the_caret_chain_open() {
    let mut buffer = Buffer::from_str(OUTLINE);
    // Caret in ## A.1 body (line 3): keep # A + ## A.1; fold ## A.2 (4) and # B (6).
    buffer.set_cursor(buffer.line_col_to_char(3, 0));
    drive_act(&mut buffer, &Action::CollapseOtherSections);
    let folds: Vec<usize> = buffer.folds().iter().copied().collect();
    assert_eq!(folds, vec![4, 6]);
}

#[test]
fn auto_expand_when_a_motion_lands_inside_a_fold() {
    let mut buffer = Buffer::from_str(OUTLINE);
    buffer.set_cursor(0); // on # A
    drive_act(&mut buffer, &Action::ToggleFold); // fold # A -> hides lines 1..=5
    assert!(buffer.folds().contains(&0));
    // Move DOWN into the (hidden) first body line — the fold auto-expands.
    drive_act(&mut buffer, &Action::NextLine);
    assert_eq!(buffer.cursor_line_col().0, 1, "caret moved into the section");
    assert!(
        buffer.folds().is_empty(),
        "landing inside a fold reveals it"
    );
}

#[test]
fn auto_expand_when_an_edit_lands_inside_a_fold() {
    let mut buffer = Buffer::from_str(OUTLINE);
    buffer.set_cursor(0);
    drive_act(&mut buffer, &Action::ToggleFold); // fold # A
    // Place the caret on a hidden line WITHOUT going through apply_core (set_cursor
    // does not reveal), then type — the edit must reveal the fold.
    buffer.set_cursor(buffer.line_col_to_char(3, 0));
    assert!(buffer.folds().contains(&0), "still folded before the edit");
    drive_act(&mut buffer, &Action::InsertChar('x'));
    assert!(
        buffer.folds().is_empty(),
        "an edit inside a fold auto-expands it"
    );
}

#[test]
fn selection_never_spans_a_fold_invisibly() {
    let mut buffer = Buffer::from_str(OUTLINE);
    // Fold ## A.1 (line 2) so its body line 3 is hidden.
    buffer.set_cursor(buffer.line_col_to_char(2, 0));
    drive_act(&mut buffer, &Action::ToggleFold);
    assert!(buffer.folds().contains(&2));
    // Mark at line 1, then extend the sticky selection down past the hidden line.
    buffer.set_cursor(buffer.line_col_to_char(1, 0));
    drive_act(&mut buffer, &Action::SetMark);
    drive_act(&mut buffer, &Action::NextLine); // -> line 2 (heading, visible)
    drive_act(&mut buffer, &Action::NextLine); // -> line 3 (was hidden)
    assert!(
        buffer.has_selection(),
        "the sticky mark built a selection"
    );
    assert!(
        buffer.folds().is_empty(),
        "a selection crossing a fold reveals it so nothing hides inside the region"
    );
}

#[test]
fn toggle_fold_is_a_calm_noop_with_no_enclosing_heading() {
    // Body text before any heading: nothing to fold.
    let mut buffer = Buffer::from_str("just prose\nmore prose");
    buffer.set_cursor(0);
    let eff = drive_act_effect(&mut buffer, &Action::ToggleFold);
    assert_eq!(eff, Effect::None);
    assert!(buffer.folds().is_empty());
}
