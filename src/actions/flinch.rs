//! The CARET FEEDBACK triggers — the pure decisions that turn a just-applied
//! `Action` into a visual caret flinch. Both read the cheap pre/post snapshots
//! `apply_core` takes around the dispatch (the cursor char index, the content
//! version, the undo/redo availability) and return the ONE [`Effect`] the caller
//! carries out on the VISUAL caret. They are mutually exclusive — a blocked
//! action recoils ([`recoil_for`]); a successful edit flinches ([`impact_for`]) —
//! and `apply_core` only consults each when no real effect already fired. Pure
//! over the buffer + the snapshots, so the triggers are unit-testable without a
//! GPU/clock. Carved out of `actions.rs` VERBATIM.

use super::*;

/// Decide whether `action` was a SUCCESSFUL edit that should FLINCH the visual caret,
/// and which flinch: a typed CHARACTER → [`Effect::TypeImpact`] (squash-pop + a
/// back-kick recoil), a BACKSPACE / C-d / word-delete → [`Effect::DeleteSquash`] (an
/// inward squash, the caret swallowing what it ate), a C-k KILL-LINE → [`Effect::Gulp`]
/// (a bigger swallow). Returns `None` for a non-edit OR an edit that did NOT change the
/// buffer — that no-op case is the blocked-action recoil's job (handled before this).
/// Pure over the buffer + the pre-action version snapshot, so the trigger is
/// unit-testable without a GPU/clock.
///
/// Only a single typed CHARACTER flinches as TYPING — a NEWLINE / TAB reflow and a bulk
/// YANK are structural relocations, not a keystroke thunk, so they are OMITTED (and a
/// settled capture is byte-identical regardless of which arm fires, since every flinch
/// decays to the same resting caret).
pub(super) fn impact_for(action: &Action, version_before: u64, ctx: &ActionCtx) -> Option<Effect> {
    if ctx.buffer.version() == version_before {
        return None; // nothing changed -> not a successful edit (no flinch)
    }
    match action {
        Action::InsertChar(_) => Some(Effect::TypeImpact),
        Action::DeleteBackward | Action::DeleteForward | Action::DeleteWordBackward => {
            Some(Effect::DeleteSquash)
        }
        Action::KillLine => Some(Effect::Gulp),
        _ => None,
    }
}

/// Decide whether `action` was BLOCKED (requested but unable to proceed) and, if
/// so, which way the visual caret should RECOIL — the direction AWAY from the wall
/// it couldn't cross. Returns `None` when the action proceeded normally (the common
/// case). Pure over the buffer + the pre-action snapshot, so the trigger logic is
/// unit-testable without a GPU/clock.
///
/// "Blocked" is read from the SAME signal in every case — nothing observable
/// changed:
///   * a directional MOTION left the cursor char index unchanged (hit the buffer
///     edge / a line wall) — C-f/C-b/C-n/C-p/M-</M-> and the word motions;
///   * a PAGE scroll left the cursor unchanged (already at top/bottom);
///   * an UNDO/REDO had nothing in its history;
///   * a DELETE with nothing to remove left the content version unchanged
///     (backspace at buffer start, C-d at buffer end).
///
/// LINE-EDGE motions (C-a/C-e) are deliberately OMITTED: pressing them when already
/// at the edge is an extremely common idempotent gesture (e.g. C-a C-a), so a bump
/// there would be noisy rather than informative.
pub(super) fn recoil_for(
    action: &Action,
    ctx: &ActionCtx,
    cursor_before: usize,
    version_before: u64,
    could_undo: bool,
    could_redo: bool,
) -> Option<crate::caret::RecoilDir> {
    use crate::caret::RecoilDir::{Down, Left, Right, Up};
    let cursor_stuck = ctx.buffer.cursor_char() == cursor_before;
    let content_stuck = ctx.buffer.version() == version_before;
    match action {
        // Horizontal motion into a wall -> bump back the way it came.
        Action::ForwardChar | Action::ForwardWord if cursor_stuck => Some(Left),
        Action::BackwardChar | Action::BackwardWord if cursor_stuck => Some(Right),
        // Vertical motion into the top/bottom wall -> bump away from it.
        Action::NextLine if cursor_stuck => Some(Up),
        Action::PreviousLine if cursor_stuck => Some(Down),
        // Buffer ends (M-> / M-<) already at the end -> bump back toward the middle.
        Action::BufferEnd if cursor_stuck => Some(Up),
        Action::BufferStart if cursor_stuck => Some(Down),
        // Page scroll that can't page further (cursor already at top/bottom). NOTE:
        // the windowed app intercepts PgUp/PgDn for its GPU-measured paging and
        // bumps the caret there; this arm covers the core/replay/no-GPU path.
        Action::PageScrollDown if cursor_stuck => Some(Up),
        Action::PageScrollUp if cursor_stuck => Some(Down),
        // Exhausted undo / redo (nothing in history). Mirrored horizontal bump
        // (undo "rewinds" left, redo "advances" right) — there is no spatial wall,
        // so the direction is a tasteful convention, not a geometry.
        Action::Undo if !could_undo => Some(Left),
        Action::Redo if !could_redo => Some(Right),
        // Delete with nothing to remove (backspace at start / C-d at end): the
        // content version never bumped, so the edit was a no-op.
        Action::DeleteBackward | Action::DeleteWordBackward if content_stuck => Some(Right),
        Action::DeleteForward if content_stuck => Some(Left),
        _ => None,
    }
}
