//! Tests for the `theme` module (the sixteen worlds + their derivation laws)
//! -- split verbatim out of the former `theme.rs` monolith's embedded
//! `mod tests` (2026-07 code-organization pass); every test's NAME and MODULE
//! PATH are unchanged (`theme::tests::foo`) -- only which file its source
//! lives in moved.

use super::*;
use super::derive::{OVERLAY_SELROW_EXTRA_STEPS, SELECTED_BAND_STEPS};

/// PALETTE-COMPOSITION round (item 5): the picker's selected-row band
/// ([`overlay_selected_band`]) is the shared [`surface_selected`] climbed
/// [`OVERLAY_SELROW_EXTRA_STEPS`] FURTHER up the SAME surface ramp — a stronger
/// VALUE step, in the ramp's own direction, never a new hue (DESIGN §3/§5; the
/// distinguishability sweep is the law that polices its visibility). The shared
/// band the HUD/menu borders read is untouched.
#[test]
fn overlay_selected_band_is_a_stronger_value_step_never_a_hue() {
    let _g = crate::testlock::serial();
    assert!(OVERLAY_SELROW_EXTRA_STEPS > 0, "the round strengthens the band by default");
    for world in ["Bowerbird", "Saltpan", "Firetail", "Tawny"] {
        let t = set_active_by_name(world).unwrap();
        assert_ne!(t.base_200, t.base_300, "{world}: ordinary (non-collapsed) ramp");
        let shared = surface_selected();
        let band = overlay_selected_band();
        // Per channel: the overlay band moves in the SAME direction the ramp step
        // does (value-only, no hue reversal) and is at least as far as the shared
        // band (stronger-or-equal, gamut-clamp permitting).
        let chans = [
            (t.base_200.r, t.base_300.r, shared.r, band.r),
            (t.base_200.g, t.base_300.g, shared.g, band.g),
            (t.base_200.b, t.base_300.b, shared.b, band.b),
        ];
        for (lo, hi, sh, bd) in chans {
            let d = (hi as i32 - lo as i32).signum();
            let band_delta = bd as i32 - hi as i32;
            let shared_delta = sh as i32 - hi as i32;
            assert!(band_delta * d >= 0, "{world}: band stays in the ramp direction");
            assert!(
                band_delta * d >= shared_delta * d,
                "{world}: overlay band is >= the shared band's step (stronger-or-equal)"
            );
        }
    }
    // Non-triviality: on a dark world with ramp headroom the strengthening is
    // STRICT (the extra step actually moves the band).
    set_active_by_name("Bowerbird").unwrap();
    assert_ne!(
        overlay_selected_band(),
        surface_selected(),
        "Bowerbird: the strengthened band differs from the shared band"
    );
    set_active(DEFAULT_THEME);
}

/// PER-ITEM LIST SURFACES round (2026-07-16 REFIT) — the OBVIOUS-GLANCE law at
/// the derivation level, covering EVERY world (the pixel test
/// `bars_draw_a_findable_surface_per_row` only exercises the headless default
/// theme). Under [`ListStyle::Bars`] the PANE is dropped — the bars float on the
/// GROUND (`base_100`, the scrim/room), not in a card. So the reference is the
/// GROUND, not the vanished card: the unselected bar ([`overlay_bar_unselected`]
/// == `base_200`) is a WHISPER one gentle step off the ground in the ramp's own
/// direction, and the selected bar's band ([`overlay_selected_band`]) sits
/// further up still — AND the selected↔unselected value step is at least as large
/// as the unselected↔ground step, so the selected bar's pop leads its whisper
/// neighbours at least as strongly as a whisper leads the bare ground. The user's
/// rejected first cut inverted the taste (unselected == a saturated rung under the
/// selected band — "a picket fence where every row shouts"); the whisper gives the
/// selection somewhere to go. Value only, never a hue. One-bit worlds are exempt
/// (a collapsed ramp draws its selected row via `InverseFill`; bars are inert).
#[test]
fn bars_unselected_sits_a_quiet_rung_below_the_selected_band() {
    let _g = crate::testlock::serial();
    // Local redmean (perceptual distance) — the same shape the distinguishability
    // sweeps carry, nested per-test like this file's other color laws.
    fn redmean(a: Srgb, b: Srgb) -> f32 {
        let rbar = (a.r as f32 + b.r as f32) * 0.5;
        let dr = a.r as f32 - b.r as f32;
        let dg = a.g as f32 - b.g as f32;
        let db = a.b as f32 - b.b as f32;
        ((2.0 + rbar / 256.0) * dr * dr + 4.0 * dg * dg + (2.0 + (255.0 - rbar) / 256.0) * db * db)
            .sqrt()
    }
    for t in THEMES.iter() {
        set_active_by_name(t.name).unwrap();
        if t.is_one_bit() {
            // Collapsed ramp: `surface_step_band` folds every step to the ink
            // pole, so the ordering degenerates by design — the selected row is
            // drawn by `InverseFill`, not this fill. Declared exemption.
            continue;
        }
        // The GROUND the bars float on now the pane is dropped (base_100), the
        // reference the old card (base_300) used to be.
        let ground = t.base_100;
        let unsel = overlay_bar_unselected();
        let sel = overlay_selected_band();
        // Per channel: `unsel` moves in the ramp direction from the GROUND (a
        // whisper), and `sel` moves at least as far again (whisper strictly between
        // ground and selected in the ramp's own direction, value-only — no hue).
        let chans = [
            (ground.r, unsel.r, sel.r),
            (ground.g, unsel.g, sel.g),
            (ground.b, unsel.b, sel.b),
        ];
        // The overall ramp direction (base_200 -> base_300 carries it onward; the
        // monotone surface ladder makes this the base_100 -> base_200 step's sign too).
        let dir = [
            (t.base_300.r as i32 - t.base_200.r as i32).signum(),
            (t.base_300.g as i32 - t.base_200.g as i32).signum(),
            (t.base_300.b as i32 - t.base_200.b as i32).signum(),
        ];
        for (i, (c, u, s)) in chans.iter().copied().enumerate() {
            let d = dir[i];
            let unsel_step = (u as i32 - c as i32) * d;
            let sel_step = (s as i32 - c as i32) * d;
            assert!(unsel_step >= 0, "{}: unselected whisper lifts off the ground in the ramp direction", t.name);
            assert!(
                sel_step >= unsel_step,
                "{}: selected band ({s}) must sit at least as far up the ramp as the unselected whisper ({u}) from the ground ({c})",
                t.name
            );
        }
        // The OBVIOUS-GLANCE law (redmean): the selected↔unselected step reads at
        // least as strong as the unselected↔ground step — selection's pop leads its
        // whisper neighbours at least as much as a whisper leads the bare ground.
        let d_sel = redmean(sel, unsel);
        let d_bar = redmean(unsel, ground);
        assert!(
            d_sel >= d_bar,
            "{}: selected bar {sel:?} must lead the unselected whisper {unsel:?} (redmean {d_sel:.1}) at least as much as the whisper leads the ground {ground:?} (redmean {d_bar:.1})",
            t.name
        );
    }
    set_active(DEFAULT_THEME);
}


#[test]
fn worlds_eleven_dark_seven_light() {
    assert_eq!(THEMES.len(), 18);
    let dark = THEMES.iter().filter(|t| t.dark).count();
    let light = THEMES.iter().filter(|t| !t.dark).count();
    // 11 dark (Tawny/Mopoke/Currawong/Potoroo/Bombora/Bowerbird/Mulga/
    // Mangrove/Wagtail/Firetail/Cassowary) / 7 light (Gumtree/Bilby/Saltpan/
    // Quokka/Galah/Magpie/Brolga). Brolga (the COOL LIGHT POLE) is a pale
    // sky-blue light world filling the cool-light-blue hole the DAWN round
    // vacated when Bilby turned warm rose-gold; Cassowary (the NERV-terminal
    // statement world) is the eighteenth, an additive dark entry.
    assert_eq!(dark, 11);
    assert_eq!(light, 7);
}

/// `Theme::is_one_bit` — Wagtail's 2026-07 rework, from greyscale (any grey
/// permitted) to a true 1-bit world (only pure black/white) — is `true` for
/// Wagtail alone, and (the stricter sub-case relationship) every one-bit
/// world is ALSO monochrome (`is_monochrome`'s broader "no hue" signal).
#[test]
fn wagtail_alone_is_one_bit() {
    let one_bit: Vec<&str> = THEMES.iter().filter(|t| t.is_one_bit()).map(|t| t.name).collect();
    assert_eq!(one_bit, ["Wagtail"], "exactly Wagtail should be one-bit");
    for t in THEMES.iter().filter(|t| t.is_one_bit()) {
        assert!(t.is_monochrome(), "{}: a one-bit world must also be monochrome", t.name);
    }
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
    // Every STATIC ground type is still exercised across the worlds.
    let used: std::collections::HashSet<&str> =
        THEMES.iter().map(|t| t.background.as_str()).collect();
    for p in ["gradient", "dots", "starfield", "pinstripe", "stripes"] {
        assert!(used.contains(p), "ground {p} unused by any world");
    }
    // Stripes stays Potoroo's alone.
    let stripes: Vec<&str> = THEMES
        .iter()
        .filter(|t| matches!(t.background, Background::Stripes { .. }))
        .map(|t| t.name)
        .collect();
    assert_eq!(stripes, ["Potoroo"], "Stripes is Potoroo's alone");
    // PROXIMITY-SCALED Dots (`edge: true`) rode Mangrove alone, and Mangrove
    // folded into a lava ground (2026-07), so no world carries proximity Dots
    // now — the `edge: bool` machinery is intact but currently unassigned (like
    // `Background::Lava` was before this round). Not a bug: a feature may ship
    // with zero worlds until one wants it.
    let edge_dots: Vec<&str> = THEMES
        .iter()
        .filter(|t| t.background.edge())
        .map(|t| t.name)
        .collect();
    assert!(edge_dots.is_empty(), "proximity Dots is unassigned since Mangrove became lava, got {edge_dots:?}");
}

/// THE LAVA-LAMP WORLDS round: EXACTLY two worlds ship a `Background::Lava` —
/// Firetail (warm, undithered) and Mangrove (cool deepsea, dithered), both with
/// the Glow edge (the probe's agent pick). Pins the roster + each world's edge/
/// dither config, and that every OTHER world stays a STATIC ground (shader id
/// 0..=4) so the lava layer is dormant there and their captures are unaffected.
#[test]
fn exactly_firetail_and_mangrove_ship_lava() {
    let _lock = crate::testlock::serial();
    let lava: Vec<&str> = THEMES
        .iter()
        .filter(|t| t.background.is_lava())
        .map(|t| t.name)
        .collect();
    assert_eq!(lava, ["Mangrove", "Firetail"], "exactly Mangrove + Firetail are lava worlds");
    for t in THEMES.iter().filter(|t| !t.background.is_lava()) {
        assert!(
            t.background.shader_id() <= 4,
            "{}: a non-lava world stays a static ground",
            t.name
        );
    }
    // Firetail: WARM, undithered, Glow edge; ground == its own base_100 (seamless).
    let f = set_active_by_name("Firetail").unwrap();
    let (fg, _flo, _fhi, fe, fd) = f.background.lava_params().unwrap();
    assert_eq!(fg, f.base_100, "Firetail lava ground == base_100 (seamless margin↔page)");
    assert_eq!(fe, model::LavaEdge::Glow, "Firetail default edge is Glow");
    assert!(!fd, "Firetail is the SMOOTH warm lamp (undithered)");
    // Mangrove: COOL deepsea, DITHERED, Glow edge; ground == its own base_100.
    let m = set_active_by_name("Mangrove").unwrap();
    let (mg, _mlo, _mhi, me, md) = m.background.lava_params().unwrap();
    assert_eq!(mg, m.base_100, "Mangrove lava ground == base_100 (seamless margin↔page)");
    assert_eq!(me, model::LavaEdge::Glow, "Mangrove default edge is Glow");
    assert!(md, "Mangrove is the DITHERED cool lamp (print-grain)");
    set_active(DEFAULT_THEME);
}

/// THE `Background::Lava` FIGURE/GROUND LAW (Firetail + Mangrove): the ANIMATED
/// metaball margins must READ AS GROUND at EVERY phase — never brightening into
/// "figure" territory that would compete with the flat page column the text sits
/// on, and always leaving the ink a strong contrast to sit against. Asserted over
/// composited PIXELS (the pure-Rust shader mirror in `crate::lava` + each world's
/// own blob colors + color arithmetic), NOT over sidecar state — the Wagtail-
/// invisible-picker-row lesson: appearance is proven over the bytes, never inferred.
#[test]
fn lava_worlds_keep_figure_ground_at_the_worst_animation_phase() {
    // Gamma-correct Rec.709 relative luminance (the `render::tests::syntax_roles`
    // `rel_luminance` recipe), so the "ground value band" is PERCEIVED brightness.
    fn rel_lum(c: Srgb) -> f32 {
        fn lin(u: u8) -> f32 {
            let s = u as f32 / 255.0;
            if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        }
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }
    // redmean color distance (the `distinguishability`/`syntax_roles` metric).
    fn redmean(a: Srgb, b: Srgb) -> f32 {
        let rbar = (a.r as f32 + b.r as f32) * 0.5;
        let dr = a.r as f32 - b.r as f32;
        let dg = a.g as f32 - b.g as f32;
        let db = a.b as f32 - b.b as f32;
        ((2.0 + rbar / 256.0) * dr * dr + 4.0 * dg * dg + (2.0 + (255.0 - rbar) / 256.0) * db * db)
            .sqrt()
    }
    for t in THEMES.iter().filter(|t| t.background.is_lava()) {
        let (ground, blob_lo, blob_hi, _edge, _dith) = t.background.lava_params().unwrap();
        assert_eq!(ground, t.base_100, "{}: lava ground must be base_100", t.name);

        // (1) VALUE BAND. The shader only ever blends ground → blob_lo → blob_hi
        //     (`rgb = mix(ground, mix(blob_lo, blob_hi, core_t), edge_t)`), and
        //     mix() is bounded by its endpoints, so blob_hi is the BRIGHTEST pixel
        //     the animated margin can ever produce. It must not brighten past the
        //     world's own brightest GROUND rung (base_300) — else the margins would
        //     read as "figure", competing with the page. (In HSL-L the probe noted a
        //     ~1–3 point overshoot; in perceptual luminance it vanishes — the wine/
        //     teal blobs are red/blue-heavy, luminance-light.)
        let band_ceiling = rel_lum(t.base_300) + 0.005; // +float epsilon only
        assert!(
            rel_lum(blob_hi) <= band_ceiling,
            "{}: blob_hi luminance {:.4} exceeds the ground band ceiling base_300 {:.4} \
             (animated margin brightens into figure territory)",
            t.name, rel_lum(blob_hi), rel_lum(t.base_300)
        );
        assert!(
            rel_lum(blob_lo) <= band_ceiling,
            "{}: blob_lo luminance {:.4} exceeds the ground band ceiling", t.name, rel_lum(blob_lo)
        );

        // (2) blob_hi is a REAL rendered pixel, not just a theoretical ceiling: drive
        //     the pure mirror over a full phase sweep and confirm the metaball field
        //     SATURATES the core blend somewhere in the margin (the shader saturates
        //     core_t at field ≥ THRESHOLD + CORE_WIDTH = 0.85; the strongest backdrop
        //     blob's weight alone exceeds that at its own animated center) — so the ground
        //     genuinely reaches blob_hi, and (1) is a check on an ACTUAL worst-phase pixel.
        let vp = (1200.0, 800.0);
        let blobs = &crate::lava::BACKDROP_BLOBS;
        let mut peak = 0.0f32;
        for step in 0..128 {
            let phase = step as f32 * crate::lava::LAVA_LOOP_CYCLES / 128.0;
            for (i, b) in blobs.iter().enumerate() {
                let (cx, cy) =
                    crate::lava::animated_center(i, b[0], b[1], b[2], vp, phase);
                let px = (cx * vp.0, cy * vp.1);
                peak = peak.max(crate::lava::metaball_field(px, vp, blobs, phase));
            }
        }
        assert!(
            peak >= 1.0,
            "{}: metaball field peaks at only {peak:.3} over a full phase sweep — the core \
             never saturates, so blob_hi is unreached (the worst-phase check would be vacuous)",
            t.name
        );

        // (3) TEXT CONTRAST PRESERVED at the worst phase: the ink (base_content) clears
        //     a strong legibility floor even against the LOUDEST reachable ground pixel
        //     (blob_hi). The floor (150) is far below the measured ~500 (both worlds), so
        //     text sitting anywhere near the margins stays unmistakably the figure.
        let d = redmean(t.base_content, blob_hi);
        assert!(
            d >= 150.0,
            "{}: base_content vs the brightest lava pixel blob_hi only {d:.1} redmean apart \
             (ground competes with the ink at the worst phase)",
            t.name
        );
    }
}

/// THE `Background::Lava` AMBER-HUE-CLEAR GUARD (mirrors the syntax role tints'
/// amber-guard): the lava blobs are ambient GROUND motion — the sole DESIGN.md §3
/// exception this round grants — but the CARET's amber must remain the one accent,
/// so any blob tone with real chroma (HSL saturation > 0.15) sits ≥30° of hue from
/// `primary`. Firetail's wine blobs clear it at ~59°; Mangrove's cool blues at ~175°.
#[test]
fn lava_blob_hues_stay_clear_of_the_amber_caret() {
    // Minimal circular hue distance in degrees.
    fn hue_gap(a: f32, b: f32) -> f32 {
        let d = (a - b).abs() % 360.0;
        d.min(360.0 - d)
    }
    for t in THEMES.iter().filter(|t| t.background.is_lava()) {
        let (_ground, blob_lo, blob_hi, _edge, _dith) = t.background.lava_params().unwrap();
        let (ph, _ps, _pl) = t.primary.to_hsl();
        for (label, blob) in [("blob_lo", blob_lo), ("blob_hi", blob_hi)] {
            let (bh, bs, _bl) = blob.to_hsl();
            if bs <= 0.15 {
                continue; // a near-grey blob reads as a value step, not a second accent.
            }
            let gap = hue_gap(bh, ph);
            assert!(
                gap >= 30.0,
                "{}: lava {label} hue {bh:.0}° sits only {gap:.0}° from the amber caret {ph:.0}° \
                 (a second accent — DESIGN §3 one-accent law)",
                t.name
            );
        }
    }
}

/// THE TWINKLING-STARS LAWS (2026-07-18 — the "aliveness ≠ loudness" round;
/// RE-SCOPED 2026-07-23 for the LIFECYCLE round). Every world's
/// `render_caps.ambient` is swept with a NO-WILDCARD match (a future
/// `AmbientStyle` variant fails to compile until it's under the law). Every tint
/// is drawn from the world's own star PALETTE ([`crate::stars::star_palette`] —
/// blue-white / white / champagne), the ONE owner the renderer draws from too,
/// and each palette entry is fenced. For a `Stars` world, four fences — the same
/// shapes that fence the lava:
///
/// (a) **VISIBILITY BAND (the RELAXED, user-blessed brightness ceiling).** THE
///     LIFECYCLE round loosens the old `<= muted` whisper cap: a star's shine
///     may now rise ABOVE the muted rung (a real glint, not a whisper), but its
///     PEAK composited pixel — each palette tint alpha-blended in LINEAR light
///     over each margin-ground endpoint, exactly the GPU's SrcAlpha blend —
///     still stays STRICTLY UNDER the `base_content` (text-ink) deviation: the
///     figure stays the text's, a star never outshines the prose. And the
///     relaxation is REAL, not vestigial: the BRIGHTEST palette tint at peak
///     genuinely exceeds the muted whisper cap over the darker ground (else the
///     old cap would still bind and nothing changed). Proven over COMPOSITED
///     values, never authored bytes (the Saltpan/camouflage lesson).
/// (b) **VISIBLE, not the invisible-band trap.** A star lit only to the band
///     FLOOR still composites at least ΔY 0.02 off its local ground — the
///     dimmest LIT star is genuinely seeable (a star that composites to nothing
///     would pass every mechanism test while the sky ships empty — the Wagtail
///     invisible-row lesson). (The DWELL is a separate, deliberate true-zero;
///     the band bounds a star while it is LIT.)
/// (c) **AMBER GUARD.** Every palette tint: a chromatic one (HSL sat > 0.15)
///     sits ≥ 30° of hue from the world's `primary`, and none is literally
///     `primary` — the one-accent law (DESIGN §3): the caret stays the only warm
///     thing. (Champagne holds this by low saturation despite its warm hue.)
/// (d) **ONE-BIT GUARD.** A star's alpha is FRACTIONAL by construction —
///     structurally illegal on a true 1-bit world (any intermediate composite is
///     a forbidden third value), so `Stars` on an `is_one_bit()` world fails
///     here before a render could paint it. (A one-bit sky would need a
///     dither-stipple star mode — banked.)
///
/// Param sanity rides along: band ordered (`0 < floor < peak <= 1`), density in
/// `(0, 1]`, and the dot small enough for its cell's jitter band.
#[test]
fn ambient_stars_laws_hold_for_every_world() {
    fn lin(u: u8) -> f32 {
        let s = u as f32 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    }
    fn rel_lum(c: Srgb) -> f32 {
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }
    fn hue_gap(a: f32, b: f32) -> f32 {
        let d = (a - b).abs() % 360.0;
        d.min(360.0 - d)
    }
    // The GPU blend (linear-space SrcAlpha over) applied to luminance — linear
    // light is additive, so Y composites exactly.
    fn composite_y(tint: Srgb, alpha: f32, ground: Srgb) -> f32 {
        alpha * rel_lum(tint) + (1.0 - alpha) * rel_lum(ground)
    }
    let mut stars_worlds = 0usize;
    for t in THEMES.iter() {
        match t.render_caps.ambient {
            model::AmbientStyle::None => continue,
            model::AmbientStyle::Stars { tint, cell_px, density, size_px, peak, floor } => {
                stars_worlds += 1;
                // Param sanity.
                assert!(
                    0.0 < floor && floor < peak && peak <= 1.0,
                    "{}: the visibility band must be ordered (0 < floor {floor} < peak {peak} <= 1)",
                    t.name
                );
                assert!(
                    (0.0..=1.0).contains(&density) && density > 0.0,
                    "{}: density {density} out of (0, 1]",
                    t.name
                );
                assert!(
                    size_px > 0.0 && size_px < cell_px * 0.3,
                    "{}: dot {size_px}px must stay well inside its {cell_px}px cell's jitter band",
                    t.name
                );
                // (d) ONE-BIT GUARD.
                assert!(
                    !t.is_one_bit(),
                    "{}: a fractional-alpha star is structurally illegal on a true \
                     1-bit world (any intermediate composite is a forbidden third value)",
                    t.name
                );
                // The PALETTE the renderer draws from IS the law's subject — one owner.
                let palette = crate::stars::star_palette(tint);
                let (ph, _ps, _pl) = t.primary.to_hsl();
                for st in palette {
                    // (c) AMBER GUARD, per palette entry.
                    assert_ne!(st, t.primary, "{}: a star tint must never BE the accent", t.name);
                    let (sh, ss, _sl) = st.to_hsl();
                    if ss > 0.15 {
                        let gap = hue_gap(sh, ph);
                        assert!(
                            gap >= 30.0,
                            "{}: star tint hue {sh:.0}° sits only {gap:.0}° from the caret's \
                             {ph:.0}° — a second accent (DESIGN §3)",
                            t.name
                        );
                    }
                }
                // (a)+(b) VISIBILITY BAND, per palette entry, per ground endpoint.
                let muted_dev = (rel_lum(t.muted) - rel_lum(t.base_100)).abs();
                let content_dev = (rel_lum(t.base_content) - rel_lum(t.base_100)).abs();
                // The BRIGHTEST palette tint drives the relaxation-is-real check.
                let brightest = palette
                    .into_iter()
                    .max_by(|a, b| rel_lum(*a).partial_cmp(&rel_lum(*b)).unwrap())
                    .unwrap();
                let mut relaxation_seen = false;
                for st in palette {
                    for (label, ground) in [("from", t.background.from()), ("to", t.background.to())] {
                        let gy = rel_lum(ground);
                        let peak_dev = (composite_y(st, peak, ground) - gy).abs();
                        // CALM CEILING: strictly under the text ink — the figure
                        // stays the prose's, however bright the glint.
                        assert!(
                            peak_dev < content_dev,
                            "{}: a peak star over the {label} ground deviates ΔY {peak_dev:.3} — \
                             not strictly under the text ink's {content_dev:.3}; a glint must \
                             never outshine the prose",
                            t.name
                        );
                        // VISIBLE FLOOR: the dimmest LIT star is still seeable.
                        let floor_dev = (composite_y(st, floor, ground) - gy).abs();
                        assert!(
                            floor_dev >= 0.02,
                            "{}: a floor (dimmest lit) star over the {label} ground deviates only \
                             ΔY {floor_dev:.3} — the invisible-band trap (lit but unseeable)",
                            t.name
                        );
                        assert!(
                            floor_dev < peak_dev,
                            "{}: the band must brighten from floor to peak (floor ΔY {floor_dev:.3} \
                             !< peak ΔY {peak_dev:.3})",
                            t.name
                        );
                        // RELAXATION IS REAL: the brightest tint at peak clears
                        // the old muted whisper cap somewhere (the deliberate,
                        // user-blessed loosening — else nothing actually changed).
                        if st == brightest && peak_dev > muted_dev {
                            relaxation_seen = true;
                        }
                    }
                }
                assert!(
                    relaxation_seen,
                    "{}: the brightest star's peak never exceeds the muted whisper cap \
                     ({muted_dev:.3}) on any ground — the LIFECYCLE round's blessed relaxation \
                     is vestigial (a real glint must rise above the old cap)",
                    t.name
                );
            }
        }
    }
    // The round's assignment: exactly ONE stars world ships (Currawong — the
    // user's pick). A second is a conscious data edit that lands here.
    assert_eq!(stars_worlds, 1, "exactly one world ships AmbientStyle::Stars today");
}

/// THE FROST PILL CONTRAST LAW (the FROST RAIL round — RE-SCOPED from the retired
/// whole-margin carve law, which asserted the old flat rail this round replaced).
/// The shipped headed-doc treatment is now per-entry FROST pills: behind each
/// outline entry the lava renders a softened (blurred SMOOTH-field) sample
/// value-DIMMED toward the flat ground (`crate::lava::frost_pixel` /
/// `crate::lava::FROST_DIM`), while the lamp stays fully alive between and around
/// the pills. This law proves the DIM outline ink stays legible over that frosted
/// pill ground at EVERY animation phase, in two halves:
///
/// (1) PHASE SWEEP (64 phases × a pill-region grid in the left margin): the ACTUAL
///     frosted pixel — the pure-Rust shader mirror `frost_field` → `frost_pixel` —
///     clears the ink-ladder floors against the outline's inks: the `faint` (every
///     non-current) entry at redmean >= 100, the `base_content` current row at >=
///     150. Proven over COMPOSITED PIXELS, never sidecar state (the Wagtail
///     invisible-picker-row lesson). WITNESSED non-vacuous: some sampled frost
///     pixel genuinely differs from the flat ground (the lamp reads THROUGH the
///     frost — it is a softened lamp, not the old flat carve).
///
/// (2) PHASE-FREE WORST BOUND: the brightest a frost pill can ever reach is
///     `mix(blob_hi, ground, FROST_DIM)` (the softened field bounded by blob_hi,
///     then dimmed) — proving the ink clears THAT covers every phase by
///     construction, a belt-and-braces guard the sweep can't miss.
///
/// The `Background` match is NO-WILDCARD: a future ground variant must decide its
/// frost story here or fail to compile.
#[test]
fn outline_frost_pills_keep_ink_contrast_on_every_lava_world() {
    fn redmean(a: Srgb, b: Srgb) -> f32 {
        let rbar = (a.r as f32 + b.r as f32) * 0.5;
        let dr = a.r as f32 - b.r as f32;
        let dg = a.g as f32 - b.g as f32;
        let db = a.b as f32 - b.b as f32;
        ((2.0 + rbar / 256.0) * dr * dr + 4.0 * dg * dg + (2.0 + (255.0 - rbar) / 256.0) * db * db)
            .sqrt()
    }
    // Representative page geometry (the 1600x1000 gallery canvas). Frost pills sit
    // in the LEFT margin (x well below col_left), hugging the outline entries.
    let vp = (1600.0f32, 1000.0f32);
    for t in THEMES.iter() {
        // NO-WILDCARD: a future ground variant must decide its frost story here.
        let (ground, blob_lo, blob_hi) = match t.background {
            // The five static grounds carry no lava — no frost.
            Background::Gradient { .. }
            | Background::Dots { .. }
            | Background::Starfield { .. }
            | Background::Pinstripe { .. }
            | Background::Stripes { .. } => continue,
            Background::Lava { ground, blob_lo, blob_hi, .. } => (ground, blob_lo, blob_hi),
        };
        // FROST-AS-CAPABILITY: read the WORLD's own recipe (`render_caps.frost`),
        // not the shipped consts — so a world that dials a gentler/stronger frost
        // is held to the SAME ink-contrast floor it must clear.
        let blur = t.render_caps.frost.blur_px;
        let dim = t.render_caps.frost.dim;
        assert_eq!(ground, t.base_100, "{}: frost ground must be base_100", t.name);

        // (1) Phase sweep × a pill-region grid: the ACTUAL frost pixel clears the
        //     ink-ladder floors, and the lamp genuinely reads through the frost.
        let mut witnessed_alive = false;
        for step in 0..64 {
            let phase = step as f32 * crate::lava::LAVA_LOOP_CYCLES / 64.0;
            // Left-margin pill band: x below the column, y across the outline rows.
            for xi in 0..24 {
                let x = 80.0 + (270.0 - 80.0) * (xi as f32 + 0.5) / 24.0;
                for y in [150.0, 320.0, 500.0, 680.0, 850.0] {
                    let field = crate::lava::frost_field((x, y), vp, &crate::lava::BACKDROP_BLOBS, phase, blur);
                    let px = crate::lava::frost_pixel(field, ground, blob_lo, blob_hi, dim);
                    let dimd = redmean(t.faint, px);
                    assert!(
                        dimd >= 100.0,
                        "{}: faint outline ink only {dimd:.1} redmean from the frost pill \
                         at x={x} y={y} phase={phase} (under the ink-ladder floor)",
                        t.name
                    );
                    let lit = redmean(t.base_content, px);
                    assert!(
                        lit >= 150.0,
                        "{}: the current outline row only {lit:.1} redmean from the frost \
                         pill at x={x} y={y} phase={phase}",
                        t.name
                    );
                    if (px.r, px.g, px.b) != (ground.r, ground.g, ground.b) {
                        witnessed_alive = true;
                    }
                }
            }
        }
        assert!(
            witnessed_alive,
            "{}: no sampled frost pixel differs from the flat ground — the frost is a \
             vacuous flat carve, not a softened LIVING lamp",
            t.name
        );

        // (2) PHASE-FREE WORST BOUND: mix(blob_hi, ground, dim) is the brightest a
        //     frost pill can reach; the ink clears the floors against it, so every
        //     phase is covered by construction.
        let worst = crate::lava::frost_pixel(1.0, ground, blob_lo, blob_hi, dim);
        assert!(
            redmean(t.faint, worst) >= 100.0,
            "{}: faint ink only {:.1} redmean from the WORST frost pill (phase-free bound)",
            t.name,
            redmean(t.faint, worst)
        );
        assert!(
            redmean(t.base_content, worst) >= 150.0,
            "{}: current row only {:.1} redmean from the worst frost pill",
            t.name,
            redmean(t.base_content, worst)
        );
    }
}

/// THE GUTTER FROST PILL CONTRAST LAW (the de-uglify round — re-scoped from the
/// old bounded-corner HARD carve this round replaced). The bottom-left page-mode
/// GUTTER (`TextPipeline::prepare_gutter` — the filename/project stack) used to
/// HARD-carve its corner out of the lava mask, dropping the band to the flat,
/// DARKEST page ground (`base_100`) — an ugly geometric dark pocket beside the much
/// lighter writing column and below the margin's own blob peaks (worst on
/// Firetail, ground lum ~12 vs column ~60). It now rides the SAME organic FROST
/// FIELD the outline does (`TextPipeline::gutter_frost_seeds` →
/// `prepare_lava_layer`'s `seeds`): the lamp renders SOFTENED (a blurred
/// SMOOTH-field sample, `crate::lava::frost_field`) and value-DIMMED toward the
/// flat ground (`crate::lava::frost_pixel` / `FROST_DIM`), so the dim gutter ink
/// keeps its contrast while the lamp reads THROUGH — a warm whisper, not a dead
/// flat rectangle. Four halves:
///
/// (1) LEGIBILITY over the FROST pill at EVERY phase (64 phases × an in-pill grid):
///     the ACTUAL frosted pixel (the pure-Rust shader mirror `frost_field` →
///     `frost_pixel`) clears the ink-ladder floors against the gutter's own inks —
///     the `faint` project line at redmean >= 100, the `muted` filename line at
///     >= 150. Proven over COMPOSITED PIXELS, never sidecar state (the Wagtail
///     invisible-picker-row lesson).
///
/// (2) THE DARK REGION IS FIXED — the lamp reads THROUGH: WITNESSED non-vacuous —
///     some sampled frost pixel genuinely differs from the flat ground, so the old
///     dead-flat dark pocket is gone (a softened living lamp, not a carve).
///
/// (3) PHASE-FREE WORST BOUND: the brightest a pill can ever reach is
///     `frost_pixel(1.0, ..)` = `mix(blob_hi, ground, FROST_DIM)`; the ink clears
///     the floors against THAT, so every phase is covered by construction.
///
/// (4) THE FROST IS LOCAL — both margins keep their lamp: the organic coverage
///     (`frost_coverage`) is solid OVER the gutter seed's ink and exactly 0 far
///     from every seed (the left margin high above the band, the whole right
///     margin), so nothing carves and the rest of both margins stay their live
///     lamp. The gutter seed geometry is pinned at the render seam by
///     `render::tests::outline::gutter_frost_seeds_follow_gutter_visibility`.
///
/// The `Background` match is NO-WILDCARD: a future ground variant must decide its
/// frost story here or fail to compile. A static-ground world carries no lava, so
/// it `continue`s — no frost, byte-identical (the unaffected-worlds guarantee).
#[test]
fn gutter_frost_pill_keeps_ink_contrast_on_every_lava_world() {
    fn redmean(a: Srgb, b: Srgb) -> f32 {
        let rbar = (a.r as f32 + b.r as f32) * 0.5;
        let dr = a.r as f32 - b.r as f32;
        let dg = a.g as f32 - b.g as f32;
        let db = a.b as f32 - b.b as f32;
        ((2.0 + rbar / 256.0) * dr * dr + 4.0 * dg * dg + (2.0 + (255.0 - rbar) / 256.0) * db * db)
            .sqrt()
    }
    // Representative page geometry (the 1600x1000 gallery canvas). The gutter seeds
    // the ORGANIC frost field near the bottom-left column edge — a capsule run
    // `[x0, x1, yc, r]` hugging the two stacked LABEL rows ~8px up from the canvas
    // bottom (`prepare_gutter` / `gutter_frost_seeds`).
    let vp = (1600.0f32, 1000.0f32);
    let gutter_seed = [40.0f32, 250.0, 930.0, 40.0];
    for t in THEMES.iter() {
        // NO-WILDCARD: a future ground variant must decide its frost story here.
        let (ground, blob_lo, blob_hi) = match t.background {
            // The five static grounds carry no lava — no frost, byte-identical.
            Background::Gradient { .. }
            | Background::Dots { .. }
            | Background::Starfield { .. }
            | Background::Pinstripe { .. }
            | Background::Stripes { .. } => continue,
            Background::Lava { ground, blob_lo, blob_hi, .. } => (ground, blob_lo, blob_hi),
        };
        // FROST-AS-CAPABILITY: the WORLD's own recipe (`render_caps.frost`), so a
        // world tuning its gutter frost is held to the same ink-contrast floor.
        let blur = t.render_caps.frost.blur_px;
        let dim = t.render_caps.frost.dim;
        // The field's un-lit floor IS the page's own ground — the ink-ladder laws
        // govern it; the frost only ever LIFTS from there toward the dimmed lamp.
        assert_eq!(ground, t.base_100, "{}: frost ground must be base_100", t.name);

        // (1)+(2) Phase sweep × in-pill grid: the ACTUAL frost pixel clears the
        //         gutter ink floors, AND the lamp genuinely reads through the frost
        //         (the dark pocket is gone).
        let mut witnessed_alive = false;
        for step in 0..64 {
            let phase = step as f32 * crate::lava::LAVA_LOOP_CYCLES / 64.0;
            for xi in 0..16 {
                // x strictly INSIDE the pill, past its right-face feather.
                let x = 12.0 + (235.0 - 12.0) * (xi as f32 + 0.5) / 16.0;
                for y in [860.0, 900.0, 940.0, 980.0] {
                    let field = crate::lava::frost_field(
                        (x, y),
                        vp,
                        &crate::lava::BACKDROP_BLOBS,
                        phase,
                        blur,
                    );
                    let px = crate::lava::frost_pixel(field, ground, blob_lo, blob_hi, dim);
                    let project = redmean(t.faint, px);
                    assert!(
                        project >= 100.0,
                        "{}: the gutter's faint project line only {project:.1} redmean from \
                         the frost pill at x={x} y={y} phase={phase} (under the ink-ladder floor)",
                        t.name
                    );
                    let name = redmean(t.muted, px);
                    assert!(
                        name >= 150.0,
                        "{}: the gutter's muted filename only {name:.1} redmean from the frost \
                         pill at x={x} y={y} phase={phase}",
                        t.name
                    );
                    if (px.r, px.g, px.b) != (ground.r, ground.g, ground.b) {
                        witnessed_alive = true;
                    }
                }
            }
        }
        assert!(
            witnessed_alive,
            "{}: no sampled gutter frost pixel differs from the flat ground — the dark pocket \
             would still be a dead-flat carve, not a softened LIVING lamp",
            t.name
        );

        // (3) PHASE-FREE WORST BOUND: frost_pixel(1.0, ..) = mix(blob_hi, ground,
        //     dim) is the brightest a pill can reach; the ink clears the floors
        //     against it, so every phase is covered by construction.
        let worst = crate::lava::frost_pixel(1.0, ground, blob_lo, blob_hi, dim);
        assert!(
            redmean(t.faint, worst) >= 100.0,
            "{}: faint project ink only {:.1} redmean from the WORST gutter frost pill (phase-free bound)",
            t.name,
            redmean(t.faint, worst)
        );
        assert!(
            redmean(t.muted, worst) >= 150.0,
            "{}: muted filename ink only {:.1} redmean from the worst gutter frost pill",
            t.name,
            redmean(t.muted, worst)
        );

        // (4) THE FROST IS LOCAL — both margins keep their live lamp. The organic
        //     coverage is solid over the gutter seed's ink (non-vacuous) and exactly
        //     zero far from every seed (the left margin high above the band, and the
        //     whole right margin), so nothing is carved and the rest of both margins
        //     are untouched. `frost_coverage` sums the seed halos and thresholds them.
        assert!(
            crate::lava::frost_coverage(120.0, 930.0, &[gutter_seed]) > 0.99,
            "{}: the gutter seed does not frost its own ink (vacuous)",
            t.name
        );
        for (x, y) in [
            (150.0, 400.0),  // left margin, far above the band
            (200.0, 200.0),  // left margin, far above the band
            (1320.0, 930.0), // right margin, at the band's y
            (1560.0, 970.0), // right margin, deep bottom
        ] {
            assert_eq!(
                crate::lava::frost_coverage(x, y, &[gutter_seed]),
                0.0,
                "{}: frost leaked far from the gutter seed at x={x} y={y} (not local — a margin lost its lamp)",
                t.name
            );
        }
    }
}

/// THE FROST-AS-CAPABILITY law (the frost round). The softened-lamp recipe is a
/// per-world RenderCaps dial ([`crate::theme::Frost`]), not bare `crate::lava`
/// consts — so a world tunes its own frost as DATA (the runtime consumer
/// `TextPipeline::prepare_lava_layer` reads `render_caps.frost`; the grep-law
/// `theme_caps_law` bans a world name in `render/`). Three invariants:
///
/// (1) ONE SOURCE: `Frost::DEFAULT` equals the `crate::lava` numeric literals its
///     pure shader-mirror tests still read, and `RenderCaps::DEFAULT.frost` is
///     that default — so promoting the recipe to a capability is byte-identical.
///
/// (2) WELL-FORMED PER WORLD: every world's recipe is a sane frost — `dim` in
///     [0,1], `blur_px` > 0, `feather_px` >= 0 — an invariant that HOLDS even
///     after a world dials a gentler/stronger recipe (the ink-contrast floor the
///     two frost laws enforce is the taste-safety net; this is the shape net).
///
/// (3) STATIC GROUNDS ARE INERT: a non-lava world carries the default recipe but
///     never renders frost (the `lava_params().is_some()` gate in the consumer),
///     so its `frost` field is dormant data — the 1-bit/static exclusion stays
///     structural, gated on the lava CAPABILITY, never a world name.
#[test]
fn frost_recipe_is_a_per_world_capability_defaulting_to_the_shipped_lava_values() {
    use crate::theme::Frost;
    // (1) One source of truth: the capability default IS the lava consts.
    assert_eq!(Frost::DEFAULT.dim, crate::lava::FROST_DIM, "frost dim default == lava const");
    assert_eq!(Frost::DEFAULT.blur_px, crate::lava::FROST_BLUR_PX, "frost blur default == lava const");
    assert_eq!(
        Frost::DEFAULT.feather_px,
        crate::lava::FROST_FEATHER_PX,
        "frost feather default == lava const"
    );
    assert_eq!(
        RenderCaps::DEFAULT.frost,
        Frost::DEFAULT,
        "the DEFAULT caps carry the shipped frost recipe (byte-identical promotion)"
    );

    // (2)+(3) Every world's recipe is well-formed, and the recipe is present as
    //     DATA on lava and static worlds alike (dormant on static — the consumer
    //     gates on the lava capability, not this field).
    let mut saw_lava = false;
    for t in THEMES.iter() {
        let f = t.render_caps.frost;
        assert!(
            (0.0..=1.0).contains(&f.dim),
            "{}: frost dim {} out of [0,1]",
            t.name,
            f.dim
        );
        assert!(f.blur_px > 0.0, "{}: frost blur must be positive ({})", t.name, f.blur_px);
        assert!(f.feather_px >= 0.0, "{}: frost feather must be non-negative ({})", t.name, f.feather_px);
        if t.background.is_lava() {
            saw_lava = true;
        }
    }
    assert!(saw_lava, "a lava world ships (the frost capability has a live consumer)");
}

/// FIRETAIL PALETTE CHARACTER law: the sixteenth world is an ORIGINAL deep
/// oxblood-charcoal + wine-lava + ember-gold system, not Potoroo's rust palette
/// copied under a moving ground. Hue arithmetic pins the authored direction:
/// Firetail's main ground is much nearer red than Bombora's violet, at least
/// 35° away from Potoroo's orange-rust ground, and both its lava and caret stay
/// in their named wine/gold bands.
#[test]
fn firetail_is_oxblood_wine_and_ember_not_potoroo_rust_or_bombora_violet() {
    fn redmean(a: Srgb, b: Srgb) -> f32 {
        let rbar = (a.r as f32 + b.r as f32) * 0.5;
        let dr = a.r as f32 - b.r as f32;
        let dg = a.g as f32 - b.g as f32;
        let db = a.b as f32 - b.b as f32;
        ((2.0 + rbar / 256.0) * dr * dr
            + 4.0 * dg * dg
            + (2.0 + (255.0 - rbar) / 256.0) * db * db)
            .sqrt()
    }
    fn hue_gap(a: f32, b: f32) -> f32 {
        let d = (a - b).abs() % 360.0;
        d.min(360.0 - d)
    }
    fn red_gap(h: f32) -> f32 {
        hue_gap(h, 0.0)
    }

    let fire_ground = FIRETAIL.base_300.to_hsl().0;
    let potoroo_rust = POTOROO.base_300.to_hsl().0;
    let bombora_violet = BOMBORA.base_300.to_hsl().0;
    assert!(
        red_gap(fire_ground) + 60.0 <= red_gap(bombora_violet),
        "Firetail ground {fire_ground:.1}° must read far redder/warmer than Bombora {bombora_violet:.1}°"
    );
    assert!(
        hue_gap(fire_ground, potoroo_rust) >= 35.0,
        "Firetail ground {fire_ground:.1}° must stay substantially clear of Potoroo's orange-rust {potoroo_rust:.1}°"
    );

    let (base_h, base_s, base_l) = FIRETAIL.base_100.to_hsl();
    assert!(
        red_gap(base_h) <= 25.0 && base_s >= 0.25 && base_l <= 0.08,
        "Firetail base_100 must stay deep oxblood-charcoal, got h={base_h:.1}° s={base_s:.2} l={base_l:.2}"
    );

    let (_ground, lo, hi, edge, dithered) = FIRETAIL.background.lava_params().unwrap();
    for (label, c) in [("blob_lo", lo), ("blob_hi", hi)] {
        let h = c.to_hsl().0;
        assert!(
            h >= 330.0,
            "Firetail {label} hue {h:.1}° must stay in the deep red/wine band"
        );
    }
    let caret_h = FIRETAIL.primary.to_hsl().0;
    assert!(
        (35.0..=50.0).contains(&caret_h),
        "Firetail caret hue {caret_h:.1}° must stay ember-gold"
    );
    assert!(
        hue_gap(caret_h, lo.to_hsl().0) >= 45.0
            && hue_gap(caret_h, hi.to_hsl().0) >= 45.0,
        "Firetail's ember caret must stay at least 45° clear of both wine-lava tones"
    );
    assert!(
        redmean(FIRETAIL.base_content, FIRETAIL.base_100) >= 500.0,
        "Firetail blush ink must keep strong contrast over the oxblood ground"
    );
    assert!(
        redmean(FIRETAIL.primary, FIRETAIL.base_100) >= 300.0,
        "Firetail ember caret must remain immediately visible over the ground"
    );
    assert_eq!(edge, model::LavaEdge::Glow, "Firetail keeps its authored glow");
    assert!(!dithered, "Firetail stays smooth; Mangrove owns lava dither");
}

/// NUMERIC INTER-WORLD DISTINCTNESS law: compare Firetail's WHOLE authored token
/// vector (not merely its animated-background enum) against every other world by
/// RMS redmean distance. A copied palette scores zero; a near-copy cannot
/// hide behind a different ground shader or font. The 70-point RMS floor is a
/// clear multi-token separation while leaving individual quiet rungs coherent.
#[test]
fn firetail_palette_is_numerically_distinct_from_every_other_world() {
    fn redmean(a: Srgb, b: Srgb) -> f32 {
        let rbar = (a.r as f32 + b.r as f32) * 0.5;
        let dr = a.r as f32 - b.r as f32;
        let dg = a.g as f32 - b.g as f32;
        let db = a.b as f32 - b.b as f32;
        ((2.0 + rbar / 256.0) * dr * dr
            + 4.0 * dg * dg
            + (2.0 + (255.0 - rbar) / 256.0) * db * db)
            .sqrt()
    }
    fn tokens(t: &Theme) -> [Srgb; 10] {
        [
            t.base_100,
            t.base_200,
            t.base_300,
            t.base_content,
            t.muted,
            t.faint,
            t.primary,
            t.primary_content,
            t.error,
            Srgb::rgb(t.selection.r, t.selection.g, t.selection.b),
        ]
    }

    let fire = tokens(&FIRETAIL);
    for other in THEMES.iter().filter(|t| t.name != FIRETAIL.name) {
        let theirs = tokens(other);
        let rms = (fire
            .iter()
            .zip(theirs)
            .map(|(&a, b)| redmean(a, b).powi(2))
            .sum::<f32>()
            / fire.len() as f32)
            .sqrt();
        assert!(
            rms >= 70.0,
            "Firetail whole-palette distance from {} is only {rms:.1} RMS redmean (floor 70)",
            other.name
        );
    }
}

/// TAWNY↔MOPOKE DIFFERENTIATION law (Option A, 2026-07-22 — see MOPOKE's own
/// doc comment in `worlds.rs`): the pair used to ship a BYTE-IDENTICAL caret
/// (`#FFC05E`) and selection (`#3A6FD8`), measuring only 24.6 RMS redmean
/// whole-palette distance apart — awl's tightest pair. Locks the separation
/// so it can never regress back to identity: Mopoke's caret and selection
/// (RGB, ignoring the unchanged selection alpha) must each differ from
/// Tawny's, and the pair's whole-palette RMS (the SAME `redmean`/`tokens`
/// recipe [`firetail_palette_is_numerically_distinct_from_every_other_world`]
/// uses) must clear a floor comfortably above the old identical-pair value —
/// measured ~76.1 post-change, floor set at 60 for margin.
#[test]
fn tawny_and_mopoke_carets_and_selections_are_now_numerically_distinct() {
    fn redmean(a: Srgb, b: Srgb) -> f32 {
        let rbar = (a.r as f32 + b.r as f32) * 0.5;
        let dr = a.r as f32 - b.r as f32;
        let dg = a.g as f32 - b.g as f32;
        let db = a.b as f32 - b.b as f32;
        ((2.0 + rbar / 256.0) * dr * dr
            + 4.0 * dg * dg
            + (2.0 + (255.0 - rbar) / 256.0) * db * db)
            .sqrt()
    }
    fn tokens(t: &Theme) -> [Srgb; 10] {
        [
            t.base_100,
            t.base_200,
            t.base_300,
            t.base_content,
            t.muted,
            t.faint,
            t.primary,
            t.primary_content,
            t.error,
            Srgb::rgb(t.selection.r, t.selection.g, t.selection.b),
        ]
    }

    assert_ne!(
        TAWNY.primary, MOPOKE.primary,
        "the caret must no longer be byte-identical between Tawny and Mopoke"
    );
    assert_ne!(
        (TAWNY.selection.r, TAWNY.selection.g, TAWNY.selection.b),
        (MOPOKE.selection.r, MOPOKE.selection.g, MOPOKE.selection.b),
        "the selection tint must no longer be byte-identical between Tawny and Mopoke"
    );
    assert_eq!(
        TAWNY.selection.a, MOPOKE.selection.a,
        "the selection ALPHA is unchanged by this round — only the hue moved"
    );

    let (tawny, mopoke) = (tokens(&TAWNY), tokens(&MOPOKE));
    let rms = (tawny
        .iter()
        .zip(mopoke)
        .map(|(&a, b)| redmean(a, b).powi(2))
        .sum::<f32>()
        / tawny.len() as f32)
        .sqrt();
    assert!(
        rms >= 60.0,
        "Tawny-Mopoke whole-palette distance is only {rms:.1} RMS redmean (floor 60; \
         was 24.6 before the differentiation round)"
    );
}

/// The `Background::Lava` DATA accessors (exercised via a literal, since no world
/// ships it yet): it degrades to a FLAT margin ground (`from == to == ground`,
/// shader 0) that the lava overlay overdraws, names itself `"lava"`, is the ONLY
/// `is_lava()` variant, and surfaces its `(ground, blob_lo, blob_hi, edge,
/// dithered)` params. Plus the `LavaEdge` mask-mode / name contract.
#[test]
fn lava_background_accessors_are_a_flat_ground_plus_metaball_params() {
    let ground = Srgb::rgb(0x11, 0x27, 0x23);
    let lo = Srgb::rgb(0x17, 0x23, 0x2b);
    let hi = Srgb::rgb(0x22, 0x3c, 0x4f);
    let bg = Background::Lava { ground, blob_lo: lo, blob_hi: hi, edge: model::LavaEdge::Glow, dithered: true };
    // Degrades to a FLAT ground of the lava `ground`, shader 0 (no margin marks).
    assert_eq!(bg.shader_id(), 0);
    assert_eq!(bg.from(), ground);
    assert_eq!(bg.to(), ground, "flat: from == to");
    assert_eq!(bg.tint(), ground);
    assert!(!bg.edge(), "the Dots proximity flag is unrelated to LavaEdge");
    assert_eq!(bg.as_str(), "lava");
    // The one is_lava variant + its params.
    assert!(bg.is_lava());
    assert!(!Background::Gradient { from: ground, to: ground, dir: (0.0, 1.0) }.is_lava());
    assert_eq!(bg.lava_params(), Some((ground, lo, hi, model::LavaEdge::Glow, true)));
    assert_eq!(
        Background::Gradient { from: ground, to: ground, dir: (0.0, 1.0) }.lava_params(),
        None
    );
    // LavaEdge contract (the shader mask-mode selector + sidecar names).
    assert_eq!(model::LavaEdge::Hard.mask_mode(), 1.0);
    assert_eq!(model::LavaEdge::Glow.mask_mode(), 2.0);
    assert_eq!(model::LavaEdge::Hard.as_str(), "hard");
    assert_eq!(model::LavaEdge::Glow.as_str(), "glow");
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
/// bundled monospace families (IBM Plex Mono / JetBrains Mono / Monaspace Xenon /
/// Iosevka). A world whose DISPLAY face is already one of those monos REUSES its own
/// face (`mono == font`); every other world borrows a bundled mono (`mono != font`).
#[test]
fn every_world_has_a_bundled_mono() {
    const BUNDLED_MONOS: [&str; 4] =
        ["IBM Plex Mono", "JetBrains Mono", "Monaspace Xenon", "Iosevka"];
    // The worlds whose DISPLAY face is itself a bundled mono (so they reuse it).
    // Wagtail was the FIFTH (sharing Mangrove's JetBrains Mono); Firetail is the
    // SIXTH — it derives from Potoroo's warm den and shares its Monaspace Xenon
    // slab-mono display (a logged, honest consequence of adding worlds faster than
    // bundled display faces; see `worlds.rs::FIRETAIL`'s own doc comment).
    // Cassowary (the NERV terminal) is the SEVENTH — it shares Currawong's
    // Iosevka as the terminal-readout face for both display and code.
    const MONO_DISPLAY: [&str; 7] =
        ["Tawny", "Currawong", "Potoroo", "Mangrove", "Wagtail", "Firetail", "Cassowary"];
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
    assert_eq!(CURRAWONG.mono, "Iosevka");
    assert_eq!(POTOROO.mono, "Monaspace Xenon");
    assert_eq!(MANGROVE.mono, "JetBrains Mono");
    assert_eq!(WAGTAIL.mono, "JetBrains Mono"); // shares Mangrove's exact display font (logged)
    // And a couple of the borrowed assignments.
    assert_eq!(SALTPAN.mono, "Monaspace Xenon"); // Fraunces serif → slab-serif mono
    assert_eq!(BOWERBIRD.mono, "JetBrains Mono"); // cool technical navy → crisp mono
    assert_eq!(GALAH.mono, "IBM Plex Mono"); // warm humanist sans → warm humanist mono
}

/// Every world declares a per-theme CJK (Japanese) fallback list whose
/// CHARACTER matches the world. After the Phase 2 "JP face variety" round
/// there are FIVE possible ladders (up from two), each still ordered
/// BUNDLED-first then mac-primary (Hiragino) then linux-fallback (Noto CJK):
/// the neutral MINCHO/GOTHIC pair for the worlds this round left alone, plus
/// three per-world overrides — SHIPPORI (bookish serif) for the warm
/// book-serif worlds, ZENMARU (rounded sans) for the two dedicated sans
/// worlds, and KLEE (kaisho/brush) for the two Klee worlds (so their JA
/// matches their ZH's WenKai). Mirrors the shape of
/// `zh_hans_ladder_matches_world_character_with_klee_override`.
#[test]
fn cjk_fallback_matches_world_character() {
    let shippori = ["Gumtree", "Bilby", "Bombora"];
    let zenmaru = ["Galah", "Bowerbird"];
    let klee = ["Mopoke", "Quokka"];
    let mincho = ["Saltpan", "Mulga", "Magpie"]; // neutral serif (Noto Serif JP)
    let gothic = ["Tawny", "Potoroo", "Mangrove", "Currawong", "Wagtail", "Firetail", "Brolga", "Cassowary"]; // neutral sans/mono (Noto Sans JP)
    for t in THEMES.iter() {
        assert!(!t.cjk.is_empty(), "{} has no CJK fallback list", t.name);
        if shippori.contains(&t.name) {
            assert_eq!(t.cjk, CJK_JA_SHIPPORI, "{} is a book-serif world -> Shippori JA", t.name);
        } else if zenmaru.contains(&t.name) {
            assert_eq!(t.cjk, CJK_JA_ZENMARU, "{} is a sans world -> Zen Maru JA", t.name);
        } else if klee.contains(&t.name) {
            assert_eq!(t.cjk, CJK_JA_KLEE, "{} is a Klee world -> Klee One JA", t.name);
        } else if mincho.contains(&t.name) {
            assert_eq!(t.cjk, CJK_MINCHO, "{} is a neutral serif world -> mincho JA", t.name);
        } else if gothic.contains(&t.name) {
            assert_eq!(t.cjk, CJK_GOTHIC, "{} is a neutral sans/mono world -> gothic JA", t.name);
        } else {
            panic!("{} not classified for CJK fallback", t.name);
        }
    }
    // Priority order: bundled face first, macOS Hiragino, Linux Noto CJK. The
    // three variety ladders keep the NEUTRAL Noto face as their bundled floor
    // (so `AWL_CJK_FORCE=floor` drops cleanly to it; never-tofu unchanged).
    assert_eq!(CJK_MINCHO, &["Noto Serif JP", "Hiragino Mincho ProN", "Noto Serif CJK JP"]);
    assert_eq!(CJK_GOTHIC, &["Noto Sans JP", "Hiragino Kaku Gothic ProN", "Noto Sans CJK JP"]);
    assert_eq!(
        CJK_JA_SHIPPORI,
        &["Shippori Mincho", "Noto Serif JP", "Hiragino Mincho ProN", "Noto Serif CJK JP"]
    );
    assert_eq!(
        CJK_JA_ZENMARU,
        &["Zen Maru Gothic", "Noto Sans JP", "Hiragino Kaku Gothic ProN", "Noto Sans CJK JP"]
    );
    assert_eq!(
        CJK_JA_KLEE,
        &["Klee One", "Noto Sans JP", "Hiragino Kaku Gothic ProN", "Noto Sans CJK JP"]
    );
}

/// THE NEVER-TOFU LAW (structural half — the environment-independent part
/// of it): every [`FontId`] has a NON-EMPTY candidate ladder on EVERY
/// world. This is the actual regression the law guards against — a world
/// accidentally shipping an empty ladder for a script would guarantee
/// tofu with no possible resolution, regardless of what's installed on
/// the machine running awl. (The COMPLEMENTARY half — that `Latin`/`Ja`
/// always resolve to a concretely-registered face via the real font DB —
/// is `render::tests::cjk::latin_and_ja_always_resolve_to_an_embedded_face`,
/// since it needs a built `FontSystem` to check against.)
#[test]
fn every_font_id_has_a_nonempty_candidate_ladder_on_every_world() {
    for t in THEMES.iter() {
        for id in ALL_FONT_IDS {
            assert!(
                !t.candidates(id).is_empty(),
                "{} has an EMPTY candidate ladder for {:?} — guaranteed tofu",
                t.name,
                id
            );
        }
    }
}

/// Every world's [`Theme::ornament_face`] is exactly one of the THREE bundled
/// ornament faces — no world ships an unregistered / typo'd family that would
/// tofu the section-break fleuron. (The font-DB half — that each face actually
/// COVERS its world's glyphs — is `render::tests::cjk::
/// ornament_glyphs_resolve_in_each_worlds_assigned_face`, which needs a built
/// `FontSystem`.) Also pins `ORNAMENT_MARKS == render::SYMBOL_FAMILY`, the one
/// coupling `theme.rs` states as data rather than importing.
#[test]
fn every_world_ornament_face_is_a_registered_ornament_face() {
    assert_eq!(
        ORNAMENT_MARKS,
        crate::render::SYMBOL_FAMILY,
        "the geometric worlds' ornament face IS the merged marks face"
    );
    for t in THEMES.iter() {
        assert!(
            matches!(
                t.ornament_face,
                ORNAMENT_GARAMOND | ORNAMENT_JUNICODE | ORNAMENT_MARKS
            ),
            "{} has an unrecognized ornament_face {:?}",
            t.name,
            t.ornament_face
        );
        // The design-table contract: THREE DISTINCT symbols per world (dash /
        // star / underscore), so a break's ornament tracks the syntax the author
        // typed instead of collapsing to one shared mark. (The font-DB half —
        // that each glyph actually resolves in `ornament_face` — is the render
        // test `ornament_glyphs_resolve_in_each_worlds_assigned_face`.)
        let (d, s, u) = (t.ornaments.dash, t.ornaments.star, t.ornaments.underscore);
        assert!(
            d != s && s != u && d != u,
            "{} ornament trio is not three distinct glyphs: dash={:?} star={:?} underscore={:?}",
            t.name,
            d,
            s,
            u
        );
    }
}

/// NEVER-DRIFT law: every world ships an [`Theme::ornament_scale`], and it is
/// exactly one of the three named tier constants — a world can't silently drift to
/// a bare literal that neither reader (`md_line_scale` / `prepare_ornaments`) would
/// then keep in lockstep. Also pins the three tier VALUES (the taste defaults) and
/// a sample world per tier, keyed to the ornament's CHARACTER.
#[test]
fn every_world_has_an_ornament_scale() {
    // The three tiers are the settled taste defaults.
    assert_eq!(ORNAMENT_SCALE_ORNATE, 2.2, "ornate tier is 2.2");
    assert_eq!(ORNAMENT_SCALE_FLEURON, 1.8, "fleuron tier is 1.8");
    assert_eq!(ORNAMENT_SCALE_GEOMETRIC, 1.5, "geometric tier is 1.5");
    assert!(
        ORNAMENT_SCALE_ORNATE > ORNAMENT_SCALE_FLEURON
            && ORNAMENT_SCALE_FLEURON > ORNAMENT_SCALE_GEOMETRIC,
        "the tiers descend ornate > fleuron > geometric"
    );

    // Every world's scale IS one of the three tiers — no stray literal.
    for t in THEMES.iter() {
        assert!(
            matches!(
                t.ornament_scale,
                ORNAMENT_SCALE_ORNATE | ORNAMENT_SCALE_FLEURON | ORNAMENT_SCALE_GEOMETRIC
            ),
            "{} has an off-tier ornament_scale {}",
            t.name,
            t.ornament_scale
        );
    }

    // One sample per tier (the spec's pinned assignments).
    let by = |name: &str| set_active_by_name(name).unwrap().ornament_scale;
    let _t = crate::testlock::serial();
    assert_eq!(by("Mopoke"), 2.2, "Mopoke (Junicode flowers) is ornate 2.2");
    assert_eq!(by("Bombora"), 1.8, "Bombora (Garamond fleurons) is fleuron 1.8");
    assert_eq!(by("Currawong"), 1.5, "Currawong (geometric marks) is geometric 1.5");
    set_active(DEFAULT_THEME);
}

/// NEVER-DRIFT law (per-world LIST BULLETS): every world ships a three-glyph
/// [`Theme::bullets`] triple (item 15's per-level rotation) whose three levels
/// are PAIRWISE DISTINCT, and a [`Theme::bullet_scale`] that is exactly one of
/// the two named tier constants (no stray literal). The font-DB half — that
/// each glyph actually resolves in the world's [`Theme::ornament_face`] — is
/// `render::tests::markdown::bullet_glyphs_resolve_in_each_worlds_assigned_face`.
/// Also pins the geometric worlds to the plain byte-identical
/// [`BULLETS_PLAIN`]/[`BULLET_SCALE_PLAIN`] (restraint) and the manicule
/// showpiece (Bombora's level-1 ☞, exclusive to that one level).
#[test]
fn every_world_has_a_bullet_pair() {
    assert_eq!(BULLETS_PLAIN, ('•', '◦', '▪'), "the plain bullet triple is • / ◦ / ▪");
    assert_eq!(BULLET_SCALE_PLAIN, 1.0, "plain bullets keep body size");
    assert!(
        BULLET_SCALE_ORNAMENT > 0.0 && BULLET_SCALE_ORNAMENT < BULLET_SCALE_PLAIN,
        "ornament bullets shape smaller than the plain body-size bullets"
    );
    for t in THEMES.iter() {
        assert_ne!(
            t.bullets.0, t.bullets.1,
            "{}: levels 1/2 must be distinct glyphs, got {:?}",
            t.name, t.bullets
        );
        assert_ne!(
            t.bullets.1, t.bullets.2,
            "{}: levels 2/3 must be distinct glyphs, got {:?}",
            t.name, t.bullets
        );
        assert_ne!(
            t.bullets.0, t.bullets.2,
            "{}: levels 1/3 must be distinct glyphs, got {:?}",
            t.name, t.bullets
        );
        // OFF-TIER EXCEPTION (theme-QA padding round, EXACTLY one now, pinned by
        // NAME and VALUE — never a loose "any float passes" escape hatch): the
        // shared [`BULLET_SCALE_ORNAMENT`] tier is a byproduct of two unrelated
        // font metrics (see that constant's own doc) that paired badly on
        // Bombora's manicule (too wide, touched the following text), so Bombora
        // carries its OWN literal instead. (Mopoke once needed one too — its
        // rosette stranded in a canyon under iA Writer Quattro S's wide duospaced
        // advance — but queue item 30 moved Mopoke's body face to the proportional
        // Bitter, whose narrower marker advance lets the rosette sit right on the
        // shared tier; that exception retired with the old face.) Every other
        // world stays on a shared tier.
        let off_tier_exception = match t.name {
            "Bombora" => Some(BOMBORA.bullet_scale),
            _ => None,
        };
        assert!(
            matches!(t.bullet_scale, BULLET_SCALE_PLAIN | BULLET_SCALE_ORNAMENT)
                || off_tier_exception == Some(t.bullet_scale),
            "{}: off-tier bullet_scale {} (not a logged theme-QA padding exception)",
            t.name,
            t.bullet_scale
        );
        // The geometric/technical worlds keep the plain pair AND body size, in
        // lockstep — a characterful pair at body size (or plain at half) would be
        // a taste drift; a geometric world is byte-identical to before this round.
        // The two off-tier exceptions are excluded from this lockstep check (their
        // whole POINT is a bullet_scale that differs from the shared ORNAMENT tier
        // while keeping a characterful, non-plain pair).
        let geometric = t.ornament_face == ORNAMENT_MARKS;
        if off_tier_exception.is_none() {
            assert_eq!(
                t.bullets == BULLETS_PLAIN,
                t.bullet_scale == BULLET_SCALE_PLAIN,
                "{}: plain-pair and plain-scale must agree (geometric restraint)",
                t.name
            );
        }
        if geometric {
            assert_eq!(
                t.bullets, BULLETS_PLAIN,
                "{}: an Awl-Marks world keeps the plain • / ◦ (restraint)",
                t.name
            );
        } else {
            assert_ne!(
                t.bullets, BULLETS_PLAIN,
                "{}: an antique/literary serif world draws a characterful bullet",
                t.name
            );
        }
    }
    // The TRIPLE CYCLES every THREE levels (item 15's per-level rotation) —
    // depth 0/1 land exactly where the pre-item-15 two-level cycle put them,
    // depth 2 is the new third rung, and depth 3 wraps back to level 1.
    assert_eq!(TAWNY.bullet_for_depth(0), '•');
    assert_eq!(TAWNY.bullet_for_depth(1), '◦');
    assert_eq!(TAWNY.bullet_for_depth(2), '▪');
    assert_eq!(TAWNY.bullet_for_depth(3), '•');
    assert_eq!(TAWNY.bullet_for_depth(4), '◦');
    assert_eq!(TAWNY.bullet_for_depth(5), '▪');
    assert_eq!(BOMBORA.bullet_for_depth(0), '☞');
    assert_eq!(BOMBORA.bullet_for_depth(1), '❧');
    assert_eq!(BOMBORA.bullet_for_depth(2), '❦');
    assert_eq!(BOMBORA.bullet_for_depth(3), '☞');
    // The manicule showpiece: Bombora alone rides the antique pointing hand,
    // at its top level (level 1) — NEVER at level 3 either (the rotation
    // composes with, never dilutes, item 7's "one world, one level" pick).
    assert_eq!(BOMBORA.bullets.0, '☞', "Bombora's level-1 bullet is the manicule");
    assert!(
        THEMES
            .iter()
            .filter(|t| t.bullets.0 == '☞' || t.bullets.1 == '☞' || t.bullets.2 == '☞')
            .count()
            == 1,
        "exactly one world uses the manicule bullet, at exactly one level (a hand everywhere is loud)"
    );
}

/// NEVER-DRIFT law (item 15, per-world LIST-ITEM INDENT): every world's
/// [`Theme::list_indent_scale`] is exactly one of the two named tier constants
/// (no stray literal, mirroring [`every_world_has_a_bullet_pair`]'s
/// `bullet_scale` sweep) and — since the shared tier IS the shared bullet-scale
/// tier's own roster — agrees with the world's own bullet PAIR: a plain `•`/
/// `◦`/`▪` world stays at the byte-identical [`LIST_INDENT_SCALE_PLAIN`], an
/// antique/literary-serif world (hedera/fleuron/manicule) steps up to
/// [`LIST_INDENT_SCALE_WIDE`]. `>= 1.0` on every world: item 15 only ever
/// WIDENS the typed indent, never narrows it below what the raw spaces alone
/// already give.
#[test]
fn every_world_has_a_list_indent_scale() {
    assert_eq!(LIST_INDENT_SCALE_PLAIN, 1.0, "the plain tier is byte-identical");
    assert!(
        LIST_INDENT_SCALE_WIDE > LIST_INDENT_SCALE_PLAIN,
        "the wide tier must actually widen the indent"
    );
    for t in THEMES.iter() {
        assert!(
            t.list_indent_scale == LIST_INDENT_SCALE_PLAIN
                || t.list_indent_scale == LIST_INDENT_SCALE_WIDE,
            "{}: off-tier list_indent_scale {}",
            t.name,
            t.list_indent_scale
        );
        assert!(t.list_indent_scale >= 1.0, "{}: indent scale must never shrink the typed indent", t.name);
        let plain_pair = t.bullets == BULLETS_PLAIN;
        assert_eq!(
            t.list_indent_scale == LIST_INDENT_SCALE_PLAIN,
            plain_pair,
            "{}: plain-pair and plain-indent-scale must agree",
            t.name
        );
    }
}

/// `Theme::candidates` for `Latin` is always exactly the world's own
/// [`Theme::font`] — a single-element floor, never a fallback list.
#[test]
fn latin_candidates_is_the_worlds_own_display_face() {
    for t in THEMES.iter() {
        assert_eq!(t.candidates(FontId::Latin), vec![t.font], "{}", t.name);
    }
}

/// THE CHINESE ROUND: zh-Hans now mirrors `cjk_fallback_matches_world_character`
/// exactly — SERIF worlds get [`CJK_ZH_HANS_SERIF`] (bundled Noto Serif SC),
/// SANS/MONO worlds get [`CJK_ZH_HANS_SANS`] (bundled Noto Sans SC), EXCEPT the
/// two Klee-derived worlds (Mopoke, Quokka) which get the CHARACTERFUL
/// [`CJK_ZH_HANS_KLEE`] override (bundled LXGW WenKai first). zh-Hant/ko remain
/// v1-uniform (zh-Hant: still no bundled asset at all; ko: one bundled face,
/// no serif/sans split yet — both documented taste calls, logged above).
#[test]
fn zh_hans_ladder_matches_world_character_with_klee_override() {
    let mincho = ["Gumtree", "Saltpan", "Bilby", "Bombora", "Mulga", "Magpie"];
    let klee = ["Mopoke", "Quokka"];
    let gothic = ["Tawny", "Potoroo", "Mangrove", "Galah", "Bowerbird", "Currawong", "Wagtail", "Firetail", "Brolga", "Cassowary"];
    for t in THEMES.iter() {
        assert!(!t.zh_hans.is_empty(), "{} has no zh-Hans candidate list", t.name);
        if klee.contains(&t.name) {
            assert_eq!(t.zh_hans, CJK_ZH_HANS_KLEE, "{} is a Klee world -> WenKai zh-Hans", t.name);
        } else if mincho.contains(&t.name) {
            assert_eq!(t.zh_hans, CJK_ZH_HANS_SERIF, "{} is a serif world -> Serif SC zh-Hans", t.name);
        } else if gothic.contains(&t.name) {
            assert_eq!(t.zh_hans, CJK_ZH_HANS_SANS, "{} is a sans/mono world -> Sans SC zh-Hans", t.name);
        } else {
            panic!("{} not classified for zh-Hans fallback", t.name);
        }
    }
    assert_eq!(CJK_ZH_HANS_SERIF, &["Noto Serif SC", "PingFang SC", "Noto Sans CJK SC"]);
    assert_eq!(CJK_ZH_HANS_SANS, &["Noto Sans SC", "PingFang SC", "Noto Sans CJK SC"]);
    assert_eq!(
        CJK_ZH_HANS_KLEE,
        &["LXGW WenKai", "Noto Sans SC", "PingFang SC", "Noto Sans CJK SC"]
    );
}

/// zh-Hant stays v1-uniform across every world — it still has NO bundled
/// asset (Big5 subsetting is banked, not attempted). ko, HOWEVER, now
/// carries a serif/sans split after the "CJK companions" round: the SERIF
/// worlds (same six that get [`CJK_ZH_HANS_SERIF`]) get [`CJK_KO_SERIF`]
/// (bundled Gowun Batang first), the SANS/MONO worlds keep the plain
/// [`CJK_KO`] (Noto Sans KR) floor — mirroring the ja/zh-Hans serif/sans
/// split's shape (`cjk_fallback_matches_world_character`,
/// `zh_hans_ladder_matches_world_character_with_klee_override`).
#[test]
fn zh_hant_uniform_ko_splits_serif_from_sans() {
    // The SERIF worlds — exactly the ones whose zh_hans is CJK_ZH_HANS_SERIF
    // (Theme::cjk is a mincho-family ja ladder). Kept as an explicit roster so
    // a world silently switching character fails HERE, not as a tofu box.
    let serif = ["Gumtree", "Bilby", "Bombora", "Saltpan", "Mulga", "Magpie"];
    for t in THEMES.iter() {
        assert_eq!(t.zh_hant, CJK_ZH_HANT, "{}: zh-Hant stays uniform", t.name);
        if serif.contains(&t.name) {
            assert_eq!(t.ko, CJK_KO_SERIF, "{} is a serif world -> Gowun Batang ko", t.name);
            // A serif world's ko is a mincho-family ja ladder, never gothic.
            assert!(
                t.zh_hans == CJK_ZH_HANS_SERIF,
                "{} classified serif for ko but not for zh-Hans — the two must agree",
                t.name
            );
        } else {
            assert_eq!(t.ko, CJK_KO, "{} is a sans/mono world -> Noto Sans KR ko", t.name);
        }
    }
    assert_eq!(CJK_ZH_HANT, &["PingFang TC", "Noto Sans CJK TC"]);
    assert_eq!(CJK_KO, &["Noto Sans KR", "Apple SD Gothic Neo", "Noto Sans CJK KR"]);
    // Gowun Batang FIRST (the bundled characterful serif Korean), then the
    // SAME Noto Sans KR bundled floor CJK_KO uses (the AWL_CJK_FORCE=floor
    // target), then serif-first system trailing candidates.
    assert_eq!(
        CJK_KO_SERIF,
        &[
            "Gowun Batang",
            "Noto Sans KR",
            "AppleMyungjo",
            "Noto Serif CJK KR",
            "Apple SD Gothic Neo",
            "Noto Sans CJK KR",
        ]
    );
    // The floor CJK_KO_SERIF drops to under AWL_CJK_FORCE=floor is exactly
    // CJK_KO's bundled floor — so the ko-worlds gallery's "floor" side is the
    // plain Noto Sans KR, machine-independent.
    assert_eq!(CJK_KO_SERIF[1], CJK_KO[0], "ko-serif floor == the bundled Noto Sans KR floor");
}

/// AXIS COVERAGE RULER (the reason [`Lens`] + [`ThemeTags`] survive after the theme
/// picker's runtime lens strip was retired, 2026-07-15): every declared axis SECTION
/// stays covered by a curated band of worlds, so the axes remain a meaningful
/// build-time description of the roster. A world may OPT OUT (`None`) of an axis, but
/// any `Some(tag)` must be one of that axis's declared sections (no world under a
/// header that doesn't exist); the name-keyed accessor [`tag_for`] agrees with the
/// inline field; every world HEADLINES at least one axis; and `All` groups nothing.
/// THIS is the coverage check meant by "the axes become a build-time ruler"
/// (retired; decision recorded in THEMES.md) — no runtime picker consults it.
#[test]
fn axis_coverage_ruler() {
    for lens in [Lens::Time, Lens::Register, Lens::Voice, Lens::Temperature] {
        let sections = lens.sections();
        for t in THEMES.iter() {
            if let Some(tag) = t.tags.section(lens) {
                assert!(
                    sections.contains(&tag),
                    "{} has invalid {:?} tag {:?} (not in {:?})",
                    t.name,
                    lens,
                    tag,
                    sections
                );
            }
            // The name-keyed accessor agrees with the inline field.
            assert_eq!(tag_for(t.name, lens), t.tags.section(lens), "{} tag_for disagrees", t.name);
        }
        // Every declared header shows a CURATED band of worlds: never an empty
        // faint header, never the pre-curation crowd (Time=Night once held 6). The
        // upper bound widened 3→4 when the roster grew to sixteen — the sixteenth
        // world (Firetail, the warm lava statement world) headlines Temperature=Warm,
        // which every section was already at its 3-cap when it arrived; 4 is still
        // curated, nowhere near the old crowd.
        for sect in sections {
            let n = THEMES
                .iter()
                .filter(|t| t.tags.section(lens) == Some(*sect))
                .count();
            assert!(
                (2..=4).contains(&n),
                "{:?} section {sect:?} shows {n} worlds (curation wants 2–4)",
                lens
            );
        }
    }
    // Every world headlines at least ONE axis (present under some section), so no
    // world is invisible to the coverage ruler.
    for t in THEMES.iter() {
        let shown = [Lens::Time, Lens::Register, Lens::Voice, Lens::Temperature]
            .iter()
            .any(|&l| t.tags.section(l).is_some());
        assert!(shown, "{} headlines no axis", t.name);
    }
    // The degenerate All axis groups nothing.
    assert!(Lens::All.sections().is_empty());
    assert_eq!(THEMES[0].tags.section(Lens::All), None);
    // The ruler's STRIP shape: All parked FIRST, five axes total.
    assert_eq!(*Lens::STRIP.first().unwrap(), Lens::All);
    assert_eq!(Lens::STRIP.len(), 5);
}

#[test]
fn default_is_saltpan() {
    // 2026-07-11 taste round: Saltpan (a warm light world) is awl's first
    // impression now, not the original dark Tawny (see `DEFAULT_THEME`'s doc).
    assert!(!THEMES[DEFAULT_THEME].dark);
    assert_eq!(THEMES[DEFAULT_THEME].name, "Saltpan");
}

/// DEBT-AUDIT LAW (2026-07-18) — INDEX-VS-NAME world access. A world INSERTED
/// mid-roster must not change any OTHER world's behaviour or a user's PERSISTED
/// selection. The two things that could break on such an insertion are:
///   (1) a position-derived constant (only `DEFAULT_THEME`, now name-derived via
///       `world_index("Saltpan")`), and
///   (2) the sticky-theme round-trip, which stores a NAME (`config.toml`'s
///       `theme` key via `App::persist_theme` → `Config::apply_sticky_globals` →
///       `set_active_by_name`), never an array index.
/// This law pins BOTH so a future roster insert can't silently repoint the
/// default or resurface a user under a different world:
///   - names are UNIQUE (name-addressing is well-defined),
///   - EVERY world round-trips through `set_active_by_name` back to itself
///     (the persisted-selection path is position-independent for all worlds),
///   - the default is name-derived (so a FRESH launch is insertion-stable too),
///   - a NON-world name is `None` (a stale/retired name falls back leniently,
///     never a crash and never a neighbour by position).
#[test]
fn roster_position_is_name_stable() {
    let _g = crate::testlock::serial();

    // (1) Names are unique — name-addressing has exactly one target per name.
    for (i, a) in THEMES.iter().enumerate() {
        for b in THEMES.iter().skip(i + 1) {
            assert_ne!(a.name, b.name, "two worlds share the name {:?}", a.name);
        }
    }

    // (2) Persisted selection is a NAME: every world round-trips to ITSELF
    // regardless of its array position, so inserting a world before/after any
    // other cannot change which world that other's remembered name reopens.
    for t in THEMES.iter() {
        let got = set_active_by_name(t.name)
            .unwrap_or_else(|| panic!("{} unreachable by its own name", t.name));
        assert_eq!(got.name, t.name);
        // Case-insensitive too (the config value is compared ASCII-insensitively).
        assert_eq!(
            set_active_by_name(&t.name.to_ascii_lowercase()).unwrap().name,
            t.name
        );
    }

    // (3) The FRESH-launch default is name-derived — a mid-roster insert leaves
    // it on Saltpan by construction (this is the const `world_index("Saltpan")`,
    // re-checked here so the property is a test, not only a compile-time fact).
    assert_eq!(THEMES[DEFAULT_THEME].name, "Saltpan");

    // (4) A name that is NOT a world falls back leniently to None (never a
    // panic, never a by-position neighbour) — the door retired names lean on.
    assert!(set_active_by_name("NotAWorld").is_none());

    set_active(DEFAULT_THEME);
}

/// RETIRED-WORLD LENIENT FALLBACK (2026-07-18 rename: Outback→Mulga,
/// Kingfisher→Bowerbird, Undertow→Bombora). A `config.toml` that still names one
/// of the three RETIRED worlds — a user who upgrades with `theme = "Outback"`
/// persisted — must not crash and must not resurface a neighbour by position:
/// `set_active_by_name` returns `None` for each retired name, and the config
/// apply seam (`Config::apply_sticky_globals`) discards that `None`, so the
/// built-in default (Saltpan) is kept. This test pins the NAME half; the
/// apply-seam half lives in `config::tests`.
#[test]
fn retired_world_names_fall_back_leniently() {
    let _g = crate::testlock::serial();
    for retired in ["Outback", "Kingfisher", "Undertow"] {
        assert!(
            set_active_by_name(retired).is_none(),
            "retired world {retired:?} must resolve to None (lenient fallback), not a live world"
        );
        // Case-insensitive: a lower-cased persisted value is equally retired.
        assert!(
            set_active_by_name(&retired.to_ascii_lowercase()).is_none(),
            "retired world {retired:?} (lower-cased) must resolve to None"
        );
    }
    // The successor names DO resolve (the rename actually landed).
    assert_eq!(set_active_by_name("Mulga").unwrap().name, "Mulga");
    assert_eq!(set_active_by_name("Bowerbird").unwrap().name, "Bowerbird");
    assert_eq!(set_active_by_name("Bombora").unwrap().name, "Bombora");
    set_active(DEFAULT_THEME);
}

#[test]
fn cycle_wraps_both_ways() {
    let _g = crate::testlock::serial();
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
    let _g = crate::testlock::serial();
    assert_eq!(set_active_by_name("quokka").unwrap().name, "Quokka");
    assert_eq!(set_active_by_name("MULGA").unwrap().name, "Mulga");
    assert!(set_active_by_name("nope").is_none());
    set_active(DEFAULT_THEME);
}

#[test]
fn surface_selected_is_an_opaque_ramp_step_past_base_300() {
    let _g = crate::testlock::serial();
    for (i, t) in THEMES.iter().enumerate() {
        set_active(i);
        let band = surface_selected();
        // A SOLID band (figure/ground by VALUE), never the translucent selection.
        assert_eq!(band.a, 0xFF, "{} band must be opaque", t.name);
        // TRUE 1-BIT WORLDS (`Theme::is_one_bit`): a DECLARED exemption from
        // "must not be the selection token" — with only two legal values,
        // `surface_selected` (the elevation BORDER, pure white) and
        // `selection` (now also pure OPAQUE white — see the test above) are
        // necessarily the SAME literal color; they're distinguished by SHAPE/
        // CONTEXT (a thin border rim vs. a punched-outline selection band),
        // never by hue or translucency, which no longer exist to distinguish
        // them with. See THEMES.md's "The 1-bit law".
        if t.is_one_bit() {
            assert_eq!(band, t.selection, "{}: one-bit surface_selected and selection are necessarily the same pure white", t.name);
            continue;
        }
        assert_ne!(band, t.selection, "{} band must not be the selection token", t.name);
        // Each channel continues the base_200 -> base_300 step SELECTED_BAND_STEPS
        // more increments, or saturates at the gamut edge (never reverses direction).
        let want = SELECTED_BAND_STEPS;
        for (lo, hi, got) in [
            (t.base_200.r, t.base_300.r, band.r),
            (t.base_200.g, t.base_300.g, band.g),
            (t.base_200.b, t.base_300.b, band.b),
        ] {
            let dir = hi as i32 - lo as i32; // ramp direction (toward the ink)
            let step = got as i32 - hi as i32; // band's move past base_300
            if dir > 0 {
                assert!(step >= 0 && (got == 255 || step == dir * want), "{} band channel reversed", t.name);
            } else if dir < 0 {
                assert!(step <= 0 && (got == 0 || step == dir * want), "{} band channel reversed", t.name);
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
        // TRUE 1-BIT WORLDS (`Theme::is_one_bit`): a DECLARED exemption from
        // "selection is THE translucent token" — any alpha strictly between 0
        // and 255 composites a forbidden grey over this world's pure ground,
        // so selection is pure OPAQUE white instead (`0xFF`), with legibility
        // over selected text carried by a separate render-side mechanism (the
        // DITHER round's TRUE inverse-video pipeline,
        // `TextPipeline::selection_invert`), not by this token's alpha. See
        // THEMES.md's "The 1-bit law".
        if t.is_one_bit() {
            assert_eq!(t.selection.a, 0xFF, "{}: one-bit selection must be fully OPAQUE", t.name);
            continue;
        }
        // Selection is the ONE translucent token — a calm highlight, never opaque
        // (a paint fill) nor so sheer it fails the contrast floor. The exact alpha
        // is PER-WORLD now: most sit at 0x52, but a world whose composited
        // selection would be sub-glance over its own ground lifts it (Bombora /
        // Mangrove → 0x60) to clear `ink_ladder_and_selection_laws_*`.
        assert!(
            (0x40..0xA0).contains(&t.selection.a),
            "{} selection alpha {:#04x} outside the calm-translucent band [0x40, 0xA0)",
            t.name, t.selection.a
        );
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
        // TRUE 1-BIT WORLDS (`Theme::is_one_bit`): a DECLARED exemption — the
        // panel/pill's "OFF" answer (base_200 flush with the ground, so the
        // WYSIWYG affordance is genuinely invisible) is the whole point on a
        // world with only two legal values and no border companion for this
        // specific primitive; see THEMES.md's "The 1-bit law".
        if t.is_one_bit() {
            assert_eq!(t.base_200, t.base_100, "{}: one-bit base_200 stays flush with the ground (the panel/pill's OFF answer)", t.name);
            continue;
        }
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
        // TRUE 1-BIT WORLDS (`Theme::is_one_bit`, Wagtail's 2026-07 rework):
        // a DECLARED exemption, not a weakening — a real (non-degenerate)
        // gradient necessarily interpolates through forbidden intermediate
        // greys between its two endpoints, so a one-bit world's margin ground
        // must be the ONE `Background` variant guaranteed not to (a flat
        // `Gradient` with `from == to`, mathematically the same color at
        // every pixel). See THEMES.md's "The 1-bit law".
        if t.is_one_bit() {
            assert_eq!(
                bg.from(), bg.to(),
                "{}: a one-bit world's margin gradient must be FLAT (from == to) — \
                 any real gradient interpolates through forbidden greys", t.name
            );
            continue;
        }
        // LAVA WORLDS (`Background::Lava`, Firetail/Mangrove): a DECLARED exemption,
        // not a weakening — the base margin ground is DELIBERATELY flat (from == to
        // == the lava `ground`), because the lava OVERLAY (`crate::lava`, a separate
        // pipeline drawn after this margin pass) carries all the marks + motion and
        // OVERDRAWS the margins opaquely; the flat base is only there so the floor is
        // painted before the overlay draws. See `Background::Lava`'s shader_id() doc.
        if t.background.is_lava() {
            assert_eq!(
                bg.from(), bg.to(),
                "{}: a lava world's BASE margin ground must be FLAT (the lava overlay \
                 carries the motion)", t.name
            );
            continue;
        }
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

/// The sixteen worlds map onto at least SIX CLEARLY-distinct display faces
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

/// Queue item 30 (user + fable): Mopoke's body face is the warm slab Bitter
/// (shared with Magpie — precedented face-sharing, no new asset) and its
/// nested-bullet triple is a one-register, weight-descends-with-depth ornament
/// set (a solid damask rosette → its open four-fold sibling → a small foliate
/// sprig), all three in Mopoke's Junicode ornament face. This pins the DATA off
/// any GPU; the render laws
/// `render::tests::markdown::bullet_glyphs_resolve_in_each_worlds_assigned_face`
/// (they resolve) and `..::bullet_glyph_never_touches_the_following_text_in_any_world`
/// (they never touch the text) cover the appearance half.
#[test]
fn mopoke_body_face_is_bitter_with_the_item_30_bullet_triple() {
    assert_eq!(MOPOKE.font, "Bitter", "Mopoke's body face is the warm slab Bitter");
    assert_eq!(MOPOKE.mono, "IBM Plex Mono", "Mopoke keeps IBM Plex Mono for code");
    assert_eq!(
        MOPOKE.bullets,
        ('\u{E670}', '\u{EF92}', '\u{E67D}'),
        "Mopoke's bullet triple descends in weight within one ornament register"
    );
    // Face-sharing is precedented, never a new asset: Magpie draws in Bitter too.
    assert_eq!(MAGPIE.font, "Bitter", "Bitter is bundled + shared (Magpie's masthead face)");
}

/// THE LAW ROUND's `Theme::highlight_treatment` — a NO-ABSENT-VARIANT
/// enum consumed by `render/chrome/overlay.rs`'s picker-row highlight and
/// `render/chrome/menubar.rs`'s open-title band, replacing the former
/// hand-rolled `if selection_style == InverseVideo { .. } else { .. }` at
/// each of those two sites. This pins the STRUCTURAL half of the contract
/// (every world resolves to EXACTLY the treatment its `selection_style`
/// names, with no third "neither" outcome reachable) across all sixteen
/// worlds; the REAL-PIXEL half — does the renderer actually honor it — lives
/// in `render::tests::distinguishability`.
#[test]
fn highlight_treatment_matches_selection_style_on_every_world_no_absent_case() {
    for t in THEMES.iter() {
        let band = crate::theme::Srgb::rgb(0x11, 0x22, 0x33);
        let treatment = t.highlight_treatment(band);
        match (t.render_caps.selection_style, treatment) {
            (
                crate::theme::SelectionStyle::Fill,
                crate::theme::HighlightTreatment::ValueBand(c),
            ) => {
                assert_eq!(c, band, "{}: ValueBand must carry the caller's own band color", t.name);
            }
            (
                crate::theme::SelectionStyle::InverseVideo,
                crate::theme::HighlightTreatment::InverseFill { band: b, ink },
            ) => {
                // A 1-bit world resolves the pair off its OWN ladder, not the
                // caller's `band`: solid `base_content` fill + `base_300` glyphs.
                assert_eq!(b, t.base_content, "{}: InverseFill band must be base_content", t.name);
                assert_eq!(ink, t.base_300, "{}: InverseFill ink must be base_300", t.name);
            }
            (style, treatment) => panic!(
                "{}: selection_style {style:?} produced the WRONG treatment {treatment:?} — \
                 the enum's whole point is that this pairing is supposed to be unreachable",
                t.name
            ),
        }
    }
}

// --- THE OVERLAY-PERSONALITY-AS-DATA ROUND -----------------------------

/// `Srgb::lerp` — the pure blend primitive `placard_ink` (below) leans on.
#[test]
fn lerp_interpolates_and_clamps() {
    let a = Srgb::rgb(0, 0, 0);
    let b = Srgb::rgb(100, 200, 40);
    assert_eq!(a.lerp(b, 0.0), a, "t=0 is exactly self");
    assert_eq!(a.lerp(b, 1.0), b, "t=1 is exactly other");
    assert_eq!(a.lerp(b, 0.5), Srgb::rgb(50, 100, 20), "t=0.5 is the exact midpoint");
    // Out-of-range t clamps rather than extrapolating past either endpoint.
    assert_eq!(a.lerp(b, -1.0), a, "t<0 clamps to self");
    assert_eq!(a.lerp(b, 2.0), b, "t>1 clamps to other");
}

/// COMPOSITION-C2 DATA SANITY for shipped placards (the old "every placard is
/// BL" pin is GONE — the poster corner now DERIVES from the card anchor via
/// [`crate::render::derived_placard_corner`], complementary so the wordmark
/// never sits under the command surface, and the no-clip OUTCOME is asserted
/// end-to-end by `render::tests::overlay_personality`'s no-clip law). Here the
/// DATA stays honest: a placard corner is either `Auto` (derive) or a concrete
/// override (Firetail's user-picked `BL`), and every scale sits in a sane band.
/// A placard world MUST NOT centre its card (`TopCenter`) — a centred card with
/// an `Auto` bottom-corner poster would still read fine, but the shipped
/// placard worlds are the statement/asymmetric temperaments that anchor their
/// card away from centre, so this guards the intended composition.
#[test]
fn every_shipped_placard_world_has_sane_corner_and_scale() {
    let placards: Vec<(&str, model::PlacardCorner, f32, model::CardAnchor)> = THEMES
        .iter()
        .filter_map(|t| match t.render_caps.title_style {
            model::TitleStyle::Placard { corner, scale, .. } => {
                Some((t.name, corner, scale, t.render_caps.card_anchor))
            }
            model::TitleStyle::InlinePrefix => None,
        })
        .collect();
    assert!(
        !placards.is_empty(),
        "at least one world ships a Placard (the round that introduced them) — a \
         zero here means the data table lost every placard, not that the guard passed"
    );
    for (name, corner, scale, anchor) in placards {
        // A legal corner: derive (`Auto`) or a concrete override — never junk.
        assert!(
            matches!(
                corner,
                model::PlacardCorner::Auto
                    | model::PlacardCorner::BL
                    | model::PlacardCorner::BR
                    | model::PlacardCorner::TL
                    | model::PlacardCorner::TR
            ),
            "{name}: placard corner {corner:?} must be a legal value"
        );
        // The shipped placard worlds anchor their card away from centre (the
        // statement temperament), so the complementary poster derivation lands
        // it cleanly opposite the card.
        assert_ne!(
            anchor,
            model::CardAnchor::TopCenter,
            "{name}: a shipped placard world anchors its card off-centre (see this test's doc)"
        );
        // The wordmark scale is a loudness dial, not a fit guarantee
        // (`overlay_shape_placard` shrinks a wider-than-canvas mark), but a
        // shipped value staying in a sane band keeps the data honest.
        assert!(
            (0.5..=5.0).contains(&scale),
            "{name}: shipped placard scale {scale} sits outside the sane 0.5..=5.0 band"
        );
    }
}

/// `theme::placard_ink` NEVER invents a free color, and is MODE-AWARE (the
/// personality-assignment round's dark-ground correction): LIGHT worlds keep
/// the gallery-validated originals byte-for-byte (`Faint` = the world's own
/// faint ink verbatim; `Ghost` = a pure `faint`/`base_300` blend); DARK
/// worlds step the SAME two rungs UP the ladder instead (pure
/// `faint`→`base_content` blends — one global lift constant per rung, never
/// a per-world hand value; the legibility floor/ceiling those lifts must
/// clear is the separate law below). `Stipple`'s pixel ink is exactly
/// `base_content` on every world — the density, not the ink, carries its
/// quietness (see `placard_stipple_density`'s own law).
#[test]
fn placard_ink_derives_from_the_ink_ladder_never_a_free_color() {
    let _g = crate::testlock::serial();
    for t in THEMES.iter() {
        set_active_by_name(t.name).unwrap();
        let faint_rung = derive::placard_ink(model::PlacardInk::Faint);
        let ghost = derive::placard_ink(model::PlacardInk::Ghost);
        if t.dark {
            // A pure blend of two rungs already on the ladder: every channel
            // of the result must sit BETWEEN faint and base_content (a lerp
            // can't leave its endpoints), and the two rungs must be exactly
            // the documented one-formula lifts (re-derived here, so a future
            // per-world special case fails loudly).
            assert_eq!(
                faint_rung,
                t.faint.lerp(t.muted, 0.75),
                "{}: dark-ground PlacardInk::Faint must be the one documented ladder lift",
                t.name
            );
            assert_eq!(
                ghost,
                t.faint.lerp(t.muted, 0.45),
                "{}: dark-ground PlacardInk::Ghost must be the one documented ladder lift",
                t.name
            );
        } else {
            assert_eq!(
                faint_rung, t.faint,
                "{}: light-ground PlacardInk::Faint must be exactly the world's own faint ink \
                 (the gallery-validated original)",
                t.name
            );
            assert_eq!(
                ghost,
                t.faint.lerp(t.base_300, 0.5),
                "{}: light-ground PlacardInk::Ghost must be a pure faint/base_300 blend \
                 (the gallery-validated original)",
                t.name
            );
        }
        assert_eq!(
            derive::placard_ink(model::PlacardInk::Stipple),
            t.base_content,
            "{}: PlacardInk::Stipple pixels draw in exactly the world's own full ink",
            t.name
        );
    }
    set_active(DEFAULT_THEME);
}

/// THE DARK-GROUND PLACARD LEGIBILITY LAW (the user's 2026-07-15 taste note,
/// enforced: "the dark worlds — there's not enough contrast for the placard";
/// Bombora's Ghost was near-invisible). On every DARK world both placard
/// rungs must clearly READ against the world's own ground — a relative-
/// luminance floor, the same domain law (h) of the role tints uses, because
/// the eye resolves luminance — while still RECEDING behind the rows: the
/// louder rung (`Faint`) stays at or under the world's own `muted` ink in
/// luminance (a legible ghost, never a competing headline), and presence
/// ordering holds (`Faint` ≥ `Ghost`, mirroring the light-mode ordering).
/// Light worlds are pinned byte-identical by the derivation law above, so
/// this law binds exactly where the taste note pointed. The AMBER GUARD
/// binds BY IDENTITY, the comment-tier way (role-tint law (e)'s own
/// exemption): a placard ink is a pure blend of existing ink-ladder rungs —
/// it IS the world's ink, which on a warm-laddered world (Potoroo) shares
/// the caret's general warmth without being the accent — so the assertable
/// half is that it is never LITERALLY `primary` (monochrome worlds exempt:
/// their caret IS their ink by design, and none ships a placard anyway —
/// the assignment table pins that).
#[test]
fn placard_inks_read_on_dark_grounds_and_stay_below_muted() {
    fn rel_lum(c: Srgb) -> f32 {
        fn lin(u: u8) -> f32 {
            let s = u as f32 / 255.0;
            if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        }
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }
    let _g = crate::testlock::serial();
    for t in THEMES.iter() {
        set_active_by_name(t.name).unwrap();
        let faint_rung = derive::placard_ink(model::PlacardInk::Faint);
        let ghost = derive::placard_ink(model::PlacardInk::Ghost);
        // AMBER GUARD by identity: never literally the accent.
        if !t.is_monochrome() {
            for (label, ink) in [("Faint", faint_rung), ("Ghost", ghost)] {
                assert_ne!(
                    ink, t.primary,
                    "{}: placard {label} ink must never be literally the accent",
                    t.name
                );
            }
        }
        if !t.dark {
            continue;
        }
        let ground = rel_lum(t.base_100);
        let dy_ghost = rel_lum(ghost) - ground;
        let dy_faint = rel_lum(faint_rung) - ground;
        // FLOOR: the same ΔY ≥ 0.05 luminance floor the role tints carry —
        // the quieter rung must clear it, so the louder one does a fortiori.
        assert!(
            dy_ghost >= 0.05,
            "{}: dark-ground Ghost placard ink {} sits only ΔY {dy_ghost:.3} above the ground \
             (near-invisible — the Bombora gallery bug)",
            t.name,
            ghost.hex()
        );
        // ORDERING: Faint is the more-present rung, on dark exactly as on light.
        assert!(
            dy_faint >= dy_ghost - 1e-4,
            "{}: placard presence ordering inverted (Faint ΔY {dy_faint:.3} < Ghost ΔY {dy_ghost:.3})",
            t.name
        );
        // CEILING: a legible GHOST, not a competing headline — the louder rung
        // stays at or under the world's own muted ink (the non-selected row
        // ink on the card it bleeds behind). Equality is legal (Wagtail's
        // collapsed ladder makes every ink rung the same white — moot anyway,
        // since Wagtail ships no placard).
        let dy_muted = rel_lum(t.muted) - ground;
        assert!(
            dy_faint <= dy_muted + 1e-4,
            "{}: dark-ground Faint placard ink {} (ΔY {dy_faint:.3}) outshines the world's own \
             muted ink (ΔY {dy_muted:.3}) — a competing headline, not a ghost",
            t.name,
            faint_rung.hex()
        );
    }
    set_active(DEFAULT_THEME);
}

/// THE STIPPLE PLACARD LAW: `Stipple`'s two derived halves stay on the
/// world's own ladder and stay LEGIBLE. (a) The pixel ink is exactly
/// `base_content` (asserted per-world by the derivation law above) — so a
/// stipple can only ever paint the ladder's full ink, never amber, never a
/// free color; on a MONOCHROME/1-bit world that ink is its legal pure white,
/// which is why `Stipple` is the one placard ink that would be monochrome-
/// legal by construction (banked — Wagtail ships no placard). (b) The
/// density is the documented perceived-tone formula, clamped to its
/// floor/ceiling band. (c) THE LEGIBILITY FLOOR OVER THE WORLD'S OWN GROUND
/// (the 3b taste-note assertion): the stipple's MEAN tone — ground blended
/// toward the ink at `density` — clears the same ΔY ≥ 0.05 luminance floor
/// the flat placard inks carry, against the flat ground AND, on a lava
/// world, against the brightest pixel the animated margin can ever produce
/// (`blob_hi` — captures render t=0, but the law covers every phase since
/// `mix()` is bounded by its endpoints; the lava figure/ground law proves
/// blob_hi is genuinely reached). Swept over EVERY world (the derivation is
/// total), so a future stipple assignment is born covered.
#[test]
fn stipple_placard_density_clears_the_legibility_floor_over_its_own_ground() {
    fn rel_lum(c: Srgb) -> f32 {
        fn lin(u: u8) -> f32 {
            let s = u as f32 / 255.0;
            if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        }
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }
    let _g = crate::testlock::serial();
    for t in THEMES.iter() {
        set_active_by_name(t.name).unwrap();
        let density = derive::placard_stipple_density();
        assert!(
            (0.12..=0.55).contains(&density),
            "{}: stipple density {density:.3} escaped the floor/ceiling band",
            t.name
        );
        let ink = derive::placard_ink(model::PlacardInk::Stipple);
        let ground = rel_lum(t.base_100);
        let mean = ground + density * (rel_lum(ink) - ground);
        assert!(
            (mean - ground).abs() >= 0.05,
            "{}: stipple mean tone ΔY {:.3} vs the flat ground fails the legibility floor",
            t.name,
            (mean - ground).abs()
        );
        // The lava arm: the ONLY moving ground a stipple placard can sit
        // over. Its brightest reachable pixel must not swallow the mark.
        if let Some((_, _, blob_hi, _, _)) = t.background.lava_params() {
            let worst = rel_lum(blob_hi);
            assert!(
                (mean - worst).abs() >= 0.05,
                "{}: stipple mean tone ΔY {:.3} vs the worst-phase lava pixel {} fails the \
                 legibility floor",
                t.name,
                (mean - worst).abs(),
                blob_hi.hex()
            );
        }
    }
    set_active(DEFAULT_THEME);
}

/// THE PERSONALITY ASSIGNMENT TABLE (2026-07-15, the user's decided picks) —
/// the conscious successor of the machinery round's all-InlinePrefix
/// byte-identity gate. Every world's `render_caps` must be EXACTLY its
/// decided value: the four placard worlds (Galah/Magpie the Ghost reference
/// look, Mangrove the stipple — the Bayer dither is its own language,
/// Firetail the loud-end statement — a big/Bold smooth placard plus the
/// Archivo Black chrome voice, the CHROME-VOICES flip), the three functional-
/// elevation borders (Currawong's OLED rim, the two lava worlds' edge over
/// motion, the six LIGHT worlds' pale-ground rim — composition round item 6),
/// the Wagtail page frame (2px, its ladder white), Wagtail's
/// user-confirmed NO-placard silence — and, just as deliberately, DEFAULT
/// for every world not named (byte-identity for the quiet roster). A NEW
/// world fails the `expected()` match until it decides its personality here
/// — the no-wildcard discipline applied to the roster.
#[test]
fn personality_assignments_are_exactly_the_decided_table() {
    use model::{
        BarCoverage, BarExtent, ChipVariant, Elevation, FacetStyle, ListStyle, PageFrame,
        PlacardCorner, PlacardInk, RenderCaps, TitleStyle,
    };
    // FLIP ROUND (user FINAL PICKS 2026-07-17): the SHIPPING poster list surface
    // every statement world carries — the Bars HUG-ALL HYBRID (`HugLabel`: plate
    // hugs the LABEL, chord bare in the right column) at the gate's mid radius,
    // every row a bar. Mirrors `worlds::POSTER_BARS` (the one owner).
    let poster_bars = ListStyle::Bars {
        radius: 6.0,
        gap: 10.0,
        grow_px: 24.0,
        extent: BarExtent::HugLabel,
        coverage: BarCoverage::All,
    };
    let expected = |name: &str| -> RenderCaps {
        // COMPOSITION-C2: the placard worlds anchor their card TOP-LEFT and let
        // the poster corner DERIVE from that anchor (`Auto` → bottom-RIGHT),
        // opening the opposite corner. Firetail alone keeps an explicit BL.
        // ITEM 45 (2026-07-23): Cassowary + Mangrove are the fable RIGHT picks —
        // TopRight card, Auto corner deriving bottom-LEFT (the mirror composition).
        let auto = |ink: PlacardInk| TitleStyle::Placard {
            corner: PlacardCorner::Auto,
            scale: 3.0,
            ink,
        };
        match name {
            // Galah / Magpie: the light-world placard PLUS the composition
            // round's light-world border (item 6); C2 TopLeft anchor + Auto corner.
            "Galah" => RenderCaps {
                title_style: auto(PlacardInk::Ghost),
                card_anchor: model::CardAnchor::TopLeft,
                elevation: Elevation::Bordered,
                // FLIP ROUND (2026-07-17): poster world → the Bars hug-all hybrid;
                // Galah wears HAIRLINE chips (user's confirmed chip map).
                list_style: poster_bars,
                facet_style: FacetStyle::Chips(ChipVariant::Hairline),
                ..RenderCaps::DEFAULT
            },
            "Magpie" => RenderCaps {
                title_style: auto(PlacardInk::Ghost),
                card_anchor: model::CardAnchor::TopLeft,
                elevation: Elevation::Bordered,
                // Magpie wears UNDERLINE chips (user's confirmed chip map).
                list_style: poster_bars,
                facet_style: FacetStyle::Chips(ChipVariant::Underline),
                ..RenderCaps::DEFAULT
            },
            "Mangrove" => RenderCaps {
                title_style: auto(PlacardInk::Stipple),
                // ITEM 45 fable pick (2026-07-23): the tidal margin flipped to a
                // RIGHT rail (Auto corner then derives bottom-LEFT).
                card_anchor: model::CardAnchor::TopRight,
                elevation: Elevation::Bordered,
                // Mangrove wears BRACKET chips (user's confirmed chip map).
                list_style: poster_bars,
                facet_style: FacetStyle::Chips(ChipVariant::Bracket),
                ..RenderCaps::DEFAULT
            },
            // CHROME-VOICES FLIP (2026-07-16): the loud-end world's own loud
            // overlay — BL placard dialed to the combo-shot scale + Bold ink,
            // and the Archivo Black chrome voice on the placard/title/strip.
            // C2: KEEPS its user-picked explicit BL corner (overrides the Auto
            // derivation) and anchors its card TopLeft.
            "Firetail" => RenderCaps {
                title_style: TitleStyle::Placard {
                    corner: PlacardCorner::BL,
                    scale: 4.5,
                    ink: PlacardInk::Bold,
                },
                card_anchor: model::CardAnchor::TopLeft,
                chrome_face: model::ChromeFace::Named("Archivo Black"),
                elevation: Elevation::Bordered,
                // FLIP ROUND (2026-07-17): the maximalist showcase world → the Bars
                // hug-all hybrid; Firetail wears FILLED chips (the loudest — user's
                // confirmed chip map).
                list_style: poster_bars,
                facet_style: FacetStyle::Chips(ChipVariant::FilledActive),
                ..RenderCaps::DEFAULT
            },
            // C2: the iconic dark-technical statement world anchors TopLeft.
            // TWINKLING STARS (2026-07-18, the user's morning verdict): Currawong
            // stays, differentiated by the ambient star field — the maximally-
            // quiet, unmistakably-alive pole ("aliveness ≠ loudness"). The
            // params are the authored taste data (BUILD + GALLERY + HOLD).
            "Currawong" => RenderCaps {
                elevation: Elevation::Bordered,
                card_anchor: model::CardAnchor::TopLeft,
                ambient: model::AmbientStyle::Stars {
                    tint: Srgb::rgb(0x9D, 0xB0, 0xCF),
                    cell_px: 34.0,
                    // LIFECYCLE round (2026-07-23): denser candidate field
                    // (~half dark-dwelling at any moment) and the visibility band
                    // re-scoped to the per-star shine range (a real visible floor,
                    // a calm ceiling above the muted whisper cap).
                    density: 0.30,
                    size_px: 2.6,
                    peak: 0.5,
                    floor: 0.18,
                },
                ..RenderCaps::DEFAULT
            },
            // Wagtail: the 1-bit escape hatch (every field away from default)
            // + the page frame's first assignment + NO placard (the silent
            // pole announces nothing — user-confirmed).
            "Wagtail" => RenderCaps {
                selection_style: model::SelectionStyle::InverseVideo,
                caret_block_style: model::CaretBlockStyle::InverseVideo,
                backdrop: model::Backdrop::Flat,
                elevation: Elevation::Bordered,
                decorative_wash: model::DecorativeWash::Off,
                image_reveal: model::ImageReveal::Opaque,
                highlight_texture: model::HighlightTexture::Stipple {
                    color: Srgb::rgb(0xFF, 0xFF, 0xFF),
                    density: crate::render::dither::WAGTAIL_HIGHLIGHT_DITHER_DENSITY,
                },
                title_style: TitleStyle::InlinePrefix,
                page_frame: PageFrame::Line { weight_px: 2.0 },
                card_anchor: model::CardAnchor::TopLeft,
                // FIRETAIL-MAXIMALIST-SHOWCASE round: both new dials landed
                // INERT on every world — the silent pole included.
                chrome_face: model::ChromeFace::Body,
                motion: model::MotionJuice::CALM,
                // PER-ITEM LIST SURFACES round: both new dials landed INERT on
                // every world — the silent pole included.
                list_style: model::ListStyle::Pane,
                facet_style: model::FacetStyle::Text,
                // SPLIT-PANE COMPOSITION round: the silent pole takes the DEFAULT
                // split like every Pane world (only Cassowary opts to `Unified`).
                pane_split: model::PaneSplit::Split,
                // TWINKLING-STARS round: no ambient life on the silent pole
                // (and a fractional-alpha breath is 1-bit-illegal besides).
                ambient: model::AmbientStyle::None,
                // SPELL-SQUIGGLE round: the silent pole keeps the shared
                // default gap.
                spell_underline_gap: model::SPELL_UNDERLINE_GAP_DEFAULT,
                // FROST-AS-CAPABILITY round: dormant default (no lava ground).
                frost: model::Frost::DEFAULT,
            },
            // DAWN ROUND (2026-07-18): Bilby is the LIGHT POLE — the roster
            // decision ("the dark-line-on-light page frame is reserved for a
            // future light-silent pole world") lands here: 1px of its own
            // night-violet ink around the writing column, Wagtail's 2px white
            // frame mirrored at the light end of the spectrum. Keeps the
            // light-world card border.
            // Dawn round: the proposed 1px light-pole page frame was REJECTED by
            // the user live ("the frame is so weird") — Bilby ships frameless.
            "Bilby" => RenderCaps {
                elevation: Elevation::Bordered,
                // SPELL-SQUIGGLE round: the tighter per-world baseline dial
                // (see `worlds::BILBY`'s own doc).
                spell_underline_gap: model::SPELL_UNDERLINE_GAP_DEFAULT - 2.0,
                ..RenderCaps::DEFAULT
            },
            // LIGHT-WORLD BORDER (composition round item 6): the remaining
            // pale-ground worlds gain the summoned-card border, DATA-only.
            // Brolga (the SEVENTEENTH world, the cool light pole) joins them —
            // a crisp rim off its pale sky-blue ground; deliberately NO page
            // frame (the DAWN round's 1px light-pole frame was user-rejected).
            "Gumtree" | "Saltpan" | "Quokka" | "Brolga" => {
                RenderCaps { elevation: Elevation::Bordered, ..RenderCaps::DEFAULT }
            }
            "Tawny" | "Mopoke" | "Potoroo" | "Bombora" | "Bowerbird" | "Mulga" => {
                RenderCaps::DEFAULT
            }
            // CASSOWARY (the NERV-terminal statement world): the loud NERV console
            // overlay — a bold Archivo-Black wordmark placard (Auto corner derives
            // bottom-LEFT off the ITEM-45 RIGHT card), BORDERED elevation, the poster
            // Bars list, and BRACKET facet chips (terminal corner-ticks). The writing
            // page stays calm.
            "Cassowary" => RenderCaps {
                // The authentic CRT phosphor cursor — an ink caret (primary ==
                // base_content) needs the Filled block so a lit green cell knocks
                // the glyph out in the ground rather than erasing it green-on-green.
                caret_block_style: model::CaretBlockStyle::Filled,
                title_style: TitleStyle::Placard {
                    corner: PlacardCorner::Auto,
                    scale: 3.0,
                    ink: PlacardInk::Bold,
                },
                // ITEM 45 fable pick (2026-07-23): the terminal readout flipped to
                // a RIGHT rail (Auto corner then derives bottom-LEFT).
                card_anchor: model::CardAnchor::TopRight,
                chrome_face: model::ChromeFace::Named("Archivo Black"),
                elevation: Elevation::Bordered,
                list_style: poster_bars,
                facet_style: FacetStyle::Chips(ChipVariant::Bracket),
                // SPLIT-PANE COMPOSITION round: the NERV console is the ONE Pane
                // exception — a UNIFIED room (dormant under its poster Bars list).
                pane_split: model::PaneSplit::Unified,
                ..RenderCaps::DEFAULT
            },
            other => panic!(
                "{other}: a NEW world must decide its personality here (placard? border? \
                 frame? or deliberately DEFAULT) — the assignment table is conscious data, \
                 never an accident"
            ),
        }
    };
    for t in THEMES.iter() {
        assert_eq!(
            t.render_caps,
            expected(t.name),
            "{}: render_caps drifted from the decided personality table",
            t.name
        );
    }
    // Corner discipline is now the COMPOSITION-C2 no-clip OUTCOME law
    // (`render::tests::overlay_personality::every_shipped_placard_world_wordmark_stays_on_canvas`)
    // + the data-sanity guard (`every_shipped_placard_world_has_sane_corner_and_scale`),
    // not a BL pin: the shrink-to-fit made every corner clip-safe, so the poster
    // corner DERIVES from the card anchor (complementary) with per-world overrides.
}

/// THE PAGE-FRAME THEME LAW: the frame can never invent a color — its ink is
/// derived in ONE owner (`page_frame_ink` = the world's own `base_content`,
/// the full-ink ladder rung) for EVERY world, assigned or not; an assigned
/// frame's weight is a real positive width. The AMBER GUARD binds here BY
/// IDENTITY, the same way the comment tiers' does (role-tint law (e)): the
/// frame ink IS an existing ink rung — definitionally the ink, never the
/// accent, even on a warm-inked world whose ink shares the caret's general
/// warmth (Mopoke) — so the assertable half is that it is never LITERALLY
/// `primary` (the WYSIWYG value-step law's own shape). The frame's PIXEL
/// half — actually drawn, in bounds, pure ink, absent on every None world —
/// is `render::tests::page_frame`.
#[test]
fn page_frame_ink_is_the_ladder_and_assigned_weights_are_real() {
    let _g = crate::testlock::serial();
    for t in THEMES.iter() {
        set_active_by_name(t.name).unwrap();
        assert_eq!(
            derive::page_frame_ink(),
            t.base_content,
            "{}: page_frame_ink must be exactly the world's own base_content",
            t.name
        );
        // An INK-CARET world's caret IS its ink (`primary == base_content`;
        // presence carried by the inverting/filled block, not a hue — Wagtail's
        // pure white, Cassowary's phosphor green), so "never literally primary" is
        // structurally inapplicable there (the frame ink == base_content == primary
        // BY DESIGN); every other world must keep frame-ink and accent distinct.
        if !t.ink_caret() {
            assert_ne!(
                derive::page_frame_ink(),
                t.primary,
                "{}: the page-frame ink must never be literally the accent",
                t.name
            );
        }
        if let model::PageFrame::Line { weight_px } = t.render_caps.page_frame {
            assert!(
                weight_px > 0.0 && weight_px.is_finite(),
                "{}: an assigned page frame must carry a real positive weight (got {weight_px})",
                t.name
            );
        }
    }
    set_active(DEFAULT_THEME);
}

/// THE SPELL-SQUIGGLE PER-WORLD BASELINE DIAL: every world carries
/// [`model::SPELL_UNDERLINE_GAP_DEFAULT`] (byte-identical to the pre-dial
/// hardcoded gap) EXCEPT Bilby, whose report ("the squiggle floats too far
/// below the baseline") earned a tighter, strictly SMALLER override — DATA on
/// `RenderCaps`, never a per-world code path (`render/tests/theme_caps_law.rs`
/// structurally bans a `"Bilby"` string or `.is_one_bit()` read under
/// `src/render/`). No-wildcard over `THEMES`, so a future 19th world defaults
/// through `RenderCaps::DEFAULT` until it consciously opts in too.
#[test]
fn spell_underline_gap_is_the_shared_default_everywhere_except_bilbys_tighter_dial() {
    for t in THEMES.iter() {
        if t.name == "Bilby" {
            assert!(
                t.render_caps.spell_underline_gap < model::SPELL_UNDERLINE_GAP_DEFAULT,
                "Bilby must carry a STRICTLY tighter (smaller) gap than the shared default \
                 ({} vs default {})",
                t.render_caps.spell_underline_gap,
                model::SPELL_UNDERLINE_GAP_DEFAULT
            );
        } else {
            assert_eq!(
                t.render_caps.spell_underline_gap,
                model::SPELL_UNDERLINE_GAP_DEFAULT,
                "{}: every world but Bilby stays on the shared default gap",
                t.name
            );
        }
    }
}

/// REPAIR ROUND 2's flagged gap, closed structurally — and extended by the
/// personality-assignment round to cover EVERY grey placard ink: a
/// `TitleStyle::Placard` whose ink is `Faint` OR `Ghost` on a TRUE 1-BIT
/// world (`Theme::is_one_bit`) would render an ordinary intermediate-grey
/// wordmark (and antialiased glyph fringes besides), which that world's own
/// law (`render::tests::syntax_roles::every_one_bit_world_renders_only_pure_
/// black_or_white`) forbids outright. `Stipple` is deliberately EXEMPT: its
/// pixels are hard-thresholded pure `base_content` at full alpha or nothing
/// (the same 1-bit-legality argument as the highlight stipple) — though no
/// one-bit world ships ANY placard today (Wagtail is the user-confirmed
/// silent pole; the assignment-table law pins that). Lives in `theme::`,
/// deliberately never `render::`, where a bare `.is_one_bit()` call is
/// banned outright (`render::tests::theme_caps_law`) — the "pin an identity,
/// not a render mechanism" carve-out that grep-law's own doc describes.
#[test]
fn a_placard_grey_ink_would_violate_a_one_bit_worlds_own_law() {
    for t in THEMES.iter() {
        if let model::TitleStyle::Placard {
            // The FIRETAIL-MAXIMALIST-SHOWCASE dial-up rungs (`Muted`/`Bold`)
            // are ordinary greys on every world today, so they join the
            // guarded set alongside `Faint`/`Ghost`; `Stipple` stays the one
            // 1-bit-legal exemption (hard pure-ink pixels).
            ink:
                ink @ (model::PlacardInk::Faint
                | model::PlacardInk::Ghost
                | model::PlacardInk::Muted
                | model::PlacardInk::Bold),
            ..
        } = t.render_caps.title_style
        {
            assert!(
                !t.is_one_bit(),
                "{}: TitleStyle::Placard{{ink: {ink:?}}} on a true 1-bit world renders an \
                 illegal intermediate grey — of the placard inks only Stipple (hard pure-ink \
                 pixels) is 1-bit-legal by construction",
                t.name
            );
        }
    }
}

/// THE FIRETAIL-MAXIMALIST-SHOWCASE round's DIAL-UP ink law: the two new
/// smooth rungs (`Muted`/`Bold`) are pure ladder derivations through the ONE
/// owner (`theme::placard_ink`) — `Muted` IS the world's own `muted` rung
/// verbatim, `Bold` is a pure `muted`→`base_content` blend that stays
/// strictly BELOW full ink (the rows always outshine the wordmark, by
/// construction), presence-ordered above `Faint` (louder is genuinely
/// louder, on every world, both grounds), and — the never-amber guard, in
/// its identity form — never literally the accent on any chromatic world
/// (they're ladder greys; the assertable half is non-identity, the same
/// shape as `page_frame_ink`'s own guard). Every world is swept even though
/// no world SHIPS a dial-up rung yet: the probe (`AWL_OVERLAY_STYLE_FORCE`)
/// makes them reachable on all sixteen today, so the law must already hold
/// everywhere, not just on a future assignee.
#[test]
fn dialup_placard_inks_stay_on_the_ladder_below_full_ink() {
    let _g = crate::testlock::serial();
    // Gamma-correct Rec.709 relative luminance (the same local recipe the
    // other placard-ink laws carry).
    fn rel_lum(c: Srgb) -> f32 {
        fn lin(u: u8) -> f32 {
            let s = u as f32 / 255.0;
            if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        }
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }
    for t in THEMES.iter() {
        set_active_by_name(t.name).unwrap();
        let muted_rung = derive::placard_ink(model::PlacardInk::Muted);
        let bold = derive::placard_ink(model::PlacardInk::Bold);
        let faint_rung = derive::placard_ink(model::PlacardInk::Faint);
        assert_eq!(
            muted_rung, t.muted,
            "{}: PlacardInk::Muted must be exactly the world's own muted rung",
            t.name
        );
        assert_eq!(
            bold,
            t.muted.lerp(t.base_content, 0.5),
            "{}: PlacardInk::Bold must be the one documented muted→base_content blend",
            t.name
        );
        // Presence ordering, in ink-distance-from-ground terms: Faint ≤ Muted ≤
        // Bold < full ink — the dial goes UP, and its ceiling is structural.
        let ground = rel_lum(t.base_100);
        let dy = |c: Srgb| (rel_lum(c) - ground).abs();
        assert!(
            dy(faint_rung) <= dy(muted_rung) + 1e-6,
            "{}: Muted must read at least as present as Faint (ΔY {:.4} < {:.4})",
            t.name,
            dy(muted_rung),
            dy(faint_rung)
        );
        assert!(
            dy(muted_rung) <= dy(bold) + 1e-6,
            "{}: Bold must read at least as present as Muted (ΔY {:.4} < {:.4})",
            t.name,
            dy(bold),
            dy(muted_rung)
        );
        // The strict below-full-ink ceiling exempts a TRUE 1-BIT world (the
        // same declared exemption arm the dark-ground placard law carries):
        // its ladder COLLAPSES (`muted == base_content`, pure white), so the
        // blend is degenerate — and a grey placard rung is already
        // structurally illegal there anyway (`a_placard_grey_ink_would_
        // violate_a_one_bit_worlds_own_law` guards Muted/Bold too).
        if !t.is_one_bit() {
            assert!(
                dy(bold) < dy(t.base_content),
                "{}: Bold (ΔY {:.4}) must stay BELOW full ink (ΔY {:.4}) — the rows always win",
                t.name,
                dy(bold),
                dy(t.base_content)
            );
        }
        // Never-amber, identity form (ladder greys can't carry the accent's
        // hue by construction; the assertable half is non-identity).
        if !t.is_monochrome() {
            for (label, c) in [("Muted", muted_rung), ("Bold", bold)] {
                assert_ne!(
                    c, t.primary,
                    "{}: dial-up placard {label} ink must never be literally the accent",
                    t.name
                );
            }
        }
    }
    set_active(DEFAULT_THEME);
}

/// WRITING-STREAKS HEATMAP tint law (`heatmap_colors`, one owner in
/// `derive.rs`): for EVERY world the calendar squares must be figure/ground by
/// value — the filled intensity rungs distinguishable FROM the card's own
/// `base_300` ground AND from each other, climbing monotonically toward ink, and
/// NEVER amber (the caret's alone). One-bit worlds (Wagtail) carry a DECLARED
/// EXEMPTION: no intermediate grey is permitted there, so the heatmap degrades to
/// BINARY (empty = ground, any writing = full ink), which the arm below asserts is
/// pure black/white instead of a 5-step ramp.
#[test]
fn streaks_heatmap_levels_are_distinguishable_every_world() {
    let _g = crate::testlock::serial();
    // Gamma-correct Rec.709 relative luminance (the `rel_lum` recipe used across
    // the theme laws) — perceived brightness, so "distinguishable" is perceptual.
    fn rel_lum(c: Srgb) -> f32 {
        fn lin(u: u8) -> f32 {
            let s = u as f32 / 255.0;
            if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        }
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }
    // A filled rung must clear the ground by at least this perceived-luminance
    // step, and each rung must clear the previous by at least this much — small
    // but non-zero, so even the tightest world's ink span reads as 4 steps.
    const MIN_STEP: f32 = 0.012;
    for t in THEMES.iter() {
        set_active_by_name(t.name).unwrap();
        let colors = heatmap_colors();
        assert_eq!(colors.len(), crate::streaks::LEVELS);
        let ground = base_300();

        if t.is_one_bit() {
            // 1-BIT DEGRADATION: empty = ground token, every filled rung = full ink,
            // and both are pure (no forbidden intermediate grey). A written cell must
            // still read against the ground.
            assert_eq!(colors[0], base_200(), "{}: 1-bit empty rung is the ground token", t.name);
            for c in &colors[1..] {
                assert_eq!(*c, base_content(), "{}: 1-bit filled rung is full ink", t.name);
            }
            assert!(
                (rel_lum(colors[4]) - rel_lum(ground)).abs() > 0.5,
                "{}: 1-bit written cell reads against the ground",
                t.name
            );
            continue;
        }

        // The direction ink climbs away from the ground (light world: down; dark:
        // up). Every filled rung must move in that one direction, monotonically.
        let ink_dir = (rel_lum(base_content()) - rel_lum(ground)).signum();
        let mut prev = rel_lum(colors[0]); // the empty rung
        for (i, c) in colors.iter().enumerate().skip(1) {
            let y = rel_lum(*c);
            // Distinguishable from the ground.
            assert!(
                (y - rel_lum(ground)).abs() >= MIN_STEP,
                "{}: filled level {i} (Y {:.4}) not distinguishable from base_300 ground (Y {:.4})",
                t.name, y, rel_lum(ground)
            );
            // Distinguishable from — and climbing past — the previous rung.
            assert!(
                (y - prev) * ink_dir >= MIN_STEP,
                "{}: level {i} (Y {:.4}) is not a clear step up from level {} (Y {:.4})",
                t.name, y, i - 1, prev
            );
            prev = y;
            // NEVER THE ACCENT: every square rides the world's own ink ladder (a
            // blend of `base_200`↔`base_content`), so it can never manufacture
            // chroma BEYOND that ladder — the caret's saturated accent is never a
            // decorative fill here. A warm world's `base_content` legitimately
            // shares the accent's HUE (it's the reading ink every glyph uses), so
            // the guard bounds SATURATION to the ladder's own, not the hue: a cell
            // must be no more saturated than the ink endpoints, and never literally
            // `primary`.
            let (_, s, _) = c.to_hsl();
            let (_, s_ink, _) = base_content().to_hsl();
            let (_, s_empty, _) = base_200().to_hsl();
            assert!(
                s <= s_ink.max(s_empty) + 0.02,
                "{}: heatmap level {i} (sat {:.2}) manufactures chroma beyond the ink ladder (max {:.2})",
                t.name, s, s_ink.max(s_empty)
            );
            // On an INK-CARET world the brightest rung IS the ink IS the accent
            // (`primary == base_content`; Wagtail's white, Cassowary's phosphor),
            // so "never literally primary" is structurally inapplicable — the
            // heatmap climbing to the full ink is the intended top level.
            if !t.ink_caret() {
                assert_ne!(*c, primary(), "{}: heatmap level {i} must never be literally the accent", t.name);
            }
        }
    }
    set_active(DEFAULT_THEME);
}
