//! Per-tier bench CONTEXT for the `--bench-suite` scenarios: one pipeline +
//! offscreen target + the corpus text, shaped once and warmed, plus the two
//! per-step primitives every scenario shares — the live-shaped FRAME (the
//! exact `RedrawRequested` aggregate, GPU serialized by a blocking poll, the
//! same shape [`super::super::framebench`]'s zoom profiler replays) and the
//! pixel SNAPSHOT (the capture harness's own `read_frame` readback, feeding
//! the outcome witnesses). Split out of [`super::scenarios`] purely for the
//! ~500-line file ceiling; the seams are unchanged.

use anyhow::{Context as _, Result};

use crate::buffer::Buffer;
use crate::clock::Instant;
use crate::config::Config;

use super::corpus::{self, Tier};
use super::{DPI, DT, HEIGHT, WIDTH};
use crate::render::{TextPipeline, ViewState};

/// Per-tier bench context: one pipeline + offscreen target + the corpus text,
/// shaped once and warmed before the scenarios run.
pub(super) struct Cx<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub config: &'a Config,
    pub p: TextPipeline,
    pub texture: wgpu::Texture,
    pub tview: wgpu::TextureView,
    pub text: String,
    /// A `Buffer` over the same text — the char<->line/col oracle the search
    /// scenario needs (the same conversion the capture harness performs).
    pub buffer: Buffer,
    pub view: ViewState,
    pub lines: usize,
    pub words: u64,
    /// The canvas width currently fed to `prepare` (the resize scenario steps it).
    pub width: u32,
}

impl<'a> Cx<'a> {
    pub(super) fn new(
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        cache: &glyphon::Cache,
        config: &'a Config,
        tier: Tier,
        text: String,
        misspelled: Vec<crate::spell::Misspelling>,
    ) -> Result<Self> {
        let mut p = TextPipeline::new(device, queue, cache, crate::capture::FORMAT);
        p.set_size(WIDTH as f32, HEIGHT as f32);
        p.set_dpi(DPI);
        let buffer = Buffer::from_str(&text);
        let lines = text.lines().count();
        let words = corpus::count_words(&text);
        let view = ViewState {
            text: text.clone(),
            misspelled,
            gutter_name: tier.doc_name().to_string(),
            gutter_project: "bench-suite".to_string(),
            is_markdown: tier.is_markdown(),
            syn_lang: tier.syn_lang(),
            ..ViewState::base()
        };
        p.set_view(&view);
        let (texture, tview) = crate::capture::gpu::offscreen_target(device, WIDTH, HEIGHT);
        let mut cx = Cx {
            device,
            queue,
            config,
            p,
            texture,
            tview,
            text,
            buffer,
            view,
            lines,
            words,
            width: WIDTH,
        };
        // Warm the atlas + caches like an editor sitting on the open document.
        for _ in 0..3 {
            cx.frame()?;
        }
        Ok(cx)
    }

    /// One live-shaped frame at the CURRENT canvas width: the exact
    /// `RedrawRequested` aggregate, GPU serialized by the blocking poll.
    pub(super) fn frame(&mut self) -> Result<()> {
        self.p.advance(DT);
        self.p.prepare(self.device, self.queue, self.width, HEIGHT)?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("awl bench suite encoder"),
            });
        self.p.render(&mut encoder, &self.tview)?;
        self.queue.submit(Some(encoder.finish()));
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .context("device poll failed")?;
        self.p.atlas.trim();
        Ok(())
    }

    /// Push the working view and draw one frame (the per-step unit most
    /// scenarios time).
    pub(super) fn sync_frame(&mut self) -> Result<()> {
        self.p.set_view(&self.view);
        self.frame()
    }

    /// Read the offscreen target back for a pixel witness.
    pub(super) fn snapshot(&mut self) -> Result<image::RgbaImage> {
        crate::capture::gpu::read_frame(self.device, self.queue, &self.texture, WIDTH, HEIGHT)
    }
}

/// Count differing pixels between two equally-sized snapshots.
pub(super) fn differing_pixels(a: &image::RgbaImage, b: &image::RgbaImage) -> u64 {
    a.pixels().zip(b.pixels()).filter(|(x, y)| x != y).count() as u64
}

/// Elapsed milliseconds since `t0` (the per-sample unit).
pub(super) fn ms(t0: Instant) -> f64 {
    t0.elapsed().as_secs_f64() * 1e3
}
