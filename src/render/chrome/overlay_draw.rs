//! SUMMONED OVERLAY chrome — the DRAW half. The geometry/hit-test owner
//! ([`super::overlay`]: `overlay_geometry`, the row-window math, the metric
//! ladder, `overlay_footer_reclaim`, and the pointer hit-tests) computes WHERE
//! every element of the summoned card sits; this file turns that settled geometry
//! into GPU work — shaping + uploading the card quad and its 1-bit elevation, the
//! selected-row band / per-item bars, the placard wordmark, the name + chord text
//! areas, the faceting-strip mark, and the amber query caret — plus the
//! park-when-off discipline and the draw-state test probes (`overlay_row_y_probe`
//! / `overlay_text_glyph_count` / `living_probe_geom`, and the `OverlayYProbe`
//! fixture the y-agreement law reads).
//!
//! Carved out of [`super::overlay`] verbatim, no behaviour change. `TextPipeline`
//! lives in [`crate::render`], of which this is a descendant module, so these
//! methods keep full access to its private GPU fields; Rust merges the inherent
//! `impl TextPipeline` blocks across the module tree, so splitting the file is a
//! pure physical carve — the chrome pixels are byte-identical. See [`super`].

use super::*;

/// PER-ITEM LIST SURFACES round — the corner radius (device px) of the faceted
/// strip's active [`theme::FacetStyle::Band`] pill. A single dial the gallery A/Bs.
const FACET_CHIP_RADIUS: f32 = 6.0;

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
        // for fill worlds; Bombora-under-Bars was the 2.53:1 exhibit) the ONE
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
        // ARM B — INK RIDES THE BAND, NOT THE STATE: when the living-band probe
        // is flying (env-gated → `None` on every ordinary run, byte-identical),
        // the rows the MOVING band currently covers take the on-band ink and the
        // target row keeps its off-band ink until the band arrives. `None`
        // recovers the old state-tied flip (the settled selected row).
        let living_covered = self.living_covered_rows(&geom);
        let has_right =
            self.overlay_shape_text(&geom, ink, muted, selected_ink, living_covered.as_deref());
        self.overlay_upload_text(
            device, queue, width, height, &geom, has_right, ink, muted, placard,
        )?;
        self.overlay_draw_card(device, queue, width, height, &geom);
        self.overlay_place_caret(queue, width, height, &geom);
        Ok(())
    }

    /// ARM B LIVING-BAND PROBE — the DISPLAY rows the moving band covers THIS
    /// frame (see [`livingband::covered_rows`]), so the shaper flips those rows'
    /// ink to the on-band pole instead of the static selected row ("ink rides
    /// the band, not the state"). `None` on every ordinary run (env unset, a
    /// Bars or empty picker, or no selection), where the shaper is byte-identical
    /// (the old `overlay_selected` flip). Applies to BOTH the flat and the FACETED
    /// (Cmd-P palette / Settings) layouts — the target row is placed through the
    /// shared [`Self::overlay_selected_display_line`] owner so it matches the fill exactly
    /// on either. Reads the SAME phase + rects owner (`living_band_phase` /
    /// `living_band_rects`) `overlay_draw_card` draws from, so the flipped rows can
    /// never disagree with the fill's position — the exact phase-seam fix the
    /// outcome audit demands. The target row is placed through
    /// [`Self::overlay_selected_display_line`] — the ONE owner also read by the
    /// selected-band fill and the secondary right-column recolor — so the band, the
    /// hint recolor, and the flipped ink can never read a different row.
    pub(in crate::render) fn living_covered_rows(&mut self, geom: &OverlayGeom) -> Option<Vec<usize>> {
        let motion = crate::render::livingband::overlay_motion_force()?;
        if !matches!(crate::render::effective_list_style(), theme::ListStyle::Pane) {
            return None;
        }
        // The band flies to (and covers) the selected candidate's DISPLAY row —
        // the SAME per-layout-family owner `overlay_draw_card` places the fill on
        // (`overlay_selected_display_line`, handling the faceted plan-position case), so
        // the rows whose ink flips can never read a different target than the fill
        // draws. `None` (empty picker) → no covered rows. FIXED: the old
        // `geom.theme` bail made this dead code on every FACETED surface (the Cmd-P
        // palette / Settings — `geom.theme == true` there), so the fill animated
        // while the ink stayed state-tied → covered rows washed out (white-on-white
        // on Wagtail). The shaper's `covered` seam threads through the faceted path
        // too now, so ink rides the band wherever the fill does.
        let sel_disp = self.overlay_selected_display_line(geom)?;
        let lh = self.overlay_lh();
        let target =
            overlay_row_top(geom.text_top, geom.header_rows, geom.header_gap, sel_disp, lh);
        let (from, to, t) = self.living_band_phase(motion, target, lh);
        let (primary, echo, _cross) =
            self.living_band_rects(motion, from, to, t, geom.card_x, geom.card_w, lh);
        // Leading band + chasing echo as vertical extents (x/width irrelevant to
        // row coverage). The morph voices carry no echo (`echo` empty).
        let bands: Vec<crate::render::livingband::BandRect> = primary
            .iter()
            .chain(echo.iter())
            .map(|r| crate::render::livingband::BandRect { top: r[1], height: r[3] })
            .collect();
        let first_top =
            overlay_row_top(geom.text_top, geom.header_rows, geom.header_gap, 0, lh);
        Some(crate::render::livingband::covered_rows(
            &bands,
            first_top,
            lh,
            geom.visible,
        ))
    }

    /// TEST ONLY — the living-band ink probe's geometry for a capture-level PIXEL
    /// law (the Wagtail fill/ink-divergence class): the covered display rows, the
    /// selected TARGET display row, the candidate-row band (`first_top`, `lh`), and
    /// the LEADING band's drawn rect `[x, top, w, h]` this frame. Reads the SAME
    /// owners the renderer draws from (`living_covered_rows`, `overlay_selected_display_line`,
    /// `overlay_row_top`, `living_band_phase`/`living_band_rects`), so a pixel test
    /// samples exactly where the fill lands. Panics unless the motion probe is armed.
    #[cfg(test)]
    pub(in crate::render) fn living_probe_geom(
        &mut self,
        geom: &OverlayGeom,
    ) -> (Vec<usize>, usize, f32, f32, [f32; 4]) {
        let motion = crate::render::livingband::overlay_motion_force()
            .expect("living_probe_geom needs the motion probe armed");
        let covered = self.living_covered_rows(geom).unwrap_or_default();
        let target = self.overlay_selected_display_line(geom).expect("a selected row");
        let lh = self.overlay_lh();
        let first_top = overlay_row_top(geom.text_top, geom.header_rows, geom.header_gap, 0, lh);
        let sel_top =
            overlay_row_top(geom.text_top, geom.header_rows, geom.header_gap, target, lh);
        let (from, to, t) = self.living_band_phase(motion, sel_top, lh);
        let (primary, _echo, _cross) =
            self.living_band_rects(motion, from, to, t, geom.card_x, geom.card_w, lh);
        (covered, target, first_top, lh, primary[0])
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
        // ARM B LIVING-BAND PROBE: the two-shape crossing quad parks empty too, so
        // a closed picker carries no stale crossing quad into the next frame.
        self.overlay_cross.prepare(device, queue, width, height, &[]);
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
            GlyphMetrics::new(m.font_size * crate::render::effective_overlay_scale() * label, lh),
        );
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
            // GRACEFUL DEGRADATION (AtlasFull fix, 2026-07-17): the quantized sizing
            // keeps the shared atlas bounded, but if it ever DOES fill (a huge display,
            // an exotic GPU with a small `max_texture_dimension_2d`), SKIP the placard
            // for this frame rather than erroring — prepare an empty pass so no stale
            // wordmark lingers, and let the next frame retry after the off-frame
            // `atlas.trim()` reclaims space. NEVER a print (the `gpu.rs` `prepare error:`
            // eprintln is the thing this silences for the placard's own overflow); a
            // non-AtlasFull error still propagates.
            let placard_prepare = self.placard_renderer.prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                placard_pass,
                &mut self.swash_cache,
            );
            match placard_prepare {
                Ok(()) => {}
                Err(glyphon::PrepareError::AtlasFull) => {
                    self.placard_renderer
                        .prepare(
                            device,
                            queue,
                            &mut self.font_system,
                            &mut self.atlas,
                            &self.viewport,
                            Vec::new(),
                            &mut self.swash_cache,
                        )
                        .map_err(|e| anyhow::anyhow!("glyphon placard skip-prepare failed: {e:?}"))?;
                }
            }
        }
        // The placard wordmark is FIRST in the panel batch under `Pane` (drawn
        // behind everything that follows), clipped to the WHOLE CANVAS — a
        // screen-corner watermark that bleeds OVER the scrim behind the card
        // (never the tighter card/text rect), per `overlay_shape_placard`'s doc.
        // Under `Bars` it was uploaded to `placard_renderer` above instead, so it
        // is withheld here.
        let mut areas: Vec<TextArea> = Vec::new();
        // Whether the placard rides THIS (Pane) panel batch as the FIRST area — the
        // one entry whose giant glyphs could overflow the shared atlas. Tracked so the
        // graceful-degradation retry below can drop exactly it (see the prepare site).
        let mut placard_in_panel = false;
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
                placard_in_panel = true;
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
            Some(_) => {
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
                // One shifted slice per candidate display row. The per-row DRAW
                // offset rides the ONE `overlay_slant_dx` owner (the fan-in
                // progress folded in — motion choreography 3), so the text and the
                // bar plates below cascade by the same amount every frame.
                for k in 0..n_lines {
                    let row_top = first_top + k as f32 * lh;
                    areas.push(TextArea {
                        buffer: &self.panel_buffer,
                        left: text_left + self.overlay_slant_dx(k),
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
        // GRACEFUL DEGRADATION (AtlasFull fix, 2026-07-17): under `Pane` the placard
        // rides this batch as `areas[0]` (drawn behind the rows). If its giant glyphs
        // ever overflow the shared atlas, re-prepare WITHOUT the placard (the rows are
        // the affordance that must survive; the watermark is the sacrificeable one), so
        // an AtlasFull never blanks the whole card. The next frame retries after the
        // off-frame `atlas.trim()`. A retry area-set is built only when the placard is
        // actually in this batch — every other run pays nothing and never re-prepares.
        // The placard-free fallback batch, built ONLY when the placard is in this batch
        // (every other run keeps `None` and never clones).
        let panel_retry: Option<Vec<TextArea>> =
            placard_in_panel.then(|| areas.iter().skip(1).cloned().collect());
        match self.panel_renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut self.swash_cache,
        ) {
            Ok(()) => {}
            Err(glyphon::PrepareError::AtlasFull) => match panel_retry {
                Some(retry) => self
                    .panel_renderer
                    .prepare(
                        device,
                        queue,
                        &mut self.font_system,
                        &mut self.atlas,
                        &self.viewport,
                        retry,
                        &mut self.swash_cache,
                    )
                    .map_err(|e| anyhow::anyhow!("glyphon overlay skip-placard prepare failed: {e:?}"))?,
                None => return Err(anyhow::anyhow!("glyphon overlay prepare failed: AtlasFull")),
            },
        }
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
        let spell = self.overlay_spell.is_some();
        let card_rect = [geom.card_x, geom.card_y, geom.card_w, geom.card_h];
        // THE ONE ROW-BACKING OWNER ([`theme::ListStyle::list_backing`]): a `Pane`
        // world backs its rows with an opaque CARD; a `Bars` world floats them as
        // bare plates over either a full-canvas ROOM (the centered picker) or, for
        // the contextual spell popup, NOTHING but each plate's own ground SCRIM
        // (option B — no room box at all, so the live document shows BETWEEN the
        // plates even on a dark world, instead of a near-black box behind them).
        // ONE classifier, read here AND by the surface-audit laws, so a
        // Firetail-family world can never box one surface while floating the other
        // (that divergence WAS the spell-popup-on-Bars pane bug).
        let backing = list_style.list_backing(spell);
        match backing {
            theme::ListBacking::Room => {
                // The centered Bars PICKER drops the pane and floats its rows on a
                // UNIFORM value VEIL of the world's own ground (`overlay_bars_room` =
                // `base_100` — a scrim, never a bordered box: no shadow, no border, no
                // bright `base_300` fill). It pulls the crisp live-preview document a
                // value back into one calm plane, killing the "gap comb seam" the raw
                // doc showed through every inter-bar gap. The room BLEEDS past all four
                // canvas edges so the panel quad pipeline's ~1px feather
                // (`selection.wgsl`'s `smoothstep(-1, 1, d)`) lands off-screen — a
                // plane flush to `[0, 0, w, h]` left row 0 only ~84% covered (a 1px
                // lighter seam along y = 0). Drawn FIRST in `draw_overlay_card`, so the
                // placard watermark, the bars, and the row text composite over it.
                const ROOM_BLEED: f32 = 2.0;
                self.panel_card.set_corner(0.0);
                self.panel_card
                    .set_color(theme::overlay_bars_room().rgba_bytes());
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
            }
            theme::ListBacking::BarePlates => {
                // OPTION B (the user's "b is good") — the contextual spell popup on a
                // Bars world floats its suggestion plates on the RAW PAGE with NO room
                // box at all. The prior round clipped the `base_100` room to the card,
                // which on a DARK world read as a prominent near-black BOX behind the
                // plates; the user wanted that gone. Legibility over the live document
                // is now carried NOT by a rectangle but by each plate's own minimal
                // ground SCRIM — a thin feathered moat confined to the plate footprint
                // (a value step, DESIGN §3/§5), prepared on `panel_card` in the plate
                // block BELOW once the plate rects are known (they depend on the
                // just-shaped label widths, so they can't be built up here). We only
                // PARK the room + float quads here so nothing boxes the popup; the
                // caret-preview pass already parked the float this frame (it and the
                // spell popup are mutually exclusive), but park explicitly so a future
                // reorder can never leak the raised pane back under the plates.
                self.prepare_float_panel(device, queue, width, height, None);
                self.panel_shadow.prepare(device, queue, width, height, &[]);
                self.panel_border.prepare(device, queue, width, height, &[]);
                // `panel_card` is DEFERRED to the plate block (the per-plate scrims).
            }
            theme::ListBacking::Card if spell => {
                // PANE world spell popup: elevate on the float primitive — a small
                // raised card at the misspelled word (UNCHANGED / byte-identical to
                // before). The flat/room `panel_*` quads stay empty here.
                self.prepare_float_panel(device, queue, width, height, Some(card_rect));
                self.panel_card.prepare(device, queue, width, height, &[]);
                self.panel_shadow.prepare(device, queue, width, height, &[]);
                self.panel_border.prepare(device, queue, width, height, &[]);
            }
            theme::ListBacking::Card => {
                // Centered PANE picker: the flat opaque card, ELEVATED (bordered) only
                // on a true 1-bit world — see `prepare_panel_card_elevation`'s doc.
                self.prepare_panel_card_elevation(device, queue, width, height, Some(card_rect));
            }
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
        // The ONE owner (`overlay_selected_display_line`) feeds the secondary
        // (right-column) shaper's selected-row recolor AND the living-band ink flip
        // (`living_covered_rows`), so band fill, hint recolor, and flipped ink can
        // never disagree on WHICH row is selected.
        let sel_disp: Option<usize> = self.overlay_selected_display_line(geom);
        // PER-ITEM LIST SURFACES round: `Pane` (default) draws the byte-identical
        // full-width selected BAND; `Bars` gives each candidate row its own
        // rounded surface (unselected → `overlay_bars`, quiet; selected →
        // `overlay_rows`, brighter + `grow_px` wider) with the gap already folded
        // into `lh`. The row-y owner `overlay_row_top` feeds BOTH so bars and text
        // agree on every row; the hit-test rides the same `lh`, so a click in a
        // gap maps to the nearest row (no dead zones).
        // `list_style` computed once at the top of this fn (drives the pane-drop).
        let mirror = crate::render::effective_card_anchor().mirrors_growth();
        // ARM B LIVING BAND (`AWL_LIVING_BAND`): the selection band's morph /
        // two-shape choreography. Ships ON (calm MORPH) — `None` only when the knob
        // is `off`. Pane-only; when active it OWNS the band rects (the ordinary
        // `overlay_band_drawn` slide is skipped for that frame). A settled frame is
        // byte-identical to the ordinary band (MORPH is calm-at-rest and
        // `living_band_phase` settles every capture / Reduce-Motion frame).
        let motion = crate::render::livingband::overlay_motion_force();
        // The selected row's settled TARGET top, through the ONE row-y owner —
        // shared by the ordinary slide and the living-band choreography.
        let sel_target: Option<f32> = sel_disp
            .map(|disp| overlay_row_top(geom.text_top, geom.header_rows, geom.header_gap, disp, lh));
        // The living-band travel + phase this frame (pinned for a capture dump,
        // else the live animator), computed ONCE. `None` unless the probe is set
        // AND the world draws a Pane list.
        let living: Option<(crate::render::livingband::MotionForce, f32, f32, f32)> =
            match (motion, sel_target) {
                (Some(force), Some(target)) if matches!(list_style, theme::ListStyle::Pane) => {
                    let (from, to, t) = self.living_band_phase(force, target, lh);
                    Some((force, from, to, t))
                }
                _ => None,
            };
        // The selected row's drawn TOP for the ORDINARY path, through the ONE
        // row-y owner + the live-only band slide (verbatim `target` in capture /
        // Snap worlds). The living-band probe owns its own rects, so it skips this.
        let sel_top: Option<f32> = match (living.is_some(), sel_target) {
            (true, _) => None,
            (false, Some(target)) => Some(self.overlay_band_drawn(target)),
            (false, None) => None,
        };
        // The two-shape CROSSING quad's rect (probe only; stays empty on every
        // ordinary run → `overlay_cross` draws nothing → byte-identical).
        let mut cross_rects: Vec<[f32; 4]> = Vec::new();
        let (sel_rects, bar_rects): (Vec<[f32; 4]>, Vec<[f32; 4]>) = match list_style {
            theme::ListStyle::Pane => {
                if let Some((force, from, to, t)) = living {
                    // LIVING-BAND PROBE — the selected band becomes the morph /
                    // two-shape choreography. `overlay_rows` carries the leading
                    // band (its `band_color` primary, already set above),
                    // `overlay_bars` the chasing ECHO (a quieter value step; Pane
                    // leaves `overlay_bars` empty otherwise), and `overlay_cross`
                    // the BRIGHTEST crossing. All value, never a hue (DESIGN §3).
                    let (primary, echo, cross) = self
                        .living_band_rects(force, from, to, t, geom.card_x, geom.card_w, lh);
                    self.overlay_bars.set_corner(2.5);
                    self.overlay_bars
                        .set_color(theme::surface_selected().rgba_bytes());
                    self.overlay_cross.set_corner(2.5);
                    self.overlay_cross
                        .set_color(theme::overlay_band_overlap().rgba_bytes());
                    cross_rects = cross;
                    (primary, echo)
                } else {
                    let rects = match (sel_disp, sel_top) {
                        (Some(disp), Some(top)) => {
                            // WILD-MENU SLANT PROBE (env-gated, `None` on every normal
                            // run): the band's left edge follows the selected row's own
                            // stair offset (fan-in progress folded in via the ONE
                            // `overlay_slant_dx` owner) so the highlight hugs its
                            // slanted row.
                            let dx = self.overlay_slant_dx(disp);
                            vec![[geom.card_x + dx, top, geom.card_w - dx, lh]]
                        }
                        _ => Vec::new(),
                    };
                    (rects, Vec::new())
                }
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
                // distinct on saturated worlds (the Bowerbird/Saltpan defect).
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
                            // SLANT-ON-BARS (choreography 2, fan-in folded in): the
                            // plate cascades right with its row through the ONE
                            // `overlay_slant_dx` owner. A full-width plate keeps its
                            // right edge flush (width shed by dx, mirroring the Pane
                            // band); a hug plate just slides. `0.0` unslanted →
                            // byte-identical.
                            let (x, w) = slant_bar_span(x, w, hug, self.overlay_slant_dx(k));
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
                        // SLANT-ON-BARS: the selected plate cascades with its row
                        // too, THEN grows its ledge — so a slanted selected bar
                        // still steps out of formation.
                        let (bx, bw) = slant_bar_span(bx, bw, hug, self.overlay_slant_dx(disp));
                        // GROW-POP (choreography 4): the `grow_px` ledge eases in on
                        // each selection move via the ONE `overlay_grow_progress`
                        // owner. Full `grow_px` in every capture (byte-identical).
                        let grow = grow_px * self.overlay_grow_progress();
                        let (x, w) = super::grow_span(bx, bw, grow, mirror);
                        vec![[x, top + bar_off, w.max(1.0), bar_h]]
                    }
                    _ => Vec::new(),
                };
                (sel, unsel)
            }
        };
        // OPTION B — the contextual spell popup's PER-PLATE GROUND SCRIMS
        // ([`theme::ListBacking::BarePlates`]). With no room box behind the
        // plates (the top match parked `panel_card`), each plate would abut the
        // live document text; a thin ground moat around each plate's footprint
        // walls it off WITHOUT a rectangle — figure/ground by VALUE (`base_100`,
        // the world's own ground; a value step, never a hue — DESIGN §3/§5). Built
        // from the SAME plate rects the bars draw (`bar_rects` = unselected +
        // footer, `sel_rects` = the grown selected plate), so the scrim can never
        // disagree with a plate's shape, then inflated a hair (`SCRIM_PAD`) and
        // fed through `panel_card` (drawn UNDER the plates in `draw_overlay_card`).
        // `2 * SCRIM_PAD` (4px) stays well inside the row `gap` (10px), so the raw
        // page still shows between the scrimmed plates — the popup reads as
        // floating chips on the document, not a box. On a light world `base_100`
        // is the page ground, so the scrim is invisible-but-harmless there (the
        // plate's own value carries it, as it already did under the room).
        if backing == theme::ListBacking::BarePlates {
            const SCRIM_PAD: f32 = 2.0;
            let radius = match list_style {
                theme::ListStyle::Bars { radius, .. } => radius.max(0.0),
                theme::ListStyle::Pane => 0.0,
            };
            let scrims: Vec<[f32; 4]> = bar_rects
                .iter()
                .chain(sel_rects.iter())
                .map(|&[x, y, w, h]| {
                    [
                        x - SCRIM_PAD,
                        y - SCRIM_PAD,
                        w + 2.0 * SCRIM_PAD,
                        h + 2.0 * SCRIM_PAD,
                    ]
                })
                .collect();
            self.panel_card.set_corner(radius + SCRIM_PAD);
            self.panel_card
                .set_color(theme::overlay_bars_room().rgba_bytes());
            self.panel_card
                .prepare(device, queue, width, height, &scrims);
        }
        self.overlay_bars
            .prepare(device, queue, width, height, &bar_rects);
        self.overlay_rows
            .prepare(device, queue, width, height, &sel_rects);
        // ARM B LIVING-BAND PROBE — the two-shape crossing quad (empty on every
        // ordinary run → byte-identical; a non-empty rect only under a `twoshape`
        // probe mid-flight where the leading band and echo overlap).
        self.overlay_cross
            .prepare(device, queue, width, height, &cross_rects);
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
        // The inactive ghost pills / active corner ticks (empty unless Chips on a theme card).
        let mut ghosts: Vec<[f32; 4]> = Vec::new();
        let band = match theme::active()
            .highlight_treatment(crate::render::effective_overlay_selrow_band())
        {
            theme::HighlightTreatment::ValueBand(c) => c,
            theme::HighlightTreatment::InverseFill { band, .. } => band,
        };
        match facet_style {
            theme::FacetStyle::Text => {}
            theme::FacetStyle::Band => {
                self.overlay_lens_underline.set_color(band.rgba_bytes());
                self.overlay_lens_underline.set_corner(FACET_CHIP_RADIUS);
                self.overlay_lens_underline.set_stroke(0.0);
            }
            // The FOUR chip TREATMENTS drive the two facet rect pipelines here
            // (geometry is recorded in `overlay_shape_theme`; only the FILL /
            // STROKE / colour differ). See [`theme::ChipVariant`].
            theme::FacetStyle::Chips(v) => {
                use theme::ChipVariant as V;
                let content = theme::base_content();
                let muted = theme::muted();
                let stroke = crate::render::BAR_OUTLINE_STROKE_PX;
                // The ACTIVE pill pipeline: (fill rgba, corner, stroke).
                let (a_fill, a_corner, a_stroke): ([u8; 4], f32, f32) = match v {
                    V::Hairline => (band.rgba_bytes(), FACET_CHIP_RADIUS, 0.0),
                    // Solid value-step fill; the active label already inverted to the
                    // card ground up in the shaper so it reads on the fill.
                    V::FilledActive => (content.rgba_bytes(), FACET_CHIP_RADIUS, 0.0),
                    // A thick short bar; small corner so it reads as a bar, not a pill.
                    V::Underline => (content.rgba_bytes(), 1.75, 0.0),
                    // No active pill under Bracket (the ticks ride the ghost pipeline).
                    V::Bracket => (content.rgba_bytes(), 0.0, 0.0),
                };
                self.overlay_lens_underline.set_color(a_fill);
                self.overlay_lens_underline.set_corner(a_corner);
                self.overlay_lens_underline.set_stroke(a_stroke);
                // The GHOST / TICK pipeline: (colour, corner, stroke). A `0.0` stroke
                // fills (Bracket's corner ticks); a positive stroke outlines (the
                // inactive ghost pills). Empty `ghosts` means these are unused.
                let (g_color, g_corner, g_stroke): ([u8; 4], f32, f32) = match v {
                    V::Hairline => (muted.rgba_bytes(), FACET_CHIP_RADIUS, stroke),
                    V::Bracket => (content.rgba_bytes(), 0.0, 0.0),
                    // FilledActive / Underline draw no inactive marks; keep sane defaults.
                    V::FilledActive | V::Underline => {
                        (muted.rgba_bytes(), FACET_CHIP_RADIUS, stroke)
                    }
                };
                self.overlay_facet_ghost.set_color(g_color);
                self.overlay_facet_ghost.set_corner(g_corner);
                self.overlay_facet_ghost.set_stroke(g_stroke);
                if geom.theme {
                    ghosts = self.overlay_theme_facet_ghosts.clone();
                }
            }
        }
        self.overlay_lens_underline
            .prepare(device, queue, width, height, &underline);
        // Non-chip skins draw an EMPTY ghost set — keep the historical corner/stroke
        // so `Text`/`Band` prepare it byte-identically (the chip arm set them above).
        if !matches!(facet_style, theme::FacetStyle::Chips(_)) {
            self.overlay_facet_ghost.set_corner(FACET_CHIP_RADIUS);
            self.overlay_facet_ghost
                .set_stroke(crate::render::BAR_OUTLINE_STROKE_PX);
        }
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
