//! The SINGLE-FRAME capture entry points and their pollster-blocked driver: the
//! plain `--screenshot` frame, the three mid-glide motion stills, and the shared
//! snapshot helpers ([`base_viewstate`] / [`follow_scroll`]) the animated per-step
//! loops also lean on. (The deterministic `--capture-timeline` / `--capture-held`
//! per-step drivers live in [`super::animated`].) Lifted out of `capture.rs`
//! VERBATIM — same input => byte-stable PNG + sidecar. See [`super`].

use anyhow::{Context, Result};
use glyphon::Cache;
use std::path::Path;

use crate::buffer::Buffer;
use crate::render::{self, TextPipeline, ViewState};

use super::gpu::{headless_device, offscreen_target, read_frame};
use super::opts::{CaptureOpts, ProjectInfo};
use super::sidecar::write_sidecar;
use super::{CANVAS_HEIGHT, CANVAS_WIDTH, FORMAT};

/// Build a capture [`ViewState`] with every search / overlay field at its INERT
/// default and the project-derived fields (`gutter_name`, `gutter_project`,
/// `is_markdown`, `syn_lang`) filled ONCE — so a new ViewState field is added in a
/// single place. The timeline / held paths use this verbatim (overriding only
/// `held`); the single-frame path overrides the search / overlay / selection fields
/// it actually drives.
pub(super) fn base_viewstate(
    buffer: &Buffer,
    project: &Option<ProjectInfo>,
    cursor: (usize, usize),
    zoom: f32,
    misspelled: Vec<crate::spell::Misspelling>,
    held: bool,
) -> ViewState {
    ViewState {
        text: buffer.text(),
        cursor_line: cursor.0,
        cursor_col: cursor.1,
        scroll_lines: 0,
        zoom,
        selection: None,
        preedit: String::new(),
        misspelled,
        is_edit_move: false,
        held,
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
        // The per-kind visible-row cap; the single-frame path overrides it from the
        // still-open overlay's mode (spell = 8 / theme = its larger cap / else 12).
        overlay_window_rows: 12,
        overlay_hint: String::new(),
        overlay_lens: Vec::new(),
        overlay_sections: Vec::new(),
        // CARET-STYLE PICKER preview: set later (from the still-open overlay) by the
        // single-frame path; the inert base leaves it None (no preview / animation).
        caret_preview: None,
        // PAGE-MODE GUTTER: the buffer display name over the project name (empty when
        // there is no project), filled here so the gutter is verifiable from a capture.
        gutter_name: buffer.display_name(),
        gutter_project: project.as_ref().map(|p| p.name.clone()).unwrap_or_default(),
        is_markdown: buffer.is_markdown(),
        syn_lang: buffer.syntax_lang(),
        // SPELL contextual panel: set later (from the still-open overlay) by the
        // single-frame path when it is the spell picker; the inert base leaves it None.
        overlay_spell: None,
        // CALM NOTICE: live-only (the autosave clobber guard) — a capture never
        // has one, so the empty default keeps the frame byte-identical.
        notice: String::new(),
        // i18n: the capture harness has no live Config, so it always uses the
        // built-in default ladder (write-back is structurally live-App-only
        // anyway; this only feeds the per-run render resolution ladder).
        cjk_priority: crate::frontmatter::DEFAULT_CJK_PRIORITY.to_vec(),
        // LINE ENDINGS: the buffer's real on-disk ending — a pure buffer fact, so a
        // CRLF fixture reports "CRLF" and an LF fixture "LF" in the sidecar's hud.eol.
        eol: buffer.eol(),
    }
}

/// Cursor-follow scroll (in VISUAL ROWS) for a settled capture: scroll just enough
/// to bring the `(line, col)` cursor's visual row on screen from the top, clamped
/// to the document's max scroll. Variable-row-height aware via the pixel-accurate
/// pipeline helpers. Shared by the timeline / held paths and the focus-Off branch
/// of the single-frame path, so the three never drift (the focus-on single-frame
/// path CENTERS instead, so it keeps its own branch). `height` is the canvas px.
pub(super) fn follow_scroll(pipeline: &TextPipeline, line: usize, col: usize, height: f32) -> usize {
    let row = pipeline.visual_row_of(line, col);
    pipeline
        .scroll_to_show_row(row, 0, height)
        .min(pipeline.max_scroll_rows(height))
}

/// How the caret is posed for a headless capture. Both modes are fully
/// deterministic (no clock): the same input yields a byte-identical PNG.
#[derive(Clone, Copy, PartialEq)]
enum CaretMode {
    /// Caret settled exactly on target: the resting amber rounded square on the
    /// glyph.
    Rest,
    /// Caret part-way through a synthetic horizontal glide: a trailing amber
    /// underline streak dropped to the baseline.
    Motion,
    /// Caret part-way through a synthetic VERTICAL glide: a thin amber bar slid to
    /// the cell's left edge, trailing up the lines it passed.
    MotionVertical,
    /// Caret part-way through a synthetic DIAGONAL glide (different row AND column):
    /// a true slanted amber tracer from source to target.
    MotionDiagonal,
}

/// Render the loaded `buffer` to an offscreen 1200x800 texture and write
/// `<out>.png` and the sidecar `<out>.json`. Opens NO window. The caret is drawn
/// AT REST (the resting amber rounded square on the glyph) at the buffer's current
/// cursor position, so the capture is byte-deterministic. Deterministic for a
/// fixed set of options.
pub fn capture_with(out_png: &Path, buffer: &Buffer, opts: &CaptureOpts) -> Result<()> {
    pollster::block_on(capture_async(out_png, buffer, CaretMode::Rest, opts))
}

/// Like [`capture`], but renders ONE frame of a caret MID-GLIDE — a synthetic,
/// deterministic still showing the caret dropped to the baseline and stretched
/// into a trailing underline streak partway along its path, so the temporal
/// effect is inspectable from a screenshot. No clock is consulted.
pub fn capture_motion(out_png: &Path, buffer: &Buffer) -> Result<()> {
    pollster::block_on(capture_async(
        out_png,
        buffer,
        CaretMode::Motion,
        &CaptureOpts::default(),
    ))
}

/// Like [`capture_motion`], but a VERTICAL mid-glide: the caret has slid to a thin
/// amber bar on the cell's left edge, trailing up the lines it just travelled.
pub fn capture_motion_vertical(out_png: &Path, buffer: &Buffer) -> Result<()> {
    pollster::block_on(capture_async(
        out_png,
        buffer,
        CaretMode::MotionVertical,
        &CaptureOpts::default(),
    ))
}

/// Like [`capture_motion`], but a DIAGONAL mid-glide: the caret is part-way through
/// a jump between two points on different rows AND columns, so its trail is a true
/// slanted tracer from source to target (not an axis-snapped bar).
pub fn capture_motion_diagonal(out_png: &Path, buffer: &Buffer) -> Result<()> {
    pollster::block_on(capture_async(
        out_png,
        buffer,
        CaretMode::MotionDiagonal,
        &CaptureOpts::default(),
    ))
}

async fn capture_async(
    out_png: &Path,
    buffer: &Buffer,
    caret_mode: CaretMode,
    opts: &CaptureOpts,
) -> Result<()> {
    // --- Device (no surface needed for offscreen) -------------------------
    let (device, queue) = headless_device().await?;

    // PHYSICAL canvas dims for this run: the flagged `--capture-size`, else the
    // byte-stable default. DPI defaults to 1.0 (a `set_dpi` no-op).
    let (width, height) = opts.canvas.unwrap_or((CANVAS_WIDTH, CANVAS_HEIGHT));
    let dpi = opts.dpi.unwrap_or(1.0);

    // --- Offscreen color target ------------------------------------------
    let (texture, view) = offscreen_target(&device, width, height);

    // --- Text pipeline (shared with windowed) ----------------------------
    let (cursor_line, cursor_col) = buffer.cursor_line_col();
    let zoom = render::clamp_zoom(opts.zoom.unwrap_or(1.0));
    // Spell-check the buffer text for the headless capture too, so `--screenshot`
    // renders the squiggles. Deterministic (fixed text -> fixed spans). If the
    // bundled dictionary fails to parse, report it and render without squiggles.
    let misspelled = match crate::spell::SpellChecker::new(crate::spell::active_variant()) {
        Ok(sc) => sc.misspellings_for(&buffer.text(), buffer.syntax_lang()),
        Err(e) => {
            eprintln!("spell-check disabled for capture: {e}");
            Vec::new()
        }
    };

    // --- Search panel (deterministic headless isearch) -------------------
    // Compute matches against the loaded buffer, pick current = first match at
    // or after the cursor (Forward, deterministic) else the first match, and
    // move the resting caret onto the current match. capture takes &Buffer
    // (immutable), so we DO NOT set_cursor; we derive sc_line/sc_col locally and
    // feed them into the ViewState so settle_caret lands the caret on the match.
    let (search_matches, search_current, mut sc_line, mut sc_col) = if let Some(q) = &opts.search {
        let cs = opts.search_case_sensitive;
        let raw = crate::search::find_all(&buffer.text(), q, cs);
        let ranges: Vec<((usize, usize), (usize, usize))> = raw
            .iter()
            .map(|m| {
                (
                    buffer.char_to_line_col(m.start),
                    buffer.char_to_line_col(m.end),
                )
            })
            .collect();
        let cur_char = buffer.cursor_char();
        let cur_idx = if raw.is_empty() {
            None
        } else {
            Some(raw.iter().position(|m| m.start >= cur_char).unwrap_or(0))
        };
        let (cl, cc) = match cur_idx {
            Some(i) => buffer.char_to_line_col(raw[i].start),
            None => (cursor_line, cursor_col),
        };
        (ranges, cur_idx, cl, cc)
    } else {
        (Vec::new(), None, cursor_line, cursor_col)
    };
    let search_active = opts.search.is_some();

    let cache = Cache::new(&device);
    let mut pipeline = TextPipeline::new(&device, &queue, &cache, FORMAT);
    pipeline.set_size(width as f32, height as f32);
    // DPI AFTER set_size: set_dpi re-wraps at column_width(), which reads window_w
    // (set by set_size). No-op at the default 1.0, so the no-flag path is unchanged.
    pipeline.set_dpi(dpi);

    // Shape the document first (at zoom 0/no-scroll) so the pipeline can report
    // wrap-aware row counts. Scroll is counted in VISUAL ROWS, so an explicit
    // `--scroll N` is N visual rows clamped to the document's total visual rows,
    // and the cursor-follow default uses the cursor's VISUAL row. Both need the
    // buffer shaped, which a preliminary `set_view` provides.
    // Start from the shared inert-default base (project status + flags filled once),
    // then drive the search / overlay / selection fields this single-frame path
    // verifies. With an active --search the resting caret lands on the current match.
    let mut vstate = base_viewstate(buffer, &opts.project, (sc_line, sc_col), zoom, misspelled, false);
    vstate.selection = opts.selection;
    vstate.preedit = opts.preedit.clone().unwrap_or_default();
    vstate.search_matches = search_matches;
    vstate.search_current = search_current;
    vstate.search_query = opts.search.clone().unwrap_or_default();
    vstate.search_active = search_active;
    vstate.search_case_sensitive = opts.search_case_sensitive;
    // REPLACE mode: `--search-replace` (or a `--keys` replay of Cmd-R / Cmd-Option-F)
    // reveals the labeled replace row + the key-hint line, surfaced here so both are
    // verifiable. A fresh Cmd-R open keeps focus on the FIND field (the amber caret
    // rides the query), matching the live open; the replacement can't be typed
    // headlessly (the isearch-input gap), so it stays empty.
    vstate.search_replace_active = opts.search_replace_active;
    vstate.search_replacement = opts.search_replacement.clone();
    vstate.search_editing_replacement = false;
    vstate.overlay_active = opts.overlay.as_ref().map(|o| o.active).unwrap_or(false);
    // CRISP-BACKDROP exception: the THEME / CARET / HISTORY pickers keep the doc
    // crisp (no frosted blur) — the theme/caret cards preview live document state,
    // and the history timeline previews the highlighted VERSION in the document
    // itself; every other full overlay gets the blur backdrop.
    vstate.overlay_crisp = opts
        .overlay
        .as_ref()
        .map(|o| o.mode == "theme" || o.mode == "caret" || o.mode == "history")
        .unwrap_or(false);
    vstate.overlay_query = opts.overlay.as_ref().map(|o| o.query.clone()).unwrap_or_default();
    vstate.overlay_items = opts.overlay.as_ref().map(|o| o.items.clone()).unwrap_or_default();
    vstate.overlay_empty = opts.overlay.as_ref().and_then(|o| o.empty.clone());
    vstate.overlay_bindings = opts.overlay.as_ref().map(|o| o.bindings.clone()).unwrap_or_default();
    vstate.overlay_git = opts.overlay.as_ref().map(|o| o.git.clone()).unwrap_or_default();
    vstate.overlay_selected = opts.overlay.as_ref().map(|o| o.selected_index).unwrap_or(0);
    // Scroll window: keep the selection visible with the same min-scroll math
    // `OverlayState::scroll_to_selected` uses (8-row cap for the spell popup, else 12),
    // so a JSON-driven capture windows a long list identically to the live picker. The
    // pipeline re-clamps to the item count, so this needs no `n_items` here.
    let spell_panel = opts.overlay.as_ref().map(|o| o.mode == "spell").unwrap_or(false);
    let theme_panel = opts.overlay.as_ref().map(|o| o.mode == "theme").unwrap_or(false);
    let win = if spell_panel { 8 } else { 12 };
    // The per-kind visible-row cap, mirroring `OverlayState::window_rows` (spell = 8,
    // theme shows every world = 64, else 12) so a JSON-driven capture windows the faceted
    // card exactly as the live picker does. The item-space scroll HINT stays the
    // min-scroll form below; the cap is what bounds the drawn window.
    vstate.overlay_window_rows = if spell_panel {
        crate::overlay::OverlayKind::Spell.window_rows()
    } else if theme_panel {
        crate::overlay::OverlayKind::Theme.window_rows()
    } else {
        12
    };
    // The THEME picker's item-space scroll is pinned at 0 (a valid window HINT — the
    // grouped-path geometry converts it to a display line and then slides the display
    // window to keep the selected row visible, bounding the card to the canvas even when
    // a faceted corpus overflows).
    vstate.overlay_scroll = if theme_panel {
        0
    } else {
        vstate.overlay_selected.saturating_sub(win - 1)
    };
    vstate.overlay_hint = opts.overlay.as_ref().map(|o| o.hint.clone()).unwrap_or_default();
    // THEME PICKER: the lens strip + per-row section labels (drives the faceted render).
    vstate.overlay_lens = opts.overlay.as_ref().map(|o| o.lens_strip.clone()).unwrap_or_default();
    vstate.overlay_sections = opts.overlay.as_ref().map(|o| o.sections.clone()).unwrap_or_default();
    // SPELL contextual panel: the misspelled word's span (from the still-open spell
    // picker) anchors the small floating panel at the word — no blur backdrop.
    vstate.overlay_spell = opts.overlay.as_ref().and_then(|o| o.spell_target);
    // CARET-STYLE PICKER preview: when the still-open overlay is the caret picker,
    // map its highlighted row label back to the look so the headless capture renders
    // that look's SETTLED preview caret (the loop is live-only; see settle_caret_preview).
    vstate.caret_preview = opts.overlay.as_ref().filter(|o| o.mode == "caret").and_then(|o| {
        o.items
            .get(o.selected_index)
            .and_then(|name| crate::caret::CaretMode::from_label(name))
    });
    // HISTORY TIMELINE live preview: the still-open History overlay's highlighted
    // row previews THAT VERSION in the document itself — override the snapshot's
    // text BEFORE the first `set_view`, so the scroll math below shapes the
    // previewed version (exactly like the live `sync_view` fold), and the sidecar
    // `text` reports it. Mirrors the live geometry safety: the cursor clamps into
    // the previewed text (the shared `clamp_line_col`) and the buffer-indexed
    // spans (selection / squiggles / search) are cleared. `None` (default) leaves
    // a plain `--screenshot` byte-identical.
    if let Some(p) = &opts.preview_text {
        vstate.text = p.clone();
        let (pl, pc) = crate::history::clamp_line_col(p, vstate.cursor_line, vstate.cursor_col);
        vstate.cursor_line = pl;
        vstate.cursor_col = pc;
        sc_line = pl;
        sc_col = pc;
        vstate.selection = None;
        vstate.misspelled = Vec::new();
        vstate.search_matches = Vec::new();
        vstate.search_current = None;
        vstate.search_query = String::new();
        vstate.search_active = false;
        vstate.search_case_sensitive = false;
        vstate.search_replace_active = false;
        vstate.search_replacement = String::new();
        vstate.search_editing_replacement = false;
    }
    pipeline.set_view(&vstate);

    // Now compute the VISUAL-ROW scroll from the shaped buffer. Variable-row-height
    // aware (headings): the pixel-accurate pipeline helpers mirror `app.rs`.
    let scroll_lines = match opts.scroll {
        // `--scroll N` is N VISUAL rows; 999 etc. clamps to the last reachable row.
        Some(n) => n.min(pipeline.max_scroll_rows(height as f32)),
        None => {
            // Cursor-follow default: scroll so the cursor's VISUAL row is on screen
            // (from the top, since the headless cursor starts at the buffer start
            // unless a selection moved it). Mirrors the windowed cursor-follow,
            // INCLUDING the focus-mode TYPEWRITER fold: with focus active the row is
            // CENTERED, otherwise it's the minimal-adjust — so a `--focus paragraph`
            // capture verifies the centered scroll deterministically.
            if crate::focus::mode() == crate::focus::FocusMode::Off {
                follow_scroll(&pipeline, sc_line, sc_col, height as f32)
            } else {
                // Focus mode CENTERS the cursor row (the typewriter fold).
                let cursor_row = pipeline.visual_row_of(sc_line, sc_col);
                pipeline
                    .scroll_to_center_row(cursor_row, height as f32)
                    .min(pipeline.max_scroll_rows(height as f32))
            }
        }
    };
    vstate.scroll_lines = scroll_lines;
    pipeline.set_view(&vstate);
    // Pose the caret deterministically for this capture.
    match caret_mode {
        CaretMode::Rest => pipeline.settle_caret(),
        CaretMode::Motion => pipeline.inject_motion_demo(),
        CaretMode::MotionVertical => pipeline.inject_motion_demo_vertical(),
        CaretMode::MotionDiagonal => pipeline.inject_motion_demo_diagonal(),
    }
    // FOCUS MODE: render the SETTLED dim/full state (active unit full, rest dim) with
    // no clock — the crossfade is live-only, so the capture is deterministic.
    pipeline.settle_focus();
    // CARET-STYLE PICKER preview: pin its looping preview caret to its SETTLED look on
    // cell 0 (the loop is live-only, so the capture renders the deterministic resting
    // caret of the highlighted style). No-op when that picker isn't open.
    pipeline.settle_caret_preview();
    // WHICH-KEY panel: summon it with the derived continuation rows when `--whichkey`
    // populated them (`None` otherwise → nothing drawn, byte-identical default).
    pipeline.set_whichkey(opts.whichkey.clone());
    pipeline.prepare(&device, &queue, width, height)?;

    // --- Draw the frame, then read it back via the shared helper ---------
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("awl capture encoder"),
    });
    pipeline.render(&mut encoder, &view)?;
    queue.submit(Some(encoder.finish()));
    let img = read_frame(&device, &queue, &texture, width, height)?;

    // --- Write PNG --------------------------------------------------------
    img.save(out_png)
        .with_context(|| format!("failed to write PNG {}", out_png.display()))?;

    // --- Write JSON sidecar ----------------------------------------------
    write_sidecar(out_png, &vstate, &pipeline, opts, None)?;

    Ok(())
}
