//! WRAP-AFFINITY — the caret parked at a SHARED soft-wrap boundary renders on the
//! visual row it VISUALLY belongs to.
//!
//! A SHARED boundary is a soft wrap with NO dropped whitespace: mid-word, a long
//! URL, and EVERY CJK wrap (kana/han carry no spaces). There the upper row's
//! `end_col` EQUALS the lower row's `start_col`, so ONE char index is a legit caret
//! position on BOTH rows — the upper row's trailing (right) edge and the lower
//! row's leading (left) edge. The caret's [`Affinity`] disambiguates:
//!   * `Upstream`  (a visual line-END motion: C-e / End / Cmd-Right) → the UPPER row.
//!   * `Downstream`(rightward / Down / a fresh cursor)               → the LOWER row.
//!
//! Born from the CJK C-e bug (2026-07-18): C-e's caret rendered a full row too LOW
//! at every space-less wrap (the lower-row `pick_row` bias won the boundary column),
//! while a dropped-space prose wrap masked it. The two laws below assert the OUTCOME
//! over REAL PIXELS (the caret's own `primary` colour, DESIGN §3) AND the end-to-end
//! motion wiring (`apply_core(LineEnd)` sets `Upstream`), on a CJK fixture AND a
//! space-less Latin URL. C-e / End / Cmd-Right all resolve to `Action::LineEnd`
//! (pinned by the keymap tests `c_e_kept` / the Cmd-arrow + Home/End arms), so
//! driving `LineEnd` covers all three advertised chords.

use super::super::*;
use super::{headless_dqp, view};
use crate::buffer::Buffer;
use crate::caret::Affinity;

/// 36 full-width kana, one logical line, NO spaces — wraps every ~8 columns at
/// measure 20 on the 1200px canvas, and every wrap is SHARED (no dropped space).
const CJK: &str = "あいうえおかきくけことさしすせそたちつてとなにぬねのはひふへほまみむめも";
/// A long path-only URL, NO spaces — mid-token wraps are SHARED too (the bug was
/// never CJK-specific; any space-less wrap trips it).
const URL: &str = "https://example.com/verylongpath/that/keeps/going/and/going/segmentxyz/more";

/// The narrow page measure that forces the fixtures to wrap on the test canvas.
const NARROW: usize = 20;

/// A single-line [`ViewState`] at `col` with an explicit caret [`Affinity`].
fn view_aff(text: &str, col: usize, aff: Affinity) -> ViewState {
    let mut v = view(text, 0, col);
    v.caret_affinity = aff;
    v
}

/// The y-centroid (px) of the caret's `primary`-coloured pixels in a rendered
/// frame — the caret is the ONE `primary` element (DESIGN §3), so an amber-tolerant
/// colour match isolates it. `None` when too few match (no caret found).
fn caret_row_y(frame: &[[u8; 4]], w: u32, h: u32) -> Option<f32> {
    let [pr, pg, pb] = theme::primary().rgb_bytes();
    let (mut sy, mut n) = (0f64, 0f64);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let [r, g, b, _] = frame[y * w as usize + x];
            let d = (r as i32 - pr as i32).abs()
                + (g as i32 - pg as i32).abs()
                + (b as i32 - pb as i32).abs();
            if d < 60 {
                sy += y as f64;
                n += 1.0;
            }
        }
    }
    (n >= 20.0).then_some((sy / n) as f32)
}

/// The shared-boundary column of the FIRST wrap of `text`, plus a proof it IS
/// shared (the next row's `start_col` equals it — no dropped space). Panics if the
/// fixture does not wrap into a shared boundary (a fixture-validity guard).
fn shared_boundary(p: &mut TextPipeline, text: &str) -> usize {
    p.set_view(&view(text, 0, 0));
    let rows = p.visual_rows(0);
    assert!(rows.len() >= 2, "fixture must wrap into >=2 visual rows, got {}", rows.len());
    let b = rows[0].end_col;
    assert_eq!(
        rows[1].start_col, b,
        "fixture must wrap with NO dropped space (shared boundary): row0.end_col={} row1.start_col={}",
        b, rows[1].start_col
    );
    b
}

#[test]
fn wrap_end_caret_renders_on_the_upper_visual_row_real_pixels() {
    // Soft-wrap geometry folds the page-width globals + the active theme; hold the
    // process lock so no parallel writer re-wraps or re-tints mid-render.
    let _g = crate::testlock::serial();
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!("skipping wrap_end_caret_renders_on_the_upper_visual_row_real_pixels: no wgpu adapter");
        return;
    };
    let (w, h) = (1200u32, 800u32);
    let old_measure = crate::page::measure();
    let old_page_on = crate::page::page_on();
    let old_theme = theme::active().name;
    let old_caret_auto = crate::caret::is_auto();
    let old_caret_mode = crate::caret::mode();
    // Pin the BLOCK caret: Morph/I-beam anchor the caret one glyph BACK / at the
    // insertion point, which at a boundary would land the Downstream caret on the
    // UPPER row too — the block sits squarely on the caret's own row, so its pixel
    // row is the clean oracle. (A parallel test may have set a non-block override.)
    crate::caret::set_mode(crate::caret::CaretMode::Block);
    // Pin a world whose `primary` is a saturated amber on a dark ground, so the
    // caret is the ONLY amber blob (a one-bit / ink-caret world would tie caret to ink).
    theme::set_active_by_name("Firetail");
    p.sync_theme(); // push Firetail's palette into the pipeline so the caret draws amber
    // PAGE MODE ON + a NARROW measure is what forces the fixtures to wrap (a parallel
    // test may have left page mode off / the measure wide). `set_size` then recomputes
    // the wrap width from these (set_view alone only re-wraps on a zoom change).
    crate::page::set_page_on(true);
    crate::page::set_measure(NARROW);
    p.set_size(1200.0, 800.0);

    for (label, text) in [("CJK", CJK), ("URL", URL)] {
        let boundary = shared_boundary(&mut p, text);
        let bchar = boundary; // single logical line, col == char index

        // Render the three arrivals at the boundary column, gathering the caret's
        // pixel ROW before any assert (so a restore always runs).
        let render = |p: &mut TextPipeline, v: &ViewState| -> Option<f32> {
            p.set_view(v);
            p.settle_caret(); // snap the spring to its target (the capture's settled state)
            p.prepare(&device, &queue, w, h).unwrap();
            let frame = super::pixeldiff::render_frame(p, &device, &queue, w, h);
            caret_row_y(&frame, w, h)
        };
        let y_row0 = render(&mut p, &view(text, 0, 0)); // col 0: unambiguously row 0
        let y_down = render(&mut p, &view_aff(text, bchar, Affinity::Downstream));
        let y_up = render(&mut p, &view_aff(text, bchar, Affinity::Upstream));
        // Selection ending AT the boundary with an Upstream caret (the S-C-e state):
        // the caret must stay on row 0 (the phase-1 bug detached it to row 1).
        let mut sel = view_aff(text, bchar, Affinity::Upstream);
        sel.selection = Some(((0, 0), (0, bchar)));
        let y_sel = render(&mut p, &sel);

        // Restore shared globals BEFORE asserting so a failure never leaks state.
        // (Deferred to after the loop's last iteration would skip on an early panic.)
        let restore = || {
            crate::page::set_measure(old_measure);
            crate::page::set_page_on(old_page_on);
            theme::set_active_by_name(old_theme);
            if old_caret_auto {
                crate::caret::clear_override();
            } else {
                crate::caret::set_mode(old_caret_mode);
            }
        };

        let (y_row0, y_down, y_up, y_sel) = match (y_row0, y_down, y_up, y_sel) {
            (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
            _ => {
                restore();
                panic!("{label}: caret pixels not found in one of the arrivals");
            }
        };

        let lh = LINE_HEIGHT;
        // Downstream sits one visual row BELOW row 0 (the boundary's lower row).
        let row_gap = y_down - y_row0;
        // Upstream renders on row 0 (near the col-0 caret), NOT on the lower row.
        let up_to_row0 = (y_up - y_row0).abs();
        let up_to_row1 = (y_up - y_down).abs();
        let sel_to_row0 = (y_sel - y_row0).abs();

        if boundary == 0
            || !(0.5 * lh..1.5 * lh).contains(&row_gap)
            || up_to_row0 >= 0.5 * lh
            || up_to_row1 <= 0.5 * lh
            || sel_to_row0 >= 0.5 * lh
        {
            restore();
            panic!(
                "{label}: wrap-affinity caret row wrong — boundary={boundary} \
                 row0_y={y_row0:.1} down_y={y_down:.1} up_y={y_up:.1} sel_y={y_sel:.1} \
                 (row_gap={row_gap:.1} up→row0={up_to_row0:.1} up→row1={up_to_row1:.1} \
                 sel→row0={sel_to_row0:.1}; line_height={lh})"
            );
        }
    }

    crate::page::set_measure(old_measure);
    crate::page::set_page_on(old_page_on);
    theme::set_active_by_name(old_theme);
    if old_caret_auto {
        crate::caret::clear_override();
    } else {
        crate::caret::set_mode(old_caret_mode);
    }
}

#[test]
fn visual_line_end_motion_sets_upstream_affinity_end_to_end() {
    // The MOTION half of the law: driving `Action::LineEnd` (the shared funnel of
    // C-e / End / Cmd-Right) through the REAL `apply_core` with the shaped pipeline
    // as the visual-line ORACLE lands the caret at the shared boundary AND stamps
    // `Upstream`; a plain rightward motion to the same column leaves `Downstream`.
    let _g = crate::testlock::serial();
    let Some((_device, _queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!("skipping visual_line_end_motion_sets_upstream_affinity_end_to_end: no wgpu adapter");
        return;
    };
    let old_measure = crate::page::measure();
    let old_page_on = crate::page::page_on();
    // PAGE MODE ON + NARROW measure forces the wrap; `set_size` re-reads them into
    // the wrap width (set_view alone only re-wraps on a zoom change).
    crate::page::set_page_on(true);
    crate::page::set_measure(NARROW);
    p.set_size(1200.0, 800.0);

    for (label, text) in [("CJK", CJK), ("URL", URL)] {
        let boundary = shared_boundary(&mut p, text);
        // Shape the oracle pipeline on THIS fixture (its internal buffer answers the
        // wrap queries `apply_core` asks).
        p.set_view(&view(text, 0, 0));

        let drive = |p: &TextPipeline, actions: &[crate::keymap::Action]| -> Buffer {
            let mut buffer = Buffer::from_str(text);
            let mut shift_selecting = false;
            let mut zoom = 1.0;
            let mut search = None;
            let mut overlay = None;
            let mut make_overlay =
                |_k: crate::overlay::OverlayKind| -> Option<crate::overlay::OverlayState> { None };
            let mut browse_to = |_k: crate::overlay::OverlayKind,
                                 _r: Option<String>|
             -> Option<crate::overlay::OverlayState> { None };
            for a in actions {
                let mut ctx = crate::actions::ActionCtx {
                    buffer: &mut buffer,
                    shift_selecting: &mut shift_selecting,
                    zoom: &mut zoom,
                    search: &mut search,
                    scroll_page_lines: 1,
                    overlay: &mut overlay,
                    make_overlay: &mut make_overlay,
                    browse_to: &mut browse_to,
                    oracle: Some(p as &dyn crate::actions::LayoutOracle),
                };
                crate::actions::apply_core(&mut ctx, a, false);
            }
            buffer
        };

        // C-e / End / Cmd-Right → LineEnd: caret lands at the boundary, Upstream.
        let ended = drive(&p, &[crate::keymap::Action::LineEnd]);
        assert_eq!(
            ended.cursor_line_col(),
            (0, boundary),
            "{label}: LineEnd stops at the current visual row's end (the shared boundary)"
        );
        assert_eq!(
            ended.affinity(),
            Affinity::Upstream,
            "{label}: LineEnd parks the caret Upstream (upper row's trailing edge)"
        );

        // A second LineEnd from the Upstream caret is a NO-OP (idempotent), not a
        // jump to the lower row's end — the oracle reads the row the caret sits on.
        let ended2 = drive(&p, &[crate::keymap::Action::LineEnd, crate::keymap::Action::LineEnd]);
        assert_eq!(
            ended2.cursor_line_col(),
            (0, boundary),
            "{label}: a repeat LineEnd on the Upstream caret stays put"
        );
        assert_eq!(ended2.affinity(), Affinity::Upstream, "{label}: repeat LineEnd keeps Upstream");

        // A rightward motion up to the boundary column leaves Downstream (the lower
        // row's leading edge) — the SAME column, the OTHER legit render.
        let stepped = drive(&p, &vec![crate::keymap::Action::ForwardChar; boundary]);
        assert_eq!(
            stepped.cursor_line_col(),
            (0, boundary),
            "{label}: {boundary}× ForwardChar reaches the boundary column"
        );
        assert_eq!(
            stepped.affinity(),
            Affinity::Downstream,
            "{label}: rightward motion leaves the caret Downstream (lower row)"
        );

        // LineStart from the Upstream caret goes to the UPPER row's start (col 0),
        // not a no-op at the lower row's start — coherent with the render.
        let homed = drive(&p, &[crate::keymap::Action::LineEnd, crate::keymap::Action::LineStart]);
        assert_eq!(
            homed.cursor_line_col(),
            (0, 0),
            "{label}: LineStart after LineEnd returns to the visual row's start (col 0)"
        );
    }

    crate::page::set_measure(old_measure);
    crate::page::set_page_on(old_page_on);
}
