#![allow(dead_code)] // 3 new tokens (BASE_200/300, PRIMARY_CONTENT) and some
                     // converters are not consumed yet — reserved for the
                     // upcoming minibuffer/panel surfaces.

//! src/theme.rs — single source of truth for the palette.
//!
//! Naming follows DaisyUI: base-100/200/300 are the dark planes (100 = deepest),
//! `*-content` is the ink that sits on a given surface, `primary` is the brand
//! accent, `error` is the signal color, and `selection` is a custom token (DaisyUI
//! has no selection role).

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
    /// reproduces the old BG floats {0.086,0.094,0.114,1.0} exactly, because
    /// those were 22/24/29/255 to begin with. Needs f64 (wgpu::Color is f64).
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
}

// --- Tokens (DaisyUI naming; "-content" = the ink that sits on this) ---

/// App background / deepest base plane (render-pass clear color).
pub const BASE_100: Srgb = Srgb::rgb(0x16, 0x18, 0x1D);
/// Raised surface, one perceptual step lighter than base-100.
pub const BASE_200: Srgb = Srgb::rgb(0x20, 0x22, 0x28);
/// Focused plane / border, one more step lighter than base-200.
pub const BASE_300: Srgb = Srgb::rgb(0x2A, 0x2D, 0x34);
/// Default ink drawn ON the base planes (-content = ink on base).
pub const BASE_CONTENT: Srgb = Srgb::rgb(0xE6, 0xE6, 0xE6);
/// Muted ink for secondary text — labels, the search "/" sigil, the hit counter.
/// A cool grey, clearly dimmer than BASE_CONTENT but legible on the base planes.
pub const BASE_CONTENT_DIM: Srgb = Srgb::rgb(0x8B, 0x91, 0x9D);
/// Brand accent (warm amber): caret hue and amber surfaces.
pub const PRIMARY: Srgb = Srgb::rgb(0xFF, 0xC0, 0x5E);
/// Dark ink drawn ON the amber primary (-content = ink on primary).
pub const PRIMARY_CONTENT: Srgb = Srgb::rgb(0x26, 0x1A, 0x08);
/// Error / spell-squiggle signal color (soft red).
pub const ERROR: Srgb = Srgb::rgb(0xE5, 0x4B, 0x4B);
/// Text-selection highlight (steel-blue, ~0.32 alpha). Custom token.
pub const SELECTION: Srgb = Srgb::rgba(0x3A, 0x6F, 0xD8, 0x52);
