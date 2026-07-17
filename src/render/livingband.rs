//! src/render/livingband.rs — ARM B "living selection band" choreography PROBES.
//!
//! The user's 10× P5-cursor direction (TASTE PRINCIPLES #3): a LIVING SELECTION
//! BAND that STRETCHES/MORPHS between rows and, in its two-shape voice, lays two
//! offset translucent shapes whose OVERLAP reads as the world's BRIGHTEST value
//! step — "mesmerizing… alive." Menu surfaces only; typing latency untouched.
//!
//! This module is the PURE PHASE MATH (no GPU, no clock, no `Theme` — unit
//! testable directly) plus the dev-only env PIN that lets a headless capture
//! dump a deterministic MID-FLIGHT frame (mirrors `AWL_LAVA`'s phase pin and the
//! wild-menu slant probe). It ships NOTHING by default: with `AWL_OVERLAY_MOTION_
//! FORCE` unset, [`overlay_motion_force`] is `None`, the renderer takes its
//! ordinary single-band path, and every capture is BYTE-IDENTICAL. The knob is a
//! live-A/B + gallery instrument only, exactly like `AWL_MOTION_FORCE`.
//!
//! Two choreographies, one shared elastic core (leading edge fast, trailing edge
//! slow — the P5 elastic):
//!   * **MORPH** — one band whose LEADING edge races the target while the
//!     TRAILING edge lags, so the band momentarily STRETCHES then snaps home with
//!     a hint of overshoot ([`ease::out_back`]).
//!   * **TWO-SHAPE** — a leading band + a chasing ECHO (the same elastic split,
//!     read as two separate one-row shapes). Their crossing region is returned
//!     explicitly so the renderer can fill it at a brighter value step — colour
//!     WHERE THEY CROSS, by VALUE math, never a second accent (DESIGN §3).
//!
//! Per-world CHOREOGRAPHY is expressed as [`Choreo`] presets over the one core
//! ([`MorphParams`]): `Slam` (Firetail-flashy: fast lead, hard overshoot) vs
//! `Soft` (a quiet world: gentle lead, smoothstep, no overshoot) — the same
//! mechanism, different voice (TASTE PRINCIPLES #2/#4: aliveness ≠ loudness).

use crate::ease;

/// The minimum drawn band height (px) the morph guard clamps to, so a transient
/// elastic recoil (the trailing edge overshooting past the leading one) can
/// never invert or vanish the quad.
pub const MIN_BAND_H: f32 = 1.0;

/// How many rows the PINNED (`phase` given) capture synthesises the band's
/// travel across — the band is dumped mid-flight arriving at the selected row
/// from this many rows BELOW it, sliding up. A few rows so the stretch/echo is
/// unmistakable in a single still.
pub const PIN_JUMP_ROWS: f32 = 3.0;

/// A single drawn band rectangle in the row's vertical axis: `top` (px, canvas
/// space) and `height` (px). The horizontal span is the caller's (card x/width).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandRect {
    pub top: f32,
    pub height: f32,
}

/// The two-shape choreography's frame: the leading band's top, the chasing
/// echo's top (both one row `height` tall), and the CROSSING region (`None` once
/// the two shapes fully separate) the renderer fills at the brightest value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TwoShape {
    pub primary_top: f32,
    pub echo_top: f32,
    pub height: f32,
    pub overlap: Option<BandRect>,
}

/// The elastic-core parameters shared by both choreographies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MorphParams {
    /// Time-gain on the LEADING edge/shape (`>= 1`). `1.0` = rigid (both edges
    /// ease together, no stretch); larger races the leading edge ahead so the
    /// band STRETCHES (morph) or the two shapes SEPARATE (two-shape) mid-flight.
    pub lead_gain: f32,
    /// `true` → both edges ease with [`ease::out_back`] (the overshoot SNAP);
    /// `false` → [`ease::smoothstep`] (the quiet soft morph, no overshoot).
    pub overshoot: bool,
}

/// The four choreography presets the env knob selects. `Morph`/`TwoShape` are
/// the two mechanisms; `Slam`/`Soft` are per-world MORPH voices (same core,
/// different [`MorphParams`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choreo {
    /// One stretching band, medium lead, gentle overshoot.
    Morph,
    /// Leading band + chasing echo, colour where they cross.
    TwoShape,
    /// Firetail-flashy morph: fast lead, hard overshoot snap.
    Slam,
    /// Quiet-world morph: gentle lead, smoothstep, no overshoot.
    Soft,
}

impl Choreo {
    /// The elastic-core params for this preset.
    pub fn params(self) -> MorphParams {
        match self {
            Choreo::Morph => MorphParams { lead_gain: 1.8, overshoot: true },
            Choreo::TwoShape => MorphParams { lead_gain: 1.9, overshoot: true },
            Choreo::Slam => MorphParams { lead_gain: 2.3, overshoot: true },
            Choreo::Soft => MorphParams { lead_gain: 1.35, overshoot: false },
        }
    }

    /// Whether this preset draws the TWO-SHAPE (echo + crossing) rather than the
    /// single morph band.
    pub fn is_two_shape(self) -> bool {
        matches!(self, Choreo::TwoShape)
    }
}

/// Ease `x` with this preset's curve: [`ease::out_back`] (overshoot) or
/// [`ease::smoothstep`] (soft). Both pin `f(0)=0`, `f(1)=1`.
fn ease_with(p: &MorphParams, x: f32) -> f32 {
    if p.overshoot {
        ease::out_back(x)
    } else {
        ease::smoothstep(x)
    }
}

/// THE MORPH choreography — the single band's drawn rect at phase `t` for a
/// travel from `from_top` to `to_top`, each row `h` tall.
///
/// The band's two edges ease at different rates: the LEADING edge (the one
/// pointing toward the destination — the bottom when moving down, the top when
/// moving up) is time-warped `lead_gain`× faster, the TRAILING edge eases on
/// plain `t`. So the band STRETCHES taller mid-flight (the leading edge pulls
/// ahead) and contracts back to exactly `h` as the trailing edge catches at the
/// end — with a hint of overshoot when `overshoot` is set.
///
/// Endpoints are EXACT: `morph_band(a, b, h, 0, ..)` is the `a` row rect and
/// `morph_band(a, b, h, 1, ..)` is the `b` row rect (both height `h`), because
/// both eases pin `f(0)=0`/`f(1)=1`. A no-move (`from_top == to_top`) is the
/// constant rect at every `t`.
pub fn morph_band(from_top: f32, to_top: f32, h: f32, t: f32, p: &MorphParams) -> BandRect {
    let t = t.clamp(0.0, 1.0);
    let gain = p.lead_gain.max(1.0);
    let from_bot = from_top + h;
    let to_bot = to_top + h;
    let e_lead = ease_with(p, (t * gain).min(1.0));
    let e_trail = ease_with(p, t);
    // The physical edge that LEADS depends on travel direction.
    let moving_down = to_top >= from_top;
    let (e_top, e_bot) = if moving_down { (e_trail, e_lead) } else { (e_lead, e_trail) };
    let top = from_top + (to_top - from_top) * e_top;
    let bot = from_bot + (to_bot - from_bot) * e_bot;
    // Guard the transient recoil (trailing edge overshooting past the leading):
    // keep the rect upright and never sub-`MIN_BAND_H`.
    let (top, bot) = if bot >= top { (top, bot) } else { (bot, top) };
    BandRect { top, height: (bot - top).max(MIN_BAND_H) }
}

/// THE TWO-SHAPE choreography — the leading band top, the chasing echo top, and
/// their crossing region at phase `t` for a travel from `from_top` to `to_top`,
/// each shape `h` tall.
///
/// The leading shape rides the `lead_gain`× time-warp; the echo rides plain `t`,
/// chasing behind. Where the two one-row shapes OVERLAP, the renderer fills the
/// returned [`TwoShape::overlap`] rect at the world's brightest value step —
/// colour where they cross. At the endpoints the shapes coincide (full overlap
/// = a solid row); mid-flight they separate and the crossing shrinks, then
/// vanishes (`None`) once the gap reaches a full row, then re-merges on arrival.
pub fn two_shape_band(from_top: f32, to_top: f32, h: f32, t: f32, p: &MorphParams) -> TwoShape {
    let t = t.clamp(0.0, 1.0);
    let gain = p.lead_gain.max(1.0);
    let d = to_top - from_top;
    let e_lead = ease_with(p, (t * gain).min(1.0));
    let e_trail = ease_with(p, t);
    let primary_top = from_top + d * e_lead;
    let echo_top = from_top + d * e_trail;
    let o_top = primary_top.max(echo_top);
    let o_bot = (primary_top + h).min(echo_top + h);
    let overlap = if o_bot - o_top > MIN_BAND_H {
        Some(BandRect { top: o_top, height: o_bot - o_top })
    } else {
        None
    };
    TwoShape { primary_top, echo_top, height: h, overlap }
}

/// INK RIDES THE BAND, NOT THE STATE — which DISPLAY rows the living band
/// currently COVERS, so their glyphs take the on-band (selected) ink while the
/// band is over them, and the target row keeps its OFF-band ink until the band
/// arrives. Rows lie at `first_top + k*lh` for `k in 0..n`; a row is "covered"
/// when a band overlaps it by MORE THAN HALF its height (so the ink flips only
/// once the fill predominantly owns the row — never a one-pixel graze). `bands`
/// is the leading band (morph) or the leading band + chasing echo (two-shape);
/// a row counts as covered if EITHER shape majority-covers it (the crossing is
/// a subset of both). At rest exactly the target row is covered; mid-flight the
/// band sits between rows, so 0, 1, or 2 rows can be — matching what the eye
/// sees the fill sitting on. Pure; unit-tested over flight phases.
///
/// The OLD (state-tied) behaviour is recovered by passing the single settled
/// target rect: `covered_rows(&[target_band], first_top, lh, n)` returns exactly
/// `[target_disp]`, so the env-unset path stays byte-identical.
pub fn covered_rows(bands: &[BandRect], first_top: f32, lh: f32, n: usize) -> Vec<usize> {
    let mut out = Vec::new();
    if lh <= 0.0 {
        return out;
    }
    for k in 0..n {
        let row_top = first_top + k as f32 * lh;
        let row_bot = row_top + lh;
        // Max single-band overlap: for two separated shapes each owns a
        // different row, and where they cross the two coincide — so MAX (not
        // sum) is the true covered depth for this row.
        let mut cov = 0.0f32;
        for b in bands {
            let top = row_top.max(b.top);
            let bot = row_bot.min(b.top + b.height);
            cov = cov.max((bot - top).max(0.0));
        }
        if cov > lh * 0.5 {
            out.push(k);
        }
    }
    out
}

/// The parsed `AWL_OVERLAY_MOTION_FORCE` value: which [`Choreo`] to draw and,
/// optionally, a PINNED phase for a deterministic mid-flight capture frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionForce {
    pub choreo: Choreo,
    /// `Some(t)` pins the band at phase `t` over a synthetic [`PIN_JUMP_ROWS`]
    /// travel (the capture frame-dump path — deterministic, no clock). `None`
    /// lets the live animator drive the phase (live-only; a capture settles).
    pub phase: Option<f32>,
}

/// Parse an `AWL_OVERLAY_MOTION_FORCE` string. Grammar: `<kind>[:<phase>]` where
/// `kind` ∈ `morph | twoshape | slam | soft` and `phase` ∈ `[0, 1]`. `off`/empty
/// → `None` (probe inert). A malformed kind or phase → `None` (fall back to the
/// ordinary single band; a bad knob never crashes or alters a default run).
pub fn parse_motion_force(s: &str) -> Option<MotionForce> {
    let s = s.trim().to_ascii_lowercase();
    if s.is_empty() || s == "off" {
        return None;
    }
    let (kind, phase) = match s.split_once(':') {
        Some((k, p)) => {
            let phase: f32 = p.trim().parse().ok()?;
            if !(0.0..=1.0).contains(&phase) {
                return None;
            }
            (k.trim(), Some(phase))
        }
        None => (s.as_str(), None),
    };
    let choreo = match kind {
        "morph" => Choreo::Morph,
        "twoshape" | "two-shape" | "two_shape" => Choreo::TwoShape,
        "slam" => Choreo::Slam,
        "soft" => Choreo::Soft,
        _ => return None,
    };
    Some(MotionForce { choreo, phase })
}

/// The `AWL_OVERLAY_MOTION_FORCE` dev knob, read ONCE and memoised. `None` on
/// every ordinary run (env unset), so the renderer's living-band branch is
/// unreachable and every default capture stays byte-identical.
fn awl_overlay_motion_force() -> &'static Option<MotionForce> {
    static ONCE: std::sync::OnceLock<Option<MotionForce>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| std::env::var("AWL_OVERLAY_MOTION_FORCE").ok().and_then(|s| parse_motion_force(&s)))
}

/// TEST-ONLY escape hatch for the living-band probe (mirrors
/// [`crate::render::set_slant_test_override`]; `serial()`-guarded at call sites)
/// — the memoised env `OnceLock` can't be re-armed per test, so a capture-level
/// law test pins the choreography + mid-flight phase through this instead.
#[cfg(test)]
static MOTION_TEST_OVERRIDE: std::sync::Mutex<Option<MotionForce>> = std::sync::Mutex::new(None);

#[cfg(test)]
pub fn set_motion_test_override(m: Option<MotionForce>) {
    *MOTION_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = m;
}

/// The EFFECTIVE living-band probe for this frame — `None` (the shipped single
/// band) on every run without the env probe / test override, so the renderer's
/// living-band branch is unreachable and every default capture is byte-identical.
/// A `cfg(test)` override wins so a capture-level law test can pin a deterministic
/// mid-flight frame without re-arming the memoised env `OnceLock`.
pub fn overlay_motion_force() -> Option<MotionForce> {
    #[cfg(test)]
    {
        if let Some(m) = *MOTION_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) {
            return Some(m);
        }
    }
    *awl_overlay_motion_force()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() <= EPS
    }

    #[test]
    fn morph_pins_both_endpoints_exactly() {
        for &pre in &[Choreo::Morph, Choreo::Slam, Choreo::Soft, Choreo::TwoShape] {
            let p = pre.params();
            let a = morph_band(100.0, 340.0, 24.0, 0.0, &p);
            assert!(approx(a.top, 100.0) && approx(a.height, 24.0), "{pre:?} t=0 == from rect: {a:?}");
            let b = morph_band(100.0, 340.0, 24.0, 1.0, &p);
            assert!(approx(b.top, 340.0) && approx(b.height, 24.0), "{pre:?} t=1 == to rect: {b:?}");
        }
    }

    #[test]
    fn morph_stretches_mid_flight_both_directions() {
        let p = Choreo::Morph.params();
        // Moving DOWN: the band is taller than one row somewhere mid-flight.
        let mut max_h_down = 0.0f32;
        // Moving UP: same.
        let mut max_h_up = 0.0f32;
        for i in 1..20 {
            let t = i as f32 / 20.0;
            max_h_down = max_h_down.max(morph_band(100.0, 340.0, 24.0, t, &p).height);
            max_h_up = max_h_up.max(morph_band(340.0, 100.0, 24.0, t, &p).height);
        }
        assert!(max_h_down > 24.0 + 1.0, "morph stretches moving down (peak {max_h_down})");
        assert!(max_h_up > 24.0 + 1.0, "morph stretches moving up (peak {max_h_up})");
    }

    #[test]
    fn morph_stays_upright_and_never_collapses() {
        for &pre in &[Choreo::Morph, Choreo::Slam, Choreo::Soft] {
            let p = pre.params();
            for i in 0..=100 {
                let t = i as f32 / 100.0;
                let r = morph_band(80.0, 500.0, 30.0, t, &p);
                assert!(r.height >= MIN_BAND_H, "{pre:?} height >= min at t={t}: {r:?}");
            }
        }
    }

    #[test]
    fn no_move_is_a_constant_rect() {
        let p = Choreo::Morph.params();
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            let r = morph_band(200.0, 200.0, 24.0, t, &p);
            assert!(approx(r.top, 200.0) && approx(r.height, 24.0), "constant at t={t}: {r:?}");
            let s = two_shape_band(200.0, 200.0, 24.0, t, &p);
            assert!(approx(s.primary_top, 200.0) && approx(s.echo_top, 200.0), "two-shape constant at t={t}: {s:?}");
            // A no-move always fully overlaps (one solid row).
            let o = s.overlap.expect("no-move fully overlaps");
            assert!(approx(o.height, 24.0), "no-move overlap == one row: {o:?}");
        }
    }

    #[test]
    fn two_shape_endpoints_fully_overlap_and_separate_mid_flight() {
        let p = Choreo::TwoShape.params();
        let h = 24.0;
        // Endpoints: the two shapes coincide (full-row overlap).
        for &t in &[0.0f32, 1.0] {
            let s = two_shape_band(100.0, 340.0, h, t, &p);
            let o = s.overlap.expect("endpoints overlap");
            assert!(approx(o.height, h), "t={t} full overlap: {o:?}");
            assert!(approx(s.primary_top, s.echo_top), "t={t} shapes coincide");
        }
        // Mid-flight: the leading shape genuinely pulls ahead of the echo.
        let mut max_sep = 0.0f32;
        for i in 1..20 {
            let t = i as f32 / 20.0;
            let s = two_shape_band(100.0, 340.0, h, t, &p);
            max_sep = max_sep.max((s.primary_top - s.echo_top).abs());
        }
        assert!(max_sep > 1.0, "two shapes separate mid-flight (peak {max_sep})");
    }

    #[test]
    fn two_shape_crossing_vanishes_when_shapes_fully_clear() {
        // A large travel (many rows) forces a phase where the shapes are more
        // than one row apart → no crossing region at all.
        let p = Choreo::TwoShape.params();
        let saw_none = (1..40)
            .map(|i| i as f32 / 40.0)
            .any(|t| two_shape_band(0.0, 600.0, 20.0, t, &p).overlap.is_none());
        assert!(saw_none, "a wide two-shape travel has a fully-cleared (no-crossing) phase");
    }

    #[test]
    fn slam_leads_harder_than_soft() {
        // The per-world voices differ measurably: Slam's leading edge is further
        // ahead than Soft's at the same early phase (flashier vs quieter).
        let (slam, soft) = (Choreo::Slam.params(), Choreo::Soft.params());
        let t = 0.3;
        let slam_lead = morph_band(0.0, 300.0, 24.0, t, &slam).top;
        let soft_lead = morph_band(0.0, 300.0, 24.0, t, &soft).top;
        // Moving down, the leading (bottom) edge races; read progress off `top`
        // (the trailing edge) too — Slam's whole band is further along.
        assert!(slam_lead > soft_lead, "Slam leads harder than Soft ({slam_lead} vs {soft_lead})");
    }

    #[test]
    fn covered_ink_rides_the_band_not_the_state() {
        // Rows: 8 rows of `lh` from `first_top`; the SELECTED (target) row is #2.
        let (first_top, lh, h) = (100.0f32, 24.0f32, 24.0f32);
        let target_disp = 2usize;
        let target_top = first_top + target_disp as f32 * lh; // 148
        let params = Choreo::Morph.params();
        // The band flies from PIN_JUMP_ROWS below the target, sliding UP to it.
        let from = target_top + PIN_JUMP_ROWS * lh; // 3 rows below

        // AT REST (t = 1): the covered set is EXACTLY the target row — the ink
        // flip is byte-identical to the old state-tied `[sel_disp]`.
        let b1 = morph_band(from, target_top, h, 1.0, &params);
        assert_eq!(
            covered_rows(&[b1], first_top, lh, 8),
            vec![target_disp],
            "settled band covers exactly the target row"
        );

        // EARLY (t = 0.1): the band is still down near its start row — the
        // TARGET keeps its off-band ink (NOT covered), and a LOWER row is.
        let b_early = morph_band(from, target_top, h, 0.1, &params);
        let cov_early = covered_rows(&[b_early], first_top, lh, 8);
        assert!(
            !cov_early.contains(&target_disp),
            "target keeps unselected ink until the band arrives (t=0.1: {cov_early:?})"
        );
        assert!(
            cov_early.iter().any(|&k| k > target_disp),
            "a row BELOW the target is under the moving band early (t=0.1: {cov_early:?})"
        );

        // The frontier climbs toward the target: the max covered row index never
        // sits below the target early and lands ON it at rest (the band arrives).
        let max_early = *cov_early.iter().max().unwrap();
        assert!(max_early > target_disp, "early frontier is below (t=0.1)");
    }

    #[test]
    fn two_shape_has_a_half_row_interpenetration_window() {
        // TASK 3 — the crossing is a REAL window, not just the coincident
        // endpoints: somewhere mid-flight the echo overlaps the lead by ROUGHLY
        // half a row (a genuine interpenetration the renderer fills one ladder
        // step brighter), distinct from the full-overlap ends and the
        // fully-cleared gap of a wide jump.
        let p = Choreo::TwoShape.params();
        let h = 24.0;
        let saw_half = (1..400)
            .map(|i| i as f32 / 400.0)
            .filter_map(|t| two_shape_band(0.0, PIN_JUMP_ROWS * h, h, t, &p).overlap)
            .any(|o| (o.height - h * 0.5).abs() < h * 0.15);
        assert!(saw_half, "two-shape crosses through a ~half-row overlap window");
    }

    #[test]
    fn parse_grammar_and_bad_input() {
        assert_eq!(parse_motion_force(""), None);
        assert_eq!(parse_motion_force("off"), None);
        assert_eq!(parse_motion_force("garbage"), None);
        assert_eq!(parse_motion_force("morph:2.0"), None, "phase out of range");
        assert_eq!(parse_motion_force("morph:notanum"), None);
        assert_eq!(
            parse_motion_force("morph"),
            Some(MotionForce { choreo: Choreo::Morph, phase: None })
        );
        assert_eq!(
            parse_motion_force("  TwoShape : 0.35 "),
            Some(MotionForce { choreo: Choreo::TwoShape, phase: Some(0.35) })
        );
        assert_eq!(
            parse_motion_force("slam:1.0"),
            Some(MotionForce { choreo: Choreo::Slam, phase: Some(1.0) })
        );
        assert_eq!(
            parse_motion_force("soft:0"),
            Some(MotionForce { choreo: Choreo::Soft, phase: Some(0.0) })
        );
    }
}
