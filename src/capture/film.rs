//! src/capture/film.rs — the STORYBOARD FILM RENDERER: one persistent offscreen
//! pipeline stepped along a VIRTUAL clock at a fixed frame cadence.
//!
//! The third member of the deterministic multi-frame family (`animated.rs`'s
//! timeline / held drivers are the other two, and this module reuses their
//! exact loop shape): device / texture / pipeline built ONCE, then every step
//! folds the CURRENT replay state into the settled view (the single-frame
//! path's own [`super::modes::settled_viewstate`] — one owner, so a film frame
//! can never disagree with what `--screenshot` would show for the same state)
//! and advances the virtual clock in fixed [`FRAME_MS`] ticks, one film frame
//! per tick. The dt is INJECTED (`TextPipeline::advance`, the one virtual-clock
//! seam) — no real clock, no RNG — so the same storyboard yields byte-identical
//! frames on every run. This is deterministic VISUAL REVIEW of motion, not a
//! claim about real compositor cadence (CAPTURE.md's live-only boundary).
//!
//! Unlike the single-frame path the caret is NEVER settled here: the spring
//! keeps whatever pose the previous tick left, so a navigation press followed
//! by `run_for` films the real glide (`set_view` re-aims the spring exactly as
//! the live window's between-frames re-sync does).
//!
//! The film frames land in `<out>/frames/frame-NNNNN.png`; per-step artifacts
//! (`step-NNN.png` — a byte-copy of the step's last film frame — plus the
//! ordinary plain-schema sidecar) are written on request. Encoding frames into
//! a WebM/MP4 is the orchestrator's job (`crate::story`), not this module's —
//! the raw frames ARE the deterministic deliverable and are always retained.

use anyhow::{Context, Result};
use glyphon::Cache;
use std::path::{Path, PathBuf};

use crate::buffer::Buffer;
use crate::render::TextPipeline;

use super::gpu::{headless_device, offscreen_target, read_frame};
use super::modes::settled_viewstate;
use super::opts::CaptureOpts;
use super::sidecar::write_sidecar;
use super::{CANVAS_HEIGHT, CANVAS_WIDTH, FORMAT};

/// The virtual clock's fixed frame step: 20 ms/frame = 50 fps (a WebM-friendly
/// integer rate). Every emitted film frame advances the clock exactly this much
/// — a `pause`/`run_for` of N ms emits ceil(N/20) frames; a press/type step
/// emits ONE frame (one tick). The ONE owner of the cadence: the trace's
/// `frame_ms`, the frame count of a pause, and the encoder's `-framerate` all
/// derive from it.
pub const FRAME_MS: u32 = 20;

/// The persistent storyboard renderer: everything a frame needs, built once.
pub struct FilmRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    pipeline: TextPipeline,
    width: u32,
    height: u32,
    frames_dir: PathBuf,
    next_frame: u32,
}

impl FilmRenderer {
    /// Build the offscreen device + pipeline and create `<out_dir>/frames/`.
    /// Fails cleanly (never panics) on a GPU-less host — the storyboard runner
    /// treats that exactly like the strict replay's missing layout oracle.
    pub fn new(out_dir: &Path) -> Result<Self> {
        let (device, queue) = pollster::block_on(headless_device())?;
        let (width, height) = (CANVAS_WIDTH, CANVAS_HEIGHT);
        let (texture, view) = offscreen_target(&device, width, height);
        let cache = Cache::new(&device);
        let mut pipeline = TextPipeline::new(&device, &queue, &cache, FORMAT);
        pipeline.set_size(width as f32, height as f32);
        let frames_dir = out_dir.join("frames");
        std::fs::create_dir_all(&frames_dir)
            .with_context(|| format!("creating {}", frames_dir.display()))?;
        Ok(Self { device, queue, texture, view, pipeline, width, height, frames_dir, next_frame: 0 })
    }

    /// Render ONE storyboard step: fold `buffer` + `opts` into the settled view
    /// (the shared single-frame owner), then advance the virtual clock `ticks`
    /// fixed frame-steps, writing one film frame each. When `step_png` is given,
    /// the step's LAST film frame is byte-copied there and its plain-schema
    /// sidecar written beside it — so a per-step artifact IS a film frame, never
    /// a divergent re-render. Returns the inclusive film-frame range emitted.
    pub fn render_step(
        &mut self,
        buffer: &Buffer,
        opts: &CaptureOpts,
        ticks: u32,
        step_png: Option<&Path>,
    ) -> Result<(u32, u32)> {
        let vstate = settled_viewstate(&mut self.pipeline, buffer, opts, self.height);
        // The caret-style picker's preview caret has no live loop headlessly;
        // pin it settled (a no-op unless that picker is open). The DOCUMENT
        // caret is deliberately NOT settled — the film's whole point.
        self.pipeline.settle_caret_preview();
        let first = self.next_frame;
        let mut last_path = PathBuf::new();
        for _ in 0..ticks.max(1) {
            // The injected virtual-clock tick — the same single seam the
            // timeline/held captures and the live loop drive.
            self.pipeline.advance(FRAME_MS as f32 / 1000.0);
            self.pipeline.prepare(&self.device, &self.queue, self.width, self.height)?;
            let mut encoder =
                self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("awl film encoder"),
                });
            self.pipeline.render(&mut encoder, &self.view)?;
            self.queue.submit(Some(encoder.finish()));
            let img = read_frame(&self.device, &self.queue, &self.texture, self.width, self.height)?;
            let path = self.frames_dir.join(format!("frame-{:05}.png", self.next_frame));
            img.save(&path)
                .with_context(|| format!("failed to write PNG {}", path.display()))?;
            self.next_frame += 1;
            last_path = path;
        }
        if let Some(png) = step_png {
            std::fs::copy(&last_path, png)
                .with_context(|| format!("copying step frame to {}", png.display()))?;
            write_sidecar(png, &vstate, &self.pipeline, opts, None)?;
        }
        Ok((first, self.next_frame - 1))
    }

    /// Total film frames emitted so far.
    pub fn frame_count(&self) -> u32 {
        self.next_frame
    }
}
