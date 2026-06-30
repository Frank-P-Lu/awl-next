//! The headless visual-line MOTION ORACLE: an offscreen-shaped [`TextPipeline`]
//! built so a `--keys` replay's visual-line motions get the SAME wrapped-row
//! geometry the live window would answer with. Lifted out of `capture.rs` VERBATIM
//! — it owns its device/queue so the borrow stays valid across the whole replay.
//! See [`super`].

use anyhow::Result;
use glyphon::Cache;

use crate::buffer::Buffer;
use crate::render::{self, TextPipeline};

use super::gpu::headless_device;
use super::modes::base_viewstate;
use super::opts::CaptureOpts;
use super::{CANVAS_HEIGHT, CANVAS_WIDTH, FORMAT};

/// A headless, offscreen-shaped [`TextPipeline`] built SOLELY to answer the
/// visual-line motion [`crate::actions::LayoutOracle`] queries during a `--keys`
/// replay — the headless twin of the live window's GPU pipeline. Because the
/// `--keys` replay (`main.rs::replay_keys`) runs BEFORE the capture builds its own
/// render pipeline, motion that needs wrap geometry has nothing to ask; this gives
/// it the SAME pipeline code the window uses, so live and `--keys` motion can't
/// drift. Owns its device/queue so the borrow stays valid across the whole replay.
///
/// It is built once from the same canvas / dpi / zoom (and the global page
/// measure) the capture will use, then shaped to the loaded buffer, giving the
/// replay's visual-line motions their wrapped-row geometry. It shapes the
/// PRE-REPLAY buffer once: motions read it as the flat default, while a replay
/// that EDITS the text then keeps moving sees slightly stale wrap geometry (the
/// accepted limit today — captures replay motion, not bulk edits-then-motion).
pub struct OraclePipeline {
    // Held only to keep the pipeline's GPU resources alive for the borrow's life.
    _device: wgpu::Device,
    _queue: wgpu::Queue,
    pipeline: TextPipeline,
}

impl OraclePipeline {
    /// Borrow as the renderer-agnostic motion oracle for `ActionCtx::oracle`.
    pub fn as_oracle(&self) -> &dyn crate::actions::LayoutOracle {
        &self.pipeline
    }
}

/// Build the headless visual-motion [`OraclePipeline`] for `buffer`, mirroring the
/// canvas / dpi / zoom the matching capture uses so the wrap geometry agrees.
/// Returns `None` (so motion falls back to LOGICAL lines, unchanged behavior) when
/// no wgpu adapter is available, keeping a GPU-less environment working.
pub fn build_oracle(buffer: &Buffer, opts: &CaptureOpts) -> Option<OraclePipeline> {
    match pollster::block_on(build_oracle_async(buffer, opts)) {
        Ok(op) => Some(op),
        Err(e) => {
            eprintln!("visual-motion oracle unavailable (falling back to logical lines): {e}");
            None
        }
    }
}

async fn build_oracle_async(buffer: &Buffer, opts: &CaptureOpts) -> Result<OraclePipeline> {
    let (device, queue) = headless_device().await?;
    let (width, height) = opts.canvas.unwrap_or((CANVAS_WIDTH, CANVAS_HEIGHT));
    let dpi = opts.dpi.unwrap_or(1.0);
    let zoom = render::clamp_zoom(opts.zoom.unwrap_or(1.0));
    let cache = Cache::new(&device);
    let mut pipeline = TextPipeline::new(&device, &queue, &cache, FORMAT);
    pipeline.set_size(width as f32, height as f32);
    pipeline.set_dpi(dpi); // AFTER set_size (reads window_w); no-op at the default 1.0.
    // Shape the document so `visual_rows` can answer wrap queries. The state beyond
    // the text/zoom (selection / search / overlay) doesn't affect wrap geometry.
    let (cl, cc) = buffer.cursor_line_col();
    let vstate = base_viewstate(buffer, &opts.project, (cl, cc), zoom, Vec::new(), false);
    pipeline.set_view(&vstate);
    Ok(OraclePipeline {
        _device: device,
        _queue: queue,
        pipeline,
    })
}
