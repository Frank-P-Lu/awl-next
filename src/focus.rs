//! src/focus.rs — FOCUS MODE state (iA Writer-style "dim everything but here").
//!
//! Focus mode keeps the ACTIVE UNIT around the cursor at full ink and renders the
//! rest of the document in a muted token, so the eye rests on the sentence /
//! paragraph being written. It is a pure RENDER concern — no buffer change — so it
//! mirrors the `page`/`caret` process-global pattern: the runtime toggle
//! (`C-x d` for "dim"), the command palette ("Focus mode"), and the headless flag
//! (`--focus off|paragraph|sentence`) all write the SAME atomic without threading a
//! config through the pipeline. The render pipeline reads [`mode`] each frame.
//!
//! The active UNIT is computed from the cursor's char index over the document text
//! by [`active_range`], which delegates to the pure boundary helpers in `buffer`
//! (blank-line-delimited paragraph; `.`/`!`/`?`-delimited sentence). The brighten/
//! dim crossfade as the cursor moves to a new unit is a LIVE-ONLY animation owned by
//! the render pipeline; the headless capture renders the SETTLED state (active full,
//! rest dim) with no clock, per CAPTURE.md.

use std::sync::atomic::{AtomicU8, Ordering};

/// How much the non-active (dim) text leans toward the muted token. 1.0 = the full
/// `muted` ink; lower values keep the dim text closer to full ink (a
/// gentler fade). Kept as a const so the dim strength is one dial.
pub const FOCUS_DIM_STRENGTH: f32 = 1.0;

/// Seconds for the brighten/dim crossfade when the cursor enters a new unit. Short
/// and calm — the awl touch. LIVE ONLY; the capture path renders the settled state.
pub const FOCUS_FADE_SECS: f32 = 0.18;

/// Which granularity focus mode is dimming at. A process-global, like the active
/// theme / page mode, so every call site reads the same value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusMode {
    /// No dimming — the whole document renders at full ink (the unchanged default).
    Off,
    /// The cursor's PARAGRAPH (the run of non-blank lines around it, delimited by
    /// blank lines) stays full; everything else dims.
    Paragraph,
    /// The cursor's SENTENCE (`.`/`!`/`?` + whitespace/EOF delimited) stays full;
    /// the rest of even its own paragraph dims.
    Sentence,
}

impl FocusMode {
    fn as_u8(self) -> u8 {
        match self {
            FocusMode::Off => 0,
            FocusMode::Paragraph => 1,
            FocusMode::Sentence => 2,
        }
    }
    fn from_u8(v: u8) -> Self {
        match v {
            1 => FocusMode::Paragraph,
            2 => FocusMode::Sentence,
            _ => FocusMode::Off,
        }
    }
    /// The lowercase wire name used by the `--focus` flag and the sidecar.
    pub fn name(self) -> &'static str {
        match self {
            FocusMode::Off => "off",
            FocusMode::Paragraph => "paragraph",
            FocusMode::Sentence => "sentence",
        }
    }
}

/// The active focus granularity. DEFAULT Off: the app opens with the whole document
/// at full ink (unchanged), and the toggle dims around the cursor.
static FOCUS_MODE: AtomicU8 = AtomicU8::new(0);

/// The SINGLE test mutex serializing every test that mutates the process-global
/// [`FOCUS_MODE`] — colocated with the global so focus's own tests AND the render
/// tests that flip the mode hold the same lock (a second, private mutex would let
/// cargo's parallel runner race one global). Mirrors `page::TEST_LOCK`.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The current focus mode.
pub fn mode() -> FocusMode {
    FocusMode::from_u8(FOCUS_MODE.load(Ordering::Relaxed))
}

/// Set the focus mode explicitly (the `--focus` flag, a settings write).
pub fn set_mode(m: FocusMode) {
    FOCUS_MODE.store(m.as_u8(), Ordering::Relaxed);
}

/// Cycle Off -> Paragraph -> Sentence -> Off and return the now-active mode (the
/// `C-x d` chord + the "Focus mode" palette entry).
pub fn cycle() -> FocusMode {
    let next = match mode() {
        FocusMode::Off => FocusMode::Paragraph,
        FocusMode::Paragraph => FocusMode::Sentence,
        FocusMode::Sentence => FocusMode::Off,
    };
    set_mode(next);
    next
}

/// The DIM ink for non-active text: `base_content` leaned toward `muted`
/// by [`FOCUS_DIM_STRENGTH`]. The single dial for "how dim is the surrounding text".
pub fn dim_srgb() -> crate::theme::Srgb {
    let full = crate::theme::base_content();
    let dim = crate::theme::muted();
    let t = FOCUS_DIM_STRENGTH.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    crate::theme::Srgb::rgb(mix(full.r, dim.r), mix(full.g, dim.g), mix(full.b, dim.b))
}

/// The char range `[start, end)` of the ACTIVE UNIT around `cursor_char` for the
/// given `mode`, computed over the document `text`. `None` when focus is Off (the
/// caller then renders the whole document at full ink). The Paragraph / Sentence
/// boundary math lives in `buffer` so the render path and the sidecar share it.
pub fn active_range(text: &str, cursor_char: usize, mode: FocusMode) -> Option<(usize, usize)> {
    match mode {
        FocusMode::Off => None,
        FocusMode::Paragraph => Some(crate::buffer::paragraph_bounds_str(text, cursor_char)),
        FocusMode::Sentence => Some(crate::buffer::sentence_bounds_str(text, cursor_char)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_off() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_mode(FocusMode::Off);
        assert_eq!(mode(), FocusMode::Off);
    }

    #[test]
    fn cycle_off_paragraph_sentence_off() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_mode(FocusMode::Off);
        assert_eq!(cycle(), FocusMode::Paragraph);
        assert_eq!(cycle(), FocusMode::Sentence);
        assert_eq!(cycle(), FocusMode::Off);
        set_mode(FocusMode::Off);
    }

    #[test]
    fn off_has_no_active_range() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(active_range("a. b. c.", 3, FocusMode::Off), None);
    }

    #[test]
    fn dim_srgb_is_full_dim_ink_at_strength_one() {
        // FOCUS_DIM_STRENGTH ships at 1.0, so the dim ink must equal the theme's
        // muted exactly (the lerp lands fully on the target). Hold the
        // theme lock so a concurrent theme-switch test can't move the global between
        // the two reads.
        let _g = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(FOCUS_DIM_STRENGTH, 1.0, "this test assumes the shipped strength");
        assert_eq!(dim_srgb(), crate::theme::muted());
    }

    #[test]
    fn active_range_delegates_to_buffer_bounds() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let text = "First sentence. Second one.\n\nSecond paragraph here.";
        let idx = 3; // inside the first sentence / first paragraph
        assert_eq!(
            active_range(text, idx, FocusMode::Paragraph),
            Some(crate::buffer::paragraph_bounds_str(text, idx)),
            "Paragraph delegates to paragraph_bounds_str"
        );
        assert_eq!(
            active_range(text, idx, FocusMode::Sentence),
            Some(crate::buffer::sentence_bounds_str(text, idx)),
            "Sentence delegates to sentence_bounds_str"
        );
    }
}
