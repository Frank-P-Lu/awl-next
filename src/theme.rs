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
    /// Currawong / Mangrove = JetBrains Mono, Potoroo = Monaspace Xenon) REUSES its
    /// own face here; every serif / sans world borrows one of the three bundled
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
    /// The fine-press SECTION-BREAK ornament SET: markdown has THREE thematic-break
    /// syntaxes (`---` / `***` / `___`, all a `<hr>` in standard md), and awl makes
    /// each EXPRESSIVE — the author picks a break's feel by which one they type, and
    /// each renders a DIFFERENT centered ornament (a printer's fleuron, not a
    /// hairline). See [`Ornaments`] for the per-syntax glyphs + defaults; every world
    /// shares [`ORNAMENTS_DEFAULT`] unless it overrides for its own face's flavour.
    /// All covered by the bundled `SYMBOL_FAMILY` face so they render in all 14 worlds.
    pub ornaments: Ornaments,
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
/// `AwlSymbols.ttf`, so they render in every world that doesn't override.
pub const ORNAMENTS_DEFAULT: Ornaments = Ornaments { dash: '❧', star: '⁂', underscore: '❦' };

// --- The faceted THEME-PICKER lenses + per-world tags -----------------------
//
// The theme picker is a FACETED lens-switcher: LEFT/RIGHT cycle a [`Lens`], each
// grouping the worlds by ONE dimension into faint sections. Every world carries a
// value on EACH of the four real lenses ([`ThemeTags`]); `All` is the flat list.

/// A faceting LENS for the theme picker. The four real dimensions group the worlds
/// into sections; `All` is the flat, fuzzy-searchable list (today's behaviour).
/// Ordered for the LEFT/RIGHT strip with `All` PARKED at the FAR RIGHT ([`Lens::STRIP`]).
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
    /// The lens STRIP order, LEFT→RIGHT, with `All` parked at the FAR RIGHT end.
    /// LEFT/RIGHT step through this (clamped at both ends); the picker opens on
    /// [`Lens::Time`], the first faceted view.
    pub const STRIP: [Lens; 5] = [Lens::Time, Lens::Register, Lens::Voice, Lens::Temperature, Lens::All];

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
/// defaults are DERIVED from the world's own palette + font (see the doc on each
/// world): Time by background lightness/warmth, Register by font formality, Voice
/// by face class, Temperature by ground hue. These are DEFAULTS the user can adjust.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeTags {
    /// Section under [`Lens::Time`] (Dawn / Day / Dusk / Night).
    pub time: &'static str,
    /// Section under [`Lens::Register`] (Humble / Everyday / Refined).
    pub register: &'static str,
    /// Section under [`Lens::Voice`] (Literary / Technical / Modern).
    pub voice: &'static str,
    /// Section under [`Lens::Temperature`] (Warm / Cool / Neutral).
    pub temperature: &'static str,
}

impl ThemeTags {
    /// This world's section under `lens` (empty string for [`Lens::All`], which
    /// does not group).
    pub fn section(&self, lens: Lens) -> &'static str {
        match lens {
            Lens::Time => self.time,
            Lens::Register => self.register,
            Lens::Voice => self.voice,
            Lens::Temperature => self.temperature,
            Lens::All => "",
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
    ornaments: ORNAMENTS_DEFAULT,
    // Pale cool-green ground → Day; Literata reading serif → Refined / Literary; green hue → Cool.
    tags: ThemeTags { time: "Day", register: "Refined", voice: "Literary", temperature: "Cool" },
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
    ornaments: ORNAMENTS_DEFAULT,
    // Dark burnt-orange room → Dusk (warm dark); Monaspace mono → Humble / Technical; rust hue → Warm.
    tags: ThemeTags { time: "Dusk", register: "Humble", voice: "Technical", temperature: "Warm" },
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
    ornaments: ORNAMENTS_DEFAULT,
    // Pale blue ground → Day; Newsreader display serif → Refined / Literary; blue hue → Cool.
    tags: ThemeTags { time: "Day", register: "Refined", voice: "Literary", temperature: "Cool" },
    role_overrides: RoleOverrides::NONE,
};

/// Saltpan — light sun-bleached salt flat (cinnamon-clay caret on warm ecru).
pub const SALTPAN: Theme = Theme {
    name: "Saltpan",
    dark: false,
    base_100: Srgb::rgb(0xFF, 0xFD, 0xF2),
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
    ornaments: ORNAMENTS_DEFAULT,
    // Warm ecru salt flat → Dawn (warm-soft light); Fraunces old-style serif → Refined / Literary; sand hue → Warm.
    tags: ThemeTags { time: "Dawn", register: "Refined", voice: "Literary", temperature: "Warm" },
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
    font: "IBM Plex Sans",
    // Warm modern sans → the warm humanist IBM Plex Mono (Plex Sans' mono kin).
    mono: "IBM Plex Mono",
    cjk: CJK_GOTHIC,
    ornaments: ORNAMENTS_DEFAULT,
    // Warm peach reef → Dawn (warm-soft light); IBM Plex Sans workhorse → Everyday / Modern; peach hue → Warm.
    tags: ThemeTags { time: "Dawn", register: "Everyday", voice: "Modern", temperature: "Warm" },
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
    // OVERRIDE (the serif nocturne's flourish): mirror the default fleuron into its
    // reversed twin ☙ for `---`, and swap `___`'s heart to the black-heart bullet ❥
    // (both NS2 ornament variants, also bundled). `***` keeps the ⁂ asterism.
    ornaments: Ornaments { dash: '☙', star: '⁂', underscore: '❥' },
    // Dark violet current → Night; EB Garamond classic serif → Refined / Literary; violet-blue hue → Cool.
    tags: ThemeTags { time: "Night", register: "Refined", voice: "Literary", temperature: "Cool" },
    role_overrides: RoleOverrides::NONE,
};

/// Outback — dark red-centre night (hays-russet caret in blackish-olive room).
pub const OUTBACK: Theme = Theme {
    name: "Outback",
    dark: true,
    base_100: Srgb::rgb(0x16, 0x1D, 0x14),
    base_200: Srgb::rgb(0x1E, 0x27, 0x1C),
    base_300: Srgb::rgb(0x3F, 0x49, 0x3C),
    base_content: Srgb::rgb(0xEC, 0xEA, 0xE0),
    muted: Srgb::rgb(0x8A, 0x8C, 0x78),
    faint: Srgb::rgb(0x51, 0x56, 0x47),
    primary: Srgb::rgb(0xDE, 0x8E, 0x7F),
    primary_content: Srgb::rgb(0x2A, 0x14, 0x10),
    error: Srgb::rgb(0xFF, 0x6B, 0x5C),
    selection: Srgb::rgba(0xFF, 0xEF, 0xAE, 0x52),
    background: Background::Starfield {
        from: Srgb::rgb(0x16, 0x1D, 0x14),
        to: Srgb::rgb(0x1E, 0x27, 0x1C),
        dir: (0.0, 1.0),
        tint: Srgb::rgb(0x7C, 0x80, 0x68),
    },
    font: "Zilla Slab",
    // Slab-serif display → Monaspace Xenon: the only slab-serif mono, matching Zilla.
    mono: "Monaspace Xenon",
    cjk: CJK_MINCHO,
    ornaments: ORNAMENTS_DEFAULT,
    // Blackish-olive night → Night; Zilla Slab workhorse slab → Everyday; slab-serif face → Literary; olive-green hue → Cool.
    tags: ThemeTags { time: "Night", register: "Everyday", voice: "Literary", temperature: "Cool" },
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
    ornaments: ORNAMENTS_DEFAULT,
    // Warm-grey neutral nocturne → Night; IBM Plex Mono → Humble / Technical; near-neutral grey → Neutral.
    tags: ThemeTags { time: "Night", register: "Humble", voice: "Technical", temperature: "Neutral" },
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
    ornaments: ORNAMENTS_DEFAULT,
    // Warm charcoal cosy dark → Dusk (warm dark); iA Writer Quattro utilitarian → Humble; sans-class writing face → Modern; warm hue → Warm.
    tags: ThemeTags { time: "Dusk", register: "Humble", voice: "Modern", temperature: "Warm" },
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
    ornaments: ORNAMENTS_DEFAULT,
    // Midnight-navy nocturne → Night; IBM Plex Sans workhorse → Everyday / Modern; blue-black hue → Cool.
    tags: ThemeTags { time: "Night", register: "Everyday", voice: "Modern", temperature: "Cool" },
    role_overrides: RoleOverrides::NONE,
};

/// Currawong — a near-pure-black OLED world: the deepest base awl ships, planes
/// of true black for maximum contrast and a power-sipping dark, cool off-white
/// ink, and a single gold-YELLOW caret echoing the Pied Currawong's yellow eye.
/// A calm, minimal margin (a plain Gradient, no pattern noise). Drawn in the crisp
/// JetBrains Mono — a quiet coding den at midnight.
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
    font: "JetBrains Mono",
    // Display face is ALREADY JetBrains Mono → reuse it for code.
    mono: "JetBrains Mono",
    cjk: CJK_GOTHIC,
    ornaments: ORNAMENTS_DEFAULT,
    // Near-pure-black OLED → Night; JetBrains Mono → Humble / Technical; true-black neutral → Neutral.
    tags: ThemeTags { time: "Night", register: "Humble", voice: "Technical", temperature: "Neutral" },
    role_overrides: RoleOverrides::NONE,
};

/// Mangrove — dark tidal-teal coding den (one warm low-tide ember at the caret).
/// The room is cool teal/blue-green; the single warm living thing is an
/// amber-coral caret. Drawn in JetBrains Mono — the second bundled mono face, a
/// crisp coding home distinct from Tawny's warm grey.
pub const MANGROVE: Theme = Theme {
    name: "Mangrove",
    dark: true,
    base_100: Srgb::rgb(0x0D, 0x1A, 0x19),
    base_200: Srgb::rgb(0x14, 0x25, 0x23),
    base_300: Srgb::rgb(0x21, 0x35, 0x2F),
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
        from: Srgb::rgb(0x0D, 0x1A, 0x19),
        to: Srgb::rgb(0x14, 0x25, 0x23),
        dir: (0.0, 1.0),
        tint: Srgb::rgb(0x23, 0x3B, 0x35),
        edge: true,
    },
    font: "JetBrains Mono",
    // Display face is ALREADY JetBrains Mono → reuse it for code.
    mono: "JetBrains Mono",
    cjk: CJK_GOTHIC,
    ornaments: ORNAMENTS_DEFAULT,
    // Dark tidal-teal den → Night; JetBrains Mono → Humble / Technical; teal hue → Cool.
    tags: ThemeTags { time: "Night", register: "Humble", voice: "Technical", temperature: "Cool" },
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
    ornaments: ORNAMENTS_DEFAULT,
    // Dusty-pink reading room → Dawn (warm-soft light); Figtree humanist sans → Everyday / Modern; rose hue → Warm.
    tags: ThemeTags { time: "Dawn", register: "Everyday", voice: "Modern", temperature: "Warm" },
    role_overrides: RoleOverrides::NONE,
};

/// Magpie — light stark high-contrast page (terracotta spark at the caret).
/// Near-neutral paper-white with near-black slab ink: maximum value contrast,
/// magpie black-and-white. The one warm thing is a terracotta-vermilion caret.
/// Drawn in bold Zilla Slab for a confident newsprint-headline stance.
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
    font: "Zilla Slab",
    // Slab-serif display → Monaspace Xenon: the slab-serif mono matches Zilla's stance.
    mono: "Monaspace Xenon",
    cjk: CJK_MINCHO,
    ornaments: ORNAMENTS_DEFAULT,
    // Paper-white high-contrast page → Day; Zilla Slab workhorse slab → Everyday; slab-serif face → Literary; near-neutral hue → Neutral.
    tags: ThemeTags { time: "Day", register: "Everyday", voice: "Literary", temperature: "Neutral" },
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
/// `page::TEST_LOCK` / `debug::TEST_LOCK`.
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
/// the NEXT rung up the SURFACE ladder — `base_300` stepped one more increment in
/// the SAME direction the ramp already moves (`base_200` -> `base_300`, i.e. toward
/// the ink). Derived per-world from each theme's own surface ramp, so it brightens
/// on a dark world and darkens on a light one — figure/ground by VALUE, not hue
/// (DESIGN §5). NOT the amber accent (§3), NOT the translucent text-`selection`
/// token — a solid, opaque band so the row reads as a forward surface step.
pub fn surface_selected() -> Srgb {
    let a = active();
    // hi + (hi - lo), clamped to [0,255]: one more increment past base_300, the
    // same delta the base_200 -> base_300 step already carries.
    let step = |lo: u8, hi: u8| -> u8 {
        let d = hi as i32 - lo as i32;
        (hi as i32 + d).clamp(0, 255) as u8
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
/// picker's grouping key. Falls back to an empty string for an unknown name (never
/// panics); [`Lens::All`] always yields empty (it does not group).
pub fn tag_for(name: &str, lens: Lens) -> &'static str {
    THEMES
        .iter()
        .find(|t| t.name == name)
        .map(|t| t.tags.section(lens))
        .unwrap_or("")
}

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
    /// three bundled monospace families (IBM Plex Mono / JetBrains Mono / Monaspace
    /// Xenon). A world whose DISPLAY face is already one of those monos REUSES its own
    /// face (`mono == font`); every other world borrows a bundled mono (`mono != font`).
    #[test]
    fn every_world_has_a_bundled_mono() {
        const BUNDLED_MONOS: [&str; 3] = ["IBM Plex Mono", "JetBrains Mono", "Monaspace Xenon"];
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
        assert_eq!(CURRAWONG.mono, "JetBrains Mono");
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

    /// Every world carries a value on EVERY real lens, and each value is one of
    /// that lens's declared sections (so grouping can never orphan a world). Also
    /// asserts every section of every lens is populated by at least one world (no
    /// empty faint header), and that `All` groups nothing.
    #[test]
    fn every_world_tagged_on_every_lens() {
        for lens in [Lens::Time, Lens::Register, Lens::Voice, Lens::Temperature] {
            let sections = lens.sections();
            for t in THEMES.iter() {
                let tag = t.tags.section(lens);
                assert!(
                    sections.contains(&tag),
                    "{} has invalid {:?} tag {:?} (not in {:?})",
                    t.name,
                    lens,
                    tag,
                    sections
                );
                // The name-keyed accessor agrees with the inline field.
                assert_eq!(tag_for(t.name, lens), tag, "{} tag_for disagrees", t.name);
            }
            // No empty section: every declared header has at least one world under it.
            for sect in sections {
                assert!(
                    THEMES.iter().any(|t| t.tags.section(lens) == *sect),
                    "{:?} section {sect:?} has no worlds",
                    lens
                );
            }
        }
        // All lens groups nothing (flat list).
        assert!(Lens::All.sections().is_empty());
        assert_eq!(THEMES[0].tags.section(Lens::All), "");
        // The strip parks All at the far right.
        assert_eq!(*Lens::STRIP.last().unwrap(), Lens::All);
        assert_eq!(Lens::STRIP.len(), 5);
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
            // Each channel continues the base_200 -> base_300 step one more increment,
            // or saturates at the gamut edge (never reverses direction).
            for (lo, hi, got) in [
                (t.base_200.r, t.base_300.r, band.r),
                (t.base_200.g, t.base_300.g, band.g),
                (t.base_200.b, t.base_300.b, band.b),
            ] {
                let dir = hi as i32 - lo as i32; // ramp direction (toward the ink)
                let step = got as i32 - hi as i32; // band's move past base_300
                if dir > 0 {
                    assert!(step >= 0 && (got == 255 || step == dir), "{} band channel reversed", t.name);
                } else if dir < 0 {
                    assert!(step <= 0 && (got == 0 || step == dir), "{} band channel reversed", t.name);
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
