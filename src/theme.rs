#![allow(dead_code)] // Some tokens (BASE_200, PRIMARY_CONTENT) and converters are
                     // not consumed by every surface yet — reserved for the
                     // upcoming minibuffer/panel surfaces. The per-theme `font`
                     // field is now LIVE: it drives the glyphon `Family::Name`
                     // used to shape/render the document (see render.rs).

//! src/theme.rs — the palette model.
//!
//! Naming follows DaisyUI: base-100/200/300 are the base planes (100 = the
//! canvas; on a dark world that is the deepest plane, on a light world the
//! lightest), `*-content` is the ink that sits on a given surface, `primary` is
//! the one organic accent (the caret), `error` is the signal color, and
//! `selection` is a custom token (DaisyUI has no selection role).
//!
//! There are fourteen [`Theme`]s ("worlds"), eight dark and six light. One is the
//! ACTIVE theme at any moment (an index into [`THEMES`]); the windowed app can
//! cycle it live (`C-x t` / `C-x T`) and the headless `--theme NAME` flag pins
//! it before a capture. Every color call site reads the active theme rather than
//! a fixed const, so a theme switch reskins the whole UI. Each world also names a
//! display `font`; that family is loaded at startup and selected per-frame, so a
//! theme switch reskins the GLYPH SHAPES too (mono / serif / slab / sans).

use std::sync::atomic::{AtomicUsize, Ordering};

/// An sRGB color stored as raw 8-bit channels. This is the authoritative
/// representation; converter methods project it into whatever the GPU /
/// glyphon call site wants. Storing bytes (not floats) keeps every existing
/// output byte-identical.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Srgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Srgb {
    /// Opaque sRGB color (alpha = 0xFF).
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 0xFF }
    }
    /// sRGB color with explicit alpha.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// wgpu clear color. Straight channel/255.0 passthrough (NO gamma): this
    /// reproduces the old BG floats exactly. Needs f64 (wgpu::Color is f64).
    pub fn to_wgpu(self) -> wgpu::Color {
        wgpu::Color {
            r: self.r as f64 / 255.0,
            g: self.g as f64 / 255.0,
            b: self.b as f64 / 255.0,
            a: self.a as f64 / 255.0,
        }
    }
    /// glyphon text color (drops alpha; glyphon::Color::rgb is opaque, matching
    /// the old FG which was Color::rgb).
    pub fn to_glyphon(self) -> glyphon::Color {
        glyphon::Color::rgb(self.r, self.g, self.b)
    }
    /// Raw sRGB bytes for the caret pipeline (which converts to linear itself).
    pub fn rgb_bytes(self) -> [u8; 3] {
        [self.r, self.g, self.b]
    }
    /// Raw sRGBA bytes for the selection / spell pipelines (which convert to
    /// linear themselves).
    pub fn rgba_bytes(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }
    /// Lowercase 6-digit `#rrggbb` hex (alpha dropped). Used by the sidecar.
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// This color's `(hue°, saturation, lightness)` in HSL space — hue in
    /// `[0, 360)`, saturation + lightness in `[0, 1]`. Alpha is ignored. f32 math
    /// INTERNALLY only: the u8 channels stay the authoritative store (this pair of
    /// converters exists for the syntax ROLE-STYLE derivation in `render/spans.rs`,
    /// which mixes each world's own ink lightness with a fixed role hue anchor).
    pub fn to_hsl(self) -> (f32, f32, f32) {
        let r = self.r as f32 / 255.0;
        let g = self.g as f32 / 255.0;
        let b = self.b as f32 / 255.0;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) * 0.5;
        let d = max - min;
        if d <= f32::EPSILON {
            return (0.0, 0.0, l); // achromatic: hue is undefined, report 0
        }
        let s = d / (1.0 - (2.0 * l - 1.0).abs());
        let h = if max == r {
            60.0 * ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        (h, s, l)
    }

    /// An OPAQUE [`Srgb`] from `(hue°, saturation, lightness)` — the inverse of
    /// [`Srgb::to_hsl`] (up to u8 rounding, which stays authoritative). Hue wraps;
    /// saturation / lightness clamp to `[0, 1]`.
    pub fn from_hsl(h: f32, s: f32, l: f32) -> Self {
        let h = h.rem_euclid(360.0);
        let s = s.clamp(0.0, 1.0);
        let l = l.clamp(0.0, 1.0);
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let hp = h / 60.0;
        let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
        let (r1, g1, b1) = match hp as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = l - c * 0.5;
        let to = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        Srgb::rgb(to(r1), to(g1), to(b1))
    }
}

/// PER-WORLD SYNTAX ROLE-STYLE OVERRIDES — the designed escape hatch for the
/// DERIVED role tints + washes (`render/spans.rs::role_style_for`, the one owner
/// of role color). ALL worlds ship [`RoleOverrides::NONE`]: every role style is a
/// pure function of the world's own palette (ink-ladder lightness × fixed hue
/// anchors). A world may PIN a role's foreground tint, PIN a wash quad color
/// (rgba — washes are computed quad colors, deliberately NOT opaque theme
/// tokens), or DISABLE a wash outright, without touching the shared derivation.
/// The law test in `render/spans.rs` sweeps the EFFECTIVE style, so an override
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
    /// No overrides: every role style comes from the shared derivation. What all
    /// fourteen worlds ship with.
    pub const NONE: RoleOverrides = RoleOverrides {
        def_fg: None,
        const_fg: None,
        str_fg: None,
        comment_wash: WashOverride::Default,
        str_wash: WashOverride::Default,
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
    /// bundled-first mincho/gothic split as [`Theme::cjk`]: [`CJK_ZH_HANS_SERIF`]
    /// (bundled Noto Serif SC) for the serif worlds, [`CJK_ZH_HANS_SANS`]
    /// (bundled Noto Sans SC) for the sans/mono worlds, and a CHARACTERFUL
    /// override [`CJK_ZH_HANS_KLEE`] (bundled LXGW WenKai) for the two
    /// Klee-derived worlds (Mopoke, Quokka).
    pub zh_hans: &'static [&'static str],
    /// PRIORITIZED font-candidate list for TRADITIONAL CHINESE text
    /// ([`FontId::ZhHant`]). STILL a v1 taste call: one shared system-only
    /// ladder for every world — a Traditional-Chinese (Big5-class, ~13k char)
    /// bundled subset is banked, not attempted, this round.
    pub zh_hant: &'static [&'static str],
    /// PRIORITIZED font-candidate list for KOREAN text ([`FontId::Ko`]). The
    /// "Chinese round"'s KO rider: bundled Noto Sans KR first ([`CJK_KO`]),
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
    /// one of [`ORNAMENT_GARAMOND`] / [`ORNAMENT_JUNICODE`] / [`ORNAMENT_MARKS`],
    /// chosen for the world's flavour (see those constants). ONLY the section-break
    /// / About ornament uses this face; keycaps + plain marks stay on the merged
    /// marks face (`render::SYMBOL_FAMILY`). Every glyph in [`Self::ornaments`] must
    /// exist in this face (NEVER-TOFU law).
    pub ornament_face: &'static str,
    /// How much bigger than body ink this world shapes its section-break ornament —
    /// and grows the break line's ROW — keyed to the ornament's CHARACTER (the
    /// detailed flowers reward size, the clean geometric marks don't): one of
    /// [`ORNAMENT_SCALE_ORNATE`] / [`ORNAMENT_SCALE_FLEURON`] /
    /// [`ORNAMENT_SCALE_GEOMETRIC`]. Read by BOTH `render::spans::md_line_scale` (the
    /// row height) and `render::layers::prepare_ornaments` (the glyph line-box), so
    /// the tall row always centers the glyph. A pure function of the active theme —
    /// a theme switch that changes this re-fits the break rows via `restyle_all_lines`
    /// (the same absolute-pixel path the heading sizes ride).
    pub ornament_scale: f32,
    /// The world's FACETING coordinates for the theme picker's lens-switcher — its
    /// value on each of the four lenses (Time / Register / Voice / Temperature),
    /// DERIVED from this world's palette + font (see [`ThemeTags`]). Every world has
    /// a value on every lens; the picker groups worlds by the active lens's section.
    pub tags: ThemeTags,
    /// Optional per-world SYNTAX ROLE-STYLE overrides (see [`RoleOverrides`]).
    /// [`RoleOverrides::NONE`] everywhere at launch: the quiet role tints + washes
    /// are derived from this world's own palette in ONE place
    /// (`render/spans.rs::role_style_for`); a world only reaches for this to pin or
    /// disable a specific role style after a live-eyeball call.
    pub role_overrides: RoleOverrides,
}

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
//     TRUE literary serifs (Bilby, Undertow). NOTE: EB Garamond ships NO ⁂ asterism
//     (nor ❡/❥) and only those THREE fleurons, so a Garamond world's trio is exactly
//     {❧, ☙, ❦} permuted — never ⁂ — see the NEVER-TOFU coverage test.
//   * [`ORNAMENT_JUNICODE`] — Junicode's antique Caslon flowers (❧ ❦ ☙ + the ⁂/⁑
//     asterisms + a deep pool of PUA botanical/damask/tile ornaments), for the
//     antique/expressive/slab worlds AND the warm/pale literary serifs whose display
//     face carries no fleurons of its own (Gumtree, Saltpan, Magpie, Mopoke, Outback):
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
/// serif worlds. Registered from `EBGaramond-Regular.ttf` (also Undertow's own
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
// the leading-`#` heading scan). That scale is now PER-WORLD ([`Theme::ornament_scale`]),
// keyed to the ornament's CHARACTER — the detailed flowers reward size, the clean
// geometric marks don't — in three tiers that line up with the three ornament faces:
//
//   * ORNATE   — the [`ORNAMENT_JUNICODE`] Caslon flowers (antique/expressive worlds).
//   * FLEURON  — the [`ORNAMENT_GARAMOND`] Renaissance fleurons (literary serifs).
//   * GEOMETRIC — the [`ORNAMENT_MARKS`] stars/florets/diamonds (modern/technical).
//
// The field is read by BOTH `render::spans::md_line_scale` (the break ROW height)
// and `render::layers::prepare_ornaments` (the glyph LINE-BOX), so the two never
// drift — the tall row always centers the glyph shaped at the same scale. These are
// TASTE DEFAULTS: one dial per tier, tuned from the gallery.

/// ORNATE ornament scale — the Junicode Caslon-flower worlds. The most detailed
/// ornaments carry the most size.
pub const ORNAMENT_SCALE_ORNATE: f32 = 2.2;

/// FLEURON ornament scale — the EB Garamond literary-serif worlds. A generous but
/// slightly quieter flourish than the ornate flowers.
pub const ORNAMENT_SCALE_FLEURON: f32 = 1.8;

/// GEOMETRIC ornament scale — the Awl Marks stars/florets/diamonds. The clean
/// geometric marks read best kept modest, so they sit lowest on the tier ladder.
pub const ORNAMENT_SCALE_GEOMETRIC: f32 = 1.5;

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

/// The bundled CJK family names — the "embedded" side of the [`FontId`]
/// resolver's asset-source classification (also the `apply_cjk_force` A/B
/// switch's "bundled" set). Data, not a code path: [`Theme::candidates`]
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
];

// --- i18n ROUND: per-script font IDs + candidate ladders --------------------
//
// [`FontId`] names the per-script font IDENTITY awl resolves independently:
// the world's own Latin display face, plus the four CJK-family scripts this
// round adds ladders for. [`Theme::candidates`] maps an ID to a PRIORITIZED
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
    /// Japanese: kana + (contextually) Han. See [`Theme::cjk`].
    Ja,
    /// Simplified Chinese: Han. See [`Theme::zh_hans`].
    ZhHans,
    /// Traditional Chinese: Han + Bopomofo. See [`Theme::zh_hant`].
    ZhHant,
    /// Korean: Hangul + (contextually) Han. See [`Theme::ko`].
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

/// Korean ladder — the Chinese round's "KO rider": bundled Noto Sans KR
/// first (Google Fonts, OFL, subset to KS X 1001 modern hangul + jamo — see
/// `render::FONT_ZH_KO_FACES`), then Apple SD Gothic Neo (macOS) / Noto Sans
/// CJK KR (Linux) trailing. ONE face for every world (no serif/sans split —
/// a v1 taste call, logged: there is no comparable bundled serif Korean
/// companion yet, unlike ja/zh-Hans' real mincho/gothic pairs).
pub const CJK_KO: &[&str] = &["Noto Sans KR", "Apple SD Gothic Neo", "Noto Sans CJK KR"];

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
    /// `render::tests::latin_and_ja_always_resolve_to_an_embedded_face`).
    pub fn candidates(&self, id: FontId) -> Vec<&'static str> {
        match id {
            FontId::Latin => vec![self.font],
            FontId::Ja => self.cjk.to_vec(),
            FontId::ZhHans => self.zh_hans.to_vec(),
            FontId::ZhHant => self.zh_hant.to_vec(),
            FontId::Ko => self.ko.to_vec(),
        }
    }
}

// --- The fourteen worlds (exact hex from the theme spec) ---------------------

/// Gumtree — light eucalyptus reading room (coral caret on a cool green page).
pub const GUMTREE: Theme = Theme {
    name: "Gumtree",
    dark: false,
    base_100: Srgb::rgb(0xE4, 0xF8, 0xE2),
    base_200: Srgb::rgb(0xCF, 0xF3, 0xCC),
    base_300: Srgb::rgb(0xB7, 0xEF, 0xB4),
    base_content: Srgb::rgb(0x16, 0x24, 0x1A),
    muted: Srgb::rgb(0x5A, 0x6B, 0x57),
    faint: Srgb::rgb(0x91, 0xA3, 0x8F),
    primary: Srgb::rgb(0xDA, 0x52, 0x5D),
    primary_content: Srgb::rgb(0xFB, 0xEC, 0xEC),
    error: Srgb::rgb(0xC0, 0x39, 0x2B),
    selection: Srgb::rgba(0x88, 0x8F, 0x5D, 0x52),
    background: Background::Dots {
        from: Srgb::rgb(0xCF, 0xF3, 0xCC),
        to: Srgb::rgb(0xB7, 0xEF, 0xB4),
        dir: (0.7, 0.7),
        tint: Srgb::rgb(0x93, 0xA8, 0x7A),
        edge: false,
    },
    font: "Literata",
    // Literary serif world → the slab-serif Monaspace Xenon: a mono that keeps a
    // whisper of the serif so the code page still reads as this world's kin.
    mono: "Monaspace Xenon",
    cjk: CJK_MINCHO,
    zh_hans: CJK_ZH_HANS_SERIF,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Warm literary serif → Junicode's Caslon botanical sprays (an upward sprig + two sibling sprays).
    ornaments: Ornaments { dash: '\u{E67D}', star: '\u{E270}', underscore: '\u{E68A}' },
    ornament_face: ORNAMENT_JUNICODE,
    ornament_scale: ORNAMENT_SCALE_ORNATE,
    // Pale cool-green ground → Day; Literata reading serif → Refined / Literary; green hue → Cool.
    // Curated: shows under Day / Literary / Cool; opts OUT of Register (crowded → Bilby/Saltpan/Undertow keep Refined).
    tags: ThemeTags { time: Some("Day"), register: None, voice: Some("Literary"), temperature: Some("Cool") },
    role_overrides: RoleOverrides::NONE,
};

/// Potoroo — dark den-warm nocturne (raw-sienna caret in a burnt-orange room).
pub const POTOROO: Theme = Theme {
    name: "Potoroo",
    dark: true,
    base_100: Srgb::rgb(0x1F, 0x04, 0x00),
    base_200: Srgb::rgb(0x31, 0x05, 0x00),
    base_300: Srgb::rgb(0x56, 0x28, 0x00),
    base_content: Srgb::rgb(0xF0, 0xE6, 0xDE),
    muted: Srgb::rgb(0x9C, 0x85, 0x76),
    faint: Srgb::rgb(0x75, 0x5D, 0x51),
    primary: Srgb::rgb(0xFE, 0xAF, 0x69),
    primary_content: Srgb::rgb(0x2A, 0x14, 0x02),
    error: Srgb::rgb(0xFF, 0x6B, 0x5C),
    selection: Srgb::rgba(0x7E, 0xB4, 0x7C, 0x52),
    // The bold rust den is the showpiece: the NEW Stripes ground — a diagonal
    // gradient (base_100 -> base_300) with a bright diagonal band hugging the page
    // edge. `band` is a MUTED tint of the rust palette (Potoroo's old pinstripe
    // tint #6B3A12, NOT the amber accent #FEAF69), at a tasteful ~34° angle.
    background: Background::Stripes {
        from: Srgb::rgb(0x1F, 0x04, 0x00),
        to: Srgb::rgb(0x56, 0x28, 0x00),
        band: Srgb::rgb(0x6B, 0x3A, 0x12),
        angle: 0.6,
    },
    // Monaspace Xenon — a slab-serif monospace, distinct from Tawny/Mopoke's
    // sans-mono so the two den-warm darks no longer share IBM Plex Mono.
    font: "Monaspace Xenon",
    // Display face is ALREADY a monospace → reuse it for code (no second grid).
    mono: "Monaspace Xenon",
    cjk: CJK_GOTHIC,
    zh_hans: CJK_ZH_HANS_SANS,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Technical mono world → the merged marks' star/diamond trio (✶ 6-star + ✦ + ◆).
    ornaments: Ornaments { dash: '✶', star: '✦', underscore: '◆' },
    ornament_face: ORNAMENT_MARKS,
    ornament_scale: ORNAMENT_SCALE_GEOMETRIC,
    // Dark burnt-orange room → Dusk (warm dark); Monaspace mono → Humble / Technical; rust hue → Warm.
    // Curated: a headliner on ALL four — Dusk / Humble / Technical / Warm are each its clearest exemplar.
    tags: ThemeTags { time: Some("Dusk"), register: Some("Humble"), voice: Some("Technical"), temperature: Some("Warm") },
    role_overrides: RoleOverrides::NONE,
};

/// Bilby — light desert dawn (deep pyrite-gold caret on a pale-blue page).
pub const BILBY: Theme = Theme {
    name: "Bilby",
    dark: false,
    base_100: Srgb::rgb(0xE8, 0xFA, 0xFF),
    base_200: Srgb::rgb(0xCF, 0xF3, 0xFF),
    base_300: Srgb::rgb(0xB3, 0xE7, 0xFB),
    base_content: Srgb::rgb(0x10, 0x24, 0x2C),
    muted: Srgb::rgb(0x55, 0x70, 0x79),
    faint: Srgb::rgb(0x8A, 0xA2, 0xA9),
    primary: Srgb::rgb(0xAA, 0x94, 0x34),
    primary_content: Srgb::rgb(0xFB, 0xF6, 0xE4),
    error: Srgb::rgb(0xC0, 0x39, 0x2B),
    selection: Srgb::rgba(0x5B, 0xA3, 0xC5, 0x52),
    background: Background::Gradient {
        from: Srgb::rgb(0xCF, 0xF3, 0xFF),
        to: Srgb::rgb(0xB3, 0xE7, 0xFB),
        dir: (0.7, 0.7),
    },
    // Newsreader registers under this exact fontdb family name (it ships as the
    // "16pt" optical-size master), so `Family::Name` must match it verbatim.
    font: "Newsreader 16pt 16pt",
    // Refined display serif → the slab-serif Monaspace Xenon for a literary code page.
    mono: "Monaspace Xenon",
    cjk: CJK_MINCHO,
    zh_hans: CJK_ZH_HANS_SERIF,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Literary serif world → EB Garamond fleurons; `***` uses ☙ (EBG has no ⁂).
    ornaments: Ornaments { dash: '❧', star: '☙', underscore: '❦' },
    ornament_face: ORNAMENT_GARAMOND,
    ornament_scale: ORNAMENT_SCALE_FLEURON,
    // Pale blue ground → Day; Newsreader display serif → Refined / Literary; blue hue → Cool.
    // Curated: shows under Day / Refined; opts OUT of Voice (Literary crowded) + Temperature (Cool crowded).
    tags: ThemeTags { time: Some("Day"), register: Some("Refined"), voice: None, temperature: None },
    role_overrides: RoleOverrides::NONE,
};

/// Saltpan — light sun-bleached salt flat (cinnamon-clay caret on warm ecru).
pub const SALTPAN: Theme = Theme {
    name: "Saltpan",
    dark: false,
    // GROUND NUDGE (distinctive-grounds pass): deepened the near-white page toward
    // a true warm ecru (#FFFDF2 → #FDF7E2) — it read almost identically to Magpie's
    // paper-white (redmean 13.9, the tightest light-world pair) and flat against its
    // own "warm ecru salt-flat" flavour. The darker cream separates from Magpie
    // (→30.1) without diving into Quokka/Galah's warm pales (min 27.2), and a lower
    // ground lightness only IMPROVES the role-tint ground-contrast floor §2 flagged
    // here. base_200/300 (already creamy) + the Pinstripe margin are unchanged.
    base_100: Srgb::rgb(0xFD, 0xF7, 0xE2),
    base_200: Srgb::rgb(0xFB, 0xF3, 0xDE),
    base_300: Srgb::rgb(0xF2, 0xE6, 0xC7),
    base_content: Srgb::rgb(0x24, 0x1D, 0x12),
    muted: Srgb::rgb(0x7A, 0x6E, 0x55),
    faint: Srgb::rgb(0xAB, 0xA3, 0x8F),
    primary: Srgb::rgb(0x8D, 0x59, 0x25),
    primary_content: Srgb::rgb(0xFB, 0xF1, 0xE6),
    error: Srgb::rgb(0xB5, 0x45, 0x2B),
    selection: Srgb::rgba(0xA5, 0x86, 0x50, 0x52),
    background: Background::Pinstripe {
        from: Srgb::rgb(0xFB, 0xF3, 0xDE),
        to: Srgb::rgb(0xF2, 0xE6, 0xC7),
        dir: (0.0, 1.0),
        tint: Srgb::rgb(0xD9, 0xC7, 0x9B),
    },
    // Fraunces 9pt — a warm old-style serif at the text optical size; distinct
    // from Gumtree's Literata so the light serifs read apart.
    font: "Fraunces 9pt",
    // Old-style literary serif → Monaspace Xenon: the slab-serif mono echoes
    // Fraunces' serifed warmth on the code grid.
    mono: "Monaspace Xenon",
    cjk: CJK_MINCHO,
    zh_hans: CJK_ZH_HANS_SERIF,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Pale serif world → Junicode's horizontal running-vine Caslon scrolls (a vine + two sibling scrolls).
    ornaments: Ornaments { dash: '\u{F01B}', star: '\u{F01D}', underscore: '\u{F01E}' },
    ornament_face: ORNAMENT_JUNICODE,
    ornament_scale: ORNAMENT_SCALE_ORNATE,
    // Warm ecru salt flat → Dawn (warm-soft light); Fraunces old-style serif → Refined / Literary; sand hue → Warm.
    // Curated: shows under Dawn / Refined; opts OUT of Voice (Literary crowded) + Temperature (Warm crowded).
    tags: ThemeTags { time: Some("Dawn"), register: Some("Refined"), voice: None, temperature: None },
    role_overrides: RoleOverrides::NONE,
};

/// Quokka — light cheerful reef (teal caret cooling a warm peach page).
pub const QUOKKA: Theme = Theme {
    name: "Quokka",
    dark: false,
    base_100: Srgb::rgb(0xFF, 0xEA, 0xDD),
    base_200: Srgb::rgb(0xFF, 0xDF, 0xCF),
    base_300: Srgb::rgb(0xFF, 0xD2, 0xBD),
    base_content: Srgb::rgb(0x2B, 0x18, 0x10),
    muted: Srgb::rgb(0x8A, 0x64, 0x53),
    faint: Srgb::rgb(0xB4, 0x94, 0x85),
    primary: Srgb::rgb(0x07, 0x70, 0x73),
    primary_content: Srgb::rgb(0xE6, 0xF6, 0xF6),
    error: Srgb::rgb(0xC0, 0x39, 0x2B),
    selection: Srgb::rgba(0xBB, 0x80, 0x20, 0x52),
    background: Background::Dots {
        from: Srgb::rgb(0xFF, 0xDF, 0xCF),
        to: Srgb::rgb(0xFF, 0xD2, 0xBD),
        dir: (0.7, 0.7),
        tint: Srgb::rgb(0xE0, 0xAE, 0x92),
        edge: false,
    },
    font: "Fira Sans",
    // Warm friendly humanist sans → the warm humanist IBM Plex Mono for code.
    mono: "IBM Plex Mono",
    cjk: CJK_GOTHIC,
    zh_hans: CJK_ZH_HANS_KLEE,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Friendly humanist sans → the merged marks' floral trio (✿ florette + ❀ + ✽).
    ornaments: Ornaments { dash: '✿', star: '❀', underscore: '✽' },
    ornament_face: ORNAMENT_MARKS,
    ornament_scale: ORNAMENT_SCALE_GEOMETRIC,
    // Warm peach reef → Dawn (warm-soft light); Fira Sans friendly humanist → Everyday / Modern; peach hue → Warm.
    // Curated: a headliner on ALL four — Dawn / Everyday / Modern / Warm each read clearly on the friendly peach sans.
    tags: ThemeTags { time: Some("Dawn"), register: Some("Everyday"), voice: Some("Modern"), temperature: Some("Warm") },
    role_overrides: RoleOverrides::NONE,
};

/// Undertow — dark deep midnight current (hot indian-lake caret in violet dark).
pub const UNDERTOW: Theme = Theme {
    name: "Undertow",
    dark: true,
    base_100: Srgb::rgb(0x15, 0x0A, 0x2C),
    base_200: Srgb::rgb(0x24, 0x15, 0x40),
    base_300: Srgb::rgb(0x3C, 0x36, 0x54),
    base_content: Srgb::rgb(0xEC, 0xE8, 0xF2),
    muted: Srgb::rgb(0x8A, 0x7F, 0xA8),
    faint: Srgb::rgb(0x53, 0x48, 0x6E),
    primary: Srgb::rgb(0xC5, 0x3C, 0x69),
    primary_content: Srgb::rgb(0x2A, 0x0A, 0x16),
    error: Srgb::rgb(0xFF, 0x6B, 0x5C),
    selection: Srgb::rgba(0x4F, 0x40, 0x86, 0x52),
    background: Background::Starfield {
        from: Srgb::rgb(0x15, 0x0A, 0x2C),
        to: Srgb::rgb(0x24, 0x15, 0x40),
        dir: (0.0, 1.0),
        tint: Srgb::rgb(0x7A, 0x6C, 0xA8),
    },
    // EB Garamond — a classic Garamond serif; distinct from Bilby's Newsreader
    // so the two share no face.
    font: "EB Garamond",
    // Classic Garamond serif nocturne → Monaspace Xenon: a refined slab-serif mono
    // for a literary code page.
    mono: "Monaspace Xenon",
    cjk: CJK_MINCHO,
    zh_hans: CJK_ZH_HANS_SERIF,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // OVERRIDE (the serif nocturne's flourish): mirror the default fleuron into its
    // reversed twin ☙ for `---`, and swap `___`'s heart to the black-heart bullet ❥
    // (both NS2 ornament variants, also bundled). `***` keeps the ⁂ asterism.
    // IN-FACE: Undertow's display IS EB Garamond, so its fleuron shapes in its own
    // face. The old {☙,⁂,❥} relied on the merged marks face (EBG has no ⁂/❥); the
    // set is now all-EBG fleurons (☙ dash keeps its distinct reversed look).
    ornaments: Ornaments { dash: '☙', star: '❧', underscore: '❦' },
    ornament_face: ORNAMENT_GARAMOND,
    ornament_scale: ORNAMENT_SCALE_FLEURON,
    // Dark violet current → Night; EB Garamond classic serif → Refined / Literary; violet-blue hue → Cool.
    // Curated: shows under Night / Refined / Literary (the classical serif's home); opts OUT of Temperature (Cool crowded).
    tags: ThemeTags { time: Some("Night"), register: Some("Refined"), voice: Some("Literary"), temperature: None },
    role_overrides: RoleOverrides::NONE,
};

/// Outback — dark red-centre night (hays-russet caret in blackish-olive room).
pub const OUTBACK: Theme = Theme {
    name: "Outback",
    dark: true,
    // GROUND NUDGE (distinctive-grounds pass): leaned the whole near-black ramp
    // toward a truer YELLOW-olive (hue ~107°→~94°, a touch more chroma). The old
    // ground read as a near-neutral dark that collided with warm-charcoal Mopoke
    // (redmean 12.3, the tightest dark pair) and with Tawny/Mangrove; the deeper
    // olive separates from Mopoke (→17.9, now a clear 60° hue gap), Mangrove
    // (→38.4) and Tawny (→27.7), and reads truer to "blackish-olive on the open
    // range." Lightness steps preserved; the Starfield `from`/`to` below track it.
    base_100: Srgb::rgb(0x16, 0x1F, 0x0F),
    base_200: Srgb::rgb(0x1E, 0x29, 0x16),
    base_300: Srgb::rgb(0x3E, 0x4A, 0x31),
    base_content: Srgb::rgb(0xEC, 0xEA, 0xE0),
    muted: Srgb::rgb(0x8A, 0x8C, 0x78),
    faint: Srgb::rgb(0x51, 0x56, 0x47),
    primary: Srgb::rgb(0xDE, 0x8E, 0x7F),
    primary_content: Srgb::rgb(0x2A, 0x14, 0x10),
    error: Srgb::rgb(0xFF, 0x6B, 0x5C),
    selection: Srgb::rgba(0xFF, 0xEF, 0xAE, 0x52),
    background: Background::Starfield {
        // from/to track the nudged base_100/base_200 above so the margin still
        // matches the page; the mark tint (a mid olive-grey) stays as-is.
        from: Srgb::rgb(0x16, 0x1F, 0x0F),
        to: Srgb::rgb(0x1E, 0x29, 0x16),
        dir: (0.0, 1.0),
        tint: Srgb::rgb(0x7C, 0x80, 0x68),
    },
    font: "Zilla Slab",
    // Slab-serif display → Monaspace Xenon: the only slab-serif mono, matching Zilla.
    mono: "Monaspace Xenon",
    cjk: CJK_MINCHO,
    zh_hans: CJK_ZH_HANS_SERIF,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Slab world → austere typographic Junicode marks (⁂ asterism + ⁑ + ❦ floral heart).
    ornaments: Ornaments { dash: '⁂', star: '⁑', underscore: '❦' },
    ornament_face: ORNAMENT_JUNICODE,
    ornament_scale: ORNAMENT_SCALE_ORNATE,
    // Blackish-olive night → Night; Zilla Slab workhorse slab → Everyday; slab-serif face → Literary; olive-green hue → Cool.
    // Curated: headlines Everyday alone (Night/Literary/Cool are each crowded); still reachable via All.
    tags: ThemeTags { time: None, register: Some("Everyday"), voice: None, temperature: None },
    role_overrides: RoleOverrides::NONE,
};

/// Tawny — the DEFAULT world: a quiet warm-grey nocturne with a tawny-gold caret.
/// It is awl's "home" look, so its display font is the original bundled IBM Plex
/// Mono — opening the app lands on a mono world that looks exactly like home, and
/// the proportional worlds (Literata / Newsreader / Plex Sans / Zilla Slab) are
/// one `C-x t` away.
pub const TAWNY: Theme = Theme {
    name: "Tawny",
    dark: true,
    base_100: Srgb::rgb(0x16, 0x18, 0x1D),
    base_200: Srgb::rgb(0x20, 0x22, 0x28),
    base_300: Srgb::rgb(0x2A, 0x2D, 0x34),
    base_content: Srgb::rgb(0xE6, 0xE6, 0xE6),
    muted: Srgb::rgb(0x8B, 0x91, 0x9D),
    faint: Srgb::rgb(0x4E, 0x52, 0x5A),
    primary: Srgb::rgb(0xFF, 0xC0, 0x5E),
    primary_content: Srgb::rgb(0x26, 0x1A, 0x08),
    error: Srgb::rgb(0xE5, 0x4B, 0x4B),
    selection: Srgb::rgba(0x3A, 0x6F, 0xD8, 0x52),
    background: Background::Dots {
        from: Srgb::rgb(0x16, 0x18, 0x1D),
        to: Srgb::rgb(0x20, 0x22, 0x28),
        dir: (0.0, 1.0),
        tint: Srgb::rgb(0x2C, 0x2F, 0x37),
        edge: false,
    },
    font: "IBM Plex Mono",
    // The home mono IS the display face → reuse it for code.
    mono: "IBM Plex Mono",
    cjk: CJK_GOTHIC,
    zh_hans: CJK_ZH_HANS_SANS,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // The default mono world → the merged marks' star/diamond trio (✦ 4-star + ✷ + ◈).
    ornaments: Ornaments { dash: '✦', star: '✷', underscore: '◈' },
    ornament_face: ORNAMENT_MARKS,
    ornament_scale: ORNAMENT_SCALE_GEOMETRIC,
    // Warm-grey neutral nocturne → Night; IBM Plex Mono → Humble / Technical; near-neutral grey → Neutral.
    // Curated: shows under Humble / Neutral (its plainest traits); opts OUT of Time (Night crowded) + Voice (Technical crowded).
    tags: ThemeTags { time: None, register: Some("Humble"), voice: None, temperature: Some("Neutral") },
    role_overrides: RoleOverrides::NONE,
};

/// Mopoke — Tawny warmed a notch: the cool near-black neutrals nudged to a warm
/// charcoal so the room reads cosy, not void. Subtlest of the warm-Tawny trio.
/// Same IBM Plex Mono home + amber-eye caret. (Provisional name; warm-Tawny "A".)
pub const MOPOKE: Theme = Theme {
    name: "Mopoke",
    dark: true,
    base_100: Srgb::rgb(0x1B, 0x18, 0x14),
    base_200: Srgb::rgb(0x25, 0x21, 0x1B),
    base_300: Srgb::rgb(0x31, 0x2B, 0x22),
    base_content: Srgb::rgb(0xE8, 0xE4, 0xDC),
    muted: Srgb::rgb(0x97, 0x8C, 0x7E),
    faint: Srgb::rgb(0x57, 0x50, 0x47),
    primary: Srgb::rgb(0xFF, 0xC0, 0x5E),
    primary_content: Srgb::rgb(0x26, 0x1A, 0x08),
    error: Srgb::rgb(0xE5, 0x4B, 0x4B),
    selection: Srgb::rgba(0x3A, 0x6F, 0xD8, 0x52),
    background: Background::Dots {
        from: Srgb::rgb(0x1B, 0x18, 0x14),
        to: Srgb::rgb(0x25, 0x21, 0x1B),
        dir: (0.0, 1.0),
        tint: Srgb::rgb(0x33, 0x2D, 0x24),
        edge: false,
    },
    // iA Writer Quattro S — a duospaced writing face; breaks up the mono darks
    // (Tawny keeps IBM Plex Mono as its signature; Potoroo takes Monaspace Xenon).
    font: "iA Writer Quattro S",
    // Warm cosy charcoal → the warm humanist IBM Plex Mono (kin to Tawny's home look).
    mono: "IBM Plex Mono",
    cjk: CJK_GOTHIC,
    zh_hans: CJK_ZH_HANS_KLEE,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Cosy expressive world → Junicode's ornate Caslon damask flourishes (a damask + candelabra + damask tile).
    ornaments: Ornaments { dash: '\u{E670}', star: '\u{F011}', underscore: '\u{F014}' },
    ornament_face: ORNAMENT_JUNICODE,
    ornament_scale: ORNAMENT_SCALE_ORNATE,
    // Warm charcoal cosy dark → Dusk (warm dark); iA Writer Quattro utilitarian → Humble; sans-class writing face → Modern; warm hue → Warm.
    // Curated: shows under Dusk / Humble (its cosy utilitarian core); opts OUT of Voice (Modern crowded) + Temperature (Warm crowded).
    tags: ThemeTags { time: Some("Dusk"), register: Some("Humble"), voice: None, temperature: None },
    role_overrides: RoleOverrides::NONE,
};

/// Kingfisher — a deep midnight-navy dark world: a cool, still room of blue-black
/// planes under a cool off-white ink, lit by ONE warm-amber caret — the thesis
/// made literal, the single warm thing in a cool room (DESIGN §3). Drawn in IBM
/// Plex Sans to set it apart from Tawny's mono family — a clean sans nocturne.
pub const KINGFISHER: Theme = Theme {
    name: "Kingfisher",
    dark: true,
    base_100: Srgb::rgb(0x0C, 0x14, 0x26),
    base_200: Srgb::rgb(0x13, 0x1D, 0x33),
    base_300: Srgb::rgb(0x1F, 0x2C, 0x49),
    base_content: Srgb::rgb(0xE7, 0xEA, 0xF2),
    muted: Srgb::rgb(0x80, 0x89, 0xA0),
    faint: Srgb::rgb(0x46, 0x4E, 0x63),
    primary: Srgb::rgb(0xF5, 0xA7, 0x42),
    primary_content: Srgb::rgb(0x2A, 0x1B, 0x06),
    error: Srgb::rgb(0xFF, 0x6B, 0x5C),
    selection: Srgb::rgba(0x3D, 0x6B, 0xC4, 0x52),
    background: Background::Dots {
        from: Srgb::rgb(0x0C, 0x14, 0x26),
        to: Srgb::rgb(0x13, 0x1D, 0x33),
        dir: (0.0, 1.0),
        tint: Srgb::rgb(0x1B, 0x27, 0x42),
        edge: false,
    },
    font: "IBM Plex Sans",
    // Cool technical navy → the crisp JetBrains Mono (a coding face for a coding den).
    mono: "JetBrains Mono",
    cjk: CJK_GOTHIC,
    zh_hans: CJK_ZH_HANS_SANS,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Clean sans nocturne → the merged marks' rosette/geometric trio (❂ rosette + ✴ + ◈).
    ornaments: Ornaments { dash: '❂', star: '✴', underscore: '◈' },
    ornament_face: ORNAMENT_MARKS,
    ornament_scale: ORNAMENT_SCALE_GEOMETRIC,
    // Midnight-navy nocturne → Night; IBM Plex Sans workhorse → Everyday / Modern; blue-black hue → Cool.
    // Curated: a headliner on ALL four — the crisp midnight dive reads clearly Night / Everyday / Modern / Cool.
    tags: ThemeTags { time: Some("Night"), register: Some("Everyday"), voice: Some("Modern"), temperature: Some("Cool") },
    role_overrides: RoleOverrides::NONE,
};

/// Currawong — a near-pure-black OLED world: the deepest base awl ships, planes
/// of true black for maximum contrast and a power-sipping dark, cool off-white
/// ink, and a single gold-YELLOW caret echoing the Pied Currawong's yellow eye.
/// A calm, minimal margin (a plain Gradient, no pattern noise). Drawn in the narrow,
/// mechanical Iosevka — a quiet coding den at midnight.
pub const CURRAWONG: Theme = Theme {
    name: "Currawong",
    dark: true,
    base_100: Srgb::rgb(0x06, 0x06, 0x07),
    base_200: Srgb::rgb(0x0E, 0x0F, 0x11),
    base_300: Srgb::rgb(0x1C, 0x1E, 0x22),
    base_content: Srgb::rgb(0xED, 0xEE, 0xF0),
    muted: Srgb::rgb(0x88, 0x8C, 0x94),
    faint: Srgb::rgb(0x44, 0x46, 0x4B),
    primary: Srgb::rgb(0xF4, 0xC5, 0x34),
    primary_content: Srgb::rgb(0x1E, 0x1A, 0x06),
    error: Srgb::rgb(0xFF, 0x6B, 0x5C),
    selection: Srgb::rgba(0x3E, 0x5C, 0x8A, 0x52),
    background: Background::Gradient {
        from: Srgb::rgb(0x06, 0x06, 0x07),
        to: Srgb::rgb(0x0E, 0x0F, 0x11),
        dir: (0.0, 1.0),
    },
    font: "Iosevka",
    // Display face is ALREADY the narrow, mechanical Iosevka mono → reuse it for code.
    mono: "Iosevka",
    cjk: CJK_GOTHIC,
    zh_hans: CJK_ZH_HANS_SANS,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Technical mono → the merged marks' star/diamond trio (✷ 8-star + ✴ + ⬥).
    ornaments: Ornaments { dash: '✷', star: '✴', underscore: '⬥' },
    ornament_face: ORNAMENT_MARKS,
    ornament_scale: ORNAMENT_SCALE_GEOMETRIC,
    // Near-pure-black OLED → Night; Iosevka → Humble / Technical; true-black neutral → Neutral.
    // Curated: shows under Night (the darkest, most iconic) / Technical / Neutral; opts OUT of Register (Humble crowded).
    tags: ThemeTags { time: Some("Night"), register: None, voice: Some("Technical"), temperature: Some("Neutral") },
    role_overrides: RoleOverrides::NONE,
};

/// Mangrove — dark tidal-teal coding den (one warm low-tide ember at the caret).
/// The room is cool teal/blue-green; the single warm living thing is an
/// amber-coral caret. Drawn in JetBrains Mono — the second bundled mono face, a
/// crisp coding home distinct from Tawny's warm grey.
pub const MANGROVE: Theme = Theme {
    name: "Mangrove",
    dark: true,
    // GROUND NUDGE (distinctive-grounds pass): pushed the ramp toward a truer,
    // more-saturated TIDAL TEAL (~169° hue, ground chroma 0.33→0.39, a touch
    // lighter). The old ground was so dark it read near-neutral and collided with
    // warm-grey Tawny (redmean 15.2) and blackish-olive Outback (16.6); the deeper
    // teal separates cleanly (Tawny →32, Outback →40, Kingfisher →36) and makes
    // "dark tidal-teal den — cool and rooted" read on the page. NOTE: a still-purer
    // teal (near-zero red) breached the comment-wash whisper ceiling — the warm
    // wash lifts a red-starved base too far (ΔL > 0.12) — so a little red is kept
    // deliberately (ΔL 0.114). Lightness steps preserved; Dots `from`/`to` track it.
    base_100: Srgb::rgb(0x11, 0x27, 0x23),
    base_200: Srgb::rgb(0x18, 0x34, 0x2E),
    base_300: Srgb::rgb(0x26, 0x43, 0x3B),
    base_content: Srgb::rgb(0xD9, 0xE6, 0xE1),
    muted: Srgb::rgb(0x6F, 0x8A, 0x83),
    faint: Srgb::rgb(0x41, 0x55, 0x51),
    primary: Srgb::rgb(0xF2, 0xA6, 0x5C),
    primary_content: Srgb::rgb(0x2A, 0x18, 0x04),
    error: Srgb::rgb(0xFF, 0x6B, 0x5C),
    selection: Srgb::rgba(0x2F, 0x80, 0x79, 0x52),
    // Same Dots colors as before, now PROXIMITY-SCALED (`edge: true`): the dots
    // are biggest/brightest hugging the page boundary and shrink + fade outward.
    background: Background::Dots {
        // from/to track the nudged base_100/base_200 above so the margin still
        // matches the page; the teal dot tint stays as-is (already coherent).
        from: Srgb::rgb(0x11, 0x27, 0x23),
        to: Srgb::rgb(0x18, 0x34, 0x2E),
        dir: (0.0, 1.0),
        tint: Srgb::rgb(0x23, 0x3B, 0x35),
        edge: true,
    },
    font: "JetBrains Mono",
    // Display face is ALREADY JetBrains Mono → reuse it for code.
    mono: "JetBrains Mono",
    cjk: CJK_GOTHIC,
    zh_hans: CJK_ZH_HANS_SANS,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // OLED geometric mono → the merged marks' diamond-cluster trio (❖ cluster + ◈ + ⬥).
    ornaments: Ornaments { dash: '❖', star: '◈', underscore: '⬥' },
    ornament_face: ORNAMENT_MARKS,
    ornament_scale: ORNAMENT_SCALE_GEOMETRIC,
    // Dark tidal-teal den → Night; JetBrains Mono → Humble / Technical; teal hue → Cool.
    // Curated: shows under Technical / Cool (its rooted teal-mono character); opts OUT of Time (Night crowded) + Register (Humble crowded).
    tags: ThemeTags { time: None, register: None, voice: Some("Technical"), temperature: Some("Cool") },
    role_overrides: RoleOverrides::NONE,
};

/// Galah — light dusty galah-pink reading room (rose-garnet ember at the caret).
/// Warm pink page over deep wine ink; the caret reads as the one alive thing by
/// VALUE (a rose-garnet lower in value than the pale ground). Drawn in Figtree,
/// a friendly humanist sans.
pub const GALAH: Theme = Theme {
    name: "Galah",
    dark: false,
    base_100: Srgb::rgb(0xFC, 0xEE, 0xF1),
    base_200: Srgb::rgb(0xF8, 0xE0, 0xE6),
    base_300: Srgb::rgb(0xF1, 0xCF, 0xD9),
    base_content: Srgb::rgb(0x2A, 0x17, 0x1D),
    muted: Srgb::rgb(0x7C, 0x60, 0x68),
    faint: Srgb::rgb(0xA9, 0x92, 0x98),
    primary: Srgb::rgb(0xB2, 0x3A, 0x60),
    primary_content: Srgb::rgb(0xFB, 0xEA, 0xEE),
    error: Srgb::rgb(0xC0, 0x39, 0x2B),
    selection: Srgb::rgba(0x9A, 0x6B, 0x86, 0x52),
    background: Background::Gradient {
        from: Srgb::rgb(0xF8, 0xE0, 0xE6),
        to: Srgb::rgb(0xF1, 0xCF, 0xD9),
        dir: (0.7, 0.7),
    },
    font: "Figtree",
    // Warm friendly humanist sans → the warm humanist IBM Plex Mono.
    mono: "IBM Plex Mono",
    cjk: CJK_GOTHIC,
    zh_hans: CJK_ZH_HANS_SANS,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Humanist sans reading room → the merged marks' floral/rosette trio (❁ daisy + ❂ + ✿).
    ornaments: Ornaments { dash: '❁', star: '❂', underscore: '✿' },
    ornament_face: ORNAMENT_MARKS,
    ornament_scale: ORNAMENT_SCALE_GEOMETRIC,
    // Dusty-pink reading room → Dawn (warm-soft light); Figtree humanist sans → Everyday / Modern; rose hue → Warm.
    // Curated: shows under Dawn / Modern / Warm (its soft rosy dawn feel); opts OUT of Register (Everyday crowded).
    tags: ThemeTags { time: Some("Dawn"), register: None, voice: Some("Modern"), temperature: Some("Warm") },
    role_overrides: RoleOverrides::NONE,
};

/// Magpie — light stark high-contrast page (terracotta spark at the caret).
/// Near-neutral paper-white with near-black slab ink: maximum value contrast,
/// magpie black-and-white. The one warm thing is a terracotta-vermilion caret.
/// Drawn in the sharp, high-contrast Bitter slab for a confident newsprint-headline stance.
pub const MAGPIE: Theme = Theme {
    name: "Magpie",
    dark: false,
    base_100: Srgb::rgb(0xFB, 0xFB, 0xFA),
    base_200: Srgb::rgb(0xF1, 0xF1, 0xEF),
    base_300: Srgb::rgb(0xE4, 0xE4, 0xE1),
    base_content: Srgb::rgb(0x11, 0x13, 0x17),
    muted: Srgb::rgb(0x6C, 0x70, 0x77),
    faint: Srgb::rgb(0x9F, 0xA2, 0xA6),
    primary: Srgb::rgb(0xDB, 0x5A, 0x2B),
    primary_content: Srgb::rgb(0xFB, 0xEF, 0xE9),
    error: Srgb::rgb(0xC0, 0x39, 0x2B),
    selection: Srgb::rgba(0x46, 0x61, 0x8F, 0x52),
    background: Background::Pinstripe {
        from: Srgb::rgb(0xF1, 0xF1, 0xEF),
        to: Srgb::rgb(0xE4, 0xE4, 0xE1),
        dir: (0.0, 1.0),
        tint: Srgb::rgb(0xC9, 0xC9, 0xC5),
    },
    font: "Bitter",
    // Sharp high-contrast slab display → Monaspace Xenon: the slab-serif mono matches Bitter's stance.
    mono: "Monaspace Xenon",
    cjk: CJK_MINCHO,
    zh_hans: CJK_ZH_HANS_SERIF,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Stark high-contrast slab → Junicode's geometric Caslon tile flowers (a quatrefoil + two lattice/damask tiles).
    ornaments: Ornaments { dash: '\u{EF90}', star: '\u{EF98}', underscore: '\u{EF9A}' },
    ornament_face: ORNAMENT_JUNICODE,
    ornament_scale: ORNAMENT_SCALE_ORNATE,
    // Paper-white high-contrast page → Day; Bitter high-contrast slab → Everyday; slab-serif face → Literary; near-neutral hue → Neutral.
    // Curated: shows under Day / Literary / Neutral (sharp black-on-white slab); opts OUT of Register (Everyday crowded).
    tags: ThemeTags { time: Some("Day"), register: None, voice: Some("Literary"), temperature: Some("Neutral") },
    role_overrides: RoleOverrides::NONE,
};

/// All fourteen worlds, in cycle order. `C-x t` advances through this list and
/// wraps; `C-x T` steps backward. The DEFAULT (index 0) is Tawny: a quiet
/// warm-grey dark world whose display font is the original bundled IBM Plex
/// Mono, so the app opens on awl's familiar mono "home" look. The two deep cool
/// darks — Currawong (OLED black) beside the neutral Tawny/Mopoke pair, and
/// Kingfisher (midnight navy) beside the violet Undertow — sit with their kin.
pub const THEMES: [Theme; 14] = [
    TAWNY, MOPOKE, CURRAWONG,
    POTOROO, GUMTREE, BILBY, SALTPAN, QUOKKA, UNDERTOW, KINGFISHER, OUTBACK, MANGROVE, GALAH, MAGPIE,
];

/// Index into [`THEMES`] of the default/startup world. Tawny (a dark, warm-grey
/// world drawn in IBM Plex Mono) is awl's "home" look, so the app opens on the
/// familiar mono world; the proportional worlds are one theme-cycle away.
pub const DEFAULT_THEME: usize = 0;

/// The active theme index. A process-global so every render call site reads the
/// same world without threading a `&Theme` through the whole pipeline. The
/// windowed app cycles it (`C-x t`); `--theme NAME` pins it for a capture.
static ACTIVE: AtomicUsize = AtomicUsize::new(DEFAULT_THEME);

/// The SINGLE test mutex serializing every test that mutates the process-global
/// [`ACTIVE`] theme — colocated with the global so theme's own tests AND the
/// render/capture tests that flip the theme can hold the same lock (a second,
/// private mutex would let cargo's parallel runner race one global). Mirrors
/// `page::test_lock()` / `debug::TEST_LOCK`.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The currently active [`Theme`].
pub fn active() -> Theme {
    THEMES[ACTIVE.load(Ordering::Relaxed) % THEMES.len()]
}

/// Index of the active theme within [`THEMES`].
pub fn active_index() -> usize {
    ACTIVE.load(Ordering::Relaxed) % THEMES.len()
}

/// Set the active theme by index (wrapping). Returns the now-active [`Theme`].
pub fn set_active(index: usize) -> Theme {
    let i = index % THEMES.len();
    ACTIVE.store(i, Ordering::Relaxed);
    THEMES[i]
}

/// Advance to the next world (`step > 0`) or a previous one (`step < 0`), with
/// wrap-around, and return the now-active [`Theme`]. `C-x t` passes +1, `C-x T`
/// passes -1.
pub fn cycle(step: isize) -> Theme {
    let n = THEMES.len() as isize;
    let cur = active_index() as isize;
    let next = (((cur + step) % n) + n) % n;
    set_active(next as usize)
}

/// Set the active theme by case-insensitive name (e.g. "potoroo"). Returns the
/// theme on success, `None` if no world matches. Used by `--theme NAME`.
pub fn set_active_by_name(name: &str) -> Option<Theme> {
    let idx = THEMES
        .iter()
        .position(|t| t.name.eq_ignore_ascii_case(name))?;
    Some(set_active(idx))
}

// --- Active-theme token accessors (read by the render call sites) ----------
//
// These replace the old fixed `const` tokens: each returns the matching field
// of the ACTIVE theme, so flipping the active world reskins everything. They
// keep the DaisyUI names the rest of the code already uses.

/// App background / clear plane of the active theme.
pub fn base_100() -> Srgb {
    active().base_100
}
/// Raised surface of the active theme.
pub fn base_200() -> Srgb {
    active().base_200
}
/// Focused plane / border (panel card) of the active theme.
pub fn base_300() -> Srgb {
    active().base_300
}
/// Default ink of the active theme.
pub fn base_content() -> Srgb {
    active().base_content
}
/// MUTED ink of the active theme (the de-emphasized rung of the ink ladder).
pub fn muted() -> Srgb {
    active().muted
}
/// FAINT ink of the active theme (the faintest rung — UI metadata/labels).
/// Reserved for the upcoming gutter/stats pass; see the crate `#![allow(dead_code)]`.
pub fn faint() -> Srgb {
    active().faint
}
/// Accent / caret hue of the active theme.
pub fn primary() -> Srgb {
    active().primary
}
/// Ink-on-accent of the active theme.
pub fn primary_content() -> Srgb {
    active().primary_content
}
/// Signal/error color of the active theme.
pub fn error() -> Srgb {
    active().error
}
/// Selection wash of the active theme.
pub fn selection() -> Srgb {
    active().selection
}

/// SELECTED-ROW value BAND for the summoned pickers (command palette / go-to /
/// theme / keybindings). The overlay card is `base_300`; the selected row reads as
/// a rung further up the SURFACE ladder — `base_300` stepped [`SELECTED_BAND_STEPS`]
/// more increments in the SAME direction the ramp already moves (`base_200` ->
/// `base_300`, i.e. toward the ink). Derived per-world from each theme's own surface ramp, so it brightens
/// on a dark world and darkens on a light one — figure/ground by VALUE, not hue
/// (DESIGN §5). NOT the amber accent (§3), NOT the translucent text-`selection`
/// token — a solid, opaque band so the row reads as a forward surface step.
/// How many EXTRA surface-ramp increments the selected-row band sits past
/// `base_300` — the ramp's own `base_200 -> base_300` delta is one increment, and
/// this many MORE are added on top. At 1 the band was only ~10-12/255 above the
/// card on tight-ramp worlds (default Tawny), too faint to read as selected (a live
/// web-build report). 2 roughly doubles the value step for a clearly-visible-but-
/// still-calm band, saturating gracefully at the gamut edge. TASTE DEFAULT — tunable,
/// flagged for review. Figure/ground by VALUE only (DESIGN §5): a larger value merely
/// deepens the value step in the ramp's own direction, never a hue and never the amber
/// accent. (Also nudges the HUD/word-count borders that share this owner one step.)
const SELECTED_BAND_STEPS: i32 = 2;

pub fn surface_selected() -> Srgb {
    let a = active();
    // hi + SELECTED_BAND_STEPS * (hi - lo), clamped to [0,255]: that many more
    // increments past base_300, in the SAME direction the base_200 -> base_300 step
    // already carries (toward the ink on dark worlds, toward the ground on light).
    let step = |lo: u8, hi: u8| -> u8 {
        let d = hi as i32 - lo as i32;
        (hi as i32 + d * SELECTED_BAND_STEPS).clamp(0, 255) as u8
    };
    Srgb::rgb(
        step(a.base_200.r, a.base_300.r),
        step(a.base_200.g, a.base_300.g),
        step(a.base_200.b, a.base_300.b),
    )
}

/// Alpha of the dim DOC SCRIM (`overlay_scrim`) — a translucent veil of the canvas
/// plane laid over the document while a FULL-takeover overlay is up. ~0.5 pulls the
/// doc HALF a step back toward the background so the overlay reads as the clear
/// figure, without spending a hue (DESIGN §5).
const OVERLAY_SCRIM_ALPHA: u8 = 0x80;

/// Translucent DIM SCRIM laid over the document when a FULL-takeover overlay is up
/// (command palette, go-to, theme picker, keybindings, spell picker, …): the canvas
/// plane (`base_100`) at part alpha, so the doc recedes a value behind the card and
/// the overlay is the clear figure (DESIGN §5 — "a full takeover dims the document
/// back a value"). A SPLIT surface (the search panel) does NOT use it; the doc
/// stays bright there (a peek, not a takeover). It is a value step toward the
/// background, never a new hue — so amber stays the caret's alone (§3).
pub fn overlay_scrim() -> Srgb {
    let b = active().base_100;
    Srgb::rgba(b.r, b.g, b.b, OVERLAY_SCRIM_ALPHA)
}
/// PAGE MODE margin GROUND of the active theme — the tagged [`Background`]
/// carrying its gradient endpoints + direction and any mark tint / band / angle /
/// proximity flag. Read by the background pipeline (render.rs) and the capture
/// sidecar (capture.rs).
pub fn background() -> Background {
    active().background
}

/// The section a world (by case-sensitive NAME) sits in under `lens` — the theme
/// picker's grouping key. `None` when the world OPTS OUT of the lens, for an unknown
/// name (never panics), or for [`Lens::All`] (which does not group).
pub fn tag_for(name: &str, lens: Lens) -> Option<&'static str> {
    THEMES
        .iter()
        .find(|t| t.name == name)
        .and_then(|t| t.tags.section(lens))
}

// --- The theme picker's GENERIC facet scheme --------------------------------
//
// The theme picker is the first consumer of the generic faceted-lens machinery
// ([`crate::facets`]). Its lens NAMES (Time / Register / Voice / Temperature) are
// genuinely theme-domain concepts, so [`Lens`] stays here as the source of truth;
// this bridges it into the picker-agnostic [`FacetScheme`] the overlay + renderer +
// sidecar all consult. [`THEME_FACET_STRIP`] mirrors [`Lens::STRIP`] element-for-
// element (a drift-guard test asserts it) and [`theme_bucket`] wraps [`tag_for`].

use crate::facets::{Facet, FacetItem, FacetScheme};

/// The theme picker's lens strip as generic [`Facet`]s — one per [`Lens::STRIP`]
/// entry, in the same order (All parked FIRST, the home). Kept in lockstep with
/// [`Lens`] by [`tests::theme_facet_strip_matches_lens`].
const THEME_FACET_STRIP: [Facet; 5] = [
    Facet { label: "All", id: "all", sections: &[] },
    Facet { label: "Time", id: "time", sections: &["Dawn", "Day", "Dusk", "Night"] },
    Facet { label: "Register", id: "register", sections: &["Humble", "Everyday", "Refined"] },
    Facet { label: "Voice", id: "voice", sections: &["Literary", "Technical", "Modern"] },
    Facet {
        label: "Temperature",
        id: "temperature",
        sections: &["Warm", "Cool", "Neutral"],
    },
];

/// Bucket a WORLD (by name) under the theme lens at strip index `lens_idx` — the
/// generic [`FacetScheme::bucket`] fn, wrapping [`tag_for`] over [`Lens::STRIP`].
/// `None` opts the world out of that lens (or for the All home at index 0).
fn theme_bucket(item: FacetItem, lens_idx: usize) -> Option<&'static str> {
    // The theme picker is a STRING-ONLY bucket: it reads only the world name, never
    // the dir/git flags (both always `false` for a world).
    Lens::STRIP.get(lens_idx).and_then(|l| tag_for(item.accept, *l))
}

/// The theme picker's registered [`FacetScheme`], consulted by
/// [`crate::facets::scheme`] (its one call site) and, through that, the overlay
/// state / renderer / sidecar — all picker-agnostic.
pub static THEME_FACETS: FacetScheme =
    FacetScheme { strip: &THEME_FACET_STRIP, bucket: theme_bucket };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worlds_eight_dark_six_light() {
        assert_eq!(THEMES.len(), 14);
        let dark = THEMES.iter().filter(|t| t.dark).count();
        let light = THEMES.iter().filter(|t| !t.dark).count();
        // 8 dark (Tawny/Mopoke/Currawong/Potoroo/Undertow/Kingfisher/Outback/
        // Mangrove) / 6 light (Gumtree/Bilby/Saltpan/Quokka/Galah/Magpie).
        assert_eq!(dark, 8);
        assert_eq!(light, 6);
    }

    /// Every world declares a [`Background`] ground whose gradient endpoints AND
    /// mark/band tint are OPAQUE (the shader owns the coverage, so the colors
    /// themselves stay fully opaque). The shader id stays within the known range.
    #[test]
    fn every_world_has_a_valid_background() {
        for t in THEMES.iter() {
            let bg = t.background;
            assert_eq!(bg.from().a, 0xFF, "{} background from must be opaque", t.name);
            assert_eq!(bg.to().a, 0xFF, "{} background to must be opaque", t.name);
            assert_eq!(bg.tint().a, 0xFF, "{} background tint must be opaque", t.name);
            assert!(bg.shader_id() <= 4, "{} bad shader id", t.name);
        }
        // The whole ground palette is exercised across the worlds (Stripes is new,
        // assigned to Potoroo; the proximity-scaled Dots ride Mangrove).
        let used: std::collections::HashSet<&str> =
            THEMES.iter().map(|t| t.background.as_str()).collect();
        for p in ["gradient", "dots", "starfield", "pinstripe", "stripes"] {
            assert!(used.contains(p), "ground {p} unused by any world");
        }
        // Exactly the two assigned worlds carry the NEW grounds.
        let stripes: Vec<&str> = THEMES
            .iter()
            .filter(|t| matches!(t.background, Background::Stripes { .. }))
            .map(|t| t.name)
            .collect();
        assert_eq!(stripes, ["Potoroo"], "Stripes is Potoroo's alone");
        let edge_dots: Vec<&str> = THEMES
            .iter()
            .filter(|t| t.background.edge())
            .map(|t| t.name)
            .collect();
        assert_eq!(edge_dots, ["Mangrove"], "proximity Dots is Mangrove's alone");
    }

    /// The JetBrains-Mono world (Mangrove) reports that font — the second bundled
    /// mono face, distinct from Tawny/Potoroo's IBM Plex Mono.
    #[test]
    fn mangrove_is_jetbrains_mono() {
        let m = THEMES
            .iter()
            .find(|t| t.name == "Mangrove")
            .expect("Mangrove world present");
        assert_eq!(m.font, "JetBrains Mono");
        assert!(m.dark);
        // Galah is the Figtree world.
        let g = THEMES.iter().find(|t| t.name == "Galah").unwrap();
        assert_eq!(g.font, "Figtree");
    }

    /// PER-WORLD CODE MONO: every world names a `mono` companion that is ONE of the
    /// bundled monospace families (IBM Plex Mono / JetBrains Mono / Monaspace Xenon /
    /// Iosevka). A world whose DISPLAY face is already one of those monos REUSES its own
    /// face (`mono == font`); every other world borrows a bundled mono (`mono != font`).
    #[test]
    fn every_world_has_a_bundled_mono() {
        const BUNDLED_MONOS: [&str; 4] =
            ["IBM Plex Mono", "JetBrains Mono", "Monaspace Xenon", "Iosevka"];
        // The worlds whose DISPLAY face is itself a bundled mono (so they reuse it).
        const MONO_DISPLAY: [&str; 4] = ["Tawny", "Currawong", "Potoroo", "Mangrove"];
        for t in THEMES.iter() {
            assert!(
                BUNDLED_MONOS.contains(&t.mono),
                "{}'s mono {:?} is not a bundled monospace family",
                t.name,
                t.mono
            );
            if MONO_DISPLAY.contains(&t.name) {
                assert_eq!(t.mono, t.font, "{} has a mono display face → must reuse it", t.name);
            } else {
                assert_ne!(
                    t.mono, t.font,
                    "{} is a serif/sans world → its code mono must differ from its display face",
                    t.name
                );
            }
        }
        // Sanity: the exact reuse assignments (confirmed from theme.rs).
        assert_eq!(TAWNY.mono, "IBM Plex Mono");
        assert_eq!(CURRAWONG.mono, "Iosevka");
        assert_eq!(POTOROO.mono, "Monaspace Xenon");
        assert_eq!(MANGROVE.mono, "JetBrains Mono");
        // And a couple of the borrowed assignments.
        assert_eq!(SALTPAN.mono, "Monaspace Xenon"); // Fraunces serif → slab-serif mono
        assert_eq!(KINGFISHER.mono, "JetBrains Mono"); // cool technical navy → crisp mono
        assert_eq!(GALAH.mono, "IBM Plex Mono"); // warm humanist sans → warm humanist mono
    }

    /// Every world declares a per-theme CJK (Japanese) fallback list whose
    /// CHARACTER matches the world: the SERIF worlds map to the MINCHO (serif)
    /// list, the SANS/MONO worlds to the GOTHIC (sans) list. Each list is ordered
    /// BUNDLED Noto JP first, then mac-primary (Hiragino), then linux-fallback
    /// (Noto CJK) — see the module doc on `CJK_MINCHO`/`CJK_GOTHIC`.
    #[test]
    fn cjk_fallback_matches_world_character() {
        let mincho = ["Gumtree", "Saltpan", "Bilby", "Undertow", "Outback", "Magpie"];
        let gothic = ["Tawny", "Potoroo", "Mangrove", "Quokka", "Galah", "Mopoke", "Kingfisher", "Currawong"];
        for t in THEMES.iter() {
            assert!(!t.cjk.is_empty(), "{} has no CJK fallback list", t.name);
            if mincho.contains(&t.name) {
                assert_eq!(t.cjk, CJK_MINCHO, "{} is a serif world -> mincho CJK", t.name);
            } else if gothic.contains(&t.name) {
                assert_eq!(t.cjk, CJK_GOTHIC, "{} is a sans/mono world -> gothic CJK", t.name);
            } else {
                panic!("{} not classified for CJK fallback", t.name);
            }
        }
        // Priority order: bundled Noto JP first, macOS Hiragino second, Linux Noto CJK third.
        assert_eq!(CJK_MINCHO, &["Noto Serif JP", "Hiragino Mincho ProN", "Noto Serif CJK JP"]);
        assert_eq!(CJK_GOTHIC, &["Noto Sans JP", "Hiragino Kaku Gothic ProN", "Noto Sans CJK JP"]);
    }

    /// THE NEVER-TOFU LAW (structural half — the environment-independent part
    /// of it): every [`FontId`] has a NON-EMPTY candidate ladder on EVERY
    /// world. This is the actual regression the law guards against — a world
    /// accidentally shipping an empty ladder for a script would guarantee
    /// tofu with no possible resolution, regardless of what's installed on
    /// the machine running awl. (The COMPLEMENTARY half — that `Latin`/`Ja`
    /// always resolve to a concretely-registered face via the real font DB —
    /// is `render::tests::latin_and_ja_always_resolve_to_an_embedded_face`,
    /// since it needs a built `FontSystem` to check against.)
    #[test]
    fn every_font_id_has_a_nonempty_candidate_ladder_on_every_world() {
        for t in THEMES.iter() {
            for id in ALL_FONT_IDS {
                assert!(
                    !t.candidates(id).is_empty(),
                    "{} has an EMPTY candidate ladder for {:?} — guaranteed tofu",
                    t.name,
                    id
                );
            }
        }
    }

    /// Every world's [`Theme::ornament_face`] is exactly one of the THREE bundled
    /// ornament faces — no world ships an unregistered / typo'd family that would
    /// tofu the section-break fleuron. (The font-DB half — that each face actually
    /// COVERS its world's glyphs — is `render::tests::
    /// ornament_glyphs_resolve_in_each_worlds_assigned_face`, which needs a built
    /// `FontSystem`.) Also pins `ORNAMENT_MARKS == render::SYMBOL_FAMILY`, the one
    /// coupling `theme.rs` states as data rather than importing.
    #[test]
    fn every_world_ornament_face_is_a_registered_ornament_face() {
        assert_eq!(
            ORNAMENT_MARKS,
            crate::render::SYMBOL_FAMILY,
            "the geometric worlds' ornament face IS the merged marks face"
        );
        for t in THEMES.iter() {
            assert!(
                matches!(
                    t.ornament_face,
                    ORNAMENT_GARAMOND | ORNAMENT_JUNICODE | ORNAMENT_MARKS
                ),
                "{} has an unrecognized ornament_face {:?}",
                t.name,
                t.ornament_face
            );
            // The design-table contract: THREE DISTINCT symbols per world (dash /
            // star / underscore), so a break's ornament tracks the syntax the author
            // typed instead of collapsing to one shared mark. (The font-DB half —
            // that each glyph actually resolves in `ornament_face` — is the render
            // test `ornament_glyphs_resolve_in_each_worlds_assigned_face`.)
            let (d, s, u) = (t.ornaments.dash, t.ornaments.star, t.ornaments.underscore);
            assert!(
                d != s && s != u && d != u,
                "{} ornament trio is not three distinct glyphs: dash={:?} star={:?} underscore={:?}",
                t.name,
                d,
                s,
                u
            );
        }
    }

    /// NEVER-DRIFT law: every world ships an [`Theme::ornament_scale`], and it is
    /// exactly one of the three named tier constants — a world can't silently drift to
    /// a bare literal that neither reader (`md_line_scale` / `prepare_ornaments`) would
    /// then keep in lockstep. Also pins the three tier VALUES (the taste defaults) and
    /// a sample world per tier, keyed to the ornament's CHARACTER.
    #[test]
    fn every_world_has_an_ornament_scale() {
        // The three tiers are the settled taste defaults.
        assert_eq!(ORNAMENT_SCALE_ORNATE, 2.2, "ornate tier is 2.2");
        assert_eq!(ORNAMENT_SCALE_FLEURON, 1.8, "fleuron tier is 1.8");
        assert_eq!(ORNAMENT_SCALE_GEOMETRIC, 1.5, "geometric tier is 1.5");
        assert!(
            ORNAMENT_SCALE_ORNATE > ORNAMENT_SCALE_FLEURON
                && ORNAMENT_SCALE_FLEURON > ORNAMENT_SCALE_GEOMETRIC,
            "the tiers descend ornate > fleuron > geometric"
        );

        // Every world's scale IS one of the three tiers — no stray literal.
        for t in THEMES.iter() {
            assert!(
                matches!(
                    t.ornament_scale,
                    ORNAMENT_SCALE_ORNATE | ORNAMENT_SCALE_FLEURON | ORNAMENT_SCALE_GEOMETRIC
                ),
                "{} has an off-tier ornament_scale {}",
                t.name,
                t.ornament_scale
            );
        }

        // One sample per tier (the spec's pinned assignments).
        let by = |name: &str| set_active_by_name(name).unwrap().ornament_scale;
        let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(by("Mopoke"), 2.2, "Mopoke (Junicode flowers) is ornate 2.2");
        assert_eq!(by("Undertow"), 1.8, "Undertow (Garamond fleurons) is fleuron 1.8");
        assert_eq!(by("Currawong"), 1.5, "Currawong (geometric marks) is geometric 1.5");
        set_active(DEFAULT_THEME);
    }

    /// `Theme::candidates` for `Latin` is always exactly the world's own
    /// [`Theme::font`] — a single-element floor, never a fallback list.
    #[test]
    fn latin_candidates_is_the_worlds_own_display_face() {
        for t in THEMES.iter() {
            assert_eq!(t.candidates(FontId::Latin), vec![t.font], "{}", t.name);
        }
    }

    /// THE CHINESE ROUND: zh-Hans now mirrors `cjk_fallback_matches_world_character`
    /// exactly — SERIF worlds get [`CJK_ZH_HANS_SERIF`] (bundled Noto Serif SC),
    /// SANS/MONO worlds get [`CJK_ZH_HANS_SANS`] (bundled Noto Sans SC), EXCEPT the
    /// two Klee-derived worlds (Mopoke, Quokka) which get the CHARACTERFUL
    /// [`CJK_ZH_HANS_KLEE`] override (bundled LXGW WenKai first). zh-Hant/ko remain
    /// v1-uniform (zh-Hant: still no bundled asset at all; ko: one bundled face,
    /// no serif/sans split yet — both documented taste calls, logged above).
    #[test]
    fn zh_hans_ladder_matches_world_character_with_klee_override() {
        let mincho = ["Gumtree", "Saltpan", "Bilby", "Undertow", "Outback", "Magpie"];
        let klee = ["Mopoke", "Quokka"];
        let gothic = ["Tawny", "Potoroo", "Mangrove", "Galah", "Kingfisher", "Currawong"];
        for t in THEMES.iter() {
            assert!(!t.zh_hans.is_empty(), "{} has no zh-Hans candidate list", t.name);
            if klee.contains(&t.name) {
                assert_eq!(t.zh_hans, CJK_ZH_HANS_KLEE, "{} is a Klee world -> WenKai zh-Hans", t.name);
            } else if mincho.contains(&t.name) {
                assert_eq!(t.zh_hans, CJK_ZH_HANS_SERIF, "{} is a serif world -> Serif SC zh-Hans", t.name);
            } else if gothic.contains(&t.name) {
                assert_eq!(t.zh_hans, CJK_ZH_HANS_SANS, "{} is a sans/mono world -> Sans SC zh-Hans", t.name);
            } else {
                panic!("{} not classified for zh-Hans fallback", t.name);
            }
        }
        assert_eq!(CJK_ZH_HANS_SERIF, &["Noto Serif SC", "PingFang SC", "Noto Sans CJK SC"]);
        assert_eq!(CJK_ZH_HANS_SANS, &["Noto Sans SC", "PingFang SC", "Noto Sans CJK SC"]);
        assert_eq!(
            CJK_ZH_HANS_KLEE,
            &["LXGW WenKai", "Noto Sans SC", "PingFang SC", "Noto Sans CJK SC"]
        );
    }

    /// zh-Hant/ko v1 ladders are shared identically across every world. zh-Hant
    /// still has NO bundled asset (Big5 subsetting is banked, not attempted, this
    /// round); ko now bundles Noto Sans KR first (the Chinese round's "KO
    /// rider"), but as ONE face for every world (no serif/sans split yet).
    #[test]
    fn zh_hant_and_ko_ladders_are_uniform_across_worlds() {
        for t in THEMES.iter() {
            assert_eq!(t.zh_hant, CJK_ZH_HANT, "{}", t.name);
            assert_eq!(t.ko, CJK_KO, "{}", t.name);
        }
        assert_eq!(CJK_ZH_HANT, &["PingFang TC", "Noto Sans CJK TC"]);
        assert_eq!(CJK_KO, &["Noto Sans KR", "Apple SD Gothic Neo", "Noto Sans CJK KR"]);
    }

    /// OPT-OUT faceting: a world may be `None` (hidden) on a lens, but any `Some(tag)`
    /// must be one of that lens's declared sections (so grouping can never place a world
    /// under a header that doesn't exist). Also asserts the CURATION invariant — every
    /// faceted bucket shows a curated 2–3 worlds (never empty, never crowded) — that the
    /// name-keyed accessor agrees with the inline field, that every world HEADLINES at
    /// least one faceted lens (still findable by browsing, not only by search), and that
    /// `All` groups nothing.
    #[test]
    fn every_world_curated_into_lenses() {
        for lens in [Lens::Time, Lens::Register, Lens::Voice, Lens::Temperature] {
            let sections = lens.sections();
            for t in THEMES.iter() {
                if let Some(tag) = t.tags.section(lens) {
                    assert!(
                        sections.contains(&tag),
                        "{} has invalid {:?} tag {:?} (not in {:?})",
                        t.name,
                        lens,
                        tag,
                        sections
                    );
                }
                // The name-keyed accessor agrees with the inline field.
                assert_eq!(tag_for(t.name, lens), t.tags.section(lens), "{} tag_for disagrees", t.name);
            }
            // Every declared header shows a CURATED 2–3 worlds: never an empty faint
            // header, never the pre-curation crowd (Time=Night once held 6).
            for sect in sections {
                let n = THEMES
                    .iter()
                    .filter(|t| t.tags.section(lens) == Some(*sect))
                    .count();
                assert!(
                    (2..=3).contains(&n),
                    "{:?} section {sect:?} shows {n} worlds (curation wants 2–3)",
                    lens
                );
            }
        }
        // Every world headlines at least ONE faceted lens (present under some section),
        // so it is reachable by browsing lenses, not only via All + fuzzy search.
        for t in THEMES.iter() {
            let shown = [Lens::Time, Lens::Register, Lens::Voice, Lens::Temperature]
                .iter()
                .any(|&l| t.tags.section(l).is_some());
            assert!(shown, "{} is hidden on every lens (headlines none)", t.name);
        }
        // All lens groups nothing (flat list).
        assert!(Lens::All.sections().is_empty());
        assert_eq!(THEMES[0].tags.section(Lens::All), None);
        // The strip parks All at the far LEFT.
        assert_eq!(*Lens::STRIP.first().unwrap(), Lens::All);
        assert_eq!(Lens::STRIP.len(), 5);
    }

    /// DRIFT GUARD: the generic [`THEME_FACET_STRIP`] (the `FacetScheme` the overlay
    /// consults) mirrors [`Lens::STRIP`] element-for-element — same order, labels,
    /// sidecar ids, and section lists — and [`theme_bucket`] agrees with [`tag_for`]
    /// on every world. So the theme picker's generic scheme can never diverge from
    /// the `Lens` source of truth.
    #[test]
    fn theme_facet_strip_matches_lens() {
        assert_eq!(THEME_FACET_STRIP.len(), Lens::STRIP.len());
        for (facet, lens) in THEME_FACET_STRIP.iter().zip(Lens::STRIP.iter()) {
            assert_eq!(facet.label, lens.label(), "{lens:?} label drift");
            assert_eq!(facet.id, lens.as_str(), "{lens:?} id drift");
            assert_eq!(facet.sections, lens.sections(), "{lens:?} sections drift");
        }
        // theme_bucket (strip index) == tag_for (lens) for every world × every lens.
        for (idx, lens) in Lens::STRIP.iter().enumerate() {
            for t in THEMES.iter() {
                let item = FacetItem::new(t.name);
                assert_eq!(theme_bucket(item, idx), tag_for(t.name, *lens));
            }
        }
    }

    #[test]
    fn default_is_dark() {
        assert!(THEMES[DEFAULT_THEME].dark);
        assert_eq!(THEMES[DEFAULT_THEME].name, "Tawny");
    }

    #[test]
    fn cycle_wraps_both_ways() {
        let _g = super::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_active(0);
        // Forward through all and back to start.
        for i in 1..=THEMES.len() {
            let t = cycle(1);
            assert_eq!(t.name, THEMES[i % THEMES.len()].name);
        }
        assert_eq!(active_index(), 0);
        // Backward wraps to the last world.
        let t = cycle(-1);
        assert_eq!(t.name, THEMES[THEMES.len() - 1].name);
        // restore default for other tests
        set_active(DEFAULT_THEME);
    }

    #[test]
    fn set_by_name_is_case_insensitive() {
        let _g = super::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(set_active_by_name("quokka").unwrap().name, "Quokka");
        assert_eq!(set_active_by_name("OUTBACK").unwrap().name, "Outback");
        assert!(set_active_by_name("nope").is_none());
        set_active(DEFAULT_THEME);
    }

    #[test]
    fn surface_selected_is_an_opaque_ramp_step_past_base_300() {
        let _g = super::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for (i, t) in THEMES.iter().enumerate() {
            set_active(i);
            let band = surface_selected();
            // A SOLID band (figure/ground by VALUE), never the translucent selection.
            assert_eq!(band.a, 0xFF, "{} band must be opaque", t.name);
            assert_ne!(band, t.selection, "{} band must not be the selection token", t.name);
            // Each channel continues the base_200 -> base_300 step SELECTED_BAND_STEPS
            // more increments, or saturates at the gamut edge (never reverses direction).
            let want = super::SELECTED_BAND_STEPS;
            for (lo, hi, got) in [
                (t.base_200.r, t.base_300.r, band.r),
                (t.base_200.g, t.base_300.g, band.g),
                (t.base_200.b, t.base_300.b, band.b),
            ] {
                let dir = hi as i32 - lo as i32; // ramp direction (toward the ink)
                let step = got as i32 - hi as i32; // band's move past base_300
                if dir > 0 {
                    assert!(step >= 0 && (got == 255 || step == dir * want), "{} band channel reversed", t.name);
                } else if dir < 0 {
                    assert!(step <= 0 && (got == 0 || step == dir * want), "{} band channel reversed", t.name);
                }
            }
        }
        set_active(DEFAULT_THEME);
    }

    #[test]
    fn selection_is_the_only_translucent_token() {
        for t in THEMES.iter() {
            assert_eq!(t.base_100.a, 0xFF);
            assert_eq!(t.primary.a, 0xFF);
            assert_eq!(t.error.a, 0xFF);
            // The margin gradient endpoints are opaque (the shader owns the
            // margin opacity), so selection stays the only translucent token.
            assert_eq!(t.background.from().a, 0xFF, "{} background from alpha", t.name);
            assert_eq!(t.background.to().a, 0xFF, "{} background to alpha", t.name);
            assert_eq!(t.selection.a, 0x52, "{} selection alpha", t.name);
        }
    }

    /// WYSIWYG VALUE-STEP LAW (`render/rects.rs`'s fenced-code PANEL + inline-code
    /// PILL, `fence_panel_pipeline`/`code_pill_pipeline` in `render.rs`): both quads
    /// reuse the ALREADY-DECLARED `base_200` token verbatim — no new color
    /// derivation, so this is not a new hue/wash formula to law-test. Two minimal
    /// properties DO matter now that the token draws as a distinct opaque surface
    /// rather than just a margin-gradient stop:
    /// (a) it must actually READ as a step off the ground (`base_100`) — an
    /// invisible panel/pill defeats its own affordance — and
    /// (b) it must never be LITERALLY the accent color (a background step sharing
    /// `primary`'s general warmth is fine and common — many worlds tint their whole
    /// ground ramp toward their signature hue, already covered by the ground-
    /// contrast + background-validity laws above — but it must never be an exact
    /// hit, which would make the panel read as a spent accent rather than a ground
    /// step).
    #[test]
    fn wysiwyg_value_step_law_holds_for_every_world() {
        for t in THEMES.iter() {
            assert_ne!(
                t.base_200, t.base_100,
                "{}: base_200 must differ from base_100 or the WYSIWYG panel/pill is invisible",
                t.name
            );
            assert_ne!(
                t.base_200, t.primary,
                "{}: base_200 must never be exactly the accent color", t.name
            );
        }
    }

    /// Every world defines a NON-DEGENERATE margin gradient: the two endpoints
    /// differ (so there is a real gradient, not a flat fill) and the direction
    /// vector is non-zero (so `dot(uv, dir)` actually varies across the margin).
    #[test]
    fn every_world_has_a_real_margin_gradient() {
        for t in THEMES.iter() {
            let bg = t.background;
            assert_ne!(
                bg.from(), bg.to(),
                "{} margin gradient is degenerate (from == to)",
                t.name
            );
            let (dx, dy) = bg.dir();
            assert!(
                dx.abs() + dy.abs() > 0.0,
                "{} background dir is the zero vector",
                t.name
            );
        }
    }

    #[test]
    fn hex_round_trips_known_values() {
        assert_eq!(POTOROO.base_100.hex(), "#1f0400");
        assert_eq!(POTOROO.primary.hex(), "#feaf69");
        assert_eq!(GUMTREE.base_100.hex(), "#e4f8e2");
        // Tawny — the default world's exact spec hexes.
        assert_eq!(TAWNY.base_100.hex(), "#16181d");
        assert_eq!(TAWNY.base_content.hex(), "#e6e6e6");
        assert_eq!(TAWNY.primary.hex(), "#ffc05e");
        assert_eq!(TAWNY.error.hex(), "#e54b4b");
        assert_eq!(TAWNY.selection.hex(), "#3a6fd8");
    }

    /// The fourteen worlds map onto at least SIX CLEARLY-distinct display faces
    /// (IBM Plex Mono / JetBrains Mono / Literata / Newsreader / IBM Plex Sans /
    /// Figtree / Zilla Slab), so cycling worlds visibly reskins the glyph shapes,
    /// not just the palette. The two newly-registered faces (JetBrains Mono,
    /// Figtree) are both present.
    #[test]
    fn at_least_six_distinct_faces() {
        let mut faces: Vec<&str> = THEMES.iter().map(|t| t.font).collect();
        faces.sort_unstable();
        faces.dedup();
        assert!(
            faces.len() >= 6,
            "expected >=6 distinct display faces, got {faces:?}"
        );
        assert!(faces.contains(&"JetBrains Mono"), "JetBrains Mono missing");
        assert!(faces.contains(&"Figtree"), "Figtree missing");
        // Home (Tawny) renders in the bundled mono so it looks exactly like home.
        assert_eq!(TAWNY.font, "IBM Plex Mono");
    }
}
