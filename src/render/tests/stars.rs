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

    // THE RELAXED (LIFECYCLE-round) CEILING: a star pixel may now glint ABOVE
    // the muted whisper cap (`muted_y`), but stays STRICTLY UNDER the text ink
    // (`content_y`) — the figure stays the prose's. Both rungs read at real
    // pixels; the muted one is kept only to prove the relaxation is genuinely
    // WIRED (a real glint clears it), not to cap.
    let muted_y = rel_lum({
        let m = theme::muted();
        [m.r, m.g, m.b, 0xFF]
    });
    let content_y = rel_lum({
        let c = theme::base_content();
        [c.r, c.g, c.b, 0xFF]
    });

    let mut changed = 0usize;
    let mut changed_left = 0usize;
    let mut changed_right = 0usize;
    let mut above_whisper = 0usize;
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
            // Calm ceiling at real pixels: no star pixel (either phase)
            // outshines the text ink. Small tolerance for the rounded-quad AA
            // edge compositing.
            for (label, px) in [("A", a), ("B", b)] {
                let l = rel_lum(px);
                assert!(
                    l <= content_y + 0.02,
                    "star pixel at ({x}, {y}) phase {label} has luminance {l:.3} — \
                     past the text ink's {content_y:.3} ceiling (a glint must never outshine prose)"
                );
                if l > muted_y + 0.02 {
                    above_whisper += 1;
                }
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
    // THE RELAXATION IS WIRED at real pixels: at least one rendered star glint
    // clears the OLD muted whisper cap — the LIFECYCLE round's blessed loosening
    // actually paints brighter stars, not merely a relaxed authored bound. This
    // fails on the pre-lifecycle render (every star capped at/under muted).
    assert!(
        above_whisper > 0,
        "no rendered star glint rose above the muted whisper cap ({muted_y:.3}) — \
         the brighter-shine relaxation is not actually wired at pixels"
    );

    crate::page::set_page_on(was_page_on);
    crate::page::set_measure(was_measure);
}

/// THE DPI-INVARIANCE LAW (the twinkling-stars density/size bug, 2026-07-18: on a
/// 2x-retina display Currawong's sky rendered ~5.6x too DENSE and every dot at HALF
/// its intended logical size — 8 stars at 1x vs 45 at 2x for the same logical
/// window). The authored `cell_px`/`size_px` (`theme/worlds.rs`) are PHYSICAL px at
/// scale 1.0; `prepare_stars_layer` scales BOTH by the total logical->physical
/// factor (user-zoom × device-DPI) before the grid scatter, so the SAME logical
/// window keeps a constant LOGICAL star DENSITY and a constant LOGICAL dot SIZE at
/// any DPI. Proven at REAL pixels over the ACTUAL render path — the pure
/// `crate::stars::layout` is untouched by the fix, so this must exercise the
/// renderer, not the layout. Render the identical logical Currawong page at 1x and
/// at 2x physical, then assert two arms, EACH of which the reverted (unscaled) fix
/// fails:
///   * DENSITY — the DRAWN instance count is ~equal (the reverted signature is
///     ~4-5x more instances at 2x, rejected by the band here), and
///   * DOT SIZE — the star-tinted pixel AREA PER instance grows ~4x at 2x (each dot
///     2x wider AND 2x taller → constant logical size); a reverted fix leaves the
///     dots the same PHYSICAL size (~1x area/instance) and fails this arm.
#[test]
fn currawong_star_field_is_dpi_invariant_in_logical_space() {
    // The SAME LOGICAL window at two device scales: 1x is WxH physical, 2x is 2Wx2H.
    const W: u32 = 900;
    const H: u32 = 600;

    let _g = crate::testlock::serial();
    let was_page_on = crate::page::page_on();
    let was_measure = crate::page::measure();
    crate::page::set_page_on(true);
    crate::page::set_measure(24); // narrow column -> wide, star-bearing margins

    // Render the identical logical Currawong page at a given device scale; return
    // (drawn instance count, star-tinted pixel area in the margins), or None on a
    // GPU-less machine.
    let render_at = |dpi: f32| -> Option<(u32, u64)> {
        let pw = (W as f32 * dpi) as u32;
        let ph = (H as f32 * dpi) as u32;
        let (device, queue, mut p) = headless_dqp(pw as f32, ph as f32)?;
        // DPI after set_size (headless_dqp already sized), mirroring the capture
        // path — set_dpi rebuilds the metrics + re-shapes at the rescaled column.
        p.set_dpi(dpi);
        theme::set_active_by_name("Currawong").unwrap();
        p.sync_theme();
        let v = view("hi\nthere\n", 0, 0);
        p.set_view(&v);
        let frame = render_frame(&device, &queue, &mut p, pw, ph);
        let count = p.stars_pipeline.instance_count();
        // The writing-column band this frame (the geometry owner the cull read) —
        // star pixels live strictly OUTSIDE it, so measure only the margins.
        let col_left = p.column_left();
        let col_right = col_left + p.column_width();
        // Currawong's star tint (#9DB0CF) reads BLUE-forward; the world's muted/faint
        // margin ink is neutral (r≈g≈b), so `b > r` isolates star pixels. (Any neutral
        // contamination would scale ~4x with the surface at BOTH DPIs and cancels in
        // the per-instance RATIO below — it can never turn a reverted fix into a pass.)
        let mut area = 0u64;
        for y in 0..ph as usize {
            for x in 0..pw as usize {
                let xf = x as f32;
                if xf >= col_left && xf < col_right {
                    continue;
                }
                let px = frame[y * pw as usize + x];
                if px[2] as i32 > px[0] as i32 + 8 && px[2] > 24 {
                    area += 1;
                }
            }
        }
        Some((count, area))
    };

    let (one, two) = (render_at(1.0), render_at(2.0));

    // Restore globals BEFORE asserting so a failing arm can't leak page/theme state.
    theme::set_active(theme::DEFAULT_THEME);
    crate::page::set_page_on(was_page_on);
    crate::page::set_measure(was_measure);

    let (Some((count_1, area_1)), Some((count_2, area_2))) = (one, two) else {
        eprintln!(
            "skipping currawong_star_field_is_dpi_invariant_in_logical_space: no wgpu adapter"
        );
        return;
    };

    assert!(
        count_1 > 5 && count_2 > 5,
        "both scales must scatter a real population (1x {count_1}, 2x {count_2})"
    );
    // DENSITY invariance: ~equal counts. The reverted-fix signature is count_2 ≈
    // 4-5.6x count_1; this band rejects it while admitting the small residual from
    // the physical-px margin gap + AA fringe (a few extra stars near the boundary).
    assert!(
        (count_2 as f32) <= 2.0 * count_1 as f32 && (count_1 as f32) <= 2.0 * count_2 as f32,
        "DPI-invariant DENSITY: the 2x sky must hold ~as many stars as 1x, not ~4x \
         (1x {count_1}, 2x {count_2}) — the density half of the twinkling-stars bug"
    );
    // DOT-SIZE invariance: area PER instance ~4x at 2x (2x wider × 2x taller). A
    // reverted fix keeps dots the same PHYSICAL size -> ~1x area/instance -> fails.
    let per_1 = area_1 as f32 / count_1 as f32;
    let per_2 = area_2 as f32 / count_2 as f32;
    assert!(
        per_2 >= 2.5 * per_1 && per_2 <= 6.0 * per_1,
        "DPI-invariant DOT SIZE: each dot's pixel area must grow ~4x at 2x so its \
         LOGICAL size is constant (per-instance area 1x {per_1:.1}, 2x {per_2:.1}) — \
         the half-size-dots half of the bug"
    );
}

/// THE DETERMINISTIC-IDENTITY LAW (item 62, 2026-07-24): the SAME star, at the
/// SAME fixed phase, renders BYTE-IDENTICAL pixels across two independent
/// captures — position, size, tint, AND phase (brightness) are all pure
/// functions of (seed, phase), never a clock or entropy (`crate::stars`'s own
/// doc). Two full end-to-end captures of the identical Currawong scene, at the
/// frozen headless phase (0.0, never advanced), must produce the identical
/// frame buffer down to the byte — the pixel-level proof that the item 62 size
/// spread stayed deterministic (a live-random size would desync the two
/// frames the instant it landed on a differently-sized star).
#[test]
fn currawong_stars_are_pixel_identical_across_two_captures_of_the_same_phase() {
    const W: u32 = 900;
    const H: u32 = 600;
    let Some((device, queue, mut p)) = headless_dqp(W as f32, H as f32) else {
        eprintln!(
            "skipping currawong_stars_are_pixel_identical_across_two_captures_of_the_same_phase: \
             no wgpu adapter"
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

    // Two independent end-to-end captures at the SAME (frozen) phase — never
    // advanced between them.
    let frame_a = render_frame(&device, &queue, &mut p, W, H);
    let count_a = p.stars_pipeline.instance_count();
    let frame_b = render_frame(&device, &queue, &mut p, W, H);
    let count_b = p.stars_pipeline.instance_count();

    crate::page::set_page_on(was_page_on);
    crate::page::set_measure(was_measure);

    assert!(count_a > 10, "a real star population must be present ({count_a} instances)");
    assert_eq!(count_a, count_b, "the drawn star count must not drift across captures");
    assert_eq!(
        frame_a, frame_b,
        "two captures of the SAME Currawong scene at the SAME fixed phase must be \
         byte-identical — every star's position, size, tint, and phase is a pure \
         function of (seed, phase), never a clock or randomness"
    );
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
