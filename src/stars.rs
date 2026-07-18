//! src/stars.rs — the TWINKLING-STARS ambient ground: awl's second time-varying
//! page ground, and the QUIET pole of the user's "aliveness ≠ loudness"
//! principle (2026-07-18: most worlds should feel ALIVE, including quiet ones —
//! twinkling stars are maximally quiet, unmistakably alive). Tiny points
//! scattered through the page-mode MARGINS — never the writing column — each
//! breathing its brightness on its own slow, individually phased seconds-scale
//! cycle. Currawong (the near-black OLED night world, named for the Australian
//! night bird whose voice is the quiet dark) is the one assignment.
//!
//! THE SHAPE (deliberately the lava lamp's, one register quieter):
//!
//! * **Capability, not a code path.** The renderer reads ONE caps field —
//!   [`crate::theme::AmbientStyle`] on `Theme::render_caps.ambient` — never a
//!   world name (`theme_caps_law`). A second world adopts stars by data alone.
//! * **One ambient clock, two consumers.** The twinkle phase IS the lava
//!   phase (`TextPipeline::lava_phase`), advanced by the live App's single
//!   ~10 fps `WaitUntil` ambient tick (`App::about_to_wait`), gated by the
//!   SAME [`crate::lava::lava_should_tick`] cadence gate (`ambient_motion`
//!   config on, motion not reduced, window focused, no transient pause), and
//!   resolved through the SAME [`crate::lava::lava_phase_for`] determinism
//!   ladder — env override > Reduce-Motion freeze > the App-driven phase
//!   (which stays the frozen `0.0` in every headless capture, since the
//!   capture never ticks). A non-ambient world schedules ZERO frames (0% idle).
//! * **Layout is a position hash, not entropy.** [`layout`] scatters one
//!   candidate star per fixed pixel grid cell via a pure INTEGER hash of the
//!   cell id ([`hash01`] — deliberately not a float `sin`-fract hash, whose
//!   libm results vary across platforms; an integer mix is bit-exact
//!   everywhere). Two captures are byte-identical; a resize keeps every
//!   surviving star anchored to its exact pixel cell.
//! * **Margins only, by a hard gate.** [`in_margin`] rejects any star whose
//!   full quad (AA fringe included) could touch the writing column band plus
//!   a breathing gap — the placement LAW, shared verbatim by the renderer
//!   (`TextPipeline::prepare_stars_layer`) and the law tests, so they can
//!   never disagree.
//! * **Quads through the existing owner.** The dots render as tiny
//!   fully-rounded quads through `SelectionPipeline::prepare_multicolor`
//!   (the same per-instance-color path the writing-streaks heatmap rides) —
//!   no new pipeline, no new shader, nothing new for the WebGL2 fallback to
//!   validate. Per-frame work is phase arithmetic over the visible star set
//!   (the proto-cache shape: layout built once per size/params, culled +
//!   tinted per frame).

use std::sync::OnceLock;

/// Breathing room (px) between the writing column's edge and the nearest star:
/// the margin gate rejects a star whose quad could land inside the column band
/// widened by this gap on each side, so no point ever crowds the text edge.
/// TASTE TUNABLE — flagged for live review (like `lava::MARGIN_GAP_PX`).
pub const STAR_MARGIN_GAP_PX: f32 = 10.0;

/// The SLOWEST twinkle rate, in whole cycles per full ambient loop. The ambient
/// phase wraps at [`crate::lava::LAVA_LOOP_CYCLES`] (~67 s at the shipped
/// speed), and every per-star rate is an INTEGER number of cycles per loop —
/// the structural guarantee that the twinkle meets its own endpoint at the
/// wrap (no seam; law `twinkle_is_seamless_across_the_ambient_loop_wrap`).
/// 3..=8 cycles per ~67 s loop = one breath every ~8–22 s: seconds-scale
/// breathing, never flicker. TASTE TUNABLE.
pub const TWINKLE_RATE_MIN: u32 = 3;

/// How many integer rate steps above [`TWINKLE_RATE_MIN`] a star may sit
/// (rates span `TWINKLE_RATE_MIN ..= TWINKLE_RATE_MIN + TWINKLE_RATE_STEPS - 1`).
pub const TWINKLE_RATE_STEPS: u32 = 6;

/// One scattered star: a jittered position (px, top-left origin) plus its own
/// hash seed in `[0, 1)` — the seed derives the star's individual twinkle rate
/// and phase offset ([`brightness`]), so no two stars breathe in unison.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Star {
    pub x: f32,
    pub y: f32,
    pub seed: f32,
}

// --- The position hash (pure, integer, bit-exact) -----------------------------

/// A 2D integer mix (xxhash/PCG-style avalanche) — the deterministic core every
/// star property derives from. Pure integer arithmetic: bit-exact on every
/// platform and every run (a float `sin`-fract hash — the background shader's
/// idiom — is fine ON the GPU where one device renders one capture, but a
/// CPU-side layout must not inherit libm's per-platform `sin` variance).
fn hash2(ix: u32, iy: u32) -> u32 {
    let mut h = ix
        .wrapping_mul(0x9E37_79B1)
        ^ iy.wrapping_mul(0x85EB_CA6B)
        ^ 0x27D4_EB2F;
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    h
}

/// The cell hash folded to `[0, 1)`, salted so one cell yields independent
/// rolls (presence / jitter-x / jitter-y / seed).
fn hash01(ix: u32, iy: u32, salt: u32) -> f32 {
    (hash2(ix.wrapping_add(salt.rotate_left(9)), iy ^ salt) >> 8) as f32 / 16_777_216.0
}

// --- Layout (built once per size/params — the proto half) ---------------------

/// Scatter the star field for a `w`×`h` px viewport: one CANDIDATE position per
/// `cell_px` grid cell, kept iff its presence roll clears `density`, jittered
/// inside the cell's middle band (so a star's quad + AA fringe never leaves its
/// own cell — star count is exactly the presence count, no cross-cell overlap).
/// Pure + deterministic: two calls with the same inputs are `==` (law
/// `layout_is_deterministic_and_stays_in_viewport`). VIEWPORT-space, column
/// independent — the margin cull happens per frame against the LIVE column
/// geometry ([`in_margin`]), so an adaptive-column shift or resize re-culls the
/// same anchored field rather than re-scattering it.
pub fn layout(w: f32, h: f32, cell_px: f32, density: f32) -> Vec<Star> {
    let cell = cell_px.max(4.0);
    let cols = (w / cell).ceil() as u32;
    let rows = (h / cell).ceil() as u32;
    let mut stars = Vec::new();
    for iy in 0..rows {
        for ix in 0..cols {
            if hash01(ix, iy, 0x51A2) >= density {
                continue;
            }
            // Jitter inside the cell's middle 70% so the dot + its 1px AA
            // fringe stays inside the cell for any sane size (< cell * 0.3).
            let jx = 0.15 + 0.70 * hash01(ix, iy, 0x9E77);
            let jy = 0.15 + 0.70 * hash01(ix, iy, 0xC0DE);
            let x = (ix as f32 + jx) * cell;
            let y = (iy as f32 + jy) * cell;
            if x >= w || y >= h {
                continue;
            }
            stars.push(Star {
                x,
                y,
                seed: hash01(ix, iy, 0x7EED),
            });
        }
    }
    stars
}

// --- Twinkle (the per-frame phase arithmetic) ---------------------------------

/// This star's brightness at ambient `phase` (in cycles, wrapping at
/// [`crate::lava::LAVA_LOOP_CYCLES`]): a slow sine breath, individually rated
/// and offset by the star's own `seed`, eased quadratically so a star spends
/// most of its cycle near its dim `floor` and BRIEFLY glints toward `peak` —
/// the twinkle read, not a uniform pulse. Returns the tint alpha in
/// `[floor, peak]`. The rate is an INTEGER cycle count per ambient loop, so the
/// breath meets its own endpoint exactly at the phase wrap (no seam). Pure —
/// a function of (seed, phase, band), never a clock.
pub fn brightness(seed: f32, phase: f32, floor: f32, peak: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let steps = TWINKLE_RATE_STEPS.max(1) as f32;
    let rate = TWINKLE_RATE_MIN as f32 + (seed * steps).floor().min(steps - 1.0);
    // A second decorrelated roll off the same seed: the golden-ratio fold
    // decorrelates the phase offset from the rate pick above.
    let offset = (seed * 61.803_4).fract();
    let t = 0.5 + 0.5 * (tau * (rate * phase / crate::lava::LAVA_LOOP_CYCLES + offset)).sin();
    // Quadratic ease: dim-biased dwell, brief glints.
    floor + (peak - floor) * (t * t)
}

// --- The margin gate (THE placement law, one owner) ---------------------------

/// May a star centered at `x` (with dot radius `half_px`) draw this frame?
/// True iff its FULL quad — the dot, plus the 1px antialiasing fringe the quad
/// shader extends — lies strictly outside the writing column band
/// `[col_left, col_right]` widened by `gap` px on each side. This is THE
/// placement law: the renderer culls through this exact predicate and the law
/// tests assert over it (plus real pixels), so "no star under the text column"
/// can never drift between the two. Page-mode OFF passes the full canvas as
/// the column (col spans everything → every star culled → no stars without
/// margins, matching the background's own collapse).
pub fn in_margin(x: f32, half_px: f32, col_left: f32, col_right: f32, gap: f32) -> bool {
    let e = half_px + 1.0; // the shader's 1px AA margin
    x + e <= col_left - gap || x - e >= col_right + gap
}

// --- The dev-only gallery knob (AWL_STARS_PHASE=<f32>) ------------------------
//
// Mirrors `AWL_LAVA` / `AWL_CJK_FORCE` exactly: read ONCE at startup, memoized,
// a TOTAL no-op unless set — so normal + headless determinism is untouched when
// absent. Pins the twinkle to a FIXED phase so the gallery can capture two
// deterministic mid-breath frames (the brightness A/B the taste gate looks at).

/// Parse the knob's value: a finite float phase (in cycles). Pure, so the
/// grammar is unit-testable without touching the environment.
fn parse_phase(raw: &str) -> Option<f32> {
    let p: f32 = raw.trim().parse().ok()?;
    p.is_finite().then_some(p)
}

/// The dev gallery override's fixed twinkle phase, if `AWL_STARS_PHASE` was set
/// at startup and parses. Consumed by `TextPipeline::stars_render_phase` through
/// [`crate::lava::lava_phase_for`] (env wins outright), exactly the lava knob's
/// seam. `None` (every normal + headless run) = no override.
pub fn env_phase() -> Option<f32> {
    static ONCE: OnceLock<Option<f32>> = OnceLock::new();
    *ONCE.get_or_init(|| {
        std::env::var("AWL_STARS_PHASE")
            .ok()
            .as_deref()
            .and_then(parse_phase)
    })
}

// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lava::{lava_phase_for, lava_should_tick, LAVA_FROZEN_PHASE, LAVA_LOOP_CYCLES};
    use crate::theme::THEMES;

    /// Currawong's shipped params, read off the world DATA itself (never a
    /// second copy that could drift).
    fn currawong_stars() -> (f32, f32, f32, f32) {
        let caps = crate::theme::CURRAWONG.render_caps.ambient;
        let Some((_tint, cell, density, _size, peak, floor)) = caps.stars_params() else {
            panic!("Currawong must ship AmbientStyle::Stars (the round's one assignment)");
        };
        (cell, density, peak, floor)
    }

    #[test]
    fn layout_is_deterministic_and_stays_in_viewport() {
        let (cell, density, _, _) = currawong_stars();
        let a = layout(1200.0, 800.0, cell, density);
        let b = layout(1200.0, 800.0, cell, density);
        assert_eq!(a, b, "the star layout must be a pure position hash — bit-identical runs");
        assert!(
            a.len() > 20,
            "a 1200x800 field at the shipped density must scatter a real population (got {})",
            a.len()
        );
        for s in &a {
            assert!(
                s.x >= 0.0 && s.x < 1200.0 && s.y >= 0.0 && s.y < 800.0,
                "star ({}, {}) fell off the viewport",
                s.x,
                s.y
            );
            assert!((0.0..1.0).contains(&s.seed), "seed {} out of [0,1)", s.seed);
        }
        // Density is a real dial: doubling it grows the population.
        let denser = layout(1200.0, 800.0, cell, (density * 2.0).min(1.0));
        assert!(
            denser.len() > a.len(),
            "density must scale the population ({} !> {})",
            denser.len(),
            a.len()
        );
    }

    #[test]
    fn twinkle_stays_inside_its_band_and_actually_breathes() {
        let (_, _, peak, floor) = currawong_stars();
        let stars = layout(1200.0, 800.0, 34.0, 0.16);
        for s in stars.iter().take(64) {
            let mut lo = f32::MAX;
            let mut hi = f32::MIN;
            for i in 0..200 {
                let phase = LAVA_LOOP_CYCLES * (i as f32 / 200.0);
                let b = brightness(s.seed, phase, floor, peak);
                assert!(
                    (floor - 1e-4..=peak + 1e-4).contains(&b),
                    "brightness {b} escaped the authored [{floor}, {peak}] band"
                );
                lo = lo.min(b);
                hi = hi.max(b);
            }
            // The breath is REAL: over a full loop each star sweeps most of its
            // band (a flat star would be present but dead — the round's whole
            // point is alive).
            assert!(
                hi - lo > 0.7 * (peak - floor),
                "star seed {} barely breathes ({lo}..{hi})",
                s.seed
            );
        }
    }

    #[test]
    fn twinkle_is_seamless_across_the_ambient_loop_wrap() {
        let (_, _, peak, floor) = currawong_stars();
        for i in 0..100 {
            let seed = i as f32 / 100.0;
            let at_zero = brightness(seed, 0.0, floor, peak);
            let at_wrap = brightness(seed, LAVA_LOOP_CYCLES, floor, peak);
            assert!(
                (at_zero - at_wrap).abs() < 1e-3,
                "seed {seed}: the twinkle must meet its own endpoint at the phase wrap \
                 ({at_zero} vs {at_wrap}) — integer rates per loop guarantee it"
            );
        }
    }

    #[test]
    fn stars_are_individually_phased_never_in_unison() {
        let (_, _, peak, floor) = currawong_stars();
        let stars = layout(1200.0, 800.0, 34.0, 0.16);
        // At one fixed mid phase, the population's brightnesses SPREAD — the
        // "individually phased breathing" contract (a shared phase would put
        // every star at the same value; that's a pulse, not a sky).
        let phase = 0.37;
        let values: Vec<f32> = stars
            .iter()
            .take(64)
            .map(|s| brightness(s.seed, phase, floor, peak))
            .collect();
        let lo = values.iter().cloned().fold(f32::MAX, f32::min);
        let hi = values.iter().cloned().fold(f32::MIN, f32::max);
        assert!(
            hi - lo > 0.5 * (peak - floor),
            "at a fixed phase the sky must show a real brightness spread ({lo}..{hi})"
        );
    }

    #[test]
    fn margin_gate_rejects_the_whole_column_band_and_gap() {
        let (col_l, col_r, gap, half) = (300.0, 900.0, STAR_MARGIN_GAP_PX, 1.3);
        // Sweep a fine grid over the canvas: every accepted x must be strictly
        // clear of the widened band; every x inside the band must be rejected.
        let mut accepted_left = 0;
        let mut accepted_right = 0;
        for i in 0..=1200 {
            let x = i as f32;
            let inside_band = x + half + 1.0 > col_l - gap && x - half - 1.0 < col_r + gap;
            let ok = in_margin(x, half, col_l, col_r, gap);
            assert_eq!(
                ok, !inside_band,
                "x={x}: the margin gate and the band predicate must be exact complements"
            );
            if ok && x < col_l {
                accepted_left += 1;
            }
            if ok && x > col_r {
                accepted_right += 1;
            }
        }
        assert!(accepted_left > 0 && accepted_right > 0, "both margins must admit stars");
        // Page-off collapse: the column spans the whole canvas -> nothing passes.
        for i in 0..=1200 {
            assert!(
                !in_margin(i as f32, half, 0.0, 1200.0, gap),
                "page-off (column == canvas) must cull every star"
            );
        }
    }

    #[test]
    fn currawong_alone_carries_the_stars_and_the_ambient_gate_composes() {
        // The assignment: exactly one world ships Stars (the taste-round's one
        // pick), and the ONE scheduling gate reads it.
        for t in THEMES.iter() {
            let has_stars = t.render_caps.ambient.is_animated();
            assert_eq!(
                has_stars,
                t.name == "Currawong",
                "{}: AmbientStyle::Stars is Currawong's assignment alone (a second world \
                 is a conscious data edit + its own gallery)",
                t.name
            );
            // The one owner composes: lava OR stars, nothing else.
            assert_eq!(
                t.has_ambient_motion(),
                t.background.is_lava() || has_stars,
                "{}: has_ambient_motion must be exactly the lava/stars OR — one owner",
                t.name
            );
        }
        // Reduce Motion freezes the twinkle at the same resolver the lava rides:
        // reduced -> the frozen phase regardless of the stored one (static stars,
        // present but not twinkling), and the cadence gate refuses to arm.
        assert_eq!(lava_phase_for(1.23, true, None), LAVA_FROZEN_PHASE);
        assert!(!lava_should_tick(true, true, true, true, false));
        // `ambient_motion = false` (the config kill-switch) also refuses.
        assert!(!lava_should_tick(true, false, false, true, false));
    }

    /// THE STARS-ARM-THE-TICK-LIKE-LAVA LAW (user report 2026-07-18: "they don't
    /// twinkle though?" — Currawong's stars render but sit STATIC while the lava
    /// worlds animate). The live App arms its ~10 fps ambient tick by feeding the
    /// ACTIVE world's `has_ambient_motion()` into `lava_should_tick` as its `active`
    /// term (`App::about_to_wait`, `let lava_active = active().has_ambient_motion()`).
    /// A stars-only world (Currawong, NOT lava) must therefore arm the tick EXACTLY
    /// like a lava world — the widening from the old `is_lava()` gate to the shared
    /// `has_ambient_motion()` one, which the vanish-fix's lava.rs deletion sweep was
    /// suspected of reverting. This pins the COMPOSITION the App performs (the prior
    /// test only fed a hardcoded `true`), so any future regression of the tick-arm
    /// term back to lava-only fails here rather than silently freezing the stars.
    #[test]
    fn a_stars_only_world_arms_the_ambient_tick_exactly_like_a_lava_world() {
        let world = |name: &str| THEMES.iter().find(|t| t.name == name).expect("real world");
        let (currawong, firetail, magpie) =
            (world("Currawong"), world("Firetail"), world("Magpie"));
        assert!(
            currawong.render_caps.ambient.is_animated() && !currawong.background.is_lava(),
            "Currawong is the stars-only (non-lava) ambient world"
        );
        assert!(firetail.background.is_lava(), "Firetail is a lava world");
        assert!(
            !magpie.has_ambient_motion(),
            "Magpie is a static world (the vanish's light destination)"
        );
        // Compose EXACTLY as `App::about_to_wait` does: active-world ambient bit ->
        // `lava_should_tick`'s `active` term, with normal live conditions (ambient
        // on, motion not reduced, focused, not paused).
        let arms = |t: &crate::theme::Theme| lava_should_tick(t.has_ambient_motion(), true, false, true, false);
        assert!(arms(firetail), "a lava world arms the tick");
        assert!(
            arms(currawong),
            "a stars-only world MUST arm the tick just like a lava world (the frozen-stars regression guard)"
        );
        assert_eq!(arms(currawong), arms(firetail), "stars and lava arm identically");
        assert!(!arms(magpie), "a static world schedules zero ambient frames");
    }

    #[test]
    fn env_phase_grammar_accepts_finite_floats_only() {
        assert_eq!(parse_phase("0.5"), Some(0.5));
        assert_eq!(parse_phase(" 1.75 "), Some(1.75), "whitespace-tolerant");
        assert_eq!(parse_phase("0"), Some(0.0));
        for bad in ["", "wat", "NaN", "inf", "-inf"] {
            assert_eq!(parse_phase(bad), None, "expected None for {bad:?}");
        }
    }
}
