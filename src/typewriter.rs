//! TYPEWRITER SCROLL — pin the caret's row at a fixed vertical position (centered)
//! so the document scrolls UNDER a stationary caret, the scroll counterpart of
//! focus mode / iA Writer's typing line. Unlike the default cursor-FOLLOW (nudge
//! the viewport just enough to keep the caret visible), typewriter scroll always
//! re-derives the scroll so the caret's visual row lands in the middle of the
//! viewport — EXCEPT clamped at the document edges (near the top the caret can't
//! be centered because there aren't enough rows above, so the scroll stays at 0;
//! near the bottom it clamps to `max_scroll_rows`), the standard typewriter feel.
//!
//! This module owns ONLY the process-global on/off flag (DEFAULT OFF — opt-in,
//! like the margin outline). The PIN GEOMETRY is pure and lives on the render
//! pipeline (`TextPipeline::scroll_to_center_row`, reused verbatim from focus
//! mode's typewriter fold — no parallel math); the DECISION of when to center is
//! `app::viewstate::follow_scroll_strategy` (which now folds this flag alongside
//! focus mode). Mirrors the [`crate::outline`] / [`crate::markdown::wysiwyg_on`]
//! global shape exactly:
//!
//!   * [`TYPEWRITER_ON`] — whether typewriter scroll pins the caret row (DEFAULT OFF).
//!   * [`typewriter_on`] / [`set_typewriter_on`] / [`toggle`] — the readers/writers.
//!
//! Set once at launch from the config sticky pref (`config::typewriter_scroll`, via
//! `Config::apply_sticky_globals`), flipped live by the "Typewriter Scroll" command
//! (`Action::ToggleTypewriter`) and the settings menu. The pin is a PURE function of
//! the caret's visual row + the viewport + document height — no clock — so unlike an
//! animation a `--keys` capture with typewriter ON renders the settled pinned scroll
//! deterministically (see `capture::modes`). A default `--screenshot` (typewriter
//! OFF) keeps the exact cursor-follow scroll → byte-identical.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether typewriter scroll pins the caret row centered. DEFAULT OFF — the calm
/// room scrolls with the ordinary cursor-follow until you ask for the pinned line
/// (palette "Typewriter Scroll" / settings menu / config `typewriter_scroll = true`).
static TYPEWRITER_ON: AtomicBool = AtomicBool::new(false);

/// True when typewriter scroll is enabled (read by `sync_view`'s cursor-follow +
/// by the capture scroll computation, so the live app and a headless capture can
/// never disagree about whether the caret row is pinned).
pub fn typewriter_on() -> bool {
    TYPEWRITER_ON.load(Ordering::Relaxed)
}

/// Set typewriter scroll on/off explicitly — the config sticky-pref launch-apply
/// (`Config::apply_sticky_globals`) and the settings-menu toggle. Mirrors
/// [`crate::outline::set_outline_on`] / [`crate::markdown::set_wysiwyg_on`].
pub fn set_typewriter_on(on: bool) {
    TYPEWRITER_ON.store(on, Ordering::Relaxed);
}

/// Flip typewriter scroll and return the now-active state (the palette
/// "Typewriter Scroll" command / a rebound chord). Mirrors [`crate::outline::toggle`].
pub fn toggle() -> bool {
    let next = !typewriter_on();
    TYPEWRITER_ON.store(next, Ordering::Relaxed);
    next
}

/// Serializes tests that read or write the process-global [`TYPEWRITER_ON`],
/// mirroring [`crate::outline::TEST_LOCK`] / [`crate::markdown::TEST_LOCK`].
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typewriter_scroll_is_off_by_default_and_toggles() {
        let _g = TEST_LOCK.lock().unwrap();
        set_typewriter_on(false);
        assert!(!typewriter_on(), "typewriter scroll is OFF by default");
        assert!(toggle(), "toggle turns it on and reports the new state");
        assert!(typewriter_on());
        assert!(!toggle(), "toggle turns it back off");
        assert!(!typewriter_on());
        set_typewriter_on(false);
    }
}
