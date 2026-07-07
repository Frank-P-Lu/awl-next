//! PERSISTENT MARGIN OUTLINE chrome — the quiet page-mode table-of-contents that
//! lingers in the LEFT margin (top-anchored), a dim line per heading with the
//! CURRENT section one value rung brighter. The counterpart to the bottom-anchored
//! orientation [`gutter`](super::gutter): orientation lingers in the two margin
//! surfaces (DESIGN.md amendment — outline top-left, gutter bottom-left), so the
//! writing column stays clean. Inherent methods on [`super::TextPipeline`];
//! mirrors the gutter machinery (a standalone glyph buffer shaped at LABEL scale,
//! parked off-screen when hidden, so a default/off capture stays byte-identical).
//! See [`super`].

use super::*;

/// The margin OUTLINE's fully decided layout for one frame — the visible heading
/// lines (each ALREADY fit to one line through the shared [`rowlayout::fit_primary`]
/// door, so the box never lays raw text into a wrapping width), plus the band's
/// left origin `left` (px), one-line `avail` width (px), and top `top` (px). The
/// `bool` per line is whether it is the CURRENT heading (drawn one value rung up —
/// MUTED vs the FAINT rest). On a long document (more headings than the margin holds)
/// the slice FOLLOWS the current heading, so the section you are in never scrolls
/// off (see [`TextPipeline::outline_layout`]).
struct OutlineLayout {
    left: f32,
    avail: f32,
    top: f32,
    lines: Vec<(String, bool)>,
}

impl TextPipeline {
    /// The persistent margin OUTLINE's fully decided layout for this frame, or
    /// `None` when the outline is HIDDEN outright — the graceful-hide rule, ANY of:
    /// the feature is OFF ([`crate::outline::outline_on`]); NOT page mode (no margin
    /// to hold it — edge-to-edge stays clean); a non-markdown buffer or a
    /// heading-free document (`!md_enabled` / `outline_headings.is_empty()`); the
    /// margin is too narrow for even a stub title ([`rowlayout::OUTLINE_MIN_CHARS`],
    /// so a narrow window collapses the outline exactly as it collapses the gutter);
    /// or there is no vertical room for even one row above the gutter's reserved
    /// bottom band. Otherwise the visible lines are each fit to ONE line through the
    /// shared elision door.
    ///
    /// **Long-doc FOLLOW (the chosen default):** when there are more headings than
    /// the margin height holds, the visible window SLIDES to keep the CURRENT
    /// heading on screen — the same [`super::scroll_window`] the pickers use, with
    /// the current heading as the "selection". So the section you are reading never
    /// scrolls out of the margin; short documents show every heading from the top.
    fn outline_layout(&self, height: u32) -> Option<OutlineLayout> {
        if !crate::outline::outline_on() || !crate::page::page_on() {
            return None;
        }
        if !self.md_enabled || self.outline_headings.is_empty() {
            return None;
        }
        let label = crate::markdown::type_scale::LABEL;
        // The LEFT MARGIN band: from the text-left pad in to a small gap shy of the
        // writing column's left edge (mirroring the gutter's `column_left - gap`).
        let left = crate::render::TEXT_LEFT;
        let gap = self.metrics.char_width * 1.5;
        let avail = self.column_left() - gap - left;
        if avail <= 0.0 {
            return None;
        }
        // Char budget at the LABEL scale the outline actually renders at (its glyphs
        // are smaller than the doc's, so its per-char footprint shrinks with it).
        let label_char_w = self.metrics.char_width * label;
        let avail_chars = if label_char_w > 0.0 {
            (avail / label_char_w).floor().max(0.0) as usize
        } else {
            0
        };
        if avail_chars < rowlayout::OUTLINE_MIN_CHARS {
            return None;
        }
        // Vertical extent: TOP-anchored from the text top down to a reserved band
        // above the BOTTOM-anchored gutter, so the two margin surfaces never collide.
        // The gutter is at most two LABEL rows + its 8px inset; reserve that plus a
        // one-row breath.
        let row_h = self.metrics.line_height * label;
        let top = crate::render::TEXT_TOP;
        let gutter_reserve = row_h * 3.0 + 8.0;
        let avail_h = height as f32 - gutter_reserve - top;
        let max_rows = if row_h > 0.0 {
            (avail_h / row_h).floor().max(0.0) as usize
        } else {
            0
        };
        if max_rows == 0 {
            return None;
        }
        // FOLLOW: keep the current heading (or the top, if the caret sits above the
        // first heading) inside the visible window on a long document.
        let len = self.outline_headings.len();
        let sel = self.outline_current().unwrap_or(0);
        let (win_top, count) = super::scroll_window(len, sel, 0, max_rows);
        let current = self.outline_current();
        let lines = self.outline_headings[win_top..win_top + count]
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let idx = win_top + i;
                let label = rowlayout::fit_primary(&h.label(), avail_chars);
                (label, current == Some(idx))
            })
            .collect();
        Some(OutlineLayout { left, avail, top, lines })
    }

    /// Shape + upload the persistent margin OUTLINE: a quiet table-of-contents in the
    /// TOP-LEFT margin — one dim line per heading (LABEL size), the CURRENT section a
    /// value rung brighter (MUTED) over the FAINT rest (figure/ground by value only,
    /// NO amber per DESIGN §4). Indented per heading level (via [`crate::markdown::Heading::label`]).
    /// HIDDEN (off / non-page / non-md / heading-free / too-narrow / no room) => empty
    /// text parked off-screen, so a default/off capture stays byte-identical.
    pub(in crate::render) fn prepare_outline(
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
        // Scale BOTH font size and line height to LABEL so the rows nest tightly
        // (this buffer is standalone, not row-aligned to the doc — like the gutter).
        self.outline_buffer.set_metrics(
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
        // Hidden: empty text parked off-screen, so nothing draws and an off / non-page
        // / non-markdown capture stays byte-identical.
        let Some(layout) = self.outline_layout(height) else {
            self.outline_buffer
                .set_size(&mut self.font_system, Some(1.0), Some(m.line_height));
            self.outline_buffer.set_text(
                &mut self.font_system,
                "",
                &base.clone().color(faint),
                Shaping::Advanced,
                None,
            );
            self.outline_buffer
                .shape_until_scroll(&mut self.font_system, false);
            let area = TextArea {
                buffer: &self.outline_buffer,
                left: 0.0,
                top: -1000.0,
                scale: 1.0,
                bounds,
                default_color: faint,
                custom_glyphs: &[],
            };
            self.outline_renderer
                .prepare(
                    device,
                    queue,
                    &mut self.font_system,
                    &mut self.atlas,
                    &self.viewport,
                    [area],
                    &mut self.swash_cache,
                )
                .map_err(|e| anyhow::anyhow!("glyphon outline prepare failed: {e:?}"))?;
            return Ok(());
        };
        // Each visible heading is ALREADY fit to one line by `outline_layout` (through
        // the shared `rowlayout::fit_primary` door), so this box NEVER lays raw,
        // possibly-overflowing text into a wrapping width. Join the lines with a
        // leading newline each (after the first) and colour each by its current flag:
        // the CURRENT heading MUTED, the rest FAINT.
        let n = layout.lines.len();
        let pieces: Vec<(String, glyphon::Color)> = layout
            .lines
            .iter()
            .enumerate()
            .map(|(i, (text, current))| {
                let color = if *current { muted } else { faint };
                let line = if i == 0 { text.clone() } else { format!("\n{text}") };
                (line, color)
            })
            .collect();
        let spans: Vec<(&str, Attrs)> = pieces
            .iter()
            .map(|(t, c)| (t.as_str(), base.clone().color(*c)))
            .collect();
        self.outline_buffer.set_size(
            &mut self.font_system,
            Some(layout.avail),
            Some(m.line_height * label * n as f32 + 1.0),
        );
        let default_attrs = base.clone().color(faint);
        // Default LEFT alignment (None) — top-left, hugging the margin from `left`.
        self.outline_buffer.set_rich_text(
            &mut self.font_system,
            spans,
            &default_attrs,
            Shaping::Advanced,
            None,
        );
        self.outline_buffer
            .shape_until_scroll(&mut self.font_system, false);
        let area = TextArea {
            buffer: &self.outline_buffer,
            left: layout.left,
            top: layout.top,
            scale: 1.0,
            bounds,
            default_color: faint,
            custom_glyphs: &[],
        };
        self.outline_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon outline prepare failed: {e:?}"))?;
        Ok(())
    }

    /// The persistent margin OUTLINE's DRAWN lines for tests: `Some(lines)` EXACTLY
    /// when the outline is drawn (the same gate + FOLLOW slice as
    /// [`Self::prepare_outline`]), each `(label, is_current)` as painted — the label
    /// already fit to one line, the flag marking the MUTED current row among the
    /// FAINT rest. `None` whenever the outline hides (off / non-page / non-md /
    /// heading-free / margin below the floor / no vertical room). Shares the ONE
    /// `outline_layout` owner with the pixels, so a test can never assert a state the
    /// frame doesn't draw. Test-only: the capture sidecar's `outline` block reports the
    /// FULL heading list + current (`Self::outline_report`), not the followed slice.
    #[cfg(test)]
    pub fn outline_draw_report(&self, height: u32) -> Option<Vec<(String, bool)>> {
        self.outline_layout(height).map(|l| l.lines)
    }
}
