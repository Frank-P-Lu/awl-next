//! ITEM 67 — ONE GLOBAL RESOLVER FOR THE SUMMONED CARD'S INTERIOR-RAIL PLACEMENT.
//!
//! Supersedes the old flush ~28px `TopLeft`/`TopRight` edge-hug
//! (`overlay_align_law.rs` / `reanchor_crossing_law.rs` / `overlay_right_hug_law.rs`
//! / `list_surfaces.rs` still pin the RENDERED/pixel half of this law at their own
//! seams — this file is the PURE-POLICY home for the arithmetic itself, all through
//! the ONE owner `render::chrome::overlay::overlay_rail_inset` /
//! `overlay_card_box_policy`):
//!
//! - `TopCenter` stays the EXACT viewport midpoint (`free * 0.5`), untouched.
//! - `TopLeft`/`TopRight` are SYMMETRIC INTERIOR rails whose card CENTERS sit NEAR
//!   the viewport's one-third / two-thirds marks when the card holds a comfortable
//!   width — generous outer breathing room instead of the old corner hug.
//! - The inset is a PURE function of the window width alone (never the anchor,
//!   never the caller's `desired_w`), so the left/right MIRROR law holds BY
//!   CONSTRUCTION, independent of the card's actual content width — item 51's
//!   right-anchor content-hug keeps sharing ONE right edge regardless of how
//!   narrow the measured content makes the card.
//! - The SAME `full -> anchored_max -> floor -> free` clamp chain the old fixed
//!   constant fed keeps the narrow-window response perfectly CONTINUOUS: no
//!   breakpoint jump, no per-anchor branch, no fourth user-facing anchor.
//! - Card width, text alignment, the right-anchor content shrink, mirrored Bars
//!   growth, and every world's own anchor DATA are untouched — only the shared
//!   placement arithmetic moved.

use super::super::*;

const REF: f32 = chrome::CARD_MAX_W; // the "comfortable" reference width the thirds law targets

// ---------------------------------------------------------------------------
// 1. WIDE — card-center fractions near 1/3, exactly 1/2, near 2/3
// ---------------------------------------------------------------------------

#[test]
fn wide_window_card_centers_sit_near_thirds_and_exactly_at_center() {
    for &ww in &[1000.0_f32, 1200.0, 1488.0, 1800.0] {
        for &desired in &[chrome::CARD_MAX_W, chrome::CARD_MAX_W_FACETED] {
            let (lx, lw) = chrome::overlay_card_box_policy(theme::CardAnchor::TopLeft, ww, desired);
            let (cx, cw) = chrome::overlay_card_box_policy(theme::CardAnchor::TopCenter, ww, desired);
            let (rx, rw) = chrome::overlay_card_box_policy(theme::CardAnchor::TopRight, ww, desired);

            let lfrac = (lx + lw * 0.5) / ww;
            let cfrac = (cx + cw * 0.5) / ww;
            let rfrac = (rx + rw * 0.5) / ww;
            let ctx = format!("ww={ww} desired={desired}");

            // CENTER is the EXACT viewport midpoint, always.
            assert!((cfrac - 0.5).abs() < 1e-4, "{ctx}: center-anchor fraction {cfrac} must be exactly 0.5");

            // LEFT/RIGHT sit NEAR one-third / two-thirds — a real generous rail,
            // not the old ~fixed-pixel corner hug (which at these widths reads
            // far below 0.30). The tolerance is loose enough for the FACETED cap
            // (a wider desired width shifts the center off the reference mark a
            // little, same as the old fixed-inset design let width changes shift
            // the center) yet tight enough that the OLD 28px hug (fraction well
            // under 0.30 at every one of these widths) would trip it.
            assert!(
                (lfrac - 1.0 / 3.0).abs() < 0.06,
                "{ctx}: left-anchor center fraction {lfrac} not near 1/3"
            );
            assert!(
                (rfrac - 2.0 / 3.0).abs() < 0.06,
                "{ctx}: right-anchor center fraction {rfrac} not near 2/3"
            );

            // At the REFERENCE width itself (the flat cap) the mark is landed on
            // the nose — an exact witness the "near" tolerance above isn't hiding
            // a drifted formula.
            if (desired - REF).abs() < 0.01 {
                assert!(
                    (lfrac - 1.0 / 3.0).abs() < 1e-4,
                    "{ctx}: at the reference width, left lands exactly on ww/3: {lfrac}"
                );
                assert!(
                    (rfrac - 2.0 / 3.0).abs() < 1e-4,
                    "{ctx}: at the reference width, right lands exactly on 2*ww/3: {rfrac}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 2. EXACT left/right MIRROR — across widths AND content widths
// ---------------------------------------------------------------------------

#[test]
fn exact_left_right_mirror_across_widths_and_content_widths() {
    // "content widths" spans the whole realistic range: the flat cap, the
    // faceted cap, and small stand-ins for an item-51 content-hugged card.
    for &desired in &[120.0_f32, 180.0, 250.0, chrome::CARD_MAX_W, chrome::CARD_MAX_W_FACETED] {
        for ww in (280u32..=2000).step_by(20) {
            let ww = ww as f32;
            let (lx, lw) = chrome::overlay_card_box_policy(theme::CardAnchor::TopLeft, ww, desired);
            let (rx, rw) = chrome::overlay_card_box_policy(theme::CardAnchor::TopRight, ww, desired);
            let ctx = format!("ww={ww} desired={desired}");

            // Alignment moves the card, never resizes it.
            assert!((lw - rw).abs() < 0.01, "{ctx}: TopLeft/TopRight must compute the same width");

            // The RIGHT inset (from the window's right edge) exactly equals the
            // LEFT inset (from the window's left edge) — the mirror law, holding
            // regardless of the card's own width.
            let left_inset = lx;
            let right_inset = ww - (rx + rw);
            assert!(
                (left_inset - right_inset).abs() < 0.01,
                "{ctx}: left inset {left_inset} must exactly mirror right inset {right_inset}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 3. NARROW SWEEP — continuous, on-canvas, symmetric (no jump/clip/drift)
// ---------------------------------------------------------------------------

#[test]
fn narrow_sweep_is_continuous_on_canvas_and_symmetric_no_jump_or_drift() {
    let desired = chrome::CARD_MAX_W;
    let mut prev_l: Option<f32> = None;
    let mut prev_r: Option<f32> = None;
    // Fine 1px steps across the full wide -> narrow -> narrowest transition.
    for ww in 150u32..=1000 {
        let ww = ww as f32;
        let (lx, lw) = chrome::overlay_card_box_policy(theme::CardAnchor::TopLeft, ww, desired);
        let (rx, rw) = chrome::overlay_card_box_policy(theme::CardAnchor::TopRight, ww, desired);
        let ctx = format!("ww={ww}");

        // ON-CANVAS at every step, both rails.
        assert!(lx >= -0.01 && lx + lw <= ww + 0.01, "{ctx}: TopLeft card off-canvas: [{lx},{}]", lx + lw);
        assert!(rx >= -0.01 && rx + rw <= ww + 0.01, "{ctx}: TopRight card off-canvas: [{rx},{}]", rx + rw);

        // SYMMETRIC at every step (no asymmetric drift as the window narrows).
        let left_inset = lx;
        let right_inset = ww - (rx + rw);
        assert!(
            (left_inset - right_inset).abs() < 0.05,
            "{ctx}: left inset {left_inset} drifted from right inset {right_inset}"
        );

        // CONTINUOUS — a genuine breakpoint jump would move a rail far more than
        // one window-px per one window-px step; every piece of the resolver
        // (the rail formula, the fill-regime width clamp, the floor clamp) is
        // built from `min`/`max`/linear terms, so it is Lipschitz with a small
        // constant. A loose-but-real bound catches an accidental branch/jump
        // without false-firing on ordinary sub-pixel float noise.
        if let Some(pl) = prev_l {
            assert!((lx - pl).abs() < 1.5, "{ctx}: TopLeft x jumped {pl} -> {lx} on a 1px width step");
        }
        if let Some(pr) = prev_r {
            assert!((rx - pr).abs() < 1.5, "{ctx}: TopRight x jumped {pr} -> {rx} on a 1px width step");
        }
        prev_l = Some(lx);
        prev_r = Some(rx);
    }
}

// ---------------------------------------------------------------------------
// 4. UNCHANGED — card width, text alignment (row/hit-test compose the box
//    verbatim, proven by the OTHER geometry law tests), right-anchor content
//    shrink, mirrored Bars growth, and every shipped world's own anchor DATA.
// ---------------------------------------------------------------------------

#[test]
fn card_width_caps_and_mirrors_growth_flag_are_untouched() {
    // The width CAPS themselves are pure data, never read by the placement
    // resolver's arithmetic — item 67 only ever moves `left`.
    assert_eq!(chrome::CARD_MAX_W, 520.0, "flat card width cap unchanged");
    assert_eq!(chrome::CARD_MAX_W_FACETED, 600.0, "faceted card width cap unchanged");

    // Mirrored Bars growth is a SEPARATE concern (`CardAnchor::mirrors_growth`)
    // from placement, and item 67 never touches it.
    assert!(theme::CardAnchor::TopRight.mirrors_growth(), "TopRight still mirrors bar growth");
    assert!(!theme::CardAnchor::TopLeft.mirrors_growth(), "TopLeft still does not mirror");
    assert!(!theme::CardAnchor::TopCenter.mirrors_growth(), "TopCenter still does not mirror");
    assert!(
        !theme::CardAnchor::Inset { x_frac: 1.0 }.mirrors_growth(),
        "a raw Inset dial still never mirrors (only the first-class TopRight does)"
    );
}

#[test]
fn right_anchor_content_shrink_still_shares_one_right_edge_at_the_policy_level() {
    // Item 51's law restated at the pure-policy seam: the RIGHT edge a TopRight
    // card holds depends ONLY on the window width, never the card's own
    // (possibly content-hugged, item-51-shrunk) desired width — so a sparse
    // content-hugged card and a wide flat card share the exact same right edge.
    let ww = 1200.0_f32;
    let (_, w_narrow) = chrome::overlay_card_box_policy(theme::CardAnchor::TopRight, ww, 180.0);
    let (rx_narrow, rw_narrow) = chrome::overlay_card_box_policy(theme::CardAnchor::TopRight, ww, 180.0);
    let (rx_wide, rw_wide) = chrome::overlay_card_box_policy(theme::CardAnchor::TopRight, ww, chrome::CARD_MAX_W);
    assert!(w_narrow < rw_wide - 100.0, "the narrow desired width genuinely produced a narrower card");
    assert!(
        ((rx_narrow + rw_narrow) - (rx_wide + rw_wide)).abs() < 0.01,
        "a content-hugged narrow card and a wide flat card share ONE right edge: {} vs {}",
        rx_narrow + rw_narrow,
        rx_wide + rw_wide
    );
}

#[test]
fn shipped_world_anchor_assignments_are_unchanged() {
    // Item 67 redefines the SHARED ARITHMETIC only — every world keeps its own
    // anchor CHOICE, byte-identical to before this round.
    let anchor_of = |name: &str| {
        theme::THEMES
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("world {name} exists"))
            .render_caps
            .card_anchor
    };
    assert_eq!(anchor_of("Wagtail"), theme::CardAnchor::TopLeft);
    assert_eq!(anchor_of("Tawny"), theme::CardAnchor::TopCenter);
    assert_eq!(anchor_of("Cassowary"), theme::CardAnchor::TopRight);
    assert_eq!(anchor_of("Mangrove"), theme::CardAnchor::TopRight);

    // And exactly those two worlds carry TopRight — nothing flipped sides.
    let mut right: Vec<&str> = theme::THEMES
        .iter()
        .filter(|t| t.render_caps.card_anchor == theme::CardAnchor::TopRight)
        .map(|t| t.name)
        .collect();
    right.sort_unstable();
    assert_eq!(right, vec!["Cassowary", "Mangrove"]);
}
