//! src/theme/worlds.rs — the WORLDS DATA TABLE: the eighteen concrete
//! [`Theme`] literals (exact hex from the theme spec) + the [`THEMES`] cycle
//! order + [`DEFAULT_THEME`]. Pure data — no derivation logic lives here (see
//! [`crate::theme::derive`] for the active-theme accessors).

use super::cjk::{
    CJK_GOTHIC, CJK_JA_KLEE, CJK_JA_SHIPPORI, CJK_JA_ZENMARU, CJK_KO, CJK_KO_SERIF, CJK_MINCHO,
    CJK_ZH_HANS_KLEE, CJK_ZH_HANS_SANS, CJK_ZH_HANS_SERIF, CJK_ZH_HANT,
};
use super::color::Srgb;
use super::model::{
    AmbientStyle, Backdrop, Background, CardAnchor, CardShape, CardTexture, CaretBlockStyle,
    ChipVariant, ChromeFace, DecorativeWash, Elevation, FacetStyle, FoldAfford, Frost,
    HighlightTexture, ImageReveal, LavaEdge, ListStyle, MotionJuice, PageFrame, PaneSplit,
    PlacardCorner, PlacardInk, RenderCaps, RoleOverrides, SelectionStyle,
    SPELL_UNDERLINE_GAP_DEFAULT, Theme, ThemeTags, TitleStyle, WashOverride,
};
use super::ornament::{
    Ornaments, BULLETS_PLAIN, BULLET_SCALE_ORNAMENT, BULLET_SCALE_PLAIN, LIST_INDENT_SCALE_PLAIN,
    LIST_INDENT_SCALE_WIDE, ORNAMENT_GARAMOND, ORNAMENT_JUNICODE, ORNAMENT_MARKS,
    ORNAMENT_SCALE_FLEURON, ORNAMENT_SCALE_GEOMETRIC, ORNAMENT_SCALE_ORNATE,
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

// --- The eighteen worlds (exact hex from the theme spec) ---------------------

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
    // GRASS-BANDS (item 69) — three large tone-on-tone diagonal bands across
    // the WHOLE margin field (cut-paper grass), not a repeating dot-grid
    // wallpaper. ONLY the eucalyptus ground ladder — `base_100`/`base_200`/
    // `base_300` verbatim, no separately-tuned tint — at a ~32° cut (cf.
    // Potoroo's Stripes at 0.6rad/34°, deliberately its own angle+shape so the
    // two diagonal grounds never read as siblings).
    background: Background::Bands {
        tones: [
            Srgb::rgb(0xE4, 0xF8, 0xE2),
            Srgb::rgb(0xCF, 0xF3, 0xCC),
            Srgb::rgb(0xB7, 0xEF, 0xB4),
        ],
        angle: 0.56,
    },
    font: "Literata",
    // Literary serif world → the slab-serif Monaspace Xenon: a mono that keeps a
    // whisper of the serif so the code page still reads as this world's kin.
    mono: "Monaspace Xenon",
    // Literata's serif contrast carries hierarchy structurally — size alone reads.
    heading_bold: false,
    cjk: CJK_JA_SHIPPORI,
    zh_hans: CJK_ZH_HANS_SERIF,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO_SERIF,
    // Warm literary serif → Junicode's Caslon botanical sprays (an upward sprig + two sibling sprays).
    ornaments: Ornaments { dash: '\u{E67D}', star: '\u{E270}', underscore: '\u{E68A}' },
    ornament_face: ORNAMENT_JUNICODE,
    ornament_scale: ORNAMENT_SCALE_ORNATE,
    // Eucalyptus reading room → a small botanical hedera leaf + its mirror + the
    // family's third fleuron for level 3 (item 15's per-level rotation).
    bullets: ('❧', '☙', '❦'),
    bullet_scale: BULLET_SCALE_ORNAMENT,
    list_indent_scale: LIST_INDENT_SCALE_WIDE,
    // Pale cool-green ground → Day; Literata reading serif → Refined / Literary; green hue → Cool.
    // Curated: shows under Day / Literary / Cool; opts OUT of Register (crowded → Bilby/Saltpan/Bombora keep Refined).
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
    // Monaspace Xenon's uniform mono strokes need weight to mark a section head.
    heading_bold: true,
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
    list_indent_scale: LIST_INDENT_SCALE_PLAIN,
    // Dark burnt-orange room → Dusk (warm dark); Monaspace mono → Humble / Technical; rust hue → Warm.
    // Curated: a headliner on ALL four — Dusk / Humble / Technical / Warm are each its clearest exemplar.
    tags: ThemeTags { time: Some("Dusk"), register: Some("Humble"), voice: Some("Technical"), temperature: Some("Warm") },
    role_overrides: RoleOverrides::NONE,
    render_caps: RenderCaps::DEFAULT,
};

/// Bilby — FIRST LIGHT: the palest, warmest-horizon light world (sunrise-gold
/// caret on a pale rose-gold page; the night's violet still in the ink).
///
/// DAWN ROUND (2026-07-18, user verdict on the Bilby/Gumtree near-pair: no
/// merge — DIFFERENTIATE; "Bilby → DAWN"): the bilby is a dawn-active desert
/// marsupial, so its world became dawn itself. The old pale-BLUE day room read
/// as Gumtree's pale-green sibling (same literary serif + Xenon + cool pale
/// ground); this retune flips the TEMPERATURE STRUCTURE outright — dawn's own
/// complementary split: a warm rose-gold horizon in the ground planes, the
/// night's cool violet-grey left in the whole ink ladder. Nothing else in the
/// roster pairs a warm ground with a violet ink end.
///
/// - **Ground**: the palest warm ground of any world (relY 0.940 — above
///   Saltpan's 0.929; only Magpie's NEUTRAL paper is brighter). Placed by a
///   max-min-redmean sweep over the crowded pale-warm band: ~19 to each of
///   Saltpan / Galah / Magpie's grounds is that band's measured ceiling.
/// - **Ink**: deep night-violet content, violet-grey muted (its low chroma is
///   deliberate — the Constant role tint anchors at 290° and the pairwise
///   role-vs-muted law needs the daylight between them), pale lilac faint.
/// - **Caret**: the first spark of sun — a deeper sunrise amber than the old
///   pyrite (hue ~37°, more present on the paler ground).
/// - **Selection**: pools the night's violet — dawn's cool side, ~135° off
///   the caret's gold.
pub const BILBY: Theme = Theme {
    name: "Bilby",
    dark: false,
    base_100: Srgb::rgb(0xFF, 0xF7, 0xEF),
    base_200: Srgb::rgb(0xFB, 0xE9, 0xDC),
    base_300: Srgb::rgb(0xF6, 0xD9, 0xC6),
    base_content: Srgb::rgb(0x26, 0x20, 0x38),
    muted: Srgb::rgb(0x6B, 0x65, 0x7A),
    faint: Srgb::rgb(0xA7, 0x9D, 0xB6),
    primary: Srgb::rgb(0xBC, 0x7E, 0x16),
    primary_content: Srgb::rgb(0xFD, 0xF4, 0xE2),
    error: Srgb::rgb(0xC0, 0x39, 0x2B),
    selection: Srgb::rgba(0x8F, 0x7B, 0xB8, 0x52),
    // The margin is the horizon itself: a VERTICAL gradient warming downward —
    // cooler pale rose above, rose-gold at the bottom edge, where first light
    // actually lives.
    background: Background::Gradient {
        from: Srgb::rgb(0xFB, 0xE9, 0xDC),
        to: Srgb::rgb(0xF6, 0xD9, 0xC6),
        dir: (0.0, 1.0),
    },
    // Newsreader registers under this exact fontdb family name (it ships as the
    // "16pt" optical-size master), so `Family::Name` must match it verbatim.
    font: "Newsreader 16pt 16pt",
    // Refined display serif → the slab-serif Monaspace Xenon for a literary code page.
    mono: "Monaspace Xenon",
    // Newsreader's display-serif contrast IS its hierarchy — bold would coarsen it.
    heading_bold: false,
    cjk: CJK_JA_SHIPPORI,
    zh_hans: CJK_ZH_HANS_SERIF,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO_SERIF,
    // Literary serif world → EB Garamond fleurons; `***` uses ☙ (EBG has no ⁂).
    ornaments: Ornaments { dash: '❧', star: '☙', underscore: '❦' },
    ornament_face: ORNAMENT_GARAMOND,
    ornament_scale: ORNAMENT_SCALE_FLEURON,
    // Refined editorial serif → refined Renaissance fleuron bullets + the
    // family's third fleuron for level 3 (item 15's per-level rotation).
    bullets: ('❧', '❦', '☙'),
    bullet_scale: BULLET_SCALE_ORNAMENT,
    list_indent_scale: LIST_INDENT_SCALE_WIDE,
    // Pale rose-gold first-light ground → Dawn (the bilby is dawn-active); Newsreader
    // display serif → Refined / Literary; warm horizon → Warm.
    // Curated: shows under Dawn / Refined; opts OUT of Voice (Literary crowded) +
    // Temperature (Warm crowded — Quokka/Galah/Potoroo/Firetail hold the cap).
    tags: ThemeTags { time: Some("Dawn"), register: Some("Refined"), voice: None, temperature: None },
    role_overrides: RoleOverrides::NONE,
    render_caps: RenderCaps {
        // LIGHT-WORLD BORDER (composition round item 6) — a crisp rim carries the
        // card edge off the pale ground. DATA, no code path.
        elevation: Elevation::Bordered,
        // DAWN ROUND: rose-gold horizon ground + night-violet ink landed on the
        // user's word ("rose gold is fine... i like it"). The 1px hairline
        // page frame the round PROPOSED for the light pole was REJECTED by the
        // user's eyes ("the frame is so weird") — Bilby stays frameless; the
        // roster's reserved dark-line-on-light assignment goes back on the
        // shelf for some future light-pole world.
        //
        // SPELL-SQUIGGLE round (user report): Newsreader's tall display-serif
        // row geometry (a generous descent baked into the caret-height box)
        // floated the squiggle noticeably below the true baseline here — a
        // tighter per-world gap (2px less than the shared default at zoom
        // 1.0) pulls it back up. DATA dial, not a code path; every other
        // world stays on `SPELL_UNDERLINE_GAP_DEFAULT`.
        spell_underline_gap: SPELL_UNDERLINE_GAP_DEFAULT - 2.0,
        ..RenderCaps::DEFAULT
    },
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
    // The origin of the serif instinct: Fraunces' wonk + contrast carry it Regular.
    heading_bold: false,
    cjk: CJK_MINCHO,
    zh_hans: CJK_ZH_HANS_SERIF,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO_SERIF,
    // Pale serif world → Junicode's horizontal running-vine Caslon scrolls (a vine + two sibling scrolls).
    ornaments: Ornaments { dash: '\u{F01B}', star: '\u{F01D}', underscore: '\u{F01E}' },
    ornament_face: ORNAMENT_JUNICODE,
    ornament_scale: ORNAMENT_SCALE_ORNATE,
    // Old-style salt-flat at first light → an airy floral-heart + leaf pair +
    // the family's third fleuron for level 3 (item 15's per-level rotation).
    bullets: ('❦', '❧', '☙'),
    bullet_scale: BULLET_SCALE_ORNAMENT,
    list_indent_scale: LIST_INDENT_SCALE_WIDE,
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
    // ITEM 70 — Quokka becomes awl's deliberately playful printed-card world:
    // OFL Sour Gummy (google/fonts ofl/sourgummy; see `docs/fonts.md` +
    // `assets/fonts/LICENSES.md` for the instance/subset provenance),
    // replacing Fira Sans as Quokka's Latin display face only. IBM Plex Mono
    // (code) and the Klee One/LXGW WenKai CJK companions are unchanged below.
    font: "Sour Gummy",
    // Warm friendly humanist sans → the warm humanist IBM Plex Mono for code.
    mono: "IBM Plex Mono",
    // Sour Gummy's real 700 companion (`FONT_THEME_BOLD_FACES`) carries the weight — no blur-into-body risk.
    heading_bold: true,
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
    list_indent_scale: LIST_INDENT_SCALE_PLAIN,
    // Warm peach reef → Dawn (warm-soft light); Fira Sans friendly humanist → Everyday / Modern; peach hue → Warm.
    // Curated: a headliner on ALL four — Dawn / Everyday / Modern / Warm each read clearly on the friendly peach sans.
    tags: ThemeTags { time: Some("Dawn"), register: Some("Everyday"), voice: Some("Modern"), temperature: Some("Warm") },
    role_overrides: RoleOverrides::NONE,
    // LIGHT-WORLD BORDER (composition round item 6) — a crisp rim carries the
    // card edge off the pale ground. DATA, no code path.
    // ITEM 70 — Quokka ALONE assigns the non-default printed-card caps: a
    // small rotated dot lattice (18° — mid of the round's 15-20° spec),
    // strongest at the card's far/right decorative side, rolling off before
    // the left-aligned content-heavy side (`shaders/selection.wgsl`'s
    // `halftone_rolloff`); and a crisp 45° chamfer (11px — mid of the
    // round's 10-12px spec) replacing the small rounded corner on every
    // eight-edge card boundary. Both taste values are the implementer's
    // first pick, captured for Fable's veto pass (see the round's own
    // captures) — not yet a graduated user sign-off.
    render_caps: RenderCaps {
        elevation: Elevation::Bordered,
        card_texture: CardTexture::HalftoneDots { angle_deg: 18.0, cell_px: 8.0, density: 0.30 },
        card_shape: CardShape::Chamfered { cut_px: 11.0 },
        ..RenderCaps::DEFAULT
    },
};

/// Bombora — the wave standing over a submerged reef: a violet-dark midnight
/// swell (hot indian-lake caret cresting the deep water).
pub const BOMBORA: Theme = Theme {
    name: "Bombora",
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
    // WAVE-TIERS (item 69) — three stacked, non-overlapping shallow wave tiers
    // (wide scalloped crests, phase-offset so they layer) replacing the static
    // starfield. ONLY the violet ground ladder — `base_100`/`base_200`/
    // `base_300` verbatim — top tier to bottom. Mulga stays the roster's sole
    // shipping `Starfield` world.
    background: Background::Waves {
        tones: [
            Srgb::rgb(0x15, 0x0A, 0x2C),
            Srgb::rgb(0x24, 0x15, 0x40),
            Srgb::rgb(0x3C, 0x36, 0x54),
        ],
    },
    // EB Garamond — a classic Garamond serif; distinct from Bilby's Newsreader
    // so the two share no face.
    font: "EB Garamond",
    // Classic Garamond serif nocturne → Monaspace Xenon: a refined slab-serif mono
    // for a literary code page.
    mono: "Monaspace Xenon",
    // EB Garamond's old-style modelling carries hierarchy; its bold reads foreign to the page.
    heading_bold: false,
    cjk: CJK_JA_SHIPPORI,
    zh_hans: CJK_ZH_HANS_SERIF,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO_SERIF,
    // OVERRIDE (the serif nocturne's flourish): mirror the default fleuron into its
    // reversed twin ☙ for `---`, and swap `___`'s heart to the black-heart bullet ❥
    // (both NS2 ornament variants, also bundled). `***` keeps the ⁂ asterism.
    // IN-FACE: Bombora's display IS EB Garamond, so its fleuron shapes in its own
    // face. The old {☙,⁂,❥} relied on the merged marks face (EBG has no ⁂/❥); the
    // set is now all-EBG fleurons (☙ dash keeps its distinct reversed look).
    ornaments: Ornaments { dash: '☙', star: '❧', underscore: '❦' },
    ornament_face: ORNAMENT_GARAMOND,
    ornament_scale: ORNAMENT_SCALE_FLEURON,
    // Classical literary midnight → the antique MANICULE ☞ (the medieval margin-
    // pointing hand, native to EB Garamond) at level 1, a hedera at level 2. The
    // one world that gets the manicule — a pointing hand on every bullet is loud,
    // so it rides the top level alone. The showpiece pick; flagged for veto.
    // Level 3 (item 15's per-level rotation) draws EB Garamond's remaining
    // fleuron, NEVER the hand again.
    bullets: ('☞', '❧', '❦'),
    // PADDING FIX (theme-QA round): the manicule's own ink is unusually WIDE for
    // a bullet glyph — at the shared [`BULLET_SCALE_ORNAMENT`] tier its right
    // edge reached (and on some rows touched) the list text that follows, since
    // EB Garamond's narrow `"- "` marker+space advance leaves it little room
    // (measured: real-pixel glyph-to-text gap went negative — see
    // `render::tests::markdown::bullet_glyph_never_touches_the_following_text_in_any_world`).
    // A dedicated, smaller-than-the-shared-tier literal (rather than retuning
    // [`BULLET_SCALE_ORNAMENT`] for every characterful world) keeps every other
    // hedera/fleuron world's bullet byte-identical.
    bullet_scale: 0.35,
    list_indent_scale: LIST_INDENT_SCALE_WIDE,
    // Dark violet current → Night; EB Garamond classic serif → Refined / Literary; violet-blue hue → Cool.
    // Curated: shows under Night / Refined / Literary (the classical serif's home); opts OUT of Temperature (Cool crowded).
    tags: ThemeTags { time: Some("Night"), register: Some("Refined"), voice: Some("Literary"), temperature: None },
    role_overrides: RoleOverrides::NONE,
    render_caps: RenderCaps::DEFAULT,
};

/// Mulga — the arid acacia scrub whose dark olive IS the room (hays-russet caret
/// in a blackish-olive night).
pub const MULGA: Theme = Theme {
    name: "Mulga",
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
    // Zilla Slab's chunky slab serifs already assert structure — Regular keeps it calm.
    heading_bold: false,
    cjk: CJK_MINCHO,
    zh_hans: CJK_ZH_HANS_SERIF,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO_SERIF,
    // Slab world → austere typographic Junicode marks (⁂ asterism + ⁑ + ❦ floral heart).
    ornaments: Ornaments { dash: '⁂', star: '⁑', underscore: '❦' },
    ornament_face: ORNAMENT_JUNICODE,
    ornament_scale: ORNAMENT_SCALE_ORNATE,
    // Slab-sturdy literary night → reversed leaf + floral heart (distinct from its
    // ⁂/⁑ asterism section trio) + the family's third fleuron for level 3
    // (item 15's per-level rotation).
    bullets: ('☙', '❦', '❧'),
    bullet_scale: BULLET_SCALE_ORNAMENT,
    list_indent_scale: LIST_INDENT_SCALE_WIDE,
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
    // Plex Mono's Light-300 body makes the 700 head a real jump — mono needs the weight.
    heading_bold: true,
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
    list_indent_scale: LIST_INDENT_SCALE_PLAIN,
    // Warm-grey neutral nocturne → Night; IBM Plex Mono → Humble / Technical; near-neutral grey → Neutral.
    // Curated: shows under Humble / Neutral (its plainest traits); opts OUT of Time (Night crowded) + Voice (Technical crowded).
    tags: ThemeTags { time: None, register: Some("Humble"), voice: None, temperature: Some("Neutral") },
    role_overrides: RoleOverrides::NONE,
    render_caps: RenderCaps::DEFAULT,
};

/// Mopoke — Tawny warmed a notch: the cool near-black neutrals nudged to a warm
/// charcoal so the room reads cosy, not void. Same IBM Plex Mono home.
///
/// TAWNY↔MOPOKE DIFFERENTIATION (DECIDED Option A, user + fable, 2026-07-22):
/// the two worlds shipped with a BYTE-IDENTICAL caret (`#FFC05E`) and selection
/// (`#3A6FD8`) — a gallery round measured the pair's whole-palette RMS redmean
/// at only 24.6, awl's tightest. Mopoke (never Tawny, the anchor) moves both:
/// the caret golden→copper/ember (hue ~16°, deeper/redder than the gallery's
/// `#F78645` exploration — still warm, still legible, stops short of pure
/// red), and the selection blue→violet-plum `#7B39C6` at the same `0x52`
/// alpha. The pair's RMS now measures ~76
/// (`tawny_and_mopoke_carets_and_selections_are_now_numerically_distinct`,
/// `theme::tests`). `primary_content` stays the shared dark-warm ink
/// (`#261A08`) — it's an authored token, not derived from `primary`, and
/// reads fine under either hue (it's also render-inert here: Mopoke's block
/// caret is `CaretBlockStyle::Normal`, which never repaints the covered
/// glyph in `primary_content` — that knockout is the `Filled` arm's alone).
/// Fable: the plum selection is the felt workhorse (it harmonizes with
/// Mopoke's warm ground, keeps its cosy character); copper+plum are warmer
/// jewellery on the same room, not a new world.
pub const MOPOKE: Theme = Theme {
    name: "Mopoke",
    dark: true,
    base_100: Srgb::rgb(0x1B, 0x18, 0x14),
    base_200: Srgb::rgb(0x25, 0x21, 0x1B),
    base_300: Srgb::rgb(0x31, 0x2B, 0x22),
    base_content: Srgb::rgb(0xE8, 0xE4, 0xDC),
    muted: Srgb::rgb(0x97, 0x8C, 0x7E),
    faint: Srgb::rgb(0x57, 0x50, 0x47),
    // Copper/ember (hue ~16°, sat 0.90, light 0.60) — was Tawny's shared gold
    // `#FFC05E`; see the doc comment above for the differentiation round.
    primary: Srgb::rgb(0xF5, 0x6E, 0x3D),
    primary_content: Srgb::rgb(0x26, 0x1A, 0x08),
    error: Srgb::rgb(0xE5, 0x4B, 0x4B),
    // Violet-plum, same 0x52 alpha as before — was Tawny's shared blue `#3A6FD8`.
    selection: Srgb::rgba(0x7B, 0x39, 0xC6, 0x52),
    background: Background::Dots {
        from: Srgb::rgb(0x1B, 0x18, 0x14),
        to: Srgb::rgb(0x25, 0x21, 0x1B),
        dir: (0.0, 1.0),
        tint: Srgb::rgb(0x33, 0x2D, 0x24),
        edge: false,
    },
    // Bitter — a warm, screen-bred slab serif; breaks up the mono darks with a
    // cosy reading face (shared with Magpie's stark-paper masthead — face-sharing
    // is precedented; Tawny keeps IBM Plex Mono as its signature, Potoroo takes
    // Monaspace Xenon). Body face swap: queue item 30 (user + fable, 2026-07-23).
    font: "Bitter",
    // Warm cosy charcoal → the warm humanist IBM Plex Mono (kin to Tawny's home look).
    mono: "IBM Plex Mono",
    // Bitter's slab weight sections cleanly — the bundled Bitter-Bold carries headings.
    heading_bold: true,
    cjk: CJK_JA_KLEE,
    zh_hans: CJK_ZH_HANS_KLEE,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Cosy expressive world → Junicode's ornate Caslon damask flourishes (a damask + candelabra + damask tile).
    ornaments: Ornaments { dash: '\u{E670}', star: '\u{F011}', underscore: '\u{F014}' },
    ornament_face: ORNAMENT_JUNICODE,
    ornament_scale: ORNAMENT_SCALE_ORNATE,
    // BULLET TRIPLE (queue item 30, user + fable): one ornament register whose
    // WEIGHT descends with depth — E670 a solid damask rosette (level 1, the very
    // glyph Mopoke's `---` ornament already draws at ORNATE scale), EF92 its open
    // four-fold sibling (level 2), E67D a small foliate sprig (level 3). All three
    // resolve in Mopoke's ornament face (Junicode) and read as one family, quiet →
    // quieter with depth (verified present + non-touching by the two
    // `render::tests::markdown` bullet laws).
    bullets: ('\u{E670}', '\u{EF92}', '\u{E67D}'),
    // Back on the shared [`BULLET_SCALE_ORNAMENT`] tier: the old off-tier 0.8 was
    // tuned to fill a CANYON left by iA Writer Quattro S's WIDE duospaced `"- "`
    // advance; Bitter is proportional with a far narrower marker advance (exactly
    // Magpie's Bitter-body case, which already rides the shared tier), so the
    // rosette sits right at half-body with no canyon and no touch (see
    // `render::tests::markdown::bullet_glyph_never_touches_the_following_text_in_any_world`).
    bullet_scale: BULLET_SCALE_ORNAMENT,
    list_indent_scale: LIST_INDENT_SCALE_WIDE,
    // Warm charcoal cosy dark → Dusk (warm dark); warm slab-serif reading face → Humble (cosy, unpretentious); warm hue → Warm.
    // Curated: shows under Dusk / Humble (its cosy core); opts OUT of Voice (Bitter's Literary slot is Magpie's) + Temperature (Warm crowded).
    tags: ThemeTags { time: Some("Dusk"), register: Some("Humble"), voice: None, temperature: None },
    role_overrides: RoleOverrides::NONE,
    render_caps: RenderCaps::DEFAULT,
};

/// Bowerbird — a deep midnight-navy dark world: the satin bowerbird's glossy
/// blue-black planes under a cool off-white ink, lit by ONE warm-amber caret —
/// the thesis made literal, the single warm thing in a cool room (DESIGN §3),
/// like the one bright treasure hoarded in a blue-black bower. Drawn in IBM Plex
/// Sans to set it apart from Tawny's mono family — a clean sans nocturne.
pub const BOWERBIRD: Theme = Theme {
    name: "Bowerbird",
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
    // Plex Sans' even grotesque strokes give size little help — weight does the sectioning.
    heading_bold: true,
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
    list_indent_scale: LIST_INDENT_SCALE_PLAIN,
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
    // Iosevka's narrow mechanical grid is all uniform strokes — weight marks the head.
    heading_bold: true,
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
    list_indent_scale: LIST_INDENT_SCALE_PLAIN,
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
    // TWINKLING STARS (2026-07-18, the user's morning verdict): Currawong stays,
    // differentiated by ambient TWINKLING STARS — the "aliveness ≠ loudness"
    // pole (maximally quiet, unmistakably alive; the Pied Currawong's voice is
    // the quiet dark). Tiny starlight points scattered sparsely through the page
    // margins, each on its own slow seconds-scale LIFECYCLE (2026-07-23): a dark
    // dwell at true zero -> rise -> brief shine -> fade, so the visible sky
    // genuinely changes (stars appear and die). Per-star tint from a low-sat
    // real-star palette (cool blue-white #9BB0D2 dominant, ~217°/~172° clear of
    // the gold caret; plus a neutral white and a subtle champagne — the amber
    // guard holds by low saturation, `crate::stars::star_palette`). A star's
    // shine may now rise ABOVE the muted whisper cap (a user-blessed relaxation
    // of the quiet-band law) but stays well under the text ink. All numbers are
    // TASTE TUNABLE — the twinkle FEEL and shine ceiling are live human-confirm.
    // CHROMA + SIZE (item 62, 2026-07-24, user-decided): the palette carries
    // ~10% more chroma than it shipped with (was #9DB0CF; every entry moved at
    // NO GREATER luminance — a richer sky, never a brighter one, see
    // `crate::stars::star_palette`'s doc and its measured-delta law test).
    // Champagne alone stays capped WELL under a full +10% (its hue sits only
    // ~5° from the gold caret, so ANY saturation above the amber guard's 0.15
    // exemption line would make it a second accent — the guard bounds how much
    // chroma this one entry may take, `STAR_TINT_CHAMPAGNE`'s own doc). Each
    // star's dot size also now spreads mildly around `size_px` by a hash roll
    // off its own seed (`crate::stars::star_size_scale`) — deterministic, no
    // new clock, no density change.
    render_caps: RenderCaps {
        elevation: Elevation::Bordered,
        card_anchor: CardAnchor::TopLeft,
        ambient: AmbientStyle::Stars {
            // +10.8% saturation over the shipped #9DB0CF at Y -0.12% (never
            // brighter) — see the module doc above.
            tint: Srgb::rgb(0x9B, 0xB0, 0xD2),
            cell_px: 34.0,
            // Denser candidate field than the old always-on breath (0.16): the
            // LIFECYCLE round leaves ~half the field dark-dwelling at any moment,
            // so the AUTHORED density is raised to keep the VISIBLE population
            // healthy as stars appear and die.
            density: 0.30,
            size_px: 2.6,
            // THE VISIBILITY BAND [floor, peak] is now the per-star SHINE-peak
            // range (each star glints to its own level here, then dies to true
            // zero — not a breath's floor). `peak` is the calm CEILING (a glint
            // may now rise ABOVE the muted whisper cap — the user-blessed
            // relaxation of the ambient quiet-band law — but stays well under the
            // text ink); `floor` is a real visible floor (the dimmest lit star is
            // still seeable). TASTE TUNABLE — the shine ceiling's FEEL is a live
            // human-confirm.
            peak: 0.5,
            floor: 0.18,
        },
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
    // warm-grey Tawny (redmean 15.2) and blackish-olive Mulga (16.6); the deeper
    // teal separates cleanly (Tawny →32, Mulga →40, Bowerbird →36) and makes
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
    // JetBrains Mono's uniform coding strokes need weight to lift a section head.
    heading_bold: true,
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
    list_indent_scale: LIST_INDENT_SCALE_PLAIN,
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
            // (item-45 fable pick flipped this world RIGHT: TopRight → bottom-LEFT)
            // — a balanced diagonal, poster off the card.
            corner: PlacardCorner::Auto,
            scale: 3.0,
            ink: PlacardInk::Stipple,
        },
        card_anchor: CardAnchor::TopRight,
        elevation: Elevation::Bordered,
        // FLIP ROUND (user FINAL PICKS 2026-07-17): a poster/statement world →
        // the Bars HUG-ALL HYBRID (label-hug plate + bare right-aligned chords,
        // `BarExtent::HugLabel`) at the gate's MID radius (6), every row a bar.
        // Facet chips = BRACKET (the terminal-register corner ticks — the
        // technical/cool voice's own frame; user's confirmed chip map 2026-07-17).
        list_style: POSTER_BARS,
        facet_style: FacetStyle::Chips(ChipVariant::Bracket),
        // ITEM 65 taste correction (Fable's audit): `LavaEdge::Glow`'s own
        // "soft light-spill under the column" lifts the WHOLE writing column
        // off flat `base_100` — the fold chevron/tail's bare `muted`/`faint`
        // rung measured only ~1.5:1 / ~1.4:1 against the ACTUAL rendered
        // ground (a screenshot pixel probe: `(0x49,0x6D,0x68)` at rest, far
        // brighter than `base_100` `(0x11,0x27,0x23)`). Both lifted toward
        // `base_content` — chevron 0.60 (→ ~3.0:1), tail 0.75 (→ ~3.1:1; a
        // shallower lift dips BELOW 1:1 first — `faint` and the lit ground
        // start almost EQUAL in luminance, so the lerp crosses a near-
        // invisible trough before climbing back out the other side; 0.75
        // clears it) — calibrated against the real rendered ground, not
        // theoretical `base_100`. See [`theme::model::FoldAfford`]'s own doc.
        fold_afford: FoldAfford { chevron_lift: 0.60, tail_lift: 0.75 },
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
    // Figtree's geometric sans is stroke-uniform by design — weight does the sectioning.
    heading_bold: true,
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
    list_indent_scale: LIST_INDENT_SCALE_PLAIN,
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
    // Bitter's sharp slab contrast carries hierarchy on its own — Regular stays sharp.
    heading_bold: false,
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
    // ornament subset lacks ☞ — hederas instead; see the round report.) Level 3
    // (item 15's per-level rotation) draws the family's third fleuron.
    bullets: ('❦', '☙', '❧'),
    bullet_scale: BULLET_SCALE_ORNAMENT,
    list_indent_scale: LIST_INDENT_SCALE_WIDE,
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

/// Brolga — the SEVENTEENTH world, and the COOL LIGHT POLE: a clear cool sky
/// after rain — pale periwinkle sky-blue, washed clean — with the brolga crane's
/// one red-crown spark at the caret.
///
/// WHY IT EXISTS: the DAWN round (2026-07-18) retuned Bilby out of its old
/// pale-BLUE day room into a warm rose-gold dawn, vacating the cool-light-blue
/// pole — the roster then had NO light world on the blue side (Bilby/Saltpan/
/// Galah warm, Magpie neutral, and the only cool light world, Gumtree, is
/// GREEN). Brolga fills that hole deliberately and is built NOT to read as a
/// resurrection of the old Bilby cyan (#E8FAFF, retired) nor as Gumtree's
/// sibling (the exact near-pair trap the dawn round fixed): a clean cool SANS
/// (IBM Plex Sans) on a pale periwinkle-blue ground, where Gumtree is a cool
/// green SERIF. The brolga is a tall grey-blue wetland crane with a vivid red
/// crown; its world is the pale blue of a clear sky reflected in still shallow
/// water, and its one warm living thing (DESIGN §3) is the crane's red crown at
/// the caret.
///
/// - **Ground**: pale periwinkle sky-blue (`base_100` #E9EFFB, WCAG relY ~0.86)
///   — its own point in the crowded pale band: measured ≥35.7 redmean from every
///   surviving light ground (min vs Galah; the warm/neutral pales sit far off in
///   hue and the blue pole was empty), well past the dawn round's ~18.8 pale-band
///   ceiling. A calm vertical `Gradient` margin (a clear sky), no pattern noise.
/// - **Ink**: a deep cool slate-navy content receding through slate-blue-grey
///   `muted` to a pale blue-grey `faint` — the clear cool sky carried into the
///   ink ladder.
/// - **Caret**: the brolga's red crown — a warm coral-vermilion (hue ~10°), the
///   one warm spark in the cool room, ≥80° of hue clear of every syntax-role
///   anchor (the amber guard holds).
/// - **Selection**: pools the sky's blue in still water — a deep cornflower
///   tint, cool and well clear of the coral caret.
pub const BROLGA: Theme = Theme {
    name: "Brolga",
    dark: false,
    base_100: Srgb::rgb(0xE9, 0xEF, 0xFB),
    base_200: Srgb::rgb(0xDC, 0xE6, 0xF8),
    base_300: Srgb::rgb(0xC7, 0xD7, 0xF2),
    base_content: Srgb::rgb(0x1B, 0x24, 0x36),
    muted: Srgb::rgb(0x58, 0x63, 0x7A),
    faint: Srgb::rgb(0x99, 0xA3, 0xB6),
    // The brolga's red crown — a warm coral-vermilion, the one warm thing.
    primary: Srgb::rgb(0xD7, 0x5B, 0x41),
    primary_content: Srgb::rgb(0xFC, 0xEE, 0xEA),
    error: Srgb::rgb(0xC0, 0x39, 0x2B),
    // A deep cornflower tint — the sky pooled in still water, cool, well clear
    // of the coral caret. Alpha 0x60 (like Bombora/Mangrove) so the composited
    // band clears the selection contrast floor over the pale blue ground.
    selection: Srgb::rgba(0x35, 0x57, 0xA0, 0x60),
    // A calm vertical gradient — the clear sky over still water — deepening
    // downward from the pale plane to the recessed margin blue.
    background: Background::Gradient {
        from: Srgb::rgb(0xDC, 0xE6, 0xF8),
        to: Srgb::rgb(0xC7, 0xD7, 0xF2),
        dir: (0.0, 1.0),
    },
    // IBM Plex Sans — awl's cool humanist sans, now worn at BOTH value poles:
    // dark Bowerbird's midnight navy and Brolga's pale sky. A clean cool sans
    // sets it apart from the only other cool LIGHT world (Gumtree, a green serif).
    font: "IBM Plex Sans",
    // Cool clean sans → its own type-family kin, the humanist IBM Plex Mono for
    // the code grid (the Plex superfamily; distinct from Bowerbird's JetBrains).
    mono: "IBM Plex Mono",
    // Plex Sans' even grotesque strokes give size little help — weight sections.
    heading_bold: true,
    cjk: CJK_GOTHIC,
    zh_hans: CJK_ZH_HANS_SANS,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Cool clean sky → the merged marks' airy star/diamond trio (✧ open star +
    // ✴ sparkle + ⬥ diamond) — a clear-sky sparkle over still water.
    ornaments: Ornaments { dash: '✧', star: '✴', underscore: '⬥' },
    ornament_face: ORNAMENT_MARKS,
    ornament_scale: ORNAMENT_SCALE_GEOMETRIC,
    // Clean cool sky → plain geometric bullets (unfussy restraint).
    bullets: BULLETS_PLAIN,
    bullet_scale: BULLET_SCALE_PLAIN,
    list_indent_scale: LIST_INDENT_SCALE_PLAIN,
    // Clear cool daylight sky → Day (roomy — Gumtree/Magpie); pale blue → Cool
    // (its defining trait — joins Gumtree/Bowerbird/Mangrove as the 4th, at the
    // curated cap). Opts OUT of Register + Voice (both already at their 3-world
    // bands) — reachable via All + fuzzy search regardless.
    tags: ThemeTags { time: Some("Day"), register: None, voice: None, temperature: Some("Cool") },
    role_overrides: RoleOverrides::NONE,
    // LIGHT-WORLD BORDER (composition round item 6) — a crisp rim carries the
    // summoned card's edge off the pale ground. DATA, no code path. The DAWN
    // round's reserved dark-line-on-light PAGE FRAME is deliberately NOT taken:
    // the user's live verdict on Bilby's 1px frame was "the frame is so weird"
    // on a light world, so Brolga stays frameless too.
    render_caps: RenderCaps { elevation: Elevation::Bordered, ..RenderCaps::DEFAULT },
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
    // A 1-bit world has NO ink rungs to spend — weight is the only second axis it owns.
    heading_bold: true,
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
    list_indent_scale: LIST_INDENT_SCALE_PLAIN,
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
        // SPLIT-PANE COMPOSITION round: the silent pole takes the DEFAULT split
        // like every other Pane world (only Cassowary opts back to `Unified`).
        // On the 1-bit ground the two surfaces read by their crisp white
        // `Bordered` rims (base_300 == base_100 = black), a stacked-panel
        // composition rather than a colour step. Listed explicitly because this
        // literal names every field (no `..DEFAULT` spread).
        pane_split: PaneSplit::Split,
        // TWINKLING-STARS round: NO ambient life — a fractional-alpha star
        // breath is structurally illegal on a true 1-bit world (any
        // intermediate composite is a forbidden third value; the theme-side
        // law `ambient_stars_laws_hold_for_every_world` guards it), and the
        // silent pole would decline the personality anyway.
        ambient: AmbientStyle::None,
        // SPELL-SQUIGGLE round: the silent pole keeps the default gap — its
        // display face isn't the tall-serif shape Bilby's dial compensates
        // for. Listed explicitly because this literal names every field (no
        // `..DEFAULT` spread).
        spell_underline_gap: SPELL_UNDERLINE_GAP_DEFAULT,
        // FROST-AS-CAPABILITY round: dormant DATA on the silent 1-bit pole — the
        // recipe never renders (no lava ground; the consumer gates on the lava
        // capability, so the 1-bit exclusion stays structural, not a world name).
        // Listed explicitly because this literal names every field (no `..DEFAULT`).
        frost: Frost::DEFAULT,
        // ITEM 65: the silent 1-bit pole carries no lava ground (its column
        // stays flat), so both lifts are inert DATA — `0.0`/`0.0`, the bare
        // ladder rung, which is already 1-bit-legal (`faint`/`muted` collapse
        // to `base_content` on a true 1-bit world — `Theme::is_one_bit`'s own
        // doc). Listed explicitly because this literal names every field (no
        // `..DEFAULT` spread).
        fold_afford: FoldAfford::DEFAULT,
        // ITEM 70: the silent 1-bit pole carries no printed-card material — a
        // fractional-alpha halftone dot would be exactly the forbidden
        // intermediate value the 1-bit law bans, and a chamfered card is
        // Quokka's own separate personality statement. Listed explicitly
        // because this literal names every field (no `..DEFAULT` spread).
        card_texture: CardTexture::DEFAULT,
        card_shape: CardShape::DEFAULT,
    },
};

/// Firetail — the SIXTEENTH world, a WARM STATEMENT world and awl's FIRST
/// lava-lamp ground: the MIRROR of Wagtail. Where Wagtail keeps NO warm thing
/// (its statement is the bare 1-bit room), Firetail's one living warm thing is
/// the GROUND ITSELF — a slow oxblood/wine metaball "lava lamp" bobbing in the
/// page margins (see [`Background::Lava`] + `crate::lava`), the DESIGN.md §3
/// ambient-motion amendment's first host (Mangrove is the cool second). The
/// room is its own deep oxblood-charcoal den — redder beside Bombora's violet,
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
    // The poster world's mono display: uniform slab-mono strokes take the bold head.
    heading_bold: true,
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
    list_indent_scale: LIST_INDENT_SCALE_PLAIN,
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
        // ITEM 65 taste correction (Fable's audit): Firetail's own CHEVRON
        // already reads fine at `muted`'s bare rung against the glow-lit
        // column (measured ~2.9:1) — left at `0.0`, untouched. Its TAIL did
        // not (measured ~1.3:1 against the real rendered ground, a
        // screenshot pixel probe: `(0x55,0x35,0x3D)`, far brighter than
        // `base_100` `(0x17,0x09,0x0C)`) — lifted 0.40 toward `base_content`
        // (→ ~3.2:1). See [`theme::model::FoldAfford`]'s own doc.
        fold_afford: FoldAfford { chevron_lift: 0.0, tail_lift: 0.40 },
        ..RenderCaps::DEFAULT
    },
};

/// Cassowary — a NERV operations terminal: phosphor-green data on near-black
/// glass, a lit CRT block cursor in that same phosphor, and a warning-red alert
/// channel.
///
/// **The register (WORLDS.md's flavour sentence):** *"The MAGI bridge after dark
/// — green terminal data on black glass, a lit phosphor block where you sit, red
/// only when something is wrong."* The cassowary is the roster's armoured,
/// prehistoric, casque-helmeted dangerous bird ("the world's most dangerous
/// bird", a living dinosaur) — glossy BLACK plumage, a red wattle, an electric
/// blue-green neck: the black-ground / green-data / red-warning palette is the
/// creature's own colouring, and its armoured-menace character is the mecha wink
/// without cosplay.
///
/// **The accent resolution (the board's named core problem — the user picked
/// PHOSPHOR).** awl's ONE accent is the caret (`primary`; DESIGN §3). Most worlds
/// spend it on amber; Cassowary spends it on the terminal's OWN phosphor GREEN —
/// the caret IS the ink's colour (`primary == base_content`, an INK CARET —
/// `Theme::ink_caret`), drawn as the authentic CRT block cursor: a lit
/// `primary`-green cell with the covered glyph knocked out in the GROUND colour
/// (`CaretBlockStyle::Filled`; `primary_content` is set to the black glass). This
/// is the generalized WAGTAIL precedent — Wagtail's caret is its own white ink,
/// presence carried by INVERSION not a hue accent — so an ink caret carries no
/// separate accent hue, and is exempt from the amber-guard's ≥30° role gap (that
/// guard exists so no syntax tint steals the caret's accent; it is moot when the
/// caret HAS no accent hue). The exemption is law-pinned to the required
/// inverting/filled block (`role_style_laws_hold_for_every_world` (e)), so the
/// green ink-ladder tints (Str ~140° among them) stay mutually distinguishable ON
/// the green ink by VALUE. RED stays the ERROR/ALERT channel alone (`error`, and
/// the warning-crimson `selection`). A clean split — green = the terminal (the
/// data AND the cursor you type at), red = alert. Ibeam mode is the clean thin
/// green bar; morph folds to the filled block.
///
/// **Face + heading.** Iosevka, the narrowest, most mechanical bundled mono — the
/// literal terminal-readout face — as both display and code (already mono, so
/// code reuses it). Uniform mono strokes need weight to lift a section head →
/// `heading_bold: true` (Iosevka ships a real Bold). The summoned overlay speaks
/// the LOUD NERV monolith voice: `chrome_face = Archivo Black` (the heavy
/// grotesque, registers at usWeightClass 400 so the plain chrome request matches
/// it) on the placard wordmark / title prefix / lens-strip — while the writing
/// column and list rows stay Iosevka (the closed chrome-surface set; legibility
/// surfaces never change face). The WRITING page stays calm green-on-black; the
/// drama is transient, only when you summon a command (the NERV console appears)
/// — exactly DESIGN §5's "transient summoned overlays, never persistent chrome".
///
/// **Ground.** `Pinstripe` — fine parallel dim-green lines in the page-mode
/// margins: CRT scan-lines, the terminal register, marginal and calm (the page
/// column stays the flat figure). Every taste number is HOLD-flagged for the
/// user's gallery pick.
pub const CASSOWARY: Theme = Theme {
    name: "Cassowary",
    dark: true,
    // BLACKGLASS ground (2026-07-18 variant, serving the user's "a bit similar to
    // Mulga no?" — both the old green-cast ground and Mulga's blackish-olive
    // read as dark GREEN rooms). Neutralised base_100/base_200 to a near-neutral
    // black GLASS (a powered CRT at rest: a hair cool, essentially achromatic —
    // sat drops 0.38 -> 0.09), so the page field is no longer a green room. The
    // green now lives ONLY where it means "terminal data": the phosphor INK, the
    // dim green PANEL/wash (base_300, below), the string wash, and the margin
    // scan-line TINT (Pinstripe, below). Vs Mulga redmean 48.8 -> 59.5, and the
    // saturation collapse is the real separation (0.09 vs Mulga's 0.35). NOTE:
    // this lands base_100 within ~3 redmean of Currawong's neutral OLED near-black
    // (#060607) — no law enforces pairwise-ground distinctness, and the two worlds
    // diverge hard elsewhere (green phosphor ink + green scan-line margins + crimson
    // selection here vs Currawong's neutral ink + twinkling-star margins).
    // ORIGINAL green-cast hexes (easy revert): base_100 #050B07, base_200 #0A160F.
    base_100: Srgb::rgb(0x05, 0x05, 0x06),
    base_200: Srgb::rgb(0x0B, 0x0C, 0x0D),
    // The focused plane STAYS a dim terminal-green panel — deliberately KEPT green
    // through the blackglass neutralisation: base_300 is the summoned NERV console
    // CARD fill + the surface-ramp step, i.e. the "wash step" where terminal
    // content sits. Keeping it green means the writing page reads as black glass
    // while the transient summoned overlay reads as a green console panel (DESIGN
    // §5's "the drama is transient" — you get the green terminal exactly when you
    // summon a command). Sat 0.38, well clear of the neutral page field above.
    base_300: Srgb::rgb(0x14, 0x2C, 0x1E),
    // Phosphor green data — bright enough to read as CRT phosphor, pale/soft
    // enough for long prose (a saturated mid-green body fatigues; this pale
    // phosphor gives the role tints their derivation room besides).
    base_content: Srgb::rgb(0xA8, 0xEC, 0xBE),
    muted: Srgb::rgb(0x5C, 0x9E, 0x70),
    faint: Srgb::rgb(0x37, 0x63, 0x4A),
    // THE PHOSPHOR CARET (the user's pick, 2026-07-18): the caret is the ink's OWN
    // phosphor green — `primary == base_content` (#A8ECBE), an INK CARET. It draws
    // as an authentic CRT block cursor via `CaretBlockStyle::Filled` (render_caps
    // below): a lit green cell with the covered glyph knocked out in the ground
    // colour. No separate accent HUE, so it is amber-guard-exempt (see the doc
    // above + `role_style_laws_hold_for_every_world` (e)); its findability is the
    // block fill (redmean ~605 vs the black glass), not a colour step off the ink.
    primary: Srgb::rgb(0xA8, 0xEC, 0xBE),
    // Ink-on-accent = the GROUND (the black glass, == base_100): the Filled block
    // knocks the covered glyph out in THIS colour, so a lit green cell reads with
    // the letter punched through in black glass — the terminal cursor.
    primary_content: Srgb::rgb(0x05, 0x05, 0x06),
    // The NERV warning red — the alert channel (spell-squiggle / failure signal),
    // a hot "PATTERN" red that only ever means something is wrong.
    error: Srgb::rgb(0xFF, 0x44, 0x36),
    // A dim warning-CRIMSON selection wash (~348°, the "target-lock" band) — the
    // world's "red on black" identity lives HERE + in `error`, never on the caret.
    // Higher alpha than the calm worlds so the crimson clears the selection
    // contrast floor over the near-black ground (redmean ≥150).
    selection: Srgb::rgba(0xD2, 0x45, 0x5F, 0x70),
    // CRT SCAN-LINES: fine parallel dim-green lines in the page-mode margins (the
    // terminal register), gradient base_100 → base_200 (now the BLACKGLASS ground,
    // so the from/to track it), marginal and calm. The scan-line TINT stays green
    // (#1E4A32) — the phosphor register is the green identity in the margin, drawn
    // OVER the neutral black glass. ORIGINAL from/to (easy revert): #050B07 / #0A160F.
    background: Background::Pinstripe {
        from: Srgb::rgb(0x05, 0x05, 0x06),
        to: Srgb::rgb(0x0B, 0x0C, 0x0D),
        dir: (0.0, 1.0),
        tint: Srgb::rgb(0x1E, 0x4A, 0x32),
    },
    // Iosevka — the narrow mechanical terminal-readout face, display AND code.
    font: "Iosevka",
    mono: "Iosevka",
    // Iosevka's uniform mechanical strokes need weight to mark a section head.
    heading_bold: true,
    cjk: CJK_GOTHIC,
    zh_hans: CJK_ZH_HANS_SANS,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    // Technical terminal → the merged marks' hazard/alert trio (◆ hazard diamond +
    // ✴ eight-spoke alert star + ◈ diamond-with-centre), three distinct geometrics.
    ornaments: Ornaments { dash: '◆', star: '✴', underscore: '◈' },
    ornament_face: ORNAMENT_MARKS,
    ornament_scale: ORNAMENT_SCALE_GEOMETRIC,
    // Stark terminal → plain geometric bullets (restraint is its character).
    bullets: BULLETS_PLAIN,
    bullet_scale: BULLET_SCALE_PLAIN,
    list_indent_scale: LIST_INDENT_SCALE_PLAIN,
    // NERV bunker terminal → Night; Iosevka mechanical mono → Technical. Opts OUT
    // of Register + Temperature (both crowded near their cap, and to leave room for
    // the concurrent roster growth) — headlines Night + Technical, its clearest reads.
    tags: ThemeTags { time: Some("Night"), register: None, voice: Some("Technical"), temperature: None },
    role_overrides: RoleOverrides::NONE,
    // THE NERV CONSOLE (a statement/poster world — the summoned overlay goes loud
    // while the writing page stays a calm green terminal): a bold Archivo-Black
    // NERV wordmark placard (Auto corner → complementary to the TopLeft card),
    // BORDERED elevation (a hard-edged console card over the black), the poster
    // Bars list (per-row console plates), and BRACKET facet chips (terminal
    // corner-ticks — "the terminal register", its own doc's words). Card anchored
    // TopLeft (a deliberate object, opening the opposite corner for the wordmark).
    render_caps: RenderCaps {
        // THE AUTHENTIC CRT PHOSPHOR CURSOR: `primary == base_content` (an ink
        // caret), so a plain opaque block would erase the letter green-on-green.
        // `Filled` draws the lit green cell + knocks the glyph out in the ground
        // (`primary_content`) — never the `InverseVideo` photo-negative (which on a
        // chromatic ink flips green → magenta). Morph folds to this block; Ibeam
        // stays the clean thin green bar.
        caret_block_style: CaretBlockStyle::Filled,
        title_style: TitleStyle::Placard {
            corner: PlacardCorner::Auto,
            scale: 3.0,
            ink: PlacardInk::Bold,
        },
        card_anchor: CardAnchor::TopRight,
        chrome_face: ChromeFace::Named("Archivo Black"),
        elevation: Elevation::Bordered,
        list_style: POSTER_BARS,
        facet_style: FacetStyle::Chips(ChipVariant::Bracket),
        // THE SPLIT-PANE EXCEPTION (as one-line DATA, never a code branch): the
        // NERV console stays a UNIFIED room. It ships the poster Bars list today,
        // so this is dormant — but it records the identity so that were the
        // console ever a Pane world it would keep its single hard-edged slab, not
        // the two-surface split every other Pane world takes by default.
        pane_split: PaneSplit::Unified,
        ..RenderCaps::DEFAULT
    },
};

/// Cassowary Light — the EXPLORATORY LIGHT variant (the user's still-OPEN "idk if
/// eva should be light?" question), the **entry-plug interior / EVA-00 (Rei)
/// register**: a pale, clinical, backlit white with a faint cool-green cast (the
/// LCL-lit plug glass), dark slate-green data ink, an amber caret, and the NERV
/// red on white. Deliberately NOT in [`THEMES`] (a gallery exploration, not a
/// shipped world) — the dark `CASSOWARY` is the anchor and now ships the PHOSPHOR
/// ink caret (the user's dark-world pick); this LIGHT const is KEPT (dead code)
/// only because the light-vs-dark question was never closed, so the user can still
/// see the light option. To ship it: add to `THEMES` (+ the count / personality /
/// axis / CJK tests) and reconsider its caret (a light ink caret would want a
/// `Filled` block off the dark slate ink, not the amber here). Its ink-ladder
/// lightnesses mirror Gumtree's (a proven light-world ladder), so it clears the
/// strict light role-tint floors the same way.
#[allow(dead_code)] // gallery-only exploration; not in THEMES (see doc above).
pub const CASSOWARY_LIGHT: Theme = Theme {
    name: "Cassowary Light",
    dark: false,
    // Pale cool-green white — the backlit entry-plug glass.
    base_100: Srgb::rgb(0xEE, 0xF4, 0xF0),
    base_200: Srgb::rgb(0xE2, 0xEC, 0xE6),
    base_300: Srgb::rgb(0xD2, 0xE1, 0xD8),
    // Dark slate-green data ink (mirrors Gumtree's ink lightness).
    base_content: Srgb::rgb(0x16, 0x24, 0x1B),
    // Muted rung lightened just enough to widen the base_content->muted band so the
    // derived light Def/Const role tints clear their pairwise floor (the classic
    // light-world tightness); mirrors Gumtree's muted lightness.
    muted: Srgb::rgb(0x5A, 0x6E, 0x62),
    faint: Srgb::rgb(0x92, 0xA3, 0x98),
    // The amber LCL caret, a shade deeper for contrast on the pale ground.
    primary: Srgb::rgb(0xD9, 0x79, 0x22),
    primary_content: Srgb::rgb(0xFB, 0xEF, 0xE2),
    // NERV red, deepened so it holds contrast on white.
    error: Srgb::rgb(0xC2, 0x34, 0x29),
    // Pale warning-crimson selection (the "target-lock" band, light register).
    selection: Srgb::rgba(0xC8, 0x36, 0x5E, 0x5E),
    // The clinical lab: fine dim scan-lines in the margins, one register up.
    background: Background::Pinstripe {
        from: Srgb::rgb(0xE2, 0xEC, 0xE6),
        to: Srgb::rgb(0xD2, 0xE1, 0xD8),
        dir: (0.0, 1.0),
        tint: Srgb::rgb(0xAF, 0xC6, 0xB8),
    },
    font: "Iosevka",
    mono: "Iosevka",
    heading_bold: true,
    cjk: CJK_GOTHIC,
    zh_hans: CJK_ZH_HANS_SANS,
    zh_hant: CJK_ZH_HANT,
    ko: CJK_KO,
    ornaments: Ornaments { dash: '◆', star: '✴', underscore: '◈' },
    ornament_face: ORNAMENT_MARKS,
    ornament_scale: ORNAMENT_SCALE_GEOMETRIC,
    bullets: BULLETS_PLAIN,
    bullet_scale: BULLET_SCALE_PLAIN,
    list_indent_scale: LIST_INDENT_SCALE_PLAIN,
    tags: ThemeTags { time: Some("Day"), register: None, voice: Some("Technical"), temperature: None },
    role_overrides: RoleOverrides::NONE,
    // The same NERV console overlay as the dark anchor, so the user compares the
    // two on equal footing.
    render_caps: RenderCaps {
        title_style: TitleStyle::Placard {
            corner: PlacardCorner::Auto,
            scale: 3.0,
            ink: PlacardInk::Bold,
        },
        card_anchor: CardAnchor::TopLeft,
        chrome_face: ChromeFace::Named("Archivo Black"),
        elevation: Elevation::Bordered,
        list_style: POSTER_BARS,
        facet_style: FacetStyle::Chips(ChipVariant::Bracket),
        // The SPLIT-PANE EXCEPTION (see the dark `CASSOWARY`): the console family
        // stays UNIFIED as DATA. Dormant under the poster Bars list.
        pane_split: PaneSplit::Unified,
        ..RenderCaps::DEFAULT
    },
};

// The two caret EXPLORATIONS (CASSOWARY_PHOSPHOR / CASSOWARY_WATTLE) that served
// the user's "maybe the cursor could be a different colour, not amber" gallery
// round are RETIRED — the user picked PHOSPHOR, now shipped as the anchor
// `CASSOWARY` above (an ink caret drawn with the authentic `CaretBlockStyle::Filled`
// CRT block, not the exploration's photo-negative `InverseVideo`).

/// All eighteen worlds, in cycle order. `C-x t` advances through this list and
/// wraps; `C-x T` steps backward. The two deep cool darks — Currawong (OLED
/// black) beside the neutral Tawny/Mopoke pair, and Bowerbird (midnight navy)
/// beside the violet Bombora — sit with their kin; Brolga (the COOL LIGHT POLE)
/// sits with the light cluster, just before the statement worlds; the three
/// STATEMENT worlds close the cycle — Wagtail (the bare 1-bit room, NO warm
/// thing) beside Firetail (the warm den whose one warm thing is the living lava
/// GROUND itself), and Cassowary (the NERV terminal) sits after Firetail as the
/// dark-technical statement.
pub const THEMES: [Theme; 18] = [
    TAWNY, MOPOKE, CURRAWONG,
    POTOROO, GUMTREE, BILBY, SALTPAN, QUOKKA, BOMBORA, BOWERBIRD, MULGA, MANGROVE, GALAH, MAGPIE,
    // Brolga — the COOL LIGHT POLE — sits with the light cluster, just before the
    // statement worlds that close the cycle.
    BROLGA,
    WAGTAIL, FIRETAIL, CASSOWARY,
];

/// Const `str` equality (`==` is not available in a `const fn` on stable).
/// Compares byte-for-byte; used only by [`world_index`] at compile time.
const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Compile-time index of a world in [`THEMES`] BY NAME. This is the ONE way a
/// position-derived constant should be born, so a world INSERTED mid-array can
/// never silently repoint it at a neighbour (the index-vs-name fragility class
/// the debt audit swept for — the class the frost "regression" made everyone
/// fear even though that one turned out to be a corrupt build). A name with no
/// world PANICS at build time — a typo fails the compile, not a capture.
const fn world_index(name: &str) -> usize {
    let mut i = 0;
    while i < THEMES.len() {
        if str_eq(THEMES[i].name, name) {
            return i;
        }
        i += 1;
    }
    panic!("world_index: no world by that name")
}

/// Index into [`THEMES`] of the default/startup world: **Saltpan**, a warm light
/// world (sun-bleached salt flat, cinnamon-clay caret on ecru), picked by the
/// user 2026-07-11 as awl's first impression — a taste round, not a bugfix (the
/// prior default, Tawny, a dark warm-grey mono nocturne, remains one `C-x t`
/// cycle away). DERIVED FROM THE NAME via [`world_index`], never a hand-counted
/// literal: inserting a world anywhere in the roster leaves the default pointing
/// at Saltpan by construction (a stale literal index would silently hand a
/// fresh-launch user a DIFFERENT world on upgrade — guarded here, re-asserted by
/// `tests::roster_position_is_name_stable`). This only governs a genuinely FRESH
/// launch/capture: the sticky theme preference (`config.toml`'s `theme` key,
/// written whenever the user switches worlds via Cmd-T — a NAME, never an index,
/// so it too is insertion-immune) always wins for an EXISTING user —
/// `Config::apply_sticky_globals` applies it over this constant unless the
/// `--theme` CLI flag already set the global (see `config/apply.rs`).
pub const DEFAULT_THEME: usize = world_index("Saltpan");

/// The full roster of world NAMES, in [`THEMES`] cycle order — the ONE
/// code-owned source every external consumer reads instead of hand-copying a
/// name list that can silently drift out of sync with the real roster (item
/// 68: `awl --help` once advertised only ten of the eighteen shipped worlds,
/// and the unknown-`--theme` error built its own separate, independently
/// drifting `Vec`). `--help`, the unknown-`--theme` error, and the CLI
/// `--list-worlds` flag (in turn the one source `scripts/capture-worlds.sh`
/// reads, rather than keeping its own shell-side name list) all resolve the
/// roster through this one function — inserting or retiring a world in
/// [`THEMES`] changes every one of them for free.
pub fn world_names() -> Vec<&'static str> {
    THEMES.iter().map(|t| t.name).collect()
}
