//! THE FROST DPI PIXEL-GEOMETRY LAW — Frost is authored in logical pixels but
//! its lava-shader uniforms and pill rectangles are physical pixels. This is the
//! render-side proof that a 2× surface keeps the same logical outline treatment:
//! it must produce the same number of pills, each with its physical EXTENTS
//! (width + height) exactly doubled — absolute origins fold in the adaptive
//! rail's fixed physical floor, which does not scale with DPI, so only the span
//! is the invariant. The companion pure `lava::tests` law covers
//! blur/feather/padding values; this test covers the actual text-derived pill
//! geometry they surround.

use super::super::*;
use super::{headless_dqp, view_md};

/// THE 1×/2× FROST PILL LAW: render-equivalent logical pages at a pair of device
/// scales. Outline labels, row bands, and the Frost padding are all measured in
/// physical pixels, so each rect's WIDTH and HEIGHT must double at 2×. Without
/// the DPI term in `frost_px`, the two horizontal padding edges fail this
/// arithmetic even though the text interior itself doubles. (Absolute origins
/// carry the adaptive rail's fixed physical floor and so are NOT asserted.)
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
    // The DPI-invariant law is the pill EXTENTS (width + height), NOT the
    // absolute edge positions. A pill's `[x0, y0, x1, y1]` origin folds in the
    // adaptive rail's FIXED physical floor, which does NOT scale with DPI, so an
    // origin edge legitimately fails edge-for-edge doubling. The logical SHAPE is
    // preserved iff each pill's span doubles at 2×.
    for (i, (a, b)) in one.iter().zip(&two).enumerate() {
        let (w_one, h_one) = (a[2] - a[0], a[3] - a[1]);
        let (w_two, h_two) = (b[2] - b[0], b[3] - b[1]);
        assert!(
            (w_two - 2.0 * w_one).abs() < 0.75,
            "pill {i} width: 2× physical extent {w_two} must double 1× {w_one}; \
             otherwise Frost changes its logical shape"
        );
        assert!(
            (h_two - 2.0 * h_one).abs() < 0.75,
            "pill {i} height: 2× physical extent {h_two} must double 1× {h_one}; \
             otherwise Frost changes its logical shape"
        );
    }
}

/// THE 1×/2× FROST SEED LAW: the ORGANIC seed field (item 32) is authored from the
/// zoomed glyph geometry in LOGICAL space, so a 2× surface must produce the SAME
/// seed count with each seed's physical EXTENTS doubled — the x-span (`x1 - x0`)
/// AND the halo radius (`r`, glyph-derived row-height fraction + the DPI-scaled
/// skirt). Absolute origins fold in the adaptive rail's fixed physical floor (like
/// the pill law above), so only the SPAN + RADIUS are the invariant. Without the
/// DPI term in `frost_seed_radius`/`frost_px` the halo would keep a fixed physical
/// size and the field's logical topology would drift across displays.
#[test]
fn frost_seed_geometry_is_dpi_invariant_in_logical_space() {
    const W: f32 = 960.0;
    const H: f32 = 640.0;
    const ZOOM: f32 = 1.25;
    let Some((_device, _queue, mut p)) = headless_dqp(W, H) else {
        eprintln!("skipping frost_seed_geometry_is_dpi_invariant_in_logical_space: no wgpu adapter");
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
    let seeds_at = |p: &mut TextPipeline, dpi: f32| {
        p.set_size(W * dpi, H * dpi);
        p.set_dpi(dpi);
        p.set_view(&v);
        p.outline_frost_seeds((H * dpi) as u32)
    };
    let one = seeds_at(&mut p, 1.0);
    let two = seeds_at(&mut p, 2.0);

    crate::theme::set_active(was_theme);
    crate::outline::set_outline_on(was_outline_on);
    crate::page::set_page_on(was_page_on);
    crate::page::set_measure(was_measure);

    assert!(!one.is_empty(), "control: the logical page seeds a frost field at 1×");
    assert_eq!(one.len(), two.len(), "same logical headings -> same seed count");
    for (i, (a, b)) in one.iter().zip(&two).enumerate() {
        let (span_one, span_two) = (a[1] - a[0], b[1] - b[0]);
        assert!(
            (span_two - 2.0 * span_one).abs() < 0.75,
            "seed {i} x-span: 2× physical {span_two} must double 1× {span_one}"
        );
        assert!(
            (b[3] - 2.0 * a[3]).abs() < 0.75,
            "seed {i} halo radius: 2× physical {} must double 1× {}",
            b[3],
            a[3]
        );
    }
}

/// Build the outline + gutter frost seeds for a small doc mixing an ISOLATED
/// single-glyph heading ("&"), a long HYPHENATED single-run label
/// ("Button-free" — no internal whitespace, so it seeds as ONE run), and an
/// ordinary multi-word heading, at 100% zoom / 1x DPI, page mode + outline on.
/// The shared fixture for the item-61 punctuation-aware / bounded-end-pad law
/// tests below.
fn item61_seeds(p: &mut TextPipeline, height: u32) -> (Vec<[f32; 4]>, Vec<[f32; 4]>) {
    let text = "# &\n\n## Button-free\n\n### The quick brown fox jumps\n\n#### A\n\n##### Getting Started Guide\n";
    let mut v = view_md(text, 0, 0);
    v.zoom = 1.0;
    v.gutter_name = "item61_fixture.md".to_string();
    p.set_dpi(1.0);
    p.set_view(&v);
    (p.outline_frost_seeds(height), p.gutter_frost_seeds(height))
}

/// THE ISOLATED-`&`-IS-NOT-A-CIRCULAR-BUMP LAW (item 61): a run's radius is no
/// longer a blanket row-height fraction — [`crate::render::frost_run_radius`]
/// bounds it by the run's OWN ink geometry, so a single-glyph run's halo radius
/// sits STRICTLY BELOW the row-height radius every multi-glyph run on the same
/// page gets. This is arithmetic over the REAL seeds a live outline draws (not
/// a hand-picked fixture): the "&" heading (one glyph) and the "A" heading (one
/// glyph) both seed a radius smaller than the "Button-free" / "The quick brown
/// fox jumps" runs on the very same page — the halo now DERIVES FROM the run's
/// own advance instead of dwarfing it into a disproportionate round bump. No
/// glyph or label is identity-matched; the bound is purely geometric (ink
/// width in, radius out).
#[test]
fn isolated_punctuation_run_radius_is_bounded_below_a_normal_runs() {
    let Some((_device, _queue, mut p)) = headless_dqp(960.0, 640.0) else {
        eprintln!("skipping isolated_punctuation_run_radius_is_bounded_below_a_normal_runs: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    let was_page_on = crate::page::page_on();
    let was_measure = crate::page::measure();
    let was_outline_on = crate::outline::outline_on();
    let was_theme = crate::theme::active_index();
    crate::page::set_page_on(true);
    crate::page::set_measure(40);
    crate::outline::set_outline_on(true);
    let lava_idx = crate::theme::THEMES
        .iter()
        .position(|t| t.background.is_lava())
        .expect("a lava world ships");
    crate::theme::set_active(lava_idx);
    p.sync_theme();
    p.set_size(960.0, 640.0);

    let (seeds, _gutter) = item61_seeds(&mut p, 640);
    crate::theme::set_active(was_theme);
    crate::outline::set_outline_on(was_outline_on);
    crate::page::set_page_on(was_page_on);
    crate::page::set_measure(was_measure);

    assert_eq!(seeds.len(), 8, "the fixture's 8 word-runs (control — a seed-count drift silently changes every other assertion here)");
    // seed[0] = "&" (row 1, one glyph); seed[1] = "Button-free" (row 2, one
    // run, no internal whitespace); seed[2] = "The" (row 3, first word).
    let (amp_r, button_free_r, the_r) = (seeds[0][3], seeds[1][3], seeds[2][3]);
    assert!(
        amp_r < button_free_r,
        "the isolated '&' radius ({amp_r}) must be STRICTLY BOUNDED below an ordinary \
         long run's radius ({button_free_r}) — not the same blanket row-height value"
    );
    assert!(
        amp_r < the_r,
        "the isolated '&' radius ({amp_r}) must sit below an ordinary short WORD's \
         radius ({the_r}) too — the bound is about the run's OWN ink, not run position"
    );
    // The halo's footprint (ink span + 2r) must still be dominated by the SKIRT
    // for a near-zero-ink run — i.e. it stays a SMALL, ink-derived hug, never
    // inflating past what `frost_run_radius`'s own ceilings allow.
    let skirt = crate::lava::frost_px(crate::lava::FROST_FEATHER_PX, 1.0, 1.0);
    let end_cap = skirt * crate::lava::FROST_END_RADIUS_SKIRTS;
    assert!(
        amp_r <= end_cap + 0.01,
        "the '&' radius ({amp_r}) must never exceed the bounded end-pad ceiling ({end_cap})"
    );
}

/// THE BOUNDED-END-PAD LAW (item 61): a run's radius — which drives how far
/// its halo reaches PAST its own final glyph — is capped at
/// `skirt * FROST_END_RADIUS_SKIRTS`, a ceiling that is INDEPENDENT of the
/// row-height radius. A synthetic tall-row scenario (row height far past
/// anything the shipped type ladder uses) proves the ceiling actually binds:
/// without it, a long single-run label's overshoot would grow without bound
/// alongside the row height; with it, the radius — and so the overshoot —
/// stays pinned at the skirt-derived ceiling.
#[test]
fn long_run_end_pad_is_bounded_independent_of_row_height() {
    let skirt = crate::lava::frost_px(crate::lava::FROST_FEATHER_PX, 1.0, 1.0);
    let end_cap = skirt * crate::lava::FROST_END_RADIUS_SKIRTS;
    // A long run (ink width far exceeds any radius candidate) at an ordinary
    // row height: capped by `end_cap`, not by the ink bound.
    let ordinary_row_h = 40.0;
    let r_row = crate::render::frost_seed_radius(ordinary_row_h, 1.0, 1.0);
    let r_ordinary = crate::render::frost_run_radius(r_row, 400.0, skirt);
    assert!(
        (r_ordinary - end_cap).abs() < 0.01,
        "a long run's radius ({r_ordinary}) is the bounded end-pad ceiling ({end_cap}), \
         not the (larger) row-height radius ({r_row})"
    );
    // A MUCH TALLER row (a hypothetical deep heading-ladder rung / large zoom)
    // must NOT grow the long run's end-pad reach — the whole point of a
    // ceiling independent of row height.
    let tall_row_h = 400.0;
    let r_row_tall = crate::render::frost_seed_radius(tall_row_h, 1.0, 1.0);
    assert!(r_row_tall > r_row * 5.0, "control: the tall row really is much taller");
    let r_tall = crate::render::frost_run_radius(r_row_tall, 400.0, skirt);
    assert!(
        (r_tall - end_cap).abs() < 0.01,
        "a long run's end-pad radius ({r_tall}) must stay pinned at the ceiling \
         ({end_cap}) even under a much taller row ({r_row_tall}) — BOUNDED, not \
         row-height-scaled"
    );
}

/// THE NEARBY-RUN-MERGE-IS-PRESERVED LAW (item 61): ordinary text — a
/// multi-word heading's own word-runs, and two adjacent multi-word headings —
/// still bridges into ONE continuous island through the REAL seeds a live
/// outline draws, exactly like before this round (the bounded/punctuation-aware
/// radius is a no-op for runs whose own ink already meets or exceeds the
/// ceiling).
#[test]
fn nearby_ordinary_runs_still_merge_after_the_bounded_radius_round() {
    let Some((_device, _queue, mut p)) = headless_dqp(960.0, 640.0) else {
        eprintln!("skipping nearby_ordinary_runs_still_merge_after_the_bounded_radius_round: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    let was_page_on = crate::page::page_on();
    let was_measure = crate::page::measure();
    let was_outline_on = crate::outline::outline_on();
    let was_theme = crate::theme::active_index();
    crate::page::set_page_on(true);
    crate::page::set_measure(40);
    crate::outline::set_outline_on(true);
    let lava_idx = crate::theme::THEMES
        .iter()
        .position(|t| t.background.is_lava())
        .expect("a lava world ships");
    crate::theme::set_active(lava_idx);
    p.sync_theme();
    p.set_size(960.0, 640.0);

    let (seeds, _gutter) = item61_seeds(&mut p, 640);
    crate::theme::set_active(was_theme);
    crate::outline::set_outline_on(was_outline_on);
    crate::page::set_page_on(was_page_on);
    crate::page::set_measure(was_measure);

    // seeds: [0]="&", [1]="Button-free", [2]="The", [3]="quick", [4]="brown",
    // [5]="A", [6]="Getting" (elided), [7] trailing fragment.
    assert_eq!(seeds.len(), 8);
    let (button_free, the, quick, brown) = (seeds[1], seeds[2], seeds[3], seeds[4]);

    // WORD-TO-WORD within "The quick brown fox jumps": each gap bridges.
    let mid_the_quick = (the[1] + quick[0]) * 0.5;
    assert!(
        crate::lava::frost_coverage(mid_the_quick, the[2], &[the, quick]) > 0.5,
        "\"The\" and \"quick\" (same row) still bridge into one island"
    );
    let mid_quick_brown = (quick[1] + brown[0]) * 0.5;
    assert!(
        crate::lava::frost_coverage(mid_quick_brown, quick[2], &[quick, brown]) > 0.5,
        "\"quick\" and \"brown\" (same row) still bridge into one island"
    );

    // ROW-TO-ROW: "Button-free" and the "The quick brown..." row below it
    // (an ordinary, non-punctuation adjacency) still bridge — sampled at the
    // midpoint of their overlapping x-span ("The" sits fully inside
    // "Button-free"'s wider span) and their shared row-pitch midpoint y.
    let overlap_x0 = button_free[0].max(the[0]);
    let overlap_x1 = button_free[1].min(the[1]);
    let mid_x = (overlap_x0 + overlap_x1) * 0.5;
    let mid_y = (button_free[2] + the[2]) * 0.5;
    assert!(
        crate::lava::frost_coverage(mid_x, mid_y, &[button_free, the]) > 0.5,
        "\"Button-free\" and the row below it still bridge into one island"
    );
}
