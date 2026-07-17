//! Fixed-measure A4 layout. Coordinates remain top-down here and are converted
//! to PDF's bottom-up page space only by the writer.

use std::collections::BTreeMap;

use glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight, Wrap};

use super::super::model::{Block, Document, ImageSource, Inline};
use super::fonts::{FontRole, Fonts, asset};
use super::images::PdfImage;
use super::inline::{Piece, Style, flatten};

pub(super) const PAGE_W: f32 = 595.276;
pub(super) const PAGE_H: f32 = 841.890;
pub(super) const MARGIN_X: f32 = 66.638;
pub(super) const MARGIN_Y: f32 = 56.693;
/// The locked prose measure: seventy Bitter body characters at the exporter’s
/// fixed 11pt metric. It is intentionally not a user setting.
pub(super) const PROSE_CHARS: usize = 70;
pub(super) const MEASURE: f32 = PROSE_CHARS as f32 * 6.6;
const BOTTOM: f32 = PAGE_H - MARGIN_Y;

pub(super) struct Layout {
    pub pages: Vec<Page>,
    pub images: Vec<PdfImage>,
    pub unicode: [BTreeMap<u16, String>; 4],
}

#[derive(Default)]
pub(super) struct Page {
    pub ops: Vec<Op>,
    pub links: Vec<LinkRect>,
}

pub(super) enum Op {
    Glyph(GlyphOp),
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        gray: f32,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        width: f32,
        gray: f32,
    },
    Image {
        index: usize,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
}

pub(super) struct GlyphOp {
    pub role: FontRole,
    pub glyph_id: u16,
    pub size: f32,
    pub x: f32,
    pub y: f32,
    pub italic: bool,
    pub actual: String,
}

pub(super) struct LinkRect {
    pub url: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone)]
pub(super) struct ShapedGlyph {
    pub role: FontRole,
    pub id: u16,
    pub size: f32,
    pub x: f32,
    pub advance: f32,
    pub style: Style,
    pub actual: String,
}
pub(super) struct ShapedLine {
    pub glyphs: Vec<ShapedGlyph>,
    pub width: f32,
    pub baseline: f32,
    pub height: f32,
}

pub(super) fn build(doc: &Document, images: &dyn ImageSource) -> Layout {
    let mut engine = Engine {
        fonts: Fonts::new(),
        images_source: images,
        pages: vec![Page::default()],
        page: 0,
        y: MARGIN_Y,
        images: Vec::new(),
        unicode: std::array::from_fn(|_| BTreeMap::new()),
    };
    engine.blocks(&doc.blocks, MARGIN_X, MEASURE);
    Layout {
        pages: engine.pages,
        images: engine.images,
        unicode: engine.unicode,
    }
}

pub(super) struct Engine<'a> {
    pub fonts: Fonts,
    pub images_source: &'a dyn ImageSource,
    pub pages: Vec<Page>,
    pub page: usize,
    pub y: f32,
    pub images: Vec<PdfImage>,
    pub unicode: [BTreeMap<u16, String>; 4],
}

impl Engine<'_> {
    pub(super) fn blocks(&mut self, blocks: &[Block], x: f32, width: f32) {
        for block in blocks {
            self.block(block, x, width);
        }
    }

    fn block(&mut self, block: &Block, x: f32, width: f32) {
        match block {
            Block::Heading { level, inlines } => {
                let size = match level {
                    1 => 22.0,
                    2 => 17.6,
                    3 => 14.85,
                    4 => 12.65,
                    5 => 11.0,
                    _ => 9.9,
                };
                let mut style = Style::body();
                style.role = FontRole::SerifBold;
                style.size = size;
                style.leading = (size * 1.28).max(14.0);
                self.rich(inlines, style, x, width, 5.0, true, false);
            }
            Block::Paragraph(inlines) => {
                self.rich(inlines, Style::body(), x, width, 8.0, false, false)
            }
            Block::BlockQuote(blocks) => {
                let start_page = self.page;
                let start_y = self.y;
                self.y += 3.0;
                self.blocks_italic(blocks, x + 18.0, (width - 18.0).max(40.0));
                if start_page == self.page {
                    self.pages[self.page].ops.push(Op::Line {
                        x1: x + 4.0,
                        y1: start_y,
                        x2: x + 4.0,
                        y2: self.y - 5.0,
                        width: 1.2,
                        gray: 0.65,
                    });
                }
            }
            Block::CodeBlock { lang: _, code } => {
                let mut style = Style::body();
                style.role = FontRole::Mono;
                style.size = 9.35;
                style.leading = 13.0;
                let pieces = vec![Piece::Text {
                    text: code.clone(),
                    style,
                }];
                self.text_pieces(&pieces, x + 9.0, width - 18.0, 8.0, false, true);
            }
            Block::List(list) => self.list(list, x, width),
            Block::Rule => {
                self.ensure(12.0);
                self.pages[self.page].ops.push(Op::Line {
                    x1: x,
                    y1: self.y + 4.0,
                    x2: x + width,
                    y2: self.y + 4.0,
                    width: 0.7,
                    gray: 0.72,
                });
                self.y += 12.0;
            }
            Block::Table(table) => self.table(table, x, width),
        }
    }

    fn blocks_italic(&mut self, blocks: &[Block], x: f32, width: f32) {
        for block in blocks {
            match block {
                Block::Paragraph(inlines) => {
                    let mut s = Style::body();
                    s.italic = true;
                    self.rich(inlines, s, x, width, 6.0, false, false);
                }
                Block::Heading { level: _, inlines } => {
                    let mut s = Style::body();
                    s.role = FontRole::SerifBold;
                    s.italic = true;
                    self.rich(inlines, s, x, width, 6.0, true, false);
                }
                Block::BlockQuote(b) => self.blocks_italic(b, x + 14.0, width - 14.0),
                Block::CodeBlock { lang: _, code } => {
                    let mut s = Style::body();
                    s.role = FontRole::Mono;
                    s.size = 9.35;
                    s.leading = 13.0;
                    self.text_pieces(
                        &[Piece::Text {
                            text: code.clone(),
                            style: s,
                        }],
                        x,
                        width,
                        6.0,
                        false,
                        true,
                    );
                }
                Block::List(l) => self.list(l, x, width),
                Block::Rule => self.block(block, x, width),
                Block::Table(t) => self.table(t, x, width),
            }
        }
    }

    pub(super) fn rich(
        &mut self,
        inlines: &[Inline],
        style: Style,
        x: f32,
        width: f32,
        after: f32,
        keep: bool,
        panel: bool,
    ) {
        let pieces = flatten(inlines, style);
        self.text_pieces(&pieces, x, width, after, keep, panel);
    }

    fn text_pieces(
        &mut self,
        pieces: &[Piece],
        x: f32,
        width: f32,
        after: f32,
        keep: bool,
        panel: bool,
    ) {
        let mut text = Vec::new();
        for piece in pieces {
            match piece {
                Piece::Text { .. } => text.push(piece.clone()),
                Piece::Image {
                    src,
                    alt,
                    width_hint,
                } => {
                    if !text.is_empty() {
                        self.shaped_segment(&text, x, width, 3.0, keep, panel);
                        text.clear();
                    }
                    self.image(src, alt, *width_hint, x, width);
                }
            }
        }
        if !text.is_empty() {
            self.shaped_segment(&text, x, width, 0.0, keep, panel);
        }
        self.y += after;
    }

    pub(super) fn shaped_segment(
        &mut self,
        pieces: &[Piece],
        x: f32,
        width: f32,
        after: f32,
        keep: bool,
        panel: bool,
    ) {
        let lines = self.shape(pieces, width);
        let total: f32 = lines.iter().map(|l| l.height).sum();
        if keep {
            self.ensure(total + 16.0);
        } else if lines.len() >= 2 && self.remaining() < lines[0].height * 2.0 {
            self.new_page();
        }
        for line in lines {
            self.ensure(line.height);
            if panel {
                self.pages[self.page].ops.push(Op::Rect {
                    x: x - 5.0,
                    y: self.y,
                    w: width + 10.0,
                    h: line.height,
                    gray: 0.955,
                });
            }
            self.place_line(&line, x, self.y);
            self.y += line.height;
        }
        self.y += after;
    }

    pub(super) fn shape(&mut self, pieces: &[Piece], width: f32) -> Vec<ShapedLine> {
        let styles: Vec<Style> = pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Text { style, .. } => Some(style.clone()),
                Piece::Image { .. } => None,
            })
            .collect();
        if styles.is_empty() {
            return Vec::new();
        }
        let spans: Vec<(&str, Attrs<'_>)> = pieces
            .iter()
            .filter_map(|piece| match piece {
                Piece::Text { text, .. } => Some(text.as_str()),
                Piece::Image { .. } => None,
            })
            .enumerate()
            .map(|(i, text)| {
                let style = &styles[i];
                let a = asset(style.role);
                (
                    text,
                    Attrs::new()
                        .family(Family::Name(a.family))
                        .weight(Weight(a.weight))
                        .metadata(i)
                        .metrics(Metrics::new(style.size, style.leading)),
                )
            })
            .collect();
        let base = &styles[0];
        let default = Attrs::new()
            .family(Family::Name(asset(base.role).family))
            .weight(Weight(asset(base.role).weight))
            .metrics(Metrics::new(base.size, base.leading));
        let mut buffer = Buffer::new(
            &mut self.fonts.system,
            Metrics::new(base.size, base.leading),
        );
        buffer.set_size(&mut self.fonts.system, Some(width), None);
        buffer.set_wrap(&mut self.fonts.system, Wrap::WordOrGlyph);
        buffer.set_rich_text(
            &mut self.fonts.system,
            spans,
            &default,
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut self.fonts.system, false);
        let mut previous_run_end = 0;
        buffer
            .layout_runs()
            .map(|run| {
                // cosmic-text can omit a literal whitespace glyph at a rich-span
                // or wrapped-line boundary while preserving its advance. Carry
                // any source bytes between adjacent shaped clusters into the NEXT
                // cluster's ActualText, so positioned text still extracts as the original
                // prose ("a [link]" must not become "areal link").
                // Cluster byte offsets are paragraph-relative, so the cursor must
                // span layout runs; resetting it on a wrapped line either loses the
                // wrap-space or repeats every earlier source byte.
                let mut previous_end = previous_run_end;
                let glyphs = run
                    .glyphs
                    .iter()
                    .map(|g| {
                        let role = self
                            .fonts
                            .role_for_id(g.font_id)
                            .expect("PDF shaping escaped closed font roster");
                        let style = styles.get(g.metadata).cloned().unwrap_or_else(Style::body);
                        let skipped = run.text.get(previous_end..g.start).unwrap_or("");
                        let cluster = style.actual.clone().unwrap_or_else(|| {
                            run.text.get(g.start..g.end).unwrap_or("").to_string()
                        });
                        let actual = format!("{skipped}{cluster}");
                        previous_end = previous_end.max(g.end);
                        ShapedGlyph {
                            role,
                            id: g.glyph_id,
                            size: g.font_size,
                            x: g.x + g.font_size * g.x_offset,
                            advance: g.w,
                            style,
                            actual,
                        }
                    })
                    .collect();
                previous_run_end = previous_end;
                ShapedLine {
                    glyphs,
                    width: run.line_w,
                    baseline: run.line_y - run.line_top,
                    height: run.line_height,
                }
            })
            .collect()
    }

    pub(super) fn remaining(&self) -> f32 {
        BOTTOM - self.y
    }
    pub(super) fn ensure(&mut self, h: f32) {
        if h > self.remaining() && self.y > MARGIN_Y {
            self.new_page();
        }
    }
    pub(super) fn new_page(&mut self) {
        self.pages.push(Page::default());
        self.page += 1;
        self.y = MARGIN_Y;
    }
}
