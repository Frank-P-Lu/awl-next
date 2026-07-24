//! ITEM 52 — THEME SELECTION DROPS YOU INTO THE COMPLETE DESTINATION WORLD.
//!
//! Item 45 froze the summoned card's alignment so a theme-preview crossing never
//! relocated it. Item 52 SUPERSEDES that for a DELIBERATE selection movement: a
//! keyboard nav / wheel crossing RE-ANCHORS the open theme picker into the
//! destination world's own left/center/right rail (choosing a world drops you
//! inside it), while EVERY other theme-owned property (palette, background,
//! Pane/Bars, chrome face, motion) already re-applies live off `theme::active()`.
//! PASSIVE pointer hover is the one exception — it re-tints the world but must NOT
//! start a spatial chase (the item-45 freeze still holds the card put).
//!
//! These laws pin the crossing at the render seam (real card x-extents, not the
//! sidecar alone), spanning left↔center↔right AND Pane↔Bars:
//!
//! 1. **re-anchor snaps to the destination** — after a deliberate crossing the
//!    card's x-extents hug the destination world's rail (left edge, centered, or
//!    right edge), and the surface treatment (`effective_list_style`) follows the
//!    same world — while the interaction state (query, selected row) survives.
//! 2. **passive hover does NOT chase** — a hover re-tints the world (the list
//!    style crosses) but the card's x-extents DO NOT move; a following keyboard
//!    move then DOES snap it (the contrast that proves hover alone is inert).

use super::super::*;
use super::{headless_pipeline, view};
use crate::overlay::OverlayState;

/// The card rect `[x, y, w, h]` a pipeline draws for the view `v` at 1200×800.
fn card_rect(p: &mut TextPipeline, v: &ViewState) -> [f32; 4] {
    p.set_size(1200.0, 800.0);
    p.sync_theme();
    p.set_view(v);
    p.overlay_card_rect().expect("an overlay card")
}

/// Build the render-side view for a summoned picker whose alignment is `align`.
fn picker_view(align: theme::CardAnchor) -> ViewState {
    let mut v = view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = vec!["Alpha".into(), "Beta".into()];
    v.overlay_align = Some(align);
    v
}

/// Cross the theme picker's SELECTION to `name` via the DELIBERATE-move owner
/// (`preview_move` = preview + re-anchor), the exact call the keyboard nav path
/// runs. Sets the selection directly so the crossing is order-independent.
fn cross_to(ov: &mut OverlayState, name: &str) {
    let ci = ov.rows.iter().position(|r| r.accept == name).expect("world in corpus");
    let pos = ov.items.iter().position(|&i| i == ci).expect("world visible on the flat lens");
    ov.selected = pos;
    crate::actions::preview_move(ov);
}

/// A PASSIVE hover onto `name`: re-highlight + the BARE `preview_overlay` (no
/// re-anchor) — exactly what `app/input/mouse.rs::overlay_hover` runs.
fn hover_to(ov: &mut OverlayState, name: &str) {
    let ci = ov.rows.iter().position(|r| r.accept == name).expect("world in corpus");
    let pos = ov.items.iter().position(|&i| i == ci).expect("world visible on the flat lens");
    ov.selected = pos;
    crate::actions::preview_overlay(ov);
}

fn anchor_of(name: &str) -> theme::CardAnchor {
    theme::THEMES
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("world {name} exists"))
        .render_caps
        .card_anchor
}

const WW: f32 = 1200.0;

/// Assert the card `[x, _, w, _]` hugs the rail its `anchor` names — item 67's
/// interior-rail inset, cw-INDEPENDENT (a pure function of `WW` alone), so both
/// arms below read the exact SAME inset regardless of the card's own width.
fn assert_on_rail(rect: [f32; 4], anchor: theme::CardAnchor, world: &str) {
    let [cx, _, cw, _] = rect;
    let inset = chrome::overlay_rail_inset(WW);
    match anchor {
        theme::CardAnchor::TopLeft => assert!(
            (cx - inset).abs() < 0.5,
            "{world} (TopLeft): card left must hug the left rail (one inset in); got x={cx}"
        ),
        theme::CardAnchor::TopRight => assert!(
            ((cx + cw) - (WW - inset)).abs() < 0.5,
            "{world} (TopRight): card right must hug the right rail; got x+w={}",
            cx + cw
        ),
        theme::CardAnchor::TopCenter => {
            let want = (WW - cw) * 0.5;
            assert!(
                (cx - want).abs() < 0.5,
                "{world} (TopCenter): card must be centered; got x={cx}, want {want}"
            );
        }
        theme::CardAnchor::Inset { .. } => unreachable!("no shipped world uses raw Inset"),
    }
}

#[test]
fn deliberate_crossing_snaps_the_card_into_the_destination_rail() {
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping deliberate_crossing_snaps_the_card_into_the_destination_rail: no wgpu adapter");
        return;
    };
    set_card_anchor_test_override(None); // the world's OWN data drives the anchor
    let restore = theme::active().name;

    // GUARD the data this crossing spans (a world-data flip flags the test).
    assert_eq!(anchor_of("Wagtail"), theme::CardAnchor::TopLeft, "Wagtail is the LEFT + Pane world");
    assert_eq!(anchor_of("Tawny"), theme::CardAnchor::TopCenter, "Tawny is the CENTER + Pane world");
    assert_eq!(anchor_of("Cassowary"), theme::CardAnchor::TopRight, "Cassowary is the RIGHT + Bars world");

    let names: Vec<String> = theme::THEMES.iter().map(|t| t.name.to_string()).collect();
    let mut ov = OverlayState::new_theme(names, theme::active_index());

    // The interaction state that must SURVIVE every crossing (item 52).
    let query_snapshot = ov.query.clone();
    let corpus_len = ov.rows.len();

    // A sequence spanning left → right → center → right → left, Pane and Bars.
    let mut rails: Vec<theme::CardAnchor> = Vec::new();
    for world in ["Wagtail", "Cassowary", "Tawny", "Mangrove", "Galah"] {
        cross_to(&mut ov, world);

        // The world crossed COMPLETELY: it is the active world, its surface
        // treatment followed, AND the card re-anchored to its rail.
        assert_eq!(theme::active().name, world, "the crossing applied {world} live");
        let want = anchor_of(world);
        assert_eq!(ov.align, want, "{world}: the card re-anchored to its own rail");

        // Surface treatment (Pane/Bars) crossed too — read live off the world.
        let bars = matches!(crate::render::effective_list_style(), theme::ListStyle::Bars { .. });
        let world_bars = matches!(
            theme::THEMES.iter().find(|t| t.name == world).unwrap().render_caps.list_style,
            theme::ListStyle::Bars { .. }
        );
        assert_eq!(bars, world_bars, "{world}: the list surface (Pane/Bars) crossed with the world");

        // Interaction state survived the crossing.
        assert_eq!(ov.query, query_snapshot, "{world}: the query survives the crossing");
        assert_eq!(ov.rows.len(), corpus_len, "{world}: the corpus survives the crossing");
        assert_eq!(ov.selected_value(), Some(world), "{world}: the selected world is the crossing target");

        // PIXEL LAW: the drawn card's x-extents hug the destination rail.
        let rect = card_rect(&mut p, &picker_view(ov.align));
        assert_on_rail(rect, want, world);
        rails.push(want);
    }

    // The sweep genuinely spanned all three rails (not one repeated placement).
    assert!(rails.contains(&theme::CardAnchor::TopLeft), "spanned a LEFT rail");
    assert!(rails.contains(&theme::CardAnchor::TopCenter), "spanned a CENTER rail");
    assert!(rails.contains(&theme::CardAnchor::TopRight), "spanned a RIGHT rail");

    theme::set_active_by_name(restore).unwrap();
    set_card_anchor_test_override(None);
}

#[test]
fn passive_hover_retints_but_does_not_relocate_the_card() {
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping passive_hover_retints_but_does_not_relocate_the_card: no wgpu adapter");
        return;
    };
    set_card_anchor_test_override(None);
    let restore = theme::active().name;

    let names: Vec<String> = theme::THEMES.iter().map(|t| t.name.to_string()).collect();
    let mut ov = OverlayState::new_theme(names, theme::active_index());

    // Land the card DELIBERATELY on Cassowary's RIGHT rail (Bars).
    cross_to(&mut ov, "Cassowary");
    let anchored = ov.align;
    assert_eq!(anchored, theme::CardAnchor::TopRight, "the deliberate move seated the RIGHT rail");
    let right_rect = card_rect(&mut p, &picker_view(ov.align));

    // Now PASSIVELY hover Wagtail — a LEFT + Pane world (a genuine crossing).
    hover_to(&mut ov, "Wagtail");
    // The world re-tinted: Wagtail is active, and its Pane surface crossed…
    assert_eq!(theme::active().name, "Wagtail", "the hover re-tinted to Wagtail");
    assert!(
        matches!(crate::render::effective_list_style(), theme::ListStyle::Pane),
        "the hover crossed the list surface to Wagtail's Pane"
    );
    // …but the card DID NOT re-anchor (no spatial chase under a hover).
    assert_eq!(ov.align, anchored, "a passive hover must NOT re-anchor the card (item 52)");
    let hover_rect = card_rect(&mut p, &picker_view(ov.align));
    assert!(
        (hover_rect[0] - right_rect[0]).abs() < 0.5 && (hover_rect[2] - right_rect[2]).abs() < 0.5,
        "the card holds its RIGHT rail across a hover: anchored=({},{}) hovered=({},{})",
        right_rect[0], right_rect[2], hover_rect[0], hover_rect[2]
    );

    // THE CONTRAST — a DELIBERATE move from here DOES snap the card to the new
    // world's rail (Wagtail's LEFT), proving the hover alone was the inert step.
    cross_to(&mut ov, "Wagtail");
    assert_eq!(ov.align, theme::CardAnchor::TopLeft, "the deliberate move re-anchored to Wagtail's LEFT rail");
    let left_rect = card_rect(&mut p, &picker_view(ov.align));
    assert_on_rail(left_rect, theme::CardAnchor::TopLeft, "Wagtail");
    assert!(
        left_rect[0] < right_rect[0] - 1.0,
        "the re-anchored LEFT card sits well left of the earlier RIGHT one: left-x={}, right-x={}",
        left_rect[0], right_rect[0]
    );

    theme::set_active_by_name(restore).unwrap();
    set_card_anchor_test_override(None);
}
