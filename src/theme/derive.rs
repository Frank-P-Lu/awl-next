//! src/theme/derive.rs — the ACTIVE-THEME accessors: the process-global index,
//! the cycle/set/lookup functions, and every DERIVED-from-active-theme token
//! (surface_selected, the scrims, `background()`, `tag_for`) plus the theme
//! picker's generic [`FacetScheme`] bridge. See [`crate::theme::worlds`] for
//! the concrete [`Theme`] data these read.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::color::Srgb;
use super::model::{Background, Elevation, ImageReveal, Lens, Theme};
use super::worlds::{DEFAULT_THEME, THEMES};

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
pub(super) const SELECTED_BAND_STEPS: i32 = 2;

pub fn surface_selected() -> Srgb {
    let a = active();
    if a.render_caps.elevation == Elevation::Bordered {
        // A true 1-bit world's elevation ladder collapses to a strict binary:
        // the CARD FILL stays the ground value (`base_300 == base_100` ==
        // black, so ink text drawn on it stays legible) and this BORDER-only
        // token reads pure white instead — "a white 1px border on a black
        // card is 1-bit-legal" (`worlds.rs::WAGTAIL`'s doc comment). The
        // ordinary base_200->base_300 step math below would just collapse to
        // black too (both endpoints equal on a one-bit world), which would
        // make every float/HUD/whichkey/menu-drop panel's border invisible —
        // so this is a DECLARED override, not a tuning of the same formula.
        return Srgb::rgb(0xFF, 0xFF, 0xFF);
    }
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

/// Alpha of the INLINE-IMAGE CAPTION SCRIM (`image_reveal_scrim`) — the soft band of
/// the world's own GROUND laid behind a revealed image's source text so the caption
/// reads over the dimmed image. A touch more opaque than the doc scrim (~0.72): the
/// scrim is the SAME ground the doc sits on, so ground-over-ground is INVISIBLE where
/// the caption clears the image, and this alpha only bites where the text overlaps the
/// image pixels. TASTE TUNABLE — flagged for live review, judged over a dark + a light
/// world (the `render/layers.rs` `IMAGE_REVEAL_DIM_ALPHA` is its partner lever).
const IMAGE_REVEAL_SCRIM_ALPHA: u8 = 0xB8;

/// Translucent CAPTION SCRIM behind a revealed inline image's source text: the canvas
/// plane (`base_100`) at part alpha, so the centred caption reads over the dimmed
/// image. A value step toward the ground, never a new hue — so amber stays the
/// caret's alone (DESIGN §3). Re-tinted per world (geometry is theme-independent).
pub fn image_reveal_scrim() -> Srgb {
    let b = active().base_100;
    if active().render_caps.image_reveal == ImageReveal::Opaque {
        // A translucent veil over an image would composite a forbidden grey
        // on a true 1-bit world — opaque ground instead (the reveal fully
        // occludes the image rather than dimming it). Unaudited beyond this:
        // images are already PHILOSOPHY.md's own logged palette exception, so
        // this narrow follow-on trade is consistent with that existing call,
        // not a new one.
        return Srgb::rgba(b.r, b.g, b.b, 0xFF);
    }
    Srgb::rgba(b.r, b.g, b.b, IMAGE_REVEAL_SCRIM_ALPHA)
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
pub(super) const THEME_FACET_STRIP: [Facet; 5] = [
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
pub(super) fn theme_bucket(item: FacetItem, lens_idx: usize) -> Option<&'static str> {
    // The theme picker is a STRING-ONLY bucket: it reads only the world name, never
    // the dir/git flags (both always `false` for a world).
    Lens::STRIP.get(lens_idx).and_then(|l| tag_for(item.accept, *l))
}

/// The theme picker's registered [`FacetScheme`], consulted by
/// [`crate::facets::scheme`] (its one call site) and, through that, the overlay
/// state / renderer / sidecar — all picker-agnostic.
pub static THEME_FACETS: FacetScheme =
    FacetScheme { strip: &THEME_FACET_STRIP, bucket: theme_bucket };
