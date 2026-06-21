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

mod app;
mod bench;
mod buffer;
mod capture;
mod caret;
mod keymap;
mod render;
mod selection;
mod spell;
mod spellunderline;

use std::path::PathBuf;

use anyhow::{bail, Result};

use crate::buffer::Buffer;
use crate::capture::CaptureOpts;

enum Mode {
    Windowed { file: Option<PathBuf> },
    /// Deterministic one-frame capture with the caret AT REST (the resting amber
    /// rounded square on the glyph), plus optional zoom / scroll / selection
    /// verification overrides.
    Screenshot {
        out: PathBuf,
        file: Option<PathBuf>,
        opts: CaptureOpts,
    },
    /// Deterministic one-frame capture of a caret MID-GLIDE (dropped to the
    /// baseline and stretched into a trailing underline streak), so the temporal
    /// effect is inspectable from a still.
    ScreenshotMotion { out: PathBuf, file: Option<PathBuf> },
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
    let mut file: Option<PathBuf> = None;
    let mut opts = CaptureOpts::default();
    let mut bench_typing = false;

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
            "-h" | "--help" => {
                println!(
                    "awl [file]\n\
                     awl --screenshot OUT.png [file]         caret at rest (rounded square)\n\
                     awl --screenshot-motion OUT.png [file]  caret mid-glide (trailing underline)\n\
                     \n\
                     verification hooks (compose with --screenshot):\n\
                     \x20 --sel L0:C0-L1:C1   selection highlight from (l0,c0)..(l1,c1)\n\
                     \x20 --zoom F            zoom factor (0.5..3.0)\n\
                     \x20 --scroll N          scroll N visual rows off the top\n\
                     \x20 --preedit STR       render STR as an IME preedit at the caret"
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
    Ok(match (out, motion) {
        (Some(out), true) => Mode::ScreenshotMotion { out, file },
        (Some(out), false) => Mode::Screenshot { out, file, opts },
        (None, _) => Mode::Windowed { file },
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

fn main() -> Result<()> {
    match parse_args()? {
        Mode::Screenshot { out, file, opts } => {
            let mut buffer = load_buffer(&file);
            // If a selection is requested, move the buffer cursor to its END so
            // the caret renders at the cursor end of the region (consistent with
            // the highlight). The selection itself is drawn via opts.selection.
            if let Some((_, (l1, c1))) = opts.selection {
                let end = buffer.line_col_to_char(l1, c1);
                buffer.set_cursor(end);
            }
            capture::capture_with(&out, &buffer, &opts)?;
            println!("wrote {} (+ sidecar .json)", out.display());
            Ok(())
        }
        Mode::ScreenshotMotion { out, file } => {
            let buffer = load_buffer(&file);
            capture::capture_motion(&out, &buffer)?;
            println!("wrote {} (mid-glide, + sidecar .json)", out.display());
            Ok(())
        }
        Mode::BenchTyping => bench::run(),
        Mode::Windowed { file } => app::run(file),
    }
}
