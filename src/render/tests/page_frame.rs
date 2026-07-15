//! THE PAGE-FRAME PIXEL LAW (`theme::PageFrame`, the personality-assignment
//! round's graduated capability) — the render-side half of the theme-side
//! `theme::tests::page_frame_ink_is_the_ladder_and_assigned_weights_are_real`:
//! the ASSIGNED state must be PIXEL-PROVABLE (frame pixels genuinely drawn,
//! in-bounds, in the world's own ladder ink — never inferred from an
//! instance count, the Wagtail-invisible-row lesson), and the None state
//! must genuinely upload NOTHING (zero instances IS the outcome there: an
//! empty instance buffer cannot draw). Wagtail — the first and only
//! assignment (2px, its ladder white) — is the fixture; a default-caps
//! world is the byte-identity control.

use super::super::*;
use super::dither::{offscreen, read_pixels, FMT};
use super::view;

// --- the AWL_PAGE_FRAME_FORCE grammar (pure) — the probe that survives the
// --- AWL_PAGE_BORDER graduation, reshaped to force the CAPABILITY only.

#[test]
fn parse_page_frame_force_accepts_none_and_positive_weights() {
    assert_eq!(parse_page_frame_force("none"), Some(theme::PageFrame::None));
    assert_eq!(parse_page_frame_force("None"), Some(theme::PageFrame::None), "case-insensitive");
    assert_eq!(
        parse_page_frame_force("2"),
        Some(theme::PageFrame::Line { weight_px: 2.0 })
    );
    assert_eq!(
        parse_page_frame_force(" 1.5 "),
        Some(theme::PageFrame::Line { weight_px: 1.5 }),
        "whitespace-tolerant"
    );
}

#[test]
fn parse_page_frame_force_rejects_garbage() {
    for bad in ["", "wat", "0", "-2", "inf", "NaN", "2px"] {
        assert_eq!(parse_page_frame_force(bad), None, "expected None for {bad:?}");
    }
}

/// A `(Device, Queue, TextPipeline)` triple, or `None` on a GPU-less machine
/// — the same accepted per-file duplication every real-pixel test module
/// carries (see `distinguishability.rs`'s own doc note).
fn headless_dqp(w: f32, h: f32) -> Option<(wgpu::Device, wgpu::Queue, TextPipeline)> {
    pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl page-frame-test device"),
                ..Default::default()
            })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let mut p = TextPipeline::new(&device, &queue, &cache, FMT);
        p.set_size(w, h);
        Some((device, queue, p))
    })
}

/// THE ASSIGNED HALF, over real GPU output: Wagtail's 2px frame draws PURE
/// LADDER WHITE (`page_frame_ink` = `base_content` = `#FFFFFF`, exactly —
/// the hard-edged dither-1.0 fill has no antialiased fringe, so the one-bit
/// law needs no tolerance here) at the expected coordinates: straddling the
/// writing column's left and right edges and its top edge, strictly INSIDE
/// the canvas, with flat pure-black ground further out in the margin and on
/// the page itself. Then THE ABSENT HALF: a default-caps world prepared
/// through the same path uploads ZERO frame rects (structurally nothing to
/// draw — the byte-identity guarantee for the fifteen None worlds).
#[test]
fn wagtail_page_frame_draws_pure_ladder_white_in_bounds_and_none_worlds_draw_none() {
    let Some((device, queue, mut p)) = headless_dqp(500.0, 360.0) else {
        eprintln!(
            "skipping wagtail_page_frame_draws_pure_ladder_white_in_bounds_and_none_worlds_draw_none: no wgpu adapter"
        );
        return;
    };
    let _g = crate::testlock::serial();
    let was_page_on = crate::page::page_on();
    let was_measure = crate::page::measure();
    crate::page::set_measure(24);
    crate::page::set_page_on(true);

    theme::set_active_by_name("Wagtail").unwrap();
    p.sync_theme();
    let v = view("hi\nthere\n", 0, 0);
    p.set_view(&v);
    p.prepare(&device, &queue, 500, 360).unwrap();

    // The frame reads the SAME geometry owners the renderer does.
    let left = p.column_left();
    let colw = p.column_width();
    let right = left + colw;
    let top = p.doc_top().max(0.0);
    let weight = 2.0f32;

    let (texture, tview) = offscreen(&device, 500, 360);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("awl page-frame encoder"),
    });
    p.render(&mut encoder, &tview).unwrap();
    queue.submit(Some(encoder.finish()));
    let pixels = read_pixels(&device, &queue, &texture, 500, 360);
    let at = |x: i64, y: i64| -> [u8; 4] { pixels[(y * 500 + x) as usize] };
    let white = [255u8, 255, 255, 255];
    let black = [0u8, 0, 0, 255];

    // Sample the CENTER of each 2px edge band, on a row safely inside the
    // document's vertical extent (mid first line — the frame spans the doc).
    let mid_y = (top + LINE_HEIGHT * 0.5) as i64;
    let left_band_x = (left - weight * 0.5).floor() as i64;
    let right_band_x = (right + weight * 0.5).floor() as i64;
    assert_eq!(
        at(left_band_x, mid_y),
        white,
        "the frame's LEFT edge band must be the pure ladder white at ({left_band_x}, {mid_y})"
    );
    assert_eq!(
        at(right_band_x, mid_y),
        white,
        "the frame's RIGHT edge band must be the pure ladder white at ({right_band_x}, {mid_y})"
    );
    // The TOP edge band, sampled mid-column (no glyph sits above the doc top).
    let mid_x = (left + colw * 0.5) as i64;
    let top_band_y = (top - weight * 0.5).floor() as i64;
    assert_eq!(
        at(mid_x, top_band_y),
        white,
        "the frame's TOP edge band must be the pure ladder white at ({mid_x}, {top_band_y})"
    );
    // IN-BOUNDS: every sampled band coordinate is strictly on-canvas (the
    // samples above would have panicked on an out-of-range index otherwise —
    // assert it explicitly so the law reads).
    for (x, y) in [(left_band_x, mid_y), (right_band_x, mid_y), (mid_x, top_band_y)] {
        assert!(
            (0..500).contains(&x) && (0..360).contains(&y),
            "frame sample ({x}, {y}) fell off the canvas — the frame must draw in-bounds"
        );
    }
    // FIGURE/GROUND stays flat around the frame: pure black just outside in
    // the margin, and pure black just inside on the page (below the text
    // lines — line 2 is empty, so no glyph interferes).
    let margin_x = (left - weight - 4.0) as i64;
    let inside_x = (left + weight + 4.0) as i64;
    let empty_row_y = (top + LINE_HEIGHT * 2.5) as i64;
    assert_eq!(
        at(margin_x.max(0), mid_y),
        black,
        "the margin just OUTSIDE the frame stays the flat pure-black ground"
    );
    assert_eq!(
        at(inside_x, empty_row_y),
        black,
        "the page just INSIDE the frame stays the flat pure-black ground"
    );

    // THE ABSENT HALF: a default-caps world (PageFrame::None) uploads zero
    // frame rects through the very same prepare path.
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
    p.set_view(&v);
    p.prepare(&device, &queue, 500, 360).unwrap();
    assert_eq!(
        p.page_frame_pipeline.instance_count(),
        0,
        "a PageFrame::None world must upload ZERO frame rects (byte-identity for the \
         unassigned roster)"
    );

    crate::page::set_page_on(was_page_on);
    crate::page::set_measure(was_measure);
}
