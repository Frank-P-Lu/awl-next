//! Positioned decorations, images, lists, and tables. Kept apart from shaping
//! so the fixed-page policy and the cosmic-text adapter remain small owners.

use super::super::model::{Align, Block, Inline, List, Table};
use super::fonts::{FontRole, role_index};
use super::images::PdfImage;
use super::inline::{Style, flatten};
use super::layout::{Engine, GlyphOp, LinkRect, MARGIN_Y, Op, PAGE_H, ShapedLine};

impl Engine<'_> {
    pub(super) fn place_line(&mut self, line: &ShapedLine, x: f32, top: f32) {
        let mut link: Option<(String, f32, f32)> = None;
        for glyph in &line.glyphs {
            if glyph.style.highlight || glyph.style.code {
                self.pages[self.page].ops.push(Op::Rect {
                    x: x + glyph.x,
                    y: top + 1.0,
                    w: glyph.advance,
                    h: line.height - 2.0,
                    gray: if glyph.style.highlight { 0.90 } else { 0.945 },
                });
            }
            let baseline = top + line.baseline;
            self.pages[self.page].ops.push(Op::Glyph(GlyphOp {
                role: glyph.role,
                glyph_id: glyph.id,
                size: glyph.size,
                x: x + glyph.x,
                y: baseline,
                italic: glyph.style.italic,
                actual: glyph.actual.clone(),
            }));
            self.unicode[role_index(glyph.role)]
                .entry(glyph.id)
                .or_insert_with(|| glyph.actual.clone());
            if glyph.style.strike {
                self.rule(
                    x + glyph.x,
                    baseline - glyph.size * 0.3,
                    x + glyph.x + glyph.advance,
                    baseline - glyph.size * 0.3,
                    0.55,
                    0.2,
                );
            }
            if glyph.style.link.is_some() {
                self.rule(
                    x + glyph.x,
                    baseline + 1.5,
                    x + glyph.x + glyph.advance,
                    baseline + 1.5,
                    0.45,
                    0.28,
                );
            }
            match (&glyph.style.link, &mut link) {
                (Some(url), Some((open, _, end))) if open == url => {
                    *end = x + glyph.x + glyph.advance
                }
                (Some(url), _) => {
                    if let Some((u, start, end)) = link.take() {
                        self.add_link(u, start, top, end - start, line.height);
                    }
                    link = Some((url.clone(), x + glyph.x, x + glyph.x + glyph.advance));
                }
                (None, _) => {
                    if let Some((u, start, end)) = link.take() {
                        self.add_link(u, start, top, end - start, line.height);
                    }
                }
            }
        }
        if let Some((u, start, end)) = link {
            self.add_link(u, start, top, end - start, line.height);
        }
    }

    fn rule(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, gray: f32) {
        self.pages[self.page].ops.push(Op::Line {
            x1,
            y1,
            x2,
            y2,
            width,
            gray,
        });
    }

    fn add_link(&mut self, url: String, x: f32, y: f32, w: f32, h: f32) {
        self.pages[self.page]
            .links
            .push(LinkRect { url, x, y, w, h });
    }

    pub(super) fn image(&mut self, src: &str, alt: &str, hint: Option<u32>, x: f32, width: f32) {
        let prepared = self
            .images_source
            .resolve(src)
            .and_then(|image| PdfImage::prepare(image, src, alt));
        let Some(prepared) = prepared else {
            let label = if alt.is_empty() {
                format!("[{src}]")
            } else {
                format!("[{alt}]")
            };
            let pieces = flatten(&[Inline::Text(label)], Style::body());
            self.shaped_segment(&pieces, x, width, 0.0, false, false);
            return;
        };
        let index = self
            .images
            .iter()
            .position(|i| i.src == src)
            .unwrap_or_else(|| {
                self.images.push(prepared);
                self.images.len() - 1
            });
        let image = &self.images[index];
        let mut w = hint
            .map_or(image.width as f32 * 0.75, |v| v as f32 * 0.75)
            .min(width);
        let mut h = w * image.height as f32 / image.width as f32;
        let max_h = PAGE_H - 2.0 * MARGIN_Y;
        if h > max_h {
            let scale = max_h / h;
            h *= scale;
            w *= scale;
        }
        self.ensure(h + 4.0);
        self.pages[self.page].ops.push(Op::Image {
            index,
            x,
            y: self.y,
            w,
            h,
        });
        self.y += h + 4.0;
    }

    pub(super) fn list(&mut self, list: &List, x: f32, width: f32) {
        for (i, item) in list.items.iter().enumerate() {
            let marker = match item.task {
                Some(true) => "[x] ".into(),
                Some(false) => "[ ] ".into(),
                None if list.ordered => format!("{}. ", list.start + i as u64),
                None => "• ".into(),
            };
            if let Some(Block::Paragraph(inlines)) = item.blocks.first() {
                let mut combined = vec![Inline::Text(marker)];
                combined.extend(inlines.clone());
                self.rich(&combined, Style::body(), x, width, 3.0, false, false);
                self.blocks(&item.blocks[1..], x + 18.0, width - 18.0);
            } else {
                self.rich(
                    &[Inline::Text(marker)],
                    Style::body(),
                    x,
                    width,
                    0.0,
                    false,
                    false,
                );
                self.blocks(&item.blocks, x + 18.0, width - 18.0);
            }
        }
        self.y += 4.0;
    }

    pub(super) fn table(&mut self, table: &Table, x: f32, width: f32) {
        let columns = table
            .aligns
            .len()
            .max(table.head.len())
            .max(table.rows.iter().map(Vec::len).max().unwrap_or(1))
            .max(1);
        let col_w = width / columns as f32;
        let head = (!table.head.is_empty()).then(|| self.shape_row(&table.head, col_w, true));
        if let Some(shaped) = &head {
            self.paint_row(shaped, &table.aligns, x, col_w, true, None);
        }
        for row in &table.rows {
            let shaped = self.shape_row(row, col_w, false);
            let height = self.row_height(&shaped);
            if height <= PAGE_H - 2.0 * MARGIN_Y && height > self.remaining() {
                self.new_page();
                if let Some(header) = &head {
                    self.paint_row(header, &table.aligns, x, col_w, true, None);
                }
            }
            self.paint_row(&shaped, &table.aligns, x, col_w, false, head.as_ref());
        }
        self.y += 6.0;
    }

    fn shape_row(&mut self, row: &[Vec<Inline>], col_w: f32, head: bool) -> Vec<Vec<ShapedLine>> {
        row.iter()
            .map(|cell| {
                let mut style = Style::body();
                style.size = 9.9;
                style.leading = 14.0;
                if head {
                    style.role = FontRole::SerifBold;
                }
                self.shape(&flatten(cell, style), col_w - 10.0)
            })
            .collect()
    }

    fn row_height(&self, shaped: &[Vec<ShapedLine>]) -> f32 {
        shaped.iter().map(Vec::len).max().unwrap_or(1) as f32 * 14.0 + 6.0
    }

    fn paint_row(
        &mut self,
        shaped: &[Vec<ShapedLine>],
        aligns: &[Align],
        x: f32,
        col_w: f32,
        head: bool,
        repeat_header: Option<&Vec<Vec<ShapedLine>>>,
    ) {
        let count = shaped.iter().map(Vec::len).max().unwrap_or(1);
        if count == 0 {
            return;
        }
        let full_height = count as f32 * 14.0 + 6.0;
        if full_height <= PAGE_H - 2.0 * MARGIN_Y {
            self.ensure(full_height);
            self.paint_row_fragment(shaped, aligns, x, col_w, head, 0, count);
        } else {
            for line in 0..count {
                if 20.0 > self.remaining() && self.y > MARGIN_Y {
                    self.new_page();
                    if let Some(header) = repeat_header {
                        self.paint_row(header, aligns, x, col_w, true, None);
                    }
                }
                self.ensure(20.0);
                self.paint_row_fragment(shaped, aligns, x, col_w, head, line, line + 1);
            }
        }
    }

    fn paint_row_fragment(
        &mut self,
        shaped: &[Vec<ShapedLine>],
        aligns: &[Align],
        x: f32,
        col_w: f32,
        head: bool,
        from: usize,
        to: usize,
    ) {
        let height = (to - from) as f32 * 14.0 + 6.0;
        let start = self.y;
        self.pages[self.page].ops.push(Op::Rect {
            x,
            y: start,
            w: col_w * shaped.len() as f32,
            h: height,
            gray: if head { 0.92 } else { 0.975 },
        });
        for (ci, lines) in shaped.iter().enumerate() {
            for (slot, line) in lines.iter().enumerate().skip(from).take(to - from) {
                let align = aligns.get(ci).copied().unwrap_or(Align::None);
                let shift = match align {
                    Align::Center => (col_w - 10.0 - line.width) / 2.0,
                    Align::Right => col_w - 10.0 - line.width,
                    Align::None | Align::Left => 0.0,
                }
                .max(0.0);
                self.place_line(
                    line,
                    x + ci as f32 * col_w + 5.0 + shift,
                    start + 3.0 + (slot - from) as f32 * 14.0,
                );
            }
        }
        for ci in 0..=shaped.len() {
            self.rule(
                x + ci as f32 * col_w,
                start,
                x + ci as f32 * col_w,
                start + height,
                0.35,
                0.78,
            );
        }
        self.rule(
            x,
            start + height,
            x + col_w * shaped.len() as f32,
            start + height,
            0.35,
            0.78,
        );
        self.y += height;
    }
}
