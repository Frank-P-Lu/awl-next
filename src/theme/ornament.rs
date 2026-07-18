//! src/theme/ornament.rs — the per-world SECTION-BREAK ornament trio + the
//! per-world LIST-BULLET pair (the ornament trio, one level down): the shared
//! [`Ornaments`] type, the three ornament FACE constants, the three ornament
//! SCALE tiers, and the two bullet-scale tiers. See [`crate::theme::worlds`]
//! for how each of the sixteen worlds picks from this data.

// --- The PER-SYNTAX thematic-break ornament set -----------------------------

/// The PER-SYNTAX thematic-break ornament set — one glyph for each of markdown's
/// three `<hr>` spellings, so a break's ORNAMENT tracks what the author typed:
/// `---` (dash), `***` (star), `___` (underscore). Each renders CENTERED in the
/// writing column from the bundled `SYMBOL_FAMILY` face (see
/// [`crate::render::spans::is_symbol`]), and is REVEALED back to its raw characters
/// when the caret lands on the line (reveal-on-cursor). The three defaults live in
/// [`ORNAMENTS_DEFAULT`]; a world may override for its own face's flavour.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Ornaments {
    /// `---` (a dash rule) → the fleuron. Default ❧ (U+2767).
    pub dash: char,
    /// `***` (a star rule) → the asterism — three stars for three asterisks, the
    /// natural match. Default ⁂ (U+2042).
    pub star: char,
    /// `___` (an underscore rule) → the floral heart. Default ❦ (U+2766).
    pub underscore: char,
}

impl Ornaments {
    /// The ornament this world draws for a given break syntax.
    pub const fn pick(&self, kind: crate::markdown::BreakKind) -> char {
        match kind {
            crate::markdown::BreakKind::Dash => self.dash,
            crate::markdown::BreakKind::Star => self.star,
            crate::markdown::BreakKind::Underscore => self.underscore,
        }
    }
}

/// The shared DEFAULT ornament set: `---` → ❧ fleuron, `***` → ⁂ asterism (three
/// stars for three asterisks), `___` → ❦ floral heart. All three are bundled in
/// the merged `AwlMarks.ttf` (the [`ORNAMENT_MARKS`] face), so they render in
/// every world that keeps that face.
pub const ORNAMENTS_DEFAULT: Ornaments = Ornaments { dash: '❧', star: '⁂', underscore: '❦' };

// --- The per-world ORNAMENT FACE (the fleuron / About end-mark face) ----------
//
// Each world draws its markdown SECTION-BREAK ornament (the `---`/`***`/`___`
// fleuron) AND its About-card closing end-mark in its OWN assigned face, instead
// of the shared merged marks face. Keycaps (⌘⌥⇧) and the plain typographic marks
// (§ † ‡ • ◦ ▪ …) stay on the merged marks face (`render::SYMBOL_FAMILY`) — ONLY
// the section-break/About ornament changes face. The three faces (all bundled,
// all OFL) map to the three flavour registers:
//
//   * [`ORNAMENT_GARAMOND`] — EB Garamond's Renaissance fleurons (❧ ❦ ☙), for the
//     TRUE literary serifs (Bilby, Bombora). NOTE: EB Garamond ships NO ⁂ asterism
//     (nor ❡/❥) and only those THREE fleurons, so a Garamond world's trio is exactly
//     {❧, ☙, ❦} permuted — never ⁂ — see the NEVER-TOFU coverage test.
//   * [`ORNAMENT_JUNICODE`] — Junicode's antique Caslon flowers (❧ ❦ ☙ + the ⁂/⁑
//     asterisms + a deep pool of PUA botanical/damask/tile ornaments), for the
//     antique/expressive/slab worlds AND the warm/pale literary serifs whose display
//     face carries no fleurons of its own (Gumtree, Saltpan, Magpie, Mopoke, Mulga):
//     each gets a distinct in-character trio (a botanical sprig / running vine /
//     quatrefoil-tile / damask-flourish / typographic-asterism family, respectively).
//   * [`ORNAMENT_MARKS`] — the merged marks face itself (`render::SYMBOL_FAMILY`),
//     for the modern/technical/GEOMETRIC worlds: it carries the Noto Sans Symbols
//     2 geometric marks (its ❡ ❥ come from NS2; ❧ ❦ ☙ from EB Garamond; ⁂ from
//     Junicode). There is no STANDALONE "Noto Sans Symbols 2" registered face —
//     its glyphs live in this merged face, which is exactly the clean geometric
//     look the technical worlds want, so they simply keep it (their ornament is
//     byte-identical to before this round).

/// The EB Garamond ornament face — refined Renaissance fleurons for the literary
/// serif worlds. Registered from `EBGaramond-Regular.ttf` (also Bombora's own
/// display face). Covers ❧ ❦ ☙ but NOT ⁂/❡/❥.
pub const ORNAMENT_GARAMOND: &str = "EB Garamond";

/// The Junicode ornament face — antique Caslon flowers for the expressive/slab
/// worlds. Registered from `Junicode-Ornaments.ttf`. Covers ❧ ❦ ☙ ⁂ ⁑ + PUA
/// fleuron clusters (NOT ❡/❥).
pub const ORNAMENT_JUNICODE: &str = "Junicode";

/// The merged marks face (== `render::SYMBOL_FAMILY`, `AwlMarks.ttf`) — the
/// geometric/technical worlds' ornament face. Carries the Noto Sans Symbols 2
/// geometric marks; covers the default ornaments (❧ ❦ ☙ ❡ ❥ ⁂) PLUS the expanded
/// star/floret/geometric pool this round draws its per-world trios from (✦ ✧ ✴ ✶
/// ✷ ✽ ✿ ❀ ❁ ❂ ❖ ◆ ◈ ⬥ ⭑). Naming the constant here keeps `theme.rs` free of a
/// `crate::render` dependency in the `const` world literals; the two are asserted
/// equal by a test.
pub const ORNAMENT_MARKS: &str = "Awl Marks";

// --- The per-world ORNAMENT SCALE (how big the section-break fleuron reads) ----
//
// A thematic-break line (`---`/`***`/`___`) grows its whole ROW by a scale factor
// so the centered ornament reads as a generous flourish (the size counterpart of
// the leading-`#` heading scan). That scale is now PER-WORLD ([`crate::theme::Theme::ornament_scale`]),
// keyed to the ornament's CHARACTER — the detailed flowers reward size, the clean
// geometric marks don't — in three tiers that line up with the three ornament faces:
//
//   * ORNATE   — the [`ORNAMENT_JUNICODE`] Caslon flowers (antique/expressive worlds).
//   * FLEURON  — the [`ORNAMENT_GARAMOND`] Renaissance fleurons (literary serifs).
//   * GEOMETRIC — the [`ORNAMENT_MARKS`] stars/florets/diamonds (modern/technical).
//
// The field is read by BOTH `render::spans::md_line_scale` (the break ROW height)
// and `render::layers::prepare_ornaments` (the glyph LINE-BOX), so the two never
// drift — the tall row always centers the glyph. These are TASTE DEFAULTS: one
// dial per tier, tuned from the gallery.

/// ORNATE ornament scale — the Junicode Caslon-flower worlds. The most detailed
/// ornaments carry the most size.
pub const ORNAMENT_SCALE_ORNATE: f32 = 2.2;

/// FLEURON ornament scale — the EB Garamond literary-serif worlds. A generous but
/// slightly quieter flourish than the ornate flowers.
pub const ORNAMENT_SCALE_FLEURON: f32 = 1.8;

/// GEOMETRIC ornament scale — the Awl Marks stars/florets/diamonds. The clean
/// geometric marks read best kept modest, so they sit lowest on the tier ladder.
pub const ORNAMENT_SCALE_GEOMETRIC: f32 = 1.5;

// --- The per-world LIST BULLET pair + scale (the ornament trio, one level down) --
//
// The unordered-list bullet ([`crate::theme::Theme::bullets`], drawn over a concealed `-`/`*`/`+`
// the caret is off) is PER-WORLD DATA in the world's own [`crate::theme::Theme::ornament_face`] —
// the same face + `no-new-machinery` discipline as the section-break fleuron trio,
// scoped by flavour:
//
//   * The MODERN / TECHNICAL / geometric worlds (the [`ORNAMENT_MARKS`] worlds) keep
//     the plain [`BULLETS_PLAIN`] `•`/`◦` at [`BULLET_SCALE_PLAIN`] (byte-identical to
//     before this round) — restraint IS their character; a bullet is not the place to
//     decorate them for symmetry.
//   * The ANTIQUE / LITERARY serifs (the [`ORNAMENT_JUNICODE`] + [`ORNAMENT_GARAMOND`]
//     worlds) draw a small hedera / fleuron — and Bombora the antique MANICULE ☞ —
//     at [`BULLET_SCALE_ORNAMENT`] (~half body), so the mark reads as quiet list
//     RHYTHM, never a loud flourish. Garamond ships the manicule (☞ U+261E) and the
//     three fleurons ❧ ❦ ☙; Junicode ships ❧ ❦ ☙ ⁑ (no plain `•`), so those worlds'
//     pairs come from that pool — every pick verified present in its own face by
//     `render::tests::markdown::bullet_glyphs_resolve_in_each_worlds_assigned_face`.

/// The plain geometric bullet pair — level-1 `•` (U+2022) / level-2 `◦` (U+25E6),
/// both in the merged [`ORNAMENT_MARKS`] face — the modern/technical worlds' bullets
/// (byte-identical to the pre-round `•`/`◦` levels).
pub const BULLETS_PLAIN: (char, char) = ('•', '◦');

/// PLAIN bullet scale — the geometric `•`/`◦` worlds keep body size (1.0), so their
/// bullets render exactly as before this round.
pub const BULLET_SCALE_PLAIN: f32 = 1.0;

/// ORNAMENT bullet scale — a hedera / fleuron / manicule shaped at ~half body so it
/// reads as a quiet bullet-sized marker, not a section-break flourish. A TASTE
/// DEFAULT (one dial for every characterful world), tuned from the veto gallery.
pub const BULLET_SCALE_ORNAMENT: f32 = 0.55;
