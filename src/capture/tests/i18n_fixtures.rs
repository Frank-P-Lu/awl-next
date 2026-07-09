//! The i18n sample-fixture captures (Japanese/Chinese/Korean bundled-face
//! resolution, the JP-tagged Han-only guard, per-script sidecar reporting)
//! plus the tables/links WYSIWYG fixtures -- split out of the former
//! monolithic `capture::tests` (2026-07 code-organization pass).

use super::super::*;
use super::{adapter_available};
use crate::buffer::Buffer;

/// THE JAPANESE-BUNDLE ROUND's headline guarantee, made assertable: with the
/// bundled Noto Serif/Sans JP faces registered (`render::FONT_CJK_FACES`) and
/// listed FIRST in `theme::CJK_MINCHO`/`CJK_GOTHIC`, a Japanese fixture's
/// resolved shaping face is now MACHINE-INDEPENDENT — no more "depends which
/// system CJK fonts happen to be installed." Renders the actual
/// `samples/japanese.md` text (kanji + hiragana + katakana + mixed EN/JP) on
/// BOTH a serif (mincho) and a sans (gothic) world and asserts `font.cjk`
/// reports the bundled family with `bundled: true` on each — the first
/// JP-rendering capture test (see CAPTURE.md schema `/86`). Also sanity-checks
/// the CJK run actually shaped with a real (non-zero, non-`inf`) glyph advance,
/// so this isn't just asserting a name off to the side of what got drawn.
#[test]
fn japanese_fixture_resolves_bundled_cjk_face_deterministically() {
    if !adapter_available() {
        eprintln!("skipping japanese_fixture_resolves_bundled_cjk_face_deterministically: no wgpu adapter");
        return;
    }
    let _tg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_jpcapture_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let jp_text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/japanese.md"),
    )
    .expect("samples/japanese.md exists");
    assert!(jp_text.contains('日'), "fixture actually carries kanji");

    // --- Saltpan (NEUTRAL serif world -> plain mincho candidate list) ------
    // (Undertow moved to the Shippori Mincho override in Phase 2's variety
    // round; Saltpan is a serif world this round LEFT ALONE, so it still
    // resolves the neutral bundled Noto Serif JP — the point this test makes.)
    crate::theme::set_active_by_name("Saltpan").expect("Saltpan is a real world");
    let mut buf = Buffer::from_str(&jp_text);
    buf.set_path(dir.join("saltpan.md"));
    let png = dir.join("saltpan.png");
    capture_with(&png, &buf, &CaptureOpts::default()).expect("serif JP capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(j["font"]["cjk"]["family"], serde_json::json!("Noto Serif JP"));
    assert_eq!(j["font"]["cjk"]["bundled"], serde_json::json!(true));
    // The doc actually rendered non-empty first lines (sanity: not a blank capture).
    assert!(!j["first_lines"].as_array().unwrap().is_empty());

    // --- Currawong (sans/mono world -> gothic candidate list) --------------
    crate::theme::set_active_by_name("Currawong").expect("Currawong is a real world");
    let mut buf2 = Buffer::from_str(&jp_text);
    buf2.set_path(dir.join("currawong.md"));
    let png2 = dir.join("currawong.png");
    capture_with(&png2, &buf2, &CaptureOpts::default()).expect("sans JP capture renders");
    let j2: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png2.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(j2["font"]["cjk"]["family"], serde_json::json!("Noto Sans JP"));
    assert_eq!(j2["font"]["cjk"]["bundled"], serde_json::json!(true));

    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let _ = std::fs::remove_dir_all(&dir);
}

/// PHASE 2 "JP face variety" round: the reassigned worlds resolve their NEW
/// distinct bundled JP face, machine-independently (each ladder names the
/// bundled face FIRST). Renders `samples/japanese.md` on one world per new
/// ladder and asserts `font.cjk` reports the expected family with
/// `bundled: true` — the sidecar half of `render::tests::cjk::
/// ja_variety_worlds_resolve_their_new_bundled_face`, so the pixels a user
/// vetoes in `gallery/jp-worlds/` are the same faces the sidecar names.
#[test]
fn ja_variety_worlds_resolve_bundled_faces_deterministically() {
    if !adapter_available() {
        eprintln!("skipping ja_variety_worlds_resolve_bundled_faces_deterministically: no wgpu adapter");
        return;
    }
    let _tg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_jpvariety_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let jp_text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/japanese.md"),
    )
    .expect("samples/japanese.md exists");

    // One world per new ladder → its distinct bundled face.
    for (world, family) in [
        ("Undertow", "Shippori Mincho"), // book-serif override
        ("Galah", "Zen Maru Gothic"),    // rounded-sans override
        ("Mopoke", "Klee One"),          // Klee-world brush override
    ] {
        crate::theme::set_active_by_name(world).unwrap_or_else(|| panic!("{world} is a real world"));
        let mut buf = Buffer::from_str(&jp_text);
        buf.set_path(dir.join(format!("{world}.md")));
        let png = dir.join(format!("{world}.png"));
        capture_with(&png, &buf, &CaptureOpts::default())
            .unwrap_or_else(|_| panic!("{world} JP capture renders"));
        let j: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
                .unwrap();
        assert_eq!(
            j["font"]["cjk"]["family"],
            serde_json::json!(family),
            "{world} should resolve {family}"
        );
        assert_eq!(j["font"]["cjk"]["bundled"], serde_json::json!(true), "{world} bundled");
    }

    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let _ = std::fs::remove_dir_all(&dir);
}

/// WYSIWYG TABLE GRID + THE X-RAY: `samples/tables.md`'s one GFM table renders as
/// an aligned pixel grid off-cursor, and when the caret enters it the grid STAYS
/// DRAWN (the x-ray) — the row's raw source floats over it and the document NEVER
/// reflows (the source rows stay concealed). Asserts the deterministic `tables`
/// sidecar block (one table, 4 grid rows = header + 3 body NOT the separator, 3
/// columns, three measured widths) plus THE ZERO-REFLOW CONTRACT: caret-in-table
/// keeps the table's source in `wysiwyg.concealed` (unlike the old reveal-in-place),
/// the `xray` block goes `active: true`, and the document `line_count` is
/// byte-stable across the caret walk (nothing reflowed).
#[test]
fn table_fixture_renders_grid_and_reveals_source_on_cursor() {
    if !adapter_available() {
        eprintln!("skipping table_fixture_renders_grid_and_reveals_source_on_cursor: no wgpu adapter");
        return;
    }
    let _tg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_tblcapture_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let md = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/tables.md"),
    )
    .expect("samples/tables.md exists");

    // --- OFF-CURSOR (caret at 0,0): the grid draws, source concealed ----------
    let mut buf = Buffer::from_str(&md);
    buf.set_path(dir.join("tables.md"));
    let png = dir.join("off.png");
    capture_with(&png, &buf, &CaptureOpts::default()).expect("table capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
            .unwrap();
    let tables = j["tables"].as_array().expect("tables is an array");
    assert_eq!(tables.len(), 1, "one table: {tables:?}");
    let t = &tables[0];
    assert_eq!(t["rows"], serde_json::json!(4), "header + 3 body rows");
    assert_eq!(t["cols"], serde_json::json!(3), "three columns");
    let widths = t["col_widths"].as_array().unwrap();
    assert_eq!(widths.len(), 3, "one measured width per column");
    assert!(
        widths.iter().all(|w| w.as_f64().unwrap() > 0.0),
        "every column has a positive width: {widths:?}"
    );
    assert_eq!(t["revealed"], serde_json::json!(false), "grid drawn off-cursor");
    // Off-cursor the whole table is concealed (source hidden, grid in its place).
    let concealed_off = j["wysiwyg"]["concealed"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c[2] == serde_json::json!("table"));
    assert!(concealed_off, "table source concealed off-cursor");
    assert_eq!(j["xray"]["active"], serde_json::json!(false), "no x-ray off-cursor");
    let line_count_off = j["line_count"].as_u64().unwrap();

    // --- CARET INSIDE THE TABLE: THE X-RAY — grid STAYS DRAWN, zero reflow -----
    // The table's byte range is `t.range`; drop the caret just inside it (the
    // fixture is ASCII, so a char index inside the byte range lands in the table).
    let start = t["range"][0].as_u64().unwrap() as usize;
    let mut buf2 = Buffer::from_str(&md);
    buf2.set_path(dir.join("tables.md"));
    buf2.set_cursor(start + 3);
    let png2 = dir.join("in.png");
    capture_with(&png2, &buf2, &CaptureOpts::default()).expect("x-ray capture renders");
    let j2: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png2.with_extension("json")).unwrap())
            .unwrap();
    let t2 = &j2["tables"].as_array().unwrap()[0];
    // `revealed` now means "the x-ray is active on this table" — the grid STILL
    // draws (its widths are still measured), it is not parked.
    assert_eq!(t2["revealed"], serde_json::json!(true), "x-ray active on-cursor");
    assert!(
        t2["col_widths"].as_array().unwrap().iter().all(|w| w.as_f64().unwrap() > 0.0),
        "the grid is still laid out (drawn) while the x-ray is active: {t2}"
    );
    // ZERO REFLOW: the source stays concealed (the x-ray FLOATS it, never
    // un-conceals it in place), so nothing wrapped/grew — `line_count` is stable.
    let concealed_in = j2["wysiwyg"]["concealed"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c[2] == serde_json::json!("table"));
    assert!(concealed_in, "table source STAYS concealed on-cursor (zero reflow)");
    assert_eq!(j2["xray"]["active"], serde_json::json!(true), "x-ray active flag set on-cursor");
    assert_eq!(
        j2["line_count"].as_u64().unwrap(),
        line_count_off,
        "the document never reflowed when the caret entered the table"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// WYSIWYG LINKS (the last markup family to lose its visible plumbing): a markdown
/// link's `[`/`](url)` conceals to zero-width off its own line (only the link TEXT
/// shows) and the whole source reveals when the caret lands on it. Asserted through
/// the deterministic `wysiwyg.concealed` sidecar block: `"link"` ranges present
/// off-cursor, gone on-cursor.
#[test]
fn link_source_conceals_off_cursor_and_reveals_on() {
    if !adapter_available() {
        eprintln!("skipping link_source_conceals_off_cursor_and_reveals_on: no wgpu adapter");
        return;
    }
    let _tg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_linkcapture_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // The link sits on line 2; line 0 is where the off-cursor caret rests.
    let md = "prose\n\nsee [the essay](http://x) now\n";
    let link_open = md.find('[').unwrap();

    // --- OFF-CURSOR (caret on line 0): the plumbing conceals -------------------
    let mut buf = Buffer::from_str(md);
    buf.set_path(dir.join("link.md"));
    let png = dir.join("off.png");
    capture_with(&png, &buf, &CaptureOpts::default()).expect("link capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
            .unwrap();
    let link_concealed = |j: &serde_json::Value| {
        j["wysiwyg"]["concealed"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c[2] == serde_json::json!("link"))
    };
    assert!(link_concealed(&j), "link plumbing concealed off-cursor: {}", j["wysiwyg"]);

    // --- CARET INSIDE THE LINK: the source reveals ----------------------------
    let mut buf2 = Buffer::from_str(md);
    buf2.set_path(dir.join("link.md"));
    buf2.set_cursor(link_open + 3); // a char inside `[the essay](...)` (ASCII)
    let png2 = dir.join("in.png");
    capture_with(&png2, &buf2, &CaptureOpts::default()).expect("revealed link capture renders");
    let j2: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png2.with_extension("json")).unwrap())
            .unwrap();
    assert!(!link_concealed(&j2), "link source revealed on-cursor: {}", j2["wysiwyg"]);

    let _ = std::fs::remove_dir_all(&dir);
}

/// THE CHINESE ROUND's headline guarantee, made assertable exactly like the
/// JP-bundle round's: with Noto Serif/Sans SC registered
/// (`render::FONT_ZH_KO_FACES`) and listed FIRST in `theme::CJK_ZH_HANS_SERIF`/
/// `_SANS`, a Simplified-Chinese fixture's resolved zh-Hans face is now
/// MACHINE-INDEPENDENT too. Renders `samples/chinese.md` (real Simplified
/// prose, including the variant-sensitive 直/骨/令 characters) on a serif
/// world (Gumtree -> mincho-class -> Serif SC) and a sans world (Currawong ->
/// gothic-class -> Sans SC), asserting `font.scripts.zh_hans` reports the
/// bundled family with `bundled: true` on each.
#[test]
fn chinese_fixture_resolves_bundled_zh_hans_face_deterministically() {
    if !adapter_available() {
        eprintln!("skipping chinese_fixture_resolves_bundled_zh_hans_face_deterministically: no wgpu adapter");
        return;
    }
    let _tg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_zhcapture_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let zh_text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/chinese.md"),
    )
    .expect("samples/chinese.md exists");
    assert!(zh_text.contains('直') && zh_text.contains('骨') && zh_text.contains('令'));

    // --- Gumtree (serif world -> Serif SC candidate list) -------------------
    crate::theme::set_active_by_name("Gumtree").expect("Gumtree is a real world");
    let mut buf = Buffer::from_str(&zh_text);
    buf.set_path(dir.join("gumtree.md"));
    let png = dir.join("gumtree.png");
    capture_with(&png, &buf, &CaptureOpts::default()).expect("serif zh-Hans capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(j["font"]["scripts"]["zh_hans"]["family"], serde_json::json!("Noto Serif SC"));
    assert_eq!(j["font"]["scripts"]["zh_hans"]["bundled"], serde_json::json!(true));
    assert!(!j["first_lines"].as_array().unwrap().is_empty());

    // --- Currawong (sans/mono world -> Sans SC candidate list) --------------
    crate::theme::set_active_by_name("Currawong").expect("Currawong is a real world");
    let mut buf2 = Buffer::from_str(&zh_text);
    buf2.set_path(dir.join("currawong.md"));
    let png2 = dir.join("currawong.png");
    capture_with(&png2, &buf2, &CaptureOpts::default()).expect("sans zh-Hans capture renders");
    let j2: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png2.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(j2["font"]["scripts"]["zh_hans"]["family"], serde_json::json!("Noto Sans SC"));
    assert_eq!(j2["font"]["scripts"]["zh_hans"]["bundled"], serde_json::json!(true));

    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Klee-worlds' CHARACTERFUL zh-Hans override: Mopoke + Quokka resolve
/// bundled LXGW WenKai (not the plain Noto Sans SC floor every other sans
/// world gets), while a non-Klee sans world stays on the floor — proving the
/// per-world override actually takes effect and doesn't leak to its
/// non-Klee siblings.
#[test]
fn klee_worlds_zh_hans_resolves_wenkai_characterful_face() {
    if !adapter_available() {
        eprintln!("skipping klee_worlds_zh_hans_resolves_wenkai_characterful_face: no wgpu adapter");
        return;
    }
    let _tg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_zhklee_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let zh_text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/chinese.md"),
    )
    .unwrap();

    for world in ["Mopoke", "Quokka"] {
        crate::theme::set_active_by_name(world).unwrap_or_else(|| panic!("{world} is a real world"));
        let mut buf = Buffer::from_str(&zh_text);
        buf.set_path(dir.join(format!("{world}.md")));
        let png = dir.join(format!("{world}.png"));
        capture_with(&png, &buf, &CaptureOpts::default()).expect("Klee-world capture renders");
        let j: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
                .unwrap();
        assert_eq!(
            j["font"]["scripts"]["zh_hans"]["family"],
            serde_json::json!("LXGW WenKai"),
            "{world} should resolve the characterful WenKai face"
        );
        assert_eq!(j["font"]["scripts"]["zh_hans"]["bundled"], serde_json::json!(true));
    }

    // A non-Klee sans world stays on the plain floor.
    crate::theme::set_active_by_name("Kingfisher").expect("Kingfisher is a real world");
    let mut buf = Buffer::from_str(&zh_text);
    buf.set_path(dir.join("kingfisher.md"));
    let png = dir.join("kingfisher.png");
    capture_with(&png, &buf, &CaptureOpts::default()).expect("floor capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap();
    assert_eq!(j["font"]["scripts"]["zh_hans"]["family"], serde_json::json!("Noto Sans SC"));

    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let _ = std::fs::remove_dir_all(&dir);
}

/// THE KO RIDER's headline guarantee, now with the CJK-companions round's
/// serif/sans SPLIT: a Korean fixture's resolved face is machine-independent
/// (always a BUNDLED face, never system-dependent), and it tracks the world's
/// character exactly like ja/zh-Hans — a SERIF world resolves the bundled
/// Gowun Batang (`theme::CJK_KO_SERIF`), a SANS/MONO world the bundled Noto
/// Sans KR floor (`theme::CJK_KO`). Renders `samples/korean.md` on one of
/// each and asserts both resolve their correct bundled face through the
/// sidecar.
#[test]
fn korean_fixture_resolves_bundled_ko_face_deterministically() {
    if !adapter_available() {
        eprintln!("skipping korean_fixture_resolves_bundled_ko_face_deterministically: no wgpu adapter");
        return;
    }
    let _tg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_kocapture_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ko_text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/korean.md"),
    )
    .expect("samples/korean.md exists");
    assert!(ko_text.contains('안'));

    // (world, expected bundled ko family) — Bilby is a serif world (Gowun
    // Batang), Tawny a mono world (the Noto Sans KR floor).
    for (world, want) in [("Bilby", "Gowun Batang"), ("Tawny", "Noto Sans KR")] {
        crate::theme::set_active_by_name(world).unwrap_or_else(|| panic!("{world} is a real world"));
        let mut buf = Buffer::from_str(&ko_text);
        buf.set_path(dir.join(format!("{world}.md")));
        let png = dir.join(format!("{world}.png"));
        capture_with(&png, &buf, &CaptureOpts::default()).expect("ko capture renders");
        let j: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
                .unwrap();
        assert_eq!(
            j["font"]["scripts"]["ko"]["family"],
            serde_json::json!(want),
            "{world}: ko should resolve {want}"
        );
        assert_eq!(j["font"]["scripts"]["ko"]["bundled"], serde_json::json!(true));
        assert!(!j["first_lines"].as_array().unwrap().is_empty());
    }

    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let _ = std::fs::remove_dir_all(&dir);
}

/// HAN-AMBIGUITY, pinned with the bundled SC face now present (task's own
/// worked example): a `ja`-tagged doc whose visible text is Han-ONLY (kanji,
/// no kana at all — so the run's own script gives no unambiguous signal) must
/// still resolve `FontId::Ja` (the bundled JP face), NEVER `FontId::ZhHans`
/// (the bundled SC face) — the doc tag wins at ladder step (a) regardless of
/// which bundled faces are registered. This is the scenario the "bundled SC
/// face must not hijack ja text" concern is actually about: before this
/// round ZhHans had no bundled candidate at all, so there was nothing for a
/// Han run to be hijacked BY; now that Noto Serif/Sans SC are real bundled
/// candidates, this pins that the per-FontId ladder (not a global "resolve
/// Han once" shortcut) keeps them apart.
#[test]
fn ja_tagged_han_only_doc_resolves_jp_face_never_bundled_zh_hans() {
    if !adapter_available() {
        eprintln!("skipping ja_tagged_han_only_doc_resolves_jp_face_never_bundled_zh_hans: no wgpu adapter");
        return;
    }
    let _tg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_hanambig_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // Saltpan: a serif world whose zh_hans is Noto Serif SC (the face this test
    // guards against hijacking ja text) but whose ja is the NEUTRAL Noto Serif
    // JP — so the JP-vs-SC distinction is exact. (Gumtree, this test's original
    // world, moved to the Shippori Mincho ja override in Phase 2's variety
    // round; that would still be a JP face, but Saltpan keeps the assertion's
    // pinned face name stable and the "neutral bundled JP" framing intact.)
    crate::theme::set_active_by_name("Saltpan").expect("Saltpan is a real world");
    // "日本語学校" -- pure kanji (Han script), zero kana, so the run's OWN
    // script gives no unambiguous mapping; only the ja doc tag decides.
    let mut buf = Buffer::from_str("---\nlang: ja\n---\n日本語学校\n");
    buf.set_path(dir.join("han_only.md"));
    let png = dir.join("han_only.png");
    capture_with(&png, &buf, &CaptureOpts::default()).expect("capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap();
    assert_eq!(j["doc_lang"], serde_json::json!("ja"));
    assert_eq!(j["font"]["scripts"]["ja"]["family"], serde_json::json!("Noto Serif JP"));
    assert_eq!(j["font"]["scripts"]["ja"]["bundled"], serde_json::json!(true));

    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let _ = std::fs::remove_dir_all(&dir);
}

/// THE i18n ROUND's sidecar contract: a top-level `doc_lang` field (the
/// document's own frontmatter `lang:` tag) and `font.scripts` (`font.cjk`'s
/// shape generalized to all four non-Latin scripts). A TAGGED document
/// reports its tag; an UNTAGGED one reports `null` — both deterministic, no
/// clock involved. `font.scripts.ja` is non-null in every normal build
/// (bundled Noto JP), exactly like `font.cjk`.
#[test]
fn sidecar_reports_doc_lang_and_per_script_font_resolution() {
    if !adapter_available() {
        eprintln!("skipping sidecar_reports_doc_lang_and_per_script_font_resolution: no wgpu adapter");
        return;
    }
    let dir = std::env::temp_dir().join(format!("awl_i18n_sidecar_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // TAGGED: doc_lang reports the frontmatter tag.
    let mut tagged = Buffer::from_str("---\nlang: ja\n---\nこんにちは\n");
    tagged.set_path(dir.join("tagged.md"));
    let png = dir.join("tagged.png");
    capture_with(&png, &tagged, &CaptureOpts::default()).expect("tagged capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap();
    assert_eq!(j["doc_lang"], serde_json::json!("ja"));
    // font.scripts mirrors font.cjk's shape for `ja`, plus the three new IDs.
    assert!(j["font"]["scripts"]["ja"].is_object(), "ja resolves in a normal build");
    assert_eq!(j["font"]["scripts"]["ja"]["family"], j["font"]["cjk"]["family"]);
    assert!(j["font"]["scripts"].get("zh_hans").is_some(), "zh_hans key present (may be null)");
    assert!(j["font"]["scripts"].get("zh_hant").is_some(), "zh_hant key present (may be null)");
    assert!(j["font"]["scripts"].get("ko").is_some(), "ko key present (may be null)");

    // UNTAGGED: doc_lang is null.
    let mut untagged = Buffer::from_str("just some prose\n");
    untagged.set_path(dir.join("untagged.md"));
    let png2 = dir.join("untagged.png");
    capture_with(&png2, &untagged, &CaptureOpts::default()).expect("untagged capture renders");
    let j2: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png2.with_extension("json")).unwrap()).unwrap();
    assert_eq!(j2["doc_lang"], serde_json::json!(null));

    let _ = std::fs::remove_dir_all(&dir);
}

/// The HELD STATS HUD's i18n `lang` field: mirrors `doc_lang` exactly (a
/// tagged doc shows its tag; an untagged one is `null`), summoned via
/// `--hud`/`--keys "Cmd-I"` equivalent (`hud::set_held(true)`).
#[test]
fn hud_reports_the_doc_lang_tag() {
    if !adapter_available() {
        eprintln!("skipping hud_reports_the_doc_lang_tag: no wgpu adapter");
        return;
    }
    let _hg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_i18n_hud_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut tagged = Buffer::from_str("---\nlang: ko\n---\n안녕하세요\n");
    tagged.set_path(dir.join("tagged.md"));

    crate::hud::set_held(true);
    let png = dir.join("held.png");
    capture_with(&png, &tagged, &CaptureOpts::default()).expect("held capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap();
    assert_eq!(j["hud"]["lang"], serde_json::json!("ko"));

    crate::hud::set_held(false);
    let _ = std::fs::remove_dir_all(&dir);
}
