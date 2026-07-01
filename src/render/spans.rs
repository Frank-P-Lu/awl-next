//! SPAN / ATTRS LAYERING — how one buffer line's `AttrsList` is assembled from
//! the base doc attrs plus every styling layer (markdown, syntax, CJK family,
//! heading SIZE scale, focus overlay).
//!
//! Unlike the caret geometry next door (which is inherent methods on
//! [`super::TextPipeline`] because it reads the live font/layout/metrics state),
//! every helper here is a PURE free function: it takes the line text, the base
//! [`Attrs`], the parsed span lists, and the resolved CJK `(family, weight)` as
//! explicit params and returns the layered attrs — no `&self`, no GPU state. That
//! is exactly why this cluster lifts out of `render.rs` cleanly: the pipeline
//! methods that drive shaping ([`super::TextPipeline::set_text_incremental`] /
//! [`super::TextPipeline::restyle_all_lines`]) call [`build_line_attrs`], the
//! single recipe that orders the layers (heading scale → markdown spans → syntax
//! spans → CJK family spans), and the focus pass calls
//! [`add_focus_overlay_spans`]. The bodies are carved out of `render.rs` VERBATIM,
//! so the capture output is byte-identical.
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

/// Lay per-theme CJK family spans over `al` for every CJK run in `text`. The
/// span inherits `base` (the doc/colored attrs — ligatures, color, etc.) but
/// overrides the family to the resolved CJK face and its concrete registered
/// weight. `cjk` is the `(family, weight)` resolved once via
/// [`TextPipeline::resolve_cjk`]; when it is `None` (neither the mincho nor the
/// gothic face is installed) this is a no-op and shaping falls through to
/// cosmic-text's neutral platform fallback. Resolving the CONCRETE weight is
/// mandatory: macOS Hiragino ships only W3/W6 (no Weight 400), and cosmic-text's
/// script fallback filters on `weight_diff == 0`, so naming the family at the
/// default 400 would drop it — the same weight trap as the mono fix.
pub(super) fn add_cjk_spans(
    al: &mut glyphon::cosmic_text::AttrsList,
    text: &str,
    base: &Attrs,
    cjk: Option<(&'static str, glyphon::Weight)>,
) {
    let Some((fam, wt)) = cjk else { return };
    let a = base.clone().family(Family::Name(fam)).weight(wt);
    for run in cjk_runs(text) {
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

/// Build the concrete `Attrs` for one markdown span kind, transforming `base`
/// (the doc attrs — family, ligature features, etc.):
/// - `Markup`/`Quote`/`ListMarker`/`Rule` → recede to the DIM ink (syntax + quiet
///   text); a `Rule` row also gets a thin centered quad drawn over it.
/// - `Heading` → no transform; reads by SIZE alone (set per-line upstream).
/// - `Task(true)`/`TaskDone` → DIM (a completed todo recedes as one); `Task(false)`
///   (an OPEN checkbox) rides the full default ink so the box stays present.
/// - `Bold`/`Italic`/`BoldItalic` → weight / style; NO color, so they ride the
///   buffer's default ink (full when focus off, dim when focus dims the region).
/// - `Code` → the registered monospace family + a subtle tint toward MUTED ink.
/// - `LinkText` → the buffer's full CONTENT ink (it lifts off the dim `Markup`
///   span; DESIGN §3 keeps `primary`/amber for the caret alone).
///
/// `color_override` is the FOCUS-mode ink: when `Some`, it replaces the kind's
/// natural color so the active unit brightens uniformly while KEEPING the span's
/// weight/style/family. This is what lets markdown compose under focus without
/// either layer clobbering the other.
pub(super) fn md_attrs(
    base: &Attrs<'static>,
    kind: crate::markdown::MdKind,
    color_override: Option<glyphon::Color>,
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
        | MdKind::Quote
        | MdKind::ListMarker
        | MdKind::Rule
        | MdKind::Task(true)
        | MdKind::TaskDone => {
            natural = Some(dim);
        }
        MdKind::Task(false) => {
            // An OPEN checkbox rides the buffer's FULL default ink so the empty box
            // reads as a present, actionable marker — one value step above the dim
            // `- ` bullet before it. No accent (amber is the caret's alone).
        }
        MdKind::Heading(_) => {
            // No-op transform: a heading reads as a heading by SIZE alone (applied
            // per-LINE upstream via [`scaled_base_attrs`], already in `base`), riding
            // the buffer's full default ink. We deliberately do NOT set:
            //  - COLOR: DESIGN.md §3 — `primary` (amber) is the caret and ONLY the
            //    caret; figure/ground is by VALUE + size, not by spending the accent.
            //  - BOLD weight: every bundled face is Regular-only, so requesting BOLD
            //    trips cosmic-text's `weight_diff == 0` fallback filter (the weight
            //    trap, see `mono_safe_weight`), DROPS the proportional theme face, and
            //    renders the title in the mono fallback on serif/sans worlds. Regular
            //    weight keeps the title in the world's own face at any size. The 1.8x
            //    size is plenty of hierarchy on its own.
        }
        MdKind::Bold => {
            a = a.weight(glyphon::Weight::BOLD);
        }
        MdKind::Italic => {
            a = a.style(glyphon::Style::Italic);
        }
        MdKind::BoldItalic => {
            a = a.weight(glyphon::Weight::BOLD).style(glyphon::Style::Italic);
        }
        MdKind::Code => {
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
            // ([`syn_role_color`]), so a fence and a `.rs` file highlight identically
            // — and the syntax role WINS the flat Code tint for these bytes because
            // this span is laid AFTER the body `Code` span (last-wins on overlap).
            a = a.family(Family::Monospace);
            natural = Some(syn_role_color(role).to_glyphon());
        }
        MdKind::LinkText => {
            // Link TEXT reads in the buffer's full CONTENT ink. It sits OVER the
            // whole-range dim `Markup` span (brackets + url), so it must set content
            // EXPLICITLY to lift back off that dim — the link then reads by its muted
            // []()-markup, not by spending the accent. DESIGN §3: `primary` (amber) is
            // the caret and ONLY the caret.
            natural = Some(th.base_content.to_glyphon());
        }
    }
    if let Some(c) = color_override.or(natural) {
        a = a.color(c);
    }
    a
}

/// Lay the markdown styling spans that intersect ONE buffer line over `al`. Maps
/// each document-byte span in `md_spans` into this line's local byte range
/// (`line_doc_start` is the line's first byte in the document) and adds it with
/// [`md_attrs`]. Spans are applied in their stored order so the intentional
/// link/code-block overlaps (whole-range dim, then inner content) resolve
/// correctly. `color_override` carries the focus ink when this line sits in the
/// active unit; otherwise `None`. No-op when `md_spans` is empty (non-markdown
/// buffers), keeping their render byte-identical.
pub(super) fn add_md_line_spans(
    al: &mut glyphon::cosmic_text::AttrsList,
    line_text: &str,
    line_doc_start: usize,
    base: &Attrs<'static>,
    md_spans: &[(std::ops::Range<usize>, crate::markdown::MdKind)],
    color_override: Option<glyphon::Color>,
) {
    add_line_spans(al, line_text, line_doc_start, base, md_spans, color_override, md_attrs);
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
    color_override: Option<glyphon::Color>,
    attrs_fn: impl Fn(&Attrs<'static>, K, Option<glyphon::Color>) -> Attrs<'static>,
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
            al.add_span(local, &attrs_fn(base, *kind, color_override));
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
/// with its depth-derived glyph (`•`/`◦`/`▪`, drawn as an ornament on the SAME row —
/// see [`super::TextPipeline::bullet_marks`]) instead of the raw dash. The marker's
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

/// SYNTAX HIGHLIGHTING: the SINGLE PLACE the four Alabaster role colors are
/// derived. There is NO per-theme syntax palette and no new `Theme` field — the
/// colors are computed from the active world's EXISTING tokens, so "the theme just
/// slides on top" automatically across all 14 worlds. The philosophy
/// (tonsky's Alabaster) is figure/ground by VALUE: the structural code (keywords,
/// operators, identifiers, punctuation) keeps the FULL ink, and the four roles
/// recede into MUTED, low-saturation tints — never a loud hue and NEVER amber
/// (DESIGN.md §3: `primary` is the caret alone). The whole ramp lives on the
/// `base_content` → `muted` axis, which on every theme already carries
/// that world's own muted, low-saturation hue, so the roles inherit it for free:
/// - `Comment`    → `muted` (the dimmest — recedes exactly like markdown
///   markup).
/// - `Definition` → `base_content` lerped 12% toward dim (the most present role:
///   the defined name barely softens off the full ink).
/// - `Constant`   → 28% toward dim.
/// - `Str`        → 44% toward dim (the quietest literal, but now clearly present).
///
/// These value steps were re-tuned once code moved to a MONOSPACE grid (per-world
/// `Theme::mono`): on the tighter mono column the old 18/34/52 ramp read faint, so
/// the roles were pulled ~6-8 points MORE PRESENT (18→12, 34→28, 52→44) while
/// keeping the same monotone ordering and ~0.16 separation between rungs — more
/// legible, still value-only, still never amber (DESIGN §3).
///
/// `color_override` is the FOCUS-mode ink: when `Some`, it replaces the role color
/// so the active unit brightens uniformly (matching the markdown focus seam).
pub(super) fn syn_attrs(
    base: &Attrs<'static>,
    kind: crate::syntax::SynKind,
    color_override: Option<glyphon::Color>,
) -> Attrs<'static> {
    let mut a = base.clone();
    a = a.color(color_override.unwrap_or(syn_role_color(kind).to_glyphon()));
    a
}

/// The Alabaster ROLE COLOR for a syntax `kind`, in the theme's own `Color`. The
/// SINGLE derivation of the four role tints, on the `base_content` → `muted` value
/// ramp (never amber; DESIGN §3): Comment recedes fully to `muted`, then the
/// literals soften progressively (Definition 12% / Constant 28% / Str 44% toward
/// dim — the more "literal", the quieter). Shared by
/// [`syn_attrs`] (code buffers) AND [`md_attrs`]'s `CodeSyntax` arm (fenced code in
/// markdown), so a fenced highlight and a code-buffer highlight derive identically.
pub(super) fn syn_role_color(kind: crate::syntax::SynKind) -> theme::Srgb {
    use crate::syntax::SynKind;
    let th = theme::active();
    let full = th.base_content;
    let dim = th.muted;
    match kind {
        SynKind::Comment => dim,
        SynKind::Definition => lerp_srgb(full, dim, 0.12),
        SynKind::Constant => lerp_srgb(full, dim, 0.28),
        SynKind::Str => lerp_srgb(full, dim, 0.44),
    }
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
    color_override: Option<glyphon::Color>,
) {
    add_line_spans(al, line_text, line_doc_start, base, syn_spans, color_override, syn_attrs);
}

/// FOCUS re-application: lay the md/syn spans that fall INSIDE the active-unit
/// colored window (`byte_lo..byte_hi`, line-local) back over `al` with the focus
/// `color` as the attrs override, so the brightened active unit keeps its
/// bold/italic/mono/heading/role styling while taking the full ink. Each span is
/// first clamped to the line (`line_byte_start..line_byte_start+text_len`), then
/// intersected with the focus window. Shared by the markdown and syntax passes.
pub(super) fn add_focus_overlay_spans<K: Copy>(
    al: &mut glyphon::cosmic_text::AttrsList,
    spans: &[(std::ops::Range<usize>, K)],
    line_byte_start: usize,
    text_len: usize,
    byte_lo: usize,
    byte_hi: usize,
    lb: &Attrs<'static>,
    color: glyphon::Color,
    attrs_fn: impl Fn(&Attrs<'static>, K, Option<glyphon::Color>) -> Attrs<'static>,
) {
    for (r, kind) in spans {
        let s = r.start.max(line_byte_start);
        let e = r.end.min(line_byte_start + text_len);
        if s < e {
            let cl = (s - line_byte_start).max(byte_lo);
            let ch = (e - line_byte_start).min(byte_hi);
            if cl < ch {
                al.add_span(cl..ch, &attrs_fn(lb, *kind, Some(color)));
            }
        }
    }
}

/// The font / line-height SCALE for ONE buffer line, driven by its LEADING `#`
/// run: `# ` → h1, `## ` → h2, `###`+ → h3 (see [`crate::markdown::heading_scale`]).
/// Keyed off the raw hash COUNT, NOT a fully-valid ATX heading, so a line grows the
/// instant you type `#` — before the space and title (and even for `#foo`). Only
/// the LEADING run counts (after optional indent), so a `#` mid-prose is ignored.
/// `md` gates it: a non-markdown buffer (and any line with no leading hash) returns
/// the byte-identical `1.0`. The DIM-markup + bold-weight styling still comes from
/// the pulldown spans in [`md_attrs`]; this governs SIZE alone, so an in-progress
/// `#foo` is big but not yet bold until it becomes a real heading.
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
    if hashes == 0 {
        return 1.0;
    }
    crate::markdown::heading_scale(hashes)
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
/// editable and no ornament is drawn. This is the SINGLE recipe shared by
/// [`TextPipeline::set_text_incremental`] and [`TextPipeline::restyle_all_lines`],
/// so the two paths can never drift on layer ordering or membership.
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
    cjk: Option<(&'static str, glyphon::Weight)>,
    conceal_off_cursor: bool,
) -> glyphon::cosmic_text::AttrsList {
    let scale = md_line_scale(line_text, md);
    let lb = scaled_base_attrs(base, base_font_size, base_line_height, scale);
    let mut al = glyphon::cosmic_text::AttrsList::new(&lb);
    add_md_line_spans(&mut al, line_text, line_doc_start, &lb, md_spans, None);
    add_syn_line_spans(&mut al, line_text, line_doc_start, &lb, syn_spans, None);
    add_cjk_spans(&mut al, line_text, &lb, cjk);
    add_symbol_spans(&mut al, line_text, &lb);
    // REVEAL-ON-CURSOR: when the caret is off this line, conceal a thematic break's
    // raw `---` (leaving the fleuron) AND a bullet's raw `-` (leaving the depth glyph).
    // Both are drawn as ornaments on the SAME rows; on the caret's own line the raw
    // markup reveals for editing and no ornament is drawn.
    if conceal_off_cursor {
        add_rule_conceal_span(&mut al, line_text, line_doc_start, &lb, md_spans);
        add_bullet_conceal_span(&mut al, line_text, &lb);
    }
    al
}
