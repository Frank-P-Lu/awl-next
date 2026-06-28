//! Markdown styling spans — the "dim the markup + style the content" model for
//! awl's prose docs. We parse the document with `pulldown-cmark`'s OFFSET
//! iterator (each event carries its byte range into the source text) and turn
//! the events into a flat list of `(byte-range, MdKind)` spans. The renderer
//! lays these as per-span `Attrs` over each line's `AttrsList`, exactly like the
//! CJK + focus spans — the markup characters (`#`, `*`, `` ` ``, `>`, list
//! markers, link brackets/URL) recede to the DIM ink while staying fully present
//! and editable, and the CONTENT gains structure (bold weight, italic style,
//! mono code, heading weight+SIZE, accent link text). Headings take NO accent
//! color — figure/ground by value + size + weight, so the amber stays the caret's
//! alone (DESIGN.md §3, the one-organic-element law).
//!
//! This is PURE: the spans are a deterministic function of the text (no clock,
//! no layout), so a headless capture renders the settled styled state and the
//! sidecar can report the spans verbatim.
//!
//! HEADING SIZE: a heading's level now also drives a per-line font/line-height
//! SCALE (see [`heading_scale`]). The renderer reads it in `render.rs` to lay the
//! whole heading line at a larger `Attrs::metrics`, so headings render physically
//! BIGGER (not just bolder). This relies on render.rs's VARIABLE-row-height layout
//! pass (a per-row geometry table feeding scroll / hit-test / caret), so the kind
//! enum still carries only the LEVEL — the concrete pixel ramp lives in one place
//! ([`heading_scale`]) and every non-heading span kind stays line-height-neutral
//! (scale 1.0), keeping a plain prose / code buffer byte-identical.

use std::ops::Range;

/// One styled span kind. Maps (in `render.rs`) to a concrete `Attrs` transform
/// over the base document attrs. `Markup` is the recede-to-dim role shared by
/// every syntax character; the rest style content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MdKind {
    /// Syntax characters that recede to the DIM ink (`#`, `*`/`_`, backticks,
    /// `>`, fences, link brackets + URL). Still present + editable, just quiet.
    Markup,
    /// A heading's CONTENT text. Level 1..=6 → heavier weight + heading color +
    /// a larger font SIZE per [`heading_scale`] (applied per-line in `render.rs`).
    Heading(u8),
    /// `**bold**` / `__bold__` content → Bold weight.
    Bold,
    /// `*italic*` / `_italic_` content → Italic style.
    Italic,
    /// `***both***` content → Bold + Italic.
    BoldItalic,
    /// Inline `` `code` `` + fenced/indented code-block body → mono family + tint.
    Code,
    /// Blockquote TEXT → dim (the `>` marker is `Markup`).
    Quote,
    /// A list item's leading marker (`-`/`*`/`+`/`1.`) → dim.
    ListMarker,
    /// A link's visible TEXT → accent color (the brackets + URL are `Markup`).
    LinkText,
}

/// The font / line-height SCALE for a heading, by the COUNT of leading `#` marks
/// (1, 2, 3+). Only THREE distinct sizes: past `###` nobody wants a finer ramp, so
/// 4+ hashes share the `h3` size. `0` (no hash) is body size. This is the SINGLE
/// source of truth for heading size: `render.rs` reads it from a line's leading-`#`
/// run (NOT from a fully-valid ATX heading — so a line grows the moment you type
/// `#`, before the space + title), lays the line's `Attrs::metrics` at `base *
/// scale`, and cosmic-text takes the row height from the max of its glyphs' line
/// heights, so the whole heading row grows by exactly this factor. Tune the *feel*
/// here, in one place.
pub fn heading_scale(level: u8) -> f32 {
    match level {
        0 => 1.0,
        1 => 1.8,
        2 => 1.5,
        _ => 1.3,
    }
}

impl MdKind {
    /// Stable tag string for the capture sidecar's `md_spans` block.
    pub fn tag(self) -> &'static str {
        match self {
            MdKind::Markup => "markup",
            MdKind::Heading(1) => "h1",
            MdKind::Heading(2) => "h2",
            MdKind::Heading(3) => "h3",
            MdKind::Heading(4) => "h4",
            MdKind::Heading(5) => "h5",
            MdKind::Heading(_) => "h6",
            MdKind::Bold => "bold",
            MdKind::Italic => "italic",
            MdKind::BoldItalic => "bold_italic",
            MdKind::Code => "code",
            MdKind::Quote => "quote",
            MdKind::ListMarker => "list_marker",
            MdKind::LinkText => "link_text",
        }
    }
}

/// Parse `text` into styling spans in DOCUMENT byte coordinates. Spans may
/// overlap by DESIGN: a link or code-block first pushes a whole-range `Markup`
/// span, then its inner text pushes a `LinkText`/`Code` span — applied in this
/// order, the later (inner) span wins for its bytes while the brackets/URL/fence
/// keep the dim `Markup`. The renderer adds them to the `AttrsList` in THIS
/// order, relying on cosmic-text's "last span wins on overlap" semantics.
pub fn spans(text: &str) -> Vec<(Range<usize>, MdKind)> {
    use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

    let mut out: Vec<(Range<usize>, MdKind)> = Vec::new();
    // Nesting depth / context flags. Headings don't nest, so a single level is
    // enough; the emphasis/quote/link/code contexts use counters so a nested
    // construct restores the outer context on close.
    let mut heading: Option<u8> = None;
    let mut strong = 0u32;
    let mut emph = 0u32;
    let mut quote = 0u32;
    let mut link = 0u32;
    let mut code_block = 0u32;

    let level_u8 = |l: HeadingLevel| -> u8 {
        match l {
            HeadingLevel::H1 => 1,
            HeadingLevel::H2 => 2,
            HeadingLevel::H3 => 3,
            HeadingLevel::H4 => 4,
            HeadingLevel::H5 => 5,
            HeadingLevel::H6 => 6,
        }
    };

    for (ev, range) in Parser::new_ext(text, Options::empty()).into_offset_iter() {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    heading = Some(level_u8(level));
                    push_heading_markers(&mut out, text, &range);
                }
                Tag::Strong => {
                    strong += 1;
                    push_delim(&mut out, &range, 2);
                }
                Tag::Emphasis => {
                    emph += 1;
                    push_delim(&mut out, &range, 1);
                }
                Tag::BlockQuote(_) => {
                    quote += 1;
                    push_quote_markers(&mut out, text, &range);
                }
                Tag::CodeBlock(_) => {
                    code_block += 1;
                    // Dim the WHOLE block (fences + info string); the body Text
                    // events below override their bytes to mono `Code`. An
                    // indented block has no fence, so this just becomes the body.
                    out.push((range.clone(), MdKind::Markup));
                }
                Tag::Link { .. } => {
                    link += 1;
                    // Dim the whole `[text](url)`; inner Text overrides the visible
                    // text to the accent, leaving brackets + URL dim.
                    out.push((range.clone(), MdKind::Markup));
                }
                Tag::Item => push_list_marker(&mut out, text, &range),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) => heading = None,
                TagEnd::Strong => strong = strong.saturating_sub(1),
                TagEnd::Emphasis => emph = emph.saturating_sub(1),
                TagEnd::BlockQuote(_) => quote = quote.saturating_sub(1),
                TagEnd::CodeBlock => code_block = code_block.saturating_sub(1),
                TagEnd::Link => link = link.saturating_sub(1),
                _ => {}
            },
            Event::Text(_) => {
                if let Some(k) = inline_kind(heading, strong, emph, quote, link, code_block) {
                    out.push((range, k));
                }
            }
            Event::Code(_) => push_inline_code(&mut out, text, &range),
            _ => {}
        }
    }
    out
}

/// Pick the content style for a Text event from the active context, in priority
/// order: a code block wins (mono), then a heading (it owns its whole line), then
/// a link's visible text (accent), then a blockquote (dim), then emphasis. Plain
/// body text returns `None` (it rides the default ink — no span needed).
fn inline_kind(
    heading: Option<u8>,
    strong: u32,
    emph: u32,
    quote: u32,
    link: u32,
    code_block: u32,
) -> Option<MdKind> {
    if code_block > 0 {
        Some(MdKind::Code)
    } else if let Some(l) = heading {
        Some(MdKind::Heading(l))
    } else if link > 0 {
        Some(MdKind::LinkText)
    } else if quote > 0 {
        Some(MdKind::Quote)
    } else if strong > 0 && emph > 0 {
        Some(MdKind::BoldItalic)
    } else if strong > 0 {
        Some(MdKind::Bold)
    } else if emph > 0 {
        Some(MdKind::Italic)
    } else {
        None
    }
}

/// Dim the `n`-byte emphasis delimiters at each end of `range` (`*`/`_` → n=1,
/// `**`/`__` → n=2). No-op if the range is too short to hold both.
fn push_delim(out: &mut Vec<(Range<usize>, MdKind)>, range: &Range<usize>, n: usize) {
    if range.end.saturating_sub(range.start) >= 2 * n {
        out.push((range.start..range.start + n, MdKind::Markup));
        out.push((range.end - n..range.end, MdKind::Markup));
    }
}

/// Dim a heading's leading `#`s (+ the space after), and any ATX closing `#`s.
fn push_heading_markers(out: &mut Vec<(Range<usize>, MdKind)>, text: &str, range: &Range<usize>) {
    let s = &text[range.clone()];
    let b = s.as_bytes();
    // Leading: optional indent whitespace, the `#` run, then the spaces after.
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    let mut h = i;
    while h < b.len() && b[h] == b'#' {
        h += 1;
    }
    if h > i {
        // Include trailing spaces between the hashes and the title.
        let mut j = h;
        while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
            j += 1;
        }
        out.push((range.start..range.start + j, MdKind::Markup));
    }
    // Trailing ATX close: spaces then a `#` run at the very end of the line.
    let mut e = b.len();
    while e > 0 && (b[e - 1] == b' ' || b[e - 1] == b'\t' || b[e - 1] == b'\n') {
        e -= 1;
    }
    let mut c = e;
    while c > 0 && b[c - 1] == b'#' {
        c -= 1;
    }
    if c < e {
        // Pull in a space before the closing hashes if present.
        let mut s0 = c;
        while s0 > 0 && (b[s0 - 1] == b' ' || b[s0 - 1] == b'\t') {
            s0 -= 1;
        }
        out.push((range.start + s0..range.start + e, MdKind::Markup));
    }
}

/// Dim the leading `>` quote markers (+ a following space) on every line of a
/// blockquote range, including nested `>>`.
fn push_quote_markers(out: &mut Vec<(Range<usize>, MdKind)>, text: &str, range: &Range<usize>) {
    let s = &text[range.clone()];
    let b = s.as_bytes();
    let mut line_start = 0usize;
    let mut i = 0usize;
    while i <= b.len() {
        if i == b.len() || b[i] == b'\n' {
            // Scan this line's leading `[ \t]*(> ?)+` marker run.
            let mut k = line_start;
            while k < i && (b[k] == b' ' || b[k] == b'\t') {
                k += 1;
            }
            let mut last = k;
            while k < i && b[k] == b'>' {
                k += 1;
                if k < i && (b[k] == b' ' || b[k] == b'\t') {
                    k += 1;
                }
                last = k;
            }
            if last > line_start {
                out.push((range.start + line_start..range.start + last, MdKind::Markup));
            }
            line_start = i + 1;
        }
        i += 1;
    }
}

/// Dim a list item's leading marker (`-`/`*`/`+` or `1.`/`1)`), plus its space.
fn push_list_marker(out: &mut Vec<(Range<usize>, MdKind)>, text: &str, range: &Range<usize>) {
    let s = &text[range.clone()];
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    let start = i;
    if i < b.len() && (b[i] == b'-' || b[i] == b'*' || b[i] == b'+') {
        i += 1;
    } else {
        let d0 = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i > d0 && i < b.len() && (b[i] == b'.' || b[i] == b')') {
            i += 1;
        } else {
            return; // not a recognizable marker
        }
    }
    // Include the single space after the marker.
    if i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    if i > start {
        out.push((range.start..range.start + i, MdKind::ListMarker));
    }
}

/// Inline `` `code` ``: dim the matching backtick runs at each end, mono-tint the
/// inner slice.
fn push_inline_code(out: &mut Vec<(Range<usize>, MdKind)>, text: &str, range: &Range<usize>) {
    let s = &text[range.clone()];
    let b = s.as_bytes();
    let open = b.iter().take_while(|&&c| c == b'`').count();
    let close = b.iter().rev().take_while(|&&c| c == b'`').count();
    if open == 0 || open + close > b.len() {
        // Degenerate (shouldn't happen for a Code event) — tint the whole thing.
        out.push((range.clone(), MdKind::Code));
        return;
    }
    out.push((range.start..range.start + open, MdKind::Markup));
    out.push((range.end - close..range.end, MdKind::Markup));
    if range.start + open < range.end - close {
        out.push((range.start + open..range.end - close, MdKind::Code));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(spans: &[(Range<usize>, MdKind)], lo: usize, hi: usize, k: MdKind) -> bool {
        spans.iter().any(|(r, kk)| r.start == lo && r.end == hi && *kk == k)
    }

    #[test]
    fn heading_dims_hashes_and_styles_title() {
        let s = spans("# Title");
        // "# " (hash + space) is dim markup; "Title" is H1 content.
        assert!(has(&s, 0, 2, MdKind::Markup), "leading '# ' should be markup: {s:?}");
        assert!(has(&s, 2, 7, MdKind::Heading(1)), "title should be h1: {s:?}");
    }

    #[test]
    fn h2_level_detected() {
        let s = spans("## Sub");
        assert!(has(&s, 0, 3, MdKind::Markup));
        assert!(has(&s, 3, 6, MdKind::Heading(2)));
    }

    #[test]
    fn bold_run_has_dim_stars_and_bold_inner() {
        let s = spans("**bold**");
        assert!(has(&s, 0, 2, MdKind::Markup), "opening ** dim: {s:?}");
        assert!(has(&s, 6, 8, MdKind::Markup), "closing ** dim: {s:?}");
        assert!(has(&s, 2, 6, MdKind::Bold), "inner bold: {s:?}");
    }

    #[test]
    fn italic_underscore() {
        let s = spans("_it_");
        assert!(has(&s, 0, 1, MdKind::Markup));
        assert!(has(&s, 3, 4, MdKind::Markup));
        assert!(has(&s, 1, 3, MdKind::Italic));
    }

    #[test]
    fn inline_code_dims_backticks() {
        let s = spans("`code`");
        assert!(has(&s, 0, 1, MdKind::Markup));
        assert!(has(&s, 5, 6, MdKind::Markup));
        assert!(has(&s, 1, 5, MdKind::Code));
    }

    #[test]
    fn link_text_accent_brackets_dim() {
        let s = spans("[awl](http://x)");
        // whole link dimmed first ...
        assert!(s.iter().any(|(r, k)| r.start == 0 && *k == MdKind::Markup));
        // ... then the visible text [1,4) overrides to LinkText.
        assert!(has(&s, 1, 4, MdKind::LinkText), "link text accent: {s:?}");
    }

    #[test]
    fn blockquote_marker_dim_text_quote() {
        let s = spans("> quoted");
        assert!(has(&s, 0, 2, MdKind::Markup), "'> ' marker dim: {s:?}");
        assert!(s.iter().any(|(_, k)| *k == MdKind::Quote), "quote text: {s:?}");
    }

    #[test]
    fn list_marker_dim() {
        let s = spans("- item");
        assert!(has(&s, 0, 2, MdKind::ListMarker), "marker dim: {s:?}");
    }

    #[test]
    fn plain_prose_has_no_spans() {
        assert!(spans("just some words").is_empty());
    }

    #[test]
    fn heading_scale_has_three_sizes_then_flattens() {
        // No hash = body; h1 > h2 > h3; 4+ hashes share the h3 size (no finer ramp).
        assert_eq!(heading_scale(0), 1.0, "no hash => body size");
        assert!(heading_scale(1) > heading_scale(2), "h1 > h2");
        assert!(heading_scale(2) > heading_scale(3), "h2 > h3");
        assert!(heading_scale(3) > 1.0, "h3 still bigger than body");
        assert_eq!(heading_scale(4), heading_scale(3), "4+ hashes == h3");
        assert_eq!(heading_scale(9), heading_scale(3), "deep counts clamp to h3");
    }
}
