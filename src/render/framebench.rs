//! FRAME PROFILER (hidden `--bench-frame` flag) — per-stage timing of the EXACT
//! live redraw sequence over the REAL repo docs, at the live-report canvas.
//!
//! The live window's hot loop (`RedrawRequested` in `app.rs`, kept hot by the
//! debug panel) runs, per frame: `pipeline.advance(dt)` → `pipeline.prepare(..)`
//! → encode `pipeline.render` → `queue.submit` → present → `atlas.trim()`. This
//! harness replays that sequence headlessly — an offscreen color target stands
//! in for the swapchain frame, and a blocking `device.poll` after submit stands
//! in for present, so the GPU-side cost is SERIALIZED into the number rather
//! than overlapped — and times EACH prepare sub-call in the same order
//! [`TextPipeline::prepare`] makes them: the chrome aggregate split into its
//! five sub-preparations, the spell / nit underline layers split into rect
//! BUILD vs GPU upload. A child module of `render` (like [`super::perfbench`])
//! so it reaches the `pub(super)` per-layer prepares and private fields
//! directly, no public shims.
//!
//! The `ViewState` is built the way the LIVE `App::sync_view` builds one —
//! including `misspelled` from the real bundled-dictionary scan
//! (`SpellChecker::misspellings(&text)`, the exact path `app/viewstate.rs`
//! caches into `spell_cache`), so the docs carry their true squiggle load —
//! and the canvas mirrors the user readout: 2910x1720 PHYSICAL pixels at dpi
//! 2.0 (`set_size` then `set_dpi`, the same order `App` wires them), debug
//! panel ON and fed a live EMA each frame. `set_view` is timed separately
//! because the live loop does NOT run it per frame — `sync_view` runs per
//! input EVENT; `RedrawRequested` never calls it.

use anyhow::Context as _;
use glyphon::{Cache, Resolution};
use std::path::Path;

use crate::buffer::Buffer;
use crate::capture::FORMAT;
use crate::clock::Instant;

use super::{TextPipeline, ViewState};

/// The user-report canvas: 2910x1720 physical pixels on a @2x display.
const WIDTH: u32 = 2910;
const HEIGHT: u32 = 1720;
const DPI: f32 = 2.0;
/// Untimed settle frames before sampling (atlas fills, caret spring settles).
const WARMUP: usize = 30;
/// Timed hot frames per document.
const FRAMES: usize = 300;
/// The dt a steady 60fps live loop feeds `advance`.
const DT: f32 = 1.0 / 60.0;

/// The per-frame stages, in the EXACT order the `mark()` calls are taken in
/// [`profile_doc`] — i.e. the order [`TextPipeline::prepare`] makes its
/// sub-calls, then the encode/submit/trim tail `Gpu::redraw` runs. Keep this
/// list and the marks in lockstep (asserted per frame).
const STAGE_NAMES: [&str; 22] = [
    "advance (spring step)",
    "sync_wrap_width",
    "viewport.update (uniforms)",
    "background layer",
    "text layer (glyphon prepare)",
    "caret layer (geom + upload)",
    "selection/search rects",
    "ornaments (rules + bullets)",
    "chrome: caret-preview panel",
    "chrome: overlay/panel park",
    "chrome: gutter",
    "chrome: debug panel",
    "chrome: stats HUD (parked)",
    "chrome: which-key (parked)",
    "spell: squiggle rect build",
    "spell: underline upload",
    "nits: rect build (line scan)",
    "nits: underline upload",
    "blur (inactive)",
    "render encode (all draws)",
    "queue.submit + device.poll",
    "atlas.trim",
];

/// Consecutive-segment stopwatch: `begin` at the frame top, `mark()` after each
/// stage. Segments are back-to-back (each mark restarts the clock), so the
/// stage sum accounts for the whole frame with no untimed gaps.
struct Marks {
    t0: Instant,
    samples: Vec<Vec<u128>>,
    i: usize,
    timed: bool,
}

impl Marks {
    fn new(n: usize) -> Self {
        Self { t0: Instant::now(), samples: vec![Vec::new(); n], i: 0, timed: false }
    }
    fn begin(&mut self, timed: bool) {
        self.i = 0;
        self.timed = timed;
        self.t0 = Instant::now();
    }
    fn mark(&mut self) {
        let ns = self.t0.elapsed().as_nanos();
        if self.timed {
            self.samples[self.i].push(ns);
        }
        self.i += 1;
        self.t0 = Instant::now();
    }
}

fn median(mut v: Vec<u128>) -> u128 {
    v.sort_unstable();
    v[v.len() / 2]
}

/// A `ViewState` built the way the LIVE `App::sync_view` builds one for a calm
/// open-file frame: cursor at the origin, no selection / search / overlay, and
/// — the load-bearing part — `misspelled` populated by the SAME
/// `SpellChecker::misspellings(&text)` scan the live app caches into
/// `spell_cache` (see `app/viewstate.rs`), so every squiggle the user sees is
/// present. Mirrors `perfbench::bench_view` otherwise.
fn live_view(buffer: &Buffer, misspelled: Vec<crate::spell::Misspelling>) -> ViewState {
    ViewState {
        text: buffer.text(),
        cursor_line: 0,
        cursor_col: 0,
        scroll_lines: 0,
        zoom: 1.0,
        selection: None,
        preedit: String::new(),
        misspelled,
        is_edit_move: false,
        held: false,
        search_matches: Vec::new(),
        search_current: None,
        search_query: String::new(),
        search_active: false,
        search_case_sensitive: false,
        search_replace_active: false,
        search_replacement: String::new(),
        search_editing_replacement: false,
        overlay_active: false,
        overlay_crisp: false,
        overlay_query: String::new(),
        overlay_items: Vec::new(),
        overlay_bindings: Vec::new(),
        overlay_times: Vec::new(),
        overlay_selected: 0,
        overlay_scroll: 0,
        overlay_hint: String::new(),
        overlay_lens: Vec::new(),
        overlay_sections: Vec::new(),
        caret_preview: None,
        gutter_name: buffer.display_name(),
        gutter_project: "awl-next".to_string(),
        is_markdown: buffer.is_markdown(),
        syn_lang: buffer.syntax_lang(),
        overlay_spell: None,
    }
}

/// Run the frame profiler and print a per-stage table per document. One
/// headless wgpu device (offscreen, no window), reused across both docs.
pub fn run() -> anyhow::Result<()> {
    pollster::block_on(run_async())
}

async fn run_async() -> anyhow::Result<()> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .map_err(|e| anyhow::anyhow!("no wgpu adapter for frame bench: {e:?}"))?;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("awl frame bench device"),
            ..Default::default()
        })
        .await?;
    let cache = Cache::new(&device);

    // The live scenario under investigation keeps the redraw loop hot via the
    // DEBUG panel, so it is ON here and fed a live EMA each frame — its corner
    // label re-shapes per frame exactly as in the window.
    crate::debug::set_debug_on(true);

    let spell = crate::spell::SpellChecker::new()
        .map_err(|e| anyhow::anyhow!("spell checker failed to load: {e}"))?;

    println!(
        "frame profiler — {WIDTH}x{HEIGHT} @{DPI}x · debug panel ON · {WARMUP} warmup + {FRAMES} timed frames"
    );
    println!("(headless: submit+poll SERIALIZES the GPU cost; the window overlaps it and adds present/acquire)");
    for name in ["CAPTURE.md", "CLAUDE.md"] {
        profile_doc(&device, &queue, &cache, &spell, name)?;
    }
    Ok(())
}

/// Profile one document: build the live-shaped view (real misspellings), run
/// the warmup + timed frames of the exact redraw sequence, and print the
/// stage | median ms | % table plus the stage-sum sanity check and the two
/// per-EVENT / off-frame costs (`set_view`, the word-count readout scan).
fn profile_doc(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cache: &Cache,
    spell: &crate::spell::SpellChecker,
    name: &str,
) -> anyhow::Result<()> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
    let buffer = Buffer::from_file(&path);
    let text = buffer.text();
    let misspelled = spell.misspellings(&text);
    let lines = text.lines().count();

    let mut p = TextPipeline::new(device, queue, cache, FORMAT);
    // Mirror the live App's wiring order: the surface size first (physical
    // pixels, `Gpu::new`), then the display scale factor (`App::resumed`),
    // then the first view sync.
    p.set_size(WIDTH as f32, HEIGHT as f32);
    p.set_dpi(DPI);
    let view = live_view(&buffer, misspelled.clone());
    p.set_view(&view);

    // Offscreen color target standing in for the swapchain frame.
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("awl frame bench target"),
        size: wgpu::Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let mut marks = Marks::new(STAGE_NAMES.len());
    let mut totals: Vec<u128> = Vec::with_capacity(FRAMES);
    let mut ema: Option<f32> = None;

    for frame in 0..(WARMUP + FRAMES) {
        let timed = frame >= WARMUP;
        let ft0 = Instant::now();
        marks.begin(timed);

        // ---- the live RedrawRequested body --------------------------------
        p.advance(DT);
        p.set_debug_frame_ms(ema); // the live loop feeds the EMA each frame
        marks.mark();

        // ---- TextPipeline::prepare, sub-call by sub-call (same order) -----
        p.sync_wrap_width();
        marks.mark();
        p.viewport.update(queue, Resolution { width: WIDTH, height: HEIGHT });
        marks.mark();
        p.prepare_background_layer(queue, WIDTH, HEIGHT);
        marks.mark();
        p.prepare_text_layer(device, queue, WIDTH, HEIGHT)?;
        marks.mark();
        p.prepare_caret_layer(device, queue, WIDTH, HEIGHT);
        marks.mark();
        p.prepare_selection_layer(device, queue, WIDTH, HEIGHT);
        marks.mark();
        p.prepare_ornaments(device, queue, WIDTH, HEIGHT)?;
        marks.mark();
        // prepare_chrome_layer, split into its five sub-preparations:
        p.prepare_caret_preview_panel(device, queue, WIDTH, HEIGHT)?;
        marks.mark();
        // no overlay + no search -> the park branch (nothing lingers)
        p.panel_card.prepare(device, queue, WIDTH, HEIGHT, &[]);
        p.overlay_rows.prepare(device, queue, WIDTH, HEIGHT, &[]);
        marks.mark();
        p.prepare_gutter(device, queue, WIDTH, HEIGHT)?;
        marks.mark();
        p.prepare_debug(device, queue, WIDTH, HEIGHT)?;
        marks.mark();
        p.prepare_hud(device, queue, WIDTH, HEIGHT)?;
        marks.mark();
        p.prepare_whichkey(device, queue, WIDTH, HEIGHT)?;
        marks.mark();
        // prepare_spell_layer, split: rect building vs GPU upload
        let squiggles = p.spell_squiggles();
        marks.mark();
        p.spell_pipeline.prepare(device, queue, WIDTH, HEIGHT, &squiggles);
        marks.mark();
        // prepare_nit_layer, split the same way
        let nits = p.nit_underlines();
        marks.mark();
        p.nit_pipeline.prepare(device, queue, WIDTH, HEIGHT, &nits);
        marks.mark();
        p.prepare_blur(device, queue, WIDTH, HEIGHT);
        marks.mark();

        // ---- Gpu::redraw's tail: encode -> submit (+poll) -> trim ---------
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("awl frame bench encoder"),
        });
        p.render(&mut encoder, &target_view)?;
        let cmd = encoder.finish();
        marks.mark();
        queue.submit(Some(cmd));
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .context("device poll failed")?;
        marks.mark();
        p.atlas.trim();
        marks.mark();

        assert_eq!(marks.i, STAGE_NAMES.len(), "stage marks out of lockstep");
        let ns = ft0.elapsed().as_nanos();
        if timed {
            totals.push(ns);
        }
        let ms = ns as f32 / 1.0e6;
        ema = Some(ema.map_or(ms, |e| e * 0.9 + ms * 0.1));
    }

    // ---- report ------------------------------------------------------------
    let total_med = median(totals.clone());
    println!();
    println!(
        "==== {name}: {lines} lines · {} misspellings (live SpellChecker scan) ====",
        misspelled.len()
    );
    if let Some((words, mins)) = p.readout_report() {
        println!("     ({words} words · {mins} min read)");
    }
    println!("{:>29} | {:>10} | {:>10}", "stage", "median ms", "% of total");
    println!("{:->29}-+-{:->10}-+-{:->10}", "", "", "");
    let mut sum_med: u128 = 0;
    for (i, stage) in STAGE_NAMES.iter().enumerate() {
        let med = median(marks.samples[i].clone());
        sum_med += med;
        println!(
            "{:>29} | {:>10.3} | {:>9.1}%",
            stage,
            med as f64 / 1.0e6,
            med as f64 / total_med as f64 * 100.0
        );
    }
    println!("{:->29}-+-{:->10}-+-{:->10}", "", "", "");
    println!(
        "{:>29} | {:>10.3} | {:>9.1}%",
        "TOTAL (median frame)",
        total_med as f64 / 1.0e6,
        100.0
    );
    // Ballpark check: back-to-back marks mean the stage sum should account for
    // ~the whole measured frame; any sizable gap is unattributed work.
    let gap = total_med as i128 - sum_med as i128;
    println!(
        "{:>29} | {:>10.3} | gap {:+.3} ms ({:+.1}% of total)",
        "sum of stage medians",
        sum_med as f64 / 1.0e6,
        gap as f64 / 1.0e6,
        gap as f64 / total_med as f64 * 100.0
    );

    // ---- per-EVENT / off-frame costs, closing the suspects list -------------
    // set_view: the live loop runs this in `sync_view` per input EVENT —
    // RedrawRequested never calls it — so it is NOT part of the frame total.
    let mut sv = Vec::with_capacity(41);
    for _ in 0..41 {
        let t0 = Instant::now();
        p.set_view(&view);
        sv.push(t0.elapsed().as_nanos());
    }
    println!(
        "  set_view (per input EVENT — sync_view; NOT per frame): median {:.3} ms over {} calls",
        median(sv.clone()) as f64 / 1.0e6,
        sv.len()
    );
    // The markdown word-count readout scan: the persistent readout moved into
    // the held HUD, so this O(doc) scan runs only while the HUD is HELD (and
    // for the capture sidecar) — never in the hot loop. Timed to close it out.
    let mut ro = Vec::with_capacity(41);
    for _ in 0..41 {
        let t0 = Instant::now();
        std::hint::black_box(p.readout_report());
        ro.push(t0.elapsed().as_nanos());
    }
    println!(
        "  readout_report word-count scan (HUD-held/sidecar only — NOT per frame): median {:.3} ms",
        median(ro.clone()) as f64 / 1.0e6
    );
    Ok(())
}
