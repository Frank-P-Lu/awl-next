//! CHROME RENDER — the summoned/quiet UI furniture composited OVER the document:
//! the top-right search/replace panel, the centered navigation overlay (go-to /
//! command palette), the bottom-left page-mode orientation GUTTER (filename over
//! project), and the single-line CORNER readouts (the bottom-right markdown
//! word-count and the opt-in top-left DEBUG frame counter).
//!
//! These are all inherent methods on [`super::TextPipeline`]: they shape into its
//! shared panel / gutter / wordcount / fps glyph buffers and `prepare` them through
//! its glyphon renderers, atlas, viewport, font-system and swash-cache — the GPU
//! aggregation that is `TextPipeline`'s whole reason for being — so they CANNOT
//! become `&self`-free free functions the way the span/attrs helpers in `render.rs`
//! could. This module is purely a physical home for that cohesive chrome cluster,
//! carved out of `render.rs` verbatim. Because a child module sees its ancestor's
//! private items, the methods keep their full access to `TextPipeline`'s private
//! fields and helpers with NO behaviour change — the chrome pixels are byte-identical.
//!
//! The corner readouts share ONE body, [`TextPipeline::prepare_corner_label`]:
//! `prepare_wordcount` / `prepare_fps` were ~95%-identical copies differing only by
//! the (renderer, buffer) pair, the text, and the [`CornerAnchor`], so they each
//! reduce to resolving their own text + column geometry and delegating to that shared
//! helper. The readout text-feeders (`word_count`, `readout_report`, `wordcount_text`,
//! `set_fps_frame_ms`, `fps_text`) ride along with their readouts. (The bottom-left
//! project status strip was REMOVED — the gutter now carries the filename/project
//! orientation, so the strip was redundant clutter.)

use super::*;

/// The WHICH-KEY panel's quiet header — the prefix it teaches the continuations of.
/// awl arms the pause timer only for `C-x`, so this is that prefix's label.
const PREFIX_HEADER: &str = "C-x";

/// Upload the three FLOAT-PANEL elevation quads (drop `shadow` -> raised `border` ->
/// opaque `card`) for `rect`, or PARK all three empty when `rect` is `None`. Shared by
/// the reusable [`TextPipeline::prepare_float_panel`] (the caret-preview / spell
/// panels) AND the which-key panel — each passes ITS OWN three pipelines, so the two
/// summoned micro-panels never race the same quads. `card` is drawn last (on top of
/// its shadow + border), matching the painter's-order draw in `render.rs`.
#[allow(clippy::too_many_arguments)]
fn set_float_quads(
    shadow: &mut SelectionPipeline,
    border: &mut SelectionPipeline,
    card: &mut SelectionPipeline,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    rect: Option<[f32; 4]>,
) {
    match rect {
        Some([x, y, w, h]) => {
            // Drop SHADOW: offset DOWN + a touch wider, translucent ink, so the card
            // reads as risen a step above the document (depth by value, DESIGN §8).
            shadow.prepare(device, queue, width, height, &[[x - 2.0, y + 4.0, w + 4.0, h + 6.0]]);
            // Crisp raised BORDER edge: a slightly larger surface-step rect whose 1px
            // rim peeks past the card, giving the box a clean, present edge.
            border.prepare(device, queue, width, height, &[[x - 1.0, y - 1.0, w + 2.0, h + 2.0]]);
            card.prepare(device, queue, width, height, &[[x, y, w, h]]);
        }
        None => {
            shadow.prepare(device, queue, width, height, &[]);
            border.prepare(device, queue, width, height, &[]);
            card.prepare(device, queue, width, height, &[]);
        }
    }
}

/// The search panel's shaped-text outcome carried from `panel_shape_text` to the
/// layout/upload/caret steps: the no-match flag + ink/error colors the card draws
/// with, and the FOCUSED field's reserved-caret-cell offsets (byte + char prefix +
/// row) handed to `panel_layout` so the amber caret tracks the real shaped advance.
struct PanelShape {
    no_match: bool,
    ink: glyphon::Color,
    red: glyphon::Color,
    caret_byte: usize,
    caret_fallback_chars: usize,
    caret_row: f32,
}

/// Resolved geometry for the summoned overlay card: the row WINDOW (`visible` rows
/// from `top_idx`, `n_items` total, plus the foot `hint`/`hint_rows`), the card
/// rectangle (`card_x/y/w/h`), and the inner text origin + width
/// (`text_left/top/w`). Computed BEFORE the rows so the binding column can
/// right-align to the text width.
/// The gap between adjacent lens labels in the theme picker's strip. Kept modest so
/// the whole strip fits one line on a wide mono world face.
const STRIP_GAP: &str = "  ";
/// The wider separator BEFORE the far-right `All` label (a faint `|` parks it at the
/// end). Must stay in sync between the shaper and the lens hit-test (they rebuild the
/// same strip string).
const STRIP_ALL_SEP: &str = "   |   ";

/// One DISPLAY line in the THEME picker's candidate area (below the query + lens
/// strip): either a faint uppercase SECTION header, or a world ROW (carrying its
/// index into `overlay_items`). Built by [`TextPipeline::theme_plan`] from the
/// parallel `overlay_sections`, so the render + hit-test share one line sequence.
#[derive(Clone)]
pub(super) enum ThemeLine {
    /// A faint section header (already uppercased for display).
    Header(String),
    /// A world row; the payload is its index into `overlay_items`.
    Item(usize),
}

pub(super) struct OverlayGeom {
    visible: usize,
    top_idx: usize,
    n_items: usize,
    hint: String,
    hint_rows: usize,
    /// THEME PICKER only: `true` when this card is the faceted theme picker (drives the
    /// strip + section-header layout branch). `false` for every other overlay.
    theme: bool,
    /// THEME PICKER only: the lens strip (label + active flag), drawn on display line 1.
    strip: Vec<(String, bool)>,
    /// THEME PICKER only: the candidate-area display sequence (headers + world rows),
    /// starting at display line 2 (below the query line 0 + strip line 1).
    plan: Vec<ThemeLine>,
    /// Rows occupied ABOVE the candidate list: `1` for the query line the flat/nav
    /// pickers show at the top (`› query`), `0` for the contextual SPELL panel (no
    /// query line — just suggestion rows). Candidate row 0 therefore begins at
    /// `text_top + header_rows * line_height`, which both the selected-row band and
    /// the pointer hit-test read, so they can't drift from the shaped rows.
    header_rows: usize,
    card_x: f32,
    // `pub(super)`: the caret-style preview (in the sibling `caret` module) reads the
    // card rect + text origin to place its preview box just below the card.
    pub(super) card_y: f32,
    card_w: f32,
    pub(super) card_h: f32,
    pub(super) text_left: f32,
    text_top: f32,
    text_w: f32,
}

impl TextPipeline {
    /// Shape + upload the top-right search panel for this frame: the opaque
    /// BASE_300 card, the panel text (calm BASE_CONTENT, or ERROR-red on the
    /// no-match state), and the amber caret block at the query end. Called from
    /// `prepare()` only when `search_active`.
    pub(super) fn prepare_panel(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        self.panel_remetric();
        let shape = self.panel_shape_text(width);
        let (card_rect, text_left, text_top, caret_x) =
            self.panel_layout(width, shape.caret_byte, shape.caret_fallback_chars);
        self.panel_upload_text(device, queue, width, height, &shape, card_rect, text_left, text_top)?;
        self.panel_place_caret(queue, width, height, caret_x, text_top, shape.caret_row);
        Ok(())
    }

    /// Re-metric the shared panel buffer to the current zoom so its glyph
    /// line-height matches the caret/layout rects (which use m.line_height).
    fn panel_remetric(&mut self) {
        let m = self.metrics;
        self.panel_buffer
            .set_metrics(&mut self.font_system, m.glyph_metrics());
    }

    /// Compose + shape the labeled find-and-replace panel text into `panel_buffer`,
    /// returning the colors the card draws with and the FOCUSED field's
    /// reserved-caret-cell offsets. The amber caret rides a RESERVED cell shaped
    /// right after the focused field so its x comes from the SAME layout as the text
    /// (no hardcoded-pitch drift).
    ///
    /// The panel is a clear labeled card, not the old terse `/` pill:
    ///   * a **find** row — the `find` label, the query, the `N/M` match counter, and
    ///     the `Aa` case indicator;
    ///   * a **replace** row (shown whenever replace is active) — the `replace` label
    ///     and the replacement text;
    ///   * a dim **key-hint** line that TEACHES the actions (`↵ replace+next`,
    ///     `⌘↵ all`, `⇥ switch`, `⌥c case`, `Esc done`) — the keycaps ride glyphs
    ///     (↵ Return, ⇥ Tab) to match ⌘/⌥, informational muted ink, NOT clickable
    ///     buttons (the button-free principle; PHILOSOPHY §2).
    /// The labels are padded to one width so the two value columns line up.
    fn panel_shape_text(&mut self, width: u32) -> PanelShape {
        let m = self.metrics;
        // Calm visual hierarchy via per-run color: muted labels + hit counter, full-ink
        // query/replacement, and an "Aa" indicator that brightens from muted to full ink
        // when case-sensitivity is ON (state without amber — the only amber is the caret
        // quad). On the no-match state the query + counter tint ERROR red.
        let no_match = self.search_no_matches();
        let ink = theme::base_content().to_glyphon();
        let muted = theme::muted().to_glyphon();
        let red = theme::error().to_glyphon();
        let total = self.search_matches.len();
        let n = self.search_current.map(|i| i + 1).unwrap_or(0);
        let query = self.search_query.clone();

        // Labels, padded to a shared width so `query` and `replacement` start in the
        // same column (ASCII, so byte len == char count — the caret-offset math below
        // relies on that). "replace " is the widest at 8 cells.
        const FIND_LABEL: &str = "find    ";
        const REPLACE_LABEL: &str = "replace ";
        // The amber caret block rides a RESERVED cell shaped right after the focused
        // field's text; on the find row two clear cells then follow so the block can
        // never collide with the `N/M` digits at any query length. Keeping the reserved
        // cell IN the shaped string means the caret x and the counter x come from the
        // SAME layout — no drift between a hardcoded advance and glyphon's shaped text.
        let gap = "   "; // [caret cell][clear][clear]
        let counter = format!("{n}/{total}   ");
        let (c_query, c_counter, c_toggle) = if no_match {
            (red, red, muted)
        } else if self.search_case_sensitive {
            (ink, muted, ink) // case ON -> "Aa" full ink
        } else {
            (ink, muted, muted) // case OFF -> "Aa" muted
        };
        // Active-world face (mono is the automatic glyph fallback); the search caret
        // reads its x from the SHAPED buffer so it tracks real advances.
        let base = panel_attrs();
        let mk = |c| base.clone().color(c);
        // The macOS modifier glyphs (⌘ ⌥) in the hint line shape from the bundled
        // SYMBOL_FAMILY face (the display/mono faces render them as tofu), the same
        // treatment the overlay chord column gives them.
        let sym = |c| Attrs::new().family(Family::Name(SYMBOL_FAMILY)).color(c);

        let replacement = self.search_replacement.clone();
        let replace_active = self.search_replace_active;
        let editing_replacement = replace_active && self.search_editing_replacement;
        // The dim key-hint line that teaches the replace actions — muted ink, present
        // only once the replace row is up (a plain find keeps the terse counter panel).
        let hint = "\u{21B5} replace+next   \u{2318}\u{21B5} all   \u{21E5} switch   \u{2325}c case   Esc done";

        // Row 0 — the find field.
        let mut spans: Vec<(&str, Attrs)> = vec![
            (FIND_LABEL, mk(muted)),
            (query.as_str(), mk(c_query)),
            (gap, mk(c_counter)),
            (counter.as_str(), mk(c_counter)),
            ("Aa", mk(c_toggle)),
        ];
        if replace_active {
            // Row 1 — the replace field (label + replacement + reserved caret cell).
            spans.push(("\n", mk(muted)));
            spans.push((REPLACE_LABEL, mk(muted)));
            spans.push((replacement.as_str(), mk(ink)));
            spans.push((" ", mk(ink)));
            // Row 2 — the dim key-hint line. Split so ⌘/⌥ ride the symbol face; the
            // rest stays in the world face, all muted.
            spans.push(("\n", mk(muted)));
            let mut last = 0usize;
            for run in symbol_runs(hint) {
                if run.start > last {
                    spans.push((&hint[last..run.start], mk(muted)));
                }
                let end = run.end;
                spans.push((&hint[run], sym(muted)));
                last = end;
            }
            if last < hint.len() {
                spans.push((&hint[last..], mk(muted)));
            }
        }
        let rows = if replace_active { 3.0 } else { 1.0 };
        // Give the buffer generous width + one line height per row so it never wraps.
        self.panel_buffer.set_size(
            &mut self.font_system,
            Some(width as f32 * 2.0),
            Some(m.line_height * rows),
        );
        let default_attrs = base.clone().color(ink);
        self.panel_buffer.set_rich_text(
            &mut self.font_system,
            spans,
            &default_attrs,
            Shaping::Advanced,
            None,
        );
        self.panel_buffer
            .shape_until_scroll(&mut self.font_system, false);

        // Byte offset + char-prefix of the FOCUSED field's reserved caret cell, so
        // the amber caret tracks the real shaped advance on whichever row has focus.
        let (caret_byte, caret_fallback_chars, caret_row) = if editing_replacement {
            let row0_len =
                FIND_LABEL.len() + query.len() + gap.len() + counter.len() + "Aa".len();
            (
                row0_len + "\n".len() + REPLACE_LABEL.len() + replacement.len(),
                REPLACE_LABEL.chars().count() + replacement.chars().count(),
                1.0_f32,
            )
        } else {
            (
                FIND_LABEL.len() + query.len(),
                FIND_LABEL.chars().count() + query.chars().count(),
                0.0_f32,
            )
        };
        PanelShape {
            no_match,
            ink,
            red,
            caret_byte,
            caret_fallback_chars,
            caret_row,
        }
    }

    /// Upload the shaped panel text (red on the no-match state, else calm ink) and
    /// the opaque BASE_300 card behind it through the panel renderer.
    #[allow(clippy::too_many_arguments)]
    fn panel_upload_text(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        shape: &PanelShape,
        card_rect: [f32; 4],
        text_left: f32,
        text_top: f32,
    ) -> anyhow::Result<()> {
        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let panel_area = TextArea {
            buffer: &self.panel_buffer,
            left: text_left,
            top: text_top,
            scale: 1.0,
            bounds,
            default_color: if shape.no_match { shape.red } else { shape.ink },
            custom_glyphs: &[],
        };
        self.panel_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [panel_area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon panel prepare failed: {e:?}"))?;

        // ELEVATE the card on the reusable floating-panel primitive (drop shadow +
        // raised border + base_300 card), so the summoned find/replace panel reads as
        // risen a step above the crisp document (DESIGN §5/§8) — clearer, more present
        // furniture than the old flat pill. The flat `panel_card` is left empty; the
        // search draw branch draws the float quads (parked whenever the panel is down).
        self.prepare_float_panel(device, queue, width, height, Some(card_rect));
        self.panel_card.prepare(device, queue, width, height, &[]);
        Ok(())
    }

    /// Place the amber query caret: a resting block matching the document caret's
    /// height, centered vertically on the FOCUSED field's row (row 0 = search,
    /// row 1 = replace). Panel rows are uniform height (no md scaling), so the row
    /// top is simply `caret_row * line_height`.
    fn panel_place_caret(
        &mut self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        caret_x: f32,
        text_top: f32,
        caret_row: f32,
    ) {
        let m = self.metrics;
        let caret_h = m.caret_h * 0.8;
        let caret_cx = caret_x + m.caret_w * 0.5;
        let caret_cy = text_top + (caret_row + 0.5) * m.line_height;
        self.panel_caret.prepare(
            queue,
            width,
            height,
            caret_cx,
            caret_cy,
            m.caret_w,
            caret_h,
            CORNER_RADIUS,
        );
    }

    /// Shape + upload the SUMMONED navigation overlay for this frame: a tall
    /// BASE_300 card, a query line (with the one amber caret at its end), the
    /// candidate list (selected row highlighted with a surface VALUE band), all
    /// composited OVER the document. Reuses the panel card / caret / text
    /// renderer; the row highlight reuses the selection-quad pipeline. This is the
    /// functional-first card look — the organic visuals come later.
    pub(super) fn prepare_overlay(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        self.overlay_remetric();
        let ink = theme::base_content().to_glyphon();
        let muted = theme::muted().to_glyphon();
        let geom = self.overlay_geometry(width);
        let has_right = self.overlay_shape_text(&geom, ink, muted);
        self.overlay_upload_text(device, queue, width, height, &geom, has_right, ink, muted)?;
        self.overlay_draw_card(device, queue, width, height, &geom);
        self.overlay_place_caret(queue, width, height, &geom);
        Ok(())
    }

    /// Re-metric BOTH shared overlay buffers to the current zoom so their glyph
    /// line-height matches the highlight/caret rects (which use m.line_height).
    /// Without this the buffer keeps its zoom-1.0 metrics and the selection
    /// highlight drifts one row off the text under zoom.
    ///
    /// The NAME buffer rides full BODY metrics (the command/item name is the figure);
    /// the right CHORD/time column rides the same LINE HEIGHT (so each chord stays on
    /// its name's row) but a smaller LABEL FONT SIZE — the type system's recessive
    /// rung (DESIGN §4: ink × size), so the secondary key-chord reads quieter than the
    /// name it annotates, not the same grey/size.
    fn overlay_remetric(&mut self) {
        let m = self.metrics;
        self.panel_buffer
            .set_metrics(&mut self.font_system, m.glyph_metrics());
        let label = crate::markdown::type_scale::LABEL;
        self.panel_bind_buffer.set_metrics(
            &mut self.font_system,
            GlyphMetrics::new(m.font_size * label, m.line_height),
        );
    }

    /// Resolve the overlay card's row WINDOW + rectangle + inner text origin. The
    /// list is capped at `MAX_ROWS` and scrolled so the selected row stays visible;
    /// the geometry is computed BEFORE the rows so the binding column can
    /// right-align to the text width.
    pub(super) fn overlay_geometry(&self, width: u32) -> OverlayGeom {
        // SPELL contextual panel: a small floating popup anchored at the misspelled
        // word (no query line, no foot hint), NOT the centered takeover card.
        if let Some((line, start_col, end_col)) = self.overlay_spell {
            return self.spell_overlay_geometry(width, line, start_col, end_col);
        }
        // THEME picker: the faceted lens-switcher (strip + section-grouped worlds),
        // which lays out differently from the flat pickers (see below).
        if !self.overlay_lens.is_empty() {
            return self.theme_overlay_geometry(width);
        }
        let m = self.metrics;
        let pad = 12.0;
        let margin = 12.0;
        // Cap how many rows we show so the card stays bounded; the selected row is
        // kept in view by a simple window starting at a scroll offset.
        const MAX_ROWS: usize = 12;
        let n_items = self.overlay_items.len();
        let visible = n_items.min(MAX_ROWS);
        // The scroll window is owned by `OverlayState::scroll` (which keeps the selection
        // visible on keyboard nav, holds still on hover, and advances on the wheel); the
        // pipeline just reads it, clamped so `[top_idx, top_idx+visible)` stays in range.
        let top_idx = self.overlay_scroll.min(n_items.saturating_sub(visible));

        // A faint, per-kind control-hint line drawn at the FOOT of the card so the
        // select-vs-descend model is discoverable (see `OverlayKind::hint`). Drawn
        // in the dim token; its own row, kept off the candidate list. Empty = none.
        let hint = self.overlay_hint.clone();
        let hint_rows = if hint.is_empty() { 0 } else { 1 };

        // Card / text-column geometry. Computed here (before the rows) so the
        // command-palette binding column can right-align to the text width. The
        // CARET-STYLE PICKER's live preview now rides its OWN floating panel BELOW this
        // card (see `prepare_caret_preview_panel`), so the list itself stays exactly as
        // familiar — no reserved preview strip carved out of the card.
        let header_rows = 1; // the `› query` line every flat/nav picker shows on top
        let total_rows = header_rows + visible + hint_rows; // query + candidates + hint
        // RESPONSIVE CARD: prefer half the window, floored at a readable width, and
        // never wider than the window minus a calm margin — so a NARROW window gets
        // a card spanning nearly its full width (mirroring the responsive page
        // column) instead of a fixed-width card whose text column starves. At the
        // default 1200 canvas this is the same 600 as ever (wide captures are
        // byte-identical); the floor only lifts sub-1120 windows.
        let card_w = (width as f32 * 0.5).max(560.0).min(width as f32 - 2.0 * margin);
        let text_w = card_w - 2.0 * pad;
        let card_h = total_rows as f32 * m.line_height + 2.0 * pad;
        // Center horizontally, anchor near the top third (summoned, transient).
        let card_x = (width as f32 - card_w) * 0.5;
        let card_y = margin + 40.0;
        let text_left = card_x + pad;
        let text_top = card_y + pad;
        OverlayGeom {
            visible,
            top_idx,
            n_items,
            hint,
            hint_rows,
            theme: false,
            strip: Vec::new(),
            plan: Vec::new(),
            header_rows,
            card_x,
            card_y,
            card_w,
            card_h,
            text_left,
            text_top,
            text_w,
        }
    }

    /// THEME PICKER display plan: the candidate-area sequence of section HEADERS +
    /// world ROWS, from the parallel `overlay_sections`. A header is emitted before a
    /// row whenever its section differs from the previous row's (so contiguous groups
    /// get one header each); the All lens / non-grouped rows emit no headers. Section
    /// labels are uppercased for the faint header display. Shared by the geometry,
    /// shaping, selected-band, and hit-test so they can never disagree.
    pub(super) fn theme_plan(&self) -> Vec<ThemeLine> {
        let mut out = Vec::with_capacity(self.overlay_items.len());
        let mut prev: Option<String> = None;
        for i in 0..self.overlay_items.len() {
            let sect = self
                .overlay_sections
                .get(i)
                .map(|s| s.as_str())
                .unwrap_or("");
            if !sect.is_empty() && prev.as_deref() != Some(sect) {
                out.push(ThemeLine::Header(sect.to_uppercase()));
            }
            out.push(ThemeLine::Item(i));
            prev = if sect.is_empty() { None } else { Some(sect.to_string()) };
        }
        out
    }

    /// Resolve the FACETED THEME picker's geometry: a centered card carrying (line 0)
    /// the `› query` line, (line 1) the lens STRIP, then the section-grouped world rows
    /// (headers + rows from [`Self::theme_plan`]), then the foot hint. The theme picker
    /// shows EVERY world with NO scroll, so the card grows to the plan; `header_rows`
    /// is 2 (query + strip), and the plan's own line offsets place the rows + band.
    fn theme_overlay_geometry(&self, width: u32) -> OverlayGeom {
        let m = self.metrics;
        let pad = 12.0;
        let margin = 12.0;
        let n_items = self.overlay_items.len();
        let plan = self.theme_plan();
        let hint = self.overlay_hint.clone();
        let hint_rows = if hint.is_empty() { 0 } else { 1 };
        // Line 0 = query, line 1 = lens strip, then the plan lines, then the hint.
        let header_rows = 2;
        let total_rows = header_rows + plan.len() + hint_rows;
        // Wider than the flat pickers so the whole lens strip (Time … All) fits on one
        // line even on a WIDE mono world face without the far-right All clipping.
        let card_w = (width as f32 * 0.58).max(560.0).min(width as f32 - 2.0 * margin);
        let text_w = card_w - 2.0 * pad;
        let card_h = total_rows as f32 * m.line_height + 2.0 * pad;
        let card_x = (width as f32 - card_w) * 0.5;
        let card_y = margin + 40.0;
        let text_left = card_x + pad;
        let text_top = card_y + pad;
        OverlayGeom {
            visible: n_items,
            top_idx: 0,
            n_items,
            hint,
            hint_rows,
            theme: true,
            strip: self.overlay_lens.clone(),
            plan,
            header_rows,
            card_x,
            card_y,
            card_w,
            card_h,
            text_left,
            text_top,
            text_w,
        }
    }

    /// Shape the SPELL panel's suggestion rows into the shared `panel_buffer` and
    /// return the WIDEST row's shaped width (logical px), or `0.0` when there are no
    /// suggestions. This is the content the card must fit — measured with the SAME
    /// [`panel_attrs`] face + BODY metrics the rows render in, so a proportional
    /// world's real advances (not the mean `char_width` estimate) drive the width and
    /// nothing overflows. Called from `set_view` (which holds `&mut font_system`) and
    /// cached in `overlay_spell_w`; the buffer is re-shaped by `overlay_shape_text`
    /// before it draws, so borrowing it here for a measurement is harmless.
    pub(super) fn measure_spell_content_w(&mut self) -> f32 {
        if self.overlay_items.is_empty() {
            return 0.0;
        }
        let m = self.metrics;
        self.panel_buffer
            .set_metrics(&mut self.font_system, m.glyph_metrics());
        // Unconstrained width (each suggestion on its own line) so shaping reports each
        // row's NATURAL width with no wrapping.
        self.panel_buffer
            .set_size(&mut self.font_system, None, None);
        let text = self.overlay_items.join("\n");
        let ink = theme::base_content().to_glyphon();
        self.panel_buffer.set_text(
            &mut self.font_system,
            &text,
            &panel_attrs().color(ink),
            Shaping::Advanced,
            None,
        );
        self.panel_buffer
            .shape_until_scroll(&mut self.font_system, false);
        let mut max_w = 0.0_f32;
        for run in self.panel_buffer.layout_runs() {
            max_w = max_w.max(run.line_w);
        }
        max_w
    }

    /// Geometry for the contextual SPELL panel: a small floating popup anchored just
    /// below the misspelled `(line, start_col, end_col)` word — no query line, no foot
    /// hint, just the suggestion rows. The card's LEFT edge aligns to the word start
    /// and its TOP hangs a hair below the word's screen rect (computed from the SAME
    /// advance-aware visual-row layout the squiggle under the word uses, so the panel
    /// tracks the word at any wrap / scroll / zoom). Clamped to stay on-canvas — it
    /// flips ABOVE the word when there is no room below.
    fn spell_overlay_geometry(
        &self,
        width: u32,
        line: usize,
        start_col: usize,
        end_col: usize,
    ) -> OverlayGeom {
        let m = self.metrics;
        let pad = 10.0;
        let margin = 8.0;
        let gap = 6.0; // the breath between the word and the panel
        const MAX_ROWS: usize = 8;
        let n_items = self.overlay_items.len();
        let visible = n_items.min(MAX_ROWS);
        // Same window model as the centered card: read the overlay-owned scroll offset,
        // clamped to the spell popup's tighter 8-row cap.
        let top_idx = self.overlay_scroll.min(n_items.saturating_sub(visible));
        // A contextual popup: no query row, no foot hint — just the corrections.
        let header_rows = 0;
        let hint = String::new();
        let hint_rows = 0;

        // The word's on-screen rect, from the same layout the squiggle rides. Only the
        // word's POSITION anchors the panel; its WIDTH does not size the card (below).
        let (word_x, word_top, _word_w, word_h) =
            self.spell_word_rect(line, start_col, end_col);

        // Width: fit the WIDEST suggestion ROW — its real SHAPED width, measured into
        // `overlay_spell_w` at sync — plus padding, NOT the anchor word. So a short
        // misspelled word ("teh") can no longer make a narrow card the longer
        // corrections overflow. A calm MIN keeps a lone short suggestion from looking
        // pinched; the card stays capped small and clamped on-canvas. (Falls back to
        // the char-count estimate only if a measurement has not run yet.)
        let content_w = if self.overlay_spell_w > 0.0 {
            self.overlay_spell_w
        } else {
            self.overlay_items
                .iter()
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0) as f32
                * m.char_width
        };
        let card_w = (content_w + 2.0 * pad)
            .clamp(140.0, 360.0)
            .min(width as f32 - 2.0 * margin);
        let text_w = card_w - 2.0 * pad;
        // At least one row tall so a (rare) flagged word with no suggestions still
        // reads as a small present card rather than a zero-height sliver.
        let rows = header_rows + visible.max(1) + hint_rows;
        let card_h = rows as f32 * m.line_height + 2.0 * pad;

        // Anchor the LEFT edge to the word start, clamped so the card stays on-canvas.
        let mut card_x = word_x;
        if card_x + card_w > width as f32 - margin {
            card_x = (width as f32 - margin - card_w).max(margin);
        }
        card_x = card_x.max(margin);
        // Hang below the word; if there is no room, flip above it.
        let below_y = word_top + word_h + gap;
        let card_y = if below_y + card_h <= self.window_h - margin {
            below_y
        } else {
            (word_top - gap - card_h).max(margin)
        };
        let text_left = card_x + pad;
        let text_top = card_y + pad;
        OverlayGeom {
            visible,
            top_idx,
            n_items,
            hint,
            hint_rows,
            theme: false,
            strip: Vec::new(),
            plan: Vec::new(),
            header_rows,
            card_x,
            card_y,
            card_w,
            card_h,
            text_left,
            text_top,
            text_w,
        }
    }

    /// The misspelled word's on-screen rect `(x, top, w, height)` for anchoring the
    /// contextual spell panel — the SAME advance-aware visual-row layout the wavy
    /// squiggle under the word uses ([`Self::spell_squiggles`]), so the panel lands
    /// directly beneath the word's glyphs. Columns are clamped to the word's visual
    /// row; `x` is relative to the canvas (text-left offset folded in).
    fn spell_word_rect(&self, line: usize, start_col: usize, end_col: usize) -> (f32, f32, f32, f32) {
        let m = self.metrics;
        let doc_top = self.doc_top();
        let rows = self.visual_rows(line);
        let row = pick_row(&rows, start_col);
        let char_count = row.xs.len().saturating_sub(1);
        let s = start_col.min(char_count);
        let e = end_col.min(char_count).max(s);
        let (x, w) = row_x_span(row, self.text_left(), s, e, m.char_width);
        let top = doc_top + row.line_top;
        (x, top, w, row.line_height)
    }

    /// Hit-test a pointer at PHYSICAL `(px, py)` against the SUMMONED overlay's
    /// candidate ROWS, returning the `items` index of the row it lands on — the value
    /// to assign to `overlay_selected` / [`crate::overlay::OverlayState::selected`] — or
    /// `None` when the pointer is off the card, on the query line, on the foot hint, or
    /// below the last visible row. It reads the SAME [`Self::overlay_geometry`] the rows
    /// are rendered from, so a hovered/clicked row can NEVER disagree with the
    /// highlighted one. This is the ONE reusable mechanic behind mouse-selecting EVERY
    /// picker kind (go-to / command / browse / theme / keybindings / spell / caret /
    /// outline / project / move-dest) — the overlay intercept is kind-agnostic, so
    /// `input.rs` maps a pointer to a row here and then drives the same selection-move +
    /// accept the keyboard does.
    /// The summoned overlay card's rectangle `[x, y, w, h]` for this frame, or `None`
    /// when no overlay is open — the centered takeover card vs. the contextual SPELL
    /// panel anchored at the misspelled word — from the SAME [`Self::overlay_geometry`]
    /// the card renders from. Used by `input.rs` for the CLICK-AWAY hit-test (a left
    /// click OUTSIDE this rect dismisses the overlay) and by headless tests to assert
    /// WHERE the card sits.
    pub fn overlay_card_rect(&self) -> Option<[f32; 4]> {
        if !self.overlay_active {
            return None;
        }
        let geom = self.overlay_geometry(self.window_w as u32);
        Some([geom.card_x, geom.card_y, geom.card_w, geom.card_h])
    }

    pub fn overlay_row_at(&self, px: f32, py: f32) -> Option<usize> {
        if !self.overlay_active {
            return None;
        }
        let geom = self.overlay_geometry(self.window_w as u32);
        // THEME PICKER: the candidate area interleaves section HEADERS with world rows
        // (below the query + strip lines), so map the pointer to a DISPLAY line, and
        // return the world index ONLY when that line is a row (a header row → None).
        if geom.theme {
            if px < geom.card_x || px > geom.card_x + geom.card_w {
                return None;
            }
            let lh = self.metrics.line_height;
            let rel = py - geom.text_top;
            if rel < 0.0 {
                return None;
            }
            let disp = (rel / lh) as usize;
            if disp < geom.header_rows {
                return None; // the query line or the lens strip
            }
            let k = disp - geom.header_rows;
            return match geom.plan.get(k) {
                Some(ThemeLine::Item(i)) => Some(*i),
                _ => None,
            };
        }
        overlay_row_index(
            geom.card_x,
            geom.card_w,
            geom.text_top,
            self.metrics.line_height,
            geom.header_rows,
            geom.visible,
            geom.top_idx,
            geom.n_items,
            px,
            py,
        )
    }

    /// THEME PICKER: hit-test a pointer against the lens STRIP (display line 1), returning
    /// the [`crate::theme::Lens`] the label under `(px, py)` selects — so a CLICK on a lens
    /// switches the facet (the pointing counterpart to LEFT/RIGHT). `None` off the strip
    /// row, off the card, or for a non-theme overlay. Uses the same per-lens byte ranges
    /// the shaper laid out, read back from the shaped strip glyphs so the hit lands on the
    /// same label the eye sees.
    pub fn overlay_lens_at(&self, px: f32, py: f32) -> Option<crate::theme::Lens> {
        if !self.overlay_active || self.overlay_lens.is_empty() {
            return None;
        }
        let geom = self.overlay_geometry(self.window_w as u32);
        if !geom.theme || px < geom.card_x || px > geom.card_x + geom.card_w {
            return None;
        }
        let lh = self.metrics.line_height;
        // Strip is display line 1 (row band [text_top + lh, text_top + 2*lh)).
        let strip_top = geom.text_top + lh;
        if py < strip_top || py >= strip_top + lh {
            return None;
        }
        // Which label's shaped glyph span contains px? Scan the shaped strip line.
        let want = px - geom.text_left;
        let mut hit: Option<usize> = None;
        for run in self.panel_buffer.layout_runs() {
            if run.line_i != 1 {
                continue;
            }
            // Labels appear in strip order; find the label index whose glyph x-span
            // covers `want`. The lens labels tile the STRIP order 1:1 with `overlay_lens`.
            // Reconstruct label boundaries from glyph byte offsets against the strip text.
            let labels: Vec<&str> = self.overlay_lens.iter().map(|(l, _)| l.as_str()).collect();
            // Build the same "\n"+labels+separators string to map bytes → label index.
            let last = labels.len().saturating_sub(1);
            let mut s = String::from("\n");
            let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
            for (i, lbl) in labels.iter().enumerate() {
                if i > 0 {
                    s.push_str(if i == last { STRIP_ALL_SEP } else { STRIP_GAP });
                }
                let a = s.len();
                s.push_str(lbl);
                ranges.push(a..s.len());
            }
            for g in run.glyphs.iter() {
                if want >= g.x && want < g.x + g.w {
                    // Line-1 glyphs are byte-indexed within the strip line text (the
                    // leading "\n" split the lines); `ranges` are `strip_s`-relative, so
                    // shift the glyph byte forward past that one "\n" to compare.
                    let b = g.start + 1;
                    for (i, r) in ranges.iter().enumerate() {
                        if b >= r.start && b < r.end {
                            hit = Some(i);
                        }
                    }
                }
            }
        }
        hit.and_then(|i| crate::theme::Lens::STRIP.get(i).copied())
    }

    /// Compose + shape the overlay text into the shared buffers: the query line +
    /// candidate rows (selected ink / rest muted) in `panel_buffer`, and the dim
    /// `Align::Right` chord/time column in `panel_bind_buffer`. Returns whether a
    /// right column was built (so the caller uploads its text area).
    ///
    /// The NAME and the RIGHT column share ONE row budget, split by the
    /// [`rowlayout`] primitive (the single owner of the rules): the comfortable
    /// regime reproduces the historical char budget byte-for-byte; when the
    /// estimate goes tight the shaped PIXELS arbitrate ([`rowlayout::fits`]) and
    /// the right column YIELDS whole rather than ever painting over a name.
    fn overlay_shape_text(
        &mut self,
        geom: &OverlayGeom,
        ink: glyphon::Color,
        muted: glyphon::Color,
    ) -> bool {
        // THEME PICKER: the faceted lens strip + section-grouped world rows lay out
        // differently from the flat pickers — its own shaper (which also records the
        // active-lens underline rect). No right column (returns false).
        self.overlay_right_shown = false;
        if geom.theme {
            return self.overlay_shape_theme(geom, ink, muted);
        }
        let visible = geom.visible;
        let top_idx = geom.top_idx;

        // The dim RIGHT-aligned column: command-palette key chords (`bindings`) OR
        // the go-to picker's relative "last edited" labels (`times`). Only one is
        // ever populated, so prefer bindings when present, else fall back to times.
        // It is drawn FLUSH at the card's right text edge by a SEPARATE buffer laid
        // out with cosmic-text `Align::Right`, so the chord column is a clean right
        // edge regardless of the proportional name width.
        let right_labels: &[String] = if !self.overlay_bindings.is_empty() {
            &self.overlay_bindings
        } else {
            &self.overlay_times
        };
        let has_right = !right_labels.is_empty();
        // One line per name row: a `\n`-prefixed label leaves line 0 (the query row)
        // empty and puts label N on candidate row N; the hint row (if any) stays empty.
        let bind_strs: Vec<String> = (0..visible)
            .map(|row| {
                let label = right_labels.get(top_idx + row).map(|s| s.as_str()).unwrap_or("");
                format!("\n{label}")
            })
            .collect();

        // ONE shared row budget, split by the rowlayout primitive: the card's text
        // width in mean glyph widths against the widest right-column label. `Split`/
        // `Full` elide the names to their granted budget (the historical math);
        // `Measure` shapes them UNELIDED and lets the shaped pixels decide below.
        let m = self.metrics;
        let total_chars = if m.char_width > 0.0 {
            (geom.text_w / m.char_width).floor() as usize
        } else {
            usize::MAX
        };
        let widest_right = if has_right {
            Some(right_labels.iter().map(|s| s.chars().count()).max().unwrap_or(0))
        } else {
            None
        };
        let budget = match rowlayout::plan(total_chars, widest_right) {
            rowlayout::Plan::Full { primary } | rowlayout::Plan::Split { primary } => Some(primary),
            rowlayout::Plan::Measure => None,
        };
        let rows: Vec<String> = (0..visible)
            .map(|row| {
                let item = &self.overlay_items[top_idx + row];
                match budget {
                    Some(b) => rowlayout::fit_primary(item, b),
                    None => item.clone(),
                }
            })
            .collect();
        self.shape_overlay_names(geom, ink, muted, &rows);
        if !has_right {
            return false;
        }
        self.shape_overlay_right(geom, ink, muted, &bind_strs);

        // THE NO-OVERLAP LAW, in shaped pixels: the widest candidate name + the gap
        // + the widest right label must tile inside the text column. When they do
        // (every comfortable window, plus tight-but-genuinely-fitting cards like the
        // caret picker's short names beside its label-size descriptions), the right
        // column shows. When they do NOT, it YIELDS — dropped whole — and the names
        // re-shape owning the full row (elided only if a name alone overflows).
        let name_px = self.widest_candidate_px(geom);
        let right_px = self.widest_right_px();
        let gap_px = rowlayout::GAP_CHARS as f32 * m.char_width;
        if rowlayout::fits(geom.text_w, gap_px, name_px, right_px) {
            self.overlay_right_shown = true;
            return true;
        }
        let full = rowlayout::full_budget(total_chars);
        let rows: Vec<String> = (0..visible)
            .map(|row| rowlayout::fit_primary(&self.overlay_items[top_idx + row], full))
            .collect();
        self.shape_overlay_names(geom, ink, muted, &rows);
        false
    }

    /// Shape the overlay's LEFT column into `panel_buffer`: the `› query` line (when
    /// the picker has one), the candidate `rows` (pre-budgeted by the caller through
    /// [`rowlayout`]), and the dim foot hint. Carved verbatim out of the old inline
    /// shaper so the no-overlap arbiter can re-shape the names after a yield.
    fn shape_overlay_names(
        &mut self,
        geom: &OverlayGeom,
        ink: glyphon::Color,
        muted: glyphon::Color,
        rows: &[String],
    ) {
        // The flat/nav pickers show a `› query` line on top (`header_rows == 1`); the
        // contextual SPELL panel shows none (`0`) — just the suggestion rows.
        let has_query = geom.header_rows > 0;
        // Per-row colors: query full ink; candidate rows ink (selected) / muted.
        // Names/query/sigil render in the ACTIVE-WORLD face (`mk`).
        let base = panel_attrs();
        let mk = |c| base.clone().color(c);
        let mut spans: Vec<(&str, glyphon::Attrs)> = Vec::new();
        // The query line (with its `› ` sigil) occupies text line 0 when present; the
        // spell panel skips it so its first suggestion IS line 0.
        let sigil = "› ";
        if has_query {
            spans.push((sigil, mk(muted)));
            spans.push((self.overlay_query.as_str(), mk(ink)));
        }
        // Every row's FILENAME is the FIGURE: content ink at BODY size. Its leading
        // DIRECTORY (through the last `/`) recedes to MUTED ink (figure/ground by value)
        // so the eye lands on the file; a folder row (trailing `/`, no filename after it)
        // stays whole in content ink. The SELECTED row is marked by a surface VALUE BAND
        // (DESIGN §5), not a brighter name. A leading `\n` puts each name on its own row
        // BELOW the query line; without a query line (spell panel) row 0 sits on line 0.
        for (row, content) in rows.iter().enumerate() {
            if !(!has_query && row == 0) {
                spans.push(("\n", mk(ink)));
            }
            let split = if content.ends_with('/') {
                0
            } else {
                crate::overlay::row_split(content)
            };
            if split > 0 {
                spans.push((&content[..split], mk(muted)));
            }
            spans.push((&content[split..], mk(ink)));
        }
        // The quiet control-hint row, last, always in the DIM token. Carries its own
        // leading newline so it sits one line below the final candidate. Its keycap
        // glyphs (↵ ⇥ ⌘ … ) ride the SYMBOL_FAMILY face — split into symbol / non-
        // symbol runs exactly like the chord column — so a hint that teaches a
        // key with a glyph (`↵ restore`) renders it instead of tofu.
        let sym = |c| Attrs::new().family(Family::Name(SYMBOL_FAMILY)).color(c);
        let hint_line = if geom.hint.is_empty() {
            String::new()
        } else {
            format!("\n{}", geom.hint)
        };
        if geom.hint_rows > 0 {
            let mut last = 0usize;
            for run in symbol_runs(&hint_line) {
                if run.start > last {
                    spans.push((&hint_line[last..run.start], mk(muted)));
                }
                let end = run.end;
                spans.push((&hint_line[run], sym(muted)));
                last = end;
            }
            if last < hint_line.len() {
                spans.push((&hint_line[last..], mk(muted)));
            }
        }

        self.panel_buffer
            .set_size(&mut self.font_system, Some(geom.text_w), Some(geom.card_h));
        // Single-line rows: NEVER wrap. A row elided a hair long clips at the card edge
        // instead of spilling onto a second visual row (which overflowed the card).
        self.panel_buffer
            .set_wrap(&mut self.font_system, Wrap::None);
        let default_attrs = base.clone().color(ink);
        self.panel_buffer.set_rich_text(
            &mut self.font_system,
            spans,
            &default_attrs,
            Shaping::Advanced,
            None,
        );
        self.panel_buffer
            .shape_until_scroll(&mut self.font_system, false);
    }

    /// Shape the RIGHT column into the `Align::Right` `panel_bind_buffer`, one
    /// (`\n`-prefixed) label line per candidate row, flush at the card's right text
    /// edge (width == `text_w`). The dim labels stay MONOSPACE; carved verbatim out
    /// of the old inline shaper.
    fn shape_overlay_right(
        &mut self,
        geom: &OverlayGeom,
        ink: glyphon::Color,
        muted: glyphon::Color,
        bind_strs: &[String],
    ) {
        let base = panel_attrs();
        let mono = |c| Attrs::new().family(Family::Monospace).color(c);
        // Split each chord label into SYMBOL / non-symbol runs so the macOS
        // modifier glyphs (⌘ ⇧ ⌥ ⌃) shape from the bundled `SYMBOL_FAMILY` face
        // — which has real, finite advances — instead of the monospace face's
        // tofu. Those flaky-fallback glyphs are what let the glyph chords
        // overshoot the right margin: cosmic-text's `Align::Right` measures the
        // shaped run width, so once the modifier glyphs carry their REAL width the
        // chord column lands flush and `⌘⇧O` lines up with the `C-x` text chords.
        let sym = |c| Attrs::new().family(Family::Name(SYMBOL_FAMILY)).color(c);
        let mut bind_spans: Vec<(&str, glyphon::Attrs)> = Vec::new();
        for s in bind_strs {
            let mut last = 0usize;
            for run in symbol_runs(s) {
                if run.start > last {
                    bind_spans.push((&s[last..run.start], mono(muted)));
                }
                let end = run.end;
                bind_spans.push((&s[run], sym(muted)));
                last = end;
            }
            if last < s.len() {
                bind_spans.push((&s[last..], mono(muted)));
            }
        }
        let default_attrs = base.clone().color(ink);
        self.panel_bind_buffer
            .set_size(&mut self.font_system, Some(geom.text_w), Some(geom.card_h));
        self.panel_bind_buffer
            .set_wrap(&mut self.font_system, Wrap::None);
        self.panel_bind_buffer.set_rich_text(
            &mut self.font_system,
            bind_spans,
            &default_attrs,
            Shaping::Advanced,
            Some(glyphon::cosmic_text::Align::Right),
        );
        self.panel_bind_buffer
            .shape_until_scroll(&mut self.font_system, false);
    }

    /// The widest shaped CANDIDATE row (px) in the just-shaped `panel_buffer` — the
    /// query line above and the hint line below are excluded (only the rows the
    /// right column could collide with count). Feeds [`rowlayout::fits`].
    fn widest_candidate_px(&self, geom: &OverlayGeom) -> f32 {
        let first = geom.header_rows;
        let last = first + geom.visible;
        let mut w = 0.0f32;
        for run in self.panel_buffer.layout_runs() {
            if run.line_i >= first && run.line_i < last {
                w = w.max(run.line_w);
            }
        }
        w
    }

    /// The widest shaped RIGHT-column label (px) in the just-shaped
    /// `panel_bind_buffer` (its line 0 — the query row — is empty, so a plain max
    /// over every run is the label column's width). Feeds [`rowlayout::fits`].
    fn widest_right_px(&self) -> f32 {
        let mut w = 0.0f32;
        for run in self.panel_bind_buffer.layout_runs() {
            w = w.max(run.line_w);
        }
        w
    }

    /// Shape the FACETED THEME picker into `panel_buffer`: the `› query` line (0), the
    /// lens STRIP (1, active lens in full ink + a recorded underline, others muted, the
    /// `All` label pushed right past a faint separator), then the section-grouped world
    /// rows (faint uppercase headers at LABEL size + rows in content ink), then the foot
    /// hint. Records the active-lens underline rect (scanned from the shaped strip
    /// glyphs, so it lands exactly under the label at any world face) into
    /// `overlay_theme_underline`. No right column (returns `false`).
    fn overlay_shape_theme(
        &mut self,
        geom: &OverlayGeom,
        ink: glyphon::Color,
        muted: glyphon::Color,
    ) -> bool {
        let m = self.metrics;

        // Build the strip LINE ("\n" then the lens labels) as one owned string, tracking
        // each label's byte range so the ACTIVE label's glyphs can be underlined. The
        // `All` label (last) is pushed right past a wider faint separator.
        let mut strip_s = String::from("\n");
        let mut label_ranges: Vec<(std::ops::Range<usize>, bool)> = Vec::new();
        let mut sep_ranges: Vec<std::ops::Range<usize>> = Vec::new();
        let mut active_range: Option<std::ops::Range<usize>> = None;
        let last = geom.strip.len().saturating_sub(1);
        for (idx, (lbl, active)) in geom.strip.iter().enumerate() {
            if idx > 0 {
                let s = strip_s.len();
                strip_s.push_str(if idx == last { STRIP_ALL_SEP } else { STRIP_GAP });
                sep_ranges.push(s..strip_s.len());
            }
            let s = strip_s.len();
            strip_s.push_str(lbl);
            let r = s..strip_s.len();
            if *active {
                active_range = Some(r.clone());
            }
            label_ranges.push((r, *active));
        }

        // Foot hint (dim), symbol glyphs from the bundled face.
        let hint_line = if geom.hint.is_empty() {
            String::new()
        } else {
            format!("\n{}", geom.hint)
        };

        // FIRST PASS at full BODY size. Then the strip's RESPONSIVE FOLD: at a
        // narrow window the full-size lens strip (Time … | All) can overflow the
        // card's text column — measured from the SHAPED line (real advances, not
        // the mean estimate), the whole strip steps down in size just enough to
        // fit, so every lens stays present + hit-testable instead of the far
        // right clipping away. At any comfortable width the measured strip fits
        // and the single full-size pass stands (byte-identical wide captures).
        self.shape_theme_spans(geom, ink, muted, &strip_s, &label_ranges, &sep_ranges, &hint_line, 1.0);
        let strip_w = self.theme_strip_px();
        if strip_w > geom.text_w {
            let scale = (geom.text_w / strip_w).max(0.5);
            self.shape_theme_spans(geom, ink, muted, &strip_s, &label_ranges, &sep_ranges, &hint_line, scale);
        }

        // Record the active-lens UNDERLINE from the shaped strip glyphs (line 1). Line-1
        // glyphs are byte-indexed WITHIN the strip line's own text — the leading "\n" in
        // `strip_s` split the lines — so the label's line-relative range is `active_range`
        // shifted back by that one "\n" byte.
        self.overlay_theme_underline = active_range.and_then(|ar| {
            let (a, b) = (ar.start.saturating_sub(1), ar.end.saturating_sub(1));
            let mut min_x = f32::MAX;
            let mut max_x = f32::MIN;
            for run in self.panel_buffer.layout_runs() {
                if run.line_i != 1 {
                    continue;
                }
                for g in run.glyphs.iter() {
                    if g.start >= a && g.start < b {
                        min_x = min_x.min(g.x);
                        max_x = max_x.max(g.x + g.w);
                    }
                }
            }
            if max_x > min_x {
                let y = geom.text_top + 2.0 * m.line_height - 3.0;
                Some([geom.text_left + min_x, y, max_x - min_x, 1.5])
            } else {
                None
            }
        });
        false
    }

    /// Compose + shape the theme picker's full span stack into `panel_buffer`:
    /// query line 0 → lens strip line 1 (at `strip_scale` of BODY size — `1.0`
    /// normally, stepped down by the responsive fold when the shaped strip
    /// overflows the text column) → plan lines (faint LABEL-size section headers +
    /// world rows, the rows budgeted through [`rowlayout`]) → the dim foot hint.
    /// Line HEIGHTS stay uniform (`m.line_height`) at any strip scale, so the plan
    /// line offsets, the selected band, and the underline `y` never drift.
    #[allow(clippy::too_many_arguments)]
    fn shape_theme_spans(
        &mut self,
        geom: &OverlayGeom,
        ink: glyphon::Color,
        muted: glyphon::Color,
        strip_s: &str,
        label_ranges: &[(std::ops::Range<usize>, bool)],
        sep_ranges: &[std::ops::Range<usize>],
        hint_line: &str,
        strip_scale: f32,
    ) {
        let m = self.metrics;
        let faint = theme::faint().to_glyphon();
        let label = crate::markdown::type_scale::LABEL;
        let header_metrics = GlyphMetrics::new(m.font_size * label, m.line_height);
        let strip_metrics = GlyphMetrics::new(m.font_size * strip_scale, m.line_height);
        let base = panel_attrs();
        let mk = |c| base.clone().color(c);
        let sym = |c| Attrs::new().family(Family::Name(SYMBOL_FAMILY)).color(c);
        let sigil = "› ";

        // The world rows share the lone-column budget every no-right-column picker
        // gets (rowlayout owns it); today's short world names ride through whole.
        let total_chars = if m.char_width > 0.0 {
            (geom.text_w / m.char_width).floor() as usize
        } else {
            usize::MAX
        };
        let row_budget = rowlayout::full_budget(total_chars);
        let fitted: Vec<Option<String>> = geom
            .plan
            .iter()
            .map(|line| match line {
                ThemeLine::Header(_) => None,
                ThemeLine::Item(i) => {
                    let name = self.overlay_items.get(*i).map(|s| s.as_str()).unwrap_or("");
                    Some(rowlayout::fit_primary(name, row_budget))
                }
            })
            .collect();

        // Compose the spans. Query line 0 → strip line 1 → plan lines → hint.
        let mut spans: Vec<(&str, glyphon::Attrs)> = Vec::new();
        spans.push((sigil, mk(muted)));
        spans.push((self.overlay_query.as_str(), mk(ink)));
        // Strip line: active label in full ink, others muted, separators + the "\n"
        // faint. One ordered pass over `strip_s` so the spans tile the line in byte
        // order (rich-text concatenates spans in push order). The label/separator
        // spans carry `strip_metrics`; the leading "\n" keeps BODY metrics so the
        // strip row's HEIGHT (and everything below it) is scale-invariant.
        {
            let mut cursor = 0usize;
            let mut pushes: Vec<(std::ops::Range<usize>, glyphon::Color)> = Vec::new();
            for (r, active) in label_ranges {
                pushes.push((r.clone(), if *active { ink } else { muted }));
            }
            for r in sep_ranges {
                pushes.push((r.clone(), faint));
            }
            pushes.sort_by_key(|(r, _)| r.start);
            spans.push((&strip_s[0..1], mk(faint))); // the "\n", BODY metrics
            cursor += 1;
            for (r, c) in pushes {
                debug_assert_eq!(r.start, cursor, "strip spans must tile the line");
                cursor = r.end;
                let attrs = if strip_scale < 1.0 {
                    mk(c).metrics(strip_metrics)
                } else {
                    mk(c)
                };
                spans.push((&strip_s[r], attrs));
            }
        }
        // Plan lines: faint uppercase section headers (LABEL size) + world rows (ink).
        for (line, fit) in geom.plan.iter().zip(fitted.iter()) {
            spans.push(("\n", mk(ink)));
            match line {
                ThemeLine::Header(h) => {
                    spans.push((h.as_str(), mk(faint).metrics(header_metrics)));
                }
                ThemeLine::Item(_) => {
                    spans.push((fit.as_deref().unwrap_or(""), mk(ink)));
                }
            }
        }
        if geom.hint_rows > 0 {
            let mut lastb = 0usize;
            for run in symbol_runs(hint_line) {
                if run.start > lastb {
                    spans.push((&hint_line[lastb..run.start], mk(muted)));
                }
                let end = run.end;
                spans.push((&hint_line[run], sym(muted)));
                lastb = end;
            }
            if lastb < hint_line.len() {
                spans.push((&hint_line[lastb..], mk(muted)));
            }
        }

        self.panel_buffer
            .set_size(&mut self.font_system, Some(geom.text_w), Some(geom.card_h));
        self.panel_buffer.set_wrap(&mut self.font_system, Wrap::None);
        let default_attrs = base.clone().color(ink);
        self.panel_buffer.set_rich_text(
            &mut self.font_system,
            spans,
            &default_attrs,
            Shaping::Advanced,
            None,
        );
        self.panel_buffer
            .shape_until_scroll(&mut self.font_system, false);
    }

    /// The shaped WIDTH (px) of the theme picker's lens-strip line (line 1 of the
    /// just-shaped `panel_buffer`) — what the responsive fold compares against the
    /// card's text column.
    fn theme_strip_px(&self) -> f32 {
        let mut w = 0.0f32;
        for run in self.panel_buffer.layout_runs() {
            if run.line_i == 1 {
                w = w.max(run.line_w);
            }
        }
        w
    }

    /// Upload the shaped overlay text areas: the name column at the panel origin,
    /// plus (when present) the right-aligned chord column whose own right edge lands
    /// at `text_left + text_w` = the card's right text edge → chords flush.
    #[allow(clippy::too_many_arguments)]
    fn overlay_upload_text(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        geom: &OverlayGeom,
        has_right: bool,
        ink: glyphon::Color,
        muted: glyphon::Color,
    ) -> anyhow::Result<()> {
        let text_left = geom.text_left;
        let text_top = geom.text_top;
        // Clip the rows to the card's TEXT column so a row elided a hair long is cut at
        // the card's right text edge rather than spilling into the backdrop.
        let bounds = TextBounds {
            left: text_left.max(0.0) as i32,
            top: 0,
            right: ((text_left + geom.text_w).min(width as f32)) as i32,
            bottom: height as i32,
        };
        let panel_area = TextArea {
            buffer: &self.panel_buffer,
            left: text_left,
            top: text_top,
            scale: 1.0,
            bounds,
            default_color: ink,
            custom_glyphs: &[],
        };
        // The right-aligned label column shares the panel origin; its own right edge
        // lands at `text_left + text_w` = the card's right text edge → chords flush.
        let mut areas: Vec<TextArea> = vec![panel_area];
        if has_right {
            areas.push(TextArea {
                buffer: &self.panel_bind_buffer,
                left: text_left,
                top: text_top,
                scale: 1.0,
                bounds,
                default_color: muted,
                custom_glyphs: &[],
            });
        }
        self.panel_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                areas,
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon overlay prepare failed: {e:?}"))?;
        Ok(())
    }

    /// Upload the card behind everything + the muted selected-row highlight quad
    /// positioned over the chosen candidate.
    ///
    /// The card is drawn one of two ways. The CENTERED overlays (go-to / command /
    /// theme / …) use the flat opaque `panel_card`. The contextual SPELL panel instead
    /// rides the reusable FLOATING-PANEL primitive ([`Self::prepare_float_panel`]) —
    /// shadow + raised border + card — so it reads as risen a step above the crisp
    /// document with NO scrim (DESIGN §5/§8); `panel_card` is left empty then.
    fn overlay_draw_card(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        geom: &OverlayGeom,
    ) {
        let m = self.metrics;
        let card_rect = [geom.card_x, geom.card_y, geom.card_w, geom.card_h];
        if self.overlay_spell.is_some() {
            // Contextual spell panel: elevate on the float primitive, no flat card.
            self.prepare_float_panel(device, queue, width, height, Some(card_rect));
            self.panel_card.prepare(device, queue, width, height, &[]);
        } else {
            // Centered overlay: the flat opaque card; the float quads stay parked.
            self.panel_card
                .prepare(device, queue, width, height, &[card_rect]);
        }

        // Selected-row highlight: a VALUE BAND, the next rung up the surface ladder
        // past the card's `base_300` (`theme::surface_selected`), set per-frame so a
        // live theme switch reskins it. Figure/ground by VALUE — not the cool
        // `selection` hue, not the amber accent (DESIGN §3/§5). The selected name
        // stays content ink, readable on the band. The band sits `header_rows` lines
        // below the card top (past the query line, if any), matching the shaped rows.
        self.overlay_rows
            .set_color(theme::surface_selected().rgba_bytes());
        let sel_rects: Vec<[f32; 4]> = if geom.n_items == 0 {
            Vec::new()
        } else if geom.theme {
            // THEME PICKER: the selected world's DISPLAY row = its position in the plan
            // (headers push it down), offset past the query + strip lines (`header_rows`).
            let disp = geom
                .plan
                .iter()
                .position(|l| matches!(l, ThemeLine::Item(i) if *i == self.overlay_selected))
                .unwrap_or(0);
            let row_top = geom.text_top + (geom.header_rows + disp) as f32 * m.line_height;
            vec![[geom.card_x, row_top, geom.card_w, m.line_height]]
        } else {
            // 0-based row among the visible window. `OverlayState` keeps the selection
            // inside `[top_idx, top_idx+visible)`; saturate + clamp defensively so a
            // transient mismatch (e.g. the list just shrank) can never underflow/overflow.
            let sel_row = self
                .overlay_selected
                .saturating_sub(geom.top_idx)
                .min(geom.visible.saturating_sub(1)); // 0-based among visible
            let row_top =
                geom.text_top + (geom.header_rows + sel_row) as f32 * m.line_height;
            vec![[geom.card_x, row_top, geom.card_w, m.line_height]]
        };
        self.overlay_rows
            .prepare(device, queue, width, height, &sel_rects);
        // THEME PICKER active-lens underline: the rect the shaper recorded; a non-theme
        // card parks it empty (so a stale rect from a prior theme picker never lingers).
        let underline: Vec<[f32; 4]> = if geom.theme {
            self.overlay_theme_underline.iter().copied().collect()
        } else {
            Vec::new()
        };
        self.overlay_lens_underline
            .prepare(device, queue, width, height, &underline);
    }

    /// Place the one amber caret: a resting block at the end of the query line. Read
    /// the first shaped row's width so the caret lands at the query end on a
    /// proportional world face too (not a fixed `char_width` assumption); fall back
    /// to fixed-pitch if shaping yielded no run.
    ///
    /// The contextual SPELL panel has NO query line to edit, so its caret is PARKED
    /// (nothing drawn) — the suggestions are picked by click / arrows + Enter, not by
    /// typing a query, so a blinking amber block would be noise (and amber stays the
    /// document caret's alone, DESIGN §3).
    fn overlay_place_caret(
        &mut self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        geom: &OverlayGeom,
    ) {
        if geom.header_rows == 0 {
            self.panel_caret.prepare_empty();
            return;
        }
        let m = self.metrics;
        let sigil = "› ";
        let caret_x = geom.text_left
            + self
                .panel_buffer
                .layout_runs()
                .next()
                .map(|r| r.line_w)
                .unwrap_or_else(|| {
                    m.char_width
                        * (sigil.chars().count() + self.overlay_query.chars().count()) as f32
                });
        let caret_h = m.caret_h * 0.8;
        let caret_cx = caret_x + m.caret_w * 0.5;
        let caret_cy = geom.text_top + m.line_height * 0.5;
        self.panel_caret.prepare(
            queue,
            width,
            height,
            caret_cx,
            caret_cy,
            m.caret_w,
            caret_h,
            CORNER_RADIUS,
        );
    }

    /// Shape one quiet corner label into `buffer` and `prepare` it into `renderer`,
    /// parking it off-screen when `text` is empty. This is the shared body behind the
    /// bottom-right word-count readout and the top-left DEBUG panel — each was a
    /// ~95%-identical copy differing only by the (renderer, buffer) pair, the text,
    /// the corner [`CornerAnchor`], and (for the debug panel) the metrics + row count.
    ///
    /// It takes `renderer` + `buffer` (and the four shared glyphon resources) as
    /// EXPLICIT `&mut` params rather than `&mut self`: the callers pass distinct
    /// fields, so a `&mut self` method couldn't also hand it `&mut
    /// self.wordcount_renderer`. `col_left` / `col_width` are the writing column's
    /// already-resolved geometry (so this stays free of `self`); `col_width` is only
    /// consulted for the right-aligned anchor. `gm` sets the buffer's glyph metrics (so
    /// a compact panel can ride a smaller size) and `rows` reserves that many
    /// line-heights of height so a STACKED multi-line label (the debug panel) shapes
    /// without clipping; a single-line label passes `rows == 1.0`.
    #[allow(clippy::too_many_arguments)]
    fn prepare_corner_label(
        renderer: &mut TextRenderer,
        buffer: &mut GlyphBuffer,
        font_system: &mut FontSystem,
        atlas: &mut TextAtlas,
        viewport: &Viewport,
        swash_cache: &mut SwashCache,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        gm: GlyphMetrics,
        rows: f32,
        col_left: f32,
        col_width: f32,
        text: &str,
        anchor: CornerAnchor,
        label: &str,
    ) -> anyhow::Result<()> {
        let muted = theme::muted().to_glyphon();
        let line_height = gm.line_height;
        buffer.set_metrics(font_system, gm);
        buffer.set_size(font_system, Some(width as f32), Some(line_height * rows.max(1.0)));
        buffer.set_text(font_system, text, &panel_attrs().color(muted), Shaping::Advanced, None);
        buffer.shape_until_scroll(font_system, false);
        // Empty text parks the label off-screen so nothing draws (and a default
        // capture stays byte-identical). The bottom row sits one line up from the
        // canvas bottom; the right-aligned anchor measures the shaped run width.
        let (left, top) = if text.is_empty() {
            (0.0, -1000.0)
        } else {
            match anchor {
                CornerAnchor::TopLeft => (col_left.max(8.0), 8.0),
                CornerAnchor::BottomRight => {
                    let mut text_w = 0.0_f32;
                    for run in buffer.layout_runs() {
                        text_w = text_w.max(run.line_w);
                    }
                    let left = (col_left + col_width - text_w).max(col_left);
                    (left, height as f32 - line_height - 8.0)
                }
                CornerAnchor::BottomCenter => {
                    let mut text_w = 0.0_f32;
                    for run in buffer.layout_runs() {
                        text_w = text_w.max(run.line_w);
                    }
                    let left = (col_left + (col_width - text_w) * 0.5).max(col_left);
                    (left, height as f32 - line_height - 8.0)
                }
                CornerAnchor::AtPoint(px, py) => {
                    let mut text_w = 0.0_f32;
                    for run in buffer.layout_runs() {
                        text_w = text_w.max(run.line_w);
                    }
                    // Float above-right of the pointer (clears the resize-cursor
                    // glyph it sits over), clamped onto the canvas so it never clips
                    // off an edge near the window border.
                    let left = (px + 14.0).min(width as f32 - text_w - 4.0).max(4.0);
                    let top = (py - line_height - 10.0).max(4.0);
                    (left, top)
                }
            }
        };
        let bounds = TextBounds { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        let area = TextArea {
            buffer,
            left,
            top,
            scale: 1.0,
            bounds,
            default_color: muted,
            custom_glyphs: &[],
        };
        renderer
            .prepare(device, queue, font_system, atlas, viewport, [area], swash_cache)
            .map_err(|e| anyhow::anyhow!("glyphon {label} prepare failed: {e:?}"))?;
        Ok(())
    }

    /// The page-mode GUTTER's available RIGHT-aligned width (px), or `None` when the
    /// gutter is HIDDEN: edge-to-edge (no margin to hold it), no buffer name, or a
    /// margin too narrow for the label. The label's right edge lands at this width — a
    /// small gap shy of the writing column's left edge — so it hugs the column from the
    /// margin. Shared by [`Self::prepare_gutter`] (what is drawn) and
    /// [`Self::gutter_report`] (what the sidecar says), so the two never drift.
    fn gutter_geom(&self) -> Option<f32> {
        let gap = self.metrics.char_width * 1.5;
        let avail = self.column_left() - gap;
        if crate::page::page_on() && !self.gutter_name.is_empty() && avail >= 60.0 {
            Some(avail)
        } else {
            None
        }
    }

    /// Shape + upload the page-mode ORIENTATION GUTTER: a quiet stacked label in the
    /// BOTTOM-LEFT margin — the filename (LABEL size × MUTED ink) over the project (LABEL ×
    /// FAINT ink), RIGHT-aligned so it hugs the writing column from the margin and
    /// anchored to the BOTTOM of the left margin. This relocates orientation OUT of the
    /// writing column into the side (DESIGN §4: the faintest inks at the smallest size,
    /// present when you look, invisible when you don't). HIDDEN edge-to-edge / with no
    /// name (parked off-screen → byte-identical).
    pub(super) fn prepare_gutter(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let m = self.metrics;
        let label = crate::markdown::type_scale::LABEL;
        let muted = theme::muted().to_glyphon();
        let faint = theme::faint().to_glyphon();
        // A compact stacked label: scale BOTH font size and line height to LABEL so the
        // two rows nest tightly (this buffer is standalone, not row-aligned to the doc).
        self.gutter_buffer.set_metrics(
            &mut self.font_system,
            GlyphMetrics::new(m.font_size * label, m.line_height * label),
        );
        let base = panel_attrs();
        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        // Hidden: empty text parked off-screen, so nothing draws and a non-page (or
        // unnamed) capture stays byte-identical.
        let Some(avail) = self.gutter_geom() else {
            self.gutter_buffer
                .set_size(&mut self.font_system, Some(1.0), Some(m.line_height));
            self.gutter_buffer.set_text(
                &mut self.font_system,
                "",
                &base.clone().color(muted),
                Shaping::Advanced,
                None,
            );
            self.gutter_buffer
                .shape_until_scroll(&mut self.font_system, false);
            let area = TextArea {
                buffer: &self.gutter_buffer,
                left: 0.0,
                top: -1000.0,
                scale: 1.0,
                bounds,
                default_color: muted,
                custom_glyphs: &[],
            };
            self.gutter_renderer
                .prepare(
                    device,
                    queue,
                    &mut self.font_system,
                    &mut self.atlas,
                    &self.viewport,
                    [area],
                    &mut self.swash_cache,
                )
                .map_err(|e| anyhow::anyhow!("glyphon gutter prepare failed: {e:?}"))?;
            return Ok(());
        };
        let name = self.gutter_name.clone();
        let project = self.gutter_project.clone();
        // Filename (muted) over project (faint). The project line carries its own
        // leading newline so it stacks under the filename; empty project => name only.
        let proj_line = if project.is_empty() {
            String::new()
        } else {
            format!("\n{project}")
        };
        let mut spans: Vec<(&str, Attrs)> = vec![(name.as_str(), base.clone().color(muted))];
        if !proj_line.is_empty() {
            spans.push((proj_line.as_str(), base.clone().color(faint)));
        }
        let lines = if proj_line.is_empty() { 1.0 } else { 2.0 };
        self.gutter_buffer.set_size(
            &mut self.font_system,
            Some(avail),
            Some(m.line_height * label * lines + 1.0),
        );
        let default_attrs = base.clone().color(muted);
        self.gutter_buffer.set_rich_text(
            &mut self.font_system,
            spans,
            &default_attrs,
            Shaping::Advanced,
            Some(glyphon::cosmic_text::Align::Right),
        );
        self.gutter_buffer
            .shape_until_scroll(&mut self.font_system, false);
        // BOTTOM-anchored in the left margin: the stacked block's BOTTOM edge sits one
        // small margin (8px) up from the canvas bottom — the same bottom row the corner
        // readouts use — so `top` is the canvas bottom minus the block's own height. Left
        // 0 with the buffer width == `avail` keeps the right edge a gap shy of the column
        // (horizontal placement unchanged; only the vertical anchor moved top → bottom).
        let block_h = m.line_height * label * lines;
        let top = height as f32 - block_h - 8.0;
        let area = TextArea {
            buffer: &self.gutter_buffer,
            left: 0.0,
            top,
            scale: 1.0,
            bounds,
            default_color: muted,
            custom_glyphs: &[],
        };
        self.gutter_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon gutter prepare failed: {e:?}"))?;
        Ok(())
    }

    /// The page-mode GUTTER state for the capture sidecar: `Some((name, project))`
    /// EXACTLY when the gutter is drawn (page mode on, a buffer name, a wide-enough
    /// margin — the same gate as [`Self::prepare_gutter`]), else `None`. So the
    /// sidecar's `gutter` block always agrees with the pixels.
    pub fn gutter_report(&self) -> Option<(String, String)> {
        self.gutter_geom()
            .map(|_| (self.gutter_name.clone(), self.gutter_project.clone()))
    }

    /// True when a FULL-takeover overlay is up and the document RECEDES behind it (the
    /// cached frosted-blur backdrop is active). False for the search SPLIT panel / no
    /// overlay (the doc stays bright), for the crisp THEME/CARET pickers (the doc stays
    /// crisp so the live theme colours / caret preview read honestly), AND for the
    /// contextual SPELL panel (a small float popup at the word — it recedes nothing).
    /// Reported in the sidecar as `dim_overlay`.
    pub fn dims_doc(&self) -> bool {
        self.overlay_active && !self.overlay_crisp && self.overlay_spell.is_none()
    }

    /// The word count of the current buffer (whitespace-separated tokens). Summed
    /// per line — a word never spans a newline — so it equals
    /// [`crate::markdown::word_count`] of the whole document without joining it.
    fn word_count(&self) -> usize {
        self.buffer
            .lines
            .iter()
            .map(|l| crate::markdown::word_count(l.text()))
            .sum()
    }

    /// The QUIET readout for a MARKDOWN buffer: `Some((words, reading_minutes))` when
    /// the buffer is markdown and has at least one word, else `None` (nothing drawn).
    /// Exposed so the capture sidecar can report exactly what the readout shows.
    pub fn readout_report(&self) -> Option<(usize, usize)> {
        if !self.md_enabled {
            return None;
        }
        let words = self.word_count();
        if words == 0 {
            return None;
        }
        Some((words, crate::markdown::reading_time_min(words)))
    }

    /// The readout string for the bottom-right corner, e.g. `"240 words · 2 min"`.
    /// Empty when there is nothing to show (non-markdown or wordless).
    ///
    /// REUSED by the held HUD's WORD COUNT figure (phase 2): the persistent
    /// bottom-right readout is no longer drawn, but this text-feeder +
    /// [`Self::readout_report`] (the sidecar source) live on as the HUD's source.
    fn wordcount_text(&self) -> String {
        match self.readout_report() {
            Some((w, m)) => {
                let unit = if w == 1 { "word" } else { "words" };
                format!("{w} {unit} · {m} min")
            }
            None => String::new(),
        }
    }

    /// Shape + upload the quiet word-count / reading-time readout. Drawn DIM and
    /// RIGHT-aligned to the writing column's right edge, on the bottom row. Empty text
    /// parks it off-screen (markdown gate / empty doc), so a non-markdown buffer draws
    /// nothing and stays byte-identical.
    ///
    /// RETAINED (unused) for phase 2: the persistent readout was removed from the
    /// chrome layer (it moves into the held HUD); this shaper stays for that reuse.
    #[allow(dead_code)]
    pub(super) fn prepare_wordcount(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let text = self.wordcount_text();
        let (gm, col_left, col_width) =
            (self.metrics.glyph_metrics(), self.column_left(), self.column_width());
        Self::prepare_corner_label(
            &mut self.wordcount_renderer,
            &mut self.wordcount_buffer,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            &mut self.swash_cache,
            device,
            queue,
            width,
            height,
            gm,
            1.0,
            col_left,
            col_width,
            &text,
            CornerAnchor::BottomRight,
            "wordcount",
        )
    }

    /// Shape + upload the CALM NOTICE — one quiet LABEL-sized line in the muted
    /// ink at the BOTTOM-CENTER of the writing column (today: the autosave
    /// clobber guard's "changed on disk outside awl — autosave held"). Mirrors
    /// [`Self::prepare_wordcount`] through the shared corner-label body; an
    /// EMPTY notice parks it off-screen, so every capture (which can never have
    /// a notice — autosave is live-only) stays byte-identical.
    pub(super) fn prepare_notice(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let text = self.notice.clone();
        let m = self.metrics;
        let label = crate::markdown::type_scale::LABEL;
        let gm = GlyphMetrics::new(m.font_size * label, m.line_height * label);
        let (col_left, col_width) = (self.column_left(), self.column_width());
        Self::prepare_corner_label(
            &mut self.notice_renderer,
            &mut self.notice_buffer,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            &mut self.swash_cache,
            device,
            queue,
            width,
            height,
            gm,
            1.0,
            col_left,
            col_width,
            &text,
            CornerAnchor::BottomCenter,
            "notice",
        )
    }

    /// Shape + upload the PAGE-WIDTH DRAG READOUT: a quiet muted char-count (e.g.
    /// "68") floating near the pointer while a page-column edge drag is in
    /// progress — Butterick's line-length rule made visible (value-only ink, NEVER
    /// amber — DESIGN §3). Mirrors [`Self::prepare_notice`]'s corner-label body but
    /// anchors AT the pointer ([`CornerAnchor::AtPoint`]) instead of a canvas
    /// corner. `page_drag_readout` is `None` (not dragging — the ONLY state a
    /// headless capture can ever see) parks it off-screen, so every capture stays
    /// byte-identical.
    pub(super) fn prepare_page_drag_readout(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let (text, anchor) = match self.page_drag_readout {
            Some((px, py, measure)) => (measure.to_string(), CornerAnchor::AtPoint(px, py)),
            None => (String::new(), CornerAnchor::AtPoint(0.0, 0.0)),
        };
        let m = self.metrics;
        let label = crate::markdown::type_scale::LABEL;
        let gm = GlyphMetrics::new(m.font_size * label, m.line_height * label);
        let (col_left, col_width) = (self.column_left(), self.column_width());
        Self::prepare_corner_label(
            &mut self.page_drag_renderer,
            &mut self.page_drag_buffer,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            &mut self.swash_cache,
            device,
            queue,
            width,
            height,
            gm,
            1.0,
            col_left,
            col_width,
            &text,
            anchor,
            "page_drag_readout",
        )
    }

    /// Feed the DEBUG panel's perf lines in one write, called at the TOP of a live
    /// redraw (the panel text is shaped inside `prepare`, so the values land on the
    /// frame being drawn): the previous completed frame's `(cost, worst)` pair, the
    /// key→px latency, the monotonic redraw count, whether this frame draws the
    /// SETTLED (`still ·`) form, and the current monitor's adaptive frame budget.
    /// The headless path never calls this, so the defaults (all `None`, still=true)
    /// compose the fixed, clockless still-form placeholders. Toggling the panel off
    /// re-feeds the defaults so the next enable starts fresh.
    pub fn set_debug_perf(
        &mut self,
        cost: Option<(f32, f32)>,
        latency_ms: Option<f32>,
        redraws: Option<u64>,
        still: bool,
        budget_ms: Option<f32>,
    ) {
        self.debug_frame_cost = cost;
        self.debug_latency_ms = latency_ms;
        self.debug_redraws = redraws;
        self.debug_still = still;
        self.debug_budget_ms = budget_ms;
    }

    /// The panel's MACHINE-READABLE perf state for the capture sidecar — the same
    /// values the drawn lines fold, exposed raw so the agent can triage numbers
    /// without parsing the text. In a capture every clocked field is `None` (the
    /// constructor defaults; no clock ever ran) and `still` is true, so the block
    /// is byte-stable across machines.
    pub fn debug_perf_report(&self) -> DebugPerfReport {
        DebugPerfReport {
            frame_ms: self.debug_frame_cost.map(|(last, _)| last),
            worst_ms: self.debug_frame_cost.map(|(_, worst)| worst),
            budget_ms: self.debug_budget_ms,
            key_px_ms: self.debug_latency_ms,
            redraws: self.debug_redraws,
            still: self.debug_still,
            autosave: self.debug_autosave,
        }
    }

    /// Feed the debug panel the latest queried GPU memory (bytes), for the `gpu <n> MB`
    /// line. `None` (no query — non-macOS backend, or a capture) leaves the fixed
    /// `gpu —` placeholder. Live-only device state, exactly like the frametime.
    pub fn set_debug_gpu_bytes(&mut self, bytes: Option<u64>) {
        self.debug_gpu_bytes = bytes;
    }

    /// Feed the debug panel the AUTOSAVE ENGINE's current state (see
    /// `crate::debug::AutosaveState`), for the `autosave …` line. Fed ONLY by the
    /// live App, composed EXCLUSIVELY from what `App::autosave_flush`'s one door
    /// (+ its clobber-guard sub-paths) already tracks — never a fresh guess — so
    /// the line cannot drift from the engine's own truth. `None` (never fed — the
    /// headless capture's only reachable value, since the engine is structurally
    /// live-App-only) leaves the fixed `"autosave —"` placeholder.
    pub fn set_debug_autosave(&mut self, state: Option<crate::debug::AutosaveState>) {
        self.debug_autosave = state;
    }

    /// LIVE-ONLY: set (or clear) the PAGE-WIDTH DRAG READOUT — the pointer position
    /// (physical px) + the current measure (chars) the quiet label floats near the
    /// cursor while a page-column edge drag is in progress. `None` clears it (drag
    /// released, or not dragging — the default), parking the label off-screen.
    /// Called only by the live App's drag handlers (`app/input.rs`); the headless
    /// capture/replay path never calls this (mouse motion isn't `--keys`-drivable),
    /// so a default capture — and every `--keys` replay — stays byte-identical.
    pub fn set_page_drag_readout(&mut self, r: Option<(f32, f32, usize)>) {
        self.page_drag_readout = r;
    }

    /// The DEBUG panel TEXT for the top-left corner: a small STACKED dev readout, one
    /// diagnostic per line. EMPTY when the panel is off (parks it off-screen, so a
    /// default capture stays byte-identical). The first THREE lines are the honest
    /// perf triad — frame cost vs the monitor's budget (`"frame 1.4 ms · worst 3.2
    /// · budget 16.6"`, still-prefixed once settled), key→px latency, and the
    /// frozen-while-idle redraw count — live numbers in the window, fixed clockless
    /// still-form placeholders in a capture. Every middle line is a PURE function of
    /// the deterministic view state, so a `--debug` capture is reproducible; the
    /// LAST line (autosave) is a fourth clock-bearing one, fed by the live loop like
    /// the perf triad. Exposed so the sidecar can report it verbatim.
    ///
    /// Lines: frame cost · key→px · redraws · zoom · viewport WxH @dpi · cursor
    /// ln:col · theme·caret·page-mode · md:yes/no·syn:lang · gpu N MB · autosave
    /// state — the md/syn line is the key styling diagnostic (is the buffer
    /// markdown; what syntax language), the gpu line is the live device memory
    /// (macOS only; `gpu —` elsewhere / in a capture), and the AUTOSAVE line is the
    /// engine's own truth (`autosave saved · Ns ago` / `held — disk changed` /
    /// `off` / `on`), fed EXCLUSIVELY through `App::autosave_flush`'s one door —
    /// a fourth clock-bearing line, so it too renders the fixed `autosave —`
    /// placeholder in a capture (the engine never runs headlessly).
    pub fn debug_text(&self) -> String {
        if !crate::debug::debug_on() {
            return String::new();
        }
        let m = self.metrics;
        // Lines 1-3 (clock-bearing): the only non-deterministic lines — fixed
        // still-form placeholders in a capture, live numbers in the window.
        let frame =
            crate::debug::frame_readout(self.debug_frame_cost, self.debug_budget_ms, self.debug_still);
        let latency = crate::debug::latency_readout(self.debug_latency_ms);
        let redraws = crate::debug::activity_readout(self.debug_redraws);
        let zoom = format!("zoom {}%", (m.zoom * 100.0).round() as i64);
        // Physical canvas WxH at the display scale (1.0 in a capture).
        let (width, height) = (self.window_w as u32, self.window_h as u32);
        let viewport = format!("{width}×{height} @{:.1}x", self.dpi);
        let cursor = format!("ln {}:{}", self.cursor_line, self.cursor_col);
        // theme · caret · page-mode — the active render-globals in one line.
        let page = if crate::page::page_on() { "page" } else { "edge" };
        let modes = format!(
            "{} · {} · {}",
            theme::active().name,
            crate::caret::mode().label(),
            page
        );
        // The KEY diagnostic: is this buffer markdown, and what syntax language? They
        // are mutually exclusive, so at most one is "yes" / named.
        let md = if self.md_enabled { "yes" } else { "no" };
        let syn = self.syn_lang_report().unwrap_or("—");
        let mdsyn = format!("md:{md} · syn:{syn}");
        // GPU-memory line (clock/device-state-ish, like the frametime): a live number
        // on macOS (Metal's currentAllocatedSize), the fixed `gpu —` placeholder
        // everywhere else and in a capture, so a `--debug` capture stays deterministic.
        let gpu = crate::debug::gpu_readout(self.debug_gpu_bytes);
        // AUTOSAVE-ENGINE line: the engine's own truth, fed EXCLUSIVELY through
        // `App::autosave_flush`'s one door — never a fresh guess. `None` (no live
        // App has ever fed this — the only value a capture ever sees) renders the
        // fixed `autosave —` placeholder, exactly like the perf triad + gpu line.
        let autosave = crate::debug::autosave_readout(self.debug_autosave);
        [frame, latency, redraws, zoom, viewport, cursor, modes, mdsyn, gpu, autosave].join("\n")
    }

    /// Shape + upload the opt-in DEBUG panel. Drawn DIM (the value-only, no-amber
    /// convention shared with the word-count readout) in the TOP-LEFT corner, at a
    /// compact LABEL size so the stacked dev lines stay quiet. Empty text (panel off)
    /// parks it off-screen, so a default capture draws nothing and stays byte-identical.
    pub(super) fn prepare_debug(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let text = self.debug_text();
        // A compact panel: LABEL-scaled font + a tight line height so the ~6 stacked
        // rows read as one quiet block, not a billboard.
        let label = crate::markdown::type_scale::LABEL;
        let m = self.metrics;
        let gm = GlyphMetrics::new(m.font_size * label, m.line_height * label);
        let rows = text.lines().count().max(1) as f32;
        // Anchor at the far-left MARGIN (col_left 0 -> the helper's 8px floor), not the
        // centered writing column: a stacked multi-line panel at the column edge would
        // sit on top of the prose, so the dev block lives clear in the left margin.
        Self::prepare_corner_label(
            &mut self.debug_renderer,
            &mut self.debug_buffer,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            &mut self.swash_cache,
            device,
            queue,
            width,
            height,
            gm,
            rows,
            0.0,
            0.0,
            &text,
            CornerAnchor::TopLeft,
            "debug",
        )
    }

    // ===== HELD STATS HUD =================================================

    /// The cursor's position as a whole-PERCENT through the document (0..=100), by
    /// CHAR offset over the total char count (newlines included). Deterministic — a
    /// pure function of the buffer + cursor — so it is shown in a capture. An empty
    /// document reads 0%.
    fn hud_percent(&self) -> u32 {
        let lines = &self.buffer.lines;
        let total_chars: usize = lines.iter().map(|l| l.text().chars().count()).sum();
        let denom = total_chars + lines.len().saturating_sub(1); // + inter-line newlines
        if denom == 0 {
            return 0;
        }
        let mut offset = 0usize;
        for l in lines.iter().take(self.cursor_line) {
            offset += l.text().chars().count() + 1; // + the line's trailing newline
        }
        offset += self.cursor_col;
        (((offset.min(denom) as f32) / denom as f32) * 100.0).round() as u32
    }

    /// The HUD's machine-readable state for the capture sidecar: which WRITER figures it
    /// shows, exactly as the rendered panel does, so the sidecar always agrees with the
    /// pixels. `words` is `None` for a non-markdown buffer (the word-count stat is
    /// omitted there); `percent` is the cursor's %-through-doc. Both are pure functions
    /// of the doc + cursor — no clock/filesystem field remains.
    pub fn hud_report(&self) -> HudReport {
        HudReport {
            held: crate::hud::hud_held(),
            words: self.readout_report(),
            percent: self.hud_percent(),
        }
    }

    /// Shape + upload the held STATS HUD: a LEFT-ALIGNED readout on a card — each stat a
    /// quiet CAPTION in FAINT ink at LABEL size over its VALUE in CONTENT ink at BODY
    /// size (the type system, ink × size) — NO amber anywhere (amber is the caret's
    /// alone). The stats share one left spine. The document recedes behind the shared
    /// FROSTED-BLUR backdrop (the `render` blur branch), NOT a grey scrim — so the HUD
    /// reads consistently with the palette. TRIMMED to the WRITER stats: WORD COUNT +
    /// reading time and %-THROUGH-DOC (the file-created date + session-time fluff were
    /// dropped). Drawn ONLY while the HUD is held (`crate::hud::hud_held`); released, the
    /// text is parked off-screen, so a default capture stays byte-identical. Every figure
    /// is a PURE function of the doc + cursor, so a `--hud` capture is deterministic.
    pub(super) fn prepare_hud(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let held = crate::hud::hud_held();
        // No scrim: while held, the document recedes behind the shared FROSTED-BLUR
        // backdrop (the `render` blur branch), so the HUD draws only its float card +
        // stats. The card rect (shadow -> raised border -> card) is uploaded once the
        // block extent is measured (held branch); released, park all three so nothing draws.
        if !held {
            set_float_quads(
                &mut self.hud_shadow,
                &mut self.hud_border,
                &mut self.hud_card,
                device,
                queue,
                width,
                height,
                None,
            );
        }

        let m = self.metrics;
        let bounds = TextBounds { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        let content = theme::base_content().to_glyphon();
        let faint = theme::faint().to_glyphon();

        // RELEASED: park an empty buffer off-screen (nothing drawn), matching the
        // corner-readout convention so a non-held capture is byte-identical.
        if !held {
            self.hud_buffer
                .set_size(&mut self.font_system, Some(1.0), Some(m.line_height));
            self.hud_buffer.set_text(
                &mut self.font_system,
                "",
                &panel_attrs().color(content),
                Shaping::Advanced,
                None,
            );
            self.hud_buffer
                .shape_until_scroll(&mut self.font_system, false);
            let area = TextArea {
                buffer: &self.hud_buffer,
                left: 0.0,
                top: -1000.0,
                scale: 1.0,
                bounds,
                default_color: content,
                custom_glyphs: &[],
            };
            self.hud_renderer
                .prepare(
                    device,
                    queue,
                    &mut self.font_system,
                    &mut self.atlas,
                    &self.viewport,
                    [area],
                    &mut self.swash_cache,
                )
                .map_err(|e| anyhow::anyhow!("glyphon hud prepare failed: {e:?}"))?;
            return Ok(());
        }

        // The stats, top to bottom: each a quiet CAPTION over its VALUE. TRIMMED to the
        // WRITER figures — WORD COUNT + reading time and %-THROUGH-DOC — both PURE
        // functions of the doc (no clock/filesystem field), so the capture is
        // deterministic. WORD COUNT is markdown-only (omitted for code/plain buffers).
        // EVERY value rides CONTENT ink — NO amber anywhere (the THROUGH-DOC % used to be
        // amber, a DESIGN §3 stretch since `primary` is the caret's alone; it is now
        // plain content ink). Built as owned strings so the span runs can borrow them.
        let label = crate::markdown::type_scale::LABEL;
        let mut stats: Vec<(&'static str, String)> = Vec::with_capacity(2);
        // WORD COUNT + reading time — markdown buffers only (omitted otherwise). Reuses
        // the same `wordcount_text` feeder the bottom-right readout used pre-phase-2.
        let words = self.wordcount_text();
        if !words.is_empty() {
            stats.push(("WORD COUNT", words));
        }
        stats.push(("THROUGH DOC", format!("{}%", self.hud_percent())));

        // LEFT-ALIGNED on a spine: each stat is a CAPTION line (faint ink, LABEL size)
        // directly over its VALUE line (content ink, BODY size — NO amber: the % is
        // plain content ink like the rest, since amber is the caret's alone), in a
        // tight vertical rhythm with a single blank LABEL line between groups (dropped
        // after the last). Owned strings first, then the borrowed span runs. Line role:
        // 0 = caption (faint/LABEL), 1 = value (content/BODY).
        let body_metrics = GlyphMetrics::new(m.font_size, m.line_height);
        let label_metrics = GlyphMetrics::new(m.font_size * label, m.line_height * label);
        let mut owned: Vec<(String, u8)> = Vec::with_capacity(stats.len() * 2);
        let last = stats.len().saturating_sub(1);
        for (i, (caption, value)) in stats.into_iter().enumerate() {
            owned.push((format!("{caption}\n"), 0)); // caption (label / faint)
            let val_line = if i == last {
                value
            } else {
                format!("{value}\n\n") // value + a blank gap before the next group
            };
            owned.push((val_line, 1));
        }
        let base = panel_attrs();
        let spans: Vec<(&str, Attrs)> = owned
            .iter()
            .map(|(s, role)| {
                let attrs = match role {
                    0 => base.clone().color(faint).metrics(label_metrics),
                    _ => base.clone().color(content).metrics(body_metrics),
                };
                (s.as_str(), attrs)
            })
            .collect();
        // No alignment (cosmic-text defaults to LEFT): each line starts at the buffer's
        // left edge, and the TextArea `left` (below) plants that spine inside the card.
        // Generous buffer width so the value lines never wrap.
        self.hud_buffer
            .set_size(&mut self.font_system, Some(width as f32), Some(height as f32));
        let default_attrs = base.clone().color(content).metrics(body_metrics);
        self.hud_buffer.set_rich_text(
            &mut self.font_system,
            spans,
            &default_attrs,
            Shaping::Advanced,
            None,
        );
        self.hud_buffer
            .shape_until_scroll(&mut self.font_system, false);
        // Vertically center the stacked block: measure the shaped run extent (height
        // AND max line width) and offset so the column sits in the middle of the canvas.
        let mut block_h = 0.0_f32;
        let mut block_w = 0.0_f32;
        for run in self.hud_buffer.layout_runs() {
            block_h = block_h.max(run.line_top + run.line_height);
            block_w = block_w.max(run.line_w);
        }
        let top = ((height as f32 - block_h) * 0.5).max(TEXT_TOP);
        // The calm card behind the stats: the block + generous padding, centered, risen
        // a value step over the dimmed doc so the figures read on a clean ground — on the
        // same float-panel elevation (shadow -> raised border -> card) as which-key.
        let pad_x = m.char_width * 3.0;
        let pad_y = m.line_height * 0.9;
        let card_w = block_w + pad_x * 2.0;
        let card_h = block_h + pad_y * 2.0;
        let card_x = (width as f32 - card_w) * 0.5;
        let card_y = top - pad_y;
        set_float_quads(
            &mut self.hud_shadow,
            &mut self.hud_border,
            &mut self.hud_card,
            device,
            queue,
            width,
            height,
            Some([card_x, card_y, card_w, card_h]),
        );
        let area = TextArea {
            buffer: &self.hud_buffer,
            left: card_x + pad_x,
            top,
            scale: 1.0,
            bounds,
            default_color: content,
            custom_glyphs: &[],
        };
        self.hud_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon hud prepare failed: {e:?}"))?;
        Ok(())
    }

    // ===== FLOATING PANEL PRIMITIVE + CARET-STYLE PREVIEW PANEL ============

    /// THE PANEL PRIMITIVE — a small, summoned, transient FLOATING PANEL: a discrete
    /// bordered box with CARD ELEVATION (a translucent drop SHADOW behind + below, a
    /// crisp raised BORDER edge, the opaque CARD), and crucially NO scrim — so it
    /// floats over the live document without dimming it, distinct from the full-width
    /// takeover overlay. `rect = Some([x, y, w, h])` summons it; `None` parks all three
    /// elevation quads empty (nothing drawn). Reusable: its FIRST use is the caret-style
    /// preview panel, and future summoned micro-panels (spell / thesaurus / which-key)
    /// prepare their own content over this same helper. "Summoned, not furniture"
    /// (DESIGN §5).
    pub(super) fn prepare_float_panel(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rect: Option<[f32; 4]>,
    ) {
        set_float_quads(
            &mut self.float_shadow,
            &mut self.float_border,
            &mut self.float_card,
            device,
            queue,
            width,
            height,
            rect,
        );
    }

    // ===== WHICH-KEY PANEL ================================================

    /// Set (or clear) the WHICH-KEY panel's rows: `Some(rows)` summons the panel with
    /// those `(key, command-name)` continuations, `None` puts it down. The App calls
    /// this on the prefix PAUSE (summon) and the instant the chord resolves/aborts
    /// (dismiss); the headless `--whichkey` capture sets it once. Idempotent — the
    /// rows only feed the next `prepare_whichkey`.
    pub fn set_whichkey(&mut self, rows: Option<Vec<(String, String)>>) {
        self.whichkey_rows = rows;
    }

    /// The which-key panel's rows for the sidecar / tests, or `None` when it is down —
    /// so a headless assertion can confirm the summoned continuation list without
    /// eyeballing pixels. Clones the small row list.
    pub fn whichkey_report(&self) -> Option<Vec<(String, String)>> {
        self.whichkey_rows.clone()
    }

    /// Shape + upload the summoned WHICH-KEY hint panel this frame: a calm bottom-left
    /// float card listing the prefix's follow-up keys, each a FAINT key label in a
    /// left column beside its MUTED command name (recessive ink — NO amber, which is
    /// the caret's alone, DESIGN §3). Parked (nothing drawn) unless `whichkey_rows` is
    /// `Some`, so a default frame stays byte-identical. Button-free: it TEACHES the
    /// keys, it is not clickable.
    pub(super) fn prepare_whichkey(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let bounds = TextBounds { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        let faint = theme::faint().to_glyphon();
        let muted = theme::muted().to_glyphon();
        let m = self.metrics;

        // DOWN: park the card elevation + the text off-screen (byte-identical default).
        let Some(rows) = self.whichkey_rows.clone() else {
            set_float_quads(
                &mut self.wk_shadow,
                &mut self.wk_border,
                &mut self.wk_card,
                device,
                queue,
                width,
                height,
                None,
            );
            self.wk_buffer
                .set_size(&mut self.font_system, Some(1.0), Some(m.line_height));
            self.wk_buffer.set_text(
                &mut self.font_system,
                "",
                &panel_attrs().color(muted),
                Shaping::Advanced,
                None,
            );
            self.wk_buffer.shape_until_scroll(&mut self.font_system, false);
            let area = TextArea {
                buffer: &self.wk_buffer,
                left: 0.0,
                top: -1000.0,
                scale: 1.0,
                bounds,
                default_color: muted,
                custom_glyphs: &[],
            };
            self.wk_renderer
                .prepare(
                    device,
                    queue,
                    &mut self.font_system,
                    &mut self.atlas,
                    &self.viewport,
                    [area],
                    &mut self.swash_cache,
                )
                .map_err(|e| anyhow::anyhow!("glyphon whichkey prepare failed: {e:?}"))?;
            return Ok(());
        };

        // A quiet HEADER (the prefix) over the continuation rows. The key column is
        // space-padded to one width so the names line up (proportional-font alignment is
        // approximate but calm — the same space-padding the find panel / gutter use).
        let key_w = rows.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0);
        // Owned line strings + a role tag: 0 = header (faint), 1 = key (faint),
        // 2 = name (muted). Each row is TWO spans (padded key, then name + newline).
        let mut owned: Vec<(String, u8)> = Vec::with_capacity(rows.len() * 2 + 1);
        owned.push((format!("{PREFIX_HEADER}\n"), 0));
        for (key, name) in &rows {
            // Right-pad the key to `key_w` then a two-space gutter before the name.
            let pad = key_w.saturating_sub(key.chars().count());
            owned.push((format!("{key}{}  ", " ".repeat(pad)), 1));
            owned.push((format!("{name}\n"), 2));
        }
        let base = panel_attrs();
        let body = GlyphMetrics::new(m.font_size, m.line_height);
        let spans: Vec<(&str, Attrs)> = owned
            .iter()
            .map(|(s, role)| {
                let attrs = match role {
                    0 | 1 => base.clone().color(faint).metrics(body),
                    _ => base.clone().color(muted).metrics(body),
                };
                (s.as_str(), attrs)
            })
            .collect();

        self.wk_buffer
            .set_size(&mut self.font_system, Some(width as f32), Some(height as f32));
        let default_attrs = base.clone().color(muted).metrics(body);
        self.wk_buffer.set_rich_text(
            &mut self.font_system,
            spans,
            &default_attrs,
            Shaping::Advanced,
            None,
        );
        self.wk_buffer.shape_until_scroll(&mut self.font_system, false);

        // Measure the shaped block, then plant a padded card in the BOTTOM-LEFT corner
        // (clear of the centered writing column, so it never covers where you type).
        let mut block_h = 0.0_f32;
        let mut block_w = 0.0_f32;
        for run in self.wk_buffer.layout_runs() {
            block_h = block_h.max(run.line_top + run.line_height);
            block_w = block_w.max(run.line_w);
        }
        let pad_x = m.char_width * 2.0;
        let pad_y = m.line_height * 0.6;
        let margin = 24.0_f32;
        let card_w = block_w + pad_x * 2.0;
        let card_h = block_h + pad_y * 2.0;
        let card_x = margin;
        let card_y = (height as f32 - margin - card_h).max(margin);
        set_float_quads(
            &mut self.wk_shadow,
            &mut self.wk_border,
            &mut self.wk_card,
            device,
            queue,
            width,
            height,
            Some([card_x, card_y, card_w, card_h]),
        );
        let area = TextArea {
            buffer: &self.wk_buffer,
            left: card_x + pad_x,
            top: card_y + pad_y,
            scale: 1.0,
            bounds,
            default_color: muted,
            custom_glyphs: &[],
        };
        self.wk_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon whichkey prepare failed: {e:?}"))?;
        Ok(())
    }

    /// The caret-style preview PANEL's geometry — a two-line-tall floating box that
    /// hangs just BELOW the picker card, sharing its left edge + width. `None` unless
    /// the caret-style picker is open. Returns `(rect, text_left, row_center_y)`: the
    /// sample line sits vertically centred in the box, indented one pad.
    fn caret_preview_panel_rect(&self, width: u32) -> Option<([f32; 4], f32, f32)> {
        self.caret_preview?;
        let m = self.metrics;
        let geom = self.overlay_geometry(width);
        let pad = 12.0;
        let gap = 10.0; // the breath between the picker card and the preview panel
        let box_h = 2.0 * m.line_height + 2.0 * pad; // a ~2-line box
        let x = geom.card_x;
        let y = geom.card_y + geom.card_h + gap;
        let text_left = x + pad;
        let row_cy = y + box_h * 0.5;
        Some(([x, y, geom.card_w, box_h], text_left, row_cy))
    }

    /// Headless report for the caret-style preview panel: `(rect, sample_text,
    /// beat_index, silhouette_drawn)` when the caret-style picker is open, else
    /// `None`. The state machine (current beat + the preview buffer's sample text) is
    /// a deterministic function of the timeline, so a SETTLED capture reports the
    /// fixed end-state (`text == SAMPLE`); `silhouette_drawn` is whether the Morph
    /// glyph-silhouette pipeline actually painted THIS frame (settled on a real
    /// inhabited glyph in Morph mode) — always `false` for Block/I-beam or a
    /// glyphless/fast-motion Morph moment — so the fix (the preview demonstrating the
    /// real silhouette, not a permanent thin bar) is assertable from the sidecar
    /// without eyeballing pixels.
    pub fn caret_preview_panel_report(&self) -> Option<([f32; 4], String, usize, bool)> {
        let (rect, _, _) = self.caret_preview_panel_rect(self.window_w as u32)?;
        Some((
            rect,
            self.caret_demo.text(),
            self.caret_demo.beat_index(),
            self.caret_preview_glyph_pipeline.is_drawn(),
        ))
    }

    /// FIRST USE of the panel primitive: the caret-style picker's live preview PANEL.
    /// A floating card below the picker holds the sample line `watch me glide, jump,
    /// and morph`, on which the SELECTED caret look runs the choreographed demo
    /// ([`crate::caret::CaretDemo`]) — typing, gliding, jumping, deleting + gulping —
    /// driven by a scripted `apply_core` timeline. Parked (nothing drawn) unless the
    /// caret-style picker is open. The choreography FEEL is live-only; a headless
    /// capture renders the deterministic SETTLED end-state (the fully-typed line at
    /// rest), pinned by `settle_caret_preview`.
    pub(super) fn prepare_caret_preview_panel(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let (look, rect, text_left, row_cy) = match (self.caret_preview, self.caret_preview_panel_rect(width)) {
            (Some(look), Some((rect, text_left, row_cy))) => (look, rect, text_left, row_cy),
            _ => {
                // Picker closed: park the panel, the caret quad(s), and the sample text.
                self.prepare_float_panel(device, queue, width, height, None);
                self.caret_preview_pipeline.prepare_empty();
                self.caret_preview_glyph_pipeline.clear();
                self.park_preview_text(device, queue, width, height)?;
                return Ok(());
            }
        };
        self.caret_demo.mode = look;
        self.prepare_float_panel(device, queue, width, height, Some(rect));

        // Shape the sample line into the preview buffer (calm content ink, world face).
        //
        // RESPONSIVE SAMPLE: at a narrow window the panel (which shares the picker
        // card's width) can be too tight for the full sample line at BODY size —
        // the line then wrapped under its one-line box and the panel read broken /
        // mostly empty. Instead the WHOLE demo steps down in scale just enough for
        // the settled sample to fit on its one line (estimated at the mean advance,
        // conservative for every face, and from the FULL sample so the scale never
        // jitters mid-choreography). At any comfortable width `s == 1.0` and the
        // panel is byte-identical.
        let m = self.metrics;
        let avail = rect[2] - 24.0;
        // One extra advance of headroom: a mono face's real width EQUALS the mean
        // estimate, so an exact-fit scale would land fractionally over and wrap.
        let est = (crate::caret::SAMPLE.chars().count() + 1) as f32 * m.char_width;
        let s = if est > avail { (avail / est).max(0.5) } else { 1.0 };
        let ink = theme::base_content().to_glyphon();
        self.preview_buffer
            .set_metrics(&mut self.font_system, GlyphMetrics::new(m.font_size * s, m.line_height * s));
        let text = self.caret_demo.text();
        self.preview_buffer
            .set_size(&mut self.font_system, Some(avail), Some(m.line_height * s));
        // The sample is ONE line by construction: never wrap it (a fractional
        // overshoot clips at the panel edge instead of folding under the box).
        self.preview_buffer.set_wrap(&mut self.font_system, Wrap::None);
        self.preview_buffer.set_text(
            &mut self.font_system,
            &text,
            &panel_attrs().color(ink),
            Shaping::Advanced,
            None,
        );
        self.preview_buffer
            .shape_until_scroll(&mut self.font_system, false);

        // Position the demo caret on the sample line: the shaped X of the char the
        // caret INHABITS. Morph mirrors the document anchor rule (one char BACK of
        // the insertion point — the glyph just typed; col-0 falls back to the
        // cursor char, see `crate::caret::morph_anchor_col`), so the picker demo
        // previews the real riding-the-last-letter behavior; Block/I-beam keep the
        // insertion cell.
        let anchor_char = match look {
            CaretMode::Morph => crate::caret::morph_anchor_col(self.caret_demo.cursor_char()),
            _ => self.caret_demo.cursor_char(),
        };
        let caret_x = text_left + self.preview_caret_local_x(anchor_char, &text);
        let target = crate::caret::Sample { x: caret_x, y: row_cy };
        let first = self.caret_demo.set_metrics(m.char_width * s, m.line_height * s);
        if first {
            // First frame: SNAP the caret onto the line (no glide-in from nowhere).
            self.caret_demo.anim.jump_to(target.x, target.y);
        } else if let Some(tick) = self.caret_demo.take_tick() {
            // Glide to the freshly-shaped cursor X on a real move, then arm the flinch
            // the fired beat earned (typing impact / delete squash / kill gulp / recoil)
            // — the SAME juice the document caret gets through `apply_core`'s effects.
            use crate::actions::Effect;
            let is_edit = matches!(
                tick.effect,
                Effect::TypeImpact | Effect::DeleteSquash | Effect::Gulp
            );
            if tick.moved {
                self.caret_demo.anim.set_edit_move(is_edit);
                self.caret_demo.anim.nav_to(target.x, target.y);
            }
            match tick.effect {
                Effect::TypeImpact => self.caret_demo.anim.type_impact(),
                Effect::DeleteSquash => self.caret_demo.anim.delete_squash(),
                Effect::Gulp => self.caret_demo.anim.gulp(),
                Effect::Recoil(dir) => self.caret_demo.anim.recoil(dir),
                _ => {}
            }
        }

        // Upload the sample text (top = row centre minus half a line height).
        let text_top = row_cy - 0.5 * m.line_height * s;
        let bounds = TextBounds { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        let area = TextArea {
            buffer: &self.preview_buffer,
            left: text_left,
            top: text_top,
            scale: 1.0,
            bounds,
            default_color: ink,
            custom_glyphs: &[],
        };
        self.preview_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon preview prepare failed: {e:?}"))?;

        // Emit the preview caret quad(s) from the demo spring, in the highlighted
        // look — the SAME spring/morph machinery as the document caret, at the
        // demo's scale, over the SAME shaped sample text just uploaded (so a Morph
        // silhouette's glyph masks match the glyphs actually on screen).
        self.emit_preview_caret(
            device, queue, width, height, look, s, anchor_char, &text, text_top,
        );
        Ok(())
    }

    /// The buffer-local pixel X (relative to the text left) of the caret at char index
    /// `cursor` on the shaped sample line: the shaped X of the glyph starting there, or
    /// the line's full width when the caret sits at the end. `0.0` for the empty line.
    fn preview_caret_local_x(&self, cursor: usize, text: &str) -> f32 {
        let byte = text
            .char_indices()
            .nth(cursor)
            .map(|(b, _)| b)
            .unwrap_or(text.len());
        let mut line_w = 0.0;
        for run in self.preview_buffer.layout_runs() {
            for g in run.glyphs.iter() {
                if g.start == byte {
                    return g.x;
                }
            }
            line_w = run.line_w;
        }
        line_w
    }

    /// Build + upload the preview caret quad(s) from the demo spring, in `look`,
    /// reusing the document caret's morph machinery: Block's settle-driven square ⇄
    /// streak, the slim I-beam comet, and — MORPH, SETTLED on a real inhabited glyph
    /// — the SAME glyph-SILHOUETTE the document caret paints, through this preview's
    /// own [`CaretGlyphPipeline`] (`caret_preview_glyph_pipeline`), so choosing Morph
    /// in the picker actually shows what it does: the sample letter recolored solid
    /// in the accent, not a permanent thin bar. Morph still DEFERS to the thin
    /// glyphless-anchor bar (a space / line start) or the plain streak (fast motion,
    /// settle factor below [`CARET_MORPH_SETTLE_SHOW`]), exactly as the document
    /// does (see [`TextPipeline::prepare_caret_layer`] for the shared three-way
    /// shape). The spring already sits in panel pixel coords (jumped/nav'd there
    /// above), so its centre is canvas-absolute.
    #[allow(clippy::too_many_arguments)]
    fn emit_preview_caret(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        look: CaretMode,
        demo_scale: f32,
        anchor_char: usize,
        text: &str,
        text_top: f32,
    ) {
        let m = self.metrics;
        let s = self.caret_demo.anim.settle_factor();

        // MORPH: resolve the anchor's inhabited glyph this frame (`None` at a line
        // start -- no produced glyph to light, mirroring `caret_inhabited_key` -- or a
        // genuinely glyphless cell like a space), and latch the OLD glyph as the
        // cross-fade "from" the moment the anchor actually changes (mirroring
        // `caret_from_key`'s document-side latch): the demo buffer has no `set_view`
        // seam to read the pre-move glyph from directly, but the anchor's glyph key
        // one frame ago is exactly `caret_preview_mask_to`'s cached key, since `to`
        // depends only on the (already-applied) cursor position, not on the spring.
        let to_key = if look == CaretMode::Morph
            && !crate::caret::morph_line_start(self.caret_demo.cursor_char())
        {
            preview_glyph_key_at(&self.preview_buffer, text, anchor_char)
        } else {
            None
        };
        let prior_to_key = self.caret_preview_mask_to.as_ref().map(|mk| mk.key);
        let latched_from = if prior_to_key != to_key {
            prior_to_key
        } else {
            self.caret_preview_from_key
        };
        self.caret_preview_from_key = latched_from;
        let paint_silhouette =
            look == CaretMode::Morph && to_key.is_some() && s >= CARET_MORPH_SETTLE_SHOW;

        if paint_silhouette {
            // The "from" glyph only fades out while the spring is actually settling
            // onto the new one; at rest (or with nothing latched) show a clean single
            // silhouette, matching `prepare_caret_masks`'s document-side gate.
            let from_key = if self.caret_demo.anim.is_animating() {
                latched_from
            } else {
                None
            };
            {
                let Self {
                    caret_preview_mask_to,
                    caret_preview_mask_from,
                    swash_cache,
                    font_system,
                    ..
                } = self;
                Self::ensure_mask(caret_preview_mask_to, swash_cache, font_system, device, queue, to_key);
                Self::ensure_mask(
                    caret_preview_mask_from,
                    swash_cache,
                    font_system,
                    device,
                    queue,
                    from_key,
                );
            }
            let pen_x = self.caret_demo.anim.pos.x;
            let baseline_y = self.preview_baseline_y(text_top);
            let box_of = |mask: &Option<GlyphMask>| -> [f32; 4] {
                match mask {
                    Some(mk) => [
                        pen_x + mk.left as f32,
                        baseline_y - mk.top as f32,
                        mk.width as f32,
                        mk.height as f32,
                    ],
                    None => [0.0, 0.0, 0.0, 0.0],
                }
            };
            let from_box = box_of(&self.caret_preview_mask_from);
            let to_box = box_of(&self.caret_preview_mask_to);
            let morph_t = if self.caret_preview_mask_from.is_some() {
                self.caret_demo.anim.settle_factor()
            } else {
                1.0
            };
            self.caret_preview_glyph_pipeline.prepare(
                device,
                queue,
                width,
                height,
                self.caret_preview_mask_from.as_ref(),
                from_box,
                self.caret_preview_mask_to.as_ref(),
                to_box,
                morph_t,
                1.0,
                CARET_MORPH_DILATE_PX * m.zoom * demo_scale,
            );
            self.caret_preview_pipeline.prepare_empty();
            return;
        }
        self.caret_preview_glyph_pipeline.clear();

        // FALLBACK (Block, I-beam, or Morph with no glyph to light / still in fast
        // motion): the settle-driven square/streak shape, unchanged from before.
        let anim = &self.caret_demo.anim;
        // The caret body rides the demo's responsive scale (1.0 at any comfortable
        // width) so it covers the scaled sample glyphs, not full-size ghosts of them.
        let (block_w, block_h, thin) = match look {
            // Block: a one-cell rounded square sitting on the character, its thin streak.
            CaretMode::Block => (m.char_width, m.caret_block_h, m.caret_streak_h),
            CaretMode::Ibeam => (IBEAM_W * m.zoom, m.caret_h, IBEAM_W * m.zoom),
            CaretMode::Morph => (CARET_SPACE_BAR_W * m.zoom, m.caret_block_h, IBEAM_W * m.zoom),
        };
        let (block_w, block_h, thin) = (block_w * demo_scale, block_h * demo_scale, thin * demo_scale);
        let speed = (anim.vel.x * anim.vel.x + anim.vel.y * anim.vel.y).sqrt();
        let streak_len = anim.streak_length(
            m.streak_len_for_speed(speed),
            m.caret_streak_max_len,
            m.caret_held_len,
        );
        let (center, half_along, half_across, axis) = anim.motion_geometry(
            block_w,
            block_h,
            thin,
            streak_len,
            m.caret_streak_gap,
            m.caret_trail_drop,
        );
        let corner = match look {
            CaretMode::Block => {
                STREAK_RADIUS * m.zoom + (CORNER_RADIUS * m.zoom - STREAK_RADIUS * m.zoom) * s
            }
            _ => (STREAK_RADIUS * m.zoom).max(half_across.min(half_along) * 0.6),
        };
        let (w, h, corner) =
            self.caret_demo
                .anim
                .pop_scale_dims(half_along * 2.0, half_across * 2.0, corner);
        self.caret_preview_pipeline.prepare_axis(
            queue, width, height, center.x, center.y, w, h, corner, 1.0, axis.0, axis.1,
        );
    }

    /// The pixel BASELINE y (canvas-absolute) of the preview panel's one shaped
    /// sample line, given the text's TOP y (`text_top`, the same value passed to the
    /// panel's `TextArea`) — the preview-panel sibling of `caret_baseline_y`, reading
    /// the SAME cosmic-text `run.line_y` convention but over the throwaway
    /// `preview_buffer`'s single run instead of the document's. Falls back to the
    /// text top on an unshaped/empty line (never actually hit by the silhouette
    /// path, which only draws once a real glyph was found there).
    fn preview_baseline_y(&self, text_top: f32) -> f32 {
        self.preview_buffer
            .layout_runs()
            .next()
            .map(|r| text_top + r.line_y)
            .unwrap_or(text_top)
    }

    /// Park the preview sample-line text off-screen (an empty buffer), matching the
    /// corner-readout convention so a non-caret-picker frame stays byte-identical.
    fn park_preview_text(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let m = self.metrics;
        let content = theme::base_content().to_glyphon();
        self.preview_buffer
            .set_size(&mut self.font_system, Some(1.0), Some(m.line_height));
        self.preview_buffer
            .set_text(&mut self.font_system, "", &panel_attrs().color(content), Shaping::Advanced, None);
        self.preview_buffer
            .shape_until_scroll(&mut self.font_system, false);
        let bounds = TextBounds { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        let area = TextArea {
            buffer: &self.preview_buffer,
            left: 0.0,
            top: -1000.0,
            scale: 1.0,
            bounds,
            default_color: content,
            custom_glyphs: &[],
        };
        self.preview_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon preview park failed: {e:?}"))?;
        Ok(())
    }
}

/// The `CacheKey` of the glyph starting at char index `idx` of `text`, as shaped
/// into `buf` (the throwaway, single-line, `Wrap::None` PREVIEW buffer) — the
/// picker-preview sibling of [`TextPipeline::cursor_glyph_key_at`]: the SAME
/// shaped-glyph-cluster walk (byte range containing the target byte ->
/// `glyph.physical((0,0),1.0).cache_key`), just over the demo buffer instead of
/// the document, and with no per-line filtering since the sample is always one
/// line. `None` past the end of the text (nothing to silhouette) or at a byte
/// with no covering glyph run (a space, or an as-yet-unshaped buffer).
fn preview_glyph_key_at(buf: &GlyphBuffer, text: &str, idx: usize) -> Option<CacheKey> {
    let byte = text
        .char_indices()
        .nth(idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len());
    if byte >= text.len() {
        return None;
    }
    for run in buf.layout_runs() {
        for g in run.glyphs.iter() {
            if byte >= g.start && byte < g.end {
                return Some(g.physical((0.0, 0.0), 1.0).cache_key);
            }
        }
    }
    None
}

/// PURE row hit-test math for the summoned overlay: map a pointer `(px, py)` to the
/// `items` index of the candidate row it lands on, given the card box (`card_x`,
/// `card_w`), the inner text origin (`text_top`), the row `line_height`, the count of
/// `header_rows` ABOVE the list (`1` = the flat/nav pickers' query line, `0` = the
/// contextual spell panel), and the visible WINDOW (`visible` rows from `top_idx`,
/// `n_items` total). Returns `None` when the pointer is off the card horizontally,
/// above the first candidate row (which begins `header_rows` lines below `text_top`),
/// or past the last visible row. Split out of [`TextPipeline::overlay_row_at`] so the
/// mapping is unit-testable without a GPU pipeline — the rendered rows and this
/// hit-test share the exact same geometry, so they cannot drift.
#[allow(clippy::too_many_arguments)]
pub(super) fn overlay_row_index(
    card_x: f32,
    card_w: f32,
    text_top: f32,
    line_height: f32,
    header_rows: usize,
    visible: usize,
    top_idx: usize,
    n_items: usize,
    px: f32,
    py: f32,
) -> Option<usize> {
    if n_items == 0 || visible == 0 || line_height <= 0.0 {
        return None;
    }
    if px < card_x || px > card_x + card_w {
        return None;
    }
    // Candidate row 0 sits `header_rows` line heights below `text_top` (past the query
    // row, if any), matching the selected-row highlight in `overlay_draw_card`.
    let first_top = text_top + header_rows as f32 * line_height;
    if py < first_top {
        return None;
    }
    let vis = ((py - first_top) / line_height) as usize;
    if vis >= visible {
        return None;
    }
    let idx = top_idx + vis;
    (idx < n_items).then_some(idx)
}

/// The held stats HUD's machine-readable figures for the capture sidecar (see
/// [`TextPipeline::hud_report`]). Each field mirrors a rendered WRITER figure so the
/// sidecar agrees with the pixels: `held` is the summoned state, `words` is
/// `Some((words, reading_min))` for a markdown buffer (else `None`, the stat omitted),
/// and `percent` is the cursor's %-through-doc. The former clock/filesystem fields
/// (file-created date, session time) were dropped along with their HUD rows.
pub struct HudReport {
    pub held: bool,
    pub words: Option<(usize, usize)>,
    pub percent: u32,
}

/// The DEBUG panel's machine-readable perf state — the raw values behind the
/// drawn lines, mirrored into the capture sidecar's `debug` block so the agent
/// triages numbers, not prose. All clocked fields are `None` in a capture (no
/// clock ever runs there) and `still` defaults true (a capture IS the settled
/// state), keeping the block byte-stable. See [`TextPipeline::debug_perf_report`].
pub struct DebugPerfReport {
    pub frame_ms: Option<f32>,
    pub worst_ms: Option<f32>,
    pub budget_ms: Option<f32>,
    pub key_px_ms: Option<f32>,
    pub redraws: Option<u64>,
    pub still: bool,
    /// The AUTOSAVE ENGINE's state (see `crate::debug::AutosaveState`), fed by the
    /// live loop from `App::autosave_flush`'s one door. `None` in every capture
    /// (the engine is structurally live-App-only), mirroring the other clocked
    /// fields' placeholder convention.
    pub autosave: Option<crate::debug::AutosaveState>,
}

#[cfg(test)]
mod hit_tests {
    use super::overlay_row_index;

    // A representative overlay card geometry (card_x=420, card_w=360, text_top=64,
    // line_height=24) with a WINDOW of 5 visible rows out of 8, scrolled so the top
    // visible row is corpus index 2 (top_idx=2). Row R (0-based visible) spans y in
    // [text_top + (1+R)*lh, text_top + (2+R)*lh) → the first row starts at 88.
    const CARD_X: f32 = 420.0;
    const CARD_W: f32 = 360.0;
    const TEXT_TOP: f32 = 64.0;
    const LH: f32 = 24.0;

    fn hit(px: f32, py: f32, visible: usize, top_idx: usize, n: usize) -> Option<usize> {
        // The flat/nav pickers: one header row (the query line).
        overlay_row_index(CARD_X, CARD_W, TEXT_TOP, LH, 1, visible, top_idx, n, px, py)
    }

    fn hit_spell(px: f32, py: f32, visible: usize, top_idx: usize, n: usize) -> Option<usize> {
        // The contextual spell panel: NO query line, so rows start at `text_top`.
        overlay_row_index(CARD_X, CARD_W, TEXT_TOP, LH, 0, visible, top_idx, n, px, py)
    }

    #[test]
    fn pointer_maps_to_the_row_under_it() {
        // First candidate row (visible 0 → items index top_idx) begins at y=88.
        assert_eq!(hit(500.0, 88.0, 5, 2, 8), Some(2)); // top of row 0
        assert_eq!(hit(500.0, 100.0, 5, 2, 8), Some(2)); // mid row 0
        assert_eq!(hit(500.0, 112.0, 5, 2, 8), Some(3)); // row 1
        // Last visible row (visible 4 → items index 6) spans [184, 208).
        assert_eq!(hit(500.0, 200.0, 5, 2, 8), Some(6));
    }

    #[test]
    fn query_row_and_above_are_not_rows() {
        // The query line occupies [text_top, text_top+lh) = [64, 88): no candidate.
        assert_eq!(hit(500.0, 70.0, 5, 2, 8), None);
        assert_eq!(hit(500.0, 0.0, 5, 2, 8), None);
    }

    #[test]
    fn below_the_last_visible_row_is_none() {
        // Past the 5th visible row (which ends at 208) — e.g. the foot hint — is None.
        assert_eq!(hit(500.0, 210.0, 5, 2, 8), None);
    }

    #[test]
    fn off_the_card_horizontally_is_none() {
        assert_eq!(hit(419.0, 100.0, 5, 2, 8), None); // left of card
        assert_eq!(hit(781.0, 100.0, 5, 2, 8), None); // right of card
        // On the card edges is in-bounds.
        assert_eq!(hit(420.0, 100.0, 5, 2, 8), Some(2));
        assert_eq!(hit(780.0, 100.0, 5, 2, 8), Some(2));
    }

    #[test]
    fn empty_list_never_hits() {
        assert_eq!(hit(500.0, 100.0, 0, 0, 0), None);
    }

    #[test]
    fn spell_panel_rows_start_at_the_top_no_query_line() {
        // With header_rows=0 (the contextual spell panel), candidate row 0 begins at
        // `text_top` itself — one line higher than the query-line pickers. Row R spans
        // y in [text_top + R*lh, text_top + (R+1)*lh) → row 0 is [64, 88).
        assert_eq!(hit_spell(500.0, 64.0, 4, 0, 4), Some(0)); // top of row 0
        assert_eq!(hit_spell(500.0, 70.0, 4, 0, 4), Some(0)); // still row 0 (query line for the others)
        assert_eq!(hit_spell(500.0, 88.0, 4, 0, 4), Some(1)); // row 1
        assert_eq!(hit_spell(500.0, 63.0, 4, 0, 4), None); // above the panel text
    }

    #[test]
    fn a_visible_row_past_the_corpus_end_clamps_to_none() {
        // visible claims 5 rows but items only run 2..=4 (n=5) from top_idx=2; the 4th
        // visible row (y≥160) would be items index 5 ≥ n=5, so it hits nothing.
        assert_eq!(hit(500.0, 88.0, 5, 2, 5), Some(2)); // vis 0 → idx 2
        assert_eq!(hit(500.0, 150.0, 5, 2, 5), Some(4)); // vis 2 → idx 4 (last valid)
        assert_eq!(hit(500.0, 160.0, 5, 2, 5), None); // vis 3 → idx 5 ≥ 5
    }
}
