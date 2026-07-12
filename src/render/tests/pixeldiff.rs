//! THE PIXEL-DIFF HELPER — the LAW ROUND's structural answer to the Wagtail
//! invisible-picker-row bug (six render surfaces shipped invisible across
//! three rounds while every MECHANISM-shaped test — `instance_count == 1`,
//! `dither() > 0.0`, … — passed green; a fully-transparent quad satisfies
//! every one of those assertions). CLAUDE.md's harness section names the
//! rule this file exists to make cheap to follow: **"the sidecar is a STATE
//! oracle, not an APPEARANCE oracle" — appearance-class properties
//! ("visible", "distinct", "legible") must be asserted over the PNG's
//! pixels, never inferred from state.** Before this file, doing that meant
//! hand-rolling a readback + a bespoke pixel loop per test (see `dither.rs`'s
//! own `offscreen`/`read_pixels`, and `one_bit.rs`'s several hand-inlined
//! sampling loops) — this module makes the OUTCOME assertion itself one line:
//! `assert_perceptibly_different(..)` / `assert_identical(..)`.
//!
//! Deterministic, no clock, no filesystem — pure arithmetic over two
//! already-rendered `Vec<[u8;4]>` pixel buffers (the same row-major shape
//! `dither::read_pixels` returns). Doesn't render anything itself; callers
//! still drive `TextPipeline::prepare`/`render` + `dither::{offscreen,
//! read_pixels}` (or the `render_region` convenience wrapper below) exactly
//! as `one_bit.rs`/`dither.rs` already do — this module is the assertion
//! layer on top, not a new rendering path.

use super::super::*;
use super::dither;

/// A rectangular pixel region in canvas (device-pixel) coordinates. `x`/`y`
/// are the top-left corner; `w`/`h` extend right/down. Coordinates are
/// clamped to the buffer's own bounds by `diff_region`/`sample_region`, so a
/// region that runs slightly past a computed edge (rounding, a `-1`/`+1`
/// overhang like the border-ring tests already use) never panics.
#[derive(Clone, Copy, Debug)]
pub(super) struct Region {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

impl Region {
    pub(super) fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Region { x: x as i64, y: y as i64, w: w as i64, h: h as i64 }
    }
    /// The whole canvas.
    pub(super) fn canvas(width: i64, height: i64) -> Self {
        Region { x: 0, y: 0, w: width, h: height }
    }
}

/// Measured difference between two same-sized pixel buffers over `region`:
/// how many of the region's pixels differ at all, the region's total pixel
/// count, and the largest single-channel delta observed anywhere in it
/// (0 if the region is byte-identical).
#[derive(Clone, Copy, Debug)]
pub(super) struct DiffReport {
    pub differing: usize,
    pub total: usize,
    pub max_channel_delta: u8,
}

impl DiffReport {
    pub(super) fn differing_fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.differing as f32 / self.total as f32
        }
    }
}

/// Walk `region` (clamped to `[0,width) x [0,height)`) over two row-major
/// `width`x`height` pixel buffers and measure how much they differ. A pixel
/// counts as "differing" if ANY of its four channels differ at all; the
/// report's `max_channel_delta` is the single largest per-channel |a-b| seen
/// anywhere in the region, over any channel of any pixel.
pub(super) fn diff_region(
    a: &[[u8; 4]],
    b: &[[u8; 4]],
    width: i64,
    height: i64,
    region: Region,
) -> DiffReport {
    assert_eq!(a.len(), b.len(), "diff_region: buffers must be the same size");
    let x0 = region.x.max(0);
    let y0 = region.y.max(0);
    let x1 = (region.x + region.w).min(width);
    let y1 = (region.y + region.h).min(height);
    let mut differing = 0usize;
    let mut total = 0usize;
    let mut max_delta: u8 = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * width + x) as usize;
            total += 1;
            let pa = a[i];
            let pb = b[i];
            let mut this_max = 0u8;
            let mut differs = false;
            for c in 0..4 {
                let d = pa[c].abs_diff(pb[c]);
                this_max = this_max.max(d);
                if d != 0 {
                    differs = true;
                }
            }
            if differs {
                differing += 1;
            }
            max_delta = max_delta.max(this_max);
        }
    }
    DiffReport { differing, total, max_channel_delta: max_delta }
}

/// The floor a `DiffReport` must clear to count as "perceptibly different" —
/// BOTH a minimum FRACTION of the region's pixels must differ at all (guards
/// against a single stray anti-aliased pixel counting as "different") AND
/// the largest single-channel delta anywhere in the region must clear a
/// minimum magnitude (guards against a fraction of barely-different pixels —
/// e.g. sub-pixel rounding noise — counting as a real visual change).
/// `DEFAULT` is deliberately conservative: real UI state changes (a fill
/// band, an inverted row, a moved highlight) clear it by a wide margin;
/// genuine anti-aliasing noise between two otherwise-identical renders does
/// not.
#[derive(Clone, Copy, Debug)]
pub(super) struct DistinguishFloor {
    pub min_fraction: f32,
    pub min_max_delta: u8,
}

impl DistinguishFloor {
    pub(super) const DEFAULT: DistinguishFloor =
        DistinguishFloor { min_fraction: 0.01, min_max_delta: 12 };
}

/// Assert that `region` (same coordinates in both buffers, both sized
/// `width`x`height`) is PERCEPTIBLY DIFFERENT between renders `a` and `b` —
/// the one-line replacement for "does state-on actually look different from
/// state-off". Fails loud with the measured numbers on a miss, so a
/// regression reads as "the highlight band stopped painting" rather than a
/// bare `assert!` false.
pub(super) fn assert_perceptibly_different(
    a: &[[u8; 4]],
    b: &[[u8; 4]],
    width: i64,
    height: i64,
    region: Region,
    floor: DistinguishFloor,
    label: &str,
) {
    let report = diff_region(a, b, width, height, region);
    assert!(report.total > 0, "{label}: region is empty ({region:?}) — nothing to compare");
    let frac = report.differing_fraction();
    assert!(
        frac >= floor.min_fraction && report.max_channel_delta >= floor.min_max_delta,
        "{label}: expected a PERCEPTIBLE difference in {region:?} but got \
         differing_fraction={frac:.4} (floor {:.4}), max_channel_delta={} (floor {}) \
         over {} pixels — the two states render the SAME here, exactly the shape of \
         the Wagtail invisible-picker-row bug (a mechanism fired, the pixels didn't move)",
        floor.min_fraction,
        report.max_channel_delta,
        floor.min_max_delta,
        report.total,
    );
}

/// The inverse assertion: `region` must be BYTE-IDENTICAL between `a` and
/// `b` — used to prove a refactor changed nothing observable (the enum-shape
/// refactor in this round proves Wagtail + a control world render pixel-for-
/// pixel identical before/after).
pub(super) fn assert_identical(
    a: &[[u8; 4]],
    b: &[[u8; 4]],
    width: i64,
    height: i64,
    region: Region,
    label: &str,
) {
    let report = diff_region(a, b, width, height, region);
    assert!(report.total > 0, "{label}: region is empty ({region:?}) — nothing to compare");
    assert_eq!(
        report.differing, 0,
        "{label}: expected byte-identical pixels in {region:?}, but {} of {} pixels differ \
         (max_channel_delta={})",
        report.differing, report.total, report.max_channel_delta,
    );
}

/// Render the pipeline's CURRENT prepared state (whatever the caller already
/// set via `set_view`/`prepare`) to an offscreen `width`x`height` texture and
/// read it back as a flat row-major `Vec<[u8;4]>` — the exact `dither`-module
/// readback dance every real-pixel test in this tree already hand-rolls,
/// pulled out to ONE call so a NEW real-pixel test doesn't have to re-inline
/// it a third/fourth time (mirrors `dither.rs`'s own doc note on why the
/// FIRST such duplication, versus `capture/gpu.rs`, is itself accepted).
pub(super) fn render_frame(
    p: &mut TextPipeline,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
) -> Vec<[u8; 4]> {
    let (texture, tview) = dither::offscreen(device, width, height);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("awl pixeldiff-test encoder"),
    });
    p.render(&mut encoder, &tview).unwrap();
    queue.submit(Some(encoder.finish()));
    dither::read_pixels(device, queue, &texture, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_region_counts_differing_pixels_and_max_delta() {
        let w = 4i64;
        let h = 2i64;
        let a = vec![[0u8, 0, 0, 255]; (w * h) as usize];
        let mut b = a.clone();
        b[0] = [10, 0, 0, 255]; // one pixel differs by 10
        b[5] = [0, 0, 0, 255]; // identical
        let report = diff_region(&a, &b, w, h, Region::canvas(w, h));
        assert_eq!(report.total, 8);
        assert_eq!(report.differing, 1);
        assert_eq!(report.max_channel_delta, 10);
    }

    #[test]
    fn region_clamps_to_buffer_bounds_never_panics() {
        let w = 4i64;
        let h = 4i64;
        let a = vec![[0u8, 0, 0, 255]; (w * h) as usize];
        let b = a.clone();
        // A region hanging off every edge — must clamp, not panic or underflow.
        let report = diff_region(&a, &b, w, h, Region { x: -2, y: -2, w: 100, h: 100 });
        assert_eq!(report.total, 16);
        assert_eq!(report.differing, 0);
    }

    #[test]
    fn assert_perceptibly_different_passes_on_a_real_change() {
        let w = 4i64;
        let h = 4i64;
        let a = vec![[0u8, 0, 0, 255]; (w * h) as usize];
        let mut b = a.clone();
        for p in b.iter_mut() {
            *p = [255, 255, 255, 255];
        }
        assert_perceptibly_different(
            &a,
            &b,
            w,
            h,
            Region::canvas(w, h),
            DistinguishFloor::DEFAULT,
            "test fixture",
        );
    }

    #[test]
    #[should_panic(expected = "expected a PERCEPTIBLE difference")]
    fn assert_perceptibly_different_fails_when_nothing_moved() {
        let w = 4i64;
        let h = 4i64;
        let a = vec![[0u8, 0, 0, 255]; (w * h) as usize];
        let b = a.clone();
        assert_perceptibly_different(
            &a,
            &b,
            w,
            h,
            Region::canvas(w, h),
            DistinguishFloor::DEFAULT,
            "test fixture",
        );
    }

    #[test]
    fn assert_identical_passes_on_byte_identical_buffers() {
        let w = 4i64;
        let h = 4i64;
        let a = vec![[12u8, 34, 56, 255]; (w * h) as usize];
        let b = a.clone();
        assert_identical(&a, &b, w, h, Region::canvas(w, h), "test fixture");
    }

    #[test]
    #[should_panic(expected = "expected byte-identical pixels")]
    fn assert_identical_fails_on_any_difference() {
        let w = 4i64;
        let h = 4i64;
        let a = vec![[0u8, 0, 0, 255]; (w * h) as usize];
        let mut b = a.clone();
        b[3] = [1, 0, 0, 255];
        assert_identical(&a, &b, w, h, Region::canvas(w, h), "test fixture");
    }
}
