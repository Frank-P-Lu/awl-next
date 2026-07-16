//! The HTML emitter: a [`Document`] → one STANDALONE, self-contained HTML file
//! carrying a PRINT-TUNED stylesheet (`@page` margins, screen + print rules,
//! clean page-break behavior). This is awl's documented "PDF path": open the
//! exported `.html` in any browser and Print → Save as PDF gives a tidy,
//! paginated document with no app, no server, no dependency.
//!
//! SELF-CONTAINED: images are embedded as `data:` URIs (base64), so the single
//! file travels whole. The markup is semantic + minimal (`<h1>`, `<blockquote>`,
//! `<pre><code>`, `<mark>`, real `<a href>`, `<ul>/<ol>`, `<table>`), and the
//! style block echoes awl's own restraint — a tasteful serif body, a heading
//! size ladder matching the editor's, quiet code shading.
//!
//! DETERMINISTIC: a pure function of the [`Document`] + resolved image bytes.

use super::model::{Align, Block, Document, ExportImage, ImageSource, Inline, List, Table};

/// Render `doc` to a standalone HTML document string, resolving images through
/// `images` (unresolvable ones fall back to their alt text).
pub fn emit(doc: &Document, images: &dyn ImageSource) -> String {
    let mut body = String::new();
    for b in &doc.blocks {
        emit_block(&mut body, b, images, 0);
    }
    let title = doc.title.as_deref().unwrap_or("awl document");
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{}</title>\n<style>\n{}</style>\n</head>\n<body>\n{}</body>\n</html>\n",
        escape(title),
        STYLESHEET,
        body
    )
}

/// The embedded, print-tuned stylesheet. Screen rules keep a comfortable reading
/// measure; the `@media print` + `@page` rules set page margins and prevent ugly
/// breaks (a heading orphaned from its section, a code block or table split mid
/// row). Neutral serif house style, size ladder echoing awl's editor.
const STYLESHEET: &str = "\
:root { --ink: #1a1a1a; --muted: #6b6b6b; --rule: #d8d8d8; --code-bg: #f4f4f2; --mark: #fdf6b2; --link: #2244aa; }
* { box-sizing: border-box; }
body { color: var(--ink); background: #fff; font-family: Georgia, 'Iowan Old Style', 'Times New Roman', serif; font-size: 12pt; line-height: 1.55; max-width: 42rem; margin: 2rem auto; padding: 0 1.25rem; }
h1, h2, h3, h4, h5, h6 { line-height: 1.2; margin: 1.6em 0 0.5em; font-weight: 600; }
h1 { font-size: 2em; } h2 { font-size: 1.6em; } h3 { font-size: 1.35em; }
h4 { font-size: 1.15em; } h5 { font-size: 1em; } h6 { font-size: 0.9em; color: var(--muted); }
p { margin: 0 0 0.9em; }
a { color: var(--link); text-decoration: underline; }
strong { font-weight: 700; } em { font-style: italic; } del { color: var(--muted); }
mark { background: var(--mark); color: inherit; padding: 0 0.1em; }
code { font-family: 'SF Mono', Menlo, Consolas, 'DejaVu Sans Mono', monospace; font-size: 0.9em; background: var(--code-bg); padding: 0.1em 0.3em; border-radius: 3px; }
pre { background: var(--code-bg); padding: 0.9em 1.1em; border-radius: 5px; overflow-x: auto; line-height: 1.4; }
pre code { background: none; padding: 0; font-size: 0.85em; }
blockquote { margin: 1em 0; padding: 0.2em 1.1em; border-left: 3px solid var(--rule); color: var(--muted); font-style: italic; }
hr { border: none; border-top: 1px solid var(--rule); margin: 2em 0; }
ul, ol { margin: 0 0 0.9em; padding-left: 1.6em; }
li { margin: 0.2em 0; }
li.task { list-style: none; margin-left: -1.3em; }
li.task input { margin-right: 0.4em; }
table { border-collapse: collapse; margin: 1em 0; width: 100%; }
th, td { border: 1px solid var(--rule); padding: 0.4em 0.7em; text-align: left; }
th { background: var(--code-bg); font-weight: 600; }
img { max-width: 100%; height: auto; }
@media print {
  @page { margin: 2cm; }
  body { max-width: none; margin: 0; font-size: 11pt; }
  a { color: var(--ink); }
  h1, h2, h3, h4, h5, h6 { break-after: avoid; page-break-after: avoid; }
  pre, blockquote, table, img, tr { break-inside: avoid; page-break-inside: avoid; }
}
";

fn emit_block(out: &mut String, block: &Block, images: &dyn ImageSource, indent: usize) {
    let pad = "  ".repeat(indent);
    match block {
        Block::Heading { level, inlines } => {
            let l = (*level).clamp(1, 6);
            out.push_str(&format!("{pad}<h{l}>"));
            emit_inlines(out, inlines, images);
            out.push_str(&format!("</h{l}>\n"));
        }
        Block::Paragraph(inlines) => {
            out.push_str(&format!("{pad}<p>"));
            emit_inlines(out, inlines, images);
            out.push_str("</p>\n");
        }
        Block::BlockQuote(blocks) => {
            out.push_str(&format!("{pad}<blockquote>\n"));
            for b in blocks {
                emit_block(out, b, images, indent + 1);
            }
            out.push_str(&format!("{pad}</blockquote>\n"));
        }
        Block::CodeBlock { lang, code } => {
            match lang {
                Some(l) => out.push_str(&format!(
                    "{pad}<pre><code class=\"language-{}\">",
                    escape(l)
                )),
                None => out.push_str(&format!("{pad}<pre><code>")),
            }
            out.push_str(&escape(code));
            out.push_str("</code></pre>\n");
        }
        Block::List(list) => emit_list(out, list, images, indent),
        Block::Rule => out.push_str(&format!("{pad}<hr>\n")),
        Block::Table(table) => emit_table(out, table, images, indent),
    }
}

fn emit_list(out: &mut String, list: &List, images: &dyn ImageSource, indent: usize) {
    let pad = "  ".repeat(indent);
    let (open, close) = if list.ordered {
        if list.start != 1 {
            (format!("<ol start=\"{}\">", list.start), "</ol>")
        } else {
            ("<ol>".to_string(), "</ol>")
        }
    } else {
        ("<ul>".to_string(), "</ul>")
    };
    out.push_str(&format!("{pad}{open}\n"));
    for item in &list.items {
        let ipad = "  ".repeat(indent + 1);
        match item.task {
            Some(checked) => {
                let boxed = if checked { " checked" } else { "" };
                out.push_str(&format!(
                    "{ipad}<li class=\"task\"><input type=\"checkbox\" disabled{boxed}> "
                ));
            }
            None => out.push_str(&format!("{ipad}<li>")),
        }
        // A tight single-paragraph item renders inline; anything richer nests.
        if item.blocks.len() == 1 {
            if let Block::Paragraph(inlines) = &item.blocks[0] {
                emit_inlines(out, inlines, images);
                out.push_str("</li>\n");
                continue;
            }
        }
        out.push('\n');
        for b in &item.blocks {
            emit_block(out, b, images, indent + 2);
        }
        out.push_str(&format!("{ipad}</li>\n"));
    }
    out.push_str(&format!("{pad}{close}\n"));
}

fn emit_table(out: &mut String, table: &Table, images: &dyn ImageSource, indent: usize) {
    let pad = "  ".repeat(indent);
    out.push_str(&format!("{pad}<table>\n"));
    let align_attr = |i: usize| -> &'static str {
        match table.aligns.get(i).copied().unwrap_or(Align::None) {
            Align::Left => " style=\"text-align:left\"",
            Align::Center => " style=\"text-align:center\"",
            Align::Right => " style=\"text-align:right\"",
            Align::None => "",
        }
    };
    if !table.head.is_empty() {
        out.push_str(&format!("{pad}  <thead>\n{pad}    <tr>\n"));
        for (i, cell) in table.head.iter().enumerate() {
            out.push_str(&format!("{pad}      <th{}>", align_attr(i)));
            emit_inlines(out, cell, images);
            out.push_str("</th>\n");
        }
        out.push_str(&format!("{pad}    </tr>\n{pad}  </thead>\n"));
    }
    out.push_str(&format!("{pad}  <tbody>\n"));
    for row in &table.rows {
        out.push_str(&format!("{pad}    <tr>\n"));
        for (i, cell) in row.iter().enumerate() {
            out.push_str(&format!("{pad}      <td{}>", align_attr(i)));
            emit_inlines(out, cell, images);
            out.push_str("</td>\n");
        }
        out.push_str(&format!("{pad}    </tr>\n"));
    }
    out.push_str(&format!("{pad}  </tbody>\n{pad}</table>\n"));
}

fn emit_inlines(out: &mut String, inlines: &[Inline], images: &dyn ImageSource) {
    for i in inlines {
        emit_inline(out, i, images);
    }
}

fn emit_inline(out: &mut String, inline: &Inline, images: &dyn ImageSource) {
    match inline {
        Inline::Text(t) => out.push_str(&escape(t)),
        Inline::Code(c) => {
            out.push_str("<code>");
            out.push_str(&escape(c));
            out.push_str("</code>");
        }
        Inline::Strong(c) => wrap(out, "strong", c, images),
        Inline::Emphasis(c) => wrap(out, "em", c, images),
        Inline::Strikethrough(c) => wrap(out, "del", c, images),
        Inline::Highlight(c) => wrap(out, "mark", c, images),
        Inline::Link { url, children } => {
            out.push_str(&format!("<a href=\"{}\">", escape_attr(url)));
            emit_inlines(out, children, images);
            out.push_str("</a>");
        }
        Inline::Image {
            src,
            alt,
            width_hint,
        } => emit_image(out, src, alt, *width_hint, images),
        Inline::SoftBreak => out.push('\n'),
        Inline::HardBreak => out.push_str("<br>\n"),
    }
}

fn wrap(out: &mut String, tag: &str, children: &[Inline], images: &dyn ImageSource) {
    out.push_str(&format!("<{tag}>"));
    emit_inlines(out, children, images);
    out.push_str(&format!("</{tag}>"));
}

fn emit_image(
    out: &mut String,
    src: &str,
    alt: &str,
    width_hint: Option<u32>,
    images: &dyn ImageSource,
) {
    match images.resolve(src) {
        Some(ExportImage {
            bytes, width, mime, ..
        }) => {
            let data = base64(&bytes);
            let w = width_hint.unwrap_or(width);
            out.push_str(&format!(
                "<img src=\"data:{};base64,{}\" alt=\"{}\" width=\"{}\">",
                mime.mime_str(),
                data,
                escape_attr(alt),
                w
            ));
        }
        // Unresolvable (remote / missing / wasm): the alt text stands in, so the
        // document never shows a broken-image glyph.
        None => {
            if !alt.is_empty() {
                out.push_str(&escape(alt));
            }
        }
    }
}

/// Escape text for HTML element content.
fn escape(s: &str) -> String {
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

/// Escape a value for a double-quoted HTML attribute.
fn escape_attr(s: &str) -> String {
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

/// Standard base64 (RFC 4648) — for the image `data:` URIs. Hand-rolled (no
/// dependency), padded, deterministic.
fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Re-exported so `mod.rs`'s tests can exercise the base64 encoder directly.
/// Gated with the native-only export test module (its sole consumer).
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(super) fn base64_for_test(data: &[u8]) -> String {
    base64(data)
}
