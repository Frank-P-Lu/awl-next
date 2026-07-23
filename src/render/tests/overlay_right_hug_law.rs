//! ITEM 51 — RIGHT-ANCHORED PICKERS ARE ONE COMPACT CONTENT-WIDTH GROUP.
//!
//! A right-anchored (`CardAnchor::mirrors_growth` — `TopRight`) takeover card no
//! longer seats a WIDE `CARD_MAX_W` card against the right edge (leaving a 300–400px
//! dead middle between the left-aligned labels and the remote right edge). It shrinks
//! to hug its measured CONTENT — the widest visible primary, plus an optional
//! secondary column, the query line, lens strip and footer — so the whole group hugs
//! the right window edge as ONE block, text still LEFT-aligned inside it.
//!
//! The laws (all read straight off the prepared geometry + the shaped-buffer row-px
//! probes, so appearance claims are arithmetic over what the draw path lays out):
//!
//! 1. **ONE right edge** — a plain right-anchored card AND a secondary-bearing one
//!    share the SAME right edge (one full `CARD_EDGE_INSET` in from the window edge);
//!    the group grows LEFTWARD as content widens, never off the right rail.
//! 2. **content-hug, no dead middle** — a right-anchored card is far NARROWER than
//!    the wide `CARD_MAX_W` a left/center card holds, and the widest primary plate
//!    sits right up against the group's right edge (no sprawling gap).
//! 3. **the secondary survives the shrink + stays content-bounded** — shrinking to
//!    content does NOT starve the right column (it still shows), and it right-aligns
//!    to the card's own text edge (a tidy shared scanning column just past the widest
//!    primary, never at a remote edge).
//! 4. **every glyph inside its plate, wide + narrow** — at a wide AND a tight window
//!    every visible primary fits inside the card's text column (so its hug plate
//!    contains it, never clipped by the full-width clamp).
//! 5. **left/center unchanged** — a non-right card keeps the fixed wide cap exactly
//!    (`overlay_content_w` stays `0.0`), so those layouts are byte-identical.

use super::super::*;
use super::{headless_dqp, view};

/// A right-anchored FLAT picker `ViewState` — its alignment FROZEN right (the value
/// `OverlayState::align` would capture while a right-rail world is active), so the
/// geometry's frozen-anchor reader content-hugs it. `bindings` empty = a plain
/// picker; non-empty = a secondary-bearing one.
fn right_flat(items: &[&str], bindings: &[&str]) -> ViewState {
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = items.iter().map(|s| s.to_string()).collect();
    v.overlay_bindings = bindings.iter().map(|s| s.to_string()).collect();
    v.overlay_selected = 0;
    v.overlay_align = Some(theme::CardAnchor::TopRight);
    v
}

/// The Bars extent the shipped right-rail worlds (Cassowary + Mangrove) use — the
/// label plate hugs the label, the chord stays in the right column.
fn hug_label_bars() -> theme::ListStyle {
    theme::ListStyle::Bars {
        radius: 6.0,
        gap: 10.0,
        grow_px: 0.0,
        extent: theme::BarExtent::HugLabel,
        coverage: theme::BarCoverage::All,
    }
}

// ---------------------------------------------------------------------------
// 1. ONE right edge — plain + secondary-bearing share it
// ---------------------------------------------------------------------------

#[test]
fn plain_and_secondary_right_anchored_cards_share_one_right_edge() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping plain_and_secondary_share_one_right_edge: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_list_style_test_override(Some(hug_label_bars()));

    let items = ["Alpha", "Beta", "Gamma"];
    let right_edge = |p: &mut TextPipeline, v: &ViewState| -> (f32, f32) {
        p.set_view(v);
        p.prepare(&device, &queue, w, h).unwrap();
        let [x, _, cw, _] = p.overlay_card_rect().expect("a right-anchored card");
        (x + cw, cw)
    };

    let (plain_right, plain_w) = right_edge(&mut p, &right_flat(&items, &[]));
    let (sec_right, sec_w) =
        right_edge(&mut p, &right_flat(&items, &["C-x", "C-c", "M-x"]));

    let want = w as f32 - chrome::CARD_EDGE_INSET;
    assert!(
        (plain_right - want).abs() < 0.5,
        "plain right-anchored card hugs the right edge: right={plain_right}, want {want}"
    );
    assert!(
        (sec_right - want).abs() < 0.5,
        "secondary-bearing right-anchored card hugs the SAME right edge: right={sec_right}, want {want}"
    );
    // The two share ONE right edge; the secondary column widens the group LEFTWARD.
    assert!(
        (plain_right - sec_right).abs() < 0.5,
        "both right-anchored cards share one right edge: plain={plain_right} secondary={sec_right}"
    );
    assert!(
        sec_w > plain_w + 8.0,
        "the secondary column grows the group leftward (wider card): plain_w={plain_w} sec_w={sec_w}"
    );

    set_list_style_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);
}

// ---------------------------------------------------------------------------
// 2. content-hug — no dead middle, far narrower than the wide cap
// ---------------------------------------------------------------------------

#[test]
fn right_anchored_card_hugs_content_far_narrower_than_left_sprawl() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping right_anchored_card_hugs_content: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_list_style_test_override(Some(hug_label_bars()));

    let items = ["Alpha", "Beta", "Gamma", "Delta"];

    // RIGHT: content-hug — the card shrinks well below the wide flat cap.
    p.set_view(&right_flat(&items, &[]));
    p.prepare(&device, &queue, w, h).unwrap();
    let [rx, _, rw, _] = p.overlay_card_rect().unwrap();
    let hpad = p.overlay_text_hpad();
    let geom = p.overlay_geometry(w);
    let widest_primary = p
        .overlay_row_primary_px(&geom)
        .values()
        .copied()
        .fold(0.0_f32, f32::max);
    let text_left = rx + hpad;
    let card_right = rx + rw;

    // The card is genuinely SHRUNK — far below the fixed wide cap (no sprawl).
    assert!(
        rw < chrome::CARD_MAX_W - 120.0,
        "a right-anchored card of short rows hugs content, far under CARD_MAX_W: card_w={rw}"
    );
    // NO DEAD MIDDLE — the widest primary plate sits right against the group's right
    // edge (the gap past the widest content is a tidy pad, never the old 300–400px).
    let dead = card_right - (text_left + widest_primary);
    assert!(
        dead < 90.0,
        "no dead middle: widest primary ends {dead}px before the right edge (want a tidy pad, not 300+)"
    );

    // LEFT twin (same rows) keeps the fixed wide cap — the contrast that proves the
    // right card actually shrank rather than the fixture just being narrow.
    let mut left = right_flat(&items, &[]);
    left.overlay_align = Some(theme::CardAnchor::TopLeft);
    p.set_view(&left);
    p.prepare(&device, &queue, w, h).unwrap();
    let [_, _, lw, _] = p.overlay_card_rect().unwrap();
    assert!(
        (lw - p.overlay_card_desired_w(chrome::CARD_MAX_W)).abs() < 0.5,
        "a LEFT-anchored card keeps the fixed wide cap: card_w={lw}"
    );
    assert!(
        lw > rw + 120.0,
        "the right-anchored card is far narrower than its left-anchored twin: left={lw} right={rw}"
    );

    set_list_style_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);
}

// ---------------------------------------------------------------------------
// 3. the secondary survives the shrink + stays content-bounded
// ---------------------------------------------------------------------------

#[test]
fn right_anchored_secondary_survives_shrink_and_stays_content_bounded() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping right_anchored_secondary_survives_shrink: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_list_style_test_override(Some(hug_label_bars()));

    let items = ["Alpha", "Beta", "Gamma", "Delta", "Epsilon"];
    let binds = ["C-x", "C-c", "M-x", "C-s", "C-o"];
    p.set_view(&right_flat(&items, &binds));
    p.prepare(&device, &queue, w, h).unwrap();

    // Shrinking to content did NOT starve the right column — it still shows.
    assert!(
        p.overlay_right_shown,
        "the secondary column survives the content-hug (not dropped by the no-overlap arbiter)"
    );

    let [rx, _, rw, _] = p.overlay_card_rect().unwrap();
    let hpad = p.overlay_text_hpad();
    let geom = p.overlay_geometry(w);
    let secs = p.overlay_row_secondary_px(&geom);
    assert!(!secs.is_empty(), "every row carries a chord, so the secondary map is populated");

    // The right column right-aligns to the card's OWN text edge (a tidy shared
    // scanning column just past the widest primary), never a remote card/window edge.
    let text_right = rx + rw - hpad;
    let widest_secondary = secs.values().copied().fold(0.0_f32, f32::max);
    let scan_col_left = text_right - widest_secondary;
    let widest_primary = p
        .overlay_row_primary_px(&geom)
        .values()
        .copied()
        .fold(0.0_f32, f32::max);
    let primary_right = (rx + hpad) + widest_primary;
    // The scanning column begins AFTER the widest primary plus a bounded gap — the
    // content-hug leaves no sprawling middle between them.
    assert!(
        scan_col_left >= primary_right - 0.5,
        "the secondary scanning column sits past the widest primary: primary_right={primary_right} scan_col_left={scan_col_left}"
    );
    assert!(
        scan_col_left - primary_right < 80.0,
        "the label-to-shortcut gap is content-bounded, not a remote-edge sprawl: gap={}",
        scan_col_left - primary_right
    );

    set_list_style_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);
}

// ---------------------------------------------------------------------------
// 4. every glyph inside its plate — wide + narrow
// ---------------------------------------------------------------------------

#[test]
fn right_anchored_primaries_stay_inside_their_plates_wide_and_narrow() {
    let _g = crate::testlock::serial();
    set_list_style_test_override(Some(hug_label_bars()));

    let items = [
        "Alpha",
        "a-rather-longer-primary-label-here",
        "Beta",
        "Gamma",
    ];
    for &(w, h) in &[(1400u32, 800u32), (420u32, 800u32)] {
        let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
            eprintln!("skipping right_anchored_primaries_stay_inside_their_plates: no wgpu adapter");
            set_list_style_test_override(None);
            return;
        };
        p.set_view(&right_flat(&items, &["C-x", "C-c", "M-x", "C-s"]));
        p.prepare(&device, &queue, w, h).unwrap();
        let [rx, _, rw, _] = p.overlay_card_rect().unwrap();
        let hpad = p.overlay_text_hpad();
        let text_w = rw - 2.0 * hpad;
        let geom = p.overlay_geometry(w);
        for (row, primary_px) in p.overlay_row_primary_px(&geom) {
            // A primary fits the card's text column (so its hug plate — which extends
            // BEYOND the glyph before clamping to the full-width edge — contains it).
            assert!(
                primary_px <= text_w + 0.5,
                "ww={w} row {row}: primary {primary_px}px overflows text column {text_w}px (glyph clipped by the plate clamp)"
            );
        }
        // Card fully on-canvas at every width (the right group never runs off-screen).
        assert!(rx >= -0.5 && rx + rw <= w as f32 + 0.5, "ww={w}: card [{rx}, {}] on-canvas", rx + rw);
    }

    set_list_style_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);
}

// ---------------------------------------------------------------------------
// 5. left/center unchanged — the fixed wide cap, byte-identical
// ---------------------------------------------------------------------------

#[test]
fn non_right_anchored_cards_keep_the_fixed_wide_cap() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping non_right_anchored_cards_keep_the_fixed_wide_cap: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_list_style_test_override(Some(hug_label_bars()));

    let items = ["Alpha", "Beta", "Gamma"];
    for anchor in [theme::CardAnchor::TopLeft, theme::CardAnchor::TopCenter] {
        let mut v = right_flat(&items, &["C-x", "C-c", "M-x"]);
        v.overlay_align = Some(anchor);
        p.set_view(&v);
        p.prepare(&device, &queue, w, h).unwrap();
        // The content-hug cache stays inert for a non-right card.
        assert_eq!(
            p.overlay_content_w, 0.0,
            "{anchor:?}: a non-right card measures no content width (byte-identical path)"
        );
        let [_, _, cw, _] = p.overlay_card_rect().unwrap();
        assert!(
            (cw - p.overlay_card_desired_w(chrome::CARD_MAX_W)).abs() < 0.5,
            "{anchor:?}: keeps the fixed wide cap, card_w={cw}"
        );
    }

    set_list_style_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);
}
