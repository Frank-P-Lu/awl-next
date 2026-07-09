//! The CARET MOTIONS that need more than a bare buffer call — the ones that
//! consult the wrap-aware [`LayoutOracle`] (the FLAT DEFAULT is VISUAL): vertical
//! (C-n/C-p), line-edge (C-a/C-e), kill-line (C-k), the deterministic page scroll,
//! and the incremental-search OPEN. Each follows the SHAPED visual rows when an
//! oracle is present and falls back to the buffer's LOGICAL lines when it is absent
//! (the pure unit tests), so a non-wrapped document behaves identically either way.
//! `apply_core`'s dispatch routes its motion arms here. Carved out of `actions.rs`
//! VERBATIM.

use super::*;

/// Vertical caret motion (C-n/Down when `down`, C-p/Up otherwise) — the FLAT
/// DEFAULT is VISUAL: with a layout oracle present it steps one VISUAL row
/// (following soft wraps, crossing logical lines at the wrap edges) and lands
/// nearest a sticky GOAL-X, so the caret stays under the same screen column across
/// a run of up/down moves through wrapped rows.
///
/// The goal-x is carried on the buffer ([`Buffer::goal_x`]): the FIRST vertical
/// move of a run reads `None` and seeds the goal-x from the caret's current visual
/// x; each subsequent move reuses it (via [`Buffer::set_cursor_visual`], which
/// keeps it), and any other motion/edit clears it. This is the wrap-aware twin of
/// the logical `goal_col`. With NO oracle (the pure unit tests) it falls back to
/// the buffer's LOGICAL `next_line` / `previous_line`, so non-wrapped behavior is
/// identical.
pub(super) fn vertical_motion(ctx: &mut ActionCtx, down: bool) {
    if let Some(oracle) = ctx.oracle {
        let (line, col) = ctx.buffer.cursor_line_col();
        // Reuse the sticky goal-x across a run; seed it on the first move.
        let goal_x = ctx
            .buffer
            .goal_x()
            .unwrap_or_else(|| oracle.visual_x_of(line, col));
        let (nl, nc) = if down {
            oracle.visual_line_down(line, col, goal_x)
        } else {
            oracle.visual_line_up(line, col, goal_x)
        };
        let idx = ctx.buffer.line_col_to_char(nl, nc);
        ctx.buffer.set_cursor_visual(idx, goal_x);
        return;
    }
    if down {
        ctx.buffer.next_line();
    } else {
        ctx.buffer.previous_line();
    }
}

/// Line-edge caret motion (C-e/End when `end`, C-a/Home otherwise) — the FLAT
/// DEFAULT is VISUAL: with an oracle present the edge is that of the current
/// VISUAL row (so on a wrapped paragraph C-a/C-e stop at the screen-row boundary,
/// not the logical line's). With NO oracle it falls back to the LOGICAL
/// `line_start_motion` / `line_end_motion`, identical to before.
pub(super) fn line_edge_motion(ctx: &mut ActionCtx, end: bool) {
    if let Some(oracle) = ctx.oracle {
        let (line, col) = ctx.buffer.cursor_line_col();
        let (nl, nc) = if end {
            oracle.visual_line_end(line, col)
        } else {
            oracle.visual_line_start(line, col)
        };
        let idx = ctx.buffer.line_col_to_char(nl, nc);
        ctx.buffer.set_cursor(idx);
        return;
    }
    if end {
        ctx.buffer.line_end_motion();
    } else {
        ctx.buffer.line_start_motion();
    }
}

/// Kill-line (C-k) — the FLAT DEFAULT is VISUAL: with an oracle present it kills
/// from the caret to the end of the current VISUAL row; if the caret is already at
/// the visual-row end (which, by the wrap-boundary bias, is the LOGICAL line end)
/// it kills the trailing newline and joins the next line, exactly as today. With
/// NO oracle it falls back to the buffer's LOGICAL `kill_line`.
pub(super) fn kill_line_motion(ctx: &mut ActionCtx) {
    if let Some(oracle) = ctx.oracle {
        let (line, col) = ctx.buffer.cursor_line_col();
        let (el, ec) = oracle.visual_line_end(line, col);
        let end = ctx.buffer.line_col_to_char(el, ec);
        ctx.buffer.kill_line_to(end);
        return;
    }
    ctx.buffer.kill_line();
}

/// Move the cursor by `scroll_page_lines` logical lines up or down, stopping at
/// the buffer boundary. The windowed app's richer visual-row paging lives in
/// `App::scroll_page` (it needs the GPU to measure a screenful); this is the
/// pure, deterministic fallback shared by replay and the no-GPU path.
pub(super) fn scroll_page(buffer: &mut Buffer, scroll_page_lines: usize, down: bool) {
    for _ in 0..scroll_page_lines.max(1) {
        let before = buffer.cursor_line_col();
        if down {
            buffer.next_line();
        } else {
            buffer.previous_line();
        }
        if buffer.cursor_line_col() == before {
            break; // hit a buffer boundary
        }
    }
}

/// Open an incremental search anchored at the cursor — the entry point for
/// Cmd-F/Cmd-Shift-F AND their Cmd-G/Cmd-Shift-G aliases (P2/W2 of the
/// keybinding-idiom audit). PREFILLS the query, in priority order: (1) an
/// active, single-line selection (Xcode's "search for selection" — the
/// compensation for keeping Cmd-E on Inline code rather than the Cocoa
/// find-pasteboard idiom); else (2) the REMEMBERED last query
/// (`search::last_query`), so a bare Cmd-G after a prior search's panel
/// closed genuinely re-finds. Reading the selection happens BEFORE
/// `clear_mark` — the open always clears any active selection/region exactly
/// as before, prefill or not.
pub(super) fn start_search(ctx: &mut ActionCtx, dir: Direction) {
    let origin = ctx.buffer.cursor_char();
    let prefill = ctx
        .buffer
        .selected_text()
        .filter(|s| !s.is_empty() && !s.contains('\n'))
        .unwrap_or_else(crate::search::last_query);
    ctx.buffer.clear_mark();
    *ctx.shift_selecting = false;
    *ctx.search = Some(if prefill.is_empty() {
        SearchState::start(origin, dir)
    } else {
        let haystack = ctx.buffer.text();
        SearchState::start_with_query(origin, dir, &prefill, &haystack)
    });
}
