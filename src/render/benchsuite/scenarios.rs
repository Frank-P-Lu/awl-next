//! The `--bench-suite` SCENARIO matrix: eight real interaction shapes, each
//! run against one corpus tier's pipeline and each WITNESSED — every scenario
//! must prove the work it timed actually happened (reshape counts, row
//! deltas, match counts, changed pixels) or the whole suite FAILS. The old
//! `--bench-perf` THEME stage once printed ~5 ms while nothing reshaped; a
//! cell here that can silently measure nothing is a defect (CLAUDE.md's
//! bench-witness law), so witnesses are `ensure!`s, not comments.
//!
//! Frames replay the live `RedrawRequested` aggregate (`advance` →
//! `prepare` → encode → submit+poll → `atlas.trim`), exactly like
//! [`super::super::framebench`]'s zoom profiler; the blocking poll serializes
//! GPU cost into the number. Pixel witnesses read the offscreen target back
//! through the SAME `capture::gpu::read_frame` the capture harness uses.

use anyhow::{ensure, Context as _, Result};

use crate::clock::Instant;
use crate::search::{Direction, SearchState};

use super::corpus::{self, Tier};
use super::cx::{differing_pixels, ms, Cx};
use super::{HEIGHT, WIDTH};

/// The scenario axis. `ALL` is the per-tier run order; `name` is a no-wildcard
/// match so a new scenario fails to compile until it is placed everywhere.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Scenario {
    ColdOpen,
    Typing,
    Scroll,
    Search,
    Palette,
    Zoom,
    Theme,
    Resize,
}

impl Scenario {
    pub(super) const ALL: [Scenario; 8] = [
        Scenario::ColdOpen,
        Scenario::Typing,
        Scenario::Scroll,
        Scenario::Search,
        Scenario::Palette,
        Scenario::Zoom,
        Scenario::Theme,
        Scenario::Resize,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            Scenario::ColdOpen => "cold_open",
            Scenario::Typing => "typing",
            Scenario::Scroll => "scroll",
            Scenario::Search => "search",
            Scenario::Palette => "palette",
            Scenario::Zoom => "zoom",
            Scenario::Theme => "theme",
            Scenario::Resize => "resize",
        }
    }
}

/// The skip table — every hole in the matrix is NAMED here with its reason
/// (documented skips, no silent gaps). Anything not listed runs.
pub(super) fn skip_reason(tier: Tier, sc: Scenario) -> Option<&'static str> {
    match (tier, sc) {
        // Zoom is a prose-READING affordance (the spec'd example skip): the
        // zoom burst's live posture is a reader resizing their page, and the
        // prose tiers (incl. both pathologies) already cover the reflow cost
        // ladder. A code buffer adds no new zoom-path work.
        (Tier::Code, Scenario::Zoom) => Some("zoom burst is a prose-reading affordance"),
        // BANKED on a real, pre-existing product gap this tier EXPOSED:
        // `full_shape_height` budgets ~8 wrapped rows per logical line, so the
        // enormous single-line paragraph is only partially shaped and
        // `total_visual_rows` under-reports IDENTICALLY at both widths — the
        // re-wrap witness structurally cannot see its work (66 -> 66). Skipped
        // rather than run witness-blind; see benches/README.md ("Known
        // pathologies the suite exposed") for the banked follow-up.
        (Tier::XPara, Scenario::Resize) => {
            Some("full_shape_height's 8-rows/line budget blinds the row-count witness on one enormous line")
        }
        _ => None,
    }
}

/// One cell's raw output: the per-sample wall times and the witness counters
/// that prove the work happened (recorded into bench.json beside the timings).
pub(super) struct CellOut {
    pub samples_ms: Vec<f64>,
    pub witness: Vec<(&'static str, u64)>,
}

pub(super) fn run_scenario(sc: Scenario, cx: &mut Cx) -> Result<CellOut> {
    match sc {
        Scenario::ColdOpen => cold_open(cx),
        Scenario::Typing => typing(cx),
        Scenario::Scroll => scroll(cx),
        Scenario::Search => search(cx),
        Scenario::Palette => palette(cx),
        Scenario::Zoom => zoom(cx),
        Scenario::Theme => theme(cx),
        Scenario::Resize => resize(cx),
    }
}

/// COLD OPEN -> first settled frame: swap the document in (the single-instance
/// editor's real "open a file" shape — the previous buffer leaves, the new one
/// shapes in full) and draw its first frame. WITNESS: exactly two reshapes per
/// sample (park-to-empty + open), and the shaped document reports its rows.
fn cold_open(cx: &mut Cx) -> Result<CellOut> {
    const SAMPLES: usize = 5;
    let mut samples = Vec::with_capacity(SAMPLES);
    let before = cx.p.reshape_count;
    for _ in 0..SAMPLES {
        // Park on an empty document (untimed) so the timed open is a true full shape.
        cx.view.text = String::new();
        cx.view.cursor_line = 0;
        cx.view.cursor_col = 0;
        cx.sync_frame()?;
        cx.view.text = cx.text.clone();
        let t0 = Instant::now();
        cx.sync_frame()?;
        samples.push(ms(t0));
    }
    let reshapes = cx.p.reshape_count - before;
    ensure!(
        reshapes == 2 * SAMPLES as u64,
        "cold_open must reshape exactly twice per sample (park + open), got {reshapes} over {SAMPLES}"
    );
    let rows = cx.p.total_visual_rows();
    ensure!(
        rows >= cx.lines && rows > 0,
        "cold_open must leave the whole document shaped ({} logical lines, {} visual rows)",
        cx.lines,
        rows
    );
    Ok(CellOut {
        samples_ms: samples,
        witness: vec![
            ("reshapes", reshapes),
            ("rows", rows as u64),
            ("words", cx.words),
            // The corpus fingerprint rides the witness block, so a baseline
            // diff flags a drifted workload (a corpus/generator change) as
            // witness drift on every tier — never a silent apples-to-oranges
            // timing comparison.
            ("corpus_fnv", corpus::fingerprint(&cx.text)),
        ],
    })
}

/// TYPING BURST (the `--bench-typing` workload shape, end-to-end): insert one
/// char at the caret per keystroke and pay set_view + one frame — the live
/// key→pixel path. The caret sits at the TOP of the document (line 0 is on
/// screen at scroll 0 in EVERY tier — including XPARA, whose enormous single
/// line under-reports its visual-row count, so a tail-following scroll cannot
/// be this scenario's oracle; the first witness run caught exactly that).
/// WITNESS: exactly one reshape per keystroke (the incremental path still
/// reshapes the edited line — for XPARA that line IS the whole pathology) and
/// the typed text visibly changed the frame.
fn typing(cx: &mut Cx) -> Result<CellOut> {
    const KEYS: usize = 30;
    let mut text = cx.text.clone();
    cx.view.scroll_lines = 0;
    cx.view.cursor_line = 0;
    cx.view.is_edit_move = true;
    // Prime one keystroke untimed so the first sample isn't a cold outlier.
    let mut col = 0usize;
    text.insert(col, 'z');
    col += 1;
    cx.view.cursor_col = col;
    cx.view.text.clone_from(&text);
    cx.sync_frame()?;
    let first = cx.snapshot()?;

    let before = cx.p.reshape_count;
    let mut samples = Vec::with_capacity(KEYS);
    for k in 0..KEYS {
        let ch = (b'a' + (k % 26) as u8) as char;
        text.insert(col, ch);
        col += 1;
        cx.view.cursor_col = col;
        cx.view.text.clone_from(&text);
        let t0 = Instant::now();
        cx.sync_frame()?;
        samples.push(ms(t0));
    }
    let reshapes = cx.p.reshape_count - before;
    ensure!(
        reshapes == KEYS as u64,
        "typing must reshape exactly once per keystroke, got {reshapes} over {KEYS}"
    );
    let last = cx.snapshot()?;
    let changed = differing_pixels(&first, &last);
    ensure!(changed > 0, "typed characters must change the rendered frame");

    // Restore the pristine document (untimed).
    cx.view.text.clone_from(&cx.text);
    cx.view.is_edit_move = false;
    cx.view.cursor_line = 0;
    cx.view.cursor_col = 0;
    cx.view.scroll_lines = 0;
    cx.sync_frame()?;
    Ok(CellOut {
        samples_ms: samples,
        witness: vec![("reshapes", reshapes), ("pixels_changed", changed)],
    })
}

/// SCROLL page-through + jump-to-end (M-> semantics): step the viewport a page
/// of visual rows at a time, then land the caret at the end of the document.
/// WITNESS the OUTCOME, not the loop: after every timed step the pixel offset
/// the renderer RESOLVED for the applied scroll row (`row_top_px` — the same
/// number `doc_top()` positions the document by) must have strictly advanced,
/// the page-through reshapes ZERO times (the O(visible) law — a pure scroll
/// must never reshape), the jump must move the viewport off the top, and the
/// end frame differs from the top frame. A no-op'd scroll (viewport pinned at
/// row 0) fails the first ensure — frame-to-frame caret animation can never
/// satisfy an offset witness (the vacuous-witness defect this cell shipped
/// with: its pixel diff passed on caret motion alone).
fn scroll(cx: &mut Cx) -> Result<CellOut> {
    const PAGE: usize = 24;
    const MAX_STEPS: usize = 16;
    // Short documents page through in 2-3 steps — too few samples for a
    // stable min under a co-loaded machine (the calibration runs saw a
    // 3-sample cell swing +226%/-64% on identical code) — so the page-through
    // REPEATS until at least this many samples exist.
    const MIN_SAMPLES: usize = 8;
    let rows = cx.p.total_visual_rows();
    let steps = (rows.saturating_sub(1) / PAGE).clamp(1, MAX_STEPS);
    let passes = MIN_SAMPLES.div_ceil(steps);
    cx.view.scroll_lines = 0;
    cx.sync_frame()?;
    let top = cx.snapshot()?;

    let before = cx.p.reshape_count;
    let mut samples = Vec::with_capacity(steps * passes + 1);
    let mut deepest_px = 0.0f32;
    for _ in 0..passes {
        cx.view.scroll_lines = 0;
        cx.sync_frame()?; // rewind to the top, untimed
        let mut prev_px = cx.p.row_top_px(cx.p.scroll_lines);
        for i in 1..=steps {
            cx.view.scroll_lines = (i * PAGE).min(rows.saturating_sub(1));
            let t0 = Instant::now();
            cx.sync_frame()?;
            samples.push(ms(t0));
            let px = cx.p.row_top_px(cx.p.scroll_lines);
            ensure!(
                px > prev_px,
                "every scroll step must advance the resolved viewport offset \
                 (row {} resolves {px}px; the previous step held {prev_px}px)",
                cx.p.scroll_lines
            );
            prev_px = px;
        }
        deepest_px = prev_px;
    }
    let scroll_reshapes = cx.p.reshape_count - before;
    ensure!(
        scroll_reshapes == 0,
        "a pure scroll page-through must schedule ZERO reshapes, got {scroll_reshapes}"
    );

    // Jump to end: caret onto the last line's end, viewport at the tail.
    let last_line = cx.lines - 1;
    let body = &cx.text[..cx.text.len() - 1];
    let line_start = body.rfind('\n').map_or(0, |i| i + 1);
    cx.view.cursor_line = last_line;
    cx.view.cursor_col = body[line_start..].chars().count();
    cx.view.scroll_lines = rows.saturating_sub(10);
    let t0 = Instant::now();
    cx.sync_frame()?;
    samples.push(ms(t0));
    let jump_px = cx.p.row_top_px(cx.p.scroll_lines);
    ensure!(
        jump_px > 0.0,
        "the jump to end must move the viewport off the top (resolved {jump_px}px)"
    );

    let end = cx.snapshot()?;
    let changed = differing_pixels(&top, &end);
    ensure!(changed > 0, "scrolling to the end must change the rendered frame");

    // Restore (untimed).
    cx.view.cursor_line = 0;
    cx.view.cursor_col = 0;
    cx.view.scroll_lines = 0;
    cx.sync_frame()?;
    Ok(CellOut {
        samples_ms: samples,
        witness: vec![
            ("pages", (steps * passes) as u64),
            ("scroll_reshapes", scroll_reshapes),
            // The OUTCOME counter: the deepest resolved offset the page-through
            // reached (deterministic per corpus + wrap — a workload/geometry
            // change shows up as baseline witness drift, not a silent no-op).
            ("scrolled_px", deepest_px.round() as u64),
            ("pixels_changed", changed),
        ],
    })
}

/// SEARCH: type the query char by char (each keystroke recomputes matches over
/// the whole document — the real isearch cost) then step to the next match six
/// times, drawing a frame per step. WITNESS: the engine's match count equals
/// an INDEPENDENT substring count (not find_all counted twice), and a current
/// match exists.
fn search(cx: &mut Cx) -> Result<CellOut> {
    const QUERY: &str = "the ";
    const NEXTS: usize = 6;
    let independent = cx.text.matches(QUERY).count() as u64;
    ensure!(independent > 0, "the corpus must contain the search query");

    let mut s = SearchState::start(0, Direction::Forward);
    // Case-sensitive so the independent `str::matches` oracle and the engine
    // agree on the same set by construction.
    s.toggle_case(&cx.text);
    let mut query_live = String::new();
    let mut samples = Vec::with_capacity(QUERY.len() + NEXTS);
    for c in QUERY.chars() {
        query_live.push(c);
        let t0 = Instant::now();
        s.push_char(c, &cx.text);
        apply_search_view(cx, &s, &query_live)?;
        samples.push(ms(t0));
    }
    for _ in 0..NEXTS {
        let t0 = Instant::now();
        s.step(Direction::Forward);
        apply_search_view(cx, &s, &query_live)?;
        samples.push(ms(t0));
    }
    let matches = s.matches().len() as u64;
    ensure!(
        matches == independent,
        "search must find every occurrence: engine {matches} vs independent {independent}"
    );
    ensure!(s.current_index().is_some(), "search must land on a current match");

    // Close the panel (untimed).
    cx.view.search_active = false;
    cx.view.search_matches = Vec::new();
    cx.view.search_current = None;
    cx.view.search_query = String::new();
    cx.view.search_case_sensitive = false;
    cx.view.cursor_line = 0;
    cx.view.cursor_col = 0;
    cx.sync_frame()?;
    Ok(CellOut {
        samples_ms: samples,
        witness: vec![("matches", matches), ("steps", (QUERY.len() + NEXTS) as u64)],
    })
}

/// Fold live search state into the working view (the same match->line/col
/// conversion the capture harness performs) and draw the frame.
fn apply_search_view(cx: &mut Cx, s: &SearchState, query: &str) -> Result<()> {
    cx.view.search_active = true;
    cx.view.search_case_sensitive = true;
    cx.view.search_query = query.to_string();
    cx.view.search_matches = s
        .matches()
        .iter()
        .map(|m| {
            (
                cx.buffer.char_to_line_col(m.start),
                cx.buffer.char_to_line_col(m.end),
            )
        })
        .collect();
    cx.view.search_current = s.current_index();
    if let Some(m) = s.current_match() {
        let (l, c) = cx.buffer.char_to_line_col(m.start);
        cx.view.cursor_line = l;
        cx.view.cursor_col = c;
    }
    cx.sync_frame()
}

/// PALETTE OPEN: build the real command palette (catalog ∪ settings rows, the
/// same `overlay::build` the live app and the replay share) and draw it over
/// the document — scrim, frosted backdrop, card, rows. WITNESS: the palette
/// has rows, the row pipeline uploaded instances, and the card visibly drew.
fn palette(cx: &mut Cx) -> Result<CellOut> {
    const SAMPLES: usize = 5;
    let baseline_frame = cx.snapshot()?;
    let keep = cx.config.effective_linux_keep();
    let temp_root = std::env::temp_dir();
    let mut samples = Vec::with_capacity(SAMPLES);
    let mut items_len = 0u64;
    let mut instances = 0u64;
    let mut open_frame: Option<image::RgbaImage> = None;
    for i in 0..SAMPLES {
        let build_ctx = crate::overlay::BuildCtx {
            goto_corpus: Vec::new(),
            goto_open: Vec::new(),
            goto_recent: Vec::new(),
            goto_times: Vec::new(),
            config_keys: &cx.config.keys,
            config_linux_keep: &keep,
            goto_headings: Vec::new(),
            spell_target: None,
            history_entries: Vec::new(),
            history_now: None,
            history_session_start: None,
            settings_values: crate::settings::SettingsValues::gather(
                cx.config,
                &temp_root,
                1.0,
                crate::dateformat::CAPTURE_PLACEHOLDER_YMD,
            ),
            assets: Vec::new(),
            has_waiter: false,
        };
        let t0 = Instant::now();
        let ov = crate::overlay::build(crate::overlay::OverlayKind::Command, &build_ctx)
            .context("the command palette must build")?;
        cx.view.overlay_active = true;
        cx.view.overlay_crisp = false;
        cx.view.overlay_title = ov.kind.title();
        cx.view.overlay_query = String::new();
        cx.view.overlay_items = ov.item_strings();
        cx.view.overlay_bindings = ov.item_bindings();
        cx.view.overlay_git = ov.item_git_tags();
        cx.view.overlay_empty = ov.empty_notice();
        cx.view.overlay_selected = ov.selected;
        cx.view.overlay_scroll = ov.scroll;
        cx.view.overlay_window_rows = ov.window_rows();
        cx.view.overlay_hint = ov.foot_hint();
        cx.sync_frame()?;
        samples.push(ms(t0));
        items_len = cx.view.overlay_items.len() as u64;
        instances = cx.p.overlay_rows.instance_count() as u64;
        if i == 0 {
            open_frame = Some(cx.snapshot()?);
        }
        // Close it again (untimed) so every sample pays the full open.
        cx.view.overlay_active = false;
        cx.view.overlay_items = Vec::new();
        cx.view.overlay_bindings = Vec::new();
        cx.view.overlay_git = Vec::new();
        cx.view.overlay_title = "";
        cx.view.overlay_hint = String::new();
        cx.view.overlay_empty = None;
        cx.sync_frame()?;
    }
    ensure!(items_len > 0, "the command palette must carry rows");
    ensure!(instances > 0, "the palette rows must upload row instances");
    let changed = differing_pixels(&baseline_frame, &open_frame.expect("snapshotted above"));
    ensure!(changed > 0, "the open palette must visibly change the frame");
    Ok(CellOut {
        samples_ms: samples,
        witness: vec![
            ("items", items_len),
            ("row_instances", instances),
            ("pixels_changed", changed),
        ],
    })
}

/// ZOOM BURST (the `--bench-zoom-burst` workload shape, per tier): five rapid
/// adjacent zoom requests applied eagerly, then the first frame at the final
/// level. WITNESS: exactly one reshape per requested level (the existing
/// zoom-burst law), every sample.
fn zoom(cx: &mut Cx) -> Result<CellOut> {
    const LEVELS: [f32; 5] = [1.1, 1.2, 1.1, 1.0, 1.1];
    const SAMPLES: usize = 5;
    let mut samples = Vec::with_capacity(SAMPLES);
    let mut total_reshapes = 0u64;
    for _ in 0..SAMPLES {
        cx.view.zoom = 1.0;
        cx.sync_frame()?; // settle at the base level, untimed
        let before = cx.p.reshape_count;
        let t0 = Instant::now();
        for z in LEVELS {
            cx.view.zoom = z;
            cx.p.set_view(&cx.view);
        }
        cx.frame()?;
        samples.push(ms(t0));
        let delta = cx.p.reshape_count - before;
        ensure!(
            delta == LEVELS.len() as u64,
            "an eager zoom burst must reshape once per level, got {delta}"
        );
        total_reshapes += delta;
    }
    cx.view.zoom = 1.0;
    cx.sync_frame()?;
    Ok(CellOut {
        samples_ms: samples,
        witness: vec![("reshapes", total_reshapes)],
    })
}

/// THEME BURST (the `--bench-theme-burst` workload shape, per tier): switch
/// between two worlds whose display faces AND mono faces both differ
/// (Gumtree: Literata / Monaspace Xenon <-> Tawny: IBM Plex Mono), paying
/// `sync_theme` (the reshape) plus the first frame after each switch.
/// WITNESS: every switch really reshaped (the exact defect the old THEME
/// stage hid) and the first switch visibly re-tinted the frame.
fn theme(cx: &mut Cx) -> Result<CellOut> {
    const WORLDS: [&str; 2] = ["Gumtree", "Tawny"];
    const SWITCHES: usize = 8;
    let start_frame = cx.snapshot()?;
    let mut samples = Vec::with_capacity(SWITCHES);
    let mut total_reshapes = 0u64;
    let mut first_switch_pixels = 0u64;
    for i in 0..SWITCHES {
        let world = WORLDS[i % 2];
        let before = cx.p.reshape_count;
        let t0 = Instant::now();
        crate::theme::set_active_by_name(world)
            .with_context(|| format!("unknown bench world {world}"))?;
        cx.p.sync_theme();
        cx.p.set_view(&cx.view);
        cx.frame()?;
        samples.push(ms(t0));
        let delta = cx.p.reshape_count - before;
        ensure!(
            delta >= 1,
            "a different-face theme switch must reshape (world {world}, got {delta}) — \
             the old theme bench measured 5ms while nothing reshaped; never again"
        );
        total_reshapes += delta;
        if i == 0 {
            first_switch_pixels = differing_pixels(&start_frame, &cx.snapshot()?);
            ensure!(
                first_switch_pixels > 0,
                "a theme switch must visibly change the frame"
            );
        }
    }
    // Restore the suite's pinned world (untimed).
    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    cx.p.sync_theme();
    cx.sync_frame()?;
    Ok(CellOut {
        samples_ms: samples,
        witness: vec![
            ("reshapes", total_reshapes),
            ("pixels_changed", first_switch_pixels),
        ],
    })
}

/// WRAP/RESIZE STEP: alternate the canvas between the full report width and a
/// narrow width that undercuts the page column, paying the re-wrap in the
/// following frame (`sync_wrap_width` — deliberately NOT `reshape_count`-
/// bumped, so the witness is the OUTCOME: the visual row count really moved).
fn resize(cx: &mut Cx) -> Result<CellOut> {
    const NARROW: u32 = 1500;
    const TOGGLES: usize = 6;
    let rows_wide = cx.p.total_visual_rows();
    let mut samples = Vec::with_capacity(TOGGLES);
    let mut rows_narrow = 0usize;
    for i in 0..TOGGLES {
        let w = if i % 2 == 0 { NARROW } else { WIDTH };
        let t0 = Instant::now();
        cx.width = w;
        cx.p.set_size(w as f32, HEIGHT as f32);
        cx.frame()?;
        samples.push(ms(t0));
        if i % 2 == 0 {
            rows_narrow = cx.p.total_visual_rows();
        }
    }
    ensure!(
        rows_narrow != rows_wide && rows_narrow > 0,
        "a narrow re-wrap must change the visual row count ({rows_wide} -> {rows_narrow})"
    );
    // Restore the canvas (untimed).
    cx.width = WIDTH;
    cx.p.set_size(WIDTH as f32, HEIGHT as f32);
    cx.frame()?;
    Ok(CellOut {
        samples_ms: samples,
        witness: vec![
            ("rows_wide", rows_wide as u64),
            ("rows_narrow", rows_narrow as u64),
        ],
    })
}
