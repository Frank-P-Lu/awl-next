//! PIPELINE GEOMETRY — the RECONFIGURE-FROM-INPUT half of [`super::TextPipeline`].
//!
//! The setters that resize / re-tint / re-ingest the pipeline's state WITHOUT
//! drawing, carved out of `render.rs` VERBATIM: theme colour + font sync
//! (`sync_theme` / `sync_theme_colors` / `sync_theme_font` + the
//! `needs_theme_reshape` gate), `ViewState` ingestion (`set_view` +
//! `sync_view_fields`), DPI + window sizing (`set_dpi` / `set_size`), and the
//! `line_count` query. NOTE the sibling [`super::geometry`] module owns the pure,
//! read-only SPATIAL math (column / wrap / hit-test); this file owns the
//! state-MUTATING setters that feed it. Methods stay inherent on `TextPipeline`
//! (a child module sees its ancestor's private fields), so the capture output is
//! byte-identical.

use super::*;

impl TextPipeline {
    /// Re-tint every baked GPU pipeline (caret, selection, search-match, panel
    /// card, panel caret, spell squiggle) from the ACTIVE theme AND, when the new
    /// world's effective display face differs from the one the document is shaped
    /// in, RESHAPE the whole document in the new family (the expensive half —
    /// see [`Self::sync_theme_colors`] for the split). Call this after switching
    /// the active theme; the next `prepare` re-uploads.
    pub fn sync_theme(&mut self) {
        self.sync_theme_colors();
        self.sync_theme_font();
    }

    /// The O(1) COLOR half of a theme switch: re-tint the baked GPU pipelines
    /// from the ACTIVE theme, touching NO text shaping. The clear color and text
    /// inks read the active theme directly each frame, so this only needs to
    /// update the pipelines that cached a color at construction.
    ///
    /// Split out so the LIVE theme-picker preview can re-color instantly per
    /// arrow while DEFERRING the font reshape ([`Self::sync_theme_font`]) until
    /// the selection settles — the theme-burst profile showed the reshape (plus
    /// the following frame's new-face prepare) dominating every preview step,
    /// while this half is microseconds. Every settled path (commit, revert,
    /// capture, tests) still goes through [`Self::sync_theme`], which runs both.
    pub fn sync_theme_colors(&mut self) {
        self.caret_pipeline.set_color(theme::primary().rgb_bytes());
        self.caret_trail_pipeline
            .set_color(theme::primary().rgb_bytes());
        // The glyph-silhouette pipeline rides `primary` (the MORPH accent letter).
        // A FILLED block caret repurposes it as the CRT KNOCKOUT and OVERRIDES this
        // colour to `primary_content` at the draw site each frame (the ONE owner is
        // `prepare_caret_block`'s `Filled` arm — authoritative in the headless
        // capture too, which never calls this `sync_theme_colors`).
        self.caret_glyph_pipeline
            .set_color(theme::primary().rgb_bytes());
        self.selection_pipeline
            .set_color(theme::selection().rgba_bytes());
        // Search matches: `theme::selection()` on an ordinary world, THE ONE
        // WAGTAIL HIGHLIGHT TEXTURE's pure white + dither density on a
        // one-bit world — see `search_match_rgba_bytes`/`wagtail_dither_density`.
        // A switch AWAY from a one-bit world must reset the density back to
        // `0.0`, not merely leave it stale, so both calls run unconditionally
        // every re-tint.
        self.match_pipeline.set_color(search_match_rgba_bytes());
        self.match_pipeline.set_dither(wagtail_dither_density());
        // CHUNK round: THE ONE WAGTAIL HIGHLIGHT TEXTURE's Bayer-cell block size
        // (physical px, Retina-aware — paired with the density above; a no-op
        // `1.0` off a one-bit world, so this resets cleanly on a switch AWAY).
        self.match_pipeline
            .set_dither_cell(wagtail_stipple_cell_px(self.dpi));
        // SYNTAX WASHES: re-tint from THE role style provider so the theme
        // picker's instant color preview recolors the bands for free (wash
        // GEOMETRY depends only on the text, so no reshape is needed).
        self.wash_comment_pipeline
            .set_color(wash_rgba_bytes(crate::syntax::SynKind::Comment));
        self.wash_string_pipeline
            .set_color(wash_rgba_bytes(crate::syntax::SynKind::Str));
        // MARKDOWN `==highlight==` wash: re-tint from its OWN violet derivation
        // (the light/dark params flip with the world's mode) — pure white +
        // dither density on a one-bit world, same reset reasoning as above.
        self.wash_highlight_pipeline
            .set_color(highlight_wash_rgba_bytes());
        self.wash_highlight_pipeline
            .set_dither(wagtail_dither_density());
        self.wash_highlight_pipeline
            .set_dither_cell(wagtail_stipple_cell_px(self.dpi));
        // WYSIWYG value-step panel/pill: re-tint from `base_200` (O(1) — geometry
        // is theme-independent, so a theme switch re-tints without rebuilding).
        self.fence_panel_pipeline
            .set_color(theme::base_200().rgba_bytes());
        self.code_pill_pipeline
            .set_color(theme::base_200().rgba_bytes());
        // INLINE IMAGES: the calm missing-file placeholder quad re-tints from
        // `base_200` (O(1); the placeholder GEOMETRY is theme-independent, so the
        // picker preview re-tints for free). The placeholder label rides `muted`,
        // re-read at prepare time; the image textures are theme-independent.
        self.image_placeholder_pipeline
            .set_color(theme::base_200().rgba_bytes());
        // INLINE IMAGES: the caption scrim re-tints from the world's own GROUND
        // (`base_100`, part-alpha) — O(1), geometry theme-independent, so the picker
        // preview re-tints for free.
        self.image_scrim_pipeline
            .set_color(theme::image_reveal_scrim().rgba_bytes());
        // WYSIWYG table header-separator hairline: re-tint from `muted` (O(1);
        // geometry is theme-independent, so the picker preview re-tints for free).
        self.table_rule_pipeline
            .set_color(theme::muted().rgba_bytes());
        self.panel_card.set_color(theme::base_300().rgba_bytes());
        // Centered-overlay elevation companions: same shadow/border tokens as
        // every other summoned card (re-tinted for free on a theme-picker preview).
        self.panel_shadow.set_color(float_shadow_srgba());
        self.panel_border
            .set_color(theme::surface_selected().rgba_bytes());
        // The frosted blur backdrop re-reads `base_100` for its dim each `prepare`
        // (via `blur.ensure`), so no color is cached here — and the held HUD now recedes
        // the doc behind that same frost, so there is no grey scrim to re-tint.
        // Held HUD elevation re-tints with the world (same float-panel tokens as which-key:
        // shadow ink, raised surface-step border, base_300 card).
        self.hud_shadow.set_color(float_shadow_srgba());
        self.hud_border.set_color(theme::surface_selected().rgba_bytes());
        self.hud_card.set_color(theme::base_300().rgba_bytes());
        // WHICH-KEY panel elevation re-tints with the world (same tokens as the
        // shared float panel: shadow ink, raised surface-step border, base_300 card).
        self.wk_shadow.set_color(float_shadow_srgba());
        self.wk_border.set_color(theme::surface_selected().rgba_bytes());
        self.wk_card.set_color(theme::base_300().rgba_bytes());
        // FORMAT POPOVER's active-button wash re-tints with the world (a `base_200`
        // value step, never amber). O(1); geometry is theme-independent. The
        // popover's ELEVATION trio is the shared `float_*` pipelines (re-tinted
        // below, alongside the caret-preview panel / spell popup / search panel
        // that already ride them) — no dedicated popover elevation tokens anymore.
        self.popover_wash.set_color(theme::base_200().rgba_bytes());
        // SELF-DEMONSTRATING buttons: `A`'s pill re-tints from the doc highlight
        // wash's own derivation (+ the one-bit dither density — a switch AWAY
        // from a one-bit world must reset it, mirroring `wash_highlight_pipeline`);
        // `S`'s line from THE strike ink.
        self.popover_hl_wash.set_color(highlight_wash_rgba_bytes());
        self.popover_hl_wash.set_dither(wagtail_dither_density());
        self.popover_hl_wash
            .set_dither_cell(wagtail_stipple_cell_px(self.dpi));
        self.popover_strike.set_color(strike_srgba_bytes());
        // WEB/LINUX MENU BAR: re-tint from the world's own tokens (O(1) — the bar/
        // dropdown GEOMETRY is theme-independent, so the theme-picker preview re-tints
        // it for free). Bar ground = a value step off the room (`base_200`); the open
        // title's highlight + the dropdown border = `surface_selected`; the dropdown
        // card = `base_300` (risen a step); the separator hairline = `muted`. NEVER
        // amber — figure/ground by value only (DESIGN §3/§4). The title/item text ink
        // (faint / muted / content) is re-read live at prepare time.
        self.menubar_bg.set_color(theme::base_200().rgba_bytes());
        // The open title's highlight band color tracks the world here so a live
        // theme switch reskins it even between menu opens; `prepare_menubar`
        // OVERRIDES it per-frame from `highlight_treatment` — a true 1-bit world
        // fills the band with solid `base_content` and recolors the open title's
        // glyphs to `base_300` (see `HighlightTreatment::InverseFill`), never the
        // old framebuffer invert of the title text.
        self.menubar_hi.set_color(theme::selection().rgba_bytes());
        self.menu_drop_shadow.set_color(float_shadow_srgba());
        self.menu_drop_border.set_color(theme::surface_selected().rgba_bytes());
        self.menu_drop_card.set_color(theme::base_300().rgba_bytes());
        self.menu_drop_sep.set_color(theme::muted().rgba_bytes());
        self.panel_caret.set_color(theme::primary().rgb_bytes());
        self.caret_preview_pipeline
            .set_color(theme::primary().rgb_bytes());
        self.caret_preview_glyph_pipeline
            .set_color(theme::primary().rgb_bytes());
        self.float_shadow.set_color(float_shadow_srgba());
        self.float_border
            .set_color(theme::surface_selected().rgba_bytes());
        self.float_card.set_color(theme::base_300().rgba_bytes());
        // DIFF-AS-PREVIEW panel: shadow/card re-tint here; the BORDER color is
        // re-decided every `prepare_diff_panel` (it carries the focus cue).
        self.diffpanel_shadow.set_color(float_shadow_srgba());
        self.diffpanel_card.set_color(theme::base_300().rgba_bytes());
        self.overlay_rows.set_color(theme::selection().rgba_bytes());
        // PER-ITEM LIST SURFACES: the bar surfaces re-tint to the new world's
        // quiet value step (their real per-frame color is set at draw time from the
        // effective bar tokens; this keeps a parked pipeline coherent on a switch).
        self.overlay_bars
            .set_color(theme::surface_selected().rgba_bytes());
        // ARM B LIVING-BAND PROBE — keep the two-shape crossing quad coherent on a
        // world switch (its real per-frame color is re-read at draw time). Parked
        // empty on every ordinary run, so this is inert there.
        self.overlay_cross
            .set_color(theme::overlay_band_overlap().rgba_bytes());
        // The theme picker's active-lens underline re-tints to the new world's ink (it
        // is drawn while the picker is up AND the world previews live, so the hairline
        // tracks the previewed world's ink).
        self.overlay_lens_underline
            .set_color(theme::base_content().rgba_bytes());
        self.spell_pipeline.set_color(theme::error().rgba_bytes());
        // Re-tint the WRITING-NIT underline to the new world's MUTED ink.
        self.nit_pipeline.set_color(nit_underline_srgba());
        // Re-tint the `~~strikethrough~~` line from THE strike-ink owner (the
        // struck text's own muted transform re-reads the theme each reshape).
        self.strike_pipeline.set_color(strike_srgba_bytes());
        // Re-tint the quiet link UNDERLINE from its own ink owner (the same
        // muted rung as the strike, decoupled instance).
        self.link_underline_pipeline.set_color(link_underline_srgba_bytes());
        // Re-tint the PAGE-MODE margin ground to the new world's tokens.
        self.background_pipeline.set_gradient(background_desc());
        // THE PAGE FRAME: re-tint from the one ink owner (`base_content`).
        // Geometry is re-prepared each frame (`prepare_page_frame`), so a
        // world switch re-tints AND re-gates (a None world uploads zero
        // rects) for free. The dither density stays the construction-time
        // 1.0 (a hard-edged full fill — never a translucent AA rim).
        self.page_frame_pipeline
            .set_color(theme::page_frame_ink().rgba_bytes());
        // THE STIPPLE PLACARD: re-tint the pixel ink + re-derive the density
        // from the new world's own ladder (both one-owner derivations).
        self.placard_stipple
            .set_color(theme::placard_ink(theme::PlacardInk::Stipple).rgba_bytes());
        self.placard_stipple
            .set_dither(theme::placard_stipple_density());
    }

    /// Does the document carry any per-span text color that was BAKED from the
    /// theme palette and would go stale on a same-face world hop? Only such spans
    /// need the theme-driven re-bake: SYNTAX role tints and markdown MARKUP dim/style
    /// spans. Plain prose body text sets NO
    /// `color_opt` ([`Self::doc_attrs`]) and reads the live active ink each frame,
    /// so a color-less buffer must NOT pay a wasted reshape on a same-face switch.
    fn has_baked_theme_colors(&self) -> bool {
        !self.syn_spans.is_empty() || !self.md_spans.is_empty()
    }

    /// Would [`Self::sync_theme_font`] actually re-shape — because the ACTIVE
    /// world's effective display face differs from the one the document is shaped
    /// in, OR its palette differs from the one the per-span colors were baked under
    /// ([`Self::shaped_theme`]) AND the document actually carries baked color spans?
    /// A restyle re-bakes BOTH the glyph shapes and the syntax/markdown span
    /// colors, so a same-FACE world hop still needs it when the palette changed on a
    /// buffer that bakes colors (else stale colors — the Magpie -> Bombora bug); a
    /// color-less prose buffer stays free (its ink reads live). Lets the live preview
    /// arm its settle-deferral only when a real restyle is pending.
    pub fn needs_theme_reshape(&self) -> bool {
        self.doc_family() != self.shaped_font
            || (theme::active_index() != self.shaped_theme && self.has_baked_theme_colors())
    }

    /// The FONT half of a theme switch (the expensive half — a full-document
    /// reshape; the theme-burst profile measured it dominating every picker
    /// preview step, which is why the live preview defers it to a settle).
    ///
    /// Re-shape the whole document when the new world uses a DIFFERENT effective
    /// display face than the one the document is shaped with (so the glyph SHAPES
    /// switch — mono <-> serif <-> sans <-> slab) OR a DIFFERENT palette than the
    /// one the per-span text colors were baked under (so a same-face world hop still
    /// re-tints the syntax/markdown spans — the Magpie -> Bombora stale-color
    /// bug). The text + zoom are unchanged, so `restyle_all_lines` (below) re-lays
    /// every line's attrs in the new family + span colors and reshapes once. A hop
    /// to the SAME world (an idle re-preview back) skips this and stays free.
    /// Compares the EFFECTIVE face (`doc_family` → the world's mono on a CODE
    /// buffer, else its display font), so two worlds that share a display font but
    /// differ in `mono` (e.g. Quokka/Bowerbird, both IBM Plex Sans) still reshape
    /// a code buffer when their mono differs; and two worlds that share the effective
    /// face but differ in palette still reshape to re-bake the span colors.
    pub fn sync_theme_font(&mut self) {
        let new_font = self.doc_family();
        let new_theme = theme::active_index();
        // Reshape when the effective FACE changed (glyph shapes) OR the world's
        // PALETTE changed on a buffer that BAKES per-span colors (syntax/markdown —
        // those were frozen under `shaped_theme` and go stale on a same-face world
        // hop; a color-less prose buffer reads its ink live and needs nothing).
        // Either way the cure is one `restyle_all_lines` — it re-lays every line's
        // attrs (family + colors) and reshapes once. A same-face, same-world call
        // stays a no-op via this compare, mirroring the original `shaped_font` guard.
        let theme_recolor = new_theme != self.shaped_theme && self.has_baked_theme_colors();
        if new_font != self.shaped_font || theme_recolor {
            self.theme_font_adopt(new_font, new_theme);
            self.restyle_all_lines();
        }
    }

    /// The FONT-phase reconfigure of a theme switch: bump the reshape count, adopt the
    /// new effective face + palette generation, and rewrap the document to the new
    /// face's column. The ONE owner of this step, shared by [`Self::sync_theme_font`]
    /// and the timed [`Self::sync_theme_font_timed`] (so the two can never drift). The
    /// following `restyle_all_lines` does the actual shape + row-geom invalidation.
    ///
    /// NOTE: the redundant `buffer.set_text` (a WHOLE-document cosmic-text reshape in
    /// the new plain family) was dropped here — `restyle_all_lines` ALREADY re-lays
    /// every line's attrs in the new family (via `doc_attrs()`) AND covers the per-line
    /// markdown / heading / CJK spans, then reshapes the document. The old `set_text`
    /// shaped every line in the new face only to have `restyle_all_lines` immediately
    /// re-lay + reshape it again — one full reshape per theme-preview step for nothing.
    /// The text is unchanged by a theme switch, so the buffer already holds it; we only
    /// need the new wrap size + the restyle. Byte-identical (same final attrs/shape).
    /// Re-derive the wrap width from the live page COLUMN, never the buffer's own
    /// (possibly stale) size — preserving `self.buffer.size().0` here would carry a
    /// divergent edge-to-edge width through a theme switch and leave the page running
    /// off the right edge. Set it BEFORE restyling so the new-face reshape wraps at the
    /// right width.
    fn theme_font_adopt(&mut self, new_font: &'static str, new_theme: usize) {
        self.reshape_count += 1;
        self.shaped_font = new_font;
        self.shaped_theme = new_theme;
        let width = Some(self.text_wrap_width());
        let shape_h = self.full_shape_height();
        self.buffer
            .set_size(&mut self.font_system, width, Some(shape_h));
    }

    /// LIVE-ONLY (DEBUG settle readout): run the SAME work as [`Self::sync_theme_font`]
    /// — the identical guard, the identical `theme_font_adopt` + `restyle_all_lines`
    /// steps — but stamp each phase boundary and return the reshape-side phase millis
    /// (font-adopt, reshape, row-geom), or `None` when the guard finds NO work (so a
    /// no-op switch never clobbers the last meaningful readout). The caller (the live
    /// App, behind `debug_on()`) folds the present-side atlas + present phases in on the
    /// settled frame. The plain `sync_theme_font` — the ONLY variant the headless path
    /// calls — reads no clock, so a capture never touches an `Instant` here.
    ///
    /// The row-geom walk is FORCED here (a plain reshape leaves it lazy for the next
    /// prepare) purely so its cost is timed as its own phase — identical work moved a
    /// few microseconds earlier, warming the cache the frame's prepare would rebuild
    /// anyway, so the rendered frame stays byte-identical.
    pub fn sync_theme_font_timed(&mut self) -> Option<crate::themeswitch::SwitchPhases> {
        use crate::clock::Instant; // wasm-safe (`web_time` on wasm); native `std`.
        use crate::themeswitch::{SwitchPhase, SwitchPhases};
        let new_font = self.doc_family();
        let new_theme = theme::active_index();
        let theme_recolor = new_theme != self.shaped_theme && self.has_baked_theme_colors();
        if new_font == self.shaped_font && !theme_recolor {
            return None; // no reshape work — nothing to time, keep the last readout.
        }
        let ms = |d: std::time::Duration| d.as_secs_f32() * 1000.0;
        let t0 = Instant::now();
        self.theme_font_adopt(new_font, new_theme);
        let t1 = Instant::now();
        self.restyle_all_lines();
        let t2 = Instant::now();
        let _ = self.row_geom.total_height(&self.buffer, &self.metrics);
        let t3 = Instant::now();
        let mut phases = SwitchPhases::default();
        phases.record(SwitchPhase::Font, ms(t1 - t0));
        phases.record(SwitchPhase::Reshape, ms(t2 - t1));
        phases.record(SwitchPhase::RowGeom, ms(t3 - t2));
        Some(phases)
    }

    /// Apply the editor view snapshot: text, cursor, scroll, zoom, selection,
    /// preedit. When a preedit (IME composition) is active it is spliced into the
    /// shaped text at the cursor so it renders with real glyphs; the caret is then
    /// placed at the preedit's end and an underline is drawn beneath it.
    pub fn set_view(&mut self, view: &ViewState) {
        // Apply zoom first: if it changed, reset the glyphon buffer metrics and
        // re-shape so glyph layout matches the zoomed caret + selection rects. The
        // metrics fold in the display DPI (`self.dpi`, set by `set_dpi`) on top of
        // the user zoom, so the live page scales correctly on a HiDPI screen.
        let new_metrics = Metrics::with_dpi(view.zoom, self.dpi);
        // Re-shape on ANY pixel-metric change (zoom OR dpi); compare a metric that
        // carries both rather than the (zoom-only) `zoom` field.
        let zoom_changed = (new_metrics.font_size - self.metrics.font_size).abs() > f32::EPSILON;
        self.metrics = new_metrics;
        if zoom_changed {
            self.buffer
                .set_metrics(&mut self.font_system, self.metrics.glyph_metrics());
            // The shaping height budget is in (zoomed) pixels, so a zoom change
            // must re-grow the buffer's shaping height to keep the WHOLE document
            // shaped (fewer rows fit per pixel at higher zoom). The wrap width is
            // recomputed from the PAGE-MODE column: zoom changed the glyph advance,
            // so a measure-derived column is wider/narrower in px and must re-wrap.
            let width = Some(self.text_wrap_width());
            let shape_h = self.full_shape_height();
            self.buffer
                .set_size(&mut self.font_system, width, Some(shape_h));
            // Row geometry is in (zoomed) line-height units, so the cached
            // total-visual-row count is stale after a zoom change.
            self.row_geom.invalidate();
        }
        // MORPH caret: before the cursor advances, capture the CacheKey of the
        // glyph the caret is LEAVING so the silhouette can cross-fade from it to
        // the newly-inhabited glyph during the glide. Read through the ONE
        // inhabited-key seam (`caret_inhabited_key` — the caret's ANCHOR column,
        // for Morph one char BACK of the insertion point; Block/I-beam the cursor
        // column; `None` at a Morph LINE START, where the caret was the thin
        // insertion bar and inhabited NO glyph, so leaving col 0 fades in the new
        // glyph from nothing rather than from the un-inhabited char ahead),
        // derived with the STILL-LATCHED look and the OLD cursor, so from/to stay
        // anchor-consistent across the move. Only latch on a real cursor move
        // (not a same-position reshape); the buffer is still shaped in the OLD
        // state here, so this reads the correct outgoing glyph.
        let cursor_moved =
            view.cursor_line != self.cursor_line || view.cursor_col != self.cursor_col;
        let from_key = if cursor_moved {
            self.caret_inhabited_key()
        } else {
            // No move: keep the prior from-key so an in-flight glide keeps fading.
            self.caret_from_key
        };
        self.cursor_line = view.cursor_line;
        self.cursor_col = view.cursor_col;
        self.caret_affinity = view.caret_affinity;
        self.caret_from_key = from_key;
        // Re-latch the effective caret LOOK for this frame (see the field doc):
        // the anchor geometry below — including the spring target — reads the
        // latched value, one global read per frame. A live text-selection DRAG
        // overrides the configured look to the thin insertion BAR (the I-beam
        // form) for the duration of the drag — item 33's drag-bar. This is the
        // ONE seam that resolves the effective look, so every reader (geometry
        // AND the paint path, which read `self.caret_look`) sees the same form.
        self.caret_look = if view.selecting_drag {
            CaretMode::Ibeam
        } else {
            crate::caret::mode()
        };
        self.sync_view_fields(view);
        // MARKDOWN STYLING gate: copy the buffer's markdown-ness BEFORE shaping so
        // the per-line span pass sees it. A flip (switching between a `.md` and a
        // non-md buffer with — unusually — the SAME text) must force a reshape, as
        // the composed-string compare would otherwise skip restyling.
        let md_changed = self.md_enabled != view.is_markdown;
        self.md_enabled = view.is_markdown;
        // SYNTAX HIGHLIGHTING gate: copy the buffer's language BEFORE shaping so the
        // per-line span pass sees it. A flip (switching to/from a code language on
        // the same text) must force a reshape + restyle, since the composed-string
        // compare and the incremental line diff would otherwise skip restyling.
        let syn_changed = self.syn_lang != view.syn_lang;
        self.syn_lang = view.syn_lang;
        // WYSIWYG / INLINE-IMAGES gate: these two rendering globals bake into each
        // line's attrs (conceal zero-width metrics / image row heights) at shape
        // time, so a live flip on UNCHANGED text (a settings-menu toggle) must force
        // a reshape + restyle the incremental diff can't catch — the same shape as
        // `md_changed` / `syn_changed`. Latched here so any producer of the flip
        // (settings menu, a future command, a config reload) applies on the next frame.
        let wysiwyg_changed = self.wysiwyg_latched != crate::markdown::wysiwyg_on();
        self.wysiwyg_latched = crate::markdown::wysiwyg_on();
        let inline_images_changed =
            self.inline_images_latched != crate::markdown::inline_images_on();
        self.inline_images_latched = crate::markdown::inline_images_on();
        // INLINE-IMAGE DRAG-RESIZE (live only): a live-preview width override was
        // just (un)set on UNCHANGED text — force the reshape that re-runs
        // `compute_image_layout` so the dragged image re-fits at the new width. Taken
        // here (one-shot) exactly like the wysiwyg/inline-images force latches.
        let image_preview_dirty = std::mem::take(&mut self.image_preview_dirty);
        let render_flag_changed = wysiwyg_changed || inline_images_changed || image_preview_dirty;
        // i18n: the Han-ambiguity tiebreak ladder (config `cjk_priority`), read
        // by the per-run render resolution ladder on the NEXT reshape — a
        // live config change with no accompanying text edit applies on the
        // document's next edit/reshape rather than forcing one immediately (a
        // narrow, accepted scope trim; `doc_lang` itself is always current,
        // since it is re-derived from the text on every reshape below).
        self.cjk_priority = view.cjk_priority.clone();
        // Shape the document text with any active preedit spliced in at the cursor.
        // This is the ONE place a reshape may happen; it is skipped when neither the
        // composed (text+preedit) string NOR the zoom changed, so cursor moves,
        // scrolling, selection changes, and spell-span refreshes are all free.
        let reshape_before = self.reshape_count;
        self.shape_with_preedit(
            &view.text,
            zoom_changed || md_changed || syn_changed || render_flag_changed,
        );
        // Did a reshape actually happen this push? (A text edit reshapes; a pure
        // cursor move / scroll / selection change does not.) Feeds the
        // reveal-on-cursor conceal rescan below, which a reshape must force since it
        // drops the per-line attrs.
        let reshaped = self.reshape_count != reshape_before;
        // HEADING SIZE: heading rows carry absolute per-span metrics, so we must
        // rebuild line attrs in two cases the incremental text path can't catch on
        // its own: (1) a ZOOM/DPI change rescales the body but not the absolute
        // heading metrics (gated to a heading doc so the common path pays nothing);
        // (2) the markdown gate FLIPPED on UNCHANGED text (the diff rebuilds no
        // lines, so stale md/heading attrs would linger).
        //
        // This MUST run before `set_caret_target` below (see the bug it fixed): the
        // caret's row-geometry reads (`cursor_row_height`/`caret_cell_top`, via
        // `visual_rows`/`row_geom`) walk the buffer's CURRENTLY-shaped runs, and on
        // a heading doc those runs are briefly INCONSISTENT right after
        // `shape_with_preedit` — body text reshaped at the new zoom, but the
        // heading line's absolute per-span pixel metrics are still the OLD size
        // until this restyle rescales them. Latching the caret's spring target
        // from that transient state (the old ordering) left the caret floating at
        // the heading row's PRE-zoom position, never catching up once the text
        // re-laid moments later — the amber block caret drifting off the glyphs on
        // a zoomed heading line. Computing the target AFTER the restyle reads the
        // one, final, settled geometry.
        let restyled = if md_changed
            || syn_changed
            || render_flag_changed
            || (zoom_changed && self.has_heading_lines())
        {
            self.restyle_all_lines();
            true
        } else {
            false
        };
        // WYSIWYG v1.1: a reveal/conceal toggle can change actual glyph GEOMETRY
        // now (the zero-width metrics override), not just color, so this MUST
        // also run before `set_caret_target` below — the EXACT same ordering bug
        // `restyled` above was already moved earlier to avoid: a pure cursor move
        // onto/off a concealable line (heading/emphasis/code/highlight) reshapes
        // that line's glyphs, and latching the caret's spring target from the
        // stale PRE-toggle geometry (the old ordering) would leave the caret one
        // step behind the just-revealed/concealed row until some unrelated event
        // caught it up. Calling it here settles the geometry first.
        self.refresh_rule_conceal(reshaped || restyled);
        // Update the spring target so a cursor move starts a glide (the first
        // call snaps, per CaretAnim::set_target). Pass whether this move was an
        // edit so typing slides as a plain block (no underline).
        self.set_caret_target(view.is_edit_move, view.held);
    }

    /// Set the FILTERED document row the pointer is hovering (LIVE only — the app
    /// derives it from the pointer; the headless capture never calls this, so hover
    /// stays `None` there). Returns whether it CHANGED, so the caller can schedule a
    /// redraw only when a collapsed heading's chevron reveal actually flips.
    pub fn set_hover_line(&mut self, line: Option<usize>) -> bool {
        if self.hover_line == line {
            return false;
        }
        self.hover_line = line;
        true
    }

    /// Copy the plain (non-metric, non-caret-latch) editor view fields — scroll,
    /// selection/preedit, spell, search, overlay, and project status — into the
    /// renderer's mirror of the view snapshot.
    fn sync_view_fields(&mut self, view: &ViewState) {
        self.scroll_lines = view.scroll_lines;
        self.image_base_dir = view.doc_dir.clone();
        self.selection = view.selection;
        // COLLAPSED-HEADING TAILS: mirror the fold-tail rows so the ornament pass can
        // hang each "… N lines" glyph (+ caret/hover chevron). `hover_line` is a
        // pointer fact set separately (live only), NOT carried on the view.
        self.fold_tails = view.fold_tails.clone();
        self.preedit = view.preedit.clone();
        // Mirror the spell list ONLY when it actually changed (a rescan landing),
        // bumping its version so the cached squiggle protos rebuild; the common
        // cursor-move / scroll event keeps the mirror, the clone, AND the cache.
        if self.misspelled != view.misspelled {
            self.misspelled = view.misspelled.clone();
            self.spell_gen = self.spell_gen.wrapping_add(1);
        }
        self.search_active = view.search_active;
        self.search_matches = view.search_matches.clone();
        self.search_query = view.search_query.clone();
        self.search_current = view.search_current;
        self.search_case_sensitive = view.search_case_sensitive;
        self.search_replace_active = view.search_replace_active;
        self.search_replacement = view.search_replacement.clone();
        self.search_editing_replacement = view.search_editing_replacement;
        // FORMAT POPOVER: mirror the model (built by the App / capture probe); the
        // geometry is (re)computed in `prepare_popover`, which also parks the quads
        // when this is `None`.
        self.popover_model = view.popover.clone();
        // A summoned overlay appears + disappears INSTANTLY (no rise-in / sink-out
        // motion) on every CALM world: the overlay content syncs verbatim from the
        // view every frame, so a close snaps the card off the frame the App clears
        // its logical `self.overlay`. THE ONE exception is the MOTION-JUICE
        // entrance (FIRETAIL-MAXIMALIST-SHOWCASE round): on an OPEN flip
        // (false→true), a live-armed pipeline whose effective `MotionJuice`
        // asks for `SpringIn` kicks the ~200ms drop-in spring. Every headless
        // pipeline is unarmed (`juice_live` false — see `arm_live_juice`), so
        // this branch is STRUCTURALLY unreachable in a capture and the settled
        // state stays byte-identical; Reduce Motion folds the kick on the very
        // next step (`step_overlay_juice`). A CLOSE flip resets both animators
        // to settled so a stale mid-flight state can never greet a re-summon.
        let overlay_opened = view.overlay_active && !self.overlay_active;
        let overlay_closed = !view.overlay_active && self.overlay_active;
        self.overlay_active = view.overlay_active;
        // ITEM 45 — the alignment FROZEN at summon rides through verbatim (`Some`
        // while open, `None` closed): the render-path anchor readers resolve it via
        // `resolve_overlay_anchor`, so an open card never relocates on a preview cross.
        self.overlay_align = view.overlay_align;
        if overlay_opened
            && self.juice_live
            && !crate::motion::reduced()
            && crate::render::effective_motion_juice().entrance
                == theme::OverlayEntrance::SpringIn
        {
            self.overlay_enter_t = 0.0;
        }
        if overlay_closed {
            self.overlay_enter_t = 1.0;
            self.overlay_band_t = 1.0;
            self.overlay_band_last = None;
        }
        self.overlay_crisp = view.overlay_crisp;
        self.overlay_query = view.overlay_query.clone();
        self.overlay_title = view.overlay_title;
        self.overlay_items = view.overlay_items.clone();
        self.overlay_empty = view.overlay_empty.clone();
        self.overlay_bindings = view.overlay_bindings.clone();
        self.overlay_times = view.overlay_times.clone();
        self.overlay_git = view.overlay_git.clone();
        self.overlay_selected = view.overlay_selected;
        self.overlay_scroll = view.overlay_scroll;
        self.overlay_window_rows = view.overlay_window_rows;
        self.overlay_hint = view.overlay_hint.clone();
        self.overlay_lens = view.overlay_lens.clone();
        self.overlay_sections = view.overlay_sections.clone();
        self.overlay_spell = view.overlay_spell;
        self.diff_panel = view.diff_panel;
        self.diff_panel_focus = view.diff_panel_focus;
        // Measure the widest suggestion NOW (a `&mut FontSystem` is in hand) so the
        // contextual spell panel can size its card to the longest correction, not the
        // anchor word. Cheap + gated: only shaped when the SPELL panel is the open
        // overlay; otherwise the cached width is cleared to 0.
        self.overlay_spell_w = if self.overlay_spell.is_some() {
            self.measure_spell_content_w()
        } else {
            0.0
        };
        // ITEM 51 — a RIGHT-ANCHORED takeover card shrinks to hug its content, so
        // measure the widest visible primary (+ optional secondary column, query
        // line, lens strip and footer) NOW, with a `&mut FontSystem` in hand. Gated
        // to the right-anchored takeover cards (the frozen anchor mirrors growth):
        // a left/center card, the contextual spell popup, or a closed overlay leaves
        // the cache `0.0`, so `overlay_desired_w` falls back to the fixed wide cap —
        // byte-identical. Reset FIRST so the provisional geometry the measurement
        // shapes into uses the wide cap (not last frame's hug width).
        self.overlay_content_w = 0.0;
        if self.overlay_active && self.overlay_spell.is_none() && self.overlay_right_anchored() {
            self.overlay_content_w = self.measure_overlay_content_w();
        }
        // CARET-STYLE PICKER preview: mirror which look the picker highlights (None
        // when it is closed). Keep the preview animator's look in step with it so the
        // SAME loop animates in whatever style the highlighted row selects; the loop
        // itself is driven by `advance` (live) / settled by `prepare` (headless).
        self.caret_preview = view.caret_preview;
        match view.caret_preview {
            Some(look) => self.caret_demo.mode = look,
            // Picker closed: reset the demo so a fresh summon re-types the line from
            // beat 0 (and nothing animates while closed — back to perfect idle).
            None => self.caret_demo.reset(),
        }
        self.gutter_name = view.gutter_name.clone();
        self.gutter_project = view.gutter_project.clone();
        self.notice = view.notice.clone();
        // LINE ENDINGS: mirror the buffer's on-disk ending (a pure fact, no reshape
        // needed) so the held stats HUD + sidecar report the active buffer's EOL.
        self.eol = view.eol;
    }

    /// Set the display DPI `scale_factor` (live app only; the capture leaves it at
    /// 1.0). Folds the new scale into the metrics on top of the current user zoom
    /// and re-shapes the document at the rescaled column width, so the page keeps its
    /// proportions (≈10% margin, capped column, larger glyphs) on a HiDPI monitor and
    /// across a monitor change. A no-op when the scale is unchanged. See
    /// [`Metrics::with_dpi`]; the per-frame `set_view` reads `self.dpi` thereafter.
    pub fn set_dpi(&mut self, dpi: f32) {
        if (dpi - self.dpi).abs() < f32::EPSILON {
            return;
        }
        self.dpi = dpi;
        // CHUNK round: the Wagtail highlight stipple's cell is PHYSICAL px, so a
        // display-scale change must re-push it (the density/color don't move on
        // a DPI change, so `sync_theme_colors` isn't otherwise called here) —
        // else the stipple would keep the OLD monitor's block size after a
        // monitor move. A no-op `1.0` off a one-bit world.
        let stipple_cell = wagtail_stipple_cell_px(dpi);
        self.match_pipeline.set_dither_cell(stipple_cell);
        self.wash_highlight_pipeline.set_dither_cell(stipple_cell);
        self.popover_hl_wash.set_dither_cell(stipple_cell);
        // Rebuild the metrics from the SAME user zoom (already clamped in the stored
        // metrics) with the new scale, then re-shape exactly like a zoom change.
        self.metrics = Metrics::with_dpi(self.metrics.zoom, dpi);
        self.buffer
            .set_metrics(&mut self.font_system, self.metrics.glyph_metrics());
        let width = Some(self.text_wrap_width());
        let shape_h = self.full_shape_height();
        self.buffer
            .set_size(&mut self.font_system, width, Some(shape_h));
        self.row_geom.invalidate();
        // Heading rows carry absolute per-span metrics; a DPI change must rebuild
        // them to rescale (same reason as the zoom path in `set_view`).
        if self.has_heading_lines() {
            self.restyle_all_lines();
        }
    }

    pub fn set_size(&mut self, width: f32, height: f32) {
        // Width drives soft-wrap (text wraps to the viewport width). We manage
        // vertical scroll ourselves via the draw offset (`doc_top`), so the
        // buffer's own scroll stays at 0 and we never rely on it to clip.
        //
        // The HEIGHT we hand cosmic-text is NOT the window height: cosmic-text
        // only lays out (and yields from `layout_runs()`) the rows that fit in
        // the buffer's height starting at its scroll. To make scrolling, overlay
        // placement, and the total-visual-row count correct for a scrolled or
        // long wrapped document, the WHOLE document must be shaped — so we pass a
        // generous height that covers every visual row. These docs are small, so
        // shaping the whole buffer is cheap. The real window `height` only bounds
        // what we DRAW (via `TextBounds` in `prepare`), not what we shape — we keep it
        // only for the DEBUG panel's `viewport WxH` readout.
        self.window_h = height;
        // Record the real window width FIRST so the column geometry derives from
        // it; then wrap the text at the (possibly narrower, centered) COLUMN width
        // rather than the whole window — that is the centered writing measure.
        self.window_w = width;
        // Remember the buffer's CURRENT wrap size so we can tell whether this call
        // actually re-wraps (cosmic-text no-ops on an unchanged size).
        let before = self.buffer.size();
        let shape_h = self.full_shape_height();
        let wrap_w = self.text_wrap_width();
        self.buffer
            .set_size(&mut self.font_system, Some(wrap_w), Some(shape_h));
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        // A CHANGED wrap size re-laid the document's runs, so every row-geometry
        // cache (row tops/heights/total, the cursor-line VisualRow memo) is stale.
        // This is the LIVE window-resize / page-mode-toggle / page-width seam: the
        // following `prepare`'s `sync_wrap_width` sees the width already in sync and
        // skips its own invalidate, so without this the scroll math, caret row, and
        // hit-tests keep answering from the PRE-RESIZE geometry until the next text
        // edit. (The headless capture sets its size before the text, so this only
        // ever fires on a real geometry change — captures stay byte-identical.)
        let changed = |a: Option<f32>, b: Option<f32>| match (a, b) {
            (Some(x), Some(y)) => (x - y).abs() > 0.5,
            (None, None) => false,
            _ => true,
        };
        if changed(before.0, Some(wrap_w)) || changed(before.1, Some(shape_h)) {
            self.row_geom.invalidate();
        }
        // TABLES: `set_size` just re-wrapped the document buffer to the new
        // width DIRECTLY (above), so by the time `prepare()`'s own
        // `sync_wrap_width` runs, `buffer.size().0` already equals
        // `text_wrap_width()` — its own drift check is false, and its
        // table-resync companion (`resync_table_layout_for_width`) never
        // fires. Without this, a real window resize (this is the ONLY seam
        // `WindowEvent::Resized` drives — a page-measure edit goes through
        // `sync_wrap_width` alone and is already covered) leaves
        // `TableGridCache` pinned to whatever width the last full `set_text`
        // reshape used, so a shrunk window keeps drawing the OLD (too-wide)
        // column geometry — the real user-reported overflow. Gated on the
        // SAME `changed(...)` width check above: a height-only resize (or no
        // real change at all) never re-shapes tables it doesn't need to.
        if changed(before.0, Some(wrap_w)) {
            self.resync_table_layout_for_width();
        }
    }

    pub fn line_count(&self) -> usize {
        self.buffer.lines.len()
    }
}
