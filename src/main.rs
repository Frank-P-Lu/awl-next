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
mod focus;
mod fuzzy;
mod index;
mod keymap;
mod keyspec;
mod overlay;
mod page;
mod project;
mod render;
mod search;
mod selection;
mod spell;
mod spellunderline;
mod theme;

use std::path::PathBuf;

use anyhow::{bail, Result};

use crate::buffer::Buffer;
use crate::capture::CaptureOpts;
use crate::keymap::Action;

enum Mode {
    Windowed {
        file: Option<PathBuf>,
        /// The ACTIVE project root (`--root`). When absent it defaults to the
        /// launch file's parent (or cwd) in `app::run`.
        root: Option<PathBuf>,
        /// Optional workspace parent (`--workspace`) whose children are the
        /// switch-project candidates. Stored for the next phase.
        workspace: Option<PathBuf>,
        /// The NOTES ROOT (`--notes-root`, default `~/notes`): the home project
        /// where C-x n captures quick scrap notes and C-x m moves them.
        notes_root: PathBuf,
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

fn parse_args() -> Result<Mode> {
    let mut args = std::env::args().skip(1);
    let mut out: Option<PathBuf> = None;
    let mut motion = false;
    let mut motion_v = false;
    let mut motion_d = false;
    // `--capture-timeline "<ms,ms,...>"` cumulative step sequence (None = not a
    // timeline capture).
    let mut timeline_steps: Option<Vec<u32>> = None;
    let mut file: Option<PathBuf> = None;
    let mut opts = CaptureOpts::default();
    let mut bench_typing = false;
    // `--keys` replay, parsed once here so a bad spec fails arg-parsing (not deep
    // in the capture). Threaded into whichever screenshot Mode is selected.
    let mut keys: Vec<Action> = Vec::new();
    let mut keys_given = false;
    let mut root: Option<PathBuf> = None;
    let mut workspace: Option<PathBuf> = None;
    let mut notes_root: Option<PathBuf> = None;

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
            }
            "--screenshot-motion" => {
                let p = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--screenshot-motion requires an output path")
                })?;
                out = Some(PathBuf::from(p));
                motion = true;
            }
            "--screenshot-motion-v" => {
                let p = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--screenshot-motion-v requires an output path")
                })?;
                out = Some(PathBuf::from(p));
                motion_v = true;
            }
            "--screenshot-motion-d" => {
                let p = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--screenshot-motion-d requires an output path")
                })?;
                out = Some(PathBuf::from(p));
                motion_d = true;
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
                let n: usize = v
                    .parse()
                    .map_err(|_| anyhow::anyhow!("bad --measure {v:?}"))?;
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
                keys = keyspec::parse_keys(&v)?;
                keys_given = true;
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
                     \x20 --measure N         page-mode column width in chars (default 80; implies --page on)\n\
                     \x20 --page on|off       page mode: centered column (on, default) vs edge-to-edge (off)\n\
                     \x20 --notes-root DIR    quick-notes home for C-x n / C-x m (default ~/notes)\n\
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
    // `--keys` only makes sense with a capture mode (it mutates the buffer for a
    // one-frame capture); refuse it for the windowed editor where live typing is
    // the input path.
    if keys_given && out.is_none() {
        bail!("--keys requires a capture mode (e.g. --screenshot OUT.png)");
    }
    let notes_root = resolve_notes_root(&notes_root);
    Ok(match out {
        Some(out) if timeline_steps.is_some() => Mode::CaptureTimeline {
            out,
            file,
            keys,
            steps: timeline_steps.unwrap(),
            root,
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
            workspace,
            notes_root,
        },
        None => Mode::Windowed {
            file,
            root,
            workspace,
            notes_root,
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

/// What a `--keys` replay produced beyond the buffer (App-level state living off
/// the `Buffer`), folded into the capture options by the caller.
struct ReplayResult {
    zoom: Option<f32>,
    selection: Option<((usize, usize), (usize, usize))>,
    search_query: Option<String>,
    search_case: bool,
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
) -> ReplayResult {
    let mut shift_selecting = false;
    let mut zoom = 1.0f32;
    let mut search: Option<crate::search::SearchState> = None;
    let mut overlay: Option<crate::overlay::OverlayState> = None;
    let mut accept: Option<(crate::overlay::OverlayKind, String)> = None;
    let mut last_buffer = false;
    let mut new_note = false;
    let corpus_vec = corpus.to_vec();
    for key in keys {
        // A tiny worklist so the COMMAND PALETTE's run-on-Enter chains: Enter on a
        // command writes `run_action`, which we then feed back through the core
        // (slot now empty) so an overlay-opening command opens its sub-overlay as
        // the final captured state. At most one chained action, so this drains in
        // one extra pass.
        let mut current: Option<Action> = Some(key.clone());
        while let Some(action) = current.take() {
        let mut make_overlay = |kind: crate::overlay::OverlayKind| match kind {
            crate::overlay::OverlayKind::Goto => Some(crate::overlay::OverlayState::new(
                kind,
                corpus_vec.clone(),
                Vec::new(),
                Vec::new(),
            )),
            crate::overlay::OverlayKind::Theme => {
                let names: Vec<String> =
                    crate::theme::THEMES.iter().map(|t| t.name.to_string()).collect();
                Some(crate::overlay::OverlayState::new_theme(
                    names,
                    crate::theme::active_index(),
                ))
            }
            crate::overlay::OverlayKind::Command => Some(crate::overlay::OverlayState::new_command(
                crate::commands::names(),
                crate::commands::bindings(),
            )),
            crate::overlay::OverlayKind::Browse
            | crate::overlay::OverlayKind::MoveDest
            | crate::overlay::OverlayKind::Project => None,
        };
        let mut browse_to = |kind: crate::overlay::OverlayKind, rel: Option<String>| {
            // PROJECT explorer: navigate by ABSOLUTE path (`rel` is the absolute
            // dir; `None` = start at the workspace dir). Child FOLDERS only,
            // git-marked, with a synthetic "." accept-this-folder row on top.
            if kind == crate::overlay::OverlayKind::Project {
                let dir = match rel
                    .clone()
                    .or_else(|| workspace.map(|w| w.to_string_lossy().to_string()))
                {
                    Some(d) => d,
                    None => return None,
                };
                let folders: Vec<(String, bool)> =
                    crate::index::list_dir_level(std::path::Path::new(&dir), None)
                        .into_iter()
                        .filter(|e| e.is_dir)
                        .map(|e| (e.name, e.is_git))
                        .collect();
                return Some(crate::overlay::OverlayState::new_project(dir, folders));
            }
            // MoveDest (C-x m) walks the NOTES root, folders only; Browse walks the
            // active root and lists files + folders.
            let move_dest = kind == crate::overlay::OverlayKind::MoveDest;
            let walk_root = if move_dest { notes_root } else { root };
            let level = crate::index::list_dir_level(walk_root, rel.as_deref());
            let mut corpus = Vec::new();
            let mut git = Vec::new();
            let mut is_dir = Vec::new();
            for e in &level {
                if move_dest && !e.is_dir {
                    continue;
                }
                corpus.push(e.name.clone());
                git.push(e.is_git);
                is_dir.push(e.is_dir);
            }
            Some(crate::overlay::OverlayState::new_marked(
                kind, corpus, git, is_dir, Vec::new(), Vec::new(), rel,
            ))
        };
        let mut run_action: Option<Action> = None;
        let mut ctx = actions::ActionCtx {
            buffer,
            shift_selecting: &mut shift_selecting,
            zoom: &mut zoom,
            search: &mut search,
            // Headless has no viewport to measure; a page is a fixed,
            // deterministic chunk of logical lines.
            page_lines: 20,
            overlay: &mut overlay,
            make_overlay: &mut make_overlay,
            overlay_accept: &mut accept,
            browse_to: &mut browse_to,
            last_buffer: &mut last_buffer,
            new_note: &mut new_note,
            run_action: &mut run_action,
        };
        // Replay is unshifted: selection comes from an explicit C-Space mark,
        // matching the emacs-style sticky region the key-spec expresses.
        actions::apply_core(&mut ctx, &action, false);
        drop(ctx);
        // C-x n: reset the buffer to a fresh quick note bound to the notes root, so
        // subsequent typed chars build the title and an explicit `C-x C-s` derives
        // the filename + writes it. The root-switch is App-only; headless only needs
        // the buffer to become a note so the explicit-Save flow is verifiable.
        if new_note {
            new_note = false;
            buffer.start_note(notes_root.to_path_buf());
        }
        // COMMAND PALETTE run-on-Enter: feed the chosen command back through the
        // core (the palette already closed), so e.g. "Go to file" opens the goto
        // overlay as the final captured state.
        current = run_action.take();
        }
    }
    let _ = last_buffer; // capture path has no 2-deep history to toggle
    let zoom_out = if zoom != 1.0 { Some(zoom) } else { None };
    let sel = buffer.selection_line_col();
    let search_query = search.as_ref().map(|s| s.query().to_string());
    let search_case = search.as_ref().map(|s| s.is_case_sensitive()).unwrap_or(false);
    ReplayResult {
        zoom: zoom_out,
        selection: sel,
        search_query,
        search_case,
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
        } => {
            // Resolve the active project + its file index BEFORE the replay so a
            // `C-x C-f` in the key-spec summons a real, scoped go-to overlay.
            let active_root = resolve_root(&root, &file);
            let proj = crate::project::Project::resolve(&active_root);
            let corpus = crate::index::build_index(&active_root);
            opts.project = Some(capture::ProjectInfo {
                root: active_root.clone(),
                name: proj.name.clone(),
                branch: proj.branch.clone(),
                dirty: proj.dirty,
            });

            let mut buffer = load_buffer(&file);
            // Replay `--keys` FIRST so the cursor/selection/search the spec
            // produces are what the capture reflects. Fold the App-level state
            // (zoom / selection / search) the replay produced into the capture
            // opts — but never clobber an explicit verification hook.
            let res = replay_keys(
                &mut buffer,
                &keys,
                &corpus,
                &active_root,
                workspace.as_deref(),
                &notes_root,
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
                        });
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
            replay_keys(&mut buffer, &keys, &[], &root, None, &root);
            capture::capture_motion(&out, &buffer)?;
            println!("wrote {} (mid-glide, + sidecar .json)", out.display());
            Ok(())
        }
        Mode::ScreenshotMotionVertical { out, file, keys } => {
            let mut buffer = load_buffer(&file);
            let root = resolve_root(&None, &file);
            replay_keys(&mut buffer, &keys, &[], &root, None, &root);
            capture::capture_motion_vertical(&out, &buffer)?;
            println!("wrote {} (mid-glide vertical, + sidecar .json)", out.display());
            Ok(())
        }
        Mode::ScreenshotMotionDiagonal { out, file, keys } => {
            let mut buffer = load_buffer(&file);
            let root = resolve_root(&None, &file);
            replay_keys(&mut buffer, &keys, &[], &root, None, &root);
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
                }),
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
        Mode::BenchTyping => bench::run(),
        Mode::Windowed {
            file,
            root,
            workspace,
            notes_root,
        } => {
            let active_root = resolve_root(&root, &file);
            app::run(file, active_root, workspace, notes_root)
        }
    }
}
