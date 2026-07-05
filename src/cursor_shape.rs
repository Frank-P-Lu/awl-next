//! src/cursor_shape.rs — CONTEXT-AWARE OS pointer shapes: winit draws the
//! actual glyph (`Window::set_cursor(CursorIcon::…)`); this module is the ONE
//! pure priority decision (hover context -> `CursorIcon`) plus the tiny
//! "only call on an actual change" cache logic, so `App` never has to
//! re-derive the priority ad hoc at each mouse call site — the same
//! single-owner discipline as `syn_role_color` / `pointer_hide::os_visibility_change`.
//!
//! **The mapping** (macOS convention, not the web one — settled taste call):
//! 1. over the TEXT AREA (the writing column, no overlay open) -> `Text` (I-beam).
//! 2. over the draggable PAGE-COLUMN EDGE, or while actively dragging it ->
//!    `ColResize` (↔).
//! 3. over a summoned OVERLAY's rows (palette / pickers / the right-click
//!    spell-suggest panel) -> `Default` (the plain ARROW, never a pointing
//!    hand — macOS menus/lists use the arrow throughout; a hand is reserved
//!    for an actual hyperlink, which awl has none of).
//! 4. everywhere else (margins, the overlay scrim, the gutter) -> `Default`.
//!
//! **Determinism:** LIVE-APP-ONLY, exactly like `pointer_hide` — the headless
//! capture has no window and no OS pointer to shape, so nothing here is
//! reachable from the capture path and a `--screenshot` needs no new sidecar
//! field (there is nothing deterministic to report: the OS cursor glyph never
//! renders into the PNG). See `App::sync_cursor_icon` (`app/input.rs`) for the
//! live wiring; the actual on-screen SHAPE appearing is flagged there for
//! human confirmation.

use winit::window::CursorIcon;

/// The hover-context inputs the priority decision reads. Each flag is computed
/// by the live `App` from the SAME hit-test geometry the rest of the mouse
/// handling already uses (`TextPipeline::page_resize_hover`,
/// `TextPipeline::over_writing_column`, `self.overlay.is_some()`,
/// `self.page_resizing`) — this struct never invents its own geometry
/// (merge-don't-align: one set of hit regions, read from here and from the
/// click/drag handlers alike).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CursorContext {
    /// A page-column-edge WIDTH DRAG is in progress right now (button held on
    /// the divider, tracking the pointer).
    pub dragging_edge: bool,
    /// A summoned overlay (palette / picker / the spell-suggest panel) is
    /// open — its scrim covers the document, so the pointer is never "over
    /// text" while this is set, regardless of where it geometrically sits.
    pub overlay_open: bool,
    /// The pointer is hovering (not yet dragging) a draggable page-column edge.
    pub over_edge: bool,
    /// The pointer is over the writing column's document text.
    pub over_text: bool,
}

/// THE priority decision: hover context -> OS cursor icon. Pure, so it is
/// exhaustively unit-testable without a window. Priority, highest first:
/// 1. an ACTIVE edge drag always wins — the resize glyph tracks the gesture
///    the user is literally performing, regardless of anything else;
/// 2. a summoned overlay's scrim wins next — it visually covers everything
///    beneath it, the page edge included;
/// 3. hovering a page-column edge (not yet dragging) still beats plain text;
/// 4. plain document text gets the I-beam;
/// 5. everywhere else (margins, scrim, gutter) is the plain arrow.
pub fn cursor_icon_for(ctx: CursorContext) -> CursorIcon {
    if ctx.dragging_edge {
        CursorIcon::ColResize
    } else if ctx.overlay_open {
        CursorIcon::Default
    } else if ctx.over_edge {
        CursorIcon::ColResize
    } else if ctx.over_text {
        CursorIcon::Text
    } else {
        CursorIcon::Default
    }
}

/// Whether (and to what) the OS `set_cursor` call should actually fire, given
/// the previously CACHED icon and the freshly decided one. `None` means no
/// call: either nothing changed, or the OS pointer is currently HIDDEN
/// (typing auto-hide, `pointer_hide::PointerHide::Hidden`) so there is
/// nothing visible to update. The caller does NOT advance its cache in that
/// case either — so the OS's real last-set icon and the cache stay in lockstep
/// (an invariant: the cache always equals the last icon actually handed to
/// `set_cursor`), and the very next un-hide (a `CursorMoved` always recomputes
/// context before this check, and any mouse motion un-hides — see
/// `pointer_hide::on_mouse_move`) compares the FRESH desired icon against that
/// still-accurate cache and fires exactly once if it truly differs — landing
/// directly in the context-correct shape instead of a stale one from before
/// the hide. Mirrors `pointer_hide::os_visibility_change`'s "only call on an
/// actual boundary" discipline, one door over for the icon instead of the
/// visibility bit.
pub fn cursor_icon_change(prev: CursorIcon, next: CursorIcon, hidden: bool) -> Option<CursorIcon> {
    if hidden || prev == next {
        None
    } else {
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(dragging_edge: bool, overlay_open: bool, over_edge: bool, over_text: bool) -> CursorContext {
        CursorContext { dragging_edge, overlay_open, over_edge, over_text }
    }

    // --- cursor_icon_for: the base four mapping cases, nothing else set -----

    #[test]
    fn nothing_hovered_is_the_plain_arrow() {
        assert_eq!(cursor_icon_for(ctx(false, false, false, false)), CursorIcon::Default);
    }

    #[test]
    fn plain_document_text_is_the_i_beam() {
        assert_eq!(cursor_icon_for(ctx(false, false, false, true)), CursorIcon::Text);
    }

    #[test]
    fn hovering_the_page_edge_is_col_resize() {
        assert_eq!(cursor_icon_for(ctx(false, false, true, false)), CursorIcon::ColResize);
    }

    #[test]
    fn overlay_open_alone_is_the_plain_arrow() {
        assert_eq!(cursor_icon_for(ctx(false, true, false, false)), CursorIcon::Default);
    }

    #[test]
    fn dragging_the_edge_alone_is_col_resize() {
        assert_eq!(cursor_icon_for(ctx(true, false, false, false)), CursorIcon::ColResize);
    }

    // --- the priority order, exhaustively (each stated relation + the ------
    // --- full four-way combination) -----------------------------------------

    #[test]
    fn edge_hover_beats_text() {
        assert_eq!(cursor_icon_for(ctx(false, false, true, true)), CursorIcon::ColResize);
    }

    #[test]
    fn overlay_open_beats_text() {
        // The scrim covers the document -- a spot that would otherwise be
        // plain document text still reads as the plain arrow, never the I-beam.
        assert_eq!(cursor_icon_for(ctx(false, true, false, true)), CursorIcon::Default);
    }

    #[test]
    fn dragging_edge_beats_text() {
        assert_eq!(cursor_icon_for(ctx(true, false, false, true)), CursorIcon::ColResize);
    }

    #[test]
    fn overlay_open_beats_edge_hover() {
        // The scrim covers the page edge too -- a would-be edge hover behind
        // an open overlay never shows the resize glyph.
        assert_eq!(cursor_icon_for(ctx(false, true, true, false)), CursorIcon::Default);
    }

    #[test]
    fn dragging_edge_beats_overlay_open() {
        // An ACTIVE drag (button down, mid-gesture) always wins -- it is never
        // masked by a summoned overlay appearing mid-drag.
        assert_eq!(cursor_icon_for(ctx(true, true, false, false)), CursorIcon::ColResize);
    }

    #[test]
    fn overlay_open_beats_edge_hover_and_text_together() {
        assert_eq!(cursor_icon_for(ctx(false, true, true, true)), CursorIcon::Default);
    }

    #[test]
    fn dragging_edge_beats_overlay_open_and_text_together() {
        assert_eq!(cursor_icon_for(ctx(true, true, false, true)), CursorIcon::ColResize);
    }

    #[test]
    fn dragging_edge_beats_every_other_flag_at_once() {
        assert_eq!(cursor_icon_for(ctx(true, true, true, true)), CursorIcon::ColResize);
    }

    // --- cursor_icon_change: the "only call on a change, never while hidden" seam

    #[test]
    fn icon_change_is_none_when_unchanged() {
        assert_eq!(cursor_icon_change(CursorIcon::Text, CursorIcon::Text, false), None);
    }

    #[test]
    fn icon_change_fires_on_an_actual_change() {
        assert_eq!(
            cursor_icon_change(CursorIcon::Default, CursorIcon::Text, false),
            Some(CursorIcon::Text)
        );
    }

    #[test]
    fn icon_change_is_suppressed_while_the_os_pointer_is_hidden() {
        // Typing hid the pointer; the context changed underneath it (e.g. a
        // click landed and moved the caret under a different hover region) --
        // no OS call fires, since nothing is visible to update.
        assert_eq!(cursor_icon_change(CursorIcon::Default, CursorIcon::Text, true), None);
    }

    #[test]
    fn icon_change_resumes_correctly_the_instant_the_pointer_is_visible_again() {
        // Simulates the seam `App::sync_cursor_icon` rides: while hidden, the
        // caller does NOT advance its cache (`prev` stays the last genuinely
        // -drawn icon); the next un-hide call (hidden = false) then sees the
        // real prev-vs-next gap and fires exactly once, landing on the
        // context-correct shape rather than a stale intermediate one.
        assert_eq!(cursor_icon_change(CursorIcon::Default, CursorIcon::Text, true), None);
        assert_eq!(
            cursor_icon_change(CursorIcon::Default, CursorIcon::Text, false),
            Some(CursorIcon::Text)
        );
    }
}
