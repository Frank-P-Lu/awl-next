//! src/theme/model.rs — the core palette DATA MODEL: [`Theme`] itself (the
//! per-world struct), its [`Background`] margin-ground union, the syntax
//! [`RoleOverrides`] escape hatch, and the theme-picker's [`Lens`]/[`ThemeTags`]
//! faceting types. See [`crate::theme::worlds`] for the sixteen concrete
//! [`Theme`] literals and [`crate::theme::derive`] for the active-theme
//! accessors that read them.

use super::cjk::FontId;
use super::color::Srgb;
use super::ornament::Ornaments;

/// PER-WORLD SYNTAX ROLE-STYLE OVERRIDES — the designed escape hatch for the
/// DERIVED role tints + washes (`render/spans.rs::role_style_for`, the one owner
/// of role color). FIFTEEN of the sixteen worlds ship [`RoleOverrides::NONE`]:
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
    /// fifteen of the sixteen worlds ship with (Wagtail is the exception —
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
// `ROADMAP.md`'s "theme capabilities as data" entry). The machinery landed
// dormant (all sixteen worlds on [`RenderCaps::DEFAULT`] except Wagtail);
// the PERSONALITY-ASSIGNMENT round proved the design by assigning fields as
// one-line DATA edits — placards on Galah/Magpie/Mangrove/Firetail, bordered
// elevation on Currawong/Mangrove/Firetail, the Wagtail page frame — with no
// world-name string comparison, no `is_one_bit()` read, anywhere in
// `src/render/**` (a structural law test, `render::tests::theme_caps_law`,
// bans both from ever reappearing there). The per-world assignment table is
// itself law-pinned: `theme::tests::personality_assignments_are_exactly_the_
// decided_table`.
///
/// Whether document SELECTION paints as the ordinary translucent `selection`
/// fill, or as TRUE inverse video (`SelectionPipeline::new_invert`, an
/// `OneMinusDst` blend drawn after text) — the only mechanism that can render
/// "selected" on a world with no intermediate grey to fill with. See
/// `TextPipeline::selection_invert`'s field doc + `prepare_selection_layer`.
/// The SAME field also drives every OTHER "highlight a row/band" surface
/// that faces the identical constraint: the picker's selected-row value band
/// (`overlay_rows`, `render/chrome/overlay.rs`) and the web/Linux menu bar's
/// open-title band (`menubar_hi`, `render/chrome/menubar.rs`) — a picker row
/// or an open menu title is, in this renderer's terms, just another "selected"
/// region; a value-band fill has the same "no legal intermediate grey" problem
/// document selection does on a one-bit world. For those SMALL-TEXT surfaces
/// the answer is [`HighlightTreatment::InverseFill`]: a SOLID `base_content`
/// band with the selected row's own glyphs recolored to solid `base_300`
/// (black on white) — NOT the framebuffer invert document selection still
/// uses, whose gamma-limited flip of the antialiased row text read as a faint
/// grey (see that variant's doc).
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
/// alone, and the blur/scrim backdrop supplies the card's contrast), or
/// additionally draws the crisp raised BORDER rim + drop shadow the float
/// panels already carry (`set_float_quads`'s `elevated` arm, border ink =
/// `surface_selected()`). Wagtail's original motivation: its surface ramp
/// COLLAPSES (`base_200 == base_300`, both pure black) and its backdrop blur
/// is disabled, so a flat fill was an invisible card — `surface_selected()`
/// detects that collapsed ramp and returns the ink pole (pure white there).
/// The personality-assignment round widened `Bordered` to three ORDINARY
/// worlds as functional elevation — Currawong (OLED true-black swallows the
/// drop shadow; the rim carries the edge) and the two lava worlds Mangrove /
/// Firetail (the card must hold an edge over a moving ground) — whose
/// selected-row band and border rim keep the ordinary ramp-step derivation
/// (the picker's selected-row band is gated by `SelectionStyle`, never by
/// this field). See `surface_selected()`, `prepare_panel_card_elevation`.
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

/// Which CORNER of the summoned overlay card a [`TitleStyle::Placard`]
/// wordmark anchors to — see [`TitleStyle`]'s own module doc for the
/// mechanism. `TL`/`TR` sit level with the query line; `BL`/`BR` sit level
/// with the card's foot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacardCorner {
    TL,
    TR,
    BL,
    BR,
}

/// The ink a [`TitleStyle::Placard`] wordmark draws in — always DERIVED from
/// the world's own ink ladder (never a free color; see [`super::derive::placard_ink`],
/// the one owner — MODE-AWARE since the personality-assignment round: on a
/// LIGHT ground `Faint` is the faintest ladder rung verbatim and `Ghost`
/// steps further toward the card's own ground (`base_300`) — the
/// gallery-validated barely-there P3R "watermark" read; on a DARK ground both
/// rungs instead step UP the ladder toward `base_content`, because the
/// gallery's dark-world shots proved the light formulas near-invisible there
/// — Undertow's Ghost vanished outright. One formula off the ladder per
/// mode, never a per-world hand constant; see `placard_ink`'s own doc).
/// `Stipple` is the personality-assignment round's texture variant: the SAME
/// wordmark rendered as a Bayer-matrix STIPPLE of individual full-ink pixels
/// (`base_content`, fully opaque or absent — never a fractional alpha) at a
/// density derived so the mark reads at roughly the `Faint` rung's tone from
/// reading distance (`super::derive::placard_stipple_density`, the density's
/// one owner). It reuses the existing ordered-dither pattern language
/// (`render::dither::BAYER8`, `shaders/selection.wgsl`'s dither branch — the
/// same matrix Wagtail's highlight texture and Mangrove's lava grain speak),
/// never a second pattern. No variant carries a raw `Srgb` — the enum itself
/// makes "invent a placard color" unrepresentable, mirroring
/// [`HighlightTreatment`]'s own no-absent-variant discipline.
///
/// **A ONE-BIT world's own law still applies on top of this ladder** (see
/// `Theme::is_one_bit`'s doc): a true 1-bit world may draw ONLY pure black or
/// pure white, so neither `Faint` nor `Ghost` (both ordinary greys on every
/// world today, and antialiased glyph renders besides) is a legal choice
/// THERE. `Stipple` is the one variant that WOULD be 1-bit-legal by
/// construction (hard-thresholded pure-ink pixels, the same argument as the
/// highlight stipple) — but Wagtail ships NO placard by the user's own call
/// (the silent pole announces nothing), so the point stays banked.
/// `theme::tests::a_placard_grey_ink_would_violate_a_one_bit_worlds_own_law`
/// guards the grey combinations structurally so a FUTURE assignment can't
/// ship one by accident (see that test's own doc for why the guard lives in
/// `theme::`, never `render::`, where reading `is_one_bit()` is banned
/// outright).
/// `Muted` and `Bold` are the FIRETAIL-MAXIMALIST-SHOWCASE round's DIAL-UP
/// rungs — SMOOTH steps LOUDER than `Faint` (the previous ceiling), still
/// pure ladder blends through the same one owner (`super::derive::placard_ink`):
/// `Muted` is the world's own `muted` rung verbatim (the rows' own ink —
/// the wordmark stops receding and reads as a peer), `Bold` steps halfway
/// further from `muted` toward `base_content` (a clear statement, still
/// under full ink so the rows always win). DELIBERATELY never dithered —
/// smooth is Firetail's contrast with Mangrove (the personality-assignment
/// round's user call), so the dial-up keeps that split: texture stays
/// `Stipple`'s alone. NO world ships either yet (probe-reachable via
/// `AWL_OVERLAY_STYLE_FORCE` only; a later data flip assigns them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacardInk {
    Faint,
    Ghost,
    Stipple,
    Muted,
    Bold,
}

/// How a summoned overlay card announces which picker it is (see
/// `OverlayKind::title`, the one owner of the announced TEXT — this field
/// only ever decides how that text RENDERS). `InlinePrefix` is the shipped
/// default on EVERY world today: the existing quiet "<title> › " prefix on
/// the picker's own input line (`overlay_shape.rs::shape_overlay_names`,
/// `theme_picker.rs`'s own mirror), untouched by this round.
///
/// `Placard` is the OVERLAY-PERSONALITY-AS-DATA round's capability: a
/// large, corner-anchored, DIM wordmark of the SAME title text drawn BEHIND
/// the card's rows (Persona 3 Reload's CONFIG-screen watermark is the
/// reference) — `scale` multiplies the markdown heading TITLE type rung
/// (`markdown::headings::type_scale::TITLE`) over the document's own body
/// font size, so a world can dial how loud its wordmark reads without a
/// second magic number; `ink` picks how it draws off the ink ladder (see
/// [`PlacardInk`]). **BLEED IS THE CONTRACT** (the user-settled semantics,
/// pinned by `render::tests::overlay_personality`'s corner-placement tests):
/// the wordmark anchors to the FULL CANVAS corners and may bleed past the
/// centered card — a screen-corner watermark over the scrim, exactly P3R's
/// own bleed (an earlier draft clipped it to the card; that original is
/// SUPERSEDED — see `render::chrome::overlay_shape::overlay_shape_placard`'s
/// "THE SCREEN-CORNER ANCHOR" doc, the render-side owner of the settled
/// behavior); rows/query text always composite OVER it (uploaded first in
/// the text batch — legibility first). Assigned per-world as DATA by the
/// personality-assignment round (Galah/Magpie/Mangrove/Firetail, all
/// bottom-left — the TR/BR corners clip long words against the canvas edge,
/// a gallery finding); every quiet/silent world stays `InlinePrefix`, and
/// Wagtail deliberately announces nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TitleStyle {
    InlinePrefix,
    Placard { corner: PlacardCorner, scale: f32, ink: PlacardInk },
}

/// WHERE the summoned overlay card ANCHORS horizontally — a per-world DATA
/// dial (the PALETTE-COMPOSITION round, stealing Persona's off-center
/// COMPOSITION without its volume). `TopCenter` is the historical placement
/// (the card centered under the top-third); `TopLeft` pins the card's left
/// edge one `margin` in from the canvas edge, which reads MORE ANCHORED (a
/// deliberate object, not a floating dialog) AND opens the right side of the
/// canvas for a [`TitleStyle::Placard`] wordmark — the board's "menu top-left
/// + wordmark bottom-corner = balanced asymmetry". Only the card's X changes;
/// its width / row geometry / the placard's own canvas-corner anchor are
/// untouched, so every downstream reader (the selected-row band, the pointer
/// hit-test, the query caret) composes it for free through the ONE owner
/// [`crate::render::TextPipeline::overlay_card_x`]. The GLOBAL DEFAULT is
/// `TopLeft` (this round's flip); `TopCenter` stays reachable as a one-line
/// data revert (and the `AWL_OVERLAY_ANCHOR_FORCE` dev probe A/Bs the two).
/// The contextual SPELL popup is NOT a takeover card and ignores this — it
/// stays anchored at its misspelled word.
///
/// `Inset` is the FIRETAIL-MAXIMALIST-SHOWCASE round's STATEMENT dial: the
/// card's left edge sits `x_frac` of the free horizontal span in from the
/// left margin (`0.0` reproduces `TopLeft` exactly; `1.0` pins the card's
/// RIGHT edge one margin in from the canvas edge; `0.5` is `TopCenter`) —
/// one owner ([`crate::render::TextPipeline::overlay_card_x`]), one float,
/// the whole horizontal composition space as DATA. A HIGH `x_frac` (the
/// dramatic right-shifted statement) deliberately composes with the shipped
/// BOTTOM-LEFT placards: card right, wordmark bottom-left — balanced
/// asymmetry with no overlap (the long-title clipping that rejected BR
/// placards is the wordmark's own concern; the card never clips, it clamps).
/// NO world ships `Inset` yet (probe-reachable via `AWL_OVERLAY_ANCHOR_FORCE`).
// NOTE: no `Eq` — `Inset`'s `x_frac` is a float (same reasoning as `TitleStyle`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CardAnchor {
    TopLeft,
    TopCenter,
    Inset { x_frac: f32 },
}

/// Whether a thin FRAME draws around the WRITING COLUMN — the page-frame
/// capability the personality-assignment round graduated from the
/// `AWL_PAGE_BORDER` gallery probe (which never shipped; this field subsumes
/// it). DISTINCT from the summoned card's border ([`Elevation::Bordered`] —
/// that one rims a transient overlay card; this one is document furniture in
/// the DESIGN §5 "orientation" sense: it makes the page read as a deliberate
/// OBJECT, the WORLD-ROLES "dark-line page-frame" idea). `Line`'s
/// `weight_px` is the stroke weight; the INK is never carried here — it is
/// derived in ONE owner ([`super::derive::page_frame_ink`], the world's own
/// `base_content`, the full-ink ladder rung: a "dark line" on a light world,
/// pure white on Wagtail) so a frame can never invent a color. `None` is a
/// REAL state (most worlds — the frame is a statement, not a default), and
/// the ASSIGNED state is pixel-proven by
/// `render::tests::page_frame`. Wagtail is the first assignment (2px, its
/// ladder white); every other world ships `None`.
// NOTE: no `Eq` — `weight_px` is a float (same reasoning as `TitleStyle`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageFrame {
    None,
    Line { weight_px: f32 },
}

/// The FACE a world's summoned-overlay CHROME shapes in — the FIRETAIL-
/// MAXIMALIST-SHOWCASE round's `chrome_face` capability. "Chrome" is a
/// CLOSED, enumerated surface set: the placard WORDMARK, the inline
/// "<title> › " overlay TITLE prefix, and the faceted picker's lens-STRIP
/// labels — the frame around the list, never the list. LIST ROWS, the query
/// text, section headers, and the WRITING COLUMN itself always keep the
/// world's own body face ([`Theme::font`]) — legibility surfaces never
/// change face (the Persona "clean core, loud frame" split as a type rule).
/// `Body` (the default on every world) is a TOTAL no-op: the chrome shapes
/// in `Theme::font` exactly as before this round. `Named` swaps ONLY those
/// chrome spans to the named registered family (a bundled face, or a
/// probe-loaded audition candidate via `AWL_CHROME_FACE_FILE`). NO world
/// ships `Named` yet (probe-reachable via `AWL_CHROME_FACE_FORCE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromeFace {
    Body,
    Named(&'static str),
}

/// How a summoned overlay ENTERS the frame — the motion half of the
/// FIRETAIL-MAXIMALIST-SHOWCASE round's [`MotionJuice`] capability.
/// `Instant` (every world today) is the shipped behavior verbatim: the card
/// appears at its settled position the frame the overlay opens. `SpringIn`
/// slides the whole card in from a few px above with a small overshoot
/// spring (~200ms) — LIVE ONLY: the animator is armed exclusively by the
/// live App ([`crate::render::TextPipeline::arm_live_juice`]), so every
/// headless capture renders the settled position byte-identically
/// (determinism law), and Reduce Motion folds it to nothing (the step
/// settles instantly — `motion.rs`'s pure-time-compression contract).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayEntrance {
    Instant,
    SpringIn,
}

/// How the picker's selected-row BAND responds to a selection move — the
/// second [`MotionJuice`] dial. `Snap` (every world today) repositions the
/// band instantly. `Slide` eases it from the previous row to the new one
/// (~110ms, slight overshoot) — the "livelier selection response". Same
/// live-only + Reduce-Motion contract as [`OverlayEntrance`]; the band is
/// purely visual (the hit-test and the shaped rows never move), so the
/// slide can never desync a click from the row it lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandResponse {
    Snap,
    Slide,
}

/// The per-world MOTION-JUICE bundle: overlay entrance + selection-band
/// response, both LIVE-ONLY animations over overlay CHROME (never the
/// writing column, never a new color — pure position easing, so the
/// never-amber and figure/ground laws are untouched by construction).
/// [`MotionJuice::CALM`] on every world (byte-identical, zero animators
/// armed); the loud pole flips fields as one-line DATA in a later round.
/// Probe: `AWL_MOTION_FORCE` (live A/B only — a capture can't show time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MotionJuice {
    pub entrance: OverlayEntrance,
    pub band: BandResponse,
}

impl MotionJuice {
    /// The calm default: no entrance motion, band snaps — what every world
    /// ships today (structurally zero animation, not "fast animation").
    pub const CALM: MotionJuice = MotionJuice {
        entrance: OverlayEntrance::Instant,
        band: BandResponse::Snap,
    };
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
/// See the module-level doc above. `DEFAULT` is what fifteen of the sixteen
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
    /// THE OVERLAY-PERSONALITY-AS-DATA round's capability: how the summoned
    /// overlay card announces its title (see [`TitleStyle`]'s own doc). The
    /// card's ELEVATION/border story needs no new field here — [`Elevation`]
    /// already names it (`Flat` vs `Bordered`), and a `Placard` world can
    /// combine with EITHER elevation freely (the wordmark draws behind the
    /// rows on the SAME card either way).
    pub title_style: TitleStyle,
    /// THE PERSONALITY-ASSIGNMENT round's graduated capability: the thin
    /// frame around the writing column (see [`PageFrame`]'s own doc).
    pub page_frame: PageFrame,
    /// THE PALETTE-COMPOSITION round's dial: where the summoned overlay card
    /// anchors horizontally (see [`CardAnchor`]'s own doc). The GLOBAL DEFAULT
    /// is `TopLeft` (the round's flip toward a more-anchored, right-side-open
    /// composition); every world inherits it unless it opts back to
    /// `TopCenter`.
    pub card_anchor: CardAnchor,
    /// THE FIRETAIL-MAXIMALIST-SHOWCASE round's chrome-face capability (see
    /// [`ChromeFace`]'s own doc): which FACE the overlay chrome (placard
    /// wordmark / title prefix / strip labels) shapes in. `Body` everywhere
    /// (byte-identical) until a world flips it as data.
    pub chrome_face: ChromeFace,
    /// THE FIRETAIL-MAXIMALIST-SHOWCASE round's motion-juice capability (see
    /// [`MotionJuice`]'s own doc): live-only overlay entrance + selection-band
    /// response. [`MotionJuice::CALM`] everywhere until a world flips it.
    pub motion: MotionJuice,
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
        title_style: TitleStyle::InlinePrefix,
        page_frame: PageFrame::None,
        // The PALETTE-COMPOSITION round's flip: every world's summoned card
        // now anchors TOP-LEFT by default (reads more anchored + opens the
        // right side for the ghost placard). Revert to the historical centered
        // placement in ONE line here (`CardAnchor::TopCenter`).
        card_anchor: CardAnchor::TopLeft,
        // FIRETAIL-MAXIMALIST-SHOWCASE round: both new dials land INERT —
        // body face chrome, zero motion — on every world (byte-identical).
        chrome_face: ChromeFace::Body,
        motion: MotionJuice::CALM,
    };

}

/// The row/title HIGHLIGHT decision every "selected region" surface reduces
/// to — see [`Theme::highlight_treatment`]'s doc for the full history.
/// Deliberately carries NO absent/no-indicator variant: a consumer that
/// matches this enum is structurally incapable of preparing neither pipeline
/// (or both), which is exactly the shape that let a fully-transparent
/// highlight band ship unnoticed. `#[must_use]` so a caller that computes a
/// treatment and discards it (rather than acting on it) is a compile
/// warning, not a silent no-op.
#[derive(Clone, Copy, Debug, PartialEq)]
#[must_use]
pub enum HighlightTreatment {
    /// The default: an ordinary opaque value-band quad, tinted `Srgb`, under
    /// the row/title's own text (which keeps its content ink).
    ValueBand(Srgb),
    /// A true 1-bit world (`SelectionStyle::InverseVideo`): fill the band with
    /// a SOLID `band` ink (pure `base_content`) AND flip the SELECTED row/
    /// title's own glyphs to `ink` (pure `base_300`), so a hard black-on-white
    /// pair lands crisply.
    ///
    /// This replaces the old framebuffer invert of the ROW (`overlay_rows_invert`
    /// / `menubar_hi_invert`, both retired). A `1 - dst` flip is exact in LINEAR
    /// space, but the selected row's antialiased near-white glyph strokes (which
    /// peak around 0.94 coverage, never a full 1.0 at this small size) inverted
    /// THROUGH the sRGB gamma curve to a faint mid-grey (~0.08 linear → sRGB ~83,
    /// a ~7.7:1 ratio against the white band versus the crisp ~19:1 every
    /// UNSELECTED row reads at) — the Wagtail selected-row low-contrast bug.
    /// Drawing the two SOLID inks directly is gamma-independent; the band GROUND
    /// still reads as a hard invert because `base_content`/`base_300` ARE that
    /// world's only ink pair. Document SELECTION and the block CARET keep the
    /// true framebuffer invert (`SelectionStyle`/`CaretBlockStyle`): their glyphs
    /// are the document's own body text, large enough that the flip stays crisp,
    /// and they cannot know a "selected row ink" up front the way a picker can.
    InverseFill { band: Srgb, ink: Srgb },
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
    /// THE LAVA-LAMP GROUND — awl's first TIME-VARYING background (the mirror of
    /// Wagtail: the one world whose one warm thing is the GROUND itself). A slow
    /// ONE viewport-space 2D metaball field ("lava lamp" register) behind the
    /// page, masked out of the writing column so the page stays the clean flat
    /// figure. Page width changes only what the page occludes; the field's layout
    /// never changes (see `crate::lava` for the field + mask math and
    /// `shaders/lava.wgsl` for the shader). `ground` is the margin floor (the
    /// world's own `base_100`); `blob_lo`/`blob_hi` are the metaball's dim edge and
    /// bright core tones (value steps up the world's ladder, hue-rotated ≥40° clear
    /// of the caret's amber `primary`, DESIGN §3's one-accent law). `edge` is the
    /// column-boundary treatment ([`LavaEdge`]); `dithered` selects the coarse
    /// ordered (Bayer) print-grain stipple. The ANIMATION cadence (slow ~10 fps
    /// tick, pause on blur, `ambient_motion`-gated, Reduce-Motion/capture frozen)
    /// lives on the live App + `crate::lava`, NOT in this data. NO world ships this
    /// yet — this is the machinery only; a lava world is a later authored-DATA step.
    Lava {
        ground: Srgb,
        blob_lo: Srgb,
        blob_hi: Srgb,
        edge: LavaEdge,
        dithered: bool,
    },
}

/// The [`Background::Lava`] margin-boundary treatment — how the metaball field
/// meets the writing column's edge. Both read the SAME live column bounds
/// (`TextPipeline::column_left`/`column_width`, the one geometry owner); only the
/// fragment-shader mask math differs (see `shaders/lava.wgsl`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LavaEdge {
    /// The field fades fully BEFORE the column edge — a clean flat page, lava
    /// strictly marginal.
    Hard,
    /// The hard fade, PLUS a faint sub-threshold glow bleeding a short way UNDER
    /// the column edge (lamp-light spilling onto a desk, capped well below
    /// text-contrast relevance). The probe's agent pick.
    Glow,
}

impl LavaEdge {
    /// The shader mask-mode selector (`g.margin.w` in `shaders/lava.wgsl`):
    /// `1.0` = hard, `2.0` = edge-glow. Kept as a method so the shader contract
    /// has one owner rather than a magic literal at the upload site.
    pub fn mask_mode(self) -> f32 {
        match self {
            LavaEdge::Hard => 1.0,
            LavaEdge::Glow => 2.0,
        }
    }
    /// Lowercase name for the capture sidecar.
    pub fn as_str(self) -> &'static str {
        match self {
            LavaEdge::Hard => "hard",
            LavaEdge::Glow => "glow",
        }
    }
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
            // LAVA rides its OWN pipeline (`shaders/lava.wgsl`), drawn AFTER this
            // margin-ground shader. Here it degrades to shader 0 (a plain FLAT
            // gradient of the lava `ground`, `from == to`, no marks) so the margin
            // floor is painted even before the lava overlay draws — the lava layer
            // then overdraws the margins opaquely. See `crate::background`'s
            // `background_desc` (which reads these accessors) + `crate::lava`.
            Background::Lava { .. } => 0,
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
            Background::Lava { .. } => "lava",
        }
    }
    /// Gradient START endpoint. For [`Background::Lava`] this is the margin
    /// `ground` (so the flat-gradient shader-0 degrade paints the lava floor).
    pub fn from(&self) -> Srgb {
        match self {
            Background::Gradient { from, .. }
            | Background::Dots { from, .. }
            | Background::Starfield { from, .. }
            | Background::Pinstripe { from, .. }
            | Background::Stripes { from, .. } => *from,
            Background::Lava { ground, .. } => *ground,
        }
    }
    /// Gradient END endpoint. For [`Background::Lava`] this equals [`Self::from`]
    /// (`ground`) so the degrade is a FLAT fill (the lava overlay carries all the
    /// motion; the base ground never gradients).
    pub fn to(&self) -> Srgb {
        match self {
            Background::Gradient { to, .. }
            | Background::Dots { to, .. }
            | Background::Starfield { to, .. }
            | Background::Pinstripe { to, .. }
            | Background::Stripes { to, .. } => *to,
            Background::Lava { ground, .. } => *ground,
        }
    }
    /// Gradient DIRECTION (a roughly unit UV vector). For [`Background::Stripes`]
    /// it is DERIVED from `angle` so the gradient runs ALONG the stripe angle. For
    /// [`Background::Lava`] the base fill is flat, so `dir` is an inert placeholder.
    pub fn dir(&self) -> (f32, f32) {
        match self {
            Background::Gradient { dir, .. }
            | Background::Dots { dir, .. }
            | Background::Starfield { dir, .. }
            | Background::Pinstripe { dir, .. } => *dir,
            Background::Stripes { angle, .. } => (angle.cos(), angle.sin()),
            Background::Lava { .. } => (0.0, 1.0),
        }
    }
    /// The marks/band tint: the dot / star / pinstripe tint, or the stripe band.
    /// A plain [`Background::Gradient`] has NO marks; it returns its `from`
    /// endpoint as an inert placeholder (shader id 0 draws no marks). [`Background::Lava`]
    /// likewise has no margin-ground marks (the metaballs are the lava layer's), so
    /// it returns `ground`.
    pub fn tint(&self) -> Srgb {
        match self {
            Background::Dots { tint, .. }
            | Background::Starfield { tint, .. }
            | Background::Pinstripe { tint, .. } => *tint,
            Background::Stripes { band, .. } => *band,
            Background::Gradient { from, .. } => *from,
            Background::Lava { ground, .. } => *ground,
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
    /// True iff this world's margin ground is the animated [`Background::Lava`]
    /// lamp — the ONE gate every "should the lava layer draw / should the ambient
    /// tick arm / should page mode auto-enable" decision reads (never a per-world
    /// name comparison, which the `render::tests::theme_caps_law` grep-law bans).
    pub fn is_lava(&self) -> bool {
        matches!(self, Background::Lava { .. })
    }
    /// The lava metaball's `(ground, blob_lo, blob_hi, edge, dithered)` params, or
    /// `None` for the five static grounds — the pipeline uploads these when active
    /// and skips the draw entirely when `None` (so every non-lava world stays
    /// byte-identical).
    pub fn lava_params(&self) -> Option<(Srgb, Srgb, Srgb, LavaEdge, bool)> {
        match self {
            Background::Lava { ground, blob_lo, blob_hi, edge, dithered } => {
                Some((*ground, *blob_lo, *blob_hi, *edge, *dithered))
            }
            _ => None,
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
    /// The world's AXIS coordinates — its value on each of the four axes (Time /
    /// Register / Voice / Temperature), DERIVED from this world's palette + font (see
    /// [`ThemeTags`]). Once the theme picker's runtime lens-switcher; that strip was
    /// retired (user decision, 2026-07-15) and the picker is now a flat browsable
    /// list. The axes survive as the BUILD-TIME coverage ruler (`tests::
    /// axis_coverage_ruler`): every axis section stays covered by a curated band of
    /// worlds, and every world headlines at least one axis.
    pub tags: ThemeTags,
    /// Optional per-world SYNTAX ROLE-STYLE overrides (see [`RoleOverrides`]).
    /// [`RoleOverrides::NONE`] on fifteen of the sixteen worlds: the quiet role
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
    /// this field. [`RenderCaps::DEFAULT`] on fifteen of the sixteen worlds;
    /// Wagtail is the escape hatch's real use (`worlds.rs::WAGTAIL`).
    pub render_caps: RenderCaps,
}

impl Theme {
    /// THE ONE owner of the row/title "selected region" highlight decision —
    /// the picker's selected row (`render/chrome/overlay.rs`) and the menu
    /// bar's open-title band (`render/chrome/menubar.rs`) both call this
    /// instead of hand-rolling their own `if selection_style == ..`
    /// conditional. See [`HighlightTreatment`]'s own doc for why the return
    /// type itself — not a bool plus a separately-computed color — is the
    /// fix: it makes "draw nothing" a compile error, closing the exact hole
    /// the Wagtail invisible-picker-row bug lived in (a fully-transparent
    /// band silently passed every mechanism-shaped test, six surfaces, three
    /// rounds, because "no indicator" was a REPRESENTABLE outcome of the old
    /// `if invert { .. } else { .. }` shape).
    ///
    /// PURE in `self` — the `InverseFill` colors are read off THIS theme's own
    /// ladder (`base_content`/`base_300`), never the global active theme, so a
    /// caller iterating every world (the distinguishability + no-absent-case
    /// laws) gets each world's own pair without having to `set_active` first.
    pub fn highlight_treatment(&self, band: Srgb) -> HighlightTreatment {
        match self.render_caps.selection_style {
            SelectionStyle::Fill => HighlightTreatment::ValueBand(band),
            // A true 1-bit world owns exactly two inks; the selected band is a
            // SOLID `base_content` fill and the selected row's own glyphs flip
            // to solid `base_300`, so the pair reads as crisp black-on-white
            // (`InverseFill`'s doc explains why this replaced the framebuffer
            // invert of the row text).
            SelectionStyle::InverseVideo => HighlightTreatment::InverseFill {
                band: self.base_content,
                ink: self.base_300,
            },
        }
    }

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

// --- The THEME AXES (a build-time coverage ruler) + per-world tags -----------
//
// These four axes ([`Lens`]) once drove a runtime lens-switcher in the theme
// picker (LEFT/RIGHT cycled them, grouping worlds into faint sections). That strip
// was RETIRED (user decision, 2026-07-15 — WORLD-ROLES.md "DECIDED — retire the
// runtime LENS picker; the axes become a build-time ruler"); the picker is now a
// flat browsable list. The axes survive here ONLY as a BUILD-TIME coverage ruler:
// every world carries a value on EACH of the four axes ([`ThemeTags`]), and
// `tests::axis_coverage_ruler` asserts every section stays covered by a curated
// band of worlds. `All` is the degenerate "no grouping" axis kept for the ruler's
// STRIP-shape assertions.

/// A THEME AXIS. The four real dimensions each classify a world into one section;
/// `All` is the degenerate "no grouping" axis. Retained as the source of truth for
/// the build-time coverage ruler (`tests::axis_coverage_ruler`), no longer a runtime
/// picker lens. [`Lens::STRIP`] keeps `All` parked FIRST (the ruler's shape check).
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
    /// The degenerate axis (no grouping) — every world, no sections.
    All,
}

impl Lens {
    /// The axis order used by the coverage ruler, with `All` parked FIRST. (Once the
    /// LEFT/RIGHT strip order for the runtime picker; kept for the ruler's shape
    /// assertions after the strip's retirement.)
    pub const STRIP: [Lens; 5] = [Lens::All, Lens::Time, Lens::Register, Lens::Voice, Lens::Temperature];

    /// The SECTIONS this axis groups worlds into, in display order. `All` has none
    /// (the degenerate axis).
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

/// A world's value on EACH of the four axes — its coverage-ruler coordinates. Each
/// axis is OPT-OUT: a `None` axis means the world does not headline that axis, so each
/// section stays a CURATED handful (~2–4) rather than every world crowding in. A
/// `Some(section)` value is DERIVED from the world's own palette + font (see the doc
/// on each world): Time by background lightness/warmth, Register by font formality,
/// Voice by face class, Temperature by ground hue. The curation lives in the world
/// literals below; `tests::axis_coverage_ruler` asserts every section stays covered
/// and every world headlines at least one axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeTags {
    /// Section under [`Lens::Time`] (Dawn / Day / Dusk / Night), or `None` = opted out.
    pub time: Option<&'static str>,
    /// Section under [`Lens::Register`] (Humble / Everyday / Refined), or `None`.
    pub register: Option<&'static str>,
    /// Section under [`Lens::Voice`] (Literary / Technical / Modern), or `None`.
    pub voice: Option<&'static str>,
    /// Section under [`Lens::Temperature`] (Warm / Cool / Neutral), or `None`.
    pub temperature: Option<&'static str>,
}

impl ThemeTags {
    /// This world's section under `lens` — `None` when the world OPTS OUT of this axis
    /// (or for [`Lens::All`], which does not group).
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
