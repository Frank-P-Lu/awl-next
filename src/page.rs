//! src/page.rs — PAGE MODE state.
//!
//! Page mode lays the document out as a CENTERED writing column (Typora-style)
//! with a maximum MEASURE in characters, so prose never stretches edge-to-edge
//! on a wide / fullscreen window. The extra window width becomes MARGINS on both
//! sides, which the [`crate::background`] shader paints with a per-world gradient
//! so the calm flat page reads as a clean shape FLOATING on a styled ground.
//!
//! Two process-globals mirror the `theme`/`caret` pattern so the runtime toggle
//! (`C-x w`), the command palette ("Toggle page mode"), and the headless flags
//! (`--page on|off`, `--measure N`) all write the same place without threading a
//! config through the pipeline:
//!   * `PAGE_ON`  — whether the centered column is active (DEFAULT ON).
//!   * `MEASURE`  — the column's maximum width in characters (DEFAULT 80).
//!
//! The render pipeline reads these each frame via [`page_on`] / [`measure`] to
//! derive the column's pixel left + width (see `TextPipeline::column_left` /
//! `column_width`), so flipping either re-wraps + re-centers the text.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Default measure (column width in characters). ~80 is the classic prose line.
pub const DEFAULT_MEASURE: usize = 80;

/// The step (in characters) the "Page wider" / "Page narrower" commands move the
/// measure by. A calm nudge — a few keypresses spans the usable band.
pub const MEASURE_STEP: usize = 4;

/// The usable band the wider/narrower commands stay within. The floor keeps the
/// column from collapsing to an unreadable sliver; the ceiling keeps prose from
/// stretching back toward edge-to-edge (which is the page toggle's job, not the
/// width's). Hand-edited config values are still honoured verbatim (only the
/// COMMANDS clamp), and the render column additionally caps to the window.
pub const MIN_MEASURE: usize = 20;
pub const MAX_MEASURE: usize = 140;

/// Whether page mode (the centered capped column) is active. DEFAULT ON: the app
/// opens with the calm centered column; the toggle drops to edge-to-edge.
static PAGE_ON: AtomicBool = AtomicBool::new(true);

/// The column's maximum width in CHARACTERS. The pixel width is this times the
/// (zoomed) glyph advance, clamped to the window. Tunable via `--measure N` and
/// reachable as a setting; the render pipeline reads it each frame.
static MEASURE: AtomicUsize = AtomicUsize::new(DEFAULT_MEASURE);

/// True when the centered column is active.
pub fn page_on() -> bool {
    PAGE_ON.load(Ordering::Relaxed)
}

/// Set page mode on/off explicitly (the `--page on|off` flag, a settings write).
pub fn set_page_on(on: bool) {
    PAGE_ON.store(on, Ordering::Relaxed);
}

/// Flip page mode and return the now-active state (the `C-x w` chord + palette).
pub fn toggle() -> bool {
    let next = !page_on();
    PAGE_ON.store(next, Ordering::Relaxed);
    next
}

/// The column measure (characters). Floored at 1 so the column never collapses.
pub fn measure() -> usize {
    MEASURE.load(Ordering::Relaxed).max(1)
}

/// Set the column measure in characters (the `--measure N` flag, a setting).
/// Setting a measure does NOT itself enable page mode; callers that want the
/// column visible also call [`set_page_on`].
pub fn set_measure(chars: usize) {
    MEASURE.store(chars.max(1), Ordering::Relaxed);
}

/// Widen the page by one [`MEASURE_STEP`] (the "Page wider" command), clamped to
/// [`MAX_MEASURE`], and return the now-active measure so the caller can persist it.
/// Zoom-independent: this changes the PAGE geometry (more chars per line at the same
/// glyph size), the settable counterpart to zoom's glyph-only scaling.
pub fn widen() -> usize {
    let next = (measure() + MEASURE_STEP).min(MAX_MEASURE);
    set_measure(next);
    next
}

/// Narrow the page by one [`MEASURE_STEP`] (the "Page narrower" command), clamped to
/// [`MIN_MEASURE`], and return the now-active measure so the caller can persist it.
pub fn narrow() -> usize {
    let next = measure().saturating_sub(MEASURE_STEP).max(MIN_MEASURE);
    set_measure(next);
    next
}

/// Serializes EVERY test that reads or writes the page globals, ACROSS modules.
/// Page mode is a process-wide `AtomicBool`/`AtomicUsize`, so a `render` test
/// reading `column_width()` (which folds `page_on()`/`measure()`) must not race
/// a page test flipping them mid-shape, or the two diverge non-deterministically.
/// `pub(crate)` so the render geometry tests can hold the same lock.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_on_at_eighty() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_page_on(true);
        set_measure(DEFAULT_MEASURE);
        assert!(page_on());
        assert_eq!(measure(), 80);
    }

    #[test]
    fn toggle_flips_on_off() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_page_on(true);
        assert!(!toggle()); // on -> off
        assert!(!page_on());
        assert!(toggle()); // off -> on
        assert!(page_on());
        set_page_on(true);
    }

    #[test]
    fn measure_floor_is_one() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_measure(0);
        assert_eq!(measure(), 1);
        set_measure(DEFAULT_MEASURE);
    }

    #[test]
    fn widen_narrow_step_the_measure_and_report_it() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_measure(DEFAULT_MEASURE); // 80
        assert_eq!(widen(), DEFAULT_MEASURE + MEASURE_STEP);
        assert_eq!(measure(), DEFAULT_MEASURE + MEASURE_STEP);
        assert_eq!(narrow(), DEFAULT_MEASURE); // back to start
        assert_eq!(measure(), DEFAULT_MEASURE);
        set_measure(DEFAULT_MEASURE);
    }

    #[test]
    fn widen_narrow_clamp_to_the_band() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Narrowing bottoms out at MIN_MEASURE, widening tops out at MAX_MEASURE.
        set_measure(MIN_MEASURE);
        assert_eq!(narrow(), MIN_MEASURE, "never below the floor");
        set_measure(MAX_MEASURE);
        assert_eq!(widen(), MAX_MEASURE, "never above the ceiling");
        // A sub-floor start snaps UP to the floor on narrow (max(.., MIN)).
        set_measure(5);
        assert_eq!(narrow(), MIN_MEASURE);
        set_measure(DEFAULT_MEASURE);
    }
}
