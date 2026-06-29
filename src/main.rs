//! awl — a fast native editor (skeleton stage).
//!
//! Usage:
//!   awl [file]                              open windowed editor (file optional)
//!   awl --screenshot OUT.png [file]         headless: one frame, caret at rest (rounded square)
//!   awl --screenshot-motion OUT.png [file]  headless: one frame, caret mid-glide (trailing underline)
//!
//! Deterministic verification hooks (compose with --screenshot):
//!   --sel L0:C0-L1:C1   draw a selection highlight from (line0,col0)..(line1,col1)
//!   --zoom F            render at zoom factor F (e.g. 1.6); clamped to [0.5,3.0]
//!   --scroll N          scroll N VISUAL rows off the top (free scroll, clamped)
//!   --preedit STR       render STR as an IME preedit (underlined) at the caret
//!   --theme NAME        set the active color theme/world before capture (e.g. Quokka)
//!   --caret-mode MODE   caret look: block | morph | auto (default: font-derived)
//!   --keys "SPEC"       replay a space-separated emacs key-spec against the freshly
//!                       loaded buffer THROUGH THE REAL KEYMAP, then capture the
//!                       post-replay editor state (e.g. --keys "C-n C-n M->")

mod actions;
mod app;
mod background;
mod bench;
mod buffer;
mod capture;
mod caret;
mod caret_glyph;
mod commands;
mod config;
mod focus;
mod fps;
mod fuzzy;
mod index;
mod keymap;
mod keyspec;
mod markdown;
mod overlay;
mod page;
mod project;
mod render;
mod search;
mod selection;
mod spell;
mod spellunderline;
mod syntax;
mod theme;

use std::path::PathBuf;

use anyhow::{bail, Result};

use crate::buffer::Buffer;
use crate::capture::CaptureOpts;
use crate::config::Config;
use crate::keymap::Action;

enum Mode {
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
        keys: Vec<Action>,
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
    },
    /// Deterministic one-frame capture of a caret MID-GLIDE (dropped to the
    /// baseline and stretched into a trailing underline streak), so the temporal
    /// effect is inspectable from a still.
    ScreenshotMotion {
        out: PathBuf,
        file: Option<PathBuf>,
        keys: Vec<Action>,
    },
    /// Like [`Mode::ScreenshotMotion`] but a VERTICAL glide: the caret slid to a
    /// thin bar on the cell's left edge, trailing up the lines it passed.
    ScreenshotMotionVertical {
        out: PathBuf,
        file: Option<PathBuf>,
        keys: Vec<Action>,
    },
    /// Like [`Mode::ScreenshotMotion`] but a DIAGONAL glide (different row AND
    /// column): the trail is a true slanted tracer from source to target.
    ScreenshotMotionDiagonal {
        out: PathBuf,
        file: Option<PathBuf>,
        keys: Vec<Action>,
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
        keys: Vec<Action>,
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
        keys: Vec<Action>,
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
    /// Hidden performance harness: time the per-keystroke update path (append a
    /// char -> reshape) on documents of 100/1000/5000 lines, BEFORE (whole-buffer
    /// reshape) vs AFTER (incremental), and print the numbers. Opens no window.
    BenchTyping,
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
/// `--focus`/`--fps` — compose with every mode and so are never "unused".)
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

fn parse_args() -> Result<Mode> {
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

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bench-typing" => {
                bench_typing = true;
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
                opts.zoom = Some(v.parse().map_err(|_| anyhow::anyhow!("bad --zoom {v:?}"))?);
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
            }
            "--fps" => {
                // Opt-in DEBUG frame counter. Sets the process-global so it composes
                // with any capture mode; with no live clock the headless render shows
                // a FIXED placeholder (deterministic), so an explicit `--fps` capture
                // stays stable while a plain capture (counter OFF) is byte-identical.
                fps::set_fps_on(true);
            }
            "--focus" => {
                let v = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--focus requires 'off', 'paragraph', or 'sentence'")
                })?;
                // Pin the process-global focus mode so the headless render dims the
                // active unit deterministically (settled state, no clock).
                match v.to_ascii_lowercase().as_str() {
                    "off" => focus::set_mode(focus::FocusMode::Off),
                    "paragraph" | "para" => focus::set_mode(focus::FocusMode::Paragraph),
                    "sentence" => focus::set_mode(focus::FocusMode::Sentence),
                    _ => bail!("unknown --focus {v:?}; choose off, paragraph, or sentence"),
                }
            }
            "--keys" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--keys requires a key-spec string"))?;
                keys_spec = Some(v);
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
                     \x20 --theme NAME        set the active color theme (Tawny, Potoroo, Gumtree, Bilby, Saltpan, Quokka, Undertow, Outback)\n\
                     \x20 --caret-mode MODE   caret look: block, morph, ibeam, or auto (default: mono->block, proportional->morph)\n\
                     \x20 --capture-size WxH  physical canvas size for the capture (default 1200x800)\n\
                     \x20 --capture-dpi N      renderer scale factor (default 1.0); WxH at dpi N == (W/N)x(H/N) logical retina window\n\
                     \x20 --measure N         page-mode column width in chars (default 80; implies --page on)\n\
                     \x20 --page on|off       page mode: centered column (on, default) vs edge-to-edge (off)\n\
                     \x20 --fps               DEBUG: draw the dim corner frame counter (OFF by default; fixed placeholder in a headless capture)\n\
                     \x20 --notes-root DIR    quick-notes home for C-x n / C-x m (default ~/notes)\n\
                     \x20 --config PATH       load settings from PATH (default ~/.config/awl/config.toml)\n\
                     \x20 --keys \"SPEC\"        replay emacs chords (e.g. \"C-n C-n M->\") then capture"
                );
                std::process::exit(0);
            }
            s if s.starts_with("--") => bail!("unknown flag: {s}"),
            s => file = Some(PathBuf::from(s)),
        }
    }

    if bench_typing {
        return Ok(Mode::BenchTyping);
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
    // Load the persistent CONFIG (flag/$AWL_CONFIG/XDG path). Absent file = all
    // defaults, so this is purely additive. Parse `--keys` THROUGH the config's
    // keybinding overrides so a replay exercises rebound chords.
    let config = Config::load(config::config_path(config_arg));
    // `--keys` only makes sense with a capture mode (it mutates the buffer for a
    // one-frame capture); refuse it for the windowed editor where live typing is
    // the input path.
    if keys_spec.is_some() && out.is_none() {
        bail!("--keys requires a capture mode (e.g. --screenshot OUT.png)");
    }
    let keys: Vec<Action> = match &keys_spec {
        Some(spec) => keyspec::parse_keys_with(spec, &config)?,
        None => Vec::new(),
    };
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
    Ok(match out {
        Some(out) if held.is_some() => {
            let (dir, steps) = held.unwrap();
            Mode::CaptureHeld {
                out,
                file,
                keys,
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
            steps: timeline_steps.unwrap(),
            root,
            canvas: capture_size,
            dpi: capture_dpi,
        },
        Some(out) if motion_d => Mode::ScreenshotMotionDiagonal { out, file, keys },
        Some(out) if motion_v => Mode::ScreenshotMotionVertical { out, file, keys },
        Some(out) if motion => Mode::ScreenshotMotion { out, file, keys },
        Some(out) => Mode::Screenshot {
            out,
            file,
            opts,
            keys,
            root,
            workspace: workspace_folded,
            notes_root: notes_root_resolved,
            config,
        },
        None => Mode::Windowed {
            file,
            root,
            workspace,
            notes_root,
            config,
        },
    })
}

/// Resolve the NOTES ROOT: explicit `--notes-root`, else `~/notes` (`$HOME/notes`),
/// else `./notes` if HOME is unset. The directory is created lazily on first use
/// (C-x n / first note save), so it need not exist yet.
fn resolve_notes_root(notes_root: &Option<PathBuf>) -> PathBuf {
    if let Some(n) = notes_root {
        return n.clone();
    }
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join("notes"),
        None => PathBuf::from("notes"),
    }
}

/// Build the editor buffer for a (possibly absent) file. A missing/unreadable
/// file yields an empty buffer bound to that path; no file yields a scratch
/// buffer.
fn load_buffer(file: &Option<PathBuf>) -> Buffer {
    match file {
        Some(p) => Buffer::from_file(p),
        None => Buffer::scratch(),
    }
}

/// Resolve the ACTIVE project root: explicit `--root`, else (if the launch file
/// is a directory) that directory, else the file's parent, else the current
/// working directory. This is what scopes the go-to overlay to THIS project.
fn resolve_root(root: &Option<PathBuf>, file: &Option<PathBuf>) -> PathBuf {
    if let Some(r) = root {
        return r.clone();
    }
    if let Some(f) = file {
        if f.is_dir() {
            return f.clone();
        }
        if let Some(p) = f.parent() {
            if !p.as_os_str().is_empty() {
                return p.to_path_buf();
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Resolve the EFFECTIVE workspace whose child dirs are the switch-project
/// (C-x p) candidates: an explicit `--workspace` wins; otherwise DEFAULT to the
/// PARENT of the active project `root`, so switch-project lists the root's
/// SIBLING projects out of the box — launched inside `~/work/repos/some-repo`,
/// the workspace defaults to `~/work/repos`, so C-x p shows all the repos. A
/// root with no usable parent (e.g. the filesystem root) falls back to the root
/// itself, so the picker still opens rather than silently doing nothing.
pub fn resolve_workspace(workspace: &Option<PathBuf>, root: &std::path::Path) -> PathBuf {
    if let Some(w) = workspace {
        return w.clone();
    }
    match root.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => root.to_path_buf(),
    }
}

/// What a `--keys` replay produced beyond the buffer (App-level state living off
/// the `Buffer`), folded into the capture options by the caller.
struct ReplayResult {
    zoom: Option<f32>,
    selection: Option<((usize, usize), (usize, usize))>,
    search_query: Option<String>,
    search_case: bool,
    /// Whether the replay left the search panel in REPLACE mode (Cmd-Option-F).
    replace_active: bool,
    /// The replacement field text (empty headlessly — the isearch-input gap).
    replacement: String,
    /// The overlay left open at the end of the replay (if any), for the sidecar.
    overlay: Option<crate::overlay::OverlayState>,
    /// If the replay ACCEPTED a go-to item (Enter), the chosen value so the
    /// caller can load that file before capturing.
    accept: Option<(crate::overlay::OverlayKind, String)>,
}

/// Replay a parsed `--keys` action stream against `buffer` THROUGH the shared
/// `actions::apply_core` seam, so headless replay is byte-for-byte identical to
/// live editing. `corpus` is the active project's file index (Goto), `root`
/// scopes the Browse navigator, and `workspace` supplies the switch-project
/// children — so a replayed `C-x C-f` / `C-x p` / `C-x j` summons a real overlay
/// the rest of the key-spec can filter / move / descend / accept. Returns the
/// post-replay App-level state.
fn replay_keys(
    buffer: &mut Buffer,
    keys: &[Action],
    corpus: &[String],
    root: &std::path::Path,
    workspace: Option<&std::path::Path>,
    notes_root: &std::path::Path,
    config: &Config,
    // The visual-line motion LAYOUT ORACLE (an offscreen-shaped pipeline), so the
    // headless replay sees the SAME wrap geometry the live window does. `None` in
    // the unit tests / GPU-less paths, where motion falls back to LOGICAL lines.
    oracle: Option<&dyn actions::LayoutOracle>,
) -> ReplayResult {
    let mut shift_selecting = false;
    let mut zoom = 1.0f32;
    let mut search: Option<crate::search::SearchState> = None;
    let mut overlay: Option<crate::overlay::OverlayState> = None;
    let mut accept: Option<(crate::overlay::OverlayKind, String)> = None;
    // The spell engine for the Cmd-`;` picker, loaded once (None if the dictionary
    // failed to parse — the summon then no-ops, like the live path with no checker).
    let spell = crate::spell::SpellChecker::new().ok();
    for key in keys {
        // A tiny worklist so the COMMAND PALETTE's run-on-Enter chains: Enter on a
        // command writes `run_action`, which we then feed back through the core
        // (slot now empty) so an overlay-opening command opens its sub-overlay as
        // the final captured state. At most one chained action, so this drains in
        // one extra pass.
        let mut current: Option<Action> = Some(key.clone());
        while let Some(action) = current.take() {
        // OUTLINE picker corpus: the current buffer's markdown headings (title
        // indented by depth, paired with its line). Read before the builder; a
        // non-markdown buffer / no headings yields an empty list (no-op summon).
        let outline_headings: Vec<(String, usize)> = if buffer.is_markdown() {
            crate::markdown::headings(&buffer.text())
                .into_iter()
                .map(|h| (h.label(), h.line))
                .collect()
        } else {
            Vec::new()
        };
        // SPELL picker target: the misspelled word the cursor is on (or adjacent to)
        // + its corrections, resolved before the builder and ONLY when the spell
        // binding fired. None when the cursor isn't on a flagged word (no-op summon).
        let spell_target: Option<(Vec<String>, (usize, usize, usize))> =
            if matches!(action, Action::OpenSpellSuggest) {
                spell.as_ref().and_then(|sc| {
                    let (line, col) = buffer.cursor_line_col();
                    sc.suggest_at(&buffer.text(), line, col).map(|t| {
                        (
                            t.suggestions,
                            (t.misspelling.line, t.misspelling.start_col, t.misspelling.end_col),
                        )
                    })
                })
            } else {
                None
            };
        // The non-navigable builder inputs. Headless leaves the GO-TO recency tiers +
        // labels EMPTY (no mtime read, no open/recent history) so the capture stays
        // byte-stable; the buffer-scoped outline / spell come from the replayed state.
        let build_ctx = crate::overlay::BuildCtx {
            goto_corpus: corpus.to_vec(),
            goto_open: Vec::new(),
            goto_recent: Vec::new(),
            goto_times: Vec::new(),
            config_keys: &config.keys,
            outline_headings,
            spell_target,
        };
        let mut make_overlay =
            |kind: crate::overlay::OverlayKind| crate::overlay::build(kind, &build_ctx);
        let mut browse_to = |kind: crate::overlay::OverlayKind, rel: Option<String>| {
            // Shared one-level builder: Project navigates the workspace by absolute
            // path, MoveDest walks the NOTES root (folders only), Browse the active
            // root (files + folders).
            crate::overlay::browse_level(kind, rel, root, notes_root, workspace)
        };
        let mut ctx = actions::ActionCtx {
            buffer,
            shift_selecting: &mut shift_selecting,
            zoom: &mut zoom,
            search: &mut search,
            // Headless has no viewport to measure; a page is a fixed,
            // deterministic chunk of logical lines.
            scroll_page_lines: 20,
            overlay: &mut overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
            oracle,
        };
        // Replay is unshifted: selection comes from an explicit C-Space mark,
        // matching the emacs-style sticky region the key-spec expresses.
        let effect = actions::apply_core(&mut ctx, &action, false);
        drop(ctx);
        // Carry out the ONE deferred effect the core signalled (mutually exclusive,
        // so a single match suffices). Quit / LastBuffer are no-ops in capture (no
        // event loop, no 2-deep history); the rest mirror the live App's handling.
        match effect {
            // C-x n: reset the buffer to a fresh quick note bound to the notes root,
            // so subsequent typed chars build the title and an explicit `C-x C-s`
            // derives the filename + writes it. The root-switch is App-only; headless
            // only needs the buffer to become a note so the Save flow is verifiable.
            actions::Effect::NewNote => buffer.start_note(notes_root.to_path_buf()),
            // Settings: load the config file into the buffer (creating the commented
            // default first if missing), so the capture reflects the config CONTENTS
            // — exactly what the live Settings command does. Opens the EFFECTIVE
            // config path (the `--config` target when one was given).
            actions::Effect::OpenSettings => {
                if !config.path.as_os_str().is_empty() {
                    if !config.path.exists() {
                        let _ = Config::write_default(&config.path);
                    }
                    *buffer = Buffer::from_file(&config.path);
                }
            }
            // An overlay accepted (Goto file / Project / MoveDest / Theme): remember
            // the chosen value for the caller to load before capturing. Persists
            // across keys like the old out-param (later accepts overwrite).
            actions::Effect::OverlayAccept(kind, val) => accept = Some((kind, val)),
            // COMMAND PALETTE run-on-Enter: feed the chosen command back through the
            // core (the palette already closed), so e.g. "Go to file" opens the goto
            // overlay as the final captured state.
            actions::Effect::RunAction(a) => current = Some(a),
            // Quit / LastBuffer have nothing to do in the headless capture path.
            actions::Effect::LastBuffer | actions::Effect::Quit | actions::Effect::None => {}
        }
        }
    }
    let zoom_out = if zoom != 1.0 { Some(zoom) } else { None };
    let sel = buffer.selection_line_col();
    let search_query = search.as_ref().map(|s| s.query().to_string());
    let search_case = search.as_ref().map(|s| s.is_case_sensitive()).unwrap_or(false);
    let replace_active = search.as_ref().map(|s| s.is_replace_active()).unwrap_or(false);
    let replacement = search.as_ref().map(|s| s.replacement().to_string()).unwrap_or_default();
    ReplayResult {
        zoom: zoom_out,
        selection: sel,
        search_query,
        search_case,
        replace_active,
        replacement,
        overlay,
        accept,
    }
}

fn main() -> Result<()> {
    match parse_args()? {
        Mode::Screenshot {
            out,
            file,
            mut opts,
            keys,
            root,
            workspace,
            notes_root,
            config,
        } => {
            // Resolve the active project + its file index BEFORE the replay so a
            // `C-x C-f` in the key-spec summons a real, scoped go-to overlay.
            let active_root = resolve_root(&root, &file);
            let proj = crate::project::Project::resolve(&active_root);
            let corpus = crate::index::build_index(&active_root);
            // Default the switch-project workspace to the active root's PARENT when
            // neither `--workspace` nor a config `workspace` was given, so the sidecar
            // reports an EFFECTIVE folder (and a replayed C-x p lists siblings).
            let effective_workspace = resolve_workspace(&workspace, &active_root);
            opts.project = Some(capture::ProjectInfo {
                root: active_root.clone(),
                name: proj.name.clone(),
                branch: proj.branch.clone(),
                dirty: proj.dirty,
                // The EFFECTIVE notes_root / workspace (flag > config > default), so a
                // `--config`-driven launch shows the configured folders with no flags.
                notes_root: Some(notes_root.clone()),
                workspace: Some(effective_workspace.clone()),
            });

            let mut buffer = load_buffer(&file);
            // Replay `--keys` FIRST so the cursor/selection/search the spec
            // produces are what the capture reflects. Fold the App-level state
            // (zoom / selection / search) the replay produced into the capture
            // opts — but never clobber an explicit verification hook.
            // Default the switch-project workspace to the active root's PARENT
            // when no explicit `--workspace` was given, so a replayed `C-x p`
            // summons the picker listing the root's SIBLING projects (rather than
            // silently doing nothing). An explicit `--workspace` still overrides.
            //
            // Visual-line motion ORACLE: when the spec has keys, build an offscreen
            // pipeline shaped like the upcoming capture so headless motion reads the
            // SAME wrap geometry the live window does. Skipped for an empty spec (no
            // motion to resolve) and absent on GPU-less hosts (logical fallback).
            let oracle = if keys.is_empty() {
                None
            } else {
                capture::build_oracle(&buffer, &opts)
            };
            let res = replay_keys(
                &mut buffer,
                &keys,
                &corpus,
                &active_root,
                Some(effective_workspace.as_path()),
                &notes_root,
                &config,
                oracle.as_ref().map(|o| o.as_oracle()),
            );
            if opts.zoom.is_none() {
                opts.zoom = res.zoom;
            }
            if opts.selection.is_none() {
                opts.selection = res.selection;
            }
            if opts.search.is_none() {
                opts.search = res.search_query;
                opts.search_case_sensitive = opts.search_case_sensitive || res.search_case;
                // REPLACE mode the replay opened (Cmd-Option-F) — surfaced so the
                // panel's replace row renders + the sidecar reports it.
                opts.search_replace_active = res.replace_active;
                opts.search_replacement = res.replacement;
            }
            // If the replay ACCEPTED an overlay item, reflect it in the capture.
            // Goto: load the opened file. Project: re-root — re-resolve the project
            // at the accepted ABSOLUTE directory and overwrite the sidecar `project`
            // block (otherwise a switch-project replay leaves NO observable trace).
            if let Some((kind, val)) = &res.accept {
                match kind {
                    crate::overlay::OverlayKind::Goto => {
                        let path = crate::index::resolve(&active_root, val);
                        buffer = Buffer::from_file(&path);
                    }
                    crate::overlay::OverlayKind::Project => {
                        let new_root = std::path::PathBuf::from(val);
                        let proj = crate::project::Project::resolve(&new_root);
                        opts.project = Some(capture::ProjectInfo {
                            root: new_root,
                            name: proj.name.clone(),
                            branch: proj.branch.clone(),
                            dirty: proj.dirty,
                            notes_root: Some(notes_root.clone()),
                            workspace: Some(effective_workspace.clone()),
                        });
                    }
                    // Outline: jump the cursor to the accepted heading LINE so the
                    // capture's `cursor` block reflects the jump (agent-verifiable).
                    crate::overlay::OverlayKind::Outline => {
                        if let Ok(line) = val.parse::<usize>() {
                            let idx = buffer.line_col_to_char(line, 0);
                            buffer.set_cursor(idx);
                        }
                    }
                    _ => {}
                }
            }
            // Reflect any still-open overlay in the capture opts (and thus the
            // sidecar `overlay` block).
            if let Some(ov) = &res.overlay {
                opts.overlay = Some(capture::OverlayInfo {
                    active: true,
                    mode: ov.kind.as_str(),
                    query: ov.query.clone(),
                    items: ov.item_strings(),
                    bindings: ov.item_bindings(),
                    selected_index: ov.selected,
                    hint: ov.kind.hint().to_string(),
                    browse_dir: ov.browse_dir.clone(),
                });
            }
            // If a selection is requested (or one came from --keys), move the
            // buffer cursor to its END so the caret renders at the cursor end of
            // the region. A --keys replay already left the cursor where it
            // belongs, so only do this for an EXPLICIT --sel (no replay).
            if keys.is_empty() {
                if let Some((_, (l1, c1))) = opts.selection {
                    let end = buffer.line_col_to_char(l1, c1);
                    buffer.set_cursor(end);
                }
            }
            capture::capture_with(&out, &buffer, &opts)?;
            println!("wrote {} (+ sidecar .json)", out.display());
            Ok(())
        }
        Mode::ScreenshotMotion { out, file, keys } => {
            let mut buffer = load_buffer(&file);
            let root = resolve_root(&None, &file);
            replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
            capture::capture_motion(&out, &buffer)?;
            println!("wrote {} (mid-glide, + sidecar .json)", out.display());
            Ok(())
        }
        Mode::ScreenshotMotionVertical { out, file, keys } => {
            let mut buffer = load_buffer(&file);
            let root = resolve_root(&None, &file);
            replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
            capture::capture_motion_vertical(&out, &buffer)?;
            println!("wrote {} (mid-glide vertical, + sidecar .json)", out.display());
            Ok(())
        }
        Mode::ScreenshotMotionDiagonal { out, file, keys } => {
            let mut buffer = load_buffer(&file);
            let root = resolve_root(&None, &file);
            replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
            capture::capture_motion_diagonal(&out, &buffer)?;
            println!("wrote {} (mid-glide diagonal, + sidecar .json)", out.display());
            Ok(())
        }
        Mode::CaptureTimeline {
            out,
            file,
            keys,
            steps,
            root,
            canvas,
            dpi,
        } => {
            let active_root = resolve_root(&root, &file);
            let proj = crate::project::Project::resolve(&active_root);
            let corpus = crate::index::build_index(&active_root);
            let notes_root = active_root.clone();
            let opts = CaptureOpts {
                project: Some(capture::ProjectInfo {
                    root: active_root.clone(),
                    name: proj.name.clone(),
                    branch: proj.branch.clone(),
                    dirty: proj.dirty,
                    notes_root: None,
                    workspace: None,
                }),
                canvas,
                dpi,
                ..CaptureOpts::default()
            };

            let mut buffer = load_buffer(&file);
            // Split the replay: all-but-last set up the ORIGIN, the LAST chord is
            // the NAVIGATION move whose glide the timeline captures. With an empty
            // or single-key spec the origin is wherever the prefix left the cursor.
            let (last, init) = match keys.split_last() {
                Some((last, init)) => (Some(last.clone()), init.to_vec()),
                None => (None, Vec::new()),
            };
            if !init.is_empty() {
                replay_keys(
                    &mut buffer,
                    &init,
                    &corpus,
                    &active_root,
                    None,
                    &notes_root,
                    &Config::empty(),
                    None,
                );
            }
            let origin = buffer.cursor_line_col();
            if let Some(last) = last {
                replay_keys(
                    &mut buffer,
                    std::slice::from_ref(&last),
                    &corpus,
                    &active_root,
                    None,
                    &notes_root,
                    &Config::empty(),
                    None,
                );
            }
            capture::capture_timeline(&out, &buffer, origin, &steps, &opts)?;
            println!(
                "wrote {} timeline frames for {} (+ per-step sidecars)",
                steps.len(),
                out.display()
            );
            Ok(())
        }
        Mode::CaptureHeld {
            out,
            file,
            keys,
            dir,
            steps,
            root,
            canvas,
            dpi,
        } => {
            let active_root = resolve_root(&root, &file);
            let proj = crate::project::Project::resolve(&active_root);
            let corpus = crate::index::build_index(&active_root);
            let notes_root = active_root.clone();
            let opts = CaptureOpts {
                project: Some(capture::ProjectInfo {
                    root: active_root.clone(),
                    name: proj.name.clone(),
                    branch: proj.branch.clone(),
                    dirty: proj.dirty,
                    notes_root: None,
                    workspace: None,
                }),
                canvas,
                dpi,
                ..CaptureOpts::default()
            };

            let mut buffer = load_buffer(&file);
            // The FULL `--keys` replay sets up the ORIGIN the held burst starts from
            // (e.g. C-n's + C-f's to land mid-line); the held re-targeting then
            // drives the motion deterministically from there.
            if !keys.is_empty() {
                replay_keys(&mut buffer, &keys, &corpus, &active_root, None, &notes_root, &Config::empty(), None);
            }
            let origin = buffer.cursor_line_col();
            capture::capture_held(&out, &buffer, origin, dir, &steps, &opts)?;
            println!(
                "wrote {} held frames for {} (+ per-step sidecars)",
                steps.len(),
                out.display()
            );
            Ok(())
        }
        Mode::BenchTyping => bench::run(),
        Mode::Windowed {
            file,
            root,
            workspace,
            notes_root,
            config,
        } => {
            let active_root = resolve_root(&root, &file);
            // Pass the RAW flags + config; `App::new` folds them (flag > config >
            // default) and re-folds on a live config reload.
            app::run(file, active_root, workspace, notes_root, config)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_keys_builds_selection_from_mark_and_motion() {
        // replay_keys is pure (Buffer + actions, no GPU) but was only reached
        // through the adapter-gated capture tests. Drive it directly: type "abc",
        // mark with C-Space at the end, then move left twice — the post-replay
        // ReplayResult must carry the ordered region.
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("a b c C-Space Left Left").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert_eq!(res.selection, Some(((0, 1), (0, 3))), "mark@3 + two Lefts -> [1,3)");
    }

    #[test]
    fn replay_keys_runs_palette_chain_into_overlay() {
        // The command-palette run-on-Enter chain (Effect::RunAction fed back through
        // the core in the same replay): Cmd-P opens the palette, "goto" filters to
        // "Go to file", Enter runs OpenGoto, which the worklist re-dispatches into
        // the Goto overlay as the final captured state.
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("s-p g o t o RET").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert_eq!(
            res.overlay.map(|o| o.kind),
            Some(crate::overlay::OverlayKind::Goto),
            "palette Enter on 'Go to file' chains into the Goto overlay",
        );
    }

    #[test]
    fn workspace_defaults_to_root_parent_when_unset() {
        // No `--workspace`: the effective workspace is the active root's PARENT,
        // so C-x p lists the root's sibling projects out of the box.
        let root = PathBuf::from("/home/me/work/repos/some-repo");
        assert_eq!(
            resolve_workspace(&None, &root),
            PathBuf::from("/home/me/work/repos")
        );
    }

    #[test]
    fn explicit_workspace_overrides_the_default() {
        // An explicit `--workspace` always wins, ignoring the root's parent.
        let root = PathBuf::from("/home/me/work/repos/some-repo");
        let ws = PathBuf::from("/elsewhere/projects");
        assert_eq!(resolve_workspace(&Some(ws.clone()), &root), ws);
    }

    #[test]
    fn workspace_falls_back_to_root_when_no_parent() {
        // A root with no usable parent (the filesystem root) falls back to the
        // root itself, so the picker still opens rather than doing nothing.
        let root = PathBuf::from("/");
        assert_eq!(resolve_workspace(&None, &root), root);
    }

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

    // ---- VISUAL-LINE MOVEMENT (Phase 2) ----------------------------------
    //
    // These drive the REAL keymap through `replay_keys` with a layout oracle
    // shaped at a NARROW measure, exactly as the live window / `--keys --measure`
    // CLI do, so a long line soft-wraps and the motions must follow the VISUAL
    // rows. The page globals are process-wide, so each test holds `page::TEST_LOCK`
    // and restores the default measure. On a GPU-less host the oracle is `None`,
    // motion falls back to logical, and the test SKIPS (prints + returns).

    /// Build a narrow-measure oracle, replay `keys` through the real keymap, and
    /// return the resulting (line, col) — or `None` when no wgpu adapter exists
    /// (skip). Holds the page lock for the whole replay and restores the measure.
    fn replay_visual(text: &str, measure: usize, keys: &str) -> Option<(usize, usize)> {
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::page::set_page_on(true);
        crate::page::set_measure(measure);
        let mut buffer = Buffer::from_str(text);
        let opts = CaptureOpts::default();
        let out = capture::build_oracle(&buffer, &opts).map(|op| {
            let keys = keyspec::parse_keys(keys).unwrap();
            let root = PathBuf::from("/tmp");
            replay_keys(
                &mut buffer,
                &keys,
                &[],
                &root,
                None,
                &root,
                &Config::empty(),
                Some(op.as_oracle()),
            );
            buffer.cursor_line_col()
        });
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
        out
    }

    // A single long paragraph that soft-wraps into several visual rows at a narrow
    // measure, followed by a SHORT line — the wrapped + crossing fixture.
    const LONG: &str = "the quick brown fox jumps over the lazy dog today\nNEXT\n";
    const LONG_LINE0_LEN: usize = 49; // chars before the first '\n'

    #[test]
    fn visual_c_n_lands_on_next_visual_row_not_next_paragraph() {
        // (1) C-n from the start of a wrapped line steps DOWN one VISUAL row of the
        // SAME logical line — not into the next paragraph.
        let Some((line, col)) = replay_visual(LONG, 15, "C-n") else {
            eprintln!("skipping visual_c_n_lands_on_next_visual_row: no wgpu adapter");
            return;
        };
        assert_eq!(line, 0, "C-n stays on the wrapped logical line, not paragraph 2");
        assert!(col > 0, "C-n moved off col 0 onto the next visual row, got {col}");
        assert!(
            col < LONG_LINE0_LEN,
            "the landing is a wrap boundary mid-line, not the logical end ({col})"
        );
    }

    #[test]
    fn visual_c_e_stops_at_visual_row_end_not_logical_line_end() {
        // (2) C-e goes to the end of the current VISUAL row, well short of the
        // logical line's end.
        let Some((line, col)) = replay_visual(LONG, 15, "C-e") else {
            eprintln!("skipping visual_c_e_stops_at_visual_row_end: no wgpu adapter");
            return;
        };
        assert_eq!(line, 0);
        assert!(col > 0, "C-e moved to the visual row end");
        assert!(
            col < LONG_LINE0_LEN,
            "C-e stopped at the VISUAL row end ({col}), not the logical line end ({LONG_LINE0_LEN})"
        );
    }

    #[test]
    fn visual_goal_x_is_preserved_across_c_n_then_c_p() {
        // (3) The sticky GOAL-X: move right 5, then C-n then C-p returns to the
        // SAME column (the down/up round-trip lands back under the seeded goal-x).
        let down_up = replay_visual(LONG, 15, "C-f C-f C-f C-f C-f C-n C-p");
        let just_right = replay_visual(LONG, 15, "C-f C-f C-f C-f C-f");
        let (Some(down_up), Some(just_right)) = (down_up, just_right) else {
            eprintln!("skipping visual_goal_x_preserved: no wgpu adapter");
            return;
        };
        assert_eq!(just_right, (0, 5), "five C-f land at col 5");
        assert_eq!(
            down_up, just_right,
            "C-n then C-p returns to the starting column via the sticky goal-x"
        );
    }

    #[test]
    fn visual_c_a_goes_to_visual_row_start() {
        // (4) C-a goes to the start of the current VISUAL row. C-n from col 0 lands
        // on the next visual row's start S; from mid that row, C-a returns to S.
        let start = replay_visual(LONG, 15, "C-n");
        let from_mid = replay_visual(LONG, 15, "C-n C-f C-f C-a");
        let (Some(start), Some(from_mid)) = (start, from_mid) else {
            eprintln!("skipping visual_c_a_goes_to_visual_row_start: no wgpu adapter");
            return;
        };
        assert_eq!(start.0, 0);
        assert!(start.1 > 0, "C-n reached a wrapped row start > 0");
        assert_eq!(
            from_mid, start,
            "C-a snaps back to the VISUAL row start, not the logical line start (col 0)"
        );
    }

    #[test]
    fn visual_c_n_at_last_visual_row_crosses_to_next_logical_line() {
        // (5) At the LAST visual row of a wrapped line, C-n crosses into the NEXT
        // logical line's FIRST visual row. Count line-0's visual rows via the
        // oracle, then drive that many C-n through the real keymap.
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::page::set_page_on(true);
        crate::page::set_measure(15);
        let probe = Buffer::from_str(LONG);
        let opts = CaptureOpts::default();
        let result = capture::build_oracle(&probe, &opts).map(|op| {
            use crate::actions::LayoutOracle;
            // Step DOWN from (0,0) with goal-x 0 until the logical line changes;
            // `steps` C-n's cross into line 1, `steps-1` stay on line 0.
            let mut steps = 0usize;
            {
                let oracle = op.as_oracle();
                let (mut l, mut c) = (0usize, 0usize);
                loop {
                    let (nl, nc) = oracle.visual_line_down(l, c, 0.0);
                    steps += 1;
                    if nl != 0 {
                        break;
                    }
                    assert!(steps < 100, "line 0 never ended");
                    l = nl;
                    c = nc;
                }
            }
            assert!(steps >= 2, "line 0 should wrap into multiple visual rows");
            let root = PathBuf::from("/tmp");
            // One fewer C-n keeps us on line 0's LAST visual row...
            let mut b0 = Buffer::from_str(LONG);
            let keys_stay = keyspec::parse_keys(&"C-n ".repeat(steps - 1)).unwrap();
            replay_keys(&mut b0, &keys_stay, &[], &root, None, &root, &Config::empty(), Some(op.as_oracle()));
            let stay = b0.cursor_line_col();
            // ...and the full count crosses into line 1's first visual row.
            let mut b1 = Buffer::from_str(LONG);
            let keys_cross = keyspec::parse_keys(&"C-n ".repeat(steps)).unwrap();
            replay_keys(&mut b1, &keys_cross, &[], &root, None, &root, &Config::empty(), Some(op.as_oracle()));
            (stay, b1.cursor_line_col())
        });
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
        let Some((stay, cross)) = result else {
            eprintln!("skipping visual_c_n_crosses_to_next_logical_line: no wgpu adapter");
            return;
        };
        assert_eq!(stay.0, 0, "one C-n short keeps us on line 0's last visual row");
        assert_eq!(cross.0, 1, "the last-row C-n crosses into the next logical line");
        // Line 1 ("NEXT") fits one visual row, so its first row starts at col 0.
        assert_eq!(cross.1, 0, "we land on line 1's FIRST visual row");
    }

    #[test]
    fn regression_non_wrapped_doc_visual_equals_logical_byte_identical() {
        // REGRESSION GUARD: on a NON-wrapped document (every logical line fits in
        // one visual row) visual motion == logical motion. Identical-content lines
        // make the vertical goal-x round-trip exact even on a proportional font.
        // Replay the SAME keys with the oracle (visual) and without it (logical);
        // the resulting cursors — and the rendered PNGs — must be IDENTICAL.
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::page::set_page_on(true);
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
        let text = "hello world foo\nhello world foo\nhello world foo\n";
        let keys = keyspec::parse_keys("C-f C-f C-f C-f C-f C-n C-n C-e C-a C-p C-k").unwrap();
        let root = PathBuf::from("/tmp");
        let opts = CaptureOpts::default();

        let mut logical = Buffer::from_str(text);
        replay_keys(&mut logical, &keys, &[], &root, None, &root, &Config::empty(), None);

        let mut visual = Buffer::from_str(text);
        let Some(op) = capture::build_oracle(&visual, &opts) else {
            crate::page::set_measure(crate::page::DEFAULT_MEASURE);
            eprintln!("skipping regression_non_wrapped byte-identical: no wgpu adapter");
            return;
        };
        replay_keys(&mut visual, &keys, &[], &root, None, &root, &Config::empty(), Some(op.as_oracle()));

        assert_eq!(
            visual.cursor_line_col(),
            logical.cursor_line_col(),
            "non-wrapped: visual motion must equal logical motion"
        );

        // Byte-identical captures: render both buffers and diff the PNG bytes.
        let dir = std::env::temp_dir();
        let pv = dir.join("awl_vl_visual.png");
        let pl = dir.join("awl_vl_logical.png");
        capture::capture_with(&pv, &visual, &opts).expect("render visual");
        capture::capture_with(&pl, &logical, &opts).expect("render logical");
        let bv = std::fs::read(&pv).expect("read visual png");
        let bl = std::fs::read(&pl).expect("read logical png");
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
        assert_eq!(
            bv, bl,
            "non-wrapped short-line doc: visual + logical captures are byte-identical"
        );
    }
}
