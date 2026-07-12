//! src/theme/model.rs — the core palette DATA MODEL: [`Theme`] itself (the
//! per-world struct), its [`Background`] margin-ground union, the syntax
//! [`RoleOverrides`] escape hatch, and the theme-picker's [`Lens`]/[`ThemeTags`]
//! faceting types. See [`crate::theme::worlds`] for the fifteen concrete
//! [`Theme`] literals and [`crate::theme::derive`] for the active-theme
//! accessors that read them.

use super::cjk::FontId;
use super::color::Srgb;
use super::ornament::Ornaments;

/// PER-WORLD SYNTAX ROLE-STYLE OVERRIDES — the designed escape hatch for the
/// DERIVED role tints + washes (`render/spans.rs::role_style_for`, the one owner
/// of role color). FOURTEEN of the fifteen worlds ship [`RoleOverrides::NONE`]:
/// every role style is a pure function of the world's own palette (ink-ladder
/// lightness × fixed hue anchors). A world may PIN a role's foreground tint, PIN
/// a wash quad color (rgba — washes are computed quad colors, deliberately NOT
/// opaque theme tokens), or DISABLE a wash outright, without touching the shared
/// derivation. **Wagtail is the escape hatch's FIRST real use** (see its doc
/// comment in `worlds.rs`): a hue-anchored derivation cannot serve a
/// zero-saturation world by construction (an anchor IS a hue), so every one of
/// Wagtail's four role fgs + both washes is pinned to a plain grey instead. The
/// law test in `render/spans.rs` sweeps the EFFECTIVE style, so an override
/// can never smuggle a style past the distinguishability / amber-guard laws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoleOverrides {
    /// Pin the `Definition` foreground tint (None = derived).
    pub def_fg: Option<Srgb>,
    /// Pin the `Constant` foreground tint (None = derived).
    pub const_fg: Option<Srgb>,
    /// Pin the `Str` foreground tint (None = derived).
    pub str_fg: Option<Srgb>,
    /// Override the prose-COMMENT background wash (all worlds carry it by default).
    pub comment_wash: WashOverride,
    /// Override the STRING background wash (dark worlds only by default).
    pub str_wash: WashOverride,
}

/// One wash-override slot: ride the derivation, opt the world out, or pin an
/// exact rgba quad color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WashOverride {
    /// Use the derived wash (the default everywhere at launch).
    Default,
    /// NO wash for this role in this world (the opt-out — e.g. if a live eyeball
    /// rejects the warm comment wash on an OLED-black world).
    Off,
    /// Pin this exact rgba wash quad color.
    Pin(Srgb),
}

impl RoleOverrides {
    /// No overrides: every role style comes from the shared derivation. What
    /// fourteen of the fifteen worlds ship with (Wagtail is the exception —
    /// see [`Theme::role_overrides`]'s doc + `worlds.rs::WAGTAIL`).
    pub const NONE: RoleOverrides = RoleOverrides {
        def_fg: None,
        const_fg: None,
        str_fg: None,
        comment_wash: WashOverride::Default,
        str_wash: WashOverride::Default,
    };
}

// --- THEME CAPABILITIES AS DATA -------------------------------------------
//
// `RenderCaps` is the declarative capability contract every per-theme render
// BEHAVIOR routes through — the roadmap's "theme capabilities as data" head
// item. Before this round, a handful of render-side call sites branched
// directly on `Theme::is_one_bit()` (an ad hoc derived boolean) to decide
// things like "does selection draw as a translucent fill or a true inverted
// video mask" or "does the elevated card get a border". That worked while
// exactly one world (Wagtail) ever needed anything other than the default —
// but it meant a FUTURE theme wanting one of those same behaviors would have
// had to grow ANOTHER `is_one_bit()`-shaped special case rather than simply
// setting a field. `RenderCaps` names each of those render decisions as its
// own field with a plain enum/number value (TOML-ready shapes — no closures,
// no trait objects — though nothing here ships an on-disk parser; see
// `ROADMAP.md`'s "theme capabilities as data" entry). FOURTEEN of the fifteen
// worlds ship [`RenderCaps::DEFAULT`] byte-identically; Wagtail is simply DATA
// that sets every field away from its default (`worlds.rs::WAGTAIL`) — no
// world-name string comparison, no `is_one_bit()` read, anywhere in
// `src/render/**` (a structural law test, `render::tests::theme_caps_law`,
// bans both from ever reappearing there).
///
/// Whether document SELECTION paints as the ordinary translucent `selection`
/// fill, or as TRUE inverse video (`SelectionPipeline::new_invert`, an
/// `OneMinusDst` blend drawn after text) — the only mechanism that can render
/// "selected" on a world with no intermediate grey to fill with. See
/// `TextPipeline::selection_invert`'s field doc + `prepare_selection_layer`.
/// The SAME field also drives every OTHER "highlight a row/band" surface
/// that faces the identical constraint: the picker's selected-row value band
/// (`overlay_rows`/`overlay_rows_invert`, `render/chrome/overlay.rs`) and the
/// web/Linux menu bar's open-title band (`menubar_hi`/`menubar_hi_invert`,
/// `render/chrome/menubar.rs`) — a picker row or an open menu title is, in
/// this renderer's terms, just another "selected" region; a value-band fill
/// has the same "no legal intermediate grey" problem document selection does
/// on a one-bit world, and the System-7 answer is the same inversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionStyle {
    /// The default: a translucent `selection`-tinted quad under the text.
    Fill,
    /// True inverse video: `1 - dst` per channel, wherever the range covers.
    /// Also switches the SEARCH-MATCH quad + the `==highlight==`/dither
    /// texture over to the same mechanism family (see `HighlightTexture`).
    InverseVideo,
}

/// Whether the BLOCK caret draws as an ordinary opaque quad UNDER the glyph
/// (the default — the glyph composites over it normally), or must instead
/// route through the same true-inverse-video mechanism as `SelectionStyle`'s
/// `InverseVideo` case, because an opaque quad tinted this world's caret
/// color would be the exact same value as the glyph's own ink and erase it
/// (a caret landing on a heading's `#` on an all-white-ink world). MORPH mode
/// degrades to BLOCK under `InverseVideo` (see `prepare_caret_layer`) — a
/// glyph-shaped invert mask has no accent color to carry in a two-value
/// world. See `TextPipeline::caret_invert`'s field doc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaretBlockStyle {
    Normal,
    InverseVideo,
}

/// Whether a full-takeover overlay / the held HUD / the lifetime card /
/// hold-peek recedes the document behind a frosted GAUSSIAN BLUR (the
/// default), or must skip the blur entirely because a defocus of a purely
/// two-value document mathematically smears every edge into a forbidden
/// intermediate grey. `Flat` falls back to the pre-existing crisp path (the
/// same one the theme/caret pickers already use, doc stays bright, no
/// blur/scrim). See `TextPipeline::backdrop_blur`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backdrop {
    Blur,
    Flat,
}

/// Whether a summoned card's elevation reads as the ordinary FLAT `base_300`
/// fill (the default — depth is carried by the surface-ramp value step
/// alone), or must instead draw a crisp raised BORDER (`surface_selected()`'s
/// one-bit override, pure white) because the surface ramp has collapsed
/// (`base_200 == base_300`) and a flat fill would be an invisible card on an
/// identical ground. Also gates the picker's selected-ROW value band
/// (`overlay_rows`) OFF under `Bordered` — filling a whole row the SAME ink
/// as its own text would hide the text; the row's own caret still marks the
/// position. See `surface_selected()`, `prepare_panel_card_elevation`,
/// `render/chrome/overlay.rs`'s `overlay_rows`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elevation {
    Flat,
    Bordered,
}

/// Whether the renderer's small DECORATIVE translucent washes — the
/// floating-panel drop SHADOW (`float_shadow_srgba`) and the writing-nit
/// underline (`nit_underline_srgba`), both an ink/muted tone at a low,
/// non-edge alpha — are allowed to draw at all. `Off` forces both fully
/// transparent: any partial alpha over a world with only two legal values
/// would composite a forbidden intermediate grey, so the decorative wash is
/// simply skipped rather than tuned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecorativeWash {
    Enabled,
    Off,
}

/// Whether the inline-image reveal CAPTION SCRIM (`image_reveal_scrim`) draws
/// as its ordinary TRANSLUCENT veil over the dimmed image (the default), or
/// must be fully OPAQUE instead — the same "no partial alpha allowed"
/// constraint as [`DecorativeWash`], but the fallback here is full occlusion
/// (the caption's ground fully replaces the image) rather than "off", since
/// the scrim's geometry still needs to draw for the caption to read at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageReveal {
    Translucent,
    Opaque,
}

/// THE ONE emphasis texture a world draws `==highlight==` spans and search
/// matches with (deliberately shared — see `worlds.rs::WAGTAIL`'s "one kind
/// of emphasis, one texture" doc). `Wash` is the default: a hue-derived
/// translucent quad (`highlight_wash`) at the ordinary alpha, and the search
/// match reads the plain `selection` token. `Stipple` names a fixed opaque
/// color (rendered via `SelectionPipeline::set_dither`, `shaders/
/// selection.wgsl`'s Bayer-ordered dither branch) plus its `density` — every
/// drawn pixel is either that color at FULL opacity or fully transparent,
/// never a fractional alpha, so it stays legal on a world with no
/// intermediate grey. See `highlight_wash`, `wagtail_dither_density`,
/// `search_match_rgba_bytes`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HighlightTexture {
    Wash,
    Stipple { color: Srgb, density: f32 },
}

/// The declarative capability bundle a world's render behavior is built from.
/// See the module-level doc above. `DEFAULT` is what fourteen of the fifteen
/// worlds carry, byte-identical to the pre-capabilities-as-data render paths;
/// only `worlds.rs::WAGTAIL` deviates, on every field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderCaps {
    pub selection_style: SelectionStyle,
    pub caret_block_style: CaretBlockStyle,
    pub backdrop: Backdrop,
    pub elevation: Elevation,
    pub decorative_wash: DecorativeWash,
    pub image_reveal: ImageReveal,
    pub highlight_texture: HighlightTexture,
}

impl RenderCaps {
    pub const DEFAULT: RenderCaps = RenderCaps {
        selection_style: SelectionStyle::Fill,
        caret_block_style: CaretBlockStyle::Normal,
        backdrop: Backdrop::Blur,
        elevation: Elevation::Flat,
        decorative_wash: DecorativeWash::Enabled,
        image_reveal: ImageReveal::Translucent,
        highlight_texture: HighlightTexture::Wash,
    };
}

/// The MARGIN ground a world paints behind its centered page (PAGE MODE).
///
/// A TAGGED union — the user's locked model: the theme DECLARES which ground it
/// wants and SUPPLIES exactly the colors/params that ground needs; no field is
/// carried by a variant that does not use it. Every variant is a pure
/// pixel-coordinate shader (no assets, no clock), so the headless capture stays
/// byte-deterministic, and every variant leaves the PAGE column flat — the marks
/// live ONLY in the margins, so the page always reads as the clear figure.
///
/// The shader-side discriminants live in [`Background::shader_id`] and MUST match
/// the `g.shader` branches in `shaders/background.wgsl`.
// NOTE: no `Eq` — the gradient `dir` / stripe `angle` are floats (not `Eq`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Background {
    /// Plain directional gradient, no marks (the calmest ground).
    Gradient { from: Srgb, to: Srgb, dir: (f32, f32) },
    /// A grid of round dots over the gradient. `edge=false` is today's UNIFORM
    /// field; `edge=true` PROXIMITY-SCALES the dots — biggest/brightest hugging
    /// the page-column boundary, shrinking + fading with distance outward.
    Dots { from: Srgb, to: Srgb, dir: (f32, f32), tint: Srgb, edge: bool },
    /// Scattered dots + the occasional 4-point sparkle — a quiet cosmos.
    Starfield { from: Srgb, to: Srgb, dir: (f32, f32), tint: Srgb },
    /// Fine parallel lines (ledger / print rules).
    Pinstripe { from: Srgb, to: Srgb, dir: (f32, f32), tint: Srgb },
    /// The N++ look: a DIAGONAL directional gradient (`from`->`to` along `angle`)
    /// with a BRIGHT BAND of diagonal stripes hugging the page-column boundary
    /// that DISSOLVES outward into the gradient. The band uses the theme-supplied
    /// MUTED `band` tint (NOT the accent — amber stays the caret's, DESIGN §3).
    Stripes { from: Srgb, to: Srgb, band: Srgb, angle: f32 },
}

impl Background {
    /// The shader's discriminant (must match `g.shader` in
    /// `shaders/background.wgsl`). `Dots` is one branch for both `edge` modes;
    /// the proximity flag rides [`Background::edge`] instead.
    pub fn shader_id(&self) -> u32 {
        match self {
            Background::Gradient { .. } => 0,
            Background::Dots { .. } => 1,
            Background::Starfield { .. } => 2,
            Background::Pinstripe { .. } => 3,
            Background::Stripes { .. } => 4,
        }
    }
    /// Lowercase variant name for the capture sidecar.
    pub fn as_str(&self) -> &'static str {
        match self {
            Background::Gradient { .. } => "gradient",
            Background::Dots { .. } => "dots",
            Background::Starfield { .. } => "starfield",
            Background::Pinstripe { .. } => "pinstripe",
            Background::Stripes { .. } => "stripes",
        }
    }
    /// Gradient START endpoint.
    pub fn from(&self) -> Srgb {
        match self {
            Background::Gradient { from, .. }
            | Background::Dots { from, .. }
            | Background::Starfield { from, .. }
            | Background::Pinstripe { from, .. }
            | Background::Stripes { from, .. } => *from,
        }
    }
    /// Gradient END endpoint.
    pub fn to(&self) -> Srgb {
        match self {
            Background::Gradient { to, .. }
            | Background::Dots { to, .. }
            | Background::Starfield { to, .. }
            | Background::Pinstripe { to, .. }
            | Background::Stripes { to, .. } => *to,
        }
    }
    /// Gradient DIRECTION (a roughly unit UV vector). For [`Background::Stripes`]
    /// it is DERIVED from `angle` so the gradient runs ALONG the stripe angle.
    pub fn dir(&self) -> (f32, f32) {
        match self {
            Background::Gradient { dir, .. }
            | Background::Dots { dir, .. }
            | Background::Starfield { dir, .. }
            | Background::Pinstripe { dir, .. } => *dir,
            Background::Stripes { angle, .. } => (angle.cos(), angle.sin()),
        }
    }
    /// The marks/band tint: the dot / star / pinstripe tint, or the stripe band.
    /// A plain [`Background::Gradient`] has NO marks; it returns its `from`
    /// endpoint as an inert placeholder (shader id 0 draws no marks).
    pub fn tint(&self) -> Srgb {
        match self {
            Background::Dots { tint, .. }
            | Background::Starfield { tint, .. }
            | Background::Pinstripe { tint, .. } => *tint,
            Background::Stripes { band, .. } => *band,
            Background::Gradient { from, .. } => *from,
        }
    }
    /// PROXIMITY-SCALING flag — only [`Background::Dots`] honors it (`true` =>
    /// dots scale/fade with distance to the page boundary).
    pub fn edge(&self) -> bool {
        matches!(self, Background::Dots { edge: true, .. })
    }
    /// Stripe angle in radians (0 for the non-stripe grounds).
    pub fn angle(&self) -> f32 {
        match self {
            Background::Stripes { angle, .. } => *angle,
            _ => 0.0,
        }
    }
}

/// One palette "world": eight color tokens plus the chosen display font.
///
/// Field names mirror the DaisyUI tokens. `selection` is the only token with a
/// non-opaque alpha (the demoted secondary hue at 0x52 so it stays a calm tonal
/// wash, never a second accent). `font` is the per-world display font family; it
/// is the exact registered family name of an embedded face and drives the live
/// glyphon `Family::Name` selection (see render.rs).
// NOTE: no `Eq` — the `background` carries floats (the gradient `dir` / stripe
// `angle`), and f32 is not `Eq`. `PartialEq` is enough (Theme is never used as a
// hash/btree key).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    /// Human name of the world (e.g. "Potoroo").
    pub name: &'static str,
    /// True for the dark worlds (dark bases, light inks), false for light.
    pub dark: bool,
    /// Canvas / clear plane (deepest on dark, lightest on light).
    pub base_100: Srgb,
    /// Raised surface, one value step toward the ink from base-100.
    pub base_200: Srgb,
    /// Focused plane / border, the plane that reads "forward" by value.
    pub base_300: Srgb,
    /// Default ink drawn ON the base planes. The TOP rung of the ink ladder
    /// (full ink — content); see [`Theme::muted`] / [`Theme::faint`] for the
    /// de-emphasized rungs below it (DESIGN.md §4).
    pub base_content: Srgb,
    /// MUTED ink — the de-emphasized rung of the ink ladder: markdown markup,
    /// code comments, the focus-dim wash, secondary labels / the "/" sigil / the
    /// hit counter. (Formerly `base_content_dim`; same value, clearer name.)
    pub muted: Srgb,
    /// FAINT ink — the FAINTEST rung of the ink ladder, for UI metadata that must
    /// barely register: a future gutter's line numbers, the stats/word-count
    /// labels. Stepped further toward the background than [`Theme::muted`].
    /// Authored per world; refined by eye in the Themes phase. (Currently unused —
    /// reserved for the gutter/stats pass; see the crate-level `#![allow(dead_code)]`.)
    pub faint: Srgb,
    /// The one organic accent: the caret hue.
    pub primary: Srgb,
    /// Ink drawn ON the primary accent (near-black on warm accents, near-white
    /// on cool ones).
    pub primary_content: Srgb,
    /// Error / spell-squiggle signal color (only ever means failure).
    pub error: Srgb,
    /// Text-selection highlight: the demoted secondary hue at ~0x52 alpha.
    pub selection: Srgb,
    /// PAGE MODE margin GROUND: a tagged [`Background`] declaring which procedural
    /// ground this world wants and carrying exactly the colors/params that ground
    /// needs (gradient endpoints + direction, plus any mark tint / band / angle /
    /// proximity flag). The page column itself stays the flat base_100 figure; the
    /// marks live only in the margins.
    pub background: Background,
    /// Chosen display font family for this world (recorded; glyphon switching is
    /// a follow-up — see the module note).
    pub font: &'static str,
    /// The world's MONOSPACE companion face: the exact registered family name of a
    /// bundled monospaced face, used to shape CODE buffers (a file whose
    /// `Buffer::syntax_lang().is_some()`) while prose / markdown keep [`Theme::font`].
    /// A world whose DISPLAY face is ALREADY monospaced (Tawny = IBM Plex Mono,
    /// Currawong = Iosevka, Mangrove = JetBrains Mono, Potoroo = Monaspace Xenon)
    /// REUSES its own face here; every serif / sans world borrows one of the bundled
    /// monos — IBM Plex Mono (warm humanist), JetBrains Mono (crisp / technical), or
    /// Monaspace Xenon (a slab-serif mono) — matched to the world's CHARACTER (see
    /// each world's doc). Code needs the true fixed grid a proportional face can't
    /// give; the mono is selected in `render.rs::doc_attrs` when the buffer is code.
    pub mono: &'static str,
    /// PRIORITIZED CJK fallback family list for this world (bundled Noto JP
    /// first, then mac primary, then linux fallback). The bundled Latin/display
    /// faces carry NO Japanese glyphs, so Japanese text resolves through this
    /// list instead — a MINCHO (serif) face for the serif worlds, a GOTHIC
    /// (sans) face for the sans/mono worlds. Since the "Japanese bundle round"
    /// the FIRST candidate is a bundled embedded face (`render::FONT_CJK_FACES`
    /// — always present, no system dependency); Hiragino/Noto-CJK system faces
    /// stay as trailing candidates (see `CJK_MINCHO`/`CJK_GOTHIC`'s module doc
    /// for the taste-gate + follow-up). cosmic-text consults these in order and
    /// uses the first family actually registered (see `render.rs::resolve_cjk`).
    /// If NONE is present (a degenerate build with the bundled faces stripped
    /// AND no system CJK face), the renderer adds no CJK span and shaping falls
    /// through to cosmic-text's neutral platform fallback.
    pub cjk: &'static [&'static str],
    /// PRIORITIZED font-candidate list for SIMPLIFIED CHINESE text
    /// ([`FontId::ZhHans`]). The "Chinese round" gave this the same
    /// bundled-first mincho/gothic split as [`Theme::cjk`]: [`super::cjk::CJK_ZH_HANS_SERIF`]
    /// (bundled Noto Serif SC) for the serif worlds, [`super::cjk::CJK_ZH_HANS_SANS`]
    /// (bundled Noto Sans SC) for the sans/mono worlds, and a CHARACTERFUL
    /// override [`super::cjk::CJK_ZH_HANS_KLEE`] (bundled LXGW WenKai) for the two
    /// Klee-derived worlds (Mopoke, Quokka).
    pub zh_hans: &'static [&'static str],
    /// PRIORITIZED font-candidate list for TRADITIONAL CHINESE text
    /// ([`FontId::ZhHant`]). STILL a v1 taste call: one shared system-only
    /// ladder for every world — a Traditional-Chinese (Big5-class, ~13k char)
    /// bundled subset is banked, not attempted, this round.
    pub zh_hant: &'static [&'static str],
    /// PRIORITIZED font-candidate list for KOREAN text ([`FontId::Ko`]). The
    /// "Chinese round"'s KO rider: bundled Noto Sans KR first ([`super::cjk::CJK_KO`]),
    /// then system trailing candidates — ONE face for every world (no
    /// serif/sans split yet, a v1 taste call).
    pub ko: &'static [&'static str],
    /// The fine-press SECTION-BREAK ornament SET: markdown has THREE thematic-break
    /// syntaxes (`---` / `***` / `___`, all a `<hr>` in standard md), and awl makes
    /// each EXPRESSIVE — the author picks a break's feel by which one they type, and
    /// each renders a DIFFERENT centered ornament (a printer's fleuron, not a
    /// hairline). See [`Ornaments`] for the per-syntax glyphs + defaults; every world
    /// carries its OWN in-character trio of THREE DISTINCT symbols, all present in
    /// its [`Self::ornament_face`] (the design-table re-pick — dash is the flagship,
    /// also the About end-mark; star + underscore are its in-face siblings).
    /// All covered by this world's [`Self::ornament_face`], asserted by the
    /// NEVER-TOFU coverage test.
    pub ornaments: Ornaments,
    /// The FACE this world shapes its section-break fleuron + About end-mark in —
    /// one of [`super::ornament::ORNAMENT_GARAMOND`] / [`super::ornament::ORNAMENT_JUNICODE`] / [`super::ornament::ORNAMENT_MARKS`],
    /// chosen for the world's flavour (see those constants). ONLY the section-break
    /// / About ornament uses this face; keycaps + plain marks stay on the merged
    /// marks face (`render::SYMBOL_FAMILY`). Every glyph in [`Self::ornaments`] must
    /// exist in this face (NEVER-TOFU law).
    pub ornament_face: &'static str,
    /// How much bigger than body ink this world shapes its section-break ornament —
    /// and grows the break line's ROW — keyed to the ornament's CHARACTER (the
    /// detailed flowers reward size, the clean geometric marks don't): one of
    /// [`super::ornament::ORNAMENT_SCALE_ORNATE`] / [`super::ornament::ORNAMENT_SCALE_FLEURON`] /
    /// [`super::ornament::ORNAMENT_SCALE_GEOMETRIC`]. Read by BOTH `render::spans::md_line_scale` (the
    /// row height) and `render::layers::prepare_ornaments` (the glyph line-box), so
    /// the tall row always centers the glyph. A pure function of the active theme —
    /// a theme switch that changes this re-fits the break rows via `restyle_all_lines`
    /// (the same absolute-pixel path the heading sizes ride).
    pub ornament_scale: f32,
    /// The per-world UNORDERED-LIST BULLET pair — the depth-derived conceal glyph
    /// drawn over a `-`/`*`/`+` marker the caret is NOT on (`.0` = level 1, `.1` =
    /// level 2, cycling every two levels; see [`Self::bullet_for_depth`]). Like the
    /// section-break [`Self::ornaments`] trio one level down, it is PER-WORLD DATA
    /// shaped in this world's own [`Self::ornament_face`] — so the antique/literary
    /// serifs draw hederas / small fleurons / a manicule while the modern/technical
    /// worlds keep the plain [`super::ornament::BULLETS_PLAIN`] `•`/`◦` (restraint IS their character).
    /// The CALM RULE: a bullet is RHYTHM, not punctuation — quieter than a section
    /// ornament, faint ink unchanged, shaped small (see [`Self::bullet_scale`]).
    /// Both glyphs must exist in [`Self::ornament_face`] (NEVER-TOFU law).
    pub bullets: (char, char),
    /// How big the list bullet reads relative to body ink — a plain `•`/`◦` keeps
    /// body size ([`super::ornament::BULLET_SCALE_PLAIN`], byte-identical to before this round), while
    /// a characterful hedera / manicule shapes at ~half body ([`super::ornament::BULLET_SCALE_ORNAMENT`])
    /// so it reads as a quiet marker, never a loud section flourish. The glyph is
    /// centered in the row's full line-height (cosmic-text's own centering), so a
    /// scaled-down bullet still sits on the text's optical middle. A pure function of
    /// the active theme, read by `render::layers::prepare_ornaments`.
    pub bullet_scale: f32,
    /// The world's FACETING coordinates for the theme picker's lens-switcher — its
    /// value on each of the four lenses (Time / Register / Voice / Temperature),
    /// DERIVED from this world's palette + font (see [`ThemeTags`]). Every world has
    /// a value on every lens; the picker groups worlds by the active lens's section.
    pub tags: ThemeTags,
    /// Optional per-world SYNTAX ROLE-STYLE overrides (see [`RoleOverrides`]).
    /// [`RoleOverrides::NONE`] on fourteen of the fifteen worlds: the quiet role
    /// tints + washes are derived from this world's own palette in ONE place
    /// (`render/spans.rs::role_style_for`); a world only reaches for this to pin or
    /// disable a specific role style after a live-eyeball call, OR — Wagtail's
    /// case — because the shared hue-anchored derivation cannot serve a
    /// zero-saturation world at all (see `worlds.rs::WAGTAIL`).
    pub role_overrides: RoleOverrides,
    /// The declarative render-CAPABILITIES bundle (see [`RenderCaps`]'s module
    /// doc) — every per-theme render BEHAVIOR (selection style, caret-block
    /// invert, backdrop blur, elevation, decorative washes, the image-reveal
    /// scrim, the highlight/search-match texture) is a plain DATA read of
    /// this field. [`RenderCaps::DEFAULT`] on fourteen of the fifteen worlds;
    /// Wagtail is the escape hatch's real use (`worlds.rs::WAGTAIL`).
    pub render_caps: RenderCaps,
}

impl Theme {
    /// THE font-ID resolver's DATA seam: the prioritized family-name candidate
    /// ladder for `id` on this world. A NO-WILDCARD match — a future
    /// [`FontId`] variant fails to compile here until it's given a ladder (the
    /// same law-test-friendly shape as `syn_role_color`/`role_style_for`).
    ///
    /// `Latin` is a SINGLE-element ladder of the world's own [`Theme::font`] —
    /// unlike the four CJK IDs it has no fallback CANDIDATES because it never
    /// needs any: `Theme::font` names a bundled embedded face
    /// (`render::FONT_THEME_FACES`), always registered, so this ladder is the
    /// NEVER-TOFU LAW's guaranteed floor (see `theme::tests::
    /// every_font_id_has_a_nonempty_candidate_ladder_on_every_world` +
    /// `render::tests::cjk::latin_and_ja_always_resolve_to_an_embedded_face`).
    pub fn candidates(&self, id: FontId) -> Vec<&'static str> {
        match id {
            FontId::Latin => vec![self.font],
            FontId::Ja => self.cjk.to_vec(),
            FontId::ZhHans => self.zh_hans.to_vec(),
            FontId::ZhHant => self.zh_hant.to_vec(),
            FontId::Ko => self.ko.to_vec(),
        }
    }

    /// The unordered-list BULLET glyph for a list item at nesting `depth` (0 = top
    /// level): the per-world [`Self::bullets`] PAIR, cycling `.0`/`.1` every two
    /// levels (even depth → level-1 glyph, odd → level-2). Pure + total — the
    /// theme's own version of the retired `markdown::bullet_for_depth`, now that the
    /// glyph is per-world DATA rather than a fixed geometric triple.
    pub const fn bullet_for_depth(&self, depth: usize) -> char {
        if depth % 2 == 0 {
            self.bullets.0
        } else {
            self.bullets.1
        }
    }

    /// True iff this world's caret carries literally NO chroma (`primary`'s HSL
    /// saturation is exactly 0) — the MONOCHROME-WORLD signal every hue-anchored
    /// derivation must check before deriving a hue FROM a hue that doesn't exist:
    /// `render/spans.rs::highlight_wash`'s split-complementary rotation reads this
    /// to fall back to a plain value-step wash instead. Wagtail (zero saturation
    /// everywhere, the caret included — THEMES.md's logged DESIGN.md §3
    /// amendment) is the first world this is true for; every other world's
    /// `primary` carries real chroma. `Srgb::to_hsl` reports saturation `0.0`
    /// exactly for an achromatic (`r == g == b`) color (see its own doc), so this
    /// is an exact equality check, not a threshold.
    pub fn is_monochrome(&self) -> bool {
        self.primary.to_hsl().1 <= 0.0
    }

    /// True iff this world is not merely monochrome (zero saturation, which
    /// still permits any grey) but a TRUE 1-BIT world: its ground, ink, and
    /// caret tokens are each EXACTLY pure black (`#000000`) or pure white
    /// (`#FFFFFF`) — no grey rung at all. Wagtail's 2026-07 1-bit rework is the
    /// first (and, as of this writing, only) world this is true for.
    /// `is_monochrome` stays the broader "no hue" signal every hue-anchored
    /// derivation already checks (any grey qualifies); `is_one_bit` is the
    /// STRICTER sub-case a handful of render call sites read to decide "must
    /// this surface avoid EVERY non-edge alpha blend, not just every hue?" —
    /// the frosted-blur backdrop (a gaussian defocus of pure black/white would
    /// smear the edge into forbidden grey), the elevation border derivation
    /// (`theme::surface_selected`), the decorative shadow/underline washes, and
    /// the syntax-role/highlight-wash law tests' declared exemption arm. Checks
    /// only the three tokens a hue-anchored world could plausibly leave grey
    /// without also being monochrome-caught elsewhere; the full per-field 1-bit
    /// law lives in the render-side sweep (`render::tests::syntax_roles::
    /// every_one_bit_world_renders_only_pure_black_or_white`), which is the
    /// exhaustive check — this predicate is just the cheap gate render call
    /// sites branch on every frame.
    pub fn is_one_bit(&self) -> bool {
        let pure_bw = |c: Srgb| matches!((c.r, c.g, c.b), (0, 0, 0) | (255, 255, 255));
        self.is_monochrome()
            && pure_bw(self.base_100)
            && pure_bw(self.base_content)
            && pure_bw(self.primary)
    }
}

// --- The faceted THEME-PICKER lenses + per-world tags -----------------------
//
// The theme picker is a FACETED lens-switcher: LEFT/RIGHT cycle a [`Lens`], each
// grouping the worlds by ONE dimension into faint sections. Every world carries a
// value on EACH of the four real lenses ([`ThemeTags`]); `All` is the flat list.

/// A faceting LENS for the theme picker. The four real dimensions group the worlds
/// into sections; `All` is the flat, fuzzy-searchable list (today's behaviour).
/// Ordered for the LEFT/RIGHT strip with `All` PARKED at the FAR LEFT ([`Lens::STRIP`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lens {
    /// Group by background lightness/warmth: Dawn / Day / Dusk / Night.
    Time,
    /// Group by font formality: Humble / Everyday / Refined.
    Register,
    /// Group by face class: Literary (serif) / Technical (mono) / Modern (sans).
    Voice,
    /// Group by ground hue: Warm / Cool / Neutral.
    Temperature,
    /// The flat, fuzzy-filterable list of every world (no grouping).
    All,
}

impl Lens {
    /// The lens STRIP order, LEFT→RIGHT, with `All` parked at the FAR LEFT end.
    /// LEFT/RIGHT step through this (clamped at both ends); the picker opens on
    /// [`Lens::Time`], the first faceted view.
    pub const STRIP: [Lens; 5] = [Lens::All, Lens::Time, Lens::Register, Lens::Voice, Lens::Temperature];

    /// The strip LABEL for this lens.
    pub fn label(self) -> &'static str {
        match self {
            Lens::Time => "Time",
            Lens::Register => "Register",
            Lens::Voice => "Voice",
            Lens::Temperature => "Temperature",
            Lens::All => "All",
        }
    }

    /// The short lowercase name used in the capture sidecar.
    pub fn as_str(self) -> &'static str {
        match self {
            Lens::Time => "time",
            Lens::Register => "register",
            Lens::Voice => "voice",
            Lens::Temperature => "temperature",
            Lens::All => "all",
        }
    }

    /// The SECTIONS this lens groups worlds into, in display order (the faint
    /// uppercase section headers). `All` has none (the flat list).
    pub fn sections(self) -> &'static [&'static str] {
        match self {
            Lens::Time => &["Dawn", "Day", "Dusk", "Night"],
            Lens::Register => &["Humble", "Everyday", "Refined"],
            Lens::Voice => &["Literary", "Technical", "Modern"],
            Lens::Temperature => &["Warm", "Cool", "Neutral"],
            Lens::All => &[],
        }
    }
}

/// A world's value on EACH of the four real lenses — its faceting coordinates. The
/// faceting is now OPT-OUT per lens: a `None` axis means the world is NOT shown under
/// that lens (still reachable via [`Lens::All`] + fuzzy search), so each lens shows
/// only a CURATED handful (~2–3) per section rather than every world crowding in.
/// A `Some(section)` value is DERIVED from the world's own palette + font (see the
/// doc on each world): Time by background lightness/warmth, Register by font
/// formality, Voice by face class, Temperature by ground hue. These are DEFAULTS the
/// user can adjust; the curation lives in the world literals below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeTags {
    /// Section under [`Lens::Time`] (Dawn / Day / Dusk / Night), or `None` = hidden.
    pub time: Option<&'static str>,
    /// Section under [`Lens::Register`] (Humble / Everyday / Refined), or `None`.
    pub register: Option<&'static str>,
    /// Section under [`Lens::Voice`] (Literary / Technical / Modern), or `None`.
    pub voice: Option<&'static str>,
    /// Section under [`Lens::Temperature`] (Warm / Cool / Neutral), or `None`.
    pub temperature: Option<&'static str>,
}

impl ThemeTags {
    /// This world's section under `lens` — `None` when the world OPTS OUT of this lens
    /// (or for [`Lens::All`], which does not group). A `Some(section)` world appears
    /// under that section's faint header; a `None` world is omitted from the lens.
    pub fn section(&self, lens: Lens) -> Option<&'static str> {
        match lens {
            Lens::Time => self.time,
            Lens::Register => self.register,
            Lens::Voice => self.voice,
            Lens::Temperature => self.temperature,
            Lens::All => None,
        }
    }
}
