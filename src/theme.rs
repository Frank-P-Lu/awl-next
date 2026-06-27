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
//! There are eight [`Theme`]s ("worlds"), four dark and four light. One is the
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
    /// Default ink drawn ON the base planes.
    pub base_content: Srgb,
    /// Muted ink for secondary text (labels, the "/" sigil, the hit counter).
    pub base_content_dim: Srgb,
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
    /// Chosen display font family for this world (recorded; glyphon switching is
    /// a follow-up — see the module note).
    pub font: &'static str,
}

// --- The eight worlds (exact hex from the theme spec) ----------------------

/// Gumtree — light eucalyptus reading room (coral caret on a cool green page).
pub const GUMTREE: Theme = Theme {
    name: "Gumtree",
    dark: false,
    base_100: Srgb::rgb(0xE4, 0xF8, 0xE2),
    base_200: Srgb::rgb(0xCF, 0xF3, 0xCC),
    base_300: Srgb::rgb(0xB7, 0xEF, 0xB4),
    base_content: Srgb::rgb(0x16, 0x24, 0x1A),
    base_content_dim: Srgb::rgb(0x5A, 0x6B, 0x57),
    primary: Srgb::rgb(0xDA, 0x52, 0x5D),
    primary_content: Srgb::rgb(0xFB, 0xEC, 0xEC),
    error: Srgb::rgb(0xC0, 0x39, 0x2B),
    selection: Srgb::rgba(0x88, 0x8F, 0x5D, 0x52),
    margin_from: Srgb::rgb(0xCF, 0xF3, 0xCC),
    margin_to: Srgb::rgb(0xB7, 0xEF, 0xB4),
    margin_dir: (0.7, 0.7),
    font: "Literata",
};

/// Potoroo — dark den-warm nocturne (raw-sienna caret in a burnt-orange room).
pub const POTOROO: Theme = Theme {
    name: "Potoroo",
    dark: true,
    base_100: Srgb::rgb(0x1F, 0x04, 0x00),
    base_200: Srgb::rgb(0x31, 0x05, 0x00),
    base_300: Srgb::rgb(0x56, 0x28, 0x00),
    base_content: Srgb::rgb(0xF0, 0xE6, 0xDE),
    base_content_dim: Srgb::rgb(0x9C, 0x85, 0x76),
    primary: Srgb::rgb(0xFE, 0xAF, 0x69),
    primary_content: Srgb::rgb(0x2A, 0x14, 0x02),
    error: Srgb::rgb(0xFF, 0x6B, 0x5C),
    selection: Srgb::rgba(0x7E, 0xB4, 0x7C, 0x52),
    margin_from: Srgb::rgb(0x1F, 0x04, 0x00),
    margin_to: Srgb::rgb(0x56, 0x28, 0x00),
    margin_dir: (0.6, 0.8),
    font: "IBM Plex Mono",
};

/// Bilby — light desert dawn (deep pyrite-gold caret on a pale-blue page).
pub const BILBY: Theme = Theme {
    name: "Bilby",
    dark: false,
    base_100: Srgb::rgb(0xE8, 0xFA, 0xFF),
    base_200: Srgb::rgb(0xCF, 0xF3, 0xFF),
    base_300: Srgb::rgb(0xB3, 0xE7, 0xFB),
    base_content: Srgb::rgb(0x10, 0x24, 0x2C),
    base_content_dim: Srgb::rgb(0x55, 0x70, 0x79),
    primary: Srgb::rgb(0xAA, 0x94, 0x34),
    primary_content: Srgb::rgb(0xFB, 0xF6, 0xE4),
    error: Srgb::rgb(0xC0, 0x39, 0x2B),
    selection: Srgb::rgba(0x5B, 0xA3, 0xC5, 0x52),
    margin_from: Srgb::rgb(0xCF, 0xF3, 0xFF),
    margin_to: Srgb::rgb(0xB3, 0xE7, 0xFB),
    margin_dir: (0.7, 0.7),
    // Newsreader registers under this exact fontdb family name (it ships as the
    // "16pt" optical-size master), so `Family::Name` must match it verbatim.
    font: "Newsreader 16pt 16pt",
};

/// Saltpan — light sun-bleached salt flat (cinnamon-clay caret on warm ecru).
pub const SALTPAN: Theme = Theme {
    name: "Saltpan",
    dark: false,
    base_100: Srgb::rgb(0xFF, 0xFD, 0xF2),
    base_200: Srgb::rgb(0xFB, 0xF3, 0xDE),
    base_300: Srgb::rgb(0xF2, 0xE6, 0xC7),
    base_content: Srgb::rgb(0x24, 0x1D, 0x12),
    base_content_dim: Srgb::rgb(0x7A, 0x6E, 0x55),
    primary: Srgb::rgb(0x8D, 0x59, 0x25),
    primary_content: Srgb::rgb(0xFB, 0xF1, 0xE6),
    error: Srgb::rgb(0xB5, 0x45, 0x2B),
    selection: Srgb::rgba(0xA5, 0x86, 0x50, 0x52),
    margin_from: Srgb::rgb(0xFB, 0xF3, 0xDE),
    margin_to: Srgb::rgb(0xF2, 0xE6, 0xC7),
    margin_dir: (0.7, 0.7),
    font: "Literata",
};

/// Quokka — light cheerful reef (teal caret cooling a warm peach page).
pub const QUOKKA: Theme = Theme {
    name: "Quokka",
    dark: false,
    base_100: Srgb::rgb(0xFF, 0xEA, 0xDD),
    base_200: Srgb::rgb(0xFF, 0xDF, 0xCF),
    base_300: Srgb::rgb(0xFF, 0xD2, 0xBD),
    base_content: Srgb::rgb(0x2B, 0x18, 0x10),
    base_content_dim: Srgb::rgb(0x8A, 0x64, 0x53),
    primary: Srgb::rgb(0x07, 0x70, 0x73),
    primary_content: Srgb::rgb(0xE6, 0xF6, 0xF6),
    error: Srgb::rgb(0xC0, 0x39, 0x2B),
    selection: Srgb::rgba(0xBB, 0x80, 0x20, 0x52),
    margin_from: Srgb::rgb(0xFF, 0xDF, 0xCF),
    margin_to: Srgb::rgb(0xFF, 0xD2, 0xBD),
    margin_dir: (0.7, 0.7),
    font: "IBM Plex Sans",
};

/// Undertow — dark deep midnight current (hot indian-lake caret in violet dark).
pub const UNDERTOW: Theme = Theme {
    name: "Undertow",
    dark: true,
    base_100: Srgb::rgb(0x15, 0x0A, 0x2C),
    base_200: Srgb::rgb(0x24, 0x15, 0x40),
    base_300: Srgb::rgb(0x3C, 0x36, 0x54),
    base_content: Srgb::rgb(0xEC, 0xE8, 0xF2),
    base_content_dim: Srgb::rgb(0x8A, 0x7F, 0xA8),
    primary: Srgb::rgb(0xC5, 0x3C, 0x69),
    primary_content: Srgb::rgb(0x2A, 0x0A, 0x16),
    error: Srgb::rgb(0xFF, 0x6B, 0x5C),
    selection: Srgb::rgba(0x4F, 0x40, 0x86, 0x52),
    margin_from: Srgb::rgb(0x15, 0x0A, 0x2C),
    margin_to: Srgb::rgb(0x24, 0x15, 0x40),
    margin_dir: (0.0, 1.0),
    // See BILBY: Newsreader's exact registered family name.
    font: "Newsreader 16pt 16pt",
};

/// Outback — dark red-centre night (hays-russet caret in blackish-olive room).
pub const OUTBACK: Theme = Theme {
    name: "Outback",
    dark: true,
    base_100: Srgb::rgb(0x16, 0x1D, 0x14),
    base_200: Srgb::rgb(0x1E, 0x27, 0x1C),
    base_300: Srgb::rgb(0x3F, 0x49, 0x3C),
    base_content: Srgb::rgb(0xEC, 0xEA, 0xE0),
    base_content_dim: Srgb::rgb(0x8A, 0x8C, 0x78),
    primary: Srgb::rgb(0xDE, 0x8E, 0x7F),
    primary_content: Srgb::rgb(0x2A, 0x14, 0x10),
    error: Srgb::rgb(0xFF, 0x6B, 0x5C),
    selection: Srgb::rgba(0xFF, 0xEF, 0xAE, 0x52),
    margin_from: Srgb::rgb(0x16, 0x1D, 0x14),
    margin_to: Srgb::rgb(0x1E, 0x27, 0x1C),
    margin_dir: (0.0, 1.0),
    font: "Zilla Slab",
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
    base_content_dim: Srgb::rgb(0x8B, 0x91, 0x9D),
    primary: Srgb::rgb(0xFF, 0xC0, 0x5E),
    primary_content: Srgb::rgb(0x26, 0x1A, 0x08),
    error: Srgb::rgb(0xE5, 0x4B, 0x4B),
    selection: Srgb::rgba(0x3A, 0x6F, 0xD8, 0x52),
    margin_from: Srgb::rgb(0x16, 0x18, 0x1D),
    margin_to: Srgb::rgb(0x20, 0x22, 0x28),
    margin_dir: (0.0, 1.0),
    font: "IBM Plex Mono",
};

/// All eight worlds, in cycle order. `C-x t` advances through this list and
/// wraps; `C-x T` steps backward. The DEFAULT (index 0) is Tawny: a quiet
/// warm-grey dark world whose display font is the original bundled IBM Plex
/// Mono, so the app opens on awl's familiar mono "home" look.
pub const THEMES: [Theme; 8] = [
    TAWNY, POTOROO, GUMTREE, BILBY, SALTPAN, QUOKKA, UNDERTOW, OUTBACK,
];

/// Index into [`THEMES`] of the default/startup world. Tawny (a dark, warm-grey
/// world drawn in IBM Plex Mono) is awl's "home" look, so the app opens on the
/// familiar mono world; the proportional worlds are one theme-cycle away.
pub const DEFAULT_THEME: usize = 0;

/// The active theme index. A process-global so every render call site reads the
/// same world without threading a `&Theme` through the whole pipeline. The
/// windowed app cycles it (`C-x t`); `--theme NAME` pins it for a capture.
static ACTIVE: AtomicUsize = AtomicUsize::new(DEFAULT_THEME);

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
/// Muted ink of the active theme.
pub fn base_content_dim() -> Srgb {
    active().base_content_dim
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The active theme is a process-global; the two tests that MUTATE it must not
    /// run concurrently (cargo runs tests in parallel). Serialize them on a shared
    /// lock so each sees a clean starting state.
    static ACTIVE_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn eight_worlds_four_dark_four_light() {
        assert_eq!(THEMES.len(), 8);
        let dark = THEMES.iter().filter(|t| t.dark).count();
        let light = THEMES.iter().filter(|t| !t.dark).count();
        assert_eq!(dark, 4);
        assert_eq!(light, 4);
    }

    #[test]
    fn default_is_dark() {
        assert!(THEMES[DEFAULT_THEME].dark);
        assert_eq!(THEMES[DEFAULT_THEME].name, "Tawny");
    }

    #[test]
    fn cycle_wraps_both_ways() {
        let _g = ACTIVE_LOCK.lock().unwrap();
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
        let _g = ACTIVE_LOCK.lock().unwrap();
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

    /// The eight worlds map onto at least four CLEARLY-distinct display faces
    /// (mono / serif / serif / sans / slab), so cycling worlds visibly reskins
    /// the glyph shapes, not just the palette.
    #[test]
    fn at_least_four_distinct_faces() {
        let mut faces: Vec<&str> = THEMES.iter().map(|t| t.font).collect();
        faces.sort_unstable();
        faces.dedup();
        assert!(
            faces.len() >= 4,
            "expected >=4 distinct display faces, got {faces:?}"
        );
        // Home (Tawny) renders in the bundled mono so it looks exactly like home.
        assert_eq!(TAWNY.font, "IBM Plex Mono");
    }
}
