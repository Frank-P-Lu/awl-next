//! Render-pipeline BEHAVIORAL tests for the TRUE 1-BIT world (Wagtail's
//! 2026-07 greyscale -> 1-bit rework): the frosted-blur backdrop disabling
//! itself, and the selection "punch" outline mechanism actually drawing (and
//! staying idle on every other world). The PALETTE-literal laws (every
//! authored color is exactly `#000000`/`#FFFFFF`) live in `syntax_roles.rs`
//! (`every_one_bit_world_renders_only_pure_black_or_white`); this file is the
//! GPU-pipeline half — does the renderer actually behave the way the palette
//! promises.

use super::super::*;
use super::{headless_pipeline, view};

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

/// THE SELECTION "PUNCH" OUTLINE — the least-bad 2-value selection Wagtail
/// ships this round (see `worlds.rs::WAGTAIL`'s doc comment for the full "why
/// not real inversion" investigation): document text selection stays the
/// EXISTING `selection_pipeline` mechanism (now authored pure opaque white on
/// Wagtail), with a SECOND, otherwise-idle `selection_punch` pipeline
/// carving a smaller pure-black rect out of each selected row so the covered
/// text stays legible. Idle (zero instances) on every other world.
#[test]
fn wagtail_selection_draws_the_punch_outline_other_worlds_stay_idle() {
    let got = pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl one-bit selection-punch test device"),
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
        eprintln!("skipping wagtail_selection_draws_the_punch_outline_other_worlds_stay_idle: no wgpu adapter");
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
        p.selection_punch.instance_count() as usize,
        sel_rects.len(),
        "Wagtail (one-bit): the punch draws one inset quad per selected-line rect"
    );

    theme::set_active_by_name("Tawny").unwrap();
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert_eq!(
        p.selection_punch.instance_count(),
        0,
        "Tawny: the punch pipeline stays idle on an ordinary (non-one-bit) world"
    );

    // Restore the default world so other tests see a clean global.
    theme::set_active(theme::DEFAULT_THEME);
}

/// A selection rect too small to inset (narrower/shorter than twice the punch
/// inset) is skipped rather than producing a negative-size instance — pure
/// unit coverage of `inset_rect` itself via a degenerate zero-height rect.
#[test]
fn inset_rect_skips_a_rect_too_small_to_punch() {
    use super::super::layers::inset_rect;
    // A rect exactly as tall as 2x the inset collapses to zero height.
    assert!(inset_rect([0.0, 0.0, 20.0, 4.0], 2.0).is_none());
    // A comfortably large rect insets cleanly.
    let got = inset_rect([10.0, 10.0, 20.0, 20.0], 2.0);
    assert_eq!(got, Some([12.0, 12.0, 16.0, 16.0]));
}
