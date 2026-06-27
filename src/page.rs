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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // The globals are process-wide; serialize the mutating tests.
    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn defaults_on_at_eighty() {
        let _g = LOCK.lock().unwrap();
        set_page_on(true);
        set_measure(DEFAULT_MEASURE);
        assert!(page_on());
        assert_eq!(measure(), 80);
    }

    #[test]
    fn toggle_flips_on_off() {
        let _g = LOCK.lock().unwrap();
        set_page_on(true);
        assert!(!toggle()); // on -> off
        assert!(!page_on());
        assert!(toggle()); // off -> on
        assert!(page_on());
        set_page_on(true);
    }

    #[test]
    fn measure_floor_is_one() {
        let _g = LOCK.lock().unwrap();
        set_measure(0);
        assert_eq!(measure(), 1);
        set_measure(DEFAULT_MEASURE);
    }
}
