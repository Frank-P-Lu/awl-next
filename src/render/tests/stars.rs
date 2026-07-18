//! THE TWINKLING-STARS PIXEL LAWS (`theme::AmbientStyle::Stars`, the
//! TWINKLING-STARS round) — the render-side half of the theme-side
//! `theme::tests::ambient_stars_laws_hold_for_every_world`: star PLACEMENT and
//! the twinkle itself must be PIXEL-PROVABLE over real GPU output (never
//! inferred from an instance count — the Wagtail-invisible-row lesson), and
//! the starless roster must genuinely upload NOTHING (zero instances IS the
//! byte-identity guarantee for the fifteen `AmbientStyle::None` worlds).
//!
//! The twinkle-diff idiom: render the SAME Currawong scene at two ambient
//! phases and diff the frames — every changed pixel IS a star pixel (nothing
//! else in the frame reads the ambient clock), so "the changed set is
//! non-empty, lives strictly in the margins, and stays under the ladder's
//! quiet-band luminance ceiling" proves presence + placement + brightness in
//! one sweep, against the LIVE column geometry the renderer itself used.

use super::super::*;
use super::dither::{offscreen, read_pixels, FMT};
use super::view;

/// A `(Device, Queue, TextPipeline)` triple, or `None` on a GPU-less machine —
/// the same accepted per-file duplication every real-pixel test module carries
/// (see `distinguishability.rs`'s own doc note).
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
                label: Some("awl stars-test device"),
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

fn render_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    p: &mut TextPipeline,
    w: u32,
    h: u32,
) -> Vec<[u8; 4]> {
    p.prepare(device, queue, w, h).unwrap();
    let (texture, tview) = offscreen(device, w, h);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("awl stars encoder"),
    });
    p.render(&mut encoder, &tview).unwrap();
    queue.submit(Some(encoder.finish()));
    read_pixels(device, queue, &texture, w, h)
}

/// WCAG relative luminance of an sRGB byte triple.
fn rel_lum(px: [u8; 4]) -> f32 {
    fn lin(u: u8) -> f32 {
        let s = u as f32 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    }
    0.2126 * lin(px[0]) + 0.7152 * lin(px[1]) + 0.0722 * lin(px[2])
}

/// THE TWINKLE-DIFF LAW: two phases of the same Currawong page differ ONLY in
/// the margins (placement — no star pixel under the writing column band, ever),
/// differ SOMEWHERE (presence — the sky genuinely twinkles between phases, in
/// BOTH margins), and every star pixel at either phase stays at or under the
/// world's own `muted`-rung luminance (the quiet-band ceiling, at real pixels).
#[test]
fn currawong_stars_twinkle_in_the_margins_only_at_real_pixels() {
    const W: u32 = 900;
    const H: u32 = 600;
    let Some((device, queue, mut p)) = headless_dqp(W as f32, H as f32) else {
        eprintln!(
            "skipping currawong_stars_twinkle_in_the_margins_only_at_real_pixels: no wgpu adapter"
        );
        return;
    };
    let _g = crate::testlock::serial();
    let was_page_on = crate::page::page_on();
    let was_measure = crate::page::measure();
    crate::page::set_page_on(true);
    crate::page::set_measure(24); // narrow column -> wide, star-bearing margins

    theme::set_active_by_name("Currawong").unwrap();
    p.sync_theme();
    let v = view("hi\nthere\n", 0, 0);
    p.set_view(&v);

    // Phase A: the frozen capture phase (0.0 — the pipeline's construction
    // default; a headless capture never ticks).
    let frame_a = render_frame(&device, &queue, &mut p, W, H);
    let count_a = p.stars_pipeline.instance_count();
    assert!(
        count_a > 10,
        "a 900x600 Currawong page at measure 24 must scatter a real star population \
         (drew {count_a} instances)"
    );

    // Phase B: advance the shared ambient clock through the App's own bounded
    // tick step (each call clamps to one 100 ms step) to a genuinely different
    // mid-breath composition.
    for _ in 0..200 {
        p.advance_lava(crate::lava::LAVA_TICK_SECONDS);
    }
    let frame_b = render_frame(&device, &queue, &mut p, W, H);

    // The renderer's OWN column band this frame (the same geometry owner the
    // cull read) — the placement law is asserted against the live values.
    let col_left = p.column_left();
    let col_right = col_left + p.column_width();

    let muted_y = rel_lum({
        let m = theme::muted();
        [m.r, m.g, m.b, 0xFF]
    });

    let mut changed = 0usize;
    let mut changed_left = 0usize;
    let mut changed_right = 0usize;
    for y in 0..H as usize {
        for x in 0..W as usize {
            let a = frame_a[y * W as usize + x];
            let b = frame_b[y * W as usize + x];
            if a == b {
                continue;
            }
            changed += 1;
            let xf = x as f32;
            assert!(
                !(xf >= col_left && xf < col_right),
                "a twinkle-diff pixel at ({x}, {y}) sits INSIDE the writing column \
                 [{col_left}, {col_right}) — stars must never render under the text"
            );
            if xf < col_left {
                changed_left += 1;
            } else {
                changed_right += 1;
            }
            // The quiet-band ceiling at real pixels: no star pixel (either
            // phase) outshines the world's own muted rung. Small tolerance for
            // the rounded-quad AA edge compositing.
            for (label, px) in [("A", a), ("B", b)] {
                let l = rel_lum(px);
                assert!(
                    l <= muted_y + 0.02,
                    "star pixel at ({x}, {y}) phase {label} has luminance {l:.3} — \
                     past the muted rung's {muted_y:.3} quiet-band ceiling"
                );
            }
        }
    }
    assert!(
        changed > 50,
        "the sky must genuinely TWINKLE between two well-separated phases \
         (only {changed} pixels changed)"
    );
    assert!(
        changed_left > 0 && changed_right > 0,
        "both margins must carry living stars (left {changed_left}, right {changed_right})"
    );

    crate::page::set_page_on(was_page_on);
    crate::page::set_measure(was_measure);
}

/// THE ABSENT HALF: every `AmbientStyle::None` world uploads ZERO star
/// instances through the same prepare path (structurally nothing to draw — the
/// byte-identity guarantee for the starless roster), and even the stars world
/// uploads zero with page mode OFF (no margins → no stars, the background
/// pass's own collapse).
#[test]
fn starless_worlds_and_page_off_upload_zero_star_instances() {
    const W: u32 = 500;
    const H: u32 = 360;
    let Some((device, queue, mut p)) = headless_dqp(W as f32, H as f32) else {
        eprintln!(
            "skipping starless_worlds_and_page_off_upload_zero_star_instances: no wgpu adapter"
        );
        return;
    };
    let _g = crate::testlock::serial();
    let was_page_on = crate::page::page_on();
    let was_measure = crate::page::measure();
    crate::page::set_page_on(true);
    crate::page::set_measure(24);

    let v = view("hi\nthere\n", 0, 0);
    for t in theme::THEMES.iter() {
        if t.render_caps.ambient.is_animated() {
            continue;
        }
        theme::set_active_by_name(t.name).unwrap();
        p.sync_theme();
        p.set_view(&v);
        p.prepare(&device, &queue, W, H).unwrap();
        assert_eq!(
            p.stars_pipeline.instance_count(),
            0,
            "{}: an AmbientStyle::None world must upload ZERO star instances \
             (byte-identity for the starless roster)",
            t.name
        );
    }

    // The stars world with page mode OFF: the column spans the canvas, the
    // margin gate culls everything.
    theme::set_active_by_name("Currawong").unwrap();
    crate::page::set_page_on(false);
    p.sync_theme();
    p.set_view(&v);
    p.prepare(&device, &queue, W, H).unwrap();
    assert_eq!(
        p.stars_pipeline.instance_count(),
        0,
        "page-off must cull every star (no margins, no sky)"
    );

    theme::set_active(theme::DEFAULT_THEME);
    crate::page::set_page_on(was_page_on);
    crate::page::set_measure(was_measure);
}
