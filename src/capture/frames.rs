//! src/capture/frames.rs — the FRAME-LOOP capture (`--screenshot-frames N OUT.png`):
//! N successive SETTLED frames driven by a REAL [`crate::app::App`]'s scheduling
//! loop under a deterministic [`crate::clock::VirtualClock`], so a LIVE-ONLY
//! cross-frame behaviour becomes inspectable from screenshots — the class the
//! single settled `--screenshot` frame is structurally blind to (it never builds an
//! App; it renders one settled `ViewState`).
//!
//! What is REAL here vs. the single-frame path: an App is constructed
//! (`App::new_headless_scheduler`, hermetic + `gpu: None`), a `VirtualClock` is
//! swapped in behind its `Box<dyn Clock>` seam, and each frame ADVANCES the clock a
//! fixed step and runs the App's ACTUAL `about_to_wait_impl` scheduling body
//! (`App::step_scheduling`, writing its deadlines into a `RecordingScheduler`
//! instead of `&ActiveEventLoop`). The demonstrated behaviour is the WHICH-KEY
//! debounce: a `C-x` prefix is armed at t=0, and the continuation panel must summon
//! EXACTLY at its `whichkey::PAUSE` (500 ms) deadline step — a false→true flip a
//! single frame cannot express. The rigorous assertion of that flip is the
//! `app::tests` scheduling law; this mode is its inspectable-artifact tier (the
//! `--capture-timeline` precedent).
//!
//! The App renders NOTHING itself (`gpu: None`), so the harness owns its own
//! offscreen device/texture/pipeline (the `FilmRenderer` shape) and draws the
//! document (`OUT.png`'s `[file]`) plus the panel the App's scheduling state reports
//! (`whichkey_continuation_rows`) — the doc is a stationary backdrop; the panel
//! appearing at the deadline frame is the real cross-frame signal. Frames land at
//! `<stem>.fNNN.png` + the ordinary plain-schema sidecar, plus one deterministic
//! `<stem>.frames.json` recording the per-frame scheduling STATE (elapsed_ms,
//! whichkey_shown, whether a `WaitUntil` was armed) — the machine-readable oracle.
//!
//! DETERMINISM: the `VirtualClock`'s only real read is an unobserved base that
//! cancels out of every delta comparison (see its doc), the document + panel are
//! pure functions of the input, and the sidecars stamp virtual `elapsed_ms`
//! integers — never a wall clock — so two runs are byte-identical.

use anyhow::{Context, Result};
use glyphon::Cache;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::app::{App, RecordingScheduler};
use crate::buffer::Buffer;
use crate::clock::VirtualClock;
use crate::config::Config;
use crate::render::TextPipeline;

use super::gpu::{headless_device, offscreen_target, read_frame};
use super::modes::settled_viewstate;
use super::opts::CaptureOpts;
use super::sidecar::{json_string, write_sidecar};
use super::{CANVAS_HEIGHT, CANVAS_WIDTH, FORMAT};

/// The default per-frame virtual step: 100 ms. Chosen so the which-key `PAUSE`
/// (500 ms) lands CRISPLY on a frame boundary — frame index 4 (t = 500 ms) is the
/// first summoned frame — making the debounce flip unambiguous in the artifacts.
pub const DEFAULT_FRAME_STEP_MS: u64 = 100;

/// One recorded frame's scheduling STATE, for the deterministic `<stem>.frames.json`
/// summary (the machine-readable oracle the single PNG can't be).
struct FrameRecord {
    frame: u32,
    elapsed_ms: u64,
    whichkey_shown: bool,
    wait_scheduled: bool,
}

/// Render `frames` successive settled frames of the real App scheduling loop under a
/// virtual clock stepped `step_ms` per frame. See the module doc.
pub fn capture_frames(
    out_png: &Path,
    buffer: &Buffer,
    frames: u32,
    step_ms: u64,
    opts: &CaptureOpts,
) -> Result<()> {
    pollster::block_on(capture_frames_async(out_png, buffer, frames, step_ms, opts))
}

async fn capture_frames_async(
    out_png: &Path,
    buffer: &Buffer,
    frames: u32,
    step_ms: u64,
    opts: &CaptureOpts,
) -> Result<()> {
    // --- Offscreen device + pipeline (the harness renders; the App does not) ---
    let (device, queue) = headless_device().await?;
    let (width, height) = opts.canvas.unwrap_or((CANVAS_WIDTH, CANVAS_HEIGHT));
    let (texture, view) = offscreen_target(&device, width, height);
    let cache = Cache::new(&device);
    let mut pipeline = TextPipeline::new(&device, &queue, &cache, FORMAT);
    pipeline.set_size(width as f32, height as f32);

    // --- The real App, driving the SCHEDULING under a virtual clock -----------
    // `gpu: None`, so it renders nothing itself; its scheduling body is what we step.
    let clock = VirtualClock::new();
    let mut app = App::new_headless_scheduler(PathBuf::from("/"), Config::empty());
    app.set_clock(Box::new(clock.clone()));
    // Arm the which-key prefix at virtual t=0 (the `C-x` edge), then step the clock
    // past `whichkey::PAUSE` across the frames to witness the summon.
    app.arm_whichkey_prefix();

    let sched = RecordingScheduler::new();
    let mut records: Vec<FrameRecord> = Vec::new();

    for i in 0..frames.max(1) {
        // Advance virtual time, then run the REAL scheduling body into the recorder.
        clock.advance_ms(step_ms);
        sched.begin_step();
        app.step_scheduling(&sched);

        let elapsed_ms = (i as u64 + 1) * step_ms;
        let shown = app.whichkey_is_shown();
        let wait_scheduled = matches!(
            sched.scheduled_this_step(),
            Some(winit::event_loop::ControlFlow::WaitUntil(_))
        );

        // Fold the (stationary) document into the settled view via the ONE shared
        // owner, then overlay the panel the App's scheduling state says is up.
        let vstate = settled_viewstate(&mut pipeline, buffer, opts, height);
        pipeline.settle_caret();
        pipeline.settle_caret_preview();
        pipeline.set_whichkey(if shown {
            Some(app.whichkey_continuation_rows())
        } else {
            None
        });
        pipeline.prepare(&device, &queue, width, height)?;

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("awl frames encoder"),
        });
        pipeline.render(&mut encoder, &view)?;
        queue.submit(Some(encoder.finish()));
        let img = read_frame(&device, &queue, &texture, width, height)?;

        let stem = out_png
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "capture".to_string());
        let frame_png = out_png.with_file_name(format!("{stem}.f{i:03}.png"));
        img.save(&frame_png)
            .with_context(|| format!("failed to write PNG {}", frame_png.display()))?;
        write_sidecar(&frame_png, &vstate, &pipeline, opts, None)?;

        records.push(FrameRecord { frame: i, elapsed_ms, whichkey_shown: shown, wait_scheduled });
    }

    write_frames_summary(out_png, step_ms, &records)?;
    Ok(())
}

/// Write the deterministic `<stem>.frames.json` scheduling-state summary beside the
/// per-frame PNGs — integers + bools only (virtual `elapsed_ms`, never a wall
/// clock), so it is the byte-stable machine oracle for the cross-frame debounce.
fn write_frames_summary(out_png: &Path, step_ms: u64, records: &[FrameRecord]) -> Result<()> {
    let stem = out_png
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "capture".to_string());
    let path = out_png.with_file_name(format!("{stem}.frames.json"));

    let rows: Vec<String> = records
        .iter()
        .map(|r| {
            format!(
                "    {{ \"frame\": {}, \"elapsed_ms\": {}, \"whichkey_shown\": {}, \"wait_scheduled\": {} }}",
                r.frame, r.elapsed_ms, r.whichkey_shown, r.wait_scheduled
            )
        })
        .collect();
    let json = format!(
        "{{\n  \"schema\": {},\n  \"driver\": \"virtual-clock frame loop (about_to_wait_impl)\",\n  \
         \"scenario\": \"whichkey-prefix-pause\",\n  \"frame_step_ms\": {},\n  \"frames\": [\n{}\n  ]\n}}\n",
        json_string("awl-frames/1"),
        step_ms,
        rows.join(",\n")
    );
    // Write via `File::create` + `write_all` (the sidecar writer's exact shape) — a
    // throwaway capture artifact, not a durable app-owned store, so it deliberately
    // does NOT route through `fs::write_atomic` (mirrors `write_sidecar`, and stays
    // clear of the `durable.rs` bare-write audit for the same reason).
    let mut f = std::fs::File::create(&path)
        .with_context(|| format!("failed to create frame-loop summary {}", path.display()))?;
    f.write_all(json.as_bytes())
        .with_context(|| format!("failed to write frame-loop summary {}", path.display()))?;
    Ok(())
}
