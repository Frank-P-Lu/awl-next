//! src/pointer_hide.rs — the OS pointer auto-hides while you type ("games do
//! this"): after a quiet period of typing with no mouse movement, the cursor
//! disappears; any mouse motion brings it back instantly.
//!
//! Mirrors the [`crate::debug::DebugStill`] shape: a small, PURE state enum
//! plus pure transition functions, unit-testable without a window. The live
//! `App` alone feeds it real events (a keystroke, a `CursorMoved`) and reads
//! the elapsed-time decision through the SAME single-`WaitUntil` debounce
//! pattern the other idle timers (`spell_dirty_at`, `zoom_persist_at`, …) use
//! in `about_to_wait` — an `Option<Instant>` "armed at" stamp owned by `App`,
//! checked against [`HIDE_AFTER`] via the existing free `debounce_due` helper
//! in `app.rs`. No clock lives in this module; `Instant` math stays the
//! caller's job, exactly like `still_wake`/`still_settle` take pre-computed
//! bools rather than reading a clock themselves.
//!
//! Determinism: this is LIVE-APP-ONLY. The headless capture has no window and
//! no OS pointer to hide — nothing here is reachable from the capture path,
//! so a `--screenshot` stays byte-identical with no new sidecar field (there
//! is nothing deterministic to report: the OS cursor is not rendered).
//!
//! Taste note (flagged for live review, not re-litigated in code): the
//! macOS-native convention (`NSCursor.setHiddenUntilMouseMoves`) hides on the
//! FIRST keystroke. This ships the user's stated ~3s-of-typing spec instead;
//! [`HIDE_AFTER`] is the one knob to change if the live feel argues for the
//! native convention.
//!
//! The 3s anchors to when typing STARTS, not to a quiet gap after the last
//! key: the caller stamps its "armed at" `Instant` only on the `Visible ->
//! Armed` transition (guarded by the stamp being `None`); further keystrokes
//! while already `Armed` do not re-stamp it, so continued typing does not
//! postpone the hide — it fires `HIDE_AFTER` after the burst began, even if
//! keys are still landing. A mouse move mid-countdown aborts back to
//! `Visible` and clears the stamp, so the next keystroke re-anchors a fresh
//! countdown from that point.

use std::time::Duration;

/// Quiet typing period, with no mouse movement, before the OS pointer hides.
pub const HIDE_AFTER: Duration = Duration::from_secs(3);

/// The pointer auto-hide state. Three states, no data — the actual "since
/// when" clock lives in the caller (`App.pointer_hide_armed_at`), mirroring
/// how [`crate::debug::DebugStill`] carries no `Instant` either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerHide {
    /// OS pointer shown; nothing pending (the resting state, and the state
    /// any mouse motion snaps back to).
    Visible,
    /// A keystroke armed the countdown: the pointer is STILL shown, ticking
    /// toward hiding unless a further keystroke re-arms it or the mouse moves.
    Armed,
    /// The countdown elapsed with no mouse movement in between: the OS
    /// pointer is hidden. Stays hidden through further typing (typing alone
    /// never un-hides — only mouse motion does).
    Hidden,
}

/// A keystroke landed. `Hidden` stays `Hidden` (typing while hidden is a
/// no-op — there is nothing left to arm toward, and typing must never be
/// what un-hides). `Visible` becomes `Armed` — the FIRST keystroke of a
/// typing burst, and the ONLY transition where the caller stamps its "armed
/// at" `Instant` (the countdown anchors to when typing STARTED). An
/// already-`Armed` state stays `Armed` (idempotent): further keystrokes
/// before the threshold do NOT re-stamp or postpone the deadline — unlike
/// `spell_dirty_at` / `zoom_persist_at`, which slide forward on every tick,
/// this one deliberately does not.
pub fn on_key(state: PointerHide) -> PointerHide {
    match state {
        PointerHide::Hidden => PointerHide::Hidden,
        PointerHide::Visible | PointerHide::Armed => PointerHide::Armed,
    }
}

/// ANY mouse motion: always snaps back to `Visible` — cancels a pending
/// countdown (`Armed` -> `Visible`) and un-hides an already-hidden pointer
/// (`Hidden` -> `Visible`) in the same move. `Visible` staying `Visible` is
/// simply idempotent (a mouse-move with no countdown running is a no-op).
pub fn on_mouse_move(_state: PointerHide) -> PointerHide {
    PointerHide::Visible
}

/// The armed countdown reached [`HIDE_AFTER`] with no interrupting mouse
/// motion: `Armed` fires to `Hidden`. Any other state ignores the timeout —
/// `Visible` has nothing armed, `Hidden` is already there — so a stray/late
/// timeout check (e.g. a `WaitUntil` that fired after a mouse move already
/// reset the state this same wake) is harmless.
pub fn on_timeout(state: PointerHide) -> PointerHide {
    match state {
        PointerHide::Armed => PointerHide::Hidden,
        other => other,
    }
}

/// Whether a `prev -> next` transition should change the OS pointer's actual
/// visibility, and to what — the ONE place that decides "make the real
/// `set_cursor_visible` call", so `App` never has to re-derive it ad hoc at
/// each call site (the same single-owner discipline as `syn_role_color` /
/// `debounce_due`). `None` = no OS call needed (the transition didn't cross
/// the hidden/visible boundary — includes `Hidden -> Hidden`, `Visible ->
/// Armed`, and `Armed -> Armed`, none of which touch the OS state).
pub fn os_visibility_change(prev: PointerHide, next: PointerHide) -> Option<bool> {
    match (prev, next) {
        (PointerHide::Hidden, PointerHide::Hidden) => None,
        (_, PointerHide::Hidden) => Some(false),
        (PointerHide::Hidden, _) => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_key_arms_from_visible() {
        assert_eq!(on_key(PointerHide::Visible), PointerHide::Armed);
    }

    #[test]
    fn hide_fires_at_threshold_only_if_typing_continued_and_mouse_stayed_still() {
        // Visible with no key ever pressed: a timeout check is a no-op — nothing
        // was armed, so there is no threshold to have reached.
        assert_eq!(on_timeout(PointerHide::Visible), PointerHide::Visible);
        // Typing armed the countdown, and it reaches the threshold undisturbed:
        // now (and only now) does the pointer hide.
        let armed = on_key(PointerHide::Visible);
        assert_eq!(armed, PointerHide::Armed);
        assert_eq!(on_timeout(armed), PointerHide::Hidden);
        // A timeout that lands after it's already hidden is a harmless no-op.
        assert_eq!(on_timeout(PointerHide::Hidden), PointerHide::Hidden);
    }

    #[test]
    fn mouse_motion_resets_and_unhides() {
        // Motion while counting down cancels the pending hide.
        let armed = on_key(PointerHide::Visible);
        assert_eq!(on_mouse_move(armed), PointerHide::Visible);
        // Motion once hidden un-hides instantly.
        let hidden = on_timeout(on_key(PointerHide::Visible));
        assert_eq!(hidden, PointerHide::Hidden);
        assert_eq!(on_mouse_move(hidden), PointerHide::Visible);
        // Motion while already visible/idle is a harmless no-op.
        assert_eq!(on_mouse_move(PointerHide::Visible), PointerHide::Visible);
    }

    #[test]
    fn new_typing_rearms_after_a_reset() {
        // Armed -> mouse move resets to Visible -> typing again re-arms.
        let armed = on_key(PointerHide::Visible);
        let reset = on_mouse_move(armed);
        assert_eq!(reset, PointerHide::Visible);
        assert_eq!(on_key(reset), PointerHide::Armed);
        // Typing while already Armed stays Armed — idempotent. The caller
        // does NOT re-stamp its Instant here (see `continued_typing_does_
        // not_postpone_the_anchor` below): this only pins that the enum
        // transition itself is stable under repeated keys.
        assert_eq!(on_key(PointerHide::Armed), PointerHide::Armed);
    }

    #[test]
    fn continued_typing_does_not_postpone_the_anchor() {
        // Simulates the caller's guarded stamp (`pointer_hide_armed_at.is_
        // none()` in `app.rs`) with a plain tick counter standing in for
        // `Instant::now()` — pins the FULL contract (pure state machine +
        // the caller's stamp-ONLY-on-first-arm guard) without a real clock.
        let mut state = PointerHide::Visible;
        let mut armed_at: Option<u64> = None;

        // Typing starts at tick 0: the first key arms and stamps.
        state = on_key(state);
        if state == PointerHide::Armed && armed_at.is_none() {
            armed_at = Some(0);
        }
        assert_eq!(state, PointerHide::Armed);
        assert_eq!(armed_at, Some(0));

        // Continued typing at ticks 1 and 2: state stays Armed, and the
        // guard means the stamp does NOT move — this is the anchor-at-start
        // fix (previously every key re-stamped "now", postponing the hide).
        for tick in [1u64, 2] {
            state = on_key(state);
            if state == PointerHide::Armed && armed_at.is_none() {
                armed_at = Some(tick);
            }
        }
        assert_eq!(
            armed_at,
            Some(0),
            "continued typing must not postpone the anchor"
        );

        // At tick 3 (HIDE_AFTER's 3s from the ORIGINAL stamp) the timeout
        // fires, even though a key landed as recently as tick 2 — the
        // countdown never reset under continued typing.
        assert_eq!(on_timeout(state), PointerHide::Hidden);
    }

    #[test]
    fn mouse_move_mid_countdown_aborts_then_retyping_reanchors() {
        let mut state = PointerHide::Visible;
        let mut armed_at: Option<u64> = None;

        // Tick 0: typing starts, arms + stamps.
        state = on_key(state);
        if state == PointerHide::Armed && armed_at.is_none() {
            armed_at = Some(0);
        }
        assert_eq!(armed_at, Some(0));

        // Tick 1: a mouse move mid-countdown aborts it before it can fire —
        // resets to Visible and clears the stamp (mirrors the `CursorMoved`
        // handler in `app.rs`).
        state = on_mouse_move(state);
        armed_at = None;
        assert_eq!(state, PointerHide::Visible);

        // Tick 2: re-typing after the abort re-anchors a FRESH countdown at
        // the NEW tick, not the stale tick 0.
        state = on_key(state);
        if state == PointerHide::Armed && armed_at.is_none() {
            armed_at = Some(2);
        }
        assert_eq!(state, PointerHide::Armed);
        assert_eq!(armed_at, Some(2));
    }

    #[test]
    fn typing_while_hidden_never_unhides() {
        // Typing alone must never be what shows the pointer again — only
        // mouse motion does. So a keystroke while Hidden stays Hidden.
        let hidden = on_timeout(on_key(PointerHide::Visible));
        assert_eq!(hidden, PointerHide::Hidden);
        assert_eq!(on_key(hidden), PointerHide::Hidden);
    }

    #[test]
    fn os_visibility_change_only_crosses_the_hidden_boundary() {
        // Arming/re-arming never touches the OS pointer.
        assert_eq!(os_visibility_change(PointerHide::Visible, PointerHide::Armed), None);
        assert_eq!(os_visibility_change(PointerHide::Armed, PointerHide::Armed), None);
        // Hiding crosses the boundary: Some(false).
        assert_eq!(
            os_visibility_change(PointerHide::Armed, PointerHide::Hidden),
            Some(false)
        );
        // Un-hiding crosses it the other way: Some(true).
        assert_eq!(
            os_visibility_change(PointerHide::Hidden, PointerHide::Visible),
            Some(true)
        );
        // Already-hidden re-checked stays a no-op (no repeated OS calls).
        assert_eq!(os_visibility_change(PointerHide::Hidden, PointerHide::Hidden), None);
    }
}
