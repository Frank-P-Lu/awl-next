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
use crate::overlay::OverlayState;
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
    /// The SUMMONED navigation overlay. `None` = editing normally; `Some` = the
    /// go-to / switch-project overlay is open, and while it is, typed chars edit
    /// the overlay query (NOT the buffer), Up/Down move the selection, Enter
    /// accepts, Esc/C-g cancels. Putting this in the shared core (not just `App`)
    /// is what makes the overlay drivable from the headless `--keys` replay.
    pub overlay: &'a mut Option<OverlayState>,
    /// The active project context the overlay needs when it OPENS: a builder that
    /// produces a fresh `OverlayState` for a given kind. The core can't read the
    /// filesystem itself (and headless replay must stay deterministic), so the
    /// caller injects this; `OpenGoto`/`OpenProject` invoke it.
    pub make_overlay: &'a mut dyn FnMut(crate::overlay::OverlayKind) -> Option<OverlayState>,
    /// Out-param: when the overlay ACCEPTS (Enter on a selected item), the chosen
    /// value (a root-relative path for Goto, a child name for Project) is written
    /// here for the caller to act on (load the file / switch the root). The core
    /// never touches the filesystem, GPU, or window, so the side effect happens in
    /// the caller.
    pub overlay_accept: &'a mut Option<(crate::overlay::OverlayKind, String)>,
    /// Browse rebuild hook: build a fresh navigator overlay of the given KIND
    /// (`Browse` for C-x j, `MoveDest` for C-x m) listing the children of a given
    /// root-relative directory (`None` = the root). The kind selects the root and
    /// the filter (MoveDest is rooted at the notes root and lists folders only).
    /// The core can't read the filesystem, so open/descend/ascend delegate here.
    /// Returns `None` if the directory can't be listed (the overlay stays put).
    pub browse_to: &'a mut dyn FnMut(crate::overlay::OverlayKind, Option<String>) -> Option<OverlayState>,
    /// Out-param: set true when `LastBuffer` (C-x b) fires, so the caller flips to
    /// the previously-opened file. The history (a tiny 2-deep stack) lives on the
    /// caller; the core just signals the toggle.
    pub last_buffer: &'a mut bool,
    /// Out-param: set true when `NewNote` (C-x n) fires, so the caller jumps to the
    /// notes project and swaps in a fresh empty note buffer. The root-switch and
    /// buffer-swap are caller-level (the core never touches the filesystem/window),
    /// so the core just signals the gesture, mirroring `last_buffer`.
    pub new_note: &'a mut bool,
}

/// Apply one resolved `action` to the editor core. `shift` is whether Shift was
/// held (so a motion extends the selection, Shift+Arrow style). Returns `true`
/// if the action is `Quit` (the caller decides what "quit" means — exit the
/// event loop, or stop a replay). Mutates only what `ActionCtx` exposes; no GPU,
/// window, or clipboard.
pub fn apply_core(ctx: &mut ActionCtx, action: &Action, shift: bool) -> bool {
    // OVERLAY INTERCEPT. When the summoned navigation overlay is open, it OWNS
    // every key: printable chars extend the overlay query (never the rope),
    // Up/Down (and C-n/C-p, which resolve to NextLine/PreviousLine) move the
    // selection, Enter accepts the highlighted item, Esc/C-g cancels. Routing
    // this through the shared core (rather than only in `App`) is exactly what
    // makes the overlay drivable under `--keys` — the same mistake the isearch
    // panel made (its query routing lives in `App`, so `--keys` can't type into
    // it) is deliberately avoided here.
    if ctx.overlay.is_some() {
        match action {
            Action::InsertChar(c) => {
                ctx.overlay.as_mut().unwrap().push(*c);
                // Typing to fuzzy-filter also PREVIEWS the new top/selected match.
                preview_theme(ctx.overlay.as_ref().unwrap());
                return false;
            }
            Action::DeleteBackward | Action::DeleteWordBackward => {
                ctx.overlay.as_mut().unwrap().pop();
                preview_theme(ctx.overlay.as_ref().unwrap());
                return false;
            }
            Action::NextLine => {
                ctx.overlay.as_mut().unwrap().move_sel(1);
                // LIVE PREVIEW: moving the selection in the Theme picker applies
                // that world immediately (no-op for the other overlay kinds).
                preview_theme(ctx.overlay.as_ref().unwrap());
                return false;
            }
            Action::ForwardChar => {
                // Down for goto/switch; in BROWSE, ForwardChar (Right) is unused
                // for motion — keep it as a plain down-move so arrow keys still
                // navigate the list. (Descend is Enter; ascend is Left.) In the
                // MOVE-DESTINATION picker, Right DESCENDS into the highlighted
                // folder (Enter is reserved for ACCEPT there).
                let ov = ctx.overlay.as_ref().unwrap();
                if ov.kind == crate::overlay::OverlayKind::MoveDest {
                    if ov.selected_is_dir() {
                        if let Some(name) = ov.selected_value().map(|s| s.to_string()) {
                            let child = join_browse(ov.browse_dir.as_deref(), &name);
                            if let Some(next) = (ctx.browse_to)(ov.kind, Some(child)) {
                                *ctx.overlay = Some(next);
                            }
                        }
                    }
                    return false;
                }
                ctx.overlay.as_mut().unwrap().move_sel(1);
                preview_theme(ctx.overlay.as_ref().unwrap());
                return false;
            }
            Action::PreviousLine => {
                ctx.overlay.as_mut().unwrap().move_sel(-1);
                preview_theme(ctx.overlay.as_ref().unwrap());
                return false;
            }
            Action::BackwardChar => {
                // Up for goto/switch; in BROWSE / MOVE-DEST, Left ASCENDS one
                // directory level (rebuilds the list with the parent's children).
                // At the root this is a no-op (parent of the root level is the root).
                let ov = ctx.overlay.as_ref().unwrap();
                if matches!(
                    ov.kind,
                    crate::overlay::OverlayKind::Browse | crate::overlay::OverlayKind::MoveDest
                ) {
                    if let Some(parent) = browse_parent(ov.browse_dir.as_deref()) {
                        if let Some(next) = (ctx.browse_to)(ov.kind, parent) {
                            *ctx.overlay = Some(next);
                        }
                    }
                    return false;
                }
                ctx.overlay.as_mut().unwrap().move_sel(-1);
                preview_theme(ctx.overlay.as_ref().unwrap());
                return false;
            }
            Action::Newline => {
                // Accept. For BROWSE, Enter on a FOLDER descends (rebuilds the
                // list with that folder's children) instead of closing; Enter on a
                // FILE opens it (emitted as a Goto path) and closes. For Goto /
                // Project, Enter emits the chosen value and closes. A no-match
                // closes without emitting.
                let ov = ctx.overlay.as_ref().unwrap();
                if ov.kind == crate::overlay::OverlayKind::Browse {
                    if let Some(name) = ov.selected_value().map(|s| s.to_string()) {
                        if ov.selected_is_dir() {
                            // Descend: parent dir = browse_dir, child = name.
                            let child = join_browse(ov.browse_dir.as_deref(), &name);
                            if let Some(next) = (ctx.browse_to)(ov.kind, Some(child)) {
                                *ctx.overlay = Some(next);
                            }
                            return false;
                        }
                        // File: open via the Goto path so the caller's open_rel
                        // loads it. The accept value is the FULL root-relative path.
                        let rel = join_browse(ov.browse_dir.as_deref(), &name);
                        *ctx.overlay_accept = Some((crate::overlay::OverlayKind::Goto, rel));
                    }
                    *ctx.overlay = None;
                    return false;
                }
                if ov.kind == crate::overlay::OverlayKind::MoveDest {
                    // ACCEPT a destination FOLDER (notes-root-relative). Enter on a
                    // highlighted folder moves into it; a typed name matching no
                    // folder is a NEW folder to create; nothing typed/selected
                    // accepts the CURRENT level. The caller does the mkdir + move.
                    if let Some(dest) = move_dest_value(ov) {
                        *ctx.overlay_accept = Some((crate::overlay::OverlayKind::MoveDest, dest));
                    }
                    *ctx.overlay = None;
                    return false;
                }
                if ov.kind == crate::overlay::OverlayKind::Theme {
                    // COMMIT: the highlighted world is ALREADY active (live preview
                    // applied it as the selection moved), so Enter just keeps it and
                    // closes. Emit the committed name so the caller can re-tint its
                    // GPU pipelines / window title to match.
                    if let Some(v) = ov.selected_value() {
                        *ctx.overlay_accept = Some((ov.kind, v.to_string()));
                    }
                    *ctx.overlay = None;
                    return false;
                }
                if let Some(v) = ov.selected_value() {
                    *ctx.overlay_accept = Some((ov.kind, v.to_string()));
                }
                *ctx.overlay = None;
                return false;
            }
            Action::Cancel => {
                // REVERT the live preview: the Theme picker restores the world that
                // was active when it opened. Other overlays just close.
                let ov = ctx.overlay.as_ref().unwrap();
                if ov.kind == crate::overlay::OverlayKind::Theme {
                    if let Some(orig) = ov.original_theme {
                        crate::theme::set_active(orig);
                    }
                    // Signal the revert so the caller can re-tint to the restored
                    // world. The accept VALUE is the restored world's name.
                    let name = crate::theme::active().name.to_string();
                    *ctx.overlay_accept = Some((crate::overlay::OverlayKind::Theme, name));
                }
                *ctx.overlay = None;
                return false;
            }
            // Any other action while the overlay is up is swallowed (the overlay
            // is modal); it never reaches the buffer.
            _ => return false,
        }
    }

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
        // Toggling the caret look is a pure render concern (no buffer change); the
        // windowed `App::apply` flips the global mode. A headless replay ignores it
        // (the headless `--caret-mode` flag pins the mode instead).
        Action::ToggleCaretMode => {}
        // Summon the navigation overlay. The caller's `make_overlay` builds the
        // candidate list (file index for Goto, workspace children for Project);
        // if it returns None (no active project), the open is a quiet no-op.
        Action::OpenGoto => {
            *ctx.overlay = (ctx.make_overlay)(crate::overlay::OverlayKind::Goto);
        }
        Action::OpenProject => {
            *ctx.overlay = (ctx.make_overlay)(crate::overlay::OverlayKind::Project);
        }
        // Summon the THEME PICKER (the 8 worlds, fuzzy-filterable, live preview).
        // The caller's `make_overlay` builds it with the world names + the active
        // index (remembered for revert-on-cancel). It opens highlighting the
        // current world, so the open frame previews exactly the active theme.
        Action::OpenThemeMenu => {
            *ctx.overlay = (ctx.make_overlay)(crate::overlay::OverlayKind::Theme);
        }
        // Summon the one-level browse navigator at the ROOT level (browse_dir =
        // None). Descend/ascend then rebuild it via `browse_to`.
        Action::OpenBrowse => {
            *ctx.overlay = (ctx.browse_to)(crate::overlay::OverlayKind::Browse, None);
        }
        // C-x b: signal the last-buffer toggle; the caller owns the 2-deep history.
        Action::LastBuffer => {
            *ctx.last_buffer = true;
        }
        // C-x n: signal a new quick note; the caller jumps to the notes project and
        // swaps in a fresh empty note buffer (filesystem/window are caller-level).
        Action::NewNote => {
            *ctx.new_note = true;
        }
        // C-x m: summon the MOVE-DESTINATION picker (Browse navigator over the
        // notes root, folders only). The accepted folder is acted on by the caller.
        Action::MoveNote => {
            *ctx.overlay = (ctx.browse_to)(crate::overlay::OverlayKind::MoveDest, None);
        }
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

/// Join a browse directory (root-relative, `None` = root) with a child entry
/// name into a single root-relative, forward-slashed path.
fn join_browse(dir: Option<&str>, name: &str) -> String {
    match dir {
        Some(d) if !d.is_empty() => format!("{d}/{name}"),
        _ => name.to_string(),
    }
}

/// The PARENT browse directory of `dir` (root-relative, `None` = root), as the
/// value to pass back to `browse_to`. Returns `None` when already at the root
/// (Left there is a no-op). One level up: `docs/api` -> `Some("docs")`, `docs`
/// -> `Some(None)` (i.e. the root), root -> `None`.
fn browse_parent(dir: Option<&str>) -> Option<Option<String>> {
    match dir {
        None => None, // already at root; nothing above
        Some(d) => match d.rsplit_once('/') {
            Some((parent, _)) => Some(Some(parent.to_string())),
            None => Some(None), // one level deep -> back to root
        },
    }
}

/// The accepted MOVE destination for a `MoveDest` overlay, as a notes-root-relative
/// directory path (`""` = the notes root itself). Precedence: a highlighted FOLDER
/// (move into it); else a non-empty typed QUERY that matched no folder (a NEW folder
/// to create at this level); else the CURRENT level. The caller mkdir's + moves.
fn move_dest_value(ov: &OverlayState) -> Option<String> {
    // A highlighted folder is the destination (descend-as-accept).
    if let Some(name) = ov.selected_value() {
        if ov.selected_is_dir() {
            return Some(join_browse(ov.browse_dir.as_deref(), name));
        }
    }
    // No folder highlighted: a typed name becomes a NEW folder at this level.
    let q = ov.query.trim();
    if !q.is_empty() {
        return Some(join_browse(ov.browse_dir.as_deref(), q));
    }
    // Nothing typed or selected: accept the current level (`None` root -> "").
    Some(ov.browse_dir.clone().unwrap_or_default())
}

/// LIVE PREVIEW for the Theme picker: if `ov` is the Theme overlay, apply its
/// currently-highlighted world to the process-global active theme so the rendered
/// frame shows it immediately. A no-op for every other overlay kind (and when no
/// item matches the filter). Driven from the overlay move / filter paths so the
/// preview is identical under `--keys` and live.
fn preview_theme(ov: &OverlayState) {
    if ov.kind != crate::overlay::OverlayKind::Theme {
        return;
    }
    if let Some(name) = ov.selected_value() {
        crate::theme::set_active_by_name(name);
    }
}

/// Open an incremental search anchored at the cursor (the entry point only).
fn start_search(ctx: &mut ActionCtx, dir: Direction) {
    let origin = ctx.buffer.cursor_char();
    ctx.buffer.clear_mark();
    *ctx.shift_selecting = false;
    *ctx.search = Some(SearchState::start(origin, dir));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::{OverlayKind, OverlayState};

    #[test]
    fn browse_path_helpers() {
        assert_eq!(join_browse(None, "docs"), "docs");
        assert_eq!(join_browse(Some("docs"), "guide.md"), "docs/guide.md");
        assert_eq!(join_browse(Some(""), "x"), "x");
        // ascend: root -> nothing; one level -> root; nested -> parent.
        assert_eq!(browse_parent(None), None);
        assert_eq!(browse_parent(Some("docs")), Some(None));
        assert_eq!(browse_parent(Some("docs/api")), Some(Some("docs".to_string())));
    }

    /// A tiny in-memory tree for the browse navigator: root has `docs/` (dir) and
    /// `README.md` (file); `docs/` has `guide.md` (file) and `api/` (dir). The
    /// `kind` is threaded through so MoveDest rebuilds stay MoveDest.
    fn browse_level(kind: OverlayKind, rel: Option<String>) -> Option<OverlayState> {
        let (corpus, git, is_dir): (Vec<String>, Vec<bool>, Vec<bool>) = match rel.as_deref() {
            None => (
                vec!["docs".into(), "README.md".into()],
                vec![false, false],
                vec![true, false],
            ),
            Some("docs") => (
                vec!["api".into(), "guide.md".into()],
                vec![false, false],
                vec![true, false],
            ),
            _ => (vec![], vec![], vec![]),
        };
        Some(OverlayState::new_marked(
            kind, corpus, git, is_dir, vec![], vec![], rel,
        ))
    }

    /// Drive a single action through `apply_core` with a browse_to backed by
    /// `browse_level`, returning the resulting (overlay, accept).
    fn drive(
        overlay: &mut Option<OverlayState>,
        accept: &mut Option<(OverlayKind, String)>,
        action: &Action,
    ) {
        let mut buffer = Buffer::scratch();
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut last_buffer = false;
        let mut new_note = false;
        let mut make_overlay = |_k: OverlayKind| None;
        let mut browse_to = |kind: OverlayKind, rel: Option<String>| browse_level(kind, rel);
        let mut ctx = ActionCtx {
            buffer: &mut buffer,
            shift_selecting: &mut shift,
            zoom: &mut zoom,
            search: &mut search,
            page_lines: 1,
            overlay,
            make_overlay: &mut make_overlay,
            overlay_accept: accept,
            browse_to: &mut browse_to,
            last_buffer: &mut last_buffer,
            new_note: &mut new_note,
        };
        apply_core(&mut ctx, action, false);
    }

    #[test]
    fn browse_descends_into_folder_then_opens_file() {
        // Open at root level.
        let mut overlay: Option<OverlayState> = browse_level(OverlayKind::Browse, None);
        let mut accept: Option<(OverlayKind, String)> = None;
        // Selected row is `docs` (a folder) -> Enter DESCENDS, not closes.
        drive(&mut overlay, &mut accept, &Action::Newline);
        let ov = overlay.as_ref().expect("still open after descend");
        assert_eq!(ov.browse_dir.as_deref(), Some("docs"));
        let items = ov.item_strings();
        assert!(items.iter().any(|s| s.contains("guide.md")), "got {items:?}");
        assert!(accept.is_none(), "descend must not accept");
        // Move selection past the `api` folder onto guide.md, then Enter opens it.
        drive(&mut overlay, &mut accept, &Action::NextLine);
        drive(&mut overlay, &mut accept, &Action::Newline);
        assert!(overlay.is_none(), "overlay closes on file open");
        assert_eq!(accept, Some((OverlayKind::Goto, "docs/guide.md".to_string())));
    }

    #[test]
    fn browse_left_ascends() {
        // Start one level deep (in docs/).
        let mut overlay: Option<OverlayState> =
            browse_level(OverlayKind::Browse, Some("docs".to_string()));
        let mut accept = None;
        // Left ASCENDS back to the root level.
        drive(&mut overlay, &mut accept, &Action::BackwardChar);
        let ov = overlay.as_ref().expect("still open after ascend");
        assert_eq!(ov.browse_dir, None, "ascend from docs -> root");
        assert!(ov.item_strings().iter().any(|s| s.contains("docs")));
        // Left at the root is a no-op (stays at root, still open).
        drive(&mut overlay, &mut accept, &Action::BackwardChar);
        assert_eq!(overlay.as_ref().unwrap().browse_dir, None);
    }

    #[test]
    fn move_dest_right_descends_left_ascends() {
        // MoveDest opens at root; Right DESCENDS into the highlighted folder.
        let mut overlay: Option<OverlayState> = browse_level(OverlayKind::MoveDest, None);
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::ForwardChar);
        let ov = overlay.as_ref().expect("still open after descend");
        assert_eq!(ov.kind, OverlayKind::MoveDest, "descend keeps MoveDest kind");
        assert_eq!(ov.browse_dir.as_deref(), Some("docs"));
        assert!(accept.is_none(), "descend must not accept");
        // Left ASCENDS back to the root.
        drive(&mut overlay, &mut accept, &Action::BackwardChar);
        assert_eq!(overlay.as_ref().unwrap().browse_dir, None);
    }

    #[test]
    fn move_dest_enter_accepts_highlighted_folder() {
        // Enter on the highlighted `docs` folder accepts it as the destination.
        let mut overlay: Option<OverlayState> = browse_level(OverlayKind::MoveDest, None);
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::Newline);
        assert!(overlay.is_none(), "accept closes the picker");
        assert_eq!(accept, Some((OverlayKind::MoveDest, "docs".to_string())));
    }

    #[test]
    fn move_dest_type_to_create_folder() {
        // Type a name that matches no listed folder -> accept CREATES that folder.
        let mut overlay: Option<OverlayState> = browse_level(OverlayKind::MoveDest, None);
        let mut accept = None;
        for c in "ideas".chars() {
            drive(&mut overlay, &mut accept, &Action::InsertChar(c));
        }
        // "ideas" matches nothing in {docs, README.md}, so the query is the dest.
        drive(&mut overlay, &mut accept, &Action::Newline);
        assert_eq!(accept, Some((OverlayKind::MoveDest, "ideas".to_string())));
    }

    /// Serialize the theme-picker tests: they mutate the process-global ACTIVE
    /// theme, and cargo runs tests in parallel.
    static THEME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn theme_overlay() -> Option<OverlayState> {
        let names: Vec<String> = crate::theme::THEMES
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        Some(OverlayState::new_theme(names, crate::theme::active_index()))
    }

    #[test]
    fn theme_move_previews_live() {
        let _g = THEME_LOCK.lock().unwrap();
        crate::theme::set_active(0); // Tawny
        let mut overlay = theme_overlay();
        let mut accept = None;
        // Opens highlighting the active world (Tawny), still active.
        assert_eq!(crate::theme::active().name, "Tawny");
        // Down once previews world index 1 (Potoroo) IMMEDIATELY.
        drive(&mut overlay, &mut accept, &Action::NextLine);
        assert_eq!(crate::theme::active().name, crate::theme::THEMES[1].name);
        // Down again previews world index 2.
        drive(&mut overlay, &mut accept, &Action::NextLine);
        assert_eq!(crate::theme::active().name, crate::theme::THEMES[2].name);
        assert_eq!(overlay.as_ref().unwrap().selected, 2);
        crate::theme::set_active(0);
    }

    #[test]
    fn theme_enter_commits_previewed_world() {
        let _g = THEME_LOCK.lock().unwrap();
        crate::theme::set_active(0);
        let mut overlay = theme_overlay();
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::NextLine); // preview world 1
        drive(&mut overlay, &mut accept, &Action::Newline); // COMMIT
        assert!(overlay.is_none(), "Enter closes the picker");
        assert_eq!(crate::theme::active().name, crate::theme::THEMES[1].name);
        assert_eq!(
            accept,
            Some((OverlayKind::Theme, crate::theme::THEMES[1].name.to_string()))
        );
        crate::theme::set_active(0);
    }

    #[test]
    fn theme_cancel_reverts_to_starting_world() {
        let _g = THEME_LOCK.lock().unwrap();
        crate::theme::set_active(0); // start on Tawny
        let mut overlay = theme_overlay();
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::NextLine); // preview world 1
        drive(&mut overlay, &mut accept, &Action::NextLine); // preview world 2
        assert_eq!(crate::theme::active().name, crate::theme::THEMES[2].name);
        drive(&mut overlay, &mut accept, &Action::Cancel); // REVERT
        assert!(overlay.is_none(), "Cancel closes the picker");
        assert_eq!(crate::theme::active().name, "Tawny", "reverted to start");
        crate::theme::set_active(0);
    }

    #[test]
    fn theme_typing_filters_and_previews() {
        let _g = THEME_LOCK.lock().unwrap();
        crate::theme::set_active(0);
        let mut overlay = theme_overlay();
        let mut accept = None;
        // Type "quo" -> Quokka is the top match, previewed immediately.
        drive(&mut overlay, &mut accept, &Action::InsertChar('q'));
        drive(&mut overlay, &mut accept, &Action::InsertChar('u'));
        drive(&mut overlay, &mut accept, &Action::InsertChar('o'));
        assert_eq!(crate::theme::active().name, "Quokka");
        crate::theme::set_active(0);
    }
}
