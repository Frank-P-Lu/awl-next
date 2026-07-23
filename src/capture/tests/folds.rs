//! item 65 capture-level fold laws, driven through the REAL `Buffer` fold
//! gestures (`toggle_fold_at_cursor`) and the REAL `capture_with` entry point —
//! the same harness a live `--screenshot` uses — so both are genuine end-to-end
//! proofs, not a re-derivation of the pure fold math `fold::tests` /
//! `render::tests::folds` already cover at the purer seams. TWO laws:
//! collapsing a section and then unfolding it again must restore the capture
//! BYTE-IDENTICALLY (PNG + sidecar) to the pre-collapse frame; and (the Fable
//! adjustment round) the fold chevron/tail's own ink must clear a real-pixel
//! contrast floor against the page ground it ACTUALLY renders on, on every
//! `Background::Lava` world.

use super::super::*;
use super::adapter_available;
use crate::buffer::Buffer;

/// IDENTICAL RESTORATION AFTER UNFOLD: capture (unfolded) -> collapse -> capture
/// (must differ from the first — else this proves nothing) -> unfold -> capture
/// (must be byte-identical to the FIRST, both the PNG and the sidecar JSON). Also
/// exercises the item 65 Outline correlation along the way: the sidecar's
/// `outline.collapsed` names the folded heading's index while collapsed, and is
/// empty again once restored.
#[test]
fn collapse_then_unfold_restores_the_capture_byte_identically() {
    if !adapter_available() {
        eprintln!("skipping collapse_then_unfold_restores_the_capture_byte_identically: no wgpu adapter");
        return;
    }
    let _g = crate::testlock::serial();

    let dir = std::env::temp_dir().join(format!("awl_fold_restore_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let text = "# Alpha\n\nalpha body 1\nalpha body 2\n\n## Beta\n\nbeta body\n\n# Gamma\n\ngamma body\n";
    let mut buf = Buffer::from_str(text);
    // The caret starts at (0,0) — on "# Alpha", the heading `toggle_fold_at_cursor`
    // below folds.

    // --- BASE: nothing folded. ---
    let base_png = dir.join("base.png");
    capture_with(&base_png, &buf, &CaptureOpts::default()).expect("base capture");
    let base_bytes = std::fs::read(&base_png).unwrap();
    let base_json = std::fs::read_to_string(base_png.with_extension("json")).unwrap();
    assert!(
        base_json.contains("\"collapsed\": []"),
        "nothing folded yet: outline.collapsed is empty in the base capture"
    );

    // --- COLLAPSE: fold "# Alpha" (the heading enclosing the caret). ---
    let folded_heading = buf.toggle_fold_at_cursor();
    assert_eq!(folded_heading, Some(0), "Alpha (line 0) is the heading that folded");
    assert!(buf.has_folds(), "the buffer now carries one fold");
    let mid_png = dir.join("mid.png");
    capture_with(&mid_png, &buf, &CaptureOpts::default()).expect("collapsed capture");
    let mid_json = std::fs::read_to_string(mid_png.with_extension("json")).unwrap();
    assert_ne!(
        mid_json, base_json,
        "the collapsed capture must actually differ from the base — else this round-trip proves nothing"
    );
    assert!(
        mid_json.contains("\"collapsed\": [0]"),
        "outline.collapsed names Alpha's own heading index (0) while it is folded: {mid_json}"
    );
    assert!(
        !mid_json.contains("Beta"),
        "DESCENDANT SUPPRESSION: Beta (buried under the folded Alpha) must not appear anywhere in the sidecar: {mid_json}"
    );

    // --- UNFOLD: toggle the SAME heading again. ---
    let unfolded_heading = buf.toggle_fold_at_cursor();
    assert_eq!(unfolded_heading, Some(0), "toggling again unfolds the same heading");
    assert!(!buf.has_folds(), "the buffer carries no folds again");
    let after_png = dir.join("after.png");
    capture_with(&after_png, &buf, &CaptureOpts::default()).expect("restored capture");
    let after_bytes = std::fs::read(&after_png).unwrap();
    let after_json = std::fs::read_to_string(after_png.with_extension("json")).unwrap();

    assert_eq!(
        after_bytes, base_bytes,
        "unfolding must restore the rendered PNG byte-for-byte identical to the pre-collapse frame"
    );
    assert_eq!(
        after_json, base_json,
        "unfolding must restore the sidecar JSON byte-for-byte identical to the pre-collapse frame"
    );
}

/// WCAG relative-contrast ratio between two opaque colors, gamma-correct
/// Rec.709 — the SAME small, deliberate duplication `theme::tests::
/// wcag_contrast` documents (a tiny pure-math helper needed at multiple test
/// seams); this copy exists because THIS law needs it over real captured
/// pixels, which `theme::tests` never decodes.
fn wcag_contrast(a: [u8; 4], b: [u8; 4]) -> f32 {
    fn lin(u: u8) -> f32 {
        let s = u as f32 / 255.0;
        if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    }
    fn rel_lum(c: [u8; 4]) -> f32 {
        0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
    }
    let (la, lb) = (rel_lum(a), rel_lum(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// The brightest pixel in a region — both the fold chevron and tail draw
/// LIGHTER than the ground on every world this law covers (`FoldAfford`
/// only ever lifts `muted`/`faint` TOWARD `base_content`, never away from
/// it), so the brightest pixel in a mark's own small region is that mark's
/// own ink at its strongest coverage (the anti-aliased-edge-safe read the
/// `date_picker_ink` file's `region_mode_color`/`solid_ink_pixels` establish
/// the precedent for: never assert on a single hand-picked coordinate).
fn brightest_in(img: &image::RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32) -> [u8; 4] {
    fn lin(u: u8) -> f32 {
        let s = u as f32 / 255.0;
        if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    }
    fn rel_lum(c: [u8; 4]) -> f32 {
        0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
    }
    let mut best = [0u8, 0, 0, 255];
    let mut best_lum = -1.0f32;
    for y in y0..y1.min(img.height()) {
        for x in x0..x1.min(img.width()) {
            let p = img.get_pixel(x, y).0;
            let l = rel_lum(p);
            if l > best_lum {
                best_lum = l;
                best = p;
            }
        }
    }
    best
}

/// ITEM 65's FABLE ADJUSTMENT — THE NAMED REAL-GROUND CONTRAST FLOOR: on a
/// `Background::Lava` world (Mangrove, Firetail), `LavaEdge::Glow`'s own "soft
/// light-spill under the column" lifts the WHOLE writing column off flat
/// `base_100` — so `theme::tests::fold_tail_ink_clears_the_readable_floor_
/// and_stays_quieter_than_heading_ink`'s theoretical `faint`-vs-`base_100`
/// check (still a valid, separate ink-ladder regression guard on the bare
/// token) cannot see this: Fable's item 65 taste audit measured the fold
/// chevron/tail against the page ground they ACTUALLY render on and found
/// Mangrove's chevron at ~1.5:1, Mangrove's tail at ~1.4:1, and Firetail's
/// tail at ~1.4:1 — all effectively invisible — while Firetail's OWN chevron
/// already read fine there (~2.9:1). `FoldAfford` (`theme::model`) fixes
/// exactly the three flagged marks, leaving Firetail's chevron untouched.
/// Proven over a genuine captured PNG through the exact `capture_with` path
/// `--screenshot` uses (CLAUDE.md's own tripwire: the sidecar is a state
/// oracle, never an appearance oracle — this claim is arithmetic over pixels).
#[test]
fn fold_afford_ink_clears_the_real_lava_ground_on_every_flagged_world() {
    if !adapter_available() {
        eprintln!("skipping fold_afford_ink_clears_the_real_lava_ground_on_every_flagged_world: no wgpu adapter");
        return;
    }
    let _g = crate::testlock::serial();

    const FLOOR: f32 = 2.7; // just under the ~2.9-3.2:1 every calibrated mark actually hits, leaving AA-rounding slack.
    let dir = std::env::temp_dir().join(format!("awl_fold_afford_lava_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let text = "# Alpha\n\nalpha body 1\nalpha body 2\n\n## Beta\n\nbeta body\n";

    for world in ["Mangrove", "Firetail"] {
        let mut buf = Buffer::from_str(text);
        // Caret starts at (0,0), on "# Alpha" — folding it also REVEALS its own
        // chevron in headless (caret-on-collapsed-heading; no pointer -> no hover).
        let folded = buf.toggle_fold_at_cursor();
        assert_eq!(folded, Some(0), "{world}: Alpha (line 0) is the heading that folded");
        assert!(crate::theme::set_active_by_name(world).is_some(), "unknown world {world:?}");

        let png = dir.join(format!("{world}_fold_afford.png"));
        capture_with(&png, &buf, &CaptureOpts::default()).expect("folded capture");
        let img = image::open(&png).expect("decode fold-afford png").to_rgba8();

        // GEOMETRY, read off THIS capture's own sidecar rather than a hand-
        // guessed pixel constant (`text_origin`/`page.column`/`font.line_height`
        // — CAPTURE.md's schema) — so the search boxes below track whatever
        // this run's ACTUAL layout is, never drift from a number typed once.
        let sidecar: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
                .expect("parse fold-afford sidecar");
        let text_left = sidecar["text_origin"]["left"].as_f64().unwrap() as u32;
        let text_top = sidecar["text_origin"]["top"].as_f64().unwrap() as u32;
        let col_left = sidecar["page"]["column"]["left"].as_f64().unwrap() as u32;
        let col_w = sidecar["page"]["column"]["width"].as_f64().unwrap() as u32;
        let line_h = sidecar["font"]["line_height"].as_f64().unwrap() as u32;
        // Generous first-visual-row band: the H1 heading row grows past ordinary
        // `line_height`, but nothing else renders below it (Alpha's own fold
        // swallows the rest of the doc through EOF), so 3x is comfortable
        // headroom with nothing else on-canvas to collide with.
        let row_bottom = text_top + line_h * 3;

        // GROUND: well below the heading row, clear of any text/ornament — the
        // lava-lit column's own flat-field color at this deterministic phase.
        let ground = img.get_pixel(col_left + 50, row_bottom + 300).0;

        // CHEVRON: the writing column's own LEADING PAD (`column_left..text_left`
        // — `fold_chevron_left`'s own doc), strictly BEFORE the heading text (and
        // the caret block, which sits over the heading's own first glyph) so it
        // can never pick up the caret's amber ink instead of the chevron's.
        let chevron = brightest_in(&img, col_left + 1, text_top, text_left.saturating_sub(1), row_bottom);
        let chevron_c = wcag_contrast(chevron, ground);
        assert!(
            chevron_c >= FLOOR,
            "{world}: fold chevron {chevron:?} only {chevron_c:.2}:1 against the real \
             rendered ground {ground:?} (floor {FLOOR}:1)"
        );

        // TAIL: well past "Alpha" (past any wrap — `line_h * 6` clears even a
        // generously-scaled H1's "Alpha"), still short of the right margin, and
        // past the small caret block, which never reaches this far.
        let tail = brightest_in(&img, text_left + line_h * 6, text_top, col_left + col_w, row_bottom);
        let tail_c = wcag_contrast(tail, ground);
        assert!(
            tail_c >= FLOOR,
            "{world}: fold tail {tail:?} only {tail_c:.2}:1 against the real rendered \
             ground {ground:?} (floor {FLOOR}:1)"
        );
    }
}
