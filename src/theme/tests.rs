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
    for world in ["Kingfisher", "Saltpan", "Firetail", "Tawny"] {
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
    set_active_by_name("Kingfisher").unwrap();
    assert_ne!(
        overlay_selected_band(),
        surface_selected(),
        "Kingfisher: the strengthened band differs from the shared band"
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
fn worlds_ten_dark_six_light() {
    assert_eq!(THEMES.len(), 16);
    let dark = THEMES.iter().filter(|t| t.dark).count();
    let light = THEMES.iter().filter(|t| !t.dark).count();
    // 10 dark (Tawny/Mopoke/Currawong/Potoroo/Undertow/Kingfisher/Outback/
    // Mangrove/Wagtail/Firetail) / 6 light (Gumtree/Bilby/Saltpan/Quokka/Galah/
    // Magpie). Firetail (the sixteenth) is the warm lava statement world.
    assert_eq!(dark, 10);
    assert_eq!(light, 6);
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
    let blur = crate::lava::FROST_BLUR_PX;
    let dim = crate::lava::FROST_DIM;
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

/// THE GUTTER LOCAL CORNER CARVE LAW (the "lava both sides" round — re-scoped
/// from the old whole-margin gutter carve). The bottom-left page-mode GUTTER
/// (`TextPipeline::prepare_gutter` — the filename/project stack) used to gate the
/// WHOLE-margin `lava_rail_carved` carve, which flattened both margins on nearly
/// every page-mode buffer (the gutter shows almost always), so the lamp was
/// right-only. It now drives only a BOUNDED corner carve around its own block
/// (`TextPipeline::lava_gutter_carve_rect` → the shader's `gutter_rect`), so its
/// `muted`/`faint` stack sits on flat ground while the REST of both margins keep
/// the lamp — an ordinary doc goes both-sides.
///
/// Three halves:
///
/// (1) STRUCTURAL, PHASE-INDEPENDENT — FLAT INSIDE THE CORNER: with the gutter
///     rect carved (`lava_mask_2d` with `Some(rect)`), the gutter's own corner
///     band contains NO lava pixel at ANY animation phase — the composited pixel
///     is bit-exactly the world's flat ground. Proven over COMPOSITED PIXELS via
///     the pure-Rust shader mirror, with a non-vacuous WITNESS.
///
/// (2) BOTH MARGINS RECLAIMED — LOCAL, not whole-margin: OUTSIDE the corner rect
///     (the left margin ABOVE the band, and the whole right margin) the 2-D mask
///     is byte-for-byte the un-carved column mask — the lamp is untouched, so an
///     ordinary doc keeps both sides. Witnessed: some sampled reclaimed pixel
///     genuinely carries a blob. The corner BOUNDS (a bottom-left box) are pinned
///     at the render seam by
///     `render::tests::outline::lava_gutter_carve_follows_gutter_visibility`.
///
/// (3) LEGIBILITY FLOOR: the gutter's two inks clear the repo's perceptible-
///     difference floors against that LOCAL corner ground (== base_100) — the
///     `faint` project line at the ink-ladder law (c) redmean >= 100, the
///     `muted` filename line at >= 150 — so the gutter can never drown again.
///
/// The `Background` match is NO-WILDCARD: a future ground variant must decide
/// its rail story here or fail to compile.
#[test]
fn gutter_corner_carve_is_local_flat_ground_and_keeps_both_margins_on_every_lava_world() {
    fn redmean(a: Srgb, b: Srgb) -> f32 {
        let rbar = (a.r as f32 + b.r as f32) * 0.5;
        let dr = a.r as f32 - b.r as f32;
        let dg = a.g as f32 - b.g as f32;
        let db = a.b as f32 - b.b as f32;
        ((2.0 + rbar / 256.0) * dr * dr + 4.0 * dg * dg + (2.0 + (255.0 - rbar) / 256.0) * db * db)
            .sqrt()
    }
    // Representative page geometry (the 1600x1000 gallery canvas at the default
    // 70-char prose measure). The gutter's local corner rect [left, top, right,
    // bottom]: left 0, right a small gap shy of the column, a BOTTOM band (the
    // two stacked LABEL rows ~8px up from the canvas bottom — `prepare_gutter` /
    // `gutter_carve_rect`).
    let vp = (1600.0f32, 1000.0f32);
    let (col_left, col_right) = (296.0f32, 1304.0f32);
    let gap = crate::lava::MARGIN_GAP_PX;
    let gutter_rect = [0.0f32, 900.0, 260.0, 1000.0];
    for t in THEMES.iter() {
        // NO-WILDCARD: the composite below uses blob_hi — the BRIGHTEST tone the
        // shader can reach — so proving the worst case covers blob_lo too.
        let (ground, blob_hi) = match t.background {
            // The five static grounds carry no lava to carve.
            Background::Gradient { .. }
            | Background::Dots { .. }
            | Background::Starfield { .. }
            | Background::Pinstripe { .. }
            | Background::Stripes { .. } => continue,
            Background::Lava { ground, blob_hi, .. } => (ground, blob_hi),
        };
        // The corner ground IS the page's own ground — the ink-ladder laws govern it.
        assert_eq!(ground, t.base_100, "{}: corner ground must be base_100", t.name);

        // (1) Phase sweep x corner-band grid: the 2-D carved mask is exactly zero
        //     INSIDE the rect, so the straight-alpha composite over the flat
        //     ground is bit-exactly the ground — even against the brightest tone.
        // (2) The RECLAIMED margin (left margin ABOVE the band + the right
        //     margin) stays byte-identical to the un-carved column mask.
        let mut witnessed_carve = false;
        let mut witnessed_reclaim = false;
        for step in 0..64 {
            let phase = step as f32 * crate::lava::LAVA_LOOP_CYCLES / 64.0;
            // CARVE samples: x strictly INSIDE the corner rect (past its 28px
            // right-face feather: rect right 260 → interior ends ~232).
            for &x in &[10.0f32, 60.0, 120.0, 180.0, 225.0] {
                // Corner-band y samples (inside the rect), well past its feather.
                for y in [930.0, 955.0, 985.0] {
                    let a = crate::lava::lava_mask_2d(
                        x,
                        y,
                        col_left,
                        col_right,
                        gap,
                        false,
                        Some(gutter_rect),
                    );
                    assert_eq!(
                        a, 0.0,
                        "{}: lava coverage in the gutter corner at x={x} y={y} phase={phase}",
                        t.name
                    );
                    let over = |gc: u8, bc: u8| -> u8 {
                        (bc as f32 * a + gc as f32 * (1.0 - a)).round() as u8
                    };
                    let px = Srgb {
                        r: over(ground.r, blob_hi.r),
                        g: over(ground.g, blob_hi.g),
                        b: over(ground.b, blob_hi.b),
                        a: 0xFF,
                    };
                    assert_eq!(
                        (px.r, px.g, px.b),
                        (ground.r, ground.g, ground.b),
                        "{}: gutter-corner pixel is not the flat ground at x={x} y={y} phase={phase}",
                        t.name
                    );
                    // WITNESS the carve: the un-carved mask WOULD paint here.
                    if crate::lava::column_mask(x, col_left, col_right, gap) >= 1.0
                        && crate::lava::metaball_field(
                            (x, y),
                            vp,
                            &crate::lava::BACKDROP_BLOBS,
                            phase,
                        ) >= 0.5
                    {
                        witnessed_carve = true;
                    }
                }
            }
            // RECLAIMED: the left margin ABOVE the corner band (both sides back)
            // AND the whole right margin are byte-identical to the plain column
            // mask — the carve is LOCAL, the lamp elsewhere is untouched.
            let reclaim: [(f32, f32); 6] = [
                (60.0, 120.0),   // left margin, above the band
                (180.0, 300.0),  // left margin, above the band
                (120.0, 520.0),  // left margin, above the band
                (1320.0, 930.0), // right margin, at the band's y (still lit)
                (1400.0, 500.0), // right margin
                (1560.0, 970.0), // right margin, deep
            ];
            for (x, y) in reclaim {
                let carved =
                    crate::lava::lava_mask_2d(x, y, col_left, col_right, gap, false, Some(gutter_rect));
                let plain = crate::lava::column_mask(x, col_left, col_right, gap);
                assert_eq!(
                    carved, plain,
                    "{}: a reclaimed margin pixel lost its lamp at x={x} y={y} (carve not local)",
                    t.name
                );
                if plain >= 1.0
                    && crate::lava::metaball_field((x, y), vp, &crate::lava::BACKDROP_BLOBS, phase)
                        >= 0.5
                {
                    witnessed_reclaim = true;
                }
            }
        }
        assert!(
            witnessed_carve,
            "{}: no sampled corner pixel would have carried lava without the carve (vacuous)",
            t.name
        );
        assert!(
            witnessed_reclaim,
            "{}: no sampled reclaimed pixel carries a blob — the both-sides claim is vacuous",
            t.name
        );
        // (3) The gutter's inks clear the corner's LOCAL ground (== base_100).
        let project = redmean(t.faint, ground);
        assert!(
            project >= 100.0,
            "{}: the gutter's faint project line only {project:.1} redmean from \
             the corner ground (under the ink-ladder perceptibility floor)",
            t.name
        );
        let name = redmean(t.muted, ground);
        assert!(
            name >= 150.0,
            "{}: the gutter's muted filename only {name:.1} redmean from the corner ground",
            t.name
        );
    }
}

/// FIRETAIL PALETTE CHARACTER law: the sixteenth world is an ORIGINAL deep
/// oxblood-charcoal + wine-lava + ember-gold system, not Potoroo's rust palette
/// copied under a moving ground. Hue arithmetic pins the authored direction:
/// Firetail's main ground is much nearer red than Undertow's violet, at least
/// 35° away from Potoroo's orange-rust ground, and both its lava and caret stay
/// in their named wine/gold bands.
#[test]
fn firetail_is_oxblood_wine_and_ember_not_potoroo_rust_or_undertow_violet() {
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
    let undertow_violet = UNDERTOW.base_300.to_hsl().0;
    assert!(
        red_gap(fire_ground) + 60.0 <= red_gap(undertow_violet),
        "Firetail ground {fire_ground:.1}° must read far redder/warmer than Undertow {undertow_violet:.1}°"
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
    const MONO_DISPLAY: [&str; 6] =
        ["Tawny", "Currawong", "Potoroo", "Mangrove", "Wagtail", "Firetail"];
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
    assert_eq!(KINGFISHER.mono, "JetBrains Mono"); // cool technical navy → crisp mono
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
    let shippori = ["Gumtree", "Bilby", "Undertow"];
    let zenmaru = ["Galah", "Kingfisher"];
    let klee = ["Mopoke", "Quokka"];
    let mincho = ["Saltpan", "Outback", "Magpie"]; // neutral serif (Noto Serif JP)
    let gothic = ["Tawny", "Potoroo", "Mangrove", "Currawong", "Wagtail", "Firetail"]; // neutral sans/mono (Noto Sans JP)
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
    assert_eq!(by("Undertow"), 1.8, "Undertow (Garamond fleurons) is fleuron 1.8");
    assert_eq!(by("Currawong"), 1.5, "Currawong (geometric marks) is geometric 1.5");
    set_active(DEFAULT_THEME);
}

/// NEVER-DRIFT law (per-world LIST BULLETS): every world ships a two-glyph
/// [`Theme::bullets`] pair whose two levels are DISTINCT, and a
/// [`Theme::bullet_scale`] that is exactly one of the two named tier constants
/// (no stray literal). The font-DB half — that each glyph actually resolves in
/// the world's [`Theme::ornament_face`] — is `render::tests::markdown::
/// bullet_glyphs_resolve_in_each_worlds_assigned_face`. Also pins the geometric
/// worlds to the plain byte-identical [`BULLETS_PLAIN`]/[`BULLET_SCALE_PLAIN`]
/// (restraint) and the manicule showpiece (Undertow's level-1 ☞).
#[test]
fn every_world_has_a_bullet_pair() {
    assert_eq!(BULLETS_PLAIN, ('•', '◦'), "the plain bullet pair is • / ◦");
    assert_eq!(BULLET_SCALE_PLAIN, 1.0, "plain bullets keep body size");
    assert!(
        BULLET_SCALE_ORNAMENT > 0.0 && BULLET_SCALE_ORNAMENT < BULLET_SCALE_PLAIN,
        "ornament bullets shape smaller than the plain body-size bullets"
    );
    for t in THEMES.iter() {
        assert_ne!(
            t.bullets.0, t.bullets.1,
            "{}: the two bullet levels must be distinct glyphs, got {:?}",
            t.name, t.bullets
        );
        assert!(
            matches!(t.bullet_scale, BULLET_SCALE_PLAIN | BULLET_SCALE_ORNAMENT),
            "{}: off-tier bullet_scale {}",
            t.name,
            t.bullet_scale
        );
        // The geometric/technical worlds keep the plain pair AND body size, in
        // lockstep — a characterful pair at body size (or plain at half) would be
        // a taste drift; a geometric world is byte-identical to before this round.
        let geometric = t.ornament_face == ORNAMENT_MARKS;
        assert_eq!(
            t.bullets == BULLETS_PLAIN,
            t.bullet_scale == BULLET_SCALE_PLAIN,
            "{}: plain-pair and plain-scale must agree (geometric restraint)",
            t.name
        );
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
    // The PAIR CYCLES every two levels (even → level 1, odd → level 2).
    assert_eq!(TAWNY.bullet_for_depth(0), '•');
    assert_eq!(TAWNY.bullet_for_depth(1), '◦');
    assert_eq!(TAWNY.bullet_for_depth(2), '•');
    assert_eq!(TAWNY.bullet_for_depth(3), '◦');
    assert_eq!(UNDERTOW.bullet_for_depth(0), '☞');
    assert_eq!(UNDERTOW.bullet_for_depth(1), '❧');
    // The manicule showpiece: Undertow alone rides the antique pointing hand,
    // at its top level (level 1).
    assert_eq!(UNDERTOW.bullets.0, '☞', "Undertow's level-1 bullet is the manicule");
    assert!(
        THEMES.iter().filter(|t| t.bullets.0 == '☞' || t.bullets.1 == '☞').count() == 1,
        "exactly one world uses the manicule bullet (a hand everywhere is loud)"
    );
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
    let mincho = ["Gumtree", "Saltpan", "Bilby", "Undertow", "Outback", "Magpie"];
    let klee = ["Mopoke", "Quokka"];
    let gothic = ["Tawny", "Potoroo", "Mangrove", "Galah", "Kingfisher", "Currawong", "Wagtail", "Firetail"];
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
    let serif = ["Gumtree", "Bilby", "Undertow", "Saltpan", "Outback", "Magpie"];
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
/// THIS is the coverage check WORLD-ROLES.md means by "the axes become a build-time
/// ruler" — no runtime picker consults it.
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
    assert_eq!(set_active_by_name("OUTBACK").unwrap().name, "Outback");
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
        // selection would be sub-glance over its own ground lifts it (Undertow /
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
/// Undertow's Ghost was near-invisible). On every DARK world both placard
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
             (near-invisible — the Undertow gallery bug)",
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
                card_anchor: model::CardAnchor::TopLeft,
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
            "Currawong" => RenderCaps {
                elevation: Elevation::Bordered,
                card_anchor: model::CardAnchor::TopLeft,
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
            },
            // LIGHT-WORLD BORDER (composition round item 6): the four remaining
            // pale-ground worlds gain the summoned-card border, DATA-only.
            "Gumtree" | "Bilby" | "Saltpan" | "Quokka" => {
                RenderCaps { elevation: Elevation::Bordered, ..RenderCaps::DEFAULT }
            }
            "Tawny" | "Mopoke" | "Potoroo" | "Undertow" | "Kingfisher" | "Outback" => {
                RenderCaps::DEFAULT
            }
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
        // A MONOCHROME world's caret IS its ink (value + motion carry it —
        // Wagtail's pure white), so "never literally primary" is structurally
        // inapplicable there; every chromatic world must keep them distinct.
        if !t.is_monochrome() {
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
            assert_ne!(*c, primary(), "{}: heatmap level {i} must never be literally the accent", t.name);
        }
    }
    set_active(DEFAULT_THEME);
}
