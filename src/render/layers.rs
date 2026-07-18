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

/// The hanging BLOCKQUOTE pull-quote mark: a big DIM opening quotation mark (`“`)
/// shaped in the WORLD'S OWN DISPLAY SERIF ([`theme::Theme::font`], NOT the ornament
/// or symbol face) and hung in the LEFT MARGIN at each blockquote block's first line
/// — semantically honest for a quote, and a showcase of the world's type. TUNABLE
/// (live-taste): the size bump over body ink. Sits in the heading/ornament band
/// (~1.8–2.2×); still value-only, never amber (DESIGN §3). See
/// [`super::TextPipeline::prepare_ornaments`].
const QUOTE_MARK_SCALE: f32 = 2.0;

/// The glyph the pull-quote mark draws — U+201C LEFT DOUBLE QUOTATION MARK (the
/// pull-quote's opening mark). Shaped in the world's display serif so it reads as
/// real type, not a symbol-font ornament.
const QUOTE_MARK_GLYPH: char = '\u{201C}';

/// One GFM table's shaped GRID layout — the shared output of
/// [`super::TextPipeline::shape_table_grid`], consumed by both the row-height
/// RESERVATION (`compute_table_layout`) and the DRAW pass (`prepare_table_grid`)
/// so the two can never disagree on a wrapped table's geometry. `col_x`/`col_w`
/// are the per-column x-offset + width; `cells` is one reshaped-at-column-width
/// buffer per non-empty cell as `(grid-row, column, buffer, wrapped width)`;
/// `row_heights` is each grid row's height (max wrapped-cell height, ≥ one
/// line-height).
struct TableGridShaped {
    col_x: Vec<f32>,
    col_w: Vec<f32>,
    cells: Vec<(usize, usize, GlyphBuffer, f32)>,
    row_heights: Vec<f32>,
}

/// THE ONE TABLE-GRID SHAPE SITE (the "merge, don't align" fix for the
/// geometry-computed-twice bug): [`TextPipeline::compute_table_layout`] shapes
/// every table block ONCE per reshape via [`TextPipeline::shape_table_grid`] and
/// WRITES the result here — it's already doing the shaping to compute the
/// row-height RESERVATION, so this just keeps what it built instead of throwing
/// it away. [`TextPipeline::prepare_table_grid`] (the per-frame draw) then only
/// ever READS this cache; it never calls `shape_table_grid` itself. That makes a
/// reservation/draw divergence structurally unrepresentable, closing the real gap
/// the old two-call-sites design had: `TextPipeline::prepare`'s per-frame
/// `sync_wrap_width` re-wraps the document buffer to the LIVE `text_wrap_width()`
/// on a width-only change (page-mode toggle, measure edit, a width-preserving
/// theme reshape) WITHOUT running a full `set_text` reshape — so `reshape_count`
/// does not advance and `compute_table_layout` does not re-run. Before this cache,
/// `prepare_table_grid` still called `shape_table_grid` itself every frame with
/// the FRESH (post-`sync_wrap_width`) `avail`, so on exactly that frame it drew a
/// table shaped at the NEW width while the row it was drawn into was still the
/// row height RESERVED at the OLD width — a taller/shorter drawn grid than the
/// document had made room for, painting over the next row. Now the draw simply
/// reads the SAME shape the reservation used, so on that frame both are equally
/// (and consistently) stale until the next real reshape catches both up together.
///
/// Keyed on `reshape_count` (bumped ONLY by a real `set_text`/`set_text_full`
/// reshape — the exact seam `compute_table_layout` itself runs on, so the key and
/// the write are inseparable). Entries are `(range.start, TableGridShaped)` for
/// every table block with `ncols > 0` found at that reshape — document byte range
/// start is stable across a pure caret move within the same table, and is the
/// same key both `compute_table_layout` and `prepare_table_grid` derive their
/// table list from (`TextPipeline::table_blocks`, itself sourced from the same
/// `md_spans` field both read).
pub(super) struct TableGridCache {
    version: std::cell::Cell<Option<u64>>,
    entries: std::cell::RefCell<Vec<(usize, TableGridShaped)>>,
}

impl TableGridCache {
    pub(super) fn new() -> Self {
        Self { version: std::cell::Cell::new(None), entries: std::cell::RefCell::new(Vec::new()) }
    }
}

/// INLINE IMAGES — the gentle rounded-corner radius (logical px, zoom-scaled) of an
/// inline image quad + its missing-file placeholder card. A calm card edge, not a
/// hard rectangle. TUNABLE.
#[cfg(not(target_arch = "wasm32"))]
const IMAGE_CORNER_PX: f32 = 4.0;

/// INLINE IMAGES — the opacity multiply applied to a REVEALED image (the caret is
/// on its `![alt](path)` source line, whose raw source reveals CENTRED OVER it — the
/// caption model). Off the caret's line the image draws at full opacity (`1.0`);
/// revealed, it recedes to a calm dimmed backdrop UNDER the editable caption. Pulled
/// down from the reveal-grow round's `0.55` to buy the centred source real contrast
/// over arbitrary image pixels (the caption model puts the text ON the image, not
/// beside it). TASTE TUNABLE — flagged for live review, judged by LOOKING at the
/// `gallery/image-reveal/` revealed captures over a dark + a light world (named like
/// `THEME_FONT_DEBOUNCE` / the copy-pulse tunables).
#[cfg(not(target_arch = "wasm32"))]
const IMAGE_REVEAL_DIM_ALPHA: f32 = 0.4;

/// INLINE IMAGES — the CAPTION SCRIM's vertical padding (px, unzoomed) above and
/// below the revealed source's own text-line band. A hair of breathing room so the
/// scrim reads as a soft caption plate, not a tight underline. TASTE TUNABLE —
/// flagged for live review. Scaled by zoom at build time.
#[cfg(not(target_arch = "wasm32"))]
const CAPTION_SCRIM_PAD_Y: f32 = 3.0;

/// INLINE IMAGES — the CAPTION SCRIM's horizontal overhang (px, unzoomed) past each
/// end of the revealed source's shaped extent, so the plate hugs the text with a
/// small even inset rather than clipping the glyphs. TASTE TUNABLE — flagged for
/// live review. Scaled by zoom at build time.
#[cfg(not(target_arch = "wasm32"))]
const CAPTION_SCRIM_PAD_X: f32 = 4.0;

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

    /// Per-frame LAVA-LAMP GROUND ([`crate::theme::Background::Lava`]): a slow 2D
    /// viewport-space metaball field behind the page, visible MARGINS-ONLY over
    /// the flat ground the background pass just laid. Column width changes only
    /// the mask; they never resize/recompose the field. A total no-op for
    /// every non-lava world — so all fifteen shipped worlds stay byte-identical.
    ///
    /// The effective background honors the dev gallery knob (`crate::lava::
    /// env_override`); the column bounds come from the SAME `page_geometry()`
    /// (the one geometry owner) the background pass reads, with the identical
    /// page-off collapse (col covers the whole canvas → the mask has no margin →
    /// nothing draws), and the effective phase from [`TextPipeline::lava_render_phase`]
    /// (env override > Reduce-Motion freeze > the App-driven [`TextPipeline::lava_phase`]).
    pub(super) fn prepare_lava_layer(&mut self, queue: &wgpu::Queue, width: u32, height: u32) {
        let (page_on, _measure, col_left, col_w) = self.page_geometry();
        let (bg_left, bg_w) = if page_on {
            (col_left, col_w)
        } else {
            (0.0, width as f32)
        };
        let rail_carved = self.lava_rail_carved(height);
        let gutter_rect = self.lava_gutter_carve_rect(height);
        // FROST PILLS (the shipped headed-doc default): one pill per drawn outline
        // entry, hugging its text extents. Empty in every non-frost frame (no lava
        // ground, no outline, or `AWL_LAVA_FROST=off`), so the shader's frost path
        // is inert (`pill_count == 0`) and a heading-less doc stays byte-identical.
        let frost_pills = if crate::lava::frost_on()
            && self.effective_background().lava_params().is_some()
        {
            self.lava_frost_pill_rects(height)
        } else {
            Vec::new()
        };
        let zoom = self.metrics.zoom;
        let frost_params = [
            crate::lava::FROST_DIM,
            crate::lava::FROST_BLUR_PX * zoom,
            crate::lava::FROST_FEATHER_PX * zoom,
        ];
        let params = self.effective_background().lava_params().map(
            |(ground, lo, hi, edge, dithered)| {
                // A Bayer-posterized source and the downsampled separable blur
                // form axis-aligned crosses. While frost is active this lava is
                // visible only through the blur capture, so feed that capture the
                // same field without posterization; the unobscured document keeps
                // the world's authored dither unchanged.
                (
                    ground,
                    lo,
                    hi,
                    edge,
                    crate::lava::dither_for_blur(dithered, self.backdrop_blur()),
                )
            },
        );
        let phase = self.lava_render_phase();
        self.lava_pipeline
            .prepare(
                queue,
                width,
                height,
                self.lava_field_viewport,
                bg_left,
                bg_w,
                rail_carved,
                gutter_rect,
                &frost_pills,
                frost_params,
                params,
                phase,
            );
    }

    /// TWINKLING STARS (`theme::AmbientStyle::Stars`, the TWINKLING-STARS round):
    /// tiny individually-phased breathing points in the page-mode MARGINS,
    /// drawn right after the lava layer and before the page frame / washes /
    /// text. A TOTAL no-op (zero instances) for every `AmbientStyle::None`
    /// world — fifteen of the sixteen stay byte-identical — and for page-off
    /// (the column spans the canvas → the margin gate culls everything, the
    /// background pass's own collapse).
    ///
    /// THE SHAPE: the star LAYOUT is a proto cache — [`crate::stars::layout`]'s
    /// deterministic position-hash scatter, rebuilt only when the (viewport,
    /// star params) key misses ([`TextPipeline::stars_proto_key`]) — and the
    /// per-frame work is pure arithmetic over the visible set: cull each proto
    /// against the LIVE column band ([`crate::stars::in_margin`], the one
    /// placement-law owner, reading the SAME `page_geometry()` every ground
    /// layer reads — an adaptive-column shift or live resize re-culls the same
    /// anchored field) plus the margin-INK zones (the outline's pill rects +
    /// the gutter's corner rect, the same owners the lava frost/carve reads —
    /// so a star never sits under the dim rail text), then breathe its alpha
    /// by [`crate::stars::brightness`] at the resolved twinkle phase
    /// ([`TextPipeline::stars_render_phase`]: env knob > Reduce-Motion freeze >
    /// the shared ambient clock — frozen at t=0 in every headless capture).
    pub(super) fn prepare_stars_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        let Some((tint, cell_px, density, size_px, peak, floor)) =
            crate::theme::active().render_caps.ambient.stars_params()
        else {
            // A starless world uploads ZERO instances (and clears the proto
            // cache so a later switch back rebuilds against fresh params).
            self.stars_proto_key = None;
            self.stars_pipeline
                .prepare_multicolor(device, queue, width, height, &[]);
            return;
        };
        // DPI/ZOOM INVARIANCE: the authored `cell_px`/`size_px` (`theme/worlds.rs`)
        // are PHYSICAL px at scale 1.0. The layout scatters one candidate per
        // `cell_px` cell over the PHYSICAL viewport (`width`/`height`), and each dot
        // draws `size_px` physical px wide — so on a 2x-retina surface (the SAME
        // logical window rasterized 2x) an unscaled cell packs ~4x the grid cells
        // (the ~5.6x-denser field the user saw) and every dot renders at half its
        // intended LOGICAL size. Scale BOTH by the TOTAL logical->physical factor
        // `scale` = user-zoom × device-DPI (exactly `Metrics::with_dpi`'s own `s`):
        // `metrics.zoom` is the user zoom ALONE (the frost path right above scales by
        // it), so it must also be multiplied by `self.dpi` to cover retina. With a
        // 2x cell over a 2x viewport the grid is the SAME logical structure (constant
        // density) and a 2x dot keeps a constant logical size. Shadowing carries the
        // scaled values through the proto key, the margin cull, and the quads below.
        // `STAR_MARGIN_GAP_PX` + the 1px AA fringe stay in physical px (the placement
        // law owner `stars::in_margin` is untouched).
        let scale = self.metrics.zoom * self.dpi;
        let cell_px = cell_px * scale;
        let size_px = size_px * scale;
        // Proto cache: rebuild the scattered layout only when the size, the DPI/zoom
        // scale, or the authored params change (a theme switch onto different star
        // data). The scale rides in through the now-scaled `cell_px`/`size_px`.
        let key = (width, height, cell_px.to_bits(), density.to_bits());
        if self.stars_proto_key != Some(key) {
            self.stars_protos =
                crate::stars::layout(width as f32, height as f32, cell_px, density);
            self.stars_proto_key = Some(key);
        }
        let (page_on, _measure, col_left, col_w) = self.page_geometry();
        let (band_left, band_w) = if page_on {
            (col_left, col_w)
        } else {
            (0.0, width as f32)
        };
        let band_right = band_left + band_w;
        // MARGIN-INK no-star zones: the outline's per-entry pill rects + the
        // gutter's corner rect — the SAME geometry owners the lava frost pills
        // and gutter carve read, so a star can never crowd the rail's dim ink.
        let mut ink_zones: Vec<[f32; 4]> = if self.outline_visible(height) {
            self.lava_frost_pill_rects(height)
        } else {
            Vec::new()
        };
        if self.gutter_visible() {
            if let Some(r) = self.gutter_carve_rect(height) {
                ink_zones.push(r);
            }
        }
        let half = size_px * 0.5;
        let phase = self.stars_render_phase();
        let gap = crate::stars::STAR_MARGIN_GAP_PX;
        let mut quads: Vec<([f32; 4], [u8; 4])> = Vec::with_capacity(self.stars_protos.len());
        for s in &self.stars_protos {
            if !crate::stars::in_margin(s.x, half, band_left, band_right, gap) {
                continue;
            }
            let e = half + 1.0;
            if ink_zones.iter().any(|r| {
                s.x + e > r[0] && s.x - e < r[2] && s.y + e > r[1] && s.y - e < r[3]
            }) {
                continue;
            }
            let a = crate::stars::brightness(s.seed, phase, floor, peak);
            let alpha = (a * 255.0).round().clamp(0.0, 255.0) as u8;
            quads.push((
                [s.x - half, s.y - half, size_px, size_px],
                [tint.r, tint.g, tint.b, alpha],
            ));
        }
        // Fully-rounded corners turn each tiny quad into a soft dot (the SDF
        // becomes a circle when the radius reaches the half-size).
        self.stars_pipeline.set_corner(half);
        self.stars_pipeline
            .prepare_multicolor(device, queue, width, height, &quads);
    }

    /// THE PAGE FRAME (`theme::PageFrame`, the personality-assignment round's
    /// graduated capability — the `AWL_PAGE_BORDER` gallery probe's geometry,
    /// now driven by per-world DATA instead of an env var): four thin quads
    /// framing the writing column over the document's vertical extent, or
    /// ZERO rects for every `PageFrame::None` world (all but Wagtail — a
    /// byte-identical no-op there). The column bounds come from the SAME
    /// `column_left()`/`column_width()` owners every other layer reads; the
    /// vertical extent is the top of the first visual row to the bottom of
    /// the last, clamped so a tall doc's frame still closes on-canvas. The
    /// frame straddles the column boundary OUTWARD (into the margin), so it
    /// never sits under the text. Ink comes from `theme::page_frame_ink()`
    /// (re-tinted in `sync_theme_colors`); the pipeline draws hard-edged
    /// (dither 1.0 — see the field's own doc). Deliberately NOT gated on
    /// page mode, mirroring the probe the taste pick was made on: page-off,
    /// the column is the full writing area and the frame hugs the window's
    /// own inset — the "page as a deliberate object" read either way.
    pub(super) fn prepare_page_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        let theme::PageFrame::Line { weight_px } = crate::render::effective_page_frame()
        else {
            self.page_frame_pipeline
                .prepare(device, queue, width, height, &[]);
            return;
        };
        let t = weight_px.max(0.1);
        let left = self.column_left();
        let w = self.column_width();
        let top = self.doc_top().max(0.0);
        let bottom = (self.doc_top() + self.total_doc_height()).min(height as f32 - 1.0);
        let h = (bottom - top).max(0.0);
        let right = left + w;
        let rects = [
            [left - t, top - t, w + 2.0 * t, t], // top edge
            [left - t, bottom, w + 2.0 * t, t],  // bottom edge
            [left - t, top - t, t, h + 2.0 * t], // left edge
            [right, top - t, t, h + 2.0 * t],    // right edge
        ];
        self.page_frame_pipeline
            .prepare(device, queue, width, height, &rects);
    }

    /// THE FULL LEFT-MARGIN RAIL CARVE decision for this frame — the OLD headed-doc
    /// treatment, now DEMOTED behind the FROST default. Under the shipped default
    /// ([`crate::lava::FROST_RAIL_DEFAULT`] `true`) this is ALWAYS `false`: a headed
    /// doc keeps BOTH margins alive and the per-entry frost pills
    /// ([`Self::lava_frost_pill_rects`]) carry the outline's legibility instead of
    /// flattening the whole rail. Flipping that ONE const to `false` re-arms this
    /// carve — a lava ground active (the CAPABILITY, never a world name, per
    /// `theme_caps_law`) AND the margin OUTLINE actually DRAWN
    /// ([`Self::outline_visible`], the same `outline_layout` gate) — feeding the
    /// shader's still-wired `rail` global, for a clean one-line data revert to the
    /// pre-frost behaviour. The ONE owner [`Self::prepare_lava_layer`] uploads it.
    pub(super) fn lava_rail_carved(&self, height: u32) -> bool {
        !crate::lava::FROST_RAIL_DEFAULT
            && self.effective_background().lava_params().is_some()
            && self.outline_visible(height)
    }

    /// THE GUTTER'S LOCAL CORNER CARVE rect for this frame: `Some([left, top,
    /// right, bottom])` (px) exactly when a lava ground is active AND the
    /// bottom-left GUTTER is actually DRAWN ([`Self::gutter_visible`]), else
    /// `None`. The bounded bottom-left region around the gutter block is carved
    /// out of the field mask (so its `muted`/`faint` stack never swims in the
    /// lamp) while the rest of both margins keep their lamp — the fix for the
    /// gutter gating the WHOLE-margin carve on nearly every page-mode buffer.
    /// Geometry comes from the SAME [`Self::gutter_carve_rect`] owner
    /// `prepare_gutter`/`gutter_layout` ride, so the carve can never disagree
    /// with the drawn gutter block.
    pub(super) fn lava_gutter_carve_rect(&self, height: u32) -> Option<[f32; 4]> {
        if self.effective_background().lava_params().is_none() || !self.gutter_visible() {
            return None;
        }
        self.gutter_carve_rect(height)
    }

    /// Upload the document text layer with the full-ink default color — the one
    /// glyphon `prepare` per frame (the caret is a quad drawn underneath).
    pub(super) fn prepare_text_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        // DIFF-AS-PREVIEW: while the page column wears its card dressing, the
        // document glyphs clip to the panel's interior band, so a scrolled
        // transcript slides UNDER the card edge instead of over the margin.
        let (clip_top, clip_bottom) = match self.doc_clip_band(height as f32) {
            Some((t, b)) => (t as i32, b as i32),
            None => (0, height as i32),
        };
        let bounds = TextBounds {
            left: 0,
            top: clip_top,
            right: width as i32,
            bottom: clip_bottom,
        };
        let doc_top = self.doc_top();

        // Every glyph whose `color_opt` is None resolves to the full-ink default at
        // prepare time (per-span md/syntax/CJK colors override where present).
        let default_color = theme::base_content().to_glyphon();
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
        // THE 1-BIT CARET ROUND: `caret_invert` is parked EMPTY up front, so
        // any branch that does NOT draw a block-look caret this frame —
        // Ibeam, or (on an ORDINARY world) the morph silhouette / glyphless
        // bar — and a live theme switch AWAY from a one-bit world never
        // leaves a stale rect from a PRIOR frame's inverted block still
        // drawing. Only `prepare_caret_block`, when it runs on a one-bit
        // world, repopulates it with this frame's real rect.
        self.caret_invert.prepare(device, queue, width, height, &[]);
        // DIFF-AS-PREVIEW: the caret is parked on the transcript's line 1; once
        // the diff scrolls, that row leaves the panel band — park every caret
        // quad rather than let it paint over the card's rim / the margin above
        // (quads don't clip to `TextBounds` the way glyphs do).
        if let Some((band_top, band_bottom)) = self.doc_clip_band(height as f32) {
            let (_, cy, _, ch) = self.caret_pixel_rect();
            if cy < band_top || cy + ch > band_bottom {
                self.caret_pipeline.prepare_empty();
                self.caret_glyph_pipeline.clear();
                return;
            }
        }
        // MORPH FOLDS TO BLOCK ON AN INK-CARET WORLD (both special block styles;
        // documented call — see CLAUDE.md's "1-bit Wagtail caret" round /
        // `caret_invert`'s field doc + `CaretBlockStyle::folds_morph_to_block`):
        // the glyph-silhouette look recolors the cursor's own letter to `primary`,
        // which on an ink-caret world is the SAME value as the letter's own ink —
        // an invisible no-op recolor (Wagtail's white-on-white), or, for a Filled
        // world, a green silhouette that vanishes into the green block beneath it.
        // Building a distinct glyph-morph for that would be per-glyph pipeline work
        // for a mode whose selling point (a colored accent letter) doesn't exist
        // when the caret IS the ink; the block path already makes the letter
        // legible (InverseVideo flips it, Filled knocks it out), so Morph degrades
        // to Block here. Ibeam is UNCHANGED — its thin bar sits BETWEEN glyph
        // cells, never over one, so it never collides with a glyph's own ink.
        let mode = if theme::active().render_caps.caret_block_style.folds_morph_to_block()
            && crate::caret::mode() == CaretMode::Morph
        {
            CaretMode::Block
        } else {
            crate::caret::mode()
        };
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
            self.prepare_caret_block(device, queue, width, height);
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
    /// [`prepare_caret_layer`]'s final dispatch arm; byte-identical on every ORDINARY
    /// world — see the one-bit branch at the bottom (added by THE 1-BIT CARET ROUND)
    /// for the true-inverse-video path.
    fn prepare_caret_block(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) {
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

        match theme::active().render_caps.caret_block_style {
        theme::CaretBlockStyle::Filled => {
            // THE AUTHENTIC CRT PHOSPHOR BLOCK (an ink-caret world, `primary ==
            // base_content`; Cassowary): an opaque `primary` cell drawn UNDER the
            // text exactly like an ORDINARY world's block (below), PLUS the covered
            // glyph knocked back through in the GROUND colour. A plain block here
            // would paint green-on-green and ERASE the letter (unlike an amber-vs-ink
            // block, whose value contrast keeps the letter legible); the knockout
            // fixes that WITHOUT the `InverseVideo` photo-negative (which on a
            // chromatic ink would flip green → magenta, not a lit cell).
            self.caret_pipeline
                .prepare_directed(queue, width, height, cx, cy, cw, ch, ccorner, ax, ay);
            // KNOCKOUT: reuse the MORPH silhouette pipeline (drawn OVER the text)
            // to repaint the covered glyph — the SAME anchor glyph the block sits on
            // (both key off `caret.pos`, so they align by construction) — in
            // `primary_content` (the ground; set at the draw site below). Only when
            // SETTLED on a real inhabited glyph: mid-glide the block is a thin streak
            // with no full cell to punch, and a glyphless anchor (space / line start /
            // empty line) is simply a solid phosphor cell with nothing to knock out.
            let settled = self.caret.settle_factor() >= CARET_MORPH_SETTLE_SHOW;
            if settled && self.prepare_caret_masks(device, queue) {
                // THE KNOCKOUT COLOUR — the GROUND (`primary_content`), set at the
                // draw site so it is authoritative even in the headless capture
                // (which never runs `sync_theme_colors`); on a Filled world the
                // silhouette pipeline is ONLY ever the knockout (morph folds to
                // block), so this override is total, never fighting a morph accent.
                self.caret_glyph_pipeline
                    .set_color(theme::primary_content().rgb_bytes());
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
            } else {
                self.caret_glyph_pipeline.clear();
            }
        }
        theme::CaretBlockStyle::InverseVideo => {
            self.caret_glyph_pipeline.clear();
            // TRUE 1-BIT WORLDS: an opaque pre-text quad here — even one
            // tinted `primary` (pure white on a one-bit world, the SAME
            // value as the text ink) — would white-out a glyph the caret
            // lands on: the glyph's own alpha-blended draw on TOP of an
            // already-white quad composites into uniform white with no
            // visible seam (the exact bug this round fixes — a caret on a
            // heading's `#` erased the `#`). Route the caret's own ANIMATED
            // rect (this frame's settle-driven position + streak size, from
            // `caret_geometry`/the descender extension above — travel-axis
            // ROTATION is dropped, `fs_invert` has no axis field, and a
            // rotated streak is rare + still legible axis-aligned) through
            // `caret_invert` instead: drawn AFTER text with a `OneMinusDst`
            // blend, so it flips whatever the ground+text already
            // composited to beneath it — black ground -> white, white glyph
            // ink -> black — making the glyph under the caret legible.
            // `caret_pipeline` draws NOTHING this frame (`prepare_empty`):
            // an opaque quad here would hand the invert pass a uniform-white
            // destination with nothing left to flip into a visible glyph.
            //
            // THE ROUNDED-SILHOUETTE FIX: `ccorner` here is the EXACT SAME
            // already-zoom/settle/squash-animated radius `caret_geometry` +
            // `pop_scaled` computed above — the ONE Rust-side owner an
            // ORDINARY world's `caret_pipeline.prepare_directed` call below
            // draws with too. Uploading it into `caret_invert` via
            // `set_corner` (consumed by `fs_invert`'s SDF discard — see that
            // shader entry point's doc) makes the 1-bit caret's silhouette
            // round the SAME way, rather than falling back to a hard square.
            self.caret_invert.set_corner(ccorner);
            let rect = [cx - cw * 0.5, cy - ch * 0.5, cw, ch];
            self.caret_invert.prepare(device, queue, width, height, &[rect]);
            self.caret_pipeline.prepare_empty();
        }
        theme::CaretBlockStyle::Normal => {
            self.caret_glyph_pipeline.clear();
            self.caret_pipeline
                .prepare_directed(queue, width, height, cx, cy, cw, ch, ccorner, ax, ay);
        }
        }
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
    ///
    /// **THE ONE SWITCH POINT (routing intent, named for the future
    /// capabilities-as-data refactor):** `selection_rects()` (-> `range_rects`)
    /// is the SOLE geometry builder for a selection — every source (per-glyph
    /// runs within a line, full-width bands for a multi-line selection's
    /// middle lines, an empty-line's own stub, the trailing newline pad) is
    /// already folded into the ONE `rects` vector below, BEFORE the `one_bit`
    /// branch ever reads it. That branch is the single place a selection's
    /// geometry is handed to a per-world PAINT MECHANISM (the ordinary
    /// translucent `selection_pipeline` fill, or `selection_invert`'s true
    /// inverse-video) — a plain `if`/`else` today because there are only two
    /// mechanisms, but structured so a later `Theme::selection_style` (or
    /// `Theme::render_caps.selection_style` field, the capabilities-as-data
    /// read) only ever has to change what THIS branch reads, never how the
    /// rects themselves are built. Never duplicate `rects` per-mechanism —
    /// that is exactly the "different builder per bucket" shape that would
    /// let one geometry source quietly diverge (and disappear) on a future
    /// world.
    pub(super) fn prepare_selection_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        // Build the selection highlight rectangles (one per visible line of the
        // region) plus any IME preedit underline. Empty when there is no
        // selection or preedit.
        let mut rects = self.selection_rects();
        rects.extend(self.preedit_rects());
        let one_bit = theme::active().render_caps.selection_style == theme::SelectionStyle::InverseVideo;

        // ORDINARY WORLDS: the translucent fill, unchanged.
        //
        // COPY PULSE: `prepare_pulsed` blends the stored base tint toward a
        // brighter peak by `(1.0 - copy_pulse_settle())` — settled (`1.0`, the
        // permanent value in every headless capture) is a byte-identical
        // short-circuit to the plain `prepare` this replaced, so a default
        // capture and every pre-existing selection render are unaffected.
        //
        // TRUE 1-BIT WORLDS: this pipeline uploads ZERO rects — the
        // `selection_invert` pipeline below takes over document selection
        // entirely (see its field doc) — so `selection_pipeline` draws
        // nothing there, never a stale white fill under the inverted text.
        let settle = self.copy_pulse_settle();
        let fill_rects: &[[f32; 4]] = if one_bit { &[] } else { &rects };
        self.selection_pipeline.prepare_pulsed(
            device,
            queue,
            width,
            height,
            fill_rects,
            copy_pulse_peak_srgba(),
            settle,
        );

        // Search-match highlights (separate instance/color — an ordinary
        // world's translucent fill, or THE ONE WAGTAIL HIGHLIGHT TEXTURE's
        // dither stipple on a one-bit world, per `search_match_rgba_bytes`/
        // `wagtail_dither_density`). Empty when search is closed so no stale
        // highlights linger.
        let mrects = if self.search_active {
            self.search_match_rects()
        } else {
            Vec::new()
        };
        self.match_pipeline
            .prepare(device, queue, width, height, &mrects);

        // TRUE 1-BIT WORLDS ONLY: the true inverse-video selection — see
        // `TextPipeline::selection_invert`'s field doc. Drawn AFTER text in
        // `draw_document_layers`; every other world uploads zero instances
        // here (parked, byte-identical).
        let invert_rects: &[[f32; 4]] = if one_bit { &rects } else { &[] };
        self.selection_invert
            .prepare(device, queue, width, height, invert_rects);
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
    /// at the active world's per-world [`theme::Theme::ornament_scale`] bump over the
    /// body size (the SAME factor `md_line_scale` grows the break ROW by) so a centered
    /// break reads with a touch more presence (quiet; amber stays the caret's). Uploads NO areas
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
        // `theme::Theme::ornament_face`). The list BULLETS (below) now ride the SAME
        // per-world ornament face — keycaps + plain marks are the only symbols still
        // pinned to `SYMBOL_FAMILY`. The ornament faces are Regular/400, so a plain
        // NORMAL weight matches (no `mono_safe_weight` trap).
        let rule_attrs = Attrs::new()
            .family(Family::Name(theme::active().ornament_face))
            .color(muted);
        // The depth-derived list BULLETS shape in the ACTIVE WORLD'S OWN ornament
        // face too (PER-WORLD DATA, `theme::Theme::bullets` — the ornament trio one
        // level down): the geometric worlds' face IS the merged marks face (so their
        // `•`/`◦` are byte-identical to before this round), while the antique/literary
        // serifs draw a hedera / fleuron / manicule from EB Garamond or Junicode.
        // Same muted ink (faint unchanged, never amber). The ornament faces are
        // Regular/400, so a plain NORMAL weight matches.
        let bullet_attrs = Attrs::new()
            .family(Family::Name(theme::active().ornament_face))
            .color(muted);
        let center = Some(glyphon::cosmic_text::Align::Center);

        // The centered section-break glyph is shaped BIGGER than body ink — a calm,
        // present flourish (still muted, never amber). The scale is PER-WORLD (keyed to
        // the ornament's character — `theme::Theme::ornament_scale`), and the break
        // line's ROW was grown by the SAME value (via `md_line_scale`), so shaping the
        // glyph in a line-box of that grown height (`line_height * ornament_scale`)
        // centers it vertically on the tall break row, exactly as a heading glyph
        // centers on its grown row.
        let ornament_scale = theme::active().ornament_scale;
        let orn_line_h = m.line_height * ornament_scale;
        let orn_metrics = GlyphMetrics::new(m.font_size * ornament_scale, orn_line_h);

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
        // per-world glyph (by nesting depth) LEFT-aligned exactly over the concealed
        // raw `-` cell, in the world's ornament face + muted ink. Shaped at
        // `font_size * bullet_scale` — body size for the plain `•`/`◦` worlds
        // (byte-identical), ~half body for a characterful hedera/manicule so it reads
        // as a quiet marker, not a section flourish. The line-box stays the row's FULL
        // `line_height`, so cosmic-text's own `(line_height - glyph_height)/2` centering
        // keeps a scaled-down bullet on the text's optical middle (same centering the
        // body run gets). Each mark carries its own `left` (the marker cell's x).
        let bullet_marks = if self.md_enabled {
            self.bullet_marks()
        } else {
            Vec::new()
        };
        let bullet_scale = theme::active().bullet_scale;
        let bullet_metrics = GlyphMetrics::new(m.font_size * bullet_scale, m.line_height);
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

        // BLOCKQUOTE PULL-QUOTE MARKS: one big DIM opening quotation mark per
        // blockquote block, shaped in the WORLD'S OWN DISPLAY SERIF (`Theme::font`, the
        // pull-quote — NOT the ornament/symbol face) and hung in the LEFT MARGIN so its
        // RIGHT edge hugs the writing column (the same margin/gap the outline + gutter
        // use). Page-mode + WYSIWYG gated inside `quote_marks` (empty otherwise), so a
        // non-page / off / non-md capture adds nothing. The glyph is identical across
        // blocks, so it shapes ONCE; each block just reuses the buffer at its own top.
        let quote_tops = self.quote_marks();
        let quote_faint = theme::faint().to_glyphon();
        let quote_metrics = GlyphMetrics::new(m.font_size * QUOTE_MARK_SCALE, m.line_height);
        let quote_attrs = Attrs::new()
            .family(Family::Name(theme::active().font))
            .color(quote_faint);
        // A box wide enough to hold the scaled glyph; its shaped advance is measured
        // below so the mark hangs a hair shy of the text's left edge.
        let quote_box_w = (m.font_size * QUOTE_MARK_SCALE * 2.0).max(1.0);
        let mut quote_buffer = GlyphBuffer::new(&mut self.font_system, quote_metrics);
        let mut quote_left = 0.0f32;
        if !quote_tops.is_empty() {
            quote_buffer.set_size(&mut self.font_system, Some(quote_box_w), Some(m.line_height));
            quote_buffer.set_text(
                &mut self.font_system,
                &QUOTE_MARK_GLYPH.to_string(),
                &quote_attrs,
                Shaping::Advanced,
                None,
            );
            quote_buffer.shape_until_scroll(&mut self.font_system, false);
            // DROP-CAP: hang the mark INSIDE the writing column, in the block's own
            // leading indent (the page column's left text-pad gutter), NOT in the
            // left margin — the margin belongs to the OUTLINE alone. Its RIGHT edge
            // sits a hair shy of `text_left` (the quote text's own left edge), so the
            // big opening mark reads as the paragraph's drop-cap and the quote text
            // clears it; its LEFT edge is clamped to `column_left` so it can never
            // spill back out past the page edge into the outline's margin.
            let mut mark_w = 0.0f32;
            for run in quote_buffer.layout_runs() {
                mark_w = mark_w.max(run.line_w);
            }
            let gap = m.char_width * 0.3;
            quote_left = super::geometry::pull_quote_left(
                self.column_left(),
                self.text_left(),
                gap,
                mark_w,
            );
        }

        // DIFF-AS-PREVIEW: ornament glyphs are document content — clip to the
        // panel band exactly like the text layer.
        let (o_top, o_bottom) = match self.doc_clip_band(height as f32) {
            Some((t, b)) => (t as i32, b as i32),
            None => (0, height as i32),
        };
        let bounds = TextBounds { left: 0, top: o_top, right: width as i32, bottom: o_bottom };
        let mut areas: Vec<TextArea> =
            Vec::with_capacity(rule_marks.len() + bullet_marks.len() + quote_tops.len());
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
        // The one shaped pull-quote buffer, reused at each block's first-line top,
        // hung at the column-hugging left computed above.
        for top in &quote_tops {
            areas.push(TextArea {
                buffer: &quote_buffer,
                left: quote_left,
                top: *top,
                scale: 1.0,
                bounds,
                default_color: quote_faint,
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

    /// Shape ONE table's cells as inline markdown, size its columns to fit `avail`,
    /// and — WRAP-NOT-CLIP — reshape each cell at its FINAL column width so an
    /// over-wide table wraps within its columns instead of hard-clipping. Returns
    /// the per-column x-offsets + widths ([`crate::markdown::table_column_layout`]),
    /// the reshaped per-cell buffers (each with its wrapped pixel width, for
    /// alignment), and each GRID ROW's height (the max wrapped-cell height across
    /// the row, never below one line-height). PURE font work — no placement, no
    /// culling — so the row-height RESERVATION ([`Self::compute_table_layout`],
    /// which reserves the tall document row so nothing overlaps) and the DRAW pass
    /// ([`Self::prepare_table_grid`]) share ONE layout and can never disagree on how
    /// tall a wrapped row is. A cell with no markup yields `cell_attrs` alone —
    /// byte-identical shaping to raw text; a table that already fits stays
    /// single-line (every row height == one line-height).
    fn shape_table_grid(
        &mut self,
        grid_rows: &[(usize, Vec<String>)],
        ncols: usize,
        avail: f32,
        gap: f32,
        pad: f32,
    ) -> TableGridShaped {
        let m = self.metrics;
        let content = theme::base_content().to_glyphon();
        let cell_attrs = self.doc_attrs().color(content);
        let body_metrics = GlyphMetrics::new(m.font_size, m.line_height);
        // PASS 1 — shape each non-empty cell to measure BOTH its MAX-content width
        // (the whole cell on one line, at the full writing column) and its
        // MIN-content floor (the widest UNBREAKABLE word, measured by re-wrapping the
        // SAME styled buffer at a 1px width under `Wrap::Word` so words never break).
        // The two feed the CSS auto-table allocation (`table_column_layout`), which
        // floors every column to its longest word so a cell NEVER wraps mid-word.
        // The cell is styled as INLINE markdown (real bold / italic / mono code +
        // zero-width markers) via the SAME span seam prose uses
        // (`spans::cell_inline_attrs`), so both measurements are the STYLED text's.
        let mut maxs = vec![0.0f32; ncols];
        let mut mins = vec![0.0f32; ncols];
        let mut cells: Vec<(usize, usize, GlyphBuffer, f32)> = Vec::new();
        for (gr, (_, row_cells)) in grid_rows.iter().enumerate() {
            for (c, cell) in row_cells.iter().enumerate() {
                if c >= ncols || cell.is_empty() {
                    continue;
                }
                // A cell can't wrap to more visual rows than it has characters, so a
                // per-cell shaping-height of `(chars + 1) * line_height` always lays
                // out every wrapped row into `layout_runs()` (a too-small height would
                // clamp the run iterator and truncate the measurement).
                let tall = m.line_height * (cell.chars().count() as f32 + 1.0);
                let mut buf = GlyphBuffer::new(&mut self.font_system, body_metrics);
                // WORD wrap (not the default WordOrGlyph): a word is never split, so
                // the min-content floor is a true word floor and pass-2 wrapping honors
                // it — the "no cell wraps mid-word" law is structural, not just floored.
                buf.set_wrap(&mut self.font_system, Wrap::Word);
                buf.set_size(&mut self.font_system, Some(avail), Some(tall));
                buf.set_text(&mut self.font_system, cell, &cell_attrs, Shaping::Advanced, None);
                let al = cell_inline_attrs(&cell_attrs, m.line_height, cell);
                if let Some(line) = buf.lines.get_mut(0) {
                    line.set_attrs_list(al);
                }
                buf.shape_until_scroll(&mut self.font_system, false);
                let mut w = 0.0f32;
                for run in buf.layout_runs() {
                    w = w.max(run.line_w);
                }
                // MIN-content: re-wrap the same buffer at a 1px width so every word
                // lands on its own line (Word wrap), then the widest run IS the widest
                // unbreakable word. Restore for pass 2 (which reshapes at box width).
                buf.set_size(&mut self.font_system, Some(1.0), Some(tall));
                buf.shape_until_scroll(&mut self.font_system, false);
                let mut word = 0.0f32;
                for run in buf.layout_runs() {
                    word = word.max(run.line_w);
                }
                maxs[c] = maxs[c].max(w + 2.0 * pad);
                mins[c] = mins[c].max(word + 2.0 * pad);
                cells.push((gr, c, buf, w));
            }
        }
        // A column of only-empty cells still occupies its padding.
        for c in 0..ncols {
            if maxs[c] <= 0.0 {
                maxs[c] = 2.0 * pad;
            }
            mins[c] = mins[c].max(2.0 * pad);
        }
        let (col_x, col_w) = crate::markdown::table_column_layout(&mins, &maxs, gap, avail);
        // PASS 2 — reshape each cell at its OWN column's inner width so it WRAPS
        // inside the column (never clips at the edge, never overruns the neighbour);
        // recompute its wrapped width (for alignment) and count its wrapped rows to
        // grow the row. A cell whose column is at least its natural width lays out on
        // one line (rows == 1) — a table that fits is unchanged.
        let mut row_heights = vec![m.line_height; grid_rows.len()];
        for (gr, c, buf, w) in cells.iter_mut() {
            let box_w = col_w.get(*c).copied().unwrap_or(avail);
            let wrap_w = (box_w - 2.0 * pad).max(1.0);
            buf.set_size(&mut self.font_system, Some(wrap_w), buf.size().1);
            buf.shape_until_scroll(&mut self.font_system, false);
            let mut mw = 0.0f32;
            let mut rows = 0usize;
            for run in buf.layout_runs() {
                mw = mw.max(run.line_w);
                rows += 1;
            }
            *w = mw;
            let h = rows.max(1) as f32 * m.line_height;
            if let Some(rh) = row_heights.get_mut(*gr) {
                *rh = rh.max(h);
            }
        }
        TableGridShaped { col_x, col_w, cells, row_heights }
    }

    /// WRAP-NOT-CLIP row RESERVATION: the per-LOGICAL-LINE reserved row height each
    /// off-cursor GFM table's rows need so a WRAPPED (too-wide) table grows its
    /// document rows instead of the drawn grid overlapping the following content —
    /// the SAME "reserve a tall row" contract inline images use
    /// (`compute_image_layout`), stored in the shared `image_heights` slot and
    /// threaded into [`build_line_attrs`]. Computed at reshape time (O(doc tables),
    /// not per-frame) directly from the fresh `text` + `md_spans`, so it is ready
    /// before the line attrs are built. `None` for every non-table line and for a
    /// table row that fits on one line (no reservation → byte-identical layout);
    /// an all-`None` vector when WYSIWYG is off / not markdown, so a plain doc is
    /// untouched.
    pub(super) fn compute_table_layout(
        &mut self,
        text: &str,
        md_spans: &[(std::ops::Range<usize>, crate::markdown::MdKind)],
    ) -> Vec<Option<f32>> {
        let lines: Vec<&str> = text.split('\n').collect();
        let mut heights = vec![None; lines.len().max(1)];
        if !(crate::markdown::wysiwyg_on() && self.md_enabled) || md_spans.is_empty() {
            // Nothing shaped this reshape (WYSIWYG off / no markdown / no table
            // spans) — the cache must not keep serving a PRIOR reshape's stale
            // grids (`prepare_table_grid` never reads it in this state anyway,
            // since it shares the same gate, but an empty cache is the honest
            // shape of "no tables were shaped here").
            self.table_grid_cache.entries.borrow_mut().clear();
            self.table_grid_cache.version.set(None);
            return heights;
        }
        let m = self.metrics;
        let avail = self.text_wrap_width().max(1.0);
        let pad = TABLE_CELL_PAD_X * m.zoom;
        let gap = TABLE_COL_GAP * m.zoom;
        // Per-line first byte offset, so a table conceal span's `start` maps to its
        // header line and its `end` bounds the block's lines.
        let mut starts = Vec::with_capacity(lines.len());
        let mut acc = 0usize;
        for l in &lines {
            starts.push(acc);
            acc += l.len() + 1;
        }
        // PHASE A (pure) — parse each table block's grid rows from the source lines,
        // collected first so the shaping loop below can take `&mut self`.
        struct TMeta {
            range: std::ops::Range<usize>,
            grid_rows: Vec<(usize, Vec<String>)>,
            ncols: usize,
        }
        let mut tmetas: Vec<TMeta> = Vec::new();
        for (r, k) in md_spans {
            if *k
                != crate::markdown::MdKind::ConcealMarkup(crate::markdown::ConcealKind::Table)
            {
                continue;
            }
            let Some(header_line) = starts.iter().position(|&s| s == r.start) else {
                continue;
            };
            let mut src: Vec<(usize, &str)> = Vec::new();
            let mut li = header_line;
            while li < lines.len() && starts[li] < r.end {
                src.push((li, lines[li]));
                li += 1;
            }
            if src.len() < 2 {
                continue;
            }
            let align_cells = crate::markdown::split_row_cells(src[1].1);
            let mut grid_rows: Vec<(usize, Vec<String>)> = Vec::new();
            for (i, (dl, line)) in src.iter().enumerate() {
                if i == 1 {
                    continue; // the separator row is the rule, no cells
                }
                if i >= 2 && line.trim().is_empty() {
                    continue;
                }
                grid_rows.push((*dl, crate::markdown::split_row_cells(line)));
            }
            let ncols = grid_rows
                .iter()
                .map(|(_, c)| c.len())
                .max()
                .unwrap_or(0)
                .max(align_cells.len());
            tmetas.push(TMeta { range: r.clone(), grid_rows, ncols });
        }
        // PHASE B — shape each table ONCE (the ONE shape site, see
        // [`TableGridCache`]'s own doc comment) and reserve the tall row on any
        // grid row that wrapped past one line-height, THEN keep the shaped result
        // in `table_grid_cache` so the draw pass (`prepare_table_grid`) reads the
        // SAME geometry instead of re-shaping — a reservation/draw divergence is
        // now structurally unrepresentable.
        let mut cache_entries: Vec<(usize, TableGridShaped)> = Vec::new();
        for tm in tmetas {
            if tm.ncols == 0 {
                continue;
            }
            let shaped = self.shape_table_grid(&tm.grid_rows, tm.ncols, avail, gap, pad);
            for (gr, (dl, _)) in tm.grid_rows.iter().enumerate() {
                let h = shaped.row_heights[gr];
                if h > m.line_height + 0.5 {
                    if let Some(slot) = heights.get_mut(*dl) {
                        *slot = Some(h);
                    }
                }
            }
            cache_entries.push((tm.range.start, shaped));
        }
        *self.table_grid_cache.entries.borrow_mut() = cache_entries;
        self.table_grid_cache.version.set(Some(self.reshape_count));
        heights
    }

    /// TEST-ONLY: the CACHED row heights for the table block whose byte range
    /// starts at `range_start` — a direct peek at the ONE shape site
    /// ([`TableGridCache`]) so a test can compare what `compute_table_layout`
    /// reserved against what `prepare_table_grid` actually reads to draw,
    /// without needing a GPU pixel diff. `None` if no table was cached at that
    /// range (off / no such table).
    #[cfg(test)]
    pub(super) fn table_grid_cache_row_heights(&self, range_start: usize) -> Option<Vec<f32>> {
        self.table_grid_cache
            .entries
            .borrow()
            .iter()
            .find(|(s, _)| *s == range_start)
            .map(|(_, g)| g.row_heights.clone())
    }

    /// TEST-ONLY: the CACHED per-column x-offsets + widths for the table block
    /// whose byte range starts at `range_start` — a direct peek at
    /// [`TableGridCache`] so a test can assert every cell's x-range stays
    /// within the writing column WITHOUT a GPU pixel diff. `None` if no table
    /// is cached at that range.
    #[cfg(test)]
    pub(super) fn table_grid_cache_col_geometry(
        &self,
        range_start: usize,
    ) -> Option<(Vec<f32>, Vec<f32>)> {
        self.table_grid_cache
            .entries
            .borrow()
            .iter()
            .find(|(s, _)| *s == range_start)
            .map(|(_, g)| (g.col_x.clone(), g.col_w.clone()))
    }

    /// TEST-ONLY: every table cell's document line the LAST [`Self::prepare_table_grid`]
    /// call actually uploaded as a `TextArea` (see `last_table_cell_lines`'s own
    /// field doc). A doc line absent from this list drew NO grid cells that frame.
    #[cfg(test)]
    pub(super) fn table_cell_lines_drawn(&self) -> Vec<usize> {
        self.last_table_cell_lines.borrow().clone()
    }

    /// THE X-RAY (the user's canonized metaphor): when the caret sits on a GFM
    /// table ROW, stash that row's RAW SOURCE shaped as ONE NON-WRAPPING line
    /// ([`crate::render::XrayRow`]) so (a) the caret's own `col_x_and_advance`
    /// redirects onto it (the concealed doc row is zero-width — see the redirect in
    /// `geometry.rs`), (b) `caret_band_scale` sizes the caret to the source band,
    /// and (c) `prepare_table_grid` floats it, centered, over the row band its OWN
    /// grid cells were skipped for (the true source SWAP — see that function's own
    /// doc comment), panning to keep the caret visible. Run BEFORE
    /// [`Self::prepare_caret_layer`] so the
    /// redirect is ready when the caret geometry is computed. Clears the stash
    /// first (a caret NOT on a table row heals the row), carrying the previous
    /// frame's pan for the same row so a walk along it doesn't jitter. Gated on
    /// WYSIWYG + markdown; `None` for every other frame (byte-identical capture).
    pub(super) fn prepare_table_xray(&mut self) {
        // Carry the previous pan for the SAME row (stable walk), then reset.
        let prev_pan = self
            .xray
            .as_ref()
            .filter(|x| x.line == self.cursor_line)
            .map(|x| x.pan)
            .unwrap_or(0.0);
        self.xray = None;
        if !(crate::markdown::wysiwyg_on() && self.md_enabled) {
            return;
        }
        let line = self.cursor_line;
        let line_byte = self.line_doc_byte_start(line);
        let in_table = self
            .table_blocks()
            .into_iter()
            .any(|(_, r)| r.start <= line_byte && line_byte < r.end);
        if !in_table {
            return;
        }
        let Some(src) = self.buffer.lines.get(line).map(|l| l.text().to_string()) else {
            return;
        };
        // Shape the RAW source NON-WRAPPING (Wrap::None) — one line that pans, so
        // the row NEVER grows (the whole point of the x-ray).
        let m = self.metrics;
        let body = GlyphMetrics::new(m.font_size, m.line_height);
        let base = self.doc_attrs().color(theme::base_content().to_glyphon());
        let mut buf = GlyphBuffer::new(&mut self.font_system, body);
        buf.set_wrap(&mut self.font_system, Wrap::None);
        buf.set_size(&mut self.font_system, None, Some(m.line_height * 2.0));
        buf.set_text(&mut self.font_system, &src, &base, Shaping::Advanced, None);
        buf.shape_until_scroll(&mut self.font_system, false);
        let mut clusters: Vec<(usize, usize, f32, f32)> = Vec::new();
        for run in buf.layout_runs() {
            for g in run.glyphs.iter() {
                clusters.push((g.start, g.end, g.x, g.x + g.w));
            }
        }
        let glyph_xs = super::geometry::assemble_glyph_xs(&src, &clusters, m.char_width);
        let content_w = glyph_xs.last().copied().unwrap_or(0.0);
        let view_w = self.text_wrap_width().max(1.0);
        let pad = crate::render::TABLE_CELL_PAD_X * m.zoom;
        let cc = self.cursor_col.min(glyph_xs.len().saturating_sub(1));
        let caret_x = glyph_xs.get(cc).copied().unwrap_or(0.0);
        let pan = super::geometry::xray_pan_for_caret(caret_x, content_w, view_w, pad, prev_pan);
        let top = self.line_ornament_top(line);
        let height = self.cursor_row_height();
        self.xray = Some(crate::render::XrayRow { line, source: src, glyph_xs, top, height, pan });
    }

    /// WYSIWYG TABLE GRID: place every off-cursor GFM table's cells by PIXEL column
    /// (a proportional face can't align with space-padding — that's the bug this
    /// fixes) via one [`TextArea`] per cell, plus ONE faint header-separator rule.
    /// The [`Self::prepare_ornaments`] pattern applied to a rectangular block: the
    /// source rows are concealed to zero-width by
    /// [`crate::markdown::ConcealKind::Table`], and the grid draws in their place.
    /// A table that FITS occupies one row per source line (header, the rule row,
    /// then body); a too-wide table WRAPS its cells and each grown row reserves a
    /// tall document row via [`Self::compute_table_layout`], so grid and source
    /// agree on the row geometry (`RowGeom` reads the reserved heights).
    ///
    /// REVEAL = TRUE SOURCE SWAP, per row (WYSIWYG amendment, corrected): a table
    /// the caret is INSIDE stays a drawn grid — every row EXCEPT the caret's own
    /// still uploads its cells at full ink. Only the ONE row the caret currently
    /// occupies uploads NO grid cells at all; its raw source floats over that
    /// row's band instead (pushed after the cell loop below, see the x-ray). This
    /// is the drop-to-source-on-cursor contract applied per row rather than
    /// parking the whole block — grid and source never share a row's pixels, but
    /// they DO still share the table (a multi-row table mid-edit reads as "grid,
    /// with one row temporarily as text", not "the whole table vanished"). Also
    /// drawn (all rows) for a non-markdown / table-less buffer trivially (there
    /// are none) and skipped wholesale with WYSIWYG off, so a default capture
    /// stays byte-identical.
    ///
    /// Cost: O(visible tables' cells) to UPLOAD. The SHAPING itself is done ONCE
    /// per reshape by [`Self::compute_table_layout`] (the ONE shape site, see
    /// [`TableGridCache`]) — this pass only ever READS that cached geometry
    /// (column widths are the max over every row, so a partly-scrolled table
    /// keeps STABLE columns rather than jumping) and places it; off-screen tables
    /// are culled whole (their cached geometry is simply never turned into
    /// `TextArea`s). Column math ([`crate::markdown::table_column_layout`] /
    /// [`crate::markdown::table_align_offset`]) is pure + unit-tested and already
    /// baked into the cached geometry.
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
        #[cfg(test)]
        self.last_table_cell_lines.borrow_mut().clear();

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
        // No `gap` here: the per-column layout (which bakes the gap in) is READ from
        // `table_grid_cache` (the one shape site), never recomputed at draw time.
        let rule_thick = (TABLE_RULE_THICKNESS * m.zoom).max(1.0);
        let cursor_byte = self.line_doc_byte_start(self.cursor_line);
        let content = theme::base_content().to_glyphon();

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

        // PHASE B — READ (never reshape) the geometry `compute_table_layout` already
        // shaped for VISIBLE blocks, from the ONE shape site
        // ([`TableGridCache`] — see its doc comment for why the draw pass must not
        // shape its own copy). A `None` here means the block is off-screen (culled,
        // matching the pre-existing "shape nothing off-screen" behavior — its report
        // carries no measured widths) or, degenerately, that no cache entry exists
        // for this range (would mean `compute_table_layout` and this frame's own
        // `table_blocks()` disagreed on the table list, which they cannot: both
        // derive from the SAME `self.md_spans` field).
        let table_cache = self.table_grid_cache.entries.borrow();
        let shaped: Vec<Option<&TableGridShaped>> = metas
            .iter()
            .map(|meta| {
                if !meta.visible || meta.ncols == 0 {
                    return None;
                }
                table_cache
                    .iter()
                    .find(|(start, _)| *start == meta.range.0)
                    .map(|(_, s)| s)
            })
            .collect();

        // PHASE C — place the reshaped cells + the header rule, fill the report. The
        // column layout was already computed inside `shape_table_grid`. A too-wide
        // grid grows into the right margin at pan 0 and, once the live gesture pans
        // it, shifts LEFT by `pan` and clips to the writing column (`table_pan_*`).
        let view_w = avail;
        let pan_bar_thick = (crate::render::TABLE_PAN_BAR_THICKNESS * m.zoom).max(1.0);
        let muted = theme::muted().to_glyphon();
        // THE X-RAY float: the caret's table-row RAW SOURCE shaped NON-WRAPPING into
        // a LOCAL buffer (so `areas` can borrow it below without fighting the
        // renderer's own `&mut self` borrows). Drawn dim ("the markdown bones") over
        // the dimmed grid cells, panned by `x.pan` to keep the caret column visible.
        let mut xray_float: Option<(GlyphBuffer, f32, f32, f32, usize)> = None;
        if let Some(x) = self.xray.clone() {
            let bodym = GlyphMetrics::new(m.font_size, m.line_height);
            let base = self.doc_attrs().color(muted);
            let mut buf = GlyphBuffer::new(&mut self.font_system, bodym);
            buf.set_wrap(&mut self.font_system, Wrap::None);
            buf.set_size(&mut self.font_system, None, Some(m.line_height * 2.0));
            buf.set_text(&mut self.font_system, &x.source, &base, Shaping::Advanced, None);
            buf.shape_until_scroll(&mut self.font_system, false);
            xray_float = Some((buf, x.top, x.height, x.pan, x.line));
        }
        let xray_line = xray_float.as_ref().map(|f| f.4);
        let mut areas: Vec<TextArea> = Vec::new();
        let mut rule_rects: Vec<[f32; 4]> = Vec::new();
        let mut pan_writeback: Option<(usize, f32)> = None;
        for (mi, meta) in metas.iter().enumerate() {
            let (col_x, col_w) = match &shaped[mi] {
                Some(s) => (s.col_x.as_slice(), s.col_w.as_slice()),
                None => (&[][..], &[][..]),
            };
            self.table_report.borrow_mut().push(crate::render::TableReport {
                range: meta.range,
                rows: meta.grid_rows.len(),
                cols: meta.ncols,
                col_widths: col_w.to_vec(),
                revealed: meta.revealed,
            });
            let Some(s) = &shaped[mi] else {
                continue;
            };
            // THE X-RAY table (the caret is inside): the grid stays DRAWN (the
            // document never reflowed — the source rows are still concealed), the
            // caret's own row is DIMMED, and its raw source floats over it (pushed
            // after the loop). No reading-pan applies while editing — the float owns
            // the horizontal.
            if meta.revealed {
                let content_w = col_x
                    .last()
                    .zip(col_w.last())
                    .map(|(x, w)| x + w)
                    .unwrap_or(0.0);
                // TRUE SWAP, not dim-under-float (the fix): the caret's OWN row
                // uploads NO grid cells at all — the x-ray source float (pushed
                // after this loop, centered in the row's own band) is the ONLY
                // text drawn in that band, per the drop-to-source-on-cursor
                // contract. Every OTHER row of a revealed table still draws its
                // grid at full ink — only the one row the caret occupies drops
                // to source; the block never "parks" wholesale.
                for (gr, c, buf, cw) in &s.cells {
                    let doc_line = meta.grid_rows[*gr].0;
                    if Some(doc_line) == xray_line {
                        continue;
                    }
                    #[cfg(test)]
                    self.last_table_cell_lines.borrow_mut().push(doc_line);
                    let top = self.line_ornament_top(doc_line);
                    let box_left = text_left + col_x[*c];
                    let box_w = col_w[*c];
                    let off =
                        crate::markdown::table_align_offset(meta.aligns[*c], box_w, *cw, pad);
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
                let sep_top = self.line_ornament_top(meta.sep_doc_line);
                let rule_y = sep_top + (m.line_height - rule_thick) * 0.5;
                if content_w > 0.0 {
                    rule_rects.push([text_left, rule_y, content_w, rule_thick]);
                }
                continue;
            }
            // The laid grid width + this table's clamped horizontal pan. A grid that
            // fits (`content_w ≤ view_w`) never pans (`table_pan_clamp → 0`).
            let content_w = col_x
                .last()
                .zip(col_w.last())
                .map(|(x, w)| x + w)
                .unwrap_or(0.0);
            let pan_req = self
                .table_pan
                .filter(|(start, _)| *start == meta.range.0)
                .map(|(_, o)| o)
                .unwrap_or(0.0);
            let pan = crate::markdown::table_pan_clamp(pan_req, content_w, view_w);
            if self.table_pan.is_some_and(|(start, _)| start == meta.range.0) {
                pan_writeback = Some((meta.range.0, pan));
            }
            // At pan 0 the grid grows into the margins (clip only at the canvas);
            // once panned, clip to the writing column so shifted content never
            // spills into the LEFT margin.
            let (vp_l, vp_r) = if pan > 0.0 {
                (text_left, text_left + view_w)
            } else {
                (0.0, width as f32)
            };
            for (gr, c, buf, cw) in &s.cells {
                let doc_line = meta.grid_rows[*gr].0;
                #[cfg(test)]
                self.last_table_cell_lines.borrow_mut().push(doc_line);
                let top = self.line_ornament_top(doc_line);
                let box_left = text_left + col_x[*c] - pan;
                let box_w = col_w[*c];
                let off = crate::markdown::table_align_offset(meta.aligns[*c], box_w, *cw, pad);
                // Each cell WRAPS within its column (shaped at the column's inner
                // width), so it never overruns its neighbour; the clip is the
                // column box intersected with the table viewport (a safety net for
                // an unbreakable over-wide token, and the pan's left-spill guard).
                let clip_left = box_left.max(vp_l).max(0.0) as i32;
                let clip_right = (box_left + box_w).min(vp_r).clamp(0.0, width as f32) as i32;
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
            // centered in the separator row's band. Spans the laid grid width at pan
            // 0 (growing into the margin), else the visible portion within the view.
            let sep_top = self.line_ornament_top(meta.sep_doc_line);
            let rule_y = sep_top + (m.line_height - rule_thick) * 0.5;
            let rule_w = if pan > 0.0 {
                (content_w - pan).min(view_w).max(0.0)
            } else {
                content_w
            };
            if rule_w > 0.0 {
                rule_rects.push([text_left, rule_y, rule_w, rule_thick]);
            }
            // The THIN transient pan-indicator bar at the table's bottom edge, while
            // this table is panned (`pan > 0`). Value-step tint (the rule pipeline's
            // own faint colour), never amber. A default capture never sets a pan, so
            // no bar draws — byte-identical. The hover/idle FADE is live-only.
            if pan > 0.0 {
                if let Some((last_dl, _)) = meta.grid_rows.last() {
                    let last_gr = s.row_heights.len().saturating_sub(1);
                    let bottom = self.line_ornament_top(*last_dl) + s.row_heights[last_gr];
                    if let Some(bar) = crate::markdown::table_pan_bar(
                        content_w,
                        view_w,
                        pan,
                        text_left,
                        bottom,
                        pan_bar_thick,
                    ) {
                        rule_rects.push(bar);
                    }
                }
            }
        }
        // THE X-RAY FLOAT — drawn LAST so it composites over the dimmed grid cells:
        // the caret row's raw source as one non-wrapping line, panned by `pan` to
        // keep the caret visible, centred in its row band, clipped to the writing
        // column (so a long source doesn't spill into the margins).
        if let Some((buf, top, row_h, pan, _line)) = xray_float.as_ref() {
            let float_top = top + (row_h - m.line_height) * 0.5;
            areas.push(TextArea {
                buffer: buf,
                left: text_left - pan,
                top: float_top,
                scale: 1.0,
                bounds: TextBounds {
                    left: text_left.max(0.0) as i32,
                    top: 0,
                    right: (text_left + view_w).clamp(0.0, width as f32) as i32,
                    bottom: height as i32,
                },
                default_color: muted,
                custom_glyphs: &[],
            });
        }
        // Persist the clamped pan so a stale offset self-corrects once the grid
        // narrows (a theme reshape / measure change), and the live gesture reads a
        // sane base next frame.
        if let Some(wb) = pan_writeback {
            self.table_pan = Some(wb);
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

    /// LIVE horizontal table PAN (the reading gesture). If `(px, py)` is over an
    /// OVERFLOWING table (its laid grid is wider than the writing column), nudge
    /// that table's pan by `dx` px (a horizontal wheel notch) and return `true` so
    /// the caller CONSUMES the scroll (the document does not also scroll). Returns
    /// `false` — fall through to normal vertical scroll — when no pannable table is
    /// under the pointer. The `content_w`/`view_w` clamp reuses the LAST frame's
    /// `tables_report` widths (a table under the pointer has necessarily been laid
    /// this session), so no reshape is needed on the hot wheel path. LIVE-ONLY:
    /// never reached by the headless capture (which has no pointer / wheel).
    pub fn try_table_pan(&mut self, px: f32, py: f32, scroll: usize, dx: f32) -> bool {
        if !(crate::markdown::wysiwyg_on() && self.md_enabled) {
            return false;
        }
        let (line, _) = self.hit_test(px, py, scroll);
        // Which table block (if any) owns the hit line?
        let line_byte = self.line_doc_byte_start(line);
        let Some((start, _range)) = self
            .table_blocks()
            .into_iter()
            .find(|(_, r)| r.start <= line_byte && line_byte < r.end)
            .map(|(_, r)| (r.start, r))
        else {
            return false;
        };
        // The laid grid width from the last report (widths + gaps), vs the view.
        let report = self.table_report.borrow();
        let Some(t) = report.iter().find(|t| t.range.0 == start) else {
            return false;
        };
        let n = t.col_widths.len();
        if n == 0 {
            return false;
        }
        let gap = crate::render::TABLE_COL_GAP * self.metrics.zoom;
        let content_w: f32 = t.col_widths.iter().sum::<f32>() + gap * (n.saturating_sub(1) as f32);
        drop(report);
        let view_w = self.text_wrap_width().max(1.0);
        if content_w <= view_w + 1e-3 {
            return false; // fits — nothing to pan
        }
        let cur = self
            .table_pan
            .filter(|(s, _)| *s == start)
            .map(|(_, o)| o)
            .unwrap_or(0.0);
        // A rightward swipe (dx < 0 by the natural-scroll convention) reveals the
        // right of the grid: pan offset += -dx. TASTE/tunable, flagged live-only.
        let next = crate::markdown::table_pan_clamp(cur - dx, content_w, view_w);
        self.table_pan = Some((start, next));
        true
    }

    /// INLINE-IMAGE CAPTION SCRIM: append one soft ground-colour band per visual row
    /// of the revealed image line `li` behind that row's shaped source text, so the
    /// centred caption reads over arbitrary image pixels. Each band hugs the row's
    /// own text extent (`xs[start_col]..xs[end_col]` + [`CAPTION_SCRIM_PAD_X`]) at a
    /// one-text-line height (+ [`CAPTION_SCRIM_PAD_Y`]) centred where cosmic-text
    /// centres the source within the `h`-tall row. Because the scrim is the WORLD'S
    /// OWN GROUND (`base_100`, part-alpha), it is INVISIBLE where the caption sits
    /// clear of the image (ground-over-ground) and only lifts value where the text
    /// actually overlaps the dimmed image — no stray band artifact. A glyphless / zero-
    /// width row contributes nothing.
    #[cfg(not(target_arch = "wasm32"))]
    fn push_caption_scrim(&self, li: usize, out: &mut Vec<[f32; 4]>) {
        let zoom = self.metrics.zoom;
        let pad_x = CAPTION_SCRIM_PAD_X * zoom;
        let pad_y = CAPTION_SCRIM_PAD_Y * zoom;
        let text_left = self.text_left();
        let doc_top = self.doc_top();
        // The caption's own text-line band height (one body line), NOT the tall row
        // `h` — the source glyphs are body-size, centred within `h`.
        let band_h = self.metrics.line_height;
        for vr in self.visual_rows(li) {
            let (Some(&x0), Some(&x1)) = (vr.xs.get(vr.start_col), vr.xs.get(vr.end_col)) else {
                continue;
            };
            if x1 - x0 <= 0.5 {
                continue; // glyphless / fully-concealed row: nothing to back
            }
            let center_y = doc_top + vr.line_top + vr.line_height * 0.5;
            out.push([
                text_left + x0 - pad_x,
                center_y - band_h * 0.5 - pad_y,
                (x1 - x0) + pad_x * 2.0,
                band_h + pad_y * 2.0,
            ]);
        }
    }

    /// The deterministic per-image layout the last [`Self::rebuild_image_rows`]
    /// (via `compute_image_layout`) produced, for the capture `images` sidecar
    /// block + the next-phase draw. `revealed` is recomputed here against the
    /// CURRENT caret line (a pure caret move re-lays the image line's conceal but
    /// does not re-read image headers), so it never goes stale. Empty when inline
    /// images are off / non-markdown / on wasm.
    /// INLINE IMAGES — the GPU draw. Decodes each visible image (O(visible):
    /// off-SCREEN images are culled), uploads it via the
    /// [`image_cache`](crate::render::image_cache) (downscaled to the display
    /// width), and builds one textured quad per image (fit-to-column, centered in
    /// the reserved tall row `compute_image_layout` produced). A REVEALED image
    /// (the caret is on its source line) is still drawn — DIMMED and UNMOVED (the
    /// caption model: the source reveals centred OVER it, over a soft scrim band) —
    /// not culled. Plus a calm rounded
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
        // cache key to fetch the view in pass B, + the per-image opacity); `missing`
        // holds the placeholder placements (dst rect + filename + alt). Only an
        // OFF-SCREEN image is culled (its row is clipped to nothing anyway).
        //
        // CAPTION-STYLE REVEAL (re-decided 2026-07-09): an image on the caret's line
        // is still DRAWN — DIMMED (`IMAGE_REVEAL_DIM_ALPHA`) but UNMOVED, filling its
        // own `dh`-tall row from the row top exactly as off-cursor. The revealed
        // `![alt](path)` source shapes at body size and cosmic-text centres it
        // VERTICALLY within that same row (`build_line_attrs` keeps the row at `h` —
        // no grow, no reflow), so the source reads as a CAPTION over the dimmed
        // image; a scrim band (`scrim_bands` below) lifts its legibility. Off-cursor
        // the image draws at full opacity.
        // With WYSIWYG off there is no reveal model: a revealed (caret-on-line)
        // image PARKS exactly as before (its source shows unconcealed in the h-tall
        // row), keeping the wysiwyg=false off state byte-identical.
        let wysiwyg = crate::markdown::wysiwyg_on();
        struct Ready {
            dst: [f32; 4],
            alpha: f32,
            key: std::path::PathBuf,
        }
        struct Missing {
            dst: [f32; 4],
            path: String,
            alt: String,
        }
        let mut ready: Vec<Ready> = Vec::new();
        let mut missing: Vec<Missing> = Vec::new();
        // CAPTION scrim bands: one soft ground-colour quad behind the revealed
        // source of each revealed image, so the caption reads over arbitrary image
        // pixels (built below, uploaded to `image_scrim_pipeline`). Empty unless a
        // real image line is revealed with WYSIWYG on.
        let mut scrim_bands: Vec<[f32; 4]> = Vec::new();
        for im in &report {
            if !self.line_ornament_visible(im.line) || (im.revealed && !wysiwyg) {
                continue;
            }
            let dw = im.display_w.max(1.0);
            let dh = im.display_h.max(1.0);
            let row_top = self.line_ornament_top(im.line);
            // Revealed: the image stays at the row top, DIMMED, and the source
            // reveals CENTRED over it (the caption model). Off-cursor / missing: full
            // opacity, source concealed. A missing placeholder never dims.
            let alpha = if im.revealed && !im.missing {
                IMAGE_REVEAL_DIM_ALPHA
            } else {
                1.0
            };
            // Fit-to-column: centered horizontally in the writing column; the row
            // reserves `dh` of height, so the quad fills it vertically.
            let left = text_left + (wrap - dw).max(0.0) * 0.5;
            let dst = [left, row_top, dw, dh];
            // Build the caption scrim behind the revealed source (a real, readable
            // image line only — a missing placeholder shows its own card).
            if im.revealed && !im.missing && wysiwyg {
                self.push_caption_scrim(im.line, &mut scrim_bands);
            }
            if im.missing {
                missing.push(Missing { dst, path: im.path.clone(), alt: im.alt.clone() });
                continue;
            }
            let resolved = self.resolve_image_path(&im.path);
            let key = ImageCache::canonical_key(&resolved);
            match self.image_cache.ensure(device, queue, &resolved, dw, max_dim) {
                ImageState::Ready { .. } => ready.push(Ready { dst, alpha, key }),
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
                        alpha: r.alpha,
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

        // The CAPTION SCRIM bands (ground-colour, part-alpha — see
        // `push_caption_scrim`), drawn OVER the dimmed image and UNDER the revealed
        // source, so the caption reads over any image pixels. Empty (parked) unless
        // a real image line is revealed, so a default frame is byte-identical.
        self.image_scrim_pipeline
            .prepare(device, queue, width, height, &scrim_bands);

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
        self.image_scrim_pipeline
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
            // The list-surface bar quads park empty while the search panel is up.
            self.overlay_bars.prepare(device, queue, width, height, &[]);
        } else {
            // NO overlay, NO search: PARK every overlay pipeline empty — the flat
            // card + row-band + lens-underline quads, the amber query caret, AND
            // the overlay TEXT renderer. The last is load-bearing: the frosted-blur
            // path (`render`'s blur branch, taken whenever the HUD is held) draws
            // the overlay card UNCONDITIONALLY, so a text renderer left holding a
            // closed palette's rows would ghost over the HUD frost. See
            // `park_overlay`.
            self.park_overlay(device, queue, width, height)?;
        }
        // The page-mode orientation gutter (bottom-left margin; parks off-screen
        // edge-to-edge or with no buffer name, so a non-page capture stays byte-identical).
        self.prepare_gutter(device, queue, width, height)?;
        // The persistent margin OUTLINE (top-left margin; parks off-screen when off /
        // not page mode / not markdown / heading-free / too narrow, so a default (off)
        // capture stays byte-identical).
        self.prepare_outline(device, queue, width, height)?;
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
        // The ZOOM READOUT (floats at the pointer while a zoom gesture is in flight;
        // live-only content). `None` parks it off-screen, so every default capture
        // stays byte-identical (the `AWL_ZOOM_READOUT` gallery probe is opt-in).
        self.prepare_zoom_readout(device, queue, width, height)?;
        // The SUMMONED-WHILE-HELD stats HUD: a dim scrim + centered stacked stats,
        // drawn only while held (`crate::hud::hud_held`); released, the scrim is empty
        // and the text is parked off-screen, so a default capture stays byte-identical.
        self.prepare_hud(device, queue, width, height)?;
        // The SUMMONED WHICH-KEY panel: a bottom-left hint card listing a pending
        // prefix's follow-up keys. Drawn only while summoned (the App set its rows on a
        // prefix pause); parked off-screen otherwise, so a default capture is byte-identical.
        self.prepare_whichkey(device, queue, width, height)?;
        // THE FORMAT POPOVER (reveal-on-select format toolbar): its own float
        // elevation + active-button wash + labels, anchored over the selection.
        // Parked (nothing drawn) unless a mouse selection summoned it (or the
        // `AWL_POPOVER` capture probe forced it), so a default capture is byte-identical.
        self.prepare_popover(device, queue, width, height)?;
        // The WEB/LINUX MENU BAR (top strip + open dropdown). Parks everything
        // off-screen/empty when the bar is hidden (default off on macOS), so a default
        // capture stays byte-identical; `--menu-bar` / a web/Linux launch shows it.
        self.prepare_menubar(device, queue, width, height)?;
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

    /// Build + upload the markdown `~~strikethrough~~` STRIKE LINES (one flat
    /// band per visual-row segment of every struck span), positioned by THE ONE
    /// strike-line owner (`super::spans::strike_line_band` — see
    /// [`super::TextPipeline::strike_lines`]). Empty (nothing uploaded, nothing
    /// drawn) for a strike-less / non-markdown buffer, so those frames stay
    /// byte-identical.
    pub(super) fn prepare_strike_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        let lines = self.strike_lines();
        self.strike_pipeline
            .prepare(device, queue, width, height, &lines);
    }
}
