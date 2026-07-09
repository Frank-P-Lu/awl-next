//! BLOCK-caret sizing/ink-box tests (glyph-advance width, wrap-boundary full
//! cell, kerned-glyph alignment, mono+ligature clusters, and the morph
//! line-start trail anchor) -- split out of the former monolithic
//! `render::tests` (2026-07 code-organization pass). See `caret` for the
//! rest of the caret geometry/trail/morph suite.

use super::super::*;
use super::{headless_pipeline, view};

/// The BLOCK caret quad's resting WIDTH tracks the REAL shaped glyph advance at
/// the cursor: on a PROPORTIONAL world it is wide on `m` and narrow on `i`
/// (exactly the glyph's advance, no fixed-cell floor); on a MONO world it is the
/// constant cell and byte-identical to the old `caret_target_w`.
#[test]
fn block_caret_width_tracks_glyph_advance() {
    // Advance reads fold the theme font AND the page wrap globals; hold both
    // (theme → page order, page.rs:95-99). The block width is read at the
    // mode-keyed ANCHOR cell (Morph shifts one back, and the no-override
    // default follows the active font — proportional Gumtree would latch
    // Morph); hold the caret lock and pin BLOCK, the look under test.
    let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _g = crate::page::test_lock();
    let _c = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::caret::set_mode(CaretMode::Block);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping block_caret_width_tracks_glyph_advance: no wgpu adapter");
        return;
    };
    let text = "milk"; // col 0 = 'm' (wide), col 1 = 'i' (narrow)

    // PROPORTIONAL (Gumtree = Literata): the block width is the REAL glyph
    // advance, so the wide 'm' yields a wider block than the narrow 'i' and the
    // narrow glyph drops BELOW the fixed cell — the old `.max(caret_w)` floor,
    // which pinned every cell to caret_w, is gone on proportional faces.
    theme::set_active_by_name("Gumtree").unwrap();
    p.sync_theme();
    p.set_view(&view(text, 0, 0)); // on 'm'
    let w_m = p.caret_block_w();
    let (_x, adv_m) = p.col_x_and_advance(0, 0);
    p.set_view(&view(text, 0, 1)); // on 'i'
    let w_i = p.caret_block_w();
    let (_x, adv_i) = p.col_x_and_advance(0, 1);
    assert!(
        w_m > w_i + 1.0,
        "proportional block must be wider on 'm' than 'i' (m={w_m}, i={w_i})"
    );
    // The block is EXACTLY the real glyph advance (no floor) on each glyph.
    assert!((w_m - adv_m).abs() < 1e-3, "block 'm' == real advance ({w_m} vs {adv_m})");
    assert!((w_i - adv_i).abs() < 1e-3, "block 'i' == real advance ({w_i} vs {adv_i})");
    // ...and the narrow glyph is thinner than the old fixed cell — proof the
    // floor that made the block too wide on thin glyphs is gone.
    assert!(
        w_i < p.metrics.caret_w,
        "narrow 'i' block must be thinner than the fixed cell (i={w_i}, cell={})",
        p.metrics.caret_w
    );

    // MONO (Tawny = IBM Plex Mono): the historical `.max(caret_w)` floor is kept,
    // so the BLOCK width is byte-identical to the old `caret_target_w` at every
    // column — the mono block is unchanged. (Keyed on the EFFECTIVE shaped
    // family — the declared doc family, not the resolved face — so this holds
    // even where the mono face isn't installed and shaping falls back: Tawny
    // still renders exactly as it did before.)
    theme::set_active_by_name("Tawny").unwrap();
    p.sync_theme();
    for col in 0..text.chars().count() {
        p.set_view(&view(text, 0, col));
        assert!(
            (p.caret_block_w() - p.caret_target_w()).abs() < 1e-6,
            "mono block must equal the old caret_target_w at col {col} (unchanged)"
        );
        // On a glyph at/above the cell the floor is a no-op (block == advance);
        // a narrow glyph is floored UP to the fixed cell — exactly the old block.
        assert!(
            p.caret_block_w() >= p.metrics.caret_w - 1e-3,
            "mono block never drops below the fixed cell at col {col}"
        );
    }

    // Restore the default world so other tests see a clean global.
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// REGRESSION (the wrap-boundary SLIVER): the SPACE where a long line
/// soft-wraps gets NO visible glyph in its row — cosmic-text collapses the
/// trailing whitespace at the break, so its two x boundaries coincide at the
/// row's right edge and the raw cell width is ~0. The block caret drawn from
/// that advance rendered as a ~1px sliver (reported on Mangrove = JetBrains
/// Mono). `col_x_and_advance` must rescue such a DEGENERATE cell to the
/// default cell width, so the block caret keeps a full visible cell there —
/// on mono AND proportional worlds alike — while genuinely narrow glyphs
/// (`i`, `l`) keep their real advance (no too-wide floor reintroduced; see
/// `block_caret_width_tracks_glyph_advance`).
#[test]
fn block_caret_full_cell_on_wrap_boundary_space() {
    // The wrap boundary IS the fixture: it folds the theme font AND the page
    // wrap globals, so hold both (theme → page order, page.rs:95-99). The block
    // width is read at the mode-keyed ANCHOR cell and the no-override default
    // follows the active font (proportional Gumtree would latch Morph); hold
    // the caret lock and pin BLOCK, the look under test.
    let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _g = crate::page::test_lock();
    let _c = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::caret::set_mode(CaretMode::Block);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping block_caret_full_cell_on_wrap_boundary_space: no wgpu adapter");
        return;
    };
    let long = "word ".repeat(80); // 400 chars, wraps on the 1200px canvas

    // A world for each shaping family: the reported mono world (Mangrove =
    // JetBrains Mono) and a proportional world (Gumtree = Literata) — the
    // degenerate-cell rescue must hold on both.
    for world in ["Mangrove", "Gumtree"] {
        theme::set_active_by_name(world).unwrap();
        p.sync_theme();
        p.set_view(&view(&long, 0, 0));
        let rows = p.visual_rows(0);
        assert!(rows.len() >= 2, "{world}: long line should wrap ({} rows)", rows.len());
        // The wrap-boundary SPACE: the char just before the second row's
        // start. It belongs to the FIRST row (pick_row's half-open span), at
        // the row's right edge, where its collapsed cell is the degenerate one.
        let space_col = rows[1].start_col - 1;
        assert_eq!(
            long.chars().nth(space_col),
            Some(' '),
            "{world}: the wrap boundary lands on the collapsed space"
        );
        // Prove the setup reproduces the degenerate cell: the RAW x delta of
        // the collapsed space is a sliver, far below a real glyph advance.
        let row = &rows[0];
        let raw = row.xs[space_col + 1] - row.xs[space_col];
        assert!(
            raw < p.metrics.char_width * 0.2,
            "{world}: wrap-boundary space cell should be collapsed (raw={raw})"
        );
        // The rescued advance is a full default cell...
        let (_x, adv) = p.col_x_and_advance(0, space_col);
        assert!(
            (adv - p.metrics.char_width).abs() < 1e-3,
            "{world}: degenerate cell advance rescued to char_width (adv={adv})"
        );
        // ...and the BLOCK caret quad drawn there is a visible full cell, not
        // the ~1px sliver.
        p.set_view(&view(&long, 0, space_col));
        let w = p.caret_block_w();
        assert!(
            w >= p.metrics.char_width * 0.5,
            "{world}: block caret at the wrap-boundary space must be a visible \
             cell, not a sliver (w={w}, cell={})",
            p.metrics.char_width
        );
    }

    // Restore the default world so other tests see a clean global.
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}

/// THE KERNED-GLYPH CARET FIX: on a PROPORTIONAL world, when the caret's
/// anchor column maps ONE-TO-ONE onto a single shaped glyph, the BLOCK
/// caret's settled rest quad must sit EXACTLY on that glyph's own swash ink
/// box (what MORPH already recolours) — never the naive advance CELL.
/// Reproduces the reported bug: on "awl" (Mopoke = iA Writer Quattro S) the
/// middle 'w' has a nonzero ink left-bearing AND an ink width narrower than
/// its advance cell, so the OLD cell-only block quad sat visibly offset +
/// narrow against the real glyph while Morph (which already samples the
/// glyph) did not.
#[test]
fn block_caret_ink_aligns_on_kerned_glyph() {
    // Ink-box lookup rides the theme font AND the page wrap globals; the
    // anchor is mode-keyed. Hold theme -> page -> caret (the suite-wide
    // order), pin BLOCK, restore both globals after.
    let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _g = crate::page::test_lock();
    let _c = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::caret::set_mode(CaretMode::Block);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping block_caret_ink_aligns_on_kerned_glyph: no wgpu adapter");
        return;
    };
    theme::set_active_by_name("Mopoke").unwrap(); // proportional (iA Writer Quattro S)
    p.sync_theme();
    let text = "awl"; // col 1 = 'w', kerned between 'a' and 'l'
    p.set_view(&view(text, 0, 1));
    p.settle_caret();

    // The naive CELL box (the pre-fix geometry): the advance-derived width.
    let (_cell_x, cell_adv) = p.col_x_and_advance(0, 1);

    // The glyph's real ink box — the SAME swash lookup MORPH's silhouette reads.
    let (ink_left, ink_w) = p
        .caret_anchor_ink_box()
        .expect("a single 'w' glyph on a proportional world must yield an ink box");

    // Fixture sanity: this glyph's ink really DOES diverge from its advance
    // cell (a nonzero left bearing and/or a width mismatch) — otherwise this
    // test would pass even with the old, unfixed cell-only geometry.
    assert!(
        ink_left.abs() > 0.5 || (ink_w - cell_adv).abs() > 0.5,
        "fixture must reproduce a real cell/ink divergence: left={ink_left} ink_w={ink_w} cell_adv={cell_adv}"
    );

    // The settled BLOCK quad must sit EXACTLY on the glyph's ink box, not
    // the naive cell.
    let pen_x = p.caret.pos.x;
    let (cx, _cy, w, _h, _corner, _ax, _ay) = p.caret_geometry();
    let got_left = cx - w * 0.5;
    assert!(
        (got_left - (pen_x + ink_left)).abs() < 1e-2,
        "block left edge must equal the glyph's ink left: got {got_left} want {}",
        pen_x + ink_left
    );
    assert!(
        (w - ink_w).abs() < 1e-2,
        "block width must equal the glyph's ink width: got {w} want {ink_w}"
    );

    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
    crate::caret::set_mode(CaretMode::Block);
}

/// The ink-box override is DELIBERATELY scoped OFF in two cases, both
/// asserted here:
///   * a MONO world (the default, Tawny): a monospace display wants a
///     perfectly uniform caret grid, not per-glyph ink wobble, so
///     `caret_anchor_ink_box` must return `None` at every column and the
///     existing `.max(caret_w)`-floored cell math
///     (`block_caret_width_tracks_glyph_advance`) stays untouched.
///   * a LIGATURE cluster (`cluster_span_at` reports a char span > 1): the
///     guard is exercised directly at the pure free-function seam with a
///     SYNTHETIC 2-char cluster, mirroring how `assemble_glyph_xs` is
///     unit-tested without a GPU. (Since the ligature-policy round enabled
///     `liga` on the proportional prose faces, a real prose "fi"/"ffi" now
///     DOES merge to one glyph — the empirical "never ligates" note that used
///     to sit here was true only while ligatures were globally disabled; the
///     real-cluster caret/hit-test path is now covered on the code monos by
///     `caret_and_hit_test_are_per_char_inside_a_programming_ligature_cluster`.)
#[test]
fn caret_ink_box_off_for_mono_and_ligature_cluster() {
    let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _g = crate::page::test_lock();
    let _c = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::caret::set_mode(CaretMode::Block);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping caret_ink_box_off_for_mono_and_ligature_cluster: no wgpu adapter");
        return;
    };

    // MONO world (Tawny, the default).
    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
    let text = "awl";
    for col in 0..text.chars().count() {
        p.set_view(&view(text, 0, col));
        assert!(
            p.caret_anchor_ink_box().is_none(),
            "mono world must never ink-align (col {col})"
        );
    }

    // LIGATURE fallback: the pure cluster-span seam, synthetic multi-char
    // cluster (as if "fi" shaped to a single glyph).
    assert_eq!(
        cluster_span_at("fi", &[(0, 2)], 0),
        Some(2),
        "a 2-char ligature cluster spans 2 chars"
    );
    assert_eq!(
        cluster_span_at("fi", &[(0, 1), (1, 2)], 0),
        Some(1),
        "one glyph per char spans 1 (the common case)"
    );
    assert_eq!(
        cluster_span_at("fi", &[(0, 2)], 5),
        None,
        "no cluster owns an out-of-range byte"
    );

    crate::caret::set_mode(CaretMode::Block);
}

/// FIX 2 (MORPH LINE-START DEGRADE): the cosmetic | trail must anchor on the
/// caret's CURRENT FORM, not the raw caret-mode global. When Morph melts to
/// the line-start insertion BAR (col 0 / a fresh line / an empty line —
/// [`crate::caret::morph_line_start`]) the trail must anchor at the bar's
/// LEFT-EDGE x, exactly like the real I-beam look's trail — NOT the
/// glyph-cell centre a settled/space-bar Morph caret uses. Extends
/// `cosmetic_trail_anchor_is_mode_aware` (FIX 1) with the Morph-specific
/// bar-form case.
#[test]
fn cosmetic_trail_anchor_follows_morph_linestart_bar() {
    // The anchor x's fold the page globals; mutates the process-global
    // caret mode. Hold both shared test locks (page -> caret, the suite-wide
    // order).
    let _p = crate::page::test_lock();
    let _g = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::caret::set_mode(CaretMode::Morph);
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping cosmetic_trail_anchor_follows_morph_linestart_bar: no wgpu adapter");
        return;
    };
    let text = "abc\ndef";
    // Cursor at col 0 of line 1 ("def"): the Morph LINE-START degrade — no
    // produced glyph before the insertion point, so the caret melts to the
    // I-beam's thin insertion bar.
    p.set_view(&view(text, 1, 0));
    assert!(
        crate::caret::morph_line_start(p.cursor_col),
        "fixture must sit at a Morph line-start degrade"
    );
    let (tx, ty) = p.caret_target_xy();

    // A VERTICAL kick (same column, one row up->down) so the | always shows.
    let from = Sample { x: tx, y: ty - p.metrics.line_height };
    let to = Sample { x: tx, y: ty };
    p.caret.kick_trail(from, to, false);
    p.caret.step_trail(0.03);
    let (morph_bar_x, ..) = p.caret_trail_geometry().expect("morph line-start trail active");

    // The SAME anchor the real I-beam bar uses at the SAME insertion x.
    let want_bar = tx + IBEAM_W * p.metrics.zoom * 0.5;
    assert!(
        (morph_bar_x - want_bar).abs() < 1e-3,
        "morph line-start | must anchor on the bar: got {morph_bar_x} want {want_bar}"
    );

    // Contrast: a MID-LINE Morph caret (settled on a real glyph, cell-form)
    // keeps anchoring on the CELL centre, unchanged.
    p.set_view(&view(text, 0, 2)); // "ab|c": anchors the 'b' glyph, cell-form
    assert!(
        !crate::caret::morph_line_start(p.cursor_col),
        "fixture must NOT be a line start"
    );
    let (tx2, ty2) = p.caret_target_xy();
    let from2 = Sample { x: tx2, y: ty2 - p.metrics.line_height };
    let to2 = Sample { x: tx2, y: ty2 };
    p.caret.kick_trail(from2, to2, false);
    p.caret.step_trail(0.03);
    let (morph_cell_x, ..) = p.caret_trail_geometry().expect("morph cell-form trail active");
    let want_cell = tx2 + p.caret_block_w() * 0.5;
    assert!(
        (morph_cell_x - want_cell).abs() < 1e-3,
        "morph cell-form | must still anchor on the cell centre: got {morph_cell_x} want {want_cell}"
    );

    crate::caret::set_mode(CaretMode::Block);
}
