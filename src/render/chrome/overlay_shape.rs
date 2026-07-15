//! OVERLAY TEXT SHAPING — the summoned overlay card's name/right-column shaping and
//! the shaped-pixel no-overlap arbiter ([`rowlayout`]). Split out of the overlay
//! geometry/draw owner ([`super::overlay`]) so each file stays cohesive; the two
//! share [`OverlayGeom`] + [`TextPipeline::overlay_geometry`]. Carved out of
//! `chrome.rs` verbatim, no behaviour change. See [`super`].

use super::*;

/// Breathing inset (px) between the anchor rect's own edge and a
/// [`theme::TitleStyle::Placard`] wordmark's glyph box — mirrors the card's
/// own `pad` (12.0, `overlay_geometry`) so the wordmark sits inside the same
/// margin every other element does.
const PLACARD_INSET: f32 = 12.0;

/// The glyph-coverage cut for a STIPPLE placard ([`theme::PlacardInk::Stipple`]):
/// a rasterized wordmark pixel joins the stipple's candidate set iff its swash
/// coverage clears this (≥ 50%). A HARD threshold, deliberately — the stipple's
/// whole contract is "individual full-ink pixels or nothing" (Bayer-legal by
/// construction, like the Wagtail highlight stipple), so the glyph's
/// antialiased fringe is CUT rather than half-drawn.
const STIPPLE_COVERAGE_THRESHOLD: u8 = 0x80;

/// Pure corner placement: the wordmark's `(x, y)` top-left, given its own
/// shaped `(w, h)` and the ANCHOR rect `(x, y, w, h)` (the full canvas — see
/// `overlay_shape_placard`). Each axis clamps BOTH bounds, symmetrically: the
/// anchored edge sits one `inset` in from its anchor edge; the OPPOSITE bound
/// clamps first (a too-wide/too-tall mark degrades to hugging the far edge
/// flush, dropping that side's inset); the anchored bound clamps last (never
/// past the anchor's own origin, so a mark wider than the whole anchor pins
/// to the near edge rather than reporting a negative origin). The audit-found
/// minimum-window overflow lived in the OLD asymmetry here: `TR`/`BR`
/// carried the `.max(ax)` guard while `BL`/`TL` had no `.min(...)` — a
/// LEFT-anchored mark's RIGHT bound was unprotected, and every shipped
/// placard is BL. (In practice `overlay_shape_placard`'s fit-to-canvas
/// shrink keeps `w` inside the anchor, so these clamps are the float-noise
/// backstop, not the primary mechanism.)
fn placard_origin(
    corner: theme::PlacardCorner,
    anchor: (f32, f32, f32, f32),
    w: f32,
    h: f32,
    inset: f32,
) -> (f32, f32) {
    let (ax, ay, aw, ah) = anchor;
    let x = match corner {
        theme::PlacardCorner::TL | theme::PlacardCorner::BL => {
            (ax + inset).min((ax + aw - w).max(ax))
        }
        theme::PlacardCorner::TR | theme::PlacardCorner::BR => (ax + aw - inset - w).max(ax),
    };
    let y = match corner {
        theme::PlacardCorner::TL | theme::PlacardCorner::TR => {
            (ay + inset).min((ay + ah - h).max(ay))
        }
        theme::PlacardCorner::BL | theme::PlacardCorner::BR => (ay + ah - inset - h).max(ay),
    };
    (x, y)
}

/// The widest laid-out run (px) of a just-shaped buffer — the wordmark's
/// natural width. Shared by [`TextPipeline::overlay_shape_placard`]'s two
/// measure points (natural, then post-shrink) so they can never disagree.
fn widest_run(buffer: &GlyphBuffer) -> f32 {
    let mut w = 0.0f32;
    for run in buffer.layout_runs() {
        w = w.max(run.line_w);
    }
    w
}

/// Build the RIGHT-column text lines for [`TextPipeline::shape_overlay_right`]:
/// one `\n`-prefixed line per candidate DISPLAY line, so label N lands on the
/// display row N of the candidate area. The FIRST line carries `header_rows`
/// leading newlines — the empties for the query line (every picker) plus the
/// lens STRIP above the candidate area on a faceted card (`header_rows == 2`)
/// — every later line carries one; an empty (`""`) label yields an empty,
/// non-binding line, which is how a faceted picker's section-HEADER row gets no
/// chord. ONE owner shared by the flat ([`TextPipeline::overlay_shape_text`])
/// and faceted ([`TextPipeline::shape_faceted`]) paths so their two alignments
/// can never drift (`same behavior ⇒ same code`); the flat path passes
/// `header_rows == 1`, reproducing the historical single leading `\n`
/// byte-for-byte.
fn right_bind_lines<'a>(header_rows: usize, labels: impl Iterator<Item = &'a str>) -> Vec<String> {
    labels
        .enumerate()
        .map(|(k, label)| {
            let leads = if k == 0 { header_rows.max(1) } else { 1 };
            format!("{}{label}", "\n".repeat(leads))
        })
        .collect()
}

impl TextPipeline {
    /// THE PLACARD RENDERER — the one owner of [`theme::TitleStyle::Placard`].
    /// Shapes the picker's own title text (`overlay_title`, the ONE owner of
    /// the announced text — see `OverlayKind::title`'s doc; already gated
    /// empty for the two kinds that orient via their own modal prompt
    /// instead) as a large, corner-anchored, DIM wordmark into
    /// `placard_buffer` — sized by `scale` over the document body's own font
    /// size × the markdown heading TITLE rung
    /// (`markdown::type_scale::TITLE`), so a world dials how loud its
    /// wordmark reads with ONE number, never a second magic constant — and
    /// CAPPED by the canvas itself (the fit-to-canvas shrink below): the
    /// window's own width is the ceiling the dial can never shout past.
    /// Uppercased (a taste call, flagged — a display wordmark reads as a
    /// title card, not running prose).
    ///
    /// Returns the wordmark's natural `(x, y, w, h)` draw rect, or `None`
    /// when this frame draws no placard: the active [`theme::TitleStyle`]
    /// (probe-forced or the active world's own, see
    /// `render::effective_title_style`) is `InlinePrefix` (every world
    /// today), the picker is the header-less spell popup (no title line at
    /// all — `header_rows == 0`), or the kind draws no title (Rename/
    /// InsertLink — `overlay_title` is already empty for those).
    ///
    /// THE SCREEN-CORNER ANCHOR (settled — supersedes the card-clipped
    /// original): the wordmark anchors to the FULL CANVAS corners and draws
    /// as a dim watermark OVER the scrim, BEHIND the card (the Persona-style
    /// bleed the card-clip original deliberately declined). The caller clips
    /// the upload to the WHOLE CANVAS (not the tighter card rect), and the
    /// wordmark's `TextArea` is still uploaded FIRST in the text batch, so
    /// the rows/query line always composite OVER it — legibility first, and
    /// the dimmed document below still shows through (the wordmark rides the
    /// text pass, above the scrim quad).
    ///
    /// COMPOSES WITH THE FACETED LAYOUT (fixed post-launch — a prior round's
    /// guard also bailed on `geom.theme`, blanking the placard on every
    /// picker [`crate::facets::scheme`] facets — the Cmd-P palette and the
    /// Settings menu included, the two surfaces that matter most): there is
    /// nothing kind-specific about this fn's OWN work — it anchors to the
    /// CANVAS (`self.window_w`/`self.window_h`, identical on both
    /// `overlay_geometry`'s flat branch and `theme_overlay_geometry`'s
    /// faceted branch) and reads only `geom.header_rows` +
    /// `self.overlay_title`/`self.placard_buffer`. The faceted shaper
    /// (`theme_picker.rs::overlay_shape_theme`) fills the SAME
    /// `panel_buffer` the flat shaper does, and both are uploaded through the
    /// SAME `overlay_upload_text` (`overlay.rs`) which always pushes the
    /// placard's `TextArea` FIRST (drawn behind) — so a faceted card's lens
    /// strip + section-grouped rows composite OVER the wordmark exactly like
    /// a flat card's query line + rows do, no new wiring needed. This
    /// includes the LITERAL Theme kind itself: nothing in `theme_picker.rs`
    /// depends on the card being placard-free (no state it reads or writes
    /// changes), so excluding it once the mechanism composes for free would
    /// just be an inconsistent special case — the exact smell
    /// `CLAUDE.md`'s "merge, don't align" principle warns against.
    pub(in crate::render) fn overlay_shape_placard(&mut self, geom: &OverlayGeom) -> Option<(f32, f32, f32, f32)> {
        if geom.header_rows == 0 || self.overlay_title.is_empty() {
            return None;
        }
        let (corner, scale, ink) = match crate::render::effective_title_style() {
            theme::TitleStyle::Placard { corner, scale, ink } => (corner, scale, ink),
            theme::TitleStyle::InlinePrefix => return None,
        };
        let font_size = self.metrics.font_size * crate::markdown::type_scale::TITLE * scale;
        // A generous plain leading — no body text ever sits inside a
        // single-line wordmark box to match against.
        let mut line_height = font_size * 1.1;
        let metrics = GlyphMetrics::new(font_size, line_height);
        self.placard_buffer.set_metrics(&mut self.font_system, metrics);
        self.placard_buffer.set_size(&mut self.font_system, None, None);
        self.placard_buffer.set_wrap(&mut self.font_system, Wrap::None);
        let text = self.overlay_title.to_uppercase();
        let color = theme::placard_ink(ink).to_glyphon();
        self.placard_buffer.set_text(
            &mut self.font_system,
            &text,
            &panel_attrs().color(color),
            Shaping::Advanced,
            None,
        );
        self.placard_buffer
            .shape_until_scroll(&mut self.font_system, false);
        let mut w = widest_run(&self.placard_buffer);
        if w <= 0.0 {
            return None;
        }
        // ANCHOR TO THE FULL CANVAS corners (a dim screen-corner watermark),
        // NOT the centered card rect. DECISION: the TOP corners respect the
        // menubar reserve (`0.0` unless the web/Linux bar is shown) so a shown
        // bar — which draws LAST, straight over the top of the canvas — never
        // overpaints the wordmark; the bottom edge uses the full window
        // height. On macbook/capture (bar off) `reserve == 0.0`, so the anchor
        // is the plain (0, 0, window_w, window_h) canvas.
        let reserve = self.menubar_reserve();
        let anchor = (0.0, reserve, self.window_w, self.window_h - reserve);
        // FIT THE CANVAS (the minimum-window overflow fix — found live by the
        // standing-policy audit): `scale` is a per-world LOUDNESS dial, not a
        // fit guarantee — a long title ("version history") at the app's own
        // enforced minimum window shapes ~2.6x wider than the whole canvas
        // and hard-clipped off the right edge. When the natural width exceeds
        // the anchor minus BOTH insets, shrink the font size proportionally
        // and re-lay out: cosmic-text shapes normalized (per-em) advances and
        // multiplies by the buffer metrics' font size at LAYOUT time, so ONE
        // linear re-metric lands the width at the target (residual float
        // noise is absorbed by `placard_origin`'s clamps). A comfortable
        // window never enters this branch — byte-identical. An ADAPTIVE
        // policy with no config knob, the `adaptive_column_left` idiom; the
        // stipple rasterizer reads the same re-shaped buffer, so it fits for
        // free.
        let avail = anchor.2 - 2.0 * PLACARD_INSET;
        if avail > 0.0 && w > avail {
            let shrink = avail / w;
            line_height *= shrink;
            self.placard_buffer.set_metrics(
                &mut self.font_system,
                GlyphMetrics::new(font_size * shrink, line_height),
            );
            self.placard_buffer
                .shape_until_scroll(&mut self.font_system, false);
            w = widest_run(&self.placard_buffer);
        }
        let (x, y) = placard_origin(corner, anchor, w, line_height, PLACARD_INSET);
        Some((x, y, w, line_height))
    }

    /// THE STIPPLE PLACARD's rasterizer: the coverage RUNS of the just-shaped
    /// `placard_buffer`'s glyphs, as 1px-tall rects positioned at the
    /// wordmark's draw origin — fed to the `placard_stipple` pipeline, whose
    /// dither branch then keeps only the Bayer-selected pixels (the SAME
    /// matrix + shader branch as the Wagtail highlight stipple — one pattern
    /// language, per the round's rule). CPU-rasterized off the SAME swash
    /// cache glyphon itself uses (the morph caret's established idiom —
    /// `render/caret.rs`'s mask rasterization), so the letterforms are the
    /// real shaped glyphs, deterministic across captures (no clock, no
    /// random: coverage is pure shaping, the Bayer cut is pure position).
    /// Emitting RUNS (not per-pixel rects) keeps the instance count at
    /// O(rows × glyphs), not O(pixels). Color-glyph (emoji) images are
    /// skipped — a wordmark title has none, and a coverage mask is the only
    /// content the stipple contract can honor.
    pub(in crate::render) fn placard_stipple_rects(&mut self, origin: (f32, f32)) -> Vec<[f32; 4]> {
        let (px, py) = origin;
        // Collect (cache_key, pen_x, baseline_y) first: `get_image` needs
        // `&mut font_system` while `layout_runs` borrows the buffer.
        let mut glyphs: Vec<(CacheKey, f32, f32)> = Vec::new();
        for run in self.placard_buffer.layout_runs() {
            let baseline_y = py + run.line_y;
            for g in run.glyphs.iter() {
                glyphs.push((g.physical((0.0, 0.0), 1.0).cache_key, px + g.x, baseline_y));
            }
        }
        let Self {
            swash_cache,
            font_system,
            ..
        } = self;
        let mut rects: Vec<[f32; 4]> = Vec::new();
        for (key, pen_x, baseline_y) in glyphs {
            let Some(img) = swash_cache.get_image(font_system, key).as_ref() else {
                continue;
            };
            if img.placement.width == 0
                || img.placement.height == 0
                || img.content != SwashContent::Mask
            {
                continue;
            }
            let gw = img.placement.width as usize;
            // Box top-left = (pen_x + placement.left, baseline - placement.top)
            // — the same placement convention the morph caret's masks use.
            let x0 = pen_x + img.placement.left as f32;
            let y0 = baseline_y - img.placement.top as f32;
            for (row, cols) in img.data.chunks_exact(gw).enumerate() {
                let y = y0 + row as f32;
                let mut start: Option<usize> = None;
                for (col, &alpha) in cols.iter().enumerate() {
                    match (alpha >= STIPPLE_COVERAGE_THRESHOLD, start) {
                        (true, None) => start = Some(col),
                        (false, Some(s)) => {
                            rects.push([x0 + s as f32, y, (col - s) as f32, 1.0]);
                            start = None;
                        }
                        _ => {}
                    }
                }
                if let Some(s) = start {
                    rects.push([x0 + s as f32, y, (gw - s) as f32, 1.0]);
                }
            }
        }
        rects
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
    pub(in crate::render) fn overlay_shape_text(
        &mut self,
        geom: &OverlayGeom,
        ink: glyphon::Color,
        muted: glyphon::Color,
    ) -> bool {
        // FACETED (lens-strip) pickers — the theme worlds AND the Cmd-P command
        // palette / Settings / Browse / … once a lens strip is populated — lay out
        // differently from the flat pickers: a section-grouped name column (its own
        // shaper, which also records the active-lens underline rect) PLUS, when the
        // picker fills a right column (chords / times / git), that column aligned to
        // the plan's item rows. `shape_faceted` owns both halves and returns whether
        // a right column was built.
        self.overlay_right_shown = false;
        if geom.theme {
            return self.shape_faceted(geom, ink, muted);
        }
        let visible = geom.visible;
        let top_idx = geom.top_idx;

        // The dim RIGHT-aligned column: command-palette key chords (`bindings`), the
        // go-to picker's relative "last edited" labels (`times`), OR the Project /
        // Browse pickers' per-row `"git"` repo tag (`git`). Only one is ever populated,
        // so prefer bindings, then times, then git. It is drawn FLUSH at the card's
        // right text edge by a SEPARATE buffer laid out with cosmic-text `Align::Right`,
        // so the column is a clean right edge regardless of the proportional name width.
        let right_labels: &[String] = if !self.overlay_bindings.is_empty() {
            &self.overlay_bindings
        } else if !self.overlay_times.is_empty() {
            &self.overlay_times
        } else {
            &self.overlay_git
        };
        let has_right = !right_labels.is_empty();
        // One line per name row, aligned to the candidate rows through the shared
        // `right_bind_lines` owner: the flat card's ONE header line (the `› query`
        // row, `header_rows == 1`) stays empty and label N lands on candidate row N;
        // the hint row (if any) stays empty.
        let bind_strs = right_bind_lines(
            geom.header_rows,
            (0..visible).map(|row| {
                right_labels.get(top_idx + row).map(|s| s.as_str()).unwrap_or("")
            }),
        );

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

    /// FACETED (lens-strip) card shaping: the section-grouped NAME column
    /// ([`Self::overlay_shape_theme`], which also records the active-lens
    /// underline), then — REUSING the SAME right-column owner the flat path uses
    /// ([`Self::shape_overlay_right`], not a copy) — the dim RIGHT column
    /// (command-palette chords / go-to "last edited" times / Browse·Project git
    /// tags), its lines offset to line up with the plan's ITEM rows. Returns
    /// whether a right column was built (so the caller uploads its text area).
    ///
    /// THE ROW MODEL (the alignment crux — got exactly right, verified by a
    /// capture): a faceted card has TWO header rows (query line 0 + lens STRIP
    /// line 1, `geom.header_rows == 2`), and its candidate area is the DISPLAY
    /// PLAN — section HEADERS ([`ThemeLine::Header`], present under a real lens
    /// where `overlay_sections` is populated) interleaved with world/command
    /// ROWS ([`ThemeLine::Item`]). So the bind column is built by walking the
    /// plan one display line at a time via the shared [`right_bind_lines`]: an
    /// `Item(i)` gets item `i`'s label (the absolute item index the plan carries,
    /// NOT a windowed offset), a `Header` gets an EMPTY line (a header is not a
    /// binding row), and the FIRST line carries `header_rows` leading newlines so
    /// the plan begins on display line 2. Both buffers share the overlay UI row
    /// height ([`Self::overlay_lh`]), so bind line N sits on the same y as name
    /// line N.
    ///
    /// THE LITERAL Theme picker (Switch theme…) has empty bindings/times/git →
    /// `has_right` false → an early `false` return with NO bind buffer built, so
    /// it renders byte-identically. Only the faceted pickers that populate a right
    /// column get one.
    fn shape_faceted(
        &mut self,
        geom: &OverlayGeom,
        ink: glyphon::Color,
        muted: glyphon::Color,
    ) -> bool {
        // The section-grouped name column + the active-lens underline (unchanged).
        self.overlay_shape_theme(geom, ink, muted);
        // The dim RIGHT column: the SAME precedence the flat path uses (bindings →
        // times → git; only one is ever populated). Empty on the literal Theme
        // picker → no right column, byte-identical.
        let right_labels: &[String] = if !self.overlay_bindings.is_empty() {
            &self.overlay_bindings
        } else if !self.overlay_times.is_empty() {
            &self.overlay_times
        } else {
            &self.overlay_git
        };
        if right_labels.is_empty() {
            return false;
        }
        // One bind line per DISPLAY line of the plan, aligned to the ITEM rows: a
        // header line gets an empty label, an item line gets its own item's label,
        // and the first line pads by `header_rows` (query + strip) so the plan
        // begins on display line 2.
        let bind_strs = right_bind_lines(
            geom.header_rows,
            geom.plan.iter().map(|line| match line {
                ThemeLine::Item(i) => {
                    right_labels.get(*i).map(|s| s.as_str()).unwrap_or("")
                }
                ThemeLine::Header(_) => "",
            }),
        );
        self.shape_overlay_right(geom, ink, muted, &bind_strs);
        self.overlay_right_shown = true;
        true
    }

    /// The inline `"<title> › "` query-line prefix, or an EMPTY string when
    /// the bare `› ` sigil should show instead. ONE owner, shared by the flat
    /// ([`Self::shape_overlay_names`]) and faceted
    /// ([`Self::overlay_shape_theme`]) shapers so the two inline sites can
    /// never diverge (`same behavior ⇒ same code`). Empty when:
    /// - this picker draws no title (`overlay_title` empty — Rename/InsertLink
    ///   orient via their own modal prompt), OR
    /// - the active [`theme::TitleStyle`] is a `Placard`: the corner wordmark
    ///   already announces the picker, so the inline prefix must NOT ALSO fire
    ///   (both firing was the reported double-title bug). `InlinePrefix` (the
    ///   default on every world) keeps the prefix — byte-identical to before.
    pub(super) fn overlay_title_prefix(&self) -> String {
        let placard = matches!(
            crate::render::effective_title_style(),
            theme::TitleStyle::Placard { .. }
        );
        if self.overlay_title.is_empty() || placard {
            String::new()
        } else {
            format!("{} › ", self.overlay_title)
        }
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
        // The query line occupies text line 0 when present; the spell panel skips it
        // so its first suggestion IS line 0. THE OVERLAY-TITLES ROUND: a picker that
        // draws its title (`overlay_title` nonempty — every kind except Rename/
        // InsertLink, which already orient via their own modal prompt) prepends it,
        // muted, before the `› ` sigil — "<title> › query", so routing from the
        // palette into another picker always says where you landed. SUPPRESSED under
        // a `Placard` title style (the corner wordmark already names the picker) —
        // `overlay_title_prefix` owns that ONE rule for both inline sites.
        let title_prefix = self.overlay_title_prefix();
        let sigil = "› ";
        if has_query {
            if title_prefix.is_empty() {
                spans.push((sigil, mk(muted)));
            } else {
                spans.push((title_prefix.as_str(), mk(muted)));
            }
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
        // EMPTY STATE: with no candidate rows, one dim, non-selectable message row
        // (styled like the foot hint) sits in the candidate area — the shared calm
        // "no matches" / "no suggestions" / … from `geom.empty`. A query line pushes
        // it to its own line below; the spell popup (no query line) puts it on line 0.
        if let Some(msg) = &geom.empty {
            if has_query {
                spans.push(("\n", mk(muted)));
            }
            spans.push((msg.as_str(), mk(muted)));
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
        // KEYBINDINGS TIPS FOOTER: the quiet "your top 3" band below the hint (chrome,
        // like the hint line — NOT selectable rows). Each tip a FAINT line (fainter than
        // the muted hint, so it's the quietest thing on the card), prefixed by a blank
        // separator so it reads as its own band. Built up front so the shaped spans can
        // borrow it past `set_rich_text` (like `hint_line`). Its chord glyphs (⌘ ⇧ …)
        // ride the SYMBOL_FAMILY face (the same `sym` split the hint uses), so a
        // "⌘O  Go to file" tip renders the glyph rather than tofu.
        let footer_lines: Vec<String> = geom.footer.iter().map(|t| format!("\n{t}")).collect();
        if geom.footer_rows > 0 {
            let faint = theme::faint().to_glyphon();
            spans.push(("\n", mk(faint))); // the blank separator line
            for line in &footer_lines {
                let mut last = 0usize;
                for run in symbol_runs(line) {
                    if run.start > last {
                        spans.push((&line[last..run.start], mk(faint)));
                    }
                    let end = run.end;
                    spans.push((&line[run], sym(faint)));
                    last = end;
                }
                if last < line.len() {
                    spans.push((&line[last..], mk(faint)));
                }
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
}
