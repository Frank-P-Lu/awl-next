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
mod keymap;
mod keyspec;
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
    Windowed { file: Option<PathBuf> },
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
    let mut file: Option<PathBuf> = None;
    let mut opts = CaptureOpts::default();
    let mut bench_typing = false;
    // `--keys` replay, parsed once here so a bad spec fails arg-parsing (not deep
    // in the capture). Threaded into whichever screenshot Mode is selected.
    let mut keys: Vec<Action> = Vec::new();
    let mut keys_given = false;

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
                    "auto" => {} // leave the font-derived default in effect
                    _ => bail!("unknown --caret-mode {v:?}; choose block, morph, or auto"),
                }
            }
            "--keys" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--keys requires a key-spec string"))?;
                keys = keyspec::parse_keys(&v)?;
                keys_given = true;
            }
            "-h" | "--help" => {
                println!(
                    "awl [file]\n\
                     awl --screenshot OUT.png [file]         caret at rest (rounded square)\n\
                     awl --screenshot-motion OUT.png [file]  caret mid-glide (trailing underline)\n\
                     awl --screenshot-motion-v OUT.png [file] caret mid-glide vertical (left-edge bar)\n\
                     \n\
                     verification hooks (compose with --screenshot):\n\
                     \x20 --sel L0:C0-L1:C1   selection highlight from (l0,c0)..(l1,c1)\n\
                     \x20 --zoom F            zoom factor (0.5..3.0)\n\
                     \x20 --scroll N          scroll N visual rows off the top\n\
                     \x20 --preedit STR       render STR as an IME preedit at the caret\n\
                     \x20 --search STR        open isearch panel for STR + highlight hits\n\
                     \x20 --search-case       make --search case-sensitive\n\
                     \x20 --theme NAME        set the active color theme (Tawny, Potoroo, Gumtree, Bilby, Saltpan, Quokka, Undertow, Outback)\n\
                     \x20 --caret-mode MODE   caret look: block, morph, or auto (default: mono->block, proportional->morph)\n\
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
    Ok(match out {
        Some(out) if motion_v => Mode::ScreenshotMotionVertical { out, file, keys },
        Some(out) if motion => Mode::ScreenshotMotion { out, file, keys },
        Some(out) => Mode::Screenshot {
            out,
            file,
            opts,
            keys,
        },
        None => Mode::Windowed { file },
    })
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

/// Replay a parsed `--keys` action stream against `buffer` THROUGH the shared
/// `actions::apply_core` seam, so headless replay is byte-for-byte identical to
/// live editing. Returns the post-replay `(zoom, selection, search)` App-level
/// state (which lives off the `Buffer`) so the caller can fold it into the
/// capture options. No-op (returns the unchanged baseline) when `keys` is empty.
fn replay_keys(
    buffer: &mut Buffer,
    keys: &[Action],
) -> (Option<f32>, Option<((usize, usize), (usize, usize))>, Option<String>, bool) {
    if keys.is_empty() {
        return (None, None, None, false);
    }
    let mut shift_selecting = false;
    let mut zoom = 1.0f32;
    let mut search: Option<crate::search::SearchState> = None;
    for action in keys {
        let mut ctx = actions::ActionCtx {
            buffer,
            shift_selecting: &mut shift_selecting,
            zoom: &mut zoom,
            search: &mut search,
            // Headless has no viewport to measure; a page is a fixed,
            // deterministic chunk of logical lines.
            page_lines: 20,
        };
        // Replay is unshifted: selection comes from an explicit C-Space mark,
        // matching the emacs-style sticky region the key-spec expresses.
        actions::apply_core(&mut ctx, action, false);
    }
    let zoom_out = if zoom != 1.0 { Some(zoom) } else { None };
    let sel = buffer.selection_line_col();
    let search_query = search.as_ref().map(|s| s.query().to_string());
    let search_case = search.as_ref().map(|s| s.is_case_sensitive()).unwrap_or(false);
    (zoom_out, sel, search_query, search_case)
}

fn main() -> Result<()> {
    match parse_args()? {
        Mode::Screenshot {
            out,
            file,
            mut opts,
            keys,
        } => {
            let mut buffer = load_buffer(&file);
            // Replay `--keys` FIRST so the cursor/selection/search the spec
            // produces are what the capture reflects. Fold the App-level state
            // (zoom / selection / search) the replay produced into the capture
            // opts — but never clobber an explicit verification hook.
            let (z, sel, sq, scs) = replay_keys(&mut buffer, &keys);
            if opts.zoom.is_none() {
                opts.zoom = z;
            }
            if opts.selection.is_none() {
                opts.selection = sel;
            }
            if opts.search.is_none() {
                opts.search = sq;
                opts.search_case_sensitive = opts.search_case_sensitive || scs;
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
            replay_keys(&mut buffer, &keys);
            capture::capture_motion(&out, &buffer)?;
            println!("wrote {} (mid-glide, + sidecar .json)", out.display());
            Ok(())
        }
        Mode::ScreenshotMotionVertical { out, file, keys } => {
            let mut buffer = load_buffer(&file);
            replay_keys(&mut buffer, &keys);
            capture::capture_motion_vertical(&out, &buffer)?;
            println!("wrote {} (mid-glide vertical, + sidecar .json)", out.display());
            Ok(())
        }
        Mode::BenchTyping => bench::run(),
        Mode::Windowed { file } => app::run(file),
    }
}
