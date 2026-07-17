//! The DOCX emitter: a [`Document`] → a minimal, VALID OOXML `.docx` package,
//! assembled by the deterministic STORED-ZIP writer ([`super::zip`]). NEUTRAL
//! HOUSE STYLE — a clean `styles.xml` (tasteful serif body, a heading size
//! ladder echoing awl's own `type_scale` ramp, mono code), lists via a real
//! `numbering.xml`, `w:highlight` for `==marked==`, links as real external-target
//! hyperlink relationships, and embedded images under `word/media/`.
//!
//! The package has the classic minimal part set:
//! - `[Content_Types].xml` — the part-type registry
//! - `_rels/.rels` — package → main-document relationship
//! - `word/document.xml` — the body
//! - `word/styles.xml` — the house style
//! - `word/numbering.xml` — bullet + ordered list definitions
//! - `word/_rels/document.xml.rels` — styles/numbering + per-hyperlink/-image rels
//! - `word/media/imageN.<ext>` — embedded image bytes
//!
//! Everything is a pure function of the [`Document`] + resolved image bytes, so
//! two exports of the same document are byte-identical (the golden-file gate).

use super::model::{Align, Block, Document, ExportImage, ImageSource, Inline, List, Table};
use super::zip::ZipWriter;

// OOXML namespace URIs (declared once on the document root).
const NS_W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_PIC: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";

/// px → EMU at 96 DPI (1 inch = 914400 EMU = 96 px).
const EMU_PER_PX: i64 = 9525;
/// Max image content width: 6 inches (a US-Letter page minus 1" margins).
const MAX_IMG_EMU: i64 = 6 * 914400;
/// Usable content width in twips: a US-Letter page (12240) minus the 1" left +
/// 1" right margins (1440 each) declared in the `sectPr` `pgMar`. Table columns
/// split this evenly. REAL column widths matter: with `w:w="0"`/`type="auto"`
/// Word auto-fits, but Pages collapses every column to one-character width — so
/// we emit explicit `dxa` widths + a `fixed` `tblLayout` and both honor the grid.
const TABLE_CONTENT_TWIPS: i64 = 12240 - 1440 - 1440;

/// A package relationship (styles/numbering/hyperlink/image).
struct Rel {
    id: String,
    rtype: String,
    target: String,
    external: bool,
}

/// Accumulated export state: the body XML plus the relationships, media parts,
/// and ordered-list numbering instances discovered while walking the tree.
struct Docx<'a> {
    body: String,
    rels: Vec<Rel>,
    media: Vec<(String, Vec<u8>)>,
    /// Start value of each ordered list, in encounter order — each gets a fresh
    /// `numId` (2 + index) so its numbering restarts independently.
    ordered_starts: Vec<u64>,
    next_rel: u32,
    next_pic: u32,
    images: &'a dyn ImageSource,
}

/// Render `doc` to `.docx` bytes, resolving images through `images`.
pub fn emit(doc: &Document, images: &dyn ImageSource) -> Vec<u8> {
    let mut d = Docx {
        body: String::new(),
        // rId1 = styles, rId2 = numbering (fixed); dynamic rels start at rId3.
        rels: vec![
            Rel {
                id: "rId1".into(),
                rtype: "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles"
                    .into(),
                target: "styles.xml".into(),
                external: false,
            },
            Rel {
                id: "rId2".into(),
                rtype:
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering"
                        .into(),
                target: "numbering.xml".into(),
                external: false,
            },
        ],
        media: Vec::new(),
        ordered_starts: Vec::new(),
        next_rel: 3,
        next_pic: 1,
        images,
    };

    for b in &doc.blocks {
        d.block(b, 0);
    }

    let mut zip = ZipWriter::new();
    zip.add("[Content_Types].xml", content_types(&d.media).as_bytes());
    zip.add("_rels/.rels", DOT_RELS.as_bytes());
    zip.add("word/document.xml", document_xml(&d.body).as_bytes());
    zip.add("word/styles.xml", STYLES_XML.as_bytes());
    zip.add(
        "word/numbering.xml",
        numbering_xml(&d.ordered_starts).as_bytes(),
    );
    zip.add(
        "word/_rels/document.xml.rels",
        document_rels(&d.rels).as_bytes(),
    );
    for (name, bytes) in &d.media {
        zip.add(&format!("word/{name}"), bytes);
    }
    zip.finish()
}

impl Docx<'_> {
    fn alloc_rel(&mut self, rtype: &str, target: &str, external: bool) -> String {
        let id = format!("rId{}", self.next_rel);
        self.next_rel += 1;
        self.rels.push(Rel {
            id: id.clone(),
            rtype: rtype.to_string(),
            target: target.to_string(),
            external,
        });
        id
    }

    fn block(&mut self, block: &Block, list_depth: usize) {
        match block {
            Block::Heading { level, inlines } => {
                let style = format!("Heading{}", (*level).clamp(1, 6));
                self.paragraph(&style, None, None, inlines);
            }
            Block::Paragraph(inlines) => self.paragraph("Normal", None, None, inlines),
            Block::BlockQuote(blocks) => {
                for b in blocks {
                    // A quoted paragraph takes the Quote style; nested non-paragraph
                    // blocks render normally (rare).
                    match b {
                        Block::Paragraph(inlines) => self.paragraph("Quote", None, None, inlines),
                        other => self.block(other, list_depth),
                    }
                }
            }
            Block::CodeBlock { code, .. } => self.code_block(code),
            Block::List(list) => self.list(list, list_depth),
            Block::Rule => self.rule(),
            Block::Table(table) => self.table(table),
        }
    }

    /// One `<w:p>` with a paragraph style, optional list `numPr`, optional
    /// explicit left indent (task items), and an inline run.
    fn paragraph(
        &mut self,
        style: &str,
        num: Option<(u32, usize)>,
        indent_left: Option<i64>,
        inlines: &[Inline],
    ) {
        self.body.push_str("<w:p><w:pPr>");
        self.body
            .push_str(&format!("<w:pStyle w:val=\"{style}\"/>"));
        if let Some((num_id, ilvl)) = num {
            self.body.push_str(&format!(
                "<w:numPr><w:ilvl w:val=\"{ilvl}\"/><w:numId w:val=\"{num_id}\"/></w:numPr>"
            ));
        }
        if let Some(left) = indent_left {
            self.body.push_str(&format!("<w:ind w:left=\"{left}\"/>"));
        }
        self.body.push_str("</w:pPr>");
        let mut runs = String::new();
        for i in inlines {
            self.inline(&mut runs, i, RunProps::default());
        }
        self.body.push_str(&runs);
        self.body.push_str("</w:p>");
    }

    /// A fenced/indented code block: one paragraph, `Code` style, source lines
    /// separated by `<w:br/>` so the shaded band stays contiguous.
    fn code_block(&mut self, code: &str) {
        self.body
            .push_str("<w:p><w:pPr><w:pStyle w:val=\"Code\"/></w:pPr>");
        let mono = RunProps {
            code: true,
            ..RunProps::default()
        };
        let lines: Vec<&str> = code.split('\n').collect();
        for (i, line) in lines.iter().enumerate() {
            self.body.push_str("<w:r>");
            self.body.push_str(&mono.rpr());
            if i > 0 {
                self.body.push_str("<w:br/>");
            }
            self.body.push_str(&text_element(line));
            self.body.push_str("</w:r>");
        }
        self.body.push_str("</w:p>");
    }

    fn rule(&mut self) {
        // A paragraph whose bottom border draws the horizontal rule.
        self.body.push_str(
            "<w:p><w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"1\" \
             w:color=\"AAAAAA\"/></w:pBdr></w:pPr></w:p>",
        );
    }

    fn list(&mut self, list: &List, depth: usize) {
        let ilvl = depth.min(2);
        let num_id = if list.ordered {
            self.ordered_starts.push(list.start);
            (self.ordered_starts.len() as u32) + 1 // numId 2.. (numId 1 = bullet)
        } else {
            1
        };
        for item in &list.items {
            // The first paragraph of the item carries the list marker; nested
            // blocks (sub-lists, extra paragraphs) follow as their own paragraphs.
            let mut blocks = item.blocks.iter();
            let first = blocks.next();
            match item.task {
                Some(checked) => {
                    // Task item: no numbering — a checkbox glyph leads the text,
                    // indented to match its depth.
                    let box_glyph = if checked { "\u{2611} " } else { "\u{2610} " };
                    let mut lead = vec![Inline::Text(box_glyph.to_string())];
                    if let Some(Block::Paragraph(inlines)) = first {
                        lead.extend(inlines.iter().cloned());
                    }
                    let indent = 720 * (depth as i64 + 1);
                    self.paragraph("ListParagraph", None, Some(indent), &lead);
                }
                None => {
                    if let Some(Block::Paragraph(inlines)) = first {
                        self.paragraph("ListParagraph", Some((num_id, ilvl)), None, inlines);
                    } else if let Some(other) = first {
                        // Non-paragraph first child (e.g. an item that is only a
                        // nested list): emit an empty numbered paragraph, then it.
                        self.paragraph("ListParagraph", Some((num_id, ilvl)), None, &[]);
                        self.block(other, depth + 1);
                    }
                }
            }
            for b in blocks {
                match b {
                    Block::List(inner) => self.list(inner, depth + 1),
                    other => self.block(other, depth),
                }
            }
        }
    }

    fn table(&mut self, table: &Table) {
        // Column count = the widest of header/rows/aligns; the usable content
        // width splits evenly across them. Computed FIRST so the real twip
        // widths flow into `tblW`, every `gridCol`, and every cell's `tcW`.
        let cols = table
            .head
            .len()
            .max(table.aligns.len())
            .max(table.rows.iter().map(|r| r.len()).max().unwrap_or(0))
            .max(1);
        let col_w = (TABLE_CONTENT_TWIPS / cols as i64).max(1);
        let total_w = col_w * cols as i64;
        self.body.push_str(&format!(
            "<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/>\
             <w:tblW w:w=\"{total_w}\" w:type=\"dxa\"/>\
             <w:tblBorders>\
             <w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"AAAAAA\"/>\
             <w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"AAAAAA\"/>\
             <w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"AAAAAA\"/>\
             <w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"AAAAAA\"/>\
             <w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"AAAAAA\"/>\
             <w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"AAAAAA\"/>\
             </w:tblBorders>\
             <w:tblLayout w:type=\"fixed\"/></w:tblPr>",
        ));
        // A `w:tblGrid` (one sized `gridCol` per column) — Word prompts to
        // "repair" a table that lacks it, and Pages needs the explicit widths.
        self.body.push_str("<w:tblGrid>");
        for _ in 0..cols {
            self.body.push_str(&format!("<w:gridCol w:w=\"{col_w}\"/>"));
        }
        self.body.push_str("</w:tblGrid>");
        if !table.head.is_empty() {
            self.table_row(&table.head, &table.aligns, col_w, true);
        }
        for row in &table.rows {
            self.table_row(row, &table.aligns, col_w, false);
        }
        self.body.push_str("</w:tbl>");
        // A trailing empty paragraph — Word requires a block after a table.
        self.body.push_str("<w:p/>");
    }

    fn table_row(&mut self, cells: &[Vec<Inline>], aligns: &[Align], col_w: i64, header: bool) {
        self.body.push_str("<w:tr>");
        for (i, cell) in cells.iter().enumerate() {
            self.body.push_str(&format!(
                "<w:tc><w:tcPr><w:tcW w:w=\"{col_w}\" w:type=\"dxa\"/></w:tcPr>"
            ));
            let jc = match aligns.get(i).copied().unwrap_or(Align::None) {
                Align::Center => Some("center"),
                Align::Right => Some("right"),
                Align::Left => Some("left"),
                Align::None => None,
            };
            self.body
                .push_str("<w:p><w:pPr><w:pStyle w:val=\"TableCellText\"/>");
            if let Some(j) = jc {
                self.body.push_str(&format!("<w:jc w:val=\"{j}\"/>"));
            }
            self.body.push_str("</w:pPr>");
            let props = RunProps {
                bold: header,
                ..RunProps::default()
            };
            let mut runs = String::new();
            for inl in cell {
                self.inline(&mut runs, inl, props);
            }
            self.body.push_str(&runs);
            self.body.push_str("</w:p></w:tc>");
        }
        self.body.push_str("</w:tr>");
    }

    /// Emit one inline node into `out`, carrying accumulated `props` down the
    /// tree (bold/italic/strike/highlight/code/hyperlink compose).
    fn inline(&mut self, out: &mut String, inline: &Inline, props: RunProps) {
        match inline {
            Inline::Text(t) => {
                out.push_str("<w:r>");
                out.push_str(&props.rpr());
                out.push_str(&text_element(t));
                out.push_str("</w:r>");
            }
            Inline::Code(c) => {
                let p = RunProps {
                    code: true,
                    ..props
                };
                out.push_str("<w:r>");
                out.push_str(&p.rpr());
                out.push_str(&text_element(c));
                out.push_str("</w:r>");
            }
            Inline::Strong(children) => {
                let p = RunProps {
                    bold: true,
                    ..props
                };
                for c in children {
                    self.inline(out, c, p);
                }
            }
            Inline::Emphasis(children) => {
                let p = RunProps {
                    italic: true,
                    ..props
                };
                for c in children {
                    self.inline(out, c, p);
                }
            }
            Inline::Strikethrough(children) => {
                let p = RunProps {
                    strike: true,
                    ..props
                };
                for c in children {
                    self.inline(out, c, p);
                }
            }
            Inline::Highlight(children) => {
                let p = RunProps {
                    highlight: true,
                    ..props
                };
                for c in children {
                    self.inline(out, c, p);
                }
            }
            Inline::Link { url, children } => {
                let rid = self.alloc_rel(
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink",
                    url,
                    true,
                );
                out.push_str(&format!("<w:hyperlink r:id=\"{rid}\">"));
                let p = RunProps {
                    hyperlink: true,
                    ..props
                };
                for c in children {
                    self.inline(out, c, p);
                }
                out.push_str("</w:hyperlink>");
            }
            Inline::Image {
                src,
                alt,
                width_hint,
            } => self.image(out, src, alt, *width_hint),
            Inline::SoftBreak => {
                out.push_str("<w:r>");
                out.push_str(&props.rpr());
                out.push_str(&text_element(" "));
                out.push_str("</w:r>");
            }
            Inline::HardBreak => out.push_str("<w:r><w:br/></w:r>"),
        }
    }

    fn image(&mut self, out: &mut String, src: &str, alt: &str, width_hint: Option<u32>) {
        let Some(ExportImage {
            bytes,
            width,
            height,
            mime,
        }) = self.images.resolve(src)
        else {
            // Unresolvable: the alt text stands in.
            if !alt.is_empty() {
                out.push_str("<w:r>");
                out.push_str(&RunProps::default().rpr());
                out.push_str(&text_element(alt));
                out.push_str("</w:r>");
            }
            return;
        };
        // Scale to fit the content width, honoring an explicit width hint (px).
        let intrinsic_w = width.max(1) as i64;
        let intrinsic_h = height.max(1) as i64;
        let target_px_w = width_hint.map(|w| w as i64).unwrap_or(intrinsic_w);
        let mut cx = target_px_w * EMU_PER_PX;
        let mut cy = cx * intrinsic_h / intrinsic_w;
        if cx > MAX_IMG_EMU {
            cy = cy * MAX_IMG_EMU / cx;
            cx = MAX_IMG_EMU;
        }

        let pic_id = self.next_pic;
        self.next_pic += 1;
        let media_name = format!("media/image{pic_id}.{}", mime.ext());
        let rid = self.alloc_rel(
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
            &media_name,
            false,
        );
        self.media.push((media_name, bytes));

        let name = format!("Picture {pic_id}");
        out.push_str(&format!(
            "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\">\
             <wp:extent cx=\"{cx}\" cy=\"{cy}\"/>\
             <wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>\
             <wp:docPr id=\"{pic_id}\" name=\"{name}\" descr=\"{desc}\"/>\
             <wp:cNvGraphicFramePr><a:graphicFrameLocks xmlns:a=\"{ns_a}\" noChangeAspect=\"1\"/></wp:cNvGraphicFramePr>\
             <a:graphic xmlns:a=\"{ns_a}\"><a:graphicData uri=\"{ns_pic}\">\
             <pic:pic xmlns:pic=\"{ns_pic}\">\
             <pic:nvPicPr><pic:cNvPr id=\"{pic_id}\" name=\"{name}\"/><pic:cNvPicPr/></pic:nvPicPr>\
             <pic:blipFill><a:blip r:embed=\"{rid}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>\
             <pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
             <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr>\
             </pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>",
            desc = attr_escape(alt),
            ns_a = NS_A,
            ns_pic = NS_PIC,
        ));
    }
}

/// The run properties accumulated down an inline subtree.
#[derive(Clone, Copy, Default)]
struct RunProps {
    bold: bool,
    italic: bool,
    strike: bool,
    highlight: bool,
    code: bool,
    hyperlink: bool,
}

impl RunProps {
    /// The `<w:rPr>` element for these props (empty string when all-default),
    /// emitted in CT_RPr schema order (rStyle, rFonts, b, i, strike, highlight, shd).
    fn rpr(&self) -> String {
        if !(self.bold
            || self.italic
            || self.strike
            || self.highlight
            || self.code
            || self.hyperlink)
        {
            return String::new();
        }
        let mut s = String::from("<w:rPr>");
        if self.hyperlink {
            s.push_str("<w:rStyle w:val=\"Hyperlink\"/>");
        } else if self.code {
            s.push_str("<w:rStyle w:val=\"CodeChar\"/>");
        }
        if self.code {
            s.push_str("<w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\" w:cs=\"Consolas\"/>");
        }
        if self.bold {
            s.push_str("<w:b/>");
        }
        if self.italic {
            s.push_str("<w:i/>");
        }
        if self.strike {
            s.push_str("<w:strike/>");
        }
        if self.highlight {
            s.push_str("<w:highlight w:val=\"yellow\"/>");
        }
        if self.code {
            s.push_str("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F4F4F2\"/>");
        }
        s.push_str("</w:rPr>");
        s
    }
}

// --- Static + generated parts ----------------------------------------------

/// A `<w:t xml:space="preserve">` element with escaped content.
fn text_element(t: &str) -> String {
    format!("<w:t xml:space=\"preserve\">{}</w:t>", xml_escape(t))
}

fn document_xml(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <w:document xmlns:w=\"{NS_W}\" xmlns:r=\"{NS_R}\" xmlns:wp=\"{NS_WP}\" \
         xmlns:a=\"{NS_A}\" xmlns:pic=\"{NS_PIC}\"><w:body>{body}\
         <w:sectPr><w:pgSz w:w=\"12240\" w:h=\"15840\"/>\
         <w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" \
         w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>"
    )
}

fn content_types(media: &[(String, Vec<u8>)]) -> String {
    let mut has_png = false;
    let mut has_jpeg = false;
    for (name, _) in media {
        if name.ends_with(".png") {
            has_png = true;
        }
        if name.ends_with(".jpeg") {
            has_jpeg = true;
        }
    }
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>",
    );
    if has_png {
        s.push_str("<Default Extension=\"png\" ContentType=\"image/png\"/>");
    }
    if has_jpeg {
        s.push_str("<Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>");
    }
    s.push_str(
        "<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
         <Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
         <Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/>\
         </Types>",
    );
    s
}

const DOT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
</Relationships>";

fn document_rels(rels: &[Rel]) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for r in rels {
        if r.external {
            s.push_str(&format!(
                "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\" TargetMode=\"External\"/>",
                r.id,
                r.rtype,
                attr_escape(&r.target)
            ));
        } else {
            s.push_str(&format!(
                "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"/>",
                r.id,
                r.rtype,
                attr_escape(&r.target)
            ));
        }
    }
    s.push_str("</Relationships>");
    s
}

fn numbering_xml(ordered_starts: &[u64]) -> String {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    s.push_str(&format!("<w:numbering xmlns:w=\"{NS_W}\">"));
    // abstractNum 0 — bullets (three levels).
    s.push_str("<w:abstractNum w:abstractNumId=\"0\">");
    for (ilvl, (glyph, left)) in [("\u{2022}", 720), ("\u{25E6}", 1440), ("\u{25AA}", 2160)]
        .into_iter()
        .enumerate()
    {
        s.push_str(&format!(
            "<w:lvl w:ilvl=\"{ilvl}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"bullet\"/>\
             <w:lvlText w:val=\"{glyph}\"/><w:lvlJc w:val=\"left\"/>\
             <w:pPr><w:ind w:left=\"{left}\" w:hanging=\"360\"/></w:pPr></w:lvl>"
        ));
    }
    s.push_str("</w:abstractNum>");
    // abstractNum 1 — ordered (decimal / lowerLetter / lowerRoman).
    s.push_str("<w:abstractNum w:abstractNumId=\"1\">");
    for (ilvl, (fmt, pat, left)) in [
        ("decimal", "%1.", 720),
        ("lowerLetter", "%2.", 1440),
        ("lowerRoman", "%3.", 2160),
    ]
    .into_iter()
    .enumerate()
    {
        s.push_str(&format!(
            "<w:lvl w:ilvl=\"{ilvl}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{fmt}\"/>\
             <w:lvlText w:val=\"{pat}\"/><w:lvlJc w:val=\"left\"/>\
             <w:pPr><w:ind w:left=\"{left}\" w:hanging=\"360\"/></w:pPr></w:lvl>"
        ));
    }
    s.push_str("</w:abstractNum>");
    // numId 1 — the shared bullet instance.
    s.push_str("<w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num>");
    // numId 2.. — one per ordered list, restarting at its own start value.
    for (i, start) in ordered_starts.iter().enumerate() {
        let num_id = i + 2;
        s.push_str(&format!(
            "<w:num w:numId=\"{num_id}\"><w:abstractNumId w:val=\"1\"/>\
             <w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"{start}\"/></w:lvlOverride></w:num>"
        ));
    }
    s.push_str("</w:numbering>");
    s
}

/// The neutral house style. Serif body (Georgia), a heading size ladder echoing
/// awl's `type_scale` ramp (h1 ≈ 1.8× body, h2 ≈ 1.5×, h3 ≈ 1.25×, stepping down
/// to body), mono code with a quiet shade, an italic-indented blockquote, a
/// blue-underline hyperlink char style, a table grid.
const STYLES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Georgia" w:hAnsi="Georgia" w:cs="Georgia"/><w:sz w:val="22"/><w:szCs w:val="22"/><w:lang w:val="en-US"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="300" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="character" w:default="1" w:styleId="DefaultParagraphFont"><w:name w:val="Default Paragraph Font"/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="240" w:after="120"/></w:pPr><w:rPr><w:b/><w:sz w:val="40"/><w:szCs w:val="40"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="200" w:after="100"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="180" w:after="80"/></w:pPr><w:rPr><w:b/><w:sz w:val="28"/><w:szCs w:val="28"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading4"><w:name w:val="heading 4"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="160" w:after="80"/></w:pPr><w:rPr><w:b/><w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading5"><w:name w:val="heading 5"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="160" w:after="80"/></w:pPr><w:rPr><w:b/><w:sz w:val="22"/><w:szCs w:val="22"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading6"><w:name w:val="heading 6"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="160" w:after="80"/></w:pPr><w:rPr><w:b/><w:color w:val="666666"/><w:sz w:val="22"/><w:szCs w:val="22"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:pPr><w:spacing w:before="120" w:after="120"/><w:ind w:left="720"/><w:pBdr><w:left w:val="single" w:sz="18" w:space="8" w:color="D0D0D0"/></w:pBdr></w:pPr><w:rPr><w:i/><w:color w:val="5A5A5A"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Code"><w:name w:val="Code Block"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:pPr><w:keepLines/><w:spacing w:before="120" w:after="120" w:line="240" w:lineRule="auto"/><w:ind w:left="120" w:right="120"/><w:shd w:val="clear" w:color="auto" w:fill="F4F4F2"/></w:pPr><w:rPr><w:rFonts w:ascii="Consolas" w:hAnsi="Consolas" w:cs="Consolas"/><w:sz w:val="20"/><w:szCs w:val="20"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:after="60"/><w:contextualSpacing/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="TableCellText"><w:name w:val="Table Cell Text"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr></w:style><w:style w:type="character" w:styleId="Hyperlink"><w:name w:val="Hyperlink"/><w:basedOn w:val="DefaultParagraphFont"/><w:rPr><w:color w:val="2244AA"/><w:u w:val="single"/></w:rPr></w:style><w:style w:type="character" w:styleId="CodeChar"><w:name w:val="Code Char"/><w:basedOn w:val="DefaultParagraphFont"/><w:rPr><w:rFonts w:ascii="Consolas" w:hAnsi="Consolas" w:cs="Consolas"/><w:sz w:val="20"/></w:rPr></w:style><w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style><w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:basedOn w:val="TableNormal"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="AAAAAA"/><w:left w:val="single" w:sz="4" w:space="0" w:color="AAAAAA"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="AAAAAA"/><w:right w:val="single" w:sz="4" w:space="0" w:color="AAAAAA"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="AAAAAA"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="AAAAAA"/></w:tblBorders></w:tblPr></w:style></w:styles>"#;

/// XML text-node escaping (`&`, `<`, `>`).
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// XML attribute-value escaping (adds `"`).
fn attr_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}
