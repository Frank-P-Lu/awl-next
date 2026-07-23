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

/// The largest single-channel absolute difference between two RGB(A) pixels —
/// POLARITY-AGNOSTIC (unlike [`brightest_in`], which only finds ink LIGHTER than
/// its ground — the right read for the two Lava worlds above, but wrong for a
/// flat/light world like Bilby, where the heading's own body ink is DARKER than
/// the page). A real ink pixel of either polarity trips this; anti-aliased noise
/// against a smooth gradient ground does not (the floor below is chosen well
/// clear of gradient dither).
fn max_channel_diff(a: [u8; 4], b: [u8; 4]) -> u8 {
    (0..3).map(|i| (a[i] as i16 - b[i] as i16).unsigned_abs() as u8).max().unwrap()
}

/// The largest [`max_channel_diff`] found in `[x0, x1) x [y0, y1)` against ITS OWN
/// COLUMN's ground, sampled at `(x, ground_y)` — PER-X, not one fixed reference
/// point, because the scanned band can straddle TWO different backgrounds: the
/// page column's own gradient/lava fill stops at the (narrower) text-column edge,
/// and the outer margin (a flat, DIFFERENT color) picks up beyond the (wider)
/// page column edge — a single far-away reference reads the column's own paint
/// as "ink" (a false bleed) the moment the scan crosses that internal seam.
/// Sampling per-X at a row (`ground_y`) known to be ink-free sidesteps this
/// entirely: it reads whatever THIS column's real background is, gradient or
/// lava blob alike — close enough to the scanned band that a smooth background
/// pattern cannot itself drift past the floor. `ground_y` must be ink-free for
/// every `x` scanned (the caller picks a row that never carries text — ABOVE
/// `text_origin.top`, safe whether or not the document below is folded).
fn max_ink_in(img: &image::RgbaImage, x0: u32, x1: u32, y0: u32, y1: u32, ground_y: u32) -> u8 {
    let mut best = 0u8;
    let gy = ground_y.min(img.height() - 1);
    for x in x0..x1.min(img.width()) {
        let bg = img.get_pixel(x, gy).0;
        for y in y0..y1.min(img.height()) {
            let d = max_channel_diff(img.get_pixel(x, y).0, bg);
            if d > best {
                best = d;
            }
        }
    }
    best
}

/// The RIGHTMOST x in `[x0, x1)` carrying real ink anywhere in `[y0, y1)` — same
/// per-column ground convention as [`max_ink_in`] — or `None` when the whole band
/// is clear.
fn rightmost_ink_x(img: &image::RgbaImage, x0: u32, x1: u32, y0: u32, y1: u32, ground_y: u32) -> Option<u32> {
    let gy = ground_y.min(img.height() - 1);
    for x in (x0..x1.min(img.width())).rev() {
        let bg = img.get_pixel(x, gy).0;
        let hit = (y0..y1.min(img.height())).any(|y| max_channel_diff(img.get_pixel(x, y).0, bg) > 12);
        if hit {
            return Some(x);
        }
    }
    None
}

/// ITEM 73 (Fable-flagged item 65 defect): a collapsed heading's own "… N lines"
/// TAIL must never bleed past the writing column's own TEXT-COLUMN right edge —
/// `text_origin.left + text_wrap_width()`, the SAME boundary [`crate::render::
/// TextPipeline::text_wrap_width`]'s own "the right margin mirrors the left" doc
/// names, and the exact quantity `render::tests::folds::
/// fold_tail_hangs_after_the_first_visual_row_when_the_heading_wraps` already
/// checks the tail's LEFT against (that item 65 law caught the FLATTENED-across-
/// wrapped-rows placement bug; this one catches the NARROWER defect Fable found
/// in the corrected placement itself — `fold_affordance_base_x` reads the first
/// visual row's own real end-x, which is right, but the old code never accounted
/// for the TAIL'S OWN shaped width, so a heading whose first row already runs
/// close to the column edge had its tail's ink carry past it). Not read off the
/// sidecar (a STATE oracle only, per CLAUDE.md's own tripwire) — measured over
/// the real captured PNG via [`max_ink_in`], polarity-agnostic so ONE test covers
/// both a dark-ink-on-light world (Bilby) and a light-ink-on-dark Lava world
/// (Firetail).
///
/// Each world gets TWO fixtures, both real H2 headings engineered to wrap with
/// their first visual row landing close to the column edge (found by sweeping
/// filler words/characters through this exact capture path and reading back
/// each candidate's own real geometry, since the outline panel this path shows
/// narrows the column vs. a plain `--screenshot`):
///   - a STRONG case reproducing Fable's own find (pre-fix: visible bleed) — the
///     fix has genuinely no room here, so it ELIDES the tail; asserted by NO ink
///     anywhere past the column edge.
///   - a NARROW case where the fix's shift branch (not elide) applies — enough
///     room remains that the tail still draws, shifted left over the row's own
///     trailing whitespace; asserted BOTH that it stays inside the column edge
///     AND that it still visibly draws (never silently vanishes when there was
///     room to shift instead).
#[test]
fn fold_tail_never_bleeds_past_the_text_column_edge_on_a_wrapped_heading() {
    if !adapter_available() {
        eprintln!(
            "skipping fold_tail_never_bleeds_past_the_text_column_edge_on_a_wrapped_heading: no wgpu adapter"
        );
        return;
    }
    let _g = crate::testlock::serial();
    crate::page::set_page_on(true);

    struct Case {
        world: &'static str,
        heading: &'static str,
        tail_still_shows: bool,
    }
    // Every heading is a real, dictionary-clean sentence (no spellcheck squiggle
    // to confound the ink scan) engineered — by sweeping real fill words through
    // THIS EXACT capture path (`Buffer` + `capture_with` + `CaptureOpts::default`,
    // which shows the margin outline and so wraps NARROWER than a plain
    // `--screenshot`) — to land its first visual row at a specific distance from
    // the text-column edge.
    let cases = [
        Case {
            world: "Bilby",
            heading: "## A section heading with many words that keeps extending further \
                      to probe the wrap boundary near the edge",
            tail_still_shows: false, // Fable's own strong-bleed find: no room, elides.
        },
        Case {
            world: "Firetail",
            // Same heading text as the Bilby case above — it elides here too,
            // by coincidence of this fixture's own geometry, not by design.
            heading: "## A section heading with many words that keeps extending further \
                      to probe the wrap boundary near the edge",
            tail_still_shows: false,
        },
        Case {
            world: "Bilby",
            heading: "## A section heading with many words that keepsiiiiii extending \
                      further to probe the wrap boundary near the edge",
            tail_still_shows: true, // just enough room: shifts left, still visible.
        },
        Case {
            world: "Firetail",
            heading: "## A section heading among many words that keeps extending further \
                      to probe the wrap boundary near the edge",
            tail_still_shows: true,
        },
    ];

    let dir = std::env::temp_dir().join(format!("awl_fold_tail_clamp_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    for (i, case) in cases.iter().enumerate() {
        let text = format!("{}\n\nbody one\n\nbody two\n", case.heading);
        assert!(
            crate::theme::set_active_by_name(case.world).is_some(),
            "unknown world {:?}",
            case.world
        );

        // UNFOLDED baseline capture first — same caret (0, 0), so the heading's
        // own WYSIWYG reveal state is identical to the folded capture below (the
        // CLAUDE.md tripwire: conceal reveal changes glyph advances). This is the
        // heading's own real text ink alone, no tail, letting the folded capture
        // below be judged by how much FARTHER RIGHT it reaches — not by a
        // hand-guessed pixel band that risks re-detecting the heading's own text.
        let unfolded_buf = Buffer::from_str(&text);
        let unfolded_png = dir.join(format!("case_{i}_{}_unfolded.png", case.world));
        capture_with(&unfolded_png, &unfolded_buf, &CaptureOpts::default()).expect("unfolded capture");
        let unfolded_img = image::open(&unfolded_png).expect("decode unfolded png").to_rgba8();

        // FOLDED capture: same buffer, heading collapsed.
        let mut buf = Buffer::from_str(&text);
        let folded = buf.toggle_fold_at_cursor();
        assert_eq!(folded, Some(0), "{}: the heading (line 0) is what folds", case.world);
        let png = dir.join(format!("case_{i}_{}.png", case.world));
        capture_with(&png, &buf, &CaptureOpts::default()).expect("folded capture");
        let img = image::open(&png).expect("decode fold-tail-clamp png").to_rgba8();

        // GEOMETRY off THIS capture's own sidecar (never a hand-typed constant).
        let sidecar: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
                .expect("parse fold-tail-clamp sidecar");
        let text_left = sidecar["text_origin"]["left"].as_f64().unwrap() as f32;
        let text_top = sidecar["text_origin"]["top"].as_f64().unwrap() as u32;
        let col_left = sidecar["page"]["column"]["left"].as_f64().unwrap() as f32;
        let col_w = sidecar["page"]["column"]["width"].as_f64().unwrap() as f32;
        let line_h = sidecar["font"]["line_height"].as_f64().unwrap() as u32;

        // THE TEXT-COLUMN right edge — `text_wrap_width()`'s own "mirrors the
        // left" doc: the text pad reserved left of `text_left` (`text_left -
        // col_left`) is mirrored on the right too, so the boundary the document's
        // own prose wraps against sits `text_pad` short of the page column's own
        // (wider) visual right edge.
        let text_pad = text_left - col_left;
        let column_right = (text_left + (col_w - 2.0 * text_pad)).round() as u32;

        // A generous first-visual-row band: an H2's grown row plus slack for the
        // wrapped second row.
        let row_top = text_top;
        let row_bottom = text_top + line_h * 3;
        // Ink-free reference row: ABOVE the text origin, never carrying a glyph
        // in EITHER capture (folded or not) — close enough to the scanned band
        // for a smooth background (gradient OR lava blob) to still read as
        // itself at every x this test scans.
        let ground_y = text_top.saturating_sub(10);

        // THE LAW: no ink anywhere past the text-column edge, in the heading's
        // own row band. (`+1` so the boundary pixel itself is never over-strict.)
        let bleed = max_ink_in(&img, column_right + 1, img.width(), row_top, row_bottom, ground_y);
        assert!(
            bleed <= 12,
            "{}: fold tail bled past the text-column edge (max channel diff {bleed} \
             past x={column_right}) — case {i} {:?}",
            case.world,
            case.heading
        );

        // The heading's own text (no tail) never reaches the column edge either —
        // a sanity floor on the fixture itself (else this case can't tell "the
        // heading's own ink" apart from "the tail's own ink" below).
        let unfolded_rightmost =
            rightmost_ink_x(&unfolded_img, col_left as u32, column_right + 1, row_top, row_bottom, ground_y)
                .unwrap_or(col_left as u32);

        if case.tail_still_shows {
            // The FOLDED capture must reach FARTHER RIGHT than the unfolded
            // baseline (the tail adds real ink the bare heading text didn't have)
            // yet still never past the column edge — proving the clamp SHIFTS
            // the tail rather than silently eliding it when there was room to.
            let folded_rightmost =
                rightmost_ink_x(&img, col_left as u32, column_right + 1, row_top, row_bottom, ground_y);
            assert!(
                folded_rightmost.is_some_and(|x| x > unfolded_rightmost + 10),
                "{}: fold tail should still be visible (shifted, not elided) past the \
                 heading's own text end (unfolded rightmost {unfolded_rightmost}, folded \
                 rightmost {folded_rightmost:?}) — case {i} {:?}",
                case.world,
                case.heading
            );
        }
    }
}
