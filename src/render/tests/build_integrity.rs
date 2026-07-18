//! BUILD-INTEGRITY CANARY (2026-07-18 debt audit) — a fast, self-contained
//! tripwire for the CORRUPT-INCREMENTAL-BUILD failure class.
//!
//! ## The signature (what a corrupt build looks like)
//! On 2026-07-18 a "frost regression" turned out NOT to be a source bug at all:
//! under heavy PARALLEL `cargo` load, an incremental build produced a BINARY
//! that computed IMPOSSIBLE metrics — `line_height` scaled ×1.6 UP and
//! `char_width` ×0.8 DOWN, ratios no correct code can emit — which cascaded into
//! a page margin of ~6950px against a 50px floor. A full source revert still
//! "failed"; a clean-target build was green. Both oracles (CI on the pushed
//! commit + a local clean rebuild) agreed the source was fine. The tells of a
//! corrupt build, distinct from a real regression:
//!   - metrics/geometry with IMPOSSIBLE ratios (self-inconsistent numbers no
//!     correct arithmetic produces),
//!   - `exit code 144` / SIGKILL-family deaths of test processes under parallel
//!     load,
//!   - a failure that VANISHES on a clean-target rebuild and does not reproduce
//!     in CI on the same commit.
//!
//! ## The protocol (do THIS before blaming source)
//! When a failure looks like the signature above — impossible-looking metrics,
//! exit-144 kills under parallel load, or a green-in-CI / red-locally split —
//! RE-RUN THE GATE WITH `CARGO_INCREMENTAL=0` (a full non-incremental build)
//! BEFORE editing any source. If it goes green, it was a corrupt incremental
//! artifact, not a regression — never `cargo clean` reflexively (slow), just
//! disable incremental for the confirming run. Only if `CARGO_INCREMENTAL=0`
//! ALSO fails is the source actually at fault. (This is the standing lesson the
//! debt audit banked; the orchestrator process fix — never chain a push on a
//! non-suite exit — is its companion.)
//!
//! ## This canary
//! The metric derivation (`render::Metrics::with_dpi`) multiplies each base
//! glyph constant by a single scale `s = zoom * dpi`, so EVERY ratio between two
//! metrics is scale-INVARIANT and must equal the corresponding ratio of the
//! source constants. A corrupt codegen of that multiply (the exact frost tell)
//! breaks the invariance and/or drives the ratios outside any sane band. This is
//! a probabilistic tripwire, not a proof — a corrupt object could in principle
//! miscompile the test too — but it is cheap, GPU-free, and would have caught
//! the documented frost signature at suite start. The heavier real guard remains
//! the protocol above plus the exact-value geometry suite (`super::geometry`).

use super::super::*;

/// Relative tolerance for an f32 ratio comparison: `(A*s)/(B*s)` can differ from
/// `A/B` only by float rounding, never by a whole scale factor, so this is tight
/// enough to catch a ×0.8/×1.6 corruption yet immune to ordinary f32 noise.
const REL_EPS: f32 = 1e-4;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= REL_EPS * a.abs().max(b.abs()).max(1.0)
}

#[test]
fn font_metrics_are_scale_invariant_and_sane() {
    // (1) The base metrics (zoom 1, dpi 1) reproduce the source constants
    // EXACTLY — the derivation is identity at s == 1.
    let base = Metrics::new(1.0);
    assert_eq!(base.font_size, FONT_SIZE, "base font_size drifted from FONT_SIZE");
    assert_eq!(base.line_height, LINE_HEIGHT, "base line_height drifted from LINE_HEIGHT");
    assert_eq!(base.char_width, CHAR_WIDTH, "base char_width drifted from CHAR_WIDTH");

    // The invariant ratios, straight from the source constants.
    let lh_ratio = LINE_HEIGHT / FONT_SIZE; // 32/24 = 1.333…
    let cw_ratio = CHAR_WIDTH / FONT_SIZE; //  14.4/24 = 0.6
    let ch_ratio = CARET_H / FONT_SIZE;

    // Sane bands — the frost corruption was line_height ×1.6 UP (ratio → ~2.13)
    // and char_width ×0.8 DOWN (ratio → ~0.48); both fall OUTSIDE these bands, so
    // even a build that corrupted numerator and denominator independently trips
    // here. Chosen wide enough that any legitimate constant retune stays inside.
    assert!((1.2..=1.45).contains(&lh_ratio), "line_height/font_size {lh_ratio} outside sane band");
    assert!((0.5..=0.7).contains(&cw_ratio), "char_width/font_size {cw_ratio} outside sane band");

    // (2) Across a spread of zoom × dpi the ratios stay put — this exercises the
    // actual `* s` multiply the corrupt build got wrong, at non-trivial scales.
    for zoom in [0.5_f32, 1.0, 1.5, 2.0, 3.0] {
        for dpi in [1.0_f32, 2.0] {
            let m = Metrics::with_dpi(zoom, dpi);
            assert!(m.font_size > 0.0, "non-positive font_size at zoom {zoom} dpi {dpi}");
            assert!(
                close(m.line_height / m.font_size, lh_ratio),
                "line_height/font_size drifted at zoom {zoom} dpi {dpi}: {} vs {lh_ratio}",
                m.line_height / m.font_size
            );
            assert!(
                close(m.char_width / m.font_size, cw_ratio),
                "char_width/font_size drifted at zoom {zoom} dpi {dpi}: {} vs {cw_ratio}",
                m.char_width / m.font_size
            );
            assert!(
                close(m.caret_h / m.font_size, ch_ratio),
                "caret_h/font_size drifted at zoom {zoom} dpi {dpi}: {} vs {ch_ratio}",
                m.caret_h / m.font_size
            );
        }
    }
}
