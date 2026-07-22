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

use crate::theme::Srgb;
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

/// THE LIFECYCLE round (2026-07-23): the fraction of a star's own cycle it is
/// LIT at all — the rest is a DARK DWELL at TRUE zero (the star is gone). A star
/// that spends half its cycle absent is what makes the visible population
/// genuinely CHANGE (stars appear, shine, and die), the point the old
/// never-vanishing breath (floor 0.12) missed. TASTE TUNABLE — the twinkle FEEL
/// over real seconds is a live human-confirm.
pub const STAR_ACTIVE_FRAC: f32 = 0.5;

/// The per-star LOW-SATURATION tint palette — real-star colors: a cool
/// blue-white (the world's own ambient `tint`, the dominant), a neutral bright
/// WHITE, and a subtle warm CHAMPAGNE. All three sit inside the ambient
/// amber-guard (blue-white/white clear the caret's hue by >150°; champagne is
/// low-sat enough — HSL sat < 0.15 — to be exempt outright), so no star ever
/// reads as a second warm accent. The palette is the ONE owner both the
/// renderer (`prepare_stars_layer`) and the amber-guard law
/// (`theme::tests::ambient_stars_laws_hold_for_every_world`) iterate, so the
/// drawn tints and the law-checked tints can never drift.
pub const STAR_TINT_WHITE: Srgb = Srgb::rgb(0xE9, 0xEC, 0xF2);
/// The warm champagne pole — deliberately kept near white (HSL sat ~0.13) so it
/// stays clear of the ambient amber guard despite its warm hue sitting near the
/// gold caret; a real champagne saturation would read as a second accent.
pub const STAR_TINT_CHAMPAGNE: Srgb = Srgb::rgb(0xEF, 0xEE, 0xEA);

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

/// This star's brightness (tint alpha) at ambient `phase` (in cycles, wrapping
/// at [`crate::lava::LAVA_LOOP_CYCLES`]) — THE LIFECYCLE round's envelope,
/// replacing the old never-vanishing sine breath. A star spends a long DARK
/// DWELL at TRUE zero (absent), then RISES, briefly SHINES near its own peak,
/// and gently FADES back to zero — so the visible population genuinely CHANGES
/// (stars appear and die), the aliveness the old floor-0.12 breath lacked.
///
/// Two decorrelated rolls off the star's `seed`:
/// * its **cycle position** `u` in `[0, 1)` — an INTEGER rate per ambient loop
///   (so the envelope meets its own endpoint at the phase wrap, no seam —
///   `twinkle_is_seamless_across_the_ambient_loop_wrap`) plus a phase offset,
///   so no two stars are lit in unison; and
/// * its own **shine peak** somewhere in the visibility band `[floor, peak]` —
///   most stars barely kindle (near `floor`), a few blaze (near `peak`) — the
///   band whose ceiling the (relaxed, user-blessed) quiet-band law bounds.
///
/// Returns `0.0` throughout the dwell, rising to at most the star's own shine
/// peak (`<= peak`). Pure — a function of (seed, phase, band), never a clock.
pub fn brightness(seed: f32, phase: f32, floor: f32, peak: f32) -> f32 {
    let steps = TWINKLE_RATE_STEPS.max(1) as f32;
    let rate = TWINKLE_RATE_MIN as f32 + (seed * steps).floor().min(steps - 1.0);
    // A decorrelated roll off the same seed: the golden-ratio fold decorrelates
    // the phase offset from the rate pick above.
    let offset = (seed * 61.803_4).fract();
    let u = (rate * phase / crate::lava::LAVA_LOOP_CYCLES + offset).fract();
    // A THIRD decorrelated roll: this star's own shine peak within the band.
    let shine = floor + (peak - floor) * (seed * 17.13).fract();
    shine * lifecycle_env(u)
}

/// The per-cycle lifecycle SHAPE in `[0, 1]` at cycle position `u` in `[0, 1)`:
/// a DARK DWELL at zero for the `1 - STAR_ACTIVE_FRAC` inactive tail, then
/// within the lit window a smoothstep RISE (0→1), a brief SHINE plateau (1), and
/// a longer, gentler smoothstep FADE (1→0). Continuous and `0.0` at both ends of
/// the lit window (and everywhere in the dwell), so the envelope is seamless
/// across the `u` wrap — combined with the integer per-loop rate, `brightness`
/// meets its own endpoint at the ambient-loop wrap. Pure.
fn lifecycle_env(u: f32) -> f32 {
    let active = STAR_ACTIVE_FRAC.clamp(0.05, 1.0);
    if u >= active {
        return 0.0; // the dark dwell — the star is gone
    }
    let v = u / active; // [0, 1) across the lit window
    fn smoothstep(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
    // rise 0.00..0.30 -> shine plateau 0.30..0.50 -> fade 0.50..1.00 (the fade
    // is longer than the rise, so a star kindles a touch quicker than it dies).
    if v < 0.30 {
        smoothstep(v / 0.30)
    } else if v < 0.50 {
        1.0
    } else {
        1.0 - smoothstep((v - 0.50) / 0.50)
    }
}

/// This star's tint, picked from the [low-saturation real-star palette](STAR_TINT_WHITE)
/// by a decorrelated roll off its `seed`: mostly the world's own cool blue-white
/// `base` (the night-sky dominant), sometimes a neutral bright WHITE, rarely a
/// warm CHAMPAGNE. The ONE owner both the renderer and the amber-guard law read,
/// so the drawn and law-checked tints can never drift. Pure + deterministic.
pub fn star_tint(base: Srgb, seed: f32) -> Srgb {
    let [blue_white, white, champagne] = star_palette(base);
    // Decorrelated from the rate/offset/shine folds (a different prime multiplier).
    let r = (seed * 7.19).fract();
    if r < 0.62 {
        blue_white
    } else if r < 0.85 {
        white
    } else {
        champagne
    }
}

/// The full star-tint palette for a world whose ambient `base` tint is given —
/// `[base, white, champagne]`. The ONE owner both the renderer (via
/// [`star_tint`]) and the amber-guard/visibility-band law iterate, so a tint the
/// law never checks can never be drawn.
pub fn star_palette(base: Srgb) -> [Srgb; 3] {
    [base, STAR_TINT_WHITE, STAR_TINT_CHAMPAGNE]
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

    /// THE SCALE-INVARIANCE GUARANTEE the DPI fix rides on (the twinkling-stars
    /// density/size bug): [`layout`] is VIEWPORT-space and unit-agnostic, so
    /// scattering over an `s×` viewport with an `s×` cell reproduces the SAME grid
    /// — identical population, every star at exactly `s×` its position, same seeds.
    /// The renderer (`prepare_stars_layer`) leans on exactly this: it multiplies the
    /// authored physical `cell_px` by the total logical->physical factor `s`
    /// (user-zoom × device-DPI) before laying out over the `s×` physical viewport, so
    /// the LOGICAL density is constant at any DPI. (`s = 2.0` is a power of two, so
    /// the `ceil(w/cell)` grid dims are BIT-identical and the counts are EXACTLY
    /// equal; the render-side `currawong_star_field_is_dpi_invariant_in_logical_space`
    /// proves the fix is actually WIRED at real pixels — this proves the layout math
    /// underneath it.)
    #[test]
    fn layout_is_scale_invariant_in_logical_space() {
        let (cell, density, _, _) = currawong_stars();
        let (lw, lh) = (1000.0_f32, 700.0_f32);
        let base = layout(lw, lh, cell, density);
        assert!(base.len() > 10, "the base sky must have a real population");
        let s = 2.0_f32;
        let scaled = layout(lw * s, lh * s, cell * s, density);
        assert_eq!(
            scaled.len(),
            base.len(),
            "an s× viewport with an s× cell must scatter the SAME population as 1× — \
             constant logical density is what makes the field DPI-invariant \
             (got {} at s={s} vs {} at 1×)",
            scaled.len(),
            base.len(),
        );
        for (b, sc) in base.iter().zip(scaled.iter()) {
            assert!(
                (sc.x - b.x * s).abs() < 1e-2 && (sc.y - b.y * s).abs() < 1e-2,
                "each star must land at exactly s× its logical position \
                 (({}, {}) × {s} vs ({}, {}))",
                b.x, b.y, sc.x, sc.y,
            );
            assert!(
                (sc.seed - b.seed).abs() < 1e-6,
                "the same grid cell must keep its seed across scales ({} vs {})",
                b.seed, sc.seed,
            );
        }
    }

    /// THE LIFECYCLE ENVELOPE (2026-07-23): over a full loop each star (a) DWELLS
    /// at TRUE zero for a real stretch — it goes fully absent, not merely dim —
    /// and (b) LIGHTS UP to its own shine peak, which lands inside the visibility
    /// band `[floor, peak]` and never exceeds `peak`. The old never-vanishing
    /// breath (which stayed `>= floor` forever) fails arm (a).
    #[test]
    fn lifecycle_dwells_dark_then_shines_within_its_band() {
        let (_, _, peak, floor) = currawong_stars();
        let stars = layout(1200.0, 800.0, 34.0, 0.30);
        for s in stars.iter().take(64) {
            let mut lo = f32::MAX;
            let mut hi = f32::MIN;
            for i in 0..400 {
                let phase = LAVA_LOOP_CYCLES * (i as f32 / 400.0);
                let b = brightness(s.seed, phase, floor, peak);
                assert!(
                    b <= peak + 1e-4,
                    "brightness {b} exceeded the band ceiling {peak}"
                );
                assert!(b >= -1e-4, "brightness {b} went negative");
                lo = lo.min(b);
                hi = hi.max(b);
            }
            // (a) DARK DWELL at true zero — the star genuinely disappears.
            assert!(
                lo < 0.01,
                "star seed {} never reaches its dark dwell (min {lo}) — it must fully vanish",
                s.seed
            );
            // (b) SHINES to at least the band floor (a real, seeable glint), and
            // no higher than the ceiling. Its own shine peak lives in [floor, peak].
            assert!(
                (floor - 1e-3..=peak + 1e-3).contains(&hi),
                "star seed {}'s shine peak {hi} fell outside the band [{floor}, {peak}]",
                s.seed
            );
        }
    }

    /// THE POPULATION CHANGES between phases — the round's headline: stars appear
    /// and die, so the set of LIT stars (brightness above a visible threshold)
    /// differs from one phase to another, and each phase's lit-count is
    /// DETERMINISTIC (a pure function of the phase — the capture oracle relies on
    /// it). A never-vanishing breath would keep the SAME (full) population lit at
    /// every phase and fail the "differs" arm.
    #[test]
    fn lifecycle_population_changes_between_phases_and_is_deterministic() {
        let (_, _, peak, floor) = currawong_stars();
        let stars = layout(1200.0, 800.0, 34.0, 0.30);
        // A star is "lit" when its envelope clears a hair off zero (the same
        // sense the renderer's `alpha == 0` cull uses).
        let lit_at = |phase: f32| -> Vec<bool> {
            stars
                .iter()
                .map(|s| brightness(s.seed, phase, floor, peak) > 0.01)
                .collect()
        };
        let a = lit_at(0.0);
        let b = lit_at(3.1);
        // Deterministic: recomputing the same phase yields the identical set.
        assert_eq!(a, lit_at(0.0), "the lit population must be a pure function of the phase");
        let count_a = a.iter().filter(|&&l| l).count();
        let count_b = b.iter().filter(|&&l| l).count();
        assert!(
            count_a > 0 && count_a < a.len(),
            "at a given phase SOME stars are lit and some are dark-dwelling (lit {count_a}/{})",
            a.len()
        );
        // The population genuinely turns over: many stars flip lit<->dark.
        let flips = a.iter().zip(b.iter()).filter(|(x, y)| x != y).count();
        assert!(
            flips > a.len() / 10,
            "the visible sky must change between phases — only {flips} of {} stars flipped \
             (lit {count_a} -> {count_b})",
            a.len()
        );
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
        let stars = layout(1200.0, 800.0, 34.0, 0.30);
        // At one fixed mid phase, the population's brightnesses SPREAD — the
        // "individually phased" contract (a shared phase would put every star at
        // the same value; that's a pulse, not a sky). With the lifecycle the
        // spread is starker still: some dark-dwelling at zero, some mid-shine.
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

    /// THE PER-STAR TINT PALETTE (2026-07-23): every star's tint is one of the
    /// three low-sat real-star palette entries (`star_palette`), the pick is a
    /// pure + deterministic function of the seed, and across a spread of seeds
    /// ALL THREE colors actually appear (the sky is not monochrome — blue-white
    /// dominant, with white and champagne glints).
    #[test]
    fn star_tint_is_a_deterministic_pick_from_the_low_sat_palette() {
        let base = Srgb::rgb(0x9D, 0xB0, 0xCF); // Currawong's ambient tint
        let palette = star_palette(base);
        let mut seen = [0usize; 3];
        for i in 0..1000 {
            let seed = i as f32 / 1000.0;
            let tint = star_tint(base, seed);
            // Deterministic.
            assert_eq!(tint, star_tint(base, seed), "star_tint must be pure");
            // A palette member, never an off-palette color.
            let idx = palette.iter().position(|p| *p == tint).unwrap_or_else(|| {
                panic!("star_tint returned {tint:?}, not a member of {palette:?}")
            });
            seen[idx] += 1;
        }
        for (i, &c) in seen.iter().enumerate() {
            assert!(c > 0, "palette color {i} ({:?}) never got picked", palette[i]);
        }
        // The world's own blue-white base dominates (a cool night sky), the
        // whites are the rarer glints.
        assert!(
            seen[0] > seen[1] && seen[0] > seen[2],
            "the world's blue-white base must dominate the sky (counts {seen:?})"
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
