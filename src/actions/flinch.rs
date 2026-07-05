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
/// (a bigger swallow), ENTER → [`Effect::LineLand`] (PHASE 3 — a caret-level
/// "touchdown" squash as it takes the new line; the markdown smart-Enter's
/// continue/end-block edits ride the same arm, keyed off the `Action`, not which
/// branch fired). Returns `None` for a non-edit OR an edit that did NOT change the
/// buffer — that no-op case is the blocked-action recoil's job (handled before this).
/// Pure over the buffer + the pre-action version snapshot, so the trigger is
/// unit-testable without a GPU/clock.
///
/// A TAB reflow and a bulk YANK are structural relocations, not a keystroke thunk, so
/// they stay OMITTED (a settled capture is byte-identical regardless of which arm
/// fires, since every flinch decays to the same resting caret).
pub(super) fn impact_for(action: &Action, version_before: u64, ctx: &ActionCtx) -> Option<Effect> {
    if ctx.buffer.version() == version_before {
        return None; // nothing changed -> not a successful edit (no flinch)
    }
    match action {
        Action::InsertChar(_) => Some(Effect::TypeImpact),
        Action::DeleteBackward
        | Action::DeleteForward
        | Action::DeleteWordBackward
        | Action::DeleteWordForward => Some(Effect::DeleteSquash),
        Action::KillLine => Some(Effect::Gulp),
        Action::Newline => Some(Effect::LineLand),
        _ => None,
    }
}

/// Decide whether `action` was a SUCCESSFUL COPY (M-w / Cmd-C) of a NON-EMPTY
/// selection, in which case the caller plays the COPY PULSE
/// ([`Effect::CopyPulse`]) — the caret's own gentle kick plus the selection
/// quad's brighten-then-decay tint. Copy is the one common action with an
/// otherwise INVISIBLE result, so it earns its own trigger distinct from
/// [`impact_for`] above: `Action::CopyRegion` never mutates the buffer, so it can
/// never pass `impact_for`'s content-version-changed gate — a separate,
/// selection-keyed check is the only way to see it. `had_selection_before` MUST be
/// snapshotted by the caller BEFORE `apply_core` dispatches the action:
/// `Buffer::copy_region` unconditionally clears the mark (even on a no-op copy
/// with nothing selected), so reading the selection AFTER the call always reads
/// false. An empty-selection copy (no mark, or mark == cursor) stays the
/// pre-existing, documented no-op: no pulse, matching "M-w with nothing selected
/// does nothing". Pure; unit-testable without a GPU/clock.
pub(super) fn copy_pulse_for(action: &Action, had_selection_before: bool) -> Option<Effect> {
    match action {
        Action::CopyRegion if had_selection_before => Some(Effect::CopyPulse),
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
///   * a LINE-EDGE motion (C-a/C-e, Cmd-Left/Right) left the cursor unchanged
///     (already at the line's start/end);
///   * a PAGE scroll left the cursor unchanged (already at top/bottom);
///   * an UNDO/REDO had nothing in its history;
///   * a DELETE with nothing to remove left the content version unchanged
///     (backspace at buffer start, C-d at buffer end).
///
/// BOUNDARY BUMP: every motion that CAN hit a wall decides its bump here — a
/// silent no-op reads as the editor ignoring the key, not as "you're at the
/// edge". (LINE-EDGE was once deliberately omitted as "too idempotent to bump",
/// but a quiet, no-sound/no-color bump reads calm even on a repeated C-a C-a, so
/// it now joins every other wall.) See the sibling completeness sweep in
/// `actions::tests` (`boundary_motions_bump_only_when_blocked`), which enumerates
/// every `Action::is_motion` variant via the same gate as the shift-selection
/// sweep, so a NEW motion can't silently ship without deciding its bump.
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
        // Line-edge motion already at the edge (C-a/C-e, Cmd-Left/Right) -> bump
        // back the way it came, same convention as the char/word walls above.
        Action::LineStart if cursor_stuck => Some(Right),
        Action::LineEnd if cursor_stuck => Some(Left),
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
        // Delete with nothing to remove (backspace at start / C-d / M-d at end):
        // the content version never bumped, so the edit was a no-op. Backward
        // deletes bump Right (away from the start wall), forward ones bump Left.
        Action::DeleteBackward | Action::DeleteWordBackward if content_stuck => Some(Right),
        Action::DeleteForward | Action::DeleteWordForward if content_stuck => Some(Left),
        _ => None,
    }
}
