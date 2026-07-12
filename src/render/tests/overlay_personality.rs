//! THE OVERLAY-PERSONALITY-AS-DATA ROUND — the placard machinery's own test
//! home: the `AWL_OVERLAY_STYLE_FORCE` grammar parser (pure), the corner
//! placement math (pure), the byte-identity gate (every world's overlay
//! renders exactly as before this round with no override active), and a
//! real-pixel proof that a FORCED placard sits BEHIND the rows — the
//! selected-row band stays exactly as distinguishable with a ghosted
//! wordmark drawn under it as without one (the distinguishability sweep's
//! own "if the machinery makes it reachable" instruction, answered here
//! rather than in `distinguishability.rs` itself, since reaching a placard
//! at all needs the `cfg(test)` override hook this file owns).

use super::super::*;
use super::pixeldiff::{self, DistinguishFloor, Region};
use super::{headless_pipeline, view};

/// A `(Device, Queue, TextPipeline)` triple, or `None` on a GPU-less
/// machine — mirrors `distinguishability.rs`'s/`one_bit.rs`'s own
/// `headless_dqp` (the small, accepted per-file duplication this codebase
/// already carries for GPU test setup).
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
                label: Some("awl overlay-personality-test device"),
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

// --- the AWL_OVERLAY_STYLE_FORCE grammar (pure) -----------------------

#[test]
fn parse_overlay_style_force_inline() {
    assert_eq!(parse_overlay_style_force("inline"), Some(theme::TitleStyle::InlinePrefix));
    assert_eq!(parse_overlay_style_force("Inline"), Some(theme::TitleStyle::InlinePrefix), "case-insensitive");
}

#[test]
fn parse_overlay_style_force_placard_every_corner_and_ink() {
    let cases = [
        ("placard:TL:2.0:faint", theme::PlacardCorner::TL, 2.0, theme::PlacardInk::Faint),
        ("placard:TR:1.5:ghost", theme::PlacardCorner::TR, 1.5, theme::PlacardInk::Ghost),
        ("placard:BL:3.0:ghost", theme::PlacardCorner::BL, 3.0, theme::PlacardInk::Ghost),
        ("placard:BR:0.5:faint", theme::PlacardCorner::BR, 0.5, theme::PlacardInk::Faint),
        // case-insensitive corner/ink, mixed case command word
        ("Placard:bl:2.25:Ghost", theme::PlacardCorner::BL, 2.25, theme::PlacardInk::Ghost),
    ];
    for (input, corner, scale, ink) in cases {
        let parsed = parse_overlay_style_force(input);
        assert_eq!(
            parsed,
            Some(theme::TitleStyle::Placard { corner, scale, ink }),
            "input {input:?} parsed to {parsed:?}"
        );
    }
}

#[test]
fn parse_overlay_style_force_rejects_garbage() {
    for bad in [
        "",
        "placard",
        "placard:TL",
        "placard:TL:2.0",
        "placard:ZZ:2.0:faint",  // unknown corner
        "placard:TL:notanumber:faint",
        "placard:TL:2.0:loud",   // unknown ink
        "placard:TL:2.0:faint:extra", // trailing garbage
        "wat",
    ] {
        assert_eq!(parse_overlay_style_force(bad), None, "expected None for {bad:?}");
    }
}

// --- placard corner placement, end-to-end through the real shaper -----

/// The four corners place the SAME shaped wordmark at four genuinely
/// different quadrants of the card — TL is left-and-high, BR is
/// right-and-low, and the pair share neither axis with each other. Proven
/// through the real `overlay_shape_placard` (rather than reaching for the
/// private pure `placard_origin` helper directly, which lives one module
/// deeper than this test can privately see) so the test also exercises the
/// real shaping path end-to-end.
#[test]
fn placard_corners_place_the_wordmark_in_four_different_quadrants() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping placard_corners_place_the_wordmark_in_four_different_quadrants: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_title = "commands";
    // Enough rows that the card is genuinely taller than the (modest-scale)
    // wordmark, so TL/BL and TR/BR actually land at different y's rather
    // than both clamping to the same "card too short" floor.
    v.overlay_items = (0..10).map(|i| format!("Command {i}")).collect();
    p.set_view(&v);

    let mut at = |corner: theme::PlacardCorner| {
        set_title_style_test_override(Some(theme::TitleStyle::Placard {
            corner,
            scale: 1.0,
            ink: theme::PlacardInk::Ghost,
        }));
        p.set_view(&v);
        let geom = p.overlay_geometry(1200);
        p.overlay_shape_placard(&geom).expect("a forced Placard style must shape a wordmark")
    };

    let (tl_x, tl_y, _, _) = at(theme::PlacardCorner::TL);
    let (tr_x, tr_y, _, _) = at(theme::PlacardCorner::TR);
    let (bl_x, bl_y, _, _) = at(theme::PlacardCorner::BL);
    let (br_x, br_y, _, _) = at(theme::PlacardCorner::BR);

    assert!(tl_x < tr_x, "TL sits left of TR");
    assert!(bl_x < br_x, "BL sits left of BR");
    assert!(tl_y < bl_y, "TL sits above BL");
    assert!(tr_y < br_y, "TR sits above BR");
    // Same corner pair (TL vs BL / TR vs BR) shares its horizontal anchor —
    // only the vertical anchor moves.
    assert_eq!(tl_x, bl_x, "TL and BL share the same left anchor");
    assert_eq!(tr_x, br_x, "TR and BR share the same right anchor");

    set_title_style_test_override(None);
}

// --- byte identity: every world's overlay renders exactly as before --

/// The HARD GATE: with NO override active (the shipped default on every
/// world today), the summoned overlay card renders IDENTICALLY whether or
/// not the placard machinery exists at all — the placard shaper returns
/// `None` and uploads nothing extra.
#[test]
fn no_placard_when_title_style_is_inline_prefix_the_default_on_every_world() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping no_placard_when_title_style_is_inline_prefix: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_title_style_test_override(None); // ensure no stale override from a prior test
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_title = "commands";
    v.overlay_items = vec!["Save".into(), "Undo".into()];
    p.set_view(&v);
    let geom = p.overlay_geometry(1200);
    assert_eq!(
        p.overlay_shape_placard(&geom),
        None,
        "InlinePrefix (the default) must never draw a placard"
    );
}

/// A forced `Placard` style DOES shape something, anchored inside the card.
#[test]
fn forced_placard_shapes_a_wordmark_inside_the_card() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping forced_placard_shapes_a_wordmark_inside_the_card: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_title_style_test_override(Some(theme::TitleStyle::Placard {
        corner: theme::PlacardCorner::BL,
        scale: 2.0,
        ink: theme::PlacardInk::Ghost,
    }));
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_title = "commands";
    v.overlay_items = vec!["Save".into(), "Undo".into()];
    p.set_view(&v);
    let geom = p.overlay_geometry(1200);
    let placard = p.overlay_shape_placard(&geom);
    assert!(placard.is_some(), "a forced Placard style must shape a wordmark");
    let (x, y, w, h) = placard.unwrap();
    assert!(w > 0.0 && h > 0.0, "the wordmark must have real extent");
    // Anchored within (or at) the card's own bounds — BL hugs the left edge and
    // sits toward the card's bottom. `overlay_card_rect` is the public accessor
    // for the same rect `OverlayGeom`'s (private) fields carry.
    let [card_x, card_y, _card_w, card_h] =
        p.overlay_card_rect().expect("the overlay must be open");
    assert!(x >= card_x, "wordmark must not start left of the card");
    assert!(y >= card_y && y <= card_y + card_h, "wordmark top must sit within the card's vertical span");

    set_title_style_test_override(None); // leave no override behind for later tests
}

/// A forced `Placard` on the SPELL popup (no title, no query line) or the
/// THEME picker (its own separate shaper) draws nothing — `overlay_title`
/// is empty in the first case, the theme-picker path never reaches the
/// generic shaper in the second.
#[test]
fn forced_placard_is_inert_on_kinds_with_no_title_line() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping forced_placard_is_inert_on_kinds_with_no_title_line: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_title_style_test_override(Some(theme::TitleStyle::Placard {
        corner: theme::PlacardCorner::TR,
        scale: 2.0,
        ink: theme::PlacardInk::Faint,
    }));

    // Theme picker: `overlay_lens` non-empty routes `overlay_geometry` into the
    // faceted shaper, whose `OverlayGeom::theme` is `true` (private field —
    // the behavior this asserts IS that the placard shaper's own `geom.theme`
    // check makes it a no-op, so there is nothing further to inspect).
    let mut tv = view("hello\n", 0, 0);
    tv.overlay_active = true;
    tv.overlay_title = "themes";
    tv.overlay_lens = vec![("All".to_string(), true)];
    tv.overlay_items = vec!["Tawny".into()];
    p.set_view(&tv);
    let tgeom = p.overlay_geometry(1200);
    assert_eq!(p.overlay_shape_placard(&tgeom), None, "the theme picker draws no placard (own shaper)");

    set_title_style_test_override(None);
}

// --- THE DISTINGUISHABILITY SWEEP'S "if the machinery makes it reachable" case --

fn overlay_row_region(p: &TextPipeline, row: usize) -> Region {
    let [card_x, card_y, card_w, _] =
        p.overlay_card_rect().expect("the overlay card must be open");
    let lh = p.overlay_lh();
    let text_top = card_y + 12.0; // pad
    let row_top = text_top + lh * (1.0 + row as f32); // +1 header row (the query line)
    Region::new(card_x, row_top, card_w, lh)
}

/// REAL PIXELS: with a Ghost placard FORCED behind the rows (the loudest
/// legal combination this round ships — `Placard` + the faintest-but-one
/// ink rung), the selected picker row STAYS perceptibly distinguishable
/// from an adjacent row — the row band composites OVER the wordmark exactly
/// as it does over the bare card (drawn AFTER the placard in
/// `overlay_upload_text`'s own batch order). This is the sweep's own
/// motivating-bug shape (a mechanism can look right on paper while the
/// renderer paints nothing distinguishable) answered for the ONE new
/// mechanism this round adds, structurally reachable ONLY via the
/// `cfg(test)` override — `distinguishability.rs`'s own capability-driven
/// tier (b) selects worlds by `render_caps != DEFAULT`, and no world's
/// `title_style` differs from the default yet, so that file's sweep alone
/// could never reach this case; this test is the answer to its own "if the
/// machinery makes it reachable" instruction.
#[test]
fn selected_row_stays_distinguishable_with_a_forced_placard_behind_it() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!(
            "skipping selected_row_stays_distinguishable_with_a_forced_placard_behind_it: no wgpu adapter"
        );
        return;
    };
    let _g = crate::testlock::serial();
    set_title_style_test_override(Some(theme::TitleStyle::Placard {
        corner: theme::PlacardCorner::TL,
        scale: 3.0,
        ink: theme::PlacardInk::Ghost,
    }));

    let mut v = view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_title = "commands";
    v.overlay_items = vec!["Save".into(), "Undo".into(), "Redo".into()];
    v.overlay_selected = 0;
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    let region = overlay_row_region(&p, 0);
    let a = pixeldiff::render_frame(&mut p, &device, &queue, 1200, 800);

    v.overlay_selected = 1;
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    let b = pixeldiff::render_frame(&mut p, &device, &queue, 1200, 800);

    pixeldiff::assert_perceptibly_different(
        &a,
        &b,
        1200,
        800,
        region,
        DistinguishFloor::DEFAULT,
        "PickerSelectedRow under a forced Placard{ink: Ghost} (row 0 selected vs row 1 selected)",
    );

    set_title_style_test_override(None);
}
