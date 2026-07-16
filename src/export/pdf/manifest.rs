//! Deterministic semantic inventory embedded as the PDF metadata stream.
//! Every neutral-model node gets a stable pre-order ID and explicit kind. The
//! inventory is deliberately redundant with the painted page content: tests and
//! accessibility tools can cross-check structure without treating it as proof
//! that glyphs, images, or annotations were actually painted.

use super::super::model::{Align, Block, Document, Inline};
use super::fonts::{FontRole, fallback_char, has_glyph};
use super::layout::Layout;

pub(super) fn build(doc: &Document, layout: &Layout) -> Vec<u8> {
    let mut manifest = Manifest {
        out: String::from(
            "<?xpacket begin=\"\u{feff}\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" xmlns:awl=\"https://awl.invalid/pdf/1\">\n<awl:document schema=\"awl-pdf-manifest/1\">",
        ),
        next_id: 1,
        resolved_images: layout
            .images
            .iter()
            .map(|image| image.src.as_str())
            .collect(),
    };
    if let Some(title) = &doc.title {
        manifest.out.push_str("<awl:title>");
        escape_into(&mut manifest.out, title);
        manifest.out.push_str("</awl:title>");
    }
    for block in &doc.blocks {
        manifest.block(block);
    }
    manifest
        .out
        .push_str("</awl:document>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>\n");
    manifest.out.into_bytes()
}

struct Manifest<'a> {
    out: String,
    next_id: u32,
    resolved_images: Vec<&'a str>,
}

impl Manifest<'_> {
    fn open(&mut self, kind: &str, attrs: &[(&str, String)]) {
        self.out.push_str("<awl:element id=\"e");
        self.out.push_str(&format!("{:04}\" kind=\"", self.next_id));
        self.next_id += 1;
        escape_into(&mut self.out, kind);
        self.out.push('"');
        for (name, value) in attrs {
            self.out.push(' ');
            self.out.push_str(name);
            self.out.push_str("=\"");
            escape_into(&mut self.out, value);
            self.out.push('"');
        }
        self.out.push('>');
    }

    fn empty(&mut self, kind: &str, attrs: &[(&str, String)]) {
        self.open(kind, attrs);
        self.out.truncate(self.out.len() - 1);
        self.out.push_str("/>");
    }

    fn close(&mut self) {
        self.out.push_str("</awl:element>");
    }

    fn block(&mut self, block: &Block) {
        match block {
            Block::Heading { level, inlines } => {
                self.open("heading", &[("level", level.to_string())]);
                self.inlines(inlines, FontRole::SerifBold);
                self.close();
            }
            Block::Paragraph(inlines) => {
                self.open("paragraph", &[]);
                self.inlines(inlines, FontRole::Serif);
                self.close();
            }
            Block::BlockQuote(blocks) => {
                self.open("blockquote", &[]);
                for block in blocks {
                    self.block(block);
                }
                self.close();
            }
            Block::CodeBlock { lang, code } => {
                let attrs = lang
                    .as_ref()
                    .map(|lang| vec![("lang", lang.clone())])
                    .unwrap_or_default();
                self.open("codeblock", &attrs);
                self.text("text", code, FontRole::Mono);
                self.close();
            }
            Block::List(list) => {
                self.open(
                    "list",
                    &[
                        ("ordered", list.ordered.to_string()),
                        ("start", list.start.to_string()),
                    ],
                );
                for item in &list.items {
                    let attrs = item.task.map_or_else(Vec::new, |checked| {
                        vec![("task", if checked { "checked" } else { "open" }.to_string())]
                    });
                    self.open("item", &attrs);
                    for block in &item.blocks {
                        self.block(block);
                    }
                    self.close();
                }
                self.close();
            }
            Block::Rule => self.empty("rule", &[]),
            Block::Table(table) => {
                self.open("table", &[("columns", table.aligns.len().to_string())]);
                let alignments = table
                    .aligns
                    .iter()
                    .map(align_name)
                    .collect::<Vec<_>>()
                    .join(",");
                self.open("table-head", &[("align", alignments.clone())]);
                self.row(&table.head, &table.aligns);
                self.close();
                self.open("table-body", &[]);
                for row in &table.rows {
                    self.row(row, &table.aligns);
                }
                self.close();
                self.close();
            }
        }
    }

    fn row(&mut self, row: &[Vec<Inline>], aligns: &[Align]) {
        self.open("table-row", &[]);
        for (index, cell) in row.iter().enumerate() {
            let align = aligns.get(index).copied().unwrap_or(Align::None);
            self.open("table-cell", &[("align", align_name(&align).to_string())]);
            self.inlines(cell, FontRole::Serif);
            self.close();
        }
        self.close();
    }

    fn inlines(&mut self, inlines: &[Inline], role: FontRole) {
        for inline in inlines {
            match inline {
                Inline::Text(text) => self.text("text", text, role),
                Inline::Strong(children) => {
                    self.open("strong", &[]);
                    let bold = match role {
                        FontRole::Serif | FontRole::SerifBold => FontRole::SerifBold,
                        FontRole::Mono | FontRole::MonoBold => FontRole::MonoBold,
                    };
                    self.inlines(children, bold);
                    self.close();
                }
                Inline::Emphasis(children) => {
                    self.open("emphasis", &[]);
                    self.inlines(children, role);
                    self.close();
                }
                Inline::Strikethrough(children) => {
                    self.open("strikethrough", &[]);
                    self.inlines(children, role);
                    self.close();
                }
                Inline::Highlight(children) => {
                    self.open("highlight", &[]);
                    self.inlines(children, role);
                    self.close();
                }
                Inline::Code(code) => {
                    let mono = if matches!(role, FontRole::SerifBold | FontRole::MonoBold) {
                        FontRole::MonoBold
                    } else {
                        FontRole::Mono
                    };
                    self.text("code", code, mono);
                }
                Inline::Link { url, children } => {
                    self.open("link", &[("url", url.clone())]);
                    self.inlines(children, role);
                    self.close();
                }
                Inline::Image {
                    src,
                    alt,
                    width_hint,
                } => {
                    let resolved = self.resolved_images.contains(&src.as_str());
                    let mut attrs = vec![
                        ("src", src.clone()),
                        ("alt", alt.clone()),
                        ("resolved", resolved.to_string()),
                    ];
                    if let Some(width) = width_hint {
                        attrs.push(("width", width.to_string()));
                    }
                    if !resolved {
                        attrs.push(("fallback", "alt-text".to_string()));
                    }
                    self.empty("image", &attrs);
                }
                Inline::SoftBreak => self.empty("softbreak", &[]),
                Inline::HardBreak => self.empty("hardbreak", &[]),
            }
        }
    }

    fn text(&mut self, kind: &str, text: &str, role: FontRole) {
        let missing = text
            .chars()
            .filter(|ch| *ch != '\n' && !has_glyph(role, *ch))
            .map(|ch| format!("U+{:04X}:U+{:04X}", ch as u32, fallback_char(role) as u32))
            .collect::<Vec<_>>();
        let attrs = if missing.is_empty() {
            Vec::new()
        } else {
            vec![("fallback", missing.join(","))]
        };
        self.open(kind, &attrs);
        escape_into(&mut self.out, text);
        self.close();
    }
}

fn align_name(align: &Align) -> &'static str {
    match align {
        Align::None => "none",
        Align::Left => "left",
        Align::Center => "center",
        Align::Right => "right",
    }
}

fn escape_into(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
}
