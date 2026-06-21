//! Headless one-frame capture: render the shared text pipeline to an offscreen
//! texture, read the pixels back to the CPU, and write a PNG + a JSON sidecar.
//!
//! This is the PRIMARY verification path for the project: same input => byte
//! stable PNG, plus a machine-readable description of render state.

use anyhow::{Context, Result};
use glyphon::Cache;
use std::io::Write;
use std::path::Path;

use crate::buffer::Buffer;
use crate::render::{self, TextPipeline, ViewState};

/// Deterministic canvas size for headless renders.
pub const CANVAS_WIDTH: u32 = 1200;
pub const CANVAS_HEIGHT: u32 = 800;
/// Offscreen format. Srgb so glyphon's default (sRGB) blending matches windowed.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Round a row byte count up to wgpu's required 256-byte alignment for buffer
/// copies (`COPY_BYTES_PER_ROW_ALIGNMENT`).
fn align_256(n: u32) -> u32 {
    (n + 255) & !255
}

/// How the caret is posed for a headless capture. Both modes are fully
/// deterministic (no clock): the same input yields a byte-identical PNG.
#[derive(Clone, Copy, PartialEq)]
enum CaretMode {
    /// Caret settled exactly on target: the resting amber rounded square on the
    /// glyph.
    Rest,
    /// Caret part-way through a synthetic horizontal glide: a trailing amber
    /// underline streak dropped to the baseline.
    Motion,
}

/// Deterministic overrides for the verification hooks. All default to the
/// byte-stable baseline (zoom 1.0, cursor-follow scroll, no selection), so a
/// plain `--screenshot` is unaffected. Each field is applied verbatim into the
/// render snapshot, letting a reviewer capture a selection / zoom / scroll still
/// as a reproducible PNG.
#[derive(Clone, Default)]
pub struct CaptureOpts {
    /// Zoom factor (None = 1.0).
    pub zoom: Option<f32>,
    /// Explicit top scroll line (None = cursor-follow default).
    pub scroll: Option<usize>,
    /// Selection as ((l0,c0),(l1,c1)) in line/col (None = no selection).
    pub selection: Option<((usize, usize), (usize, usize))>,
    /// Synthetic IME preedit (composition) string to render at the cursor for the
    /// IME verify path (None/empty = no composition). Drawn underlined via the
    /// same Advanced-shaping path as the live IME overlay; never enters the
    /// buffer, so the capture stays deterministic.
    pub preedit: Option<String>,
}

/// Render the loaded `buffer` to an offscreen 1200x800 texture and write
/// `<out>.png` and the sidecar `<out>.json`. Opens NO window. The caret is drawn
/// AT REST (the resting amber rounded square on the glyph) at the buffer's current
/// cursor position, so the capture is byte-deterministic. (Plain no-options entry
/// point; `main` uses
/// [`capture_with`], but this is kept as the canonical baseline API.)
#[allow(dead_code)]
pub fn capture(out_png: &Path, buffer: &Buffer) -> Result<()> {
    pollster::block_on(capture_async(
        out_png,
        buffer,
        CaretMode::Rest,
        &CaptureOpts::default(),
    ))
}

/// Like [`capture`] but with deterministic state overrides (zoom / scroll /
/// selection) for the verification hooks. Still byte-deterministic for a fixed
/// set of options.
pub fn capture_with(out_png: &Path, buffer: &Buffer, opts: &CaptureOpts) -> Result<()> {
    pollster::block_on(capture_async(out_png, buffer, CaretMode::Rest, opts))
}

/// Like [`capture`], but renders ONE frame of a caret MID-GLIDE — a synthetic,
/// deterministic still showing the caret dropped to the baseline and stretched
/// into a trailing underline streak partway along its path, so the temporal
/// effect is inspectable from a screenshot. No clock is consulted.
pub fn capture_motion(out_png: &Path, buffer: &Buffer) -> Result<()> {
    pollster::block_on(capture_async(
        out_png,
        buffer,
        CaretMode::Motion,
        &CaptureOpts::default(),
    ))
}

async fn capture_async(
    out_png: &Path,
    buffer: &Buffer,
    caret_mode: CaretMode,
    opts: &CaptureOpts,
) -> Result<()> {
    // --- Device (no surface needed for offscreen) -------------------------
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    // (capture runs without a window, so no display handle is needed)
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .context("no wgpu adapter for headless capture")?;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("awl headless device"),
            ..Default::default()
        })
        .await
        .context("request_device failed")?;

    let (width, height) = (CANVAS_WIDTH, CANVAS_HEIGHT);

    // --- Offscreen color target ------------------------------------------
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("awl offscreen"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    // --- Text pipeline (shared with windowed) ----------------------------
    let (cursor_line, cursor_col) = buffer.cursor_line_col();
    let zoom = render::clamp_zoom(opts.zoom.unwrap_or(1.0));
    let line_height = render::LINE_HEIGHT * zoom;
    // Spell-check the buffer text for the headless capture too, so `--screenshot`
    // renders the squiggles. Deterministic (fixed text -> fixed spans). If the
    // bundled dictionary fails to parse, report it and render without squiggles.
    let misspelled = match crate::spell::SpellChecker::new() {
        Ok(sc) => sc.misspellings(&buffer.text()),
        Err(e) => {
            eprintln!("spell-check disabled for capture: {e}");
            Vec::new()
        }
    };

    let cache = Cache::new(&device);
    let mut pipeline = TextPipeline::new(&device, &queue, &cache, FORMAT);
    pipeline.set_size(width as f32, height as f32);

    // Shape the document first (at zoom 0/no-scroll) so the pipeline can report
    // wrap-aware row counts. Scroll is counted in VISUAL ROWS, so an explicit
    // `--scroll N` is N visual rows clamped to the document's total visual rows,
    // and the cursor-follow default uses the cursor's VISUAL row. Both need the
    // buffer shaped, which a preliminary `set_view` provides.
    let mut vstate = ViewState {
        text: buffer.text(),
        cursor_line,
        cursor_col,
        scroll_lines: 0,
        zoom,
        selection: opts.selection,
        preedit: opts.preedit.clone().unwrap_or_default(),
        misspelled,
    };
    pipeline.set_view(&vstate);

    // Now compute the VISUAL-ROW scroll from the shaped buffer.
    let total_rows = pipeline.total_visual_rows();
    let scroll_lines = match opts.scroll {
        // `--scroll N` is N VISUAL rows; 999 etc. clamps to the last reachable row.
        Some(n) => n.min(render::max_scroll(total_rows, height as f32, line_height)),
        None => {
            // Cursor-follow default: scroll so the cursor's VISUAL row is on screen
            // (top, since the headless cursor starts at the buffer start unless a
            // selection moved it). Mirrors the windowed minimal-adjust-from-0.
            let cursor_row = pipeline.visual_row_of(cursor_line, cursor_col);
            let visible = render::visible_lines_z(height as f32, line_height);
            let mut s = 0usize;
            if cursor_row >= s + visible {
                s = cursor_row + 1 - visible;
            }
            s.min(render::max_scroll(total_rows, height as f32, line_height))
        }
    };
    vstate.scroll_lines = scroll_lines;
    pipeline.set_view(&vstate);
    // Pose the caret deterministically for this capture.
    match caret_mode {
        CaretMode::Rest => pipeline.settle_caret(),
        CaretMode::Motion => pipeline.inject_motion_demo(),
    }
    pipeline.prepare(&device, &queue, width, height)?;

    // --- Readback buffer (row-aligned) -----------------------------------
    let unpadded_bpr = width * 4; // RGBA8
    let padded_bpr = align_256(unpadded_bpr);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("awl readback"),
        size: (padded_bpr * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    // --- Encode: draw, then copy texture -> buffer -----------------------
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("awl capture encoder"),
    });
    pipeline.render(&mut encoder, &view)?;
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
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
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    // --- Map and read back -----------------------------------------------
    let (tx, rx) = std::sync::mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .context("device poll failed")?;
    rx.recv()
        .context("map_async channel closed")?
        .context("buffer map failed")?;

    // Drop padding: copy each row's unpadded prefix into a tight RGBA buffer.
    let mut rgba = vec![0u8; (unpadded_bpr * height) as usize];
    {
        let mapped = readback.slice(..).get_mapped_range();
        for y in 0..height {
            let src = (y * padded_bpr) as usize;
            let dst = (y * unpadded_bpr) as usize;
            rgba[dst..dst + unpadded_bpr as usize]
                .copy_from_slice(&mapped[src..src + unpadded_bpr as usize]);
        }
    }
    readback.unmap();

    // --- Write PNG --------------------------------------------------------
    let img = image::RgbaImage::from_raw(width, height, rgba)
        .context("failed to build RgbaImage from readback")?;
    img.save(out_png)
        .with_context(|| format!("failed to write PNG {}", out_png.display()))?;

    // --- Write JSON sidecar ----------------------------------------------
    write_sidecar(out_png, &vstate, &pipeline)?;

    Ok(())
}

/// Minimal hand-rolled JSON so we don't pull in serde.
fn write_sidecar(out_png: &Path, view: &ViewState, pipeline: &TextPipeline) -> Result<()> {
    let json_path = out_png.with_extension("json");

    let text = &view.text;
    let cursor_line = view.cursor_line;
    let cursor_col = view.cursor_col;

    let first_lines: Vec<String> = text.lines().take(12).map(|s| s.to_string()).collect();
    let first_lines_json = first_lines
        .iter()
        .map(|l| json_string(l))
        .collect::<Vec<_>>()
        .join(", ");

    let json = format!(
        "{{\n  \"schema\": \"awl-capture/1\",\n  \"canvas\": {{ \"width\": {w}, \"height\": {h} }},\n  \"font\": {{ \"family\": \"monospace\", \"size\": {fs}, \"line_height\": {lh} }},\n  \"text_origin\": {{ \"left\": {left}, \"top\": {top} }},\n  \"line_count\": {lc},\n  \"scroll_lines\": {sl},\n  \"cursor\": {{ \"line\": {cl}, \"col\": {cc} }},\n  \"text\": {text_json},\n  \"first_lines\": [{fl}]\n}}\n",
        w = CANVAS_WIDTH,
        h = CANVAS_HEIGHT,
        fs = render::FONT_SIZE,
        lh = render::LINE_HEIGHT,
        left = render::TEXT_LEFT,
        top = render::TEXT_TOP,
        lc = pipeline.line_count(),
        sl = view.scroll_lines,
        cl = cursor_line,
        cc = cursor_col,
        text_json = json_string(text),
        fl = first_lines_json,
    );

    let mut f = std::fs::File::create(&json_path)
        .with_context(|| format!("failed to create {}", json_path.display()))?;
    f.write_all(json.as_bytes())?;
    Ok(())
}

/// Escape a string as a JSON string literal (quotes included).
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
