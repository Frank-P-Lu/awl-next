//! FRAME PREPARE — per-frame buffer preparation and blur-cache state.

use super::*;

impl TextPipeline {
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        // INVARIANT: the document buffer's soft-wrap width must ALWAYS equal the
        // live page COLUMN width. `column_left()` / `column_width()` and the margin
        // background are recomputed from the live page state EVERY frame, but the
        // buffer is only re-wrapped at the scattered `set_size` / `set_dpi` /
        // `set_text` call sites. Any state flip those sites miss (a page-mode toggle
        // or measure change that doesn't re-wrap, the width-preserving theme reshape)
        // leaves the buffer wrapped at a STALE, wider width while the column re-centers
        // — so the text wraps too wide from the centered left, overflowing the right
        // edge with NO right margin. Re-deriving here makes divergence impossible at
        // any window size / DPI. cosmic-text no-ops when the width is unchanged, so a
        // settled frame stays free.
        self.sync_wrap_width();
        self.viewport.update(queue, Resolution { width, height });

        self.prepare_background_layer(queue, width, height);
        // THE LAVA-LAMP GROUND: over the flat margin ground, before the washes.
        // A no-op (draws nothing) for every non-lava world.
        self.prepare_lava_layer(queue, width, height);
        // TWINKLING STARS: the ambient star field in the margins (zero
        // instances for every AmbientStyle::None world — byte-identical).
        self.prepare_stars_layer(device, queue, width, height);
        // THE PAGE FRAME: the thin writing-column frame (zero rects for every
        // PageFrame::None world, so those stay byte-identical).
        self.prepare_page_frame(device, queue, width, height);
        // DIFF-AS-PREVIEW: the page-column card dressing (parked on every
        // ordinary frame). Prepared before the washes/text so its quads sit
        // under them in the document band (painter's order is the draw fn's).
        self.prepare_diff_panel(device, queue, width, height);
        self.prepare_wash_layer(device, queue, width, height);
        self.prepare_wysiwyg_wash_layer(device, queue, width, height);
        self.prepare_text_layer(device, queue, width, height)?;
        // THE X-RAY: stash the caret's table-row floated source BEFORE the caret /
        // selection layers, so their `col_x_and_advance` redirects onto it (the
        // concealed doc row is zero-width). A no-op off a table row.
        self.prepare_table_xray();
        self.prepare_caret_layer(device, queue, width, height);
        self.prepare_selection_layer(device, queue, width, height);
        self.prepare_ornaments(device, queue, width, height)?;
        self.prepare_table_grid(device, queue, width, height)?;
        // INLINE IMAGES: the tall rows are reserved at reshape (the per-line height
        // override in `build_line_attrs`); this decodes each visible off-cursor image
        // (`image_cache`, downscaled), builds the textured quads (fit-to-column,
        // centered in the reserved row), and the calm missing-file placeholders. All
        // three layers park empty when off / no images, so a capture is byte-identical.
        self.prepare_images(device, queue, width, height)?;
        self.prepare_chrome_layer(device, queue, width, height)?;
        self.prepare_spell_layer(device, queue, width, height);
        self.prepare_nit_layer(device, queue, width, height);
        self.prepare_strike_layer(device, queue, width, height);
        self.prepare_link_underline_layer(device, queue, width, height);
        self.prepare_blur(device, queue, width, height);
        Ok(())
    }

    /// True when the FROSTED-BLUR backdrop applies this frame: a full-takeover
    /// overlay is up AND it is NOT a crisp-exception picker (theme / caret) NOR the
    /// contextual SPELL panel (a small floating popup at the word — it recedes
    /// nothing, DESIGN §5). The search SPLIT panel (`search_active`, not
    /// `overlay_active`) is never blurred.
    fn overlay_blur(&self) -> bool {
        self.overlay_active && !self.overlay_crisp && self.overlay_spell.is_none()
    }

    /// True when the SUMMONED-WHILE-HELD stats HUD should actually DRAW this frame.
    /// The HUD and a full summoned overlay are MUTUALLY EXCLUSIVE (the overlay wins):
    /// a still-held Option-Cmd-I must not draw its card over an open picker — nor force the
    /// frosted blur that would defeat the theme picker's crisp live-color preview.
    /// One owner for both gates (`backdrop_blur` + `prepare_hud`), keyed off the same
    /// `overlay_active` flag the overlay draw path already reads, so they can't drift;
    /// the HUD reappears once the overlay closes if the key is still held.
    pub(in crate::render) fn hud_showing(&self) -> bool {
        crate::hud::hud_held() && !self.overlay_active
    }

    /// True when the HOLD-⌘ SHORTCUT PEEK should DRAW this frame. Like the held HUD, it
    /// yields to an open summoned overlay (`!overlay_active`) so it never draws its card
    /// over a picker — the bare-⌘ hold that summons it can't coexist with a modal picker
    /// in practice, but the gate keeps the two mutually exclusive by construction, same
    /// as `hud_showing`.
    pub(in crate::render) fn peek_showing(&self) -> bool {
        crate::peek::peek_open() && !self.overlay_active
    }

    /// True when ANY frosted-blur backdrop applies this frame: a blur-eligible full
    /// overlay ([`Self::overlay_blur`]) OR the SUMMONED-WHILE-HELD stats HUD. The HUD now
    /// recedes the document behind the SAME hue-preserving frost the palette uses — not
    /// the old neutral grey scrim — so the two takeovers read consistently (DESIGN §5:
    /// the doc recedes by BLUR, not grey). Drives both the blur prepare + the render
    /// path's offscreen-capture branch.
    ///
    /// **TRUE 1-BIT WORLDS (`Theme::render_caps.backdrop == Backdrop::Flat`) forgo the frost entirely.** A
    /// gaussian defocus of a document that is only ever pure black or pure
    /// white mathematically SMEARS every edge into intermediate grey — there
    /// is no tuning of the blur that avoids this, it is the nature of the
    /// operation. Every consumer (overlay takeover, held HUD, the lifetime
    /// card, hold-peek) falls back to the EXISTING crisp path instead — the
    /// same "document stays bright, no blur, no scrim" exception the
    /// theme/caret pickers already use — so the solid white-bordered card
    /// still reads clearly over a SHARP, not smeared, black/white document.
    pub(in crate::render) fn backdrop_blur(&self) -> bool {
        if theme::active().render_caps.backdrop == theme::Backdrop::Flat {
            return false;
        }
        self.overlay_blur()
            || self.hud_showing()
            || crate::lifetime::lifetime_open()
            || crate::streaks::streaks_open()
            || self.peek_showing()
    }

    /// Size the blur textures + decide whether the cached frosted backdrop must be
    /// RECOMPUTED this frame. Only does work while a blur-eligible overlay is up; the
    /// actual doc-capture + blur passes run in [`Self::render`] (they need the frame
    /// encoder). The recompute gate compares a signature of the doc/size/theme behind
    /// the overlay, so an idle overlay-open frame re-blurs nothing (DESIGN §6).
    pub(in crate::render) fn prepare_blur(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        if !self.backdrop_blur() {
            return;
        }
        let base100 = srgb_u8_to_linear3(theme::base_100().rgba_bytes());
        let recreated = self.blur.ensure(device, queue, width, height, base100);
        let sig = self.blur_signature(width, height);
        self.blur_recompute = recreated || self.blur_sig != Some(sig);
        if self.blur_recompute {
            self.blur_sig = Some(sig);
        }
    }

    /// A cheap signature of everything that affects the BACKDROP pixels: the canvas
    /// size + DPI, the active theme, the document's render state (reshape count,
    /// scroll, cursor, zoom, markdown-ness), and the PAGE / WRAP geometry. The live
    /// caret SPRING is deliberately excluded so an in-flight caret settle behind a
    /// freshly-opened overlay does not keep re-blurring — the backdrop is frozen the
    /// moment it is captured.
    ///
    /// The page/wrap piece fixes a real staleness bug: `reshape_count` only bumps on
    /// a TEXT reshape (`set_text`), not on a pure re-wrap from a width change (page
    /// drag, `C-x {`/`}`, a page-mode toggle) — `set_size`/`sync_wrap_width` re-wrap
    /// without touching `reshape_count`. So on a width-only change the cached frosted
    /// backdrop passed stale, rendering the OLD column behind a freshly-opened
    /// overlay. `prepare` calls `sync_wrap_width` before `prepare_blur`, so by the
    /// time this runs, `row_geom`'s generation (bumped by `RowGeom::invalidate`
    /// whenever the shaped runs actually re-wrap) already reflects this frame's wrap
    /// width — the same generation the squiggle/nit proto caches key on. Hashing
    /// `page::page_on()` + `page::measure()` alongside it also catches the rare case
    /// where those flip WITHOUT changing the resulting wrap width (e.g. toggling page
    /// mode when the window is already narrower than the measure) — the page surface
    /// itself still needs a recompute even though `row_geom` wouldn't invalidate.
    pub(in crate::render) fn blur_signature(&self, width: u32, height: u32) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        width.hash(&mut h);
        height.hash(&mut h);
        self.dpi.to_bits().hash(&mut h);
        theme::active().name.hash(&mut h);
        self.reshape_count.hash(&mut h);
        self.row_geom.generation().hash(&mut h);
        crate::page::page_on().hash(&mut h);
        crate::page::measure().hash(&mut h);
        self.scroll_lines.hash(&mut h);
        self.cursor_line.hash(&mut h);
        self.cursor_col.hash(&mut h);
        self.metrics.zoom.to_bits().hash(&mut h);
        self.md_enabled.hash(&mut h);
        self.lava_render_phase().to_bits().hash(&mut h);
        h.finish()
    }
}
