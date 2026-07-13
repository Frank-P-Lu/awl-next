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
//!
//! THE PLACARD-FACETS FIX ROUND added the missing case the original round's
//! fixtures never exercised: every fixture above used a FLAT (non-faceted)
//! picker, so the guard bug in `overlay_shape_placard` (it also bailed on
//! ANY picker with `overlay_lens` set — the Cmd-P palette and Settings menu
//! included, not just the literal Theme kind) had no test surface to fail
//! against. See `forced_placard_composes_with_a_faceted_picker_lens_strip_set`,
//! `forced_placard_composes_with_the_literal_theme_picker_too`, and the
//! faceted sibling of the real-pixel distinguishability test, below.

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

/// The four corners place the SAME shaped wordmark in the four SCREEN
/// quadrants — relative to the 1200x800 canvas CENTER, not the centered
/// card's — since the wordmark now anchors to the full CANVAS corners: TL is
/// left-and-high, BR is right-and-low, and the pair share neither axis with
/// each other. Proven through the real `overlay_shape_placard` (rather than
/// reaching for the private pure `placard_origin` helper directly, which
/// lives one module deeper than this test can privately see) so the test also
/// exercises the real shaping path end-to-end.
#[test]
fn placard_corners_place_the_wordmark_in_four_screen_quadrants() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping placard_corners_place_the_wordmark_in_four_screen_quadrants: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_title = "commands";
    // A few rows so the card is a normal centered picker; the wordmark's y no
    // longer depends on the card height at all (it anchors to the canvas), but
    // a realistic card keeps the fixture honest.
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

    let tl = at(theme::PlacardCorner::TL);
    let tr = at(theme::PlacardCorner::TR);
    let bl = at(theme::PlacardCorner::BL);
    let br = at(theme::PlacardCorner::BR);

    // The canvas is 1200x800 (headless_pipeline's set_size); its center splits
    // the four screen quadrants. Each corner's ANCHOR-adjacent box edge must
    // land on the correct side of that center: TL's top-left edges above/left,
    // BR's bottom-right edges below/right, etc.
    let (cx, cy) = (1200.0_f32 / 2.0, 800.0_f32 / 2.0);
    let in_quadrant = |(x, y, w, h): (f32, f32, f32, f32), right: bool, bottom: bool| {
        let edge_x = if right { x + w } else { x };
        let edge_y = if bottom { y + h } else { y };
        let x_ok = if right { edge_x >= cx } else { edge_x <= cx };
        let y_ok = if bottom { edge_y >= cy } else { edge_y <= cy };
        x_ok && y_ok
    };
    assert!(in_quadrant(tl, false, false), "TL sits in the top-left screen quadrant");
    assert!(in_quadrant(tr, true, false), "TR sits in the top-right screen quadrant");
    assert!(in_quadrant(bl, false, true), "BL sits in the bottom-left screen quadrant");
    assert!(in_quadrant(br, true, true), "BR sits in the bottom-right screen quadrant");

    // Cross-corner ordering: TL left of TR, above BL; the pair sharing a
    // vertical edge shares its horizontal anchor and vice-versa.
    assert!(tl.0 < tr.0, "TL sits left of TR");
    assert!(bl.0 < br.0, "BL sits left of BR");
    assert!(tl.1 < bl.1, "TL sits above BL");
    assert!(tr.1 < br.1, "TR sits above BR");
    assert_eq!(tl.0, bl.0, "TL and BL share the same left anchor");
    assert_eq!(tr.0, br.0, "TR and BR share the same right anchor");

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

/// A forced `Placard` style DOES shape something, anchored inside the
/// CANVAS/screen corner (not the centered card — the screen-corner watermark
/// anchor).
#[test]
fn forced_placard_shapes_a_wordmark_inside_the_canvas_corner() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping forced_placard_shapes_a_wordmark_inside_the_canvas_corner: no wgpu adapter");
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
    // Anchored to the 1200x800 CANVAS corners now, NOT the card. BL hugs the
    // LEFT screen edge (x == the 12px inset — the same literal `overlay_row_region`
    // hardcodes for the card pad, since PLACARD_INSET is private) and sits toward
    // the canvas BOTTOM half. The whole box stays on-canvas.
    let (canvas_w, canvas_h) = (1200.0_f32, 800.0_f32);
    assert!(x >= 0.0 && x + w <= canvas_w, "wordmark sits within the canvas horizontally");
    assert!(y >= 0.0 && y + h <= canvas_h, "wordmark sits within the canvas vertically");
    assert!((x - 12.0).abs() < 0.01, "BL anchors the wordmark's left edge at the inset");
    assert!(y > canvas_h * 0.5, "BL sits in the bottom half of the canvas");

    set_title_style_test_override(None); // leave no override behind for later tests
}

/// A forced `Placard` on the SPELL popup draws nothing — it has no title
/// line at all (`header_rows == 0`, no query line to prefix), so there is
/// nothing for `overlay_shape_placard`'s own `header_rows == 0` guard to do
/// but bail. This is the ONE genuinely kind-shaped exclusion left in the
/// guard (see `THE PLACARD-FACETS FIX ROUND` below for the other guard arm
/// this round REMOVED — a faceted picker used to be excluded here too, which
/// was the actual bug).
#[test]
fn forced_placard_is_inert_on_the_spell_popup_no_title_line() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping forced_placard_is_inert_on_the_spell_popup_no_title_line: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_title_style_test_override(Some(theme::TitleStyle::Placard {
        corner: theme::PlacardCorner::TR,
        scale: 2.0,
        ink: theme::PlacardInk::Faint,
    }));

    let mut sv = view("hello\n", 0, 0);
    sv.overlay_active = true;
    sv.overlay_spell = Some((0, 0, 3));
    sv.overlay_items = vec!["hello".into(), "cello".into()];
    p.set_view(&sv);
    let sgeom = p.overlay_geometry(1200);
    assert_eq!(
        p.overlay_shape_placard(&sgeom),
        None,
        "the header-less spell popup draws no placard (no title line to prefix)"
    );

    set_title_style_test_override(None);
}

// --- THE PLACARD-FACETS FIX ROUND: composes with a lens-strip card too ---
//
// THE BUG (root-caused, see `overlay_shape.rs::overlay_shape_placard`'s own
// updated doc for the fix's reasoning): `overlay_geometry` routes ANY picker
// with a non-empty `overlay_lens` — not just the literal Theme kind, but
// every faceting picker `crate::facets::scheme` names (Theme, Goto, Browse,
// Project, Command, History, Settings) — through `theme_overlay_geometry`,
// which sets `OverlayGeom::theme = true`. The placard guard used to read
// that flag as "this IS the theme picker" and bail unconditionally, so a
// FORCED placard silently drew nothing on the Cmd-P command palette or the
// Settings menu the instant either had a non-trivial lens strip — the two
// surfaces a probe/gallery run would actually want to see it on. The fix
// dropped that `geom.theme` check entirely: the placard renderer never
// needed it (it only reads `geom.card_x/_y/_w/_h`, identical shape on both
// branches), so a faceted card composes with a placard exactly like a flat
// one, no new wiring.

/// A forced `Placard` on a FACETED picker (a non-empty lens strip, e.g. the
/// command palette's own facet scheme) now shapes a wordmark — the bug this
/// round fixes. Mirrors `forced_placard_shapes_a_wordmark_inside_the_card`'s
/// flat-picker assertions exactly, over a fixture with `overlay_lens` set (so
/// `overlay_geometry` takes the SAME `theme_overlay_geometry` branch the real
/// Cmd-P palette / Settings menu take once either shows more than one lens).
#[test]
fn forced_placard_composes_with_a_faceted_picker_lens_strip_set() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping forced_placard_composes_with_a_faceted_picker_lens_strip_set: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_title_style_test_override(Some(theme::TitleStyle::Placard {
        corner: theme::PlacardCorner::BL,
        scale: 2.0,
        ink: theme::PlacardInk::Ghost,
    }));

    // A Command-palette-shaped fixture with its lens strip populated (mirrors
    // `commands::COMMAND_FACETS`' own "All / File / Edit / View" shape closely
    // enough to exercise the same geometry branch without depending on the
    // full command-catalog machinery).
    let mut cv = view("hello\n", 0, 0);
    cv.overlay_active = true;
    cv.overlay_title = "commands";
    cv.overlay_lens = vec![
        ("All".to_string(), true),
        ("File".to_string(), false),
        ("Edit".to_string(), false),
    ];
    cv.overlay_items = vec!["Save".into(), "Undo".into(), "Redo".into()];
    p.set_view(&cv);
    let cgeom = p.overlay_geometry(1200);
    let placard = p.overlay_shape_placard(&cgeom);
    assert!(
        placard.is_some(),
        "a forced Placard style must shape a wordmark on a faceted (lens-strip) card too"
    );
    let (x, y, w, h) = placard.unwrap();
    assert!(w > 0.0 && h > 0.0, "the wordmark must have real extent");
    // Anchored to the full 1200x800 CANVAS corners now, NOT the centered
    // faceted card — BL hugs the left screen edge (x == the 12px inset) and
    // sits in the canvas bottom half. The faceted branch composes with the
    // screen-corner watermark exactly like the flat one.
    let (canvas_w, canvas_h) = (1200.0_f32, 800.0_f32);
    assert!(x >= 0.0 && x + w <= canvas_w, "wordmark sits within the canvas horizontally");
    assert!(y >= 0.0 && y + h <= canvas_h, "wordmark sits within the canvas vertically");
    assert!((x - 12.0).abs() < 0.01, "BL anchors the wordmark's left edge at the inset");
    assert!(y > canvas_h * 0.5, "BL sits in the bottom half of the canvas");

    set_title_style_test_override(None);
}

/// THE LITERAL THEME PICKER decision, made explicit: it is now treated
/// exactly like every other faceting picker (Command / Settings / Goto /
/// …) rather than a special-cased exclusion. Nothing in `theme_picker.rs`
/// depends on the card being placard-free — `overlay_shape_theme` fills the
/// same `panel_buffer` a flat picker's `shape_overlay_names` does, and both
/// are uploaded through the same `overlay_upload_text`, which always draws
/// the placard FIRST (behind). Singling the Theme kind back out post-fix
/// would just reintroduce an inconsistent special case for no reason the
/// mechanism gives; this test pins the (justified) decision to include it.
#[test]
fn forced_placard_composes_with_the_literal_theme_picker_too() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping forced_placard_composes_with_the_literal_theme_picker_too: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_title_style_test_override(Some(theme::TitleStyle::Placard {
        corner: theme::PlacardCorner::TR,
        scale: 2.0,
        ink: theme::PlacardInk::Faint,
    }));

    let mut tv = view("hello\n", 0, 0);
    tv.overlay_active = true;
    tv.overlay_title = "themes";
    tv.overlay_lens = vec![("All".to_string(), true)];
    tv.overlay_items = vec!["Tawny".into(), "Mopoke".into()];
    p.set_view(&tv);
    let tgeom = p.overlay_geometry(1200);
    let placard = p.overlay_shape_placard(&tgeom);
    assert!(
        placard.is_some(),
        "the literal Theme picker now composes with a forced placard too, like any other faceted kind"
    );
    let (_x, _y, w, h) = placard.unwrap();
    assert!(w > 0.0 && h > 0.0, "the wordmark must have real extent");

    set_title_style_test_override(None);
}

// --- THE DISTINGUISHABILITY SWEEP'S "if the machinery makes it reachable" case --

/// `header_rows` mirrors the private `OverlayGeom::header_rows` this test file
/// cannot read directly (see `forced_placard_is_inert_on_the_spell_popup_no_title_line`'s
/// own note on field privacy) — `1` for a flat/nav picker's `› query` line alone,
/// `2` for a faceted picker's query + lens-strip lines (`theme_overlay_geometry`'s
/// own documented shape).
fn overlay_row_region(p: &TextPipeline, header_rows: usize, row: usize) -> Region {
    let [card_x, card_y, card_w, _] =
        p.overlay_card_rect().expect("the overlay card must be open");
    let lh = p.overlay_lh();
    let text_top = card_y + 12.0; // pad
    let row_top = text_top + lh * (header_rows as f32 + row as f32);
    Region::new(card_x, row_top, card_w, lh)
}

/// REAL PIXELS: with a Ghost placard FORCED as a screen-corner watermark
/// (the loudest legal combination this round ships — `Placard` + the
/// faintest-but-one ink rung, at the TL canvas corner), the selected picker
/// row STAYS perceptibly distinguishable from an adjacent row — the row band
/// remains legible INDEPENDENT of the wordmark, and where the corner
/// watermark's box happens to reach the card's top rows the band still
/// composites cleanly OVER it (the placard is drawn FIRST — under the rows —
/// in `overlay_upload_text`'s batch order, and is IDENTICAL between the two
/// frames, so the selection-change diff isolates the band regardless). This
/// is the sweep's own motivating-bug shape (a mechanism can look right on
/// paper while the renderer paints nothing distinguishable) answered for the
/// ONE new mechanism this round adds, structurally reachable ONLY via the
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
    let region = overlay_row_region(&p, 1, 0);
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

/// The SAME real-pixel distinguishability proof, over the FACETED geometry
/// branch (`overlay_lens` set, `header_rows == 2` for the query + lens-strip
/// lines) — the branch the placard-facets fix re-enabled the placard on, now
/// with the screen-corner watermark anchor. Reachable cheaply from this
/// file's own `cfg(test)` override, exactly like its flat-picker sibling
/// above; without it, the fix would ship with only a geometry-shape assertion
/// (`w > 0.0 && h > 0.0`) and no proof the selected-row band stays legible
/// independent of the corner watermark on the faceted branch.
#[test]
fn selected_row_stays_distinguishable_with_a_forced_placard_behind_it_on_a_faceted_picker() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!(
            "skipping selected_row_stays_distinguishable_with_a_forced_placard_behind_it_on_a_faceted_picker: no wgpu adapter"
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
    v.overlay_lens = vec![
        ("All".to_string(), true),
        ("File".to_string(), false),
        ("Edit".to_string(), false),
    ];
    v.overlay_items = vec!["Save".into(), "Undo".into(), "Redo".into()];
    v.overlay_selected = 0;
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    let region = overlay_row_region(&p, 2, 0);
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
        "PickerSelectedRow (faceted) under a forced Placard{ink: Ghost} (row 0 selected vs row 1 selected)",
    );

    set_title_style_test_override(None);
}

// --- THE INLINE-PREFIX SUPPRESSION: a Placard drops the "<title> › " prefix --
//
// THE BUG (user-reported): with a Placard active, BOTH the corner wordmark AND
// the inline "<title> › " query-line prefix fired — two titles for one picker.
// The fix suppresses the inline prefix under a `Placard` (the corner wordmark
// already names the picker), falling back to the bare `› ` sigil; `InlinePrefix`
// (the default on every world) is UNCHANGED. `overlay_title_prefix` owns that
// ONE rule for both inline sites (flat `shape_overlay_names` + faceted
// `overlay_shape_theme`), so they cannot diverge.

/// Under a forced `Placard`, the query line shows the BARE `› ` sigil — NOT a
/// "<title> › " prefix; under the default `InlinePrefix` the "<title> › "
/// prefix still leads it. Driven END-TO-END through the real shapers (flat via
/// `overlay_shape_text`'s `shape_overlay_names`, faceted via its
/// `overlay_shape_theme` branch, selected by `overlay_lens`); line 0 of the
/// shaped `panel_buffer` IS the query row, so its `LayoutRun::text` (the whole
/// logical line) is exactly the sigil/prefix here (the fixture leaves
/// `overlay_query` empty).
#[test]
fn forced_placard_suppresses_the_inline_title_prefix_on_both_shapers() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping forced_placard_suppresses_the_inline_title_prefix_on_both_shapers: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    let ink = theme::base_content().to_glyphon();
    let muted = theme::muted().to_glyphon();

    // Run the real shaper the given view routes to (flat vs faceted by
    // `overlay_lens`) and return the shaped query row's text (line 0).
    let query_line = |p: &mut TextPipeline, v: &ViewState| -> String {
        p.set_view(v);
        let geom = p.overlay_geometry(1200);
        p.overlay_shape_text(&geom, ink, muted);
        p.panel_buffer
            .layout_runs()
            .find(|r| r.line_i == 0)
            .map(|r| r.text.to_string())
            .unwrap_or_default()
    };

    // FLAT picker (no lens strip → shape_overlay_names).
    let mut flat = view("", 0, 0);
    flat.overlay_active = true;
    flat.overlay_title = "commands";
    flat.overlay_items = vec!["Save".into(), "Undo".into()];

    // FACETED picker (lens strip set → theme_overlay_geometry + overlay_shape_theme).
    let mut faceted = view("", 0, 0);
    faceted.overlay_active = true;
    faceted.overlay_title = "commands";
    faceted.overlay_lens = vec![("All".to_string(), true), ("File".to_string(), false)];
    faceted.overlay_items = vec!["Save".into(), "Undo".into()];

    // DEFAULT (InlinePrefix): the "<title> › " prefix leads the query line.
    set_title_style_test_override(Some(theme::TitleStyle::InlinePrefix));
    assert_eq!(
        query_line(&mut p, &flat),
        "commands › ",
        "InlinePrefix keeps the inline title prefix (flat shaper)"
    );
    assert_eq!(
        query_line(&mut p, &faceted),
        "commands › ",
        "InlinePrefix keeps the inline title prefix (faceted shaper)"
    );

    // Placard: the inline prefix is SUPPRESSED — the bare `› ` sigil instead.
    set_title_style_test_override(Some(theme::TitleStyle::Placard {
        corner: theme::PlacardCorner::TL,
        scale: 2.0,
        ink: theme::PlacardInk::Ghost,
    }));
    assert_eq!(
        query_line(&mut p, &flat),
        "› ",
        "a forced Placard suppresses the inline prefix (flat shaper → bare sigil)"
    );
    assert_eq!(
        query_line(&mut p, &faceted),
        "› ",
        "a forced Placard suppresses the inline prefix (faceted shaper → bare sigil)"
    );

    set_title_style_test_override(None);
}
