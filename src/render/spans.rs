//! SPAN / ATTRS LAYERING — how one buffer line's `AttrsList` is assembled from
//! the base doc attrs plus every styling layer (markdown, syntax, CJK family,
//! heading SIZE scale, reveal-on-cursor conceal).
//!
//! Unlike the caret geometry next door (which is inherent methods on
//! [`super::TextPipeline`] because it reads the live font/layout/metrics state),
//! every helper here is a PURE free function: it takes the line text, the base
//! [`Attrs`], the parsed span lists, and the resolved CJK `(family, weight)` as
//! explicit params and returns the layered attrs — no `&self`, no GPU state. That
//! is exactly why this cluster lifts out of `render.rs` cleanly: the pipeline
//! methods that drive shaping ([`super::TextPipeline::set_text_incremental`] /
//! [`super::TextPipeline::restyle_all_lines`] / the caret-driven
//! [`super::TextPipeline::refresh_rule_conceal`]) all funnel through the SINGLE
//! recipe [`build_line_attrs`], which orders the layers (heading scale → markdown
//! spans → syntax spans → CJK family spans → symbol spans → conceal). The bodies
//! are carved out of `render.rs` VERBATIM, so the capture output is byte-identical.
//!
//! `use super::*;` pulls in the parent's `glyphon` re-exports (`Attrs`, `Family`,
//! `GlyphMetrics`), the `theme` alias, and the sibling free helpers these reuse
//! (`lerp_srgb`); a child module sees its ancestor's private items, so the layer
//! recipe keeps working with NO behaviour change. Re-exported via `use spans::*`
//! in `render.rs` so the unqualified call sites — and the in-module tests — keep
//! resolving these by their bare names.

use super::*;

/// True for scalar values that should shape in the per-theme CJK (Japanese)
/// fallback face rather than the world's Latin display face. Covers the Japanese
/// core (Hiragana, Katakana + phonetic extensions, CJK Unified Ideographs + Ext A,
/// compatibility ideographs) plus the shared CJK symbols/punctuation and
/// full-/half-width forms that read as Japanese in running text. This is a
/// deliberately broad "is this a CJK glyph" test, not a precise script split — it
/// only decides which family a run is *offered* to first; cosmic-text still does
/// the real per-glyph resolution.
pub(super) fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3000..=0x303F   // CJK symbols & punctuation (、。「」…)
        | 0x3040..=0x309F // Hiragana
        | 0x30A0..=0x30FF // Katakana
        | 0x31F0..=0x31FF // Katakana phonetic extensions
        | 0x3400..=0x4DBF // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs
        | 0xF900..=0xFAFF // CJK Compatibility Ideographs
        | 0xFF00..=0xFFEF // Halfwidth & Fullwidth Forms
    )
}

/// Maximal contiguous byte ranges of [`is_cjk`] scalar values within `text`.
/// Used to lay per-theme CJK family spans over a shaped line so Japanese resolves
/// to the world-matching mincho/gothic face (see [`add_cjk_spans`]). Byte indices
/// are valid `char` boundaries (from `char_indices`), so the ranges are safe to
/// hand to `AttrsList::add_span`.
pub(super) fn cjk_runs(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut runs = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if is_cjk(c) {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            runs.push(s..i);
        }
    }
    if let Some(s) = start.take() {
        runs.push(s..text.len());
    }
    runs
}

/// i18n: lay PER-SCRIPT family spans over `al` for every CJK-family run in
/// `text` — the render wiring for `crate::script`'s classifier + ladder,
/// generalizing what used to be a single ja-only CJK family span into an
/// independent [`theme::FontId`] resolution per run. Walks
/// [`crate::script::script_runs`] (kana / hangul / bopomofo / han, each
/// named) and resolves EACH run's [`theme::FontId`] via
/// [`crate::script::resolve_font_id`]'s ladder — (a) the document's own
/// frontmatter `lang:` tag, if compatible with the run's script; (b) else the
/// script's own unambiguous mapping; (c) else (a Han run with no compatible
/// tag) the `cjk_priority` tiebreak; (d) else no override at all (a
/// `FontId::Latin` result, or a script whose ladder resolved to nothing on
/// this machine — `fonts.get` returns `None` either way, so the base doc face
/// wins — the same degenerate fallback the old single-script version had).
/// `fonts` is [`super::text::ScriptFonts`], resolved ONCE per reshape by
/// [`TextPipeline::resolve_script_fonts`] — this function does no font-DB
/// work itself, just the per-run ladder + span laying.
///
/// WEIGHT + STYLE PIN (bold/italic-breaks-Japanese fix): each per-script span
/// PINS the run's weight AND style to the resolved face's REGISTERED values —
/// `.weight(wt)` (the concrete weight nearest 400 the font DB has for that
/// family) and `.style(Normal)`. Every bundled CJK face
/// ([`crate::render::FONT_CJK_FACES`] / [`FONT_ZH_KO_FACES`]) registers ONLY at
/// Regular/400/Normal — there is no bold or italic CJK cut in v1 — so pinning is
/// exactly "the resolved face's registered values", never a guess. This layer
/// runs LAST over the markdown layer in [`build_line_attrs`] (script spans UNDER
/// nothing that re-weights a CJK run), and `AttrsList::add_span` REPLACES the
/// whole run range, so a `**bold**` (Weight 700) / `*italic*` (Style::Italic)
/// markdown span sitting under a CJK run is overwritten on exactly those bytes:
/// Japanese inside emphasis keeps its correct per-world face instead of dropping
/// it (cosmic-text's fallback keeps only `weight_diff == 0` + style-matching
/// faces — a 700/italic request would drop the 400/Normal bundled JP face and
/// tofu/system-fall mid-sentence). The pin derives from `base` (the plain doc
/// attrs, already Normal), so even a styled base can never leak a synthetic
/// slant/weight onto a CJK run. The emphasis still reads — via the revealed
/// `**`/`*` markers on the caret's line and the surrounding Latin styling.
///
/// LOGGED TASTE CALL: NO synthetic bold/italic for CJK in v1 — a CJK run in a
/// `**bold**`/`*italic*` span renders at the bundled face's own Regular weight,
/// upright, rather than letting glyphon synthesize an oblique or drop to a
/// heavier fallback. A future real JP/zh/ko bold-or-italic bundled face would
/// lift this clamp (resolve the emphasis to that cut instead of pinning Normal).
pub(super) fn add_script_spans(
    al: &mut glyphon::cosmic_text::AttrsList,
    text: &str,
    base: &Attrs,
    doc_lang: Option<crate::frontmatter::Lang>,
    cjk_priority: &[crate::frontmatter::Lang],
    fonts: &super::text::ScriptFonts,
) {
    for (run, script) in crate::script::script_runs(text) {
        let id = crate::script::resolve_font_id(doc_lang, Some(script), cjk_priority);
        let Some((fam, wt)) = fonts.get(id) else { continue };
        let a = base
            .clone()
            .family(Family::Name(fam))
            .weight(wt)
            .style(glyphon::Style::Normal);
        al.add_span(run, &a);
    }
}

/// True for the SYMBOL / ORNAMENT codepoints the bundled mono + proportional
/// display faces lack — the macOS modifier glyphs (⌘ ⇧ ⌥ ⌃), the key-hint keycaps
/// (↵ Return, ⇥ Tab), the fine-press ornaments / fleurons (❧ ❦ ☙ ❡ ❥), the
/// asterism (⁂), and the reference marks (§ † ‡). These render as TOFU under the
/// global fallback (IBM Plex Mono Light), so the renderer overlays the bundled
/// [`SYMBOL_FAMILY`] face on their runs (see [`add_symbol_spans`]). Exactly the
/// glyph set bundled in `AwlSymbols.ttf`; keep the two in sync.
pub(super) fn is_symbol(c: char) -> bool {
    matches!(c as u32,
        0x2318   // ⌘ Command
        | 0x21E7 // ⇧ Shift
        | 0x2325 // ⌥ Option
        | 0x2303 // ⌃ Control
        | 0x21B5 // ↵ Downwards arrow with corner leftwards (Return / Enter)
        | 0x21E5 // ⇥ Rightwards arrow to bar (Tab)
        | 0x2767 // ❧ Rotated floral heart (fleuron — the hr ornament)
        | 0x2766 // ❦ Floral heart (the `___` break ornament)
        | 0x2619 // ☙ Reversed rotated floral heart (fleuron variant)
        | 0x2761 // ❡ Curved stem paragraph sign ornament
        | 0x2765 // ❥ Rotated heavy black heart bullet (fleuron variant)
        | 0x2042 // ⁂ Asterism
        | 0x00A7 // § Section sign
        | 0x2020 // † Dagger
        | 0x2021 // ‡ Double dagger
    )
}

/// Maximal contiguous byte ranges of [`is_symbol`] scalar values within `text`,
/// mirroring [`cjk_runs`]. Byte indices are valid `char` boundaries (from
/// `char_indices`), so the ranges are safe to hand to `AttrsList::add_span`.
pub(super) fn symbol_runs(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut runs = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if is_symbol(c) {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            runs.push(s..i);
        }
    }
    if let Some(s) = start.take() {
        runs.push(s..text.len());
    }
    runs
}

/// THE ONE OWNER of the chrome symbol-split PUSH loop. Append `text` onto `spans`
/// as alternating non-symbol / [`is_symbol`] runs (via [`symbol_runs`]): every
/// symbol run takes `sym`'s attrs (the bundled [`SYMBOL_FAMILY`] face — real,
/// finite advances for the macOS modifier glyphs ⌘ ⇧ ⌥ ⌃ and keycap ornaments
/// ↵ ⇥ …, which the display/mono faces render as tofu), every other run takes
/// `plain`'s. The overlay foot hint, the keybindings-tips footer, the inline
/// trailing shortcut, and the right-aligned chord column all shared this loop
/// verbatim (the C2 footer round's `push_overlay_hint_spans` was the first copy);
/// they now route through here so a symbol-split can never drift between them.
/// A symbol-free `text` pushes exactly ONE `plain` span — byte-identical to a bare
/// `spans.push((text, plain()))`. The attrs come from CLOSURES so each caller keeps
/// its own color / metrics without this owner knowing them. Does NOT emit any line
/// break: a caller that wants the run on its own line pushes the `"\n"` itself.
pub(super) fn push_symbol_split<'a>(
    spans: &mut Vec<(&'a str, Attrs<'a>)>,
    text: &'a str,
    plain: impl Fn() -> Attrs<'a>,
    sym: impl Fn() -> Attrs<'a>,
) {
    let mut last = 0usize;
    for run in symbol_runs(text) {
        if run.start > last {
            spans.push((&text[last..run.start], plain()));
        }
        let end = run.end;
        spans.push((&text[run], sym()));
        last = end;
    }
    if last < text.len() {
        spans.push((&text[last..], plain()));
    }
}

/// Lay [`SYMBOL_FAMILY`] family spans over `al` for every [`is_symbol`] run in
/// `text`, mirroring [`add_cjk_spans`]. The span inherits `base` (the doc/colored
/// attrs — color, metrics, etc.) but overrides the family to the bundled symbol
/// face so the modifier glyphs + ornaments shape from it instead of the display
/// face's tofu/fallback. Applied LAST in [`build_line_attrs`] so symbol runs win
/// the family on exactly those codepoints, leaving every other glyph in the
/// world's display face. The bundled face is Regular/400, so no weight trap (unlike
/// the CJK face); a default-weight name matches. No-op when `text` has no symbols,
/// keeping symbol-free lines byte-identical.
pub(super) fn add_symbol_spans(
    al: &mut glyphon::cosmic_text::AttrsList,
    text: &str,
    base: &Attrs,
) {
    let runs = symbol_runs(text);
    if runs.is_empty() {
        return;
    }
    let a = base.clone().family(Family::Name(SYMBOL_FAMILY));
    for run in runs {
        al.add_span(run, &a);
    }
}

/// TASTE FLAG: does blockquote BODY text ([`crate::markdown::MdKind::Quote`]) step
/// one ink rung down to `muted` (`true` — the calm, set-apart voice, the DEFAULT
/// and the pre-flag behaviour) or ride the full content ink (`false` — a louder
/// quote)? Default `true`; the dev-only `AWL_QUOTE_FULL_INK` env var flips it to
/// full ink for the pull-quote round's A/B review capture WITHOUT a rebuild (a
/// total no-op unless set, mirroring `render::apply_cjk_force` /
/// `chrome::outline`'s `AWL_OUTLINE_REVEAL`). Cached once (this is called per span
/// per line — hot), so determinism holds: a default capture (env unset) is
/// byte-identical to before the flag existed.
const QUOTE_TEXT_DIM: bool = true;

/// Whether blockquote body text dims — the [`QUOTE_TEXT_DIM`] const AND the absence
/// of the dev-only `AWL_QUOTE_FULL_INK` override (set → full ink). Read once via a
/// `OnceLock` so the per-span hot path never re-reads the environment.
fn quote_text_dim() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| QUOTE_TEXT_DIM && std::env::var_os("AWL_QUOTE_FULL_INK").is_none())
}

/// Build the concrete `Attrs` for one markdown span kind, transforming `base`
/// (the doc attrs — family, ligature features, etc.):
/// - `Markup`/`ConcealMarkup`/`ListMarker`/`Rule` → recede to the DIM ink (syntax +
///   quiet text); a `Rule` row also gets a thin centered quad drawn over it. `Quote`
///   (blockquote body) dims too BY DEFAULT — a taste flag, see [`quote_text_dim`].
///   `ConcealMarkup` additionally hides off the caret's line/block — see
///   [`add_wysiwyg_conceal_spans`], applied as a later layer over this one.
/// - `Heading` → no transform; reads by SIZE alone (set per-line upstream).
/// - `Task(true)`/`TaskDone` → DIM (a completed todo recedes as one); `Task(false)`
///   (an OPEN checkbox) rides the full default ink so the box stays present.
/// - `Bold`/`Italic`/`BoldItalic` → weight / style; NO color, so they ride the
///   buffer's default ink (full when focus off, dim when focus dims the region).
/// - `Code` → the registered monospace family + a subtle tint toward MUTED ink.
/// - `LinkText` → the buffer's full CONTENT ink (it lifts off the dim `Markup`
///   span; DESIGN §3 keeps `primary`/amber for the caret alone).
pub(super) fn md_attrs(
    base: &Attrs<'static>,
    kind: crate::markdown::MdKind,
) -> Attrs<'static> {
    use crate::markdown::MdKind;
    let th = theme::active();
    let dim = th.muted.to_glyphon();
    let mut a = base.clone();
    let mut natural: Option<glyphon::Color> = None;
    match kind {
        // Syntax + quiet text recede to the dim ink. A CHECKED checkbox + a checked
        // task's body join them: a completed todo recedes as one (figure/ground by
        // value), while an OPEN checkbox stays present below.
        MdKind::Markup
        | MdKind::ConcealMarkup(_)
        | MdKind::ListMarker
        | MdKind::Rule
        | MdKind::Task(true)
        | MdKind::TaskDone
        // A table's `|` pipes + its `|---|` separator row are structural markup:
        // they recede to the dim ink like every other syntax character (awl shows
        // the table as styled SOURCE, never a drawn grid).
        | MdKind::TablePipe
        | MdKind::TableSep => {
            natural = Some(dim);
        }
        MdKind::TableHeader => {
            // No-op transform (like `Heading`/`Highlight`): a header cell rides the
            // buffer's full CONTENT ink — figure/ground by value, no accent, amber is
            // the caret's alone (DESIGN §3). Body cells get the same full ink with no
            // span, so this only exists to tag the header in the sidecar.
        }
        MdKind::Task(false) => {
            // An OPEN checkbox rides the buffer's FULL default ink so the empty box
            // reads as a present, actionable marker — one value step above the dim
            // `- ` bullet before it. No accent (amber is the caret's alone).
        }
        MdKind::Quote => {
            // Blockquote BODY text. A TASTE FLAG ([`quote_text_dim`]) decides whether
            // it steps one ink rung down to `muted` (the calmer, set-apart voice —
            // the DEFAULT + the pre-flag behaviour, byte-identical) or rides the full
            // content ink (a louder quote). Never amber (DESIGN §3). The dim path is
            // the historical default; the full-ink path exists for the A/B capture.
            if quote_text_dim() {
                natural = Some(dim);
            }
        }
        MdKind::Heading(level) => {
            // SIZE comes per-LINE upstream ([`scaled_base_attrs`], already in `base`,
            // the Ladder-J rungs); WEIGHT is the per-world ONE BIT
            // ([`crate::theme::Theme::heading_bold`]) composed through THE one owner
            // [`crate::markdown::heading_weight_bold`] — SECTION (`##`) and SUBHEAD
            // (`###`+) shape at real `Weight::BOLD` where the world's bit grants it
            // (the same bundled same-family 700 request the `MdKind::Bold` arm below
            // makes — a real weight change, never synthetic), while the TITLE (`#`)
            // NEVER bolds, on any world (it spends pure size, 1.6x). We still
            // deliberately do NOT set COLOR: DESIGN.md §3 — `primary` (amber) is the
            // caret and ONLY the caret; figure/ground stays VALUE + size + (per-world)
            // weight, never the accent. Data through the one renderer — no world is
            // named here (`theme_caps_law`).
            if crate::markdown::heading_weight_bold(th.heading_bold, level) {
                a = a.weight(glyphon::Weight::BOLD);
            }
        }
        MdKind::Bold => {
            // Resolves to the world's real bundled BOLD (700) face — for EVERY world,
            // proportional AND mono. Each display family ships a 700 companion under
            // the SAME family name as its Regular (`FONT_THEME_BOLD_FACES`), so this
            // plain `Weight::BOLD` request matches `weight_diff == 0` and lands on the
            // bold FILE. The five mono-display worlds (Tawny = IBM Plex Mono, Mangrove
            // = JetBrains Mono, Firetail/Potoroo = Monaspace Xenon, Currawong =
            // Iosevka) used to have no 700, so this request tripped the trap and fell
            // into a FOREIGN proportional sans (the "weird fi-ligature" bug); the mono
            // bolds keep the fixed grid AND give true emphasis.
            a = a.weight(glyphon::Weight::BOLD);
        }
        MdKind::Italic => {
            a = a.style(glyphon::Style::Italic);
        }
        MdKind::BoldItalic => {
            // Same as `Bold` above (real bundled 700 on every world, proportional AND
            // mono) plus glyphon's synthesized slant (no bundled italic face).
            a = a.weight(glyphon::Weight::BOLD).style(glyphon::Style::Italic);
        }
        MdKind::Code { .. } => {
            a = a.family(Family::Monospace);
            // A subtle tint toward the MUTED ink so inline/fenced code reads as a
            // distinct surface even where mono ≈ the body face (the mono worlds).
            // Never amber — this rides the same base_content→muted ramp as the
            // Alabaster syntax roles (DESIGN §3: `primary` is the caret's alone).
            natural = Some(lerp_srgb(th.base_content, th.muted, 0.28).to_glyphon());
        }
        MdKind::CodeSyntax { role, .. } => {
            // A highlighted byte of a recognized fenced block: KEEP the mono family
            // (like `Code`) but take the syntax ROLE COLOR instead of the flat tint,
            // so the fence body reads as Alabaster-highlighted code in mono. The role
            // color comes from the SAME single derivation the code-buffer pass uses
            // ([`role_style_for`], THE role style provider), so a fence and a `.rs`
            // file highlight identically — prose comments prominent, commented-out
            // code muted, tints per world — and the syntax role WINS the flat Code
            // tint for these bytes because this span is laid AFTER the body `Code`
            // span (last-wins on overlap). The role's wash (if any) rides the wash
            // pipelines via the same md-span source (see `rects.rs::wash_rects`).
            a = a.family(Family::Monospace);
            natural = Some(role_style_for(&theme::active(), role).fg.to_glyphon());
        }
        MdKind::LinkText => {
            // Link TEXT reads in the buffer's full CONTENT ink. It sits OVER the
            // whole-range dim `Markup` span (brackets + url), so it must set content
            // EXPLICITLY to lift back off that dim — the link then reads by its muted
            // []()-markup, not by spending the accent. DESIGN §3: `primary` (amber) is
            // the caret and ONLY the caret.
            natural = Some(th.base_content.to_glyphon());
        }
        MdKind::Highlight => {
            // No-op transform, like `Heading`: `==marked==` text rides the buffer's
            // full default ink (it may sit OVER a dimmer context span — e.g. inside a
            // blockquote — and, like `LinkText`, is pushed AFTER that span so it lifts
            // back to full ink). The highlighter identity is carried entirely by the
            // WASH quad drawn behind it (`rects.rs::ensure_wash_protos`'s dedicated
            // `Highlight` bucket → its own per-world [`highlight_wash`] tint/pipeline,
            // a split-complementary of the world's accent, DECOUPLED from the warm
            // comment wash so it POPS), never a text color change. Never amber (DESIGN §3).
        }
        MdKind::Strikethrough => {
            // `~~struck~~` content RECEDES to the strike ink — [`strike_ink`], THE
            // one owner the drawn strike LINE shares (`rects.rs`'s strike bucket +
            // the format popover's `S` button), so text and line can never disagree
            // on the register. The same muted rung the writer's-diff deletions
            // recede to (their blockquoted form), never amber (DESIGN §3). The
            // LINE itself is a quad (`strike_lines`), not a text transform.
            natural = Some(strike_ink(&th).to_glyphon());
        }
    }
    if let Some(c) = natural {
        a = a.color(c);
    }
    a
}

/// Lay the markdown styling spans that intersect ONE buffer line over `al`. Maps
/// each document-byte span in `md_spans` into this line's local byte range
/// (`line_doc_start` is the line's first byte in the document) and adds it with
/// [`md_attrs`]. Spans are applied in their stored order so the intentional
/// link/code-block overlaps (whole-range dim, then inner content) resolve
/// correctly. No-op when `md_spans` is empty (non-markdown buffers), keeping their
/// render byte-identical.
pub(super) fn add_md_line_spans(
    al: &mut glyphon::cosmic_text::AttrsList,
    line_text: &str,
    line_doc_start: usize,
    base: &Attrs<'static>,
    md_spans: &[(std::ops::Range<usize>, crate::markdown::MdKind)],
) {
    add_line_spans(al, line_text, line_doc_start, base, md_spans, md_attrs);
}

/// Shared body of [`add_md_line_spans`] / [`add_syn_line_spans`]: lay the document-
/// byte spans that intersect ONE buffer line over `al`, clamping each into the
/// line's local byte range (`line_doc_start` is the line's first byte) and adding
/// it with `attrs_fn`. Spans are applied in their stored order so intentional
/// overlaps (whole-range dim, then inner content) resolve correctly. No-op when
/// `spans` is empty, keeping non-styled buffers byte-identical.
pub(super) fn add_line_spans<K: Copy>(
    al: &mut glyphon::cosmic_text::AttrsList,
    line_text: &str,
    line_doc_start: usize,
    base: &Attrs<'static>,
    spans: &[(std::ops::Range<usize>, K)],
    attrs_fn: impl Fn(&Attrs<'static>, K) -> Attrs<'static>,
) {
    if spans.is_empty() {
        return;
    }
    let line_end = line_doc_start + line_text.len();
    for (r, kind) in spans {
        let lo = r.start.max(line_doc_start);
        let hi = r.end.min(line_end);
        if lo < hi {
            let local = (lo - line_doc_start)..(hi - line_doc_start);
            al.add_span(local, &attrs_fn(base, *kind));
        }
    }
}

/// The fully-transparent ink used to CONCEAL a markdown horizontal-rule line's raw
/// `---`/`***`/`___` glyphs (see [`add_rule_conceal_span`]). Alpha 0, so the dashes
/// still SHAPE (the row keeps its height and the byte offsets stay editable) but draw
/// invisibly — leaving the centered `hr_ornament` fleuron as the only mark a reader
/// sees. An hr line is pure MARKUP with no content, so when the caret is elsewhere we
/// "show the content" by showing nothing but the fine-press break.
pub(super) const RULE_CONCEAL_COLOR: glyphon::Color = glyphon::Color::rgba(0, 0, 0, 0);

/// WYSIWYG v1.1 — TRUE ZERO-WIDTH conceal (the live-review headline fix). v1
/// hid a `ConcealMarkup` span with transparent ink alone, which kept the
/// marker glyphs' natural ADVANCE: a concealed `"## "` still indented the
/// heading off the column edge, and concealed `"**"`/`"*"` left a visible
/// word-gap ("almost  italics"). The cure rides the SAME per-span `AttrsList`
/// seam CJK/syntax/markdown already use: `Attrs::metrics` lets ONE byte-range
/// override its font size independent of the rest of the line
/// ([`scaled_base_attrs`] already proved this for headings, just per-LINE
/// instead of per-span). cosmic-text computes a glyph's pixel advance as
/// `metrics_opt.font_size * glyph.x_advance` at LAYOUT time — shaping itself
/// (kerning/ligatures/clustering) happens earlier and is UNAFFECTED, because
/// `Attrs::compatible` (the run-splitting test) checks family/stretch/style/
/// weight only, never `metrics_opt` — so a concealed run shapes seamlessly
/// alongside its visible neighbors and only its FINAL on-screen width
/// collapses. A near-zero (not exactly `0.0`, defensively) font size shrinks
/// the advance to sub-pixel — true zero-width — while glyphon already
/// tolerates a zero-size rasterized glyph bitmap (`width == 0 || height ==
/// 0`, `text_render.rs`), so nothing panics; the alpha-0 color means nothing
/// would draw regardless. The paired `Attrs::metrics` line-height half MUST
/// be set to the line's own (already heading-scaled) row height, never a
/// small value — cosmic-text keys a visual row's height off the MAX
/// `line_height_opt` across every glyph on the row, but only among glyphs
/// that carry an EXPLICIT override; a stray small value here would apply
/// even when every other glyph on the row has none, shrinking the WHOLE row
/// rather than "staying keyed to the surviving glyphs" (see
/// [`add_wysiwyg_conceal_spans`]'s caller in [`build_line_attrs`], which
/// threads the line's real scaled height through). Hit-testing / caret
/// placement need no new logic: `col_in_run`/`col_in_row` (`geometry.rs`)
/// already walk glyphs sequentially comparing midpoints, so several
/// near-coincident zero-width x boundaries just resolve to the nearest one
/// in sequence — no panic, no infinite loop, a valid (if visually ambiguous)
/// byte column.
const CONCEAL_ZERO_WIDTH_FONT_SIZE: f32 = 0.01;

/// INLINE IMAGES: the number of body line-heights of row to reserve for an image
/// whose file is MISSING/unreadable (no intrinsic dimensions to fit-to-column).
/// A modest placeholder box; the placeholder GLYPH (a broken-image mark) is the
/// next phase — this only reserves the row so the layout is complete. TUNABLE.
#[cfg(not(target_arch = "wasm32"))]
pub(super) const IMAGE_MISSING_ROW_LINES: f32 = 3.0;

/// INLINE IMAGES — the fraction of the window's HEIGHT an image's DISPLAY size may
/// occupy at most. Guards the "retina screenshot as a full-bleed wall" case: a
/// paste stamps no width (fit-to-column governs its display width), but a very
/// TALL native image can still fit the column width while towering over the whole
/// viewport. Applied on top of (never instead of) the fit-to-column width — see
/// [`image_display_size`]. RENDER-ONLY: the cap shrinks the DRAWN size, never the
/// file on disk or the `|NNN` hint written by a user's own drag-resize gesture.
///
/// TASTE TUNABLE (flagged for live review): the fraction itself.
pub(super) const IMAGE_MAX_VIEWPORT_FRAC: f32 = 0.65;

/// INLINE IMAGES — the pure fit-to-column, viewport-capped display size (px) for
/// one image. `display_w = min(desired, wrap_width)` where `desired` is the
/// `|NNN` width HINT if present, else the image's intrinsic width; `display_h`
/// preserves the intrinsic aspect (`display_w * intrinsic_h / intrinsic_w`). Never
/// wider than the text column (so an image always fits) and never zero (a 1px
/// floor). Then, if the resulting height exceeds `max_h` (the caller's
/// [`IMAGE_MAX_VIEWPORT_FRAC`]-scaled window height), BOTH dimensions shrink
/// proportionally so the height lands exactly at `max_h` — the aspect never
/// distorts, only the whole image scales down. A non-positive `max_h` (e.g. the
/// window height isn't known yet) disables the cap outright. Pure + total, so a
/// headless capture reserves the identical row a live frame does.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn image_display_size(
    intrinsic_w: u32,
    intrinsic_h: u32,
    width_hint: Option<u32>,
    wrap_width: f32,
    max_h: f32,
) -> (f32, f32) {
    let iw = (intrinsic_w.max(1)) as f32;
    let ih = (intrinsic_h.max(1)) as f32;
    let desired = width_hint.map(|h| h as f32).unwrap_or(iw);
    let w = desired.min(wrap_width.max(1.0)).max(1.0);
    let h = (w * ih / iw).max(1.0);
    if max_h > 0.0 && h > max_h {
        let scale = max_h / h;
        ((w * scale).max(1.0), max_h)
    } else {
        (w, h)
    }
}

/// REVEAL-ON-CURSOR concealment for a markdown horizontal rule: overlay the
/// [`RULE_CONCEAL_COLOR`] (transparent) ink over any `Rule` span's bytes that fall on
/// THIS line, hiding the literal `---` glyphs so the line reads as a clean centered
/// fleuron (the ornament layer draws it on the SAME rows — see
/// [`super::TextPipeline::rule_lines`]). Applied LAST in [`build_line_attrs`] so the
/// transparent ink wins over the `Rule` span's dim markup color. The caller gates
/// this on `conceal_rule` (true only when the caret is on a DIFFERENT line); when the
/// caret IS on the hr line the dashes are left in their dim markup color — REVEALED,
/// fully editable, and the fleuron yields to them. No-op when no `Rule` span
/// intersects the line (non-hr / non-markdown), keeping those lines byte-identical.
pub(super) fn add_rule_conceal_span(
    al: &mut glyphon::cosmic_text::AttrsList,
    line_text: &str,
    line_doc_start: usize,
    base: &Attrs<'static>,
    md_spans: &[(std::ops::Range<usize>, crate::markdown::MdKind)],
) {
    let line_end = line_doc_start + line_text.len();
    let hidden = base.clone().color(RULE_CONCEAL_COLOR);
    for (r, kind) in md_spans {
        if *kind != crate::markdown::MdKind::Rule {
            continue;
        }
        let lo = r.start.max(line_doc_start);
        let hi = r.end.min(line_end);
        if lo < hi {
            al.add_span((lo - line_doc_start)..(hi - line_doc_start), &hidden);
        }
    }
}

/// REVEAL-ON-CURSOR concealment for an unordered-list BULLET, mirroring
/// [`add_rule_conceal_span`]: overlay the transparent [`RULE_CONCEAL_COLOR`] ink over
/// the single raw marker CHARACTER (`-`/`*`/`+`) of a bullet line, so the line reads
/// with its depth-derived glyph (the active world's own [`crate::theme::Theme::bullets`]
/// pair, drawn as an ornament on the SAME row — see
/// [`super::TextPipeline::bullet_marks`]) instead of the raw dash. The marker's
/// trailing space is left alone, so the concealed dash keeps its cell and the content
/// stays put — the glyph simply draws where the dash was. Detected per-line via the
/// SHARED [`crate::markdown::list_item`] (only UNORDERED items conceal; an ordered
/// `1.` keeps its number). The caller gates this on the caret being on a DIFFERENT
/// line (the same gate as the rule conceal); when the caret IS on the line the raw
/// marker reveals (dim, editable) and no glyph is drawn. No-op for non-list lines,
/// keeping them byte-identical.
pub(super) fn add_bullet_conceal_span(
    al: &mut glyphon::cosmic_text::AttrsList,
    line_text: &str,
    base: &Attrs<'static>,
) {
    let Some(it) = crate::markdown::list_item(line_text) else {
        return;
    };
    if it.ordered {
        return;
    }
    // The marker char sits at byte `it.indent` (the indent is spaces); conceal just it.
    let hidden = base.clone().color(RULE_CONCEAL_COLOR);
    al.add_span(it.indent..it.indent + 1, &hidden);
}

/// THE reveal decision for ONE `ConcealMarkup` span — the single rule shared by
/// [`add_wysiwyg_conceal_spans`] (the renderer) and
/// [`super::TextPipeline::wysiwyg_report`] (the capture sidecar), so the two can
/// never drift on what "concealed" means. `range` is the span's own document
/// byte range; `conceal_off_cursor` is true when the caret is on a DIFFERENT
/// line than the span's own (irrelevant for the BLOCK-scoped kinds);
/// `cursor_byte` is the document byte offset of the caret's own line's first
/// byte.
pub(super) fn wysiwyg_reveals(
    ck: crate::markdown::ConcealKind,
    conceal_off_cursor: bool,
    cursor_byte: usize,
    range: &std::ops::Range<usize>,
) -> bool {
    use crate::markdown::ConcealKind;
    match ck {
        // BLOCK-scoped: reveal iff the caret's line sits anywhere in the block.
        // A frontmatter block reuses the exact `Fence` rule (it has no body
        // sub-span to carve out, so the whole range conceals/reveals as one).
        ConcealKind::Fence | ConcealKind::Frontmatter => range.contains(&cursor_byte),
        // A TABLE's source NEVER un-conceals in place — THE X-RAY (the user's
        // canonized metaphor): the drawn GRID stays put so the document never
        // reflows during a keyboard walk, and the caret's own row floats its raw
        // source as one non-wrapping line OVER the dimmed grid cells
        // (`prepare_table_xray`), never by growing the document rows. So the source
        // rows stay zero-width (concealed) at every caret position — the float and
        // the caret redirect (`col_x_and_advance`) do the reveal, not this in-place
        // un-conceal.
        ConcealKind::Table => false,
        // LINE-scoped: reveal iff the caret is on THIS line. An IMAGE ref is one
        // line, and follows the "heading model" (source reveals for editing when
        // the caret lands, the drawn image parks) exactly like every other
        // line-scoped kind — see `ConcealKind::Image`.
        ConcealKind::Heading
        | ConcealKind::Emphasis
        | ConcealKind::Code
        | ConcealKind::Highlight
        // A `~~strike~~` pair's tilde markers hide off their own line exactly
        // like an emphasis run's `**` — the drawn strike LINE is the affordance.
        | ConcealKind::Strikethrough
        | ConcealKind::Image
        // A link's `[`/`](url)` plumbing hides off its own line, leaving the
        // content-ink link TEXT (its separate `LinkText` span) visible; the whole
        // source reveals when the caret lands on the line.
        | ConcealKind::Link
        // A blockquote's leading `>` marker(s) hide off their own line — the
        // block's affordance off-caret is the margin-hung pull-quote mark; the
        // raw markers reveal when the caret lands on that line.
        | ConcealKind::Blockquote => !conceal_off_cursor,
    }
}

/// True when a `ConcealMarkup(Image)` span overlaps the document byte range
/// `[line_doc_start, line_end)` — i.e. this logical line is an inline-image
/// reference (`![alt](path)`). Used to distinguish a real IMAGE line from a
/// WRAPPED TABLE row: both reserve a tall `image_heights` slot, but on the caret's
/// line an image follows the CAPTION model (the source reveals body-size, so the
/// caret must be sized to the BODY glyphs — `cursor_scale` returns `1.0` — not the
/// tall row), while a table row keeps the pure heading model. Read by
/// [`super::TextPipeline::line_is_inline_image`] (the caret-scale gate), the one
/// owner of "is this an image line".
pub(super) fn line_has_image_span(
    md_spans: &[(std::ops::Range<usize>, crate::markdown::MdKind)],
    line_doc_start: usize,
    line_end: usize,
) -> bool {
    use crate::markdown::{ConcealKind, MdKind};
    md_spans.iter().any(|(cr, ck)| {
        matches!(ck, MdKind::ConcealMarkup(ConcealKind::Image))
            && cr.start < line_end
            && cr.end > line_doc_start
    })
}

/// True when a `Code`/`CodeSyntax` span (a fenced-block BODY byte) overlaps the
/// document byte range `[line_doc_start, line_end)` — i.e. this line is a fence
/// BODY line, not a marker line. Shared by [`add_wysiwyg_conceal_spans`]'s
/// `Fence` arm and [`super::TextPipeline::wysiwyg_report`] so both agree on
/// exactly which of a fenced block's lines are marker-concealable.
pub(super) fn line_has_code_span(
    md_spans: &[(std::ops::Range<usize>, crate::markdown::MdKind)],
    line_doc_start: usize,
    line_end: usize,
) -> bool {
    use crate::markdown::MdKind;
    md_spans.iter().any(|(cr, ck2)| {
        matches!(ck2, MdKind::Code { .. } | MdKind::CodeSyntax { .. })
            && cr.start < line_end
            && cr.end > line_doc_start
    })
}

/// REVEAL-ON-CURSOR concealment for the WYSIWYG amendment ("if the caret is on
/// that line, show the actual markdown; otherwise show the preview") —
/// GENERALIZES [`add_rule_conceal_span`]/[`add_bullet_conceal_span`] to the five
/// markup kinds [`crate::markdown::ConcealKind`] names, over the SAME transparent
/// [`RULE_CONCEAL_COLOR`] mechanism: the marker glyphs still SHAPE (the row keeps
/// its height and the bytes stay editable) but draw invisibly.
///
/// Every [`crate::markdown::MdKind::ConcealMarkup`] span in `md_spans` is
/// scoped by its [`crate::markdown::ConcealKind`]:
///  - `Heading`/`Emphasis`/`Code`/`Highlight` are LINE-scoped: `conceal_off_cursor`
///    (the caller's "caret is on a DIFFERENT line" gate — the same one
///    `add_rule_conceal_span`/`add_bullet_conceal_span` already use) decides all
///    four in lockstep with the hr/bullet conceal.
///  - `Fence` is BLOCK-scoped: a fenced code block's marker LINES (the fence open
///    line + the fence close line, including the info string) conceal unless
///    `cursor_byte` — the document byte offset of the CARET'S OWN line's first
///    byte — falls anywhere inside the whole span's byte range (`r.contains`),
///    i.e. the caret sits somewhere in the block, not just on this one line. A
///    BODY line inside the block (one carrying its own `Code`/`CodeSyntax` span)
///    is NEVER concealed by this arm regardless — only the marker lines hide;
///    the always-present PANEL (drawn from this same span, see
///    `super::TextPipeline::fence_panel_rects`) is the block's affordance.
///
/// Gated on [`crate::markdown::wysiwyg_on`]: OFF is a total no-op, so `wysiwyg =
/// false` reproduces the always-visible markup this round shipped without,
/// byte-identically (no `ConcealMarkup` span is ever concealed, only ever dimmed
/// like plain `Markup` — see `md_attrs`). No-op when no `ConcealMarkup` span
/// intersects the line, keeping non-WYSIWYG lines untouched.
///
/// `line_height` is the LINE's own effective row height (already
/// heading-scaled — i.e. `base_line_height * scale`, exactly what
/// [`scaled_base_attrs`] used to build `base`/`lb`) — see
/// [`CONCEAL_ZERO_WIDTH_FONT_SIZE`]'s doc comment for why the concealed span's
/// paired line-height override must match it exactly rather than shrinking.
pub(super) fn add_wysiwyg_conceal_spans(
    al: &mut glyphon::cosmic_text::AttrsList,
    line_text: &str,
    line_doc_start: usize,
    base: &Attrs<'static>,
    md_spans: &[(std::ops::Range<usize>, crate::markdown::MdKind)],
    conceal_off_cursor: bool,
    cursor_byte: usize,
    line_height: f32,
) {
    if !crate::markdown::wysiwyg_on() {
        return;
    }
    use crate::markdown::{ConcealKind, MdKind};
    let line_end = line_doc_start + line_text.len();
    // TRUE ZERO-WIDTH: transparent ink (draws nothing) PLUS a near-zero font
    // size (collapses the advance to sub-pixel), paired with the line's own
    // real line-height so the row's height stays keyed to its surviving
    // (unconcealed) glyphs — see this fn's doc comment + `CONCEAL_ZERO_WIDTH_FONT_SIZE`.
    let hidden = base
        .clone()
        .color(RULE_CONCEAL_COLOR)
        .metrics(GlyphMetrics::new(CONCEAL_ZERO_WIDTH_FONT_SIZE, line_height));
    for (r, kind) in md_spans {
        let ck = match *kind {
            MdKind::ConcealMarkup(ck) => ck,
            _ => continue,
        };
        if wysiwyg_reveals(ck, conceal_off_cursor, cursor_byte, r) {
            continue;
        }
        if ck == ConcealKind::Fence && line_has_code_span(md_spans, line_doc_start, line_end) {
            // Never conceal a BODY line — only a fence's own marker lines (open,
            // close, info string) have no `Code`/`CodeSyntax` span of their own;
            // a body line's entire text is covered by one, so skip it wholesale
            // rather than painting transparent ink over the body's own coloring.
            continue;
        }
        let lo = r.start.max(line_doc_start);
        let hi = r.end.min(line_end);
        if lo < hi {
            al.add_span((lo - line_doc_start)..(hi - line_doc_start), &hidden);
        }
    }
}

/// Build the per-cell `AttrsList` for a GFM table GRID cell (the tables-v1 styled,
/// off-cursor render — `layers::prepare_table_grid`). A cell is a SMALL INLINE
/// markdown context: it may carry `**bold**` / `*italic*` / `` `code` `` /
/// `==highlight==`, but no block construct. This reuses the EXACT inline-styling
/// seam prose uses — [`crate::markdown::spans`] on the cell substring, then
/// [`add_md_line_spans`] (content styling: real bundled Bold weight, italic,
/// mono+tint inline code) followed by [`add_wysiwyg_conceal_spans`] (the emphasis
/// / code / highlight DELIMITERS collapse to true zero-width) — so a cell styles
/// identically to the same run in body prose, with the raw markers gone from both
/// the pixels AND the shaped WIDTH (the concealed advance is sub-pixel, so a
/// caller measuring `run.line_w` after shaping sizes the column to the styled
/// content, not the raw source). `line_height` is the cell row's own height,
/// threaded into the zero-width conceal so the concealed markers never shrink the
/// row (the same contract [`build_line_attrs`] honors). A grid cell is ALWAYS the
/// off-cursor styled form — the caret's OWN table parks the grid and reveals raw
/// source a level up (`prepare_table_grid`), so `conceal_off_cursor = true` and
/// `cursor_byte` is irrelevant (a cell has no fenced block). A cell with NO inline
/// markup yields an empty span set → the returned list is `base` alone, so a
/// plain cell shapes BYTE-IDENTICALLY to the pre-styling `set_text(cell, base)`.
/// Gated implicitly on `wysiwyg_on()` (the caller only builds a grid when it is
/// on; `add_wysiwyg_conceal_spans` also self-gates), so nothing conceals off.
pub(super) fn cell_inline_attrs(
    base: &Attrs<'static>,
    line_height: f32,
    cell: &str,
) -> glyphon::cosmic_text::AttrsList {
    let md_spans = crate::markdown::spans(cell);
    let mut al = glyphon::cosmic_text::AttrsList::new(base);
    add_md_line_spans(&mut al, cell, 0, base, &md_spans);
    add_wysiwyg_conceal_spans(&mut al, cell, 0, base, &md_spans, true, 0, line_height);
    al
}

/// SYNTAX HIGHLIGHTING: apply THE role style ([`role_style_for`], the one owner)
/// to one span's attrs. The structural code (keywords, operators, identifiers,
/// punctuation) keeps the FULL ink; only the roles take a style — quiet,
/// desaturated per-world tints, never a loud hue and NEVER amber (DESIGN.md §3:
/// `primary` is the caret alone). The role's optional background WASH is drawn by
/// the wash pipelines (see `rects.rs::wash_rects`), not through attrs.
pub(super) fn syn_attrs(
    base: &Attrs<'static>,
    kind: crate::syntax::SynKind,
) -> Attrs<'static> {
    base.clone()
        .color(role_style_for(&theme::active(), kind).fg.to_glyphon())
}

// --- THE ROLE STYLE PROVIDER — one owner of role tint + wash -----------------
//
// The tonsky follow-up to Alabaster's four-role model: each world derives four
// QUIET, LOW-SATURATION role tints from its OWN palette, plus low-alpha
// background WASHES for the prose regions (comments everywhere, strings on the
// dark worlds). No per-theme syntax palette: [`role_style_for`] is a pure
// function of the theme's existing tokens (+ its optional
// [`theme::RoleOverrides`] escape hatch), so a new world inherits lawful role
// styles for free and the law test sweeps every world automatically.

/// Fixed role HUE ANCHORS (degrees). Strings lean GREEN, definitions BLUE,
/// constants VIOLET, the comment wash WARM YELLOW — min pairwise distance 70°,
/// and ≥ 38° from every world's `primary` hue (the amber guard's 30° floor).
const HUE_STR: f32 = 140.0;
const HUE_DEF: f32 = 220.0;
const HUE_CONST: f32 = 290.0;
const HUE_COMMENT_WASH: f32 = 50.0;

/// Foreground tint SATURATION per mode — both quiet (law cap 0.50): a dark
/// world's light ink is barely tinted (pale pastels); a light world's dark ink
/// needs a touch more saturation to read a hue at all. `S_FG_DARK` was raised
/// from the original 0.32 (see `T_DARK` below — the two moved together to fix
/// the imperceptible-Definition bug); it stays a comfortable 0.04 under the law
/// cap.
///
/// `S_FG_LIGHT` moved THREE times across two retunes. Round 1 (see `T_LIGHT`'s
/// history below) went UP toward the 0.50 cap then back DOWN to `0.28`: redmean
/// alone rewards MORE saturation (more hue-driven RGB spread), but the eye
/// resolves LUMINANCE, and the fixed hue anchors here (blue 220°, violet 290°)
/// carry very little of the Rec.709 luminance weight (`0.7152` sits on G, only
/// `0.0722` on B) — so *more* saturation at a light world's necessarily-dark
/// lightness actively pulls the tint AWAY from grey and DOWN in relative
/// luminance.
///
/// Round 2 (the ground-contrast retune — see law (i) and `T_LIGHT` below) landed
/// on `0.18`: round 1 fixed ink-luminance separation by pushing `T_LIGHT`'s
/// rungs UP toward `muted` — which, on the light worlds, is itself already most
/// of the way toward the pale `base_100` ground. That satisfied "distinct from
/// the ink" while quietly failing "readable on the page" (a live taste-gate
/// verdict on Saltpan: "too hard to read" — the roles read as washed-out
/// pastels). Lowering both `T_LIGHT` (back toward the ink) and `S_FG_LIGHT`
/// (less chroma competing with the now-smaller lightness excursion) restores
/// ground contrast while `sweep_light_ladder`'s grid search (now searching
/// jointly for the luminance floor AND the ground-contrast floor) still clears
/// every law with margin.
const S_FG_DARK: f32 = 0.46;
const S_FG_LIGHT: f32 = 0.18;

/// The PRESENCE t-ladder: each role's LIGHTNESS rides the world's OWN ink ladder,
/// `L = lerp(L(base_content), L(muted), t)` — `[Definition, Constant, Str]`, most
/// present (closest to full ink) first. Ordering preserved in both modes.
///
/// Dark's `Definition` rung was originally `0.12` (paired with `S_FG_DARK =
/// 0.32`): on every dark world that put Def's fg redmean distance from
/// `base_content` at 36–65 — under the perceptibility floor the role-style law
/// now enforces (`role_style_laws_hold_for_every_world`, law (g), floor 70) and,
/// per a live Currawong screenshot, visually indistinguishable from plain ink
/// (measured ≈43, barely over the *pairwise* law floor of 40 but perceptually
/// nil). Raised to `0.26` (+ `S_FG_DARK` to `0.46`) — measured worst case across
/// the 8 dark worlds is now redmean 86 vs `base_content` (Bowerbird) and 59 vs
/// `Constant` (Bombora), both comfortably clear of their floors, while the
/// 0.26 vs 0.28 gap to `Constant` keeps monotone presence ordering intact (the
/// ordering is an exact `t`-proportional relationship, not a numeric coin-flip,
/// so a 0.02 gap is as safe as a 0.16 one) and saturation stays 0.46 — under the
/// law's 0.50 cap with room to spare.
///
/// Light's rungs went through TWO retunes, both instances of the SAME class of
/// bug caught by measurement before a screenshot forced it.
///
/// Round 1 rungs were `[0.55, 0.75, 0.95]` (the user's own words: "function
/// colour way too close to everything else on a lot of themes; Saltpan
/// especially bad"). The pairwise redmean law (a) and the vs-ink floor (g) both
/// PASSED at those rungs (Saltpan Definition redmean 148 vs `base_content`,
/// comfortably over the 70 floor) — because almost all of that distance sat in
/// the BLUE channel, which the eye barely weighs (sparse S-cones; the classic
/// "dark blue link reads as black" problem). A dedicated relative-luminance
/// measurement (`measure_role_luminance`, an ignored scratch test) found the
/// true worst case: light `Definition` bought only 0.027–0.042
/// relative-luminance separation from `base_content` — a tenth of what `Str`
/// (green, luminance-heavy) got for free. Raised to `[0.84, 0.90, 0.94]` (+
/// `S_FG_LIGHT` up to `0.28`) via `sweep_light_ladder`'s grid search, maximizing
/// worst-case light `Definition` ΔY subject to laws (a)/(g) alone.
///
/// Round 2 (THIS retune) found the bug in round 1's own fix: a live taste-gate
/// verdict on Saltpan ("too hard to read") traced to round 1's cure creating a
/// NEW failure mode — pushing `t` up toward `muted` raises ink-luminance
/// separation, but on a light world `muted` is already most of the way toward
/// the pale `base_100` ground, so the SAME move that satisfies "distinct from
/// ink" (law h) actively fails "readable on the page" — no test measured
/// contrast against the GROUND, only against the ink. Law (i) below closes that
/// gap (a WCAG contrast-ratio floor vs `base_100`), and `sweep_light_ladder` now
/// searches `(t_def, t_const, t_str, s)` for the point that clears BOTH the
/// ink-luminance floor (h) and the ground-contrast floor (i) simultaneously —
/// LOWER `t` (back toward the ink, away from the washed-out ground) with LOWER
/// saturation (less chroma fighting the now-smaller lightness excursion) is the
/// answer both times pastel-camouflage shows up. Landed at `[0.76, 0.78, 0.80]`
/// (+ `S_FG_LIGHT` down to `0.18`): worst-case ground contrast 4.84:1 (Quokka
/// `Str`) and worst-case ink ΔY 0.056 (Gumtree `Definition`/`Constant`) — both
/// clear their floors with margin across all six light worlds. Monotone
/// ordering is preserved by construction (`t_def < t_const < t_str`).
const T_DARK: [f32; 3] = [0.26, 0.28, 0.44];
const T_LIGHT: [f32; 3] = [0.76, 0.78, 0.80];

/// WASH quad color params (rgba — computed quad colors, NOT theme tokens): dark
/// worlds wash with `hsl(anchor, 0.62, 0.66)` at alpha 0x2A (~16%); light worlds
/// with `hsl(50, 0.55, 0.50)` at 0x2E (~18%, comment wash only). Law-tested on
/// the COMPOSITED result over `base_100`: ΔL in [0.03, 0.12] — a wash is
/// structurally a whisper, incapable of reading as the accent.
const WASH_S_DARK: f32 = 0.62;
const WASH_L_DARK: f32 = 0.66;
const WASH_ALPHA_DARK: u8 = 0x2A;
const WASH_S_LIGHT: f32 = 0.55;
const WASH_L_LIGHT: f32 = 0.50;
const WASH_ALPHA_LIGHT: u8 = 0x2E;

/// HIGHLIGHT wash (`==marked==`) params — a DEDICATED wash, DECOUPLED from the
/// warm comment wash above (a deliberate, narrow break of the "one warm-wash
/// owner": a highlighter and a comment wash are DIFFERENT intents). The comment
/// wash is a subtle prose-warmth whisper (a low-alpha `hsl(50°)` cream); a
/// highlighter must POP ("look here"). The old shared-with-comment cream read
/// MUDDY on the cool pale light grounds (Gumtree pale-green, Bilby pale-cyan) —
/// a faint warm-over-cool blend with almost no hue contrast. The first fix was a
/// clean, cool VIOLET at higher saturation + alpha — but a FIXED foreign 280°
/// read as an imported, un-native color, the same on every world.
///
/// THE PER-WORLD FIX (this round): the highlight HUE is now DERIVED from each
/// world's OWN accent — `hue(primary) + HIGHLIGHT_HUE_OFFSET_FROM_PRIMARY` — a
/// SPLIT-COMPLEMENTARY of the caret's hue, so the highlighter reads as belonging
/// to the world (it harmonizes with the world's one warm accent) while the fixed
/// 165° rotation structurally GUARANTEES the amber guard (the highlight sits
/// exactly 165° off `primary` on every world — ≫ DESIGN §3's 30° floor, so the
/// caret's amber stays its own). The offset was chosen by a 14-world sweep (see
/// the retired `probe_highlight_hues` scratch analysis / the law test): among the
/// offsets that keep the absolute composited pop ≥ 70 AND out-pop the comment
/// wash on every world, 165° MAXIMIZES the worst-case separation from each
/// world's GROUND hue (min 20.8° — so no world's highlight muddies against its
/// own page) while giving the strongest out-pop margin over the comment whisper
/// (≥ 23.8 redmean). Saturation/lightness/alpha (the PRESENCE the violet round
/// added) are UNCHANGED and still split per light/dark class — only the hue
/// became per-world. Law-tested (`highlight_wash_laws_hold_for_every_world`) on
/// the COMPOSITED result over each world's `base_100`: distinct from the comment
/// wash, ≥ 30° off `primary` (amber guard), pops (redmean floor + out-pops the
/// comment whisper), a calm ΔL ceiling, AND per-world variation (≥ 8 distinct
/// hues across the 14 worlds — proof the hue is no longer a single fixed value).
const HIGHLIGHT_HUE_OFFSET_FROM_PRIMARY: f32 = 165.0;
const HIGHLIGHT_S_DARK: f32 = 0.58;
const HIGHLIGHT_L_DARK: f32 = 0.64;
const HIGHLIGHT_ALPHA_DARK: u8 = 0x3A;
const HIGHLIGHT_S_LIGHT: f32 = 0.50;
const HIGHLIGHT_L_LIGHT: f32 = 0.58;
const HIGHLIGHT_ALPHA_LIGHT: u8 = 0x4D;

/// The style ONE Alabaster role renders with in a given world: the quiet
/// foreground TINT plus an optional low-alpha background WASH (an rgba quad
/// color the wash pipelines composite behind the span's glyphs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RoleStyle {
    pub fg: theme::Srgb,
    pub wash: Option<theme::Srgb>,
}

/// THE role style provider — the single place a syntax role's foreground tint +
/// optional wash are derived (what `syn_role_color` grew into). A PURE function
/// of the passed theme (`base_content` / `muted` lightness, `dark` mode, plus its
/// [`theme::RoleOverrides`]), NOT of the process-global active theme, so the law
/// test can sweep all fourteen worlds lock-free. Consumers: [`syn_attrs`] (code
/// buffers), [`md_attrs`]'s `CodeSyntax` arm (markdown fences inherit
/// automatically), and the wash geometry/tint plumbing — no second copy.
///
/// The derivation:
/// - `Definition` / `Constant` / `Str` foregrounds: `hsl(anchor, S_mode,
///   lerp(L(base_content), L(muted), t))` — hue from the fixed anchors, presence
///   from the world's own ink ladder. Never washed (`Str` excepted below) —
///   single-token washes read as confetti.
/// - `Comment` (PROSE tier — tonsky inverted): fg is `base_content` EXACTLY
///   (comments are the prose in the code — FULL ink) + the warm comment wash on
///   every world.
/// - `CommentCode` (commented-out code): fg is `muted` EXACTLY, no wash —
///   today's grey.
/// - `Str` additionally carries the green wash on DARK worlds only (wash-first
///   on dark, fg-tint-first on light, per the essay).
///
/// A world's `role_overrides` may pin any fg, pin a wash, or disable a wash; the
/// law test validates the EFFECTIVE style, so overrides cannot break the laws.
pub(super) fn role_style_for(th: &theme::Theme, kind: crate::syntax::SynKind) -> RoleStyle {
    use crate::syntax::SynKind;
    let ov = th.role_overrides;
    let (_, _, l_full) = th.base_content.to_hsl();
    let (_, _, l_dim) = th.muted.to_hsl();
    let (t, s_fg) = if th.dark {
        (T_DARK, S_FG_DARK)
    } else {
        (T_LIGHT, S_FG_LIGHT)
    };
    let fg_at =
        |anchor: f32, ti: f32| theme::Srgb::from_hsl(anchor, s_fg, l_full + (l_dim - l_full) * ti);
    let derived_wash = |anchor: f32| {
        if th.dark {
            let c = theme::Srgb::from_hsl(anchor, WASH_S_DARK, WASH_L_DARK);
            theme::Srgb::rgba(c.r, c.g, c.b, WASH_ALPHA_DARK)
        } else {
            let c = theme::Srgb::from_hsl(HUE_COMMENT_WASH, WASH_S_LIGHT, WASH_L_LIGHT);
            theme::Srgb::rgba(c.r, c.g, c.b, WASH_ALPHA_LIGHT)
        }
    };
    let with_override = |derived: Option<theme::Srgb>, ov: theme::WashOverride| match ov {
        theme::WashOverride::Default => derived,
        theme::WashOverride::Off => None,
        theme::WashOverride::Pin(c) => Some(c),
    };
    match kind {
        // PROSE comments are PROMINENT (decision: comments are the prose in the
        // code): FULL content ink + the warm wash carrying the comment identity.
        SynKind::Comment => RoleStyle {
            fg: th.base_content,
            wash: with_override(Some(derived_wash(HUE_COMMENT_WASH)), ov.comment_wash),
        },
        // Commented-OUT code recedes to the muted grey it always had — no wash.
        SynKind::CommentCode => RoleStyle { fg: th.muted, wash: None },
        SynKind::Definition => RoleStyle {
            fg: ov.def_fg.unwrap_or_else(|| fg_at(HUE_DEF, t[0])),
            wash: None,
        },
        SynKind::Constant => RoleStyle {
            fg: ov.const_fg.unwrap_or_else(|| fg_at(HUE_CONST, t[1])),
            wash: None,
        },
        // Strings: green fg tint everywhere; the green wash only on DARK worlds
        // (light worlds carry string identity in the fg tint alone).
        SynKind::Str => RoleStyle {
            fg: ov.str_fg.unwrap_or_else(|| fg_at(HUE_STR, t[2])),
            wash: with_override(
                if th.dark { Some(derived_wash(HUE_STR)) } else { None },
                ov.str_wash,
            ),
        },
    }
}

/// The ACTIVE world's wash quad rgba for a role, for the two fixed-tint wash
/// pipelines (`render.rs` construction + `sync_theme_colors` re-tint). A role /
/// world with NO wash yields fully-transparent bytes (the pipeline also uploads
/// zero instances then, so nothing draws either way).
pub(super) fn wash_rgba_bytes(kind: crate::syntax::SynKind) -> [u8; 4] {
    role_style_for(&theme::active(), kind)
        .wash
        .unwrap_or(theme::Srgb::rgba(0, 0, 0, 0))
        .rgba_bytes()
}

/// The DEDICATED markdown `==highlight==` wash quad color for a world — its hue
/// DERIVED from the world's OWN accent (`hue(primary) +
/// HIGHLIGHT_HUE_OFFSET_FROM_PRIMARY`, a split-complementary), with the presence
/// (saturation / lightness / alpha) split per light/dark class. Decoupled from
/// the warm comment wash so a highlighter POPS while comments stay a subtle prose
/// whisper, but — unlike the retired fixed violet — now reads as NATIVE to each
/// world (see the `HIGHLIGHT_HUE_OFFSET_FROM_PRIMARY` doc above for the "why" and
/// the sweep that picked 165°). A PURE function of the passed theme (its
/// `primary` hue + `dark` flag), so the law test can sweep every world lock-free.
/// Every world carries it (no override hatch in v1 — unlike the syntax washes, a
/// highlight is never opted out).
///
/// **MONOCHROME WORLDS (`Theme::is_monochrome`, THEMES.md's logged DESIGN.md
/// §3 "no warm thing" amendment):** an achromatic `primary` has NO hue to
/// rotate — `hue(primary)` is a meaningless `0.0` for a plain grey (see
/// `Srgb::to_hsl`'s achromatic case), so deriving a highlight hue from it would
/// silently produce a color the world otherwise renders none of. Forced to
/// saturation `0.0` instead: the highlight becomes a pure VALUE-STEP wash — the
/// same "no hue, only lightness" idiom the WYSIWYG panel/pill already use — at
/// the SAME per-mode `l`/`alpha` every other world's highlight uses, so it still
/// pops exactly as loud, just without a hue to pop WITH.
///
/// **TRUE 1-BIT WORLDS (`Theme::is_monochrome` is the general case;
/// `Theme::is_one_bit` — Wagtail's 2026-07 rework — is the stricter one):**
/// the monochrome branch above still leaves a MID-LIGHTNESS grey wash
/// (`HIGHLIGHT_L_DARK`/`_LIGHT` sit well short of 0.0/1.0), which is exactly
/// the kind of authored grey a 1-bit world forbids outright.
///
/// **THE DITHER ROUND (supersedes the old "fully OFF" answer):** a 1-bit
/// world no longer drops the highlight wash to `alpha = 0` — it routes
/// through **THE ONE WAGTAIL HIGHLIGHT TEXTURE** instead (the user's razor:
/// one kind of emphasis, one texture — see THEMES.md's 1-bit section), a
/// deterministic Bayer-ordered dither stipple (`shaders/selection.wgsl`'s
/// `fs_main` dither branch, density `render::dither::
/// WAGTAIL_HIGHLIGHT_DITHER_DENSITY`) that is EVERY pixel either pure quad
/// color at full opacity or fully transparent — never a fractional alpha, so
/// it never composites a forbidden grey the way the old flat-alpha wash
/// would have. This function's job for a one-bit world simplifies to naming
/// the dither's ONE color: pure opaque white (the token
/// [`highlight_wash_rgba_bytes`] feeds the pipeline; the DENSITY that turns
/// dither mode on is a separate call, [`wagtail_dither_density`], applied at
/// the same construction/re-tint call sites). `==highlight==` still reads
/// structurally either way (the `==` delimiters still conceal/reveal, the
/// marked text still keeps full ink) — now it ALSO carries the dither band,
/// exactly like search matches do on a one-bit world (see
/// `wagtail_dither_density`'s doc for the "one texture, two consumers" wiring).
pub(super) fn highlight_wash(th: &theme::Theme) -> theme::Srgb {
    if let theme::HighlightTexture::Stipple { color, .. } = th.render_caps.highlight_texture {
        return theme::Srgb::rgba(color.r, color.g, color.b, 0xFF);
    }
    let (s, l, alpha) = if th.dark {
        (HIGHLIGHT_S_DARK, HIGHLIGHT_L_DARK, HIGHLIGHT_ALPHA_DARK)
    } else {
        (HIGHLIGHT_S_LIGHT, HIGHLIGHT_L_LIGHT, HIGHLIGHT_ALPHA_LIGHT)
    };
    let s = if th.is_monochrome() { 0.0 } else { s };
    let (primary_hue, _, _) = th.primary.to_hsl();
    let hue = (primary_hue + HIGHLIGHT_HUE_OFFSET_FROM_PRIMARY).rem_euclid(360.0);
    let c = theme::Srgb::from_hsl(hue, s, l);
    theme::Srgb::rgba(c.r, c.g, c.b, alpha)
}

/// The ACTIVE world's `==highlight==` wash quad rgba, for the fixed-tint highlight
/// wash pipeline (`render.rs` construction + `sync_theme_colors` re-tint) — the
/// sibling of [`wash_rgba_bytes`] for the dedicated highlight bucket.
pub(super) fn highlight_wash_rgba_bytes() -> [u8; 4] {
    highlight_wash(&theme::active()).rgba_bytes()
}

/// THE ONE WAGTAIL HIGHLIGHT TEXTURE's density switch — `0.0` (dither mode
/// OFF, every non-one-bit world) or [`dither::WAGTAIL_HIGHLIGHT_DITHER_DENSITY`]
/// (one-bit worlds). Fed into `SelectionPipeline::set_dither` at the SAME two
/// call sites [`highlight_wash_rgba_bytes`] feeds `set_color` — construction
/// AND every `sync_theme_colors` re-tint (a switch AWAY from a one-bit world
/// must reset this back to `0.0`, never merely leave it stale). The two
/// consumers this drives — `wash_highlight_pipeline` (`==highlight==` spans)
/// and `match_pipeline` (search matches) — deliberately share this ONE
/// function + density: the razor is ONE texture for ONE meaning ("something
/// here is marked"), not a per-consumer ladder.
pub(super) fn wagtail_dither_density() -> f32 {
    match theme::active().render_caps.highlight_texture {
        theme::HighlightTexture::Stipple { density, .. } => density,
        theme::HighlightTexture::Wash => 0.0,
    }
}

/// The ACTIVE world's SEARCH-MATCH quad rgba — `theme::selection()` on every
/// ordinary world (unchanged), but on a one-bit world this NO LONGER shares
/// the (now true-inverse-video) document-selection token: it instead reads
/// pure opaque white, the SAME single color [`highlight_wash_rgba_bytes`]
/// feeds the dither pipeline, since a one-bit search match renders through
/// THE ONE WAGTAIL HIGHLIGHT TEXTURE too (paired with [`wagtail_dither_density`]
/// on `match_pipeline`) rather than the old solid-white/punch-outline
/// mechanism document selection used to share with it.
pub(super) fn search_match_rgba_bytes() -> [u8; 4] {
    match theme::active().render_caps.highlight_texture {
        theme::HighlightTexture::Stipple { color, .. } => {
            theme::Srgb::rgba(color.r, color.g, color.b, 0xFF).rgba_bytes()
        }
        theme::HighlightTexture::Wash => theme::selection().rgba_bytes(),
    }
}

// --- THE STRIKE-LINE OWNER — one owner of strike geometry + ink --------------
//
// "The strike-line geometry over a text run" lives HERE and only here: the
// thickness, the vertical position as a fraction of the run's text band, and
// the ink derivation. Two consumers, both routing through these fns so they can
// never drift: (1) the DOCUMENT renderer's `~~strike~~` quads
// (`rects.rs::strike_lines`, one thin band per visual-row segment of every
// `MdKind::Strikethrough` span), and (2) the format POPOVER's
// self-demonstrating `S` button (`chrome/popover.rs`), which draws the same
// line through its own glyph ink band. The struck TEXT's color rides
// [`strike_ink`] too (`md_attrs`' `Strikethrough` arm), so text and line share
// one register by construction.

/// Strike-line stroke thickness (px at zoom 1.0). A hair heavier than the
/// writing-nit hint ([`super::NIT_THICKNESS`], 1.3) is NOT wanted — a strike is
/// content styling, not an annotation, and the muted ink already carries the
/// receding register; the same fine weight reads as one family of quiet lines.
pub(in crate::render) const STRIKE_THICKNESS: f32 = 1.3;

/// Vertical position of the strike line's CENTER, as a fraction of the text
/// band's height from its top. 0.5 — the middle of the band. For the document
/// this is the caret-height glyph cell (`row_band_for`), whose middle crosses
/// lowercase letters just above their waist; for the popover's `S` it is the
/// measured cap-height ink band, whose middle bisects the glyph. Both read as
/// the conventional struck-through look.
pub(in crate::render) const STRIKE_V_FRAC: f32 = 0.5;

/// THE LINE-BAND PRIMITIVE — "a thin flat line over a fraction of a text band"
/// (thickness + antialiasing feather, centered at `v_frac` of `height` from
/// `top`), the ONE owner both [`strike_line_band`] (mid-run, `STRIKE_V_FRAC`)
/// and [`link_underline_band`] (near the baseline, `LINK_UNDERLINE_V_FRAC`)
/// route through — a strike-through and an underline are the SAME primitive,
/// differing only in vertical placement (same behavior => same code). Pure +
/// total; every caller hands the result straight to a
/// [`crate::spellunderline::Squiggle`] with `amp: 0.0` (a flat line).
fn line_band(top: f32, height: f32, zoom: f32, v_frac: f32, thickness: f32) -> (f32, f32, f32) {
    let stroke = thickness * zoom;
    let band_h = stroke + 2.0;
    let center = top + height * v_frac;
    (center - band_h * 0.5, band_h, stroke)
}

/// THE strike-line geometry over a text band: given the band's `top`/`height`
/// (the run's glyph cell — caret band in the document, measured ink band in the
/// popover) and the current `zoom`, the line's `(band_top, band_h, stroke)` — a
/// quad band centered at `STRIKE_V_FRAC` of the text band, just tall enough for
/// the stroke plus a 2px antialiasing feather (the same `thickness + 2.0`
/// envelope the nit underline uses), and the zoom-scaled stroke thickness
/// itself. Pure + total; both call sites hand these straight to a
/// [`crate::spellunderline::Squiggle`] with `amp: 0.0` (a flat line).
pub(in crate::render) fn strike_line_band(top: f32, height: f32, zoom: f32) -> (f32, f32, f32) {
    line_band(top, height, zoom, STRIKE_V_FRAC, STRIKE_THICKNESS)
}

/// Link-underline stroke thickness (px at zoom 1.0) — the SAME fine weight as
/// [`STRIKE_THICKNESS`] (one family of quiet lines; the underline is a quiet
/// affordance, not an annotation).
pub(in crate::render) const LINK_UNDERLINE_THICKNESS: f32 = STRIKE_THICKNESS;

/// Vertical position of the link underline's CENTER, as a fraction of the
/// text band's height from its top — near the BASELINE (unlike the strike's
/// mid-run `STRIKE_V_FRAC`), so it reads as an underline under the link text
/// rather than a line through it. `0.92` sits just under the caret-height
/// glyph cell's bottom (inside it, never spilling into the next row).
pub(in crate::render) const LINK_UNDERLINE_V_FRAC: f32 = 0.92;

/// THE link-underline geometry over a text band — [`line_band`] at
/// [`LINK_UNDERLINE_V_FRAC`]/[`LINK_UNDERLINE_THICKNESS`], the SAME primitive
/// [`strike_line_band`] rides, just a different vertical band: "a line under a
/// run" and "a line through a run" are one mechanism, not two.
pub(in crate::render) fn link_underline_band(top: f32, height: f32, zoom: f32) -> (f32, f32, f32) {
    line_band(top, height, zoom, LINK_UNDERLINE_V_FRAC, LINK_UNDERLINE_THICKNESS)
}

/// THE strike ink — the world's `muted` rung EXACTLY: the receding markup ink
/// every dim syntax character already rides, so struck text (and its line)
/// recede to a register the world is guaranteed to render legibly, with zero
/// saturation risk toward the caret's amber (DESIGN §3 — `muted` is an ink-
/// ladder rung, not a hue). On a 1-bit world this is an authored pure
/// black/white token, so the strike stays lawful there by construction. Shared
/// by `md_attrs` (the struck TEXT) and [`strike_srgba_bytes`] (the LINE
/// pipelines) — one derivation, two surfaces.
pub(in crate::render) fn strike_ink(th: &theme::Theme) -> theme::Srgb {
    th.muted
}

/// The ACTIVE world's strike-line rgba for the two strike pipelines (document +
/// popover), fed at construction and every `sync_theme_colors` re-tint — the
/// sibling of [`highlight_wash_rgba_bytes`] for the strike stroke.
pub(in crate::render) fn strike_srgba_bytes() -> [u8; 4] {
    strike_ink(&theme::active()).rgba_bytes()
}

/// THE link-underline ink — the SAME world `muted` rung [`strike_ink`] uses (a
/// quiet, value-only affordance under the link's full-content-ink TEXT, never
/// the caret's amber — DESIGN §3: the link TEXT stays full content ink per the
/// 2026-07-22 decision, so only the underline itself carries the muted tint).
pub(in crate::render) fn link_underline_ink(th: &theme::Theme) -> theme::Srgb {
    strike_ink(th)
}

/// The ACTIVE world's link-underline rgba, fed to the underline pipeline at
/// construction and every `sync_theme_colors` re-tint — the sibling of
/// [`strike_srgba_bytes`] for the link-underline stroke.
pub(in crate::render) fn link_underline_srgba_bytes() -> [u8; 4] {
    link_underline_ink(&theme::active()).rgba_bytes()
}

/// SYNTAX HIGHLIGHTING: lay the syntax spans that intersect ONE buffer line over
/// `al`, mirroring [`add_md_line_spans`] (markdown and syntax never both apply, so
/// this composes on the SAME per-span seam as a parallel base layer). Maps each
/// document-byte span into this line's local byte range and adds it with
/// [`syn_attrs`]. No-op when `syn_spans` is empty (non-code buffers), keeping their
/// render byte-identical.
pub(super) fn add_syn_line_spans(
    al: &mut glyphon::cosmic_text::AttrsList,
    line_text: &str,
    line_doc_start: usize,
    base: &Attrs<'static>,
    syn_spans: &[(std::ops::Range<usize>, crate::syntax::SynKind)],
) {
    add_line_spans(al, line_text, line_doc_start, base, syn_spans, syn_attrs);
}

/// The font / line-height SCALE for ONE buffer line, driven by its LEADING `#`
/// run: `# ` → h1, `## ` → h2, `###`+ → h3 (see [`crate::markdown::heading_scale`]).
/// Keyed off the raw hash COUNT, NOT a fully-valid ATX heading, so a line grows the
/// instant you type `#` — before the space and title (and even for `#foo`). Only
/// the LEADING run counts (after optional indent), so a `#` mid-prose is ignored.
/// A THEMATIC-BREAK line (`---`/`***`/`___`, see [`crate::markdown::is_thematic_break`])
/// grows to the ACTIVE world's [`crate::theme::Theme::ornament_scale`] (per-world by
/// the ornament's character — see that field) so its row fits the bigger centered
/// break fleuron (drawn separately by `prepare_ornaments`, which reads the SAME
/// per-world scale for its glyph line-box, so the two stay in lockstep). `md` gates it:
/// a non-markdown buffer (and any plain line) returns the byte-identical `1.0`. The
/// DIM-markup + bold-weight styling still comes from the pulldown spans in
/// [`md_attrs`]; this governs SIZE alone, so an in-progress `#foo` is big but not yet
/// bold until it becomes a real heading.
pub(super) fn md_line_scale(line_text: &str, md: bool) -> f32 {
    if !md {
        return 1.0;
    }
    let b = line_text.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    let mut hashes = 0u8;
    while i < b.len() && b[i] == b'#' {
        hashes = hashes.saturating_add(1);
        i += 1;
    }
    if hashes > 0 {
        return crate::markdown::heading_scale(hashes);
    }
    // A THEMATIC BREAK (`---`/`***`/`___`) grows its row to fit the bigger centered
    // ornament fleuron (drawn by `prepare_ornaments`), exactly the heading-row
    // machinery above — the tall row centers the glyph. The scale is the ACTIVE
    // world's per-world `ornament_scale` (the SAME value `prepare_ornaments` shapes
    // the glyph at), UNIFORM per break line regardless of caret, so the row never
    // reflows on cursor move (the raw `---` reveals in place when the caret lands, at
    // the same scaled size). A theme switch that changes the scale re-fits the row via
    // `restyle_all_lines`, like the heading sizes.
    if crate::markdown::is_thematic_break(line_text) {
        return crate::theme::active().ornament_scale;
    }
    1.0
}

/// `base` with a per-line metrics override applied (heading lines render LARGER).
/// At `scale == 1.0` this returns a plain clone with NO `metrics_opt`, so a
/// non-heading line shapes byte-identically to the pre-heading-size renderer.
/// Otherwise it sets `Attrs::metrics(base_font * scale, base_line * scale)`;
/// cosmic-text derives a row's height from the MAX of its glyphs' per-span line
/// heights (`shape.rs`), so applying this to the line's default attrs AND to every
/// span built from it makes the whole heading row taller and its glyphs bigger.
/// The values are ABSOLUTE pixels (already zoom/DPI-folded), so any zoom/DPI change
/// must rebuild these (see [`TextPipeline::restyle_all_lines`]).
pub(super) fn scaled_base_attrs(
    base: &Attrs<'static>,
    base_font_size: f32,
    base_line_height: f32,
    scale: f32,
) -> Attrs<'static> {
    if (scale - 1.0).abs() < 1e-3 {
        return base.clone();
    }
    base.clone()
        .metrics(GlyphMetrics::new(base_font_size * scale, base_line_height * scale))
}

/// Assemble ONE buffer line's complete `AttrsList` from the base doc attrs plus
/// every styling layer, in the canonical order: heading SIZE scale
/// ([`scaled_base_attrs`]) → markdown spans → syntax spans → CJK family spans →
/// SYMBOL family spans → (optional) RULE + BULLET concealment (symbol family wins on
/// symbol runs, CJK family on CJK runs; markdown/syntax weight/color/style win
/// elsewhere; the concealed markup's transparent ink wins LAST over its own glyphs).
/// `line_doc_start` is the line's first document byte (so the whole-document span
/// lists map into this line's local range). `conceal_off_cursor` is the reveal-on-
/// cursor gate: when set (the caret is on a DIFFERENT line) a markdown horizontal-rule
/// line's literal `---` are hidden via [`add_rule_conceal_span`] (leaving the centered
/// fleuron) AND a bullet's raw `-`/`*`/`+` via [`add_bullet_conceal_span`] (leaving its
/// depth glyph); when clear (the caret is on the line) the raw markup stays dim +
/// editable and no ornament is drawn. `cursor_byte` (the caret line's first document
/// byte) additionally drives the WYSIWYG conceal ([`add_wysiwyg_conceal_spans`]) for
/// its one BLOCK-scoped kind (a fenced code block's marker lines). This is the SINGLE
/// recipe shared by [`TextPipeline::set_text_incremental`] and
/// [`TextPipeline::restyle_all_lines`], so the two paths can never drift on layer
/// ordering or membership.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_line_attrs(
    base: &Attrs<'static>,
    base_font_size: f32,
    base_line_height: f32,
    md: bool,
    line_text: &str,
    line_doc_start: usize,
    md_spans: &[(std::ops::Range<usize>, crate::markdown::MdKind)],
    syn_spans: &[(std::ops::Range<usize>, crate::syntax::SynKind)],
    doc_lang: Option<crate::frontmatter::Lang>,
    cjk_priority: &[crate::frontmatter::Lang],
    fonts: &super::text::ScriptFonts,
    conceal_off_cursor: bool,
    cursor_byte: usize,
    image_row_height: Option<f32>,
) -> glyphon::cosmic_text::AttrsList {
    // An IMAGE line reserves a TALL row at its display height — NORMAL font size
    // (so the revealed `![alt](path)` source stays readable when the caret lands)
    // over a tall LINE-HEIGHT (the row cosmic-text derives from the row's max
    // glyph line-height). This is the "per-line metric override" the headings use,
    // but with an ABSOLUTE line-height rather than a font-size scale, so it stays
    // decoupled from the font size. `row_lh` also feeds the zero-width conceal
    // below so the off-cursor (fully concealed) source keeps the row tall.
    //
    // CAPTION-STYLE REVEAL (re-decided 2026-07-09, supersedes the reveal-GROW
    // model): the image row is ALWAYS exactly the image height `h` — the caret
    // landing on / leaving the line causes ZERO row-height change and ZERO reflow
    // (the headline win). Off the caret's line the source CONCEALS (zero-width) and
    // the image fills the row. ON the caret's line the source REVEALS at body size
    // and cosmic-text centres it VERTICALLY within the same `h`-tall row — i.e.
    // OVER the still-drawn, DIMMED image (a deliberate caption; a scrim band behind
    // the text lifts legibility — see `layers::prepare_images`). Growing the row to
    // stack source-above-image is geometrically impossible with one layout line per
    // row (cosmic-text gives each line ONE vertically-centred baseline), so we lean
    // into the centering rather than fight it.
    let scale = md_line_scale(line_text, md);
    let (lb, row_lh) = match image_row_height {
        Some(h) => (
            base.clone().metrics(GlyphMetrics::new(base_font_size, h)),
            h,
        ),
        None => (
            scaled_base_attrs(base, base_font_size, base_line_height, scale),
            base_line_height * scale,
        ),
    };
    let mut al = glyphon::cosmic_text::AttrsList::new(&lb);
    add_md_line_spans(&mut al, line_text, line_doc_start, &lb, md_spans);
    add_syn_line_spans(&mut al, line_text, line_doc_start, &lb, syn_spans);
    add_script_spans(&mut al, line_text, &lb, doc_lang, cjk_priority, fonts);
    add_symbol_spans(&mut al, line_text, &lb);
    // REVEAL-ON-CURSOR: when the caret is off this line, conceal a thematic break's
    // raw `---` (leaving the fleuron) AND a bullet's raw `-` (leaving the depth glyph).
    // Both are drawn as ornaments on the SAME rows; on the caret's own line the raw
    // markup reveals for editing and no ornament is drawn.
    if conceal_off_cursor {
        add_rule_conceal_span(&mut al, line_text, line_doc_start, &lb, md_spans);
        add_bullet_conceal_span(&mut al, line_text, &lb);
    }
    // WYSIWYG: heading/emphasis/inline-code/highlight markup (line-scoped, same
    // gate as above) + a fenced block's marker lines (block-scoped, `cursor_byte`).
    // A total no-op when `wysiwyg_on()` is false. `base_line_height * scale` is
    // this LINE's own effective row height (matches what `scaled_base_attrs`
    // used to build `lb`), so the zero-width conceal spans below never shrink
    // the row — see `add_wysiwyg_conceal_spans`'s doc comment.
    add_wysiwyg_conceal_spans(
        &mut al, line_text, line_doc_start, &lb, md_spans, conceal_off_cursor, cursor_byte,
        row_lh,
    );
    al
}
