//! OVERLAY SUMMON/DISMISS MOTION — the one calm, unified transition every summoned
//! overlay shares.
//!
//! A summoned overlay (command palette, go-to, theme picker, spell popup, …) no
//! longer SNAPS into and out of existence: it RISES a few pixels into place on
//! summon and SINKS back on dismiss, eased through the shared
//! [`crate::ease::smoothstep`] curve. ONE owner drives ALL of them, so the feel is
//! identical for every picker (the single-owner discipline the codebase leans on
//! for role colour, float elevation, and row layout).
//!
//! This is a PURE, clock-free state machine — the render pipeline OWNS one of these
//! and advances it on the LIVE clock (mirroring the caret spring / copy pulse
//! exactly). It exposes only a scalar `rise_px()` the geometry reads. It touches no
//! GPU and no wall clock, so it is unit-testable directly AND — crucially —
//! DETERMINISM-SAFE: it is constructed [`settled`](OverlaySummon::settled) (fully
//! present, `rise_px() == 0`), and the headless capture path NEVER kicks it, so a
//! `--screenshot` of an open overlay renders the exact settled pixels it always did
//! (byte-identical). Only the live App ever calls [`summon`](OverlaySummon::summon)
//! / [`dismiss`](OverlaySummon::dismiss).

use crate::ease::smoothstep;

/// How far (logical px) the overlay sits BELOW its resting position at the very
/// start of a summon (and where it sinks to at the end of a dismiss). Small — a
/// calm settle, not a slam. A TASTE TUNABLE flagged for live review (named like
/// `THEME_FONT_DEBOUNCE` / the `CARET_*` flinch magnitudes).
pub const OVERLAY_RISE_PX: f32 = 10.0;

/// The summon/dismiss travel time (ms). ~150ms reads as quick-but-present — of a
/// piece with the caret pop / copy pulse durations, not a sluggish slide. TASTE
/// TUNABLE (live review).
pub const OVERLAY_MOTION_MS: f32 = 150.0;

/// The pure summon/dismiss animator: a single progress `t` easing toward a
/// `target`. `t == 1` is FULLY PRESENT (resting, `rise_px() == 0`); `t == 0` is
/// FULLY HIDDEN (risen `OVERLAY_RISE_PX` away). Both endpoints are exact (no float
/// drift) because [`step`](Self::step) snaps onto the target once within one
/// frame's reach.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlaySummon {
    /// Presence in `[0, 1]`: 1 = fully shown/resting, 0 = fully hidden/away.
    t: f32,
    /// Where `t` is easing toward: `1.0` while summoning/settled-open, `0.0` while
    /// dismissing/settled-closed.
    target: f32,
}

impl OverlaySummon {
    /// The DETERMINISM DEFAULT: fully present, at rest, nothing animating. The
    /// pipeline constructs one of these, and the headless capture NEVER kicks it, so
    /// `rise_px()` is a hard `0.0` in every capture (byte-identical). A live overlay
    /// open replaces this via [`summon`](Self::summon).
    pub fn settled() -> Self {
        Self { t: 1.0, target: 1.0 }
    }

    /// SUMMON: start a rise-in from fully hidden. Snaps `t` to 0 (fully away) and
    /// aims at 1 (resting). Idempotent under rapid re-fire — re-summoning simply
    /// restarts the rise from 0. Called on an overlay OPEN transition (live only).
    pub fn summon(&mut self) {
        self.t = 0.0;
        self.target = 1.0;
    }

    /// DISMISS: start a sink-out from wherever `t` currently is toward fully hidden
    /// (0). Called on an overlay CLOSE transition (live only). If summoned mid-rise,
    /// it reverses smoothly from the current `t` (no jump).
    pub fn dismiss(&mut self) {
        self.target = 0.0;
    }

    /// Advance the presence toward its target by `dt` seconds, moving at a constant
    /// `1 / OVERLAY_MOTION_MS` rate and SNAPPING onto the target within one frame's
    /// reach (so both endpoints are exact — no lingering sub-pixel animation, no
    /// float drift at rest). Returns `true` while still in flight, so the caller's
    /// `advance(dt)` "keep redrawing" OR-fold stays hot ONLY while the overlay
    /// moves, then idles at 0% CPU — mirrors [`crate::caret::CaretAnim::step_pop`].
    pub fn step(&mut self, dt: f32) -> bool {
        if self.t == self.target {
            return false;
        }
        let d = dt * 1000.0 / OVERLAY_MOTION_MS;
        if (self.target - self.t).abs() <= d {
            self.t = self.target;
        } else if self.target > self.t {
            self.t += d;
        } else {
            self.t -= d;
        }
        self.t != self.target
    }

    /// The current vertical OFFSET (logical px, positive = DOWN) to add to the
    /// overlay card + text top this frame: `OVERLAY_RISE_PX` when fully hidden,
    /// smoothstep-eased to `0.0` at rest. The card thus RISES up into place on
    /// summon and sinks back on dismiss. A hard `0.0` at `t == 1` (the settled
    /// default), so a capture adds nothing (byte-identical).
    pub fn rise_px(&self) -> f32 {
        (1.0 - smoothstep(self.t)) * OVERLAY_RISE_PX
    }

    /// True while a DISMISS is in flight — the target is fully-hidden but the
    /// overlay has not yet finished sinking away (`0 < t`). The pipeline reads this
    /// to KEEP drawing the (logically-closed) overlay's retained content through the
    /// sink-out, rather than snapping it off the instant the App clears
    /// `self.overlay`. `false` at rest, while summoning, and once fully hidden.
    pub fn dismissing(&self) -> bool {
        self.target == 0.0 && self.t > 0.0
    }

    /// True once a dismiss has fully completed (`t == 0`, target hidden): the moment
    /// the pipeline drops the retained overlay content and stops drawing it. `false`
    /// at rest / while summoning / mid-dismiss.
    pub fn fully_hidden(&self) -> bool {
        self.target == 0.0 && self.t == 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A frame long enough to CROSS the whole travel in one step (so a single
    // `step` snaps to the endpoint) — used to reach a settled state deterministically.
    const BIG_DT: f32 = 1.0; // 1s ≫ 150ms

    #[test]
    fn settled_default_is_fully_present_with_zero_rise_and_not_animating() {
        let mut s = OverlaySummon::settled();
        assert_eq!(s.rise_px(), 0.0, "settled overlay must add no offset (capture-safe)");
        assert!(!s.dismissing());
        assert!(!s.fully_hidden());
        // Nothing to animate: step reports settled immediately.
        assert!(!s.step(1.0 / 60.0));
        assert_eq!(s.rise_px(), 0.0);
    }

    #[test]
    fn summon_starts_hidden_and_eases_to_resting() {
        let mut s = OverlaySummon::settled();
        s.summon();
        // Fully hidden at the kick: the full rise offset, eased.
        assert_eq!(s.rise_px(), OVERLAY_RISE_PX);
        assert!(!s.dismissing(), "a summon is not a dismiss");
        // In flight until it reaches the top.
        assert!(s.step(1.0 / 240.0), "still rising after a tiny step");
        assert!(s.rise_px() < OVERLAY_RISE_PX && s.rise_px() > 0.0);
        // A big step lands EXACTLY at rest (no drift), and reports settled.
        assert!(!s.step(BIG_DT));
        assert_eq!(s.rise_px(), 0.0);
    }

    #[test]
    fn dismiss_sinks_back_out_then_reports_fully_hidden() {
        let mut s = OverlaySummon::settled();
        // From a resting/open overlay, dismiss.
        s.dismiss();
        assert!(s.dismissing(), "target hidden + still present = dismissing");
        assert!(!s.fully_hidden());
        // Mid-sink: some offset, still dismissing, still animating.
        assert!(s.step(1.0 / 240.0));
        assert!(s.dismissing());
        assert!(s.rise_px() > 0.0);
        // Completes EXACTLY at hidden.
        assert!(!s.step(BIG_DT));
        assert!(!s.dismissing(), "no longer in flight");
        assert!(s.fully_hidden(), "settled at fully hidden");
        assert_eq!(s.rise_px(), OVERLAY_RISE_PX);
    }

    #[test]
    fn plain_summon_always_restarts_from_hidden() {
        let mut s = OverlaySummon::settled();
        s.summon();
        s.step(BIG_DT); // fully open
        s.summon(); // re-summon
        assert_eq!(s.rise_px(), OVERLAY_RISE_PX, "a fresh summon restarts from hidden");
    }

    #[test]
    fn step_snaps_onto_the_target_within_one_frame_reach_no_overshoot() {
        let mut s = OverlaySummon::settled();
        s.summon(); // t = 0, target = 1
        // A step just past the remaining distance lands exactly on 1, never past.
        let _ = s.step(BIG_DT);
        assert_eq!(s.t, 1.0);
        assert_eq!(s.target, 1.0);
        assert!(!s.step(1.0 / 60.0));
    }
}
