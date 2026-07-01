//! src/hud.rs — the SUMMONED-WHILE-HELD stats HUD state.
//!
//! A calm metadata panel that appears WHILE a key is HELD and dismisses the moment
//! it is released — the "hold to peek the map" affordance from games, applied to a
//! prose editor. It floats centered over a document that recedes behind the shared
//! FROSTED-BLUR backdrop (the same hue-preserving frost the command palette uses; NOT
//! a grey scrim), and shows a few quiet WRITER figures — word count + reading time and
//! %-through-doc — then vanishes. It is NOT persistent chrome: you summon it, you
//! glance, you let go.
//!
//! One process-global mirrors the `page`/`focus`/`fps` pattern so the three doors
//! all write the SAME place without threading a config through the pipeline:
//!   * `HUD_HELD` — whether the held stats panel is drawn (DEFAULT OFF / released).
//!
//! The live window sets it true on the binding's key PRESS and false on its RELEASE
//! (a true hold); the headless `--hud` flag / a `--keys "Cmd-I"` replay set it true
//! for the single captured frame (there is no release in a replay), so a capture
//! renders the SETTLED held HUD. A default capture (key not held) draws nothing and
//! is byte-identical.
//!
//! Determinism: every HUD figure is now a PURE function of the document + cursor (the
//! former clock / filesystem-time fields — session time and the file's created date —
//! were dropped as incidental fluff), so a `--hud` capture is deterministic and
//! byte-stable across machines with no clockless placeholders to fold in.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the held stats HUD is drawn. DEFAULT OFF: the calm room shows no HUD
/// until you HOLD the key (the live binding / `--hud` / `--keys "Cmd-I"`).
static HUD_HELD: AtomicBool = AtomicBool::new(false);

/// True when the held stats HUD is currently summoned (the key is held).
pub fn hud_held() -> bool {
    HUD_HELD.load(Ordering::Relaxed)
}

/// Set the HUD held/released explicitly. The live window calls this with `true` on
/// the binding's key PRESS and `false` on its RELEASE; the `--hud` flag passes
/// `true` for a settled capture.
pub fn set_held(held: bool) {
    HUD_HELD.store(held, Ordering::Relaxed);
}

/// Serializes EVERY test that reads or writes the HUD global, ACROSS modules — the
/// flag is process-wide, so a `render`/`capture` test asserting the HUD is drawn (or
/// absent) must not race a test flipping it. `pub(crate)` so those tests can hold
/// the same lock. Mirrors `debug::TEST_LOCK`.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_released() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_held(false);
        assert!(!hud_held(), "the stats HUD is released (off) by default");
    }

    #[test]
    fn set_held_drives_the_flag() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_held(false);
        set_held(true); // press
        assert!(hud_held());
        set_held(false); // release
        assert!(!hud_held());
    }
}
