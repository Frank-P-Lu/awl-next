//! OVERLAY TEXT SHAPING — the summoned overlay card's name/right-column shaping and
//! the shaped-pixel no-overlap arbiter ([`rowlayout`]). Split out of the overlay
//! geometry/draw owner ([`super::overlay`]) so each file stays cohesive; the two
//! share [`OverlayGeom`] + [`TextPipeline::overlay_geometry`]. Carved out of
//! `chrome.rs` verbatim, no behaviour change. See [`super`].

use super::*;

/// Breathing inset (px) between the card's own edge and a
/// [`theme::TitleStyle::Placard`] wordmark's glyph box — mirrors the card's
/// own `pad` (12.0, `overlay_geometry`) so the wordmark sits inside the same
/// margin every other card element does.
const PLACARD_INSET: f32 = 12.0;

/// Pure corner placement: the wordmark's `(x, y)` top-left, given its own
/// shaped `(w, h)` and the card rect `(x, y, w, h)`. `TR`/`BR` anchor the
/// wordmark's RIGHT edge to the card's right edge (never past `card_x`, so a
/// wordmark wider than the card degrades to hugging the LEFT edge rather
/// than reporting a negative origin); `BL`/`BR` anchor the BOTTOM edge
/// likewise.
fn placard_origin(
    corner: theme::PlacardCorner,
    card: (f32, f32, f32, f32),
    w: f32,
    h: f32,
    inset: f32,
) -> (f32, f32) {
    let (cx, cy, cw, ch) = card;
    let x = match corner {
        theme::PlacardCorner::TL | theme::PlacardCorner::BL => cx + inset,
        theme::PlacardCorner::TR | theme::PlacardCorner::BR => (cx + cw - inset - w).max(cx),
    };
    let y = match corner {
        theme::PlacardCorner::TL | theme::PlacardCorner::TR => cy + inset,
        theme::PlacardCorner::BL | theme::PlacardCorner::BR => (cy + ch - inset - h).max(cy),
    };
    (x, y)
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
    /// wordmark reads with ONE number, never a second magic constant.
    /// Uppercased (a taste call, flagged — a display wordmark reads as a
    /// title card, not running prose).
    ///
    /// Returns the wordmark's natural `(x, y, w, h)` draw rect, or `None`
    /// when this frame draws no placard: the active [`theme::TitleStyle`]
    /// (probe-forced or the active world's own, see
    /// `render::effective_title_style`) is `InlinePrefix` (every world
    /// today), the picker is the theme picker (its own separate shaper,
    /// `theme_picker.rs`) or the header-less spell popup (no title line at
    /// all), or the kind draws no title (Rename/InsertLink — `overlay_title`
    /// is already empty for those). The CALLER clips the upload to the
    /// CARD's rect regardless of this fn's returned `w`/`h` — CLIPPED TO THE
    /// CARD, never bleeding into the scrim (a deliberate, logged deviation
    /// from Persona 3 Reload's own bleed: the card is the ONE region every
    /// other overlay element already reasons about, so a placard that could
    /// paint over the document/scrim would need its own separate clip/z-order
    /// story for no gain a calm dim wordmark behind the rows doesn't already
    /// deliver — rows/text always composite OVER it, legibility first).
    pub(in crate::render) fn overlay_shape_placard(&mut self, geom: &OverlayGeom) -> Option<(f32, f32, f32, f32)> {
        if geom.theme || geom.header_rows == 0 || self.overlay_title.is_empty() {
            return None;
        }
        let (corner, scale, ink) = match crate::render::effective_title_style() {
            theme::TitleStyle::Placard { corner, scale, ink } => (corner, scale, ink),
            theme::TitleStyle::InlinePrefix => return None,
        };
        let font_size = self.metrics.font_size * crate::markdown::type_scale::TITLE * scale;
        // A generous plain leading — no body text ever sits inside a
        // single-line wordmark box to match against.
        let line_height = font_size * 1.1;
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
        let mut w = 0.0f32;
        for run in self.placard_buffer.layout_runs() {
            w = w.max(run.line_w);
        }
        if w <= 0.0 {
            return None;
        }
        let card = (geom.card_x, geom.card_y, geom.card_w, geom.card_h);
        let (x, y) = placard_origin(corner, card, w, line_height, PLACARD_INSET);
        Some((x, y, w, line_height))
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
    pub(super) fn overlay_shape_text(
        &mut self,
        geom: &OverlayGeom,
        ink: glyphon::Color,
        muted: glyphon::Color,
    ) -> bool {
        // THEME PICKER: the faceted lens strip + section-grouped world rows lay out
        // differently from the flat pickers — its own shaper (which also records the
        // active-lens underline rect). No right column (returns false).
        self.overlay_right_shown = false;
        if geom.theme {
            return self.overlay_shape_theme(geom, ink, muted);
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
        // One line per name row: a `\n`-prefixed label leaves line 0 (the query row)
        // empty and puts label N on candidate row N; the hint row (if any) stays empty.
        let bind_strs: Vec<String> = (0..visible)
            .map(|row| {
                let label = right_labels.get(top_idx + row).map(|s| s.as_str()).unwrap_or("");
                format!("\n{label}")
            })
            .collect();

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
        // palette into another picker always says where you landed.
        let title_prefix = if self.overlay_title.is_empty() {
            String::new()
        } else {
            format!("{} › ", self.overlay_title)
        };
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
