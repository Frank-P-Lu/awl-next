//! THE FROST DPI PIXEL-GEOMETRY LAW — Frost is authored in logical pixels but
//! its lava-shader uniforms and pill rectangles are physical pixels. This is the
//! render-side proof that a 2× surface keeps the same logical outline treatment:
//! it must produce the same number of pills, with every physical edge exactly
//! doubled. The companion pure `lava::tests` law covers blur/feather/padding
//! values; this test covers the actual text-derived pill geometry they surround.

use super::super::*;
use super::{headless_dqp, view_md};

/// THE 1×/2× FROST PILL LAW: render-equivalent logical pages at a pair of device
/// scales. Outline labels, row bands, and the Frost padding are all measured in
/// physical pixels, so each rect must double edge-for-edge at 2×. Without the
/// DPI term in `frost_px`, the two horizontal padding edges fail this arithmetic
/// even though the text interior itself doubles.
#[test]
fn frost_pill_geometry_is_dpi_invariant_in_logical_space() {
    const W: f32 = 960.0;
    const H: f32 = 640.0;
    const ZOOM: f32 = 1.25;
    let Some((_device, _queue, mut p)) = headless_dqp(W, H) else {
        eprintln!(
            "skipping frost_pill_geometry_is_dpi_invariant_in_logical_space: no wgpu adapter"
        );
        return;
    };
    let _g = crate::testlock::serial();
    let was_page_on = crate::page::page_on();
    let was_measure = crate::page::measure();
    let was_outline_on = crate::outline::outline_on();
    let was_theme = crate::theme::active_index();
    crate::page::set_page_on(true);
    crate::page::set_measure(28);
    crate::outline::set_outline_on(true);
    let lava_idx = crate::theme::THEMES
        .iter()
        .position(|t| t.background.is_lava())
        .expect("a lava world ships");
    crate::theme::set_active(lava_idx);
    p.sync_theme();

    let mut v = view_md("# Title\n\n## Section\n\n### Detail\n", 0, 0);
    v.zoom = ZOOM;
    let geometry_at = |p: &mut TextPipeline, dpi: f32| {
        // Mirror the live startup order: physical surface size first, then its
        // scale factor; set_view applies the user zoom on top of that factor.
        p.set_size(W * dpi, H * dpi);
        p.set_dpi(dpi);
        p.set_view(&v);
        let pills = p.lava_frost_pill_rects((H * dpi) as u32);
        (
            pills,
            crate::lava::frost_px(crate::lava::FROST_PILL_PAD_X, ZOOM, dpi),
        )
    };
    let (one, pad_one) = geometry_at(&mut p, 1.0);
    let (two, pad_two) = geometry_at(&mut p, 2.0);

    // Restore process-wide presentation state before any assertion can panic.
    crate::theme::set_active(was_theme);
    crate::outline::set_outline_on(was_outline_on);
    crate::page::set_page_on(was_page_on);
    crate::page::set_measure(was_measure);

    assert!(
        !one.is_empty(),
        "control: the logical page exposes Frost pills at 1×"
    );
    assert_eq!(
        one.len(),
        two.len(),
        "same logical headings -> same Frost pill count"
    );
    assert!(
        (pad_two - 2.0 * pad_one).abs() < f32::EPSILON,
        "Frost pill padding doubles in physical pixels: 1× {pad_one}, 2× {pad_two}"
    );
    for (i, (a, b)) in one.iter().zip(&two).enumerate() {
        for (edge, (one_px, two_px)) in a.iter().zip(b).enumerate() {
            assert!(
                (*two_px - 2.0 * *one_px).abs() < 0.75,
                "pill {i} edge {edge}: 2× physical geometry {two_px} must double \
                 1× {one_px}; otherwise Frost changes its logical shape"
            );
        }
    }
}
