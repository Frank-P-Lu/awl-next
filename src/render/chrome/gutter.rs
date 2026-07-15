//! PAGE-MODE ORIENTATION GUTTER chrome — the quiet bottom-left stacked label
//! (filename over project), right-aligned to hug the writing column from the
//! margin, plus its sidecar report and the doc-dimming predicate. Inherent methods
//! on [`super::TextPipeline`]; carved out of `chrome.rs` verbatim, no behaviour
//! change. See [`super`].

use super::*;

/// The vertical breath (in LABEL rows) added ABOVE the gutter block when carving
/// its local lava corner — a half-row so the feathered top face clears the top
/// glyph. Read by [`TextPipeline::gutter_carve_rect`] and pinned by the
/// gutter-corner bounds law (`theme::tests`).
pub(in crate::render) const GUTTER_CARVE_BREATH: f32 = 0.5;

impl TextPipeline {
    /// The page-mode GUTTER's fully decided layout for this frame: the available
    /// RIGHT-aligned box width (px), the filename AND the project line each
    /// ALREADY fit to ONE line independently (never left to cosmic-text's own
    /// word-wrap — see [`Self::prepare_gutter`]'s doc). `None` when the gutter is
    /// HIDDEN outright: edge-to-edge (no margin to hold it), no buffer name, or a
    /// margin too narrow for even a stub filename ([`rowlayout::GUTTER_MIN_NAME_CHARS`]
    /// — better absent than confetti). The label's right edge lands at `avail` — a
    /// small gap shy of the writing column's left edge — so it hugs the column
    /// from the margin. Shared by [`Self::prepare_gutter`] (what is drawn) and
    /// [`Self::gutter_report`] (what the sidecar says), so the two never drift:
    /// this is the ONE place that decides the gutter's text, never `prepare_gutter`
    /// laying raw text into a wrapping box.
    ///
    /// **Neither line yields to the other from width pressure** — both share the
    /// SAME `avail_chars` budget and elide independently through
    /// [`rowlayout::fit_primary`]; the project line comes back empty here only
    /// when `self.gutter_project` itself is empty (no project at all), never as a
    /// forced yield to protect the filename.
    fn gutter_layout(&self) -> Option<GutterLayout> {
        if !crate::page::page_on() || self.gutter_name.is_empty() {
            return None;
        }
        let gap = self.metrics.char_width * MARGIN_COLUMN_GAP_CHARS;
        let avail = self.column_left() - gap;
        // Char budget at the LABEL scale the gutter actually renders at (the doc's
        // own `metrics.char_width` is the FULL-size advance; the gutter's glyphs
        // are smaller, so its per-char footprint shrinks with it).
        let label_char_w = self.metrics.char_width * crate::markdown::type_scale::LABEL;
        let avail_chars = if label_char_w > 0.0 {
            (avail / label_char_w).floor().max(0.0) as usize
        } else {
            0
        };
        let plan = rowlayout::gutter_plan(avail_chars)?;
        let name = rowlayout::fit_primary(&self.gutter_name, plan.name_budget);
        let project = if plan.show_project && !self.gutter_project.is_empty() {
            rowlayout::fit_primary(&self.gutter_project, plan.project_budget)
        } else {
            String::new()
        };
        Some(GutterLayout { avail, name, project })
    }

    /// Shape + upload the page-mode ORIENTATION GUTTER: a quiet stacked label in the
    /// BOTTOM-LEFT margin — the filename (LABEL size × MUTED ink) over the project (LABEL ×
    /// FAINT ink), RIGHT-aligned so it hugs the writing column from the margin and
    /// anchored to the BOTTOM of the left margin. This relocates orientation OUT of the
    /// writing column into the side (DESIGN §4: the faintest inks at the smallest size,
    /// present when you look, invisible when you don't). HIDDEN edge-to-edge / with no
    /// name (parked off-screen → byte-identical).
    pub(in crate::render) fn prepare_gutter(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let m = self.metrics;
        let label = crate::markdown::type_scale::LABEL;
        let muted = theme::muted().to_glyphon();
        let faint = theme::faint().to_glyphon();
        // A compact stacked label: scale BOTH font size and line height to LABEL so the
        // two rows nest tightly (this buffer is standalone, not row-aligned to the doc).
        self.gutter_buffer.set_metrics(
            &mut self.font_system,
            GlyphMetrics::new(m.font_size * label, m.line_height * label),
        );
        let base = panel_attrs();
        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        // Hidden: empty text parked off-screen, so nothing draws and a non-page (or
        // unnamed) capture stays byte-identical.
        let Some(layout) = self.gutter_layout() else {
            self.gutter_buffer
                .set_size(&mut self.font_system, Some(1.0), Some(m.line_height));
            self.gutter_buffer.set_text(
                &mut self.font_system,
                "",
                &base.clone().color(muted),
                Shaping::Advanced,
                None,
            );
            self.gutter_buffer
                .shape_until_scroll(&mut self.font_system, false);
            let area = TextArea {
                buffer: &self.gutter_buffer,
                left: 0.0,
                top: -1000.0,
                scale: 1.0,
                bounds,
                default_color: muted,
                custom_glyphs: &[],
            };
            self.gutter_renderer
                .prepare(
                    device,
                    queue,
                    &mut self.font_system,
                    &mut self.atlas,
                    &self.viewport,
                    [area],
                    &mut self.swash_cache,
                )
                .map_err(|e| anyhow::anyhow!("glyphon gutter prepare failed: {e:?}"))?;
            return Ok(());
        };
        // The filename AND the project line are ALREADY fit to one line each by
        // `gutter_layout` (through the shared `rowlayout::fit_primary` door) — this
        // box NEVER lays raw, possibly-overflowing text into a wrapping width, so
        // neither line can ever word-wrap mid-word.
        let name = layout.name;
        let project = layout.project;
        // Filename (muted) over project (faint). The project line carries its own
        // leading newline so it stacks under the filename; an empty project (no
        // project at all — never a width-pressure yield) => name only.
        let proj_line = if project.is_empty() {
            String::new()
        } else {
            format!("\n{project}")
        };
        let mut spans: Vec<(&str, Attrs)> = vec![(name.as_str(), base.clone().color(muted))];
        if !proj_line.is_empty() {
            spans.push((proj_line.as_str(), base.clone().color(faint)));
        }
        let lines = if proj_line.is_empty() { 1.0 } else { 2.0 };
        self.gutter_buffer.set_size(
            &mut self.font_system,
            Some(layout.avail),
            Some(m.line_height * label * lines + 1.0),
        );
        let default_attrs = base.clone().color(muted);
        self.gutter_buffer.set_rich_text(
            &mut self.font_system,
            spans,
            &default_attrs,
            Shaping::Advanced,
            Some(glyphon::cosmic_text::Align::Right),
        );
        self.gutter_buffer
            .shape_until_scroll(&mut self.font_system, false);
        // BOTTOM-anchored in the left margin: the stacked block's BOTTOM edge sits one
        // small margin (8px) up from the canvas bottom — the same bottom row the corner
        // readouts use — so `top` is the canvas bottom minus the block's own height. Left
        // 0 with the buffer width == `avail` keeps the right edge a gap shy of the column
        // (horizontal placement unchanged; only the vertical anchor moved top → bottom).
        let block_h = m.line_height * label * lines;
        let top = height as f32 - block_h - 8.0;
        let area = TextArea {
            buffer: &self.gutter_buffer,
            left: 0.0,
            top,
            scale: 1.0,
            bounds,
            default_color: muted,
            custom_glyphs: &[],
        };
        self.gutter_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon gutter prepare failed: {e:?}"))?;
        Ok(())
    }

    /// Whether the page-mode GUTTER is actually DRAWN this frame — THE one
    /// visibility rule, read straight off [`Self::gutter_layout`]'s own full gate
    /// (page mode + a buffer name + a margin past the hard floor), never a
    /// re-derivation. Exposed for the LAVA gutter corner carve
    /// ([`TextPipeline::lava_gutter_carve_rect`], `render/layers.rs`). Reading the
    /// SAME owner `prepare_gutter`/`gutter_report` share means the carve can never
    /// disagree with what the frame draws.
    pub(in crate::render) fn gutter_visible(&self) -> bool {
        self.gutter_layout().is_some()
    }

    /// THE GUTTER'S LOCAL LAVA CARVE RECT `[left, top, right, bottom]` (px) — the
    /// bounded bottom-left region the lava field vanishes from while the gutter
    /// draws, so its `muted`/`faint` stack sits on flat ground while the REST of
    /// both margins keep the lamp (an ordinary doc goes both-sides — the fix for
    /// the gutter gating the whole-margin carve). Derived from the SAME
    /// [`Self::gutter_layout`] owner `prepare_gutter` lays the block from, so the
    /// carve exactly covers the drawn block:
    ///
    /// * `left = 0`, `right = avail` (the gutter's own box — the filename/project
    ///   are RIGHT-aligned within `[0, avail]`, `avail` a small gap shy of the
    ///   writing column), so the carve spans the block's full horizontal extent.
    /// * `bottom = height` (the block is bottom-anchored an 8px inset up), `top =
    ///   the block top minus a half-row breath` — the bottom band the two stacked
    ///   LABEL rows occupy. `None` when the gutter is HIDDEN (nothing to carve).
    ///
    /// The half-row breath and the `+1.0` mirror `prepare_gutter`'s own box; the
    /// [`GUTTER_CARVE_BREATH`] constant names the pad the bounds law reads.
    pub(in crate::render) fn gutter_carve_rect(&self, height: u32) -> Option<[f32; 4]> {
        let layout = self.gutter_layout()?;
        let label = crate::markdown::type_scale::LABEL;
        let lines = if layout.project.is_empty() { 1.0 } else { 2.0 };
        let block_h = self.metrics.line_height * label * lines;
        // `prepare_gutter` anchors the block bottom 8px up from the canvas bottom.
        let block_top = height as f32 - block_h - 8.0;
        let breath = self.metrics.line_height * label * GUTTER_CARVE_BREATH;
        let top = (block_top - breath).max(0.0);
        Some([0.0, top, layout.avail, height as f32])
    }

    /// The page-mode GUTTER state for the capture sidecar: `Some((name, project))`
    /// EXACTLY when the gutter is drawn (page mode on, a buffer name, a margin past
    /// the hard floor — the same gate as [`Self::prepare_gutter`]), else `None`.
    /// Both `name` and `project` are EXACTLY as drawn — each already fit to one
    /// line, independently middle-elided (extension preserved) only once the
    /// margin can't hold it whole. Neither one yields to the other from width
    /// pressure: `project` is empty here only when there is genuinely no project
    /// to show, so the sidecar always agrees with the pixels.
    pub fn gutter_report(&self) -> Option<(String, String)> {
        self.gutter_layout().map(|g| (g.name, g.project))
    }

    /// True when a FULL-takeover overlay is up and the document RECEDES behind it (the
    /// cached frosted-blur backdrop is active). False for the search SPLIT panel / no
    /// overlay (the doc stays bright), for the crisp THEME/CARET pickers (the doc stays
    /// crisp so the live theme colours / caret preview read honestly), AND for the
    /// contextual SPELL panel (a small float popup at the word — it recedes nothing).
    /// Reported in the sidecar as `dim_overlay`.
    pub fn dims_doc(&self) -> bool {
        self.overlay_active && !self.overlay_crisp && self.overlay_spell.is_none()
    }
}
