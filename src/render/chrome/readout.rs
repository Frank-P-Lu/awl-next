//! CORNER READOUTS chrome — the ONE shared corner-label body
//! ([`TextPipeline::prepare_corner_label`], `pub(super)` so the debug panel in
//! [`super::debug_text`] rides it too) plus the bottom-right word-count / reading-time
//! readout, the bottom-center calm notice, and the page-width drag readout that ride
//! it. Carved out of `chrome.rs` verbatim, no behaviour change. See [`super`].

use super::*;

/// The (left, top) device-px origin of a non-empty corner label, given its widest
/// shaped run width `text_w`, its `line_height`, the canvas `width`/`height`, and the
/// writing column's `col_left`/`col_width`. The ONE owner of the corner-anchor
/// placement math — split out of [`TextPipeline::prepare_corner_label`] so each anchor
/// is unit-testable without a GPU (the empty-text off-screen park stays in the caller).
/// An 8px inset from the canvas edges for the docked corners; a small clamped float
/// for the at-pointer readout.
pub(in crate::render) fn corner_origin(
    anchor: CornerAnchor,
    text_w: f32,
    line_height: f32,
    width: f32,
    height: f32,
    col_left: f32,
    col_width: f32,
) -> (f32, f32) {
    match anchor {
        CornerAnchor::TopLeft => (col_left.max(8.0), 8.0),
        // Right-aligned to the CANVAS edge (8px inset), top row — clear of the top-left
        // margin the persistent outline owns. Never off the left edge on a tiny canvas.
        CornerAnchor::TopRight => ((width - text_w - 8.0).max(8.0), 8.0),
        CornerAnchor::BottomRight => {
            let left = (col_left + col_width - text_w).max(col_left);
            (left, height - line_height - 8.0)
        }
        CornerAnchor::BottomCenter => {
            let left = (col_left + (col_width - text_w) * 0.5).max(col_left);
            (left, height - line_height - 8.0)
        }
        CornerAnchor::AtPoint(px, py) => {
            // Float above-right of the pointer (clears the resize-cursor glyph it sits
            // over), clamped onto the canvas so it never clips off an edge.
            let left = (px + 14.0).min(width - text_w - 4.0).max(4.0);
            let top = (py - line_height - 10.0).max(4.0);
            (left, top)
        }
    }
}

impl TextPipeline {
    /// Shape one quiet corner label into `buffer` and `prepare` it into `renderer`,
    /// parking it off-screen when `text` is empty. This is the shared body behind the
    /// bottom-right word-count readout and the top-left DEBUG panel — each was a
    /// ~95%-identical copy differing only by the (renderer, buffer) pair, the text,
    /// the corner [`CornerAnchor`], and (for the debug panel) the metrics + row count.
    ///
    /// It takes `renderer` + `buffer` (and the four shared glyphon resources) as
    /// EXPLICIT `&mut` params rather than `&mut self`: the callers pass distinct
    /// fields, so a `&mut self` method couldn't also hand it `&mut
    /// self.wordcount_renderer`. `col_left` / `col_width` are the writing column's
    /// already-resolved geometry (so this stays free of `self`); `col_width` is only
    /// consulted for the right-aligned anchor. `gm` sets the buffer's glyph metrics (so
    /// a compact panel can ride a smaller size) and `rows` reserves that many
    /// line-heights of height so a STACKED multi-line label (the debug panel) shapes
    /// without clipping; a single-line label passes `rows == 1.0`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_corner_label(
        renderer: &mut TextRenderer,
        buffer: &mut GlyphBuffer,
        font_system: &mut FontSystem,
        atlas: &mut TextAtlas,
        viewport: &Viewport,
        swash_cache: &mut SwashCache,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        gm: GlyphMetrics,
        rows: f32,
        col_left: f32,
        col_width: f32,
        text: &str,
        anchor: CornerAnchor,
        label: &str,
    ) -> anyhow::Result<()> {
        let muted = theme::muted().to_glyphon();
        let line_height = gm.line_height;
        buffer.set_metrics(font_system, gm);
        buffer.set_size(font_system, Some(width as f32), Some(line_height * rows.max(1.0)));
        buffer.set_text(font_system, text, &panel_attrs().color(muted), Shaping::Advanced, None);
        buffer.shape_until_scroll(font_system, false);
        // Empty text parks the label off-screen so nothing draws (and a default
        // capture stays byte-identical). Otherwise measure the widest shaped run once
        // and hand the placement to the pure `corner_origin` owner.
        let (left, top) = if text.is_empty() {
            (0.0, -1000.0)
        } else {
            let mut text_w = 0.0_f32;
            for run in buffer.layout_runs() {
                text_w = text_w.max(run.line_w);
            }
            corner_origin(anchor, text_w, line_height, width as f32, height as f32, col_left, col_width)
        };
        let bounds = TextBounds { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        let area = TextArea {
            buffer,
            left,
            top,
            scale: 1.0,
            bounds,
            default_color: muted,
            custom_glyphs: &[],
        };
        renderer
            .prepare(device, queue, font_system, atlas, viewport, [area], swash_cache)
            .map_err(|e| anyhow::anyhow!("glyphon {label} prepare failed: {e:?}"))?;
        Ok(())
    }

    /// The word count of the current buffer (whitespace-separated tokens). Summed
    /// per line — a word never spans a newline — so it equals
    /// [`crate::markdown::word_count`] of the whole document without joining it.
    /// EXCLUDES a leading frontmatter block ([`crate::markdown::frontmatter_end`])
    /// — metadata, not manuscript, so a `lang:`/`title:` line never inflates the
    /// reading-time readout.
    fn word_count(&self) -> usize {
        let fm_end = crate::markdown::frontmatter_end(&self.md_spans);
        let mut start = 0usize;
        let mut total = 0usize;
        for line in &self.buffer.lines {
            let text = line.text();
            if fm_end.is_none_or(|end| start >= end) {
                total += crate::markdown::word_count(text);
            }
            start += text.len() + 1;
        }
        total
    }

    /// The QUIET readout for a MARKDOWN buffer: `Some((words, reading_minutes))` when
    /// the buffer is markdown and has at least one word, else `None` (nothing drawn).
    /// Exposed so the capture sidecar can report exactly what the readout shows.
    pub fn readout_report(&self) -> Option<(usize, usize)> {
        if !self.md_enabled {
            return None;
        }
        let words = self.word_count();
        if words == 0 {
            return None;
        }
        Some((words, crate::markdown::reading_time_min(words)))
    }

    /// The readout string for the bottom-right corner, e.g. `"240 words · 2 min"`.
    /// Empty when there is nothing to show (non-markdown or wordless).
    ///
    /// REUSED by the held HUD's WORD COUNT figure (phase 2): the persistent
    /// bottom-right readout is no longer drawn, but this text-feeder +
    /// [`Self::readout_report`] (the sidecar source) live on as the HUD's source.
    pub(super) fn wordcount_text(&self) -> String {
        match self.readout_report() {
            Some((w, m)) => {
                let unit = if w == 1 { "word" } else { "words" };
                format!("{w} {unit} · {m} min")
            }
            None => String::new(),
        }
    }

    /// Shape + upload the quiet word-count / reading-time readout. Drawn DIM and
    /// RIGHT-aligned to the writing column's right edge, on the bottom row. Empty text
    /// parks it off-screen (markdown gate / empty doc), so a non-markdown buffer draws
    /// nothing and stays byte-identical.
    ///
    /// RETAINED (unused) for phase 2: the persistent readout was removed from the
    /// chrome layer (it moves into the held HUD); this shaper stays for that reuse.
    #[allow(dead_code)]
    pub(in crate::render) fn prepare_wordcount(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let text = self.wordcount_text();
        let (gm, col_left, col_width) =
            (self.metrics.glyph_metrics(), self.column_left(), self.column_width());
        Self::prepare_corner_label(
            &mut self.wordcount_renderer,
            &mut self.wordcount_buffer,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            &mut self.swash_cache,
            device,
            queue,
            width,
            height,
            gm,
            1.0,
            col_left,
            col_width,
            &text,
            CornerAnchor::BottomRight,
            "wordcount",
        )
    }

    /// Shape + upload the CALM NOTICE — one quiet LABEL-sized line in the muted
    /// ink at the BOTTOM-CENTER of the writing column (today: the autosave
    /// clobber guard's "changed on disk outside awl — autosave held"). Mirrors
    /// [`Self::prepare_wordcount`] through the shared corner-label body; an
    /// EMPTY notice parks it off-screen, so every capture (which can never have
    /// a notice — autosave is live-only) stays byte-identical.
    pub(in crate::render) fn prepare_notice(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let text = self.notice.clone();
        let m = self.metrics;
        let label = crate::markdown::type_scale::LABEL;
        let gm = GlyphMetrics::new(m.font_size * label, m.line_height * label);
        let (col_left, col_width) = (self.column_left(), self.column_width());
        Self::prepare_corner_label(
            &mut self.notice_renderer,
            &mut self.notice_buffer,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            &mut self.swash_cache,
            device,
            queue,
            width,
            height,
            gm,
            1.0,
            col_left,
            col_width,
            &text,
            CornerAnchor::BottomCenter,
            "notice",
        )
    }

    /// Shape + upload the PAGE-WIDTH DRAG READOUT: a quiet muted char-count (e.g.
    /// "68") floating near the pointer while a page-column edge drag is in
    /// progress — Butterick's line-length rule made visible (value-only ink, NEVER
    /// amber — DESIGN §3). Mirrors [`Self::prepare_notice`]'s corner-label body but
    /// anchors AT the pointer ([`CornerAnchor::AtPoint`]) instead of a canvas
    /// corner. `page_drag_readout` is `None` (not dragging — the ONLY state a
    /// headless capture can ever see) parks it off-screen, so every capture stays
    /// byte-identical.
    pub(in crate::render) fn prepare_page_drag_readout(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let (text, anchor) = match self.page_drag_readout {
            Some((px, py, measure)) => (measure.to_string(), CornerAnchor::AtPoint(px, py)),
            None => (String::new(), CornerAnchor::AtPoint(0.0, 0.0)),
        };
        let m = self.metrics;
        let label = crate::markdown::type_scale::LABEL;
        let gm = GlyphMetrics::new(m.font_size * label, m.line_height * label);
        let (col_left, col_width) = (self.column_left(), self.column_width());
        Self::prepare_corner_label(
            &mut self.page_drag_renderer,
            &mut self.page_drag_buffer,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            &mut self.swash_cache,
            device,
            queue,
            width,
            height,
            gm,
            1.0,
            col_left,
            col_width,
            &text,
            anchor,
            "page_drag_readout",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::corner_origin;
    use crate::render::CornerAnchor;

    /// THE DEBUG PANEL is TOP-RIGHT: right-aligned to the CANVAS edge (8px inset),
    /// top row — clear of the top-left margin the persistent outline now owns.
    #[test]
    fn debug_panel_anchors_top_right() {
        // Canvas 1000 wide, a 200px-wide block: its right edge sits 8px in from the
        // canvas edge (left = 1000 − 200 − 8 = 792), top row 8px down.
        let (left, top) = corner_origin(CornerAnchor::TopRight, 200.0, 18.0, 1000.0, 800.0, 0.0, 0.0);
        assert!((left - 792.0).abs() < 1e-3, "right edge hugs the canvas edge, got left={left}");
        assert_eq!(top, 8.0, "the top row sits 8px down");
        // The block's right edge is a fixed 8px inset regardless of its width.
        let (l2, _) = corner_origin(CornerAnchor::TopRight, 350.0, 18.0, 1000.0, 800.0, 0.0, 0.0);
        assert!((l2 + 350.0 - (1000.0 - 8.0)).abs() < 1e-3, "right edge is width−8 for any block width");
        // On a canvas too narrow for the block it never runs off the LEFT edge.
        let (l3, _) = corner_origin(CornerAnchor::TopRight, 500.0, 18.0, 300.0, 800.0, 0.0, 0.0);
        assert_eq!(l3, 8.0, "clamps to the left inset on a tiny canvas");
    }

    /// The docked corners keep their historical placement (TopRight is the only new
    /// arm; the others are byte-identical to the pre-extraction inline math).
    #[test]
    fn docked_corners_keep_their_placement() {
        // Top-left: at the column left, floored to the 8px margin.
        assert_eq!(corner_origin(CornerAnchor::TopLeft, 100.0, 18.0, 1000.0, 800.0, 0.0, 0.0), (8.0, 8.0));
        // Bottom-right: right-aligned to the writing COLUMN (col_left + col_width − w).
        let (l, t) = corner_origin(CornerAnchor::BottomRight, 120.0, 18.0, 1000.0, 800.0, 100.0, 600.0);
        assert!((l - (100.0 + 600.0 - 120.0)).abs() < 1e-3);
        assert_eq!(t, 800.0 - 18.0 - 8.0);
    }
}
