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
    /// How many logical lines one PageScrollDown/PageScrollUp moves. The windowed
    /// app passes a screenful computed from the live viewport; headless passes a
    /// fixed value (no GPU to measure), keeping replay deterministic.
    pub scroll_page_lines: usize,
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
    /// Browse rebuild hook: build a fresh navigator overlay of the given KIND
    /// (`Browse` for C-x j, `MoveDest` for C-x m) listing the children of a given
    /// root-relative directory (`None` = the root). The kind selects the root and
    /// the filter (MoveDest is rooted at the notes root and lists folders only).
    /// The core can't read the filesystem, so open/descend/ascend delegate here.
    /// Returns `None` if the directory can't be listed (the overlay stays put).
    pub browse_to: &'a mut dyn FnMut(crate::overlay::OverlayKind, Option<String>) -> Option<OverlayState>,
}

/// The single deferred side effect an `apply_core` call signals back to its
/// caller. The pure core can't touch the filesystem, GPU, window, or the caller's
/// buffer history, so rather than PERFORM those effects it RETURNS one of these
/// for the caller to carry out. The signalling paths are mutually exclusive — at
/// most one effect fires per call — so the caller matches ONCE and leans on
/// exhaustiveness. This replaces the former cluster of `&mut` out-params.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Nothing deferred: the buffer/overlay/search mutations already applied are
    /// the whole story.
    None,
    /// `Quit` (C-x C-c): the caller exits the event loop, or stops the replay.
    Quit,
    /// C-x b: flip to the previously-opened file. The 2-deep history lives on the
    /// caller; the core just signals the toggle.
    LastBuffer,
    /// C-x n: jump to the notes project and swap in a fresh empty note buffer. The
    /// root-switch + buffer-swap are caller-level (the core never touches the
    /// filesystem/window).
    NewNote,
    /// Settings: open the config file into the buffer for editing — creating the
    /// commented default first if it is missing. The path + filesystem live on the
    /// caller.
    OpenSettings,
    /// The COMMAND PALETTE accepted (Enter on a command). The palette CLOSED itself
    /// first; the caller re-dispatches this catalog `Action` through its NORMAL
    /// apply path AFTER the close — so an overlay-opening command (Go to file) opens
    /// its overlay into the now-empty slot, and terminal commands run uniformly.
    /// Re-dispatching at the caller (not recursing in the core) is required because
    /// `App::apply` specially handles some actions the core no-ops (e.g.
    /// ToggleCaretMode).
    RunAction(Action),
    /// An overlay ACCEPTED (Enter on a selected item, or a Theme cancel-revert):
    /// the chosen value — a root-relative path for Goto/Browse, an absolute dir for
    /// Project, a notes-root-relative folder for MoveDest, or a world name for
    /// Theme — for the caller to act on (load the file / switch the root / move the
    /// note / re-tint). The core never touches the filesystem, GPU, or window.
    OverlayAccept(crate::overlay::OverlayKind, String),
}

/// Apply one resolved `action` to the editor core. `shift` is whether Shift was
/// held (so a motion extends the selection, Shift+Arrow style). Returns the one
/// deferred [`Effect`] the action signals back to the caller (`Effect::None` for
/// the common case) — the caller carries out the filesystem/window/quit work the
/// pure core can't. Mutates only what `ActionCtx` exposes; no GPU, window, or
/// clipboard.
pub fn apply_core(ctx: &mut ActionCtx, action: &Action, shift: bool) -> Effect {
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
                return Effect::None;
            }
            Action::DeleteBackward | Action::DeleteWordBackward => {
                // In the navigable explorers (Browse / MoveDest / Project),
                // Backspace doubles as "go to PARENT" once the fuzzy filter is
                // empty (file-explorer muscle memory): a non-empty query pops a
                // char (preserving filtering), an empty query ASCENDS like Left.
                let ov = ctx.overlay.as_ref().unwrap();
                let navigable = matches!(
                    ov.kind,
                    crate::overlay::OverlayKind::Browse
                        | crate::overlay::OverlayKind::MoveDest
                        | crate::overlay::OverlayKind::Project
                );
                if navigable && ov.query.is_empty() {
                    if let Some(parent) = ascend_target(ov) {
                        if let Some(next) = (ctx.browse_to)(ov.kind, parent) {
                            *ctx.overlay = Some(next);
                        }
                    }
                    return Effect::None;
                }
                ctx.overlay.as_mut().unwrap().pop();
                preview_theme(ctx.overlay.as_ref().unwrap());
                return Effect::None;
            }
            Action::NextLine => {
                ctx.overlay.as_mut().unwrap().move_sel(1);
                // LIVE PREVIEW: moving the selection in the Theme picker applies
                // that world immediately (no-op for the other overlay kinds).
                preview_theme(ctx.overlay.as_ref().unwrap());
                return Effect::None;
            }
            Action::ForwardChar => {
                // In every navigable explorer (BROWSE / MOVE-DEST / PROJECT) Right
                // DESCENDS into the highlighted folder (a no-op on a file row), so
                // navigation is uniform: Right/Enter descend, Left/Backspace ascend.
                // For the flat pickers (goto/theme/command) Right is a down-move.
                let ov = ctx.overlay.as_ref().unwrap();
                if matches!(
                    ov.kind,
                    crate::overlay::OverlayKind::Browse
                        | crate::overlay::OverlayKind::MoveDest
                        | crate::overlay::OverlayKind::Project
                ) {
                    if ov.selected_is_dir() {
                        if let Some(name) = ov.selected_value().map(|s| s.to_string()) {
                            let child = descend_target(ov, &name);
                            if let Some(next) = (ctx.browse_to)(ov.kind, Some(child)) {
                                *ctx.overlay = Some(next);
                            }
                        }
                    }
                    return Effect::None;
                }
                ctx.overlay.as_mut().unwrap().move_sel(1);
                preview_theme(ctx.overlay.as_ref().unwrap());
                return Effect::None;
            }
            Action::PreviousLine => {
                ctx.overlay.as_mut().unwrap().move_sel(-1);
                preview_theme(ctx.overlay.as_ref().unwrap());
                return Effect::None;
            }
            Action::BackwardChar => {
                // Up for goto/theme; in BROWSE / MOVE-DEST / PROJECT, Left ASCENDS
                // one directory level (rebuilds the list with the parent's
                // children). Browse/MoveDest floor at their root; Project climbs by
                // absolute path with no floor (so it can go ABOVE the workspace).
                let ov = ctx.overlay.as_ref().unwrap();
                if matches!(
                    ov.kind,
                    crate::overlay::OverlayKind::Browse
                        | crate::overlay::OverlayKind::MoveDest
                        | crate::overlay::OverlayKind::Project
                ) {
                    if let Some(parent) = ascend_target(ov) {
                        if let Some(next) = (ctx.browse_to)(ov.kind, parent) {
                            *ctx.overlay = Some(next);
                        }
                    }
                    return Effect::None;
                }
                ctx.overlay.as_mut().unwrap().move_sel(-1);
                preview_theme(ctx.overlay.as_ref().unwrap());
                return Effect::None;
            }
            Action::Newline => {
                // Accept. For BROWSE, Enter on a FOLDER descends (rebuilds the
                // list with that folder's children) instead of closing; Enter on a
                // FILE opens it (emitted as a Goto path) and closes. For Goto /
                // Project, Enter emits the chosen value and closes (Project Enter on
                // a folder PICKS it as the root — descend is on Right). A no-match
                // closes without emitting.
                let ov = ctx.overlay.as_ref().unwrap();
                if ov.kind == crate::overlay::OverlayKind::Browse {
                    let mut eff = Effect::None;
                    if let Some(name) = ov.selected_value().map(|s| s.to_string()) {
                        if ov.selected_is_dir() {
                            // Descend: parent dir = browse_dir, child = name.
                            let child = join_browse(ov.browse_dir.as_deref(), &name);
                            if let Some(next) = (ctx.browse_to)(ov.kind, Some(child)) {
                                *ctx.overlay = Some(next);
                            }
                            return Effect::None;
                        }
                        // File: open via the Goto path so the caller's open_rel
                        // loads it. The accept value is the FULL root-relative path.
                        let rel = join_browse(ov.browse_dir.as_deref(), &name);
                        eff = Effect::OverlayAccept(crate::overlay::OverlayKind::Goto, rel);
                    }
                    *ctx.overlay = None;
                    return eff;
                }
                if ov.kind == crate::overlay::OverlayKind::Project {
                    // PROJECT PICKER: the primary action of Enter is "make this
                    // folder the project". Enter on a real FOLDER ACCEPTS that
                    // folder's ABSOLUTE path as the new root (descend is on Right,
                    // not Enter). Enter on the synthetic "." row ACCEPTS the CURRENT
                    // directory. Either way we emit the absolute path the caller
                    // feeds to set_root (re-index + recompute branch/dirty), and
                    // CLOSE — never a silent no-op.
                    let dir = if ov.selected_is_dir() {
                        // The highlighted folder's absolute path = current dir + name.
                        ov.selected_value().map(|name| descend_target(ov, name))
                    } else {
                        // The "." accept-this-folder row (or no match): the current
                        // directory itself (always the absolute browse_dir).
                        ov.browse_dir.clone()
                    };
                    let eff = match dir.filter(|d| !d.is_empty()) {
                        Some(dir) => Effect::OverlayAccept(crate::overlay::OverlayKind::Project, dir),
                        None => Effect::None,
                    };
                    *ctx.overlay = None;
                    return eff;
                }
                if ov.kind == crate::overlay::OverlayKind::MoveDest {
                    // ACCEPT a destination FOLDER (notes-root-relative). Enter on a
                    // highlighted folder moves into it; a typed name matching no
                    // folder is a NEW folder to create; nothing typed/selected
                    // accepts the CURRENT level. The caller does the mkdir + move.
                    let eff = match move_dest_value(ov) {
                        Some(dest) => Effect::OverlayAccept(crate::overlay::OverlayKind::MoveDest, dest),
                        None => Effect::None,
                    };
                    *ctx.overlay = None;
                    return eff;
                }
                if ov.kind == crate::overlay::OverlayKind::Command {
                    // RUN the highlighted command. The corpus order == the catalog
                    // order, so the selected corpus index maps straight back to
                    // `COMMANDS[i]`. Close the palette FIRST so the caller's
                    // re-dispatch lands with the slot empty (an overlay-opening
                    // command can then open into it); a no-match closes silently.
                    let eff = ov
                        .selected_corpus_index()
                        .map(|i| Effect::RunAction(crate::commands::COMMANDS[i].action.clone()))
                        .unwrap_or(Effect::None);
                    *ctx.overlay = None;
                    return eff;
                }
                if ov.kind == crate::overlay::OverlayKind::Theme {
                    // COMMIT: the highlighted world is ALREADY active (live preview
                    // applied it as the selection moved), so Enter just keeps it and
                    // closes. Emit the committed name so the caller can re-tint its
                    // GPU pipelines / window title to match.
                    let eff = match ov.selected_value() {
                        Some(v) => Effect::OverlayAccept(ov.kind, v.to_string()),
                        None => Effect::None,
                    };
                    *ctx.overlay = None;
                    return eff;
                }
                let eff = match ov.selected_value() {
                    Some(v) => Effect::OverlayAccept(ov.kind, v.to_string()),
                    None => Effect::None,
                };
                *ctx.overlay = None;
                return eff;
            }
            Action::Cancel => {
                // REVERT the live preview: the Theme picker restores the world that
                // was active when it opened. Other overlays just close.
                let ov = ctx.overlay.as_ref().unwrap();
                let eff = if ov.kind == crate::overlay::OverlayKind::Theme {
                    if let Some(orig) = ov.original_theme {
                        crate::theme::set_active(orig);
                    }
                    // Signal the revert so the caller can re-tint to the restored
                    // world. The accept VALUE is the restored world's name.
                    let name = crate::theme::active().name.to_string();
                    Effect::OverlayAccept(crate::overlay::OverlayKind::Theme, name)
                } else {
                    Effect::None
                };
                *ctx.overlay = None;
                return eff;
            }
            // Any other action while the overlay is up is swallowed (the overlay
            // is modal); it never reaches the buffer.
            _ => return Effect::None,
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

    let mut effect = Effect::None;
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
        Action::PageScrollDown => scroll_page(ctx.buffer, ctx.scroll_page_lines, true),
        Action::PageScrollUp => scroll_page(ctx.buffer, ctx.scroll_page_lines, false),
        Action::Save => {
            if let Err(e) = ctx.buffer.save() {
                eprintln!("save failed: {e}");
            } else if let Some(p) = ctx.buffer.path() {
                eprintln!("wrote {}", p.display());
            }
        }
        Action::Quit => effect = Effect::Quit,
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
        // Toggling the caret look is a pure render concern (no buffer change). The
        // process-global flip lives HERE on the shared seam, so BOTH the windowed
        // `App::apply` and the headless `--keys` replay toggle through one place
        // (no double-toggle); `App` then does only the window-side follow-up (the
        // stderr log) as a post-`apply_core` side effect. A `--keys "C-x c"` capture
        // renders — and records in its sidecar — the toggled mode (Block ⇄ I-beam).
        Action::ToggleCaretMode => {
            crate::caret::toggle_mode();
        }
        // Toggling page mode is a pure render/layout concern (no buffer change). The
        // process-global flip lives HERE on the shared seam (like the caret toggle);
        // `App::apply` does the GPU re-wrap + view resync the core can't reach as a
        // post-`apply_core` side effect. A `--keys "C-x w"` capture renders (and
        // records in its sidecar) the toggled state.
        Action::TogglePageMode => {
            crate::page::toggle();
        }
        // Cycling focus mode is a pure render concern (no buffer change), like the
        // caret / page toggles. The process-global cycle lives HERE on the shared
        // seam; `App::apply` re-syncs the view afterwards (a post-`apply_core` side
        // effect) so the new dimming shows. A `--keys "C-x d"` capture renders (and
        // records in its sidecar) the new mode.
        Action::CycleFocusMode => {
            crate::focus::cycle();
        }
        // Summon the navigation overlay. The caller's `make_overlay` builds the
        // candidate list (file index for Goto, workspace children for Project);
        // if it returns None (no active project), the open is a quiet no-op.
        Action::OpenGoto => {
            *ctx.overlay = (ctx.make_overlay)(crate::overlay::OverlayKind::Goto);
        }
        // Summon the navigable PROJECT explorer at the workspace dir (browse_dir
        // = None tells the hook to start at the `--workspace` directory). Unlike
        // the other kinds this walks by ABSOLUTE path via `browse_to`, so it can
        // climb above the workspace and descend into any subtree.
        Action::OpenProject => {
            *ctx.overlay = (ctx.browse_to)(crate::overlay::OverlayKind::Project, None);
        }
        // Summon the THEME PICKER (all worlds, fuzzy-filterable, live preview).
        // The caller's `make_overlay` builds it with the world names + the active
        // index (remembered for revert-on-cancel). It opens highlighting the
        // current world, so the open frame previews exactly the active theme.
        Action::OpenThemeMenu => {
            *ctx.overlay = (ctx.make_overlay)(crate::overlay::OverlayKind::Theme);
        }
        // Cmd-P: summon the COMMAND PALETTE (the named-command fuzzy list). The
        // caller's `make_overlay` builds it from `commands::COMMANDS`.
        Action::OpenCommandPalette => {
            *ctx.overlay = (ctx.make_overlay)(crate::overlay::OverlayKind::Command);
        }
        // Summon the one-level browse navigator at the ROOT level (browse_dir =
        // None). Descend/ascend then rebuild it via `browse_to`.
        Action::OpenBrowse => {
            *ctx.overlay = (ctx.browse_to)(crate::overlay::OverlayKind::Browse, None);
        }
        // C-x b: signal the last-buffer toggle; the caller owns the 2-deep history.
        Action::LastBuffer => {
            effect = Effect::LastBuffer;
        }
        // C-x n: signal a new quick note; the caller jumps to the notes project and
        // swaps in a fresh empty note buffer (filesystem/window are caller-level).
        Action::NewNote => {
            effect = Effect::NewNote;
        }
        // C-x m: summon the MOVE-DESTINATION picker (Browse navigator over the
        // notes root, folders only). The accepted folder is acted on by the caller.
        Action::MoveNote => {
            *ctx.overlay = (ctx.browse_to)(crate::overlay::OverlayKind::MoveDest, None);
        }
        // Settings: signal the caller to open the config file into the buffer (it
        // owns the path + the create-default-if-missing step). Like NewNote, the
        // core only flips the flag; the filesystem/window work is caller-level.
        Action::OpenSettings => {
            effect = Effect::OpenSettings;
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
    effect
}

/// Move the cursor by `scroll_page_lines` logical lines up or down, stopping at
/// the buffer boundary. The windowed app's richer visual-row paging lives in
/// `App::scroll_page` (it needs the GPU to measure a screenful); this is the
/// pure, deterministic fallback shared by replay and the no-GPU path.
fn scroll_page(buffer: &mut Buffer, scroll_page_lines: usize, down: bool) {
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

/// The DESCEND target for the highlighted folder `name` in `ov`, as the value to
/// pass back to `browse_to`. `Project` navigates by ABSOLUTE path (so it can roam
/// the whole filesystem); `Browse`/`MoveDest` stay root-relative.
fn descend_target(ov: &OverlayState, name: &str) -> String {
    match ov.kind {
        crate::overlay::OverlayKind::Project => std::path::Path::new(ov.browse_dir.as_deref().unwrap_or(""))
            .join(name)
            .to_string_lossy()
            .to_string(),
        _ => join_browse(ov.browse_dir.as_deref(), name),
    }
}

/// The ASCEND target (parent directory) for `ov`. Outer `None` = can't ascend
/// (no-op). `Project` uses real `Path::parent()` with NO root floor (so Left /
/// Backspace climb ABOVE the workspace, stopping only at the filesystem root);
/// `Browse`/`MoveDest` floor at their root via [`browse_parent`].
fn ascend_target(ov: &OverlayState) -> Option<Option<String>> {
    match ov.kind {
        crate::overlay::OverlayKind::Project => std::path::Path::new(ov.browse_dir.as_deref().unwrap_or("/"))
            .parent()
            .map(|p| Some(p.to_string_lossy().to_string())),
        _ => browse_parent(ov.browse_dir.as_deref()),
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
        let mut make_overlay = |k: OverlayKind| match k {
            OverlayKind::Command => Some(OverlayState::new_command(
                crate::commands::names(),
                crate::commands::bindings(),
            )),
            _ => None,
        };
        let mut browse_to = |kind: OverlayKind, rel: Option<String>| browse_level(kind, rel);
        let mut ctx = ActionCtx {
            buffer: &mut buffer,
            shift_selecting: &mut shift,
            zoom: &mut zoom,
            search: &mut search,
            scroll_page_lines: 1,
            overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
        };
        // Mirror the old `overlay_accept` out-param: an accept effect writes the
        // chosen value into `accept`, accumulating across calls like before.
        if let Effect::OverlayAccept(kind, val) = apply_core(&mut ctx, action, false) {
            *accept = Some((kind, val));
        }
    }

    /// Like [`drive`], but also returns the palette's `run_action` out-param so a
    /// test can assert which command Enter dispatched.
    fn drive_run(
        overlay: &mut Option<OverlayState>,
        accept: &mut Option<(OverlayKind, String)>,
        action: &Action,
    ) -> Option<Action> {
        let mut buffer = Buffer::scratch();
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut make_overlay = |k: OverlayKind| match k {
            OverlayKind::Command => Some(OverlayState::new_command(
                crate::commands::names(),
                crate::commands::bindings(),
            )),
            _ => None,
        };
        let mut browse_to = |kind: OverlayKind, rel: Option<String>| browse_level(kind, rel);
        let mut ctx = ActionCtx {
            buffer: &mut buffer,
            shift_selecting: &mut shift,
            zoom: &mut zoom,
            search: &mut search,
            scroll_page_lines: 1,
            overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
        };
        // Surface BOTH the palette's run-on-Enter (returned) and any accept value
        // (mirrored into `accept`), matching the former two out-params.
        match apply_core(&mut ctx, action, false) {
            Effect::RunAction(a) => Some(a),
            Effect::OverlayAccept(kind, val) => {
                *accept = Some((kind, val));
                None
            }
            _ => None,
        }
    }

    #[test]
    fn open_settings_signals_caller() {
        // OpenSettings is a pure signal: it returns Effect::OpenSettings for the
        // caller to open the config file (no buffer/overlay change in the core).
        let mut buffer = Buffer::scratch();
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut overlay = None;
        let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
        let mut browse_to =
            |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
        let mut ctx = ActionCtx {
            buffer: &mut buffer,
            shift_selecting: &mut shift,
            zoom: &mut zoom,
            search: &mut search,
            scroll_page_lines: 1,
            overlay: &mut overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
        };
        let effect = apply_core(&mut ctx, &Action::OpenSettings, false);
        assert_eq!(effect, Effect::OpenSettings, "OpenSettings must signal the caller");
        assert!(overlay.is_none(), "OpenSettings opens no overlay");
    }

    #[test]
    fn command_palette_opens_then_filters() {
        // OpenCommandPalette summons the palette via make_overlay.
        let mut overlay: Option<OverlayState> = None;
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::OpenCommandPalette);
        let ov = overlay.as_ref().expect("palette opened");
        assert_eq!(ov.kind, OverlayKind::Command);
        // Typing "theme" fuzzy-narrows to "Switch theme" at/near the top.
        for c in "theme".chars() {
            drive(&mut overlay, &mut accept, &Action::InsertChar(c));
        }
        let ov = overlay.as_ref().unwrap();
        assert_eq!(ov.selected_value(), Some("Switch theme"));
    }

    #[test]
    fn command_palette_enter_dispatches_selected_action() {
        // Open, filter to "Go to file", Enter -> run_action == OpenGoto and the
        // palette closed (so the caller can re-dispatch into the goto overlay).
        let mut overlay: Option<OverlayState> = None;
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::OpenCommandPalette);
        for c in "goto".chars() {
            drive(&mut overlay, &mut accept, &Action::InsertChar(c));
        }
        let run = drive_run(&mut overlay, &mut accept, &Action::Newline);
        assert!(overlay.is_none(), "palette closes on accept");
        assert_eq!(run, Some(Action::OpenGoto));
        assert!(accept.is_none(), "the palette runs an action, it does not accept a value");
    }

    #[test]
    fn command_palette_run_action_reopens_into_overlay() {
        // The re-dispatch: feeding the run_action (OpenGoto) back through the core
        // with the slot empty opens the goto overlay, proving run-on-Enter chains
        // into another overlay.
        let mut overlay: Option<OverlayState> = None;
        // make_overlay here returns a real Goto overlay so the re-dispatch opens.
        let mut buffer = Buffer::scratch();
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut make_overlay = |k: OverlayKind| match k {
            OverlayKind::Goto => Some(OverlayState::new(
                OverlayKind::Goto,
                vec!["a.rs".into(), "b.rs".into()],
                vec![],
                vec![],
            )),
            _ => None,
        };
        let mut browse_to = |kind: OverlayKind, rel: Option<String>| browse_level(kind, rel);
        let mut ctx = ActionCtx {
            buffer: &mut buffer,
            shift_selecting: &mut shift,
            zoom: &mut zoom,
            search: &mut search,
            scroll_page_lines: 1,
            overlay: &mut overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
        };
        // Re-dispatch OpenGoto (the palette already closed) -> goto overlay opens.
        apply_core(&mut ctx, &Action::OpenGoto, false);
        assert_eq!(overlay.as_ref().map(|o| o.kind), Some(OverlayKind::Goto));
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

    /// Drive one action with a CUSTOM `browse_to` (the project explorer tests use
    /// a real temp-dir tree so absolute-path ascend/descend exercise the FS).
    fn drive_bt(
        overlay: &mut Option<OverlayState>,
        accept: &mut Option<(OverlayKind, String)>,
        browse_to: &mut dyn FnMut(OverlayKind, Option<String>) -> Option<OverlayState>,
        action: &Action,
    ) {
        let mut buffer = Buffer::scratch();
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
        let mut ctx = ActionCtx {
            buffer: &mut buffer,
            shift_selecting: &mut shift,
            zoom: &mut zoom,
            search: &mut search,
            scroll_page_lines: 1,
            overlay,
            make_overlay: &mut make_overlay,
            browse_to,
        };
        // Mirror the old `overlay_accept` out-param into `accept` (accumulating).
        if let Effect::OverlayAccept(kind, val) = apply_core(&mut ctx, action, false) {
            *accept = Some((kind, val));
        }
    }

    /// Build a unique temp `ws/` tree for the project explorer tests:
    /// `ws/child-a/sub/`, `ws/child-b/`.
    fn proj_tree() -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut ws = std::env::temp_dir();
        ws.push(format!("awl_proj_test_{}_{}", std::process::id(), id));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(ws.join("child-a/sub")).unwrap();
        std::fs::create_dir_all(ws.join("child-b")).unwrap();
        ws
    }

    /// A `browse_to` that drives the PROJECT explorer over an absolute temp tree,
    /// exactly like the windowed app's hook (folders-only + synthetic "." row).
    fn project_browse(ws: &std::path::Path, rel: Option<String>) -> Option<OverlayState> {
        let dir = rel.unwrap_or_else(|| ws.to_string_lossy().to_string());
        let folders: Vec<(String, bool)> =
            crate::index::list_dir_level(std::path::Path::new(&dir), None)
                .into_iter()
                .filter(|e| e.is_dir)
                .map(|e| (e.name, e.is_git))
                .collect();
        Some(OverlayState::new_project(dir, folders))
    }

    #[test]
    fn switch_project_enter_picks_highlighted_folder() {
        let ws = proj_tree();
        let mut browse_to = |k: OverlayKind, rel: Option<String>| {
            assert_eq!(k, OverlayKind::Project);
            project_browse(&ws, rel)
        };
        // Open at ws: corpus is [".", child-a, child-b], default-selected on the
        // first real folder (child-a). Enter PICKS it as the new root (the primary
        // action of the project picker) — it does NOT descend.
        let mut overlay = browse_to(OverlayKind::Project, None);
        let mut accept = None;
        assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("child-a"));
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::Newline);
        assert!(overlay.is_none(), "Enter on a folder PICKS it and closes");
        assert_eq!(
            accept,
            Some((
                OverlayKind::Project,
                ws.join("child-a").to_string_lossy().to_string()
            )),
            "Enter accepts the highlighted folder's ABSOLUTE path"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn switch_project_right_descends_into_child() {
        let ws = proj_tree();
        let mut browse_to = |k: OverlayKind, rel: Option<String>| {
            assert_eq!(k, OverlayKind::Project);
            project_browse(&ws, rel)
        };
        // Open at ws, selection on child-a. Right DESCENDS into it (drill in to
        // pick a subfolder); the overlay stays open with child-a's contents.
        let mut overlay = browse_to(OverlayKind::Project, None);
        let mut accept = None;
        assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("child-a"));
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::ForwardChar);
        let ov = overlay.as_ref().expect("still open after descend");
        assert_eq!(
            ov.browse_dir.as_deref(),
            Some(ws.join("child-a").to_string_lossy().as_ref())
        );
        // The descended level lists child-a's subfolder `sub`.
        assert!(ov.item_strings().iter().any(|s| s.contains("sub")), "{:?}", ov.item_strings());
        assert!(accept.is_none(), "descend must not accept");
        // Right again descends (into `sub`).
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::ForwardChar);
        assert_eq!(
            overlay.as_ref().unwrap().browse_dir.as_deref(),
            Some(ws.join("child-a/sub").to_string_lossy().as_ref())
        );
        // `sub` has no subfolders, so selection rests on the "." row; Enter there
        // PICKS the drilled-in current directory (child-a/sub) as the root.
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::Newline);
        assert!(overlay.is_none(), "Enter picks the drilled-in directory");
        assert_eq!(
            accept,
            Some((
                OverlayKind::Project,
                ws.join("child-a/sub").to_string_lossy().to_string()
            )),
            "drilled-in pick is its absolute path"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// C-f / C-b reach the navigable intercept AS ForwardChar / BackwardChar while
    /// the overlay is open (the keymap is overlay-unaware, so the chord resolves the
    /// same as the arrows). Resolve the chords through the REAL keymap, then drive
    /// the resulting actions: C-f DESCENDS into the highlighted child (same as Right)
    /// and C-b ASCENDS to the parent (same as Left).
    #[test]
    fn switch_project_c_f_descends_c_b_ascends() {
        use crate::keymap::KeymapState;
        use winit::keyboard::{Key, ModifiersState, SmolStr};
        let ctrl = winit::event::Modifiers::from(ModifiersState::CONTROL);
        let mut km = KeymapState::new();
        // C-f and C-b resolve to the SAME actions the arrows do.
        let c_f = km.resolve(&Key::Character(SmolStr::new("f")), &ctrl);
        let c_b = km.resolve(&Key::Character(SmolStr::new("b")), &ctrl);
        assert_eq!(c_f, Action::ForwardChar, "C-f must resolve to ForwardChar");
        assert_eq!(c_b, Action::BackwardChar, "C-b must resolve to BackwardChar");

        let ws = proj_tree();
        let mut browse_to = |k: OverlayKind, rel: Option<String>| project_browse(&ws, rel);
        let mut overlay = browse_to(OverlayKind::Project, None);
        let mut accept = None;
        assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("child-a"));
        // C-f (ForwardChar) DESCENDS into child-a, overlay still open at its level.
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &c_f);
        let ov = overlay.as_ref().expect("still open after C-f descend");
        assert_eq!(
            ov.browse_dir.as_deref(),
            Some(ws.join("child-a").to_string_lossy().as_ref())
        );
        assert!(accept.is_none(), "descend must not accept");
        // C-b (BackwardChar) ASCENDS back to the workspace level.
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &c_b);
        assert_eq!(
            overlay.as_ref().unwrap().browse_dir.as_deref(),
            Some(ws.to_string_lossy().as_ref()),
            "C-b ascends back to the workspace"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// Enter on a Project FOLDER SELECTS it as the root (does NOT descend): the
    /// overlay closes and the accept value is that folder's absolute path. Descending
    /// is Right / C-f only. (Companion to `switch_project_right_descends_into_child`.)
    #[test]
    fn switch_project_enter_selects_does_not_descend() {
        let ws = proj_tree();
        let mut browse_to = |k: OverlayKind, rel: Option<String>| {
            assert_eq!(k, OverlayKind::Project);
            project_browse(&ws, rel)
        };
        let mut overlay = browse_to(OverlayKind::Project, None);
        let mut accept = None;
        assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("child-a"));
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::Newline);
        assert!(overlay.is_none(), "Enter on a folder SELECTS + closes (no descend)");
        assert_eq!(
            accept,
            Some((
                OverlayKind::Project,
                ws.join("child-a").to_string_lossy().to_string()
            )),
            "Enter selects the highlighted folder, it does not drill into it"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn switch_project_ascends_to_parent() {
        let ws = proj_tree();
        let mut browse_to = |_k: OverlayKind, rel: Option<String>| project_browse(&ws, rel);
        let mut overlay = browse_to(OverlayKind::Project, None);
        let mut accept = None;
        // Backspace (empty query) ASCENDS to ws's PARENT — ABOVE the workspace.
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::DeleteBackward);
        let parent = ws.parent().unwrap().to_string_lossy().to_string();
        let ov = overlay.as_ref().unwrap();
        assert_eq!(ov.browse_dir.as_deref(), Some(parent.as_str()));
        // ws itself now appears as a child folder of its parent.
        let ws_name = ws.file_name().unwrap().to_str().unwrap();
        assert!(ov.item_strings().iter().any(|s| s.contains(ws_name)));
        // Left ascends one MORE level (no root floor for Project).
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::BackwardChar);
        let grandparent = ws.parent().unwrap().parent().unwrap().to_string_lossy().to_string();
        assert_eq!(overlay.as_ref().unwrap().browse_dir.as_deref(), Some(grandparent.as_str()));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn switch_project_accept_current_dir_sets_root() {
        let ws = proj_tree();
        let ws_str = ws.to_string_lossy().to_string();
        let mut browse_to = |_k: OverlayKind, rel: Option<String>| project_browse(&ws, rel);
        let mut overlay = browse_to(OverlayKind::Project, None);
        let mut accept = None;
        // Up moves from the first folder onto the synthetic "." accept row...
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::PreviousLine);
        assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("."));
        // ...and Enter ACCEPTS the current directory (ws) as the new root.
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::Newline);
        assert!(overlay.is_none(), "accept closes the explorer");
        assert_eq!(accept, Some((OverlayKind::Project, ws_str)));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn browse_backspace_ascends() {
        // Backspace (empty query) ascends in Browse, same as Left.
        let mut overlay = browse_level(OverlayKind::Browse, Some("docs".to_string()));
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::DeleteBackward);
        assert_eq!(overlay.as_ref().unwrap().browse_dir, None, "Backspace ascends docs -> root");
    }

    #[test]
    fn move_dest_backspace_ascends() {
        let mut overlay = browse_level(OverlayKind::MoveDest, Some("docs".to_string()));
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::DeleteBackward);
        assert_eq!(overlay.as_ref().unwrap().browse_dir, None, "Backspace ascends docs -> root");
    }

    #[test]
    fn browse_backspace_pops_filter_before_ascending() {
        // With a non-empty fuzzy query, Backspace pops a CHAR (keeps the level);
        // only an EMPTY query ascends. Preserves type-to-filter within a level.
        let mut overlay = browse_level(OverlayKind::Browse, Some("docs".to_string()));
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::InsertChar('g'));
        drive(&mut overlay, &mut accept, &Action::DeleteBackward);
        let ov = overlay.as_ref().unwrap();
        assert_eq!(ov.query, "");
        assert_eq!(ov.browse_dir.as_deref(), Some("docs"), "popping the filter must not ascend");
        drive(&mut overlay, &mut accept, &Action::DeleteBackward);
        assert_eq!(overlay.as_ref().unwrap().browse_dir, None, "now-empty query ascends");
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
