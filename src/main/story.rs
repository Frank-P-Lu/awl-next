//! main/story.rs — the STORYBOARD ORCHESTRATOR: run one parsed storyboard
//! (`crate::storyboard`) end-to-end through the strict replay session and the
//! film renderer, emitting every artifact of a run:
//!
//!   * `step-NNN.png` + `step-NNN.json` — one settled frame + plain-schema
//!     sidecar per action step (press / type / pause / run_for; an `expect`
//!     step changes no state and renders nothing);
//!   * `frames/frame-NNNNN.png` — the film's raw frames, one per fixed
//!     [`capture::FRAME_MS`] virtual-clock tick — ALWAYS retained (they are
//!     the deterministic deliverable);
//!   * `trace.json` — every chord's action + effect classification, every
//!     assertion outcome, the abort (if any); byte-identical across runs
//!     ([`crate::storyboard::render_trace`]);
//!   * `film.webm` (+ `film.mp4`) — encoded FROM the frames by a local
//!     `ffmpeg` when one is present; its absence only skips the encode,
//!     never the run (the raw-frames fallback the spec names).
//!
//! PHASE-1 STRICTNESS: the session runs [`crate::replay::Mode::Strict`], so an
//! unbound chord, a dangling prefix, or a live-App-only (Unsupported) effect
//! aborts the run NAMING the offender — after writing the partial trace with
//! its `abort` record, so the trace and stderr never disagree. A failed
//! `expect` aborts the same way. The run is HERMETIC: `args.rs` installed the
//! in-memory sandbox (seeded from the storyboard's own file + an explicit
//! `--config`) before the config loaded, so the only real writes are the
//! artifacts above (written with `std::fs` — the deliverable deliberately
//! bypasses the app fs seam, like the capture PNG itself).

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::capture::{self, CaptureOpts};
use crate::config::Config;
use crate::storyboard::{
    self, StateView, StepKind, Storyboard, Trace, TraceAbort, TraceStep,
};

/// Run one storyboard. `file` is the board's document resolved against the
/// board's own directory (already seeded into the hermetic sandbox);
/// `root`/`workspace`/`notes_root`/`config` mirror `capture_screenshot`'s
/// context exactly, so a storyboard session and a `--keys` capture resolve the
/// same project the same way.
#[allow(clippy::too_many_arguments)] // mirrors capture_screenshot's own surface
pub(crate) fn run_storyboard(
    board: Storyboard,
    file: Option<PathBuf>,
    out_dir: PathBuf,
    root: Option<PathBuf>,
    workspace: Option<PathBuf>,
    notes_root: PathBuf,
    config: Config,
    mut km: crate::keymap::KeymapState,
) -> Result<()> {
    // The board's optional WORLD (the `--theme` vocabulary + setter).
    if let Some(t) = &board.theme {
        crate::theme::set_active_by_name(t).ok_or_else(|| {
            let names: Vec<&str> = crate::theme::THEMES.iter().map(|t| t.name).collect();
            anyhow::anyhow!("storyboard: unknown theme {t:?}; choose one of {}", names.join(", "))
        })?;
    }
    // Project context — the same resolution capture_screenshot performs, inside
    // the hermetic sandbox (a seeded root resolves as non-git; the index walk
    // sees exactly the seeded files).
    let active_root = crate::run::resolve_root(&root, &file, config.project_root.as_deref());
    let proj = crate::project::Project::resolve(&active_root);
    let corpus = crate::index::build_index(&active_root);
    let effective_workspace = crate::run::resolve_workspace(&workspace, &active_root);
    let project = capture::ProjectInfo {
        root: active_root.clone(),
        name: proj.name.clone(),
        branch: proj.branch.clone(),
        dirty: proj.dirty,
        notes_root: Some(notes_root.clone()),
        workspace: Some(effective_workspace.clone()),
        keymap_flavor: config.keymap_flavor().config_name(),
    };
    let mut buffer = crate::run::load_buffer(&file);
    // Sticky page measure for the OPENING buffer's own class (mirrors the
    // replay Goto arm / `App::sync_page_measure`).
    crate::page::set_measure(config.measure_for(buffer.page_class()));

    // STRICT REPLAY: a storyboard's motion MUST ride the real wrap geometry —
    // refuse the logical-line fallback up front, exactly like `--strict-replay`.
    let mut oracle = capture::build_oracle(&buffer, &CaptureOpts::default());
    if oracle.is_none() {
        return Err(crate::replay::missing_oracle_error());
    }

    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("creating {}", out_dir.display()))?;
    let mut renderer = capture::FilmRenderer::new(&out_dir)?;
    let mut session = crate::run::ReplaySession::new(
        crate::replay::Mode::Strict,
        &mut buffer,
        &corpus,
        &active_root,
        Some(effective_workspace.as_path()),
        &notes_root,
        &config,
        oracle.as_mut(),
        &mut km,
    );

    let mut trace = Trace {
        storyboard: board.name.clone(),
        frame_ms: capture::FRAME_MS,
        steps: Vec::new(),
        abort: None,
    };
    for (i, step) in board.steps.iter().enumerate() {
        let mut entry = TraceStep {
            index: i,
            kind: step.kind_str(),
            input: step.input_str(),
            frames: None,
            chords: Vec::new(),
            asserts: Vec::new(),
        };
        // 1) Apply the step's chords (press / type) through the strict session.
        let apply_err = match step {
            StepKind::Press { chords, .. } | StepKind::Type { chords, .. } => chords
                .iter()
                .try_for_each(|chord| session.apply_chord(chord))
                .err(),
            StepKind::Pause { .. } | StepKind::RunFor { .. } | StepKind::Expect(_) => None,
        };
        entry.chords = session.drain_records();
        if let Some(e) = apply_err {
            // STRICT ABORT: write the partial trace naming the offender, then
            // surface the exact same error (trace and stderr can't disagree).
            trace.abort = Some(TraceAbort { step: i, reason: e.to_string() });
            trace.steps.push(entry);
            write_trace(&out_dir, &trace)?;
            return Err(e.context(format!(
                "storyboard {:?}: step {i} ({}) aborted — trace.json records it",
                board.name,
                step.kind_str(),
            )));
        }
        // 2) Check expectations, or render the step's frame(s).
        match step {
            StepKind::Expect(exp) => {
                entry.asserts = storyboard::eval_expect(exp, &state_view(&session));
                let failures: Vec<String> = entry
                    .asserts
                    .iter()
                    .filter(|a| !a.pass)
                    .map(|a| format!("{}: expected {:?}, got {:?}", a.check, a.expected, a.actual))
                    .collect();
                let failed = !failures.is_empty();
                trace.steps.push(entry);
                if failed {
                    let msg = failures.join("; ");
                    trace.abort =
                        Some(TraceAbort { step: i, reason: format!("expectation failed — {msg}") });
                    write_trace(&out_dir, &trace)?;
                    bail!("storyboard {:?}: step {i} expectation failed — {msg}", board.name);
                }
            }
            StepKind::Press { .. }
            | StepKind::Type { .. }
            | StepKind::Pause { .. }
            | StepKind::RunFor { .. } => {
                // One virtual-clock tick per press/type step; a pause/run_for's
                // duration in whole frames (ceil — a 1ms pause still films once).
                let ticks = match step {
                    StepKind::Pause { ms } | StepKind::RunFor { ms } => {
                        ms.div_ceil(capture::FRAME_MS).max(1)
                    }
                    _ => 1,
                };
                let opts = step_opts(&session, &project);
                let step_png = out_dir.join(format!("step-{i:03}.png"));
                entry.frames =
                    Some(renderer.render_step(session.buffer(), &opts, ticks, Some(&step_png))?);
                trace.steps.push(entry);
            }
        }
    }
    write_trace(&out_dir, &trace)?;
    println!(
        "wrote {} steps, {} film frames, trace.json to {}",
        board.steps.len(),
        renderer.frame_count(),
        out_dir.display()
    );
    encode_films(&out_dir);
    Ok(())
}

/// The session state an `expect` step is checked against — the plain data view
/// [`storyboard::eval_expect`] consumes.
fn state_view(session: &crate::run::ReplaySession) -> StateView {
    StateView {
        cursor: session.buffer().cursor_line_col(),
        overlay: session
            .overlay()
            .map(|o| o.kind.as_str().to_string())
            .unwrap_or_else(|| "none".to_string()),
        search_active: session.search().is_some(),
        search_query: session.search().map(|s| s.query().to_string()).unwrap_or_default(),
        selection: session.buffer().selection_line_col().is_some(),
        text: session.buffer().text(),
    }
}

/// Fold the CURRENT session state into the per-step [`CaptureOpts`] — the same
/// zoom / selection / search / overlay / buffers fold `capture_screenshot`
/// performs on a finished replay, evaluated mid-run so every step's frame +
/// sidecar reflect the state at that step.
fn step_opts(session: &crate::run::ReplaySession, project: &capture::ProjectInfo) -> CaptureOpts {
    let mut opts = CaptureOpts::default();
    opts.project = Some(project.clone());
    opts.zoom = (session.zoom() != 1.0).then(|| session.zoom());
    opts.selection = session.buffer().selection_line_col();
    if let Some(s) = session.search() {
        opts.search = Some(s.query().to_string());
        opts.search_case_sensitive = s.is_case_sensitive();
        opts.search_replace_active = s.is_replace_active();
        opts.search_replacement = s.replacement().to_string();
        opts.search_editing_replacement = s.is_editing_replacement();
    }
    if let Some(ov) = session.overlay() {
        let (info, preview_text, diff) = crate::run::overlay_capture_info(ov, session.buffer());
        opts.overlay = Some(info);
        opts.preview_text = preview_text;
        // DIFF-AS-PREVIEW: mirror the one-shot capture's fold (diff state block
        // + the overlay-owned diff scroll), so a storyboard step reports the
        // same preview the single-frame path would.
        if opts.diff.is_none() {
            opts.diff = diff;
        }
        if opts.scroll.is_none() && opts.preview_text.is_some() {
            opts.scroll = Some(ov.diff_scroll);
        }
    }
    opts.buffers = Some(capture::BuffersInfo {
        open: session.buffers_open(),
        active: match session.buffer().path() {
            Some(p) => p.display().to_string(),
            None => "scratch".to_string(),
        },
    });
    opts
}

/// Write (or overwrite) `trace.json` — `std::fs`, the deliverable seam.
fn write_trace(out_dir: &Path, trace: &Trace) -> Result<()> {
    let path = out_dir.join("trace.json");
    std::fs::write(&path, storyboard::render_trace(trace))
        .with_context(|| format!("writing {}", path.display()))
}

/// Encode `frames/` into `film.webm` (+ `film.mp4`) via a LOCAL `ffmpeg`, when
/// one is on PATH. The taste call (flagged in the phase report): every
/// pure-Rust WebM route needs a VP8/VP9/AV1 encoder — a heavyweight dependency
/// this lean crate shouldn't carry — so the spec's frames-plus-optional-ffmpeg
/// fallback is the shipped design. Encoding is BEST-EFFORT by contract: a
/// missing or failing ffmpeg only prints a note; the raw frames (the
/// byte-deterministic deliverable) are always retained either way. `-bitexact`
/// pins the container metadata so the same ffmpeg build re-encodes stably;
/// byte-determinism is only CLAIMED for trace + frames, never the films.
fn encode_films(out_dir: &Path) {
    let pattern = out_dir.join("frames").join("frame-%05d.png");
    let Some(pattern) = pattern.to_str().map(str::to_string) else {
        eprintln!("film not encoded (non-UTF-8 output path) — raw frames retained in frames/");
        return;
    };
    let fps = (1000 / capture::FRAME_MS).to_string();
    let targets: [(&str, &[&str]); 2] = [
        ("film.webm", &["-c:v", "libvpx-vp9", "-pix_fmt", "yuv420p", "-b:v", "0", "-crf", "32"]),
        ("film.mp4", &["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18", "-preset", "medium"]),
    ];
    for (name, codec) in targets {
        let out = out_dir.join(name);
        let mut cmd = std::process::Command::new("ffmpeg");
        cmd.args(["-y", "-loglevel", "error", "-framerate", &fps, "-i", &pattern])
            .args(codec)
            .args(["-threads", "1", "-fflags", "+bitexact", "-flags", "+bitexact"])
            .arg(&out);
        match cmd.status() {
            Ok(s) if s.success() => println!("wrote {}", out.display()),
            Ok(s) => eprintln!(
                "ffmpeg exited with {s} encoding {name} — raw frames retained in frames/"
            ),
            Err(_) => {
                eprintln!("film not encoded (no ffmpeg on PATH) — raw frames retained in frames/");
                return; // no point trying the second target
            }
        }
    }
}
