//! src/debug.rs — DEBUG-mode state (the renamed FPS mode).
//!
//! An opt-in, DEBUG-only developer panel drawn quietly in the top-left corner
//! (dim, value-only — NO amber per DESIGN §3; amber is the caret's alone). It is
//! OFF by default and exists to spot lag / frame starvation AND to surface the
//! buffer's diagnostic state (zoom, viewport, cursor, theme/caret/page mode, and
//! the key md/syn line) under heavy machine load or while debugging styling.
//!
//! One process-global mirrors the `page`/`focus`/`caret` pattern so the runtime
//! toggle (palette "Toggle Debug" / `C-x r`), the headless `--debug` flag, and a
//! config rebind all write the SAME place without threading a config through the
//! pipeline:
//!   * `DEBUG_ON` — whether the corner panel is drawn (DEFAULT OFF).
//!
//! Determinism: the panel's frametime LINE comes from a live frame clock the
//! headless capture does not have. [`readout`] folds that in — given a real
//! `frame_ms` it shows the live timing, but given `None` (the capture path: no
//! clock) it renders a FIXED PLACEHOLDER. Every OTHER line (zoom, viewport, cursor,
//! theme/caret/page, md/syn) is a pure function of the deterministic view state, so
//! it renders identically in a capture. The render pipeline only draws anything at
//! all when [`debug_on`] is true, so a default `--screenshot` (debug off) is
//! BYTE-IDENTICAL; only an explicit `--debug` capture shows the (fixed-clock)
//! placeholder line plus the deterministic diagnostics.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the debug panel is drawn. DEFAULT OFF: the calm room shows no debug
/// chrome until you ask for it (palette / `C-x r` / `--debug`).
static DEBUG_ON: AtomicBool = AtomicBool::new(false);

/// True when the debug panel is enabled.
pub fn debug_on() -> bool {
    DEBUG_ON.load(Ordering::Relaxed)
}

/// Set the panel on/off explicitly (the `--debug` flag, a config/setting write).
pub fn set_debug_on(on: bool) {
    DEBUG_ON.store(on, Ordering::Relaxed);
}

/// Flip the panel and return the now-active state (the `C-x r` chord + palette
/// "Toggle Debug").
pub fn toggle() -> bool {
    let next = !debug_on();
    DEBUG_ON.store(next, Ordering::Relaxed);
    next
}

/// The FRAMETIME line for the debug panel, given the latest measured frame time.
///
/// Pure (so it is unit-testable without a window): a real `frame_ms` becomes a
/// live `"<n> fps · <ms> ms"` line, while `None` (no live clock — the headless
/// capture, or before the first measured frame) becomes a FIXED PLACEHOLDER with
/// no numbers, keeping a clockless render deterministic.
pub fn readout(frame_ms: Option<f32>) -> String {
    match frame_ms {
        Some(ms) if ms > 0.0 => {
            let fps = (1000.0 / ms).round() as i64;
            format!("{fps} fps · {ms:.1} ms")
        }
        // No (positive) measured frame time: the capture path has no clock, so a
        // numberless placeholder keeps the line present but deterministic.
        _ => "fps · — ms".to_string(),
    }
}

/// Serializes EVERY test that reads or writes the DEBUG global, ACROSS modules — the
/// flag is process-wide, so a `render`/`capture` test asserting the panel is drawn
/// (or absent) must not race a test flipping it. `pub(crate)` so those tests can
/// hold the same lock. Mirrors `page::TEST_LOCK`.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_off() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_debug_on(false);
        assert!(!debug_on(), "the debug panel is OFF by default");
    }

    #[test]
    fn toggle_flips_on_off() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_debug_on(false);
        assert!(toggle()); // off -> on
        assert!(debug_on());
        assert!(!toggle()); // on -> off
        assert!(!debug_on());
        set_debug_on(false);
    }

    #[test]
    fn readout_is_fixed_placeholder_without_a_clock() {
        // No frame time (the headless capture path) => a fixed, numberless string,
        // so a clockless render stays byte-deterministic.
        assert_eq!(readout(None), "fps · — ms");
        // A non-positive dt is treated as "no measurement" too.
        assert_eq!(readout(Some(0.0)), "fps · — ms");
    }

    #[test]
    fn readout_reports_live_timing() {
        // ~16.7ms/frame => 60 fps.
        assert_eq!(readout(Some(16.6667)), "60 fps · 16.7 ms");
        // ~33.3ms/frame => 30 fps (the starvation we want to spot).
        assert_eq!(readout(Some(33.3333)), "30 fps · 33.3 ms");
    }
}
