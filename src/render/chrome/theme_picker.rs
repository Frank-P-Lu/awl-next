//! FACETED THEME PICKER chrome — the lens-strip / section-grouped world-row variant
//! of the summoned overlay: its display plan, its own geometry + span shaping, the
//! responsive strip fold, and the lens-strip hit-test. Lays out differently from the
//! flat pickers in [`super::overlay`] but shares [`OverlayGeom`]. Carved out of
//! `chrome.rs` verbatim, no behaviour change. See [`super`].

use super::*;

impl TextPipeline {
    /// THEME PICKER display plan: the candidate-area sequence of section HEADERS +
    /// world ROWS, from the parallel `overlay_sections`. A header is emitted before a
    /// row whenever its section differs from the previous row's (so contiguous groups
    /// get one header each); the All lens / non-grouped rows emit no headers. Section
    /// labels are uppercased for the faint header display. Shared by the geometry,
    /// shaping, selected-band, and hit-test so they can never disagree.
    pub(in crate::render) fn theme_plan(&self) -> Vec<ThemeLine> {
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
    pub(super) fn theme_overlay_geometry(&self, width: u32) -> OverlayGeom {
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

    /// Shape the FACETED THEME picker into `panel_buffer`: the `› query` line (0), the
    /// lens STRIP (1, active lens in full ink + a recorded underline, others muted, the
    /// `All` label pushed right past a faint separator), then the section-grouped world
    /// rows (faint uppercase headers at LABEL size + rows in content ink), then the foot
    /// hint. Records the active-lens underline rect (scanned from the shaped strip
    /// glyphs, so it lands exactly under the label at any world face) into
    /// `overlay_theme_underline`. No right column (returns `false`).
    pub(super) fn overlay_shape_theme(
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
}
