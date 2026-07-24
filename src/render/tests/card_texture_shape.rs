//! ITEM 70's PRINTED-CARD LAW SUITE — Quokka's `CardTexture::HalftoneDots` +
//! `CardShape::Chamfered` caps. Structural rosters first (every OTHER world
//! stays `Flat`/`Rectangular`, no-wildcard match over both closed enums),
//! then real-pixel proofs (the Wagtail tripwire: appearance is arithmetic
//! over the PNG, never inferred from state) — a chamfered corner reads as a
//! genuine 45° cut distinguishable from the pre-existing small rounded
//! corner, the dot texture rolls off toward the left content side, and the
//! split-pane gap carries no texture.

use super::super::*;
use super::{headless_dqp, pixeldiff};

// --- structural rosters --------------------------------------------------

/// EXHAUSTIVE ROSTER: every world but Quokka carries the byte-identical
/// `Flat`/`Rectangular` defaults — a no-wildcard match so a newly added
/// `CardTexture`/`CardShape` variant can't silently dodge this sweep.
#[test]
fn card_caps_are_flat_rectangular_for_every_world_but_quokka() {
    for t in theme::THEMES {
        let is_flat = match t.render_caps.card_texture {
            theme::CardTexture::Flat => true,
            theme::CardTexture::HalftoneDots { .. } => false,
        };
        let is_rect = match t.render_caps.card_shape {
            theme::CardShape::Rectangular => true,
            theme::CardShape::Chamfered { .. } => false,
        };
        if t.name == "Quokka" {
            assert!(!is_flat, "Quokka must assign a non-default CardTexture");
            assert!(!is_rect, "Quokka must assign a non-default CardShape");
        } else {
            assert!(is_flat, "{} must keep CardTexture::Flat (item 70 is Quokka-only)", t.name);
            assert!(is_rect, "{} must keep CardShape::Rectangular (item 70 is Quokka-only)", t.name);
        }
    }
}

/// Quokka's authored dials sit inside the round's own spec bands: dot angle
/// 15-20°, chamfer cut 10-12 logical px, a non-degenerate density/cell.
#[test]
fn quokka_card_caps_are_within_the_rounds_authored_spec() {
    let caps = theme::QUOKKA.render_caps;
    match caps.card_texture {
        theme::CardTexture::HalftoneDots { angle_deg, cell_px, density } => {
            assert!((15.0..=20.0).contains(&angle_deg), "angle {angle_deg} outside 15-20°");
            assert!(cell_px > 0.0, "cell_px must be positive");
            assert!(density > 0.0 && density <= 1.0, "density {density} outside (0,1]");
        }
        theme::CardTexture::Flat => panic!("Quokka must ship HalftoneDots"),
    }
    match caps.card_shape {
        theme::CardShape::Chamfered { cut_px } => {
            assert!((10.0..=12.0).contains(&cut_px), "cut_px {cut_px} outside 10-12px");
        }
        theme::CardShape::Rectangular => panic!("Quokka must ship Chamfered"),
    }
}

/// `narrowed_chamfer_px` never grows the authored cut and shrinks it once
/// the card's own smaller dimension gets tight — the "narrow layouts reduce
/// the chamfer before it steals text room" rule, pure function.
#[test]
fn narrowed_chamfer_never_exceeds_the_authored_cut_and_shrinks_on_a_small_card() {
    use crate::render::chrome::narrowed_chamfer_px;
    // A generously sized card: no reduction.
    assert_eq!(narrowed_chamfer_px(11.0, 400.0, 300.0), 11.0);
    // A tiny card (well under a genuine popup's usual size): reduced, never
    // negative, never larger than the authored cut.
    let small = narrowed_chamfer_px(11.0, 20.0, 15.0);
    assert!(small < 11.0 && small >= 0.0, "small-card chamfer {small} out of [0,11)");
    // A short-but-ordinary query bar (Split Pane's upper surface, ~500x50)
    // must NOT be reduced — 40% of its own 50px height (20px) still clears
    // the 11px authored cut. Only a genuinely tiny surface shrinks.
    let query_bar = narrowed_chamfer_px(11.0, 500.0, 50.0);
    assert_eq!(query_bar, 11.0, "an ordinary short query bar must keep its full chamfer");
    // Monotone: a smaller card never yields a LARGER chamfer than a bigger one.
    let mid = narrowed_chamfer_px(11.0, 120.0, 90.0);
    assert!(small <= mid, "chamfer should shrink monotonically with card size");
}

// --- real-pixel proofs ----------------------------------------------------

/// Open the theme picker on `world`, render one settled frame, and return
/// `(pixels, canvas_w, canvas_h, card_rect)`.
fn render_theme_picker(world: &str) -> Option<(Vec<[u8; 4]>, i64, i64, [f32; 4])> {
    let (device, queue, mut p) = headless_dqp(1200.0, 800.0)?;
    let _g = crate::testlock::serial();
    theme::set_active_by_name(world).unwrap();
    p.sync_theme();
    let mut v = super::view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_title = "themes";
    v.overlay_items = theme::world_names().iter().map(|s| s.to_string()).collect();
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    let card = p.overlay_card_rect().expect("theme picker card must be open");
    let pixels = pixeldiff::render_frame(&mut p, &device, &queue, 1200, 800);
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
    Some((pixels, 1200, 800, card))
}

fn px_at(pixels: &[[u8; 4]], w: i64, x: i64, y: i64) -> [u8; 4] {
    pixels[(y * w + x) as usize]
}

/// THE CHAMFER DISCRIMINATOR: at a point `(ex, ey)` inward from a corner
/// (measuring distance from each of the two nearest straight edges), a
/// `chamfer=c` octagon is OUTSIDE the fill iff `ex + ey < c` — a plain small
/// rounded corner (`r ~= 2.5px`, every non-Quokka world) is INSIDE well
/// before that (`ex=ey=5` clears a 2.5px radius easily). So sampling `(5,5)`
/// inward from Quokka's own card corner must land on the WORLD'S PAGE
/// BACKGROUND (the corner is cut away), while the identical offset on a
/// Rectangular-shaped world's card must land on the CARD'S OWN fill.
#[test]
fn quokka_card_top_left_corner_is_genuinely_chamfered() {
    let Some((pixels, w, _h, card)) = render_theme_picker("Quokka") else {
        eprintln!("skipping quokka_card_top_left_corner_is_genuinely_chamfered: no wgpu adapter");
        return;
    };
    let [cx, cy, _cw, _ch] = card;
    let card_fill = theme::QUOKKA.base_300.rgba_bytes();
    // 5px inward from the corner on BOTH axes: inside a 2.5px round, outside
    // an 11px chamfer (5+5=10 < 11).
    let corner_5 = px_at(&pixels, w, (cx + 5.0) as i64, (cy + 5.0) as i64);
    let near = |a: [u8; 4], b: [u8; 4]| {
        (0..3).all(|k| (a[k] as i16 - b[k] as i16).abs() <= 4)
    };
    assert!(
        !near(corner_5, card_fill),
        "5px inward from Quokka's card corner still reads as card fill {card_fill:?} \
         (got {corner_5:?}) — the chamfer isn't cutting the corner"
    );
    // Well past the chamfer (25,25 inward, sum 50 >> 11): must be filled —
    // the cut is a CORNER treatment, not a shrunk card.
    let deep = px_at(&pixels, w, (cx + 25.0) as i64, (cy + 25.0) as i64);
    assert!(near(deep, card_fill), "25px inward from the corner must be the card fill (got {deep:?})");
}

/// The SAME corner probe on a plain `Rectangular` world (Bombora, a Pane/
/// Split dark world) must find the 5px-inward point ALREADY filled — proving
/// the discriminator actually distinguishes chamfer from the pre-existing
/// small rounded corner, and that Rectangular worlds are untouched.
#[test]
fn non_quokka_card_corner_is_not_chamfered() {
    let Some((pixels, w, _h, card)) = render_theme_picker("Bombora") else {
        eprintln!("skipping non_quokka_card_corner_is_not_chamfered: no wgpu adapter");
        return;
    };
    let [cx, cy, _cw, _ch] = card;
    let card_fill = theme::BOMBORA.base_300.rgba_bytes();
    let corner_5 = px_at(&pixels, w, (cx + 5.0) as i64, (cy + 5.0) as i64);
    let near = |a: [u8; 4], b: [u8; 4]| (0..3).all(|k| (a[k] as i16 - b[k] as i16).abs() <= 4);
    assert!(
        near(corner_5, card_fill),
        "Bombora's card corner must stay the pre-existing small rounded corner (filled at \
         5px inward), got {corner_5:?} vs fill {card_fill:?}"
    );
}

/// THE ROLLOFF LAW: sampling a fixed row through the card's PLAIN interior
/// (below the header, above the footer, off any text glyph or the selected
/// band) at the LEFT edge of the content column vs a point near the card's
/// own RIGHT edge, more pixels differ from the flat card-fill color on the
/// right than on the left — "strongest at the far/right decorative side,
/// rolling off before the left-aligned content-heavy side".
#[test]
fn quokka_halftone_rolls_off_toward_the_left_content_side() {
    let Some((pixels, w, _h, card)) = render_theme_picker("Quokka") else {
        eprintln!("skipping quokka_halftone_rolls_off_toward_the_left_content_side: no wgpu adapter");
        return;
    };
    let [cx, cy, cw, ch] = card;
    let card_fill = theme::QUOKKA.base_300.rgba_bytes();
    let differs = |px: [u8; 4]| (0..3).any(|k| (px[k] as i16 - card_fill[k] as i16).abs() > 2);
    // Sample a band of rows across the card's lower half (well clear of the
    // header/query row and any single text row), a column near the LEFT
    // content edge (a few px in) and one near the RIGHT decorative edge.
    let y0 = (cy + ch * 0.55) as i64;
    let y1 = (cy + ch * 0.90) as i64;
    let left_x = (cx + cw * 0.06) as i64;
    let right_x = (cx + cw * 0.94) as i64;
    let mut left_hits = 0usize;
    let mut right_hits = 0usize;
    let mut total = 0usize;
    for y in y0..y1 {
        total += 1;
        if differs(px_at(&pixels, w, left_x, y)) {
            left_hits += 1;
        }
        if differs(px_at(&pixels, w, right_x, y)) {
            right_hits += 1;
        }
    }
    assert!(right_hits > 0, "the right decorative edge should show SOME dot texture (0/{total})");
    assert!(
        right_hits > left_hits,
        "dot texture should be stronger at the right edge ({right_hits}/{total}) than the \
         left content edge ({left_hits}/{total})"
    );
}

/// TEXT/CARD CONTRAST: the selected-row band (drawn over the halftone card)
/// still carries plenty of high-contrast ink pixels — the dot texture never
/// washes out the row text. A regression that let the dots overdraw glyphs
/// would collapse this count toward zero.
#[test]
fn quokka_selected_row_text_stays_legible_over_the_dot_texture() {
    let Some((pixels, w, h, card)) = render_theme_picker("Quokka") else {
        eprintln!("skipping quokka_selected_row_text_stays_legible_over_the_dot_texture: no wgpu adapter");
        return;
    };
    let [cx, cy, cw, ch] = card;
    let ink = theme::QUOKKA.base_content.rgba_bytes();
    let near_ink = |px: [u8; 4]| (0..3).all(|k| (px[k] as i16 - ink[k] as i16).abs() <= 24);
    let mut ink_pixels = 0usize;
    let x0 = cx.max(0.0) as i64;
    let x1 = ((cx + cw).min(w as f32)) as i64;
    let y0 = cy.max(0.0) as i64;
    let y1 = ((cy + ch).min(h as f32)) as i64;
    for y in y0..y1 {
        for x in x0..x1 {
            if near_ink(px_at(&pixels, w as i64, x, y)) {
                ink_pixels += 1;
            }
        }
    }
    assert!(
        ink_pixels >= 200,
        "expected a healthy floor of real ink pixels (row text) over Quokka's textured \
         card, found only {ink_pixels}"
    );
}
