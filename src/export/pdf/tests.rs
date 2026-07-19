mod fixture;
mod parser;
mod semantic;

use std::collections::BTreeSet;
use std::path::PathBuf;

use ttf_parser::{Face, GlyphId, Permissions};

use super::fonts::{ASSETS, ROLES};
use super::layout::{MARGIN_X, MARGIN_Y, MEASURE, PAGE_H, PAGE_W, PROSE_CHARS};
use super::*;
use crate::export::model;
use fixture::{Images, NoImages};
use parser::{Pdf, hex_value_after, reference, refs_in_array};
use semantic::*;

fn rich_bytes() -> (String, Vec<u8>) {
    let markdown = fixture::markdown();
    let bytes = emit(&model::parse(&markdown), &Images::new());
    (markdown, bytes)
}

#[test]
fn pdf_has_exact_classic_xref_object_plan_pages_and_a4_geometry() {
    let (_, bytes) = rich_bytes();
    let pdf = Pdf::parse(&bytes);
    assert_eq!(
        pdf.object(1).offset,
        15,
        "catalog is the first object after the binary header"
    );
    assert!(
        pdf.object(1)
            .text()
            .starts_with("<< /Type /Catalog /Pages 2 0 R")
    );
    assert!(pdf.object(2).text().starts_with("<< /Type /Pages "));

    for (index, base) in [3, 8, 13, 18].into_iter().enumerate() {
        assert!(
            pdf.object(base).text().contains("/Subtype /Type0"),
            "font {index} Type0"
        );
        assert!(
            pdf.object(base + 1)
                .text()
                .contains("/Subtype /CIDFontType2")
        );
        assert!(
            pdf.object(base + 2)
                .text()
                .contains("/Type /FontDescriptor")
        );
        assert!(
            pdf.object(base + 2)
                .text()
                .contains(&format!("/FontFile2 {} 0 R", base + 3))
        );
        assert!(pdf.object(base + 3).stream().is_some());
        assert!(pdf.object(base + 4).stream().is_some());
    }
    assert!(
        pdf.object(23).text().contains("/Subtype /Image"),
        "first image is PNG RGB"
    );
    assert!(
        pdf.object(24).text().contains("/DeviceGray"),
        "PNG alpha follows its image"
    );
    assert!(
        pdf.object(25).text().contains("/DCTDecode"),
        "JPEG follows in encounter order"
    );
    assert!(
        pdf.object(26)
            .text()
            .contains("/Type /Metadata /Subtype /XML")
    );
    assert_eq!(reference(pdf.object(1).text(), "/Metadata "), 26);

    let pages = pdf.page_ids();
    assert!(
        pages.len() >= 3,
        "rich fixture must paginate automatically: {pages:?}"
    );
    let pages_text = pdf.object(2).text();
    assert!(pages_text.contains(&format!("/Count {}", pages.len())));
    assert_eq!(refs_in_array(pages_text, "/Kids ["), pages);

    let mut next = 27;
    for page_id in pages {
        let page = pdf.object(page_id).text();
        assert_eq!(
            reference(page.clone(), "/Contents "),
            next,
            "content precedes annotations/page"
        );
        next += 1;
        let annots = if page.contains("/Annots [") {
            refs_in_array(page.clone(), "/Annots [")
        } else {
            Vec::new()
        };
        for annot in annots {
            assert_eq!(annot, next, "annotation encounter order");
            assert!(pdf.object(annot).text().contains("/Subtype /Link"));
            next += 1;
        }
        assert_eq!(
            page_id, next,
            "page dictionary follows its content and annotations"
        );
        next += 1;
        assert!(page.contains("/MediaBox [0 0 595.276 841.890]"));
        assert!(page.contains("/Parent 2 0 R"));
    }
    assert_eq!(
        next as usize,
        pdf.objects.len() + 1,
        "no unplanned trailing objects"
    );
    assert_eq!(&pdf.bytes()[pdf.startxref..pdf.startxref + 5], b"xref\n");
    assert_eq!(
        (PAGE_W, PAGE_H, MARGIN_X, MARGIN_Y, MEASURE, PROSE_CHARS),
        (595.276, 841.890, 66.638, 56.693, 462.0, 70)
    );
}

#[test]
fn four_repository_fonts_are_per_document_glyph_subsets() {
    let (_, bytes) = rich_bytes();
    let pdf = Pdf::parse(&bytes);
    let inventory = crate::embedded_docs::FONT_LICENSES_MD;
    let ofl = crate::embedded_docs::FONT_OFL_TXT;
    assert!(ofl.contains("SIL OPEN FONT LICENSE Version 1.1"));

    for (index, asset) in ASSETS.iter().enumerate() {
        assert_eq!(asset.role, ROLES[index]);
        let base = 3 + index as u32 * 5;
        let type0 = pdf.object(base).text();
        let cid = pdf.object(base + 1).text();
        let descriptor = pdf.object(base + 2).text();
        assert!(type0.contains(&format!("/BaseFont /{}", asset.pdf_name)));
        assert!(type0.contains("/Encoding /Identity-H"));
        assert!(type0.contains(&format!("/ToUnicode {} 0 R", base + 4)));
        assert!(cid.contains("/CIDToGIDMap /Identity"));
        assert!(descriptor.contains(&format!("/FontName /{}", asset.pdf_name)));
        let embedded = pdf.object(base + 3).stream().unwrap();
        assert!(
            embedded.len() < asset.bytes.len() / 2,
            "{} subset is {} bytes versus {}-byte source",
            asset.pdf_name,
            embedded.len(),
            asset.bytes.len()
        );
        let subset = Face::parse(embedded, 0).expect("embedded subset remains valid TrueType");
        let face = Face::parse(asset.bytes, 0).expect(asset.pdf_name);
        assert_eq!(face.permissions(), Some(Permissions::Installable));
        assert!(face.is_outline_embedding_allowed());
        assert!(face.is_subsetting_allowed());
        assert!(face.tables().cmap.is_some());
        assert!(face.tables().glyf.is_some());
        assert!(face.tables().hmtx.is_some());
        let filename = match asset.pdf_name {
            "AWLBitter-Regular" => "Bitter-Regular.ttf",
            "AWLBitter-Bold" => "Bitter-Bold.ttf",
            "AWLIBMPlexMono-Light" => "IBMPlexMono-Light.ttf",
            "AWLIBMPlexMono-Bold" => "IBMPlexMono-Bold.ttf",
            other => panic!("unreviewed PDF face {other}"),
        };
        let row = inventory
            .lines()
            .find(|line| line.contains(&format!("`{filename}`")))
            .unwrap();
        assert!(row.contains("SIL OFL 1.1"), "license inventory row: {row}");

        assert_eq!(subset.number_of_glyphs(), face.number_of_glyphs());
        let start = cid.find("/W [").unwrap() + "/W [".len();
        let end = cid[start..].find("] >>").unwrap() + start;
        let values = cid[start..end]
            .replace(['[', ']'], " ")
            .split_whitespace()
            .map(|value| value.parse::<u16>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(values.len() % 2, 0);
        let upm = u32::from(face.units_per_em());
        for pair in values.chunks_exact(2) {
            let id = pair[0];
            let width = pair[1];
            let raw = u32::from(face.glyph_hor_advance(GlyphId(id)).unwrap_or(0));
            assert_eq!(
                width,
                ((raw * 1000 + upm / 2) / upm) as u16,
                "hmtx width {id}"
            );
            assert_eq!(
                subset.glyph_hor_advance(GlyphId(id)),
                face.glyph_hor_advance(GlyphId(id)),
                "subset hmtx {id}"
            );
        }
        let cmap = std::str::from_utf8(pdf.object(base + 4).stream().unwrap()).unwrap();
        assert!(cmap.contains("begincmap"));
        assert!(
            cmap.contains("beginbfchar"),
            "used glyphs have ToUnicode entries"
        );
    }
}

#[test]
fn png_alpha_jpeg_and_link_are_backed_by_real_pdf_objects() {
    let (_, bytes) = rich_bytes();
    let pdf = Pdf::parse(&bytes);
    let png = pdf.object(23);
    assert!(
        png.text()
            .contains("/Width 2 /Height 1 /ColorSpace /DeviceRGB")
    );
    assert!(png.text().contains("/SMask 24 0 R"));
    assert_eq!(png.stream().unwrap(), &[0x20, 0x40, 0x60, 0x80, 0x60, 0x40]);
    assert_eq!(pdf.object(24).stream().unwrap(), &[0x80, 0xff]);

    let jpeg = pdf.object(25);
    assert!(
        jpeg.text()
            .contains("/Width 120 /Height 48 /ColorSpace /DeviceRGB")
    );
    assert!(jpeg.text().contains("/Filter /DCTDecode"));
    assert!(!jpeg.text().contains("/SMask"));
    assert_eq!(jpeg.stream().unwrap(), fixture::jpeg());

    let annotations = pdf
        .objects
        .values()
        .filter(|object| object.text().contains("/Subtype /Link"))
        .collect::<Vec<_>>();
    assert_eq!(annotations.len(), 1);
    let annot = annotations[0].text();
    let uri = hex_value_after(&annot, "/URI <");
    assert_eq!(
        String::from_utf8(hex_bytes(uri)).unwrap(),
        "https://example.com/path?q=1&r=2"
    );
    let rect = numbers_between(&annot, "/Rect [", ']');
    assert_eq!(rect.len(), 4);
    assert!(rect[0] >= MARGIN_X && rect[2] <= PAGE_W - MARGIN_X + 0.01);
    assert!(rect[1] >= MARGIN_Y && rect[3] <= PAGE_H - MARGIN_Y + 0.01);

    let content = pdf.page_streams().concat();
    let content = std::str::from_utf8(&content).unwrap();
    assert!(content.contains("/Im1 Do"));
    assert!(content.contains("/Im2 Do"));
    assert!(content.contains("/Alt <FEFF0061006C00700068006100200070006900630074007500720065>"));
    assert!(content.contains("/Alt <FEFF006A007000650067002000700068006F0074006F>"));
}

#[test]
fn manifest_roster_and_actual_page_text_cover_every_model_element() {
    let (markdown, bytes) = rich_bytes();
    let doc = model::parse(&markdown);
    let pdf = Pdf::parse(&bytes);
    let metadata_id = reference(pdf.object(1).text(), "/Metadata ");
    let metadata = std::str::from_utf8(pdf.object(metadata_id).stream().unwrap()).unwrap();
    let expected = expected_kinds(&doc);
    let actual = manifest_elements(metadata);
    assert_eq!(
        actual
            .iter()
            .map(|(_, kind)| kind.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    for (index, (id, _)) in actual.iter().enumerate() {
        assert_eq!(
            id,
            &format!("e{:04}", index + 1),
            "stable pre-order semantic ID"
        );
    }
    assert!(metadata.contains("kind=\"heading\" level=\"6\""));
    assert!(metadata.contains("kind=\"list\" ordered=\"true\" start=\"3\""));
    assert!(metadata.contains("kind=\"item\" task=\"open\""));
    assert!(metadata.contains("kind=\"item\" task=\"checked\""));
    assert!(metadata.contains("kind=\"table\" columns=\"4\""));
    assert!(metadata.contains("kind=\"table-head\" align=\"none,left,center,right\""));
    assert!(metadata.contains("kind=\"link\" url=\"https://example.com/path?q=1&amp;r=2\""));
    assert!(metadata.contains(
        "kind=\"image\" src=\"alpha.png\" alt=\"alpha picture\" resolved=\"true\" width=\"48\""
    ));
    assert!(
        metadata.contains("kind=\"image\" src=\"photo.jpg\" alt=\"jpeg photo\" resolved=\"true\"")
    );
    assert!(metadata.contains("kind=\"image\" src=\"missing.png\" alt=\"missing picture\" resolved=\"false\" fallback=\"alt-text\""));
    assert!(metadata.contains("fallback=\"U+1F989:U+25A1\""));
    assert!(
        !metadata.contains("excluded"),
        "frontmatter never enters the manifest"
    );

    let page_text = recover_page_text(&pdf).join("");
    let fragments = text_fragments(&doc);
    assert!(
        fragments.len() >= 85,
        "fixture coverage net is non-vacuous: {}",
        fragments.len()
    );
    for fragment in fragments {
        let fragment = fragment.trim_end_matches('\n');
        if !fragment.is_empty() {
            assert!(
                page_text.contains(fragment),
                "painted page text dropped {fragment:?}\nrecovered prefix: {:?}",
                &page_text[..page_text.len().min(800)]
            );
        }
    }
    assert!(
        page_text.contains('🦉'),
        "ActualText restores the unsupported scalar"
    );

    let block_roster = doc
        .blocks
        .iter()
        .flat_map(block_roster)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        block_roster,
        BTreeSet::from([
            "blockquote",
            "codeblock",
            "heading",
            "list",
            "paragraph",
            "rule",
            "table"
        ])
    );
    let inline_roster = all_inlines(&doc)
        .into_iter()
        .map(inline_kind)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        inline_roster,
        BTreeSet::from([
            "code",
            "emphasis",
            "hardbreak",
            "highlight",
            "image",
            "link",
            "softbreak",
            "strikethrough",
            "strong",
            "text"
        ])
    );
}

#[test]
fn pagination_stays_in_margins_and_prevents_heading_or_paragraph_orphans() {
    let (_, bytes) = rich_bytes();
    let pdf = Pdf::parse(&bytes);
    assert!(pdf.page_ids().len() >= 3);
    for stream in pdf.page_streams() {
        let content = std::str::from_utf8(stream).unwrap();
        for line in content.lines() {
            let words = line.split_whitespace().collect::<Vec<_>>();
            if let Some(tm) = words.iter().position(|word| *word == "Tm") {
                let x = words[tm - 2].parse::<f32>().unwrap();
                let y = words[tm - 1].parse::<f32>().unwrap();
                assert!(
                    (MARGIN_X - 0.01..=PAGE_W - MARGIN_X + 0.01).contains(&x),
                    "glyph x={x}"
                );
                assert!(
                    (MARGIN_Y - 0.01..=PAGE_H - MARGIN_Y + 0.01).contains(&y),
                    "glyph y={y}"
                );
            }
            if let Some(cm) = words.iter().position(|word| *word == "cm") {
                let x = words[cm - 2].parse::<f32>().unwrap();
                let y = words[cm - 1].parse::<f32>().unwrap();
                let w = words[cm - 6].parse::<f32>().unwrap();
                let h = words[cm - 3].parse::<f32>().unwrap();
                assert!(x >= MARGIN_X - 0.01 && x + w <= PAGE_W - MARGIN_X + 0.01);
                assert!(y >= MARGIN_Y - 0.01 && y + h <= PAGE_H - MARGIN_Y + 0.01);
            }
        }
    }

    let mut heading_boundary = None;
    for count in 24..34 {
        let prefix = pad_paragraphs(count);
        let markdown = format!("{prefix}## KEEP HEADING\n\nFOLLOWER stays with heading.\n");
        let bytes = emit(&model::parse(&markdown), &NoImages);
        let pages = recover_page_text(&Pdf::parse(&bytes));
        let previous = page_of(&pages, &format!("pad {:02}", count - 1));
        let heading = page_of(&pages, "KEEP HEADING");
        let follower = page_of(&pages, "FOLLOWER stays with heading");
        if heading > previous {
            assert_eq!(
                heading, follower,
                "heading must keep at least the next line"
            );
            heading_boundary = Some(count);
            break;
        }
    }
    assert!(
        heading_boundary.is_some(),
        "fixture search reached a heading page boundary"
    );

    let mut paragraph_boundary = None;
    for count in 24..34 {
        let prefix = pad_paragraphs(count);
        let markdown = format!(
            "{prefix}---\n\nSPLIT ALPHA begins a deliberately long paragraph whose second wrapped line must never be orphaned behind its first line near the bottom margin; SPLIT OMEGA closes it.\n"
        );
        let bytes = emit(&model::parse(&markdown), &NoImages);
        let pages = recover_page_text(&Pdf::parse(&bytes));
        let previous = page_of(&pages, &format!("pad {:02}", count - 1));
        let first = page_of(&pages, "SPLIT ALPHA");
        let last = page_of(&pages, "SPLIT OMEGA");
        if first > previous {
            assert_eq!(
                first, last,
                "a short paragraph moves whole instead of orphaning one line"
            );
            paragraph_boundary = Some(count);
            break;
        }
    }
    assert!(
        paragraph_boundary.is_some(),
        "fixture search reached a paragraph page boundary"
    );
}

#[test]
fn pdf_golden_and_public_format_path_are_byte_identical() {
    let markdown = fixture::markdown();
    let direct = crate::export::to_pdf(&markdown, &Images::new());
    let second = crate::export::to_pdf(&markdown, &Images::new());
    assert_eq!(direct, second, "two complete exports are byte-identical");
    assert_eq!(
        crate::export::to_bytes(&markdown, crate::export::Format::Pdf, &Images::new()),
        direct
    );
    assert_eq!(crate::export::Format::Pdf.ext(), "pdf");
    let parsed = Pdf::parse(&direct);
    assert!(parsed.page_ids().len() >= 3);
    assert!(parsed.object(6).stream().unwrap().len() < ASSETS[0].bytes.len() / 2);
    golden("rich.pdf", &direct);
}

fn numbers_between(text: &str, marker: &str, end_char: char) -> Vec<f32> {
    let start = text.find(marker).unwrap() + marker.len();
    let end = text[start..].find(end_char).unwrap() + start;
    text[start..end]
        .split_whitespace()
        .map(|n| n.parse().unwrap())
        .collect()
}

fn hex_bytes(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn golden(name: &str, got: &[u8]) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/export/testdata")
        .join(name);
    if std::env::var("AWL_BLESS").is_ok() {
        std::fs::write(path, got).unwrap();
        return;
    }
    let want = std::fs::read(&path).unwrap_or_else(|_| {
        panic!("golden {name} missing; bless only after the PDF law suite passes")
    });
    assert_eq!(got, want, "{name} exact-byte golden drift");
}
