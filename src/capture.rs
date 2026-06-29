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

/// The sidecar SCHEMA strings, one per emitted shape — the SINGLE source of truth
/// for the version number so a bump is one edit and the `write_sidecar` match arms
/// can't drift from each other:
/// - [`SCHEMA_PLAIN`]: the `--screenshot` single frame (caret block absent).
/// - [`SCHEMA_TIMELINE`]: a `--capture-timeline` step (caret block, no `trail`).
/// - [`SCHEMA_HELD`]: a `--capture-held` step (caret block WITH the `trail`).
pub const SCHEMA_PLAIN: &str = "awl-capture/30";
pub const SCHEMA_TIMELINE: &str = "awl-capture/31";
pub const SCHEMA_HELD: &str = "awl-capture/32";

/// Round a row byte count up to wgpu's required 256-byte alignment for buffer
/// copies (`COPY_BYTES_PER_ROW_ALIGNMENT`).
fn align_256(n: u32) -> u32 {
    (n + 255) & !255
}

/// Request a headless wgpu device + queue — no surface, no window. The adapter /
/// device boilerplate is identical for every capture variant, so it lives here
/// once; all three async entry points open on this.
async fn headless_device() -> Result<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    // (capture runs without a window, so no display handle is needed)
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .context("no wgpu adapter for headless capture")?;
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("awl headless device"),
            ..Default::default()
        })
        .await
        .context("request_device failed")
}

/// Create the offscreen color target (texture + its default view) for a headless
/// render: a single-sample [`FORMAT`] texture usable as a render attachment AND a
/// copy source. The descriptor is the same in every variant, so it lives here once.
fn offscreen_target(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
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
    (texture, view)
}

/// Read an already-rendered offscreen `texture` back to the CPU as a tight
/// [`image::RgbaImage`]: allocate a row-aligned readback buffer, encode + submit the
/// texture->buffer copy, map + poll to completion, then drop wgpu's 256-byte row
/// padding into a packed RGBA image. The caret/document must already be drawn into
/// `texture` (submitted) before calling. Shared by the single-frame path and BOTH
/// per-step loops so the readback dance lives in one place.
fn read_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<image::RgbaImage> {
    // --- Readback buffer (row-aligned) -----------------------------------
    let unpadded_bpr = width * 4; // RGBA8
    let padded_bpr = align_256(unpadded_bpr);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("awl readback"),
        size: (padded_bpr * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    // --- Encode + submit the texture -> buffer copy ----------------------
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("awl capture copy encoder"),
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

    image::RgbaImage::from_raw(width, height, rgba)
        .context("failed to build RgbaImage from readback")
}

/// Build a capture [`ViewState`] with every search / overlay field at its INERT
/// default and the project-derived fields (`project_status`, `project_dirty`,
/// `is_markdown`, `syn_lang`) filled ONCE — so a new ViewState field is added in a
/// single place, and the `name · branch` status formatting lives here only. The
/// timeline / held paths use this verbatim (overriding only `held`); the single-
/// frame path overrides the search / overlay / selection fields it actually drives.
fn base_viewstate(
    buffer: &Buffer,
    project: &Option<ProjectInfo>,
    cursor: (usize, usize),
    zoom: f32,
    misspelled: Vec<crate::spell::Misspelling>,
    held: bool,
) -> ViewState {
    ViewState {
        text: buffer.text(),
        cursor_line: cursor.0,
        cursor_col: cursor.1,
        scroll_lines: 0,
        zoom,
        selection: None,
        preedit: String::new(),
        misspelled,
        is_edit_move: false,
        held,
        search_matches: Vec::new(),
        search_current: None,
        search_query: String::new(),
        search_active: false,
        search_case_sensitive: false,
        search_replace_active: false,
        search_replacement: String::new(),
        search_editing_replacement: false,
        overlay_active: false,
        overlay_query: String::new(),
        overlay_items: Vec::new(),
        overlay_bindings: Vec::new(),
        overlay_times: Vec::new(),
        overlay_selected: 0,
        overlay_hint: String::new(),
        project_status: project
            .as_ref()
            .map(|p| match &p.branch {
                Some(b) => format!("{} · {}", p.name, b),
                None => p.name.clone(),
            })
            .unwrap_or_default(),
        project_dirty: project.as_ref().map(|p| p.dirty).unwrap_or(false),
        is_markdown: buffer.is_markdown(),
        syn_lang: buffer.syntax_lang(),
    }
}

/// Cursor-follow scroll (in VISUAL ROWS) for a settled capture: scroll just enough
/// to bring the `(line, col)` cursor's visual row on screen from the top, clamped
/// to the document's max scroll. Variable-row-height aware via the pixel-accurate
/// pipeline helpers. Shared by the timeline / held paths and the focus-Off branch
/// of the single-frame path, so the three never drift (the focus-on single-frame
/// path CENTERS instead, so it keeps its own branch). `height` is the canvas px.
fn follow_scroll(pipeline: &TextPipeline, line: usize, col: usize, height: f32) -> usize {
    let row = pipeline.visual_row_of(line, col);
    pipeline
        .scroll_to_show_row(row, 0, height)
        .min(pipeline.max_scroll_rows(height))
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
    /// Caret part-way through a synthetic VERTICAL glide: a thin amber bar slid to
    /// the cell's left edge, trailing up the lines it passed.
    MotionVertical,
    /// Caret part-way through a synthetic DIAGONAL glide (different row AND column):
    /// a true slanted amber tracer from source to target.
    MotionDiagonal,
}

/// Deterministic overrides for the verification hooks. All default to the
/// byte-stable baseline (zoom 1.0, cursor-follow scroll, no selection), so a
/// plain `--screenshot` is unaffected. Each field is applied verbatim into the
/// render snapshot, letting a reviewer capture a selection / zoom / scroll still
/// as a reproducible PNG.
/// Read-only project metadata for the sidecar `project` block (`--root`-derived).
#[derive(Clone)]
pub struct ProjectInfo {
    pub root: std::path::PathBuf,
    pub name: String,
    pub branch: Option<String>,
    pub dirty: bool,
    /// The EFFECTIVE notes_root (flag > config > `~/notes`), surfaced so a
    /// `--config`-driven launch's configured folder is verifiable from the sidecar
    /// with no flags. `None` (timeline/held paths) -> JSON null.
    pub notes_root: Option<std::path::PathBuf>,
    /// The EFFECTIVE workspace (flag > config > root.parent). `None` -> JSON null.
    pub workspace: Option<std::path::PathBuf>,
}

/// Summoned-overlay state for the sidecar `overlay` block. Populated when a
/// `--keys` replay left the go-to / switch overlay open (or when it accepted).
#[derive(Clone)]
pub struct OverlayInfo {
    pub active: bool,
    pub mode: &'static str,
    pub query: String,
    pub items: Vec<String>,
    /// Command palette only: binding labels parallel to `items` (each command's
    /// current chord). Empty for every other mode; emitted as a parallel array so
    /// the palette's binding column is verifiable from the sidecar.
    pub bindings: Vec<String>,
    pub selected_index: usize,
    /// The per-kind control-hint line drawn dim at the foot of the card (e.g.
    /// "->/C-f open   Enter select   <-/C-b up" for switch-project). Surfaced to
    /// the sidecar so the discoverability hint is agent-verifiable.
    pub hint: String,
    /// Browse only: the root-relative directory the current level lists (`None` =
    /// the root). Surfaced so a `--keys` descend/ascend is verifiable; emitted as
    /// JSON null for the goto/switch modes.
    pub browse_dir: Option<String>,
}

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
    /// Live isearch query to render the panel + highlights deterministically
    /// (None = no search). Matches are computed against the loaded buffer.
    pub search: Option<String>,
    /// Case-sensitive toggle for the headless search (default false).
    pub search_case_sensitive: bool,
    /// REPLACE mode revealed on the search panel (default false). A `--keys`
    /// replay of Cmd-Option-F (`s-M-f`) opens the panel into replace mode, so this
    /// is verifiable from the capture; the replacement itself can't be typed
    /// headlessly (the documented isearch-input gap), so it stays empty.
    pub search_replace_active: bool,
    /// The replacement string (always empty headlessly; present for symmetry).
    pub search_replacement: String,
    /// The active project (`--root`-derived) for the sidecar `project` block.
    /// None (default) -> `project: null` so a plain `--screenshot` is unchanged.
    pub project: Option<ProjectInfo>,
    /// The summoned overlay state for the sidecar `overlay` block. None ->
    /// overlay inactive.
    pub overlay: Option<OverlayInfo>,
    /// PHYSICAL canvas dimensions for this run (`--capture-size WxH`). `None` =
    /// the byte-stable default [`CANVAS_WIDTH`]x[`CANVAS_HEIGHT`] (1200x800), so a
    /// plain `--screenshot` is unchanged. Lets a capture render at the REAL window
    /// size so size-dependent layout bugs (e.g. the page right-margin) are visible.
    pub canvas: Option<(u32, u32)>,
    /// Display DPI `scale_factor` fed to the renderer metrics (`--capture-dpi N`).
    /// `None` = 1.0 (today's implied capture scale, a no-op via `set_dpi`'s guard),
    /// so the no-flag path stays byte-identical. A 2400x1600 canvas at dpi 2.0
    /// renders like a 1200x800 LOGICAL retina window (text + column geometry scale
    /// exactly like the live retina app).
    pub dpi: Option<f32>,
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

/// Like [`capture_motion`], but a VERTICAL mid-glide: the caret has slid to a thin
/// amber bar on the cell's left edge, trailing up the lines it just travelled.
pub fn capture_motion_vertical(out_png: &Path, buffer: &Buffer) -> Result<()> {
    pollster::block_on(capture_async(
        out_png,
        buffer,
        CaretMode::MotionVertical,
        &CaptureOpts::default(),
    ))
}

/// Like [`capture_motion`], but a DIAGONAL mid-glide: the caret is part-way through
/// a jump between two points on different rows AND columns, so its trail is a true
/// slanted tracer from source to target (not an axis-snapped bar).
pub fn capture_motion_diagonal(out_png: &Path, buffer: &Buffer) -> Result<()> {
    pollster::block_on(capture_async(
        out_png,
        buffer,
        CaretMode::MotionDiagonal,
        &CaptureOpts::default(),
    ))
}

/// DETERMINISTIC TIMELINE capture. After a `--keys` replay sets up a NAVIGATION
/// caret move (the buffer cursor now rests at the DESTINATION `buffer`; `origin`
/// is the line/col it started from), prime the caret spring at `origin`, start the
/// glide toward the destination, then advance a VIRTUAL clock through the
/// cumulative `steps` (ms since the move started) — the dt fed to each step is the
/// delta to the previous entry. After EACH step a frame is rendered to
/// `<out>.t<ms>.png` + `<out>.t<ms>.json`, the sidecar recording the caret's
/// animated `pos` + `animating` flag so the trajectory (origin -> mid -> settled)
/// is machine-readable. The dt is INJECTED (no real clock, no RNG), so stepping
/// the same sequence twice yields byte-identical frames + sidecars.
pub fn capture_timeline(
    out_png: &Path,
    buffer: &Buffer,
    origin: (usize, usize),
    steps: &[u32],
    opts: &CaptureOpts,
) -> Result<()> {
    pollster::block_on(capture_timeline_async(out_png, buffer, origin, steps, opts))
}

/// DETERMINISTIC HELD-MOTION capture. Reproduces a HELD arrow (the OS auto-repeat
/// that re-aims the caret one char/line every ~30ms): prime the caret at `origin`
/// (where a `--keys` replay left the cursor), then for EACH cumulative-ms entry in
/// `steps` RE-TARGET the caret one step further in `dir` (one char for Left/Right,
/// one line for Up/Down) with `held=true`, advance the VIRTUAL clock by the delta
/// to the previous entry, and render a frame (`<out>.t<ms>.png` + `.json`). The
/// sidecar records the caret pos AND the drawn TRAIL geometry (length + endpoints +
/// holding flag) so the held streak is machine-verifiable per step. Both the held
/// flag and the dt are INJECTED (no winit, no real clock, no RNG), so the run is
/// byte-deterministic.
pub fn capture_held(
    out_png: &Path,
    buffer: &Buffer,
    origin: (usize, usize),
    dir: HeldDir,
    steps: &[u32],
    opts: &CaptureOpts,
) -> Result<()> {
    pollster::block_on(capture_held_async(out_png, buffer, origin, dir, steps, opts))
}

async fn capture_async(
    out_png: &Path,
    buffer: &Buffer,
    caret_mode: CaretMode,
    opts: &CaptureOpts,
) -> Result<()> {
    // --- Device (no surface needed for offscreen) -------------------------
    let (device, queue) = headless_device().await?;

    // PHYSICAL canvas dims for this run: the flagged `--capture-size`, else the
    // byte-stable default. DPI defaults to 1.0 (a `set_dpi` no-op).
    let (width, height) = opts.canvas.unwrap_or((CANVAS_WIDTH, CANVAS_HEIGHT));
    let dpi = opts.dpi.unwrap_or(1.0);

    // --- Offscreen color target ------------------------------------------
    let (texture, view) = offscreen_target(&device, width, height);

    // --- Text pipeline (shared with windowed) ----------------------------
    let (cursor_line, cursor_col) = buffer.cursor_line_col();
    let zoom = render::clamp_zoom(opts.zoom.unwrap_or(1.0));
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

    // --- Search panel (deterministic headless isearch) -------------------
    // Compute matches against the loaded buffer, pick current = first match at
    // or after the cursor (Forward, deterministic) else the first match, and
    // move the resting caret onto the current match. capture takes &Buffer
    // (immutable), so we DO NOT set_cursor; we derive sc_line/sc_col locally and
    // feed them into the ViewState so settle_caret lands the caret on the match.
    let (search_matches, search_current, sc_line, sc_col) = if let Some(q) = &opts.search {
        let cs = opts.search_case_sensitive;
        let raw = crate::search::find_all(&buffer.text(), q, cs);
        let ranges: Vec<((usize, usize), (usize, usize))> = raw
            .iter()
            .map(|m| {
                (
                    buffer.char_to_line_col(m.start),
                    buffer.char_to_line_col(m.end),
                )
            })
            .collect();
        let cur_char = buffer.cursor_char();
        let cur_idx = if raw.is_empty() {
            None
        } else {
            Some(raw.iter().position(|m| m.start >= cur_char).unwrap_or(0))
        };
        let (cl, cc) = match cur_idx {
            Some(i) => buffer.char_to_line_col(raw[i].start),
            None => (cursor_line, cursor_col),
        };
        (ranges, cur_idx, cl, cc)
    } else {
        (Vec::new(), None, cursor_line, cursor_col)
    };
    let search_active = opts.search.is_some();

    let cache = Cache::new(&device);
    let mut pipeline = TextPipeline::new(&device, &queue, &cache, FORMAT);
    pipeline.set_size(width as f32, height as f32);
    // DPI AFTER set_size: set_dpi re-wraps at column_width(), which reads window_w
    // (set by set_size). No-op at the default 1.0, so the no-flag path is unchanged.
    pipeline.set_dpi(dpi);

    // Shape the document first (at zoom 0/no-scroll) so the pipeline can report
    // wrap-aware row counts. Scroll is counted in VISUAL ROWS, so an explicit
    // `--scroll N` is N visual rows clamped to the document's total visual rows,
    // and the cursor-follow default uses the cursor's VISUAL row. Both need the
    // buffer shaped, which a preliminary `set_view` provides.
    // Start from the shared inert-default base (project status + flags filled once),
    // then drive the search / overlay / selection fields this single-frame path
    // verifies. With an active --search the resting caret lands on the current match.
    let mut vstate = base_viewstate(buffer, &opts.project, (sc_line, sc_col), zoom, misspelled, false);
    vstate.selection = opts.selection;
    vstate.preedit = opts.preedit.clone().unwrap_or_default();
    vstate.search_matches = search_matches;
    vstate.search_current = search_current;
    vstate.search_query = opts.search.clone().unwrap_or_default();
    vstate.search_active = search_active;
    vstate.search_case_sensitive = opts.search_case_sensitive;
    // REPLACE mode: a `--keys` replay of Cmd-Option-F opens the panel into replace
    // mode, surfaced here so the second-row render is verifiable. The replacement
    // field can't be typed headlessly (the isearch-input gap), so focus stays on the
    // (empty) replacement and the text is empty.
    vstate.search_replace_active = opts.search_replace_active;
    vstate.search_replacement = opts.search_replacement.clone();
    vstate.search_editing_replacement = opts.search_replace_active;
    vstate.overlay_active = opts.overlay.as_ref().map(|o| o.active).unwrap_or(false);
    vstate.overlay_query = opts.overlay.as_ref().map(|o| o.query.clone()).unwrap_or_default();
    vstate.overlay_items = opts.overlay.as_ref().map(|o| o.items.clone()).unwrap_or_default();
    vstate.overlay_bindings = opts.overlay.as_ref().map(|o| o.bindings.clone()).unwrap_or_default();
    vstate.overlay_selected = opts.overlay.as_ref().map(|o| o.selected_index).unwrap_or(0);
    vstate.overlay_hint = opts.overlay.as_ref().map(|o| o.hint.clone()).unwrap_or_default();
    pipeline.set_view(&vstate);

    // Now compute the VISUAL-ROW scroll from the shaped buffer. Variable-row-height
    // aware (headings): the pixel-accurate pipeline helpers mirror `app.rs`.
    let scroll_lines = match opts.scroll {
        // `--scroll N` is N VISUAL rows; 999 etc. clamps to the last reachable row.
        Some(n) => n.min(pipeline.max_scroll_rows(height as f32)),
        None => {
            // Cursor-follow default: scroll so the cursor's VISUAL row is on screen
            // (from the top, since the headless cursor starts at the buffer start
            // unless a selection moved it). Mirrors the windowed cursor-follow,
            // INCLUDING the focus-mode TYPEWRITER fold: with focus active the row is
            // CENTERED, otherwise it's the minimal-adjust — so a `--focus paragraph`
            // capture verifies the centered scroll deterministically.
            if crate::focus::mode() == crate::focus::FocusMode::Off {
                follow_scroll(&pipeline, sc_line, sc_col, height as f32)
            } else {
                // Focus mode CENTERS the cursor row (the typewriter fold).
                let cursor_row = pipeline.visual_row_of(sc_line, sc_col);
                pipeline
                    .scroll_to_center_row(cursor_row, height as f32)
                    .min(pipeline.max_scroll_rows(height as f32))
            }
        }
    };
    vstate.scroll_lines = scroll_lines;
    pipeline.set_view(&vstate);
    // Pose the caret deterministically for this capture.
    match caret_mode {
        CaretMode::Rest => pipeline.settle_caret(),
        CaretMode::Motion => pipeline.inject_motion_demo(),
        CaretMode::MotionVertical => pipeline.inject_motion_demo_vertical(),
        CaretMode::MotionDiagonal => pipeline.inject_motion_demo_diagonal(),
    }
    // FOCUS MODE: render the SETTLED dim/full state (active unit full, rest dim) with
    // no clock — the crossfade is live-only, so the capture is deterministic.
    pipeline.settle_focus();
    pipeline.prepare(&device, &queue, width, height)?;

    // --- Draw the frame, then read it back via the shared helper ---------
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("awl capture encoder"),
    });
    pipeline.render(&mut encoder, &view)?;
    queue.submit(Some(encoder.finish()));
    let img = read_frame(&device, &queue, &texture, width, height)?;

    // --- Write PNG --------------------------------------------------------
    img.save(out_png)
        .with_context(|| format!("failed to write PNG {}", out_png.display()))?;

    // --- Write JSON sidecar ----------------------------------------------
    write_sidecar(out_png, &vstate, &pipeline, opts, None)?;

    Ok(())
}

async fn capture_timeline_async(
    out_png: &Path,
    buffer: &Buffer,
    origin: (usize, usize),
    steps: &[u32],
    opts: &CaptureOpts,
) -> Result<()> {
    // --- Device (no surface needed for offscreen) -------------------------
    let (device, queue) = headless_device().await?;

    let (width, height) = opts.canvas.unwrap_or((CANVAS_WIDTH, CANVAS_HEIGHT));
    let dpi = opts.dpi.unwrap_or(1.0);

    // --- Offscreen color target (reused each frame) ----------------------
    let (texture, view) = offscreen_target(&device, width, height);

    // --- Text pipeline (shared with windowed) ----------------------------
    let zoom = render::clamp_zoom(opts.zoom.unwrap_or(1.0));
    let misspelled = match crate::spell::SpellChecker::new() {
        Ok(sc) => sc.misspellings(&buffer.text()),
        Err(e) => {
            eprintln!("spell-check disabled for capture: {e}");
            Vec::new()
        }
    };

    // The buffer cursor rests at the DESTINATION; `origin` is where the glide
    // STARTS. Both poses share ONE stationary viewport so only the caret moves
    // across the timeline (the document never scrolls mid-glide).
    let (dest_line, dest_col) = buffer.cursor_line_col();
    let (orig_line, orig_col) = origin;

    let cache = Cache::new(&device);
    let mut pipeline = TextPipeline::new(&device, &queue, &cache, FORMAT);
    pipeline.set_size(width as f32, height as f32);
    pipeline.set_dpi(dpi); // AFTER set_size (reads window_w); no-op at default 1.0.

    // Timeline mode focuses on caret MOTION; the search / overlay verification hooks
    // are not driven here, so they stay at their inert defaults (the shared base).
    // `held` stays false: a NAVIGATION glide (not an edit reflow), so the spring
    // glides A->B instead of snapping — the flag that keeps the trajectory visible.
    let mut vstate = base_viewstate(buffer, &opts.project, (dest_line, dest_col), zoom, misspelled, false);
    // Shape at the destination first so visual-row counts are available; this also
    // PRIMES the spring (first set_caret_target snaps).
    pipeline.set_view(&vstate);

    // ONE fixed scroll for the whole timeline: follow the DESTINATION's visual row
    // (where the caret settles), mirroring capture_async's cursor-follow default.
    let scroll = follow_scroll(&pipeline, dest_line, dest_col, height as f32);
    vstate.scroll_lines = scroll;

    // Pose the spring AT REST on the ORIGIN, then start the glide to the
    // DESTINATION. settle_caret() reads the pipeline's current cursor, so move the
    // cursor to the origin first; the destination set_view then begins a primed
    // navigation glide from origin -> destination.
    vstate.cursor_line = orig_line;
    vstate.cursor_col = orig_col;
    pipeline.set_view(&vstate);
    pipeline.settle_caret();
    vstate.cursor_line = dest_line;
    vstate.cursor_col = dest_col;
    pipeline.set_view(&vstate);
    // FOCUS MODE: the timeline path animates the CARET, not the focus fade — pin the
    // focus coloring to its settled state so the dim/full split stays deterministic.
    pipeline.settle_focus();

    // --- Step the virtual clock + render a frame per entry ----------------
    let mut prev_ms = 0u32;
    for &t_ms in steps {
        // Inject dt = delta to the previous cumulative entry (entry 0 => dt 0, a
        // no-op, so it renders the pre-step frame). The dt is purely injected, so
        // the sequence is byte-deterministic.
        let dt = (t_ms.saturating_sub(prev_ms)) as f32 / 1000.0;
        prev_ms = t_ms;
        pipeline.advance(dt);
        pipeline.prepare(&device, &queue, width, height)?;

        // Draw the frame, then read it back via the shared helper.
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("awl timeline encoder"),
        });
        pipeline.render(&mut encoder, &view)?;
        queue.submit(Some(encoder.finish()));
        let img = read_frame(&device, &queue, &texture, width, height)?;

        // Per-step output paths: <out>.t<ms>.png / .json.
        let stem = out_png
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "capture".to_string());
        let frame_png = out_png.with_file_name(format!("{stem}.t{t_ms}.png"));

        img.save(&frame_png)
            .with_context(|| format!("failed to write PNG {}", frame_png.display()))?;

        let (pos, target, settle, animating) = pipeline.caret_snapshot();
        let (scale, block_w, block_h) = pipeline.caret_pop_report();
        let (cpresent, clen, cvert, cheld, calpha, csweep, ctail, chead) =
            pipeline.caret_cosmetic_report();
        let frame = CaretFrame {
            t_ms,
            pos,
            target,
            settle,
            animating,
            scale,
            block_w,
            block_h,
            trail: None,
            cosmetic: CosmeticReport {
                present: cpresent,
                length: clen,
                vertical: cvert,
                held: cheld,
                alpha: calpha,
                sweep: csweep,
                tail: ctail,
                head: chead,
            },
        };
        write_sidecar(&frame_png, &vstate, &pipeline, opts, Some(&frame))?;
    }

    Ok(())
}

async fn capture_held_async(
    out_png: &Path,
    buffer: &Buffer,
    origin: (usize, usize),
    dir: HeldDir,
    steps: &[u32],
    opts: &CaptureOpts,
) -> Result<()> {
    // --- Device (no surface needed for offscreen) -------------------------
    let (device, queue) = headless_device().await?;

    let (width, height) = opts.canvas.unwrap_or((CANVAS_WIDTH, CANVAS_HEIGHT));
    let dpi = opts.dpi.unwrap_or(1.0);

    // --- Offscreen color target (reused each frame) ----------------------
    let (texture, view) = offscreen_target(&device, width, height);

    // --- Text pipeline (shared with windowed) ----------------------------
    let zoom = render::clamp_zoom(opts.zoom.unwrap_or(1.0));
    let misspelled = match crate::spell::SpellChecker::new() {
        Ok(sc) => sc.misspellings(&buffer.text()),
        Err(e) => {
            eprintln!("spell-check disabled for capture: {e}");
            Vec::new()
        }
    };

    // Per-line char lengths, so each held re-target clamps to a real document
    // position (one char/line at a time, like the OS auto-repeat) instead of
    // running off the end of a line or the document.
    let text = buffer.text();
    let line_lens: Vec<usize> = text.split('\n').map(|l| l.chars().count()).collect();
    let last_line = line_lens.len().saturating_sub(1);
    let (orig_line, orig_col) = origin;

    let cache = Cache::new(&device);
    let mut pipeline = TextPipeline::new(&device, &queue, &cache, FORMAT);
    pipeline.set_size(width as f32, height as f32);
    pipeline.set_dpi(dpi); // AFTER set_size (reads window_w); no-op at default 1.0.

    // Held mode focuses on the caret TRAIL; the search / overlay verification hooks
    // are not driven here, so they stay at their inert defaults (the shared base).
    // HELD / auto-repeat: `held` is latched true for every re-target so the spring
    // stays springy and the lag accumulates into a continuous multi-char streak —
    // the field `--capture-timeline` hardcodes false; DRIVING it true on the virtual
    // clock is the whole point of this mode.
    let mut vstate = base_viewstate(buffer, &opts.project, (orig_line, orig_col), zoom, misspelled, true);
    // Shape at the origin first so visual-row counts are available.
    pipeline.set_view(&vstate);

    // ONE fixed scroll for the whole run: follow the ORIGIN's visual row, mirroring
    // the timeline path. The held re-targets move at most a handful of cells, so the
    // viewport stays put (a mid-run rescroll would break determinism / the trail).
    let scroll = follow_scroll(&pipeline, orig_line, orig_col, height as f32);
    vstate.scroll_lines = scroll;

    // Pose the spring AT REST on the ORIGIN (the initial key PRESS, not yet a
    // repeat): settle_caret reads the pipeline's current cursor, which set_view just
    // placed at the origin.
    pipeline.set_view(&vstate);
    pipeline.settle_caret();
    // FOCUS MODE: pin the dim/full split to its settled state (the held run animates
    // the CARET, not the focus fade), so the coloring stays deterministic.
    pipeline.settle_focus();

    // --- Step the virtual clock: one held re-target + advance per entry ---
    let mut prev_ms = 0u32;
    let mut cur = (orig_line, orig_col);
    for &t_ms in steps {
        // One OS auto-repeat: re-aim the caret one char/line further in `dir`
        // (clamped to the document), keeping held=true so the spring stays springy.
        cur = step_held(cur, dir, &line_lens, last_line);
        vstate.cursor_line = cur.0;
        vstate.cursor_col = cur.1;
        pipeline.set_view(&vstate);

        // Inject dt = delta to the previous cumulative entry (entry 0 => dt 0, a
        // re-target with no advance, so the trail starts forming). The dt is purely
        // injected, so the sequence is byte-deterministic.
        let dt = (t_ms.saturating_sub(prev_ms)) as f32 / 1000.0;
        prev_ms = t_ms;
        pipeline.advance(dt);
        pipeline.prepare(&device, &queue, width, height)?;

        // Draw the frame, then read it back via the shared helper.
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("awl held encoder"),
        });
        pipeline.render(&mut encoder, &view)?;
        queue.submit(Some(encoder.finish()));
        let img = read_frame(&device, &queue, &texture, width, height)?;

        // Per-step output paths: <out>.t<ms>.png / .json.
        let stem = out_png
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "capture".to_string());
        let frame_png = out_png.with_file_name(format!("{stem}.t{t_ms}.png"));

        img.save(&frame_png)
            .with_context(|| format!("failed to write PNG {}", frame_png.display()))?;

        let (pos, target, settle, animating) = pipeline.caret_snapshot();
        let (holding, length, tail, head) = pipeline.caret_trail_report();
        let (scale, block_w, block_h) = pipeline.caret_pop_report();
        let (cpresent, clen, cvert, cheld, calpha, csweep, ctail, chead) =
            pipeline.caret_cosmetic_report();
        let frame = CaretFrame {
            t_ms,
            pos,
            target,
            settle,
            animating,
            scale,
            block_w,
            block_h,
            trail: Some(TrailReport {
                holding,
                length,
                tail,
                head,
            }),
            cosmetic: CosmeticReport {
                present: cpresent,
                length: clen,
                vertical: cvert,
                held: cheld,
                alpha: calpha,
                sweep: csweep,
                tail: ctail,
                head: chead,
            },
        };
        write_sidecar(&frame_png, &vstate, &pipeline, opts, Some(&frame))?;
    }

    Ok(())
}

/// Advance a (line, col) one step in `dir` like a single OS auto-repeat, clamped
/// to the document: Left/Right move one CHAR within the line (saturating at the
/// ends), Up/Down move one LINE (clamped to `[0, last_line]`) with the column
/// pinned to the destination line's length. Pure, so the held loop stays
/// deterministic.
fn step_held(
    (line, col): (usize, usize),
    dir: HeldDir,
    line_lens: &[usize],
    last_line: usize,
) -> (usize, usize) {
    let len_at = |l: usize| line_lens.get(l).copied().unwrap_or(0);
    match dir {
        HeldDir::Left => (line, col.saturating_sub(1)),
        HeldDir::Right => (line, (col + 1).min(len_at(line))),
        HeldDir::Up => {
            let l = line.saturating_sub(1);
            (l, col.min(len_at(l)))
        }
        HeldDir::Down => {
            let l = (line + 1).min(last_line);
            (l, col.min(len_at(l)))
        }
    }
}

/// One timeline frame's caret-spring snapshot, written into the sidecar `caret`
/// block so a `--capture-timeline` step's trajectory is machine-readable: the
/// animated `pos` (where the caret is drawn THIS step), the true `target`, the
/// [0,1] `settle_factor`, and whether the spring is still animating. `t_ms` is the
/// cumulative virtual-clock time (ms since the move started) this frame renders.
struct CaretFrame {
    t_ms: u32,
    pos: (f32, f32),
    target: (f32, f32),
    settle: f32,
    animating: bool,
    /// The cosmetic SQUASH-POP factor (1.0 settled, dipping to `CARET_POP_SCALE`
    /// right after a move) and the caret BLOCK rect's DRAWN width/height (the morph
    /// geometry already multiplied by `scale`). Lets a timeline run assert, straight
    /// from the JSON, that the block starts squashed (<1) and eases back to full size
    /// while the position stays pinned to target. From `TextPipeline::caret_pop_report`.
    scale: f32,
    block_w: f32,
    block_h: f32,
    /// The drawn TRAIL geometry, present ONLY for a `--capture-held` step (the
    /// plain `--capture-timeline` path leaves it `None`). Carries the held latch +
    /// the streak length/endpoints so a held run is machine-verifiable: each step's
    /// `length` should clear the streak gap and never collapse to zero.
    trail: Option<TrailReport>,
    /// The COSMETIC | TRAIL drawn OVER the snapped caret this step (present on BOTH the
    /// timeline AND held paths, since the cosmetic streak is what both now verify).
    /// `present` flags whether a streak draws, with its `length`/`direction`/`alpha` +
    /// endpoints, so a capture can assert: a vertical move shows the | , a 1-char hop
    /// shows none, a held-down run is present + steady, a held-right run shows none.
    cosmetic: CosmeticReport,
}

/// The caret's COSMETIC | TRAIL geometry for a capture step's sidecar `caret.cosmetic`
/// block: whether a streak is `present`, its on-screen `length` + `alpha` + whether it
/// is the `vertical` up/down | , and the `tail`/`head` endpoints in canvas pixels.
struct CosmeticReport {
    present: bool,
    length: f32,
    vertical: bool,
    held: bool,
    alpha: f32,
    /// The eased SWEEP progress in [0,1]: 0 = the streak's leading edge sits at the OLD
    /// caret position (just kicked), 1 = it has swept onto the NEW (caret) position.
    /// Lets a timeline assert the directional sweep old→new (and held = 1.0 steady)
    /// straight from JSON without re-deriving it from the endpoints.
    sweep: f32,
    tail: (f32, f32),
    head: (f32, f32),
}

/// The caret's drawn trailing-streak geometry for a held-capture step's sidecar
/// `caret.trail` block: the latched `holding` flag, the on-screen streak `length`
/// along the travel axis, and the trail's `tail` (origin-side) + `head`
/// (caret-side) endpoints in canvas pixels.
struct TrailReport {
    holding: bool,
    length: f32,
    tail: (f32, f32),
    head: (f32, f32),
}

/// Which arrow is HELD for a `--capture-held` run. Left/Right re-target the caret
/// one CHARACTER per step; Up/Down one LINE per step — exactly what an OS
/// auto-repeat does, replayed on the virtual clock.
#[derive(Clone, Copy, PartialEq)]
pub enum HeldDir {
    Left,
    Right,
    Up,
    Down,
}

/// Minimal hand-rolled JSON so we don't pull in serde. `caret` is `Some` ONLY for
/// a `--capture-timeline`/`--capture-held` step (it adds the per-step `caret` block —
/// including the cosmetic squash-pop `pop_scale` + drawn `block` size — and selects
/// [`SCHEMA_TIMELINE`]/[`SCHEMA_HELD`]); the plain `--screenshot` path passes `None`,
/// keeping its byte-stable [`SCHEMA_PLAIN`] sidecar unchanged.
fn write_sidecar(
    out_png: &Path,
    view: &ViewState,
    pipeline: &TextPipeline,
    opts: &CaptureOpts,
    caret: Option<&CaretFrame>,
) -> Result<()> {
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

    let search_cur = view
        .search_current
        .map(|i| i.to_string())
        .unwrap_or_else(|| "null".into());
    // Selection block: `null` when there is no active region, else the ordered
    // ((l0,c0),(l1,c1)) endpoints. Lets a reviewer assert the post-`--keys`
    // region (e.g. C-Space + motion) straight from the sidecar.
    let selection_json = match view.selection {
        Some(((l0, c0), (l1, c1))) => format!(
            "{{ \"start\": {{ \"line\": {l0}, \"col\": {c0} }}, \"end\": {{ \"line\": {l1}, \"col\": {c1} }} }}"
        ),
        None => "null".to_string(),
    };
    // Active theme block: the world the capture was rendered with. Schema bumped
    // 2 -> 3 to carry it. `font.family` reports the active theme's display font,
    // which is now LIVE: the document is shaped with that family (Family::Name), so
    // the sidecar's reported family matches the glyph shapes actually rendered.
    let active = crate::theme::active();
    // The EFFECTIVE caret mode this capture rendered (explicit --caret-mode
    // override, else the font-derived default), so a reviewer can assert which
    // caret look the PNG shows straight from the sidecar.
    let caret_mode = match crate::caret::mode() {
        crate::caret::CaretMode::Block => "block",
        crate::caret::CaretMode::Morph => "morph",
        crate::caret::CaretMode::Ibeam => "ibeam",
    };
    // Read-only PROJECT block (`--root`-derived). `null` when no active project,
    // so a plain `--screenshot` keeps its byte-stable baseline. `dirty` is a bare
    // bool; nothing here colorizes it (the dim-dot styling is a render concern).
    let project_json = match &opts.project {
        Some(p) => {
            let branch = p
                .branch
                .as_ref()
                .map(|b| json_string(b))
                .unwrap_or_else(|| "null".into());
            let opt_path = |p: &Option<std::path::PathBuf>| {
                p.as_ref()
                    .map(|v| json_string(&v.to_string_lossy()))
                    .unwrap_or_else(|| "null".into())
            };
            format!(
                "{{ \"root\": {}, \"name\": {}, \"branch\": {}, \"dirty\": {}, \"notes_root\": {}, \"workspace\": {} }}",
                json_string(&p.root.to_string_lossy()),
                json_string(&p.name),
                branch,
                p.dirty,
                opt_path(&p.notes_root),
                opt_path(&p.workspace),
            )
        }
        None => "null".to_string(),
    };
    // SUMMONED-OVERLAY block. `active: false` (default) when no overlay is open;
    // otherwise the mode / query / filtered items / selected index, so the whole
    // go-to flow (open -> type -> move -> Enter) is verifiable from the sidecar.
    let overlay_json = match &opts.overlay {
        Some(o) => {
            let items = o
                .items
                .iter()
                .map(|i| json_string(i))
                .collect::<Vec<_>>()
                .join(", ");
            let bindings = o
                .bindings
                .iter()
                .map(|b| json_string(b))
                .collect::<Vec<_>>()
                .join(", ");
            let browse_dir = o
                .browse_dir
                .as_ref()
                .map(|d| json_string(d))
                .unwrap_or_else(|| "null".into());
            format!(
                "{{ \"active\": {}, \"mode\": {}, \"query\": {}, \"selected_index\": {}, \"browse_dir\": {}, \"hint\": {}, \"items\": [{}], \"bindings\": [{}] }}",
                o.active,
                json_string(o.mode),
                json_string(&o.query),
                o.selected_index,
                browse_dir,
                json_string(&o.hint),
                items,
                bindings
            )
        }
        None => "{ \"active\": false, \"mode\": null, \"query\": \"\", \"selected_index\": null, \"browse_dir\": null, \"hint\": null, \"items\": [], \"bindings\": [] }".to_string(),
    };
    // PAGE MODE block: the centered-column geometry actually rendered + the active
    // world's margin gradient, so a reviewer can assert the page shape + the
    // figure/ground from the sidecar. `text_origin.left` is TRUTHFUL — it reports
    // where the TEXT actually starts (the centered column left PLUS the page-mode
    // writing inset), while `page.column.left` reports the surface edge.
    // CANVAS block: the PHYSICAL render dims + the dpi the geometry was scaled by,
    // so geometry assertions are self-describing. Byte-stable default: with NO
    // `--capture-size`/`--capture-dpi` flags, emit today's exact `{ "width", "height" }`
    // string (no `dpi` key) so every existing sidecar is unchanged; a non-default run
    // appends `"dpi"`.
    let (canvas_w, canvas_h) = opts.canvas.unwrap_or((CANVAS_WIDTH, CANVAS_HEIGHT));
    let canvas_json = match (opts.canvas, opts.dpi) {
        (None, None) => format!("{{ \"width\": {canvas_w}, \"height\": {canvas_h} }}"),
        _ => format!(
            "{{ \"width\": {canvas_w}, \"height\": {canvas_h}, \"dpi\": {} }}",
            opts.dpi.unwrap_or(1.0)
        ),
    };
    let (page_on, page_measure, col_left, col_w) = pipeline.page_geometry();
    let (gd0, gd1) = crate::theme::margin_dir();
    let page_json = format!(
        "{{ \"on\": {}, \"measure\": {}, \"column\": {{ \"left\": {}, \"width\": {} }}, \"gradient\": {{ \"from\": {}, \"to\": {}, \"dir\": [{}, {}] }}, \"pattern\": {{ \"kind\": {}, \"color\": {} }} }}",
        page_on,
        page_measure,
        col_left,
        col_w,
        json_string(&crate::theme::margin_from().hex()),
        json_string(&crate::theme::margin_to().hex()),
        gd0,
        gd1,
        json_string(crate::theme::pattern().as_str()),
        json_string(&crate::theme::pattern_color().hex()),
    );
    // FOCUS MODE block: the active granularity + the active-unit char range the
    // capture rendered at full ink (the rest dimmed). `active_start`/`active_end` are
    // `null` when focus is Off, so a plain capture keeps a stable shape. Added in the
    // `/7`->`/8` (plain) and `/8`->`/9` (timeline) schema bump.
    let (focus_mode, focus_range) = pipeline.focus_report();
    // MARKDOWN STYLING block: the styled spans the capture rendered, as
    // `[start_byte, end_byte, "tag"]` over the document text. Additive + always
    // present (an empty array for a non-markdown buffer), so the schema revs in
    // lockstep with the focus block. Deterministic (pure function of the text).
    let md_spans_json = {
        let spans = pipeline.md_report();
        let body = spans
            .iter()
            .map(|(s, e, tag)| format!("[{}, {}, {}]", s, e, json_string(tag)))
            .collect::<Vec<_>>()
            .join(", ");
        format!("[{}]", body)
    };
    // SYNTAX HIGHLIGHTING block: the syntax role spans the capture rendered, as
    // `[start_byte, end_byte, "tag"]` over the document text (tag is one of
    // `comment`/`string`/`constant`/`definition`). Additive + always present (an
    // empty array for a non-code buffer), so the schema revs in lockstep with the
    // md_spans block. Deterministic (pure function of the text + language).
    let syn_spans_json = {
        let spans = pipeline.syn_report();
        let body = spans
            .iter()
            .map(|(s, e, tag)| format!("[{}, {}, {}]", s, e, json_string(tag)))
            .collect::<Vec<_>>()
            .join(", ");
        format!("[{}]", body)
    };
    // QUIET READOUT block: the word count + reading-time minutes the bottom-right
    // readout shows. `null` when nothing is drawn (a non-markdown or wordless
    // buffer), so a plain capture keeps a stable shape. Pure function of the text.
    let readout_json = match pipeline.readout_report() {
        Some((words, reading_min)) => {
            format!("{{ \"words\": {words}, \"reading_min\": {reading_min} }}")
        }
        None => "null".to_string(),
    };
    // DEBUG FRAME COUNTER block: `enabled` is the opt-in toggle state, and `text`
    // is what the corner readout draws — empty (off => byte-identical capture) or
    // the FIXED clockless placeholder (`--fps` / `--keys "C-x r"` => deterministic).
    // The capture has no clock, so a live number never appears here.
    let fps_json = format!(
        "{{ \"enabled\": {}, \"text\": {} }}",
        crate::fps::fps_on(),
        json_string(&pipeline.fps_text()),
    );
    let focus_json = match focus_range {
        Some((s, e)) => format!(
            "{{ \"mode\": {}, \"active_start\": {}, \"active_end\": {} }}",
            json_string(focus_mode),
            s,
            e
        ),
        None => format!(
            "{{ \"mode\": {}, \"active_start\": null, \"active_end\": null }}",
            json_string(focus_mode)
        ),
    };
    // Per-step caret block: present ONLY in a timeline/held frame. The schemas rev
    // in lockstep across the three shapes (see the SCHEMA_* constants): the plain
    // `--screenshot` path is [`SCHEMA_PLAIN`] (caret `None`), the `--capture-timeline`
    // path [`SCHEMA_TIMELINE`] (caret `Some` with the cosmetic-pop `pop_scale` +
    // drawn `block`, no `trail`), and the `--capture-held` path [`SCHEMA_HELD`]
    // (caret `Some` WITH the pop AND a `trail` block), keeping the three sidecar
    // shapes distinct.
    let (schema, caret_extra) = match caret {
        Some(c) => {
            // Optional `trail` sub-block: the drawn POSITION streak geometry for a held
            // step, present only on the held path ([`SCHEMA_HELD`]). The
            // `cosmetic_trail` block (with the streak's `sweep` progress) is emitted on
            // BOTH the timeline and held paths.
            let (schema, trail_extra) = match &c.trail {
                Some(tr) => (
                    SCHEMA_HELD,
                    format!(
                        ", \"trail\": {{ \"holding\": {h}, \"length\": {len}, \"tail\": {{ \"x\": {tlx}, \"y\": {tly} }}, \"head\": {{ \"x\": {hdx}, \"y\": {hdy} }} }}",
                        h = tr.holding,
                        len = tr.length,
                        tlx = tr.tail.0,
                        tly = tr.tail.1,
                        hdx = tr.head.0,
                        hdy = tr.head.1,
                    ),
                ),
                None => (SCHEMA_TIMELINE, String::new()),
            };
            // The COSMETIC | TRAIL block, present on BOTH the timeline and held paths.
            let co = &c.cosmetic;
            let cosmetic_extra = format!(
                ", \"cosmetic_trail\": {{ \"present\": {pr}, \"length\": {len}, \"direction\": {dir}, \"held\": {hd}, \"alpha\": {al}, \"sweep\": {sw}, \"tail\": {{ \"x\": {tlx}, \"y\": {tly} }}, \"head\": {{ \"x\": {hdx}, \"y\": {hdy} }} }}",
                pr = co.present,
                len = co.length,
                dir = json_string(if co.vertical { "vertical" } else { "horizontal" }),
                hd = co.held,
                al = co.alpha,
                sw = co.sweep,
                tlx = co.tail.0,
                tly = co.tail.1,
                hdx = co.head.0,
                hdy = co.head.1,
            );
            (
                schema,
                format!(
                    ",\n  \"caret\": {{ \"t_ms\": {t}, \"pos\": {{ \"x\": {px}, \"y\": {py} }}, \"target\": {{ \"x\": {tx}, \"y\": {ty} }}, \"settle_factor\": {sf}, \"animating\": {an}, \"pop_scale\": {ps}, \"block\": {{ \"w\": {bw}, \"h\": {bh} }}{trail_extra}{cosmetic_extra} }}",
                    t = c.t_ms,
                    px = c.pos.0,
                    py = c.pos.1,
                    tx = c.target.0,
                    ty = c.target.1,
                    sf = c.settle,
                    an = c.animating,
                    ps = c.scale,
                    bw = c.block_w,
                    bh = c.block_h,
                    trail_extra = trail_extra,
                    cosmetic_extra = cosmetic_extra,
                ),
            )
        }
        None => (SCHEMA_PLAIN, String::new()),
    };
    let json = format!(
        "{{\n  \"schema\": {schema_json},\n  \"canvas\": {canvas},\n  \"font\": {{ \"family\": {ff}, \"size\": {fs}, \"line_height\": {lh} }},\n  \"theme\": {{ \"name\": {tn}, \"font_family\": {tf}, \"mode\": {tm}, \"base100\": {tb100}, \"primary\": {tp} }},\n  \"caret_mode\": {cm},\n  \"text_origin\": {{ \"left\": {left}, \"top\": {top} }},\n  \"page\": {page},\n  \"focus\": {focus},\n  \"md_spans\": {md_spans},\n  \"syn_spans\": {syn_spans},\n  \"readout\": {readout},\n  \"fps\": {fps},\n  \"line_count\": {lc},\n  \"scroll_lines\": {sl},\n  \"cursor\": {{ \"line\": {cl}, \"col\": {cc} }},\n  \"selection\": {sel},\n  \"text\": {text_json},\n  \"first_lines\": [{fl}],\n  \"search\": {{ \"query\": {sq}, \"active\": {sa}, \"case_sensitive\": {scs}, \"hit_count\": {hc}, \"current\": {cur}, \"replace_active\": {ra}, \"replacement\": {rep} }},\n  \"project\": {project},\n  \"overlay\": {overlay}{caret_extra}\n}}\n",
        schema_json = json_string(schema),
        caret_extra = caret_extra,
        fps = fps_json,
        focus = focus_json,
        md_spans = md_spans_json,
        syn_spans = syn_spans_json,
        readout = readout_json,
        canvas = canvas_json,
        ff = json_string(active.font),
        fs = render::FONT_SIZE,
        lh = render::LINE_HEIGHT,
        tn = json_string(active.name),
        tf = json_string(active.font),
        tm = json_string(if active.dark { "dark" } else { "light" }),
        tb100 = json_string(&active.base_100.hex()),
        tp = json_string(&active.primary.hex()),
        cm = json_string(caret_mode),
        left = pipeline.text_left(),
        top = render::TEXT_TOP,
        page = page_json,
        lc = pipeline.line_count(),
        sl = view.scroll_lines,
        cl = cursor_line,
        cc = cursor_col,
        sel = selection_json,
        text_json = json_string(text),
        fl = first_lines_json,
        sq = json_string(&view.search_query),
        sa = view.search_active,
        scs = view.search_case_sensitive,
        hc = view.search_matches.len(),
        cur = search_cur,
        ra = view.search_replace_active,
        rep = json_string(&view.search_replacement),
        project = project_json,
        overlay = overlay_json,
    );

    let mut f = std::fs::File::create(&json_path)
        .with_context(|| format!("failed to create {}", json_path.display()))?;
    f.write_all(json.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caret::CaretAnim;

    #[test]
    fn json_string_escapes_quote_backslash_newline_and_control() {
        // Every sidecar string field flows through json_string; this is its only
        // direct test (the schema test that exercises it is GPU-gated, so on a
        // headless box the JSON contract is otherwise untested).
        assert_eq!(json_string("a\"b\\c\n\t"), "\"a\\\"b\\\\c\\n\\t\"");
        // A control char below 0x20 becomes a \uXXXX escape (0x01 -> ).
        assert_eq!(json_string("\u{01}"), "\"\\u0001\"");
        // Carriage return + tab are their short escapes.
        assert_eq!(json_string("\r\t"), "\"\\r\\t\"");
        // Round-trip a tricky string back through a real JSON parser: the escaped
        // literal must parse to exactly the original bytes.
        let tricky = "path \"with\" \\slashes\\ and\n\tcontrol\u{01}\u{1f}";
        let parsed: String = serde_json::from_str(&json_string(tricky))
            .expect("json_string output must be valid JSON");
        assert_eq!(parsed, tricky);
    }

    #[test]
    fn step_held_advances_and_clamps() {
        // line lengths: line 0 = 5 chars, line 1 = 2 chars, line 2 = 8 chars.
        let lens = [5usize, 2, 8];
        let last = 2;
        // RIGHT advances one char, then clamps at the line end.
        assert_eq!(step_held((0, 3), HeldDir::Right, &lens, last), (0, 4));
        assert_eq!(step_held((0, 5), HeldDir::Right, &lens, last), (0, 5));
        // LEFT decrements, saturating at column 0.
        assert_eq!(step_held((0, 1), HeldDir::Left, &lens, last), (0, 0));
        assert_eq!(step_held((0, 0), HeldDir::Left, &lens, last), (0, 0));
        // DOWN advances a line and pins the column to the shorter dest line.
        assert_eq!(step_held((0, 4), HeldDir::Down, &lens, last), (1, 2));
        assert_eq!(step_held((2, 8), HeldDir::Down, &lens, last), (2, 8)); // clamp at last line
        // UP retreats a line and clamps the column to that line's length.
        assert_eq!(step_held((2, 7), HeldDir::Up, &lens, last), (1, 2));
        assert_eq!(step_held((0, 3), HeldDir::Up, &lens, last), (0, 3)); // saturate at line 0
    }

    /// Re-derive the DRAWN streak length (px) for the caret's current spring state
    /// through the exact production path (`streak_length` → `motion_geometry`),
    /// mirroring the renderer's `caret_geometry`/`caret_trail_report`.
    fn drawn_streak_len(a: &CaretAnim, m: &render::Metrics) -> f32 {
        let speed = (a.vel.x * a.vel.x + a.vel.y * a.vel.y).sqrt();
        let streak_len = a.streak_length(
            m.streak_len_for_speed(speed),
            m.caret_streak_max_len,
            m.caret_held_len,
        );
        let (_c, half_along, _half_across, _axis) = a.motion_geometry(
            m.caret_w,
            m.caret_block_h,
            m.caret_streak_h,
            streak_len,
            m.caret_streak_gap,
            m.caret_trail_drop,
        );
        half_along * 2.0
    }

    /// Drive the SAME deterministic re-targeting the held-capture harness uses
    /// (`step_held` one char/line per virtual-clock step, `held=true`), and assert
    /// the DRAWN trail across the sustained held run is (a) always clear of the gap
    /// (never flickering out) AND (b) STEADY — a low-variance, near-constant length,
    /// not the per-repeat pulse the instantaneous-velocity length used to draw. This
    /// is the harness-level guarantee a human reads off the per-step sidecar
    /// `caret.trail.length`.
    fn held_run_keeps_steady_streak(dir: HeldDir, lens: &[usize], origin: (usize, usize)) {
        let m = render::Metrics::new(1.0);
        let adv = m.char_width;
        let lh = m.line_height;
        let gap = m.caret_streak_gap;
        let last = lens.len() - 1;
        // Cumulative-ms steps like the smoke run (0,30,60,...,210): one held
        // re-target + one injected-dt advance per entry.
        let steps: [u32; 8] = [0, 30, 60, 90, 120, 150, 180, 210];

        let mut a = CaretAnim::new();
        a.set_glyph_advance(adv);
        a.set_line_height(lh);
        // Prime AT REST on the origin (the initial press).
        let to_px = |(l, c): (usize, usize)| (c as f32 * adv + 100.0, l as f32 * lh + 100.0);
        let (ox, oy) = to_px(origin);
        a.set_target(ox, oy);
        a.snap_to_target();

        let mut cur = origin;
        let mut prev_ms = 0u32;
        let mut lengths: Vec<f32> = Vec::new();
        for (i, &t_ms) in steps.iter().enumerate() {
            cur = step_held(cur, dir, lens, last);
            let (x, y) = to_px(cur);
            a.set_held(true);
            a.set_target(x, y);
            let dt = (t_ms.saturating_sub(prev_ms)) as f32 / 1000.0;
            prev_ms = t_ms;
            a.step(dt);
            // Skip the dt=0 priming entry (step 0): no time advanced, so the spring
            // has not yet lagged. From the first real advance on, the trail must be
            // present + steady every step.
            if i >= 1 {
                assert!(a.is_holding(), "held run must stay latched at step {i}");
                lengths.push(drawn_streak_len(&a, &m));
            }
        }
        assert!(!lengths.is_empty());
        // (a) every held step clears the gap — the streak never flickers out.
        for (k, &len) in lengths.iter().enumerate() {
            assert!(
                len > gap,
                "held {:?} step {k} streak {len} must clear the gap {gap}",
                dir as u8
            );
        }
        // (b) the held trail is STEADY: the spread across the run is a small
        // fraction of the mean, not the per-repeat pulse (~13px on ~29px) it was.
        let mean = lengths.iter().sum::<f32>() / lengths.len() as f32;
        let max = lengths.iter().cloned().fold(f32::MIN, f32::max);
        let min = lengths.iter().cloned().fold(f32::MAX, f32::min);
        assert!(
            (max - min) <= 0.10 * mean,
            "held {:?} streak must be steady: spread {} ({min}..{max}) exceeds 10% of mean {mean}",
            dir as u8,
            max - min
        );
    }

    #[test]
    fn held_right_run_streak_steady_over_gap() {
        // A long line so RIGHT never clamps mid-run.
        held_run_keeps_steady_streak(HeldDir::Right, &[40, 40, 40, 40, 40, 40, 40], (3, 5));
    }

    #[test]
    fn held_down_run_streak_steady_over_gap() {
        // Enough lines (all wide) so DOWN advances a real line each step.
        held_run_keeps_steady_streak(HeldDir::Down, &[20; 12], (0, 5));
    }

    /// True when a wgpu adapter is present, so the GPU-dependent capture tests can
    /// skip gracefully on a headless/CI box (mirrors `render::tests::headless_pipeline`).
    fn adapter_available() -> bool {
        pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .is_ok()
        })
    }

    /// Extract the integer/float that follows `"key":` AFTER the first occurrence of
    /// `anchor` in the sidecar JSON. Scoped by `anchor` so `page.column.left` /
    /// `canvas.width` don't collide with same-named keys elsewhere.
    fn num_after(json: &str, anchor: &str, key: &str) -> f64 {
        let from = json.find(anchor).expect("anchor present");
        let rest = &json[from..];
        let kpos = rest.find(key).expect("key present after anchor");
        let after = &rest[kpos + key.len()..];
        // Skip `": ` and read the leading numeric token.
        let token: String = after
            .chars()
            .skip_while(|c| !(c.is_ascii_digit() || *c == '-' || *c == '+'))
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
            .collect();
        token.parse().unwrap_or_else(|_| panic!("bad number for {key:?}: {token:?}"))
    }

    /// The harness now reproduces the margin-class geometry: a capture at a REAL
    /// retina size (2400x1600 @ dpi 2.0) yields a page column CENTERED with a margin
    /// on BOTH sides (left == right within rounding, both > 0) — the assertion the old
    /// hardcoded 1200/dpi-1 capture could never make. And the DEFAULT (no size/dpi)
    /// column geometry is byte-for-byte unchanged (left=120, width=960 at 1200).
    #[test]
    fn retina_capture_centers_page_column_symmetrically() {
        if !adapter_available() {
            eprintln!("skipping retina_capture_centers_page_column_symmetrically: no wgpu adapter");
            return;
        }
        // Page globals are process-wide; serialize with every other page/render test.
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let dir = std::env::temp_dir().join(format!("awl_capture_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let buf = Buffer::from_str("the quick brown fox jumps over the lazy dog\nsecond line of prose here\nand a third line to fill the page");

        // --- RETINA run: 2400x1600 @ dpi 2.0, narrow column so margins are real. ---
        crate::page::set_page_on(true);
        crate::page::set_measure(40);
        let retina_png = dir.join("retina.png");
        let opts = CaptureOpts {
            canvas: Some((2400, 1600)),
            dpi: Some(2.0),
            ..CaptureOpts::default()
        };
        capture_with(&retina_png, &buf, &opts).expect("retina capture");
        let json = std::fs::read_to_string(retina_png.with_extension("json")).unwrap();
        let cw = num_after(&json, "\"canvas\":", "\"width\"");
        let dpi = num_after(&json, "\"canvas\":", "\"dpi\"");
        let left = num_after(&json, "\"column\":", "\"left\"");
        let width = num_after(&json, "\"column\":", "\"width\"");
        assert_eq!(cw, 2400.0, "sidecar canvas.width self-describes the physical size");
        assert_eq!(dpi, 2.0, "sidecar canvas.dpi self-describes the scale factor");
        let right = 2400.0 - (left + width);
        assert!(left > 0.0, "retina page column needs a LEFT margin, got {left}");
        assert!(right > 0.0, "retina page column needs a RIGHT margin, got {right}");
        assert!(
            (left - right).abs() <= 1.0,
            "retina page column must be CENTERED: left {left} vs right {right}"
        );

        // --- DEFAULT run: no size/dpi flags -> unchanged 1200/dpi-1 geometry. ---
        crate::page::set_measure(80);
        let def_png = dir.join("default.png");
        capture_with(&def_png, &buf, &CaptureOpts::default()).expect("default capture");
        let djson = std::fs::read_to_string(def_png.with_extension("json")).unwrap();
        let dleft = num_after(&djson, "\"column\":", "\"left\"");
        let dwidth = num_after(&djson, "\"column\":", "\"width\"");
        assert!(
            (dleft - 120.0).abs() <= 0.5 && (dwidth - 960.0).abs() <= 0.5,
            "default column geometry must be unchanged (left 120, width 960), got left {dleft} width {dwidth}"
        );
        // The no-flag sidecar must NOT carry a dpi key (byte-stable canvas block).
        let canvas_block = &djson[djson.find("\"canvas\":").unwrap()..djson.find("\"font\":").unwrap()];
        assert!(
            !canvas_block.contains("\"dpi\""),
            "no-flag sidecar canvas block must omit dpi for byte-identity: {canvas_block:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// CONTRACT LOCK: the hand-rolled sidecar must be WELL-FORMED JSON (a real
    /// parser, not the substring scanners the other tests use, would catch a stray
    /// comma / unescaped value / duplicate key) AND carry the right SCHEMA + the
    /// blocks the whole verification path depends on. Covers all three shapes:
    /// plain (`SCHEMA_PLAIN`, no caret block), timeline (`SCHEMA_TIMELINE`, caret
    /// without `trail`), held (`SCHEMA_HELD`, caret WITH `trail`).
    #[test]
    fn sidecar_is_wellformed_json_with_expected_schema() {
        if !adapter_available() {
            eprintln!("skipping sidecar_is_wellformed_json_with_expected_schema: no wgpu adapter");
            return;
        }
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("awl_json_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut buf =
            Buffer::from_str("# Title\n\nsome **bold** prose to fill a line\nsecond line\n");
        buf.set_path(dir.join("doc.md")); // .md so md_spans populate

        // --- PLAIN single frame -----------------------------------------------
        let png = dir.join("plain.png");
        capture_with(&png, &buf, &CaptureOpts::default()).expect("plain capture");
        let text = std::fs::read_to_string(png.with_extension("json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("plain sidecar is not valid JSON: {e}\n{text}"));
        let obj = v.as_object().expect("sidecar root is a JSON object");
        assert_eq!(obj["schema"], serde_json::json!(SCHEMA_PLAIN), "plain schema");
        // The blocks the agent contract reads, present + the right JSON shape.
        for key in [
            "canvas", "font", "theme", "caret_mode", "page", "focus", "md_spans",
            "syn_spans", "readout", "fps", "cursor", "selection", "search", "project",
            "overlay",
        ] {
            assert!(obj.contains_key(key), "plain sidecar missing {key:?}");
        }
        assert!(obj["md_spans"].is_array(), "md_spans is an array");
        assert!(!obj["md_spans"].as_array().unwrap().is_empty(), "markdown buffer has md spans");
        assert!(obj["page"].is_object() && obj["focus"].is_object(), "page + focus are objects");
        assert!(obj["cursor"].is_object(), "cursor is an object");
        // project / overlay are an object when present, JSON null when absent.
        assert!(obj["project"].is_object() || obj["project"].is_null());
        assert!(obj["overlay"].is_object() || obj["overlay"].is_null());
        // A PLAIN frame carries NO caret block (that is the timeline/held shape).
        assert!(!obj.contains_key("caret"), "plain frame must omit the caret block");

        // --- TIMELINE frame (caret block, no trail) ---------------------------
        let tl = dir.join("tl.png");
        capture_timeline(&tl, &buf, (0, 0), &[0, 30], &CaptureOpts::default()).expect("timeline");
        let ttext = std::fs::read_to_string(dir.join("tl.t0.json")).unwrap();
        let tv: serde_json::Value = serde_json::from_str(&ttext)
            .unwrap_or_else(|e| panic!("timeline sidecar is not valid JSON: {e}\n{ttext}"));
        assert_eq!(tv["schema"], serde_json::json!(SCHEMA_TIMELINE), "timeline schema");
        assert!(tv.get("caret").is_some(), "timeline carries a caret block");
        assert!(tv["caret"].get("trail").is_none(), "timeline caret has no trail block");
        assert!(tv["caret"].get("cosmetic_trail").is_some(), "timeline caret has cosmetic_trail");

        // --- HELD frame (caret block WITH trail) ------------------------------
        let hd = dir.join("hd.png");
        capture_held(&hd, &buf, (0, 0), HeldDir::Down, &[0, 30], &CaptureOpts::default())
            .expect("held");
        let htext = std::fs::read_to_string(dir.join("hd.t30.json")).unwrap();
        let hv: serde_json::Value = serde_json::from_str(&htext)
            .unwrap_or_else(|e| panic!("held sidecar is not valid JSON: {e}\n{htext}"));
        assert_eq!(hv["schema"], serde_json::json!(SCHEMA_HELD), "held schema");
        assert!(hv["caret"].get("trail").is_some(), "held caret carries a trail block");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SYNTAX HIGHLIGHTING regression: the capture sidecar's `syn_spans` block is
    /// populated for a recognized CODE buffer but EMPTY for a markdown / plain-text
    /// buffer — so a `.md` / `.txt` capture stays byte-identical (the gate in
    /// `Buffer::syntax_lang`). Also confirms the schema bumped to `/30`.
    #[test]
    fn syntax_sidecar_gated_to_code() {
        if !adapter_available() {
            eprintln!("skipping syntax_sidecar_gated_to_code: no wgpu adapter");
            return;
        }
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("awl_syn_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // A Rust buffer: syn_spans must carry a "comment" role span.
        let mut code = Buffer::from_str("// hi\nfn main() {}\n");
        code.set_path(dir.join("main.rs"));
        let code_png = dir.join("code.png");
        capture_with(&code_png, &code, &CaptureOpts::default()).expect("code capture");
        let cjson = std::fs::read_to_string(code_png.with_extension("json")).unwrap();
        assert!(cjson.contains("\"schema\": \"awl-capture/30\""), "schema bumped: {cjson:.80}");
        let syn = &cjson[cjson.find("\"syn_spans\":").unwrap()..];
        assert!(syn.contains("\"comment\""), "code syn_spans must carry a comment: {syn:.120}");
        assert!(syn.contains("\"definition\""), "code syn_spans must carry the fn name: {syn:.120}");

        // A markdown buffer: syn_spans must be the empty array (no code highlight).
        let mut md = Buffer::from_str("# title\nsome prose\n");
        md.set_path(dir.join("notes.md"));
        let md_png = dir.join("notes.png");
        capture_with(&md_png, &md, &CaptureOpts::default()).expect("md capture");
        let mjson = std::fs::read_to_string(md_png.with_extension("json")).unwrap();
        assert!(mjson.contains("\"syn_spans\": []"), "markdown must emit empty syn_spans");

        // A plain-text buffer: syn_spans empty too.
        let mut txt = Buffer::from_str("just words\n");
        txt.set_path(dir.join("scratch.txt"));
        let txt_png = dir.join("scratch.png");
        capture_with(&txt_png, &txt, &CaptureOpts::default()).expect("txt capture");
        let tjson = std::fs::read_to_string(txt_png.with_extension("json")).unwrap();
        assert!(tjson.contains("\"syn_spans\": []"), ".txt must emit empty syn_spans");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEBUG FRAME COUNTER: the counter is ABSENT from a default capture (empty
    /// readout, `enabled=false`, so the frame is byte-identical), and the `--fps`
    /// toggle flips its state — drawing a FIXED, clockless placeholder. The
    /// assertions read the deterministic SIDECAR (`text` is exactly what is drawn)
    /// rather than racing raw PNG bytes against concurrent global-mutating tests;
    /// the placeholder's byte-determinism is covered by `fps::tests`.
    #[test]
    fn fps_counter_absent_by_default_and_toggles() {
        if !adapter_available() {
            eprintln!("skipping fps_counter_absent_by_default_and_toggles: no wgpu adapter");
            return;
        }
        // Lock BOTH globals the capture folds in (page geometry + the fps flag) so
        // this never races a page/fps test in another thread.
        let _pg = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _fg = crate::fps::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("awl_fps_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let buf = Buffer::from_str("hello frame counter\n");

        // DEFAULT (counter OFF): absent — empty readout text + enabled=false, so the
        // capture path draws nothing (byte-identical to a pre-feature capture).
        crate::fps::set_fps_on(false);
        let off_png = dir.join("off.png");
        capture_with(&off_png, &buf, &CaptureOpts::default()).expect("off capture");
        let off_json = std::fs::read_to_string(off_png.with_extension("json")).unwrap();
        assert!(
            off_json.contains("\"fps\": { \"enabled\": false, \"text\": \"\" }"),
            "default capture: counter absent: {off_json}"
        );

        // ENABLED (`--fps` / `C-x r`): the toggle flips state — the readout shows the
        // fixed clockless placeholder (no live number) and enabled=true.
        crate::fps::set_fps_on(true);
        let on_png = dir.join("on.png");
        capture_with(&on_png, &buf, &CaptureOpts::default()).expect("on capture");
        let on_json = std::fs::read_to_string(on_png.with_extension("json")).unwrap();
        assert!(
            on_json.contains("\"fps\": { \"enabled\": true, \"text\": \"fps · — ms\" }"),
            "enabled capture: fixed placeholder + enabled=true: {on_json}"
        );

        // Restore the default so later tests see the counter off.
        crate::fps::set_fps_on(false);
        let _ = std::fs::remove_dir_all(&dir);
    }
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
