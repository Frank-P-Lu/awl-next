//! CLI argument parsing + capture-mode selection.
//!
//! This is the front half of `main.rs`: it turns `std::env::args` into a
//! [`Mode`] — the windowed editor or one of the headless capture variants — plus
//! all the small `parse_*` validators and the "did the chosen mode silently drop
//! a hook?" guard ([`unused_hooks`]). It is the pure decision layer; the actual
//! work each `Mode` performs lives in [`crate::run`].

use std::path::PathBuf;

use anyhow::{bail, Result};

use crate::capture::{self, CaptureOpts};
use crate::config::{self, Config};
use crate::keymap::KeymapState;
use crate::{caret, debug, hud, keyspec, lifetime, page, theme, whichkey};

pub(crate) enum Mode {
    Windowed {
        file: Option<PathBuf>,
        /// The ACTIVE project root (`--root`). When absent it defaults to the
        /// launch file's parent (or cwd) in `app::run`.
        root: Option<PathBuf>,
        /// The RAW `--workspace` flag (None = unset). Folded with the config inside
        /// `App::new` so a later live config reload can re-apply precedence.
        workspace: Option<PathBuf>,
        /// The RAW `--notes-root` flag (None = unset). Folded with the config (flag >
        /// config > `~/notes`) inside `App::new`; kept raw so reload keeps flag wins.
        notes_root: Option<PathBuf>,
        /// The loaded persistent config (keybinding overrides + folder defaults +
        /// the Settings-open path). Empty/all-None when no config file exists.
        config: Config,
        /// The raw `--wait` flag (single-instance daemon; `EDITOR=awl --wait` for
        /// git). Native-only meaning — see `crate::daemon`'s module doc for the
        /// documented scope of what it does and doesn't block on.
        wait: bool,
    },
    /// Deterministic one-frame capture with the caret AT REST (the resting amber
    /// rounded square on the glyph), plus optional zoom / scroll / selection
    /// verification overrides. `keys` is an optional `--keys` replay applied to
    /// the buffer BEFORE the capture, so the PNG + sidecar reflect post-replay
    /// state (cursor / selection / search).
    Screenshot {
        out: PathBuf,
        file: Option<PathBuf>,
        opts: CaptureOpts,
        keys: Vec<keyspec::Chord>,
        /// The keymap the replay loop resolves `keys` through, chord by chord
        /// (config `[keys]` rebinds + the `linux_keep_emacs` door applied) —
        /// resolution happens INSIDE the replay so the search guard can
        /// intercept a chord before the keymap ever sees it.
        km: KeymapState,
        /// The active project root for the capture (`--root`); scopes the go-to
        /// overlay and populates the sidecar `project` block.
        root: Option<PathBuf>,
        /// Optional workspace parent (`--workspace`): its child dirs are the
        /// switch-project candidates a replayed `C-x p` lists (with git markers).
        workspace: Option<PathBuf>,
        /// The notes root (`--notes-root`): scopes a replayed `C-x m` move-dest
        /// picker so the sidecar `overlay` reflects the notes folders.
        notes_root: PathBuf,
        /// The loaded persistent config: supplies the `[keys]` overrides reflected in
        /// the palette's effective bindings, and the Settings-open target.
        config: Config,
        /// STRICT REPLAY (`--strict-replay`, opt-in): abort on any unbound
        /// chord (checked at replay time by `keyspec::ChordResolver`, AFTER
        /// the search guard has had its chance to consume the chord), any
        /// Unsupported effect, or a missing layout oracle — naming the exact
        /// offender — instead of the legacy permissive warn-and-continue. The
        /// scenario-runner default the later harness phases plumb through;
        /// see `crate::replay`'s module doc. Also HERMETIC: by the time this
        /// Mode exists the process fs has been swapped to the seeded sandbox
        /// (`crate::scenario::install_hermetic_fs`, called before the config
        /// loaded), so the whole run never touches the user's real files.
        strict: bool,
    },
    /// Deterministic one-frame capture of a caret MID-GLIDE (dropped to the
    /// baseline and stretched into a trailing underline streak), so the temporal
    /// effect is inspectable from a still.
    ScreenshotMotion {
        out: PathBuf,
        file: Option<PathBuf>,
        keys: Vec<keyspec::Chord>,
        km: KeymapState,
    },
    /// Like [`Mode::ScreenshotMotion`] but a VERTICAL glide: the caret slid to a
    /// thin bar on the cell's left edge, trailing up the lines it passed.
    ScreenshotMotionVertical {
        out: PathBuf,
        file: Option<PathBuf>,
        keys: Vec<keyspec::Chord>,
        km: KeymapState,
    },
    /// Like [`Mode::ScreenshotMotion`] but a DIAGONAL glide (different row AND
    /// column): the trail is a true slanted tracer from source to target.
    ScreenshotMotionDiagonal {
        out: PathBuf,
        file: Option<PathBuf>,
        keys: Vec<keyspec::Chord>,
        km: KeymapState,
    },
    /// DETERMINISTIC TIMELINE capture: after the `--keys` replay sets up a
    /// NAVIGATION caret move (a glide, not an edit-snap), advance a VIRTUAL clock
    /// by the given cumulative-ms `steps` with an INJECTED dt, writing a frame
    /// (`OUT.t<ms>.png` + `.json`) after each step so an animation's TRAJECTORY is
    /// inspectable. `keys` is split: all-but-last set up the origin, the LAST chord
    /// is the navigation move that glides.
    CaptureTimeline {
        out: PathBuf,
        file: Option<PathBuf>,
        keys: Vec<keyspec::Chord>,
        km: KeymapState,
        /// Cumulative ms since the move started; the dt for step i is `t[i]-t[i-1]`.
        steps: Vec<u32>,
        root: Option<PathBuf>,
        /// `--capture-size` physical canvas dims (None = default 1200x800).
        canvas: Option<(u32, u32)>,
        /// `--capture-dpi` renderer scale factor (None = 1.0).
        dpi: Option<f32>,
    },
    /// DETERMINISTIC HELD-MOTION capture: reproduce a HELD arrow (OS auto-repeat)
    /// by re-targeting the caret one char/line in `dir` at EACH virtual-clock step
    /// with `held=true`, advancing the spring by the injected dt, and writing a
    /// frame (`OUT.t<ms>.png` + `.json`) per step. The `--keys` replay sets the
    /// ORIGIN the held burst starts from; the per-step sidecar records the drawn
    /// trail (length/endpoints/holding) so the held streak is machine-verifiable.
    CaptureHeld {
        out: PathBuf,
        file: Option<PathBuf>,
        keys: Vec<keyspec::Chord>,
        km: KeymapState,
        dir: capture::HeldDir,
        /// Cumulative ms; the dt for step i is `t[i]-t[i-1]`. One held re-target is
        /// applied per entry.
        steps: Vec<u32>,
        root: Option<PathBuf>,
        /// `--capture-size` physical canvas dims (None = default 1200x800).
        canvas: Option<(u32, u32)>,
        /// `--capture-dpi` renderer scale factor (None = 1.0).
        dpi: Option<f32>,
    },
    /// STORYBOARD run (`--storyboard <file.toml>`): a checked-in scenario file
    /// drives one HERMETIC, STRICT replay session end-to-end (see
    /// `crate::storyboard` + `crate::story`), emitting per-step PNG+sidecar
    /// artifacts, deterministic film frames on the virtual clock, a byte-stable
    /// `trace.json`, and (via a local ffmpeg, when present) a WebM/MP4 film.
    /// Like `--strict-replay`, by the time this Mode exists the process fs has
    /// been swapped to the seeded sandbox (`crate::scenario`).
    Storyboard {
        board: crate::storyboard::Storyboard,
        /// The board's document resolved against the storyboard file's own
        /// directory (`None` = scratch); already seeded into the sandbox.
        file: Option<PathBuf>,
        /// Where the run's artifacts land (`--storyboard-out`, defaulting to
        /// `<storyboard-stem>.run/` beside the storyboard file — gitignored).
        out_dir: PathBuf,
        root: Option<PathBuf>,
        workspace: Option<PathBuf>,
        notes_root: PathBuf,
        config: Config,
        km: KeymapState,
    },
    /// Hidden performance harness: time the per-keystroke update path (append a
    /// char -> reshape) on documents of 100/1000/5000 lines, BEFORE (whole-buffer
    /// reshape) vs AFTER (incremental), and print the numbers. Opens no window.
    BenchTyping,
    /// Hidden performance harness: time the FIVE traced hot paths (motion oracle,
    /// ornament marks, rule conceal, theme reshape) over the long fixtures under
    /// `benches/fixtures`, printing median ns per call. Opens no window.
    BenchPerf,
    /// Hidden performance harness: per-stage FRAME profile of the exact live
    /// redraw sequence (each `prepare` sub-call, render encode, submit+poll,
    /// atlas trim) over the real repo docs (CAPTURE.md / CLAUDE.md) with their
    /// real spell-squiggle load, at the live-report 2910x1720 @2x canvas,
    /// printing a stage | median ms | % table. Opens no window.
    BenchFrame,
    /// Hidden performance harness: the THEME-BURST profile — N successive
    /// font-changing theme switches (the faceted picker's live preview) over
    /// CLAUDE.md with its real spell load at the live-report 5120x2756 @2x
    /// zoom-1.1 canvas, timing `sync_theme` (the reshape) AND the first frame
    /// after each switch (the new face's atlas rasterization), two laps
    /// (cold/warm) to expose atlas retention. Opens no window.
    BenchThemeBurst,
    /// Hidden performance harness: replay a rapid adjacent-level zoom burst at
    /// the reported 3538x2610 @2x / 60% posture, comparing the old eager
    /// per-input reflow with latest-wins present-boundary coalescing. Opens no
    /// window.
    BenchZoomBurst,
    /// Hidden performance harness: the UNIFIED BENCH SUITE — deterministic
    /// corpus tiers x interaction scenarios, every cell witnessed, printed as
    /// a table and written to `bench.json` beside the invocation. `baseline`
    /// (`--bench-baseline <path>`) additionally diffs the run against a
    /// checked-in machine-keyed baseline, exiting nonzero on a >20% cell
    /// regression (the `scripts/bench.sh` merge-day gate). Opens no window.
    BenchSuite { baseline: Option<PathBuf> },
}

/// Parse a `--sel L0:C0-L1:C1` argument into ordered line/col endpoints.
fn parse_sel(s: &str) -> Result<((usize, usize), (usize, usize))> {
    let (a, b) = s
        .split_once('-')
        .ok_or_else(|| anyhow::anyhow!("--sel expects L0:C0-L1:C1, got {s:?}"))?;
    let parse_pt = |p: &str| -> Result<(usize, usize)> {
        let (l, c) = p
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("--sel endpoint expects L:C, got {p:?}"))?;
        Ok((l.trim().parse()?, c.trim().parse()?))
    };
    let p0 = parse_pt(a)?;
    let p1 = parse_pt(b)?;
    // Order so the first endpoint is earlier in the buffer.
    Ok(if p0 <= p1 { (p0, p1) } else { (p1, p0) })
}

/// Parse a `--capture-timeline "0,16,50,150"` argument into a cumulative-ms step
/// sequence. Each entry is the virtual-clock time (ms since the move started) at
/// which a frame is rendered; the dt fed to step `i` is `t[i]-t[i-1]`.
fn parse_steps(s: &str) -> Result<Vec<u32>> {
    let steps: Vec<u32> = s
        .split(',')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| {
            p.parse::<u32>()
                .map_err(|_| anyhow::anyhow!("bad --capture-timeline step {p:?} (want ms integers)"))
        })
        .collect::<Result<_>>()?;
    if steps.is_empty() {
        bail!("--capture-timeline needs at least one ms step (e.g. \"0,16,50,150\")");
    }
    Ok(steps)
}

/// Parse a `--capture-size "WxH"` argument into PHYSICAL canvas dimensions. Accepts
/// `x` or `X` as the separator (e.g. "2400x1600").
fn parse_size(s: &str) -> Result<(u32, u32)> {
    let (w, h) = s
        .split_once(['x', 'X'])
        .ok_or_else(|| anyhow::anyhow!("--capture-size expects WxH, got {s:?}"))?;
    let w: u32 = w
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("bad --capture-size width in {s:?}"))?;
    let h: u32 = h
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("bad --capture-size height in {s:?}"))?;
    if w == 0 || h == 0 {
        bail!("--capture-size dimensions must be non-zero, got {s:?}");
    }
    Ok((w, h))
}

/// Parse a `--capture-dpi` factor: a FINITE, strictly-positive scale (mirrors
/// parse_size's non-zero guard). A non-finite (`inf`/`nan`) or `<= 0` factor
/// would scale the canvas to a degenerate / zero-area render target, so reject it
/// up front rather than render garbage.
fn parse_dpi(s: &str) -> Result<f32> {
    let v: f32 = s
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("bad --capture-dpi {s:?}"))?;
    if !v.is_finite() || v <= 0.0 {
        bail!("--capture-dpi must be finite and > 0, got {s:?}");
    }
    Ok(v)
}

/// Parse a `--zoom` factor: a FINITE, strictly-positive scale (mirrors
/// parse_dpi's guard). A non-finite (`inf`/`nan`) factor would poison every
/// zoom-derived metric downstream (NaN propagates through the step/clamp
/// arithmetic), so reject it up front with a readable error rather than render
/// garbage; the in-range [0.5, 3.0] clamp stays `render::clamp_zoom`'s job.
fn parse_zoom(s: &str) -> Result<f32> {
    let v: f32 = s
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("bad --zoom {s:?}"))?;
    if !v.is_finite() || v <= 0.0 {
        bail!("--zoom must be finite and > 0, got {s:?}");
    }
    Ok(v)
}

/// Parse a `--measure` column width: a strictly-positive char count (mirrors
/// parse_size's non-zero guard — a zero-width writing column is degenerate).
fn parse_measure(s: &str) -> Result<usize> {
    let n: usize = s
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("bad --measure {s:?}"))?;
    if n == 0 {
        bail!("--measure must be > 0, got {s:?}");
    }
    Ok(n)
}

/// The capture mode resolved from the CLI flags, used ONLY to decide which
/// verification hooks the run honors (the real `Mode` is built separately). The
/// precedence mirrors the `Mode` construction below: held > timeline > motion >
/// plain screenshot; no output path at all means the windowed editor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CaptureKind {
    Windowed,
    Screenshot,
    Motion,
    Timeline,
    Held,
}

/// Which verification-hook flags were SUPPLIED on the command line (each bool =
/// "this flag was given"). Used to reject a hook the chosen mode would silently
/// drop — see `unused_hooks`.
#[derive(Clone, Copy, Default)]
struct SuppliedHooks {
    sel: bool,
    zoom: bool,
    scroll: bool,
    preedit: bool,
    search: bool,
    search_case: bool,
    search_replace: bool,
    capture_size: bool,
    capture_dpi: bool,
    root: bool,
    workspace: bool,
    notes_root: bool,
}

/// Return the supplied hooks that the chosen `kind` does NOT thread into its
/// `Mode` (so it would silently ignore them), in a stable order. Each `Mode`
/// variant carries only a subset of the hooks: the per-frame render hooks
/// (`--sel`/`--zoom`/`--scroll`/`--preedit`/`--search`/`--search-case`) ride
/// `CaptureOpts` and reach ONLY the plain `--screenshot` mode; `--capture-size`/
/// `--capture-dpi` reach screenshot/timeline/held (not motion/windowed); `--root`
/// reaches every mode but motion; `--workspace`/`--notes-root` reach only
/// screenshot + the windowed editor. An empty result means every supplied hook is
/// honored. (Process-global flags — `--theme`/`--caret-mode`/`--measure`/`--page`/
/// `--debug` — compose with every mode and so are never "unused".)
fn unused_hooks(kind: CaptureKind, h: &SuppliedHooks) -> Vec<&'static str> {
    let mut u = Vec::new();
    // Per-frame render hooks: only the plain `--screenshot` mode threads `CaptureOpts`.
    if kind != CaptureKind::Screenshot {
        for (name, set) in [
            ("--sel", h.sel),
            ("--zoom", h.zoom),
            ("--scroll", h.scroll),
            ("--preedit", h.preedit),
            ("--search", h.search),
            ("--search-case", h.search_case),
            ("--search-replace", h.search_replace),
        ] {
            if set {
                u.push(name);
            }
        }
    }
    // Canvas size / dpi: screenshot, timeline, held carry them; motion + windowed don't.
    let canvas_ok = matches!(
        kind,
        CaptureKind::Screenshot | CaptureKind::Timeline | CaptureKind::Held
    );
    if !canvas_ok {
        if h.capture_size {
            u.push("--capture-size");
        }
        if h.capture_dpi {
            u.push("--capture-dpi");
        }
    }
    // Project root: every mode but motion threads it (windowed scopes its project).
    if kind == CaptureKind::Motion && h.root {
        u.push("--root");
    }
    // Workspace / notes-root: only the plain screenshot mode + the windowed editor.
    let ws_ok = matches!(kind, CaptureKind::Screenshot | CaptureKind::Windowed);
    if !ws_ok {
        if h.workspace {
            u.push("--workspace");
        }
        if h.notes_root {
            u.push("--notes-root");
        }
    }
    u
}

/// Reject MORE THAN ONE capture-mode flag. Each capture-mode flag sets the output
/// path AND selects a `Mode` by a fixed precedence, so passing two would silently
/// honor one and drop the other; name them all and refuse instead.
fn ensure_single_capture_mode(modes: &[&str]) -> Result<()> {
    if modes.len() > 1 {
        bail!(
            "conflicting capture-mode flags: {} (choose exactly one)",
            modes.join(", ")
        );
    }
    Ok(())
}

/// Parse a `--capture-held` direction (`left|right|up|down`).
fn parse_held_dir(s: &str) -> Result<capture::HeldDir> {
    match s.to_ascii_lowercase().as_str() {
        "left" | "l" => Ok(capture::HeldDir::Left),
        "right" | "r" => Ok(capture::HeldDir::Right),
        "up" | "u" => Ok(capture::HeldDir::Up),
        "down" | "d" => Ok(capture::HeldDir::Down),
        _ => bail!("bad --capture-held direction {s:?} (want left|right|up|down)"),
    }
}

pub(crate) fn parse_args() -> Result<Mode> {
    let mut args = std::env::args().skip(1);
    let mut out: Option<PathBuf> = None;
    let mut motion = false;
    let mut motion_v = false;
    let mut motion_d = false;
    // Every capture-mode flag seen, in order. More than one is a conflict (each
    // sets `out` + selects a Mode by precedence, so a second would silently win
    // or lose); checked after the loop via `ensure_single_capture_mode`.
    let mut capture_modes: Vec<&str> = Vec::new();
    // `--capture-timeline "<ms,ms,...>"` cumulative step sequence (None = not a
    // timeline capture).
    let mut timeline_steps: Option<Vec<u32>> = None;
    // `--capture-held DIR "<ms,ms,...>"` (None = not a held capture).
    let mut held: Option<(capture::HeldDir, Vec<u32>)> = None;
    // `--capture-size WxH` PHYSICAL canvas dims (None = default 1200x800) and
    // `--capture-dpi N` renderer scale factor (None = 1.0). Both purely additive:
    // absent -> today's byte-identical capture. Threaded onto every capture mode.
    let mut capture_size: Option<(u32, u32)> = None;
    let mut capture_dpi: Option<f32> = None;
    let mut file: Option<PathBuf> = None;
    let mut opts = CaptureOpts::default();
    let mut bench_typing = false;
    let mut bench_perf = false;
    let mut bench_frame = false;
    let mut bench_theme_burst = false;
    let mut bench_zoom_burst = false;
    let mut bench_suite = false;
    // `--bench-baseline <path>`: only meaningful with `--bench-suite` (rejected
    // otherwise below, so it can never be silently dropped).
    let mut bench_baseline: Option<PathBuf> = None;
    // `--keys` replay spec, kept RAW until after the arg loop so it parses THROUGH
    // the loaded config's keybinding overrides (the `--config` flag may appear after
    // `--keys` on the command line). Threaded into whichever screenshot Mode runs.
    let mut keys_spec: Option<String> = None;
    let mut root: Option<PathBuf> = None;
    let mut workspace: Option<PathBuf> = None;
    let mut notes_root: Option<PathBuf> = None;
    // `--config <path>` override for the config file location (also via `$AWL_CONFIG`),
    // so a test config can be pointed at headlessly.
    let mut config_arg: Option<PathBuf> = None;
    // Did the user pass an EXPLICIT sticky-pref flag? A flag always WINS over the
    // config's remembered value (flag > config > default), so the config is applied
    // only where its flag is absent. (Zoom rides `opts.zoom.is_some()` already.)
    let mut theme_flag = false;
    let mut caret_flag = false;
    let mut page_flag = false;
    let mut measure_flag = false;
    // `--wait` (single-instance daemon; `EDITOR=awl --wait` for git): only
    // meaningful for the windowed editor — see `crate::daemon`'s module doc.
    let mut wait_flag = false;
    // `--strict-replay`: the strict replay gate on `--screenshot --keys` — see
    // `crate::replay`'s module doc. Parsed keys go through the STRICT door
    // (unbound chords error) and the replay aborts on Unsupported effects.
    let mut strict_replay = false;
    // `--storyboard <file.toml>` (+ optional `--storyboard-out <dir>`): the
    // scenario runner — always strict, always hermetic. Kept as the raw path
    // here; parsed after the loop (its named file seeds the sandbox).
    let mut storyboard_arg: Option<PathBuf> = None;
    let mut storyboard_out: Option<PathBuf> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bench-typing" => {
                bench_typing = true;
            }
            "--bench-perf" => {
                bench_perf = true;
            }
            "--bench-frame" => {
                bench_frame = true;
            }
            "--bench-theme-burst" => {
                bench_theme_burst = true;
            }
            "--bench-zoom-burst" => {
                bench_zoom_burst = true;
            }
            "--bench-suite" => {
                bench_suite = true;
            }
            "--bench-baseline" => {
                let v = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--bench-baseline requires a path (e.g. benches/baseline.json)")
                })?;
                bench_baseline = Some(PathBuf::from(v));
            }
            "--screenshot" => {
                let p = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--screenshot requires an output path"))?;
                out = Some(PathBuf::from(p));
                capture_modes.push("--screenshot");
            }
            "--screenshot-motion" => {
                let p = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--screenshot-motion requires an output path")
                })?;
                out = Some(PathBuf::from(p));
                motion = true;
                capture_modes.push("--screenshot-motion");
            }
            "--screenshot-motion-v" => {
                let p = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--screenshot-motion-v requires an output path")
                })?;
                out = Some(PathBuf::from(p));
                motion_v = true;
                capture_modes.push("--screenshot-motion-v");
            }
            "--screenshot-motion-d" => {
                let p = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--screenshot-motion-d requires an output path")
                })?;
                out = Some(PathBuf::from(p));
                motion_d = true;
                capture_modes.push("--screenshot-motion-d");
            }
            "--capture-timeline" => {
                // `--capture-timeline "<ms,ms,...>" OUT.png`: a cumulative-ms step
                // sequence FOLLOWED by the output path.
                let spec = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--capture-timeline requires a \"<ms,ms,...>\" step sequence")
                })?;
                let p = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--capture-timeline requires an output path after the steps")
                })?;
                timeline_steps = Some(parse_steps(&spec)?);
                out = Some(PathBuf::from(p));
                capture_modes.push("--capture-timeline");
            }
            "--capture-held" => {
                // `--capture-held DIR "<ms,ms,...>" OUT.png`: a held arrow
                // direction, a cumulative-ms step sequence, then the output path.
                let d = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--capture-held requires a direction (left|right|up|down)")
                })?;
                let spec = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--capture-held requires a \"<ms,ms,...>\" step sequence")
                })?;
                let p = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--capture-held requires an output path after the steps")
                })?;
                held = Some((parse_held_dir(&d)?, parse_steps(&spec)?));
                out = Some(PathBuf::from(p));
                capture_modes.push("--capture-held");
            }
            "--storyboard" => {
                let p = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--storyboard requires a storyboard .toml path")
                })?;
                storyboard_arg = Some(PathBuf::from(p));
                capture_modes.push("--storyboard");
            }
            "--storyboard-out" => {
                let p = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--storyboard-out requires an output directory")
                })?;
                storyboard_out = Some(PathBuf::from(p));
            }
            "--sel" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--sel requires L0:C0-L1:C1"))?;
                opts.selection = Some(parse_sel(&v)?);
            }
            "--zoom" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--zoom requires a factor (e.g. 1.6)"))?;
                opts.zoom = Some(parse_zoom(&v)?);
            }
            "--capture-size" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--capture-size requires WxH (e.g. 2400x1600)"))?;
                capture_size = Some(parse_size(&v)?);
            }
            "--capture-dpi" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--capture-dpi requires a factor (e.g. 2.0)"))?;
                capture_dpi = Some(parse_dpi(&v)?);
            }
            "--scroll" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--scroll requires a line count"))?;
                opts.scroll =
                    Some(v.parse().map_err(|_| anyhow::anyhow!("bad --scroll {v:?}"))?);
            }
            "--preedit" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--preedit requires a string"))?;
                opts.preedit = Some(v);
            }
            "--search" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--search requires a query"))?;
                opts.search = Some(v);
            }
            "--search-case" => {
                opts.search_case_sensitive = true;
            }
            "--search-replace" => {
                // Reveal the labeled REPLACE row + the key-hint line on the panel (the
                // fresh Cmd-R open state: find field focused, empty replacement). A
                // `--keys` replay can drive the panel further — typing the replacement,
                // replacing — through the shared search-key seam (`search::keys`).
                opts.search_replace_active = true;
            }
            "--theme" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--theme requires a world name"))?;
                // Set the process-global active theme NOW so it composes with any
                // capture mode (the headless render reads the active theme). Order
                // among flags is irrelevant since the active theme is global.
                theme::set_active_by_name(&v).ok_or_else(|| {
                    let names: Vec<&str> = theme::THEMES.iter().map(|t| t.name).collect();
                    anyhow::anyhow!("unknown --theme {v:?}; choose one of {}", names.join(", "))
                })?;
                theme_flag = true;
            }
            "--caret-mode" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--caret-mode requires 'block' or 'morph'"))?;
                // Pin the process-global caret mode so the headless render is
                // deterministic and verifiable. 'auto' clears any override and
                // falls back to the font-derived default (Block on mono).
                match v.to_ascii_lowercase().as_str() {
                    "block" => caret::set_mode(caret::CaretMode::Block),
                    "morph" => caret::set_mode(caret::CaretMode::Morph),
                    "ibeam" => caret::set_mode(caret::CaretMode::Ibeam),
                    "auto" => {} // leave the font-derived default in effect
                    _ => bail!("unknown --caret-mode {v:?}; choose block, morph, ibeam, or auto"),
                }
                caret_flag = true;
            }
            "--measure" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--measure requires a char count"))?;
                let n = parse_measure(&v)?;
                // Setting a measure implies page mode ON (so the narrow column +
                // gradient margins are visible in the capture).
                page::set_measure(n);
                page::set_page_on(true);
                page_flag = true;
                measure_flag = true;
            }
            "--page" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--page requires 'on' or 'off'"))?;
                match v.to_ascii_lowercase().as_str() {
                    "on" => page::set_page_on(true),
                    "off" => page::set_page_on(false),
                    _ => bail!("unknown --page {v:?}; choose on or off"),
                }
                page_flag = true;
            }
            "--debug" => {
                // Opt-in DEBUG panel. Sets the process-global so it composes with any
                // capture mode; the frametime line shows a FIXED placeholder with no
                // live clock (deterministic), while the rest of the panel is a pure
                // function of the view state — so an explicit `--debug` capture stays
                // stable and a plain capture (panel OFF) is byte-identical.
                debug::set_debug_on(true);
            }
            "--hud" => {
                // Summon the HELD STATS HUD for the capture. Sets the process-global
                // so it composes with any capture mode; the clock / file-date fields
                // render FIXED placeholders (no live clock), so an explicit `--hud`
                // capture is deterministic while a plain capture (HUD released) is
                // byte-identical. The live window summons it by HOLDING the binding
                // (Option-Cmd-I) instead.
                hud::set_held(true);
            }
            "--menu-bar" => {
                // Show the WEB/LINUX MENU BAR for the capture (mirrors `--hud`). Sets
                // the process-global so it composes with any capture mode; the bar is
                // pure geometry + theme (no clock), so an explicit `--menu-bar` capture
                // is deterministic while a plain capture (default OFF on macOS) is
                // byte-identical. On web/Linux the live app shows it by default.
                crate::menubar::set_menu_bar_on(true);
            }
            "--menu-open" => {
                // Show the menu bar AND drop the dropdown for menu index N (0 = the App
                // menu), so a capture can exercise the open-dropdown render + sidecar
                // `menubar.open_menu` deterministically. A bad/absent index just shows
                // the closed bar.
                crate::menubar::set_menu_bar_on(true);
                if let Some(n) = args.next().and_then(|s| s.parse::<usize>().ok()) {
                    crate::menubar::set_open(Some(n));
                }
            }
            "--lifetime" => {
                // Summon the LIFETIME STATS card for the capture (mirrors `--hud`).
                // Sets the process-global so it composes with any capture mode; the
                // odometer figures render FIXED "—" placeholders (no live persisted
                // store), so an explicit `--lifetime` capture is deterministic while a
                // plain capture (card closed) is byte-identical. The live app summons
                // it via the palette "Lifetime stats" command instead.
                lifetime::set_open(true);
            }
            "--peek" => {
                // Summon the HOLD-⌘ SHORTCUT PEEK for the capture (mirrors `--hud` /
                // `--lifetime`). Sets the process-global so it composes with any capture
                // mode; the card renders the curated STARTER SIX (no live ledger to
                // personalize from), so an explicit `--peek` capture is deterministic and
                // byte-stable while a plain capture (not summoned) is byte-identical. The
                // live app summons it by HOLDING the active convention's bare arming
                // modifier (⌘ on Mac, Ctrl on Linux — `peek::arming_modifier`) for ~600ms
                // instead.
                crate::peek::set_open(true);
            }
            "--whichkey" => {
                // Summon the WHICH-KEY continuation panel for the capture. Sets the
                // process-global so it composes with any capture mode; `run.rs` then
                // derives the `C-x` rows from the catalog + config and renders the
                // SETTLED summoned panel (the live 500ms pause is windowed). A plain
                // capture (unset) draws no panel and stays byte-identical. The live
                // window summons it by pressing `C-x` and PAUSING instead.
                whichkey::set_force_shown(true);
            }
            "--keys" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--keys requires a key-spec string"))?;
                keys_spec = Some(v);
            }
            "--strict-replay" => {
                strict_replay = true;
            }
            "--config" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--config requires a path"))?;
                config_arg = Some(PathBuf::from(v));
            }
            "--root" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--root requires a directory"))?;
                root = Some(PathBuf::from(v));
            }
            "--workspace" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--workspace requires a directory"))?;
                workspace = Some(PathBuf::from(v));
            }
            "--notes-root" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--notes-root requires a directory"))?;
                notes_root = Some(PathBuf::from(v));
            }
            "--wait" => {
                wait_flag = true;
            }
            "-h" | "--help" => {
                println!(
                    "awl [file]\n\
                     awl --screenshot OUT.png [file]         caret at rest (rounded square)\n\
                     awl --screenshot-motion OUT.png [file]  caret mid-glide (centred trailing streak)\n\
                     awl --screenshot-motion-v OUT.png [file] caret mid-glide vertical (left-edge bar)\n\
                     awl --screenshot-motion-d OUT.png [file] caret mid-glide diagonal (slanted tracer)\n\
                     awl --capture-timeline \"0,16,50,150\" OUT.png [file]  deterministic timeline: step the caret glide by injected ms, frame per step (OUT.t<ms>.png)\n\
                     awl --capture-held DIR \"0,30,60,90\" OUT.png [file]  deterministic HELD arrow (DIR=left|right|up|down): re-target one char/line per step (held=true), frame per step with trail geometry\n\
                     \n\
                     verification hooks (compose with --screenshot):\n\
                     \x20 --sel L0:C0-L1:C1   selection highlight from (l0,c0)..(l1,c1)\n\
                     \x20 --zoom F            zoom factor (0.5..3.0)\n\
                     \x20 --scroll N          scroll N visual rows off the top\n\
                     \x20 --preedit STR       render STR as an IME preedit at the caret\n\
                     \x20 --search STR        open isearch panel for STR + highlight hits\n\
                     \x20 --search-case       make --search case-sensitive\n\
                     \x20 --theme NAME        set the active color theme (Tawny, Potoroo, Gumtree, Bilby, Saltpan, Quokka, Undertow, Outback, Mangrove, Firetail)\n\
                     \x20 --caret-mode MODE   caret look: block, morph, ibeam, or auto (default: mono->block, proportional->morph)\n\
                     \x20 --capture-size WxH  physical canvas size for the capture (default 1200x800)\n\
                     \x20 --capture-dpi N      renderer scale factor (default 1.0); WxH at dpi N == (W/N)x(H/N) logical retina window\n\
                     \x20 --measure N         page-mode column width in chars (default 80; implies --page on)\n\
                     \x20 --page on|off       page mode: centered column (on, default) vs edge-to-edge (off)\n\
                     \x20 --debug             DEBUG: draw the dim top-left dev panel — frametime/zoom/viewport/cursor/theme/md+syn (OFF by default; frametime is a fixed placeholder in a headless capture)\n\
                     \x20 --hud               summon the HELD stats HUD (live: hold Option-Cmd-I; clock/file-date fields are fixed placeholders in a capture)\n\
                     \x20 --menu-bar          show the web/Linux MENU BAR (default on web/Linux, off on macOS which has the native bar); --menu-open N drops menu N's dropdown\n\
                     \x20 --peek              summon the HOLD-⌘ shortcut peek (live: hold the convention's bare arming modifier — ⌘ on Mac, Ctrl on Linux — ~600ms; a capture shows the curated starter six)\n\
                     \x20 --whichkey          summon the WHICH-KEY panel: the C-x prefix's follow-up keys (live: press C-x and pause ~500ms)\n\
                     \x20 --notes-root DIR    quick-notes home for C-x n / C-x m (default ~/notes)\n\
                     \x20 --config PATH       load settings from PATH (default ~/.config/awl/config.toml)\n\
                     \x20 --wait              windowed editor only: single-instance daemon — hand `file` to an already-running awl and block until C-x # finishes it (EDITOR=awl --wait for git)\n\
                     \x20 --keys \"SPEC\"        replay emacs chords (e.g. \"C-n C-n M->\") then capture\n\
                     \x20 --strict-replay     with --screenshot --keys: abort (naming the offender) on an unbound chord, a live-only effect the replay can't perform, or a missing layout oracle; runs HERMETIC (an in-memory fs seeded from the named file + --config — a replayed save never touches the real file, the user's own config/notes/history are never read or written)\n\
                     \x20 --storyboard TOML   run a scenario storyboard (press/type/pause/run_for/expect steps — see scenarios/): strict + hermetic, emitting per-step PNG+JSON, deterministic film frames, a byte-stable trace.json, and (with ffmpeg on PATH) film.webm/film.mp4\n\
                     \x20 --storyboard-out DIR where the storyboard run's artifacts land (default: <storyboard>.run/ beside the .toml)"
                );
                std::process::exit(0);
            }
            s if s.starts_with("--") => bail!("unknown flag: {s}"),
            s => file = Some(PathBuf::from(s)),
        }
    }

    if bench_suite {
        return Ok(Mode::BenchSuite { baseline: bench_baseline });
    }
    if bench_baseline.is_some() {
        bail!("--bench-baseline requires --bench-suite");
    }
    if bench_typing {
        return Ok(Mode::BenchTyping);
    }
    if bench_perf {
        return Ok(Mode::BenchPerf);
    }
    if bench_frame {
        return Ok(Mode::BenchFrame);
    }
    if bench_theme_burst {
        return Ok(Mode::BenchThemeBurst);
    }
    if bench_zoom_burst {
        return Ok(Mode::BenchZoomBurst);
    }
    // CLI VALIDATION (error paths only — valid runs are unaffected).
    // 1) At most ONE capture-mode flag. With more than one, the Mode chosen below
    //    would silently follow a precedence and drop the rest; refuse instead.
    ensure_single_capture_mode(&capture_modes)?;
    // 2) Reject verification hooks the chosen mode would silently ignore. After the
    //    single-mode check above at most one mode category is active, so this
    //    mirrors the Mode construction's precedence (held > timeline > motion >
    //    screenshot; no output = windowed).
    let kind = if out.is_none() {
        CaptureKind::Windowed
    } else if held.is_some() {
        CaptureKind::Held
    } else if timeline_steps.is_some() {
        CaptureKind::Timeline
    } else if motion || motion_v || motion_d {
        CaptureKind::Motion
    } else {
        CaptureKind::Screenshot
    };
    let supplied = SuppliedHooks {
        sel: opts.selection.is_some(),
        zoom: opts.zoom.is_some(),
        scroll: opts.scroll.is_some(),
        preedit: opts.preedit.is_some(),
        search: opts.search.is_some(),
        search_case: opts.search_case_sensitive,
        search_replace: opts.search_replace_active,
        capture_size: capture_size.is_some(),
        capture_dpi: capture_dpi.is_some(),
        root: root.is_some(),
        workspace: workspace.is_some(),
        notes_root: notes_root.is_some(),
    };
    let unused = unused_hooks(kind, &supplied);
    if !unused.is_empty() {
        bail!(
            "{} not honored by the chosen capture mode",
            unused.join(", ")
        );
    }
    // `--strict-replay` gates a `--keys` replay, and only the plain
    // `--screenshot` mode threads the strict engine (the motion/timeline/held
    // variants stay permissive one-offs); refuse the combinations that would
    // silently ignore it. Validated BEFORE the hermetic install below so a
    // refused flag combination never swaps the process filesystem first.
    if strict_replay {
        if keys_spec.is_none() {
            bail!("--strict-replay requires --keys (there is no replay to be strict about)");
        }
        if kind != CaptureKind::Screenshot {
            bail!("--strict-replay only applies to --screenshot (not motion/timeline/held captures)");
        }
    }
    // `--storyboard` drives its own input/document; refuse the flags it would
    // silently ignore, then parse the scenario file NOW (std::fs — the one
    // boundary crossing before the sandbox exists) so its named document can
    // seed the hermetic sandbox below, exactly like `--strict-replay`'s file.
    if storyboard_arg.is_some() {
        if keys_spec.is_some() {
            bail!("--storyboard drives its own steps; --keys does not apply");
        }
        if file.is_some() {
            bail!("--storyboard takes its document from the storyboard file; drop the file argument");
        }
        if wait_flag {
            bail!("--wait only applies to the windowed editor (no capture mode)");
        }
        if strict_replay {
            bail!("--storyboard is always strict; --strict-replay does not apply");
        }
    } else if storyboard_out.is_some() {
        bail!("--storyboard-out requires --storyboard");
    }
    let storyboard: Option<(crate::storyboard::Storyboard, PathBuf)> = match &storyboard_arg {
        Some(p) => {
            let src = std::fs::read_to_string(p)
                .map_err(|e| anyhow::anyhow!("reading storyboard {}: {e}", p.display()))?;
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "storyboard".to_string());
            let board = crate::storyboard::parse(&src, &stem)
                .map_err(|e| e.context(format!("parsing storyboard {}", p.display())))?;
            Some((board, p.clone()))
        }
        None => None,
    };
    // The board's document, resolved against the storyboard file's own directory
    // (so a checked-in `scenarios/demo.toml` names its fixture as `demo.md`).
    let storyboard_file: Option<PathBuf> = storyboard.as_ref().and_then(|(b, p)| {
        b.file.as_ref().map(|f| match p.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir.join(f),
            _ => PathBuf::from(f),
        })
    });
    // HERMETIC SCENARIO FILESYSTEM — the ONE production door (`crate::scenario`'s
    // module doc is the contract): a strict (scenario) run swaps the process fs
    // to an in-memory sandbox seeded from exactly the CLI-named inputs BEFORE
    // the config loads, so the load below — and every fs consumer after it —
    // reads the sandbox, never the user's real files. The legacy permissive
    // paths never install it (real-fs behavior kept byte-for-byte). A
    // storyboard run is hermetic UNCONDITIONALLY, through its own door (the
    // same sandbox, plus the document's parent-directory marker).
    #[cfg(not(target_arch = "wasm32"))]
    if strict_replay {
        crate::scenario::install_hermetic_fs(file.as_deref(), config_arg.as_deref(), root.as_deref());
    }
    #[cfg(not(target_arch = "wasm32"))]
    if storyboard.is_some() {
        crate::scenario::install_hermetic_fs(
            storyboard_file.as_deref(),
            config_arg.as_deref(),
            root.as_deref(),
        );
    }
    // Load the persistent CONFIG (flag/$AWL_CONFIG/XDG path — resolved inside
    // the hermetic sandbox for a strict run, where an un-seeded path degrades
    // to pure defaults). Absent file = all defaults, so this is purely
    // additive. Parse `--keys` THROUGH the config's keybinding overrides so a
    // replay exercises rebound chords.
    let config = Config::load(config::config_path(config_arg));
    // STICKY PREFERENCES: restore the remembered THEME / PAGE / CARET onto the
    // process-globals (the same globals the flags set), honouring flag > config —
    // a config value is applied only where its flag was ABSENT, so an explicit flag
    // still wins. These globals serve BOTH the windowed editor and the headless
    // capture, so a `--config` with theme/page/caret set produces a capture reflecting
    // them. ZOOM is per-instance (not a global): the capture folds it into `opts.zoom`
    // below and the windowed `App::new` reads `config.zoom`.
    //
    // The page-width MEASURE is now a per-KIND sticky pref (`page_width_prose` /
    // `page_width_code`) — resolve the STARTING buffer's class from the launch
    // `file` argument (no `Buffer` exists yet here) so the very first frame reads
    // the right one; a later buffer switch re-resolves against whichever kind is
    // then active (`App::sync_page_measure` / the headless `--keys` Goto switch).
    let initial_page_class =
        page::PageClass::of_path(storyboard_file.as_deref().or(file.as_deref()));
    config.apply_sticky_globals(theme_flag, page_flag, caret_flag, measure_flag, initial_page_class);
    // `--keys` only makes sense with a capture mode (it mutates the buffer for a
    // one-frame capture); refuse it for the windowed editor where live typing is
    // the input path.
    if keys_spec.is_some() && out.is_none() {
        bail!("--keys requires a capture mode (e.g. --screenshot OUT.png)");
    }
    // `--wait` is a windowed-editor-only concern (the single-instance daemon's
    // handoff); a capture mode has no daemon to wait on (see `crate::daemon`'s
    // CAPTURE GATE).
    if wait_flag && out.is_some() {
        bail!("--wait only applies to the windowed editor (no capture mode)");
    }
    // STRUCTURAL parse only — a garbled token still errors right here. The
    // chords stay UNRESOLVED: the replay loop resolves them one press at a time
    // through `km` below (`keyspec::ChordResolver`), interleaved with the
    // search guard, so a chord an open search panel consumes never reaches the
    // keymap — and the STRICT unbound/dangling-prefix refusals fire there,
    // where "was this chord for the keymap at all" is actually decidable.
    let keys: Vec<keyspec::Chord> = match &keys_spec {
        Some(spec) => keyspec::parse_chords(spec)?,
        None => Vec::new(),
    };
    // The keymap every capture replay resolves through: config `[keys]`
    // rebinds + the `linux_keep_emacs` door, exactly what live `App::new` builds.
    let km = KeymapState::with_overrides_and_keep(&config.keys, &config.effective_linux_keep());
    // PRECEDENCE: explicit flag > config > built-in default. Fold the config value in
    // BEHIND the flag (the flag wins via `.or`) before the existing resolvers add the
    // built-in default. The Windowed path keeps the RAW flag + config so a live reload
    // can re-fold; capture modes fold here (one-shot, no reload).
    let notes_root_resolved = resolve_notes_root(&notes_root.clone().or_else(|| config.notes_root.clone()));
    let workspace_folded = workspace.clone().or_else(|| config.workspace.clone());
    // Thread the capture canvas size + dpi onto the screenshot opts (timeline/held
    // carry them on their Mode variants). Absent flags -> None -> byte-stable default.
    opts.canvas = capture_size;
    opts.dpi = capture_dpi;
    // STICKY ZOOM (capture): fold the remembered zoom in BEHIND `--zoom` (the flag
    // wins). The windowed editor applies `config.zoom` in `App::new` instead.
    if opts.zoom.is_none() {
        opts.zoom = config.zoom;
    }
    // STORYBOARD mode: everything below the sandbox install composes normally
    // (config from the sandbox, km with its rebinds); the run outputs land in
    // `--storyboard-out`, defaulting to `<storyboard>.run/` beside the board.
    if let Some((board, board_path)) = storyboard {
        let out_dir = storyboard_out.unwrap_or_else(|| board_path.with_extension("run"));
        return Ok(Mode::Storyboard {
            board,
            file: storyboard_file,
            out_dir,
            root,
            workspace: workspace_folded,
            notes_root: notes_root_resolved,
            config,
            km,
        });
    }
    Ok(match out {
        Some(out) if held.is_some() => {
            let (dir, steps) = held.unwrap();
            Mode::CaptureHeld {
                out,
                file,
                keys,
                km,
                dir,
                steps,
                root,
                canvas: capture_size,
                dpi: capture_dpi,
            }
        }
        Some(out) if timeline_steps.is_some() => Mode::CaptureTimeline {
            out,
            file,
            keys,
            km,
            steps: timeline_steps.unwrap(),
            root,
            canvas: capture_size,
            dpi: capture_dpi,
        },
        Some(out) if motion_d => Mode::ScreenshotMotionDiagonal { out, file, keys, km },
        Some(out) if motion_v => Mode::ScreenshotMotionVertical { out, file, keys, km },
        Some(out) if motion => Mode::ScreenshotMotion { out, file, keys, km },
        Some(out) => Mode::Screenshot {
            out,
            file,
            opts,
            keys,
            km,
            root,
            workspace: workspace_folded,
            notes_root: notes_root_resolved,
            config,
            strict: strict_replay,
        },
        None => Mode::Windowed {
            file,
            root,
            workspace,
            notes_root,
            config,
            wait: wait_flag,
        },
    })
}

/// Resolve the NOTES ROOT: explicit `--notes-root`, else `~/notes` (`$HOME/notes`),
/// else `./notes` if HOME is unset. The directory is created lazily on first use
/// (C-x n / first note save), so it need not exist yet.
pub(crate) fn resolve_notes_root(notes_root: &Option<PathBuf>) -> PathBuf {
    if let Some(n) = notes_root {
        return n.clone();
    }
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join("notes"),
        None => PathBuf::from("notes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sel_orders_endpoints_and_rejects_malformed() {
        // Endpoints are ordered earliest-first regardless of input order.
        assert_eq!(parse_sel("0:0-2:3").unwrap(), ((0, 0), (2, 3)));
        assert_eq!(parse_sel("2:3-0:0").unwrap(), ((0, 0), (2, 3)));
        assert_eq!(parse_sel(" 1:2 - 1:5 ").unwrap(), ((1, 2), (1, 5)));
        // Malformed: missing `-`, missing `:`, non-numeric.
        assert!(parse_sel("0:0").is_err());
        assert!(parse_sel("00-23").is_err());
        assert!(parse_sel("a:b-c:d").is_err());
    }

    #[test]
    fn parse_steps_reads_ms_and_rejects_junk() {
        assert_eq!(parse_steps("0,16,50,150").unwrap(), vec![0, 16, 50, 150]);
        // Whitespace + trailing/empty entries are tolerated.
        assert_eq!(parse_steps(" 0 , 30 ,").unwrap(), vec![0, 30]);
        // Empty / all-blank / non-numeric are errors.
        assert!(parse_steps("").is_err());
        assert!(parse_steps("  ,  ").is_err());
        assert!(parse_steps("0,x,2").is_err());
    }

    #[test]
    fn parse_size_accepts_both_separators_and_rejects_zero() {
        assert_eq!(parse_size("2400x1600").unwrap(), (2400, 1600));
        assert_eq!(parse_size("800X600").unwrap(), (800, 600));
        // Missing separator, zero dimension, non-numeric are errors.
        assert!(parse_size("1200").is_err());
        assert!(parse_size("0x600").is_err());
        assert!(parse_size("800x0").is_err());
        assert!(parse_size("axb").is_err());
    }

    #[test]
    fn parse_held_dir_accepts_aliases_and_rejects_bad() {
        assert!(parse_held_dir("left").unwrap() == capture::HeldDir::Left);
        assert!(parse_held_dir("L").unwrap() == capture::HeldDir::Left);
        assert!(parse_held_dir("RIGHT").unwrap() == capture::HeldDir::Right);
        assert!(parse_held_dir("u").unwrap() == capture::HeldDir::Up);
        assert!(parse_held_dir("Down").unwrap() == capture::HeldDir::Down);
        assert!(parse_held_dir("sideways").is_err());
        assert!(parse_held_dir("").is_err());
    }

    #[test]
    fn parse_dpi_requires_finite_positive() {
        assert_eq!(parse_dpi("2.0").unwrap(), 2.0);
        assert_eq!(parse_dpi(" 1 ").unwrap(), 1.0);
        // Zero, negative, non-finite, and non-numeric are all errors (mirrors
        // parse_size's non-zero guard).
        assert!(parse_dpi("0").is_err());
        assert!(parse_dpi("-1.5").is_err());
        assert!(parse_dpi("inf").is_err());
        assert!(parse_dpi("nan").is_err());
        assert!(parse_dpi("x").is_err());
    }

    #[test]
    fn parse_zoom_requires_finite_positive() {
        assert_eq!(parse_zoom("1.6").unwrap(), 1.6);
        assert_eq!(parse_zoom(" 0.5 ").unwrap(), 0.5);
        // Zero, negative, non-finite, and non-numeric are all errors (mirrors
        // parse_dpi's guard) — a NaN factor would otherwise poison every
        // zoom-derived metric downstream.
        assert!(parse_zoom("0").is_err());
        assert!(parse_zoom("-1").is_err());
        assert!(parse_zoom("inf").is_err());
        assert!(parse_zoom("nan").is_err());
        assert!(parse_zoom("x").is_err());
    }

    #[test]
    fn clamp_zoom_never_returns_non_finite() {
        // The LAST line of defence behind the --zoom / config seams above:
        // `render::clamp_zoom` must yield a finite in-range factor for ANY input.
        // (Tested here beside the zoom-flag seam; render/tests/geometry.rs owns
        // the geometry suite.) NaN — the propagating poison — falls back to the 1.0
        // default; ±inf saturates through the ordinary clamp.
        use crate::render::{clamp_zoom, ZOOM_MAX, ZOOM_MIN};
        for z in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -7.0, 1e30] {
            let c = clamp_zoom(z);
            assert!(
                c.is_finite() && (ZOOM_MIN..=ZOOM_MAX).contains(&c),
                "clamp_zoom({z}) -> {c} must be finite in [{ZOOM_MIN}, {ZOOM_MAX}]"
            );
        }
        assert_eq!(clamp_zoom(f32::NAN), 1.0, "NaN falls back to the default");
        assert_eq!(clamp_zoom(f32::INFINITY), ZOOM_MAX, "+inf saturates high");
        assert_eq!(clamp_zoom(f32::NEG_INFINITY), ZOOM_MIN, "-inf saturates low");
        // A normal factor still step-rounds + clamps exactly as before.
        assert!((clamp_zoom(1.234) - 1.2).abs() < 1e-5, "step rounding unchanged");
        assert_eq!(clamp_zoom(9.0), ZOOM_MAX);
        assert_eq!(clamp_zoom(0.0), ZOOM_MIN);
    }

    #[test]
    fn parse_measure_requires_positive() {
        assert_eq!(parse_measure("80").unwrap(), 80);
        assert_eq!(parse_measure(" 40 ").unwrap(), 40);
        // Zero and non-numeric are errors (mirrors parse_size's non-zero guard).
        assert!(parse_measure("0").is_err());
        assert!(parse_measure("-1").is_err());
        assert!(parse_measure("x").is_err());
    }

    #[test]
    fn single_capture_mode_rejects_conflicts() {
        // Zero or one capture-mode flag is fine.
        assert!(ensure_single_capture_mode(&[]).is_ok());
        assert!(ensure_single_capture_mode(&["--screenshot"]).is_ok());
        // Two distinct modes — or the same flag twice — is a conflict.
        assert!(ensure_single_capture_mode(&["--screenshot", "--capture-held"]).is_err());
        assert!(ensure_single_capture_mode(&["--screenshot", "--screenshot"]).is_err());
        // The error names every conflicting flag.
        let msg = ensure_single_capture_mode(&["--screenshot", "--screenshot-motion"])
            .unwrap_err()
            .to_string();
        assert!(msg.contains("--screenshot") && msg.contains("--screenshot-motion"));
    }

    #[test]
    fn unused_hooks_flags_only_what_a_mode_drops() {
        // A plain screenshot honors every hook → nothing unused.
        let all = SuppliedHooks {
            sel: true,
            zoom: true,
            scroll: true,
            preedit: true,
            search: true,
            search_case: true,
            search_replace: true,
            capture_size: true,
            capture_dpi: true,
            root: true,
            workspace: true,
            notes_root: true,
        };
        assert!(unused_hooks(CaptureKind::Screenshot, &all).is_empty());

        // Motion threads only keys/file: every other hook is dropped.
        let motion = unused_hooks(CaptureKind::Motion, &all);
        for f in [
            "--sel",
            "--zoom",
            "--scroll",
            "--preedit",
            "--search",
            "--search-case",
            "--search-replace",
            "--capture-size",
            "--capture-dpi",
            "--root",
            "--workspace",
            "--notes-root",
        ] {
            assert!(motion.contains(&f), "motion should drop {f}");
        }

        // Timeline / held carry root + canvas/dpi but still drop the per-frame
        // render hooks and workspace/notes-root.
        for kind in [CaptureKind::Timeline, CaptureKind::Held] {
            let u = unused_hooks(kind, &all);
            assert!(u.contains(&"--sel") && u.contains(&"--search-case"));
            assert!(u.contains(&"--workspace") && u.contains(&"--notes-root"));
            assert!(!u.contains(&"--root"));
            assert!(!u.contains(&"--capture-size") && !u.contains(&"--capture-dpi"));
        }

        // The windowed editor honors project context but not capture hooks.
        let win = unused_hooks(CaptureKind::Windowed, &all);
        assert!(win.contains(&"--sel") && win.contains(&"--capture-size"));
        assert!(!win.contains(&"--root"));
        assert!(!win.contains(&"--workspace") && !win.contains(&"--notes-root"));

        // Nothing supplied → nothing unused, for every mode.
        let none = SuppliedHooks::default();
        for kind in [
            CaptureKind::Windowed,
            CaptureKind::Screenshot,
            CaptureKind::Motion,
            CaptureKind::Timeline,
            CaptureKind::Held,
        ] {
            assert!(unused_hooks(kind, &none).is_empty());
        }
    }
}
