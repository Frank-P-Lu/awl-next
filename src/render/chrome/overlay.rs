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

/// EDGE-INSET token (device px): the calm margin the summoned card holds off the
/// window's edges when TOP-LEFT anchored, echoing the page column's own
/// left-margin rhythm so the card reads as *placed*, not stuck in the corner
/// (composition round item 2 — the old flush 12px hug was too tight for the
/// top-left anchor to read as deliberate). Collapses toward
/// [`CARD_EDGE_INSET_FLOOR`] as the window narrows, then the card re-centers and
/// fills (item 7) — see [`overlay_card_box_policy`].
pub(in crate::render) const CARD_EDGE_INSET: f32 = 28.0;
/// The smallest edge pad the card keeps as the window tightens (the narrow +
/// narrowest regimes of [`overlay_card_box_policy`]).
pub(in crate::render) const CARD_EDGE_INSET_FLOOR: f32 = 10.0;
/// The FLAT card's tightest WIDTH cap (device px) — the ONE width owner the
/// composition round tightened (item 3; the card used to sprawl to half the
/// window). A single dial the gallery A/Bs.
pub(in crate::render) const CARD_MAX_W: f32 = 520.0;
/// The FACETED card's width cap — a touch wider than the flat cap so the whole
/// lens strip (Time … All) never clips, still tighter than the old 0.58×window.
pub(in crate::render) const CARD_MAX_W_FACETED: f32 = 600.0;

/// The QUERY-INPUT BEAT (item 4), as a fraction of the overlay row height — the
/// clear breath between the input line and the first result row. A single dial
/// the gallery A/Bs; see [`TextPipeline::overlay_header_gap`].
///
/// REFIT (2026-07-16): the user found `0.72` still read cramped under the input
/// box on EVERY picker (Pane and Bars alike). Widened to a clearly-breathing
/// FULL row of air — the beat moves the candidate band AND the glyphs together
/// by construction (the shaper inflates the last header line's real metrics by
/// exactly this; the y-agreement law holds), so this is a pure taste dial with
/// no alignment risk. LIVE-ONLY: whether the fuller beat reads right needs an eye.
///
/// THE ROUND'S ONE SHIPPED VISUAL CHANGE. This `0.72 -> 1.0` widening is a
/// user-directed taste change that moves EVERY summoned picker's query line (and
/// the whole candidate stack below it) down a fraction vs the `main` base — so
/// byte-identity-vs-`main` is by design IMPOSSIBLE for any query-line surface,
/// and the Persona-list inert guarantee is scoped to self-consistency + the
/// model-level inert law instead (see `render/tests/list_surfaces.rs`'s module
/// doc). NOTE the caret's y is NOT derived from this constant: it reads the
/// query line's real shaped `line_height` (`overlay_place_caret`), so it tracks
/// the glyphs through cosmic-text's half-leading whatever this dial is set to
/// (the full-bleed caret bug this refit closed).
const OVERLAY_QUERY_BEAT: f32 = 1.0;

/// PER-ITEM LIST SURFACES round — the corner radius (device px) of the faceted
/// strip's active [`theme::FacetStyle::Band`] pill. A single dial the gallery A/Bs.
const FACET_CHIP_RADIUS: f32 = 6.0;

/// The foot HINT row height (item 5), as a fraction of the overlay row height —
/// a compact footer that hugs the card's bottom edge instead of floating a full
/// row high. A single dial the gallery A/Bs; see [`TextPipeline::overlay_hint_h`].
const OVERLAY_HINT_ROW: f32 = 0.62;

/// The comfortable BREATH kept below the compact foot-hint before the card's
/// bottom pad (C2 footer-tuning). The card counts each hint row as a full `lh`
/// but renders it at [`OVERLAY_HINT_ROW`]; the height owner reclaims that
/// difference LESS this breath, so the footer reads calm, never cramped against
/// the edge. ONE token, applied identically to every `OverlayKind` through
/// [`TextPipeline::overlay_footer_reclaim`].
const OVERLAY_FOOTER_PAD: f32 = 5.0;

/// PURE horizontal-placement policy for the summoned card: given the window
/// width `ww`, the card's WIDE desired width, return its `(left, width)`.
///
/// THREE REGIMES (the `adaptive_column` idiom, applied to the takeover card):
/// - WIDE — hold the desired width; sit one full [`CARD_EDGE_INSET`] in from the
///   anchored edge (item 2's page-margin rhythm).
/// - NARROW — the edge inset COLLAPSES toward [`CARD_EDGE_INSET_FLOOR`] so the
///   card keeps its width as the window tightens (it slides toward the edge
///   before it shrinks).
/// - NARROWEST — once even the floor can't seat the width, the card fills the
///   window minus a floor pad each side and RE-CENTERS (item 7). By construction
///   `left >= 0` and `left + width <= ww - floor` in every regime, so a card is
///   always fully on-canvas (the width-sweep law pins this).
pub(in crate::render) fn overlay_card_box_policy(
    anchor: theme::CardAnchor,
    ww: f32,
    desired_w: f32,
) -> (f32, f32) {
    let floor = CARD_EDGE_INSET_FLOOR;
    let full = CARD_EDGE_INSET;
    // Never wider than the window minus a floor pad each side (the fill ceiling).
    let cw = desired_w.min((ww - 2.0 * floor).max(0.0));
    let free = (ww - cw).max(0.0);
    // The anchored-edge inset never leaves less than `floor` on the far side.
    let anchored_max = (ww - floor - cw).max(floor);
    let left = match anchor {
        theme::CardAnchor::TopCenter => free * 0.5,
        // Full inset when there's room, collapsing to the floor as the window
        // tightens; re-centers (left == floor, symmetric) in the fill regime.
        theme::CardAnchor::TopLeft => full.min(anchored_max).max(floor).min(free),
        // The statement dial sweeps the RIGHT inset from full (x_frac 1.0) to the
        // left edge (0.0 == TopLeft), through the SAME collapse clamp.
        theme::CardAnchor::Inset { x_frac } => {
            let span = (ww - cw - 2.0 * full).max(0.0);
            (full + x_frac.clamp(0.0, 1.0) * span)
                .min(anchored_max)
                .max(floor)
                .min(free)
        }
        // RIGHT-ANCHOR MIRROR: PLACEMENT mirrors `Inset { x_frac: 1.0 }` — the
        // card's right edge one full inset in from the canvas right, collapsing
        // toward the floor as the window tightens (the mirror of `TopLeft`). The
        // MIRROR half (bar-growth direction) is a separate concern read via
        // `CardAnchor::mirrors_growth`, not a placement change.
        theme::CardAnchor::TopRight => {
            let span = (ww - cw - 2.0 * full).max(0.0);
            (full + span).min(anchored_max).max(floor).min(free)
        }
    };
    (left, cw)
}

/// Whether the summoned card is forced into its NARROWEST (fill) regime for a
/// WIDE desired width `desired_w` at window width `ww`: the window is too tight
/// to seat the card at even the floor inset each side, so
/// [`overlay_card_box_policy`] clamps the width below `desired_w` and re-centers.
///
/// THE ONE OWNER of the narrow-fallback test, shared two ways (item 4 — the
/// NARROW FOLD): the card LAYOUT enters fill exactly here, and a `Placard` title
/// FOLDS to the calm `InlinePrefix` here (the placard shaper returns `None`, the
/// inline `title › ` prefix comes back) so no partial/clipped poster wordmark
/// ever shows below the card's own fallback point. Reads the SAME
/// [`CARD_EDGE_INSET_FLOOR`] geometry the policy clamps against, so the fold
/// threshold and the width fallback can never drift.
pub(in crate::render) fn overlay_card_fill_regime(ww: f32, desired_w: f32) -> bool {
    desired_w > (ww - 2.0 * CARD_EDGE_INSET_FLOOR).max(0.0)
}

/// TEST-ONLY snapshot of every summoned-overlay row's Y, per element — the fixture
/// the y-agreement law reads (see [`TextPipeline::overlay_row_y_probe`]).
#[cfg(test)]
pub(in crate::render) struct OverlayYProbe {
    /// The overlay UI row height (device px).
    pub lh: f32,
    /// The selected row's band TOP, from the ONE forward owner `overlay_row_top`.
    pub band_top: f32,
    /// The selected row's 0-based DISPLAY index (band lands here).
    pub sel_disp: usize,
    /// The amber caret's query-line center, from `overlay_query_center`.
    pub caret_center: f32,
    /// The query line's own glyph TOP (absolute canvas y).
    pub query_line_top: f32,
    /// The query line's ACTUAL shaped height (its first run's `line_height`) —
    /// inflated by `header_gap` on the flat pickers, plain on the faceted ones.
    /// The caret centre must ride THIS, not `lh` (the full-bleed caret bug).
    pub query_line_height: f32,
    /// The query line's BASELINE (absolute canvas y) — an INDEPENDENT witness of
    /// where the glyphs sit, so the y-agreement law is not circular: the caret
    /// centre must sit a sane, constant offset above this, not a half-beat high.
    pub query_baseline: f32,
    /// Candidate-row index → the PRIMARY name's absolute glyph TOP.
    pub primary: std::collections::BTreeMap<usize, f32>,
    /// Candidate-row index → the SECONDARY label's absolute glyph TOP.
    pub secondary: std::collections::BTreeMap<usize, f32>,
    /// The faceting-lens STRIP run's shaped BASELINE (absolute canvas y), if a
    /// strip is present (faceted/theme cards). `None` on a flat picker.
    pub strip_baseline: Option<f32>,
    /// The strip line box's BOTTOM (absolute canvas y) — the underline must stay
    /// inside `[baseline, strip_line_bottom]`, never drift into the rows below.
    pub strip_line_bottom: Option<f32>,
    /// The active-lens UNDERLINE's y (`overlay_theme_underline`), if recorded.
    /// The C2 y-owner law asserts this sits BELOW the strip baseline (never
    /// mid-glyph — the Tawny/Firetail strike-through bug) yet within the row.
    pub strip_underline_y: Option<f32>,
}

impl TextPipeline {
    /// The ONE metric every overlay ROW shapes + measures at: the reading body stepped
    /// down by [`OVERLAY_UI_SCALE`]. [`Self::overlay_remetric`] sets the shared buffers
    /// to it, and [`Self::overlay_lh`] (its line-height half) is what every geometry
    /// reader shares — so shaping and geometry can never drift on the row size.
    pub(in crate::render) fn overlay_metrics(&self) -> GlyphMetrics {
        let m = self.metrics;
        GlyphMetrics::new(
            m.font_size * OVERLAY_UI_SCALE,
            m.line_height * OVERLAY_UI_SCALE + self.overlay_row_gap(),
        )
    }

    /// PER-ITEM LIST SURFACES round — the vertical GAP (device px) opened
    /// between candidate rows under [`theme::ListStyle::Bars`]; `0.0` under
    /// `Pane` (byte-identical). It is folded into the ONE overlay row-pitch
    /// owner [`Self::overlay_lh`] (and thus into `overlay_metrics`), so the card
    /// height, the shaped text spread, the selected band, and the pointer
    /// hit-test all widen the row pitch TOGETHER — bars and text can never
    /// disagree about a row's y (round A's y-agreement law holds by
    /// construction). The bar surfaces then draw `lh - gap` tall, leaving the
    /// gap as the space between them.
    pub(in crate::render) fn overlay_row_gap(&self) -> f32 {
        match crate::render::effective_list_style() {
            theme::ListStyle::Bars { gap, .. } => gap.max(0.0),
            theme::ListStyle::Pane => 0.0,
        }
    }

    /// PER-ITEM LIST SURFACES round — the horizontal inset (device px) the row
    /// TEXT column holds from the layout bound (`card_x` .. `card_x + card_w`).
    /// `Pane` keeps the historical `12` pad (byte-identical). `Bars` insets
    /// `BAR_SIDE_INSET + BAR_TEXT_PAD` so the glyphs sit a comfortable pad INSIDE
    /// each bar's edge (the user's "bar text needs real left padding" refit),
    /// symmetric so the secondary chord column mirrors it inside the bar's right
    /// edge. The ONE owner both `overlay_geometry` and `theme_overlay_geometry`
    /// read for `text_left`/`text_w`, so shaping, hit-test, caret, and the
    /// right-aligned chords all inset together.
    pub(in crate::render) fn overlay_text_hpad(&self) -> f32 {
        match crate::render::effective_list_style() {
            theme::ListStyle::Bars { .. } => BAR_SIDE_INSET + BAR_TEXT_PAD,
            theme::ListStyle::Pane => 12.0,
        }
    }

    /// The overlay row LINE HEIGHT — the single-owner metric the card height, the
    /// row-Y ([`overlay_row_top`]), the hit-test ([`overlay_row_of`]), and the
    /// selected-row band all read, so a click always lands on the row it highlights.
    pub(in crate::render) fn overlay_lh(&self) -> f32 {
        self.metrics.line_height * OVERLAY_UI_SCALE + self.overlay_row_gap()
    }

    /// THE ONE OWNER of the summoned takeover card's horizontal BOX — its
    /// `(left, width)`. Composes three things so the flat [`Self::overlay_geometry`]
    /// and faceted [`TextPipeline::theme_overlay_geometry`] can never disagree
    /// about where the card sits OR how wide it is:
    /// - the per-world ANCHOR ([`theme::CardAnchor`], via
    ///   [`crate::render::effective_card_anchor`] so the gallery probe A/Bs it);
    /// - the EDGE-INSET rhythm ([`CARD_EDGE_INSET`], item 2 — a real left margin
    ///   echoing the page column, not the old flush corner hug);
    /// - the NARROW-WINDOW fallback (item 7 — the inset collapses toward
    ///   [`CARD_EDGE_INSET_FLOOR`], then the card re-centers and fills), all in
    ///   the pure policy [`overlay_card_box_policy`].
    ///
    /// The caller passes the card's WIDE desired width (its own `CARD_MAX_W*`
    /// cap, item 3); the box narrows it only in the fill regime. The placard's
    /// own canvas-corner anchor is untouched; the contextual spell popup does
    /// NOT call this (it anchors at its word).
    pub(in crate::render) fn overlay_card_box(&self, width: u32, desired_w: f32) -> (f32, f32) {
        overlay_card_box_policy(crate::render::effective_card_anchor(), width as f32, desired_w)
    }

    /// THE QUERY-INPUT BEAT token (device px): the calm slab of negative space
    /// inserted after the header rows (query + optional lens strip) and before
    /// the candidate list, on the palette AND every faceted picker uniformly (the
    /// divider is negative space, never a drawn rule). Sized off the overlay row
    /// height so it scales with zoom/DPI like every other overlay metric.
    ///
    /// COMPOSITION ROUND (item 4) widened it from ~0.55 to [`OVERLAY_QUERY_BEAT`]
    /// of a row — a clearer beat between the input line and the first result,
    /// still short of the "fat lip" of a whole blank row. It is STRUCTURAL, not a
    /// leading newline (the f2cb656 tripwire): the shaper inflates the last
    /// header line's REAL glyph metrics by exactly this, and the band, primary
    /// name, secondary chord, hit-test, and caret all fold it in through the ONE
    /// y-owner family ([`overlay_row_top`] / [`overlay_secondary_top`]) — so text
    /// and band move together, never a half-row split. Both geometry owners read
    /// this; the contextual spell popup passes `0.0` (no header to divide from).
    /// LIVE-ONLY taste: whether the widened beat reads right needs a human eye.
    pub(in crate::render) fn overlay_header_gap(&self) -> f32 {
        (self.overlay_lh() * OVERLAY_QUERY_BEAT).round()
    }

    /// THE HINT-ROW HEIGHT (device px, item 5) — the foot hint reads as the
    /// card's own bottom EDGE, not a floating orphan row. The user's report ("i
    /// do see a lip, and its really ugly") was the full-height `lh` row the hint
    /// used to occupy: a `lh`-tall line with a small glyph at its top left a fat
    /// empty band below it before the pad, so the hint hovered above the card's
    /// bottom. This SHORTER line ([`OVERLAY_HINT_ROW`] of a row) hugs the hint
    /// tight under the last result; the shaper draws it at the LABEL rung, FAINT,
    /// and BOTH geometry owners shrink the card by `lh - overlay_hint_h()` per
    /// hint row so the card fits the tighter footer exactly (the
    /// card-fits-content law follows). Spell popup has no hint (`hint_rows == 0`).
    pub(in crate::render) fn overlay_hint_h(&self) -> f32 {
        (self.overlay_lh() * OVERLAY_HINT_ROW).round()
    }

    /// THE ONE FOOTER-PAD OWNER (C2) — the card-height reclaim for `hint_rows`
    /// compact foot-hint rows. Each hint row is budgeted as a full `lh` in a
    /// card's `total_rows`, but RENDERS at the shorter [`Self::overlay_hint_h`];
    /// the card reclaims that difference LESS one comfortable breath
    /// ([`OVERLAY_FOOTER_PAD`]) so the footer never crams against the bottom edge.
    /// BOTH card-height owners ([`Self::overlay_geometry`] and the theme
    /// [`Self::theme_geometry`]) call this, so every `OverlayKind` carries the
    /// IDENTICAL bottom geometry the card-fits-content law now asserts no-wildcard.
    pub(in crate::render) fn overlay_footer_reclaim(&self, hint_rows: usize) -> f32 {
        hint_rows as f32 * (self.overlay_lh() - self.overlay_hint_h() - OVERLAY_FOOTER_PAD).max(0.0)
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
        // variant's doc). On an ordinary (`ValueBand`) world the row normally
        // keeps its content ink (`None`, byte-identical), but when the value band
        // washes that ink out (`theme::selected_row_ink` — the InverseFill lesson
        // for fill worlds; Undertow-under-Bars was the 2.53:1 exhibit) the ONE
        // derive owner flips it to the reading pole and the shaper recolors the
        // selected row to match. Read from the SAME band `overlay_draw_card` fills.
        let sel_band = crate::render::effective_overlay_selrow_band();
        let selected_ink = match theme::active().highlight_treatment(sel_band) {
            theme::HighlightTreatment::InverseFill { ink, .. } => Some(ink.to_glyphon()),
            theme::HighlightTreatment::ValueBand(band) => {
                let flipped = theme::selected_row_ink(band);
                (flipped != theme::base_content()).then(|| flipped.to_glyphon())
            }
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
        // PER-ITEM LIST SURFACES: the bar surfaces park empty too, so a closed
        // picker carries no stale bar quads into the next frame.
        self.overlay_bars.prepare(device, queue, width, height, &[]);
        self.overlay_lens_underline
            .prepare(device, queue, width, height, &[]);
        // V6 P5: the Chips ghost pills park empty too, so a closed picker carries
        // no stale ghost-pill quads into the next frame.
        self.overlay_facet_ghost
            .prepare(device, queue, width, height, &[]);
        // The stipple placard: parked (zero instances) — the frame after a
        // stipple-world overlay closes carries zero stale wordmark pixels.
        self.placard_stipple
            .prepare(device, queue, width, height, &[]);
        // The Bars behind-the-bars placard pass: parked (no areas) so a closed
        // picker carries no stale wordmark into the next frame.
        self.placard_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                Vec::<TextArea>::new(),
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon placard park failed: {e:?}"))?;
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

    /// TEST HOOK — the Y-AGREEMENT probe: for the currently-prepared overlay, the
    /// absolute canvas Y-TOP of every candidate row's PRIMARY glyphs
    /// (`panel_buffer`) and SECONDARY label (`panel_bind_buffer`), keyed by the
    /// candidate row index, alongside the band top [`overlay_row_top`] draws and
    /// the caret's query-line center. The `primary`/`secondary` maps let the law
    /// assert that — per row — the name, the chord column, and the selected-row
    /// band all share ONE y (no element computes its own row y): the exact
    /// invariant the composition-round gap broke for the right column. Reads the
    /// SAME `overlay_geometry` + upload owners (`overlay_secondary_top`) the
    /// render path uses, so it can never claim an alignment the pixels don't show.
    #[cfg(test)]
    pub(in crate::render) fn overlay_row_y_probe(&self) -> OverlayYProbe {
        use std::collections::BTreeMap;
        let geom = self.overlay_geometry(self.window_w as u32);
        let lh = self.overlay_lh();
        let header_rows = geom.header_rows;
        let last = header_rows + if geom.theme { geom.plan.len() } else { geom.visible };
        // Primary rows: absolute top = card text origin + the run's own line_top
        // (the header-line inflation that carries the gap is baked INTO these
        // line_tops, so a primary row already sits on the band).
        let mut primary = BTreeMap::new();
        for run in self.panel_buffer.layout_runs() {
            let li = run.line_i;
            if li >= header_rows && li < last {
                primary.insert(li - header_rows, geom.text_top + run.line_top);
            }
        }
        // Secondary labels: absolute top = the shared upload owner
        // (`overlay_secondary_top`, text_top + the gap) + the run's line_top (the
        // right buffer is uniform-lh, its leading empties supply header_rows*lh).
        let sec_top = overlay_secondary_top(geom.text_top, geom.header_gap);
        let mut secondary = BTreeMap::new();
        for run in self.panel_bind_buffer.layout_runs() {
            let li = run.line_i;
            if li >= header_rows && li < last {
                secondary.insert(li - header_rows, sec_top + run.line_top);
            }
        }
        // The selected row's band top, from the ONE forward owner.
        let sel_disp = if geom.theme {
            geom.plan
                .iter()
                .position(|l| matches!(l, ThemeLine::Item(i) if *i == self.overlay_selected))
                .unwrap_or(0)
        } else {
            self.overlay_selected.saturating_sub(geom.top_idx)
        };
        let band_top = overlay_row_top(geom.text_top, header_rows, geom.header_gap, sel_disp, lh);
        // The strip line (line_i == 1) baseline + box bottom, so the C2 law can
        // assert the active-lens underline sits below the label glyphs, in-row.
        let mut strip_baseline = None;
        let mut strip_line_bottom = None;
        for run in self.panel_buffer.layout_runs() {
            if run.line_i == 1 {
                strip_baseline = Some(geom.text_top + run.line_y);
                strip_line_bottom = Some(geom.text_top + run.line_top + run.line_height);
                break;
            }
        }
        let strip_underline_y = self.overlay_theme_underline.map(|q| q[1]);
        // The query line's OWN shaped run — the SAME first run the render path
        // reads for the caret's y and x. Its `line_height` is inflated by
        // `header_gap` on the flat pickers; `line_y` is its baseline.
        let query_run = self.panel_buffer.layout_runs().next();
        let query_line_height = query_run
            .as_ref()
            .map(|r| r.line_height)
            .unwrap_or_else(|| self.overlay_lh());
        let query_line_top = query_run
            .as_ref()
            .map(|r| geom.text_top + r.line_top)
            .unwrap_or(geom.text_top);
        let query_baseline = query_run
            .as_ref()
            .map(|r| geom.text_top + r.line_y)
            .unwrap_or(geom.text_top);
        OverlayYProbe {
            lh,
            band_top,
            sel_disp,
            // Mirror the render path EXACTLY: the caret centres on the query
            // line's real shaped height, not the bare `lh`.
            caret_center: overlay_query_center(geom.text_top, query_line_height),
            query_line_top,
            query_line_height,
            query_baseline,
            primary,
            secondary,
            strip_baseline,
            strip_line_bottom,
            strip_underline_y,
        }
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
        // RESPONSIVE CARD via the ONE horizontal-box owner: the tightened flat
        // width cap ([`CARD_MAX_W`], item 3) placed with the edge-inset rhythm
        // (item 2) + the narrow-window collapse/fill fallback (item 7). The box
        // narrows the width only in the fill regime, so the text column can
        // never starve.
        let (card_x, card_w) = self.overlay_card_box(width, CARD_MAX_W);
        // item 4 (NARROW FOLD): the placard folds to InlinePrefix once even the
        // floor inset can't seat the flat card's desired width — the SAME owner
        // the width fallback above reads (`overlay_card_box`'s policy).
        let card_narrow = overlay_card_fill_regime(width as f32, CARD_MAX_W);
        // Horizontal text inset is list-style aware (`Bars` pads the glyphs inside
        // each bar's edge — the ONE owner `overlay_text_hpad`); vertical padding
        // stays `pad` (12) so the card height math is untouched. `Pane` keeps
        // `hpad == pad`, byte-identical.
        let hpad = self.overlay_text_hpad();
        let text_w = card_w - 2.0 * hpad;
        // The header gap adds to the card height alongside the row stack + padding,
        // so the card still FITS its content exactly (bottom padding == `pad`). The
        // foot hint (item 5) rides a SHORTER line, so reclaim `lh - hint_h` per
        // hint row — the card hugs the tighter footer instead of the old lip.
        let card_h = total_rows as f32 * self.overlay_lh() + header_gap + 2.0 * pad
            - self.overlay_footer_reclaim(hint_rows);
        // vertical anchor near the top third (summoned, transient).
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
        let text_left = card_x + hpad;
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
            card_narrow,
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
            // The contextual spell popup carries no title and never a placard, so
            // the fold flag is inert here.
            card_narrow: false,
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
        // DESIGNER PIXEL-PASS FIX (2026-07-16) — the placard's DRAW SLOT depends on
        // the list style. Under `Bars` it must sit BEHIND the bar quads, so it rides
        // its own `placard_renderer` pass (run between the room veil and the bars in
        // `draw_overlay_card`); under `Pane` it stays FIRST-in-batch in
        // `panel_renderer` below (drawn behind the rows, over the opaque card — the
        // byte-identical historical slot). The dedicated pass is prepared empty
        // whenever it is not used, so a stale wordmark never lingers.
        let bars = matches!(
            crate::render::effective_list_style(),
            theme::ListStyle::Bars { .. }
        );
        let canvas_bounds = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        {
            let placard_pass: Vec<TextArea> = match placard {
                Some((px, py, _pw, _ph)) if bars => vec![TextArea {
                    buffer: &self.placard_buffer,
                    left: px,
                    top: py,
                    scale: 1.0,
                    bounds: canvas_bounds,
                    default_color: ink,
                    custom_glyphs: &[],
                }],
                _ => Vec::new(),
            };
            self.placard_renderer
                .prepare(
                    device,
                    queue,
                    &mut self.font_system,
                    &mut self.atlas,
                    &self.viewport,
                    placard_pass,
                    &mut self.swash_cache,
                )
                .map_err(|e| anyhow::anyhow!("glyphon placard prepare failed: {e:?}"))?;
        }
        // The placard wordmark is FIRST in the panel batch under `Pane` (drawn
        // behind everything that follows), clipped to the WHOLE CANVAS — a
        // screen-corner watermark that bleeds OVER the scrim behind the card
        // (never the tighter card/text rect), per `overlay_shape_placard`'s doc.
        // Under `Bars` it was uploaded to `placard_renderer` above instead, so it
        // is withheld here.
        let mut areas: Vec<TextArea> = Vec::new();
        if let Some((px, py, _pw, _ph)) = placard {
            if !bars {
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
            // The right column's labels lead with `header_rows` EMPTY lines, so
            // uploading its buffer at `overlay_secondary_top` (text_top + the
            // header gap) lands label N on candidate row N — the SAME band
            // `overlay_row_top` draws. Before this it uploaded flush at
            // `text_top`, missing the gap the primary + band both fold in, so
            // every chord rode a half-row high (the composition-round bug).
            areas.push(TextArea {
                buffer: &self.panel_bind_buffer,
                left: text_left,
                top: overlay_secondary_top(text_top, geom.header_gap),
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
        let list_style = crate::render::effective_list_style();
        let card_rect = [geom.card_x, geom.card_y, geom.card_w, geom.card_h];
        if self.overlay_spell.is_some() {
            // Contextual spell panel: elevate on the float primitive, no flat card.
            self.prepare_float_panel(device, queue, width, height, Some(card_rect));
            self.panel_card.prepare(device, queue, width, height, &[]);
            self.panel_shadow.prepare(device, queue, width, height, &[]);
            self.panel_border.prepare(device, queue, width, height, &[]);
        } else if matches!(list_style, theme::ListStyle::Bars { .. }) {
            // PER-ITEM LIST SURFACES round — BARS DROP THE PANE (the user's refit:
            // "with the bars, there shouldn't be a pane!"). No boxed card (no
            // border, no shadow, no bright `base_300` fill) — the bars, title,
            // query, strip and hint float in the Persona ROOM: bars sit ON the
            // room, not IN a box.
            //
            // DESIGNER PIXEL-PASS FIX (2026-07-16, the "gap comb seam"): the theme
            // picker is CRISP (no blur, no scrim — the doc stays bright so the live
            // theme preview reads honestly), so with the box gone the RAW document
            // showed through every gap between bars — its page-margin band on the
            // left meeting the writing column mid-gap made a hard vertical seam
            // repeated down the whole list. The room was never actually a room. So
            // Bars lays a UNIFORM full-canvas value VEIL of the world's own ground
            // (`base_100` at part alpha — a scrim, never a bordered box): it pulls
            // the document a value back into one calm plane, killing the comb seam
            // and the left-overhang stub in one stroke, while a hint of the
            // re-tinted ground still ghosts through so the theme preview survives.
            // The shadow/border stay empty (no elevation — it is a room, not a
            // card). Drawn FIRST in `draw_overlay_card`, so the placard watermark,
            // the bars, and the row text all composite over it.
            self.panel_card.set_corner(0.0);
            self.panel_card
                .set_color(theme::overlay_bars_room().rgba_bytes());
            // BLEED the room plane past ALL FOUR canvas edges. The panel quad
            // pipeline feathers a ~1px antialiased edge (`selection.wgsl`'s
            // `smoothstep(-1, 1, d)`), so a plane sized flush to `[0, 0, w, h]`
            // left the FIRST pixel row (centre 0.5px inside the top edge) only
            // ~84% covered — the calm ground showed a 1px LIGHTER seam along
            // y = 0 (the designer's first-scanline nit). Growing the rect
            // `ROOM_BLEED` px on every side pushes the whole feather off-screen,
            // so every on-canvas pixel sits fully inside the plane (`d < -1`).
            const ROOM_BLEED: f32 = 2.0;
            self.panel_card.prepare(
                device,
                queue,
                width,
                height,
                &[[
                    -ROOM_BLEED,
                    -ROOM_BLEED,
                    width as f32 + 2.0 * ROOM_BLEED,
                    height as f32 + 2.0 * ROOM_BLEED,
                ]],
            );
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
        // PER-ITEM LIST SURFACES round: `Pane` (default) draws the byte-identical
        // full-width selected BAND; `Bars` gives each candidate row its own
        // rounded surface (unselected → `overlay_bars`, quiet; selected →
        // `overlay_rows`, brighter + `grow_px` wider) with the gap already folded
        // into `lh`. The row-y owner `overlay_row_top` feeds BOTH so bars and text
        // agree on every row; the hit-test rides the same `lh`, so a click in a
        // gap maps to the nearest row (no dead zones).
        // `list_style` computed once at the top of this fn (drives the pane-drop).
        let mirror = crate::render::effective_card_anchor().mirrors_growth();
        // The selected row's drawn TOP, through the ONE row-y owner + the live-only
        // band slide (verbatim `target` in capture / Snap worlds). Computed ONCE
        // here so the animator runs once and both list styles read the same y.
        let sel_top: Option<f32> = sel_disp.map(|disp| {
            let target =
                overlay_row_top(geom.text_top, geom.header_rows, geom.header_gap, disp, lh);
            self.overlay_band_drawn(target)
        });
        let (sel_rects, bar_rects): (Vec<[f32; 4]>, Vec<[f32; 4]>) = match list_style {
            theme::ListStyle::Pane => {
                let rects = match (sel_disp, sel_top) {
                    (Some(disp), Some(top)) => {
                        // WILD-MENU SLANT PROBE (env-gated, `None` on every normal
                        // run): the band's left edge follows the selected row's own
                        // stair offset so the highlight hugs its slanted row.
                        let dx = crate::render::overlay_slant()
                            .map(|s| crate::render::slant_offset(&s, disp))
                            .unwrap_or(0.0);
                        vec![[geom.card_x + dx, top, geom.card_w - dx, lh]]
                    }
                    _ => Vec::new(),
                };
                (rects, Vec::new())
            }
            theme::ListStyle::Bars { radius, gap, grow_px, extent, coverage } => {
                let r = radius.max(0.0);
                let g = gap.max(0.0);
                let bar_h = (lh - g).max(1.0);
                // V6 P5 hug extents — per-row primary widths, measured from the
                // just-shaped name buffer (read before the &mut pipeline calls
                // below). BOTH hug arms (`extent.hugs()`) size the plate to the
                // shaped name line: under `HugText` the shortcut is composed INLINE
                // into that line (so the width includes it and the plate hugs the
                // whole content); under the `HugLabel` HYBRID the line carries the
                // LABEL alone (the chord stays in the right column, outside the
                // plate), so the plate hugs the label. `FullWidth` → no hug.
                let hug = extent.hugs();
                let primary_px = if hug {
                    self.overlay_row_primary_px(geom)
                } else {
                    std::collections::BTreeMap::new()
                };
                // V6 P5 [`theme::BarExtent::HugText`] — the natural `(x, w)` span
                // for a display row: full-width, or hugging the row's own content
                // (label + inline shortcut) + a symmetric pad. EVERY row hugs; the
                // rag derives from content length only (V7 taste-gate).
                let span_of = |k: usize| -> (f32, f32) {
                    if hug {
                        super::bar_hug_span(
                            geom.card_x,
                            geom.card_w,
                            geom.text_left,
                            primary_px.get(&k).copied().unwrap_or(0.0),
                        )
                    } else {
                        super::bar_full_span(geom.card_x, geom.card_w)
                    }
                };
                let bar_off = g * 0.5;
                // Both bar pipelines round to the world's radius (0 = sharp
                // P4-Status bars, large = Velvet capsules). Bars are always FILLED
                // (the V7 taste-gate dropped the outline-fill axis — the rim read
                // as a focus ring, not a Persona ledge).
                self.overlay_rows.set_corner(r);
                self.overlay_bars.set_corner(r);
                self.overlay_rows.set_stroke(0.0);
                self.overlay_bars.set_stroke(0.0);
                // Unselected bars: a QUIET rung one step above the card
                // (`overlay_bar_unselected`, steps `1`) — deliberately well below
                // the selected bar's band (`overlay_selected_band`, steps `3`), so
                // the selected bar leads by a ~2-step VALUE gap (an obvious glance,
                // never a hue — DESIGN §3). The old `surface_selected` (steps `2`)
                // sat one lone step under the selected band and read as barely
                // distinct on saturated worlds (the Kingfisher/Saltpan defect).
                self.overlay_bars
                    .set_color(theme::overlay_bar_unselected().rgba_bytes());
                // The DISPLAY rows that get a bar: every drawn ITEM row (the theme
                // picker's section-HEADER lines get none — a header is a label).
                let item_rows: Vec<usize> = if geom.theme {
                    geom.plan
                        .iter()
                        .enumerate()
                        .filter_map(|(k, l)| matches!(l, ThemeLine::Item(_)).then_some(k))
                        .collect()
                } else {
                    (0..geom.visible).collect()
                };
                // V6 P5 [`theme::BarCoverage::SelectedOnly`] — unselected rows
                // render as BARE floating text on the room (the P5 settings-screen
                // look): no unselected bars at all. `All` (v5) gives every row a
                // surface. The footer plate below is pushed regardless (it guards
                // the hint over the placard, not a per-row affordance).
                let mut unsel: Vec<[f32; 4]> = match coverage {
                    theme::BarCoverage::SelectedOnly => Vec::new(),
                    theme::BarCoverage::All => item_rows
                        .iter()
                        .copied()
                        .filter(|k| Some(*k) != sel_disp)
                        .map(|k| {
                            let top = overlay_row_top(
                                geom.text_top,
                                geom.header_rows,
                                geom.header_gap,
                                k,
                                lh,
                            );
                            let (x, w) = span_of(k);
                            [x, top + bar_off, w, bar_h]
                        })
                        .collect(),
                };
                // THE FOOTER-OVER-POSTER GUARANTEE (taste-gate finding): under
                // Bars the pane is dropped, so a giant corner PLACARD bleeds up
                // behind the dim foot-hint / keybindings-tips footer and the muted
                // glyphs drown in the poster letters. Lay an opaque whisper-value
                // plate over that zone (same `overlay_bars` z-slot — over the
                // placard, under the text) so the footer keeps its designed
                // ground. Only when the picker HAS a footer (`hint`/`tips`);
                // shares the ONE `overlay_row_top` owner via `footer_plate_rect`.
                if geom.hint_rows + geom.footer_rows > 0 {
                    let content_rows = if geom.theme {
                        geom.plan.len()
                    } else {
                        geom.visible + geom.empty.is_some() as usize
                    };
                    // V8 — under HUG bars the footer plate hugs its own content
                    // (same padding rule as the rows), so it never reads as a lone
                    // full-width plate stretched under the ragged pills. Measured
                    // from the SAME just-shaped `panel_buffer` as the row widths
                    // (read before the &mut prepare calls below, like `primary_px`).
                    let footer_hug = hug.then(|| {
                        (geom.text_left, self.overlay_footer_content_px(geom, content_rows))
                    });
                    unsel.push(super::footer_plate_rect(
                        geom.text_top,
                        geom.header_rows,
                        geom.header_gap,
                        content_rows,
                        lh,
                        geom.card_x,
                        geom.card_w,
                        geom.card_y + geom.card_h,
                        footer_hug,
                    ));
                }
                // The SELECTED bar: its natural span (full or hugged), grown
                // `grow_px` toward the open margin — RIGHT by default, mirrored
                // LEFT under a right-anchored (`TopRight`) card. `grow_span` is the
                // ONE pure owner shared with the full-width `bar_rect_selected`.
                let sel = match (sel_disp, sel_top) {
                    (Some(disp), Some(top)) => {
                        let (bx, bw) = span_of(disp);
                        let (x, w) = super::grow_span(bx, bw, grow_px, mirror);
                        vec![[x, top + bar_off, w.max(1.0), bar_h]]
                    }
                    _ => Vec::new(),
                };
                (sel, unsel)
            }
        };
        self.overlay_bars
            .prepare(device, queue, width, height, &bar_rects);
        self.overlay_rows
            .prepare(device, queue, width, height, &sel_rects);
        // FACETED STRIP active-lens mark: the rect the shaper recorded (its SHAPE
        // set by `facet_style` — hairline underline / band / active chip); a
        // non-theme card parks it empty (so a stale rect never lingers).
        let underline: Vec<[f32; 4]> = if geom.theme {
            self.overlay_theme_underline.iter().copied().collect()
        } else {
            Vec::new()
        };
        // PER-ITEM LIST SURFACES round: `Text` (default) keeps the content-ink
        // hairline byte-identically; `Band` recolors the ACTIVE mark to the
        // selected-row band VALUE (never amber) and rounds it into a pill.
        // V6 P5 [`theme::FacetStyle::Chips`] — REAL chips (third attempt): the
        // ACTIVE label rides `overlay_lens_underline` as a FILLED value pill
        // (same as `Band`), and EACH INACTIVE label draws a GHOST pill — a MUTED
        // hairline STROKE — via `overlay_facet_ghost`. Both are recorded from the
        // SAME shaped strip glyphs (in `overlay_shape_theme`), so the skin can't
        // disagree with the hit-test.
        let facet_style = crate::render::effective_facet_style();
        // The inactive ghost pills (empty unless Chips on a theme card).
        let mut ghosts: Vec<[f32; 4]> = Vec::new();
        match facet_style {
            theme::FacetStyle::Text => {}
            theme::FacetStyle::Band | theme::FacetStyle::Chips => {
                let band = match theme::active()
                    .highlight_treatment(crate::render::effective_overlay_selrow_band())
                {
                    theme::HighlightTreatment::ValueBand(c) => c,
                    theme::HighlightTreatment::InverseFill { band, .. } => band,
                };
                self.overlay_lens_underline.set_color(band.rgba_bytes());
                self.overlay_lens_underline.set_corner(FACET_CHIP_RADIUS);
                if matches!(facet_style, theme::FacetStyle::Chips) && geom.theme {
                    ghosts = self.overlay_theme_facet_ghosts.clone();
                }
            }
        }
        self.overlay_lens_underline
            .prepare(device, queue, width, height, &underline);
        // The Chips ghost pills: a muted hairline stroke pill per inactive facet.
        self.overlay_facet_ghost.set_corner(FACET_CHIP_RADIUS);
        self.overlay_facet_ghost
            .set_stroke(crate::render::BAR_OUTLINE_STROKE_PX);
        self.overlay_facet_ghost
            .prepare(device, queue, width, height, &ghosts);
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
        // The caret rides the QUERY line through the SAME layout the query TEXT
        // does — its first shaped run's own `line_height`, NOT the bare
        // `overlay_lh()`. The FLAT pickers inflate the query line's height by
        // `header_gap` to open the beat before the candidates (`shape_overlay_names`),
        // and cosmic-text HALF-LEADS the glyphs — it centres them in that taller
        // line rather than pinning them to the top. So the query text drops by
        // `header_gap * 0.5`; a caret pinned to `overlay_lh() * 0.5` floated a full
        // half-beat ABOVE it (the full-bleed caret bug — visible on `Bars`, where
        // no card frames the mismatch, and present-but-masked on `Pane`). Reading
        // the run's real `line_height` reproduces the known-good faceted offset
        // (the caret centre lands a constant ~1/3-row above the baseline) in BOTH
        // paths, since the faceted query line is NOT inflated (its beat rides the
        // strip). Fallback to `overlay_lh()` only if shaping yielded no run.
        let query_line_height = self
            .panel_buffer
            .layout_runs()
            .next()
            .map(|r| r.line_height)
            .unwrap_or_else(|| self.overlay_lh());
        let caret_cy = overlay_query_center(geom.text_top, query_line_height);
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
