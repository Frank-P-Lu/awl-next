//! Blockquote pull-quote/hanging-mark conceal + the heading-size variable-row
//! machinery (`md_line_scale`, thematic-break row growth, zoom-on-heading
//! caret alignment) -- split out of the former monolithic `render::tests`
//! (2026-07 code-organization pass). See `markdown` for the rest of the
//! markdown-styling suite.

use super::super::*;
use super::{headless_pipeline, view};

/// The blockquote `>` marker CONCEALS off the caret's line (collapses to
/// near-zero advance, so the quote text starts flush at the column edge) and
/// REVEALS at its real advance when the caret lands on the line — the same
/// reveal-on-cursor contract as the heading/emphasis conceal, now generalized
/// to `ConcealKind::Blockquote`.
#[test]
fn blockquote_marker_conceals_off_caret_and_reveals_on_caret() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping blockquote_marker_conceals_off_caret_and_reveals_on_caret: no wgpu adapter");
        return;
    };
    // "> quoted": the "> " marker is chars 0..2; "quoted" starts at char col 2.
    let text = "> quoted\nprose\n";

    // Caret on line 1 (a DIFFERENT line): line 0's "> " conceals to near-zero,
    // so "quoted" starts flush at ~0.
    let mut off = view(text, 1, 0);
    off.is_markdown = true;
    p.set_view(&off);
    let xs_off = p.visual_rows(0)[0].xs.clone();
    assert!(
        xs_off[2] < 1.0,
        "concealed '> ' collapses, quote text starts flush off-cursor: {xs_off:?}"
    );

    // Caret ON the blockquote line: the "> " reveals at its real advance.
    let mut on = view(text, 0, 0);
    on.is_markdown = true;
    p.set_view(&on);
    let xs_on = p.visual_rows(0)[0].xs.clone();
    assert!(
        xs_on[2] > 5.0,
        "revealed on-cursor: '> ' keeps its real advance (reflow accepted): {xs_on:?}"
    );

    crate::markdown::set_wysiwyg_on(true);
}

/// ONE hanging pull-quote mark per contiguous blockquote BLOCK — not per line.
/// Two separate blockquotes yield two blocks; a nested `>>` line stays part of
/// its contiguous block (the markers coalesce), so it never spawns a second
/// mark. Asserted via the page/scroll-independent `quote_block_lines` cache.
#[test]
fn blockquote_hanging_mark_is_one_per_block_nested_coalesces() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping blockquote_hanging_mark_is_one_per_block_nested_coalesces: no wgpu adapter");
        return;
    };
    // Block A: lines 0-1. A blank + a paragraph break the run. Block B: lines
    // 5-6, whose line 6 is a NESTED `>>` (still one contiguous block).
    //  0: "> a"   1: "> b"   2: ""   3: "para"   4: ""   5: "> c"   6: ">> d"
    let text = "> a\n> b\n\npara\n\n> c\n>> d\n";
    let mut v = view(text, 3, 0); // caret on the plain paragraph
    v.is_markdown = true;
    p.set_view(&v);
    assert_eq!(
        p.quote_block_lines(),
        vec![0, 5],
        "one block starting at line 0 (a,b) and one at line 5 (c + nested d)"
    );
}

/// The margin PULL-QUOTE mark is PAGE-MODE only (the left margin exists only in
/// page mode) — `quote_marks` yields a top per visible block in page mode and
/// NOTHING edge-to-edge (the documented non-page fallback: the concealed marker
/// alone). Also present regardless of the caret (a block affordance, not
/// reveal-on-cursor).
#[test]
fn blockquote_pull_quote_mark_page_mode_only() {
    let _w = crate::testlock::serial();
    let _g = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let was_page = crate::page::page_on();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping blockquote_pull_quote_mark_page_mode_only: no wgpu adapter");
        crate::page::set_page_on(was_page);
        return;
    };
    let text = "> a\n> b\n\npara\n\n> c\n";
    let mut v = view(text, 0, 0); // caret INSIDE block A — mark still present
    v.is_markdown = true;
    p.set_view(&v);

    crate::page::set_page_on(true);
    assert_eq!(
        p.quote_marks().len(),
        2,
        "page mode: one hanging mark per visible block, present even with the caret in a block"
    );

    crate::page::set_page_on(false);
    assert!(
        p.quote_marks().is_empty(),
        "edge-to-edge (non-page): no margin, so no hanging mark (concealed marker only)"
    );

    crate::page::set_page_on(was_page);
}

/// DETERMINISM GUARD: a doc with no blockquote produces NO pull-quote marks and
/// NO blockquote conceal spans — nothing here touches a non-blockquote render.
#[test]
fn non_blockquote_doc_has_no_quote_marks() {
    let _w = crate::testlock::serial();
    let _g = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let was_page = crate::page::page_on();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping non_blockquote_doc_has_no_quote_marks: no wgpu adapter");
        crate::page::set_page_on(was_page);
        return;
    };
    let text = "# Title\nplain prose with a > not-a-quote inline\n";
    let mut v = view(text, 0, 0);
    v.is_markdown = true;
    p.set_view(&v);
    crate::page::set_page_on(true);
    assert!(p.quote_block_lines().is_empty(), "no blockquote blocks in a plain doc");
    assert!(p.quote_marks().is_empty(), "no pull-quote marks in a plain doc");
    crate::page::set_page_on(was_page);
}

/// FIX (2026-07-09): the hanging pull-quote DROP-CAP mark must live INSIDE the
/// writing column (in the quote block's own left text-pad gutter), NOT out in the
/// left margin where it collided with the now-default-on OUTLINE. The pure
/// placement law (`geometry::pull_quote_left`): the mark's RIGHT edge
/// clears the quote text's left edge, and its LEFT edge never spills back out of
/// the page into the margin.
#[test]
fn pull_quote_hangs_in_the_column_gutter_never_the_margin() {
    use geometry::pull_quote_left;
    // Typical page-mode geometry: page column at 240, text inset to 280, a small
    // clearance gap, a narrow mark that fits the gutter.
    let (column_left, text_left, gap, mark_w) = (240.0_f32, 280.0_f32, 4.0_f32, 22.0_f32);
    let x = pull_quote_left(column_left, text_left, gap, mark_w);
    assert!(
        x >= column_left - 1e-4,
        "mark left never past the page edge into the outline's margin: {x} < {column_left}"
    );
    assert!(
        x + mark_w <= text_left - gap + 1e-4,
        "mark right edge clears the quote text (a `gap` shy of `text_left`): {} vs {text_left}",
        x + mark_w
    );
    assert!(
        x > column_left + 1e-4,
        "a mark that fits the gutter hangs shy of the text, not flush at the page edge: {x}"
    );
    // An OVER-WIDE mark (wider than the gutter) clamps to `column_left` — it stays
    // INSIDE the page (out of the margin) rather than spilling left into the
    // outline; the accepted cost is a slight overlap with the text, never a
    // collision with the margin.
    let wide = pull_quote_left(column_left, text_left, gap, 100.0);
    assert!(
        (wide - column_left).abs() < 1e-4,
        "an over-wide mark clamps to the page edge, never the margin: {wide}"
    );
}

#[test]
fn md_line_scale_keys_off_leading_hash_count() {
    use crate::markdown::heading_scale;
    // Non-markdown buffer: always body size, whatever the text.
    assert_eq!(md_line_scale("# heading", false), 1.0);
    // Size by the leading-hash COUNT (valid ATX or not).
    assert_eq!(md_line_scale("# h1", true), heading_scale(1));
    assert_eq!(md_line_scale("## h2", true), heading_scale(2));
    assert_eq!(md_line_scale("### h3", true), heading_scale(3));
    assert_eq!(md_line_scale("###### deep", true), heading_scale(3)); // 4+ clamps
    // Grows the instant you type `#`, before the space + title.
    assert_eq!(md_line_scale("#", true), heading_scale(1));
    assert_eq!(md_line_scale("#nospace", true), heading_scale(1));
    assert_eq!(md_line_scale("  ## indented", true), heading_scale(2));
    // A `#` that is NOT the line's leading run is ignored (body size).
    assert_eq!(md_line_scale("not a #heading", true), 1.0);
    assert_eq!(md_line_scale("plain prose", true), 1.0);
}

#[test]
fn md_line_scale_grows_thematic_break_rows_to_the_active_worlds_ornament_scale() {
    // A thematic break grows its row to the ACTIVE WORLD'S per-world ornament scale
    // (no longer a single global rung), so the tall row centers the bigger fleuron
    // — and by the SAME value `prepare_ornaments` shapes the glyph at. md_line_scale
    // reads `theme::active().ornament_scale`, so hold the theme lock while flipping.
    let _t = crate::testlock::serial();

    // A GEOMETRIC world (Currawong → 1.5): every break syntax grows to ITS scale.
    crate::theme::set_active_by_name("Currawong").unwrap();
    let geo = crate::theme::active().ornament_scale;
    assert_eq!(geo, crate::theme::ORNAMENT_SCALE_GEOMETRIC);
    assert_eq!(md_line_scale("---", true), geo);
    assert_eq!(md_line_scale("***", true), geo);
    assert_eq!(md_line_scale("___", true), geo);
    assert_eq!(md_line_scale("- - -", true), geo);

    // An ORNATE world (Mopoke → 2.2): the SAME break lines now grow to the LARGER
    // scale — proof the row height is per-world, not a fixed rung.
    crate::theme::set_active_by_name("Mopoke").unwrap();
    let ornate = crate::theme::active().ornament_scale;
    assert_eq!(ornate, crate::theme::ORNAMENT_SCALE_ORNATE);
    assert!(ornate > geo, "the ornate world grows the break row more than a geometric one");
    assert_eq!(md_line_scale("---", true), ornate);
    assert_eq!(md_line_scale("***", true), ornate);

    // Gated to markdown; a non-md buffer keeps the break at body size (per-world
    // scale never applies), and a dash LIST item (not a break) stays body size.
    assert_eq!(md_line_scale("---", false), 1.0);
    assert_eq!(md_line_scale("- item", true), 1.0);

    crate::theme::set_active(crate::theme::DEFAULT_THEME);
}

#[test]
fn heading_rows_are_taller_and_gated_to_markdown() {
    // The row-count assertion assumes NOTHING wraps, which folds the page
    // globals (column width); hold the page lock (page.rs:95-99).
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping heading_rows_are_taller_and_gated_to_markdown: no wgpu adapter");
        return;
    };
    // line0 = h1, line1 blank, line2/3 body, line4 trailing empty.
    let text = "# Big\n\nbody one\nbody two\n";

    // MARKDOWN: the heading row (row 0) is taller than a body row (row 2) by
    // ~heading_scale(1), while the body rows stay uniform.
    let mut md = view(text, 0, 0);
    md.is_markdown = true;
    p.set_view(&md);
    assert_eq!(p.total_visual_rows(), 5, "no wrap => one row per logical line");
    let h1 = p.row_height_px(0);
    let body = p.row_height_px(2);
    assert!(body > 0.0);
    let ratio = h1 / body;
    let want = crate::markdown::heading_scale(1);
    assert!(
        (ratio - want).abs() < 0.05,
        "h1 row should be ~{want}x a body row, got {ratio} ({h1}/{body})"
    );
    // Body rows are uniform among themselves.
    assert!((p.row_height_px(2) - p.row_height_px(3)).abs() < 0.01);
    let md_doc_h = p.total_doc_height();

    // NON-MARKDOWN: the SAME text shapes with uniform rows (no heading growth),
    // proving the size is gated like every other md effect.
    let mut plain = view(text, 0, 0);
    plain.is_markdown = false;
    p.set_view(&plain);
    assert!(
        (p.row_height_px(0) - p.row_height_px(2)).abs() < 0.01,
        "a non-markdown buffer must keep every row a uniform height"
    );
    assert!(
        md_doc_h > p.total_doc_height(),
        "the heading must make the markdown document taller in pixels"
    );

    // Non-wrapped: visual_row_of still equals the logical line, so cursor-follow
    // is unchanged when nothing wraps even though rows differ in height.
    p.set_view(&md);
    assert_eq!(p.visual_row_of(2, 0), 2);
}

/// PER-WORLD HEADING WEIGHT — the DISTINGUISHABILITY law: in EVERY world,
/// under its OWN proposed `heading_bold` bit (no force, the shipped data),
/// each heading level stays measurably distinct from body at the shaped-pixel
/// outcome — the Ladder-J rungs (1.6/1.3/1.15) must survive as three strictly
/// descending row heights above body, whatever the world's face or weight bit
/// does. Outcome, not mechanism: the assertion is over the real pipeline's
/// per-row pixel heights, never the consts (a future ladder retune that
/// collapses two rungs — or a face whose metrics swallow a step — fails here,
/// not in a mirror of the arithmetic).
#[test]
fn heading_levels_stay_measurably_distinct_from_body_in_every_world() {
    // Row-height math folds the page wrap globals AND the active theme —
    // hold both locks (theme, then page).
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping heading_levels_stay_measurably_distinct_from_body_in_every_world: no wgpu adapter");
        return;
    };
    // h1 / h2 / h3 / body, one per line; caret parked on the body line so the
    // heading rows sit in their settled (marker-concealed) state.
    let text = "# word\n## word\n### word\nword\n";
    for t in crate::theme::THEMES.iter() {
        crate::theme::set_active_by_name(t.name).unwrap();
        p.sync_theme();
        let mut md = view(text, 3, 0);
        md.is_markdown = true;
        p.set_view(&md);
        let (h1, h2, h3, body) = (
            p.row_height_px(0),
            p.row_height_px(1),
            p.row_height_px(2),
            p.row_height_px(3),
        );
        assert!(body > 0.0, "{}: body row must have height", t.name);
        for (name, h) in [("h1", h1), ("h2", h2), ("h3", h3)] {
            assert!(
                h > body + 1.0,
                "{}: {name} row ({h}px) must read measurably taller than body ({body}px)",
                t.name
            );
        }
        assert!(
            h1 > h2 + 1.0 && h2 > h3 + 1.0,
            "{}: the ladder must descend strictly — h1 {h1} > h2 {h2} > h3 {h3}",
            t.name
        );
    }
    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    p.sync_theme();
}

#[test]
fn thematic_break_row_grows_by_the_active_worlds_ornament_scale_and_refits_on_theme_switch() {
    // Row-height math folds the page wrap globals AND reads the active theme's
    // per-world ornament scale — hold both locks (order: theme, then page).
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping thematic_break_row_ornament_scale: no wgpu adapter");
        return;
    };
    // A thematic break (row 0) over a plain body line (row 2).
    let text = "---\n\nbody line\n";
    let mut md = view(text, 2, 0); // caret on the body line (logical line 2), NOT the break
    md.is_markdown = true;

    // GEOMETRIC world (Currawong → 1.5): the break row grows to ~1.5x a body row.
    crate::theme::set_active_by_name("Currawong").unwrap();
    p.set_view(&md);
    let body = p.row_height_px(2);
    assert!(body > 0.0);
    let geo_break = p.row_height_px(0);
    let geo_ratio = geo_break / body;
    assert!(
        (geo_ratio - crate::theme::ORNAMENT_SCALE_GEOMETRIC).abs() < 0.05,
        "Currawong break row should be ~{}x a body row, got {geo_ratio}",
        crate::theme::ORNAMENT_SCALE_GEOMETRIC
    );

    // Switch to an ORNATE world (Mopoke → 2.2) and RESHAPE via the same theme-font
    // seam a live theme switch rides: the break row must RE-FIT to the larger scale
    // (proof the row-height ↔ glyph-box coupling is per-world, picked up on switch).
    crate::theme::set_active_by_name("Mopoke").unwrap();
    p.sync_theme_font();
    let body2 = p.row_height_px(2);
    let ornate_break = p.row_height_px(0);
    let ornate_ratio = ornate_break / body2;
    assert!(
        (ornate_ratio - crate::theme::ORNAMENT_SCALE_ORNATE).abs() < 0.05,
        "Mopoke break row should be ~{}x a body row, got {ornate_ratio}",
        crate::theme::ORNAMENT_SCALE_ORNATE
    );
    assert!(
        ornate_break > geo_break + 0.5,
        "the ornate world must grow the break row taller than the geometric one \
         ({ornate_break} vs {geo_break})"
    );

    crate::theme::set_active(crate::theme::DEFAULT_THEME);
}

#[test]
fn heading_size_survives_theme_switch() {
    // Shaping folds the theme font AND the page wrap globals; hold both
    // (theme → page order, page.rs:95-99).
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping heading_size_survives_theme_switch: no wgpu adapter");
        return;
    };
    theme::set_active_by_name("Tawny").unwrap();
    p.sync_theme();
    let text = "# Big\n\nbody one\nbody two\n";
    let mut md = view(text, 0, 0);
    md.is_markdown = true;
    p.set_view(&md);
    let ratio_before = p.row_height_px(0) / p.row_height_px(2);
    assert!(ratio_before > 1.4, "sanity: heading taller before switch ({ratio_before})");

    // Switch to a DIFFERENT-font world: the heading must STAY bigger. The bug was
    // `sync_theme` rebuilding CJK-only attrs, which dropped the markdown styling
    // and shrank headings back to body size on a live theme switch.
    theme::set_active_by_name("Gumtree").unwrap();
    p.sync_theme();
    let ratio_after = p.row_height_px(0) / p.row_height_px(2);
    assert!(
        ratio_after > 1.4,
        "heading must stay larger than body after a theme/font switch ({ratio_after})"
    );

    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// BUG regression (user screenshot 2026-07-04): zooming with the caret ON a
/// heading line left the amber block caret floating ~half a row above the
/// glyphs while the text itself re-laid correctly. Root cause: `set_view`
/// called `set_caret_target` (which reads the cursor's row geometry via
/// `cursor_row_height`/`caret_cell_top`) BEFORE the zoom-triggered
/// `restyle_all_lines` — so on a doc with headings, a zoom step reshaped body
/// text at the new metrics while the heading line's ABSOLUTE per-span pixel
/// metrics (set by the PREVIOUS restyle) were still stale until
/// `restyle_all_lines` ran, moments later, with no caret-target recompute
/// after it. The caret spring latched a target built from the transient,
/// pre-restyle row geometry — and nothing ever asked it to recompute once the
/// geometry settled.
#[test]
fn zoom_on_heading_line_keeps_caret_target_aligned() {
    // Shaping folds the theme font AND the page wrap globals; hold both
    // (theme -> page order, page.rs:95-99).
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping zoom_on_heading_line_keeps_caret_target_aligned: no wgpu adapter");
        return;
    };
    let text = "## h2\n\nbody one\nbody two\n";

    // 1) Open the markdown doc with the caret on a BODY line at zoom 1.0. The
    // md-flip restyle fires here, but the cursor's own row is a body row
    // (unaffected by heading scale), so this establishes a clean baseline.
    let mut v = view(text, 2, 0);
    v.is_markdown = true;
    v.zoom = 1.0;
    p.set_view(&v);

    // 2) Move the caret ONTO the heading line, zoom unchanged: a plain
    // cursor-move target update against already-settled heading geometry.
    let mut v2 = view(text, 0, 3);
    v2.is_markdown = true;
    v2.zoom = 1.0;
    p.set_view(&v2);
    let (_, target_before_zoom, _, _) = p.caret_snapshot();

    // 3) Zoom, caret still on the heading line. This is the exact repro: the
    // zoom step both rescales body metrics AND (because the doc has a
    // heading) triggers `restyle_all_lines` to rescale the heading's
    // absolute pixel metrics to match.
    let row0_h_before = p.row_height_px(0);
    let mut v3 = view(text, 0, 3);
    v3.is_markdown = true;
    v3.zoom = 1.6;
    p.set_view(&v3);
    let (_, target_after_zoom, _, _) = p.caret_snapshot();

    // Sanity: the heading row itself really did grow with the zoom (the
    // "text re-lays correctly" half of the bug report) — read fresh from the
    // settled row-geometry table, not the caret.
    let row0_h_after = p.row_height_px(0);
    assert!(
        row0_h_after > row0_h_before * 1.3,
        "sanity: a 1.6x zoom must actually grow the heading row's height \
         (before={row0_h_before} after={row0_h_after})"
    );
    let _ = target_before_zoom;

    // The pipeline's state is fully settled after `set_view` returns (the
    // conditional restyle, if any, has already run), so a FRESH read of the
    // pure `caret_target_xy()` reflects the true, post-restyle geometry —
    // independent of whatever order `set_view` computed things in. The
    // caret's LATCHED spring target must agree with it.
    let (correct_x, correct_y) = p.caret_target_xy();
    assert!(
        (target_after_zoom.0 - correct_x).abs() < 0.5,
            "caret target x must match the settled heading-row geometry \
         (latched={:?}, correct=({correct_x}, {correct_y}))",
        target_after_zoom
    );
    assert!(
        (target_after_zoom.1 - correct_y).abs() < 0.5,
        "caret target y must match the settled heading-row geometry, not a \
         stale pre-restyle row height (latched={:?}, correct=({correct_x}, {correct_y}))",
        target_after_zoom
    );
}
