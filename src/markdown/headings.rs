//! The heading SIZE ladder ([`type_scale`]/[`heading_scale`]) and the
//! [`Heading`] extraction ([`headings`]/[`headings_from_spans`]) that feeds
//! the go-to-heading picker. Split out of the former `markdown.rs` monolith
//! (2026-07 code-organization pass); every item's path is unchanged
//! (`markdown::heading_scale`, `markdown::Heading`, …) -- only the file it
//! lives in moved.

use super::spans::spans;
use super::MdKind;
use std::ops::Range;

/// The TYPE SCALE — awl's SIZE LADDER, one of the two ladders in the text system
/// (the other is the ink ramp in `theme/`: `base_content` / `muted` / `faint`).
/// Every element is exactly ONE ink × ONE size (DESIGN.md §4), and these named
/// tiers are the size half: each is a multiplier over the body metrics. Naming the
/// rungs (rather than scattering bare `1.8`/`1.5` literals) makes the ladder
/// explicit and keeps the ratios tunable in ONE place.
pub mod type_scale {
    /// h1 — the document / top TITLE (the biggest rung). LADDER J (the
    /// heading-probe pick, user-decided 2026-07-18, superseding the original
    /// 1.8/1.5/1.25 ladder): the title stays Regular WEIGHT always (see
    /// [`super::heading_weight_bold`] — `#` never bolds, on any world), so it
    /// spends SIZE alone; 1.6 buys clear daylight over the SECTION rung below
    /// it (which may carry per-world bold) without shouting the way 1.8 did.
    pub const TITLE: f32 = 1.6;
    /// h2 — a SECTION head. LADDER J: 1.3 — size spends less here than the old
    /// 1.5; on the worlds whose [`crate::theme::Theme::heading_bold`] bit is
    /// set, WEIGHT backfills the difference.
    pub const SECTION: f32 = 1.3;
    /// h3+ — a SUBHEAD. LADDER J: 1.15 — one quiet step over body, weight
    /// (where the world's bit grants it) doing the rest.
    pub const SUBHEAD: f32 = 1.15;
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
    // the two stay in lockstep. Tune the three tiers in `theme/ornament.rs`
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

/// The ROW-HEIGHT LEAD — vertical breathing room a heading's ROW grows
/// BEYOND what its own [`heading_scale`] SIZE already gives its font, decoupled
/// from glyph size exactly like an inline image's absolute row height
/// (`render::spans::build_line_attrs`'s `image_row_height` arm). THEME-QA
/// round (2026-07-22): a no-bold world (`Theme::heading_bold == false` —
/// Bombora, Mulga, …) has neither WEIGHT nor, at the SUBHEAD rung, much SIZE
/// to carry `###` above body text — measured, its row-to-row gap around an h3
/// was pixel-identical to the gap between two ordinary body paragraphs (the
/// reported bug: "h3 reads as body"). A real spacing floor gives every
/// world's heading hierarchy a SECOND axis to read on (size AND space, not
/// size alone), universal DATA keyed by LEVEL — never a per-world branch, so
/// it helps the no-bold worlds where it is load-bearing and costs the bold
/// worlds only a little extra (still calibrated small enough that Tawny's
/// already-legible weight+size break doesn't grow into an ungainly gap — see
/// `render::tests::markdown_headings::heading_levels_stay_measurably_distinct_from_body_in_every_world`).
///
/// Rungs run OPPOSITE `heading_scale`'s own direction: SUBHEAD has the LEAST
/// size to lean on, so it gets the MOST lead; TITLE already commands the room
/// via 1.6x size alone, so it gets the least. `0` (body / a non-heading line,
/// including a thematic break — see `render::spans::md_line_scale`) is
/// exactly `1.0`, so every non-heading row's height is BYTE-IDENTICAL to
/// before this constant existed.
pub fn heading_row_lead(level: u8) -> f32 {
    match level {
        0 => 1.0,
        1 => 1.15,
        2 => 1.26,
        _ => 1.34,
    }
}

/// THE ONE OWNER of "does THIS heading level shape at real BOLD weight?" —
/// the weight half of the heading ladder, beside [`heading_scale`]'s size
/// half. Two facts compose here and nowhere else:
///
///  - **The per-world ONE BIT** (`theme_bit`, the caller passes its world's
///    [`crate::theme::Theme::heading_bold`]): whether this world's display
///    face wants weight in its hierarchy at all. Serif worlds lean `false`
///    (a serif's stroke contrast carries hierarchy structurally); the
///    mono-display worlds lean `true` (uniform strokes need weight); see the
///    per-world reasoning in `theme/worlds.rs`.
///  - **The level gate**: TITLE (`#`) NEVER bolds, on any world, under any
///    override — Ladder J spends pure SIZE there (1.6x) — so only SECTION
///    (`##`) and SUBHEAD (`###`+) take the world's bit. `0` (a non-heading
///    line) is always `false`.
///
/// The render seam is `render/spans.rs::md_attrs`'s `MdKind::Heading` arm
/// (mirroring `MdKind::Bold`'s real bundled-700-face request — every display
/// family ships a genuine Bold under its own family name, so this is a real
/// weight change, never synthetic); the capture sidecar reports the same
/// composition (`theme.heading_bold`), so renderer and oracle can't drift.
///
/// The dev knob `AWL_HEADING_BOLD_FORCE=on|off` (env, CLI-invisible — the
/// `AWL_CJK_FORCE` precedent) overrides the BIT (never the level gate) so the
/// A/B galleries shoot both states without data edits; unset, it is a total
/// no-op and a default capture is a pure function of the world's own bit.
pub fn heading_weight_bold(theme_bit: bool, level: u8) -> bool {
    heading_weight_bold_with(heading_bold_force(), theme_bit, level)
}

/// [`heading_weight_bold`]'s PURE core, with the (memoized, process-wide) env
/// force injected as a plain argument so unit tests can exercise all three
/// force states without touching the environment: `Some(v)` replaces the
/// world's bit with `v`; `None` (the shipping default) respects it. The
/// TITLE-never-bold gate applies in every arm.
fn heading_weight_bold_with(force: Option<bool>, theme_bit: bool, level: u8) -> bool {
    let bit = force.unwrap_or(theme_bit);
    bit && level >= 2
}

/// The `AWL_HEADING_BOLD_FORCE` dev knob, read ONCE and memoized (the
/// `AWL_CJK_FORCE` read-once precedent): `"on"` → `Some(true)`, `"off"` →
/// `Some(false)`, anything else / unset → `None` (a total no-op — the
/// determinism promise: a capture with the env unset is a pure function of
/// the active world's data).
fn heading_bold_force() -> Option<bool> {
    static V: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *V.get_or_init(|| match std::env::var("AWL_HEADING_BOLD_FORCE").ok().as_deref() {
        Some("on") => Some(true),
        Some("off") => Some(false),
        _ => None,
    })
}

#[cfg(test)]
pub(crate) fn heading_weight_bold_with_for_tests(
    force: Option<bool>,
    theme_bit: bool,
    level: u8,
) -> bool {
    heading_weight_bold_with(force, theme_bit, level)
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
