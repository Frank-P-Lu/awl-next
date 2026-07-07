//! SUMMONED OVERLAY chrome — the centered navigation/command/theme takeover card
//! and the contextual spell popup: the row WINDOW geometry (the just-merged
//! overlay row->Y owner lives beside its consumers here — the selected-row band in
//! [`TextPipeline::overlay_draw_card`] and the pointer hit-test
//! [`TextPipeline::overlay_row_at`]), the spell-word anchoring, the card upload, and
//! the amber query caret. The text SHAPING half lives in [`super::overlay_shape`];
//! the faceted theme picker in [`super::theme_picker`]. Carved out of `chrome.rs`
//! verbatim, no behaviour change. See [`super`].

use super::*;

impl TextPipeline {
    /// Shape + upload the SUMMONED navigation overlay for this frame: a tall
    /// BASE_300 card, a query line (with the one amber caret at its end), the
    /// candidate list (selected row highlighted with a surface VALUE band), all
    /// composited OVER the document. Reuses the panel card / caret / text
    /// renderer; the row highlight reuses the selection-quad pipeline. This is the
    /// functional-first card look — the organic visuals come later.
    pub(in crate::render) fn prepare_overlay(
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
    /// Resolve the overlay card geometry AND fold in the live SUMMON/DISMISS rise
    /// offset (see [`crate::overlay_motion`]) at the ONE shared source, so every
    /// reader — the render path AND the hit-tests (`overlay_row_at` /
    /// `over_overlay_query` / `overlay_card_rect`) — sees the card at the SAME risen
    /// position and can never disagree. `card_y`/`text_top` (the two origins every
    /// other position derives from) shift together, so the whole card + rows + text +
    /// caret + swatches + lens move as one. At rest the offset is a hard `0.0`
    /// (`overlay_motion` is the settled default in every capture), so a `--screenshot`
    /// is byte-identical.
    pub(in crate::render) fn overlay_geometry(&self, width: u32) -> OverlayGeom {
        let mut geom = self.overlay_geometry_inner(width);
        let rise = self.overlay_motion.rise_px();
        if rise != 0.0 {
            geom.card_y += rise;
            geom.text_top += rise;
        }
        geom
    }

    fn overlay_geometry_inner(&self, width: u32) -> OverlayGeom {
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
        let n_items = self.overlay_items.len();
        // The scroll window rides the ONE shared `scroll_window` owner (also used by the
        // spell popup and the faceted/grouped path); the CAP is the per-kind
        // `overlay_window_rows` (12 for the flat pickers — the former inline `MAX_ROWS`),
        // and the WINDOW POSITION is owned by `OverlayState::scroll` (which keeps the
        // selection visible on keyboard nav, holds still on hover, and advances on the
        // wheel), passed as the hint. For a flat list the hint already keeps
        // `overlay_selected` in view, so the slide is inert and `(top_idx, visible)` are
        // byte-identical to the previous inline `min` math.
        let (top_idx, visible) = scroll_window(
            n_items,
            self.overlay_selected,
            self.overlay_scroll,
            self.overlay_window_rows.max(1),
        );

        // A faint, per-kind control-hint line drawn at the FOOT of the card so the
        // select-vs-descend model is discoverable (see `OverlayKind::hint`). Drawn
        // in the dim token; its own row, kept off the candidate list. Empty = none.
        let hint = self.overlay_hint.clone();
        let hint_rows = if hint.is_empty() { 0 } else { 1 };

        // EMPTY STATE: no candidate rows (empty corpus / query matched nothing) → the
        // shared dim message row occupies ONE candidate line (grows the card by one).
        let empty = if n_items == 0 {
            self.overlay_empty.clone()
        } else {
            None
        };
        let empty_rows = empty.is_some() as usize;

        // Card / text-column geometry. Computed here (before the rows) so the
        // command-palette binding column can right-align to the text width. The
        // CARET-STYLE PICKER's live preview now rides its OWN floating panel BELOW this
        // card (see `prepare_caret_preview_panel`), so the list itself stays exactly as
        // familiar — no reserved preview strip carved out of the card.
        let header_rows = 1; // the `› query` line every flat/nav picker shows on top
        let total_rows = header_rows + visible + empty_rows + hint_rows; // query + rows/empty + hint
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
            empty,
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
    pub(in crate::render) fn measure_spell_content_w(&mut self) -> f32 {
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
        let n_items = self.overlay_items.len();
        // Same window model as the centered card via the shared `scroll_window` owner,
        // capped by the spell popup's own `overlay_window_rows` (8 — the former inline
        // `MAX_ROWS`; byte-identical, since the overlay-owned scroll hint already keeps
        // `sel` visible).
        let (top_idx, visible) = scroll_window(
            n_items,
            self.overlay_selected,
            self.overlay_scroll,
            self.overlay_window_rows.max(1),
        );
        // A contextual popup: no query row, no foot hint — just the corrections.
        let header_rows = 0;
        let hint = String::new();
        let hint_rows = 0;
        // EMPTY STATE: a flagged word with NO suggestions shows the shared calm
        // "no suggestions" message row (in the one row the popup already reserves
        // below via `visible.max(1)`), rather than a blank sliver.
        let empty = if n_items == 0 {
            self.overlay_empty.clone()
        } else {
            None
        };

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
            empty,
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

    /// The SUMMONED overlay's drawn scroll-WINDOW for the sidecar, or `None` when no
    /// overlay is open: `(top, lines, sel_row, card_h, canvas_h)` — the first candidate
    /// ITEM shown (`top`), the number of candidate DISPLAY LINES actually drawn (`lines`:
    /// headers + rows for the grouped/faceted path, rows for the flat path), the 0-based
    /// position of the SELECTED row AMONG those drawn candidate lines (`sel_row`), and the
    /// card / canvas heights. Lets a headless test assert the card is BOUNDED (`card_h ≤
    /// canvas_h`) and the selection stays visible (`sel_row < lines`) — the two guarantees
    /// the windowing exists to keep. Reads the SAME [`Self::overlay_geometry`] the card
    /// renders from, so the report can never claim a window the pixels don't show.
    pub fn overlay_window_report(&self) -> Option<(usize, usize, usize, f32, f32)> {
        if !self.overlay_active {
            return None;
        }
        let geom = self.overlay_geometry(self.window_w as u32);
        let canvas_h = self.window_h;
        if geom.theme {
            // Grouped/faceted: `geom.plan` is the WINDOWED display slice (headers + item
            // rows); `top_idx` is the first ITEM shown. `sel_row` is the selected item's
            // display position within that slice — present by construction, since the
            // window slides to keep it visible.
            let sel_row = geom
                .plan
                .iter()
                .position(|l| matches!(l, ThemeLine::Item(i) if *i == self.overlay_selected))
                .unwrap_or(0);
            Some((geom.top_idx, geom.plan.len(), sel_row, geom.card_h, canvas_h))
        } else {
            // Flat: `visible` rows from item `top_idx`; the selected row's 0-based position
            // among them (clamped defensively, mirroring the selected-band math).
            let sel_row = self
                .overlay_selected
                .saturating_sub(geom.top_idx)
                .min(geom.visible.saturating_sub(1));
            Some((geom.top_idx, geom.visible, sel_row, geom.card_h, canvas_h))
        }
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
            // Below the query + lens-strip header lines, the candidate area is a plain
            // stack of DISPLAY rows (headers interleaved with world rows); the SAME
            // inverse the flat pickers use maps the pointer to a row `k`, which we then
            // read out of the plan (a header row → None, a world row → its world index).
            let k = overlay_row_of(
                geom.text_top,
                geom.header_rows,
                self.metrics.line_height,
                py,
            )?;
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

    /// Hit-test a pointer at PHYSICAL `(px, py)` against the SUMMONED overlay's
    /// editable QUERY-INPUT line — the `› query` filter field every flat/nav/theme
    /// picker draws on top (`header_rows == 1`). Returns `true` when the pointer
    /// sits on that one row, within the card's x-bounds. The contextual SPELL
    /// panel has NO query line (`header_rows == 0`), so it always returns `false`.
    /// Reads the SAME [`Self::overlay_geometry`] the query line renders from (its
    /// row is `text_top .. text_top + line_height`, the row just above the
    /// candidate window), so this can never disagree with where the field draws.
    /// Used by `input.rs::sync_cursor_icon` to give the field the I-beam.
    pub fn over_overlay_query(&self, px: f32, py: f32) -> bool {
        if !self.overlay_active {
            return false;
        }
        let geom = self.overlay_geometry(self.window_w as u32);
        if geom.header_rows == 0 {
            return false;
        }
        let lh = self.metrics.line_height;
        px >= geom.card_x
            && px <= geom.card_x + geom.card_w
            && py >= geom.text_top
            && py < geom.text_top + lh
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
            let row_top = overlay_row_top(geom.text_top, geom.header_rows, disp, m.line_height);
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
                overlay_row_top(geom.text_top, geom.header_rows, sel_row, m.line_height);
            vec![[geom.card_x, row_top, geom.card_w, m.line_height]]
        };
        self.overlay_rows
            .prepare(device, queue, width, height, &sel_rects);
        // THEME PICKER per-row palette SWATCHES: each world's ground band + accent dot,
        // in its OWN colours (per-quad `prepare_colored`). Empty for every other card, so
        // the swatch pipeline parks (a non-theme overlay draws byte-identically). Computed
        // into a local first so the immutable borrow drops before the mutable prepare.
        let swatches = self.theme_swatch_quads(geom);
        self.overlay_swatches
            .prepare_colored(device, queue, width, height, &swatches);
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
}
