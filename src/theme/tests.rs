//! Tests for the `theme` module (the fifteen worlds + their derivation laws)
//! -- split verbatim out of the former `theme.rs` monolith's embedded
//! `mod tests` (2026-07 code-organization pass); every test's NAME and MODULE
//! PATH are unchanged (`theme::tests::foo`) -- only which file its source
//! lives in moved.

use super::*;
use super::derive::{theme_bucket, SELECTED_BAND_STEPS, THEME_FACET_STRIP};
use crate::facets::FacetItem;


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
        //     core_t at field ≥ THRESHOLD + CORE_WIDTH = 0.85; the strongest base blob's
        //     weight 1.2 alone exceeds that at its own animated center) — so the ground
        //     genuinely reaches blob_hi, and (1) is a check on an ACTUAL worst-phase pixel.
        let vp = (1200.0, 800.0);
        let mut peak = 0.0f32;
        for step in 0..64 {
            let phase = step as f32 / 64.0;
            for (i, b) in crate::lava::BASE_BLOBS.iter().enumerate() {
                let (cx, cy) = crate::lava::animated_center(i, b[0], b[1], phase);
                let px = (cx * vp.0, cy * vp.1);
                peak = peak.max(crate::lava::metaball_field(px, vp, &crate::lava::BASE_BLOBS, phase));
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
/// `primary`. Firetail's wine blobs clear it at ~44°; Mangrove's cool blues at ~175°.
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

/// OPT-OUT faceting: a world may be `None` (hidden) on a lens, but any `Some(tag)`
/// must be one of that lens's declared sections (so grouping can never place a world
/// under a header that doesn't exist). Also asserts the CURATION invariant — every
/// faceted bucket shows a curated 2–3 worlds (never empty, never crowded) — that the
/// name-keyed accessor agrees with the inline field, that every world HEADLINES at
/// least one faceted lens (still findable by browsing, not only by search), and that
/// `All` groups nothing.
#[test]
fn every_world_curated_into_lenses() {
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
    // Every world headlines at least ONE faceted lens (present under some section),
    // so it is reachable by browsing lenses, not only via All + fuzzy search.
    for t in THEMES.iter() {
        let shown = [Lens::Time, Lens::Register, Lens::Voice, Lens::Temperature]
            .iter()
            .any(|&l| t.tags.section(l).is_some());
        assert!(shown, "{} is hidden on every lens (headlines none)", t.name);
    }
    // All lens groups nothing (flat list).
    assert!(Lens::All.sections().is_empty());
    assert_eq!(THEMES[0].tags.section(Lens::All), None);
    // The strip parks All at the far LEFT.
    assert_eq!(*Lens::STRIP.first().unwrap(), Lens::All);
    assert_eq!(Lens::STRIP.len(), 5);
}

/// DRIFT GUARD: the generic [`THEME_FACET_STRIP`] (the `FacetScheme` the overlay
/// consults) mirrors [`Lens::STRIP`] element-for-element — same order, labels,
/// sidecar ids, and section lists — and [`theme_bucket`] agrees with [`tag_for`]
/// on every world. So the theme picker's generic scheme can never diverge from
/// the `Lens` source of truth.
#[test]
fn theme_facet_strip_matches_lens() {
    assert_eq!(THEME_FACET_STRIP.len(), Lens::STRIP.len());
    for (facet, lens) in THEME_FACET_STRIP.iter().zip(Lens::STRIP.iter()) {
        assert_eq!(facet.label, lens.label(), "{lens:?} label drift");
        assert_eq!(facet.id, lens.as_str(), "{lens:?} id drift");
        assert_eq!(facet.sections, lens.sections(), "{lens:?} sections drift");
    }
    // theme_bucket (strip index) == tag_for (lens) for every world × every lens.
    for (idx, lens) in Lens::STRIP.iter().enumerate() {
        for t in THEMES.iter() {
            let item = FacetItem::new(t.name);
            assert_eq!(theme_bucket(item, idx), tag_for(t.name, *lens));
        }
    }
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

/// The fourteen worlds map onto at least SIX CLEARLY-distinct display faces
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

/// THE LAW ROUND's `RenderCaps::highlight_treatment` — a NO-ABSENT-VARIANT
/// enum consumed by `render/chrome/overlay.rs`'s picker-row highlight and
/// `render/chrome/menubar.rs`'s open-title band, replacing the former
/// hand-rolled `if selection_style == InverseVideo { .. } else { .. }` at
/// each of those two sites. This pins the STRUCTURAL half of the contract
/// (every world resolves to EXACTLY the treatment its `selection_style`
/// names, with no third "neither" outcome reachable) across all fifteen
/// worlds; the REAL-PIXEL half — does the renderer actually honor it — lives
/// in `render::tests::distinguishability`.
#[test]
fn highlight_treatment_matches_selection_style_on_every_world_no_absent_case() {
    for t in THEMES.iter() {
        let band = crate::theme::Srgb::rgb(0x11, 0x22, 0x33);
        let treatment = t.render_caps.highlight_treatment(band);
        match (t.render_caps.selection_style, treatment) {
            (
                crate::theme::SelectionStyle::Fill,
                crate::theme::HighlightTreatment::ValueBand(c),
            ) => {
                assert_eq!(c, band, "{}: ValueBand must carry the caller's own band color", t.name);
            }
            (
                crate::theme::SelectionStyle::InverseVideo,
                crate::theme::HighlightTreatment::Invert,
            ) => {}
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

/// `theme::placard_ink` NEVER invents a free color — `Faint` is exactly
/// [`derive::faint`], and `Ghost` is a pure blend of two tokens already on
/// the active world's own palette (`faint` and `base_300`), for EVERY world.
#[test]
fn placard_ink_derives_from_the_ink_ladder_never_a_free_color() {
    let _g = crate::testlock::serial();
    for t in THEMES.iter() {
        set_active_by_name(t.name).unwrap();
        assert_eq!(
            derive::placard_ink(model::PlacardInk::Faint),
            t.faint,
            "{}: PlacardInk::Faint must be exactly the world's own faint ink",
            t.name
        );
        let ghost = derive::placard_ink(model::PlacardInk::Ghost);
        let expected = t.faint.lerp(t.base_300, 0.5);
        assert_eq!(ghost, expected, "{}: PlacardInk::Ghost must be a pure faint/base_300 blend", t.name);
    }
    set_active(DEFAULT_THEME);
}

/// ALL FIFTEEN worlds ship [`model::TitleStyle::InlinePrefix`] this round —
/// the byte-identity gate the round's own spec demands (no world's rendering
/// may change). A future round FLIPPING a world to `Placard` edits this
/// test consciously; it can never happen by accident.
#[test]
fn every_world_ships_inline_prefix_title_style_this_round() {
    for t in THEMES.iter() {
        assert!(
            matches!(t.render_caps.title_style, model::TitleStyle::InlinePrefix),
            "{}: expected InlinePrefix (no world assigns Placard yet)",
            t.name
        );
    }
}

/// REPAIR ROUND 2's flagged gap, closed structurally: a `TitleStyle::Placard`
/// paired with `PlacardInk::Ghost` on a TRUE 1-BIT world (`Theme::is_one_bit`)
/// would render a plain mid-grey wordmark — a `faint`/`base_300` blend is an
/// ordinary intermediate grey on every world today, and a 1-bit world's own
/// law (`render::tests::syntax_roles::every_one_bit_world_renders_only_pure_
/// black_or_white`) permits ONLY pure black or pure white, no grey rung at
/// all. No world ships `Placard` yet (the test above pins that), so this is
/// a BANKED guard against a future assignment, not a live bug — but the
/// guard itself is real: it fails loudly the moment any world's
/// `render_caps.title_style` becomes `Placard { ink: Ghost, .. }` while that
/// same world is `is_one_bit()`. Lives in `theme::`, deliberately never
/// `render::`, where a bare `.is_one_bit()` call is banned outright
/// (`render::tests::theme_caps_law`) — this is exactly the "pin an identity,
/// not a render mechanism" carve-out that grep-law's own doc describes.
#[test]
fn a_placard_ghost_title_style_would_violate_a_one_bit_worlds_own_law() {
    for t in THEMES.iter() {
        if let model::TitleStyle::Placard { ink: model::PlacardInk::Ghost, .. } = t.render_caps.title_style {
            assert!(
                !t.is_one_bit(),
                "{}: TitleStyle::Placard{{ink: Ghost}} on a true 1-bit world renders an \
                 illegal intermediate grey — pick PlacardInk::Faint isn't legal there either \
                 (still an ordinary grey); a 1-bit world needs its own render_caps escape hatch \
                 (mirroring Wagtail's own render_caps overrides) before it can ship a placard at all",
                t.name
            );
        }
    }
}
