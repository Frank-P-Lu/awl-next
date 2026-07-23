//! CARET LOOKUP WITNESS (hidden `--bench-caret` flag, item 57) — proves the caret's
//! per-frame glyph lookup cost is INDEPENDENT of the caret's document position.
//!
//! The caret's animated look reads, every frame, the glyph the cursor inhabits: its
//! `CacheKey` (morph masks), its ink box (block width/x), its descender depth (block
//! bottom drop), and its baseline y. Those lookups USED to filter the whole
//! document's `layout_runs()` stream, breaking once past the cursor line — so each
//! visited one shaped run per visual row of the entire PREFIX before the caret, and
//! the cost grew with how far down the document the caret sat, re-paid every
//! animating frame.
//!
//! This harness places the SAME caret cases at the document TOP, MIDDLE, and TAIL of
//! a long fixture and records, per position:
//!   * `prefix_runs`  — the shaped runs a whole-doc `layout_runs()` walk would visit
//!     before the cursor line (the OLD cost's driver — GROWS top→tail).
//!   * `local_glyphs` — the glyph clusters the TARGET-LINE-LOCAL record actually
//!     visits (the fixed lookup's work; asserted NONZERO so we never "measure" 0
//!     work — and a function of the cursor LINE alone, so it is FLAT top→tail).
//!   * `old_ns`       — median time of one whole-prefix walk (what a single old
//!     per-frame lookup cost).
//!   * `new_ns`       — median time of the real fixed per-frame caret glyph bundle
//!     (baseline + descender + ink-box + inhabited-key), warm — FLAT top→tail.
//!
//! A child module of `render` (like [`super::perfbench`]) so it reaches the
//! `pub(super)` caret methods + private fields directly, timed against the real
//! shaping path with no public shims. Dev-only; never on the render path.

use glyphon::Cache;
use std::path::Path;

use crate::buffer::Buffer;
use crate::capture::FORMAT;

use super::{TextPipeline, ViewState};

fn median(mut v: Vec<u128>) -> u128 {
    v.sort_unstable();
    v[v.len() / 2]
}

/// The number of shaped runs a WHOLE-DOCUMENT `layout_runs()` walk would visit before
/// it reached (and broke past) the cursor line — i.e. the prefix the OLD caret lookup
/// re-walked every frame. A structural fact of the shaped document (GROWS with the
/// cursor's position). Lives HERE, not in `caret.rs`, so the caret render module stays
/// free of any `layout_runs()` call (banned by `caret_no_whole_doc_walk_law`). Reads
/// `TextPipeline`'s private fields directly — a descendant module of `render`.
fn prefix_run_count(p: &TextPipeline) -> usize {
    let mut visited = 0usize;
    for run in p.buffer.layout_runs() {
        if run.line_i > p.cursor_line {
            break;
        }
        visited += 1;
    }
    visited
}

/// A `ViewState` at `(cursor)` for `buffer`, every search / overlay field inert (the
/// perfbench pattern) — enough to shape the document through `set_view`.
fn bench_view(buffer: &Buffer, cursor: (usize, usize)) -> ViewState {
    ViewState {
        text: buffer.text(),
        cursor_line: cursor.0,
        cursor_col: cursor.1,
        gutter_name: buffer.display_name(),
        is_markdown: buffer.is_markdown(),
        syn_lang: buffer.syntax_lang(),
        eol: buffer.eol(),
        ..ViewState::base()
    }
}

fn fixture(name: &str) -> Buffer {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("benches/fixtures")
        .join(name);
    Buffer::from_file(&path)
}

/// One measured (position, cursor line) cell.
struct Cell {
    label: &'static str,
    line: usize,
    prefix_runs: usize,
    local_glyphs: usize,
    old_ns: u128,
    new_ns: u128,
}

pub fn run() -> anyhow::Result<()> {
    pollster::block_on(run_async())
}

async fn run_async() -> anyhow::Result<()> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .map_err(|e| anyhow::anyhow!("no wgpu adapter for caret bench: {e:?}"))?;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("awl caret bench device"),
            ..Default::default()
        })
        .await?;
    let cache = Cache::new(&device);
    let (width, height) = (1200.0f32, 800.0f32);

    let doc = fixture("long_plain.txt");
    let text = doc.text();
    let lines: Vec<&str> = text.lines().collect();
    let n = lines.len();

    // Pick a content-bearing line (>= a few glyphs) at/after `near`, so every case
    // anchors on a real glyph the lookups actually resolve — no blank-line no-op.
    let content_line = |near: usize| -> usize {
        (near..n)
            .chain((0..near).rev())
            .find(|&i| lines.get(i).map(|l| l.chars().count() >= 6).unwrap_or(false))
            .unwrap_or(0)
    };
    let top = content_line(2);
    let middle = content_line(n / 2);
    let tail = content_line(n.saturating_sub(3));

    const ITERS: usize = 400;
    let mut p = TextPipeline::new(&device, &queue, &cache, FORMAT);
    p.set_size(width, height);

    let mut cells: Vec<Cell> = Vec::new();
    for (label, line) in [("TOP", top), ("MIDDLE", middle), ("TAIL", tail)] {
        // Anchor a few glyphs in, on the chosen content line.
        p.set_view(&bench_view(&doc, (line, 3)));

        // Structural counts (position-dependent prefix vs line-local work).
        let prefix_runs = prefix_run_count(&p);
        let local_glyphs = p.caret_line_glyph_count();
        // The whole point of the fix: the lookup MUST touch real work (a nonzero
        // line-local glyph count) — never "measure" 0 work.
        anyhow::ensure!(
            local_glyphs > 0,
            "{label}: caret line-local glyph count is 0 — the witness measured no work \
             (cursor line {line} shaped empty?)",
        );

        // Warm the target-line-local record + the cursor-line visual-row memo, exactly
        // as the first per-frame lookup does, so the timed loop is the STEADY cost.
        let _ = p.caret_baseline_y();
        let _ = p.cursor_glyph_descender();
        let _ = p.caret_anchor_ink_box();
        let _ = p.caret_inhabited_key();

        // OLD-cost proxy: one whole-prefix `layout_runs()` walk (the inner loop each
        // old per-frame caret lookup ran; a real old frame paid several of these).
        let mut old = Vec::with_capacity(ITERS);
        for _ in 0..ITERS {
            let t0 = crate::clock::Instant::now();
            let v = prefix_run_count(&p);
            old.push(t0.elapsed().as_nanos());
            std::hint::black_box(v);
        }

        // NEW cost: the real fixed per-frame caret glyph bundle, warm.
        let mut new = Vec::with_capacity(ITERS);
        for _ in 0..ITERS {
            let t0 = crate::clock::Instant::now();
            let a = p.caret_baseline_y();
            let b = p.cursor_glyph_descender();
            let c = p.caret_anchor_ink_box();
            let d = p.caret_inhabited_key();
            new.push(t0.elapsed().as_nanos());
            std::hint::black_box((a, b, c, d.is_some()));
        }

        cells.push(Cell {
            label,
            line,
            prefix_runs,
            local_glyphs,
            old_ns: median(old),
            new_ns: median(new),
        });
    }

    println!("caret lookup witness (item 57) — long_plain.txt: {n} lines");
    println!(
        "  cost independent of document position when new_ns stays flat while prefix_runs grows"
    );
    println!(
        "{:>7} | {:>6} | {:>11} | {:>12} | {:>14} | {:>14}",
        "pos", "line", "prefix_runs", "local_glyphs", "old walk ns", "new lookup ns"
    );
    println!(
        "{:->7}-+-{:->6}-+-{:->11}-+-{:->12}-+-{:->14}-+-{:->14}",
        "", "", "", "", "", ""
    );
    for c in &cells {
        println!(
            "{:>7} | {:>6} | {:>11} | {:>12} | {:>14} | {:>14}",
            c.label, c.line, c.prefix_runs, c.local_glyphs, c.old_ns, c.new_ns
        );
    }

    // Machine-readable verdict line. `prefix_runs` must grow top→tail (the prefix the
    // old walk paid); `local_glyphs` must stay constant (line-local work). The `new_ns`
    // flatness is the human-confirmed perf claim (dev frames lie; judge in --release).
    let top_c = &cells[0];
    let tail_c = &cells[2];
    let prefix_grew = tail_c.prefix_runs > top_c.prefix_runs;
    println!(
        "verdict: prefix_runs {} top→tail ({} → {}); local_glyphs {} ({} vs {})",
        if prefix_grew { "GREW" } else { "flat" },
        top_c.prefix_runs,
        tail_c.prefix_runs,
        if top_c.local_glyphs == tail_c.local_glyphs {
            "CONSTANT"
        } else {
            "varied (different-length lines)"
        },
        top_c.local_glyphs,
        tail_c.local_glyphs,
    );

    Ok(())
}
