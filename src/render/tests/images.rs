//! Inline image sizing (fit-to-column, viewport-height cap), the reserved
//! tall row + reveal-on-cursor source, resize-handle arming, and the drawn
//! quad/placeholder tests (real device+queue) -- split out of the former
//! monolithic `render::tests` (2026-07 code-organization pass).

#[cfg(not(target_arch = "wasm32"))]
use super::super::*;
use super::super::LINE_HEIGHT;
use super::{headless_pipeline, view};

/// The pure fit-to-column display-size math: never wider than the column,
/// aspect preserved, an optional width hint replacing the intrinsic width.
/// `max_h = 0.0` disables the viewport-height cap (see the dedicated
/// `image_display_size_caps_at_the_viewport_height` test below for that half).
#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
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

/// SELECTION REVEAL REGRESSION (item 16 follow-up, defect found in adversarial
/// verify of `0828112`): a SELECTION touching a BARE image's own line — the
/// caret itself parked elsewhere — must reveal the source EXACTLY like the
/// caret landing on the line does (mirrors the test above): the raw markup
/// un-conceals, `images_report().revealed` flips true, and the reserved row
/// height stays the image's own `dh` unchanged (never a second, taller
/// reservation stacked on top — the pre-fix shape would have left `revealed`
/// caret-only while the markup already revealed, i.e. an un-dimmed image
/// drawn under now-visible raw source). Fixture: `samples/tiny.png`.
#[test]
fn inline_image_reveals_under_selection_caret_elsewhere() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    let prev = crate::markdown::inline_images_on();
    crate::markdown::set_inline_images_on(true);
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping inline_image_reveals_under_selection_caret_elsewhere: no wgpu adapter");
        crate::markdown::set_inline_images_on(prev);
        return;
    };
    let text = "![pic](samples/tiny.png)\nprose here\n";
    // Caret on line 1 (prose), no selection: off-cursor, concealed, unrevealed.
    let mut off = view(text, 1, 0);
    off.is_markdown = true;
    p.set_view(&off);
    assert!(p.concealed_at(0, 0), "no selection: the image source stays concealed");
    assert!(!p.images_report()[0].revealed, "no selection: not revealed");

    // Caret STILL on line 1 (prose) — it never lands on the image line at all —
    // but a SELECTION spans from line 0 (the image) through line 1.
    let mut sel = view(text, 1, 4);
    sel.is_markdown = true;
    sel.selection = Some(((0, 0), (1, 4)));
    p.set_view(&sel);
    assert!(
        !p.concealed_at(0, 0),
        "a selection touching the image line reveals its raw source, caret or not"
    );
    assert!(
        p.images_report()[0].revealed,
        "SELECTION REVEAL: images_report().revealed is selection-aware, not caret-only"
    );
    // No double reservation: the row height is still exactly the image's own
    // display height (48px) — the reveal parks it, it never grows/duplicates
    // the row on top of the revealed source.
    let h = p.visual_rows(0)[0].line_height;
    assert!(
        (h - 48.0).abs() < 2.0,
        "selection reveal parks at the image's own row height, no double reservation: {h}"
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

/// ITEM 27 REGRESSION: `image_hit_rects` reads the reveal flag through the
/// always-fresh `images_report()` override, NOT the stored `image_report`'s
/// stale field. A pure CARET move onto an off-cursor MIXED image line (caption
/// text before the image on the same line) runs `refresh_rule_conceal`, which
/// flips that line's reservation (`image_force`/`image_heights` → `None`)
/// WITHOUT re-running `compute_image_layout` — so the STORED `image_report`
/// still holds frame-1's `revealed: false`. Under that stale flag the skip in
/// `image_hit_rects` (`revealed && !reserved`) read `false && true == false` and
/// armed a resize handle at a now-undefined position for the parked line. Read
/// through the FRESH override (`revealed == true` on the caret line) the skip
/// fires (`true && true`) and the handle correctly drops. Zero selection — this
/// is orthogonal to the selection-reveal path, a pure caret move. MUTATION
/// CHECK: reverting `image_hit_rects` to `self.image_report.borrow()` re-arms the
/// stale handle and this fails (`rects_on.len()` reverts to 1). Fixture:
/// `samples/tiny.png`.
#[test]
fn image_hit_rects_use_fresh_reveal_on_pure_caret_move_onto_mixed_line() {
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
        eprintln!(
            "skipping image_hit_rects_use_fresh_reveal_on_pure_caret_move_onto_mixed_line: no wgpu adapter"
        );
        restore();
        return;
    };
    // A MIXED image line: caption text precedes the image on the SAME line, so a
    // reveal collapses the reservation entirely (unlike a BARE image line, which
    // stays drawn + reserved under its floating caption and keeps arming).
    let text = "a caption before the pic ![pic](samples/tiny.png)\nprose here\n";

    // Frame 1: caret on line 1 (OFF the image line) — the mixed line is
    // off-cursor, reserves its forced trailing row, and arms exactly one handle.
    let mut v_off = view(text, 1, 0);
    v_off.is_markdown = true;
    p.set_view(&v_off);
    assert!(!p.images_report()[0].revealed, "sanity: off-cursor, not revealed");
    let rects_off = p.image_hit_rects();
    assert_eq!(rects_off.len(), 1, "off-cursor mixed line arms one handle: {rects_off:?}");

    // Frame 2: caret MOVES onto line 0 (the mixed image line), ZERO selection. A
    // pure caret move — no reshape — so the STORED report still holds frame-1's
    // `revealed: false`, while `refresh_rule_conceal` has already un-reserved the
    // row. The FRESH override reports `revealed: true`.
    let reshape_before = p.reshape_count;
    let mut v_on = view(text, 0, 0);
    v_on.is_markdown = true;
    p.set_view(&v_on);
    assert_eq!(
        p.reshape_count, reshape_before,
        "sanity: a pure caret move on unchanged text does not reshape"
    );
    assert!(
        p.images_report()[0].revealed,
        "caret on the mixed line: the fresh override reveals it"
    );
    // THE FIX: with the fresh reveal, the parked mixed line arms NO handle. Under
    // the stale-flag bug this returned 1 (a handle at an undefined position).
    let rects_on = p.image_hit_rects();
    assert!(
        rects_on.is_empty(),
        "FRESH reveal parks the mixed line: no resize handle armed (the stale flag armed one): {rects_on:?}"
    );
    restore();
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
/// on a GPU-less machine (skip). Native-only: its three callers below are all
/// `#[cfg(not(target_arch = "wasm32"))]` GPU-draw tests.
#[cfg(not(target_arch = "wasm32"))]
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

/// GPU DRAW — SELECTION REVEAL REGRESSION (item 16 follow-up, the defect this
/// round fixes): the SAME dimmed + scrim draw the caret-revealed test above
/// asserts, reached through a SELECTION instead — the caret stays parked on
/// the prose line; a selection spans the image line. BEFORE the fix
/// `images_report().revealed` stayed caret-only, so this exact frame drew the
/// image at FULL brightness (alpha 1.0, no scrim quad) directly under the
/// already-revealed raw `![alt](path)` source (item 16 already made the
/// MARKUP selection-aware) — an un-dimmed, unscrimmed, illegible
/// double-render. MUTATION CHECK: reverting `images_report`'s `revealed` (or
/// `compute_image_layout`'s `revealed_now`) back to `r.line ==
/// self.cursor_line` alone fails this test (scrim count drops to 0). Fixture:
/// `samples/tiny.png`.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn inline_image_selection_reveal_draws_dimmed_with_scrim_not_full_bright() {
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
        eprintln!(
            "skipping inline_image_selection_reveal_draws_dimmed_with_scrim_not_full_bright: no wgpu adapter"
        );
        restore();
        return;
    };
    let text = "![pic](samples/tiny.png)\nprose here\n";
    // Caret on line 1 (prose) — a SELECTION spans line 0 (the image) through
    // line 1. The caret itself never touches the image line.
    let mut v = view(text, 1, 4);
    v.is_markdown = true;
    v.selection = Some(((0, 0), (1, 4)));
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert!(
        p.images_report()[0].revealed,
        "the selection touches the image line: revealed"
    );
    assert_eq!(
        p.image_pipeline.instance_count(),
        1,
        "SELECTION REVEAL: still exactly one image quad drawn (no double geometry)"
    );
    assert!(
        p.image_scrim_pipeline.instance_count() >= 1,
        "SELECTION REVEAL: a caption scrim band backs the source, exactly like the caret-revealed case (never a bare full-brightness draw): {}",
        p.image_scrim_pipeline.instance_count()
    );
    restore();
}

/// GPU DRAW (item 5 rework): the MIXED-line counterpart to the bare-line test
/// above — OFF-cursor the image draws its one quad (at the forced trailing
/// row's own top, per `TextPipeline::image_draw_top`); ON-cursor (revealed,
/// its raw source wrapping as plain text — see
/// `mixed_list_image_reveal_wraps_as_plain_text_and_parks_the_image`) it draws
/// NO quad at all — the caption model's "stays drawn, dimmed" only holds for a
/// BARE line's fixed single-row geometry, not a mixed line's wrap-dependent
/// reveal (see `compute_image_layout`'s doc comment for why). Fixture:
/// `samples/photo.png`.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn mixed_list_image_draws_off_cursor_and_parks_when_revealed() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    if std::fs::metadata("samples/photo.png").is_err() {
        eprintln!("skipping: samples/photo.png fixture not present");
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
        eprintln!("skipping mixed_list_image_draws_off_cursor_and_parks_when_revealed: no wgpu adapter");
        restore();
        return;
    };
    let text = "- a caption sits before the image on this very same list line ![alt|300](samples/photo.png)\nafter\n";
    // Caret on line 1 ("after") — the image line is off-cursor.
    let mut v = view(text, 1, 0);
    v.is_markdown = true;
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert_eq!(
        p.image_pipeline.instance_count(),
        1,
        "off-cursor: the mixed line's image draws its one quad"
    );
    // MARKER-STRAND REGRESSION GUARD (larger fixture — `photo.png` fits to a
    // `dh` in the hundreds of px, so the prior round's `base_lh + 2*dh` bug
    // would have produced the dramatic real-fixture void this item reworked):
    // the caption row still stays at plain `base_lh`, and the image draws with
    // NO gap directly below it.
    let base_lh = p.metrics.line_height;
    let rows0 = p.visual_rows(0);
    assert!(
        (rows0[0].line_height - base_lh).abs() < 1.0,
        "the caption row is untouched even with a large dh: {} vs base_lh {base_lh}",
        rows0[0].line_height
    );
    let dh = p.images_report()[0].display_h;
    assert!(dh > base_lh * 3.0, "sanity: this fixture's dh is genuinely large: {dh}");
    let rects = p.image_hit_rects();
    assert_eq!(rects.len(), 1, "one image hit rect off-cursor: {rects:?}");
    let row0_bottom = p.line_ornament_top(0) + rows0[0].line_height;
    assert!(
        (rects[0].1[1] - row0_bottom).abs() < 1.0,
        "the image draws immediately below the caption row, no void: img_top={} row0_bottom={row0_bottom}",
        rects[0].1[1]
    );

    // Caret ON the image line: the reveal wraps as plain text; the image parks.
    let mut v0 = view(text, 0, 0);
    v0.is_markdown = true;
    p.set_view(&v0);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert_eq!(
        p.image_pipeline.instance_count(),
        0,
        "revealed: the mixed line's image draws NO quad (parked for this one frame)"
    );
    restore();
}

/// GPU DRAW — SELECTION REVEAL REGRESSION, item-5 INTERACTION (item 16
/// follow-up): the MIXED-line counterpart to the bare-line selection test
/// above, reached via a PURE selection change on ALREADY-SHAPED text — no
/// caret move, no edit. `set_view`'s composed-text compare skips the reshape
/// entirely on the second frame here (asserted below via `reshape_count`), so
/// this exercises `refresh_rule_conceal`'s forced-trailing-row rescan ALONE,
/// never `compute_image_layout`. That rescan re-derives `image_force` on
/// every caret/selection tick; if it only widened `compute_image_layout`'s own
/// gate and kept its OWN `want` test caret-only, it would immediately
/// re-force the row open again on this very tick, silently undoing the park a
/// selection-only interaction that never triggers a reshape depends on
/// entirely. MUTATION CHECK: reverting `refresh_rule_conceal`'s `revealed_now`
/// back to `li == cursor_line` alone re-forces the row and fails this test
/// (instance count reverts to 1). Fixture: `samples/photo.png`.
///
/// GAP CLOSED BY ITEM 27 (found probing this exact scenario, then fixed): the
/// same reveal-staleness reaches `image_hit_rects` (drag-resize handle arming,
/// live pointer-only). It once read the STORED `image_report`'s `revealed`
/// field directly, so on a pure-selection (or a pure CARET) tick — no reshape,
/// so `compute_image_layout` never re-runs — it held frame 1's stale
/// `revealed: false` and armed a handle at a garbage position for the
/// now-parked line. It now reads through `images_report()`'s always-fresh
/// override; the dedicated regression is
/// `image_hit_rects_use_fresh_reveal_on_pure_caret_move_onto_mixed_line`.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn mixed_list_image_parks_under_a_pure_selection_change_no_reshape() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    if std::fs::metadata("samples/photo.png").is_err() {
        eprintln!("skipping: samples/photo.png fixture not present");
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
        eprintln!(
            "skipping mixed_list_image_parks_under_a_pure_selection_change_no_reshape: no wgpu adapter"
        );
        restore();
        return;
    };
    let text = "- a caption sits before the image on this very same list line ![alt|300](samples/photo.png)\nafter\n";
    // First frame: caret on line 1, no selection — off-cursor, one quad drawn.
    // A fresh pipeline's `shaped_key` is `None`, so this ALSO forces the
    // initial reshape (`compute_image_layout` runs once, populating
    // `image_force` for the off-cursor mixed line).
    let mut v0 = view(text, 1, 0);
    v0.is_markdown = true;
    p.set_view(&v0);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert_eq!(p.image_pipeline.instance_count(), 1, "off-cursor: one quad drawn");
    let reshape_before = p.reshape_count;

    // Second frame: SAME TEXT, caret STILL on line 1 — only a SELECTION is
    // added, spanning line 0 (the image line). Confirm no reshape ran (the
    // park below can only be coming from `refresh_rule_conceal`).
    let mut v1 = view(text, 1, 0);
    v1.is_markdown = true;
    v1.selection = Some(((0, 0), (0, 10)));
    p.set_view(&v1);
    assert_eq!(
        p.reshape_count, reshape_before,
        "a pure selection change on unchanged text does not reshape"
    );
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert_eq!(
        p.image_pipeline.instance_count(),
        0,
        "SELECTION REVEAL, no reshape: the mixed line's image still parks (0 quads)"
    );
    assert!(
        p.images_report()[0].revealed,
        "the selection touches the mixed image line: revealed"
    );
    restore();
}

/// GPU DRAW — SELECTION REVEAL REGRESSION, `compute_image_layout` site (item 16
/// follow-up): the RESHAPE-time counterpart to the pure-selection test above —
/// here the selection is present on the VERY FIRST `set_view` (a fresh
/// pipeline's `shaped_key` is `None`, so this call always reshapes and runs
/// `compute_image_layout` itself), so a MIXED line the selection touches must
/// never even reserve the forced trailing row in the first place — the same
/// outcome as the caret-revealed case, reached the OTHER way this feature can
/// fire (open a doc / edit elsewhere with a stale selection already sitting on
/// an image line). MUTATION CHECK: reverting `compute_image_layout`'s
/// `revealed_now` back to `line == cursor_line` alone re-reserves the row and
/// fails this test (instance count reverts to 1). Fixture: `samples/photo.png`.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn mixed_list_image_parks_when_selection_present_at_first_reshape() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    if std::fs::metadata("samples/photo.png").is_err() {
        eprintln!("skipping: samples/photo.png fixture not present");
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
        eprintln!(
            "skipping mixed_list_image_parks_when_selection_present_at_first_reshape: no wgpu adapter"
        );
        restore();
        return;
    };
    let text = "- a caption sits before the image on this very same list line ![alt|300](samples/photo.png)\nafter\n";
    // The VERY FIRST `set_view` call already carries a selection spanning the
    // image line, caret parked on line 1 — this call always reshapes (a fresh
    // `shaped_key` is `None`), so `compute_image_layout` itself must decide the
    // park, not `refresh_rule_conceal`.
    let mut v = view(text, 1, 0);
    v.is_markdown = true;
    v.selection = Some(((0, 0), (0, 10)));
    p.set_view(&v);
    assert!(p.reshape_count > 0, "sanity: the first set_view reshapes");
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert_eq!(
        p.image_pipeline.instance_count(),
        0,
        "RESHAPE-time selection reveal: the mixed line's image parks (0 quads), never reserved"
    );
    assert!(
        p.images_report()[0].revealed,
        "the selection touches the mixed image line at reshape time: revealed"
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

/// NESTED-LIST IMAGE CAPTION: an image nested inside a list item (`  - ![alt|W](p)`,
/// two levels of indent) reports its ALT TEXT + resolved LINE + destination PATH
/// exactly like a top-level image — the caption the placeholder card draws (see
/// `layers::prepare_images`'s `Missing.alt` label) is driven straight off this
/// report, so a correct report here IS the caption rendering correctly. Guards the
/// "the caption never renders for a nested-list image" report: it already reads
/// right off `images_report()` (which is untouched by the list-nesting depth — an
/// image's `ConcealMarkup(Image)` span + `parse_image_source` never look at the
/// preceding marker at all), so this pins the OUTCOME as a regression guard.
///
/// Native-only: `ImageReport::alt` (and the whole `images_report()` population in
/// `compute_image_layout`) is `#[cfg(not(target_arch = "wasm32"))]` — image file
/// headers are read on the native filesystem, never in the browser build, so the
/// report is always empty on wasm. The gate matches that design (and its sibling
/// `no_image_buffer_draws_neither_quad_nor_placeholder` below).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn nested_list_image_reports_alt_caption_and_line() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    let prev = crate::markdown::inline_images_on();
    crate::markdown::set_inline_images_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping nested_list_image_reports_alt_caption_and_line: no wgpu adapter");
        crate::markdown::set_inline_images_on(prev);
        return;
    };
    let text = "- Item one\n  - ![a test caption|400](assets/does-not-exist.png)\n- Item two\n";
    let mut v = view(text, 0, 0);
    v.is_markdown = true;
    p.set_view(&v);
    let report = p.images_report();
    assert_eq!(report.len(), 1, "one nested-list image reported: {report:?}");
    let im = &report[0];
    assert_eq!(im.alt, "a test caption", "the caption text (alt, hint stripped) survives nesting: {im:?}");
    assert_eq!(im.path, "assets/does-not-exist.png", "the path is unaffected by nesting: {im:?}");
    assert_eq!(im.line, 1, "the image resolves to its OWN (nested) line: {im:?}");
    assert_eq!(im.width_hint, Some(400), "the |NNN width hint still parses: {im:?}");
    crate::markdown::set_inline_images_on(prev);
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

/// ITEM 5 REWORK REGRESSION GUARD — the forcing measurement MUST shape its
/// isolated probe in the REAL document face (`TextPipeline::doc_attrs`), not a
/// generic fallback. Caught live: measuring the "iA Writer Quattro S" world's
/// (Mopoke) caption prefix with a blank `Attrs::new()` under-measured it
/// against the font it ACTUALLY renders in, so the forcing glyph fired while
/// the caption's own natural wrap still had a row left — stranding the image
/// mid-text again (the exact bug this whole round fixed, just relocated). This
/// combines the three conditions that exposed it: a non-default WORLD (its own
/// display face, likely wider/narrower than the fallback), PAGE MODE at a
/// REALISTIC measure (not the wide 1200px unpaged test default), and a caption
/// long enough to wrap on ITS OWN even before the image markup — so the
/// forcing measurement's font MUST match reality or this fails.
#[test]
fn mixed_list_image_forcing_measures_in_the_real_world_font_under_page_mode() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    if std::fs::metadata("samples/photo.png").is_err() {
        eprintln!("skipping: samples/photo.png fixture not present");
        return;
    }
    let prev = crate::markdown::inline_images_on();
    crate::markdown::set_inline_images_on(true);
    crate::markdown::set_wysiwyg_on(true);
    let prev_page = crate::page::page_on();
    let prev_measure = crate::page::measure();
    crate::page::set_page_on(true);
    crate::page::set_measure(105);
    let prev_theme = crate::theme::active().name;
    crate::theme::set_active_by_name("Mopoke");
    let restore = || {
        crate::page::set_page_on(prev_page);
        crate::page::set_measure(prev_measure);
        crate::theme::set_active_by_name(&prev_theme);
        crate::markdown::set_inline_images_on(prev);
    };
    let Some(mut p) = headless_pipeline() else {
        eprintln!(
            "skipping mixed_list_image_forcing_measures_in_the_real_world_font_under_page_mode: no wgpu adapter"
        );
        restore();
        return;
    };
    let text = "# Mixed list image\n\n- a caption sits before the image on this very same list line ![a soft gradient with a pale sun|300](samples/photo.png)\n- item two, plain text, no image at all\n- item three\n";
    let mut v = view(text, 0, 0);
    v.is_markdown = true;
    p.set_view(&v);
    let dh = p.images_report()[0].display_h;
    let base_lh = p.metrics.line_height;

    // The caption genuinely wraps on its own under these conditions (the
    // scenario the bug needed) — sanity-check the setup itself is exercising
    // the multi-row path, not silently degenerating to a single row.
    let rows = p.visual_rows(2);
    assert!(
        rows.len() >= 2,
        "sanity: the caption wraps on its own at this width/font: {} row(s)",
        rows.len()
    );
    // Every row of the caption's own wrap stays at plain base_lh — the forcing
    // glyph never lands ON one of them (which would inflate it to `dh`).
    for (i, r) in rows[..rows.len() - 1].iter().enumerate() {
        assert!(
            (r.line_height - base_lh).abs() < 1.0,
            "caption row {i} stays at base_lh, the forcing glyph didn't land here: {} vs base_lh {base_lh}",
            r.line_height
        );
    }
    // The LAST row is the forcing row, sized to dh.
    let last = rows.last().unwrap();
    assert!(
        (last.line_height - dh).abs() < 1.0,
        "the trailing forcing row is dh tall: {} vs dh {dh}",
        last.line_height
    );

    // NO VOID: the image draws immediately after the caption's OWN last real
    // row (not after row 0, which was the live bug — the image floated up
    // over the caption's second wrapped row instead of below it).
    let rects = p.image_hit_rects();
    assert_eq!(rects.len(), 1, "one image hit rect: {rects:?}");
    let caption_last_row_bottom = p.line_ornament_top(2) + (last.line_top - rows[0].line_top);
    assert!(
        (rects[0].1[1] - caption_last_row_bottom).abs() < 1.0,
        "image draws directly below the caption's OWN last wrapped row, no void: img_top={} caption_last_row_bottom={caption_last_row_bottom}",
        rects[0].1[1]
    );

    restore();
}

/// ITEM 5 REWORK (2026-07-22) — LIST ITEM WITH TEXT AND AN IMAGE
/// (`- caption text ![pic](p)`): the PRIOR round's `base_lh + 2*dh` whole-row
/// inflation is REJECTED — cosmic-text's unconditional row-centering rendered
/// the caption glyphs ~`dh` px below its own list marker (a real fixture
/// produced a ~200px void with nothing connecting marker to caption). The fix:
/// the marker+caption row is NEVER touched (stays exactly `base_lh`, so the
/// marker draws immediately adjacent to its own caption — same row, same
/// normal metrics, like any other list item), and the image instead gets a
/// GENUINE second cosmic-text visual row of the SAME logical line (a forced
/// `Wrap::WordOrGlyph` break via a large `letter_spacing` on the concealed
/// image markup's first byte — see `TextPipeline::image_force`'s field doc),
/// sized to `dh`, directly after it — real layout, not a side table, so
/// RowGeom/hit-test/scroll all agree with what's actually painted for free.
/// THIS TEST is the direct regression guard for the marker-strand bug: it
/// asserts the ADJACENCY property the prior round's tests never checked (they
/// asserted the reserved height matched a formula, not that the marker and
/// caption ended up next to each other). Fixture: `samples/tiny.png` (120x48 ->
/// 48px `dh`).
#[test]
fn mixed_list_image_keeps_marker_adjacent_to_caption_and_draws_image_directly_below() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    let prev = crate::markdown::inline_images_on();
    crate::markdown::set_inline_images_on(true);
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!(
            "skipping mixed_list_image_keeps_marker_adjacent_to_caption_and_draws_image_directly_below: no wgpu adapter"
        );
        crate::markdown::set_inline_images_on(prev);
        return;
    };
    let text = "- caption text ![pic](samples/tiny.png)\nafter\n";
    // Caret OFF the image line (line 1) so the row sits at its steady reservation.
    let mut v = view(text, 1, 0);
    v.is_markdown = true;
    p.set_view(&v);
    let report = p.images_report();
    assert_eq!(report.len(), 1, "one image reported: {report:?}");
    let dh = report[0].display_h;
    assert!((dh - 48.0).abs() < 1.0, "fixture fit height: {dh}");
    let base_lh = p.metrics.line_height;

    // MARKER ADJACENT TO CAPTION: the shared row is UNTOUCHED — exactly
    // `base_lh`, never inflated by any multiple of `dh`. This is the direct
    // fix: the row that draws the marker ornament (`line_ornament_top`) is the
    // SAME row cosmic-text centres the caption glyphs within, and a `base_lh`
    // row leaves only the normal, tiny body-text centring offset between them
    // — nothing like the old `dh`-scale void.
    let rows = p.visual_rows(0);
    let row0_h = rows[0].line_height;
    assert!(
        (row0_h - base_lh).abs() < 1.0,
        "marker+caption row stays at base_lh, never inflated: {row0_h} vs base_lh {base_lh}"
    );

    // The forcing mechanism produces a genuine SECOND visual row of this SAME
    // logical line (real cosmic-text layout — `RowGeom`/hit-test read it for
    // free, no side table to keep in sync), sized to `dh`, immediately after.
    assert!(
        rows.len() >= 2,
        "a trailing row exists for the image on this logical line: {} row(s)",
        rows.len()
    );
    let row1_h = rows[1].line_height;
    assert!(
        (row1_h - dh).abs() < 1.0,
        "the trailing row is exactly dh tall: {row1_h} vs dh {dh}"
    );

    // IMAGE DIRECTLY BELOW, NO VOID: the drawn quad's top sits EXACTLY at the
    // caption row's own bottom edge — not offset by `dh`, not offset by any
    // stray gap (the old bug's signature).
    let rects = p.image_hit_rects();
    assert_eq!(rects.len(), 1, "one image hit rect: {rects:?}");
    let img_top = rects[0].1[1];
    let row0_top = p.line_ornament_top(0); // the marker's own draw y
    assert!(
        (img_top - (row0_top + row0_h)).abs() < 1.0,
        "image draws directly below the caption row, no void: img_top={img_top} row0_bottom={}",
        row0_top + row0_h
    );

    // NO OVERLAP: the image's bottom edge lands at (or before) the following
    // document line's own row top — it never bleeds into "after".
    let img_bottom = img_top + rects[0].1[3];
    let next_line_top = p.line_ornament_top(1);
    assert!(
        img_bottom <= next_line_top + 1.0,
        "image never overlaps the following document line: img_bottom={img_bottom} next_line_top={next_line_top}"
    );

    crate::markdown::set_inline_images_on(prev);
}

/// ITEM 5 REWORK — REVEALED MIXED LINE (caret ON the line): unconcealed, the
/// raw source (caption + `![alt](path)`) is long enough here to WRAP onto a
/// second visual row on its own. A per-LINE metrics override applied
/// regardless of reveal state would inflate EVERY wrapped row independently
/// (cosmic-text's per-visual-row `line_height_opt` MAX is taken PER ROW, not
/// once per logical line — confirmed empirically while building the prior
/// round: two wrapped rows landed at 432px EACH, not one combined total) and
/// strand the image mid-text — so `compute_image_layout` reserves NOTHING at
/// all for a revealed mixed line (plain, un-scaled rows — ordinary word wrap,
/// no forcing `letter_spacing` either) and the draw side skips the image for
/// that one frame (`image_row_reserved`). This asserts the OUTCOME: every
/// wrapped row of the revealed line stays at the PLAIN base line height (no
/// inflation, no matter how many rows it wraps onto), and no image hit-rect is
/// armed while revealed. The image reappears the instant the caret leaves —
/// covered by
/// `mixed_list_image_keeps_marker_adjacent_to_caption_and_draws_image_directly_below`
/// (caret off) above. Fixture: `samples/photo.png` (420x280).
#[test]
fn mixed_list_image_reveal_wraps_as_plain_text_and_parks_the_image() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    let prev = crate::markdown::inline_images_on();
    crate::markdown::set_inline_images_on(true);
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping mixed_list_image_reveal_wraps_as_plain_text: no wgpu adapter");
        crate::markdown::set_inline_images_on(prev);
        return;
    };
    if std::fs::metadata("samples/photo.png").is_err() {
        eprintln!("skipping: samples/photo.png fixture not present");
        crate::markdown::set_inline_images_on(prev);
        return;
    }
    let text = "- a caption sits before the image on this very same list line ![alt|300](samples/photo.png)\nafter\n";
    // Caret ON the image line (0): the raw source reveals, unconcealed, and
    // (at this narrow-ish default pipeline width) wraps onto >1 visual row.
    let mut v = view(text, 0, 0);
    v.is_markdown = true;
    p.set_view(&v);
    let base_lh = p.metrics.line_height;
    let rows = p.visual_rows(0);
    assert!(
        rows.len() > 1,
        "the revealed source is long enough to wrap: {} row(s)",
        rows.len()
    );
    for (i, r) in rows.iter().enumerate() {
        assert!(
            (r.line_height - base_lh).abs() < 1.0,
            "revealed row {i} stays at the PLAIN base line height, not the combined reservation: {} vs base_lh {base_lh}",
            r.line_height
        );
    }
    // No image drawn/armed while its own line is being edited — nothing to
    // overlap the wrapped-out reveal text.
    assert!(
        p.image_hit_rects().is_empty(),
        "the image parks (no hit rect) while its mixed line is revealed: {:?}",
        p.image_hit_rects()
    );
    crate::markdown::set_inline_images_on(prev);
}

/// ITEM 5 REWORK companion: a BARE list image (`- ![pic](p)`, no other caption
/// text) keeps the pre-existing `dh`-only reservation and draws at the row TOP
/// — `image_force` never fires for it (`image_draw_top` falls through to the
/// plain row top there), so this is byte-identical to the images-v1 behavior.
/// Guards against the mixed-line detector over-firing on the list marker
/// itself (which conceals to its own bullet glyph and must NOT count as
/// "other content"). Fixture: `samples/tiny.png`.
#[test]
fn bare_list_image_keeps_the_dh_only_row_and_draws_at_the_top() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    let prev = crate::markdown::inline_images_on();
    crate::markdown::set_inline_images_on(true);
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping bare_list_image_keeps_the_dh_only_row: no wgpu adapter");
        crate::markdown::set_inline_images_on(prev);
        return;
    };
    let text = "- ![pic](samples/tiny.png)\nafter\n";
    let mut v = view(text, 1, 0);
    v.is_markdown = true;
    p.set_view(&v);
    let dh = p.images_report()[0].display_h;
    let row_h = p.visual_rows(0)[0].line_height;
    assert!(
        (row_h - dh).abs() < 1.0,
        "bare list image: row is dh alone, no combined reservation: {row_h} vs {dh}"
    );
    let rects = p.image_hit_rects();
    let row_top = p.line_ornament_top(0);
    assert!(
        (rects[0].1[1] - row_top).abs() < 1.0,
        "bare list image draws at the row top (offset 0): {} vs {row_top}",
        rects[0].1[1]
    );
    crate::markdown::set_inline_images_on(prev);
}

/// ITEM 5b — ROW GEOMETRY OWNS IMAGE HEIGHT (the core fix, PRESERVED across the
/// item-5 rework): the scroll<->pixel table (`RowGeom`, delegated via
/// `TextPipeline::row_top_px`) must place the row AFTER an image at the
/// image's REAL rendered height — not a constant `LINE_HEIGHT` — or scroll
/// visibly JUMPS as the image enters/leaves view (the item's reported bug).
/// `build_line_attrs` bakes the reserved height into the shaped row's
/// `line_height` (an absolute `Attrs::metrics` override, the same seam
/// headings use), and `RowGeom::ensure` reads it straight off
/// `layout_runs()` — so this is the END-TO-END proof the mechanism holds for
/// both the BARE image case (unchanged since images-v1) and the item-5-REWORK
/// MIXED case, which now reserves `base_lh` (the untouched caption row) PLUS
/// `dh` (the forced trailing row) — a genuine two-visual-row split of the same
/// logical line, not a single inflated `base_lh + 2*dh` row (see
/// `TextPipeline::image_force`'s field doc). Fixture: `samples/tiny.png`
/// (120x48 -> 48px `dh`).
#[test]
fn row_geometry_places_the_row_after_an_image_at_its_real_height() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    let prev = crate::markdown::inline_images_on();
    crate::markdown::set_inline_images_on(true);
    crate::markdown::set_wysiwyg_on(true);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping row_geometry_places_the_row_after_an_image: no wgpu adapter");
        crate::markdown::set_inline_images_on(prev);
        return;
    };
    // BARE case: `![pic](p)` on its own line.
    let bare = "![pic](samples/tiny.png)\nprose one\nprose two\n";
    let mut vb = view(bare, 1, 0);
    vb.is_markdown = true;
    p.set_view(&vb);
    let dh = p.images_report()[0].display_h;
    let row1 = p.visual_row_of(1, 0);
    let top1 = p.row_top_px(row1);
    assert!(
        (top1 - dh).abs() < 1.0,
        "bare: the row after the image sits at dh ({dh}), not a jump: {top1}"
    );
    assert!(
        (top1 - LINE_HEIGHT).abs() > 5.0,
        "sanity: a constant-LINE_HEIGHT guess would be wrong here (that's the bug): {top1} vs {LINE_HEIGHT}"
    );
    // No jump at the document's total height either — it must include the
    // image's real height, not fall back to a uniform per-row count. Compares
    // against the LAST visual row's own top+height (not a hardcoded line index,
    // since `text.split('\n')` yields a trailing empty logical line too).
    let doc_h = p.total_doc_height();
    let last_row = p.total_visual_rows() - 1;
    let expected_doc_h = p.row_top_px(last_row) + p.row_height_px(last_row);
    assert!(
        (doc_h - expected_doc_h).abs() < 1.0,
        "total doc height matches the real cumulative row geometry: {doc_h} vs {expected_doc_h}"
    );

    // MIXED case (item 5 rework): `- text ![pic](p)` reserves base_lh (the
    // untouched caption row) + dh (the forced trailing row) — two REAL visual
    // rows of the SAME logical line, so "after" lands at base_lh + dh, not the
    // old base_lh + 2*dh single-row inflation.
    let mixed = "- text ![pic](samples/tiny.png)\nafter\n";
    let mut vm = view(mixed, 1, 0);
    vm.is_markdown = true;
    p.set_view(&vm);
    let dh2 = p.images_report()[0].display_h;
    let base_lh = p.metrics.line_height;
    let rows0 = p.visual_rows(0);
    assert!(
        rows0.len() >= 2,
        "the mixed line's own trailing row exists: {} row(s)",
        rows0.len()
    );
    let expected2 = base_lh + dh2;
    let row1b = p.visual_row_of(1, 0);
    let top1b = p.row_top_px(row1b);
    assert!(
        (top1b - expected2).abs() < 1.0,
        "mixed: the row after the image sits at base_lh + dh ({expected2}): {top1b}"
    );
    crate::markdown::set_inline_images_on(prev);
}

/// ITEM 5c — THE THEME-SWITCH SLOWDOWN PROBE (user-reported: "first switches
/// slow, later fine", suspects image reload). WITNESS, not a vague timing claim
/// (the documented "a bench measuring nothing" trap): `ImageCache::ensure` is
/// keyed by canonical PATH + file MTIME (`image_cache.rs`), never by theme, and
/// `sync_theme`/`sync_theme_colors`/`sync_theme_font` (the whole live theme-
/// switch apply path) never touch `TextPipeline::image_cache` — so a switch
/// RE-TINTS colors + re-shapes TEXT but must NEVER re-decode an already-cached
/// image. Drives a real `prepare()` (the only call site of `ImageCache::ensure`)
/// across five real theme switches with the SAME image on screen throughout,
/// asserting the DECODE counter (a new instrumentation counter, incremented
/// only on an actual cache MISS) stays flat at 1 the whole time — the mechanism
/// itself, not a proxy. Fixture: `samples/tiny.png`.
///
/// NOT REPRODUCED: this test PASSES on the current code — image decode is
/// already keyed independently of theme, so a switch never reloads. The
/// user-visible "first switches slow" is very likely `sync_theme`'s documented
/// FONT reshape (a real per-face cosmic-text restyle + fresh-glyph atlas
/// rasterization the FIRST time a family is visited — see `--bench-theme-burst`
/// / `pipeline_geometry.rs`'s `sync_theme` doc), unrelated to images; "later
/// fine" matches atlas retention once every face has been visited once. Flagged
/// for LIVE confirmation (the harness cannot measure real wall-clock feel), not
/// claimed fixed — there was nothing to fix here.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn theme_switch_never_redecodes_a_cached_image() {
    let _w = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    if std::fs::metadata("samples/tiny.png").is_err() {
        eprintln!(
            "skipping theme_switch_never_redecodes_a_cached_image: samples/tiny.png fixture not present"
        );
        return;
    }
    let prev_images = crate::markdown::inline_images_on();
    crate::markdown::set_inline_images_on(true);
    let prev_theme = crate::theme::active().name;
    let Some((device, queue, mut p)) = headless_pipeline_dq() else {
        eprintln!("skipping theme_switch_never_redecodes_a_cached_image: no wgpu adapter");
        crate::markdown::set_inline_images_on(prev_images);
        return;
    };
    let text = "![pic](samples/tiny.png)\nprose here\n";
    let mut v = view(text, 1, 0);
    v.is_markdown = true;
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert_eq!(
        p.image_decode_count(),
        1,
        "the first frame decodes the image exactly once"
    );

    // Five theme switches, the SAME image staying on screen throughout — the
    // exact "switching themes with images" scenario reported.
    for name in ["Mopoke", "Currawong", "Potoroo", "Bombora", "Tawny"] {
        crate::theme::set_active_by_name(name).expect("a real world name");
        p.sync_theme();
        p.set_view(&v);
        p.prepare(&device, &queue, 1200, 800).unwrap();
    }
    assert_eq!(
        p.image_decode_count(),
        1,
        "5 theme switches with the same image on screen: still exactly ONE decode (no reload)"
    );

    crate::theme::set_active_by_name(prev_theme);
    crate::markdown::set_inline_images_on(prev_images);
}
