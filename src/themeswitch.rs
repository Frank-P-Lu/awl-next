//! src/themeswitch.rs — THE THEME-SWITCH SETTLE-LATENCY readout (DEBUG-mode,
//! LIVE-ONLY): a once-per-switch measurement of how long a theme change takes to
//! SETTLE on screen, plus a per-phase breakdown so the dominant cost NAMES ITSELF
//! instead of being guessed.
//!
//! WHAT IT REPORTS (drawn as two extra lines in the debug panel, `debug.rs`, only
//! after a real switch has been measured):
//!   * `theme settled N ms` — the FELT latency: from the input event that triggered
//!     the switch (the preview arrow re-stamps it; a direct switch stamps it at the
//!     retint) to the SETTLED present (the frame that carried the reshaped document
//!     to the screen). For a debounced preview this includes the settle debounce the
//!     user's own pause armed — that gap is the difference between this total and the
//!     summed phases below, and is the honest "how long until it settled" number.
//!   * `font X · reshape Y · rowgeom Z · atlas W · present P` — each WORK phase's own
//!     duration (ms), in wall-clock order:
//!       - `font`    — adopt the new world's effective face + rewrap the document to it
//!                     (`sync_theme_font`'s pre-shape reconfigure; cosmic-text loads the
//!                     face lazily, so its file-load cost is amortized into `reshape`/`atlas`).
//!       - `reshape` — re-lay every line's attrs + shape the whole document in the new face.
//!       - `rowgeom` — recompute the variable-row visual-geometry cache.
//!       - `atlas`   — the settled frame's `prepare` span (rasterize + upload the new
//!                     face's glyphs into the atlas; on a switch frame this dominates prepare).
//!       - `present` — that frame's encode + submit + present (the reshaped doc reaches screen).
//!
//! THE PURE / LIVE SPLIT (mirrors `debug.rs`'s readout functions). This module reads
//! NO clock: it is a pure accumulator ([`SwitchPhases`]) fed synthetic-or-real millis
//! by a caller that owns every `Instant`, plus the pure formatting below. So the whole
//! module is unit-testable with fixed durations, and the readout is STRUCTURALLY ABSENT
//! from the headless capture: [`settle_lines`] returns an EMPTY vec for the `None`
//! (no-switch-measured) value — the ONLY value a capture ever holds, because the live
//! App never feeds a switch on the deterministic path (the reshape timers live behind
//! `debug_on()` + the live App, exactly like the frametime/autosave/gpu readouts). A
//! `--debug` screenshot is therefore byte-identical to before this feature: no data →
//! no lines. The real millisecond values are LIVE-ONLY (a real clock, a real present),
//! flagged for human confirmation on a live run.

/// The named phases of a theme-switch settle, in wall-clock order. Each names a
/// real segment of the switch work so the dominant cost identifies itself in the
/// breakdown line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchPhase {
    /// Adopt the new world's effective face + rewrap the document to it.
    Font,
    /// Re-lay every line's attrs + shape the whole document in the new face.
    Reshape,
    /// Recompute the variable-row visual-geometry cache.
    RowGeom,
    /// Rasterize + upload the new face's glyphs into the atlas (the settled
    /// frame's `prepare` span).
    Atlas,
    /// Encode + submit + present the reshaped frame (the first — settled — present).
    Present,
}

impl SwitchPhase {
    /// The five phases in wall-clock order — the breakdown line's fixed column order.
    pub const ORDER: [SwitchPhase; 5] = [
        SwitchPhase::Font,
        SwitchPhase::Reshape,
        SwitchPhase::RowGeom,
        SwitchPhase::Atlas,
        SwitchPhase::Present,
    ];

    /// The compact label the breakdown line uses for this phase.
    pub fn label(self) -> &'static str {
        match self {
            SwitchPhase::Font => "font",
            SwitchPhase::Reshape => "reshape",
            SwitchPhase::RowGeom => "rowgeom",
            SwitchPhase::Atlas => "atlas",
            SwitchPhase::Present => "present",
        }
    }
}

/// A once-per-switch PHASE ACCUMULATOR: the live theme-switch path stamps an
/// `Instant` at each phase boundary and records the elapsed millis here. Reads NO
/// clock itself — the caller owns every `Instant` — so it is fully unit-testable
/// with synthetic durations and structurally inert on the headless path (which
/// never constructs one). A phase left unrecorded reads back as `None` and shows a
/// `—` in the breakdown.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SwitchPhases {
    font: Option<f32>,
    reshape: Option<f32>,
    row_geom: Option<f32>,
    atlas: Option<f32>,
    present: Option<f32>,
}

impl SwitchPhases {
    /// Record (or overwrite) one phase's own duration, in milliseconds.
    pub fn record(&mut self, phase: SwitchPhase, ms: f32) {
        *match phase {
            SwitchPhase::Font => &mut self.font,
            SwitchPhase::Reshape => &mut self.reshape,
            SwitchPhase::RowGeom => &mut self.row_geom,
            SwitchPhase::Atlas => &mut self.atlas,
            SwitchPhase::Present => &mut self.present,
        } = Some(ms);
    }

    /// This phase's recorded duration (ms), or `None` if it was never recorded.
    pub fn get(&self, phase: SwitchPhase) -> Option<f32> {
        match phase {
            SwitchPhase::Font => self.font,
            SwitchPhase::Reshape => self.reshape,
            SwitchPhase::RowGeom => self.row_geom,
            SwitchPhase::Atlas => self.atlas,
            SwitchPhase::Present => self.present,
        }
    }
}

/// The debug-panel LINES for a settled theme switch: the felt-latency headline plus
/// the per-phase breakdown, or an EMPTY vec when no switch has been measured.
///
/// `None` is the ONLY value the headless capture ever holds (the live App never
/// feeds a switch on the deterministic path), so an empty vec is what keeps the
/// readout STRUCTURALLY ABSENT from a `--debug` screenshot — no data, no lines, a
/// byte-identical capture. The determinism law rests on this: it is asserted
/// directly in the tests.
pub fn settle_lines(measured: Option<(f32, SwitchPhases)>) -> Vec<String> {
    let Some((total_ms, phases)) = measured else {
        return Vec::new();
    };
    vec![settled_readout(total_ms), breakdown_readout(&phases)]
}

/// The HEADLINE settle line: the felt latency from the triggering input to the
/// settled present, in whole-tenths of a millisecond.
pub fn settled_readout(total_ms: f32) -> String {
    format!("theme settled {total_ms:.1} ms")
}

/// The once-per-switch PHASE BREAKDOWN line: each phase's own duration in
/// wall-clock order (`SwitchPhase::ORDER`), `·`-separated, so the dominant cost
/// names itself. A phase with no recorded duration shows `—`.
pub fn breakdown_readout(phases: &SwitchPhases) -> String {
    let parts: Vec<String> = SwitchPhase::ORDER
        .iter()
        .map(|&p| match phases.get(p) {
            Some(ms) => format!("{} {:.1}", p.label(), ms),
            None => format!("{} —", p.label()),
        })
        .collect();
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_get_roundtrip_per_phase() {
        let mut p = SwitchPhases::default();
        // A fresh accumulator has nothing recorded.
        for ph in SwitchPhase::ORDER {
            assert_eq!(p.get(ph), None);
        }
        p.record(SwitchPhase::Font, 0.2);
        p.record(SwitchPhase::Reshape, 6.8);
        p.record(SwitchPhase::RowGeom, 0.9);
        p.record(SwitchPhase::Atlas, 2.0);
        p.record(SwitchPhase::Present, 0.6);
        assert_eq!(p.get(SwitchPhase::Font), Some(0.2));
        assert_eq!(p.get(SwitchPhase::Reshape), Some(6.8));
        assert_eq!(p.get(SwitchPhase::RowGeom), Some(0.9));
        assert_eq!(p.get(SwitchPhase::Atlas), Some(2.0));
        assert_eq!(p.get(SwitchPhase::Present), Some(0.6));
        // Recording again overwrites (a phase is measured once per switch).
        p.record(SwitchPhase::Reshape, 4.1);
        assert_eq!(p.get(SwitchPhase::Reshape), Some(4.1));
    }

    #[test]
    fn settled_readout_formats_one_decimal_ms() {
        assert_eq!(settled_readout(12.44), "theme settled 12.4 ms");
        assert_eq!(settled_readout(0.0), "theme settled 0.0 ms");
        assert_eq!(settled_readout(155.25), "theme settled 155.2 ms");
    }

    #[test]
    fn breakdown_readout_names_each_phase_in_order() {
        // Feed SYNTHETIC durations (no clock) and assert the exact formatted line —
        // the phases appear in wall-clock order, the dominant cost (reshape) visible.
        let mut p = SwitchPhases::default();
        p.record(SwitchPhase::Font, 0.2);
        p.record(SwitchPhase::Reshape, 6.8);
        p.record(SwitchPhase::RowGeom, 0.9);
        p.record(SwitchPhase::Atlas, 2.0);
        p.record(SwitchPhase::Present, 0.6);
        assert_eq!(
            breakdown_readout(&p),
            "font 0.2 · reshape 6.8 · rowgeom 0.9 · atlas 2.0 · present 0.6"
        );
    }

    #[test]
    fn breakdown_readout_shows_dash_for_an_unrecorded_phase() {
        // A partial accumulator (only the reshape-side phases recorded, e.g. a switch
        // whose present frame was skipped) shows `—` for the missing present-side ones.
        let mut p = SwitchPhases::default();
        p.record(SwitchPhase::Font, 0.1);
        p.record(SwitchPhase::Reshape, 5.0);
        p.record(SwitchPhase::RowGeom, 0.8);
        assert_eq!(
            breakdown_readout(&p),
            "font 0.1 · reshape 5.0 · rowgeom 0.8 · atlas — · present —"
        );
    }

    #[test]
    fn settle_lines_are_absent_without_a_measured_switch() {
        // DETERMINISM LAW (formatting seam): the `None` value — the ONLY value a
        // headless capture ever holds, since the live App never feeds a switch on the
        // deterministic path — yields ZERO lines. No data, no readout: a `--debug`
        // screenshot stays byte-identical to before this feature.
        assert_eq!(settle_lines(None), Vec::<String>::new());
    }

    #[test]
    fn settle_lines_are_the_headline_then_the_breakdown() {
        let mut p = SwitchPhases::default();
        p.record(SwitchPhase::Font, 0.2);
        p.record(SwitchPhase::Reshape, 6.8);
        p.record(SwitchPhase::RowGeom, 0.9);
        p.record(SwitchPhase::Atlas, 2.0);
        p.record(SwitchPhase::Present, 0.6);
        assert_eq!(
            settle_lines(Some((155.2, p))),
            vec![
                "theme settled 155.2 ms".to_string(),
                "font 0.2 · reshape 6.8 · rowgeom 0.9 · atlas 2.0 · present 0.6".to_string(),
            ]
        );
    }
}
