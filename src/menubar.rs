//! THE WEB/LINUX MENU BAR — a slim, awl-RENDERED strip of menu titles across the
//! top of the canvas, the THIRD DOOR to actions on the platforms the OS gives no
//! chrome. macOS ships a real NSMenu bar (`crate::menu` + muda); a browser tab and
//! most Linux window managers give a bare wgpu canvas with nothing discoverable
//! unless you already know ⌘P — so awl draws its OWN calm menu bar there, reading
//! the SAME roster (`crate::menu::roster`) the native bar does. Clicking a title
//! drops a menu; clicking an item fires its `Action` through the SAME
//! `menu::resolve` -> `App::apply` seam a keypress uses — never new behaviour, never
//! a menu-only code path (the design law shared with `crate::menu`).
//!
//! This module owns two things, both PURE and deterministic (no clock):
//!   * the process-GLOBALS ([`MENU_BAR_ON`] shown-or-not, [`OPEN_MENU`] which
//!     dropdown is dropped) — the exact shape as [`crate::outline`] / [`crate::debug`]
//!     / [`crate::hud`], set at launch from the sticky config pref
//!     (`config::menu_bar`), flipped live by the "Toggle menu bar" command, and
//!     forced on for a capture by `--menu-bar`;
//!   * the LAYOUT MATH — where each title's clickable band sits, and where an open
//!     dropdown's rows land. The render pipeline feeds these the REAL shaped title
//!     widths (so the drawn glyphs and the hit-test can never drift — merge, don't
//!     align) and reads the boxes back for BOTH the draw and the click hit-test.
//!
//! **PLATFORM DEFAULT (the one `cfg`):** [`MENU_BAR_ON`] defaults ON for web/Linux
//! and OFF for macOS (where the native bar is the door). A capture runs native, so
//! its default is OFF — meaning a plain `--screenshot` on this machine is
//! byte-identical (the bar draws nothing, reserves no space); `--menu-bar` forces
//! the global on to drive the bar deterministically for the capture tests. The
//! DOCUMENT is inset below the bar only while it is shown (`TextPipeline::doc_top`
//! adds [`crate::render::TextPipeline::menubar_reserve`]), so caret / selection /
//! hit-test all shift together and a bar-off frame keeps its exact geometry. Every
//! OTHER top-anchored persistent surface reads the SAME accessor rather than a
//! parallel offset: the margin Outline's own vertical origin (`chrome/outline.rs`'s
//! `outline_layout`, whose draw + click hit-test share ONE `top`) and the top-right
//! search/replace panel's card (`rects.rs::panel_layout`'s `card_y`).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Whether the awl-rendered menu bar is drawn. DEFAULT: ON for web/Linux (the OS
/// gives no chrome there), OFF for macOS (the native NSMenu bar is the door — see
/// `crate::menu`). A capture runs native, so this defaults OFF and a plain
/// `--screenshot` is byte-identical; `--menu-bar` / the "Toggle menu bar" command /
/// config `menu_bar` flip it. Mirrors [`crate::outline::OUTLINE_ON`].
static MENU_BAR_ON: AtomicBool = AtomicBool::new(cfg!(not(target_os = "macos")));

/// Sentinel for "no dropdown open" in [`OPEN_MENU`].
const NONE: usize = usize::MAX;

/// Which top-level menu's dropdown is currently OPEN — an index into
/// [`crate::menu::roster`], or [`NONE`]. Transient interaction state (set by a title
/// click, cleared by an item click / a click away / hiding the bar), owned here as a
/// process-global exactly like [`crate::hud`]'s held flag so both the renderer and
/// the capture sidecar read ONE source and a `--menu-open` capture can drive it.
static OPEN_MENU: AtomicUsize = AtomicUsize::new(NONE);

/// True when the menu bar is enabled (read by the renderer each frame + the capture
/// sidecar's `menubar` block, so the two can never disagree).
pub fn menu_bar_on() -> bool {
    MENU_BAR_ON.load(Ordering::Relaxed)
}

/// Set the menu bar on/off explicitly — the config sticky-pref launch-apply
/// (`Config::apply_sticky_globals`), the settings-menu toggle, and the `--menu-bar`
/// capture flag. Turning it OFF also closes any open dropdown (a hidden bar can hold
/// no open menu). Mirrors [`crate::outline::set_outline_on`].
pub fn set_menu_bar_on(on: bool) {
    // Self-serialize into the GEOMETRY test-lock domain: the bar reserves vertical
    // space folded into `TextPipeline::doc_top`, so flipping it races any test reading
    // doc geometry. Acquiring the page test-lock (reentrant per thread) here — exactly
    // as the page-global writers do (see `page::test_lock`) — keeps a parallel geometry
    // test that holds the same lock from observing a half-flipped bar. No-op in a real
    // (non-test) build.
    #[cfg(test)]
    let _g = crate::testlock::serial();
    MENU_BAR_ON.store(on, Ordering::Relaxed);
    if !on {
        set_open(None);
    }
}

/// Flip the bar and return the now-active state (the "Toggle menu bar" command).
/// Closing the bar closes any open dropdown. Mirrors [`crate::outline::toggle`].
pub fn toggle() -> bool {
    let next = !menu_bar_on();
    set_menu_bar_on(next);
    next
}

/// Which dropdown is open (`Some(menu_index)` into [`crate::menu::roster`]), or
/// `None`. Always `None` when the bar itself is hidden ([`set_menu_bar_on`] clears
/// it), so a caller need not double-check `menu_bar_on()`.
pub fn open_menu() -> Option<usize> {
    let v = OPEN_MENU.load(Ordering::Relaxed);
    (v != NONE).then_some(v)
}

/// Open the dropdown for menu `i` (`None` closes any open one). A no-op-safe setter:
/// the renderer / hit-test tolerate an out-of-range index (nothing draws / nothing
/// hits), so a stale index can never panic.
pub fn set_open(i: Option<usize>) {
    // Serialize with the same geometry lock as [`set_menu_bar_on`] (an open dropdown
    // rides the shown bar's reserved strip); reentrant, so `set_menu_bar_on`'s internal
    // `set_open(None)` is a nested no-op. No-op in a real build.
    #[cfg(test)]
    let _g = crate::testlock::serial();
    OPEN_MENU.store(i.unwrap_or(NONE), Ordering::Relaxed);
}

/// Toggle the dropdown for menu `i`: open it if closed (or a different one is open),
/// close it if it is already the open one — the click-the-title-again behaviour of a
/// real menu bar. Returns the now-open index (or `None`).
pub fn toggle_open(i: usize) -> Option<usize> {
    let next = if open_menu() == Some(i) { None } else { Some(i) };
    set_open(next);
    next
}

// ─────────────────────────────────────────────────────────────────────────────
// LAYOUT MATH — pure, deterministic, unit-tested without a GPU. The pipeline feeds
// these the real shaped title widths and reads the results back for BOTH the draw
// and the hit-test, so the two can never drift (the merge-don't-align discipline).
// ─────────────────────────────────────────────────────────────────────────────

/// Left inset (px) of the FIRST title's clickable band from the canvas edge.
pub const BAR_INSET_X: f32 = 8.0;
/// Horizontal padding (px) each side of a title's text WITHIN its clickable band, so
/// adjacent bands abut with a comfortable gap and the hover/open highlight has room.
pub const TITLE_PAD_X: f32 = 12.0;
/// Vertical padding (px) above AND below the title text — the bar height is the text
/// line height plus twice this. Kept small so the bar reads SLIM (DESIGN: calm chrome).
pub const BAR_PAD_Y: f32 = 5.0;

/// Inner horizontal padding (px) of an open dropdown card, each side of the rows.
pub const DROP_PAD_X: f32 = 10.0;
/// Inner vertical padding (px) of an open dropdown card, above the first row + below
/// the last.
pub const DROP_PAD_Y: f32 = 6.0;

/// The slim bar's total height (px) for a given text `line_height` — one line of
/// title text plus [`BAR_PAD_Y`] above and below. The document is inset by exactly
/// this while the bar is shown (see the module doc).
pub fn bar_height(line_height: f32) -> f32 {
    line_height + 2.0 * BAR_PAD_Y
}

/// THE TOP-EDGE SLIVER FIX. How far (px) to bleed a bar quad's edge PAST a canvas
/// boundary it runs flush to, so [`shaders/selection.wgsl`]'s `fs_main` rounded-rect
/// AA feather (a ~1px-wide `smoothstep` centered on the quad's TRUE geometric edge)
/// never lands on a visible pixel. A quad whose top/left/right edge sits EXACTLY at
/// the canvas boundary (`y=0`, `x=0`, `x=width` — the bar's ground + open-title
/// highlight, which both run flush to the window) has only ~0.5px of "inside" margin
/// at the first sampled pixel center, so that row/column renders at ~84% coverage —
/// a visible sliver of whatever was drawn underneath (confirmed empirically: a
/// `--menu-bar` capture's row 0 measured EXACTLY the linear-space blend of the bar's
/// own color at ~84% opacity over the pre-existing frame content, both in the
/// margins and inside the page column). Bleeding the edge a few px PAST the boundary
/// (into space the rasterizer silently clips — nothing there is ever visible) moves
/// the true edge off-screen, so every visible pixel sits comfortably inside the
/// shape (SDF distance well past the smoothstep's `-1` floor) regardless of DPI/zoom
/// — the feather is a FIXED physical-pixel width, so a small constant bleed is safe
/// at every scale. `0.5` is the mathematical minimum; `4.0` is a generous, still
/// invisible margin.
pub const EDGE_BLEED_PX: f32 = 4.0;

/// Extend `rect`'s TOP edge (always) and its LEFT/RIGHT edges (only the ones already
/// flush with a canvas boundary, within a tiny epsilon) by [`EDGE_BLEED_PX`] — the
/// fix for the sliver documented on that constant. A rect that does NOT touch a
/// given edge is left exactly alone on that side (bleeding an INTERIOR edge would
/// visibly grow the shape, not just hide off-canvas geometry that the rasterizer was
/// always going to clip). `canvas_w` is the frame width `rect`'s x-extent is measured
/// against. Pure; unit-tested without a GPU. Used by
/// [`crate::render::TextPipeline::prepare_menubar`] for BOTH the bar's ground strip
/// (always flush on all three sides) and the open title's highlight band (always
/// flush on top; flush on a side only for the first/last title).
pub fn bleed_to_canvas_edges(rect: [f32; 4], canvas_w: f32) -> [f32; 4] {
    const FLUSH_EPS: f32 = 0.5;
    let [mut x, mut y, mut w, mut h] = rect;
    // TOP.
    if y <= FLUSH_EPS {
        y -= EDGE_BLEED_PX;
        h += EDGE_BLEED_PX;
    }
    // LEFT.
    if x <= FLUSH_EPS {
        x -= EDGE_BLEED_PX;
        w += EDGE_BLEED_PX;
    }
    // RIGHT.
    if x + w >= canvas_w - FLUSH_EPS {
        w += EDGE_BLEED_PX;
    }
    [x, y, w, h]
}

/// One title's laid-out horizontal extents (px, absolute canvas x), from
/// [`boxes_from_extents`]. Built from the SHAPED glyph positions the pipeline read
/// back (never a parallel layout), so the drawn glyphs and the click bands agree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TitleBox {
    /// Left edge of the CLICKABLE band (bands abut, so no dead gap between titles).
    pub band_left: f32,
    /// Left edge of the title's shaped GLYPHS (for the open-title highlight inset).
    pub text_left: f32,
    /// Right edge of the shaped glyphs (for the highlight inset).
    pub text_right: f32,
    /// Right edge of the clickable band (== the next band's `band_left`).
    pub band_right: f32,
}

/// Build each title's clickable band from its SHAPED glyph extents (`extents[k]` =
/// the absolute `(left, right)` canvas x of title k's glyphs, in bar order — read
/// straight off the shaped `menubar_buffer`). Adjacent bands ABUT at the midpoint
/// between neighbouring titles (so a click anywhere along the bar resolves to the
/// nearest title, no dead zones — the real menu-bar feel); the first band reaches
/// [`TITLE_PAD_X`] left of its text, the last [`TITLE_PAD_X`] right. Pure — the
/// pipeline feeds it real shaped positions and reads the boxes back for BOTH the
/// open-title highlight and the click/hover hit-test (merge, don't align).
pub fn boxes_from_extents(extents: &[(f32, f32)]) -> Vec<TitleBox> {
    let n = extents.len();
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let (l, r) = extents[k];
        let band_left = if k == 0 {
            (l - TITLE_PAD_X).max(0.0)
        } else {
            (extents[k - 1].1 + l) * 0.5
        };
        let band_right = if k + 1 < n { (r + extents[k + 1].0) * 0.5 } else { r + TITLE_PAD_X };
        out.push(TitleBox { band_left, text_left: l, text_right: r, band_right });
    }
    out
}

/// Which title's band contains the point `(px, py)` — `Some(index)` when `py` is
/// within the bar's height and `px` falls in a title band, else `None`. The single
/// hit-test owner for the bar, read by the click handler AND the cursor-shape flag,
/// so a hovered title can never disagree with a clickable one.
pub fn title_at(boxes: &[TitleBox], bar_h: f32, px: f32, py: f32) -> Option<usize> {
    if py < 0.0 || py >= bar_h {
        return None;
    }
    boxes.iter().position(|b| px >= b.band_left && px < b.band_right)
}

/// True when `(px, py)` is anywhere in the bar's own strip `[0, bar_h)` (whether or
/// not it hit a title) — the cursor-shape "over the bar chrome" band, so the pointer
/// reads as the plain arrow over dead bar space and the hand over a title.
pub fn in_bar(bar_h: f32, py: f32) -> bool {
    py >= 0.0 && py < bar_h
}

/// One dropdown row's vertical placement (px, relative to the first row's top), from
/// [`drop_rows`]. UNIFORM height: every row (item OR separator) is [`row_h`] tall, so
/// the item-text buffer lays one contiguous line per row and the hit-test is simple.
/// A SEPARATOR row draws only a thin centered hairline (no text, not clickable).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropRow {
    pub top: f32,
    pub height: f32,
    pub separator: bool,
}

/// Lay an open dropdown's rows top-to-bottom, one uniform [`row_h`]-tall slot per
/// roster item. `separators[i]` marks a non-clickable hairline row. Returns the rows
/// (tops relative to the first row) and the total stacked height.
pub fn drop_rows(separators: &[bool], row_h: f32) -> (Vec<DropRow>, f32) {
    let mut rows = Vec::with_capacity(separators.len());
    let mut top = 0.0;
    for &sep in separators {
        rows.push(DropRow { top, height: row_h, separator: sep });
        top += row_h;
    }
    (rows, top)
}

/// The dropdown CARD rectangle `[x, y, w, h]` for the menu anchored under `anchor`
/// (its title box), given the bar height, the widest row's content width, and the
/// stacked row height ([`drop_rows`]'s total). The card's left edge aligns to the
/// title's band left (a POSITIONING VARIANT — anchored under its title, not centered
/// like the summoned pickers) and it hangs just below the bar.
pub fn drop_rect(anchor: &TitleBox, bar_h: f32, content_w: f32, rows_total_h: f32) -> [f32; 4] {
    let w = content_w.max(0.0) + 2.0 * DROP_PAD_X;
    let h = rows_total_h + 2.0 * DROP_PAD_Y;
    [anchor.band_left, bar_h, w, h]
}

/// Which dropdown ITEM row `(px, py)` hits — `Some(index)` for a clickable row inside
/// the card, `None` for a separator row, off the card, or the padding. `rect` +
/// `rows` are exactly what [`drop_rect`] / [`drop_rows`] produced this frame (read
/// from the pipeline's stored geometry), so the hit-test matches the drawn rows.
pub fn drop_item_at(rect: [f32; 4], rows: &[DropRow], px: f32, py: f32) -> Option<usize> {
    let [x, y, w, h] = rect;
    if px < x || px >= x + w || py < y || py >= y + h {
        return None;
    }
    let local_y = py - (y + DROP_PAD_Y);
    if local_y < 0.0 {
        return None;
    }
    rows.iter().position(|r| !r.separator && local_y >= r.top && local_y < r.top + r.height)
}


#[cfg(test)]
mod tests {
    use super::*;

    /// THE SLIVER FIX, pure: a rect flush on all three canvas-touching sides (the
    /// bar's own ground strip) bleeds top/left/right by `EDGE_BLEED_PX`, and its
    /// BOTTOM (never a canvas edge for the bar) is untouched.
    #[test]
    fn bleed_extends_every_flush_edge_and_leaves_the_bottom_alone() {
        let rect = [0.0, 0.0, 1200.0, 32.0];
        let bled = bleed_to_canvas_edges(rect, 1200.0);
        assert_eq!(bled[0], -EDGE_BLEED_PX, "left bleeds past x=0");
        assert_eq!(bled[1], -EDGE_BLEED_PX, "top bleeds past y=0");
        // Width grew by the left AND right bleed (both edges were flush); height
        // grew by the top bleed only (bottom untouched).
        assert_eq!(bled[2], 1200.0 + 2.0 * EDGE_BLEED_PX, "width bleeds on both flush sides");
        assert_eq!(bled[3], 32.0 + EDGE_BLEED_PX, "height bleeds on the top side only");
        // The bottom edge (y + h) moves by exactly the top bleed, i.e. the BOTTOM
        // itself (a non-flush edge) never moved: bled_y + bled_h == rect_y + rect_h.
        assert_eq!(bled[1] + bled[3], rect[1] + rect[3], "the bottom edge itself is unmoved");
    }

    /// A rect that touches NEITHER the left nor the right canvas edge (an open
    /// title's highlight band in the MIDDLE of the bar) only bleeds its top — its x
    /// extent is untouched, since bleeding an interior edge would visibly grow it.
    #[test]
    fn bleed_leaves_interior_left_and_right_edges_untouched() {
        let rect = [400.0, 0.0, 80.0, 32.0]; // nowhere near x=0 or x=1200
        let bled = bleed_to_canvas_edges(rect, 1200.0);
        assert_eq!(bled[0], 400.0, "left edge is interior, untouched");
        assert_eq!(bled[2], 80.0, "width is untouched (no side bled)");
        assert_eq!(bled[1], -EDGE_BLEED_PX, "top still bleeds — it's always flush for the bar");
        assert_eq!(bled[3], 32.0 + EDGE_BLEED_PX);
    }

    /// A rect touching ONLY the right canvas edge (the LAST title's highlight band,
    /// which can run flush to the window's right side) bleeds top + right, not left.
    #[test]
    fn bleed_is_independent_per_side() {
        let rect = [1100.0, 0.0, 100.0, 32.0]; // right edge exactly at canvas_w=1200
        let bled = bleed_to_canvas_edges(rect, 1200.0);
        assert_eq!(bled[0], 1100.0, "left edge is interior, untouched");
        assert_eq!(bled[2], 100.0 + EDGE_BLEED_PX, "right bleeds (flush to canvas_w)");
        assert_eq!(bled[1], -EDGE_BLEED_PX);
    }

    /// A rect NOT touching the canvas top at all (hypothetical future caller) is
    /// left exactly alone on every side — the fix only ever touches an edge that is
    /// ACTUALLY flush with the canvas boundary, never a rect drawn purely elsewhere.
    #[test]
    fn bleed_is_a_total_no_op_off_every_canvas_edge() {
        let rect = [200.0, 50.0, 300.0, 40.0];
        assert_eq!(bleed_to_canvas_edges(rect, 1200.0), rect);
    }

    #[test]
    fn globals_toggle_and_open_close() {
        let _g = crate::testlock::serial();
        // The default matches the platform: on for web/Linux, off for macOS.
        set_menu_bar_on(true);
        assert!(menu_bar_on());
        // Opening a dropdown, then re-clicking the same title closes it.
        assert_eq!(toggle_open(2), Some(2));
        assert_eq!(open_menu(), Some(2));
        assert_eq!(toggle_open(2), None);
        assert_eq!(open_menu(), None);
        // A different title while one is open just switches.
        set_open(Some(1));
        assert_eq!(toggle_open(3), Some(3));
        // Hiding the bar closes any open dropdown.
        set_open(Some(0));
        set_menu_bar_on(false);
        assert!(!menu_bar_on());
        assert_eq!(open_menu(), None, "a hidden bar holds no open dropdown");
        // toggle reports the new state and closes on the way down.
        set_open(Some(0));
        assert!(toggle(), "toggle from off -> on");
        set_open(Some(0));
        assert!(!toggle(), "toggle from on -> off closes the dropdown");
        assert_eq!(open_menu(), None);
        set_menu_bar_on(cfg!(not(target_os = "macos")));
    }

    #[test]
    fn boxes_from_extents_abut_at_midpoints() {
        // Three titles' shaped extents: File [20,50], Edit [70,96], View [110,146].
        let boxes = boxes_from_extents(&[(20.0, 50.0), (70.0, 96.0), (110.0, 146.0)]);
        assert_eq!(boxes.len(), 3);
        // First band reaches TITLE_PAD_X left of its text; text extents preserved.
        assert_eq!(boxes[0].band_left, 20.0 - TITLE_PAD_X);
        assert_eq!(boxes[0].text_left, 20.0);
        assert_eq!(boxes[0].text_right, 50.0);
        // Interior boundaries sit at the midpoint between neighbouring titles, so
        // adjacent bands abut exactly (no dead zones, no overlap).
        assert_eq!(boxes[0].band_right, (50.0 + 70.0) / 2.0);
        assert_eq!(boxes[1].band_left, boxes[0].band_right, "bands abut");
        assert_eq!(boxes[1].band_right, (96.0 + 110.0) / 2.0);
        assert_eq!(boxes[2].band_left, boxes[1].band_right);
        assert_eq!(boxes[2].band_right, 146.0 + TITLE_PAD_X);
    }

    #[test]
    fn title_at_maps_x_across_the_whole_bar() {
        let boxes = boxes_from_extents(&[(20.0, 50.0), (70.0, 96.0), (110.0, 146.0)]);
        let bar_h = bar_height(20.0);
        // A click in each band resolves to that title.
        assert_eq!(title_at(&boxes, bar_h, boxes[0].text_left + 1.0, 4.0), Some(0));
        assert_eq!(title_at(&boxes, bar_h, boxes[1].text_left + 1.0, 4.0), Some(1));
        assert_eq!(title_at(&boxes, bar_h, boxes[2].band_right - 1.0, 4.0), Some(2));
        // A click below the bar, or left of the first band, or past the last, misses.
        assert_eq!(title_at(&boxes, bar_h, boxes[0].text_left, bar_h + 1.0), None);
        assert_eq!(title_at(&boxes, bar_h, 0.0, 4.0), None);
        assert_eq!(title_at(&boxes, bar_h, boxes[2].band_right + 5.0, 4.0), None);
    }

    #[test]
    fn drop_rows_stack_uniform_slots_marking_separators() {
        // item, item, separator, item — the App-menu-ish shape (uniform height).
        let (rows, total) = drop_rows(&[false, false, true, false], 22.0);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].top, 0.0);
        assert_eq!(rows[1].top, 22.0);
        assert_eq!(rows[2].top, 44.0);
        assert!(rows[2].separator, "the third row is the separator");
        assert_eq!(rows[3].top, 66.0);
        assert_eq!(total, 4.0 * 22.0);
    }

    #[test]
    fn drop_item_at_hits_clickable_rows_only() {
        let anchor = TitleBox { band_left: 40.0, text_left: 52.0, text_right: 84.0, band_right: 90.0 };
        let bar_h = bar_height(20.0);
        let (rows, total) = drop_rows(&[false, true, false], 22.0);
        let rect = drop_rect(&anchor, bar_h, 120.0, total);
        assert_eq!(rect[0], 40.0, "the dropdown left-aligns under its title");
        assert_eq!(rect[1], bar_h, "it hangs just below the bar");
        assert_eq!(rect[2], 120.0 + 2.0 * DROP_PAD_X);
        // First row is clickable.
        let (x, y) = (rect[0] + 5.0, rect[1] + DROP_PAD_Y + 1.0);
        assert_eq!(drop_item_at(rect, &rows, x, y), Some(0));
        // The separator row (index 1) is never a hit.
        let sep_y = rect[1] + DROP_PAD_Y + rows[1].top + 1.0;
        assert_eq!(drop_item_at(rect, &rows, x, sep_y), None);
        // The third row (index 2) is clickable.
        let third_y = rect[1] + DROP_PAD_Y + rows[2].top + 1.0;
        assert_eq!(drop_item_at(rect, &rows, x, third_y), Some(2));
        // Off the card horizontally / above the first row misses.
        assert_eq!(drop_item_at(rect, &rows, rect[0] - 1.0, y), None);
        assert_eq!(drop_item_at(rect, &rows, x, rect[1] + 1.0), None);
    }
}
