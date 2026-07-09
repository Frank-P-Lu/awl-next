//! The persistent margin Outline (current heading, follow-window, edge-fade,
//! narrow-margin hide) -- split out of the former monolithic `render::tests`
//! (2026-07 code-organization pass).

use super::super::*;
use super::{headless_pipeline, view, view_md};

#[test]
fn outline_headings_stashed_and_current_is_nearest_at_or_above_caret() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping outline_headings_stashed: no wgpu adapter");
        return;
    };
    // "# Title" (line 0), "## Section A" (line 4), "### Deep" (line 8).
    let text = "# Title\n\nsome prose\n\n## Section A\n\nbody\n\n### Deep\n";

    // A NON-markdown buffer stashes NO outline headings (gated on md_enabled).
    let mut plain = view(text, 0, 0);
    plain.is_markdown = false;
    p.set_view(&plain);
    let (_on, headings, current) = p.outline_report();
    assert!(headings.is_empty(), "non-markdown buffer has no outline: {headings:?}");
    assert_eq!(current, None);

    // A MARKDOWN buffer distills the three headings (riding the md parse).
    let mut md = view(text, 0, 0);
    md.is_markdown = true;
    p.set_view(&md);
    let (_on, headings, current) = p.outline_report();
    assert_eq!(
        headings,
        vec![("Title", 1u8, 0usize), ("Section A", 2, 4), ("Deep", 3, 8)],
        "three headings in document order"
    );
    // Caret on line 0 (the first heading): current is that heading.
    assert_eq!(current, Some(0));

    // Caret on line 2 (prose under the first heading): still the first heading —
    // the nearest AT or ABOVE the caret line.
    p.set_view(&view_md(text, 2, 0));
    assert_eq!(p.outline_current(), Some(0));

    // Caret on line 4 (the second heading's own line): that heading.
    p.set_view(&view_md(text, 4, 0));
    assert_eq!(p.outline_current(), Some(1));

    // Caret on line 6 (body under the second heading): still the second.
    p.set_view(&view_md(text, 6, 0));
    assert_eq!(p.outline_current(), Some(1));

    // Caret on the deepest heading's line 8: the third heading.
    p.set_view(&view_md(text, 8, 0));
    assert_eq!(p.outline_current(), Some(2));
}

#[test]
fn outline_current_is_none_above_the_first_heading() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping outline_current_none_above: no wgpu adapter");
        return;
    };
    // Prose BEFORE the first heading: a caret up there has no heading at/above it.
    let text = "intro line\nmore intro\n\n# First\n\nbody\n";
    p.set_view(&view_md(text, 0, 0));
    assert_eq!(p.outline_current(), None, "caret above the first heading");
    // Move onto the heading line (line 3): now the first heading is current.
    p.set_view(&view_md(text, 3, 0));
    assert_eq!(p.outline_current(), Some(0));
}

/// THE MARGIN OUTLINE RENDER: it draws its heading list ONLY when on + page mode +
/// markdown + a wide-enough margin, and hides gracefully otherwise (off / edge-to-edge
/// / non-markdown / heading-free). The CURRENT heading (nearest at/above the caret)
/// is the one CONTENT (dark) row among the FAINT rest — asserted here via the
/// drawn-lines report, the SAME `outline_layout` owner the pixels shape from.
#[test]
fn outline_draws_on_page_md_and_the_current_row_is_flagged() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping outline_draws_on_page_md: no wgpu adapter");
        return;
    };
    let _o = crate::testlock::serial();
    let _g = crate::testlock::serial();
    crate::outline::set_outline_on(true);
    crate::page::set_measure(40);
    crate::page::set_page_on(true);
    // A WIDE window so the left margin comfortably clears the OUTLINE_MIN_CHARS floor
    // (at the 1200px default the page margin is too narrow — see the floor test).
    p.set_size(1900.0, 900.0);
    // Three headings; caret on the first (line 0).
    let text = "# Title\n\nprose\n\n## Section A\n\nbody\n\n### Deep\n";
    p.set_view(&view_md(text, 0, 0));

    use chrome::{OutlineRow, OutlineRung};
    // `line` is the source heading's 0-based document line (the click-to-jump
    // target): "# Title" is line 0, "## Section A" line 4, "### Deep" line 8.
    // These cases are fully visible (nothing clips), so every row is un-`faded`.
    let row = |label: &str, rung: OutlineRung, current: bool, gap_before: bool, line: usize| {
        OutlineRow { label: label.to_string(), rung, faded: false, current, gap_before, line }
    };

    let lines = p
        .outline_draw_report(900)
        .expect("page + md + on + a wide margin => the outline is drawn");
    // The per-level indent rides `Heading::label()` (h1 flush, h2/h3 indented).
    // TWO-STATE ink: caret on line 0 lights ONLY the current H1 (Content); every
    // other heading is Faint (depth reads from the indent, not ink). A half-row
    // group gap precedes the H2 (a later top-level section), never the H3.
    assert_eq!(
        lines,
        vec![
            row("Title", OutlineRung::Content, true, false, 0),
            row("  Section A", OutlineRung::Faint, false, true, 4),
            row("    Deep", OutlineRung::Faint, false, false, 8),
        ],
        "current H1 = Content; every other heading Faint; a group gap before the H2"
    );

    // The current row FOLLOWS the caret: move onto the second heading's line (4).
    // Now ONLY the H2 is current (Content); the H1 — an ancestor, but ancestry no
    // longer lifts — drops back to Faint alongside the H3.
    p.set_view(&view_md(text, 4, 0));
    let lines = p.outline_draw_report(900).unwrap();
    assert_eq!(
        lines,
        vec![
            row("Title", OutlineRung::Faint, false, false, 0),
            row("  Section A", OutlineRung::Content, true, true, 4),
            row("    Deep", OutlineRung::Faint, false, false, 8),
        ],
        "only the caret's current H2 is Content; the H1 ancestor is Faint (no lift)"
    );

    // OFF => hidden (None), so a default (off) frame is byte-identical.
    crate::outline::set_outline_on(false);
    assert_eq!(p.outline_draw_report(900), None, "outline off hides it");
    crate::outline::set_outline_on(true);

    // EDGE-TO-EDGE (page off): no margin, so the outline hides.
    crate::page::set_page_on(false);
    p.set_view(&view_md(text, 0, 0));
    assert_eq!(p.outline_draw_report(900), None, "edge-to-edge hides the outline");
    crate::page::set_page_on(true);

    // NON-MARKDOWN: no headings distilled, so the outline hides.
    let mut plain = view(text, 0, 0);
    plain.is_markdown = false;
    p.set_view(&plain);
    assert_eq!(p.outline_draw_report(900), None, "a non-markdown buffer has no outline");

    // A markdown buffer with NO headings hides too.
    p.set_view(&view_md("just prose, no headings here\n", 0, 0));
    assert_eq!(p.outline_draw_report(900), None, "a heading-free doc hides the outline");

    crate::outline::set_outline_on(false);
    crate::page::set_page_on(false);
    crate::page::set_measure(80);
}

/// GRACEFUL HIDE: below the [`rowlayout::OUTLINE_MIN_CHARS`] margin floor the whole
/// outline vanishes rather than draw a useless sliver — exactly as the gutter
/// collapses on a narrow margin. The fixture derives the char budget from the same
/// pure geometry the pipeline uses, so a future constant tweak can't make it stale.
#[test]
fn outline_hides_below_the_narrow_margin_floor() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping outline_hides_below_the_narrow_margin_floor: no wgpu adapter");
        return;
    };
    let _o = crate::testlock::serial();
    let _g = crate::testlock::serial();
    crate::outline::set_outline_on(true);
    let measure = 70usize;
    crate::page::set_measure(measure);
    crate::page::set_page_on(true);
    // The 1200px default width: the page margin is genuinely narrow here.
    let window_w = 1200.0;
    p.set_size(window_w, 800.0);
    let text = "# Title\n\n## Section\n";
    p.set_view(&view_md(text, 0, 0));

    // Self-check the fixture lands BELOW the floor (derived, not guessed) — the
    // outline's band is `[TEXT_LEFT, column_left - gap)`, one pad narrower than the
    // gutter's, at the LABEL scale it renders at.
    let col_left = column_left_for(window_w, CHAR_WIDTH, true, measure);
    let gap = CHAR_WIDTH * 1.5;
    let avail = col_left - gap - TEXT_LEFT;
    let label_char_w = CHAR_WIDTH * crate::markdown::type_scale::LABEL;
    let avail_chars = (avail / label_char_w).floor().max(0.0) as usize;
    assert!(
        avail_chars < rowlayout::OUTLINE_MIN_CHARS,
        "fixture must land the margin BELOW the outline floor, got avail_chars={avail_chars}"
    );
    assert_eq!(
        p.outline_draw_report(800),
        None,
        "a margin below the floor hides the outline (graceful collapse)"
    );

    crate::outline::set_outline_on(false);
    crate::page::set_page_on(false);
    crate::page::set_measure(80);
}

/// LONG-DOC FOLLOW (the chosen default): when the headings outnumber the rows the
/// margin can hold, the visible window SLIDES to keep the CURRENT heading on screen —
/// the section you are in never scrolls off. Uses a SHORT canvas height so only a
/// few rows fit, with the caret deep in the document.
#[test]
fn outline_follow_keeps_the_current_heading_visible_on_a_long_doc() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping outline_follow_keeps_current_visible: no wgpu adapter");
        return;
    };
    let _o = crate::testlock::serial();
    let _g = crate::testlock::serial();
    crate::outline::set_outline_on(true);
    crate::page::set_measure(40);
    crate::page::set_page_on(true);
    // Wide enough for the char floor, SHORT enough that only a few rows fit.
    let height = 220u32;
    p.set_size(1900.0, height as f32);
    // 40 headings, each block "# Hi\n\nbody\n\n" => heading i sits on line 4*i.
    let mut text = String::new();
    for i in 0..40 {
        text.push_str(&format!("# H{i}\n\nbody\n\n"));
    }
    let last = 39usize;
    p.set_view(&view_md(&text, 4 * last, 0));

    let lines = p
        .outline_draw_report(height)
        .expect("a wide margin + real headings => the outline draws");
    // The margin holds FEWER rows than there are headings (the follow is exercised).
    assert!(
        lines.len() < 40,
        "the short canvas must hold fewer rows than headings, got {}",
        lines.len()
    );
    // EXACTLY one row is the current one, and it is the LAST heading — the caret's
    // section, kept visible by the follow rather than scrolled off the top.
    let current: Vec<&chrome::OutlineRow> = lines.iter().filter(|r| r.current).collect();
    assert_eq!(current.len(), 1, "the current section is always in the followed window");
    assert_eq!(current[0].label, "H39", "the followed window keeps the caret's heading");

    crate::outline::set_outline_on(false);
    crate::page::set_page_on(false);
    crate::page::set_measure(80);
}

/// EDGE FADE: when the follow-window CLIPS (more headings than fit), the clipped
/// first / last visible row is marked `faded` — its Faint ink drops toward the
/// ground via ALPHA, a quiet "more above / more below" — while the current row
/// (Content, pinned to the bottom edge by the follow) is NEVER faded. A fully-
/// visible outline fades nothing.
#[test]
fn outline_edge_fade_dims_the_clipped_rows_but_not_the_current() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping outline_edge_fade: no wgpu adapter");
        return;
    };
    let _o = crate::testlock::serial();
    let _g = crate::testlock::serial();
    crate::outline::set_outline_on(true);
    crate::page::set_measure(40);
    crate::page::set_page_on(true);
    use chrome::OutlineRung;

    // CLIPPING: 40 top-level headings, a SHORT canvas, caret mid-doc (heading 20).
    // The follow pins heading 20 to the bottom edge (clips below) and clips above.
    let height = 220u32;
    p.set_size(1900.0, height as f32);
    let mut text = String::new();
    for i in 0..40 {
        text.push_str(&format!("# H{i}\n\nbody\n\n"));
    }
    p.set_view(&view_md(&text, 4 * 20, 0));
    let lines = p.outline_draw_report(height).expect("outline draws");
    assert!(lines.len() < 40 && lines.len() >= 3, "the window clips, got {}", lines.len());
    // The clipped FIRST row is a non-current heading (Faint) marked `faded` — the
    // "more above" whisper rides ALPHA now (no rung below Faint to step down to).
    assert!(!lines[0].current, "the first clipped row is not the current heading");
    assert_eq!(lines[0].rung, OutlineRung::Faint, "every non-current row is Faint");
    assert!(lines[0].faded, "the clipped top row fades toward the ground (alpha)");
    // The LAST row is the current heading, pinned to the bottom edge — Content and
    // NEVER faded despite the below-clip (the you-are-here row wins over the hint).
    let last = lines.last().unwrap();
    assert!(last.current, "the follow pins the current heading to the bottom edge");
    assert_eq!(last.rung, OutlineRung::Content, "the current row is Content");
    assert!(!last.faded, "the current row is never faded by the edge hint");
    // Interior non-current rows are Faint and NOT faded (only the clipped edges are).
    assert!(
        lines[1..lines.len() - 1].iter().any(|r| !r.current && !r.faded && r.rung == OutlineRung::Faint),
        "interior rows are un-faded Faint"
    );

    // FULLY VISIBLE: 3 headings on a tall canvas, caret on the LAST — nothing clips,
    // so the first (non-current) row is plain un-faded Faint.
    p.set_size(1900.0, 900.0);
    let short = "# One\n\nbody\n\n# Two\n\nbody\n\n# Three\n";
    p.set_view(&view_md(short, 16, 0)); // caret on "# Three"
    let lines = p.outline_draw_report(900).expect("outline draws");
    assert_eq!(lines.len(), 3, "all headings visible");
    assert!(!lines[0].current);
    assert_eq!(lines[0].rung, OutlineRung::Faint, "a non-current row is Faint");
    assert!(!lines[0].faded, "a fully-visible outline fades no edge");

    crate::outline::set_outline_on(false);
    crate::page::set_page_on(false);
    crate::page::set_measure(80);
}

/// THE CLICK-TARGETING BUG, FIXED: a heading whose CHAR COUNT fits the estimated
/// budget but whose WIDE GLYPHS (a run of `⌘`) shape wider than `avail` used to
/// WORD-WRAP onto a SECOND visual row (no `Wrap::None`, the label fit by a
/// monospace char-count estimate alone) — pushing every row below it one `row_h`
/// lower on screen than `outline_hit_line`'s fixed-`row_h`-per-heading math assumed,
/// so a click on a LATER heading landed on the heading BEFORE it. This drives the
/// REAL draw path (`prepare`, a real device/queue — not just `outline_layout`) so
/// the assertions read the ACTUAL shaped `glyphon` geometry, then confirms
/// `outline_hit_line` resolves each row's own drawn y-band back to its own
/// heading — draw and hit-test can no longer disagree.
#[test]
fn outline_hit_test_stays_aligned_past_a_wide_glyph_heading() {
    let got = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl outline hit-test device"),
                ..Default::default()
            })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let mut p = TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
        p.set_size(1900.0, 900.0);
        Some((device, queue, p))
    });
    let Some((device, queue, mut p)) = got else {
        eprintln!("skipping outline_hit_test_stays_aligned_past_a_wide_glyph_heading: no wgpu adapter");
        return;
    };
    let _o = crate::testlock::serial();
    let _g = crate::testlock::serial();
    crate::outline::set_outline_on(true);
    crate::page::set_measure(40);
    crate::page::set_page_on(true);
    p.set_size(1900.0, 900.0);

    // All H3 (never top-level -> `group_gap_before` is always false), so the row
    // math below is exactly one drawn line per heading, in order, no interleaved
    // group-gap lines to account for.
    let wide = "⌘".repeat(40);
    let text = format!("### {wide}\n\n### Second\n\n### Third\n\n### Fourth\n");
    p.set_view(&view_md(&text, 0, 0));
    p.prepare(&device, &queue, 1900, 900).unwrap();

    // Capture the REAL drawn glyphon geometry FIRST — `outline_draw_report` (below)
    // reuses `outline_buffer` for its own pixel measurements and would otherwise
    // clobber it before we get to read the actual prepared draw.
    let runs: Vec<f32> = p.outline_buffer.layout_runs().map(|r| r.line_top).collect();
    assert_eq!(runs.len(), 4, "one visual row per heading — nothing wrapped: {runs:?}");

    let lines = p.outline_draw_report(900).expect("outline draws");
    assert_eq!(lines.len(), 4);
    assert!(lines.iter().all(|r| !r.gap_before), "an H3-only fixture opens no group gaps");
    // Self-check the fixture actually stresses the fix: the wide heading's DRAWN
    // label is shorter than the raw (INDENTED) text — proving the char estimate
    // alone left it too wide and the pixel correction had to shrink it further.
    let raw_indented_len = format!("    {wide}").chars().count();
    assert!(
        lines[0].label.chars().count() < raw_indented_len,
        "the wide heading must have needed pixel-fit shrinking: {:?}",
        lines[0].label
    );

    // Walk each row's REAL drawn y (top + its own run's line_top) and confirm a
    // click landing there resolves through `outline_hit_line` to THAT row's OWN
    // heading, never a neighbour's.
    let m = p.metrics;
    let row_h = m.line_height * crate::markdown::type_scale::LABEL;
    for (i, row) in lines.iter().enumerate() {
        let drawn_y = TEXT_TOP + runs[i];
        let band_center = drawn_y + row_h * 0.5;
        let hit = p.outline_hit_line(TEXT_LEFT + 1.0, band_center, 900);
        assert_eq!(
            hit,
            Some(row.line),
            "row {i} ({row:?}) drawn at y={drawn_y} must hit-test to its own heading line"
        );
    }

    crate::outline::set_outline_on(false);
    crate::page::set_page_on(false);
    crate::page::set_measure(80);
}

/// THE ONE-LINE-PER-ROW LAW: after `outline_pixel_fit`, every DRAWN row's label
/// measures AT OR UNDER `avail` px — never merely "fits the char-count estimate" —
/// so `Wrap::None` (`prepare_outline`) never actually needs to clip anything, and a
/// wide-glyph heading can never visually spill past the margin into the document.
/// Swept over a plain title, a wide-glyph title, a subtitle-bearing title, and a
/// wide-but-plain-Latin title, all at a DELIBERATELY generous-by-char /
/// cramped-by-pixel `avail` so the estimate alone would routinely overflow.
#[test]
fn outline_pixel_fit_never_leaves_a_label_wider_than_avail() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping outline_pixel_fit_never_leaves_a_label_wider_than_avail: no wgpu adapter");
        return;
    };
    let _o = crate::testlock::serial();
    let _g = crate::testlock::serial();
    crate::outline::set_outline_on(true);
    crate::page::set_measure(40);
    crate::page::set_page_on(true);
    p.set_size(1900.0, 900.0);

    let text = format!(
        "### {}\n\n### {}\n\n### Plain title\n\n### {}\n",
        "⌘".repeat(40),
        "Head — a subtitle that would normally be dropped first, quite a long one",
        "M".repeat(60),
    );
    p.set_view(&view_md(&text, 0, 0));

    let avail = p.outline_avail_px(900).expect("outline draws at this size");
    let lines = p.outline_draw_report(900).expect("outline draws");
    assert_eq!(lines.len(), 4, "all four headings show");
    for row in &lines {
        let w = p.measure_outline_label_px(&row.label);
        assert!(
            w <= avail + 0.5, // sub-pixel float slop
            "row {row:?} measures {w}px, past avail {avail}px — a fitted label must never overflow its own row"
        );
    }

    crate::outline::set_outline_on(false);
    crate::page::set_page_on(false);
    crate::page::set_measure(80);
}
