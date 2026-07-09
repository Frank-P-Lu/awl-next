//! Markdown styling gating, symbol/ornament faces, the `---`/`***`/`___`
//! thematic-break ornament, nested bullet reveal-on-cursor, and the
//! word-count/reading-time readout -- split out of the former monolithic
//! `render::tests` (2026-07 code-organization pass). See `markdown_headings`
//! for blockquote + heading-size tests.

use super::super::*;
use super::{headless_pipeline, view};

#[test]
fn markdown_styling_gated_and_composed() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping markdown_styling_gated_and_composed: no wgpu adapter");
        return;
    };
    let text = "# Title\n\nsome **bold** words\n";
    // NON-markdown buffer: NO md spans at all (byte-identical render).
    let mut plain = view(text, 0, 0);
    plain.is_markdown = false;
    p.set_view(&plain);
    assert!(
        p.md_report().is_empty(),
        "a non-markdown buffer must yield NO md spans"
    );
    // MARKDOWN buffer: the heading hashes dim to `markup`, the title is `h1`,
    // and the `**bold**` run yields a `bold` span with dim `**` markers.
    let mut md = view(text, 0, 0);
    md.is_markdown = true;
    p.set_view(&md);
    let spans = p.md_report();
    assert!(
        spans.iter().any(|(s, e, t)| *s == 0 && *e == 2 && *t == "markup"),
        "leading '# ' should be a markup span: {spans:?}"
    );
    assert!(
        spans.iter().any(|(s, e, t)| *s == 2 && *e == 7 && *t == "h1"),
        "title 'Title' should be an h1 span: {spans:?}"
    );
    // "some " starts at byte 9; "**bold**" → ** at 14..16, bold 16..20, ** 20..22.
    assert!(
        spans.iter().any(|(_, _, t)| *t == "bold"),
        "a **bold** run should yield a bold span: {spans:?}"
    );
    let bold = spans.iter().find(|(_, _, t)| *t == "bold").unwrap();
    assert!(
        spans
            .iter()
            .any(|(_s, e, t)| *t == "markup" && *e == bold.0),
        "the '**' before a bold run should be a markup span: {spans:?}"
    );
}

#[test]
fn symbol_runs_isolate_modifier_and_ornament_glyphs() {
    // The macOS modifier glyphs + the ornaments are SYMBOLS; ASCII / letters are
    // not, so a chord like "⌘⇧O" yields ONE run over the two leading glyphs and
    // leaves the "O" to the display face.
    assert!(is_symbol('\u{2318}') && is_symbol('\u{21E7}')); // ⌘ ⇧
    assert!(is_symbol('❧') && is_symbol('❦')); // the hr + end ornaments
    assert!(!is_symbol('O') && !is_symbol('-') && !is_symbol('A'));
    let s = "\u{2318}\u{21E7}O"; // ⌘⇧O
    let runs = symbol_runs(s);
    assert_eq!(runs.len(), 1, "the two modifier glyphs form one run: {runs:?}");
    assert_eq!(&s[runs[0].clone()], "\u{2318}\u{21E7}", "run covers ⌘⇧ only");
    // Mid-text section sign: an isolated symbol run between plain text.
    let t = "a \u{00A7}3 b"; // "a §3 b"
    let r2 = symbol_runs(t);
    assert_eq!(r2.len(), 1);
    assert_eq!(&t[r2[0].clone()], "\u{00A7}");
    // A symbol-free line yields no runs (so its render stays byte-identical).
    assert!(symbol_runs("plain ascii line").is_empty());
}

#[test]
fn symbol_face_registered_under_private_family() {
    let Some(p) = headless_pipeline() else {
        eprintln!("skipping symbol_face_registered_under_private_family: no wgpu adapter");
        return;
    };
    // The bundled subset registers under the private SYMBOL_FAMILY name (named
    // only via per-run family spans, never as a display face), so the modifier
    // glyphs + ornaments have a home face to resolve to instead of tofu.
    let registered = p
        .font_system
        .db()
        .faces()
        .any(|f| f.families.iter().any(|(n, _)| n == SYMBOL_FAMILY));
    assert!(registered, "the bundled symbol face must register under {SYMBOL_FAMILY:?}");
}

#[test]
fn horizontal_rule_ornament_gated_and_centered() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping horizontal_rule_ornament_gated_and_centered: no wgpu adapter");
        return;
    };
    // A `---` alone (blank lines around it) is a thematic break on line 2.
    let text = "intro\n\n---\n\nmore\n";

    // MARKDOWN: exactly one section-break ornament (the centered fleuron that
    // REPLACES the old thin rule line), placed on the `---` row; the sidecar
    // still tags the line `rule`.
    let mut md = view(text, 0, 0);
    md.is_markdown = true;
    p.set_view(&md);
    let tops = p.rule_tops();
    assert_eq!(tops.len(), 1, "one --- line => one rule ornament: {tops:?}");
    assert!(
        p.md_report().iter().any(|(_, _, t)| *t == "rule"),
        "the rule line should be tagged `rule` in the sidecar"
    );

    // NON-markdown: the SAME text yields NO rule ornament (gated like every md
    // effect); `prepare_ornaments` uploads no areas, so nothing draws.
    let mut plain = view(text, 0, 0);
    plain.is_markdown = false;
    p.set_view(&plain);
    assert!(
        p.rule_tops().is_empty(),
        "a non-markdown buffer must draw no rule ornaments"
    );
}

#[test]
fn horizontal_rule_conceals_dashes_until_the_caret_lands() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping horizontal_rule_conceals_dashes_until_the_caret_lands: no wgpu adapter");
        return;
    };
    // A `---` thematic break alone on logical line 2 (blank lines around it).
    let text = "intro\n\n---\n\nmore\n";

    // CARET OFF the hr (line 0): an hr is pure markup, so the raw `---` CONCEAL
    // (transparent ink) and the centered fleuron is the only mark — exactly one
    // rule ornament, on the `---` row. The sidecar still tags the line `rule`.
    let mut off = view(text, 0, 0);
    off.is_markdown = true;
    p.set_view(&off);
    assert_eq!(
        p.rule_tops().len(),
        1,
        "caret off the hr => the fleuron draws on the --- row: {:?}",
        p.rule_tops()
    );
    assert!(
        p.rule_line_concealed(2),
        "caret off the hr => the raw --- are concealed (transparent)"
    );
    assert!(
        p.md_report().iter().any(|(_, _, t)| *t == "rule"),
        "the rule line stays tagged `rule` in the sidecar even when concealed"
    );

    // CARET ON the hr line (line 2): the dashes REVEAL (visible, editable) and the
    // fleuron is SUPPRESSED so editing the rule is unobstructed.
    let mut on = view(text, 2, 0);
    on.is_markdown = true;
    p.set_view(&on);
    assert!(
        p.rule_tops().is_empty(),
        "caret on the hr => the fleuron yields to the revealed dashes: {:?}",
        p.rule_tops()
    );
    assert!(
        !p.rule_line_concealed(2),
        "caret on the hr => the raw --- reveal (not transparent)"
    );

    // Moving the caret back OFF re-conceals (the toggle is live, both directions).
    p.set_view(&off);
    assert!(p.rule_line_concealed(2), "caret leaves => --- re-conceal");
    assert_eq!(p.rule_tops().len(), 1, "caret leaves => the fleuron returns");
}

#[test]
fn thematic_break_ornament_tracks_the_syntax_per_line() {
    // This test WRITES the process-global active theme (the pin below); hold
    // the theme lock so it can't yank the world out from under a theme test.
    let _t = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping thematic_break_ornament_tracks_the_syntax_per_line: no wgpu adapter");
        return;
    };
    // Pin the default world (Tawny) so the ornament set is its own trio; read the
    // three glyphs from the world itself so this test tracks a future re-pick.
    theme::set_active(theme::DEFAULT_THEME);
    let orn = theme::active().ornaments;
    let (dash, star, under) = (orn.dash, orn.star, orn.underscore);
    // Three DISTINCT glyphs (the design-table contract) — otherwise the
    // reveal-on-cursor half below can't tell which mark dropped.
    assert!(dash != star && star != under && dash != under);
    // Three DIFFERENT break syntaxes, each alone on its own line (blank-separated):
    // line 2 = `---`, line 4 = `***`, line 6 = `___`.
    let text = "intro\n\n---\n\n***\n\n___\n\nmore\n";

    // CARET OFF every break (line 0): all three ornaments draw, each the glyph its
    // OWN syntax picked — dash / star / underscore in document order. This is the
    // whole feature: the mark tracks the type the author wrote.
    let mut off = view(text, 0, 0);
    off.is_markdown = true;
    p.set_view(&off);
    let marks: Vec<char> = p.rule_marks().into_iter().map(|(_, c)| c).collect();
    assert_eq!(
        marks,
        vec![dash, star, under],
        "--- ⁄ *** ⁄ ___ must pick the world's dash ⁄ star ⁄ underscore: {marks:?}"
    );

    // REVEAL-ON-CURSOR still holds PER LINE: put the caret on the `***` line (4).
    // Its ornament yields (the raw *** reveal for editing) while the OTHER two
    // breaks keep their distinct ornaments — dash and underscore, the star dropped.
    let mut on_star = view(text, 4, 0);
    on_star.is_markdown = true;
    p.set_view(&on_star);
    let revealed: Vec<char> = p.rule_marks().into_iter().map(|(_, c)| c).collect();
    assert_eq!(
        revealed,
        vec![dash, under],
        "caret on the *** line suppresses only its star; dash and underscore remain: {revealed:?}"
    );
}

#[test]
fn nested_bullets_cycle_by_depth_and_reveal_on_cursor() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping nested_bullets_cycle_by_depth_and_reveal_on_cursor: no wgpu adapter");
        return;
    };
    // Three nested bullets at depth 0/1/2 (0/2/4 leading spaces), typed with MIXED
    // markers (-, *, +) to prove the glyph is DEPTH-derived, not char-derived.
    let text = "- top\n  * mid\n    + deep\n";

    // Default world (Tawny) → the plain `•`/`◦` pair, cycling every TWO levels.
    // CARET OFF every list line (on the trailing blank line 3): each bullet draws
    // its depth glyph • ◦ • and its raw marker is concealed (transparent ink).
    let mut off = view(text, 3, 0);
    off.is_markdown = true;
    p.set_view(&off);
    assert_eq!(
        p.bullet_glyphs(),
        vec!['•', '◦', '•'],
        "depth 0/1/2 => • ◦ • (pair cycles) regardless of the -,*,+ typed: {:?}",
        p.bullet_glyphs()
    );
    for li in 0..3 {
        assert!(
            p.bullet_marker_concealed(li),
            "caret off => the raw marker on line {li} is concealed"
        );
    }

    // CARET ON the middle bullet (line 1): its raw `*` REVEALS (editable) and no
    // glyph draws for it; the other two keep their depth-0/2 glyph (both •).
    let mut on = view(text, 1, 3);
    on.is_markdown = true;
    p.set_view(&on);
    assert_eq!(
        p.bullet_glyphs(),
        vec!['•', '•'],
        "caret on the mid bullet suppresses only its ◦ (lines 0 and 2 keep •): {:?}",
        p.bullet_glyphs()
    );
    assert!(!p.bullet_marker_concealed(1), "caret on => the mid `*` reveals");
    assert!(
        p.bullet_marker_concealed(0) && p.bullet_marker_concealed(2),
        "the other bullets stay concealed"
    );

    // An ORDERED item keeps its number (no bullet glyph).
    let mut ord = view("1. one\n2. two\n", 2, 0);
    ord.is_markdown = true;
    p.set_view(&ord);
    assert!(p.bullet_glyphs().is_empty(), "ordered lists get no bullet glyph");

    // NON-markdown buffer: no bullets at all (a `.rs` file with `- x` is
    // byte-identical — the glyph is gated on `md_enabled`).
    let mut plain = view(text, 3, 0);
    plain.is_markdown = false;
    p.set_view(&plain);
    assert!(p.bullet_glyphs().is_empty(), "non-markdown => no bullet glyphs");
}

/// PER-WORLD BULLETS: the depth-derived glyph swaps to the ACTIVE world's own
/// [`theme::Theme::bullets`] pair (drawn in its ornament face) — a technical
/// world keeps `•`/`◦`, a literary serif draws its characterful pair, and
/// Undertow the manicule. Reveal-on-cursor is unchanged (off-caret only). Proves
/// the glyph is theme-DATA, not a fixed geometric triple.
#[test]
fn bullet_glyphs_swap_per_world() {
    // set_active_by_name mutates the theme global; bullet_marks folds page
    // geometry → hold theme then page (the documented theme→…→page order).
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping bullet_glyphs_swap_per_world: no wgpu adapter");
        return;
    };
    // Two nested bullets at depth 0/1, caret parked off both list lines (line 2).
    let text = "- top\n  - sub\n";
    let cases = [
        ("Tawny", ('•', '◦')),       // geometric world: plain, byte-identical
        ("Undertow", ('☞', '❧')),    // the manicule showpiece + hedera
        ("Gumtree", ('❧', '☙')),     // Junicode botanical hederas
        ("Bilby", ('❧', '❦')),       // Garamond Renaissance fleurons
        ("Mopoke", ('⁑', '❦')),      // the quiet utilitarian Junicode mark
    ];
    for (world, (g0, g1)) in cases {
        theme::set_active_by_name(world).unwrap();
        let mut off = view(text, 2, 0);
        off.is_markdown = true;
        p.set_view(&off);
        assert_eq!(
            p.bullet_glyphs(),
            vec![g0, g1],
            "{world}: depth 0/1 draws its per-world pair {:?}",
            (g0, g1)
        );
        // Reveal-on-cursor still holds: caret on the top bullet (line 0) drops
        // its glyph, leaving only the depth-1 glyph.
        let mut on = view(text, 0, 2);
        on.is_markdown = true;
        p.set_view(&on);
        assert_eq!(
            p.bullet_glyphs(),
            vec![g1],
            "{world}: caret on the top bullet reveals its raw marker (no glyph)"
        );
    }
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// NEVER-TOFU (per-world LIST BULLETS): both glyphs of every world's
/// [`theme::Theme::bullets`] pair resolve to a REAL glyph in that world's
/// [`theme::Theme::ornament_face`] — the font-DB half of the structural
/// `theme::tests::every_world_has_a_bullet_pair` law, mirroring
/// `ornament_glyphs_resolve_in_each_worlds_assigned_face` for the section trio.
/// This is what proves the manicule ☞ actually lives in EB Garamond and every
/// Junicode hedera in the bundled ornament subset.
#[test]
fn bullet_glyphs_resolve_in_each_worlds_assigned_face() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping bullet_glyphs_resolve_in_each_worlds_assigned_face: no wgpu adapter");
        return;
    };
    for t in theme::THEMES.iter() {
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
        for (level, ch) in [("level-1", t.bullets.0), ("level-2", t.bullets.1)] {
            assert!(
                charmap.map(ch) != 0,
                "{}: {} bullet {:?} (U+{:04X}) is NOT in its ornament face {:?} — tofu",
                t.name,
                level,
                ch,
                ch as u32,
                t.ornament_face
            );
        }
    }
}

/// PERF O(visible): `bullet_marks` places each visible bullet's glyph WITHOUT the
/// retired per-line O(li) `line_glyph_xs` walk (an O(doc) `layout_runs` walk from
/// doc start, per bullet — O(visible_bullets × scroll) each frame, breaking the
/// O(visible) law its sibling `rule_marks` honours by reading cached row geometry).
/// An UNINDENTED bullet needs no walk at all (its marker sits at column 0); an
/// INDENTED bullet resolves through the BATCHED, memo-safe `visual_rows_for_lines`,
/// never a per-line `visual_rows` (which would clobber the single-slot cursor-line
/// row memo). Placement stays byte-identical to the retired `line_glyph_xs`-based x.
/// Mirrors `range_rects_selection_is_visible_bounded_and_memo_safe`.
#[test]
fn bullet_marks_placement_unchanged_and_geometry_is_o_visible() {
    // Bullet x folds the page globals (writing-column left); hold the page lock so
    // a parallel page write can't move the column mid-test.
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping bullet_marks_placement_unchanged_and_geometry_is_o_visible: no wgpu adapter");
        return;
    };

    // PART A — PLACEMENT UNCHANGED. A small doc mixing an UNINDENTED bullet
    // (indent 0, my x==0 branch) and an INDENTED one (indent 2, the batched path),
    // caret on the trailing blank line so every bullet is placed. Each mark's x
    // must equal the retired `line_glyph_xs(li)[indent]`-based x, byte-for-byte.
    let mut small = view("- a\n  - b\n- c\n\n", 3, 0);
    small.is_markdown = true;
    p.set_view(&small);
    let text_left = p.text_left();
    let marks = p.bullet_marks(); // ascending line order: lines 0, 1, 2
    assert_eq!(marks.len(), 3, "all three bullets placed: {marks:?}");
    let expect = |li: usize, indent: usize| -> f32 {
        text_left + p.line_glyph_xs(li).get(indent).copied().unwrap_or(0.0)
    };
    for (mark, (li, indent)) in marks.iter().zip([(0, 0), (1, 2), (2, 0)]) {
        let want = expect(li, indent);
        assert!(
            (mark.1 - want).abs() < 0.01,
            "bullet x on line {li} (indent {indent}) changed: {} vs {want}",
            mark.1
        );
    }
    // Sanity: the indented bullet really sits right of the unindented ones (so the
    // batched path is exercised on a genuinely offset marker, not a vacuous 0).
    assert!(
        marks[1].1 > marks[0].1 + 0.5,
        "the indented bullet's marker must sit right of column 0: {marks:?}"
    );

    // PART B — O(visible) + memo-safe. A TALL doc (many bullets, every 3rd
    // INDENTED so the visible band always contains some) scrolled to the middle:
    // only the on-screen band's bullets are placed, and the batched resolve leaves
    // the warm cursor-line row memo intact.
    const N: usize = 400;
    let text: String = (0..N)
        .map(|i| if i % 3 == 0 { "  - x\n" } else { "- y\n" })
        .collect();
    let cursor_line = N / 2;
    let mut tall = view(&text, cursor_line, 0);
    tall.is_markdown = true;
    tall.scroll_lines = cursor_line - 5; // put the caret near the view top
    p.set_view(&tall);

    // WARM the single-slot cursor-line memo, then prove `bullet_marks` leaves it
    // intact — a per-line `visual_rows` walk (the wrong fix) would stomp it.
    let _ = p.visual_rows(cursor_line);
    assert!(
        p.row_geom.cached_rows(cursor_line).is_some(),
        "precondition: the cursor-line row memo is warm"
    );

    let tall_marks = p.bullet_marks();
    assert!(!tall_marks.is_empty(), "the visible bullets must be placed");
    assert!(
        tall_marks.len() < 100,
        "only the visible band's bullets, got {} of {N}",
        tall_marks.len()
    );
    // WITNESS THE WORK: an INDENTED bullet is in the visible band (some x sits
    // right of column 0), so `visual_rows_for_lines` genuinely ran — and it left
    // the cursor-line memo warm (the batched, memo-safe path, not per-line
    // `visual_rows`).
    assert!(
        tall_marks.iter().any(|m| m.1 > text_left + 0.5),
        "an indented bullet must be visible so the batched geometry path runs"
    );
    assert!(
        p.row_geom.cached_rows(cursor_line).is_some(),
        "bullet_marks must resolve indented bullets via the batched (memo-safe) path"
    );
}

#[test]
fn wordcount_readout_gated_to_markdown() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wordcount_readout_gated_to_markdown: no wgpu adapter");
        return;
    };
    let text = "one two three four five\n"; // 5 words

    // MARKDOWN: the readout reports the word count + a (rounded-up) reading time.
    let mut md = view(text, 0, 0);
    md.is_markdown = true;
    p.set_view(&md);
    assert_eq!(
        p.readout_report(),
        Some((5, 1)),
        "5 words => `5 words · 1 min`"
    );

    // NON-markdown: NO readout (gated, so a plain buffer stays byte-identical).
    let mut plain = view(text, 0, 0);
    plain.is_markdown = false;
    p.set_view(&plain);
    assert_eq!(p.readout_report(), None, "non-markdown => no readout");

    // An empty markdown buffer has nothing to read.
    let mut blank = view("", 0, 0);
    blank.is_markdown = true;
    p.set_view(&blank);
    assert_eq!(p.readout_report(), None, "a wordless buffer => no readout");
}

/// i18n: a leading frontmatter block is METADATA, not manuscript — its
/// `lang:`/etc. lines never inflate the word-count/reading-time readout.
#[test]
fn readout_excludes_frontmatter_block() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping readout_excludes_frontmatter_block: no wgpu adapter");
        return;
    };
    // Frontmatter contributes "lang ja" (2 words) which must NOT count;
    // only the 5-word body should.
    let text = "---\nlang: ja\n---\none two three four five\n";
    let mut md = view(text, 0, 0);
    md.is_markdown = true;
    p.set_view(&md);
    assert_eq!(
        p.readout_report(),
        Some((5, 1)),
        "the frontmatter's own words must not count toward the readout"
    );

    // A document that is FRONTMATTER ONLY (no body) reads as wordless.
    let fm_only = "---\nlang: ja\ntitle: x\n---\n";
    let mut md2 = view(fm_only, 0, 0);
    md2.is_markdown = true;
    p.set_view(&md2);
    assert_eq!(p.readout_report(), None, "a frontmatter-only doc has nothing to read");
}

#[test]
fn notice_parked_offscreen_when_empty() {
    // The CALM NOTICE mirrors the ViewState field and defaults EMPTY — the
    // empty string routes through the shared corner-label body's park-off-
    // screen arm (the same gate the wordcount/gutter byte-identity rides),
    // so every capture (which can never carry a notice — autosave is
    // live-only) draws nothing. A live notice lands in the mirror verbatim
    // and clears back to empty when the view drops it.
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping notice_parked_offscreen_when_empty: no wgpu adapter");
        return;
    };
    let v = view("hello\n", 0, 0);
    p.set_view(&v);
    assert!(p.notice.is_empty(), "default view carries no notice");
    let mut warned = view("hello\n", 0, 0);
    warned.notice = "changed on disk outside awl — autosave held".to_string();
    p.set_view(&warned);
    assert_eq!(
        p.notice, "changed on disk outside awl — autosave held",
        "a live notice mirrors into the pipeline"
    );
    p.set_view(&v);
    assert!(p.notice.is_empty(), "the notice clears when the view drops it");
}
