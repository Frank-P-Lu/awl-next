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
