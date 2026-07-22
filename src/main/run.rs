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
pub(crate) fn load_buffer(file: &Option<PathBuf>) -> Buffer {
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
pub(crate) fn resolve_root(
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
    /// Whether the replay left the search panel in REPLACE mode (Cmd-R / Tab /
    /// Cmd-Option-F — all drivable through the shared search-key seam).
    replace_active: bool,
    /// The replacement field text, typed through the SAME interception seam the
    /// live panel uses (the old "isearch-input gap" — where this could only
    /// ever be empty — is retired).
    replacement: String,
    /// Whether typing currently edits the REPLACEMENT field (vs. the query) —
    /// folded into the sidecar's `search.editing_replacement` so a replayed
    /// Tab/Cmd-R focus move is assertable.
    editing_replacement: bool,
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
    /// STRICT REPLAY TRUTHFULNESS: every INTERCEPTED external handoff the
    /// replay observed + recorded without performing (URL open, mailto, Trash,
    /// download — see [`crate::replay::classify`]), in replay order. Recorded
    /// under BOTH modes; the phase-5 scenario trace consumes this seam (no
    /// consumer beyond the tests yet, hence the allow).
    #[allow(dead_code)]
    intercepts: Vec<crate::replay::Intercept>,
    /// The permissive-mode warning lines (exactly what was printed to stderr,
    /// one per Unsupported/Intercepted crossing — [`crate::replay::warn_line`]),
    /// so the warnings themselves are testable. Empty under strict mode (it
    /// aborts on Unsupported and records Intercepted silently) and for any
    /// replay that stays on the Applied path.
    #[allow(dead_code)]
    warnings: Vec<String>,
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

/// Replay a parsed `--keys` CHORD stream against `buffer` — each chord either
/// consumed by the SEARCH GUARD (the shared `crate::search::keys::intercept`
/// seam, while the isearch panel is open) or resolved through `km` and applied
/// THROUGH the shared `actions::apply_core` seam, so headless replay is
/// byte-for-byte identical to live editing. `corpus` is the active project's
/// file index (Goto), `root` scopes the Browse navigator, and `workspace`
/// supplies the switch-project children — so a replayed Cmd-O / Cmd-Shift-P /
/// Browse summons a real overlay the rest of the key-spec can filter / move /
/// descend / accept. Returns the post-replay App-level state.
fn replay_keys(
    buffer: &mut Buffer,
    keys: &[crate::keyspec::Chord],
    corpus: &[String],
    root: &std::path::Path,
    workspace: Option<&std::path::Path>,
    notes_root: &std::path::Path,
    config: &Config,
    oracle: Option<&mut capture::OraclePipeline>,
    km: &mut crate::keymap::KeymapState,
) -> ReplayResult {
    match replay_keys_mode(
        crate::replay::Mode::Permissive,
        buffer,
        keys,
        corpus,
        root,
        workspace,
        notes_root,
        config,
        oracle,
        km,
    ) {
        Ok(res) => res,
        // Every abort in the loop is Strict-gated, so the permissive door is
        // structurally infallible — pinned by
        // `permissive_replay_never_aborts_and_warns_on_both_non_applied_seams`.
        Err(e) => unreachable!("permissive replay never aborts: {e}"),
    }
}

/// The mode-aware core both doors share — a thin loop over [`ReplaySession`]
/// (construct, apply every chord, finish). PERMISSIVE never errors (it warns on
/// stderr + records); STRICT returns the exact offender the moment an
/// Unsupported effect fires ([`crate::replay::strict_error`]) — the scenario
/// runner's truthfulness contract (see [`crate::replay`]'s module doc).
fn replay_keys_mode(
    mode: crate::replay::Mode,
    buffer: &mut Buffer,
    keys: &[crate::keyspec::Chord],
    corpus: &[String],
    root: &std::path::Path,
    workspace: Option<&std::path::Path>,
    notes_root: &std::path::Path,
    config: &Config,
    oracle: Option<&mut capture::OraclePipeline>,
    km: &mut crate::keymap::KeymapState,
) -> Result<ReplayResult> {
    let mut session =
        ReplaySession::new(mode, buffer, corpus, root, workspace, notes_root, config, oracle, km);
    for chord in keys {
        session.apply_chord(chord)?;
    }
    Ok(session.finish())
}

/// ONE headless replay in progress: the whole `--keys` engine's state (search
/// guard, overlay, multi-buffer registry, zoom, chord resolver) held as a
/// STRUCT so a caller can interleave key application with other work — the
/// storyboard runner (`crate::story`) applies a step's chords, then renders a
/// film frame from the CURRENT state, then applies the next step's. The one-shot
/// doors ([`replay_keys`] / [`replay_keys_mode`]) are thin loops over this same
/// session, so a storyboard step and a `--keys` replay can never disagree on
/// what a chord does.
pub(crate) struct ReplaySession<'a> {
    mode: crate::replay::Mode,
    buffer: &'a mut Buffer,
    corpus: &'a [String],
    root: &'a std::path::Path,
    workspace: Option<&'a std::path::Path>,
    notes_root: &'a std::path::Path,
    config: &'a Config,
    // The visual-line motion LAYOUT ORACLE (an offscreen-shaped pipeline), so the
    // headless replay sees the SAME wrap geometry the live window does. Held
    // MUTABLY because the loop RE-SHAPES it from the current buffer / zoom /
    // page-measure state before EVERY action (`OraclePipeline::refresh` — the one
    // freshness seam), mirroring the live window's between-keystrokes re-sync so
    // an edit / zoom / Goto switch can never leave a later motion on stale wrap
    // geometry. `None` in the unit tests / GPU-less paths, where motion falls
    // back to LOGICAL lines.
    oracle: Option<&'a mut capture::OraclePipeline>,
    // The chord→action resolver: ONE persistent keymap across the stream (prefix
    // sequences + rebinds compose exactly as live), with the STRICT
    // unbound/dangling-prefix refusals armed under `Mode::Strict` — resolution
    // is interleaved with the search guard exactly like live key dispatch, and
    // the underlying keymap is borrowed `&mut` so the `C-x` prefix state
    // survives a caller that replays a split spec in two calls (the timeline's
    // origin/glide halves).
    resolver: crate::keyspec::ChordResolver<'a>,
    // The spell engine for the Cmd-`;` picker, loaded once (None if the dictionary
    // failed to parse — the summon then no-ops, like the live path with no checker).
    spell: Option<crate::spell::SpellChecker>,
    intercepts: Vec<crate::replay::Intercept>,
    warnings: Vec<String>,
    /// The storyboard trace's per-chord record ([`crate::storyboard::ChordTrace`]):
    /// what each chord resolved to and how its effect was classified. Recorded
    /// under both modes (cheap; replay is never hot), drained per-step by the
    /// storyboard runner via [`Self::drain_records`].
    records: Vec<crate::storyboard::ChordTrace>,
    shift_selecting: bool,
    zoom: f32,
    search: Option<crate::search::SearchState>,
    overlay: Option<crate::overlay::OverlayState>,
    accept: Option<(crate::overlay::OverlayKind, String)>,
    // MULTI-BUFFER REGISTRY: the same `crate::buffers::BufferRegistry` the live
    // App uses, so a `--keys` spec that Goes-to file A, edits, Goes-to file B,
    // edits, then Goes back to A sees A's PRESERVED cursor/edits/undo — the
    // v1 multi-buffer win, headlessly drivable. Carries no extra payload
    // (`()`): headless replay tracks nothing per-buffer beyond the `Buffer`
    // itself (no scroll/spell/autosave state to preserve here).
    registry: crate::buffers::BufferRegistry<()>,
}

impl<'a> ReplaySession<'a> {
    #[allow(clippy::too_many_arguments)] // mirrors replay_keys_mode's own surface
    pub(crate) fn new(
        mode: crate::replay::Mode,
        buffer: &'a mut Buffer,
        corpus: &'a [String],
        root: &'a std::path::Path,
        workspace: Option<&'a std::path::Path>,
        notes_root: &'a std::path::Path,
        config: &'a Config,
        oracle: Option<&'a mut capture::OraclePipeline>,
        km: &'a mut crate::keymap::KeymapState,
    ) -> Self {
        let resolver =
            crate::keyspec::ChordResolver::new(km, mode == crate::replay::Mode::Strict);
        Self {
            mode,
            buffer,
            corpus,
            root,
            workspace,
            notes_root,
            config,
            oracle,
            resolver,
            spell: crate::spell::SpellChecker::new(crate::spell::active_variant()).ok(),
            intercepts: Vec::new(),
            warnings: Vec::new(),
            records: Vec::new(),
            shift_selecting: false,
            zoom: 1.0,
            search: None,
            overlay: None,
            accept: None,
            registry: crate::buffers::BufferRegistry::default(),
        }
    }

    /// Apply ONE chord — the live `App::on_keyboard_input` order: search guard
    /// first, then keymap resolution, then `apply_core` + the deferred-effect
    /// arms. `Err` only under STRICT (an unbound chord, a dangling prefix, or
    /// an Unsupported effect, each naming the offender).
    pub(crate) fn apply_chord(&mut self, chord: &crate::keyspec::Chord) -> Result<()> {
        // SEARCH GUARD — the live `App::on_keyboard_input` guard's exact position,
        // now the exact same code: while the isearch panel is open EVERY chord is
        // consumed by the ONE interception seam (`crate::search::keys::intercept`)
        // and never reaches the keymap — query/replacement typing, Backspace,
        // C-s/C-r/arrow steps, M-c case toggle, Tab/Cmd-R field moves, Enter
        // accept / replace-one, Cmd-Enter replace-all, Esc/C-g abort. The returned
        // recoil is a LIVE-only caret flourish, dropped here exactly like
        // `Effect::Recoil` (no clock, settled frame unchanged). Strict never
        // judges a consumed chord "unbound" — the panel owning it IS its binding.
        if self.search.is_some() {
            let _ = crate::search::keys::intercept(
                &mut self.search,
                self.buffer,
                &chord.key,
                chord.mods.state(),
            );
            self.records.push(crate::storyboard::ChordTrace {
                chord: chord.spec.clone(),
                action: None,
                effect: "search_input".to_string(),
                class: "applied",
                detail: String::new(),
            });
            return Ok(());
        }
        // Resolve THIS chord through the persistent keymap; a dropped
        // `Ignore`/`BeginPrefix` yields no action, and a strict refusal aborts
        // naming the offending chord.
        let Some(resolved) = self.resolver.resolve(chord)? else {
            self.records.push(crate::storyboard::ChordTrace {
                chord: chord.spec.clone(),
                action: None,
                effect: "prefix".to_string(),
                class: "applied",
                detail: String::new(),
            });
            return Ok(());
        };
        // SHIFT = SELECT-INTENT, the live dispatch's exact derivation
        // (`app/input/keys.rs::on_keyboard_input`): the chord's `S-` modifier
        // extends a selection across a motion, routed through the ONE owner
        // `crate::app::motion_honors_shift_select` — keyed on the pressed chord's
        // KEY, not the Action alone, so `M-<` / `M->` (a `Key::Character` whose
        // Shift is incidental to typing the glyph) stay pure motion while
        // `S-s-Up`/`S-s-Down` and `S-C-Home`/`S-C-End` (named nav keys reaching
        // the same actions) extend, exactly like live. Derived ONCE per pressed
        // chord from the FIRST resolved action and carried into a palette-chained
        // re-dispatch unchanged — mirroring the live `Effect::RunAction` arm,
        // which re-applies with the same `shift` bool. (This retired the old
        // "replay is unshifted" hole: `--keys "S-Right"` silently ran the motion
        // unshifted and left `selection: null`.)
        let shift = chord.mods.state().contains(winit::keyboard::ModifiersState::SHIFT)
            && crate::app::motion_honors_shift_select(&resolved, &chord.key);
        // A tiny worklist so the COMMAND PALETTE's run-on-Enter chains: Enter on a
        // command writes `run_action`, which we then feed back through the core
        // (slot now empty) so an overlay-opening command opens its sub-overlay as
        // the final captured state. At most one chained action, so this drains in
        // one extra pass.
        let mut current: Option<Action> = Some(resolved);
        // BREADCRUMB: when the palette's Enter re-dispatches a `RunAction` (below),
        // stamp `return_to = Command` onto whatever overlay that command opens, so a
        // later POP (Esc / value-pick) re-summons the palette — mirroring the live
        // `App::apply` seam. Set by the `RunAction` arm, consumed right after the
        // re-dispatched action's `apply_core` opens its overlay.
        let mut pending_return_to: Option<crate::overlay::OverlayKind> = None;
        while let Some(action) = current.take() {
        // FRESH LAYOUT ORACLE PER ACTION: re-shape the oracle from the CURRENT
        // buffer / zoom / page-measure state BEFORE the action consults it —
        // the live window's pipeline re-syncs between keystrokes, so the
        // headless twin must too, or an edit that re-wraps a line (or a zoom
        // change, or the Goto arm's buffer + measure switch below) leaves the
        // NEXT motion reading stale wrap geometry. One seam, unconditional by
        // design (both underlying calls no-op cheaply when nothing changed).
        if let Some(op) = self.oracle.as_deref_mut() {
            op.refresh(self.buffer, self.zoom);
        }
        // GO-TO's HEADINGS lens corpus: the current buffer's markdown headings (title
        // indented by depth, paired with its line) — the fold that retired the
        // standalone Outline picker. Gathered ONLY when a Go-to door fires (Cmd-O /
        // "Go to heading…"), matching the live app; a non-markdown buffer / no headings
        // yields an empty list (the Headings lens then reads empty).
        let goto_headings: Vec<(String, usize)> =
            if matches!(action, Action::OpenGoto | Action::OpenOutline) && self.buffer.is_markdown() {
                crate::markdown::headings(&self.buffer.text())
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
                self.spell.as_ref().and_then(|sc| {
                    let (line, col) = self.buffer.cursor_line_col();
                    sc.suggest_at(&self.buffer.text(), line, col, self.buffer.syntax_lang()).map(|t| {
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
            if matches!(action, Action::OpenHistory | Action::CompareVersion) {
                match crate::history::source_path(self.buffer.path(), None, self.buffer.is_note()) {
                    Some(path) => crate::history::timeline_rows(
                        &path,
                        &self.buffer.text(),
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
            crate::assets::scan(self.root, self.corpus)
        } else {
            Vec::new()
        };
        // The non-navigable builder inputs. Headless leaves the GO-TO recency tiers +
        // labels EMPTY (no mtime read, no open/recent history) so the capture stays
        // byte-stable; the buffer-scoped headings / spell / history come from the
        // replayed state + the store.
        let effective_keep = self.config.effective_linux_keep();
        let build_ctx = crate::overlay::BuildCtx {
            goto_corpus: self.corpus.to_vec(),
            goto_open: Vec::new(),
            goto_recent: Vec::new(),
            goto_times: Vec::new(),
            config_keys: &self.config.keys,
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
            // `today_ymd` is the FIXED headless placeholder (no clock in a capture —
            // see `dateformat::CAPTURE_PLACEHOLDER_YMD`'s doc), so the "Date format"
            // row's preview is byte-stable.
            settings_values: crate::settings::SettingsValues::gather(
                self.config,
                self.root,
                self.zoom,
                crate::dateformat::CAPTURE_PLACEHOLDER_YMD,
            ),
            assets,
        };
        let mut make_overlay =
            |kind: crate::overlay::OverlayKind| crate::overlay::build(kind, &build_ctx);
        let (root, notes_root, workspace) = (self.root, self.notes_root, self.workspace);
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
            buffer: &mut *self.buffer,
            shift_selecting: &mut self.shift_selecting,
            zoom: &mut self.zoom,
            search: &mut self.search,
            // Headless has no viewport to measure; a page is a fixed,
            // deterministic chunk of logical lines.
            scroll_page_lines: 20,
            overlay: &mut self.overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
            oracle: self.oracle.as_deref().map(|op| op.as_oracle()),
        };
        let effect = actions::apply_core(&mut ctx, &action, shift);
        drop(ctx);
        // STRICT REPLAY TRUTHFULNESS: consult the ONE classification
        // (`crate::replay::classify`, a no-wildcard match over `Effect`) BEFORE
        // the apply arms below run. Strict ABORTS on an Unsupported effect,
        // naming the exact action + effect; permissive keeps the legacy
        // behavior byte-identically but WARNS on stderr (and records the same
        // line) when it crosses an Unsupported or Intercepted seam. Every
        // Intercepted handoff is recorded under BOTH modes — the phase-5
        // trace seam.
        let classified = crate::replay::classify(&effect);
        // TRACE RECORD: one entry per applied action (a palette Enter's chained
        // re-dispatch records twice under the same chord) — the storyboard
        // trace's raw material, including an Unsupported offender (recorded
        // BEFORE the strict abort below, so the trace names it too).
        self.records.push(crate::storyboard::ChordTrace {
            chord: chord.spec.clone(),
            action: Some(format!("{action:?}")),
            effect: classified.name.to_string(),
            class: match &classified.class {
                crate::replay::EffectClass::Applied => "applied",
                crate::replay::EffectClass::Intercepted { .. } => "intercepted",
                crate::replay::EffectClass::Unsupported { .. } => "unsupported",
            },
            detail: match &classified.class {
                crate::replay::EffectClass::Intercepted { detail } => detail.clone(),
                _ => String::new(),
            },
        });
        // An intercepted handoff is RECORDED under both modes (the trace seam).
        if let crate::replay::EffectClass::Intercepted { detail } = &classified.class {
            self.intercepts.push(crate::replay::Intercept {
                effect: classified.name,
                detail: detail.clone(),
            });
        }
        // Strict refuses an Unsupported effect outright, naming the offender.
        if self.mode == crate::replay::Mode::Strict {
            if let crate::replay::EffectClass::Unsupported { .. } = classified.class {
                return Err(crate::replay::strict_error(&action, &classified));
            }
        }
        // Permissive warns on either non-Applied crossing (`warn_line` is
        // `None` for Applied) — the ONE stderr seam, mirrored into `warnings`
        // so the exact printed line is testable.
        if self.mode == crate::replay::Mode::Permissive {
            if let Some(w) = crate::replay::warn_line(&action, &classified) {
                eprintln!("{w}");
                self.warnings.push(w);
            }
        }
        // BREADCRUMB: stamp the overlay this action just opened (if any) with the
        // palette parent a preceding `RunAction` re-dispatch set — a no-op unless the
        // previous iteration was a palette Enter (`pending_return_to` still None here
        // for a direct summon).
        crate::actions::stamp_return_to(&mut self.overlay, pending_return_to.take());
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
                park_active(self.buffer, &mut self.registry);
                self.buffer.start_note(self.notes_root.to_path_buf());
            }
            // Settings: load the config file into the buffer (creating the commented
            // default first if missing), so the capture reflects the config CONTENTS
            // — exactly what the live Settings command does. Opens the EFFECTIVE
            // config path (the `--config` target when one was given).
            actions::Effect::OpenSettings => {
                if !self.config.path.as_os_str().is_empty() {
                    if !crate::fs::active().exists(&self.config.path) {
                        let _ = Config::write_default(&self.config.path);
                    }
                    *self.buffer = Buffer::from_file(&self.config.path);
                }
            }
            // Credits: load the embedded CREDITS.md text directly into the buffer
            // — no filesystem write at all (the headless capture path stays
            // side-effect-light, mirroring OpenSettings' spirit without needing a
            // disk round trip, since the text is compiled in rather than
            // user-owned). No park needed here either: `replay_keys` never stashes
            // scratch (structurally autosave-free), so there is nothing to protect.
            actions::Effect::OpenCredits => {
                *self.buffer = Buffer::from_str(crate::credits::CREDITS_MD);
            }
            // Guide: load the embedded GUIDE.md text directly into the buffer —
            // mirrors OpenCredits exactly, same side-effect-light reasoning.
            // Rendered through `guide::render` (chord-token substitution) for
            // the headless replay's own convention/platform, exactly like the
            // live `App::open_guide` door.
            actions::Effect::OpenGuide => {
                *self.buffer = Buffer::from_str(&crate::guide::render(
                    crate::convention::Convention::current(),
                    crate::commands::Platform::current(),
                ));
            }
            // INSERT DATE: the SAME insert `App::insert_date` performs live, against
            // the FIXED placeholder date instead of the real clock (the determinism
            // gate — see `dateformat::CAPTURE_PLACEHOLDER_YMD`'s doc), reading the
            // SAME active-format process-global (seeded from `--config`/defaults by
            // `apply_sticky_globals`, exactly like `caret_mode`/`dictionary`).
            actions::Effect::InsertDate => {
                let (y, m, d) = crate::dateformat::CAPTURE_PLACEHOLDER_YMD;
                let text = crate::dateformat::active_format().format(y, m, d);
                self.buffer.insert_text(&text);
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
                    let path = crate::index::resolve(self.root, &val);
                    // Compared via the normalized registry identity, not raw
                    // path equality — mirrors `App::load_path`'s "already
                    // active" check (see `BufferKey::path`'s doc: a launch
                    // file argument that stayed relative and this ALWAYS
                    // root-joined Goto path must be recognized as the same
                    // file, or the switch below re-reads it fresh from disk
                    // and orphans the relative spelling's live edit).
                    let new_key = crate::buffers::BufferKey::path(&path);
                    if crate::buffers::BufferKey::of(self.buffer).as_ref() != Some(&new_key) {
                        park_active(self.buffer, &mut self.registry);
                        *self.buffer = match self.registry.take(&new_key) {
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
                        crate::page::set_measure(self.config.measure_for(self.buffer.page_class()));
                    }
                }
                self.accept = Some((kind, val));
            }
            // SAVE-FEEDBACK round: `Action::Save` on the true scratch surface —
            // the ONE headless-reachable half of the effect (see its own doc):
            // convert the buffer into a real note under the harness's own
            // `notes_root`, using the SAME `Buffer::save_as_note` the live App
            // calls. This actually writes through the active `fs` backend (the
            // fixture / real disk), so the sidecar's `cursor`/buffer state and a
            // later Goto both see the new file — no notice to reflect (live-only).
            actions::Effect::ConvertScratchAndSave => {
                let _ = self.buffer.save_as_note(self.notes_root);
            }
            // Go-to's HEADINGS lens accepted (the retired Outline picker): jump the
            // cursor to the accepted heading LINE so the capture's `cursor` block
            // reflects the jump (agent-verifiable), mirroring the live App.
            actions::Effect::JumpToLine(line) => {
                let idx = self.buffer.line_col_to_char(line, 0);
                self.buffer.set_cursor(idx);
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
                if let Some(ov) = self.overlay.as_mut() {
                    ov.notice = format!("bound {slug} -> {binding}");
                    ov.capture_abort();
                }
            }
            // REBIND MENU reset: likewise reflected in the NOTICE only (intercept
            // already set it); no file mutation in the capture path.
            actions::Effect::RebindReset { slug } => {
                if let Some(ov) = self.overlay.as_mut() {
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
            // KEEP THIS VERSION: pinning a (possibly NAMED) snapshot writes the
            // local-history store, a live-App-only concern (`App::keep_version`) —
            // the history determinism gate keeps every store write off the capture
            // path, so this is a no-op here (the pin/name/exemption logic is
            // unit-tested in `history/` instead; the naming MINIBUFFER's
            // open/type/cancel flow IS core-driven and stays fully
            // `--keys`-drivable, mirroring Rename — only the commit is inert).
            | actions::Effect::KeepVersion { .. }
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
            // DOWNLOAD FILE (web-only): building a Blob/object-URL and clicking a
            // synthetic download anchor is a live-App-only DOM handoff
            // (`App::download_file`) — a capture must never touch the DOM, so this
            // is a no-op here; the filename derivation itself is unit-tested pure
            // (`web_export::filename_for`). Also gated off entirely on native by
            // `commands::action_available` before this effect can even be signaled.
            | actions::Effect::DownloadFile
            // EXPORT: rendering the document + writing the `.docx`/`.html` sibling
            // (or a web download) is a live-App-only concern (`App::export_document`)
            // — a capture must never write an export file, so this is a documented
            // no-op here; the exporter core itself is unit-tested pure (`export/`).
            | actions::Effect::Export(_)
            // CHECK FOR UPDATES: recording the local "last checked" marker and
            // opening the site's `/check?v=…` page are both live-App-only
            // concerns (`App::check_for_updates`) — a capture must never touch
            // the marker file or spawn a browser, so this is a documented no-op
            // here; the URL composition + marker round-trip are unit-tested pure
            // (`updates.rs`). Matches `ReportProblem`'s own headless behavior
            // exactly — see `updates.rs`'s module doc.
            | actions::Effect::CheckForUpdates
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
        Ok(())
    }

    /// Fold the session's end-state into the [`ReplayResult`] the capture path
    /// consumes. Consumes the session (the borrows end here).
    fn finish(self) -> ReplayResult {
        // The active `buffer` + whatever the registry still has backgrounded.
        let buffers_open = self.registry.len() + 1;
        let zoom_out = if self.zoom != 1.0 { Some(self.zoom) } else { None };
        let sel = self.buffer.selection_line_col();
        let search_query = self.search.as_ref().map(|s| s.query().to_string());
        let search_case = self.search.as_ref().map(|s| s.is_case_sensitive()).unwrap_or(false);
        let replace_active = self.search.as_ref().map(|s| s.is_replace_active()).unwrap_or(false);
        let replacement =
            self.search.as_ref().map(|s| s.replacement().to_string()).unwrap_or_default();
        let editing_replacement =
            self.search.as_ref().map(|s| s.is_editing_replacement()).unwrap_or(false);
        ReplayResult {
            zoom: zoom_out,
            selection: sel,
            search_query,
            search_case,
            replace_active,
            replacement,
            editing_replacement,
            overlay: self.overlay,
            accept: self.accept,
            buffers_open,
            intercepts: self.intercepts,
            warnings: self.warnings,
        }
    }

    // ── Storyboard-runner views over the LIVE session (crate::story) ──

    /// The active buffer, read-only (cursor / selection / text for expectations
    /// and the per-step render).
    pub(crate) fn buffer(&self) -> &Buffer {
        self.buffer
    }

    /// The replay's current zoom factor (1.0 = default).
    pub(crate) fn zoom(&self) -> f32 {
        self.zoom
    }

    /// The open isearch panel, if any.
    pub(crate) fn search(&self) -> Option<&crate::search::SearchState> {
        self.search.as_ref()
    }

    /// The open overlay, if any.
    pub(crate) fn overlay(&self) -> Option<&crate::overlay::OverlayState> {
        self.overlay.as_ref()
    }

    /// Open-buffer count (the active one + everything backgrounded).
    pub(crate) fn buffers_open(&self) -> usize {
        self.registry.len() + 1
    }

    /// Drain the per-chord trace records accumulated since the last drain — the
    /// storyboard runner calls this once per step.
    pub(crate) fn drain_records(&mut self) -> Vec<crate::storyboard::ChordTrace> {
        std::mem::take(&mut self.records)
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
    keys: Vec<crate::keyspec::Chord>,
    mut km: crate::keymap::KeymapState,
    root: Option<PathBuf>,
    workspace: Option<PathBuf>,
    notes_root: PathBuf,
    config: Config,
    // STRICT REPLAY (`--strict-replay`): abort on any Unsupported effect, an
    // unbound/dangling chord (the resolver refuses at replay time), or a
    // missing layout oracle instead of degrading quietly — see `crate::replay`.
    strict: bool,
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
            // PROSE-DIFF VIEW (capture harness, env-gated — a no-op unless
            // AWL_DIFF_OLD/NEW are set): render the marked-up-manuscript transcript
            // (the pure `prosediff` core → awl's own strike / highlight / blockquote-
            // dim vocabulary) as a markdown scratch buffer, so `--screenshot` renders
            // the READ-ONLY diff view (a live `App` feature) pixel-for-pixel and the
            // sidecar `diff` block reports its state. See `src/prosediff.rs`.
            if let Some((md, counts, label)) = crate::prosediff::env_capture_render() {
                buffer = crate::buffer::Buffer::from_str(&md);
                // Park the caret on the blank line 1 (between the title and the first
                // diff block) so NO line's WYSIWYG conceal reveals — the reveal is
                // caret-line-scoped and line 1 carries no markup, so the title's `#`
                // and every `==`/`>`/strike marker below stay concealed: the clean
                // marked-up manuscript, never a revealed-raw line. Mirrors the live
                // History-preview fold (`sync_view` parks the caret the same way —
                // the ONE reveal-suppression rule, shared, so live == capture).
                buffer.set_cursor(buffer.line_col_to_char(1, 0));
                opts.diff = Some(capture::DiffInfo {
                    active: true,
                    label,
                    struck: counts.struck,
                    washed: counts.washed,
                    modified: counts.modified,
                    moved: counts.moved,
                    folds: counts.folds,
                });
            }
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
            // SAME wrap geometry the live window does (and is re-shaped from the
            // current replay state before every action — `OraclePipeline::refresh`,
            // called inside the replay loop). Skipped for an empty spec (no motion
            // to resolve) and absent on GPU-less hosts (logical fallback).
            let mut oracle = if keys.is_empty() {
                None
            } else {
                capture::build_oracle(&buffer, &opts)
            };
            // STRICT REPLAY: a spec with keys MUST ride the real wrap geometry —
            // a missing oracle (no GPU adapter) means visual-line motion would
            // silently fall back to logical lines, so strict refuses up front.
            if strict && !keys.is_empty() && oracle.is_none() {
                return Err(crate::replay::missing_oracle_error());
            }
            let mode = if strict {
                crate::replay::Mode::Strict
            } else {
                crate::replay::Mode::Permissive
            };
            let res = replay_keys_mode(
                mode,
                &mut buffer,
                &keys,
                &corpus,
                &active_root,
                Some(effective_workspace.as_path()),
                &notes_root,
                &config,
                oracle.as_mut(),
                &mut km,
            )?;
            if opts.zoom.is_none() {
                opts.zoom = res.zoom;
            }
            if opts.selection.is_none() {
                opts.selection = res.selection;
            }
            if opts.search.is_none() {
                opts.search = res.search_query;
                opts.search_case_sensitive = opts.search_case_sensitive || res.search_case;
                // REPLACE mode the replay opened (Cmd-R / Tab / Cmd-Option-F) —
                // surfaced so the panel's replace row renders + the sidecar
                // reports it, along with the replayed replacement TEXT and which
                // field currently has focus (all typed/moved through the shared
                // search-key seam).
                opts.search_replace_active = res.replace_active;
                opts.search_replacement = res.replacement;
                opts.search_editing_replacement = res.editing_replacement;
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
                let (info, preview_text, diff) = overlay_capture_info(ov, &buffer);
                opts.overlay = Some(info);
                opts.preview_text = preview_text;
                // DIFF-AS-PREVIEW: surface the preview's diff STATE in the
                // top-level `diff` block (the AWL_DIFF harness's env request, if
                // any, was set earlier and wins), and honor the overlay's diff
                // scroll as the capture's scroll unless the spec pinned one.
                if opts.diff.is_none() {
                    opts.diff = diff;
                }
                if opts.scroll.is_none() && opts.preview_text.is_some() {
                    opts.scroll = Some(ov.diff_scroll);
                }
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

/// Fold ONE still-open overlay into its sidecar [`capture::OverlayInfo`] block
/// plus the History live-preview TEXT (if that overlay is the History timeline
/// — see [`history_preview_for`]). Extracted from [`capture_screenshot`]
/// VERBATIM so the storyboard runner's per-step render (`crate::story`) and the
/// one-shot `--keys` capture share ONE owner of "what does an open overlay
/// report" — the two can never drift.
pub(crate) fn overlay_capture_info(
    ov: &crate::overlay::OverlayState,
    buffer: &Buffer,
) -> (capture::OverlayInfo, Option<String>, Option<capture::DiffInfo>) {
    // HISTORY timeline (DIFF-AS-PREVIEW): the highlighted row's writer's-DIFF
    // previews in the document itself — resolve it here so the capture folds
    // the transcript over the snapshot text and the sidecar reports
    // `preview_id` + the previewed `text` + the `diff` counts block (exactly
    // what the live preview shows).
    let preview = history_preview_for(ov, buffer);
    let preview_text = preview.as_ref().map(|(_, transcript, _)| transcript.clone());
    // The sidecar's top-level `diff` block now ALSO reports a History preview's
    // counts (the same DiffInfo shape the `AWL_DIFF_*` harness fills) — the
    // label is the picker row the user is looking at.
    let diff = preview.as_ref().map(|(_, _, c)| capture::DiffInfo {
        active: true,
        label: ov.selected_value().unwrap_or("an earlier version").to_string(),
        struck: c.struck,
        washed: c.washed,
        modified: c.modified,
        moved: c.moved,
        folds: c.folds,
    });
    let info = capture::OverlayInfo {
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
        preview_id: preview.map(|(id, _, _)| id),
        diff_focus: ov.diff_focus,
        diff_scroll: ov.diff_scroll,
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
        title: ov.kind.title(),
    };
    (info, preview_text, diff)
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
) -> Option<(String, String, crate::prosediff::DiffCounts)> {
    // DIFF-AS-PREVIEW: the preview IS the writer's diff of the current buffer vs
    // the highlighted version — built by the SAME one owner the live App renders
    // through (`history::diff_preview`), synchronously (the live debounce is a
    // wall-clock concern the deterministic capture never has).
    crate::history::diff_preview(ov, buffer.path(), None, buffer.is_note(), &buffer.text())
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
            km,
            root,
            workspace,
            notes_root,
            config,
            strict,
        } => capture_screenshot(out, file, opts, keys, km, root, workspace, notes_root, config, strict),
        Mode::ScreenshotMotion { out, file, keys, mut km } => {
            let mut buffer = load_buffer(&file);
            let root = resolve_root(&None, &file, None);
            replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None, &mut km);
            capture::capture_motion(&out, &buffer)?;
            println!("wrote {} (mid-glide, + sidecar .json)", out.display());
            Ok(())
        }
        Mode::ScreenshotMotionVertical { out, file, keys, mut km } => {
            let mut buffer = load_buffer(&file);
            let root = resolve_root(&None, &file, None);
            replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None, &mut km);
            capture::capture_motion_vertical(&out, &buffer)?;
            println!("wrote {} (mid-glide vertical, + sidecar .json)", out.display());
            Ok(())
        }
        Mode::ScreenshotMotionDiagonal { out, file, keys, mut km } => {
            let mut buffer = load_buffer(&file);
            let root = resolve_root(&None, &file, None);
            replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None, &mut km);
            capture::capture_motion_diagonal(&out, &buffer)?;
            println!("wrote {} (mid-glide diagonal, + sidecar .json)", out.display());
            Ok(())
        }
        #[cfg(not(target_arch = "wasm32"))]
        Mode::ScreenshotFrames { out, file, frames, step_ms } => {
            // The document is a stationary backdrop; the real App (built inside the
            // harness) drives the scheduling. No `--keys` replay — the which-key
            // prefix is armed directly at virtual t=0 (see `capture::frames`).
            let buffer = load_buffer(&file);
            capture::capture_frames(&out, &buffer, frames, step_ms, &CaptureOpts::default())?;
            let stem = out
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "capture".to_string());
            println!(
                "wrote {frames} frame(s) to {stem}.fNNN.png (step {step_ms}ms, + per-frame sidecars, + {stem}.frames.json)"
            );
            Ok(())
        }
        Mode::CaptureTimeline {
            out,
            file,
            keys,
            mut km,
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
            // ONE keymap across both halves, so a prefix armed by the origin
            // replay still resolves the glide chord (the split is at CHORD
            // boundaries now that resolution lives inside the replay loop).
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
                    &mut km,
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
                    &mut km,
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
            mut km,
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
                replay_keys(&mut buffer, &keys, &corpus, &active_root, None, &notes_root, &Config::empty(), None, &mut km);
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
        Mode::Storyboard { board, file, out_dir, root, workspace, notes_root, config, km } => {
            crate::story::run_storyboard(
                board, file, out_dir, root, workspace, notes_root, config, km,
            )
        }
        Mode::BenchTyping => bench::run(),
        Mode::BenchPerf => crate::render::perfbench::run(),
        Mode::BenchFrame => crate::render::framebench::run(),
        Mode::BenchThemeBurst => crate::render::framebench::run_theme_burst(),
        Mode::BenchZoomBurst => crate::render::framebench::run_zoom_burst(),
        Mode::BenchSuite { baseline } => crate::render::benchsuite::run(baseline),
        #[cfg(not(target_arch = "wasm32"))]
        Mode::SoakGpu(config) => {
            let root = std::env::temp_dir().join(format!("awl-soak-gpu-{}", std::process::id()));
            std::fs::create_dir_all(&root)?;
            let result = app::run(
                None,
                root.clone(),
                None,
                None,
                Config::empty(),
                false,
                Some(config),
                None,
            );
            let _ = std::fs::remove_dir(&root);
            result
        }
        Mode::Windowed {
            file,
            root,
            workspace,
            notes_root,
            config,
            wait,
            live,
        } => {
            // STICKY PROJECT RESTORE: on a bare launch (no file argument, no
            // explicit --root) the remembered project root wins; see
            // `resolve_root`'s doc comment.
            let active_root = resolve_root(&root, &file, config.project_root.as_deref());
            // Pass the RAW flags + config; `App::new` folds them (flag > config >
            // default) and re-folds on a live config reload. `wait` (native-only,
            // the single-instance daemon's `--wait`) rides straight through, as
            // does `live` (the `--live-script` probe — see `crate::probe`).
            #[cfg(not(target_arch = "wasm32"))]
            { app::run(file, active_root, workspace, notes_root, config, wait, None, live) }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = live; // native-live-only; parsed as None on wasm
                app::run(file, active_root, workspace, notes_root, config, wait)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // CONVENTION-PROOF SHADOWS: this whole file's `--keys` replay tests hardcode
    // MAC-form literal specs ("Cmd-S-h", "s-p", a bare "C-n"/"C-x" whose letter
    // Linux's collision table displaces, …) — pinning resolution to
    // `Convention::Mac` is the honest fix (these tests document specifically
    // what a MAC-convention chord does; Linux's own displacement/collision
    // behavior is separately, exhaustively law-tested in `keymap.rs`). Chord
    // PARSING is now convention-free (`parse_chords` never touches the keymap),
    // so the pinning moved WITH resolution into the replay loop: these local
    // `replay_keys`/`replay_keys_mode` wrappers SHADOW the module-level fns
    // (a local item wins over a glob import) and supply a Mac-pinned
    // `KeymapState`, so none of the ~60 call sites below needed rewriting. The
    // local `keyspec` module keeps the old call shape for the same reason.
    mod keyspec {
        pub fn parse_keys(spec: &str) -> anyhow::Result<Vec<crate::keyspec::Chord>> {
            crate::keyspec::parse_chords(spec)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn replay_keys_mode(
        mode: crate::replay::Mode,
        buffer: &mut Buffer,
        keys: &[crate::keyspec::Chord],
        corpus: &[String],
        root: &std::path::Path,
        workspace: Option<&std::path::Path>,
        notes_root: &std::path::Path,
        config: &Config,
        oracle: Option<&mut capture::OraclePipeline>,
    ) -> Result<ReplayResult> {
        let mut km =
            crate::keymap::KeymapState::new_with_convention(crate::convention::Convention::Mac);
        super::replay_keys_mode(
            mode, buffer, keys, corpus, root, workspace, notes_root, config, oracle, &mut km,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn replay_keys(
        buffer: &mut Buffer,
        keys: &[crate::keyspec::Chord],
        corpus: &[String],
        root: &std::path::Path,
        workspace: Option<&std::path::Path>,
        notes_root: &std::path::Path,
        config: &Config,
        oracle: Option<&mut capture::OraclePipeline>,
    ) -> ReplayResult {
        match replay_keys_mode(
            crate::replay::Mode::Permissive,
            buffer,
            keys,
            corpus,
            root,
            workspace,
            notes_root,
            config,
            oracle,
        ) {
            Ok(res) => res,
            Err(e) => unreachable!("permissive replay never aborts: {e}"),
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

    // ── REPLAY SHIFT-SELECT LAWS: `S-` on a motion is select-intent, exactly
    // as a live held Shift (the retired "replay is unshifted" hole). The
    // replay derives its `apply_core` shift flag through the ONE owner
    // (`crate::app::motion_honors_shift_select`), so these laws pin the
    // OUTCOME: a spec's `S-` chord builds the same selection live Shift+motion
    // does, and the documented non-movers stay non-movers. ──

    /// The shared fixture the shift-select laws replay over: three lines, so
    /// every catalog motion has somewhere to go from the middle.
    const SHIFT_FIXTURE: &str = "alpha beta\ngamma delta\nepsilon zeta\n";

    /// Replay `spec` against a fresh [`SHIFT_FIXTURE`] buffer, returning the
    /// post-replay `(selection, cursor)` — the exact pair the capture sidecar
    /// publishes (`ReplayResult::selection` feeds `CaptureOpts::selection`
    /// feeds the sidecar `selection` field; `cursor` is read off the buffer
    /// the same way the capture's `ViewState` is).
    #[allow(clippy::type_complexity)]
    fn shift_replay(
        spec: &str,
    ) -> (Option<((usize, usize), (usize, usize))>, (usize, usize)) {
        let mut buffer = Buffer::from_str(SHIFT_FIXTURE);
        let keys = keyspec::parse_keys(spec).unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        (res.selection, buffer.cursor_line_col())
    }

    #[test]
    fn replay_shift_arrow_extends_a_real_selection_then_unshifted_motion_collapses() {
        // THE LAW that closes the trap: `--keys "S-Right S-Right"` extends a
        // real two-char selection headlessly (the sidecar `selection` is this
        // exact value — see `shift_replay`'s doc), where the old unshifted
        // replay silently produced `selection: null` with the cursor at (0,2).
        let (sel, cursor) = shift_replay("S-Right S-Right");
        assert_eq!(cursor, (0, 2), "the motion itself still runs");
        assert_eq!(
            sel,
            Some(((0, 0), (0, 2))),
            "S-Right S-Right spans exactly the two chars the live Shift+Right pair selects"
        );
        // And the OTHER half of the live transient-selection contract: the next
        // UNSHIFTED motion collapses it (GUI style), exactly like live.
        let (sel, cursor) = shift_replay("S-Right S-Right Right");
        assert_eq!(cursor, (0, 3));
        assert_eq!(sel, None, "an unshifted motion collapses the transient shift-selection");
    }

    #[test]
    fn replay_shift_extends_every_catalog_motion_exactly_as_live() {
        // Enumerate the keymap's own motion roster — every catalog command whose
        // action `is_motion()`, over BOTH binding slots — rather than a hand
        // list that can drift: a new motion command is swept automatically.
        // For each, replay its chord with an `S-` prefix from mid-document and
        // assert the LIVE equivalence through the one owner: the selection
        // extends iff `motion_honors_shift_select` — keyed on the pressed CHORD's
        // KEY, not the Action alone — says the Shift is select-intent. So
        // Shift+Cmd-Up/Down (a NAMED nav key reaching BufferStart/BufferEnd) DO
        // extend, GUI-style; only an incidental-Shift printable glyph (`M-<`/`M->`,
        // a `Key::Character`, pinned at the pure-fn seam in `app.rs`) would not.
        // When it extends it spans exactly (pre-cursor, post-cursor).
        // Setup: plain arrows walk to (1,3) — unshifted motions build no selection.
        const SETUP: &str = "Down Right Right Right";
        let (pre_sel, pre_cursor) = shift_replay(SETUP);
        assert_eq!(pre_sel, None);
        assert_eq!(pre_cursor, (1, 3), "setup parks the cursor mid-document");
        let mut swept = 0usize;
        let mut extended_a_named_endpoint = false;
        for cmd in crate::commands::COMMANDS.iter().filter(|c| c.action.is_motion()) {
            for chord in [cmd.native, cmd.emacs] {
                if chord.is_empty() {
                    continue;
                }
                swept += 1;
                let spec = format!("S-{chord}");
                // Key on the pressed CHORD exactly as the replay + live dispatch
                // do: parse the same `S-{chord}` token and read its resolved key.
                let key = keyspec::parse_keys(&spec)
                    .unwrap()
                    .last()
                    .expect("one chord")
                    .key
                    .clone();
                let (sel, cursor) = shift_replay(&format!("{SETUP} {spec}"));
                assert_ne!(
                    cursor, pre_cursor,
                    "{} (S-{chord}): the motion must actually move (witness)",
                    cmd.name
                );
                if crate::app::motion_honors_shift_select(&cmd.action, &key) {
                    let expected = (pre_cursor.min(cursor), pre_cursor.max(cursor));
                    assert_eq!(
                        sel,
                        Some(expected),
                        "{} (S-{chord}): Shift+motion extends exactly like live",
                        cmd.name
                    );
                    // Witness the fix: at least one BufferStart/BufferEnd endpoint,
                    // reached via a named key, DID extend (the old rule excluded
                    // these outright, so Shift+Cmd-Up/Down silently didn't select).
                    if matches!(cmd.action, Action::BufferStart | Action::BufferEnd) {
                        extended_a_named_endpoint = true;
                    }
                } else {
                    assert_eq!(
                        sel, None,
                        "{} (S-{chord}): incidental Shift stays pure motion, like live",
                        cmd.name
                    );
                }
            }
        }
        assert!(swept >= 10, "the catalog motion roster shrank? swept only {swept} chords");
        assert!(
            extended_a_named_endpoint,
            "a named-key BufferStart/BufferEnd chord must extend now (the shift-select fix)"
        );
    }

    #[test]
    fn replay_shift_named_key_arms_extend_like_live() {
        // The KEYMAP-ONLY named-key arms (plain arrows, Home/End, and the
        // convention-free Ctrl-arrow word aliases live as hand-written input
        // policy in `resolve_named` — no data table exists to enumerate, so
        // these pins mirror `keymap.rs`'s own arm-by-arm style). Each replays
        // with `S-` from mid-document and must extend, spanning exactly
        // (pre, post) — including Shift COMPOSED with M-/C- (the shifted-
        // variant fill / Ctrl-arrow alias, resolving identically to live).
        const SETUP: &str = "Down Right Right Right";
        let (_, pre) = shift_replay(SETUP);
        for chord in ["S-Left", "S-Right", "S-Up", "S-Down", "S-Home", "S-End", "S-M-Right", "S-M-Left", "S-C-Right", "S-C-Left"] {
            let (sel, cursor) = shift_replay(&format!("{SETUP} {chord}"));
            assert_ne!(cursor, pre, "{chord}: the motion must actually move (witness)");
            assert_eq!(
                sel,
                Some((pre.min(cursor), pre.max(cursor))),
                "{chord}: Shift extends the selection exactly like live"
            );
        }
    }

    #[test]
    fn replay_shift_cmd_up_down_extend_to_document_bounds() {
        // THE BUG THIS ROUND FIXED, pinned at the sidecar seam: Shift+Cmd-Up /
        // Shift+Cmd-Down (`S-s-Up` / `S-s-Down` — `s-` is Super/Cmd) select to
        // the document start / end, exactly like every platform text field. The
        // old rule excluded BufferStart/BufferEnd outright (guarding the retired
        // `M-<`/`M->` incidental Shift), so these silently produced `selection:
        // null` — the reported defect. `--keys "S-s-Down"` is what a live drive
        // would witness in the capture sidecar.
        const SETUP: &str = "Down Right Right Right";
        let (_, pre) = shift_replay(SETUP);
        assert_eq!(pre, (1, 3), "setup parks the cursor mid-document");
        // Shift+Cmd-Up extends up to the very start (0,0).
        let (sel, cursor) = shift_replay(&format!("{SETUP} S-s-Up"));
        assert_eq!(cursor, (0, 0), "Cmd-Up still lands on document start");
        assert_eq!(
            sel,
            Some(((0, 0), (1, 3))),
            "S-s-Up extends the selection from mid-document to the start"
        );
        // Shift+Cmd-Down extends down to the document end.
        let (sel, cursor) = shift_replay(&format!("{SETUP} S-s-Down"));
        assert_ne!(cursor, pre, "Cmd-Down moves the caret to the document end");
        assert_eq!(
            sel,
            Some((pre.min(cursor), pre.max(cursor))),
            "S-s-Down extends the selection from mid-document to the end"
        );
    }

    #[test]
    fn replay_shift_page_scroll_stays_a_documented_non_mover() {
        // Shift-PageDown/PageUp deliberately do NOT extend a selection (the
        // documented divergence — `is_motion` excludes PageScroll*, so the
        // shift-select block never arms). Pin it so the replay-shift fix can
        // never silently promote them; promoting is a conscious follow-up.
        let (sel, cursor) = shift_replay("S-PageDown");
        assert_ne!(cursor, (0, 0), "the page scroll still moves the cursor");
        assert_eq!(sel, None, "Shift-PageDown stays a non-extending non-mover");
        let (sel, _) = shift_replay("S-PageDown S-PageUp");
        assert_eq!(sel, None, "Shift-PageUp stays a non-extending non-mover");
    }

    // ── STRICT REPLAY TRUTHFULNESS: the mode-aware replay engine ──

    #[test]
    fn strict_replay_aborts_on_an_unsupported_effect_naming_action_and_effect() {
        // Cmd-Q's `Effect::Quit` is classified Unsupported (live exits the
        // event loop; a replay would keep applying keys past it) — the strict
        // door must refuse it, naming the exact action AND effect.
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("s-q").unwrap();
        let root = PathBuf::from("/tmp");
        let err = replay_keys_mode(
            crate::replay::Mode::Strict,
            &mut buffer,
            &keys,
            &[],
            &root,
            None,
            &root,
            &Config::empty(),
            None,
        )
        .err()
        .expect("strict replay aborts on an unsupported effect")
        .to_string();
        assert!(err.contains("`quit`"), "effect named: {err}");
        assert!(err.contains("Quit"), "action named: {err}");
        assert!(err.starts_with("strict replay:"), "{err}");
    }

    #[test]
    fn strict_replay_records_intercepted_handoffs_without_aborting() {
        // C-c C-o on a link produces `Effect::FollowLink(url)` — an EXTERNAL
        // handoff the replay observes and records but never performs. Strict
        // must PASS it (that's the intercept contract, not a violation) and the
        // recorded intercept must carry the observed URL — the phase-5 trace seam.
        let mut buffer = Buffer::from_str("[a](https://awl.example/doc) tail");
        buffer.set_cursor(1); // inside the link
        let root = PathBuf::from("/tmp");
        // Follow link's real chord (the emacs slot, `C-c C-o`) — resolution now
        // happens inside the replay loop, so the spec drives the same door.
        let keys = keyspec::parse_keys("C-c C-o").unwrap();
        let res = replay_keys_mode(
            crate::replay::Mode::Strict,
            &mut buffer,
            &keys,
            &[],
            &root,
            None,
            &root,
            &Config::empty(),
            None,
        )
        .expect("intercepted handoffs are legal under strict");
        assert_eq!(
            res.intercepts,
            vec![crate::replay::Intercept {
                effect: "follow_link",
                detail: "https://awl.example/doc".into()
            }]
        );
        assert!(res.warnings.is_empty(), "strict records silently, never warns");
    }

    #[test]
    fn permissive_replay_never_aborts_and_warns_on_both_non_applied_seams() {
        // The legacy `--keys` door: an Unsupported effect (Quit) and an
        // Intercepted one (FollowLink) both WARN — the same strings printed to
        // stderr are recorded here — and the replay runs to completion (the
        // key AFTER the Quit still applies, today's documented behavior).
        let mut buffer = Buffer::from_str("[a](https://awl.example/x) tail");
        buffer.set_cursor(1);
        let keys = keyspec::parse_keys("s-q C-c C-o s-Down").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert_eq!(res.warnings.len(), 2, "one warning per crossing: {:?}", res.warnings);
        assert!(
            res.warnings[0].contains("skipped unsupported effect `quit`"),
            "{}",
            res.warnings[0]
        );
        assert!(
            res.warnings[1].contains("intercepted `follow_link`")
                && res.warnings[1].contains("https://awl.example/x"),
            "{}",
            res.warnings[1]
        );
        assert_eq!(res.intercepts.len(), 1, "the handoff is recorded permissively too");
        let (line, col) = buffer.cursor_line_col();
        assert!(line > 0 || col > 0, "the key after Quit still applied (BufferEnd moved)");
    }

    #[test]
    fn a_fully_applied_replay_stays_warning_and_intercept_free() {
        // The common case (typing/motion) crosses no seam: the permissive door
        // is byte-identical to the pre-round replay, with silent stderr.
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("a b c C-a C-e").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert!(res.warnings.is_empty(), "{:?}", res.warnings);
        assert!(res.intercepts.is_empty());
    }

    // ── HERMETIC SCENARIO FILESYSTEM: the strict door's sandbox ──
    //
    // `crate::scenario` owns the seam (its own tests pin seeding + install);
    // these pin the COMPOSITION with the replay engine: a strict scenario's
    // writes land in the sandbox and its external handoffs stay intercepted,
    // while the REAL files keep every byte.

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn hermetic_scenario_save_lands_in_the_sandbox_never_on_real_disk() {
        // Arrange a REAL input file (the storyboard input), enter the sandbox
        // through the ONE production door, then strict-replay an edit + save:
        // the sandboxed copy updates, the real file keeps every byte — the
        // hermetic inverse of CAPTURE.md's legacy "save writes to disk" caveat.
        let dir = std::env::temp_dir().join(format!("awl-hermetic-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("doc.md");
        std::fs::write(&input, "alpha\n").unwrap();
        {
            // FsGuard(current) restores whatever the install swaps in, even on
            // a failed assert, so no sibling test ever sees the sandbox.
            let _restore = crate::fs::FsGuard::install(crate::fs::active());
            crate::scenario::install_hermetic_fs(Some(&input), None, Some(&dir));
            let mut buffer = load_buffer(&Some(input.clone()));
            assert_eq!(buffer.text(), "alpha\n", "the sandbox seeded the real input's bytes");
            let keys = keyspec::parse_keys("X s-s").unwrap();
            let res = replay_keys_mode(
                crate::replay::Mode::Strict,
                &mut buffer,
                &keys,
                &[],
                &dir,
                None,
                &dir,
                &Config::empty(),
                None,
            )
            .expect("an edit + save crosses no unsupported seam");
            assert!(res.intercepts.is_empty());
            assert_eq!(
                crate::fs::active().read_to_string(&input).unwrap(),
                "Xalpha\n",
                "the replayed save landed in the sandbox"
            );
        }
        assert_eq!(
            std::fs::read_to_string(&input).unwrap(),
            "alpha\n",
            "the REAL file keeps every byte a hermetic scenario 'saved'"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn hermetic_scenario_witnesses_the_url_handoff_as_an_intercept() {
        // The phase-1 intercept seam COMPOSED with the sandbox: a strict
        // scenario driving "open link at caret" records the handoff — URL
        // included — performs nothing, and leaves both filesystems byte-
        // identical (the sandbox untouched beyond its seed, the real file
        // untouched entirely).
        let dir = std::env::temp_dir().join(format!("awl-hermetic-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("linked.md");
        let body = "[a](https://awl.example/doc) tail\n";
        std::fs::write(&input, body).unwrap();
        {
            let _restore = crate::fs::FsGuard::install(crate::fs::active());
            crate::scenario::install_hermetic_fs(Some(&input), None, Some(&dir));
            let mut buffer = load_buffer(&Some(input.clone()));
            // Right lands the caret inside the link, C-c C-o follows it.
            let keys = keyspec::parse_keys("Right C-c C-o").unwrap();
            let res = replay_keys_mode(
                crate::replay::Mode::Strict,
                &mut buffer,
                &keys,
                &[],
                &dir,
                None,
                &dir,
                &Config::empty(),
                None,
            )
            .expect("an intercepted handoff is legal under strict");
            assert_eq!(
                res.intercepts,
                vec![crate::replay::Intercept {
                    effect: "follow_link",
                    detail: "https://awl.example/doc".into()
                }],
                "the handoff was observed and recorded, not performed"
            );
            assert_eq!(
                crate::fs::active().read_to_string(&input).unwrap(),
                body,
                "the sandbox copy is untouched (following a link edits nothing)"
            );
        }
        assert_eq!(std::fs::read_to_string(&input).unwrap(), body, "the real file too");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── SHARED SEARCH/REPLACE INPUT ROUTING: the replay-side search guard ──
    //
    // While the isearch panel is open the replay loop consumes EVERY chord
    // through the ONE interception seam the live window uses
    // (`crate::search::keys::intercept`), BEFORE keymap resolution — so the
    // whole in-panel operation set is `--keys`-drivable. The seam itself is
    // unit-tested in `search::keys::tests`; these pin the replay WIRING.

    #[test]
    fn replay_search_typing_extends_the_query_never_the_buffer() {
        // THE search-typing regression (the retired isearch-input gap): typing
        // after C-s used to insert into the BUFFER because in-panel chars
        // resolved through the keymap to `InsertChar`. Now the guard routes
        // them to the query, and the caret lands on the current match.
        let mut buffer = Buffer::from_str("say hi twice: hi");
        let keys = keyspec::parse_keys("C-s h i").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert_eq!(buffer.text(), "say hi twice: hi", "the document is untouched");
        assert_eq!(res.search_query.as_deref(), Some("hi"));
        assert_eq!(buffer.cursor_char(), 4, "the caret sits on the first match");
    }

    #[test]
    fn replay_search_steps_case_toggle_and_prefix_chords_stay_in_the_panel() {
        // STEP next/previous while open: C-s/arrows advance the current match
        // (they used to RESTART the search through `start_search`).
        let mut buffer = Buffer::from_str("x.x.x");
        let keys = keyspec::parse_keys("C-s x Down C-s").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert_eq!(res.search_query.as_deref(), Some("x"));
        assert_eq!(buffer.cursor_char(), 4, "two steps advanced 0 -> 2 -> 4");

        // CASE TOGGLE (M-c) — a chord with NO keymap binding at all, reachable
        // only because the guard consumes it before resolution.
        let mut buffer = Buffer::from_str("Hello HELLO hello");
        let keys = keyspec::parse_keys("C-s h e l l o M-c").unwrap();
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert!(res.search_case, "M-c toggled case sensitivity inside the panel");
        assert_eq!(res.search_query.as_deref(), Some("hello"));

        // A `C-x` while searching is CONSUMED (a no-op), never a prefix arm —
        // the following `r` extends the QUERY instead of resolving to the
        // `C-x r` ToggleDebug sequence. (The old parse-time resolution got
        // this wrong by construction.)
        let _g = crate::testlock::serial();
        let debug_before = crate::debug::debug_on();
        let mut buffer = Buffer::from_str("xr marks");
        let keys = keyspec::parse_keys("C-s x C-x r").unwrap();
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert_eq!(res.search_query.as_deref(), Some("xr"), "C-x was eaten; r joined the query");
        assert_eq!(crate::debug::debug_on(), debug_before, "C-x r never reached the keymap");
    }

    #[test]
    fn replay_search_replacement_typing_replace_one_and_replace_all() {
        // Tab reveals + focuses the replace field; typed chars fill it; Enter
        // replaces the CURRENT match and advances (panel stays open).
        let mut buffer = Buffer::from_str("line one\nline two\nline three");
        let keys = keyspec::parse_keys("C-s l i n e Tab r o w Enter").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert_eq!(buffer.text(), "row one\nline two\nline three");
        assert_eq!(res.search_query.as_deref(), Some("line"));
        assert!(res.replace_active);
        assert_eq!(res.replacement, "row");
        assert!(res.editing_replacement, "focus stayed in the replace field");
        assert_eq!(buffer.cursor_char(), 8, "the caret advanced to the next match");

        // Cmd-Enter (s-Enter) REPLACES ALL remaining matches in one edit.
        let mut buffer = Buffer::from_str("line one\nline two\nline three");
        let keys = keyspec::parse_keys("C-s l i n e Tab r o w s-Enter").unwrap();
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert_eq!(buffer.text(), "row one\nrow two\nrow three");
        assert!(res.replace_active, "the panel is still open after replace-all");
    }

    #[test]
    fn replay_search_enter_accepts_and_esc_restores_origin() {
        // ENTER on a plain find ACCEPTS: the panel closes, the cursor stays on
        // the match, and the query is remembered (a process global -> serial()).
        let _g = crate::testlock::serial();
        crate::search::clear_last_query();
        let mut buffer = Buffer::from_str("alpha beta alpha");
        let keys = keyspec::parse_keys("C-s b e t a Enter").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert_eq!(res.search_query, None, "Enter closed the panel");
        assert_eq!(buffer.cursor_char(), 6, "the cursor stays on the accepted match");
        assert_eq!(crate::search::last_query(), "beta");

        // ESC aborts: the panel closes AND the origin cursor is restored —
        // live behavior (the old headless-only `Cancel` close skipped the
        // origin restore; that divergence is gone with the shared seam).
        let mut buffer = Buffer::from_str("alpha beta alpha");
        let keys = keyspec::parse_keys("C-f C-f C-s b e t a Esc").unwrap();
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert_eq!(res.search_query, None, "Esc closed the panel");
        assert_eq!(buffer.cursor_char(), 2, "the origin cursor is restored");
        assert_eq!(buffer.text(), "alpha beta alpha");
        crate::search::clear_last_query();
    }

    #[test]
    fn strict_replay_allows_panel_consumed_chords_but_rejects_them_outside() {
        // `s-l` (Cmd-L) is deliberately unbound (the unbound-Super swallow
        // guard): OUTSIDE a search, strict refuses it — the relocated
        // parse-time check. (`M-c` is NOT the contrast chord: outside a panel
        // the retired Option-letter layer lets it fall through to self-insert.)
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("s-l").unwrap();
        let root = PathBuf::from("/tmp");
        let err = replay_keys_mode(
            crate::replay::Mode::Strict,
            &mut buffer,
            &keys,
            &[],
            &root,
            None,
            &root,
            &Config::empty(),
            None,
        )
        .err()
        .expect("strict refuses an unbound chord outside the panel")
        .to_string();
        assert!(err.contains("\"s-l\"") && err.contains("unbound"), "{err}");

        // …but INSIDE the panel the guard owns every chord (`s-l` is a
        // consumed no-op, `M-c` a case toggle, `C-x` never arms the prefix),
        // so the same spec is legal under strict — the reason strictness had
        // to move from parse time to replay time.
        let mut buffer = Buffer::from_str("Hello HELLO hello");
        let keys = keyspec::parse_keys("C-s h s-l M-c C-x").unwrap();
        let res = replay_keys_mode(
            crate::replay::Mode::Strict,
            &mut buffer,
            &keys,
            &[],
            &root,
            None,
            &root,
            &Config::empty(),
            None,
        )
        .expect("panel-consumed chords are legal under strict");
        assert!(res.search_case, "the M-c actually toggled case");
        assert_eq!(res.search_query.as_deref(), Some("h"));
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

    // ── NAMED SAVE POINTS: the Keep-version minibuffer stays --keys-drivable ──

    #[test]
    fn replay_keys_drives_the_keep_version_minibuffer_prompt_and_sidecar_reflects_typing() {
        // Cmd-P → "keep" → Enter opens the naming minibuffer (empty — a fresh
        // point has no old name); typing builds the optional name live — all
        // through the shared core, so both the overlay STATE and its
        // sidecar-facing `foot_hint()` (the same seam Rename/InsertLink ride)
        // reflect the in-progress edit with zero live App involved.
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("s-p k e e p RET d r a f t").unwrap();
        let root = PathBuf::from("/proj");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        let ov = res.overlay.expect("Keep version… opens the naming minibuffer");
        assert_eq!(ov.kind, crate::overlay::OverlayKind::KeepName);
        assert_eq!(ov.corpus, vec!["draft".to_string()], "typing builds the name from empty");
        assert_eq!(
            ov.foot_hint(),
            "name this version: draft   Enter keep   Esc cancel",
            "the live prompt is sidecar-visible via the same foot_hint seam Rename uses"
        );
    }

    #[test]
    fn replay_keys_keep_version_minibuffer_esc_cancels_with_no_overlay_left() {
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("s-p k e e p RET x Esc").unwrap();
        let root = PathBuf::from("/proj");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert!(res.overlay.is_none(), "Esc closes the minibuffer outright, nothing kept");
    }

    #[test]
    fn replay_keys_keep_version_commit_closes_and_defers_the_store_write() {
        // Enter commits through the REAL keymap: the overlay closes and the
        // deferred Effect::KeepVersion { name } is the documented headless no-op
        // (the history determinism gate — a capture never touches the store), so
        // the buffer and fs stay untouched.
        use crate::fs::InMemoryFs;
        let _g = crate::fs::FsGuard::install(std::sync::Arc::new(InMemoryFs::new()));
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("h i s-p k e e p RET d r a f t RET").unwrap();
        let root = PathBuf::from("/proj");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert!(res.overlay.is_none(), "commit closes the minibuffer");
        assert_eq!(buffer.text(), "hi", "the keep never edits the buffer");
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
        let expected = crate::guide::render(crate::convention::Convention::current(), crate::commands::Platform::current());
        assert_eq!(buffer.text(), expected, "the buffer now holds the token-rendered guide text");
        assert!(!buffer.text().contains("{{key:"), "no raw chord token survives in the opened guide");
        assert!(buffer.path().is_none(), "headless replay never writes/loads a real on-disk guide.md");
    }

    #[test]
    fn replay_keys_palette_filter_surfaces_the_marked_settings_row() {
        // Cmd-P → "keymap" (no Enter): the palette stays open with the union corpus
        // filtered down to the settings row, its display text carrying the `§ `
        // marker glyph — assertable straight from `res.overlay.items` (and, via the
        // sidecar, `overlay.items`).
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("s-p k e y m a p").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        let ov = res.overlay.expect("the palette is still open");
        assert_eq!(ov.kind, crate::overlay::OverlayKind::Command);
        assert!(
            ov.item_strings().iter().any(|s| s == "§ Keymap"),
            "the union corpus surfaces the marked settings row: {:?}",
            ov.item_strings()
        );
    }

    #[test]
    fn replay_keys_palette_filters_to_a_settings_row_and_toggles_it() {
        // THE UNION ROUND: Cmd-P → "keymap" filters to the SETTINGS row "Keymap"
        // (the union palette's marked settings corpus, `§ Keymap`) → Enter signals
        // the SAME `Effect::SettingToggle{key:"keymap"}` the Settings menu's own
        // accept would, and CLOSES the palette (the palette's "activation closes
        // it" convention). Note the honest scope boundary: `Effect::SettingToggle`
        // is a documented headless no-op (see the `Effect` match above) — flipping
        // + persisting the live keymap flavor is the live App's job
        // (`App::toggle_keymap_flavor`, unit-tested there); this replay proves the
        // dispatch reaches the toggle EFFECT end-to-end through the real keymap +
        // fuzzy filter + accept seam, not that the flavor value itself flips in a
        // capture (which the architecture never claims for any settings toggle).
        let mut buffer = Buffer::scratch();
        let keys = keyspec::parse_keys("s-p k e y m a p RET").unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert!(res.overlay.is_none(), "activating a settings row closes the palette");
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

    /// THE BUG this round fixes, end-to-end through the REAL `--keys` replay
    /// (the reported symptom's actual repro, not just the pure `apply_core`
    /// unit — see `actions::tests::overlay_drive::
    /// caret_picker_cancel_from_auto_restores_auto_not_a_pin` for that
    /// purer-seam sibling). Riding AUTO on a PROPORTIONAL world (Gumtree ->
    /// Morph), merely OPENING the Caret-style picker from the palette and
    /// backing out with Esc (no pick made) must be a true no-op: a LATER
    /// switch to a MONO world must still resolve Block, exactly as auto
    /// always would. Before the fix, the Cancel silently pinned the caret at
    /// Morph (auto's momentary resolution on Gumtree), so Potoroo (mono)
    /// stayed wrongly Morph.
    #[test]
    fn replay_keys_caret_picker_cancel_from_auto_does_not_pin_it() {
        let _g = crate::testlock::serial();
        let _t = crate::testlock::serial();
        crate::caret::clear_override();
        crate::theme::set_active_by_name("Gumtree").unwrap();
        assert!(crate::caret::is_auto());
        assert_eq!(crate::caret::mode(), crate::caret::CaretMode::Morph);

        let mut buffer = Buffer::scratch();
        // Palette -> filter to "Caret style…" -> Enter opens it (breadcrumb =
        // Command) -> Esc pops back to the palette -> Esc closes to the buffer
        // (no pick was ever made) -> Cmd-T -> filter "Potoroo" -> Enter commits.
        let keys = keyspec::parse_keys(
            "s-p C a r e t Space s t y l e RET Esc Esc s-t P o t o r o o RET",
        )
        .unwrap();
        let root = PathBuf::from("/tmp");
        let res = replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert!(res.overlay.is_none(), "the whole journey lands back in the buffer");
        assert_eq!(crate::theme::active().name, "Potoroo", "the theme switch landed");

        // THE LAW: auto survived the picker-open-and-cancel detour, so it
        // still tracks Potoroo's mono font — Block, not a Morph pin left over
        // from glancing at the picker while Gumtree was active.
        assert!(crate::caret::is_auto(), "cancelling the caret picker must not pin auto");
        assert_eq!(
            crate::caret::mode(),
            crate::caret::CaretMode::Block,
            "auto correctly resolves Block on the now-active mono world"
        );

        crate::caret::clear_override();
        crate::theme::set_active(crate::theme::DEFAULT_THEME);
    }

    #[test]
    fn replay_keys_goto_open_file_closes_all_no_overlay() {
        // A NAVIGATING accept closes the whole stack: ⌘O → Enter on a file lands you
        // IN the file with NO overlay left open (like a palette value-pick keep, and
        // unlike the Esc breadcrumb pop).
        let mut buffer = Buffer::scratch();
        let corpus = vec!["doc-fixture.md".to_string()];
        let root = PathBuf::from("/tmp");
        let keys = keyspec::parse_keys("s-o RET").unwrap();
        let res = replay_keys(&mut buffer, &keys, &corpus, &root, None, &root, &Config::empty(), None);
        assert!(res.overlay.is_none(), "opening a file closes the overlay to the buffer");
        assert_eq!(
            res.accept,
            Some((crate::overlay::OverlayKind::Goto, "doc-fixture.md".to_string())),
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
            "doc-fixture.md".to_string(),
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
        assert!(shown.iter().any(|s| s == "doc-fixture.md"));
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
        // process-global directly, exactly like this), then replay "Reset page
        // width" (no default chord — palette/double-click only) through a config
        // `[keys]` rebind, the real door a chord-driven replay has for a
        // defaultless command now that resolution lives inside the loop. The
        // sidecar's `page.measure` field reads this SAME global, so this is the
        // capture-level half of the reset (the config-file override removal is
        // App-only + unit-tested separately in `config/`). Holds the process-wide
        // page TEST_LOCK and restores it after, like every other page-global test.
        let _pg = crate::testlock::serial();
        crate::page::set_measure(40);
        let mut buffer = Buffer::scratch();
        let root = PathBuf::from("/tmp");
        let keys = keyspec::parse_keys("C-j").unwrap();
        let mut km = crate::keymap::KeymapState::with_overrides_and_convention(
            &[("reset_page_width".into(), vec!["C-j".into()])],
            crate::convention::Convention::Mac,
        );
        let _ = super::replay_keys_mode(
            crate::replay::Mode::Permissive,
            &mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None, &mut km,
        )
        .unwrap();
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
        // Same rebind door as the prose sibling above (palette-only command).
        let keys = keyspec::parse_keys("C-j").unwrap();
        let mut km = crate::keymap::KeymapState::with_overrides_and_convention(
            &[("reset_page_width".into(), vec!["C-j".into()])],
            crate::convention::Convention::Mac,
        );
        let _ = super::replay_keys_mode(
            crate::replay::Mode::Permissive,
            &mut buffer, &keys, &[], &root, None, &root, &Config::empty(), None, &mut km,
        )
        .unwrap();
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
        // DIFF-AS-PREVIEW: the capture-side preview resolver — the still-open
        // History overlay's highlighted row resolves to (id, TRANSCRIPT, counts):
        // the writer's diff of the current buffer vs that version, exactly what
        // the live App renders (the shared `history::diff_preview` owner). The
        // buffer here is "v2\n", so row 0 (v2, identical) is a titled folds-only
        // transcript with NO change marks; row 1 (v1, older) carries them.
        with_seeded_history(|p| {
            let buffer = Buffer::from_file(&p);
            let rows = crate::history::timeline_rows(
                &p,
                &buffer.text(),
                crate::history::now_millis(),
            );
            assert_eq!(rows.len(), 2, "two seeded versions");
            let mut ov = crate::overlay::OverlayState::new_history(rows, None, None);
            let (id, transcript, _counts) =
                history_preview_for(&ov, &buffer).expect("the newest row resolves");
            assert!(
                transcript.starts_with("# Comparing with "),
                "a titled diff transcript: {transcript}"
            );
            assert!(
                !transcript.contains("~~") && !transcript.contains("=="),
                "row 0 is identical to the buffer → no change marks: {transcript}"
            );
            assert_eq!(Some(id.as_str()), ov.selected_history_id());
            // Arrow down: the OLDER version's diff previews — its marks present.
            ov.move_sel(1);
            let (_, older, _) = history_preview_for(&ov, &buffer).expect("row 1 resolves");
            assert!(
                older.contains("~~") || older.contains("=="),
                "the highlighted row's diff carries change marks: {older}"
            );
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
            crate::keymap::KeymapState::new(),
            None, // no explicit --root
            None,
            notes_root,
            config,
            false, // permissive (the legacy default)
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
    fn capture_scenario_search_replace_replay_lands_in_the_sidecar_search_block() {
        // SIDECAR EVIDENCE for the shared search/replace routing: one `--keys`
        // spec drives open + query typing + replace-field reveal + replacement
        // typing + replace-one through the REAL `capture_screenshot` seam, and
        // every operation's outcome is assertable from the sidecar `search`
        // block + `text` — the round's done-criteria witness. Real disk +
        // capture -> hold the fs TEST_LOCK like the sticky-root test above.
        let _fs = crate::testlock::serial();
        let dir = std::env::temp_dir().join(format!("awl-search-replay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fixture = dir.join("doc.txt");
        std::fs::write(&fixture, "line one\nline two\nline three\n").unwrap();
        let out = dir.join("cap.png");
        let keys = keyspec::parse_keys("C-s l i n e Tab r o w Enter").unwrap();
        capture_screenshot(
            out.clone(),
            Some(fixture),
            CaptureOpts::default(),
            keys,
            crate::keymap::KeymapState::new_with_convention(crate::convention::Convention::Mac),
            Some(dir.clone()),
            None,
            dir.join("notes"),
            Config::empty(),
            true, // strict: the whole spec must ride real seams
        )
        .expect("capture succeeds");
        let json = std::fs::read_to_string(out.with_extension("json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let s = &v["search"];
        assert_eq!(s["query"].as_str().unwrap(), "line", "typed query");
        assert_eq!(s["active"].as_bool().unwrap(), true);
        assert_eq!(s["replace_active"].as_bool().unwrap(), true, "Tab revealed the row");
        assert_eq!(s["replacement"].as_str().unwrap(), "row", "typed replacement");
        assert_eq!(s["editing_replacement"].as_bool().unwrap(), true, "focus is in the field");
        assert_eq!(s["hit_count"].as_u64().unwrap(), 2, "one of three matches was replaced");
        assert!(
            v["text"].as_str().unwrap().starts_with("row one\nline two"),
            "replace-one swapped exactly the first match"
        );
        assert_eq!(v["cursor"]["line"].as_u64().unwrap(), 1, "caret advanced to the next match");
        assert_eq!(v["cursor"]["col"].as_u64().unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// USER-BUG LAW: changing the page measure must only reveal/occlude ONE
    /// already-authored lava backdrop. The pixels that remain exposed at both
    /// widths are therefore byte-identical, while pixels well inside the page
    /// stay one flat color. This renders through the real shader/uniform path;
    /// the pure `lava::tests` sibling alone cannot catch a bad upload or mask.
    #[test]
    fn lava_backdrop_pixels_are_page_width_invariant_and_page_interior_is_flat() {
        let _g = crate::testlock::serial();
        let old_theme = crate::theme::active_index();
        let old_measure = crate::page::measure();
        let old_page = crate::page::page_on();
        crate::theme::set_active_by_name("Mangrove").unwrap();
        crate::page::set_page_on(true);

        let dir = std::env::temp_dir().join(format!("awl-lava-width-law-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let render = |measure, stem: &str| {
            crate::page::set_measure(measure);
            let out = dir.join(format!("{stem}.png"));
            let opts = CaptureOpts {
                canvas: Some((1200, 800)),
                ..CaptureOpts::default()
            };
            capture_screenshot(
                out.clone(),
                None,
                opts,
                Vec::new(),
                crate::keymap::KeymapState::new(),
                Some(dir.clone()),
                Some(dir.clone()),
                dir.clone(),
                Config::empty(),
                false, // permissive (the legacy default)
            )
            .expect("lava width-law capture succeeds");
            let json: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(out.with_extension("json")).unwrap(),
            )
            .unwrap();
            let left = json["page"]["column"]["left"].as_f64().unwrap() as u32;
            let width = json["page"]["column"]["width"].as_f64().unwrap() as u32;
            (image::open(out).unwrap().to_rgba8(), left, left + width)
        };
        let (narrow, narrow_l, narrow_r) = render(40, "narrow");
        let (wide, wide_l, wide_r) = render(70, "wide");

        let left_full = narrow_l.min(wide_l).saturating_sub(crate::lava::MARGIN_GAP_PX as u32);
        let right_full = (narrow_r.max(wide_r) + crate::lava::MARGIN_GAP_PX as u32).min(1200);
        let mut compared = 0usize;
        for y in 80..720 {
            for x in (0..left_full).chain(right_full..1200) {
                assert_eq!(
                    narrow.get_pixel(x, y),
                    wide.get_pixel(x, y),
                    "common exposed backdrop changed at ({x},{y})"
                );
                compared += 1;
            }
        }
        assert!(compared > 50_000, "width law sampled a substantial common margin");

        let x0 = narrow_l.max(wide_l) + 64;
        let x1 = narrow_r.min(wide_r).saturating_sub(64);
        for (label, frame) in [("narrow", &narrow), ("wide", &wide)] {
            let flat = *frame.get_pixel(600, 650);
            for y in 600..720 {
                for x in x0..x1 {
                    assert_eq!(
                        *frame.get_pixel(x, y),
                        flat,
                        "{label} page is not flat at ({x},{y})"
                    );
                }
            }
        }

        crate::theme::set_active(old_theme);
        crate::page::set_measure(old_measure);
        crate::page::set_page_on(old_page);
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
        let out = capture::build_oracle(&buffer, &opts).map(|mut op| {
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
                Some(&mut op),
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
        let result = capture::build_oracle(&probe, &opts).map(|mut op| {
            // Step DOWN from (0,0) with goal-x 0 until the logical line changes;
            // `steps` C-n's cross into line 1, `steps-1` stay on line 0.
            let mut steps = 0usize;
            {
                let oracle = op.as_oracle();
                let (mut l, mut c) = (0usize, 0usize);
                loop {
                    let (nl, nc) = oracle.visual_line_down(l, c, 0.0, crate::caret::Affinity::Downstream);
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
            replay_keys(&mut b0, &keys_stay, &[], &root, None, &root, &Config::empty(), Some(&mut op));
            let stay = b0.cursor_line_col();
            // ...and the full count crosses into line 1's first visual row.
            let mut b1 = Buffer::from_str(LONG);
            let keys_cross = keyspec::parse_keys(&"C-n ".repeat(steps)).unwrap();
            replay_keys(&mut b1, &keys_cross, &[], &root, None, &root, &Config::empty(), Some(&mut op));
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
        let Some(mut op) = capture::build_oracle(&visual, &opts) else {
            crate::page::set_measure(crate::page::DEFAULT_MEASURE);
            eprintln!("skipping regression_non_wrapped byte-identical: no wgpu adapter");
            return;
        };
        replay_keys(&mut visual, &keys, &[], &root, None, &root, &Config::empty(), Some(&mut op));

        assert_eq!(
            visual.cursor_line_col(),
            logical.cursor_line_col(),
            "non-wrapped: visual motion must equal logical motion"
        );

        // Byte-identical captures: render both buffers and diff the PNG bytes.
        // PID-suffixed (not just `serial()`-guarded): `serial()` is a per-process
        // reentrant lock, so a SECOND concurrent `cargo test` process (e.g. a
        // parallel native + AWL_CONVENTION_FORCE=linux run) can't be excluded by
        // it — only a unique path can (mirrors every other temp-file test).
        let dir = std::env::temp_dir();
        let pid = std::process::id();
        let pv = dir.join(format!("awl_vl_visual_{pid}.png"));
        let pl = dir.join(format!("awl_vl_logical_{pid}.png"));
        capture::capture_with(&pv, &visual, &opts).expect("render visual");
        capture::capture_with(&pl, &logical, &opts).expect("render logical");
        let bv = std::fs::read(&pv).expect("read visual png");
        let bl = std::fs::read(&pl).expect("read logical png");
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
        assert_eq!(
            bv, bl,
            "non-wrapped short-line doc: visual + logical captures are byte-identical"
        );
        let _ = std::fs::remove_file(&pv);
        let _ = std::fs::remove_file(&pl);
    }

    // ---- FRESH LAYOUT ORACLE PER ACTION (Phase 2) --------------------------
    //
    // The replay loop re-shapes the oracle from the CURRENT buffer / zoom /
    // page-measure state before EVERY action (`OraclePipeline::refresh` — the
    // one freshness seam), mirroring the live window's between-keystrokes
    // re-sync. Each test below drives one staleness source end-to-end through
    // the REAL replay and FAILS on the pre-phase build-once oracle (which
    // shaped the pre-replay buffer exactly once). The per-source refresh
    // mechanics are unit-tested beside the seam (`capture::oracle::tests`).

    #[test]
    fn regression_edit_then_wrapped_motion_sees_fresh_wrap_geometry() {
        // THE known stale case this round retires: a spec that EDITS (wrapping
        // line 0) and then moves DOWN. The pre-phase oracle still held the
        // pre-replay shape (line 0 short, unwrapped), so C-n stepped straight
        // into logical line 1 at (1, 0); fresh per-action geometry lands on
        // line 0's SECOND visual row instead.
        let _g = crate::testlock::serial();
        crate::page::set_page_on(true);
        crate::page::set_measure(15);
        let mut buffer = Buffer::from_str("ab\ntail\n");
        let opts = CaptureOpts::default();
        let Some(mut op) = capture::build_oracle(&buffer, &opts) else {
            crate::page::set_measure(crate::page::DEFAULT_MEASURE);
            eprintln!("skipping regression_edit_then_wrapped_motion: no wgpu adapter");
            return;
        };
        // Type 30 chars at the head of line 0 (it now wraps at the 15-char
        // measure), return to the buffer start, then move DOWN one visual row.
        let mut spec: Vec<String> = "the quick brown fox jumps over"
            .chars()
            .map(|c| if c == ' ' { "Space".to_string() } else { c.to_string() })
            .collect();
        spec.push("s-Up".to_string()); // BufferStart (mac native)
        spec.push("C-n".to_string()); // NextLine
        let keys = keyspec::parse_keys(&spec.join(" ")).unwrap();
        let root = PathBuf::from("/tmp");
        replay_keys(&mut buffer, &keys, &[], &root, None, &root, &Config::empty(), Some(&mut op));
        let (line, col) = buffer.cursor_line_col();
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
        assert_eq!(line, 0, "Down follows the freshly-wrapped line 0 (stale geometry crossed into line 1)");
        assert!(col > 0, "landing on line 0's second visual row, got col {col}");
    }

    #[test]
    fn zoom_change_mid_replay_re_wraps_the_oracle_for_later_motion() {
        // With the column capped by the WINDOW (MAX_MEASURE), a bigger zoom
        // fits fewer chars per visual row — so Down after a replayed Cmd-+
        // must land at a strictly SMALLER column than the same Down at zoom
        // 1.0. The pre-phase oracle kept its build-time zoom, landing the two
        // replays identically.
        let _g = crate::testlock::serial();
        crate::page::set_page_on(true);
        crate::page::set_measure(crate::page::MAX_MEASURE);
        let text = format!("{}\ntail\n", "word ".repeat(80));
        let root = PathBuf::from("/tmp");
        let opts = CaptureOpts::default();

        let mut plain = Buffer::from_str(&text);
        let Some(mut op1) = capture::build_oracle(&plain, &opts) else {
            crate::page::set_measure(crate::page::DEFAULT_MEASURE);
            eprintln!("skipping zoom_change_mid_replay_re_wraps_the_oracle: no wgpu adapter");
            return;
        };
        let keys_plain = keyspec::parse_keys("C-n").unwrap();
        replay_keys(&mut plain, &keys_plain, &[], &root, None, &root, &Config::empty(), Some(&mut op1));
        let (l1, c1) = plain.cursor_line_col();

        let mut zoomed = Buffer::from_str(&text);
        let Some(mut op2) = capture::build_oracle(&zoomed, &opts) else {
            crate::page::set_measure(crate::page::DEFAULT_MEASURE);
            eprintln!("skipping zoom_change_mid_replay_re_wraps_the_oracle: no wgpu adapter");
            return;
        };
        let keys_zoom = keyspec::parse_keys("s-= C-n").unwrap();
        replay_keys(&mut zoomed, &keys_zoom, &[], &root, None, &root, &Config::empty(), Some(&mut op2));
        let (l2, c2) = zoomed.cursor_line_col();

        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
        assert_eq!((l1, l2), (0, 0), "both Downs stay on the wrapped line 0");
        assert!(c1 > 0 && c2 > 0, "both landed on a second visual row: {c1}, {c2}");
        assert!(
            c2 < c1,
            "the zoomed row holds fewer chars, so its wrap boundary is earlier: {c2} < {c1}"
        );
    }

    #[test]
    fn goto_switch_mid_replay_reshapes_the_oracle_to_the_arriving_buffer() {
        // The Goto arm swaps the ACTIVE buffer (and re-applies its sticky page
        // measure) mid-replay; a following Down must read the ARRIVING
        // buffer's wrap geometry. Launched on a CODE file (configured measure
        // 100 — b.md's long line would NOT wrap there), the switch to the
        // prose b.md re-applies measure 15 and swaps the text: both must reach
        // the oracle for Down to stay on b.md's wrapped line 0. The pre-phase
        // oracle stayed shaped on a.rs, so Down crossed into line 1 at (1, 0).
        let _fs = crate::testlock::serial();
        let dir = std::env::temp_dir().join(format!("awl-oracle-goto-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.join("b.md"), "the quick brown fox jumps over\ntail\n").unwrap();
        let cfg = Config {
            page_width_prose: Some(15),
            page_width_code: Some(100),
            ..Config::empty()
        };
        crate::page::set_page_on(true);
        crate::page::set_measure(100); // the launch file's own (code) measure
        let mut buffer = Buffer::from_file(&dir.join("a.rs"));
        let corpus = vec!["a.rs".to_string(), "b.md".to_string()];
        let opts = CaptureOpts::default();
        let Some(mut op) = capture::build_oracle(&buffer, &opts) else {
            crate::page::set_measure(crate::page::DEFAULT_MEASURE);
            let _ = std::fs::remove_dir_all(&dir);
            eprintln!("skipping goto_switch_mid_replay_reshapes_the_oracle: no wgpu adapter");
            return;
        };
        let keys = keyspec::parse_keys("s-o b . m d RET C-n").unwrap();
        replay_keys(&mut buffer, &keys, &corpus, &dir, None, &dir, &cfg, Some(&mut op));
        let (line, col) = buffer.cursor_line_col();
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(buffer.path(), Some(dir.join("b.md").as_path()), "the Goto switch landed on b.md");
        assert_eq!(line, 0, "Down follows b.md's line 0, wrapped at ITS re-applied measure");
        assert!(col > 0, "landing on line 0's second visual row, got col {col}");
    }

    /// LAW: the caret-MODE preference (an explicit pin, OR auto) must never be
    /// mutated by mere THEME movement — a COMMITTED round-trip switch through a
    /// one-bit world (Wagtail), or a theme-picker PREVIEW-and-Esc of one — is a
    /// true no-op on the caret global. Covers both suspects the caret-style-change
    /// bug report named: the 1-bit round's render-time override (`prepare_caret_
    /// layer` reads `crate::caret::mode()` but never writes it — this is the
    /// sticky round-trip proof of that) and auto-by-design (auto is legitimately
    /// theme-dependent, but a journey that ENDS back on the same world must
    /// resolve identically to never having left).
    #[test]
    fn caret_mode_survives_theme_journeys_committed_and_preview_esc() {
        let _g = crate::testlock::serial();
        let _t = crate::testlock::serial();
        let root = PathBuf::from("/tmp");
        // Cmd-T Wagtail Enter (commit) -> Cmd-T Gumtree Enter (commit) ->
        // Cmd-T Wagtail Esc (preview-then-cancel, reverting to Gumtree).
        let keys =
            keyspec::parse_keys("s-t W a g t a i l RET s-t G u m t r e e RET s-t W a g t a i l Esc")
                .unwrap();

        // AN EXPLICIT PIN survives the journey untouched.
        crate::theme::set_active_by_name("Gumtree").unwrap();
        crate::caret::set_mode(crate::caret::CaretMode::Block);
        let mut buf = Buffer::scratch();
        replay_keys(&mut buf, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert_eq!(crate::theme::active().name, "Gumtree", "the journey lands back on Gumtree");
        assert!(!crate::caret::is_auto(), "an explicit pin is never cleared by a theme journey");
        assert_eq!(crate::caret::mode(), crate::caret::CaretMode::Block);

        // AUTO survives the SAME journey too — no caret picker was ever opened,
        // so nothing should touch the override at all (the render-time one-bit
        // fallback in `prepare_caret_layer` only ever READS `caret::mode()`).
        crate::caret::clear_override();
        crate::theme::set_active_by_name("Gumtree").unwrap();
        let mut buf2 = Buffer::scratch();
        replay_keys(&mut buf2, &keys, &[], &root, None, &root, &Config::empty(), None);
        assert_eq!(crate::theme::active().name, "Gumtree");
        assert!(crate::caret::is_auto(), "a theme-only journey never pins auto");
        assert_eq!(
            crate::caret::mode(),
            crate::caret::CaretMode::Morph,
            "Gumtree (proportional) resolves Morph, exactly as if never visiting Wagtail"
        );

        crate::theme::set_active(crate::theme::DEFAULT_THEME);
        crate::caret::clear_override();
    }

    /// STATELESSNESS LAW: the DRAWN caret for mode `M` in world `W` is a pure
    /// function of `(M, W)` — never of the journey that got there. Proves it by
    /// rendering the identical settled `(mode, world)` twice — once landed on
    /// directly, once after a full COMMITTED Wagtail (one-bit) detour plus a
    /// theme-picker preview-and-Esc of Wagtail — and diffing the PNG bytes. This
    /// is the capture-level regression guard for suspect #1 (the 1-bit round's
    /// `prepare_caret_layer` Morph->Block override must stay a pure per-frame
    /// render decision, never leaking into the mode global or any pipeline
    /// Globals left set from Wagtail's own frame).
    #[test]
    fn caret_render_is_a_pure_function_of_mode_and_world_across_a_wagtail_detour() {
        let _g = crate::testlock::serial();
        let _t = crate::testlock::serial();
        let root = PathBuf::from("/tmp");
        let opts = CaptureOpts::default();
        let text = "hello frame\n";
        let detour_keys = keyspec::parse_keys(
            "s-t W a g t a i l RET s-t G u m t r e e RET s-t W a g t a i l Esc",
        )
        .unwrap();

        for mode in
            [crate::caret::CaretMode::Block, crate::caret::CaretMode::Morph, crate::caret::CaretMode::Ibeam]
        {
            // BASELINE: land directly on Gumtree + `mode`, no detour at all.
            crate::theme::set_active_by_name("Gumtree").unwrap();
            crate::caret::set_mode(mode);
            let base_buf = Buffer::from_str(text);
            let Some(_op) = capture::build_oracle(&base_buf, &opts) else {
                eprintln!(
                    "skipping caret_render_is_a_pure_function_of_mode_and_world_across_a_wagtail_detour: no wgpu adapter"
                );
                crate::theme::set_active(crate::theme::DEFAULT_THEME);
                crate::caret::clear_override();
                return;
            };
            // PID-suffixed: `serial()` only excludes other tests IN THIS SAME
            // process — a second concurrent `cargo test` process (e.g. a
            // parallel native + AWL_CONVENTION_FORCE=linux run) has its own
            // `serial()` and would clobber a fixed name (the ~1-in-3 flake
            // under a full parallel suite; 6/6 clean in isolation).
            let dir = std::env::temp_dir();
            let pid = std::process::id();
            let base_png = dir.join(format!("awl_caret_stateless_base_{mode:?}_{pid}.png"));
            capture::capture_with(&base_png, &base_buf, &opts).expect("baseline capture");

            // DETOUR: the SAME (mode, world), reached via a real committed
            // Wagtail visit + a theme-picker preview-of-Wagtail-then-Esc, all
            // through the real apply_core seam.
            crate::theme::set_active_by_name("Gumtree").unwrap();
            crate::caret::set_mode(mode);
            let mut detour_buf = Buffer::from_str(text);
            replay_keys(&mut detour_buf, &detour_keys, &[], &root, None, &root, &Config::empty(), None);
            assert_eq!(crate::theme::active().name, "Gumtree", "the detour lands back on Gumtree");
            assert_eq!(crate::caret::mode(), mode, "the detour never touched the pinned mode");
            let detour_png = dir.join(format!("awl_caret_stateless_detour_{mode:?}_{pid}.png"));
            capture::capture_with(&detour_png, &detour_buf, &opts).expect("detour capture");

            let b1 = std::fs::read(&base_png).expect("read baseline png");
            let b2 = std::fs::read(&detour_png).expect("read detour png");
            assert_eq!(
                b1, b2,
                "mode {mode:?}: caret pixels must be byte-identical whether or not Wagtail was visited in between"
            );
            let _ = std::fs::remove_file(&base_png);
            let _ = std::fs::remove_file(&detour_png);
        }

        crate::theme::set_active(crate::theme::DEFAULT_THEME);
        crate::caret::clear_override();
    }
}
