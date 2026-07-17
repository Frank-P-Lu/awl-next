//! src/theme/worlds.rs — the WORLDS DATA TABLE: the sixteen concrete
//! [`Theme`] literals (exact hex from the theme spec) + the [`THEMES`] cycle
//! order + [`DEFAULT_THEME`]. Pure data — no derivation logic lives here (see
//! [`crate::theme::derive`] for the active-theme accessors).

use super::cjk::{
    CJK_GOTHIC, CJK_JA_KLEE, CJK_JA_SHIPPORI, CJK_JA_ZENMARU, CJK_KO, CJK_KO_SERIF, CJK_MINCHO,
    CJK_ZH_HANS_KLEE, CJK_ZH_HANS_SANS, CJK_ZH_HANS_SERIF, CJK_ZH_HANT,
};
use super::color::Srgb;
use super::model::{
    Backdrop, Background, CardAnchor, CaretBlockStyle, ChipVariant, ChromeFace, DecorativeWash,
    Elevation, FacetStyle, HighlightTexture, ImageReveal, LavaEdge, ListStyle, MotionJuice, PageFrame,
    PlacardCorner, PlacardInk, RenderCaps, RoleOverrides, SelectionStyle, Theme, ThemeTags,
    TitleStyle, WashOverride,
};
use super::ornament::{
    Ornaments, BULLETS_PLAIN, BULLET_SCALE_ORNAMENT, BULLET_SCALE_PLAIN, ORNAMENT_GARAMOND,
    ORNAMENT_JUNICODE, ORNAMENT_MARKS, ORNAMENT_SCALE_FLEURON, ORNAMENT_SCALE_GEOMETRIC,
    ORNAMENT_SCALE_ORNATE,
};

/// FLIP ROUND (user FINAL PICKS 2026-07-17) — the SHIPPING poster list surface,
/// shared by every statement world (Firetail / Galah / Magpie / Mangrove) so the
/// four can never drift: `Bars` with the HUG-ALL HYBRID extent
/// ([`BarExtent::HugLabel`] — the plate hugs the LABEL, the shortcut chord
/// renders as bare dim text in the right-aligned column OUTSIDE the plate), the
/// gate's MID corner radius (6.0), every row a bar ([`BarCoverage::All`]), the
/// default gap (10) + selected-bar grow (24 px, one step past the label plate).
/// The calm/quiet worlds keep [`ListStyle::Pane`] (their selected row is already
/// the full-width band; the panel wants an unbroken rectangle).
const POSTER_BARS: ListStyle = ListStyle::Bars {
    radius: 6.0,
    gap: 10.0,
    grow_px: 24.0,
    extent: super::model::BarExtent::HugLabel,
    coverage: super::model::BarCoverage::All,
};

// --- The sixteen worlds (exact hex from the theme spec) ----------------------

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
    cjk: CJK_JA_SHIPPORI,
    zh_hans: CJK_ZH_HANS_SERIF,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO_SERIF,
    // Warm literary serif → Junicode's Caslon botanical sprays (an upward sprig + two sibling sprays).
    ornaments: Ornaments { dash: '\u{E67D}', star: '\u{E270}', underscore: '\u{E68A}' },
    ornament_face: ORNAMENT_JUNICODE,
    ornament_scale: ORNAMENT_SCALE_ORNATE,
    // Eucalyptus reading room → a small botanical hedera leaf + its mirror.
    bullets: ('❧', '☙'),
    bullet_scale: BULLET_SCALE_ORNAMENT,
    // Pale cool-green ground → Day; Literata reading serif → Refined / Literary; green hue → Cool.
    // Curated: shows under Day / Literary / Cool; opts OUT of Register (crowded → Bilby/Saltpan/Undertow keep Refined).
    tags: ThemeTags { time: Some("Day"), register: None, voice: Some("Literary"), temperature: Some("Cool") },
    role_overrides: RoleOverrides::NONE,
    // LIGHT-WORLD BORDER (composition round item 6, veto 3 adopted: "border on
    // light worlds totally works") — the summoned card's soft fill barely reads
    // off a pale ground, so a crisp rim carries its edge. DATA, no code path.
    render_caps: RenderCaps { elevation: Elevation::Bordered, ..RenderCaps::DEFAULT },
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
    // All-mono burrow → plain geometric bullets (restraint is its character).
    bullets: BULLETS_PLAIN,
    bullet_scale: BULLET_SCALE_PLAIN,
    // Dark burnt-orange room → Dusk (warm dark); Monaspace mono → Humble / Technical; rust hue → Warm.
    // Curated: a headliner on ALL four — Dusk / Humble / Technical / Warm are each its clearest exemplar.
    tags: ThemeTags { time: Some("Dusk"), register: Some("Humble"), voice: Some("Technical"), temperature: Some("Warm") },
    role_overrides: RoleOverrides::NONE,
    render_caps: RenderCaps::DEFAULT,
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
    cjk: CJK_JA_SHIPPORI,
    zh_hans: CJK_ZH_HANS_SERIF,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO_SERIF,
    // Literary serif world → EB Garamond fleurons; `***` uses ☙ (EBG has no ⁂).
    ornaments: Ornaments { dash: '❧', star: '☙', underscore: '❦' },
    ornament_face: ORNAMENT_GARAMOND,
    ornament_scale: ORNAMENT_SCALE_FLEURON,
    // Refined editorial serif → refined Renaissance fleuron bullets.
    bullets: ('❧', '❦'),
    bullet_scale: BULLET_SCALE_ORNAMENT,
    // Pale blue ground → Day; Newsreader display serif → Refined / Literary; blue hue → Cool.
    // Curated: shows under Day / Refined; opts OUT of Voice (Literary crowded) + Temperature (Cool crowded).
    tags: ThemeTags { time: Some("Day"), register: Some("Refined"), voice: None, temperature: None },
    role_overrides: RoleOverrides::NONE,
    // LIGHT-WORLD BORDER (composition round item 6) — a crisp rim carries the
    // card edge off the pale ground. DATA, no code path.
    render_caps: RenderCaps { elevation: Elevation::Bordered, ..RenderCaps::DEFAULT },
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
    ko: CJK_KO_SERIF,
    // Pale serif world → Junicode's horizontal running-vine Caslon scrolls (a vine + two sibling scrolls).
    ornaments: Ornaments { dash: '\u{F01B}', star: '\u{F01D}', underscore: '\u{F01E}' },
    ornament_face: ORNAMENT_JUNICODE,
    ornament_scale: ORNAMENT_SCALE_ORNATE,
    // Old-style salt-flat at first light → an airy floral-heart + leaf pair.
    bullets: ('❦', '❧'),
    bullet_scale: BULLET_SCALE_ORNAMENT,
    // Warm ecru salt flat → Dawn (warm-soft light); Fraunces old-style serif → Refined / Literary; sand hue → Warm.
    // Curated: shows under Dawn / Refined; opts OUT of Voice (Literary crowded) + Temperature (Warm crowded).
    tags: ThemeTags { time: Some("Dawn"), register: Some("Refined"), voice: None, temperature: None },
    role_overrides: RoleOverrides::NONE,
    // LIGHT-WORLD BORDER (composition round item 6) — a crisp rim carries the
    // card edge off the pale ground. DATA, no code path.
    render_caps: RenderCaps { elevation: Elevation::Bordered, ..RenderCaps::DEFAULT },
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
    cjk: CJK_JA_KLEE,
    zh_hans: CJK_ZH_HANS_KLEE,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Friendly humanist sans → the merged marks' floral trio (✿ florette + ❀ + ✽).
    ornaments: Ornaments { dash: '✿', star: '❀', underscore: '✽' },
    ornament_face: ORNAMENT_MARKS,
    ornament_scale: ORNAMENT_SCALE_GEOMETRIC,
    // Friendly modern reef → plain geometric bullets (unfussy, restrained).
    bullets: BULLETS_PLAIN,
    bullet_scale: BULLET_SCALE_PLAIN,
    // Warm peach reef → Dawn (warm-soft light); Fira Sans friendly humanist → Everyday / Modern; peach hue → Warm.
    // Curated: a headliner on ALL four — Dawn / Everyday / Modern / Warm each read clearly on the friendly peach sans.
    tags: ThemeTags { time: Some("Dawn"), register: Some("Everyday"), voice: Some("Modern"), temperature: Some("Warm") },
    role_overrides: RoleOverrides::NONE,
    // LIGHT-WORLD BORDER (composition round item 6) — a crisp rim carries the
    // card edge off the pale ground. DATA, no code path.
    render_caps: RenderCaps { elevation: Elevation::Bordered, ..RenderCaps::DEFAULT },
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
    // Selection contrast floor (2026-07-09): the old (0x4F,0x40,0x86,0x52) composited
    // to only ΔL 0.090 over this deep-violet ground — sub-glance ("you can't tell it's
    // highlighted"). Lifted L + alpha within the SAME violet hue family (~251°, still
    // ≥30° off the amber primary) to clear the contrast law.
    selection: Srgb::rgba(0x60, 0x50, 0xA8, 0x60),
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
    cjk: CJK_JA_SHIPPORI,
    zh_hans: CJK_ZH_HANS_SERIF,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO_SERIF,
    // OVERRIDE (the serif nocturne's flourish): mirror the default fleuron into its
    // reversed twin ☙ for `---`, and swap `___`'s heart to the black-heart bullet ❥
    // (both NS2 ornament variants, also bundled). `***` keeps the ⁂ asterism.
    // IN-FACE: Undertow's display IS EB Garamond, so its fleuron shapes in its own
    // face. The old {☙,⁂,❥} relied on the merged marks face (EBG has no ⁂/❥); the
    // set is now all-EBG fleurons (☙ dash keeps its distinct reversed look).
    ornaments: Ornaments { dash: '☙', star: '❧', underscore: '❦' },
    ornament_face: ORNAMENT_GARAMOND,
    ornament_scale: ORNAMENT_SCALE_FLEURON,
    // Classical literary midnight → the antique MANICULE ☞ (the medieval margin-
    // pointing hand, native to EB Garamond) at level 1, a hedera at level 2. The
    // one world that gets the manicule — a pointing hand on every bullet is loud,
    // so it rides the top level alone. The showpiece pick; flagged for veto.
    bullets: ('☞', '❧'),
    bullet_scale: BULLET_SCALE_ORNAMENT,
    // Dark violet current → Night; EB Garamond classic serif → Refined / Literary; violet-blue hue → Cool.
    // Curated: shows under Night / Refined / Literary (the classical serif's home); opts OUT of Temperature (Cool crowded).
    tags: ThemeTags { time: Some("Night"), register: Some("Refined"), voice: Some("Literary"), temperature: None },
    role_overrides: RoleOverrides::NONE,
    render_caps: RenderCaps::DEFAULT,
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
    ko: CJK_KO_SERIF,
    // Slab world → austere typographic Junicode marks (⁂ asterism + ⁑ + ❦ floral heart).
    ornaments: Ornaments { dash: '⁂', star: '⁑', underscore: '❦' },
    ornament_face: ORNAMENT_JUNICODE,
    ornament_scale: ORNAMENT_SCALE_ORNATE,
    // Slab-sturdy literary night → reversed leaf + floral heart (distinct from its
    // ⁂/⁑ asterism section trio).
    bullets: ('☙', '❦'),
    bullet_scale: BULLET_SCALE_ORNAMENT,
    // Blackish-olive night → Night; Zilla Slab workhorse slab → Everyday; slab-serif face → Literary; olive-green hue → Cool.
    // Curated: headlines Everyday alone (Night/Literary/Cool are each crowded); still reachable via All.
    tags: ThemeTags { time: None, register: Some("Everyday"), voice: None, temperature: None },
    role_overrides: RoleOverrides::NONE,
    render_caps: RenderCaps::DEFAULT,
};

/// Tawny — a quiet warm-grey nocturne with a tawny-gold caret; awl's original
/// "home" look (the DEFAULT world through 2026-07-10 — see [`DEFAULT_THEME`]'s
/// own doc comment for the 2026-07-11 default pick). Its display font is the
/// original bundled IBM Plex Mono — one `C-x t` reaches it from any other world.
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
    // The plain default home world → plain geometric bullets.
    bullets: BULLETS_PLAIN,
    bullet_scale: BULLET_SCALE_PLAIN,
    // Warm-grey neutral nocturne → Night; IBM Plex Mono → Humble / Technical; near-neutral grey → Neutral.
    // Curated: shows under Humble / Neutral (its plainest traits); opts OUT of Time (Night crowded) + Voice (Technical crowded).
    tags: ThemeTags { time: None, register: Some("Humble"), voice: None, temperature: Some("Neutral") },
    role_overrides: RoleOverrides::NONE,
    render_caps: RenderCaps::DEFAULT,
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
    cjk: CJK_JA_KLEE,
    zh_hans: CJK_ZH_HANS_KLEE,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Cosy expressive world → Junicode's ornate Caslon damask flourishes (a damask + candelabra + damask tile).
    ornaments: Ornaments { dash: '\u{E670}', star: '\u{F011}', underscore: '\u{F014}' },
    ornament_face: ORNAMENT_JUNICODE,
    ornament_scale: ORNAMENT_SCALE_ORNATE,
    // Utilitarian-and-soft charcoal room → the quiet ⁑ mark (least floral of the
    // Junicode pool) + a soft heart, honouring "utilitarian" while staying in-face.
    bullets: ('⁑', '❦'),
    bullet_scale: BULLET_SCALE_ORNAMENT,
    // Warm charcoal cosy dark → Dusk (warm dark); iA Writer Quattro utilitarian → Humble; sans-class writing face → Modern; warm hue → Warm.
    // Curated: shows under Dusk / Humble (its cosy utilitarian core); opts OUT of Voice (Modern crowded) + Temperature (Warm crowded).
    tags: ThemeTags { time: Some("Dusk"), register: Some("Humble"), voice: None, temperature: None },
    role_overrides: RoleOverrides::NONE,
    render_caps: RenderCaps::DEFAULT,
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
    cjk: CJK_JA_ZENMARU,
    zh_hans: CJK_ZH_HANS_SANS,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Clean sans nocturne → the merged marks' rosette/geometric trio (❂ rosette + ✴ + ◈).
    ornaments: Ornaments { dash: '❂', star: '✴', underscore: '◈' },
    ornament_face: ORNAMENT_MARKS,
    ornament_scale: ORNAMENT_SCALE_GEOMETRIC,
    // Crisp technical navy → plain geometric bullets.
    bullets: BULLETS_PLAIN,
    bullet_scale: BULLET_SCALE_PLAIN,
    // Midnight-navy nocturne → Night; IBM Plex Sans workhorse → Everyday / Modern; blue-black hue → Cool.
    // Curated: a headliner on ALL four — the crisp midnight dive reads clearly Night / Everyday / Modern / Cool.
    tags: ThemeTags { time: Some("Night"), register: Some("Everyday"), voice: Some("Modern"), temperature: Some("Cool") },
    role_overrides: RoleOverrides::NONE,
    render_caps: RenderCaps::DEFAULT,
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
    // Stark OLED coder's den → plain geometric bullets (stark restraint).
    bullets: BULLETS_PLAIN,
    bullet_scale: BULLET_SCALE_PLAIN,
    // Near-pure-black OLED → Night; Iosevka → Humble / Technical; true-black neutral → Neutral.
    // Curated: shows under Night (the darkest, most iconic) / Technical / Neutral; opts OUT of Register (Humble crowded).
    tags: ThemeTags { time: Some("Night"), register: None, voice: Some("Technical"), temperature: Some("Neutral") },
    role_overrides: RoleOverrides::NONE,
    // PERSONALITY ASSIGNMENT (2026-07-15): BORDERED elevation — OLED true-black
    // swallows a drop shadow entirely (black on black), so the raised border RIM
    // is this world's functional elevation, not decoration. The rim ink stays
    // the ordinary ramp-step `surface_selected` derivation (the ramp is not
    // collapsed here — only Wagtail's is). No placard: Currawong's stark den
    // stays quiet chrome.
    // COMPOSITION-C2: the iconic dark-technical statement world anchors its card
    // TOP-LEFT (a deliberate object, not a centred dialog).
    render_caps: RenderCaps {
        elevation: Elevation::Bordered,
        card_anchor: CardAnchor::TopLeft,
        ..RenderCaps::DEFAULT
    },
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
    // Selection contrast floor (2026-07-09): the old (0x2F,0x80,0x79,0x52) composited
    // to only ΔL 0.076 over this deep-teal ground — the weakest of every world. Lifted
    // L + alpha within the SAME teal hue family (~174°) to clear the contrast law.
    selection: Srgb::rgba(0x40, 0xA8, 0x9E, 0x60),
    // THE LAVA-LAMP GROUND (folded in 2026-07 — Mangrove is the COOL lava world,
    // the deepsea companion to Firetail's warm den): a slow DEEP-SEA metaball
    // field bobbing in the page margins (see `Background::Lava` + `crate::lava`),
    // deepening the existing "dark tidal-teal den, cool and rooted" identity that
    // the proximity Dots only gestured at. `ground` == base_100 (#112723) so the
    // flat page column and the margin floor read as one deep tidal den; blob_lo/
    // blob_hi are the dim-edge and bright-core COOL-BLUE tones (probe `deepsea`
    // palette, ~174° off the amber caret — nowhere near it — and both inside the
    // base_100..base_300 value band, so the animated margins read as GROUND, never
    // figure). Glow edge (soft light-spill under the column) + DITHERED: the coarse
    // ordered (Bayer) print-grain suits Mangrove's rooted, OLED-geometric-mono
    // character (and distinguishes the cool lamp from Firetail's smooth warm one).
    background: Background::Lava {
        ground: Srgb::rgb(0x11, 0x27, 0x23),
        blob_lo: Srgb::rgb(0x17, 0x23, 0x2B),
        blob_hi: Srgb::rgb(0x22, 0x3C, 0x4F),
        edge: LavaEdge::Glow,
        dithered: true,
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
    // Cool rooted tidal-teal → plain geometric bullets.
    bullets: BULLETS_PLAIN,
    bullet_scale: BULLET_SCALE_PLAIN,
    // Dark tidal-teal den → Night; JetBrains Mono → Humble / Technical; teal hue → Cool.
    // Curated: shows under Technical / Cool (its rooted teal-mono character); opts OUT of Time (Night crowded) + Register (Humble crowded).
    tags: ThemeTags { time: None, register: None, voice: Some("Technical"), temperature: Some("Cool") },
    role_overrides: RoleOverrides::NONE,
    // PERSONALITY ASSIGNMENT (2026-07-15): the STIPPLE placard — the Bayer
    // dither IS Mangrove's own language (its lava ground is the one dithered
    // lamp), so its wordmark speaks it too: bottom-left, scale 3.0, individual
    // full-ink pixels at a density derived to read at ~Faint tone
    // (`theme::placard_stipple_density`). TASTE-FLAGGED: scale 3.0 is the
    // gallery reference start, the stipple-vs-flat call is the user's A/B.
    // Plus BORDERED elevation: the summoned card must hold a crisp edge over
    // the moving lava margins (a value step alone swims against motion).
    render_caps: RenderCaps {
        title_style: TitleStyle::Placard {
            // COMPOSITION-C2: derive the poster corner from the card anchor
            // (TopLeft → bottom-RIGHT) — a balanced diagonal, poster off the card.
            corner: PlacardCorner::Auto,
            scale: 3.0,
            ink: PlacardInk::Stipple,
        },
        card_anchor: CardAnchor::TopLeft,
        elevation: Elevation::Bordered,
        // FLIP ROUND (user FINAL PICKS 2026-07-17): a poster/statement world →
        // the Bars HUG-ALL HYBRID (label-hug plate + bare right-aligned chords,
        // `BarExtent::HugLabel`) at the gate's MID radius (6), every row a bar.
        // Facet chips = BRACKET (the terminal-register corner ticks — the
        // technical/cool voice's own frame; user's confirmed chip map 2026-07-17).
        list_style: POSTER_BARS,
        facet_style: FacetStyle::Chips(ChipVariant::Bracket),
        ..RenderCaps::DEFAULT
    },
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
    cjk: CJK_JA_ZENMARU,
    zh_hans: CJK_ZH_HANS_SANS,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Humanist sans reading room → the merged marks' floral/rosette trio (❁ daisy + ❂ + ✿).
    ornaments: Ornaments { dash: '❁', star: '❂', underscore: '✿' },
    ornament_face: ORNAMENT_MARKS,
    ornament_scale: ORNAMENT_SCALE_GEOMETRIC,
    // Warm friendly dawn → plain geometric bullets (modern, unfussy).
    bullets: BULLETS_PLAIN,
    bullet_scale: BULLET_SCALE_PLAIN,
    // Dusty-pink reading room → Dawn (warm-soft light); Figtree humanist sans → Everyday / Modern; rose hue → Warm.
    // Curated: shows under Dawn / Modern / Warm (its soft rosy dawn feel); opts OUT of Register (Everyday crowded).
    tags: ThemeTags { time: Some("Dawn"), register: None, voice: Some("Modern"), temperature: Some("Warm") },
    role_overrides: RoleOverrides::NONE,
    // PERSONALITY ASSIGNMENT (2026-07-15): the gallery REFERENCE placard —
    // bottom-left Ghost at scale 3.0 was the shot the whole treatment was
    // validated on (placards read BEST on light worlds; BL because the TR/BR
    // corners clip long picker titles against the canvas edge).
    render_caps: RenderCaps {
        title_style: TitleStyle::Placard {
            // COMPOSITION-C2: poster corner derives from the card anchor
            // (TopLeft → bottom-RIGHT).
            corner: PlacardCorner::Auto,
            scale: 3.0,
            ink: PlacardInk::Ghost,
        },
        card_anchor: CardAnchor::TopLeft,
        // LIGHT-WORLD BORDER (composition round item 6) — a crisp rim carries
        // the card edge off the pale ground.
        elevation: Elevation::Bordered,
        // FLIP ROUND (2026-07-17): poster world → the Bars hug-all hybrid.
        // Facet chips = HAIRLINE (the landed baseline: filled active pill +
        // 1.5px ghost-stroke inactive pills — the soft dawn room's quiet frame;
        // user's confirmed chip map 2026-07-17, "Galah wears hairline").
        list_style: POSTER_BARS,
        facet_style: FacetStyle::Chips(ChipVariant::Hairline),
        ..RenderCaps::DEFAULT
    },
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
    ko: CJK_KO_SERIF,
    // Stark high-contrast slab → Junicode's geometric Caslon tile flowers (a quatrefoil + two lattice/damask tiles).
    ornaments: Ornaments { dash: '\u{EF90}', star: '\u{EF98}', underscore: '\u{EF9A}' },
    ornament_face: ORNAMENT_JUNICODE,
    ornament_scale: ORNAMENT_SCALE_ORNATE,
    // Paper-white high-contrast manuscript → floral-heart + leaf, marginalia on
    // stark paper. (The manicule would suit Magpie too, but the bundled Junicode
    // ornament subset lacks ☞ — hederas instead; see the round report.)
    bullets: ('❦', '☙'),
    bullet_scale: BULLET_SCALE_ORNAMENT,
    // Paper-white high-contrast page → Day; Bitter high-contrast slab → Everyday; slab-serif face → Literary; near-neutral hue → Neutral.
    // Curated: shows under Day / Literary / Neutral (sharp black-on-white slab); opts OUT of Register (Everyday crowded).
    tags: ThemeTags { time: Some("Day"), register: None, voice: Some("Literary"), temperature: Some("Neutral") },
    role_overrides: RoleOverrides::NONE,
    // PERSONALITY ASSIGNMENT (2026-07-15): bottom-left Ghost placard — the
    // newsprint-headline slab EARNS a masthead wordmark. TASTE-FLAGGED: starts
    // at the Galah-reference scale 3.0; Magpie's higher-contrast paper may
    // want it dialed after the user's gallery pass.
    render_caps: RenderCaps {
        title_style: TitleStyle::Placard {
            // COMPOSITION-C2: poster corner derives from the card anchor
            // (TopLeft → bottom-RIGHT).
            corner: PlacardCorner::Auto,
            scale: 3.0,
            ink: PlacardInk::Ghost,
        },
        card_anchor: CardAnchor::TopLeft,
        // LIGHT-WORLD BORDER (composition round item 6) — a crisp rim carries
        // the card edge off the pale ground.
        elevation: Elevation::Bordered,
        // FLIP ROUND (2026-07-17): poster world → the Bars hug-all hybrid.
        // Facet chips = UNDERLINE (no box; a thick short bar hugs the active
        // label — the newsprint-headline nav idiom, stark like the slab;
        // user's confirmed chip map 2026-07-17).
        list_style: POSTER_BARS,
        facet_style: FacetStyle::Chips(ChipVariant::Underline),
        ..RenderCaps::DEFAULT
    },
};

/// Wagtail — the FIFTEENTH world, and awl's first true MONOCHROME one — REWORKED
/// 2026-07 from its original GREYSCALE form (any grey permitted, zero saturation
/// only) into a TRUE 1-BIT world: **only `#000000` and `#FFFFFF` — nothing
/// between** (anti-aliased glyph/quad edges excepted; the law is about AUTHORED
/// colors). Named for the Willie Wagtail — the fearless, crepuscular
/// (dawn/dusk-active) black-and-white bird — this stays the deliberate
/// DESIGN.md §3 EXCEPTION: every other world keeps one WARM thing; Wagtail
/// keeps none, now pushed all the way to its logical floor. `Theme::is_one_bit`
/// (the STRICTER sub-case of `is_monochrome` this rework added) is `true` for
/// this world alone.
///
/// **The palette, literally:** ground `base_100`/`base_200`/`base_300` all pure
/// BLACK, ink `base_content`/`muted`/`faint` all pure WHITE (the ink ladder
/// COLLAPSES to one value — a true 1-bit world has nothing else to step
/// through; "comments/strings undifferentiated" is the deliberate 1-bit
/// statement, not an oversight), caret `primary` pure WHITE (motion + block
/// mass carry it, same as before), `primary_content` pure BLACK, `error` pure
/// WHITE (shape/inversion carries urgency, not a second brightness rung that
/// no longer exists), `selection` pure OPAQUE white (see the render-side note
/// below — a translucent selection was the old greyscale mechanism and is
/// gone). `background` is a flat `Gradient` with `from == to` (both pure
/// black) — the ONE `Background` variant guaranteed to introduce no
/// interpolated grey, since a gradient with identical endpoints is the same
/// color at every pixel by construction; the `Dots`/`Starfield`/`Pinstripe`/
/// `Stripes` variants all draw a translucent MARK tint over the ground and
/// were rejected for exactly that reason.
///
/// **Syntax roles — deliberately FLAT.** `role_overrides` pins
/// `def_fg`/`const_fg`/`str_fg` to the SAME pure white as `base_content` (not
/// merely "a grey" — literally the identical token), and turns BOTH washes
/// `Off`: a translucent wash quad of any alpha other than 0/255 would
/// composite white-over-black into a forbidden grey, so "OFF" is the only
/// 1-bit-legal answer for a SYNTAX role wash specifically — see "THE DITHER
/// ROUND" below for the markdown `==highlight==` wash's own different answer
/// (a dithered stipple, not OFF). The role-distinguishability laws
/// (`role_style_laws_hold_for_every_world`) gained a DECLARED EXEMPTION arm
/// for `Theme::is_one_bit()`, replaced by a FLAT LAW (every role's effective fg
/// is EXACTLY `base_content`, no role carries a wash) — never weakened for the
/// other fifteen worlds, which still clear the full pairwise/perceptibility/
/// luminance/ground-contrast suite unchanged.
///
/// **Elevation (cards/panels) — BORDER, not fill.** The 1-bit answer for
/// "raised surface" is a `theme::surface_selected()` one-bit override that
/// returns pure white regardless of the (now-degenerate, base_200==base_300)
/// ladder math — every FLOAT/HUD/WHICHKEY/menu-drop-panel BORDER (the
/// pre-existing "shadow → 1px-larger border → card" float-panel primitive,
/// `render/chrome/mod.rs::set_float_quads` — unchanged geometry, zero new
/// pipeline) reads pure white, while the CARD FILL itself (`base_300`, read
/// raw by `panel_card`/`float_card`/`hud_card`/`wk_card`) stays pure black —
/// flush with the canvas, so ink text drawn on it stays legible. A WYSIWYG
/// fence panel / inline-code pill (`base_200` raw, no border companion) is the
/// documented "OFF" case instead: black fill flush with the ground, invisible
/// — exactly the allowed washes/pills/panels answer ("OFF or a 1px white
/// outline", and a pill/panel has no existing outline mechanism to reuse
/// without building a new border pipeline, which this round explicitly does
/// not do). The picker's selected-ROW band (`overlay_rows`,
/// `render/chrome/overlay.rs`) is forced OFF (not `surface_selected`, which
/// would fill the WHOLE row white and hide the row's own white text) for a
/// one-bit world — the row's own amber caret still marks the current
/// position.
///
/// **Selection — ORIGINALLY the loudest open call; RESOLVED by the DITHER
/// round.** The greyscale/1-bit rework's own investigation (preserved below
/// for the history) found TRUE per-glyph inversion NOT reachable in THAT
/// round without new renderer machinery: `primary_content` — the token the
/// original spec assumed the block caret already used for an ink flip — was,
/// as of that investigation, DEAD CODE (declared per-world, read by exactly
/// one accessor, called by nothing) — the block caret draws BELOW the glyph
/// cell and never recolors it; only the MORPH caret's `CaretGlyphPipeline`
/// recolors text, by sampling a per-glyph coverage MASK for exactly ONE glyph
/// (the cursor's own letter) — generalizing that to an arbitrary multi-glyph
/// SELECTION RANGE is real pipeline-scale work. The OTHER path identified
/// then — a `OneMinusDst` invert-blend `RenderPipeline` drawn AFTER text — was
/// judged mathematically real but needing "a renderer round, not a theme
/// round" to build its own `wgpu::RenderPipeline` (blend state is baked in at
/// construction) and reorder the document draw list. **The DITHER round WAS
/// that renderer round:** `TextPipeline::selection_invert`
/// (`SelectionPipeline::new_invert`, `src/selection.rs`) is exactly that
/// `OneMinusDst`/`Zero`-blended pipeline, sharing `shaders/selection.wgsl`'s
/// geometry via a second fragment entry point (`fs_invert`) that always writes
/// pure white — combined with the blend factors, this computes an exact
/// `result = 1 - dst` per channel wherever the quad covers, drawn strictly
/// AFTER the document text in `draw_document_layers` (the reorder the old
/// investigation flagged as necessary). Black text flips white, white ground
/// flips black — the LITERAL "inverted text" ask, not a fallback. The old
/// "punch outline" mechanism (a translucent-white-quad-plus-inset-black-punch
/// approximation, kept as WAGTAIL's shipped v1 answer for one round) is
/// RETIRED outright: `selection_pipeline` uploads zero rects for a one-bit
/// world (`prepare_selection_layer`), and `selection_punch`/`inset_rect` were
/// deleted rather than kept as declared-dead code (no other world ever wanted
/// an outline, so there was nothing to preserve behind a "some day" comment —
/// same-behavior-same-code: a mechanism with zero remaining callers is a
/// mechanism that should not exist). `selection` itself stays pure OPAQUE
/// white (unchanged token) — it no longer drives the render directly; the
/// invert pipeline always writes its own fixed white regardless of any
/// theme's `selection` value, so the token's role today is closer to "the
/// LEGACY value other worlds' translucent fill still reads" than an active
/// one-bit control. AA edges under inversion: a glyph's antialiased ~50%-grey
/// edge pixel inverts to `1 - 0.5 = 0.5`, i.e. stays ~50%-grey — the SAME
/// AA-edge tolerance the one-bit pixel law already grants ordinary
/// (non-inverted) text, not a new exception. See
/// `render::tests::dither::invert_pipeline_flips_pure_black_and_pure_white_exactly`
/// for the real-pixel proof of the blend math itself.
///
/// **THE DITHER ROUND's second half — THE ONE WAGTAIL HIGHLIGHT TEXTURE.**
/// The user's razor: one kind of emphasis, one texture. `==highlight==` spans
/// and search matches — previously TWO different one-bit answers (highlight:
/// fully OFF/transparent; search match: the same solid-white/punch mechanism
/// document selection used) — now SHARE one mechanism: an ordered (8x8 Bayer)
/// dither stipple at a fixed ~25% density
/// (`render::dither::WAGTAIL_HIGHLIGHT_DITHER_DENSITY`, a TASTE TUNABLE),
/// where every drawn pixel is the pure quad color (opaque white) or fully
/// transparent — never a fractional alpha, so the stipple is 1-bit-legal by
/// construction rather than by staying invisible. Implemented as a MODE on
/// the EXISTING `shaders/selection.wgsl` quad shader (`Globals::dither`, `>
/// 0.0` switches `fs_main` from its ordinary soft alpha fill into the hard-
/// edged Bayer-thresholded branch) rather than a new pipeline class — one
/// shader, one owner, the SAME `SelectionPipeline` type every other quad
/// (selection fill, syntax washes, WYSIWYG panel/pill) already uses.
/// `wash_highlight_pipeline` (the `==highlight==` band) and `match_pipeline`
/// (search matches) both flip into dither mode together
/// (`render::spans::wagtail_dither_density`), so the two consumers can never
/// drift to different densities. **The banding-kill half of the DITHER
/// round (an ordered ±half-8-bit-step dither added to EVERY world's margin
/// gradient before quantization) is an EXACT no-op for Wagtail specifically**
/// — its `background` is the one `Gradient` variant with `from == to`, and
/// the shader gates the dither offset on `from != to` for precisely this
/// reason (see `render::dither`'s module doc + `shaders/background.wgsl`'s
/// `fs_main`), so this round's banding fix introduces zero risk to the
/// one-bit law even though it touches every world's gradient shader
/// uniformly.
///
/// **Frosted-blur backdrop (overlay takeover / held HUD / lifetime card /
/// hold-peek) — disabled outright for a one-bit world.** The scrim mechanism
/// investigated: the OLD flat `overlay_scrim()` token (`theme/derive.rs`) is
/// itself DEAD CODE today (superseded by a real gaussian-blur backdrop,
/// `render.rs`'s `backdrop_blur`/`BlurBackdrop`) — a gaussian defocus of a
/// pure black/white document mathematically SMEARS every edge into
/// intermediate grey, so it is structurally incompatible with the 1-bit law
/// regardless of tuning. `TextPipeline::backdrop_blur` gained a one-bit
/// short-circuit (`theme::active().is_one_bit()` → `false`, before the
/// existing OR-chain) so every backdrop-blur consumer falls back to the
/// EXISTING crisp path (the same "document stays bright, no blur, no scrim"
/// exception the theme/caret pickers already use) — the solid white-bordered
/// card still reads clearly over a SHARP, not smeared, black/white document.
/// The decorative drop-SHADOW (`float_shadow_srgba`, ink-at-low-alpha over the
/// canvas) and the writing-nit underline (`nit_underline_srgba`,
/// muted-at-alpha) are two more translucent renderer-wide washes that would
/// otherwise composite grey; both gained a one-bit branch returning fully
/// transparent (`[0,0,0,0]`) — "OFF", the same sanctioned answer as the
/// pill/panel case — leaving the crisp white BORDER alone to carry elevation.
///
/// **WYSIWYG in 1-bit (accepted, documented — DESIGN.md's own instruction):**
/// concealed markup is invisible (fine, unchanged); REVEALED markup renders
/// full white (no dim `muted` rung exists to recede to — `muted == base_content`
/// by construction) — structure-by-render, not by tone, is this world's
/// character, not a bug.
///
/// Drawn in JetBrains Mono still — unchanged from the greyscale round; "a
/// crisp, tall coding monospace" is exactly the character a 1-bit world wants
/// too, so Wagtail stays a MONO-DISPLAY world sharing its exact display font
/// with Mangrove (logged, unchanged consequence of the original round).
///
/// See `render::tests::syntax_roles::every_one_bit_world_renders_only_pure_black_or_white`
/// (the NEW law this rework demands — supersedes `every_monochrome_world_
/// renders_zero_saturation_everywhere`'s old "any grey" tolerance for whichever
/// worlds are ALSO one-bit), `render/tests/one_bit.rs` (the render-pipeline
/// instance-level half: backdrop-blur disabled, the invert pipeline's/dither
/// mode's on-off gating), and `render/tests/dither.rs` (the DITHER round's
/// REAL-PIXEL half: the invert blend math, the dither stipple's pixel purity,
/// and the flat-gradient no-op, all verified against actual GPU output).
pub const WAGTAIL: Theme = Theme {
    name: "Wagtail",
    dark: true,
    base_100: Srgb::rgb(0x00, 0x00, 0x00),
    base_200: Srgb::rgb(0x00, 0x00, 0x00),
    base_300: Srgb::rgb(0x00, 0x00, 0x00),
    base_content: Srgb::rgb(0xFF, 0xFF, 0xFF),
    // The ink ladder COLLAPSES to one value in a true 1-bit world — there is
    // nothing else to step through. See the doc comment above.
    muted: Srgb::rgb(0xFF, 0xFF, 0xFF),
    faint: Srgb::rgb(0xFF, 0xFF, 0xFF),
    // The caret: PURE WHITE — the brightest (only) ink value, carried by value
    // + motion alone, never hue.
    primary: Srgb::rgb(0xFF, 0xFF, 0xFF),
    primary_content: Srgb::rgb(0x00, 0x00, 0x00),
    // Shape/inversion carries urgency now — no brighter-than-white rung exists.
    error: Srgb::rgb(0xFF, 0xFF, 0xFF),
    // Pure OPAQUE white — legibility over selected text is carried by the
    // TRUE inverse-video render-side mechanism (`TextPipeline::selection_invert`,
    // the DITHER round), NOT by this token's alpha (the invert pipeline
    // always writes its own fixed white regardless of this value). See the
    // doc comment above for the full mechanism.
    selection: Srgb::rgba(0xFF, 0xFF, 0xFF, 0xFF),
    // A flat gradient with from == to: the one `Background` variant that is
    // mathematically guaranteed to introduce no interpolated grey.
    background: Background::Gradient {
        from: Srgb::rgb(0x00, 0x00, 0x00),
        to: Srgb::rgb(0x00, 0x00, 0x00),
        dir: (0.0, 1.0),
    },
    // Display face IS already the crisp/technical JetBrains Mono → reuse it
    // for code too (the fifth mono-display world; unchanged from the
    // greyscale round's logged font-sharing consequence).
    font: "JetBrains Mono",
    mono: "JetBrains Mono",
    cjk: CJK_GOTHIC,
    zh_hans: CJK_ZH_HANS_SANS,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Crisp mono-display world → the merged marks' unused star/paragraph trio
    // (✧ open star + ⭑ solid star + ❡ paragraph ornament).
    ornaments: Ornaments { dash: '✧', star: '⭑', underscore: '❡' },
    ornament_face: ORNAMENT_MARKS,
    ornament_scale: ORNAMENT_SCALE_GEOMETRIC,
    // Restraint IS monochrome's whole character → plain geometric bullets.
    bullets: BULLETS_PLAIN,
    bullet_scale: BULLET_SCALE_PLAIN,
    // Willie Wagtails are crepuscular (dawn/dusk-active) → Dusk (the one lens
    // section with curation room: Potoroo + Mopoke, 2 of a 2-3 band). Register /
    // Voice / Temperature are ALL already at their curated 3-world cap, so
    // Wagtail opts out of them rather than crowd a section — reachable via
    // All + fuzzy search regardless, and it still headlines Time.
    tags: ThemeTags { time: Some("Dusk"), register: None, voice: None, temperature: None },
    // Wagtail's own escape hatch, now pushed to FLAT rather than "a plain
    // grey": a hue-anchored derivation cannot serve a zero-saturation world at
    // all, and a 1-bit world additionally has no room for a SECOND ink value —
    // every role fg is pinned to the exact SAME token as `base_content`
    // (identity, not merely a nearby grey), and both washes are `Off` (any
    // non-0/255 alpha over black would be a forbidden grey). See
    // `role_style_laws_hold_for_every_world`'s one-bit exemption arm.
    role_overrides: RoleOverrides {
        def_fg: Some(Srgb::rgb(0xFF, 0xFF, 0xFF)),
        const_fg: Some(Srgb::rgb(0xFF, 0xFF, 0xFF)),
        str_fg: Some(Srgb::rgb(0xFF, 0xFF, 0xFF)),
        comment_wash: WashOverride::Off,
        str_wash: WashOverride::Off,
    },
    // THEME CAPABILITIES AS DATA: Wagtail is the escape hatch's real use —
    // every field deviates from `RenderCaps::DEFAULT`, DATA-encoding exactly
    // the render decisions this world's doc comment above walks through
    // mechanism-by-mechanism (selection/caret true inverse video, no
    // frosted blur, bordered elevation, decorative washes off, an opaque
    // image-reveal scrim, and the one dithered stipple texture shared by
    // `==highlight==` spans + search matches).
    render_caps: RenderCaps {
        selection_style: SelectionStyle::InverseVideo,
        caret_block_style: CaretBlockStyle::InverseVideo,
        backdrop: Backdrop::Flat,
        elevation: Elevation::Bordered,
        decorative_wash: DecorativeWash::Off,
        image_reveal: ImageReveal::Opaque,
        highlight_texture: HighlightTexture::Stipple {
            color: Srgb::rgb(0xFF, 0xFF, 0xFF),
            density: crate::render::dither::WAGTAIL_HIGHLIGHT_DITHER_DENSITY,
        },
        // PERSONALITY ASSIGNMENT (2026-07-15, user-confirmed): NO placard —
        // Wagtail is the SILENT pole; announcing itself in a corner wordmark
        // would be personality, which this world's whole statement is having
        // none of. `InlinePrefix` (the quiet "<title> › " line) stays.
        title_style: TitleStyle::InlinePrefix,
        // PERSONALITY ASSIGNMENT (2026-07-15): the PAGE FRAME's first (and
        // only) assignment — a 2px frame around the writing column in this
        // world's ladder white (`theme::page_frame_ink` = `base_content`),
        // the "page reads as a deliberate object" idea (retired; decision
        // recorded in THEMES.md). Drawn
        // hard-edged (dither-1.0 fill, no fractional-alpha AA rim) so it is
        // 1-bit-legal by construction. Graduated from the AWL_PAGE_BORDER
        // gallery probe (2px white was the user's pick over 1px).
        page_frame: PageFrame::Line { weight_px: 2.0 },
        // The PALETTE-COMPOSITION round's global flip — Wagtail rides it too
        // (the silent pole is still an anchored object). Listed explicitly
        // because this literal names every field (no `..DEFAULT` spread).
        card_anchor: CardAnchor::TopLeft,
        // FIRETAIL-MAXIMALIST-SHOWCASE round: the silent pole keeps BOTH new
        // dials at their calm defaults, deliberately — body-face chrome, zero
        // motion (the no-personality statement, again).
        chrome_face: ChromeFace::Body,
        motion: MotionJuice::CALM,
        // PER-ITEM LIST SURFACES round: the silent pole keeps the single Pane +
        // plain-text strip — bars/chips would be personality. Listed explicitly
        // because this literal names every field (no `..DEFAULT` spread).
        list_style: ListStyle::Pane,
        facet_style: FacetStyle::Text,
    },
};

/// Firetail — the SIXTEENTH world, a WARM STATEMENT world and awl's FIRST
/// lava-lamp ground: the MIRROR of Wagtail. Where Wagtail keeps NO warm thing
/// (its statement is the bare 1-bit room), Firetail's one living warm thing is
/// the GROUND ITSELF — a slow oxblood/wine metaball "lava lamp" bobbing in the
/// page margins (see [`Background::Lava`] + `crate::lava`), the DESIGN.md §3
/// ambient-motion amendment's first host (Mangrove is the cool second). The
/// room is its own deep oxblood-charcoal den — redder beside Undertow's violet,
/// substantially less orange/rust than Potoroo. Warm blush ink, muted claret
/// chrome, wine lava, and an ember-gold caret form one coherent original palette.
/// The caret stays ≥40° of hue clear of the wine lava so amber remains the
/// caret's alone (DESIGN §3, the amber-guard law). Named for the Red-browed
/// Firetail finch's flame; drawn in Monaspace Xenon — technical restraint so the
/// living ground is the whole statement.
pub const FIRETAIL: Theme = Theme {
    name: "Firetail",
    dark: true,
    // ORIGINAL OXBLOOD-CHARCOAL ladder — base_100 doubles as the lava `ground`
    // so the flat page column and animated margin floor meet without a seam.
    // The grounds stay near red-wine (never Potoroo's orange/rust); warm blush
    // content recedes through dusty mauve `muted` to claret `faint`.
    base_100: Srgb::rgb(0x17, 0x09, 0x0C),
    base_200: Srgb::rgb(0x24, 0x0D, 0x12),
    // The focused plane reaches the lamp core's value without crossing into
    // figure territory; the lava value-band law pins that relationship.
    base_300: Srgb::rgb(0x52, 0x16, 0x29),
    base_content: Srgb::rgb(0xEF, 0xE5, 0xE2),
    muted: Srgb::rgb(0x9F, 0x7E, 0x7C),
    faint: Srgb::rgb(0x69, 0x48, 0x4A),
    // Ember-gold caret (hue ~41°), held ~59° clear of the wine lava (~342°):
    // gold stays the ONE accent (DESIGN §3, the amber-guard).
    primary: Srgb::rgb(0xF2, 0xB1, 0x40),
    primary_content: Srgb::rgb(0x23, 0x14, 0x05),
    error: Srgb::rgb(0xE6, 0x4E, 0x48),
    // A lifted dusty-wine wash: in-family, visible over the oxblood floor, but
    // neither gold nor a second loud accent.
    selection: Srgb::rgba(0xB6, 0x5A, 0x6E, 0x60),
    // THE LAVA-LAMP GROUND (the world's whole statement): a slow oxblood/wine
    // metaball field in the margins, `ground` == base_100 (seamless). blob_lo/
    // blob_hi are the dim-edge and bright-core WINE tones (~342° hue — ≥40° off
    // the ember caret — both inside the base_100..base_300
    // value band, so the animated margins always read as GROUND, never figure).
    // Glow edge (soft light-spill under the column), UNDITHERED — the smooth warm
    // lamp (Mangrove takes the dithered cool one).
    background: Background::Lava {
        ground: Srgb::rgb(0x17, 0x09, 0x0C),
        blob_lo: Srgb::rgb(0x24, 0x0C, 0x14),
        blob_hi: Srgb::rgb(0x52, 0x18, 0x2C),
        edge: LavaEdge::Glow,
        dithered: false,
    },
    // Monaspace Xenon is the typographic kinship with Potoroo; the palette itself
    // remains Firetail's own. The display face IS mono, so code reuses it.
    font: "Monaspace Xenon",
    mono: "Monaspace Xenon",
    cjk: CJK_GOTHIC,
    zh_hans: CJK_ZH_HANS_SANS,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Warm technical den → the merged marks' spark trio (✷ 8-star + ✶ 6-star + ✦ 4-star).
    ornaments: Ornaments { dash: '✷', star: '✶', underscore: '✦' },
    ornament_face: ORNAMENT_MARKS,
    ornament_scale: ORNAMENT_SCALE_GEOMETRIC,
    // The living ground IS the statement → plain geometric bullets, restrained chrome.
    bullets: BULLETS_PLAIN,
    bullet_scale: BULLET_SCALE_PLAIN,
    // Warm lava den → Temperature=Warm (its clearest read). Every Time / Register /
    // Voice section already sits at its curated cap, so Firetail — like Wagtail —
    // opts OUT of them rather than crowd a section, headlining Warm alone (which the
    // roster-growth curation widening now seats as a 4-world band).
    tags: ThemeTags { time: None, register: None, voice: None, temperature: Some("Warm") },
    role_overrides: RoleOverrides::NONE,
    // CHROME-VOICES FLIP (2026-07-16, from the maximalist-showcase gallery — the
    // user's picks): Firetail is awl's LOUD-END statement world, so its summoned
    // overlay speaks loud too. Bottom-left BOLD placard at the combo-shot scale
    // (`Bold` = the muted→base_content half-step, the showcase round's loudest
    // smooth ladder rung; still under full ink so the rows always win) — bigger
    // AND louder than the old `Faint`/3.0, deliberately NOT dithered (smooth is
    // Firetail's contrast with Mangrove — the wordmark speaks that same split).
    // Plus `chrome_face = Archivo Black`: the placard wordmark / inline title
    // prefix / lens-strip labels shape in the LOUD chrome voice, while the LIST
    // ROWS, query text and the writing column stay Monaspace Xenon (the closed
    // chrome surface set — legibility surfaces never change face). Archivo Black
    // registers at usWeightClass 400 (verified in-file), so `chrome_attrs`'s
    // plain `Weight::NORMAL` request matches it — no `mono_safe_weight`
    // exception. Retains BORDERED elevation: the card holds a crisp edge over
    // the moving lava margins. Every OTHER world stays `Body`/`InlinePrefix`
    // (byte-identical) — Firetail alone flips this round.
    render_caps: RenderCaps {
        title_style: TitleStyle::Placard {
            // COMPOSITION-C2: Firetail KEEPS its user-picked BOTTOM-LEFT placard
            // (an explicit corner overrides the Auto derivation) — the dramatic
            // combo the user locked from the flip-round gallery. Card TopLeft +
            // poster BL sit on the same left rail but separate vertical bands
            // (card near the top, poster at the very foot), no overlap.
            corner: PlacardCorner::BL,
            scale: 4.5,
            ink: PlacardInk::Bold,
        },
        card_anchor: CardAnchor::TopLeft,
        chrome_face: ChromeFace::Named("Archivo Black"),
        elevation: Elevation::Bordered,
        // FLIP ROUND (2026-07-17): the maximalist showcase world → the Bars
        // hug-all HYBRID (label-hug plate + bare right-aligned chords).
        // Facet chips = FILLED (the active label a SOLID value-step fill with
        // INVERTED ink, inactive bare text — the loudest chip for the loud-end
        // world; user's confirmed chip map 2026-07-17).
        list_style: POSTER_BARS,
        facet_style: FacetStyle::Chips(ChipVariant::FilledActive),
        ..RenderCaps::DEFAULT
    },
};

/// All sixteen worlds, in cycle order. `C-x t` advances through this list and
/// wraps; `C-x T` steps backward. The two deep cool darks — Currawong (OLED
/// black) beside the neutral Tawny/Mopoke pair, and Kingfisher (midnight navy)
/// beside the violet Undertow — sit with their kin; the two STATEMENT worlds
/// close the cycle as mirror bookends — Wagtail (the bare 1-bit room, NO warm
/// thing) beside Firetail (the warm den whose one warm thing is the living lava
/// GROUND itself).
pub const THEMES: [Theme; 16] = [
    TAWNY, MOPOKE, CURRAWONG,
    POTOROO, GUMTREE, BILBY, SALTPAN, QUOKKA, UNDERTOW, KINGFISHER, OUTBACK, MANGROVE, GALAH, MAGPIE,
    WAGTAIL, FIRETAIL,
];

/// Index into [`THEMES`] of the default/startup world: **Saltpan** (index 6 —
/// `TAWNY, MOPOKE, CURRAWONG, POTOROO, GUMTREE, BILBY, SALTPAN, ...`), a warm
/// light world (sun-bleached salt flat, cinnamon-clay caret on ecru), picked by
/// the user 2026-07-11 as awl's first impression — a taste round, not a bugfix
/// (the prior default, index 0 = Tawny, a dark warm-grey mono nocturne, remains
/// one `C-x t` cycle away). This only governs a genuinely FRESH launch/capture:
/// the sticky theme preference (`config.toml`'s `theme` key, written whenever
/// the user switches worlds via Cmd-T) always wins for an EXISTING user —
/// `Config::apply_sticky_globals` applies it over this constant unless the
/// `--theme` CLI flag already set the global (see `config/apply.rs`).
pub const DEFAULT_THEME: usize = 6;
