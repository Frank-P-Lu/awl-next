//! The WYSIWYG reveal-on-cursor conceal contract -- line-scoped vs
//! block-scoped reveal, the zero-width metrics collapse, and the pill/panel
//! wash geometry -- split out of the former monolithic `render::tests`
//! (2026-07 code-organization pass).

use super::{headless_pipeline, view};

/// WYSIWYG (the PHILOSOPHY.md amendment): the four LINE-scoped conceal kinds
/// — heading, emphasis, inline code, highlight — each conceal (transparent
/// ink) when the caret is on a DIFFERENT line, and reveal independently the
/// instant the caret lands on their own line, exactly mirroring the
/// pre-existing hr/bullet reveal-on-cursor toggle.
#[test]
fn wysiwyg_conceals_each_line_scoped_kind_off_cursor_and_reveals_on() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wysiwyg_conceals_each_line_scoped_kind_off_cursor_and_reveals_on: no wgpu adapter");
        return;
    };
    // Line 0: heading '#' at byte 0. Line 1: emphasis '**' at byte 0. Line 2:
    // inline-code backtick at byte 0. Line 3: highlight '==' at byte 0. Line 4
    // is a blank line the caret can sit on with NOTHING concealable on it.
    let text = "# Title\n**bold**\n`code`\n==mark==\n";
    let mut off = view(text, 4, 0);
    off.is_markdown = true;
    p.set_view(&off);
    assert!(p.concealed_at(0, 0), "heading '#' concealed off its own line");
    assert!(p.concealed_at(1, 0), "emphasis '**' concealed off its own line");
    assert!(p.concealed_at(2, 0), "inline-code backtick concealed off its own line");
    assert!(p.concealed_at(3, 0), "highlight '==' concealed off its own line");

    // Caret on the HEADING line: only it reveals; the other three stay concealed.
    let mut on0 = view(text, 0, 0);
    on0.is_markdown = true;
    p.set_view(&on0);
    assert!(!p.concealed_at(0, 0), "caret on the heading line reveals its '#'");
    assert!(p.concealed_at(1, 0), "emphasis stays concealed (caret elsewhere)");
    assert!(p.concealed_at(2, 0), "code stays concealed (caret elsewhere)");
    assert!(p.concealed_at(3, 0), "highlight stays concealed (caret elsewhere)");

    // Caret on the EMPHASIS line: only it reveals now; the heading re-conceals.
    let mut on1 = view(text, 1, 0);
    on1.is_markdown = true;
    p.set_view(&on1);
    assert!(p.concealed_at(0, 0), "heading re-conceals once the caret leaves");
    assert!(!p.concealed_at(1, 0), "caret on the emphasis line reveals its '**'");
    assert!(p.concealed_at(2, 0), "code stays concealed");
    assert!(p.concealed_at(3, 0), "highlight stays concealed");

    crate::markdown::set_wysiwyg_on(true);
}

/// WYSIWYG FENCE (BLOCK-scoped): a fenced code block's marker lines (the
/// info-string line + the closing fence) conceal when the caret is OUTSIDE
/// the whole block, and reveal together the instant the caret lands
/// ANYWHERE inside it — including on a BODY line, which itself is NEVER
/// concealed regardless of caret position (it carries its own `Code`
/// coloring, never blanked).
#[test]
fn wysiwyg_fence_markers_are_block_scoped_body_never_conceals() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wysiwyg_fence_markers_are_block_scoped_body_never_conceals: no wgpu adapter");
        return;
    };
    // line0 "prose", line1 "```rust" (open+info), line2 body, line3 "```"
    // (close), line4 "more".
    let text = "prose\n```rust\nlet x = 1;\n```\nmore\n";
    let mut outside = view(text, 0, 0);
    outside.is_markdown = true;
    p.set_view(&outside);
    assert!(p.concealed_at(1, 0), "fence open+info concealed with caret outside the block");
    assert!(p.concealed_at(3, 0), "fence close concealed with caret outside the block");
    assert!(!p.concealed_at(2, 0), "a body line must NEVER conceal");

    // Caret on the BODY line (line 2, inside the block): BOTH marker lines
    // reveal together, and the body line still never conceals.
    let mut inside_body = view(text, 2, 0);
    inside_body.is_markdown = true;
    p.set_view(&inside_body);
    assert!(!p.concealed_at(1, 0), "fence open+info reveals: caret is inside the block");
    assert!(!p.concealed_at(3, 0), "fence close reveals: caret is inside the block");
    assert!(!p.concealed_at(2, 0), "the body line still never conceals");

    // Caret AFTER the block (line 4): both markers re-conceal.
    let mut after = view(text, 4, 0);
    after.is_markdown = true;
    p.set_view(&after);
    assert!(p.concealed_at(1, 0), "fence open+info re-conceals once the caret leaves the block");
    assert!(p.concealed_at(3, 0), "fence close re-conceals once the caret leaves the block");

    crate::markdown::set_wysiwyg_on(true);
}

/// WYSIWYG FRONTMATTER (BLOCK-scoped, reuses the Fence seam verbatim): a
/// `---`-delimited frontmatter block conceals wholesale when the caret is
/// OUTSIDE it and reveals wholesale the instant the caret lands ANYWHERE
/// inside it — no per-line body carve-out (unlike Fence, a frontmatter
/// block has no highlighted body, so the whole thing is markup).
#[test]
fn wysiwyg_frontmatter_is_block_scoped_like_fence() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wysiwyg_frontmatter_is_block_scoped_like_fence: no wgpu adapter");
        return;
    };
    // line0 "---", line1 "lang: ja", line2 "---", line3 "# Title", line4 body.
    let text = "---\nlang: ja\n---\n# Title\nbody\n";
    let mut outside = view(text, 3, 0);
    outside.is_markdown = true;
    p.set_view(&outside);
    assert!(p.concealed_at(0, 0), "opening --- concealed with caret outside the block");
    assert!(p.concealed_at(1, 0), "lang: ja concealed with caret outside the block");
    assert!(p.concealed_at(2, 0), "closing --- concealed with caret outside the block");

    // Caret INSIDE the block (line 1): the whole block reveals together.
    let mut inside = view(text, 1, 0);
    inside.is_markdown = true;
    p.set_view(&inside);
    assert!(!p.concealed_at(0, 0), "opening --- reveals: caret is inside the block");
    assert!(!p.concealed_at(1, 0), "lang: ja reveals: caret is inside the block");
    assert!(!p.concealed_at(2, 0), "closing --- reveals: caret is inside the block");

    // Caret back outside (line 4, the body): re-conceals.
    let mut after = view(text, 4, 0);
    after.is_markdown = true;
    p.set_view(&after);
    assert!(p.concealed_at(0, 0), "re-conceals once the caret leaves the block");
    assert!(p.concealed_at(2, 0), "re-conceals once the caret leaves the block");

    crate::markdown::set_wysiwyg_on(true);
}

/// WYSIWYG OFF (`wysiwyg = false`): a total no-op — every concealable span
/// stays REVEALED (plain dim `Markup`-like styling, exactly the pre-round
/// always-visible markup) regardless of the caret, and the value-step
/// PANEL/PILL washes upload zero geometry, reproducing today's rendering
/// byte-identically.
#[test]
fn wysiwyg_off_never_conceals_and_uploads_no_wash_geometry() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(false);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wysiwyg_off_never_conceals_and_uploads_no_wash_geometry: no wgpu adapter");
        return;
    };
    let text = "# Title\n**bold**\n`code`\n==mark==\nprose\n```rust\nlet x = 1;\n```\nmore\n";
    // Caret nowhere near any concealable line — with WYSIWYG on this would
    // conceal everything; with it OFF, nothing ever conceals.
    let mut v = view(text, 4, 0);
    v.is_markdown = true;
    p.set_view(&v);
    assert!(!p.concealed_at(0, 0), "wysiwyg=false: heading never conceals");
    assert!(!p.concealed_at(1, 0), "wysiwyg=false: emphasis never conceals");
    assert!(!p.concealed_at(2, 0), "wysiwyg=false: inline code never conceals");
    assert!(!p.concealed_at(3, 0), "wysiwyg=false: highlight never conceals");
    assert!(!p.concealed_at(5, 0), "wysiwyg=false: fence open never conceals");
    assert!(!p.concealed_at(7, 0), "wysiwyg=false: fence close never conceals");
    assert!(p.code_pill_rects().is_empty(), "wysiwyg=false: no inline-code pill geometry");
    assert!(p.fence_panel_rects().is_empty(), "wysiwyg=false: no fence-panel geometry");

    crate::markdown::set_wysiwyg_on(true);
}

/// WYSIWYG WASH GEOMETRY: the inline-code PILL and the fenced-code PANEL each
/// upload non-empty geometry when WYSIWYG is on and the buffer has the
/// matching construct — the panel spans EVERY visual row of the block
/// (fence lines AND body), MERGED into ONE continuous quad from block top
/// to block bottom (`merge_row_bands` — the live-review fix for the panel
/// reading as separate striped rows; see its doc comment for the shader
/// seam antialiasing reason a per-row panel looked broken even though the
/// underlying row geometry was already mathematically contiguous).
#[test]
fn wysiwyg_pill_and_panel_rects_present_when_on() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wysiwyg_pill_and_panel_rects_present_when_on: no wgpu adapter");
        return;
    };
    let text = "prose with `inline code` here\n\n```rust\nlet x = 1;\nlet y = 2;\n```\n";
    let mut v = view(text, 0, 0);
    v.is_markdown = true;
    p.set_view(&v);
    let pills = p.code_pill_rects();
    assert_eq!(pills.len(), 1, "one inline-code span => one pill quad: {pills:?}");
    let panels = p.fence_panel_rects();
    // 4 visual rows in the block (the open+info line, the two body lines,
    // and the closing fence line) MERGE into exactly one continuous card —
    // no internal seam between rows.
    assert_eq!(panels.len(), 1, "the whole block merges into one panel quad: {panels:?}");
    let expected_h = 4.0 * p.metrics.line_height;
    assert!(
        (panels[0][3] - expected_h).abs() < 1.0,
        "the merged panel spans all 4 rows' combined height: {panels:?} vs {expected_h}"
    );

    crate::markdown::set_wysiwyg_on(true);
}

/// The pure reveal decision for an IMAGE conceal is LINE-scoped, exactly like
/// heading/emphasis: reveal (show source) iff the caret is on the image's own
/// line; conceal (draw image) otherwise.
#[test]
fn wysiwyg_reveals_image_is_line_scoped() {
    use crate::markdown::ConcealKind;
    let range = 5..30;
    // off-cursor (caret on a DIFFERENT line) -> conceal the source.
    assert!(!super::spans::wysiwyg_reveals(ConcealKind::Image, true, 0, &range));
    // on-cursor (caret on THIS line) -> reveal the raw `![alt](path)` source.
    assert!(super::spans::wysiwyg_reveals(ConcealKind::Image, false, 10, &range));
}

/// A link's `[`/`](url)` plumbing is LINE-scoped, exactly like emphasis /
/// headings / images: concealed off its own line, revealed on it.
#[test]
fn wysiwyg_reveals_link_is_line_scoped() {
    use crate::markdown::ConcealKind;
    let range = 4..25;
    assert!(!super::spans::wysiwyg_reveals(ConcealKind::Link, true, 0, &range));
    assert!(super::spans::wysiwyg_reveals(ConcealKind::Link, false, 10, &range));
}

/// END-TO-END WYSIWYG links: off the caret's line the `[`/`](url)` plumbing
/// conceals to transparent (zero-width) ink while the link TEXT stays visible
/// content ink — so `see [the essay](http://x) now` reads as `see the essay
/// now`; on the caret's own line the whole source reveals for editing. Asserted
/// through the shared `concealed_at` conceal-state reader.
#[test]
fn wysiwyg_link_plumbing_conceals_off_cursor_text_stays_visible() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wysiwyg_link_plumbing_conceals: no wgpu adapter");
        return;
    };
    // Line 0: `see [the essay](http://x) now`. Byte 4 = `[`, bytes 5..14 =
    // `the essay` (link text), bytes 14..25 = `](http://x)` tail, 16 in the url.
    let text = "see [the essay](http://x) now\nprose\n";
    // Caret on line 1 (prose): line 0's link plumbing conceals.
    let mut off = view(text, 1, 0);
    off.is_markdown = true;
    p.set_view(&off);
    assert!(p.concealed_at(0, 4), "opening '[' concealed off the line");
    assert!(p.concealed_at(0, 16), "the url inside the tail concealed off the line");
    assert!(
        !p.concealed_at(0, 8),
        "the link TEXT stays visible (never concealed)"
    );

    // Caret ON line 0: the whole `[text](url)` source reveals for editing.
    let mut on = view(text, 0, 0);
    on.is_markdown = true;
    p.set_view(&on);
    assert!(!p.concealed_at(0, 4), "caret on the link line reveals '['");
    assert!(!p.concealed_at(0, 16), "caret on the link line reveals the url");

    crate::markdown::set_wysiwyg_on(true);
}

/// GHOST SPACING is gone: a concealed heading's `"# "` collapses to ~0
/// advance, so the title starts FLUSH at the column edge (not indented by
/// the markup's natural width), and a concealed emphasis pair collapses to
/// a SINGLE normal word-space between the words on either side — not the
/// "almost  italics" double-gap v1 shipped. Compares the concealed line's
/// `VisualRow::xs` (per-char pixel boundaries) against a PLAIN reference
/// buffer carrying the identical visible characters with no markup at all;
/// zero-width conceal must make the two indistinguishable.
#[test]
fn wysiwyg_zero_width_conceal_collapses_heading_indent_and_emphasis_gap() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!(
            "skipping wysiwyg_zero_width_conceal_collapses_heading_indent_and_emphasis_gap: no wgpu adapter"
        );
        return;
    };

    // --- Heading: "# Title" with the caret on a DIFFERENT line (line 2),
    // so line 0's "# " markup conceals. ---
    let heading_text = "# Title\nprose\nmore prose\n";
    let mut v = view(heading_text, 2, 0);
    v.is_markdown = true;
    p.set_view(&v);
    let rows = p.visual_rows(0);
    let xs = &rows[0].xs;
    // "T" (byte/char col 2, right after the concealed "# ") sits at ~0 —
    // flush at the column edge, not indented by the hash+space's natural
    // width (which would be several pixels).
    assert!(
        xs[2] < 1.0,
        "concealed '# ' collapses to near-zero advance, title starts flush: xs={xs:?}"
    );

    // --- Emphasis: "almost *italics* end" concealed vs the IDENTICAL
    // visible text with no markup at all — the gap between "almost" and
    // "italics" must match a plain single space exactly. ---
    let concealed_text = "almost *italics* end\nprose\n";
    let mut vc = view(concealed_text, 1, 0); // caret on line 1: line 0 conceals
    vc.is_markdown = true;
    p.set_view(&vc);
    let rows_c = p.visual_rows(0);
    let xs_c = &rows_c[0].xs;
    // col 6 = end of "almost" (before the space); col 8 = start of "italics"
    // (right after the concealed '*' at col 7).
    let concealed_gap = xs_c[8] - xs_c[6];

    let plain_text = "almost italics end\nprose\n";
    let mut vp = view(plain_text, 1, 0);
    vp.is_markdown = true;
    p.set_view(&vp);
    let rows_p = p.visual_rows(0);
    let xs_p = &rows_p[0].xs;
    // col 6 = end of "almost"; col 7 = start of "italics" (one real space
    // apart, no markup at all).
    let plain_gap = xs_p[7] - xs_p[6];

    assert!(
        (concealed_gap - plain_gap).abs() < 1.0,
        "concealed '*' collapses so the word-gap matches a plain single space: \
         concealed={concealed_gap} plain={plain_gap} (xs_c={xs_c:?} xs_p={xs_p:?})"
    );

    crate::markdown::set_wysiwyg_on(true);
}

/// The accepted REVEAL-REFLOW cost: the instant the caret enters a
/// concealed line, its markup reveals at FULL width again (the Obsidian
/// behavior this round's spec explicitly accepted) — proving the
/// zero-width collapse is reveal-gated, not a permanent layout change.
#[test]
fn wysiwyg_zero_width_conceal_reveals_full_width_when_caret_enters_line() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!(
            "skipping wysiwyg_zero_width_conceal_reveals_full_width_when_caret_enters_line: no wgpu adapter"
        );
        return;
    };
    let text = "# Title\nprose\n";

    // Caret elsewhere: concealed, title flush at ~0.
    let mut off = view(text, 1, 0);
    off.is_markdown = true;
    p.set_view(&off);
    let xs_off = p.visual_rows(0)[0].xs.clone();
    assert!(xs_off[2] < 1.0, "concealed off-cursor: flush: {xs_off:?}");

    // Caret ON the heading line: reveals at full (real) width — "# " keeps
    // its natural several-pixel advance again.
    let mut on = view(text, 0, 0);
    on.is_markdown = true;
    p.set_view(&on);
    let xs_on = p.visual_rows(0)[0].xs.clone();
    assert!(
        xs_on[2] > 5.0,
        "revealed on-cursor: '# ' keeps its real advance (reflow accepted): {xs_on:?}"
    );

    crate::markdown::set_wysiwyg_on(true);
}

/// HIT-TEST + CARET SANITY on a concealed line: several near-coincident
/// zero-width x boundaries must never panic and must always resolve to a
/// column within the line's valid range — the risk area this round's spec
/// called out explicitly. Sweeps a click across the FULL row width of a
/// concealed heading line (including squarely inside the collapsed "# "
/// run) and asserts every result is in-bounds; also confirms two adjacent
/// concealed byte positions can resolve to DIFFERENT columns without
/// panicking (sequential linear scan over degenerate/duplicate x's).
#[test]
fn wysiwyg_zero_width_conceal_hit_test_stays_in_bounds() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wysiwyg_zero_width_conceal_hit_test_stays_in_bounds: no wgpu adapter");
        return;
    };
    // Line 0 conceals ("# Title"); caret sits on line 1.
    let text = "# Title\nprose\n";
    let mut v = view(text, 1, 0);
    v.is_markdown = true;
    p.set_view(&v);
    let line_char_count = "# Title".chars().count();

    let doc_top = p.doc_top();
    let text_left = p.text_left();
    let py = doc_top + p.metrics.line_height * 0.5;
    // Sweep x from well left of the column through well past the last
    // glyph, including right where the collapsed "# " used to occupy space.
    let mut cols_seen = std::collections::BTreeSet::new();
    for i in -5..40 {
        let px = text_left + i as f32 * 2.0;
        let (line, col) = p.hit_test(px, py, 0);
        assert_eq!(line, 0, "click on row 0's band must resolve to line 0");
        assert!(
            col <= line_char_count,
            "column must stay within the line's char range: col={col} max={line_char_count}"
        );
        cols_seen.insert(col);
    }
    // The sweep must resolve to MORE than one column (not every click
    // collapsing to a single degenerate point) — proves the sequential
    // walk still discriminates real content despite the concealed run's
    // near-coincident x boundaries.
    assert!(
        cols_seen.len() > 1,
        "hit-test sweep should resolve multiple distinct columns: {cols_seen:?}"
    );

    crate::markdown::set_wysiwyg_on(true);
}

/// REGRESSION GUARD: `wysiwyg = false` stays a total no-op for the
/// zero-width mechanism too — a concealable span is never given the
/// near-zero-font-size metrics override (it's only ever plain-dimmed, byte-
/// identical to the pre-WYSIWYG-round rendering), so a heading's `"# "` and
/// an emphasis pair's `"*"` keep their REAL advances regardless of caret
/// position.
#[test]
fn wysiwyg_off_keeps_real_advances_never_zero_width() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(false);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wysiwyg_off_keeps_real_advances_never_zero_width: no wgpu adapter");
        return;
    };
    let text = "# Title\nprose\n";
    let mut v = view(text, 1, 0); // caret elsewhere: would conceal if wysiwyg were on
    v.is_markdown = true;
    p.set_view(&v);
    let xs = p.visual_rows(0)[0].xs.clone();
    assert!(
        xs[2] > 5.0,
        "wysiwyg=false: '# ' keeps its real advance even off-cursor: {xs:?}"
    );

    crate::markdown::set_wysiwyg_on(true);
}

/// REGRESSION GUARD: a non-markdown buffer never runs the WYSIWYG conceal
/// pass at all (no `md_spans`, so `add_wysiwyg_conceal_spans` no-ops
/// trivially) — a `.rs`-style line containing literal `# ` / `*` characters
/// renders at their real advances, byte-identical to before this round.
#[test]
fn wysiwyg_non_markdown_buffer_untouched_by_zero_width_conceal() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wysiwyg_non_markdown_buffer_untouched_by_zero_width_conceal: no wgpu adapter");
        return;
    };
    // A CODE-shaped line with literal '#'/'*' characters, is_markdown=false.
    let text = "# not a heading\nlet y = 2;\n";
    let mut v = view(text, 1, 0);
    v.is_markdown = false;
    p.set_view(&v);
    let xs = p.visual_rows(0)[0].xs.clone();
    assert!(
        xs[2] > 5.0,
        "non-markdown buffer: '# ' is plain text at its real advance: {xs:?}"
    );

    crate::markdown::set_wysiwyg_on(true);
}
