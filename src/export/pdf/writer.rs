//! Minimal binary-safe PDF 1.7 object writer. Object IDs are planned up front;
//! every offset and stream length is derived from the bytes actually emitted.

use std::fmt::Write as _;

use super::fonts::{FontRole, ROLES, asset, descriptor, glyph_widths, role_index};
use super::layout::{GlyphOp, Layout, Op, PAGE_H, PAGE_W};

const FONT_BASE: u32 = 3;
const FONT_OBJECTS: u32 = 5;

pub(super) fn emit(layout: &Layout, metadata: &[u8]) -> Vec<u8> {
    let mut next = FONT_BASE + FONT_OBJECTS * ROLES.len() as u32;
    let mut image_ids = Vec::new();
    for image in &layout.images {
        let main = next;
        next += 1;
        let alpha = image.alpha.as_ref().map(|_| {
            let id = next;
            next += 1;
            id
        });
        image_ids.push((main, alpha));
    }
    let metadata_id = next;
    next += 1;
    struct PageIds {
        content: u32,
        annots: Vec<u32>,
        page: u32,
    }
    let mut page_ids = Vec::new();
    for page in &layout.pages {
        let content = next;
        next += 1;
        let annots = page
            .links
            .iter()
            .map(|_| {
                let id = next;
                next += 1;
                id
            })
            .collect();
        let page = next;
        next += 1;
        page_ids.push(PageIds {
            content,
            annots,
            page,
        });
    }

    let mut objects: Vec<Option<Vec<u8>>> = vec![None; next as usize];
    objects[1] =
        Some(format!("<< /Type /Catalog /Pages 2 0 R /Metadata {metadata_id} 0 R >>").into_bytes());
    let kids = page_ids
        .iter()
        .map(|p| format!("{} 0 R", p.page))
        .collect::<Vec<_>>()
        .join(" ");
    objects[2] = Some(
        format!(
            "<< /Type /Pages /Count {} /Kids [{kids}] >>",
            page_ids.len()
        )
        .into_bytes(),
    );

    for role in ROLES {
        write_font_objects(&mut objects, role, &layout.unicode[role_index(role)]);
    }
    for (index, image) in layout.images.iter().enumerate() {
        let (main, alpha) = image_ids[index];
        if let (Some(id), Some(data)) = (alpha, image.alpha.as_ref()) {
            objects[id as usize] = Some(stream(
                &format!(
                    "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 8",
                    image.width, image.height
                ),
                data,
            ));
        }
        let filter = if image.jpeg {
            " /Filter /DCTDecode"
        } else {
            ""
        };
        let smask = alpha.map_or(String::new(), |id| format!(" /SMask {id} 0 R"));
        objects[main as usize] = Some(stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {} /BitsPerComponent 8{filter}{smask}",
                image.width, image.height, image.color_space
            ),
            &image.data,
        ));
    }
    objects[metadata_id as usize] = Some(stream("/Type /Metadata /Subtype /XML", metadata));

    let font_resources = ROLES
        .iter()
        .enumerate()
        .map(|(i, _)| format!("/F{} {} 0 R", i + 1, font_type0_id(i)))
        .collect::<Vec<_>>()
        .join(" ");
    let image_resources = image_ids
        .iter()
        .enumerate()
        .map(|(i, (id, _))| format!("/Im{} {id} 0 R", i + 1))
        .collect::<Vec<_>>()
        .join(" ");
    for (pi, page) in layout.pages.iter().enumerate() {
        let ids = &page_ids[pi];
        let content = page_content(page, layout);
        objects[ids.content as usize] = Some(stream("", &content));
        for (link, id) in page.links.iter().zip(&ids.annots) {
            let bottom = PAGE_H - link.y - link.h;
            objects[*id as usize] = Some(format!(
                "<< /Type /Annot /Subtype /Link /Rect [{:.3} {:.3} {:.3} {:.3}] /Border [0 0 0] /A << /S /URI /URI <{}> >> >>",
                link.x, bottom, link.x+link.w, bottom+link.h, hex(link.url.as_bytes())).into_bytes());
        }
        let annots = if ids.annots.is_empty() {
            String::new()
        } else {
            format!(
                " /Annots [{}]",
                ids.annots
                    .iter()
                    .map(|id| format!("{id} 0 R"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        };
        objects[ids.page as usize] = Some(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.3} {:.3}] /Resources << /Font << {font_resources} >> /XObject << {image_resources} >> >> /Contents {} 0 R{annots} >>",
            PAGE_W, PAGE_H, ids.content).into_bytes());
    }
    finish(objects)
}

fn font_type0_id(index: usize) -> u32 {
    FONT_BASE + index as u32 * FONT_OBJECTS
}

fn write_font_objects(
    objects: &mut [Option<Vec<u8>>],
    role: FontRole,
    unicode: &std::collections::BTreeMap<u16, String>,
) {
    let index = role_index(role);
    let base = font_type0_id(index);
    let a = asset(role);
    let cid = base + 1;
    let desc_id = base + 2;
    let file = base + 3;
    let cmap = base + 4;
    objects[base as usize] = Some(format!("<< /Type /Font /Subtype /Type0 /BaseFont /{} /Encoding /Identity-H /DescendantFonts [{cid} 0 R] /ToUnicode {cmap} 0 R >>", a.pdf_name).into_bytes());
    let widths = glyph_widths(role)
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    objects[cid as usize] = Some(format!("<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {desc_id} 0 R /CIDToGIDMap /Identity /DW 1000 /W [0 [{widths}]] >>", a.pdf_name).into_bytes());
    let d = descriptor(role);
    let fixed = matches!(role, FontRole::Mono | FontRole::MonoBold);
    let flags = if fixed { 33 } else { 34 };
    let stem = if matches!(role, FontRole::SerifBold | FontRole::MonoBold) {
        140
    } else {
        80
    };
    objects[desc_id as usize] = Some(format!("<< /Type /FontDescriptor /FontName /{} /Flags {flags} /FontBBox [{} {} {} {}] /ItalicAngle 0 /Ascent {} /Descent {} /CapHeight {} /StemV {stem} /FontFile2 {file} 0 R >>",a.pdf_name,d.bbox[0],d.bbox[1],d.bbox[2],d.bbox[3],d.ascent,d.descent,d.cap_height).into_bytes());
    objects[file as usize] = Some(stream(&format!("/Length1 {}", a.bytes.len()), a.bytes));
    objects[cmap as usize] = Some(stream("", to_unicode(a.pdf_name, unicode).as_bytes()));
}

fn to_unicode(name: &str, map: &std::collections::BTreeMap<u16, String>) -> String {
    let mut out = format!(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /{name}-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n"
    );
    let entries = map
        .iter()
        .filter(|(_, s)| !s.is_empty())
        .collect::<Vec<_>>();
    for chunk in entries.chunks(100) {
        writeln!(out, "{} beginbfchar", chunk.len()).unwrap();
        for (id, text) in chunk {
            writeln!(out, "<{id:04X}> <{}>", utf16_hex(text, false)).unwrap();
        }
        out.push_str("endbfchar\n");
    }
    out.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    out
}

fn page_content(page: &super::layout::Page, layout: &Layout) -> Vec<u8> {
    let mut out = String::new();
    for op in &page.ops {
        match op {
            Op::Rect { x, y, w, h, gray } => writeln!(
                out,
                "q {:.3} g {:.3} {:.3} {:.3} {:.3} re f Q",
                gray,
                x,
                PAGE_H - y - h,
                w,
                h
            )
            .unwrap(),
            Op::Line {
                x1,
                y1,
                x2,
                y2,
                width,
                gray,
            } => writeln!(
                out,
                "q {:.3} G {:.3} w {:.3} {:.3} m {:.3} {:.3} l S Q",
                gray,
                width,
                x1,
                PAGE_H - y1,
                x2,
                PAGE_H - y2
            )
            .unwrap(),
            Op::Glyph(g) => write_glyph(&mut out, g),
            Op::Image { index, x, y, w, h } => {
                let alt = &layout.images[*index].alt;
                writeln!(out, "/Figure << /Alt <{}> >> BDC", utf16_hex(alt, true)).unwrap();
                writeln!(
                    out,
                    "q {:.3} 0 0 {:.3} {:.3} {:.3} cm /Im{} Do Q",
                    w,
                    h,
                    x,
                    PAGE_H - y - h,
                    index + 1
                )
                .unwrap();
                out.push_str("EMC\n");
            }
        }
    }
    out.into_bytes()
}

fn write_glyph(out: &mut String, g: &GlyphOp) {
    let shear = if g.italic { 0.212_556 } else { 0.0 };
    let font = role_index(g.role) + 1;
    writeln!(
        out,
        "/Span << /ActualText <{}> >> BDC",
        utf16_hex(&g.actual, true)
    )
    .unwrap();
    writeln!(
        out,
        "BT /F{font} {:.3} Tf 1 0 {:.6} 1 {:.3} {:.3} Tm <{:04X}> Tj ET EMC",
        g.size,
        shear,
        g.x,
        PAGE_H - g.y,
        g.glyph_id
    )
    .unwrap();
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}
fn finish(objects: Vec<Option<Vec<u8>>>) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = vec![0usize; objects.len()];
    for id in 1..objects.len() {
        offsets[id] = out.len();
        out.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        out.extend_from_slice(objects[id].as_ref().expect("planned PDF object"));
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len()).as_bytes());
    for offset in offsets.iter().skip(1) {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len()
        )
        .as_bytes(),
    );
    out
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}
fn utf16_hex(text: &str, bom: bool) -> String {
    let mut out = String::new();
    if bom {
        out.push_str("FEFF");
    }
    for unit in text.encode_utf16() {
        write!(out, "{unit:04X}").unwrap();
    }
    out
}
