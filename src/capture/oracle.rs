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
use super::opts::{CaptureOpts, ProjectInfo};
use super::{CANVAS_HEIGHT, CANVAS_WIDTH, FORMAT};

/// A headless, offscreen-shaped [`TextPipeline`] built SOLELY to answer the
/// visual-line motion [`crate::actions::LayoutOracle`] queries during a `--keys`
/// replay — the headless twin of the live window's GPU pipeline. Because the
/// `--keys` replay (`main/run.rs::replay_keys`) runs BEFORE the capture builds its
/// own render pipeline, motion that needs wrap geometry has nothing to ask; this
/// gives it the SAME pipeline code the window uses, so live and `--keys` motion
/// can't drift. Owns its device/queue so the borrow stays valid across the whole
/// replay.
///
/// It is built once from the same canvas / dpi / zoom (and the global page
/// measure) the capture will use, then RE-SHAPED FROM THE CURRENT REPLAY STATE
/// before every scripted action ([`Self::refresh`], called by the replay loop) —
/// exactly as the live window's pipeline is re-synced between keystrokes — so an
/// edit that re-wraps a line, a replayed zoom change, a Goto buffer switch, or
/// the page-measure re-apply that rides it can never leave a later motion
/// reading STALE wrap geometry (the retired build-once limit).
pub struct OraclePipeline {
    // Held only to keep the pipeline's GPU resources alive for the borrow's life.
    _device: wgpu::Device,
    _queue: wgpu::Queue,
    pipeline: TextPipeline,
    /// The capture's canvas, remembered so [`Self::refresh`]'s `set_size` re-reads
    /// the page-measure global at the SAME dimensions every time (the seam that
    /// picks up a mid-replay `page::set_measure` — the Goto arm).
    width: f32,
    height: f32,
    /// An EXPLICIT `--zoom` flag, which pins the final frame's zoom regardless of
    /// replayed zoom keys ("never clobber an explicit verification hook") — so the
    /// oracle rides it too, staying consistent with the frame it verifies. `None`
    /// (no flag) follows the replay's own live zoom on every refresh.
    cli_zoom: Option<f32>,
    /// The capture's project block, re-fed to [`base_viewstate`] on every refresh
    /// (it fills the gutter/markdown/syntax fields the shaping reads).
    project: Option<ProjectInfo>,
}

impl OraclePipeline {
    /// Borrow as the renderer-agnostic motion oracle for `ActionCtx::oracle`.
    pub fn as_oracle(&self) -> &dyn crate::actions::LayoutOracle {
        &self.pipeline
    }

    /// FRESH LAYOUT ORACLE PER ACTION — the ONE freshness seam: re-shape from the
    /// CURRENT buffer / zoom / page-measure state, so the wrap geometry a motion
    /// is about to consult is never stale. The replay loop calls this before
    /// EVERY scripted action (mirroring the live window, whose pipeline re-syncs
    /// between keystrokes), which covers all four staleness sources at once:
    ///
    ///   * an EDIT that re-wraps a line (`set_view` reshapes on changed text),
    ///   * a replayed ZOOM change (`set_view` re-wraps on a changed metric —
    ///     unless an explicit `--zoom` pinned it, see [`Self::cli_zoom`]),
    ///   * a GOTO buffer switch (`set_view` shapes the arriving buffer's text),
    ///   * the PAGE-MEASURE re-apply riding that switch (`set_size` re-reads the
    ///     measure global and re-wraps only when the wrap width really changed).
    ///
    /// Unconditional by design — both calls no-op cheaply when nothing changed
    /// (cosmic-text skips an unchanged size; `set_view` skips an unchanged
    /// composed text + metric), and correctness beats invalidation cleverness
    /// here (the capture path is one-shot, not per-frame).
    pub fn refresh(&mut self, buffer: &Buffer, replay_zoom: f32) {
        self.pipeline.set_size(self.width, self.height);
        let zoom = render::clamp_zoom(self.cli_zoom.unwrap_or(replay_zoom));
        let (cl, cc) = buffer.cursor_line_col();
        let vstate = base_viewstate(buffer, &self.project, (cl, cc), zoom, Vec::new(), false);
        self.pipeline.set_view(&vstate);
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
        width: width as f32,
        height: height as f32,
        cli_zoom: opts.zoom,
        project: opts.project.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // GPU-backed refresh-seam unit tests: each isolates ONE staleness source the
    // per-action refresh must pick up, asserting through the same LayoutOracle
    // queries the replayed motions use. All hold `testlock::serial()` (wrap
    // geometry folds the page globals) and skip cleanly with no wgpu adapter.

    #[test]
    fn refresh_picks_up_a_page_measure_change() {
        // The Goto arm re-applies the arriving buffer's measure mid-replay via
        // `page::set_measure`; `refresh`'s `set_size` must re-read it. A
        // narrower measure wraps the long line earlier, so one visual-row-down
        // step from (0,0) lands at a strictly SMALLER column.
        let _g = crate::testlock::serial();
        crate::page::set_page_on(true);
        crate::page::set_measure(30);
        let buffer = Buffer::from_str(&"word ".repeat(20));
        let opts = CaptureOpts::default();
        let Some(mut op) = build_oracle(&buffer, &opts) else {
            crate::page::set_measure(crate::page::DEFAULT_MEASURE);
            eprintln!("skipping refresh_picks_up_a_page_measure_change: no wgpu adapter");
            return;
        };
        let wide = op.as_oracle().visual_line_down(0, 0, 0.0);
        crate::page::set_measure(15);
        op.refresh(&buffer, 1.0);
        let narrow = op.as_oracle().visual_line_down(0, 0, 0.0);
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
        assert_eq!(wide.0, 0, "the long line wraps at measure 30, down stays on line 0");
        assert_eq!(narrow.0, 0, "still wrapped at measure 15");
        assert!(
            narrow.1 < wide.1,
            "a narrower measure wraps earlier: {} < {}",
            narrow.1,
            wide.1
        );
    }

    #[test]
    fn refresh_reshapes_to_a_swapped_buffer() {
        // A Goto switch replaces the ACTIVE buffer mid-replay; `refresh` must
        // shape the ARRIVING text. Built on a short-lined buffer, refreshed to
        // one whose line 0 wraps: down from (0,0) now stays on logical line 0.
        let _g = crate::testlock::serial();
        crate::page::set_page_on(true);
        crate::page::set_measure(15);
        let short = Buffer::from_str("ab\ncd\n");
        let opts = CaptureOpts::default();
        let Some(mut op) = build_oracle(&short, &opts) else {
            crate::page::set_measure(crate::page::DEFAULT_MEASURE);
            eprintln!("skipping refresh_reshapes_to_a_swapped_buffer: no wgpu adapter");
            return;
        };
        assert_eq!(
            op.as_oracle().visual_line_down(0, 0, 0.0).0,
            1,
            "the short line 0 does not wrap: down crosses into line 1"
        );
        let long = Buffer::from_str(&format!("{}\ntail\n", "word ".repeat(10)));
        op.refresh(&long, 1.0);
        let (line, col) = op.as_oracle().visual_line_down(0, 0, 0.0);
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
        assert_eq!(line, 0, "after refresh, down follows the arriving buffer's wrapped line 0");
        assert!(col > 0, "landing on line 0's second visual row, got col {col}");
    }

    #[test]
    fn refresh_follows_the_replay_zoom_unless_an_explicit_cli_zoom_pins_it() {
        // With the column capped by the WINDOW (MAX_MEASURE), a bigger zoom fits
        // fewer chars per visual row — a replayed Cmd-+ must move the wrap
        // boundary. An explicit `--zoom` is a verification hook that pins the
        // final frame, so the oracle ignores the replay zoom and stays put.
        let _g = crate::testlock::serial();
        crate::page::set_page_on(true);
        crate::page::set_measure(crate::page::MAX_MEASURE);
        let buffer = Buffer::from_str(&"word ".repeat(80));
        let opts = CaptureOpts::default();
        let Some(mut op) = build_oracle(&buffer, &opts) else {
            crate::page::set_measure(crate::page::DEFAULT_MEASURE);
            eprintln!("skipping refresh_follows_the_replay_zoom: no wgpu adapter");
            return;
        };
        let at_one = op.as_oracle().visual_line_down(0, 0, 0.0);
        op.refresh(&buffer, 1.5);
        let zoomed = op.as_oracle().visual_line_down(0, 0, 0.0);
        assert_eq!((at_one.0, zoomed.0), (0, 0), "the long line wraps at both zooms");
        assert!(
            zoomed.1 < at_one.1,
            "zoomed glyphs fit fewer chars per row: {} < {}",
            zoomed.1,
            at_one.1
        );

        // The explicit-flag pin: a `--zoom 1.0` oracle refreshed at replay zoom
        // 1.5 keeps the flag's geometry (byte-identical wrap boundary).
        let pinned_opts = CaptureOpts { zoom: Some(1.0), ..CaptureOpts::default() };
        let Some(mut pinned) = build_oracle(&buffer, &pinned_opts) else {
            crate::page::set_measure(crate::page::DEFAULT_MEASURE);
            eprintln!("skipping refresh zoom-pin half: no wgpu adapter");
            return;
        };
        pinned.refresh(&buffer, 1.5);
        let still = pinned.as_oracle().visual_line_down(0, 0, 0.0);
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
        assert_eq!(still, at_one, "an explicit --zoom pins the oracle's geometry too");
    }
}
