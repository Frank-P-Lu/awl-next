//! SUMMONED OVERLAY chrome — the centered navigation/command/theme takeover card
//! and the contextual spell popup: the row WINDOW geometry (the just-merged
//! overlay row->Y owner lives beside its consumers here — the selected-row band in
//! [`TextPipeline::overlay_draw_card`] and the pointer hit-test
//! [`TextPipeline::overlay_row_at`]), the spell-word anchoring, the card upload, and
//! the amber query caret. The text SHAPING half lives in [`super::overlay_shape`];
//! the faceted theme picker in [`super::theme_picker`]. Carved out of `chrome.rs`
//! verbatim, no behaviour change. See [`super`].

use super::*;

/// The summoned picker/overlay chrome renders at a UI size a step SMALLER than the
/// reading body (DESIGN §4 — the size ladder), so a picker reads as DENSE CHROME (a
/// scannable list), not prose, and MORE rows fit in the same card. ONE tunable:
/// dialing it re-flows the whole overlay through the single-owner
/// [`TextPipeline::overlay_metrics`] / [`TextPipeline::overlay_lh`] pair, so the card
/// height, the row-Y geometry ([`overlay_row_top`]), the hit-test ([`overlay_row_of`]),
/// and the selected-row band can NEVER disagree about a row's size. Non-overlay
/// rendering (the document, gutter, HUD, ornaments) is untouched.
pub(in crate::render) const OVERLAY_UI_SCALE: f32 = 0.85;

impl TextPipeline {
    /// The ONE metric every overlay ROW shapes + measures at: the reading body stepped
    /// down by [`OVERLAY_UI_SCALE`]. [`Self::overlay_remetric`] sets the shared buffers
    /// to it, and [`Self::overlay_lh`] (its line-height half) is what every geometry
    /// reader shares — so shaping and geometry can never drift on the row size.
    pub(in crate::render) fn overlay_metrics(&self) -> GlyphMetrics {
        let m = self.metrics;
        GlyphMetrics::new(m.font_size * OVERLAY_UI_SCALE, m.line_height * OVERLAY_UI_SCALE)
    }

    /// The overlay row LINE HEIGHT — the single-owner metric the card height, the
    /// row-Y ([`overlay_row_top`]), the hit-test ([`overlay_row_of`]), and the
    /// selected-row band all read, so a click always lands on the row it highlights.
    pub(in crate::render) fn overlay_lh(&self) -> f32 {
        self.metrics.line_height * OVERLAY_UI_SCALE
    }

    /// THE ONE OWNER of the summoned takeover card's LEFT edge — the
    /// PALETTE-COMPOSITION round's per-world anchor dial ([`theme::CardAnchor`],
    /// resolved through [`crate::render::effective_card_anchor`] so the gallery
    /// probe can A/B it). `TopLeft` (the global default this round) pins the card
    /// one `margin` in from the canvas edge (more anchored, right side opened for
    /// the ghost placard); `TopCenter` is the historical centered placement.
    /// Both flat [`Self::overlay_geometry`] and faceted
    /// [`TextPipeline::theme_overlay_geometry`] read this, so their card X can
    /// never disagree; the width, row geometry, and the placard's own
    /// canvas-corner anchor are untouched. The contextual spell popup does NOT
    /// call this (it anchors at its word).
    pub(in crate::render) fn overlay_card_x(&self, width: u32, card_w: f32, margin: f32) -> f32 {
        match crate::render::effective_card_anchor() {
            theme::CardAnchor::TopLeft => margin,
            theme::CardAnchor::TopCenter => (width as f32 - card_w) * 0.5,
            // FIRETAIL-MAXIMALIST-SHOWCASE round's statement dial: the left
            // edge sits `x_frac` of the free span in from the left margin —
            // 0.0 IS TopLeft, 1.0 pins the card's right edge one margin in,
            // 0.5 IS TopCenter (see `theme::CardAnchor::Inset`'s doc). The
            // free span floors at 0 (a card as wide as the window clamps to
            // the margin rather than reporting a negative origin).
            theme::CardAnchor::Inset { x_frac } => {
                let free = (width as f32 - card_w - 2.0 * margin).max(0.0);
                margin + x_frac.clamp(0.0, 1.0) * free
            }
        }
    }

    /// THE HEADER-GAP token (device px): the calm slab of negative space the
    /// PALETTE-COMPOSITION round inserts after the header rows (query + optional
    /// lens strip) and before the candidate list, on the palette AND every
    /// faceted picker uniformly (the divider is negative space, never a drawn
    /// rule). Sized off the overlay row height so it scales with zoom/DPI like
    /// every other overlay metric — ~0.55 of a row reads as a clear beat without
    /// re-opening the "fat lip" of a whole blank row. ONE tunable; both geometry
    /// owners read it, and the shaper inflates the last header line by exactly
    /// this. The contextual spell popup passes `0.0` (no header to divide from).
    pub(in crate::render) fn overlay_header_gap(&self) -> f32 {
        (self.overlay_lh() * 0.55).round()
    }

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
        // THE PLACARD RENDERER: shaped BEFORE the name/chord columns so its
        // upload (below) can be the FIRST `TextArea` — drawn behind the rows,
        // never over them (legibility first). `None` on every `InlinePrefix`
        // world — see `overlay_shape_placard`'s own doc.
        let placard = self.overlay_shape_placard(&geom);
        // THE STIPPLE PLACARD (`PlacardInk::Stipple` — Mangrove's assignment):
        // the SAME shaped wordmark renders as Bayer-stippled pixel runs
        // through the `placard_stipple` pipeline instead of an ordinary
        // antialiased text area — so the TextArea upload is withheld and the
        // coverage runs go to the quad pipeline (drawn in the same
        // behind-the-rows slot, `draw_overlay_card`). Every other ink keeps
        // the text path byte-identically, and the stipple pipeline parks.
        let stipple = matches!(
            crate::render::effective_title_style(),
            theme::TitleStyle::Placard { ink: theme::PlacardInk::Stipple, .. }
        );
        let (placard, stipple_rects) = match placard {
            // The (w, h) extent lives in the shaped buffer itself; the
            // rasterizer only needs the draw origin.
            Some((x, y, _w, _h)) if stipple => (None, self.placard_stipple_rects((x, y))),
            other => (other, Vec::new()),
        };
        self.placard_stipple
            .prepare(device, queue, width, height, &stipple_rects);
        // The SELECTED row's own glyphs recolor to a solid contrasting ink on a
        // true 1-bit world (`HighlightTreatment::InverseFill`) — black on the
        // white band — so they land crisp instead of the gamma-grey a
        // framebuffer invert of the antialiased row text produced (see that
        // variant's doc). `None` on every ordinary (`ValueBand`) world, where
        // the selected row keeps its content ink on the value band — the
        // shaper then stays byte-identical. Read from the SAME owner
        // (`highlight_treatment`) `overlay_draw_card` fills the band from.
        let selected_ink = match theme::active()
            .highlight_treatment(crate::render::effective_overlay_selrow_band())
        {
            theme::HighlightTreatment::InverseFill { ink, .. } => Some(ink.to_glyphon()),
            theme::HighlightTreatment::ValueBand(_) => None,
        };
        let has_right = self.overlay_shape_text(&geom, ink, muted, selected_ink);
        self.overlay_upload_text(
            device, queue, width, height, &geom, has_right, ink, muted, placard,
        )?;
        self.overlay_draw_card(device, queue, width, height, &geom);
        self.overlay_place_caret(queue, width, height, &geom);
        Ok(())
    }

    /// PARK every overlay pipeline empty for a frame with NO active overlay —
    /// the park-when-off discipline `prepare_hud` / `park_preview_text` already
    /// follow, applied to the summoned card. Without this the overlay TEXT
    /// renderer keeps its last-open glyph buffer (a whole palette of rows), and
    /// the frosted-blur backdrop path (`render`'s blur branch, taken whenever the
    /// HUD is held) calls `draw_overlay_card` UNCONDITIONALLY — so a closed
    /// palette's sharp rows ghost over the HUD's frost. Parking the renderer +
    /// its quads here makes that draw HARMLESS regardless of HUD state: the frame
    /// AFTER an overlay closes carries zero stale overlay pixels.
    ///
    /// Zeroes the flat card, its 1-bit elevation companions, the selected-row band,
    /// and the theme-lens underline quads (`instance_count` → 0), parks the amber
    /// query caret, and re-prepares the text renderer from an EMPTY off-screen
    /// buffer (nothing to draw). The float-panel quads (shared with the spell
    /// popup) are parked earlier this frame by `prepare_caret_preview_panel`, so
    /// they are not touched here.
    pub(in crate::render) fn park_overlay(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        // Quads: flat card, selected-row band, theme-lens underline → zero instances.
        self.panel_card.prepare(device, queue, width, height, &[]);
        self.panel_shadow.prepare(device, queue, width, height, &[]);
        self.panel_border.prepare(device, queue, width, height, &[]);
        self.overlay_rows.prepare(device, queue, width, height, &[]);
        self.overlay_lens_underline
            .prepare(device, queue, width, height, &[]);
        // The stipple placard: parked (zero instances) — the frame after a
        // stipple-world overlay closes carries zero stale wordmark pixels.
        self.placard_stipple
            .prepare(device, queue, width, height, &[]);
        // The amber query caret: parked (nothing drawn).
        self.panel_caret.prepare_empty();
        // The overlay TEXT renderer: shape an EMPTY buffer off-screen and prepare
        // the renderer from it, so its last-open glyph buffer can never linger and
        // draw. Mirrors `prepare_hud` / `park_preview_text` exactly.
        let m = self.metrics;
        let ink = theme::base_content().to_glyphon();
        self.panel_buffer
            .set_size(&mut self.font_system, Some(1.0), Some(m.line_height));
        self.panel_buffer.set_text(
            &mut self.font_system,
            "",
            &panel_attrs().color(ink),
            Shaping::Advanced,
            None,
        );
        self.panel_buffer
            .shape_until_scroll(&mut self.font_system, false);
        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let area = TextArea {
            buffer: &self.panel_buffer,
            left: 0.0,
            top: -1000.0,
            scale: 1.0,
            bounds,
            default_color: ink,
            custom_glyphs: &[],
        };
        self.panel_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon overlay park failed: {e:?}"))?;
        Ok(())
    }

    /// TEST HOOK: total shaped glyphs the overlay text renderer would draw this
    /// frame (summed across the name buffer's layout runs). `0` once
    /// [`Self::park_overlay`] has emptied it — the assertion that a closed
    /// overlay carries no stale palette glyphs into the next frame.
    #[cfg(test)]
    pub(in crate::render) fn overlay_text_glyph_count(&self) -> usize {
        self.panel_buffer
            .layout_runs()
            .map(|r| r.glyphs.len())
            .sum()
    }

    /// Re-metric BOTH shared overlay buffers to the current zoom so their glyph
    /// line-height matches the highlight/caret rects (which use m.line_height).
    /// Without this the buffer keeps its zoom-1.0 metrics and the selection
    /// highlight drifts one row off the text under zoom.
    ///
    /// The NAME buffer rides the overlay UI metrics ([`Self::overlay_metrics`] — a step
    /// below reading body so the picker reads as dense chrome, DESIGN §4); the right
    /// CHORD/time column rides the same UI LINE HEIGHT (so each chord stays on its
    /// name's row) but a smaller LABEL FONT SIZE on top — the type system's recessive
    /// rung (ink × size), so the secondary key-chord reads quieter than the name it
    /// annotates, not the same grey/size.
    fn overlay_remetric(&mut self) {
        let m = self.metrics;
        let name_metrics = self.overlay_metrics();
        let lh = self.overlay_lh();
        self.panel_buffer
            .set_metrics(&mut self.font_system, name_metrics);
        let label = crate::markdown::type_scale::LABEL;
        self.panel_bind_buffer.set_metrics(
            &mut self.font_system,
            GlyphMetrics::new(m.font_size * OVERLAY_UI_SCALE * label, lh),
        );
    }

    /// Resolve the overlay card's row WINDOW + rectangle + inner text origin. The
    /// list is capped at `MAX_ROWS` and scrolled so the selected row stays visible;
    /// the geometry is computed BEFORE the rows so the binding column can
    /// right-align to the text width.
    /// Resolve the overlay card geometry — the ONE shared source every reader (the
    /// render path AND the hit-tests `overlay_row_at` / `over_overlay_query` /
    /// `overlay_card_rect`) reads, so they can never disagree about where the card
    /// sits. A summoned overlay appears INSTANTLY at this settled position (no
    /// rise-in / sink-out offset).
    pub(in crate::render) fn overlay_geometry(&self, width: u32) -> OverlayGeom {
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

        // KEYBINDINGS TIPS FOOTER: the quiet "your top 3" band below the hint. The App
        // pushes `keybindings_tips` ONLY while the Keybindings overlay is open (empty for
        // every other flat picker, and in a capture), so a non-empty vec here IS the
        // keybindings-menu case — no kind check needed. `+ 1` reserves a blank separator
        // line between the hint and the band.
        let footer = self.keybindings_tips.clone();
        let footer_rows = if footer.is_empty() { 0 } else { footer.len() + 1 };

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
        // PALETTE-COMPOSITION round: a calm gap after the query header, before the
        // candidate list (negative space as the divider). Grows the card by exactly
        // this and offsets the candidate band/hit-test through `overlay_row_top`.
        let header_gap = self.overlay_header_gap();
        // query + rows/empty + hint + the keybindings tips footer (0 unless summoned).
        let total_rows = header_rows + visible + empty_rows + hint_rows + footer_rows;
        // RESPONSIVE CARD: prefer half the window, floored at a readable width, and
        // never wider than the window minus a calm margin — so a NARROW window gets
        // a card spanning nearly its full width (mirroring the responsive page
        // column) instead of a fixed-width card whose text column starves. At the
        // default 1200 canvas this is the same 600 as ever (wide captures are
        // byte-identical); the floor only lifts sub-1120 windows.
        let card_w = (width as f32 * 0.5).max(560.0).min(width as f32 - 2.0 * margin);
        let text_w = card_w - 2.0 * pad;
        // The header gap adds to the card height alongside the row stack + padding,
        // so the card still FITS its content exactly (bottom padding == `pad`).
        let card_h = total_rows as f32 * self.overlay_lh() + header_gap + 2.0 * pad;
        // Horizontal anchor via the ONE owner (top-left default this round, or
        // centered); vertical anchor near the top third (summoned, transient).
        let card_x = self.overlay_card_x(width, card_w, margin);
        // `self.menubar_reserve()` (`0.0` unless the WEB/LINUX MENU BAR is shown) —
        // the SAME accessor `doc_top`/the margin Outline/the search panel/the debug
        // panel already fold in, so the palette can never disagree with its siblings
        // about the bar's bottom edge (a shown bar draws LAST, `draw_chrome_tail`,
        // straight over an unyielding card's own top rows).
        // MOTION-JUICE ENTRANCE (live-only; exactly `+ 0.0` when settled, i.e.
        // in every capture and on every CALM world — see
        // `overlay_entrance_offset`'s doc): folded in AFTER all row-fit math,
        // so the transient drop can never change what the card shows, and
        // BEFORE `text_top`, so the card quad, rows, band, caret, and
        // hit-tests all ride the spring together through this ONE geometry.
        let card_y = margin + 40.0 + self.menubar_reserve() + self.overlay_entrance_offset();
        let text_left = card_x + pad;
        let text_top = card_y + pad;
        OverlayGeom {
            visible,
            top_idx,
            n_items,
            hint,
            hint_rows,
            footer,
            footer_rows,
            theme: false,
            strip: Vec::new(),
            plan: Vec::new(),
            header_rows,
            header_gap,
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
        let ui_metrics = self.overlay_metrics();
        self.panel_buffer
            .set_metrics(&mut self.font_system, ui_metrics);
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
        let card_h = rows as f32 * self.overlay_lh() + 2.0 * pad;

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
            footer: Vec::new(),
            footer_rows: 0,
            theme: false,
            strip: Vec::new(),
            plan: Vec::new(),
            header_rows,
            // The contextual popup has no header rows to divide from.
            header_gap: 0.0,
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
                geom.header_gap,
                self.overlay_lh(),
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
            self.overlay_lh(),
            geom.header_rows,
            geom.header_gap,
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
        let lh = self.overlay_lh();
        px >= geom.card_x
            && px <= geom.card_x + geom.card_w
            && py >= geom.text_top
            && py < geom.text_top + lh
    }

    /// Upload the shaped overlay text areas: the OPTIONAL placard wordmark FIRST
    /// (drawn behind everything else that follows in this same batch), then the
    /// name column at the panel origin, plus (when present) the right-aligned
    /// chord column whose own right edge lands at `text_left + text_w` = the
    /// card's right text edge → chords flush.
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
        placard: Option<(f32, f32, f32, f32)>,
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
        // The placard wordmark is FIRST in the batch (drawn behind everything
        // that follows), clipped to the WHOLE CANVAS — a screen-corner
        // watermark that bleeds OVER the scrim behind the card (never the
        // tighter card/text rect), per `overlay_shape_placard`'s doc. It is
        // still uploaded first, so the rows composite over it, and it rides
        // the text pass (above the scrim quad) so the dimmed document below
        // still shows through.
        let mut areas: Vec<TextArea> = Vec::new();
        if let Some((px, py, _pw, _ph)) = placard {
            let canvas_bounds = TextBounds {
                left: 0,
                top: 0,
                right: width as i32,
                bottom: height as i32,
            };
            areas.push(TextArea {
                buffer: &self.placard_buffer,
                left: px,
                top: py,
                scale: 1.0,
                bounds: canvas_bounds,
                default_color: ink,
                custom_glyphs: &[],
            });
        }
        // WILD-MENU SLANT PROBE (env-gated; `None` on every normal run, which
        // keeps the single verbatim `panel_area` push below — byte-identical):
        // the SAME shaped buffer uploads as one area per candidate DISPLAY
        // row, each clipped to its own row band and shifted right by its
        // stair offset — a pure DRAW-TIME row-origin transform (the shaping,
        // rowlayout elision, geometry, and hit-tests all read the settled
        // layout; the shaper already paid the width tax so a shifted row
        // still clips inside the card's right text edge). The header
        // (query/strip) and tail (hint/footer/empty) slices stay unshifted.
        let slant = crate::render::overlay_slant();
        match slant {
            None => {
                // The right-aligned label column shares the panel origin; its own right
                // edge lands at `text_left + text_w` = the card's right text edge →
                // chords flush.
                areas.push(panel_area);
            }
            Some(s) => {
                let lh = self.overlay_lh();
                let n_lines = if geom.theme { geom.plan.len() } else { geom.visible };
                let first_top =
                    overlay_row_top(geom.text_top, geom.header_rows, geom.header_gap, 0, lh);
                let clip = |top: f32, bottom: f32| TextBounds {
                    left: bounds.left,
                    top: top.max(0.0) as i32,
                    right: bounds.right,
                    bottom: (bottom.min(height as f32)) as i32,
                };
                // Header slice (query + strip lines), unshifted.
                areas.push(TextArea {
                    buffer: &self.panel_buffer,
                    left: text_left,
                    top: text_top,
                    scale: 1.0,
                    bounds: clip(0.0, first_top),
                    default_color: ink,
                    custom_glyphs: &[],
                });
                // One shifted slice per candidate display row.
                for k in 0..n_lines {
                    let row_top = first_top + k as f32 * lh;
                    areas.push(TextArea {
                        buffer: &self.panel_buffer,
                        left: text_left + crate::render::slant_offset(&s, k),
                        top: text_top,
                        scale: 1.0,
                        bounds: clip(row_top, row_top + lh),
                        default_color: ink,
                        custom_glyphs: &[],
                    });
                }
                // Tail slice (empty message / hint / footer), unshifted.
                let tail_top = first_top + n_lines as f32 * lh;
                areas.push(TextArea {
                    buffer: &self.panel_buffer,
                    left: text_left,
                    top: text_top,
                    scale: 1.0,
                    bounds: clip(tail_top, height as f32),
                    default_color: ink,
                    custom_glyphs: &[],
                });
            }
        }
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
    /// The card is drawn one of two ways. The contextual SPELL panel rides the
    /// reusable FLOATING-PANEL primitive ([`Self::prepare_float_panel`]) — shadow +
    /// raised border + card, unconditionally — so it reads as risen a step above the
    /// crisp document with NO scrim (DESIGN §5/§8); `panel_card` is left empty then.
    /// Every OTHER (CENTERED) overlay — go-to / command / theme / keybindings /
    /// settings / … — uses `panel_card` through
    /// [`Self::prepare_panel_card_elevation`]: the flat opaque fill on every
    /// ordinary world (BYTE-IDENTICAL to the old bare `panel_card.prepare` call —
    /// the blur/scrim backdrop behind it already carries the card's contrast there),
    /// PLUS a crisp white `panel_border` on a true 1-bit world, where that backdrop
    /// is disabled outright (`backdrop_blur`'s one-bit short-circuit) and the card
    /// would otherwise be an invisible black rect on black — the SAME elevation
    /// mechanism the menu-bar dropdown / HUD / which-key / spell popup already
    /// carry, closing the gap for this last summoned-card family.
    fn overlay_draw_card(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        geom: &OverlayGeom,
    ) {
        let lh = self.overlay_lh();
        let card_rect = [geom.card_x, geom.card_y, geom.card_w, geom.card_h];
        if self.overlay_spell.is_some() {
            // Contextual spell panel: elevate on the float primitive, no flat card.
            self.prepare_float_panel(device, queue, width, height, Some(card_rect));
            self.panel_card.prepare(device, queue, width, height, &[]);
            self.panel_shadow.prepare(device, queue, width, height, &[]);
            self.panel_border.prepare(device, queue, width, height, &[]);
        } else {
            // Centered overlay: the flat opaque card, ELEVATED (bordered) only on a
            // true 1-bit world — see `prepare_panel_card_elevation`'s doc.
            self.prepare_panel_card_elevation(device, queue, width, height, Some(card_rect));
        }

        // Selected-row highlight: a VALUE BAND, the next rung up the surface ladder
        // past the card's `base_300` (`theme::surface_selected`), set per-frame so a
        // live theme switch reskins it. Figure/ground by VALUE — not the cool
        // `selection` hue, not the amber accent (DESIGN §3/§5). The selected name
        // stays content ink, readable on the band. The band sits `header_rows` lines
        // below the card top (past the query line, if any), matching the shaped rows.
        //
        // TRUE 1-BIT WORLDS (`render_caps.selection_style ==
        // SelectionStyle::InverseVideo`): a flat fill would need SOME token
        // between `base_300`/`base_content` (both pure black/white here) to read
        // as "selected without erasing the row's own text" — no such token
        // exists on a one-bit world. The answer is a SOLID `base_content`
        // (white) band with the selected row's own glyphs recolored to solid
        // `base_300` (black) up in the shaper (`selected_ink`, threaded through
        // `overlay_shape_text`) — a hard black-on-white pair, gamma-independent
        // and CRISP. This supersedes the earlier framebuffer invert of the row
        // (`overlay_rows_invert`, retired): a `1 - dst` flip of the antialiased
        // near-white row text landed at a faint mid-grey (the Wagtail
        // selected-row low-contrast bug — see `HighlightTreatment::InverseFill`).
        // Both regimes now drive the ONE `overlay_rows` fill pipeline; the band
        // COLOR is the only thing that differs, so "prepare neither / draw text
        // that can't be read" is unreachable.
        // The `ValueBand` band VALUE is the PALETTE-COMPOSITION round's
        // strengthened, calm-by-VALUE band (`effective_overlay_selrow_band`, one
        // ramp step past the shared `surface_selected`; the gallery A/Bs it and
        // the old band is one line away — see that fn's REVERT note). Never a hue
        // (DESIGN §3/§5); the distinguishability sweep polices it.
        let band_color = match theme::active()
            .highlight_treatment(crate::render::effective_overlay_selrow_band())
        {
            theme::HighlightTreatment::ValueBand(color) => color,
            theme::HighlightTreatment::InverseFill { band, .. } => band,
        };
        self.overlay_rows.set_color(band_color.rgba_bytes());
        // The selected row's DISPLAY index + settled row-top, per layout family.
        let sel_disp: Option<usize> = if geom.n_items == 0 {
            None
        } else if geom.theme {
            // THEME PICKER: the selected world's DISPLAY row = its position in the plan
            // (headers push it down), offset past the query + strip lines (`header_rows`).
            Some(
                geom.plan
                    .iter()
                    .position(|l| matches!(l, ThemeLine::Item(i) if *i == self.overlay_selected))
                    .unwrap_or(0),
            )
        } else {
            // 0-based row among the visible window. `OverlayState` keeps the selection
            // inside `[top_idx, top_idx+visible)`; saturate + clamp defensively so a
            // transient mismatch (e.g. the list just shrank) can never underflow/overflow.
            Some(
                self.overlay_selected
                    .saturating_sub(geom.top_idx)
                    .min(geom.visible.saturating_sub(1)),
            )
        };
        let sel_rects: Vec<[f32; 4]> = match sel_disp {
            None => Vec::new(),
            Some(disp) => {
                let target =
                    overlay_row_top(geom.text_top, geom.header_rows, geom.header_gap, disp, lh);
                // MOTION-JUICE BAND SLIDE (live-only; returns `target` verbatim
                // on every Snap world / capture / Reduce Motion — see
                // `overlay_band_drawn`'s doc). Purely visual: the shaped rows
                // and the hit-test read the settled geometry.
                let row_top = self.overlay_band_drawn(target);
                // WILD-MENU SLANT PROBE (env-gated, `None` on every normal
                // run): the band's left edge follows the selected row's own
                // stair offset so the highlight hugs its slanted row.
                let dx = crate::render::overlay_slant()
                    .map(|s| crate::render::slant_offset(&s, disp))
                    .unwrap_or(0.0);
                vec![[geom.card_x + dx, row_top, geom.card_w - dx, lh]]
            }
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
        // The query caret rides the UI row: scaled a hair short of the smaller row
        // height, centered on the query line's own (UI-height) band.
        let caret_h = m.caret_h * 0.8 * OVERLAY_UI_SCALE;
        let caret_cx = caret_x + m.caret_w * 0.5;
        let caret_cy = geom.text_top + self.overlay_lh() * 0.5;
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
