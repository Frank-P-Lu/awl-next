//! Code-buffer mono shaping + the three-way ligature-feature split (prose
//! liga-only, code calt-on, discretionary-off) and the mono/Monaspace
//! uniform-pitch + per-char hit-test guards inside a ligature cluster -- split
//! out of the former monolithic `render::tests` (2026-07 code-organization
//! pass). See `syntax_roles` for the Alabaster role-color law tests.

use super::super::*;
use super::{headless_pipeline, view, view_md};

/// PER-WORLD CODE MONO: a CODE buffer (`syn_lang == Some`) shapes in the world's
/// monospace companion (`Theme::mono`) even on a SERIF world, so its columns have
/// a uniform fixed pitch — while a PROSE buffer in the SAME world keeps the
/// proportional display face (i and m differ). Gumtree is a Literata (serif)
/// world whose `mono` is Monaspace Xenon, so it exercises the mono/prose split.
#[test]
fn code_buffer_shapes_in_world_mono_while_prose_stays_display() {
    // Pitch reads fold the theme font AND the page wrap globals; hold both
    // (theme → page order, page.rs:95-99).
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping code_buffer_shapes_in_world_mono...: no wgpu adapter");
        return;
    };
    let pitch = |xs: &[f32]| -> f32 { xs[1] - xs[0] };
    let uniform = |xs: &[f32]| -> bool {
        let p0 = xs[1] - xs[0];
        xs.windows(2).all(|w| (w[1] - w[0] - p0).abs() < 0.5)
    };

    // SERIF world whose code face is a mono (Gumtree: Literata display / Monaspace
    // Xenon mono).
    theme::set_active_by_name("Gumtree").unwrap();
    p.sync_theme();
    assert_eq!(theme::active().font, "Literata");
    assert_eq!(theme::active().mono, "Monaspace Xenon");

    // A CODE buffer: mark it as Rust so the mono face is selected.
    let mut code = view("iiiiiiiiii", 0, 0);
    code.syn_lang = Some(crate::syntax::Lang::Rust);
    p.set_view(&code);
    let xs_i = p.line_glyph_xs(0);
    let mut code_m = view("mmmmmmmmmm", 0, 0);
    code_m.syn_lang = Some(crate::syntax::Lang::Rust);
    p.set_view(&code_m);
    let xs_m = p.line_glyph_xs(0);
    let (pi, pm) = (pitch(&xs_i), pitch(&xs_m));
    assert!(
        uniform(&xs_i) && uniform(&xs_m),
        "a code buffer must shape monospace (uniform pitch) even on a serif world (i={pi}, m={pm})"
    );
    assert!(
        (pi - pm).abs() < 0.5,
        "code buffer must shape i and m at the SAME mono pitch (i={pi}, m={pm})"
    );

    // A PROSE buffer (no syn_lang, not markdown) in the SAME world keeps the
    // proportional serif face: i and m differ.
    p.set_view(&view("iiiiiiiiii", 0, 0));
    let pi2 = pitch(&p.line_glyph_xs(0));
    p.set_view(&view("mmmmmmmmmm", 0, 0));
    let pm2 = pitch(&p.line_glyph_xs(0));
    assert!(
        (pi2 - pm2).abs() > 1.0,
        "prose in a serif world must stay proportional (i={pi2}, m={pm2})"
    );

    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// THE PURE FONT-FEATURE OWNER (`text::font_features`) returns the right
/// three-way ligature split per (is_code, face, code_ligatures) — a pure fn,
/// no GPU / locks. Covers: prose → standard on + discretionary off (NOT gated
/// by the toggle); a pitch-safe code mono (JBM/Iosevka) → programming
/// ligatures via `calt`; an unsafe/inert mono (Monaspace/IBM Plex) → the
/// ligature-free set (`calt`+`rclt`+`ccmp` off); the toggle OFF → the
/// ligature-free set for a safe mono too; and DISCRETIONARY off in EVERY case.
#[test]
fn font_features_owner_is_the_three_way_ligature_split() {
    use glyphon::cosmic_text::{FeatureTag, FontFeatures};
    let ff = |is_code: bool, face: &str, code_ligs: bool| -> FontFeatures {
        super::text::font_features(is_code, face, code_ligs)
    };
    // Last-set value of a tag (None = untouched → the font default applies).
    let val = |f: &FontFeatures, tag: FeatureTag| -> Option<u32> {
        f.features.iter().rev().find(|x| x.tag == tag).map(|x| x.value)
    };
    let liga = FeatureTag::STANDARD_LIGATURES;
    let clig = FeatureTag::CONTEXTUAL_LIGATURES;
    let calt = FeatureTag::CONTEXTUAL_ALTERNATES;
    let dlig = FeatureTag::DISCRETIONARY_LIGATURES;
    let rclt = FeatureTag::new(b"rclt");
    let ccmp = FeatureTag::new(b"ccmp");

    // PROSE (proportional display face): standard + contextual ON, discretionary
    // OFF — and NOT gated by the code_ligatures toggle (both toggle states equal).
    // `calt` is explicitly OFF too, on EVERY face (incl. a mono display face) —
    // the prose-ligature-leak fix: `calt` is Monaspace's programming-ligature
    // engine, and prose must never inherit a font's own default `calt` state.
    for code_ligs in [true, false] {
        for face in ["Literata", "Monaspace Xenon", "JetBrains Mono"] {
            let f = ff(false, face, code_ligs);
            assert_eq!(val(&f, liga), Some(1), "{face}: prose standard ligatures ON");
            assert_eq!(val(&f, clig), Some(1), "{face}: prose contextual ligatures ON");
            assert_eq!(val(&f, dlig), Some(0), "{face}: prose discretionary OFF");
            assert_eq!(val(&f, calt), Some(0), "{face}: prose calt OFF (no ligature leak)");
        }
    }

    // CODE on a PITCH-SAFE mono, toggle ON: programming ligatures via calt;
    // standard/contextual OFF; discretionary OFF.
    for face in ["JetBrains Mono", "Iosevka"] {
        let f = ff(true, face, true);
        assert_eq!(val(&f, calt), Some(1), "{face}: programming ligatures via calt ON");
        assert_eq!(val(&f, liga), Some(0), "{face}: standard OFF");
        assert_eq!(val(&f, clig), Some(0), "{face}: contextual-lig OFF");
        assert_eq!(val(&f, dlig), Some(0), "{face}: discretionary OFF");
    }

    // CODE on the SAME safe mono, toggle OFF: the ligature-free code set — calt
    // OFF (no programming ligatures), rclt+ccmp OFF. This is the "back to the
    // current no-ligature code behaviour" branch.
    let f = ff(true, "JetBrains Mono", false);
    assert_eq!(val(&f, calt), Some(0), "toggle off: calt OFF (no code ligatures)");
    assert_eq!(val(&f, rclt), Some(0), "toggle off: rclt OFF");
    assert_eq!(val(&f, ccmp), Some(0), "toggle off: ccmp OFF");
    assert_eq!(val(&f, dlig), Some(0), "toggle off: discretionary OFF");

    // CODE on an UNSAFE mono (Monaspace), toggle ON: STILL ligature-free — its
    // rclt+ccmp texture-healing must be disabled to keep uniform pitch (no safe
    // ligature option). Same for the INERT IBM Plex Mono.
    for face in ["Monaspace Xenon", "IBM Plex Mono"] {
        let f = ff(true, face, true);
        assert_eq!(val(&f, calt), Some(0), "{face}: calt OFF (unsafe/inert)");
        assert_eq!(val(&f, rclt), Some(0), "{face}: rclt OFF (stop cluster merge)");
        assert_eq!(val(&f, ccmp), Some(0), "{face}: ccmp OFF (stop cluster merge)");
        assert_eq!(val(&f, dlig), Some(0), "{face}: discretionary OFF");
    }

    // PROSE LIGATURE LEAK regression: no face should NOT restore calt — even
    // an unclassified/unknown display face stays explicitly OFF in prose,
    // since the prose branch returns before `mono_is_pitch_safe` is ever
    // consulted (calt has no legitimate prose role, safe mono or not).
    let f = ff(false, "Some Future Mono", true);
    assert_eq!(val(&f, calt), Some(0), "prose on an unknown face: calt still OFF");

    // An UNKNOWN mono defaults to the conservative ligature-free set.
    let f = ff(true, "Some Future Mono", true);
    assert_eq!(val(&f, calt), Some(0), "unknown mono: conservative ligature-free");
    assert_eq!(val(&f, rclt), Some(0), "unknown mono: rclt OFF");

    // The per-mono safety classifier itself: only the measured-safe monos.
    assert!(super::text::mono_is_pitch_safe("JetBrains Mono"));
    assert!(super::text::mono_is_pitch_safe("Iosevka"));
    assert!(!super::text::mono_is_pitch_safe("Monaspace Xenon"));
    assert!(!super::text::mono_is_pitch_safe("IBM Plex Mono"));
    assert!(!super::text::mono_is_pitch_safe("Some Future Mono"));
}

/// THE REPORTED PROSE LIGATURE LEAK, shaped for REAL (not the pure
/// `font_features` unit above): a markdown line `==x!!==` on Mangrove
/// (JetBrains Mono — a PITCH-SAFE mono display world, per
/// `mono_is_pitch_safe`, whose programming ligatures ride `calt` while
/// keeping exactly 1 glyph per source char — the exact mechanism the
/// leak rides, and empirically confirmed live on this bundled face: with
/// `calt` forced back on, the trailing `!` right before the highlight's
/// closing `==` picks up a DIFFERENT contextual glyph purely because it
/// sits next to `=`, even though `!!` is unrelated prose content, never a
/// code construct — the reported `==foo!!==` → `==foo≠=`-reading fusion).
/// Before this round's fix, the prose branch of `font_features` never
/// touched `calt` at all, so it inherited the font's own (on) default.
#[test]
fn prose_calt_off_keeps_highlight_delimiters_as_separate_glyphs() {
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!(
            "skipping prose_calt_off_keeps_highlight_delimiters_as_separate_glyphs: no wgpu adapter"
        );
        return;
    };
    theme::set_active_by_name("Mangrove").unwrap();
    assert_eq!(theme::active().font, "JetBrains Mono");
    p.sync_theme();
    let line_text = "==x!!==";

    // THE REAL PRODUCTION PATH: a markdown (prose) buffer's own doc_attrs
    // — `calt` OFF, this round's fix.
    p.set_view(&view_md(line_text, 0, 0));
    let glyph_at = |p: &TextPipeline, byte: usize| -> u16 {
        p.buffer
            .layout_runs()
            .find(|r| r.line_i == 0)
            .and_then(|r| r.glyphs.iter().find(|g| g.start == byte))
            .map(|g| g.glyph_id)
            .expect("a glyph must start at this byte")
    };
    // Byte 4 is the SECOND `!` of `!!` — the char immediately before the
    // trailing `==` delimiter, i.e. the exact `!`+`=` adjacency reported.
    let prose_bang = glyph_at(&p, 4);

    // THE COUNTERFACTUAL, shaped directly (not through `font_features`):
    // the SAME text + face with `calt` forcibly RE-ENABLED, proving the
    // mechanism is real on this bundled font, independent of this test's
    // own assertions about the fix.
    let mut ff_calt_on = glyphon::cosmic_text::FontFeatures::new();
    ff_calt_on.disable(glyphon::cosmic_text::FeatureTag::DISCRETIONARY_LIGATURES);
    ff_calt_on.enable(glyphon::cosmic_text::FeatureTag::STANDARD_LIGATURES);
    ff_calt_on.enable(glyphon::cosmic_text::FeatureTag::CONTEXTUAL_LIGATURES);
    ff_calt_on.enable(glyphon::cosmic_text::FeatureTag::CONTEXTUAL_ALTERNATES);
    let attrs = Attrs::new()
        .family(Family::Name("JetBrains Mono"))
        .weight(mono_safe_weight("JetBrains Mono"))
        .font_features(ff_calt_on);
    p.buffer.set_text(&mut p.font_system, line_text, &attrs, Shaping::Advanced, None);
    let calt_on_bang = glyph_at(&p, 4);

    assert_ne!(
        prose_bang, calt_on_bang,
        "sanity: JetBrains Mono's `calt` must actually change this glyph, or \
         this test can't discriminate the fix (both gid={prose_bang})"
    );

    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// The per-mono probe's exact ligature-dense content — arrows, comparisons,
/// path-sep, pipe. All ASCII, so on a TRUE mono every cell is one pitch; a
/// cluster merge (a ligature spanning >1 source char as one glyph) makes the
/// per-char `line_glyph_xs` non-uniform on these operators.
const LIG_CONTENT: &str = "-> => != >= <= == :: |>";

/// Uniform-pitch predicate over a `line_glyph_xs`: every consecutive delta
/// equals the first (within 0.5px).
fn xs_uniform(xs: &[f32]) -> bool {
    assert!(xs.len() >= 3, "need a few glyphs to measure pitch");
    let p0 = xs[1] - xs[0];
    xs.windows(2).all(|w| (w[1] - w[0] - p0).abs() < 0.5)
}

/// THE CODE-LIGATURE PITCH GUARD (the critical regression the three-way split
/// must not break, and the exact gap the probe flagged): with the code-ligature
/// features applied, every FONT-FEATURE-CONTROLLABLE mono (JetBrains Mono,
/// Iosevka, IBM Plex Mono) STILL shapes real programming-ligature content
/// (`-> => != >= <= == :: |>`) at STRICT uniform pitch — the per-char
/// `line_glyph_xs` stay evenly spaced, so caret/hit-test/selection column math
/// is honest. `font_features` keeps this uniform (calt for JBM/Iosevka's
/// GSUB programming ligatures, 1 glyph per source char; ligature-free for the
/// inert IBM Plex Mono).
///
/// Monaspace Xenon is EXCLUDED here and covered by the characterization test
/// below: its ligatures are AAT/`morx`-driven and CANNOT be suppressed via
/// OpenType feature tags in this shaper (cosmic-text 0.18.2 / harfrust 0.5.2 —
/// `rclt` isn't even in harfrust's AAT feature-mapping table), so it remains
/// non-uniform on operator sequences. The pre-existing `mono_world_shapes_
/// uniform_pitch` only shaped `iiii`/`mmmm` (no ligature triggers), so it
/// MISSED this entirely — this pair of tests pins both the fixed monos and the
/// known-unfixed one.
#[test]
fn code_ligature_content_stays_uniform_pitch_on_feature_controllable_monos() {
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!(
            "skipping code_ligature_content_stays_uniform_pitch_on_feature_controllable_monos: no wgpu adapter"
        );
        return;
    };
    let mut covered = std::collections::HashSet::new();
    for t in theme::THEMES.iter() {
        // Monaspace's AAT ligatures resist OT-feature suppression (see below).
        if t.mono == "Monaspace Xenon" {
            continue;
        }
        theme::set_active_by_name(t.name).unwrap();
        p.sync_theme();
        let mut code = view(LIG_CONTENT, 0, 0);
        code.syn_lang = Some(crate::syntax::Lang::Rust);
        p.set_view(&code);
        let xs = p.line_glyph_xs(0);
        assert!(
            xs_uniform(&xs),
            "{} (mono {}): code ligatures must keep uniform pitch on `{}` — xs={:?}",
            t.name,
            t.mono,
            LIG_CONTENT,
            xs
        );
        covered.insert(t.mono);
    }
    // Sanity: the three controllable monos were actually exercised (a mis-rename
    // of a mono face would otherwise silently shrink this guard to nothing).
    for m in ["JetBrains Mono", "Iosevka", "IBM Plex Mono"] {
        assert!(covered.contains(m), "expected a world with mono {m} to be tested");
    }
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// THE MONASPACE CLUSTER-FIX REGRESSION GUARD (flipped from the old
/// characterization test — see its history below). Monaspace Xenon's
/// programming ligatures are AAT/`morx`-driven "texture-healing": `-> => !=
/// :: …` shape to one glyph PER source char but all carry the SAME cluster
/// span, and they CANNOT be suppressed via OpenType feature tags in this
/// shaper (cosmic-text 0.18.2 / harfrust 0.5.2 — `rclt` isn't even in
/// harfrust's AAT feature table). The font-feature path could never make
/// these uniform; the DEEPER fix did — `assemble_glyph_xs` now groups the
/// glyphs sharing a span and spreads the source chars EVENLY over the
/// group's combined advance, so the per-char `line_glyph_xs` are uniform
/// again and the caret / selection / hit-test column math on a Monaspace
/// code line is honest. Shapes BOTH the mixed letters-and-operators content
/// the round named AND the pure-operator `LIG_CONTENT` the guard above uses,
/// asserting strict uniform pitch (maxdev < 0.5px) on each.
///
/// (History: this test used to assert the OPPOSITE — that Monaspace stayed
/// non-uniform, a documented AAT limitation — with a note that its assertion
/// should flip the day the `assemble_glyph_xs` cluster fix landed. It has.)
#[test]
fn monaspace_ligatures_shape_uniform_pitch_after_the_cluster_fix() {
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!(
            "skipping monaspace_ligatures_shape_uniform_pitch_after_the_cluster_fix: no wgpu adapter"
        );
        return;
    };
    // Potoroo's mono is Monaspace Xenon (asserted, so a reassignment surfaces here).
    theme::set_active_by_name("Potoroo").unwrap();
    assert_eq!(theme::active().mono, "Monaspace Xenon");
    p.sync_theme();
    // Mixed letters + texture-healed operators (the round's named fixture) AND
    // the pure-operator sequence — both must land on a strict uniform grid.
    for content in ["a => b != c :: d", LIG_CONTENT] {
        let mut code = view(content, 0, 0);
        code.syn_lang = Some(crate::syntax::Lang::Rust);
        p.set_view(&code);
        let xs = p.line_glyph_xs(0);
        assert!(
            xs_uniform(&xs),
            "Monaspace texture-healed ligatures must now shape UNIFORM pitch on \
             `{content}` (the assemble_glyph_xs cluster fix) — xs={xs:?}"
        );
    }
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// CARET / SELECTION / HIT-TEST INSIDE A PROGRAMMING-LIGATURE CLUSTER (the
/// subtle-bug zone the ligature-policy round had to clear): with code
/// ligatures ON, a pitch-safe mono (JetBrains Mono / Iosevka) substitutes the
/// `=>` / `!=` glyph SHAPES via `calt` while keeping 1 glyph per source char,
/// so the per-char column model stays exact even though the on-screen glyph
/// reads as one arrow. This drives the REAL pipeline (not the pure seam) and
/// asserts all three consumers of `line_glyph_xs` agree per-char:
///   * CARET: the caret x BETWEEN the two ligature chars (col of `>`) sits one
///     full pitch past the `=` — i.e. `col_x_and_advance` gives an exact
///     per-char boundary, never the whole-cluster width.
///   * SELECTION: a per-column advance across the cluster equals one pitch each
///     (a selection of just `=` covers exactly one cell, not the whole `=>`).
///   * HIT-TEST: a click in the first quarter of a char's cell resolves to that
///     char, the last quarter to the next — round-tripping every column,
///     including the two chars fused into the arrow glyph.
#[test]
fn caret_and_hit_test_are_per_char_inside_a_programming_ligature_cluster() {
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let saved_lig = crate::render::code_ligatures_on();
    crate::render::set_code_ligatures_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!(
            "skipping caret_and_hit_test_are_per_char_inside_a_programming_ligature_cluster: no wgpu adapter"
        );
        crate::render::set_code_ligatures_on(saved_lig);
        return;
    };
    // `a => b != c` — the ligature clusters `=>` (cols 2-3) and `!=` (cols 7-8)
    // sit mid-line with plain chars on either side, so a per-char boundary is
    // measurable against its neighbours.
    let content = "a => b != c";
    // The world PAIRS: Currawong=Iosevka, Mangrove=JetBrains Mono.
    for world in ["Currawong", "Mangrove"] {
        theme::set_active_by_name(world).unwrap();
        assert!(
            super::text::mono_is_pitch_safe(theme::active().mono),
            "{world}: expected a pitch-safe programming-ligature mono (mono={})",
            theme::active().mono
        );
        p.sync_theme();
        let mut code = view(content, 0, 0);
        code.syn_lang = Some(crate::syntax::Lang::Rust);
        p.set_view(&code);

        let xs = p.line_glyph_xs(0);
        let n = content.chars().count();
        assert_eq!(xs.len(), n + 1, "{world}: one x boundary per char + end");
        let pitch = xs[1] - xs[0];

        // CARET: every column boundary is an EXACT per-char multiple of the
        // pitch — the `>` of `=>` lands one pitch past the `=`, never fused.
        for c in 0..=n {
            let (x, _adv) = p.col_x_and_advance(0, c);
            let expect = c as f32 * pitch;
            assert!(
                (x - expect).abs() < 0.5,
                "{world}: caret x at col {c} must be per-char ({expect}), got {x} (xs={xs:?})"
            );
        }
        // SELECTION: the advance of each interior column is one pitch — a
        // one-char selection over the `=` (col 2) or `!` (col 7) is one cell.
        for c in [2usize, 3, 7, 8] {
            let (_x, adv) = p.col_x_and_advance(0, c);
            assert!(
                (adv - pitch).abs() < 0.5,
                "{world}: col {c} advance must be one pitch ({pitch}), got {adv}"
            );
        }
        // HIT-TEST: a click in the first quarter of each char's cell resolves
        // to that char; the last quarter to the next gap — round-trips every
        // column, including the two chars fused into an arrow glyph.
        let text_left = p.text_left();
        let py = p.doc_top() + p.metrics.line_height * 0.5;
        for c in 0..n {
            let cell = xs[c + 1] - xs[c];
            let (_l, col_lo) = p.hit_test(text_left + xs[c] + cell * 0.25, py, 0);
            assert_eq!(col_lo, c, "{world}: click in the near quarter of col {c} → col {c}");
            let (_l, col_hi) = p.hit_test(text_left + xs[c] + cell * 0.75, py, 0);
            assert_eq!(col_hi, c + 1, "{world}: click in the far quarter of col {c} → col {}", c + 1);
        }
    }
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
    crate::render::set_code_ligatures_on(saved_lig);
}
