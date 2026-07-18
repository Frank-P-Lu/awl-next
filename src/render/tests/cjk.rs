//! Per-script (ja/zh-Hans/zh-Hant/ko) font resolution -- the never-tofu law,
//! bundled-face registration, and `add_script_spans`'s per-run resolution
//! ladder -- split out of the former monolithic `render::tests` (2026-07
//! code-organization pass). See `theme` for the theme-switch reshape tests.

use super::super::*;
use super::{headless_pipeline};

/// THE NEVER-TOFU LAW (font-DB half — complements `theme::tests::
/// every_font_id_has_a_nonempty_candidate_ladder_on_every_world`'s
/// structural check): `FontId::Latin` and `FontId::Ja` resolve to a
/// CONCRETELY-REGISTERED face via the real font DB on EVERY world, in a
/// normal build — the guaranteed floor. Both ladders' first candidate is
/// always a bundled embedded face (the world's own `Theme::font` for
/// Latin; bundled Noto Serif/Sans JP for Ja — see `theme::CJK_MINCHO`/
/// `CJK_GOTHIC`), so this never depends on what's installed on the
/// machine running the test. zh-Hans/zh-Hant/ko are NOT asserted here —
/// v1 ships no bundled asset for them, so whether they resolve is
/// genuinely machine-dependent (the documented degenerate path: `None` ->
/// no span added -> cosmic-text's neutral fallback, never a panic).
#[test]
fn latin_and_ja_always_resolve_to_an_embedded_face() {
    let _t = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping latin_and_ja_always_resolve_to_an_embedded_face: no wgpu adapter");
        return;
    };
    for t in theme::THEMES.iter() {
        theme::set_active_by_name(t.name).unwrap();
        p.sync_theme();
        assert!(
            p.resolve_font_id(theme::FontId::Latin).is_some(),
            "{}: Latin must always resolve (its own embedded display face)",
            t.name
        );
        assert!(
            p.resolve_font_id(theme::FontId::Ja).is_some(),
            "{}: Ja must always resolve (bundled Noto Serif/Sans JP)",
            t.name
        );
    }
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// NEVER-TOFU (per-world ORNAMENT FACE): every world's three section-break
/// glyphs (`Ornaments::dash`/`star`/`underscore`) resolve to a REAL glyph in
/// that world's assigned [`theme::Theme::ornament_face`] — no world can ship a
/// fleuron its own ornament face lacks (the ⁂/❡/❥-not-in-EB-Garamond trap). The
/// font-DB half of the structural `theme::tests::
/// every_world_ornament_face_is_a_registered_ornament_face` law. Also pins the
/// design-table contract that the three glyphs are DISTINCT per world (dash /
/// star / underscore each read as their own symbol, never a shared fallback).
#[test]
fn ornament_glyphs_resolve_in_each_worlds_assigned_face() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping ornament_glyphs_resolve_in_each_worlds_assigned_face: no wgpu adapter");
        return;
    };
    for t in theme::THEMES.iter() {
        let (d, s, u) = (t.ornaments.dash, t.ornaments.star, t.ornaments.underscore);
        assert!(
            d != s && s != u && d != u,
            "{}: ornament trio must be THREE DISTINCT glyphs, got dash={:?} star={:?} underscore={:?}",
            t.name,
            d,
            s,
            u
        );
        let id = p
            .font_system
            .db()
            .faces()
            .find(|f| f.families.iter().any(|(n, _)| n == t.ornament_face))
            .map(|f| f.id)
            .unwrap_or_else(|| panic!("{}: ornament face {:?} is registered", t.name, t.ornament_face));
        let font = p
            .font_system
            .get_font(id, glyphon::cosmic_text::fontdb::Weight::NORMAL)
            .unwrap_or_else(|| panic!("{}: ornament face {:?} loads", t.name, t.ornament_face));
        let charmap = font.as_swash().charmap();
        for (label, ch) in [
            ("dash `---`", t.ornaments.dash),
            ("star `***`", t.ornaments.star),
            ("underscore `___`", t.ornaments.underscore),
        ] {
            assert!(
                charmap.map(ch) != 0,
                "{}: {} glyph {:?} (U+{:04X}) is NOT in its ornament face {:?} — renders as tofu",
                t.name,
                label,
                ch,
                ch as u32,
                t.ornament_face
            );
        }
    }
}

/// THE CHINESE ROUND extends the never-tofu floor to `ZhHans`/`Ko`: since
/// both now bundle a face too (Noto Serif/Sans SC + LXGW WenKai for
/// zh-Hans; Noto Sans KR for ko — `render::FONT_ZH_KO_FACES`), they
/// resolve on EVERY world in a normal build, exactly like Latin/Ja.
/// `ZhHant` is deliberately NOT asserted here — it still ships no bundled
/// asset this round (Big5 subsetting is banked), so whether it resolves
/// stays genuinely machine-dependent (the documented degenerate path).
#[test]
fn zh_hans_and_ko_always_resolve_to_an_embedded_face() {
    let _t = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping zh_hans_and_ko_always_resolve_to_an_embedded_face: no wgpu adapter");
        return;
    };
    for t in theme::THEMES.iter() {
        theme::set_active_by_name(t.name).unwrap();
        p.sync_theme();
        assert!(
            p.resolve_font_id(theme::FontId::ZhHans).is_some(),
            "{}: ZhHans must always resolve (bundled Noto Serif/Sans SC or LXGW WenKai)",
            t.name
        );
        assert!(
            p.resolve_font_id(theme::FontId::Ko).is_some(),
            "{}: Ko must always resolve (bundled Gowun Batang on serif worlds, else Noto Sans KR)",
            t.name
        );
    }
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// PER-FACE registration: each of the Chinese round's four bundled faces
/// registers in the font DB under its exact expected family name (the
/// same "verified through fontdb" guarantee `FONT_CJK_FACES`'s JP pair
/// already carries) — a subsetting/instancing mistake that silently
/// renamed or corrupted a face would fail this immediately rather than
/// surfacing as a confusing tofu box downstream.
#[test]
fn zh_ko_faces_register_under_their_expected_family_names() {
    let Some(p) = headless_pipeline() else {
        eprintln!("skipping zh_ko_faces_register_under_their_expected_family_names: no wgpu adapter");
        return;
    };
    for expected in ["Noto Serif SC", "Noto Sans SC", "Noto Sans KR", "LXGW WenKai"] {
        let registered = p
            .font_system
            .db()
            .faces()
            .any(|f| f.families.iter().any(|(n, _)| n == expected));
        assert!(registered, "{expected:?} must be registered in the font DB");
    }
}

/// PER-FACE registration (Phase 2 "JP face variety" round): each of the
/// three new bundled JP faces ([`render::FONT_JA_VARIETY_FACES`]) registers
/// under its exact expected family name — the same "verified through fontdb"
/// guarantee the Noto/Chinese faces carry. A subsetting mistake that renamed
/// or corrupted a face fails HERE, not as a downstream tofu box.
#[test]
fn ja_variety_faces_register_under_their_expected_family_names() {
    let Some(p) = headless_pipeline() else {
        eprintln!("skipping ja_variety_faces_register_under_their_expected_family_names: no wgpu adapter");
        return;
    };
    for expected in ["Shippori Mincho", "Zen Maru Gothic", "Klee One"] {
        let registered = p
            .font_system
            .db()
            .faces()
            .any(|f| f.families.iter().any(|(n, _)| n == expected));
        assert!(registered, "{expected:?} must be registered in the font DB");
    }
}

/// Phase 2 "JP face variety": each reassigned world's `FontId::Ja` resolves
/// to its NEW bundled face on the real font DB (machine-independent, since
/// each ladder names the bundled face FIRST) — the font-DB half of the
/// `theme::tests::cjk_fallback_matches_world_character` structural law. This
/// is the fact the capture test asserts through the sidecar, proven here at
/// the purest reachable seam.
#[test]
fn ja_variety_worlds_resolve_their_new_bundled_face() {
    let _t = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping ja_variety_worlds_resolve_their_new_bundled_face: no wgpu adapter");
        return;
    };
    // (world, expected FontId::Ja family) — one per new ladder, both members.
    let cases = [
        ("Gumtree", "Shippori Mincho"),
        ("Bilby", "Shippori Mincho"),
        ("Bombora", "Shippori Mincho"),
        ("Galah", "Zen Maru Gothic"),
        ("Bowerbird", "Zen Maru Gothic"),
        ("Mopoke", "Klee One"),
        ("Quokka", "Klee One"),
        // Two worlds this round left ALONE keep the neutral Noto face.
        ("Saltpan", "Noto Serif JP"),
        ("Currawong", "Noto Sans JP"),
    ];
    for (world, want) in cases {
        theme::set_active_by_name(world).unwrap();
        p.sync_theme();
        let (fam, _) = p
            .resolve_font_id(theme::FontId::Ja)
            .unwrap_or_else(|| panic!("{world}: Ja must resolve"));
        assert_eq!(fam, want, "{world}: Ja should resolve to {want}");
    }
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// PER-FACE registration ("CJK companions" round): the one bundled Korean
/// serif companion ([`render::FONT_CJK_COMPANION_FACES`]) registers under its
/// exact expected family name — the same "verified through fontdb" guarantee
/// the JP/ZH faces carry. A subsetting/rename mistake fails HERE, not as a
/// downstream tofu box.
#[test]
fn ko_companion_face_registers_under_its_family_name() {
    let Some(p) = headless_pipeline() else {
        eprintln!("skipping ko_companion_face_registers_under_its_family_name: no wgpu adapter");
        return;
    };
    let registered = p
        .font_system
        .db()
        .faces()
        .any(|f| f.families.iter().any(|(n, _)| n == "Gowun Batang"));
    assert!(registered, "\"Gowun Batang\" must be registered in the font DB");
}

/// "CJK companions" round: each SERIF world's `FontId::Ko` resolves to the
/// bundled Gowun Batang on the real font DB (machine-independent — the serif
/// ko ladder names it FIRST), while a SANS/MONO world's `Ko` stays the
/// neutral Noto Sans KR floor. The font-DB half of the
/// `theme::tests::zh_hant_uniform_ko_splits_serif_from_sans` structural law,
/// proven at the purest reachable seam (mirrors
/// `ja_variety_worlds_resolve_their_new_bundled_face`).
#[test]
fn ko_serif_worlds_resolve_gowun_batang() {
    let _t = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping ko_serif_worlds_resolve_gowun_batang: no wgpu adapter");
        return;
    };
    // (world, expected FontId::Ko family) — serif worlds get Gowun Batang;
    // two sans/mono controls keep the Noto Sans KR floor.
    let cases = [
        ("Gumtree", "Gowun Batang"),
        ("Bilby", "Gowun Batang"),
        ("Bombora", "Gowun Batang"),
        ("Saltpan", "Gowun Batang"),
        ("Mulga", "Gowun Batang"),
        ("Magpie", "Gowun Batang"),
        // Sans/mono controls — the neutral bundled floor, never Gowun Batang.
        ("Currawong", "Noto Sans KR"),
        ("Bowerbird", "Noto Sans KR"),
    ];
    for (world, want) in cases {
        theme::set_active_by_name(world).unwrap();
        p.sync_theme();
        let (fam, _) = p
            .resolve_font_id(theme::FontId::Ko)
            .unwrap_or_else(|| panic!("{world}: Ko must resolve"));
        assert_eq!(fam, want, "{world}: Ko should resolve to {want}");
    }
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

fn family_name(al: &glyphon::cosmic_text::AttrsList, byte: usize) -> Option<String> {
    match al.get_span(byte).family {
        Family::Name(n) => Some(n.to_string()),
        _ => None,
    }
}

#[test]
fn add_script_spans_ja_tagged_doc_with_hangul_run_uses_ko_not_ja() {
    // THE task-spec example verbatim: a ja-tagged doc with an embedded
    // hangul run. Step (a) has no ko mapping for a `ja` tag -> falls to
    // step (b): the run's OWN script (hangul -> ko).
    let fonts = super::text::ScriptFonts {
        ja: Some(("JaFace", glyphon::Weight(400))),
        zh_hans: None,
        zh_hant: None,
        ko: Some(("KoFace", glyphon::Weight(400))),
    };
    let base = Attrs::new();
    let text = "한글"; // pure hangul
    let mut al = glyphon::cosmic_text::AttrsList::new(&base);
    add_script_spans(
        &mut al, text, &base, Some(crate::frontmatter::Lang::Ja),
        &crate::frontmatter::DEFAULT_CJK_PRIORITY, &fonts,
    );
    assert_eq!(family_name(&al, 0), Some("KoFace".to_string()));
}

#[test]
fn add_script_spans_ja_tagged_doc_with_han_run_uses_ja() {
    // A ja tag DOES map Han (kanji) -> its own step (a) mapping wins.
    let fonts = super::text::ScriptFonts {
        ja: Some(("JaFace", glyphon::Weight(400))),
        zh_hans: Some(("ZhHansFace", glyphon::Weight(400))),
        zh_hant: None,
        ko: None,
    };
    let base = Attrs::new();
    let text = "日本語"; // pure han (kanji)
    let mut al = glyphon::cosmic_text::AttrsList::new(&base);
    add_script_spans(
        &mut al, text, &base, Some(crate::frontmatter::Lang::Ja),
        &crate::frontmatter::DEFAULT_CJK_PRIORITY, &fonts,
    );
    assert_eq!(family_name(&al, 0), Some("JaFace".to_string()));
}

#[test]
fn add_script_spans_untagged_han_uses_cjk_priority_tiebreak() {
    // No doc tag at all: an untagged Han-only run falls to (c), the
    // cjk_priority ladder — here configured zh-Hans-first.
    let fonts = super::text::ScriptFonts {
        ja: Some(("JaFace", glyphon::Weight(400))),
        zh_hans: Some(("ZhHansFace", glyphon::Weight(400))),
        zh_hant: None,
        ko: None,
    };
    let base = Attrs::new();
    let text = "汉字";
    let priority = [
        crate::frontmatter::Lang::ZhHans,
        crate::frontmatter::Lang::Ja,
        crate::frontmatter::Lang::ZhHant,
        crate::frontmatter::Lang::Ko,
    ];
    let mut al = glyphon::cosmic_text::AttrsList::new(&base);
    add_script_spans(&mut al, text, &base, None, &priority, &fonts);
    assert_eq!(family_name(&al, 0), Some("ZhHansFace".to_string()));
}

#[test]
fn add_script_spans_mixed_run_each_script_resolves_independently() {
    // "hi漢字ですは" -- latin "hi" (untouched), han "漢字" (-> ja tag),
    // kana "ですは" (-> ja, unambiguous) — every script resolves per-run.
    let fonts = super::text::ScriptFonts {
        ja: Some(("JaFace", glyphon::Weight(400))),
        zh_hans: None,
        zh_hant: None,
        ko: None,
    };
    let base = Attrs::new();
    let text = "hi漢字ですは";
    let mut al = glyphon::cosmic_text::AttrsList::new(&base);
    add_script_spans(
        &mut al, text, &base, Some(crate::frontmatter::Lang::Ja),
        &crate::frontmatter::DEFAULT_CJK_PRIORITY, &fonts,
    );
    // "hi" (bytes 0..2): no override -> base family (no Name span).
    assert_eq!(family_name(&al, 0), None, "the latin run must not be overridden");
    // "漢" starts at byte 2 (han).
    assert_eq!(family_name(&al, 2), Some("JaFace".to_string()));
    // "で" starts after "漢字" (2 kanji, 3 bytes each = byte 8) (kana).
    assert_eq!(family_name(&al, 8), Some("JaFace".to_string()));
}

#[test]
fn add_script_spans_unresolved_script_leaves_base_face() {
    // zh-Hans has NO candidate resolved on this machine (`None`) — the
    // documented degenerate case: no override span, base face wins.
    let fonts = super::text::ScriptFonts { ja: None, zh_hans: None, zh_hant: None, ko: None };
    let base = Attrs::new();
    let text = "汉字";
    let mut al = glyphon::cosmic_text::AttrsList::new(&base);
    add_script_spans(&mut al, text, &base, None, &crate::frontmatter::DEFAULT_CJK_PRIORITY, &fonts);
    assert_eq!(family_name(&al, 0), None, "no candidate resolved -> no override span");
}

#[test]
fn add_script_spans_pins_weight_and_style_over_bold_italic_base() {
    // THE bold/italic-breaks-Japanese fix at its purest seam: a CJK run must
    // resolve to its face's REGISTERED weight+style (400/Normal for every
    // bundled CJK face — no bold/italic CJK cut exists in v1), NEVER a
    // `**bold**`(700) / `*italic*` emphasis leaking onto it. Model the worst
    // case explicitly — a base ALREADY carrying Weight::BOLD + Style::Italic
    // (as if an emphasis span sat under the run) — and assert the script span
    // overwrites BOTH. Pre-fix the weight was pinned but the STYLE was
    // inherited from the base, so the italic leaked; the `.style(Normal)` pin
    // closes it.
    let fonts = super::text::ScriptFonts {
        ja: Some(("JaFace", glyphon::Weight(400))),
        zh_hans: None,
        zh_hant: None,
        ko: None,
    };
    let base = Attrs::new()
        .weight(glyphon::Weight::BOLD)
        .style(glyphon::Style::Italic);
    let text = "太字"; // pure kanji
    let mut al = glyphon::cosmic_text::AttrsList::new(&base);
    add_script_spans(
        &mut al,
        text,
        &base,
        Some(crate::frontmatter::Lang::Ja),
        &crate::frontmatter::DEFAULT_CJK_PRIORITY,
        &fonts,
    );
    let a = al.get_span(0);
    assert_eq!(family_name(&al, 0), Some("JaFace".to_string()), "CJK run keeps its resolved face");
    assert_eq!(a.weight, glyphon::Weight(400), "weight pinned to the resolved face's 400, not the bold 700");
    assert_eq!(a.style, glyphon::Style::Normal, "style pinned to Normal, not the italic base");
}
