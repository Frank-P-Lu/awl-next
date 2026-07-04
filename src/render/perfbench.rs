//! PERF micro-benchmark (hidden `--bench-perf` flag) for the FIVE traced hot paths.
//!
//! An in-crate `std::time::Instant` harness (the Criterion fallback CAPTURE.md
//! allows for the GPU-coupled [`TextPipeline`]): these hot fns are `pub(super)`
//! methods over a SHAPED cosmic-text buffer, so the cleanest, most representative
//! seam is a child module of `render` — it sees the private methods + fields and
//! times them directly against the real shaping path, no public shims. Mirrors
//! [`crate::bench`] (the typing bench) in shape + reporting.
//!
//! Paths (each timed as median ns per call over the long fixtures under
//! `benches/fixtures`, built headlessly exactly like the capture harness):
//!   * MOTION    — the arrow up/down oracle (`visual_line_down`) marching down the
//!     long PLAIN doc (the settings-file 40fps path).
//!   * ORNAMENTS — `rule_marks` + `bullet_marks` over the long MARKDOWN doc (the
//!     long-md-doc palette-open ~39fps path; the biggest offender).
//!   * CONCEAL   — `refresh_rule_conceal` over the long markdown doc, repeated with
//!     the caret line UNCHANGED (the pure-scroll cost F2 gates).
//!   * THEME     — `sync_theme`'s font-branch reshape over the long doc (a
//!     display-face theme switch).

use glyphon::Cache;
use std::path::Path;

use crate::actions::LayoutOracle;
use crate::buffer::Buffer;
use crate::capture::FORMAT;

use super::{TextPipeline, ViewState};

fn median(mut v: Vec<u128>) -> u128 {
    v.sort_unstable();
    v[v.len() / 2]
}

/// A `ViewState` at `(cursor)` for `buffer`, every search / overlay field inert —
/// enough to shape the document through `set_view` (the fields beyond text / zoom /
/// cursor / markdown / syntax don't touch the paths we bench).
fn bench_view(buffer: &Buffer, cursor: (usize, usize)) -> ViewState {
    ViewState {
        text: buffer.text(),
        cursor_line: cursor.0,
        cursor_col: cursor.1,
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
        overlay_bindings: Vec::new(),
        overlay_times: Vec::new(),
        overlay_selected: 0,
        overlay_scroll: 0,
        overlay_hint: String::new(),
        overlay_lens: Vec::new(),
        overlay_sections: Vec::new(),
        caret_preview: None,
        gutter_name: buffer.display_name(),
        gutter_project: String::new(),
        is_markdown: buffer.is_markdown(),
        syn_lang: buffer.syntax_lang(),
        overlay_spell: None,
        notice: String::new(),
        cjk_priority: crate::frontmatter::DEFAULT_CJK_PRIORITY.to_vec(),
    }
}

/// Load a fixture buffer from `benches/fixtures/<name>`, resolved relative to the
/// crate manifest dir so the bench works from any cwd.
fn fixture(name: &str) -> Buffer {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("benches/fixtures")
        .join(name);
    Buffer::from_file(&path)
}

/// Run the perf benchmark and print a table to stdout. One headless wgpu device
/// (offscreen, no window), reused across every path.
pub fn run() -> anyhow::Result<()> {
    pollster::block_on(run_async())
}

async fn run_async() -> anyhow::Result<()> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .map_err(|e| anyhow::anyhow!("no wgpu adapter for perf bench: {e:?}"))?;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("awl perf bench device"),
            ..Default::default()
        })
        .await?;
    let cache = Cache::new(&device);
    let (width, height) = (1200.0f32, 800.0f32);

    let md = fixture("long_bullets.md");
    let plain = fixture("long_plain.txt");
    let md_lines = md.text().lines().count();
    let plain_lines = plain.text().lines().count();

    println!("perf micro-benchmark — median ns per call over the long fixtures");
    println!("  long_bullets.md: {md_lines} lines   long_plain.txt: {plain_lines} lines");
    println!("{:>10} | {:>14} | {:>28}", "path", "median ns", "detail");
    println!("{:->10}-+-{:->14}-+-{:->28}", "", "", "");

    // ---- MOTION: visual_line_down marching down the long PLAIN doc -----------
    {
        let mut p = TextPipeline::new(&device, &queue, &cache, FORMAT);
        p.set_size(width, height);
        p.set_view(&bench_view(&plain, (0, 0)));
        let last = plain.text().lines().count().saturating_sub(1);
        // March the caret down the whole doc a few times; each step is one arrow-down.
        const PASSES: usize = 4;
        let mut samples = Vec::with_capacity(PASSES * last);
        for _ in 0..PASSES {
            let mut line = 0usize;
            let col = 0usize;
            let goal_x = 0.0f32;
            while line < last {
                let t0 = crate::clock::Instant::now();
                let (nl, _nc) = p.visual_line_down(line, col, goal_x);
                samples.push(t0.elapsed().as_nanos());
                line = if nl > line { nl } else { line + 1 };
            }
        }
        println!(
            "{:>10} | {:>14} | {:>28}",
            "MOTION",
            median(samples),
            "visual_line_down / arrow-down"
        );
    }

    // ---- ORNAMENTS: rule_marks + bullet_marks over the long MARKDOWN doc ------
    {
        let mut p = TextPipeline::new(&device, &queue, &cache, FORMAT);
        p.set_size(width, height);
        p.set_view(&bench_view(&md, (0, 0)));
        const ITERS: usize = 200;
        let mut samples = Vec::with_capacity(ITERS);
        let mut sink = 0usize;
        for _ in 0..ITERS {
            let t0 = crate::clock::Instant::now();
            let r = p.rule_marks();
            let b = p.bullet_marks();
            samples.push(t0.elapsed().as_nanos());
            sink = sink.wrapping_add(r.len()).wrapping_add(b.len());
        }
        std::hint::black_box(sink);
        println!(
            "{:>10} | {:>14} | {:>28}",
            "ORNAMENTS",
            median(samples),
            "rule_marks + bullet_marks"
        );
    }

    // ---- CONCEAL: refresh_rule_conceal, caret line UNCHANGED (pure-scroll) ----
    {
        let mut p = TextPipeline::new(&device, &queue, &cache, FORMAT);
        p.set_size(width, height);
        p.set_view(&bench_view(&md, (0, 0)));
        // Prime once so the first measured call is the settled repeat (the pure
        // scroll / redraw case: caret line never moves, spans already laid).
        p.refresh_rule_conceal(false);
        const ITERS: usize = 400;
        let mut samples = Vec::with_capacity(ITERS);
        for _ in 0..ITERS {
            let t0 = crate::clock::Instant::now();
            p.refresh_rule_conceal(false);
            samples.push(t0.elapsed().as_nanos());
        }
        println!(
            "{:>10} | {:>14} | {:>28}",
            "CONCEAL",
            median(samples),
            "refresh_rule_conceal (no move)"
        );
    }

    // ---- THEME: sync_theme font-branch reshape over the long MARKDOWN doc -----
    {
        let mut p = TextPipeline::new(&device, &queue, &cache, FORMAT);
        p.set_size(width, height);
        p.set_view(&bench_view(&md, (0, 0)));
        const ITERS: usize = 40;
        let mut samples = Vec::with_capacity(ITERS);
        // Alternate between two REAL worlds with DIFFERENT display faces
        // (Literata <-> JetBrains Mono) so every iteration pays the GENUINE
        // family change: cosmic-text's `set_attrs_list` resets a line's shaping
        // only when the attrs actually DIFFER, so the old form here — forcing
        // the branch via a fake `shaped_font` while the active face stayed the
        // same — rebuilt identical attrs, no-oped line-by-line, and measured
        // only the attrs-rebuild (~5 ms), reading ~45x under the real live
        // switch (see `--bench-theme-burst` for the full per-switch profile).
        let worlds = ["Gumtree", "Currawong"];
        for i in 0..ITERS {
            crate::theme::set_active_by_name(worlds[i % 2]);
            let t0 = crate::clock::Instant::now();
            p.sync_theme();
            samples.push(t0.elapsed().as_nanos());
        }
        crate::theme::set_active(crate::theme::DEFAULT_THEME);
        println!(
            "{:>10} | {:>14} | {:>28}",
            "THEME",
            median(samples),
            "sync_theme font reshape"
        );
    }

    Ok(())
}
