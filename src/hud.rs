//! src/hud.rs — the SUMMONED-WHILE-HELD stats HUD state.
//!
//! A calm metadata panel that appears WHILE a key is HELD and dismisses the moment
//! it is released — the "hold to peek the map" affordance from games, applied to a
//! prose editor. It floats centered over a dimmed document (a value step back, like
//! a full-takeover overlay; DESIGN §5) and shows a few quiet figures about the file
//! and the session, then vanishes. It is NOT persistent chrome: you summon it, you
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
//! Determinism: the HUD shows two CLOCK / filesystem-time fields — the SESSION TIME
//! and the file's CREATED date — that the headless capture has no clock to know.
//! [`session_readout`] folds that in exactly like the fps counter: a real `elapsed`
//! becomes a live `"12m"` line, but `None` (the capture path: no clock) renders the
//! FIXED [`PLACEHOLDER`]. The file-created date is read from the filesystem only in
//! the live app and is likewise placeholdered in a capture, so the sidecar stays
//! byte-stable across machines. The word count and the %-through-doc figures are a
//! pure function of the text and ARE shown in a capture.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Whether the held stats HUD is drawn. DEFAULT OFF: the calm room shows no HUD
/// until you HOLD the key (the live binding / `--hud` / `--keys "Cmd-I"`).
static HUD_HELD: AtomicBool = AtomicBool::new(false);

/// The FIXED, numberless placeholder a clock / filesystem-time field renders in a
/// headless capture (no clock), mirroring the fps counter's `"fps · — ms"`. A real
/// reading only ever appears in a live window.
pub const PLACEHOLDER: &str = "—";

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

/// The SESSION-TIME readout, given the live elapsed time since the editing session
/// began.
///
/// Pure (so it is unit-testable without a window): a real `elapsed` becomes a
/// compact `"45s"` / `"12m"` / `"1h 04m"` line, while `None` (no live clock — the
/// headless capture) becomes the FIXED [`PLACEHOLDER`], keeping a clockless render
/// deterministic.
pub fn session_readout(elapsed: Option<Duration>) -> String {
    let Some(d) = elapsed else {
        return PLACEHOLDER.to_string();
    };
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        format!("{h}h {m:02}m")
    }
}

/// Format a Unix timestamp (whole seconds since 1970-01-01 UTC) as a calendar
/// `"YYYY-MM-DD"` date — the FILE CREATED figure's live value.
///
/// Pure + std-only (awl has no date crate): the day count is converted to a civil
/// `(year, month, day)` via Howard Hinnant's well-known `civil_from_days` algorithm,
/// so it is leap-year correct without pulling in `chrono`. The clock half of a
/// timestamp is dropped (a created DATE, not a time). Used by the live window only —
/// the headless capture never reads a file's date (it shows the placeholder), so
/// this stays unit-tested rather than exercised through a capture.
pub fn civil_date(epoch_secs: u64) -> String {
    let days = (epoch_secs / 86_400) as i64;
    // Shift the epoch to 0000-03-01 so leap days land at the end of the 400-year era.
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as i64; // day-of-era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // year-of-era [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day-of-year (Mar 1 = 0)
    let mp = (5 * doy + 2) / 153; // month, shifted so March = 0
    let d = doy - (153 * mp + 2) / 5 + 1; // day [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // month [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
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

    #[test]
    fn session_is_fixed_placeholder_without_a_clock() {
        // No elapsed time (the headless capture path) => the fixed, numberless
        // placeholder, so a clockless render stays byte-deterministic.
        assert_eq!(session_readout(None), PLACEHOLDER);
    }

    #[test]
    fn civil_date_is_leap_year_correct() {
        // The Unix epoch itself.
        assert_eq!(civil_date(0), "1970-01-01");
        // One day later.
        assert_eq!(civil_date(86_400), "1970-01-02");
        // 2000-02-29 exists (a leap year): 2000-02-29 00:00:00 UTC = 951_782_400.
        assert_eq!(civil_date(951_782_400), "2000-02-29");
        // 2021-03-01 00:00:00 UTC = 1_614_556_800 (the day after a NON-leap Feb).
        assert_eq!(civil_date(1_614_556_800), "2021-03-01");
        // The clock half is dropped — any second within a day yields that day.
        assert_eq!(civil_date(86_400 + 3661), "1970-01-02");
    }

    #[test]
    fn session_reports_live_elapsed_compactly() {
        assert_eq!(session_readout(Some(Duration::from_secs(0))), "0s");
        assert_eq!(session_readout(Some(Duration::from_secs(45))), "45s");
        assert_eq!(session_readout(Some(Duration::from_secs(60))), "1m");
        assert_eq!(session_readout(Some(Duration::from_secs(12 * 60 + 30))), "12m");
        assert_eq!(session_readout(Some(Duration::from_secs(3600))), "1h 00m");
        assert_eq!(session_readout(Some(Duration::from_secs(3600 + 4 * 60))), "1h 04m");
    }
}
