//! The caret recoil/flinch feedback triggers at blocked motions, no-op
//! deletes, and copy -- split out of the former monolithic `actions::tests`
//! (2026-07 code-organization pass).

use super::super::*;
use crate::overlay::OverlayKind;
use super::{drive_effect_and_cursor, drive_effect, delete_flinch_fixture, drive_act_effect, motion_boundary_fixture, all_actions};

#[test]
fn blocked_motions_arm_recoil_away_from_the_wall() {
    use crate::caret::RecoilDir::{Down, Left, Right, Up};
    let txt = "ab\ncd"; // chars: a b \n c d  (end == char 5)
    // Horizontal walls.
    assert_eq!(drive_effect(txt, 5, &Action::ForwardChar), Effect::Recoil(Left));
    assert_eq!(drive_effect(txt, 0, &Action::BackwardChar), Effect::Recoil(Right));
    assert_eq!(drive_effect(txt, 5, &Action::ForwardWord), Effect::Recoil(Left));
    assert_eq!(drive_effect(txt, 0, &Action::BackwardWord), Effect::Recoil(Right));
    // BOUNDARY BUMP — line-edge motions already at the edge (C-a/C-e,
    // Cmd-Left/Right): cursor 0 is already line 0's start; cursor 2 is already
    // line 0's end (right before the '\n').
    assert_eq!(drive_effect(txt, 0, &Action::LineStart), Effect::Recoil(Right));
    assert_eq!(drive_effect(txt, 2, &Action::LineEnd), Effect::Recoil(Left));
    // Vertical walls (cursor parked at the end of the last / start of the first
    // line so the logical motion truly can't move).
    assert_eq!(drive_effect(txt, 5, &Action::NextLine), Effect::Recoil(Up));
    assert_eq!(drive_effect(txt, 0, &Action::PreviousLine), Effect::Recoil(Down));
    // Buffer ends already at the end / start.
    assert_eq!(drive_effect(txt, 5, &Action::BufferEnd), Effect::Recoil(Up));
    assert_eq!(drive_effect(txt, 0, &Action::BufferStart), Effect::Recoil(Down));
    // Page scroll that can't page (1 line per page; already at top/bottom).
    assert_eq!(drive_effect(txt, 5, &Action::PageScrollDown), Effect::Recoil(Up));
    assert_eq!(drive_effect(txt, 0, &Action::PageScrollUp), Effect::Recoil(Down));
}

#[test]
fn unblocked_motions_do_not_recoil() {
    let txt = "ab\ncd";
    // Each of these CAN proceed, so no recoil (and the cursor really moved).
    assert_eq!(drive_effect(txt, 0, &Action::ForwardChar), Effect::None);
    assert_eq!(drive_effect(txt, 5, &Action::BackwardChar), Effect::None);
    assert_eq!(drive_effect(txt, 0, &Action::NextLine), Effect::None);
    assert_eq!(drive_effect(txt, 5, &Action::PreviousLine), Effect::None);
    assert_eq!(drive_effect(txt, 0, &Action::BufferEnd), Effect::None);
    assert_eq!(drive_effect(txt, 5, &Action::BufferStart), Effect::None);
    // Line-edge motions NOT already at the edge proceed too (a real relocation).
    assert_eq!(drive_effect(txt, 1, &Action::LineStart), Effect::None);
    assert_eq!(drive_effect(txt, 0, &Action::LineEnd), Effect::None);
}

#[test]
fn blocked_recoil_leaves_buffer_and_cursor_untouched() {
    // The whole point of a recoil: the logical state does NOT change (only the
    // visual caret bumps, live-only), so a settled capture is byte-identical.
    let mut buffer = Buffer::from_str("ab\ncd");
    buffer.set_cursor(5);
    let before_text = buffer.text();
    let before_cursor = buffer.cursor_char();
    let eff = drive_effect("ab\ncd", 5, &Action::ForwardChar);
    assert!(matches!(eff, Effect::Recoil(_)));
    // Re-run on the same buffer instance to assert no mutation slipped through.
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut overlay = None;
    let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
    let mut browse_to =
        |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
    let mut ctx = ActionCtx {
        buffer: &mut buffer,
        shift_selecting: &mut shift,
        zoom: &mut zoom,
        search: &mut search,
        scroll_page_lines: 1,
        overlay: &mut overlay,
        make_overlay: &mut make_overlay,
        browse_to: &mut browse_to,
        oracle: None,
    };
    apply_core(&mut ctx, &Action::ForwardChar, false);
    drop(ctx);
    assert_eq!(buffer.text(), before_text);
    assert_eq!(buffer.cursor_char(), before_cursor);
}

#[test]
fn exhausted_undo_redo_recoil() {
    use crate::caret::RecoilDir::{Left, Right};
    // A fresh buffer has no history: undo/redo are no-ops -> recoil.
    assert_eq!(drive_effect("hello", 0, &Action::Undo), Effect::Recoil(Left));
    assert_eq!(drive_effect("hello", 0, &Action::Redo), Effect::Recoil(Right));
}

#[test]
fn blocked_delete_recoils_no_op_delete() {
    use crate::caret::RecoilDir::{Left, Right};
    // Backspace at buffer start / C-d / M-d at buffer end remove nothing -> recoil.
    assert_eq!(drive_effect("hi", 0, &Action::DeleteBackward), Effect::Recoil(Right));
    assert_eq!(drive_effect("hi", 2, &Action::DeleteForward), Effect::Recoil(Left));
    assert_eq!(drive_effect("hi", 2, &Action::DeleteWordForward), Effect::Recoil(Left));
    // A delete that DOES remove a char SUCCEEDS -> the caret swallows what it ate
    // (the PHASE 2 inward squash), mutually exclusive with the blocked recoil.
    assert_eq!(drive_effect("hi", 1, &Action::DeleteBackward), Effect::DeleteSquash);
    assert_eq!(drive_effect("hi", 0, &Action::DeleteForward), Effect::DeleteSquash);
}

#[test]
fn successful_edits_arm_the_caret_flinch() {
    // PHASE 2 — a SUCCESSFUL edit flinches the visual caret: a typed char → a
    // typing impact, a backspace / C-d / word-delete → an inward squash, a
    // kill-line → a gulp. The trigger reads the SAME content-version signal the
    // recoil uses (here it CHANGED), so it's drivable + unit-testable with no GPU.
    assert_eq!(drive_effect("hi", 1, &Action::InsertChar('x')), Effect::TypeImpact);
    assert_eq!(drive_effect("hi", 1, &Action::DeleteBackward), Effect::DeleteSquash);
    assert_eq!(drive_effect("hi", 0, &Action::DeleteForward), Effect::DeleteSquash);
    assert_eq!(drive_effect("foo bar", 7, &Action::DeleteWordBackward), Effect::DeleteSquash);
    assert_eq!(drive_effect("foo bar", 0, &Action::DeleteWordForward), Effect::DeleteSquash);
    // A kill-line that removes text gulps.
    assert_eq!(drive_effect("hello", 0, &Action::KillLine), Effect::Gulp);
    // PHASE 3 — ENTER JUICE: a plain Enter lands a caret-level touchdown squash,
    // and so does the markdown smart-Enter's list-continuation edit (same Action,
    // same arm — the flinch is keyed off `Action::Newline`, not which branch fired).
    assert_eq!(drive_effect("hi", 1, &Action::Newline), Effect::LineLand);
    assert_eq!(drive_effect("- item", 6, &Action::Newline), Effect::LineLand);
}

#[test]
fn no_op_edits_and_non_edits_do_not_flinch() {
    // A kill-line at the very end of the buffer removes nothing -> the content
    // version is unchanged, so NO gulp (and no recoil — kill-line has no wall arm).
    assert_eq!(drive_effect("hi", 2, &Action::KillLine), Effect::None);
    // A plain motion is not an edit: it never flinches (it may recoil, tested
    // elsewhere). A mid-buffer forward-char just moves -> None.
    assert_eq!(drive_effect("hi", 0, &Action::ForwardChar), Effect::None);
}

#[test]
fn every_delete_squashes_on_success_and_recoils_on_a_no_op() {
    // COMPLETENESS SWEEP over `all_actions()` (compile-time-complete via its
    // `_assert_covers`): every DELETE flinches BOTH ways — an inward
    // `DeleteSquash` when it removes a char, and a boundary `Recoil` when it
    // removes nothing at the buffer edge. `delete_flinch_fixture`'s no-wildcard
    // match forces every new `Action` to be classified, so a future delete
    // can't silently ship with no caret feedback — the exact gap M-d
    // (`DeleteWordForward`) fell through, missing from BOTH `impact_for` and
    // `recoil_for` while every other delete flinched.
    for action in all_actions() {
        let Some((text, ok_cursor, wall_cursor, dir)) = delete_flinch_fixture(&action) else {
            continue;
        };
        assert_eq!(
            drive_effect(text, ok_cursor, &action),
            Effect::DeleteSquash,
            "{action:?}: a delete that removes a char must squash"
        );
        assert_eq!(
            drive_effect(text, wall_cursor, &action),
            Effect::Recoil(dir),
            "{action:?}: a delete with nothing to remove must recoil {dir:?}"
        );
    }
}

#[test]
fn copy_with_selection_arms_the_copy_pulse() {
    // M-w / Cmd-C over a NON-EMPTY selection: the caret gets a gentle pulse
    // and the selection quad brightens (`Effect::CopyPulse`) — copy's one
    // common, otherwise-invisible action finally gets in-world feedback. The
    // document itself is untouched (copy never edits).
    let mut b = Buffer::from_str("copy me");
    b.set_mark();
    b.set_cursor(4); // "copy" selected
    assert_eq!(drive_act_effect(&mut b, &Action::CopyRegion), Effect::CopyPulse);
    assert_eq!(b.text(), "copy me", "copy leaves the document unchanged");
    assert!(!b.has_selection(), "copy_region still clears the mark as before");
}

#[test]
fn copy_without_selection_does_not_pulse() {
    // No mark at all: M-w is the pre-existing documented no-op (nothing
    // selected, nothing to copy) — it must NOT gain a pulse.
    let mut b = Buffer::from_str("nothing selected");
    assert_eq!(drive_act_effect(&mut b, &Action::CopyRegion), Effect::None);

    // A mark set exactly AT the cursor (an EMPTY region, `anchor == cursor`)
    // is the same documented no-op — `has_selection()` is false either way.
    let mut b2 = Buffer::from_str("nothing selected");
    b2.set_mark();
    assert_eq!(drive_act_effect(&mut b2, &Action::CopyRegion), Effect::None);
}

#[test]
fn cut_does_not_arm_the_copy_pulse() {
    // C-w / KillRegion has a VISIBLE result (the text vanishes) — it must
    // never arm the copy pulse, even over an active selection identical to
    // the one that just armed it above.
    let mut b = Buffer::from_str("cut me");
    b.set_mark();
    b.set_cursor(3);
    assert_eq!(drive_act_effect(&mut b, &Action::KillRegion), Effect::None);
    assert_eq!(b.text(), " me", "the cut actually removed the selected text");
}

#[test]
fn line_edge_motions_recoil_at_the_edge_and_move_off_it() {
    // BOUNDARY BUMP: C-a at col 0 / C-e at line end are common idempotent
    // presses, but a silent no-op still reads as "nothing happened" rather than
    // "you're at the edge" — so they now bump the caret, quiet like every other
    // wall (a superseded decision; see `recoil_for`'s doc). Off the edge they
    // still just move (no recoil).
    use crate::caret::RecoilDir::{Left, Right};
    assert_eq!(drive_effect("abc", 0, &Action::LineStart), Effect::Recoil(Right));
    assert_eq!(drive_effect("abc", 3, &Action::LineEnd), Effect::Recoil(Left));
    assert_eq!(drive_effect("abc", 1, &Action::LineStart), Effect::None);
    assert_eq!(drive_effect("abc", 0, &Action::LineEnd), Effect::None);
}

#[test]
fn boundary_motions_bump_only_when_blocked() {
    // BOUNDARY BUMP completeness sweep, over `all_actions()`'s compile-time-complete
    // enumeration (the SAME gate `every_classified_motion_extends_shift_selection_
    // and_no_mover_is_missing` uses below): for every `Action::is_motion` variant,
    // the motion BLOCKED at its wall (`motion_boundary_fixture`) must recoil AND
    // leave the cursor exactly where it was, while the SAME motion driven from a
    // mid-document position with no wall in any direction (cursor 14 in the fixture
    // below — the shift-selection sweep already proves every motion actually moves
    // the point from there) must NOT recoil and must actually move the cursor. This
    // pins "the bump fires only when the motion did not move the cursor" at the
    // `apply_core` seam, for every wall on both sides.
    let no_wall = "alpha beta\ngamma delta\nepsilon zeta\n";
    for action in all_actions() {
        if !action.is_motion() {
            continue;
        }
        let (wall_text, wall_cursor, dir) = motion_boundary_fixture(&action);
        let (blocked_effect, blocked_cursor) =
            drive_effect_and_cursor(wall_text, wall_cursor, &action);
        assert_eq!(
            blocked_effect,
            Effect::Recoil(dir),
            "{action:?}: blocked at its wall must recoil {dir:?}"
        );
        assert_eq!(
            blocked_cursor, wall_cursor,
            "{action:?}: a recoil must not move the cursor"
        );

        let (unblocked_effect, unblocked_cursor) =
            drive_effect_and_cursor(no_wall, 14, &action);
        assert_ne!(
            unblocked_effect,
            Effect::Recoil(dir),
            "{action:?}: an unblocked motion must not bump"
        );
        assert_ne!(
            unblocked_cursor, 14,
            "{action:?}: an unblocked motion must actually move the cursor"
        );
    }
}
