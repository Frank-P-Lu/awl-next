//! THE FIRETAIL-MAXIMALIST-SHOWCASE PROBE ROUND — test home for the five
//! dials' machinery, ALL of which land INERT (every world byte-identical by
//! default; `overlay_personality.rs`'s byte-identity gates still stand, and
//! the merge train's main-base byte-compare is the hard gate):
//!
//! 1. PLACARD DIAL-UP — the `muted`/`bold` grammar rungs (ink laws live in
//!    `theme::tests::dialup_placard_inks_stay_on_the_ladder_below_full_ink`).
//! 2. DRAMATIC CARD ANCHOR — `CardAnchor::Inset` through the one owner
//!    `overlay_card_x` (+ its `AWL_OVERLAY_ANCHOR_FORCE` grammar arm).
//! 3. CHROME FACE — `chrome_attrs`'s Body-is-panel_attrs identity + the
//!    Named swap (the closed chrome surface set is documented on
//!    `theme::ChromeFace`; rows/query/document never call `chrome_attrs`).
//! 4. MOTION JUICE — live-only arming (a headless pipeline can NEVER kick —
//!    the determinism law, structural), the Reduce-Motion fold (instant
//!    settle, zero frames of ease), and the settle-after-duration contract.
//! 5. WILD MENU SLANT — the `AWL_OVERLAY_SLANT_FORCE` grammar, the
//!    row-origin offset math, and the elision-respects-reduced-width law
//!    (rows still flow through `rowlayout`; the slant only shrinks the width
//!    it budgets against).

use super::super::*;
use super::{headless_pipeline, view};

/// A `(Device, Queue, TextPipeline)` triple, or `None` on a GPU-less machine —
/// the small, accepted per-file duplication every GPU test file carries
/// (mirrors `overlay_personality.rs`'s own `headless_dqp`).
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
                label: Some("awl firetail-showcase-test device"),
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

// --- dial 1: the placard dial-up grammar (pure) -----------------------------

#[test]
fn parse_overlay_style_force_accepts_the_dialup_inks() {
    let cases = [
        ("placard:BL:4.0:muted", theme::PlacardInk::Muted),
        ("placard:BL:4.5:bold", theme::PlacardInk::Bold),
        ("placard:bl:3.0:Muted", theme::PlacardInk::Muted),
        ("Placard:BL:5.0:BOLD", theme::PlacardInk::Bold),
    ];
    for (input, ink) in cases {
        match parse_overlay_style_force(input) {
            Some(theme::TitleStyle::Placard { ink: got, .. }) => {
                assert_eq!(got, ink, "input {input:?}");
            }
            other => panic!("input {input:?} parsed to {other:?}"),
        }
    }
    // The dial-up never grew a dithered rung by accident: unknown ink words
    // still reject (smooth-only is Firetail's contract with Mangrove).
    assert_eq!(parse_overlay_style_force("placard:BL:4.0:dither"), None);
}

// --- dial 2: the Inset statement anchor -------------------------------------

#[test]
fn parse_overlay_anchor_force_inset_grammar() {
    assert_eq!(
        parse_overlay_anchor_force("inset:0.85"),
        Some(theme::CardAnchor::Inset { x_frac: 0.85 })
    );
    assert_eq!(
        parse_overlay_anchor_force(" Inset:0.0 "),
        Some(theme::CardAnchor::Inset { x_frac: 0.0 })
    );
    assert_eq!(
        parse_overlay_anchor_force("inset:1"),
        Some(theme::CardAnchor::Inset { x_frac: 1.0 })
    );
    for bad in ["inset:", "inset:1.5", "inset:-0.2", "inset:wat"] {
        assert_eq!(parse_overlay_anchor_force(bad), None, "expected None for {bad:?}");
    }
    // The two existing anchors still parse (the extension is additive).
    assert_eq!(parse_overlay_anchor_force("tl"), Some(theme::CardAnchor::TopLeft));
    assert_eq!(parse_overlay_anchor_force("tc"), Some(theme::CardAnchor::TopCenter));
}

/// `Inset` spans the whole horizontal composition space through the ONE
/// owner: `0.0` IS `TopLeft` (bit-equal card x), `0.5` IS `TopCenter`, and
/// `1.0` pins the card's right edge one margin in from the canvas edge —
/// the dramatic statement placement that leaves the bottom-left placard
/// corner open (the shipped-placard interplay the board round names).
#[test]
fn inset_anchor_sweeps_from_topleft_through_center_to_right_pinned() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping inset_anchor_sweeps: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_title = "commands";
    v.overlay_items = vec!["Save".into(), "Undo".into()];
    p.set_view(&v);

    let card_at = |p: &mut TextPipeline, anchor: theme::CardAnchor| -> [f32; 4] {
        set_card_anchor_test_override(Some(anchor));
        let r = p.overlay_card_rect().expect("overlay open");
        set_card_anchor_test_override(None);
        r
    };

    let tl = card_at(&mut p, theme::CardAnchor::TopLeft);
    let tc = card_at(&mut p, theme::CardAnchor::TopCenter);
    let i0 = card_at(&mut p, theme::CardAnchor::Inset { x_frac: 0.0 });
    let i5 = card_at(&mut p, theme::CardAnchor::Inset { x_frac: 0.5 });
    let i1 = card_at(&mut p, theme::CardAnchor::Inset { x_frac: 1.0 });

    assert_eq!(i0[0], tl[0], "Inset 0.0 IS TopLeft");
    assert!(
        (i5[0] - tc[0]).abs() < 0.51,
        "Inset 0.5 lands at TopCenter's x ({} vs {})",
        i5[0],
        tc[0]
    );
    // 1.0: right edge = canvas width - the edge inset (CARD_EDGE_INSET, 28, the
    // composition round's page-margin rhythm — up from the old flush 12).
    let right = i1[0] + i1[2];
    assert!(
        (right - (1200.0 - 28.0)).abs() < 0.51,
        "Inset 1.0 pins the card's right edge one edge-inset in (right {right})"
    );
    // Monotone: the dial genuinely sweeps rightward.
    assert!(i0[0] < i5[0] && i5[0] < i1[0], "x_frac sweeps monotonically right");
    // In bounds at both extremes.
    assert!(i1[0] >= 0.0 && right <= 1200.0, "right-pinned card stays on-canvas");
}

// --- dial 3: the chrome face seam --------------------------------------------

/// `ChromeFace::Body` (every world today) makes `chrome_attrs` return
/// `panel_attrs` VERBATIM — the capability is structurally inert — while a
/// `Named` face swaps the family on exactly this one seam. (That the chrome
/// spans are the ONLY callers of `chrome_attrs` — rows/query/document never
/// change face — is the closed surface set documented on `theme::ChromeFace`
/// and reviewable by grep: `chrome_attrs(` appears only at the placard /
/// title-prefix / strip-label span sites.)
#[test]
fn chrome_attrs_is_panel_attrs_under_body_and_swaps_only_under_named() {
    let _g = crate::testlock::serial();
    set_chrome_face_test_override(None);
    assert_eq!(
        format!("{:?}", chrome_attrs()),
        format!("{:?}", panel_attrs()),
        "Body (the default on every world) must shape chrome in the body face verbatim"
    );
    set_chrome_face_test_override(Some(theme::ChromeFace::Named("JetBrains Mono")));
    let named = format!("{:?}", chrome_attrs());
    set_chrome_face_test_override(None);
    assert!(
        named.contains("JetBrains Mono"),
        "Named must carry the named family (got {named})"
    );
    assert_ne!(
        named,
        format!("{:?}", panel_attrs()),
        "Named must actually differ from the body face"
    );
}

/// NEVER-TOFU FOR CHROME (the font-DB half): both bundled CHROME-VOICE faces
/// (CHROME-VOICES round — `render::FONT_CHROME_FACES`) register under their
/// EXACT expected family names, AND every world whose `render_caps.chrome_face`
/// is `Named(fam)` names a family the built font DB actually carries — so a
/// flipped world can never tofu its placard/title/strip chrome. A subset/rename
/// slip, or a typo in a world's `Named("…")`, fails HERE, not as a downstream
/// invisible-glyph box (the Wagtail-picker lesson: assert the OUTCOME). Abril
/// Fatface is bundled but assigned to no world's DATA yet (gallery-only, pending
/// the user's veto pass), so it is asserted registered explicitly, not via the
/// world sweep. NO-WILDCARD over `ChromeFace` so a future variant fails to
/// compile until it is brought under this law.
#[test]
fn every_chrome_voice_registers_and_no_world_names_an_unregistered_one() {
    let Some(p) = headless_pipeline() else {
        eprintln!("skipping chrome-voice registration law: no wgpu adapter");
        return;
    };
    let registered = |fam: &str| {
        p.font_system
            .db()
            .faces()
            .any(|f| f.families.iter().any(|(n, _)| n == fam))
    };
    // The two bundled voices register under their authentic family names.
    for fam in ["Archivo Black", "Abril Fatface"] {
        assert!(registered(fam), "chrome voice {fam:?} must be registered in the font DB");
    }
    // Every world's DATA-assigned chrome face resolves. NO-WILDCARD match: a new
    // ChromeFace variant must be handled here before it compiles.
    for t in theme::THEMES.iter() {
        match t.render_caps.chrome_face {
            theme::ChromeFace::Body => {}
            theme::ChromeFace::Named(fam) => assert!(
                registered(fam),
                "{}: chrome_face names unregistered family {fam:?} — guaranteed tofu",
                t.name
            ),
        }
    }
}

// --- dial 4: motion juice — grammar, structural determinism, RM fold ---------

#[test]
fn parse_motion_force_grammar() {
    use theme::{BandResponse, MotionJuice, OverlayEntrance};
    assert_eq!(parse_motion_force("off"), Some(MotionJuice::CALM));
    assert_eq!(parse_motion_force("calm"), Some(MotionJuice::CALM));
    assert_eq!(
        parse_motion_force("spring"),
        Some(MotionJuice { entrance: OverlayEntrance::SpringIn, band: BandResponse::Snap })
    );
    assert_eq!(
        parse_motion_force("slide"),
        Some(MotionJuice { entrance: OverlayEntrance::Instant, band: BandResponse::Slide })
    );
    for s in ["spring:slide", "full", "on"] {
        assert_eq!(
            parse_motion_force(s),
            Some(MotionJuice { entrance: OverlayEntrance::SpringIn, band: BandResponse::Slide }),
            "{s:?}"
        );
    }
    for bad in ["", "bounce", "spring:bounce", "2"] {
        assert_eq!(parse_motion_force(bad), None, "expected None for {bad:?}");
    }
}

/// THE DETERMINISM LAW, structural half: a headless pipeline is UNARMED
/// (`arm_live_juice` is called only by the live App's GPU init), so even
/// with the loudest motion bundle forced, an overlay OPEN flip kicks
/// nothing — the entrance offset is exactly `0.0` and the geometry is the
/// settled geometry. This is what makes every capture byte-identical by
/// construction rather than by a per-frame check.
#[test]
fn unarmed_pipeline_never_kicks_the_entrance() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping unarmed_pipeline_never_kicks_the_entrance: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_motion_test_override(Some(theme::MotionJuice {
        entrance: theme::OverlayEntrance::SpringIn,
        band: theme::BandResponse::Slide,
    }));
    let closed = view("hello\n", 0, 0);
    p.set_view(&closed); // overlay closed
    let mut open = view("hello\n", 0, 0);
    open.overlay_active = true;
    open.overlay_title = "commands";
    open.overlay_items = vec!["Save".into(), "Undo".into()];
    p.set_view(&open); // the OPEN flip — the only kick site
    assert_eq!(
        p.overlay_entrance_offset(),
        0.0,
        "an unarmed (headless) pipeline must render the settled position"
    );
    set_motion_test_override(None);
}

/// An ARMED pipeline (the live App's state) with `SpringIn` forced kicks the
/// entrance on the open flip; REDUCE MOTION then folds it to nothing on the
/// very next step — same final position, zero frames of ease (`motion.rs`'s
/// pure-time-compression contract), mirroring `step_copy_pulse`'s gate.
#[test]
fn armed_entrance_kicks_then_reduce_motion_folds_instantly() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping armed_entrance_kicks_then_reduce_motion_folds_instantly: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    let saved_reduced = crate::motion::reduced();
    crate::motion::set_reduced(false);
    set_motion_test_override(Some(theme::MotionJuice {
        entrance: theme::OverlayEntrance::SpringIn,
        band: theme::BandResponse::Snap,
    }));
    p.arm_live_juice();

    let closed = view("hello\n", 0, 0);
    p.set_view(&closed);
    let mut open = view("hello\n", 0, 0);
    open.overlay_active = true;
    open.overlay_title = "commands";
    open.overlay_items = vec!["Save".into(), "Undo".into()];
    p.set_view(&open);
    let kicked = p.overlay_entrance_offset();
    assert!(
        kicked < 0.0,
        "an armed SpringIn pipeline must start above the resting place (offset {kicked})"
    );
    assert!(
        (kicked + OVERLAY_ENTRANCE_DROP_PX).abs() < 0.01,
        "at t=0 the offset is the full drop ({kicked} vs -{OVERLAY_ENTRANCE_DROP_PX})"
    );

    // THE REDUCE-MOTION FOLD: one step under `reduced` settles instantly.
    crate::motion::set_reduced(true);
    let still = p.advance(0.001);
    assert_eq!(
        p.overlay_entrance_offset(),
        0.0,
        "Reduce Motion folds the entrance to the settled position instantly"
    );
    assert!(!still, "nothing keeps animating under Reduce Motion");

    // And WITHOUT Reduce Motion, the spring settles by its own duration.
    crate::motion::set_reduced(false);
    p.set_view(&closed);
    p.set_view(&open); // re-kick
    assert!(p.overlay_entrance_offset() < 0.0);
    p.advance(1.0); // 1s >> 200ms
    assert_eq!(
        p.overlay_entrance_offset(),
        0.0,
        "the entrance settles to exactly 0.0 after its duration"
    );
    assert!(!p.advance(0.016), "a settled entrance no longer holds the loop hot");

    crate::motion::set_reduced(saved_reduced);
    set_motion_test_override(None);
}

/// The selection BAND seam: Snap (every world today) + unarmed + Reduce
/// Motion all return the target verbatim; an armed Slide pipeline eases from
/// the previous row and settles ON the target (never elsewhere).
#[test]
fn band_slide_snaps_by_default_slides_when_asked_and_folds_under_reduce_motion() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping band_slide_snaps_by_default: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    let saved_reduced = crate::motion::reduced();
    crate::motion::set_reduced(false);

    // DEFAULT (Snap, unarmed): verbatim passthrough — the shipped behavior.
    set_motion_test_override(None);
    assert_eq!(p.overlay_band_drawn(100.0), 100.0);
    assert_eq!(p.overlay_band_drawn(200.0), 200.0, "Snap repositions instantly");

    // Armed + Slide: the first frame after a move draws AT the previous row
    // (ease t=0), then settles exactly on the target after the duration.
    set_motion_test_override(Some(theme::MotionJuice {
        entrance: theme::OverlayEntrance::Instant,
        band: theme::BandResponse::Slide,
    }));
    p.arm_live_juice();
    // Settle onto row 100 first (the snap phase above left the memo at 200,
    // so this itself starts a slide — let it land).
    let _ = p.overlay_band_drawn(100.0);
    p.advance(1.0); // >> 110ms
    assert_eq!(p.overlay_band_drawn(100.0), 100.0, "the slide settles on its target");
    // A fresh move: the FIRST frame draws AT the previous row (ease t=0)...
    let first = p.overlay_band_drawn(300.0);
    assert!(
        (first - 100.0).abs() < 0.01,
        "the slide starts from the previous row (drew {first})"
    );
    // ...and settles exactly ON the new target after the duration.
    p.advance(1.0);
    let settled = p.overlay_band_drawn(300.0);
    assert_eq!(settled, 300.0, "the slide settles exactly on the target row");

    // REDUCE MOTION: the same move draws the target verbatim (fold law).
    crate::motion::set_reduced(true);
    assert_eq!(
        p.overlay_band_drawn(500.0),
        500.0,
        "Reduce Motion folds the band slide to an instant snap"
    );

    crate::motion::set_reduced(saved_reduced);
    set_motion_test_override(None);
}

// --- dial 5: the wild-menu slant probe ---------------------------------------

#[test]
fn parse_overlay_slant_force_grammar() {
    assert_eq!(
        parse_overlay_slant_force("10"),
        Some(SlantProbe { px_per_row: 10.0, italic: false })
    );
    assert_eq!(
        parse_overlay_slant_force("7.5:italic"),
        Some(SlantProbe { px_per_row: 7.5, italic: true })
    );
    assert_eq!(
        parse_overlay_slant_force(" 4 : ITALIC "),
        Some(SlantProbe { px_per_row: 4.0, italic: true })
    );
    for bad in ["", "0", "-3", "nan", "10:bold", "italic"] {
        assert_eq!(parse_overlay_slant_force(bad), None, "expected None for {bad:?}");
    }
}

#[test]
fn slant_offset_math_is_a_stair_with_row_zero_unshifted() {
    let s = SlantProbe { px_per_row: 8.0, italic: false };
    assert_eq!(slant_offset(&s, 0), 0.0, "the top row never shifts");
    assert_eq!(slant_offset(&s, 3), 24.0);
    assert_eq!(slant_max_offset(&s, 12), 88.0, "deepest of 12 rows = 11 steps");
    assert_eq!(slant_max_offset(&s, 0), 0.0);
    assert_eq!(slant_max_offset(&s, 1), 0.0, "a single row pays no width tax");
}

/// THE ELISION LAW under slant: the probe's width tax flows through the SAME
/// `rowlayout` budget every picker row already rides (the rowlayout law is
/// untouched — this only shrinks the width it sees), so a row that fits
/// comfortably WITHOUT the slant elides once the stair eats its span. Driven
/// end-to-end through the real flat shaper; the shaped `panel_buffer` rows
/// are read back and compared across the two states.
#[test]
fn slant_width_tax_makes_rowlayout_elide_what_no_longer_fits() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping slant_width_tax_makes_rowlayout_elide: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_slant_test_override(None);

    // Rows sized to just about fill the card's text column, so ANY real
    // width tax must force elision.
    let long = "a".repeat(200);
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_title = "commands";
    v.overlay_items = vec![long.clone(), long.clone(), long.clone()];
    p.set_view(&v);
    let geom = p.overlay_geometry(1200);
    // The card's text column width, re-derived the way `overlay_row_region`
    // (overlay_personality.rs) re-derives geometry this test file can't read
    // directly: card width minus the 12px pad on each side.
    let [_, _, card_w, _] = p.overlay_card_rect().expect("overlay open");
    let text_w = card_w - 24.0;
    let ink = theme::base_content().to_glyphon();
    let muted = theme::muted().to_glyphon();

    // Widest candidate row (px), shaped WITHOUT the slant.
    let widest = |p: &TextPipeline| -> f32 {
        let mut w = 0.0f32;
        for run in p.panel_buffer.layout_runs() {
            if run.line_i >= 1 && run.line_i <= 3 {
                w = w.max(run.line_w);
            }
        }
        w
    };
    p.overlay_shape_text(&geom, ink, muted, None);
    let plain_w = widest(&p);
    assert!(plain_w > 0.0);

    // A hefty stair: 40px/row over 3 rows = an 80px tax.
    set_slant_test_override(Some(SlantProbe { px_per_row: 40.0, italic: false }));
    p.overlay_shape_text(&geom, ink, muted, None);
    let slanted_w = widest(&p);
    set_slant_test_override(None);

    assert!(
        slanted_w < plain_w - 40.0,
        "the slant's width tax must shorten the elided rows \
         (plain {plain_w:.1}px vs slanted {slanted_w:.1}px, tax 80px)"
    );
    // And the shifted deepest row still fits the card's text column: the
    // shaped width plus the max stair offset stays inside text_w (the whole
    // point of taxing the budget rather than letting the clip eat the text).
    assert!(
        slanted_w + 80.0 <= text_w + 1.0,
        "deepest shifted row must still land inside the text column \
         ({slanted_w:.1} + 80 > {text_w:.1})"
    );
}

/// A slanted card still PREPARES + RENDERS end-to-end (the multi-TextArea
/// upload path), and the frame genuinely differs from the unslanted one
/// inside the card — the transform draws, it isn't a silent no-op (the
/// parked/transparent bug shape, checked at real pixels).
#[test]
fn slanted_overlay_renders_and_differs_from_the_straight_one() {
    use super::pixeldiff;
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!("skipping slanted_overlay_renders_and_differs: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_slant_test_override(None);

    let mut v = view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_title = "commands";
    v.overlay_items = vec!["Save".into(), "Undo".into(), "Redo".into()];
    v.overlay_selected = 1;
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    let straight = pixeldiff::render_frame(&mut p, &device, &queue, 1200, 800);

    set_slant_test_override(Some(SlantProbe { px_per_row: 12.0, italic: false }));
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    let slanted = pixeldiff::render_frame(&mut p, &device, &queue, 1200, 800);
    set_slant_test_override(None);

    let changed = straight
        .iter()
        .zip(slanted.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        changed > 500,
        "a 12px/row slant must visibly move the rows (only {changed} pixels changed)"
    );
}
