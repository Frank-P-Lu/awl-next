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

#[cfg(test)]
mod tests {
    use super::smoothstep;

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
