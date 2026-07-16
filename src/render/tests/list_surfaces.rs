//! PER-ITEM LIST SURFACES round — the law suite for the three INERT-by-default
//! capabilities (the "Persona list"): `ListStyle` (Pane | Bars), the
//! RIGHT-ANCHOR MIRROR (`CardAnchor::TopRight`, a first-class anchor value),
//! and `FacetStyle` (Text | Band). Each capability's DEFAULT arm is inert: the
//! divergent rendering is reachable only through the `AWL_*_FORCE` probes / the
//! test overrides, and is proven to be a PERCEPTIBLE, findable change over real
//! pixels (the Wagtail invisible-row lesson — assert the OUTCOME, not the
//! mechanism).
//!
//! THE INERT GUARANTEE — re-scoped (2026-07-16). The gate is NO LONGER
//! "byte-identical to the `main` base": this refit round ships ONE deliberate
//! visual change — the QUERY-INPUT BEAT widened from `0.72` to `1.0` of a row
//! (`OVERLAY_QUERY_BEAT`, a user-directed taste dial), so EVERY summoned
//! picker's query line and everything below it moves down a fraction vs `main`
//! by design. Byte-identity-vs-`main` is therefore impossible for any
//! query-line surface and must not be claimed. What the inert guarantee DOES
//! assert, two ways:
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

// --- grammar (pure) ----------------------------------------------------------

#[test]
fn parse_list_style_force_grammar() {
    assert_eq!(parse_list_style_force("pane"), Some(theme::ListStyle::Pane));
    // Bare `bars` → the default treatment (a real Bars value).
    assert!(matches!(parse_list_style_force("bars"), Some(theme::ListStyle::Bars { .. })));
    // Parametric radius:gap:grow.
    assert_eq!(
        parse_list_style_force("bars:0:6:10"),
        Some(theme::ListStyle::Bars { radius: 0.0, gap: 6.0, grow_px: 10.0 })
    );
    assert_eq!(
        parse_list_style_force("bars:14.5:8:12"),
        Some(theme::ListStyle::Bars { radius: 14.5, gap: 8.0, grow_px: 12.0 })
    );
    // Malformed / negative / wrong arity → None (the world's own data).
    assert_eq!(parse_list_style_force("bars:1:2"), None);
    assert_eq!(parse_list_style_force("bars:-1:2:3"), None);
    assert_eq!(parse_list_style_force("capsule"), None);
    assert_eq!(parse_list_style_force(""), None);
}

#[test]
fn parse_facet_style_force_grammar() {
    assert_eq!(parse_facet_style_force("text"), Some(theme::FacetStyle::Text));
    assert_eq!(parse_facet_style_force("BAND"), Some(theme::FacetStyle::Band));
    // The `Chips` skin was killed in the designer pixel-pass — its grammar word
    // now parses to None (falls back to the world's own facet style).
    assert_eq!(parse_facet_style_force("chips"), None);
    assert_eq!(parse_facet_style_force("pill"), None);
    assert_eq!(parse_facet_style_force(""), None);
}

/// The force-knob classifier must tell UNSET (silent world default) apart from
/// SET-BUT-RETIRED (the killed `chips` word) — the reader turns the latter LOUD.
/// This is the guard against the facet-chips GALLERY TRAP: a re-shoot forcing a
/// retired variant silently produced a byte-identical duplicate of `text`.
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
    // The KILLED `chips` skin, or any typo, but SET → Retired (loud fallback):
    // never a silent duplicate of the default masquerading under a `-chips` name.
    assert!(matches!(
        classify_forced_knob(Some("chips"), parse_facet_style_force),
        ForcedKnob::Retired
    ));
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

// --- Bars DROP THE PANE; Pane KEEPS it (the card-fill law, gated by style) ----

/// THE PANE-DROP LAW (the user's refit: "with the bars, there shouldn't be a
/// pane!"). Under `ListStyle::Bars` the boxed pane's ELEVATION disappears — the
/// `panel_shadow` and `panel_border` companions draw ZERO instances, so the bars
/// never sit in a raised box. In place of the boxed fill, `panel_card` draws a
/// single FULL-CANVAS ROOM VEIL (a value scrim of the ground, no elevation — the
/// Persona room, added in the designer pixel-pass to kill the crisp-doc comb
/// seam), so it keeps its one instance but is a room, not a card. Under `Pane`
/// (the default every world ships) the card fill stays (one instance), the pane
/// the whole picker family has always drawn. The `card_rect` still governs
/// LAYOUT in both (anchor/width/hit-tests via `overlay_geometry`) — only the
/// PAINT is gated. Without this law a future "always draw the elevated card"
/// regression would silently restore the boxed pane the user rejected.
#[test]
fn bars_drop_the_pane_pane_keeps_it() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!("skipping bars_drop_the_pane_pane_keeps_it: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_card_anchor_test_override(Some(theme::CardAnchor::TopLeft));

    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = (0..8).map(|i| format!("Command {i}")).collect();
    v.overlay_selected = 2;

    // PANE (default): the card fill draws its one instance; no bars.
    set_list_style_test_override(Some(theme::ListStyle::Pane));
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert_eq!(p.panel_card.instance_count(), 1, "Pane draws the card fill");
    assert_eq!(p.overlay_bars.instance_count(), 0, "Pane draws no bars");

    // BARS: the boxed pane vanishes — shadow + border park empty (no elevation) —
    // and a bar draws per unselected row. `panel_card` now paints ONE full-canvas
    // room veil in place of the boxed fill (not a raised card).
    set_list_style_test_override(Some(theme::ListStyle::Bars {
        radius: 6.0,
        gap: 10.0,
        grow_px: 24.0,
    }));
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert_eq!(p.panel_card.instance_count(), 1, "Bars paint one full-canvas room veil");
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
    // lays its OPAQUE base_100 room plane behind the bars (designer pixel-pass),
    // so the between-bars "ground" is now the paper; the whisper is exactly the
    // base_100 → base_200 step, which reads on Saltpan (a flat-ramp world would
    // make it vanish by its palette — not this law's concern).
    theme::set_active_by_name("Saltpan").unwrap();

    // A flat (non-faceted) picker, selection on row 2, plenty of rows.
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = (0..8).map(|i| format!("Command number {i}")).collect();
    v.overlay_selected = 2;

    // Sharp bars (radius 0 → a clean left edge to sample), a real gap, a real grow.
    set_list_style_test_override(Some(theme::ListStyle::Bars {
        radius: 0.0,
        gap: 8.0,
        grow_px: 10.0,
    }));
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
    // `bars_drop_the_pane_pane_keeps_it`) and replaced by the OPAQUE base_100 ROOM
    // PLANE — so the between-bars region is that paper, the ground the unselected
    // whisper (base_200) lifts off of. `overlay_card_rect` still returns the
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
    // The gap between row 0 and row 1 shows the ROOM PLANE (base_100 paper — no
    // pane). Bar 0's bottom is `row_top(0) + bar_off + bar_h`; the gap runs from
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
        d_bar >= 10.0,
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
    // Kingfisher: an amber-accent, coloured-ground world where the bug was shot.
    theme::set_active_by_name("Kingfisher").unwrap();
    p.sync_theme();
    crate::render::set_list_style_test_override(Some(theme::ListStyle::Bars {
        radius: 6.0,
        gap: 8.0,
        grow_px: 24.0,
    }));
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

    crate::render::set_list_style_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);
}

/// FIRST-SCANLINE LAW (real pixels): the `Bars` full-canvas ROOM plane is drawn
/// through the panel quad pipeline, which feathers a ~1px antialiased edge. Sized
/// flush to `[0, 0, w, h]` it left the FIRST pixel row only ~84% covered — a 1px
/// LIGHTER seam along y = 0 (the designer's first-scanline nit). The room now
/// bleeds past every canvas edge, so row 0 must be BYTE-IDENTICAL to the interior
/// room ground.
#[test]
fn bars_room_plane_covers_the_first_scanline() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping bars_room_plane_covers_the_first_scanline: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    theme::set_active_by_name("Kingfisher").unwrap();
    p.sync_theme();
    crate::render::set_list_style_test_override(Some(theme::ListStyle::Bars {
        radius: 6.0,
        gap: 8.0,
        grow_px: 24.0,
    }));
    let mut v = view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_title = "themes";
    v.overlay_items = (0..8).map(|i| format!("Command {i}")).collect();
    v.overlay_selected = 1;
    p.set_view(&v);
    p.prepare(&device, &queue, w, h).unwrap();
    let px = pixeldiff::render_frame(&mut p, &device, &queue, w, h);

    // Sample a stretch of empty room (to the RIGHT of the bars, above/away from any
    // glyphs): row 0 must match row 6 exactly, channel for channel.
    let idx = |x: i64, y: i64| px[(y * w as i64 + x) as usize];
    let interior = idx(800, 6);
    let mut worst = 0i64;
    for x in (700..1100).step_by(7) {
        let top = idx(x, 0);
        for c in 0..4 {
            worst = worst.max((top[c] as i64 - interior[c] as i64).abs());
        }
    }
    assert!(
        worst <= 1,
        "first-scanline nit: row 0 differs from the interior room ground by {worst} \
         (the room plane must bleed past y=0, leaving no lighter seam)"
    );

    crate::render::set_list_style_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);
}

// --- FacetStyle: Band visibly differs from the Text baseline -----------------

/// THE FACET-ARM-DRAWS LAW (instance-count + pixel delta). Born from the
/// facet-chips GALLERY TRAP (fixed @ e56d689): the retired `Chips` skin parsed
/// to `None` and SILENTLY rendered the `Text` default, so `kingfisher-facet-
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
/// proves, for EVERY kind, that Bars drops the boxed pane:
///   - the contextual SPELL popup is the ONE kind that legitimately stays a
///     RAISED float panel (a popup over the doc, not a centered list card); it
///     draws NO list-card fill (`panel_card == 0`) — its elevation rides the
///     separate `float_*` pipelines.
///   - EVERY other kind: ZERO card BORDER + ZERO card SHADOW (no boxed
///     elevation), exactly ONE full-canvas ROOM VEIL (`panel_card == 1`, not a
///     boxed card), a bar per row, and the selected bar drawn — so its `grow_px`
///     jut has NO card wall to clip against (the board bug). Faceting kinds are
///     driven through the `geom.theme` card path too (an active lens strip),
///     since the board bug lived on the FACETED card, not the flat one.
/// A new `OverlayKind` fails to compile here until it declares which regime it
/// is — the structural guard against a future per-kind card special case.
#[test]
fn bars_drop_the_pane_for_every_overlay_kind() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping bars_drop_the_pane_for_every_overlay_kind: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    set_card_anchor_test_override(Some(theme::CardAnchor::TopLeft));
    set_list_style_test_override(Some(theme::ListStyle::Bars {
        radius: 6.0,
        gap: 10.0,
        grow_px: 24.0,
    }));

    use crate::overlay::OverlayKind;
    for kind in OverlayKind::ALL {
        // NO-WILDCARD: Spell is the sole contextual FLOAT panel; every other kind
        // is a centered list card that must drop its boxed pane under Bars.
        let is_float = match kind {
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
            | OverlayKind::MoveDest
            | OverlayKind::Keybindings
            | OverlayKind::Assets
            | OverlayKind::Rename
            | OverlayKind::InsertLink => false,
        };

        let mut v = view("the quick brown fox jumps\n", 0, 0);
        v.overlay_active = true;
        v.overlay_items = (0..6).map(|i| format!("Item {i}")).collect();
        v.overlay_selected = 2;
        v.overlay_hint = "hint".into();
        if is_float {
            // the contextual spell popup, anchored at the word "quick" (cols 4..9)
            v.overlay_spell = Some((0, 4, 9));
        } else if crate::facets::scheme(kind).is_some() {
            // exercise the FACETED (`geom.theme`) card path too — the board bug
            // lived on the faceted card. An active lens gives the strip a target.
            v.overlay_lens = vec![("All".into(), true), ("File".into(), false)];
        }
        p.set_view(&v);
        p.prepare(&device, &queue, w, h).unwrap();

        if is_float {
            // Spell: a raised float panel, NOT a list card — its own elevation is
            // correct. The list-card fill stays empty for it.
            assert_eq!(
                p.panel_card.instance_count(),
                0,
                "{kind:?}: the spell float popup draws no list-card fill"
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
        assert_eq!(
            p.panel_card.instance_count(),
            1,
            "{kind:?}: Bars paints exactly ONE full-canvas room veil, not a boxed card"
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
    set_list_style_test_override(Some(theme::ListStyle::Bars {
        radius: 6.0,
        gap: 10.0,
        grow_px: 24.0,
    }));

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
