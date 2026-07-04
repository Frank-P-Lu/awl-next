//! src/pointer_hide.rs — the OS pointer auto-hides while you type (the
//! macOS-native convention, `NSCursor.setHiddenUntilMouseMoves`): the FIRST
//! real keystroke hides the cursor immediately; any mouse motion brings it
//! back instantly.
//!
//! A small, PURE two-state enum plus pure transition functions, unit-testable
//! without a window. The live `App` alone feeds it real events (a keystroke,
//! a `CursorMoved`, a window blur) and makes the actual `set_cursor_visible`
//! OS call, gated through [`os_visibility_change`] so it only ever fires on
//! an actual boundary crossing.
//!
//! Determinism: this is LIVE-APP-ONLY. The headless capture has no window and
//! no OS pointer to hide — nothing here is reachable from the capture path,
//! so a `--screenshot` stays byte-identical with no new sidecar field (there
//! is nothing deterministic to report: the OS cursor is not rendered).
//!
//! v2 (2026-07-04, supersedes the original ~3s-of-typing spec): the earlier
//! version armed a countdown timer (`Armed`, `HIDE_AFTER`, an `about_to_wait`
//! timeout branch) so the hide fired some quiet period after typing started.
//! That machinery is gone — matching TextEdit/Xcode/any `NSTextView`, a
//! keystroke hides the pointer on the spot, and the state machine collapses
//! to just `Visible`/`Hidden` with no clock of its own. A window blur is now
//! also a hard reset to `Visible` (never leave the pointer hidden across a
//! focus change) — that reset lives at the call site (`app.rs`'s existing
//! blur hook), not here, since it needs no new transition of its own (it's
//! the same "force Visible" shape as `on_mouse_move`).

/// The pointer auto-hide state: OS pointer shown, or OS pointer hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerHide {
    /// OS pointer shown — the resting state, and the state any mouse motion
    /// or window blur snaps back to.
    Visible,
    /// The OS pointer is currently hidden. Stays hidden through further
    /// typing (typing alone never un-hides — only mouse motion, or losing
    /// focus, does).
    Hidden,
}

/// A real keystroke landed (past the lone-modifier/IME filters at the call
/// site) — hides the pointer immediately, regardless of the prior state
/// (idempotent: already-`Hidden` stays `Hidden`).
pub fn on_key(_state: PointerHide) -> PointerHide {
    PointerHide::Hidden
}

/// ANY mouse motion: always snaps back to `Visible` — un-hides an
/// already-hidden pointer, and is a harmless no-op if already `Visible`.
pub fn on_mouse_move(_state: PointerHide) -> PointerHide {
    PointerHide::Visible
}

/// Whether a `prev -> next` transition should change the OS pointer's actual
/// visibility, and to what — the ONE place that decides "make the real
/// `set_cursor_visible` call", so `App` never has to re-derive it ad hoc at
/// each call site (the same single-owner discipline as `syn_role_color` /
/// `debounce_due`). `None` = no OS call needed (the transition didn't cross
/// the hidden/visible boundary).
pub fn os_visibility_change(prev: PointerHide, next: PointerHide) -> Option<bool> {
    match (prev, next) {
        (PointerHide::Visible, PointerHide::Hidden) => Some(false),
        (PointerHide::Hidden, PointerHide::Visible) => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_hides_from_visible() {
        assert_eq!(on_key(PointerHide::Visible), PointerHide::Hidden);
    }

    #[test]
    fn key_while_hidden_stays_hidden() {
        assert_eq!(on_key(PointerHide::Hidden), PointerHide::Hidden);
    }

    #[test]
    fn mouse_motion_unhides_and_is_idempotent_when_visible() {
        assert_eq!(on_mouse_move(PointerHide::Hidden), PointerHide::Visible);
        assert_eq!(on_mouse_move(PointerHide::Visible), PointerHide::Visible);
    }

    #[test]
    fn retyping_after_a_move_rehides() {
        let hidden = on_key(PointerHide::Visible);
        let shown = on_mouse_move(hidden);
        assert_eq!(shown, PointerHide::Visible);
        assert_eq!(on_key(shown), PointerHide::Hidden);
    }

    #[test]
    fn os_visibility_change_only_crosses_the_boundary() {
        // Hiding crosses the boundary: Some(false).
        assert_eq!(
            os_visibility_change(PointerHide::Visible, PointerHide::Hidden),
            Some(false)
        );
        // Un-hiding crosses it the other way: Some(true).
        assert_eq!(
            os_visibility_change(PointerHide::Hidden, PointerHide::Visible),
            Some(true)
        );
        // Already-settled re-checks are no-ops (no repeated OS calls).
        assert_eq!(
            os_visibility_change(PointerHide::Hidden, PointerHide::Hidden),
            None
        );
        assert_eq!(
            os_visibility_change(PointerHide::Visible, PointerHide::Visible),
            None
        );
    }
}
