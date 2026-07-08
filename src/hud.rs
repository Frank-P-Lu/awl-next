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

/// The FIXED, numberless placeholder a LIFETIME-ODOMETER row renders in a headless
/// capture (no live App, so no persisted `Stats` to read), mirroring the fps
/// counter's `"fps · — ms"` and the retired session-time/file-created rows. A real
/// odometer reading only ever appears in a live window.
pub const PLACEHOLDER: &str = "—";

/// The whimsy conversion behind the CARET TRAVEL row: how many on-screen pixels
/// the caret must travel to have crossed one METRE of "desk". A nominal 96 CSS px
/// per inch (the OS/web reference density) × 39.3701 inches per metre ≈ 3779.5
/// px/metre — a deliberately playful, not-scientific figure (there is no real
/// physical distance a caret moves), documented so the "you've walked the caret
/// 3.4 km" readout is reproducible rather than a magic number.
pub const CARET_PX_PER_METRE: f64 = 3779.5;

/// The LIFETIME-ODOMETER figures the held HUD shows, as RAW values snapshotted
/// from the live App's [`crate::stats::Stats`] store. The live App pushes `Some`
/// into the pipeline every `sync_view` (`App::stats_sync_hud`); the headless
/// capture never calls that seam, so the pipeline field stays `None` and every
/// odometer row renders the fixed [`PLACEHOLDER`] — the SAME determinism boundary
/// the retired session-time/file-created rows used. Formatting lives in the pure
/// (unit-testable) helpers below, NOT on the raw values, so the whimsy math is
/// tested with no window.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HudStats {
    /// Lifetime printable characters written (`Stats::chars_typed`).
    pub chars_typed: u64,
    /// Lifetime honest active-writing time, millis (`Stats::active_writing_ms`).
    pub active_writing_ms: u64,
    /// Distinct files ever opened (`Stats::files_touched_count`).
    pub files_touched: usize,
    /// Lifetime caret travel in document pixels (`Stats::caret_distance_px`).
    pub caret_distance_px: f64,
    /// The most-lived-in theme world's name (`Stats::most_used_world`), or `None`
    /// when nothing has accrued yet (a fresh install) → the row shows the placeholder.
    pub world: Option<String>,
}

/// Group a count with thousands separators — `1234567` → `"1,234,567"`. The
/// CHARACTERS / FILES TOUCHED figures read as a human odometer, not a bare int.
pub fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// The TIME WRITING readout: honest active-writing millis as a compact
/// `"45s"` / `"12m"` / `"47h 12m"` line (the same shape the retired session-time
/// row used). A lifetime total can be hundreds of hours, so the hour form has no
/// upper bound.
pub fn writing_time_readout(ms: u64) -> String {
    let secs = ms / 1000;
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

/// The CARET TRAVEL readout: document-pixel distance as a fun metric distance —
/// `"820 m"` under a kilometre (rounded to the metre), `"3.4 km"` at or above one
/// (one decimal). Uses [`CARET_PX_PER_METRE`]. Never negative (the accumulator
/// only grows); a finite non-negative `px` is assumed.
pub fn caret_travel_readout(px: f64) -> String {
    let metres = px / CARET_PX_PER_METRE;
    if metres < 1000.0 {
        format!("{} m", metres.round() as u64)
    } else {
        format!("{:.1} km", metres / 1000.0)
    }
}

/// The FIVE lifetime-odometer `(caption, value)` rows — the ONE owner of the
/// odometer's HUD presentation, called by BOTH `prepare_hud` (the pixels) and
/// `hud_report` (the sidecar), so the two can never drift. `Some` formats each
/// raw figure through the pure helpers above; `None` (the capture path — no live
/// store) folds every row to the fixed [`PLACEHOLDER`], keeping a `--hud` capture
/// deterministic and byte-stable across machines.
pub fn odometer_rows(stats: Option<&HudStats>) -> [(&'static str, String); 5] {
    match stats {
        Some(s) => [
            ("CHARACTERS", group_thousands(s.chars_typed)),
            ("TIME WRITING", writing_time_readout(s.active_writing_ms)),
            ("FILES TOUCHED", group_thousands(s.files_touched as u64)),
            ("CARET TRAVEL", caret_travel_readout(s.caret_distance_px)),
            (
                "YOUR WORLD",
                s.world.clone().unwrap_or_else(|| PLACEHOLDER.to_string()),
            ),
        ],
        None => [
            ("CHARACTERS", PLACEHOLDER.to_string()),
            ("TIME WRITING", PLACEHOLDER.to_string()),
            ("FILES TOUCHED", PLACEHOLDER.to_string()),
            ("CARET TRAVEL", PLACEHOLDER.to_string()),
            ("YOUR WORLD", PLACEHOLDER.to_string()),
        ],
    }
}

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

    #[test]
    fn group_thousands_inserts_separators() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(42), "42");
        assert_eq!(group_thousands(1_000), "1,000");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn writing_time_readout_matches_the_compact_shape() {
        assert_eq!(writing_time_readout(45_000), "45s");
        assert_eq!(writing_time_readout(12 * 60_000), "12m");
        // 47h 12m — a lifetime total, no upper bound on hours.
        assert_eq!(writing_time_readout((47 * 3600 + 12 * 60) * 1000), "47h 12m");
        // The minutes are zero-padded in the hour form.
        assert_eq!(writing_time_readout((1 * 3600 + 4 * 60) * 1000), "1h 04m");
    }

    #[test]
    fn caret_travel_readout_switches_metres_to_kilometres() {
        // Under a kilometre: rounded metres.
        assert_eq!(caret_travel_readout(820.0 * CARET_PX_PER_METRE), "820 m");
        assert_eq!(caret_travel_readout(0.0), "0 m");
        // At/over a kilometre: one decimal.
        assert_eq!(caret_travel_readout(3_400.0 * CARET_PX_PER_METRE), "3.4 km");
        assert_eq!(caret_travel_readout(1_000.0 * CARET_PX_PER_METRE), "1.0 km");
    }

    #[test]
    fn odometer_rows_none_is_all_placeholder() {
        let rows = odometer_rows(None);
        assert_eq!(rows.len(), 5);
        for (_, v) in &rows {
            assert_eq!(v, PLACEHOLDER, "no live store => every odometer row is the placeholder");
        }
        assert_eq!(rows[0].0, "CHARACTERS");
        assert_eq!(rows[4].0, "YOUR WORLD");
    }

    #[test]
    fn odometer_rows_some_formats_each_figure() {
        let s = HudStats {
            chars_typed: 1_234_567,
            active_writing_ms: (47 * 3600 + 12 * 60) * 1000,
            files_touched: 42,
            caret_distance_px: 3_400.0 * CARET_PX_PER_METRE,
            world: Some("Tawny".to_string()),
        };
        let rows = odometer_rows(Some(&s));
        assert_eq!(rows[0], ("CHARACTERS", "1,234,567".to_string()));
        assert_eq!(rows[1], ("TIME WRITING", "47h 12m".to_string()));
        assert_eq!(rows[2], ("FILES TOUCHED", "42".to_string()));
        assert_eq!(rows[3], ("CARET TRAVEL", "3.4 km".to_string()));
        assert_eq!(rows[4], ("YOUR WORLD", "Tawny".to_string()));
    }

    #[test]
    fn odometer_rows_some_without_a_world_placeholders_only_that_row() {
        let s = HudStats { world: None, ..HudStats::default() };
        let rows = odometer_rows(Some(&s));
        assert_eq!(rows[4], ("YOUR WORLD", PLACEHOLDER.to_string()));
        // The other rows still show their (zeroed) real values, not the placeholder.
        assert_eq!(rows[0].1, "0");
    }
}
