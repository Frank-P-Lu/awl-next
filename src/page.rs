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

/// Default PROSE measure (column width in characters). ~70 sits squarely in
/// Butterick's 45–90 comfort band (≈2.5 lowercase alphabets); 80 read a touch
/// wide (just past 3 alphabets). Sticky + adjustable via Page wider/narrower.
/// See [`PageClass`]: this is the `Prose` class's own built-in default —
/// `page_width_prose` in config (`crate::config::Config::page_width_prose`).
pub const DEFAULT_MEASURE: usize = 70;

/// Default CODE measure (column width in characters) — rustfmt's own
/// `max_width` convention (the settled call: code reads comfortably wider
/// than prose's 70-char comfort band, and already wraps at this width by
/// convention). See [`PageClass::Code`] — `page_width_code` in config
/// (`crate::config::Config::page_width_code`).
pub const DEFAULT_MEASURE_CODE: usize = 100;

/// Which STICKY page-width MEASURE a buffer draws its column width from — the
/// 70-char prose comfort measure is a PROSE number (Butterick's line-length
/// band), and code wants its own, wider convention. Two independent config
/// keys (`page_width_prose` / `page_width_code`, `crate::config::Config`) each
/// persist their class's override; [`Self::default_measure`] is the built-in
/// fallback when a class has none.
///
/// THE ONE CLASSIFIER: [`Self::of_syntax`] — a recognized CODE language means
/// `Code`; `None` (markdown, the no-path scratch/quick-note surface, or an
/// unrecognized plain-text file like `.txt`/`.env`) means `Prose`. Both
/// `crate::buffer::Buffer::page_class` (the live/headless buffer) and
/// `crate::render::TextPipeline::page_class` (the sidecar, driven by the
/// pipeline's own shaped `syn_lang`) delegate here, so the two can never
/// disagree about which class a document belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PageClass {
    /// Markdown, the no-path scratch/quick-note surface, or an unrecognized
    /// plain-text file — everything [`crate::syntax::Lang::from_path`] does
    /// NOT recognize as a code language.
    Prose,
    /// A recognized CODE file (`Buffer::syntax_lang().is_some()`).
    Code,
}

impl PageClass {
    /// THE classifier, driven by a resolved (or absent) syntax-highlighting
    /// language — mirrors `Buffer::syntax_lang`'s own code/not-code gate
    /// exactly, so page-width class and syntax highlighting can never
    /// disagree about what counts as "code".
    pub fn of_syntax(syn_lang: Option<crate::syntax::Lang>) -> Self {
        match syn_lang {
            Some(_) => PageClass::Code,
            None => PageClass::Prose,
        }
    }

    /// Classify a FILE PATH the same way, for the ONE call site that must
    /// decide a class before any `Buffer` exists: the initial sticky-measure
    /// apply at launch (`crate::config::Config::apply_sticky_globals`,
    /// called from `main::args` with the launch file argument). `None` (no
    /// file / a bare launch) is always `Prose`, matching `Buffer::is_markdown`'s
    /// own no-path default.
    pub fn of_path(path: Option<&std::path::Path>) -> Self {
        Self::of_syntax(path.and_then(crate::syntax::Lang::from_path))
    }

    /// This class's own BUILT-IN default measure, used when its config key
    /// (`page_width_prose`/`page_width_code`) is unset — [`DEFAULT_MEASURE`]
    /// for `Prose`, [`DEFAULT_MEASURE_CODE`] for `Code`.
    pub fn default_measure(self) -> usize {
        match self {
            PageClass::Prose => DEFAULT_MEASURE,
            PageClass::Code => DEFAULT_MEASURE_CODE,
        }
    }
}

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
    fn defaults_on_at_seventy() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_page_on(true);
        set_measure(DEFAULT_MEASURE);
        assert!(page_on());
        assert_eq!(measure(), 70);
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

    // ── PageClass (prose/code page-width split) ─────────────────────────────

    #[test]
    fn of_syntax_classifies_code_vs_prose() {
        assert_eq!(PageClass::of_syntax(Some(crate::syntax::Lang::Rust)), PageClass::Code);
        assert_eq!(PageClass::of_syntax(None), PageClass::Prose);
    }

    #[test]
    fn of_path_classifies_by_recognized_extension() {
        use std::path::Path;
        assert_eq!(PageClass::of_path(Some(Path::new("/a/main.rs"))), PageClass::Code);
        // Markdown, an unrecognized extension, and no path at all are all Prose.
        assert_eq!(PageClass::of_path(Some(Path::new("/a/notes.md"))), PageClass::Prose);
        assert_eq!(PageClass::of_path(Some(Path::new("/a/notes.txt"))), PageClass::Prose);
        assert_eq!(PageClass::of_path(None), PageClass::Prose);
    }

    #[test]
    fn default_measure_per_class() {
        assert_eq!(PageClass::Prose.default_measure(), DEFAULT_MEASURE);
        assert_eq!(PageClass::Code.default_measure(), DEFAULT_MEASURE_CODE);
        assert_eq!(DEFAULT_MEASURE, 70);
        assert_eq!(DEFAULT_MEASURE_CODE, 100);
    }
}
