//! The spell-suggest panel float/width, the replace-field reserved caret
//! cell, panel hit-testing, overlay click regions/lens labels/right-column
//! elision/card sizing, and the keycap glyph -- split out of the former
//! monolithic `render::tests` (2026-07 code-organization pass). See
//! `chrome_overlay` for the gutter + caret-preview-panel tests.

use super::super::*;
use super::{headless_pipeline, view};

/// The CONTEXTUAL SPELL PANEL: the spell overlay renders as a SMALL floating panel
/// anchored AT the misspelled word (its left edge at the word start, hanging just
/// below the word's row), on the reusable float primitive with NO scrim/blur — NOT
/// the centered takeover card the other pickers use. Contrasted against a centered
/// overlay to prove the geometry actually differs.
#[test]
fn spell_panel_floats_at_the_word_not_center_screen() {
    let got = pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl spell-panel test device"),
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
        eprintln!("skipping spell_panel_floats_at_the_word_not_center_screen: no wgpu adapter");
        return;
    };
    // The card anchors to the word via text_left, which folds the page
    // globals; hold the page lock so the anchor can't move between the
    // prepare and the assertion reads (page.rs:95-99).
    let _g = crate::page::test_lock();

    // The spell overlay: "teh" is the misspelled word at line 0, cols [0, 3); the
    // panel is anchored at that span and lists the corrections as rows.
    let mut v = view("teh quick brown fox\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = vec!["the".into(), "tea".into(), "ten".into()];
    v.overlay_selected = 0;
    v.overlay_spell = Some((0, 0, 3));
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();

    // It recedes NOTHING (no frosted blur, no scrim) — it's a small popup, not a
    // takeover.
    assert!(!p.dims_doc(), "the contextual spell panel keeps the document crisp");
    // The card floats AT the word: its left edge sits at the word start (text_left,
    // since "teh" begins at col 0) and it is SMALL — nowhere near a centered ~half-
    // canvas card. And it hangs BELOW the word's row (top past the first line).
    let word_left = p.text_left();
    let [x, y, w, _h] = p.overlay_card_rect().expect("the spell overlay has a card");
    assert!((x - word_left).abs() < 2.0, "card left edge anchors to the word start: {x} vs {word_left}");
    assert!(w <= 360.0, "the panel is a small popup, not a wide takeover: w={w}");
    assert!(x + w < 500.0, "the panel stays over the word, not centered: x={x} w={w}");
    assert!(y > p.metrics.line_height, "the panel hangs below the word's row: y={y}");
    // It rides the FLOAT primitive (shadow + border + card), and the flat centered
    // card + the amber query caret are BOTH parked.
    assert_eq!(p.float_card.instance_count(), 1, "the spell panel is a floating card");
    assert_eq!(p.float_shadow.instance_count(), 1, "with a drop shadow");
    assert_eq!(p.float_border.instance_count(), 1, "and a raised border edge");
    assert_eq!(p.panel_card.instance_count(), 0, "no flat centered card for the spell panel");
    assert!(!p.panel_caret.is_drawn(), "no amber query caret on the spell panel");

    // CONTRAST: a centered overlay (no spell target) is a wide card near screen
    // center, on the flat panel card — NOT the float primitive.
    let mut c = view("teh quick brown fox\n", 0, 0);
    c.overlay_active = true;
    c.overlay_items = vec!["the".into(), "tea".into(), "ten".into()];
    p.set_view(&c);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    let [cx, _cy, cw, _ch] = p.overlay_card_rect().expect("the centered overlay has a card");
    assert!(cw >= 360.0, "a centered overlay is a wide card: w={cw}");
    assert!((cx - (1200.0 - cw) * 0.5).abs() < 2.0, "the centered card is horizontally centered: x={cx}");
    assert_eq!(p.float_card.instance_count(), 0, "a centered overlay parks the float card");
    assert_eq!(p.panel_card.instance_count(), 1, "a centered overlay uses the flat card");
}

/// SPELL PANEL WIDTH is CONTENT-driven, not word-driven: the card sizes to the
/// widest suggestion ROW's shaped width + padding (with a calm MIN), so a SHORT
/// misspelled word can't make a narrow card the longer corrections overflow. The
/// same short word yields a WIDER card when its suggestions are longer — proof the
/// width tracks the content, not the (fixed) anchor word.
#[test]
fn spell_panel_width_fits_longest_suggestion_not_the_word() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping spell_panel_width_fits_longest_suggestion_not_the_word: no wgpu adapter");
        return;
    };
    let pad = 10.0_f32; // the spell panel's inner padding (spell_overlay_geometry)
    let margin = 8.0_f32;
    let canvas = 1200.0_f32;

    // The SAME short misspelled word ("teh"), once with a LONG suggestion.
    let mut long = view("teh quick brown fox\n", 0, 0);
    long.overlay_active = true;
    long.overlay_items = vec!["the".into(), "thoroughgoingly".into(), "ten".into()];
    long.overlay_selected = 0;
    long.overlay_spell = Some((0, 0, 3));
    p.set_view(&long);
    // The measured content width == the widest shaped suggestion row.
    let content = p.measure_spell_content_w();
    assert!(content > 0.0, "a shaped suggestion has a positive width");
    let [_lx, _ly, w_long, _lh] = p.overlay_card_rect().expect("the spell overlay has a card");
    // The card width follows the formula: content + padding, floored at the calm
    // MIN (140) and capped small (360), kept on-canvas — NOT the word's width.
    let expect = (content + 2.0 * pad).clamp(140.0, 360.0).min(canvas - 2.0 * margin);
    assert!(
        (w_long - expect).abs() < 0.5,
        "card width is content-driven (max-row + pad, min 140, cap 360): got {w_long}, expected {expect} (content {content})"
    );
    // The long suggestion pushed the card PAST the min floor (so this case is
    // meaningful) and its inner text column FITS the suggestion — no overflow.
    assert!(w_long > 140.0, "the long suggestion widens the card past the min: {w_long}");
    assert!(
        w_long - 2.0 * pad >= content - 0.5,
        "the card's text column ({}) fits the longest suggestion ({content})",
        w_long - 2.0 * pad
    );
    assert!(w_long <= 360.0, "still a small popup, not a takeover: {w_long}");

    // The SAME word with only SHORT suggestions → a NARROWER card, clamped to the
    // calm MIN. Width tracks the content, not the (identical) word.
    let mut short = view("teh quick brown fox\n", 0, 0);
    short.overlay_active = true;
    short.overlay_items = vec!["the".into(), "ten".into(), "tea".into()];
    short.overlay_selected = 0;
    short.overlay_spell = Some((0, 0, 3));
    p.set_view(&short);
    let [_sx, _sy, w_short, _sh] = p.overlay_card_rect().expect("the spell overlay has a card");
    assert!(w_short >= 140.0, "a short suggestion set still respects the min width: {w_short}");
    assert!(
        w_short < w_long,
        "the longer suggestions make a WIDER card ({w_long}) than the short set ({w_short}) at the SAME word — content-driven, not word-driven"
    );
}

/// THE REPLACE-FIELD CARET rides the reserved cell shaped right after the
/// REPLACEMENT text on its OWN row (line 1), exactly the way the find caret sits
/// after the query on row 0. The regression: the reserved cell's byte offset was
/// computed BUFFER-GLOBAL (`row0_len + "\n" + "replace " + replacement`), but
/// cosmic-text's `LayoutGlyph::start` is LINE-relative (resets to 0 after every
/// `\n`), so that offset matched NO line-1 glyph and the caret dropped onto the
/// hardcoded char-pitch fallback — floating mid-panel on a proportional world.
/// The caret-x is a PURE function of the shaped layout, so we drive
/// `panel_shape_text` + `panel_layout` directly and compare against the
/// INDEPENDENTLY-scanned x of the reserved glyph on line 1.
#[test]
fn replace_caret_rides_the_reserved_cell_after_the_replacement_text() {
    // A PROPORTIONAL world (Literata) so the shaped advance genuinely differs from
    // the char-pitch fallback — the bug is invisible on a mono grid where the two
    // coincide. set_active_by_name mutates the theme global → hold the theme lock.
    let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::theme::set_active_by_name("Gumtree").unwrap();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping replace_caret_rides_the_reserved_cell_after_the_replacement_text: no wgpu adapter");
        return;
    };
    let width = 1200u32;
    const REPLACE_LABEL: &str = "replace "; // must match panel.rs's label

    // The reserved-cell glyph's x on line 1, scanned INDEPENDENTLY of the
    // caret-offset math under test — the ground truth the caret must land on.
    let reserved_x = |p: &TextPipeline, text_left: f32, replacement: &str| -> f32 {
        let cell = REPLACE_LABEL.len() + replacement.len();
        for run in p.panel_buffer.layout_runs() {
            if run.line_i != 1 {
                continue;
            }
            for g in run.glyphs.iter() {
                if g.start == cell {
                    return text_left + g.x;
                }
            }
        }
        panic!("no reserved-cell glyph on the replace row for {replacement:?}");
    };

    for replacement in ["world", ""] {
        let mut v = view("hello\nhello\n", 0, 0);
        v.search_active = true;
        v.search_query = "hello".into();
        v.search_matches = vec![((0, 0), (0, 5)), ((1, 0), (1, 5))];
        v.search_current = Some(0);
        v.search_replace_active = true;
        v.search_replacement = replacement.into();
        v.search_editing_replacement = true; // focus on the REPLACE field
        p.set_view(&v);

        let shape = p.panel_shape_text(width);
        assert_eq!(shape.caret_row, 1.0, "replace focus targets row 1");
        // The offset is LINE-relative: the label + replacement WITHIN line 1 only —
        // no find-row bytes, no `\n`.
        assert_eq!(
            shape.caret_byte,
            REPLACE_LABEL.len() + replacement.len(),
            "reserved-cell byte is line-relative for {replacement:?}"
        );
        let (_card, text_left, _top, caret_x) =
            p.panel_layout(width, shape.caret_byte, shape.caret_fallback_chars, shape.caret_row);

        let expected = reserved_x(&p, text_left, replacement);
        assert!(
            (caret_x - expected).abs() < 0.5,
            "replace caret rides the shaped reserved cell (x={caret_x}, expected {expected}) for {replacement:?}"
        );
        // And it is the SHAPED advance, not the hardcoded char-pitch fallback
        // (the old bug's landing spot) — proof we resolved a real line-1 glyph.
        let fallback = text_left + p.metrics.char_width * shape.caret_fallback_chars as f32;
        assert!(
            (caret_x - fallback).abs() > 0.5,
            "on a proportional world the caret is NOT the char-pitch fallback \
             (x={caret_x}, fallback {fallback}) for {replacement:?}"
        );
    }

    // REGRESSION: with the SAME replace panel up but focus on the FIND field, the
    // caret returns to row 0 riding the query end — the row filter must not have
    // stranded the find caret.
    let mut v = view("hello\nhello\n", 0, 0);
    v.search_active = true;
    v.search_query = "hello".into();
    v.search_matches = vec![((0, 0), (0, 5)), ((1, 0), (1, 5))];
    v.search_current = Some(0);
    v.search_replace_active = true;
    v.search_replacement = "world".into();
    v.search_editing_replacement = false; // focus on the FIND field
    p.set_view(&v);
    let shape = p.panel_shape_text(width);
    assert_eq!(shape.caret_row, 0.0, "find focus targets row 0");
    let (_card, text_left, _top, caret_x) =
        p.panel_layout(width, shape.caret_byte, shape.caret_fallback_chars, shape.caret_row);
    // Ground truth: the reserved gap glyph on line 0 sits at byte "find "+query.
    let cell = "find    ".len() + "hello".len();
    let mut find_expected = None;
    for run in p.panel_buffer.layout_runs() {
        if run.line_i != 0 {
            continue;
        }
        for g in run.glyphs.iter() {
            if g.start == cell {
                find_expected = Some(text_left + g.x);
            }
        }
    }
    let find_expected = find_expected.expect("reserved gap glyph on the find row");
    assert!(
        (caret_x - find_expected).abs() < 0.5,
        "find caret still rides the query end on row 0 (x={caret_x}, expected {find_expected})"
    );
}

/// CLICK-TO-SWITCH-FIELD: the pure `panel_hit` maps a physical pointer to the
/// find/replace field it lands on, from the SAME `panel_layout` the fields draw
/// from (no parallel geometry). Row 0 = find, row 1 = replace (present only once
/// revealed); inside the card but off a row = `Elsewhere` (a swallowed no-op);
/// off the card / panel down = `None` (falls through to the document). This is
/// the purest seam of `App::panel_click`'s find↔replace decision.
#[test]
fn panel_hit_maps_the_pointer_to_the_find_or_replace_field() {
    // The top-right panel card is anchored to the window's right edge, not the
    // page-mode writing column, so no page-global geometry is folded (no lock).
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping panel_hit_maps_the_pointer_to_the_find_or_replace_field: no wgpu adapter");
        return;
    };
    let width = p.window_w as u32;

    // Replace REVEALED: three panel rows (find / replace / key-hint).
    let mut v = view("hello\nhello\n", 0, 0);
    v.search_active = true;
    v.search_query = "hello".into();
    v.search_matches = vec![((0, 0), (0, 5)), ((1, 0), (1, 5))];
    v.search_current = Some(0);
    v.search_replace_active = true;
    v.search_replacement = "world".into();
    v.search_editing_replacement = false;
    p.set_view(&v);
    // Shape the panel so panel_layout has real rows to measure.
    let shape = p.panel_shape_text(width);
    let ([card_x, card_y, card_w, card_h], _tl, text_top, _cx) =
        p.panel_layout(width, shape.caret_byte, shape.caret_fallback_chars, shape.caret_row);
    let lh = p.metrics.line_height;
    let mid = card_x + card_w * 0.5; // safely inside the card horizontally

    assert_eq!(p.panel_hit(mid, text_top + 0.5 * lh), Some(PanelHit::Find));
    assert_eq!(p.panel_hit(mid, text_top + 1.5 * lh), Some(PanelHit::Replace));
    // The key-hint line (row 2) is inside the card but not editable -> Elsewhere.
    assert_eq!(p.panel_hit(mid, text_top + 2.5 * lh), Some(PanelHit::Elsewhere));
    // Off the card (far left / above / below) -> None: the press falls through.
    assert_eq!(p.panel_hit(card_x - 20.0, text_top + 0.5 * lh), None);
    assert_eq!(p.panel_hit(mid, card_y - 5.0), None);
    assert_eq!(p.panel_hit(mid, card_y + card_h + 5.0), None);

    // Replace NOT revealed: a single find row. Row 0 -> Find; below the one row
    // is off the (1-row) card -> None; the replace band never resolves.
    let mut v1 = view("hello\nhello\n", 0, 0);
    v1.search_active = true;
    v1.search_query = "hello".into();
    v1.search_matches = vec![((0, 0), (0, 5)), ((1, 0), (1, 5))];
    v1.search_current = Some(0);
    v1.search_replace_active = false;
    p.set_view(&v1);
    let shape1 = p.panel_shape_text(width);
    let ([cx1, _cy1, cw1, ch1], _t1, top1, _c1) = p.panel_layout(
        width,
        shape1.caret_byte,
        shape1.caret_fallback_chars,
        shape1.caret_row,
    );
    let mid1 = cx1 + cw1 * 0.5;
    assert_eq!(p.panel_hit(mid1, top1 + 0.5 * lh), Some(PanelHit::Find));
    // The would-be replace band sits below the one-row card -> off card -> None.
    assert!(top1 + 1.5 * lh > _cy1 + ch1, "replace band is below the 1-row card");
    assert_eq!(p.panel_hit(mid1, top1 + 1.5 * lh), None);

    // Panel DOWN -> always None (the press falls through to the document).
    let v2 = view("hello\nhello\n", 0, 0); // search_active defaults false
    p.set_view(&v2);
    assert_eq!(p.panel_hit(mid1, top1 + 0.5 * lh), None);
}

/// CLICK-AWAY on a summoned overlay: the three pointer regions `input.rs` resolves
/// from the SAME `overlay_card_rect` + `overlay_row_at` geometry — ON a candidate
/// row (→ select+accept), OUTSIDE the card (→ dismiss via `Action::Cancel`, the
/// close Esc uses; see `actions::overlay_nav` tests), and INSIDE-but-off-a-row (→
/// swallowed, stays modal). This is the kind-agnostic geometry every overlay shares.
#[test]
fn overlay_click_regions_select_inside_row_and_dismiss_outside() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping overlay_click_regions_select_inside_row_and_dismiss_outside: no wgpu adapter");
        return;
    };
    // A centered picker: a query line on top, three candidate rows, a foot hint.
    let mut v = view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = vec!["Alpha".into(), "Beta".into(), "Gamma".into()];
    v.overlay_selected = 0;
    v.overlay_hint = "\u{21B5} run".into();
    p.set_view(&v);

    let [cx, cy, cw, ch] = p.overlay_card_rect().expect("the overlay has a card");
    let lh = p.metrics.line_height;
    let pad = 12.0_f32; // centered-overlay inner padding (overlay_geometry)
    let text_top = cy + pad;
    // The exact predicate input.rs uses for "inside the card".
    let inside = |px: f32, py: f32| px >= cx && px <= cx + cw && py >= cy && py <= cy + ch;

    // ON the first candidate row (one line below the query row): hit-tests to row 0
    // → input.rs selects + accepts it.
    let row_x = cx + cw * 0.5;
    let row0_y = text_top + 1.5 * lh;
    assert_eq!(p.overlay_row_at(row_x, row0_y), Some(0), "a click on the first candidate row selects it");
    assert!(inside(row_x, row0_y), "the row is inside the card");

    // OUTSIDE the card entirely: no row hit AND outside the rect → input.rs routes
    // this to Action::Cancel (dismiss), the same close Esc uses.
    let out_x = cx - 40.0;
    let out_y = cy - 40.0;
    assert_eq!(p.overlay_row_at(out_x, out_y), None, "a click off the card hits no row");
    assert!(!inside(out_x, out_y), "the point is outside the card → dismiss");

    // INSIDE the card but on the QUERY line (not a candidate row): no row hit, yet
    // inside the rect → swallowed, the picker stays modal (no dismiss).
    let query_y = text_top + 0.5 * lh;
    assert_eq!(p.overlay_row_at(row_x, query_y), None, "the query line is not a candidate row");
    assert!(inside(row_x, query_y), "but it is inside the card → swallowed, not dismissed");

    // CURSOR-SHAPE flag sources on this NON-spell picker (the pointing-hand
    // generalization + the query-input I-beam): a candidate row lights the
    // clickable-row flag (→ Pointer) but NOT the query flag; the query line
    // lights the query flag (→ I-beam) but NOT the row flag; off the card
    // lights neither.
    assert!(p.overlay_row_at(row_x, row0_y).is_some(), "row → clickable-overlay-row flag (hand)");
    assert!(!p.over_overlay_query(row_x, row0_y), "a candidate row is not the query field");
    assert!(p.over_overlay_query(row_x, query_y), "the query line → query-input flag (I-beam)");
    assert_eq!(p.overlay_row_at(row_x, query_y), None, "the query line lights no row flag");
    assert!(!p.over_overlay_query(out_x, out_y), "off the card → no query field");
}

/// CLICKABLE LENS STRIP: `overlay_lens_at` is the pure x/y → facet-STRIP-INDEX
/// hit-test `overlay_click` (input.rs) and the cursor-shape hover flag both ride
/// (one owner — the same geometry the strip SHAPER laid out, read back from the
/// shaped glyphs). A click/hover on a facet label resolves to its own strip
/// index regardless of which lens is currently active; off the strip row (the
/// query line, a candidate row, off the card) resolves to `None`.
#[test]
fn overlay_lens_at_resolves_facet_labels_by_their_own_strip_index() {
    let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _g = crate::page::test_lock();
    let got = pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl test device"),
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
        eprintln!("skipping overlay_lens_at_resolves_facet_labels_by_their_own_strip_index: no wgpu adapter");
        return;
    };

    // A faceted picker shaped like the theme picker: five strip lenses (All,
    // Time, Register, Voice, Temperature — All never drawn), Time active.
    let strip = |active: usize| -> Vec<(String, bool)> {
        ["All", "Time", "Register", "Voice", "Temperature"]
            .iter()
            .enumerate()
            .map(|(i, l)| (l.to_string(), i == active))
            .collect()
    };
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = vec!["Alpha".into(), "Beta".into(), "Gamma".into()];
    v.overlay_selected = 0;
    v.overlay_lens = strip(1); // Time active
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();

    let lh = p.overlay_lh();
    let [cx, cy, _cw, _ch] = p.overlay_card_rect().expect("the faceted overlay has a card");
    let pad = 12.0_f32; // centered-overlay inner padding (overlay_geometry)
    let text_top = cy + pad;
    let strip_y = text_top + 1.5 * lh; // mid strip row (display line 1)
    let query_y = text_top + 0.5 * lh; // the query line — not the strip
    let row_y = text_top + 2.5 * lh; // a candidate item row — below the strip

    // The ACTIVE facet's own recorded underline rect pinpoints its shaped x-span —
    // a click in its middle resolves to ITS OWN strip index (1, Time).
    let [ux, uy, uw, _uh] = p.overlay_theme_underline.expect("Time is active, so it is underlined");
    assert!(
        uy >= text_top + lh - 5.0 && uy <= text_top + 2.0 * lh + 5.0,
        "underline sits on the strip row (line 1)"
    );
    let time_mid_x = ux + uw * 0.5;
    assert_eq!(p.overlay_lens_at(time_mid_x, strip_y), Some(1), "a click on Time resolves to strip index 1");

    // Off the strip row entirely (query line, a candidate row) never hits a lens,
    // even at the exact same x as a real facet label.
    assert_eq!(p.overlay_lens_at(time_mid_x, query_y), None, "the query line is not the strip");
    assert_eq!(p.overlay_lens_at(time_mid_x, row_y), None, "a candidate row is not the strip");

    // Off the card entirely (far outside its rect) never hits a lens.
    assert_eq!(p.overlay_lens_at(cx - 200.0, cy - 200.0), None, "off the card hits no lens");

    // Re-shape with Register (index 2) active instead — the SAME x position that
    // hit "Time" above still resolves to strip index 1 (Time's label metrics never
    // move: only its COLOR changes with which lens is active, never its width), and
    // Register's own new underline resolves to its own index (2), not Time's.
    v.overlay_lens = strip(2); // Register active
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert_eq!(
        p.overlay_lens_at(time_mid_x, strip_y),
        Some(1),
        "Time's own x-span still resolves to index 1 even while Register is active"
    );
    let [rx, _ry, rw, _rh] = p.overlay_theme_underline.expect("Register is now active");
    let register_mid_x = rx + rw * 0.5;
    assert_eq!(
        p.overlay_lens_at(register_mid_x, strip_y),
        Some(2),
        "a click on Register resolves to strip index 2"
    );
}

/// THE NO-OVERLAP LAW at the pipeline level (rowlayout end-to-end): a row's name
/// and its dim right column share ONE budget — when the shaped pixels say both
/// cannot fit, the RIGHT column YIELDS (dropped whole) and the short names stay
/// crisp (never elided); when both genuinely fit — even at the minimum window —
/// both show. This is the caret-picker regression: its long descriptions used to
/// collapse the name budget to a 4-char floor ("Block" → "B…ck") and then paint
/// straight over the munched names.
#[test]
fn overlay_right_column_yields_before_names_elide() {
    // Shaped pixel widths fold the active THEME font and prepare reads the PAGE
    // globals — hold both test locks (theme → page order, page.rs:95-99).
    let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _g = crate::page::test_lock();
    let got = pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl test device"),
                ..Default::default()
            })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let mut p =
            TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
        p.set_size(464.0, 600.0);
        Some((device, queue, p))
    });
    let Some((device, queue, mut p)) = got else {
        eprintln!("skipping overlay_right_column_yields_before_names_elide: no wgpu adapter");
        return;
    };

    // A caret-picker-shaped view: SHORT names beside one enormous description no
    // face can fit beside them at the minimum window width.
    let long_desc = "a deliberately enormous description line that no world face could \
                     ever fit beside a candidate name at the minimum window width";
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = vec!["Block".into(), "Morph".into(), "I-beam".into()];
    v.overlay_bindings = vec![long_desc.into(), "short".into(), "also short".into()];
    v.overlay_selected = 0;
    p.set_view(&v);
    p.prepare(&device, &queue, 464, 600).unwrap();
    assert!(
        !p.overlay_right_shown,
        "narrow + oversized right column: the right column must YIELD"
    );
    let line = |p: &TextPipeline, i: usize| p.panel_buffer.lines[i].text().to_string();
    assert_eq!(line(&p, 1), "Block", "a 5-char name is NEVER elided");
    assert_eq!(line(&p, 2), "Morph");
    assert_eq!(line(&p, 3), "I-beam");

    // The SAME names beside SHORT labels at the SAME minimum window: both cells
    // genuinely fit, so the right column shows and the names stay whole —
    // disclosure follows the measured fit, not the window size alone.
    v.overlay_bindings = vec!["hi".into(), "yo".into(), "ok".into()];
    p.set_view(&v);
    p.prepare(&device, &queue, 464, 600).unwrap();
    assert!(
        p.overlay_right_shown,
        "narrow + short right column: both cells fit, the column shows"
    );
    assert_eq!(line(&p, 1), "Block", "names stay whole beside a granted column");

    // And the oversized description yields even at the DEFAULT canvas — the rule
    // is one budget, not a narrow-window special case.
    v.overlay_bindings = vec![long_desc.into(), "short".into(), "also short".into()];
    p.set_view(&v);
    p.set_size(1200.0, 800.0);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert!(
        !p.overlay_right_shown,
        "an oversized right column yields at any width"
    );
    assert_eq!(line(&p, 1), "Block", "…and the names still never pay for it");
}

/// RESPONSIVE CARD: at the minimum window width the centered picker card spans
/// nearly the full window (window − 2·margin), mirroring the responsive page
/// column, instead of the old fixed 360 that starved the text column; at the
/// default 1200 canvas it stays the familiar 600 (wide captures byte-identical).
#[test]
fn overlay_card_spans_nearly_the_full_narrow_window() {
    let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _g = crate::page::test_lock();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping overlay_card_spans_nearly_the_full_narrow_window: no wgpu adapter");
        return;
    };
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = vec!["Alpha".into(), "Beta".into()];
    p.set_view(&v);

    // Minimum window (≈ 30 columns + insets): the card spans window − 24.
    p.set_size(464.0, 600.0);
    let [x, _y, w, _h] = p.overlay_card_rect().expect("overlay card");
    assert!((w - 440.0).abs() < 0.5, "narrow card spans nearly the window: w={w}");
    assert!((x - 12.0).abs() < 0.5, "with the calm 12px margin: x={x}");

    // Default canvas: the same half-window card as ever.
    p.set_size(1200.0, 800.0);
    let [_x, _y, w, _h] = p.overlay_card_rect().expect("overlay card");
    assert!((w - 600.0).abs() < 0.5, "wide card is unchanged: w={w}");
}

/// KEY-HINT KEYCAPS: ↵ (Return) and ⇥ (Tab) are classified as SYMBOLS (so the hint
/// lines shape them from the bundled SYMBOL_FAMILY face like ⌘/⌥, not tofu) AND the
/// bundled AwlSymbols face actually COVERS both codepoints.
#[test]
fn keycap_glyphs_are_symbols_and_bundled() {
    // Classification: both keycaps are symbols; a plain letter is not.
    assert!(is_symbol('\u{21B5}'), "↵ Return is a symbol keycap");
    assert!(is_symbol('\u{21E5}'), "⇥ Tab is a symbol keycap");
    assert!(!is_symbol('r') && !is_symbol('t'), "plain letters are not symbols");
    // A hint fragment isolates the leading glyph run from the plain text remainder.
    let s = "\u{21B5} restore";
    let runs = symbol_runs(s);
    assert_eq!(runs.len(), 1, "one run over the ↵ keycap: {runs:?}");
    assert_eq!(&s[runs[0].clone()], "\u{21B5}", "the run covers ↵ only");

    // Font coverage: the bundled AwlSymbols face resolves both keycaps.
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping keycap_glyphs_are_symbols_and_bundled font-coverage half: no wgpu adapter");
        return;
    };
    let id = p
        .font_system
        .db()
        .faces()
        .find(|f| f.families.iter().any(|(n, _)| n == SYMBOL_FAMILY))
        .map(|f| f.id)
        .expect("the bundled symbol face is registered");
    let font = p
        .font_system
        .get_font(id, glyphon::cosmic_text::fontdb::Weight::NORMAL)
        .expect("the symbol face loads");
    // A nonzero glyph id in the face's charmap means the codepoint resolves to a
    // real glyph (not .notdef / tofu).
    let charmap = font.as_swash().charmap();
    assert!(charmap.map('\u{21B5}') != 0, "AwlSymbols must cover ↵ (U+21B5) — else it renders as tofu");
    assert!(charmap.map('\u{21E5}') != 0, "AwlSymbols must cover ⇥ (U+21E5) — else it renders as tofu");
    // Sanity: the pre-existing ⌘ still resolves, and an uncovered codepoint does not.
    assert!(charmap.map('\u{2318}') != 0, "the ⌘ glyph still resolves");
    assert!(charmap.map('Z') == 0, "a plain letter is NOT in the symbol face");
}
