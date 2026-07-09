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

// The dispatch (`apply_core`) + its public seam (the [`LayoutOracle`] trait,
// [`ActionCtx`], [`Effect`]) stay in this module root; the cohesive clusters the
// dispatch leans on are carved into submodules and re-exported by bare name, exactly
// the precedent that split `render.rs` into `render/{caret,chrome,geometry,…}`. Each
// submodule pulls this root's items back in with its own `use super::*`.
mod edit; // the markdown smart-Enter edit (smart_newline + its pure decision)
mod flinch; // the caret-feedback triggers (impact_for / recoil_for)
mod format; // the markdown formatting-command toggles (block + inline)
mod motion; // the oracle-aware caret motions + page scroll + search open
mod overlay_nav; // the modal overlay intercept + browse-path helpers + live preview
mod rebind; // the game-style rebind-menu key handling
use edit::*;
use flinch::*;
use format::*;
use motion::*;
use overlay_nav::*;
use rebind::*;

// The overlay live-preview seam is shared with `app/input/mouse.rs`, where a HOVER over a
// picker row previews it exactly like a keyboard move (Theme re-tints, Caret swaps
// the look). Re-exported so the mouse path applies the identical preview.
pub(crate) use overlay_nav::preview_overlay;

// The palette/menu re-dispatch BREADCRUMB stamp is shared by both re-dispatch seams
// — the live `App::apply` `RunAction` handler and the headless `replay_keys` worklist
// — so an overlay opened by a palette-chosen command returns to the palette on a pop.
pub(crate) use overlay_nav::stamp_return_to;

/// A read-only LAYOUT ORACLE: the wrap-aware visual-row geometry that visual-line
/// motion needs, answered by whoever owns the SHAPED text — the GPU
/// [`crate::render::TextPipeline`] (live, in `app.rs`) and an offscreen-shaped
/// pipeline (headless, in `capture.rs`).
///
/// `apply_core` stays renderer-agnostic by asking THIS trait instead of reaching
/// into the pipeline directly, so the motion logic remains UNIFIED in `apply_core`
/// and shared by the live window and the `--keys` replay (awl's "both flows call
/// apply_core" principle). The pipeline keeps the GEOMETRY (it owns `visual_rows` /
/// `pick_row` / the per-char `xs`); the oracle returns MOTION-READY results.
///
/// All columns are CHAR columns and all `goal_x` / returned x values are pixels
/// RELATIVE TO THE TEXT'S LEFT EDGE (the same space the pipeline's `xs` live in),
/// so a goal-x read by [`Self::visual_x_of`] feeds straight back into
/// [`Self::visual_line_up`] / [`Self::visual_line_down`].
///
/// When the oracle is ABSENT (the pure `apply_core` unit tests, which own no
/// pipeline), motion falls back to the buffer's LOGICAL lines — so on a
/// non-wrapped document (and in those tests) behavior is identical to before.
/// Visual-line motion is the FLAT DEFAULT (no logical/visual toggle): every
/// caller that has a pipeline supplies an oracle, and `apply_core`'s vertical /
/// line-edge / kill-line motions consult it.
pub trait LayoutOracle {
    /// The cursor's pixel x on its own visual row (for the sticky goal-x).
    fn visual_x_of(&self, line: usize, col: usize) -> f32;
    /// One visual row UP from (`line`, `col`), landing the caret nearest `goal_x`.
    /// At the TOP visual row of the current logical line this crosses into the
    /// PREVIOUS logical line's LAST visual row; at the very top of the document it
    /// stays put.
    fn visual_line_up(&self, line: usize, col: usize, goal_x: f32) -> (usize, usize);
    /// One visual row DOWN from (`line`, `col`), landing nearest `goal_x`. At the
    /// BOTTOM visual row of the current logical line this crosses into the NEXT
    /// logical line's FIRST visual row; at the very bottom it stays put.
    fn visual_line_down(&self, line: usize, col: usize, goal_x: f32) -> (usize, usize);
    /// The start (first column) of the current VISUAL row.
    fn visual_line_start(&self, line: usize, col: usize) -> (usize, usize);
    /// The end (last column) of the current VISUAL row.
    fn visual_line_end(&self, line: usize, col: usize) -> (usize, usize);
}

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
    /// The visual-line motion LAYOUT ORACLE (the SHAPED text's wrap geometry),
    /// supplied by the live GPU pipeline (`app.rs`) and the headless offscreen
    /// pipeline (`capture.rs`) so the two flows can't drift. `None` in the pure
    /// `apply_core` unit tests (no pipeline), where motion falls back to LOGICAL
    /// lines. Consulted by the vertical (C-n/C-p, Up/Down), line-edge (C-a/C-e,
    /// Home/End) and kill-line (C-k) motions, which follow the SHAPED visual rows
    /// whenever it is present (the flat default).
    pub oracle: Option<&'a dyn LayoutOracle>,
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
    /// `Quit` (Cmd-Q): the caller exits the event loop, or stops the replay.
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
    /// caller. Now the SETTINGS MENU's "Edit config as text" ACTION row (the raw
    /// escape hatch), not the friendly default — [`OpenSettingsMenu`] is that.
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
    /// Go-to's HEADINGS lens accepted (Enter on a heading row): JUMP the cursor to
    /// document line `.0` (0-based). The fold that retired the standalone Outline
    /// picker — a heading row's accept is a cursor move, not a file open, so it rides
    /// its own effect rather than `OverlayAccept(Goto, …)` (which opens a path). The
    /// caller moves the cursor (live App + headless replay both); the core never
    /// touches the buffer here.
    JumpToLine(usize),
    /// REBIND MENU committed a capture: write `binding` into the `[keys]` SLOT of the
    /// command `slug` (the caller persists to config + live-reloads). `confirmed` is
    /// true when the user already accepted a CONFLICT warning (Confirm stage), so the
    /// caller must NOT re-gate on the clash. The core leaves the overlay open (the
    /// menu stays up); the caller refreshes its bindings + notice after the reload.
    RebindCommit {
        slug: String,
        binding: String,
        confirmed: bool,
    },
    /// REBIND MENU reset (Delete on a command): REMOVE `slug` from `[keys]` so its
    /// default applies again (the caller persists + live-reloads). The overlay stays open.
    RebindReset { slug: String },
    /// A discrete action was REQUESTED but could NOT PROCEED (a motion into a wall,
    /// a page that can't page further, an exhausted undo/redo, a delete with nothing
    /// to remove). The caller bumps the VISUAL caret in `dir` — away from the wall —
    /// via [`crate::caret::CaretAnim::recoil`]; the spring self-settles it back to
    /// rest. The buffer/cursor are UNCHANGED (that's the whole point — it's a
    /// blocked action), so a settled capture stays byte-identical; the headless
    /// replay simply ignores it (no clock/animation). Mutually exclusive with the
    /// real effects: a recoil only arms when the action produced no other effect.
    Recoil(crate::caret::RecoilDir),
    /// PHASE 2 — TYPING IMPACT: a character was SUCCESSFULLY inserted. The caller
    /// flinches the VISUAL caret (a squash-pop + a velocity back-kick against the type
    /// direction) in every caret look via [`crate::caret::CaretAnim::type_impact`]; the
    /// spring settles it back to the SAME rest, so a settled capture is byte-identical
    /// and the headless replay ignores it (no clock). The buffer is already mutated.
    TypeImpact,
    /// PHASE 2 — DELETION SQUASH: a backspace / C-d SUCCESSFULLY removed a character.
    /// The caller squashes the caret INWARD (it swallows what it ate) via
    /// [`crate::caret::CaretAnim::delete_squash`]. Live-only, byte-identical settled.
    DeleteSquash,
    /// PHASE 2 — KILL-LINE GULP: a C-k SUCCESSFULLY killed (part of) a line. The caller
    /// pulses a BIGGER caret gulp via [`crate::caret::CaretAnim::gulp`]. Live-only,
    /// byte-identical settled. Like the squash, mutually exclusive with the recoil (a
    /// no-op kill changes nothing → no gulp; only a real edit flinches).
    Gulp,
    /// PHASE 3 — ENTER JUICE / LINE LANDING: Enter SUCCESSFULLY inserted a newline
    /// (including the markdown smart-Enter continue/end-block edits). The caller
    /// gives the VISUAL caret a caret-level "touchdown" squash via
    /// [`crate::caret::CaretAnim::line_land`] — CARET-LEVEL ONLY (no content
    /// reflow / row animation; rows never dance). Live-only, byte-identical
    /// settled; the headless replay ignores it (no clock).
    LineLand,
    /// C-x #: the core already SAVED the buffer (identically to [`Action::Save`]).
    /// The caller notifies any daemon `--wait` client waiting on this buffer (a
    /// live-App-only concern — the pure core can't reach the socket) and switches
    /// to the previously-open buffer (the same swap `Effect::LastBuffer` performs).
    /// Headless replay treats this exactly like `LastBuffer` — a no-op (no daemon,
    /// no 2-deep history in a one-shot replay).
    FinishBuffer,
    /// THE CONSCIOUS MARK ("Keep version"): record the current buffer as a
    /// PINNED, prune-EXEMPT local-history snapshot. The pure core can't reach the
    /// store (no fs / config / buffer path), so it signals this for the live App to
    /// perform ([`crate::app::App::keep_version`] → [`crate::history::record_pinned`]).
    /// LIVE-APP-ONLY: the headless `--keys` replay no-ops it (the history determinism
    /// gate — a capture never touches the store), so a settled frame stays
    /// byte-identical.
    KeepVersion,
    /// C-c C-o (follow-link-at-point): the caret sat inside a markdown link, whose
    /// destination URL is carried here for the caller to open in the OS default
    /// browser (a user-initiated handoff — the app never fetches it, so the
    /// zero-network invariant holds). LIVE-APP-ONLY: `App::follow_link` performs the
    /// `open`/`xdg-open`/`window.open` launch; the headless `--keys` replay no-ops it
    /// (a capture must never spawn a browser), so a settled frame stays byte-identical.
    /// A caret OUTSIDE every link never produces this effect (`Effect::None`, the calm
    /// no-op) — `Action::FollowLink` only arms it when `markdown::link_at` is `Some`.
    FollowLink(String),
    /// COPY PULSE: M-w / Cmd-C successfully copied a NON-EMPTY selection into the
    /// kill ring — copy's one common but otherwise INVISIBLE result finally gets an
    /// in-world confirmation. The caller plays a gentle caret kick
    /// ([`crate::caret::CaretAnim::copy_pulse`], distinct from every edit flinch —
    /// nothing was edited) AND brightens the selection quad's own tint, decaying
    /// back over the live clock (`TextPipeline::copy_pulse`) — "obvious and
    /// understated", never amber. Unlike the edit flinches this never touches the
    /// buffer content, so it can't ride `impact_for`'s version-changed gate; see
    /// `copy_pulse_for`. Live-only, byte-identical settled: the headless replay
    /// ignores it (no clock), and `has_selection() == false` (an empty-selection
    /// copy) never arms it — that stays the documented no-op.
    ///
    /// DESIGN CALL, logged: `DESIGN.md` §3 states "the caret is the only thing
    /// allowed juice… selection, errors: Calm, geometric, precise. No juice." This
    /// round is a deliberate, user-approved, NARROW exception — the selection
    /// brightens only as a direct, one-shot REACTION to the caret's own copy
    /// action (never ambient, never idle chrome), and decays back to the exact
    /// same calm rendering within `COPY_PULSE_MS`. Flagged here rather than
    /// silently widening the law; a future pass may want to fold this into an
    /// explicit `DESIGN.md` amendment (mirroring the WYSIWYG conceal-on-cursor
    /// round's own "settled 2026-07" `PHILOSOPHY.md` amendment) rather than
    /// leaving it as an unstated one-off.
    CopyPulse,
    /// SETTINGS MENU: Enter on a TOGGLE row (page mode / wysiwyg / spellcheck / …).
    /// The core can't flip a process-global-and-persist (no config path / GPU), so it
    /// signals the sticky `key` back for the caller to (a) flip the live global +
    /// re-render this frame, (b) `persist_pref` the negated value into `config.toml`,
    /// and (c) refresh the STILL-OPEN menu's value cell (`App::setting_toggle`). The
    /// core leaves the overlay open (the menu stays up); the `key` is the config key
    /// from [`crate::settings::toggle_key`]. Headless replay reflects nothing (the
    /// capture path has no live global setter / config write) — a no-op there.
    SettingToggle { key: String },
    /// SETTINGS MENU: Enter COMMITTED an inline VALUE edit (page widths / zoom). The
    /// core built + committed the typed `value` for config `key`; it can't parse-clamp-
    /// apply-persist (no config path / GPU / zoom owner), so it signals the raw typed
    /// string back for the caller to parse + clamp (`settings::clamp_page_width` /
    /// `settings::parse_zoom`), apply LIVE (`page::set_measure` via `sync_page_measure`
    /// / `set_zoom`), persist the NAMED key, and refresh the still-open menu's cell
    /// (`App::setting_value_commit`). The core already cleared the value-edit sub-state
    /// (the menu stays open). Headless replay reflects nothing (no live setter / config).
    SettingValueCommit { key: String, value: String },
    /// SETTINGS MENU: a PATH row's folder NAVIGATOR accepted a folder. The core routed
    /// the pick back (the navigator's Enter, when it carried a `setting_path_key`); the
    /// caller writes the named `key` to config format-preservingly and — for
    /// `project_root` — additionally re-scopes the active project
    /// (`App::setting_path_pick`), then the menu is already re-summoned via the
    /// `return_to` breadcrumb. Headless replay reflects nothing (live-App-only).
    SettingPathPick { key: String, path: String },
    /// ASSET CLEANER: Enter on an orphan row REQUESTED that its file (root-relative
    /// `rel`) be moved to the OS Trash. The pure core can't reach the Trash / the
    /// filesystem (no root, no [`crate::assets::TrashCan`]), so it signals `rel` back
    /// for the live App to (a) trash `self.root.join(rel)` via the trash seam and (b),
    /// on success, REMOVE that row from the still-open picker
    /// ([`crate::overlay::OverlayState::remove_asset_row`]) — the picker stays open. The
    /// core leaves the overlay OPEN and does NOT remove the row (the determinism gate:
    /// a headless `--keys` replay no-ops this effect, so its orphan list stays whole
    /// and the sidecar never claims a file was trashed that wasn't). A trash FAILURE
    /// leaves the row + shows a calm notice. LIVE-APP-ONLY; a default `--screenshot`
    /// never reaches it (the command is summon-by-name).
    TrashAsset { rel: String },
}

/// Apply one resolved `action` to the editor core. `shift` is whether Shift was
/// held (so a motion extends the selection, Shift+Arrow style). Returns the one
/// deferred [`Effect`] the action signals back to the caller (`Effect::None` for
/// the common case) — the caller carries out the filesystem/window/quit work the
/// pure core can't. Mutates only what `ActionCtx` exposes; no GPU, window, or
/// clipboard.
pub fn apply_core(ctx: &mut ActionCtx, action: &Action, shift: bool) -> Effect {
    // Serializes this whole action against any other thread's global-touching
    // test, under test only (see [`crate::testlock`]): `about_open()` /
    // `lifetime_open()` are read unconditionally just below, for every action, so
    // a concurrently-running test that flips one (only `Action::About` /
    // `Action::LifetimeStats` ever do) could otherwise leak its state into a
    // totally unrelated test's action, changing its returned `Effect`. It is the
    // ONE reentrant guard, so a test that already holds it around its own drive
    // nests here for free, and there is no lock ORDER left to ABBA (the page
    // writers acquire the SAME guard, reentrantly). Held for the whole function;
    // zero cost outside `cfg(test)`.
    #[cfg(test)]
    let _test_guard = crate::testlock::serial();

    // PLATFORM-SCOPED COMMANDS: the DISPATCH gate. Hiding a command from the palette
    // / rebind menu / menu bar (`commands::visible`) is not enough on its own — a
    // still-configured keymap CHORD (native or emacs; e.g. Cmd-Q for Quit) reaches
    // `apply_core` directly, bypassing every picker, and a stray `Effect::RunAction`
    // re-dispatch could in principle name a hidden action too. This is the BELT: any
    // action unavailable on `commands::Platform::current()` (`commands::action_available`)
    // is a calm, total no-op RIGHT HERE, before it can touch the buffer, open an
    // overlay, or signal an effect the caller would act on. Native is a single `==`
    // branch that always returns available (nothing is ever gated on the desktop
    // build); web is a small bounded scan of the ~60-entry catalog, no allocation.
    if !crate::commands::action_available(action, crate::commands::Platform::current()) {
        return Effect::None;
    }

    // MODAL CARD DISMISSAL (About / Lifetime stats). While either summoned card is
    // open it OWNS the very next key — ANY key closes it and is otherwise consumed
    // (no other effect), mirroring the "any key/click dismisses" spec rather than
    // the navigation overlay's narrower Esc/Enter contract (a card has nothing to
    // navigate). ONE owner of the check+close (`card::dismiss_summoned_card`),
    // shared verbatim with the live App's mouse-press handler. Checked BEFORE the
    // overlay intercept: the two cards are never open at once, nor with an overlay
    // (each opens via `Effect::RunAction` after the palette that summoned it has
    // already closed).
    if crate::card::dismiss_summoned_card() {
        return Effect::None;
    }

    // OVERLAY INTERCEPT. When the summoned navigation overlay is open it OWNS
    // every key (printable chars filter the query, Up/Down move the selection,
    // Right/Left descend/ascend the explorers, Enter accepts, Esc/C-g cancels);
    // routing it through the shared core is what makes the overlay `--keys`-
    // drivable. The modal dispatch lives in [`overlay_nav::overlay_intercept`].
    if ctx.overlay.is_some() {
        return overlay_intercept(ctx, action);
    }

    // SEARCH PANEL single-key REPLACE toggle. While a search is live, a bare Tab
    // reveals the replace field (first press) then flips focus between the query
    // and replacement fields — the SAME single-key affordance `handle_search_key`
    // gives the windowed editor (where in-panel keys never reach `apply_core`),
    // mirrored here so a `--keys "C-s <Tab>"` replay drives it and the sidecar's
    // `replace_active` is assertable WITHOUT the Cmd-Option-F chord. Routed
    // through the core like the overlay keys above; it intercepts ONLY Tab (the
    // panel's query typing still arrives via `--search`, the documented headless
    // input gap), so every other action falls through unchanged.
    if ctx.search.is_some() {
        match action {
            // Tab is the one FIELD-SWITCH key: flip focus find↔replace (revealing the
            // replace row the first time). `--keys "C-s <Tab>"` drives it headlessly.
            Action::InsertTab => {
                if let Some(st) = ctx.search.as_mut() {
                    st.toggle_replace();
                }
                return Effect::None;
            }
            // Cmd-R while the panel is ALREADY open focuses the replace field (the
            // fresh open revealed it with focus on find; a second Cmd-R jumps in).
            // Mirrors `handle_search_key`'s live Cmd-R, so `--keys "C-s Cmd-r"` drives
            // it and the sidecar's `replace_active` / focus is assertable.
            Action::OpenReplace => {
                if let Some(st) = ctx.search.as_mut() {
                    st.focus_replacement();
                }
                return Effect::None;
            }
            _ => {}
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

    // RECOIL PRIMITIVE — snapshot the pre-action state so we can detect a BLOCKED
    // action (one that couldn't proceed) AFTER the match and bump the caret away
    // from the wall. Cheap scalars: the cursor char index (a motion that hit a wall
    // leaves it unchanged), the content version (a no-op delete never bumps it), and
    // whether undo/redo had anything to do. See `recoil_for`.
    let cursor_before = ctx.buffer.cursor_char();
    let version_before = ctx.buffer.version();
    let could_undo = ctx.buffer.can_undo();
    let could_redo = ctx.buffer.can_redo();
    // COPY PULSE snapshot: whether a NON-EMPTY selection existed BEFORE dispatch.
    // `Buffer::copy_region` unconditionally clears the mark (even on a no-op copy
    // with nothing selected), so reading `has_selection()` AFTER the call would
    // always read false — this has to be taken here, alongside the other
    // pre-action snapshots. See `copy_pulse_for`.
    let had_selection_before = ctx.buffer.has_selection();

    let mut effect = Effect::None;
    match action {
        Action::ForwardChar => ctx.buffer.forward_char(),
        Action::BackwardChar => ctx.buffer.backward_char(),
        // The motions with a VISUAL-ROW analogue route through `vertical_motion` /
        // `line_edge_motion`, which follow the SHAPED visual rows via the layout
        // oracle (the FLAT DEFAULT — no logical/visual toggle). With no oracle (the
        // pure unit tests) they fall back to the buffer's LOGICAL lines, so on a
        // NON-wrapped document visual == logical and behavior is unchanged.
        Action::NextLine => vertical_motion(ctx, true),
        Action::PreviousLine => vertical_motion(ctx, false),
        Action::LineStart => line_edge_motion(ctx, false),
        Action::LineEnd => line_edge_motion(ctx, true),
        Action::ForwardWord => ctx.buffer.forward_word(),
        Action::BackwardWord => ctx.buffer.backward_word(),
        Action::BufferStart => ctx.buffer.buffer_start(),
        Action::BufferEnd => ctx.buffer.buffer_end(),
        Action::InsertChar(c) => ctx.buffer.insert_char(*c),
        // MARKDOWN smart Enter: continue a list / blockquote (ordered lists
        // AUTO-INCREMENT), END the block on an empty item (strip the dangling
        // marker), or carry leading indentation forward. Pure + `--keys`-drivable
        // (reads only the current line + cursor, edits via the buffer's atomic
        // seam). A non-markdown buffer — or any line the helper declines — falls
        // through to a plain newline, byte-identical to before.
        Action::Newline => {
            if !smart_newline(ctx) {
                ctx.buffer.insert_newline();
            }
        }
        // TAB: indent a markdown list item one level (across a selection), else a soft
        // tab. SHIFT-TAB: outdent one level (clamped), or strip leading spaces off a
        // list. Both flow through the buffer's atomic edit seam (one undo step) and are
        // `--keys`-drivable; the list-vs-plain gate is `list_tab`.
        Action::InsertTab => list_tab(ctx),
        Action::Outdent => list_outdent(ctx),
        Action::DeleteBackward => ctx.buffer.delete_backward(),
        Action::DeleteWordBackward => ctx.buffer.delete_word_backward(),
        Action::DeleteWordForward => ctx.buffer.delete_word_forward(),
        Action::DeleteToLineStart => ctx.buffer.delete_to_line_start(),
        Action::DeleteForward => ctx.buffer.delete_forward(),
        Action::KillLine => kill_line_motion(ctx),
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
        // Cmd-A: select the WHOLE buffer — mark at document start, point at
        // document end, so every existing region op (C-w cut, M-w copy, a
        // delete/backspace, or typing a char) then acts on the entire doc. A
        // no-op empty region on an empty buffer (no panic). Drop any transient
        // Shift-selection flag: this is a discrete, sticky region, not a
        // Shift-extend.
        Action::SelectAll => {
            ctx.buffer.select_all();
            *ctx.shift_selecting = false;
        }
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
        // Cmd-R (or the legacy Cmd-Option-F): open the SAME isearch panel with the
        // labeled REPLACE row revealed — but focus stays on the FIND field so you
        // type the needle first (Cmd-R again / Tab moves into the replacement). While
        // a search is already live this arm is unreachable — the search intercept
        // above focuses the replace field instead — so both doors are `--keys`-drivable.
        Action::OpenReplace => {
            start_search(ctx, Direction::Forward);
            if let Some(st) = ctx.search.as_mut() {
                st.reveal_replace();
            }
        }
        // Toggling the caret look is a pure render concern (no buffer change). The
        // process-global flip lives HERE on the shared seam, so BOTH the windowed
        // `App::apply` and the headless `--keys` replay toggle through one place
        // (no double-toggle); `App` then does only the window-side follow-up (the
        // stderr log) as a post-`apply_core` side effect. A palette-run capture
        // (Cmd-P → "Toggle caret style") renders — and records in its sidecar — the
        // toggled mode (Block ⇄ I-beam).
        Action::ToggleCaretMode => {
            crate::caret::toggle_mode();
        }
        // Toggling page mode is a pure render/layout concern (no buffer change). The
        // process-global flip lives HERE on the shared seam (like the caret toggle);
        // `App::apply` does the GPU re-wrap + view resync the core can't reach as a
        // post-`apply_core` side effect. A palette-run capture (Cmd-P → "Toggle page
        // mode") renders (and records in its sidecar) the toggled state.
        Action::TogglePageMode => {
            crate::page::toggle();
        }
        // Page WIDER / NARROWER: adjust the centered writing column's MEASURE (the
        // settable page width) by a step, clamped to the usable band. Zoom-independent
        // — this resizes the PAGE, not the glyphs — so it lives on the shared seam like
        // the page toggle. `App::apply` does the GPU re-wrap + view resync + sticky
        // persist afterwards (a post-`apply_core` side effect the core can't reach). A
        // palette-run capture (Cmd-P → "Widen page") renders + records the new measure.
        Action::PageWider => {
            crate::page::widen();
        }
        Action::PageNarrower => {
            crate::page::narrow();
        }
        // RESET PAGE WIDTH: snap the measure back to the ACTIVE buffer's OWN
        // built-in default (`PageClass::default_measure` — 70 prose / 100 code) —
        // "there's no easy way back" once you've widened/narrowed/dragged. Reads
        // `ctx.buffer.page_class()` so a `.rs` file resets to 100, never a stray
        // 70. Pure process-global reset on the shared seam, like the
        // wider/narrower arms above. `App::apply` does the GPU re-wrap + view
        // resync afterwards AND clears the sticky `page_width_prose`/
        // `page_width_code` config override matching the SAME class entirely
        // (format-preserving removal — the core has no config to write to) as a
        // post-`apply_core` side effect. A `--keys`-driven reset (no default
        // chord; palette/double-click only) renders — and records in its
        // sidecar — the reset measure.
        Action::PageReset => {
            crate::page::set_measure(ctx.buffer.page_class().default_measure());
        }
        // Toggling the DEBUG panel is a pure render concern (no buffer change), like
        // the caret / page toggles. The windowed `App::apply` intercepts this
        // to ALSO keep the redraw loop hot (so the live frametime line updates); the
        // headless replay path just flips the process-global so a `--debug` (or
        // palette-run) capture renders (and records in its sidecar) the toggled state — the
        // frametime line drawn as a fixed placeholder since the capture has no clock.
        Action::ToggleDebug => {
            crate::debug::toggle();
        }
        // Toggling the persistent MARGIN OUTLINE is a pure render concern (no buffer
        // change), exactly like the debug / page toggles: flip the
        // process-global here so a `--keys "Cmd-S-o"` capture renders (and records in
        // its sidecar) the toggled state; the live `App::apply` intercepts this to
        // ALSO persist the sticky pref + request a redraw. ON by default (flipped
        // 2026-07-09), so a default capture reports `outline.on: true`.
        Action::ToggleOutline => {
            crate::outline::toggle();
        }
        // Toggling the awl-RENDERED MENU BAR is a pure render concern (no buffer
        // change), exactly like the outline / debug / page toggles: flip the
        // process-global here so a `--keys "..."` capture renders (and records in its
        // sidecar `menubar` block) the toggled state; the live `App::apply` intercepts
        // this to ALSO persist the sticky pref + request a redraw. Default OFF on
        // macOS (the native bar is the door), ON on web/Linux, so a default macOS
        // capture reports `menubar.shown: false`.
        Action::ToggleMenuBar => {
            crate::menubar::toggle();
        }
        // Toggling TYPEWRITER SCROLL is a pure SCROLL concern (no buffer change),
        // like the outline toggle: flip the process-global here so a
        // `--keys` capture with typewriter on renders (and its sidecar `scroll_lines`
        // reports) the pinned centered scroll; the live `App::apply` intercepts this
        // to ALSO persist the sticky pref + re-pin the caret row. OFF by default, so a
        // default capture keeps the cursor-follow scroll.
        Action::ToggleTypewriter => {
            crate::typewriter::toggle();
        }
        // Summon the held STATS HUD. This is a HELD key, not a toggle: the press
        // SETS the process-global true, and the live window clears it on the matching
        // key RELEASE (`App::on_key_release`). A headless `--keys "Cmd-I"` replay has
        // no release, so it leaves the HUD held for the single captured frame — the
        // settled-state render of an in-motion peek, like the other render globals.
        // Render-only (no buffer change); `App::apply` keeps the redraw loop hot.
        Action::ShowStatsHud => {
            crate::hud::set_held(true);
        }
        // OPEN the summoned About card (name/version/world/end-mark). Stays
        // open until this same function's top-of-function intercept consumes
        // the next key (or the live App's mouse-press handler closes it on a
        // click — `app/input/mouse.rs`). Render-only (no buffer change).
        Action::About => {
            // Re-take the lock the top intercept released (see the SCOPE note): the
            // open-flag WRITE stays serialized against a concurrent reader, but only
            // for this leaf arm — never across a page writer. Reentrant for a test
            // holding it around its own drive.
            #[cfg(test)]
            let _g = crate::testlock::serial();
            crate::about::set_open(true);
        }
        // OPEN the summoned Lifetime stats card (the personal odometer). Stays
        // open until this same function's top-of-function intercept consumes the
        // next key (or the live App's mouse-press handler closes it on a click —
        // `app/input/mouse.rs`). Render-only (no buffer change). See `lifetime.rs`.
        Action::LifetimeStats => {
            // Re-take the lock the top intercept released (see the SCOPE note),
            // scoped to this leaf arm — never held across a page writer. Reentrant.
            #[cfg(test)]
            let _g = crate::testlock::serial();
            crate::lifetime::set_open(true);
        }
        // Toggle the active buffer's line-ending discipline (LF <-> CRLF). The rope
        // is byte-identical (always pure `\n`); only the on-disk encoding a save
        // restores differs, so this is document-level METADATA, not an undoable
        // edit (Cmd-Z does not restore it — see `Buffer::set_eol`). A real switch
        // marks the buffer dirty + bumps `version` so autosave rewrites with the
        // new ending on the next flush.
        Action::ConvertLineEndings => {
            let next = ctx.buffer.eol().toggled();
            ctx.buffer.set_eol(next);
        }
        // ALIGN TABLE: re-pad the GFM table under the caret so its `|` line up
        // (Prettier-style monospace alignment of the SOURCE — awl never draws a
        // grid). Find the table block around the caret line, re-emit it via
        // `markdown::align_table`, and replace it as ONE undoable edit; a calm
        // no-op when the caret is not in a table (or the table is already aligned,
        // so Cmd-Z stays meaningful). Pure `markdown` core + the buffer's atomic
        // replace seam, so `--keys "..."` drives it and the result is assertable.
        Action::AlignTable => align_table_at_cursor(ctx),
        // MARKDOWN FORMATTING COMMANDS: pure toggle transforms (block prefix / inline
        // wrapper) applied as ONE undoable edit through `Buffer::apply_format`. Each is
        // a TOGGLE (apply when absent on the target, strip when present) and markdown-
        // only (a calm no-op on a `.rs`/`.txt` buffer); the pure transforms live in
        // `actions::format`, so `--keys` drives them and the sidecar reflects the result.
        Action::ToggleBlockquote => apply_block_format(ctx, format::BlockKind::Blockquote),
        Action::ToggleBulletList => apply_block_format(ctx, format::BlockKind::Bullet),
        Action::ToggleNumberedList => apply_block_format(ctx, format::BlockKind::Numbered),
        Action::ToggleTaskList => apply_block_format(ctx, format::BlockKind::Task),
        Action::ToggleHeading => apply_block_format(ctx, format::BlockKind::Heading),
        Action::ToggleCodeBlock => apply_block_format(ctx, format::BlockKind::CodeBlock),
        Action::Bold => apply_inline_format(ctx, format::InlineKind::Bold),
        Action::Italic => apply_inline_format(ctx, format::InlineKind::Italic),
        Action::InlineCode => apply_inline_format(ctx, format::InlineKind::InlineCode),
        Action::Highlight => apply_inline_format(ctx, format::InlineKind::Highlight),
        Action::Strikethrough => apply_inline_format(ctx, format::InlineKind::Strikethrough),
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
        // "Recent projects…" (palette + File menu): open the SWITCH-PROJECT navigator
        // pre-lensed onto its RECENT lens — the fold that retired the standalone
        // RecentProjects picker. Same door as `OpenProject` (the navigator needs a
        // directory LEVEL, so it builds via `browse_to`, not `make_overlay`), then the
        // lens is focused to `recent` so it opens showing the recent-projects MRU. No
        // workspace yields None → a quiet no-op, exactly like `OpenProject`.
        Action::OpenRecentProjects => {
            let mut ov = (ctx.browse_to)(crate::overlay::OverlayKind::Project, None);
            if let Some(o) = ov.as_mut() {
                o.focus_facet_id("recent");
            }
            *ctx.overlay = ov;
        }
        // Summon the THEME PICKER (all worlds, fuzzy-filterable, live preview).
        // The caller's `make_overlay` builds it with the world names + the active
        // index (remembered for revert-on-cancel). It opens highlighting the
        // current world, so the open frame previews exactly the active theme.
        Action::OpenThemeMenu => {
            *ctx.overlay = (ctx.make_overlay)(crate::overlay::OverlayKind::Theme);
        }
        // Summon the CARET-STYLE PICKER (the three looks + descriptions, with a live
        // animated preview). The caller's `make_overlay` builds it with the looks +
        // the active one (remembered for revert-on-cancel); it opens highlighting the
        // current look, so the open frame previews exactly the active caret style.
        Action::OpenCaretMenu => {
            *ctx.overlay = (ctx.make_overlay)(crate::overlay::OverlayKind::Caret);
        }
        // Summon the DICTIONARY PICKER (the three bundled variants + descriptions,
        // NO live preview — see `overlay/`'s Dictionary doc). Opens highlighting
        // the currently-active variant.
        Action::OpenDictionaryMenu => {
            *ctx.overlay = (ctx.make_overlay)(crate::overlay::OverlayKind::Dictionary);
        }
        // Toggling spellcheck is a pure render/detection concern (no buffer change).
        // The process-global flip lives HERE on the shared seam (like the page/caret
        // toggles); `App::apply` persists the sticky pref + forces an immediate
        // rescan as a post-`apply_core` side effect the core can't reach. A
        // `--keys "..."` capture renders (and records in its sidecar) the toggled
        // state — every `misspellings_for`/`suggest_at` call already reads the
        // global fresh, so the flip is visible with no extra plumbing headlessly.
        Action::ToggleSpellcheck => {
            crate::spell::toggle();
        }
        // Toggling WRITING NITS is a pure render concern (no buffer change), exactly
        // like the spellcheck toggle: flip the process-global here so a `--keys`
        // capture renders (and its sidecar reflects) the toggled state — every nit
        // proto rebuilds from `nits::nits_on()` each frame — and the live `App::apply`
        // intercepts this to ALSO persist the sticky pref + repaint. ON by default.
        Action::ToggleWritingNits => {
            crate::nits::toggle();
        }
        // Cmd-Shift-. : reveal/hide dotfiles in the active file picker. It only has
        // meaning while an overlay is open — and there, `overlay_intercept` handles
        // it BEFORE this match is ever reached (the `ctx.overlay.is_some()` early
        // return at the top of `apply_core`). With no picker open there is nothing to
        // toggle, so this is a calm no-op.
        Action::ToggleHiddenFiles => {}
        // Cmd-P: summon the COMMAND PALETTE (the named-command fuzzy list). The
        // caller's `make_overlay` builds it from `commands::COMMANDS`.
        Action::OpenCommandPalette => {
            *ctx.overlay = (ctx.make_overlay)(crate::overlay::OverlayKind::Command);
        }
        // Cmd-P → "Keybindings…": summon the GAME-STYLE REBIND MENU (the command
        // catalog in capture mode). Built by `make_overlay` from `commands::COMMANDS`,
        // exactly like the palette but opened to rebind rather than run.
        Action::OpenKeybindings => {
            *ctx.overlay = (ctx.make_overlay)(crate::overlay::OverlayKind::Keybindings);
        }
        // "Go to heading…" (palette): open GO-TO pre-lensed onto its HEADINGS lens —
        // the fold that retired the standalone Outline picker. `make_overlay` builds
        // the Go-to overlay with the doc's headings already folded in (its Headings
        // lens's corpus); focusing the `headings` lens opens it showing them. Over a
        // buffer with no headings the lens reads "no headings yet" (never a no-op —
        // the file list is still there behind the other lenses; also reachable via
        // ⌘O → ←/→).
        Action::OpenOutline => {
            let mut ov = (ctx.make_overlay)(crate::overlay::OverlayKind::Goto);
            if let Some(o) = ov.as_mut() {
                o.focus_facet_id("headings");
            }
            *ctx.overlay = ov;
        }
        // Cmd-`;`: summon the SPELL-SUGGESTION picker for the misspelled word at the
        // cursor. The caller's `make_overlay` resolves the word the cursor is on (or
        // adjacent to) + its corrections; if the cursor isn't on a flagged word it
        // returns None, so the open is a calm no-op. Enter then replaces the word.
        Action::OpenSpellSuggest => {
            *ctx.overlay = (ctx.make_overlay)(crate::overlay::OverlayKind::Spell);
        }
        // Cmd-Shift-H: summon the HISTORY TIMELINE picker for the current file. The
        // caller's `make_overlay` gathers the file's versions (via
        // `history::timeline_rows`); an empty history still opens (the calm "no
        // history yet" row), so this is never a silent no-op. Enter then RESTORES the
        // highlighted version into the buffer as an undoable edit.
        Action::OpenHistory => {
            *ctx.overlay = (ctx.make_overlay)(crate::overlay::OverlayKind::History);
        }
        // Cmd-P → "Clean unused assets…": summon the ASSET CLEANER. The caller's
        // `make_overlay` builds it from the scanned orphan list (`assets::scan`,
        // threaded via `BuildCtx::assets`); an empty list still opens (the calm "no
        // unused assets" row), so this is never a silent no-op. Enter then requests the
        // highlighted orphan be trashed (`Effect::TrashAsset`), keeping the picker open.
        Action::OpenAssetClean => {
            *ctx.overlay = (ctx.make_overlay)(crate::overlay::OverlayKind::Assets);
        }
        // "Keep version": THE CONSCIOUS MARK — record the current buffer as a
        // PINNED, prune-exempt snapshot. The core can't reach the store (fs/config/
        // path), so it signals the caller; the live App writes it, the headless
        // replay no-ops it (history determinism gate). See `Effect::KeepVersion`.
        Action::KeepVersion => {
            effect = Effect::KeepVersion;
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
        // Settings menu: summon the faceted settings overlay (the friendly default).
        // Built by `make_overlay` from the settings corpus + the gathered value
        // cells; it always summons (a non-empty static table).
        Action::OpenSettingsMenu => {
            *ctx.overlay = (ctx.make_overlay)(crate::overlay::OverlayKind::Settings);
        }
        // C-x #: SAVE the buffer (the SAME `Buffer::save` call `Action::Save` makes)
        // then signal the caller to notify daemon waiters + switch to the
        // previously-open buffer. The caller (`App::finish_buffer`) mirrors
        // `Action::Save`'s history-snapshot + mtime bookkeeping itself, BEFORE the
        // buffer swap — `post_apply_effects` runs after this effect and would
        // otherwise stamp the wrong (just-switched-to) buffer. The core can't reach
        // the daemon socket or the 2-deep buffer history itself.
        Action::FinishBuffer => {
            if let Err(e) = ctx.buffer.save() {
                eprintln!("save failed: {e}");
            } else if let Some(p) = ctx.buffer.path() {
                eprintln!("wrote {}", p.display());
            }
            effect = Effect::FinishBuffer;
        }
        // C-c C-o: FOLLOW the markdown link under the caret. Extract its URL from
        // the parsed spans ([`crate::markdown::link_at`], a pure function of the
        // text + caret BYTE offset); a link → signal the URL back for the caller to
        // open in the browser, a caret outside every link → a calm no-op
        // (`Effect::None`). The core never opens anything itself (no window/process
        // reach) — the live App performs the OS handoff, the headless replay no-ops.
        Action::FollowLink => {
            if let Some(url) = crate::markdown::link_at(&ctx.buffer.text(), ctx.buffer.cursor_byte())
            {
                effect = Effect::FollowLink(url);
            }
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

    // RECOIL PRIMITIVE — if the action produced no other effect, see whether it was
    // BLOCKED (couldn't proceed) and, if so, arm a caret bump away from the wall.
    // Mutually exclusive with the real effects (a blocked action never sets one), so
    // we only test when `effect` is still `None`.
    if effect == Effect::None {
        if let Some(dir) = recoil_for(action, ctx, cursor_before, version_before, could_undo, could_redo) {
            effect = Effect::Recoil(dir);
        }
    }
    // DELETION SQUASH + TYPING IMPACT (PHASE 2) — if the action produced no other
    // effect AND it was a SUCCESSFUL edit (the content version actually bumped), arm
    // the caret FLINCH for the edit. Mutually exclusive with the blocked-action recoil
    // above (a no-op delete recoils away from the wall; a REAL delete squashes inward),
    // so we only test when `effect` is still `None`.
    if effect == Effect::None {
        if let Some(imp) = impact_for(action, version_before, ctx) {
            effect = imp;
        }
    }
    // COPY PULSE — a successful M-w/Cmd-C copy of a NON-EMPTY selection: arm the
    // caret kick + selection-tint brighten/decay. Never touches buffer content, so
    // it can't ride `impact_for`'s version-changed gate above; a separate check
    // against the PRE-action selection snapshot (`copy_region` always clears the
    // mark, even on a no-op). Mutually exclusive with the other effects by
    // construction (`Action::CopyRegion` never recoils or flinches), so gating on
    // `effect == Effect::None` here is a formality that keeps the same shape as
    // the recoil/impact cascade above.
    if effect == Effect::None {
        if let Some(e) = copy_pulse_for(action, had_selection_before) {
            effect = e;
        }
    }
    effect
}

#[cfg(test)]
mod tests;
