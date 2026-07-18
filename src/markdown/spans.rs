//! The per-span markdown model: [`MdKind`] (one styled span kind) and
//! [`spans`] (the `pulldown-cmark`-driven parser that turns a document into
//! a flat list of `(byte-range, MdKind)` spans the renderer lays as
//! per-span `Attrs`). Split out of the former `markdown.rs` monolith
//! (2026-07 code-organization pass); every item's path is unchanged
//! (`markdown::MdKind`, `markdown::spans`, …) -- only the file it lives in
//! moved.

use super::inline_images_on;
use super::tables::push_table_markup;
use super::ConcealKind;
use std::ops::Range;

/// One styled span kind. Maps (in `render.rs`) to a concrete `Attrs` transform
/// over the base document attrs. `Markup` is the recede-to-dim role shared by
/// every syntax character; the rest style content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MdKind {
    /// Syntax characters that recede to the DIM ink (`#`, `*`/`_`, backticks,
    /// `>`, fences, link brackets + URL). Still present + editable, just quiet.
    /// NOT WYSIWYG-concealable — see [`ConcealMarkup`](MdKind::ConcealMarkup) for
    /// the markup kinds that DO hide off the caret's line/block. `Markup` still
    /// covers a link's brackets + URL (which additionally carry their own
    /// [`ConcealMarkup`](MdKind::ConcealMarkup)`(`[`ConcealKind::Link`]`)` span)
    /// and an INDENTED (no-fence) code block's whole range — the indented block
    /// has no fence to hide behind a panel affordance, so it keeps the plain,
    /// non-concealing `Markup`. (The blockquote `>` marker is NO LONGER plain
    /// `Markup`: it conceals off-caret via [`ConcealKind::Blockquote`] now — the
    /// pull-quote round.)
    Markup,
    /// A WYSIWYG-concealable markup span — same DIM styling as [`MdKind::Markup`]
    /// (see `md_attrs`), but additionally hidden (transparent ink) per the
    /// reveal-on-cursor rule "if the caret is on that line, show the actual
    /// markdown; otherwise show the preview" (the PHILOSOPHY.md WYSIWYG
    /// amendment) — see [`ConcealKind`] for exactly which scope reveals which
    /// kind, and `render::spans::add_wysiwyg_conceal_spans` for the mechanism
    /// (mirrors the pre-existing `Rule`/bullet-marker conceal, generalized).
    /// Gated on `wysiwyg_on()`: OFF, this renders EXACTLY like plain `Markup`
    /// (dim, never concealed) — the sidecar tag is `"markup"` for both, so
    /// `md_spans` stays unchanged; the WYSIWYG state is reported separately.
    ConcealMarkup(ConcealKind),
    /// A heading's CONTENT text. Drives a larger font SIZE per [`heading_scale`]
    /// (applied per-line in `render.rs`) — no bold/color by DESIGN call: size + value
    /// carry the hierarchy on their own. (Inline [`Bold`](Self::Bold) does shape real
    /// bold on every world now; a heading just doesn't spend it.)
    Heading(u8),
    /// `**bold**` / `__bold__` content → Bold weight. Resolves to the world's real
    /// bundled 700 face on EVERY world — proportional and mono alike
    /// (`render::FONT_THEME_BOLD_FACES`).
    Bold,
    /// `*italic*` / `_italic_` content → Italic style.
    Italic,
    /// `***both***` content → Bold + Italic.
    BoldItalic,
    /// Inline `` `code` `` + fenced/indented code-block body → mono family + tint.
    /// `inline` distinguishes the two for the WYSIWYG wash: an INLINE span
    /// (`inline: true`) gets a small background PILL (see
    /// `render::rects::ensure_code_pill_protos`); a BLOCK body (`inline: false`,
    /// fenced or indented) does not — a fenced block instead gets the whole-block
    /// PANEL (from its [`ConcealKind::Fence`] span), and an indented block gets
    /// neither. The sidecar tag is `"code"` for both (unchanged).
    Code { inline: bool },
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
    /// `==highlight==` content (the de-facto Obsidian/Typora/iA convention — NOT
    /// CommonMark, which has no `==` construct at all). Rendered as a highlighter
    /// stroke: the warm comment-wash quad BEHIND full content ink (reusing the
    /// existing wash pipeline — see `rects.rs::ensure_wash_protos`), never a color
    /// change on the text itself (no-op transform in `md_attrs`, like `Heading`).
    /// The `==` delimiters are separate `Markup` spans (dim, like every other
    /// syntax character). See [`push_highlight_spans`] for the delimiter rules
    /// (single `=` is deliberately meaningless; only an ISOLATED `==` pair counts).
    Highlight,
    /// `~~struck~~` content (GFM strikethrough, `ENABLE_STRIKETHROUGH`, gated to
    /// EXACTLY-two-tilde delimiters — a single `~x~`, which pulldown also accepts,
    /// stays inert, mirroring the `==` exactly-two rule). Struck text RECEDES: the
    /// content takes the muted strike ink (see `render::spans::strike_ink`, the one
    /// owner the drawn LINE shares) and the renderer draws a thin STRIKE LINE
    /// through the run (`render::rects` strike bucket → `strike_lines`, positioned
    /// by `render::spans::strike_line_band` — the SAME one owner the format
    /// popover's self-demonstrating `S` button rides). Never amber (DESIGN §3).
    /// The `~~` delimiters are separate [`ConcealMarkup`](Self::ConcealMarkup)
    /// spans ([`ConcealKind::Strikethrough`], line-scoped like Emphasis). Pushed
    /// ADDITIVELY over the context span (like [`Highlight`](Self::Highlight)), so
    /// struck text inside a heading/quote/bold run still dims + strikes.
    Strikethrough,
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
    /// A GitHub-flavored TABLE's cell-delimiter `|` pipe → dim `Markup` styling.
    /// awl is a SOURCE editor: a table renders as styled SOURCE (the structural
    /// `|` recedes to the muted ink like every other syntax character), NEVER a
    /// drawn grid widget. One span per literal `|` within a table's byte range
    /// (see [`push_table_markup`]); the sidecar tag is `"table_pipe"`.
    TablePipe,
    /// A GFM table's HEADER-SEPARATOR row (`|---|:--:|---|`) — the whole `-`/`:`/`|`
    /// run on that one line → dim `Markup` styling. pulldown emits no event for the
    /// separator row at all, so [`push_table_markup`] identifies it by shape (a line
    /// of only `|-: \t` containing a `-`). Sidecar tag `"table_sep"`.
    TableSep,
    /// A GFM table HEADER cell's CONTENT (the text between the first row's pipes) →
    /// a no-op transform in `md_attrs` (full CONTENT ink, exactly like [`Heading`](Self::Heading)
    /// / [`Highlight`](Self::Highlight) — NO amber, NO new accent; header vs body is
    /// the "safe minimum" value-only treatment). Emitted only so a header cell is
    /// distinguishable in the sidecar (`"table_header"`); it does not change pixels
    /// (body cells ride the same full default ink with no span).
    TableHeader,
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

/// True when `line` is a CommonMark THEMATIC BREAK: after up to 3 leading spaces, a
/// run of THREE-OR-MORE matching `-`, `_`, or `*`, separated/surrounded only by
/// spaces or tabs, and nothing else. This is the bare-text heuristic
/// [`crate::render::spans::md_line_scale`] uses to grow a break line's row to fit the
/// bigger ornament glyph — sized by the active world's
/// [`crate::theme::Theme::ornament_scale`] — the size counterpart of the leading-`#`
/// heading scan (a per-line grow that never needs the whole parse). Pure + total.
///
/// KNOWN, ACCEPTED false positive (documented, matching the existing setext-heading
/// gap): a `---` that pulldown actually rules a SETEXT-heading underline (a `---`
/// directly under a paragraph line) is indistinguishable from a thematic break at
/// the single-line level, so its row grows too — even though no ornament draws over
/// it (the ornament layer reads the real `md_spans`, not this scan). `***`/`___`
/// are never setext, so only a dash underline is affected; awl's own docs use ATX
/// headings, so this is rare in practice.
pub fn is_thematic_break(line: &str) -> bool {
    let t = line.trim_matches(|c| c == ' ' || c == '\t');
    // The run char is the first non-space glyph; every non-space char must match it,
    // and there must be at least three of them.
    let mut run_char: Option<char> = None;
    let mut count = 0usize;
    for ch in t.chars() {
        match ch {
            ' ' | '\t' => {}
            '-' | '_' | '*' => {
                match run_char {
                    None => run_char = Some(ch),
                    Some(rc) if rc == ch => {}
                    Some(_) => return false, // mixed run chars => not a break
                }
                count += 1;
            }
            _ => return false, // any other glyph disqualifies the line
        }
    }
    count >= 3
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
            // A WYSIWYG-concealable markup span reports the SAME "markup" tag as
            // plain Markup — `md_spans` is unchanged by this round; the conceal
            // STATE (not the kind) is what's new, reported separately (see
            // `render::TextPipeline::wysiwyg_report`).
            MdKind::Markup | MdKind::ConcealMarkup(_) => "markup",
            MdKind::Heading(1) => "h1",
            MdKind::Heading(2) => "h2",
            MdKind::Heading(3) => "h3",
            MdKind::Heading(4) => "h4",
            MdKind::Heading(5) => "h5",
            MdKind::Heading(_) => "h6",
            MdKind::Bold => "bold",
            MdKind::Italic => "italic",
            MdKind::BoldItalic => "bold_italic",
            MdKind::Code { .. } => "code",
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
            MdKind::Highlight => "highlight",
            MdKind::Strikethrough => "strikethrough",
            MdKind::Rule => "rule",
            MdKind::TablePipe => "table_pipe",
            MdKind::TableSep => "table_sep",
            MdKind::TableHeader => "table_header",
        }
    }

    /// True for the three GFM-table structural span kinds ([`MdKind::TablePipe`],
    /// [`MdKind::TableSep`], [`MdKind::TableHeader`]) — used to identify which
    /// document LINES are table rows so the double-space writing-nit is exempted on
    /// them (column alignment like `| Name  | Value |` is intentional, not a slip).
    /// See `render::rects::ensure_nit_protos`.
    pub fn is_table_markup(self) -> bool {
        matches!(self, MdKind::TablePipe | MdKind::TableSep | MdKind::TableHeader)
    }
}

/// The document's FRONTMATTER block END byte, if `md_spans` carries a
/// `ConcealMarkup(Frontmatter)` span (always spanning byte `0..end`) — the
/// SHARED exclusion point for word-count/reading-time
/// ([`render::chrome::TextPipeline::word_count`](crate::render::TextPipeline)),
/// writing-nits (`render/rects.rs::ensure_nit_protos`), and — indirectly, via
/// its own [`crate::frontmatter::detect`] call — spell-check
/// (`spell::SpellChecker::misspellings_for`). `None` when the document has no
/// frontmatter block (the exclusion is then a no-op everywhere it's used).
pub fn frontmatter_end(md_spans: &[(Range<usize>, MdKind)]) -> Option<usize> {
    md_spans.iter().find_map(|(r, k)| {
        matches!(k, MdKind::ConcealMarkup(ConcealKind::Frontmatter)).then_some(r.end)
    })
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

/// THE strikethrough ENGAGEMENT gate — awl's exactly-two-tilde rule, the ONE
/// place BOTH the live renderer ([`spans`]) and every exporter
/// ([`crate::export`]'s `model::parse`) decide whether a pulldown
/// `Tag::Strikethrough` span actually strikes. pulldown's GFM option ALSO parses
/// single-tilde `~x~`; awl deliberately keeps that INERT (the `==` exactly-two
/// precedent — the format command and the writer's-diff serializer both emit
/// `~~`), so prose like `2~3 weeks` or a stray `~word~` never silently strikes.
/// `src` is the whole `~~…~~` span slice (from the offset iterator); ENGAGED iff
/// its LEADING tilde run is EXACTLY two (GFM guarantees the closing run matches).
/// A `~~~` run is a block-level FENCE and never reaches an inline strikethrough
/// tag, so it can't reach this gate. Pure + total. Sharing this ONE owner is what
/// keeps the RENDER (inert `~x~` → no strike span) and the EXPORT (inert `~x~` →
/// no `<del>`/`<w:strike/>`) from disagreeing — the render/export strike
/// divergence this fixed.
pub fn strike_engaged(src: &str) -> bool {
    src.bytes().take_while(|&b| b == b'~').count() == 2
}

/// Parse `text` into styling spans in DOCUMENT byte coordinates. Spans may
/// overlap by DESIGN: a link or code-block first pushes a whole-range `Markup`
/// span, then its inner text pushes a `LinkText`/`Code` span — applied in this
/// order, the later (inner) span wins for its bytes while the brackets/URL/fence
/// keep the dim `Markup`. The renderer adds them to the `AttrsList` in THIS
/// order, relying on cosmic-text's "last span wins on overlap" semantics.
///
/// A leading FRONTMATTER block ([`crate::frontmatter::detect`]) is carved off
/// FIRST: its whole range becomes one `ConcealMarkup(Frontmatter)` span, and
/// the REST of the document (past the block) is what pulldown actually parses
/// — so a frontmatter block's `key: value` lines never confuse the markdown
/// parser (no stray thematic-break/setext-heading reads), and every span this
/// function would otherwise emit is simply offset by the block's byte length.
/// A document with no (or no well-formed) frontmatter block parses exactly as
/// before, byte-identically.
pub fn spans(text: &str) -> Vec<(Range<usize>, MdKind)> {
    use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

    let mut out: Vec<(Range<usize>, MdKind)> = Vec::new();
    let (text, body_offset) = match crate::frontmatter::detect(text) {
        Some(fm) => {
            out.push((0..fm.range.end, MdKind::ConcealMarkup(ConcealKind::Frontmatter)));
            (&text[fm.range.end..], fm.range.end)
        }
        None => (text, 0),
    };
    let mut body: Vec<(Range<usize>, MdKind)> = Vec::new();
    // Nesting depth / context flags. Headings don't nest, so a single level is
    // enough; the emphasis/quote/link/code contexts use counters so a nested
    // construct restores the outer context on close.
    let mut heading: Option<u8> = None;
    let mut strong = 0u32;
    let mut emph = 0u32;
    let mut quote = 0u32;
    let mut link = 0u32;
    let mut code_block = 0u32;
    // STRIKETHROUGH nesting. `strike` counts only ENGAGED (exactly-two-tilde)
    // spans; pulldown also parses single-tilde `~x~` with the option on, which
    // awl deliberately keeps INERT (the `==` exactly-two precedent — the format
    // command inserts `~~`, so only `~~` means struck). `strike_engaged` is a
    // tiny per-Start stack so a skipped single-tilde span's `TagEnd` never
    // decrements a counter it didn't increment.
    let mut strike = 0u32;
    let mut strike_engaged_stack: Vec<bool> = Vec::new();
    // IMAGE nesting depth. An image's `![alt](path)` source is emitted as ONE
    // `ConcealMarkup(Image)` span over its whole range (see the `Tag::Image`
    // arm); while inside one, the inner alt-text `Event::Text` is SUPPRESSED
    // (the whole ref is concealed off-cursor and reveals as raw source
    // on-cursor, so a per-run styling span on the alt would be dead weight and
    // could mis-highlight an `==`/emphasis run in the alt). Gated on
    // `inline_images_on()` — off/wasm, no image span is pushed and the source
    // stays plain default-ink text (byte-identical to the pre-feature editor).
    let mut image = 0u32;
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
    // TABLE: true while inside a `TableHead` (its cells get the `TableHeader` tag).
    // The pipes + separator row are emitted up-front from the whole table range on
    // `Tag::Table` (pulldown emits no event for either), so no per-row bookkeeping is
    // needed beyond this one header flag.
    let mut in_table_head = false;

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
    // ENABLE_STRIKETHROUGH so `~~struck~~` surfaces as `Tag::Strikethrough`
    // (matching the export model's own option set — `export/model.rs` already
    // parsed it; the RENDER now catches up). Every other construct parses
    // exactly as before (the options are additive).
    let opts =
        Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    for (ev, range) in Parser::new_ext(text, opts).into_offset_iter() {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    heading = Some(level_u8(level));
                    push_heading_markers(&mut body, text, &range);
                }
                Tag::Strong => {
                    strong += 1;
                    push_delim(&mut body, &range, 2, ConcealKind::Emphasis);
                }
                Tag::Emphasis => {
                    emph += 1;
                    push_delim(&mut body, &range, 1, ConcealKind::Emphasis);
                }
                // STRIKETHROUGH: engage ONLY for exactly-two-tilde delimiters
                // (`~~struck~~`). pulldown's GFM option also parses single-tilde
                // `~x~`; awl keeps that inert (no marker span, no content span,
                // no strike line — the bytes render as plain text), mirroring the
                // `==` exactly-two rule. A `~~~` run is a FENCE at block level and
                // never reaches this inline arm.
                Tag::Strikethrough => {
                    // THE shared exactly-two-tilde gate (`strike_engaged`) — the
                    // export's `model::parse` reads the SAME owner, so a single
                    // `~x~` stays inert in both the render and every export.
                    let engaged = strike_engaged(&text[range.clone()]);
                    strike_engaged_stack.push(engaged);
                    if engaged {
                        strike += 1;
                        push_delim(&mut body, &range, 2, ConcealKind::Strikethrough);
                    }
                }
                Tag::BlockQuote(_) => {
                    quote += 1;
                    push_quote_markers(&mut body, text, &range);
                }
                Tag::CodeBlock(kind) => {
                    code_block += 1;
                    // Dim the WHOLE block (fences + info string); the body Text
                    // events below override their bytes to mono `Code`. A FENCED
                    // block's whole-range span is WYSIWYG-concealable (`Fence`) —
                    // its marker lines hide behind the always-present panel unless
                    // the caret sits inside the block; an INDENTED block has no
                    // fence to hide behind a panel, so it keeps the plain,
                    // non-concealing `Markup` (byte-identical to before this round).
                    let fenced = matches!(kind, CodeBlockKind::Fenced(_));
                    body.push((
                        range.clone(),
                        if fenced {
                            MdKind::ConcealMarkup(ConcealKind::Fence)
                        } else {
                            MdKind::Markup
                        },
                    ));
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
                    // Conceal the `[` + `](url)` PLUMBING (WYSIWYG `Link`, line-
                    // scoped) while the inner Text pushes a `LinkText` span over the
                    // visible text (full content ink). Off the caret's line the
                    // plumbing hides to zero-width and only the text shows; on the
                    // line the whole `[text](url)` reveals for editing. A reference /
                    // malformed link with no `](` falls back to a plain dim `Markup`.
                    push_link_markers(&mut body, text, &range);
                }
                // IMAGE: the whole `![alt](path)` reference. Emitted as one
                // WYSIWYG-concealable span (line-scoped) so its source hides off
                // the caret's line while the decoded image draws in the tall row
                // the renderer reserves (the draw + the path/hint payload are
                // read back from this span's byte range — see
                // `render::TextPipeline::rebuild_image_rows`). Only when inline
                // images are ON (native + enabled): off/wasm pushes nothing, so
                // the source renders as plain text exactly as before this round.
                Tag::Image { .. } => {
                    // Only engage (span + alt-text suppression) when images are
                    // ON: off/wasm leaves `image` at 0 so the alt text flows
                    // through the ordinary Text path, byte-identical to before.
                    if inline_images_on() {
                        image += 1;
                        body.push((range.clone(), MdKind::ConcealMarkup(ConcealKind::Image)));
                    }
                }
                Tag::Item => push_list_marker(&mut body, text, &range),
                // TABLE: dim the structural markup (the `|` pipes on every row + the
                // whole `|---|` separator row) up-front from the table's byte range —
                // pulldown emits no event for either. Rendered as styled SOURCE, never
                // a drawn grid (awl is a source editor).
                Tag::Table(_) => push_table_markup(&mut body, text, &range),
                Tag::TableHead => in_table_head = true,
                // A HEADER cell's content (between the header row's pipes) gets the
                // `TableHeader` tag — a no-op full-ink transform (see `md_attrs`), so
                // it's only distinguishable in the sidecar, never in pixels.
                Tag::TableCell if in_table_head => {
                    body.push((range.clone(), MdKind::TableHeader));
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) => heading = None,
                TagEnd::Strong => strong = strong.saturating_sub(1),
                TagEnd::Emphasis => emph = emph.saturating_sub(1),
                TagEnd::Strikethrough => {
                    if strike_engaged_stack.pop().unwrap_or(false) {
                        strike = strike.saturating_sub(1);
                    }
                }
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
                                body.push((bs + r.start..bs + r.end, MdKind::CodeSyntax { role, lang }));
                            }
                        }
                    }
                }
                TagEnd::Link => link = link.saturating_sub(1),
                TagEnd::Image => image = image.saturating_sub(1),
                TagEnd::Item => task_done = false,
                TagEnd::TableHead => in_table_head = false,
                _ => {}
            },
            // A thematic break (`---`/`***`/`___` alone on a line): mark the literal
            // characters as a Rule; the renderer drops a centered fleuron on the row
            // and conceals the dashes unless the caret is editing the line.
            Event::Rule => body.push((range, MdKind::Rule)),
            // The `[ ]`/`[x]` checkbox. Style the marker (+ its trailing space)
            // distinctly; a CHECKED box also dims the item's body text.
            Event::TaskListMarker(checked) => {
                task_done = checked;
                push_task_marker(&mut body, text, &range, checked);
            }
            // Inside an IMAGE, the alt-text `Event::Text` is swallowed: the whole
            // `![alt](path)` is one concealable span already, so no per-run alt
            // styling is wanted (and an `==`/`*` in the alt must not highlight).
            Event::Text(_) if image > 0 => {}
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
                    body.push((range.clone(), k));
                }
                // STRIKETHROUGH: pushed ADDITIVELY over the context span above
                // (the `Highlight` precedent, but receding instead of lifting) —
                // last-wins on overlap means struck text inside a heading / quote
                // / bold / link run still takes the muted strike ink, and the
                // strike-line bucket (`render::rects`) covers exactly these
                // bytes. Never inside a code block (pulldown treats `~~` in code
                // as literal, so `strike` can't be armed there).
                if strike > 0 {
                    body.push((range.clone(), MdKind::Strikethrough));
                }
                // HIGHLIGHT: scan this text run for `==marked==` pairs, pushed
                // AFTER the context span above so a highlighted sub-range always
                // lifts back to the full content ink (mirrors `LinkText` lifting
                // off the whole-range `Markup`). Skipped inside a code block body
                // (fenced or indented) — `==` inside code is never a highlight;
                // inline code never reaches here at all (it arrives via
                // `Event::Code`, not `Event::Text`).
                if code_block == 0 {
                    push_highlight_spans(&mut body, text, &range);
                }
            }
            Event::Code(_) => push_inline_code(&mut body, text, &range),
            _ => {}
        }
    }
    // Shift every body-relative span back into DOCUMENT byte coordinates (a
    // no-op add of 0 when there was no frontmatter block) and append after the
    // frontmatter span pushed above.
    out.extend(body.into_iter().map(|(r, k)| (r.start + body_offset..r.end + body_offset, k)));
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
        Some(MdKind::Code { inline: false })
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

/// Dim the `n`-byte inline delimiters at each end of `range` (`*`/`_` → n=1,
/// `**`/`__`/`~~` → n=2). No-op if the range is too short to hold both. WYSIWYG-
/// concealable as `ck` ([`ConcealKind::Emphasis`] for bold/italic,
/// [`ConcealKind::Strikethrough`] for `~~`): the delimiters hide off the caret's
/// line, leaving the styled content alone.
fn push_delim(out: &mut Vec<(Range<usize>, MdKind)>, range: &Range<usize>, n: usize, ck: ConcealKind) {
    if range.end.saturating_sub(range.start) >= 2 * n {
        let k = MdKind::ConcealMarkup(ck);
        out.push((range.start..range.start + n, k));
        out.push((range.end - n..range.end, k));
    }
}

/// Dim a heading's leading `#`s (+ the space after), and any ATX closing `#`s.
/// WYSIWYG-concealable ([`ConcealKind::Heading`]): both marker runs hide off the
/// caret's line, leaving the sized title alone.
fn push_heading_markers(out: &mut Vec<(Range<usize>, MdKind)>, text: &str, range: &Range<usize>) {
    let k = MdKind::ConcealMarkup(ConcealKind::Heading);
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
        out.push((range.start..range.start + j, k));
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
        out.push((range.start + s0..range.start + e, k));
    }
}

/// Conceal a link's MARKUP plumbing — the opening `[` and the whole `](url)`
/// tail — as WYSIWYG-concealable [`ConcealKind::Link`] spans, leaving the visible
/// link TEXT untouched (the inner `Event::Text` styles it `LinkText`, full content
/// ink). `range` is the whole `[text](url)` reference. The text/plumbing split is
/// the FIRST `](` in the source: everything before it is `[text`, everything from
/// it on is the `](url…)` tail. Off the caret's line the two plumbing runs hide to
/// zero-width so only the text shows; on the line they reveal for editing.
///
/// A reference-style (`[text][ref]` / `[text]`) or otherwise malformed link has no
/// `](`, so it falls back to a single plain, NON-concealing [`MdKind::Markup`] span
/// over the whole range — byte-identical to the pre-WYSIWYG-links rendering (dim
/// brackets, content-ink text), never a mis-conceal.
fn push_link_markers(out: &mut Vec<(Range<usize>, MdKind)>, text: &str, range: &Range<usize>) {
    let s = &text[range.clone()];
    // The `](` separating the visible text from the destination. Requires the
    // source to actually open with `[` (an inline link always does).
    if s.starts_with('[') {
        if let Some(rel) = s.find("](") {
            let k = MdKind::ConcealMarkup(ConcealKind::Link);
            // Opening `[`.
            out.push((range.start..range.start + 1, k));
            // The `](url…)` tail — closing bracket, parens, destination + title.
            out.push((range.start + rel..range.end, k));
            return;
        }
    }
    // Reference / malformed: dim the whole thing, no conceal (as before).
    out.push((range.clone(), MdKind::Markup));
}

/// Dim + WYSIWYG-conceal the leading `>` quote markers (+ a following space) on
/// every line of a blockquote range, including nested `>>`. Each line's whole
/// marker run is ONE [`ConcealKind::Blockquote`] span (LINE-scoped): dim like plain
/// `Markup` with WYSIWYG off, concealed to zero-width off the caret's line with it
/// on. Nested markers on one line share that line's run, so they conceal together.
/// The block's affordance off-caret is the renderer's margin-hung pull-quote mark.
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
                out.push((
                    range.start + line_start..range.start + last,
                    MdKind::ConcealMarkup(ConcealKind::Blockquote),
                ));
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

/// Byte ranges of every ISOLATED two-`=` run in `s` — a valid `==` delimiter
/// candidate for [`push_highlight_spans`]. "Isolated" means the byte immediately
/// before AND after the pair (if any) is NOT itself `=`, so a run of exactly 1
/// (`=`), 3 (`===`), or 4+ (`====`) equals yields ZERO candidates at any offset
/// within it — every position in a longer run fails the "not `=`" check on one
/// side or the other. This single rule is what makes a bare `=` meaningless
/// (never a run of 2) and what makes an adjacent `====` inert (no candidate
/// anywhere in it) — no special-casing either edge case separately. Pure, O(n).
fn equals_runs(s: &str) -> Vec<Range<usize>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 1 < b.len() {
        if b[i] == b'=' && b[i + 1] == b'='
            && (i == 0 || b[i - 1] != b'=')
            && (i + 2 >= b.len() || b[i + 2] != b'=')
        {
            out.push(i..i + 2);
            i += 2; // consume the whole marker; never rescan its bytes
        } else {
            i += 1;
        }
    }
    out
}

/// Detect `==marked==` runs within ONE text event's range (`range`, into the
/// document `text`) and push `Markup` (the `==` delimiters) + `Highlight` (the
/// marked content) spans onto `out`. NOT CommonMark — there is no `==` construct
/// in the spec; this ships the de-facto Obsidian/Typora/iA convention.
///
/// Delimiter candidates come from [`equals_runs`] (isolated two-`=` runs only).
/// They pair up GREEDILY, consuming two consecutive candidates at a time:
/// candidate `k` opens, candidate `k+1` closes. A trailing UNPAIRED candidate (an
/// odd one out at the end of the list) is simply left as literal `=` characters —
/// the "unclosed `==`" case: no span, no panic, just plain text. A candidate pair
/// separated by a `\n` is rejected too (NO CROSS-LINE SPANS): the open is
/// discarded as literal and the rejected close is retried as a fresh open against
/// the NEXT candidate, so `a==\nb==c==` still highlights `c` from the trailing
/// pair. In practice a soft-wrapped paragraph already arrives as separate `Text`
/// events split at the line break (pulldown emits `Event::SoftBreak` between
/// them, never embedding the `\n` in a `Text` range), so this mostly guards a
/// defensive edge the parser doesn't otherwise produce — see the direct
/// [`push_highlight_spans`] unit test that constructs one by hand.
/// WYSIWYG-concealable ([`ConcealKind::Highlight`]): the `==` delimiters hide off
/// the caret's line — the wash stroke IS the affordance once they do.
pub(super) fn push_highlight_spans(out: &mut Vec<(Range<usize>, MdKind)>, text: &str, range: &Range<usize>) {
    let s = &text[range.clone()];
    let markers = equals_runs(s);
    let k_markup = MdKind::ConcealMarkup(ConcealKind::Highlight);
    let mut k = 0usize;
    while k + 1 < markers.len() {
        let open = markers[k].clone();
        let close = markers[k + 1].clone();
        if s[open.end..close.start].contains('\n') {
            k += 1; // no cross-line spans: discard `open`, retry `close` as a new open
            continue;
        }
        out.push((range.start + open.start..range.start + open.end, k_markup));
        out.push((range.start + open.end..range.start + close.start, MdKind::Highlight));
        out.push((range.start + close.start..range.start + close.end, k_markup));
        k += 2;
    }
}

/// Inline `` `code` ``: dim the matching backtick runs at each end, mono-tint the
/// inner slice. The backticks are WYSIWYG-concealable ([`ConcealKind::Code`]); the
/// content span is `MdKind::Code { inline: true }` — the renderer washes it with a
/// small pill (see `render::rects::ensure_code_pill_protos`), unlike a block body.
fn push_inline_code(out: &mut Vec<(Range<usize>, MdKind)>, text: &str, range: &Range<usize>) {
    let s = &text[range.clone()];
    let b = s.as_bytes();
    let open = b.iter().take_while(|&&c| c == b'`').count();
    let close = b.iter().rev().take_while(|&&c| c == b'`').count();
    if open == 0 || open + close > b.len() {
        // Degenerate (shouldn't happen for a Code event) — tint the whole thing.
        out.push((range.clone(), MdKind::Code { inline: true }));
        return;
    }
    let k_markup = MdKind::ConcealMarkup(ConcealKind::Code);
    out.push((range.start..range.start + open, k_markup));
    out.push((range.end - close..range.end, k_markup));
    if range.start + open < range.end - close {
        out.push((range.start + open..range.end - close, MdKind::Code { inline: true }));
    }
}
