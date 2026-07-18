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

impl TextPipeline {
    /// The ONE metric every overlay ROW shapes + measures at: the reading body stepped
    /// down by [`OVERLAY_UI_SCALE`]. [`Self::overlay_remetric`] sets the shared buffers
    /// to it, and [`Self::overlay_lh`] (its line-height half) is what every geometry
    /// reader shares — so shaping and geometry can never drift on the row size.
    pub(in crate::render) fn overlay_metrics(&self) -> GlyphMetrics {
        let m = self.metrics;
        let scale = crate::render::effective_overlay_scale();
        GlyphMetrics::new(
            m.font_size * scale,
            m.line_height * scale
                + crate::render::effective_overlay_leading()
                + self.overlay_row_gap(),
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
        self.metrics.line_height * crate::render::effective_overlay_scale()
            + crate::render::effective_overlay_leading()
            + self.overlay_row_gap()
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

    /// The pixel scale the overlay CHROME shapes at this frame — the same
    /// `zoom * dpi` factor [`crate::render::Metrics::with_dpi`] folds into every
    /// glyph metric, recovered from the live body `font_size`. `1.0` at zoom 1.0 /
    /// dpi 1.0 (the capture default and the tuning canvas), so anything scaled by
    /// it is byte-identical there.
    pub(in crate::render) fn overlay_pixel_scale(&self) -> f32 {
        self.metrics.font_size / crate::render::FONT_SIZE
    }

    /// THE ONE OWNER of the summoned card's WIDE desired width (device px) at the
    /// CURRENT zoom/DPI: the base cap ([`CARD_MAX_W`] / [`CARD_MAX_W_FACETED`],
    /// tuned for the 1:1 capture canvas) GROWN by [`Self::overlay_pixel_scale`] so
    /// the card widens WITH the glyphs.
    ///
    /// Without this the cap stayed an unzoomed 520/600 while the overlay text
    /// DOUBLED under zoom — the card read proportionally half as wide, and
    /// [`rowlayout`]'s primary-cell elision + the footer's yield fired even though
    /// the WINDOW had abundant room (the zoom-blind card bug: at 200% every
    /// palette row came back "Go t…ile…", "Comp…ion…"). The window clamp in
    /// [`overlay_card_box_policy`] still bounds the result, so a card never
    /// overruns the window — it just stops eliding when there IS room, and enters
    /// the fill regime only when the window GENUINELY lacks it. Both geometry
    /// paths ([`Self::overlay_geometry`], [`TextPipeline::theme_overlay_geometry`])
    /// and the fill-regime fold read this ONE scaled width, so the width and the
    /// fold threshold can never drift.
    ///
    /// GROW-ONLY (`scale.max(1.0)`): the scale only ever WIDENS the base cap. The
    /// bug is a high-zoom COLLAPSE, so the fix touches exactly the `zoom·dpi > 1.0`
    /// regime. At the SHIPPED default (zoom 0.8, dpi 1.0 → scale 0.8) and every
    /// scale ≤ 1.0 this is the identity — the card holds the base cap, BYTE-
    /// IDENTICAL to the pre-fix `base`-passthrough (so the 0.8 default look and
    /// every ≤1.0 capture/law are untouched; a slightly-roomier-than-proportional
    /// card at low zoom never clips).
    pub(in crate::render) fn overlay_card_desired_w(&self, base: f32) -> f32 {
        base * self.overlay_pixel_scale().max(1.0)
    }

    /// TEST HOOK (zoom-aware card width) — the currently-set overlay's candidate
    /// items that the card's REAL width would ELIDE at window `width`, at the
    /// current zoom. Reconstructs the shaper's own decision from the live
    /// [`Self::overlay_geometry`]`.text_w` (so a card-width regression IS caught)
    /// through the ONE elision owner: `total_chars = floor(text_w / char_width)`,
    /// `full_budget`, then [`rowlayout::fit_primary`] — the exact two lines both
    /// `overlay_shape_text` and `overlay_shape_theme` run for the palette's
    /// primary column. Reports the OUTCOME (which names lose text), never a
    /// mechanism count. Empty = no primary elided (the whole list fits).
    #[cfg(test)]
    pub(in crate::render) fn overlay_elided_candidates(&self, width: u32) -> Vec<String> {
        let geom = self.overlay_geometry(width);
        let cw = self.metrics.char_width;
        let total_chars = if cw > 0.0 {
            (geom.text_w / cw).floor() as usize
        } else {
            usize::MAX
        };
        let budget = rowlayout::full_budget(total_chars);
        self.overlay_items
            .iter()
            .filter(|item| rowlayout::fit_primary(item.as_str(), budget).as_str() != item.as_str())
            .cloned()
            .collect()
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

    /// THE ONE CARD-HEIGHT OWNER — every takeover/popup card's `card_h`. The card
    /// hugs its content exactly: `total_rows` display lines at [`Self::overlay_lh`],
    /// PLUS the query-beat `header_gap` ([`Self::overlay_header_gap`], `0.0` on the
    /// contextual spell popup) and `2 * pad` top/bottom padding, LESS the compact
    /// foot-hint reclaim ([`Self::overlay_footer_reclaim`], `0.0` when `hint_rows ==
    /// 0`). All three card-height sites — the flat picker ([`Self::overlay_geometry`]),
    /// the faceted theme picker ([`TextPipeline::theme_overlay_geometry`]), and the
    /// spell popup ([`Self::spell_overlay_geometry`]) — route through here, so a
    /// footer/gap tweak can never drift the bottom edge per `OverlayKind` again (the
    /// C2 divergence class the `overlay_card_geometry_agrees_per_kind` law now pins).
    /// `pad` stays a parameter: the flat/theme cards breathe at `12.0`, the small
    /// contextual spell popup at `10.0`.
    pub(in crate::render) fn overlay_card_h(
        &self,
        total_rows: usize,
        header_gap: f32,
        hint_rows: usize,
        pad: f32,
    ) -> f32 {
        total_rows as f32 * self.overlay_lh() + header_gap + 2.0 * pad
            - self.overlay_footer_reclaim(hint_rows)
    }

    /// THE ONE STRIP-BAND OWNER — the faceted theme picker's lens STRIP sits on
    /// display line 1, whose height is inflated to `lh + header_gap` by the query
    /// BEAT (cosmic-text half-leads the labels into that taller box, so they center
    /// below a plain `lh` band). Returns `(strip_top, strip_lh)`: the strip's top
    /// edge (`text_top + lh`) and its inflated line height. The lens hit-test
    /// ([`TextPipeline::overlay_lens_at`]), the active-facet pill center, and the
    /// strip-label glyph metrics all read THIS — so the clickable band, the pill,
    /// and the shaped glyphs can never disagree about where the strip sits (the
    /// misaligned-chip / half-row band-vs-text drift class). Flat pickers have no
    /// strip; this is meaningful only when `geom.theme`.
    pub(in crate::render) fn overlay_strip_band(&self, geom: &OverlayGeom) -> (f32, f32) {
        let lh = self.overlay_lh();
        (geom.text_top + lh, lh + geom.header_gap)
    }

    /// THE ONE RIGHT-COLUMN LABEL OWNER — which dim right-aligned column a picker
    /// draws. Exactly one of the three slices is ever populated, so the precedence
    /// (key `bindings` → relative `times` → repo `git` tag) is arbitrary but must be
    /// IDENTICAL between the flat ([`TextPipeline::overlay_shape_text`]) and faceted
    /// ([`TextPipeline::shape_faceted`]) shapers, which both read it. `&[]` when no
    /// column applies. One owner so a fourth column kind (or a reordering) lands in
    /// one place, never diverging the two shapers.
    pub(in crate::render) fn overlay_right_labels(&self) -> &[String] {
        if !self.overlay_bindings.is_empty() {
            &self.overlay_bindings
        } else if !self.overlay_times.is_empty() {
            &self.overlay_times
        } else {
            &self.overlay_git
        }
    }

    /// TEST ONLY — whether this geom takes the FACETED (lens-strip) layout.
    #[cfg(test)]
    pub(in crate::render) fn overlay_geom_is_faceted(&self, geom: &OverlayGeom) -> bool {
        geom.theme
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
        // width cap ([`CARD_MAX_W`], item 3), scaled to the current zoom/DPI by
        // the ONE owner [`Self::overlay_card_desired_w`] (so the card grows WITH
        // the glyphs instead of pinning to an unzoomed 520 — the zoom-blind card
        // bug), then placed with the edge-inset rhythm (item 2) + the
        // narrow-window collapse/fill fallback (item 7). The box narrows the width
        // only in the fill regime, so the text column can never starve.
        let desired_w = self.overlay_card_desired_w(CARD_MAX_W);
        let (card_x, card_w) = self.overlay_card_box(width, desired_w);
        // item 4 (NARROW FOLD): the placard folds to InlinePrefix once even the
        // floor inset can't seat the flat card's desired width — reads the SAME
        // scaled `desired_w` the width fallback above does, so the fold threshold
        // and the width can never drift.
        let card_narrow = overlay_card_fill_regime(width as f32, desired_w);
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
        let card_h = self.overlay_card_h(total_rows, header_gap, hint_rows, pad);
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
            // theme / strip / plan are the faceted-only trio — inert on a flat card.
            ..OverlayGeom::base()
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
        // The MIN/MAX bounds are tuned for the 1:1 capture canvas; GROW them with the
        // current zoom/DPI (the SAME grow-only `overlay_pixel_scale` the takeover
        // card's width uses) so a long correction isn't clamped to an unzoomed 360
        // while its shaped `content_w` doubled under zoom — the zoom-blind card bug,
        // contextual sibling. Grow-only (`scale.max(1.0)`): byte-identical at every
        // scale ≤ 1.0 (the shipped 0.8 default + all captures untouched).
        let scale = self.overlay_pixel_scale().max(1.0);
        let card_w = (content_w + 2.0 * pad)
            .clamp(140.0 * scale, 360.0 * scale)
            .min(width as f32 - 2.0 * margin);
        let text_w = card_w - 2.0 * pad;
        // At least one row tall so a (rare) flagged word with no suggestions still
        // reads as a small present card rather than a zero-height sliver.
        let rows = header_rows + visible.max(1) + hint_rows;
        // Through the ONE card-height owner: the popup has no query beat
        // (`header_gap == 0.0`) and no foot hint (`hint_rows == 0`), so this reduces
        // to `rows * lh + 2 * pad` byte-for-byte — but it can never drift from the
        // takeover cards' bottom geometry.
        let card_h = self.overlay_card_h(rows, 0.0, 0, pad);

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
            header_rows,
            empty,
            card_x,
            card_y,
            card_w,
            card_h,
            text_left,
            text_top,
            text_w,
            // The contextual popup is inert on every faceted/footer field:
            // no footer, no lens strip/plan, no query beat (`header_gap == 0`), no
            // title/placard (`card_narrow`) — all from `base()`.
            ..OverlayGeom::base()
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

    /// THE ONE owner of the selected candidate's DISPLAY-line index (0-based
    /// among the shown candidate lines, past the header). The selected-row band
    /// ([`overlay_draw_card`]) and the secondary right-column recolor
    /// ([`shape_overlay_right`]) both read it, so they can never disagree on which
    /// row is highlighted. Two layout families: a faceted/theme plan's selected
    /// world sits at its POSITION in the plan (section headers push it down); a
    /// flat picker's selection is its offset in the visible window (saturated +
    /// clamped defensively so a transient list-shrink can never over/underflow).
    /// `None` iff there are no items.
    pub(in crate::render) fn overlay_selected_display_line(
        &self,
        geom: &OverlayGeom,
    ) -> Option<usize> {
        if geom.n_items == 0 {
            None
        } else if geom.theme {
            Some(
                geom.plan
                    .iter()
                    .position(|l| matches!(l, ThemeLine::Item(i) if *i == self.overlay_selected))
                    .unwrap_or(0),
            )
        } else {
            Some(
                self.overlay_selected
                    .saturating_sub(geom.top_idx)
                    .min(geom.visible.saturating_sub(1)),
            )
        }
    }

}
