//! src/ease.rs — the one owner of the smoothstep easing curve.
//!
//! `3t² − 2t³` on a `[0, 1]` input (Hermite smoothstep): 0 at the ends, `0.5`
//! at the middle, zero slope at both ends so a spring-back / crossfade lands
//! with no linear kink. This exact curve was hand-inlined at six sites (the
//! copy-pulse ease, the focus crossfade, the caret pop / trail / re-form eases,
//! and the spring's damping ramp) — one owner, so "same behavior ⇒ same code".
//!
//! PURE (no GPU/clock), so it is unit-testable directly; out-of-range `t`
//! clamps to `[0, 1]` first (idempotent for callers already in range).

/// Hermite smoothstep `3t² − 2t³`, clamped to `[0, 1]`. `f(0)==0`, `f(1)==1`,
/// `f(0.5)==0.5`, monotonic, symmetric about `t = 0.5`.
pub fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Ease-out-back — the small OVERSHOOT spring (Penner's `easeOutBack`,
/// overshoot constant `c1 = 1.70158` ≈ 10% peak overshoot): starts fast,
/// overshoots `1.0` slightly (peaking ~1.10 around `t ≈ 0.7`), and settles
/// back to exactly `1.0`. The FIRETAIL-MAXIMALIST-SHOWCASE round's motion-
/// juice curve — the overlay entrance drop and the selection-band slide both
/// read THIS one owner, so the two juices share one spring character.
/// `f(0)==0`, `f(1)==1`; input clamps to `[0, 1]` (idempotent past the end).
pub fn out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    const C1: f32 = 1.70158;
    const C3: f32 = C1 + 1.0;
    1.0 + C3 * (t - 1.0).powi(3) + C1 * (t - 1.0).powi(2)
}

#[cfg(test)]
mod tests {
    use super::{out_back, smoothstep};

    #[test]
    fn out_back_pins_endpoints_overshoots_once_and_clamps() {
        // Endpoints exact (float-exact: the polynomial collapses to 0/1 there).
        assert_eq!(out_back(0.0), 0.0);
        assert_eq!(out_back(1.0), 1.0);
        // It genuinely OVERSHOOTS (that's the spring): some sample exceeds 1,
        // but never wildly (the c1=1.70158 overshoot peaks ≈ 1.10).
        let mut peak = 0.0f32;
        for i in 0..=100 {
            let v = out_back(i as f32 / 100.0);
            peak = peak.max(v);
        }
        assert!(peak > 1.0, "out_back must overshoot past 1.0 (peak {peak})");
        assert!(peak < 1.2, "out_back's overshoot stays gentle (peak {peak})");
        // Out-of-range input clamps to the settled endpoints.
        assert_eq!(out_back(-1.0), 0.0);
        assert_eq!(out_back(2.0), 1.0);
    }

    #[test]
    fn smoothstep_pins_endpoints_midpoint_monotone_and_clamps() {
        // Endpoints + the symmetric midpoint are exact.
        assert_eq!(smoothstep(0.0), 0.0);
        assert_eq!(smoothstep(1.0), 1.0);
        assert_eq!(smoothstep(0.5), 0.5);

        // Monotone non-decreasing across a sampled sweep of [0, 1].
        let mut prev = smoothstep(0.0);
        for i in 1..=100 {
            let e = smoothstep(i as f32 / 100.0);
            assert!(e >= prev, "not monotone at i={i}: {e} < {prev}");
            assert!((0.0..=1.0).contains(&e), "out of range at i={i}: {e}");
            prev = e;
        }

        // Out-of-range input clamps (no extrapolation past the endpoints).
        assert_eq!(smoothstep(-0.5), 0.0);
        assert_eq!(smoothstep(-1000.0), 0.0);
        assert_eq!(smoothstep(1.5), 1.0);
        assert_eq!(smoothstep(1000.0), 1.0);
    }
}
