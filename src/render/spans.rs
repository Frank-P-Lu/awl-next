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
            // WASH quad drawn behind it (`rects.rs::ensure_wash_protos`, reusing the
            // warm comment-wash tint/pipeline — see `role_style_for`'s `Comment` arm),
            // never a text color change. Never amber (DESIGN §3).
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

/// SYNTAX HIGHLIGHTING: apply THE role style ([`role_style_for`], the one owner)
/// to one span's attrs. The structural code (keywords, operators, identifiers,
/// punctuation) keeps the FULL ink; only the roles take a style — quiet,
/// desaturated per-world tints, never a loud hue and NEVER amber (DESIGN.md §3:
/// `primary` is the caret alone). `color_override` is the FOCUS-mode ink: when
/// `Some`, it replaces the role color so the active unit brightens uniformly
/// (matching the markdown focus seam). The role's optional background WASH is
/// drawn by the wash pipelines (see `rects.rs::wash_rects`), not through attrs.
pub(super) fn syn_attrs(
    base: &Attrs<'static>,
    kind: crate::syntax::SynKind,
    color_override: Option<glyphon::Color>,
) -> Attrs<'static> {
    let mut a = base.clone();
    a = a.color(
        color_override.unwrap_or(role_style_for(&theme::active(), kind).fg.to_glyphon()),
    );
    a
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
/// the 8 dark worlds is now redmean 86 vs `base_content` (Kingfisher) and 59 vs
/// `Constant` (Undertow), both comfortably clear of their floors, while the
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
