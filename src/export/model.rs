//! THE ONE EXPORTER CORE: a single walk of `pulldown-cmark`'s events into a
//! neutral [`Document`] tree that BOTH emitters (`docx`, `html`) consume. The
//! renderer's own `markdown::spans` walk styles the LIVE buffer in place (spans
//! over the source bytes); an export instead wants a nesting-aware TREE (a
//! blockquote's paragraphs, a list's items, a table's cells), so this is a
//! second, purpose-built fold — but it reuses the SAME parse configuration
//! (`Options::ENABLE_TASKLISTS | ENABLE_TABLES`, plus `ENABLE_STRIKETHROUGH`
//! since `~~strike~~` is in the export's coverage) so the two never disagree on
//! what the markdown MEANS.
//!
//! STRIKETHROUGH shares the RENDERER's exactly-two-tilde gate: pulldown's GFM
//! option ALSO parses single-tilde `~x~`, but awl keeps that INERT (only `~~x~~`
//! strikes). This fold reads the SAME owner the renderer does
//! ([`crate::markdown::strike_engaged`]) off the offset iterator's span slice, so
//! `~x~` exports UNSTRUCK exactly as it renders — the render/export strike
//! divergence this closed. An inert single-tilde span contributes its inner
//! content to the parent (inheriting the surrounding strike context) and drops
//! its `~` delimiters, so the set of struck text matches `markdown::spans` byte
//! for byte (see the `render_export_strikethrough_agree` law test).
//!
//! `==highlight==` is NOT a CommonMark construct (pulldown emits it as literal
//! text), so — reading the SAME delimiter gate `markdown::spans` renders through
//! ([`crate::markdown::equals_runs`], the shared owner) — we split each text run
//! on isolated `==…==` pairs into [`Inline::Highlight`] runs after the fact, and
//! the two paths can't disagree on which `==` runs count (see the
//! `render_export_highlight_agree` law test). FRONTMATTER is excluded up front
//! (`frontmatter::detect`), matching its exclusion from word-count / spell / render.
//!
//! PURE + DETERMINISTIC: `parse` is a function of the text alone; images are
//! resolved through a caller-supplied [`ImageSource`] (the live App reads disk;
//! tests hand in a fixed map), so the tree — and therefore every exported byte —
//! is reproducible.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// A resolved, embeddable image: raw file bytes + intrinsic pixel dimensions +
/// which of the two container-supported encodings it is. The exporter never
/// decodes pixels; it only needs the header dims (for the DOCX drawing extent /
/// the HTML `width`) and the raw bytes (to embed). A source the resolver can't
/// turn into one of these — a remote URL, a missing file, an unsupported format,
/// or the wasm build with no readable path — yields `None`, and the image
/// degrades to its alt text (never a broken embed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportImage {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub mime: ImageMime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageMime {
    Png,
    Jpeg,
}

impl ImageMime {
    /// The file extension the DOCX media part carries (drives the
    /// `[Content_Types].xml` default + the `word/media/imageN.<ext>` name).
    pub fn ext(self) -> &'static str {
        match self {
            ImageMime::Png => "png",
            ImageMime::Jpeg => "jpeg",
        }
    }

    /// The MIME type string (HTML `data:` URI prefix + the DOCX content type).
    pub fn mime_str(self) -> &'static str {
        match self {
            ImageMime::Png => "image/png",
            ImageMime::Jpeg => "image/jpeg",
        }
    }
}

/// How a document reference (`![alt](src)`) becomes an embeddable image. The
/// live App implements this over its filesystem + the doc directory; a test
/// implements it over a fixed map. Returning `None` is always valid (the image
/// falls back to alt text).
pub trait ImageSource {
    fn resolve(&self, src: &str) -> Option<ExportImage>;
}

/// Sniff the intrinsic dimensions + encoding of an image from its leading bytes,
/// WITHOUT decoding pixels. Handles PNG (the IHDR width/height at a fixed offset)
/// and baseline/progressive JPEG (scan for an `SOFn` marker). Returns `None` for
/// anything else — the caller then drops the image to alt text. Pure + total, so
/// the App's resolver and the tests share one honest dimension source.
pub fn sniff_image(bytes: &[u8]) -> Option<(u32, u32, ImageMime)> {
    // PNG: \x89PNG\r\n\x1a\n then an IHDR chunk whose data starts at byte 16
    // (width u32 BE at 16, height u32 BE at 20).
    const PNG_SIG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() >= 24 && bytes.starts_with(PNG_SIG) && &bytes[12..16] == b"IHDR" {
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        if w > 0 && h > 0 {
            return Some((w, h, ImageMime::Png));
        }
    }
    // JPEG: starts with SOI (FFD8); walk segment markers to the first SOFn
    // (0xC0..=0xCF except C4/C8/CC), whose payload holds height then width (BE).
    if bytes.len() >= 4 && bytes[0] == 0xFF && bytes[1] == 0xD8 {
        let mut i = 2usize;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = bytes[i + 1];
            // Standalone markers (no length): padding 0xFF, RSTn, SOI, EOI.
            if marker == 0xFF || (0xD0..=0xD9).contains(&marker) {
                i += 2;
                continue;
            }
            let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            let is_sof = (0xC0..=0xCF).contains(&marker)
                && marker != 0xC4
                && marker != 0xC8
                && marker != 0xCC;
            if is_sof && i + 9 < bytes.len() {
                let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
                if w > 0 && h > 0 {
                    return Some((w, h, ImageMime::Jpeg));
                }
            }
            if len < 2 {
                break;
            }
            i += 2 + len;
        }
    }
    None
}

// --- The neutral document tree ---------------------------------------------

/// A block-level node.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading { level: u8, inlines: Vec<Inline> },
    Paragraph(Vec<Inline>),
    BlockQuote(Vec<Block>),
    CodeBlock { lang: Option<String>, code: String },
    List(List),
    Rule,
    Table(Table),
}

#[derive(Clone, Debug, PartialEq)]
pub struct List {
    pub ordered: bool,
    pub start: u64,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// `Some(checked)` for a GFM task-list item, `None` for a plain one.
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub aligns: Vec<Align>,
    pub head: Vec<Vec<Inline>>,
    pub rows: Vec<Vec<Vec<Inline>>>,
}

/// An inline-level node.
#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Highlight(Vec<Inline>),
    Code(String),
    Link {
        url: String,
        children: Vec<Inline>,
    },
    Image {
        src: String,
        alt: String,
        width_hint: Option<u32>,
    },
    SoftBreak,
    HardBreak,
}

/// The whole parsed document: an optional title (the first `# heading`, used for
/// the HTML `<title>` + DOCX core), and the block tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub title: Option<String>,
    pub blocks: Vec<Block>,
}

// --- The walk ---------------------------------------------------------------

/// Parse `markdown` into a [`Document`]. Strips a leading frontmatter block
/// (excluded from the export, matching every other awl consumer) and folds the
/// pulldown event stream into the nesting-aware tree.
pub fn parse(markdown: &str) -> Document {
    let body_start = crate::frontmatter::detect(markdown)
        .map(|f| f.range.end)
        .unwrap_or(0);
    let src = &markdown[body_start..];

    let opts = Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let mut stack: Vec<Frame> = vec![Frame::Root(Vec::new())];
    // A pending task marker: pulldown emits `TaskListMarker` at the START of the
    // item's paragraph, so we stash it and stamp the enclosing Item on close.
    let mut pending_task: Option<bool> = None;

    // The OFFSET iterator: each event carries its byte range into `src`, which the
    // strikethrough gate needs (the exactly-two-tilde decision reads the span's
    // source slice — see `crate::markdown::strike_engaged`).
    for (ev, range) in Parser::new_ext(src, opts).into_offset_iter() {
        match ev {
            // STRIKETHROUGH is gated at its Start on the SHARED exactly-two-tilde
            // owner: an ENGAGED `~~x~~` opens a real `Strikethrough` frame; an
            // INERT single-tilde `~x~` opens a passthrough frame whose children
            // flush UNWRAPPED to the parent on close (so `~x~` is never struck,
            // matching the renderer). Every other Start routes through `open_frame`.
            Event::Start(Tag::Strikethrough) => {
                if crate::markdown::strike_engaged(&src[range.clone()]) {
                    stack.push(Frame::Strikethrough(Vec::new()));
                } else {
                    stack.push(Frame::StrikethroughInert(Vec::new()));
                }
            }
            Event::Start(tag) => open_frame(&mut stack, tag),
            Event::End(tag) => close_frame(&mut stack, tag, &mut pending_task),
            Event::Text(t) => push_text(&mut stack, &t),
            Event::Code(c) => push_inline(&mut stack, Inline::Code(c.into_string())),
            Event::SoftBreak => push_inline(&mut stack, Inline::SoftBreak),
            Event::HardBreak => push_inline(&mut stack, Inline::HardBreak),
            Event::Rule => accept_block(&mut stack, Block::Rule),
            Event::TaskListMarker(checked) => pending_task = Some(checked),
            // Raw HTML / footnote refs: flatten to their literal text (footnotes
            // are explicitly out of scope; inline HTML renders as text).
            Event::Html(h) | Event::InlineHtml(h) => push_text(&mut stack, &h),
            Event::FootnoteReference(_) => {}
            Event::InlineMath(_) | Event::DisplayMath(_) => {}
        }
    }

    let blocks = match stack.pop() {
        Some(Frame::Root(b)) => b,
        _ => Vec::new(),
    };
    let title = first_heading_text(&blocks);
    Document { title, blocks }
}

/// One open container on the fold stack. Block containers accumulate `Block`s;
/// inline containers accumulate `Inline`s; the table + image frames carry their
/// own bookkeeping.
enum Frame {
    Root(Vec<Block>),
    BlockQuote(Vec<Block>),
    List {
        ordered: bool,
        start: u64,
        items: Vec<Item>,
    },
    /// A list item. In a LOOSE list pulldown wraps each item's content in its
    /// own `Paragraph`; in a TIGHT list (no blank line between items — the
    /// dominant form) it emits the item's inlines BARE, with no paragraph. So
    /// the item is inline-accepting too: bare inlines land in `loose` and are
    /// flushed into an implicit `Paragraph` when a real block arrives or the
    /// item closes (dropping them was the tight-list content-loss bug).
    Item {
        blocks: Vec<Block>,
        loose: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    Heading {
        level: u8,
        inlines: Vec<Inline>,
    },
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    /// An INERT single-tilde `~x~` span — pulldown parsed it as strikethrough, but
    /// awl's exactly-two-tilde gate ([`crate::markdown::strike_engaged`]) keeps it
    /// UNSTRUCK. It collects its inner inlines exactly like any inline container,
    /// but on close it flushes them UNWRAPPED into the parent (dropping the `~`
    /// delimiters), so the content inherits the surrounding strike context and no
    /// `Inline::Strikethrough` node is ever produced — the render's inert behavior
    /// (the `~x~` bytes render as plain, unstruck text) projected into the tree.
    StrikethroughInert(Vec<Inline>),
    Link {
        url: String,
        children: Vec<Inline>,
    },
    Image {
        src: String,
        alt: String,
        width_hint: Option<u32>,
    },
    CodeBlock {
        lang: Option<String>,
        code: String,
    },
    Table(TableFrame),
}

struct TableFrame {
    aligns: Vec<Align>,
    head: Vec<Vec<Inline>>,
    rows: Vec<Vec<Vec<Inline>>>,
    in_head: bool,
    /// Cells of the row currently being built (head or body).
    cur_row: Vec<Vec<Inline>>,
    /// The inline run of the cell currently open (between TableCell start/end).
    cur_cell: Option<Vec<Inline>>,
}

fn open_frame(stack: &mut Vec<Frame>, tag: Tag) {
    match tag {
        Tag::Paragraph => stack.push(Frame::Paragraph(Vec::new())),
        Tag::Heading { level, .. } => stack.push(Frame::Heading {
            level: heading_level(level),
            inlines: Vec::new(),
        }),
        Tag::BlockQuote(_) => stack.push(Frame::BlockQuote(Vec::new())),
        Tag::CodeBlock(kind) => {
            let lang = match kind {
                CodeBlockKind::Fenced(info) => {
                    let first = info.split_whitespace().next().unwrap_or("");
                    if first.is_empty() {
                        None
                    } else {
                        Some(first.to_string())
                    }
                }
                CodeBlockKind::Indented => None,
            };
            stack.push(Frame::CodeBlock {
                lang,
                code: String::new(),
            })
        }
        Tag::List(start) => stack.push(Frame::List {
            ordered: start.is_some(),
            start: start.unwrap_or(1),
            items: Vec::new(),
        }),
        Tag::Item => stack.push(Frame::Item {
            blocks: Vec::new(),
            loose: Vec::new(),
        }),
        Tag::Emphasis => stack.push(Frame::Emphasis(Vec::new())),
        Tag::Strong => stack.push(Frame::Strong(Vec::new())),
        // `Tag::Strikethrough` is intercepted in `parse`'s loop BEFORE `open_frame`
        // (it needs the span's byte range for the exactly-two-tilde gate), so it
        // never arrives here.
        Tag::Link { dest_url, .. } => stack.push(Frame::Link {
            url: dest_url.into_string(),
            children: Vec::new(),
        }),
        Tag::Image { dest_url, .. } => stack.push(Frame::Image {
            src: dest_url.into_string(),
            alt: String::new(),
            width_hint: None,
        }),
        Tag::Table(aligns) => stack.push(Frame::Table(TableFrame {
            aligns: aligns.iter().map(|a| map_align(*a)).collect(),
            head: Vec::new(),
            rows: Vec::new(),
            in_head: false,
            cur_row: Vec::new(),
            cur_cell: None,
        })),
        Tag::TableHead => {
            if let Some(Frame::Table(t)) = stack.last_mut() {
                t.in_head = true;
                t.cur_row = Vec::new();
            }
        }
        Tag::TableRow => {
            if let Some(Frame::Table(t)) = stack.last_mut() {
                t.cur_row = Vec::new();
            }
        }
        Tag::TableCell => {
            if let Some(Frame::Table(t)) = stack.last_mut() {
                t.cur_cell = Some(Vec::new());
            }
        }
        // Out-of-scope containers (footnote definitions, HTML blocks, metadata):
        // open a throwaway paragraph so their inner text has somewhere to land
        // without corrupting a real block; it's dropped as an empty paragraph if
        // it collects nothing.
        _ => stack.push(Frame::Paragraph(Vec::new())),
    }
}

fn close_frame(stack: &mut Vec<Frame>, tag: TagEnd, pending_task: &mut Option<bool>) {
    // Table sub-elements close in place (no frame of their own).
    match tag {
        TagEnd::TableCell => {
            if let Some(Frame::Table(t)) = stack.last_mut() {
                let cell = t.cur_cell.take().unwrap_or_default();
                t.cur_row.push(cell);
            }
            return;
        }
        TagEnd::TableHead => {
            if let Some(Frame::Table(t)) = stack.last_mut() {
                t.head = std::mem::take(&mut t.cur_row);
                t.in_head = false;
            }
            return;
        }
        TagEnd::TableRow => {
            if let Some(Frame::Table(t)) = stack.last_mut() {
                let row = std::mem::take(&mut t.cur_row);
                t.rows.push(row);
            }
            return;
        }
        _ => {}
    }

    let Some(frame) = stack.pop() else { return };
    match frame {
        Frame::Paragraph(inlines) => {
            if !inlines.is_empty() {
                accept_block(stack, Block::Paragraph(inlines));
            }
        }
        Frame::Heading { level, inlines } => accept_block(stack, Block::Heading { level, inlines }),
        Frame::BlockQuote(blocks) => accept_block(stack, Block::BlockQuote(blocks)),
        Frame::CodeBlock { lang, mut code } => {
            // pulldown includes a trailing newline on the final code text run;
            // trim exactly one so the emitters don't render a blank last line.
            if code.ends_with('\n') {
                code.pop();
            }
            accept_block(stack, Block::CodeBlock { lang, code });
        }
        Frame::List {
            ordered,
            start,
            items,
        } => accept_block(
            stack,
            Block::List(List {
                ordered,
                start,
                items,
            }),
        ),
        Frame::Item {
            mut blocks,
            mut loose,
        } => {
            // A tight item's bare inlines (or a trailing loose run after a
            // nested block) become an implicit paragraph so they survive.
            if !loose.is_empty() {
                blocks.push(Block::Paragraph(std::mem::take(&mut loose)));
            }
            let task = pending_task.take();
            if let Some(Frame::List { items, .. }) = stack.last_mut() {
                items.push(Item { task, blocks });
            }
        }
        Frame::Emphasis(children) => push_inline(stack, Inline::Emphasis(children)),
        Frame::Strong(children) => push_inline(stack, Inline::Strong(children)),
        Frame::Strikethrough(children) => push_inline(stack, Inline::Strikethrough(children)),
        // An inert single-tilde span: flush its inner inlines UNWRAPPED into the
        // parent (in source order), dropping the `~` delimiters. Nested inside an
        // engaged `~~…~~` the content lands in that Strikethrough frame (struck);
        // at top level it lands in the paragraph (unstruck) — matching the render.
        Frame::StrikethroughInert(children) => {
            for c in children {
                push_inline(stack, c);
            }
        }
        Frame::Link { url, children } => push_inline(stack, Inline::Link { url, children }),
        Frame::Image {
            src,
            alt,
            width_hint,
        } => push_inline(
            stack,
            Inline::Image {
                src,
                alt,
                width_hint,
            },
        ),
        Frame::Table(t) => accept_block(
            stack,
            Block::Table(Table {
                aligns: t.aligns,
                head: t.head,
                rows: t.rows,
            }),
        ),
        Frame::Root(_) => {}
    }
}

/// Append `block` to the nearest block-accepting container.
fn accept_block(stack: &mut [Frame], block: Block) {
    for frame in stack.iter_mut().rev() {
        match frame {
            Frame::Root(b) | Frame::BlockQuote(b) => {
                b.push(block);
                return;
            }
            Frame::Item { blocks, loose } => {
                // Flush any bare inlines gathered before this block into an
                // implicit paragraph FIRST, preserving source order.
                if !loose.is_empty() {
                    blocks.push(Block::Paragraph(std::mem::take(loose)));
                }
                blocks.push(block);
                return;
            }
            _ => {}
        }
    }
}

/// Append `inline` to the nearest inline-accepting container (paragraph,
/// heading, emphasis/strong/strike, link, the open table cell, or a tight list
/// item's bare-inline run).
fn push_inline(stack: &mut [Frame], inline: Inline) {
    for frame in stack.iter_mut().rev() {
        match frame {
            Frame::Paragraph(v)
            | Frame::Heading { inlines: v, .. }
            | Frame::Emphasis(v)
            | Frame::Strong(v)
            | Frame::Strikethrough(v)
            | Frame::StrikethroughInert(v)
            | Frame::Link { children: v, .. } => {
                v.push(inline);
                return;
            }
            Frame::Table(t) => {
                if let Some(cell) = t.cur_cell.as_mut() {
                    cell.push(inline);
                }
                return;
            }
            // A tight list item is inline-accepting: bare inlines (no enclosing
            // paragraph) gather here and are flushed to an implicit paragraph on
            // block-accept / item close. A LOOSE item has its own Paragraph
            // frame ABOVE this one, so the rev-walk reaches that first and this
            // arm never fires for it.
            Frame::Item { loose, .. } => {
                loose.push(inline);
                return;
            }
            _ => {}
        }
    }
}

/// A text run: routed to the open image's ALT (with the Obsidian `|WIDTH` hint
/// split out), to an open code block's body, or split on `==highlight==` pairs
/// into the nearest inline container.
fn push_text(stack: &mut Vec<Frame>, text: &str) {
    if let Some(Frame::Image {
        alt, width_hint, ..
    }) = stack.last_mut()
    {
        // Alt text may carry the `|300` / `|300x200` size hint.
        let (a, hint) = split_alt_hint(text);
        alt.push_str(&a);
        if hint.is_some() {
            *width_hint = hint;
        }
        return;
    }
    if let Some(Frame::CodeBlock { code, .. }) = stack.last_mut() {
        code.push_str(text);
        return;
    }
    for inline in split_highlight(text) {
        push_inline(stack, inline);
    }
}

// --- Highlight scan (`==marked==`) -----------------------------------------

/// Split one text run into [`Inline::Text`] / [`Inline::Highlight`] runs on
/// isolated `==…==` pairs — the same de-facto Obsidian/Typora convention
/// `markdown::spans` renders (exactly-two `=`, greedy pairing, an unpaired
/// trailing marker left literal). pulldown never embeds a `\n` in one text run
/// (soft breaks are their own events), so single-line pairing is exact. The
/// delimiter gate is the SHARED owner [`crate::markdown::equals_runs`] — the same
/// candidate set the renderer scans, so render and export can't disagree on which
/// `==` runs count (see the `render_export_highlight_agree` law test).
fn split_highlight(text: &str) -> Vec<Inline> {
    let markers = crate::markdown::equals_runs(text);
    if markers.len() < 2 {
        return vec![Inline::Text(text.to_string())];
    }
    let mut out = Vec::new();
    let mut cursor = 0usize;
    let mut k = 0usize;
    while k + 1 < markers.len() {
        let open = markers[k].clone();
        let close = markers[k + 1].clone();
        if cursor < open.start {
            out.push(Inline::Text(text[cursor..open.start].to_string()));
        }
        let inner = &text[open.end..close.start];
        out.push(Inline::Highlight(vec![Inline::Text(inner.to_string())]));
        cursor = close.end;
        k += 2;
    }
    if cursor < text.len() {
        out.push(Inline::Text(text[cursor..].to_string()));
    }
    out
}

/// Split an image's alt on the Obsidian size hint: `alt|300` → `("alt", 300)`,
/// `alt|300x200` → `("alt", 300)` (width only, height derived from aspect).
fn split_alt_hint(alt: &str) -> (String, Option<u32>) {
    if let Some((a, hint)) = alt.rsplit_once('|') {
        let width = hint
            .split(['x', 'X'])
            .next()
            .and_then(|w| w.trim().parse::<u32>().ok());
        if let Some(w) = width {
            return (a.to_string(), Some(w));
        }
    }
    (alt.to_string(), None)
}

// --- Small mappers ----------------------------------------------------------

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn map_align(a: pulldown_cmark::Alignment) -> Align {
    use pulldown_cmark::Alignment::*;
    match a {
        None => Align::None,
        Left => Align::Left,
        Center => Align::Center,
        Right => Align::Right,
    }
}

/// The first `# heading`'s plain text — the document title (HTML `<title>`).
fn first_heading_text(blocks: &[Block]) -> Option<String> {
    for b in blocks {
        if let Block::Heading { inlines, .. } = b {
            let t = plain_text(inlines);
            if !t.trim().is_empty() {
                return Some(t.trim().to_string());
            }
        }
    }
    None
}

/// Flatten an inline run to its plain text (titles, image alts).
pub fn plain_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for i in inlines {
        match i {
            Inline::Text(t) | Inline::Code(t) => out.push_str(t),
            Inline::Strong(c)
            | Inline::Emphasis(c)
            | Inline::Strikethrough(c)
            | Inline::Highlight(c)
            | Inline::Link { children: c, .. } => out.push_str(&plain_text(c)),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::SoftBreak | Inline::HardBreak => out.push(' '),
        }
    }
    out
}
