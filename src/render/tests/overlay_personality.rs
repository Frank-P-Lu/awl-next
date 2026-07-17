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
        // The personality-assignment round's stipple variant joins the grammar
        // (the Magpie STIPPLE PROBE the taste gallery shoots is exactly this).
        ("placard:BL:3.0:stipple", theme::PlacardCorner::BL, 3.0, theme::PlacardInk::Stipple),
        // case-insensitive corner/ink, mixed case command word
        ("Placard:bl:2.25:Ghost", theme::PlacardCorner::BL, 2.25, theme::PlacardInk::Ghost),
        ("placard:bl:3.0:Stipple", theme::PlacardCorner::BL, 3.0, theme::PlacardInk::Stipple),
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

// --- the PALETTE-COMPOSITION round's gallery A/B probes (pure) ---------

#[test]
fn parse_overlay_anchor_force_grammar() {
    for s in ["tl", "TL", "topleft", "left", " Left "] {
        assert_eq!(parse_overlay_anchor_force(s), Some(theme::CardAnchor::TopLeft), "{s:?}");
    }
    for s in ["tc", "center", "centre", "TopCenter"] {
        assert_eq!(parse_overlay_anchor_force(s), Some(theme::CardAnchor::TopCenter), "{s:?}");
    }
    // PER-ITEM LIST SURFACES round: `tr`/`right`/`mirror` now name the
    // first-class RIGHT-ANCHOR MIRROR value (was previously unrecognized).
    for s in ["tr", "topright", "right", "mirror"] {
        assert_eq!(parse_overlay_anchor_force(s), Some(theme::CardAnchor::TopRight), "{s:?}");
    }
    for bad in ["", "middle", "bottom"] {
        assert_eq!(parse_overlay_anchor_force(bad), None, "expected None for {bad:?}");
    }
}

#[test]
fn parse_overlay_elevation_force_grammar() {
    for s in ["bordered", "Border", "on"] {
        assert_eq!(parse_overlay_elevation_force(s), Some(theme::Elevation::Bordered), "{s:?}");
    }
    for s in ["flat", "OFF"] {
        assert_eq!(parse_overlay_elevation_force(s), Some(theme::Elevation::Flat), "{s:?}");
    }
    for bad in ["", "raised", "shadow"] {
        assert_eq!(parse_overlay_elevation_force(bad), None, "expected None for {bad:?}");
    }
}

#[test]
fn parse_overlay_selrow_force_grammar() {
    for s in ["new", "strong", "on"] {
        assert_eq!(parse_overlay_selrow_force(s), Some(true), "{s:?}");
    }
    for s in ["old", "weak", "OFF"] {
        assert_eq!(parse_overlay_selrow_force(s), Some(false), "{s:?}");
    }
    for bad in ["", "stronger", "2"] {
        assert_eq!(parse_overlay_selrow_force(bad), None, "expected None for {bad:?}");
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

/// COMPOSITION-C2 pure derivation: an `Auto` corner resolves COMPLEMENTARY to
/// the card anchor (never under the card); an explicit corner passes through.
#[test]
fn derived_placard_corner_is_complementary_to_the_card_anchor() {
    use theme::{CardAnchor, PlacardCorner};
    // Card top-left → poster bottom-right (the balanced diagonal).
    assert_eq!(
        crate::render::derived_placard_corner(PlacardCorner::Auto, CardAnchor::TopLeft),
        PlacardCorner::BR
    );
    // A centred card defaults the poster to bottom-right.
    assert_eq!(
        crate::render::derived_placard_corner(PlacardCorner::Auto, CardAnchor::TopCenter),
        PlacardCorner::BR
    );
    // A right-shifted statement card → the poster drops to bottom-left.
    assert_eq!(
        crate::render::derived_placard_corner(
            PlacardCorner::Auto,
            CardAnchor::Inset { x_frac: 0.9 }
        ),
        PlacardCorner::BL
    );
    // A left-of-centre inset keeps the diagonal to bottom-right.
    assert_eq!(
        crate::render::derived_placard_corner(
            PlacardCorner::Auto,
            CardAnchor::Inset { x_frac: 0.2 }
        ),
        PlacardCorner::BR
    );
    // An EXPLICIT corner (Firetail's BL) is never overridden by the derivation.
    for anchor in [CardAnchor::TopLeft, CardAnchor::TopCenter, CardAnchor::Inset { x_frac: 1.0 }] {
        assert_eq!(
            crate::render::derived_placard_corner(PlacardCorner::BL, anchor),
            PlacardCorner::BL
        );
    }
}

/// COMPOSITION-C2 NO-CLIP LAW (replaces the old "every placard is BL" pin in
/// `theme::tests`): for EVERY shipped placard world, at its OWN card anchor and
/// its OWN (possibly `Auto`-derived) corner, the wordmark box stays fully on
/// canvas — no stroke clips at any edge. The shrink-to-fit in
/// `overlay_shape_placard` (added after the TR/BR-clip finding that once
/// justified pinning BL) makes every corner safe; this asserts the OUTCOME with
/// a deliberately LONG title (the worst-case width). Data-driven off the theme
/// table so a NEW placard world is swept automatically, and it cross-checks the
/// derived corner against the ONE pure owner.
#[test]
fn every_shipped_placard_world_wordmark_stays_on_canvas() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping every_shipped_placard_world_wordmark_stays_on_canvas: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_title_style_test_override(None); // each world's OWN placard data
    set_card_anchor_test_override(None); // each world's OWN card anchor

    let placard_worlds: Vec<(&str, theme::PlacardCorner)> = theme::THEMES
        .iter()
        .filter_map(|t| match t.render_caps.title_style {
            theme::TitleStyle::Placard { corner, .. } => Some((t.name, corner)),
            theme::TitleStyle::InlinePrefix => None,
        })
        .collect();
    assert!(
        !placard_worlds.is_empty(),
        "the theme table must ship at least one placard world"
    );
    let (ww, wh) = (1200.0_f32, 800.0_f32);
    for (world, data_corner) in placard_worlds {
        theme::set_active_by_name(world).unwrap();
        p.sync_theme();
        let mut v = view("hello\n", 0, 0);
        v.overlay_active = true;
        v.overlay_title = "version history"; // a long worst-case wordmark
        v.overlay_items = (0..10).map(|i| format!("Command {i}")).collect();
        p.set_view(&v);
        let geom = p.overlay_geometry(ww as u32);
        let (x, y, w, h) = p
            .overlay_shape_placard(&geom)
            .expect("a placard world must shape a wordmark");
        // Fully within the canvas — never a clipped word at ANY assigned corner.
        assert!(
            x >= -0.5 && x + w <= ww + 0.5,
            "{world}: wordmark x-span [{x:.1}..{:.1}] must stay inside canvas width {ww}",
            x + w
        );
        assert!(
            y >= -0.5 && y + h <= wh + 0.5,
            "{world}: wordmark y-span [{y:.1}..{:.1}] must stay inside canvas height {wh}",
            y + h
        );
        // The corner it drew is the ONE pure owner's resolution of the data.
        let resolved = crate::render::derived_placard_corner(
            data_corner,
            crate::render::effective_card_anchor(),
        );
        assert_ne!(
            resolved,
            theme::PlacardCorner::Auto,
            "{world}: the derivation must resolve Auto to a concrete corner"
        );
    }
    theme::set_active(theme::DEFAULT_THEME);
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
    // Force the CENTERED card anchor so the placard's canvas-corner bleed is
    // exercised HORIZONTALLY (the PALETTE-COMPOSITION round flipped the default to
    // TopLeft, which would put the card AND a BL placard both at the 12px inset —
    // the placard anchors to the CANVAS, independent of where the card sits, and
    // that independence is exactly what a centered card demonstrates).
    set_card_anchor_test_override(Some(theme::CardAnchor::TopCenter));
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

    // BLEED IS THE CONTRACT (the personality-assignment round's pinned
    // semantics — the stale "clipped to the card" doc claim is the thing
    // this assertion retires): the wordmark anchors to the CANVAS corner and
    // MAY extend past the centered card — here the BL wordmark's box starts
    // at the canvas inset, well LEFT of the card's own left edge, and hangs
    // BELOW the card's bottom edge (the card is vertically centered; the
    // wordmark hugs the canvas foot).
    let [card_x, card_y, _card_w, card_h] =
        p.overlay_card_rect().expect("the overlay card must be open");
    assert!(
        x < card_x,
        "bleed contract: the BL wordmark (x {x:.1}) starts left of the centered card \
         (card_x {card_x:.1}) — outside the card, over the scrim"
    );
    assert!(
        y + h > card_y + card_h,
        "bleed contract: the BL wordmark's box (bottom {:.1}) hangs below the centered \
         card's bottom edge ({:.1})",
        y + h,
        card_y + card_h
    );

    set_title_style_test_override(None); // leave no override behind for later tests
    set_card_anchor_test_override(None);
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
    // Fold in the PALETTE-COMPOSITION round's header gap (the divider after the
    // query/strip header) through the SAME owner the renderer uses, so the
    // sampled band tracks the shaped selected row.
    let row_top = text_top + lh * header_rows as f32 + p.overlay_header_gap() + lh * row as f32;
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
        p.overlay_shape_text(&geom, ink, muted, None);
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

// --- THE PERSONALITY-ASSIGNMENT ROUND: the STIPPLE placard's pixel laws ---

/// THE STIPPLE INK LAW, at REAL PIXELS (the placard-ink law extended to
/// `Stipple`, as the round demands): on Mangrove — the world that actually
/// SHIPS the stipple placard, running its own `render_caps`, no override —
/// diffing an overlay frame against the same frame with the placard forced
/// `InlinePrefix` isolates exactly the wordmark's contribution. Within the
/// wordmark's own box, every changed pixel must be the world's own
/// `placard_ink(Stipple)` ink (= `base_content`, ±1 LSB of sRGB round-trip)
/// — INDIVIDUAL LADDER-INK PIXELS ONLY, never amber, never a blend (a
/// fractional-alpha regression would show as intermediate tints here) — and
/// there must be genuinely MANY of them (a parked/transparent regression —
/// the Wagtail-invisible-row bug shape — fails the count floor, not just a
/// mechanism assert). Determinism rides for free: coverage is pure shaping,
/// the Bayer cut is pure position.
#[test]
fn mangrove_stipple_placard_paints_only_ladder_ink_pixels_at_real_density() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!(
            "skipping mangrove_stipple_placard_paints_only_ladder_ink_pixels_at_real_density: no wgpu adapter"
        );
        return;
    };
    let _g = crate::testlock::serial();
    theme::set_active_by_name("Mangrove").unwrap();
    p.sync_theme();

    let mut v = view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_title = "commands";
    v.overlay_items = vec!["Save".into(), "Undo".into(), "Redo".into()];
    p.set_view(&v);

    // Frame A: the placard silenced (InlinePrefix forced) — everything else
    // identical. NOTE the query line also changes ("commands › " vs "› "),
    // which is why the assertion below is scoped to the wordmark's own box,
    // far from the centered card's query row.
    set_title_style_test_override(Some(theme::TitleStyle::InlinePrefix));
    p.prepare(&device, &queue, 1200, 800).unwrap();
    let a = pixeldiff::render_frame(&mut p, &device, &queue, 1200, 800);

    // Frame B: Mangrove's OWN caps (Placard BL 3.0 Stipple).
    set_title_style_test_override(None);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    let geom = p.overlay_geometry(1200);
    let (bx, by, bw, bh) = p
        .overlay_shape_placard(&geom)
        .expect("Mangrove's own caps ship a placard");
    let b = pixeldiff::render_frame(&mut p, &device, &queue, 1200, 800);

    let ink = theme::placard_ink(theme::PlacardInk::Stipple).rgba_bytes();
    let (w, h) = (1200i64, 800i64);
    let (x0, y0) = (bx.floor() as i64, by.floor() as i64);
    let (x1, y1) = ((bx + bw).ceil() as i64, (by + bh).ceil() as i64);
    let mut changed = 0usize;
    let mut off_ink = 0usize;
    let mut worst: Option<[u8; 4]> = None;
    for y in y0.max(0)..y1.min(h) {
        for x in x0.max(0)..x1.min(w) {
            let i = (y * w + x) as usize;
            if a[i] == b[i] {
                continue;
            }
            changed += 1;
            let px = b[i];
            let near = |got: u8, want: u8| (got as i16 - want as i16).abs() <= 1;
            if !(near(px[0], ink[0]) && near(px[1], ink[1]) && near(px[2], ink[2])) {
                off_ink += 1;
                worst = Some(px);
            }
        }
    }
    assert!(
        changed >= 200,
        "the stipple wordmark changed only {changed} pixels in its own box — \
         near-invisible (the parked/transparent bug shape)"
    );
    assert_eq!(
        off_ink, 0,
        "{off_ink}/{changed} changed pixels in the wordmark box are NOT the world's own \
         stipple ink {ink:?} (worst offender {worst:?}) — the stipple contract is \
         individual ladder-ink pixels only"
    );

    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// THE SHIPPING-PLACARD REAL-PIXEL SWEEP — the Mangrove stipple pixel law
/// (above) generalized to a NO-WILDCARD roster over EVERY world that actually
/// SHIPS a `TitleStyle::Placard` (no hardcoded name list — a future placard
/// assignment is swept automatically, and a world that STOPS shipping one
/// drops out for free), each running its OWN `render_caps` (no forced
/// override on the measured frame). This closes the Wagtail-invisible-row
/// exposure the architecture pass flagged: 3 of the 4 shipping placard worlds
/// (Galah / Magpie / Ghost glyph, Firetail / Bold glyph) were proven only by
/// geometry + derived-color MATH — a parked/transparent wordmark, or one that
/// paints the wrong ink, would pass every existing test green. Here the proof
/// is over the RENDERED BYTES.
///
/// One law spans all three shipped ink treatments (Ghost, Bold — anti-aliased
/// GLYPH text; Stipple — individual Bayer-dithered full-ink pixels) because
/// each composites the SAME way: `result = ink·α + background·(1-α)`, a
/// per-channel blend that can never leave the `[background, ink]` segment
/// (α = glyph coverage for glyph inks; α ∈ {0,1} per pixel for stipple). So,
/// diffing the world's own placard frame against the same frame with the
/// placard silenced (`InlinePrefix` forced) and scanning the wordmark's own
/// box, PER WORLD:
///  1. VISIBILITY — many pixels actually change (a parked/transparent wordmark,
///     the invisible-row bug shape, fails this count floor, not just a
///     mechanism assert).
///  2. GROUND MATH — every changed pixel lands on THIS world's own
///     `placard_ink`-to-local-background segment (±LSB slack). A wrong-hue
///     wordmark — amber above all (`placard_ink` is a grey/ink ladder rung,
///     never `primary`; pinned below) — leaves the segment and fails.
///  3. REAL LETTERFORMS — a floor of HIGH-COVERAGE pixels (α ≥ 0.6 along the
///     ink's dominant channel) proves stroke BODIES painted, not a 1%-alpha
///     smear that would still count as "changed".
/// Determinism rides for free: coverage is pure shaping, the Bayer cut is pure
/// position, and the whole thing is a byte diff of two headless frames.
#[test]
fn every_shipping_placard_world_paints_visible_wordmark_ink_pixels() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!(
            "skipping every_shipping_placard_world_paints_visible_wordmark_ink_pixels: no wgpu adapter"
        );
        return;
    };
    let _g = crate::testlock::serial();

    // Per-channel slack for the ink-blend segment. A monotonic ink-over-bg
    // composite never overshoots `[bg, ink]` in either sRGB or linear space, so
    // this is pure rounding/atlas-sampling slack — kept small so a saturated
    // amber pixel (primary) still lands well outside a near-grey ink's segment.
    const SEG_TOL: i16 = 6;
    // A pixel counts as a real letterform BODY when it reached ≥60% of the way
    // from local background to the ink along the ink's most-moved channel.
    const CORE_ALPHA: f32 = 0.6;
    // Only trust the α estimate where the ink stands clear of the local
    // background on that channel (else the ratio is noise).
    const CORE_CHANNEL_GAP: i16 = 4;

    let (cw, ch) = (1200i64, 800i64);
    let mut failures: Vec<String> = Vec::new();

    for t in theme::THEMES.iter() {
        // NO-WILDCARD roster: exactly the worlds shipping a placard, by DATA.
        let theme::TitleStyle::Placard { ink: world_ink, .. } = t.render_caps.title_style else {
            continue;
        };
        theme::set_active_by_name(t.name).unwrap();
        p.sync_theme();

        // GROUND MATH pin: the world's own placard ink is a ladder/ink rung,
        // never the caret's `primary` accent (the DESIGN §3 amber guard, held
        // here by identity so the segment check below cannot legalize amber).
        let ink = theme::placard_ink(world_ink).rgba_bytes();
        if ink == theme::primary().rgba_bytes() {
            failures.push(format!(
                "{}: placard_ink({world_ink:?}) == primary — the amber guard is violated at the source",
                t.name
            ));
        }

        let mut v = view("hello world\n", 0, 0);
        v.overlay_active = true;
        v.overlay_title = "commands";
        v.overlay_items = vec!["Save".into(), "Undo".into(), "Redo".into()];
        p.set_view(&v);

        // Frame A: the placard silenced (InlinePrefix forced) — the local
        // BACKGROUND under the wordmark, everything else identical.
        set_title_style_test_override(Some(theme::TitleStyle::InlinePrefix));
        p.prepare(&device, &queue, 1200, 800).unwrap();
        let a = pixeldiff::render_frame(&mut p, &device, &queue, 1200, 800);

        // Frame B: the world's OWN caps (its shipped placard).
        set_title_style_test_override(None);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        let geom = p.overlay_geometry(1200);
        let Some((bx, by, bw, bh)) = p.overlay_shape_placard(&geom) else {
            failures.push(format!(
                "{}: ships a Placard in render_caps but overlay_shape_placard returned None \
                 (nothing to paint — the invisible-wordmark bug shape)",
                t.name
            ));
            continue;
        };
        let b = pixeldiff::render_frame(&mut p, &device, &queue, 1200, 800);

        let (x0, y0) = (bx.floor() as i64, by.floor() as i64);
        let (x1, y1) = ((bx + bw).ceil() as i64, (by + bh).ceil() as i64);
        let mut changed = 0usize;
        let mut off_seg = 0usize;
        let mut core = 0usize;
        let mut worst: Option<[u8; 4]> = None;
        for y in y0.max(0)..y1.min(ch) {
            for x in x0.max(0)..x1.min(cw) {
                let i = (y * cw + x) as usize;
                let (pa, pb) = (a[i], b[i]);
                if pa == pb {
                    continue;
                }
                changed += 1;
                // (2) GROUND MATH: on THIS world's ink-blend segment?
                let mut on_seg = true;
                for c in 0..3 {
                    let lo = (pa[c].min(ink[c]) as i16) - SEG_TOL;
                    let hi = (pa[c].max(ink[c]) as i16) + SEG_TOL;
                    let got = pb[c] as i16;
                    if got < lo || got > hi {
                        on_seg = false;
                    }
                }
                if !on_seg {
                    off_seg += 1;
                    worst = Some(pb);
                }
                // (3) REAL LETTERFORMS: coverage α along the ink's dominant
                // channel (the one where ink stands clearest of local bg).
                let mut cmax = 0usize;
                let mut gap = 0i16;
                for c in 0..3 {
                    let g = (ink[c] as i16 - pa[c] as i16).abs();
                    if g > gap {
                        gap = g;
                        cmax = c;
                    }
                }
                if gap >= CORE_CHANNEL_GAP {
                    let denom = ink[cmax] as f32 - pa[cmax] as f32;
                    let alpha = (pb[cmax] as f32 - pa[cmax] as f32) / denom;
                    if alpha >= CORE_ALPHA {
                        core += 1;
                    }
                }
            }
        }

        if changed < 200 {
            failures.push(format!(
                "{}: the wordmark changed only {changed} pixels in its own box \
                 ({bw:.0}x{bh:.0}) — near-invisible (the parked/transparent bug shape)",
                t.name
            ));
        }
        if off_seg > 0 {
            failures.push(format!(
                "{}: {off_seg}/{changed} changed pixels leave the ink-blend segment toward \
                 ink {ink:?} (worst offender {worst:?}) — a wrong-hue/amber wordmark",
                t.name
            ));
        }
        if core < 20 {
            failures.push(format!(
                "{}: only {core} high-coverage (α≥{CORE_ALPHA}) pixels — no real letterform \
                 bodies, just a faint smear",
                t.name
            ));
        }
    }

    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
    set_title_style_test_override(None);

    assert!(
        failures.is_empty(),
        "shipping placard world(s) failed the real-pixel wordmark law:\n{}",
        failures.join("\n")
    );
}

/// The distinguishability sweep's placard case, extended to the STIPPLE
/// treatment (the round's own law list: "selected row findable ... over
/// dither"): with a stipple placard forced at the loudest shipped scale, the
/// selected picker row stays perceptibly distinguishable from an adjacent
/// row — mirrors `selected_row_stays_distinguishable_with_a_forced_placard_
/// behind_it` exactly, over the new ink. (The bordered-card case rides the
/// capability-driven tier (b) of `distinguishability.rs` automatically now
/// that Currawong/Mangrove/Firetail carry non-default caps.)
#[test]
fn selected_row_stays_distinguishable_with_a_forced_stipple_placard_behind_it() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!(
            "skipping selected_row_stays_distinguishable_with_a_forced_stipple_placard_behind_it: no wgpu adapter"
        );
        return;
    };
    let _g = crate::testlock::serial();
    set_title_style_test_override(Some(theme::TitleStyle::Placard {
        corner: theme::PlacardCorner::TL,
        scale: 3.0,
        ink: theme::PlacardInk::Stipple,
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
        "PickerSelectedRow under a forced Placard{ink: Stipple} (row 0 selected vs row 1 selected)",
    );

    set_title_style_test_override(None);
}

// --- STANDING-POLICY AUDIT (2026-07-15): the minimum-window placard overflow ---
//
// Found by the personality-assignment round's OWN standing-policy audit
// (CLAUDE.md's spot-check trigger 1: a new axis value — the four shipped
// placard worlds — landed, so the FULL surface roster got probed, sampled
// across states including a resized window). NOT covered by any existing
// law: every geometry test above (`forced_placard_shapes_a_wordmark_inside_
// the_canvas_corner`, the corner-quadrant sweep, …) fixes the canvas at the
// standard 1200x800 capture size and a short title ("commands"). Neither
// axis — a NARROW window, or a LONG title — was ever swept, so the gap
// survived every existing test green.

/// THE MINIMUM-WINDOW PLACARD OVERFLOW — a REAL, LIVE-REACHABLE defect (now
/// fixed, see THE FIX below), not a synthetic edge case: `placard_origin`'s
/// BL/TL branch (`overlay_shape.rs`) anchored the wordmark's LEFT edge at
/// `ax + inset` UNCONDITIONALLY. TR/BR's branch clamped with `.max(ax)` —
/// but that clamp only protects the anchor's LEFT bound when the wordmark is
/// WIDER than the anchor (it keeps a too-wide RIGHT-anchored mark from
/// reporting a negative origin). There was NO symmetric clamp protecting the
/// RIGHT bound for a LEFT-anchored corner — and every shipped placard is BL
/// (`theme::tests::personality_assignments_are_exactly_the_decided_table`'s
/// own corner-discipline pin). `scale` was a fixed per-world multiplier, not
/// adaptive to title length or window width, so a LONG overlay title at a
/// SMALL window overflowed the canvas outright.
///
/// At the app's own DOCUMENTED minimum window (`app.rs::resumed`'s
/// `MIN_COLS(30) * CHAR_WIDTH + 2*TEXT_LEFT` by `MIN_LINES(8) * LINE_HEIGHT +
/// 2*TEXT_TOP` = 464x288 — a size a real user CAN resize the live window
/// down to, enforced by `with_min_inner_size`, so this is NOT an
/// unreachable synthetic size), `OverlayKind::History`'s title ("version
/// history") HARD-CLIPPED past the canvas's right edge — confirmed at REAL GPU
/// pixels by the audit that added this test, on all four shipped placard
/// worlds (Galah/Magpie/Mangrove/Firetail) alike, live: the rightmost
/// wordmark-ink pixel lands on the canvas's OWN last column, with "RY"
/// (Galah/Magpie/Firetail) or "ORY" (Mangrove's stipple) missing from the
/// render entirely — not an antialiasing artifact.
///
/// THE FIX (same round, layered — an origin clamp alone could NOT fix this:
/// the audit's own numbers put the wordmark's natural width at ~1205px on a
/// 464px canvas, wider than the WHOLE anchor, so no placement can contain
/// it): (1) `overlay_shape_placard`'s FIT-TO-CANVAS shrink — when the
/// naturally-scaled wordmark shapes wider than the canvas minus both insets,
/// the font size re-metrics proportionally and re-lays out (cosmic-text
/// multiplies normalized advances by the buffer font size at layout time, so
/// one linear pass lands the width); `scale` stays a per-world loudness
/// DIAL, and the window's own width is the ceiling. (2) `placard_origin`
/// grew the symmetric two-bound clamps this comment originally named
/// (BL/TL's right bound, TL/TR's bottom bound) as the float-noise backstop.
/// A comfortable window enters neither path — byte-identical. This test is
/// the enforcing law; do not loosen its bound.
#[test]
fn placard_wordmark_stays_in_bounds_at_the_apps_own_minimum_window_size() {
    use crate::overlay::OverlayKind;

    let Some(mut p) = headless_pipeline() else {
        eprintln!(
            "skipping placard_wordmark_stays_in_bounds_at_the_apps_own_minimum_window_size: no wgpu adapter"
        );
        return;
    };
    let _g = crate::testlock::serial();

    // The app's OWN enforced floor (`app.rs::resumed`'s `MIN_COLS`/`MIN_LINES`
    // — private consts local to that fn, so mirrored here rather than
    // imported; a real user can resize the live window down to exactly this
    // and no smaller, via `with_min_inner_size`).
    const MIN_COLS: f32 = 30.0;
    const MIN_LINES: f32 = 8.0;
    let min_w = MIN_COLS * CHAR_WIDTH + 2.0 * TEXT_LEFT;
    let min_h = MIN_LINES * LINE_HEIGHT + 2.0 * TEXT_TOP;
    p.set_size(min_w, min_h);

    let mut failures = Vec::new();
    for t in theme::THEMES.iter() {
        // Only the worlds that actually SHIP a placard (no hardcoded name
        // list — a future assignment is swept automatically).
        if !matches!(t.render_caps.title_style, theme::TitleStyle::Placard { .. }) {
            continue;
        }
        theme::set_active_by_name(t.name).unwrap();
        p.sync_theme();
        // Every real overlay title this world's placard could ever be asked
        // to draw (the no-wildcard `OverlayKind` roster), not just the
        // short "commands"/"settings" fixtures the other tests use.
        for kind in OverlayKind::ALL {
            let title = kind.title();
            let mut v = view("hello\n", 0, 0);
            v.overlay_active = true;
            v.overlay_title = title;
            v.overlay_items = vec!["Row one".into(), "Row two".into()];
            p.set_view(&v);
            let geom = p.overlay_geometry(min_w as u32);
            let Some((x, _y, w, _h)) = p.overlay_shape_placard(&geom) else {
                continue;
            };
            if x < 0.0 {
                failures.push(format!(
                    "{}/{title:?}: wordmark left edge {x:.1} sits off-canvas left",
                    t.name
                ));
            }
            if x + w > min_w {
                failures.push(format!(
                    "{}/{title:?}: wordmark right edge {:.1} exceeds the {:.1}px-wide \
                     minimum-window canvas by {:.1}px",
                    t.name,
                    x + w,
                    min_w,
                    x + w - min_w
                ));
            }
        }
    }
    theme::set_active(theme::DEFAULT_THEME);
    assert!(
        failures.is_empty(),
        "placard wordmark(s) overflow the canvas at the app's own minimum window size \
         (found by the personality-assignment round's standing-policy audit):\n{}",
        failures.join("\n")
    );
}
