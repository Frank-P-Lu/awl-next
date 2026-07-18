//! Export gates: a rich fixture exercising EVERY covered element, exported to
//! byte-stable golden `.docx` + `.html`; a minimal STORED-zip reader that
//! round-trips the docx (every entry parses, every CRC-32 validates); a small
//! dev-only XML well-formedness checker over the OOXML parts (no runtime dep);
//! and determinism (two exports are byte-identical).
//!
//! The golden files live under `src/export/testdata/` and are read at RUNTIME
//! (not `include_bytes!`, so a first `AWL_BLESS=1` run can create them). Re-bless
//! with `AWL_BLESS=1 cargo test export::` after an intentional format change.

use super::model::{self, Align, Block, ExportImage, ImageMime, ImageSource, Inline};
use super::zip::crc32;
use super::{Format, to_bytes, to_docx, to_html, to_pdf};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A no-op image resolver: every image degrades to alt text. The
/// unresolvable-image fallback path's test double.
struct NoImages;
impl ImageSource for NoImages {
    fn resolve(&self, _src: &str) -> Option<ExportImage> {
        None
    }
}

// --- The rich fixture -------------------------------------------------------

/// Every covered construct: frontmatter (excluded), all heading levels,
/// bold/italic/strike/highlight, inline + fenced code, a link, bullet/numbered/
/// task lists (with nesting), a blockquote, a thematic break, a GFM table, and
/// an embedded image.
const FIXTURE: &str = "\
---
lang: en
title: ignored
---
# Export Fixture

A paragraph with **bold**, *italic*, ~~struck~~, ==highlighted==, and `inline code`.
Here is a [link](https://example.com/path?q=1&r=2).

## Section Two

Body text under a section, with a soft
break across two source lines.

### Subsection

- first bullet
- second bullet
  - nested bullet
- third bullet

1. one
2. two
3. three

- [ ] open task
- [x] done task

> A quoted line.
> A second quoted line.

---

| Left | Center | Right |
|:-----|:------:|------:|
| a | b | c |
| dee | eee | eff |

```rust
fn main() {
    println!(\"hello\");
}
```

![a picture|48](assets/pic.png)

The end.
";

/// A tiny, deterministic PNG (6×4, solid) for the fixture image — built through
/// the app's own PNG encoder so it is a real, sniffable file.
fn fixture_png() -> Vec<u8> {
    let (w, h) = (6usize, 4usize);
    let rgba = vec![0x40u8; w * h * 4];
    crate::paste_image::encode_rgba_png(w, h, &rgba).expect("encode fixture png")
}

/// The fixture's image resolver: `assets/pic.png` → the tiny PNG; anything else
/// (the remote link is not an image) unresolved.
struct FixtureImages(Vec<u8>);
impl ImageSource for FixtureImages {
    fn resolve(&self, src: &str) -> Option<ExportImage> {
        if src == "assets/pic.png" {
            let (width, height, mime) = model::sniff_image(&self.0)?;
            Some(ExportImage {
                bytes: self.0.clone(),
                width,
                height,
                mime,
            })
        } else {
            None
        }
    }
}

fn fixture_images() -> FixtureImages {
    FixtureImages(fixture_png())
}

// --- Primitive checks -------------------------------------------------------

#[test]
fn crc32_matches_the_standard_check_value() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b""), 0);
}

#[test]
fn base64_matches_rfc_vectors() {
    use super::html::base64_for_test as b64;
    assert_eq!(b64(b""), "");
    assert_eq!(b64(b"f"), "Zg==");
    assert_eq!(b64(b"fo"), "Zm8=");
    assert_eq!(b64(b"foo"), "Zm9v");
    assert_eq!(b64(b"foobar"), "Zm9vYmFy");
}

#[test]
fn sniff_reads_png_and_jpeg_dimensions() {
    let png = fixture_png();
    assert_eq!(model::sniff_image(&png), Some((6, 4, ImageMime::Png)));
    // A hand-built minimal JPEG header: SOI, then an SOF0 marker giving 3×7.
    let jpeg = [
        0xFF, 0xD8, // SOI
        0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0x07, 0x00, 0x03, 0x03, 0x01, 0x22, 0x00, 0x02, 0x11,
        0x01, 0x03, 0x11, 0x01,
    ];
    assert_eq!(model::sniff_image(&jpeg), Some((3, 7, ImageMime::Jpeg)));
    assert_eq!(model::sniff_image(b"not an image"), None);
}

// --- The parse walk ---------------------------------------------------------

#[test]
fn frontmatter_is_excluded_and_title_is_the_first_heading() {
    let doc = model::parse(FIXTURE);
    assert_eq!(doc.title.as_deref(), Some("Export Fixture"));
    // The frontmatter `title: ignored` never becomes a block.
    let flat = format!("{:?}", doc.blocks);
    assert!(
        !flat.contains("ignored"),
        "frontmatter leaked into the body"
    );
}

#[test]
fn highlight_splits_into_its_own_inline() {
    let doc = model::parse("plain ==hi== plain\n");
    let Block::Paragraph(inlines) = &doc.blocks[0] else {
        panic!("expected paragraph")
    };
    assert!(
        inlines.iter().any(|i| matches!(i, Inline::Highlight(_))),
        "no Highlight inline: {inlines:?}"
    );
    // A lone/odd `=` stays literal.
    let doc2 = model::parse("a = b and == unclosed\n");
    let Block::Paragraph(inl2) = &doc2.blocks[0] else {
        panic!()
    };
    assert!(!inl2.iter().any(|i| matches!(i, Inline::Highlight(_))));
}

#[test]
fn tables_carry_alignment_and_task_items_carry_state() {
    let doc = model::parse(FIXTURE);
    let table = doc.blocks.iter().find_map(|b| match b {
        Block::Table(t) => Some(t),
        _ => None,
    });
    let table = table.expect("a table block");
    assert_eq!(table.aligns, vec![Align::Left, Align::Center, Align::Right]);
    assert_eq!(table.head.len(), 3);
    assert_eq!(table.rows.len(), 2);

    // The task list's two items carry Some(false)/Some(true).
    let mut tasks = Vec::new();
    fn collect(blocks: &[Block], out: &mut Vec<Option<bool>>) {
        for b in blocks {
            if let Block::List(l) = b {
                for it in &l.items {
                    out.push(it.task);
                    collect(&it.blocks, out);
                }
            }
        }
    }
    collect(&doc.blocks, &mut tasks);
    assert!(tasks.contains(&Some(false)) && tasks.contains(&Some(true)));
}

/// LAW: a TIGHT list item (no blank line between items — the dominant form)
/// emits its inlines BARE, with no wrapping paragraph. The parse walk MUST
/// collect those into an implicit paragraph, and every emitter MUST carry the
/// text through. This is the guard for the tight-list content-loss bug (bare
/// item inlines fell through `push_inline` and were silently dropped, blessed
/// into empty `<li>`s and glyph-only docx runs). Every item's own words must
/// survive into the tree AND into both emitted documents.
#[test]
fn tight_list_item_text_survives_into_both_emitters() {
    // The fixture's three tight lists (bullets, numbered, tasks).
    let words = [
        "first bullet",
        "second bullet",
        "nested bullet",
        "third bullet",
        "one",
        "two",
        "three",
        "open task",
        "done task",
    ];

    // (a) The neutral tree: gather every list item's plain text; each word is
    //     present and non-empty (no item collapsed to an empty block list).
    let doc = model::parse(FIXTURE);
    let mut item_texts = Vec::new();
    fn walk(blocks: &[Block], out: &mut Vec<String>) {
        for b in blocks {
            if let Block::List(l) = b {
                for it in &l.items {
                    // An item that lost its inlines has NO paragraph — this is
                    // exactly what the bug produced.
                    let text: String = it
                        .blocks
                        .iter()
                        .filter_map(|blk| match blk {
                            Block::Paragraph(inl) => Some(model::plain_text(inl)),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    out.push(text);
                    walk(&it.blocks, out);
                }
            } else if let Block::BlockQuote(inner) = b {
                walk(inner, out);
            }
        }
    }
    walk(&doc.blocks, &mut item_texts);
    let joined = item_texts.join("\u{1}");
    for w in words {
        assert!(
            item_texts.iter().any(|t| t.contains(w)),
            "list item text {w:?} dropped from the parse tree (items: {item_texts:?})"
        );
    }
    // No item is text-empty (every tight item carried at least its own words).
    assert!(
        !joined.split('\u{1}').any(|t| t.trim().is_empty()),
        "an item rendered with empty text: {item_texts:?}"
    );

    // (b) Both emitters carry every word through.
    let html = to_html(FIXTURE, &fixture_images());
    let docx = to_docx(FIXTURE, &fixture_images());
    let doc_xml = String::from_utf8(unzip_stored(&docx)["word/document.xml"].clone()).unwrap();
    for w in words {
        assert!(html.contains(w), "HTML export dropped list item text {w:?}");
        assert!(
            doc_xml.contains(w),
            "DOCX export dropped list item text {w:?}"
        );
    }
}

/// LAW: the PER-ELEMENT coverage sweep. Walk the parsed fixture tree and gather
/// EVERY leaf text fragment across the WHOLE covered surface — heading, paragraph,
/// blockquote, list item (nested included), table head + body cell, fenced-code
/// line — then assert each one survives into BOTH emitted documents: the docx
/// `<w:t>` run text AND the HTML body. This generalizes the tight-list guard
/// (`tight_list_item_text_survives_into_both_emitters`, the c9bead0 bug class)
/// from lists to the entire element roster, so ANY element the emitter silently
/// drops — not just the one bug we already caught — fails a test.
///
/// Image alt is deliberately excluded: a RESOLVED image carries its alt in a
/// `descr`/`alt` ATTRIBUTE, not a text run, so it is not a `<w:t>` fragment.
#[test]
fn every_fixture_text_fragment_survives_into_both_emitters() {
    /// Gather the leaf text of an inline subtree (skips resolved-image alt).
    fn inline_fragments(inlines: &[Inline], out: &mut Vec<String>) {
        for inl in inlines {
            match inl {
                Inline::Text(t) | Inline::Code(t) => out.push(t.clone()),
                Inline::Strong(c)
                | Inline::Emphasis(c)
                | Inline::Strikethrough(c)
                | Inline::Highlight(c)
                | Inline::Link { children: c, .. } => inline_fragments(c, out),
                Inline::Image { .. } | Inline::SoftBreak | Inline::HardBreak => {}
            }
        }
    }
    fn block_fragments(blocks: &[Block], out: &mut Vec<String>) {
        for b in blocks {
            match b {
                Block::Heading { inlines, .. } | Block::Paragraph(inlines) => {
                    inline_fragments(inlines, out)
                }
                Block::BlockQuote(inner) => block_fragments(inner, out),
                Block::CodeBlock { code, .. } => {
                    // The docx emitter splits code into one run per line
                    // (separated by `<w:br/>`), so assert at line granularity.
                    for line in code.split('\n') {
                        out.push(line.to_string());
                    }
                }
                Block::List(list) => {
                    for it in &list.items {
                        block_fragments(&it.blocks, out);
                    }
                }
                Block::Table(t) => {
                    for cell in &t.head {
                        inline_fragments(cell, out);
                    }
                    for row in &t.rows {
                        for cell in row {
                            inline_fragments(cell, out);
                        }
                    }
                }
                Block::Rule => {}
            }
        }
    }

    let doc = model::parse(FIXTURE);
    let mut fragments = Vec::new();
    block_fragments(&doc.blocks, &mut fragments);
    // The fixture is rich enough that the sweep is a real net (guards against a
    // future refactor that silently empties the collection and passes vacuously).
    assert!(
        fragments.iter().filter(|f| !f.trim().is_empty()).count() >= 25,
        "fixture fragment collection looks too small ({} non-empty): {fragments:?}",
        fragments.iter().filter(|f| !f.trim().is_empty()).count()
    );

    let html = to_html(FIXTURE, &fixture_images());
    let docx = to_docx(FIXTURE, &fixture_images());
    let doc_xml = String::from_utf8(unzip_stored(&docx)["word/document.xml"].clone()).unwrap();
    let docx_text = docx_run_text(&doc_xml);

    for frag in &fragments {
        let f = frag.trim();
        if f.is_empty() {
            continue;
        }
        assert!(
            docx_text.contains(f),
            "DOCX dropped text fragment {f:?}\n(all run text: {docx_text:?})"
        );
        assert!(html.contains(f), "HTML dropped text fragment {f:?}");
    }
}

/// Concatenate the text content of every `<w:t xml:space="preserve">…</w:t>` run
/// in `document.xml`, un-escaping the three XML text entities, so a coverage
/// assertion can search the ACTUAL emitted run text (not attributes/markup).
fn docx_run_text(doc_xml: &str) -> String {
    const OPEN: &str = "<w:t xml:space=\"preserve\">";
    const CLOSE: &str = "</w:t>";
    let mut out = String::new();
    let mut rest = doc_xml;
    while let Some(i) = rest.find(OPEN) {
        rest = &rest[i + OPEN.len()..];
        match rest.find(CLOSE) {
            Some(j) => {
                out.push_str(&rest[..j]);
                rest = &rest[j + CLOSE.len()..];
            }
            None => break,
        }
    }
    // Un-escape in the order that avoids double-decoding `&amp;lt;` → `<`.
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

// --- HTML emitter -----------------------------------------------------------

#[test]
fn html_has_the_expected_structure() {
    let html = to_html(FIXTURE, &fixture_images());
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("<title>Export Fixture</title>"));
    assert!(html.contains("<h1>Export Fixture</h1>"));
    assert!(html.contains("<strong>bold</strong>"));
    assert!(html.contains("<em>italic</em>"));
    assert!(html.contains("<del>struck</del>"));
    assert!(html.contains("<mark>highlighted</mark>"));
    assert!(html.contains("<code>inline code</code>"));
    assert!(html.contains("href=\"https://example.com/path?q=1&amp;r=2\""));
    assert!(html.contains("<blockquote>"));
    assert!(html.contains("<hr>"));
    assert!(html.contains("<table>"));
    assert!(html.contains("text-align:center"));
    assert!(html.contains("<pre><code class=\"language-rust\">"));
    assert!(html.contains("type=\"checkbox\" disabled checked"));
    assert!(html.contains("<img src=\"data:image/png;base64,"));
    assert!(html.contains("width=\"48\"")); // the |48 size hint won
    assert!(html.contains("@page"));
    assert!(html.contains("break-inside: avoid"));
    // An unresolvable image degrades to alt text, never a broken embed.
    let html_no_img = to_html("![missing](nope.png)\n", &NoImages);
    assert!(!html_no_img.contains("<img"));
    assert!(html_no_img.contains("missing"));
}

// --- DOCX emitter + container -----------------------------------------------

/// A minimal STORED-zip reader: parse the end-of-central-directory + central
/// directory, then each local entry, validating that every entry is STORED and
/// its CRC-32 matches its bytes. Returns name → bytes.
fn unzip_stored(archive: &[u8]) -> BTreeMap<String, Vec<u8>> {
    // Locate EOCD (fixed 22 bytes here — no archive comment).
    let eocd = archive.len() - 22;
    assert_eq!(
        &archive[eocd..eocd + 4],
        &0x0605_4b50u32.to_le_bytes(),
        "no EOCD sig"
    );
    let count = u16::from_le_bytes([archive[eocd + 10], archive[eocd + 11]]) as usize;
    let cd_offset = u32::from_le_bytes([
        archive[eocd + 16],
        archive[eocd + 17],
        archive[eocd + 18],
        archive[eocd + 19],
    ]) as usize;

    let mut out = BTreeMap::new();
    let mut p = cd_offset;
    for _ in 0..count {
        assert_eq!(
            &archive[p..p + 4],
            &0x0201_4b50u32.to_le_bytes(),
            "bad central dir sig"
        );
        let method = u16::from_le_bytes([archive[p + 10], archive[p + 11]]);
        assert_eq!(method, 0, "entry is not STORED");
        let crc = u32::from_le_bytes([
            archive[p + 16],
            archive[p + 17],
            archive[p + 18],
            archive[p + 19],
        ]);
        let size = u32::from_le_bytes([
            archive[p + 20],
            archive[p + 21],
            archive[p + 22],
            archive[p + 23],
        ]) as usize;
        let name_len = u16::from_le_bytes([archive[p + 28], archive[p + 29]]) as usize;
        let extra_len = u16::from_le_bytes([archive[p + 30], archive[p + 31]]) as usize;
        let comment_len = u16::from_le_bytes([archive[p + 32], archive[p + 33]]) as usize;
        let lho = u32::from_le_bytes([
            archive[p + 42],
            archive[p + 43],
            archive[p + 44],
            archive[p + 45],
        ]) as usize;
        let name = String::from_utf8(archive[p + 46..p + 46 + name_len].to_vec()).unwrap();

        // Follow the local header offset to the data.
        assert_eq!(
            &archive[lho..lho + 4],
            &0x0403_4b50u32.to_le_bytes(),
            "bad local header sig"
        );
        let l_name_len = u16::from_le_bytes([archive[lho + 26], archive[lho + 27]]) as usize;
        let l_extra_len = u16::from_le_bytes([archive[lho + 28], archive[lho + 29]]) as usize;
        let data_start = lho + 30 + l_name_len + l_extra_len;
        let data = archive[data_start..data_start + size].to_vec();
        assert_eq!(crc32(&data), crc, "CRC-32 mismatch for {name}");

        out.insert(name, data);
        p += 46 + name_len + extra_len + comment_len;
    }
    out
}

#[test]
fn docx_unzips_and_every_crc_validates() {
    let bytes = to_docx(FIXTURE, &fixture_images());
    let parts = unzip_stored(&bytes);
    // The required minimal part set is present.
    for required in [
        "[Content_Types].xml",
        "_rels/.rels",
        "word/document.xml",
        "word/styles.xml",
        "word/numbering.xml",
        "word/_rels/document.xml.rels",
    ] {
        assert!(parts.contains_key(required), "missing part {required}");
    }
    // The embedded image landed as a media part with the exact PNG bytes.
    let media = parts
        .get("word/media/image1.png")
        .expect("media/image1.png");
    assert_eq!(media, &fixture_png());
}

#[test]
fn every_docx_xml_part_is_well_formed() {
    let bytes = to_docx(FIXTURE, &fixture_images());
    let parts = unzip_stored(&bytes);
    for (name, data) in &parts {
        if name.ends_with(".xml") || name.ends_with(".rels") {
            let text = std::str::from_utf8(data).unwrap();
            check_xml_well_formed(text).unwrap_or_else(|e| panic!("{name} not well-formed: {e}"));
        }
    }
}

#[test]
fn docx_body_carries_the_expected_ooxml() {
    let bytes = to_docx(FIXTURE, &fixture_images());
    let parts = unzip_stored(&bytes);
    let doc = std::str::from_utf8(&parts["word/document.xml"]).unwrap();
    assert!(doc.contains("<w:pStyle w:val=\"Heading1\"/>"));
    assert!(doc.contains("<w:b/>"));
    assert!(doc.contains("<w:i/>"));
    assert!(doc.contains("<w:strike/>"));
    assert!(doc.contains("<w:highlight w:val=\"yellow\"/>"));
    assert!(doc.contains("<w:hyperlink r:id="));
    assert!(doc.contains("<w:numPr>"));
    assert!(doc.contains("<w:tbl>"));
    assert!(doc.contains("<w:drawing>"));
    assert!(doc.contains("\u{2611}")); // checked task glyph
    // The hyperlink target is a real external relationship.
    let rels = std::str::from_utf8(&parts["word/_rels/document.xml.rels"]).unwrap();
    assert!(rels.contains("TargetMode=\"External\""));
    assert!(rels.contains("example.com/path?q=1&amp;r=2"));
    assert!(rels.contains("Target=\"media/image1.png\""));
    // Content types register the PNG default.
    let ct = std::str::from_utf8(&parts["[Content_Types].xml"]).unwrap();
    assert!(ct.contains("Extension=\"png\""));
    // Numbering restarts each ordered list (numId 2 with a startOverride).
    let numbering = std::str::from_utf8(&parts["word/numbering.xml"]).unwrap();
    assert!(numbering.contains("w:startOverride"));
}

// --- Determinism + goldens --------------------------------------------------

#[test]
fn exports_are_byte_deterministic() {
    let a = to_docx(FIXTURE, &fixture_images());
    let b = to_docx(FIXTURE, &fixture_images());
    assert_eq!(a, b, "docx export is not deterministic");
    let h1 = to_html(FIXTURE, &fixture_images());
    let h2 = to_html(FIXTURE, &fixture_images());
    assert_eq!(h1, h2, "html export is not deterministic");
    // to_bytes agrees with the direct emitters.
    assert_eq!(to_bytes(FIXTURE, Format::Docx, &fixture_images()), a);
    assert_eq!(
        to_bytes(FIXTURE, Format::Html, &fixture_images()),
        h1.into_bytes()
    );
    let p = to_pdf(FIXTURE, &fixture_images());
    assert_eq!(to_bytes(FIXTURE, Format::Pdf, &fixture_images()), p);
    assert_eq!(Format::Pdf.ext(), "pdf");
}

fn testdata_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/export/testdata")
        .join(name)
}

/// Compare `got` against a committed golden file, or (re)write it under
/// `AWL_BLESS=1`. Keeps the golden gate exact-byte without a compile-time
/// `include_bytes!` dependency on a not-yet-generated file.
fn golden(name: &str, got: &[u8]) {
    let path = testdata_path(name);
    if std::env::var("AWL_BLESS").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, got).unwrap();
        return;
    }
    let want = std::fs::read(&path).unwrap_or_else(|_| {
        panic!("golden {name} missing — run `AWL_BLESS=1 cargo test export::` to create it")
    });
    assert!(
        got == want.as_slice(),
        "{name} drifted from its golden ({} vs {} bytes); AWL_BLESS=1 to update after an intentional change",
        got.len(),
        want.len()
    );
}

#[test]
fn docx_golden_is_byte_stable() {
    golden("rich.docx", &to_docx(FIXTURE, &fixture_images()));
}

#[test]
fn html_golden_is_byte_stable() {
    golden("rich.html", to_html(FIXTURE, &fixture_images()).as_bytes());
}

// --- A tiny dev-only XML well-formedness checker ----------------------------
//
// Just enough to catch a malformed OOXML part: balanced tags, quoted
// attributes, no stray `<` in text. Not a validator (no schema, no entity
// table) — a well-formedness smoke that needs no dependency.

fn check_xml_well_formed(s: &str) -> Result<(), String> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut stack: Vec<String> = Vec::new();
    while i < bytes.len() {
        if bytes[i] != b'<' {
            if bytes[i] == b'>' {
                return Err(format!("stray '>' at {i}"));
            }
            i += 1;
            continue;
        }
        // A `<...>` construct: find the closing `>` outside quotes.
        let start = i;
        i += 1;
        // Processing instruction / declaration / comment: skip to matching `>`.
        if bytes.get(i) == Some(&b'?') || bytes.get(i) == Some(&b'!') {
            while i < bytes.len() && bytes[i] != b'>' {
                i += 1;
            }
            if i >= bytes.len() {
                return Err("unterminated <? / <!".into());
            }
            i += 1;
            continue;
        }
        let mut quote: Option<u8> = None;
        let mut end = i;
        while end < bytes.len() {
            let c = bytes[end];
            match quote {
                Some(q) => {
                    if c == q {
                        quote = None;
                    }
                }
                None => {
                    if c == b'"' || c == b'\'' {
                        quote = Some(c);
                    } else if c == b'>' {
                        break;
                    } else if c == b'<' {
                        return Err(format!("nested '<' in tag at {end}"));
                    }
                }
            }
            end += 1;
        }
        if end >= bytes.len() {
            return Err("unterminated tag".into());
        }
        let inner = &s[start + 1..end]; // between < and >
        let self_closing = inner.ends_with('/');
        let inner = inner.trim_end_matches('/').trim();
        if let Some(name) = inner.strip_prefix('/') {
            // Close tag.
            let name = name.trim();
            match stack.pop() {
                Some(open) if open == name => {}
                Some(open) => return Err(format!("mismatched close </{name}> for <{open}>")),
                None => return Err(format!("close </{name}> with empty stack")),
            }
        } else {
            // Open (or self-closing) tag: name then attributes.
            let (name, attrs) = split_name(inner);
            if name.is_empty() {
                return Err(format!("empty tag name at {start}"));
            }
            check_attrs(attrs)?;
            if !self_closing {
                stack.push(name.to_string());
            }
        }
        i = end + 1;
    }
    if let Some(open) = stack.last() {
        return Err(format!("unclosed <{open}>"));
    }
    Ok(())
}

fn split_name(inner: &str) -> (&str, &str) {
    match inner.find(|c: char| c.is_whitespace()) {
        Some(idx) => (&inner[..idx], inner[idx..].trim_start()),
        None => (inner, ""),
    }
}

/// Every attribute must be `name="value"` or `name='value'`.
fn check_attrs(mut attrs: &str) -> Result<(), String> {
    while !attrs.is_empty() {
        attrs = attrs.trim_start();
        if attrs.is_empty() {
            break;
        }
        let eq = attrs
            .find('=')
            .ok_or_else(|| format!("attribute without '=': {attrs:?}"))?;
        let _name = &attrs[..eq];
        let rest = attrs[eq + 1..].trim_start();
        let quote = rest.chars().next().ok_or("attribute value missing")?;
        if quote != '"' && quote != '\'' {
            return Err(format!("unquoted attribute value: {rest:?}"));
        }
        let close = rest[1..]
            .find(quote)
            .ok_or("unterminated attribute value")?;
        attrs = &rest[1 + close + 1..];
    }
    Ok(())
}

// --- Render/export strikethrough agreement (the exactly-two-tilde gate) ------
//
// The BUG this closed: `markdown::spans` gates strikethrough to EXACTLY-two
// tildes (`~x~` inert, `~~x~~` struck), but the export enabled pulldown's GFM
// strikethrough WITHOUT that gate — so `~x~` exported STRUCK while rendering
// inert. Both paths now read the ONE shared owner `markdown::strike_engaged`.
// These laws assert the render's struck-set and the export's struck-set are the
// SAME for the truth table, and drive the real HTML emitter to prove the gate
// reaches actual exported bytes.

/// The struck TEXT tokens as the RENDERER sees them: every `MdKind::Strikethrough`
/// span's source substring, split into whitespace tokens, in document order.
fn render_struck_tokens(md: &str) -> Vec<String> {
    let mut spans = crate::markdown::spans(md);
    spans.sort_by_key(|(r, _)| r.start);
    let mut out = Vec::new();
    for (r, k) in &spans {
        if *k == crate::markdown::MdKind::Strikethrough {
            out.extend(md[r.clone()].split_whitespace().map(str::to_string));
        }
    }
    out
}

/// The struck TEXT tokens as the EXPORT tree sees them: every `Inline::Strikethrough`
/// node's flattened text, split into whitespace tokens, in document order. A struck
/// node's full `plain_text` covers all its (possibly nested) content once, so we do
/// not recurse into it — mirroring the renderer, where every byte under an engaged
/// strike is struck.
fn export_struck_tokens(md: &str) -> Vec<String> {
    fn walk_inlines(inlines: &[Inline], out: &mut Vec<String>) {
        for i in inlines {
            match i {
                Inline::Strikethrough(c) => {
                    out.extend(model::plain_text(c).split_whitespace().map(str::to_string));
                }
                Inline::Strong(c)
                | Inline::Emphasis(c)
                | Inline::Highlight(c)
                | Inline::Link { children: c, .. } => walk_inlines(c, out),
                _ => {}
            }
        }
    }
    fn walk_blocks(blocks: &[Block], out: &mut Vec<String>) {
        for b in blocks {
            match b {
                Block::Heading { inlines, .. } | Block::Paragraph(inlines) => {
                    walk_inlines(inlines, out)
                }
                Block::BlockQuote(bs) => walk_blocks(bs, out),
                Block::List(l) => {
                    for it in &l.items {
                        walk_blocks(&it.blocks, out)
                    }
                }
                Block::Table(t) => {
                    for cell in &t.head {
                        walk_inlines(cell, out)
                    }
                    for row in &t.rows {
                        for cell in row {
                            walk_inlines(cell, out)
                        }
                    }
                }
                Block::CodeBlock { .. } | Block::Rule => {}
            }
        }
    }
    let doc = model::parse(md);
    let mut out = Vec::new();
    walk_blocks(&doc.blocks, &mut out);
    out
}

#[test]
fn render_export_strikethrough_agree() {
    // The truth table: single-tilde inert, two-tilde struck, tilde-fence, a
    // nested single-inside-double, an engaged span across a soft break, a prose
    // false-positive, and a plain mid-sentence pair.
    let cases: &[(&str, &[&str])] = &[
        ("~x~", &[]),                        // single tilde: inert, nothing struck
        ("~~x~~", &["x"]),                   // two tildes: struck
        ("~~~\nbody\n~~~\n", &[]),           // `~~~` is a FENCE, never a strike
        ("~~a ~b~ c~~", &["a", "b", "c"]),   // engaged outer; inert inner `~` dropped
        ("~~cut\nline~~", &["cut", "line"]), // engaged across a soft break
        ("2~3 weeks and 4~5 days", &[]),     // bare single `~` in prose: never struck
        ("keep ~~cut this~~ keep", &["cut", "this"]),
    ];
    for (md, expected) in cases {
        let r = render_struck_tokens(md);
        let e = export_struck_tokens(md);
        assert_eq!(
            r, e,
            "render vs export struck-set diverge on {md:?}: render={r:?} export={e:?}"
        );
        assert_eq!(
            r,
            expected.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            "struck-set unexpected for {md:?}: got {r:?} want {expected:?}"
        );
    }
}

#[test]
fn export_html_strikethrough_gate() {
    // Drive the REAL HTML emitter end-to-end: the engaged `~~x~~` yields a
    // `<del>`; the inert single-tilde `~x~` never does — yet still exports its
    // content (the bug was `~x~` exporting STRUCK).
    let struck = to_html("~~x~~", &NoImages);
    assert!(
        struck.contains("<del>x</del>"),
        "engaged ~~x~~ must render <del>: {struck}"
    );

    let inert = to_html("~x~", &NoImages);
    assert!(!inert.contains("<del>"), "inert ~x~ must NOT render <del>: {inert}");
    assert!(inert.contains("<p>x</p>"), "inert ~x~ content still exported: {inert}");

    // The nested pathological case: ONE `<del>` wrapping the whole content, and
    // the inert inner `~` delimiters dropped (never emitted as literal tildes).
    let nested = to_html("~~a ~b~ c~~", &NoImages);
    assert_eq!(
        nested.matches("<del>").count(),
        1,
        "nested case is one strike wrapper: {nested}"
    );
    assert!(!nested.contains('~'), "inert inner ~ delimiters dropped: {nested}");
}

#[test]
fn export_docx_strikethrough_gate() {
    // The DOCX path shares the same model tree, so the gate reaches it too: the
    // engaged pair emits `<w:strike/>`; the inert single tilde emits none.
    let doc_xml = |md: &str| {
        String::from_utf8(unzip_stored(&to_docx(md, &NoImages))["word/document.xml"].clone())
            .unwrap()
    };
    assert!(
        doc_xml("~~x~~").contains("<w:strike/>"),
        "engaged ~~x~~ emits <w:strike/>"
    );
    assert!(
        !doc_xml("~x~").contains("<w:strike/>"),
        "inert ~x~ must emit no <w:strike/>"
    );
}

// --- Render/export highlight agreement (the isolated-exactly-two-`=` gate) ----
//
// The DEBT this closed: the `==highlight==` delimiter gate (`equals_runs`, the
// isolated-exactly-two-`=` rule) was duplicated VERBATIM in `markdown::spans` and
// `export::model`. They agreed byte-for-byte, but it was the exact two-owner
// shape that produced the `~x~` strike divergence — one edit away from
// disagreeing. Both paths now read the ONE shared owner `markdown::equals_runs`.
// These laws assert render's highlighted-set == export's highlighted-set for the
// truth table, and drive the real HTML/DOCX emitters to prove the gate reaches
// actual exported bytes (`<mark>` / `<w:highlight/>`).

/// The highlighted TEXT tokens as the RENDERER sees them: every `MdKind::Highlight`
/// span's source substring, split into whitespace tokens, in document order.
fn render_highlighted_tokens(md: &str) -> Vec<String> {
    let mut spans = crate::markdown::spans(md);
    spans.sort_by_key(|(r, _)| r.start);
    let mut out = Vec::new();
    for (r, k) in &spans {
        if *k == crate::markdown::MdKind::Highlight {
            out.extend(md[r.clone()].split_whitespace().map(str::to_string));
        }
    }
    out
}

/// The highlighted TEXT tokens as the EXPORT tree sees them: every
/// `Inline::Highlight` node's flattened text, split into whitespace tokens, in
/// document order. A highlighted node's full `plain_text` covers all its content
/// once, so we do not recurse into it — mirroring the renderer, where every byte
/// under an engaged `==…==` pair is highlighted.
fn export_highlighted_tokens(md: &str) -> Vec<String> {
    fn walk_inlines(inlines: &[Inline], out: &mut Vec<String>) {
        for i in inlines {
            match i {
                Inline::Highlight(c) => {
                    out.extend(model::plain_text(c).split_whitespace().map(str::to_string));
                }
                Inline::Strong(c)
                | Inline::Emphasis(c)
                | Inline::Strikethrough(c)
                | Inline::Link { children: c, .. } => walk_inlines(c, out),
                _ => {}
            }
        }
    }
    fn walk_blocks(blocks: &[Block], out: &mut Vec<String>) {
        for b in blocks {
            match b {
                Block::Heading { inlines, .. } | Block::Paragraph(inlines) => {
                    walk_inlines(inlines, out)
                }
                Block::BlockQuote(bs) => walk_blocks(bs, out),
                Block::List(l) => {
                    for it in &l.items {
                        walk_blocks(&it.blocks, out)
                    }
                }
                Block::Table(t) => {
                    for cell in &t.head {
                        walk_inlines(cell, out)
                    }
                    for row in &t.rows {
                        for cell in row {
                            walk_inlines(cell, out)
                        }
                    }
                }
                Block::CodeBlock { .. } | Block::Rule => {}
            }
        }
    }
    let doc = model::parse(md);
    let mut out = Vec::new();
    walk_blocks(&doc.blocks, &mut out);
    out
}

#[test]
fn render_export_highlight_agree() {
    // The truth table: single `=` inert, two-`=` engaged, a bare three-`=` run and
    // a four-`=` run (never a candidate), an outer engaged pair with an inert
    // single-`=` run literal inside it, a `==` pair split by a soft break (no
    // cross-line span in either path), a prose false-positive (`2=3 and 4=5`), and
    // a plain mid-sentence pair.
    let cases: &[(&str, &[&str])] = &[
        ("=x=", &[]),                             // single `=`: inert, nothing marked
        ("==x==", &["x"]),                        // two `=`: highlighted
        ("===", &[]),                             // three `=`: no candidate anywhere
        ("a ==== b", &[]),                        // four `=`: inert run, no candidate
        ("==a =b= c==", &["a", "=b=", "c"]),      // engaged outer; inert `=b=` literal inside
        ("==cut\nline==", &[]),                   // soft break splits: neither run pairs
        ("2=3 and 4=5", &[]),                     // bare single `=` in prose: never marked
        ("keep ==mark this== keep", &["mark", "this"]),
    ];
    for (md, expected) in cases {
        let r = render_highlighted_tokens(md);
        let e = export_highlighted_tokens(md);
        assert_eq!(
            r, e,
            "render vs export highlight-set diverge on {md:?}: render={r:?} export={e:?}"
        );
        assert_eq!(
            r,
            expected.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            "highlight-set unexpected for {md:?}: got {r:?} want {expected:?}"
        );
    }
}

#[test]
fn export_html_highlight_gate() {
    // Drive the REAL HTML emitter end-to-end: the engaged `==x==` yields a
    // `<mark>`; the inert single-`=` `=x=` never does — yet still exports its
    // content (the legitimate wrapper still emits; the inert case stays plain).
    let marked = to_html("==x==", &NoImages);
    assert!(
        marked.contains("<mark>x</mark>"),
        "engaged ==x== must render <mark>: {marked}"
    );

    let inert = to_html("=x=", &NoImages);
    assert!(!inert.contains("<mark>"), "inert =x= must NOT render <mark>: {inert}");
    assert!(inert.contains("=x="), "inert =x= content still exported literally: {inert}");

    // The pathological case: ONE `<mark>` wrapping the whole content, the inner
    // single-`=` run kept as literal `=` text (never a second highlight).
    let nested = to_html("==a =b= c==", &NoImages);
    assert_eq!(
        nested.matches("<mark>").count(),
        1,
        "one highlight wrapper for the outer pair: {nested}"
    );
    assert!(nested.contains("=b="), "inert inner =b= kept literal: {nested}");
}

#[test]
fn export_docx_highlight_gate() {
    // The DOCX path shares the same model tree, so the gate reaches it too: the
    // engaged pair emits `<w:highlight/>`; the inert single `=` emits none.
    let doc_xml = |md: &str| {
        String::from_utf8(unzip_stored(&to_docx(md, &NoImages))["word/document.xml"].clone())
            .unwrap()
    };
    assert!(
        doc_xml("==x==").contains("<w:highlight"),
        "engaged ==x== emits <w:highlight/>"
    );
    assert!(
        !doc_xml("=x=").contains("<w:highlight"),
        "inert =x= must emit no <w:highlight/>"
    );
}
