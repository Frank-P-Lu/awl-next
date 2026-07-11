//! ORDERED (BAYER) DITHER — the pure math shared by two shader-side consumers,
//! kept here as a Rust MIRROR for unit testing (WGSL itself can't run in
//! `cargo test`; `shaders/background.wgsl` and `shaders/selection.wgsl` each
//! carry their OWN copy of the same 8x8 matrix — a small, deliberate
//! duplication across the Rust/WGSL boundary, the same pattern this codebase
//! already accepts for `srgba_u8_to_linear` between `selection.rs` and
//! `background.rs`. If the matrix ever needs to change, change it in all
//! three places and re-run `dither::tests` + the GPU pixel-proof tests below.
//!
//! Two independent uses, one matrix:
//! 1. **BANDING KILL** (`background.wgsl`'s margin gradient): a ±half-LSB
//!    offset added to the smooth interpolated color BEFORE the GPU quantizes
//!    it to the 8-bit sRGB render target — imperceptible as texture, breaks up
//!    the banding a smooth `mix()` produces on a wide gradient. A FLAT gradient
//!    (`from == to`, e.g. Wagtail's one-bit background) must see this offset
//!    as an exact NO-OP — any nonzero nudge on a pure `#000000` would round to
//!    a forbidden off-black, breaking the one-bit law.
//! 2. **THE ONE WAGTAIL HIGHLIGHT TEXTURE** (`selection.wgsl`'s dither mode,
//!    used by `==highlight==` spans AND search matches on a one-bit world): a
//!    HARD threshold (not an offset) at a fixed density — every pixel is
//!    either the pure quad color (fully opaque) or fully transparent, never a
//!    fractional alpha, so no blending step can introduce a forbidden
//!    intermediate grey.

/// The classic 8x8 ordered (Bayer) dither matrix, values `0..64` — each cell's
/// rank among its 64 neighbors, chosen so nearby cells never share a rank
/// (avoids the "clumping" a naive per-pixel hash would produce). A pure
/// function of `(x, y)` alone — no time, no random state — so both the
/// gradient offset and the highlight stipple stay deterministic across
/// captures.
///
/// `#[allow(dead_code)]` on this and the three functions below: the REAL
/// runtime dither happens in `shaders/background.wgsl`/`shaders/
/// selection.wgsl`'s own duplicated copy of this exact matrix/math — these
/// Rust functions exist ONLY as the pure-math mirror `dither::tests` exercises
/// (mirroring `SelectionPipeline::instance_count`'s established
/// `#[allow(dead_code)]` idiom for a test-only accessor). `WAGTAIL_HIGHLIGHT_
/// DITHER_DENSITY` is the one exception with a real non-test caller
/// (`render::spans::wagtail_dither_density`).
#[rustfmt::skip]
#[allow(dead_code)]
pub(super) const BAYER8: [u8; 64] = [
     0, 32,  8, 40,  2, 34, 10, 42,
    48, 16, 56, 24, 50, 18, 58, 26,
    12, 44,  4, 36, 14, 46,  6, 38,
    60, 28, 52, 20, 62, 30, 54, 22,
     3, 35, 11, 43,  1, 33,  9, 41,
    51, 19, 59, 27, 49, 17, 57, 25,
    15, 47,  7, 39, 13, 45,  5, 37,
    63, 31, 55, 23, 61, 29, 53, 21,
];

/// The Bayer threshold at pixel `(x, y)`, normalized to `[0, 1)` — cell `(x %
/// 8, y % 8)`'s rank over 64. Tiles seamlessly across the whole canvas (pure
/// modulo indexing), so a highlight band spanning several quads reads as ONE
/// continuous texture rather than restarting phase per quad.
#[allow(dead_code)]
pub(super) fn bayer_threshold01(x: u32, y: u32) -> f32 {
    let idx = ((y % 8) * 8 + (x % 8)) as usize;
    BAYER8[idx] as f32 / 64.0
}

/// TASTE TUNABLE: the ONE Wagtail highlight/search-match dither density —
/// deliberately a single fixed value, not a ladder (the razor: one kind of
/// emphasis, one texture — see THEMES.md's 1-bit section). ~25% pattern
/// coverage reads as a clear stipple band without swallowing the covered text.
pub(super) const WAGTAIL_HIGHLIGHT_DITHER_DENSITY: f32 = 0.25;

/// The BANDING-KILL gradient offset at pixel `(x, y)`, in units of ONE 8-bit
/// sRGB step (i.e. the caller adds this directly to a `[0,1]` float channel
/// before the GPU quantizes it to `u8`) — a signed value in `(-0.5/255,
/// 0.5/255)`. `flat` is `true` for a gradient whose `from == to` (Wagtail's
/// one-bit background, or any world's degenerate zero-length gradient): the
/// offset is an EXACT `0.0` then, never merely small — a flat color must stay
/// bit-identical, since any nonzero nudge on `#000000`/`#FFFFFF` would round
/// to a forbidden third value under the one-bit law.
#[allow(dead_code)]
pub(super) fn gradient_dither_offset(x: u32, y: u32, flat: bool) -> f32 {
    if flat {
        return 0.0;
    }
    (bayer_threshold01(x, y) - 0.5) / 255.0
}

/// THE ONE WAGTAIL HIGHLIGHT TEXTURE's per-pixel hit test: pixel `(x, y)` is
/// "on" (draw the pure quad color, fully opaque) iff its Bayer rank falls
/// under `density`'s proportional cutoff — an ordered-dither threshold, not a
/// random roll, so the ~25% coverage is exact and deterministic.
#[allow(dead_code)]
pub(super) fn highlight_dither_on(x: u32, y: u32, density: f32) -> bool {
    bayer_threshold01(x, y) < density
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 8x8 Bayer matrix must be a permutation of `0..64` — every rank used
    /// exactly once. A typo'd duplicate rank would silently break the ordered
    /// (as opposed to merely "some values repeat, banding reappears") property.
    #[test]
    fn bayer_matrix_is_a_permutation_of_0_to_63() {
        let mut seen = [false; 64];
        for &v in BAYER8.iter() {
            assert!((v as usize) < 64, "rank {v} out of range");
            assert!(!seen[v as usize], "rank {v} appears more than once");
            seen[v as usize] = true;
        }
        assert!(seen.iter().all(|&s| s), "every rank 0..64 must appear");
    }

    /// `bayer_threshold01` stays in `[0, 1)` for a wide sweep of pixel
    /// coordinates (tiling never overflows the unit range).
    #[test]
    fn bayer_threshold_is_always_in_unit_range() {
        for y in 0..40u32 {
            for x in 0..40u32 {
                let t = bayer_threshold01(x, y);
                assert!((0.0..1.0).contains(&t), "({x},{y}) -> {t} out of [0,1)");
            }
        }
    }

    /// Tiling: the threshold at `(x, y)` and `(x + 8, y + 8)` (one full period
    /// in both axes) must agree exactly — the pattern repeats seamlessly, so a
    /// wide highlight band drawn as several quads reads as one texture.
    #[test]
    fn bayer_threshold_tiles_every_8_pixels() {
        for y in 0..17u32 {
            for x in 0..17u32 {
                assert_eq!(bayer_threshold01(x, y), bayer_threshold01(x + 8, y));
                assert_eq!(bayer_threshold01(x, y), bayer_threshold01(x, y + 8));
            }
        }
    }

    /// THE FLAT-GRADIENT NO-OP (deliverable 1's one-bit interplay guard): a
    /// flat gradient (`from == to`, e.g. Wagtail's background) must see EXACTLY
    /// zero offset at every pixel, never merely a small one — the one-bit law
    /// has no tolerance for "almost zero".
    #[test]
    fn flat_gradient_dither_offset_is_an_exact_no_op_everywhere() {
        for y in 0..20u32 {
            for x in 0..20u32 {
                assert_eq!(
                    gradient_dither_offset(x, y, true),
                    0.0,
                    "flat gradient must produce an EXACT zero offset at ({x},{y})"
                );
            }
        }
    }

    /// A REAL (non-flat) gradient's offset spans the full ±half-LSB range
    /// (proof the dither actually varies pixel-to-pixel, not a constant), and
    /// every value stays strictly within one 8-bit step either way — small
    /// enough to be imperceptible as its own texture.
    #[test]
    fn real_gradient_dither_offset_spans_a_half_lsb_band() {
        let half_lsb = 0.5 / 255.0;
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for y in 0..8u32 {
            for x in 0..8u32 {
                let o = gradient_dither_offset(x, y, false);
                assert!(
                    o > -half_lsb - 1e-9 && o < half_lsb + 1e-9,
                    "offset {o} at ({x},{y}) exceeds ±half an 8-bit step"
                );
                min = min.min(o);
                max = max.max(o);
            }
        }
        // The matrix's extreme ranks (0 and 63) must appear within one 8x8
        // tile, so the offset genuinely reaches close to both ends of the band.
        assert!(min < -half_lsb * 0.9, "min offset {min} does not reach the low end");
        assert!(max > half_lsb * 0.9, "max offset {max} does not reach the high end");
    }

    /// `highlight_dither_on` at density 0 never fires, at density 1 always
    /// fires — the two degenerate sanity bounds around the real ~25% density.
    #[test]
    fn highlight_dither_on_respects_degenerate_densities() {
        for y in 0..8u32 {
            for x in 0..8u32 {
                assert!(!highlight_dither_on(x, y, 0.0), "density 0 must never fire");
                assert!(highlight_dither_on(x, y, 1.0), "density 1 must always fire");
            }
        }
    }

    /// At the round's chosen density (~25%), exactly a proportional COUNT of
    /// the 64 cells in one full tile fire — an ordered dither's whole point is
    /// an EXACT, not merely statistical, coverage fraction.
    #[test]
    fn highlight_dither_density_hits_the_expected_exact_cell_count() {
        let on = (0..8u32)
            .flat_map(|y| (0..8u32).map(move |x| (x, y)))
            .filter(|&(x, y)| highlight_dither_on(x, y, WAGTAIL_HIGHLIGHT_DITHER_DENSITY))
            .count();
        let expected = (WAGTAIL_HIGHLIGHT_DITHER_DENSITY * 64.0).round() as usize;
        assert_eq!(
            on, expected,
            "density {WAGTAIL_HIGHLIGHT_DITHER_DENSITY} over one 8x8 tile should light exactly \
             {expected} of 64 cells, got {on}"
        );
    }
}
