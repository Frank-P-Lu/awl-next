//! CHROME RENDER — the summoned/quiet UI furniture composited OVER the document:
//! the top-right search/replace panel, the centered navigation overlay (go-to /
//! command palette), and the three single-line CORNER readouts (the bottom-left
//! project status strip, the bottom-right markdown word-count, and the opt-in
//! top-left DEBUG frame counter).
//!
//! These are all inherent methods on [`super::TextPipeline`]: they shape into its
//! shared panel / status / wordcount / fps glyph buffers and `prepare` them through
//! its glyphon renderers, atlas, viewport, font-system and swash-cache — the GPU
//! aggregation that is `TextPipeline`'s whole reason for being — so they CANNOT
//! become `&self`-free free functions the way the span/attrs helpers in `render.rs`
//! could. This module is purely a physical home for that cohesive chrome cluster,
//! carved out of `render.rs` verbatim. Because a child module sees its ancestor's
//! private items, the methods keep their full access to `TextPipeline`'s private
//! fields and helpers with NO behaviour change — the chrome pixels are byte-identical.
//!
//! The three corner readouts already share ONE body, [`TextPipeline::prepare_corner_label`]:
//! `prepare_status` / `prepare_wordcount` / `prepare_fps` were ~95%-identical copies
//! differing only by the (renderer, buffer) pair, the text, and the [`CornerAnchor`],
//! so they each reduce to resolving their own text + column geometry and delegating
//! to that shared helper. The readout text-feeders (`word_count`, `readout_report`,
//! `wordcount_text`, `set_fps_frame_ms`, `fps_text`) ride along with their readouts.

use super::*;

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
struct OverlayGeom {
    visible: usize,
    top_idx: usize,
    n_items: usize,
    hint: String,
    hint_rows: usize,
    card_x: f32,
    card_y: f32,
    card_w: f32,
    card_h: f32,
    text_left: f32,
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

    /// Compose + shape the search/replace field text into `panel_buffer`, returning
    /// the colors the card draws with and the FOCUSED field's reserved-caret-cell
    /// offsets. The amber caret rides a RESERVED cell shaped right after the focused
    /// field so its x comes from the SAME layout as the text (no hardcoded-pitch drift).
    fn panel_shape_text(&mut self, width: u32) -> PanelShape {
        let m = self.metrics;
        // Per-run colors give the panel a calm visual hierarchy: a muted "/" sigil
        // and hit counter, full-ink query, and an "Aa" toggle that brightens from
        // muted to full ink when case-sensitivity is ON (so the toggle shows its
        // state without using amber — the only amber anywhere is the caret quad).
        // On the no-match state the whole field tints ERROR red.
        let no_match = self.search_no_matches();
        let ink = theme::base_content().to_glyphon();
        let muted = theme::muted().to_glyphon();
        let red = theme::error().to_glyphon();
        let total = self.search_matches.len();
        let n = self.search_current.map(|i| i + 1).unwrap_or(0);
        let query = self.search_query.clone();
        // The amber caret block rides in a RESERVED cell shaped right after the
        // query (the `gap` span below). The counter then starts a clear two cells
        // later, so the block can never collide with the `N/M` digits at any query
        // length. Keeping the reserved cell IN the shaped string means the caret x
        // and the counter x come from the SAME monospace layout — no drift between
        // a hardcoded CHAR_WIDTH caret and glyphon's shaped text (the old overlap
        // bug). One reserved caret cell + two clear cells, then the counter.
        let gap = "   "; // [caret cell][clear][clear]
        let counter = format!("{n}/{total}   ");
        // (sigil, query, counter, toggle) colors. The reserved gap is invisible
        // (spaces) so its color is irrelevant; reuse the counter color.
        let (c_sigil, c_query, c_counter, c_toggle) = if no_match {
            (red, red, red, red)
        } else if self.search_case_sensitive {
            (muted, ink, muted, ink) // case ON -> "Aa" full ink
        } else {
            (muted, ink, muted, muted) // case OFF -> "Aa" muted
        };
        // Active-world face (mono is the automatic glyph fallback); the search
        // caret reads its x from the SHAPED buffer so it tracks real advances.
        let base = panel_attrs();
        let mk = |c| base.clone().color(c);
        // Row 0 = the search field (sigil, query, reserved caret cell, counter,
        // "Aa" toggle). When REPLACE is active a second row holds the replacement
        // field on the SAME card — the find-and-replace mode of the one warm panel,
        // never separate chrome (DESIGN §5). The amber caret rides whichever field
        // has focus; the other field keeps its calm ink.
        const REPLACE_SIGIL: &str = "\u{00bb} "; // "» " — the replace affordance
        let replacement = self.search_replacement.clone();
        let replace_active = self.search_replace_active;
        let editing_replacement = replace_active && self.search_editing_replacement;
        let mut spans: Vec<(&str, Attrs)> = vec![
            ("/ ", mk(c_sigil)),
            (query.as_str(), mk(c_query)),
            (gap, mk(c_counter)),
            (counter.as_str(), mk(c_counter)),
            ("Aa", mk(c_toggle)),
        ];
        if replace_active {
            spans.push(("\n", mk(muted)));
            spans.push((REPLACE_SIGIL, mk(muted)));
            spans.push((replacement.as_str(), mk(ink)));
            spans.push((" ", mk(ink))); // reserved caret cell on the replace row
        }
        let lines = if replace_active { 2.0 } else { 1.0 };
        // Give the buffer generous width + one line height per row so it never wraps.
        self.panel_buffer.set_size(
            &mut self.font_system,
            Some(width as f32 * 2.0),
            Some(m.line_height * lines),
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
            let line0_len = "/ ".len() + query.len() + gap.len() + counter.len() + "Aa".len();
            (
                line0_len + "\n".len() + REPLACE_SIGIL.len() + replacement.len(),
                REPLACE_SIGIL.chars().count() + replacement.chars().count(),
                1.0_f32,
            )
        } else {
            (
                "/ ".len() + query.len(),
                "/ ".chars().count() + query.chars().count(),
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

        // Opaque card behind the panel text.
        self.panel_card
            .prepare(device, queue, width, height, &[card_rect]);
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
    /// candidate list (selected row highlighted with the muted selection token),
    /// all composited OVER the document. Reuses the panel card / caret / text
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
    fn overlay_remetric(&mut self) {
        let m = self.metrics;
        self.panel_buffer
            .set_metrics(&mut self.font_system, m.glyph_metrics());
        self.panel_bind_buffer
            .set_metrics(&mut self.font_system, m.glyph_metrics());
    }

    /// Resolve the overlay card's row WINDOW + rectangle + inner text origin. The
    /// list is capped at `MAX_ROWS` and scrolled so the selected row stays visible;
    /// the geometry is computed BEFORE the rows so the binding column can
    /// right-align to the text width.
    fn overlay_geometry(&self, width: u32) -> OverlayGeom {
        let m = self.metrics;
        let pad = 12.0;
        let margin = 12.0;
        // Cap how many rows we show so the card stays bounded; the selected row is
        // kept in view by a simple window starting at a scroll offset.
        const MAX_ROWS: usize = 12;
        let n_items = self.overlay_items.len();
        let visible = n_items.min(MAX_ROWS);
        // Scroll the list so the selected row is visible.
        let top_idx = if self.overlay_selected >= MAX_ROWS {
            self.overlay_selected + 1 - MAX_ROWS
        } else {
            0
        };

        // A faint, per-kind control-hint line drawn at the FOOT of the card so the
        // select-vs-descend model is discoverable (see `OverlayKind::hint`). Drawn
        // in the dim token; its own row, kept off the candidate list. Empty = none.
        let hint = self.overlay_hint.clone();
        let hint_rows = if hint.is_empty() { 0 } else { 1 };

        // Card / text-column geometry. Computed here (before the rows) so the
        // command-palette binding column can right-align to the text width.
        let total_rows = 1 + visible + hint_rows; // query line + candidates + hint
        let card_w = (width as f32 * 0.5).max(360.0).min(width as f32 - 2.0 * margin);
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
            card_x,
            card_y,
            card_w,
            card_h,
            text_left,
            text_top,
            text_w,
        }
    }

    /// Compose + shape the overlay text into the shared buffers: the query line +
    /// candidate rows (selected ink / rest muted) in `panel_buffer`, and the dim
    /// `Align::Right` chord/time column in `panel_bind_buffer`. Returns whether a
    /// right column was built (so the caller uploads its text area).
    fn overlay_shape_text(
        &mut self,
        geom: &OverlayGeom,
        ink: glyphon::Color,
        muted: glyphon::Color,
    ) -> bool {
        let visible = geom.visible;
        let top_idx = geom.top_idx;
        let text_w = geom.text_w;
        let card_h = geom.card_h;
        let hint_rows = geom.hint_rows;
        let hint = &geom.hint;

        // Compose the multi-line panel text: query line, then candidate rows.
        let sigil = "› ";
        let mut composed = String::new();
        composed.push_str(sigil);
        composed.push_str(&self.overlay_query);
        for row in 0..visible {
            composed.push('\n');
            composed.push_str(&self.overlay_items[top_idx + row]);
        }
        // Per-row colors: query full ink; candidate rows ink (selected) / muted.
        // Names/query/sigil render in the ACTIVE-WORLD face (`mk`); the dim
        // right-aligned chord/label column stays MONOSPACE (`mono`).
        let base = panel_attrs();
        let mk = |c| base.clone().color(c);
        let mono = |c| Attrs::new().family(Family::Monospace).color(c);
        let mut spans: Vec<(&str, glyphon::Attrs)> = Vec::new();
        spans.push((sigil, mk(muted)));
        spans.push((self.overlay_query.as_str(), mk(ink)));
        // The dim RIGHT-aligned column: command-palette key chords (`bindings`) OR
        // the go-to picker's relative "last edited" labels (`times`). Only one is
        // ever populated, so prefer bindings when present, else fall back to times.
        // It is drawn FLUSH at the card's right text edge by a SEPARATE buffer laid
        // out with cosmic-text `Align::Right` (built below), so the chord column is a
        // clean right edge regardless of the proportional name width — no char-count
        // space padding (which went ragged on a proportional face).
        let right_labels: &[String] = if !self.overlay_bindings.is_empty() {
            &self.overlay_bindings
        } else {
            &self.overlay_times
        };
        let has_right = !right_labels.is_empty();
        // The NAME column: each candidate's name on its own line, no padding. The
        // matching right-edge chord/time rides the separate right-aligned buffer.
        let mut row_name_strs: Vec<String> = Vec::with_capacity(visible);
        for row in 0..visible {
            let idx = top_idx + row;
            row_name_strs.push(format!("\n{}", self.overlay_items[idx]));
        }
        for row in 0..visible {
            let selected = top_idx + row == self.overlay_selected;
            spans.push((row_name_strs[row].as_str(), mk(if selected { ink } else { muted })));
        }
        // The quiet control-hint row, last, always in the DIM token. Carries its own
        // leading newline so it sits one line below the final candidate.
        let hint_line = if hint.is_empty() {
            String::new()
        } else {
            format!("\n{hint}")
        };
        if hint_rows > 0 {
            spans.push((hint_line.as_str(), mk(muted)));
        }

        self.panel_buffer
            .set_size(&mut self.font_system, Some(text_w), Some(card_h));
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

        // RIGHT COLUMN: build the separate `Align::Right` chord/time buffer, one line
        // per name row so each label sits on its name's row, flush at the card's
        // right text edge (width == `text_w`). A `\n`-prefixed label leaves line 0
        // (the query row) empty and puts label N on candidate row N; the hint row
        // (if any) stays empty. Only built/drawn when a right column exists.
        let mut bind_strs: Vec<String> = Vec::with_capacity(visible);
        if has_right {
            for row in 0..visible {
                let idx = top_idx + row;
                let label = right_labels.get(idx).map(|s| s.as_str()).unwrap_or("");
                bind_strs.push(format!("\n{label}"));
            }
            let bind_spans: Vec<(&str, glyphon::Attrs)> =
                bind_strs.iter().map(|s| (s.as_str(), mono(muted))).collect();
            self.panel_bind_buffer
                .set_size(&mut self.font_system, Some(text_w), Some(card_h));
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
        has_right
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

    /// Upload the opaque card behind everything + the muted selected-row highlight
    /// quad positioned over the chosen candidate.
    fn overlay_draw_card(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        geom: &OverlayGeom,
    ) {
        let m = self.metrics;
        // Opaque card behind everything.
        self.panel_card.prepare(
            device,
            queue,
            width,
            height,
            &[[geom.card_x, geom.card_y, geom.card_w, geom.card_h]],
        );

        // Selected-row highlight (muted), positioned over the chosen candidate.
        let sel_rects: Vec<[f32; 4]> = if geom.n_items > 0 {
            let sel_row = self.overlay_selected - geom.top_idx; // 0-based among visible
            let row_top = geom.text_top + (1 + sel_row) as f32 * m.line_height;
            vec![[geom.card_x, row_top, geom.card_w, m.line_height]]
        } else {
            Vec::new()
        };
        self.overlay_rows
            .prepare(device, queue, width, height, &sel_rects);
    }

    /// Place the one amber caret: a resting block at the end of the query line. Read
    /// the first shaped row's width so the caret lands at the query end on a
    /// proportional world face too (not a fixed `char_width` assumption); fall back
    /// to fixed-pitch if shaping yielded no run.
    fn overlay_place_caret(
        &mut self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        geom: &OverlayGeom,
    ) {
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

    /// Shape one quiet single-line corner label into `buffer` and `prepare` it into
    /// `renderer`, parking it off-screen when `text` is empty. This is the shared
    /// body behind the bottom-left status strip, the bottom-right word-count readout,
    /// and the top-left FPS counter — each was a ~95%-identical copy differing only
    /// by the (renderer, buffer) pair, the text, and the corner [`CornerAnchor`].
    ///
    /// It takes `renderer` + `buffer` (and the four shared glyphon resources) as
    /// EXPLICIT `&mut` params rather than `&mut self`: the three callers pass
    /// distinct fields, so a `&mut self` method couldn't also hand it `&mut
    /// self.status_renderer`. `col_left` / `col_width` are the writing column's
    /// already-resolved geometry (so this stays free of `self`); `col_width` is only
    /// consulted for the right-aligned anchor.
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
        line_height: f32,
        col_left: f32,
        col_width: f32,
        text: &str,
        anchor: CornerAnchor,
        label: &str,
    ) -> anyhow::Result<()> {
        let muted = theme::muted().to_glyphon();
        buffer.set_size(font_system, Some(width as f32), Some(line_height));
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
                CornerAnchor::BottomLeft => (col_left, height as f32 - line_height - 8.0),
                CornerAnchor::BottomRight => {
                    let mut text_w = 0.0_f32;
                    for run in buffer.layout_runs() {
                        text_w = text_w.max(run.line_w);
                    }
                    let left = (col_left + col_width - text_w).max(col_left);
                    (left, height as f32 - line_height - 8.0)
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

    /// Shape + upload the quiet bottom status strip ("name · branch · ●"). Drawn
    /// in the muted token (theme.muted); the dirty marker is a DIM filled
    /// dot appended to the value, value-only — never accent-colored (amber is the
    /// caret's alone). Empty `project_status` uploads nothing.
    pub(super) fn prepare_status(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let mut text = self.project_status.clone();
        if !text.is_empty() && self.project_dirty {
            // A dim filled dot, value-only (NOT accent). Spaced for breathing room.
            text.push_str(" · ●");
        }
        let (lh, col_left) = (self.metrics.line_height, self.column_left());
        Self::prepare_corner_label(
            &mut self.status_renderer,
            &mut self.status_buffer,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            &mut self.swash_cache,
            device,
            queue,
            width,
            height,
            lh,
            col_left,
            0.0,
            &text,
            CornerAnchor::BottomLeft,
            "status",
        )
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
    /// RIGHT-aligned to the writing column's right edge, on the same bottom row as
    /// the status strip. Empty text parks it off-screen (markdown gate / empty doc),
    /// so a non-markdown buffer draws nothing and stays byte-identical.
    pub(super) fn prepare_wordcount(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let text = self.wordcount_text();
        let (lh, col_left, col_width) =
            (self.metrics.line_height, self.column_left(), self.column_width());
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
            lh,
            col_left,
            col_width,
            &text,
            CornerAnchor::BottomRight,
            "wordcount",
        )
    }

    /// Feed the latest measured frame time (ms) into the DEBUG counter. The live
    /// loop calls this each redraw while the counter is on; `None` clears it (no
    /// clock / counter off), which renders the fixed placeholder. No-op on the
    /// headless path, where the counter is never fed (so it stays clockless).
    pub fn set_fps_frame_ms(&mut self, ms: Option<f32>) {
        self.fps_frame_ms = ms;
    }

    /// The DEBUG frame-counter STRING for the top-left corner, e.g.
    /// `"60 fps · 16.7 ms"` live or the fixed placeholder `"fps · — ms"` with no
    /// clock. EMPTY when the counter is off, which parks it off-screen so a default
    /// capture stays byte-identical. Exposed so the sidecar can report it verbatim.
    pub fn fps_text(&self) -> String {
        if !crate::fps::fps_on() {
            return String::new();
        }
        crate::fps::readout(self.fps_frame_ms)
    }

    /// Shape + upload the opt-in DEBUG frame counter. Drawn DIM in the TOP-LEFT
    /// corner (the value-only, no-amber convention shared with the word-count
    /// readout). Empty text (counter off) parks it off-screen, so a default capture
    /// draws nothing and stays byte-identical.
    pub(super) fn prepare_fps(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let text = self.fps_text();
        let (lh, col_left) = (self.metrics.line_height, self.column_left());
        Self::prepare_corner_label(
            &mut self.fps_renderer,
            &mut self.fps_buffer,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            &mut self.swash_cache,
            device,
            queue,
            width,
            height,
            lh,
            col_left,
            0.0,
            &text,
            CornerAnchor::TopLeft,
            "fps",
        )
    }
}
