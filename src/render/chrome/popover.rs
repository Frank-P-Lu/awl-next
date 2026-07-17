//! THE FORMAT POPOVER chrome — the reveal-on-select floating button row. Given a
//! [`crate::popover::PopoverModel`] (from [`ViewState::popover`]) + the live
//! selection, this shapes the button labels, measures their real glyph spans,
//! lays out a small elevated card ANCHORED just above (or below) the selection's
//! first endpoint, and uploads: the float elevation (shadow → raised border →
//! `base_300` card), a `base_200` value-step wash behind each LIT button, and the
//! button labels themselves. Parked (nothing drawn) when the model is `None`, so a
//! popover-down frame is byte-identical.
//!
//! The laid-out geometry is stashed in `popover_geom` so the pure `&self`
//! [`TextPipeline::popover_hit`] (click + cursor-shape) and the sidecar read the
//! SAME spans the buttons draw from — a click can never disagree with paint.

use super::*;
use crate::popover::PopoverButton;

/// One button's laid-out pixel span (absolute, physical px) + its lit state.
#[derive(Clone, Debug)]
pub(in crate::render) struct PopoverButtonGeom {
    pub button: PopoverButton,
    pub x0: f32,
    pub x1: f32,
    pub active: bool,
    pub label: String,
}

/// The popover's whole laid-out geometry for a frame: the card rect + each
/// button's pixel span. Read by the hit-test + the sidecar.
#[derive(Clone, Debug)]
pub(in crate::render) struct PopoverGeom {
    pub card: [f32; 4],
    pub row_top: f32,
    pub row_h: f32,
    pub buttons: Vec<PopoverButtonGeom>,
}

/// Inner horizontal pad from the card edge to the first/last button glyph.
const HPAD: f32 = 12.0;
/// Inner vertical pad above/below the button row.
const VPAD: f32 = 7.0;
/// The two-space separator shaped BETWEEN buttons — its width becomes the visible
/// inter-button gap (the hit-test splits it at the midpoint, so no dead zone).
const SEP: &str = "   ";
/// Breath between the card and the selection it points at.
const ANCHOR_GAP: f32 = 8.0;

impl TextPipeline {
    /// Build + upload the format popover for this frame, or park it. Mirrors the
    /// which-key panel's shape (own float trio + text renderer), drawn in
    /// `draw_chrome_tail`. Reads `self.popover_model` (mirrored from the view) +
    /// `self.selection` (the anchor). See the module doc.
    pub(in crate::render) fn prepare_popover(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let geom = self.popover_layout(width, height);
        match geom {
            Some(geom) => {
                // Float elevation (shadow → raised border → card), the SAME primitive
                // every summoned micro-panel rides.
                set_float_quads(
                    &mut self.popover_shadow,
                    &mut self.popover_border,
                    &mut self.popover_card,
                    device,
                    queue,
                    width,
                    height,
                    Some(geom.card),
                    true,
                );
                // A value-step wash behind each LIT button (never amber).
                let washes: Vec<[f32; 4]> = geom
                    .buttons
                    .iter()
                    .filter(|b| b.active)
                    .map(|b| {
                        let pad = 4.0;
                        [
                            b.x0 - pad,
                            geom.row_top - 1.0,
                            (b.x1 - b.x0) + 2.0 * pad,
                            geom.row_h + 2.0,
                        ]
                    })
                    .collect();
                self.popover_wash.prepare(device, queue, width, height, &washes);
                self.popover_upload_text(device, queue, width, height, &geom)?;
                self.popover_geom = Some(geom);
                Ok(())
            }
            None => {
                set_float_quads(
                    &mut self.popover_shadow,
                    &mut self.popover_border,
                    &mut self.popover_card,
                    device,
                    queue,
                    width,
                    height,
                    None,
                    true,
                );
                self.popover_wash.prepare(device, queue, width, height, &[]);
                self.park_popover_text(device, queue, width, height)?;
                self.popover_geom = None;
                Ok(())
            }
        }
    }

    /// Shape the button labels, measure each button's real glyph span, and lay out
    /// the card anchored above (or below, when there's no room) the selection's
    /// first endpoint. `None` when the popover is down / no selection to anchor to.
    fn popover_layout(&mut self, width: u32, height: u32) -> Option<PopoverGeom> {
        let model = self.popover_model.clone()?;
        if model.buttons.is_empty() {
            return None;
        }
        let ((line0, col0), _) = self.selection?;
        let m = self.metrics;

        // Compose the row string, recording each button's BYTE range so the shaped
        // glyph scan can attribute each glyph to its button (mirrors
        // `panel_case_toggle_span`'s real-advance read — no hardcoded pitch).
        let ink = theme::base_content().to_glyphon();
        let muted = theme::muted().to_glyphon();
        let base = panel_attrs();
        let mut spans: Vec<(String, glyphon::Attrs)> = Vec::new();
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        let mut row = String::new();
        for (i, b) in model.buttons.iter().enumerate() {
            if i > 0 {
                let start = row.len();
                row.push_str(SEP);
                spans.push((SEP.to_string(), base.clone().color(muted)));
                let _ = start;
            }
            let start = row.len();
            row.push_str(&b.label);
            let end = row.len();
            ranges.push((start, end));
            // Lit buttons draw full ink; unlit recede to muted (state without amber).
            spans.push((b.label.clone(), base.clone().color(if b.active { ink } else { muted })));
        }

        self.popover_buffer
            .set_metrics(&mut self.font_system, m.glyph_metrics());
        self.popover_buffer
            .set_size(&mut self.font_system, Some(width as f32 * 2.0), Some(m.line_height));
        self.popover_buffer.set_wrap(&mut self.font_system, Wrap::None);
        let default_attrs = base.clone().color(ink);
        let rich: Vec<(&str, glyphon::Attrs)> =
            spans.iter().map(|(s, a)| (s.as_str(), a.clone())).collect();
        self.popover_buffer.set_rich_text(
            &mut self.font_system,
            rich,
            &default_attrs,
            Shaping::Advanced,
            None,
        );
        self.popover_buffer
            .shape_until_scroll(&mut self.font_system, false);

        // Measure each button's glyph span (relative to the buffer origin x=0).
        let mut spans_px: Vec<(f32, f32)> = vec![(f32::MAX, f32::MIN); model.buttons.len()];
        let mut total_w = 0.0f32;
        for run in self.popover_buffer.layout_runs() {
            for g in run.glyphs.iter() {
                total_w = total_w.max(g.x + g.w);
                for (bi, &(bs, be)) in ranges.iter().enumerate() {
                    if g.start >= bs && g.start < be {
                        let e = &mut spans_px[bi];
                        e.0 = e.0.min(g.x);
                        e.1 = e.1.max(g.x + g.w);
                    }
                }
            }
        }

        let card_w = total_w + 2.0 * HPAD;
        let card_h = m.line_height + 2.0 * VPAD;

        // Anchor: the selection's first endpoint, in screen space.
        let sel_x = self.text_left() + self.col_x_and_advance(line0, col0).0;
        let sel_top = self.visual_row_top(line0, col0);
        let sel_row_h = self.row_height_px(self.visual_row_of(line0, col0));

        // Prefer ABOVE the selection; drop BELOW when there's no room.
        let mut card_y = sel_top - ANCHOR_GAP - card_h;
        if card_y < ANCHOR_GAP {
            card_y = sel_top + sel_row_h + ANCHOR_GAP;
        }
        // Clamp within the canvas (never off the bottom either).
        card_y = card_y.min(height as f32 - card_h - ANCHOR_GAP).max(ANCHOR_GAP);

        // Center horizontally over the selection start, clamped to the canvas.
        let pad = 6.0;
        let card_x = (sel_x - card_w * 0.5)
            .min(width as f32 - card_w - pad)
            .max(pad);

        let text_left = card_x + HPAD;
        let row_top = card_y + VPAD;

        let buttons = model
            .buttons
            .iter()
            .enumerate()
            .map(|(bi, b)| {
                let (rx0, rx1) = spans_px[bi];
                // A degenerate (unmeasured) span never happens for a non-empty label,
                // but stay safe: fall back to a thin cell at the card center.
                let (rx0, rx1) = if rx0 <= rx1 { (rx0, rx1) } else { (0.0, total_w) };
                PopoverButtonGeom {
                    button: b.button,
                    x0: text_left + rx0,
                    x1: text_left + rx1,
                    active: b.active,
                    label: b.label.clone(),
                }
            })
            .collect();

        Some(PopoverGeom {
            card: [card_x, card_y, card_w, card_h],
            row_top,
            row_h: m.line_height,
            buttons,
        })
    }

    /// Upload the shaped button labels over the card.
    fn popover_upload_text(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        geom: &PopoverGeom,
    ) -> anyhow::Result<()> {
        let ink = theme::base_content().to_glyphon();
        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let area = TextArea {
            buffer: &self.popover_buffer,
            left: geom.card[0] + HPAD,
            top: geom.row_top,
            scale: 1.0,
            bounds,
            default_color: ink,
            custom_glyphs: &[],
        };
        self.popover_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon popover prepare failed: {e:?}"))?;
        Ok(())
    }

    /// Park the popover label renderer off-screen (empty buffer at y=-1000), so a
    /// popover-down frame draws nothing. Mirrors `park_preview_text`.
    fn park_popover_text(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let content = theme::base_content().to_glyphon();
        self.popover_buffer
            .set_size(&mut self.font_system, Some(1.0), Some(self.metrics.line_height));
        self.popover_buffer.set_text(
            &mut self.font_system,
            "",
            &panel_attrs().color(content),
            Shaping::Advanced,
            None,
        );
        self.popover_buffer
            .shape_until_scroll(&mut self.font_system, false);
        let bounds = TextBounds { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        let area = TextArea {
            buffer: &self.popover_buffer,
            left: 0.0,
            top: -1000.0,
            scale: 1.0,
            bounds,
            default_color: content,
            custom_glyphs: &[],
        };
        self.popover_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon popover park failed: {e:?}"))?;
        Ok(())
    }

    /// Hit-test a physical pointer against the summoned format popover: the button
    /// whose span (extended to the midpoints of the inter-button gaps) contains
    /// `px`, when `py` is within the card. `None` off the card / popover down. Pure
    /// `&self` — reads the stashed `popover_geom` (the SAME layout the buttons draw
    /// from), so a click never disagrees with paint. The caller fires
    /// `button.action()` through `App::apply`.
    pub fn popover_hit(&self, px: f32, py: f32) -> Option<PopoverButton> {
        let geom = self.popover_geom.as_ref()?;
        let [cx, cy, cw, ch] = geom.card;
        if px < cx || px > cx + cw || py < cy || py > cy + ch {
            return None;
        }
        // Extend each button's clickable span to the midpoint of the gap to its
        // neighbours (and to the card edges at the ends), so the inter-button gaps
        // are not dead zones.
        let n = geom.buttons.len();
        for (i, b) in geom.buttons.iter().enumerate() {
            let lo = if i == 0 {
                cx
            } else {
                (geom.buttons[i - 1].x1 + b.x0) * 0.5
            };
            let hi = if i + 1 == n {
                cx + cw
            } else {
                (b.x1 + geom.buttons[i + 1].x0) * 0.5
            };
            if px >= lo && px <= hi {
                return Some(b.button);
            }
        }
        None
    }

    /// Whether the physical pointer is anywhere over the popover CARD (for the
    /// pointing-hand cursor + swallowing a press that isn't on a button). `false`
    /// when the popover is down.
    pub fn over_popover(&self, px: f32, py: f32) -> bool {
        match self.popover_geom.as_ref() {
            Some(g) => {
                let [cx, cy, cw, ch] = g.card;
                px >= cx && px <= cx + cw && py >= cy && py <= cy + ch
            }
            None => false,
        }
    }

    /// Headless report for the sidecar `popover` block: `(card_rect, rows)` where
    /// each row is `(label, active, [x0, x1])`, or `None` when the popover is down.
    /// Reads the same stashed geometry the buttons draw + the hit-test reads.
    pub fn popover_report(&self) -> Option<([f32; 4], Vec<(String, bool, [f32; 2])>)> {
        let g = self.popover_geom.as_ref()?;
        let rows = g
            .buttons
            .iter()
            .map(|b| (b.label.clone(), b.active, [b.x0, b.x1]))
            .collect();
        Some((g.card, rows))
    }
}
