#![allow(dead_code)] // Some tokens (BASE_200, PRIMARY_CONTENT) and converters are
                     // not consumed by every surface yet — reserved for the
                     // upcoming minibuffer/panel surfaces. The per-theme `font`
                     // field is now LIVE: it drives the glyphon `Family::Name`
                     // used to shape/render the document (see render.rs).

//! src/theme/ — the palette model, split by natural seam (2026-07
//! code-organization pass) out of the former `theme.rs` monolith:
//! [`color`] (the [`Srgb`] primitive), [`model`] (the [`Theme`]/[`Background`]/
//! [`Lens`] data model), [`ornament`] (the section-break + list-bullet trios),
//! [`cjk`] (the per-script fallback ladders + [`FontId`]), [`worlds`] (the
//! fifteen concrete [`Theme`] literals), and [`derive`] (the active-theme
//! index + every derived-from-active-theme accessor). Every external path
//! (`theme::Theme`, `theme::THEMES`, `theme::CJK_MINCHO`, …) is unchanged —
//! this file only re-exports.
//!
//! Naming follows DaisyUI: base-100/200/300 are the base planes (100 = the
//! canvas; on a dark world that is the deepest plane, on a light world the
//! lightest), `*-content` is the ink that sits on a given surface, `primary` is
//! the one organic accent (the caret), `error` is the signal color, and
//! `selection` is a custom token (DaisyUI has no selection role).
//!
//! There are sixteen [`Theme`]s ("worlds"), ten dark and six light. Two are
//! DESIGN.md §3 statement worlds: Wagtail (awl's first true MONOCHROME/1-bit
//! world — zero saturation everywhere, the caret included) and Firetail (awl's
//! first LAVA-LAMP world — a slow metaball ground whose living warmth IS the
//! statement; Mangrove folds the cool second lava ground). See their own doc
//! comments in `worlds.rs` and THEMES.md's logged DESIGN.md §3 amendments. One is the
//! ACTIVE theme at any moment (an index into [`THEMES`]); the windowed app can
//! cycle it live (`C-x t` / `C-x T`) and the headless `--theme NAME` flag pins
//! it before a capture. Every color call site reads the active theme rather than
//! a fixed const, so a theme switch reskins the whole UI. Each world also names a
//! display `font`; that family is loaded at startup and selected per-frame, so a
//! theme switch reskins the GLYPH SHAPES too (mono / serif / slab / sans).

mod cjk;
mod color;
mod derive;
mod model;
mod ornament;
mod worlds;

pub use cjk::FontId;
#[allow(unused_imports)] // per-world CJK ladders: public API surface consumed by
// `theme::worlds` internally + named in doc comments crate-wide; no NON-TEST
// in-crate caller reaches them through this re-export path today.
pub use cjk::{
    ALL_FONT_IDS, CJK_GOTHIC, CJK_JA_KLEE, CJK_JA_SHIPPORI, CJK_JA_ZENMARU, CJK_KO, CJK_KO_SERIF,
    CJK_MINCHO, CJK_ZH_HANS_KLEE, CJK_ZH_HANS_SANS, CJK_ZH_HANS_SERIF, CJK_ZH_HANT,
};
pub(crate) use cjk::EMBEDDED_CJK_FAMILIES;
pub use color::Srgb;
pub use derive::{
    active, active_index, background, base_100, base_200, base_300, base_content, error, faint,
    image_reveal_scrim, muted, placard_ink, primary, selection, set_active, set_active_by_name,
    surface_selected, THEME_FACETS,
};
#[allow(unused_imports)] // cycle/overlay_scrim/primary_content/tag_for: public API
// surface, no NON-TEST in-crate caller today (tag_for's real callers all live
// under `#[cfg(test)]`).
pub use derive::{cycle, overlay_scrim, primary_content, tag_for};
pub use model::{Background, LavaEdge, Theme, WashOverride};
#[allow(unused_imports)] // Lens/RoleOverrides/ThemeTags: public API surface, no
// NON-TEST in-crate caller today.
pub use model::{Lens, RoleOverrides, ThemeTags};
// THEME CAPABILITIES AS DATA: the declarative render-behavior bundle every
// per-theme render decision reads instead of an ad hoc `is_one_bit()` branch.
// See `model::RenderCaps`'s own module doc.
#[allow(unused_imports)] // RenderCaps/ImageReveal: public API surface (the full
// bundle type + one field's enum); every non-test in-crate caller today reaches
// them through `Theme::render_caps.<field>` rather than this bare re-export.
pub use model::{
    Backdrop, CaretBlockStyle, DecorativeWash, Elevation, HighlightTexture, HighlightTreatment,
    ImageReveal, PlacardCorner, PlacardInk, RenderCaps, SelectionStyle, TitleStyle,
};
#[allow(unused_imports)] // the per-world ornament/bullet data: public API
// surface, no NON-TEST in-crate caller today.
pub use ornament::{
    Ornaments, ORNAMENTS_DEFAULT, BULLETS_PLAIN, BULLET_SCALE_ORNAMENT, BULLET_SCALE_PLAIN,
    ORNAMENT_GARAMOND, ORNAMENT_JUNICODE, ORNAMENT_MARKS, ORNAMENT_SCALE_FLEURON,
    ORNAMENT_SCALE_GEOMETRIC, ORNAMENT_SCALE_ORNATE,
};
pub use worlds::{DEFAULT_THEME, THEMES};
#[allow(unused_imports)] // the fifteen named world consts: public API surface
// (each usable individually, e.g. `theme::TAWNY.mono`); non-test code always
// reaches them through the `THEMES` array instead.
pub use worlds::{
    BILBY, CURRAWONG, FIRETAIL, GALAH, GUMTREE, KINGFISHER, MAGPIE, MANGROVE, MOPOKE, OUTBACK,
    POTOROO, QUOKKA, SALTPAN, TAWNY, UNDERTOW, WAGTAIL,
};

#[cfg(test)]
mod tests;
