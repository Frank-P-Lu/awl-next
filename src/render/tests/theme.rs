//! Theme-switch font reshape (incl. the debounced preview split), code-mono
//! switching, span-color rebaking, and the bundled bold/ornament/text face
//! registration laws -- split out of the former monolithic `render::tests`
//! (2026-07 code-organization pass). See `cjk` for the per-script font
//! resolution ladder.

use super::super::*;
use super::{headless_pipeline, view, view_md};

#[test]
fn theme_font_switch_reshapes_document() {
    // The caret-x reads below fold BOTH globals: the theme font (the shaped
    // advances) AND the page state (`column_width()` folds `page_on()` /
    // `measure()` — geometry.rs — into the wrap width + text_left every x is
    // measured from). Other tests flip the page globals under page::test_lock()
    // (measure 15/40/50…), so reading them here with only the theme lock raced
    // a parallel page write — the historical parallel-run flake of this very
    // test. Hold both, in the suite-wide theme → page order (see page::test_lock()'s doc).
    // The caret x is also ANCHOR-keyed (Morph shifts one cell back, and with no
    // override the mode DEFAULTS off the active theme's font — proportional
    // Gumtree would flip it to Morph mid-test); hold the caret lock and pin
    // BLOCK so the x reads stay on the cursor cell across the world switches.
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let _c = crate::testlock::serial();
    crate::caret::set_mode(CaretMode::Block);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping theme_font_switch_reshapes_document: no wgpu adapter");
        return;
    };
    // Start on a MONO world (IBM Plex Mono) so the caret x is on a fixed cell.
    theme::set_active_by_name("Tawny").unwrap();
    p.sync_theme();
    let text = "The quick brown fox";
    // Place the caret 10 chars in (on the 'b' of "brown").
    p.set_view(&view(text, 0, 10));
    let mono_x = p.caret_target_xy().0;
    let reshapes_before = p.reshape_count;

    // Switch to a PROPORTIONAL serif world (Literata). sync_theme must reshape
    // the document in the new family (text + zoom unchanged) so the glyph shapes
    // — and the real advances — change.
    theme::set_active_by_name("Gumtree").unwrap();
    p.sync_theme();
    assert!(
        p.reshape_count > reshapes_before,
        "a theme font switch must reshape the document"
    );
    // The caret x is derived from the REAL shaped advances; on a proportional
    // face the cumulative advance to col 10 differs from the mono cell grid, so
    // the caret tracked the new advances rather than staying on the mono cell.
    let serif_x = p.caret_target_xy().0;
    assert!(
        (serif_x - mono_x).abs() > 1.0,
        "caret x must follow the proportional advances after a font switch \
         (mono={mono_x}, serif={serif_x})"
    );

    // A switch that leaves the SHAPED face unchanged must NOT reshape: the
    // document is already shaped in that family. With the taste-review face
    // swaps every world now names a UNIQUE display face, so the former
    // distinct-world-same-font pair (Quokka + Kingfisher, both IBM Plex Sans)
    // no longer exists; the realizable instance is a redundant switch to the
    // same world — sync_theme keys the reshape on the shaped face, not the call.
    theme::set_active_by_name("Quokka").unwrap();
    p.sync_theme();
    let n = p.reshape_count;
    theme::set_active_by_name("Quokka").unwrap(); // same world, already shaped
    p.sync_theme();
    assert_eq!(
        p.reshape_count, n,
        "a switch that leaves the shaped face unchanged must NOT reshape"
    );

    // Restore the default world so other tests see a clean global.
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// THE PREVIEW DEBOUNCE SPLIT (`sync_theme` = `sync_theme_colors` +
/// `sync_theme_font`): the live theme-picker preview re-colors instantly per
/// arrow and DEFERS the font reshape until the selection settles, so an arrow
/// burst must cost ZERO reshapes until the one deferred `sync_theme_font` —
/// which must land the IDENTICAL shaped state the synchronous `sync_theme`
/// produces (the settled frame is byte-identical; the debounce only re-orders
/// WHEN the reshape happens, never what it shapes). And the Esc-revert path
/// (`retint_theme_now` = a full `sync_theme` on the restored world) must leave
/// NOTHING for a stray deferred fire to do — a late `sync_theme_font` after
/// the revert is a strict no-op.
#[test]
fn theme_preview_color_split_defers_reshape_and_revert_leaves_none() {
    // Shaping folds the theme font AND the page wrap globals; hold both locks
    // (theme → page order, page.rs:95-99). The caret-x equality below is also
    // ANCHOR-keyed (with no override the mode defaults off the active theme's
    // font — proportional Quokka would latch Morph and shift the x one cell);
    // hold the caret lock and pin BLOCK so both pipelines anchor identically.
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let _c = crate::testlock::serial();
    crate::caret::set_mode(CaretMode::Block);
    let Some(mut p) = headless_pipeline() else {
        eprintln!(
            "skipping theme_preview_color_split_defers_reshape_and_revert_leaves_none: no wgpu adapter"
        );
        return;
    };
    let text = "The quick brown fox";

    // Open on a MONO world; the doc shapes in IBM Plex Mono.
    theme::set_active_by_name("Tawny").unwrap();
    p.sync_theme();
    p.set_view(&view(text, 0, 10));
    let n = p.reshape_count;

    // ARROW BURST (the live preview path): colors only, per hop. No hop may
    // reshape; the doc stays shaped in the opening face while the pending
    // font change is visible via `needs_theme_reshape`.
    for world in ["Gumtree", "Bilby", "Saltpan", "Quokka"] {
        theme::set_active_by_name(world).unwrap();
        p.sync_theme_colors();
    }
    assert_eq!(
        p.reshape_count, n,
        "a color-only preview burst must not reshape the document"
    );
    assert_eq!(p.shaped_font, "IBM Plex Mono", "still shaped in the opening face");
    assert!(
        p.needs_theme_reshape(),
        "the deferred font change is pending (Quokka is Fira Sans)"
    );

    // SETTLE: the one deferred reshape lands. Exactly one reshape, and the
    // shaped state is identical to the synchronous `sync_theme` route.
    p.sync_theme_font();
    assert_eq!(p.reshape_count, n + 1, "the settle pays exactly ONE reshape");
    assert_eq!(p.shaped_font, "Fira Sans");
    let deferred_x = p.caret_target_xy().0;
    let Some(mut q) = headless_pipeline() else { return };
    q.sync_theme(); // synchronous full switch to the same (Quokka) world
    q.set_view(&view(text, 0, 10));
    assert_eq!(
        deferred_x,
        q.caret_target_xy().0,
        "the deferred reshape must land the same settled geometry as a synchronous sync_theme"
    );

    // ESC-REVERT with a pending deferral: previews colored ahead to Undertow,
    // then the revert applies the ORIGINAL world fully + synchronously (the
    // `retint_theme_now` path). The doc is already shaped in that face, so the
    // revert itself reshapes nothing — and a STRAY deferred fire afterwards
    // (the case the App cancels; harmless even if it raced through) no-ops.
    theme::set_active_by_name("Undertow").unwrap();
    p.sync_theme_colors();
    assert!(p.needs_theme_reshape(), "a deferral is pending toward EB Garamond");
    let m = p.reshape_count;
    theme::set_active_by_name("Quokka").unwrap(); // the world the picker opened on
    p.sync_theme(); // retint_theme_now: full, synchronous
    assert_eq!(p.reshape_count, m, "reverting to the shaped face reshapes nothing");
    p.sync_theme_font(); // the stray late fire
    assert_eq!(
        p.reshape_count, m,
        "a stray deferred reshape after the revert must be a strict no-op"
    );
    assert_eq!(p.shaped_font, "Fira Sans");

    // Restore the default world so other tests see a clean global.
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// PER-WORLD CODE MONO — `sync_theme` tracks the EFFECTIVE shaped face
/// (`doc_family` — the world's mono on a CODE buffer, else its display font;
/// render.rs), NOT the display font. On a code buffer a switch whose MONO
/// changes (Quokka → Kingfisher: IBM Plex Mono vs JetBrains Mono) MUST retrack
/// `shaped_font` to the new mono. The stronger, converse isolation: two worlds
/// with DIFFERENT display faces but the SAME mono (Kingfisher → Mangrove — IBM
/// Plex Sans vs JetBrains Mono display, both JetBrains Mono code) leave
/// `shaped_font` UNCHANGED (the effective face didn't move) even though the
/// display changed — proving the reshape/track gate keys on the MONO, not the
/// display; the world switch still reshapes to re-bake the per-span syntax
/// COLORS (`shaped_theme`, the same-face recolor path). (The taste-review face
/// swaps left every world with a UNIQUE display face, so the former
/// shared-display isolation — two worlds sharing ONE display sans — is no
/// longer expressible; the same-mono / different-display leg carries the gate
/// proof.) The PROSE reshape half is pinned by
/// `theme_font_switch_reshapes_document` next door; this is the code half.
#[test]
fn code_mono_switch_reshapes_effective_face() {
    // Shaping folds the theme font AND the page wrap globals; hold both locks
    // (theme → page order, page.rs:95-99) so a parallel mutator can't flip
    // either between the reshape-count reads.
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping code_mono_switch_reshapes_effective_face: no wgpu adapter");
        return;
    };

    // A CODE buffer on Quokka shapes in the world's mono companion.
    theme::set_active_by_name("Quokka").unwrap();
    p.sync_theme();
    assert_eq!(theme::active().font, "Fira Sans");
    assert_eq!(theme::active().mono, "IBM Plex Mono");
    let mut code = view("fn main() { let x = 1; }", 0, 0);
    code.syn_lang = Some(crate::syntax::Lang::Rust);
    p.set_view(&code);
    assert_eq!(
        p.shaped_font, "IBM Plex Mono",
        "a code buffer shapes in the world's mono, not its display sans"
    );
    let n = p.reshape_count;

    // Quokka → Kingfisher: the code MONO changes (IBM Plex Mono → JetBrains
    // Mono). The effective-face compare must see the mono change and reshape
    // the code buffer, retracking `shaped_font` to the new mono.
    theme::set_active_by_name("Kingfisher").unwrap();
    p.sync_theme();
    assert_eq!(theme::active().font, "IBM Plex Sans");
    assert_eq!(theme::active().mono, "JetBrains Mono");
    assert!(
        p.reshape_count > n,
        "a mono change must reshape a code buffer"
    );
    assert_eq!(
        p.shaped_font, "JetBrains Mono",
        "shaped_font tracks the NEW mono after the switch"
    );

    // Kingfisher → Mangrove: DIFFERENT display faces (IBM Plex Sans vs
    // JetBrains Mono) but the SAME code mono (both JetBrains Mono) — the
    // converse case. The code buffer is already shaped in the shared mono, so
    // the effective FACE is unchanged and `shaped_font` must NOT move even
    // though the display font did — proving the gate keys on the mono, not the
    // display. The WORLD (palette) DID change, so the switch still reshapes
    // once to re-bake the per-span syntax colors (`shaped_theme` — the
    // Magpie→Undertow stale-color fix), landing back on the same shared mono.
    let m = p.reshape_count;
    theme::set_active_by_name("Mangrove").unwrap();
    p.sync_theme();
    assert_ne!(
        theme::active().font,
        "IBM Plex Sans",
        "Mangrove's display face differs from Kingfisher's"
    );
    assert_eq!(theme::active().mono, "JetBrains Mono", "Mangrove shares Kingfisher's mono");
    assert!(
        p.reshape_count > m,
        "a world switch re-bakes span colors even when the code mono is shared"
    );
    assert_eq!(
        p.shaped_font, "JetBrains Mono",
        "the shared mono means the effective FACE is unchanged across the re-bake"
    );

    // Restore the default world so other tests see a clean global.
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// STALE SPAN-COLOR fix: per-span syntax/markdown/focus colors are BAKED into
/// the buffer `AttrsList` at shape time, so a theme switch that keeps the SAME
/// effective face (Magpie -> Undertow, both Monaspace Xenon, on a code buffer)
/// used to skip the re-bake and leave those spans colored for the OLD world's
/// derivation on the NEW ground. `sync_theme_font` now compares `shaped_theme`
/// alongside `shaped_font`, so a same-face palette change still restyles and the
/// baked color tracks the NEW world's `role_style_for`. Also pins the same-world
/// no-op guard (a redundant `sync_theme` must not restyle).
#[test]
fn theme_switch_rebakes_span_colors_across_shared_effective_face() {
    // Shaping folds the theme font AND the page wrap globals; hold both locks
    // (theme → page order, page.rs:95-99).
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!(
            "skipping theme_switch_rebakes_span_colors_across_shared_effective_face: no wgpu adapter"
        );
        return;
    };

    // Magpie (light) and Undertow (dark) BOTH shape code in Monaspace Xenon, so a
    // code buffer's EFFECTIVE face is identical across the switch — the font
    // tracker alone would skip the reshape. Their palettes differ sharply (light
    // vs dark ink ladder), so the baked syntax colors MUST change.
    theme::set_active_by_name("Magpie").unwrap();
    p.sync_theme();
    assert_eq!(theme::active().mono, "Monaspace Xenon");
    let text = "let x = 42;";
    let mut code = view(text, 0, 0);
    code.syn_lang = Some(crate::syntax::Lang::Rust);
    p.set_view(&code);

    // Find a byte whose span carries a baked syntax COLOR (a role fg tint); the
    // exact offset doesn't matter, only that the SAME byte is re-read after the
    // switch (same text + lexer -> same role at that byte, only the derivation
    // moves).
    let colored_byte = (0..text.len())
        .find(|&b| {
            p.buffer.lines[0].attrs_list().get_span(b).color_opt.is_some()
        })
        .expect("a rust code buffer bakes at least one colored syntax span");
    let magpie_color = p.buffer.lines[0].attrs_list().get_span(colored_byte).color_opt;
    assert!(magpie_color.is_some());
    let n = p.reshape_count;

    // Switch to a SAME-effective-face world (Undertow, also Monaspace Xenon).
    theme::set_active_by_name("Undertow").unwrap();
    assert_eq!(
        theme::active().mono,
        "Monaspace Xenon",
        "the two worlds share the code face, so the font tracker alone would skip"
    );
    p.sync_theme();
    assert!(
        p.reshape_count > n,
        "a same-face world switch must still restyle to re-bake the span colors"
    );
    assert_eq!(
        p.shaped_font, "Monaspace Xenon",
        "the effective face is unchanged across the color re-bake"
    );
    let undertow_color = p.buffer.lines[0].attrs_list().get_span(colored_byte).color_opt;
    assert!(undertow_color.is_some());
    assert_ne!(
        magpie_color, undertow_color,
        "the baked syntax color must reflect the NEW world's role_style_for, not the old"
    );

    // SAME-world, same-face: a redundant `sync_theme` is a strict no-op (the
    // `shaped_theme == active_index()` guard mirrors the `shaped_font` one).
    let m = p.reshape_count;
    p.sync_theme();
    assert_eq!(p.reshape_count, m, "re-syncing the SAME world must not restyle");
    assert_eq!(
        p.buffer.lines[0].attrs_list().get_span(colored_byte).color_opt,
        undertow_color,
        "an idempotent re-sync leaves the baked color untouched"
    );

    // Restore the default world so other tests see a clean global.
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// MONO FIX regression: the mono worlds (IBM Plex Mono) must shape in TRUE
/// monospace — a line of all-'i' and a line of all-'m' have the SAME, uniform
/// glyph pitch. The bug (a default Weight-400 request dropping the bundled
/// Light face and falling through to proportional `.SF NS`) made i ~5px / m
/// ~19px; the `mono_safe_weight(300)` fix realigns the request with the face.
/// Contrast a proportional world (Literata) where i and m differ by design.
#[test]
fn mono_world_shapes_uniform_pitch() {
    // Pitch reads fold the theme font AND the page wrap globals (a mid-test
    // measure write would re-wrap the lines); hold both (theme → page).
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping mono_world_shapes_uniform_pitch: no wgpu adapter");
        return;
    };
    // Advance between consecutive glyph xs (the per-column pitch). A line of N
    // identical chars yields N+1 xs (the last is the end-of-line caret slot).
    let pitch = |xs: &[f32]| -> f32 {
        assert!(xs.len() >= 3, "need a few glyphs to measure pitch");
        xs[1] - xs[0]
    };
    let uniform = |xs: &[f32]| -> bool {
        let p0 = xs[1] - xs[0];
        xs.windows(2).all(|w| (w[1] - w[0] - p0).abs() < 0.5)
    };

    // MONO world: i-pitch == m-pitch, and each line is internally uniform.
    theme::set_active_by_name("Tawny").unwrap();
    p.sync_theme();
    p.set_view(&view("iiiiiiiiii", 0, 0));
    let xs_i = p.line_glyph_xs(0);
    p.set_view(&view("mmmmmmmmmm", 0, 0));
    let xs_m = p.line_glyph_xs(0);
    let (pi, pm) = (pitch(&xs_i), pitch(&xs_m));
    assert!(
        uniform(&xs_i) && uniform(&xs_m),
        "mono world: each line must have uniform internal pitch (i={pi}, m={pm})"
    );
    assert!(
        (pi - pm).abs() < 0.5,
        "mono world must shape i and m at the SAME pitch (i={pi}, m={pm}); \
         a proportional fallback would give i<<m"
    );

    // PROPORTIONAL world (Literata): i and m have visibly different advances —
    // proves the test actually discriminates mono from proportional shaping.
    theme::set_active_by_name("Gumtree").unwrap();
    p.sync_theme();
    p.set_view(&view("iiiiiiiiii", 0, 0));
    let pi2 = pitch(&p.line_glyph_xs(0));
    p.set_view(&view("mmmmmmmmmm", 0, 0));
    let pm2 = pitch(&p.line_glyph_xs(0));
    assert!(
        (pi2 - pm2).abs() > 1.0,
        "proportional world should give i != m (i={pi2}, m={pm2})"
    );

    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// Every bundled display family ships a bold in `render::FONT_THEME_BOLD_FACES` —
/// the 10 proportional faces PLUS the 4 monospace display faces (the mono-bolds
/// round). A bold face only fixes the `weight_diff == 0` fallback trap if it
/// registers under the SAME family name its Regular uses AND declares usWeightClass
/// 700 — a subsetting/name-fixup mistake (the exact failure the CJK round guards
/// against for its faces) that renamed the face or left it at weight 400 would
/// silently keep the fallback bug. This asserts the font-DB fact directly for all
/// 14 (including Fira Sans / Bitter, registered but not yet assigned to any world,
/// so the resolution tests below can't reach them through a theme switch).
#[test]
fn bold_display_faces_register_under_their_family_names_at_weight_700() {
    let Some(p) = headless_pipeline() else {
        eprintln!("skipping bold_display_faces_register_under_their_family_names_at_weight_700: no wgpu adapter");
        return;
    };
    for expected in [
        "Literata",
        "Newsreader 16pt 16pt",
        "IBM Plex Sans",
        "Zilla Slab",
        "Figtree",
        "iA Writer Quattro S",
        "Fraunces 9pt",
        "EB Garamond",
        "Fira Sans",
        "Bitter",
        // Mono display faces (the mono-bolds round).
        "IBM Plex Mono",
        "JetBrains Mono",
        "Monaspace Xenon",
        "Iosevka",
    ] {
        let has_bold = p.font_system.db().faces().any(|f| {
            f.weight.0 == 700 && f.families.iter().any(|(n, _)| n == expected)
        });
        assert!(
            has_bold,
            "a weight-700 face must be registered under {expected:?} (the family its \
             Regular uses) — else a `**bold**` request trips the weight_diff==0 mono trap"
        );
    }
}

/// THE `**bold**` REGRESSION, resolved through the REAL font system: shaping bold
/// markdown on a world whose display face is one of the 10 bundled bolds must
/// resolve the bold content glyphs to a WEIGHT-700, NON-MONOSPACE face — never
/// cosmic-text's mono fallback (the shipping bug: with only the 400 Regular present,
/// a `Weight::BOLD` request drops the proportional face via `|400-700| == 300` and
/// lands in Menlo/Monaspace). Iterates every world whose `Theme::font` is a bundled
/// bold family and inspects the shaped `layout_runs`, mapping each bold-content
/// glyph's `font_id` back to its `FaceInfo`. `!monospaced` is the load-bearing
/// assertion — the mono fallback is the exact failure signature.
#[test]
fn markdown_bold_resolves_to_a_real_bold_face_never_the_mono_fallback() {
    let _t = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping markdown_bold_resolves_to_a_real_bold_face_never_the_mono_fallback: no wgpu adapter");
        return;
    };
    let bold_families = [
        "Literata",
        "Newsreader 16pt 16pt",
        "IBM Plex Sans",
        "Zilla Slab",
        "Figtree",
        "iA Writer Quattro S",
        "Fraunces 9pt",
        "EB Garamond",
    ];
    let mut checked = 0usize;
    for t in theme::THEMES.iter() {
        if !bold_families.contains(&t.font) {
            continue; // mono worlds stay Regular-only; unassigned faces covered above
        }
        theme::set_active_by_name(t.name).unwrap();
        p.sync_theme();
        // Bold on line 1 (line 0 blank), caret parked on line 0 — off the bold
        // line, so WYSIWYG conceal is inert; weight applies regardless. Content
        // "bold" is line-relative bytes 2..6 of "**bold**".
        p.set_view(&view_md("\n**bold**", 0, 0));
        let mut saw_glyph = false;
        for run in p.buffer.layout_runs() {
            if run.line_i != 1 {
                continue;
            }
            for g in run.glyphs.iter() {
                if g.start < 2 || g.start >= 6 {
                    continue; // only the "bold" content, not the `**` delimiters
                }
                let face = p
                    .font_system
                    .db()
                    .face(g.font_id)
                    .expect("shaped glyph maps to a registered face");
                assert_eq!(
                    face.families[0].0, t.font,
                    "{}: bold content glyph resolved to {:?}, not the world face {:?}",
                    t.name, face.families[0].0, t.font
                );
                assert_eq!(
                    face.weight.0, 700,
                    "{}: bold content glyph resolved to weight {}, not 700",
                    t.name, face.weight.0
                );
                assert!(
                    !face.monospaced,
                    "{}: bold content glyph fell to a MONOSPACE face — the weight_diff==0 mono-fallback bug",
                    t.name
                );
                saw_glyph = true;
            }
        }
        assert!(saw_glyph, "{}: found no bold content glyph to check", t.name);
        checked += 1;
    }
    assert!(checked >= 8, "expected to check all 8 assigned bold worlds, checked {checked}");
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// THE MONO-BOLDS REGRESSION (user report 2026-07-15, Firetail). The five
/// mono-display worlds — a world whose display face IS its own monospace companion
/// (`t.font == t.mono`): Tawny (IBM Plex Mono), Mangrove (JetBrains Mono),
/// Firetail + Potoroo (Monaspace Xenon), Currawong (Iosevka), and the one-bit
/// Wagtail (JetBrains Mono) as a bonus — used to shape `**bold**` in a FOREIGN
/// proportional sans (the "weird fi-ligature" bug): a `Weight::BOLD` request in a
/// Regular-only mono family tripped the `weight_diff == 0` fallback and dropped the
/// family entirely, landing in `.SF NS`. The mono-bolds round bundles a real 700
/// under each mono family. This is the OUTCOME sweep: an EXHAUSTIVE, no-skip pass
/// over every world whose display face resolves as monospaced (a data predicate —
/// a NEW mono world is automatically swept, no wildcard exclusion), asserting each
/// bold-content glyph resolves to (a) the world's OWN display family, never a
/// foreign face; (b) weight 700; (c) STILL MONOSPACED — the grid is kept, which is
/// the whole reason option (b) beat pinning to the Regular weight.
#[test]
fn markdown_bold_on_mono_worlds_keeps_the_grid_never_a_foreign_face() {
    let _t = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping markdown_bold_on_mono_worlds_keeps_the_grid_never_a_foreign_face: no wgpu adapter");
        return;
    };
    // A world is mono-display iff its display family has a registered monospaced
    // face — derived from the font DB, not a hand-list, so a new mono world can't
    // slip the sweep.
    let is_mono_family = |p: &crate::render::TextPipeline, family: &str| -> bool {
        p.font_system
            .db()
            .faces()
            .any(|f| f.monospaced && f.families.iter().any(|(n, _)| n == family))
    };
    let mut checked = 0usize;
    let mut names = Vec::new();
    for t in theme::THEMES.iter() {
        if !is_mono_family(&p, t.font) {
            continue; // proportional-display world — covered by the sweep above
        }
        theme::set_active_by_name(t.name).unwrap();
        p.sync_theme();
        p.set_view(&view_md("\n**bold**", 0, 0));
        let mut saw_glyph = false;
        for run in p.buffer.layout_runs() {
            if run.line_i != 1 {
                continue;
            }
            for g in run.glyphs.iter() {
                if g.start < 2 || g.start >= 6 {
                    continue; // only the "bold" content, not the `**` delimiters
                }
                let face = p
                    .font_system
                    .db()
                    .face(g.font_id)
                    .expect("shaped glyph maps to a registered face");
                assert_eq!(
                    face.families[0].0, t.font,
                    "{}: bold content glyph resolved to {:?}, not the world's own mono face {:?} \
                     (the foreign-sans fallback bug)",
                    t.name, face.families[0].0, t.font
                );
                assert_eq!(
                    face.weight.0, 700,
                    "{}: bold content glyph resolved to weight {}, not 700",
                    t.name, face.weight.0
                );
                assert!(
                    face.monospaced,
                    "{}: bold content glyph is NOT monospaced — the mono bold must keep the fixed grid",
                    t.name
                );
                saw_glyph = true;
            }
        }
        assert!(saw_glyph, "{}: found no bold content glyph to check", t.name);
        checked += 1;
        names.push(t.name);
    }
    assert!(
        checked >= 5,
        "expected at least the 5 mono-display worlds (Tawny/Mangrove/Firetail/Potoroo/Currawong), \
         checked {checked}: {names:?}"
    );
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// THE bold/italic-breaks-Japanese REGRESSION, resolved through the REAL font
/// system: shaping `**bold**` / `*italic*` / `***bold-italic***` Japanese must
/// resolve every CJK content glyph to the world's BUNDLED JP face at its
/// registered Weight 400 / Normal style — NEVER a heavier / slanted / mono /
/// system fallback. The failure signature the fix guards against: a markdown
/// emphasis span sets `Weight(700)` / `Style::Italic`, and without the
/// script-span layer's weight+style PIN (see `spans::add_script_spans`) that
/// request drops the 400/Normal-only bundled face (`weight_diff != 0` +
/// style-mismatch) and tofu/system-falls mid-sentence. Checks a serif world
/// (Undertow → Shippori Mincho, its Phase-2 ja override) and a sans world
/// (Currawong → Noto Sans JP) — `want_fam` is read dynamically from the
/// resolver, so it tracks each world's assigned face rather than a literal;
/// caret parked on the blank line 0, so the styled lines are OFF-cursor (their
/// `**`/`*` markers conceal — the emphasis weight/style still applies to the
/// content, which is exactly the run under test).
#[test]
fn markdown_emphasis_keeps_the_bundled_cjk_face_never_a_fallback() {
    let _t = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping markdown_emphasis_keeps_the_bundled_cjk_face_never_a_fallback: no wgpu adapter");
        return;
    };
    for world in ["Undertow", "Currawong"] {
        theme::set_active_by_name(world).unwrap();
        p.sync_theme();
        let (want_fam, _) = p
            .resolve_font_id(theme::FontId::Ja)
            .expect("Ja must resolve to a bundled face");
        // line 0 blank (caret here); line 1 bold, line 2 italic, line 3 bold-italic.
        let text = "\n**太字**\n*斜体*\n***両方***";
        p.set_view(&view_md(text, 0, 0));
        let lines: Vec<String> =
            p.buffer.lines.iter().map(|l| l.text().to_string()).collect();
        let mut checked = 0usize;
        for run in p.buffer.layout_runs() {
            if run.line_i == 0 {
                continue;
            }
            let lt = &lines[run.line_i];
            for g in run.glyphs.iter() {
                let ch = lt.get(g.start..g.end).unwrap_or("");
                if !ch.chars().next().map(super::spans::is_cjk).unwrap_or(false) {
                    continue; // skip the `**`/`*` delimiter glyphs, only CJK content
                }
                let face = p
                    .font_system
                    .db()
                    .face(g.font_id)
                    .expect("shaped glyph maps to a registered face");
                assert_eq!(
                    face.families[0].0, want_fam,
                    "{world}: emphasized CJK glyph {ch:?} resolved to {:?}, not the bundled JP face {want_fam:?}",
                    face.families[0].0
                );
                assert_eq!(
                    face.weight.0, 400,
                    "{world}: emphasized CJK glyph {ch:?} resolved to weight {} — the bold(700) leaked past the pin",
                    face.weight.0
                );
                assert!(
                    matches!(face.style, glyphon::cosmic_text::fontdb::Style::Normal),
                    "{world}: emphasized CJK glyph {ch:?} resolved to a slanted style {:?} — the italic leaked past the pin",
                    face.style
                );
                assert!(
                    !face.monospaced,
                    "{world}: emphasized CJK glyph {ch:?} fell to a MONOSPACE fallback",
                );
                checked += 1;
            }
        }
        assert!(
            checked >= 6,
            "{world}: expected the 6 emphasized CJK content glyphs, checked {checked}"
        );
    }
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// The bundled text + ornament faces (Fira Sans, Iosevka, Bitter, Junicode)
/// and the rebuilt symbol face (Awl Marks) must each resolve under their
/// expected registered family name — so they are addressable via `Family::Name`
/// (the section-break fleuron / About end-mark now names Junicode this way), and
/// a renamed/corrupted face fails here rather than surfacing as downstream tofu.
/// (Vollkorn-Ornaments was dropped — it shipped no classic fleurons, so no world
/// could use it for a section break.)
#[test]
fn bundled_text_and_ornament_faces_register_under_their_family_names() {
    let Some(p) = headless_pipeline() else {
        eprintln!("skipping bundled_text_and_ornament_faces_register_under_their_family_names: no wgpu adapter");
        return;
    };
    for expected in ["Fira Sans", "Iosevka", "Bitter", "Junicode", "Awl Marks"] {
        let registered = p
            .font_system
            .db()
            .faces()
            .any(|f| f.families.iter().any(|(n, _)| n == expected));
        assert!(registered, "{expected:?} must be registered in the font DB");
    }
}
