//! DRIVE-TEST helpers for `actions::tests` -- split out of the former
//! monolithic `actions::tests` module (2026-07 code-organization pass) into
//! this `actions/tests/` directory; every test's NAME is unchanged, only its
//! module path grew one segment (`actions::tests::foo` ->
//! `actions::tests::<area>::foo`). This file owns every shared drive-helper
//! (`drive*`, `browse_level`, `settings_*`, `theme_overlay`, `proj_tree`,
//! `all_actions`, `smoke_command_kind`, ...) since several are called from
//! tests in more than one area; each child re-derives `actions` root access
//! via its own `use super::super::*;` plus a targeted `use super::{..};` for
//! whichever helpers it actually calls.

use super::*;
use crate::overlay::OverlayKind;

mod format_editing;
mod overlay_drive;
mod picker_misc_smoke;
mod pickers_nav;
mod recoil_flinch;
mod save_feedback;

/// A tiny in-memory tree for the browse navigator: root has `docs/` (dir) and
/// `README.md` (file); `docs/` has `guide.md` (file) and `api/` (dir). The
/// `kind` is threaded through so MoveDest rebuilds stay MoveDest.
pub(super) fn browse_level(kind: OverlayKind, rel: Option<String>) -> Option<OverlayState> {
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
pub(super) fn drive(
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
        // The SETTINGS breadcrumb target: re-summoned when a value-pick launched
        // FROM Settings pops back on commit (the one parent that retains its child).
        OverlayKind::Settings => Some(settings_overlay()),
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
        oracle: None,
    };
    // Mirror the old `overlay_accept` out-param: an accept effect writes the
    // chosen value into `accept`, accumulating across calls like before.
    if let Effect::OverlayAccept(kind, val) = apply_core(&mut ctx, action, false) {
        *accept = Some((kind, val));
    }
}

/// Like [`drive`], but also returns the palette's `run_action` out-param so a
/// test can assert which command Enter dispatched.
pub(super) fn drive_run(
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
        oracle: None,
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

/// Drive one action against a mutable overlay through `apply_core`, returning the
/// raw [`Effect`] — for the rebind-menu flow assertions.
pub(super) fn drive_eff(overlay: &mut Option<OverlayState>, action: &Action) -> Effect {
    let mut buffer = Buffer::scratch();
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut make_overlay = |k: OverlayKind| match k {
        // `visible_names`/`visible_effective_bindings` (NOT the raw `names`/
        // `effective_bindings`) — matches `overlay::build`'s real production
        // wiring exactly (see `new_keybindings`'s own doc: the corpus is the
        // PLATFORM-FILTERED view, so a highlighted row's index maps back
        // through `visible_slug_of`, not a raw catalog index).
        OverlayKind::Keybindings => Some(OverlayState::new_keybindings(
            crate::commands::visible_names(),
            crate::commands::visible_effective_bindings(&[], &[]),
        )),
        _ => None,
    };
    let mut browse_to = |_k: OverlayKind, _r: Option<String>| None;
    let mut ctx = ActionCtx {
        buffer: &mut buffer,
        shift_selecting: &mut shift,
        zoom: &mut zoom,
        search: &mut search,
        scroll_page_lines: 1,
        overlay,
        make_overlay: &mut make_overlay,
        browse_to: &mut browse_to,
        oracle: None,
    };
    apply_core(&mut ctx, action, false)
}

/// A fresh SETTINGS overlay (table-order corpus + value cells), for the
/// interaction tests below. Selection lands on row 0 ("Caret style", a Picker).
pub(super) fn settings_overlay() -> OverlayState {
    let mut ov = OverlayState::new(
        OverlayKind::Settings,
        crate::settings::names(),
        vec![],
        vec![],
    );
    ov.bindings = crate::settings::value_cells(&Default::default());
    ov
}

/// A make_overlay for the settings interaction tests: re-summons Settings (the
/// breadcrumb target) and builds the Caret sub-picker; everything else is None.
pub(super) fn settings_drive(overlay: &mut Option<OverlayState>, action: &Action) -> Effect {
    let mut buffer = Buffer::scratch();
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut make_overlay = |k: OverlayKind| match k {
        OverlayKind::Settings => Some(settings_overlay()),
        OverlayKind::Caret => Some(OverlayState::new_caret(crate::caret::mode())),
        OverlayKind::CjkLang => Some(OverlayState::new_cjk_lang(
            crate::frontmatter::cjk_priority().first().copied().unwrap_or(crate::frontmatter::Lang::Ja),
        )),
        _ => None,
    };
    // A Path row routes to the Project folder navigator; hand back a small one.
    let mut browse_to = |k: OverlayKind, _r: Option<String>| match k {
        OverlayKind::Project => Some(OverlayState::new_project(
            "/work".to_string(),
            vec![("sub".to_string(), false)],
            &[],
        )),
        _ => None,
    };
    let mut ctx = ActionCtx {
        buffer: &mut buffer,
        shift_selecting: &mut shift,
        zoom: &mut zoom,
        search: &mut search,
        scroll_page_lines: 1,
        overlay,
        make_overlay: &mut make_overlay,
        browse_to: &mut browse_to,
        oracle: None,
    };
    apply_core(&mut ctx, action, false)
}

/// Drive one action through the REAL `apply_core` seam over a fresh markdown
/// buffer (a no-path scratch buffer is markdown), seeding the cursor + optional
/// mark first, and return the buffer for assertions. Mirrors `align_table`'s
/// harness — the same seam a key / palette / `--keys` invocation rides.
pub(super) fn drive_format(src: &str, anchor: Option<usize>, cursor: usize, action: &Action) -> Buffer {
    let mut buffer = Buffer::from_str(src);
    if let Some(a) = anchor {
        buffer.set_cursor(a);
        buffer.set_mark();
    }
    buffer.set_cursor(cursor);
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut overlay = None;
    let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
    let mut browse_to =
        |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
    {
        let mut ctx = ActionCtx {
            buffer: &mut buffer,
            shift_selecting: &mut shift,
            zoom: &mut zoom,
            search: &mut search,
            scroll_page_lines: 1,
            overlay: &mut overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
            oracle: None,
        };
        apply_core(&mut ctx, action, false);
    }
    buffer
}

/// Drive one action with a CUSTOM `browse_to` (the project explorer tests use
/// a real temp-dir tree so absolute-path ascend/descend exercise the FS).
pub(super) fn drive_bt(
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
        oracle: None,
    };
    // Mirror the old `overlay_accept` out-param into `accept` (accumulating).
    if let Effect::OverlayAccept(kind, val) = apply_core(&mut ctx, action, false) {
        *accept = Some((kind, val));
    }
}

/// Build a `ws/` tree for the project explorer tests — `ws/child-a/sub/`,
/// `ws/child-b/` — in an InMemoryFs installed via the FILESYSTEM SEAM, so the
/// explorer's `list_dir_level` runs against a fake (no temp dir). Returns the
/// workspace root AND an `FsGuard` the caller binds (`let (ws, _fs) = …`) to keep
/// the fake installed (and the shared lock held) for the test's duration.
pub(super) fn proj_tree() -> (std::path::PathBuf, crate::fs::FsGuard) {
    // A deep-enough root so an ascend test can walk to ws's parent AND its
    // grandparent (`/home/dev/ws` → `/home/dev` → `/home`).
    let ws = std::path::PathBuf::from("/home/dev/ws");
    let mem = crate::fs::InMemoryFs::new()
        .with_dir(ws.join("child-a/sub"))
        .with_dir(ws.join("child-b"));
    let guard = crate::fs::FsGuard::install(std::sync::Arc::new(mem));
    (ws, guard)
}

/// A `browse_to` that drives the PROJECT explorer over an absolute temp tree,
/// exactly like the windowed app's hook (folders-only + synthetic "." row). No
/// recent-projects MRU (the Recent lens is exercised separately in `overlay/`).
pub(super) fn project_browse(ws: &std::path::Path, rel: Option<String>) -> Option<OverlayState> {
    let dir = rel.unwrap_or_else(|| ws.to_string_lossy().to_string());
    let folders: Vec<(String, bool)> =
        crate::index::list_dir_level(std::path::Path::new(&dir), None)
            .into_iter()
            .filter(|e| e.is_dir)
            .map(|e| (e.name, e.is_git))
            .collect();
    Some(OverlayState::new_project(dir, folders, &[]))
}

pub(super) fn theme_overlay() -> Option<OverlayState> {
    let names: Vec<String> = crate::theme::THEMES
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    Some(OverlayState::new_theme(names, crate::theme::active_index()))
}

/// Drive one `Newline` through the REAL `apply_core` seam on `buffer` (with the
/// caret already placed), so a test exercises the smart-Enter wiring end-to-end
/// exactly as `--keys "RET"` would.
pub(super) fn drive_newline(buffer: &mut Buffer) {
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut overlay = None;
    let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
    let mut browse_to =
        |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
    let mut ctx = ActionCtx {
        buffer,
        shift_selecting: &mut shift,
        zoom: &mut zoom,
        search: &mut search,
        scroll_page_lines: 1,
        overlay: &mut overlay,
        make_overlay: &mut make_overlay,
        browse_to: &mut browse_to,
        oracle: None,
    };
    apply_core(&mut ctx, &Action::Newline, false);
}

/// A markdown buffer (`.md` path) holding `text` with the caret at char `cursor`.
pub(super) fn md(text: &str, cursor: usize) -> Buffer {
    let mut b = Buffer::from_str(text);
    b.set_path(std::path::PathBuf::from("note.md"));
    b.set_cursor(cursor);
    b
}

/// Drive one action through the REAL `apply_core` seam on `buffer` (no overlay /
/// search), exactly as `--keys` would — for the Tab / Shift-Tab list-edit tests.
pub(super) fn drive_act(buffer: &mut Buffer, action: &Action) {
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut overlay = None;
    let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
    let mut browse_to =
        |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
    let mut ctx = ActionCtx {
        buffer,
        shift_selecting: &mut shift,
        zoom: &mut zoom,
        search: &mut search,
        scroll_page_lines: 1,
        overlay: &mut overlay,
        make_overlay: &mut make_overlay,
        browse_to: &mut browse_to,
        oracle: None,
    };
    apply_core(&mut ctx, action, false);
}

/// Drive one action through `apply_core` against a real buffer + a (possibly
/// live) search panel, so a test can step the find/replace surface.
pub(super) fn drive_search(buffer: &mut Buffer, search: &mut Option<SearchState>, action: &Action) {
    let mut shift = false;
    let mut zoom = 1.0;
    let mut overlay = None;
    let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
    let mut browse_to =
        |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
    let mut ctx = ActionCtx {
        buffer,
        shift_selecting: &mut shift,
        zoom: &mut zoom,
        search,
        scroll_page_lines: 1,
        overlay: &mut overlay,
        make_overlay: &mut make_overlay,
        browse_to: &mut browse_to,
        oracle: None,
    };
    apply_core(&mut ctx, action, false);
}

/// Drive one action through `apply_core` against a fresh buffer seeded with
/// `text` and the cursor at char `cursor`, returning the resulting `(Effect,
/// cursor_char)` — the cursor is exposed too so a caller can pin the "bump
/// fires only when the motion did NOT move the cursor" rule alongside the
/// effect. No oracle (logical-line fallback), so vertical motion uses the
/// buffer lines.
pub(super) fn drive_effect_and_cursor(text: &str, cursor: usize, action: &Action) -> (Effect, usize) {
    let mut buffer = Buffer::from_str(text);
    buffer.set_cursor(cursor);
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
        oracle: None,
    };
    let effect = apply_core(&mut ctx, action, false);
    drop(ctx);
    (effect, buffer.cursor_char())
}

/// [`drive_effect_and_cursor`], discarding the resulting cursor — the common
/// case for the recoil-trigger tests that only care about the [`Effect`].
pub(super) fn drive_effect(text: &str, cursor: usize, action: &Action) -> Effect {
    drive_effect_and_cursor(text, cursor, action).0
}

/// Per-DELETE fixture for the flinch completeness sweep below. Returns `Some`
/// for every action that removes a character — a `(text, ok_cursor,
/// wall_cursor, wall_recoil)`: a cursor where the delete REMOVES a char (must
/// `DeleteSquash`) and one at the buffer edge where it removes NOTHING (must
/// recoil that way). Backward deletes eat leftward (blocked at the START,
/// bump `Right`); forward deletes eat rightward (blocked at the END, bump
/// `Left`). `KillLine` deletes too but GULPs (its own effect, no wall arm), so
/// it — and every NON-delete action — returns `None`. The match has NO
/// wildcard (mirroring `all_actions`'s `_assert_covers`), so a NEW `Action`
/// variant fails to compile until it is classified here: a future delete can't
/// silently skip the squash/recoil decision the way `DeleteWordForward` once did.
pub(super) fn delete_flinch_fixture(
    action: &Action,
) -> Option<(&'static str, usize, usize, crate::caret::RecoilDir)> {
    use crate::caret::RecoilDir::{Left, Right};
    match action {
        Action::DeleteBackward | Action::DeleteWordBackward | Action::DeleteToLineStart => {
            Some(("hi", 1, 0, Right))
        }
        Action::DeleteForward | Action::DeleteWordForward => Some(("hi", 0, 2, Left)),
        // Not a squash/recoil delete: KillLine gulps; the rest never delete.
        Action::ForwardChar
        | Action::BackwardChar
        | Action::NextLine
        | Action::PreviousLine
        | Action::LineStart
        | Action::LineEnd
        | Action::ForwardWord
        | Action::BackwardWord
        | Action::BufferStart
        | Action::BufferEnd
        | Action::InsertChar(_)
        | Action::Newline
        | Action::InsertTab
        | Action::Outdent
        | Action::KillLine
        | Action::Yank
        | Action::Undo
        | Action::Redo
        | Action::SetMark
        | Action::CopyRegion
        | Action::KillRegion
        | Action::SelectAll
        | Action::ZoomIn
        | Action::ZoomOut
        | Action::ZoomReset
        | Action::PageScrollDown
        | Action::PageScrollUp
        | Action::Save
        | Action::Quit
        | Action::SearchForward
        | Action::SearchBackward
        | Action::OpenReplace
        | Action::Cancel
        | Action::OpenThemeMenu
        | Action::OpenCommandPalette
        | Action::OpenOutline
        | Action::OpenSpellSuggest
        | Action::ToggleCaretMode
        | Action::OpenCaretMenu
        | Action::OpenDictionaryMenu
        | Action::ToggleSpellcheck
        | Action::TogglePageMode
        | Action::PageWider
        | Action::PageNarrower
        | Action::PageReset
        | Action::ToggleDebug
        | Action::ToggleOutline
        | Action::ToggleTypewriter
        | Action::ToggleMenuBar
        | Action::ToggleWritingNits
        | Action::ToggleHiddenFiles
        | Action::ShowStatsHud
        | Action::OpenGoto
        | Action::OpenProject
        | Action::OpenRecentProjects
        | Action::OpenBrowse
        | Action::LastBuffer
        | Action::NewNote
        | Action::MoveNote
        | Action::OpenRenameNote
        | Action::DuplicateNote
        | Action::OpenSettings
        | Action::OpenSettingsMenu
        | Action::OpenKeybindings
        | Action::OpenCredits
        | Action::OpenGuide
        | Action::OpenHistory
        | Action::OpenAssetClean
        | Action::KeepVersion
        | Action::FinishBuffer
        | Action::FollowLink
        | Action::InsertLink
        | Action::ReportProblem
        | Action::DownloadFile
        | Action::CheckForUpdates
        | Action::BeginPrefix
        | Action::About
        | Action::LifetimeStats
        | Action::ConvertLineEndings
        | Action::AlignTable
        | Action::ToggleBlockquote
        | Action::ToggleBulletList
        | Action::ToggleNumberedList
        | Action::ToggleTaskList
        | Action::ToggleHeading
        | Action::ToggleCodeBlock
        | Action::Bold
        | Action::Italic
        | Action::InlineCode
        | Action::Highlight
        | Action::Strikethrough
        | Action::Ignore => None,
    }
}

/// Like [`drive_act`] but returns the resulting [`Effect`] — the COPY PULSE
/// tests need to `set_mark`/`set_cursor` on the buffer BEFORE dispatch
/// (unlike [`drive_effect`], which only ever seeds a bare cursor position),
/// so they build the buffer themselves and drive it through the REAL
/// `apply_core` seam directly.
pub(super) fn drive_act_effect(buffer: &mut Buffer, action: &Action) -> Effect {
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut overlay = None;
    let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
    let mut browse_to =
        |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
    let mut ctx = ActionCtx {
        buffer,
        shift_selecting: &mut shift,
        zoom: &mut zoom,
        search: &mut search,
        scroll_page_lines: 1,
        overlay: &mut overlay,
        make_overlay: &mut make_overlay,
        browse_to: &mut browse_to,
        oracle: None,
    };
    apply_core(&mut ctx, action, false)
}

/// Per-MOTION boundary FIXTURE for the boundary-bump completeness sweep below:
/// text + cursor char index that puts THIS `Action::is_motion` variant already
/// at its wall (so it truly cannot move), plus the [`crate::caret::RecoilDir`]
/// the bump must fire in. Reuses the same two-line "ab\ncd" fixture as
/// `blocked_motions_arm_recoil_away_from_the_wall`. A NEW motion classified
/// `is_motion` without a decision here panics `boundary_motions_bump_only_when_blocked`
/// LOUDLY instead of silently shipping a silent no-op boundary.
pub(super) fn motion_boundary_fixture(action: &Action) -> (&'static str, usize, crate::caret::RecoilDir) {
    use crate::caret::RecoilDir::{Down, Left, Right, Up};
    const TXT: &str = "ab\ncd"; // chars: a b \n c d (end == char 5); "ab" / "cd"
    match action {
        Action::ForwardChar => (TXT, 5, Left),
        Action::BackwardChar => (TXT, 0, Right),
        Action::ForwardWord => (TXT, 5, Left),
        Action::BackwardWord => (TXT, 0, Right),
        Action::NextLine => (TXT, 5, Up),
        Action::PreviousLine => (TXT, 0, Down),
        Action::LineStart => (TXT, 0, Right),
        Action::LineEnd => (TXT, 2, Left),
        Action::BufferStart => (TXT, 0, Down),
        Action::BufferEnd => (TXT, 5, Up),
        _ => panic!(
            "{action:?} is classified Action::is_motion but has no boundary-bump \
             fixture decided in `motion_boundary_fixture` — pick its wall position \
             + recoil direction there"
        ),
    }
}

/// Drive one action through the REAL `apply_core` seam with an EXPLICIT
/// `shift` flag and a PERSISTENT `shift_selecting` slot, so a test can walk a
/// whole Shift+motion run (set → extend → collapse) across calls.
pub(super) fn drive_shift(
    buffer: &mut Buffer,
    shift_selecting: &mut bool,
    action: &Action,
    shift: bool,
) -> Effect {
    let mut zoom = 1.0;
    let mut search = None;
    let mut overlay = None;
    let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
    let mut browse_to =
        |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
    let mut ctx = ActionCtx {
        buffer,
        shift_selecting,
        zoom: &mut zoom,
        search: &mut search,
        scroll_page_lines: 1,
        overlay: &mut overlay,
        make_overlay: &mut make_overlay,
        browse_to: &mut browse_to,
        oracle: None,
    };
    apply_core(&mut ctx, action, shift)
}

/// EVERY `Action` variant, one representative each, for the completeness
/// sweep below. The inner `_assert_covers` match lists every variant with NO
/// wildcard arm, so ADDING a new Action variant fails to compile here until
/// the new variant is added to this list — the sweep can never silently miss
/// a new motion.
pub(super) fn all_actions() -> Vec<Action> {
    fn _assert_covers(a: &Action) {
        match a {
            Action::ForwardChar
            | Action::BackwardChar
            | Action::NextLine
            | Action::PreviousLine
            | Action::LineStart
            | Action::LineEnd
            | Action::ForwardWord
            | Action::BackwardWord
            | Action::BufferStart
            | Action::BufferEnd
            | Action::InsertChar(_)
            | Action::Newline
            | Action::InsertTab
            | Action::Outdent
            | Action::DeleteBackward
            | Action::DeleteWordBackward
            | Action::DeleteWordForward
            | Action::DeleteToLineStart
            | Action::DeleteForward
            | Action::KillLine
            | Action::Yank
            | Action::Undo
            | Action::Redo
            | Action::SetMark
            | Action::CopyRegion
            | Action::KillRegion
            | Action::SelectAll
            | Action::ZoomIn
            | Action::ZoomOut
            | Action::ZoomReset
            | Action::PageScrollDown
            | Action::PageScrollUp
            | Action::Save
            | Action::Quit
            | Action::SearchForward
            | Action::SearchBackward
            | Action::OpenReplace
            | Action::Cancel
            | Action::OpenThemeMenu
            | Action::OpenCommandPalette
            | Action::OpenOutline
            | Action::OpenSpellSuggest
            | Action::ToggleCaretMode
            | Action::OpenCaretMenu
            | Action::OpenDictionaryMenu
            | Action::ToggleSpellcheck
            | Action::TogglePageMode
            | Action::PageWider
            | Action::PageNarrower
            | Action::PageReset
            | Action::ToggleDebug
            | Action::ToggleOutline
            | Action::ToggleTypewriter
            | Action::ToggleMenuBar
            | Action::ToggleWritingNits
            | Action::ToggleHiddenFiles
            | Action::ShowStatsHud
            | Action::OpenGoto
            | Action::OpenProject
            | Action::OpenRecentProjects
            | Action::OpenBrowse
            | Action::LastBuffer
            | Action::NewNote
            | Action::MoveNote
            | Action::OpenRenameNote
            | Action::DuplicateNote
            | Action::OpenSettings
            | Action::OpenSettingsMenu
            | Action::OpenKeybindings
            | Action::OpenCredits
            | Action::OpenGuide
            | Action::OpenHistory
            | Action::OpenAssetClean
            | Action::KeepVersion
            | Action::FinishBuffer
            | Action::FollowLink
            | Action::InsertLink
            | Action::ReportProblem
            | Action::DownloadFile
            | Action::CheckForUpdates
            | Action::BeginPrefix
            | Action::About
            | Action::LifetimeStats
            | Action::ConvertLineEndings
            | Action::AlignTable
            | Action::ToggleBlockquote
            | Action::ToggleBulletList
            | Action::ToggleNumberedList
            | Action::ToggleTaskList
            | Action::ToggleHeading
            | Action::ToggleCodeBlock
            | Action::Bold
            | Action::Italic
            | Action::InlineCode
            | Action::Highlight
            | Action::Strikethrough
            | Action::Ignore => {}
        }
    }
    vec![
        Action::ForwardChar,
        Action::BackwardChar,
        Action::NextLine,
        Action::PreviousLine,
        Action::LineStart,
        Action::LineEnd,
        Action::ForwardWord,
        Action::BackwardWord,
        Action::BufferStart,
        Action::BufferEnd,
        Action::InsertChar('x'),
        Action::Newline,
        Action::InsertTab,
        Action::Outdent,
        Action::DeleteBackward,
        Action::DeleteWordBackward,
        Action::DeleteWordForward,
        Action::DeleteToLineStart,
        Action::DeleteForward,
        Action::KillLine,
        Action::Yank,
        Action::Undo,
        Action::Redo,
        Action::SetMark,
        Action::CopyRegion,
        Action::KillRegion,
        Action::SelectAll,
        Action::ZoomIn,
        Action::ZoomOut,
        Action::ZoomReset,
        Action::PageScrollDown,
        Action::PageScrollUp,
        Action::Save,
        Action::Quit,
        Action::SearchForward,
        Action::SearchBackward,
        Action::OpenReplace,
        Action::Cancel,
        Action::OpenThemeMenu,
        Action::OpenCommandPalette,
        Action::OpenOutline,
        Action::OpenSpellSuggest,
        Action::ToggleCaretMode,
        Action::OpenCaretMenu,
        Action::OpenDictionaryMenu,
        Action::ToggleSpellcheck,
        Action::TogglePageMode,
        Action::PageWider,
        Action::PageNarrower,
        Action::PageReset,
        Action::ToggleDebug,
        Action::ToggleOutline,
        Action::ToggleTypewriter,
        Action::ToggleMenuBar,
        Action::ToggleWritingNits,
        Action::ToggleHiddenFiles,
        Action::ShowStatsHud,
        Action::OpenGoto,
        Action::OpenProject,
        Action::OpenRecentProjects,
        Action::OpenBrowse,
        Action::LastBuffer,
        Action::NewNote,
        Action::MoveNote,
        Action::OpenRenameNote,
        Action::DuplicateNote,
        Action::OpenSettings,
        Action::OpenSettingsMenu,
        Action::OpenKeybindings,
        Action::OpenCredits,
        Action::OpenGuide,
        Action::OpenHistory,
        Action::OpenAssetClean,
        Action::KeepVersion,
        Action::FinishBuffer,
        Action::FollowLink,
        Action::InsertLink,
        Action::ReportProblem,
        Action::DownloadFile,
        Action::CheckForUpdates,
        Action::BeginPrefix,
        Action::About,
        Action::LifetimeStats,
        Action::ConvertLineEndings,
        Action::AlignTable,
        Action::ToggleBlockquote,
        Action::ToggleBulletList,
        Action::ToggleNumberedList,
        Action::ToggleTaskList,
        Action::ToggleHeading,
        Action::ToggleCodeBlock,
        Action::Bold,
        Action::Italic,
        Action::InlineCode,
        Action::Highlight,
        Action::Strikethrough,
        Action::Ignore,
    ]
}

/// How the every-command smoke sweep classifies a catalog Action's expected
/// outcome — used to pick the coherence assertion after a dispatch. Exhaustive
/// over `Action` (a no-wildcard match in [`smoke_command_kind`]), so a future
/// variant is a compile error until it lands under the sweep.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum SmokeKind {
    /// Summons a modal overlay — `ctx.overlay` must be `Some` after dispatch
    /// (the smoke `BuildCtx`/`browse_to` feed each picker what it needs to open).
    Opener,
    /// Signals a caller-level deferred [`Effect`] the pure core can't perform
    /// itself (quit / buffer-swap / notes-swap / pinned snapshot / daemon finish
    /// / browser handoff) — the returned effect is asserted exactly.
    Deferred,
    /// Any other real catalog command: edits, in-place toggles (globals),
    /// zoom, search-open, or the inert `Ignore` sentinel (Writing nits). Only
    /// checked to not panic + leave a valid buffer (plus the About-card global).
    InPlace,
    /// NOT a catalog command (a char/line arrow motion, self-insert, prefix, or
    /// a keymap-only action like `ShowStatsHud`/`OpenCommandPalette`). Never
    /// appears in `COMMANDS`; present ONLY to keep the match exhaustive so a new
    /// `Action` variant forces a decision here. (The curated NAVIGATION motions
    /// are catalog rows since 2026-07-10 — they classify `InPlace` above.)
    NotCatalog,
}

/// Classify an `Action`'s expected smoke outcome. NO-WILDCARD over the whole
/// `Action` enum: every variant is named, so adding one to `keymap::Action`
/// (which a new catalog command needs) fails to compile until it is placed —
/// that is the compile-time coverage guarantee (`rowlayout`/menu-roster idiom).
pub(super) fn smoke_command_kind(a: &Action) -> SmokeKind {
    match a {
        // Overlay summons that open given the smoke BuildCtx / browse_to.
        Action::OpenGoto
        | Action::OpenProject
        | Action::OpenRecentProjects
        | Action::OpenBrowse
        | Action::OpenOutline
        | Action::OpenSpellSuggest
        | Action::OpenHistory
        | Action::MoveNote
        | Action::OpenThemeMenu
        | Action::OpenCaretMenu
        | Action::OpenDictionaryMenu
        | Action::OpenSettingsMenu
        | Action::OpenKeybindings
        | Action::OpenAssetClean
        // LINKS V2: the smoke fixture is a markdown buffer with the caret inside
        // an existing link (see the FollowLink note below), so Cmd-K always opens
        // the minibuffer here — an Opener, like every other summon.
        | Action::InsertLink => SmokeKind::Opener,

        // Deferred effects (the pure core signals; the live App performs).
        Action::Quit
        | Action::LastBuffer
        | Action::NewNote
        | Action::OpenCredits
        | Action::OpenGuide
        | Action::KeepVersion
        | Action::FinishBuffer
        | Action::FollowLink
        | Action::ReportProblem
        | Action::DownloadFile
        | Action::CheckForUpdates
        | Action::DuplicateNote => SmokeKind::Deferred,

        // Real catalog commands that mutate locally (buffer / globals / zoom /
        // search) — asserted only to not panic. (`Ignore` is no longer a catalog
        // command — the Writing-nits sentinel is retired for a real
        // `ToggleWritingNits`; `Ignore` now sits in the NotCatalog group below.)
        Action::Save
        | Action::SearchForward
        | Action::SearchBackward
        | Action::OpenReplace
        | Action::Undo
        | Action::Redo
        | Action::CopyRegion
        | Action::KillRegion
        | Action::Yank
        | Action::SelectAll
        | Action::ZoomIn
        | Action::ZoomOut
        | Action::ZoomReset
        // The curated NAVIGATION motions joined the catalog (2026-07-10, to
        // become palette-visible + rebindable — see `commands.rs`'s module doc;
        // WIDENED by the emacs-hands-on-Linux round to the last four bare-control
        // motions below); a motion dispatched from the palette just moves the
        // caret in place.
        | Action::ForwardWord
        | Action::BackwardWord
        | Action::LineStart
        | Action::LineEnd
        | Action::BufferStart
        | Action::BufferEnd
        | Action::ForwardChar
        | Action::BackwardChar
        | Action::NextLine
        | Action::PreviousLine
        | Action::ToggleCaretMode
        | Action::ToggleSpellcheck
        | Action::ToggleHiddenFiles
        | Action::TogglePageMode
        | Action::PageWider
        | Action::PageNarrower
        | Action::PageReset
        | Action::ToggleDebug
        | Action::ToggleOutline
        | Action::ToggleTypewriter
        | Action::ToggleMenuBar
        | Action::ToggleWritingNits
        | Action::About
        | Action::LifetimeStats
        | Action::ConvertLineEndings
        | Action::AlignTable
        | Action::ToggleBlockquote
        | Action::ToggleBulletList
        | Action::ToggleNumberedList
        | Action::ToggleTaskList
        | Action::ToggleHeading
        | Action::ToggleCodeBlock
        | Action::Bold
        | Action::Italic
        | Action::InlineCode
        | Action::Highlight
        | Action::Strikethrough
        // The smoke fixture (`rich_markdown_buffer`) is a NO-PATH buffer, so
        // `Action::OpenRenameNote`'s pure-buffer-state gate declines to open —
        // an in-place no-op under this harness, not an Opener.
        | Action::OpenRenameNote => SmokeKind::InPlace,

        // Not catalog commands — self-insert, editing primitives, prefix, and
        // keymap-only actions (the plain, unmodified ARROW keys still dispatch
        // these four motions directly and stay uncataloged themselves; only
        // the actions moved into the catalog, above). Present for
        // exhaustiveness only.
        Action::InsertChar(_)
        | Action::Newline
        | Action::InsertTab
        | Action::Outdent
        | Action::DeleteBackward
        | Action::DeleteWordBackward
        | Action::DeleteWordForward
        | Action::DeleteToLineStart
        | Action::DeleteForward
        | Action::KillLine
        | Action::SetMark
        | Action::PageScrollDown
        | Action::PageScrollUp
        | Action::Cancel
        | Action::OpenCommandPalette
        | Action::ShowStatsHud
        | Action::OpenSettings
        | Action::BeginPrefix
        | Action::Ignore => SmokeKind::NotCatalog,
    }
}

/// A rich markdown document for the smoke sweep: a heading, a paragraph with a
/// bold run and a link, a GFM table, and a fenced code block. The caret is
/// placed with a small selection INSIDE the `[link](…)`, so formatting/edit
/// toggles operate on real content and `Action::FollowLink` actually resolves a
/// URL (rather than a bare no-op). A no-path buffer, so `is_markdown()` is true
/// and the 11 formatting toggles + Align Table + Convert Line Endings are live.
pub(super) fn rich_markdown_buffer() -> Buffer {
    // Raw string (no `"#` inside) keeps the inner `"` / backticks literal.
    let mut buffer = Buffer::from_str(
        r#"# Heading One

A paragraph with **bold text** and a [link](https://example.com) inline.

| Col A | Col B |
| ----- | ----- |
| a1    | b1    |

```rust
fn main() {
let n = 42;
println!("{n}");
}
```
"#,
    );
    assert!(buffer.is_markdown(), "the smoke fixture must be a markdown buffer");
    // Caret + small selection inside `[link]` (ASCII text ⇒ byte == char index).
    let text = buffer.text();
    let pos = text.find("link").expect("the fixture contains a link");
    buffer.select_range(pos, pos + 3);
    assert!(buffer.has_selection(), "the smoke fixture must seed a small selection");
    buffer
}
