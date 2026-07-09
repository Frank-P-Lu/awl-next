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
//! own face at any size — a DESIGN call: size alone carries the hierarchy. (Inline
//! `**bold**` DOES shape in a real bold face on proportional worlds — the 10
//! display faces bundle a 700 weight, `render::FONT_THEME_BOLD_FACES`; the mono
//! worlds stay Regular-only and bold falls back gracefully there.)
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

/// Whether INLINE IMAGES are active. DEFAULT ON — a markdown `![alt](path.png)`
/// image reference conceals its source off the caret's line and reserves a TALL
/// row (fit-to-column, its display height), which the renderer fills with the
/// decoded image (the GPU draw lands next phase). OFF reproduces the
/// pre-feature rendering byte-for-byte: no image span is ever emitted (see
/// [`spans`]), so the `![alt](path)` source renders as plain default-ink text
/// exactly as it did before this round — no conceal, no tall row, no image.
///
/// NATIVE-ONLY: images read a file's header dimensions off disk and (next
/// phase) decode its pixels, neither of which the wasm build does — so
/// [`inline_images_on`] is unconditionally `false` on `wasm32`, making the
/// whole feature vanish there (the source renders plain, byte-identical to the
/// native-off case). Mirrors the daemon/session native-only gate.
static INLINE_IMAGES_ON: AtomicBool = AtomicBool::new(true);

/// True when inline images are active (read by [`spans`] to gate the image
/// span + by the renderer to gate the tall row / draw). Always `false` on wasm
/// (the feature is native-only — see [`INLINE_IMAGES_ON`]).
pub fn inline_images_on() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        false
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        INLINE_IMAGES_ON.load(Ordering::Relaxed)
    }
}

/// Set inline images on/off explicitly — the config sticky-pref launch-apply
/// (mirrors [`set_wysiwyg_on`]). A no-op-in-effect on wasm, where
/// [`inline_images_on`] ignores the flag and always reports `false`.
pub fn set_inline_images_on(on: bool) {
    INLINE_IMAGES_ON.store(on, Ordering::Relaxed);
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
    /// bold on proportional worlds now; a heading just doesn't spend it.)
    Heading(u8),
    /// `**bold**` / `__bold__` content → Bold weight. Resolves to the world's real
    /// bundled 700 face on a proportional world (`render::FONT_THEME_BOLD_FACES`),
    /// graceful Regular fallback on a mono-display world.
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
    /// A GFM TABLE's ENTIRE byte range — BLOCK-scoped exactly like [`Fence`]
    /// (reveals iff the caret sits anywhere inside the table). Unlike every other
    /// conceal kind this one hides the WHOLE block (all rows — content, pipes,
    /// separator), because the renderer replaces the source with a drawn pixel
    /// GRID (`render::TextPipeline::prepare_table_grid`): grid and source can't
    /// share the same rows without overlapping, so the caret entering the table
    /// reveals the raw source and parks the grid (the WYSIWYG "heading model").
    /// The dim `TablePipe`/`TableSep`/`TableHeader` spans still style that
    /// revealed source; this additive span only drives the off-cursor conceal.
    Table,
    /// A markdown IMAGE reference's ENTIRE `![alt](path)` source range —
    /// LINE-scoped exactly like [`Heading`](Self::Heading)/[`Emphasis`](Self::Emphasis)
    /// (an image ref is one line): reveals iff the caret is on the image's own
    /// line. Off-cursor the source conceals (zero-width) and the decoded image
    /// draws in the TALL row the line reserves (image height `h`). On-cursor the
    /// raw `![alt](path)` source reveals at body size CENTRED OVER the still-drawn,
    /// DIMMED image (the caption model, re-decided 2026-07-09) — the row stays
    /// exactly `h`, so the caret landing on / leaving the line causes ZERO reflow;
    /// a soft scrim band lifts the caption's legibility over the image pixels
    /// (`render::spans::build_line_attrs` / `render::layers::prepare_images`). This
    /// differs from the pure "heading model" the [`Table`](Self::Table) kind follows
    /// (grid parks entirely on reveal): an image shows source AND a dimmed preview at
    /// once. Emitted by
    /// [`spans`] ONLY when [`inline_images_on`] is true (native + enabled), so an
    /// images-off / wasm build emits no image span at all and renders the source
    /// byte-identically to the pre-feature editor.
    Image,
    /// A markdown link's MARKUP plumbing — the opening `[`, and the whole
    /// `](url)` tail (closing bracket, parens, destination + any title) — the
    /// LAST markup family that used to keep its brackets/URL visible as dim
    /// [`MdKind::Markup`]. LINE-scoped exactly like [`Heading`](Self::Heading)/
    /// [`Emphasis`](Self::Emphasis): off the caret's line the plumbing conceals
    /// to zero-width and only the link TEXT (its own [`MdKind::LinkText`] span,
    /// full content ink) shows, so `see [the essay](http://…)` reads as `see the
    /// essay`; on the caret's own line the full `[text](url)` source reveals for
    /// editing. Note the link TEXT is NOT part of this span (only the markup
    /// pieces are), so the conceal pass never hides the text. Emitted per
    /// [`push_link_markers`]; a reference-style / malformed link with no `](`
    /// falls back to the plain non-concealing [`MdKind::Markup`]. Calm — plain
    /// content ink, no hyperlink color, no amber (awl has no link accent).
    Link,
    /// A blockquote line's leading `>` marker run (`> `, or a nested `> > `, plus
    /// the trailing space) — LINE-scoped exactly like [`Heading`](Self::Heading)/
    /// [`Emphasis`](Self::Emphasis): off the caret's line the marker(s) conceal to
    /// zero-width, and the block's affordance is the big DIM hanging quotation mark
    /// the renderer hangs in the LEFT MARGIN at the block's first line (page mode
    /// only — see `render::TextPipeline::quote_marks` / `prepare_ornaments`). On
    /// the caret's own line the raw `>` markers reveal for editing. One
    /// [`push_quote_markers`] span per blockquote LINE (nested `>>` markers all
    /// live in one line's run, so they conceal together). The blockquote BODY text
    /// keeps its own [`MdKind::Quote`] styling span (dim or full, a taste flag).
    Blockquote,
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
            ConcealKind::Table => "table",
            ConcealKind::Image => "image",
            ConcealKind::Link => "link",
            ConcealKind::Blockquote => "blockquote",
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
    // NOTE: the centered `---`/`***`/`___` section-break FLEURON's size is NO LONGER a
    // single rung here — it is PER-WORLD ([`crate::theme::Theme::ornament_scale`]), keyed
    // to the ornament's character (Junicode flowers reward size; clean geometric marks
    // don't). Both readers — `render::spans::md_line_scale` (the break ROW height) and
    // `render::layers::prepare_ornaments` (the glyph LINE-BOX) — consult that field, so
    // the two stay in lockstep. Tune the three tiers in `theme.rs`
    // (`ORNAMENT_SCALE_ORNATE` / `_FLEURON` / `_GEOMETRIC`).
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
    // every other construct parses exactly as before (the option is additive).
    let opts = Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES;
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
/// is the count of newlines before the span. ATX (`# …`) headings ONLY — a
/// SETEXT heading (a paragraph underlined by `===`/`---`) is filtered OUT
/// (`headings_from_spans`), matching heading-SIZE + the WYSIWYG conceal, both of
/// which key off the leading `#`; without the filter a stray `-` typed under a
/// paragraph promotes it to an outline heading. One entry per
/// heading line — a title built from several runs (e.g. `# a *b*`) emits multiple
/// Heading spans on the same line, so we keep the first. A heading whose title is
/// ENTIRELY styled (e.g. `# *all italic*`) yields no plain Heading span and is the
/// one documented gap; in practice outline titles are plain text. Empty for a
/// document with no headings (the caller then declines to summon the picker).
pub fn headings(text: &str) -> Vec<Heading> {
    headings_from_spans(text, &spans(text))
}

/// The heading-distillation CORE, over an already-parsed span list — so the
/// persistent margin outline (`render/text.rs`) can ride the SAME
/// `markdown::spans` parse the styling pass already pays for, never a second
/// pulldown parse. [`headings`] is the thin wrapper for callers holding only
/// `text` (the summoned outline picker + tests). `spans` MUST be the whole
/// document's span list in document byte coords (as [`spans`] returns) or the
/// per-span newline count is wrong.
pub fn headings_from_spans(
    text: &str,
    spans: &[(Range<usize>, MdKind)],
) -> Vec<Heading> {
    let mut out: Vec<Heading> = Vec::new();
    for (range, kind) in spans {
        let range = range.clone();
        let kind = *kind;
        let MdKind::Heading(level) = kind else {
            continue;
        };
        // ATX-ONLY. A SETEXT heading (a paragraph underlined by `===`/`---`) is a
        // `Tag::Heading` to pulldown too, but awl treats ONLY leading-`#` (ATX)
        // lines as headings everywhere else — heading SIZE counts `#`s
        // (`md_line_scale`) and the WYSIWYG conceal hides `#`s — so the outline
        // must agree, or a stray `-` typed under a paragraph silently promotes it
        // to a heading (the reported bug). The title span starts AFTER any `# `
        // markers, so the on-line prefix before it is indent + markers: it's ATX
        // iff that prefix (leading whitespace trimmed) opens with `#`.
        let line_start = text[..range.start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        if !text[line_start..range.start].trim_start().starts_with('#') {
            continue;
        }
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

/// One parsed IMAGE reference, recovered from an `![alt](path)` SOURCE substring
/// (the byte range of a [`MdKind::ConcealMarkup`]`(`[`ConcealKind::Image`]`)`
/// span). PURE data — the renderer feeds `text[range]` to [`parse_image_source`]
/// each reshape to get the destination PATH (to read the image's header
/// dimensions + draw it), the ALT text, and an optional Obsidian-style width
/// HINT, without a second pulldown parse. This is the "side table keyed by
/// span" the design chose over widening the `Copy` [`MdKind`] with `String`
/// fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRef {
    /// The alt text with any trailing `|NNN`/`|WxH` size hint stripped off.
    pub alt: String,
    /// The image destination — the `(path)` link target (title/angle-brackets
    /// stripped). May be relative (resolved against the doc's dir by the caller).
    pub path: String,
    /// The width HINT parsed OUT of the alt (`![alt|300](p)` → `Some(300)`;
    /// `![alt|300x200](p)` → `Some(300)`, the WIDTH — the height is derived from
    /// the intrinsic aspect, so a `WxH`'s `H` is ignored in v1). `None` when the
    /// alt carries no `|NNN`/`|WxH` suffix.
    pub width_hint: Option<u32>,
}

/// Parse an `![alt](path)` image SOURCE substring into its [`ImageRef`] parts.
/// Lenient + total (returns `None` only if the substring isn't a well-formed
/// `![...](...)`), operating on the exact byte range pulldown ruled an image, so
/// there is no ambiguity about where the ref begins/ends. Handles a `(path
/// "title")` (path = first whitespace token) and a `(<path>)` angle form, and
/// splits the Obsidian size hint out of the alt via [`split_alt_hint`].
pub fn parse_image_source(src: &str) -> Option<ImageRef> {
    let rest = src.trim().strip_prefix("![")?;
    let close = rest.find(']')?;
    let raw_alt = &rest[..close];
    let inner = rest[close + 1..].trim_start().strip_prefix('(')?;
    let end = inner.find(')')?;
    let dest = inner[..end].trim();
    let path = if let Some(a) = dest.strip_prefix('<') {
        a.split('>').next().unwrap_or("").to_string()
    } else {
        dest.split_whitespace().next().unwrap_or("").to_string()
    };
    if path.is_empty() {
        return None;
    }
    let (alt, width_hint) = split_alt_hint(raw_alt);
    Some(ImageRef { alt, path, width_hint })
}

/// Extract EVERY inline image reference `![alt](path)` from `text`, UNGATED by the
/// inline-images toggle. [`spans`] only emits an image span when [`inline_images_on`]
/// is true (native + enabled), so it can't be the scanner's source; this walks the
/// SAME pulldown parse for `Tag::Image` and feeds each image's byte range to the SAME
/// [`parse_image_source`] the renderer trusts — the real parser, never a regex. A
/// reference-style / remote image `parse_image_source` can't resolve to a local
/// `(path)` is skipped (it names no local asset). Frontmatter is stripped first
/// (mirroring [`spans`]), so a metadata value never mis-parses as an image.
///
/// Used by [`crate::assets::scan`] (the Asset Cleaner) to collect the images a
/// document references, so an unreferenced `assets/` file can be found. PURE — no
/// clock, no filesystem — over the document text.
pub fn image_refs(text: &str) -> Vec<ImageRef> {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    let text = match crate::frontmatter::detect(text) {
        Some(fm) => &text[fm.range.end..],
        None => text,
    };
    let opts = Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES;
    let mut out = Vec::new();
    for (ev, range) in Parser::new_ext(text, opts).into_offset_iter() {
        if let Event::Start(Tag::Image { .. }) = ev {
            if let Some(img) = text.get(range).and_then(parse_image_source) {
                out.push(img);
            }
        }
    }
    out
}

/// Split an image alt on a trailing `|NNN` / `|WxH` size hint (the Obsidian
/// `![alt|300](p)` convention — the size lives in the ALT so pulldown still
/// parses the image cleanly). Returns the alt with the hint removed + the WIDTH
/// (the `NNN`, or the `W` of `WxH`; `H` is ignored — height rides the intrinsic
/// aspect in v1). No `|`, or a non-numeric suffix (so an alt that legitimately
/// contains `|`, like `"a | b"`, is preserved verbatim), yields the alt
/// unchanged + `None`.
fn split_alt_hint(alt: &str) -> (String, Option<u32>) {
    let Some((head, tail)) = alt.rsplit_once('|') else {
        return (alt.to_string(), None);
    };
    let t = tail.trim();
    let (w, h) = match t.split_once(['x', 'X']) {
        Some((w, h)) => (w, Some(h)),
        None => (t, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if digits(w) && h.map(digits).unwrap_or(true) {
        if let Ok(n) = w.parse::<u32>() {
            return (head.trim_end().to_string(), Some(n));
        }
    }
    (alt.to_string(), None)
}

/// Set the Obsidian `|NNN` width hint on an image ALT to `width` — the inverse of
/// [`split_alt_hint`]. If the alt already carries a `|NNN`/`|WxH` hint it is
/// REPLACED (the alt text before it is preserved verbatim); otherwise `|width` is
/// appended after the alt text. An alt that legitimately contains a `|` but no
/// numeric suffix (`"a | b"`) is treated as hint-less, so the new hint appends
/// cleanly (`"a | b|300"`). Pure — the drag-resize write-back builds the new alt
/// with this and applies it as ONE undoable edit.
fn set_alt_width_hint(raw_alt: &str, width: u32) -> String {
    let (base, _) = split_alt_hint(raw_alt);
    if base.is_empty() {
        format!("|{}", width)
    } else {
        format!("{}|{}", base, width)
    }
}

/// DRAG-RESIZE WRITE-BACK: given an image SOURCE substring `![alt](path)` and a new
/// pixel `width`, compute the BYTE RANGE within `src` of the ALT text and the
/// replacement alt (the Obsidian `![alt|NNN](path)` form — the hint set/replaced by
/// [`set_alt_width_hint`]). Returns `None` if `src` isn't a well-formed
/// `![...](...)`. Pure: the app converts the `src`-relative byte offsets to absolute
/// buffer positions and applies ONE [`crate::buffer::Buffer::replace_char_range`] —
/// exactly the single-undoable-edit shape `write_back_lang_tag_once` uses, so a
/// whole drag writes back ONCE on release and Cmd-Z restores the pre-drag size.
pub fn image_width_hint_edit(src: &str, width: u32) -> Option<(usize, usize, String)> {
    let open = src.find("![")?;
    let alt_start = open + 2;
    let close_rel = src.get(alt_start..)?.find(']')?;
    let alt_end = alt_start + close_rel;
    // Must be a real image: a `(path)` link target follows the `]`.
    let after = src.get(alt_end + 1..)?.trim_start();
    if !after.starts_with('(') {
        return None;
    }
    let raw_alt = &src[alt_start..alt_end];
    Some((alt_start, alt_end, set_alt_width_hint(raw_alt, width)))
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

/// The destination URL of the markdown link CONTAINING document byte offset
/// `byte`, or `None` when the caret is not inside any link — the pure extraction
/// behind [`crate::keymap::Action::FollowLink`] (open-link-at-point). Reuses
/// pulldown (the SAME parse [`spans`] drives), tracking each `Tag::Link`'s own
/// `dest_url` against its byte range: the first link whose `[text](url)` range
/// contains `byte` wins. A leading [`crate::frontmatter`] block is skipped exactly
/// like [`spans`] (a link can't live in frontmatter), so `byte` is measured in the
/// same DOCUMENT coordinates the caret uses. Pure + total — never opens anything
/// itself (the live App performs the OS browser handoff on the returned URL); a
/// caret outside every link is the calm `None` no-op.
pub fn link_at(text: &str, byte: usize) -> Option<String> {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    let (body, body_offset) = match crate::frontmatter::detect(text) {
        Some(fm) => (&text[fm.range.end..], fm.range.end),
        None => (text, 0),
    };
    if byte < body_offset {
        return None;
    }
    let target = byte - body_offset;
    let opts = Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES;
    for (ev, range) in Parser::new_ext(body, opts).into_offset_iter() {
        if let Event::Start(Tag::Link { dest_url, .. }) = ev {
            if range.contains(&target) {
                return Some(dest_url.to_string());
            }
        }
    }
    None
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

/// Dim a GFM table's STRUCTURAL markup within its byte `range` (into `text`):
/// every literal `|` cell-delimiter pipe on a row becomes a [`MdKind::TablePipe`]
/// span, and the whole HEADER-SEPARATOR row (`|---|:--:|---|`) becomes one
/// [`MdKind::TableSep`] span. pulldown emits NO event for the pipes or the
/// separator row, so we derive both from the table's raw text — but we only ever
/// look INSIDE a range pulldown already ruled a table, so this never mis-fires on
/// a stray `|` in ordinary prose. awl is a SOURCE editor: the markup recedes to
/// the dim ink, no grid is ever drawn. The header/body CELL content is left to the
/// inline Text pass (header cells additionally get a [`MdKind::TableHeader`] tag
/// from the `TableCell` event); a pipe never overlaps a cell's content, so the
/// spans compose cleanly.
fn push_table_markup(out: &mut Vec<(Range<usize>, MdKind)>, text: &str, range: &Range<usize>) {
    // The whole-table BLOCK conceal span (WYSIWYG): off the caret's block the
    // renderer hides every source row and draws a pixel GRID in its place; the
    // caret entering the block reveals the source and parks the grid (the
    // heading model — see `ConcealKind::Table`). Additive, laid FIRST so the
    // dim `TablePipe`/`TableSep`/`TableHeader` spans still ride the revealed source.
    out.push((range.clone(), MdKind::ConcealMarkup(ConcealKind::Table)));
    let s = &text[range.clone()];
    let mut off = 0usize; // byte offset of the current line, relative to `s`
    for (li, line) in s.split_inclusive('\n').enumerate() {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let base = range.start + off;
        // GFM's header-separator is ALWAYS the table's second line — guarding by
        // index (not shape alone) means a body cell whose content is literally `---`
        // is never mistaken for it.
        if li == 1 && is_separator_row(content) {
            // The whole `-`/`:`/`|` run (first to last non-whitespace) is one dim span.
            let lead = content.len() - content.trim_start().len();
            let tail = content.trim_end().len();
            if tail > lead {
                out.push((base + lead..base + tail, MdKind::TableSep));
            }
        } else {
            for (i, b) in content.bytes().enumerate() {
                if b == b'|' {
                    out.push((base + i..base + i + 1, MdKind::TablePipe));
                }
            }
        }
        off += line.len();
    }
}

/// True when `s` is a GFM table HEADER-SEPARATOR row — a non-empty line built only
/// of pipes / dashes / colons / spaces / tabs that contains at least one `-` (the
/// delimiter run under the header). pulldown consumes this row without an event, so
/// [`push_table_markup`] recognizes it by shape to dim it whole.
fn is_separator_row(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty()
        && t.contains('-')
        && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

// --- Align Table (on-demand column alignment of the SOURCE) ------------------
//
// awl is a SOURCE editor, not a WYSIWYG grid: aligning a table re-pads the raw
// pipe-delimited text so the `|` line up (Prettier-style), it never draws a grid.
// The command ([`crate::keymap::Action::AlignTable`]) finds the table under the
// caret via [`table_block_lines`] and replaces it with [`align_table`]'s output as
// one undoable edit. Both are PURE + exhaustively unit-tested.
//
// FOLLOW-UP (deferred): auto-align-on-type (re-align the table as you edit a cell)
// needs live cursor-preservation care (map the caret's cell/offset across the
// re-pad so it doesn't jump) — banked, not built. The on-demand command is v1.

/// A GFM column's alignment, parsed from its header-separator cell (`:---` left,
/// `---:` right, `:--:` center, `---` none). Drives how [`sep_cell`] re-emits the
/// separator's colons at the aligned column width; the DATA cells are always
/// left-aligned (padded on the right) in v1 — the markers are preserved for the
/// reader/other tools, not used to re-justify cell content.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ColAlign {
    None,
    Left,
    Right,
    Center,
}

/// Display width of `s` in monospace cells — a heuristic, since the codebase
/// bundles no `unicode-width` crate: a CJK / fullwidth scalar (the SAME broad
/// range set the renderer's `is_cjk` uses) counts as 2 columns, every other char
/// as 1. CAVEAT: this is not a full East-Asian-Width table (combining marks,
/// emoji ZWJ sequences, and a `\|` escape — counted as its two literal source
/// bytes' worth, 2 — are all approximated), but it matches how awl's own monospace
/// grid renders CJK, so the pipes line up for the common Latin+CJK case.
fn cell_display_width(s: &str) -> usize {
    s.chars()
        .map(|c| if is_wide_cell_char(c) { 2 } else { 1 })
        .sum()
}

/// Whether `c` occupies two monospace columns — the same CJK / fullwidth ranges
/// the renderer's `is_cjk` treats as a wide glyph (kept in sync by construction;
/// see [`cell_display_width`]'s caveat).
fn is_wide_cell_char(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F   // Hangul Jamo
        | 0x2E80..=0x303E // CJK radicals / Kangxi / symbols & punctuation
        | 0x3041..=0x33FF // Hiragana … CJK compatibility
        | 0x3400..=0x4DBF // CJK Ext A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs
        | 0xA000..=0xA4CF // Yi
        | 0xAC00..=0xD7A3 // Hangul syllables
        | 0xF900..=0xFAFF // CJK compatibility ideographs
        | 0xFE30..=0xFE4F // CJK compatibility forms
        | 0xFF00..=0xFF60 // fullwidth forms
        | 0xFFE0..=0xFFE6 // fullwidth signs
    )
}

/// Parse a header-separator cell (its content between two pipes, e.g. `:--:`) into
/// its [`ColAlign`]. Colons on both ends = center, left end = left, right end =
/// right, neither = none.
pub(crate) fn parse_col_align(cell: &str) -> ColAlign {
    let t = cell.trim();
    match (t.starts_with(':'), t.ends_with(':') && t.len() > 1) {
        (true, true) => ColAlign::Center,
        (true, false) => ColAlign::Left,
        (false, true) => ColAlign::Right,
        (false, false) => ColAlign::None,
    }
}

/// Split ONE table row's source into its trimmed cell contents, honoring a `\|`
/// escape (an escaped pipe is part of the cell, never a delimiter). The structural
/// empty cells produced by the leading/trailing outer pipes are dropped, so
/// `| a | b |` yields `["a", "b"]` and a pipeless line yields the whole line as one
/// cell.
pub(crate) fn split_row_cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let mut cells: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut escaped = false;
    for c in t.chars() {
        if escaped {
            cur.push(c);
            escaped = false;
        } else if c == '\\' {
            cur.push(c);
            escaped = true;
        } else if c == '|' {
            cells.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    cells.push(cur.trim().to_string());
    // Drop the empty cell before the first `|` / after the last `|` (the outer pipes).
    if t.starts_with('|') && cells.first().is_some_and(|c| c.is_empty()) {
        cells.remove(0);
    }
    if t.ends_with('|') && cells.last().is_some_and(|c| c.is_empty()) {
        cells.pop();
    }
    cells
}

/// Re-emit one column's SEPARATOR cell (`ColAlign` + target `width`), keeping the
/// alignment colons and filling the rest with `-` so its total width matches the
/// data cells. `width` is already floored to each align's minimum by [`align_table`].
fn sep_cell(align: ColAlign, width: usize) -> String {
    match align {
        ColAlign::None => "-".repeat(width),
        ColAlign::Left => format!(":{}", "-".repeat(width - 1)),
        ColAlign::Right => format!("{}:", "-".repeat(width - 1)),
        ColAlign::Center => format!(":{}:", "-".repeat(width - 2)),
    }
}

/// Re-pad ONE GFM table's source so every `|` lines up (Prettier-style monospace
/// alignment), returning the aligned lines joined by `\n` (no trailing newline).
///
/// Contract:
/// - Column count = the MAX cell count across all rows; RAGGED rows (missing
///   trailing cells) are padded with empty cells so every row has the same pipes.
/// - Each column's width = the max [`cell_display_width`] of its non-separator
///   cells, floored to what its alignment marker needs (none≥1, left/right≥2,
///   center≥3) so the re-emitted separator is always valid.
/// - Data cells are LEFT-aligned (padded on the right) with exactly one space of
///   padding inside each pipe: `| cell  | cell |`.
/// - The header-SEPARATOR row (always the second line of a GFM table) is re-emitted
///   as dashes at the column width with its `:` alignment markers PRESERVED.
/// - IDEMPOTENT: aligning already-aligned source returns it unchanged.
///
/// Width uses DISPLAY width (CJK = 2) where possible — see [`cell_display_width`]'s
/// caveat for the heuristic's limits. Pure; no clock, no allocation beyond output.
pub fn align_table(table_src: &str) -> String {
    let lines: Vec<&str> = table_src.split('\n').collect();
    // The separator is ALWAYS the 2nd line of a GFM table (guarded by index, like
    // `push_table_markup`), so a body cell of literal `---` is never mistaken for it.
    let sep_idx = 1usize;
    let rows: Vec<Vec<String>> = lines.iter().map(|l| split_row_cells(l)).collect();
    let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if ncols == 0 {
        return table_src.to_string();
    }
    // Per-column alignment, read from the separator row's cells (missing → none).
    let aligns: Vec<ColAlign> = (0..ncols)
        .map(|c| {
            rows.get(sep_idx)
                .and_then(|r| r.get(c))
                .map(|s| parse_col_align(s))
                .unwrap_or(ColAlign::None)
        })
        .collect();
    // Per-column width = max non-separator cell display width, floored to the
    // alignment marker's minimum so the separator re-emits validly.
    let mut widths = vec![0usize; ncols];
    for (ri, row) in rows.iter().enumerate() {
        if ri == sep_idx {
            continue;
        }
        for (ci, cell) in row.iter().enumerate() {
            widths[ci] = widths[ci].max(cell_display_width(cell));
        }
    }
    for (c, w) in widths.iter_mut().enumerate() {
        let min = match aligns[c] {
            ColAlign::None => 1,
            ColAlign::Left | ColAlign::Right => 2,
            ColAlign::Center => 3,
        };
        *w = (*w).max(min);
    }
    // Re-emit every row at the aligned widths.
    let mut out = Vec::with_capacity(lines.len());
    for (ri, row) in rows.iter().enumerate() {
        let mut s = String::from("|");
        for (c, width) in widths.iter().copied().enumerate() {
            s.push(' ');
            if ri == sep_idx {
                s.push_str(&sep_cell(aligns[c], width));
            } else {
                let cell = row.get(c).map(String::as_str).unwrap_or("");
                s.push_str(cell);
                for _ in cell_display_width(cell)..width {
                    s.push(' ');
                }
            }
            s.push(' ');
            s.push('|');
        }
        out.push(s);
    }
    out.join("\n")
}

// --- Table GRID pixel layout (WYSIWYG render) --------------------------------
//
// awl renders a GFM table as an aligned pixel GRID (not space-padded source,
// which can't align in a proportional face). These two PURE functions own the
// column math: [`table_column_layout`] turns per-column NATURAL widths (measured
// by the renderer as `max shaped cell width + padding`) into laid-out column
// boxes, and [`table_align_offset`] places one cell inside its box per its
// [`ColAlign`]. Both take already-measured pixel widths as `f32` inputs, so they
// carry no font dependency and are exhaustively unit-tested. See
// `render::TextPipeline::prepare_table_grid` for the measurement + placement.

/// Lay out a table's columns in pixels. `naturals[c]` is column `c`'s natural
/// width (its widest shaped cell + inner padding); `gap` is the inter-column
/// whitespace; `avail` is the writing-column width. Returns each column's left x
/// (relative to the text origin, 0-based) and final width.
///
/// If the natural total (all columns + all gaps) FITS `avail`, columns keep their
/// natural widths, left-anchored. If it OVERFLOWS, every column AND gap is scaled
/// by `avail / total` so the grid fits exactly (the v1 proportional-shrink clamp;
/// each cell's text then clips at its own column's right edge — a true horizontal
/// scroll / middle-elision is deferred to v2). Pure; no clock, O(columns).
pub(crate) fn table_column_layout(naturals: &[f32], gap: f32, avail: f32) -> (Vec<f32>, Vec<f32>) {
    let n = naturals.len();
    if n == 0 {
        return (Vec::new(), Vec::new());
    }
    let sum: f32 = naturals.iter().copied().map(|w| w.max(0.0)).sum();
    let total = sum + gap.max(0.0) * (n - 1) as f32;
    let scale = if total > avail && total > 0.0 { avail / total } else { 1.0 };
    let g = gap.max(0.0) * scale;
    let mut xs = Vec::with_capacity(n);
    let mut ws = Vec::with_capacity(n);
    let mut x = 0.0f32;
    for &nat in naturals {
        let w = nat.max(0.0) * scale;
        xs.push(x);
        ws.push(w);
        x += w + g;
    }
    (xs, ws)
}

/// The horizontal offset (from a column box's LEFT edge) at which to place a cell
/// of shaped width `cell_w` inside a column of width `col_w`, honoring the
/// column's `align` and an inner `pad`. `None`/`Left` anchor at `pad`; `Right`
/// pushes the cell to `col_w - cell_w - pad`; `Center` splits the slack. Clamped
/// so an OVER-WIDE cell (wider than its column) always left-anchors at `pad` and
/// clips at the right edge rather than spilling left. Pure.
pub(crate) fn table_align_offset(align: ColAlign, col_w: f32, cell_w: f32, pad: f32) -> f32 {
    let raw = match align {
        ColAlign::None | ColAlign::Left => pad,
        ColAlign::Right => col_w - cell_w - pad,
        ColAlign::Center => (col_w - cell_w) * 0.5,
    };
    // The cell's left must sit in [pad, col_w - cell_w] (the right bound collapses
    // to `pad` for an over-wide cell, so both clamps agree on left-anchoring it).
    let hi = (col_w - cell_w).max(pad);
    raw.max(pad).min(hi)
}

/// A line "looks like" a GFM table row for BLOCK detection: trimmed non-empty and
/// containing at least one `|`. (A real table block must ALSO carry a separator
/// row — see [`table_block_lines`] — so pipe-bearing prose is never aligned.)
fn looks_like_table_row(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && t.contains('|')
}

/// The `[start, end)` LINE range of the GFM table containing `cursor_line`, or
/// `None` if the caret is not inside one. A table is the MAXIMAL run of consecutive
/// [`looks_like_table_row`] lines around the caret that ALSO contains a
/// header-separator row (`|---|`) — the separator requirement is what keeps a stray
/// run of pipe-bearing prose from being treated as a table. Pure; `lines` is the
/// document split on `\n`. Used by [`crate::keymap::Action::AlignTable`].
pub fn table_block_lines(lines: &[&str], cursor_line: usize) -> Option<(usize, usize)> {
    if cursor_line >= lines.len() || !looks_like_table_row(lines[cursor_line]) {
        return None;
    }
    let mut start = cursor_line;
    while start > 0 && looks_like_table_row(lines[start - 1]) {
        start -= 1;
    }
    let mut end = cursor_line + 1;
    while end < lines.len() && looks_like_table_row(lines[end]) {
        end += 1;
    }
    if lines[start..end].iter().any(|l| is_separator_row(l)) {
        Some((start, end))
    } else {
        None
    }
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
    fn parse_image_source_extracts_path_alt_and_hint() {
        // No hint: alt + path recovered, hint None.
        assert_eq!(
            parse_image_source("![a cat](cat.png)"),
            Some(ImageRef { alt: "a cat".into(), path: "cat.png".into(), width_hint: None })
        );
        // `|300` width hint parsed OUT of the alt (Obsidian convention).
        assert_eq!(
            parse_image_source("![a cat|300](cat.png)"),
            Some(ImageRef { alt: "a cat".into(), path: "cat.png".into(), width_hint: Some(300) })
        );
        // `|WxH` → the WIDTH is the hint (H rides the intrinsic aspect in v1).
        assert_eq!(
            parse_image_source("![cat|300x200](cat.png)"),
            Some(ImageRef { alt: "cat".into(), path: "cat.png".into(), width_hint: Some(300) })
        );
        // A NON-numeric `|` suffix is NOT a hint — the alt (which legitimately
        // contains `|`) is preserved verbatim.
        assert_eq!(
            parse_image_source("![a | b](cat.png)"),
            Some(ImageRef { alt: "a | b".into(), path: "cat.png".into(), width_hint: None })
        );
        // A `(path "title")` — the path is the first whitespace token.
        assert_eq!(
            parse_image_source("![x](cat.png \"my title\")"),
            Some(ImageRef { alt: "x".into(), path: "cat.png".into(), width_hint: None })
        );
        // Not an image: None (never panics).
        assert_eq!(parse_image_source("just text"), None);
        assert_eq!(parse_image_source("![no dest]"), None);
    }

    #[test]
    fn image_width_hint_edit_inserts_replaces_and_bails_cleanly() {
        // INSERT: a hint-less alt gains `|NNN` after the alt text, before `]`.
        let src = "![a cat](cat.png)";
        let (b0, b1, new_alt) = image_width_hint_edit(src, 300).unwrap();
        assert_eq!(&src[b0..b1], "a cat", "byte range spans exactly the raw alt");
        assert_eq!(new_alt, "a cat|300");
        // Splicing the replacement into the range yields the Obsidian form.
        let spliced = format!("{}{}{}", &src[..b0], new_alt, &src[b1..]);
        assert_eq!(spliced, "![a cat|300](cat.png)");

        // REPLACE: an existing `|NNN` is swapped, the alt text preserved.
        let src = "![a cat|300](cat.png)";
        let (b0, b1, new_alt) = image_width_hint_edit(src, 512).unwrap();
        assert_eq!(&src[b0..b1], "a cat|300");
        assert_eq!(new_alt, "a cat|512");
        let spliced = format!("{}{}{}", &src[..b0], new_alt, &src[b1..]);
        assert_eq!(spliced, "![a cat|512](cat.png)");

        // A `|WxH` hint is also replaced (collapsing to the single width form).
        let (_, _, new_alt) = image_width_hint_edit("![cat|300x200](c.png)", 120).unwrap();
        assert_eq!(new_alt, "cat|120");

        // An alt that legitimately contains `|` (no numeric suffix) keeps it and
        // appends the new hint cleanly.
        let (_, _, new_alt) = image_width_hint_edit("![a | b](c.png)", 90).unwrap();
        assert_eq!(new_alt, "a | b|90");

        // An EMPTY alt gets a bare `|NNN`.
        let (_, _, new_alt) = image_width_hint_edit("![](c.png)", 64).unwrap();
        assert_eq!(new_alt, "|64");

        // Not a well-formed image -> None (never panics).
        assert_eq!(image_width_hint_edit("just text", 100), None);
        assert_eq!(image_width_hint_edit("![no dest]", 100), None);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))] // `inline_images_on()` is always false on wasm
    fn spans_emits_image_conceal_span_when_on_and_nothing_when_off() {
        let _g = TEST_LOCK.lock().unwrap();
        let prev = inline_images_on();
        // ON: the whole `![alt](path)` is one line-scoped ConcealMarkup(Image) span.
        set_inline_images_on(true);
        let src = "![a cat|300](cat.png)";
        let on = spans(src);
        assert!(
            on.iter().any(|(r, k)| *k == MdKind::ConcealMarkup(ConcealKind::Image)
                && r.start == 0
                && r.end == src.len()),
            "images ON should emit one ConcealMarkup(Image) over the whole ref: {on:?}"
        );
        // OFF (native): NO image span at all — byte-identical to the pre-feature
        // editor, which emitted no span for an image line.
        set_inline_images_on(false);
        let off = spans(src);
        assert!(
            !off.iter().any(|(_, k)| *k == MdKind::ConcealMarkup(ConcealKind::Image)),
            "images OFF should emit no image span: {off:?}"
        );
        set_inline_images_on(prev);
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
    fn setext_underline_is_not_a_heading_in_the_outline() {
        // The reported bug: typing a `-`/`---`/`===` on the line below a paragraph
        // (a SETEXT heading to CommonMark) silently promoted that paragraph to an
        // outline heading — even though heading-SIZE (which counts leading `#`s)
        // never treated it as one. awl is ATX-only everywhere; the outline must
        // agree. A paragraph + underline yields ZERO outline headings.
        for underline in ["-", "---", "===", "=", "--------"] {
            let doc = format!("Just a sentence.\n{underline}\n");
            let hs = headings(&doc);
            assert!(
                hs.is_empty(),
                "paragraph + {underline:?} underline must NOT be an outline heading, got {hs:?}"
            );
        }
        // ATX `#` headings are unaffected — still extracted with level + title.
        let atx = headings("# Real Heading\n\nbody\n");
        assert_eq!(atx.len(), 1, "ATX heading still counts: {atx:?}");
        assert_eq!(atx[0].level, 1);
        assert_eq!(atx[0].text, "Real Heading");
    }

    #[test]
    fn headings_from_spans_core_matches_the_wrapper() {
        // The persistent margin outline distills headings from an ALREADY-parsed
        // span list (no second pulldown parse); the core must produce the exact
        // same list as the text-only wrapper. Also proves the core is the shared
        // owner (the wrapper delegates to it).
        let doc = "# Title\n\nprose\n\n## Section A\n\nbody\n\n### Deep\n";
        let via_core = headings_from_spans(doc, &spans(doc));
        assert_eq!(via_core, headings(doc));
        assert_eq!(via_core.len(), 3, "three headings: {via_core:?}");
        assert_eq!(via_core[0], Heading { level: 1, text: "Title".into(), line: 0 });
        assert_eq!(via_core[1], Heading { level: 2, text: "Section A".into(), line: 4 });
        assert_eq!(via_core[2], Heading { level: 3, text: "Deep".into(), line: 8 });
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
    fn link_markup_conceals_and_text_stays_content_ink() {
        // `[awl](http://x)`: the `[` and the `](http://x)` tail are the concealable
        // PLUMBING (WYSIWYG `Link`); the visible text `awl` keeps its own content-ink
        // `LinkText` span. The old whole-range dim `Markup` span is gone — off-caret
        // the plumbing hides to zero-width and only the text shows.
        let s = spans("[awl](http://x)");
        let link = MdKind::ConcealMarkup(ConcealKind::Link);
        assert!(has(&s, 0, 1, link), "opening '[' conceals: {s:?}");
        assert!(has(&s, 4, 15, link), "'](url)' tail conceals: {s:?}");
        assert!(has(&s, 1, 4, MdKind::LinkText), "link text stays content ink: {s:?}");
        // The text bytes are NOT covered by any conceal span (so the conceal pass
        // never hides the visible text).
        assert!(
            !s.iter().any(|(r, k)| *k == link && r.start <= 1 && r.end >= 4),
            "link text must not sit under a conceal span: {s:?}"
        );
    }

    #[test]
    fn reference_link_falls_back_to_plain_markup_no_conceal() {
        // A reference-style link (`[text][ref]`) has no `](`, so it keeps the plain
        // NON-concealing `Markup` — never mis-concealed.
        let s = spans("[awl][ref]\n\n[ref]: http://x\n");
        assert!(
            s.iter().any(|(r, k)| r.start == 0 && *k == MdKind::Markup),
            "reference link is plain Markup: {s:?}"
        );
        assert!(
            !s.iter().any(|(_, k)| *k == MdKind::ConcealMarkup(ConcealKind::Link)),
            "no Link conceal span for a reference link: {s:?}"
        );
    }

    #[test]
    fn link_at_returns_url_inside_and_none_outside() {
        // `see [the essay](http://x/y) now`
        //  0123456789...  caret in `essay` (byte ~9) is inside the link.
        let text = "see [the essay](http://x/y) now";
        let inside = text.find("essay").unwrap() + 1; // a byte within the link text
        assert_eq!(link_at(text, inside).as_deref(), Some("http://x/y"));
        // Caret in the leading `see ` prose (byte 1) is OUTSIDE every link.
        assert_eq!(link_at(text, 1), None);
        // Caret in the trailing ` now` prose is outside too.
        assert_eq!(link_at(text, text.find("now").unwrap()), None);
        // A doc with no link at all: always None.
        assert_eq!(link_at("just prose here", 3), None);
    }

    #[test]
    fn blockquote_marker_conceals_text_quote() {
        // The `> ` marker is now a WYSIWYG-concealable `Blockquote` span (not plain
        // `Markup`): dim off-cursor, zero-width off the caret's line — the pull-quote
        // round. The body text keeps its `Quote` styling span.
        let bq = MdKind::ConcealMarkup(ConcealKind::Blockquote);
        let s = spans("> quoted");
        assert!(has(&s, 0, 2, bq), "'> ' marker conceal span: {s:?}");
        assert!(s.iter().any(|(_, k)| *k == MdKind::Quote), "quote text: {s:?}");
    }

    #[test]
    fn multiline_and_nested_quote_markers_conceal() {
        // A two-line blockquote emits ONE `Blockquote` conceal span per line (the
        // per-line `[ \t]*(> ?)+` scan), not one for the whole range — so each line
        // conceals/reveals independently on the caret's line.
        let bq = MdKind::ConcealMarkup(ConcealKind::Blockquote);
        let s = spans("> a\n> b");
        assert!(has(&s, 0, 2, bq), "first line '> ' marker: {s:?}");
        assert!(has(&s, 4, 6, bq), "second line '> ' marker: {s:?}");
        // A nested `>>` conceals its whole leading marker run as one span.
        let s = spans(">> deep");
        assert!(has(&s, 0, 3, bq), "'>> ' nested marker run: {s:?}");
    }

    #[test]
    fn list_marker_dim() {
        let s = spans("- item");
        assert!(has(&s, 0, 2, MdKind::ListMarker), "marker dim: {s:?}");
    }

    #[test]
    fn table_pipes_separator_and_header_spans() {
        //        0      7 9        (line 0 "| a | b |" is 9 bytes incl newline at 9)
        let doc = "| a | b |\n|---|---|\n| c | d |\n";
        let s = spans(doc);
        // Every literal `|` on a data row is a dim TablePipe span. Header row pipes
        // sit at bytes 0, 4, 8.
        assert!(has(&s, 0, 1, MdKind::TablePipe), "leading header pipe: {s:?}");
        assert!(has(&s, 4, 5, MdKind::TablePipe), "middle header pipe: {s:?}");
        assert!(has(&s, 8, 9, MdKind::TablePipe), "trailing header pipe: {s:?}");
        // The separator row (`|---|---|`, bytes 10..19) is ONE dim TableSep span; its
        // pipes are NOT separately emitted as TablePipe.
        assert!(has(&s, 10, 19, MdKind::TableSep), "separator row dim: {s:?}");
        assert!(
            !s.iter().any(|(r, k)| *k == MdKind::TablePipe && r.start >= 10 && r.end <= 19),
            "no TablePipe inside the separator row: {s:?}"
        );
        // The header CELLS get the (no-op, full-ink) TableHeader tag; body cells do not.
        assert!(
            s.iter().any(|(_, k)| *k == MdKind::TableHeader),
            "a header cell is tagged TableHeader: {s:?}"
        );
        // A body-row pipe on line 2 (byte 20) is still a TablePipe.
        assert!(has(&s, 20, 21, MdKind::TablePipe), "body-row pipe: {s:?}");
    }

    #[test]
    fn aligned_separator_colons_dim_whole_row() {
        // A `:--:` / `:---` alignment separator is still recognized as the sep row and
        // dimmed whole (colons included).
        let doc = "| a | b |\n|:--|--:|\n| c | d |\n";
        let s = spans(doc);
        assert!(has(&s, 10, 19, MdKind::TableSep), "aligned separator row dim: {s:?}");
    }

    #[test]
    fn non_table_pipe_in_prose_is_not_table_markup() {
        // A stray `|` in ordinary prose (no separator row => pulldown never rules it a
        // table) is never a TablePipe — we only scan INSIDE a parsed table range.
        let s = spans("a | b is a pipe, not a table\n");
        assert!(
            !s.iter().any(|(_, k)| k.is_table_markup()),
            "a prose pipe is not table markup: {s:?}"
        );
    }

    #[test]
    fn align_table_pads_ragged_and_is_idempotent() {
        // A ragged, messily-spaced GFM table: uneven cell widths, a missing trailing
        // cell on the last row. Align re-pads so every `|` lines up.
        let src = "| Name | Value |\n|---|---|\n| a | 100 |\n| bb |";
        let out = align_table(src);
        let want = "| Name | Value |\n| ---- | ----- |\n| a    | 100   |\n| bb   |       |";
        assert_eq!(out, want, "ragged input aligns + fills the missing cell");
        // IDEMPOTENT: aligning the aligned output is a fixed point.
        assert_eq!(align_table(&out), out, "already-aligned input is unchanged");
    }

    #[test]
    fn align_table_preserves_alignment_markers() {
        // `:---` left, `---:` right, `:--:` center — the colons must survive, and each
        // column is floored to its marker's minimum width (left/right≥2, center≥3), so
        // the one-char cells widen to keep the separator valid.
        let src = "| a | b | c |\n|:--|--:|:-:|\n| 1 | 2 | 3 |";
        let out = align_table(src);
        let want = "| a  | b  | c   |\n| :- | -: | :-: |\n| 1  | 2  | 3   |";
        assert_eq!(out, want, "left/right/center markers preserved: {out}");
        // A wider column keeps the markers at the ENDS, dashes in the middle.
        let src2 = "| xxxx | y | zzz |\n|:--|--:|:-:|\n| 1 | 2 | 3 |";
        let out2 = align_table(src2);
        let want2 = "| xxxx | y  | zzz |\n| :--- | -: | :-: |\n| 1    | 2  | 3   |";
        assert_eq!(out2, want2, "markers hug the ends at width: {out2}");
    }

    #[test]
    fn align_table_uses_display_width_for_cjk() {
        // A CJK cell counts as 2 columns each, so the Latin column pads to match its
        // DISPLAY width, not its byte length (5 bytes for a 2-col wide char would
        // over-pad; the width helper counts it as 2).
        let src = "| 名前 | v |\n|---|---|\n| x | yy |";
        let out = align_table(src);
        // "名前" is 4 display cols; "x" pads to 4; header dashes fill 4.
        let want = "| 名前 | v  |\n| ---- | -- |\n| x    | yy |";
        assert_eq!(out, want, "CJK cell uses display width: {out}");
    }

    #[test]
    fn table_block_lines_finds_the_block_and_needs_a_separator() {
        let text = "intro\n| a | b |\n|---|---|\n| c | d |\n\ntail | pipe";
        let lines: Vec<&str> = text.split('\n').collect();
        // Caret on any of the three table lines (1,2,3) finds the same [1,4) block.
        for caret in 1..=3 {
            assert_eq!(
                table_block_lines(&lines, caret),
                Some((1, 4)),
                "caret on table line {caret} finds the block"
            );
        }
        // Caret on prose (line 0) or the pipe-bearing-but-separator-less tail (line 5)
        // is None — a pipe run with no separator row is never a table.
        assert_eq!(table_block_lines(&lines, 0), None, "prose line is not a table");
        assert_eq!(table_block_lines(&lines, 5), None, "pipe prose w/o sep is not a table");
    }

    #[test]
    fn table_conceal_span_covers_the_whole_block() {
        // The WYSIWYG whole-table conceal span spans the table's exact byte range.
        let text = "| a | b |\n|---|---|\n| c | d |\n";
        let s = spans(text);
        let table_end = "| a | b |\n|---|---|\n| c | d |".len();
        assert!(
            s.iter().any(|(r, k)| *k == MdKind::ConcealMarkup(ConcealKind::Table)
                && r.start == 0
                && r.end >= table_end),
            "whole-table conceal span present: {s:?}"
        );
    }

    #[test]
    fn table_column_layout_fits_and_clamps() {
        // Fits: natural widths preserved, left-anchored, gaps applied.
        let (xs, ws) = table_column_layout(&[100.0, 60.0, 40.0], 10.0, 1000.0);
        assert_eq!(ws, vec![100.0, 60.0, 40.0], "fitting keeps natural widths");
        assert_eq!(xs[0], 0.0);
        assert!((xs[1] - 110.0).abs() < 1e-3, "col1 = 100 + gap 10");
        assert!((xs[2] - 180.0).abs() < 1e-3, "col2 = 110 + 60 + gap 10");
        // Overflow: total (200 + 3*10 = 230) scaled to avail 115 => scale 0.5.
        let (xs, ws) = table_column_layout(&[100.0, 100.0], 30.0, 115.0);
        assert!((ws[0] - 50.0).abs() < 1e-3, "col0 scaled by 0.5");
        assert!((ws[1] - 50.0).abs() < 1e-3, "col1 scaled by 0.5");
        assert!((xs[1] - 65.0).abs() < 1e-3, "col1 x = 50 + gap*0.5 (15)");
        // The laid grid never exceeds `avail`.
        let right = xs[1] + ws[1];
        assert!(right <= 115.0 + 1e-3, "clamped grid fits avail: {right}");
        // Empty input is inert.
        assert_eq!(table_column_layout(&[], 10.0, 100.0), (vec![], vec![]));
    }

    #[test]
    fn table_align_offset_honors_alignment_and_clamps_overflow() {
        let pad = 4.0;
        let col = 100.0;
        let cell = 20.0;
        assert!((table_align_offset(ColAlign::Left, col, cell, pad) - pad).abs() < 1e-3);
        assert!((table_align_offset(ColAlign::None, col, cell, pad) - pad).abs() < 1e-3);
        // Right: 100 - 20 - 4 = 76.
        assert!((table_align_offset(ColAlign::Right, col, cell, pad) - 76.0).abs() < 1e-3);
        // Center: (100 - 20)/2 = 40.
        assert!((table_align_offset(ColAlign::Center, col, cell, pad) - 40.0).abs() < 1e-3);
        // Over-wide cell (wider than its column): every alignment left-anchors at pad.
        for a in [ColAlign::Left, ColAlign::Right, ColAlign::Center] {
            let off = table_align_offset(a, col, 200.0, pad);
            assert!((off - pad).abs() < 1e-3, "over-wide {a:?} clamps to pad: {off}");
        }
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
        // The ornament scale is PER-WORLD now (`theme::Theme::ornament_scale`), no
        // longer a single `type_scale` rung; its own tiers + row-coupling are asserted
        // in `theme::tests` and `render::tests` (see `every_world_has_an_ornament_scale`
        // + `md_line_scale_grows_thematic_break_rows_to_the_active_worlds_ornament_scale`).
        // The tallest ornate tier (2.2) still reads bigger than h1.
        assert!(
            crate::theme::ORNAMENT_SCALE_ORNATE > heading_scale(1),
            "the ornate ornament reads bigger than h1"
        );
    }

    #[test]
    fn is_thematic_break_matches_commonmark_breaks_only() {
        // The three break syntaxes, bare and spaced/indented, all qualify.
        assert!(is_thematic_break("---"));
        assert!(is_thematic_break("***"));
        assert!(is_thematic_break("___"));
        assert!(is_thematic_break("- - -"), "spaced dashes are a break");
        assert!(is_thematic_break("   ---"), "up-to-3 indent still a break");
        assert!(is_thematic_break("*****"), "5 stars still a break");
        // NOT breaks: too few, mixed run chars, or any other content on the line.
        assert!(!is_thematic_break("--"), "two dashes is not a break");
        assert!(!is_thematic_break("-*-"), "mixed run chars are not a break");
        assert!(!is_thematic_break("- item"), "a list item is not a break");
        assert!(!is_thematic_break("# heading"), "a heading is not a break");
        assert!(!is_thematic_break("plain prose"), "prose is not a break");
        assert!(!is_thematic_break(""), "empty line is not a break");
    }
}
