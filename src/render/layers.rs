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
/// is shaped — the single-owner [`crate::markdown::type_scale::ORNAMENT`] rung, the
/// SAME factor [`crate::render::spans::md_line_scale`] grows the break line's ROW by,
/// so the glyph (shaped at this scale in a line-box of the grown row height) centers
/// in its tall row. Still MUTED, never amber (DESIGN §3). Dial it in `type_scale`.
const ORNAMENT_SCALE: f32 = crate::markdown::type_scale::ORNAMENT;

/// INLINE IMAGES — the gentle rounded-corner radius (logical px, zoom-scaled) of an
/// inline image quad + its missing-file placeholder card. A calm card edge, not a
/// hard rectangle. TUNABLE.
#[cfg(not(target_arch = "wasm32"))]
const IMAGE_CORNER_PX: f32 = 4.0;

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
        //   * SETTLED on a real INHABITED glyph → paint the accent SILHOUETTE
        //     (glyph pipeline, OVER the text) with its glyph-to-glyph cross-fade
        //     as it lands.
        //   * NOTHING to inhabit → a SLIM accent bar via the BLOCK pipeline (a
        //     thin I-beam, not a full block). Two flavours below: a LINE START
        //     (col 0 — no produced glyph before the insertion point) degrades to
        //     the I-beam's insertion bar at the insertion x; a GLYPHLESS anchor
        //     past col 0 (the space just typed / emoji) keeps the cell-centered
        //     space bar.
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
            // Settled (or short-hopped) with NO inhabited glyph. LINE START (col 0
            // — incl. a fresh line after Enter and an empty line): the morph
            // DEGRADES to the I-beam look's thin insertion bar at the insertion x
            // (there is no produced glyph to light, and lighting the char AHEAD
            // would misplace the caret). Otherwise the glyphless-anchor SPACE BAR,
            // a thin version of the fat caret CENTERED in the cell. Both resolve
            // directly here without a full-block intermediate (see
            // `paint_space_bar` above); a genuine fast glide keeps `settle < SHOW`
            // and falls to the streak in the final else — so C-a's melt-to-bar
            // streaks across the travel, then forms the bar.
            let (cx, cy, cw, ch, ccorner) = if crate::caret::morph_line_start(self.cursor_col) {
                self.caret_linestart_bar_geometry()
            } else {
                self.caret_space_bar_geometry()
            };
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

    /// Build + upload the SYNTAX WASH quads: the warm low-alpha band behind every
    /// PROSE-comment span (all worlds — the identity carrier now that prose
    /// comments ride FULL ink), the green band behind string spans (dark worlds
    /// only), and the DEDICATED violet band behind every markdown `==highlight==`
    /// span (all worlds — decoupled from the comment wash so it POPS, see
    /// [`super::spans::highlight_wash`]). Geometry comes from the proto-cached
    /// [`TextPipeline::wash_rects`] (O(visible) per frame); the comment/string
    /// buckets are GATED here on the ACTIVE world's effective [`role_style_for`]
    /// wash — a role with no wash (light-world strings, or a world that opted out
    /// via `Theme::role_overrides`) uploads ZERO instances, so nothing draws (the
    /// highlight bucket has no opt-out, but an empty rect list draws nothing just
    /// the same). Empty for prose / non-highlight / non-fence buffers, keeping
    /// those frames byte-identical.
    pub(super) fn prepare_wash_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        let (mut comment_rects, mut string_rects, highlight_rects) = self.wash_rects();
        let th = theme::active();
        if role_style_for(&th, crate::syntax::SynKind::Comment).wash.is_none() {
            comment_rects.clear();
        }
        if role_style_for(&th, crate::syntax::SynKind::Str).wash.is_none() {
            string_rects.clear();
        }
        self.wash_comment_pipeline
            .prepare(device, queue, width, height, &comment_rects);
        self.wash_string_pipeline
            .prepare(device, queue, width, height, &string_rects);
        // The markdown `==highlight==` band rides its OWN violet tint (every
        // world carries it — no opt-out hatch, unlike the syntax washes), so no
        // gating: an empty `highlight_rects` (prose / non-highlight buffer)
        // uploads zero instances and draws nothing, byte-identical.
        self.wash_highlight_pipeline
            .prepare(device, queue, width, height, &highlight_rects);
    }

    /// Build + upload the WYSIWYG value-step quads: the fenced-code PANEL (whole
    /// text column, every visual row of the block) and the inline-code PILL
    /// (a small overhang past each `Code { inline: true }` span). Both ride the
    /// SAME fixed `base_200` tint (re-tinted in `sync_theme_colors`, unlike the
    /// per-role syntax washes) and both empty — zero instances uploaded — with
    /// [`crate::markdown::wysiwyg_on`] off or for a fence/inline-code-less buffer,
    /// keeping those frames byte-identical.
    pub(super) fn prepare_wysiwyg_wash_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        let panel_rects = self.fence_panel_rects();
        let pill_rects = self.code_pill_rects();
        self.fence_panel_pipeline
            .prepare(device, queue, width, height, &panel_rects);
        self.code_pill_pipeline
            .prepare(device, queue, width, height, &pill_rects);
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
        //
        // COPY PULSE: `prepare_pulsed` blends the stored base tint toward a
        // brighter peak by `(1.0 - copy_pulse_settle())` — settled (`1.0`, the
        // permanent value in every headless capture) is a byte-identical
        // short-circuit to the plain `prepare` this replaced, so a default
        // capture and every pre-existing selection render are unaffected.
        let mut rects = self.selection_rects();
        rects.extend(self.preedit_rects());
        let settle = self.copy_pulse_settle();
        self.selection_pipeline.prepare_pulsed(
            device,
            queue,
            width,
            height,
            &rects,
            copy_pulse_peak_srgba(),
            settle,
        );

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

        // The section-break FLEURON shapes in the ACTIVE WORLD'S OWN ornament face
        // (EB Garamond / Junicode / the merged marks face), NOT the shared symbol
        // face — so each world's `---`/`***`/`___` reads in its assigned flavour (see
        // `theme::Theme::ornament_face`). Bullets (below) stay on `SYMBOL_FAMILY`:
        // only the section-break/About ornament changes face. The ornament faces are
        // Regular/400, so a plain NORMAL weight matches (no `mono_safe_weight` trap).
        let rule_attrs = Attrs::new()
            .family(Family::Name(theme::active().ornament_face))
            .color(muted);
        // The depth-derived list BULLETS keep the merged marks face — ▪ lives only
        // there, and a bullet is a plain marker, not a section-break ornament.
        let bullet_attrs = Attrs::new()
            .family(Family::Name(SYMBOL_FAMILY))
            .color(muted);
        let center = Some(glyphon::cosmic_text::Align::Center);

        // The centered section-break glyph is shaped BIGGER than body ink — a calm,
        // present flourish (still muted, never amber). The break line's ROW was grown
        // by the SAME `ORNAMENT_SCALE` (via `md_line_scale`), so shaping the glyph in
        // a line-box of that grown height (`line_height * ORNAMENT_SCALE`) centers it
        // vertically on the tall break row, exactly as a heading glyph centers on its
        // grown row.
        let orn_line_h = m.line_height * ORNAMENT_SCALE;
        let orn_metrics = GlyphMetrics::new(m.font_size * ORNAMENT_SCALE, orn_line_h);

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
            buf.set_size(&mut self.font_system, Some(col_w), Some(orn_line_h));
            buf.set_text(&mut self.font_system, &ch.to_string(), &rule_attrs, Shaping::Advanced, center);
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
            buf.set_text(&mut self.font_system, &ch.to_string(), &bullet_attrs, Shaping::Advanced, None);
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

    /// WYSIWYG TABLE GRID: place every off-cursor GFM table's cells by PIXEL column
    /// (a proportional face can't align with space-padding — that's the bug this
    /// fixes) via one [`TextArea`] per cell, plus ONE faint header-separator rule.
    /// The [`Self::prepare_ornaments`] pattern applied to a rectangular block: the
    /// source rows are concealed to zero-width by
    /// [`crate::markdown::ConcealKind::Table`], and the grid draws in their place
    /// at 1:1 row occupancy (so no `RowGeom` work — each source line is one grid
    /// row: header, the rule row, then body).
    ///
    /// The heading model (WYSIWYG amendment): a table the caret is INSIDE is
    /// PARKED — grid + rule upload nothing — and its raw source reveals for
    /// editing (grid and source can't share the same rows). Also parked for a
    /// non-markdown / table-less buffer and with WYSIWYG off, so a default capture
    /// stays byte-identical.
    ///
    /// Cost: O(visible tables' cells). Off-screen tables are culled whole; a
    /// visible table shapes ALL its cells (column widths are the max over every
    /// row, so a partly-scrolled table keeps STABLE columns rather than jumping) —
    /// awl tables are small, matching the ornament pass's own "small docs" ethos.
    /// Column math ([`crate::markdown::table_column_layout`] /
    /// [`crate::markdown::table_align_offset`]) is pure + unit-tested; here we only
    /// measure (shaped `run.line_w`) and place.
    pub(super) fn prepare_table_grid(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        use crate::markdown::ColAlign;
        // Always reflect THIS frame's grid in the sidecar report; refill below.
        self.table_report.borrow_mut().clear();

        let wysiwyg = crate::markdown::wysiwyg_on();
        let blocks = if wysiwyg && self.md_enabled {
            self.table_blocks()
        } else {
            Vec::new()
        };
        if blocks.is_empty() {
            // Park both layers — byte-identical to a no-table frame.
            self.table_rule_pipeline.prepare(device, queue, width, height, &[]);
            self.table_renderer
                .prepare(
                    device,
                    queue,
                    &mut self.font_system,
                    &mut self.atlas,
                    &self.viewport,
                    Vec::new(),
                    &mut self.swash_cache,
                )
                .map_err(|e| anyhow::anyhow!("glyphon table prepare failed: {e:?}"))?;
            return Ok(());
        }

        let m = self.metrics;
        let text_left = self.text_left();
        let avail = self.text_wrap_width().max(1.0);
        let pad = TABLE_CELL_PAD_X * m.zoom;
        let gap = TABLE_COL_GAP * m.zoom;
        let rule_thick = (TABLE_RULE_THICKNESS * m.zoom).max(1.0);
        let cursor_byte = self.line_doc_byte_start(self.cursor_line);
        let content = theme::base_content().to_glyphon();
        let cell_attrs = self.doc_attrs().color(content);

        // PHASE A — parse each block into owned data (no font work yet).
        struct Meta {
            range: (usize, usize),
            ncols: usize,
            aligns: Vec<ColAlign>,
            sep_doc_line: usize,
            revealed: bool,
            visible: bool,
            /// (doc line, cells) for every GRID row (header + body; NOT the separator).
            grid_rows: Vec<(usize, Vec<String>)>,
        }
        let mut metas: Vec<Meta> = Vec::new();
        for (header_line, range) in &blocks {
            // Collect the table's source lines by walking doc lines across the range.
            let mut src_lines: Vec<String> = Vec::new();
            let mut li = *header_line;
            let mut b = range.start;
            while li < self.buffer.lines.len() && b < range.end {
                let t = self.buffer.lines[li].text();
                b += t.len() + 1;
                src_lines.push(t.to_string());
                li += 1;
            }
            if src_lines.len() < 2 {
                continue; // a real table always has header + separator
            }
            let align_cells = crate::markdown::split_row_cells(&src_lines[1]);
            // Grid rows = header (src 0) + body (src 2..); the separator (src 1) is
            // the rule row (drawn as the hairline, no cells).
            let mut grid_rows: Vec<(usize, Vec<String>)> = Vec::new();
            for (i, line) in src_lines.iter().enumerate() {
                if i == 1 {
                    continue;
                }
                if i >= 2 && line.trim().is_empty() {
                    continue; // a trailing blank swept into the range is not a row
                }
                grid_rows.push((*header_line + i, crate::markdown::split_row_cells(line)));
            }
            let ncols = grid_rows
                .iter()
                .map(|(_, c)| c.len())
                .max()
                .unwrap_or(0)
                .max(align_cells.len());
            let aligns: Vec<ColAlign> = (0..ncols)
                .map(|c| {
                    align_cells
                        .get(c)
                        .map(|s| crate::markdown::parse_col_align(s))
                        .unwrap_or(ColAlign::None)
                })
                .collect();
            let last_doc_line = *header_line + src_lines.len().saturating_sub(1);
            let visible = (*header_line..=last_doc_line).any(|dl| self.line_ornament_visible(dl));
            metas.push(Meta {
                range: (range.start, range.end),
                ncols,
                aligns,
                sep_doc_line: *header_line + 1,
                revealed: range.contains(&cursor_byte),
                visible,
                grid_rows,
            });
        }

        // PHASE B — shape cells (needs &mut font_system) for VISIBLE blocks; measure
        // each column's natural width (max shaped cell + padding). Parked/off-screen
        // blocks shape nothing (their report carries no measured widths).
        struct Shaped {
            /// (grid-row index, column, shaped buffer, shaped width).
            cells: Vec<(usize, usize, GlyphBuffer, f32)>,
            naturals: Vec<f32>,
        }
        let body_metrics = GlyphMetrics::new(m.font_size, m.line_height);
        let mut shaped: Vec<Option<Shaped>> = Vec::with_capacity(metas.len());
        for meta in &metas {
            if !meta.visible || meta.ncols == 0 {
                shaped.push(None);
                continue;
            }
            let mut naturals = vec![0.0f32; meta.ncols];
            let mut cells: Vec<(usize, usize, GlyphBuffer, f32)> = Vec::new();
            for (gr, (_, row_cells)) in meta.grid_rows.iter().enumerate() {
                for (c, cell) in row_cells.iter().enumerate() {
                    if c >= meta.ncols || cell.is_empty() {
                        continue;
                    }
                    let mut buf = GlyphBuffer::new(&mut self.font_system, body_metrics);
                    buf.set_size(&mut self.font_system, Some(avail), Some(m.line_height));
                    buf.set_text(&mut self.font_system, cell, &cell_attrs, Shaping::Advanced, None);
                    buf.shape_until_scroll(&mut self.font_system, false);
                    let mut w = 0.0f32;
                    for run in buf.layout_runs() {
                        w = w.max(run.line_w);
                    }
                    naturals[c] = naturals[c].max(w + 2.0 * pad);
                    cells.push((gr, c, buf, w));
                }
            }
            // A column of only-empty cells still occupies its padding.
            for w in naturals.iter_mut() {
                if *w <= 0.0 {
                    *w = 2.0 * pad;
                }
            }
            shaped.push(Some(Shaped { cells, naturals }));
        }

        // PHASE C — lay out columns, place cells + the header rule, fill the report.
        let mut areas: Vec<TextArea> = Vec::new();
        let mut rule_rects: Vec<[f32; 4]> = Vec::new();
        for (mi, meta) in metas.iter().enumerate() {
            let (col_x, col_w) = match &shaped[mi] {
                Some(s) => crate::markdown::table_column_layout(&s.naturals, gap, avail),
                None => (Vec::new(), Vec::new()),
            };
            self.table_report.borrow_mut().push(crate::render::TableReport {
                range: meta.range,
                rows: meta.grid_rows.len(),
                cols: meta.ncols,
                col_widths: col_w.clone(),
                revealed: meta.revealed,
            });
            // Draw only a visible, NON-revealed table (the caret's own table parks —
            // its raw source reveals via the conceal seam).
            if meta.revealed {
                continue;
            }
            let Some(s) = &shaped[mi] else {
                continue;
            };
            for (gr, c, buf, cw) in &s.cells {
                let doc_line = meta.grid_rows[*gr].0;
                let top = self.line_ornament_top(doc_line);
                let box_left = text_left + col_x[*c];
                let box_w = col_w[*c];
                let off = crate::markdown::table_align_offset(meta.aligns[*c], box_w, *cw, pad);
                // Clip each cell to its OWN column box's right edge so an over-wide
                // cell truncates at its column rather than overrunning its neighbour.
                let clip_left = box_left.max(0.0) as i32;
                let clip_right = (box_left + box_w).clamp(0.0, width as f32) as i32;
                areas.push(TextArea {
                    buffer: buf,
                    left: box_left + off,
                    top,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: clip_left,
                        top: 0,
                        right: clip_right,
                        bottom: height as i32,
                    },
                    default_color: content,
                    custom_glyphs: &[],
                });
            }
            // The ONE faint header-separator hairline (the grid's only drawn line),
            // centered in the separator row's band, spanning the laid grid width.
            if let (Some(&last_x), Some(&last_w)) = (col_x.last(), col_w.last()) {
                let grid_w = last_x + last_w;
                let sep_top = self.line_ornament_top(meta.sep_doc_line);
                let y = sep_top + (m.line_height - rule_thick) * 0.5;
                rule_rects.push([text_left, y, grid_w, rule_thick]);
            }
        }

        self.table_rule_pipeline
            .prepare(device, queue, width, height, &rule_rects);
        self.table_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                areas,
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon table prepare failed: {e:?}"))?;
        Ok(())
    }

    /// The deterministic per-table geometry the last [`Self::prepare_table_grid`]
    /// laid out, for the capture `tables` sidecar block (a clone of the stashed
    /// report). Empty for a non-table / WYSIWYG-off frame.
    pub fn tables_report(&self) -> Vec<crate::render::TableReport> {
        self.table_report.borrow().clone()
    }

    /// The deterministic per-image layout the last [`Self::rebuild_image_rows`]
    /// (via `compute_image_layout`) produced, for the capture `images` sidecar
    /// block + the next-phase draw. `revealed` is recomputed here against the
    /// CURRENT caret line (a pure caret move re-lays the image line's conceal but
    /// does not re-read image headers), so it never goes stale. Empty when inline
    /// images are off / non-markdown / on wasm.
    /// INLINE IMAGES — the GPU draw. Decodes each visible, OFF-CURSOR image
    /// (O(visible): off-screen + revealed images are culled), uploads it via the
    /// [`image_cache`](crate::render::image_cache) (downscaled to the display
    /// width), and builds one textured quad per image (fit-to-column, centered in
    /// the reserved tall row `compute_image_layout` produced) plus a calm rounded
    /// PLACEHOLDER (opaque `base_200` quad + a muted filename / faint alt label) for
    /// every MISSING-file image. All three layers (image quads / placeholder quads /
    /// placeholder labels) park EMPTY when the feature is off / no visible images /
    /// non-markdown, so a default capture stays byte-identical.
    ///
    /// The tall rows themselves are reserved at reshape time (`compute_image_layout`
    /// → `image_heights`); the DECODE is synchronous here, so it never changes a
    /// reserved row height after the fact (the row was sized from the header dims,
    /// and the same file decodes to the same aspect) — no deferred-height
    /// invalidation is needed, the missing live-bug class the design flagged.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn prepare_images(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        use crate::render::image_cache::{ImageCache, ImageState};
        let report = self.images_report();

        // Prune the decode cache to the OPEN DOC's images (visible or not), keyed by
        // canonical path — buffer-swap-safe, and scrolling back to an image never
        // re-decodes (it stays cached while it's in this doc's set).
        let mut keep: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
        for im in &report {
            let resolved = self.resolve_image_path(&im.path);
            keep.insert(ImageCache::canonical_key(&resolved));
        }
        self.image_cache.retain_paths(&keep);

        let max_dim = device.limits().max_texture_dimension_2d;
        let zoom = self.metrics.zoom;
        let corner = IMAGE_CORNER_PX * zoom;
        let text_left = self.text_left();
        let wrap = self.text_wrap_width().max(1.0);

        // PASS A — cull + decode. `ready` holds the quad placements (dst rect + the
        // cache key to fetch the view in pass B); `missing` holds the placeholder
        // placements (dst rect + filename + alt). Reveal-on-cursor: an image on the
        // caret's line is PARKED (its raw source reveals), and an off-screen image is
        // culled (its row is clipped to nothing anyway).
        struct Ready {
            dst: [f32; 4],
            key: std::path::PathBuf,
        }
        struct Missing {
            dst: [f32; 4],
            path: String,
            alt: String,
        }
        let mut ready: Vec<Ready> = Vec::new();
        let mut missing: Vec<Missing> = Vec::new();
        for im in &report {
            if im.revealed || !self.line_ornament_visible(im.line) {
                continue;
            }
            let dw = im.display_w.max(1.0);
            let dh = im.display_h.max(1.0);
            let top = self.line_ornament_top(im.line);
            // Fit-to-column: centered horizontally in the writing column; the row is
            // exactly `dh` tall (reserved), so the quad fills it vertically.
            let left = text_left + (wrap - dw).max(0.0) * 0.5;
            let dst = [left, top, dw, dh];
            if im.missing {
                missing.push(Missing { dst, path: im.path.clone(), alt: im.alt.clone() });
                continue;
            }
            let resolved = self.resolve_image_path(&im.path);
            let key = ImageCache::canonical_key(&resolved);
            match self.image_cache.ensure(device, queue, &resolved, dw, max_dim) {
                ImageState::Ready { .. } => ready.push(Ready { dst, key }),
                // Header read OK at layout but the full decode failed at draw (a rare
                // race — the file changed/vanished): fall to the placeholder, drawn in
                // the aspect-reserved box.
                ImageState::Missing => {
                    missing.push(Missing { dst, path: im.path.clone(), alt: im.alt.clone() })
                }
            }
        }

        // PASS B — build the image quads from the cached views (a distinct IMMUTABLE
        // cache borrow, disjoint from the mutable `image_pipeline` field).
        {
            let cache = &self.image_cache;
            let pipeline = &mut self.image_pipeline;
            let placed: Vec<crate::image_pipeline::PlacedImage> = ready
                .iter()
                .filter_map(|r| {
                    cache.view(&r.key).map(|view| crate::image_pipeline::PlacedImage {
                        dst: r.dst,
                        alpha: 1.0,
                        corner,
                        view,
                    })
                })
                .collect();
            pipeline.prepare(device, queue, width, height, &placed);
        }

        // The calm MISSING-file placeholder quads (base_200 rounded cards).
        let placeholder_rects: Vec<[f32; 4]> = missing.iter().map(|m| m.dst).collect();
        self.image_placeholder_pipeline
            .prepare(device, queue, width, height, &placeholder_rects);

        // The placeholder LABELS: a muted filename over a faint alt, centered in each
        // card (the ornament pattern — one shaped buffer per line, borrowed by its
        // TextArea). Empty `missing` parks the renderer off-screen (no areas).
        let m = self.metrics;
        let label = crate::markdown::type_scale::LABEL;
        let gm = GlyphMetrics::new(m.font_size * label, m.line_height * label);
        let line_h = m.line_height * label;
        let muted = theme::muted().to_glyphon();
        let faint = theme::faint().to_glyphon();
        let center = Some(glyphon::cosmic_text::Align::Center);
        let name_attrs = self.doc_attrs().color(muted);
        let alt_attrs = self.doc_attrs().color(faint);
        // (buffer, left, top, color) tuples; the buffers outlive the areas below.
        let mut buffers: Vec<(GlyphBuffer, f32, f32, glyphon::Color)> = Vec::new();
        for mss in &missing {
            let filename = std::path::Path::new(&mss.path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(mss.path.as_str());
            let box_w = mss.dst[2].max(1.0);
            let box_left = mss.dst[0];
            let box_top = mss.dst[1];
            let box_h = mss.dst[3];
            let two = !mss.alt.trim().is_empty();
            let block_h = if two { line_h * 2.0 } else { line_h };
            let start_y = box_top + (box_h - block_h).max(0.0) * 0.5;
            let mut name_buf = GlyphBuffer::new(&mut self.font_system, gm);
            name_buf.set_size(&mut self.font_system, Some(box_w), Some(line_h));
            name_buf.set_text(&mut self.font_system, filename, &name_attrs, Shaping::Advanced, center);
            name_buf.shape_until_scroll(&mut self.font_system, false);
            buffers.push((name_buf, box_left, start_y, muted));
            if two {
                let mut alt_buf = GlyphBuffer::new(&mut self.font_system, gm);
                alt_buf.set_size(&mut self.font_system, Some(box_w), Some(line_h));
                alt_buf.set_text(&mut self.font_system, mss.alt.trim(), &alt_attrs, Shaping::Advanced, center);
                alt_buf.shape_until_scroll(&mut self.font_system, false);
                buffers.push((alt_buf, box_left, start_y + line_h, faint));
            }
        }
        let bounds = TextBounds { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        let areas: Vec<TextArea> = buffers
            .iter()
            .map(|(buf, left, top, color)| TextArea {
                buffer: buf,
                left: *left,
                top: *top,
                scale: 1.0,
                bounds,
                default_color: *color,
                custom_glyphs: &[],
            })
            .collect();
        self.image_placeholder_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                areas,
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon image placeholder prepare failed: {e:?}"))?;
        Ok(())
    }

    /// INLINE IMAGES on wasm: the feature is native-only (no decode cache), so all
    /// three layers park EMPTY — byte-identical to the feature being off.
    #[cfg(target_arch = "wasm32")]
    pub(super) fn prepare_images(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        self.image_pipeline.clear();
        self.image_placeholder_pipeline
            .prepare(device, queue, width, height, &[]);
        self.image_placeholder_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                Vec::new(),
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon image placeholder prepare failed: {e:?}"))?;
        Ok(())
    }

    pub fn images_report(&self) -> Vec<crate::render::ImageReport> {
        self.image_report
            .borrow()
            .iter()
            .cloned()
            .map(|mut r| {
                r.revealed = r.line == self.cursor_line;
                r
            })
            .collect()
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
        // The CALM NOTICE (bottom-center; live-only content — the autosave clobber
        // guard). Empty parks off-screen, so every capture stays byte-identical.
        self.prepare_notice(device, queue, width, height)?;
        // The PAGE-WIDTH DRAG READOUT (floats at the pointer; live-only, mouse-driven
        // content). `None` parks it off-screen, so every capture stays byte-identical.
        self.prepare_page_drag_readout(device, queue, width, height)?;
        // The SUMMONED-WHILE-HELD stats HUD: a dim scrim + centered stacked stats,
        // drawn only while held (`crate::hud::hud_held`); released, the scrim is empty
        // and the text is parked off-screen, so a default capture stays byte-identical.
        self.prepare_hud(device, queue, width, height)?;
        // The SUMMONED WHICH-KEY panel: a bottom-left hint card listing a pending
        // prefix's follow-up keys. Drawn only while summoned (the App set its rows on a
        // prefix pause); parked off-screen otherwise, so a default capture is byte-identical.
        self.prepare_whichkey(device, queue, width, height)?;
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

    /// Build + upload the STRAIGHT muted WRITING-NIT underlines (one per nit span),
    /// on the SAME advance-aware glyph-x grid as the spell squiggles + selection
    /// rects. Empty (nothing uploaded, so nothing drawn) when the highlighter is
    /// toggled off, so a nits-off frame is byte-identical to no nits at all.
    pub(super) fn prepare_nit_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        let underlines = self.nit_underlines();
        self.nit_pipeline
            .prepare(device, queue, width, height, &underlines);
    }
}
