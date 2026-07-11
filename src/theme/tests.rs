//! Tests for the `theme` module (the fifteen worlds + their derivation laws)
//! -- split verbatim out of the former `theme.rs` monolith's embedded
//! `mod tests` (2026-07 code-organization pass); every test's NAME and MODULE
//! PATH are unchanged (`theme::tests::foo`) -- only which file its source
//! lives in moved.

use super::*;
use super::derive::{theme_bucket, SELECTED_BAND_STEPS, THEME_FACET_STRIP};
use crate::facets::FacetItem;


#[test]
fn worlds_nine_dark_six_light() {
    assert_eq!(THEMES.len(), 15);
    let dark = THEMES.iter().filter(|t| t.dark).count();
    let light = THEMES.iter().filter(|t| !t.dark).count();
    // 9 dark (Tawny/Mopoke/Currawong/Potoroo/Undertow/Kingfisher/Outback/
    // Mangrove/Wagtail) / 6 light (Gumtree/Bilby/Saltpan/Quokka/Galah/Magpie).
    assert_eq!(dark, 9);
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
    // The whole ground palette is exercised across the worlds (Stripes is new,
    // assigned to Potoroo; the proximity-scaled Dots ride Mangrove).
    let used: std::collections::HashSet<&str> =
        THEMES.iter().map(|t| t.background.as_str()).collect();
    for p in ["gradient", "dots", "starfield", "pinstripe", "stripes"] {
        assert!(used.contains(p), "ground {p} unused by any world");
    }
    // Exactly the two assigned worlds carry the NEW grounds.
    let stripes: Vec<&str> = THEMES
        .iter()
        .filter(|t| matches!(t.background, Background::Stripes { .. }))
        .map(|t| t.name)
        .collect();
    assert_eq!(stripes, ["Potoroo"], "Stripes is Potoroo's alone");
    let edge_dots: Vec<&str> = THEMES
        .iter()
        .filter(|t| t.background.edge())
        .map(|t| t.name)
        .collect();
    assert_eq!(edge_dots, ["Mangrove"], "proximity Dots is Mangrove's alone");
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
    // Wagtail is the FIFTH — and the first to share its exact display font with
    // another world (Mangrove, also JetBrains Mono) — a logged, honest
    // consequence of adding a 15th world without bundling a 15th display face;
    // see `worlds.rs::WAGTAIL`'s own doc comment.
    const MONO_DISPLAY: [&str; 5] = ["Tawny", "Currawong", "Potoroo", "Mangrove", "Wagtail"];
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
    let gothic = ["Tawny", "Potoroo", "Mangrove", "Currawong", "Wagtail"]; // neutral sans/mono (Noto Sans JP)
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
    let gothic = ["Tawny", "Potoroo", "Mangrove", "Galah", "Kingfisher", "Currawong", "Wagtail"];
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
        // Every declared header shows a CURATED 2–3 worlds: never an empty faint
        // header, never the pre-curation crowd (Time=Night once held 6).
        for sect in sections {
            let n = THEMES
                .iter()
                .filter(|t| t.tags.section(lens) == Some(*sect))
                .count();
            assert!(
                (2..=3).contains(&n),
                "{:?} section {sect:?} shows {n} worlds (curation wants 2–3)",
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
fn default_is_dark() {
    assert!(THEMES[DEFAULT_THEME].dark);
    assert_eq!(THEMES[DEFAULT_THEME].name, "Tawny");
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
