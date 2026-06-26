//! The pure, GPU-/winit-free core of action application. This is the single
//! seam through which BOTH the windowed app and the headless `--keys` replay
//! drive the buffer, so live editing and captured replay behave identically.
//!
//! `apply_core` is a near-mechanical lift of the big `match action` in
//! `App::apply`: it touches only the `Buffer`, the transient Shift-selection
//! flag, the zoom scalar, and the optional `SearchState`. It deliberately does
//! NOT touch the GPU, the window, or the system clipboard — the windowed
//! `App::apply` wraps this with its clipboard mirroring, and the headless
//! replay drives it with no side channels at all. The kill ring lives on the
//! `Buffer`, so cut/copy/yank still work headlessly without a clipboard.

use crate::buffer::Buffer;
use crate::keymap::Action;
use crate::render;
use crate::search::{Direction, SearchState};

/// Everything `apply_core` may mutate, gathered so the one seam can serve both
/// the windowed `App` (which owns these as fields) and a headless replay (which
/// owns them as locals). Borrowed mutably as a group to keep the signature
/// short and the call sites symmetric.
pub struct ActionCtx<'a> {
    pub buffer: &'a mut Buffer,
    /// Transient Shift-selection flag (Shift+motion GUI selection).
    pub shift_selecting: &'a mut bool,
    /// Zoom factor (ZoomIn/Out/Reset mutate this in place).
    pub zoom: &'a mut f32,
    /// Active incremental search, started by SearchForward/Backward.
    pub search: &'a mut Option<SearchState>,
    /// How many logical lines one PageDown/PageUp moves. The windowed app passes
    /// a screenful computed from the live viewport; headless passes a fixed
    /// value (no GPU to measure), keeping replay deterministic.
    pub page_lines: usize,
}

/// Apply one resolved `action` to the editor core. `shift` is whether Shift was
/// held (so a motion extends the selection, Shift+Arrow style). Returns `true`
/// if the action is `Quit` (the caller decides what "quit" means — exit the
/// event loop, or stop a replay). Mutates only what `ActionCtx` exposes; no GPU,
/// window, or clipboard.
pub fn apply_core(ctx: &mut ActionCtx, action: &Action, shift: bool) -> bool {
    // Selection-on-motion, two distinct modes:
    //   * Shift+motion = TRANSIENT (GUI style): extends only while Shift is
    //     held; the next unshifted motion collapses the selection.
    //   * C-Space mark = STICKY (Emacs style): every motion extends the region
    //     until C-g / an edit clears it.
    if action.is_motion() {
        if shift {
            if ctx.buffer.anchor_char().is_none() {
                ctx.buffer.set_mark();
            }
            *ctx.shift_selecting = true;
        } else if *ctx.shift_selecting {
            // Shift released, then moved: drop the transient selection.
            ctx.buffer.clear_mark();
            *ctx.shift_selecting = false;
        }
    }

    let mut quit = false;
    match action {
        Action::ForwardChar => ctx.buffer.forward_char(),
        Action::BackwardChar => ctx.buffer.backward_char(),
        Action::NextLine => ctx.buffer.next_line(),
        Action::PreviousLine => ctx.buffer.previous_line(),
        Action::LineStart => ctx.buffer.line_start_motion(),
        Action::LineEnd => ctx.buffer.line_end_motion(),
        Action::ForwardWord => ctx.buffer.forward_word(),
        Action::BackwardWord => ctx.buffer.backward_word(),
        Action::BufferStart => ctx.buffer.buffer_start(),
        Action::BufferEnd => ctx.buffer.buffer_end(),
        Action::InsertChar(c) => ctx.buffer.insert_char(*c),
        Action::Newline => ctx.buffer.insert_newline(),
        Action::InsertTab => ctx.buffer.insert_tab(),
        Action::DeleteBackward => ctx.buffer.delete_backward(),
        Action::DeleteWordBackward => ctx.buffer.delete_word_backward(),
        Action::DeleteForward => ctx.buffer.delete_forward(),
        Action::KillLine => ctx.buffer.kill_line(),
        Action::Yank => ctx.buffer.yank(),
        Action::Undo => {
            ctx.buffer.undo();
            *ctx.shift_selecting = false;
        }
        Action::Redo => {
            ctx.buffer.redo();
            *ctx.shift_selecting = false;
        }
        Action::SetMark => {
            ctx.buffer.set_mark();
            *ctx.shift_selecting = false; // C-Space is a sticky mark
        }
        Action::CopyRegion => ctx.buffer.copy_region(),
        Action::KillRegion => ctx.buffer.kill_region(),
        Action::ZoomIn => *ctx.zoom = render::clamp_zoom(*ctx.zoom + render::ZOOM_STEP),
        Action::ZoomOut => *ctx.zoom = render::clamp_zoom(*ctx.zoom - render::ZOOM_STEP),
        Action::ZoomReset => *ctx.zoom = render::clamp_zoom(1.0),
        Action::PageDown => page_move(ctx.buffer, ctx.page_lines, true),
        Action::PageUp => page_move(ctx.buffer, ctx.page_lines, false),
        Action::Save => {
            if let Err(e) = ctx.buffer.save() {
                eprintln!("save failed: {e}");
            } else if let Some(p) = ctx.buffer.path() {
                eprintln!("wrote {}", p.display());
            }
        }
        Action::Quit => quit = true,
        // C-g / Escape: cancel clears any active selection (and any search).
        Action::Cancel => {
            ctx.buffer.clear_mark();
            *ctx.shift_selecting = false;
            *ctx.search = None;
        }
        // C-s / C-r: open an incremental search anchored at the cursor. (While a
        // search is already live the windowed app routes keys elsewhere; here we
        // only model the OPEN, which is all a one-frame capture needs.)
        Action::SearchForward => start_search(ctx, Direction::Forward),
        Action::SearchBackward => start_search(ctx, Direction::Backward),
        // Theme cycling is a pure UI / GPU concern (it re-tints pipelines), so the
        // editor core does nothing; the windowed `App::apply` performs the switch.
        // A headless replay simply ignores it.
        Action::CycleTheme(_) => {}
        // Toggling the caret look is a pure render concern (no buffer change); the
        // windowed `App::apply` flips the global mode. A headless replay ignores it
        // (the headless `--caret-mode` flag pins the mode instead).
        Action::ToggleCaretMode => {}
        Action::BeginPrefix | Action::Ignore => {}
    }

    // Seal the undo group after any NON-edit command so the next edit starts a
    // fresh group. Undo/Redo manage history themselves and must not seal.
    if !action.is_edit() && !matches!(action, Action::Undo | Action::Redo) {
        ctx.buffer.seal_undo_group();
    }
    // Keep the flag honest: no selection => not shift-selecting.
    if !ctx.buffer.has_selection() {
        *ctx.shift_selecting = false;
    }
    quit
}

/// Move the cursor by `page_lines` logical lines up or down, stopping at the
/// buffer boundary. The windowed app's richer visual-row paging lives in
/// `App::page_move` (it needs the GPU to measure a screenful); this is the
/// pure, deterministic fallback shared by replay and the no-GPU path.
fn page_move(buffer: &mut Buffer, page_lines: usize, down: bool) {
    for _ in 0..page_lines.max(1) {
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

/// Open an incremental search anchored at the cursor (the entry point only).
fn start_search(ctx: &mut ActionCtx, dir: Direction) {
    let origin = ctx.buffer.cursor_char();
    ctx.buffer.clear_mark();
    *ctx.shift_selecting = false;
    *ctx.search = Some(SearchState::start(origin, dir));
}
