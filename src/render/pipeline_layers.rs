//! RENDER LAYERS — render-pass composition and ordered layer emission.

use super::*;

impl TextPipeline {
    /// A render pass on `view` that CLEARS to the theme's `base_100` (the calm page
    /// ground every frame starts from).
    fn begin_clear_pass<'a>(
        encoder: &'a mut wgpu::CommandEncoder,
        view: &'a wgpu::TextureView,
    ) -> wgpu::RenderPass<'a> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("awl text pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(theme::base_100().to_wgpu()),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }

    /// Record the clear + text/caret draw into `encoder`, targeting `view`.
    ///
    /// Two paths. For the COMMON case (no overlay, the search SPLIT panel, OR a crisp
    /// THEME/CARET picker) everything composites in ONE pass over the cleared view —
    /// byte-identical to before, so a non-overlay document capture is unchanged. For a
    /// blur-eligible full overlay the document is rendered ONCE to an offscreen
    /// texture, blurred (only when [`Self::blur_recompute`] — else the cache stands),
    /// and the frosted result is composited behind the overlay card in the final pass.
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
    ) -> anyhow::Result<()> {
        if self.backdrop_blur() {
            // 1) Capture the document into the offscreen texture + blur it — but ONLY
            //    when the cached backdrop is stale (a fresh open / resize / doc or
            //    theme change). A settled overlay-open (or HUD-held) frame skips straight
            //    to the composite, re-blurring nothing (DESIGN §6).
            if self.blur_recompute {
                if let Some(doc_view) = self.blur.doc_view() {
                    let mut pass = Self::begin_clear_pass(encoder, doc_view);
                    self.draw_document_layers(&mut pass)?;
                }
                self.blur.encode_blur(encoder);
            }
            // 2) Final pass: the frosted backdrop (hue-preserving defocus, dimmed a
            //    value toward base_100) THEN the overlay card (empty for a HUD-only
            //    frame) + the chrome tail (the held HUD card + stats) on top — NO grey
            //    scrim, for either takeover.
            let mut pass = Self::begin_clear_pass(encoder, view);
            self.blur.draw_backdrop(&mut pass);
            self.draw_overlay_card(&mut pass)?;
            self.draw_chrome_tail(&mut pass)?;
            return Ok(());
        }

        // COMMON path: one pass over the cleared view.
        let mut pass = Self::begin_clear_pass(encoder, view);
        self.draw_document_layers(&mut pass)?;
        // The search panel / crisp overlay composites OVER the document text. There is
        // no depth buffer (depth_stencil: None everywhere) so painter's order == draw
        // submission order.
        if self.overlay_active {
            // A CRISP overlay (theme / caret picker): the document stays bright behind
            // it — NO blur, NO scrim — so the live theme colours / caret preview read
            // honestly. Just the card on top.
            self.draw_overlay_card(&mut pass)?;
        } else if self.search_active {
            // The find/replace panel is ELEVATED on the float primitive (shadow ->
            // raised border -> base_300 card), then the amber caret + labeled text on
            // top. The float quads are prepared in `prepare_panel` and parked whenever
            // the panel is down, so a no-panel frame stays byte-identical.
            self.float_shadow.draw(&mut pass);
            self.float_border.draw(&mut pass);
            self.float_card.draw(&mut pass);
            self.panel_card.draw(&mut pass);
            self.panel_caret.draw(&mut pass);
            self.panel_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .map_err(|e| anyhow::anyhow!("glyphon panel render failed: {e:?}"))?;
        }
        self.draw_chrome_tail(&mut pass)?;
        Ok(())
    }

    /// Draw the DOCUMENT layers (everything behind any overlay) into an open pass, in
    /// painter's order: PAGE-MODE margin gradient -> selection -> search-match ->
    /// wavy spell underlines -> straight muted nit underlines -> BLOCK caret quad -> cosmetic trail -> document text ->
    /// MORPH caret silhouette (OVER the text) -> page-mode gutter -> markdown
    /// ornaments. The block caret sits BELOW the glyph cell so the letter is never
    /// covered; the morph caret paints the cursor glyph's silhouette OVER the letter
    /// to recolour it the accent. Shared by the common path and the blur path's
    /// offscreen doc capture, so the captured backdrop matches the live document.
    fn draw_document_layers<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) -> anyhow::Result<()> {
        self.background_pipeline.draw(pass);
        // THE LAVA-LAMP GROUND: over the flat margin ground, before every
        // foreground layer. A total no-op (draws nothing) for every non-lava
        // world — so all fifteen shipped worlds render byte-identically.
        self.lava_pipeline.draw(pass);
        // TWINKLING STARS: the ambient star field, over the margin ground and
        // under everything foreground. Zero instances (draws nothing) for
        // every AmbientStyle::None world.
        self.stars_pipeline.draw(pass);
        // THE PAGE FRAME (theme::PageFrame): the thin writing-column frame,
        // right after the ground and before every wash/text layer — so text,
        // washes, selection all composite OVER it if they ever meet it (they
        // shouldn't: the frame straddles the column boundary, in the margin).
        // Zero instances (draws nothing) for every PageFrame::None world.
        self.page_frame_pipeline.draw(pass);
        // DIFF-AS-PREVIEW panel dressing: shadow -> border -> card, UNDER every
        // wash/text layer (the transcript draws ON the card, clipped to it via
        // `doc_clip_band`). Zero instances on every ordinary frame.
        self.diffpanel_shadow.draw(pass);
        self.diffpanel_border.draw(pass);
        self.diffpanel_card.draw(pass);
        // WYSIWYG value-step panel/pill sit directly ON the ground, BEFORE the
        // syntax washes — so a fenced block's comment/string wash composites over
        // the panel exactly as it does over the bare ground, and a selection over
        // either in turn.
        self.fence_panel_pipeline.draw(pass);
        self.code_pill_pipeline.draw(pass);
        // WYSIWYG table grid's faint header-separator hairline sits on the ground
        // with the other value-step quads, before the syntax washes + text.
        self.table_rule_pipeline.draw(pass);
        // SYNTAX WASHES sit directly ON the ground, UNDER selection / search /
        // squiggles / text — so a selection composites over a washed comment
        // exactly as it does over the bare ground.
        self.wash_comment_pipeline.draw(pass);
        self.wash_string_pipeline.draw(pass);
        // MARKDOWN `==highlight==` band: its own violet tint, same layer as the
        // syntax washes (under selection / text).
        self.wash_highlight_pipeline.draw(pass);
        // INLINE IMAGES: the decoded image quads + missing-file placeholder cards,
        // drawn AFTER the washes and BEFORE selection — so a selection / the caret /
        // a revealed source line all composite OVER the image, exactly the design's
        // layer slot. Empty (nothing drawn) when the feature is off / no visible
        // images, keeping the frame byte-identical.
        self.image_placeholder_pipeline.draw(pass);
        self.image_pipeline.draw(pass);
        // CAPTION SCRIM: over the dimmed image, UNDER selection / caret / the revealed
        // source text — so the caption reads over the image while a selection over it
        // still composites correctly. Parked empty unless an image line is revealed.
        self.image_scrim_pipeline.draw(pass);
        // Ordinary document-selection quads (an ORDINARY world's translucent
        // fill). On a one-bit world `prepare_selection_layer` uploads ZERO
        // rects here — the true-inverse-video pipeline below takes over
        // selection entirely — so this draws nothing there.
        self.selection_pipeline.draw(pass);
        // Search-match quads: an ordinary world's translucent fill, OR (on a
        // one-bit world) THE ONE WAGTAIL HIGHLIGHT TEXTURE's dither stipple —
        // either way this stays a BEFORE-text wash/highlight layer.
        self.match_pipeline.draw(pass);
        self.spell_pipeline.draw(pass);
        self.nit_pipeline.draw(pass);
        // `~~strikethrough~~` lines — same under-text slot as the nit hint: the
        // stroke shares the struck text's own muted ink, so under vs over the
        // glyphs composites identically where they meet.
        self.strike_pipeline.draw(pass);
        // The quiet link UNDERLINE — same under-text slot, its own instance.
        self.link_underline_pipeline.draw(pass);
        self.caret_pipeline.draw(pass);
        self.caret_trail_pipeline.draw(pass);
        self.renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon render failed: {e:?}"))?;
        // TRUE 1-BIT WORLDS ONLY: the inverse-video selection, drawn strictly
        // AFTER the document text above — the `OneMinusDst` blend trick needs
        // the destination to already hold the composited text+ground pixels
        // it's about to flip. Idle (zero instances) on every other world.
        self.selection_invert.draw(pass);
        // THE 1-BIT CARET ROUND: the block caret's own true-inverse-video
        // quad, same AFTER-text slot as `selection_invert` immediately above
        // (see `caret_invert`'s field doc + `prepare_caret_block`). Idle on
        // every other world. NOTE (documented, not fixed — out of this
        // round's narrow scope): if the caret's rect and an active
        // selection's rect ever genuinely overlap on a one-bit world, the
        // two invert passes compose by applying the flip TWICE in the
        // overlap (cancelling back toward the original colors there) rather
        // than merging into one flip — the caret ordinarily sits at a
        // selection's boundary, not inside it, so this is not the bug this
        // round fixes, but it's a real edge case for a future round.
        self.caret_invert.draw(pass);
        self.caret_glyph_pipeline.draw(pass);
        self.gutter_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon gutter render failed: {e:?}"))?;
        // PERSISTENT MARGIN OUTLINE: the top-left table-of-contents, in the same
        // text/chrome band as the gutter (so it recedes behind overlays like all
        // document chrome). Parked off-screen when hidden, so a default frame is
        // byte-identical.
        self.outline_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon outline render failed: {e:?}"))?;
        self.ornament_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon ornament render failed: {e:?}"))?;
        // WYSIWYG table-grid cell text, in the same text/ornament band (over the
        // ground + its own header rule). Parked for a non-table / parked frame.
        self.table_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon table render failed: {e:?}"))?;
        // INLINE IMAGES: the missing-file placeholder LABELS (filename + alt), over
        // their base_200 card (drawn earlier, before selection). Parked (no areas)
        // when nothing is missing, so a default frame is byte-identical.
        self.image_placeholder_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon image placeholder render failed: {e:?}"))?;
        Ok(())
    }

    /// Draw the summoned OVERLAY card into an open pass (over whatever backdrop the
    /// caller set — the crisp document or the frosted blur).
    ///
    /// The FLOATING-PANEL elevation (shadow -> raised border -> card) is drawn FIRST,
    /// BEHIND the overlay card + text, because it is the background for two summoned
    /// micro-panels that ride the same three quads: the SPELL contextual panel (the
    /// panel IS this floating card — `panel_card` is empty then) and the caret-style
    /// preview panel that hangs BELOW the picker (it doesn't overlap the picker card,
    /// so drawing its elevation first is harmless). NEXT: `panel_shadow`/
    /// `panel_border` — the SAME shadow/border shape, over `panel_card`'s own rect,
    /// non-empty ONLY on a true 1-bit world (`overlay_draw_card`'s prepare-time
    /// gate) — so the flat picker card gets a crisp white border exactly where the
    /// now-disabled blur/scrim used to carry its contrast; parked empty (byte-
    /// identical to before) on every other world. Then: the opaque picker card ->
    /// selected-row value band -> amber query caret -> overlay text, and last the
    /// caret-style preview's demo caret + sample line ON its (already-drawn) card.
    /// Every float / preview quad parks empty unless one of those two panels is open.
    fn draw_overlay_card<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) -> anyhow::Result<()> {
        self.float_shadow.draw(pass);
        self.float_border.draw(pass);
        self.float_card.draw(pass);
        self.panel_shadow.draw(pass);
        self.panel_border.draw(pass);
        self.panel_card.draw(pass);
        // DESIGNER PIXEL-PASS FIX (2026-07-16): under `Bars` the placard watermark
        // must sit BEHIND the bar quads (the row surfaces are the figure; the
        // wordmark stays part of the live backdrop). Its dedicated pass draws HERE —
        // over the page and bounded scrims, under the bars. Parked empty under `Pane`
        // (byte-identical there — the placard rides `panel_renderer` below). The
        // stipple placard likewise slots behind the bars in this mode.
        let bars = matches!(
            crate::render::effective_list_style(),
            theme::ListStyle::Bars { .. }
        );
        if bars {
            self.placard_stipple.draw(pass);
            self.placard_renderer
                .render(&self.atlas, &self.viewport, pass)
                .map_err(|e| anyhow::anyhow!("glyphon placard render failed: {e:?}"))?;
        }
        // PER-ITEM LIST SURFACES: the unselected bar surfaces sit ON the card and
        // UNDER the selected bar (`overlay_rows`). Parked empty on every Pane
        // world, so this is byte-identical there.
        self.overlay_bars.draw(pass);
        self.overlay_rows.draw(pass);
        // ARM B LIVING-BAND PROBE — the two-shape CROSSING quad sits just ABOVE the
        // leading band (`overlay_rows`) so the brightest value reads where the two
        // shapes overlap. Parked empty on every ordinary run → byte-identical.
        self.overlay_cross.draw(pass);
        // THEME PICKER: the active-lens hairline under the strip (content ink), UNDER
        // the overlay text so the glyphs sit on top. Parked empty for every other card.
        // V6 P5: the Chips ghost pills draw first (inactive, muted stroke), then the
        // active pill on top — both under the strip labels.
        self.overlay_facet_ghost.draw(pass);
        self.overlay_lens_underline.draw(pass);
        self.panel_caret.draw(pass);
        // THE STIPPLE PLACARD (`Pane` only — under `Bars` it was drawn behind the
        // bars above): the same "behind the rows, over the card/band quads" slot
        // the TEXT placard occupies via its first-in-batch upload — so row/query
        // glyphs always composite OVER the stippled wordmark. Zero instances on
        // every non-stipple world / closed overlay.
        if !bars {
            self.placard_stipple.draw(pass);
        }
        self.panel_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon overlay render failed: {e:?}"))?;
        // CARET-STYLE PICKER: the animated demo caret (under the sample text, like the
        // document block caret), then the sample line, then — Morph only, settled on
        // a real glyph — the demo's OWN silhouette pipeline OVER the text, exactly
        // mirroring the document's block-caret -> text -> glyph-silhouette painter's
        // order (`draw_document_layers`). Both on the preview card drawn above.
        // Parked/empty unless the caret-style picker is open.
        self.caret_preview_pipeline.draw(pass);
        self.preview_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon preview render failed: {e:?}"))?;
        self.caret_preview_glyph_pipeline.draw(pass);
        Ok(())
    }

    /// Draw the floating CHROME tail into an open pass: the opt-in DEBUG panel
    /// (top-left, dim; parked off-screen when off) then the SUMMONED-WHILE-HELD stats
    /// HUD (its card + stats, drawn LAST so it floats over everything). While held, the
    /// document already recedes behind the shared FROSTED-BLUR backdrop (the `render`
    /// blur branch), so the HUD needs no scrim of its own. Both park off-screen when
    /// inactive, so a default render is byte-identical.
    fn draw_chrome_tail<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) -> anyhow::Result<()> {
        self.debug_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon debug render failed: {e:?}"))?;
        // The CALM NOTICE (bottom-center, muted): parked off-screen when empty,
        // so a notice-less frame — every capture — is byte-identical.
        self.notice_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon notice render failed: {e:?}"))?;
        // The PAGE-WIDTH DRAG READOUT (floats at the pointer): parked off-screen
        // while not dragging, so a default render is byte-identical.
        self.page_drag_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon page-drag-readout render failed: {e:?}"))?;
        // The ZOOM READOUT (floats at the pointer while a zoom gesture is in flight):
        // parked off-screen while settled, so a default render is byte-identical.
        self.zoom_readout_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon zoom-readout render failed: {e:?}"))?;
        // Float-panel elevation, painter's order: drop shadow -> raised border -> card.
        self.hud_shadow.draw(pass);
        self.hud_border.draw(pass);
        self.hud_card.draw(pass);
        // WRITING-STREAKS heatmap squares ride ON the card, under its text (empty
        // unless the Writing streaks card is the summoned one this frame).
        self.streak_cells.draw(pass);
        self.hud_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon hud render failed: {e:?}"))?;
        // WHICH-KEY panel LAST: the summoned prefix-continuation hint card (its own
        // float elevation + text), floating over everything. Parked/empty unless the
        // App summoned it on a prefix pause, so a default render is byte-identical.
        self.wk_shadow.draw(pass);
        self.wk_border.draw(pass);
        self.wk_card.draw(pass);
        self.wk_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon whichkey render failed: {e:?}"))?;
        // WEB/LINUX MENU BAR, drawn LAST so it floats over everything (the persistent
        // top chrome stays on top, and an open dropdown — mutually exclusive with a
        // summoned overlay — hangs over the document). Bar ground -> open-title
        // highlight -> title glyphs; then the dropdown's float elevation (shadow ->
        // border -> card) -> separator hairlines -> item labels -> chords. ALL parked
        // off-screen/empty when the bar is hidden, so a default render is byte-identical.
        self.menubar_bg.draw(pass);
        // The open-title highlight is ONE solid fill UNDER the title glyphs (its
        // color the value-band tint, or solid `base_content` on a 1-bit world
        // where the open title's glyphs are recolored to `base_300` — see
        // `HighlightTreatment::InverseFill`), so the title composites OVER it.
        self.menubar_hi.draw(pass);
        self.menubar_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon menubar render failed: {e:?}"))?;
        self.menu_drop_shadow.draw(pass);
        self.menu_drop_border.draw(pass);
        self.menu_drop_card.draw(pass);
        self.menu_drop_sep.draw(pass);
        self.menu_drop_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon menu-drop label render failed: {e:?}"))?;
        self.menu_chord_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon menu-drop chord render failed: {e:?}"))?;
        // THE FORMAT POPOVER, drawn LAST so it floats over the document (like the
        // which-key panel): float elevation (shadow -> raised border -> card) ->
        // active-button value-step wash -> button labels. ALL parked off-screen/empty
        // when the popover is down, so a default render is byte-identical.
        self.popover_shadow.draw(pass);
        self.popover_border.draw(pass);
        self.popover_card.draw(pass);
        self.popover_wash.draw(pass);
        // SELF-DEMONSTRATING quads: `A`'s highlight pill over the value-step
        // washes, `S`'s strike line — both UNDER the labels (the doc's own
        // wash-under-text / line-in-own-ink layering).
        self.popover_hl_wash.draw(pass);
        self.popover_strike.draw(pass);
        self.popover_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon popover render failed: {e:?}"))?;
        Ok(())
    }
}
