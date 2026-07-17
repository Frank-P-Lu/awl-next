//! src/theme/derive.rs — the ACTIVE-THEME accessors: the process-global index,
//! the cycle/set/lookup functions, and every DERIVED-from-active-theme token
//! (surface_selected, the scrims, `background()`, `tag_for`) plus the theme
//! picker's generic [`FacetScheme`] bridge. See [`crate::theme::worlds`] for
//! the concrete [`Theme`] data these read.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::color::Srgb;
use super::model::{Background, ImageReveal, Lens, Theme};
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

/// How far a DARK world's `Faint` placard rung steps up the ink ladder, from
/// `faint` toward `muted` — the personality-assignment round's DARK-GROUND
/// CONTRAST correction (the probe gallery's light-world Ghost was
/// gallery-validated, but the same formulas were near-invisible on the dark
/// grounds — Undertow's Ghost vanished; the user's taste note demanded the
/// wordmark "clearly READ" there while staying a receding ghost). ONE global
/// constant per rung, never a per-world hand value. Blending toward `muted`
/// — the next rung UP the same ladder — rather than all the way toward
/// `base_content` makes the "legible ghost, not a competing headline"
/// ceiling hold BY CONSTRUCTION (a `faint`→`muted` lerp can never outshine
/// `muted`, the very ink the card's own rows read in; the first
/// toward-`base_content` draft overshot it on Potoroo). Floor + ordering +
/// ceiling are law-tested by `theme::tests::
/// placard_inks_read_on_dark_grounds_and_stay_below_muted`.
const PLACARD_DARK_LIFT_FAINT: f32 = 0.75;
/// The `Ghost` sibling of [`PLACARD_DARK_LIFT_FAINT`]: a shorter step up the
/// same ladder, so `Ghost` stays the quieter rung on dark grounds exactly as
/// it is on light ones (presence ordering is law-tested).
const PLACARD_DARK_LIFT_GHOST: f32 = 0.45;

/// THE ONE owner of a [`super::model::PlacardInk`] rung's color — always a
/// pure blend of tokens already on the active world's own ink ladder, never
/// a free color (see [`super::model::PlacardInk`]'s own doc for why the enum
/// has no raw-`Srgb` variant to smuggle one in through), and MODE-AWARE
/// since the personality-assignment round:
///
/// - **LIGHT grounds** keep the gallery-validated originals: `Faint` is the
///   [`faint`] rung verbatim; `Ghost` steps HALFWAY further from `faint`
///   toward `base_300` — barely-there, the P3R watermark read.
/// - **DARK grounds** step the OTHER way — from `faint` UP toward [`muted`]
///   — because on a dark world `faint` already sits close to the ground and
///   the light formulas rendered near-invisible (the user's dark-ground
///   taste note; Undertow's Ghost was the exhibit). One formula off the
///   ladder ([`PLACARD_DARK_LIFT_FAINT`]/[`_GHOST`]), never a per-world
///   constant; the result recedes behind the rows BY CONSTRUCTION (a
///   `faint`→`muted` blend cannot outshine `muted`, the rows' own ink).
///
/// `Stipple` draws INDIVIDUAL PIXELS in the full-ink `base_content` rung
/// (perceived tone is carried by DENSITY instead — see
/// [`placard_stipple_density`], its partner owner), so this returns
/// `base_content` for it: the pixel color half of the stipple pair.
///
/// The FIRETAIL-MAXIMALIST-SHOWCASE round's DIAL-UP rungs sit ABOVE `Faint`
/// on the same ladder, mode-INDEPENDENT (they name absolute ladder positions,
/// not ground-relative corrections — `muted` already carries each world's own
/// contrast): `Muted` is the [`muted`] rung verbatim (the rows' own ink);
/// `Bold` steps [`PLACARD_BOLD_LIFT`] further from `muted` toward
/// [`base_content`] — a clear statement that stays under full ink BY
/// CONSTRUCTION (a `muted`→`base_content` lerp at < 1.0 can never reach the
/// rows' brightest ink). Still never a free color, still never dithered.
pub fn placard_ink(ink: super::model::PlacardInk) -> Srgb {
    let t = active();
    match ink {
        super::model::PlacardInk::Faint if t.dark => {
            faint().lerp(muted(), PLACARD_DARK_LIFT_FAINT)
        }
        super::model::PlacardInk::Ghost if t.dark => {
            faint().lerp(muted(), PLACARD_DARK_LIFT_GHOST)
        }
        super::model::PlacardInk::Faint => faint(),
        super::model::PlacardInk::Ghost => faint().lerp(base_300(), 0.5),
        super::model::PlacardInk::Stipple => base_content(),
        super::model::PlacardInk::Muted => muted(),
        super::model::PlacardInk::Bold => muted().lerp(base_content(), PLACARD_BOLD_LIFT),
    }
}

/// How far the `Bold` dial-up rung steps from `muted` toward `base_content`
/// — one global constant, never a per-world hand value (the same discipline
/// as [`PLACARD_DARK_LIFT_FAINT`]). Halfway reads as a clear statement while
/// structurally staying below the full-ink rows (law-tested by
/// `theme::tests::dialup_placard_inks_stay_on_the_ladder_below_full_ink`).
const PLACARD_BOLD_LIFT: f32 = 0.5;

/// Gamma-correct Rec.709 relative luminance — the same recipe the law tests
/// use (`theme::tests`' `rel_lum`, `render::tests::syntax_roles`'
/// `rel_luminance`), needed at RUNTIME here because the stipple density is a
/// perceptual-tone formula, not a channel blend.
fn rel_lum(c: Srgb) -> f32 {
    fn lin(u: u8) -> f32 {
        let s = u as f32 / 255.0;
        if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    }
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// The floor/ceiling a stipple placard's DENSITY may occupy — below the floor
/// too few pixels survive to read as letterforms at all (the legibility floor
/// the dark-ground taste note demands, asserted over Mangrove's lava ground by
/// `theme::tests::stipple_placard_density_clears_the_legibility_floor_over_
/// its_own_ground`); above the ceiling the mark stops being a ghost and
/// reads as solid text.
const PLACARD_STIPPLE_DENSITY_FLOOR: f32 = 0.12;
const PLACARD_STIPPLE_DENSITY_CEILING: f32 = 0.55;

/// THE ONE owner of the stipple placard's DENSITY — the fraction of wordmark
/// pixels that draw (each in the pure [`placard_ink`]`(Stipple)` ink =
/// `base_content`, fully opaque; the Bayer matrix decides WHICH — see
/// `render::dither`). Derived, never authored per world: the density is
/// chosen so the stipple's MEAN tone over the ground matches the world's own
/// strengthened `Faint` placard rung —
/// `density = (Y(faint_rung) - Y(ground)) / (Y(ink) - Y(ground))` in
/// relative luminance — i.e. "reads at roughly Faint tone from reading
/// distance", the same ladder-derived loudness every other placard ink
/// speaks, clamped to the floor/ceiling band above. (Mangrove, the first
/// assignment, lands ≈0.24 — the same neighborhood as Wagtail's 0.25
/// highlight stipple, a reassuring convergence of two independent
/// derivations.)
pub fn placard_stipple_density() -> f32 {
    let ground = rel_lum(base_100());
    let ink = rel_lum(base_content());
    let target = rel_lum(placard_ink(super::model::PlacardInk::Faint));
    let span = ink - ground;
    let density = if span.abs() < 1e-6 { 0.0 } else { (target - ground) / span };
    density.clamp(PLACARD_STIPPLE_DENSITY_FLOOR, PLACARD_STIPPLE_DENSITY_CEILING)
}

/// THE ONE owner of the PAGE-FRAME ink ([`super::model::PageFrame`], the
/// writing-column frame capability): the world's own `base_content` — the
/// full-ink ladder rung, never a free color and never the amber accent. The
/// WORLD-ROLES "dark-line page-frame" idea IS full ink (a dark line on a
/// light world; on Wagtail, the first assignment, this is its ladder's pure
/// white). Weight lives on the capability; ink derivation lives here, so a
/// frame can never invent a color (law-tested).
pub fn page_frame_ink() -> Srgb {
    base_content()
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

/// EXTRA surface-ramp increments the PICKER'S selected ROW sits past the shared
/// [`surface_selected`] band — the PALETTE-COMPOSITION round's "clearer-but-calm
/// selected row", strengthened by VALUE ALONE (never a hue, never the amber
/// accent — DESIGN §3/§5; the distinguishability sweep is the law that polices
/// it). The shared `surface_selected` (steps `2`) still drives the HUD /
/// word-count / menu-drop borders untouched; ONLY the overlay's selected-row
/// band ([`overlay_selected_band`]) climbs this one further increment, so the
/// row it marks reads a touch more present without the borders moving with it.
/// TASTE DEFAULT — `1` is the calm pick; the gallery A/Bs it against the old
/// band (steps `2`) via `AWL_OVERLAY_SELROW_FORCE`, and the revert to the old
/// band is one line at the `overlay_draw_card` call site (or `0` here).
pub(super) const OVERLAY_SELROW_EXTRA_STEPS: i32 = 1;

/// The shared selected/border band: `base_300` stepped `SELECTED_BAND_STEPS`
/// ramp increments past itself. Split from [`surface_selected`] so the overlay
/// row's stronger band ([`overlay_selected_band`]) reuses the SAME step math
/// with one more increment — one owner, no drift, both value-only.
fn surface_step_band(extra_steps: i32) -> Srgb {
    let a = active();
    if a.base_200 == a.base_300 {
        // A COLLAPSED surface ramp (Wagtail's 1-bit ladder) — see
        // `surface_selected`'s own doc; the ink pole is the only rung left.
        // (Wagtail's overlay row uses `SelectionStyle::InverseVideo`, so this
        // band color is never actually drawn there either way.)
        return a.base_content;
    }
    let steps = SELECTED_BAND_STEPS + extra_steps;
    let step = |lo: u8, hi: u8| -> u8 {
        let d = hi as i32 - lo as i32;
        (hi as i32 + d * steps).clamp(0, 255) as u8
    };
    Srgb::rgb(
        step(a.base_200.r, a.base_300.r),
        step(a.base_200.g, a.base_300.g),
        step(a.base_200.b, a.base_300.b),
    )
}

/// ARM B LIVING-BAND PROBE — the BRIGHTEST value step the two-shape
/// choreography fills WHERE its leading band and chasing echo cross
/// ([`crate::render::livingband`]). ONE ladder step past the selected-row band
/// (which the leading band already wears), on the SAME surface ramp — so the
/// crossing reads exactly one calm step brighter than the lead (echo `+0`, lead
/// `+1`, crossing `+2` past `surface_selected`: a clean monotone value climb),
/// colour where they cross by VALUE only, never a hue / never amber (DESIGN §3).
/// Consumed only when `AWL_OVERLAY_MOTION_FORCE=twoshape…` is set; inert on
/// every ordinary run.
pub fn overlay_band_overlap() -> Srgb {
    surface_step_band(OVERLAY_SELROW_EXTRA_STEPS + 1)
}

/// The PICKER'S selected-row VALUE band — [`surface_selected`] climbed
/// [`OVERLAY_SELROW_EXTRA_STEPS`] further up the SAME surface ramp (value only,
/// never a hue). The `overlay_draw_card` band reads this; the shared borders
/// keep `surface_selected`. See [`OVERLAY_SELROW_EXTRA_STEPS`] for the A/B.
pub fn overlay_selected_band() -> Srgb {
    surface_step_band(OVERLAY_SELROW_EXTRA_STEPS)
}

/// WCAG relative-contrast ratio `(L_hi + 0.05) / (L_lo + 0.05)` between two
/// opaque colors, on the same gamma-correct [`rel_lum`] the law tests use.
/// Needed at RUNTIME to pick the selected-row ink that actually reads on its
/// own value band.
fn contrast_ratio(a: Srgb, b: Srgb) -> f32 {
    let (la, lb) = (rel_lum(a), rel_lum(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// The minimum contrast the selected picker row's INK must clear against its
/// own value band ([`overlay_selected_band`]) — the taste floor enforced for
/// EVERY world by `render::tests::distinguishability::
/// selected_row_text_clears_contrast_floor_on_every_world`. 3:1 is the WCAG
/// large-text / UI-component floor; below it the glyphs wash into the fill (the
/// Undertow-under-Bars exhibit: light ink on a mid sage band = 2.53:1).
pub(super) const SELECTED_ROW_INK_CONTRAST_FLOOR: f32 = 3.0;

/// THE ONE owner of the selected picker row's INK on a `ValueBand` world — the
/// [`super::HighlightTreatment::InverseFill`] lesson (a selected row that erases
/// its own text is the bug) applied to the ORDINARY-fill worlds. The row keeps
/// the world's `base_content` ink UNLESS the selected-row value `band` washes it
/// out (contrast below [`SELECTED_ROW_INK_CONTRAST_FLOOR`]), in which case the
/// ink FLIPS to whichever ladder POLE (`base_100` ground vs `base_content` ink)
/// reads harder against the fill. Derived purely from the fill's own luminance,
/// never a per-world hand value: on a DARK world the light `base_content` fails
/// against a mid band and the dark ground wins; on a LIGHT world the reverse.
/// Undertow under Bars was the exhibit — light ink (236,232,242) on a mid sage
/// band (132,152,144) = 2.53:1. Wagtail's 1-bit worlds resolve their pair
/// through `InverseFill` instead and never reach here.
pub fn selected_row_ink(band: Srgb) -> Srgb {
    let content = base_content();
    if contrast_ratio(band, content) >= SELECTED_ROW_INK_CONTRAST_FLOOR {
        return content;
    }
    let ground = base_100();
    if contrast_ratio(band, ground) > contrast_ratio(band, content) {
        ground
    } else {
        content
    }
}

/// PER-ITEM LIST SURFACES round — the UNSELECTED bar's fill under
/// [`super::ListStyle::Bars`]. A WHISPER: the `base_200` code-fence-wash
/// register — one gentle value step off the GROUND (`base_100`), near-invisible
/// rhythm rather than a slab. The user's verdict on the first cut (unselected ==
/// `surface_step_band(-1)`, a saturated rung one step below the card) was "a
/// picket fence where every row shouts": with no card behind the bars (the Bars
/// treatment drops the pane — see `overlay_draw_card`), the ground IS the scrim,
/// and the unselected bar should barely lift off it so the SELECTED bar's strong
/// `overlay_selected_band` pop has somewhere to go. Value only — never a hue,
/// never the amber accent (DESIGN §3/§5). On a collapsed 1-bit ramp `base_200 ==
/// base_100` (invisible), but Wagtail ships `Pane` + draws its selected row via
/// `InverseFill`, so this fill is inert there anyway.
pub fn overlay_bar_unselected() -> Srgb {
    base_200()
}

/// PER-ITEM LIST SURFACES round — the ROOM PLANE laid full-canvas behind the
/// bars under [`super::ListStyle::Bars`]. The world's own OPAQUE ground plane
/// (`base_100` — the paper): a uniform field, never a bordered box (no shadow,
/// no border, no bright `base_300` fill), so the bars read as floating ON the
/// room rather than IN a card — the user's "with the bars, there shouldn't be a
/// pane!" honoured. OPAQUE (not a translucent veil) for two reasons the designer
/// pixel-pass proved: (1) a translucent scrim let the crisp document's page
/// margin bleed through every gap, so the comb SEAM survived at reduced alpha; a
/// solid plane erases it outright; (2) the unselected bar is a WHISPER one value
/// step off the ground (`base_200`) — a translucent veil pulls the gap toward
/// `base_200` too and COLLAPSES the whisper (invisible on light worlds), while
/// the solid paper keeps the `base_100 → base_200` step the whisper needs to
/// read. The trade — the document preview no longer ghosts behind the bars — is
/// right for a bars world (a maximalist statement room, not a peek). Value only,
/// never a hue (DESIGN §3/§5).
pub fn overlay_bars_room() -> Srgb {
    base_100()
}

pub fn surface_selected() -> Srgb {
    // The shared band: `base_300` stepped `SELECTED_BAND_STEPS` further up the
    // surface ramp, in the SAME direction `base_200 -> base_300` carries (toward
    // the ink on dark worlds, toward the ground on light). A COLLAPSED ramp
    // (Wagtail's 1-bit ladder: base_200 == base_300 == pure black) has no
    // direction to move in — `surface_step_band` returns the ink pole
    // (`base_content`, pure white on Wagtail: "a white 1px border on a black
    // card is 1-bit-legal") so every float/HUD/whichkey/menu-drop border AND
    // the picker's selected-row band stays visible. Keyed on the RAMP SHAPE, not
    // `Elevation::Bordered`: Currawong/Mangrove/Firetail carry `Bordered` as
    // functional elevation yet keep their ordinary ramp-step band (returning
    // white there would fill the selected row the same value as its own text —
    // the Wagtail invisible-row bug class).
    surface_step_band(0)
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

/// The section a world (by case-sensitive NAME) sits in under `lens` — a THEME-AXIS
/// coordinate. `None` when the world OPTS OUT of the axis, for an unknown name (never
/// panics), or for [`Lens::All`] (which does not group).
///
/// The runtime lens strip that once GROUPED the theme picker by these axes was
/// retired (user decision, 2026-07-15 — the theme picker is now a flat browsable
/// list, see [`crate::facets`]'s module doc); the [`Lens`] axes + per-world
/// [`super::model::ThemeTags`] survive purely as the BUILD-TIME coverage ruler asserted by
/// [`tests::axis_coverage_ruler`]. `tag_for` is that ruler's name-keyed accessor.
pub fn tag_for(name: &str, lens: Lens) -> Option<&'static str> {
    THEMES
        .iter()
        .find(|t| t.name == name)
        .and_then(|t| t.tags.section(lens))
}
