//! Render-pipeline BEHAVIORAL tests for the TRUE 1-BIT world (Wagtail's
//! 2026-07 greyscale -> 1-bit rework, plus the later DITHER round): the
//! frosted-blur backdrop disabling itself, the TRUE INVERSE-VIDEO selection
//! pipeline actually drawing (and staying idle on every other world), and THE
//! ONE WAGTAIL HIGHLIGHT TEXTURE's dither mode switching on/off with the
//! theme. The PALETTE-literal laws (every authored color is exactly
//! `#000000`/`#FFFFFF`) live in `syntax_roles.rs`
//! (`every_one_bit_world_renders_only_pure_black_or_white`); this file is the
//! GPU-pipeline half — does the renderer actually behave the way the palette
//! promises. The REAL-PIXEL half (does the shader actually paint only pure
//! values) lives in `dither.rs`.

use super::super::*;
use super::{headless_pipeline, view};

/// A `(Device, Queue, TextPipeline)` triple sized `w`x`h`, or `None` on a
/// GPU-less machine. Some assertions in this file need to READ instance
/// counts a real `prepare()` call left behind (`headless_pipeline`'s bare
/// `TextPipeline` has no device/queue of its own to drive one) — shared here
/// rather than re-inlined per test, mirroring `dither.rs`'s own `headless_dq`.
fn headless_dqp(w: f32, h: f32) -> Option<(wgpu::Device, wgpu::Queue, TextPipeline)> {
    pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl one-bit-test device"),
                ..Default::default()
            })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let mut p =
            TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
        p.set_size(w, h);
        Some((device, queue, p))
    })
}

/// A true 1-bit world (`Theme::is_one_bit`) disables the frosted-blur
/// backdrop OUTRIGHT, for every consumer that would otherwise trigger it — a
/// gaussian defocus of a pure black/white document mathematically smears
/// every edge into forbidden grey, so there is no tuning that avoids it.
/// Contrasted against an ordinary dark world (Tawny), which DOES blur under
/// the identical view state, proving the gate is theme-specific rather than
/// globally broken.
#[test]
fn wagtail_disables_the_frosted_blur_backdrop_every_other_world_still_gets_it() {
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wagtail_disables_the_frosted_blur_backdrop_every_other_world_still_gets_it: no wgpu adapter");
        return;
    };

    // A full-takeover overlay (NOT the crisp theme/caret picker, NOT the
    // contextual spell popup) — exactly `overlay_blur()`'s eligible case.
    let mut v = view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = vec!["one".into(), "two".into()];

    theme::set_active_by_name("Wagtail").unwrap();
    p.set_view(&v);
    assert!(
        !p.backdrop_blur(),
        "Wagtail (one-bit): an open overlay must NOT trigger the frosted blur"
    );

    theme::set_active_by_name("Tawny").unwrap();
    p.set_view(&v);
    assert!(
        p.backdrop_blur(),
        "Tawny (ordinary dark world): the SAME overlay state must still trigger the frosted blur \
         (proves the one-bit gate is theme-specific, not a global regression)"
    );

    // Restore the default world so other tests see a clean global.
    theme::set_active(theme::DEFAULT_THEME);
}

/// The SUMMONED-WHILE-HELD stats HUD is another `backdrop_blur` consumer
/// (`hud_showing()`); Wagtail must suppress it exactly like the overlay case.
#[test]
fn wagtail_disables_the_frosted_blur_backdrop_for_the_held_hud_too() {
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wagtail_disables_the_frosted_blur_backdrop_for_the_held_hud_too: no wgpu adapter");
        return;
    };
    crate::hud::set_held(true);
    let v = view("hello world\n", 0, 0);

    theme::set_active_by_name("Wagtail").unwrap();
    p.set_view(&v);
    assert!(
        !p.backdrop_blur(),
        "Wagtail (one-bit): the held HUD must NOT trigger the frosted blur"
    );

    theme::set_active_by_name("Tawny").unwrap();
    p.set_view(&v);
    assert!(
        p.backdrop_blur(),
        "Tawny: the held HUD must still trigger the frosted blur under the SAME state"
    );

    crate::hud::set_held(false);
    theme::set_active(theme::DEFAULT_THEME);
}

/// TRUE INVERSE-VIDEO SELECTION (the DITHER round's upgrade, replacing the
/// old "punch outline" fallback outright — see `worlds.rs::WAGTAIL`'s doc
/// comment + THEMES.md's 1-bit section for the full history): on Wagtail, the
/// ORDINARY `selection_pipeline` uploads ZERO rects (its translucent fill is
/// retired for one-bit selection), while the NEW `selection_invert` pipeline
/// draws exactly one instance per selected-line rect. On an ordinary world
/// (Tawny) it is the other way around — `selection_invert` stays idle and
/// `selection_pipeline` carries the real fill. See `dither.rs` for the
/// REAL-PIXEL proof that the invert math itself flips black<->white.
#[test]
fn wagtail_selection_uses_the_invert_pipeline_other_worlds_use_the_ordinary_fill() {
    let got = pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl one-bit selection-invert test device"),
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
        eprintln!("skipping wagtail_selection_uses_the_invert_pipeline_other_worlds_use_the_ordinary_fill: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    let text = "alpha\nbeta\ngamma";
    let mut v = view(text, 2, 3);
    v.selection = Some(((0, 2), (2, 3)));

    theme::set_active_by_name("Wagtail").unwrap();
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    let sel_rects = p.selection_rects();
    assert!(!sel_rects.is_empty(), "the fixture selection must actually produce rects");
    assert_eq!(
        p.selection_invert.instance_count() as usize,
        sel_rects.len(),
        "Wagtail (one-bit): the invert pipeline draws one quad per selected-line rect"
    );
    assert_eq!(
        p.selection_pipeline.instance_count(),
        0,
        "Wagtail (one-bit): the ordinary translucent-fill pipeline uploads nothing — \
         retired for one-bit selection in favor of true inversion"
    );

    theme::set_active_by_name("Tawny").unwrap();
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert_eq!(
        p.selection_invert.instance_count(),
        0,
        "Tawny: the invert pipeline stays idle on an ordinary (non-one-bit) world"
    );
    assert_eq!(
        p.selection_pipeline.instance_count() as usize,
        sel_rects.len(),
        "Tawny: the ordinary translucent-fill pipeline carries the real selection"
    );

    // Restore the default world so other tests see a clean global.
    theme::set_active(theme::DEFAULT_THEME);
}

/// THE ROUND'S OWN "vertical selection reads invisible" report, at the
/// INSTANCE-COUNT seam: a 3-line selection whose MIDDLE line is EMPTY still
/// yields one rect per selected row (the text row, the empty row's own
/// newline-pad stub, the next text row — see `range_rects`'s doc for why an
/// empty line still emits a non-degenerate rect) and EVERY one of those rects
/// reaches `selection_invert` on Wagtail — never silently dropped by a gate
/// that only recognizes non-empty/text-bearing rows. This is the sibling
/// proof to `wagtail_selection_uses_the_invert_pipeline_other_worlds_use_the_
/// ordinary_fill` above (which already covers a real multi-line span but with
/// no empty line in it) and to `dither.rs`'s `wagtail_multiline_selection_
/// shows_inverted_text_and_solid_white_on_empty_line`, which proves the SAME
/// fixture's shape at the real-pixel level (solid white on the empty
/// stretch, legible black-on-white text on the other two rows).
#[test]
fn wagtail_multiline_selection_with_empty_line_reaches_invert_pipeline_entirely() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!("skipping wagtail_multiline_selection_with_empty_line_reaches_invert_pipeline_entirely: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    let text = "first\n\nthird\nfourth";
    let mut v = view(text, 2, 5);
    v.selection = Some(((0, 0), (2, 5)));

    theme::set_active_by_name("Wagtail").unwrap();
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    let sel_rects = p.selection_rects();
    assert_eq!(
        sel_rects.len(),
        3,
        "one rect per selected row -- text, EMPTY, text -- the empty middle line must not be dropped"
    );
    assert_eq!(
        p.selection_invert.instance_count() as usize,
        sel_rects.len(),
        "Wagtail (one-bit): every rect -- text row, EMPTY row, text row -- reaches the invert \
         pipeline; none of the three geometry sources (text-row run, empty-line stub, the row \
         that reaches the newline tail) is routed anywhere else"
    );
    assert_eq!(
        p.selection_pipeline.instance_count(),
        0,
        "Wagtail (one-bit): the ordinary translucent fill stays empty even with an empty line \
         in the middle of the selection"
    );

    // Restore the default world so other tests see a clean global.
    theme::set_active(theme::DEFAULT_THEME);
}

/// THE 1-BIT CARET ROUND: on Wagtail, a BLOCK-mode caret routes through the
/// NEW `caret_invert` pipeline (true inverse-video, same mechanism as
/// `selection_invert` above) instead of the ordinary `caret_pipeline` — the
/// fix for "a white block over a white glyph erases the glyph" (see
/// `caret_invert`'s own field doc + `dither.rs`'s real-pixel readability
/// test for the actual bug fixture). On an ordinary world (Tawny), it's the
/// other way around: `caret_pipeline` draws the real block and `caret_invert`
/// stays idle. Instance-count seam only — see `dither.rs` for the REAL-PIXEL
/// proof that the flip actually keeps a glyph legible.
#[test]
fn wagtail_caret_uses_the_invert_pipeline_other_worlds_use_the_ordinary_block() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!("skipping wagtail_caret_uses_the_invert_pipeline_other_worlds_use_the_ordinary_block: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    crate::caret::set_mode(CaretMode::Block);

    let v = view("hello world\n", 0, 3);

    theme::set_active_by_name("Wagtail").unwrap();
    p.set_view(&v);
    p.settle_caret();
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert!(
        !p.caret_pipeline.is_drawn(),
        "Wagtail (one-bit): the ordinary block pipeline must draw NOTHING — an opaque \
         pre-text quad here would hand the invert pass a uniform-white destination"
    );
    assert_eq!(
        p.caret_invert.instance_count(),
        1,
        "Wagtail (one-bit): the caret's own true-inverse-video quad must carry exactly \
         one instance — this frame's animated rect"
    );

    theme::set_active_by_name("Tawny").unwrap();
    p.set_view(&v);
    p.settle_caret();
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert!(
        p.caret_pipeline.is_drawn(),
        "Tawny: the ordinary block pipeline must carry the real caret quad, unchanged"
    );
    assert_eq!(
        p.caret_invert.instance_count(),
        0,
        "Tawny: caret_invert stays idle on an ordinary (non-one-bit) world"
    );

    crate::caret::set_mode(CaretMode::Block);
    theme::set_active(theme::DEFAULT_THEME);
}

/// MORPH-IN-ONE-BIT FALLS BACK TO THE INVERTED BLOCK (documented call — see
/// `caret_invert`'s field doc + `prepare_caret_layer`'s mode override): on
/// Wagtail, settled on a real inhabited glyph, Morph mode does NOT paint its
/// usual glyph-silhouette recolor (`caret_glyph_pipeline`) — on a one-bit
/// world that recolor is `primary` == the SAME pure white as the glyph's own
/// ink, an invisible no-op — it degrades to the SAME block-invert path a
/// plain Block-mode caret uses. Contrasted against Tawny under the IDENTICAL
/// view + mode, where the ordinary silhouette still paints, proving the
/// degrade is theme-specific, not a global regression to Morph itself.
#[test]
fn wagtail_morph_caret_falls_back_to_the_inverted_block_not_the_invisible_silhouette() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!("skipping wagtail_morph_caret_falls_back_to_the_inverted_block_not_the_invisible_silhouette: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    crate::caret::set_mode(CaretMode::Morph);

    // "ab|c": col 2 anchors the 'b' glyph one back — a real inhabited glyph,
    // settled (no in-flight glide), so Morph's silhouette branch is the one
    // that would otherwise fire.
    let v = view("abc\n", 0, 2);

    theme::set_active_by_name("Wagtail").unwrap();
    p.set_view(&v);
    p.settle_caret();
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert!(
        !p.caret_glyph_pipeline.is_drawn(),
        "Wagtail (one-bit): Morph's glyph-silhouette pipeline must NOT paint — it would \
         recolor the letter to the SAME pure white as its own ink, an invisible no-op"
    );
    assert!(
        !p.caret_pipeline.is_drawn(),
        "Wagtail (one-bit): the ordinary block pipeline must also stay empty (the invert \
         pass takes over, exactly like plain Block mode)"
    );
    assert_eq!(
        p.caret_invert.instance_count(),
        1,
        "Wagtail (one-bit): Morph degrades to the SAME block-invert quad Block mode uses"
    );

    theme::set_active_by_name("Tawny").unwrap();
    p.set_view(&v);
    p.settle_caret();
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert!(
        p.caret_glyph_pipeline.is_drawn(),
        "Tawny: settled on a real glyph, Morph's own silhouette must still paint, unchanged"
    );
    assert_eq!(
        p.caret_invert.instance_count(),
        0,
        "Tawny: caret_invert stays idle — Morph never degrades on an ordinary world"
    );

    crate::caret::set_mode(CaretMode::Block);
    theme::set_active(theme::DEFAULT_THEME);
}

/// THE ONE WAGTAIL HIGHLIGHT TEXTURE's dither mode switches ON (a nonzero
/// density) for BOTH its consumers — `wash_highlight_pipeline`
/// (`==highlight==` spans) and `match_pipeline` (search matches) — on a
/// one-bit world, and OFF (density exactly `0.0`, the ordinary alpha fill) on
/// every other world. The REAL-PIXEL proof that dither mode only ever paints
/// pure values lives in `dither.rs`; this is the cheaper instance-level
/// seam — does the theme switch actually flip the mode at all.
#[test]
fn wagtail_turns_on_highlight_and_match_dither_mode_other_worlds_leave_it_off() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping wagtail_turns_on_highlight_and_match_dither_mode_other_worlds_leave_it_off: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    theme::set_active_by_name("Wagtail").unwrap();
    p.sync_theme_colors();
    assert!(
        p.wash_highlight_pipeline.dither() > 0.0,
        "Wagtail (one-bit): the highlight wash's dither mode must be ON"
    );
    assert!(
        p.match_pipeline.dither() > 0.0,
        "Wagtail (one-bit): the search-match pipeline's dither mode must be ON — \
         the SAME one texture as the highlight wash"
    );
    assert_eq!(
        p.wash_highlight_pipeline.dither(),
        p.match_pipeline.dither(),
        "the two dither consumers must share the identical density — one texture, one meaning"
    );

    theme::set_active_by_name("Tawny").unwrap();
    p.sync_theme_colors();
    assert_eq!(
        p.wash_highlight_pipeline.dither(),
        0.0,
        "Tawny: the highlight wash's dither mode must be OFF (the ordinary alpha fill)"
    );
    assert_eq!(
        p.match_pipeline.dither(),
        0.0,
        "Tawny: the search-match pipeline's dither mode must be OFF"
    );

    // Restore the default world so other tests see a clean global.
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme_colors();
}

/// THE PALETTE CARD, at the REAL-PIXEL seam (mirrors `dither.rs`'s own
/// style — "does the renderer actually behave the way the palette promises",
/// not just the instance-count proxy the sibling tests above use): a Wagtail
/// command-palette capture must show a crisp WHITE border ring hugging the
/// card's edge and a PURE BLACK interior — the same "border, not fill" 1-bit
/// elevation answer `theme::worlds::WAGTAIL`'s doc comment describes, and the
/// menu-bar dropdown already carries (the mechanism this round extends to the
/// centered-overlay family). The card fill (`base_300`) is pure black on
/// Wagtail (flush with the canvas — ink text stays legible), so the border's
/// OWN ~1px `smoothstep` antialiased edge (`shaders/selection.wgsl::fs_main`)
/// is the ONLY thing between the card and the identically-black backdrop —
/// its measured peak (220/255, empirically sampled, comfortably distinct
/// from pure black) is asserted with a safety margin rather than a brittle
/// exact 255, since the 1-bit LAW itself excepts "anti-aliased glyph/quad
/// edges" from the pure-black/white requirement (`worlds.rs::WAGTAIL`'s own
/// wording) — a hard 2px BAND either side of the ring stays exactly pure
/// black, proving this is a crisp RING, not a wide wash.
#[test]
fn wagtail_palette_card_real_pixels_show_a_white_border_ring_black_interior() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!(
            "skipping wagtail_palette_card_real_pixels_show_a_white_border_ring_black_interior: no wgpu adapter"
        );
        return;
    };
    let _g = crate::testlock::serial();

    let mut v = view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = vec!["Save".into(), "Undo".into(), "Redo".into()];
    v.overlay_selected = 0;

    theme::set_active_by_name("Wagtail").unwrap();
    p.sync_theme();
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();

    let rect = p.overlay_card_rect().expect("the centered overlay card must be open");
    let [card_x, card_y, card_w, card_h] = rect;

    let (texture, tview) = super::dither::offscreen(&device, 1200, 800);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("awl one-bit palette-card encoder"),
    });
    p.render(&mut encoder, &tview).unwrap();
    queue.submit(Some(encoder.finish()));
    let pixels = super::dither::read_pixels(&device, &queue, &texture, 1200, 800);
    let at = |x: i64, y: i64| pixels[(y as u32 * 1200 + x as u32) as usize];

    let is_white_ish = |px: [u8; 4]| {
        px[3] == 255 && px[0] >= 180 && px[0] == px[1] && px[1] == px[2]
    };
    let pure_black = [0u8, 0, 0, 255];

    // LEFT edge: the border's true edge sits at `card_x - 1` (the reusable
    // primitive's `set_float_quads` overhang) — sampled in the card's BOTTOM
    // PADDING (a few px above the foot). Deliberately NOT the vertical middle:
    // the selected row draws as TRUE INVERTED VIDEO on a one-bit world (white
    // ground), so a "must be black interior" sample has to sit on plain
    // `base_300` fill — the bottom pad below the last row always is, at any card
    // height (the PALETTE-COMPOSITION round's header gap made the middle of a
    // short flat card land on the inverted selected row).
    let mid_y = (card_y + card_h - 4.0) as i64;
    let ring_x = (card_x - 1.0) as i64;
    assert!(
        is_white_ish(at(ring_x, mid_y)),
        "left border ring at x={ring_x} must read white-ish, got {:?}",
        at(ring_x, mid_y)
    );
    assert_eq!(
        at(ring_x - 2, mid_y),
        pure_black,
        "2px outside the left ring must be pure black — a crisp ring, not a wide wash"
    );
    assert_eq!(
        at(ring_x + 2, mid_y),
        pure_black,
        "2px inside the left ring (deep in the card fill) must be pure black — `base_300` \
         is pure black on Wagtail, flush with the canvas"
    );

    // TOP edge: same overhang, vertical side — proves the ring wraps the
    // whole card, not just the two side columns sampled above.
    let mid_x = (card_x + card_w * 0.5) as i64;
    let ring_y = (card_y - 1.0) as i64;
    assert!(
        is_white_ish(at(mid_x, ring_y)),
        "top border ring at y={ring_y} must read white-ish, got {:?}",
        at(mid_x, ring_y)
    );
    assert_eq!(
        at(mid_x, ring_y - 2),
        pure_black,
        "2px above the top ring must be pure black"
    );

    // INTERIOR: well inside the card's left PAD (12px — before any glyph, at
    // `text_left = card_x + 12`), far from any edge's antialiasing — pure
    // black fill, unambiguous.
    let interior_px = at((card_x + 5.0) as i64, mid_y);
    assert_eq!(
        interior_px, pure_black,
        "the card interior must be pure black (base_300 flush with the canvas), got {interior_px:?}"
    );

    theme::set_active(theme::DEFAULT_THEME);
}

/// ARM B LIVING-BAND — the FILL/INK DIVERGENCE law, at the REAL-PIXEL seam (the
/// Wagtail-tripwire class: the sidecar/state oracle can't see it, only bytes
/// can). On a 1-bit world the moving selection band is a SOLID WHITE fill; the
/// covered rows' glyphs must flip to BLACK so they read on it. The bug this
/// guards: the ink flip used to bail on the FACETED (`geom.theme`) layout — the
/// Cmd-P palette IS faceted — so the band FILL animated over the palette while
/// the ink stayed state-tied to the settled selected row, painting white glyphs
/// on the white band (white-on-white, invisible) for every row the band passed
/// that wasn't the selection. This forces the palette faceted layout (a
/// non-empty `overlay_lens`), pins a mid-flight morph phase, and asserts, over
/// the rendered pixels: (1) every row the band majority-covers carries BLACK
/// glyph pixels sitting WITHIN the white band (legible, not washed out), and
/// (2) the not-yet-reached TARGET row keeps WHITE (unflipped) glyph pixels — its
/// ink did NOT prematurely flip to black-on-black. Morph (single band) so the
/// band region is unambiguous.
#[test]
fn wagtail_living_band_ink_rides_the_band_on_the_faceted_palette() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!("skipping wagtail_living_band_ink_rides_the_band_on_the_faceted_palette: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    use crate::render::livingband::{Choreo, MotionForce};

    // FACETED palette: a non-empty lens strip routes `overlay_geometry` →
    // `theme_overlay_geometry` (`geom.theme == true`) — the exact surface the
    // old gate killed. TARGET = a MIDDLE row: the pinned band climbs from
    // PIN_JUMP_ROWS BELOW it, so early in the flight it sits over rows BELOW the
    // target (still on-card → covered) while the leading edge hasn't yet reached
    // the target (target clear). Rows below the target must exist on-card, hence
    // a middle selection with a full list.
    let mut v = view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_title = "commands";
    v.overlay_lens = vec![("All".into(), true), ("File".into(), false)];
    v.overlay_items = vec![
        "Save all files".into(),
        "Open recent".into(),
        "Close window".into(),
        "Find and replace".into(),
        "Toggle sidebar".into(),
        "Command palette".into(),
    ];
    v.overlay_selected = 2;

    theme::set_active_by_name("Wagtail").unwrap();
    // An EARLY morph phase: the band has just left its start row (PIN_JUMP_ROWS
    // below the target) and covers the rows immediately below the target, whose
    // leading edge has not yet climbed onto the target — so the target stays
    // clear (asserted below, not assumed). Mirrors the unit test's t=0.1 regime.
    crate::render::livingband::set_motion_test_override(Some(MotionForce {
        choreo: Choreo::Morph,
        phase: Some(0.1),
    }));

    p.sync_theme();
    p.set_view(&v);

    let geom = p.overlay_geometry(1200);
    assert!(p.overlay_geom_is_faceted(&geom), "the lens strip must route the FACETED layout");
    let (covered, target, first_top, lh, band) = p.living_probe_geom(&geom);
    let [_bx, band_top, _bw, band_h] = band;
    let band_bot = band_top + band_h;

    // Preconditions the pixel law rests on: the band covers rows the fill
    // animates over, and the TOP target is NOT among them (mid-flight).
    assert!(!covered.is_empty(), "mid-flight band must cover at least one row (got {covered:?})");
    assert!(
        !covered.contains(&target),
        "the not-yet-reached target row {target} must NOT be covered mid-flight (covered {covered:?})"
    );

    p.prepare(&device, &queue, 1200, 800).unwrap();
    let [card_x, _cy, card_w, _ch] = p.overlay_card_rect().expect("the faceted palette card must be open");
    let (texture, tview) = super::dither::offscreen(&device, 1200, 800);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("awl living-band ink-rides encoder"),
    });
    p.render(&mut encoder, &tview).unwrap();
    queue.submit(Some(encoder.finish()));
    let pixels = super::dither::read_pixels(&device, &queue, &texture, 1200, 800);
    let at = |x: i64, y: i64| pixels[(y as u32 * 1200 + x as u32) as usize];
    // Pane world: text inset is the card pad (12px, `overlay_text_hpad == pad`).
    let x0 = (card_x + 14.0) as i64;
    let x1 = (card_x + card_w - 6.0) as i64;
    let near_white = |px: [u8; 4]| px[3] == 255 && px[0] >= 170 && px[1] >= 170 && px[2] >= 170;
    let near_black = |px: [u8; 4]| px[3] == 255 && px[0] <= 80 && px[1] <= 80 && px[2] <= 80;

    // (1) EVERY covered row: within the band's overlap with that row (all white
    // fill), there MUST be BLACK glyph pixels — the flipped ink reading on the
    // band. A row with the white band but zero black glyphs IS the invisible bug.
    for &k in &covered {
        let row_top = first_top + k as f32 * lh;
        let row_bot = row_top + lh;
        // The slab that is genuinely UNDER the band (so any black there is glyph
        // ink, never the black card ground): the row∩band intersection.
        let y_lo = row_top.max(band_top).ceil() as i64;
        let y_hi = row_bot.min(band_bot).floor() as i64;
        assert!(y_hi > y_lo, "covered row {k} must have a real band overlap slab");
        let mut white = 0usize;
        let mut black = 0usize;
        for y in y_lo..y_hi {
            for x in x0..x1 {
                let px = at(x, y);
                if near_white(px) {
                    white += 1;
                } else if near_black(px) {
                    black += 1;
                }
            }
        }
        assert!(white > 200, "covered row {k}: the white band fill must be present (white px {white})");
        assert!(
            black > 40,
            "covered row {k}: BLACK glyph pixels must ride the white band (got {black}); \
             white-on-white here is the faceted-palette invisible-ink bug"
        );
    }

    // (2) The TARGET row keeps its WHITE (unflipped) glyph ink on the black
    // ground — legible, and proof the flip did NOT fire early. Sample the row's
    // own y-band (clear of the band). Erroneous black-on-black would leave ~0
    // white glyph pixels here.
    let t_top = (first_top + target as f32 * lh + 3.0) as i64;
    let t_bot = (first_top + (target as f32 + 1.0) * lh - 3.0) as i64;
    let mut target_white = 0usize;
    for y in t_top..t_bot {
        for x in x0..x1 {
            if near_white(at(x, y)) {
                target_white += 1;
            }
        }
    }
    assert!(
        target_white > 40,
        "the not-yet-reached target row {target} must keep WHITE glyph pixels (got {target_white}); \
         a premature flip would paint them black-on-black (invisible)"
    );

    crate::render::livingband::set_motion_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);
}

/// THE CENTERED-OVERLAY FAMILY, at the NO-WILDCARD [`crate::overlay::OverlayKind`]
/// seam: every summoned picker EXCEPT the contextual SPELL popup (which floats
/// at the misspelled word on its own float-panel primitive, UNCONDITIONALLY
/// elevated in every world — see `chrome_panels.rs`'s
/// `spell_panel_floats_at_the_word_not_center_screen`) rides `panel_card` +
/// its `panel_border`/`panel_shadow` companions
/// (`TextPipeline::prepare_panel_card_elevation`). This mirrors the SAME
/// production fact `app/viewstate.rs` encodes (`overlay_spell` is `Some`
/// IFF `o.kind == OverlayKind::Spell` — `overlay/state.rs::new_spell` is the
/// only constructor that ever sets `spell_target`) with a NO-WILDCARD match,
/// so a future 16th `OverlayKind` fails to compile here until someone
/// decides which elevation family it joins — the same "merge, don't align"
/// law-test shape as `accept_disposition`/`hides_dotfiles` in
/// `overlay/kind.rs` itself. The render layer genuinely cannot distinguish
/// among the 14 non-spell kinds (`ViewState` carries no `OverlayKind` field
/// at all, only the derived `overlay_spell`), so this sweep classifies every
/// kind once, then drives ONE real render per family and asserts BOTH halves
/// of the law: `panel_border` gains instances on Wagtail (the fix) and stays
/// at ZERO on Tawny (the ordinary-world byte-identity guarantee).
#[test]
fn every_overlay_kind_is_classified_and_the_two_families_render_as_declared() {
    use crate::overlay::OverlayKind;

    #[derive(PartialEq, Eq)]
    enum CardFamily {
        /// Rides the shared float-panel primitive (`float_shadow`/`float_border`/
        /// `float_card`), unconditionally elevated — today only `Spell`.
        FloatAnchored,
        /// Rides `panel_card` + `panel_shadow`/`panel_border`, elevated (bordered)
        /// ONLY on a true 1-bit world.
        CenteredPanel,
    }

    let mut spell_count = 0usize;
    let mut centered_count = 0usize;
    for kind in OverlayKind::ALL {
        let family = match kind {
            OverlayKind::Spell => CardFamily::FloatAnchored,
            OverlayKind::Goto
            | OverlayKind::Project
            | OverlayKind::Browse
            | OverlayKind::Theme
            | OverlayKind::Caret
            | OverlayKind::MoveDest
            | OverlayKind::Dictionary
            | OverlayKind::CjkLang
            | OverlayKind::Command
            | OverlayKind::Keybindings
            | OverlayKind::History
            | OverlayKind::Settings
            | OverlayKind::Assets
            | OverlayKind::Rename
            | OverlayKind::InsertLink
            | OverlayKind::KeepName => CardFamily::CenteredPanel,
        };
        match family {
            CardFamily::FloatAnchored => spell_count += 1,
            CardFamily::CenteredPanel => centered_count += 1,
        }
    }
    assert_eq!(spell_count, 1, "exactly one kind (Spell) floats at its own anchor");
    assert_eq!(
        centered_count,
        OverlayKind::ALL.len() - 1,
        "every other kind belongs to the centered `panel_card` family"
    );

    // Drive ONE real render per family (the render layer cannot distinguish
    // further — see this test's own doc) and assert the elevation law holds
    // on both a 1-bit world (Wagtail) and an ordinary one (Tawny).
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!(
            "skipping every_overlay_kind_is_classified_and_the_two_families_render_as_declared: no wgpu adapter"
        );
        return;
    };
    let _g = crate::testlock::serial();

    // CenteredPanel representative: any non-spell overlay (`overlay_spell: None`).
    let mut centered = view("hello world\n", 0, 0);
    centered.overlay_active = true;
    centered.overlay_items = vec!["Save".into(), "Undo".into(), "Redo".into()];

    // FloatAnchored representative: the Spell popup.
    let mut spell = view("teh quick brown fox\n", 0, 0);
    spell.overlay_active = true;
    spell.overlay_items = vec!["the".into(), "tea".into()];
    spell.overlay_spell = Some((0, 0, 3));

    for world in ["Wagtail", "Tawny"] {
        theme::set_active_by_name(world).unwrap();
        p.sync_theme();

        p.set_view(&centered);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        let panel_border_n = p.panel_border.instance_count();
        if world == "Wagtail" {
            assert!(
                panel_border_n > 0,
                "Wagtail (one-bit): the CenteredPanel family's `panel_border` must draw"
            );
        } else {
            assert_eq!(
                panel_border_n, 0,
                "{world}: the CenteredPanel family's `panel_border` must stay parked — \
                 byte-identical to the pre-round flat card"
            );
        }
        assert!(p.panel_card.instance_count() > 0, "{world}: the card fill itself always draws");

        p.set_view(&spell);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        assert!(
            p.float_border.instance_count() > 0,
            "{world}: the FloatAnchored (Spell) family's border is UNCONDITIONAL — \
             pre-existing behaviour this round does not touch"
        );
    }

    theme::set_active(theme::DEFAULT_THEME);
}

/// LIGHT-WORLD BORDER LAW (composition round item 6, veto 3 adopted: "border on
/// light worlds totally works") — every LIGHT world (a pale ground,
/// `dark == false`) carries [`theme::Elevation::Bordered`], so the summoned
/// card's soft fill gains a crisp rim off the pale ground instead of dissolving
/// into it. STRUCTURAL sweep over the whole roster (a light world may never ship
/// Flat again — DATA, no code path), PLUS a real render asserting `panel_border`
/// draws on a light world and stays parked on a Flat dark world (the OUTCOME,
/// the Wagtail lesson).
#[test]
fn light_worlds_carry_the_summoned_card_border() {
    // STRUCTURAL: every light-ground world is Bordered (Wagtail is dark/one-bit,
    // so the `dark == false` gate excludes it — it carries its own border via
    // the one-bit path already law-tested above).
    for t in theme::THEMES.iter() {
        if !t.dark {
            assert_eq!(
                t.render_caps.elevation,
                theme::Elevation::Bordered,
                "{}: a light-ground world must carry the summoned-card border",
                t.name
            );
        }
    }

    // OUTCOME: the border actually draws on a light world, and a Flat dark world
    // keeps its border parked (byte-identical to before this round).
    //
    // FLIP ROUND (2026-07-17): the OUTCOME worlds must be PANE light worlds — a
    // world that ships `ListStyle::Bars` (Galah / Magpie / Mangrove / Firetail
    // now do) DROPS the pane entirely (bars float on the room), so its
    // `panel_border` is legitimately parked even though its `elevation` DATA
    // stays `Bordered` (inert under Bars). Saltpan + Bilby are calm PANE light
    // worlds, so their card border still draws — the property under test.
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!("skipping light_worlds_carry_the_summoned_card_border: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    let mut v = view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = vec!["Save".into(), "Undo".into(), "Redo".into()];
    v.overlay_selected = 0;
    for (world, want_border) in [("Saltpan", true), ("Bilby", true), ("Tawny", false)] {
        theme::set_active_by_name(world).unwrap();
        p.sync_theme();
        p.set_view(&v);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        let n = p.panel_border.instance_count();
        if want_border {
            assert!(n > 0, "{world}: the light-world card border must draw (got {n} instances)");
        } else {
            assert_eq!(n, 0, "{world}: a Flat dark world keeps `panel_border` parked");
        }
    }
    theme::set_active(theme::DEFAULT_THEME);
}

/// THE PICKER-ROW-HIGHLIGHT report's selected-row half, at the INSTANCE-COUNT
/// seam. The old framebuffer invert (`overlay_rows_invert`) is RETIRED: every
/// world — 1-bit included — now drives the ONE `overlay_rows` fill pipeline for
/// the selected band, its COLOR the only thing `HighlightTreatment` changes
/// (solid `base_content` on a 1-bit world, the value band elsewhere) and the
/// selected row's own glyphs recolored in the shaper. So `overlay_rows` carries
/// exactly ONE instance on BOTH a 1-bit and an ordinary world — the
/// "prepare neither pipeline" hole the old two-pipeline split could represent is
/// closed by construction (there is only one pipeline left to prepare).
#[test]
fn wagtail_picker_selected_band_rides_the_one_fill_pipeline_like_every_world() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!(
            "skipping wagtail_picker_selected_band_rides_the_one_fill_pipeline_like_every_world: no wgpu adapter"
        );
        return;
    };
    let _g = crate::testlock::serial();

    let mut v = view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = vec!["Save".into(), "Undo".into(), "Redo".into()];
    v.overlay_selected = 0;

    for world in ["Wagtail", "Tawny"] {
        theme::set_active_by_name(world).unwrap();
        p.sync_theme();
        p.set_view(&v);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        assert_eq!(
            p.overlay_rows.instance_count(),
            1,
            "{world}: the ONE fill pipeline carries the selected-row band (the invert \
             pipeline is retired — a 1-bit world differs only in the band COLOR + its \
             recolored row glyphs, not in which pipeline draws)"
        );
    }

    theme::set_active(theme::DEFAULT_THEME);
}

/// THE ROUND'S OWN motivating bug PLUS the repair round's contrast fix, at the
/// REAL-PIXEL seam. On Wagtail the selected picker row is a SOLID `base_content`
/// (white) fill band with the row's own glyphs recolored to solid `base_300`
/// (black): a crisp black-on-white pair. This test reads the rendered pixels and
/// asserts BOTH halves:
///   1. the selected row carries a WIDE run of pure-white band pixels (the solid
///      fill — never the uniform all-black the original transparent band gave),
///      AND
///   2. the row's text strokes reach GENUINELY NEAR-BLACK. The retired
///      framebuffer invert bottomed out at a gamma-limited mid-grey (~sRGB 83,
///      the `1 - dst` flip of antialiased near-white strokes); a solid black
///      recolor lands far darker. The threshold (`< 60`) sits below that grey
///      floor, so a regression back to the invert (or a half-covered band, the
///      pre-alignment bug where the row's glyph tops fell on the black card and
///      only their bottoms reached the white) fails the assertion.
/// Also proves the highlight FOLLOWS the selection: after moving down, the old
/// row loses its white band and the new row picks it up.
#[test]
fn wagtail_picker_selected_row_is_crisp_black_on_a_solid_white_band() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!(
            "skipping wagtail_picker_selected_row_is_crisp_black_on_a_solid_white_band: no wgpu adapter"
        );
        return;
    };
    let _g = crate::testlock::serial();

    let mut v = view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = vec!["Save".into(), "Undo".into(), "Redo".into()];
    v.overlay_selected = 0;

    theme::set_active_by_name("Wagtail").unwrap();
    p.sync_theme();
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    let [card_x, card_y, card_w, card_h] = p.overlay_card_rect().expect("the card must be open");

    // Read every rendered pixel once, then locate the selected row's band by
    // SCANNING for it (the black card makes a pure-white fill unmistakable) —
    // robust to the exact row geometry (header rows + the header_gap divider),
    // which the old fixed `text_top + lh*row` math got wrong once the gap landed.
    let read = |p: &TextPipeline, device: &wgpu::Device, queue: &wgpu::Queue| -> Vec<[u8; 4]> {
        let (texture, tview) = super::dither::offscreen(device, 1200, 800);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("awl one-bit picker-row encoder"),
        });
        p.render(&mut encoder, &tview).unwrap();
        queue.submit(Some(encoder.finish()));
        super::dither::read_pixels(device, queue, &texture, 1200, 800)
    };
    let x0 = card_x as i64;
    let x1 = (card_x + card_w) as i64;
    let y0 = card_y as i64;
    let y1 = (card_y + card_h) as i64;
    // Per row inside the card: the count of pure-white pixels (the solid band
    // ground) and the DARKEST luminance seen (the recolored black text).
    let white_run = |pixels: &[[u8; 4]], y: i64| -> usize {
        (x0..x1).filter(|&x| pixels[(y as u32 * 1200 + x as u32) as usize] == [255, 255, 255, 255]).count()
    };
    let darkest_on = |pixels: &[[u8; 4]], y: i64| -> u8 {
        (x0..x1)
            .map(|x| pixels[(y as u32 * 1200 + x as u32) as usize][0])
            .min()
            .unwrap_or(255)
    };
    // The band row = the card row with the widest pure-white run.
    let band_row = |pixels: &[[u8; 4]]| -> (i64, usize) {
        (y0..y1).map(|y| (y, white_run(pixels, y))).max_by_key(|&(_, w)| w).unwrap()
    };

    let pixels0 = read(&p, &device, &queue);
    let (by0, bw0) = band_row(&pixels0);
    assert!(
        bw0 > (card_w as usize) / 2,
        "Wagtail selected band must be a WIDE solid-white fill — widest white run {bw0}px \
         of a {}px card (a transparent/partial band would leave the row black)",
        card_w as usize
    );
    // The text is on the band rows. Sweep the few rows around the widest-white
    // row for the darkest stroke — solid black recolor reaches far below the
    // old invert's ~83 gamma-grey floor.
    let darkest0 = (by0 - 8..=by0 + 8).map(|y| darkest_on(&pixels0, y)).min().unwrap();
    assert!(
        darkest0 < 60,
        "Wagtail selected row text must be CRISP near-black on the white band — darkest \
         stroke sRGB {darkest0} (the retired invert bottomed out at ~83 mid-grey; \
         a value ≥ 60 means the recolor regressed or the band no longer covers the glyphs)"
    );

    // Move the selection down one and re-derive: the white band FOLLOWS it.
    v.overlay_selected = 1;
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    let (_, _, sel_row2, _, _) = p.overlay_window_report().unwrap();
    assert_eq!(sel_row2, 1, "the selection moved to row 1");
    let pixels1 = read(&p, &device, &queue);
    let (by1, bw1) = band_row(&pixels1);
    assert!(
        bw1 > (card_w as usize) / 2,
        "the now-selected row 1 must carry the same wide white band — got {bw1}px"
    );
    assert!(
        (by1 - by0).abs() >= 4,
        "the white band must MOVE to the newly selected row (was y={by0}, now y={by1})"
    );
    assert!(
        white_run(&pixels1, by0) < bw0,
        "the previously selected row must lose its white band once the highlight moves off it \
         (row y={by0}: was {bw0}px white, now {}px)",
        white_run(&pixels1, by0)
    );

    theme::set_active(theme::DEFAULT_THEME);
}

/// THE OTHER NON-OVERLAY SUMMONED CARDS (HUD / About / the menu-bar dropdown)
/// already rode the shared float-panel primitive UNCONDITIONALLY before this
/// round (see `render.rs`'s `hud_shadow`/`hud_border`/`hud_card` and
/// `menu_drop_shadow`/`menu_drop_border`/`menu_drop_card` construction) — this
/// is the reference case the user's own report named as ALREADY working
/// ("the menu-bar dropdown shows the border"). Asserted here alongside the
/// palette fix so the full "every summoned card" enumeration the round asked
/// for lives in one place, not scattered.
#[test]
fn hud_about_and_menu_dropdown_already_carry_unconditional_elevation() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!(
            "skipping hud_about_and_menu_dropdown_already_carry_unconditional_elevation: no wgpu adapter"
        );
        return;
    };
    let _g = crate::testlock::serial();

    theme::set_active_by_name("Wagtail").unwrap();
    p.sync_theme();

    // HUD.
    crate::hud::set_held(true);
    let v = view("hello world\n", 0, 0);
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert!(p.hud_border.instance_count() > 0, "Wagtail: the held HUD's border must draw");
    crate::hud::set_held(false);

    // About (shares the SAME hud_* pipelines, gated on `about::about_open()`).
    crate::about::set_open(true);
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert!(p.hud_border.instance_count() > 0, "Wagtail: the About card's border must draw");
    crate::about::set_open(false);

    // Menu-bar dropdown (the user's own confirmed-working reference case).
    crate::menubar::set_menu_bar_on(true);
    crate::menubar::set_open(Some(0));
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert!(
        p.menu_drop_border.instance_count() > 0,
        "Wagtail: the menu-bar dropdown's border must draw (the pre-existing reference case)"
    );
    crate::menubar::set_open(None);
    crate::menubar::set_menu_bar_on(false);

    theme::set_active(theme::DEFAULT_THEME);
}
