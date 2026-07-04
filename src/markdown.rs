//! Markdown styling spans — the "dim the markup + style the content" model for
//! awl's prose docs. We parse the document with `pulldown-cmark`'s OFFSET
//! iterator (each event carries its byte range into the source text) and turn
//! the events into a flat list of `(byte-range, MdKind)` spans. The renderer
//! lays these as per-span `Attrs` over each line's `AttrsList`, exactly like the
//! CJK + focus spans — the markup characters (`#`, `*`, `` ` ``, `>`, list
//! markers, link brackets/URL, `==`) recede to the DIM ink while staying fully
//! present and editable, and the CONTENT gains structure (bold weight, italic
//! style, mono code, heading SIZE, accent link text, a highlighter wash behind
//! `==marked==` text). Headings take NO accent color and
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
//!
//! WYSIWYG (the PHILOSOPHY.md amendment): "if the caret is on that line, show the
//! actual markdown; otherwise show the preview." A settled markdown line already
//! dims its markup and styles its content; WYSIWYG goes one step further and
//! CONCEALS the markup entirely (transparent ink, same trick as the pre-existing
//! hr/bullet reveal-on-cursor) for headings, bold/italic, inline code, and
//! `==highlight==` off the caret's line, plus a fenced code block's marker lines
//! off the caret's whole BLOCK — seed [`MdKind::ConcealMarkup`] / [`ConcealKind`]
//! for which spans qualify and `render::spans::add_wysiwyg_conceal_spans` for the
//! mechanism. Gated by the sticky [`wysiwyg_on`] global (default ON; `false`
//! reproduces today's always-visible markup byte-identically) — mirrors
//! `nits::NITS_ON` / `spell::SPELLCHECK_ON` exactly: a process-global read by the
//! renderer, set once at launch from the config sticky pref (`config.rs`).

use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the WYSIWYG markup conceal is active. DEFAULT ON — the editor opens
/// with headings/emphasis/inline-code/highlight markup hiding off the caret's
/// line (and a fenced block's markers hiding off the caret's whole block); OFF
/// reproduces the always-visible markup this round shipped without, byte-for-byte
/// (no conceal, no pill, no panel — just the pre-existing dim-the-markup styling).
static WYSIWYG_ON: AtomicBool = AtomicBool::new(true);

/// True when the WYSIWYG conceal is active (read by the renderer each reshape).
pub fn wysiwyg_on() -> bool {
    WYSIWYG_ON.load(Ordering::Relaxed)
}

/// Set the WYSIWYG conceal on/off explicitly — the config sticky-pref launch-
/// apply (mirrors [`crate::nits::set_nits_on`]).
pub fn set_wysiwyg_on(on: bool) {
    WYSIWYG_ON.store(on, Ordering::Relaxed);
}

/// Serializes tests that read or write the process-global [`WYSIWYG_ON`],
/// mirroring [`crate::nits::TEST_LOCK`].
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One styled span kind. Maps (in `render.rs`) to a concrete `Attrs` transform
/// over the base document attrs. `Markup` is the recede-to-dim role shared by
/// every syntax character; the rest style content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MdKind {
    /// Syntax characters that recede to the DIM ink (`#`, `*`/`_`, backticks,
    /// `>`, fences, link brackets + URL). Still present + editable, just quiet.
    /// NOT WYSIWYG-concealable — see [`ConcealMarkup`](MdKind::ConcealMarkup) for
    /// the markup kinds that DO hide off the caret's line/block. `Markup` still
    /// covers the blockquote `>` marker, a link's brackets + URL, and an INDENTED
    /// (no-fence) code block's whole range — none of those conceal in v1 (links
    /// + quotes are OUT of WYSIWYG scope; an indented block has no fence to hide
    /// behind a panel affordance).
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

/// WHICH markdown construct a [`MdKind::ConcealMarkup`] span belongs to — the
/// WYSIWYG amendment's dispatch key ("if the caret is on that line, show the
/// actual markdown; otherwise show the preview"). Every kind but [`Fence`](Self::Fence)
/// is LINE-scoped: it reveals when the caret sits on the span's OWN line, exactly
/// mirroring the pre-existing hr/bullet reveal-on-cursor. `Fence` is BLOCK-scoped:
/// a fenced code block's marker lines reveal only when the caret is ANYWHERE
/// inside the whole block, because the PANEL (drawn from the same span's byte
/// range, always present) is the block's affordance — ducking the markers in and
/// out per LINE inside a multi-line block the caret is actively editing would
/// flicker distractingly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConcealKind {
    /// A heading's leading `#` run (+ a trailing ATX close, if any).
    Heading,
    /// A bold/italic emphasis delimiter run (`**`/`*`/`_`).
    Emphasis,
    /// An inline code span's backtick delimiters (the CONTENT keeps its own
    /// `MdKind::Code { inline: true }` span + wash pill — only the backticks hide).
    Code,
    /// A `==highlight==` delimiter pair (the wash stroke IS the affordance once
    /// the `==` marks hide — see `MdKind::Highlight`).
    Highlight,
    /// A FENCED code block's ENTIRE range — both fence lines (open + close) and
    /// the info string. The renderer only ever conceals the MARKER lines from
    /// this span (never the body, which carries its own `Code`/`CodeSyntax`
    /// spans) — see `render::spans::add_wysiwyg_conceal_spans`. An INDENTED (no
    /// fence) code block has no marker to hide behind a panel, so it keeps the
    /// plain, non-concealing [`MdKind::Markup`] instead of this kind.
    Fence,
    /// A `---`-delimited FRONTMATTER block's ENTIRE range (see
    /// [`crate::frontmatter::detect`]) — BLOCK-scoped exactly like [`Fence`]
    /// (reveals iff the caret sits anywhere inside the block), reusing the SAME
    /// seam with zero new machinery. Unlike `Fence` there is no body sub-span
    /// to carve out (a frontmatter block is entirely markup, no highlighted
    /// content), so the whole range conceals/reveals as one unit.
    Frontmatter,
}

impl ConcealKind {
    /// Stable tag string for the capture sidecar's `wysiwyg.concealed` block.
    pub fn tag(self) -> &'static str {
        match self {
            ConcealKind::Heading => "heading",
            ConcealKind::Emphasis => "emphasis",
            ConcealKind::Code => "code",
            ConcealKind::Highlight => "highlight",
            ConcealKind::Fence => "fence",
            ConcealKind::Frontmatter => "frontmatter",
        }
    }
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
            MdKind::Rule => "rule",
        }
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
                    push_heading_markers(&mut body, text, &range);
                }
                Tag::Strong => {
                    strong += 1;
                    push_delim(&mut body, &range, 2);
                }
                Tag::Emphasis => {
                    emph += 1;
                    push_delim(&mut body, &range, 1);
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
                    // Dim the whole `[text](url)`; inner Text overrides the visible
                    // text to the accent, leaving brackets + URL dim.
                    body.push((range.clone(), MdKind::Markup));
                }
                Tag::Item => push_list_marker(&mut body, text, &range),
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
                                body.push((bs + r.start..bs + r.end, MdKind::CodeSyntax { role, lang }));
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
            Event::Rule => body.push((range, MdKind::Rule)),
            // The `[ ]`/`[x]` checkbox. Style the marker (+ its trailing space)
            // distinctly; a CHECKED box also dims the item's body text.
            Event::TaskListMarker(checked) => {
                task_done = checked;
                push_task_marker(&mut body, text, &range, checked);
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
                    body.push((range.clone(), k));
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

/// Dim the `n`-byte emphasis delimiters at each end of `range` (`*`/`_` → n=1,
/// `**`/`__` → n=2). No-op if the range is too short to hold both. WYSIWYG-
/// concealable ([`ConcealKind::Emphasis`]): the delimiters hide off the caret's
/// line, leaving the bold/italic content alone.
fn push_delim(out: &mut Vec<(Range<usize>, MdKind)>, range: &Range<usize>, n: usize) {
    if range.end.saturating_sub(range.start) >= 2 * n {
        let k = MdKind::ConcealMarkup(ConcealKind::Emphasis);
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
fn push_highlight_spans(out: &mut Vec<(Range<usize>, MdKind)>, text: &str, range: &Range<usize>) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn has(spans: &[(Range<usize>, MdKind)], lo: usize, hi: usize, k: MdKind) -> bool {
        spans.iter().any(|(r, kk)| r.start == lo && r.end == hi && *kk == k)
    }

    #[test]
    fn heading_dims_hashes_and_styles_title() {
        let s = spans("# Title");
        // "# " (hash + space) is dim, WYSIWYG-concealable markup; "Title" is H1 content.
        let heading_markup = MdKind::ConcealMarkup(ConcealKind::Heading);
        assert!(has(&s, 0, 2, heading_markup), "leading '# ' should be markup: {s:?}");
        assert!(has(&s, 2, 7, MdKind::Heading(1)), "title should be h1: {s:?}");
    }

    #[test]
    fn h2_level_detected() {
        let s = spans("## Sub");
        assert!(has(&s, 0, 3, MdKind::ConcealMarkup(ConcealKind::Heading)));
        assert!(has(&s, 3, 6, MdKind::Heading(2)));
    }

    #[test]
    fn atx_closing_hashes_dim() {
        // `# Title #`: the leading `# ` AND the trailing ` #` both dim as Markup
        // (the backward close-fence scan), with `Title` the h1 content between.
        let s = spans("# Title #");
        let heading_markup = MdKind::ConcealMarkup(ConcealKind::Heading);
        assert!(has(&s, 0, 2, heading_markup), "leading '# ' dim: {s:?}");
        assert!(has(&s, 2, 7, MdKind::Heading(1)), "'Title' is h1: {s:?}");
        assert!(has(&s, 7, 9, heading_markup), "trailing ' #' close dim: {s:?}");
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
        let emph_markup = MdKind::ConcealMarkup(ConcealKind::Emphasis);
        assert!(has(&s, 0, 2, emph_markup), "opening ** dim: {s:?}");
        assert!(has(&s, 6, 8, emph_markup), "closing ** dim: {s:?}");
        assert!(has(&s, 2, 6, MdKind::Bold), "inner bold: {s:?}");
    }

    #[test]
    fn bold_italic_triple_star() {
        // `***x***` is BOTH strong and emphasis: pulldown nests an emphasis (outer
        // single `*`) around a strong (inner `**`), so the inner `x` is BoldItalic
        // and the three stars at each end dim as Markup (outer 1 + inner 2).
        let s = spans("***x***");
        let emph_markup = MdKind::ConcealMarkup(ConcealKind::Emphasis);
        assert!(has(&s, 3, 4, MdKind::BoldItalic), "inner x is bold+italic: {s:?}");
        assert!(has(&s, 0, 1, emph_markup), "outer opening `*` dim: {s:?}");
        assert!(has(&s, 1, 3, emph_markup), "inner opening `**` dim: {s:?}");
        assert!(has(&s, 4, 6, emph_markup), "inner closing `**` dim: {s:?}");
        assert!(has(&s, 6, 7, emph_markup), "outer closing `*` dim: {s:?}");
    }

    #[test]
    fn italic_underscore() {
        let s = spans("_it_");
        let emph_markup = MdKind::ConcealMarkup(ConcealKind::Emphasis);
        assert!(has(&s, 0, 1, emph_markup));
        assert!(has(&s, 3, 4, emph_markup));
        assert!(has(&s, 1, 3, MdKind::Italic));
    }

    #[test]
    fn inline_code_dims_backticks() {
        let s = spans("`code`");
        let code_markup = MdKind::ConcealMarkup(ConcealKind::Code);
        assert!(has(&s, 0, 1, code_markup));
        assert!(has(&s, 5, 6, code_markup));
        assert!(has(&s, 1, 5, MdKind::Code { inline: true }));
    }

    #[test]
    fn link_text_accent_brackets_dim() {
        let s = spans("[awl](http://x)");
        // whole link dimmed first (links are NOT WYSIWYG-concealable in v1 — plain
        // `Markup`, not `ConcealMarkup`) ...
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
        // A FENCED block dims the WHOLE range as the WYSIWYG-concealable
        // `ConcealMarkup(Fence)` (fences + info), then the body Text overrides to
        // mono `Code { inline: false }` with HIGHEST priority.
        let s = spans("```\nlet x=1;\n```");
        assert!(
            has(&s, 0, 16, MdKind::ConcealMarkup(ConcealKind::Fence)),
            "whole fenced block is the concealable Fence markup: {s:?}"
        );
        assert!(has(&s, 4, 13, MdKind::Code { inline: false }), "fenced body is Code: {s:?}");
        // An INDENTED (no-fence) code block: the body (range excludes the 4-space
        // indent) is Code, and the whole-block wrapper stays PLAIN (non-concealing)
        // `Markup` — no fence to hide behind a panel.
        let s = spans("    code\n");
        assert!(has(&s, 4, 9, MdKind::Code { inline: false }), "indented body is Code: {s:?}");
        assert!(
            s.iter().any(|(_, k)| *k == MdKind::Markup),
            "an indented block's wrapper stays plain, non-concealing Markup: {s:?}"
        );
        assert!(
            !s.iter().any(|(_, k)| matches!(k, MdKind::ConcealMarkup(ConcealKind::Fence))),
            "an indented block must never carry the Fence conceal kind: {s:?}"
        );
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
        // The fence markers + the info string ("rust") stay dim, WYSIWYG-concealable
        // `ConcealMarkup(Fence)` — the whole block is dimmed first and NO role span
        // ever falls on the info-string bytes.
        assert!(
            s.iter().any(|(r, k)| {
                *k == MdKind::ConcealMarkup(ConcealKind::Fence) && r.start <= 3 && r.end >= 7
            }),
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
    fn tilde_fence_highlights_body_same_as_backtick_fence() {
        use crate::syntax::{Lang, SynKind};
        // A `~~~` fence (pulldown's OTHER `CodeBlockKind::Fenced` delimiter) must
        // hit the exact same fence-syntax path as a backtick fence — the parse is
        // delimiter-agnostic by construction (pulldown reports both as `Fenced`),
        // this pins that with a real assertion rather than leaving it unverified.
        let doc = "~~~rust\n// c\nlet s=\"x\";\n~~~";
        let s = spans(doc);
        assert!(
            has(&s, 8, 12, MdKind::CodeSyntax { role: SynKind::Comment, lang: Lang::Rust }),
            "'// c' is a rust comment role span under a tilde fence: {s:?}"
        );
        assert!(
            has(&s, 19, 22, MdKind::CodeSyntax { role: SynKind::Str, lang: Lang::Rust }),
            "'\"x\"' is a rust string role span under a tilde fence: {s:?}"
        );
        assert!(
            s.iter().any(|(r, k)| {
                *k == MdKind::ConcealMarkup(ConcealKind::Fence) && r.start <= 3 && r.end >= 7
            }),
            "the info string 'rust' stays markup under a tilde fence: {s:?}"
        );
        assert!(
            !s.iter().any(|(r, k)| matches!(k, MdKind::CodeSyntax { .. }) && r.start < 8),
            "no role span may touch the fence/info bytes before the body: {s:?}"
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
        assert!(
            s.iter().any(|(_, k)| *k == MdKind::Code { inline: false }),
            "body is still Code: {s:?}"
        );
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
    fn highlight_basic_pair_dims_markers_and_marks_content() {
        let s = spans("==marked==");
        let hl_markup = MdKind::ConcealMarkup(ConcealKind::Highlight);
        assert!(has(&s, 0, 2, hl_markup), "opening == dim: {s:?}");
        assert!(has(&s, 8, 10, hl_markup), "closing == dim: {s:?}");
        assert!(has(&s, 2, 8, MdKind::Highlight), "inner content highlighted: {s:?}");
    }

    #[test]
    fn highlight_multiple_pairs_on_one_line() {
        let s = spans("==a== and ==b==");
        assert!(has(&s, 2, 3, MdKind::Highlight), "first pair 'a': {s:?}");
        assert!(has(&s, 12, 13, MdKind::Highlight), "second pair 'b': {s:?}");
        assert_eq!(
            s.iter().filter(|(_, k)| *k == MdKind::Highlight).count(),
            2,
            "exactly two highlight spans: {s:?}"
        );
    }

    #[test]
    fn single_equals_never_matches() {
        // The whole motivation for choosing `==`: a bare `=` (prose like `x = y`,
        // or a single-equals assignment) must never be treated as a delimiter.
        let s = spans("if x = y then z");
        assert!(
            !s.iter().any(|(_, k)| *k == MdKind::Highlight),
            "a single '=' must never highlight: {s:?}"
        );
    }

    #[test]
    fn unclosed_highlight_stays_literal() {
        // An opening `==` with no matching close: no span at all (not even a dim
        // Markup for the stray delimiter) — it just reads as plain `=` characters.
        let s = spans("==never closed");
        assert!(s.is_empty(), "an unclosed == must stay completely plain: {s:?}");
    }

    #[test]
    fn adjacent_four_equals_is_inert() {
        // A run of exactly 4 `=` is ambiguous (not a valid isolated `==` pair at
        // any offset within it) and is left as plain literal text — no highlight,
        // no markup, matching a `===`/`====` divider-typo staying inert too.
        assert!(spans("before ==== after").is_empty(), "==== must not highlight");
        assert!(spans("a === b").is_empty(), "=== (odd run) must not highlight either");
    }

    #[test]
    fn highlight_ignored_inside_inline_code() {
        // Inline code arrives via `Event::Code`, never `Event::Text`, so the
        // highlight scan structurally never sees it — `==x==` inside backticks
        // stays plain mono Code, no Highlight span.
        let s = spans("`==x==`");
        assert!(has(&s, 1, 6, MdKind::Code { inline: true }), "inner text is plain Code: {s:?}");
        assert!(
            !s.iter().any(|(_, k)| *k == MdKind::Highlight),
            "inline code must never highlight: {s:?}"
        );
    }

    #[test]
    fn highlight_ignored_inside_fenced_code() {
        let s = spans("```\n==x==\n```");
        assert!(
            !s.iter().any(|(_, k)| *k == MdKind::Highlight),
            "a fenced code body must never highlight: {s:?}"
        );
        assert!(
            s.iter().any(|(_, k)| *k == MdKind::Code { inline: false }),
            "body is still Code: {s:?}"
        );
    }

    #[test]
    fn highlight_no_cross_line_span_through_soft_wrap() {
        // A soft-wrapped paragraph ("==a" / newline / "b==") is ONE paragraph but
        // arrives as two `Text` events split at the break (pulldown emits a
        // `SoftBreak` between them, never embedding the `\n` in a `Text` range),
        // so neither half sees a complete pair — no highlight spans a line break.
        let s = spans("==a\nb==");
        assert!(
            !s.iter().any(|(_, k)| *k == MdKind::Highlight),
            "a highlight must never span a soft-wrapped line break: {s:?}"
        );
    }

    #[test]
    fn highlight_no_cross_line_guard_fires_directly() {
        // A defensive unit test of the guard itself (pulldown's Text events don't
        // normally embed a raw '\n', so this constructs the case by hand): a
        // candidate pair separated by a newline is REJECTED, and the rejected
        // close is retried as a fresh open against the NEXT candidate.
        let mut out = Vec::new();
        let text = "==ab\ncd==ef==";
        push_highlight_spans(&mut out, text, &(0..text.len()));
        assert!(
            !out.iter().any(|(r, k)| *k == MdKind::Highlight && text[r.clone()].contains('\n')),
            "no highlight span may contain a newline: {out:?}"
        );
        assert!(
            has(&out, 9, 11, MdKind::Highlight),
            "the rejected close re-pairs with the next candidate ('ef'): {out:?}"
        );
    }

    #[test]
    fn non_markdown_code_buffer_never_sees_highlight() {
        // `markdown::spans` is only ever CALLED on an `is_markdown` buffer (see
        // `render/text.rs::parse_doc_spans`'s `md_enabled` gate); a `.rs` file's
        // `a == b` comparison never reaches this module at all — the render-level
        // `non_markdown_...never_matches` test in `render/tests.rs` pins that gate.
        // This is a belt-and-braces check on the function's OWN behavior: even
        // called directly on Rust-shaped text, a single comparison `==` (with no
        // SECOND `==` anywhere to pair with) can never highlight — an unpaired
        // marker is always the "unclosed" case, never a false-positive match.
        let s = spans("fn main() {\n    if a == b {}\n}\n");
        assert!(
            !s.iter().any(|(_, k)| *k == MdKind::Highlight),
            "a rust-shaped '==' comparison must never highlight: {s:?}"
        );
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
