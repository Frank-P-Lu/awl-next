//! Markdown styling spans — the "dim the markup + style the content" model for
//! awl's prose docs. We parse the document with `pulldown-cmark`'s OFFSET
//! iterator (each event carries its byte range into the source text) and turn
//! the events into a flat list of `(byte-range, MdKind)` spans. The renderer
//! lays these as per-span `Attrs` over each line's `AttrsList`, exactly like the
//! CJK + focus spans — the markup characters (`#`, `*`, `` ` ``, `>`, list
//! markers, link brackets/URL) recede to the DIM ink while staying fully present
//! and editable, and the CONTENT gains structure (bold weight, italic style,
//! mono code, heading SIZE, accent link text). Headings take NO accent color and
//! NO bold — figure/ground by value + size, so the amber stays the caret's alone
//! (DESIGN.md §3, the one-organic-element law) and the title renders in the world's
//! own face at any size (the bundled faces are Regular-only, so bold would fall
//! back to mono on a serif/sans world).
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
    /// A heading's CONTENT text. Drives a larger font SIZE per [`heading_scale`]
    /// (applied per-line in `render.rs`) — no bold/color: size + value carry it, and
    /// the bundled faces are Regular-only so requesting bold would fall back to mono.
    Heading(u8),
    /// `**bold**` / `__bold__` content → Bold weight.
    Bold,
    /// `*italic*` / `_italic_` content → Italic style.
    Italic,
    /// `***both***` content → Bold + Italic.
    BoldItalic,
    /// Inline `` `code` `` + fenced/indented code-block body → mono family + tint.
    Code,
    /// A FENCED code-block body byte that a recognized info-string language lexed
    /// into an Alabaster syntax ROLE. It rides the SAME mono family as [`MdKind::Code`]
    /// (the fence body is mono) but takes the syntax role's VALUE-based color instead
    /// of the flat Code tint — so a ```` ```rust ```` fence highlights its comments /
    /// strings / constants / definitions in mono, while the fence markers + info
    /// string stay dim [`MdKind::Markup`]. Carries the `role` (which color) and the
    /// `lang` (for the sidecar). Emitted ONLY inside a recognized fence, so an
    /// unknown-lang / no-lang fence and every non-fence buffer stay byte-identical.
    CodeSyntax { role: crate::syntax::SynKind, lang: crate::syntax::Lang },
    /// Blockquote TEXT → dim (the `>` marker is `Markup`).
    Quote,
    /// A list item's leading marker (`-`/`*`/`+`/`1.`) → dim.
    ListMarker,
    /// A link's visible TEXT → accent color (the brackets + URL are `Markup`).
    LinkText,
    /// A task-list checkbox marker (`[ ]` open / `[x]` checked, plus its trailing
    /// space). The bool is the CHECKED state. Rendered distinctly by value: an open
    /// box stays present (full ink), a checked box recedes to the DIM ink — no accent,
    /// figure/ground by value, amber is the caret's alone (DESIGN §3).
    Task(bool),
    /// The TEXT of a CHECKED task item → DIM, so a completed line recedes the way a
    /// struck-through todo does. An open task's text rides the default ink.
    TaskDone,
    /// A horizontal rule line (`---`/`***`/`___` alone on a line). An hr is pure
    /// MARKUP with no content, so the renderer drops a centered ornament on the row —
    /// which ONE depends on the syntax the author typed (see [`BreakKind`]): `---` →
    /// ❧, `***` → ⁂, `___` → ❦ by default — and — REVEAL-ON-CURSOR — CONCEALS the raw
    /// `---` glyphs (transparent ink) whenever the caret is NOT on the line, so a
    /// settled rule reads as a clean fine-press break. When the caret IS on the line
    /// the raw characters REVEAL (dim markup, fully editable) and the ornament yields
    /// to them. The conceal/reveal toggle lives in the renderer
    /// (`spans::add_rule_conceal_span` + `TextPipeline::rule_lines`), keyed off the
    /// cursor line; this span only marks WHERE the rule is.
    Rule,
}

/// WHICH of markdown's three thematic-break syntaxes a `Rule` line was typed with.
/// All three render a `<hr>` in standard markdown, but awl makes each EXPRESSIVE:
/// the syntax picks the ornament (see [`crate::theme::Ornaments`]). Detected from
/// the line's first run character by [`break_kind`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BreakKind {
    /// `---` (three-or-more dashes).
    Dash,
    /// `***` (three-or-more asterisks).
    Star,
    /// `___` (three-or-more underscores).
    Underscore,
}

/// Classify a thematic-break line by its RUN CHARACTER, per CommonMark: a thematic
/// break is a line of three-or-more matching `-`, `*`, or `_`, which MAY be
/// separated and surrounded by spaces/tabs (and indented up to 3 spaces). Since the
/// run char is uniform across a valid break, the FIRST non-space `-`/`*`/`_` decides
/// the kind. Callers only ask about lines pulldown already ruled a thematic break;
/// anything unexpected falls back to [`BreakKind::Dash`] (the plainest ornament).
pub fn break_kind(line: &str) -> BreakKind {
    for ch in line.chars() {
        match ch {
            '-' => return BreakKind::Dash,
            '*' => return BreakKind::Star,
            '_' => return BreakKind::Underscore,
            _ => {}
        }
    }
    BreakKind::Dash
}

/// The TYPE SCALE — awl's SIZE LADDER, one of the two ladders in the text system
/// (the other is the ink ramp in `theme.rs`: `base_content` / `muted` / `faint`).
/// Every element is exactly ONE ink × ONE size (DESIGN.md §4), and these named
/// tiers are the size half: each is a multiplier over the body metrics. Naming the
/// rungs (rather than scattering bare `1.8`/`1.5` literals) makes the ladder
/// explicit and keeps the ratios tunable in ONE place.
pub mod type_scale {
    /// h1 — the document / top TITLE (the biggest rung).
    pub const TITLE: f32 = 1.8;
    /// h2 — a SECTION head.
    pub const SECTION: f32 = 1.5;
    /// h3+ — a SUBHEAD. Nudged from the old 1.3 to 1.25 so the steps down the
    /// ladder ease evenly (it sits closer to body, smoothing the ratio bump).
    pub const SUBHEAD: f32 = 1.25;
    /// BODY prose / code — the baseline rung (no scaling).
    pub const BODY: f32 = 1.0;
    /// LABEL — UI metadata that should read SMALLER than body: a future gutter's
    /// line numbers, the stats / word-count readout. Pairs with the `faint` ink
    /// (DESIGN.md §4). Defined now; consumed by the later gutter/stats pass.
    #[allow(dead_code)] // reserved for the gutter/stats pass (see DESIGN.md §4).
    pub const LABEL: f32 = 0.8;
}

/// The font / line-height SCALE for a heading, by the COUNT of leading `#` marks
/// (1, 2, 3+), in terms of the named [`type_scale`] rungs. Only THREE distinct
/// sizes: past `###` nobody wants a finer ramp, so 4+ hashes share the `h3`
/// ([`type_scale::SUBHEAD`]) size. `0` (no hash) is [`type_scale::BODY`]. This is
/// the SINGLE source of truth for heading size: `render.rs` reads it from a line's
/// leading-`#` run (NOT from a fully-valid ATX heading — so a line grows the moment
/// you type `#`, before the space + title), lays the line's `Attrs::metrics` at
/// `base * scale`, and cosmic-text takes the row height from the max of its glyphs'
/// line heights, so the whole heading row grows by exactly this factor. Tune the
/// *feel* via the [`type_scale`] tiers, in one place.
pub fn heading_scale(level: u8) -> f32 {
    use type_scale::*;
    match level {
        0 => BODY,
        1 => TITLE,
        2 => SECTION,
        _ => SUBHEAD,
    }
}

/// The DEPTH-CYCLING unordered-list bullet glyphs, one per nesting level modulo 3:
/// depth 0 → `•` (U+2022), depth 1 → `◦` (U+25E6), depth 2 → `▪` (U+25AA), then the
/// cycle repeats. The glyph is DERIVED FROM DEPTH, independent of which marker char
/// (`-`/`*`/`+`) the author typed — so re-indenting a line (Tab) re-derives the glyph
/// for free. All three are bundled in `AwlSymbols.ttf` (see
/// [`crate::render::spans::is_symbol`]/`SYMBOL_FAMILY`), so they render — never tofu —
/// in every world and on the web build. See [`bullet_for_depth`].
pub const BULLETS: [char; 3] = ['•', '◦', '▪'];

/// The bullet glyph for an unordered-list item at nesting `depth` (0 = top level),
/// cycling [`BULLETS`] every three levels. Pure + total.
pub fn bullet_for_depth(depth: usize) -> char {
    BULLETS[depth % BULLETS.len()]
}

/// The number of leading-indent SPACES that make up ONE nesting level for a
/// markdown list. awl's list model is "every 2 spaces = one level" (see
/// [`ListItem::depth`]); this is the single place that ratio lives, shared by the
/// depth derivation (rendering) and the Tab/Shift-Tab indent step (editing).
pub const LIST_INDENT: usize = 2;

/// A detected markdown LIST ITEM on ONE line — the SHARED list-detection primitive
/// behind both the depth-derived bullet GLYPH (rendering, `spans.rs`/`rects.rs`) and
/// the Tab/Shift-Tab indent/outdent EDIT (`actions.rs`/`buffer`). Pure per-line scan
/// (no full parse), matching the per-line precedent of [`crate::render::spans::md_line_scale`]:
/// optional leading spaces, then either an unordered marker (`-`/`*`/`+`) or an
/// ordered one (digits + `.`/`)`), then a REQUIRED single space. Byte offsets are
/// into the line; since the indent is spaces, `indent` is both the leading-space
/// COUNT and the marker char's byte offset. See [`list_item`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListItem {
    /// Leading-space count == the byte/char offset of the marker character.
    pub indent: usize,
    /// True for an ordered (`1.`) item; false for an unordered (`-`) bullet. Only
    /// unordered items get a depth-cycling bullet glyph — ordered keep their number.
    pub ordered: bool,
    /// Byte offset where the item's CONTENT begins (after the marker + its space).
    pub content: usize,
    /// True when the item has no content (just the marker + optional trailing
    /// whitespace) — the "empty item" that Enter ends the list on.
    pub empty: bool,
}

impl ListItem {
    /// Nesting depth = leading spaces / [`LIST_INDENT`] (every 2 spaces one level).
    pub fn depth(&self) -> usize {
        self.indent / LIST_INDENT
    }
}

/// Detect a markdown list item on `line` — the SHARED detection used by the bullet
/// glyph, its reveal-on-cursor concealment, and the Tab/Shift-Tab indent edit.
/// Recognizes, after optional leading spaces, an unordered marker (`-`/`*`/`+`) or an
/// ordered one (a digit run + `.`/`)`), each REQUIRING a single following space (so a
/// bare `-` or `12 monkeys` is NOT a list). Returns `None` for a non-list line. Pure.
pub fn list_item(line: &str) -> Option<ListItem> {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    let indent = i;
    if i >= b.len() {
        return None;
    }
    let ordered = if matches!(b[i], b'-' | b'*' | b'+') {
        i += 1;
        false
    } else if b[i].is_ascii_digit() {
        let d0 = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i > d0 && i < b.len() && (b[i] == b'.' || b[i] == b')') {
            i += 1;
            true
        } else {
            return None;
        }
    } else {
        return None;
    };
    // A real list item's marker is followed by a single space.
    if i >= b.len() || b[i] != b' ' {
        return None;
    }
    i += 1;
    let content = i;
    let empty = line[content..].chars().all(|c| c.is_whitespace());
    Some(ListItem { indent, ordered, content, empty })
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
            // Role-only tag; the capture sidecar's `md_report` enriches a fence span
            // with its language (see `render::TextPipeline::md_report`).
            MdKind::CodeSyntax { role, .. } => match role {
                crate::syntax::SynKind::Comment => "code_comment",
                crate::syntax::SynKind::CommentCode => "code_comment_code",
                crate::syntax::SynKind::Str => "code_string",
                crate::syntax::SynKind::Constant => "code_constant",
                crate::syntax::SynKind::Definition => "code_definition",
            },
            MdKind::Quote => "quote",
            MdKind::ListMarker => "list_marker",
            MdKind::LinkText => "link_text",
            MdKind::Task(false) => "task_open",
            MdKind::Task(true) => "task_checked",
            MdKind::TaskDone => "task_done",
            MdKind::Rule => "rule",
        }
    }
}

/// The words-per-minute used to turn a word count into a reading-time estimate.
/// 200 wpm is the conventional silent-prose reading rate; this is the SINGLE place
/// it is defined, so the readout and its test agree.
pub const READING_WPM: usize = 200;

/// Count words in `text` — whitespace-separated tokens. A blank document is 0.
/// Pure + cheap; markup characters ride along with their word (`**bold**` counts as
/// one), which is a fine approximation for a calm prose readout.
pub fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Estimate reading time in WHOLE minutes for `words` at [`READING_WPM`], rounded
/// UP so any prose reads as at least `1 min`. Zero words → `0` (nothing to read).
pub fn reading_time_min(words: usize) -> usize {
    if words == 0 {
        0
    } else {
        words.div_ceil(READING_WPM)
    }
}

/// Parse `text` into styling spans in DOCUMENT byte coordinates. Spans may
/// overlap by DESIGN: a link or code-block first pushes a whole-range `Markup`
/// span, then its inner text pushes a `LinkText`/`Code` span — applied in this
/// order, the later (inner) span wins for its bytes while the brackets/URL/fence
/// keep the dim `Markup`. The renderer adds them to the `AttrsList` in THIS
/// order, relying on cosmic-text's "last span wins on overlap" semantics.
pub fn spans(text: &str) -> Vec<(Range<usize>, MdKind)> {
    use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

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
    // FENCE SYNTAX: `Some((lang, body_start, body_end))` while inside a FENCED code
    // block whose info string named a recognized language. The body byte extent is
    // grown across the block's Text events and lexed as ONE unit at the block's End,
    // so multi-line constructs (block comments, strings) resolve. Left `None` for an
    // indented block, or a fenced block with an unknown / absent language — those
    // keep the plain mono `Code` body and stay byte-identical.
    let mut fence: Option<(crate::syntax::Lang, Option<usize>, usize)> = None;
    // A CHECKED task colours its body text DIM. Set on the checked `TaskListMarker`
    // and cleared at the item's end; flat task lists (the common case) resolve
    // cleanly. A checked PARENT with nested children loses the flag to the child's
    // marker — accepted to keep the walk single-pass.
    let mut task_done = false;

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

    // ENABLE_TASKLISTS so `- [ ]` / `- [x]` surface as `TaskListMarker` events;
    // every other construct parses exactly as before (the option is additive).
    let opts = Options::ENABLE_TASKLISTS;
    for (ev, range) in Parser::new_ext(text, opts).into_offset_iter() {
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
                Tag::CodeBlock(kind) => {
                    code_block += 1;
                    // Dim the WHOLE block (fences + info string); the body Text
                    // events below override their bytes to mono `Code`. An
                    // indented block has no fence, so this just becomes the body.
                    out.push((range.clone(), MdKind::Markup));
                    // A FENCED block whose info string names a recognized language
                    // arms the body accumulator; its End (below) lexes the body and
                    // emits per-role `CodeSyntax` spans over the mono body. An
                    // indented / unknown-lang / no-lang block leaves `fence` None.
                    if let CodeBlockKind::Fenced(info) = kind {
                        if let Some(lang) = crate::syntax::Lang::from_info(&info) {
                            fence = Some((lang, None, 0));
                        }
                    }
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
                TagEnd::CodeBlock => {
                    code_block = code_block.saturating_sub(1);
                    // The fenced body is complete: lex it as ONE unit and translate
                    // the syntax spans into DOCUMENT byte offsets (body_start + span).
                    // Pushed AFTER the body `Code` spans so a role span WINS its bytes
                    // (mono face from `Code`, role color from `CodeSyntax`); the fence
                    // markers + info string keep the earlier dim `Markup`.
                    if let Some((lang, Some(bs), be)) = fence.take() {
                        if bs < be {
                            for (r, role) in crate::syntax::spans(lang, &text[bs..be]) {
                                out.push((bs + r.start..bs + r.end, MdKind::CodeSyntax { role, lang }));
                            }
                        }
                    }
                }
                TagEnd::Link => link = link.saturating_sub(1),
                TagEnd::Item => task_done = false,
                _ => {}
            },
            // A thematic break (`---`/`***`/`___` alone on a line): mark the literal
            // characters as a Rule; the renderer drops a centered fleuron on the row
            // and conceals the dashes unless the caret is editing the line.
            Event::Rule => out.push((range, MdKind::Rule)),
            // The `[ ]`/`[x]` checkbox. Style the marker (+ its trailing space)
            // distinctly; a CHECKED box also dims the item's body text.
            Event::TaskListMarker(checked) => {
                task_done = checked;
                push_task_marker(&mut out, text, &range, checked);
            }
            Event::Text(_) => {
                // FENCE SYNTAX: grow the recognized fenced block's body extent to
                // cover this text run (`range.start`/`.end` are copies, so `range`
                // still moves into the push below). Lexed at the block's End.
                if let Some((_, body_start, body_end)) = fence.as_mut() {
                    body_start.get_or_insert(range.start);
                    *body_end = range.end;
                }
                if let Some(k) =
                    inline_kind(heading, strong, emph, quote, link, code_block, task_done)
                {
                    out.push((range, k));
                }
            }
            Event::Code(_) => push_inline_code(&mut out, text, &range),
            _ => {}
        }
    }
    out
}

/// One document HEADING, distilled for the summoned outline picker: its `level`
/// (1-6), the trimmed title `text`, and the 0-based `line` it sits on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub line: usize,
}

impl Heading {
    /// The picker DISPLAY label: the title indented two spaces per level below the
    /// top, so a flat list still reads as a tree (h1 flush-left, h2 indented, …).
    /// The indentation is cosmetic — the fuzzy filter still matches the title text,
    /// and Enter jumps by [`Heading::line`], never by this string.
    pub fn label(&self) -> String {
        let depth = self.level.saturating_sub(1) as usize;
        format!("{}{}", "  ".repeat(depth), self.text)
    }
}

/// The document's headings in document order, for the SUMMONED outline picker.
/// Derived from [`spans`]: every `MdKind::Heading(level)` span marks a heading's
/// TITLE text by byte range, so the title is `text[range]` (trimmed) and the line
/// is the count of newlines before the span. Covers ATX (`# …`) and setext
/// (`===`/`---` underline) headings alike (pulldown reports both). One entry per
/// heading line — a title built from several runs (e.g. `# a *b*`) emits multiple
/// Heading spans on the same line, so we keep the first. A heading whose title is
/// ENTIRELY styled (e.g. `# *all italic*`) yields no plain Heading span and is the
/// one documented gap; in practice outline titles are plain text. Empty for a
/// document with no headings (the caller then declines to summon the picker).
pub fn headings(text: &str) -> Vec<Heading> {
    let mut out: Vec<Heading> = Vec::new();
    for (range, kind) in spans(text) {
        let MdKind::Heading(level) = kind else {
            continue;
        };
        let line = text[..range.start].bytes().filter(|&b| b == b'\n').count();
        // One row per heading line: later spans on the SAME line are extra runs of
        // the same title (the spans arrive in document order), so skip them.
        if out.last().map(|h| h.line) == Some(line) {
            continue;
        }
        let title = text[range].trim().to_string();
        if title.is_empty() {
            continue;
        }
        out.push(Heading { level, text: title, line });
    }
    out
}

/// Pick the content style for a Text event from the active context, in priority
/// order: a code block wins (mono), then a heading (it owns its whole line), then a
/// CHECKED task (the whole line recedes), then a link's visible text (accent), then
/// a blockquote (dim), then emphasis. Plain body text returns `None` (it rides the
/// default ink — no span needed).
fn inline_kind(
    heading: Option<u8>,
    strong: u32,
    emph: u32,
    quote: u32,
    link: u32,
    code_block: u32,
    task_done: bool,
) -> Option<MdKind> {
    if code_block > 0 {
        Some(MdKind::Code)
    } else if let Some(l) = heading {
        Some(MdKind::Heading(l))
    } else if task_done {
        Some(MdKind::TaskDone)
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

/// Style a task checkbox: the `[ ]`/`[x]` marker `range` (from the `TaskListMarker`
/// event) plus the single space that follows it, so the whole checkbox + gap reads
/// as one unit. `checked` selects the open/closed [`MdKind::Task`] role.
fn push_task_marker(
    out: &mut Vec<(Range<usize>, MdKind)>,
    text: &str,
    range: &Range<usize>,
    checked: bool,
) {
    let mut end = range.end;
    let b = text.as_bytes();
    if end < b.len() && (b[end] == b' ' || b[end] == b'\t') {
        end += 1;
    }
    out.push((range.start..end, MdKind::Task(checked)));
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
    fn atx_closing_hashes_dim() {
        // `# Title #`: the leading `# ` AND the trailing ` #` both dim as Markup
        // (the backward close-fence scan), with `Title` the h1 content between.
        let s = spans("# Title #");
        assert!(has(&s, 0, 2, MdKind::Markup), "leading '# ' dim: {s:?}");
        assert!(has(&s, 2, 7, MdKind::Heading(1)), "'Title' is h1: {s:?}");
        assert!(has(&s, 7, 9, MdKind::Markup), "trailing ' #' close dim: {s:?}");
    }

    #[test]
    fn headings_extracts_level_text_and_line() {
        let doc = "# Title\n\nsome prose\n\n## Section A\n\nbody\n\n### Deep\n";
        let h = headings(doc);
        assert_eq!(h.len(), 3, "three headings: {h:?}");
        assert_eq!(h[0], Heading { level: 1, text: "Title".into(), line: 0 });
        assert_eq!(h[1], Heading { level: 2, text: "Section A".into(), line: 4 });
        assert_eq!(h[2], Heading { level: 3, text: "Deep".into(), line: 8 });
        // The picker label indents two spaces per level below the top.
        assert_eq!(h[0].label(), "Title");
        assert_eq!(h[1].label(), "  Section A");
        assert_eq!(h[2].label(), "    Deep");
    }

    #[test]
    fn headings_one_entry_per_line_for_styled_title() {
        // A title with an inline-styled run still yields ONE outline row for the
        // line (the first plain run), not a duplicate.
        let h = headings("# Hello *world*\n");
        assert_eq!(h.len(), 1, "one row per heading line: {h:?}");
        assert_eq!(h[0].line, 0);
        assert_eq!(h[0].level, 1);
    }

    #[test]
    fn headings_empty_without_headings() {
        assert!(headings("just some prose\nwith no headings\n").is_empty());
    }

    #[test]
    fn bold_run_has_dim_stars_and_bold_inner() {
        let s = spans("**bold**");
        assert!(has(&s, 0, 2, MdKind::Markup), "opening ** dim: {s:?}");
        assert!(has(&s, 6, 8, MdKind::Markup), "closing ** dim: {s:?}");
        assert!(has(&s, 2, 6, MdKind::Bold), "inner bold: {s:?}");
    }

    #[test]
    fn bold_italic_triple_star() {
        // `***x***` is BOTH strong and emphasis: pulldown nests an emphasis (outer
        // single `*`) around a strong (inner `**`), so the inner `x` is BoldItalic
        // and the three stars at each end dim as Markup (outer 1 + inner 2).
        let s = spans("***x***");
        assert!(has(&s, 3, 4, MdKind::BoldItalic), "inner x is bold+italic: {s:?}");
        assert!(has(&s, 0, 1, MdKind::Markup), "outer opening `*` dim: {s:?}");
        assert!(has(&s, 1, 3, MdKind::Markup), "inner opening `**` dim: {s:?}");
        assert!(has(&s, 4, 6, MdKind::Markup), "inner closing `**` dim: {s:?}");
        assert!(has(&s, 6, 7, MdKind::Markup), "outer closing `*` dim: {s:?}");
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
    fn multiline_and_nested_quote_markers_dim() {
        // A two-line blockquote emits ONE dim marker per line (the per-line
        // `[ \t]*(> ?)+` scan), not one for the whole range.
        let s = spans("> a\n> b");
        assert!(has(&s, 0, 2, MdKind::Markup), "first line '> ' marker: {s:?}");
        assert!(has(&s, 4, 6, MdKind::Markup), "second line '> ' marker: {s:?}");
        // A nested `>>` dims its whole leading marker run.
        let s = spans(">> deep");
        assert!(has(&s, 0, 3, MdKind::Markup), "'>> ' nested marker run dim: {s:?}");
    }

    #[test]
    fn list_marker_dim() {
        let s = spans("- item");
        assert!(has(&s, 0, 2, MdKind::ListMarker), "marker dim: {s:?}");
    }

    #[test]
    fn ordered_list_markers_dim() {
        // `1. ` and `12) ` ordered markers (digit run + `.`/`)` + space) dim as the
        // ListMarker role, just like a bullet.
        let s = spans("1. item");
        assert!(has(&s, 0, 3, MdKind::ListMarker), "'1. ' ordered marker: {s:?}");
        let s = spans("12) item");
        assert!(has(&s, 0, 4, MdKind::ListMarker), "'12) ' ordered marker: {s:?}");
        // A bare number that is NOT a list (no `.`/`)`) must not be mis-marked.
        let s = spans("12 monkeys");
        assert!(
            !s.iter().any(|(_, k)| *k == MdKind::ListMarker),
            "a plain number-led line is not a list: {s:?}"
        );
    }

    #[test]
    fn list_item_detects_unordered_depth_and_marker() {
        // Top-level bullet: no indent, depth 0, unordered, content after "- ".
        let it = list_item("- item").expect("a bullet is a list item");
        assert_eq!(it.indent, 0);
        assert_eq!(it.depth(), 0);
        assert!(!it.ordered);
        assert_eq!(it.content, 2);
        assert!(!it.empty);
        // Nesting is by leading spaces, 2 per level: 2 -> depth 1, 4 -> depth 2.
        assert_eq!(list_item("  * nested").unwrap().depth(), 1);
        assert_eq!(list_item("    + deep").unwrap().depth(), 2);
        // Any of -,*,+ counts (the glyph is depth-derived, not char-derived).
        assert!(!list_item("+ plus").unwrap().ordered);
    }

    #[test]
    fn list_item_detects_ordered_and_empty_and_rejects_non_lists() {
        let it = list_item("1. first").expect("ordered item");
        assert!(it.ordered);
        assert_eq!(it.depth(), 0);
        assert_eq!(list_item("  12) two").unwrap().depth(), 1);
        // An empty item (marker only) is flagged so Enter can END the list.
        assert!(list_item("- ").unwrap().empty);
        assert!(list_item("  1. ").unwrap().empty);
        // Non-lists: a bare dash (no space), a plain number, and prose.
        assert!(list_item("-nope").is_none());
        assert!(list_item("12 monkeys").is_none());
        assert!(list_item("just prose").is_none());
        assert!(list_item("").is_none());
    }

    #[test]
    fn bullet_glyph_cycles_by_depth() {
        // depth 0 -> •, 1 -> ◦, 2 -> ▪, then the cycle repeats every three levels.
        assert_eq!(bullet_for_depth(0), '•');
        assert_eq!(bullet_for_depth(1), '◦');
        assert_eq!(bullet_for_depth(2), '▪');
        assert_eq!(bullet_for_depth(3), '•');
        assert_eq!(bullet_for_depth(4), '◦');
        assert_eq!(bullet_for_depth(5), '▪');
        assert_eq!(LIST_INDENT, 2, "one nesting level is two spaces");
    }

    #[test]
    fn fenced_and_indented_code_block_body_is_code() {
        // A fenced block dims the WHOLE range as Markup (fences + info), then the
        // body Text overrides to mono Code with HIGHEST priority.
        let s = spans("```\nlet x=1;\n```");
        assert!(has(&s, 0, 16, MdKind::Markup), "whole fenced block dim: {s:?}");
        assert!(has(&s, 4, 13, MdKind::Code), "fenced body is Code: {s:?}");
        // An INDENTED (no-fence) code block: the body (range excludes the 4-space
        // indent) is both the whole-block Markup and the Code body.
        let s = spans("    code\n");
        assert!(has(&s, 4, 9, MdKind::Code), "indented body is Code: {s:?}");
    }

    #[test]
    fn rust_tagged_fence_highlights_body_and_dims_markers() {
        use crate::syntax::{Lang, SynKind};
        // ```rust\n// c\nlet s="x";\n```
        //  bytes: fence+info "```rust" 0..7, body "// c\n" 8..13, `let s="x";\n` 13..24,
        //  closing "```" 24..27.
        let doc = "```rust\n// c\nlet s=\"x\";\n```";
        let s = spans(doc);
        // The fenced body's comment + string literal carry the Alabaster ROLE spans
        // (in the fence's language), translated into DOCUMENT byte offsets.
        assert!(
            has(&s, 8, 12, MdKind::CodeSyntax { role: SynKind::Comment, lang: Lang::Rust }),
            "'// c' is a rust comment role span: {s:?}"
        );
        assert!(
            has(&s, 19, 22, MdKind::CodeSyntax { role: SynKind::Str, lang: Lang::Rust }),
            "'\"x\"' is a rust string role span: {s:?}"
        );
        // The fence markers + the info string ("rust") stay dim Markup — the whole
        // block is dimmed first and NO role span ever falls on the info-string bytes.
        assert!(
            s.iter().any(|(r, k)| *k == MdKind::Markup && r.start <= 3 && r.end >= 7),
            "the info string 'rust' stays markup: {s:?}"
        );
        assert!(
            !s.iter().any(|(r, k)| matches!(k, MdKind::CodeSyntax { .. }) && r.start < 8),
            "no role span may touch the fence/info bytes before the body: {s:?}"
        );
    }

    #[test]
    fn sh_tagged_fence_maps_to_bash_and_highlights_comment() {
        use crate::syntax::{Lang, SynKind};
        // ```sh\n# hi\n``` — the `sh` info string maps to the Bash lexer.
        let s = spans("```sh\n# hi\n```");
        assert!(
            has(&s, 6, 10, MdKind::CodeSyntax { role: SynKind::Comment, lang: Lang::Bash }),
            "'# hi' is a bash comment role span: {s:?}"
        );
    }

    #[test]
    fn unknown_and_no_lang_and_indented_fences_stay_plain_code() {
        // An UNKNOWN language: body stays plain mono Code, no role spans.
        let s = spans("```plaintext\n// c\n```");
        assert!(
            !s.iter().any(|(_, k)| matches!(k, MdKind::CodeSyntax { .. })),
            "an unknown-lang fence must not highlight: {s:?}"
        );
        assert!(s.iter().any(|(_, k)| *k == MdKind::Code), "body is still Code: {s:?}");
        // A NO-LANG bare fence: same — plain Code, no role spans.
        let s = spans("```\n// c\n```");
        assert!(
            !s.iter().any(|(_, k)| matches!(k, MdKind::CodeSyntax { .. })),
            "a no-lang fence must not highlight: {s:?}"
        );
        // An INDENTED code block: no info string at all, so no role spans.
        let s = spans("    // c\n");
        assert!(
            !s.iter().any(|(_, k)| matches!(k, MdKind::CodeSyntax { .. })),
            "an indented block must not highlight: {s:?}"
        );
    }

    #[test]
    fn non_fence_markdown_emits_no_code_syntax() {
        // Prose, headings, emphasis, inline code — none of these produce a fence
        // syntax span, so a non-fence markdown buffer stays byte-identical.
        let s = spans("# Title\n\nsome **bold** and `inline` words\n");
        assert!(
            !s.iter().any(|(_, k)| matches!(k, MdKind::CodeSyntax { .. })),
            "non-fence markdown must not emit CodeSyntax: {s:?}"
        );
    }

    #[test]
    fn plain_prose_has_no_spans() {
        assert!(spans("just some words").is_empty());
    }

    #[test]
    fn open_task_marks_box_not_text() {
        // "- [ ] buy milk": '- ' is the list marker, '[ ] ' the open checkbox, and
        // the body text rides the DEFAULT ink (no span) so an open task stays present.
        let s = spans("- [ ] buy milk");
        assert!(has(&s, 0, 2, MdKind::ListMarker), "'- ' list marker: {s:?}");
        assert!(has(&s, 2, 6, MdKind::Task(false)), "'[ ] ' open checkbox: {s:?}");
        assert!(
            !s.iter().any(|(_, k)| *k == MdKind::TaskDone),
            "an OPEN task must not dim its body: {s:?}"
        );
    }

    #[test]
    fn checked_task_dims_box_and_text() {
        // "- [x] done thing": the checkbox is a CHECKED task marker and the body
        // text dims (TaskDone) so the whole line recedes like a struck todo.
        let s = spans("- [x] done thing");
        assert!(has(&s, 2, 6, MdKind::Task(true)), "'[x] ' checked checkbox: {s:?}");
        assert!(has(&s, 6, 16, MdKind::TaskDone), "checked body dims: {s:?}");
    }

    #[test]
    fn task_done_does_not_leak_to_next_item() {
        // A checked item followed by an OPEN one: only the first item's body dims.
        let s = spans("- [x] closed\n- [ ] open");
        assert!(s.iter().any(|(_, k)| *k == MdKind::TaskDone), "first dims: {s:?}");
        assert_eq!(
            s.iter().filter(|(_, k)| *k == MdKind::TaskDone).count(),
            1,
            "the open sibling must NOT dim: {s:?}"
        );
    }

    #[test]
    fn thematic_break_is_a_rule_span() {
        // A `---` alone on a line (blank lines around it) is a thematic break; the
        // Rule span covers the line (the renderer draws the rule quad over it).
        let s = spans("a\n\n---\n\nb");
        assert!(
            s.iter().any(|(r, k)| *k == MdKind::Rule && r.start == 3),
            "--- should yield a Rule span at byte 3: {s:?}"
        );
        // `***` and `___` are rules too.
        assert!(spans("\n***\n").iter().any(|(_, k)| *k == MdKind::Rule));
        assert!(spans("\n___\n").iter().any(|(_, k)| *k == MdKind::Rule));
    }

    #[test]
    fn break_kind_tracks_the_syntax_and_maps_to_default_ornaments() {
        use crate::theme::ORNAMENTS_DEFAULT;
        // The three thematic-break spellings classify by their run character — incl.
        // the CommonMark 3+ / spaced / indented forms.
        assert_eq!(break_kind("---"), BreakKind::Dash);
        assert_eq!(break_kind("***"), BreakKind::Star);
        assert_eq!(break_kind("___"), BreakKind::Underscore);
        assert_eq!(break_kind("- - -"), BreakKind::Dash);
        assert_eq!(break_kind("  * * *"), BreakKind::Star);
        assert_eq!(break_kind("_____"), BreakKind::Underscore);
        // …and each default-world ornament is the expressive glyph for that syntax:
        // `---` → ❧ fleuron, `***` → ⁂ asterism (three stars), `___` → ❦ floral heart.
        assert_eq!(ORNAMENTS_DEFAULT.pick(BreakKind::Dash), '❧');
        assert_eq!(ORNAMENTS_DEFAULT.pick(BreakKind::Star), '⁂');
        assert_eq!(ORNAMENTS_DEFAULT.pick(BreakKind::Underscore), '❦');
    }

    #[test]
    fn setext_underline_is_not_a_rule() {
        // "Title\n---" is a setext H2 underline, NOT a thematic break — spans() must
        // not emit a Rule there (the heading is the authority, not the bare scan).
        let s = spans("Title\n---");
        assert!(
            !s.iter().any(|(_, k)| *k == MdKind::Rule),
            "a setext underline must not be a rule: {s:?}"
        );
    }

    #[test]
    fn word_count_and_reading_time() {
        assert_eq!(word_count(""), 0);
        assert_eq!(word_count("   \n  "), 0);
        assert_eq!(word_count("one two three"), 3);
        assert_eq!(word_count("line one\nline two\n"), 4);
        // Reading time rounds UP and floors at 1 min for any prose; 0 for empty.
        assert_eq!(reading_time_min(0), 0);
        assert_eq!(reading_time_min(1), 1);
        assert_eq!(reading_time_min(READING_WPM), 1);
        assert_eq!(reading_time_min(READING_WPM + 1), 2);
        assert_eq!(reading_time_min(READING_WPM * 3), 3);
    }

    #[test]
    fn tag_maps_deep_heading_levels() {
        // The sidecar wire tags for h4/h5/h6, plus the `_` catch-all that collapses
        // any level past 6 to "h6".
        assert_eq!(MdKind::Heading(4).tag(), "h4");
        assert_eq!(MdKind::Heading(5).tag(), "h5");
        assert_eq!(MdKind::Heading(6).tag(), "h6");
        assert_eq!(MdKind::Heading(9).tag(), "h6");
    }

    #[test]
    fn heading_scale_has_three_sizes_then_flattens() {
        // The size ladder's named rungs: body 1.0 / subhead 1.25 / section 1.5 /
        // title 1.8. h3 was nudged 1.3 -> 1.25 to ease the steps down the ladder.
        assert_eq!(heading_scale(0), type_scale::BODY, "no hash => body size");
        assert_eq!(heading_scale(0), 1.0, "body rung is 1.0");
        assert_eq!(heading_scale(1), type_scale::TITLE, "h1 => title");
        assert_eq!(heading_scale(1), 1.8, "title rung is 1.8");
        assert_eq!(heading_scale(2), type_scale::SECTION, "h2 => section");
        assert_eq!(heading_scale(2), 1.5, "section rung is 1.5");
        assert_eq!(heading_scale(3), type_scale::SUBHEAD, "h3 => subhead");
        assert_eq!(heading_scale(3), 1.25, "h3 nudged to the 1.25 subhead rung");
        // Strict ladder ordering, and 4+ hashes share the h3 (subhead) size.
        assert!(heading_scale(1) > heading_scale(2), "h1 > h2");
        assert!(heading_scale(2) > heading_scale(3), "h2 > h3");
        assert!(heading_scale(3) > 1.0, "h3 still bigger than body");
        assert_eq!(heading_scale(4), heading_scale(3), "4+ hashes == h3");
        assert_eq!(heading_scale(9), heading_scale(3), "deep counts clamp to h3");
        // The label rung sits BELOW body (for the future gutter/stats, faint ink).
        assert_eq!(type_scale::LABEL, 0.8, "label rung is 0.8");
        assert!(type_scale::LABEL < type_scale::BODY, "label reads smaller than body");
    }
}
