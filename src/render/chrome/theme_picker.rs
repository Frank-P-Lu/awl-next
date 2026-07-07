//! FACETED THEME PICKER chrome — the lens-strip / section-grouped world-row variant
//! of the summoned overlay: its display plan, its own geometry + span shaping, the
//! responsive strip fold, and the lens-strip hit-test. Lays out differently from the
//! flat pickers in [`super::overlay`] but shares [`OverlayGeom`]. Carved out of
//! `chrome.rs` verbatim, no behaviour change. See [`super`].

use super::*;

/// Theme-picker SWATCH geometry: a per-row colour chip in the world-row's LEFT
/// gutter — a GROUND band (the target world's own `base_100`) with its warm ACCENT
/// dot (`primary`) laid on the band's right end, so each world's palette reads at a
/// glance (DESIGN: one warm element on its own ground). Drawn on the reused
/// selection-quad pipeline (`overlay_swatches`); ONLY the theme picker has a per-row
/// colour to show. `SWATCH_BAND_W` is the ground band's width; the world name is
/// indented past `SWATCH_GUTTER_PX` so the chip can never overlap it — the name still
/// budgets + elides through [`rowlayout`] against the REMAINING column width.
const SWATCH_BAND_W: f32 = 20.0;
const SWATCH_GUTTER_PX: f32 = 30.0;

/// Chars of leading indent a world-row name needs to clear the swatch gutter, at the
/// row's own `char_width`. The shaper prepends this many spaces to each world row AND
/// shrinks that row's elision budget by the same count, so the name lands just past
/// the chip and still elides correctly. `0` on a degenerate zero-width metric.
fn swatch_indent_chars(char_width: f32) -> usize {
    if char_width <= 0.0 {
        0
    } else {
        (SWATCH_GUTTER_PX / char_width).ceil() as usize
    }
}

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
    /// The theme-picker SWATCH quads for this frame: for each WORLD row in the plan, a
    /// GROUND band + an ACCENT dot in that world's own palette ([`theme::swatch_for`]),
    /// each as `([x, y, w, h], srgba)`. Empty for a non-theme card / an unknown world.
    /// The row Y rides the SAME [`overlay_row_top`] owner the selected band + hit-test
    /// use, so a chip always sits on its own row.
    pub(in crate::render) fn theme_swatch_quads(
        &self,
        geom: &OverlayGeom,
    ) -> Vec<([f32; 4], [u8; 4])> {
        if !geom.theme {
            return Vec::new();
        }
        let m = self.metrics;
        let band_h = m.line_height * 0.5;
        let dot_d = m.line_height * 0.34;
        let mut quads: Vec<([f32; 4], [u8; 4])> = Vec::new();
        for (disp, line) in geom.plan.iter().enumerate() {
            let ThemeLine::Item(i) = line else { continue };
            let name = self.overlay_items.get(*i).map(|s| s.as_str()).unwrap_or("");
            let Some((ground, accent)) = theme::swatch_for(name) else {
                continue;
            };
            let row_top = overlay_row_top(geom.text_top, geom.header_rows, disp, m.line_height);
            let cy = row_top + m.line_height * 0.5;
            // GROUND band: the world's `base_100`, vertically centered in the row.
            quads.push((
                [geom.text_left, cy - band_h * 0.5, SWATCH_BAND_W, band_h],
                ground.rgba_bytes(),
            ));
            // ACCENT dot: the world's `primary`, laid on the band's right end (the one
            // warm element on its ground). A small square softened by the pipeline's
            // corner radius reads as a dot.
            quads.push((
                [
                    geom.text_left + SWATCH_BAND_W - dot_d,
                    cy - dot_d * 0.5,
                    dot_d,
                    dot_d,
                ],
                accent.rgba_bytes(),
            ));
        }
        quads
    }

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
        let m = self.metrics;
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
        let card_y = margin + 40.0;
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
        let avail_px = (self.window_h - card_y - margin - 2.0 * pad).max(m.line_height);
        let fit_lines = (avail_px / m.line_height).floor() as usize;
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
        let card_h = total_rows as f32 * m.line_height + 2.0 * pad;
        let card_x = (width as f32 - card_w) * 0.5;
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
            theme: true,
            strip: self.overlay_lens.clone(),
            plan,
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
            let mut s = String::from("\n");
            let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
            for (i, lbl) in labels.iter().enumerate() {
                if i > 0 {
                    // The wide All-separator sits after the leftmost All (index 0 → 1).
                    s.push_str(if i == 1 { STRIP_ALL_SEP } else { STRIP_GAP });
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
        // The hit is already the strip index (the lens labels tile STRIP order 1:1).
        hit
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
        // `All` label (FIRST) is set apart from the faceted lenses by a wider faint
        // separator that follows it (between strip index 0 and 1).
        let mut strip_s = String::from("\n");
        let mut label_ranges: Vec<(std::ops::Range<usize>, bool)> = Vec::new();
        let mut sep_ranges: Vec<std::ops::Range<usize>> = Vec::new();
        let mut active_range: Option<std::ops::Range<usize>> = None;
        for (idx, (lbl, active)) in geom.strip.iter().enumerate() {
            if idx > 0 {
                let s = strip_s.len();
                strip_s.push_str(if idx == 1 { STRIP_ALL_SEP } else { STRIP_GAP });
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
        // gets (rowlayout owns it); today's short world names ride through whole. Each
        // world row is INDENTED past the palette SWATCH gutter (a leading run of spaces
        // sized to `SWATCH_GUTTER_PX`), so the chip in the row's left gutter never
        // overlaps the name — the elision budget is SHRUNK by the same indent so a
        // (future) long world name still fits + middle-elides against the column that
        // REMAINS after the swatch, keeping the rowlayout no-overlap law honest.
        let total_chars = if m.char_width > 0.0 {
            (geom.text_w / m.char_width).floor() as usize
        } else {
            usize::MAX
        };
        let indent_chars = swatch_indent_chars(m.char_width);
        let indent: String = " ".repeat(indent_chars);
        let row_budget = rowlayout::full_budget(total_chars.saturating_sub(indent_chars));
        let fitted: Vec<Option<String>> = geom
            .plan
            .iter()
            .map(|line| match line {
                ThemeLine::Header(_) => None,
                ThemeLine::Item(i) => {
                    let name = self.overlay_items.get(*i).map(|s| s.as_str()).unwrap_or("");
                    Some(format!("{indent}{}", rowlayout::fit_primary(name, row_budget)))
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
    use super::{swatch_indent_chars, window_plan, ThemeLine, SWATCH_BAND_W, SWATCH_GUTTER_PX};

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

    /// The world-row name's leading indent (spaces sized to `SWATCH_GUTTER_PX` at the
    /// row's `char_width`) always clears the SWATCH chip in the row's left gutter — so
    /// the chip's ground band + accent dot can never overlap the name at any zoom /
    /// world face. Swept over a range of char widths (a mono narrow ~7px through a
    /// wide serif ~16px). The gutter is strictly wider than the band, and the ceil'd
    /// indent lands at or past the gutter, so the name is always to the right of the
    /// chip. (The chip itself lives entirely within `[0, SWATCH_BAND_W]` of the row's
    /// text-left — see `theme_swatch_quads`.)
    #[test]
    fn swatch_indent_clears_the_chip_at_every_char_width() {
        assert!(
            SWATCH_GUTTER_PX > SWATCH_BAND_W,
            "the name gutter must be wider than the chip band"
        );
        for &cw in &[7.0f32, 9.0, 12.0, 14.4, 16.0] {
            let indent_px = swatch_indent_chars(cw) as f32 * cw;
            assert!(
                indent_px >= SWATCH_GUTTER_PX,
                "indent {indent_px} px (cw {cw}) must reach the gutter {SWATCH_GUTTER_PX}"
            );
            assert!(
                indent_px > SWATCH_BAND_W,
                "the name at {indent_px} px must start past the chip band {SWATCH_BAND_W}"
            );
        }
    }

    /// A degenerate zero (or negative) char width yields NO indent rather than a
    /// divide blow-up — the row simply renders flush (the swatch draw is independently
    /// gated), never a panic.
    #[test]
    fn swatch_indent_is_zero_on_a_degenerate_metric() {
        assert_eq!(swatch_indent_chars(0.0), 0);
        assert_eq!(swatch_indent_chars(-1.0), 0);
    }
}
