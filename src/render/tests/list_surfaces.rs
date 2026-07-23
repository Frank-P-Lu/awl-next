//! PER-ITEM LIST SURFACES round — the law suite for the INERT-by-default
//! capabilities (the "Persona list"): `ListStyle` (Pane | Bars, plus the V6 P5
//! bar axes — extent HugText, coverage SelectedOnly; the V7 taste-gate DROPPED the
//! `fill` Outline axis), the RIGHT-ANCHOR MIRROR (`CardAnchor::TopRight`, a
//! first-class anchor value), and `FacetStyle` (Text | Band | Chips). Each
//! capability's DEFAULT arm is inert: the
//! divergent rendering is reachable only through the `AWL_*_FORCE` probes / the
//! test overrides, and is proven to be a PERCEPTIBLE, findable change over real
//! pixels (the Wagtail invisible-row lesson — assert the OUTCOME, not the
//! mechanism).
//!
//! THE INERT GUARANTEE — re-scoped (2026-07-16, widened again in the
//! overlay/chrome polish round). The gate is NO LONGER "byte-identical to the
//! `main` base": ONE deliberate visual change rides every summoned picker —
//! the QUERY-INPUT BEAT, widened `0.72 -> 1.0 -> 1.3` of a row across the two
//! rounds (`OVERLAY_QUERY_BEAT`, a user-directed taste dial) — so EVERY
//! summoned picker's query line and everything below it moves down a fraction
//! vs `main` by design. Byte-identity-vs-`main` is therefore impossible for
//! any query-line surface and must not be claimed. What the inert guarantee
//! DOES assert, two ways:
//!   1. SELF-CONSISTENCY (`list_and_facet_probe_off_matches_world_default`):
//!      forcing a probe to its OFF value (`AWL_OVERLAY_LIST_FORCE=pane` /
//!      `AWL_FACET_STYLE_FORCE=text`) renders BYTE-IDENTICAL to the world's own
//!      default with NO probe set — the probe's off arm perturbs nothing IN
//!      THIS worktree. (Both sides carry the widened beat equally, so the beat
//!      is invisible to this comparison.)
//!   2. THE MODEL-LEVEL INERT LAW
//!      (`list_and_facet_default_are_inert_no_bars_no_chips_no_gap`): the
//!      default draws ZERO bar surfaces and opens ZERO row gap.
//! Together these pin "the Persona capabilities cost nothing when off" without
//! the false byte-identity-vs-`main` claim the beat retired.

use super::super::*;
use super::{headless_dqp, pixeldiff, view};

/// A `ListStyle::Bars` with the shipped-v5 DEFAULT axes (full-width, every row) —
/// the fixture every pre-v6 test uses, so those tests stay concerned only with
/// radius/gap/grow. The V6 axis variants have their own dedicated tests below.
fn bars(radius: f32, gap: f32, grow_px: f32) -> theme::ListStyle {
    theme::ListStyle::Bars {
        radius,
        gap,
        grow_px,
        extent: theme::BarExtent::FullWidth,
        coverage: theme::BarCoverage::All,
    }
}

// --- grammar (pure) ----------------------------------------------------------

#[test]
fn parse_list_style_force_grammar() {
    assert_eq!(parse_list_style_force("pane"), Some(theme::ListStyle::Pane));
    // Bare `bars` → the default treatment (a real Bars value).
    assert!(matches!(parse_list_style_force("bars"), Some(theme::ListStyle::Bars { .. })));
    // Parametric radius:gap:grow.
    assert_eq!(parse_list_style_force("bars:0:6:10"), Some(bars(0.0, 6.0, 10.0)));
    assert_eq!(parse_list_style_force("bars:14.5:8:12"), Some(bars(14.5, 8.0, 12.0)));
    // V6 P5 axis keywords fold into the SAME grammar word (any order, mixable
    // with the positional floats). A bare `bars` keeps the shipped-v5 defaults.
    assert_eq!(
        parse_list_style_force("bars:hug"),
        Some(theme::ListStyle::Bars {
            radius: 6.0,
            gap: 10.0,
            grow_px: 24.0,
            extent: theme::BarExtent::HugText,
            coverage: theme::BarCoverage::All,
        })
    );
    assert_eq!(
        parse_list_style_force("bars:0:12:0:hug:selected"),
        Some(theme::ListStyle::Bars {
            radius: 0.0,
            gap: 12.0,
            grow_px: 0.0,
            extent: theme::BarExtent::HugText,
            coverage: theme::BarCoverage::SelectedOnly,
        })
    );
    // FLIP-ROUND HYBRID — the `huglabel`/`hybrid` extent keyword (label-hug plate
    // + bare right-aligned chord). Both spellings parse; the rest of the axes keep
    // their defaults.
    for word in ["huglabel", "hybrid"] {
        assert_eq!(
            parse_list_style_force(&format!("bars:{word}")),
            Some(theme::ListStyle::Bars {
                radius: 6.0,
                gap: 10.0,
                grow_px: 24.0,
                extent: theme::BarExtent::HugLabel,
                coverage: theme::BarCoverage::All,
            }),
            "bars:{word} → the HugLabel hybrid extent"
        );
    }
    // Keyword order is free; `all`/`full` here are the defaults, so this is the
    // shipped-v5 look regardless of order. (The V7 taste-gate dropped the
    // outline-fill axis — `outline`/`filled` are no longer recognized keywords.)
    assert_eq!(
        parse_list_style_force("bars:selected:all:full"),
        Some(theme::ListStyle::Bars {
            radius: 6.0,
            gap: 10.0,
            grow_px: 24.0,
            extent: theme::BarExtent::FullWidth,
            coverage: theme::BarCoverage::All,
        })
    );
    // Malformed / negative / wrong arity / unknown keyword → None (the world's own data).
    assert_eq!(parse_list_style_force("bars:1:2:3:4"), None); // a fourth float
    assert_eq!(parse_list_style_force("bars:-1:2:3"), None);
    assert_eq!(parse_list_style_force("bars:wobble"), None); // unknown keyword
    assert_eq!(parse_list_style_force("bars:outline"), None); // retired fill axis
    assert_eq!(parse_list_style_force("capsule"), None);
    assert_eq!(parse_list_style_force(""), None);
}

#[test]
fn parse_facet_style_force_grammar() {
    assert_eq!(parse_facet_style_force("text"), Some(theme::FacetStyle::Text));
    assert_eq!(parse_facet_style_force("BAND"), Some(theme::FacetStyle::Band));
    // V6 P5 round — `chips` is WIRED for real now (the two prior attempts left it
    // unrecognized, so a `-chips` shot silently came out as `text`). It parses.
    // The bare `chips` word == the landed baseline (`Hairline`); each suffix maps
    // to its treatment (CHIP-VARIATIONS PROBE).
    let chips = |v| Some(theme::FacetStyle::Chips(v));
    assert_eq!(parse_facet_style_force("chips"), chips(theme::ChipVariant::Hairline));
    assert_eq!(parse_facet_style_force("CHIPS"), chips(theme::ChipVariant::Hairline));
    assert_eq!(parse_facet_style_force("chips:filled"), chips(theme::ChipVariant::FilledActive));
    assert_eq!(parse_facet_style_force("chips:underline"), chips(theme::ChipVariant::Underline));
    assert_eq!(parse_facet_style_force("chips:bracket"), chips(theme::ChipVariant::Bracket));
    // The DROPPED variants (user's confirmed map) no longer parse — they fall to None.
    assert_eq!(parse_facet_style_force("chips:bold"), None);
    assert_eq!(parse_facet_style_force("chips:tinted"), None);
    // An unrelated typo — or an unknown chip suffix — falls back to None.
    assert_eq!(parse_facet_style_force("pill"), None);
    assert_eq!(parse_facet_style_force("chips:sparkle"), None);
    assert_eq!(parse_facet_style_force(""), None);
}

/// The force-knob classifier must tell UNSET (silent world default) apart from
/// SET-AND-PARSED and SET-BUT-RETIRED (a typo'd word) — the reader turns the
/// last LOUD. This is the guard against the facet-chips GALLERY TRAP: a re-shoot
/// forcing an unrecognized variant silently produced a byte-identical duplicate
/// of `text`. V6 P5 wired `chips` for REAL, so it now classifies as Parsed
/// (rendering the pills), NOT Retired — the trap is closed by the value existing.
#[test]
fn forced_knob_classifies_unset_parsed_and_retired() {
    // Unset → the world's own default, no note.
    assert!(matches!(
        classify_forced_knob(None, parse_facet_style_force),
        ForcedKnob::Unset
    ));
    // A recognized value → Parsed.
    assert!(matches!(
        classify_forced_knob(Some("band"), parse_facet_style_force),
        ForcedKnob::Parsed(theme::FacetStyle::Band)
    ));
    // V6: `chips` is now a REAL value → Parsed (the pills render), never the
    // silent `text` duplicate the two prior attempts shipped.
    assert!(matches!(
        classify_forced_knob(Some("chips"), parse_facet_style_force),
        ForcedKnob::Parsed(theme::FacetStyle::Chips(theme::ChipVariant::Hairline))
    ));
    // A genuine typo, but SET → Retired (loud fallback): never a silent
    // duplicate of the default masquerading under a bogus name.
    assert!(matches!(
        classify_forced_knob(Some("pill"), parse_facet_style_force),
        ForcedKnob::Retired
    ));
    // The list-style knob shares the classifier: a retired `capsule` word is loud.
    assert!(matches!(
        classify_forced_knob(Some("capsule"), parse_list_style_force),
        ForcedKnob::Retired
    ));
    assert!(matches!(
        classify_forced_knob(None, parse_list_style_force),
        ForcedKnob::Unset
    ));
}

#[test]
fn parse_overlay_anchor_force_accepts_topright_as_the_mirror() {
    for s in ["tr", "topright", "right", "mirror", "MIRROR", "Right"] {
        assert_eq!(
            parse_overlay_anchor_force(s),
            Some(theme::CardAnchor::TopRight),
            "input {s:?}"
        );
    }
    // Only TopRight mirrors bar growth; every other anchor grows toward the
    // open right margin.
    assert!(theme::CardAnchor::TopRight.mirrors_growth());
    for a in [
        theme::CardAnchor::TopLeft,
        theme::CardAnchor::TopCenter,
        theme::CardAnchor::Inset { x_frac: 1.0 },
    ] {
        assert!(!a.mirrors_growth(), "{a:?} must not mirror");
    }
}

// --- the mirror: right-anchored placement stays on canvas (pure) -------------

#[test]
fn topright_card_box_is_right_anchored_and_on_canvas_across_the_width_sweep() {
    let floor = chrome::CARD_EDGE_INSET_FLOOR;
    for &desired in &[chrome::CARD_MAX_W, chrome::CARD_MAX_W_FACETED] {
        for ww in (320u32..=1800).step_by(40) {
            let ww = ww as f32;
            let (left, w) =
                chrome::overlay_card_box_policy(theme::CardAnchor::TopRight, ww, desired);
            let right = left + w;
            let ctx = format!("ww={ww} desired={desired}");
            assert!(w > 24.0, "{ctx}: card width {w} must leave room for text");
            assert!(left >= floor - 0.01, "{ctx}: left {left} >= floor {floor}");
            assert!(
                right <= ww - floor + 0.01,
                "{ctx}: right edge {right} keeps a floor margin inside {ww}"
            );
            // WIDE: the card's RIGHT edge sits one full edge inset in from the
            // canvas right (the mirror of TopLeft's left inset).
            if desired + 2.0 * chrome::CARD_EDGE_INSET <= ww {
                assert!(
                    (right - (ww - chrome::CARD_EDGE_INSET)).abs() < 0.01,
                    "{ctx}: wide window pins the right edge one inset in, got right={right}"
                );
            }
        }
    }
}

// --- the mirror + grow: the selected bar rect (pure) -------------------------

#[test]
fn selected_bar_grows_wider_toward_the_open_margin_and_mirrors() {
    // A card at x=100, width=500, one row at top=200, bar 20 tall, grow 6.
    let (cx, cw, top, bh, g) = (100.0, 500.0, 200.0, 20.0, 6.0);
    let unsel = chrome::bar_rect_unselected(cx, cw, top, bh);
    let def = chrome::bar_rect_selected(cx, cw, top, bh, g, false);
    let mir = chrome::bar_rect_selected(cx, cw, top, bh, g, true);

    // Both selected bars are WIDER than the unselected one by exactly `g`.
    assert!((def[2] - (unsel[2] + g)).abs() < 1e-3, "default grows width by g");
    assert!((mir[2] - (unsel[2] + g)).abs() < 1e-3, "mirror grows width by g");

    // DEFAULT: shares the unselected LEFT edge, juts further RIGHT.
    assert!((def[0] - unsel[0]).abs() < 1e-3, "default keeps the left edge");
    assert!(def[0] + def[2] > unsel[0] + unsel[2] + g - 1e-3, "default juts right");

    // MIRROR: shares the unselected RIGHT edge, juts further LEFT.
    assert!(
        ((mir[0] + mir[2]) - (unsel[0] + unsel[2])).abs() < 1e-3,
        "mirror keeps the right edge"
    );
    assert!(mir[0] < unsel[0] - 1e-3, "mirror juts left");

    // DESIGNER PIXEL-PASS FIX (2026-07-16): the selected bar juts INTO THE ROOM,
    // past the card's own edge — the pane is dropped, so there is no box to stay
    // within; the framebuffer clips the trailing edge at the canvas. A big grow
    // therefore extends the jut fully (no `card_w` clamp capping it at
    // `BAR_SIDE_INSET`). Only the LEADING edge is floored at the canvas (0.0) so a
    // mirrored jut never runs off the left side.
    let big_def = chrome::bar_rect_selected(cx, cw, top, bh, 999.0, false);
    assert!(
        big_def[0] + big_def[2] > cx + cw,
        "a large default grow juts past the card's right edge into the room: {big_def:?}"
    );
    let big_mir = chrome::bar_rect_selected(cx, cw, top, bh, 999.0, true);
    assert!(big_mir[0] >= -1e-3, "a mirrored jut is floored at the canvas left edge: {big_mir:?}");
    assert!(
        (big_mir[0] + big_mir[2] - (unsel[0] + unsel[2])).abs() < 1e-3,
        "a mirrored jut keeps the unselected RIGHT edge no matter how large: {big_mir:?}"
    );
}

// --- INERT by default: no bar / chip instances, no gap (real pipeline) -------

#[test]
fn list_and_facet_default_are_inert_no_bars_no_chips_no_gap() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!("skipping list_and_facet_default_are_inert: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    // Belt-and-braces: no test override is set, so the world's own (Pane/Text)
    // data governs — the inert default this whole round preserves.
    set_list_style_test_override(None);
    set_facet_style_test_override(None);

    for faceted in [false, true] {
        let mut v = view("hello\n", 0, 0);
        v.overlay_active = true;
        v.overlay_items = (0..8).map(|i| format!("Command {i}")).collect();
        v.overlay_selected = 3;
        if faceted {
            v.overlay_lens = vec![("All".into(), true), ("File".into(), false)];
        }
        p.set_view(&v);
        p.prepare(&device, &queue, 1200, 800).unwrap();
        assert_eq!(p.overlay_row_gap(), 0.0, "Pane opens no row gap (faceted={faceted})");
        assert_eq!(
            p.overlay_bars.instance_count(),
            0,
            "Pane draws ZERO bar surfaces (faceted={faceted})"
        );
        // The selected row still gets its single Pane band (unchanged).
        assert_eq!(p.overlay_rows.instance_count(), 1, "Pane keeps its one selected band");
    }
}

/// THE INERT SELF-CONSISTENCY LAW (real pixels) — the re-scoped replacement for
/// the retired "byte-identical to `main`" gate (see the module doc). Forcing a
/// probe to its OFF value must render BYTE-IDENTICAL to the world's own default
/// with NO probe set, IN THIS WORKTREE — so the probe's off arm is proven to
/// perturb nothing without any claim about `main` (which the widened query beat
/// legitimately diverges from). Both sides carry the same beat, so it cancels.
#[test]
fn list_and_facet_probe_off_matches_world_default() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping list_and_facet_probe_off_matches_world_default: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    // A default (Pane/Text) world — the inert arm this round preserves.
    theme::set_active_by_name("Currawong").unwrap();
    p.sync_theme();

    for faceted in [false, true] {
        let mut v = view("hello\n", 0, 0);
        v.overlay_active = true;
        v.overlay_title = "themes";
        v.overlay_items = (0..8).map(|i| format!("Command {i}")).collect();
        v.overlay_selected = 3;
        if faceted {
            v.overlay_lens = vec![("All".into(), true), ("File".into(), false)];
        }

        let frame = |p: &mut TextPipeline,
                     list: Option<theme::ListStyle>,
                     facet: Option<theme::FacetStyle>| {
            set_list_style_test_override(list);
            set_facet_style_test_override(facet);
            p.set_view(&v);
            p.prepare(&device, &queue, w, h).unwrap();
            pixeldiff::render_frame(p, &device, &queue, w, h)
        };

        // World DEFAULT (no probe) vs probe FORCED to its OFF value.
        let default_arm = frame(&mut p, None, None);
        let probe_off = frame(
            &mut p,
            Some(theme::ListStyle::Pane),
            Some(theme::FacetStyle::Text),
        );
        set_list_style_test_override(None);
        set_facet_style_test_override(None);

        pixeldiff::assert_identical(
            &default_arm,
            &probe_off,
            w as i64,
            h as i64,
            pixeldiff::Region::canvas(w as i64, h as i64),
            &format!("Pane/Text probe-off == world default (faceted={faceted})"),
        );
    }
    theme::set_active(theme::DEFAULT_THEME);
}

// --- Bars FLOAT BOUNDED PLATES; Pane KEEPS its CARD --------------------------

/// THE BARE-PLATE LAW: a Bars list keeps the live page by drawing only one local
/// scrim per plate, never an elevated card or a full-canvas room. Pane remains
/// the historical card treatment. The pixel outcome law below proves the page
/// itself survives; this count-level companion keeps a future broad rectangle
/// from quietly returning beneath the plates.
#[test]
fn bars_float_bounded_plates_pane_keeps_its_card() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!("skipping bars_float_bounded_plates_pane_keeps_its_card: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_card_anchor_test_override(Some(theme::CardAnchor::TopLeft));

    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = (0..8).map(|i| format!("Command {i}")).collect();
    v.overlay_selected = 2;

    // PANE (default): the card fill draws its instance(s); no bars. Pin
    // `Unified` so this Pane-vs-Bars mechanism check reads the single historical
    // card fill — the two-surface SPLIT (the DEFAULT) is its own law
    // (`split_pane.rs`).
    set_pane_split_test_override(Some(theme::PaneSplit::Unified));
    set_list_style_test_override(Some(theme::ListStyle::Pane));
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert_eq!(p.panel_card.instance_count(), 1, "Pane draws the card fill");
    assert_eq!(p.overlay_bars.instance_count(), 0, "Pane draws no bars");
    set_pane_split_test_override(None);

    // BARS: the boxed pane vanishes — shadow + border park empty (no elevation) —
    // and `panel_card` carries only one local scrim per bar plate.
    set_list_style_test_override(Some(bars(6.0, 10.0, 24.0)));
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    let plates = p.overlay_bars.instance_count() + p.overlay_rows.instance_count();
    assert_eq!(p.panel_card.instance_count(), plates, "Bars paint one bounded scrim per plate");
    assert_eq!(p.panel_shadow.instance_count(), 0, "Bars draw no card shadow (no elevation)");
    assert_eq!(p.panel_border.instance_count(), 0, "Bars draw no card border (no elevation)");
    assert!(p.overlay_bars.instance_count() > 0, "Bars draw a surface per row");

    set_list_style_test_override(None);
    set_card_anchor_test_override(None);
}

// --- Bars: a surface per row, the selected one FINDABLE (real pixels) --------

/// The average RGB over a small region of one rendered frame (surface color,
/// sampled where no glyphs fall). A tiny local copy of the same shape
/// `distinguishability.rs`/`syntax_roles.rs` carry (accepted per-file dup).
fn avg(pixels: &[[u8; 4]], w: i64, h: i64, x: i64, y: i64, rw: i64, rh: i64) -> theme::Srgb {
    let (x0, y0) = (x.max(0), y.max(0));
    let (x1, y1) = ((x + rw).min(w), (y + rh).min(h));
    let mut s = [0u64; 3];
    let mut n = 0u64;
    for yy in y0..y1 {
        for xx in x0..x1 {
            let p = pixels[(yy * w + xx) as usize];
            s[0] += p[0] as u64;
            s[1] += p[1] as u64;
            s[2] += p[2] as u64;
            n += 1;
        }
    }
    assert!(n > 0, "empty sample region");
    theme::Srgb::rgb((s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8)
}

fn redmean(a: theme::Srgb, b: theme::Srgb) -> f32 {
    let rbar = (a.r as f32 + b.r as f32) * 0.5;
    let dr = a.r as f32 - b.r as f32;
    let dg = a.g as f32 - b.g as f32;
    let db = a.b as f32 - b.b as f32;
    ((2.0 + rbar / 256.0) * dr * dr + 4.0 * dg * dg + (2.0 + (255.0 - rbar) / 256.0) * db * db)
        .sqrt()
}

#[test]
fn bars_draw_a_findable_surface_per_row() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping bars_draw_a_findable_surface_per_row: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    // Pin SALTPAN — the critique's own light-world proof that the whisper reads
    // (its base_100 paper vs base_200 bar is a clear value step). A bars world
    // floats bounded plates over the live page. The whisper remains intentionally
    // quiet; this law only requires it to stay visibly present against the gap.
    theme::set_active_by_name("Saltpan").unwrap();

    // A flat (non-faceted) picker, selection on row 2, plenty of rows.
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = (0..8).map(|i| format!("Command number {i}")).collect();
    v.overlay_selected = 2;

    // Sharp bars (radius 0 → a clean left edge to sample), a real gap, a real grow.
    set_list_style_test_override(Some(bars(0.0, 8.0, 10.0)));
    set_card_anchor_test_override(Some(theme::CardAnchor::TopLeft));
    p.set_view(&v);
    p.prepare(&device, &queue, w, h).unwrap();

    // Mechanism witness: one bar per UNSELECTED item row (7), one selected bar.
    assert_eq!(p.overlay_bars.instance_count(), 7, "one bar per unselected item row");
    assert_eq!(p.overlay_rows.instance_count(), 1, "one selected bar");
    assert!(p.overlay_row_gap() > 0.0, "Bars opens a positive row gap");

    // OUTCOME (real pixels): the selected bar reads distinct from an unselected
    // bar, and an unselected bar still reads (a whisper) against the GROUND
    // between bars. NOTE: under Bars the boxed pane is dropped (see
    // `bars_float_bounded_plates_pane_keeps_its_card`) and each plate instead has a
    // local base_100 scrim — so the between-bars region is the live page, the ground
    // the unselected whisper (base_200) lifts off of. `overlay_card_rect` still returns the
    // layout bound (the pane's paint is gone, its geometry is not).
    let rect = p.overlay_card_rect().expect("overlay card rect");
    let (card_x, card_y, _cw) = (rect[0], rect[1], rect[2]);
    let text_top = card_y + 12.0; // `overlay_geometry`'s inner pad (vertical)
    let lh = p.overlay_lh();
    let gap = p.overlay_row_gap();
    let hg = p.overlay_header_gap();
    let bar_h = lh - gap;
    // The bars are CENTERED in their row pitch-cell (designer pixel-pass): each
    // bar sits `gap/2` below the cell top, so the gap splits half above / half
    // below. The sample coords fold in this offset so `sel`/`unsel` land ON a bar
    // and `ground` lands in the true (centered) gap between two bars.
    let bar_off = gap * 0.5;
    // Sample column x: inside the bar's left inset (8px) but LEFT of text_left
    // (12px) — pure surface, no glyphs.
    let sx = (card_x + 9.0) as i64;
    let row_top = |r: usize| chrome::overlay_row_top(text_top, 1, hg, r, lh);
    let px = pixeldiff::render_frame(&mut p, &device, &queue, w, h);
    let (wi, hi) = (w as i64, h as i64);

    let sel = avg(&px, wi, hi, sx, (row_top(2) + bar_off + 2.0) as i64, 2, (bar_h - 4.0) as i64);
    let unsel = avg(&px, wi, hi, sx, (row_top(0) + bar_off + 2.0) as i64, 2, (bar_h - 4.0) as i64);
    // The gap between row 0 and row 1 shows the live page (no pane). Bar 0's
    // bottom is `row_top(0) + bar_off + bar_h`; the gap runs from
    // there for `gap` px.
    let ground = avg(
        &px,
        wi,
        hi,
        sx,
        (row_top(0) + bar_off + bar_h + 1.0) as i64,
        2,
        (gap - 2.0) as i64,
    );

    let d_sel = redmean(sel, unsel);
    let d_bar = redmean(unsel, ground);
    set_list_style_test_override(None);
    set_card_anchor_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);
    assert!(
        d_sel >= 10.0,
        "selected bar {sel:?} must be findable vs an unselected bar {unsel:?} (redmean {d_sel:.1})"
    );
    assert!(
        d_bar >= 5.0,
        "an unselected bar {unsel:?} must still read (a present whisper) against the ground {ground:?} between bars (redmean {d_bar:.1})"
    );
    // THE OBVIOUS-GLANCE LAW (the Firetail "picket fence" gallery defect the user
    // rejected): the first cut drew unselected bars as saturated slabs one lone
    // rung under the selected band — every row shouted, and the selected bar had
    // nowhere to go. The refit drops the pane and quiets the unselected bar to a
    // WHISPER off the ground (`base_200`), so the selected bar's strong pop now
    // leads its NEIGHBOURS at least as much as a neighbouring whisper leads the
    // bare GROUND — selection dominates the rhythm, an obvious glance. (With the
    // whisper this holds comfortably: d_bar is small by design, d_sel large.)
    assert!(
        d_sel >= d_bar,
        "selected bar must lead its neighbours (redmean {d_sel:.1}) at least as much as a whisper bar leads the bare ground (redmean {d_bar:.1}) — an obvious glance, not close inspection"
    );
}

// --- Spell popup: floats BARE (no room box) on Bars, keeps the card on Pane ---

/// THE SPELL-POPUP BACKING LAW, ENUMERATED OVER EVERY WORLD (real pixels) —
/// OPTION B (the user's "b is good"): on a Bars world the contextual autocorrect
/// popup floats its suggestion plates on the RAW PAGE with NO room box AT ALL.
/// The prior round clipped a `base_100` room to the card; on a DARK world that
/// read as a prominent near-black BOX behind the plates, which the user wanted
/// gone. Legibility over the live document is now carried by each plate's own
/// minimal ground SCRIM (a thin feathered moat confined to the plate footprint —
/// `overlay_draw_card`), never a rectangle. Classified by the ONE row-backing
/// owner [`theme::ListStyle::list_backing`] so a NEW world is auto-decided by its
/// own `list_style` (NO WILDCARD, no per-world branch — a new Bars world joins
/// the bare-plates set for free, a new Pane world the card set). Asserted over
/// the rendered BYTES (the Wagtail lesson — appearance over PIXELS, never
/// inferred from state):
///   - BARS worlds (Firetail-family, DARK ONES INCLUDED): NO raised float pane
///     (`float_card == 0`), and — the crux — the DOCUMENT SHOWS THROUGH between
///     the plates. Rendered twice (bare page vs page + popup); in every
///     inter-plate GAP the two frames are byte-for-byte equal, so nothing (no
///     `base_100` box) covers the page there. The bare-page strip is proven to
///     carry real doc text first, so the check is not vacuous. The SELECTED plate
///     stays a findable surface (a clear value step from the ground it floats on).
///   - PANE worlds (unchanged): the raised float pane IS present
///     (`float_card == 1`) and its opaque `base_300` fill covers the strip just
///     inside the card's left edge — the pre-refit popup the user kept everywhere
///     else. (A one-bit world's ramp collapses `base_300` onto `base_100`, so the
///     fill-vs-ground redmean is ~0 there — the `d_pane <= floor` form holds
///     regardless, and the pane's own definition is its border, law-tested in
///     `one_bit.rs`.)
#[test]
fn spell_popup_floats_bare_on_bars_keeps_the_card_on_pane() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping spell_popup_floats_bare_on_bars_keeps_the_card_on_pane: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    let (wi, hi) = (w as i64, h as i64);

    // A DENSE document: the misspelled word "teh" at line 0, then many long
    // single-token lines (no spaces → every sampled strip is full of glyphs) that
    // sit DIRECTLY BEHIND the popup. So every between-plate gap is guaranteed to
    // land over real document text — the case a room box WOULD cover, which is the
    // whole point of proving the page shows THROUGH.
    let mut doc = String::from("teh quick brown fox jumps over\n");
    for _ in 0..18 {
        doc.push_str("loremipsumdolorsitametconsecteturadipiscingelit\n");
    }

    // NO-WILDCARD over the whole roster: every shipped world, each decided by its
    // OWN list_style through the one row-backing owner.
    for t in theme::THEMES.iter() {
        theme::set_active_by_name(t.name).unwrap();
        p.sync_theme();
        // Per-world tokens (read AFTER set_active — never hoist a palette value
        // out of the world loop).
        let base300 = theme::base_300();
        let backing = t.render_caps.list_style.list_backing(true);

        // Frame B: the page + the spell popup over "teh" (cols [0,3)).
        let mut b = view(&doc, 0, 0);
        b.overlay_active = true;
        b.overlay_items = vec!["the".into(), "tea".into(), "ten".into(), "ted".into()];
        b.overlay_selected = 0;
        b.overlay_spell = Some((0, 0, 3));
        p.set_view(&b);
        p.prepare(&device, &queue, w, h).unwrap();
        // Read geometry + mechanism witnesses from THIS (popup) prepare, before the
        // bare-page render below parks the plates.
        let float_n = p.float_card.instance_count();
        let n_plates = p.overlay_bars.instance_count() + p.overlay_rows.instance_count();
        let rect = p.overlay_card_rect().expect("spell popup has a card rect");
        let (cx, cy, cw, ch) = (rect[0], rect[1], rect[2], rect[3]);
        let lh = p.overlay_lh();
        let gap = p.overlay_row_gap();
        let pb = pixeldiff::render_frame(&mut p, &device, &queue, w, h);
        let base100 = theme::base_100();

        match backing {
            theme::ListBacking::BarePlates => {
                assert_eq!(
                    float_n, 0,
                    "{}: a Bars world floats the spell plates BARE — no raised float pane",
                    t.name
                );
                assert!(
                    n_plates >= 2,
                    "{}: need >= 2 plates to sample a between-plate gap (got {n_plates})",
                    t.name
                );

                // Frame A: the BARE PAGE (no popup) — the "page ground" B is compared
                // to. The spell popup recedes nothing (no blur/scrim — `overlay_blur`
                // exempts it), so the document layer is rendered IDENTICALLY in both
                // frames; wherever the popup draws nothing, B must equal A.
                let a = view(&doc, 0, 0);
                p.set_view(&a);
                p.prepare(&device, &queue, w, h).unwrap();
                let pa = pixeldiff::render_frame(&mut p, &device, &queue, w, h);

                // THE NO-ROOM-BOX OUTCOME (DARK-INCLUSIVE), measured directly over the
                // whole CARD footprint: how much of the bare page's DOCUMENT TEXT
                // survives UNCHANGED under the popup. A `base_100` room box (the prior
                // round's clipped room — a near-black box on the dark worlds) covers
                // the entire card, so ZERO bare-page text would show through; option B
                // draws only the plates + their thin scrims, so the page — text and
                // all — reads BETWEEN and AROUND them. Compare A vs B per pixel: a
                // bare-page TEXT pixel (far enough from the ground to be a glyph, not
                // AA) that is byte-for-byte identical in B SURVIVED — the document
                // showing through. Independent of the exact plate geometry, so it can
                // never land in a doc inter-line gap and read vacuous.
                let (x0, y0) = (cx.max(0.0) as i64, cy.max(0.0) as i64);
                let x1 = ((cx + cw).min(w as f32)) as i64;
                let y1 = ((cy + ch).min(h as f32)) as i64;
                let mut card_text = 0i64; // bare-page glyph pixels inside the card
                let mut survived = 0i64; // ... that show THROUGH the popup unchanged
                for yy in y0..y1 {
                    for xx in x0..x1 {
                        let ia = pa[(yy * wi + xx) as usize];
                        let is_text =
                            redmean(theme::Srgb::rgb(ia[0], ia[1], ia[2]), base100) > 40.0;
                        if !is_text {
                            continue;
                        }
                        card_text += 1;
                        let ib = pb[(yy * wi + xx) as usize];
                        if (0..3).all(|c| (ia[c] as i64 - ib[c] as i64).abs() <= 2) {
                            survived += 1;
                        }
                    }
                }
                // NOT VACUOUS: the fixture really does put doc text behind the popup.
                assert!(
                    card_text > 1000,
                    "{}: the fixture must put real doc text behind the popup (only {card_text} glyph px in the card) — else the no-box law is vacuous",
                    t.name
                );
                // NO ROOM BOX: a large share of that text shows THROUGH untouched. A
                // room box would cover the whole card → survived ~ 0.
                assert!(
                    survived * 3 >= card_text,
                    "{}: the document must show THROUGH the popup — only {survived}/{card_text} bare-page glyph px survived under it; a base_100 room box would cover them ALL (the near-black box on the dark worlds)",
                    t.name
                );

                // LEGIBILITY: the SELECTED plate is a findable surface — a clear value
                // step from the ground it floats on (per the bar laws), so the popup
                // reads as a floating object over the live document even without a
                // room. Sample the selected (row-0) plate + its glyph. row 0 top = the
                // popup's inner pad (10) + the bar's own gap/2 offset.
                let row0 = chrome::overlay_row_top(cy + 10.0, 0, 0.0, 0, lh);
                let sel_top = row0 + gap * 0.5;
                let sel = avg(
                    &pb,
                    wi,
                    hi,
                    (cx + 10.0) as i64,
                    (sel_top + 2.0) as i64,
                    60,
                    (lh - gap - 4.0).max(2.0) as i64,
                );
                let d_sel = redmean(sel, base100);
                assert!(
                    d_sel >= 20.0,
                    "{}: the SELECTED suggestion plate {sel:?} must read as a clear value step from the ground {base100:?} (redmean {d_sel:.1})",
                    t.name
                );
            }
            theme::ListBacking::Card => {
                assert_eq!(
                    float_n, 1,
                    "{}: a Pane world keeps its raised float pane behind the spell popup",
                    t.name
                );
                // The card background reads the opaque base_300 float fill — the
                // unchanged pre-refit pane. A thin strip just inside the card's LEFT
                // edge, over the lower rows. `<= 20` tolerates AA + the one-bit ramp
                // collapse (base_300 == base_100 there → ~0).
                let bg = avg(
                    &pb,
                    wi,
                    hi,
                    (cx + 5.0) as i64,
                    (cy + ch * 0.5) as i64,
                    3,
                    (ch * 0.5 - 12.0) as i64,
                );
                let d_pane = redmean(bg, base300);
                assert!(
                    d_pane <= 20.0,
                    "{}: the spell popup background {bg:?} must read the base_300 PANE fill {base300:?} \
                     (redmean {d_pane:.1}) — the unchanged pre-refit pane, not a Bars ground room",
                    t.name
                );
            }
        }
    }
    theme::set_active(theme::DEFAULT_THEME);
}

// --- Bars: the query caret sits ON the query text (real pixels) --------------

/// FULL-BLEED CARET LAW (real pixels): under `Bars` the flat picker draws no
/// card, and the beat inflates the query line's height by `header_gap` — where
/// cosmic-text half-leads the glyphs DOWN. A caret pinned to `overlay_lh() *
/// 0.5` floated a full half-beat ABOVE the text (the designer's full-bleed
/// caret bug: caret y 73-91 while the glyphs sat at 94-108, ZERO overlap). This
/// asserts the amber caret's pixel band VERTICALLY OVERLAPS the query glyphs'
/// pixel band — the OUTCOME (visible alignment), not the geometry the fix and
/// the probe both compute.
#[test]
fn bars_query_caret_overlaps_the_query_text() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping bars_query_caret_overlaps_the_query_text: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    // Bowerbird: an amber-accent, coloured-ground world where the bug was shot.
    theme::set_active_by_name("Bowerbird").unwrap();
    p.sync_theme();
    crate::render::set_list_style_test_override(Some(bars(6.0, 8.0, 24.0)));
    // The full-bleed premise is a LEFT-placed card (pre-anchor default); pin the
    // anchor to TopLeft so the query line sits at the left margin where this test's
    // pixel windows expect it (the DEFAULT anchor is now `TopCenter` — COMPOSITION-C2 —
    // which centres the card; the caret-Y-overlaps-text law under test is anchor-agnostic).
    crate::render::set_card_anchor_test_override(Some(theme::CardAnchor::TopLeft));
    let mut v = view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_title = "themes";
    v.overlay_items = (0..8).map(|i| format!("Command {i}")).collect();
    v.overlay_selected = 1;
    // FLAT path (no lens) — the real theme picker with faceting off, where the
    // query line itself carries the beat inflation.
    p.set_view(&v);
    p.prepare(&device, &queue, w, h).unwrap();
    let px = pixeldiff::render_frame(&mut p, &device, &queue, w, h);

    // The caret sits at the END of "themes › " — scan a tall column strip there
    // for AMBER pixels (high R, mid G, low B) and for the query TEXT (muted grey,
    // clearly above the dark room ground) to its LEFT.
    let idx = |x: i64, y: i64| px[(y * w as i64 + x) as usize];
    let is_amber = |q: [u8; 4]| q[0] > 180 && q[1] > 100 && q[1] < 200 && q[2] < 110;
    let (mut a_y0, mut a_y1) = (i64::MAX, i64::MIN);
    for y in 40..140 {
        for x in 110..175 {
            if is_amber(idx(x, y)) {
                a_y0 = a_y0.min(y);
                a_y1 = a_y1.max(y);
            }
        }
    }
    assert!(a_y0 <= a_y1, "amber query caret not found near the query line");
    // The query GLYPH band: pixels in the title text x-range (40..108) that are
    // notably brighter than the dark room ground.
    let ground = idx(700, 60); // empty room, above the first bar
    let bright = |q: [u8; 4]| {
        let d = |c: usize| (q[c] as i64 - ground[c] as i64).max(0);
        d(0) + d(1) + d(2) > 45
    };
    let (mut t_y0, mut t_y1) = (i64::MAX, i64::MIN);
    for y in 40..140 {
        for x in 40..108 {
            if bright(idx(x, y)) {
                t_y0 = t_y0.min(y);
                t_y1 = t_y1.max(y);
            }
        }
    }
    assert!(t_y0 <= t_y1, "query text glyphs not found on the query line");

    let a_mid = (a_y0 + a_y1) / 2;
    // OUTCOME: the caret's vertical centre must land INSIDE the query glyphs'
    // own vertical band (a small margin for the caret extending a hair past the
    // x-height top/baseline). The OLD bug put `a_mid` a whole line above `t_y0`.
    assert!(
        a_mid >= t_y0 - 3 && a_mid <= t_y1 + 3,
        "full-bleed caret bug: amber caret band [{a_y0},{a_y1}] (mid {a_mid}) must sit \
         ON the query text band [{t_y0},{t_y1}], not float above it"
    );
    // And a stronger check that the two bands genuinely OVERLAP, not merely touch.
    let overlap = a_y1.min(t_y1) - a_y0.max(t_y0);
    assert!(
        overlap > 0,
        "caret band [{a_y0},{a_y1}] and text band [{t_y0},{t_y1}] must overlap"
    );

    crate::render::set_card_anchor_test_override(None);
    crate::render::set_list_style_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);
}

/// POSTER BARS KEEP THE LIVE PAGE (real pixels): Mangrove, Firetail, and
/// Cassowary all ship the shared Bars treatment. Every centered list kind must
/// therefore leave meaningful source glyphs untouched inside its own layout
/// footprint — a full-canvas room would make that count exactly zero — while its
/// selected and unselected plates still read as distinct surfaces. The explicit,
/// no-wildcard `OverlayKind` match below makes a new centered kind declare its
/// regime before this outcome law can compile.
#[test]
fn poster_bars_centered_lists_preserve_page_and_distinguish_plates() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping poster_bars_centered_lists_preserve_page_and_distinguish_plates: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    use crate::overlay::OverlayKind;
    let doc = (0..42)
        .map(|line| format!("Witness glyphs remain visible behind summoned list surface {line:02}."))
        .collect::<Vec<_>>()
        .join("\n");

    for world in ["Mangrove", "Firetail", "Cassowary"] {
        theme::set_active_by_name(world).unwrap();
        p.sync_theme();
        assert!(
            matches!(theme::active().render_caps.list_style, theme::ListStyle::Bars { .. }),
            "{world} must remain a shipped Bars world for this poster treatment law"
        );

        // A no-text frame is the source-glyph oracle: comparing the document frame
        // to it identifies actual document pixels without mistaking a world's loud
        // backdrop, page frame, or poster treatment for writing.
        let empty = view("", 0, 0);
        p.set_view(&empty);
        p.prepare(&device, &queue, w, h).unwrap();
        let blank_page = pixeldiff::render_frame(&mut p, &device, &queue, w, h);

        for kind in OverlayKind::ALL {
            let centered = match kind {
                OverlayKind::Spell => false,
                OverlayKind::Theme
                | OverlayKind::Goto
                | OverlayKind::Browse
                | OverlayKind::Project
                | OverlayKind::Command
                | OverlayKind::History
                | OverlayKind::Settings
                | OverlayKind::Caret
                | OverlayKind::Dictionary
                | OverlayKind::CjkLang
                | OverlayKind::Date
                | OverlayKind::MoveDest
                | OverlayKind::Keybindings
                | OverlayKind::Assets
                | OverlayKind::Rename
                | OverlayKind::InsertLink
                | OverlayKind::KeepName => true,
            };
            if !centered {
                continue;
            }

            let bare = view(&doc, 0, 0);
            p.set_view(&bare);
            p.prepare(&device, &queue, w, h).unwrap();
            let page = pixeldiff::render_frame(&mut p, &device, &queue, w, h);

            let mut overlay = view(&doc, 0, 0);
            overlay.overlay_active = true;
            overlay.overlay_title = "actions";
            overlay.overlay_items = (0..7)
                .map(|i| format!("{kind:?} action label {i}"))
                .collect();
            overlay.overlay_selected = 3;
            overlay.overlay_hint = "↑/↓ move · Enter choose · Esc dismiss".into();
            if crate::facets::scheme(kind).is_some() {
                overlay.overlay_lens = vec![("All".into(), true), ("Writing".into(), false)];
            }
            p.set_view(&overlay);
            p.prepare(&device, &queue, w, h).unwrap();
            let over = pixeldiff::render_frame(&mut p, &device, &queue, w, h);

            // The matching no-document overlay frame discounts every intentional
            // overlay effect (placard, plates, local scrims, and any backdrop
            // treatment). What remains is specifically source writing that can
            // still be seen through the summoned list.
            overlay.text.clear();
            p.set_view(&overlay);
            p.prepare(&device, &queue, w, h).unwrap();
            let over_blank = pixeldiff::render_frame(&mut p, &device, &queue, w, h);

            let [cx, cy, cw, ch] = p.overlay_card_rect().expect("centered overlay card rect");
            let (wi, hi) = (w as i64, h as i64);
            let (x0, y0) = (cx.max(0.0) as i64, cy.max(0.0) as i64);
            let (x1, y1) = (((cx + cw).min(w as f32)) as i64, ((cy + ch).min(h as f32)) as i64);
            let mut source_glyphs = 0i64;
            let mut survived = 0i64;
            for yy in y0..y1 {
                for xx in x0..x1 {
                    let before = page[(yy * wi + xx) as usize];
                    let blank = blank_page[(yy * wi + xx) as usize];
                    let source_delta = (0..3)
                        .map(|c| (before[c] as i64 - blank[c] as i64).abs())
                        .max()
                        .unwrap_or(0);
                    if source_delta <= 20 {
                        continue;
                    }
                    source_glyphs += 1;
                    let after = over[(yy * wi + xx) as usize];
                    let after_blank = over_blank[(yy * wi + xx) as usize];
                    let after_delta = (0..3)
                        .map(|c| (after[c] as i64 - after_blank[c] as i64).abs())
                        .max()
                        .unwrap_or(0);
                    if after_delta > 20 {
                        survived += 1;
                    }
                }
            }
            assert!(
                source_glyphs > 1_000,
                "{world} {kind:?}: the fixture must put real source glyphs under the list (got {source_glyphs})"
            );
            assert!(
                survived * 4 >= source_glyphs,
                "{world} {kind:?}: only {survived}/{source_glyphs} source glyph pixels survived; a full-page room obscures them"
            );

            let probe = p.overlay_row_y_probe();
            let gap = p.overlay_row_gap();
            let bar_h = (probe.lh - gap).max(1.0);
            let sx = (cx + 14.0) as i64;
            let selected = avg(
                &over, wi, hi, sx, (probe.band_top + gap * 0.5 + 2.0) as i64, 3,
                (bar_h - 4.0).max(2.0) as i64,
            );
            let unselected = avg(
                &over, wi, hi, sx, (probe.band_top - probe.lh + gap * 0.5 + 2.0) as i64, 3,
                (bar_h - 4.0).max(2.0) as i64,
            );
            let distinction = redmean(selected, unselected);
            assert!(
                distinction >= 10.0,
                "{world} {kind:?}: selected plate {selected:?} must read distinctly from unselected {unselected:?} (redmean {distinction:.1})"
            );
        }
    }
    theme::set_active(theme::DEFAULT_THEME);
}

// --- FacetStyle: Band visibly differs from the Text baseline -----------------

/// THE FACET-ARM-DRAWS LAW (instance-count + pixel delta). Born from the
/// facet-chips GALLERY TRAP (fixed @ e56d689): the retired `Chips` skin parsed
/// to `None` and SILENTLY rendered the `Text` default, so `bowerbird-facet-
/// chips.png` came out byte-identical to `-text` — a shot named for a variant
/// that never fired. `Chips` is dropped for cause (the designer pixel-pass chose
/// `Band`); this law pins the SURVIVING facet arm honestly, both ways the trap
/// asked for:
///   - INSTANCE COUNT: the active-lens mark pipeline (`overlay_lens_underline`)
///     draws a NON-ZERO instance under BOTH skins — the facet arm is never a
///     silent no-op (a transparent/empty mark is exactly what shipped invisible
///     in the Wagtail bug).
///   - PIXEL DELTA: `Band` renders PERCEPTIBLY DIFFERENT from `Text` in the strip
///     row (the value pill vs the hairline) — so the two are a real two-way, not
///     a masquerading duplicate.
#[test]
fn facet_band_draws_and_differs_from_text_in_the_strip() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping facet_band_draws_and_differs: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    // A faceted picker with an ACTIVE facet so the band has a target.
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = (0..8).map(|i| format!("Command {i}")).collect();
    v.overlay_selected = 1;
    v.overlay_lens = vec![("All".into(), false), ("File".into(), true), ("Edit".into(), false)];

    let frame = |p: &mut TextPipeline, style: Option<theme::FacetStyle>| {
        set_facet_style_test_override(style);
        p.set_view(&v);
        p.prepare(&device, &queue, w, h).unwrap();
        pixeldiff::render_frame(p, &device, &queue, w, h)
    };

    let text = frame(&mut p, Some(theme::FacetStyle::Text));
    let text_marks = p.overlay_lens_underline.instance_count();
    let band = frame(&mut p, Some(theme::FacetStyle::Band));
    let band_marks = p.overlay_lens_underline.instance_count();
    set_facet_style_test_override(None);

    // INSTANCE COUNT: the active-lens mark is actually painted in BOTH skins —
    // never the silent empty draw the chips shot masqueraded as.
    assert!(text_marks > 0, "Text facet arm must draw the active-lens hairline (got 0)");
    assert!(band_marks > 0, "Band facet arm must draw the active-lens pill (got 0)");

    // PIXEL DELTA: the strip row (display line 1) must visibly change under Band.
    let rect = p.overlay_card_rect().expect("overlay card rect");
    let (card_x, card_y, cw) = (rect[0], rect[1], rect[2]);
    let text_top = card_y + 12.0;
    let lh = p.overlay_lh();
    let strip = pixeldiff::Region::new(card_x, text_top + lh, cw, lh);
    pixeldiff::assert_perceptibly_different(
        &text, &band, w as i64, h as i64, strip, pixeldiff::DistinguishFloor::DEFAULT,
        "facet Band vs Text strip",
    );
}

// --- Bars DROP THE PANE for EVERY overlay kind (the no-wildcard card-fill law) -

/// THE PER-KIND PANE-DROP LAW (user-caught bug, 2026-07-16): the no-pane-under-
/// Bars change reached the THEME picker but NOT the go-to / faceted kinds — the
/// maximalist shot showed a full BORDERED card behind the bars, the selected bar
/// overflowing its right edge (the jut colliding with a wall that shouldn't
/// exist). The fix lives at the ONE overlay-card paint owner
/// (`overlay_draw_card`), gated only by `effective_list_style()` — never per
/// kind — so this law enumerates `OverlayKind::ALL` with a NO-WILDCARD match and
/// proves, for EVERY kind, that Bars drops the boxed pane and avoids a room:
///   - the contextual SPELL popup ALSO drops its pane under Bars (the user's
///     Firetail refit extended to the autocorrect popup — "for the autocorrect,
///     get rid of the pane too"): NO raised float pane (`float_card == 0`, its
///     Pane-world elevation), the suggestion plates floating over the live page
///     with one bounded scrim each, never the boxed base_300 float card it draws
///     on Pane.
///   - EVERY other kind: ZERO card BORDER + ZERO card SHADOW (no boxed
///     elevation), one bounded SCRIM per plate (never a canvas room), a bar per
///     row, and the selected bar drawn — so its `grow_px` jut has NO card wall to
///     clip against (the board bug). Faceting kinds are
///     driven through the `geom.theme` card path too (an active lens strip),
///     since the board bug lived on the FACETED card, not the flat one.
/// A new `OverlayKind` fails to compile here until it declares which regime it
/// is — the structural guard against a future per-kind card special case.
#[test]
fn bars_float_bounded_plates_for_every_overlay_kind() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping bars_float_bounded_plates_for_every_overlay_kind: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_card_anchor_test_override(Some(theme::CardAnchor::TopLeft));
    set_list_style_test_override(Some(bars(6.0, 10.0, 24.0)));

    use crate::overlay::OverlayKind;
    for kind in OverlayKind::ALL {
        // NO-WILDCARD: Spell is the sole CONTEXTUAL popup (a float over the doc,
        // not a centered list card); every kind — Spell included — must drop its
        // boxed pane under Bars. Spell's drop looks different (no `float_*` pane)
        // so it has its own assertion block below.
        let is_spell = match kind {
            OverlayKind::Spell => true,
            OverlayKind::Theme
            | OverlayKind::Goto
            | OverlayKind::Browse
            | OverlayKind::Project
            | OverlayKind::Command
            | OverlayKind::History
            | OverlayKind::Settings
            | OverlayKind::Caret
            | OverlayKind::Dictionary
            | OverlayKind::CjkLang
            | OverlayKind::Date
            | OverlayKind::MoveDest
            | OverlayKind::Keybindings
            | OverlayKind::Assets
            | OverlayKind::Rename
            | OverlayKind::InsertLink
            | OverlayKind::KeepName => false,
        };

        let mut v = view("the quick brown fox jumps\n", 0, 0);
        v.overlay_active = true;
        v.overlay_items = (0..6).map(|i| format!("Item {i}")).collect();
        v.overlay_selected = 2;
        v.overlay_hint = "hint".into();
        if is_spell {
            // the contextual spell popup, anchored at the word "quick" (cols 4..9)
            v.overlay_spell = Some((0, 4, 9));
        } else if crate::facets::scheme(kind).is_some() {
            // exercise the FACETED (`geom.theme`) card path too — the board bug
            // lived on the faceted card. An active lens gives the strip a target.
            v.overlay_lens = vec![("All".into(), true), ("File".into(), false)];
        }
        p.set_view(&v);
        p.prepare(&device, &queue, w, h).unwrap();

        if is_spell {
            // SPELL ON BARS (OPTION B — the user's "b is good"): the autocorrect
            // popup floats its suggestion plates on the RAW PAGE with NO room box
            // at all (the prior round's clipped `base_100` room read as a
            // near-black BOX on the dark worlds). Its Pane-world `float_*`
            // elevation parks empty (no raised pane), and the
            // `panel_shadow`/`panel_border` stay empty (no card) — but in place of
            // any broad room, `panel_card` carries ONE minimal ground SCRIM
            // PER PLATE (a thin feathered moat confined to each plate's footprint),
            // so its count MATCHES the plate count, never 1. The document shows
            // BETWEEN the plates; the pixel-level no-box + legibility outcome is
            // law-tested in `spell_popup_floats_bare_on_bars_keeps_the_card_on_pane`.
            assert_eq!(
                p.float_card.instance_count(),
                0,
                "{kind:?}: Bars draws NO raised float pane behind the spell popup"
            );
            assert_eq!(
                p.float_shadow.instance_count(),
                0,
                "{kind:?}: Bars draws NO float shadow behind the spell popup"
            );
            assert_eq!(
                p.float_border.instance_count(),
                0,
                "{kind:?}: Bars draws NO float border behind the spell popup"
            );
            let plates = p.overlay_bars.instance_count() + p.overlay_rows.instance_count();
            assert!(
                plates > 1,
                "{kind:?}: the spell suggestions draw a plate per row (got {plates})"
            );
            assert_eq!(
                p.panel_card.instance_count(),
                plates,
                "{kind:?}: the spell popup floats BARE — ONE ground scrim per plate ({plates}), NOT a single room box"
            );
            assert_eq!(
                p.panel_shadow.instance_count(),
                0,
                "{kind:?}: the spell scrims carry no shadow (no elevation)"
            );
            assert_eq!(
                p.panel_border.instance_count(),
                0,
                "{kind:?}: the spell scrims carry no border (no elevation)"
            );
            continue;
        }
        assert_eq!(
            p.panel_border.instance_count(),
            0,
            "{kind:?}: Bars draws NO card border (the wall the selected bar's grow collided with)"
        );
        assert_eq!(
            p.panel_shadow.instance_count(),
            0,
            "{kind:?}: Bars draws NO card shadow (no boxed elevation)"
        );
        let plates = p.overlay_bars.instance_count() + p.overlay_rows.instance_count();
        assert_eq!(
            p.panel_card.instance_count(),
            plates,
            "{kind:?}: Bars paint one bounded scrim per plate, not a full-canvas room"
        );
        assert!(
            p.overlay_bars.instance_count() > 0,
            "{kind:?}: Bars draws a surface per row"
        );
        assert_eq!(
            p.overlay_rows.instance_count(),
            1,
            "{kind:?}: the selected bar is drawn — unclipped, no card wall (grow geometry proven by selected_bar_grows_wider_toward_the_open_margin_and_mirrors)"
        );
    }

    set_list_style_test_override(None);
    set_card_anchor_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);
}

// --- Bars: the foot hint stays legible over a giant corner PLACARD ------------

/// THE FOOTER-OVER-POSTER GUARANTEE (taste-gate finding): under Bars the pane is
/// dropped, so a giant corner PLACARD (`TitleStyle::Placard` — Firetail's
/// wordmark, bottom-left anchored, bleeding UP) sat directly BEHIND the dim
/// foot-hint row and drowned it — the muted glyphs and the poster letters at
/// near-equal value (DESIGN §5's legibility floor breached). The fix lays an
/// opaque whisper-value PLATE (`footer_plate_rect`, drawn in the bars' z-slot —
/// over the placard, under the text) across the hint/footer zone, so the footer
/// keeps its designed ground no matter what the wordmark does behind it.
///
/// This asserts the OUTCOME over real pixels, both directions of the trap:
///   - WITNESS the poster is genuinely LOUD (the test is not vacuous): a
///     bottom-left region shows a big value swing between placard-ON and
///     placard-OFF.
///   - The FOOTER band is IMMUNE to it: with the plate, the footer zone renders
///     the SAME with or without the giant wordmark behind it — the poster can no
///     longer bleed into the footer's ground. Remove the plate and the poster
///     leaks straight through, and this delta blows past the floor.
#[test]
fn bars_footer_stays_legible_over_a_giant_placard() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping bars_footer_stays_legible_over_a_giant_placard: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    // Firetail: a DARK world that actually ships a placard — the poster ink lifts
    // toward `base_content` (light) over the dark ground, the worst-case contrast.
    theme::set_active_by_name("Firetail").unwrap();
    set_card_anchor_test_override(Some(theme::CardAnchor::TopLeft));
    set_list_style_test_override(Some(bars(6.0, 10.0, 24.0)));

    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_title = "commands";
    v.overlay_items = (0..8).map(|i| format!("Command {i}")).collect();
    v.overlay_selected = 2;
    v.overlay_hint = "up/dn move    run    esc close".into();
    v.overlay_window_rows = 12;

    let render_with = |p: &mut TextPipeline, ts: theme::TitleStyle| {
        set_title_style_test_override(Some(ts));
        p.set_view(&v);
        p.prepare(&device, &queue, w, h).unwrap();
        pixeldiff::render_frame(p, &device, &queue, w, h)
    };

    // WORST CASE: a big BOLD wordmark anchored bottom-left, bleeding up behind
    // the footer.
    let loud = theme::TitleStyle::Placard {
        corner: theme::PlacardCorner::BL,
        scale: 5.0,
        ink: theme::PlacardInk::Bold,
    };
    let a = render_with(&mut p, loud);
    // Geometry (title-style-independent — the placard is a canvas watermark, not
    // part of the card layout), read after prepare.
    let rect = p.overlay_card_rect().expect("overlay card rect");
    let (card_x, card_y, card_w, card_h) = (rect[0], rect[1], rect[2], rect[3]);
    let lh = p.overlay_lh();
    let hg = p.overlay_header_gap();
    // Flat picker: 8 item rows above the hint (content_rows), then the hint.
    let content_rows = 8usize;
    let hint_top = chrome::overlay_row_top(card_y + 12.0, 1, hg, content_rows, lh);
    let card_bottom = card_y + card_h;

    // CONTROL: no placard at all.
    let b = render_with(&mut p, theme::TitleStyle::InlinePrefix);
    set_title_style_test_override(None);
    set_list_style_test_override(None);
    set_card_anchor_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);

    let (wi, hi) = (w as i64, h as i64);
    // FOOTER band: from the hint row's top down to the card bottom (the plate).
    let inset = chrome::BAR_SIDE_INSET as i64;
    let fx = card_x as i64 + inset + 2;
    let fy = hint_top as i64;
    let fw = (card_w as i64 - 2 * inset - 4).max(2);
    let fh = (card_bottom - hint_top).max(2.0) as i64;
    let a_footer = avg(&a, wi, hi, fx, fy, fw, fh);
    let b_footer = avg(&b, wi, hi, fx, fy, fw, fh);

    // WITNESS region: bottom-left canvas, where the BL wordmark bleeds — pure
    // poster under `a`, plain ground under `b`.
    let a_poster = avg(&a, wi, hi, 30, hi - 90, 140, 60);
    let b_poster = avg(&b, wi, hi, 30, hi - 90, 140, 60);

    let poster_swing = redmean(a_poster, b_poster);
    let footer_delta = redmean(a_footer, b_footer);

    assert!(
        poster_swing > 20.0,
        "witness: the giant BL wordmark must render loudly (poster region redmean {poster_swing:.1}) — else this law is vacuous"
    );
    assert!(
        footer_delta < 8.0,
        "the footer ground must be IMMUNE to the poster behind it (plate guarantee): with-vs-without the wordmark the footer band changed by redmean {footer_delta:.1} (poster swung {poster_swing:.1})"
    );
}

// ============================================================================
// V6 PERSONA-5 VARIANTS — text-hugging bars, selected-only, outline, real chips
// ============================================================================
//
// Four INERT-by-default axes the user's P5 study asked for. Each is reachable
// only through the `AWL_*_FORCE` probes / test overrides; the DEFAULT arm is the
// shipped v5 look (a bare `bars` is byte-identical to before this round — the
// `parse_list_style_force_grammar` case above pins that). These prove the
// OUTCOME over real pixels / pure geometry (the Wagtail invisible-row lesson),
// never the mere mechanism.

/// TEXT-HUGGING BARS (pure geometry) — `bar_hug_span` sizes a bar to its own
/// row's CONTENT text width: a SHORT primary yields a bar much narrower than full
/// width (ragged right, the P5 main-menu look), sharing the full-width LEFT edge;
/// a LONGER primary widens its bar (widths track content). V7 TASTE-GATE: a
/// shortcut is composed INLINE into the row's name line (label + gap + shortcut),
/// so a shortcut row is just a row with a wider `primary_px` — EVERY row hugs its
/// own content, there is no full-width special case. A very long primary CLAMPS at
/// the full-width right edge, never jutting past the card.
#[test]
fn bar_hug_span_hugs_content_and_rags_by_length() {
    let (cx, cw) = (100.0, 500.0);
    // text_left = card_x + BAR_SIDE_INSET + BAR_TEXT_PAD (what the renderer feeds).
    let text_left = cx + chrome::BAR_SIDE_INSET + chrome::BAR_TEXT_PAD;
    let full = chrome::bar_full_span(cx, cw);

    // A short primary → a bar much narrower than full width, same left edge.
    let short = chrome::bar_hug_span(cx, cw, text_left, 60.0);
    assert!(
        short.1 < full.1 - 100.0,
        "a short-text hug bar is much narrower than full width: {short:?} vs full {full:?}"
    );
    assert!((short.0 - full.0).abs() < 1e-3, "the hug bar shares the full-width LEFT edge");

    // A LONGER primary (e.g. label + inline shortcut) → a wider bar (ragged:
    // widths track content), still short of full width.
    let longer = chrome::bar_hug_span(cx, cw, text_left, 200.0);
    assert!(longer.1 > short.1 + 100.0, "a longer content widens its hug bar (ragged edges)");
    assert!(
        longer.1 < full.1,
        "a mid-length content still hugs — never pinned to full width: {longer:?} vs {full:?}"
    );

    // A very long primary clamps at the full-width right edge (never juts past).
    let long = chrome::bar_hug_span(cx, cw, text_left, 9999.0);
    assert!(
        long.0 + long.1 <= full.0 + full.1 + 1e-3,
        "a long primary clamps at the full-width right edge, never past the card: {long:?}"
    );
}

/// V8 — the FOOTER PLATE follows the SAME hug rule as the rows: under a hugging
/// list style (`hug = Some`) the plate HUGS its footer content (a lone full-width
/// plate under ragged pills read as out of family — the "all rows hug" finding);
/// full-width bars (`hug = None`) keep the byte-identical `card_w`-spanning plate.
/// EITHER way the plate still COVERS the footer glyphs (right edge >= text_left +
/// content_px), so the footer-over-poster legibility guarantee survives the hug.
/// The vertical span (top, height) is hug-independent — only the horizontal moves.
#[test]
fn footer_plate_hugs_content_under_hug_bars() {
    let (text_top, header_rows, header_gap) = (100.0, 1usize, 12.0);
    let (content_rows, lh) = (8usize, 30.0);
    let (card_x, card_w, card_bottom) = (60.0, 620.0, 520.0);
    let text_left = card_x + chrome::BAR_SIDE_INSET + chrome::BAR_TEXT_PAD;
    let content_px = 240.0; // a footer narrower than the full card width

    let full = chrome::footer_plate_rect(
        text_top, header_rows, header_gap, content_rows, lh, card_x, card_w, card_bottom, None,
    );
    let hug = chrome::footer_plate_rect(
        text_top, header_rows, header_gap, content_rows, lh, card_x, card_w, card_bottom,
        Some((text_left, content_px)),
    );

    // Full-width arm is the historical `card_w`-spanning plate, inset each side.
    let (fx, fw) = chrome::bar_full_span(card_x, card_w);
    assert!((full[0] - fx).abs() < 1e-3 && (full[2] - fw).abs() < 1e-3, "None → full-width plate");

    // Hug arm shares the full-width LEFT edge but is much narrower (out-of-family
    // full-width plate under ragged pills is gone).
    assert!((hug[0] - full[0]).abs() < 1e-3, "the hug plate shares the full-width LEFT edge");
    assert!(hug[2] < full[2] - 100.0, "the hug plate is much narrower than full width: {hug:?} vs {full:?}");

    // …yet still COVERS the footer glyphs + pad (legibility over a placard holds).
    assert!(
        hug[0] + hug[2] >= text_left + content_px - 1e-3,
        "the hug plate still covers the footer content (right edge {:.1} >= text end {:.1})",
        hug[0] + hug[2], text_left + content_px,
    );

    // Vertical span is hug-independent — only the horizontal changed.
    assert!((hug[1] - full[1]).abs() < 1e-3 && (hug[3] - full[3]).abs() < 1e-3, "y-span is hug-independent");
}

/// TEXT-HUGGING BARS (real pixels) — with SHORT candidate names and no right
/// column, `HugText` leaves the RIGHT side of each row as bare ROOM (ragged),
/// where `FullWidth` fills it edge-to-edge with the bar. So a region on the
/// right of the candidate area renders PERCEPTIBLY DIFFERENT between the two
/// extents — the ragged look is real, not a silent duplicate of full width.
#[test]
fn hug_extent_leaves_room_to_the_right_where_full_width_fills_it() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping hug_extent_leaves_room_to_the_right: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_card_anchor_test_override(Some(theme::CardAnchor::TopLeft));

    // SHORT names, NO right column → hug bars go ragged and short.
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = (0..6).map(|i| format!("It{i}")).collect();
    v.overlay_selected = 2;

    let frame = |p: &mut TextPipeline, ext: theme::BarExtent| {
        set_list_style_test_override(Some(theme::ListStyle::Bars {
            radius: 6.0,
            gap: 10.0,
            grow_px: 0.0, // no selected jut — isolate the extent difference
            extent: ext,
            coverage: theme::BarCoverage::All,
        }));
        p.set_view(&v);
        p.prepare(&device, &queue, w, h).unwrap();
        pixeldiff::render_frame(p, &device, &queue, w, h)
    };

    let full = frame(&mut p, theme::BarExtent::FullWidth);
    let hug = frame(&mut p, theme::BarExtent::HugText);

    let rect = p.overlay_card_rect().expect("overlay card rect");
    let (card_x, card_y, cw, ch) = (rect[0], rect[1], rect[2], rect[3]);
    // The RIGHT ~35% of the candidate area: full-width fills it, hug leaves room.
    let region = pixeldiff::Region::new(card_x + cw * 0.6, card_y, cw * 0.35, ch);
    pixeldiff::assert_perceptibly_different(
        &full, &hug, w as i64, h as i64, region, pixeldiff::DistinguishFloor::DEFAULT,
        "hug vs full-width bars (ragged right edge)",
    );

    set_list_style_test_override(None);
    set_card_anchor_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);
}

/// V7 TASTE-GATE — ALL ROWS HUG: a SHORTCUT-BEARING row under `HugText` must hug
/// its own content (label + inline shortcut), NOT pin full-width. Before the fix a
/// row that carried a right-column chord extended its bar to the card's right edge
/// (two populations: ragged hug rows + full-width shortcut rows). Now the shortcut
/// rides INLINE on the row's own name line, so the separate right-aligned column is
/// dropped (`overlay_right_shown == false`) and the shortcut row's bar still leaves
/// ROOM on the right — perceptibly different from the full-width extent, where the
/// same row fills edge-to-edge.
#[test]
fn hug_shortcut_rows_hug_inline_and_leave_room() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping hug_shortcut_rows_hug_inline: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_card_anchor_test_override(Some(theme::CardAnchor::TopLeft));

    // A FLAT picker with a right column: short names + a short chord each. The chord
    // is what pinned the bar full-width before the fix.
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = (0..6).map(|i| format!("It{i}")).collect();
    v.overlay_bindings = (0..6).map(|_| "C-x".to_string()).collect();
    v.overlay_selected = 2;

    let frame = |p: &mut TextPipeline, ext: theme::BarExtent| {
        set_list_style_test_override(Some(theme::ListStyle::Bars {
            radius: 6.0,
            gap: 10.0,
            grow_px: 0.0, // isolate the extent difference
            extent: ext,
            coverage: theme::BarCoverage::All,
        }));
        p.set_view(&v);
        p.prepare(&device, &queue, w, h).unwrap();
        pixeldiff::render_frame(p, &device, &queue, w, h)
    };

    let full = frame(&mut p, theme::BarExtent::FullWidth);
    let hug = frame(&mut p, theme::BarExtent::HugText);

    // Under hug the shortcut rode INLINE — no separate right-aligned column.
    assert!(
        !p.overlay_right_shown,
        "HugText composes the shortcut inline, so the separate right column is dropped"
    );

    // The RIGHT ~35% of the candidate area: full-width fills it (bars + right-aligned
    // chords), hug leaves ROOM (short content hugs left). If shortcut rows still
    // pinned full-width, this region would match.
    let rect = p.overlay_card_rect().expect("overlay card rect");
    let (card_x, card_y, cw, ch) = (rect[0], rect[1], rect[2], rect[3]);
    let region = pixeldiff::Region::new(card_x + cw * 0.6, card_y, cw * 0.35, ch);
    pixeldiff::assert_perceptibly_different(
        &full, &hug, w as i64, h as i64, region, pixeldiff::DistinguishFloor::DEFAULT,
        "hug shortcut rows leave room (not pinned full-width)",
    );

    set_list_style_test_override(None);
    set_card_anchor_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);
}

/// FLIP-ROUND HYBRID (`BarExtent::HugLabel`) — the user's FINAL PICK: each bar's
/// PLATE hugs the row's LABEL ONLY, and the SHORTCUT chord renders as bare dim
/// text in the RIGHT-ALIGNED column, OUTSIDE any plate. Proven three ways over a
/// shortcut-bearing flat picker:
///   1. The bare right column IS drawn (`overlay_right_shown == true`) — unlike
///      `HugText`, which folds the chord INLINE and drops the column
///      (`overlay_right_shown == false`). So the hybrid keeps the chord separate.
///   2. The plate hugs the LABEL ALONE: a row's hug-width source
///      (`overlay_row_primary_px`) under `HugLabel` is STRICTLY NARROWER than
///      under `HugText` (whose name line additionally carries `gap + shortcut`).
///      The plate therefore ends at the label, never swallowing the chord.
///   3. The plate leaves ROOM on the right where `FullWidth` fills it — the
///      chord floats in bare room past the plate (a perceptible pixel diff).
#[test]
fn huglabel_hybrid_hugs_label_and_keeps_chord_in_the_right_column() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping huglabel_hybrid_hugs_label: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_card_anchor_test_override(Some(theme::CardAnchor::TopLeft));

    // A FLAT picker with a right column: short names + a short chord each.
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = (0..6).map(|i| format!("Item {i}")).collect();
    v.overlay_bindings = (0..6).map(|_| "C-x".to_string()).collect();
    v.overlay_selected = 2;

    let frame = |p: &mut TextPipeline, ext: theme::BarExtent| {
        set_list_style_test_override(Some(theme::ListStyle::Bars {
            radius: 6.0,
            gap: 10.0,
            grow_px: 0.0, // isolate the extent difference
            extent: ext,
            coverage: theme::BarCoverage::All,
        }));
        p.set_view(&v);
        p.prepare(&device, &queue, w, h).unwrap();
        pixeldiff::render_frame(p, &device, &queue, w, h)
    };

    // --- HugText frame: chord inline, no right column, plate = label + chord ---
    let _hugtext = frame(&mut p, theme::BarExtent::HugText);
    assert!(
        !p.overlay_right_shown,
        "HugText folds the chord INLINE — no separate right column"
    );
    let geom = p.overlay_geometry(w);
    let primary_hugtext = *p
        .overlay_row_primary_px(&geom)
        .get(&0)
        .expect("row 0 primary width (HugText)");

    // --- HugLabel frame: chord in the bare right column, plate = label alone ----
    let huglabel = frame(&mut p, theme::BarExtent::HugLabel);
    assert!(
        p.overlay_right_shown,
        "HugLabel keeps the chord in the BARE right-aligned column (the hybrid)"
    );
    let geom = p.overlay_geometry(w);
    let primary_huglabel = *p
        .overlay_row_primary_px(&geom)
        .get(&0)
        .expect("row 0 primary width (HugLabel)");

    // (2) The hybrid plate hugs the LABEL ALONE — strictly narrower than HugText's
    // name line (which additionally carries the inline `gap + shortcut`).
    assert!(
        primary_huglabel + 4.0 < primary_hugtext,
        "HugLabel's plate hugs the label alone (primary {primary_huglabel:.1}px) — \
         narrower than HugText's label+shortcut line ({primary_hugtext:.1}px)"
    );

    // (3) The plate leaves ROOM on the right where FullWidth fills it — the chord
    // floats past the plate. Compare the right ~35% of the candidate area.
    let full = frame(&mut p, theme::BarExtent::FullWidth);
    let rect = p.overlay_card_rect().expect("overlay card rect");
    let (card_x, card_y, cw, ch) = (rect[0], rect[1], rect[2], rect[3]);
    let region = pixeldiff::Region::new(card_x + cw * 0.6, card_y, cw * 0.35, ch);
    pixeldiff::assert_perceptibly_different(
        &full, &huglabel, w as i64, h as i64, region, pixeldiff::DistinguishFloor::DEFAULT,
        "HugLabel leaves room on the right (plate hugs the label, chord floats past it)",
    );

    set_list_style_test_override(None);
    set_card_anchor_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);
}

/// SELECTED-ONLY BAR — `SelectedOnly` coverage draws NO unselected bars (the
/// rows are bare floating text on the room, the P5 settings-screen look) while
/// the selected bar is still drawn. Proven two ways: the unselected-bar
/// instance count COLLAPSES (fewer `overlay_bars` instances than `All`), the
/// selected bar stays present (`overlay_rows == 1`), AND an unselected row's
/// bar region renders as ROOM under `SelectedOnly` vs a bar under `All`.
#[test]
fn selected_only_coverage_drops_unselected_bars_but_keeps_the_selected() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping selected_only_coverage: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_card_anchor_test_override(Some(theme::CardAnchor::TopLeft));

    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = (0..6).map(|i| format!("Item {i}")).collect();
    v.overlay_selected = 3;

    let frame = |p: &mut TextPipeline, cov: theme::BarCoverage| {
        set_list_style_test_override(Some(theme::ListStyle::Bars {
            radius: 6.0,
            gap: 10.0,
            grow_px: 0.0,
            extent: theme::BarExtent::FullWidth,
            coverage: cov,
        }));
        p.set_view(&v);
        p.prepare(&device, &queue, w, h).unwrap();
        pixeldiff::render_frame(p, &device, &queue, w, h)
    };

    let all = frame(&mut p, theme::BarCoverage::All);
    let all_bars = p.overlay_bars.instance_count();
    let all_sel = p.overlay_rows.instance_count();
    let probe = p.overlay_row_y_probe();
    let sel = frame(&mut p, theme::BarCoverage::SelectedOnly);
    let sel_bars = p.overlay_bars.instance_count();
    let sel_sel = p.overlay_rows.instance_count();

    // The unselected-bar surfaces collapse; the selected bar survives in both.
    assert!(
        all_bars > sel_bars,
        "SelectedOnly must draw FEWER unselected bars than All (all={all_bars}, selected-only={sel_bars})"
    );
    assert_eq!(all_sel, 1, "All coverage draws the selected bar");
    assert_eq!(sel_sel, 1, "SelectedOnly still draws the selected bar");

    // Real pixels: an UNSELECTED row (the one ABOVE the selected) is a bar under
    // All, bare room under SelectedOnly.
    // Real pixels: an UNSELECTED row (the one ABOVE the selected) carries a
    // WHISPER bar under All (a deliberately quiet value step off the ground —
    // `overlay_bar_unselected`) and bare ROOM under SelectedOnly. The whisper is
    // subtle by design (~a few levels), so this asserts the SHIFT with a redmean
    // threshold rather than the strict distinguish-floor the bolder surfaces use.
    let rect = p.overlay_card_rect().expect("overlay card rect");
    let (card_x, cw) = (rect[0], rect[2]);
    let lh = probe.lh;
    let (wi, hi) = (w as i64, h as i64);
    let unsel_top = probe.band_top - lh; // the row above the selected one
    let x0 = (card_x + cw * 0.3) as i64;
    let y0 = (unsel_top + lh * 0.4) as i64;
    let (rw, rh) = ((cw * 0.3) as i64, (lh * 0.3) as i64);
    let all_row = avg(&all, wi, hi, x0, y0, rw, rh);
    let sel_row = avg(&sel, wi, hi, x0, y0, rw, rh);
    let shift = redmean(all_row, sel_row);
    assert!(
        shift > 5.0,
        "an unselected row must lose its whisper bar under SelectedOnly \
         (redmean all={all_row:?} vs selected-only={sel_row:?} = {shift:.1}, want > 5)"
    );

    set_list_style_test_override(None);
    set_card_anchor_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);
}

// OUTLINE VARIANT — DROPPED in the V7 taste-gate (the rim read as a focus ring,
// not a Persona ledge). The `BarFill` axis and its law are gone; the selection
// pipeline's `stroke` uniform now serves ONLY the `FacetStyle::Chips` ghost pills
// (see `facet_chips_render_a_pill_per_label_and_differ_from_text`).

/// REAL CHIPS (third attempt, MUST render) — `FacetStyle::Chips` draws a rounded
/// pill hugging EACH facet label: the ACTIVE label a FILLED value pill
/// (`overlay_lens_underline`), every INACTIVE label a GHOST hairline-stroke pill
/// (`overlay_facet_ghost`). Proven the way the two prior silent attempts were
/// not: one pill INSTANCE per label (active pill ≥ 1, ghost pills == inactive
/// count) AND a PIXEL DELTA vs the `Text` skin over the strip.
#[test]
fn facet_chips_render_a_pill_per_label_and_differ_from_text() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping facet_chips_render_a_pill_per_label: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    // A faceted picker: All (index 0, never drawn) + three lenses, one active.
    // The drawn labels are File / Edit / View → 3 pills, 1 active + 2 ghost.
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = (0..8).map(|i| format!("Command {i}")).collect();
    v.overlay_selected = 1;
    v.overlay_lens = vec![
        ("All".into(), false),
        ("File".into(), true),
        ("Edit".into(), false),
        ("View".into(), false),
    ];

    let frame = |p: &mut TextPipeline, style: theme::FacetStyle| {
        set_facet_style_test_override(Some(style));
        p.set_view(&v);
        p.prepare(&device, &queue, w, h).unwrap();
        pixeldiff::render_frame(p, &device, &queue, w, h)
    };

    let text = frame(&mut p, theme::FacetStyle::Text);
    let chips = frame(&mut p, theme::FacetStyle::Chips(theme::ChipVariant::Hairline));
    let active_pills = p.overlay_lens_underline.instance_count();
    let ghost_pills = p.overlay_facet_ghost.instance_count();
    let ghost_stroke = p.overlay_facet_ghost.stroke();
    set_facet_style_test_override(None);

    // ONE pill per label: 1 active (filled) + 2 inactive (ghost stroke). This is
    // the assertion the two prior attempts never made — they rendered nothing.
    assert_eq!(active_pills, 1, "Chips draws exactly ONE filled active pill (got {active_pills})");
    assert_eq!(
        ghost_pills, 2,
        "Chips draws ONE ghost pill per INACTIVE drawn facet (File active, Edit+View ghost) — got {ghost_pills}"
    );
    assert!(ghost_stroke > 0.0, "the ghost pills are a hairline STROKE, not a fill (got {ghost_stroke})");

    // PIXEL DELTA: the strip row (display line 1) changes visibly vs Text.
    let rect = p.overlay_card_rect().expect("overlay card rect");
    let (card_x, card_y, cw) = (rect[0], rect[1], rect[2]);
    let text_top = card_y + 12.0;
    let lh = p.overlay_lh();
    let strip = pixeldiff::Region::new(card_x, text_top + lh, cw, lh);
    pixeldiff::assert_perceptibly_different(
        &text, &chips, w as i64, h as i64, strip, pixeldiff::DistinguishFloor::DEFAULT,
        "facet Chips vs Text strip (per-label pills)",
    );

    theme::set_active(theme::DEFAULT_THEME);
}

/// V7 TASTE-GATE — CHIP GAPS: adjacent facet pills must read as DISCRETE pills,
/// not a segmented control. The taste gate measured the pills ABUTTING (a ~3px
/// OVERLAP) because the 2-space strip gap couldn't host each pill's `CHIP_HPAD`
/// plus a readable gap; the fix widened the inter-label separator ONLY under
/// `Chips` (`CHIP_STRIP_GAP`). This law reads the recorded pill rects (active +
/// ghosts), orders them left-to-right, and asserts every adjacent pair leaves a
/// POSITIVE breathing gap (≥ 4px — the 6-8px target with float/rounding margin).
#[test]
fn facet_chips_leave_a_breathing_gap_between_pills() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping facet_chips_leave_a_breathing_gap: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = (0..8).map(|i| format!("Command {i}")).collect();
    v.overlay_selected = 1;
    v.overlay_lens = vec![
        ("All".into(), false),
        ("File".into(), true),
        ("Edit".into(), false),
        ("View".into(), false),
    ];

    set_facet_style_test_override(Some(theme::FacetStyle::Chips(theme::ChipVariant::Hairline)));
    p.set_view(&v);
    p.prepare(&device, &queue, w, h).unwrap();

    // Every drawn pill: the active (filled) mark + each inactive ghost, ordered by x.
    let mut pills: Vec<[f32; 4]> = Vec::new();
    if let Some(a) = p.overlay_theme_underline {
        pills.push(a);
    }
    pills.extend(p.overlay_theme_facet_ghosts.iter().copied());
    set_facet_style_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);

    assert_eq!(pills.len(), 3, "File/Edit/View draw 3 pills (1 active + 2 ghost)");
    pills.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap());
    for pair in pills.windows(2) {
        let gap = pair[1][0] - (pair[0][0] + pair[0][2]);
        assert!(
            gap >= 4.0,
            "adjacent chip pills must leave a breathing gap (≥4px), got {gap:.1} \
             between {:?} and {:?}",
            pair[0],
            pair[1]
        );
    }
}

/// ITEM 46 — the faceted grouped-lens SECTION HEADERS sit on a plate (the wave-2
/// "floating commands" class, header edition). Item 35 plated the bare shortcut
/// chords; the section-header plan lines ([`ThemeLine::Header`], e.g. "FILE") were
/// still the ONE candidate-area line the Bars draw skipped ("a header is a label"),
/// so on a Bars world a header floated BARE over the blurred backdrop while every
/// item row sat on a plate. This is the OUTCOME proof over REAL pixels, swept across
/// EVERY Bars world: the section header's BACKGROUND (its plate interior, sampled in
/// the left inset LEFT of the header glyphs) must MATCH the quiet unselected item-row
/// plate wash — the SAME `overlay_bar_unselected` value — and thus sit a full value
/// step off the bare backdrop the defect left it floating over.
///
/// Non-vacuous: the witness asserts the item-row plate wash is itself a full step off
/// the bare backdrop (redmean ≥ 15 — a real surface), so "header ≈ wash" is a genuine
/// constraint, not trivially satisfiable. Before the fix the header BACKGROUND WAS that
/// bare backdrop, a full step from the wash, so the "header ≈ wash" match would fail.
#[test]
fn faceted_section_header_sits_on_a_plate_on_every_bars_world() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping faceted_section_header_sits_on_a_plate_on_every_bars_world: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    let bars_worlds: Vec<&theme::Theme> = theme::THEMES
        .iter()
        .filter(|t| matches!(t.render_caps.list_style, theme::ListStyle::Bars { .. }))
        .collect();
    assert!(
        !bars_worlds.is_empty(),
        "expected at least one Bars world (Firetail/Galah/Magpie/Mangrove)"
    );

    for th in &bars_worlds {
        theme::set_active_by_name(th.name).unwrap();
        p.sync_theme();

        // A faceted (grouped) palette: three commands under ONE section "File" +
        // a lens strip with File active. `theme_plan` emits a "FILE" header before
        // row 0, so the plan is [Header(0), Item0(1), Item1(2), Item2(3)]. Select
        // row 2 so rows 0/1 are QUIET unselected plates (the wash reference), never
        // the bright selected band.
        let mut v = view("hello world\n", 0, 0);
        v.overlay_active = true;
        v.overlay_items = vec![
            "Switch project".into(),
            "Recent projects".into(),
            "Browse files".into(),
        ];
        v.overlay_sections = vec!["File".into(), "File".into(), "File".into()];
        v.overlay_lens = vec![
            ("All".into(), false),
            ("File".into(), true),
            ("Edit".into(), false),
            ("View".into(), false),
        ];
        v.overlay_selected = 2;
        p.set_view(&v);
        p.prepare(&device, &queue, w, h).unwrap();

        let rect = p
            .overlay_card_rect()
            .expect("the faceted Bars picker must have a card");
        let (card_x, card_y, card_w) = (rect[0], rect[1], rect[2]);
        let text_top = card_y + 12.0; // `theme_overlay_geometry`'s inner vertical pad
        let lh = p.overlay_lh();
        let hg = p.overlay_header_gap();
        let gap = p.overlay_row_gap();
        let bar_off = gap * 0.5;
        let bar_h = (lh - gap).max(1.0);
        // Sample column: inside the plate's left inset (BAR_SIDE_INSET = 8) but LEFT
        // of `text_left` (the Bars hpad) — pure surface, no glyphs.
        let sx = (card_x + 9.0) as i64;
        let (wi, hi) = (w as i64, h as i64);
        let px = pixeldiff::render_frame(&mut p, &device, &queue, w, h);
        let sample = |top: f32| {
            avg(
                &px,
                wi,
                hi,
                sx,
                (top + bar_off + 2.0) as i64,
                2,
                (bar_h - 4.0).max(1.0) as i64,
            )
        };
        // The faceted card carries two header lines (query + strip), so the plan
        // begins on display line 2 — `overlay_row_top` folds that in for plan line k.
        let row_top = |plan_line: usize| chrome::overlay_row_top(text_top, 2, hg, plan_line, lh);
        let header = sample(row_top(0)); // the section-header plate (plan line 0)
        let wash = sample(row_top(1)); // an unselected item-row plate (Item0, plan line 1)
        // A bare backdrop AT THE HEADER'S OWN Y: the header plate hugs only the short
        // "FILE" label on the left, so the card's horizontal MIDDLE at that row is the
        // bare blurred page — exactly the ground the header floated over before the fix.
        let back = avg(
            &px,
            wi,
            hi,
            (card_x + card_w * 0.5) as i64,
            (row_top(0) + bar_off + 2.0) as i64,
            20,
            (bar_h - 4.0).max(1.0) as i64,
        );

        let d_wash_back = redmean(wash, back);
        let d_header_wash = redmean(header, wash);
        assert!(
            d_wash_back >= 15.0,
            "{}: the item-row plate wash {wash:?} must be a real surface step off the bare \
             backdrop {back:?} (redmean {d_wash_back:.1}) — the non-vacuity witness",
            th.name
        );
        assert!(
            d_header_wash < 12.0,
            "{}: the section-header BACKGROUND {header:?} must MATCH the quiet item-row plate \
             wash {wash:?} (redmean {d_header_wash:.1} < 12) — the header is floating BARE over \
             the backdrop, not on a plate",
            th.name
        );
    }
    theme::set_active(theme::DEFAULT_THEME);
}

/// ITEM 46 — the faceted lens-strip TABS sit on a plate (the wave-2 "floating
/// commands" class, strip edition). Item 35 plated the chords; the strip's ACTIVE
/// tab already carried its facet mark (underline / band / bracket / chip), but the
/// INACTIVE tabs — and a bracket/underline active tab, which has no fill — floated
/// BARE, crisp text over the blurred backdrop. Now every drawn tab gets a quiet
/// plate. This is the OUTCOME proof over REAL pixels, swept across EVERY Bars world:
/// EACH tab's plate interior (its glyph-free left pad) must sit a full value step off
/// the bare backdrop (it is a real surface, not floating), and at least the TWO
/// inactive tabs must MATCH the quiet item-row plate wash — the same
/// `overlay_bar_unselected` value the section headers + unselected rows use.
///
/// The tab plate rects come from the shaping owner (`overlay_strip_tab_plates`) for
/// LOCATION only; the appearance is asserted over the PIXELS there — the Wagtail
/// tripwire: a recorded-but-undrawn plate leaves the backdrop showing and fails
/// "step off backdrop". Non-vacuous: before the fix an inactive tab's interior WAS
/// the bare backdrop (redmean ~0 from it), so "step off backdrop" would fail.
#[test]
fn faceted_lens_strip_tabs_sit_on_plates_on_every_bars_world() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping faceted_lens_strip_tabs_sit_on_plates_on_every_bars_world: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    let bars_worlds: Vec<&theme::Theme> = theme::THEMES
        .iter()
        .filter(|t| matches!(t.render_caps.list_style, theme::ListStyle::Bars { .. }))
        .collect();
    assert!(
        !bars_worlds.is_empty(),
        "expected at least one Bars world (Firetail/Galah/Magpie/Mangrove)"
    );

    for th in &bars_worlds {
        theme::set_active_by_name(th.name).unwrap();
        p.sync_theme();

        // A faceted palette: File ACTIVE + Edit/View inactive (TWO inactive tabs are
        // drawn — the `All` home at strip index 0 is never a label), three commands
        // under section "File". Select row 2 so rows 0/1 are quiet plates.
        let mut v = view("hello world\n", 0, 0);
        v.overlay_active = true;
        v.overlay_items = vec![
            "Switch project".into(),
            "Recent projects".into(),
            "Browse files".into(),
        ];
        v.overlay_sections = vec!["File".into(), "File".into(), "File".into()];
        v.overlay_lens = vec![
            ("All".into(), false),
            ("File".into(), true),
            ("Edit".into(), false),
            ("View".into(), false),
        ];
        v.overlay_selected = 2;
        p.set_view(&v);
        p.prepare(&device, &queue, w, h).unwrap();

        let rect = p
            .overlay_card_rect()
            .expect("the faceted Bars picker must have a card");
        let (card_x, card_y, card_w) = (rect[0], rect[1], rect[2]);
        let text_top = card_y + 12.0;
        let lh = p.overlay_lh();
        let hg = p.overlay_header_gap();
        let gap = p.overlay_row_gap();
        let bar_off = gap * 0.5;
        let bar_h = (lh - gap).max(1.0);
        let (wi, hi) = (w as i64, h as i64);
        let sx = (card_x + 9.0) as i64;
        let row_top = |plan_line: usize| chrome::overlay_row_top(text_top, 2, hg, plan_line, lh);

        // The recorded tab plate rects — LOCATION only; the pixels below prove they
        // are truly drawn (the recorded-but-undrawn tripwire).
        let tabs: Vec<[f32; 4]> = p.overlay_strip_tab_plates.clone();

        let px = pixeldiff::render_frame(&mut p, &device, &queue, w, h);
        // The quiet unselected item-row plate wash (Item0, plan line 1).
        let wash = avg(
            &px,
            wi,
            hi,
            sx,
            (row_top(1) + bar_off + 2.0) as i64,
            2,
            (bar_h - 4.0).max(1.0) as i64,
        );
        // A bare backdrop: the card's horizontal MIDDLE at the header row's y — proven
        // bare in `faceted_section_header_sits_on_a_plate_on_every_bars_world` (the
        // header plate hugs only the short "FILE", so the middle is the blurred page).
        let back = avg(
            &px,
            wi,
            hi,
            (card_x + card_w * 0.5) as i64,
            (row_top(0) + bar_off + 2.0) as i64,
            20,
            (bar_h - 4.0).max(1.0) as i64,
        );

        assert_eq!(
            tabs.len(),
            3,
            "{}: File/Edit/View draw 3 tab plates (the All home is not a label), got {}",
            th.name,
            tabs.len()
        );
        // Non-vacuity witness: the quiet wash is itself a real step off the backdrop,
        // so "tab interior steps off the backdrop" is a genuine constraint.
        let d_wash_back = redmean(wash, back);
        assert!(
            d_wash_back >= 15.0,
            "{}: the item-row plate wash {wash:?} must be a real surface step off the bare \
             backdrop {back:?} (redmean {d_wash_back:.1}) — the non-vacuity witness",
            th.name
        );

        let mut matches_wash = 0;
        for (i, t) in tabs.iter().enumerate() {
            let [tx, ty, _tw, thh] = *t;
            // The tab plate's glyph-free LEFT PAD (the CHIP_HPAD before the first
            // glyph), vertical middle — pure surface, clear of the rounded corners.
            let interior = avg(&px, wi, hi, (tx + 3.0) as i64, (ty + thh * 0.5 - 1.0) as i64, 2, 3);
            let d_back = redmean(interior, back);
            assert!(
                d_back >= 15.0,
                "{}: lens-strip tab {i} interior {interior:?} must sit a full step off the bare \
                 backdrop {back:?} (redmean {d_back:.1}) — the tab is floating BARE over the \
                 blurred page, not on a plate",
                th.name
            );
            if redmean(interior, wash) < 12.0 {
                matches_wash += 1;
            }
        }
        // The TWO inactive tabs (Edit, View) ride the quiet item-row plate wash; the
        // active tab may carry a brighter facet fill (Band / FilledActive), so only
        // the inactive pair is required to match — but on a bracket/underline world the
        // active tab is ALSO the quiet plate, so `>= 2` holds everywhere.
        assert!(
            matches_wash >= 2,
            "{}: at least the two INACTIVE tabs must match the quiet item-row plate wash \
             {wash:?} (got {matches_wash} of {} tabs)",
            th.name,
            tabs.len()
        );
    }
    theme::set_active(theme::DEFAULT_THEME);
}
