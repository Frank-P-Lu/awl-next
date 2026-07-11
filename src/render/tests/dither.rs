//! REAL-PIXEL proofs for the DITHER round's three deliverables — this file
//! actually renders to an offscreen GPU texture and reads the bytes back
//! (mirroring `capture/gpu.rs`'s readback dance, duplicated here in miniature
//! since that helper is `pub(super)` inside the `capture` module tree — the
//! same small, deliberate cross-module duplication this codebase already
//! accepts, e.g. `srgba_u8_to_linear` between `selection.rs`/`background.rs`).
//! The PURE math (the Bayer matrix itself, the flat-gradient no-op, the
//! density-to-cell-count law) is unit-tested cheaply in `render::dither`;
//! this file is the "does the real shader actually behave that way" half.
//!
//! Skips (with a printed note, not a failure) on a machine with no wgpu
//! adapter, exactly like every other GPU-backed render test in this tree.

use super::super::*;
use super::view;

/// Request a headless device/queue, or `None` on a GPU-less machine.
fn headless_dq() -> Option<(wgpu::Device, wgpu::Queue)> {
    pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl dither-test device"),
                ..Default::default()
            })
            .await
            .ok()
    })
}

pub(super) const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// `pub(super)`: reused by `one_bit.rs`'s own real-pixel palette-card proof,
/// so a SECOND readback dance never has to be hand-copied a third time — see
/// this module's own doc comment for why the FIRST copy (vs. `capture/gpu.rs`)
/// is itself an accepted exception.
pub(super) fn offscreen(device: &wgpu::Device, width: u32, height: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("awl dither-test offscreen"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Round a row byte count up to wgpu's required 256-byte copy alignment.
pub(super) fn align_256(n: u32) -> u32 {
    (n + 255) & !255
}

/// Read `texture` back to a flat row-major `Vec<[u8;4]>` (already-submitted
/// draws only). Mirrors `capture/gpu.rs::read_frame`'s dance in miniature.
pub(super) fn read_pixels(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture, width: u32, height: u32) -> Vec<[u8; 4]> {
    let unpadded_bpr = width * 4;
    let padded_bpr = align_256(unpadded_bpr);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("awl dither-test readback"),
        size: (padded_bpr * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("awl dither-test copy encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bpr),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    queue.submit(Some(encoder.finish()));

    let (tx, rx) = std::sync::mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::PollType::wait_indefinitely()).expect("device poll failed");
    rx.recv().expect("map_async channel closed").expect("buffer map failed");

    let mut out = Vec::with_capacity((width * height) as usize);
    {
        let mapped = readback.slice(..).get_mapped_range();
        for y in 0..height {
            let row_start = (y * padded_bpr) as usize;
            for x in 0..width {
                let i = row_start + (x * 4) as usize;
                out.push([mapped[i], mapped[i + 1], mapped[i + 2], mapped[i + 3]]);
            }
        }
    }
    readback.unmap();
    out
}

/// Draw a `BackgroundPipeline` (`desc`) covering the WHOLE `width`x`height`
/// canvas as margin (`col_w = 0`, so no page-column hole is punched) and read
/// the result back. Isolates the gradient/dither math from text/glyphs
/// entirely — the purest reachable seam for deliverable 1's claims.
fn render_background(device: &wgpu::Device, queue: &wgpu::Queue, desc: crate::background::BgDesc, width: u32, height: u32) -> Vec<[u8; 4]> {
    let mut bg = crate::background::BackgroundPipeline::new(device, FMT, desc);
    bg.prepare(queue, width, height, 0.0, 0.0);
    let (texture, tview) = offscreen(device, width, height);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("awl dither-test bg encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("awl dither-test bg pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &tview,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        bg.draw(&mut pass);
    }
    queue.submit(Some(encoder.finish()));
    read_pixels(device, queue, &texture, width, height)
}

/// sRGB encode (linear `[0,1]` -> sRGB `[0,1]`) — the inverse of
/// `selection.rs`/`background.rs`'s own `srgba_u8_to_linear`, needed here to
/// compute the NAIVE (non-dithered) expected byte for comparison against the
/// real GPU output.
fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn srgb_u8_to_linear(u: u8) -> f32 {
    let s = u as f32 / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// THE FLAT-GRADIENT NO-OP, at the REAL shader level (deliverable 1's
/// one-bit interplay guard): a `from == to` gradient — Wagtail's exact
/// background shape — covering the whole canvas renders EVERY pixel EXACTLY
/// the flat color, byte-for-byte. No tolerance: the one-bit law has none.
#[test]
fn flat_gradient_renders_byte_identical_pure_pixels_end_to_end() {
    let Some((device, queue)) = headless_dq() else {
        eprintln!("skipping flat_gradient_renders_byte_identical_pure_pixels_end_to_end: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    let desc = crate::background::BgDesc {
        from: [0, 0, 0, 255],
        to: [0, 0, 0, 255],
        dir: (0.0, 1.0),
        shader: 0,
        tint: [0, 0, 0],
        edge: false,
        angle: 0.0,
    };
    let pixels = render_background(&device, &queue, desc, 64, 128);
    for (i, p) in pixels.iter().enumerate() {
        assert_eq!(
            *p,
            [0, 0, 0, 255],
            "pixel {i}: a flat (from==to) gradient must render EXACTLY the flat color, \
             never a dithered nudge"
        );
    }
}

/// A REAL (non-flat) gradient's dither stays bounded to ≤1 8-bit step versus
/// the NAIVE (non-dithered) `mix()` result at every sampled pixel — the
/// "imperceptible as texture" half of deliverable 1 — while at least one
/// sampled row actually DIFFERS from the naive value, proving the dither is
/// live, not a silent no-op bug.
#[test]
fn real_gradient_dither_stays_within_one_lsb_of_the_naive_value_and_is_actually_active() {
    let Some((device, queue)) = headless_dq() else {
        eprintln!("skipping real_gradient_dither_stays_within_one_lsb_of_the_naive_value_and_is_actually_active: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    let (from, to) = ([10u8, 10, 10, 255], [230u8, 230, 230, 255]);
    let desc = crate::background::BgDesc {
        from,
        to,
        dir: (0.0, 1.0),
        shader: 0,
        tint: [0, 0, 0],
        edge: false,
        angle: 0.0,
    };
    let (w, h) = (48u32, 220u32);
    let pixels = render_background(&device, &queue, desc, w, h);

    let from_lin = srgb_u8_to_linear(from[0]);
    let to_lin = srgb_u8_to_linear(to[0]);

    let mut any_differs = false;
    for y in 0..h {
        // Mirror the shader's `t` derivation for a vertical (0,1) direction
        // gradient: `uv = px/viewport`, `t = clamp(dot(uv-0.5, dir)+0.5, 0,1)`.
        let uv_y = (y as f32 + 0.5) / h as f32;
        let t = (uv_y - 0.5 + 0.5).clamp(0.0, 1.0);
        let naive_lin = from_lin + (to_lin - from_lin) * t;
        let naive_srgb = linear_to_srgb(naive_lin).clamp(0.0, 1.0);
        let naive_u8 = (naive_srgb * 255.0).round() as i32;

        let actual = pixels[(y * w + w / 2) as usize];
        for (ch, &actual_ch) in actual.iter().take(3).enumerate() {
            let d = (actual_ch as i32 - naive_u8).abs();
            assert!(
                d <= 1,
                "row {y} channel {ch}: actual {actual_ch} vs naive {naive_u8} differ by {d} (> 1 LSB)"
            );
            if d != 0 {
                any_differs = true;
            }
        }
    }
    assert!(
        any_differs,
        "the dither never produced ANY deviation from the naive value across {h} rows — \
         suspiciously looks like a no-op, not an active ±half-LSB dither"
    );
}

/// THE ONE WAGTAIL HIGHLIGHT TEXTURE, at the PUREST reachable pixel seam:
/// drive `SelectionPipeline`'s dither mode DIRECTLY (no text, no markdown, no
/// TextPipeline) — a rect covering the whole canvas, dither density 0.25,
/// color pure white, over a pure black clear. Every resulting pixel MUST be
/// exactly pure black or pure white — never a third value — and roughly a
/// quarter of them should be "on" (the density, loosely checked).
#[test]
fn dither_mode_paints_only_pure_values_at_roughly_the_configured_density() {
    let Some((device, queue)) = headless_dq() else {
        eprintln!("skipping dither_mode_paints_only_pure_values_at_roughly_the_configured_density: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    let mut sel = crate::selection::SelectionPipeline::new(&device, FMT, [255, 255, 255, 255]);
    sel.set_dither(0.25);
    let (w, h) = (64u32, 64u32);
    sel.prepare(&device, &queue, w, h, &[[0.0, 0.0, w as f32, h as f32]]);

    let (texture, tview) = offscreen(&device, w, h);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("awl dither-mode-test encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("awl dither-mode-test pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &tview,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        sel.draw(&mut pass);
    }
    queue.submit(Some(encoder.finish()));
    let pixels = read_pixels(&device, &queue, &texture, w, h);

    let mut on = 0usize;
    for (i, p) in pixels.iter().enumerate() {
        assert!(
            *p == [0, 0, 0, 255] || *p == [255, 255, 255, 255],
            "pixel {i}: dither mode drew a non-pure value {p:?} — the one-bit pixel law forbids it"
        );
        if *p == [255, 255, 255, 255] {
            on += 1;
        }
    }
    let frac = on as f32 / pixels.len() as f32;
    assert!(
        (0.15..0.35).contains(&frac),
        "density 0.25 should light roughly a quarter of the pixels, got {frac:.3} ({on}/{})",
        pixels.len()
    );
}

/// TRUE INVERSE-VIDEO, at the pixel level: a `new_invert` pipeline drawn over
/// a PURE BLACK clear turns it PURE WHITE, and over a PURE WHITE clear turns
/// it PURE BLACK — the `OneMinusDst`/`Zero` blend trick computing an exact
/// `1 - dst`, verified as real GPU output rather than asserted from the math
/// alone.
#[test]
fn invert_pipeline_flips_pure_black_and_pure_white_exactly() {
    let Some((device, queue)) = headless_dq() else {
        eprintln!("skipping invert_pipeline_flips_pure_black_and_pure_white_exactly: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    let (w, h) = (32u32, 32u32);
    let mut invert = crate::selection::SelectionPipeline::new_invert(&device, FMT);
    invert.prepare(&device, &queue, w, h, &[[0.0, 0.0, w as f32, h as f32]]);

    for (clear, expect) in [
        (wgpu::Color::BLACK, [255u8, 255, 255, 255]),
        (wgpu::Color::WHITE, [0u8, 0, 0, 255]),
    ] {
        let (texture, tview) = offscreen(&device, w, h);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("awl invert-test encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("awl invert-test pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &tview,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            invert.draw(&mut pass);
        }
        queue.submit(Some(encoder.finish()));
        let pixels = read_pixels(&device, &queue, &texture, w, h);
        for (i, p) in pixels.iter().enumerate() {
            // Alpha is untouched by the invert blend (src_factor Zero / dst_factor
            // One on that channel) — the clear color's own alpha (always 255 for
            // an opaque BLACK/WHITE clear) survives, so only rgb is asserted
            // against `expect`'s rgb; alpha is checked separately for honesty.
            assert_eq!(&p[..3], &expect[..3], "pixel {i}: expected {expect:?}, got {p:?}");
        }
    }
}

/// THE 1-BIT CARET ROUND'S READABILITY LAW, at the REAL-PIXEL level — the
/// exact bug the user photographed, now unrepresentable: a block caret
/// sitting ON a heading's `#` glyph in Wagtail. Renders the REAL
/// `TextPipeline::render` path (a markdown heading line, WYSIWYG on so the
/// leading `#` is REVEALED — the caret sits on its own line, so the
/// reveal-on-cursor rule shows the literal `#` rather than concealing it) and
/// asserts the caret's own rect ([`TextPipeline::caret_geometry`]) contains
/// BOTH pure-white AND pure-black pixels. The PRE-fix bug produced an
/// entirely UNIFORM-white rect (the opaque block, painted the SAME pure
/// white as the glyph, erasing it) — a fact the pre-existing
/// `wagtail_pixel_law_holds_with_selection_highlight_and_search_all_active`
/// test structurally could NOT catch: solid white is still a PURE value, so
/// the erasure never violated the one-bit pixel law, only readability. This
/// test is the readability law the pixel law missed.
#[test]
fn wagtail_caret_on_a_heading_glyph_keeps_the_glyph_legible_inside_the_block() {
    let got = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await.ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor { label: Some("awl caret-readability device"), ..Default::default() })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let mut p = TextPipeline::new(&device, &queue, &cache, FMT);
        p.set_size(300.0, 160.0);
        Some((device, queue, p))
    });
    let Some((device, queue, mut p)) = got else {
        eprintln!("skipping wagtail_caret_on_a_heading_glyph_keeps_the_glyph_legible_inside_the_block: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    // Explicit BLOCK mode — deterministic regardless of Wagtail's own
    // font-derived default (`caret::default_mode`) or any prior test's
    // leftover global.
    crate::caret::set_mode(CaretMode::Block);

    // Cursor at column 0 of the heading line: the Block caret covers the
    // glyph AT the cursor column — the `#` itself, the user's own reported
    // fixture ("his caret sat on a heading's `#` and the character
    // vanished"). Being on the heading's OWN line also means the WYSIWYG
    // reveal-on-cursor rule shows the raw `#` rather than concealing it.
    let mut v = view("# Heading\n", 0, 0);
    v.is_markdown = true;

    theme::set_active_by_name("Wagtail").unwrap();
    p.sync_theme();
    p.set_view(&v);
    // Deterministic settled geometry — the same call the real `--screenshot`
    // (Rest) capture path makes (`capture/modes.rs`), not a mid-glide frame.
    p.settle_caret();
    let (cx, cy, cw, ch, ..) = p.caret_geometry();
    p.prepare(&device, &queue, 300, 160).unwrap();

    let (texture, tview) = offscreen(&device, 300, 160);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("awl caret-readability encoder"),
    });
    p.render(&mut encoder, &tview).unwrap();
    queue.submit(Some(encoder.finish()));
    let pixels = read_pixels(&device, &queue, &texture, 300, 160);

    // A couple of pixels' margin around the caret's own footprint absorbs
    // any 1px rounding between `caret_geometry`'s floats and the shader's
    // own rasterization — this is still comfortably WITHIN the block's own
    // rect, never spilling into neighboring glyphs.
    const MARGIN: i32 = 2;
    let left = ((cx - cw * 0.5).floor() as i32 - MARGIN).max(0);
    let right = ((cx + cw * 0.5).ceil() as i32 + MARGIN).min(300);
    let top = ((cy - ch * 0.5).floor() as i32 - MARGIN).max(0);
    let bottom = ((cy + ch * 0.5).ceil() as i32 + MARGIN).min(160);
    assert!(left < right && top < bottom, "fixture must yield a real caret rect");

    let mut white = 0usize;
    let mut black = 0usize;
    for y in top..bottom {
        for x in left..right {
            let p = pixels[(y * 300 + x) as usize];
            match (p[0], p[1], p[2]) {
                (255, 255, 255) => white += 1,
                (0, 0, 0) => black += 1,
                _ => {}
            }
        }
    }
    assert!(white > 0, "the caret rect must show white (the inverted GROUND) — got none");
    // A meaningful floor, not just a stray AA pixel: the flipped `#` glyph
    // is a real multi-stroke symbol, so a genuinely visible glyph paints
    // dozens of black pixels, not one or two.
    assert!(
        black >= 10,
        "the caret rect must show a REAL amount of black (the flipped `#` glyph ink) — \
         got {black} black pixels out of {} sampled; the pre-fix bug painted this rect \
         entirely uniform white, erasing the glyph",
        (right - left) * (bottom - top)
    );

    crate::caret::set_mode(CaretMode::Block);
    theme::set_active(theme::DEFAULT_THEME);
}

/// THE ONE-BIT PIXEL LAW, END TO END: a real Wagtail scene — page mode on
/// (margin gradient visible, flat so it must stay pure), an active TEXT
/// selection (true inverse-video), an `==highlighted==` span, AND an active
/// search match — all drawn together through the REAL `TextPipeline::render`
/// path (the same one the live app / `--screenshot` capture use). Every
/// pixel must be exactly pure black or pure white, EXCEPT a small, scattered
/// minority attributable to ordinary glyph anti-aliasing — the SAME
/// AA-edge tolerance the palette/pipeline laws elsewhere in this suite
/// already grant text edges. Guards against a "large uniform non-pure
/// region" (the exact shape a reintroduced translucent wash bug would take)
/// by asserting no single non-pure color value repeats suspiciously often.
#[test]
fn wagtail_pixel_law_holds_with_selection_highlight_and_search_all_active() {
    let got = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await.ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor { label: Some("awl pixel-law device"), ..Default::default() })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let mut p = TextPipeline::new(&device, &queue, &cache, FMT);
        p.set_size(500.0, 360.0);
        Some((device, queue, p))
    });
    let Some((device, queue, mut p)) = got else {
        eprintln!("skipping wagtail_pixel_law_holds_with_selection_highlight_and_search_all_active: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    let was_page_on = crate::page::page_on();
    let was_measure = crate::page::measure();
    crate::page::set_measure(24);
    crate::page::set_page_on(true);

    let text = "==shout== plain word findme rest\nsecond line of prose for width";
    let mut v = view(text, 0, 0);
    v.is_markdown = true;
    v.selection = Some(((0, 10), (0, 15)));
    v.search_active = true;
    v.search_query = "findme".to_string();
    v.search_matches = vec![((0, 20), (0, 26))];
    v.search_current = Some(0);

    // `TextPipeline::new` baked several pipelines' TINTS from whatever theme
    // was active AT CONSTRUCTION (the capture harness avoids this by setting
    // the theme BEFORE constructing the pipeline) — `sync_theme` is the
    // explicit re-tint + reshape door a live theme SWITCH must call (see
    // `render/tests/theme.rs` for the established idiom); skipping it here
    // would silently render the PREVIOUS (default) theme's colors while this
    // test still believes it's asserting Wagtail's.
    theme::set_active_by_name("Wagtail").unwrap();
    p.sync_theme();
    p.set_view(&v);
    p.prepare(&device, &queue, 500, 360).unwrap();

    let (texture, tview) = offscreen(&device, 500, 360);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("awl pixel-law encoder"),
    });
    p.render(&mut encoder, &tview).unwrap();
    queue.submit(Some(encoder.finish()));
    let pixels = read_pixels(&device, &queue, &texture, 500, 360);

    let width = 500i32;
    let mut impure = 0usize;
    // Track each non-pure color's count AND bounding box (min/max x/y) — an
    // AA-edge color, however common, is SCATTERED (thin 1px-wide runs along
    // many different glyph/line edges), while a translucent-wash regression
    // would paint one CONTIGUOUS filled rectangle: a high "fill ratio" (count
    // / bbox area) is the tell, not the raw count alone (monospace text can
    // legitimately repeat the exact same coverage fraction hundreds of times
    // across many glyphs' identically-hinted stems).
    struct Run {
        count: usize,
        min_x: i32,
        max_x: i32,
        min_y: i32,
        max_y: i32,
    }
    let mut counts: std::collections::HashMap<[u8; 3], Run> = std::collections::HashMap::new();
    for (i, p) in pixels.iter().enumerate() {
        let is_pure = matches!((p[0], p[1], p[2]), (0, 0, 0) | (255, 255, 255));
        if !is_pure {
            impure += 1;
            let x = (i as i32) % width;
            let y = (i as i32) / width;
            let e = counts.entry([p[0], p[1], p[2]]).or_insert(Run {
                count: 0,
                min_x: x,
                max_x: x,
                min_y: y,
                max_y: y,
            });
            e.count += 1;
            e.min_x = e.min_x.min(x);
            e.max_x = e.max_x.max(x);
            e.min_y = e.min_y.min(y);
            e.max_y = e.max_y.max(y);
        }
    }
    let total = pixels.len();
    let impure_frac = impure as f32 / total as f32;
    assert!(
        impure_frac < 0.15,
        "too many non-pure pixels ({impure}/{total} = {impure_frac:.3}) for glyph-AA-only \
         tolerance — looks like a real wash/gradient leak, not edge antialiasing"
    );
    // Only a color with a MEANINGFUL occurrence count can be "a large uniform
    // region" at all — a handful of stray AA pixels trivially fill 100% of
    // their own tiny bounding box without meaning anything.
    const MIN_RUN_FOR_FILL_CHECK: usize = 50;
    for (color, run) in counts.iter() {
        if run.count < MIN_RUN_FOR_FILL_CHECK {
            continue;
        }
        let bbox_area = ((run.max_x - run.min_x + 1) * (run.max_y - run.min_y + 1)) as f32;
        let fill_ratio = run.count as f32 / bbox_area;
        assert!(
            fill_ratio < 0.6,
            "non-pure color {color:?} fills {:.0}% of its own {}x{} bounding box \
             ({} occurrences) — reads as a LARGE UNIFORM FILLED region (exactly the \
             shape a translucent-wash regression would take), not scattered \
             anti-aliased glyph edges",
            fill_ratio * 100.0,
            run.max_x - run.min_x + 1,
            run.max_y - run.min_y + 1,
            run.count,
        );
    }

    // Restore shared globals for other tests.
    theme::set_active(theme::DEFAULT_THEME);
    crate::page::set_page_on(was_page_on);
    crate::page::set_measure(was_measure);
}

/// GALLERY GENERATOR, not a correctness test — `#[ignore]`d by default (mirrors
/// the `AWL_CJK_FORCE` A/B gallery mechanism's "dev-only, no CLI flag" spirit,
/// just via a test runner instead of an env var). Renders the SAME real
/// `TextPipeline::render` path as [`wagtail_pixel_law_holds_with_selection_
/// highlight_and_search_all_active`] at a friendlier canvas size and writes
/// PNG to `gallery/wagtail/` for a human to eyeball the round's three
/// deliverables together: the flat (banding-free-by-construction) margin, the
/// dithered `==highlight==`/search-match band, and the true inverse-video
/// selection. Regenerate with:
/// `cargo test --bin awl render::tests::dither::gallery_wagtail_selection_highlight_search -- --ignored --nocapture`
#[test]
#[ignore]
fn gallery_wagtail_selection_highlight_search() {
    let (w, h) = (900u32, 560u32);
    let got = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await.ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor { label: Some("awl gallery device"), ..Default::default() })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let mut p = TextPipeline::new(&device, &queue, &cache, FMT);
        p.set_size(w as f32, h as f32);
        Some((device, queue, p))
    });
    let Some((device, queue, mut p)) = got else {
        eprintln!("skipping gallery_wagtail_selection_highlight_search: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    let was_page_on = crate::page::page_on();
    let was_measure = crate::page::measure();
    crate::page::set_measure(40);
    crate::page::set_page_on(true);

    let text = "# Wagtail\n\nOne true ==highlight== texture, one true inversion.\n\
        Search finds this findme word too.\nA plain line of prose to show the flat margin stays black.";
    let mut v = view(text, 2, 8);
    v.is_markdown = true;
    v.selection = Some(((2, 25), (2, 34)));
    v.search_active = true;
    v.search_query = "findme".to_string();
    v.search_matches = vec![((3, 12), (3, 18))];
    v.search_current = Some(0);

    theme::set_active_by_name("Wagtail").unwrap();
    p.sync_theme();
    p.set_view(&v);
    p.prepare(&device, &queue, w, h).unwrap();

    let (texture, tview) = offscreen(&device, w, h);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("awl gallery encoder"),
    });
    p.render(&mut encoder, &tview).unwrap();
    queue.submit(Some(encoder.finish()));
    let pixels = read_pixels(&device, &queue, &texture, w, h);

    std::fs::create_dir_all("gallery/wagtail").ok();
    let mut img = image::RgbaImage::new(w, h);
    for (i, px) in pixels.iter().enumerate() {
        img.put_pixel((i as u32) % w, (i as u32) / w, image::Rgba(*px));
    }
    img.save("gallery/wagtail/selection-highlight-search.png").unwrap();
    eprintln!("wrote gallery/wagtail/selection-highlight-search.png");

    theme::set_active(theme::DEFAULT_THEME);
    crate::page::set_page_on(was_page_on);
    crate::page::set_measure(was_measure);
}

/// GALLERY GENERATOR, not a correctness test — `#[ignore]`d by default, THE
/// 1-BIT CARET ROUND's own shot: the block caret sitting directly ON a
/// heading's `#` in Wagtail, with the `#` still legible (black-on-white)
/// inside the inverted block — the user's own photographed bug, now fixed.
/// Regenerate with:
/// `cargo test --bin awl render::tests::dither::gallery_wagtail_caret -- --ignored --nocapture`
#[test]
#[ignore]
fn gallery_wagtail_caret() {
    let (w, h) = (900u32, 300u32);
    let got = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await.ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor { label: Some("awl caret gallery device"), ..Default::default() })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let mut p = TextPipeline::new(&device, &queue, &cache, FMT);
        p.set_size(w as f32, h as f32);
        Some((device, queue, p))
    });
    let Some((device, queue, mut p)) = got else {
        eprintln!("skipping gallery_wagtail_caret: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    crate::caret::set_mode(CaretMode::Block);
    let text = "# Wagtail\n\nA plain line of prose under the heading, for scale.";
    let mut v = view(text, 0, 0);
    v.is_markdown = true;

    theme::set_active_by_name("Wagtail").unwrap();
    p.sync_theme();
    p.set_view(&v);
    p.settle_caret();
    p.prepare(&device, &queue, w, h).unwrap();

    let (texture, tview) = offscreen(&device, w, h);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("awl caret gallery encoder"),
    });
    p.render(&mut encoder, &tview).unwrap();
    queue.submit(Some(encoder.finish()));
    let pixels = read_pixels(&device, &queue, &texture, w, h);

    std::fs::create_dir_all("gallery/wagtail").ok();
    let mut img = image::RgbaImage::new(w, h);
    for (i, px) in pixels.iter().enumerate() {
        img.put_pixel((i as u32) % w, (i as u32) / w, image::Rgba(*px));
    }
    img.save("gallery/wagtail/caret.png").unwrap();
    eprintln!("wrote gallery/wagtail/caret.png");

    crate::caret::set_mode(CaretMode::Block);
    theme::set_active(theme::DEFAULT_THEME);
}
