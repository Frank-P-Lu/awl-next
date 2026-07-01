//! LAYER GEOMETRY — the rect / squiggle builders that turn document + view state
//! into the instanced quads each draw layer uploads: selection + arbitrary
//! char-range rectangles, the search-match highlights, the markdown horizontal-rule
//! quads, the spell-underline squiggles, the IME preedit cells, and the
//! search/replace panel layout.
//!
//! These are inherent methods on [`super::TextPipeline`]: they read its shaped
//! buffer / cursor / selection / search / metrics state (the real glyph advances,
//! wrap-aware visual rows) to place pixels, so they can't be `&self`-free. This
//! module is purely a physical home for that cohesive rect-building cluster, carved
//! out of `render.rs` verbatim. A child module sees its ancestor's private items, so
//! the methods keep full access to `TextPipeline`'s fields/helpers and to the
//! `geometry` row helpers with NO behaviour change — the quads are byte-identical.

use super::*;

impl TextPipeline {
    /// Logical line indices that carry a Markdown `Rule` span (a thematic break)
    /// AND should render the centered `hr_ornament` fleuron — i.e. every hr line the
    /// caret is NOT on. Driven by the parsed `md_spans` — NOT a bare line scan — so a
    /// setext `---` heading underline is correctly NOT a rule. REVEAL-ON-CURSOR: the
    /// hr line the caret sits on is EXCLUDED here (its raw `---` reveal for editing
    /// and the fleuron yields to them), exactly the line [`build_line_attrs`] leaves
    /// un-concealed — so the dash-conceal and the fleuron toggle stay in lockstep.
    /// Empty for a non-markdown buffer.
    pub(super) fn rule_lines(&self) -> Vec<usize> {
        if self.md_spans.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut start = 0usize;
        for (li, line) in self.buffer.lines.iter().enumerate() {
            let end = start + line.text().len();
            let is_rule = self
                .md_spans
                .iter()
                .any(|(r, k)| *k == crate::markdown::MdKind::Rule && r.start < end + 1 && r.end > start);
            if is_rule && li != self.cursor_line {
                out.push(li);
            }
            start = end + 1;
        }
        out
    }

    /// True when buffer line `li`'s markdown horizontal-rule `---` glyphs are CONCEALED
    /// (rendered with transparent ink) in the currently-laid attrs — the reveal-on-
    /// cursor state for an hr the caret is NOT on. Reads the laid color at the line's
    /// first byte: `false` for a non-rule line, an out-of-range index, or when the
    /// caret is on the line (the dashes reveal). Used by the tests to assert the
    /// conceal/reveal toggle without eyeballing pixels.
    #[cfg(test)]
    pub(super) fn rule_line_concealed(&self, li: usize) -> bool {
        let Some(line) = self.buffer.lines.get(li) else {
            return false;
        };
        if line.text().is_empty() {
            return false;
        }
        matches!(line.attrs_list().get_span(0).color_opt, Some(c) if c.a() == 0)
    }

    /// The centered ornament for each markdown thematic-break line the caret is NOT
    /// on: its first visual row's absolute top-y (current scroll + zoom) paired with
    /// the GLYPH to draw there — chosen PER SYNTAX from the active world's
    /// [`theme::Ornaments`] set by which break the author typed (`---` → dash, `***`
    /// → star, `___` → underscore; see [`crate::markdown::break_kind`]). One entry
    /// per [`Self::rule_lines`]; the dim raw glyphs stay underneath (present +
    /// editable). Empty for a non-markdown buffer. Off-screen rows still produce
    /// geometry (cheap — awl docs are small).
    pub(super) fn rule_marks(&self) -> Vec<(f32, char)> {
        let lines = self.rule_lines();
        if lines.is_empty() {
            return Vec::new();
        }
        let orn = theme::active().ornaments;
        let doc_top = self.doc_top();
        lines
            .into_iter()
            .map(|li| {
                let top = doc_top + self.visual_rows(li)[0].line_top;
                let kind = crate::markdown::break_kind(self.buffer.lines[li].text());
                (top, orn.pick(kind))
            })
            .collect()
    }

    /// The absolute top-y of each markdown thematic-break line's ornament — the
    /// tops half of [`Self::rule_marks`]. Kept as its own accessor for the geometry
    /// tests (which assert placement independent of which ornament renders).
    #[cfg(test)]
    pub(super) fn rule_tops(&self) -> Vec<f32> {
        self.rule_marks().into_iter().map(|(t, _)| t).collect()
    }

    /// The absolute top-y of the END-OF-DOCUMENT mark — one row BELOW the last
    /// logical line's final visual row (current scroll + zoom). The renderer drops
    /// the world's centered `end_mark` colophon there for markdown buffers. Always
    /// computable; the caller gates drawing on `md_enabled`.
    pub(super) fn end_mark_top(&self) -> f32 {
        let last = self.buffer.lines.len().saturating_sub(1);
        let rows = self.visual_rows(last);
        let r = rows.last().expect("visual_rows is never empty");
        self.doc_top() + r.line_top + r.line_height
    }

    /// Build the wavy-underline geometry for every misspelled span, in pixels,
    /// for the current scroll + zoom. Mirrors [`Self::selection_rects`]: it reads
    /// the line's real per-char x boundaries (advance-aware) so the squiggle's
    /// x-range matches the word's glyphs, and places the band just below the
    /// glyph cell. Spans on scrolled-off lines still produce geometry (the
    /// shader/quad simply lands off-screen); the count is tiny so this is cheap.
    /// The row-centred caret-height band `(y, height)` for one visual `row`, where
    /// `line_top` is the row's ABSOLUTE top (`doc_top + row.line_top`). The caret
    /// height is scaled by the row's own height (so a tall heading row gets a taller
    /// band), then centred vertically in the row. Shared by the squiggle and
    /// selection rect builders so both scale identically to a heading.
    pub(super) fn row_caret_band(&self, row: &VisualRow, line_top: f32) -> (f32, f32) {
        let m = &self.metrics;
        let row_caret_h = m.caret_h * (row.line_height / m.line_height);
        let y = line_top + (row.line_height - row_caret_h) * 0.5;
        (y, row_caret_h)
    }

    pub(super) fn spell_squiggles(&self) -> Vec<Squiggle> {
        if self.misspelled.is_empty() {
            return Vec::new();
        }
        let m = &self.metrics;
        let doc_top = self.doc_top();
        let amp = SPELL_AMP * m.zoom;
        let period = SPELL_PERIOD * m.zoom;
        let thickness = SPELL_THICKNESS * m.zoom;
        // The band must be tall enough to contain the wave crests + the stroke.
        let band_h = amp * 2.0 + thickness + 2.0;
        let mut out = Vec::with_capacity(self.misspelled.len());
        for sp in &self.misspelled {
            // A misspelled span is a single word; cosmic-text wraps at spaces so
            // the word stays on ONE visual run. Find the run owning its start
            // column and use that run's wrap-aware top + own x boundaries, so the
            // squiggle sits directly under the word's glyphs at any wrap/zoom.
            let rows = self.visual_rows(sp.line);
            let row = pick_row(&rows, sp.start_col);
            let char_count = row.xs.len().saturating_sub(1);
            let s = sp.start_col.min(char_count);
            let e = sp.end_col.min(char_count);
            if e <= s {
                continue;
            }
            let (x, w) = row_x_span(row, self.text_left(), s, e, 1.0);
            // Sit the squiggle just below the glyph cell (a hair under the
            // bottom of the caret-height box), centered vertically in its band.
            let line_top = doc_top + row.line_top;
            let (band_y, row_caret_h) = self.row_caret_band(row, line_top);
            let cell_bottom = band_y + row_caret_h;
            // Center the wave band a touch below the cell bottom.
            let y = cell_bottom + 1.0 * m.zoom;
            out.push(Squiggle {
                x,
                y,
                w,
                h: band_h,
                amp,
                period,
                thickness,
            });
        }
        out
    }

    /// Compute the selection highlight rectangles in pixels for the current
    /// selection, scroll, and zoom. Multi-line: first line from anchor-col to
    /// end-of-line, full-width middle lines, last line up to cursor-col. Each
    /// rect is `[x, y, w, h]`. Reads the SAME metrics + scroll as glyph layout,
    /// so the highlight sits exactly behind the selected glyphs.
    pub(super) fn selection_rects(&self) -> Vec<[f32; 4]> {
        let Some(((l0, c0), (l1, c1))) = self.selection else {
            return Vec::new();
        };
        self.range_rects((l0, c0), (l1, c1))
    }

    /// All translucent-quad rects (in pixels, current scroll+zoom) for ONE
    /// ordered ((l0,c0),(l1,c1)) CHAR range. Extracted from `selection_rects`
    /// so search-match highlights reuse the EXACT same advance-aware geometry.
    pub(super) fn range_rects(&self, (l0, c0): (usize, usize), (l1, c1): (usize, usize)) -> Vec<[f32; 4]> {
        let m = &self.metrics;
        let doc_top = self.doc_top();
        // A small fill so a zero-width (empty-line) selected line still shows a
        // sliver, and so end-of-line highlights extend slightly past the last
        // glyph (the way most editors render a selected newline).
        let eol_pad = m.char_width * 0.5;
        let mut rects = Vec::new();
        for line in l0..=l1 {
            // The logical line's column span [sel_start, sel_end] within the
            // selection. For lines before the last, the selection runs through the
            // (virtual) newline at end-of-line; the last line stops at c1.
            let line_char_count = {
                let xs = self.line_glyph_xs(line);
                xs.len().saturating_sub(1)
            };
            let sel_start = if line == l0 { c0 } else { 0 };
            let (sel_end, extends_to_eol) = if line == l1 {
                (c1.min(line_char_count), false)
            } else {
                (line_char_count, true)
            };
            let sel_start = sel_start.min(line_char_count);
            // Emit one rect per VISUAL row of this logical line, clipped to the
            // selection's column span on that row. Each row uses its OWN wrap-aware
            // top + x boundaries, so a selection that spans a wrap boundary follows
            // the text down to the next row. For a non-wrapped line this is exactly
            // one row at `line * line_height` -> identical to the old behavior.
            let rows = self.visual_rows(line);
            for (ri, row) in rows.iter().enumerate() {
                let row_char_count = row.xs.len().saturating_sub(1);
                // Intersect the selection's column span with this row's columns.
                let rs = sel_start.max(row.start_col);
                let re = sel_end.min(row.end_col);
                if re < rs {
                    continue;
                }
                let is_last_row = ri + 1 == rows.len();
                // Only the row that actually reaches the logical end-of-line gets
                // the newline pad (the trailing-selection sliver editors show).
                let pad = if extends_to_eol && is_last_row && re >= row_char_count {
                    eol_pad
                } else {
                    0.0
                };
                let a = rs.min(row_char_count);
                let b = re.min(row_char_count);
                let (x, w_raw) = row_x_span(row, self.text_left(), a, b, 0.0);
                let w = w_raw + pad;
                if w <= 0.0 {
                    continue;
                }
                // Scale the highlight to the row so a heading's selection is as tall
                // as its glyphs (a base-height band on a big heading reads as broken).
                let (y, row_caret_h) = self.row_caret_band(row, doc_top + row.line_top);
                rects.push([x, y, w, row_caret_h]);
            }
        }
        rects
    }

    /// Translucent highlight rects for ALL active search matches (one set per
    /// match, in document order). The CURRENT match gets no distinct color: the
    /// real amber caret already sits on it.
    pub(super) fn search_match_rects(&self) -> Vec<[f32; 4]> {
        let mut r = Vec::new();
        for &(a, b) in &self.search_matches {
            r.extend(self.range_rects(a, b));
        }
        r
    }

    /// True only when the query is non-empty and yields zero hits — the single
    /// state that tints the panel field with ERROR red.
    pub(super) fn search_no_matches(&self) -> bool {
        self.search_active && !self.search_query.is_empty() && self.search_matches.is_empty()
    }

    /// Geometry of the top-right panel for the current canvas `width`, derived
    /// from the SHAPED panel_buffer advances. Returns:
    /// (card_rect [x,y,w,h], text_left, text_top, caret_x). `caret_byte` is the
    /// byte offset (into the shaped panel string) of the focused field's reserved
    /// caret cell; `fallback_chars` is the char-column to place it at if shaping
    /// produced no glyph there. The card sizes to ALL shaped rows (one for plain
    /// search, two once the replace field is revealed).
    pub(super) fn panel_layout(
        &self,
        width: u32,
        caret_byte: usize,
        fallback_chars: usize,
    ) -> ([f32; 4], f32, f32, f32) {
        let m = &self.metrics;
        let pad = 12.0;
        let margin = 12.0;
        // Measure the shaped panel: widest run sets the card width, the run count
        // sets its height (so the replace row grows the card by one line).
        let mut text_w = 0.0_f32;
        let mut rows = 0usize;
        for run in self.panel_buffer.layout_runs() {
            text_w = text_w.max(run.line_w);
            rows += 1;
        }
        let rows = rows.max(1) as f32;
        let card_w = text_w + 2.0 * pad;
        let card_h = rows * m.line_height + 2.0 * pad;
        let card_x = width as f32 - card_w - margin;
        let card_y = margin;
        let text_left = card_x + pad;
        let text_top = card_y + pad;
        // The caret block rides in the RESERVED cell shaped immediately after the
        // focused field's text. Read its x from the SHAPED panel_buffer so the
        // caret and the counter live in ONE coordinate system — placing it via a
        // hardcoded CHAR_WIDTH instead let the block drift relative to glyphon's
        // real advances and collide with "N/M" (the old overlap bug). Find the
        // glyph whose byte `start` is at the cell; fall back to the hardcoded
        // advance only if shaping produced no glyph there.
        let mut caret_x = None;
        for run in self.panel_buffer.layout_runs() {
            for g in run.glyphs.iter() {
                if g.start == caret_byte {
                    caret_x = Some(text_left + g.x);
                    break;
                }
            }
            if caret_x.is_some() {
                break;
            }
        }
        let caret_x = caret_x.unwrap_or(text_left + m.char_width * fallback_chars as f32);
        ([card_x, card_y, card_w, card_h], text_left, text_top, caret_x)
    }

    /// Underline rectangle(s) for an active IME preedit, in the SAME `[x,y,w,h]`
    /// pixel form as selection rects (they share the translucent-quad pipeline).
    /// The preedit occupies `[start_col, cursor_col)` on the cursor line (it was
    /// spliced in there and the caret advanced to its end); the underline is a
    /// thin bar beneath those real shaped glyphs so composing CJK/kana reads as
    /// provisional. Empty when no composition is active.
    pub(super) fn preedit_rects(&self) -> Vec<[f32; 4]> {
        let n = self.preedit.chars().count();
        if n == 0 {
            return Vec::new();
        }
        let line = self.cursor_line;
        let end_col = self.cursor_col;
        let start_col = end_col.saturating_sub(n);
        // Place on the wrap-aware visual row that owns the preedit's start column
        // (using that row's own x boundaries), matching the caret which sits at
        // the preedit's end.
        let rows = self.visual_rows(line);
        let row = pick_row(&rows, start_col);
        let char_count = row.xs.len().saturating_sub(1);
        let s = start_col.min(char_count);
        let e = end_col.min(char_count);
        let (x, w) = row_x_span(row, self.text_left(), s, e, 1.0);
        let m = &self.metrics;
        let line_top = self.doc_top() + row.line_top;
        // Sit the bar just below the glyph cell (bottom of the caret-height box).
        let cell_top = line_top + (m.line_height - m.caret_h) * 0.5;
        let thickness = PREEDIT_UNDERLINE_H * m.zoom;
        let y = cell_top + m.caret_h - thickness;
        vec![[x, y, w, thickness]]
    }
}
