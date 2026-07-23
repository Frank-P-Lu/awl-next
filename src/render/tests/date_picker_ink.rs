//! ITEM 66 — real-pixel law: every example date the Date-format picker draws
//! (`07/03/09`, `2009-03-07`, …) must render its OWN glyphs in ONE uniform ink,
//! in both the SELECTED and an UNSELECTED row, across a Pane world, a Bars
//! world, a light world, and the one-bit (`InverseVideo`) world.
//!
//! THE BUG this guards: `shape_overlay_names`'s per-row shaper applied
//! `crate::overlay::row_split`'s muted-directory/content-filename figure/
//! ground split to every FLAT picker row unconditionally. Three of the five
//! example date formats (`DD/MM/YY`, `MM/DD/YY`, `YYYY/MM/DD`) use `/` as a
//! DATE separator, and `row_split`'s "split at the last `/`" rule mistook that
//! separator for a directory boundary — muting everything before it, leaving
//! everything after it in content ink, WITHIN one date string. The fix gates
//! that split on `OverlayKind::row_path_splits` (`false` for Date), threaded
//! through `ViewState::overlay_row_path_splits` -> `TextPipeline` (and, for a
//! headless capture, `capture::modes::settled_viewstate`, resolved from the
//! SAME owner via the overlay's `mode` string). Per CLAUDE.md's harness law,
//! the sidecar is a STATE oracle only — this test proves the APPEARANCE claim
//! ("one ink") by arithmetic over a REAL captured PNG's pixels, through the
//! exact `capture_with` path `--screenshot` uses.
//!
//! GEOMETRY NOTE: row positions are read off a throwaway single-`prepare()`
//! headless `TextPipeline` (never sampled for PIXELS — only its geometry
//! accessors), built from the identical overlay content/state `capture_with`
//! renders, so the two agree on WHERE each row's text sits without this file
//! hand-computing (and risking drifting from) the layout math.

use super::super::*;
use super::{headless_dqp, view};

/// Build the ONE Date-picker overlay content this whole file drives: all five
/// live example dates (the fixed capture placeholder date, so every run is
/// byte-stable) + their format-name labels.
fn date_examples() -> (Vec<String>, Vec<String>) {
    let today = crate::dateformat::CAPTURE_PLACEHOLDER_YMD;
    let items: Vec<String> = crate::dateformat::DateFormat::ALL
        .iter()
        .map(|f| f.format(today.0, today.1, today.2))
        .collect();
    let labels: Vec<String> =
        crate::dateformat::DateFormat::ALL.iter().map(|f| f.label().to_string()).collect();
    (items, labels)
}

/// A `ViewState` with the Date picker open on `selected`, for the geometry-only
/// probe pipeline — mirrors exactly what `capture::modes::settled_viewstate`
/// builds from an `OverlayInfo{mode: "date", ..}` (title/items/bindings/
/// row_path_splits all resolved the SAME way), so its row geometry matches the
/// captured PNG's.
fn date_picker_view(selected: usize) -> ViewState {
    let (items, labels) = date_examples();
    let mut v = view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_title = crate::overlay::OverlayKind::Date.title();
    v.overlay_items = items;
    v.overlay_bindings = labels;
    v.overlay_selected = selected;
    v.overlay_row_path_splits = crate::overlay::OverlayKind::Date.row_path_splits();
    v
}

/// Each example date row's primary-text pixel region for a FLAT picker
/// (`header_rows == 1`, the `› query` line) — a throwaway pipeline's geometry
/// accessors only (never its own rendered pixels; the real capture's are what
/// this file samples). Width is a fixed 16-char-cell budget: comfortably wider
/// than the longest example (`7 March 2009`, 12 chars) yet well short of the
/// right-aligned secondary (format-name) column the item explicitly allows to
/// differ in ink.
fn date_row_regions(world: &str, selected: usize) -> Vec<(f32, f32, f32, f32)> {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        return Vec::new();
    };
    crate::theme::set_active_by_name(world);
    let v = date_picker_view(selected);
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    let [card_x, card_y, _card_w, _] =
        p.overlay_card_rect().expect("the overlay card must be open");
    let hpad = p.overlay_text_hpad();
    let text_left = card_x + hpad;
    let lh = p.overlay_lh();
    let header_gap = p.overlay_header_gap();
    let primary_w = p.metrics.char_width * 16.0;
    (0..v.overlay_items.len())
        .map(|row| {
            let row_top = card_y + 12.0 /* pad */ + lh + header_gap + lh * row as f32;
            (text_left, row_top, primary_w, lh)
        })
        .collect()
}

/// Render the Date picker through the REAL headless-capture code path
/// (`capture::capture_with` — the exact function `--screenshot` calls) and
/// decode the written PNG back to an `RgbaImage`.
fn capture_date_picker(dir: &std::path::Path, world: &str, selected: usize, tag: &str) -> image::RgbaImage {
    use crate::capture::{capture_with, CaptureOpts, OverlayInfo};
    assert!(crate::theme::set_active_by_name(world).is_some(), "unknown world {world:?}");
    let (items, labels) = date_examples();
    let buf = crate::buffer::Buffer::from_str("hello world\n");
    let mut opts = CaptureOpts::default();
    opts.overlay = Some(OverlayInfo {
        active: true,
        mode: crate::overlay::OverlayKind::Date.as_str(),
        title: crate::overlay::OverlayKind::Date.title(),
        align: crate::render::effective_card_anchor(),
        query: String::new(),
        items,
        empty: None,
        bindings: labels,
        git: Vec::new(),
        selected_index: selected,
        hint: crate::overlay::OverlayKind::Date.hint().to_string(),
        browse_dir: None,
        return_to: None,
        spell_target: None,
        capture: None,
        notice: String::new(),
        lens: None,
        lens_strip: Vec::new(),
        sections: Vec::new(),
        preview_id: None,
        diff_focus: false,
        diff_scroll: 0,
        show_hidden: false,
    });
    let png = dir.join(format!("{world}_{tag}.png"));
    capture_with(&png, &buf, &opts).expect("date picker capture renders");
    image::open(&png).expect("decode date picker png").to_rgba8()
}

/// The single most-common pixel color over the region — a row's own
/// BACKGROUND, since glyph ink covers a small minority of a text row's total
/// area. Works uniformly whether that background is a flat card surface, a
/// selected-row VALUE band, a Bars plate/pill, or (Wagtail) a solid
/// inverse-video fill.
fn region_mode_color(img: &image::RgbaImage, x0: i64, y0: i64, x1: i64, y1: i64) -> [u8; 4] {
    use std::collections::HashMap;
    let mut counts: HashMap<[u8; 4], usize> = HashMap::new();
    for y in y0.max(0)..y1.min(img.height() as i64) {
        for x in x0.max(0)..x1.min(img.width() as i64) {
            let p = img.get_pixel(x as u32, y as u32).0;
            *counts.entry(p).or_insert(0) += 1;
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(c, _)| c).unwrap_or([0, 0, 0, 0])
}

/// Every pixel in the region whose max-channel distance from `bg` clears
/// `threshold` — a low NOISE FLOOR (this file calls with 24 of 255), not a
/// "solid fill only" bar: a genuinely low-contrast ink must still enter the
/// population for [`assert_row_one_ink`]'s gap check to see it, and that
/// check (a population-backed jump, not raw min/max range) isn't confused by
/// the anti-aliased blend continuum a single ink's edges add.
fn solid_ink_pixels(
    img: &image::RgbaImage,
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
    bg: [u8; 4],
    threshold: u8,
) -> Vec<[u8; 4]> {
    let mut out = Vec::new();
    for y in y0.max(0)..y1.min(img.height() as i64) {
        for x in x0.max(0)..x1.min(img.width() as i64) {
            let p = img.get_pixel(x as u32, y as u32).0;
            let d = p[0].abs_diff(bg[0]).max(p[1].abs_diff(bg[1])).max(p[2].abs_diff(bg[2]));
            if d > threshold {
                out.push(p);
            }
        }
    }
    out
}

/// THE LAW: every solid-ink pixel in the region clusters to ONE color within a
/// tight tolerance. Panics with the observed min/max spread on a miss, so a
/// regression reads as "two inks in one date row", not a bare assert.
fn assert_row_one_ink(img: &image::RgbaImage, region: (f32, f32, f32, f32), label: &str) {
    let (x, y, w, h) = region;
    let (x0, y0, x1, y1) = (x as i64, y as i64, (x + w) as i64, (y + h) as i64);
    let bg = region_mode_color(img, x0, y0, x1, y1);
    // A LOW noise floor, not a "solid glyph fill only" threshold: a low-
    // contrast ink (this file's own synthetic `muted` control measures ~90
    // against Tawny's real card surface — well under a naive high bar) must
    // still enter the population for the GAP check below to see it. The
    // anti-aliasing continuum a single ink produces doesn't confuse the gap
    // check (it looks for a real population-backed jump, not raw range), so
    // there is no accuracy cost to keeping the floor low.
    let solid = solid_ink_pixels(img, x0, y0, x1, y1, bg, 24);
    assert!(
        !solid.is_empty(),
        "{label}: found no solid ink pixels in [{x0},{y0})..[{x1},{y1}) (bg={bg:?}) — the row reads empty"
    );
    // BIMODALITY, not raw spread: a real numeral/letter glyph at this font size
    // has few genuinely flat interior pixels, so even at this low noise floor
    // a SINGLE ink's pixels form a continuous anti-aliasing gradient (measured
    // spanning >60 in one channel on real captures) with no gap — a
    // TWO-ink row (the pre-fix bug) instead shows two well-separated, well-
    // POPULATED clusters (muted-ink pixels near one contrast level, content-ink
    // pixels near another) with a real GAP between them. Sort each pixel's
    // max-channel distance from `bg` and look for the largest gap between
    // consecutive values; a gap only counts as "two inks" if BOTH sides hold a
    // real share of the pixels (not one glyph's stray AA corner against the
    // rest of the row).
    let mut d: Vec<u8> = solid
        .iter()
        .map(|p| p[0].abs_diff(bg[0]).max(p[1].abs_diff(bg[1])).max(p[2].abs_diff(bg[2])))
        .collect();
    d.sort_unstable();
    let mut max_gap = 0u8;
    let mut split_at = 0usize;
    for i in 1..d.len() {
        let g = d[i] - d[i - 1];
        if g > max_gap {
            max_gap = g;
            split_at = i;
        }
    }
    let (low_n, high_n) = (split_at, d.len() - split_at);
    let minority_frac = low_n.min(high_n) as f32 / d.len() as f32;
    const GAP_TOL: u8 = 45;
    const MINORITY_FLOOR: f32 = 0.05;
    assert!(
        max_gap <= GAP_TOL || minority_frac < MINORITY_FLOOR,
        "{label}: solid ink pixels split into two contrast clusters at a gap of {max_gap} \
         (> tolerance {GAP_TOL}) — {low_n} px below / {high_n} px above out of {} \
         (minority {:.1}%) — this reads as TWO inks in one example date (bg={bg:?})",
        d.len(),
        minority_frac * 100.0,
    );
}

/// THE LAW ITSELF: sweep a Pane world, a Bars world (also this roster's
/// warmest — stands in for "lava"), a light world, and the ONE-BIT
/// (InverseVideo) world; in each, every example date row's primary text must
/// be one ink — checked with row 0 SELECTED and again with row 1 SELECTED (so
/// every row is sampled unselected at least once, and the mechanism is
/// checked in both states).
#[test]
fn date_picker_examples_render_one_ink_across_worlds_and_states() {
    if headless_dqp(1200.0, 800.0).is_none() {
        eprintln!(
            "skipping date_picker_examples_render_one_ink_across_worlds_and_states: no wgpu adapter"
        );
        return;
    }
    let _g = crate::testlock::serial();
    let orig_theme = crate::theme::active_index();
    let dir = std::env::temp_dir().join(format!("awl_item66_ink_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let (items, _) = date_examples();
    assert!(
        items.iter().any(|s| s.contains('/')),
        "the law needs at least one live example date containing '/' to bite"
    );

    // Tawny: Pane, dark. Firetail: Bars, warm ("lava"). Saltpan: Pane, LIGHT
    // (the shipped default). Wagtail: Pane, the ONE-BIT (InverseVideo) world.
    for world in ["Tawny", "Firetail", "Saltpan", "Wagtail"] {
        for selected in [0usize, 1] {
            let img = capture_date_picker(&dir, world, selected, &format!("sel{selected}"));
            let regions = date_row_regions(world, selected);
            assert_eq!(regions.len(), items.len(), "{world}: one region per example date");
            for (row, (example, region)) in items.iter().zip(regions.iter()).enumerate() {
                let state = if row == selected { "selected" } else { "unselected" };
                assert_row_one_ink(
                    &img,
                    *region,
                    &format!("{world} row {row} ({example:?}, {state})"),
                );
            }
        }
    }

    crate::theme::set_active(orig_theme);
}

/// NEGATIVE CONTROL: `assert_row_one_ink` DOES fail on a genuine two-ink row,
/// and passes on a genuine one-ink row — proved directly at the pixel level
/// with REAL production color values (Tawny's own measured `muted`/
/// `base_content`/card-surface `Srgb`s, read live off the active theme, never
/// hand-picked constants) rather than re-deriving a full render of the exact
/// pre-fix shape (selected-row ink recolor makes the ONE always-selected row
/// InsertLink's single-row minibuffer offers immune to the split — the same
/// reason the pre-fix bug itself only ever showed on UNSELECTED rows). Builds
/// a small synthetic strip: LEFT half painted as glyph fill in `muted`, RIGHT
/// half in `content` — exactly the shape `row_split` + the pre-fix
/// unconditional call produced (muted-prefix / content-suffix WITHIN one
/// row) — and confirms the SAME assertion the law test above calls rejects
/// it, then accepts the one-ink counterpart.
#[test]
fn assert_row_one_ink_would_have_caught_the_pre_fix_split() {
    crate::theme::set_active_by_name("Tawny");
    let s = crate::theme::base_300();
    let bg = [s.r, s.g, s.b, 255];
    let s = crate::theme::muted();
    let muted = [s.r, s.g, s.b, 255];
    let s = crate::theme::base_content();
    let content = [s.r, s.g, s.b, 255];
    assert!(
        muted[0].abs_diff(content[0]) > 60,
        "this control needs Tawny's real muted/content inks to actually differ"
    );

    let w = 100u32;
    let h = 20u32;
    let paint_half = |right_color: [u8; 4]| -> image::RgbaImage {
        let mut img = image::RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                // A checkerboard-ish "glyph" texture over half the strip (never a
                // flat block) so the detector sees the same kind of partial-
                // coverage/AA-like pixel population a real glyph run would, not an
                // artificially clean two-color image.
                let base = if x < w / 2 { muted } else { right_color };
                // ~25% ink coverage, background the clear majority — matching a
                // real glyph run (thin strokes over mostly-empty row background),
                // which is what `region_mode_color`'s "the mode IS the
                // background" assumption relies on.
                let solid = (x * 7 + y * 13) % 10 < 3;
                img.put_pixel(x, y, image::Rgba(if solid { base } else { bg }));
            }
        }
        img
    };

    // TWO-INK strip (the pre-fix shape: muted prefix, content suffix) — must FAIL.
    let two_ink = paint_half(content);
    let region = (0.0, 0.0, w as f32, h as f32);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_row_one_ink(&two_ink, region, "synthetic pre-fix control (muted | content)");
    }));
    assert!(
        result.is_err(),
        "a synthetic two-ink strip (Tawny's real muted vs content ink) rendered as ONE ink — \
         the detector can't see the two-ink shape it exists to catch"
    );

    // ONE-INK strip (this item's fix: the whole row in content ink) — must PASS.
    let one_ink = paint_half(muted); // both halves `muted` now — genuinely uniform
    assert_row_one_ink(&one_ink, region, "synthetic post-fix control (muted | muted)");
}
