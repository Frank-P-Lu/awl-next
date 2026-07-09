//! Inline image sizing (fit-to-column, viewport-height cap), the reserved
//! tall row + reveal-on-cursor source, resize-handle arming, and the drawn
//! quad/placeholder tests (real device+queue) -- split out of the former
//! monolithic `render::tests` (2026-07 code-organization pass).

use super::super::*;
use super::{headless_pipeline, view};

/// The pure fit-to-column display-size math: never wider than the column,
/// aspect preserved, an optional width hint replacing the intrinsic width.
/// `max_h = 0.0` disables the viewport-height cap (see the dedicated
/// `image_display_size_caps_at_the_viewport_height` test below for that half).
#[test]
fn image_display_size_fits_to_column_and_preserves_aspect() {
    // 120x48 (aspect 2.5), wide column -> full intrinsic, height = 120/2.5 = 48.
    let (w, h) = super::spans::image_display_size(120, 48, None, 1000.0, 0.0);
    assert!((w - 120.0).abs() < 0.1 && (h - 48.0).abs() < 0.1, "{w}x{h}");
    // Narrow column clamps width AND scales height with it.
    let (w2, h2) = super::spans::image_display_size(120, 48, None, 60.0, 0.0);
    assert!((w2 - 60.0).abs() < 0.1 && (h2 - 24.0).abs() < 0.1, "{w2}x{h2}");
    // A `|300` hint upsizes toward 300 but stays clamped to the column.
    let (w3, _) = super::spans::image_display_size(120, 48, Some(300), 1000.0, 0.0);
    assert!((w3 - 300.0).abs() < 0.1, "hint sets width: {w3}");
    let (w4, _) = super::spans::image_display_size(120, 48, Some(300), 200.0, 0.0);
    assert!((w4 - 200.0).abs() < 0.1, "hint still clamped to column: {w4}");
}

/// The viewport-height cap: a huge-native-size (retina-paste-shaped) image's
/// display HEIGHT never exceeds `max_h`, and its width shrinks PROPORTIONALLY
/// (the aspect never distorts) — the "full-bleed wall" fix.
#[test]
fn image_display_size_caps_at_the_viewport_height() {
    // A tall retina paste: 2241x4000 (aspect ~0.56), a generous wide column so
    // fit-to-column alone would draw it near-full native size.
    let (w, h) = super::spans::image_display_size(2241, 4000, None, 2000.0, 500.0);
    assert!((h - 500.0).abs() < 0.1, "height pinned to the cap: {h}");
    // Width follows the SAME scale factor the height was cut by (500/4000).
    let expected_w = 2241.0 * (500.0 / 4000.0);
    assert!((w - expected_w).abs() < 0.5, "width scales proportionally: {w} vs {expected_w}");
    // A short-and-wide image well under the cap is untouched by it.
    let (w2, h2) = super::spans::image_display_size(1200, 480, None, 2000.0, 500.0);
    assert!((w2 - 1200.0).abs() < 0.1 && (h2 - 480.0).abs() < 0.1, "under the cap, unchanged: {w2}x{h2}");
    // A non-positive max_h disables the cap outright (the "window height not
    // known yet" escape hatch).
    let (w3, h3) = super::spans::image_display_size(2241, 4000, None, 2000.0, 0.0);
    assert!((w3 - 2000.0).abs() < 0.1 && (h3 - 3570.7).abs() < 1.0, "cap disabled: {w3}x{h3}");
}

/// END-TO-END: an `![alt](img.png)` line reserves a TALL row equal to the
/// bundled fixture's fit-to-column DISPLAY height (120x48 -> 48px) via the
/// same variable-row-height machinery headings use; off the caret's line the
/// source CONCEALS (zero-width) and on the caret's line it REVEALS at full
/// width. CAPTION MODEL (re-decided 2026-07-09): the caret's own image row
/// height is UNCHANGED on reveal (stays the image height `h` = 48) — ZERO
/// reflow — and the revealed body-size source renders CENTRED OVER the
/// still-drawn, dimmed image. Fixture: `samples/tiny.png`.
#[test]
fn inline_image_reserves_tall_row_and_reveals_source_on_cursor() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    let prev = crate::markdown::inline_images_on();
    crate::markdown::set_inline_images_on(true);
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping inline_image_reserves_tall_row: no wgpu adapter");
        crate::markdown::set_inline_images_on(prev);
        return;
    };
    // `doc_dir` is None (the `view` helper), so the relative path resolves
    // against the test cwd (the crate root) — `samples/tiny.png` is 120x48.
    let text = "![pic](samples/tiny.png)\nprose here\n";
    // Caret on line 1 (prose): line 0's image source conceals, the tall row shows.
    let mut v = view(text, 1, 0);
    v.is_markdown = true;
    p.set_view(&v);
    let rows0 = p.visual_rows(0);
    let h = rows0[0].line_height;
    assert!((h - 48.0).abs() < 2.0, "image row reserves the 48px display height: {h}");
    let xs = &rows0[0].xs;
    let total = xs.last().copied().unwrap_or(0.0) - xs.first().copied().unwrap_or(0.0);
    assert!(total < 2.0, "off-cursor image source collapses to ~0 width: {total} ({xs:?})");
    let report = p.images_report();
    assert_eq!(report.len(), 1, "one image reported: {report:?}");
    assert!(!report[0].missing, "the bundled fixture reads: {report:?}");
    assert!(!report[0].revealed, "caret off the image line: {report:?}");
    assert!(
        (report[0].display_h - 48.0).abs() < 1.0 && (report[0].display_w - 120.0).abs() < 1.0,
        "report carries the fit-to-column size: {report:?}"
    );

    // Caret ON line 0: the source reveals at full width, but the row height is
    // UNCHANGED (still 48, the image height) — the caption model reflows
    // nothing; the source just renders centred over the dimmed image.
    let mut v0 = view(text, 0, 0);
    v0.is_markdown = true;
    p.set_view(&v0);
    let rows0b = p.visual_rows(0);
    assert!(
        (rows0b[0].line_height - 48.0).abs() < 2.0,
        "CAPTION MODEL: the revealed image row height is UNCHANGED (still 48, no grow): {}",
        rows0b[0].line_height
    );
    let xs2 = &rows0b[0].xs;
    let total2 = xs2.last().copied().unwrap_or(0.0) - xs2.first().copied().unwrap_or(0.0);
    assert!(total2 > 20.0, "on-cursor the image source reveals at full width: {total2}");
    assert!(p.images_report()[0].revealed, "caret on the image line reveals it");
    // CARET SIZE: the caret sizes to the body-size SOURCE (scale 1.0), NOT the
    // tall reserved row — a row-scaled caret balloons to the whole image row.
    // `caret_cell_top` centres the body-height caret in the h-tall row, exactly
    // where cosmic-text centres the source glyphs, so it lands on the caption.
    assert!(
        (p.cursor_scale() - 1.0).abs() < 1e-6,
        "caret on an image line is body-size (scale 1.0), never the tall row: {}",
        p.cursor_scale()
    );

    crate::markdown::set_inline_images_on(prev);
}

/// FIX (2026-07-09): selecting chars on a REVEALED image line must draw a
/// BODY-height selection band — the SAME height the caret draws there — NOT a
/// char-wide × whole-image-height PILLAR (the reported selection bug). The caret
/// was already pinned to the caption text (`cursor_scale` → 1.0 on an image
/// line) but the selection / squiggle row-bands still sized to the tall image
/// row; both now share the ONE owner [`TextPipeline::caret_band_scale`]. Mirrors
/// the caret test above. Fixture: `samples/tiny.png` (120×48 → a 48px row).
#[test]
fn selection_on_image_line_is_body_height_not_the_image_pillar() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    let prev = crate::markdown::inline_images_on();
    crate::markdown::set_inline_images_on(true);
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping selection_on_image_line_is_body_height_not_the_image_pillar: no wgpu adapter");
        crate::markdown::set_inline_images_on(prev);
        return;
    };
    let text = "![pic](samples/tiny.png)\nprose here\n";
    // Caret ON the image line so its source reveals at full width; select 4 chars.
    let mut v = view(text, 0, 4);
    v.is_markdown = true;
    v.selection = Some(((0, 0), (0, 4)));
    p.set_view(&v);
    let img_h = p.visual_rows(0)[0].line_height;
    assert!(img_h > 30.0, "image row reserves the tall display height: {img_h}");
    let sel = p.selection_rects();
    assert!(!sel.is_empty(), "selection on the revealed image line produces a rect: {sel:?}");
    let band_h = sel[0][3];
    let caret_h = p.metrics.caret_h;
    // BODY height (the caret's own band), never the tall image row => no pillar.
    assert!(
        (band_h - caret_h).abs() < 0.5,
        "image-line selection band is body caret height ({caret_h}), not the image pillar: {band_h} (row {img_h})"
    );
    assert!(
        band_h < img_h * 0.6,
        "selection band is far shorter than the image row (no pillar): {band_h} vs {img_h}"
    );
    // And it matches a PROSE line's selection band exactly (the same body anchor).
    let mut vp = view(text, 1, 4);
    vp.is_markdown = true;
    vp.selection = Some(((1, 0), (1, 4)));
    p.set_view(&vp);
    let prose = p.selection_rects();
    assert!(!prose.is_empty(), "prose-line selection produces a rect: {prose:?}");
    assert!(
        (prose[0][3] - band_h).abs() < 0.5,
        "image-line band == prose-line band (both body caret height): {} vs {band_h}",
        prose[0][3]
    );
    crate::markdown::set_inline_images_on(prev);
}

/// CAPTION MODEL (settled `df773ba`): the image is DRAWN on every line now —
/// caret-on-line only floats the raw source as a caption overlay, it no longer
/// hides the drawn image — so the resize handles must arm REGARDLESS of caret
/// position. This supersedes the old images-v2 reveal-hides-the-image model's
/// `im.revealed` exclusion in `image_hit_rects` (dead code once the caption
/// model landed, since a revealed image is a drawn image too). Fixture:
/// `samples/tiny.png`.
#[test]
fn revealed_images_still_arm_resize_handles() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    let prev = crate::markdown::inline_images_on();
    crate::markdown::set_inline_images_on(true);
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping revealed_images_still_arm_resize_handles: no wgpu adapter");
        crate::markdown::set_inline_images_on(prev);
        return;
    };
    let text = "![pic](samples/tiny.png)\nprose here\n";
    // Caret OFF the image line: exactly one hit rect, as expected off-reveal.
    let mut v_off = view(text, 1, 0);
    v_off.is_markdown = true;
    p.set_view(&v_off);
    let rects_off = p.image_hit_rects();
    assert_eq!(rects_off.len(), 1, "off-cursor: the drawn image arms a handle target: {rects_off:?}");

    // Caret ON the image line (the image REVEALS its source as a caption): the
    // handle target is STILL present — same byte range, same on-screen rect —
    // since the image itself is still drawn underneath the caption.
    let mut v_on = view(text, 0, 0);
    v_on.is_markdown = true;
    p.set_view(&v_on);
    assert!(p.images_report()[0].revealed, "caret on the image line reveals it");
    let rects_on = p.image_hit_rects();
    assert_eq!(
        rects_on.len(),
        1,
        "REVEALED: the handle target survives caret-on-line (the caption model draws the image regardless): {rects_on:?}"
    );
    assert_eq!(rects_off[0].0, rects_on[0].0, "same image byte range either way");
    crate::markdown::set_inline_images_on(prev);
}

/// IMAGES OFF: the `![alt](path)` line keeps a NORMAL-height row, emits no
/// image report, and its source renders as plain full-width text — byte-
/// identical to the pre-feature editor.
#[test]
fn inline_images_off_keeps_normal_row_and_no_report() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    let prev = crate::markdown::inline_images_on();
    crate::markdown::set_inline_images_on(false);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping inline_images_off: no wgpu adapter");
        crate::markdown::set_inline_images_on(prev);
        return;
    };
    let text = "![pic](samples/tiny.png)\nprose\n";
    let mut v = view(text, 1, 0);
    v.is_markdown = true;
    p.set_view(&v);
    let rows0 = p.visual_rows(0);
    assert!(
        (rows0[0].line_height - p.metrics.line_height).abs() < 1.0,
        "images OFF: the image line keeps a normal-height row: {}",
        rows0[0].line_height
    );
    assert!(p.images_report().is_empty(), "images OFF: nothing reported");
    let xs = &rows0[0].xs;
    let total = xs.last().copied().unwrap_or(0.0) - xs.first().copied().unwrap_or(0.0);
    assert!(total > 20.0, "images OFF: source renders as plain full-width text: {total}");
    crate::markdown::set_inline_images_on(prev);
}

/// WYSIWYG OFF byte-identity guard: with inline images ON but WYSIWYG OFF there
/// is no reveal/conceal model at all — the image row is exactly the image height
/// `h` (48, same as the caption model) AND the source shows UNCONCEALED at full
/// width whether or not the caret is on it. The caption model never grows the
/// row, so `h` matches on-caret too; the distinguishing off-state fact is the
/// unconcealed source. Fixture: `samples/tiny.png`.
#[test]
fn wysiwyg_off_image_line_does_not_grow_on_reveal() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    let prev = crate::markdown::inline_images_on();
    let prevw = crate::markdown::wysiwyg_on();
    crate::markdown::set_inline_images_on(true);
    crate::markdown::set_wysiwyg_on(false);
    let restore = || {
        crate::markdown::set_inline_images_on(prev);
        crate::markdown::set_wysiwyg_on(prevw);
    };
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wysiwyg_off_image_line: no wgpu adapter");
        restore();
        return;
    };
    let text = "![pic](samples/tiny.png)\nprose\n";
    // Caret OFF the image line (line 1): with WYSIWYG off the source is NOT
    // concealed — it shows at full width even though the caret is elsewhere.
    let mut voff = view(text, 1, 0);
    voff.is_markdown = true;
    p.set_view(&voff);
    let rows_off = p.visual_rows(0);
    assert!(
        (rows_off[0].line_height - 48.0).abs() < 2.0,
        "WYSIWYG off: the image row is h (48): {}",
        rows_off[0].line_height
    );
    let xs_off = &rows_off[0].xs;
    let total_off = xs_off.last().copied().unwrap_or(0.0) - xs_off.first().copied().unwrap_or(0.0);
    assert!(
        total_off > 20.0,
        "WYSIWYG off: the source shows UNCONCEALED (full width) off the caret line: {total_off}"
    );
    // Caret ON the image line 0 — the row is still h (48), never grows.
    let mut v = view(text, 0, 0);
    v.is_markdown = true;
    p.set_view(&v);
    let rows0 = p.visual_rows(0);
    assert!(
        (rows0[0].line_height - 48.0).abs() < 2.0,
        "WYSIWYG off: the caret's image row stays h (48), never grows: {}",
        rows0[0].line_height
    );
    restore();
}

/// HIT-TEST across a REVEALED image row (the `h`-tall row, source shown at body
/// size CENTRED in it — the caption model): a full-width x sweep at the row's
/// vertical centre always resolves to logical line 0 and an in-bounds column,
/// AND the sweep still discriminates (more than one distinct column), so the
/// revealed caption stays clickable. Fixture: `samples/tiny.png`.
#[test]
fn revealed_image_row_hit_test_stays_in_bounds() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    let prev = crate::markdown::inline_images_on();
    let prevw = crate::markdown::wysiwyg_on();
    crate::markdown::set_inline_images_on(true);
    crate::markdown::set_wysiwyg_on(true);
    let restore = || {
        crate::markdown::set_inline_images_on(prev);
        crate::markdown::set_wysiwyg_on(prevw);
    };
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping revealed_image_row_hit_test: no wgpu adapter");
        restore();
        return;
    };
    let src = "![pic](samples/tiny.png)";
    let text = format!("{src}\nprose here\n");
    let char_count = src.chars().count();
    // Caret ON line 0: the source reveals in the grown row.
    let mut v0 = view(&text, 0, 0);
    v0.is_markdown = true;
    p.set_view(&v0);
    let rows0 = p.visual_rows(0);
    let row_h = rows0[0].line_height;
    // Vertical centre of the revealed row (where cosmic-text centres the source).
    let py = p.line_ornament_top(0) + row_h * 0.5;
    let left = p.text_left();
    let wrap = p.text_wrap_width();
    let mut cols = std::collections::BTreeSet::new();
    let steps = 48;
    for i in 0..=steps {
        let px = left + wrap * (i as f32 / steps as f32);
        let (line, col) = p.hit_test(px, py, 0);
        assert_eq!(line, 0, "every click on the revealed image row lands on line 0");
        assert!(
            col <= char_count,
            "hit column {col} stays within the source's {char_count} chars"
        );
        cols.insert(col);
    }
    assert!(
        cols.len() > 3,
        "the x sweep discriminates columns on the revealed source: {cols:?}"
    );
    restore();
}

/// A headless pipeline PLUS its device/queue, so a test can drive the full
/// `prepare` frame (the image draw's instance counts are only set there). `None`
/// on a GPU-less machine (skip).
fn headless_pipeline_dq() -> Option<(wgpu::Device, wgpu::Queue, TextPipeline)> {
    pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl image-draw test device"),
                ..Default::default()
            })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let mut p =
            TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
        p.set_size(1200.0, 800.0);
        Some((device, queue, p))
    })
}

/// GPU DRAW: an OFF-CURSOR image on a visible line decodes the bundled fixture
/// and draws exactly ONE image quad (no placeholder) and NO caption scrim;
/// moving the caret ONTO the image line REVEALS the source but the image STAYS
/// DRAWN (dimmed, UNMOVED — the caption model, source centred over it) and a
/// caption SCRIM band appears behind the revealed source. Fixture:
/// `samples/tiny.png`.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn inline_image_off_cursor_draws_one_quad_and_stays_drawn_when_revealed() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    if std::fs::metadata("samples/tiny.png").is_err() {
        eprintln!("skipping: samples/tiny.png fixture not present");
        return;
    }
    let prev = crate::markdown::inline_images_on();
    let prevw = crate::markdown::wysiwyg_on();
    crate::markdown::set_inline_images_on(true);
    crate::markdown::set_wysiwyg_on(true);
    let restore = || {
        crate::markdown::set_inline_images_on(prev);
        crate::markdown::set_wysiwyg_on(prevw);
    };
    let Some((device, queue, mut p)) = headless_pipeline_dq() else {
        eprintln!("skipping inline_image_off_cursor_draws_one_quad: no wgpu adapter");
        restore();
        return;
    };
    let text = "![pic](samples/tiny.png)\nprose here\n";
    // Caret on line 1 (prose) — the image on line 0 is off-cursor + visible.
    let mut v = view(text, 1, 0);
    v.is_markdown = true;
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert_eq!(p.image_pipeline.instance_count(), 1, "one image quad drawn off-cursor");
    assert_eq!(
        p.image_placeholder_pipeline.instance_count(),
        0,
        "a readable fixture draws NO placeholder"
    );
    assert_eq!(
        p.image_scrim_pipeline.instance_count(),
        0,
        "off-cursor: no caption scrim (the source is concealed)"
    );

    // Caret ON the image line — the source reveals, but the image STAYS DRAWN
    // (dimmed, UNMOVED — the caption model): still one quad. A caption SCRIM
    // band now backs the revealed source (at least one band; a wrapped source
    // could produce more).
    let mut v0 = view(text, 0, 0);
    v0.is_markdown = true;
    p.set_view(&v0);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert_eq!(
        p.image_pipeline.instance_count(),
        1,
        "the image stays drawn (dimmed) when its source line is revealed"
    );
    assert!(
        p.image_scrim_pipeline.instance_count() >= 1,
        "revealed: a caption scrim band backs the source: {}",
        p.image_scrim_pipeline.instance_count()
    );
    restore();
}

/// GPU DRAW: a MISSING-file image draws the calm rounded PLACEHOLDER quad (one),
/// and NO image quad — a missing image is a calm state, never an error.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn inline_image_missing_file_draws_placeholder_not_quad() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    let prev = crate::markdown::inline_images_on();
    let prevw = crate::markdown::wysiwyg_on();
    crate::markdown::set_inline_images_on(true);
    crate::markdown::set_wysiwyg_on(true);
    let restore = || {
        crate::markdown::set_inline_images_on(prev);
        crate::markdown::set_wysiwyg_on(prevw);
    };
    let Some((device, queue, mut p)) = headless_pipeline_dq() else {
        eprintln!("skipping inline_image_missing_file_draws_placeholder: no wgpu adapter");
        restore();
        return;
    };
    let text = "![a caption](does-not-exist-awl.png)\nprose\n";
    let mut v = view(text, 1, 0);
    v.is_markdown = true;
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    let report = p.images_report();
    assert_eq!(report.len(), 1, "one image reported: {report:?}");
    assert!(report[0].missing, "the absent file is reported missing: {report:?}");
    assert_eq!(
        p.image_placeholder_pipeline.instance_count(),
        1,
        "the missing image draws exactly one placeholder card"
    );
    assert_eq!(
        p.image_pipeline.instance_count(),
        0,
        "a missing image draws NO textured quad"
    );
    restore();
}

/// A NON-IMAGE markdown buffer draws neither an image quad nor a placeholder —
/// byte-identical to the pre-feature editor at the GPU layer.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn no_image_buffer_draws_neither_quad_nor_placeholder() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    let prev = crate::markdown::inline_images_on();
    crate::markdown::set_inline_images_on(true);
    let Some((device, queue, mut p)) = headless_pipeline_dq() else {
        eprintln!("skipping no_image_buffer_draws_neither: no wgpu adapter");
        crate::markdown::set_inline_images_on(prev);
        return;
    };
    let mut v = view("# heading\n\nplain prose only\n", 0, 0);
    v.is_markdown = true;
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert_eq!(p.image_pipeline.instance_count(), 0, "no images: no quad");
    assert_eq!(
        p.image_placeholder_pipeline.instance_count(),
        0,
        "no images: no placeholder"
    );
    crate::markdown::set_inline_images_on(prev);
}
