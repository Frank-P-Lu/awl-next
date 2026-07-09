//! Table column allocation (min-content floor, rigid token columns), the
//! x-ray caret redirect/pan, and per-cell inline-markdown conceal (bold /
//! italic / code) -- split out of the former monolithic `render::tests`
//! (2026-07 code-organization pass).

use super::super::*;
use super::{headless_pipeline, view, view_md};

/// TABLE COLUMN ALLOCATION (the CSS auto-table shape — the fix for the
/// "Da wn"/"Tim e" mid-word-break bug): a wide GFM table's TOKEN columns
/// (single-word cells — World / Time / Register) hold a rigid min-content
/// floor and NEVER shrink as the writing column narrows, while its PHRASE
/// columns (Ground / Ornament) absorb the whole squeeze by word-wrapping.
/// Driven end-to-end through the REAL font: `prepare_table_grid` measures the
/// per-column min/max content and lays them out, and the deterministic
/// `tables_report()` carries the laid widths. The distinctive signature vs the
/// retired proportional-shrink clamp is that the token columns are
/// BYTE-IDENTICAL across two very different measures (the old clamp scaled
/// EVERY column, so they would have differed).
#[test]
fn table_allocation_holds_token_columns_rigid_across_widths() {
    let _w = crate::testlock::serial();
    let _g = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    crate::page::set_page_on(true);
    let got = pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl table-alloc test device"),
                ..Default::default()
            })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let mut p =
            TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
        p.set_size(1200.0, 800.0);
        Some((device, queue, p))
    });
    let Some((device, queue, mut p)) = got else {
        eprintln!("skipping table_allocation_holds_token_columns_rigid_across_widths: no wgpu adapter");
        return;
    };
    // A WORLDS.md-style wide table: token columns 0/4/5 (World/Time/Register)
    // are single words; phrase columns 1/3 (Ground/Ornament) carry multi-word
    // phrases. The caret sits on the trailing prose (line 5), off the table, so
    // the grid draws off-cursor and its widths are measured + reported.
    let text = "\
| World      | Ground                | Display     | Ornament                          | Time  | Register |\n\
|------------|-----------------------|-------------|-----------------------------------|-------|----------|\n\
| Gumtree    | pale eucalyptus-green | Literata    | Junicode botanical sprig fleur    | Day   | Refined  |\n\
| Kingfisher | midnight-navy         | IBM Sans    | Awl Marks pinwheel star lozenge   | Night | Everyday |\n\
\n\
prose after\n";

    let widths_at = |p: &mut TextPipeline, device: &wgpu::Device, queue: &wgpu::Queue, measure: usize| -> Vec<f32> {
        crate::page::set_measure(measure);
        let mut v = view(text, 5, 0);
        v.is_markdown = true;
        p.set_view(&v);
        p.prepare(device, queue, 1200, 800).unwrap();
        let rep = p.tables_report();
        assert_eq!(rep.len(), 1, "one table laid out at measure {measure}");
        assert_eq!(rep[0].cols, 6, "six columns at measure {measure}");
        rep[0].col_widths.clone()
    };

    // A NARROW measure (squeeze/overflow) and a WIDE one (the phrases fit at
    // max-content). The token columns must be byte-identical between them.
    let narrow = widths_at(&mut p, &device, &queue, 44);
    let wide = widths_at(&mut p, &device, &queue, 90);

    for c in [0usize, 4, 5] {
        assert!(
            (narrow[c] - wide[c]).abs() < 0.01,
            "token column {c} is rigid across widths (never shrinks below its word): \
             narrow={:?} wide={:?}",
            narrow, wide
        );
    }
    // The phrase columns absorbed the extra room at the wide measure — they
    // GREW (word-wrapping at the narrow one) rather than the token columns
    // shrinking.
    for c in [1usize, 3] {
        assert!(
            wide[c] > narrow[c] + 1.0,
            "phrase column {c} absorbs the squeeze (grows with room): \
             narrow={:?} wide={:?}",
            narrow, wide
        );
    }

    crate::markdown::set_wysiwyg_on(true);
    crate::page::set_page_on(false);
    crate::page::set_measure(crate::page::DEFAULT_MEASURE);
}

/// THE X-RAY pure caret redirect + pan-to-caret (`xray_col_x` /
/// `xray_pan_for_caret`): the caret on a concealed table row rides the FLOATED
/// source's own glyph advances (minus the pan), and the pan keeps the caret
/// column inside the padded viewport window (the find-field single-line pan),
/// clamped to the row's scrollable range. Pure — no GPU, no font.
#[test]
fn xray_caret_redirect_and_pan_are_pure_and_clamped() {
    let x = crate::render::XrayRow {
        line: 3,
        source: "abc".into(),
        glyph_xs: vec![0.0, 10.0, 25.0, 40.0], // 3 chars, row ends at 40
        top: 0.0,
        height: 20.0,
        pan: 5.0,
    };
    // Redirect: x = glyph_xs[col] − pan; advance = next − this.
    let (gx, adv) = geometry::xray_col_x(&x, 0, 8.0);
    assert!((gx + 5.0).abs() < 1e-3 && (adv - 10.0).abs() < 1e-3, "col 0: {gx} {adv}");
    let (gx, adv) = geometry::xray_col_x(&x, 2, 8.0);
    assert!((gx - 20.0).abs() < 1e-3 && (adv - 15.0).abs() < 1e-3, "col 2: {gx} {adv}");
    // End of row (col == n) falls back to a default char cell.
    let (gx, adv) = geometry::xray_col_x(&x, 3, 8.0);
    assert!((gx - 35.0).abs() < 1e-3 && (adv - 8.0).abs() < 1e-3, "end col: {gx} {adv}");
    // Past the end clamps to n (never panics / reads OOB).
    let (gx, _) = geometry::xray_col_x(&x, 99, 8.0);
    assert!((gx - 35.0).abs() < 1e-3, "past-end clamps to n: {gx}");

    use geometry::xray_pan_for_caret as pan;
    // A row that fits never pans.
    assert_eq!(pan(50.0, 100.0, 200.0, 8.0, 0.0), 0.0);
    // Caret past the right of the window nudges the pan so the caret sits a pad
    // shy of the right edge (clamped to the scrollable max = content − view).
    let p = pan(480.0, 500.0, 200.0, 10.0, 0.0);
    assert!((p - 290.0).abs() < 1e-3, "right-nudge: {p}");
    // Caret already comfortably in the window keeps the previous pan (no jitter).
    let p = pan(150.0, 500.0, 200.0, 10.0, 50.0);
    assert!((p - 50.0).abs() < 1e-3, "in-window keeps prev: {p}");
    // Caret left of the window nudges the pan left to a pad shy of the caret.
    let p = pan(20.0, 500.0, 200.0, 10.0, 100.0);
    assert!((p - 10.0).abs() < 1e-3, "left-nudge: {p}");
    // The pan never exceeds the scrollable max, whatever the caret asks.
    let p = pan(9999.0, 500.0, 200.0, 10.0, 0.0);
    assert!((p - 300.0).abs() < 1e-3, "clamped to content − view: {p}");
}

#[test]
fn table_cell_bold_marker_conceals_and_content_is_bold() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let base = Attrs::new();
    // "**bold**": `*`=0,`*`=1, "bold"=2..6, `*`=6,`*`=7.
    let al = cell_inline_attrs(&base, 20.0, "**bold**");
    // Content shapes in the real BOLD weight (the world's bundled 700 face).
    assert_eq!(al.get_span(2).weight.0, 700, "the cell content is bold weight");
    // The `**` delimiters are concealed (transparent ink) — no literal asterisks.
    assert!(
        matches!(al.get_span(0).color_opt, Some(c) if c.a() == 0),
        "leading `**` marker is concealed (transparent)"
    );
    assert!(
        matches!(al.get_span(7).color_opt, Some(c) if c.a() == 0),
        "trailing `**` marker is concealed (transparent)"
    );
    crate::markdown::set_wysiwyg_on(true);
}

#[test]
fn table_cell_italic_marker_conceals_and_content_is_italic() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let base = Attrs::new();
    // "*x*": `*`=0, "x"=1, `*`=2.
    let al = cell_inline_attrs(&base, 20.0, "*x*");
    assert!(
        matches!(al.get_span(1).style, glyphon::Style::Italic),
        "the cell content is italic"
    );
    assert!(
        matches!(al.get_span(0).color_opt, Some(c) if c.a() == 0),
        "the `*` marker is concealed (transparent) — no literal asterisk"
    );
    crate::markdown::set_wysiwyg_on(true);
}

#[test]
fn table_cell_code_marker_conceals_and_content_is_mono() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let base = Attrs::new();
    // "`x`": backtick=0, "x"=1, backtick=2 (inline code arrives via Event::Code).
    let al = cell_inline_attrs(&base, 20.0, "`x`");
    assert!(
        matches!(al.get_span(1).family, Family::Monospace),
        "the cell content shapes in the mono family"
    );
    assert!(
        matches!(al.get_span(0).color_opt, Some(c) if c.a() == 0),
        "the backtick delimiter is concealed (transparent) — no literal backtick"
    );
    crate::markdown::set_wysiwyg_on(true);
}

#[test]
fn table_cell_plain_text_is_unchanged_from_base() {
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let base = Attrs::new();
    // No inline markup -> `markdown::spans` is empty -> the list is `base`
    // alone: byte-identical to the pre-styling `set_text(cell, base)`.
    let al = cell_inline_attrs(&base, 20.0, "Monaspace Xenon");
    let s = al.get_span(0);
    assert_eq!(s.weight.0, 400, "plain cell keeps the normal weight");
    assert!(matches!(s.style, glyphon::Style::Normal), "plain cell is not italic");
    assert!(!matches!(s.family, Family::Monospace), "plain cell is not mono");
    assert!(s.color_opt.is_none(), "plain cell has no conceal / tint override");
    assert!(s.metrics_opt.is_none(), "plain cell has no zero-width metrics override");
    crate::markdown::set_wysiwyg_on(true);
}

/// WRAP-NOT-CLIP: a too-wide GFM table row wraps its long cell and RESERVES a
/// tall document row (`compute_table_layout` → the shared `image_heights`
/// slot), while a row that fits on one line reserves nothing. This is the
/// mechanism that grows the row so the drawn grid never overlaps the following
/// content — the alternative to the old hard-clip. Drives the real
/// `compute_table_layout` seam over a headless pipeline.
#[test]
fn wide_table_wraps_and_reserves_a_tall_row_while_a_short_row_does_not() {
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let _w = crate::testlock::serial();
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wide_table_wraps...: no wgpu adapter");
        return;
    };
    // A long cell whose natural width far exceeds its column, so it MUST wrap to
    // several lines; the sibling cells + header/short rows all fit on one line.
    let long = "pale eucalyptus-green with a very long description that keeps \
                going well past any single column width so it is forced to wrap \
                onto several lines inside its own narrow column";
    let text = format!(
        "| World | Ground |\n|-------|--------|\n| Short | {long} |\n| Tiny | ok |\n"
    );
    let md_spans = crate::markdown::spans(&text);
    // set_view configures md_enabled + metrics + wrap width for the pipeline.
    p.set_view(&view_md(&text, 0, 0));
    let lh = p.metrics.line_height;
    let heights = p.compute_table_layout(&text, &md_spans);
    // Body row carrying the long cell (doc line 2) reserves a MULTI-line row.
    let wide = heights[2].expect("the wrapping table row reserves a tall row");
    assert!(
        wide > lh * 1.5,
        "the wrapped row grows to several line-heights (got {wide}, lh {lh})"
    );
    // The long cell wraps to MORE lines than any short cell, so its row is the
    // tallest reserved row (a proportionally-squeezed header column may itself
    // wrap a little — that is correct wrap-not-clip too — but never as tall).
    for (li, h) in heights.iter().enumerate() {
        if li != 2 {
            if let Some(other) = h {
                assert!(wide > *other, "the long row (got {wide}) is tallest (line {li}: {other})");
            }
        }
    }
    // The separator (doc line 1) is never a grid row → never a reservation.
    assert!(heights[1].is_none(), "the separator row is not a grid row");

    // CONTROL — a table whose columns all fit reserves NOTHING (byte-identical
    // single-line rows, exactly the pre-round layout).
    let fits = "| a | b |\n|---|---|\n| c | d |\n";
    let fits_spans = crate::markdown::spans(fits);
    p.set_view(&view_md(fits, 0, 0));
    let fh = p.compute_table_layout(fits, &fits_spans);
    assert!(
        fh.iter().all(|h| h.is_none()),
        "a table that fits reserves no tall row (got {fh:?})"
    );
    crate::markdown::set_wysiwyg_on(true);
}
