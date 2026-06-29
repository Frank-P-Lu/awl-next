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
}

/// The procedural MARGIN pattern a world paints over its gradient ground (PAGE
/// MODE). All four are pure pixel-coordinate shaders (no assets, no clock), so
/// the headless capture stays byte-deterministic. They whisper: a dim
/// `pattern_color` is mixed into the gradient at low coverage so the page (the
/// flat base_100 column) always stays the clear figure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BgPattern {
    /// Plain gradient, no extra marks (the calmest ground).
    Gradient,
    /// A subtle perforated grid of dots.
    DotGrid,
    /// Scattered sparkles + dots — a quiet cosmos.
    Starfield,
    /// Fine parallel lines (ledger / print rules).
    Pinstripe,
}

impl BgPattern {
    /// The shader's discriminant (must match `shaders/background.wgsl`).
    pub fn shader_id(self) -> u32 {
        match self {
            BgPattern::Gradient => 0,
            BgPattern::DotGrid => 1,
            BgPattern::Starfield => 2,
            BgPattern::Pinstripe => 3,
        }
    }
    /// Lowercase name for the capture sidecar (`gradient`/`dotgrid`/…).
    pub fn as_str(self) -> &'static str {
        match self {
            BgPattern::Gradient => "gradient",
            BgPattern::DotGrid => "dotgrid",
            BgPattern::Starfield => "starfield",
            BgPattern::Pinstripe => "pinstripe",
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
// NOTE: no `Eq` — `margin_dir: (f32, f32)` is a float pair (f32 is not `Eq`).
// `PartialEq` is enough (Theme is never used as a hash/btree key).
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
    /// PAGE MODE margin gradient START color (the styled ground the centered page
    /// floats on). Opaque; the margin shader applies its own opacity. Dark worlds
    /// stay subtle (base_100 -> base_200); light/warm worlds read a touch richer.
    pub margin_from: Srgb,
    /// PAGE MODE margin gradient END color (gradient target).
    pub margin_to: Srgb,
    /// PAGE MODE margin gradient DIRECTION as a (roughly unit) vector in UV space:
    /// (0,1) = vertical (top->bottom), (0.7,0.7) = diagonal.
    pub margin_dir: (f32, f32),
    /// PAGE MODE margin PATTERN: a quiet procedural shader drawn over the gradient
    /// (dots / stars / stripes) so the ground reads tactile, never loud. The page
    /// column itself stays the flat base_100 figure.
    pub pattern: BgPattern,
    /// Tint of the margin pattern's marks: a dim, low-contrast value so the dots /
    /// stars / stripes whisper against the gradient (the shader mixes it in at low
    /// coverage). Opaque; the shader owns the coverage.
    pub pattern_color: Srgb,
    /// Chosen display font family for this world (recorded; glyphon switching is
    /// a follow-up — see the module note).
    pub font: &'static str,
    /// PRIORITIZED CJK fallback family list for this world (mac primary, linux
    /// fallback). The bundled Latin/display faces carry NO Japanese glyphs, so
    /// Japanese text falls back to a system CJK face; this picks one whose
    /// CHARACTER matches the world — a MINCHO (serif) face for the serif worlds,
    /// a GOTHIC (sans) face for the sans/mono worlds. cosmic-text consults these
    /// in order and uses the first family the system actually has (see
    /// `render.rs::resolve_cjk`). If NONE is installed, the renderer adds no CJK
    /// span and shaping falls through to cosmic-text's neutral platform fallback.
    pub cjk: &'static [&'static str],
}

// --- Per-theme CJK fallback families (mincho / gothic) ---------------------
//
// Two prioritized lists, macOS primary then Linux fallback. These are SYSTEM
// fonts (never bundled): on macOS the Hiragino family, on Linux the Noto CJK
// family. cosmic-text picks the first one the running system has installed.

/// MINCHO (serif) Japanese fallback for the SERIF worlds: Hiragino Mincho ProN
/// on macOS, Noto Serif CJK JP on Linux.
pub const CJK_MINCHO: &[&str] = &["Hiragino Mincho ProN", "Noto Serif CJK JP"];

/// GOTHIC (sans) Japanese fallback for the SANS / MONO worlds: Hiragino Kaku
/// Gothic ProN on macOS, Noto Sans CJK JP on Linux.
pub const CJK_GOTHIC: &[&str] = &["Hiragino Kaku Gothic ProN", "Noto Sans CJK JP"];

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
    faint: Srgb::rgb(0x8A, 0x9C, 0x88),
    primary: Srgb::rgb(0xDA, 0x52, 0x5D),
    primary_content: Srgb::rgb(0xFB, 0xEC, 0xEC),
    error: Srgb::rgb(0xC0, 0x39, 0x2B),
    selection: Srgb::rgba(0x88, 0x8F, 0x5D, 0x52),
    margin_from: Srgb::rgb(0xCF, 0xF3, 0xCC),
    margin_to: Srgb::rgb(0xB7, 0xEF, 0xB4),
    margin_dir: (0.7, 0.7),
    pattern: BgPattern::DotGrid,
    pattern_color: Srgb::rgb(0x93, 0xA8, 0x7A),
    font: "Literata",
    cjk: CJK_MINCHO,
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
    faint: Srgb::rgb(0x70, 0x58, 0x4D),
    primary: Srgb::rgb(0xFE, 0xAF, 0x69),
    primary_content: Srgb::rgb(0x2A, 0x14, 0x02),
    error: Srgb::rgb(0xFF, 0x6B, 0x5C),
    selection: Srgb::rgba(0x7E, 0xB4, 0x7C, 0x52),
    margin_from: Srgb::rgb(0x1F, 0x04, 0x00),
    margin_to: Srgb::rgb(0x56, 0x28, 0x00),
    margin_dir: (0.7, 0.7),
    pattern: BgPattern::Pinstripe,
    pattern_color: Srgb::rgb(0x6B, 0x3A, 0x12),
    font: "IBM Plex Mono",
    cjk: CJK_GOTHIC,
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
    faint: Srgb::rgb(0x88, 0xA0, 0xA8),
    primary: Srgb::rgb(0xAA, 0x94, 0x34),
    primary_content: Srgb::rgb(0xFB, 0xF6, 0xE4),
    error: Srgb::rgb(0xC0, 0x39, 0x2B),
    selection: Srgb::rgba(0x5B, 0xA3, 0xC5, 0x52),
    margin_from: Srgb::rgb(0xCF, 0xF3, 0xFF),
    margin_to: Srgb::rgb(0xB3, 0xE7, 0xFB),
    margin_dir: (0.7, 0.7),
    pattern: BgPattern::Gradient,
    pattern_color: Srgb::rgb(0x8F, 0xC4, 0xDB),
    // Newsreader registers under this exact fontdb family name (it ships as the
    // "16pt" optical-size master), so `Family::Name` must match it verbatim.
    font: "Newsreader 16pt 16pt",
    cjk: CJK_MINCHO,
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
    faint: Srgb::rgb(0xA9, 0xA0, 0x8C),
    primary: Srgb::rgb(0x8D, 0x59, 0x25),
    primary_content: Srgb::rgb(0xFB, 0xF1, 0xE6),
    error: Srgb::rgb(0xB5, 0x45, 0x2B),
    selection: Srgb::rgba(0xA5, 0x86, 0x50, 0x52),
    margin_from: Srgb::rgb(0xFB, 0xF3, 0xDE),
    margin_to: Srgb::rgb(0xF2, 0xE6, 0xC7),
    margin_dir: (0.0, 1.0),
    pattern: BgPattern::Pinstripe,
    pattern_color: Srgb::rgb(0xD9, 0xC7, 0x9B),
    font: "Literata",
    cjk: CJK_MINCHO,
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
    faint: Srgb::rgb(0xB3, 0x93, 0x83),
    primary: Srgb::rgb(0x07, 0x70, 0x73),
    primary_content: Srgb::rgb(0xE6, 0xF6, 0xF6),
    error: Srgb::rgb(0xC0, 0x39, 0x2B),
    selection: Srgb::rgba(0xBB, 0x80, 0x20, 0x52),
    margin_from: Srgb::rgb(0xFF, 0xDF, 0xCF),
    margin_to: Srgb::rgb(0xFF, 0xD2, 0xBD),
    margin_dir: (0.7, 0.7),
    pattern: BgPattern::DotGrid,
    pattern_color: Srgb::rgb(0xE0, 0xAE, 0x92),
    font: "IBM Plex Sans",
    cjk: CJK_GOTHIC,
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
    faint: Srgb::rgb(0x61, 0x56, 0x7D),
    primary: Srgb::rgb(0xC5, 0x3C, 0x69),
    primary_content: Srgb::rgb(0x2A, 0x0A, 0x16),
    error: Srgb::rgb(0xFF, 0x6B, 0x5C),
    selection: Srgb::rgba(0x4F, 0x40, 0x86, 0x52),
    margin_from: Srgb::rgb(0x15, 0x0A, 0x2C),
    margin_to: Srgb::rgb(0x24, 0x15, 0x40),
    margin_dir: (0.0, 1.0),
    pattern: BgPattern::Starfield,
    pattern_color: Srgb::rgb(0x7A, 0x6C, 0xA8),
    // See BILBY: Newsreader's exact registered family name.
    font: "Newsreader 16pt 16pt",
    cjk: CJK_MINCHO,
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
    faint: Srgb::rgb(0x61, 0x65, 0x55),
    primary: Srgb::rgb(0xDE, 0x8E, 0x7F),
    primary_content: Srgb::rgb(0x2A, 0x14, 0x10),
    error: Srgb::rgb(0xFF, 0x6B, 0x5C),
    selection: Srgb::rgba(0xFF, 0xEF, 0xAE, 0x52),
    margin_from: Srgb::rgb(0x16, 0x1D, 0x14),
    margin_to: Srgb::rgb(0x1E, 0x27, 0x1C),
    margin_dir: (0.0, 1.0),
    pattern: BgPattern::Starfield,
    pattern_color: Srgb::rgb(0x7C, 0x80, 0x68),
    font: "Zilla Slab",
    cjk: CJK_MINCHO,
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
    faint: Srgb::rgb(0x62, 0x67, 0x70),
    primary: Srgb::rgb(0xFF, 0xC0, 0x5E),
    primary_content: Srgb::rgb(0x26, 0x1A, 0x08),
    error: Srgb::rgb(0xE5, 0x4B, 0x4B),
    selection: Srgb::rgba(0x3A, 0x6F, 0xD8, 0x52),
    margin_from: Srgb::rgb(0x16, 0x18, 0x1D),
    margin_to: Srgb::rgb(0x20, 0x22, 0x28),
    margin_dir: (0.0, 1.0),
    pattern: BgPattern::DotGrid,
    pattern_color: Srgb::rgb(0x2C, 0x2F, 0x37),
    font: "IBM Plex Mono",
    cjk: CJK_GOTHIC,
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
    faint: Srgb::rgb(0x6C, 0x63, 0x59),
    primary: Srgb::rgb(0xFF, 0xC0, 0x5E),
    primary_content: Srgb::rgb(0x26, 0x1A, 0x08),
    error: Srgb::rgb(0xE5, 0x4B, 0x4B),
    selection: Srgb::rgba(0x3A, 0x6F, 0xD8, 0x52),
    margin_from: Srgb::rgb(0x1B, 0x18, 0x14),
    margin_to: Srgb::rgb(0x25, 0x21, 0x1B),
    margin_dir: (0.0, 1.0),
    pattern: BgPattern::DotGrid,
    pattern_color: Srgb::rgb(0x33, 0x2D, 0x24),
    font: "IBM Plex Mono",
    cjk: CJK_GOTHIC,
};

/// Frogmouth — the namesake's coat: a balanced warm grey-brown dim room, the
/// sweet spot of the warm-Tawny trio. IBM Plex Mono + amber-eye caret.
/// (Provisional name; warm-Tawny "B".)
pub const FROGMOUTH: Theme = Theme {
    name: "Frogmouth",
    dark: true,
    base_100: Srgb::rgb(0x22, 0x1E, 0x18),
    base_200: Srgb::rgb(0x2C, 0x27, 0x1F),
    base_300: Srgb::rgb(0x39, 0x32, 0x29),
    base_content: Srgb::rgb(0xEA, 0xE5, 0xDC),
    muted: Srgb::rgb(0xA1, 0x96, 0x86),
    faint: Srgb::rgb(0x75, 0x6C, 0x60),
    primary: Srgb::rgb(0xFF, 0xC0, 0x5E),
    primary_content: Srgb::rgb(0x26, 0x1A, 0x08),
    error: Srgb::rgb(0xE5, 0x4B, 0x4B),
    selection: Srgb::rgba(0x3A, 0x6F, 0xD8, 0x52),
    margin_from: Srgb::rgb(0x22, 0x1E, 0x18),
    margin_to: Srgb::rgb(0x2C, 0x27, 0x1F),
    margin_dir: (0.0, 1.0),
    pattern: BgPattern::DotGrid,
    pattern_color: Srgb::rgb(0x3B, 0x34, 0x2A),
    font: "IBM Plex Mono",
    cjk: CJK_GOTHIC,
};

/// Bracken — the warmest, lightest warm-Tawny: a taupe-brown lift toward the
/// frogmouth's mottled plumage, coziest of the trio. IBM Plex Mono + amber-eye
/// caret. (Provisional name; warm-Tawny "C".)
pub const BRACKEN: Theme = Theme {
    name: "Bracken",
    dark: true,
    base_100: Srgb::rgb(0x2A, 0x24, 0x1D),
    base_200: Srgb::rgb(0x35, 0x2E, 0x24),
    base_300: Srgb::rgb(0x44, 0x3A, 0x2E),
    base_content: Srgb::rgb(0xED, 0xE7, 0xDC),
    muted: Srgb::rgb(0xAB, 0x9E, 0x8C),
    faint: Srgb::rgb(0x7E, 0x73, 0x65),
    primary: Srgb::rgb(0xFF, 0xC0, 0x5E),
    primary_content: Srgb::rgb(0x26, 0x1A, 0x08),
    error: Srgb::rgb(0xE5, 0x4B, 0x4B),
    selection: Srgb::rgba(0x3A, 0x6F, 0xD8, 0x52),
    margin_from: Srgb::rgb(0x2A, 0x24, 0x1D),
    margin_to: Srgb::rgb(0x35, 0x2E, 0x24),
    margin_dir: (0.0, 1.0),
    pattern: BgPattern::DotGrid,
    pattern_color: Srgb::rgb(0x46, 0x3C, 0x2E),
    font: "IBM Plex Mono",
    cjk: CJK_GOTHIC,
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
    faint: Srgb::rgb(0x4D, 0x63, 0x5E),
    primary: Srgb::rgb(0xF2, 0xA6, 0x5C),
    primary_content: Srgb::rgb(0x2A, 0x18, 0x04),
    error: Srgb::rgb(0xFF, 0x6B, 0x5C),
    selection: Srgb::rgba(0x2F, 0x80, 0x79, 0x52),
    margin_from: Srgb::rgb(0x0D, 0x1A, 0x19),
    margin_to: Srgb::rgb(0x14, 0x25, 0x23),
    margin_dir: (0.0, 1.0),
    pattern: BgPattern::DotGrid,
    pattern_color: Srgb::rgb(0x23, 0x3B, 0x35),
    font: "JetBrains Mono",
    cjk: CJK_GOTHIC,
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
    margin_from: Srgb::rgb(0xF8, 0xE0, 0xE6),
    margin_to: Srgb::rgb(0xF1, 0xCF, 0xD9),
    margin_dir: (0.7, 0.7),
    pattern: BgPattern::Gradient,
    pattern_color: Srgb::rgb(0xC9, 0x9F, 0xAE),
    font: "Figtree",
    cjk: CJK_GOTHIC,
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
    faint: Srgb::rgb(0x9E, 0xA1, 0xA5),
    primary: Srgb::rgb(0xDB, 0x5A, 0x2B),
    primary_content: Srgb::rgb(0xFB, 0xEF, 0xE9),
    error: Srgb::rgb(0xC0, 0x39, 0x2B),
    selection: Srgb::rgba(0x46, 0x61, 0x8F, 0x52),
    margin_from: Srgb::rgb(0xF1, 0xF1, 0xEF),
    margin_to: Srgb::rgb(0xE4, 0xE4, 0xE1),
    margin_dir: (0.0, 1.0),
    pattern: BgPattern::Pinstripe,
    pattern_color: Srgb::rgb(0xC9, 0xC9, 0xC5),
    font: "Zilla Slab",
    cjk: CJK_MINCHO,
};

/// All fourteen worlds, in cycle order. `C-x t` advances through this list and
/// wraps; `C-x T` steps backward. The DEFAULT (index 0) is Tawny: a quiet
/// warm-grey dark world whose display font is the original bundled IBM Plex
/// Mono, so the app opens on awl's familiar mono "home" look. The three newest
/// worlds (Mangrove / Galah / Magpie) append after the original eight.
pub const THEMES: [Theme; 14] = [
    TAWNY, MOPOKE, FROGMOUTH, BRACKEN,
    POTOROO, GUMTREE, BILBY, SALTPAN, QUOKKA, UNDERTOW, OUTBACK, MANGROVE, GALAH, MAGPIE,
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
/// `page::TEST_LOCK` / `fps::TEST_LOCK`.
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
/// PAGE MODE margin gradient START color of the active theme.
pub fn margin_from() -> Srgb {
    active().margin_from
}
/// PAGE MODE margin gradient END color of the active theme.
pub fn margin_to() -> Srgb {
    active().margin_to
}
/// PAGE MODE margin gradient DIRECTION of the active theme.
pub fn margin_dir() -> (f32, f32) {
    active().margin_dir
}
/// PAGE MODE margin PATTERN of the active theme.
pub fn pattern() -> BgPattern {
    active().pattern
}
/// PAGE MODE margin pattern TINT of the active theme.
pub fn pattern_color() -> Srgb {
    active().pattern_color
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worlds_eight_dark_six_light() {
        assert_eq!(THEMES.len(), 14);
        let dark = THEMES.iter().filter(|t| t.dark).count();
        let light = THEMES.iter().filter(|t| !t.dark).count();
        // 5 dark (Tawny/Potoroo/Undertow/Outback/Mangrove) + 3 warm-Tawny variants
        // (Mopoke/Frogmouth/Bracken, all dark) => 8 dark / 6 light.
        assert_eq!(dark, 8);
        assert_eq!(light, 6);
    }

    /// Every world defines a margin background: a pattern + an OPAQUE pattern tint
    /// (the shader owns the coverage, so the tint itself is fully opaque like the
    /// gradient endpoints).
    #[test]
    fn every_world_has_a_valid_background() {
        for t in THEMES.iter() {
            assert_eq!(
                t.pattern_color.a, 0xFF,
                "{} pattern_color must be opaque",
                t.name
            );
            // The shader_id round-trips to a known pattern (exhaustive match below
            // would fail to compile if a variant were added without a discriminant).
            assert!(t.pattern.shader_id() <= 3, "{} bad pattern id", t.name);
        }
        // The whole pattern palette is exercised across the worlds.
        let used: std::collections::HashSet<&str> =
            THEMES.iter().map(|t| t.pattern.as_str()).collect();
        for p in ["gradient", "dotgrid", "starfield", "pinstripe"] {
            assert!(used.contains(p), "pattern {p} unused by any world");
        }
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

    /// Every world declares a per-theme CJK (Japanese) fallback list whose
    /// CHARACTER matches the world: the SERIF worlds map to the MINCHO (serif)
    /// list, the SANS/MONO worlds to the GOTHIC (sans) list. Each list is ordered
    /// mac-primary (Hiragino) then linux-fallback (Noto) so cosmic-text picks the
    /// first the running system has.
    #[test]
    fn cjk_fallback_matches_world_character() {
        let mincho = ["Gumtree", "Saltpan", "Bilby", "Undertow", "Outback", "Magpie"];
        let gothic = ["Tawny", "Potoroo", "Mangrove", "Quokka", "Galah", "Mopoke", "Frogmouth", "Bracken"];
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
        // Priority order: macOS Hiragino first, Linux Noto second.
        assert_eq!(CJK_MINCHO, &["Hiragino Mincho ProN", "Noto Serif CJK JP"]);
        assert_eq!(CJK_GOTHIC, &["Hiragino Kaku Gothic ProN", "Noto Sans CJK JP"]);
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
    fn selection_is_the_only_translucent_token() {
        for t in THEMES.iter() {
            assert_eq!(t.base_100.a, 0xFF);
            assert_eq!(t.primary.a, 0xFF);
            assert_eq!(t.error.a, 0xFF);
            // The margin gradient endpoints are opaque (the shader owns the
            // margin opacity), so selection stays the only translucent token.
            assert_eq!(t.margin_from.a, 0xFF, "{} margin_from alpha", t.name);
            assert_eq!(t.margin_to.a, 0xFF, "{} margin_to alpha", t.name);
            assert_eq!(t.selection.a, 0x52, "{} selection alpha", t.name);
        }
    }

    /// Every world defines a NON-DEGENERATE margin gradient: the two endpoints
    /// differ (so there is a real gradient, not a flat fill) and the direction
    /// vector is non-zero (so `dot(uv, dir)` actually varies across the margin).
    #[test]
    fn every_world_has_a_real_margin_gradient() {
        for t in THEMES.iter() {
            assert_ne!(
                t.margin_from, t.margin_to,
                "{} margin gradient is degenerate (from == to)",
                t.name
            );
            let (dx, dy) = t.margin_dir;
            assert!(
                dx.abs() + dy.abs() > 0.0,
                "{} margin_dir is the zero vector",
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
