//! DOCUMENT GEOMETRY — the read-only spatial query layer that turns the shaped
//! buffer into pixels and back: the centered PAGE-MODE writing column (its width /
//! left edge / text inset / wrap width), the scroll<->pixel mapping (`doc_top`, the
//! per-row top/height delegated to [`rowgeom::RowGeom`], the variable-row-height
//! `max_scroll_rows` / `scroll_to_show_row` / `scroll_to_center_row`), the
//! wrap-aware visual-row model (`visual_rows`, `visual_row_of`, `visual_row_top`,
//! `total_visual_rows`), the per-glyph advance maps (`line_glyph_xs`,
//! `col_x_and_advance`, the cursor row height/scale), and the pixel->`(line,col)`
//! hit test. The free helpers (`visible_lines`, `clamp_scroll`, `max_scroll`,
//! `column_width_for` / `column_left_for`, `pick_row`, `row_x_span`,
//! `assemble_glyph_xs`) are the pure, GPU-free math these read.
//!
//! Like [`caret`] and [`chrome`], the methods stay inherent on [`super::TextPipeline`]
//! (they read its buffer / metrics / scroll state heavily), so this module is purely
//! a physical home for that cohesive cluster, carved out of `render.rs` VERBATIM. A
//! child module sees its ancestor's private items, so the methods keep full access to
//! `TextPipeline`'s private fields with NO behaviour change — the capture output is
//! byte-identical. The two app-facing free fns (`hit_test`, `visible_lines_z`) stay
//! re-exported from `render` so `render::hit_test` / `render::visible_lines_z` resolve
//! unchanged.

use super::*;

/// Compute how many text lines fit in `height` pixels at the DEFAULT line
/// height (zoom 1.0). Kept for the existing tests + zoom-1 callers.
#[allow(dead_code)]
pub fn visible_lines(height: f32) -> usize {
    visible_lines_z(height, LINE_HEIGHT)
}

/// Zoom-aware variant: how many lines of `line_height` fit in `height` pixels.
pub fn visible_lines_z(height: f32, line_height: f32) -> usize {
    ((height - TEXT_TOP) / line_height).floor().max(1.0) as usize
}

/// Given the cursor line and current scroll, return a (possibly updated) scroll
/// so the cursor stays on screen (zoom 1.0 line height).
#[allow(dead_code)]
pub fn clamp_scroll(scroll_lines: usize, cursor_line: usize, height: f32) -> usize {
    clamp_scroll_z(scroll_lines, cursor_line, height, LINE_HEIGHT)
}

/// Zoom-aware cursor-follow scroll clamp, in the NON-WRAP model where the scroll
/// unit is a logical line (== a visual row when nothing wraps). The live app now
/// does cursor-follow in VISUAL rows (using the cursor's wrap-aware visual row),
/// but this is retained as the documented non-wrap reference + tested invariant:
/// when nothing wraps, `cursor_line` IS the cursor's visual row, so this matches.
#[allow(dead_code)]
pub fn clamp_scroll_z(
    scroll_lines: usize,
    cursor_line: usize,
    height: f32,
    line_height: f32,
) -> usize {
    let rows = visible_lines_z(height, line_height);
    let mut scroll = scroll_lines;
    if cursor_line < scroll {
        scroll = cursor_line;
    } else if cursor_line >= scroll + rows {
        scroll = cursor_line + 1 - rows;
    }
    scroll
}

/// Maximum free-scroll offset, measured in VISUAL ROWS, in the UNIFORM-height
/// model. The LIVE path now computes this VARIABLE-row-height aware on the pipeline
/// ([`TextPipeline::max_scroll_rows`]) because a heading row is taller than a body
/// row; this free function is retained as the documented uniform REFERENCE + the
/// tested overscroll-semantics invariant (a doc that fits can't scroll; otherwise
/// the last row can rise to the top, bounded by [`OVERSCROLL_KEEP_ROWS`]). When all
/// rows ARE a uniform `line_height` (no headings), the pipeline method agrees with
/// this exactly. `total_visual_rows` counts every soft-wrapped continuation row.
#[allow(dead_code)]
pub fn max_scroll(total_visual_rows: usize, height: f32, line_height: f32) -> usize {
    let visible = visible_lines_z(height, line_height);
    // Base: scroll until the last visual row reaches the BOTTOM of the viewport.
    let base = total_visual_rows.saturating_sub(visible);
    // A doc that fully fits the viewport has nothing pinned to the bottom, so it
    // gets no overscroll (it can't scroll content into the void).
    if base == 0 {
        return 0;
    }
    // "Scroll past end": add up to one screenful of overscroll, capped so at least
    // OVERSCROLL_KEEP_ROWS of the document's last rows stay on screen. With the
    // default keep of 1 this resolves to `total_visual_rows - 1` (last row at top).
    let overscroll = visible.saturating_sub(OVERSCROLL_KEEP_ROWS);
    base + overscroll
}

/// Pixel -> text hit-test. Given a click at `(px, py)` in physical pixels, the
/// current `scroll_lines`, the zoom `metrics`, and the column's `left` edge,
/// return the (line, col) the click maps to.
/// `line = scroll + floor((py - TEXT_TOP) / line_height)`;
/// `col = round((px - left) / char_width)`, both clamped to be >= 0. `left` is
/// the centered PAGE-MODE column left (or `TEXT_LEFT` edge-to-edge). The caller
/// clamps `line`/`col` to the actual buffer (via `line_col_to_char`), since this
/// function does not know the document. Mirrors EXACTLY the layout math used to
/// place glyphs + the caret, so a click lands on the right glyph.
pub fn hit_test(px: f32, py: f32, scroll_lines: usize, metrics: &Metrics, left: f32) -> (usize, usize) {
    let rel_y = (py - TEXT_TOP).max(0.0);
    let line = scroll_lines + (rel_y / metrics.line_height).floor() as usize;
    let rel_x = (px - left).max(0.0);
    // round() so a click on the right half of a glyph lands AFTER it (natural
    // caret placement), matching how editors snap to the nearer gap.
    let col = (rel_x / metrics.char_width).round() as usize;
    (line, col)
}

/// PAGE MODE responsive-collapse padding: the SMALL uniform inset kept on EACH side
/// once the window is too NARROW to seat the full measure with room to spare. Equal
/// to [`TEXT_LEFT`] so a squeezed page column collapses to the SAME inset as
/// edge-to-edge mode — the margins (and with them the bottom-left gutter and the
/// gradient pattern band, which both gate on having margin ROOM) fall to ~0 and the
/// writing runs effectively edge-to-edge instead of being strangled into a sliver.
pub const PAGE_MIN_PAD: f32 = TEXT_LEFT;

/// PAGE MODE column glyph ADVANCE (px): the char advance that DRIVES the page
/// column's pixel width — the base advance at zoom 1.0, still DPI-scaled, with the
/// user ZOOM divided back out. `char_width` is the LIVE (zoomed × DPI) advance
/// `metrics.char_width` (= `CHAR_WIDTH * zoom * dpi`); dividing by `zoom` recovers
/// `CHAR_WIDTH * dpi`, which depends on the DISPLAY only, never on the user zoom.
///
/// This is THE seam that DECOUPLES zoom from the page width: the column pixel width
/// (see [`column_width_for`]) is `measure * this`, so it tracks the WINDOW + the
/// settable measure but is INVARIANT under zoom. Zooming then only scales the glyph
/// metrics that SHAPE/wrap text INSIDE the fixed column — bigger glyphs, FEWER chars
/// per line, but the page surface + gutter + margins stay put. (Previously the column
/// used the zoomed advance directly, so zooming IN grew `measure_px` past the window
/// cap and collapsed the margins — the gutter vanished. This strips the zoom.)
///
/// At zoom 1.0 (the deterministic capture path) this is an IDENTITY, so wide captures
/// stay byte-identical.
pub fn page_column_advance(char_width: f32, zoom: f32) -> f32 {
    if zoom > 0.0 {
        char_width / zoom
    } else {
        char_width
    }
}

/// PAGE MODE column WIDTH (px) for a given window width + ZOOM-INDEPENDENT glyph
/// advance (see [`page_column_advance`]) + page state + measure. The single source
/// of truth, factored out of [`TextPipeline::column_width`] so it is unit-testable
/// without a GPU device. NOTE: `char_width` here is the PAGE-COLUMN advance
/// ([`page_column_advance`]), NOT the live zoomed `metrics.char_width` — feeding the
/// zoom-stripped advance is what keeps the column (and its margins + gutter) constant
/// across zoom levels.
///
/// Edge-to-edge (`page_on == false`): the plain content width
/// `window - 2*NONPAGE_INSET` (a slightly wider side inset than page's collapse
/// floor, so a tad more ground shows). Page mode on, ONE responsive formula — no mode toggle,
/// smooth across a resize. The side margin is the GENEROUS [`page_min_margin`] when
/// the window has room for it, but it COLLAPSES toward the small uniform
/// [`PAGE_MIN_PAD`] as the measure crowds the width, so the column is:
///
/// ```text
/// margin = clamp((window - measure_px)/2, PAGE_MIN_PAD, page_min_margin(window))
/// column = min(measure_px, window - 2*margin)             // centered
/// ```
///
/// * WIDE window (room for the measure plus a generous band): the margin sits at the
///   generous `page_min_margin`, the column sits at the target measure
///   (`measure * char_width`), and the leftover splits into MARGINS — the gradient
///   pattern band and the gutter both show.
/// * NARROW window (the measure ≈ or exceeds the width): the margin collapses to the
///   small [`PAGE_MIN_PAD`] and the column FILLS the width minus that pad, so the
///   margins fall to ~0, the gutter + patterns auto-hide, and the page runs
///   effectively edge-to-edge.
///
/// (Previously the cap was the generous `page_min_margin` even at the full measure;
/// that over-reserved on narrow windows and squeezed the text into a sliver. Letting
/// the margin collapse fixes that while leaving WIDE captures — where the measure
/// binds well inside the available width — byte-identical.)
pub fn column_width_for(window_w: f32, char_width: f32, page_on: bool, measure: usize) -> f32 {
    let edge = (window_w - 2.0 * NONPAGE_INSET).max(1.0);
    if !page_on {
        return edge;
    }
    let measure_px = measure as f32 * char_width;
    // The side margin shrinks from the generous band down to the small uniform pad as
    // the measure crowds the window: WIDE -> page_min_margin, NARROW -> PAGE_MIN_PAD.
    let margin = ((window_w - measure_px) * 0.5).clamp(PAGE_MIN_PAD, page_min_margin(window_w));
    let avail = (window_w - 2.0 * margin).max(1.0);
    measure_px.min(avail).max(1.0)
}

/// PAGE MODE column LEFT edge (px). Edge-to-edge this is the fixed `NONPAGE_INSET`
/// origin (the plain writing-column inset). Page mode on, the column is CENTERED in the window,
/// floored at [`PAGE_MIN_PAD`] so it never crosses the left edge (when the window is
/// narrow and the column fills, the centered left lands exactly at that pad). Every
/// origin-derived x adds this. Factored out (with [`column_width_for`]) for testing.
pub fn column_left_for(window_w: f32, char_width: f32, page_on: bool, measure: usize) -> f32 {
    if !page_on {
        return NONPAGE_INSET;
    }
    let w = column_width_for(window_w, char_width, page_on, measure);
    ((window_w - w) * 0.5).max(PAGE_MIN_PAD)
}

/// DIRECT-MANIPULATION page resize: how close (px) the pointer must come to a page
/// column's surface EDGE for the horizontal-resize affordance to arm — the cursor
/// flips to a resize glyph and a press begins a width drag. A few px, awl-minimal:
/// there is NO visible handle, the proximity zone IS the affordance.
pub const PAGE_RESIZE_GRAB_PX: f32 = 6.0;

/// A glyph cell whose advance is below this fraction of `metrics.char_width` is
/// DEGENERATE — a collapsed / glyphless mid-line cell rather than a real narrow
/// glyph. The canonical case is the SPACE at a soft-wrap boundary: cosmic-text
/// collapses the trailing whitespace at the break, so the cell's two x boundaries
/// coincide at the row's right edge and its raw width is ~0 (the block-caret
/// "1px sliver" bug). [`TextPipeline::col_x_and_advance`] rescues such a cell to
/// the default `char_width`, exactly like its end-of-line fallback. The fraction
/// is deliberately tiny relative to any REAL advance — the narrowest genuine
/// glyphs (a proportional `i`/`l` ≈ 0.25em, even a hair space ≈ 0.1em) sit well
/// above it at every zoom (both sides scale with zoom × dpi), so only truly
/// collapsed cells are rescued and thin glyphs keep their exact advance.
pub(super) const DEGENERATE_CELL_FRAC: f32 = 0.1;

/// Which page-column surface EDGE the pointer is hovering, for the drag-to-resize
/// affordance. The width math is symmetric about center so the drag itself does not
/// need the side, but the hover test reports it for precision (and testability).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResizeEdge {
    Left,
    Right,
}

/// Is `pointer_x` within `tol` px of a page column's LEFT or RIGHT surface edge?
/// (`column_left` .. `column_left + column_width`.) Returns the NEARER edge when both
/// are in reach (a very narrow column), else `None`. Pure — the caller gates on page
/// mode being on WITH real margin room; this only does the proximity geometry.
pub fn page_boundary_hit(
    pointer_x: f32,
    column_left: f32,
    column_width: f32,
    tol: f32,
) -> Option<ResizeEdge> {
    let right = column_left + column_width;
    let dl = (pointer_x - column_left).abs();
    let dr = (pointer_x - right).abs();
    if dl <= tol && dl <= dr {
        Some(ResizeEdge::Left)
    } else if dr <= tol {
        Some(ResizeEdge::Right)
    } else {
        None
    }
}

/// CURSOR SHAPE — is `pointer_x` within a column's horizontal extent
/// (`column_left` .. `column_left + column_width`, inclusive of both edges)?
/// The membership counterpart to [`page_boundary_hit`]'s proximity test: pure,
/// so the "is the pointer over document TEXT" half of the context-aware OS
/// cursor (`cursor_shape::CursorContext::over_text`,
/// `TextPipeline::over_writing_column`) is unit-testable without a GPU.
pub fn in_writing_column(pointer_x: f32, column_left: f32, column_width: f32) -> bool {
    pointer_x >= column_left && pointer_x <= column_left + column_width
}

/// The page MEASURE (chars) implied by dragging a column edge to `pointer_x`. The
/// column is CENTERED, so the grabbed edge's distance from the window center is HALF
/// the column width; the full width is twice that, and dividing by the ZOOM-STRIPPED
/// `advance` (see [`page_column_advance`]) converts px back to chars. ABSOLUTE /
/// direct-manipulation: the edge tracks the pointer (dragging OUT widens, IN narrows),
/// symmetric on both sides. Clamped to the settable band [`crate::page::MIN_MEASURE`]
/// ..= [`crate::page::MAX_MEASURE`] so a drag can never exceed the width the keyboard
/// commands reach. Zoom-independent because `advance` already has the zoom divided out.
pub fn page_resize_measure(window_w: f32, advance: f32, pointer_x: f32) -> usize {
    let center = window_w * 0.5;
    let half = (pointer_x - center).abs();
    let width = (2.0 * half).max(1.0);
    let measure = if advance > 0.0 { (width / advance).round() } else { 0.0 };
    (measure.max(0.0) as usize).clamp(crate::page::MIN_MEASURE, crate::page::MAX_MEASURE)
}

/// INLINE-IMAGE drag-resize: how close (px) the pointer must come to an image's
/// EDGE or CORNER for the resize affordance to arm — a small tolerance around the
/// image's border, the standard direct-manipulation resize band. A few px larger
/// than the page-column edge zone since a corner is a smaller target than a
/// full-height edge. Like [`PAGE_RESIZE_GRAB_PX`], there is no visible handle glyph
/// — the proximity band IS the affordance.
///
/// TASTE TUNABLE (flagged for live review): the grab width; a corner target is
/// smaller than a full edge, so it's a touch wider than the page-edge zone.
pub const IMAGE_RESIZE_GRAB_PX: f32 = 12.0;

/// The MINIMUM display width (px) a drag can shrink an inline image to — a floor so
/// a drag can never collapse the image to nothing (and pairs with the fit-to-column
/// MAX, the text wrap width). Companion to [`crate::page::MIN_MEASURE`] for images.
///
/// TASTE TUNABLE (flagged for live review): the clamp floor. Matches the task's
/// stated `[64, column width]` band — a `|64` hint is the smallest an image can be
/// dragged to; the ceiling is the writing column width (fit-to-column).
pub const MIN_IMAGE_W: f32 = 64.0;

/// Which HANDLE (edge or corner) of an inline image the pointer is over, for the
/// drag-to-resize affordance. A resize can grab ANY of the four edges or four
/// corners; each maps to its own OS cursor glyph + drag-drive axis
/// (`cursor_shape::image_handle_icon`; [`image_resize_width`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImageHandle {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Is `pointer` within `tol` px of an EDGE or CORNER of an image whose on-screen
/// rect is `image_rect` = `[left, top, w, h]`? Returns which handle (edge/corner)
/// the pointer grabs, CORNERS FIRST (a corner is the intersection of two edges, so
/// its diagonal grip wins over either edge where they meet). An edge only arms
/// within the perpendicular SPAN of the image (plus `tol` slop), so a pointer far
/// above/below the left edge never arms it. Pure — the caller supplies the rect from
/// the SAME images layout the `ImageQuadPipeline` draws + the sidecar reports (no
/// parallel geometry), and gates on the feature being on; this only does the border
/// proximity. The proximity counterpart to [`page_boundary_hit`], unit-testable
/// without a GPU.
pub fn image_handle_hit(pointer: (f32, f32), image_rect: [f32; 4], tol: f32) -> Option<ImageHandle> {
    let [left, top, w, h] = image_rect;
    let (px, py) = pointer;
    let right = left + w;
    let bottom = top + h;
    let near_l = (px - left).abs() <= tol;
    let near_r = (px - right).abs() <= tol;
    let near_t = (py - top).abs() <= tol;
    let near_b = (py - bottom).abs() <= tol;
    // Within the edge's perpendicular span (with `tol` slop): so a side edge only
    // arms alongside the image, never far past its top/bottom (and vice-versa).
    let in_x = px >= left - tol && px <= right + tol;
    let in_y = py >= top - tol && py <= bottom + tol;
    // Corners first — a corner box is the intersection of two edge bands, and its
    // diagonal grip must win over either edge there.
    if near_l && near_t {
        Some(ImageHandle::TopLeft)
    } else if near_r && near_t {
        Some(ImageHandle::TopRight)
    } else if near_l && near_b {
        Some(ImageHandle::BottomLeft)
    } else if near_r && near_b {
        Some(ImageHandle::BottomRight)
    } else if near_l && in_y {
        Some(ImageHandle::Left)
    } else if near_r && in_y {
        Some(ImageHandle::Right)
    } else if near_t && in_x {
        Some(ImageHandle::Top)
    } else if near_b && in_x {
        Some(ImageHandle::Bottom)
    } else {
        None
    }
}

/// The aspect-locked width contribution of a CORNER drag: the orthogonal PROJECTION
/// of the pointer's growth `(gx, gy)` from the anchored (opposite) corner onto the
/// image's own diagonal `(w, h)`. Reduces to `t·w` when the pointer stays exactly on
/// the diagonal (`gx = t·w, gy = t·h`), so a straight diagonal drag maps 1:1 to size;
/// off-diagonal motion blends both axes. Degenerate `(w,h) == (0,0)` falls back to the
/// larger raw growth. Pure — the corner arms of [`image_resize_width`] call this.
fn diagonal_width(gx: f32, gy: f32, w: f32, h: f32) -> f32 {
    let denom = w * w + h * h;
    if denom <= 0.0 {
        return gx.max(gy);
    }
    w * (gx * w + gy * h) / denom
}

/// The new DISPLAY WIDTH (px) an inline image gets from dragging one of its edges or
/// corners (`handle`) to `pointer`, given the image's PRESS-TIME on-screen `rect`
/// `[left, top, w, h]`. Direct manipulation, ALWAYS aspect-locked (only a width is
/// ever produced — the height rides the fixed aspect, so no distortion): the OPPOSITE
/// edge/corner is the fixed anchor and the grabbed one tracks the pointer.
///   * left/right edges — the pointer's `x` distance past the anchored edge drives.
///   * top/bottom edges — the pointer's `y` distance past the anchored edge drives,
///     converted to a width through the fixed aspect (`w/h`).
///   * corners — the diagonal projection ([`diagonal_width`]) of the pointer's growth
///     from the anchored corner drives.
/// Clamped to `[min, wrap]`: never below [`MIN_IMAGE_W`] and never past the
/// writing-column `wrap` width (the fit-to-column ceiling — the same bound v1's
/// [`super::spans::image_display_size`] enforces). Pure, so the px→width mapping is
/// unit-testable without a GPU.
pub fn image_resize_width(handle: ImageHandle, rect: [f32; 4], pointer: (f32, f32), wrap: f32, min: f32) -> f32 {
    let [left, top, w, h] = rect;
    let (px, py) = pointer;
    let right = left + w;
    let bottom = top + h;
    // Fixed aspect (w per unit h); a degenerate zero height falls back to square.
    let aspect = if h > 0.0 { w / h } else { 1.0 };
    let raw = match handle {
        ImageHandle::Right => px - left,
        ImageHandle::Left => right - px,
        ImageHandle::Bottom => (py - top) * aspect,
        ImageHandle::Top => (bottom - py) * aspect,
        ImageHandle::BottomRight => diagonal_width(px - left, py - top, w, h),
        ImageHandle::TopLeft => diagonal_width(right - px, bottom - py, w, h),
        ImageHandle::TopRight => diagonal_width(px - left, bottom - py, w, h),
        ImageHandle::BottomLeft => diagonal_width(right - px, py - top, w, h),
    };
    raw.clamp(min, wrap.max(min))
}

/// Choose the visual row of `rows` that owns char column `col`. A column is owned
/// by the row whose `[start_col, end_col)` contains it; at a wrap boundary the
/// column equals both the previous row's `end_col` and the next row's
/// `start_col`, and the NEXT (lower) row wins — that is where the caret sits when
/// you move onto a wrapped continuation. Past the logical end-of-line (col ==
/// last row's end_col with no following row) the LAST row is used. `rows` is
/// never empty (see [`TextPipeline::visual_rows`]).
pub(super) fn pick_row<'r>(rows: &'r [VisualRow], col: usize) -> &'r VisualRow {
    &rows[pick_row_index(rows, col)]
}

/// The INDEX form of [`pick_row`]: the position within `rows` of the visual row
/// that owns char column `col`, with the identical wrap-boundary bias (the later
/// row wins at a boundary). Used by the visual-motion oracle to step to the
/// adjacent (up/down) row, while [`pick_row`] keeps returning the reference its
/// existing callers want. `rows` is never empty (see [`TextPipeline::visual_rows`]).
pub(super) fn pick_row_index(rows: &[VisualRow], col: usize) -> usize {
    // First, a row that strictly contains the column in its half-open span: this
    // also resolves the wrap boundary in favor of the later row (its start_col).
    for (i, r) in rows.iter().enumerate() {
        if col >= r.start_col && col < r.end_col {
            return i;
        }
    }
    // No strict container: the column is at/after some row's end_col. Use the
    // last row whose start_col <= col (the row the position trails), defaulting to
    // the final row for an end-of-line column.
    rows.iter()
        .enumerate()
        .rev()
        .find(|(_, r)| col >= r.start_col)
        .map(|(i, _)| i)
        .unwrap_or(rows.len().saturating_sub(1))
}

/// The pixel `(x, width)` of a `[s, e)` char-column span on one visual `row`,
/// from that row's own x boundaries (`xs[s]`..`xs[e]`, offset by `text_left`). The
/// width is floored at `min_w` so a zero-width span still shows a sliver where the
/// caller wants one. `s`/`e` must already be clamped to the row's column count.
/// Shared by the squiggle / selection / preedit rect builders.
pub(super) fn row_x_span(row: &VisualRow, text_left: f32, s: usize, e: usize, min_w: f32) -> (f32, f32) {
    // Belt-and-suspenders: every current caller clamps `s`/`e` to the row's column
    // count, so these indices are in range today. Read through `.get` anyway so a
    // future mis-clamping caller degrades to a benign zero instead of panicking —
    // behavior-identical for all in-range accesses.
    let xs_s = row.xs.get(s).copied().unwrap_or(0.0);
    let xs_e = row.xs.get(e).copied().unwrap_or(xs_s);
    let x = text_left + xs_s;
    let w = (xs_e - xs_s).max(min_w);
    (x, w)
}

/// Assemble ONE [`VisualRow`] from a shaped layout `run` of the logical line whose
/// text is `line_text` — the per-run body shared VERBATIM by
/// [`TextPipeline::visual_rows`] and [`TextPipeline::visual_rows_for_lines`], so
/// the two sources produce byte-identical rows. Gathers the run's glyph clusters,
/// maps its byte range onto the full line's char columns (`assemble_glyph_xs`
/// keys off the line text, so the returned vector is char_count+1 long; only
/// columns within this run's byte span carry real x's, the rest are
/// forward-filled — callers index it by GLOBAL char column and clamp to this
/// row's [start_col,end_col]), and carries the run's wrap-aware top/height.
pub(super) fn visual_row_from_run(
    line_text: &str,
    run: &glyphon::cosmic_text::LayoutRun<'_>,
    char_width: f32,
) -> VisualRow {
    let mut clusters: Vec<(usize, usize, f32, f32)> = Vec::new();
    let mut byte_start = usize::MAX;
    let mut byte_end = 0usize;
    for g in run.glyphs.iter() {
        clusters.push((g.start, g.end, g.x, g.x + g.w));
        byte_start = byte_start.min(g.start);
        byte_end = byte_end.max(g.end);
    }
    if byte_start == usize::MAX {
        // A run with no glyphs (e.g. an empty wrapped row): cover nothing.
        byte_start = 0;
        byte_end = 0;
    }
    let xs = assemble_glyph_xs(line_text, &clusters, char_width);
    let start_col = byte_col(line_text, byte_start);
    let end_col = byte_col(line_text, byte_end);
    VisualRow {
        line_top: run.line_top,
        line_height: run.line_height,
        byte_start,
        byte_end,
        start_col,
        end_col,
        xs,
    }
}

/// Build the per-CHAR x boundaries for a line from its shaped glyph CLUSTERS.
///
/// `clusters` are `(start_byte, end_byte, left_x, right_x)` tuples (byte ranges
/// into `line_text`, pixel x's relative to the text left). Returns `char_count+1`
/// boundaries: `xs[col]` is the left edge of the cell at char-column `col`, and
/// `xs[char_count]` is the right edge of the last glyph (end of line).
///
/// This is the core char<->byte + advance mapping for advance-aware layout, kept
/// as a pure free function so the CJK (multi-byte) behavior is unit-testable
/// without a GPU. `char_width` is the fixed-pitch fallback used for empty /
/// glyphless lines.
///
/// LIGATURE CLUSTERS — the general N-source-chars → M-glyphs case. A single
/// `(start_byte, end_byte)` cluster SPAN may be shaped by several glyphs, all
/// stamped with that SAME span:
///   * `M < N` — a TRUE ligature (`fi`/`fl`, or `->` on a `calt` mono) collapses
///     several source chars into ONE glyph carrying the whole span.
///   * `M = N` — Monaspace Xenon's AAT/`morx` "texture-healing" ligatures
///     (`=> != -> >= <= == ::`) emit one glyph PER source char but stamp EVERY
///     one with the SAME (start,end) span (unsuppressable by OpenType features).
/// Either way the fix is one rule: gather the whole GROUP of consecutive glyphs
/// that share a span, take its COMBINED advance `A = (max right x) − (min left
/// x)` across all `M` glyphs, and distribute the `(end − start)` source chars
/// EVENLY over it — char `i` sits at `group_left + (i − start) · A / (end −
/// start)`. Splitting one glyph's advance fairly across its chars (`M<N`) and
/// summing several glyphs' advances into a uniform grid (`M=N`) fall out of the
/// same formula. Taking only the FIRST glyph's advance (the old behavior)
/// collapsed a texture-healed `=>` to a half-pitch interior column, mismapping
/// the caret / selection / click on every Monaspace code line with an operator.
pub(super) fn assemble_glyph_xs(
    line_text: &str,
    clusters: &[(usize, usize, f32, f32)],
    char_width: f32,
) -> Vec<f32> {
    let char_count = line_text.chars().count();
    // Byte offset -> char index map (cluster starts land on char boundaries).
    let mut byte_to_col = vec![char_count; line_text.len() + 1];
    for (col, (b, _)) in line_text.char_indices().enumerate() {
        byte_to_col[b] = col;
    }
    byte_to_col[line_text.len()] = char_count;

    let mut xs = vec![f32::NAN; char_count + 1];
    let mut max_right = 0.0f32;
    let any = !clusters.is_empty();
    // Walk the glyph clusters, GROUPING consecutive glyphs that share the exact
    // same (start_byte, end_byte) span into one logical cluster (LTR shaping
    // emits a span's glyphs contiguously, so a linear scan finds the whole
    // group). The group's COMBINED advance — max right minus min left across ITS
    // glyphs — is what the source chars are spread over, so a texture-healed
    // ligature (several glyphs, one span) yields a uniform grid instead of the
    // first glyph's advance winning and halving the interior columns.
    let mut i = 0;
    while i < clusters.len() {
        let (start_b, end_b, _, _) = clusters[i];
        let mut group_left = f32::INFINITY;
        let mut group_right = f32::NEG_INFINITY;
        let mut j = i;
        while j < clusters.len() && clusters[j].0 == start_b && clusters[j].1 == end_b {
            group_left = group_left.min(clusters[j].2);
            group_right = group_right.max(clusters[j].3);
            j += 1;
        }
        i = j;

        let start_col = byte_to_col.get(start_b).copied().unwrap_or(char_count).min(char_count);
        let end_col = byte_to_col.get(end_b).copied().unwrap_or(char_count).min(char_count);
        max_right = max_right.max(group_right);
        // Left edge of the cluster's first char.
        if xs[start_col].is_nan() {
            xs[start_col] = group_left;
        }
        // Distribute interior char boundaries EVENLY across the group's TOTAL
        // advance, and set the boundary AFTER the cluster to its combined right.
        let span = end_col.saturating_sub(start_col).max(1);
        for k in 1..=span {
            let col = start_col + k;
            if col <= char_count {
                let frac = k as f32 / span as f32;
                let x = group_left + (group_right - group_left) * frac;
                if xs[col].is_nan() {
                    xs[col] = x;
                }
            }
        }
    }

    if !any {
        // Empty or unshaped line: fixed-pitch fallback so the caret cell and any
        // selection sliver still render where a Latin glyph would sit.
        return (0..=char_count).map(|c| c as f32 * char_width).collect();
    }

    // Fill any boundary still unset (e.g. col 0 with no glyph at byte 0) by
    // forward-filling from the previous known boundary, defaulting col 0 to 0.
    if xs[0].is_nan() {
        xs[0] = 0.0;
    }
    for i in 1..xs.len() {
        if xs[i].is_nan() {
            xs[i] = xs[i - 1].max(max_right);
        }
    }
    if let Some(last) = xs.last_mut() {
        *last = last.max(max_right);
    }
    xs
}

/// The char SPAN of the glyph CLUSTER (a `(start_byte, end_byte)` pair — one
/// entry per shaped glyph, the same clustering `assemble_glyph_xs` reads) that
/// owns byte `cur_byte` on `line_text`: `end_col - start_col`, clamped to at
/// least 1. `None` when no cluster in `clusters` owns `cur_byte`.
///
/// `1` is the overwhelmingly common case (one glyph per char); `>1` is a
/// LIGATURE — several chars shape into a single glyph (e.g. an "fi"/"ffi"
/// fixture on a font that ligates it). This is what
/// [`TextPipeline::caret_anchor_ink_box`](super::caret) reads to decide whether
/// a caret anchor may safely be replaced by its glyph's own ink box (a 1-char
/// cluster IS that glyph, one-to-one) or must keep the CELL math's fair linear
/// split (a multi-char cluster's cell already spreads one glyph's ink fairly
/// across the chars it covers).
///
/// Kept free + pure (no GPU / no live shaping), mirroring `assemble_glyph_xs`,
/// so the ligature-fallback decision is unit-testable with a SYNTHETIC
/// multi-char cluster — no bundled awl font actually ligates "fi"/"ffi" under
/// the current shaper (verified empirically across every world), so this is
/// the only way to exercise that branch.
pub(super) fn cluster_span_at(
    line_text: &str,
    clusters: &[(usize, usize)],
    cur_byte: usize,
) -> Option<usize> {
    for &(start_b, end_b) in clusters {
        if cur_byte >= start_b && cur_byte < end_b {
            let start_col = byte_col(line_text, start_b);
            let end_col = byte_col(line_text, end_b);
            return Some(end_col.saturating_sub(start_col).max(1));
        }
    }
    None
}

impl TextPipeline {
    /// The ZOOM-INDEPENDENT glyph advance that drives the page column pixel width:
    /// the live advance with the user zoom stripped (see [`page_column_advance`]). The
    /// column geometry reads THIS, not `metrics.char_width`, so the page + margins +
    /// gutter stay put across zoom levels (zoom only resizes the glyphs INSIDE).
    pub(super) fn page_advance(&self) -> f32 {
        page_column_advance(self.metrics.char_width, self.metrics.zoom)
    }

    /// PAGE MODE: the WIDTH (px) of the writing column for the current window +
    /// measure. Driven by the ZOOM-INDEPENDENT [`Self::page_advance`], so zoom does
    /// NOT change it. See [`column_width_for`] for the pure math.
    pub fn column_width(&self) -> f32 {
        column_width_for(
            self.window_w,
            self.page_advance(),
            crate::page::page_on(),
            crate::page::measure(),
        )
    }

    /// PAGE MODE: the LEFT edge (px) of the writing column. See [`column_left_for`].
    /// Zoom-independent (driven by [`Self::page_advance`]).
    pub fn column_left(&self) -> f32 {
        column_left_for(
            self.window_w,
            self.page_advance(),
            crate::page::page_on(),
            crate::page::measure(),
        )
    }

    /// DIRECT-MANIPULATION resize — is the pointer at `pointer_x` (physical px)
    /// hovering a DRAGGABLE page-column edge? True only in page mode AND when the
    /// column has REAL margin room (left past the small [`PAGE_MIN_PAD`]) — a
    /// collapsed, effectively edge-to-edge page has no margin to give, so it offers
    /// no handle. The pure proximity test is [`page_boundary_hit`]. The live app reads
    /// this to flip the OS cursor to a resize glyph and to decide whether a press
    /// begins a width drag instead of a text selection.
    pub fn page_resize_hover(&self, pointer_x: f32) -> bool {
        if !crate::page::page_on() {
            return false;
        }
        let left = self.column_left();
        if left <= PAGE_MIN_PAD + 1.0 {
            return false;
        }
        page_boundary_hit(pointer_x, left, self.column_width(), PAGE_RESIZE_GRAB_PX).is_some()
    }

    /// CURSOR SHAPE — is `pointer_x` within the writing column's horizontal
    /// extent? This is the "is the pointer over document TEXT" half of the
    /// context-aware OS cursor (`cursor_shape::CursorContext::over_text`) —
    /// reuses the SAME `column_left`/`column_width` accessors
    /// [`Self::page_resize_hover`] already reads (through the shared pure
    /// [`in_writing_column`]), so the column geometry can never drift between
    /// the two hover decisions. Edge-to-edge (page mode off), the column spans
    /// nearly the whole window (`NONPAGE_INSET` on both sides), so this is
    /// true almost everywhere; in page mode it's exactly the lighter page
    /// surface, so the outer margins / gutter read as `false` (the OS cursor
    /// falls back to the plain arrow there).
    pub fn over_writing_column(&self, pointer_x: f32) -> bool {
        in_writing_column(pointer_x, self.column_left(), self.column_width())
    }

    /// DIRECT-MANIPULATION resize — the page MEASURE (chars) implied by dragging a
    /// column edge to `pointer_x` (physical px), symmetric about the window center and
    /// clamped to the settable band. Driven by the ZOOM-INDEPENDENT [`Self::page_advance`]
    /// (like the column width itself), so a drag maps px→chars the same at any zoom. See
    /// [`page_resize_measure`].
    pub fn page_resize_measure_at(&self, pointer_x: f32) -> usize {
        page_resize_measure(self.window_w, self.page_advance(), pointer_x)
    }

    /// INLINE-IMAGE DRAG-RESIZE (v2) — the DISPLAY WIDTH (px) an image gets from
    /// dragging its `handle` (edge/corner) to `pointer`, given its PRESS-TIME on-screen
    /// `rect` `[left, top, w, h]`: the pure [`image_resize_width`] clamped to
    /// `[MIN_IMAGE_W, text_wrap_width()]`. Mirrors [`Self::page_resize_measure_at`] —
    /// the app supplies the handle + press rect + pointer, the pipeline owns the column
    /// geometry (the fit-to-column wrap ceiling), so no raw geometry leaks to the app.
    pub fn image_resize_width_at(&self, handle: ImageHandle, rect: [f32; 4], pointer: (f32, f32)) -> f32 {
        image_resize_width(handle, rect, pointer, self.text_wrap_width(), MIN_IMAGE_W)
    }

    /// PAGE MODE geometry bundle for the sidecar: (on, measure_chars, left, width).
    /// Reports the page SURFACE (the lighter column the background punches out), NOT
    /// the inset text box — the text margin lives inside it (see [`Self::text_left`]).
    pub fn page_geometry(&self) -> (bool, usize, f32, f32) {
        (
            crate::page::page_on(),
            crate::page::measure(),
            self.column_left(),
            self.column_width(),
        )
    }

    /// Which STICKY page-width CLASS (prose/code) the currently-shaped document
    /// belongs to — the sidecar's `page.class` field. Delegates to the ONE
    /// classifier (`crate::page::PageClass::of_syntax`), driven by `self.syn_lang`
    /// (set from `ViewState::syn_lang` in `set_view`), so it can never disagree
    /// with `Buffer::page_class` for the same document.
    pub fn page_class(&self) -> crate::page::PageClass {
        crate::page::PageClass::of_syntax(self.syn_lang)
    }

    /// Horizontal inset of the document TEXT within the page column — the writing
    /// margin inside the lighter page surface, so glyphs don't sit flush against the
    /// column edge. Page mode only (edge-to-edge keeps its `TEXT_LEFT` origin).
    /// Scales with the glyph advance, so it tracks zoom/DPI and stays proportional.
    pub(super) fn text_pad(&self) -> f32 {
        if crate::page::page_on() {
            self.metrics.char_width * PAGE_TEXT_PAD_CHARS
        } else {
            0.0
        }
    }

    /// The x where document text / caret / selection start: the page column's left
    /// edge plus the writing inset [`Self::text_pad`]. The page SURFACE still spans
    /// from `column_left`, so this inset reads as an inner margin. Public so the
    /// capture sidecar can report the TRUE text origin (not the surface edge).
    pub fn text_left(&self) -> f32 {
        self.column_left() + self.text_pad()
    }

    /// The soft-wrap width available to TEXT: the page column width minus the inset
    /// on BOTH sides, so the right margin mirrors the left. This is THE buffer wrap
    /// width (the invariant `sync_wrap_width` enforces); every wrap-setter uses it.
    pub(super) fn text_wrap_width(&self) -> f32 {
        (self.column_width() - 2.0 * self.text_pad()).max(1.0)
    }

    /// Pixel y of the top of the document after applying scroll. Negative when
    /// scrolled so that earlier lines are pushed above the viewport. The scroll
    /// unit is a VISUAL ROW index; with variable-height rows (headings) the pixel
    /// offset is the cumulative top of the first visible row, read from the
    /// row-geometry table rather than `scroll_lines * line_height`.
    pub(super) fn doc_top(&self) -> f32 {
        TEXT_TOP - self.row_top_px(self.scroll_lines)
    }

    /// Buffer-relative top y (px) of visual row `row` (clamped to the last row).
    /// `0.0` for an unshaped/empty buffer, so `doc_top()` resolves to `TEXT_TOP`.
    /// Delegates to the owning [`rowgeom::RowGeom`].
    pub(super) fn row_top_px(&self, row: usize) -> f32 {
        self.row_geom.top_px(&self.buffer, &self.metrics, row)
    }

    /// Height (px) of visual row `row` (clamped to the last row). Falls back to the
    /// uniform line height for an unshaped/empty buffer. Delegates to the owning
    /// [`rowgeom::RowGeom`].
    pub(super) fn row_height_px(&self, row: usize) -> f32 {
        self.row_geom.height_px(&self.buffer, &self.metrics, row)
    }

    /// Total pixel height of the shaped document (bottom of the last visual row).
    /// Delegates to the owning [`rowgeom::RowGeom`].
    pub(super) fn total_doc_height(&self) -> f32 {
        self.row_geom.total_height(&self.buffer, &self.metrics)
    }

    /// Maximum free-scroll offset in VISUAL ROWS, variable-height aware. The whole
    /// document fits when its pixel height is within the text viewport, so it cannot
    /// scroll (returns 0); otherwise the last [`OVERSCROLL_KEEP_ROWS`] rows stay
    /// reachable — with the default keep of 1 that is `total_rows - 1` (the last row
    /// can rise to the top), matching the uniform [`max_scroll`] but using a
    /// pixel-accurate "does it fit" test so a tall heading near the end can't hide
    /// content the uniform row count would have deemed visible.
    pub fn max_scroll_rows(&self, height: f32) -> usize {
        let total = self.total_visual_rows();
        if total == 0 {
            return 0;
        }
        let avail = (height - TEXT_TOP).max(0.0);
        if self.total_doc_height() <= avail {
            return 0;
        }
        total.saturating_sub(OVERSCROLL_KEEP_ROWS)
    }

    /// Minimal new scroll (in visual rows) so visual `row` is fully visible given the
    /// current `scroll` and viewport `height`. Scrolls UP to `row` if it's above the
    /// view; otherwise advances the top row until `row`'s bottom fits within the text
    /// viewport. Variable-height aware (sums real row heights), so cursor-follow
    /// lands correctly even when the cursor sits on — or just past — a tall heading.
    pub fn scroll_to_show_row(&self, row: usize, scroll: usize, height: f32) -> usize {
        if row < scroll {
            return row;
        }
        let avail = (height - TEXT_TOP).max(1.0);
        let bottom = self.row_top_px(row) + self.row_height_px(row);
        let mut s = scroll;
        while s < row && bottom - self.row_top_px(s) > avail {
            s += 1;
        }
        s
    }

    /// TYPEWRITER cursor-follow: the scroll (in visual rows) that CENTERS visual
    /// `row` vertically in the text viewport — used while TYPEWRITER SCROLL is on so
    /// the caret row rests at the eye line. Picks the
    /// scroll row whose top puts `row`'s vertical CENTER nearest the viewport center,
    /// clamping at the document top (row 0) when centering would scroll above it.
    /// Variable-row-height aware (reads each row's real top + height, so a tall
    /// heading still lands centered); unlike [`Self::scroll_to_show_row`] it takes no
    /// current scroll — centering is ABSOLUTE, always re-derived from `row`. The
    /// caller still clamps the result to [`Self::max_scroll_rows`] so the document
    /// tail can't be pulled past its bottom. When focus is Off the minimal-adjust
    /// `scroll_to_show_row` is used instead, so default scrolling is byte-identical.
    pub fn scroll_to_center_row(&self, row: usize, height: f32) -> usize {
        let total = self.total_visual_rows();
        if total == 0 {
            return 0;
        }
        let avail = (height - TEXT_TOP).max(1.0);
        // Buffer-relative top the viewport would need so `row`'s center sits at the
        // viewport's vertical center. Negative means `row` is near the document top
        // and can't be centered (no content above it), so we pin at the top.
        let target_top = self.row_top_px(row) + self.row_height_px(row) / 2.0 - avail / 2.0;
        if target_top <= 0.0 {
            return 0;
        }
        // `row_top_px` is monotonic in the scroll row, so walk up to the last row
        // whose top is still at/below the target, then pick whichever of it or its
        // successor lands nearer the target (closest integer-row centering).
        let mut s = 0usize;
        while s + 1 < total && self.row_top_px(s + 1) <= target_top {
            s += 1;
        }
        if s + 1 < total {
            let below = self.row_top_px(s);
            let above = self.row_top_px(s + 1);
            if (above - target_top).abs() < (target_top - below).abs() {
                s += 1;
            }
        }
        // Never scroll the cursor's own row off the top (a degenerate sub-row-height
        // viewport could otherwise pick s > row).
        s.min(row)
    }

    /// Real shaped-glyph X boundaries for a logical `line`, in pixels RELATIVE to
    /// the text's left edge (TEXT_LEFT not yet added). The returned vector has one
    /// entry per CHAR boundary: `xs[col]` is the left edge of the glyph cell at
    /// char-column `col`, and `xs[char_count]` is the right edge of the last glyph
    /// (end of line). So a line of N chars yields N+1 boundaries.
    ///
    /// This is the SINGLE SOURCE OF TRUTH for horizontal placement under advance-
    /// aware layout: it reads the actual advances cosmic-text produced (full-width
    /// for CJK, the mono advance for Latin), so caret / hit-test / selection all
    /// land on the real glyph cells for mixed CJK + Latin text.
    ///
    /// cosmic-text glyphs carry BYTE ranges (`start`/`end`) into the line text;
    /// awl columns are CHAR indices. We walk the line's chars and, for each, take
    /// the left x of the glyph cluster covering that char's byte. Multi-char
    /// clusters (rare here) share the cluster's span linearly. Empty / glyphless
    /// lines fall back to CHAR_WIDTH so an empty line still has a sane caret cell.
    pub(super) fn line_glyph_xs(&self, line: usize) -> Vec<f32> {
        let Some(line_text) = self.buffer.lines.get(line).map(|l| l.text().to_string()) else {
            return vec![0.0];
        };
        // Gather all glyph clusters of this logical line across its (possibly
        // wrapped) visual runs as (start_byte, end_byte, left_x, right_x). Glyph
        // x's reset to ~0 at the start of each wrapped run, so to keep the
        // FLATTENED single-row x map monotonic we offset each run's x's so they
        // continue after the previous run. This preserves the old single-row
        // horizontal model for callers that don't care about which visual row a
        // column lands on (the vertical position now comes from `visual_rows`).
        let mut clusters: Vec<(usize, usize, f32, f32)> = Vec::new();
        let mut x_offset = 0.0f32;
        for run in self.buffer.layout_runs() {
            if run.line_i != line {
                // Runs arrive in document order (non-decreasing `line_i`), so once
                // we pass the target line no later run can own it — stop instead of
                // walking the rest of the document's runs. Byte-identical: only
                // non-matching trailing runs are skipped (same as `cursor_glyph_key_at`).
                if run.line_i > line {
                    break;
                }
                continue;
            }
            let mut run_max_right = 0.0f32;
            for g in run.glyphs.iter() {
                let left = g.x + x_offset;
                let right = g.x + g.w + x_offset;
                clusters.push((g.start, g.end, left, right));
                run_max_right = run_max_right.max(right);
            }
            // Next wrapped run's local x's continue past this run's content.
            x_offset = run_max_right.max(x_offset);
        }
        assemble_glyph_xs(&line_text, &clusters, self.metrics.char_width)
    }

    /// The visual rows (wrapped sub-lines) of logical `line`, in top-to-bottom
    /// order. Each [`VisualRow`] carries the row's wrap-aware top y RELATIVE to
    /// the buffer top (add [`Self::doc_top`] for an absolute pixel y), the byte
    /// range of the original line it covers, and that row's own per-char x
    /// boundaries (relative to TEXT_LEFT) so an overlay can be placed on the
    /// correct row horizontally too. When `line` has no shaped runs (empty /
    /// glyphless line) a single synthetic row is returned at the line's uniform
    /// `line * line_height` top, so callers still get a sane row.
    pub(super) fn visual_rows(&self, line: usize) -> Vec<VisualRow> {
        // SINGLE-SLOT MEMO (see `rowgeom::RowGeom`): the caret geometry reads the
        // cursor line's wrap rows ~4× per redraw, and each rebuild walks every shaped
        // run of the document. The memo is cleared only at a shaped-geometry seam
        // (reshape/zoom/restyle), never on a cursor move, so a hit is always valid —
        // a motion keeps the same shaped runs. Calls 2–4 (and idle glide frames, where
        // the cursor line is unchanged) clone the cached rows instead of rebuilding.
        if let Some(cached) = self.row_geom.cached_rows(line) {
            return cached;
        }
        let line_text = self
            .buffer
            .lines
            .get(line)
            .map(|l| l.text().to_string())
            .unwrap_or_default();
        let mut rows: Vec<VisualRow> = Vec::new();
        for run in self.buffer.layout_runs() {
            if run.line_i != line {
                // Runs arrive in document order (non-decreasing `line_i`), so once
                // we pass the target line no later run can own it — stop instead of
                // walking the rest of the document's runs. Byte-identical: only
                // non-matching trailing runs are skipped (same as `cursor_glyph_key_at`).
                if run.line_i > line {
                    break;
                }
                continue;
            }
            rows.push(visual_row_from_run(&line_text, &run, self.metrics.char_width));
        }
        if rows.is_empty() {
            // Empty / glyphless logical line: synthesize one row at the uniform
            // top so the caret / selection sliver still renders sanely. This is
            // the only path that falls back to `line * line_height` and it matches
            // the pre-wrap behavior exactly for a blank line.
            rows.push(self.synthetic_visual_row(line, &line_text));
        }
        // Memoize for the next read of this line (the per-frame caret path re-asks for
        // the cursor line; the memo is dropped at the next shaped-geometry seam).
        self.row_geom.store_rows(line, &rows);
        rows
    }

    /// The [`VisualRow`]s of EVERY logical line in `lines`, built in ONE
    /// `layout_runs()` walk — the batched twin of [`Self::visual_rows`] for the
    /// spell-squiggle / nit-underline proto rebuilds, which need the rows of MANY
    /// lines at once. Calling `visual_rows` per line re-walks every shaped run of
    /// the document each time (O(lines × doc)); this walks the runs once and
    /// assembles rows only for the requested lines (O(doc + requested rows)).
    ///
    /// Per line the rows are IDENTICAL to `visual_rows(line)` — the same
    /// [`visual_row_from_run`] assembly per shaped run, and the same synthetic
    /// uniform-top fallback row for an empty / glyphless / out-of-range line — so
    /// geometry derived from either source is byte-identical. Does NOT touch the
    /// single-slot cursor-line memo (so the caret path's warm memo survives).
    pub(super) fn visual_rows_for_lines(
        &self,
        lines: &std::collections::BTreeSet<usize>,
    ) -> std::collections::HashMap<usize, Vec<VisualRow>> {
        let mut out: std::collections::HashMap<usize, Vec<VisualRow>> =
            std::collections::HashMap::with_capacity(lines.len());
        let Some(&max_line) = lines.iter().next_back() else {
            return out;
        };
        // A line's runs arrive consecutively, so its text is fetched once and
        // reused for each of its wrapped rows.
        let mut cur: Option<(usize, String)> = None;
        for run in self.buffer.layout_runs() {
            if run.line_i > max_line {
                break; // document order: nothing later can be a requested line
            }
            if !lines.contains(&run.line_i) {
                continue;
            }
            if cur.as_ref().map(|(li, _)| *li) != Some(run.line_i) {
                let text = self
                    .buffer
                    .lines
                    .get(run.line_i)
                    .map(|l| l.text().to_string())
                    .unwrap_or_default();
                cur = Some((run.line_i, text));
            }
            let line_text = &cur.as_ref().unwrap().1;
            out.entry(run.line_i)
                .or_default()
                .push(visual_row_from_run(line_text, &run, self.metrics.char_width));
        }
        // Same fallback as `visual_rows`: a requested line with no shaped runs
        // (empty / glyphless / out-of-range) synthesizes one uniform-top row.
        for &line in lines {
            if out.contains_key(&line) {
                continue;
            }
            let line_text = self
                .buffer
                .lines
                .get(line)
                .map(|l| l.text().to_string())
                .unwrap_or_default();
            out.insert(line, vec![self.synthetic_visual_row(line, &line_text)]);
        }
        out
    }

    /// The synthetic single [`VisualRow`] for an EMPTY / glyphless logical line —
    /// the shared fallback of [`Self::visual_rows`] and
    /// [`Self::visual_rows_for_lines`], at the uniform `line * line_height` top
    /// (the only remaining use of that pre-wrap formula).
    fn synthetic_visual_row(&self, line: usize, line_text: &str) -> VisualRow {
        let char_count = line_text.chars().count();
        let xs = assemble_glyph_xs(line_text, &[], self.metrics.char_width);
        VisualRow {
            line_top: line as f32 * self.metrics.line_height,
            line_height: self.metrics.line_height,
            byte_start: 0,
            byte_end: line_text.len(),
            start_col: 0,
            end_col: char_count,
            xs,
        }
    }

    /// LOCAL wrap rows of logical `line` — the O(line) twin of [`Self::visual_rows`]
    /// for the visual-line MOTION oracle. It reads ONLY that line's already-shaped
    /// [`cosmic_text::BufferLine::layout_opt`] (its `Vec<LayoutLine>`), so it does NOT
    /// walk the whole document's `layout_runs()` the way `visual_rows` does — the fix
    /// for the O(doc)-per-keypress cost when a motion targets a line the single-slot
    /// row memo hasn't cached (the destination line ± 1 every arrow press).
    ///
    /// The returned rows carry the SAME per-char `xs` + `start_col`/`end_col` as
    /// `visual_rows` (built from the identical glyph clusters, so the oracle's
    /// `pick_row_index` / `col_in_row` land on the identical column), but the
    /// `line_top` / `line_height` are NOT the doc-absolute wrap tops — the motion
    /// oracle only needs the horizontal + column geometry, never the absolute y.
    /// Callers that need the absolute row top (caret / selection / ornament
    /// placement) MUST keep using `visual_rows`.
    ///
    /// Falls back to `visual_rows(line)` when the line is unshaped / has no layout
    /// (an empty or not-yet-laid line), so the synthetic-row edge case stays exactly
    /// as before.
    pub(super) fn line_rows_local(&self, line: usize) -> Vec<VisualRow> {
        let Some(bline) = self.buffer.lines.get(line) else {
            return self.visual_rows(line);
        };
        let Some(layout) = bline.layout_opt() else {
            // Not yet laid out: defer to the whole-doc path (which synthesizes a row
            // for an empty/glyphless line) so behaviour is unchanged.
            return self.visual_rows(line);
        };
        if layout.is_empty() {
            return self.visual_rows(line);
        }
        let line_text = bline.text().to_string();
        let mut rows: Vec<VisualRow> = Vec::with_capacity(layout.len());
        for lline in layout.iter() {
            let mut clusters: Vec<(usize, usize, f32, f32)> = Vec::new();
            let mut byte_start = usize::MAX;
            let mut byte_end = 0usize;
            for g in lline.glyphs.iter() {
                clusters.push((g.start, g.end, g.x, g.x + g.w));
                byte_start = byte_start.min(g.start);
                byte_end = byte_end.max(g.end);
            }
            if byte_start == usize::MAX {
                byte_start = 0;
                byte_end = 0;
            }
            let xs = assemble_glyph_xs(&line_text, &clusters, self.metrics.char_width);
            let start_col = byte_col(&line_text, byte_start);
            let end_col = byte_col(&line_text, byte_end);
            rows.push(VisualRow {
                // The motion oracle ignores these two; use benign placeholders (the
                // uniform line height) rather than the absolute wrap top this path
                // deliberately does NOT compute.
                line_top: 0.0,
                line_height: self.metrics.line_height,
                byte_start,
                byte_end,
                start_col,
                end_col,
                xs,
            });
        }
        if rows.is_empty() {
            return self.visual_rows(line);
        }
        rows
    }

    /// TOTAL number of VISUAL ROWS in the whole document (every soft-wrapped
    /// continuation counts as its own row). This is the unit the scroll offset is
    /// measured in: a doc whose logical lines wrap has MORE visual rows than
    /// logical lines, and scrolling must reach the last one.
    ///
    /// Rows are NOT a uniform height (a heading row is taller), so this is simply
    /// the COUNT of shaped runs (one per visual row), read from the row-geometry
    /// table. Requires the whole document to be shaped (see [`Self::set_size`] /
    /// [`Self::full_shape_height`]); an unshaped tail would undercount. Falls back
    /// to the logical line count if nothing is shaped (degenerate empty buffer).
    pub fn total_visual_rows(&self) -> usize {
        self.row_geom.total_visual_rows(&self.buffer, &self.metrics)
    }

    /// The 0-based VISUAL ROW index of the position at (`line`, `col`): the index in
    /// the document-wide row-geometry table of the visual row that owns `col` on that
    /// logical line (matched by its `line_top`, which both this and the table read
    /// from the same `run.line_top`). This is the row the cursor sits on for
    /// cursor-follow, and the inverse of the visual-row -> (line,col) walk used by
    /// hit-testing. For a non-wrapped, no-heading document the tops are evenly spaced
    /// so this still equals the logical line index — cursor-follow is unchanged when
    /// nothing wraps and no heading grows a row.
    pub fn visual_row_of(&self, line: usize, col: usize) -> usize {
        let rows = self.visual_rows(line);
        let target = pick_row(&rows, col).line_top;
        self.row_geom.nearest_row(&self.buffer, &self.metrics, target)
    }

    /// Wrap-aware visual-row top y (absolute, scroll-applied) for the position at
    /// (`line`, char `col`). Picks the wrapped run whose char span contains `col`;
    /// at/after end-of-line it uses the LAST run of the line. Empty / glyphless
    /// lines fall back to the synthetic row from [`Self::visual_rows`] (which is
    /// at the uniform `line * line_height` top), so a blank line keeps a sane
    /// caret row. This is THE replacement for `doc_top() + line * line_height` in
    /// every overlay, so caret / selection / squiggles ride the real wrapped row.
    pub(super) fn visual_row_top(&self, line: usize, col: usize) -> f32 {
        let rows = self.visual_rows(line);
        self.doc_top() + pick_row(&rows, col).line_top
    }

    /// Pixel x (relative to TEXT_LEFT) of the glyph boundary at char-column `col`
    /// on logical `line`, plus the advance width of the glyph cell starting there
    /// (full-width for CJK, mono for Latin). At end-of-line the advance falls back
    /// to CHAR_WIDTH so the caret keeps a visible cell past the last glyph, and a
    /// DEGENERATE mid-line cell (see [`DEGENERATE_CELL_FRAC`]) falls back the same
    /// way so the caret stays visible on a collapsed wrap-boundary space.
    pub(super) fn col_x_and_advance(&self, line: usize, col: usize) -> (f32, f32) {
        // Use the VISUAL ROW that owns `col` so a wrapped column reads its run's
        // own left-aligned x's (each wrapped run restarts near x=0). For a
        // non-wrapped line there is exactly one row whose xs == line_glyph_xs, so
        // this is identical to the previous behavior.
        let rows = self.visual_rows(line);
        let row = pick_row(&rows, col);
        let n = row.xs.len().saturating_sub(1); // char count on the logical line
        let c = col.min(n);
        let x = row.xs[c];
        let advance = if c < n {
            let raw = row.xs[c + 1] - row.xs[c];
            if raw < self.metrics.char_width * DEGENERATE_CELL_FRAC {
                // DEGENERATE cell: a mid-line column with (near-)coincident x
                // boundaries — no visible glyph owns it. The canonical case is the
                // SPACE at a soft-wrap boundary: cosmic-text collapses the trailing
                // whitespace at the break, so both its boundaries sit on the row's
                // right edge and the raw width is ~0 — which used to draw the block
                // caret as a ~1px SLIVER there. Fall back to the same default cell
                // the end-of-line branch uses, so the caret on the collapsed wrap
                // space reads exactly like the caret past the last glyph. Real
                // narrow glyphs (`i`, `l`, thin spaces) sit well above the
                // threshold and keep their true advance.
                self.metrics.char_width
            } else {
                raw
            }
        } else {
            // End of line: no glyph to cover; use a default Latin-ish cell.
            self.metrics.char_width
        };
        (x, advance)
    }

    /// Height (px) of the visual row the cursor sits on — `run.line_height` for the
    /// owning wrapped run, which is LARGER on a heading line. Used to centre the
    /// caret box (and via it the spring anchor) within the real row.
    pub(super) fn cursor_row_height(&self) -> f32 {
        let rows = self.visual_rows(self.cursor_line);
        pick_row(&rows, self.cursor_col).line_height
    }

    /// The cursor row's height as a MULTIPLE of the base line height: `1.0` on body
    /// text, the heading scale (e.g. 1.8) when the caret sits on a heading line. The
    /// resting block caret multiplies its height by this so it COVERS the whole big
    /// glyph (its width already tracks the real advance, and the descender-aware
    /// bottom already reads the real glyph), keeping the "the caret possesses the
    /// character" feel (DESIGN.md §6) at any heading size. Exactly `1.0` for body
    /// rows, so the body caret is byte-identical.
    ///
    /// IMAGE LINE (the caption model): the caret sizes to the SOURCE text — body
    /// glyphs at scale `1.0` — NOT the tall reserved row. The row height covers the
    /// (revealed, dimmed) image, and a row-scaled caret would balloon to the whole
    /// image-row height (the reported bug); the source glyphs are body-size, so the
    /// caret must be too. `caret_cell_top` still centres the body-height caret in
    /// the full (tall) row, which is exactly where cosmic-text centres the source
    /// glyphs, so the caret lands ON the centred caption.
    pub(super) fn cursor_scale(&self) -> f32 {
        // Only with WYSIWYG on (the reveal/caption model applies): with it off the
        // source shows unconcealed and the caret keeps its pre-existing sizing
        // (byte-identical off state).
        if crate::markdown::wysiwyg_on() && self.line_is_inline_image(self.cursor_line) {
            return 1.0;
        }
        let lh = self.metrics.line_height;
        if lh > 0.0 {
            (self.cursor_row_height() / lh).max(1.0)
        } else {
            1.0
        }
    }

    /// Advance-aware, WRAP-aware pixel -> (line, col) hit test. Walks the real
    /// cosmic-text layout runs once, finds the visual row whose
    /// `[line_top, line_top+line_height)` band contains the click's y (so a click
    /// on a wrapped continuation maps to the right logical line, not the Nth
    /// uniform row), then walks that row's glyph advances to pick the char-column
    /// whose cell the pointer x falls in. A click past a glyph's midpoint snaps to
    /// the next gap (natural caret placement). Accounts for scroll + zoom; the
    /// caller clamps (line, col) to the buffer.
    pub fn hit_test(&self, px: f32, py: f32, scroll_lines: usize) -> (usize, usize) {
        // Absolute pixel y of the click, in the same buffer-top frame as
        // `run.line_top` (so wrapped rows compare correctly). Recompute doc_top for
        // the requested `scroll_lines` (which may differ from self.scroll_lines
        // mid-drag within a frame).
        let doc_top = TEXT_TOP - self.row_top_px(scroll_lines);
        let want_top = (py - doc_top).max(0.0); // y relative to buffer top
        let target_x = (px - self.text_left()).max(0.0);

        // One pass over the visual runs: pick the run whose band contains the
        // click. The first run also catches a click ABOVE all text (clamp to it).
        let mut first_run = true;
        for run in self.buffer.layout_runs() {
            let above_first = first_run && want_top < run.line_top;
            let in_band =
                want_top >= run.line_top && want_top < run.line_top + run.line_height;
            if above_first || in_band {
                return (run.line_i, Self::col_in_run(&run, target_x));
            }
            first_run = false;
        }
        // Click BELOW all rows -> clamp to the LAST visual row. An entirely empty
        // buffer (no runs) maps to the origin.
        match self.buffer.layout_runs().last() {
            Some(run) => (run.line_i, Self::col_in_run(&run, target_x)),
            None => (0, 0),
        }
    }

    /// Char column on a cosmic-text layout RUN whose cell contains `target_x`
    /// (relative to TEXT_LEFT). Walks the run's glyphs (byte-keyed) and snaps a
    /// click past a glyph's midpoint to the next gap. A click past the run's last
    /// glyph maps to the char column just after it (end of this visual row). The
    /// returned column is a GLOBAL char column on the logical line.
    pub(super) fn col_in_run(run: &glyphon::cosmic_text::LayoutRun, target_x: f32) -> usize {
        let line_text = run.text;
        for g in run.glyphs.iter() {
            let left = g.x;
            let right = g.x + g.w;
            let mid = (left + right) * 0.5;
            if target_x < mid {
                return byte_col(line_text, g.start);
            } else if target_x < right {
                return byte_col(line_text, g.end);
            }
        }
        // Past the last glyph: end of this run. Use the last glyph's end byte, or
        // the run's start column if it has no glyphs.
        match run.glyphs.last() {
            Some(g) => byte_col(line_text, g.end),
            None => 0,
        }
    }

    /// Char column on a visual row whose cell contains `target_x` (relative to
    /// TEXT_LEFT). Searches only this row's `[start_col, end_col]` and snaps a
    /// position past a glyph's midpoint to the next gap (natural caret placement).
    /// A position past the row's last glyph maps to the row's end column. This is a
    /// pure, GPU-free analogue of [`Self::col_in_run`] (which walks a real
    /// cosmic-text run); it lands the caret nearest a target x on a known row,
    /// shared by the unit tests AND the visual-line motion oracle (which uses it to
    /// place the caret under the sticky goal-x after stepping rows).
    pub(super) fn col_in_row(row: &VisualRow, target_x: f32) -> usize {
        let mut col = row.end_col; // default: past last glyph on this row
        for c in row.start_col..row.end_col {
            let left = row.xs[c];
            let right = row.xs[c + 1];
            let mid = (left + right) * 0.5;
            if target_x < mid {
                col = c;
                break;
            } else if target_x < right {
                col = c + 1;
                break;
            }
        }
        col
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The RESPONSIVE page column: `min(measure_px, window - 2*margin)`, centered, with
    // the margin collapsing from the generous `page_min_margin` to the small
    // `PAGE_MIN_PAD` as the measure crowds the width. These exercise the pure formula
    // (no GPU, no page globals) across the WIDE / NARROW / transition regimes.
    const CW: f32 = CHAR_WIDTH; // 14.4

    #[test]
    fn wide_window_seats_centered_column_at_measure() {
        // Plenty of room for a 40-char measure on a 1200px window: the column sits at
        // exactly measure*advance and the generous leftover splits as symmetric margins
        // — the gradient band + gutter have room to show.
        let measure_px = 40.0 * CW; // 576
        let w = column_width_for(1200.0, CW, true, 40);
        let left = column_left_for(1200.0, CW, true, 40);
        assert!((w - measure_px).abs() < 1e-3, "wide: column == measure, got {w}");
        assert!((left - (1200.0 - measure_px) * 0.5).abs() < 1e-3, "wide: centered, got {left}");
        // The leftover margin is the generous band (well past the small pad).
        assert!(left > page_min_margin(1200.0) - 1e-3, "wide leftover >= generous margin");
    }

    #[test]
    fn narrow_window_fills_minus_small_pad() {
        // The measure can't fit: the column fills the width minus only PAGE_MIN_PAD on
        // each side (margins collapse to ~0 -> patterns + gutter naturally hide).
        for &win in &[300.0_f32, 400.0, 700.0] {
            let w = column_width_for(win, CW, true, 80); // 80-char measure ~1152px >> win
            let left = column_left_for(win, CW, true, 80);
            assert!((w - (win - 2.0 * PAGE_MIN_PAD)).abs() < 1e-3, "narrow {win}: fills minus pad, got {w}");
            assert!((left - PAGE_MIN_PAD).abs() < 1e-3, "narrow {win}: left at small pad, got {left}");
            assert!(w + 2.0 * left <= win + 1e-3, "narrow {win}: never overflows");
        }
    }

    #[test]
    fn column_is_monotonic_and_never_overflows_across_a_resize() {
        // ONE smooth formula: as the window grows the column never shrinks and never
        // exceeds the measure, and always leaves at least the small pad each side. No
        // mode toggle / discontinuity from narrow fill to wide centered.
        let measure_px = 80.0 * CW;
        let mut prev = 0.0_f32;
        let mut w = 200.0;
        while w <= 2600.0 {
            let col = column_width_for(w, CW, true, 80);
            let left = column_left_for(w, CW, true, 80);
            assert!(col >= prev - 1e-3, "column must not shrink as window grows (w={w})");
            assert!(col <= measure_px + 1e-3, "column never exceeds the measure (w={w})");
            assert!(left >= PAGE_MIN_PAD - 1e-3, "always at least the small pad (w={w})");
            assert!(col + 2.0 * left <= w + 1e-2, "never overflows the window (w={w})");
            prev = col;
            w += 50.0;
        }
        // Far enough out the measure binds and the column settles at measure_px.
        assert!((column_width_for(2600.0, CW, true, 80) - measure_px).abs() < 1e-3);
    }

    #[test]
    fn wide_capture_is_byte_identical_to_the_old_cap() {
        // DISCIPLINE: where the measure binds well inside the available width (the
        // standard `--measure 40` capture on the 1200px canvas), the responsive formula
        // yields the SAME centered column the old generous-margin cap did — so wide
        // captures stay byte-identical. min(576, 1200-2*margin) == 576 either way.
        let measure_px = 40.0 * CW; // 576
        assert!((column_width_for(1200.0, CW, true, 40) - measure_px).abs() < 1e-3);
        assert!((column_left_for(1200.0, CW, true, 40) - (1200.0 - measure_px) * 0.5).abs() < 1e-3);
    }

    #[test]
    fn page_off_is_edge_to_edge_unaffected() {
        // Page mode off keeps the fixed NONPAGE_INSET origin + full content width.
        assert!((column_left_for(1200.0, CW, false, 80) - NONPAGE_INSET).abs() < 1e-3);
        assert!((column_width_for(1200.0, CW, false, 80) - (1200.0 - 2.0 * NONPAGE_INSET)).abs() < 1e-3);
        // The plain inset is a touch wider than the page collapse floor.
        assert!(NONPAGE_INSET > PAGE_MIN_PAD);
    }

    // === ZOOM DECOUPLING (the bug fix) =====================================
    // The page column pixel width — and thus the side MARGINS + the bottom-left
    // gutter that gate on having margin room — must be driven by the WINDOW + the
    // settable measure ONLY, never by zoom. Zoom scales `metrics.char_width` (=
    // CHAR_WIDTH * zoom * dpi); `page_column_advance` strips the zoom back out, so the
    // advance fed to `column_width_for` is the zoom-1 base and the column is invariant.

    #[test]
    fn page_column_advance_strips_zoom_keeps_dpi() {
        // The live advance is CHAR_WIDTH * zoom * dpi; page_column_advance divides the
        // zoom out, leaving CHAR_WIDTH * dpi (display-only, zoom-invariant).
        for &dpi in &[1.0_f32, 2.0] {
            let base = CW * dpi;
            for &zoom in &[0.5_f32, 1.0, 1.6, 2.5, 3.0] {
                let live = CW * zoom * dpi; // == metrics.char_width
                let adv = page_column_advance(live, zoom);
                assert!((adv - base).abs() < 1e-3, "zoom={zoom} dpi={dpi}: advance must be zoom-free");
            }
        }
        // Zoom 1.0 (the deterministic capture path) is an exact identity.
        assert!((page_column_advance(CW, 1.0) - CW).abs() < 1e-6);
    }

    #[test]
    fn zooming_in_keeps_column_and_margins_constant_gutter_stays() {
        // THE BUG: zooming IN removed the gutter because the column grew past the
        // window cap and the margins collapsed. Now the column + margins are computed
        // from the ZOOM-INDEPENDENT advance, so a WIDE window keeps its page + gutter
        // at every zoom. Take the zoom-1 column as the reference and assert every other
        // zoom reproduces it EXACTLY (column px + both margins identical).
        let window = 1200.0;
        let measure = 40; // narrow measure -> generous, clearly-present margins
        let base_adv = page_column_advance(CW, 1.0);
        let ref_w = column_width_for(window, base_adv, true, measure);
        let ref_left = column_left_for(window, base_adv, true, measure);
        // A real gutter needs real margin room at zoom 1.0 (sanity for the fixture).
        assert!(ref_left > PAGE_MIN_PAD + 1.0, "fixture must have a visible margin/gutter");
        for &zoom in &[0.5_f32, 1.0, 1.6, 2.5, 3.0] {
            let live = CW * zoom; // metrics.char_width at this zoom (dpi 1.0)
            let adv = page_column_advance(live, zoom);
            let w = column_width_for(window, adv, true, measure);
            let left = column_left_for(window, adv, true, measure);
            assert!((w - ref_w).abs() < 1e-3, "zoom={zoom}: column px must not change (got {w}, want {ref_w})");
            assert!((left - ref_left).abs() < 1e-3, "zoom={zoom}: left margin must not change");
            // The RIGHT margin (window - left - width) is the mirror; it too is fixed.
            let right = window - left - w;
            let ref_right = window - ref_left - ref_w;
            assert!((right - ref_right).abs() < 1e-3, "zoom={zoom}: right margin must not change");
        }
    }

    // === DIRECT-MANIPULATION PAGE RESIZE (hover zone + drag math) ==========
    // The LIVE feel (cursor flip + the drag tracking a finger) is winit-only, but the
    // TWO decisions under it are pure and tested here: (1) is the pointer near a column
    // edge? and (2) what measure does a drag to a given x imply? Both feed the same
    // zoom-stripped advance the column width uses, so resize is zoom-independent too.

    #[test]
    fn hover_zone_arms_only_within_grab_px_of_an_edge() {
        // 40-char column centered on 1200px: left = (1200-576)/2 = 312, right = 888.
        let measure_px = 40.0 * CW; // 576
        let left = (1200.0 - measure_px) * 0.5; // 312
        let tol = PAGE_RESIZE_GRAB_PX;
        // Right ON the left edge -> Left; just inside grab -> Left; past grab -> None.
        assert_eq!(page_boundary_hit(left, left, measure_px, tol), Some(ResizeEdge::Left));
        assert_eq!(page_boundary_hit(left + tol - 0.5, left, measure_px, tol), Some(ResizeEdge::Left));
        assert_eq!(page_boundary_hit(left + tol + 2.0, left, measure_px, tol), None);
        // The right edge arms the Right handle; dead center (far from both) is None.
        let right = left + measure_px; // 888
        assert_eq!(page_boundary_hit(right - 1.0, left, measure_px, tol), Some(ResizeEdge::Right));
        assert_eq!(page_boundary_hit(600.0, left, measure_px, tol), None);
    }

    #[test]
    fn in_writing_column_is_true_inside_and_on_both_edges_false_outside() {
        // CURSOR SHAPE's "over document text" membership test (the counterpart to the
        // proximity test above): same 40-char column centered on 1200px.
        let measure_px = 40.0 * CW; // 576
        let left = (1200.0 - measure_px) * 0.5; // 312
        let right = left + measure_px; // 888
        assert!(in_writing_column(left, left, measure_px), "exactly on the left edge counts as inside");
        assert!(in_writing_column(right, left, measure_px), "exactly on the right edge counts as inside");
        assert!(in_writing_column(600.0, left, measure_px), "dead center is inside");
        assert!(!in_writing_column(left - 1.0, left, measure_px), "just past the left margin is outside");
        assert!(!in_writing_column(right + 1.0, left, measure_px), "just past the right margin is outside");
    }

    #[test]
    fn image_handle_hit_arms_the_right_zone_per_edge_and_corner() {
        // A rect at (100,50) sized 300x200: left=100 right=400 top=50 bottom=250,
        // mid-edges at x=250 / y=150. Corners take priority over the edges they meet.
        let rect = [100.0_f32, 50.0, 300.0, 200.0];
        let tol = IMAGE_RESIZE_GRAB_PX;
        // The four corners (each the intersection of two edge bands -> the corner).
        assert_eq!(image_handle_hit((100.0, 50.0), rect, tol), Some(ImageHandle::TopLeft));
        assert_eq!(image_handle_hit((400.0, 50.0), rect, tol), Some(ImageHandle::TopRight));
        assert_eq!(image_handle_hit((100.0, 250.0), rect, tol), Some(ImageHandle::BottomLeft));
        assert_eq!(image_handle_hit((400.0, 250.0), rect, tol), Some(ImageHandle::BottomRight));
        // The four MID-edges (near one border, far from both its corners).
        assert_eq!(image_handle_hit((100.0, 150.0), rect, tol), Some(ImageHandle::Left));
        assert_eq!(image_handle_hit((400.0, 150.0), rect, tol), Some(ImageHandle::Right));
        assert_eq!(image_handle_hit((250.0, 50.0), rect, tol), Some(ImageHandle::Top));
        assert_eq!(image_handle_hit((250.0, 250.0), rect, tol), Some(ImageHandle::Bottom));
        // Just inside the tolerance band still arms (bottom-right corner).
        assert_eq!(
            image_handle_hit((400.0 - tol + 1.0, 250.0 - tol + 1.0), rect, tol),
            Some(ImageHandle::BottomRight)
        );
        // Dead center arms nothing.
        assert_eq!(image_handle_hit((250.0, 150.0), rect, tol), None, "center");
        // Past the border band on the perpendicular axis: a left-edge x but far
        // above the image is NOT the left edge (the span gate rejects it).
        assert_eq!(image_handle_hit((100.0, 50.0 - tol - 5.0), rect, tol), None, "above the top-left, off both");
        // Well outside the whole rect arms nothing.
        assert_eq!(image_handle_hit((1000.0, 1000.0), rect, tol), None, "far outside");
    }

    #[test]
    fn image_resize_width_drives_per_handle_clamped_to_min_and_wrap() {
        // A square-ish rect: left=100 right=400 top=50 bottom=250, w=300 h=200,
        // aspect = 1.5. Wrap 500, min the real floor.
        let rect = [100.0_f32, 50.0, 300.0, 200.0];
        let (wrap, min) = (500.0_f32, MIN_IMAGE_W);
        let w = |h: ImageHandle, p: (f32, f32)| image_resize_width(h, rect, p, wrap, min);
        // RIGHT edge: width = pointer_x - left. Pointer at 350 -> 250 wide.
        assert!((w(ImageHandle::Right, (350.0, 150.0)) - 250.0).abs() < 1e-3);
        // LEFT edge (mirror): width = right - pointer_x. Pointer at 200 -> 200 wide.
        assert!((w(ImageHandle::Left, (200.0, 150.0)) - 200.0).abs() < 1e-3);
        // BOTTOM edge: dy drives via aspect. Pointer y at 150 -> height 100 -> width 150.
        assert!((w(ImageHandle::Bottom, (250.0, 150.0)) - 150.0).abs() < 1e-3);
        // TOP edge (mirror): height = bottom - y = 250-150 = 100 -> width 150.
        assert!((w(ImageHandle::Top, (250.0, 150.0)) - 150.0).abs() < 1e-3);
        // A CORNER drag STAYING ON the diagonal maps 1:1 to size: from top-left,
        // a pointer at (left + t·w, top + t·h) yields width t·w. t=0.5 -> 150.
        assert!((w(ImageHandle::BottomRight, (100.0 + 150.0, 50.0 + 100.0)) - 150.0).abs() < 1e-3);
        // At the original corner the size is unchanged (t = 1 -> w). This holds for
        // ALL FOUR corners — each anchored at its OPPOSITE corner, so a pointer sitting
        // on the native corner reproduces the original width regardless of which grip.
        assert!((w(ImageHandle::BottomRight, (400.0, 250.0)) - 300.0).abs() < 1e-3);
        assert!((w(ImageHandle::TopLeft, (100.0, 50.0)) - 300.0).abs() < 1e-3);
        assert!((w(ImageHandle::TopRight, (400.0, 50.0)) - 300.0).abs() < 1e-3);
        assert!((w(ImageHandle::BottomLeft, (100.0, 250.0)) - 300.0).abs() < 1e-3);
        // Each corner GROWS when dragged outward past its native corner and SHRINKS
        // toward center — a TopLeft grip dragged up-left widens; toward center narrows.
        assert!(w(ImageHandle::TopLeft, (60.0, 20.0)) > 300.0, "TopLeft out widens");
        assert!(w(ImageHandle::TopLeft, (250.0, 150.0)) < 300.0, "TopLeft toward center narrows");
        // Clamps: dragging way out clamps to wrap; way in clamps up to the floor.
        assert!((w(ImageHandle::Right, (5000.0, 150.0)) - wrap).abs() < 1e-3);
        assert!((w(ImageHandle::Right, (100.0, 150.0)) - min).abs() < 1e-3);
        // A degenerate wrap below the floor never inverts the clamp band.
        assert!((image_resize_width(ImageHandle::Right, rect, (350.0, 150.0), 10.0, min) - min).abs() < 1e-3);
    }

    #[test]
    fn drag_math_is_symmetric_about_center_and_zoom_independent() {
        // Dragging either edge to the SAME distance from center yields the SAME measure
        // (the column is centered), and the px->char mapping uses the zoom-stripped
        // advance, so the mapping is identical at any zoom.
        let window = 1200.0;
        let center = window * 0.5; // 600
        for &zoom in &[0.5_f32, 1.0, 2.0] {
            let adv = page_column_advance(CW * zoom, zoom); // == CW at dpi 1.0
            // Right edge at 888 (half-width 288 -> width 576 -> 40 chars).
            let m_right = page_resize_measure(window, adv, 888.0);
            // Left edge mirrored at 312 (same 288 from center) -> same measure.
            let m_left = page_resize_measure(window, adv, 312.0);
            assert_eq!(m_right, 40, "zoom={zoom}: 288px from center -> 40 chars");
            assert_eq!(m_left, m_right, "zoom={zoom}: left/right mirror to same measure");
            // Dragging OUT (farther from center) widens; IN narrows.
            let wider = page_resize_measure(window, adv, center + 350.0);
            let narrower = page_resize_measure(window, adv, center + 200.0);
            assert!(wider > m_right && narrower < m_right, "zoom={zoom}: out widens, in narrows");
        }
    }

    #[test]
    fn drag_math_clamps_to_the_settable_band() {
        // A drag can never push the measure past the keyboard-command band: pulling the
        // edge out to the window rim tops out at MAX_MEASURE; jamming it toward center
        // bottoms out at MIN_MEASURE. Same [20,140] band the C-x } / { commands honour.
        let window = 4000.0; // wide enough that MAX would otherwise be exceeded
        let adv = CW;
        assert_eq!(page_resize_measure(window, adv, 3999.0), crate::page::MAX_MEASURE);
        // Pointer at the exact center -> zero width -> clamps UP to the floor.
        assert_eq!(page_resize_measure(window, adv, window * 0.5), crate::page::MIN_MEASURE);
        // A degenerate zero advance can't divide; it floors safely, never panics.
        assert_eq!(page_resize_measure(window, 0.0, 3999.0), crate::page::MIN_MEASURE);
    }

    #[test]
    fn narrow_window_still_collapses_edge_to_edge_at_any_zoom() {
        // The edge-to-edge collapse survives, but its trigger is the WINDOW being too
        // narrow to seat the measure — NOT the zoom. A genuinely narrow window fills to
        // the small pad at every zoom (gutter hides only because the WINDOW is narrow).
        let window = 360.0; // 40-char measure ~576px >> window -> collapse
        for &zoom in &[0.5_f32, 1.0, 1.6, 3.0] {
            let adv = page_column_advance(CW * zoom, zoom);
            let w = column_width_for(window, adv, true, 40);
            let left = column_left_for(window, adv, true, 40);
            assert!((w - (window - 2.0 * PAGE_MIN_PAD)).abs() < 1e-3, "zoom={zoom}: fills minus pad");
            assert!((left - PAGE_MIN_PAD).abs() < 1e-3, "zoom={zoom}: collapses to the small pad");
        }
    }
}
