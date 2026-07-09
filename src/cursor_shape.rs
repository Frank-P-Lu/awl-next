//! src/cursor_shape.rs — CONTEXT-AWARE OS pointer shapes: winit draws the
//! actual glyph (`Window::set_cursor(CursorIcon::…)`); this module is the ONE
//! pure priority decision (hover context -> `CursorIcon`) plus the tiny
//! "only call on an actual change" cache logic, so `App` never has to
//! re-derive the priority ad hoc at each mouse call site — the same
//! single-owner discipline as `syn_role_color` / `pointer_hide::os_visibility_change`.
//!
//! **The mapping** (macOS convention, not the web one — settled taste call),
//! priority highest-first:
//! 1. over the draggable PAGE-COLUMN EDGE, or while actively dragging it ->
//!    `ColResize` (↔).
//! 2. over ANY summoned overlay's clickable ROWS (Command-P / go-to / browse /
//!    theme / history / keybindings / spell / … — every faceting/list picker) OR
//!    a clickable LENS-STRIP facet label (Time/Register/… — every FACETING
//!    picker's strip) -> `Pointer` (the pointing hand — a clickable-affordance
//!    signal). This GENERALIZES the former spell-suggest-only override to every
//!    picker row (and now the strip): a row or facet you can click to act on
//!    earns the hand, uniformly.
//! 3. over the overlay's QUERY-INPUT line (the editable filter field at the top
//!    of a flat/nav/theme picker) -> `Text` (I-beam — it is a text field you type
//!    into, so it reads like one).
//! 4. over any OTHER part of a summoned OVERLAY (its scrim, foot hint, empty
//!    gaps) -> `Default` (the plain ARROW — macOS menus/lists use the arrow for
//!    dead space; the hand is reserved for an actual clickable row).
//! 4b. over the awl-rendered WEB/LINUX MENU BAR: a clickable TITLE / dropdown ITEM ->
//!    `Pointer` (hand); dead bar/dropdown space -> `Default` (arrow, over the doc it covers).
//! 5. over the TEXT AREA (the writing column, no overlay open) -> `Text` (I-beam).
//! 6. everywhere else (margins, the overlay scrim, the gutter) -> `Default`.
//!
//! **Determinism:** LIVE-APP-ONLY, exactly like `pointer_hide` — the headless
//! capture has no window and no OS pointer to shape, so nothing here is
//! reachable from the capture path and a `--screenshot` needs no new sidecar
//! field (there is nothing deterministic to report: the OS cursor glyph never
//! renders into the PNG). See `App::sync_cursor_icon` (`app/input/mouse.rs`) for the
//! live wiring; the actual on-screen SHAPE appearing is flagged there for
//! human confirmation.

use crate::render::ImageHandle;
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
    /// The pointer is over a CLICKABLE ROW of the currently-summoned overlay —
    /// ANY faceting/list picker (Command-P / go-to / browse / theme / history /
    /// keybindings / spell / …), computed from the SAME `overlay_row_at`
    /// hit-test the pickers use for a click. A clickable row earns the pointing
    /// hand as a clickable-affordance signal. Only ever set while `overlay_open`.
    pub over_clickable_overlay_row: bool,
    /// The pointer is over a CLICKABLE LENS-STRIP facet label of the currently-
    /// summoned FACETING picker (Theme / Go-to / Browse / Project / Command /
    /// History / Settings — any picker with a lens strip), computed from the
    /// SAME `overlay_lens_at` hit-test the strip's click handling uses
    /// (`overlay_click`). A clickable facet earns the pointing hand, exactly
    /// like a clickable row — the strip is a second clickable-affordance
    /// surface, not a new priority tier. Only ever set while `overlay_open`;
    /// `None`/`false` for a non-faceting picker (no strip drawn, so nothing to
    /// hit) or a picker whose strip is off-screen.
    pub over_clickable_lens: bool,
    /// The pointer is over the overlay's editable QUERY-INPUT line (the filter
    /// field at the top of a flat/nav/theme picker; the spell panel has none).
    /// It is a text field, so it reads as the I-beam. Only ever set while
    /// `overlay_open`.
    pub over_query_input: bool,
    /// The pointer is over a CLICKABLE ROW of the persistent MARGIN OUTLINE (the
    /// opt-in left-margin table-of-contents — NOT an overlay). A row you can click
    /// to jump the caret to that heading earns the pointing hand, exactly like a
    /// picker row. Computed from the outline's OWN row geometry
    /// (`TextPipeline::outline_hit_line`), only ever set while no overlay is open
    /// (an open overlay's scrim covers the outline).
    pub over_outline_row: bool,
    /// The pointer is over a CLICKABLE menu-bar TITLE or an open-dropdown ITEM (the
    /// awl-rendered WEB/LINUX menu bar — NOT an overlay). A title/item you can click to
    /// act earns the pointing hand, exactly like a picker row. Computed from the bar's
    /// OWN hit-test (`TextPipeline::menubar_hand_at`).
    pub over_menu_hand: bool,
    /// The pointer is over the menu bar's own strip OR an open dropdown's card, but NOT
    /// on a clickable title/item — dead chrome space, which reads as the plain ARROW
    /// (never the document I-beam beneath the bar). Ranked ABOVE `over_edge`/`over_text`
    /// (the bar covers them). Computed from `TextPipeline::over_menu_surface`.
    pub over_menu_bar: bool,
    /// An inline-image DRAG-RESIZE is in progress right now (button held on one of an
    /// image's edges/corners, its width tracking the pointer) — `Some(handle)` names
    /// the grabbed edge/corner, whose glyph ([`image_handle_icon`]) tracks the gesture.
    /// The image analogue of `dragging_edge`: an active resize regardless of what sits
    /// beneath (a page-edge drag is the one thing that outranks it; the two are
    /// mutually exclusive). `None` when no image drag is in progress.
    pub image_drag: Option<ImageHandle>,
    /// The pointer is hovering (not yet dragging) one of an inline image's resize
    /// EDGES/CORNERS — `Some(handle)` names which, whose glyph ([`image_handle_icon`])
    /// reads as the resize affordance (↔ for a side, ↕ for top/bottom, ⤡/⤢ for a
    /// corner), exactly like a page-column edge. Computed from the SAME images layout
    /// the `ImageQuadPipeline` draws (`TextPipeline::image_handle_at`), never a parallel
    /// geometry. Ranked with the page edge (below an open overlay's scrim, which covers
    /// the images). `None` when the pointer is over no image border.
    pub image_hover: Option<ImageHandle>,
}

/// The OS cursor glyph for a given inline-image resize HANDLE: a horizontal
/// ↔ for the left/right edges, a vertical ↕ for the top/bottom edges, and a
/// diagonal ⤡ (`NwseResize`, "\") / ⤢ (`NeswResize`, "/") for the corners along
/// each diagonal. THE single owner of the handle→glyph mapping — a no-wildcard
/// `match`, so a new [`ImageHandle`] variant fails to compile until it is mapped
/// here (the same exhaustive-sweep discipline as `cursor_icon_for` itself).
pub fn image_handle_icon(handle: ImageHandle) -> CursorIcon {
    match handle {
        ImageHandle::Left | ImageHandle::Right => CursorIcon::ColResize,
        ImageHandle::Top | ImageHandle::Bottom => CursorIcon::RowResize,
        ImageHandle::TopLeft | ImageHandle::BottomRight => CursorIcon::NwseResize,
        ImageHandle::TopRight | ImageHandle::BottomLeft => CursorIcon::NeswResize,
    }
}

/// THE priority decision: hover context -> OS cursor icon. Pure, so it is
/// exhaustively unit-testable without a window. Priority, highest first:
/// 1. an ACTIVE page-edge drag always wins — the resize glyph tracks the gesture
///    the user is literally performing, regardless of anything else;
/// 2. an ACTIVE image drag-resize wins next — the grabbed edge/corner's own glyph
///    ([`image_handle_icon`]: ↔ side, ↕ top/bottom, ⤡/⤢ corner) tracks that gesture
///    (the two active drags are mutually exclusive; the page-edge drag is arbitrarily
///    ordered first);
/// 3. hovering a clickable menu-bar TITLE / dropdown ITEM gets the pointing HAND —
///    the awl-rendered web/Linux menu bar's clickable-affordance signal, ranked with
///    the other hands (the menu + a summoned overlay are mutually exclusive, so the
///    relative order among the hands never matters, only that a clickable menu
///    surface earns the hand);
/// 3b. hovering ANY clickable overlay ROW *or* a clickable LENS-STRIP facet gets
///    the pointing HAND — the clickable-affordance signal, sitting ABOVE the
///    generic overlay→arrow rule (but still under an in-progress resize drag);
///    the two never geometrically overlap (the strip sits on its own line above
///    the rows), so which one is set never matters, only that either is;
/// 4. hovering the overlay's editable QUERY-INPUT line gets the I-beam — it is
///    a text field, ranked above the generic overlay→arrow but below a row;
/// 5. any other part of a summoned overlay wins next — its scrim visually
///    covers everything beneath it, the page edge + images included → the plain arrow;
/// 5b. dead menu-bar space (the bar strip / an open dropdown's card, off any clickable
///    title/item) → the plain arrow, ranked ABOVE the page edge + text it covers, so the
///    bar reads as chrome not the document beneath it;
/// 6. hovering a page-column edge (not yet dragging) still beats plain text;
/// 7. hovering an inline image's resize EDGE/CORNER gets that handle's glyph — a
///    resize affordance like the page edge, ranked just under it (the page edge wins
///    where a full-width image's border meets the column edge);
/// 8. hovering a clickable MARGIN-OUTLINE row gets the pointing HAND — the same
///    click-to-jump affordance signal as a picker row, below the page edge (the
///    outline lives just inside the column, so the edge grab wins where they meet);
/// 9. plain document text gets the I-beam;
/// 10. everywhere else (margins, scrim, gutter) is the plain arrow.
pub fn cursor_icon_for(ctx: CursorContext) -> CursorIcon {
    if ctx.dragging_edge {
        CursorIcon::ColResize
    } else if let Some(handle) = ctx.image_drag {
        image_handle_icon(handle)
    } else if ctx.over_menu_hand {
        CursorIcon::Pointer
    } else if ctx.over_clickable_overlay_row || ctx.over_clickable_lens {
        CursorIcon::Pointer
    } else if ctx.over_query_input {
        CursorIcon::Text
    } else if ctx.overlay_open {
        CursorIcon::Default
    } else if ctx.over_menu_bar {
        CursorIcon::Default
    } else if ctx.over_edge {
        CursorIcon::ColResize
    } else if let Some(handle) = ctx.image_hover {
        image_handle_icon(handle)
    } else if ctx.over_outline_row {
        CursorIcon::Pointer
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
        CursorContext {
            dragging_edge,
            overlay_open,
            over_edge,
            over_text,
            over_clickable_overlay_row: false,
            over_clickable_lens: false,
            over_query_input: false,
            over_outline_row: false,
            over_menu_hand: false,
            over_menu_bar: false,
            image_drag: None,
            image_hover: None,
        }
    }

    /// A context with an active image-resize drag on `handle`, the analogue of
    /// `dragging_edge`.
    fn ctx_image_drag(handle: ImageHandle, dragging_edge: bool, overlay_open: bool, over_text: bool) -> CursorContext {
        CursorContext {
            dragging_edge,
            overlay_open,
            over_edge: false,
            over_text,
            over_clickable_overlay_row: false,
            over_clickable_lens: false,
            over_query_input: false,
            over_outline_row: false,
            over_menu_hand: false,
            over_menu_bar: false,
            image_drag: Some(handle),
            image_hover: None,
        }
    }

    /// A context hovering (not dragging) an image's resize `handle` — the analogue
    /// of `over_edge`.
    fn ctx_image_handle(handle: ImageHandle, overlay_open: bool, over_edge: bool, over_text: bool) -> CursorContext {
        CursorContext {
            dragging_edge: false,
            overlay_open,
            over_edge,
            over_text,
            over_clickable_overlay_row: false,
            over_clickable_lens: false,
            over_query_input: false,
            over_outline_row: false,
            over_menu_hand: false,
            over_menu_bar: false,
            image_drag: None,
            image_hover: Some(handle),
        }
    }

    /// A context with the margin-outline-row flag set (no overlay — the outline is
    /// margin chrome, hidden behind an overlay's scrim, so the two never co-occur).
    fn ctx_outline(dragging_edge: bool, over_edge: bool, over_text: bool) -> CursorContext {
        CursorContext {
            dragging_edge,
            overlay_open: false,
            over_edge,
            over_text,
            over_clickable_overlay_row: false,
            over_clickable_lens: false,
            over_query_input: false,
            over_outline_row: true,
            over_menu_hand: false,
            over_menu_bar: false,
            image_drag: None,
            image_hover: None,
        }
    }

    /// A context with the clickable-overlay-row flag set, over the (implied open)
    /// overlay — an overlay is always open when a row is hovered.
    fn ctx_row(dragging_edge: bool, over_edge: bool, over_text: bool) -> CursorContext {
        CursorContext {
            dragging_edge,
            overlay_open: true,
            over_edge,
            over_text,
            over_clickable_overlay_row: true,
            over_clickable_lens: false,
            over_query_input: false,
            over_outline_row: false,
            over_menu_hand: false,
            over_menu_bar: false,
            image_drag: None,
            image_hover: None,
        }
    }

    /// A context with the clickable-LENS flag set, over the (implied open) overlay's
    /// facet strip — the analogue of [`ctx_row`] for the strip surface.
    fn ctx_lens(dragging_edge: bool, over_edge: bool, over_text: bool) -> CursorContext {
        CursorContext {
            dragging_edge,
            overlay_open: true,
            over_edge,
            over_text,
            over_clickable_overlay_row: false,
            over_clickable_lens: true,
            over_query_input: false,
            over_outline_row: false,
            over_menu_hand: false,
            over_menu_bar: false,
            image_drag: None,
            image_hover: None,
        }
    }

    /// A context with the query-input flag set, over the (implied open) overlay's
    /// editable filter line.
    fn ctx_query(dragging_edge: bool, over_edge: bool, over_text: bool) -> CursorContext {
        CursorContext {
            dragging_edge,
            overlay_open: true,
            over_edge,
            over_text,
            over_clickable_overlay_row: false,
            over_clickable_lens: false,
            over_query_input: true,
            over_outline_row: false,
            over_menu_hand: false,
            over_menu_bar: false,
            image_drag: None,
            image_hover: None,
        }
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

    // --- the clickable-overlay-row pointing HAND (generalized to EVERY picker) --

    #[test]
    fn any_clickable_overlay_row_is_the_pointing_hand() {
        // Generalized from spell-only: a hovered row of ANY summoned picker
        // (Command-P / go-to / browse / theme / history / spell / …) reads as
        // clickable -- the flag is computed uniformly from `overlay_row_at`.
        assert_eq!(cursor_icon_for(ctx_row(false, false, false)), CursorIcon::Pointer);
    }

    #[test]
    fn a_non_row_overlay_region_is_the_arrow_never_the_hand() {
        // overlay_open with NEITHER the row nor query flag set (the scrim, a foot
        // hint, an empty gap): the plain arrow -- the hand is scoped to a real row.
        assert_eq!(cursor_icon_for(ctx(false, true, false, false)), CursorIcon::Default);
    }

    #[test]
    fn clickable_row_beats_the_generic_overlay_arrow() {
        // overlay_open AND the row flag set: the hand wins over the generic
        // overlay->arrow rule it sits above.
        assert_eq!(cursor_icon_for(ctx_row(false, false, false)), CursorIcon::Pointer);
    }

    #[test]
    fn clickable_row_beats_a_would_be_edge_or_text_beneath_it() {
        // The scrim covers the document, so edge/text beneath a row never
        // surface -- the hand still wins with those flags also set.
        assert_eq!(cursor_icon_for(ctx_row(false, true, true)), CursorIcon::Pointer);
    }

    #[test]
    fn an_active_edge_drag_still_beats_the_clickable_row_hand() {
        // A page-resize drag in progress is the one higher-priority case: it
        // tracks the literal gesture even over a clickable row.
        assert_eq!(cursor_icon_for(ctx_row(true, false, false)), CursorIcon::ColResize);
    }

    #[test]
    fn dragging_edge_beats_the_row_hand_with_every_flag_at_once() {
        assert_eq!(cursor_icon_for(ctx_row(true, true, true)), CursorIcon::ColResize);
    }

    // --- the clickable LENS-STRIP facet also earns the pointing HAND ------------
    // (extends the Batch-3 cursor pass to the strip — the missing surface: rows
    // already got the hand, the strip did not).

    #[test]
    fn a_clickable_lens_facet_is_the_pointing_hand() {
        assert_eq!(cursor_icon_for(ctx_lens(false, false, false)), CursorIcon::Pointer);
    }

    #[test]
    fn clickable_lens_beats_the_generic_overlay_arrow() {
        assert_eq!(cursor_icon_for(ctx_lens(false, false, false)), CursorIcon::Pointer);
    }

    #[test]
    fn clickable_lens_beats_a_would_be_edge_or_text_beneath_it() {
        // The scrim covers the document, so edge/text beneath the strip never
        // surface -- the hand still wins with those flags also set.
        assert_eq!(cursor_icon_for(ctx_lens(false, true, true)), CursorIcon::Pointer);
    }

    #[test]
    fn an_active_edge_drag_still_beats_the_lens_hand() {
        assert_eq!(cursor_icon_for(ctx_lens(true, false, false)), CursorIcon::ColResize);
    }

    #[test]
    fn dragging_edge_beats_the_lens_hand_with_every_flag_at_once() {
        assert_eq!(cursor_icon_for(ctx_lens(true, true, true)), CursorIcon::ColResize);
    }

    #[test]
    fn the_row_hand_and_the_lens_hand_both_resolve_to_the_pointer_if_ever_set_together() {
        // The strip and the rows sit on different lines and never geometrically
        // overlap, but the priority is stated regardless: either flag alone (or
        // both) resolves to the hand -- neither out-ranks the other.
        let both = CursorContext {
            dragging_edge: false,
            overlay_open: true,
            over_edge: false,
            over_text: false,
            over_clickable_overlay_row: true,
            over_clickable_lens: true,
            over_query_input: false,
            over_outline_row: false,
            over_menu_hand: false,
            over_menu_bar: false,
            image_drag: None,
            image_hover: None,
        };
        assert_eq!(cursor_icon_for(both), CursorIcon::Pointer);
    }

    // --- the overlay QUERY-INPUT line reads as an editable text field (I-beam) --

    #[test]
    fn the_overlay_query_input_line_is_the_i_beam() {
        // The editable filter field at the top of a picker: a text field, so the
        // I-beam, not the arrow -- even though an overlay is open.
        assert_eq!(cursor_icon_for(ctx_query(false, false, false)), CursorIcon::Text);
    }

    #[test]
    fn query_input_beats_the_generic_overlay_arrow() {
        // Ranked above the generic overlay->arrow rule (it is a real editable
        // region), below a clickable row.
        assert_eq!(cursor_icon_for(ctx_query(false, false, false)), CursorIcon::Text);
    }

    #[test]
    fn a_clickable_row_outranks_the_query_input_field() {
        // A row and the query line never geometrically overlap, but the priority
        // is stated regardless: a row (were both set) resolves to the hand.
        let both = CursorContext {
            dragging_edge: false,
            overlay_open: true,
            over_edge: false,
            over_text: false,
            over_clickable_overlay_row: true,
            over_clickable_lens: false,
            over_query_input: true,
            over_outline_row: false,
            over_menu_hand: false,
            over_menu_bar: false,
            image_drag: None,
            image_hover: None,
        };
        assert_eq!(cursor_icon_for(both), CursorIcon::Pointer);
    }

    // --- the margin-OUTLINE row pointing HAND (persistent chrome, not an overlay) --

    #[test]
    fn a_margin_outline_row_is_the_pointing_hand() {
        // A hovered clickable outline row reads as click-to-jump — the pointing hand,
        // exactly like a picker row, though the outline is margin chrome not an overlay.
        assert_eq!(cursor_icon_for(ctx_outline(false, false, false)), CursorIcon::Pointer);
    }

    #[test]
    fn a_margin_outline_row_beats_the_plain_text_beneath_it() {
        // The outline sits in the left margin, but its band can overlap where the
        // column starts; a row still wins the hand over plain text.
        assert_eq!(cursor_icon_for(ctx_outline(false, false, true)), CursorIcon::Pointer);
    }

    // --- the WEB/LINUX MENU BAR: title/item = hand, dead bar space = arrow --------

    /// A context over a clickable menu-bar title / dropdown item (the pointing hand).
    fn ctx_menu_hand(over_edge: bool, over_text: bool) -> CursorContext {
        CursorContext {
            dragging_edge: false,
            overlay_open: false,
            over_edge,
            over_text,
            over_clickable_overlay_row: false,
            over_clickable_lens: false,
            over_query_input: false,
            over_outline_row: false,
            over_menu_hand: true,
            over_menu_bar: true, // the hand is always within the bar surface
            image_drag: None,
            image_hover: None,
        }
    }

    /// A context over the menu bar's dead space (strip / dropdown card, no clickable
    /// title or item under the pointer) — the plain arrow.
    fn ctx_menu_bar(over_edge: bool, over_text: bool) -> CursorContext {
        CursorContext {
            dragging_edge: false,
            overlay_open: false,
            over_edge,
            over_text,
            over_clickable_overlay_row: false,
            over_clickable_lens: false,
            over_query_input: false,
            over_outline_row: false,
            over_menu_hand: false,
            over_menu_bar: true,
            image_drag: None,
            image_hover: None,
        }
    }

    #[test]
    fn a_clickable_menu_title_or_item_is_the_pointing_hand() {
        assert_eq!(cursor_icon_for(ctx_menu_hand(false, false)), CursorIcon::Pointer);
    }

    #[test]
    fn a_menu_title_hand_beats_the_text_and_edge_beneath_the_bar() {
        // The bar reserves space over the document; a clickable title still wins the
        // hand over the would-be edge/text under it.
        assert_eq!(cursor_icon_for(ctx_menu_hand(true, true)), CursorIcon::Pointer);
    }

    #[test]
    fn dead_menu_bar_space_is_the_plain_arrow_never_the_i_beam() {
        // Over the bar strip / dropdown card but off any clickable title/item: the
        // plain arrow, NOT the document I-beam that `over_text` would otherwise give.
        assert_eq!(cursor_icon_for(ctx_menu_bar(false, true)), CursorIcon::Default);
    }

    #[test]
    fn dead_menu_bar_space_beats_a_would_be_page_edge_beneath_it() {
        assert_eq!(cursor_icon_for(ctx_menu_bar(true, false)), CursorIcon::Default);
    }

    #[test]
    fn the_page_edge_still_beats_a_margin_outline_row() {
        // The outline hugs just inside the column edge; where the two meet, the
        // page-resize edge (hover or drag) wins — the outline is below it in priority.
        assert_eq!(cursor_icon_for(ctx_outline(false, true, false)), CursorIcon::ColResize);
        assert_eq!(cursor_icon_for(ctx_outline(true, false, false)), CursorIcon::ColResize);
    }

    #[test]
    fn an_active_edge_drag_still_beats_the_query_input_i_beam() {
        assert_eq!(cursor_icon_for(ctx_query(true, false, false)), CursorIcon::ColResize);
    }

    // --- the inline-image resize handles (hover + drag): one glyph per edge/corner

    #[test]
    fn image_handle_icon_maps_each_edge_and_corner_to_its_glyph() {
        // The single owner of the handle->glyph mapping (a no-wildcard match). Sides
        // are ↔, top/bottom are ↕, the "\" diagonal is NwseResize, the "/" is NeswResize.
        assert_eq!(image_handle_icon(ImageHandle::Left), CursorIcon::ColResize);
        assert_eq!(image_handle_icon(ImageHandle::Right), CursorIcon::ColResize);
        assert_eq!(image_handle_icon(ImageHandle::Top), CursorIcon::RowResize);
        assert_eq!(image_handle_icon(ImageHandle::Bottom), CursorIcon::RowResize);
        assert_eq!(image_handle_icon(ImageHandle::TopLeft), CursorIcon::NwseResize);
        assert_eq!(image_handle_icon(ImageHandle::BottomRight), CursorIcon::NwseResize);
        assert_eq!(image_handle_icon(ImageHandle::TopRight), CursorIcon::NeswResize);
        assert_eq!(image_handle_icon(ImageHandle::BottomLeft), CursorIcon::NeswResize);
    }

    #[test]
    fn hovering_each_image_handle_reads_as_that_handles_glyph() {
        // A hover over each edge/corner surfaces its own glyph through cursor_icon_for.
        for (h, want) in [
            (ImageHandle::Left, CursorIcon::ColResize),
            (ImageHandle::Right, CursorIcon::ColResize),
            (ImageHandle::Top, CursorIcon::RowResize),
            (ImageHandle::Bottom, CursorIcon::RowResize),
            (ImageHandle::TopLeft, CursorIcon::NwseResize),
            (ImageHandle::BottomRight, CursorIcon::NwseResize),
            (ImageHandle::TopRight, CursorIcon::NeswResize),
            (ImageHandle::BottomLeft, CursorIcon::NeswResize),
        ] {
            assert_eq!(cursor_icon_for(ctx_image_handle(h, false, false, false)), want);
        }
    }

    #[test]
    fn dragging_each_image_handle_reads_as_that_handles_glyph() {
        for (h, want) in [
            (ImageHandle::Right, CursorIcon::ColResize),
            (ImageHandle::Bottom, CursorIcon::RowResize),
            (ImageHandle::BottomRight, CursorIcon::NwseResize),
            (ImageHandle::TopRight, CursorIcon::NeswResize),
        ] {
            assert_eq!(cursor_icon_for(ctx_image_drag(h, false, false, false)), want);
        }
    }

    #[test]
    fn an_image_handle_hover_beats_plain_text_beneath_it() {
        // The handle sits on the image's border, inside the writing column; the
        // resize affordance still wins over the plain-text I-beam under it.
        assert_eq!(
            cursor_icon_for(ctx_image_handle(ImageHandle::BottomRight, false, false, true)),
            CursorIcon::NwseResize
        );
    }

    #[test]
    fn an_open_overlay_scrim_beats_an_image_handle_hover() {
        // The overlay's scrim covers the images too — a would-be handle hover
        // behind an open overlay reads as the plain arrow, never the resize glyph.
        assert_eq!(
            cursor_icon_for(ctx_image_handle(ImageHandle::Right, true, false, false)),
            CursorIcon::Default
        );
    }

    #[test]
    fn a_page_edge_hover_beats_an_image_handle_hover() {
        // Both are resize affordances; where they meet (an image near the column
        // edge), the page edge is ranked higher, so it wins.
        assert_eq!(
            cursor_icon_for(ctx_image_handle(ImageHandle::Right, false, true, false)),
            CursorIcon::ColResize
        );
    }

    #[test]
    fn an_active_page_edge_drag_still_beats_an_active_image_drag() {
        // The two active drags are mutually exclusive in practice, but the priority
        // is stated: were both set, the page-edge drag is ordered first.
        assert_eq!(
            cursor_icon_for(ctx_image_drag(ImageHandle::BottomRight, true, false, false)),
            CursorIcon::ColResize
        );
    }

    #[test]
    fn an_active_image_drag_beats_an_open_overlay() {
        // Like a page-edge drag, an in-progress image resize tracks the literal
        // gesture even if a summoned overlay appears mid-drag.
        assert_eq!(
            cursor_icon_for(ctx_image_drag(ImageHandle::Top, false, true, false)),
            CursorIcon::RowResize
        );
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
