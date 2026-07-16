//! PER-ITEM LIST SURFACES round — the law suite for the three INERT-by-default
//! capabilities (the "Persona list"): `ListStyle` (Pane | Bars), the
//! RIGHT-ANCHOR MIRROR (`CardAnchor::TopRight`, a first-class anchor value),
//! and `FacetStyle` (Text | Band). Every capability lands byte-identical
//! on every world (proven pixel-for-pixel against the main base in the round's
//! CLI sweep + the inert instance-count law here); the divergent rendering is
//! reachable only through the `AWL_*_FORCE` probes / the test overrides, and is
//! proven to be a PERCEPTIBLE, findable change over real pixels (the Wagtail
//! invisible-row lesson — assert the OUTCOME, not the mechanism).

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

// --- FacetStyle: Band visibly differs from the Text baseline -----------------

/// The `Band` skin (the designer pixel-pass winner over the killed `Chips`)
/// must visibly change the faceted strip vs the `Text` baseline — the active
/// lens gains a value BAND pill, so the strip row reads perceptibly different.
#[test]
fn facet_band_differs_from_text_in_the_strip() {
    let (w, h) = (1200u32, 800u32);
    let Some((device, queue, mut p)) = headless_dqp(w as f32, h as f32) else {
        eprintln!("skipping facet_band_differs: no wgpu adapter");
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
    let band = frame(&mut p, Some(theme::FacetStyle::Band));
    set_facet_style_test_override(None);

    // The strip row (display line 1) must visibly change under the Band skin.
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
