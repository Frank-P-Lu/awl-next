//! Mode execution: the back half of `main.rs`.
//!
//! [`run`] takes the [`Mode`](crate::args::Mode) that argument parsing resolved
//! and DOES it — renders a headless capture, runs the typing benchmark, or hands
//! off to the windowed editor. The headless captures share one seam:
//! [`replay_keys`] applies a parsed `--keys` action stream against a [`Buffer`]
//! THROUGH `actions::apply_core`, so a replay is byte-for-byte identical to live
//! editing. The small `resolve_*` / `load_buffer` helpers build the project +
//! buffer context every mode needs.

use std::path::PathBuf;

use anyhow::Result;

use crate::args::Mode;
use crate::buffer::Buffer;
use crate::capture::{self, CaptureOpts};
use crate::config::Config;
use crate::keymap::Action;
use crate::{actions, app, bench};

/// Build the editor buffer for a (possibly absent) file. A missing/unreadable
/// file yields an empty buffer bound to that path; no file yields a scratch
/// buffer.
fn load_buffer(file: &Option<PathBuf>) -> Buffer {
    match file {
        Some(p) => Buffer::from_file(p),
        None => Buffer::scratch(),
    }
}

/// Resolve the ACTIVE project root: explicit `--root` wins outright; otherwise,
/// on a BARE launch (no file argument at all) the remembered STICKY PROJECT
/// ROOT (`config.project_root`, written by every switch-project / Cmd-Shift-P commit)
/// restores the last-worked-in project — mirroring the scratch-buffer stash's
/// exact restore condition, so `awl` with no arguments reopens where you left
/// off. A launch WITH a file argument is unaffected (still resolves from that
/// file's own directory below), so opening some other file never silently
/// redirects the project scope. Absent `sticky_root` (no config, or a config
/// with no remembered project) reproduces today's behaviour exactly: the
/// launch file's directory (if it's a dir), else its parent, else cwd.
fn resolve_root(
    root: &Option<PathBuf>,
    file: &Option<PathBuf>,
    sticky_root: Option<&std::path::Path>,
) -> PathBuf {
    if let Some(r) = root {
        return r.clone();
    }
    if file.is_none() {
        if let Some(p) = sticky_root {
            return p.to_path_buf();
        }
    }
    if let Some(f) = file {
        if crate::fs::active().is_dir(f) {
            return f.clone();
        }
        if let Some(p) = f.parent() {
            if !p.as_os_str().is_empty() {
                return p.to_path_buf();
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Resolve the EFFECTIVE workspace whose child dirs are the switch-project
/// (Cmd-Shift-P) candidates: an explicit `--workspace` wins; otherwise DEFAULT to the
/// PARENT of the active project `root`, so switch-project lists the root's
/// SIBLING projects out of the box — launched inside `~/work/repos/some-repo`,
/// the workspace defaults to `~/work/repos`, so Cmd-Shift-P shows all the repos. A
/// root with no usable parent (e.g. the filesystem root) falls back to the root
/// itself, so the picker still opens rather than silently doing nothing.
pub(crate) fn resolve_workspace(workspace: &Option<PathBuf>, root: &std::path::Path) -> PathBuf {
    if let Some(w) = workspace {
        return w.clone();
    }
    match root.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => root.to_path_buf(),
    }
}

/// What a `--keys` replay produced beyond the buffer (App-level state living off
/// the `Buffer`), folded into the capture options by the caller.
struct ReplayResult {
    zoom: Option<f32>,
    selection: Option<((usize, usize), (usize, usize))>,
    search_query: Option<String>,
    search_case: bool,
    /// Whether the replay left the search panel in REPLACE mode (Cmd-Option-F).
    replace_active: bool,
    /// The replacement field text (empty headlessly — the isearch-input gap).
    replacement: String,
    /// The overlay left open at the end of the replay (if any), for the sidecar.
    overlay: Option<crate::overlay::OverlayState>,
    /// If the replay ACCEPTED a go-to item (Enter), the chosen value so the
    /// caller can load that file before capturing.
    accept: Option<(crate::overlay::OverlayKind, String)>,
    /// How many buffers are open at the end of the replay (the active `buffer`
    /// + everything the MULTI-BUFFER REGISTRY still has backgrounded) — feeds
    /// the sidecar `buffers.open` count. Stays `1` for any replay that never
    /// drives a Goto accept, so a plain `--screenshot` (no `--keys`, or keys
    /// that never open a second file) is unaffected.
    buffers_open: usize,
}

/// PARK `buffer` into `registry` under its stable identity (a no-op for an
/// unnamed, still-empty quick note — see [`crate::buffers::BufferKey::of`]),
/// leaving `buffer` a scratch placeholder for the caller to immediately
/// overwrite. The headless replay's mirror of `App::park_active_buffer` — same
/// behavior, same code, so EVERY "the active buffer is about to be replaced"
/// site in [`replay_keys`] (a Goto switch, `Cmd-N`) backgrounds it identically
/// rather than one path silently discarding it (the `Effect::NewNote` gap a
/// code review caught: it used to reset `buffer` in place with no park at
/// all, permanently losing whatever the buffer being left held).
fn park_active(buffer: &mut Buffer, registry: &mut crate::buffers::BufferRegistry<()>) {
    if let Some(key) = crate::buffers::BufferKey::of(buffer) {
        let old = std::mem::replace(buffer, Buffer::scratch());
        registry.park(key, crate::buffers::Entry { buffer: old, extra: () });
    }
}

/// Replay a parsed `--keys` action stream against `buffer` THROUGH the shared
/// `actions::apply_core` seam, so headless replay is byte-for-byte identical to
/// live editing. `corpus` is the active project's file index (Goto), `root`
/// scopes the Browse navigator, and `workspace` supplies the switch-project
/// children — so a replayed Cmd-O / Cmd-Shift-P / Browse summons a real overlay
/// the rest of the key-spec can filter / move / descend / accept. Returns the
/// post-replay App-level state.
fn replay_keys(
    buffer: &mut Buffer,
    keys: &[Action],
    corpus: &[String],
    root: &std::path::Path,
    workspace: Option<&std::path::Path>,
    notes_root: &std::path::Path,
    config: &Config,
    // The visual-line motion LAYOUT ORACLE (an offscreen-shaped pipeline), so the
    // headless replay sees the SAME wrap geometry the live window does. `None` in
    // the unit tests / GPU-less paths, where motion falls back to LOGICAL lines.
    oracle: Option<&dyn actions::LayoutOracle>,
) -> ReplayResult {
    let mut shift_selecting = false;
    let mut zoom = 1.0f32;
    let mut search: Option<crate::search::SearchState> = None;
    let mut overlay: Option<crate::overlay::OverlayState> = None;
    let mut accept: Option<(crate::overlay::OverlayKind, String)> = None;
    // MULTI-BUFFER REGISTRY: the same `crate::buffers::BufferRegistry` the live
    // App uses, so a `--keys` spec that Goes-to file A, edits, Goes-to file B,
    // edits, then Goes back to A sees A's PRESERVED cursor/edits/undo — the
    // v1 multi-buffer win, headlessly drivable. Carries no extra payload
    // (`()`): headless replay tracks nothing per-buffer beyond the `Buffer`
    // itself (no scroll/spell/autosave state to preserve here).
    let mut registry: crate::buffers::BufferRegistry<()> = crate::buffers::BufferRegistry::default();
    // The spell engine for the Cmd-`;` picker, loaded once (None if the dictionary
    // failed to parse — the summon then no-ops, like the live path with no checker).
    let spell = crate::spell::SpellChecker::new(crate::spell::active_variant()).ok();
    for key in keys {
        // A tiny worklist so the COMMAND PALETTE's run-on-Enter chains: Enter on a
        // command writes `run_action`, which we then feed back through the core
        // (slot now empty) so an overlay-opening command opens its sub-overlay as
        // the final captured state. At most one chained action, so this drains in
        // one extra pass.
        let mut current: Option<Action> = Some(key.clone());
        // BREADCRUMB: when the palette's Enter re-dispatches a `RunAction` (below),
        // stamp `return_to = Command` onto whatever overlay that command opens, so a
        // later POP (Esc / value-pick) re-summons the palette — mirroring the live
        // `App::apply` seam. Set by the `RunAction` arm, consumed right after the
        // re-dispatched action's `apply_core` opens its overlay.
        let mut pending_return_to: Option<crate::overlay::OverlayKind> = None;
        while let Some(action) = current.take() {
        // GO-TO's HEADINGS lens corpus: the current buffer's markdown headings (title
        // indented by depth, paired with its line) — the fold that retired the
        // standalone Outline picker. Gathered ONLY when a Go-to door fires (Cmd-O /
        // "Go to heading…"), matching the live app; a non-markdown buffer / no headings
        // yields an empty list (the Headings lens then reads empty).
        let goto_headings: Vec<(String, usize)> =
            if matches!(action, Action::OpenGoto | Action::OpenOutline) && buffer.is_markdown() {
                crate::markdown::headings(&buffer.text())
                    .into_iter()
                    .map(|h| (h.label(), h.line))
                    .collect()
            } else {
                Vec::new()
            };
        // SPELL picker target: the misspelled word the cursor is on (or adjacent to)
        // + its corrections, resolved before the builder and ONLY when the spell
        // binding fired. None when the cursor isn't on a flagged word (no-op summon).
        let spell_target: Option<(Vec<String>, (usize, usize, usize))> =
            if matches!(action, Action::OpenSpellSuggest) {
                spell.as_ref().and_then(|sc| {
                    let (line, col) = buffer.cursor_line_col();
                    sc.suggest_at(&buffer.text(), line, col, buffer.syntax_lang()).map(|t| {
                        (
                            t.suggestions,
                            (t.misspelling.line, t.misspelling.start_col, t.misspelling.end_col),
                        )
                    })
                })
            } else {
                None
            };
        // HISTORY TIMELINE rows for the current file (newest-first), each answering
        // WHEN + WHICH with a "+N −M" changed-count vs the current buffer. Read from
        // the store ONLY when the History binding fired (so a `--keys "Cmd-S-h"`
        // capture shows the real versions of the seeded file); the history key comes
        // from the ONE shared derivation (`history::source_path`: buffer path, else
        // the scratch stash — the replay has no App-level `file`), matching the live
        // gather. `now` stamps the relative labels. History is an explicitly-summoned
        // overlay, so this never runs in a default capture.
        let history_entries: Vec<crate::history::TimelineRow> =
            if matches!(action, Action::OpenHistory) {
                match crate::history::source_path(buffer.path(), None, buffer.is_note()) {
                    Some(path) => crate::history::timeline_rows(
                        &path,
                        &buffer.text(),
                        crate::history::now_millis(),
                    ),
                    None => Vec::new(),
                }
            } else {
                Vec::new()
            };
        // ASSET CLEANER orphan list: scanned from the replay's active root + corpus
        // ONLY when the "Clean unused assets" binding fired, so a `--keys` capture sees
        // the real orphan list via the sidecar. Reads through the FileSystem seam (the
        // capture's fixture / real `--root`), mirroring the live gather. The TRASH
        // itself is a documented no-op in the effect match below (the determinism gate).
        let assets: Vec<crate::assets::Orphan> = if matches!(action, Action::OpenAssetClean) {
            crate::assets::scan(root, corpus)
        } else {
            Vec::new()
        };
        // The non-navigable builder inputs. Headless leaves the GO-TO recency tiers +
        // labels EMPTY (no mtime read, no open/recent history) so the capture stays
        // byte-stable; the buffer-scoped headings / spell / history come from the
        // replayed state + the store.
        let effective_keep = config.effective_linux_keep();
        let build_ctx = crate::overlay::BuildCtx {
            goto_corpus: corpus.to_vec(),
            goto_open: Vec::new(),
            goto_recent: Vec::new(),
            goto_times: Vec::new(),
            config_keys: &config.keys,
            config_linux_keep: &effective_keep,
            goto_headings,
            spell_target,
            history_entries,
            // Headless capture has no wall clock: History's Session / Today lenses
            // stay inert (the determinism gate), so a `--keys` History capture groups
            // nothing regardless of what the store's stamps say.
            history_now: None,
            history_session_start: None,
            // SETTINGS MENU value cells: gathered from the replay's config + active
            // root + zoom, so a `--keys "Settings"` capture reports each setting's
            // real value (deterministic — config is loaded from --config or defaults).
            settings_values: crate::settings::SettingsValues::gather(config, root, zoom),
            assets,
        };
        let mut make_overlay =
            |kind: crate::overlay::OverlayKind| crate::overlay::build(kind, &build_ctx);
        let mut browse_to = |kind: crate::overlay::OverlayKind, rel: Option<String>| {
            // Shared one-level builder: Project navigates the workspace by absolute
            // path, MoveDest walks the NOTES root (folders only), Browse the active
            // root (files + folders).
            // The recent-PROJECTS MRU is live-only persisted state; the headless
            // replay passes an empty list (the determinism gate), so the Project
            // navigator's Recent lens is inert in a capture — byte-stable.
            crate::overlay::browse_level(kind, rel, root, notes_root, workspace, &[])
        };
        let mut ctx = actions::ActionCtx {
            buffer,
            shift_selecting: &mut shift_selecting,
            zoom: &mut zoom,
            search: &mut search,
            // Headless has no viewport to measure; a page is a fixed,
            // deterministic chunk of logical lines.
            scroll_page_lines: 20,
            overlay: &mut overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
            oracle,
        };
        // Replay is unshifted: selection comes from an explicit C-Space mark,
        // matching the emacs-style sticky region the key-spec expresses.
        let effect = actions::apply_core(&mut ctx, &action, false);
        drop(ctx);
        // BREADCRUMB: stamp the overlay this action just opened (if any) with the
        // palette parent a preceding `RunAction` re-dispatch set — a no-op unless the
        // previous iteration was a palette Enter (`pending_return_to` still None here
        // for a direct summon).
        crate::actions::stamp_return_to(&mut overlay, pending_return_to.take());
        // Carry out the ONE deferred effect the core signalled (mutually exclusive,
        // so a single match suffices). Quit / LastBuffer are no-ops in capture (no
        // event loop, no 2-deep history); the rest mirror the live App's handling.
        match effect {
            // New note (Cmd-N): PARK the buffer being left (exactly like the live
            // `App::new_note` — a code-review-caught gap used to skip this and
            // just reset `buffer` in place, silently discarding it) then reset
            // to a fresh quick note bound to the notes root, so subsequent
            // typed chars build the title and an explicit Cmd-S derives
            // the filename + writes it. The root-switch is App-only; headless
            // only needs the buffer to become a note so the Save flow is
            // verifiable.
            actions::Effect::NewNote => {
                park_active(buffer, &mut registry);
                buffer.start_note(notes_root.to_path_buf());
            }
            // Settings: load the config file into the buffer (creating the commented
            // default first if missing), so the capture reflects the config CONTENTS
            // — exactly what the live Settings command does. Opens the EFFECTIVE
            // config path (the `--config` target when one was given).
            actions::Effect::OpenSettings => {
                if !config.path.as_os_str().is_empty() {
                    if !crate::fs::active().exists(&config.path) {
                        let _ = Config::write_default(&config.path);
                    }
                    *buffer = Buffer::from_file(&config.path);
                }
            }
            // Credits: load the embedded CREDITS.md text directly into the buffer
            // — no filesystem write at all (the headless capture path stays
            // side-effect-light, mirroring OpenSettings' spirit without needing a
            // disk round trip, since the text is compiled in rather than
            // user-owned). No park needed here either: `replay_keys` never stashes
            // scratch (structurally autosave-free), so there is nothing to protect.
            actions::Effect::OpenCredits => {
                *buffer = Buffer::from_str(crate::credits::CREDITS_MD);
            }
            // Guide: load the embedded GUIDE.md text directly into the buffer —
            // mirrors OpenCredits exactly, same side-effect-light reasoning.
            actions::Effect::OpenGuide => {
                *buffer = Buffer::from_str(crate::guide::GUIDE_MD);
            }
            // An overlay accepted (Goto file / Project / MoveDest / Theme): remember
            // the chosen value for the caller to load before capturing. Persists
            // across keys like the old out-param (later accepts overwrite).
            //
            // A Goto accept ALSO drives the real MULTI-BUFFER switch right here,
            // inline in the replay loop (not deferred to the caller, which only
            // ever sees the FINAL accepted value): opening a path already
            // resident in `registry` (a previous Goto in this same `--keys` run)
            // restores its live buffer — cursor, edits, undo intact — instead of
            // re-reading disk, mirroring `App::load_path` exactly. This is what
            // makes an A -> B -> A round trip verifiable from one `--keys` spec.
            actions::Effect::OverlayAccept(kind, val) => {
                if kind == crate::overlay::OverlayKind::Goto {
                    let path = crate::index::resolve(root, &val);
                    // Compared via the normalized registry identity, not raw
                    // path equality — mirrors `App::load_path`'s "already
                    // active" check (see `BufferKey::path`'s doc: a launch
                    // file argument that stayed relative and this ALWAYS
                    // root-joined Goto path must be recognized as the same
                    // file, or the switch below re-reads it fresh from disk
                    // and orphans the relative spelling's live edit).
                    let new_key = crate::buffers::BufferKey::path(&path);
                    if crate::buffers::BufferKey::of(buffer).as_ref() != Some(&new_key) {
                        park_active(buffer, &mut registry);
                        *buffer = match registry.take(&new_key) {
                            Some(entry) => entry.buffer,
                            None => Buffer::from_file(&path),
                        };
                        // STICKY PAGE WIDTH: re-apply the measure for the ARRIVING
                        // buffer's own kind, mirroring `App::load_path`'s post-switch
                        // resync (`App::sync_page_measure`) — a `--keys` Goto from a
                        // `.md` to a `.rs` fixture (or back) picks up that file's own
                        // configured/default measure, exactly like the live app.
                        // (This made every Goto-replay TEST a page-global writer;
                        // `set_measure` self-serializes under cfg(test) — see
                        // `page::test_lock()` — so those tests need no lock of
                        // their own and can never stomp a locked reader again.)
                        crate::page::set_measure(config.measure_for(buffer.page_class()));
                    }
                }
                accept = Some((kind, val));
            }
            // SAVE-FEEDBACK round: `Action::Save` on the true scratch surface —
            // the ONE headless-reachable half of the effect (see its own doc):
            // convert the buffer into a real note under the harness's own
            // `notes_root`, using the SAME `Buffer::save_as_note` the live App
            // calls. This actually writes through the active `fs` backend (the
            // fixture / real disk), so the sidecar's `cursor`/buffer state and a
            // later Goto both see the new file — no notice to reflect (live-only).
            actions::Effect::ConvertScratchAndSave => {
                let _ = buffer.save_as_note(notes_root);
            }
            // Go-to's HEADINGS lens accepted (the retired Outline picker): jump the
            // cursor to the accepted heading LINE so the capture's `cursor` block
            // reflects the jump (agent-verifiable), mirroring the live App.
            actions::Effect::JumpToLine(line) => {
                let idx = buffer.line_col_to_char(line, 0);
                buffer.set_cursor(idx);
            }
            // COMMAND PALETTE run-on-Enter: feed the chosen command back through the
            // core (the palette already closed), so e.g. "Go to file" opens the goto
            // overlay as the final captured state.
            actions::Effect::RunAction(a) => {
                pending_return_to = Some(crate::overlay::OverlayKind::Command);
                current = Some(a);
            }
            // REBIND MENU commit: the headless capture path does NOT mutate the user's
            // config file (a screenshot stays side-effect-light); it reflects the
            // completed capture in the menu's NOTICE so the sidecar shows what was
            // bound, then returns to the command list. The real write + live-reload is
            // the live App's job (`App::rebind_commit`), unit-tested via `Config`.
            actions::Effect::RebindCommit { slug, binding, .. } => {
                if let Some(ov) = overlay.as_mut() {
                    ov.notice = format!("bound {slug} -> {binding}");
                    ov.capture_abort();
                }
            }
            // REBIND MENU reset: likewise reflected in the NOTICE only (intercept
            // already set it); no file mutation in the capture path.
            actions::Effect::RebindReset { slug } => {
                if let Some(ov) = overlay.as_mut() {
                    if ov.notice.is_empty() {
                        ov.notice = format!("reset {slug}");
                    }
                }
            }
            // Quit / LastBuffer have nothing to do in the headless capture path.
            // Recoil and the edit flinches (TypeImpact / DeleteSquash / Gulp /
            // LineLand / CopyPulse) are LIVE-ONLY caret flourishes (a squash-pop /
            // velocity kick / selection-tint brighten that self-settles) — the
            // headless capture has no clock and renders the SETTLED caret + selection,
            // so they are no-ops here and the frame stays byte-identical (CopyPulse
            // never touches the buffer either way — the copy itself already ran).
            // FinishBuffer (Finish file): the core already ran the SAME `buffer.save()` a
            // headless `Action::Save` replay always has (writes through the active
            // `fs` backend); the daemon-notify + buffer-swap are live-App-only (no
            // daemon, no 2-deep buffer history, in a one-shot replay) — a no-op here,
            // exactly like `LastBuffer`.
            actions::Effect::LastBuffer
            | actions::Effect::Quit
            | actions::Effect::Recoil(_)
            | actions::Effect::TypeImpact
            | actions::Effect::DeleteSquash
            | actions::Effect::Gulp
            | actions::Effect::LineLand
            | actions::Effect::CopyPulse
            // SETTINGS MENU toggle: flipping a live global + writing config is the
            // live App's job (`App::setting_toggle`) — the capture path has neither a
            // global setter it should mutate nor a config file to write, so it reflects
            // nothing here (the menu stays open on its pre-toggle value cells). The
            // toggle round-trip is unit-tested at the apply seam instead.
            | actions::Effect::SettingToggle { .. }
            // SETTINGS MENU inline VALUE commit / PATH pick: parse-clamp-apply-persist
            // and folder-key writes are the live App's job (`App::setting_value_commit`
            // / `setting_path_pick`) — the capture path has no live global setter it
            // should mutate nor a config file to write, so both reflect nothing here
            // (the value-edit round-trip is unit-tested at the apply seam instead). The
            // pure inline-edit sub-state itself IS driven by the shared core, so the
            // still-open menu's cell reflects the typed value; only the commit is inert.
            | actions::Effect::SettingValueCommit { .. }
            | actions::Effect::SettingPathPick { .. }
            | actions::Effect::FinishBuffer
            // KEEP THIS VERSION: pinning a snapshot writes the local-history store,
            // a live-App-only concern (`App::keep_version`) — the history determinism
            // gate keeps every store write off the capture path, so this is a no-op
            // here (the pin/exemption logic is unit-tested in `history/` instead).
            | actions::Effect::KeepVersion
            // FollowLink (C-c C-o): opening the OS browser is a live-App-only
            // handoff (`App::follow_link`) — a capture must never spawn a browser,
            // so it is a no-op here (the URL extraction itself is unit-tested pure).
            | actions::Effect::FollowLink(_)
            // REPORT A PROBLEM: composing the mailto: URL (which needs the
            // crash-log directory) and opening it are both live-App-only
            // concerns (`App::report_problem`) — a capture must never spawn a
            // mail client, so this is a no-op here; the composition itself is
            // unit-tested pure (`crashlog::report_problem_mailto`).
            | actions::Effect::ReportProblem
            // TRASH ASSET: moving an orphan to the OS Trash is a live-App-only
            // concern (`App::trash_asset`) — a capture must never touch the real Trash,
            // so this is a documented no-op here. The picker's orphan list therefore
            // stays WHOLE in a `--keys` replay (the sidecar never claims a file was
            // trashed that wasn't); the trash + row-removal wiring is unit-tested at
            // the apply seam with a fake trash instead.
            | actions::Effect::TrashAsset { .. }
            // SAVE-FEEDBACK round: the write already happened inside the core
            // (`Buffer::save`, through the active `fs` backend); the notice is
            // live-only (`App::notice` has no sidecar field) and history
            // snapshotting-on-save is a live-App-only concern (see
            // `App::snapshot_after_save`'s call site) — so both fates are a
            // no-op here, same shape as `FinishBuffer`.
            | actions::Effect::SaveDone { .. }
            // NOTES VERBS round: both the actual disk RENAME (`App::rename_current_file`
            // — git-managed gate, no-clobber refusal, the one-owner path-keyed
            // bookkeeping) and the DUPLICATE copy+swap (`App::duplicate_current_file`)
            // are live-App-only, mirroring `MoveDest`'s own real-move precedent (its
            // ACCEPT is reflected below via `accept`, but the actual `fs::rename` is
            // live-only too) — a no-op here. The RENAME MINIBUFFER's typing/open/
            // cancel flow IS driven by the shared core (`overlay_intercept`'s
            // `rename_edit` block), so it stays fully `--keys`-drivable and sidecar-
            // reflected via `overlay.hint` (`OverlayState::foot_hint`) up to the
            // moment of commit; only the disk write itself is deferred here.
            | actions::Effect::RenameNoteCommit { .. }
            | actions::Effect::DuplicateNote
            | actions::Effect::None => {}
        }
        }
    }
    // The active `buffer` + whatever the registry still has backgrounded.
    let buffers_open = registry.len() + 1;
    let zoom_out = if zoom != 1.0 { Some(zoom) } else { None };
    let sel = buffer.selection_line_col();
    let search_query = search.as_ref().map(|s| s.query().to_string());
    let search_case = search.as_ref().map(|s| s.is_case_sensitive()).unwrap_or(false);
    let replace_active = search.as_ref().map(|s| s.is_replace_active()).unwrap_or(false);
    let replacement = search.as_ref().map(|s| s.replacement().to_string()).unwrap_or_default();
    ReplayResult {
        zoom: zoom_out,
        selection: sel,
        search_query,
        search_case,
        replace_active,
        replacement,
        overlay,
        accept,
        buffers_open,
    }
}

/// A plain `--screenshot` capture: resolve the project, replay `--keys`, fold the
/// replay's App-level state into the capture opts, then render one settled frame.
/// This is the heaviest mode (the only one that threads the full verification-hook
/// `CaptureOpts` + overlay/accept handling), so it lives in its own seam.
fn capture_screenshot(
    out: PathBuf,
    file: Option<PathBuf>,
    mut opts: CaptureOpts,
    keys: Vec<Action>,
    root: Option<PathBuf>,
    workspace: Option<PathBuf>,
    notes_root: PathBuf,
    config: Config,
) -> Result<()> {
            // Resolve the active project + its file index BEFORE the replay so a
            // `Cmd-O` in the key-spec summons a real, scoped go-to overlay.
            let active_root = resolve_root(&root, &file, config.project_root.as_deref());
            let proj = crate::project::Project::resolve(&active_root);
            let corpus = crate::index::build_index(&active_root);
            // Default the switch-project workspace to the active root's PARENT when
            // neither `--workspace` nor a config `workspace` was given, so the sidecar
            // reports an EFFECTIVE folder (and a replayed Cmd-Shift-P lists siblings).
            let effective_workspace = resolve_workspace(&workspace, &active_root);
            opts.project = Some(capture::ProjectInfo {
                root: active_root.clone(),
                name: proj.name.clone(),
                branch: proj.branch.clone(),
                dirty: proj.dirty,
                // The EFFECTIVE notes_root / workspace (flag > config > default), so a
                // `--config`-driven launch shows the configured folders with no flags.
                notes_root: Some(notes_root.clone()),
                workspace: Some(effective_workspace.clone()),
                keymap_flavor: config.keymap_flavor().config_name(),
            });

            let mut buffer = load_buffer(&file);
            // Replay `--keys` FIRST so the cursor/selection/search the spec
            // produces are what the capture reflects. Fold the App-level state
            // (zoom / selection / search) the replay produced into the capture
            // opts — but never clobber an explicit verification hook.
            // Default the switch-project workspace to the active root's PARENT
            // when no explicit `--workspace` was given, so a replayed `Cmd-Shift-P`
            // summons the picker listing the root's SIBLING projects (rather than
            // silently doing nothing). An explicit `--workspace` still overrides.
            //
            // Visual-line motion ORACLE: when the spec has keys, build an offscreen
            // pipeline shaped like the upcoming capture so headless motion reads the
            // SAME wrap geometry the live window does. Skipped for an empty spec (no
            // motion to resolve) and absent on GPU-less hosts (logical fallback).
            let oracle = if keys.is_empty() {
                None
            } else {
                capture::build_oracle(&buffer, &opts)
            };
            let res = replay_keys(
                &mut buffer,
                &keys,
                &corpus,
                &active_root,
                Some(effective_workspace.as_path()),
                &notes_root,
                &config,
                oracle.as_ref().map(|o| o.as_oracle()),
            );
            if opts.zoom.is_none() {
                opts.zoom = res.zoom;
            }
            if opts.selection.is_none() {
                opts.selection = res.selection;
            }
            if opts.search.is_none() {
                opts.search = res.search_query;
                opts.search_case_sensitive = opts.search_case_sensitive || res.search_case;
                // REPLACE mode the replay opened (Cmd-Option-F) — surfaced so the
                // panel's replace row renders + the sidecar reports it.
                opts.search_replace_active = res.replace_active;
                opts.search_replacement = res.replacement;
            }
            // If the replay ACCEPTED an overlay item, reflect it in the capture.
            // Goto is handled ALREADY, INLINE inside `replay_keys` (the
            // multi-buffer registry switch happens there, so a LATER Goto in
            // the same spec can see an EARLIER one's backgrounded buffer) —
            // re-doing it here would clobber that with a fresh disk read.
            // Project: re-root — re-resolve the project at the accepted
            // ABSOLUTE directory and overwrite the sidecar `project` block
            // (otherwise a switch-project replay leaves NO observable trace).
            if let Some((kind, val)) = &res.accept {
                match kind {
                    crate::overlay::OverlayKind::Goto => {}
                    crate::overlay::OverlayKind::Project => {
                        let new_root = std::path::PathBuf::from(val);
                        let proj = crate::project::Project::resolve(&new_root);
                        opts.project = Some(capture::ProjectInfo {
                            root: new_root,
                            name: proj.name.clone(),
                            branch: proj.branch.clone(),
                            dirty: proj.dirty,
                            notes_root: Some(notes_root.clone()),
                            workspace: Some(effective_workspace.clone()),
                            keymap_flavor: config.keymap_flavor().config_name(),
                        });
                    }
                    // History: RESTORE the accepted version into the buffer (an undoable
                    // edit), so a `--keys "Cmd-S-h <down> <enter>"` capture reflects the
                    // restored text — the same `history::load` + `set_text` the App runs,
                    // keyed by the same shared `source_path` derivation.
                    crate::overlay::OverlayKind::History => {
                        if let Some(path) =
                            crate::history::source_path(buffer.path(), None, buffer.is_note())
                        {
                            if let Some(content) = crate::history::load(&path, val) {
                                buffer.set_text(&content);
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Reflect any still-open overlay in the capture opts (and thus the
            // sidecar `overlay` block).
            if let Some(ov) = &res.overlay {
                // HISTORY timeline: the highlighted row's VERSION previews in the
                // document itself — resolve it here so the capture folds it over
                // the snapshot text and the sidecar reports `preview_id` + the
                // previewed `text` (exactly what the live preview shows).
                let preview = history_preview_for(ov, &buffer);
                opts.preview_text = preview.as_ref().map(|(_, content)| content.clone());
                opts.overlay = Some(capture::OverlayInfo {
                    active: true,
                    mode: ov.kind.as_str(),
                    query: ov.query.clone(),
                    items: ov.item_strings(),
                    empty: ov.empty_notice(),
                    bindings: ov.item_bindings(),
                    git: ov.item_git_tags(),
                    selected_index: ov.selected,
                    hint: ov.foot_hint(),
                    browse_dir: ov.browse_dir.clone(),
                    return_to: ov.return_to.map(|k| k.as_str()),
                    spell_target: ov.spell_target,
                    preview_id: preview.map(|(id, _)| id),
                    show_hidden: ov.show_hidden,
                    capture: ov.capture.as_ref().map(|c| capture::CaptureInfo {
                        command: c.cmd_name.clone(),
                        stage: match c.stage {
                            crate::overlay::CaptureStage::ChooseMode => "choose",
                            crate::overlay::CaptureStage::Recording => "recording",
                            crate::overlay::CaptureStage::Confirm => "confirm",
                        },
                        chord_mode: c.chord_mode,
                        captured: c.captured.clone(),
                        prompt: c.prompt(),
                    }),
                    notice: ov.notice.clone(),
                    // FACETED PICKER: the active lens + strip + per-row section labels so
                    // a `--keys "Cmd-T <right>"` capture renders + reports the faceted view.
                    // `None` for a non-faceting picker (no scheme).
                    lens: ov.active_facet_id(),
                    lens_strip: ov.lens_strip(),
                    sections: ov.item_sections(),
                });
            }
            // If a selection is requested (or one came from --keys), move the
            // buffer cursor to its END so the caret renders at the cursor end of
            // the region. A --keys replay already left the cursor where it
            // belongs, so only do this for an EXPLICIT --sel (no replay).
            if keys.is_empty() {
                if let Some((_, (l1, c1))) = opts.selection {
                    let end = buffer.line_col_to_char(l1, c1);
                    buffer.set_cursor(end);
                }
            }
            // WHICH-KEY force (`--whichkey`): render the SETTLED summoned panel by
            // deriving the `C-x` continuation rows from the command catalog + this
            // capture's config, exactly as the live App does on the pause. The live
            // 500ms timer is windowed (human-confirm); the shown STATE + derived list
            // are what a capture pins.
            if crate::whichkey::force_shown() {
                opts.whichkey = Some(
                    crate::whichkey::continuations_cx(&config.keys)
                        .into_iter()
                        .map(|c| (c.key, c.name))
                        .collect(),
                );
            }
            // MULTI-BUFFER: report the replay's final open-buffer count + which
            // one is active (its path, or the literal "scratch") — so a `--keys`
            // spec driving Goto A -> edit -> Goto B -> edit -> Goto A is
            // assertable straight from the sidecar (`buffers.open` stays 2,
            // `buffers.active` reports A again with its preserved cursor/text).
            opts.buffers = Some(capture::BuffersInfo {
                open: res.buffers_open,
                active: match buffer.path() {
                    Some(p) => p.display().to_string(),
                    None => "scratch".to_string(),
                },
            });
            capture::capture_with(&out, &buffer, &opts)?;
            println!("wrote {} (+ sidecar .json)", out.display());
            Ok(())
}

/// The HISTORY timeline's headless live preview: when the replay left the History
/// overlay OPEN, resolve its highlighted row's restore id to that version's
/// `(id, content)` via [`crate::history::load`] — keyed by the same shared
/// [`crate::history::source_path`] derivation the live App uses — so the capture
/// shows THAT VERSION in the document itself and the sidecar reports which.
/// `None` for every other overlay kind, the empty-state row, or an unresolvable
/// id (the capture then just shows the buffer — the live degrade). Pure over the
/// store, so it is unit-testable with a seeded log.
fn history_preview_for(
    ov: &crate::overlay::OverlayState,
    buffer: &Buffer,
) -> Option<(String, String)> {
    if ov.kind != crate::overlay::OverlayKind::History {
        return None;
    }
    let id = ov.selected_history_id()?.to_string();
    let path = crate::history::source_path(buffer.path(), None, buffer.is_note())?;
    let content = crate::history::load(&path, &id)?;
    Some((id, content))
}

/// Execute the resolved [`Mode`]: render a headless capture, run the typing
/// benchmark, or open the windowed editor. This is the dispatch the native
/// `fn main` defers to once argument parsing has chosen a mode. The heavy
/// `--screenshot` path lives in [`capture_screenshot`]; the lighter modes stay
/// inline.
pub(crate) fn run(mode: Mode) -> Result<()> {
    match mode {
        Mode::Screenshot {
            out,
            file,
            opts,
            keys,
            root,
            workspace,
            notes_root,
            config,
        } => capture_screenshot(out, file, opts, keys, root, workspace, notes_root, config),
        Mode::ScreenshotMotion { out, file, keys } => {
            let mut buffer = load_buffer(&file);
            let root = resolve_root(&None, &file, None);
            replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
            capture::capture_motion(&out, &buffer)?;
            println!("wrote {} (mid-glide, + sidecar .json)", out.display());
            Ok(())
        }
        Mode::ScreenshotMotionVertical { out, file, keys } => {
            let mut buffer = load_buffer(&file);
            let root = resolve_root(&None, &file, None);
            replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
            capture::capture_motion_vertical(&out, &buffer)?;
            println!("wrote {} (mid-glide vertical, + sidecar .json)", out.display());
            Ok(())
        }
        Mode::ScreenshotMotionDiagonal { out, file, keys } => {
            let mut buffer = load_buffer(&file);
            let root = resolve_root(&None, &file, None);
            replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
            capture::capture_motion_diagonal(&out, &buffer)?;
            println!("wrote {} (mid-glide diagonal, + sidecar .json)", out.display());
            Ok(())
        }
        Mode::CaptureTimeline {
            out,
            file,
            keys,
            steps,
            root,
            canvas,
            dpi,
        } => {
            let active_root = resolve_root(&root, &file, None);
            let proj = crate::project::Project::resolve(&active_root);
            let corpus = crate::index::build_index(&active_root);
            let notes_root = active_root.clone();
            let opts = CaptureOpts {
                project: Some(capture::ProjectInfo {
                    root: active_root.clone(),
                    name: proj.name.clone(),
                    branch: proj.branch.clone(),
                    dirty: proj.dirty,
                    notes_root: None,
                    workspace: None,
                    keymap_flavor: "native",
                }),
                canvas,
                dpi,
                ..CaptureOpts::default()
            };

            let mut buffer = load_buffer(&file);
            // Split the replay: all-but-last set up the ORIGIN, the LAST chord is
            // the NAVIGATION move whose glide the timeline captures. With an empty
            // or single-key spec the origin is wherever the prefix left the cursor.
            let (last, init) = match keys.split_last() {
                Some((last, init)) => (Some(last.clone()), init.to_vec()),
                None => (None, Vec::new()),
            };
            if !init.is_empty() {
                replay_keys(
                    &mut buffer,
                    &init,
                    &corpus,
                    &active_root,
                    None,
                    &notes_root,
                    &Config::empty(),
                    None,
                );
            }
            let origin = buffer.cursor_line_col();
            if let Some(last) = last {
                replay_keys(
                    &mut buffer,
                    std::slice::from_ref(&last),
                    &corpus,
                    &active_root,
                    None,
                    &notes_root,
                    &Config::empty(),
                    None,
                );
            }
            capture::capture_timeline(&out, &buffer, origin, &steps, &opts)?;
            println!(
                "wrote {} timeline frames for {} (+ per-step sidecars)",
                steps.len(),
                out.display()
            );
            Ok(())
        }
        Mode::CaptureHeld {
            out,
            file,
            keys,
            dir,
            steps,
            root,
            canvas,
            dpi,
        } => {
            let active_root = resolve_root(&root, &file, None);
            let proj = crate::project::Project::resolve(&active_root);
            let corpus = crate::index::build_index(&active_root);
            let notes_root = active_root.clone();
            let opts = CaptureOpts {
                project: Some(capture::ProjectInfo {
                    root: active_root.clone(),
                    name: proj.name.clone(),
                    branch: proj.branch.clone(),
                    dirty: proj.dirty,
                    notes_root: None,
                    workspace: None,
                    keymap_flavor: "native",
                }),
                canvas,
                dpi,
                ..CaptureOpts::default()
            };

            let mut buffer = load_buffer(&file);
            // The FULL `--keys` replay sets up the ORIGIN the held burst starts from
            // (e.g. C-n's + C-f's to land mid-line); the held re-targeting then
            // drives the motion deterministically from there.
            if !keys.is_empty() {
                replay_keys(&mut buffer, &keys, &corpus, &active_root, None, &notes_root, &Config::empty(), None);
            }
            let origin = buffer.cursor_line_col();
            capture::capture_held(&out, &buffer, origin, dir, &steps, &opts)?;
            println!(
                "wrote {} held frames for {} (+ per-step sidecars)",
                steps.len(),
                out.display()
            );
            Ok(())
        }
        Mode::BenchTyping => bench::run(),
        Mode::BenchPerf => crate::render::perfbench::run(),
        Mode::BenchFrame => crate::render::framebench::run(),
        Mode::BenchThemeBurst => crate::render::framebench::run_theme_burst(),
        Mode::Windowed {
            file,
            root,
            workspace,
            notes_root,
            config,
            wait,
        } => {
            // STICKY PROJECT RESTORE: on a bare launch (no file argument, no
            // explicit --root) the remembered project root wins; see
            // `resolve_root`'s doc comment.
            let active_root = resolve_root(&root, &file, config.project_root.as_deref());
            // Pass the RAW flags + config; `App::new` folds them (flag > config >
            // default) and re-folds on a live config reload. `wait` (native-only,
            // the single-instance daemon's `--wait`) rides straight through.
            app::run(file, active_root, workspace, notes_root, config, wait)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // CONVENTION-PROOF SHADOW: this whole file's `--keys` replay tests hardcode
    // MAC-form literal specs ("Cmd-S-h", "s-p", a bare "C-n"/"C-x" whose letter
    // Linux's collision table displaces, …) — pinning resolution to
    // `Convention::Mac` is the honest fix (these tests document specifically
    // what a MAC-convention chord does; Linux's own displacement/collision
    // behavior is separately, exhaustively law-tested in `keymap.rs`). This
    // local `keyspec` module SHADOWS the real `crate::keyspec` module for every
    // `keyspec::parse_keys(...)` call below, so none of the ~30 hardcoded specs
    // needed individual rewriting.
    mod keyspec {
        pub fn parse_keys(spec: &str) -> anyhow::Result<Vec<crate::keymap::Action>> {
            crate::keyspec::parse_keys_pinned(spec, crate::convention::Convention::Mac)
        }
    }

    #[test]
    fn replay_keys_builds_selection_from_mark_and_motion() {
        // replay_keys is pure (Buffer + actions, no GPU) but was only reached
        // through the adapter-gated capture tests. Drive it directly: type "abc",
        // mark with C-Space at the end, then move left twice — the post-replay
        // ReplayResult must carry the ordered region.
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("a b c C-Space Left Left").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert_eq!(res.selection, Some(((0, 1), (0, 3))), "mark@3 + two Lefts -> [1,3)");
    }

    // ── SAVE-FEEDBACK round: Cmd-S on a scratch buffer is headless-reachable ──

    #[test]
    fn replay_keys_cmd_s_on_scratch_buffer_converts_it_into_a_note_under_notes_root() {
        // The scratch-conversion effect (`Effect::ConvertScratchAndSave`) IS
        // headless-reachable, and behaves IDENTICALLY to the live App: a
        // no-path, non-note buffer's Cmd-S creates a real file under the
        // harness's own `notes_root`, through the active `fs` backend (here:
        // an `InMemoryFs`, so the assertion is a real file-creation check,
        // not a guess).
        use crate::fs::{FileSystem, InMemoryFs};
        let mem = InMemoryFs::new();
        let _g = crate::fs::FsGuard::install(std::sync::Arc::new(mem.clone()));
        let mut buffer = Buffer::scratch();
        assert!(buffer.path().is_none() && !buffer.is_note());
        let keys = keyspec::parse_keys("m e a d o w s-s").unwrap();
        let root = PathBuf::from("/tmp");
        let notes_root = PathBuf::from("/tmp/notes");
        let _res =
            replay_keys(&mut buffer, &keys, &[], &root, None, &notes_root, &Config::empty(), None);
        assert!(buffer.is_note(), "Cmd-S promoted the scratch buffer into a note");
        let p = buffer.path().expect("a real path was derived");
        assert!(p.starts_with(&notes_root), "landed under the harness's own notes_root: {p:?}");
        assert_eq!(mem.read_to_string(p).unwrap(), "meadow");
    }

    #[test]
    fn replay_keys_cmd_s_on_an_already_pathed_buffer_is_a_plain_save() {
        // The contrast case: an already-pathed buffer's Cmd-S is a PLAIN save
        // (the pre-existing behavior) — never routed through the scratch
        // conversion, never re-homed under notes_root.
        use crate::fs::{FileSystem, InMemoryFs};
        let mem = InMemoryFs::new().with_dir("/proj");
        let _g = crate::fs::FsGuard::install(std::sync::Arc::new(mem.clone()));
        let mut buffer = Buffer::scratch();
        buffer.set_path(PathBuf::from("/proj/a.md"));
        let keys = keyspec::parse_keys("h i s-s").unwrap();
        let root = PathBuf::from("/proj");
        let notes_root = PathBuf::from("/tmp/notes");
        let _res =
            replay_keys(&mut buffer, &keys, &[], &root, None, &notes_root, &Config::empty(), None);
        assert!(!buffer.is_note(), "an already-pathed buffer never becomes a note");
        assert_eq!(buffer.path(), Some(std::path::Path::new("/proj/a.md")));
        assert_eq!(mem.read_to_string(std::path::Path::new("/proj/a.md")).unwrap(), "hi");
    }

    // ── NOTES VERBS round: the Rename minibuffer stays --keys-drivable ──

    #[test]
    fn replay_keys_drives_the_rename_minibuffer_prompt_and_sidecar_reflects_typing() {
        // Cmd-P → "rename" → Enter opens the Rename overlay pre-filled with the
        // current filename; typing MORE characters extends it live — all through
        // the shared core, so both the overlay STATE and its sidecar-facing
        // `foot_hint()` (the same seam the Keybindings capture prompt rides)
        // reflect the in-progress edit with zero live App involved.
        let mut buffer = Buffer::scratch();
        buffer.set_path(PathBuf::from("/proj/old.md"));
        let keys = keyspec::parse_keys("s-p r e n a m e RET 2").unwrap();
        let root = PathBuf::from("/proj");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        let ov = res.overlay.expect("Rename note… opens the minibuffer overlay");
        assert_eq!(ov.kind, crate::overlay::OverlayKind::Rename);
        assert_eq!(ov.corpus, vec!["old.md2".to_string()], "typing extends the seeded name");
        assert_eq!(
            ov.foot_hint(),
            "rename to: old.md2   Enter commit   Esc cancel",
            "the live prompt is sidecar-visible via the same foot_hint seam Keybindings uses"
        );
    }

    #[test]
    fn replay_keys_rename_minibuffer_esc_cancels_with_no_overlay_left() {
        let mut buffer = Buffer::scratch();
        buffer.set_path(PathBuf::from("/proj/old.md"));
        let keys = keyspec::parse_keys("s-p r e n a m e RET x Esc").unwrap();
        let root = PathBuf::from("/proj");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert!(res.overlay.is_none(), "Esc closes the minibuffer outright, no breadcrumb pop");
        assert_eq!(buffer.path(), Some(std::path::Path::new("/proj/old.md")), "no disk rename happened");
    }

    #[test]
    fn replay_keys_rename_minibuffer_does_not_open_on_a_pathless_buffer() {
        // A pathless (scratch) buffer has nothing to rename yet — the pure gate
        // is a buffer-state check (no fs needed), so this is a calm no-op even
        // headlessly.
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("s-p r e n a m e RET").unwrap();
        let root = PathBuf::from("/proj");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert!(res.overlay.is_none(), "nothing to rename on a pathless buffer");
    }

    // ── LINKS V2: Cmd-K stays --keys-drivable through the shared core ──

    #[test]
    fn replay_keys_cmd_k_wraps_a_selection_as_a_markdown_link() {
        // Type "hello", mark it with C-Space + move left across it, Cmd-K opens
        // the URL minibuffer pre-filled empty (WithText mode wrapping "hello"),
        // type a URL, RET commits — one atomic edit, fully sidecar-drivable.
        let mut buffer = Buffer::scratch();
        let keys =
            keyspec::parse_keys("h e l l o C-Space Left Left Left Left Left s-k h t t p s : / / x . t e s t RET")
                .unwrap();
        let root = PathBuf::from("/proj");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert!(res.overlay.is_none(), "commit closes the minibuffer");
        assert_eq!(buffer.text(), "[hello](https://x.test)");
    }

    #[test]
    fn replay_keys_cmd_k_prompt_is_sidecar_visible_while_typing() {
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys(
            "h e l l o C-Space Left Left Left Left Left s-k h t t p s : / /",
        )
        .unwrap();
        let root = PathBuf::from("/proj");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        let ov = res.overlay.expect("Cmd-K opens the link minibuffer");
        assert_eq!(ov.kind, crate::overlay::OverlayKind::InsertLink);
        assert_eq!(ov.corpus, vec!["https://".to_string()]);
        assert_eq!(
            ov.foot_hint(),
            "link to: https://   Enter commit   Esc cancel",
            "the live prompt is sidecar-visible via the same foot_hint seam Rename/Keybindings use"
        );
        // The buffer is UNTOUCHED until commit.
        assert_eq!(buffer.text(), "hello");
    }

    #[test]
    fn replay_keys_cmd_k_esc_cancels_with_no_buffer_change() {
        let mut buffer = Buffer::scratch();
        let keys =
            keyspec::parse_keys("h e l l o C-Space Left Left Left Left Left s-k x x x Esc").unwrap();
        let root = PathBuf::from("/proj");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert!(res.overlay.is_none(), "Esc closes the minibuffer outright");
        assert_eq!(buffer.text(), "hello", "cancel never edits the buffer");
    }

    #[test]
    fn replay_keys_cmd_k_no_selection_inserts_empty_markup_caret_between_brackets() {
        // No selection, no existing link under the caret: Cmd-K inserts empty
        // `[](url)` markup; committing an empty URL is still a harmless, one-shot
        // edit (never a silent cancel the user didn't ask for).
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("h i s-k RET").unwrap();
        let root = PathBuf::from("/proj");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert!(res.overlay.is_none());
        assert_eq!(buffer.text(), "hi[]()");
    }

    #[test]
    fn replay_keys_cmd_k_on_a_non_markdown_buffer_is_a_calm_no_op() {
        use std::path::PathBuf as PB;
        let mut buffer = Buffer::scratch();
        buffer.set_path(PB::from("/proj/main.rs"));
        buffer.insert_char('a');
        assert!(!buffer.is_markdown());
        let keys = keyspec::parse_keys("s-k").unwrap();
        let root = PathBuf::from("/proj");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert!(res.overlay.is_none(), "Cmd-K is a calm no-op on a non-markdown buffer");
        assert_eq!(buffer.text(), "a");
    }

    #[test]
    fn replay_keys_runs_palette_chain_into_overlay() {
        // The command-palette run-on-Enter chain (Effect::RunAction fed back through
        // the core in the same replay): Cmd-P opens the palette, "goto" filters to
        // "Go to file", Enter runs OpenGoto, which the worklist re-dispatches into
        // the Goto overlay as the final captured state.
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("s-p g o t o RET").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert_eq!(
            res.overlay.map(|o| o.kind),
            Some(crate::overlay::OverlayKind::Goto),
            "palette Enter on 'Go to file' chains into the Goto overlay",
        );
    }

    #[test]
    fn replay_keys_drives_palette_guide_and_opens_the_guide_buffer() {
        // Cmd-P → "guide" filters to "Guide" → Enter runs Action::OpenGuide,
        // which (headlessly, no filesystem write — see Effect::OpenGuide above)
        // loads the embedded GUIDE.md text straight into the buffer. Mirrors the
        // palette-chain shape `replay_keys_runs_palette_chain_into_overlay` uses
        // for an overlay-opening command, but for a buffer-opening one instead —
        // so the palette door is verified all the way to its settled content.
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("s-p g u i d e RET").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert!(res.overlay.is_none(), "the palette closed itself on accept, no overlay left open");
        assert_eq!(buffer.text(), crate::guide::GUIDE_MD, "the buffer now holds the embedded guide text");
        assert!(buffer.path().is_none(), "headless replay never writes/loads a real on-disk guide.md");
    }

    #[test]
    fn replay_keys_palette_sub_picker_stamps_command_breadcrumb() {
        // Cmd-P → "theme" filters to "Switch theme…" → Enter runs OpenThemeMenu, which
        // the worklist re-dispatches into the Theme picker STAMPED return_to = Command
        // (the palette re-dispatch breadcrumb seam). Serialize on the theme lock: the
        // picker reads/reverts the process-global active theme.
        let _g = crate::testlock::serial();
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("s-p t h e m e RET").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        let ov = res.overlay.expect("palette chained into the theme picker");
        assert_eq!(ov.kind, crate::overlay::OverlayKind::Theme);
        assert_eq!(
            ov.return_to,
            Some(crate::overlay::OverlayKind::Command),
            "a palette-opened sub-picker remembers its way back to the palette",
        );
        crate::theme::set_active(0);
    }

    #[test]
    fn replay_keys_palette_theme_esc_pops_back_to_palette() {
        // The breadcrumb POP end-to-end: palette → theme picker → Esc lands back on
        // the PALETTE (not the buffer). The re-summoned palette carries no breadcrumb
        // of its own (single-level).
        let _g = crate::testlock::serial();
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("s-p t h e m e RET Esc").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        let ov = res.overlay.expect("Esc pops back to the palette, not the buffer");
        assert_eq!(ov.kind, crate::overlay::OverlayKind::Command, "back at the command palette");
        assert_eq!(ov.return_to, None, "single-level: the palette carries no breadcrumb");
        crate::theme::set_active(0);
    }

    #[test]
    fn replay_keys_palette_theme_keep_closes_to_buffer_not_a_recent_menu() {
        // SHIP-BLOCKER REGRESSION, end-to-end: palette → theme → Enter (keep) LANDS IN
        // THE BUFFER with no overlay left — NOT back on the palette (which re-appears on
        // its Recent lens and reads as a stray "recent files menu", the user report).
        // The theme is still committed by the keep (`res.accept`). Contrast the Esc test
        // above, which DOES pop back — only ACCEPT closes to the buffer.
        let _g = crate::testlock::serial();
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("s-p t h e m e RET RET").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert!(res.overlay.is_none(), "keeping a palette-launched theme lands in the buffer");
        assert!(
            matches!(res.accept, Some((crate::overlay::OverlayKind::Theme, _))),
            "the theme keep still committed, got {:?}",
            res.accept
        );
        crate::theme::set_active(0);
    }

    #[test]
    fn replay_keys_goto_open_file_closes_all_no_overlay() {
        // A NAVIGATING accept closes the whole stack: ⌘O → Enter on a file lands you
        // IN the file with NO overlay left open (like a palette value-pick keep, and
        // unlike the Esc breadcrumb pop).
        let mut buffer = Buffer::scratch();
        let corpus = vec!["README.md".to_string()];
        let root = PathBuf::from("/tmp");
        let keys = keyspec::parse_keys("s-o RET").unwrap();
        let res = replay_keys(&mut buffer, &keys, &corpus, &root, None, &root, &Config::empty(), None);
        assert!(res.overlay.is_none(), "opening a file closes the overlay to the buffer");
        assert_eq!(
            res.accept,
            Some((crate::overlay::OverlayKind::Goto, "README.md".to_string())),
            "the file open still fired",
        );
    }

    #[test]
    fn replay_keys_goto_hides_dotfiles_until_cmd_shift_period() {
        // The go-to picker HIDES dot-prefixed corpus entries by default; Cmd-Shift-.
        // (headless `s-S-.`) reveals them. Drive it end-to-end through the real
        // keymap + apply_core, asserting the overlay listing at each phase.
        let mut buffer = Buffer::scratch();
        let corpus = vec![
            ".gitignore".to_string(),
            ".env".to_string(),
            "README.md".to_string(),
            "src/main.rs".to_string(),
        ];
        let root = PathBuf::from("/tmp");
        // Open the go-to overlay (Cmd-O), then assert dotfiles are hidden.
        let keys = keyspec::parse_keys("s-o").unwrap();
        let res = replay_keys(&mut buffer, &keys, &corpus, &root, None, &root, &Config::empty(), None);
        let ov = res.overlay.expect("goto overlay open");
        assert_eq!(ov.kind, crate::overlay::OverlayKind::Goto);
        assert!(!ov.show_hidden);
        let shown = ov.item_strings();
        assert!(!shown.iter().any(|s| s == ".gitignore"), "dotfile hidden by default: {shown:?}");
        assert!(shown.iter().any(|s| s == ".env"), ".env stays visible: {shown:?}");
        assert!(shown.iter().any(|s| s == "README.md"));
        // Now open + toggle: the reveal chord flips show_hidden and .gitignore appears.
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("s-o s-S-.").unwrap();
        let res = replay_keys(&mut buffer, &keys, &corpus, &root, None, &root, &Config::empty(), None);
        let ov = res.overlay.expect("goto overlay still open after toggle");
        assert!(ov.show_hidden, "Cmd-Shift-. revealed dotfiles");
        assert!(
            ov.item_strings().iter().any(|s| s == ".gitignore"),
            "dotfile shown after the reveal toggle: {:?}",
            ov.item_strings()
        );
    }

    #[test]
    fn replay_keys_asset_cleaner_lists_only_the_orphans_from_the_scan() {
        // Summon the ASSET CLEANER headlessly via the palette (Cmd-P → "clean" → Enter
        // runs OpenAssetClean, which chains into the Assets overlay) and assert the
        // orphan list the scan produced — end-to-end through the real keymap +
        // apply_core + the run.rs orphan gather over the FileSystem seam.
        use std::sync::Arc;
        let root = PathBuf::from("/proj");
        let mem = crate::fs::InMemoryFs::new()
            .with_file("/proj/doc.md", "text\n![a](assets/used.png)\n")
            .with_file("/proj/assets/used.png", "U")
            .with_file("/proj/assets/orphan.png", "OO");
        let corpus = vec![
            "assets/orphan.png".to_string(),
            "assets/used.png".to_string(),
            "doc.md".to_string(),
        ];
        crate::fs::with_fs(Arc::new(mem), || {
            let mut buffer = Buffer::scratch();
            let keys = keyspec::parse_keys("s-p c l e a n RET").unwrap();
            let res =
                replay_keys(&mut buffer, &keys, &corpus, &root, None, &root, &Config::empty(), None);
            let ov = res.overlay.expect("asset cleaner open after the palette chain");
            assert_eq!(ov.kind, crate::overlay::OverlayKind::Assets);
            // Only the UNREFERENCED asset is listed; the primary cell is the leaf name.
            assert_eq!(ov.item_strings(), vec!["orphan.png"]);
            // The secondary column carries the human size + parent dir.
            assert_eq!(ov.item_bindings(), vec!["2 B · assets"]);
            // The sidecar mode string agrees.
            assert_eq!(ov.kind.as_str(), "assets");
        });
    }

    #[test]
    fn replay_keys_project_hides_dotfolders_marks_git_tag() {
        // The switch-project picker (Cmd-Shift-P) over a real (in-memory) workspace: it now
        // HIDES dotfolders (`.claude`) by default while keeping the synthetic "."
        // accept row; a git-repo child carries a `"git"` SECONDARY-column tag (no name
        // bullet); Cmd-Shift-. reveals the dotfolders. Driven end-to-end through the
        // real keymap + apply_core + `browse_level`'s filesystem seam.
        use std::sync::Arc;
        let ws = PathBuf::from("/ws");
        let mem = crate::fs::InMemoryFs::new()
            .with_dir("/ws/.claude")
            .with_dir("/ws/.git") // junk-filtered before the overlay ever sees it
            .with_dir("/ws/plain")
            .with_dir("/ws/repo")
            .with_dir("/ws/repo/.git"); // marks `repo` a git repo
        crate::fs::with_fs(Arc::new(mem), || {
            // Open the switch-project overlay over the workspace children (Cmd-Shift-P).
            let mut buffer = Buffer::scratch();
            let keys = keyspec::parse_keys("s-S-p").unwrap();
            let res = replay_keys(
                &mut buffer, &keys, &[], &ws, Some(ws.as_path()), &ws, &Config::empty(), None,
            );
            let ov = res.overlay.expect("switch-project overlay open");
            assert_eq!(ov.kind, crate::overlay::OverlayKind::Project);
            assert!(!ov.show_hidden);
            let shown = ov.item_strings();
            // The "." accept-this-folder row survives the dotfolder filter.
            assert!(shown.iter().any(|s| s == "."), "'.' accept row kept: {shown:?}");
            // `.claude` (and junk `.git`) are hidden; the plain + repo folders show.
            assert!(!shown.iter().any(|s| s.starts_with(".claude")), "dotfolder hidden: {shown:?}");
            assert!(!shown.iter().any(|s| s.starts_with(".git")), "junk .git hidden: {shown:?}");
            assert!(shown.iter().any(|s| s.starts_with("plain")), "plain shown: {shown:?}");
            assert!(shown.iter().any(|s| s.starts_with("repo")), "repo shown: {shown:?}");
            // No name carries the old bullet; the git repo carries the "git" tag, the
            // plain folder none.
            assert!(shown.iter().all(|s| !s.contains('•')), "no name bullet: {shown:?}");
            let tags = ov.item_git_tags();
            let ipos = |name: &str| shown.iter().position(|s| s.starts_with(name)).unwrap();
            assert_eq!(tags[ipos("repo")], "git", "repo is git-tagged");
            assert_eq!(tags[ipos("plain")], "", "plain folder has no tag");

            // Cmd-Shift-. reveals the overlay-hidden dotfolder (`.claude`); junk `.git`
            // stays hidden (it never reaches the overlay corpus).
            let mut buffer = Buffer::scratch();
            let keys = keyspec::parse_keys("s-S-p s-S-.").unwrap();
            let res = replay_keys(
                &mut buffer, &keys, &[], &ws, Some(ws.as_path()), &ws, &Config::empty(), None,
            );
            let ov = res.overlay.expect("project overlay still open after toggle");
            assert!(ov.show_hidden, "Cmd-Shift-. revealed dotfolders");
            let revealed = ov.item_strings();
            assert!(revealed.iter().any(|s| s.starts_with(".claude")), "revealed: {revealed:?}");
            assert!(revealed.iter().any(|s| s == "."), "'.' still present after reveal");
        });
    }

    #[test]
    fn replay_keys_drives_rebind_menu_capture() {
        // The GAME-STYLE REBIND MENU, driven entirely through the headless replay:
        // Cmd-P → "keyb" → Enter opens the Keybindings menu, "undo" filters to Undo,
        // Enter starts a capture (ChooseMode), Enter begins recording (KEY), and a
        // plain 'q' is captured → committed (the menu's NOTICE reflects the binding).
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("s-p k e y b RET u n d o RET RET q").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        let ov = res.overlay.expect("the rebind menu stays open after a commit");
        assert_eq!(ov.kind, crate::overlay::OverlayKind::Keybindings);
        assert!(ov.capture.is_none(), "capture closed after committing the key");
        assert_eq!(ov.notice, "bound undo -> q", "notice reflects the captured binding");
    }

    #[test]
    fn replay_keys_rebind_menu_recording_state_visible() {
        // Stopping mid-capture leaves the RECORDING sub-state on the overlay, so the
        // sidecar `capture` block is assertable (mode, command, empty captured list).
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("s-p k e y b RET s a v e RET Down RET").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        let ov = res.overlay.expect("menu open");
        let cap = ov.capture.expect("a capture is in progress");
        assert_eq!(cap.cmd_name, "Save");
        assert_eq!(cap.stage, crate::overlay::CaptureStage::Recording);
        assert!(cap.chord_mode, "Down selected the CHORD row before recording");
        assert!(cap.captured.is_empty(), "no combo pressed yet");
    }

    #[test]
    fn replay_keys_settings_cjk_picker_round_trips_headlessly() {
        // The CJK-priority LANGUAGE picker's whole point, driven end-to-end through
        // the headless `--keys` replay: Cmd-P -> "settings" -> Enter opens the
        // Settings menu; "ambiguous" filters to the "Ambiguous CJK reads as" row;
        // Enter opens the CjkLang sub-picker (breadcrumbed back to Settings); three
        // Downs select "Korean" (Japanese/Simplified/Traditional/Korean order);
        // Enter PROMOTES it (core-level, so this is observable with no live App at
        // all) and pops back to Settings — whose re-summoned value cell reads
        // "Korean", not the raw "ko" code.
        let _g = crate::testlock::serial();
        crate::frontmatter::set_cjk_priority(&crate::frontmatter::DEFAULT_CJK_PRIORITY);
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys(
            "s-p s e t t i n g s RET a m b i g u o u s RET Down Down Down RET",
        )
        .unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);

        // The live global was promoted (core-level — the reason this test needs no
        // App at all).
        assert_eq!(
            crate::frontmatter::cjk_priority(),
            vec![
                crate::frontmatter::Lang::Ko,
                crate::frontmatter::Lang::Ja,
                crate::frontmatter::Lang::ZhHans,
                crate::frontmatter::Lang::ZhHant,
            ],
        );

        let ov = res.overlay.expect("popped back to the Settings menu, not closed");
        assert_eq!(ov.kind, crate::overlay::OverlayKind::Settings, "back at Settings");
        assert_eq!(ov.return_to, None, "single-level: no N-deep stack");
        let row_idx = crate::settings::SETTINGS
            .iter()
            .position(|r| r.name == "Ambiguous CJK reads as")
            .unwrap();
        assert_eq!(
            ov.bindings[row_idx], "Korean",
            "the re-summoned Settings menu's value cell is FRESH, in writer-words"
        );

        crate::frontmatter::set_cjk_priority(&crate::frontmatter::DEFAULT_CJK_PRIORITY);
    }

    #[test]
    fn replay_keys_page_reset_restores_default_measure() {
        // The "no easy way back" fix: mirror `--measure 40` (the flag writes the
        // process-global directly, exactly like this), then replay the "Reset Page
        // Width" Action (no default chord — palette/double-click only, so it's
        // constructed directly here rather than parsed from a `--keys` chord,
        // matching how `replay_keys` already takes a resolved `Action` stream). The
        // sidecar's `page.measure` field reads this SAME global, so this is the
        // capture-level half of the reset (the config-file override removal is
        // App-only + unit-tested separately in `config/`). Holds the process-wide
        // page TEST_LOCK and restores it after, like every other page-global test.
        let _pg = crate::testlock::serial();
        crate::page::set_measure(40);
        let mut buffer = Buffer::scratch();
        let root = PathBuf::from("/tmp");
        let keys = vec![Action::PageReset];
        let _ = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert_eq!(
            crate::page::measure(),
            crate::page::DEFAULT_MEASURE,
            "PageReset snaps the measure back to the built-in default"
        );
        crate::page::set_measure(crate::page::DEFAULT_MEASURE); // leave as found
    }

    #[test]
    fn replay_keys_page_reset_restores_the_code_default_for_a_code_buffer() {
        // The prose/code page-width split: PageReset on a CODE buffer (a `.rs`
        // path) must snap to DEFAULT_MEASURE_CODE (100), never the prose default
        // (70) — `Action::PageReset` resolves via `ctx.buffer.page_class()` on
        // the shared `apply_core` seam, so this is byte-identical to the live
        // App's own reset.
        let _pg = crate::testlock::serial();
        crate::page::set_measure(40);
        let mut buffer = Buffer::from_str("fn main() {}\n");
        buffer.set_path(PathBuf::from("/tmp/main.rs"));
        let root = PathBuf::from("/tmp");
        let keys = vec![Action::PageReset];
        let _ = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert_eq!(
            crate::page::measure(),
            crate::page::DEFAULT_MEASURE_CODE,
            "PageReset on a code buffer snaps to the CODE default, not the prose one"
        );
        crate::page::set_measure(crate::page::DEFAULT_MEASURE); // leave as found
    }

    #[test]
    fn replay_keys_goto_switch_reapplies_measure_per_buffer_kind() {
        // The prose/code page-width split's HEADLESS switch wiring: a `--keys`
        // Goto from a `.md` fixture to a `.rs` fixture (and back) re-applies the
        // sticky measure for whichever kind is NOW active, exactly like the live
        // App's `load_path` -> `sync_page_measure`. Configured overrides (not just
        // the built-in defaults) flow through too, since both read
        // `Config::measure_for`.
        let _fs = crate::testlock::serial();
        let _pg = crate::testlock::serial();
        let measure0 = crate::page::measure();
        let dir = std::env::temp_dir().join(format!("awl-mb-measure-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.md"), "# hello\n").unwrap();
        std::fs::write(dir.join("b.rs"), "fn main() {}\n").unwrap();
        let cfg = Config { page_width_prose: Some(55), page_width_code: Some(120), ..Config::empty() };
        let mut buffer = Buffer::scratch();
        let corpus = vec!["a.md".to_string(), "b.rs".to_string()];
        crate::page::set_measure(1); // deliberately wrong, so the switch below can't coincide

        let keys_to_b = keyspec::parse_keys("s-o b . r s RET").unwrap();
        let _ = replay_keys(&mut buffer, &keys_to_b, &corpus, &dir, None, &dir, &cfg, None);
        assert_eq!(crate::page::measure(), 120, "b.rs (code) picks up the configured code measure");

        let keys_to_a = keyspec::parse_keys("s-o a . m d RET").unwrap();
        let _ = replay_keys(&mut buffer, &keys_to_a, &corpus, &dir, None, &dir, &cfg, None);
        assert_eq!(crate::page::measure(), 55, "back to a.md (prose) picks up the configured prose measure");

        crate::page::set_measure(measure0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replay_scrolled_deep_then_open_swaps_to_the_short_file() {
        // The SCROLLED-DEEP-THEN-OPEN replay (the open-then-blank-screen hunt): park
        // the cursor at the END of a long document (Cmd-Down — cursor-follow scroll
        // then sits far past a one-line file's end), summon the Goto picker (Cmd-O),
        // filter to the short file, and accept with Enter. The replay must surface
        // the ACCEPT; the RunCapture arm's swap (mirrored here) yields the SHORT
        // file's buffer with the cursor at (0,0) — the capture re-derives its follow
        // scroll from THAT cursor, so the frame can never render past the new
        // document's EOF. Locks the headless half of the hunt (the live half is the
        // App view-text cache across a swap, tested in `app::tests`).
        // Reads the REAL disk through the fs seam → hold the fs TEST_LOCK so a
        // parallel InMemoryFs installation can't swallow the temp files.
        let _fs = crate::testlock::serial();
        let dir = std::env::temp_dir().join(format!("awl-goto-swap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let long: String = (0..300).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.join("long.txt"), &long).unwrap();
        std::fs::write(dir.join("short.txt"), "just one line\n").unwrap();
        let mut buffer = Buffer::from_file(&dir.join("long.txt"));
        let keys = keyspec::parse_keys("s-Down s-o s h o r t RET").unwrap();
        let corpus = vec!["long.txt".to_string(), "short.txt".to_string()];
        let res =
            replay_keys(&mut buffer, &keys, &corpus, &dir, None, &dir, &Config::empty(), None);
        let (kind, val) = res.accept.expect("Enter accepts the filtered picker row");
        assert_eq!(kind, crate::overlay::OverlayKind::Goto);
        assert_eq!(val, "short.txt");
        // The RunCapture arm swaps in the accepted file's buffer; scroll derives
        // from its fresh (0,0) cursor, never the old document's depth.
        let swapped = Buffer::from_file(&crate::index::resolve(&dir, &val));
        assert_eq!(swapped.text(), "just one line\n");
        assert_eq!(swapped.cursor_line_col(), (0, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replay_keys_goto_a_then_b_then_a_preserves_edits_and_cursor() {
        // THE MULTI-BUFFER v1 win, driven entirely through `--keys`: A -> edit ->
        // B -> edit -> A round-trips through the SAME `crate::buffers::BufferRegistry`
        // the live App uses (wired inline inside `replay_keys`, not deferred to the
        // caller), so the FINAL buffer must be A's LIVE edited content — not a fresh
        // disk re-read — with A's own cursor. This is what makes "assert preserved
        // cursor after an A -> B -> A switch" a headless, agent-verifiable capture.
        let _fs = crate::testlock::serial();
        let dir = std::env::temp_dir().join(format!("awl-mb-replay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "alpha\n").unwrap();
        std::fs::write(dir.join("b.txt"), "beta\n").unwrap();
        let mut buffer = Buffer::scratch();
        let corpus = vec!["a.txt".to_string(), "b.txt".to_string()];
        let keys = keyspec::parse_keys(
            "s-o a . t x t RET X s-o b . t x t RET Y s-o a . t x t RET",
        )
        .unwrap();
        let res =
            replay_keys(&mut buffer, &keys, &corpus, &dir, None, &dir, &Config::empty(), None);
        assert_eq!(
            buffer.text(),
            "Xalpha\n",
            "A's live edit survived the A -> B -> A round trip, not a fresh disk read"
        );
        assert_eq!(buffer.path(), Some(dir.join("a.txt").as_path()), "A is active again");
        assert_eq!(
            res.buffers_open, 3,
            "the launch scratch + A (active) + B (backgrounded, still holding its own edit)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replay_keys_reopening_the_active_file_is_a_noop() {
        // Guards the same "already active" short-circuit the live `App::load_path`
        // takes: Goto-ing the file that's ALREADY active must not disturb its edit.
        let _fs = crate::testlock::serial();
        let dir = std::env::temp_dir().join(format!("awl-mb-replay-noop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "alpha\n").unwrap();
        let mut buffer = Buffer::from_file(&dir.join("a.txt"));
        let corpus = vec!["a.txt".to_string()];
        let keys = keyspec::parse_keys("X s-o a . t x t RET").unwrap();
        let res =
            replay_keys(&mut buffer, &keys, &corpus, &dir, None, &dir, &Config::empty(), None);
        assert_eq!(buffer.text(), "Xalpha\n", "the edit survives a no-op reopen of the active file");
        assert_eq!(res.buffers_open, 1, "nothing was ever backgrounded");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replay_keys_goto_recognizes_the_active_file_under_a_differently_spelled_but_equal_path() {
        // REGRESSION (code review): the SAME file reached under two different
        // (but equal-after-normalization) path spellings must resolve to the
        // SAME registry entry, or a later Goto silently re-reads it from disk
        // and discards the live edit, orphaning the first spelling's dirty
        // entry in the registry forever. This is the real report's shape (a
        // CLI file argument that stayed relative vs. the Goto picker's always
        // ROOT-JOINED, absolute spelling) reproduced with a `..`-bearing path
        // instead, so the test is deterministic and independent of the test
        // process's real cwd.
        let _fs = crate::testlock::serial();
        let dir = std::env::temp_dir().join(format!("awl-mb-relid-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "alpha\n").unwrap();
        std::fs::write(dir.join("b.txt"), "beta\n").unwrap();
        // A differently-spelled-but-identical path to a.txt: `dir/sub/../a.txt`
        // is lexically distinct from the CLEAN `dir/a.txt` the Goto picker
        // always resolves to, but names the same file.
        let messy = dir.join("sub").join("..").join("a.txt");
        let mut buffer = Buffer::from_file(&messy);
        let corpus = vec!["a.txt".to_string(), "b.txt".to_string()];
        let keys =
            keyspec::parse_keys("X s-o b . t x t RET Y s-o a . t x t RET").unwrap();
        let res =
            replay_keys(&mut buffer, &keys, &corpus, &dir, None, &dir, &Config::empty(), None);
        assert_eq!(
            buffer.text(),
            "Xalpha\n",
            "the edit to a.txt (opened via a differently-spelled but identical path) survived \
             the round trip to B and back"
        );
        assert_eq!(
            res.buffers_open, 2,
            "a.txt (active) + b.txt (backgrounded) — no orphaned duplicate entry for the messy \
             spelling"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replay_keys_new_note_parks_the_leaving_buffer_instead_of_discarding_it() {
        // REGRESSION (code review): `Effect::NewNote` used to reset `buffer` in
        // place with no park at all, so A's live edit was gone for good and a
        // later Goto back to it silently re-read a stale disk copy. `Cmd-N`
        // must park the leaving buffer through the SAME registry a Goto switch
        // uses, mirroring the live `App::new_note` (Cmd-N). (The note itself types
        // content but is never named in headless replay — no autosave engine
        // here to derive its filename — so it stays pathless and correctly
        // has NO stable identity to register; see `BufferKey::of`. Only A's
        // survival is under test.)
        let _fs = crate::testlock::serial();
        let dir = std::env::temp_dir().join(format!("awl-mb-newnote-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "alpha\n").unwrap();
        let mut buffer = Buffer::from_file(&dir.join("a.txt"));
        let corpus = vec!["a.txt".to_string()];
        // Edit A, spawn a new note (Cmd-N), type into the note, then Goto back to A.
        let keys = keyspec::parse_keys("X s-n Z s-o a . t x t RET").unwrap();
        let res =
            replay_keys(&mut buffer, &keys, &corpus, &dir, None, &dir, &Config::empty(), None);
        assert_eq!(
            buffer.text(),
            "Xalpha\n",
            "A's edit survived being left for a new note, not a fresh disk re-read"
        );
        assert_eq!(
            res.buffers_open, 1,
            "A active again; the still-unnamed note was never registered (no stable identity), \
             not lost from anywhere else A could be found"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn headless_replay_never_arms_autosave_or_stashes_scratch() {
        // The DETERMINISM LAW as a tripwire: a `--keys` replay drives edits
        // through the pure core against a bare Buffer — the autosave engine
        // lives only on the live App and is structurally out of reach. After
        // typing on a scratch buffer, neither the scratch stash nor the history
        // store may exist (a default capture stays side-effect-light).
        use std::sync::Arc;
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let mut buffer = Buffer::scratch();
            let keys = keyspec::parse_keys("h i RET t h e r e").unwrap();
            let root = PathBuf::from("/tmp");
            let _ =
                replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
            assert_eq!(buffer.text(), "hi\nthere", "the edits themselves landed");
            assert!(
                crate::fs::active()
                    .read(&crate::fs::scratch_stash_path())
                    .is_err(),
                "no scratch stash is ever written headlessly"
            );
            let hist = crate::fs::data_root().join("history");
            assert!(
                crate::fs::active()
                    .read_dir(&hist)
                    .map(|v| v.is_empty())
                    .unwrap_or(true),
                "no history log is ever written headlessly"
            );
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn headless_screenshot_never_installs_the_crash_hook() {
        // The CRASH-VISIBILITY CAPTURE GATE as the same tripwire shape:
        // `crashlog::install_hook` is called from exactly ONE door,
        // `crate::app::run`'s native branch — never reached by any headless
        // `--screenshot`/`--keys`/`--bench-*` mode, every one of which drives a
        // bare `Buffer` straight through `replay_keys` (this file's own shared
        // seam) and never constructs a live `App` or calls `crate::app::run`.
        // The witness global stays false across a whole replay.
        use std::sync::Arc;
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let mut buffer = Buffer::scratch();
            let keys = keyspec::parse_keys("h i").unwrap();
            let root = PathBuf::from("/tmp");
            let _ =
                replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
            assert!(
                !crate::crashlog::hook_installed_for_test(),
                "a headless replay must never install the panic hook"
            );
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn headless_replay_never_touches_the_session_file() {
        // The SESSION RESTORE determinism law as the same tripwire shape:
        // `session_flush`/`apply_session_restore` live only on the live App
        // (`app/session.rs`), which `replay_keys` never constructs — so a
        // `--keys` replay against a bare `Buffer` must never create
        // `session.toml`, even after edits + a save.
        use std::sync::Arc;
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let mut buffer = Buffer::scratch();
            let keys = keyspec::parse_keys("h i s-s").unwrap();
            let root = PathBuf::from("/tmp");
            let _ =
                replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
            assert!(
                crate::fs::active()
                    .read(&crate::session::session_path())
                    .is_err(),
                "no session file is ever written headlessly"
            );
        });
    }

    #[test]
    fn headless_replay_never_touches_the_recent_files_store() {
        // The RECENTLY-OPENED FILES determinism law as the same tripwire shape:
        // `push_recent_file` (and the `recent_files` load) live only on the live
        // `App` (`app/files.rs`), which `replay_keys` never constructs — so a
        // `--keys` replay against a bare `Buffer` must never create
        // `recent-files.toml`, even after edits + a save.
        use std::sync::Arc;
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let mut buffer = Buffer::scratch();
            let keys = keyspec::parse_keys("h i s-s").unwrap();
            let root = PathBuf::from("/tmp");
            let _ =
                replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
            assert!(
                crate::fs::active()
                    .read(&crate::recent_files::recent_files_path())
                    .is_err(),
                "no recent-files store is ever written headlessly"
            );
        });
    }

    #[test]
    fn headless_replay_never_touches_reduced_motion() {
        // ACCESSIBILITY TIER 1's determinism law: `motion::apply_at_startup` (the
        // ONLY function that ever consults OS/browser detection OR reads
        // `Config::reduce_motion`) lives exclusively on the live App's own
        // startup path (`App::new`), which `replay_keys` never constructs — so a
        // `--keys` replay must leave `motion::reduced()` at its default `false`
        // EVEN WHEN the passed config explicitly names `reduce_motion: true`
        // (proving the config value itself is never read here, not merely that
        // the OS call is skipped).
        let _g = crate::testlock::serial();
        let saved = crate::motion::reduced();
        crate::motion::set_reduced(false);
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("h i s-s").unwrap();
        let root = PathBuf::from("/tmp");
        let cfg = Config { reduce_motion: Some(true), ..Config::empty() };
        let _ = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &cfg, None);
        assert!(
            !crate::motion::reduced(),
            "a headless --keys replay must never apply the config's reduce_motion pref"
        );
        crate::motion::set_reduced(saved);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn headless_replay_never_touches_the_stats_file() {
        // The LIFETIME STATS determinism law as the same tripwire shape: every
        // stats tracking hook + `stats_flush` lives only on the live `App`
        // (`app/stats.rs`), which `replay_keys` never constructs — so a `--keys`
        // replay against a bare `Buffer` must never create `stats.toml`, even
        // after edits + a save. The SILENT USAGE LEDGER (`command_usage`) rides
        // the SAME `stats.toml`, recorded only in `App::apply` (never the headless
        // core), so this one tripwire covers it too — no capture can attribute a
        // command dispatch to any door.
        use std::sync::Arc;
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let mut buffer = Buffer::scratch();
            let keys = keyspec::parse_keys("h i s-s").unwrap();
            let root = PathBuf::from("/tmp");
            let _ =
                replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
            assert!(
                crate::fs::active().read(&crate::stats::stats_path()).is_err(),
                "no stats file is ever written headlessly"
            );
        });
    }

    #[test]
    fn headless_load_buffer_never_writes_back_frontmatter() {
        // The i18n round's DETERMINISM LAW as a tripwire (mirrors the autosave
        // one above): `load_buffer` is the headless capture's ONLY file-load
        // door, and the write-back-once tagger lives exclusively on the live
        // `App` (`App::new` / `App::load_path`), never here — so an untagged
        // Japanese fixture loads byte-identically, with NO frontmatter block
        // ever appearing headlessly.
        use std::sync::Arc;
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let p = PathBuf::from("/notes/japanese.md");
            let original = "これは日本語の文章です。\n";
            crate::fs::active().write(&p, original.as_bytes()).unwrap();
            let buffer = load_buffer(&Some(p));
            assert_eq!(buffer.text(), original, "no frontmatter ever appears headlessly");
        });
    }

    // ── The HISTORY TIMELINE, replay-driven (--keys drivable, sidecar-honest) ─

    /// Seed an InMemoryFs with `file` at "v2\n" and two history versions, run
    /// `body` with the store installed. The standard preview-test fixture.
    fn with_seeded_history(body: impl FnOnce(PathBuf)) {
        use std::sync::Arc;
        let p = PathBuf::from("/notes/draft.md");
        let mem = crate::fs::InMemoryFs::new().with_file(&p, "v2\n");
        crate::fs::with_fs(Arc::new(mem), || {
            crate::history::record(&p, "v1\n", &Config::empty());
            crate::history::record(&p, "v2\n", &Config::empty());
            body(p.clone());
        });
    }

    #[test]
    fn history_preview_for_resolves_selected_row() {
        // The capture-side preview resolver: the still-open History overlay's
        // highlighted row resolves to (id, content) — the version the capture
        // then shows in the document; another overlay kind resolves to None.
        with_seeded_history(|p| {
            let buffer = Buffer::from_file(&p);
            let rows = crate::history::timeline_rows(
                &p,
                &buffer.text(),
                crate::history::now_millis(),
            );
            assert_eq!(rows.len(), 2, "two seeded versions");
            let mut ov = crate::overlay::OverlayState::new_history(rows, None, None);
            let (id, content) =
                history_preview_for(&ov, &buffer).expect("the newest row resolves");
            assert_eq!(content, "v2\n");
            assert_eq!(Some(id.as_str()), ov.selected_history_id());
            // Arrow down: the OLDER version previews.
            ov.move_sel(1);
            let (_, older) = history_preview_for(&ov, &buffer).expect("row 1 resolves");
            assert_eq!(older, "v1\n", "the highlighted row IS the previewed version");
            // A non-history overlay never previews.
            let goto = crate::overlay::OverlayState::new(
                crate::overlay::OverlayKind::Goto,
                vec!["a.md".into()],
                Vec::new(),
                Vec::new(),
            );
            assert!(history_preview_for(&goto, &buffer).is_none());
        });
    }

    #[test]
    fn replay_history_esc_leaves_buffer_text_exact() {
        // The Esc-reverts-exactly proof at the replay seam: summon the timeline
        // (Cmd-S-h), arrow to the older version (C-n), Esc — the overlay is gone,
        // NOTHING was accepted, and the buffer text is byte-for-byte what it was
        // (the preview is a ViewState-level derivation; the buffer never moved).
        with_seeded_history(|p| {
            let mut buffer = Buffer::from_file(&p);
            let before = buffer.text();
            let keys = keyspec::parse_keys("Cmd-S-h C-n Esc").unwrap();
            let root = PathBuf::from("/notes");
            let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
            assert!(res.overlay.is_none(), "Esc closed the timeline");
            assert!(res.accept.is_none(), "nothing was accepted");
            assert_eq!(buffer.text(), before, "Esc leaves the buffer text exact");
        });
    }

    #[test]
    fn replay_history_enter_restores_undoably() {
        // Enter on the older row ACCEPTS its restore id; the capture arm's
        // `history::load` + `set_text` lands it as ONE undoable edit, so a
        // replayed Cmd-Z returns the buffer to its pre-restore text.
        with_seeded_history(|p| {
            let mut buffer = Buffer::from_file(&p);
            let keys = keyspec::parse_keys("Cmd-S-h C-n RET").unwrap();
            let root = PathBuf::from("/notes");
            let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
            let (kind, id) = res.accept.expect("Enter accepts the highlighted version");
            assert_eq!(kind, crate::overlay::OverlayKind::History);
            // The capture arm's restore (same shared source_path + load + set_text).
            let path = crate::history::source_path(buffer.path(), None, buffer.is_note())
                .expect("a pathed buffer keys under itself");
            let content = crate::history::load(&path, &id).expect("the id round-trips");
            buffer.set_text(&content);
            assert_eq!(buffer.text(), "v1\n", "the older version restored");
            // UNDO through the real keymap: one sealed edit, back to now.
            let undo = keyspec::parse_keys("s-z").unwrap();
            replay_keys(&mut buffer, &undo, &[], &root, None, &root, &Config::empty(), None);
            assert_eq!(buffer.text(), "v2\n", "the restore is one undoable edit");
        });
    }

    // ---- STICKY PROJECT ROOT (resolve_root precedence) -------------------

    #[test]
    fn resolve_root_explicit_flag_wins_over_sticky_and_file() {
        // --root always wins, regardless of a remembered sticky root or a file arg.
        let flag = PathBuf::from("/flag/root");
        let sticky = PathBuf::from("/sticky/root");
        let file = PathBuf::from("/some/file.txt");
        assert_eq!(
            resolve_root(&Some(flag.clone()), &Some(file), Some(&sticky)),
            flag
        );
    }

    #[test]
    fn resolve_root_bare_launch_restores_sticky_root() {
        // No --root, no file argument (the SAME condition the scratch-stash
        // restore uses): the remembered project root wins over cwd.
        let sticky = PathBuf::from("/home/me/work/repo-a");
        assert_eq!(resolve_root(&None, &None, Some(&sticky)), sticky);
    }

    #[test]
    fn resolve_root_file_argument_ignores_sticky_root() {
        // A launch WITH a file argument still resolves from that file's own
        // directory — the sticky root is never consulted, so opening some other
        // file never silently redirects the project scope.
        let sticky = PathBuf::from("/home/me/work/repo-a");
        let dir = std::env::temp_dir().join(format!("awl-resolve-root-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("note.txt");
        std::fs::write(&file, "hi").unwrap();
        assert_eq!(resolve_root(&None, &Some(file), Some(&sticky)), dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_root_absent_sticky_reproduces_todays_default() {
        // No --root, no file, no remembered sticky root: falls all the way to
        // cwd (today's byte-identical default for an absent config key).
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        assert_eq!(resolve_root(&None, &None, None), cwd);
    }

    #[test]
    fn capture_scenario_bare_launch_restores_sticky_project_root() {
        // The CAPTURE-LEVEL round trip for STICKY PROJECT RESTORE: a `--config`
        // carrying a remembered `project_root`, driving a BARE capture (no file,
        // no `--root`) through the real `capture_screenshot` seam — the sidecar
        // `project.root` must report the RESTORED project, not cwd, exactly as a
        // live bare relaunch would resolve it (`resolve_root`'s new arm above).
        // Reads the REAL disk (Project::resolve / build_index walk it) -> hold
        // the fs TEST_LOCK like the other real-fs test in this module.
        let _fs = crate::testlock::serial();
        let dir = std::env::temp_dir().join(format!("awl-sticky-root-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = Config {
            project_root: Some(dir.clone()),
            ..Config::empty()
        };
        let out = dir.join("cap.png");
        let notes_root = dir.join("notes");
        capture_screenshot(
            out.clone(),
            None, // no file argument: a bare launch
            CaptureOpts::default(),
            Vec::new(),
            None, // no explicit --root
            None,
            notes_root,
            config,
        )
        .expect("capture succeeds");
        let json = std::fs::read_to_string(out.with_extension("json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            v["project"]["root"].as_str().unwrap(),
            dir.to_string_lossy(),
            "sidecar project.root reflects the restored sticky project, not cwd"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn workspace_defaults_to_root_parent_when_unset() {
        // No `--workspace`: the effective workspace is the active root's PARENT,
        // so Cmd-Shift-P lists the root's sibling projects out of the box.
        let root = PathBuf::from("/home/me/work/repos/some-repo");
        assert_eq!(
            resolve_workspace(&None, &root),
            PathBuf::from("/home/me/work/repos")
        );
    }

    #[test]
    fn explicit_workspace_overrides_the_default() {
        // An explicit `--workspace` always wins, ignoring the root's parent.
        let root = PathBuf::from("/home/me/work/repos/some-repo");
        let ws = PathBuf::from("/elsewhere/projects");
        assert_eq!(resolve_workspace(&Some(ws.clone()), &root), ws);
    }

    #[test]
    fn workspace_falls_back_to_root_when_no_parent() {
        // A root with no usable parent (the filesystem root) falls back to the
        // root itself, so the picker still opens rather than doing nothing.
        let root = PathBuf::from("/");
        assert_eq!(resolve_workspace(&None, &root), root);
    }

    // ---- VISUAL-LINE MOVEMENT (Phase 2) ----------------------------------
    //
    // These drive the REAL keymap through `replay_keys` with a layout oracle
    // shaped at a NARROW measure, exactly as the live window / `--keys --measure`
    // CLI do, so a long line soft-wraps and the motions must follow the VISUAL
    // rows. The page globals are process-wide, so each test holds `page::test_lock()`
    // and restores the default measure. On a GPU-less host the oracle is `None`,
    // motion falls back to logical, and the test SKIPS (prints + returns).

    /// Build a narrow-measure oracle, replay `keys` through the real keymap, and
    /// return the resulting (line, col) — or `None` when no wgpu adapter exists
    /// (skip). Holds the page lock for the whole replay and restores the measure.
    fn replay_visual(text: &str, measure: usize, keys: &str) -> Option<(usize, usize)> {
        let _g = crate::testlock::serial();
        crate::page::set_page_on(true);
        crate::page::set_measure(measure);
        let mut buffer = Buffer::from_str(text);
        let opts = CaptureOpts::default();
        let out = capture::build_oracle(&buffer, &opts).map(|op| {
            let keys = keyspec::parse_keys(keys).unwrap();
            let root = PathBuf::from("/tmp");
            replay_keys(
                &mut buffer,
                &keys,
                &[],
                &root,
                None,
                &root,
                &Config::empty(),
                Some(op.as_oracle()),
            );
            buffer.cursor_line_col()
        });
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
        out
    }

    // A single long paragraph that soft-wraps into several visual rows at a narrow
    // measure, followed by a SHORT line — the wrapped + crossing fixture.
    const LONG: &str = "the quick brown fox jumps over the lazy dog today\nNEXT\n";
    const LONG_LINE0_LEN: usize = 49; // chars before the first '\n'

    #[test]
    fn visual_c_n_lands_on_next_visual_row_not_next_paragraph() {
        // (1) C-n from the start of a wrapped line steps DOWN one VISUAL row of the
        // SAME logical line — not into the next paragraph.
        let Some((line, col)) = replay_visual(LONG, 15, "C-n") else {
            eprintln!("skipping visual_c_n_lands_on_next_visual_row: no wgpu adapter");
            return;
        };
        assert_eq!(line, 0, "C-n stays on the wrapped logical line, not paragraph 2");
        assert!(col > 0, "C-n moved off col 0 onto the next visual row, got {col}");
        assert!(
            col < LONG_LINE0_LEN,
            "the landing is a wrap boundary mid-line, not the logical end ({col})"
        );
    }

    #[test]
    fn visual_c_e_stops_at_visual_row_end_not_logical_line_end() {
        // (2) C-e goes to the end of the current VISUAL row, well short of the
        // logical line's end.
        let Some((line, col)) = replay_visual(LONG, 15, "C-e") else {
            eprintln!("skipping visual_c_e_stops_at_visual_row_end: no wgpu adapter");
            return;
        };
        assert_eq!(line, 0);
        assert!(col > 0, "C-e moved to the visual row end");
        assert!(
            col < LONG_LINE0_LEN,
            "C-e stopped at the VISUAL row end ({col}), not the logical line end ({LONG_LINE0_LEN})"
        );
    }

    #[test]
    fn visual_goal_x_is_preserved_across_c_n_then_c_p() {
        // (3) The sticky GOAL-X: move right 5, then C-n then C-p returns to the
        // SAME column (the down/up round-trip lands back under the seeded goal-x).
        let down_up = replay_visual(LONG, 15, "C-f C-f C-f C-f C-f C-n C-p");
        let just_right = replay_visual(LONG, 15, "C-f C-f C-f C-f C-f");
        let (Some(down_up), Some(just_right)) = (down_up, just_right) else {
            eprintln!("skipping visual_goal_x_preserved: no wgpu adapter");
            return;
        };
        assert_eq!(just_right, (0, 5), "five C-f land at col 5");
        assert_eq!(
            down_up, just_right,
            "C-n then C-p returns to the starting column via the sticky goal-x"
        );
    }

    #[test]
    fn visual_c_a_goes_to_visual_row_start() {
        // (4) C-a goes to the start of the current VISUAL row. C-n from col 0 lands
        // on the next visual row's start S; from mid that row, C-a returns to S.
        let start = replay_visual(LONG, 15, "C-n");
        let from_mid = replay_visual(LONG, 15, "C-n C-f C-f C-a");
        let (Some(start), Some(from_mid)) = (start, from_mid) else {
            eprintln!("skipping visual_c_a_goes_to_visual_row_start: no wgpu adapter");
            return;
        };
        assert_eq!(start.0, 0);
        assert!(start.1 > 0, "C-n reached a wrapped row start > 0");
        assert_eq!(
            from_mid, start,
            "C-a snaps back to the VISUAL row start, not the logical line start (col 0)"
        );
    }

    #[test]
    fn visual_c_n_at_last_visual_row_crosses_to_next_logical_line() {
        // (5) At the LAST visual row of a wrapped line, C-n crosses into the NEXT
        // logical line's FIRST visual row. Count line-0's visual rows via the
        // oracle, then drive that many C-n through the real keymap.
        let _g = crate::testlock::serial();
        crate::page::set_page_on(true);
        crate::page::set_measure(15);
        let probe = Buffer::from_str(LONG);
        let opts = CaptureOpts::default();
        let result = capture::build_oracle(&probe, &opts).map(|op| {
            // Step DOWN from (0,0) with goal-x 0 until the logical line changes;
            // `steps` C-n's cross into line 1, `steps-1` stay on line 0.
            let mut steps = 0usize;
            {
                let oracle = op.as_oracle();
                let (mut l, mut c) = (0usize, 0usize);
                loop {
                    let (nl, nc) = oracle.visual_line_down(l, c, 0.0);
                    steps += 1;
                    if nl != 0 {
                        break;
                    }
                    assert!(steps < 100, "line 0 never ended");
                    l = nl;
                    c = nc;
                }
            }
            assert!(steps >= 2, "line 0 should wrap into multiple visual rows");
            let root = PathBuf::from("/tmp");
            // One fewer C-n keeps us on line 0's LAST visual row...
            let mut b0 = Buffer::from_str(LONG);
            let keys_stay = keyspec::parse_keys(&"C-n ".repeat(steps - 1)).unwrap();
            replay_keys(&mut b0, &keys_stay, &[], &root, None, &root, &Config::empty(), Some(op.as_oracle()));
            let stay = b0.cursor_line_col();
            // ...and the full count crosses into line 1's first visual row.
            let mut b1 = Buffer::from_str(LONG);
            let keys_cross = keyspec::parse_keys(&"C-n ".repeat(steps)).unwrap();
            replay_keys(&mut b1, &keys_cross, &[], &root, None, &root, &Config::empty(), Some(op.as_oracle()));
            (stay, b1.cursor_line_col())
        });
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
        let Some((stay, cross)) = result else {
            eprintln!("skipping visual_c_n_crosses_to_next_logical_line: no wgpu adapter");
            return;
        };
        assert_eq!(stay.0, 0, "one C-n short keeps us on line 0's last visual row");
        assert_eq!(cross.0, 1, "the last-row C-n crosses into the next logical line");
        // Line 1 ("NEXT") fits one visual row, so its first row starts at col 0.
        assert_eq!(cross.1, 0, "we land on line 1's FIRST visual row");
    }

    #[test]
    fn regression_non_wrapped_doc_visual_equals_logical_byte_identical() {
        // REGRESSION GUARD: on a NON-wrapped document (every logical line fits in
        // one visual row) visual motion == logical motion. Identical-content lines
        // make the vertical goal-x round-trip exact even on a proportional font.
        // Replay the SAME keys with the oracle (visual) and without it (logical);
        // the resulting cursors — and the rendered PNGs — must be IDENTICAL.
        let _g = crate::testlock::serial();
        crate::page::set_page_on(true);
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
        let text = "hello world foo\nhello world foo\nhello world foo\n";
        let keys = keyspec::parse_keys("C-f C-f C-f C-f C-f C-n C-n C-e C-a C-p C-k").unwrap();
        let root = PathBuf::from("/tmp");
        let opts = CaptureOpts::default();

        let mut logical = Buffer::from_str(text);
        replay_keys(&mut logical, &keys, &[], &root, None, &root, &Config::empty(), None);

        let mut visual = Buffer::from_str(text);
        let Some(op) = capture::build_oracle(&visual, &opts) else {
            crate::page::set_measure(crate::page::DEFAULT_MEASURE);
            eprintln!("skipping regression_non_wrapped byte-identical: no wgpu adapter");
            return;
        };
        replay_keys(&mut visual, &keys, &[], &root, None, &root, &Config::empty(), Some(op.as_oracle()));

        assert_eq!(
            visual.cursor_line_col(),
            logical.cursor_line_col(),
            "non-wrapped: visual motion must equal logical motion"
        );

        // Byte-identical captures: render both buffers and diff the PNG bytes.
        let dir = std::env::temp_dir();
        let pv = dir.join("awl_vl_visual.png");
        let pl = dir.join("awl_vl_logical.png");
        capture::capture_with(&pv, &visual, &opts).expect("render visual");
        capture::capture_with(&pl, &logical, &opts).expect("render logical");
        let bv = std::fs::read(&pv).expect("read visual png");
        let bl = std::fs::read(&pl).expect("read logical png");
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
        assert_eq!(
            bv, bl,
            "non-wrapped short-line doc: visual + logical captures are byte-identical"
        );
    }
}
