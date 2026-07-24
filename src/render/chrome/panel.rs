//! SEARCH PANEL chrome — the summoned top-right find/replace card: its opaque
//! elevated card, the labeled find/replace rows, the teaching key-hint line, and
//! the amber query caret riding the shaped advance. Inherent methods on
//! [`super::TextPipeline`] (they shape into its shared panel buffers); carved out
//! of `chrome.rs` verbatim, no behaviour change. See [`super`].

use super::*;

impl TextPipeline {
    /// Shape + upload the top-right search panel for this frame: the opaque
    /// BASE_300 card, the panel text (calm BASE_CONTENT, or ERROR-red on the
    /// no-match state), and the amber caret block at the query end. Called from
    /// `prepare()` only when `search_active`.
    pub(in crate::render) fn prepare_panel(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        self.panel_remetric();
        let shape = self.panel_shape_text(width);
        let (card_rect, text_left, text_top, caret_x) =
            self.panel_layout(width, shape.caret_byte, shape.caret_fallback_chars, shape.caret_row);
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
    ///     the `Aa` case indicator (which is ALSO a click target — a press on it
    ///     toggles case, `PanelHit::CaseToggle`);
    ///   * a **replace** row (shown whenever replace is active) — the `replace` label
    ///     and the replacement text;
    ///   * a dim **key-hint** line that TEACHES the actions (`↵ replace+next`,
    ///     `⌘↵ all`, `⇥ switch`, `⌘⌥c case`, `Esc done`) — the keycaps ride glyphs
    ///     (↵ Return, ⇥ Tab) to match ⌘/⌥, informational muted ink, NOT clickable
    ///     buttons (the button-free principle; PHILOSOPHY §2). The case hint shows
    ///     the MAC-REACHABLE ⌘⌥c chord (bare ⌥c composes to 'ç' on macOS).
    /// The labels are padded to one width so the two value columns line up.
    pub(in crate::render) fn panel_shape_text(&mut self, width: u32) -> PanelShape {
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
        let hint = "\u{21B5} replace+next   \u{2318}\u{21B5} all   \u{21E5} switch   \u{2318}\u{2325}c case   Esc done";
        // DISCOVERABILITY: the "⌘⌥c case" chunk BRIGHTENS from muted to full ink when
        // case-sensitivity is ON — the same value cue the `Aa` indicator carries (never
        // amber; state by value), pointing the eye at the exact chord that toggles it.
        // Brightens iff the `Aa` cell also does (case ON and there IS a match), so the
        // two cues never disagree.
        const CASE_HINT: &str = "\u{2318}\u{2325}c case";
        let case_hint_on = self.search_case_sensitive && !no_match;
        let case_hint_span = hint.find(CASE_HINT).map(|s| (s, s + CASE_HINT.len()));
        let hint_color = |b: usize| match case_hint_span {
            Some((s, e)) if case_hint_on && b >= s && b < e => ink,
            _ => muted,
        };

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
            // rest stays in the world face. Each run is FURTHER split at the case-chunk
            // edges so a single span never mixes colors, letting ONLY "⌘⌥c case"
            // brighten (`hint_color`) when case-sensitivity is on.
            spans.push(("\n", mk(muted)));
            // The color-change boundaries within the hint: the case-chunk edges.
            let bounds: [usize; 2] =
                case_hint_span.map(|(s, e)| [s, e]).unwrap_or([hint.len(), hint.len()]);
            // A run is emitted piecewise, cutting at any boundary strictly inside it, so
            // each emitted piece is uniformly inside or outside the case chunk.
            let emit = |spans: &mut Vec<(&str, Attrs)>, mut s: usize, e: usize, is_sym: bool| {
                let mut cuts: Vec<usize> = bounds.iter().copied().filter(|&b| b > s && b < e).collect();
                cuts.push(e);
                for c in cuts {
                    let col = hint_color(s);
                    let attrs = if is_sym { sym(col) } else { mk(col) };
                    spans.push((&hint[s..c], attrs));
                    s = c;
                }
            };
            let mut last = 0usize;
            for run in symbol_runs(hint) {
                if run.start > last {
                    emit(&mut spans, last, run.start, false);
                }
                let end = run.end;
                emit(&mut spans, run.start, end, true);
                last = end;
            }
            if last < hint.len() {
                emit(&mut spans, last, hint.len(), false);
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

        // ITEM 10 — byte offset + char-prefix of the FOCUSED field's caret, at
        // its OWN CHAR-index position (`TextBox::caret`, mid-string reachable via
        // char/word motion) — not always the field's end. The offset is
        // LINE-relative (cosmic-text's `LayoutGlyph::start` counts from each
        // line's own origin, resetting to 0 after every `\n`), so the replace
        // row's cell is `REPLACE_LABEL.len() + <byte offset within replacement>`
        // WITHIN line 1 — NOT a buffer-global offset carrying the find row's
        // bytes, which would never match a line-1 glyph and drop the caret onto
        // the hardcoded-pitch fallback. `panel_layout` scopes its glyph scan to
        // `caret_row`, so a line-relative offset can never false-match the
        // identically-numbered byte on the find row. At the field's END (the
        // pre-item-10 ONLY position) the target byte lands on the reserved
        // trailing cell (`gap`/the replace row's own trailing space), exactly as
        // before; mid-field it lands on a real shaped glyph of the field's own text.
        let (caret_byte, caret_fallback_chars, caret_row) = if editing_replacement {
            let caret_char = self.search_replacement_caret.min(replacement.chars().count());
            (
                REPLACE_LABEL.len() + field_caret_byte(&replacement, caret_char),
                REPLACE_LABEL.chars().count() + caret_char,
                1.0_f32,
            )
        } else {
            let caret_char = self.search_query_caret.min(query.chars().count());
            (
                FIND_LABEL.len() + field_caret_byte(&query, caret_char),
                FIND_LABEL.chars().count() + caret_char,
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

        // ELEVATE the card on the reusable floating-panel primitive (raised
        // border + base_300 card, no drop shadow — dark-depth Option C), so the
        // summoned find/replace panel reads as risen a step above the crisp
        // document (DESIGN §5) — clearer, more present furniture than the old
        // flat pill. The flat `panel_card` is left empty; the search draw
        // branch draws the float quads (parked whenever the panel is down).
        self.prepare_float_panel(
            device,
            queue,
            width,
            height,
            Some(card_rect),
            FloatElevation::Rimmed,
            0.0,
            None,
        );
        self.panel_card.prepare(device, queue, width, height, &[]);
        self.panel_shadow.prepare(device, queue, width, height, &[]);
        self.panel_border.prepare(device, queue, width, height, &[]);
        Ok(())
    }

    /// Hit-test a physical pointer `(px, py)` against the summoned find/replace
    /// panel, for CLICK-TO-SWITCH-FIELD. Reuses `panel_layout`'s card + row
    /// geometry — the SAME layout the caret/text draw from, no parallel geometry —
    /// so a click can never disagree with where a field is painted:
    ///   * row 0, within the `Aa` cell at the row's right edge →
    ///     [`PanelHit::CaseToggle`] (the caller flips case sensitivity);
    ///   * row 0 elsewhere (`text_top .. +line_height`) → [`PanelHit::Find`];
    ///   * row 1 (present only once the replace row is revealed) → [`PanelHit::Replace`];
    ///   * anywhere else INSIDE the card (the key-hint line, inter-row gaps, the
    ///     pad) → [`PanelHit::Elsewhere`] (the caller swallows it — a calm no-op,
    ///     it never dismisses the search or moves the doc cursor beneath the card);
    ///   * OFF the card, or the panel is down → `None` (the caller lets the press
    ///     fall through to the document).
    /// The caret args to `panel_layout` do not affect the card rect / text origin,
    /// so pass zeros. Reads `self.window_w`, exactly like `overlay_geometry`.
    pub fn panel_hit(&self, px: f32, py: f32) -> Option<PanelHit> {
        if !self.search_active {
            return None;
        }
        let width = self.window_w as u32;
        let ([card_x, card_y, card_w, card_h], text_left, text_top, _caret_x) =
            self.panel_layout(width, 0, 0, 0.0);
        if px < card_x || px > card_x + card_w || py < card_y || py > card_y + card_h {
            return None;
        }
        let row = ((py - text_top) / self.metrics.line_height).floor() as i64;
        Some(match row {
            0 => match self.panel_case_toggle_span(text_left) {
                Some((x0, x1)) if px >= x0 && px <= x1 => PanelHit::CaseToggle,
                _ => PanelHit::Find,
            },
            1 if self.search_replace_active => PanelHit::Replace,
            _ => PanelHit::Elsewhere,
        })
    }

    /// Physical x-span `[x0, x1]` of the `Aa` case indicator on the find row
    /// (line 0), read from the SHAPED `panel_buffer` — the trailing two glyphs
    /// of the row (`"Aa"` is always the LAST span shaped onto row 0, `panel_shape_text`).
    /// Reading the real shaped advances keeps the click target in the SAME
    /// coordinate system the indicator paints in (no hardcoded pitch drift, the
    /// bug class `panel_layout` already guards for the caret). `text_left` is
    /// `panel_layout`'s inner text origin. `None` when row 0 has fewer than two
    /// glyphs (never in practice — `"Aa"` is always present).
    fn panel_case_toggle_span(&self, text_left: f32) -> Option<(f32, f32)> {
        for run in self.panel_buffer.layout_runs() {
            if run.line_i != 0 {
                continue;
            }
            let n = run.glyphs.len();
            if n < 2 {
                return None;
            }
            let a = &run.glyphs[n - 2]; // 'A'
            let z = &run.glyphs[n - 1]; // 'a'
            return Some((text_left + a.x, text_left + z.x + z.w));
        }
        None
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
}
