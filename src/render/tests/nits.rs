//! Writing-nit + spell-squiggle underline gating (mechanical typos, table
//! alignment exemption, caret-line/word suppression, frontmatter exclusion)
//! and their underline-cache rebuild triggers -- split out of the former
//! monolithic `render::tests` (2026-07 code-organization pass).

use super::super::*;
use super::{headless_pipeline, view};

/// WRITING NITS: the muted STRAIGHT underline geometry flags exactly the three
/// mechanical typos (double space, space-before-punct, trailing whitespace) and
/// NOT the stylistic ones (`!!!`, a 2-space Markdown hard break) — and the whole
/// layer parks empty when the toggle is off (so a nits-off frame is byte-identical
/// to no nits). Also proves the underline is FLAT (amplitude 0), the shape that
/// distinguishes it from the wavy spell squiggle.
#[test]
fn nit_underlines_flag_mechanical_typos_straight_and_gate_on_the_toggle() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping nit_underlines_flag_mechanical_typos_straight_and_gate_on_the_toggle: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    // line0: double space (nit). line1: space-before-comma (nit). line2: one
    // trailing space (nit). line3: repeated punctuation (NOT a nit). line4: a
    // 2-space Markdown hard break (NOT a nit). Cursor parked on line3 (the
    // clean, nit-free line) — REVEAL-ON-CURSOR suppresses nits on the CARET's
    // own line, so the fixture avoids that line entirely.
    let text = "a  b\nhi ,x\ntrail \nwow!!!\nbreak  \n";
    let v = view(text, 3, 0);
    p.set_view(&v);

    crate::nits::set_nits_on(true);
    let ul = p.nit_underlines();
    assert_eq!(
        ul.len(),
        3,
        "exactly the double-space, space-before-comma, and trailing-space nits"
    );
    // Every nit underline is STRAIGHT (amp 0) — a flat muted line, NOT a squiggle.
    assert!(
        ul.iter().all(|s| s.amp == 0.0 && s.thickness > 0.0 && s.w > 0.0),
        "nit underlines are straight (amp 0), stroked, and non-empty"
    );

    // Toggled OFF: the layer builds NOTHING (byte-identical to no nits at all).
    crate::nits::set_nits_on(false);
    assert!(
        p.nit_underlines().is_empty(),
        "the nits toggle hides every underline"
    );
    crate::nits::set_nits_on(true);
}

/// GFM-TABLE nit exemption: a markdown TABLE row's column-alignment double
/// spaces (`| Name  | Value |`) must NOT nit — the parsed table spans mark those
/// lines as rows, and `ensure_nit_protos` picks `line_nits_table_row` for them
/// (the multi-space rule suppressed). A real prose double space OUTSIDE the table
/// still flags, proving the exemption is scoped to table rows, not blanket.
#[test]
fn nit_underlines_exempt_table_row_column_alignment() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping nit_underlines_exempt_table_row_column_alignment: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    crate::nits::set_nits_on(true);
    // A table whose header + body rows use column-alignment double spaces, then a
    // prose paragraph carrying a GENUINE double space. Caret parked on the prose
    // line's sibling (line 5) so reveal-on-cursor never masks the assertions.
    let text = "| Name  | Value |\n|-------|-------|\n| foo   | 1     |\n\nreal  slip\n\n";
    let mut v = view(text, 5, 0);
    v.is_markdown = true;
    p.set_view(&v);
    let ul = p.nit_underlines();
    // EXACTLY one nit — the prose "real  slip" double space; every table row's
    // alignment run is exempt.
    assert_eq!(
        ul.len(),
        1,
        "only the prose double space nits; table alignment is exempt: {} nits",
        ul.len()
    );

    // Sanity: with the SAME text rendered as PLAIN (non-markdown) — no table
    // spans — the alignment double spaces DO nit, proving the exemption rides the
    // parsed table markup, not the buffer text.
    let mut plain = view(text, 5, 0);
    plain.is_markdown = false;
    p.set_view(&plain);
    assert!(
        p.nit_underlines().len() > 1,
        "without table markup the alignment runs flag as ordinary double spaces"
    );
    crate::nits::set_nits_on(true);
}

/// REVEAL-ON-CURSOR (nits): the CARET's own line never nit-flags, no matter how
/// many mechanical typos it holds — "typing 'word  ' flags instantly" is
/// exactly the mid-thought flicker this suppresses. Move the caret to the
/// OTHER line and that line's nit appears, while the (now caret-owned) line's
/// own nit vanishes — a pure per-frame READ, not a cache rebuild (no reshape
/// between the two reads).
#[test]
fn nit_underlines_suppress_the_entire_caret_line_only() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping nit_underlines_suppress_the_entire_caret_line_only: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    crate::nits::set_nits_on(true);
    // line0 and line1 each carry one double-space nit.
    let text = "a  b\nc  d";
    let mut v = view(text, 0, 0); // caret ON line0
    p.set_view(&v);
    let reshapes = p.reshape_count;
    let ul = p.nit_underlines();
    assert_eq!(ul.len(), 1, "only line1's nit survives while the caret sits on line0");

    v.cursor_line = 1; // caret moves to line1 — a pure cursor move, no reshape
    v.cursor_col = 0;
    p.set_view(&v);
    assert_eq!(p.reshape_count, reshapes, "a pure cursor move must not reshape");
    let ul2 = p.nit_underlines();
    assert_eq!(ul2.len(), 1, "line0's nit now shows; line1's (caret's) is suppressed");
    assert!(
        (ul2[0].x - ul[0].x).abs() > 1.0 || (ul2[0].y - ul[0].y).abs() > 1.0,
        "the surviving nit is the OTHER line's, not the same geometry replayed"
    );
    crate::nits::set_nits_on(true);
}

/// REVEAL-ON-CURSOR (spell): suppresses ONLY the word the caret sits on/next
/// to, NOT the whole line — a DIFFERENT misspelling on the SAME line still
/// squiggles (the taste call the queue flagged explicitly).
#[test]
fn spell_squiggles_suppress_only_the_caret_word_not_the_whole_line() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping spell_squiggles_suppress_only_the_caret_word_not_the_whole_line: no wgpu adapter");
        return;
    };
    // "helo" cols 0..4, "wrld" cols 5..9, both misspelled on the SAME line.
    let text = "helo wrld";
    let mis = vec![
        crate::spell::Misspelling { line: 0, start_col: 0, end_col: 4 },
        crate::spell::Misspelling { line: 0, start_col: 5, end_col: 9 },
    ];
    // Caret ON "helo" (col 0, the word's start — inclusive adjacency).
    let mut v = view(text, 0, 0);
    v.misspelled = mis.clone();
    p.set_view(&v);
    let s = p.spell_squiggles();
    assert_eq!(s.len(), 1, "only 'wrld' squiggles; 'helo' (under the caret) yields");

    // Caret moves to "wrld" (col 5): now "helo" squiggles, "wrld" yields.
    v.cursor_col = 5;
    v.misspelled = mis.clone();
    p.set_view(&v);
    let s2 = p.spell_squiggles();
    assert_eq!(s2.len(), 1, "the OTHER word now squiggles");
    assert!(
        (s2[0].x - s[0].x).abs() > 1.0,
        "the surviving squiggle moved to the other word (helo x={}, wrld x={})",
        s[0].x,
        s2[0].x
    );

    // Caret parked well away from BOTH words: both squiggle.
    v.cursor_col = 100;
    v.misspelled = mis;
    p.set_view(&v);
    assert_eq!(p.spell_squiggles().len(), 2, "no word under the caret => both flag");
}

/// CODE-BUFFER SCOPE (nits): a recognized code buffer restricts nits to the
/// lexer's PROSE regions (comment + string), mirroring spell's scoping — a
/// code-side alignment double-space never nits, while the SAME shape inside a
/// prose comment still does.
#[test]
fn nit_underlines_scope_to_prose_spans_in_a_code_buffer() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping nit_underlines_scope_to_prose_spans_in_a_code_buffer: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    crate::nits::set_nits_on(true);
    // line0: `let x  = 5; // ok  now` — a CODE-side double space ("x  =", cols
    // 5..7, alignment-shaped) and a COMMENT-side double space ("ok  now", cols
    // 17..19, genuine prose). line1 is an untouched parking spot for the caret
    // (reveal-on-cursor must not be the thing suppressing line0's nit here).
    let text = "let x  = 5; // ok  now\nzzz";
    let mut v = view(text, 1, 0);
    v.syn_lang = Some(crate::syntax::Lang::Rust);
    p.set_view(&v);
    let ul = p.nit_underlines();
    assert_eq!(
        ul.len(),
        1,
        "only the comment's prose double-space nits; the code alignment space doesn't"
    );

    // The SAME text with NO recognized language (prose/plain buffer): both
    // double-spaces are eligible (the pre-existing, unscoped behavior).
    let mut v2 = view(text, 1, 0);
    v2.syn_lang = None;
    p.set_view(&v2);
    assert_eq!(
        p.nit_underlines().len(),
        2,
        "a non-code buffer is unscoped: both double-spaces nit"
    );
    crate::nits::set_nits_on(true);
}

/// i18n: a leading frontmatter block's lines never nit — metadata, not
/// manuscript, mirroring the word-count/spell exclusions exactly.
#[test]
fn nit_underlines_exclude_frontmatter_block() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping nit_underlines_exclude_frontmatter_block: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    crate::nits::set_nits_on(true);
    // The frontmatter's own line has a mechanical double-space nit shape
    // (mid-line, not the 2-trailing-spaces hard-break exception); the body
    // has a genuine one too.
    let text = "---\nlang:  ja\n---\nbody  here\nmore\n";
    let v = view(text, 4, 0); // caret on "more" (nit-free), body's own nit stays eligible
    let mut vmd = v;
    vmd.is_markdown = true;
    p.set_view(&vmd);
    let ul = p.nit_underlines();
    assert_eq!(
        ul.len(),
        1,
        "only the body's double-space nits; the frontmatter's own trailing space never does"
    );

    // The SAME text as NON-markdown: frontmatter detection never even runs
    // (it's a markdown-only concept), so BOTH nits are eligible.
    let mut vplain = view(text, 4, 0);
    vplain.is_markdown = false;
    p.set_view(&vplain);
    assert_eq!(
        p.nit_underlines().len(),
        2,
        "a non-markdown buffer never parses frontmatter, so nothing is excluded"
    );
    crate::nits::set_nits_on(true);
}

/// SPELL-GEN + EDIT INVALIDATION: (a) a NEW spell list over the SAME text —
/// only the spell generation moves, NO reshape — must re-place the squiggle
/// under the newly-flagged word; (b) an EDIT that shifts the flagged word
/// right must move BOTH the squiggle and the nit underline (the reshape bumps
/// the RowGeom generation both caches key on). GPU-backed; skips w/o adapter.
#[test]
fn underline_cache_rebuilds_on_spell_list_and_edit() {
    // Squiggle x-positions fold the theme advances + the page wrap globals;
    // nits also read their process toggle. Hold all three (theme → page →
    // nits) so no parallel mutator moves the geometry between reads.
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let _n = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping underline_cache_rebuilds_on_spell_list_and_edit: no wgpu adapter");
        return;
    };
    crate::nits::set_nits_on(true);
    // "helo" (cols 0..4) and "wrld" (cols 5..9) on line0; the double space at
    // cols 9..11 is the nit. A second, untouched line1 gives the cursor
    // somewhere to sit OFF line0 — REVEAL-ON-CURSOR suppresses every nit (and
    // the caret's own word) on the caret's line, which would otherwise
    // swallow this fixture's line0-only spans.
    let text = "helo wrld  x\nzzz";
    let span = |s: usize, e: usize| crate::spell::Misspelling { line: 0, start_col: s, end_col: e };
    let mut v = view(text, 1, 0);
    v.misspelled = vec![span(0, 4)];
    p.set_view(&v);
    let reshapes = p.reshape_count;
    let s1 = p.spell_squiggles();
    assert_eq!(s1.len(), 1, "one misspelling => one squiggle");
    let n1 = p.nit_underlines();
    assert_eq!(n1.len(), 1, "the double space => one nit underline");

    // (a) SAME text, the OTHER word flagged: no reshape (no generation bump),
    // only the spell list generation — the squiggle must still move right.
    let mut v2 = view(text, 1, 0);
    v2.misspelled = vec![span(5, 9)];
    p.set_view(&v2);
    assert_eq!(p.reshape_count, reshapes, "a spell-list-only push must not reshape");
    let s2 = p.spell_squiggles();
    assert_eq!(s2.len(), 1);
    assert!(
        s2[0].x > s1[0].x + 1.0,
        "a new spell list over unchanged text must re-place the squiggle \
         (old x={}, new x={})",
        s1[0].x,
        s2[0].x
    );

    // (b) EDIT: prefix "zz " shifts every flagged span right by 3 columns.
    // The reshape bumps the RowGeom generation, so BOTH proto caches rebuild.
    let edited = "zz helo wrld  x\nzzz";
    let mut v3 = view(edited, 1, 0);
    v3.misspelled = vec![span(3, 7)];
    p.set_view(&v3);
    assert_eq!(p.reshape_count, reshapes + 1, "the edit reshapes once");
    let s3 = p.spell_squiggles();
    assert_eq!(s3.len(), 1);
    assert!(
        s3[0].x > s1[0].x + 1.0,
        "the squiggle must follow the shifted word (old x={}, new x={})",
        s1[0].x,
        s3[0].x
    );
    let n3 = p.nit_underlines();
    assert_eq!(n3.len(), 1);
    assert!(
        n3[0].x > n1[0].x + 1.0,
        "the nit underline must follow the shifted double space \
         (old x={}, new x={})",
        n1[0].x,
        n3[0].x
    );
    crate::nits::set_nits_on(true);
}

/// ZOOM INVALIDATION: a zoom change re-shapes at the new metrics and bumps the
/// RowGeom GENERATION; the cached squiggle/nit protos keyed on it must rebuild
/// so the bands scale with the glyphs instead of replaying zoom-1 pixels.
#[test]
fn underline_cache_rebuilds_on_zoom_change() {
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let _n = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping underline_cache_rebuilds_on_zoom_change: no wgpu adapter");
        return;
    };
    crate::nits::set_nits_on(true);
    // Double space at cols 2..4 (the nit), "helo" at cols 7..11 (the squiggle):
    // both sit past col 0 so their x carries the zoom-scaled advances. A
    // second, untouched line1 gives the cursor somewhere to sit OFF line0
    // (REVEAL-ON-CURSOR would otherwise suppress both fixtures on the caret's
    // own line).
    let text = "aa  bb helo\nzzz";
    let mis = vec![crate::spell::Misspelling { line: 0, start_col: 7, end_col: 11 }];
    let mut v1 = view(text, 1, 0);
    v1.misspelled = mis.clone();
    p.set_view(&v1);
    let s1 = p.spell_squiggles();
    let n1 = p.nit_underlines();
    assert_eq!((s1.len(), n1.len()), (1, 1));

    let mut v2 = view(text, 1, 0);
    v2.misspelled = mis;
    v2.zoom = 1.6;
    p.set_view(&v2);
    let s2 = p.spell_squiggles();
    let n2 = p.nit_underlines();
    assert_eq!((s2.len(), n2.len()), (1, 1));
    // The word starts 7 zoomed advances in: x must move right with the scale.
    assert!(
        s2[0].x > s1[0].x + 1.0,
        "zoom must re-place the squiggle on the scaled advances \
         (z1 x={}, z1.6 x={})",
        s1[0].x,
        s2[0].x
    );
    assert!(
        s2[0].w > s1[0].w + 1.0,
        "the squiggle band must widen with the zoomed glyphs \
         (z1 w={}, z1.6 w={})",
        s1[0].w,
        s2[0].w
    );
    assert!(
        (s2[0].amp - s1[0].amp * 1.6).abs() < 1e-3,
        "the wave amplitude scales with zoom"
    );
    assert!(
        n2[0].x > n1[0].x + 1.0,
        "zoom must re-place the nit underline too (z1 x={}, z1.6 x={})",
        n1[0].x,
        n2[0].x
    );
    crate::nits::set_nits_on(true);
}

/// THEME-FONT-SWITCH INVALIDATION: a display-face switch reshapes
/// (`sync_theme` → `restyle_all_lines` → RowGeom invalidate), so the squiggle
/// protos rebuild against the NEW advances — the band under "brown" must
/// follow the proportional x-range, not replay the mono cell grid.
#[test]
fn underline_cache_rebuilds_on_theme_font_switch() {
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping underline_cache_rebuilds_on_theme_font_switch: no wgpu adapter");
        return;
    };
    theme::set_active_by_name("Tawny").unwrap(); // mono grid
    p.sync_theme();
    let text = "The quick brown fox";
    let mut v = view(text, 0, 0);
    v.misspelled = vec![crate::spell::Misspelling { line: 0, start_col: 10, end_col: 15 }];
    p.set_view(&v);
    let s1 = p.spell_squiggles();
    assert_eq!(s1.len(), 1);

    theme::set_active_by_name("Gumtree").unwrap(); // proportional Literata
    p.sync_theme();
    let s2 = p.spell_squiggles();
    assert_eq!(s2.len(), 1, "the squiggle survives the font switch");
    // The prefix "The quick " and the word "brown" both shape to different
    // advances on the proportional face, so the band's x-range must move.
    assert!(
        (s2[0].x - s1[0].x).abs() > 1.0 || (s2[0].w - s1[0].w).abs() > 1.0,
        "a font switch must rebuild the squiggle on the new advances \
         (mono x={} w={}, serif x={} w={})",
        s1[0].x,
        s1[0].w,
        s2[0].x,
        s2[0].w
    );

    // Restore the default world so other tests see a clean global.
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// SCROLL CULL + REVEAL: the protos are scroll-INDEPENDENT — each frame just
/// adds the current `doc_top` and culls bands outside the viewport plus the
/// generous 8-line margin. A squiggle far below the canvas must emit NOTHING
/// at scroll 0, then appear (inside the canvas) once scrolled into view — all
/// WITHOUT a reshape, so both frames are served by the SAME cached protos.
#[test]
fn squiggle_scroll_culls_offscreen_and_reveals_on_scroll() {
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping squiggle_scroll_culls_offscreen_and_reveals_on_scroll: no wgpu adapter");
        return;
    };
    // 100 short lines; "helo" misspelled on line 60 — ~1900px below the top
    // of the 800px canvas, far past the 8-line-height cull margin.
    let mut text = String::new();
    for i in 0..100 {
        if i == 60 {
            text.push_str("helo\n");
        } else {
            text.push_str(&format!("line {i}\n"));
        }
    }
    let mis = vec![crate::spell::Misspelling { line: 60, start_col: 0, end_col: 4 }];
    let mut v = view(&text, 0, 0);
    v.misspelled = mis.clone();
    p.set_view(&v);
    let reshapes = p.reshape_count;
    assert!(
        p.spell_squiggles().is_empty(),
        "a squiggle far below the viewport is culled (would rasterize nothing)"
    );

    // Scroll the word's row into view: a scroll-only push (no reshape) — the
    // cached proto must now emit a band inside the canvas.
    let mut v2 = view(&text, 0, 0);
    v2.misspelled = mis;
    v2.scroll_lines = 55;
    p.set_view(&v2);
    assert_eq!(p.reshape_count, reshapes, "a scroll-only push must not reshape");
    let s = p.spell_squiggles();
    assert_eq!(s.len(), 1, "scrolled into view: the cached proto now emits");
    assert!(
        s[0].y > 0.0 && s[0].y < p.window_h,
        "the revealed band sits inside the canvas: y={}",
        s[0].y
    );
}

/// SQUIGGLE THICKNESS AT DEFAULT ZOOM (user report: "the 200%-zoom look is
/// right — default zoom reads too thin") — MUTATION-CHECK against the
/// pre-round thin values. The old constants (amp 1.6, period 6.0, thickness
/// 1.8 at zoom 1.0) read correctly ONLY at 2x zoom, since all three multiply
/// by `m.zoom` identically; this round doubles all three, so zoom 1.0 now
/// renders the exact pixels the OLD constants produced at zoom 2.0 — a revert
/// to the old numbers fails every assertion below, and the wave stays exactly
/// as scale-aware as before (still a flat per-constant multiply, checked at
/// zoom 1.6 too).
#[test]
fn spell_squiggle_thickens_at_default_zoom_matching_the_old_200pct_look() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping spell_squiggle_thickens_at_default_zoom_matching_the_old_200pct_look: no wgpu adapter");
        return;
    };
    // "helo" on line0; caret parked on line1 so reveal-on-cursor never
    // suppresses the one squiggle this test reads.
    let text = "helo\nzzz";
    let mut v = view(text, 1, 0);
    v.misspelled = vec![crate::spell::Misspelling { line: 0, start_col: 0, end_col: 4 }];
    p.set_view(&v);
    let s = p.spell_squiggles();
    assert_eq!(s.len(), 1);
    // MUTATION-CHECK: strictly thicker than the pre-round default-zoom value.
    assert!(
        s[0].thickness > 1.8 + 0.5,
        "squiggle stroke must be thicker than the OLD default-zoom value \
         (1.8px, which only looked right at 2x zoom): got {}",
        s[0].thickness
    );
    // Exact new value: zoom 1.0 now matches the OLD zoom-2.0 pixels exactly.
    assert!((s[0].thickness - 3.6).abs() < 1e-3, "thickness = {}", s[0].thickness);
    assert!((s[0].amp - 3.2).abs() < 1e-3, "amp = {}", s[0].amp);
    assert!((s[0].period - 12.0).abs() < 1e-3, "period = {}", s[0].period);

    // SCALE-AWARE: still correct at a non-1.0 zoom — every param keeps
    // multiplying by the SAME zoom factor, so the ratio to zoom 1.0 is exact.
    let mut v2 = view(text, 1, 0);
    v2.misspelled = vec![crate::spell::Misspelling { line: 0, start_col: 0, end_col: 4 }];
    v2.zoom = 1.6;
    p.set_view(&v2);
    let s2 = p.spell_squiggles();
    assert_eq!(s2.len(), 1);
    assert!((s2[0].thickness - s[0].thickness * 1.6).abs() < 1e-3);
    assert!((s2[0].amp - s[0].amp * 1.6).abs() < 1e-3);
    assert!((s2[0].period - s[0].period * 1.6).abs() < 1e-3);
}

/// PER-WORLD BASELINE DIAL (user report: on Bilby the squiggle floats too far
/// below the baseline) — a pixel cross-check against the SAME line's nit
/// underline, which shares the exact same row geometry (`row_band_for`) but
/// rides its OWN, UNCHANGED `cell_bottom + 1.0*zoom` formula (see
/// `render::rects`'s nit-underline builder). On a DEFAULT-dial world
/// (`RenderCaps::spell_underline_gap == SPELL_UNDERLINE_GAP_DEFAULT == 1.0`)
/// the two underlines must land at the EXACT SAME y; on Bilby's tighter dial
/// the squiggle must sit measurably HIGHER (closer to the baseline) by
/// exactly the dial's 2px delta at zoom 1.0 — proof the per-world offset is
/// live DATA driving real geometry, not a comment.
#[test]
fn spell_squiggle_baseline_dial_pulls_bilby_tighter_than_the_shared_default() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping spell_squiggle_baseline_dial_pulls_bilby_tighter_than_the_shared_default: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    crate::nits::set_nits_on(true);
    // "helo" (misspelling, cols 0..4) then a double space (nit, cols 4..6) —
    // SAME line, so both read the SAME row geometry. Caret on line1, off both.
    let text = "helo  wrld\nzzz";
    let mut v = view(text, 1, 0);
    v.misspelled = vec![crate::spell::Misspelling { line: 0, start_col: 0, end_col: 4 }];

    // CONTROL: a default-dial world (Tawny — no override).
    theme::set_active_by_name("Tawny").unwrap();
    p.sync_theme();
    p.set_view(&v);
    let n_ctrl = p.nit_underlines();
    let s_ctrl = p.spell_squiggles();
    assert_eq!((n_ctrl.len(), s_ctrl.len()), (1, 1), "one nit + one squiggle on line0");
    assert!(
        (n_ctrl[0].y - s_ctrl[0].y).abs() < 0.01,
        "on the DEFAULT dial the squiggle must sit at the SAME y as the nit \
         underline (both cell_bottom + 1.0*zoom): nit y={}, squiggle y={}",
        n_ctrl[0].y,
        s_ctrl[0].y
    );

    // BILBY: the tighter per-world override.
    theme::set_active_by_name("Bilby").unwrap();
    p.sync_theme();
    p.set_view(&v);
    let n_bilby = p.nit_underlines();
    let s_bilby = p.spell_squiggles();
    assert_eq!((n_bilby.len(), s_bilby.len()), (1, 1), "one nit + one squiggle on line0");
    let delta = n_bilby[0].y - s_bilby[0].y;
    assert!(
        (delta - 2.0).abs() < 0.05,
        "Bilby's squiggle must sit exactly 2px ABOVE the nit underline's y \
         (the dial's delta at zoom 1.0) — nit y={}, squiggle y={}, delta={delta}",
        n_bilby[0].y,
        s_bilby[0].y
    );

    // OTHER WORLDS UNCHANGED (sweep): every world but Bilby keeps
    // squiggle-y == nit-y, exactly like the control above.
    for t in theme::THEMES.iter().filter(|t| t.name != "Bilby") {
        theme::set_active_by_name(t.name).unwrap();
        p.sync_theme();
        p.set_view(&v);
        let n = p.nit_underlines();
        let s = p.spell_squiggles();
        assert_eq!((n.len(), s.len()), (1, 1), "{}: one nit + one squiggle", t.name);
        assert!(
            (n[0].y - s[0].y).abs() < 0.01,
            "{}: not Bilby, so the squiggle must sit at the SAME y as the nit \
             underline (nit y={}, squiggle y={})",
            t.name,
            n[0].y,
            s[0].y
        );
    }

    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
    crate::nits::set_nits_on(true);
}
