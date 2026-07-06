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
/// ROOT (`config.project_root`, written by every switch-project / C-x p commit)
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
/// (C-x p) candidates: an explicit `--workspace` wins; otherwise DEFAULT to the
/// PARENT of the active project `root`, so switch-project lists the root's
/// SIBLING projects out of the box — launched inside `~/work/repos/some-repo`,
/// the workspace defaults to `~/work/repos`, so C-x p shows all the repos. A
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
/// site in [`replay_keys`] (a Goto switch, `C-x n`) backgrounds it identically
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
/// children — so a replayed `C-x C-f` / `C-x p` / `C-x j` summons a real overlay
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
        while let Some(action) = current.take() {
        // OUTLINE picker corpus: the current buffer's markdown headings (title
        // indented by depth, paired with its line). Read before the builder; a
        // non-markdown buffer / no headings yields an empty list (no-op summon).
        let outline_headings: Vec<(String, usize)> = if buffer.is_markdown() {
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
        // The non-navigable builder inputs. Headless leaves the GO-TO recency tiers +
        // labels EMPTY (no mtime read, no open/recent history) so the capture stays
        // byte-stable; the buffer-scoped outline / spell / history come from the
        // replayed state + the store.
        let build_ctx = crate::overlay::BuildCtx {
            goto_corpus: corpus.to_vec(),
            goto_open: Vec::new(),
            goto_recent: Vec::new(),
            goto_times: Vec::new(),
            config_keys: &config.keys,
            outline_headings,
            spell_target,
            history_entries,
        };
        let mut make_overlay =
            |kind: crate::overlay::OverlayKind| crate::overlay::build(kind, &build_ctx);
        let mut browse_to = |kind: crate::overlay::OverlayKind, rel: Option<String>| {
            // Shared one-level builder: Project navigates the workspace by absolute
            // path, MoveDest walks the NOTES root (folders only), Browse the active
            // root (files + folders).
            crate::overlay::browse_level(kind, rel, root, notes_root, workspace)
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
        // Carry out the ONE deferred effect the core signalled (mutually exclusive,
        // so a single match suffices). Quit / LastBuffer are no-ops in capture (no
        // event loop, no 2-deep history); the rest mirror the live App's handling.
        match effect {
            // C-x n: PARK the buffer being left (exactly like the live
            // `App::new_note` — a code-review-caught gap used to skip this and
            // just reset `buffer` in place, silently discarding it) then reset
            // to a fresh quick note bound to the notes root, so subsequent
            // typed chars build the title and an explicit `C-x C-s` derives
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
            // COMMAND PALETTE run-on-Enter: feed the chosen command back through the
            // core (the palette already closed), so e.g. "Go to file" opens the goto
            // overlay as the final captured state.
            actions::Effect::RunAction(a) => current = Some(a),
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
            // FinishBuffer (C-x #): the core already ran the SAME `buffer.save()` a
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
            | actions::Effect::FinishBuffer
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
            // `C-x C-f` in the key-spec summons a real, scoped go-to overlay.
            let active_root = resolve_root(&root, &file, config.project_root.as_deref());
            let proj = crate::project::Project::resolve(&active_root);
            let corpus = crate::index::build_index(&active_root);
            // Default the switch-project workspace to the active root's PARENT when
            // neither `--workspace` nor a config `workspace` was given, so the sidecar
            // reports an EFFECTIVE folder (and a replayed C-x p lists siblings).
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
            });

            let mut buffer = load_buffer(&file);
            // Replay `--keys` FIRST so the cursor/selection/search the spec
            // produces are what the capture reflects. Fold the App-level state
            // (zoom / selection / search) the replay produced into the capture
            // opts — but never clobber an explicit verification hook.
            // Default the switch-project workspace to the active root's PARENT
            // when no explicit `--workspace` was given, so a replayed `C-x p`
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
                        });
                    }
                    // Outline: jump the cursor to the accepted heading LINE so the
                    // capture's `cursor` block reflects the jump (agent-verifiable).
                    crate::overlay::OverlayKind::Outline => {
                        if let Ok(line) = val.parse::<usize>() {
                            let idx = buffer.line_col_to_char(line, 0);
                            buffer.set_cursor(idx);
                        }
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
                    bindings: ov.item_bindings(),
                    selected_index: ov.selected,
                    hint: ov.foot_hint(),
                    browse_dir: ov.browse_dir.clone(),
                    spell_target: ov.spell_target,
                    preview_id: preview.map(|(id, _)| id),
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
                    // THEME PICKER: the active lens + strip + per-row section labels so
                    // a `--keys "C-x t <right>"` capture renders + reports the faceted view.
                    lens: if ov.kind == crate::overlay::OverlayKind::Theme {
                        Some(ov.theme_lens.as_str())
                    } else {
                        None
                    },
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
    use crate::keyspec;

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
    fn replay_keys_page_reset_restores_default_measure() {
        // The "no easy way back" fix: mirror `--measure 40` (the flag writes the
        // process-global directly, exactly like this), then replay the "Reset Page
        // Width" Action (no default chord — palette/double-click only, so it's
        // constructed directly here rather than parsed from a `--keys` chord,
        // matching how `replay_keys` already takes a resolved `Action` stream). The
        // sidecar's `page.measure` field reads this SAME global, so this is the
        // capture-level half of the reset (the config-file override removal is
        // App-only + unit-tested separately in `config.rs`). Holds the process-wide
        // page TEST_LOCK and restores it after, like every other page-global test.
        let _pg = crate::page::test_lock();
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
        let _pg = crate::page::test_lock();
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
        let _fs = crate::fs::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _pg = crate::page::test_lock();
        let measure0 = crate::page::measure();
        let dir = std::env::temp_dir().join(format!("awl-mb-measure-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.md"), "# hello\n").unwrap();
        std::fs::write(dir.join("b.rs"), "fn main() {}\n").unwrap();
        let cfg = Config { page_width_prose: Some(55), page_width_code: Some(120), ..Config::empty() };
        let mut buffer = Buffer::scratch();
        let corpus = vec!["a.md".to_string(), "b.rs".to_string()];
        crate::page::set_measure(1); // deliberately wrong, so the switch below can't coincide

        let keys_to_b = keyspec::parse_keys("C-x C-f b . r s RET").unwrap();
        let _ = replay_keys(&mut buffer, &keys_to_b, &corpus, &dir, None, &dir, &cfg, None);
        assert_eq!(crate::page::measure(), 120, "b.rs (code) picks up the configured code measure");

        let keys_to_a = keyspec::parse_keys("C-x C-f a . m d RET").unwrap();
        let _ = replay_keys(&mut buffer, &keys_to_a, &corpus, &dir, None, &dir, &cfg, None);
        assert_eq!(crate::page::measure(), 55, "back to a.md (prose) picks up the configured prose measure");

        crate::page::set_measure(measure0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replay_scrolled_deep_then_open_swaps_to_the_short_file() {
        // The SCROLLED-DEEP-THEN-OPEN replay (the open-then-blank-screen hunt): park
        // the cursor at the END of a long document (M-> — cursor-follow scroll then
        // sits far past a one-line file's end), summon the Goto picker (C-x C-f),
        // filter to the short file, and accept with Enter. The replay must surface
        // the ACCEPT; the RunCapture arm's swap (mirrored here) yields the SHORT
        // file's buffer with the cursor at (0,0) — the capture re-derives its follow
        // scroll from THAT cursor, so the frame can never render past the new
        // document's EOF. Locks the headless half of the hunt (the live half is the
        // App view-text cache across a swap, tested in `app::tests`).
        // Reads the REAL disk through the fs seam → hold the fs TEST_LOCK so a
        // parallel InMemoryFs installation can't swallow the temp files.
        let _fs = crate::fs::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("awl-goto-swap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let long: String = (0..300).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.join("long.txt"), &long).unwrap();
        std::fs::write(dir.join("short.txt"), "just one line\n").unwrap();
        let mut buffer = Buffer::from_file(&dir.join("long.txt"));
        let keys = keyspec::parse_keys("M-> C-x C-f s h o r t RET").unwrap();
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
        let _fs = crate::fs::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("awl-mb-replay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "alpha\n").unwrap();
        std::fs::write(dir.join("b.txt"), "beta\n").unwrap();
        let mut buffer = Buffer::scratch();
        let corpus = vec!["a.txt".to_string(), "b.txt".to_string()];
        let keys = keyspec::parse_keys(
            "C-x C-f a . t x t RET X C-x C-f b . t x t RET Y C-x C-f a . t x t RET",
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
        let _fs = crate::fs::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("awl-mb-replay-noop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "alpha\n").unwrap();
        let mut buffer = Buffer::from_file(&dir.join("a.txt"));
        let corpus = vec!["a.txt".to_string()];
        let keys = keyspec::parse_keys("X C-x C-f a . t x t RET").unwrap();
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
        let _fs = crate::fs::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
            keyspec::parse_keys("X C-x C-f b . t x t RET Y C-x C-f a . t x t RET").unwrap();
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
        // later Goto back to it silently re-read a stale disk copy. `C-x n`
        // must park the leaving buffer through the SAME registry a Goto switch
        // uses, mirroring the live `App::new_note`. (The note itself types
        // content but is never named in headless replay — no autosave engine
        // here to derive its filename — so it stays pathless and correctly
        // has NO stable identity to register; see `BufferKey::of`. Only A's
        // survival is under test.)
        let _fs = crate::fs::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("awl-mb-newnote-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "alpha\n").unwrap();
        let mut buffer = Buffer::from_file(&dir.join("a.txt"));
        let corpus = vec!["a.txt".to_string()];
        // Edit A, spawn a new note (C-x n), type into the note, then Goto back to A.
        let keys = keyspec::parse_keys("X C-x n Z C-x C-f a . t x t RET").unwrap();
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
            let keys = keyspec::parse_keys("h i C-x C-s").unwrap();
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
            let mut ov = crate::overlay::OverlayState::new_history(rows);
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
        let _fs = crate::fs::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        // so C-x p lists the root's sibling projects out of the box.
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
        let _g = crate::page::test_lock();
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
        let _g = crate::page::test_lock();
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
        let _g = crate::page::test_lock();
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
