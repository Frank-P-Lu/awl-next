//! src/theme/cjk.rs — the per-world CJK fallback LADDERS + the [`FontId`]
//! script-identity enum they're keyed by (Japanese mincho/gothic/variety,
//! Simplified Chinese serif/sans/Klee, Traditional Chinese, Korean
//! sans/serif) plus the bundled-face membership list [`EMBEDDED_CJK_FAMILIES`].
//! See [`crate::theme::worlds`] for which world picks which ladder.

// --- Per-theme CJK fallback families (mincho / gothic) ---------------------
//
// Two prioritized lists, BUNDLED-first then system fallback. The "Japanese
// bundle round" (TASTE-GATED — see CLAUDE.md) added Noto Serif JP / Noto Sans
// JP as embedded faces (`render::FONT_CJK_FACES`, a JIS X 0208 subset of the
// Google-Fonts JP-scoped builds); they are listed FIRST so a Japanese run
// resolves without depending on any system font. Hiragino (macOS) / Noto CJK
// (Linux) stay as TRAILING candidates for now — belt-and-suspenders while the
// user eyeballs the gallery/jp-compare captures (bundled Noto vs system
// Hiragino) and picks a winner. Only after that nod should this collapse to
// bundled-only (dropping the system entries + simplifying `resolve_cjk`'s
// weight-matching, which exists purely because system faces don't register at
// the default Weight 400) — a deliberate two-step, not an oversight.

/// MINCHO (serif) Japanese fallback for the SERIF worlds: bundled Noto Serif
/// JP first, then Hiragino Mincho ProN (macOS) / Noto Serif CJK JP (Linux).
pub const CJK_MINCHO: &[&str] = &["Noto Serif JP", "Hiragino Mincho ProN", "Noto Serif CJK JP"];

/// GOTHIC (sans) Japanese fallback for the SANS / MONO worlds: bundled Noto
/// Sans JP first, then Hiragino Kaku Gothic ProN (macOS) / Noto Sans CJK JP
/// (Linux).
pub const CJK_GOTHIC: &[&str] = &["Noto Sans JP", "Hiragino Kaku Gothic ProN", "Noto Sans CJK JP"];

// --- Phase 2 "JP face variety" round: per-WORLD Japanese overrides ----------
//
// The user's note: "with kana we probably want a couple more — they don't
// really change much across themes." Latin varies per world; Japanese used to
// resolve to just two faces (CJK_MINCHO / CJK_GOTHIC). These three ladders each
// name a distinct-character bundled JP face FIRST (see
// `render::FONT_JA_VARIETY_FACES`), then fall back to the SAME Noto bundled
// FLOOR their neutral sibling uses, then the identical system candidates — so
// the never-tofu floor is unchanged and `AWL_CJK_FORCE=floor` cleanly drops
// each to its plain Noto face for the before/after `gallery/jp-worlds/`
// captures. See THEMES.md's assignment table for which world gets which + why.

/// JAPANESE bookish-mincho ladder — the warm LITERARY serif for the book-serif
/// worlds (Gumtree, Bilby, Undertow): bundled Shippori Mincho first, then the
/// Noto Serif JP floor + the same Hiragino/Noto-CJK system trailing candidates
/// as [`CJK_MINCHO`].
pub const CJK_JA_SHIPPORI: &[&str] =
    &["Shippori Mincho", "Noto Serif JP", "Hiragino Mincho ProN", "Noto Serif CJK JP"];

/// JAPANESE rounded-gothic ladder — the warm rounded "maru" sans for the two
/// dedicated sans worlds (Galah, Kingfisher): bundled Zen Maru Gothic first,
/// then the Noto Sans JP floor + the same gothic system trailing candidates as
/// [`CJK_GOTHIC`].
pub const CJK_JA_ZENMARU: &[&str] =
    &["Zen Maru Gothic", "Noto Sans JP", "Hiragino Kaku Gothic ProN", "Noto Sans CJK JP"];

/// JAPANESE Klee ladder — the CHARACTERFUL kaisho/brush override for the two
/// Klee-derived worlds (Mopoke, Quokka), so their JA shares the brush character
/// of their ZH (LXGW WenKai, a Klee One-derived Chinese face — see
/// [`CJK_ZH_HANS_KLEE`], whose doc anticipated exactly this pairing): bundled
/// Klee One first, then the Noto Sans JP floor + gothic system candidates.
pub const CJK_JA_KLEE: &[&str] =
    &["Klee One", "Noto Sans JP", "Hiragino Kaku Gothic ProN", "Noto Sans CJK JP"];

/// The bundled CJK family names — the "embedded" side of the [`FontId`]
/// resolver's asset-source classification (also the `apply_cjk_force` A/B
/// switch's "bundled" set). Data, not a code path: [`crate::theme::Theme::candidates`]
/// returns plain family-name ladders for every [`FontId`], and a name here is
/// simply one that's ALWAYS present (loaded in `build_font_system`) rather
/// than one that may or may not be installed on this machine. Extended by the
/// "Chinese round" with the four new bundled faces (`render::FONT_ZH_KO_FACES`)
/// alongside the JP-bundle round's original two, so `TextPipeline::
/// script_font_report`'s `bundled` flag is accurate for zh-Hans/ko too.
pub(crate) const EMBEDDED_CJK_FAMILIES: &[&str] = &[
    "Noto Serif JP",
    "Noto Sans JP",
    "Noto Serif SC",
    "Noto Sans SC",
    "Noto Sans KR",
    "LXGW WenKai",
    // Phase 2 "JP face variety" round (`render::FONT_JA_VARIETY_FACES`).
    "Shippori Mincho",
    "Zen Maru Gothic",
    "Klee One",
    // "CJK companions" round (`render::FONT_CJK_COMPANION_FACES`) — the serif
    // worlds' Korean serif override.
    "Gowun Batang",
];

// --- i18n ROUND: per-script font IDs + candidate ladders --------------------
//
// [`FontId`] names the per-script font IDENTITY awl resolves independently:
// the world's own Latin display face, plus the four CJK-family scripts this
// round adds ladders for. [`crate::theme::Theme::candidates`] maps an ID to a PRIORITIZED
// family-name ladder (bundled-first where one exists); the resolver
// (`render/text.rs::TextPipeline::resolve_font_id`) walks it and returns the
// first family actually registered in the font DB — exactly `resolve_cjk`'s
// existing algorithm, now shared across five IDs instead of hard-coded to one.
//
// V1 TASTE CALL (logged, not hidden), UPDATED by the "Chinese round": ja
// keeps its existing bundled mincho/gothic split (`Theme::cjk`, unchanged —
// the JP-bundle round already shipped it). The Chinese round gives zh-Hans
// the SAME bundled-first treatment: Noto Serif SC / Noto Sans SC (the
// user's own 思源宋体/思源黑体 pick — "Source Han" is Adobe/Google's shared
// name for the Noto CJK SC family) mirror the mincho/gothic serif/sans split
// exactly, PLUS a per-world CHARACTERFUL override for the two Klee-derived
// worlds (Mopoke, Quokka) — LXGW WenKai (霞鹜文楷), a Klee One-derived
// Chinese face, so ja and zh-Hans share the same brush character on those
// two worlds. zh-Hant and ko stay v1 system-only in one respect each: ko
// now bundles Noto Sans KR first (one face, no serif/sans split — a v1 taste
// call, logged: there is no comparable bundled serif KR companion yet), but
// zh-Hant remains FULLY system-only (PingFang TC / Noto Sans CJK TC) — a
// Big5-class Traditional-Chinese subset (~13k chars) is banked, not bundled,
// this round: Big5 coverage is a genuinely bigger lift (~13k chars vs GB
// 2312's ~6.8k), so it is EXPLICITLY BANKED rather than attempted here (see
// THEMES.md's Han-unification note) — TC keeps borrowing the system
// PingFang TC / Noto Sans CJK TC pair exactly as before this round.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum FontId {
    /// The world's own Latin display/mono face (never a fallback — always
    /// resolves to the currently-shaping doc family, itself an embedded face).
    Latin,
    /// Japanese: kana + (contextually) Han. See [`crate::theme::Theme::cjk`].
    Ja,
    /// Simplified Chinese: Han. See [`crate::theme::Theme::zh_hans`].
    ZhHans,
    /// Traditional Chinese: Han + Bopomofo. See [`crate::theme::Theme::zh_hant`].
    ZhHant,
    /// Korean: Hangul + (contextually) Han. See [`crate::theme::Theme::ko`].
    Ko,
}

/// Every [`FontId`] variant — the never-tofu law test's sweep list, kept in
/// lockstep with the enum by hand (a `match` elsewhere enumerating `FontId`
/// with a no-wildcard arm is the actual compile-time guard; this is for
/// iteration convenience in tests).
pub const ALL_FONT_IDS: [FontId; 5] =
    [FontId::Latin, FontId::Ja, FontId::ZhHans, FontId::ZhHant, FontId::Ko];

/// Simplified Chinese SERIF ladder — the "Chinese round"'s zh-Hans mincho
/// companion, for the SERIF worlds (`Theme::cjk == CJK_MINCHO`): bundled Noto
/// Serif SC first (Google Fonts' Source Han Serif SC build, OFL, subset to
/// GB 2312 — see `render::FONT_ZH_KO_FACES`), then the system PingFang SC
/// (macOS) / Noto Sans CJK SC (Linux) trailing candidates — mirrors
/// [`CJK_MINCHO`]'s bundled-first shape exactly.
pub const CJK_ZH_HANS_SERIF: &[&str] = &["Noto Serif SC", "PingFang SC", "Noto Sans CJK SC"];

/// Simplified Chinese SANS ladder — the gothic companion, for the SANS/MONO
/// worlds (`Theme::cjk == CJK_GOTHIC`): bundled Noto Sans SC first, then the
/// same system trailing candidates as [`CJK_ZH_HANS_SERIF`].
pub const CJK_ZH_HANS_SANS: &[&str] = &["Noto Sans SC", "PingFang SC", "Noto Sans CJK SC"];

/// Simplified Chinese KLEE ladder — the CHARACTERFUL per-world override for
/// the two Klee-derived worlds (Mopoke, Quokka): bundled LXGW WenKai
/// (霞鹜文楷, OFL, github.com/lxgw/LxgwWenKai — a Klee One-derived Chinese
/// face, subset to GB 2312) FIRST, so ja and zh-Hans share the same brush
/// character on those two worlds, then falls back through the same Noto Sans
/// SC floor + system trailing candidates as [`CJK_ZH_HANS_SANS`] (Mopoke/
/// Quokka are both sans/mono worlds) if WenKai is ever unavailable. A TASTE
/// CALL (logged): this pairing anticipates the (separately landed, not yet
/// merged into this branch) "JP world-faces round"'s Klee One ↔ Mopoke/Quokka
/// assignment — see CLAUDE.md's Chinese-round report for the exact status.
pub const CJK_ZH_HANS_KLEE: &[&str] =
    &["LXGW WenKai", "Noto Sans SC", "PingFang SC", "Noto Sans CJK SC"];

/// Traditional Chinese v1 ladder: PingFang TC (macOS) then Noto Sans CJK TC
/// (Linux). STILL no bundled asset — Big5 coverage (~13k chars) is banked,
/// not attempted, this round; see the module note above.
pub const CJK_ZH_HANT: &[&str] = &["PingFang TC", "Noto Sans CJK TC"];

/// Korean SANS ladder — the Chinese round's "KO rider", now the SANS/MONO
/// worlds' ko floor after the CJK-companions round's serif split: bundled Noto
/// Sans KR first (Google Fonts, OFL, subset to KS X 1001 modern hangul + jamo —
/// see `render::FONT_ZH_KO_FACES`), then Apple SD Gothic Neo (macOS) / Noto
/// Sans CJK KR (Linux) trailing. The SERIF worlds get [`CJK_KO_SERIF`] instead.
pub const CJK_KO: &[&str] = &["Noto Sans KR", "Apple SD Gothic Neo", "Noto Sans CJK KR"];

/// Korean SERIF ladder — the "CJK companions" round's Gowun Batang rider, for
/// the SERIF worlds (`Theme::cjk` is a mincho-family ja ladder: Gumtree, Bilby,
/// Undertow, Saltpan, Outback, Magpie). Bundled Gowun Batang (a Korean BATANG /
/// serif, OFL, subset to the SAME KS X 1001 set as the Noto Sans KR floor — see
/// `render::FONT_CJK_COMPANION_FACES`) FIRST, closing the Chinese round's logged
/// gap ("no comparable bundled serif Korean companion yet"), then the SAME
/// bundled Noto Sans KR floor + serif-then-sans system trailing candidates —
/// mirroring [`CJK_JA_SHIPPORI`]'s "characterful serif first, neutral bundled
/// floor next, system last" shape. There is no NEUTRAL bundled serif Korean
/// floor, so the guaranteed floor stays the (sans) Noto Sans KR — which is
/// exactly what `AWL_CJK_FORCE=floor` drops to for the `gallery/ko-worlds/`
/// characterful-vs-floor A/B (Gowun Batang is a [`CHARACTERFUL_CJK_FAMILIES`]
/// member). AppleMyungjo (macOS) / Noto Serif CJK KR (Linux) are serif system
/// candidates reached only under `AWL_CJK_FORCE=system` (all bundled faces
/// pruned), so a serif world's `system` capture still reads as a serif Korean.
pub const CJK_KO_SERIF: &[&str] = &[
    "Gowun Batang",
    "Noto Sans KR",
    "AppleMyungjo",
    "Noto Serif CJK KR",
    "Apple SD Gothic Neo",
    "Noto Sans CJK KR",
];
