//! src/fps.rs — DEBUG frame-counter state.
//!
//! An opt-in, DEBUG-only FPS / frame-time readout drawn quietly in a corner (dim,
//! value-only — NO amber per DESIGN §3; amber is the caret's alone). It is OFF by
//! default and exists to spot lag / frame starvation under heavy machine load.
//!
//! One process-global mirrors the `page`/`focus`/`caret` pattern so the runtime
//! toggle (palette "Toggle FPS" / `C-x r`), the headless `--fps` flag, and a
//! config rebind all write the SAME place without threading a config through the
//! pipeline:
//!   * `FPS_ON` — whether the corner readout is drawn (DEFAULT OFF).
//!
//! Determinism: the readout's TEXT comes from a live frame clock the headless
//! capture does not have. [`readout`] folds that in — given a real `frame_ms` it
//! shows the live timing, but given `None` (the capture path: no clock) it renders
//! a FIXED PLACEHOLDER. The render pipeline only draws anything at all when
//! [`fps_on`] is true, so a default `--screenshot` (FPS off) is BYTE-IDENTICAL;
//! only an explicit `--fps` capture shows the (fixed, clockless) placeholder.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the debug frame counter is drawn. DEFAULT OFF: the calm room shows no
/// debug chrome until you ask for it (palette / `C-x r` / `--fps`).
static FPS_ON: AtomicBool = AtomicBool::new(false);

/// True when the debug frame counter is enabled.
pub fn fps_on() -> bool {
    FPS_ON.load(Ordering::Relaxed)
}

/// Set the counter on/off explicitly (the `--fps` flag, a config/setting write).
pub fn set_fps_on(on: bool) {
    FPS_ON.store(on, Ordering::Relaxed);
}

/// Flip the counter and return the now-active state (the `C-x r` chord + palette
/// "Toggle FPS").
pub fn toggle() -> bool {
    let next = !fps_on();
    FPS_ON.store(next, Ordering::Relaxed);
    next
}

/// The readout STRING for the corner, given the latest measured frame time.
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
        // numberless placeholder keeps the readout present but deterministic.
        _ => "fps · — ms".to_string(),
    }
}

/// Serializes EVERY test that reads or writes the FPS global, ACROSS modules — the
/// flag is process-wide, so a `render`/`capture` test asserting the readout is
/// drawn (or absent) must not race a test flipping it. `pub(crate)` so those tests
/// can hold the same lock. Mirrors `page::TEST_LOCK`.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_off() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_fps_on(false);
        assert!(!fps_on(), "the debug counter is OFF by default");
    }

    #[test]
    fn toggle_flips_on_off() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_fps_on(false);
        assert!(toggle()); // off -> on
        assert!(fps_on());
        assert!(!toggle()); // on -> off
        assert!(!fps_on());
        set_fps_on(false);
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
