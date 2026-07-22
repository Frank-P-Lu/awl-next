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
pub(crate) const WAGTAIL_HIGHLIGHT_DITHER_DENSITY: f32 = 0.25;

/// TASTE TUNABLE (CHUNK round): the edge of ONE Bayer cell, in LOGICAL pixels,
/// for THE ONE WAGTAIL HIGHLIGHT TEXTURE. The shader quantizes its absolute
/// canvas position to blocks this wide (× the display scale — see
/// `render::spans::wagtail_stipple_cell_px`) before the Bayer lookup, so a
/// block of pixels shares one on/off decision and the stipple reads as
/// DELIBERATE dithered pixels rather than fine per-pixel noise. ~2 logical px
/// is the chosen coarseness (candidate screenshots at 1/2/3 logical px were
/// eyeballed; 2 reads as a clean stipple without turning blocky). The density
/// (`WAGTAIL_HIGHLIGHT_DITHER_DENSITY`) is unchanged — an ordered dither's
/// exact coverage fraction is invariant under this block quantization (each
/// cell's rank decision is merely shared by `cell²` pixels).
pub(crate) const WAGTAIL_HIGHLIGHT_STIPPLE_CELL_LOGICAL: f32 = 2.0;

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
///
/// `cell` (CHUNK round) is the Bayer-cell edge in PIXELS: the position is
/// quantized to `floor((x,y) / cell)` before the lookup, so a `cell`x`cell`
/// block shares one rank (one on/off decision) and the stipple coarsens. This
/// is the EXACT mirror of `shaders/selection.wgsl`'s `bayer_threshold01(px,
/// cell)`. `cell = 1.0` is the pre-chunk per-pixel behavior. A `max(cell, 1.0)`
/// guard matches the shader's own divide-by-zero guard.
#[allow(dead_code)]
pub(super) fn highlight_dither_on(x: u32, y: u32, density: f32, cell: f32) -> bool {
    let c = cell.max(1.0);
    let cx = (x as f32 / c).floor() as u32;
    let cy = (y as f32 / c).floor() as u32;
    bayer_threshold01(cx, cy) < density
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
    /// fires — the two degenerate sanity bounds around the real ~25% density —
    /// regardless of the CHUNK cell (a coarser block quantization changes WHICH
    /// pixels share a decision, never the degenerate all-off / all-on answers).
    #[test]
    fn highlight_dither_on_respects_degenerate_densities() {
        for cell in [1.0f32, 2.0, 4.0] {
            for y in 0..8u32 {
                for x in 0..8u32 {
                    assert!(!highlight_dither_on(x, y, 0.0, cell), "density 0 must never fire");
                    assert!(highlight_dither_on(x, y, 1.0, cell), "density 1 must always fire");
                }
            }
        }
    }

    /// At the round's chosen density (~25%), exactly a proportional COUNT of
    /// the pixels in one full tile fire — an ordered dither's whole point is an
    /// EXACT, not merely statistical, coverage fraction — and that fraction is
    /// INVARIANT under the CHUNK cell: over an `8*cell`x`8*cell` region (one
    /// full Bayer period at that block size) exactly `round(density*64) *
    /// cell²` pixels fire, because each of the 64 cell-ranks now governs a
    /// `cell`x`cell` block instead of a single pixel. `cell = 1` recovers the
    /// original 8x8/64-cell law.
    #[test]
    fn highlight_dither_density_hits_the_expected_exact_cell_count() {
        let expected_cells = (WAGTAIL_HIGHLIGHT_DITHER_DENSITY * 64.0).round() as usize;
        for cell in [1u32, 2, 3, 4] {
            let period = 8 * cell;
            let on = (0..period)
                .flat_map(|y| (0..period).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    highlight_dither_on(x, y, WAGTAIL_HIGHLIGHT_DITHER_DENSITY, cell as f32)
                })
                .count();
            let expected = expected_cells * (cell * cell) as usize;
            assert_eq!(
                on, expected,
                "density {WAGTAIL_HIGHLIGHT_DITHER_DENSITY} at cell {cell} over one \
                 {period}x{period} period should light exactly {expected} pixels \
                 ({expected_cells} cells × {cell}² px), got {on}"
            );
        }
    }

    /// THE CHUNK LAW (this round's fix): with `cell = 2` every `2x2` block of
    /// physical pixels shares ONE on/off decision — the stipple coarsens into
    /// deliberate blocks rather than fine per-pixel noise. Asserted directly:
    /// every pixel's decision equals its block's top-left pixel's decision. And
    /// NON-VACUOUS — the chunk genuinely differs from the un-chunked stipple: at
    /// least one pixel flips its on/off answer between `cell = 1` and `cell = 2`
    /// (a no-op chunk that ignored `cell` would fail this, so the test fails
    /// without the quantization).
    #[test]
    fn chunk_cell_coarsens_into_uniform_blocks_and_actually_changes_the_pattern() {
        const CELL: u32 = 2;
        let d = WAGTAIL_HIGHLIGHT_DITHER_DENSITY;
        let mut differs_from_unchunked = false;
        // A couple of full Bayer periods so blocks straddle the 8-cell wrap.
        for y in 0..(8 * CELL * 2) {
            for x in 0..(8 * CELL * 2) {
                let block_x = (x / CELL) * CELL;
                let block_y = (y / CELL) * CELL;
                assert_eq!(
                    highlight_dither_on(x, y, d, CELL as f32),
                    highlight_dither_on(block_x, block_y, d, CELL as f32),
                    "pixel ({x},{y}) must share its {CELL}x{CELL} block's ({block_x},{block_y}) \
                     on/off decision — the chunk is not uniform"
                );
                if highlight_dither_on(x, y, d, CELL as f32) != highlight_dither_on(x, y, d, 1.0) {
                    differs_from_unchunked = true;
                }
            }
        }
        assert!(
            differs_from_unchunked,
            "cell {CELL} produced the IDENTICAL pattern to the un-chunked (cell 1) stipple — \
             the quantization is a silent no-op, not a real chunk"
        );
    }
}
