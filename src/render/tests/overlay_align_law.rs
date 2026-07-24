//! ITEM 45 — OVERLAY/PICKER ALIGNMENT AS PERSONALITY DATA.
//!
//! Three laws for the round's mechanism, mirroring the themes-as-data doctrine
//! (`theme_caps_law` / `glide_anchor_law` scanner shape):
//!
//! 1. **alignment-is-data grep-law** — the overlay alignment is resolved through
//!    ONE owner (`effective_card_anchor` → the frozen `ViewState::overlay_align`
//!    via `resolve_overlay_anchor`). No render CONSUMER re-reads the live world
//!    anchor: `effective_card_anchor(` and `render_caps.card_anchor` appear in the
//!    render tree ONLY in `render.rs` (the resolver's own definition). A stray
//!    live read in `chrome/` would relocate an open overlay on a preview cross —
//!    exactly the HARD RULE this round forbids — so the scanner bans it.
//! 2. **frozen-holds-under-a-passive-crossing** (item 45; the HOVER case after
//!    item 52) — the frozen alignment WINS over the live anchor: a theme-preview
//!    crossing that changes which world is active WITHOUT re-stamping `overlay_align`
//!    (simulated by moving `set_card_anchor_test_override` under a held frozen value —
//!    the render mirror of a passive pointer HOVER) does NOT move the open card's
//!    x-extents. The `None`-frozen contrast proves the mechanism is real (the live
//!    anchor WOULD have moved it). Item 52 adds the OTHER half: a DELIBERATE crossing
//!    (keyboard nav / wheel) DOES re-stamp `overlay_align` and relocates the card —
//!    pinned in `reanchor_crossing_law`. The render CONSUMERS still never read the
//!    live world (law 1 holds); only an upstream `reanchor` moves the card.
//! 3. **right-anchor** — `CardAnchor::TopRight` genuinely RIGHT-anchors: the row
//!    column's x-extents hug the RIGHT window edge (one inset in), the mirror of
//!    the left-anchored card hugging the LEFT edge.
//!
//! Plus a pure grammar test for the `AWL_OVERLAY_ALIGN` capture knob.

use super::super::*;
use super::{headless_pipeline, view};

// ---------------------------------------------------------------------------
// 1. ALIGNMENT-IS-DATA grep-law
// ---------------------------------------------------------------------------

/// The banned LIVE-anchor read patterns. A render CONSUMER must read the FROZEN
/// `self.overlay_align` (through `resolve_overlay_anchor`), never these.
const BANNED: &[&str] = &["effective_card_anchor(", "render_caps.card_anchor"];

/// The ONE file allowed to carry them: `render.rs`, the resolver's own home.
const OWNER: &str = "render.rs";

/// True iff `line` (real code, not a comment) contains a banned live read.
fn line_violates(line: &str) -> Option<&'static str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return None; // doc / plain comment — prose, not code.
    }
    BANNED.iter().copied().find(|p| line.contains(p))
}

/// Walk `dir`, skipping any `tests` subdirectory (the exact exemption
/// `theme_caps_law`/`glide_anchor_law` use — that's where the placement policy
/// is legitimately driven through `set_card_anchor_test_override`), collecting
/// `(basename, line_no, pattern)` violations.
fn scan_dir(dir: &std::path::Path, out: &mut Vec<(String, usize, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some("tests") {
                continue;
            }
            scan_dir(&path, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        scan_file(&path, out);
    }
}

fn scan_file(path: &std::path::Path, out: &mut Vec<(String, usize, String)>) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
    for (i, line) in text.lines().enumerate() {
        if let Some(p) = line_violates(line) {
            out.push((name.clone(), i + 1, p.to_string()));
        }
    }
}

#[test]
fn alignment_is_data_no_live_read_in_render_consumers() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    // The resolver's own home (`src/render.rs`, a FILE beside the `render/` dir)…
    scan_file(&root.join("render.rs"), &mut hits);
    // …plus every render CONSUMER (`src/render/**`, tests excluded).
    scan_dir(&root.join("render"), &mut hits);

    let stray: Vec<_> = hits.iter().filter(|(f, _, _)| f != OWNER).collect();
    assert!(
        stray.is_empty(),
        "overlay alignment is DATA through ONE owner: only `{OWNER}` may read the \
         live anchor (`effective_card_anchor(` / `render_caps.card_anchor`); every \
         render consumer must read the FROZEN `self.overlay_align` via \
         `resolve_overlay_anchor`, or an open overlay would relocate on a preview \
         cross. offending lines:\n{}",
        stray
            .iter()
            .map(|(f, l, p)| format!("  {f}:{l}  ({p})"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // NON-VACUOUS: `render.rs` really is the resolver's home — the definition line
    // (`effective_card_anchor(`) and the world-data fallback (`render_caps.card_anchor`).
    // If either were ever deleted the count would drop and the law would go quiet.
    let owner_hits = hits.iter().filter(|(f, _, _)| f == OWNER).count();
    assert!(
        owner_hits >= 2,
        "expected the resolver definition + the world-data fallback in `{OWNER}`; found {owner_hits}"
    );
}

#[test]
fn line_violates_catches_reads_and_skips_comments() {
    assert!(line_violates("    overlay_card_box_policy(crate::render::effective_card_anchor(), w, d)").is_some());
    assert!(line_violates("        None => theme::active().render_caps.card_anchor,").is_some());
    assert!(line_violates("/// falls through to the world's own `render_caps.card_anchor`").is_none());
    assert!(line_violates("// mentions effective_card_anchor( in prose").is_none());
    // The frozen path is NOT a live read.
    assert!(line_violates("resolve_overlay_anchor(self.overlay_align)").is_none());
}

// ---------------------------------------------------------------------------
// 2. AWL_OVERLAY_ALIGN capture-knob grammar (pure)
// ---------------------------------------------------------------------------

#[test]
fn awl_overlay_align_knob_parses_left_center_right() {
    use theme::CardAnchor::*;
    assert_eq!(parse_overlay_align("left"), Some(TopLeft));
    assert_eq!(parse_overlay_align("LEFT"), Some(TopLeft));
    assert_eq!(parse_overlay_align(" left "), Some(TopLeft));
    assert_eq!(parse_overlay_align("center"), Some(TopCenter));
    assert_eq!(parse_overlay_align("centre"), Some(TopCenter));
    assert_eq!(parse_overlay_align("right"), Some(TopRight));
    assert_eq!(parse_overlay_align("Right"), Some(TopRight));
    // Malformed → None (falls through to the world's own data).
    assert_eq!(parse_overlay_align("middle"), None);
    assert_eq!(parse_overlay_align(""), None);
    // `right` carries the growth mirror (right-anchor is more than placement).
    assert!(parse_overlay_align("right").unwrap().mirrors_growth());
    assert!(!parse_overlay_align("left").unwrap().mirrors_growth());
}

// ---------------------------------------------------------------------------
// 3. RIGHT-ANCHOR — the row column's x-extents hug the column edge (pure policy)
// ---------------------------------------------------------------------------

/// A right-aligned card genuinely RIGHT-anchors: at a comfortable window its
/// right edge sits one full interior-rail inset (item 67) in from the window's
/// right margin, the mirror of a left-aligned card sitting one rail inset in
/// from the LEFT edge — so the row column (`card_x + card_w`, one `hpad` shy of
/// the card's right edge) reads flush to the right rail. Center sits, well,
/// centered between the two.
#[test]
fn right_anchor_hugs_the_right_edge_left_hugs_the_left() {
    let ww = 1200.0_f32;
    let desired = chrome::CARD_MAX_W; // comfortable — no fill regime
    let inset = chrome::overlay_rail_inset(ww);

    let (lx, lw) = chrome::overlay_card_box_policy(theme::CardAnchor::TopLeft, ww, desired);
    let (cx, cw) = chrome::overlay_card_box_policy(theme::CardAnchor::TopCenter, ww, desired);
    let (rx, rw) = chrome::overlay_card_box_policy(theme::CardAnchor::TopRight, ww, desired);

    // Same width in every regime — alignment moves the card, never resizes it.
    assert!((lw - rw).abs() < 0.5 && (cw - rw).abs() < 0.5, "alignment must not resize the card");

    // LEFT: the card's LEFT extent hugs the left window margin (one inset in).
    assert!((lx - inset).abs() < 0.5, "left-anchored card hugs the left edge: x={lx}");

    // RIGHT: the card's RIGHT extent hugs the right window margin (one inset in).
    let right_extent = rx + rw;
    assert!(
        (right_extent - (ww - inset)).abs() < 0.5,
        "right-anchored row column must hug the right edge: card_x+card_w={right_extent}, want {}",
        ww - inset
    );

    // Genuinely three distinct rails, monotonic left→center→right.
    assert!(lx < cx && cx < rx, "left({lx}) < center({cx}) < right({rx})");
    // And the right card's CENTER sits well past the viewport midpoint — item 67's
    // generous interior rail means the wide card's BODY may now straddle the
    // midline (breathing room, not a corner hug), but the card unmistakably
    // reads as a RIGHT rail: its center sits near the two-thirds mark, not the
    // half mark.
    let rcx = rx + rw * 0.5;
    assert!(
        rcx > ww * 0.5 + 1.0,
        "the right-anchored card's CENTER sits right of the midpoint: center={rcx}, mid={}",
        ww * 0.5
    );
}

// ---------------------------------------------------------------------------
// 2. FROZEN-HOLDS-UNDER-A-PASSIVE-CROSSING (rendered geometry) — the frozen
//    alignment holds an open card in place when a theme-preview crossing changes
//    the live anchor WITHOUT a deliberate re-anchor (the HOVER case; item 52's
//    deliberate crossing is `reanchor_crossing_law`).
// ---------------------------------------------------------------------------

/// Read the currently-set overlay's card rect from a pipeline that has ingested
/// `v`, at 1200×800.
fn card_x_after(p: &mut TextPipeline, v: &ViewState) -> [f32; 4] {
    p.set_size(1200.0, 800.0);
    p.set_view(v);
    p.overlay_card_rect().expect("an overlay card")
}

#[test]
fn open_overlay_never_relocates_when_preview_crosses_worlds() {
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping open_overlay_never_relocates_when_preview_crosses_worlds: no wgpu adapter");
        return;
    };

    // A summoned picker with a couple of rows, its alignment FROZEN CENTER at the
    // moment it opened (as `OverlayState::align` would capture on a centered world).
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = vec!["Alpha".into(), "Beta".into()];
    v.overlay_align = Some(theme::CardAnchor::TopCenter);

    // Frame A — the world active at summon (its live anchor == the frozen one).
    set_card_anchor_test_override(Some(theme::CardAnchor::TopCenter));
    let [ax, _, aw, _] = card_x_after(&mut p, &v);

    // Frame B — the SAME open picker, but a PASSIVE theme-preview crossing (a
    // hover: the live anchor now differs but `overlay_align` is NOT re-stamped) to
    // a LEFT-anchored world. The frozen value is unchanged, so the card must NOT
    // move. (A DELIBERATE crossing re-stamps it — see `reanchor_crossing_law`.)
    set_card_anchor_test_override(Some(theme::CardAnchor::TopLeft));
    let [bx, _, bw, _] = card_x_after(&mut p, &v);

    assert!(
        (ax - bx).abs() < 0.5 && (aw - bw).abs() < 0.5,
        "an open overlay must hold its x-extents across a preview crossing: \
         A=({ax},{aw}) B=({bx},{bw})"
    );

    // NON-VACUOUS CONTRAST — WITHOUT the freeze (`overlay_align = None`), the very
    // same live crossing DOES relocate the card, proving the freeze is what holds it.
    let mut vlive = view("hello\n", 0, 0);
    vlive.overlay_active = true;
    vlive.overlay_items = vec!["Alpha".into(), "Beta".into()];
    vlive.overlay_align = None;
    set_card_anchor_test_override(Some(theme::CardAnchor::TopCenter));
    let [cx, _, _, _] = card_x_after(&mut p, &vlive);
    set_card_anchor_test_override(Some(theme::CardAnchor::TopLeft));
    let [dx, _, _, _] = card_x_after(&mut p, &vlive);
    assert!(
        (cx - dx).abs() > 1.0,
        "the live (unfrozen) card WOULD move on a crossing — the test's own control: \
         center-x={cx}, left-x={dx}"
    );

    set_card_anchor_test_override(None);
}

/// The rendered mirror of the pure-policy right-anchor law: an overlay whose
/// alignment froze RIGHT draws its card hugging the right window edge, while the
/// LEFT-frozen twin hugs the left — read straight off the prepared geometry.
#[test]
fn frozen_right_alignment_renders_against_the_right_edge() {
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping frozen_right_alignment_renders_against_the_right_edge: no wgpu adapter");
        return;
    };
    let ww = 1200.0_f32;
    let inset = chrome::overlay_rail_inset(ww);
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = vec!["Alpha".into(), "Beta".into()];

    v.overlay_align = Some(theme::CardAnchor::TopRight);
    let [rx, _, rw, _] = card_x_after(&mut p, &v);
    assert!(
        ((rx + rw) - (ww - inset)).abs() < 0.5,
        "the frozen-right card's row column hugs the right edge: card_x+card_w={}",
        rx + rw
    );

    v.overlay_align = Some(theme::CardAnchor::TopLeft);
    let [lx, _, _, _] = card_x_after(&mut p, &v);
    assert!((lx - inset).abs() < 0.5, "the frozen-left card hugs the left edge: x={lx}");
    assert!(rx > lx + 1.0, "right-frozen card ({rx}) sits well right of the left-frozen one ({lx})");
}

// ---------------------------------------------------------------------------
// 3b. FABLE PICKS (item 45) — the right-anchor law reaching REAL WORLD DATA.
// ---------------------------------------------------------------------------

/// The overlay-audition's fable pass (item 45) flipped exactly two shipped
/// worlds to a RIGHT rail — Cassowary (a terminal readout) and Mangrove (a tidal
/// margin) — leaving every other world at its own alignment. This anchors the
/// pure/rendered right-anchor laws above to the SHIPPED data: those two worlds
/// (and ONLY those two) carry `CardAnchor::TopRight`, and an overlay summoned
/// while one of them is active freezes that world's own RIGHT anchor and draws
/// its card hugging the right window edge — the mechanism reaching all the way
/// through world data, not just a hand-forced `overlay_align`.
#[test]
fn fable_right_picks_ship_right_anchor_and_render_against_the_right_edge() {
    // DATA — the RIGHT-anchored shipped worlds are EXACTLY the two fable picks.
    let mut right: Vec<&str> = theme::THEMES
        .iter()
        .filter(|t| t.render_caps.card_anchor == theme::CardAnchor::TopRight)
        .map(|t| t.name)
        .collect();
    right.sort_unstable();
    assert_eq!(
        right,
        vec!["Cassowary", "Mangrove"],
        "item 45's fable RIGHT picks are exactly Cassowary + Mangrove; every other \
         shipped world keeps its own alignment. found: {right:?}"
    );

    // RENDERED — summon under each flipped world and read the card straight off
    // geometry: its own frozen RIGHT anchor hugs the right window edge.
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping fable_right_picks_ship_right_anchor_and_render_against_the_right_edge: no wgpu adapter");
        return;
    };
    let ww = 1200.0_f32;
    let inset = chrome::overlay_rail_inset(ww);
    let restore = theme::active().name;
    set_card_anchor_test_override(None); // the world's OWN data drives placement

    for world in ["Cassowary", "Mangrove"] {
        theme::set_active_by_name(world).unwrap();
        p.sync_theme();
        let mut v = view("hello\n", 0, 0);
        v.overlay_active = true;
        v.overlay_items = vec!["Alpha".into(), "Beta".into()];
        // Freeze the world's OWN anchor exactly as the summon path does
        // (`OverlayState::align` = `effective_card_anchor()` at open).
        v.overlay_align = Some(crate::render::effective_card_anchor());
        assert_eq!(
            v.overlay_align,
            Some(theme::CardAnchor::TopRight),
            "{world} must freeze a RIGHT anchor at summon"
        );
        let [rx, _, rw, _] = card_x_after(&mut p, &v);
        assert!(
            ((rx + rw) - (ww - inset)).abs() < 0.5,
            "{world}'s summoned card hugs the right edge: card_x+card_w={}, want {}",
            rx + rw,
            ww - inset
        );
        // Its CENTER sits well past the viewport midpoint — genuinely a right
        // rail (near the two-thirds mark), not a nudge (item 67's generous rail
        // may let a wide card's body straddle the midline; the center never does).
        let rcx = rx + rw * 0.5;
        assert!(
            rcx > ww * 0.5 + 1.0,
            "{world}'s right-anchored card center sits right of the midpoint: center={rcx}"
        );
    }

    theme::set_active_by_name(restore).unwrap();
}
