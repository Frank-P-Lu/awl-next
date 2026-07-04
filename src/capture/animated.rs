//! The DETERMINISTIC per-step animation captures: the `--capture-timeline` caret
//! glide and the `--capture-held` auto-repeat run, each stepping a VIRTUAL clock
//! (no winit, no real clock, no RNG) and writing one PNG + sidecar per entry. Plus
//! the held re-target stepper ([`step_held`]) and the [`HeldDir`] it reads. Lifted
//! out of `capture.rs` VERBATIM; the shared single-frame seams ([`super::modes`])
//! and GPU plumbing ([`super::gpu`]) are reused so the trajectories can't drift. See
//! [`super`].

use anyhow::{Context, Result};
use glyphon::Cache;
use std::path::Path;

use crate::buffer::Buffer;
use crate::render::{self, TextPipeline};

use super::gpu::{headless_device, offscreen_target, read_frame};
use super::modes::{base_viewstate, follow_scroll};
use super::opts::CaptureOpts;
use super::sidecar::{write_sidecar, CaretFrame, CosmeticReport, TrailReport};
use super::{CANVAS_HEIGHT, CANVAS_WIDTH, FORMAT};

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
    let misspelled = match crate::spell::SpellChecker::new(crate::spell::active_variant()) {
        Ok(sc) => sc.misspellings_for(&buffer.text(), buffer.syntax_lang()),
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
    let misspelled = match crate::spell::SpellChecker::new(crate::spell::active_variant()) {
        Ok(sc) => sc.misspellings_for(&buffer.text(), buffer.syntax_lang()),
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
pub(super) fn step_held(
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
