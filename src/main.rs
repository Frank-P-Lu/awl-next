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
mod bench;
mod buffer;
mod capture;
mod caret;
mod caret_glyph;
mod fuzzy;
mod index;
mod keymap;
mod keyspec;
mod overlay;
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

fn parse_args() -> Result<Mode> {
    let mut args = std::env::args().skip(1);
    let mut out: Option<PathBuf> = None;
    let mut motion = false;
    let mut motion_v = false;
    let mut motion_d = false;
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
            "--caret-anim-phase" => {
                // PROTOTYPE I-beam: pin the breathe pulse to a FIXED phase so a
                // headless capture can sample a representative shape deterministically
                // (0.0 = rest peak / full + thin; 0.5 = trough / dim + swollen). The
                // frozen path never advances the live clock, so without this flag the
                // breathe stays at the rest phase and captures are byte-stable.
                let v = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--caret-anim-phase requires a number (e.g. 0.5)")
                })?;
                let p: f32 = v
                    .parse()
                    .map_err(|_| anyhow::anyhow!("bad --caret-anim-phase {v:?}"))?;
                caret::set_ibeam_phase(p);
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
                     awl --screenshot-motion OUT.png [file]  caret mid-glide (trailing underline)\n\
                     awl --screenshot-motion-v OUT.png [file] caret mid-glide vertical (left-edge bar)\n\
                     awl --screenshot-motion-d OUT.png [file] caret mid-glide diagonal (slanted tracer)\n\
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
                     \x20 --caret-anim-phase F pin the ibeam breathe phase for capture (0=rest peak, 0.5=trough)\n\
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
    // Switch-project children (workspace dirs) with git markers, mirroring the
    // windowed app so a replayed `C-x p` lists the same marked candidates.
    let (proj_corpus, proj_git): (Vec<String>, Vec<bool>) = match workspace {
        Some(ws) => {
            let mut children: Vec<(String, bool)> = std::fs::read_dir(ws)
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                        .map(|e| {
                            let name = e.file_name().to_string_lossy().to_string();
                            let is_git = e.path().join(".git").exists();
                            (name, is_git)
                        })
                        .collect()
                })
                .unwrap_or_default();
            children.sort_by(|a, b| a.0.cmp(&b.0));
            children.into_iter().unzip()
        }
        None => (Vec::new(), Vec::new()),
    };
    for action in keys {
        let mut make_overlay = |kind: crate::overlay::OverlayKind| match kind {
            crate::overlay::OverlayKind::Goto => Some(crate::overlay::OverlayState::new(
                kind,
                corpus_vec.clone(),
                Vec::new(),
                Vec::new(),
            )),
            crate::overlay::OverlayKind::Project => {
                let n = proj_corpus.len();
                Some(crate::overlay::OverlayState::new_marked(
                    kind,
                    proj_corpus.clone(),
                    proj_git.clone(),
                    vec![true; n],
                    Vec::new(),
                    Vec::new(),
                    None,
                ))
            }
            crate::overlay::OverlayKind::Theme => {
                let names: Vec<String> =
                    crate::theme::THEMES.iter().map(|t| t.name.to_string()).collect();
                Some(crate::overlay::OverlayState::new_theme(
                    names,
                    crate::theme::active_index(),
                ))
            }
            crate::overlay::OverlayKind::Browse | crate::overlay::OverlayKind::MoveDest => None,
        };
        let mut browse_to = |kind: crate::overlay::OverlayKind, rel: Option<String>| {
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
        };
        // Replay is unshifted: selection comes from an explicit C-Space mark,
        // matching the emacs-style sticky region the key-spec expresses.
        actions::apply_core(&mut ctx, action, false);
        // C-x n: reset the buffer to a fresh quick note bound to the notes root, so
        // subsequent typed chars build the title and an explicit `C-x C-s` derives
        // the filename + writes it. The root-switch is App-only; headless only needs
        // the buffer to become a note so the explicit-Save flow is verifiable.
        if new_note {
            new_note = false;
            buffer.start_note(notes_root.to_path_buf());
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
            // If the replay ACCEPTED a go-to item (Enter on a file), load that
            // file into the buffer so the capture shows the opened file.
            if let Some((kind, val)) = &res.accept {
                if *kind == crate::overlay::OverlayKind::Goto {
                    let path = crate::index::resolve(&active_root, val);
                    buffer = Buffer::from_file(&path);
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
