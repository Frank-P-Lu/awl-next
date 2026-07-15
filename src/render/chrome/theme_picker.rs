//! FACETED THEME PICKER chrome — the lens-strip / section-grouped world-row variant
//! of the summoned overlay: its display plan, its own geometry + span shaping, the
//! responsive strip fold, and the lens-strip hit-test. Lays out differently from the
//! flat pickers in [`super::overlay`] but shares [`OverlayGeom`]. Carved out of
//! `chrome.rs` verbatim, no behaviour change. See [`super`].

use super::*;

/// Slice a full display plan (headers + item rows, from [`TextPipeline::theme_plan`]) to
/// the ITEM window `[lo, hi)`: keep every `Item(i)` with `lo ≤ i < hi`, and re-hang the
/// SECTION HEADER above the first surviving item of each section (a header whose whole
/// section fell outside the window is dropped). Items in the window form a contiguous run
/// in the plan (the plan is built in item-index order), so one forward pass — carrying
/// the most-recent header until an in-window item consumes it — yields the correct
/// header→rows grouping for the visible slice. A window at the start of a section shows
/// that section's header at the top (`a section header at the window top is fine`).
fn window_plan(full: &[ThemeLine], lo: usize, hi: usize) -> Vec<ThemeLine> {
    let mut out: Vec<ThemeLine> = Vec::new();
    let mut pending: Option<&ThemeLine> = None;
    for line in full {
        match line {
            ThemeLine::Header(_) => pending = Some(line),
            ThemeLine::Item(i) => {
                if *i >= lo && *i < hi {
                    if let Some(h) = pending.take() {
                        out.push(h.clone());
                    }
                    out.push(line.clone());
                }
            }
        }
    }
    out
}

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

    /// Resolve the FACETED/GROUPED picker's geometry: a centered card carrying (line 0)
    /// the `› query` line, (line 1) the lens STRIP, then the section-grouped rows
    /// (headers + rows from [`Self::theme_plan`]), then the foot hint. `header_rows` is 2
    /// (query + strip), and the plan's own line offsets place the rows + band.
    ///
    /// The candidate area is WINDOWED (the grouped counterpart to the flat pickers'
    /// `MAX_ROWS` window): the shared [`scroll_window`] owner caps the visible ITEMS at
    /// the picker's own `overlay_window_rows` ([`crate::overlay::OverlayKind::window_rows`],
    /// canvas-reduced) and slides the window to keep the SELECTED row visible, then
    /// [`window_plan`] carries the section HEADERS that introduce those items. So a
    /// big faceted corpus (go-to / browse under a Recent/By-type/Folders lens, or the
    /// theme worlds) can never grow the card off the bottom of the screen, and every
    /// off-window row stays reachable by keyboard / wheel scroll. Windowing over ITEMS
    /// (not display lines) keeps the drawn rows in lockstep with the hover / keyboard
    /// item-window (same cap), so a click can never land on a row the item-window rejects.
    pub(super) fn theme_overlay_geometry(&self, width: u32) -> OverlayGeom {
        let lh = self.overlay_lh();
        let pad = 12.0;
        let margin = 12.0;
        let n_items = self.overlay_items.len();
        let full_plan = self.theme_plan();
        let hint = self.overlay_hint.clone();
        let hint_rows = if hint.is_empty() { 0 } else { 1 };
        // EMPTY STATE: a query that filtered every world out (or an empty corpus) →
        // the shared dim message row takes one candidate line below the strip.
        let empty = if n_items == 0 {
            self.overlay_empty.clone()
        } else {
            None
        };
        let empty_rows = empty.is_some() as usize;
        // Line 0 = query, line 1 = lens strip, then the plan lines / empty row, then hint.
        let header_rows = 2;
        // PALETTE-COMPOSITION round: the calm gap AFTER the query + lens-strip
        // header, before the section-grouped rows (negative space as the divider),
        // uniform with the flat pickers via the shared `overlay_header_gap` owner.
        let header_gap = self.overlay_header_gap();
        // `self.menubar_reserve()` — see [`TextPipeline::overlay_geometry`]'s identical
        // note; the SAME one-owner accessor, so the theme/caret picker's card yields
        // to a shown bar exactly like the flat/nav picker's does.
        let card_y = margin + 40.0 + self.menubar_reserve();
        // The visible-ITEM cap: the picker's own `overlay_window_rows` (the ONE owner,
        // `OverlayState::window_rows` — matching the flat/hover item-window exactly so the
        // drawn rows can never disagree with what hover/keyboard accept), FURTHER reduced
        // so the card can never exceed the canvas height on a short window. `fit_lines` is
        // how many candidate+chrome lines fit between the card top and the bottom margin;
        // the item budget is what remains after the fixed chrome (query + strip + hint +
        // empty) AND a reservation for every SECTION HEADER the plan can carry
        // (`total_headers` — a header adds a line ON TOP of its items), so `items + headers
        // + chrome ≤ fit_lines`. Reducing the cap (never raising it above the item-window)
        // keeps the drawn items a subset of the hover/keyboard item-window.
        let total_headers = full_plan.len() - n_items;
        let chrome_rows = header_rows + hint_rows + empty_rows;
        let avail_px = (self.window_h - card_y - margin - 2.0 * pad - header_gap).max(lh);
        let fit_lines = (avail_px / lh).floor() as usize;
        let fit_items = fit_lines
            .saturating_sub(chrome_rows)
            .saturating_sub(total_headers)
            .max(1);
        let item_cap = self.overlay_window_rows.max(1).min(fit_items);
        // Window over ITEMS via the shared owner (the pipeline owns the slide, so the
        // selected row is always in view regardless of the item-space scroll hint), then
        // re-hang the section headers for the items that survived.
        let (item_top, item_visible) =
            scroll_window(n_items, self.overlay_selected, self.overlay_scroll, item_cap);
        let plan = window_plan(&full_plan, item_top, item_top + item_visible);
        let total_rows = header_rows + plan.len() + empty_rows + hint_rows;
        // Wider than the flat pickers so the whole lens strip (Time … All) fits on one
        // line even on a WIDE mono world face without the far-right All clipping.
        let card_w = (width as f32 * 0.58).max(560.0).min(width as f32 - 2.0 * margin);
        let text_w = card_w - 2.0 * pad;
        let card_h = total_rows as f32 * lh + header_gap + 2.0 * pad;
        let card_x = self.overlay_card_x(width, card_w, margin);
        let text_left = card_x + pad;
        let text_top = card_y + pad;
        OverlayGeom {
            // The DRAWN window: `visible` = candidate DISPLAY LINES shown (headers + item
            // rows), `top_idx` = the first ITEM shown (0 when the whole list fits). The
            // theme draw/hit-test read `plan` directly (already the windowed slice), so
            // these feed the sidecar window report, not the row math.
            visible: plan.len(),
            top_idx: item_top,
            n_items,
            hint,
            hint_rows,
            footer: Vec::new(),
            footer_rows: 0,
            theme: true,
            strip: self.overlay_lens.clone(),
            plan,
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

    /// FACETED PICKER: hit-test a pointer against the lens STRIP (display line 1),
    /// returning the STRIP INDEX (into the picker's [`crate::facets::FacetScheme::strip`])
    /// the label under `(px, py)` selects — so a CLICK on a lens switches the facet (the
    /// pointing counterpart to LEFT/RIGHT). `None` off the strip row, off the card, or for
    /// a non-faceting overlay. Uses the same per-lens byte ranges the shaper laid out, read
    /// back from the shaped strip glyphs so the hit lands on the same label the eye sees.
    pub fn overlay_lens_at(&self, px: f32, py: f32) -> Option<usize> {
        if !self.overlay_active || self.overlay_lens.is_empty() {
            return None;
        }
        let geom = self.overlay_geometry(self.window_w as u32);
        if !geom.theme || px < geom.card_x || px > geom.card_x + geom.card_w {
            return None;
        }
        let lh = self.overlay_lh();
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
            // Find the facet whose glyph x-span covers `want`, returning its STRIP INDEX
            // (≥ 1). Rebuild the SAME strip string the shaper laid out — the `All` home
            // (strip index 0) is skipped, only the facets draw — tracking each range's
            // strip index so a hit maps back to the true facet, not a shifted position.
            let mut s = String::from("\n");
            let mut ranges: Vec<(usize, std::ops::Range<usize>)> = Vec::new();
            for (idx, (lbl, _)) in self.overlay_lens.iter().enumerate() {
                if idx == 0 {
                    continue; // the All home is not a drawn label
                }
                if idx > 1 {
                    s.push_str(STRIP_GAP);
                }
                let a = s.len();
                s.push_str(lbl);
                ranges.push((idx, a..s.len()));
            }
            for g in run.glyphs.iter() {
                if want >= g.x && want < g.x + g.w {
                    // Line-1 glyphs are byte-indexed within the strip line text (the
                    // leading "\n" split the lines); `ranges` are `strip_s`-relative, so
                    // shift the glyph byte forward past that one "\n" to compare.
                    let b = g.start + 1;
                    for (idx, r) in ranges.iter() {
                        if b >= r.start && b < r.end {
                            hit = Some(*idx);
                        }
                    }
                }
            }
        }
        // The hit is the facet's own STRIP INDEX (≥ 1), ready for `set_facet_lens`.
        hit
    }

    /// Shape the FACETED THEME picker into `panel_buffer`: the `› query` line (0), the
    /// lens STRIP (1, active lens in full ink + a recorded underline, others muted),
    /// then the section-grouped world rows (faint uppercase headers at LABEL size + rows
    /// in content ink), then the foot hint. Records the active-lens underline rect
    /// (scanned from the shaped strip glyphs, so it lands exactly under the label at any
    /// world face) into `overlay_theme_underline`. Shapes only the NAME column (returns
    /// `false`); its faceted caller ([`TextPipeline::shape_faceted`]) overlays the dim
    /// RIGHT column (chords / times / git) aligned to the plan's item rows when the
    /// picker fills one — the literal Theme picker has none, so it stays name-only.
    ///
    /// The strip renders ONLY the faceting lenses (strip index ≥ 1) — the `All` HOME
    /// (index 0, the flat/unfiltered corpus) is NOT drawn as a label. The flat state
    /// (`facet_lens == 0`) is simply NO facet underlined; `←` from the first facet
    /// returns there.
    pub(super) fn overlay_shape_theme(
        &mut self,
        geom: &OverlayGeom,
        ink: glyphon::Color,
        muted: glyphon::Color,
        selected_ink: Option<glyphon::Color>,
    ) -> bool {
        // Build the strip LINE ("\n" then the faceting-lens labels) as one owned string,
        // tracking each label's byte range so the ACTIVE label's glyphs can be underlined.
        // Strip index 0 (the `All` home) is SKIPPED — only the facets (index ≥ 1) draw.
        let mut strip_s = String::from("\n");
        let mut label_ranges: Vec<(std::ops::Range<usize>, bool)> = Vec::new();
        let mut sep_ranges: Vec<std::ops::Range<usize>> = Vec::new();
        let mut active_range: Option<std::ops::Range<usize>> = None;
        for (idx, (lbl, active)) in geom.strip.iter().enumerate() {
            if idx == 0 {
                continue; // the All home is the flat corpus, not a drawn label
            }
            if idx > 1 {
                let s = strip_s.len();
                strip_s.push_str(STRIP_GAP);
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
        // narrow window the full-size lens strip (Time Register …) can overflow the
        // card's text column — measured from the SHAPED line (real advances, not
        // the mean estimate), the whole strip steps down in size just enough to
        // fit, so every lens stays present + hit-testable instead of the far
        // right clipping away. At any comfortable width the measured strip fits
        // and the single full-size pass stands (byte-identical wide captures).
        self.shape_theme_spans(geom, ink, muted, selected_ink, &strip_s, &label_ranges, &sep_ranges, &hint_line, 1.0);
        let strip_w = self.theme_strip_px();
        if strip_w > geom.text_w {
            let scale = (geom.text_w / strip_w).max(0.5);
            self.shape_theme_spans(geom, ink, muted, selected_ink, &strip_s, &label_ranges, &sep_ranges, &hint_line, scale);
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
                let y = geom.text_top + 2.0 * self.overlay_lh() - 3.0;
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
    /// Line HEIGHTS stay uniform (the overlay UI `overlay_lh`) at any strip scale, so
    /// the plan line offsets, the selected band, and the underline `y` never drift.
    #[allow(clippy::too_many_arguments)]
    fn shape_theme_spans(
        &mut self,
        geom: &OverlayGeom,
        ink: glyphon::Color,
        muted: glyphon::Color,
        selected_ink: Option<glyphon::Color>,
        strip_s: &str,
        label_ranges: &[(std::ops::Range<usize>, bool)],
        sep_ranges: &[std::ops::Range<usize>],
        hint_line: &str,
        strip_scale: f32,
    ) {
        let m = self.metrics;
        let faint = theme::faint().to_glyphon();
        let label = crate::markdown::type_scale::LABEL;
        // Per-line font sizes ride the overlay UI base (`OVERLAY_UI_SCALE`), and their
        // LINE HEIGHTS stay the uniform UI row height (`overlay_lh`) so the plan line
        // offsets, the selected band, and the underline `y` never drift from a per-span
        // metric taller than the row.
        let ui = super::overlay::OVERLAY_UI_SCALE;
        let lh = self.overlay_lh();
        let header_metrics = GlyphMetrics::new(m.font_size * ui * label, lh);
        let base = panel_attrs();
        let mk = |c| base.clone().color(c);
        let sym = |c| Attrs::new().family(Family::Name(SYMBOL_FAMILY)).color(c);
        let sigil = "› ";

        // The world rows share the lone-column budget every no-right-column picker
        // gets (rowlayout owns it); today's short world names ride through whole. Rows
        // sit FLUSH-LEFT like every other picker (the live doc preview shows each
        // world's colours, so no per-row swatch chip / indent).
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
                    Some(rowlayout::fit_primary(name, row_budget).to_string())
                }
            })
            .collect();

        // Compose the spans. Query line 0 → strip line 1 → plan lines → hint. THE
        // OVERLAY-TITLES ROUND: prepend "<title> › " (muted) instead of the bare
        // sigil when this picker draws a title — the theme picker always does
        // (`draws_title_prefix` excludes only Rename/InsertLink, neither of which
        // this shaper serves). SUPPRESSED under a `Placard` title style (the corner
        // wordmark already names the picker) — the SAME `overlay_title_prefix` owner
        // the flat shaper uses, so the two inline paths cannot diverge.
        let title_prefix = self.overlay_title_prefix();
        let mut spans: Vec<(&str, glyphon::Attrs)> = Vec::new();
        if title_prefix.is_empty() {
            spans.push((sigil, mk(muted)));
        } else {
            spans.push((title_prefix.as_str(), mk(muted)));
        }
        spans.push((self.overlay_query.as_str(), mk(ink)));
        // Strip line: active label in full ink, others muted, separators + the "\n"
        // faint. One ordered pass over `strip_s` so the spans tile the line in byte
        // order (rich-text concatenates spans in push order). The label/separator
        // spans carry the strip font size at the `strip_lh` (= `lh + header_gap`)
        // row height; the leading "\n" keeps the buffer's UI font size so the strip
        // row's font stays scale-invariant.
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
            // The strip row's HEIGHT is inflated by `header_gap` (PALETTE-COMPOSITION
            // round) so the calm divider space falls after the lens strip, before the
            // section-grouped rows — uniform with the flat pickers' query-line gap.
            // The plan-line offsets, selected band, and underline all fold the same
            // gap in through `overlay_row_top`, so nothing below the strip drifts.
            //
            // The gap MUST ride the strip line's REAL LABEL glyphs, NOT its leading
            // "\n": cosmic-text sizes a line from the glyphs ON it, and the "\n" is a
            // BREAK that terminates the PRIOR (query) line — its own metrics never
            // grow the strip line, so inflating only the "\n" moved the selected BAND
            // (which reads `header_gap` off `overlay_row_top`) down a half-row while
            // the TEXT stayed put. That half-row band/text drift was invisible under
            // a gentle value band but clipped the top of the selected row's own
            // glyphs once a 1-bit world drew them as solid black on a white band
            // (the Wagtail selected-row bug's second half). `strip_lh` on the labels
            // makes text and band agree; the "\n" keeps the row's scale-invariant
            // baseline size.
            let strip_lh = lh + geom.header_gap;
            spans.push((&strip_s[0..1], mk(faint).metrics(GlyphMetrics::new(m.font_size * ui, lh))));
            cursor += 1;
            for (r, c) in pushes {
                debug_assert_eq!(r.start, cursor, "strip spans must tile the line");
                cursor = r.end;
                let fs = if strip_scale < 1.0 {
                    m.font_size * ui * strip_scale
                } else {
                    m.font_size * ui
                };
                spans.push((&strip_s[r], mk(c).metrics(GlyphMetrics::new(fs, strip_lh))));
            }
        }
        // Plan lines: faint uppercase section headers (LABEL size) + world rows (ink).
        // On a true 1-bit world the SELECTED item's own glyphs recolor to the solid
        // contrasting ink (`selected_ink`) so black text lands crisp on the white
        // band — the same crisp black-on-white the flat pickers get, one rule (see
        // `HighlightTreatment::InverseFill`). Byte-identical (`None`) elsewhere.
        for (line, fit) in geom.plan.iter().zip(fitted.iter()) {
            spans.push(("\n", mk(ink)));
            match line {
                ThemeLine::Header(h) => {
                    spans.push((h.as_str(), mk(faint).metrics(header_metrics)));
                }
                ThemeLine::Item(i) => {
                    let c = match selected_ink {
                        Some(c) if *i == self.overlay_selected => c,
                        _ => ink,
                    };
                    spans.push((fit.as_deref().unwrap_or(""), mk(c)));
                }
            }
        }
        // EMPTY STATE: a query that filtered every world out (or an empty corpus)
        // shows the shared dim message row below the strip — the same calm
        // "no matches" the flat pickers show, one owner (`geom.empty`).
        if let Some(msg) = &geom.empty {
            spans.push(("\n", mk(muted)));
            spans.push((msg.as_str(), mk(muted)));
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

#[cfg(test)]
mod tests {
    use super::{window_plan, ThemeLine};

    /// A plan mirroring `theme_plan`: two sections (`A`: items 0,1,2 — `B`: items 3,4).
    fn sample_plan() -> Vec<ThemeLine> {
        vec![
            ThemeLine::Header("A".into()),
            ThemeLine::Item(0),
            ThemeLine::Item(1),
            ThemeLine::Item(2),
            ThemeLine::Header("B".into()),
            ThemeLine::Item(3),
            ThemeLine::Item(4),
        ]
    }

    fn shape(plan: &[ThemeLine]) -> Vec<String> {
        plan.iter()
            .map(|l| match l {
                ThemeLine::Header(h) => format!("#{h}"),
                ThemeLine::Item(i) => format!("i{i}"),
            })
            .collect()
    }

    /// The whole list fitting under the cap returns the plan verbatim (headers + rows).
    #[test]
    fn window_plan_returns_the_full_plan_when_it_fits() {
        assert_eq!(shape(&window_plan(&sample_plan(), 0, 5)), shape(&sample_plan()));
    }

    /// A mid-list window keeps ONLY the in-range items and re-hangs the header of each
    /// section it touches — a section with no in-range item drops its header entirely.
    #[test]
    fn window_plan_keeps_only_touched_sections_headers() {
        // Items [2, 4): item 2 (section A) + item 3 (section B). Header A rides above 2,
        // header B above 3; item 0/1/4 and neither section's other rows appear.
        assert_eq!(
            shape(&window_plan(&sample_plan(), 2, 4)),
            vec!["#A", "i2", "#B", "i3"]
        );
        // Items [3, 5): only section B — section A's header is dropped, B's header leads.
        assert_eq!(
            shape(&window_plan(&sample_plan(), 3, 5)),
            vec!["#B", "i3", "i4"]
        );
    }

    /// A window that starts mid-section shows that section's header at the TOP (the
    /// documented "a section header at the window top is fine"), and never duplicates it.
    #[test]
    fn window_plan_header_at_window_top_and_no_duplicates() {
        // Items [1, 3): both in section A — one A header, then the two rows.
        assert_eq!(
            shape(&window_plan(&sample_plan(), 1, 3)),
            vec!["#A", "i1", "i2"]
        );
    }

    /// An empty window (no items in range) yields nothing — no stray headers.
    #[test]
    fn window_plan_empty_range_is_empty() {
        assert!(window_plan(&sample_plan(), 9, 9).is_empty());
        assert!(window_plan(&sample_plan(), 5, 5).is_empty());
    }
}
