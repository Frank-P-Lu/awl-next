//! The persistent MARGIN OUTLINE — the ambient table-of-contents that lingers
//! in the page margin so the document's structure stays oriented without a
//! summoned picker (PHILOSOPHY.md's "orientation lingers in two margin
//! surfaces" amendment). This module owns the process-global on/off flag; the
//! heading DATA rides the markdown parse the styling pass already pays for
//! (`markdown::headings_from_spans` → `TextPipeline::outline_headings`), and the
//! RENDER lands in a later phase.
//!
//! DEFAULT OFF — unlike the other sticky toggles (WYSIWYG / spellcheck / nits),
//! the outline is opt-in ambient chrome. Mirrors the [`crate::debug`] /
//! [`crate::markdown::wysiwyg_on`] global shape exactly:
//!
//!   * [`OUTLINE_ON`] — whether the margin outline is drawn (DEFAULT OFF).
//!   * [`outline_on`] / [`set_outline_on`] / [`toggle`] — the readers/writers.
//!
//! Set once at launch from the config sticky pref (`config::outline`, via
//! `Config::apply_sticky_globals`), flipped live by the "Toggle Outline" command
//! (`Action::ToggleOutline`) and the settings menu. The render reads
//! [`outline_on`] each reshape, so a default `--screenshot` (outline OFF) is
//! byte-identical.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the persistent margin outline is drawn. DEFAULT OFF — the calm room
/// shows no outline chrome until you ask for it (palette / `Cmd-Shift-O` /
/// config `outline = true`).
static OUTLINE_ON: AtomicBool = AtomicBool::new(false);

/// True when the margin outline is enabled (read by the renderer each reshape
/// + by the capture sidecar's `outline` block, so the two can never disagree).
pub fn outline_on() -> bool {
    OUTLINE_ON.load(Ordering::Relaxed)
}

/// Set the outline on/off explicitly — the config sticky-pref launch-apply
/// (`Config::apply_sticky_globals`) and the settings-menu toggle. Mirrors
/// [`crate::debug::set_debug_on`] / [`crate::markdown::set_wysiwyg_on`].
pub fn set_outline_on(on: bool) {
    OUTLINE_ON.store(on, Ordering::Relaxed);
}

/// Flip the outline and return the now-active state (the `Cmd-Shift-O` chord +
/// palette "Toggle Outline"). Mirrors [`crate::debug::toggle`].
pub fn toggle() -> bool {
    let next = !outline_on();
    OUTLINE_ON.store(next, Ordering::Relaxed);
    next
}

/// Serializes tests that read or write the process-global [`OUTLINE_ON`],
/// mirroring [`crate::markdown::TEST_LOCK`] / [`crate::nits::TEST_LOCK`].
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_is_off_by_default_and_toggles() {
        let _g = TEST_LOCK.lock().unwrap();
        set_outline_on(false);
        assert!(!outline_on(), "the margin outline is OFF by default");
        assert!(toggle(), "toggle turns it on and reports the new state");
        assert!(outline_on());
        assert!(!toggle(), "toggle turns it back off");
        assert!(!outline_on());
        set_outline_on(false);
    }
}
