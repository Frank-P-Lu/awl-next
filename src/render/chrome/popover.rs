//! THE FORMAT POPOVER chrome — the reveal-on-select floating button row. Given a
//! [`crate::popover::PopoverModel`] (from [`ViewState::popover`]) + the live
//! selection, this shapes the button labels, measures their real glyph spans,
//! lays out a small elevated card ANCHORED just above (or below) the selection's
//! first endpoint, and uploads: the RIMMED float elevation (raised border →
//! `base_300` card, NO drop shadow — see [`FloatElevation::Rimmed`], the "fat
//! chin" cure), a `base_200` value-step wash behind each LIT button, and the
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
    /// The label buffer's upload origin (TextArea `top`), chosen so the button
    /// glyphs' ink band lands exactly `VPAD` below the card top.
    pub text_top: f32,
    /// Top of the button glyphs' actual ink band, absolute px (`card_y + VPAD`) —
    /// the reference the lit-button washes hug.
    pub band_top: f32,
    /// Height of that ink band (tallest glyph's ink top → lowest glyph's ink
    /// bottom). The card is `band_h + 2·VPAD`, so it hugs the row with uniform pad.
    pub band_h: f32,
    pub buttons: Vec<PopoverButtonGeom>,
}

/// Inner horizontal pad from the card edge to the first/last button glyph.
const HPAD: f32 = 12.0;
/// Inner vertical pad above/below the button GLYPH INK BAND — the ONE pad token
/// the card hugs the row with (uniform top and bottom). Exposed `pub(crate)` (re-
/// exported as `render::POPOVER_VPAD`) so the card-fits law asserts against the
/// SAME token the layout uses, never a hand-copied literal.
pub(crate) const VPAD: f32 = 7.0;
/// The two-space separator shaped BETWEEN buttons — its width becomes the visible
/// inter-button gap (the hit-test splits it at the midpoint, so no dead zone).
const SEP: &str = "   ";
/// Breath between the card and the selection it points at.
const ANCHOR_GAP: f32 = 8.0;

/// SELF-DEMONSTRATING label attrs: the per-button `Attrs` transform that makes a
/// button PREVIEW ITS OWN EFFECT — the same transforms the document's `md_attrs`
/// applies to real content, minus color (lit/unlit ink stays the state signal,
/// applied by the caller). `B` shapes in the world's real bundled 700 face, `I`
/// in the italic style, `C` in the registered monospace family (the same
/// `Family::Monospace` resolution the doc's inline code rides). Every other
/// button keeps the plain panel face — `A`'s highlight wash, `S`'s strike line
/// and `C`'s pill are QUADS built in [`TextPipeline::prepare_popover`], not text
/// transforms. Pure (unit-tested below).
fn demo_attrs(button: PopoverButton, base: &Attrs<'static>) -> Attrs<'static> {
    match button {
        PopoverButton::Bold => base.clone().weight(glyphon::Weight::BOLD),
        PopoverButton::Italic => base.clone().style(glyphon::Style::Italic),
        PopoverButton::Code => base.clone().family(Family::Monospace),
        PopoverButton::Highlight
        | PopoverButton::Strike
        | PopoverButton::Heading
        | PopoverButton::Link => base.clone(),
    }
}

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
                // Float elevation via the SAME primitive every summoned micro-panel
                // rides — but RIMMED (border + card, no shadow): the drop-shadow quad
                // hung a hard-edged ~9px slab below this two-line-height card's rim,
                // out-massing its own 7px pad — the LIVE "fat chin" that survived the
                // card-hug fix, at every scale (the card rect measured tight while
                // the slab painted OUTSIDE it). See [`FloatElevation::Rimmed`].
                set_float_quads(
                    &mut self.popover_shadow,
                    &mut self.popover_border,
                    &mut self.popover_card,
                    device,
                    queue,
                    width,
                    height,
                    Some(geom.card),
                    FloatElevation::Rimmed,
                );
                // A value-step wash behind each LIT button (never amber) — a pill
                // hugging the glyph ink band with a small halo (the card hugs the same
                // band, so the wash never floats in dead space).
                let mut washes: Vec<[f32; 4]> = geom
                    .buttons
                    .iter()
                    .filter(|b| b.active)
                    .map(|b| {
                        let hpad = 4.0;
                        let vpad = 3.0;
                        [
                            b.x0 - hpad,
                            geom.band_top - vpad,
                            (b.x1 - b.x0) + 2.0 * hpad,
                            geom.band_h + 2.0 * vpad,
                        ]
                    })
                    .collect();
                // SELF-DEMONSTRATING labels — the quad half (the FACE half is
                // [`demo_attrs`]): `C` always sits in the inline-code pill wash
                // (the SAME `base_200` value-step tint the doc pill carries —
                // `popover_wash` shares it, so the pill rides that pipeline), `A`
                // in the real `==highlight==` wash (`popover_hl_wash`, tinted by
                // the doc wash's own derivation), and `S` carries a strike line
                // from THE ONE OWNER (`strike_line_band` — the same fn the
                // document's `~~strike~~` quads read, so the demo IS the
                // effect). TRIPWIRE (`popover_card_hugs_the_button_row`): every
                // demo quad stays INSIDE the measured glyph ink band — the pills
                // span exactly `[band_top, band_top + band_h]` (no vertical
                // halo, unlike the lit wash above, which the law excludes via
                // its lit filter) and the strike band is centered within it, so
                // the card-hug measurement never widens.
                let band_pill = |b: &PopoverButtonGeom| {
                    let hpad = CODE_PILL_INSET_X;
                    [b.x0 - hpad, geom.band_top, (b.x1 - b.x0) + 2.0 * hpad, geom.band_h]
                };
                let mut hl_pills: Vec<[f32; 4]> = Vec::new();
                let mut strikes: Vec<Squiggle> = Vec::new();
                for b in &geom.buttons {
                    match b.button {
                        PopoverButton::Code => washes.push(band_pill(b)),
                        PopoverButton::Highlight => hl_pills.push(band_pill(b)),
                        PopoverButton::Strike => {
                            let (y, h, stroke) =
                                strike_line_band(geom.band_top, geom.band_h, self.metrics.zoom);
                            strikes.push(Squiggle {
                                x: b.x0 - 1.0,
                                y,
                                w: (b.x1 - b.x0) + 2.0,
                                h,
                                amp: 0.0,    // flat — a strike is a calm straight line
                                period: 1.0, // unused at amp 0 (kept > 0, shader-safe)
                                thickness: stroke,
                            });
                        }
                        _ => {}
                    }
                }
                self.popover_wash.prepare(device, queue, width, height, &washes);
                self.popover_hl_wash.prepare(device, queue, width, height, &hl_pills);
                self.popover_strike.prepare(device, queue, width, height, &strikes);
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
                    FloatElevation::Rimmed,
                );
                self.popover_wash.prepare(device, queue, width, height, &[]);
                self.popover_hl_wash.prepare(device, queue, width, height, &[]);
                self.popover_strike.prepare(device, queue, width, height, &[]);
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
            // Lit buttons draw full ink; unlit recede to muted (state without
            // amber). The FACE previews the button's own effect ([`demo_attrs`]:
            // B bold, I italic, C mono).
            spans.push((
                b.label.clone(),
                demo_attrs(b.button, &base).color(if b.active { ink } else { muted }),
            ));
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

        // Measure each button's glyph span (relative to the buffer origin x=0), and
        // collect (glyph, baseline) for the VERTICAL ink-band measurement below. The
        // "fat chin" was `card_h = line_height + 2·VPAD` with the glyphs top-anchored:
        // the line's descent + leading sat as a band of dead card BELOW the row. The
        // card must hug the buttons' ACTUAL ink, not the leading-inflated line box.
        let mut spans_px: Vec<(f32, f32)> = vec![(f32::MAX, f32::MIN); model.buttons.len()];
        let mut total_w = 0.0f32;
        let mut ink_glyphs: Vec<(CacheKey, f32)> = Vec::new();
        for run in self.popover_buffer.layout_runs() {
            for g in run.glyphs.iter() {
                total_w = total_w.max(g.x + g.w);
                // Baseline (`run.line_y`, relative to the buffer origin) travels with
                // each glyph so the ink pass below can place it — the SAME convention
                // the morph caret + placard-stipple masks read.
                ink_glyphs.push((g.physical((0.0, 0.0), 1.0).cache_key, run.line_y));
                for (bi, &(bs, be)) in ranges.iter().enumerate() {
                    if g.start >= bs && g.start < be {
                        let e = &mut spans_px[bi];
                        e.0 = e.0.min(g.x);
                        e.1 = e.1.max(g.x + g.w);
                    }
                }
            }
        }

        // The buttons' ACTUAL ink band, relative to the buffer origin (y=0):
        // `baseline − placement.top` is a glyph's ink top, `+ placement.height` its
        // ink bottom (deterministic in captures — pure shaping + rasterization, the
        // same swash placement the placard stipple reads). The card hugs THIS.
        let (mut band_top_rel, mut band_bot_rel) = (f32::MAX, f32::MIN);
        {
            let Self {
                swash_cache,
                font_system,
                ..
            } = self;
            for (key, baseline_rel) in ink_glyphs {
                let Some(img) = swash_cache.get_image(font_system, key).as_ref() else {
                    continue;
                };
                if img.placement.height == 0 || img.content != SwashContent::Mask {
                    continue;
                }
                let top = baseline_rel - img.placement.top as f32;
                band_top_rel = band_top_rel.min(top);
                band_bot_rel = band_bot_rel.max(top + img.placement.height as f32);
            }
        }
        // Degenerate fallback (no measurable ink — never for the real roster): hug the
        // full line box, byte-identical to the pre-fix behavior.
        if band_bot_rel <= band_top_rel {
            band_top_rel = 0.0;
            band_bot_rel = m.line_height;
        }
        let band_h = band_bot_rel - band_top_rel;

        let card_w = total_w + 2.0 * HPAD;
        let card_h = band_h + 2.0 * VPAD;

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
        // The glyph ink band sits a uniform `VPAD` below the card top; the label
        // buffer uploads at an origin chosen so `band_top_rel` lands exactly there.
        let band_top = card_y + VPAD;
        let text_top = band_top - band_top_rel;

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
            text_top,
            band_top,
            band_h,
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
            top: geom.text_top,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// SELF-DEMONSTRATING labels, the FACE half at its purest seam: each button's
    /// label attrs carry exactly the transform its Action applies to real content
    /// — B a real 700 weight, I the italic style, C the monospace family — and
    /// every other button keeps the base face untouched (their demos are QUADS,
    /// not text transforms). No-wildcard over the roster, so a new button must
    /// take a conscious stance here.
    #[test]
    fn demo_attrs_previews_each_buttons_own_effect() {
        let base = panel_attrs();
        for &b in crate::popover::ALL {
            let a = demo_attrs(b, &base);
            match b {
                PopoverButton::Bold => {
                    assert_eq!(a.weight, glyphon::Weight::BOLD, "B previews bold");
                    assert_eq!(a.style, base.style);
                }
                PopoverButton::Italic => {
                    assert_eq!(a.style, glyphon::Style::Italic, "I previews italic");
                    assert_eq!(a.weight, base.weight);
                }
                PopoverButton::Code => {
                    assert_eq!(a.family, Family::Monospace, "C previews mono");
                    assert_eq!(a.weight, base.weight);
                }
                PopoverButton::Highlight
                | PopoverButton::Strike
                | PopoverButton::Heading
                | PopoverButton::Link => {
                    assert_eq!(a.weight, base.weight, "{b:?} keeps the panel face");
                    assert_eq!(a.style, base.style, "{b:?} keeps the panel face");
                    assert_eq!(a.family, base.family, "{b:?} keeps the panel face");
                }
            }
        }
    }

    /// The `S` button's strike line rides THE ONE OWNER (`strike_line_band`) —
    /// the same fn the document's `~~strike~~` quads read — so its band always
    /// sits strictly INSIDE the glyph ink band the card hugs (the
    /// `popover_card_hugs_the_button_row` tripwire: a demo element outside the
    /// band would widen the measured pad and regress the law).
    #[test]
    fn strike_demo_band_stays_inside_the_ink_band() {
        let (band_top, band_h) = (100.0_f32, 14.0_f32);
        for zoom in [1.0_f32, 2.0] {
            let (y, h, stroke) = strike_line_band(band_top, band_h, zoom);
            assert!(y > band_top, "strike band starts below the ink top");
            assert!(y + h < band_top + band_h, "strike band ends above the ink bottom");
            assert!(stroke > 0.0 && stroke <= h, "stroke fits its band");
            // Centered at the owner's fraction of the band.
            let center = y + h * 0.5;
            let expect = band_top + band_h * STRIKE_V_FRAC;
            assert!((center - expect).abs() < 1e-4, "centered at STRIKE_V_FRAC");
        }
    }
}
