//! FACETED THEME PICKER chrome — the lens-strip / section-grouped world-row variant
//! of the summoned overlay: its display plan, its own geometry + span shaping, the
//! responsive strip fold, and the lens-strip hit-test. Lays out differently from the
//! flat pickers in [`super::overlay`] but shares [`OverlayGeom`]. Carved out of
//! `chrome.rs` verbatim, no behaviour change. See [`super`].

use super::*;

/// Pixels the active-lens UNDERLINE sits BELOW the strip run's shaped baseline
/// (`overlay_shape_theme`). Small so the rule hugs the label — enough to clear
/// the baseline for every chrome/mono/display face without striking the glyphs.
const UNDERLINE_BASELINE_DROP: f32 = 2.0;

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
        // Wider than the flat pickers so the whole lens strip (Time … All) fits on
        // one line even on a WIDE mono world face without the far-right All clipping
        // — via the SAME horizontal-box owner (edge inset + narrow-window fallback),
        // just a slightly wider cap ([`CARD_MAX_W_FACETED`]).
        let (card_x, card_w) = self.overlay_card_box(width, super::overlay::CARD_MAX_W_FACETED);
        // item 4 (NARROW FOLD): the placard folds to InlinePrefix once even the
        // floor inset can't seat the faceted card's desired width — the SAME
        // owner the width fallback above reads.
        let card_narrow =
            super::overlay::overlay_card_fill_regime(width as f32, super::overlay::CARD_MAX_W_FACETED);
        // List-style-aware horizontal text inset (the ONE owner shared with the
        // flat picker); vertical padding stays `pad`. `Pane` keeps `hpad == pad`.
        let hpad = self.overlay_text_hpad();
        let text_w = card_w - 2.0 * hpad;
        // Foot hint (item 5) rides a SHORTER line — reclaim `lh - hint_h` per hint
        // row so the card hugs the tighter footer (matching the flat owner).
        let card_h = total_rows as f32 * lh + header_gap + 2.0 * pad
            - self.overlay_footer_reclaim(hint_rows);
        // MOTION-JUICE ENTRANCE: folded in AFTER the `avail_px`/row-fit math
        // above (which reads the SETTLED `card_y` — the transient drop must
        // never change how many rows fit), mirroring `overlay_geometry`'s own
        // placement of the same one-owner offset. `+ 0.0` when settled.
        let card_y = card_y + self.overlay_entrance_offset();
        let text_left = card_x + hpad;
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
            card_narrow,
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
        // Strip is display line 1, whose HEIGHT is inflated to `lh + header_gap` by
        // the query BEAT (the labels center in that tall box, so they sit lower than
        // a plain `lh` band — the whole inflated line is the strip's clickable region,
        // meeting row 0's top exactly). Using the plain `lh` band would leave the
        // lower half of the labels un-clickable once the beat widened.
        let strip_top = geom.text_top + lh;
        let strip_lh = lh + geom.header_gap;
        if py < strip_top || py >= strip_top + strip_lh {
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
                    s.push_str(super::strip_gap());
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
        // V7 TASTE-GATE — one trailing INLINE-SHORTCUT string per PLAN line
        // (already `INLINE_SHORTCUT_GAP`-prefixed; empty = none / a header). Non-empty
        // ONLY under `HugText` bars with a right column, where each item's shortcut
        // rides its own name line so the bar hugs `label + gap + shortcut`. `&[]`
        // otherwise — byte-identical (no trailing spans).
        trailing: &[String],
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
                strip_s.push_str(super::strip_gap());
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

        // FIRST PASS at full BODY size. Then the strip's RESPONSIVE FOLD: at a
        // narrow window the full-size lens strip (Time Register …) can overflow the
        // card's text column — measured from the SHAPED line (real advances, not
        // the mean estimate), the whole strip steps down in size just enough to
        // fit, so every lens stays present + hit-testable instead of the far
        // right clipping away. At any comfortable width the measured strip fits
        // and the single full-size pass stands (byte-identical wide captures).
        // CHIP-VARIATIONS PROBE — the `FilledActive` chip is a SOLID fill, so its
        // active label INVERTS to the card ground (`base_300`) to read on the fill;
        // every other facet style keeps the active label in content ink.
        let active_ink = match crate::render::effective_facet_style() {
            theme::FacetStyle::Chips(theme::ChipVariant::FilledActive) => {
                theme::base_300().to_glyphon()
            }
            _ => ink,
        };
        self.shape_theme_spans(geom, ink, active_ink, muted, selected_ink, &strip_s, &label_ranges, &sep_ranges, trailing, 1.0);
        let strip_w = self.theme_strip_px();
        if strip_w > geom.text_w {
            let scale = (geom.text_w / strip_w).max(0.5);
            self.shape_theme_spans(geom, ink, active_ink, muted, selected_ink, &strip_s, &label_ranges, &sep_ranges, trailing, scale);
        }

        // Record the active-lens mark from the shaped strip glyphs (line 1). Line-1
        // glyphs are byte-indexed WITHIN the strip line's own text — the leading "\n" in
        // `strip_s` split the lines — so a label's line-relative range is its `strip_s`
        // range shifted back by that one "\n" byte. The MARK'S SHAPE is the
        // PER-ITEM LIST SURFACES round's `facet_style`:
        //   - `Text`   (default, byte-identical) — a hairline UNDERLINE under the
        //     active label.
        //   - `Band`   — a rounded value PILL behind the active label (the killed
        //     `Chips` skin's ghost-pill-per-label was dropped; only the active
        //     mark draws).
        // The x-spans come from the SAME shaped glyphs the strip hit-test reads, so
        // the skin can never disagree with where a label is clicked.
        let lh = self.overlay_lh();
        // Scan line 1 for a strip-range's glyph x-span (min_x, max_x) + the shaped
        // baseline (C2 y-owner), `None` if empty.
        let span_of = |buf: &GlyphBuffer, r: &std::ops::Range<usize>| -> Option<(f32, f32, f32)> {
            let (a, b) = (r.start.saturating_sub(1), r.end.saturating_sub(1));
            let mut min_x = f32::MAX;
            let mut max_x = f32::MIN;
            // Y-OWNER FIX (COMPOSITION-C2): the underline y must be read from the
            // strip run's SHAPED BASELINE, never a fixed `2*lh` formula. The strip
            // row is inflated by `header_gap` (a taller line box), and the label
            // may shape at a display/mono CHROME face whose baseline sits high in
            // that box — so `text_top + 2*lh - 3` landed MID-GLYPH (the underline
            // struck through "File" on Tawny/Firetail). `run.line_y` is the real
            // baseline in buffer space (same `geom.text_top + run.line_*` mapping
            // the primary/secondary columns use); the underline sits a hair BELOW
            // it for every face. The strip's responsive fold reshapes into the
            // same `panel_buffer`, so this reads the FINAL (possibly scaled) run.
            let mut baseline = f32::MIN;
            for run in buf.layout_runs() {
                if run.line_i != 1 {
                    continue;
                }
                baseline = baseline.max(run.line_y);
                for g in run.glyphs.iter() {
                    if g.start >= a && g.start < b {
                        min_x = min_x.min(g.x);
                        max_x = max_x.max(g.x + g.w);
                    }
                }
            }
            (max_x > min_x && baseline > f32::MIN).then_some((min_x, max_x, baseline))
        };
        let facet_style = crate::render::effective_facet_style();
        // Horizontal + vertical pad the active `Band` pill holds around its label.
        const CHIP_HPAD: f32 = 6.0;
        const CHIP_VPAD: f32 = 2.0;
        // VERTICAL PLACEMENT (the misaligned-chip refit): the strip line's height is
        // inflated to `strip_lh = lh + header_gap` by the query BEAT, and cosmic-text
        // CENTERS the glyphs in that tall line box — so the labels sit near the box's
        // vertical middle, well BELOW a plain `lh` band at line 1. The old pill top
        // (`text_top + lh + CHIP_VPAD`) tracked that plain band, so the pills floated
        // ABOVE the labels; the widened beat made it glaring. A facet mark that HUGS
        // its label must center on the GLYPH center: `line-1 top (== lh) + strip_lh/2`.
        // `chip_h` is sized off the strip's own gap-independent text line height (not
        // `lh`, which swells with a Bars row-gap), so the pill hugs the label the same
        // whether or not Bars is also active.
        let strip_lh = lh + geom.header_gap;
        let mark_cy = geom.text_top + lh + strip_lh * 0.5;
        let strip_text_lh = self.metrics.line_height * crate::render::effective_overlay_scale();
        let chip_h = (strip_text_lh - 2.0 * CHIP_VPAD).max(1.0);
        // A PILL rect from an already-resolved (left, right) glyph-x pair (device
        // px): centered on the strip glyphs' vertical middle. The active band
        // builds through this — one owner, so shape + geometry can't drift.
        let pill_px = |left: f32, right: f32| -> [f32; 4] {
            [
                geom.text_left + left,
                mark_cy - chip_h * 0.5,
                (right - left).max(1.0),
                chip_h,
            ]
        };
        // CHIP-VARIATIONS PROBE — the ACTIVE-label corner TICKS for
        // `ChipVariant::Bracket` (no closed box; small L-marks at each corner of the
        // label pill box). Eight thin FILLED rects, drawn by `overlay_facet_ghost`
        // as fills (stroke 0) — the ghost pipeline is otherwise idle under Bracket.
        let corner_ticks = |l: f32, r: f32| -> Vec<[f32; 4]> {
            const TICK: f32 = 6.0; // arm length
            const TH: f32 = 1.6; // arm thickness
            let top = mark_cy - chip_h * 0.5;
            let bot = mark_cy + chip_h * 0.5;
            let x0 = geom.text_left + l;
            let x1 = geom.text_left + r;
            vec![
                [x0, top, TICK, TH],
                [x0, top, TH, TICK], // TL
                [x1 - TICK, top, TICK, TH],
                [x1 - TH, top, TH, TICK], // TR
                [x0, bot - TH, TICK, TH],
                [x0, bot - TICK, TH, TICK], // BL
                [x1 - TICK, bot - TH, TICK, TH],
                [x1 - TH, bot - TICK, TH, TICK], // BR
            ]
        };
        // The active mark rect (single-rect skins) + the ghost/tick collection. THE
        // ONE owner: every skin below reads the SAME shaped glyph spans the strip
        // hit-test does, so the mark can never disagree with where a label is clicked.
        let mut ghosts: Vec<[f32; 4]> = Vec::new();
        // The inactive ghost PILLS, shared by every chip skin that outlines its
        // inactive labels (Hairline only, of the four). Filled per skin in overlay.rs.
        let inactive_pills = || -> Vec<[f32; 4]> {
            let mut v = Vec::new();
            for (r, active) in &label_ranges {
                if *active {
                    continue;
                }
                if let Some((min_x, max_x, _)) = span_of(&self.panel_buffer, r) {
                    v.push(pill_px(min_x - CHIP_HPAD, max_x + CHIP_HPAD));
                }
            }
            v
        };
        self.overlay_theme_underline = active_range.as_ref().and_then(|ar| {
            let (min_x, max_x, baseline) = span_of(&self.panel_buffer, ar)?;
            match facet_style {
                theme::FacetStyle::Text => {
                    // Y-OWNER (COMPOSITION-C2): a hairline just UNDER the active label,
                    // read from the strip run's SHAPED baseline + a small drop — never
                    // a fixed `2*lh` formula that struck mid-glyph on the taller CHROME
                    // faces. The strip's responsive fold reshapes into `panel_buffer`,
                    // so `baseline` is the FINAL (possibly scaled) run's baseline.
                    let y = geom.text_top + baseline + UNDERLINE_BASELINE_DROP;
                    Some([geom.text_left + min_x, y, max_x - min_x, 1.5])
                }
                // A single active BAND is a FILLED value pill hugging the label.
                theme::FacetStyle::Band => Some(pill_px(min_x - CHIP_HPAD, max_x + CHIP_HPAD)),
                theme::FacetStyle::Chips(v) => match v {
                    // HAIRLINE (baseline, ships Galah) + FILLED-ACTIVE (ships
                    // Firetail) — both a single pill hugging the active label; the
                    // FILL / STROKE / colour per skin is set in `prepare_overlay`.
                    theme::ChipVariant::Hairline | theme::ChipVariant::FilledActive => {
                        // Ghost pills only for HAIRLINE (it outlines inactive labels;
                        // FilledActive leaves them bare).
                        if matches!(v, theme::ChipVariant::Hairline) {
                            ghosts = inactive_pills();
                        }
                        Some(pill_px(min_x - CHIP_HPAD, max_x + CHIP_HPAD))
                    }
                    // UNDERLINE-CHIP (ships Magpie) — no box; a THICK SHORT bar hugging
                    // the label width, sitting just under the baseline. No inactive marks.
                    theme::ChipVariant::Underline => {
                        let y = geom.text_top + baseline + UNDERLINE_BASELINE_DROP;
                        Some([geom.text_left + min_x, y, max_x - min_x, 3.5])
                    }
                    // BRACKET (ships Mangrove) — no box; corner ticks around the active
                    // label, routed through the (otherwise idle) ghost pipeline as fills.
                    theme::ChipVariant::Bracket => {
                        ghosts = corner_ticks(min_x - CHIP_HPAD, max_x + CHIP_HPAD);
                        None
                    }
                },
            }
        });
        // Cleared for `Text`/`Band` (byte-identical); carries the inactive ghost
        // pills or the active corner ticks under the chip skins that draw them.
        self.overlay_theme_facet_ghosts = ghosts;
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
        // CHIP-VARIATIONS PROBE — the ACTIVE lens label's ink, split from `ink` so
        // `FacetStyle::Chips(FilledActive)` can invert it to the card ground while
        // every other style passes `ink` through (byte-identical).
        active_ink: glyphon::Color,
        muted: glyphon::Color,
        selected_ink: Option<glyphon::Color>,
        strip_s: &str,
        label_ranges: &[(std::ops::Range<usize>, bool)],
        sep_ranges: &[std::ops::Range<usize>],
        // V7 TASTE-GATE — trailing inline shortcut per PLAN line (see
        // `overlay_shape_theme`); `&[]` on a non-hug frame.
        trailing: &[String],
        strip_scale: f32,
    ) {
        let m = self.metrics;
        let faint = theme::faint().to_glyphon();
        let label = crate::markdown::type_scale::LABEL;
        // Per-line font sizes ride the overlay UI base (`OVERLAY_UI_SCALE`), and their
        // LINE HEIGHTS stay the uniform UI row height (`overlay_lh`) so the plan line
        // offsets, the selected band, and the underline `y` never drift from a per-span
        // metric taller than the row.
        let ui = crate::render::effective_overlay_scale();
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
        // WILD-MENU SLANT PROBE (env-gated; zero tax on every normal run —
        // byte-identical): the deepest display line's stair offset shrinks the
        // effective row span BEFORE the rowlayout budget, so elision respects
        // the reduced width — the same rule the flat shaper applies (see
        // `overlay_shape_text`'s slant note).
        let slant = crate::render::overlay_slant();
        let slant_tax = slant
            .map(|s| crate::render::slant_max_offset(&s, geom.plan.len()))
            .unwrap_or(0.0);
        let total_chars = if m.char_width > 0.0 {
            (((geom.text_w - slant_tax).max(0.0)) / m.char_width).floor() as usize
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
        // the flat shaper uses, so the two inline paths cannot diverge; the NARROW
        // FOLD (`geom.card_narrow`) brings the prefix back when the poster folds.
        let title_prefix = self.overlay_title_prefix(geom);
        let mut spans: Vec<(&str, glyphon::Attrs)> = Vec::new();
        // The "<title> › " prefix is CHROME — the chrome face (== the body
        // face on every `ChromeFace::Body` world, byte-identical today); the
        // bare sigil + the query text keep the body face (input, not chrome).
        // Mirrors the flat shaper's own split exactly.
        if title_prefix.is_empty() {
            spans.push((sigil, mk(muted)));
        } else {
            spans.push((title_prefix.as_str(), chrome_attrs().color(muted)));
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
                pushes.push((r.clone(), if *active { active_ink } else { muted }));
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
                // The lens-STRIP labels (+ their separators) are CHROME — the
                // third and last surface of the closed `ChromeFace` set
                // (placard / title prefix / strip). `chrome_attrs` ==
                // `panel_attrs` on every Body world, byte-identical today.
                spans.push((
                    &strip_s[r],
                    chrome_attrs().color(c).metrics(GlyphMetrics::new(fs, strip_lh)),
                ));
            }
        }
        // Plan lines: faint uppercase section headers (LABEL size) + world rows (ink).
        // On a true 1-bit world the SELECTED item's own glyphs recolor to the solid
        // contrasting ink (`selected_ink`) so black text lands crisp on the white
        // band — the same crisp black-on-white the flat pickers get, one rule (see
        // `HighlightTreatment::InverseFill`). Byte-identical (`None`) elsewhere.
        // WILD-MENU SLANT PROBE, italic half — row NAMES only, mirroring the
        // flat shaper's `rk` exactly (headers/strip/query never slant).
        let slant_italic = slant.map(|s| s.italic).unwrap_or(false);
        let rk = |c| {
            if slant_italic {
                mk(c).style(glyphon::cosmic_text::Style::Italic)
            } else {
                mk(c)
            }
        };
        for (idx, (line, fit)) in geom.plan.iter().zip(fitted.iter()).enumerate() {
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
                    spans.push((fit.as_deref().unwrap_or(""), rk(c)));
                    // V7 TASTE-GATE — the trailing INLINE SHORTCUT (HugText bars),
                    // muted, on the SAME item line so the bar hugs label + gap +
                    // shortcut. Symbol-split so ⌘ ⇧ ⌥ ⌃ shape from the bundled face.
                    if let Some(t) = trailing.get(idx).filter(|t| !t.is_empty()) {
                        let mut last = 0usize;
                        for sr in symbol_runs(t) {
                            if sr.start > last {
                                spans.push((&t[last..sr.start], mk(muted)));
                            }
                            let end = sr.end;
                            spans.push((&t[sr], sym(muted)));
                            last = end;
                        }
                        if last < t.len() {
                            spans.push((&t[last..], mk(muted)));
                        }
                    }
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
            // The compact foot-hint through the ONE shared owner — IDENTICAL bottom
            // geometry to the flat pickers (C2 footer-drift fix; the theme/faceted
            // path used to draw this at FULL row height, a fat lip under the hint).
            self.push_overlay_hint_spans(&mut spans, geom.hint.as_str(), muted);
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
