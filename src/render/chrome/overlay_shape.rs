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

/// PROPORTIONAL PLACARD SIZING (flip-round spec, user "yeah sounds good") — the
/// reference canvas SHORT side the wordmark fraction is calibrated against (the
/// capture's 1200×800 canvas → 800). The wordmark HEIGHT is
/// `clamp(floor, scale · PLACARD_HEIGHT_PER_SCALE · window_short_side, ceiling)`
/// — WINDOW-scaled (chrome is Frame, like the page column), never ZOOM-scaled
/// (the old `metrics.font_size · TITLE · scale` rode zoom). At this reference
/// short side the fraction reproduces exactly today's `FONT_SIZE · TITLE · scale`
/// look, so every default-zoom capture (window 1200×800, zoom = dpi = 1) is
/// byte-identical to before the round.
const PLACARD_REFERENCE_SHORT_SIDE: f32 = crate::capture::CANVAS_HEIGHT as f32;

/// PROPORTIONAL PLACARD SIZING — the wordmark height as a fraction of the window
/// short side PER UNIT of a world's `scale` dial, chosen so that at the reference
/// short side ([`PLACARD_REFERENCE_SHORT_SIDE`]) the height equals the old
/// `FONT_SIZE · TITLE · scale`. Derivation: `FONT_SIZE · TITLE / REFERENCE` =
/// `24 · 1.8 / 800` = `0.054`. So Firetail (`scale` 4.5) reproduces at `24.3%` of
/// the short side, the `scale` 3.0 posters at `16.2%` — the board's "~20%" band.
const PLACARD_HEIGHT_PER_SCALE: f32 =
    crate::render::FONT_SIZE * crate::markdown::type_scale::TITLE / PLACARD_REFERENCE_SHORT_SIDE;

/// PROPORTIONAL PLACARD SIZING — the clamp FLOOR (px): a wordmark never shapes
/// smaller than this however small the window (below the card's narrow-fallback
/// point it FOLDS to InlinePrefix entirely — see `OverlayGeom::card_narrow` — so
/// this floor only bites in the mid band above the fold). Comfortably below every
/// shipped world's reference height (`scale` 3.0 → 129.6, 4.5 → 194.4), so the
/// clamp is inert at the reference canvas.
const PLACARD_MIN_HEIGHT: f32 = 56.0;

/// PROPORTIONAL PLACARD SIZING — the clamp CEILING (px): a wordmark never shapes
/// larger than this however large the window (a 4K/5K short side would otherwise
/// drive a `scale` 4.5 poster past ~500 px). Comfortably above every shipped
/// world's reference height, so the clamp is inert at the reference canvas.
const PLACARD_MAX_HEIGHT: f32 = 512.0;

/// PLACARD ATLAS-SAFETY (AtlasFull fix, 2026-07-17) — the geometric step the
/// wordmark's font size is SNAPPED to. The proportional sizing above tracks the
/// window short side CONTINUOUSLY, so a live resize sweep asked the shaper for a
/// fresh giant Archivo-Black size every pixel of drag travel; each distinct size
/// rasterizes its own glyph set into the ONE shared glyph atlas
/// (`TextPipeline::atlas` — document body text, rows and placard all live there),
/// and a fast sweep on a large display filled it faster than the per-frame
/// `atlas.trim()` reclaimed → [`glyphon::PrepareError::AtlasFull`], which blanked
/// the card AND starved the document text sharing the atlas. Snapping to a ~3%
/// ladder bounds the whole clamp band [`PLACARD_MIN_HEIGHT`]..[`PLACARD_MAX_HEIGHT`]
/// to ≈ `log(512/56)/log(1.03) ≈ 75` distinct rungs — a wordmark this large moves
/// a pixel or two between rungs, imperceptible, while the atlas stays bounded no
/// matter how long the drag. 3–4% was the board's call; 1.03 sits at the calm end.
const PLACARD_SIZE_STEP: f32 = 1.03;

/// Snap a placard font target (px) to the geometric ladder of
/// [`PLACARD_SIZE_STEP`], anchored at `anchor` (the world's REFERENCE-canvas
/// size — so the 1200×800 reference short side is an EXACT fixed point and every
/// default-zoom placard capture stays byte-identical). `round_down` FLOORS to the
/// ladder (used by the shrink-to-fit path, so the snapped size never exceeds the
/// fit target and the wordmark still fits the canvas); otherwise rounds to the
/// nearest rung. Because BOTH the main and shrink paths anchor at the same
/// `anchor`, every size either path can produce is `anchor · step^k` for integer
/// `k` — ONE ladder, so the two paths' union stays bounded (never a product). Pure
/// → the bounded-ladder law is unit-testable without a GPU (see
/// `render::tests::overlay_personality`).
pub(in crate::render) fn snap_placard_size(target: f32, anchor: f32, round_down: bool) -> f32 {
    if !(target > 0.0) || !(anchor > 0.0) {
        return target;
    }
    let steps = (target / anchor).ln() / PLACARD_SIZE_STEP.ln();
    let k = if round_down { steps.floor() } else { steps.round() };
    anchor * (k * PLACARD_SIZE_STEP.ln()).exp()
}

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
    // `Auto` is resolved to a concrete corner by `derived_placard_corner` before
    // this pure placer runs; the arms below fall it back to LEFT/BOTTOM defensively.
    let x = match corner {
        theme::PlacardCorner::TL | theme::PlacardCorner::BL | theme::PlacardCorner::Auto => {
            (ax + inset).min((ax + aw - w).max(ax))
        }
        theme::PlacardCorner::TR | theme::PlacardCorner::BR => (ax + aw - inset - w).max(ax),
    };
    let y = match corner {
        theme::PlacardCorner::TL | theme::PlacardCorner::TR => {
            (ay + inset).min((ay + ah - h).max(ay))
        }
        theme::PlacardCorner::BL | theme::PlacardCorner::BR | theme::PlacardCorner::Auto => {
            (ay + ah - inset - h).max(ay)
        }
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
        // item 4 (NARROW FOLD): below the card's narrow-fallback regime the poster
        // FOLDS to `InlinePrefix` — no partial/clipped wordmark at any width. The
        // ONE owner (`OverlayGeom::card_narrow`, computed via
        // `overlay_card_fill_regime` from the SAME geometry the card width fallback
        // reads) is shared with the inline-prefix reader (`overlay_title_prefix`),
        // so exactly one of the poster wordmark / inline `title › ` prefix ever
        // fires. Zero placard pixels in the narrowest cells (the width-sweep law).
        if geom.card_narrow {
            return None;
        }
        // Resolve an `Auto` corner COMPLEMENTARY to the card anchor (the ONE pure
        // owner) so the wordmark lands opposite the command surface, never under it.
        let corner = crate::render::derived_placard_corner(corner, crate::render::effective_card_anchor());
        // PROPORTIONAL PLACARD SIZING (flip-round spec): the wordmark tracks the
        // WINDOW short side (chrome is Frame — the page-column philosophy), NEVER
        // the zoom-scaled `metrics.font_size` the old formula rode. `scale` stays
        // the per-world LOUDNESS dial; the fraction is calibrated
        // ([`PLACARD_HEIGHT_PER_SCALE`]) so that at the 1200×800 reference canvas
        // (zoom = dpi = 1) this equals the old `metrics.font_size · TITLE · scale`
        // exactly → every default capture is byte-identical. Clamped floor..ceiling
        // so a tiny window never shrinks it to nothing (it FOLDS first, above) and a
        // 4K/5K window never blows it up. `line_height = font_size · 1.1` below, so
        // this drives the whole box.
        let short_side = self.window_w.min(self.window_h);
        // The world's REFERENCE-canvas height (short side == PLACARD_REFERENCE_SHORT_SIDE):
        // the anchor the size ladder is pinned to, so the reference canvas is an exact
        // fixed point (byte-identical there) and both this main size and the shrink-to-fit
        // size below land on the SAME ladder (see `snap_placard_size`).
        let reference_size = scale * PLACARD_HEIGHT_PER_SCALE * PLACARD_REFERENCE_SHORT_SIDE;
        // ATLAS-SAFETY: snap the continuous window-tracked size to the ladder BEFORE the
        // clamp, so a live resize sweep produces a BOUNDED set of distinct giant sizes
        // (never a fresh atlas entry per drag pixel — the AtlasFull fix).
        let font_size = snap_placard_size(scale * PLACARD_HEIGHT_PER_SCALE * short_side, reference_size, false)
            .clamp(PLACARD_MIN_HEIGHT, PLACARD_MAX_HEIGHT);
        // A generous plain leading — no body text ever sits inside a
        // single-line wordmark box to match against.
        let mut line_height = font_size * 1.1;
        let metrics = GlyphMetrics::new(font_size, line_height);
        self.placard_buffer.set_metrics(&mut self.font_system, metrics);
        self.placard_buffer.set_size(&mut self.font_system, None, None);
        self.placard_buffer.set_wrap(&mut self.font_system, Wrap::None);
        let text = self.overlay_title.to_uppercase();
        let color = theme::placard_ink(ink).to_glyphon();
        // The wordmark is CHROME (the frame around the list, never the list),
        // so it shapes in the world's chrome face — `chrome_attrs` is
        // `panel_attrs` verbatim on every `ChromeFace::Body` world (all of
        // them today), and swaps only under a `Named` face / the
        // `AWL_CHROME_FACE_FORCE` audition probe.
        self.placard_buffer.set_text(
            &mut self.font_system,
            &text,
            &chrome_attrs().color(color),
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
            // ATLAS-SAFETY: snap the fit target DOWN to the same ladder the main size
            // rode. Flooring guarantees the shrunk mark still fits `avail` (the snapped
            // size never exceeds `font_size · avail/w`), and anchoring at the same
            // `reference_size` keeps every width-only-resize shrink size on the ONE
            // bounded ladder — so a horizontal drag of a long title can't fill the atlas
            // either (the width-sweep the main clamp alone never covered).
            let shrunk =
                snap_placard_size(font_size * (avail / w), reference_size, true);
            line_height = shrunk * 1.1;
            self.placard_buffer.set_metrics(
                &mut self.font_system,
                GlyphMetrics::new(shrunk, line_height),
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
        // The SELECTED row's own glyph color on a true 1-bit world
        // (`HighlightTreatment::InverseFill` — solid `base_300` so black text
        // lands crisply on the white band). `None` on every ordinary world,
        // where the selected row keeps its content ink and the shaper is
        // byte-identical to before.
        selected_ink: Option<glyphon::Color>,
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
            return self.shape_faceted(geom, ink, muted, selected_ink);
        }
        let visible = geom.visible;
        let top_idx = geom.top_idx;

        // The dim RIGHT-aligned column: command-palette key chords (`bindings`), the
        // go-to picker's relative "last edited" labels (`times`), OR the Project /
        // Browse pickers' per-row `"git"` repo tag (`git`). Only one is ever populated,
        // so prefer bindings, then times, then git. It is drawn FLUSH at the card's
        // right text edge by a SEPARATE buffer laid out with cosmic-text `Align::Right`,
        // so the column is a clean right edge regardless of the proportional name width.
        let right_labels = self.overlay_right_labels();
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
        //
        // WILD-MENU SLANT PROBE (env-gated; `slant_tax == 0.0` on every normal
        // run — byte-identical): the deepest row's stair offset is subtracted
        // from the effective row span BEFORE the rowlayout math, so elision
        // respects the reduced width (a shifted row can never paint past the
        // card's right text edge). Rows still flow through `rowlayout` — the
        // law is untouched; only the width it budgets against shrinks.
        let slant = crate::render::overlay_slant();
        let slant_tax = slant
            .map(|s| crate::render::slant_max_offset(&s, visible))
            .unwrap_or(0.0);
        let slant_text_w = (geom.text_w - slant_tax).max(0.0);
        let m = self.metrics;
        let total_chars = if m.char_width > 0.0 {
            (slant_text_w / m.char_width).floor() as usize
        } else {
            usize::MAX
        };
        // V7 TASTE-GATE — TEXT-HUGGING (`HugText`) bars with a right column: the
        // shortcut rides its own name line (trailing the label, `INLINE_SHORTCUT_GAP`
        // between) so EVERY row's bar hugs its own content; the ragged right derives
        // from content length alone. The names shape at FULL budget (the pane is
        // dropped under bars — no right column to reserve against), and the separate
        // right-aligned column is NOT drawn (`overlay_right_shown` stays false). The
        // `HugLabel` HYBRID does NOT take this path — its chord stays in the
        // right-aligned column below, so the plate hugs the label ALONE.
        if has_right && super::bars_inline_shortcut() {
            let full = rowlayout::full_budget(total_chars);
            let rows: Vec<String> = (0..visible)
                .map(|row| rowlayout::fit_primary(&self.overlay_items[top_idx + row], full))
                .collect();
            let trailing: Vec<String> = (0..visible)
                .map(|row| match right_labels.get(top_idx + row) {
                    Some(s) if !s.is_empty() => format!("{}{}", super::INLINE_SHORTCUT_GAP, s),
                    _ => String::new(),
                })
                .collect();
            self.shape_overlay_names(geom, ink, muted, selected_ink, &rows, &trailing);
            return false;
        }
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
        self.shape_overlay_names(geom, ink, muted, selected_ink, &rows, &[]);
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
        if rowlayout::fits(slant_text_w, gap_px, name_px, right_px) {
            self.overlay_right_shown = true;
            return true;
        }
        let full = rowlayout::full_budget(total_chars);
        let rows: Vec<String> = (0..visible)
            .map(|row| rowlayout::fit_primary(&self.overlay_items[top_idx + row], full))
            .collect();
        self.shape_overlay_names(geom, ink, muted, selected_ink, &rows, &[]);
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
        selected_ink: Option<glyphon::Color>,
    ) -> bool {
        // The dim RIGHT column through the SAME one-owner precedence the flat path
        // reads (bindings → times → git; only one is ever populated). Empty on the
        // literal Theme picker → no right column, byte-identical.
        let right_labels = self.overlay_right_labels();
        let has_right = !right_labels.is_empty();
        // V7 TASTE-GATE — under `HugText` bars a right column rides INLINE on each
        // ITEM row (trailing the label, `INLINE_SHORTCUT_GAP` between) so the bar
        // hugs the whole content; the separate right-aligned column is dropped.
        // One trailing string per PLAN line (a header line gets none). Empty on a
        // non-`HugText` frame (incl. the `HugLabel` hybrid, whose chord stays in
        // the right column) or a picker with no right column → byte-identical.
        let hug_inline = has_right && super::bars_inline_shortcut();
        // Both the INLINE trailing (hug) and the right-aligned bind lines (non-hug)
        // are materialized into OWNED `Vec<String>`s up front, so the `right_labels`
        // borrow of `self` ends before the `&mut self` shaper calls below.
        let trailing: Vec<String> = if hug_inline {
            geom.plan
                .iter()
                .map(|line| match line {
                    ThemeLine::Item(i) => match right_labels.get(*i) {
                        Some(s) if !s.is_empty() => {
                            format!("{}{}", super::INLINE_SHORTCUT_GAP, s)
                        }
                        _ => String::new(),
                    },
                    ThemeLine::Header(_) => String::new(),
                })
                .collect()
        } else {
            Vec::new()
        };
        // One bind line per DISPLAY line of the plan, aligned to the ITEM rows: a
        // header line gets an empty label, an item line gets its own item's label,
        // and the first line pads by `header_rows` (query + strip) so the plan
        // begins on display line 2. Empty under hug (the shortcut rides inline).
        let bind_strs: Vec<String> = if has_right && !hug_inline {
            right_bind_lines(
                geom.header_rows,
                geom.plan.iter().map(|line| match line {
                    ThemeLine::Item(i) => {
                        right_labels.get(*i).map(|s| s.as_str()).unwrap_or("")
                    }
                    ThemeLine::Header(_) => "",
                }),
            )
        } else {
            Vec::new()
        };
        // The section-grouped name column + the active-lens underline (unchanged,
        // save the inline shortcuts composed onto the ITEM rows under hug bars).
        self.overlay_shape_theme(geom, ink, muted, selected_ink, &trailing);
        if !has_right || hug_inline {
            return false;
        }
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
    /// - the active [`theme::TitleStyle`] is a `Placard` AND the placard is NOT
    ///   folded (the corner wordmark already announces the picker, so the inline
    ///   prefix must NOT ALSO fire — both firing was the reported double-title
    ///   bug). `InlinePrefix` (the default on every world) keeps the prefix —
    ///   byte-identical to before.
    ///
    /// item 4 (NARROW FOLD) — when the card is in its narrow-fallback regime
    /// (`geom.card_narrow`, the SAME owner the placard shaper folds on), a
    /// `Placard` world FOLDS to `InlinePrefix`: the wordmark is suppressed there,
    /// so the prefix RETURNS here. Exactly one of the two fires at any width.
    pub(super) fn overlay_title_prefix(&self, geom: &OverlayGeom) -> String {
        let placard_drawn = matches!(
            crate::render::effective_title_style(),
            theme::TitleStyle::Placard { .. }
        ) && !geom.card_narrow;
        if self.overlay_title.is_empty() || placard_drawn {
            String::new()
        } else {
            format!("{} › ", self.overlay_title)
        }
    }

    /// ONE OWNER of the summoned card's FOOT-HINT spans (the "↑/↓ move …" control
    /// row) — appends the break-`\n` (at the prior row's NORMAL height) then the hint
    /// TEXT on a SHORTER line ([`Self::overlay_hint_h`]) at the LABEL rung, keycap
    /// glyphs (↵ ⇥ ⌘ …) split onto the SYMBOL_FAMILY face. Shared by the flat
    /// ([`Self::shape_overlay_names`]) and faceted/theme
    /// ([`Self::shape_theme_spans`]) shapers so EVERY `OverlayKind`'s footer carries
    /// IDENTICAL bottom geometry (the C2 footer-drift fix — before this the theme /
    /// faceted path drew the hint at FULL row height while the flat path drew it
    /// compact, so the card's bottom pad differed per kind). The card-height owners
    /// ([`overlay_geometry`] / [`theme_geometry`]) reclaim `lh - hint_h` per hint row
    /// to match this compact strip exactly.
    pub(super) fn push_overlay_hint_spans<'a>(
        &self,
        spans: &mut Vec<(&'a str, glyphon::Attrs<'a>)>,
        hint: &'a str,
        muted: glyphon::Color,
    ) {
        let name_fs = self.overlay_metrics().font_size;
        let hint_fs = name_fs * crate::markdown::type_scale::LABEL;
        let hint_h = self.overlay_hint_h();
        let base = panel_attrs();
        let hk_hint = |c| base.clone().color(c).metrics(GlyphMetrics::new(hint_fs, hint_h));
        let sym_hint = |c| {
            Attrs::new()
                .family(Family::Name(SYMBOL_FAMILY))
                .color(c)
                .metrics(GlyphMetrics::new(hint_fs, hint_h))
        };
        // Break the last content line at its OWN (normal) height first.
        spans.push(("\n", base.clone().color(muted)));
        // The compact foot hint through the ONE symbol-split owner (⌘ ⇧ ⌥ ⌃ ↵ ⇥
        // ride SYMBOL_FAMILY, the rest the chrome face) at the LABEL rung.
        push_symbol_split(spans, hint, || hk_hint(muted), || sym_hint(muted));
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
        selected_ink: Option<glyphon::Color>,
        rows: &[String],
        // V7 TASTE-GATE — one trailing INLINE-SHORTCUT string per candidate row
        // (already `INLINE_SHORTCUT_GAP`-prefixed; empty = none). Non-empty ONLY
        // under `HugText` bars with a right column, where the shortcut rides its
        // own name line (muted, symbol-split for modifier glyphs) instead of the
        // right-aligned column, so the bar hugs `label + gap + shortcut`. Pass
        // `&[]` everywhere else — byte-identical (no trailing spans).
        trailing: &[String],
    ) {
        // The flat/nav pickers show a `› query` line on top (`header_rows == 1`); the
        // contextual SPELL panel shows none (`0`) — just the suggestion rows.
        let has_query = geom.header_rows > 0;
        // Per-row colors: query full ink; candidate rows ink (selected) / muted.
        // Names/query/sigil render in the ACTIVE-WORLD face (`mk`).
        let base = panel_attrs();
        let mk = |c| base.clone().color(c);
        // Symbol-face attrs for the inline shortcut's modifier glyphs (⌘ ⇧ ⌥ ⌃) —
        // the same bundled `SYMBOL_FAMILY` the right-aligned column + hint use.
        let sym_name = |c| Attrs::new().family(Family::Name(SYMBOL_FAMILY)).color(c);
        let mut spans: Vec<(&str, glyphon::Attrs)> = Vec::new();
        // The query line occupies text line 0 when present; the spell panel skips it
        // so its first suggestion IS line 0. THE OVERLAY-TITLES ROUND: a picker that
        // draws its title (`overlay_title` nonempty — every kind except Rename/
        // InsertLink, which already orient via their own modal prompt) prepends it,
        // muted, before the `› ` sigil — "<title> › query", so routing from the
        // palette into another picker always says where you landed. SUPPRESSED under
        // a `Placard` title style (the corner wordmark already names the picker) —
        // `overlay_title_prefix` owns that ONE rule for both inline sites, and the
        // NARROW FOLD (`geom.card_narrow`) brings the prefix back when the poster folds.
        let title_prefix = self.overlay_title_prefix(geom);
        let sigil = "› ";
        // PALETTE-COMPOSITION round's HEADER GAP: inflate the query line's own
        // height by `header_gap`, so the extra negative space falls between the
        // query header and the first candidate row (the divider is negative
        // space, no drawn rule). The candidate rows keep their normal height and
        // the selected-row band folds the same gap in through `overlay_row_top`,
        // so the band still lands on each row. `hk` = the header spans' attrs
        // (taller line only when a gap is set). NOTE cosmic-text HALF-LEADS the
        // glyphs into this taller line — the query text sits `header_gap * 0.5`
        // BELOW the top, NOT pinned to it — so the amber caret centres on this
        // line's REAL shaped height (`overlay_place_caret` reads the run's own
        // `line_height`), never the bare `overlay_lh()`, or it floats a half-beat
        // above the text (the full-bleed caret bug).
        let name_fs = self.overlay_metrics().font_size;
        let header_lh = self.overlay_lh() + geom.header_gap;
        let hk = |c| {
            if geom.header_gap > 0.0 {
                mk(c).metrics(GlyphMetrics::new(name_fs, header_lh))
            } else {
                mk(c)
            }
        };
        // The "<title> › " prefix is CHROME (it names the picker), so it rides
        // the chrome face (`chrome_attrs` == `panel_attrs` on every Body world
        // — byte-identical today); the bare `› ` sigil and the query TEXT are
        // the input affordance, never chrome — they keep the body face.
        let hkc = |c| {
            let a = chrome_attrs().color(c);
            if geom.header_gap > 0.0 {
                a.metrics(GlyphMetrics::new(name_fs, header_lh))
            } else {
                a
            }
        };
        if has_query {
            if title_prefix.is_empty() {
                spans.push((sigil, hk(muted)));
            } else {
                spans.push((title_prefix.as_str(), hkc(muted)));
            }
            spans.push((self.overlay_query.as_str(), hk(ink)));
        }
        // Every row's FILENAME is the FIGURE: content ink at BODY size. Its leading
        // DIRECTORY (through the last `/`) recedes to MUTED ink (figure/ground by value)
        // so the eye lands on the file; a folder row (trailing `/`, no filename after it)
        // stays whole in content ink. The SELECTED row is marked by a surface VALUE BAND
        // (DESIGN §5), not a brighter name. A leading `\n` puts each name on its own row
        // BELOW the query line; without a query line (spell panel) row 0 sits on line 0.
        //
        // ONE EXCEPTION — a true 1-bit world (`selected_ink.is_some()`): the
        // SELECTED row's own glyphs (name AND its dir prefix) recolor to the
        // solid contrasting ink so black text lands crisp on the white band,
        // instead of the gamma-grey a framebuffer invert of the row produced
        // (see `HighlightTreatment::InverseFill`). `sel_vis` is the 0-based row
        // among those SHOWN, matching `overlay_draw_card`'s band placement.
        let sel_vis = self.overlay_selected.saturating_sub(geom.top_idx);
        // WILD-MENU SLANT PROBE, italic half (env-gated; `false` on every
        // normal run): the Persona-style italic on the row NAMES only — the
        // query/hint/chrome never slant. The face may not carry a true italic;
        // cosmic-text then matches the nearest style — acceptable for a
        // gallery probe (which faces carry real italics is a probe FINDING).
        let slant_italic =
            crate::render::overlay_slant().map(|s| s.italic).unwrap_or(false);
        let rk = |c| {
            if slant_italic {
                mk(c).style(glyphon::cosmic_text::Style::Italic)
            } else {
                mk(c)
            }
        };
        for (row, content) in rows.iter().enumerate() {
            if !(!has_query && row == 0) {
                spans.push(("\n", mk(ink)));
            }
            let (name_c, dir_c) = match selected_ink {
                Some(c) if row == sel_vis => (c, c),
                _ => (ink, muted),
            };
            let split = if content.ends_with('/') {
                0
            } else {
                crate::overlay::row_split(content)
            };
            if split > 0 {
                spans.push((&content[..split], rk(dir_c)));
            }
            spans.push((&content[split..], rk(name_c)));
            // V7 TASTE-GATE — the trailing INLINE SHORTCUT (HugText bars only),
            // muted, on the SAME name line so the bar hugs label + gap + shortcut.
            // Symbol-split so ⌘ ⇧ ⌥ ⌃ shape from the bundled face (real advances),
            // exactly like the right-aligned column + the foot hint.
            if let Some(t) = trailing.get(row).filter(|t| !t.is_empty()) {
                push_symbol_split(&mut spans, t, || mk(muted), || sym_name(muted));
            }
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
        // The quiet control-hint row, last. LIP FIX (item 5): a leading "\n"
        // breaks the last candidate line at its NORMAL height, then the hint
        // TEXT rides a SHORTER line ([`Self::overlay_hint_h`]) at the LABEL rung
        // — a compact footer that hugs the card's bottom edge instead of
        // floating a full row high (the ugly "lip"). Both geometry owners shrink
        // the card by `lh - overlay_hint_h()` so it fits this tighter strip
        // exactly. Its keycap glyphs (↵ ⇥ ⌘ … ) ride the SYMBOL_FAMILY face —
        // split into symbol / non-symbol runs exactly like the chord column — so
        // a hint that teaches a key with a glyph (`↵ restore`) renders it.
        if geom.hint_rows > 0 {
            // The compact foot-hint through the ONE shared owner (C2 footer-drift).
            self.push_overlay_hint_spans(&mut spans, geom.hint.as_str(), muted);
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
            let sym = |c| Attrs::new().family(Family::Name(SYMBOL_FAMILY)).color(c);
            spans.push(("\n", mk(faint))); // the blank separator line
            for line in &footer_lines {
                push_symbol_split(&mut spans, line, || mk(faint), || sym(faint));
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
        // SECONDARY-INK FLIP (Potoroo taste-gate defect — the primary flip landed
        // but the dim right column never followed, so the selected row's chord
        // hints washed into a saturated band, invisible). The SELECTED display
        // line's chord recolors to [`theme::selected_row_secondary_ink`] (the ONE
        // derive owner) when the band would drop `muted` below the contrast floor;
        // every OTHER line — and every world whose band already clears the floor —
        // stays `muted`, byte-identical. The selected line is the SAME index the
        // band uses ([`overlay_selected_display_line`]), so recolor and highlight
        // can never disagree.
        let sel_line = self.overlay_selected_display_line(geom);
        // The flip is CORRECT only when the chord actually sits ON the band. Under a
        // HUGGING plate (`HugLabel`) the bare right chord rides the GROUND, not the
        // plate, so contrasting the band drives it into the ground — the chord stays
        // `muted` there (legible, identical to the unselected rows). One owner:
        // `selected_secondary_on_band`.
        let sel_muted = if !super::selected_secondary_on_band() {
            None
        } else {
            let band = crate::render::effective_overlay_selrow_band();
            match theme::active().highlight_treatment(band) {
                theme::HighlightTreatment::InverseFill { ink, .. } => Some(ink.to_glyphon()),
                theme::HighlightTreatment::ValueBand(b) => {
                    let flipped = theme::selected_row_secondary_ink(b);
                    (flipped != theme::muted()).then(|| flipped.to_glyphon())
                }
            }
        };
        let mut bind_spans: Vec<(&str, glyphon::Attrs)> = Vec::new();
        for (li, s) in bind_strs.iter().enumerate() {
            let c = match (sel_line, sel_muted) {
                (Some(sl), Some(flip)) if sl == li => flip,
                _ => muted,
            };
            push_symbol_split(&mut bind_spans, s, || mono(c), || sym(c));
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

    /// V6 P5 [`theme::BarExtent::HugText`] — per DISPLAY-row PRIMARY text width
    /// (px), read from the just-shaped `panel_buffer`, keyed by display-row index
    /// (`line_i - header_rows`). A display row is a candidate line: for the flat
    /// pickers that is candidate `N`, for the theme picker it is plan line `N`
    /// (a section-header plan line is present too, but the bar draw only looks up
    /// the ITEM rows). The hug-extent bars read this so each bar's right edge
    /// hugs its own row's text — the SAME shaped glyphs the text draws from, so
    /// bar and glyph can't disagree.
    pub(in crate::render) fn overlay_row_primary_px(
        &self,
        geom: &OverlayGeom,
    ) -> std::collections::BTreeMap<usize, f32> {
        let mut m = std::collections::BTreeMap::new();
        for run in self.panel_buffer.layout_runs() {
            if run.line_i >= geom.header_rows {
                m.insert(run.line_i - geom.header_rows, run.line_w);
            }
        }
        m
    }

    /// V8 — the WIDEST shaped FOOTER content line (px) in the just-shaped
    /// `panel_buffer`: the dim foot-hint plus the keybindings-tips lines, which
    /// all sit BELOW the `content_rows` candidate/empty lines (`line_i >=
    /// header_rows + content_rows`). Read so the [`super::footer_plate_rect`] can
    /// HUG the footer text under [`theme::BarExtent::HugText`] instead of drawing
    /// a lone full-width plate under hugging rows — the SAME shaped glyphs the
    /// footer draws from, so plate and text can't disagree. `0.0` when there is no
    /// footer content (the plate then collapses to its 1px floor, never drawn).
    pub(in crate::render) fn overlay_footer_content_px(
        &self,
        geom: &OverlayGeom,
        content_rows: usize,
    ) -> f32 {
        let first = geom.header_rows + content_rows;
        let mut w = 0.0f32;
        for run in self.panel_buffer.layout_runs() {
            if run.line_i >= first {
                w = w.max(run.line_w);
            }
        }
        w
    }

}
