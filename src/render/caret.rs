//! CARET RENDER GEOMETRY — the layout-entangled half of the animated caret.
//!
//! The caret's *spring physics*, *global mode*, and *GPU pipelines* already live
//! in their own crate modules ([`crate::caret`] — `CaretAnim` spring + `CaretMode`
//! global + the block/trail `CaretPipeline`; [`crate::caret_glyph`] — the morph
//! `CaretGlyphPipeline` + `GlyphMask`). What remains here is everything that turns
//! that machinery into *pixels for the current frame*: the per-frame caret/streak
//! geometry, the morph glyph-mask rasterisation + placement, the IME cell rect,
//! the spring TARGET wiring, the deterministic settle/inject poses, and the
//! capture-sidecar reports.
//!
//! These are all inherent methods on [`super::TextPipeline`] — they read its font
//! / layout / metrics / buffer state heavily (the cursor's visual row, the real
//! glyph advance, the shaped baseline), so they CANNOT become `&self`-free free
//! functions the way the span/attrs helpers over in `render.rs` already are
//! (`build_line_attrs`, `add_md_line_spans`, `scaled_base_attrs`, … take explicit
//! params, so they could move to a sibling module unchanged). The row-geometry
//! cache (`ensure_row_geom` and friends), by contrast, is still inherent methods
//! on `TextPipeline` just like the caret geometry here. So this module is purely a
//! physical home for that cohesive cluster, carved out of `render.rs` verbatim.
//! Because a child module sees its ancestor's private items, the methods keep
//! their full access to `TextPipeline`'s private fields and helpers with NO
//! behaviour change — the capture output is byte-identical.
//!
//! The two NON-caret neighbours from the original region — the virtual-clock seam
//! `advance()` and the focus-fade `step_focus()` — deliberately stay in the parent
//! `render` module beside the rest of the focus machinery; only `step_caret()`
//! (which `advance()` OR-folds in) lives here.

use super::*;

impl TextPipeline {
    /// Pixel y of the TOP of the glyph cell box at the cursor (the box that the
    /// selection / preedit / IME rect share), wrap-aware. The caret underline sits
    /// at the BOTTOM of this box.
    fn caret_cell_top(&self) -> f32 {
        let m = &self.metrics;
        let line_top = self.visual_row_top(self.cursor_line, self.cursor_col);
        // Centre the caret box in the cursor's ACTUAL row height, so on a (taller)
        // heading row the caret sits on the heading's optical centre rather than
        // floating high in a base-height cell. The caret anchor is built from this
        // (`caret_cell_top + caret_h/2`), so the block/morph caret recentres too.
        let row_h = self.cursor_row_height();
        line_top + (row_h - m.caret_h) * 0.5
    }

    /// The caret spring ANCHOR target: the pixel position the spring chases. This
    /// is the LEFT edge x of the glyph cell and the CENTER y of the glyph cell box
    /// (so the resting rounded square sits centered ON the character). Using the
    /// real glyph advance + wrap-aware visual row keeps the anchor correct for
    /// full-width CJK and wrapped lines. The drawn caret rect is built around this
    /// anchor by [`Self::caret_geometry`], which applies the motion drop + shape
    /// stretch on top of it.
    pub fn caret_target_xy(&self) -> (f32, f32) {
        let m = &self.metrics;
        let (gx, _adv) = self.col_x_and_advance(self.cursor_line, self.cursor_col);
        let x = self.text_left() + gx;
        // Cell-box vertical center: the resting square is centered on the glyph.
        let y = self.caret_cell_top() + m.caret_h * 0.5;
        (x, y)
    }

    /// Width of the resting caret SQUARE at the current cursor: the real advance of
    /// the glyph under the cursor (so a full-width CJK glyph gets a full-width
    /// block), clamped to at least the default Latin cell so an end-of-line /
    /// empty caret stays visible. Used by the Morph space-bar and the IME rect,
    /// which want the floored cell; the BLOCK quad uses [`Self::caret_block_w`].
    pub fn caret_target_w(&self) -> f32 {
        let (_x, adv) = self.col_x_and_advance(self.cursor_line, self.cursor_col);
        adv.max(self.metrics.caret_w)
    }

    /// Width of the resting BLOCK caret quad at the current cursor: the REAL shaped
    /// glyph ADVANCE under the cursor, so on a PROPORTIONAL world the block exactly
    /// covers the glyph it sits on — wide on an `m`/`w`, narrow on an `i`/`l` —
    /// instead of the fixed mono cell that read too wide on thin glyphs. The advance
    /// comes from the same `col_x_and_advance` the caret X / Morph silhouette / I-beam
    /// already ride, so the block tracks the exact cell the cursor is on. At a
    /// GLYPHLESS cell (end-of-line / space / empty line) `col_x_and_advance` already
    /// falls back to a sensible default — the space's own advance, or `char_width`
    /// past the last glyph — so the block keeps a visible width there.
    ///
    /// On a MONO world every advance equals the cell, so we keep the historical
    /// `.max(caret_w)` floor: the block stays byte-identical to the old fixed cell
    /// (`caret_block_w == caret_target_w`). The floor — the very thing that made the
    /// block too wide on a narrow proportional glyph — is dropped ONLY on
    /// proportional faces.
    pub fn caret_block_w(&self) -> f32 {
        let (_x, adv) = self.col_x_and_advance(self.cursor_line, self.cursor_col);
        if crate::caret::font_is_mono(crate::theme::active().font) {
            adv.max(self.metrics.caret_w)
        } else {
            adv
        }
    }

    /// Resolve the cosmic-text [`CacheKey`] of the glyph under the cursor at
    /// (`line`, `col`), or `None` when there is no rasterizable glyph there
    /// (end-of-line, an empty/glyphless line, or a whitespace glyph whose mask is
    /// empty). The MORPH caret uses this key both to capture the "from" glyph at a
    /// move and to rasterize the "to" glyph for the current cursor.
    ///
    /// Walks the cursor line's shaped runs (same pattern as `line_glyph_xs`) and
    /// picks the glyph cluster whose BYTE range covers the cursor column's byte;
    /// `glyph.physical((0,0),1.0)` then yields the `CacheKey` (font + glyph id +
    /// size + subpixel), which is exactly what the swash cache consumes.
    pub(super) fn cursor_glyph_key_at(&self, line: usize, col: usize) -> Option<CacheKey> {
        let line_text = self.buffer.lines.get(line)?.text().to_string();
        // Byte offset of the cursor column on this logical line.
        let cur_byte = line_text
            .char_indices()
            .nth(col)
            .map(|(b, _)| b)
            .unwrap_or(line_text.len());
        if cur_byte >= line_text.len() {
            // End of line: no glyph cell to silhouette.
            return None;
        }
        for run in self.buffer.layout_runs() {
            if run.line_i != line {
                continue;
            }
            for g in run.glyphs.iter() {
                if cur_byte >= g.start && cur_byte < g.end {
                    return Some(g.physical((0.0, 0.0), 1.0).cache_key);
                }
            }
        }
        None
    }

    /// Pixels the cursor glyph's real rasterized ink DIPS BELOW the baseline — the
    /// font-correct descender depth measured from the glyph's swash placement box
    /// (NOT a hardcoded letter list), so it is right across all 11 worlds' faces.
    /// `placement.top` is the px from the baseline UP to the raster top; the raster
    /// bottom is `top - height`, so the depth below the baseline is
    /// `(height - top).max(0)`: 0 for non-dipping glyphs (`a`/`m`/`C`), positive for
    /// descenders (`g`/`y`/`p`/`q`/`j`). Used by the BLOCK caret to drop ONLY its
    /// bottom edge so the reverse-video glyph's descender stays inside the block.
    /// Returns 0 on a glyphless cell (end-of-line / space / empty line).
    pub(super) fn cursor_glyph_descender(&mut self) -> f32 {
        let Some(key) = self.cursor_glyph_key_at(self.cursor_line, self.cursor_col) else {
            return 0.0;
        };
        let Self {
            swash_cache,
            font_system,
            ..
        } = self;
        match swash_cache.get_image(font_system, key) {
            Some(img) => (img.placement.height as i32 - img.placement.top).max(0) as f32,
            None => 0.0,
        }
    }

    /// Ensure `slot`'s cached mask matches `key`, rasterizing only when the key
    /// changed (the key folds glyph id + font + size + subpixel, so zoom / font /
    /// world switches re-rasterize automatically). A `None` key clears the slot.
    fn ensure_mask(
        slot: &mut Option<GlyphMask>,
        swash_cache: &mut SwashCache,
        font_system: &mut FontSystem,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: Option<CacheKey>,
    ) {
        match key {
            None => *slot = None,
            Some(k) => {
                if slot.as_ref().map(|m| m.key) == Some(k) {
                    return; // already cached
                }
                let mask = swash_cache
                    .get_image_uncached(font_system, k)
                    .and_then(|image| {
                        if image.content != SwashContent::Mask {
                            return None;
                        }
                        let w = image.placement.width;
                        let h = image.placement.height;
                        if w == 0 || h == 0 || image.data.is_empty() {
                            return None;
                        }
                        Some(GlyphMask::from_coverage(
                            device,
                            queue,
                            k,
                            image.placement.left,
                            image.placement.top,
                            w,
                            h,
                            &image.data,
                        ))
                    });
                *slot = mask;
            }
        }
    }

    /// The baseline y (absolute, scroll-applied pixels) of the cursor's visual row:
    /// the EXACT pen baseline glyphon draws the real glyph at, so the MORPH
    /// silhouette overlaps it pixel-for-pixel. Each glyph mask's placement box is
    /// positioned relative to this baseline (box top = baseline - placement.top),
    /// mirroring how the swash placement box hangs off the pen origin — which is
    /// the same convention glyphon uses to blit the real glyph. Because the morph
    /// caret now draws OVER the text, exact alignment matters: a few-px error would
    /// show as a doubled/shifted letter rather than a clean recolour.
    ///
    /// The truth source is cosmic-text's `run.line_y` (the baseline offset relative
    /// to the buffer top) for the cursor's wrapped run; absolute baseline =
    /// `doc_top() + run.line_y`. A glyphless / empty line has no run, so it falls
    /// back to the metrics-derived ascent approximation (only ever used by the
    /// space/EOL case, which doesn't paint a glyph silhouette anyway).
    pub(super) fn caret_baseline_y(&self) -> f32 {
        // Find the shaped run that owns the cursor's column and read its real
        // baseline. Match the run by char column span (same logic as `pick_row`):
        // the run whose [start_col, end_col) contains the cursor column.
        let line_text = self
            .buffer
            .lines
            .get(self.cursor_line)
            .map(|l| l.text().to_string())
            .unwrap_or_default();
        for run in self.buffer.layout_runs() {
            if run.line_i != self.cursor_line {
                continue;
            }
            let (mut bs, mut be) = (usize::MAX, 0usize);
            for g in run.glyphs.iter() {
                bs = bs.min(g.start);
                be = be.max(g.end);
            }
            if bs == usize::MAX {
                continue;
            }
            let start_col = byte_col(&line_text, bs);
            let end_col = byte_col(&line_text, be);
            if self.cursor_col >= start_col && self.cursor_col < end_col {
                return self.doc_top() + run.line_y;
            }
        }
        // Fallback (no run owns the column — glyphless/empty line): approximate the
        // baseline from the row top + an ascent proportion. The morph caret never
        // paints a silhouette here (it falls back to the slim space bar), so this
        // only keeps the value finite.
        let m = &self.metrics;
        let line_top = self.visual_row_top(self.cursor_line, self.cursor_col);
        line_top + (m.line_height - m.font_size) * 0.5 + m.font_size * 0.8
    }

    /// Geometry for the MORPH caret this frame: the two glyph placement boxes
    /// (`from`/`to`) positioned at the ANIMATED caret anchor (so they slide along
    /// the spring), plus the cross-fade `morph_t`. Returns the boxes as
    /// `[min_x, min_y, w, h]` in absolute pixels. The masks themselves are cached
    /// in `caret_mask_from`/`caret_mask_to`. There is no soft halo; the silhouette
    /// is the glyph's own crisp coverage, HARD-dilated ~`CARET_MORPH_DILATE_PX` in
    /// the shader so the caret reads a touch fatter than the letter but stays solid.
    ///
    /// `morph_t` is driven by the spring's settle factor: 0 mid-glide (show the
    /// FROM glyph), rising to 1 as the caret decelerates onto the destination (show
    /// the TO glyph). At rest there is no `from`, so it pins to 1.
    pub(super) fn caret_glyph_geometry(&self) -> ([f32; 4], [f32; 4], f32) {
        // Animated caret left-edge x (the spring chases the cell's left edge x).
        let pen_x = self.caret.pos.x;
        let baseline_y = self.caret_baseline_y();

        // Position a placement box at the animated pen origin: box top-left =
        // (pen_x + placement.left, baseline_y - placement.top). This mirrors how
        // glyphon hangs the real glyph off the pen, so the silhouette overlaps it.
        let box_of = |mask: &Option<GlyphMask>| -> [f32; 4] {
            match mask {
                Some(mk) => [
                    pen_x + mk.left as f32,
                    baseline_y - mk.top as f32,
                    mk.width as f32,
                    mk.height as f32,
                ],
                None => [0.0, 0.0, 0.0, 0.0],
            }
        };
        let from_box = box_of(&self.caret_mask_from);
        let to_box = box_of(&self.caret_mask_to);

        // Cross-fade: the settle factor rises 0->1 as the caret arrives, so the new
        // glyph fades in as the old one fades out. With no FROM glyph there is
        // nothing to fade from, so show the TO glyph fully.
        let morph_t = if self.caret_mask_from.is_some() {
            self.caret.settle_factor()
        } else {
            1.0
        };
        (from_box, to_box, morph_t)
    }

    /// Refresh the cached MORPH masks for this frame: rasterize the current cursor
    /// glyph (the "to" mask) and the glyph the caret is leaving (the "from" mask),
    /// re-rasterizing each only when its `CacheKey` changed. Returns `true` when
    /// there IS a rasterizable cursor glyph (so morph mode can draw); `false` when
    /// the cursor sits on a glyphless cell (end-of-line / whitespace / empty line /
    /// emoji), signalling the caller to fall back to the block caret this frame.
    pub(super) fn prepare_caret_masks(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        let to_key = self.cursor_glyph_key_at(self.cursor_line, self.cursor_col);
        // The "from" glyph fades out only while a glide is settling; once at rest
        // (or with no captured from-key) drop it so the resting caret is a clean
        // single silhouette.
        let from_key = if self.caret.is_animating() {
            self.caret_from_key
        } else {
            None
        };
        // Split the borrows: ensure_mask needs the swash cache + font system by
        // &mut alongside each slot, all distinct fields of self. Scoped so the
        // partial borrows release before the final whole-field read below.
        {
            let Self {
                caret_mask_to,
                caret_mask_from,
                swash_cache,
                font_system,
                ..
            } = self;
            Self::ensure_mask(caret_mask_to, swash_cache, font_system, device, queue, to_key);
            Self::ensure_mask(
                caret_mask_from,
                swash_cache,
                font_system,
                device,
                queue,
                from_key,
            );
        }
        self.caret_mask_to.is_some()
    }

    /// The drawn caret rectangle `(center_x, center_y, w, h, corner)` for THIS
    /// frame. The caret morphs between TWO states by the spring's settle factor
    /// `s` (1 = at rest, 0 = fully in motion); `motion = 1 - s` drives the move.
    ///
    /// - AT REST (s≈1): a "roundish square" centered on the glyph cell — width =
    ///   full glyph advance, height = `caret_block_h`, large corner radius; center
    ///   y = the spring anchor (cell-box center).
    /// - IN MOTION (s→0): the square stretches into a thin streak along the TRUE
    ///   travel vector (horizontal / vertical / diagonal alike, no per-axis branch),
    ///   anchored at the TEXT optical centre — the line-box centre `pos.y` dropped by
    ///   `caret_trail_drop` to the x-height middle (so the trail runs THROUGH the
    ///   letters, not slightly above them). There is no baseline drop: a horizontal
    ///   move runs a centred sweep through the text centre rather than dropping to an
    ///   underline. The streak TRAILS the leading edge (the
    ///   leading edge tracks the animated position; the body extends BACK toward
    ///   where the caret came from), its length growing with speed.
    ///
    /// The shape stretch and the corner-radius morph are keyed off the same `s`, so
    /// the caret re-forms as it decelerates onto the destination glyph. The
    /// centre-to-centre trail (via `motion_geometry`) is shared by Block, Morph's
    /// fast-motion deferral, and the I-beam.
    pub(super) fn caret_geometry(&self) -> (f32, f32, f32, f32, f32, f32, f32) {
        let m = &self.metrics;
        let s = self.caret.settle_factor();

        // --- Shape endpoints --------------------------------------------------
        let block_w = self.caret_block_w(); // real glyph advance (narrow i, wide m)
        // Scale the resting block height to the cursor's row so it COVERS a big
        // heading glyph (1.0 on body text -> byte-identical there).
        let block_h = m.caret_block_h * self.cursor_scale();
        let streak_thin = m.caret_streak_h; // the streak's thin cross-dimension
        // Corner radius: small bar radius in motion, large soft radius at rest.
        let corner =
            STREAK_RADIUS * m.zoom + (CORNER_RADIUS * m.zoom - STREAK_RADIUS * m.zoom) * s;

        // --- ONE rule for every direction (no if-vertical / if-horizontal) -----
        // The trail is a DIRECT line along the TRUE travel vector (diagonal too),
        // not mirrored onto an axis. Length scales with the (euclidean) speed,
        // floored by this frame's advance so a fast glide bridges with no gaps; the
        // unified `motion_geometry` orients it and trails it behind the leading edge.
        let speed = (self.caret.vel.x * self.caret.vel.x + self.caret.vel.y * self.caret.vel.y)
            .sqrt();
        // While HOLDING (continuous/held motion) the length is a STEADY constant
        // (`caret_held_len`) so the trail is a smooth, near-constant streak instead
        // of breathing once per auto-repeat. Non-held is the old speed-derived
        // length floored by the per-frame bridge.
        let streak_len = self.caret.streak_length(
            m.streak_len_for_speed(speed),
            m.caret_streak_max_len,
            m.caret_held_len,
        );
        let (center, half_along, half_across, axis) = self.caret.motion_geometry(
            block_w,
            block_h,
            streak_thin,
            streak_len,
            m.caret_streak_gap,
            m.caret_trail_drop,
        );
        (
            center.x,
            center.y,
            half_along * 2.0,
            half_across * 2.0,
            corner,
            axis.0,
            axis.1,
        )
    }

    /// Scale a caret rect's `(w, h, corner)` by the cosmetic SQUASH-POP factor for
    /// THIS frame. Applied at the draw site (after the geometry is computed) about the
    /// rect's UNCHANGED centre, so the caret squashes and springs back IN PLACE — the
    /// centre (hence the on-screen position) is never touched. At rest the factor is
    /// 1.0, so this is an identity (and the deterministic capture, which renders the
    /// settled state, is byte-unchanged). Shared by the block / space-bar / I-beam
    /// draw paths so the pop reads consistently across the looks.
    pub(super) fn pop_scaled(&self, w: f32, h: f32, corner: f32) -> (f32, f32, f32) {
        let s = self.caret.pop_scale();
        (w * s, h * s, corner * s)
    }

    /// The SLIM accent-bar geometry `(center_x, center_y, w, h, corner)` for the
    /// MORPH caret on a GLYPHLESS cell (a space / end-of-line / empty line), where
    /// there is no letterform to recolour: a THIN VERSION of the fat resting caret
    /// — same rounded style and same `caret_block_h` height — just narrowed to
    /// `CARET_SPACE_BAR_W`, and CENTERED in the cell.
    ///
    /// The key fix is the x position. The resting block (`caret_geometry`) centers
    /// on the cell using the REAL advance (`caret_target_w`): `cx = pos.x +
    /// advance*0.5`. The old space bar instead pinned its LEFT edge at `pos.x`
    /// (`cx = pos.x + w*0.5`), which dropped the thin bar against the cell's left
    /// edge — at the boundary BEFORE the space, not inside it — because it ignored
    /// the space's advance entirely. Here we center the thin bar on the same cell
    /// midpoint the block uses (`pos.x + advance*0.5`), so it sits in the middle of
    /// the space gap exactly where the block would. It rides the spring anchor
    /// (`pos`) so it slides with the caret. Drawn through the BLOCK pipeline (a
    /// solid accent rounded rect), which is exactly the slim-bar look we want.
    pub(super) fn caret_space_bar_geometry(&self) -> (f32, f32, f32, f32, f32) {
        let m = &self.metrics;
        let w = CARET_SPACE_BAR_W * m.zoom;
        // ~the glyph cell height tall (the same box the resting block covers), so
        // the bar reads as a line-tall thin caret on the empty cell. Row-scaled so a
        // glyphless heading line gets a tall bar too (1.0 on body text -> unchanged).
        let h = m.caret_block_h * self.cursor_scale();
        // CENTER the thin bar on the cell using the real advance, mirroring the
        // resting block's `pos.x + advance*0.5`. This lands it in the middle of the
        // space gap (not pinned to the left edge as before).
        let advance = self.caret_target_w();
        let cx = self.caret.pos.x + advance * 0.5;
        let cy = self.caret.pos.y;
        // Same generous resting corner radius as the fat caret (so it reads as a
        // narrow version of the same rounded caret), clamped so a thin bar can't
        // over-round into a lozenge.
        let corner = (CORNER_RADIUS * m.zoom).min(w * 0.5);
        (cx, cy, w, h, corner)
    }

    /// Geometry `(center_x, center_y, w, h, corner)` for the PROTOTYPE I-beam caret:
    /// a thin vertical bar pinned at the INSERTION POINT (the cursor glyph's left
    /// edge / pen origin `pos.x`), spanning the glyph cell box. AT REST it is a
    /// STEADY thin, tall bar (no breathing — fully static when idle). Reuses the
    /// spring's settle factor + velocity + the streak machinery for VELOCITY
    /// SQUASH/STRETCH (the elongating comet — the I-beam's speed cue, retained):
    ///   * HORIZONTAL motion: stretches into a horizontal comet/lozenge — width
    ///     grows with horizontal speed, height collapses toward the bar's thin
    ///     dimension — trailing back opposite the travel.
    ///   * VERTICAL motion: stretches into a tall lozenge — height grows with
    ///     vertical speed — trailing back along the jump.
    /// CENTRE-anchored (the comet body trails through the caret's vertical centre,
    /// like Block/Morph) and the origin-side tail is inset by the shared streak GAP
    /// so it stops short of where the move started. The underdamped spring supplies
    /// the overshoot/wobble on landing for free; the recoil kick (see `caret_kick`)
    /// rides the same spring.
    pub(super) fn caret_ibeam_geometry(&self) -> (f32, f32, f32, f32, f32) {
        let m = &self.metrics;
        let s = self.caret.settle_factor();
        let motion = 1.0 - s;

        // Rest endpoints: a steady thin, tall bar (no breathe swell). Scale the
        // height to the cursor's row so the bar spans a big heading line's glyphs,
        // not a body-height sliver (1.0 on body text -> unchanged).
        let thin = IBEAM_W * m.zoom;
        let tall = m.caret_h * self.cursor_scale(); // full glyph cell box, row-scaled
        // Shared origin GAP: the elongated comet's tail stops ~1.5 chars short of the
        // move's start, consistent with the Block/Morph trail's tail inset. While
        // HOLDING (continuous/held motion) the gap is demoted to a cosmetic trim and
        // the comet is floored by the real travel span + a held floor, so a held
        // drag elongates stably instead of vanishing/strobing — matching Block/Morph.
        let holding = self.caret.is_holding();
        let gap = if holding {
            m.caret_streak_gap * crate::caret::HELD_GAP_FRAC
        } else {
            m.caret_streak_gap
        };
        let held_len = m.caret_held_len;
        // While HELD, pin the squash/stretch blend to full motion so the steady
        // held length below isn't re-compressed by the oscillating settle factor —
        // a constant comet, not a per-repeat pulse (matching Block/Morph).
        let motion = if holding { 1.0 } else { motion };

        let (vx, vy) = (self.caret.vel.x, self.caret.vel.y);
        let dxt = self.caret.target.x - self.caret.pos.x;
        let dyt = self.caret.target.y - self.caret.pos.y;

        if self.caret.is_vertical_move() {
            // VERTICAL travel: a tall lozenge. Length grows with vertical speed,
            // floored by this frame's vertical advance so a fast line jump bridges;
            // the origin tail is inset by the shared gap.
            let mut raw = m
                .streak_len_for_speed(vy.abs())
                .max(self.caret.frame_dy().abs());
            if holding {
                // Steady, constant comet length while held (no per-repeat pulse).
                raw = held_len.min(m.caret_streak_max_len);
            }
            let streak_len = (raw - gap).max(tall);
            let w = thin;
            let h = tall + (streak_len - tall) * motion;
            let cx = self.caret.pos.x + w * 0.5;
            // Trail along Y: leading edge at the cell-centre anchor, body extends
            // BACK opposite the direction of travel.
            let dir = if vy.abs() > 1.0 {
                vy.signum()
            } else if dyt.abs() > f32::EPSILON {
                dyt.signum()
            } else {
                1.0
            };
            // Drop the trail anchor to the TEXT optical centre (scaled by motion, so
            // the resting bar is unchanged), consistent with the Block/Morph trail.
            let cy = self.caret.pos.y + m.caret_trail_drop * motion
                - dir * ((h - tall) * 0.5) * motion;
            let corner = 0.5 * w.min(h);
            return (cx, cy, w, h, corner);
        }

        // HORIZONTAL travel (and rest): a horizontal comet. Width grows with speed
        // (floored by this frame's horizontal advance, less the shared origin gap);
        // height collapses from the tall bar toward the thin dimension so it reads as
        // a lozenge, not a block.
        let mut raw = m
            .streak_len_for_speed(vx.abs())
            .max(self.caret.frame_dx().abs());
        if holding {
            // Steady, constant comet length while held (no per-repeat pulse).
            raw = held_len.min(m.caret_streak_max_len);
        }
        let streak_len = (raw - gap).max(thin);
        let w = thin + (streak_len - thin) * motion;
        let h = tall + (thin - tall) * motion;
        // Leading edge tracks the insertion point; the body trails BACK.
        let lead = self.caret.pos.x + thin * 0.5;
        let dir = if vx.abs() > 1.0 {
            vx.signum()
        } else if dxt.abs() > f32::EPSILON {
            dxt.signum()
        } else {
            1.0
        };
        let cx = lead - dir * (w * 0.5) * motion;
        // Drop the trail anchor to the TEXT optical centre (scaled by motion, so the
        // resting bar is unchanged), consistent with the Block/Morph trail.
        let cy = self.caret.pos.y + m.caret_trail_drop * motion;
        let corner = 0.5 * w.min(h);
        (cx, cy, w, h, corner)
    }

    /// The caret's pixel rectangle `(x, y, w, h)` of the glyph CELL at its resting
    /// target (the END of any active preedit). Handed to winit's
    /// `set_ime_cursor_area` so the OS candidate window floats just below/beside
    /// the composition caret. This is the full cell box (top-left + cell height),
    /// not the thin underline, so the IME candidate window is placed sensibly.
    pub fn caret_pixel_rect(&self) -> (f32, f32, f32, f32) {
        let (gx, _adv) = self.col_x_and_advance(self.cursor_line, self.cursor_col);
        let x = self.text_left() + gx;
        let y = self.caret_cell_top();
        (x, y, self.caret_target_w(), self.metrics.caret_h)
    }

    /// Push the current cursor position into the spring as its target. The first
    /// call snaps; later calls (after a cursor move) start a glide.
    pub fn set_caret_target(&mut self, is_edit: bool, held: bool) {
        // Keep the spring's glyph + line yardsticks in sync with the current zoom
        // so the distance-aware damping judges moves in glyphs and the row-crossing
        // (vertical) test uses the real line height.
        self.caret.set_glyph_advance(self.metrics.char_width);
        self.caret.set_line_height(self.metrics.line_height);
        // Edits always slide as a plain block; navigation streaks only on jumps.
        self.caret.set_edit_move(is_edit);
        // HELD / auto-repeat navigation builds a continuous lagging trail (the
        // spring stays springy and the streak spans the real travel).
        self.caret.set_held(held);
        let (x, y) = self.caret_target_xy();
        if is_edit {
            // EDIT-driven REFLOW moves SNAP. When a text edit carries the caret
            // across a ROW — Enter, a backspace-join, a multi-line paste/yank — the
            // text reflowed *under* the caret, so the caret must arrive exactly as
            // instantly as the text did; a spring glide there reads as the caret
            // lagging the insertion point (the "caret lags on Enter" bug). Same-line
            // typing (a horizontal edit) is NOT a reflow, so it keeps its
            // near-critical glide.
            if self.caret.crosses_row(y) {
                self.caret.jump_to(x, y);
            } else {
                self.caret.set_target(x, y);
            }
        } else {
            // NAVIGATION goes through the ZIP DISTANCE GATE: a SMALL / incremental
            // move (single char incl. held L/R, single line incl. held U/D) SNAPS
            // instantly with no glide and no trail (the plain snappy cursor), while a
            // BIG jump (a long C-a/C-e, M-</M->, a page, a search hop) keeps the
            // spring glide + trailing "----" streak. Gated on the actual distance
            // moved, not the key — so a C-e a few chars from the end snaps, a C-e
            // across a long line zips. See [`crate::caret::CaretAnim::nav_to`].
            self.caret.nav_to(x, y);
        }
    }

    /// Advance the caret spring by `dt` seconds and report whether the caret is
    /// still animating (so the windowed app knows to keep redrawing). The cosmetic
    /// SQUASH-POP is ticked on the SAME clock and OR-folded in: a small move snaps
    /// the position instantly (the spring never animates) yet still plays its pop, so
    /// the loop must stay hot while the pop runs, then idle. The pop is a draw-time
    /// scale only — ticking it touches no position state.
    pub fn step_caret(&mut self, dt: f32) -> bool {
        self.caret.step(dt);
        let popping = self.caret.step_pop(dt);
        // The cosmetic | trail fades on the same live clock; a small move snaps the
        // position instantly yet the trail still fades, so keep the loop hot while it
        // does, then idle. Decoupled from position (ticking it touches no spring state).
        let trailing = self.caret.step_trail(dt);
        self.caret.is_animating() | popping | trailing
    }

    /// Read-only snapshot of the caret spring for the timeline-capture sidecar:
    /// the animated `pos`, the true `target`, the [0,1] `settle_factor`, and
    /// whether the spring is still animating. Lets a timeline frame record the
    /// caret's trajectory (0 -> mid -> settled) machine-readably per step.
    pub fn caret_snapshot(&self) -> ((f32, f32), (f32, f32), f32, bool) {
        (
            (self.caret.pos.x, self.caret.pos.y),
            (self.caret.target.x, self.caret.target.y),
            self.caret.settle_factor(),
            self.caret.is_animating(),
        )
    }

    /// Read-only report of the cosmetic SQUASH-POP for the timeline-capture sidecar:
    /// `(scale, drawn_w, drawn_h)`. `scale` is the pop factor this frame (1.0 settled,
    /// dipping to `CARET_POP_SCALE` right after a move); `drawn_w`/`drawn_h` are the
    /// caret BLOCK rect's dimensions AS DRAWN — the morph geometry scaled by the pop —
    /// so a timeline run can assert, machine-readably, that the block starts squashed
    /// (<1) and eases back to full size while the position stays pinned to target. The
    /// `--screenshot` path renders the settled state (scale 1.0), so a plain capture
    /// reports a full-size block.
    pub fn caret_pop_report(&self) -> (f32, f32, f32) {
        let s = self.caret.pop_scale();
        let (_cx, _cy, w, h, _c, _ax, _ay) = self.caret_geometry();
        (s, w * s, h * s)
    }

    /// Read-only report of the caret's drawn TRAIL geometry for the held-capture
    /// sidecar: `(holding, length, tail, head)`. Wraps the SAME private
    /// `caret_geometry()` the Block/Morph renderer draws from — `length` is the
    /// on-screen streak length along the travel axis (`half_along * 2`) and the
    /// endpoints are `center ± axis * (length/2)` — plus the latched `holding`
    /// flag. Lets a HELD run assert, per step, that the trail is present (length
    /// past the streak gap) and never collapses to zero, straight from the JSON.
    pub fn caret_trail_report(&self) -> (bool, f32, (f32, f32), (f32, f32)) {
        let (cx, cy, w, _h, _corner, ax, ay) = self.caret_geometry();
        let half = w * 0.5;
        let tail = (cx - ax * half, cy - ay * half);
        let head = (cx + ax * half, cy + ay * half);
        (self.caret.is_holding(), w, tail, head)
    }

    /// The COSMETIC | TRAIL quad for THIS frame, or `None` when no streak is active:
    /// `(center_x, center_y, w, h, corner, ax, ay, alpha)`. Wraps the spring's pure
    /// [`crate::caret::CaretAnim::trail_geometry`] with the zoomed streak thickness /
    /// gap / text-centre drop, the small motion corner radius, and the spring's fading
    /// `trail_alpha`. Decoupled from position — it spans the latched OLD→NEW caret
    /// points, not `pos`/`target`. Shared by `prepare` (to draw it) and
    /// `caret_cosmetic_report` (to report it), so the JSON matches the drawn quad.
    pub(super) fn caret_trail_geometry(&self) -> Option<(f32, f32, f32, f32, f32, f32, f32, f32)> {
        if !self.caret.trail_active() {
            return None;
        }
        let m = &self.metrics;
        // The cosmetic | anchors on the SAME x the active caret look uses:
        //   * Block / Morph rest on a CELL (the block covers the glyph) → centre the
        //     streak on the cell (half the block width) so the | runs down the MIDDLE.
        //   * I-beam sits at the INSERTION POINT (the thin bar at `pos.x`, centred on
        //     `IBEAM_W`) → anchor the | on that bar, NOT the cell centre, matching
        //     `caret_ibeam_geometry`'s `cx = pos.x + thin*0.5`.
        let center_x_drop = match crate::caret::mode() {
            CaretMode::Ibeam => IBEAM_W * m.zoom * 0.5,
            _ => self.caret_block_w() * 0.5,
        };
        let (center, half_along, half_across, axis) = self.caret.trail_geometry(
            m.caret_streak_h,
            m.caret_streak_gap,
            m.caret_trail_drop,
            center_x_drop,
        );
        let w = half_along * 2.0;
        if w <= 0.0 {
            return None;
        }
        let corner = STREAK_RADIUS * m.zoom;
        Some((
            center.x,
            center.y,
            w,
            half_across * 2.0,
            corner,
            axis.0,
            axis.1,
            self.caret.trail_alpha(),
        ))
    }

    /// Read-only report of the COSMETIC | TRAIL for the timeline/held-capture sidecar:
    /// `(present, length, vertical, held, alpha, sweep, tail, head)`. `present` is
    /// whether a streak draws this frame; `length` is its on-screen span along the
    /// travel axis (it GROWS old→new as the sweep draws on); `alpha` the current fade;
    /// `vertical` whether it is the up/down | vs a horizontal jump streak; `held`
    /// whether it belongs to an auto-repeat (one steady |); `sweep` ∈ [0,1] the eased
    /// SWEEP progress (0 = head at old, 1 = head arrived on the caret); `tail`/`head`
    /// its endpoints in canvas px (the `head` advances old→new over the sweep). Lets a
    /// capture assert, straight from JSON, that the streak SWEEPS from the old position
    /// toward the caret over the first ~55ms while pos stays pinned, then fades; that a
    /// 1-char hop shows none; a held-down run stays present + steady; a held-right none.
    pub fn caret_cosmetic_report(
        &self,
    ) -> (bool, f32, bool, bool, f32, f32, (f32, f32), (f32, f32)) {
        let held = self.caret.is_trail_held();
        // The eased SWEEP progress (0 = head at the OLD caret, 1 = swept onto the NEW
        // one): exposed straight so a timeline run can assert the sweep old→new without
        // re-deriving it from the endpoints.
        let sweep = self.caret.trail_sweep_p();
        match self.caret_trail_geometry() {
            Some((cx, cy, w, _h, _c, ax, ay, alpha)) => {
                let half = w * 0.5;
                let tail = (cx - ax * half, cy - ay * half);
                let head = (cx + ax * half, cy + ay * half);
                (true, w, self.caret.is_trail_vertical(), held, alpha, sweep, tail, head)
            }
            None => (
                false,
                0.0,
                self.caret.is_trail_vertical(),
                held,
                0.0,
                sweep,
                (0.0, 0.0),
                (0.0, 0.0),
            ),
        }
    }

    /// Inject the I-beam typing-RECOIL impulse into the caret spring (px/s). A
    /// no-op for the Block/Morph looks — the windowed app only calls this when the
    /// I-beam mode is active — so their spring behaviour is untouched. The spring
    /// self-settles the kick through its normal integration.
    pub fn caret_kick(&mut self, dx: f32, dy: f32) {
        self.caret.kick(dx, dy);
    }

    /// RECOIL the caret in `dir` (a blocked-action bump). Unlike the I-beam typing
    /// kick this fires in EVERY caret look — a blocked motion/scroll/undo/delete
    /// bumps the caret away from the wall in Block/Morph/I-beam alike. Delegates to
    /// [`crate::caret::CaretAnim::recoil`]; the spring self-settles it back to rest.
    pub fn caret_recoil(&mut self, dir: crate::caret::RecoilDir) {
        self.caret.recoil(dir);
    }

    /// Place the caret AT REST on the current target (no glide; settle_factor 1 =
    /// the resting rounded square on the glyph). Used by the deterministic
    /// `--screenshot` path.
    pub fn settle_caret(&mut self) {
        self.set_caret_target(false, false);
        self.caret.snap_to_target();
    }

    /// Inject a deterministic mid-glide state for the `--screenshot-motion`
    /// path: the logical cursor target is the cursor position, but the animated
    /// caret is part-way through a fast HORIZONTAL glide along the line (coming
    /// from the LEFT, heading right toward the target), so its `settle_factor()`
    /// is ~0 — the caret has dropped to the baseline and stretched into a long
    /// trailing underline whose tail points back to the left. A horizontal glide
    /// (the common "move along a line" case) is chosen so the streak + its trail
    /// read clearly. No clock is consulted, so the produced frame is reproducible.
    pub fn inject_motion_demo(&mut self) {
        // Place the logical cursor at a deterministic, comfortably on-screen
        // mid-line spot so the rightward streak AND its leftward tail are fully
        // visible (a cursor at col 0 would push the trailing tail off-screen).
        // Clamp to the document so this is safe on short sample files.
        let demo_line = 2usize.min(self.line_count().saturating_sub(1));
        let line_chars = self.line_glyph_xs(demo_line).len().saturating_sub(1);
        self.cursor_line = demo_line;
        self.cursor_col = 24usize.min(line_chars);
        self.set_caret_target(false, false);
        let (tx, ty) = self.caret_target_xy();
        let target = Sample { x: tx, y: ty };

        // The glide started well to the LEFT of the target and is part-way along,
        // moving RIGHT fast. The animated x is several glyph cells short of the
        // target; the high horizontal speed forces the settle factor toward 0 so
        // the caret is a long trailing streak (tail to the left), not a square.
        let back: f32 = 9.0 * self.metrics.char_width; // ~9 cells left of target
        const PHASE: f32 = 0.55; // fraction of the gap still remaining to the left
        let pos = Sample { x: tx - back * PHASE, y: ty };
        // Moving rightward (toward the target) fast: the high speed both collapses
        // the settle factor and drives the velocity-scaled streak length long.
        let vel = Sample { x: 1900.0, y: 0.0 };
        self.caret.inject_motion(target, pos, vel);
    }

    /// Vertical sibling of [`Self::inject_motion_demo`] for `--screenshot-motion-v`:
    /// a deterministic mid-glide caret travelling DOWN between lines, coming from
    /// ABOVE the target, so `settle_factor()` is ~0 and the caret has slid to a
    /// thin amber bar on the cell's LEFT edge whose tail trails UP the lines it
    /// passed. No clock is consulted, so the frame is reproducible.
    pub fn inject_motion_demo_vertical(&mut self) {
        // Cursor a few lines down with room ABOVE for the trailing bar to show.
        let demo_line = 6usize.min(self.line_count().saturating_sub(1));
        let line_chars = self.line_glyph_xs(demo_line).len().saturating_sub(1);
        self.cursor_line = demo_line;
        self.cursor_col = 12usize.min(line_chars);
        self.set_caret_target(false, false);
        let (tx, ty) = self.caret_target_xy();
        let target = Sample { x: tx, y: ty };

        // The glide started several lines ABOVE the target and is part-way along,
        // moving DOWN fast. The high vertical speed collapses the settle factor and
        // drives the streak long, so the caret is a tall left-edge bar trailing up.
        let back: f32 = 5.0 * self.metrics.line_height; // ~5 lines above target
        const PHASE: f32 = 0.55; // fraction of the gap still remaining above
        let pos = Sample { x: tx, y: ty - back * PHASE };
        let vel = Sample { x: 0.0, y: 1900.0 };
        self.caret.inject_motion(target, pos, vel);
    }

    /// DIAGONAL sibling of [`Self::inject_motion_demo`] for `--screenshot-motion-d`:
    /// a deterministic mid-glide caret jumping between two points on DIFFERENT rows
    /// AND columns (e.g. an incremental-search hop between matches), coming from the
    /// upper-LEFT toward the lower-right. The trail must render as a TRUE SLANT from
    /// source to target — not a vertical-only bar (the axis-snapped bug). No clock is
    /// consulted, so the frame is reproducible.
    pub fn inject_motion_demo_diagonal(&mut self) {
        // Land a few lines down and well along the line, with room up-and-left for
        // the trailing slant to show.
        let demo_line = 6usize.min(self.line_count().saturating_sub(1));
        let line_chars = self.line_glyph_xs(demo_line).len().saturating_sub(1);
        self.cursor_line = demo_line;
        self.cursor_col = 22usize.min(line_chars);
        self.set_caret_target(false, false);
        let (tx, ty) = self.caret_target_xy();
        let target = Sample { x: tx, y: ty };

        // The glide started up-and-left of the target and is part-way along, moving
        // DOWN-RIGHT fast. Equal-magnitude x/y velocity ⇒ a ~45° travel vector, so
        // the streak is a clean diagonal tracer (not snapped to either axis).
        let back_x: f32 = 9.0 * self.metrics.char_width;
        let back_y: f32 = 4.0 * self.metrics.line_height;
        const PHASE: f32 = 0.55;
        let pos = Sample {
            x: tx - back_x * PHASE,
            y: ty - back_y * PHASE,
        };
        let vel = Sample { x: 1600.0, y: 1600.0 };
        self.caret.inject_motion(target, pos, vel);
    }
}
