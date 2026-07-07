//! Typing micro-benchmark (hidden `--bench-typing` flag).
//!
//! Proves the typing-lag fix: the per-keystroke update path (append one char ->
//! `set_view`) must be BOUNDED and roughly FLAT with document size, not growing
//! with it. We build a headless `TextPipeline` (no window, same shaping path as
//! the live editor) and time, for documents of ~100 / ~1000 / ~5000 lines, the
//! cost of one keystroke two ways on the SAME pipeline:
//!
//!   * BEFORE  — `set_text_full`: cosmic-text's `Buffer::set_text`, which clears +
//!     rebuilds every line and reshapes the WHOLE document each keystroke (the bug).
//!   * AFTER   — `set_view`: the incremental path that reshapes only the edited
//!     line and reuses cached shaping for the rest.
//!
//! Reported as median nanoseconds per keystroke. The AFTER column should stay
//! roughly constant across 100/1000/5000 lines; the BEFORE column should grow.

use glyphon::Cache;

use crate::capture::FORMAT;
use crate::render::{TextPipeline, ViewState};

/// A document of `lines` lines of representative prose (each line is the same
/// realistic width so per-line shaping cost is comparable across sizes).
fn make_doc(lines: usize) -> String {
    let line = "The quick brown fox jumps over the lazy dog while typing fast.";
    let mut s = String::with_capacity(lines * (line.len() + 1));
    for i in 0..lines {
        // Vary the line a touch so they aren't byte-identical (defeats any
        // accidental dedup and is closer to real text).
        s.push_str(line);
        s.push_str(&format!(" #{i}"));
        s.push('\n');
    }
    s
}

/// Build a `ViewState` that appends `extra` to the LAST line of `base` (one
/// keystroke worth of text on the line the cursor sits on), with the cursor at
/// end-of-document. This mirrors the live "type a char at the cursor" update.
fn view_for(base: &str, last_line_idx: usize, last_line: &str) -> ViewState {
    ViewState {
        text: base.to_string(),
        cursor_line: last_line_idx,
        cursor_col: last_line.chars().count(),
        scroll_lines: 0,
        zoom: 1.0,
        selection: None,
        preedit: String::new(),
        misspelled: Vec::new(),
        is_edit_move: false,
        held: false,
        search_matches: Vec::new(),
        search_current: None,
        search_query: String::new(),
        search_active: false,
        search_case_sensitive: false,
        search_replace_active: false,
        search_replacement: String::new(),
        search_editing_replacement: false,
        overlay_active: false,
        overlay_crisp: false,
        overlay_query: String::new(),
        overlay_items: Vec::new(),
        overlay_empty: None,
        overlay_bindings: Vec::new(),
        overlay_times: Vec::new(),
        overlay_git: Vec::new(),
        overlay_selected: 0,
        overlay_scroll: 0,
        overlay_window_rows: 12,
        overlay_hint: String::new(),
        overlay_lens: Vec::new(),
        overlay_sections: Vec::new(),
        caret_preview: None,
        gutter_name: String::new(),
        gutter_project: String::new(),
        is_markdown: false,
        syn_lang: None,
        overlay_spell: None,
        notice: String::new(),
        cjk_priority: crate::frontmatter::DEFAULT_CJK_PRIORITY.to_vec(),
        eol: crate::buffer::Eol::Lf,
    }
}

fn median(mut v: Vec<u128>) -> u128 {
    v.sort_unstable();
    v[v.len() / 2]
}

/// Run the typing benchmark and print a table to stdout. Creates one headless
/// wgpu device (offscreen, no window) and reuses it for every size.
pub fn run() -> anyhow::Result<()> {
    pollster::block_on(run_async())
}

async fn run_async() -> anyhow::Result<()> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .map_err(|e| anyhow::anyhow!("no wgpu adapter for bench: {e:?}"))?;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("awl bench device"),
            ..Default::default()
        })
        .await?;
    let cache = Cache::new(&device);

    // A realistic editor viewport so wrap + shape height match the live app.
    let (width, height) = (1200.0f32, 800.0f32);

    const ITERS: usize = 60;
    const SIZES: [usize; 3] = [100, 1000, 5000];

    println!("typing micro-benchmark (median ns per keystroke: append a char -> reshape)");
    println!(
        "{:>7} | {:>14} | {:>14} | {:>9}",
        "lines", "BEFORE (full)", "AFTER (incr)", "speedup"
    );
    println!("{:->7}-+-{:->14}-+-{:->14}-+-{:->9}", "", "", "", "");

    for &n in &SIZES {
        let doc = make_doc(n);
        let last_idx = n.saturating_sub(1);

        // The document the keystrokes append to: the base doc with one growable
        // last line that the cursor sits at the end of. `text` always ends in '\n',
        // so the last LOGICAL line is the one we grow (insert before that newline).
        let text0 = doc.clone();

        // ---- AFTER: the incremental per-keystroke path via set_view ---------
        // Fresh pipeline shaped on the base doc, then time appending one char per
        // iteration through the SAME set_view path the live editor uses.
        let mut after = TextPipeline::new(&device, &queue, &cache, FORMAT);
        after.set_size(width, height);
        let mut text = text0.clone();
        let last_line = text.lines().nth(last_idx).unwrap_or("").to_string();
        // Prime the cache so the first measured keystroke isn't the cold full shape.
        after.set_view(&view_for(&text, last_idx, &last_line));
        let mut after_samples = Vec::with_capacity(ITERS);
        for k in 0..ITERS {
            // Append one char to the last logical line, before its trailing '\n'.
            let ch = (b'a' + (k % 26) as u8) as char;
            let insert_at = text.len() - 1;
            text.insert(insert_at, ch);
            let last_line = text.lines().nth(last_idx).unwrap_or("").to_string();
            let v = view_for(&text, last_idx, &last_line);
            let t0 = crate::clock::Instant::now();
            after.set_view(&v);
            after_samples.push(t0.elapsed().as_nanos());
        }

        // ---- BEFORE: whole-buffer reshape per keystroke ---------------------
        let mut before = TextPipeline::new(&device, &queue, &cache, FORMAT);
        before.set_size(width, height);
        let mut btext = text0.clone();
        before.set_text_full(&btext);
        let mut before_samples = Vec::with_capacity(ITERS);
        for k in 0..ITERS {
            let ch = (b'a' + (k % 26) as u8) as char;
            let insert_at = btext.len() - 1;
            btext.insert(insert_at, ch);
            let t0 = crate::clock::Instant::now();
            before.set_text_full(&btext);
            before_samples.push(t0.elapsed().as_nanos());
        }

        let b = median(before_samples);
        let a = median(after_samples).max(1);
        let speedup = b as f64 / a as f64;
        println!("{:>7} | {:>11} ns | {:>11} ns | {:>7.1}x", n, b, a, speedup);
    }
    Ok(())
}
