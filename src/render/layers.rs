//! PER-LAYER PREPARE ORCHESTRATION — the per-frame `prepare_*_layer` steps the
//! aggregating [`TextPipeline::prepare`] (which stays in `render.rs`) folds together:
//! the background, the document text, the animated caret, the selection / search
//! highlights, the chrome (panel / overlay / gutter / readouts), and the spell
//! underlines. Each uploads ONE layer's instances / glyphs into its glyphon
//! renderer or GPU pipeline against the shared atlas / viewport / queue.
//!
//! These are inherent methods on [`super::TextPipeline`] — they ARE the GPU
//! aggregation that is the pipeline's reason for being, driving its renderers /
//! pipelines / buffers, so they CANNOT become free functions. This module is purely
//! a physical home for that cohesive per-layer cluster, carved out of `render.rs`
//! verbatim; a child module sees its ancestor's private items, so the methods keep
//! full access to `TextPipeline`'s fields/helpers (and the sibling `rects` builders)
//! with NO behaviour change — the rendered frame is byte-identical.

use super::*;

/// How much bigger than body ink a centered thematic-break ornament (`❧`/`⁂`/`❦`)
/// is shaped. A modest bump so the break reads as a present-but-quiet flourish —
/// still MUTED, never amber (DESIGN §3).
const ORNAMENT_SCALE: f32 = 1.45;

impl TextPipeline {
    /// Per-frame PAGE-MODE margin gradient: punch a hole for the page column and
    /// paint the margins (the whole canvas, no margins, when page mode is off).
    pub(super) fn prepare_background_layer(&mut self, queue: &wgpu::Queue, width: u32, height: u32) {
        // PAGE MODE margin gradient: punch a hole for the page column so the flat
        // base_100 clear shows there, and paint the margins. When page mode is OFF
        // we pass `col_w == width` so the column covers everything and the margins
        // vanish (identical to the old flat clear).
        let (page_on, _measure, col_left, col_w) = self.page_geometry();
        let (bg_left, bg_w) = if page_on {
            (col_left, col_w)
        } else {
            (0.0, width as f32)
        };
        self.background_pipeline
            .prepare(queue, width, height, bg_left, bg_w);
    }

    /// Upload the document text layer with the FOCUS-MODE dim default color — the
    /// one glyphon `prepare` per frame (the caret is a quad drawn underneath).
    pub(super) fn prepare_text_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let doc_top = self.doc_top();

        // FOCUS MODE: the non-active text is dimmed for FREE by choosing the DIM ink
        // as the buffer's default_color — every glyph whose `color_opt` is None (the
        // whole document except the active unit, which carries explicit full-ink
        // spans) resolves to it at prepare time, exactly like a theme switch recolors
        // with no reshape. Off keeps the full-ink default (unchanged behavior).
        let default_color = if crate::focus::mode() == crate::focus::FocusMode::Off {
            theme::base_content().to_glyphon()
        } else {
            crate::focus::dim_srgb().to_glyphon()
        };
        let text_area = TextArea {
            buffer: &self.buffer,
            left: self.text_left(),
            top: doc_top,
            scale: 1.0,
            bounds,
            default_color,
            custom_glyphs: &[],
        };

        self.renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                // Text only; the caret is a GPU quad drawn underneath the text
                // in the render pass (clear -> caret -> text).
                [text_area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon prepare failed: {e:?}"))?;
        Ok(())
    }

    /// Select + upload exactly one caret look (block / morph silhouette / I-beam /
    /// glyphless bar) plus the cosmetic trail, clearing the unused pipelines.
    pub(super) fn prepare_caret_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        // The caret has two selectable LOOKS (block vs glyph-silhouette morph).
        // Exactly one of the two pipelines emits geometry per frame; the other is
        // cleared so nothing stale lingers when the mode (or fallback) changes.
        //
        // BLOCK: `caret_geometry` reads the spring's settle factor to interpolate
        // between the resting rounded square (full advance width) and the moving
        // trailing-underline streak, and the real glyph advance so a full-width CJK
        // glyph gets a full-width block (Latin keeps caret_w). Drawn UNDER the text.
        //
        // MORPH has three sub-cases, all keyed off the spring:
        //   * FAST MOTION (settle_factor < SHOW threshold) → DEFER to the BLOCK
        //     pipeline's trailing-underline STREAK. Holding an arrow / a big jump
        //     makes the spring lag, settle drops toward 0, and the streak shows; the
        //     per-glyph silhouette would strobe badly during travel, so we don't
        //     paint it until motion settles.
        //   * SETTLED on a real glyph → paint the accent SILHOUETTE (glyph pipeline,
        //     OVER the text) with its glyph-to-glyph cross-fade as it lands.
        //   * GLYPHLESS cell (space / end-of-line / empty line / emoji) → a SLIM
        //     accent bar via the BLOCK pipeline (a thin I-beam, not a full block).
        let mode = crate::caret::mode();
        let settle = self.caret.settle_factor();
        let has_glyph = mode == CaretMode::Morph && self.prepare_caret_masks(device, queue);
        let paint_silhouette = has_glyph && settle >= CARET_MORPH_SETTLE_SHOW;
        // MORPH on a glyphless cell (space / EOL / empty line). Gate the thin bar on
        // the SAME settle threshold the silhouette uses, NOT on `!is_animating()`:
        // the old `!is_animating()` gate meant that while the spring was still
        // settling onto a space the code fell through to the block ⇄ streak path,
        // so arriving on a space FLASHED the full block and only snapped to the thin
        // bar after motion fully stopped. Using `settle >= SHOW` makes a short hop
        // onto a space (settle stays high) resolve DIRECTLY to the thin bar with no
        // block frame, while a genuine fast glide (settle < SHOW) still streaks via
        // the final `else`.
        let paint_space_bar = mode == CaretMode::Morph && !has_glyph && settle >= CARET_MORPH_SETTLE_SHOW;
        if mode == CaretMode::Ibeam {
            // I-BEAM (prototype): a STEADY thin bar at the insertion point (no
            // breathing — fully static at rest), drawn via the block (rounded-quad)
            // pipeline at full opacity. Velocity squash/stretch (the elongating
            // comet) + the recoil kick ride the same spring as Block, so Block/Morph
            // paths are untouched.
            let (cx, cy, cw, ch, ccorner) = self.caret_ibeam_geometry();
            let (cw, ch, ccorner) = self.pop_scaled(cw, ch, ccorner);
            self.caret_pipeline
                .prepare(queue, width, height, cx, cy, cw, ch, ccorner);
            self.caret_glyph_pipeline.clear();
        } else if paint_silhouette {
            // Settled on a glyph: the accent silhouette recolours the letter.
            let (from_box, to_box, morph_t) = self.caret_glyph_geometry();
            self.caret_glyph_pipeline.prepare(
                device,
                queue,
                width,
                height,
                self.caret_mask_from.as_ref(),
                from_box,
                self.caret_mask_to.as_ref(),
                to_box,
                morph_t,
                1.0,
                CARET_MORPH_DILATE_PX * self.metrics.zoom,
            );
            self.caret_pipeline.prepare_empty();
        } else if paint_space_bar {
            // Settled (or short-hopped) onto a glyphless cell: a thin version of the
            // fat caret, CENTERED in the cell. Resolves directly here without a
            // full-block intermediate (see `paint_space_bar` above). A genuine fast
            // glide keeps `settle < SHOW` and falls to the streak in the final else.
            let (cx, cy, cw, ch, ccorner) = self.caret_space_bar_geometry();
            let (cw, ch, ccorner) = self.pop_scaled(cw, ch, ccorner);
            self.caret_pipeline
                .prepare(queue, width, height, cx, cy, cw, ch, ccorner);
            self.caret_glyph_pipeline.clear();
        } else {
            // BLOCK mode, OR MORPH deferring to the streak during fast travel: the
            // block pipeline's settle-driven square ⇄ trailing-underline streak,
            // oriented along the true travel vector (diagonal trails truly slant).
            // See [`prepare_caret_block`].
            self.prepare_caret_block(queue, width, height);
        }

        // COSMETIC | TRAIL: a fading accent streak from the OLD caret position to the
        // NEW, layered OVER the snapped caret. Independent of the caret's resting/morph
        // quad above and of the position (it spans the latched OLD→NEW points), so a
        // small move that SNAPS still shows the | . Empty when no streak is active, so
        // the deterministic `--screenshot` (trail-absent settled state) draws nothing.
        // See [`prepare_caret_trail`].
        self.prepare_caret_trail(queue, width, height);
    }

    /// BLOCK-caret upload — the settle-driven resting square ⇄ trailing-underline
    /// streak, oriented along the true travel vector. Folds in the DESCENDER-AWARE
    /// bottom so a dipping cursor glyph (g/y/p/q/j) stays inside the reverse-video
    /// block. The fast-travel MORPH path defers here too (the per-glyph silhouette
    /// would strobe), so this is the shared block/streak draw. Lifted verbatim out of
    /// [`prepare_caret_layer`]'s final dispatch arm; byte-identical.
    fn prepare_caret_block(&mut self, queue: &wgpu::Queue, width: u32, height: u32) {
        let (cx, cy, cw, ch, ccorner, ax, ay) = self.caret_geometry();
        // DESCENDER-AWARE BOTTOM (stable top): keep the block TOP fixed and drop
        // ONLY its bottom edge to cover the cursor glyph's real per-glyph
        // descender ink, so dippers (g/y/p/q/j) stay inside the reverse-video
        // block while a/m/C are unchanged (extend == 0 when the glyph doesn't dip
        // below the existing block bottom). Scaled by the settle factor so the
        // moving thin streak is untouched mid-glide; at rest (settled capture,
        // s == 1) the extension is deterministic.
        let s = self.caret.settle_factor();
        let descender = self.cursor_glyph_descender();
        // Pad a dipping glyph's descender a hair (pixel-scaled) so its antialiased
        // ink edge stays inside the block; non-dippers (descender 0) are untouched.
        let desc_pad = if descender > 0.0 {
            CARET_DESCENDER_PAD * (self.metrics.caret_h / CARET_H)
        } else {
            0.0
        };
        let block_bottom = cy + ch * 0.5;
        let desc_bottom = self.caret_baseline_y() + descender + desc_pad;
        let extend = (desc_bottom - block_bottom).max(0.0) * s;
        // `ch += extend; cy += extend/2` drops the bottom by `extend` while the
        // top (`cy - ch/2`) is invariant.
        let ch = ch + extend;
        let cy = cy + extend * 0.5;
        let (cw, ch, ccorner) = self.pop_scaled(cw, ch, ccorner);
        self.caret_pipeline
            .prepare_directed(queue, width, height, cx, cy, cw, ch, ccorner, ax, ay);
        self.caret_glyph_pipeline.clear();
    }

    /// COSMETIC | TRAIL upload — the fading accent streak from the latched OLD caret
    /// position to the NEW, layered OVER the snapped caret (so even a SNAP move shows
    /// the | ). Empty when no streak is active, so a deterministic `--screenshot`
    /// draws nothing. Lifted verbatim out of [`prepare_caret_layer`]; byte-identical.
    fn prepare_caret_trail(&mut self, queue: &wgpu::Queue, width: u32, height: u32) {
        match self.caret_trail_geometry() {
            Some((cx, cy, cw, ch, ccorner, ax, ay, alpha)) => {
                self.caret_trail_pipeline
                    .prepare_axis(queue, width, height, cx, cy, cw, ch, ccorner, alpha, ax, ay);
            }
            None => self.caret_trail_pipeline.prepare_empty(),
        }
    }

    /// Build + upload the selection / preedit, search-match, and horizontal-rule
    /// quads (each empty — so nothing lingers — when its feature is inactive).
    pub(super) fn prepare_selection_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        // Build the translucent selection highlight rectangles (one per visible
        // line of the region) plus any IME preedit underline, and upload them via
        // the same quad pipeline. Empty when there is no selection or preedit.
        let mut rects = self.selection_rects();
        rects.extend(self.preedit_rects());
        self.selection_pipeline
            .prepare(device, queue, width, height, &rects);

        // Search-match highlights (separate instance/color). Empty when search is
        // closed so no stale highlights linger.
        let mrects = if self.search_active {
            self.search_match_rects()
        } else {
            Vec::new()
        };
        self.match_pipeline
            .prepare(device, queue, width, height, &mrects);
    }

    /// Shape + upload the markdown ORNAMENTS: the world's PER-SYNTAX break glyph
    /// CENTERED in the writing column on each thematic-break line, AND the depth-derived
    /// `•`/`◦`/`▪` BULLET left-aligned over each unordered list line's marker cell
    /// (reveal-on-cursor: neither is drawn on the caret's own line). Both shape from the
    /// bundled [`SYMBOL_FAMILY`] face in muted ink and share this one quiet renderer.
    /// The break glyph — `---`/`***`/`___`
    /// each draw a DIFFERENT ornament from the active [`theme::Ornaments`] set (the
    /// fine-press section break that REPLACES the old thin rule line, chosen by which
    /// syntax the author typed). Each glyph is shaped from the bundled
    /// [`SYMBOL_FAMILY`] face (the mono/display faces lack them) in the MUTED ink,
    /// at a modest ORNAMENT_SCALE bump over the body size so a centered break reads
    /// with a touch more presence (quiet; amber stays the caret's). Uploads NO areas
    /// for a non-markdown buffer (`!md_enabled`), so a default capture stays
    /// byte-identical.
    pub(super) fn prepare_ornaments(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let m = self.metrics;
        let muted = theme::muted().to_glyphon();
        let left = self.text_left();
        let col_w = self.text_wrap_width().max(1.0);
        // Positions (computed from &self before the disjoint-field borrow split).
        // Each break carries the ornament its syntax picked (`---`/`***`/`___`).
        let rule_marks = if self.md_enabled {
            self.rule_marks()
        } else {
            Vec::new()
        };

        let attrs = Attrs::new()
            .family(Family::Name(SYMBOL_FAMILY))
            .color(muted);
        let center = Some(glyphon::cosmic_text::Align::Center);

        // The centered section-break glyph is shaped a touch BIGGER than body ink —
        // a calm, present flourish (still muted, never amber). Bumping the font size
        // while keeping the row's `line_height` keeps the glyph vertically centred on
        // its break row.
        let orn_metrics = GlyphMetrics::new(m.font_size * ORNAMENT_SCALE, m.line_height);

        // The breaks may mix syntaxes (`---` here, `***` there), so each needs its OWN
        // shaped glyph. Dedupe by ornament char — at most three distinct — into local
        // buffers the `TextArea`s below borrow; a doc with one break-style shapes once.
        let mut distinct: Vec<char> = Vec::new();
        for (_, ch) in &rule_marks {
            if !distinct.contains(ch) {
                distinct.push(*ch);
            }
        }
        let mut rule_buffers: Vec<GlyphBuffer> = Vec::with_capacity(distinct.len());
        for &ch in &distinct {
            let mut buf = GlyphBuffer::new(&mut self.font_system, orn_metrics);
            buf.set_size(&mut self.font_system, Some(col_w), Some(m.line_height));
            buf.set_text(&mut self.font_system, &ch.to_string(), &attrs, Shaping::Advanced, center);
            buf.shape_until_scroll(&mut self.font_system, false);
            rule_buffers.push(buf);
        }

        // DEPTH-DERIVED BULLETS: an unordered list line the caret is NOT on draws its
        // `•`/`◦`/`▪` glyph (by nesting depth) LEFT-aligned exactly over the concealed
        // raw `-` cell. Shaped at BODY size (unlike the bigger centered break ornament)
        // from the same bundled `SYMBOL_FAMILY` face + muted ink, so bullets read as a
        // quiet marker in line with the text. Each mark carries its own `left` (the
        // marker cell's x) since bullets are placed per-column, not centered.
        let bullet_marks = if self.md_enabled {
            self.bullet_marks()
        } else {
            Vec::new()
        };
        let bullet_metrics = GlyphMetrics::new(m.font_size, m.line_height);
        let bullet_w = (m.char_width * 2.0).max(1.0);
        let mut bullet_distinct: Vec<char> = Vec::new();
        for (_, _, ch) in &bullet_marks {
            if !bullet_distinct.contains(ch) {
                bullet_distinct.push(*ch);
            }
        }
        let mut bullet_buffers: Vec<GlyphBuffer> = Vec::with_capacity(bullet_distinct.len());
        for &ch in &bullet_distinct {
            let mut buf = GlyphBuffer::new(&mut self.font_system, bullet_metrics);
            buf.set_size(&mut self.font_system, Some(bullet_w), Some(m.line_height));
            buf.set_text(&mut self.font_system, &ch.to_string(), &attrs, Shaping::Advanced, None);
            buf.shape_until_scroll(&mut self.font_system, false);
            bullet_buffers.push(buf);
        }

        let bounds = TextBounds { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        let mut areas: Vec<TextArea> =
            Vec::with_capacity(rule_marks.len() + bullet_marks.len());
        for (top, ch) in &rule_marks {
            let idx = distinct.iter().position(|c| c == ch).expect("char was deduped in");
            areas.push(TextArea {
                buffer: &rule_buffers[idx],
                left,
                top: *top,
                scale: 1.0,
                bounds,
                default_color: muted,
                custom_glyphs: &[],
            });
        }
        for (top, bleft, ch) in &bullet_marks {
            let idx = bullet_distinct.iter().position(|c| c == ch).expect("char was deduped in");
            areas.push(TextArea {
                buffer: &bullet_buffers[idx],
                left: *bleft,
                top: *top,
                scale: 1.0,
                bounds,
                default_color: muted,
                custom_glyphs: &[],
            });
        }
        self.ornament_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                areas,
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon ornament prepare failed: {e:?}"))?;
        Ok(())
    }

    /// Build + upload the summoned chrome: the nav overlay OR search panel, the
    /// bottom-left page-mode gutter, the DEBUG frame counter, and the held stats HUD.
    pub(super) fn prepare_chrome_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        // CARET-STYLE PICKER: the floating preview PANEL below the picker card (the
        // sample line with the choreographed demo caret). Parked (nothing drawn) unless
        // that picker is open, so every other frame stays byte-identical. Built on the
        // reusable `prepare_float_panel` primitive. Prepared BEFORE the overlay so the
        // SPELL contextual panel (which reuses the SAME float quads for its own
        // elevation, see `prepare_overlay`) sets them LAST and isn't parked here — the
        // caret picker and the spell panel are mutually exclusive, so only one ever
        // owns the float quads on a frame.
        self.prepare_caret_preview_panel(device, queue, width, height)?;

        // The summoned navigation overlay takes priority over the search panel
        // (they are mutually exclusive in practice). When neither is up we upload
        // zero card / row instances so nothing lingers. A full-takeover overlay's
        // backdrop is now the cached FROSTED BLUR (prepared in `prepare_blur` / drawn
        // in `render`), not a grey scrim — except the crisp THEME/CARET pickers (they
        // keep the doc crisp) and the contextual SPELL panel (a small float popup at
        // the word). The search SPLIT panel / no overlay leave the doc bright.
        if self.overlay_active {
            self.prepare_overlay(device, queue, width, height)?;
        } else if self.search_active {
            self.prepare_panel(device, queue, width, height)?;
            self.overlay_rows.prepare(device, queue, width, height, &[]);
        } else {
            self.panel_card.prepare(device, queue, width, height, &[]);
            self.overlay_rows.prepare(device, queue, width, height, &[]);
        }
        // The page-mode orientation gutter (bottom-left margin; parks off-screen
        // edge-to-edge or with no buffer name, so a non-page capture stays byte-identical).
        self.prepare_gutter(device, queue, width, height)?;
        // The opt-in DEBUG panel (top-left; parks off-screen when off, so a default
        // capture stays byte-identical). NOTE: the persistent bottom word-count
        // readout is no longer drawn here — it moves into the held HUD (phase 2); the
        // `word_count` / `reading_time` helpers + the sidecar `readout` block remain.
        self.prepare_debug(device, queue, width, height)?;
        // The SUMMONED-WHILE-HELD stats HUD: a dim scrim + centered stacked stats,
        // drawn only while held (`crate::hud::hud_held`); released, the scrim is empty
        // and the text is parked off-screen, so a default capture stays byte-identical.
        self.prepare_hud(device, queue, width, height)?;
        Ok(())
    }

    /// Build + upload the wavy spell-check underlines (one per misspelled span),
    /// laid out on the same advance-aware glyph-x grid as the selection rects.
    pub(super) fn prepare_spell_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        // Build the wavy spell-check underlines (one per misspelled span) using
        // the SAME advance-aware glyph-x layout as the selection rects, so each
        // squiggle lands under its word's real glyph cells at any zoom/scroll.
        let squiggles = self.spell_squiggles();
        self.spell_pipeline
            .prepare(device, queue, width, height, &squiggles);
    }
}
