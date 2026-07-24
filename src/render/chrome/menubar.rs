//! WEB/LINUX MENU BAR render — the slim awl-drawn strip of menu titles across the
//! top of the canvas (the third door to actions where the OS gives no chrome), plus
//! its anchored dropdown. Inherent methods on [`super::TextPipeline`], mirroring the
//! [`gutter`](super::gutter) / [`outline`](super::outline) chrome machinery: standalone
//! glyph buffers shaped at LABEL scale, parked off-screen when hidden, so a default
//! (macOS — bar off) capture stays byte-identical. See [`crate::menubar`] for the
//! process-globals + the PURE layout math this feeds and reads back, and [`super`].
//!
//! **THEME-DERIVED, NEVER amber (DESIGN §3/§4 — figure/ground by value):** the bar
//! ground is a value step off the room (`base_200`); titles are FAINT ink, the OPEN
//! one MUTED over the muted `selection` highlight band (never amber); the dropdown is the overlay
//! float-card pattern (`base_300` risen a step) with `base_content` item labels and
//! FAINT native chords in the secondary column. Re-tinted O(1) in `sync_theme_colors`
//! (the geometry is theme-independent), so the theme-picker preview re-tints for free.
//!
//! **Merge, don't align:** the titles are shaped as ONE line and their per-title x
//! extents read straight back off the shaped glyphs, so `menubar::boxes_from_extents`
//! produces the clickable bands from the SAME positions the pixels use — the drawn
//! titles and the click/hover hit-test ([`TextPipeline::menubar_title_at`]) can never
//! drift.

use super::*;

/// The gap drawn BETWEEN adjacent menu titles (as literal spaces in the shaped title
/// line) — comfortable separation without a heavy bar. The exact width doesn't affect
/// correctness (the hit-test reads the shaped positions, not this).
const TITLE_SEP: &str = "    ";

/// A dropdown row's height as a multiple of the LABEL-scaled line height — a touch of
/// vertical breathing room around each item so the rows read as clickable, not cramped.
const DROP_ROW_SCALE: f32 = 1.4;

/// Slack factor + floor on the estimated dropdown content width (label + gap + chord).
/// The card is sized generously from a char-count estimate (no second shaping pass);
/// Align-left labels + Align-right chords then position exactly within it, so a small
/// over-estimate just widens the calm card and never overlaps the two columns.
const DROP_WIDTH_SLACK: f32 = 1.15;
const DROP_MIN_WIDTH: f32 = 150.0;

impl TextPipeline {
    /// Shape + upload the whole WEB/LINUX MENU BAR for this frame: the bar ground
    /// strip, the title glyphs (with the open title's highlight band), and — when a
    /// dropdown is open ([`crate::menubar::open_menu`]) — its float card, item labels,
    /// native-chord secondary column, and separator hairlines. Records the hit-test
    /// geometry (`menubar_boxes` / `menubar_bar_h` / `menu_drop_rect` / `menu_drop_rows`
    /// / `menu_drop_menu`) read back by [`Self::menubar_title_at`] / [`Self::menubar_item_at`].
    /// PARKS everything empty/off-screen when the bar is hidden (`!menu_bar_on()`), so a
    /// default (macOS) capture is byte-identical.
    pub(in crate::render) fn prepare_menubar(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let bounds = TextBounds { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        if !crate::menubar::menu_bar_on() {
            // PARKED: no bar. Clear every quad + park both text layers off-screen +
            // reset the stored geometry, so a bar-off frame is byte-identical.
            self.menubar_boxes.clear();
            self.menubar_bar_h = 0.0;
            self.menu_drop_rect = None;
            self.menu_drop_rows.clear();
            self.menu_drop_menu = None;
            self.menubar_bg.prepare(device, queue, width, height, &[]);
            self.menubar_hi.prepare(device, queue, width, height, &[]);
            self.park_menu_text(device, queue, width, height, bounds)?;
            self.park_menu_dropdown(device, queue, width, height, bounds)?;
            return Ok(());
        }

        let m = self.metrics;
        let label = crate::markdown::type_scale::LABEL;
        let label_lh = m.line_height * label;
        let bar_h = crate::menubar::bar_height(label_lh);
        self.menubar_bar_h = bar_h;
        let faint = theme::faint().to_glyphon();
        let muted = theme::muted().to_glyphon();
        let content = theme::base_content().to_glyphon();
        // The open title's highlight decision, from the ONE owner every
        // "selected region" surface shares (`highlight_treatment`). A true 1-bit
        // world fills the band with solid `base_content` and recolors the OPEN
        // title's own glyphs to solid `base_300` (`open_ink`), so black text
        // lands crisp on the white band — NOT the gamma-grey a framebuffer
        // invert of the antialiased title produced (see
        // `HighlightTreatment::InverseFill`). Ordinary worlds keep the muted
        // open title on a value-band fill, byte-identical.
        let hi_treatment = theme::active().highlight_treatment(theme::selection());
        let (band_srgb, open_ink) = match hi_treatment {
            theme::HighlightTreatment::ValueBand(c) => (c, muted),
            theme::HighlightTreatment::InverseFill { band, ink } => (band, ink.to_glyphon()),
        };

        // Bar GROUND: a full-width value-step strip at the very top. Bled past the
        // top/left/right canvas edges it runs flush to (`menubar::bleed_to_canvas_
        // edges`) so the rounded-rect shader's AA feather never lands on a visible
        // pixel — see that fn's doc for the sliver bug this fixes (a shown bar used
        // to let ~16% of the frame underneath bleed through at row 0, and similarly
        // at the leftmost/rightmost column).
        let bg_rect =
            crate::menubar::bleed_to_canvas_edges([0.0, 0.0, width as f32, bar_h], width as f32);
        self.menubar_bg.prepare(device, queue, width, height, &[bg_rect]);

        // TITLES: shaped as ONE line, faint (the open one muted), tracking each
        // title's byte range so its shaped x-extent reads straight back off the glyphs.
        let menus = crate::menu::roster();
        let open = crate::menubar::open_menu().filter(|&i| i < menus.len());
        let mut spans: Vec<(&str, Attrs)> = Vec::with_capacity(menus.len() * 2);
        let mut ranges: Vec<std::ops::Range<usize>> = Vec::with_capacity(menus.len());
        let base = panel_attrs();
        let mut byte = 0usize;
        for (i, menu) in menus.iter().enumerate() {
            if i > 0 {
                spans.push((TITLE_SEP, base.clone().color(faint)));
                byte += TITLE_SEP.len();
            }
            let ink = if open == Some(i) { open_ink } else { faint };
            let start = byte;
            spans.push((menu.title, base.clone().color(ink)));
            byte += menu.title.len();
            ranges.push(start..byte);
        }
        self.menubar_buffer.set_metrics(
            &mut self.font_system,
            GlyphMetrics::new(m.font_size * label, label_lh),
        );
        self.menubar_buffer.set_size(&mut self.font_system, Some(width as f32), Some(bar_h + 1.0));
        let default_attrs = base.clone().color(faint);
        self.menubar_buffer.set_rich_text(
            &mut self.font_system,
            spans,
            &default_attrs,
            Shaping::Advanced,
            None,
        );
        self.menubar_buffer.shape_until_scroll(&mut self.font_system, false);

        // Read back each title's absolute x-extent (draw origin + shaped glyph x).
        let draw_left = crate::menubar::BAR_INSET_X;
        let mut extents: Vec<(f32, f32)> = vec![(f32::MAX, f32::MIN); ranges.len()];
        for run in self.menubar_buffer.layout_runs() {
            for g in run.glyphs.iter() {
                for (k, r) in ranges.iter().enumerate() {
                    if g.start >= r.start && g.start < r.end {
                        let e = &mut extents[k];
                        e.0 = e.0.min(draw_left + g.x);
                        e.1 = e.1.max(draw_left + g.x + g.w);
                    }
                }
            }
        }
        // A title with no shaped glyph (never happens for the static roster, but stay
        // defensive) collapses to a zero-width sliver at the draw origin.
        for e in extents.iter_mut() {
            if e.0 > e.1 {
                *e = (draw_left, draw_left);
            }
        }
        self.menubar_boxes = crate::menubar::boxes_from_extents(&extents);

        // OPEN title's highlight band (full bar height), else parked empty. Bled the
        // SAME way as the bar ground (top always, left/right only when THIS band
        // itself happens to run flush to a canvas edge — e.g. the first/last title).
        let hi: &[[f32; 4]] = &match open.and_then(|i| self.menubar_boxes.get(i)) {
            Some(b) => vec![crate::menubar::bleed_to_canvas_edges(
                [b.band_left, 0.0, b.band_right - b.band_left, bar_h],
                width as f32,
            )],
            None => Vec::new(),
        };
        // The band is ONE solid fill on every world (`menubar_hi`); its COLOR is
        // the only thing the treatment changes — a value-band tint on ordinary
        // worlds, solid `base_content` (white) on a 1-bit world, with the open
        // title's glyphs already recolored to `base_300` above. Mirrors
        // `overlay_draw_card`'s identical single-fill path for the picker;
        // routed through the ONE `highlight_treatment` owner.
        self.menubar_hi.set_color(band_srgb.rgba_bytes());
        self.menubar_hi.prepare(device, queue, width, height, hi);

        // Draw the title text (vertically centered in the bar).
        let title_top = (bar_h - label_lh) * 0.5;
        let area = TextArea {
            buffer: &self.menubar_buffer,
            left: draw_left,
            top: title_top,
            scale: 1.0,
            bounds,
            default_color: faint,
            custom_glyphs: &[],
        };
        self.menubar_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon menubar prepare failed: {e:?}"))?;

        // DROPDOWN: the open menu's anchored card + item rows, or parked.
        match open {
            Some(i) => {
                self.prepare_menu_dropdown(device, queue, width, height, bounds, i, bar_h, label, muted, content)?;
            }
            None => {
                self.menu_drop_rect = None;
                self.menu_drop_rows.clear();
                self.menu_drop_menu = None;
                self.park_menu_dropdown(device, queue, width, height, bounds)?;
            }
        }
        Ok(())
    }

    /// Shape + upload one open dropdown: the float card (raised border -> `base_300`
    /// card, no drop shadow — dark-depth Option C), the item LABELS (left, `base_content`),
    /// the native CHORDS (right, MUTED secondary column), and the separator hairlines.
    /// Records `menu_drop_rect` / `menu_drop_rows` / `menu_drop_menu` for the click hit-test.
    #[allow(clippy::too_many_arguments)]
    fn prepare_menu_dropdown(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        bounds: TextBounds,
        menu_i: usize,
        bar_h: f32,
        label: f32,
        muted: glyphon::Color,
        content: glyphon::Color,
    ) -> anyhow::Result<()> {
        let m = self.metrics;
        let label_lh = m.line_height * label;
        let row_h = label_lh * DROP_ROW_SCALE;
        let menus = crate::menu::roster();
        let items = &menus[menu_i].items;

        // Per-row content: label, native chord, separator flag. Labels/chords are ONE
        // line per row (separator rows blank), so the buffer lines land on the uniform
        // row grid `drop_rows` lays out.
        let mut labels = String::new();
        let mut chords = String::new();
        let mut separators: Vec<bool> = Vec::with_capacity(items.len());
        let mut widest_label = 0usize;
        let mut widest_chord = 0usize;
        for (idx, item) in items.iter().enumerate() {
            if idx > 0 {
                labels.push('\n');
                chords.push('\n');
            }
            match item {
                crate::menu::RosterItem::Routed { id, label, .. } => {
                    let chord = crate::menu::item_chord_for_id(id);
                    widest_label = widest_label.max(label.chars().count());
                    widest_chord = widest_chord.max(chord.chars().count());
                    labels.push_str(label);
                    chords.push_str(&chord);
                    separators.push(false);
                }
                crate::menu::RosterItem::Predefined(kind) => {
                    let lbl = crate::menu::predefined_label(*kind);
                    widest_label = widest_label.max(lbl.chars().count());
                    labels.push_str(lbl);
                    separators.push(false);
                }
                crate::menu::RosterItem::Separator => {
                    // Blank line; a hairline is drawn in this row's slot.
                    separators.push(true);
                }
            }
        }

        let (rows, rows_total) = crate::menubar::drop_rows(&separators, row_h);
        // Card width: a generous char-count estimate (no second shaping pass) — labels
        // Align-left + chords Align-right then sit exactly within it.
        let label_char_w = m.char_width * label;
        let gap_chars = rowlayout::GAP_CHARS;
        let content_w = ((widest_label + gap_chars + widest_chord) as f32 * label_char_w
            * DROP_WIDTH_SLACK)
            .max(DROP_MIN_WIDTH);
        let anchor = self.menubar_boxes.get(menu_i).copied().unwrap_or(crate::menubar::TitleBox {
            band_left: 0.0,
            text_left: 0.0,
            text_right: 0.0,
            band_right: 0.0,
        });
        let rect = crate::menubar::drop_rect(&anchor, bar_h, content_w, rows_total);
        self.menu_drop_rect = Some(rect);
        self.menu_drop_rows = rows.clone();
        self.menu_drop_menu = Some(menu_i);

        // Card elevation (raised border -> opaque card, no drop shadow) — unconditional
        // (this dropdown has no scrim/blur backdrop of its own to lean on instead).
        super::set_float_quads(
            &mut self.menu_drop_shadow,
            &mut self.menu_drop_border,
            &mut self.menu_drop_card,
            device,
            queue,
            width,
            height,
            Some(rect),
            super::FloatElevation::Rimmed,
            0.0,
            None,
        );

        // Separator hairlines (one thin quad centered in each separator row).
        let inner_left = rect[0] + crate::menubar::DROP_PAD_X;
        let inner_top = rect[1] + crate::menubar::DROP_PAD_Y;
        let seps: Vec<[f32; 4]> = rows
            .iter()
            .filter(|r| r.separator)
            .map(|r| [inner_left, inner_top + r.top + r.height * 0.5 - 0.5, content_w, 1.0])
            .collect();
        self.menu_drop_sep.prepare(device, queue, width, height, &seps);

        // Item LABELS (left) + native CHORDS (right), one line per row on the uniform
        // `row_h` grid, drawn from the card's inner top-left.
        let base = panel_attrs();
        for (buf, text, ink, align) in [
            (&mut self.menu_drop_buffer, labels, content, glyphon::cosmic_text::Align::Left),
            (&mut self.menu_chord_buffer, chords, muted, glyphon::cosmic_text::Align::Right),
        ] {
            buf.set_metrics(&mut self.font_system, GlyphMetrics::new(m.font_size * label, row_h));
            buf.set_size(&mut self.font_system, Some(content_w), Some(rows_total + 1.0));
            buf.set_text(&mut self.font_system, &text, &base.clone().color(ink), Shaping::Advanced, Some(align));
            buf.shape_until_scroll(&mut self.font_system, false);
        }
        let label_area = TextArea {
            buffer: &self.menu_drop_buffer,
            left: inner_left,
            top: inner_top,
            scale: 1.0,
            bounds,
            default_color: content,
            custom_glyphs: &[],
        };
        self.menu_drop_renderer
            .prepare(device, queue, &mut self.font_system, &mut self.atlas, &self.viewport, [label_area], &mut self.swash_cache)
            .map_err(|e| anyhow::anyhow!("glyphon menu-drop label prepare failed: {e:?}"))?;
        let chord_area = TextArea {
            buffer: &self.menu_chord_buffer,
            left: inner_left,
            top: inner_top,
            scale: 1.0,
            bounds,
            default_color: muted,
            custom_glyphs: &[],
        };
        self.menu_chord_renderer
            .prepare(device, queue, &mut self.font_system, &mut self.atlas, &self.viewport, [chord_area], &mut self.swash_cache)
            .map_err(|e| anyhow::anyhow!("glyphon menu-drop chord prepare failed: {e:?}"))?;
        Ok(())
    }

    /// Park the bar TITLE text renderer off-screen with empty text (so a hidden bar
    /// ghosts nothing), mirroring the gutter's parked branch.
    fn park_menu_text(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _width: u32,
        _height: u32,
        bounds: TextBounds,
    ) -> anyhow::Result<()> {
        self.menubar_buffer.set_size(&mut self.font_system, Some(1.0), Some(self.metrics.line_height));
        self.menubar_buffer.set_text(&mut self.font_system, "", &panel_attrs(), Shaping::Advanced, None);
        self.menubar_buffer.shape_until_scroll(&mut self.font_system, false);
        let area = TextArea {
            buffer: &self.menubar_buffer,
            left: 0.0,
            top: -1000.0,
            scale: 1.0,
            bounds,
            default_color: theme::faint().to_glyphon(),
            custom_glyphs: &[],
        };
        self.menubar_renderer
            .prepare(device, queue, &mut self.font_system, &mut self.atlas, &self.viewport, [area], &mut self.swash_cache)
            .map_err(|e| anyhow::anyhow!("glyphon menubar park failed: {e:?}"))?;
        Ok(())
    }

    /// Park the DROPDOWN quads + both item text renderers off-screen/empty, so a
    /// closed dropdown (or hidden bar) ghosts nothing.
    fn park_menu_dropdown(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        bounds: TextBounds,
    ) -> anyhow::Result<()> {
        super::set_float_quads(
            &mut self.menu_drop_shadow,
            &mut self.menu_drop_border,
            &mut self.menu_drop_card,
            device,
            queue,
            width,
            height,
            None,
            super::FloatElevation::Rimmed,
            0.0,
            None,
        );
        self.menu_drop_sep.prepare(device, queue, width, height, &[]);
        for buf in [&mut self.menu_drop_buffer, &mut self.menu_chord_buffer] {
            buf.set_size(&mut self.font_system, Some(1.0), Some(self.metrics.line_height));
            buf.set_text(&mut self.font_system, "", &panel_attrs(), Shaping::Advanced, None);
            buf.shape_until_scroll(&mut self.font_system, false);
        }
        let muted = theme::muted().to_glyphon();
        let label_area = TextArea {
            buffer: &self.menu_drop_buffer,
            left: 0.0,
            top: -1000.0,
            scale: 1.0,
            bounds,
            default_color: muted,
            custom_glyphs: &[],
        };
        self.menu_drop_renderer
            .prepare(device, queue, &mut self.font_system, &mut self.atlas, &self.viewport, [label_area], &mut self.swash_cache)
            .map_err(|e| anyhow::anyhow!("glyphon menu-drop label park failed: {e:?}"))?;
        let chord_area = TextArea {
            buffer: &self.menu_chord_buffer,
            left: 0.0,
            top: -1000.0,
            scale: 1.0,
            bounds,
            default_color: muted,
            custom_glyphs: &[],
        };
        self.menu_chord_renderer
            .prepare(device, queue, &mut self.font_system, &mut self.atlas, &self.viewport, [chord_area], &mut self.swash_cache)
            .map_err(|e| anyhow::anyhow!("glyphon menu-drop chord park failed: {e:?}"))?;
        Ok(())
    }

    /// Which menu-bar TITLE the point `(px, py)` hits (`Some(roster index)`), or `None`
    /// — the click + cursor-shape hit-test, reading the stored `menubar_boxes` /
    /// `menubar_bar_h` through the pure [`crate::menubar::title_at`], so it can never
    /// disagree with the drawn titles. `None` when the bar is hidden.
    pub fn menubar_title_at(&self, px: f32, py: f32) -> Option<usize> {
        if !crate::menubar::menu_bar_on() {
            return None;
        }
        crate::menubar::title_at(&self.menubar_boxes, self.menubar_bar_h, px, py)
    }

    /// True when `(px, py)` is over the shown bar's own strip OR the open dropdown's
    /// card — the cursor-shape "over menu chrome" band (the plain arrow over dead
    /// space, so it reads as chrome, never the document I-beam). `false` when the bar
    /// is hidden. A clickable title/item within it still wins the pointing hand via
    /// [`Self::menubar_hand_at`] (ranked higher in `cursor_shape`).
    pub fn over_menu_surface(&self, px: f32, py: f32) -> bool {
        if !crate::menubar::menu_bar_on() {
            return false;
        }
        if crate::menubar::in_bar(self.menubar_bar_h, py) {
            return true;
        }
        match self.menu_drop_rect {
            Some(r) => px >= r[0] && px < r[0] + r[2] && py >= r[1] && py < r[1] + r[3],
            None => false,
        }
    }

    /// True when `(px, py)` is over a CLICKABLE menu-bar title OR a clickable open-
    /// dropdown item — the pointing-hand affordance (a click there acts), the menu-bar
    /// analogue of a clickable picker row.
    pub fn menubar_hand_at(&self, px: f32, py: f32) -> bool {
        self.menubar_title_at(px, py).is_some() || self.menubar_item_at(px, py).is_some()
    }

    /// Which open-dropdown ITEM `(px, py)` hits — `Some((menu index, item index))` for a
    /// clickable roster item, `None` for a separator / off the card / no dropdown open.
    /// Reads the stored `menu_drop_rect` / `menu_drop_rows` through the pure
    /// [`crate::menubar::drop_item_at`], so it matches the drawn rows.
    pub fn menubar_item_at(&self, px: f32, py: f32) -> Option<(usize, usize)> {
        let menu = self.menu_drop_menu?;
        let rect = self.menu_drop_rect?;
        crate::menubar::drop_item_at(rect, &self.menu_drop_rows, px, py).map(|item| (menu, item))
    }

    /// The MENU BAR state for the capture sidecar: `(shown, open menu title, titles)` —
    /// read from the SAME globals + roster the renderer draws from, so the sidecar can
    /// never claim a bar state the pixels don't match. `open` is the OPEN dropdown's
    /// title (or `None`); `titles` is the bar's top-level menu titles in roster order.
    pub fn menubar_report(&self) -> (bool, Option<String>, Vec<String>) {
        let shown = crate::menubar::menu_bar_on();
        let menus = crate::menu::roster();
        let titles: Vec<String> = menus.iter().map(|m| m.title.to_string()).collect();
        let open = crate::menubar::open_menu().and_then(|i| menus.get(i)).map(|m| m.title.to_string());
        (shown, open, titles)
    }
}
