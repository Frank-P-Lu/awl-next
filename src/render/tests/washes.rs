//! Selection-rect geometry + the comment/string/fence WASH quads (proto-cache
//! contract, row-band merging, fence-panel cache warmth) -- split out of the
//! former monolithic `render::tests` (2026-07 code-organization pass).

use super::{headless_pipeline, view};

#[test]
fn selection_rects_multiline_geometry_and_eol_pad() {
    // Selection x geometry folds the page globals (text_left + wrap width);
    // hold the page lock so a parallel page write can't move it (page.rs:95-99).
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping selection_rects_multiline_geometry_and_eol_pad: no wgpu adapter");
        return;
    };
    // A 3-line buffer, selection from line0 col2 through line2 col3: line0 is a
    // partial first line (col2..eol), line1 a full middle line, line2 a partial
    // last line (0..col3).
    let text = "alpha\nbeta\ngamma";
    let mut v = view(text, 2, 3);
    v.selection = Some(((0, 2), (2, 3)));
    p.set_view(&v);

    let rects = p.selection_rects();
    assert_eq!(rects.len(), 3, "one rect per logical line: {rects:?}");

    let m = &p.metrics;
    let eol_pad = m.char_width * 0.5;
    let doc_top = p.doc_top();
    let left = p.text_left();

    // The middle + last lines start at the writing-column left; the first line is
    // inset by its start column.
    assert!((rects[1][0] - left).abs() < 1e-3, "middle line starts at left");
    assert!((rects[2][0] - left).abs() < 1e-3, "last line starts at left");
    assert!(rects[0][0] > left + 1e-3, "first line is inset by its start col");

    // Rows descend in order by one line_height each (uniform, non-heading).
    assert!(rects[0][1] < rects[1][1] && rects[1][1] < rects[2][1], "rows descend");
    assert!(
        (rects[1][1] - rects[0][1] - m.line_height).abs() < 1e-3,
        "row spacing == line_height"
    );
    // Row 0 sits at doc_top centered within its line height.
    let want_y0 = doc_top + (m.line_height - m.caret_h) * 0.5;
    assert!((rects[0][1] - want_y0).abs() < 1e-3, "row0 y centered: {} vs {}", rects[0][1], want_y0);
    // Each rect is one (unscaled) caret-height band.
    for r in &rects {
        assert!((r[3] - m.caret_h).abs() < 1e-3, "rect height == caret_h: {r:?}");
    }

    // The EOL pad: the full middle line equals a no-EOL full selection of the
    // same line PLUS the trailing-newline sliver.
    let mid_no_eol = p.range_rects((1, 0), (1, 4));
    assert_eq!(mid_no_eol.len(), 1, "single-line full selection: {mid_no_eol:?}");
    assert!(
        (rects[1][2] - (mid_no_eol[0][2] + eol_pad)).abs() < 1e-3,
        "middle width == full line + eol_pad: {} vs {}+{}",
        rects[1][2], mid_no_eol[0][2], eol_pad
    );
    // The last line has NO eol pad (it stops at the cursor column).
    let last_only = p.range_rects((2, 0), (2, 3));
    assert!(
        (rects[2][2] - last_only[0][2]).abs() < 1e-3,
        "last line width has no eol pad: {} vs {}",
        rects[2][2], last_only[0][2]
    );
}

/// PERF O(visible): `range_rects` (selection / search) over a Select-All in a
/// TALL doc scrolled to the MIDDLE emits only the visible band's rects — never
/// one per document line — AND resolves the geometry through the BATCHED
/// `visual_rows_for_lines`, so it never clobbers the single-slot cursor-line row
/// memo. The pre-fix per-line `line_glyph_xs` + `visual_rows` walk did BOTH: an
/// O(doc^2)-per-frame Select-All and a memo stomp on the last selected line. This
/// WITNESSES THE WORK (the memo survives) rather than just the bounded return.
#[test]
fn range_rects_selection_is_visible_bounded_and_memo_safe() {
    // Selection x/y geometry folds the page globals; hold the page lock so a
    // parallel page write can't move the writing column mid-test.
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping range_rects_selection_is_visible_bounded_and_memo_safe: no wgpu adapter");
        return;
    };
    // 2000 short single-row lines: every line is a Select-All member, but only
    // the on-screen band can paint. Scroll to the middle so the lines above sit
    // off the top of the viewport and the tail below it.
    const N: usize = 2000;
    let text: String = (0..N).map(|i| format!("line {i}\n")).collect();
    let cursor_line = N / 2;
    let mut v = view(&text, cursor_line, 0);
    v.scroll_lines = cursor_line - 5; // put the cursor line near the view top
    p.set_view(&v);

    // WARM the single-slot cursor-line memo, then prove Select-All leaves it
    // intact — a per-line `visual_rows` walk (the retired path) would have
    // overwritten it with the LAST selected line's rows.
    let _ = p.visual_rows(cursor_line);
    assert!(
        p.row_geom.cached_rows(cursor_line).is_some(),
        "precondition: the cursor-line row memo is warm"
    );

    let last_col = format!("line {}", N - 1).chars().count();
    let rects = p.range_rects((0, 0), (N - 1, last_col));

    // O(visible): the emitted rects are bounded by the visible band + margin, NOT
    // one per document line (2000).
    assert!(!rects.is_empty(), "the visible selection must produce rects");
    assert!(
        rects.len() < 200,
        "Select-All must emit only the visible band's rects, got {} of {N}",
        rects.len()
    );

    // WITNESS THE WORK: the batched resolve left the cursor-line memo warm.
    assert!(
        p.row_geom.cached_rows(cursor_line).is_some(),
        "range_rects must resolve via the batched path and NOT clobber the cursor-line memo"
    );

    // The cull is exact per row: every emitted rect lands within the viewport +
    // the generous ornament margin (the same band `proto_visible` gates on).
    let margin = p.metrics.line_height * 8.0;
    for r in &rects {
        let (y, h) = (r[1], r[3]);
        assert!(
            y + h > -margin && y < p.window_h + margin,
            "every emitted rect is within the visible band: {r:?}"
        );
    }
}

/// SYNTAX WASH CACHE + GEOMETRY: a code buffer's PROSE comment and STRING
/// spans produce wash quads; commented-out code (CommentCode) produces NONE;
/// a cursor move / scroll keeps the proto cache WARM (version unchanged, no
/// reshape — the squiggle-cache invalidation contract); an EDIT rebuilds it;
/// and a prose buffer yields zero rects (byte-identical render).
#[test]
fn wash_cache_and_geometry_contract() {
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wash_cache_and_geometry_contract: no wgpu adapter");
        return;
    };
    // A rust buffer: a prose comment (washed), a commented-out statement
    // (NOT washed) and a string literal (washed on this dark default world).
    let text = "// a calm prose note\n// let x = foo(bar);\nlet s = \"hi\";\n";
    let mut v = view(text, 0, 0);
    v.syn_lang = Some(crate::syntax::Lang::Rust);
    p.set_view(&v);
    let (comments, strings, highlights) = p.wash_rects();
    assert!(highlights.is_empty(), "a code buffer never has highlight washes");
    assert_eq!(
        comments.len(), 1,
        "one prose comment => one wash band (the commented-out statement gets none): {comments:?}"
    );
    assert_eq!(strings.len(), 1, "one string literal => one string wash band");
    let key = p.wash_cache_version().expect("protos built");
    let reshapes = p.reshape_count;

    // A CURSOR MOVE keeps the cache warm (no reshape, no rebuild).
    let mut v2 = view(text, 2, 3);
    v2.syn_lang = Some(crate::syntax::Lang::Rust);
    p.set_view(&v2);
    let _ = p.wash_rects();
    assert_eq!(p.reshape_count, reshapes, "a cursor move must not reshape");
    assert_eq!(p.wash_cache_version(), Some(key), "a cursor move keeps the wash protos warm");

    // A SCROLL keeps it warm too (scroll only shifts the per-frame offset).
    let mut v3 = view(text, 2, 3);
    v3.syn_lang = Some(crate::syntax::Lang::Rust);
    v3.scroll_lines = 1;
    p.set_view(&v3);
    let _ = p.wash_rects();
    assert_eq!(p.wash_cache_version(), Some(key), "a scroll keeps the wash protos warm");

    // An EDIT reshapes once and rebuilds the protos (new version key).
    let edited = "// a calm prose note!!\n// let x = foo(bar);\nlet s = \"hi\";\n";
    let mut v4 = view(edited, 0, 0);
    v4.syn_lang = Some(crate::syntax::Lang::Rust);
    p.set_view(&v4);
    let (c2, s2, _h2) = p.wash_rects();
    assert_eq!(p.reshape_count, reshapes + 1, "the edit reshapes once");
    assert_ne!(p.wash_cache_version(), Some(key), "an edit rebuilds the wash protos");
    assert_eq!((c2.len(), s2.len()), (1, 1));

    // PROSE (no syn_lang, not markdown): zero rects — byte-identical render.
    p.set_view(&view("plain prose here\n", 0, 0));
    let (c3, s3, _h3) = p.wash_rects();
    assert!(c3.is_empty() && s3.is_empty(), "prose buffers carry no washes");
}

/// WASH O(visible): on a TALL code doc the per-frame wash pass emits only the
/// visible band's quads (proto cull) — never one per document line.
#[test]
fn wash_rects_cull_to_visible_band() {
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wash_rects_cull_to_visible_band: no wgpu adapter");
        return;
    };
    // 600 prose-comment lines: every line carries a wash PROTO, but the frame
    // must emit only the visible band (canvas rows + the generous margin).
    let text: String = (0..600).map(|i| format!("// prose note number {i}\n")).collect();
    let mut v = view(&text, 0, 0);
    v.syn_lang = Some(crate::syntax::Lang::Rust);
    p.set_view(&v);
    let (comments, _, _) = p.wash_rects();
    assert!(!comments.is_empty(), "the visible comments must wash");
    assert!(
        comments.len() < 150,
        "emitted wash quads must be bounded by the visible band, got {} of 600",
        comments.len()
    );
}

/// MARKDOWN FENCES inherit the washes through the SAME seam (decision 4):
/// a ```rust fence's prose comment + string wash; markdown WITHOUT fences
/// (and the fence's own surrounding prose) yields zero wash quads.
#[test]
fn markdown_fence_inherits_washes() {
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping markdown_fence_inherits_washes: no wgpu adapter");
        return;
    };
    let text = "prose before\n```rust\n// a calm fence note\nlet s = \"hi\";\n```\nprose after\n";
    let mut v = view(text, 0, 0);
    v.is_markdown = true;
    p.set_view(&v);
    let (comments, strings, highlights) = p.wash_rects();
    assert_eq!(comments.len(), 1, "the fence's prose comment washes: {comments:?}");
    assert_eq!(strings.len(), 1, "the fence's string washes: {strings:?}");
    assert!(highlights.is_empty(), "a fenced code block carries no highlight washes");

    // Markdown with NO fence: no washes at all (prose byte-identity).
    let mut v2 = view("# title\nplain prose paragraph\n", 0, 0);
    v2.is_markdown = true;
    p.set_view(&v2);
    let (c, s, h) = p.wash_rects();
    assert!(c.is_empty() && s.is_empty() && h.is_empty(), "fence-less markdown carries no washes");
}

/// MARKDOWN `==highlight==`: the marked text carries an `MdKind::Highlight`
/// span (reported as `"highlight"` in the sidecar) and its wash quad rides
/// its OWN dedicated HIGHLIGHT bucket + violet pipeline — DECOUPLED from the
/// prose-comment wash (a deliberate, narrow break of the one-warm-wash owner
/// so a highlighter POPS): the highlight produces exactly one quad in the
/// third `wash_rects` slot and ZERO in the comment/string buckets. A
/// `.rs`-style CODE buffer (`syn_lang` set, `is_markdown` false) with the
/// identical `==` bytes — a comparison operator, never a highlight — carries
/// NEITHER an `md_spans` entry nor an extra wash quad, because
/// `markdown::spans` is never invoked at all off the `is_markdown` gate
/// (`parse_doc_spans`).
#[test]
fn markdown_highlight_inherits_wash_and_code_buffers_never_match() {
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!(
            "skipping markdown_highlight_inherits_wash_and_code_buffers_never_match: no wgpu adapter"
        );
        return;
    };
    let text = "prose before ==marked text== prose after\n";
    let mut v = view(text, 0, 0);
    v.is_markdown = true;
    p.set_view(&v);
    let spans = p.md_report();
    assert!(
        spans.iter().any(|(s, e, t)| *s == 15 && *e == 26 && *t == "highlight"),
        "'marked text' (15..26) should be a highlight span: {spans:?}"
    );
    assert!(
        spans.iter().any(|(s, e, t)| *s == 13 && *e == 15 && *t == "markup"),
        "the opening '==' dims to markup: {spans:?}"
    );
    assert!(
        spans.iter().any(|(s, e, t)| *s == 26 && *e == 28 && *t == "markup"),
        "the closing '==' dims to markup: {spans:?}"
    );
    let (comments, strings, highlights) = p.wash_rects();
    assert_eq!(
        highlights.len(), 1,
        "the highlight rides its OWN dedicated highlight-wash bucket: {highlights:?}"
    );
    assert!(
        comments.is_empty(),
        "a highlight is DECOUPLED from the comment wash, never in its bucket: {comments:?}"
    );
    assert!(strings.is_empty(), "a highlight never touches the string bucket");

    // The IDENTICAL `==` bytes in a CODE buffer (a comparison operator, not a
    // highlight): no md spans at all, and consequently no extra wash quad.
    let code_text = "let ok = a ==marked text== b;\n";
    let mut vc = view(code_text, 0, 0);
    vc.is_markdown = false;
    vc.syn_lang = Some(crate::syntax::Lang::Rust);
    p.set_view(&vc);
    assert!(
        p.md_report().is_empty(),
        "a code buffer must never run the markdown highlight pass: {:?}",
        p.md_report()
    );
}

/// THE WRITER'S DIFF — the marked-up-manuscript transcript DRAWS awl's real diff
/// vocabulary: an inserted paragraph rides the `==highlight==` WASH (a drawn quad in
/// `wash_rects`' highlight bucket, its own violet tint), a struck deletion carries
/// REAL `~~` strikethrough (drawn strike-line quads from `strike_lines`, positioned
/// by THE one owner `spans::strike_line_band` — the strikethrough-render round
/// retired the combining-stroke mechanism), and the fold/deletion blockquotes dim. This
/// asserts the APPEARANCE at the DRAWN-quad level (the `wash_rects` oracle the
/// codebase uses instead of pixel-diffing), so the washed region is proven present
/// from the actual render, never inferred from state. The transcript is exactly what
/// `prosediff::render_markdown` produces, so the live diff view + the capture harness
/// both draw this.
#[test]
fn prose_diff_transcript_draws_highlight_wash_and_struck_deletion() {
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping prose_diff_transcript_draws_highlight_wash_and_struck_deletion: no wgpu adapter");
        return;
    };
    // A deletion, an insertion, and an untouched-fold — the real serializer output.
    let old = "Keep this opening line.\n\nDrop this whole paragraph entirely now.";
    let new = "Keep this opening line.\n\nA brand new inserted paragraph arrives.";
    let transcript = crate::prosediff::render_markdown(
        old,
        new,
        crate::prosediff::Params::shipping(),
        "Comparing with earlier",
    );
    // The transcript IS awl's diff vocabulary: a struck deletion + a washed insertion.
    assert!(transcript.contains("~~"), "struck deletion speaks real `~~` markdown");
    assert!(transcript.contains("=="), "highlight-washed insertion in the drawn text");

    // Park the caret on the blank line 1 (as the diff view does) so nothing reveals.
    let mut v = view(&transcript, 1, 0);
    v.is_markdown = true;
    p.set_view(&v);

    // md_report shows the washed insertion as a `highlight` span (its `==` dim markup).
    let spans = p.md_report();
    assert!(
        spans.iter().any(|(_, _, t)| *t == "highlight"),
        "the inserted paragraph is a highlight span: {spans:?}"
    );
    // APPEARANCE ORACLE: the highlight rides its OWN drawn wash quad (violet tint),
    // decoupled from the comment/string buckets — the washed region is really drawn.
    let (comments, strings, highlights) = p.wash_rects();
    assert!(
        !highlights.is_empty(),
        "the inserted paragraph draws a highlight-wash quad: {highlights:?}"
    );
    assert!(comments.is_empty() && strings.is_empty(), "no code washes in a prose diff");
    // APPEARANCE ORACLE, strike half: the deleted paragraph really draws its
    // strike-line quads (the diff's struck rendering routes through the SAME
    // `MdKind::Strikethrough` → `strike_lines` path body prose uses — one owner,
    // no diff-only strike mechanism).
    let strikes = p.strike_lines();
    assert!(!strikes.is_empty(), "the struck deletion draws strike-line quads");
    for s in &strikes {
        assert!(s.amp == 0.0 && s.thickness > 0.0, "a strike is a flat positive stroke: {s:?}");
    }
}

/// `merge_row_bands` PURE UNIT CONTRACT: vertically-contiguous same-x
/// bands collapse to one quad spanning their union; a variable-width run
/// merges to the UNION x-range; two bands on the SAME row (equal y) never
/// merge into each other; a real vertical GAP (an intervening unlisted row)
/// keeps bands separate.
#[test]
fn merge_row_bands_contract() {
    use super::rects::merge_row_bands;
    // Three contiguous same-width rows (a uniform "panel") -> one quad.
    let uniform = vec![[10.0, 0.0, 100.0, 32.0], [10.0, 32.0, 100.0, 32.0], [10.0, 64.0, 100.0, 32.0]];
    let merged = merge_row_bands(uniform);
    assert_eq!(merged.len(), 1, "three contiguous rows merge to one: {merged:?}");
    assert!((merged[0][1] - 0.0).abs() < 1e-3, "merged top == first row's top");
    assert!((merged[0][3] - 96.0).abs() < 1e-3, "merged height == sum of all three: {merged:?}");
    assert!((merged[0][0] - 10.0).abs() < 1e-3 && (merged[0][2] - 100.0).abs() < 1e-3);

    // Variable-width contiguous rows (a wrapped prose wash) -> ONE quad at
    // the UNION x-range.
    let variable = vec![[20.0, 0.0, 30.0, 32.0], [5.0, 32.0, 80.0, 32.0]];
    let merged_v = merge_row_bands(variable);
    assert_eq!(merged_v.len(), 1, "variable-width contiguous rows still merge: {merged_v:?}");
    assert!((merged_v[0][0] - 5.0).abs() < 1e-3, "union left == the wider row's left");
    assert!((merged_v[0][2] - 80.0).abs() < 1e-3, "union width == max(20+30, 5+80) - 5 = 80: {merged_v:?}");
    assert!((merged_v[0][3] - 64.0).abs() < 1e-3);

    // Two bands on the SAME row (equal y, disjoint x) never merge into
    // each other.
    let same_row = vec![[0.0, 0.0, 10.0, 32.0], [50.0, 0.0, 10.0, 32.0]];
    let merged_s = merge_row_bands(same_row);
    assert_eq!(merged_s.len(), 2, "same-row bands stay separate: {merged_s:?}");

    // A real vertical GAP (row 2 skipped entirely) keeps the two runs apart.
    let gapped = vec![[0.0, 0.0, 10.0, 32.0], [0.0, 64.0, 10.0, 32.0]];
    let merged_g = merge_row_bands(gapped);
    assert_eq!(merged_g.len(), 2, "a real gap keeps bands separate: {merged_g:?}");

    // Empty / single input pass through untouched.
    assert!(merge_row_bands(Vec::new()).is_empty());
    let one = vec![[1.0, 2.0, 3.0, 4.0]];
    assert_eq!(merge_row_bands(one.clone()), one);
}

/// MULTI-ROW WASH SEAM: a multi-line `/* ... */` block comment (three
/// contiguous visual rows, same bucket) merges into ONE continuous quad —
/// the live-review's "python docstring wash striping" report. Compares
/// against the merged height so a future regression (e.g. reintroducing
/// per-row emission without the merge) is caught directly.
#[test]
fn multiline_comment_wash_merges_into_one_continuous_band() {
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping multiline_comment_wash_merges_into_one_continuous_band: no wgpu adapter");
        return;
    };
    let text = "/* line one\n   line two\n   line three */\nlet x = 1;\n";
    let mut v = view(text, 3, 0);
    v.syn_lang = Some(crate::syntax::Lang::Rust);
    p.set_view(&v);
    let (comments, _strings, _highlights) = p.wash_rects();
    assert_eq!(
        comments.len(), 1,
        "a 3-row block comment merges into one continuous wash band: {comments:?}"
    );
    let expected_h = 3.0 * p.metrics.line_height;
    assert!(
        (comments[0][3] - expected_h).abs() < 1.0,
        "merged band spans all 3 rows: {comments:?} vs {expected_h}"
    );
}

/// FENCE-PANEL CACHE contract, mirroring `wash_cache_and_geometry_contract`:
/// a cursor move / scroll keeps the proto cache warm (no rebuild); an edit
/// reshapes once and rebuilds it (a new version key).
#[test]
fn fence_panel_cache_stays_warm_across_cursor_and_scroll_rebuilds_on_edit() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping fence_panel_cache_stays_warm_across_cursor_and_scroll_rebuilds_on_edit: no wgpu adapter");
        return;
    };
    let text = "```rust\nlet x = 1;\n```\n";
    let mut v = view(text, 0, 0);
    v.is_markdown = true;
    p.set_view(&v);
    let _ = p.fence_panel_rects();
    let key = p.fence_panel_cache_version().expect("protos built");
    let reshapes = p.reshape_count;

    // A cursor move (revealing the fence) keeps the cache warm.
    let mut v2 = view(text, 1, 0);
    v2.is_markdown = true;
    p.set_view(&v2);
    let _ = p.fence_panel_rects();
    assert_eq!(p.reshape_count, reshapes, "a cursor move must not reshape");
    assert_eq!(
        p.fence_panel_cache_version(), Some(key),
        "a cursor move keeps the fence-panel protos warm"
    );

    // An edit reshapes once and rebuilds the protos (new version key).
    let edited = "```rust\nlet x = 2;\n```\n";
    let mut v3 = view(edited, 0, 0);
    v3.is_markdown = true;
    p.set_view(&v3);
    let _ = p.fence_panel_rects();
    assert_eq!(p.reshape_count, reshapes + 1, "the edit reshapes once");
    assert_ne!(
        p.fence_panel_cache_version(), Some(key),
        "an edit rebuilds the fence-panel protos"
    );

    crate::markdown::set_wysiwyg_on(true);
}

/// WYSIWYG rides the WASH + FENCE-PANEL cache KEYS: both caches BUILD a
/// WYSIWYG-gated bucket (the inline-code pill; the whole fence panel), and
/// `wysiwyg_on()` is a process-global that can flip WITHOUT a reshape. So a
/// runtime flip must REKEY each cache and force a rebuild — never serve the
/// stale on-state protos. Pre-fix the key was only `(generation, reshape_count)`,
/// so a flip left the key unchanged and the stale pill/panel kept drawing.
#[test]
fn wysiwyg_flip_rekeys_wash_and_fence_panel_caches() {
    let _w = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wysiwyg_flip_rekeys_wash_and_fence_panel_caches: no wgpu adapter");
        return;
    };
    // A markdown buffer carrying BOTH a WYSIWYG-gated inline-code pill and a
    // WYSIWYG-gated fenced-block panel. The md_spans (inline `Code` + the
    // `ConcealMarkup(Fence)` range) are parsed at set_view time and do NOT depend
    // on the wysiwyg global — only the PROTO build does — so flipping the global
    // afterward isolates the cache-key contract.
    let text = "an `inline` bit\n\n```rust\nlet x = 1;\n```\n";
    let mut v = view(text, 0, 0);
    v.is_markdown = true;

    // WYSIWYG ON: the pill + panel are present; capture each cache's key.
    crate::markdown::set_wysiwyg_on(true);
    p.set_view(&v);
    assert!(!p.code_pill_rects().is_empty(), "wysiwyg on: the inline-code pill draws");
    assert!(!p.fence_panel_rects().is_empty(), "wysiwyg on: the fence panel draws");
    let wash_key_on = p.wash_cache_version().expect("wash protos built");
    let panel_key_on = p.fence_panel_cache_version().expect("panel protos built");

    // Flip WYSIWYG OFF with NO reshape / geometry change (same buffer, same
    // view) — only the process-global toggled. Both caches must REKEY (the
    // wysiwyg half of the key flips) and rebuild to the empty buckets. A stale
    // (generation, reshape_count)-only key would still serve the on-state pill /
    // panel here.
    crate::markdown::set_wysiwyg_on(false);
    assert!(
        p.code_pill_rects().is_empty(),
        "wysiwyg off: no pill (a stale wash bucket would still draw one)"
    );
    assert!(
        p.fence_panel_rects().is_empty(),
        "wysiwyg off: no panel (a stale fence-panel bucket would still draw one)"
    );
    assert_ne!(
        p.wash_cache_version(), Some(wash_key_on),
        "flipping wysiwyg rekeys the wash cache"
    );
    assert_ne!(
        p.fence_panel_cache_version(), Some(panel_key_on),
        "flipping wysiwyg rekeys the fence-panel cache"
    );

    // restore the sticky default for any later test on this thread
    crate::markdown::set_wysiwyg_on(true);
}

/// `~~strikethrough~~` — the drawn STRIKE LINE's geometry contract, at the
/// render seam: one flat quad band per struck run, x-hugging the struck glyphs
/// (the SAME `xs` boundaries the selection rect reads, so line and text can't
/// disagree), vertically centered by THE ONE OWNER (`spans::strike_line_band` at
/// `STRIKE_V_FRAC` of the row's glyph cell — the fn the popover's `S` button
/// also rides), NOT caret-gated (content styling: the line stays while the
/// caret edits the run — only the `~~` MARKER conceal is reveal-on-cursor), and
/// absent entirely for a strike-less buffer (byte-identity's geometry half).
#[test]
fn strike_lines_hug_the_struck_run_and_survive_the_caret() {
    let _t = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping strike_lines_hug_the_struck_run_and_survive_the_caret: no wgpu adapter");
        return;
    };
    let text = "keep ~~cut~~ end\nplain second line\n";
    // Caret parked on line 1: the ~~ markers conceal, the strike line draws.
    let mut v = view(text, 1, 0);
    v.is_markdown = true;
    p.set_view(&v);

    let strikes = p.strike_lines();
    assert_eq!(strikes.len(), 1, "one struck run, one strike line: {strikes:?}");
    let s = strikes[0];
    assert_eq!(s.amp, 0.0, "a strike is FLAT");
    assert!(s.thickness > 0.0, "positive stroke");

    // The struck content "cut" is chars 7..10 on line 0; its selection rect
    // reads the SAME row xs boundaries the strike proto captured.
    let sel = p.range_rects((0, 7), (0, 10));
    assert_eq!(sel.len(), 1, "one selection rect for the struck run: {sel:?}");
    let [rx, ry, rw, rh] = sel[0];
    assert!((s.x - rx).abs() < 0.6, "strike x hugs the run: {} vs {rx}", s.x);
    assert!((s.w - rw).abs() < 2.5, "strike width hugs the run: {} vs {rw}", s.w);
    // Vertically INSIDE the run's glyph cell, centered by the owner's fraction.
    let center = s.y + s.h * 0.5;
    assert!(
        center > ry && center < ry + rh,
        "strike center {center} inside the glyph cell [{ry}, {}]",
        ry + rh
    );

    // Caret ON the struck line: the raw markers reveal, but the strike LINE
    // stays (content styling, not marker conceal) — it re-hugs the now-wider
    // run (the revealed `~~` advances shift the xs).
    let mut on = view(text, 0, 8);
    on.is_markdown = true;
    p.set_view(&on);
    let on_strikes = p.strike_lines();
    assert_eq!(on_strikes.len(), 1, "the strike line survives the caret landing");

    // A strike-less buffer: zero strike geometry (the byte-identity half — the
    // pipeline uploads zero instances, so nothing can draw).
    let mut plain = view("no struck text here\n", 0, 0);
    plain.is_markdown = true;
    p.set_view(&plain);
    assert!(p.strike_lines().is_empty(), "no struck span, no strike geometry");
}

/// THE ONE-OWNER LAW, ink half: the struck TEXT's ink (`md_attrs`'
/// `Strikethrough` arm) and the strike LINE's pipeline tint
/// (`strike_srgba_bytes`) both read `spans::strike_ink` — for EVERY world, the
/// two are byte-equal, and equal to the world's own `muted` rung (an ink-ladder
/// value, structurally incapable of reading as the caret's amber — DESIGN §3).
#[test]
fn strike_text_and_line_share_one_ink_in_every_world() {
    for th in crate::theme::THEMES.iter() {
        let ink = super::spans::strike_ink(th);
        assert_eq!(
            ink, th.muted,
            "{}: strike ink IS the world's muted rung",
            th.name
        );
    }
}
