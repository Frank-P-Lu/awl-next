//! Windowed mode: open a winit window, create a wgpu surface, and run the
//! interactive editor. Keyboard events flow through the keymap into the buffer,
//! and every change triggers a redraw of the shared text pipeline.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
// `Instant` is the live editor's monotonic wall-clock (spring dt, debounces, the
// session timer); std's `Instant::now()` PANICS on wasm32, so it comes from
// `crate::clock` (std on native — byte-identical; `web-time` on wasm). The
// notes-recency `SystemTime` stamp `apply` reads is a wasm-SAFE clock read via
// `crate::clock::system_now()`. Never reach for raw `std::time::…::now()` here.
use crate::clock::Instant;

// OS clipboard bridge. Native = arboard (the real platform clipboard). wasm =
// a no-op stub: the browser clipboard is an async, permission-gated API that
// doesn't fit arboard's sync surface (and arboard itself won't compile for
// wasm32), so the web build runs on the internal kill-ring only. The stub's
// `new()` always Errs, so `App::new` stores `None` and the mirror paths no-op —
// exactly the graceful-degradation path a headless/no-display native run takes.
#[cfg(not(target_arch = "wasm32"))]
use arboard::Clipboard;
#[cfg(target_arch = "wasm32")]
use web_clipboard::Clipboard;

#[cfg(target_arch = "wasm32")]
mod web_clipboard {
    /// No-op clipboard stub for the browser build. Mirrors the slice of arboard's
    /// API `app.rs` uses (`new`/`set_text`/`get_text`), each failing quietly so the
    /// editor degrades to its internal kill-ring (the same path native takes when
    /// no system clipboard is available). A real async Clipboard-API bridge is
    /// future work (Phase 2+).
    pub struct Clipboard;
    impl Clipboard {
        pub fn new() -> Result<Self, &'static str> {
            Err("clipboard unavailable on web (internal kill-ring only)")
        }
        pub fn set_text(&mut self, _text: String) -> Result<(), &'static str> {
            Err("clipboard unavailable on web")
        }
        pub fn get_text(&mut self) -> Result<String, &'static str> {
            Err("clipboard unavailable on web")
        }
    }
}

/// Quiet period after the last edit before spell-check re-scans (debounce).
const SPELL_DEBOUNCE: Duration = Duration::from_millis(150);

/// Quiet period after the last edit before a quick note is auto-saved (debounce),
/// so a note is written calmly as you pause typing rather than on every keystroke.
const AUTOSAVE_DEBOUNCE: Duration = Duration::from_millis(400);

/// Quiet period after the last edit before the open DOCUMENT is autosaved (the
/// config-gated `autosave` engine, default ON): ~1s of idle writes the file
/// atomically, via the same single-`WaitUntil` pattern the other debounces use
/// (no hot loop). Blur / file switch / quit flush immediately instead.
const AUTOSAVE_IDLE: Duration = Duration::from_secs(1);

/// Quiet period after the last zoom step before the STICKY ZOOM is persisted to
/// config (debounce). Cmd-=/Cmd-- fire one step per press, so a write-per-step would
/// hammer the disk; instead `about_to_wait` writes the SETTLED zoom once you pause.
const ZOOM_PERSIST_DEBOUNCE: Duration = Duration::from_millis(500);

/// Quiet period after the last theme-picker PREVIEW step before the deferred FONT
/// reshape applies (debounce). Arrowing through the picker re-colors instantly
/// (`sync_theme_colors`, O(1) pipeline re-tints) but the font half of a switch is a
/// full-document reshape + a new-face atlas rasterization — the theme-burst profile
/// (`--bench-theme-burst`) measured it dominating every preview step. Deferring it
/// until the selection rests turns a 10-world arrow burst into 10 cheap recolors +
/// ONE reshape; a paused hover still shows the true face well inside a beat. Commit
/// (Enter), revert (Esc/C-g), and the headless capture all stay SYNCHRONOUS.
const THEME_FONT_DEBOUNCE: Duration = Duration::from_millis(150);

use glyphon::Cache;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Ime, Modifiers, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState};
// Exposes `KeyEvent::key_without_modifiers()` — the logical key BEFORE OS modifier
// composition. Used to undo macOS Option dead-key composition (Option-f -> 'ƒ') for
// Meta chords without breaking Option-accent text input. The trait lives on the
// DESKTOP backends (macOS / Windows / X11 / Wayland); the web backend has no such
// composition layer, so on wasm `key_without_modifiers` falls back to the plain
// logical key (see the cfg-split helper near the bottom of this file).
#[cfg(not(target_arch = "wasm32"))]
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::{CursorIcon, Window};

use crate::actions;
use crate::buffer::Buffer;
use crate::config::Config;
use crate::keymap::{Action, KeymapState};
use crate::render::{self, TextPipeline, ViewState};
use crate::search::Direction;

/// Max interval between clicks to count as a multi-click (double/triple).
const MULTICLICK_MS: u64 = 400;
/// The LIVE app's FIRST-RUN launch zoom factor (the user wanted text ~2 steps
/// smaller out of the box). This is only the default for a fresh install with no
/// remembered zoom: `App::new` overrides it with the config's persisted `zoom` when
/// present (sticky preferences), so the editor relaunches at whatever zoom you last
/// left it. `Cmd-0` (`ZoomReset`) still snaps to the natural 1.0 base. The headless
/// capture geometry (which builds its own pipeline at the `--zoom` default of 1.0)
/// stays fixed and all existing geometry/scroll tests are unchanged.
const INITIAL_ZOOM: f32 = 0.8;
/// Lines scrolled per mouse-wheel notch (LineDelta of 1.0).
const WHEEL_LINES_PER_NOTCH: f32 = 3.0;
/// Pixels of trackpad scroll that equal one line (PixelDelta accumulation).
const WHEEL_PIXELS_PER_LINE: f32 = 16.0;

/// What kind of unit the current drag is selecting by (set on press).
#[derive(Clone, Copy, PartialEq)]
enum DragGranularity {
    Char,
    Word,
    Line,
}

/// Which edit FLINCH the next `sync_view` fires on the visual caret: a typed char
/// squash-pops + back-kicks (PHASE 2), a delete squashes inward (PHASE 2), a
/// kill-line gulps (PHASE 2), Enter lands a caret-level touchdown squash (PHASE 3).
/// Armed from the matching [`actions::Effect`] (`TypeImpact` / `DeleteSquash` /
/// `Gulp` / `LineLand`).
#[derive(Clone, Copy)]
enum CaretImpact {
    Type,
    Delete,
    Gulp,
    Land,
}

struct Gpu {
    instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: TextPipeline,
    window: Arc<Window>,
}

/// GPU surface + frame loop (device/queue/surface, prepare/render).
mod gpu;
/// Buffer/config management: file open, notes, sticky prefs, rebind writes.
mod files;
/// View snapshot: build the `ViewState` and push it into the pipeline.
mod viewstate;
/// Input handling: search keys, mouse/drag, wheel/zoom, IME, HUD release.
mod input;
/// The apply bridge: resolve an `Action` + live-only side effects.
mod apply;
/// The single-instance DAEMON's App-side wiring (native only): react to a
/// posted `DaemonEvent`, finish a buffer for C-x #, tear down on quit.
mod daemon;
/// SESSION RESTORE's App-side wiring (native only): capture + apply the
/// persisted open-file set / active buffer / cursor+scroll / window frame.
mod session;

pub struct App {
    file: Option<PathBuf>,
    buffer: Buffer,
    keymap: KeymapState,
    mods: Modifiers,
    /// WHICH-KEY: when a multi-key PREFIX (`C-x`) is pending its second key, the
    /// instant it was pressed — the pause-timer anchor. `Some` ONLY while a prefix
    /// awaits resolution; cleared the instant the chord completes or aborts. Drives the
    /// single `WaitUntil` deadline in `about_to_wait` (armed only while pending → idle
    /// stays 0% CPU, DESIGN §6). See `crate::whichkey`.
    prefix_pending_at: Option<Instant>,
    /// WHICH-KEY: whether the continuation panel is currently SUMMONED (the pause
    /// elapsed with the prefix still pending). Once true the pause timer stops arming
    /// (no ongoing tick); reset to false — and the panel put down — the instant the
    /// prefix resolves/aborts.
    whichkey_shown: bool,
    scroll_lines: usize,
    gpu: Option<Gpu>,
    /// WASM-only handoff slot for the ASYNC GPU init. The browser main thread can't
    /// block, so `Gpu::new` runs on a `spawn_local` future that parks its result
    /// here; `window_event` moves it into `gpu` on the first frame. `Rc<RefCell>`
    /// because the future and the App share it on the (single) wasm main thread.
    #[cfg(target_arch = "wasm32")]
    gpu_pending: std::rc::Rc<std::cell::RefCell<Option<Gpu>>>,
    /// Timestamp of the previous animated frame, for real-time spring dt. `None`
    /// while idle; set on the first animating redraw and cleared once settled.
    last_frame: Option<Instant>,
    /// DEBUG panel: ring of the last 120 drawn frames' costs (ms) — `last()` is
    /// the previous completed frame (the one-frame-lag line 1 value), `worst()`
    /// the rolling max that survives stillness. Fed ONLY while the panel is on
    /// (the pane-off editor does zero timing work); the settle-stamp frame's own
    /// cost is measured and DISCARDED (panel bookkeeping, not user workload).
    frame_costs: crate::debug::CostRing,
    /// DEBUG panel key→px: `Instant` stamped when an input (key press / mouse
    /// press / scroll) reached `window_event` while the panel is on. Only the
    /// FIRST un-rendered input per frame stamps (`get_or_insert`), so under
    /// coalescing the measured latency is the worst case; taken (→
    /// `last_latency_ms`) when the frame it caused actually PRESENTS. `None`
    /// while the panel is off or no input is awaiting pixels.
    input_stamp: Option<Instant>,
    /// DEBUG panel key→px: the most recent measured input→present-return latency
    /// (ms), shown until the next input-driven frame updates it. `None` before
    /// any input (the fixed placeholder).
    last_latency_ms: Option<f32>,
    /// DEBUG panel: monotonic count of frames drawn since launch. Incremented
    /// unconditionally (a single add — counting even while the panel is off means
    /// toggling debug on mid-hot-loop shows a climbing count immediately). Its
    /// whole diagnostic value is being FROZEN while idle: a climb without input
    /// is a hot-loop bug made visible.
    redraw_count: u64,
    /// DEBUG panel stillness: the settle-stamp state machine (`debug::DebugStill`)
    /// — Active while frames happen anyway, StampQueued for the ONE extra redraw
    /// that draws the `still ·` readout after the app settles, Still while fully
    /// quiet (no frames scheduled, 0% CPU). Pure transitions live in `debug.rs`.
    debug_still: crate::debug::DebugStill,
    /// POINTER AUTO-HIDE state ("games do this" / the macOS-native
    /// `NSCursor.setHiddenUntilMouseMoves` convention): `Visible` (resting),
    /// or `Hidden` (the OS pointer is currently hidden, having been hidden by
    /// a keystroke). Pure transitions live in `pointer_hide.rs`; this field
    /// is the live App's only copy of "where are we". A real keystroke hides
    /// it immediately (see the `KeyboardInput` arm below); any mouse motion,
    /// or the window losing focus, un-hides it instantly. LIVE-ONLY: the
    /// headless capture never touches this (no window, no OS pointer to
    /// hide).
    pointer_hide: crate::pointer_hide::PointerHide,
    /// The logical key currently HOLDING the stats HUD open (`Action::ShowStatsHud`
    /// pressed), or `None` when released. The press records it; the matching key
    /// RELEASE clears the HUD (`hud::set_held(false)`), as does releasing a summoning
    /// modifier (`hud_mods`). So the HUD is a true HOLD — summoned while down, dismissed
    /// the instant the chord lifts. See `on_key_release` / `hud_release_on_mods`.
    hud_key: Option<Key>,
    /// The MODIFIER state held when the stats HUD was summoned, so dropping ANY of those
    /// modifiers also dismisses it. macOS does NOT deliver a key-UP for a character key
    /// while Cmd is held (and the user often lifts Cmd first), so the key-release path
    /// alone leaves the HUD stuck-on; watching `ModifiersChanged` for a released
    /// summoning modifier closes that gap. See `hud_release_on_mods`.
    hud_mods: ModifiersState,
    /// Current zoom factor. Single source of truth for the LIVE app; pushed into the
    /// pipeline via the view snapshot. Launches at [`INITIAL_ZOOM`] (the natural 1.0
    /// base) so text starts at a calm default; the headless capture is unaffected (it
    /// builds its own pipeline at the fixed `--zoom` default of 1.0).
    zoom: f32,
    /// The window's display DPI `scale_factor` (1.0 on a 1:1 screen, 2.0 on a 2x
    /// Retina panel). The window width and the cursor position arrive in PHYSICAL
    /// pixels, but the glyph metrics are tuned for a 1:1 canvas, so this factor is
    /// folded into them (pipeline `set_dpi` + the local scroll/page math below) to
    /// keep the live page proportioned like the capture. Updated on creation and on
    /// `ScaleFactorChanged` (a monitor move). The headless capture never sets it.
    dpi: f32,
    /// Last known cursor position in PHYSICAL pixels (for wheel-zoom anchoring
    /// and hit-testing on press). Updated on every CursorMoved.
    cursor_px: (f32, f32),
    /// True while the primary mouse button is held (a drag is in progress).
    dragging: bool,
    /// True while a DIRECT page-width resize drag is in progress (a press that landed
    /// on a page-column edge; the pointer's distance from center drives the measure
    /// LIVE, and the release commits + persists it). Mutually exclusive with a text
    /// selection `dragging` — a press near a boundary starts this instead.
    page_resizing: bool,
    /// The CACHED last icon actually handed to `Window::set_cursor` — the invariant
    /// `cursor_shape::cursor_icon_change` leans on (this always equals the OS's real
    /// last-set icon), so the context-aware cursor (`sync_cursor_icon`) only ever
    /// calls `set_cursor` on an actual change, never every move. See `cursor_shape.rs`.
    cursor_icon: CursorIcon,
    /// Selection granularity of the active drag (char/word/line).
    drag_granularity: DragGranularity,
    /// For double/triple-click detection: time + position of the last press and
    /// the running click count.
    last_click_time: Option<Instant>,
    last_click_px: (f32, f32),
    click_count: u32,
    /// Accumulated trackpad pixel scroll not yet converted to a whole line.
    scroll_px_accum: f32,
    /// True while the active selection was begun with Shift (TRANSIENT: a later
    /// unshifted motion collapses it). C-Space marks set this false (sticky).
    shift_selecting: bool,
    /// The in-progress IME composition (romaji->kana->kanji) string. Empty when
    /// not composing. Shown as an underlined overlay at the caret WITHOUT being
    /// inserted into the ropey buffer; a Commit finalizes it into the buffer.
    preedit: String,
    /// True between Ime::Enabled and Ime::Disabled (the IME is active). Used to
    /// know composition is possible; the actual suppression of raw key insertion
    /// keys off a non-empty `preedit`.
    ime_enabled: bool,
    /// Active incremental search, modeled like the IME `preedit`: a transient
    /// surface owned by `App` (NOT in the keymap, NOT in the rope). `None` =
    /// editing normally; `Some` = isearch active, and every key is routed to
    /// `handle_search_key` instead of the keymap.
    search: Option<crate::search::SearchState>,
    /// The spell-check engine (bundled en_US Hunspell), loaded ONCE at startup.
    /// `None` if the dictionary failed to parse (reported to stderr); spell-check
    /// then no-ops rather than crashing the editor.
    spell: Option<crate::spell::SpellChecker>,
    /// Cached misspelled spans plus the buffer EDIT VERSION they were computed for.
    /// A whole-buffer re-check only runs when the version actually changed (cursor
    /// moves / scroll reuse the cached spans); comparing a `u64` version avoids
    /// cloning + comparing the whole rope string on every keystroke (the old hot
    /// path did `spell_cache.0 != buffer.text()`).
    spell_cache: Vec<crate::spell::Misspelling>,
    /// Buffer version the `spell_cache` reflects. `None` until the first check, so
    /// the first edit always schedules one.
    spell_checked_version: Option<u64>,
    /// When the buffer text last changed; spell-check is recomputed only after a
    /// ~150ms quiet period (debounce) so squiggles don't flicker mid-word.
    spell_dirty_at: Option<Instant>,
    /// Buffer version at the previous `sync_view`. A change since then means the
    /// cursor moved BECAUSE of an edit (typing/delete/paste/newline), so the
    /// caret slides as a plain block; an unchanged version means navigation.
    caret_synced_version: u64,
    /// CACHED document text for [`Self::sync_view`], keyed by the buffer VERSION at
    /// the clone. A pure cursor move / scroll / selection change does NOT bump the
    /// version, yet `sync_view` runs every one of them and would re-materialise the
    /// whole rope into a `String` each time; this reuses the last clone (a cheap
    /// memcpy) whenever the version is unchanged, walking the rope only after an
    /// actual edit. Same bytes either way, so the pushed `ViewState.text` is identical.
    sync_text_cache: Option<(u64, String)>,
    /// Set by `apply` for the ONE next `sync_view` when an edit should still
    /// streak its caret glide (delete-word-backward), so the removed span and the
    /// caret motion read as a single concurrent move instead of "text vanishes,
    /// then a bare block slides". Consumed (and reset) by the next `sync_view`.
    /// Defaults false: a normal edit (typing/Backspace/paste) keeps the plain
    /// no-underline slide.
    caret_edit_streaks: bool,
    /// Set from `winit`'s `KeyEvent.repeat` for the ONE next `sync_view`: true when
    /// the keypress that triggered this sync is an OS AUTO-REPEAT (a HELD arrow /
    /// motion key) rather than a discrete tap. Held navigation builds a continuous
    /// lagging caret trail; a lone tap stays gap-suppressed. Consumed (and reset)
    /// by the next `sync_view`, so non-keyboard syncs (IME, wheel) read `false`.
    caret_held: bool,
    /// Edit FLINCH requested by `apply` for the ONE next `sync_view`: a SUCCESSFUL
    /// typed char ([`CaretImpact::Type`]) squash-pops + back-kicks, a delete
    /// ([`CaretImpact::Delete`]) squashes the caret inward, a kill-line
    /// ([`CaretImpact::Gulp`]) pulses a bigger gulp, Enter ([`CaretImpact::Land`])
    /// lands a caret-level touchdown squash (PHASE 3). Consumed by the next
    /// `sync_view` AFTER it sets the spring target, so the flinch rides on top and
    /// the spring self-settles it back to rest. Fires in EVERY caret look (all
    /// juice on the caret). `None` = no edit flinch this sync.
    caret_impact: Option<CaretImpact>,
    /// BLOCKED-ACTION RECOIL bump requested by `apply` for the ONE next `sync_view`:
    /// a motion into a wall / a page that can't page / an exhausted undo-redo / a
    /// delete with nothing to remove bumps the VISUAL caret away from the wall. Fires
    /// in EVERY caret look (Block/Morph/I-beam), and is mutually exclusive with the
    /// edit flinch (a blocked edit recoils away from the wall; a successful edit
    /// flinches). Consumed by the next `sync_view`. `None` = no recoil.
    caret_recoil: Option<crate::caret::RecoilDir>,
    /// OS clipboard bridge. None when arboard cannot init (headless / no
    /// display / no Wayland seat); editor then runs on the internal kill-ring
    /// only, exactly like `spell` degrades to None.
    clipboard: Option<Clipboard>,
    /// The exact text WE last wrote to (sync_kill_to_clipboard) or read from
    /// (refresh_kill_from_clipboard) the OS clipboard. Used to (a) skip
    /// redundant mirror writes and (b) detect an external copy on yank without
    /// mistaking our own write for an external change. None until first sync.
    clipboard_last_written: Option<String>,
    /// The ACTIVE project root. Exactly one at a time; it scopes the go-to file
    /// index so typing ".env" finds THIS repo's env.
    root: PathBuf,
    /// Resolved active project (name / branch / dirty) for the quiet status
    /// strip. Recomputed whenever the root changes.
    project: crate::project::Project,
    /// The active root's file index (corpus the go-to overlay matches against).
    /// Rebuilt on a root switch.
    file_index: Vec<String>,
    /// Optional workspace parent whose children are the switch-project
    /// candidates (stored for the next phase).
    workspace: Option<PathBuf>,
    /// MRU stack of opened ROOT-RELATIVE paths (most-recent last), feeding the
    /// go-to ranker's "recently opened" tier.
    opened: Vec<String>,
    /// The PREVIOUSLY-opened absolute file path, for the C-x b last-buffer toggle
    /// (a tiny 2-deep history: the current `file` + this one). `None` until a
    /// second file has been opened. Toggling swaps `file` <-> `prev_file`.
    prev_file: Option<PathBuf>,
    /// The SUMMONED navigation overlay (go-to / switch-project). `None` when not
    /// showing. Lives here AND is threaded through `apply_core` so `--keys` can
    /// drive it identically.
    overlay: Option<crate::overlay::OverlayState>,
    /// The NOTES ROOT: the home project where C-x n captures quick scrap notes
    /// (default `~/notes`, overridable with `--notes-root`). C-x n jumps here and
    /// opens a fresh note; C-x m moves the current note into a folder under it.
    notes_root: PathBuf,
    /// When the active NOTE last changed and an auto-save is pending; the debounced
    /// write fires after `AUTOSAVE_DEBOUNCE` of quiet in `about_to_wait` (live
    /// only — headless never schedules this). `None` = nothing pending.
    autosave_dirty_at: Option<Instant>,
    /// Buffer version the note was last auto-saved at, so an unchanged buffer is
    /// not re-written. `None` until the first save.
    autosave_saved_version: Option<u64>,
    /// When the open DOCUMENT last changed and an idle AUTOSAVE is pending; the
    /// debounced flush fires after [`AUTOSAVE_IDLE`] of quiet in `about_to_wait`
    /// (live only — armed exclusively in `sync_view` behind the gpu-present gate,
    /// so the headless capture can never schedule it). `None` = nothing pending.
    doc_autosave_at: Option<Instant>,
    /// Buffer version of the open document whose content is known to be ON DISK
    /// (from load, a manual save, or an autosave), so an unchanged buffer is
    /// never re-written. `None` until known.
    doc_saved_version: Option<u64>,
    /// Our last-known on-disk MODIFIED time of the open file (stamped on load and
    /// after each of our own writes) — the CLOBBER GUARD's baseline: an autosave
    /// first re-stats the file, and a mismatch means someone else wrote it, so
    /// the write is HELD with a calm notice instead of overwriting external
    /// edits. Wasm-safe (`crate::clock::SystemTime`, never std).
    disk_mtime: Option<crate::clock::SystemTime>,
    /// Buffer version of the no-path SCRATCH buffer last stashed to
    /// [`crate::fs::scratch_stash_path`], so an unchanged scratch isn't re-written.
    scratch_saved_version: Option<u64>,
    /// Last-known on-disk mtime of the scratch stash (two-instance clobber
    /// safety, mirroring `disk_mtime`).
    scratch_mtime: Option<crate::clock::SystemTime>,
    /// When the AUTOSAVE ENGINE last wrote successfully THIS session (the doc
    /// autosave OR the scratch stash) — stamped ONLY inside `autosave_doc_now` /
    /// `stash_scratch_now`'s `Ok` arms (i.e. exclusively through
    /// `App::autosave_flush`'s one door, past its clobber-guard check), so the
    /// debug panel's `autosave saved · Ns ago` line can never claim a write the
    /// engine didn't just make. `None` before the first successful write. Feeds
    /// `crate::debug::autosave_state` at redraw time (gated on `debug_on()`, like
    /// every other clock read the panel takes) — never read otherwise.
    autosave_last_ok: Option<Instant>,
    /// A transient CALM NOTICE for the bottom of the canvas (today: the autosave
    /// clobber guard's "changed on disk outside awl — autosave held"). `None`
    /// draws nothing. LIVE-ONLY by construction — autosave can never fire
    /// headlessly — so it has no sidecar field and a default capture is
    /// byte-identical (the empty notice parks off-screen).
    notice: Option<String>,
    /// HISTORY TIMELINE live preview cache: the `(id, content)` of the version the
    /// open History overlay's highlighted row resolves to, loaded once per id (via
    /// [`crate::history::load`]) so an arrow/hover/wheel burst over the rows never
    /// re-reads the store per sync. The preview itself is DERIVED at
    /// ViewState-build time (`sync_view` overrides the pushed text) — the Buffer,
    /// its version, and its undo history are NEVER touched. Dropped the moment the
    /// overlay closes (Esc = back to now exactly). `None` = no preview.
    history_preview: Option<(String, String)>,
    /// The document scroll (visual rows) captured when the History timeline
    /// OPENED, restored on a close-without-restore — a shorter previewed version
    /// can destructively clamp `scroll_lines` against ITS max-scroll, and "Esc =
    /// back to now exactly" includes the viewport. Taken (not restored) on a real
    /// Enter-restore. `None` while the timeline isn't open.
    history_scroll_before: Option<usize>,
    /// When the zoom last changed and a STICKY-ZOOM write is pending; the debounced
    /// write fires after `ZOOM_PERSIST_DEBOUNCE` of quiet in `about_to_wait`, so a
    /// rapid Cmd-=/Cmd-- run persists the SETTLED value once instead of per step.
    /// `None` = nothing pending (live only — headless never schedules this).
    zoom_persist_at: Option<Instant>,
    /// When the theme-picker live PREVIEW last landed on a world whose display face
    /// differs from the shaped one, and the deferred FONT reshape is pending; the
    /// debounced `sync_theme_font` fires after `THEME_FONT_DEBOUNCE` of quiet in
    /// `about_to_wait` (each further arrow re-stamps it, sliding the deadline).
    /// Cleared by the settle itself and by every SYNCHRONOUS retint (commit /
    /// revert — `retint_theme_now`), so Esc can never leave a stray reshape to land
    /// after the picker closed. `None` = nothing pending (live only — the headless
    /// replay applies theme fonts synchronously through the pure core + a fresh
    /// pipeline, so captures are untouched).
    theme_font_at: Option<Instant>,
    /// The loaded persistent config (keybinding overrides + folder defaults + the
    /// Settings-open path). Re-loaded when the config file is SAVED in the editor,
    /// which live-reapplies the keymap + folders.
    config: Config,
    /// The RAW `--notes-root` flag (None = unset), remembered so a live config reload
    /// re-folds precedence (flag > config > default) without the flag ever losing.
    cli_notes_root: Option<PathBuf>,
    /// The RAW `--workspace` flag (None = unset), remembered for the same reason.
    cli_workspace: Option<PathBuf>,
    /// MULTI-BUFFER REGISTRY: every OTHER currently-open buffer (backgrounded —
    /// not the active `self.buffer`), keyed by stable identity. Opening a path
    /// already resident here SWITCHES to its live buffer (unsaved edits, cursor,
    /// scroll, undo, spell state all survive) instead of re-reading disk — the
    /// v1 multi-buffer win. See `files::BufferExtra` + `files::park_active_buffer`.
    buffer_registry: crate::buffers::BufferRegistry<files::BufferExtra>,
    /// SESSION RESTORE (native only): the window FRAME a previous session left
    /// (already clamped to whatever screens were connected at THAT save time —
    /// re-clamped again in `resumed()` against the CURRENT screens), applied
    /// once when the window is first created. `None` when there was nothing to
    /// restore (no session file, the kill-switch is off, or this platform never
    /// captures one) — `resumed()` then falls back to the fixed 1200x800
    /// default, unchanged from before this round.
    #[cfg(not(target_arch = "wasm32"))]
    restored_window: Option<crate::session::WindowFrame>,
    /// SINGLE-INSTANCE DAEMON (native only): the socket special file's path, so
    /// `daemon::daemon_shutdown` can unlink it on a clean quit — `None` when this
    /// launch never became the instance (a socket error degraded to a normal,
    /// non-singleton launch; see `crate::app::run`).
    #[cfg(not(target_arch = "wasm32"))]
    daemon_socket_path: Option<PathBuf>,
    /// SINGLE-INSTANCE DAEMON (native only): every daemon `--wait` client's still-
    /// open connection, keyed by the [`crate::buffers::BufferKey`] of the buffer it
    /// is waiting on. `Action::FinishBuffer` (C-x #) notifies + drains the entry for
    /// the buffer being finished; `daemon::daemon_shutdown` drains everything on
    /// quit (a dropped `Waiter` closes its socket, which the client treats as done
    /// too — see `crate::daemon`'s module doc).
    #[cfg(not(target_arch = "wasm32"))]
    wait_conns: std::collections::HashMap<crate::buffers::BufferKey, Vec<crate::daemon::Waiter>>,
}

impl App {
    fn new(
        file: Option<PathBuf>,
        root: PathBuf,
        cli_workspace: Option<PathBuf>,
        cli_notes_root: Option<PathBuf>,
        config: Config,
    ) -> Self {
        // SESSION RESTORE (native only) reads this BEFORE `file` moves into the
        // struct literal below — see `Self::apply_session_restore`'s doc for why
        // a launch WITH a file argument still restores the rest of the session
        // (just never lets it override the active buffer).
        #[cfg(not(target_arch = "wasm32"))]
        let file_arg_given = file.is_some();
        // SCRATCH RESTORE: a no-argument launch resumes the persistent scratch
        // buffer from its stash (written by the autosave engine on idle/blur/
        // quit). Path stays None — still a true scratch, still markdown-first.
        // ONLY the live App restores; the headless `load_buffer` never reads the
        // stash, so a default no-file capture stays byte-identical.
        let stash = crate::fs::scratch_stash_path();
        let buffer = match &file {
            Some(p) => Buffer::from_file(p),
            None => match crate::fs::active().read_to_string(&stash) {
                Ok(s) if !s.is_empty() => Buffer::from_str(&s),
                _ => Buffer::scratch(),
            },
        };
        let initial_version = buffer.version();
        // CLOBBER-GUARD baselines: the launch file's current on-disk mtime (its
        // content just became the buffer), and — for a no-file launch — the
        // stash's mtime (present even when the stash was empty, so the first
        // stash write isn't mistaken for an external edit).
        let disk_mtime = file.as_deref().and_then(Self::disk_mtime_of);
        let scratch_mtime = if file.is_none() {
            Self::disk_mtime_of(&stash)
        } else {
            None
        };
        let project = crate::project::Project::resolve(&root);
        let file_index = crate::index::build_index(&root);
        // PRECEDENCE flag > config > default. Fold the config folder values in BEHIND
        // the raw CLI flags (the flag wins via `.or`), then the existing resolvers add
        // the built-in default (`~/notes`; the root's PARENT for the workspace), so
        // C-x n / C-x p work out of the box with the configured folders, no flags.
        let notes_root =
            crate::resolve_notes_root(&cli_notes_root.clone().or_else(|| config.notes_root.clone()));
        let workspace_opt = cli_workspace.clone().or_else(|| config.workspace.clone());
        let workspace = Some(crate::resolve_workspace(&workspace_opt, &root));
        // Build the keymap with the config `[keys]` rebinds applied over the defaults.
        let keymap = KeymapState::with_overrides(&config.keys);
        // STICKY ZOOM: relaunch at the remembered zoom, else the first-run default
        // (`INITIAL_ZOOM`). Clamped to the valid range so a hand-edited extreme can't
        // wedge the view. (Theme / page / caret are process-globals already restored
        // in `main` before `App::new`; zoom is per-instance so it lands here.)
        let zoom = render::clamp_zoom(config.zoom.unwrap_or(INITIAL_ZOOM));
        let mut app = Self {
            file,
            buffer,
            keymap,
            mods: Modifiers::default(),
            prefix_pending_at: None,
            whichkey_shown: false,
            scroll_lines: 0,
            gpu: None,
            #[cfg(target_arch = "wasm32")]
            gpu_pending: std::rc::Rc::new(std::cell::RefCell::new(None)),
            last_frame: None,
            frame_costs: crate::debug::CostRing::default(),
            input_stamp: None,
            last_latency_ms: None,
            redraw_count: 0,
            debug_still: crate::debug::DebugStill::Active,
            pointer_hide: crate::pointer_hide::PointerHide::Visible,
            hud_key: None,
            hud_mods: ModifiersState::empty(),
            zoom,
            dpi: 1.0,
            cursor_px: (0.0, 0.0),
            dragging: false,
            page_resizing: false,
            cursor_icon: CursorIcon::Default,
            drag_granularity: DragGranularity::Char,
            last_click_time: None,
            last_click_px: (0.0, 0.0),
            click_count: 0,
            scroll_px_accum: 0.0,
            shift_selecting: false,
            preedit: String::new(),
            ime_enabled: false,
            search: None,
            spell: match crate::spell::SpellChecker::new(crate::spell::active_variant()) {
                Ok(sc) => Some(sc),
                Err(e) => {
                    eprintln!("spell-check disabled: {e}");
                    None
                }
            },
            spell_cache: Vec::new(),
            spell_checked_version: None,
            spell_dirty_at: None,
            caret_synced_version: initial_version,
            sync_text_cache: None,
            caret_edit_streaks: false,
            caret_held: false,
            caret_impact: None,
            caret_recoil: None,
            clipboard: match Clipboard::new() {
                Ok(c) => Some(c),
                Err(e) => {
                    eprintln!("system clipboard disabled: {e}");
                    None
                }
            },
            clipboard_last_written: None,
            root,
            project,
            file_index,
            workspace,
            opened: Vec::new(),
            prev_file: None,
            overlay: None,
            notes_root,
            autosave_dirty_at: None,
            autosave_saved_version: None,
            doc_autosave_at: None,
            // The just-loaded buffer IS the on-disk content (and a just-restored
            // scratch IS the stash), so neither starts "unsaved".
            doc_saved_version: Some(initial_version),
            disk_mtime,
            scratch_saved_version: Some(initial_version),
            scratch_mtime,
            autosave_last_ok: None,
            notice: None,
            history_preview: None,
            history_scroll_before: None,
            zoom_persist_at: None,
            theme_font_at: None,
            config,
            cli_notes_root,
            cli_workspace,
            buffer_registry: crate::buffers::BufferRegistry::default(),
            #[cfg(not(target_arch = "wasm32"))]
            restored_window: None,
            #[cfg(not(target_arch = "wasm32"))]
            daemon_socket_path: None,
            #[cfg(not(target_arch = "wasm32"))]
            wait_conns: std::collections::HashMap::new(),
        };
        // i18n WRITE-BACK-ONCE (see `files::write_back_lang_tag_once`'s doc):
        // covers the `awl somefile.md` LAUNCH-ARGUMENT open, mirroring the
        // C-x f / C-x b / goto path's own call in `App::load_path` — a real
        // FILE only (never the no-argument scratch/stash-restore buffer,
        // which isn't "opening a document").
        if app.file.is_some() {
            app.write_back_lang_tag_once();
        }
        // SESSION RESTORE (native only, kill-switch gated): the OTHER open
        // files (parked into the buffer registry) and, on a bare launch, the
        // ACTIVE file + its cursor/scroll. Composes with — never replaces —
        // whatever the scratch-stash restore above already picked.
        #[cfg(not(target_arch = "wasm32"))]
        app.apply_session_restore(file_arg_given);
        app
    }
}

/// TEST HERMETICITY: the ONE door every test that needs a real `App` should
/// build it through, instead of calling `App::new` directly. `App::new` reads
/// two pieces of ambient state a plain test never intends to touch:
///
///  - **Session restore** (`apply_session_restore`, native-only): unless the
///    passed `Config` disables it, this reads `~/.local/share/awl/session.toml`
///    (or wherever `$XDG_DATA_HOME` points) through the REAL `FileSystem`
///    backend and PARKS every surviving buffer it names into the registry —
///    regardless of whether `file` is `Some` or `None`. On the developer's own
///    machine this is his ACTUAL live session: whatever files happen to be
///    open in a real `awl` right now leak into the test's `buffer_registry`,
///    and `open_buffer_count()`/similar assertions silently start tracking his
///    editing session instead of the test's fixture (`d93109e` fixed one
///    instance of exactly this leak — this closes the door everywhere else it
///    was still open).
///  - **Scratch stash**: a `file: None` launch reads the scratch buffer's
///    stash (`~/.local/share/awl/scratch.md`) through the SAME real backend —
///    UNCONDITIONALLY; unlike session restore there is no config gate for it
///    at all. A test with no fake FS installed gets the developer's real
///    scratch content loaded as the initial buffer.
///
/// This constructor closes both doors by (a) forcing `session_restore:
/// Some(false)` into the passed `Config` and (b) installing a throwaway,
/// empty `InMemoryFs` for the SCOPE of construction only (via `fs::with_fs`,
/// which restores whatever backend was active before, on return) — so both
/// reads land on a fake with nothing in it, and any directory scan the
/// constructor does along the way (`crate::index::build_index`,
/// `crate::project::Project::resolve`'s `.git` probe) also finds nothing
/// rather than walking a real directory.
///
/// **Explicitly NOT closed, and why that's fine:**
///  - The **daemon socket**: `App::new` itself never binds one — only
///    `crate::app::run` does (see `crate::daemon`'s module doc) — there is
///    nothing here to guard.
///  - The **config file path**: `Config` is a plain value passed in by the
///    caller; `App::new` never re-reads `config.toml` off disk itself (only
///    `main::run`'s startup path does that, before `App::new` is ever built).
///  - **Sticky prefs** (theme / caret mode / the zoom PERSISTED default /
///    etc.): these are process-globals restored by `main::run` before
///    `App::new` runs; `App::new` only reads the *passed* `Config`'s `zoom`
///    field for the per-instance zoom, never re-derives a sticky preference
///    from the environment on its own.
///
/// **When NOT to use this:** a test that genuinely needs the App to see REAL
/// file content (verifying an actual save landed on disk, or that a second
/// real file's bytes reach the view after `load_path`) cannot use this — the
/// injected `InMemoryFs` would make `Buffer::from_file` find nothing. Call
/// `App::new` directly instead, but still merge `Config{session_restore:
/// Some(false), ..}` into the passed config yourself, and hold
/// `fs::TEST_LOCK` for the test's life — see
/// `open_serves_the_new_files_text_despite_equal_buffer_versions` below, or
/// `app/daemon.rs`'s `finish_buffer_saves_notifies_the_waiter_and_switches_
/// to_the_previous_buffer`, for the pattern. `real_fs_app_new_calls_are_
/// all_accounted_for` (in this file's test module) is the structural guard
/// making sure a raw `App::new` call never gets added silently without one
/// of these two treatments.
#[cfg(test)]
impl App {
    pub(crate) fn new_hermetic(file: Option<PathBuf>, root: PathBuf, config: Config) -> Self {
        let config = Config { session_restore: Some(false), ..config };
        let fake: Arc<dyn crate::fs::FileSystem> = Arc::new(crate::fs::InMemoryFs::new());
        crate::fs::with_fs(fake, || Self::new(file, root, None, None, config))
    }
}

impl App {
    /// Shared post-GPU-init: fold the monitor's DPI scale into the metrics BEFORE
    /// the first sync (so the opening frame is proportioned like the capture on a
    /// HiDPI screen), push the initial view, and request the opening frame. Called
    /// inline after the NATIVE blocking init, and from `window_event` once the WASM
    /// async init deposits its GPU.
    fn on_gpu_ready(&mut self) {
        let sf = self.gpu.as_ref().unwrap().window.scale_factor() as f32;
        self.dpi = sf;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.pipeline.set_dpi(sf);
        }
        // WASM: the surface was configured inside the async `Gpu::new` against the
        // canvas's size AT CREATION — which is 1x1 before the browser lays the page
        // out, and the `Resized` events carrying the real canvas size fired WHILE the
        // GPU future was still pending (so they were dropped by the `gpu.is_none()`
        // guard). winit still tracked the latest observed size, so re-read it now and
        // resize the surface to the true canvas size, else the first frame draws into
        // a 1x1 surface (a blank page). Native's size is already correct here, so the
        // fix is web-only and leaves the native path untouched.
        #[cfg(target_arch = "wasm32")]
        {
            let size = self.gpu.as_ref().unwrap().window.inner_size();
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.resize(size.width.max(1), size.height.max(1));
            }
        }
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// DEBUG key→px: stamp the receipt of an input that will request a redraw —
    /// key presses AND mouse press/scroll, which share the `request_redraw` path.
    /// `get_or_insert` keeps only the FIRST un-rendered input per frame, so under
    /// coalescing the measured latency is the worst case. Gated on the panel
    /// being on (zero clock reads otherwise); the span closes at present-return
    /// in `RedrawRequested`. Wasm-safe (`crate::clock::Instant`).
    fn stamp_input(&mut self) {
        if crate::debug::debug_on() {
            self.input_stamp.get_or_insert(Instant::now());
        }
    }
}

/// The winit USER EVENT type this app's event loop carries: the single-instance
/// daemon's posted events on native, an uninhabited no-op on wasm (the browser
/// has no process/socket concept — `crate::daemon` compiles out there entirely).
#[cfg(not(target_arch = "wasm32"))]
type AwlEvent = crate::daemon::DaemonEvent;
#[cfg(target_arch = "wasm32")]
type AwlEvent = ();

impl ApplicationHandler<AwlEvent> for App {
    /// A daemon event (native only) posted by the accept-loop thread via
    /// `EventLoopProxy::send_event` — always runs on this, the normal winit
    /// thread. A no-op on wasm (there is no `AwlEvent` variant to construct).
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: AwlEvent) {
        #[cfg(not(target_arch = "wasm32"))]
        self.handle_daemon_event(_event);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        let title = match &self.file {
            Some(p) => format!("awl - {}", p.display()),
            None => "awl - *scratch*".to_string(),
        };
        // MINIMUM window size, tied to the font metrics so the window can never be
        // dragged below roughly ONE readable line. Width = ~30 columns at the default
        // advance plus the side insets; height = a handful of lines plus the top inset.
        // Below this the responsive page column would have nothing left to show, so we
        // stop the drag here (LOGICAL px, so it scales with the monitor's DPI).
        const MIN_COLS: f32 = 30.0;
        const MIN_LINES: f32 = 8.0;
        let min_w = MIN_COLS * render::CHAR_WIDTH + 2.0 * render::TEXT_LEFT;
        let min_h = MIN_LINES * render::LINE_HEIGHT + 2.0 * render::TEXT_TOP;
        // NATIVE ONLY: pin a fixed opening size (1200x800 logical px — also the
        // capture harness's own default canvas, so `--screenshot` stays
        // byte-identical). On the WEB this must NOT be set: winit's web backend
        // maps `with_inner_size` straight onto an INLINE `style.width`/
        // `style.height` on the `<canvas>` (`web_sys::set_canvas_size`), which
        // permanently pins the element at that pixel size and overrides
        // index.html's responsive `width:100vw;height:100vh` CSS outright — a
        // viewport under 1200x800 then clips unreachably (`body{overflow:
        // hidden}`). Leaving `inner_size` unset means winit only ever writes
        // `min-width`/`min-height` (a floor, not a pin) and the canvas keeps its
        // CSS-driven size; winit's web backend installs a `ResizeObserver` on the
        // canvas unconditionally (`window_target.rs`'s `on_resize_scale`, wired
        // regardless of whether `inner_size` was ever set) that fires
        // `WindowEvent::Resized` on every CSS/viewport size change — the SAME
        // generic `Resized` arm below already re-syncs the layout on that event,
        // so no bespoke web resize plumbing (no `ResizeObserver` of our own) is
        // needed; winit already tracks the browser viewport for us.
        // SESSION RESTORE (native only): a previous session's window FRAME wins
        // over the fixed default, RE-CLAMPED here against the CURRENTLY connected
        // screens (`Self::apply_session_restore` already loaded + stashed it in
        // `self.restored_window`, but screens can change between quit and this
        // very relaunch — a disconnected external monitor must never strand the
        // window off every visible display). `None` (no session, kill-switch
        // off, or first-ever launch) falls back to the pre-existing fixed
        // 1200x800 default, so a plain `--screenshot` and a fresh install are
        // both unaffected.
        #[cfg(not(target_arch = "wasm32"))]
        let attrs = {
            let attrs = Window::default_attributes()
                .with_min_inner_size(LogicalSize::new(min_w, min_h))
                .with_title(title);
            match self.restored_window {
                Some(frame) => {
                    let screens: Vec<crate::session::ScreenRect> = event_loop
                        .available_monitors()
                        .map(|m| {
                            let pos = m.position();
                            let size = m.size();
                            crate::session::ScreenRect {
                                x: pos.x,
                                y: pos.y,
                                width: size.width,
                                height: size.height,
                            }
                        })
                        .collect();
                    let clamped = crate::session::clamp_frame_to_screens(frame, &screens);
                    attrs
                        .with_inner_size(winit::dpi::PhysicalSize::new(
                            clamped.width,
                            clamped.height,
                        ))
                        .with_position(winit::dpi::PhysicalPosition::new(clamped.x, clamped.y))
                }
                None => attrs.with_inner_size(LogicalSize::new(1200.0, 800.0)),
            }
        };
        #[cfg(target_arch = "wasm32")]
        let attrs = Window::default_attributes()
            .with_min_inner_size(LogicalSize::new(min_w, min_h))
            .with_title(title);
        // On the WEB, render INTO the page's <canvas id="awl-canvas"> (placed by
        // index.html) instead of letting winit mint a detached, un-appended canvas.
        #[cfg(target_arch = "wasm32")]
        let attrs = {
            use wasm_bindgen::JsCast;
            use winit::platform::web::WindowAttributesExtWebSys;
            let canvas = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id("awl-canvas"))
                .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok());
            attrs.with_canvas(canvas)
        };
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        // Ask the platform to deliver IME events so CJK (Japanese) composition
        // works: without this, WindowEvent::Ime is never sent and the user can
        // only type raw ASCII. Safe to call unconditionally; platforms without an
        // IME simply never emit the events.
        window.set_ime_allowed(true);
        // The display handle taken BY VALUE so the wasm future can own it 'static.
        let display_handle = event_loop.owned_display_handle();

        // NATIVE: the main thread is free to block on GPU init (pollster), so the
        // GPU is ready synchronously and we finish init inline.
        #[cfg(not(target_arch = "wasm32"))]
        match pollster::block_on(Gpu::new(window, display_handle)) {
            Ok(gpu) => {
                self.gpu = Some(gpu);
                self.on_gpu_ready();
            }
            Err(e) => {
                eprintln!("failed to init render state: {e}");
                event_loop.exit();
            }
        }

        // WASM: the browser main thread CANNOT block, so adapter/device request is
        // an async that we drive on the microtask queue via `spawn_local`. The
        // finished GPU is parked in a shared slot; the trailing `request_redraw`
        // wakes `window_event`, which installs it and runs `on_gpu_ready` on the
        // first frame. (The event-loop borrow can't cross the await, hence the slot.)
        #[cfg(target_arch = "wasm32")]
        {
            let slot = self.gpu_pending.clone();
            let win = window.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match Gpu::new(window, display_handle).await {
                    Ok(gpu) => {
                        *slot.borrow_mut() = Some(gpu);
                        win.request_redraw();
                    }
                    Err(e) => log::error!("failed to init render state: {e}"),
                }
            });
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        // WASM: install the GPU the async init parked in the shared slot (its
        // trailing `request_redraw` is what delivered us here). The first frame
        // after init lands here with `gpu` still `None` but the slot full.
        #[cfg(target_arch = "wasm32")]
        if self.gpu.is_none() {
            // Take into a local FIRST so the `RefCell` borrow is dropped before
            // `on_gpu_ready` re-borrows `self`.
            let pending = self.gpu_pending.borrow_mut().take();
            if let Some(gpu) = pending {
                self.gpu = Some(gpu);
                self.on_gpu_ready();
            }
        }
        if self.gpu.is_none() {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(false) => {
                // ROBUST AUTOSAVE: the window lost focus (the user switched away);
                // flush a pending note write now so a note is never left unsaved
                // behind another app — and flush the document autosave / scratch
                // stash on the same trigger (locked decision: save on blur).
                self.flush_note();
                self.autosave_flush();
                // SESSION RESTORE: persist the open-file set / active buffer /
                // cursor+scroll / window frame on the SAME blur trigger the
                // autosave engine uses (native only; kill-switch gated inside).
                #[cfg(not(target_arch = "wasm32"))]
                self.session_flush();
                // POINTER AUTO-HIDE: a focus change must never leave the OS
                // pointer hidden behind another app — reset to Visible on blur
                // too, on the same trigger as the autosave flush above.
                let prev_pointer_hide = self.pointer_hide;
                self.pointer_hide = crate::pointer_hide::PointerHide::Visible;
                if let Some(visible) =
                    crate::pointer_hide::os_visibility_change(prev_pointer_hide, self.pointer_hide)
                {
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.window.set_cursor_visible(visible);
                    }
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                }
                self.sync_view(true);
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // The window moved to a monitor with a different DPI. Refold the new
                // scale into the metrics; a paired `Resized` (physical size change)
                // follows to re-wrap the column. Both keep the page proportioned.
                let sf = scale_factor as f32;
                self.dpi = sf;
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.pipeline.set_dpi(sf);
                }
                self.sync_view(true);
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                self.mods = m;
                // A held stats HUD is a true momentary hold: releasing a SUMMONING
                // modifier (e.g. lifting Cmd of Cmd-I) breaks the chord and dismisses it,
                // covering the macOS case where the character key-UP is never delivered.
                self.hud_release_on_mods(m.state());
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_px = (position.x as f32, position.y as f32);
                // POINTER AUTO-HIDE: ANY mouse motion snaps back to Visible instantly —
                // cancels a pending typing-hide countdown and un-hides an already-hidden
                // pointer in the same move (`pointer_hide::on_mouse_move` is always
                // `-> Visible`). `os_visibility_change` decides whether that crossed the
                // hidden/visible boundary, so `set_cursor_visible` is only ever called on
                // an actual change.
                let prev_pointer_hide = self.pointer_hide;
                self.pointer_hide = crate::pointer_hide::on_mouse_move(prev_pointer_hide);
                if let Some(visible) =
                    crate::pointer_hide::os_visibility_change(prev_pointer_hide, self.pointer_hide)
                {
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.window.set_cursor_visible(visible);
                    }
                }
                // A summoned picker OWNS the pointer (it is modal, the doc receding
                // behind it): a hover moves + previews the row under the cursor, exactly
                // like an arrow move. A live PAGE-WIDTH resize drag owns the pointer next
                // (the grabbed column edge tracks it, re-wrapping live); otherwise a live
                // text selection extends.
                if self.overlay.is_some() {
                    self.overlay_hover();
                } else if self.page_resizing {
                    self.on_page_resize_drag();
                } else if self.dragging {
                    self.on_drag();
                    self.sync_view(true);
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.window.request_redraw();
                    }
                }
                // CONTEXT-AWARE CURSOR SHAPE: recompute on every move regardless of which
                // branch above fired (a text-selection drag still reads as "over text",
                // an overlay hover still reads as the plain arrow, …) — one decision, not
                // a per-branch special case. See `cursor_shape.rs`.
                self.sync_cursor_icon();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                // DEBUG key→px: a mouse press is input awaiting pixels too — it
                // shares the request_redraw path (left falls through to it below;
                // right redraws inside `on_right_press`). Other buttons return
                // without a frame, so they are not stamped.
                if state == ElementState::Pressed
                    && matches!(button, MouseButton::Left | MouseButton::Right)
                {
                    self.stamp_input();
                }
                // RIGHT-CLICK → spell suggestions: hit-test + place the cursor at the
                // word under the pointer (same hit_test as a left-click), then fire the
                // EXISTING spell-suggestion picker. On a misspelled word it lists
                // corrections; elsewhere it's a calm no-op. Reuses suggest_at /
                // OpenSpellSuggest wholesale — no new spell logic.
                if button == MouseButton::Right {
                    if state == ElementState::Pressed {
                        self.on_right_press(event_loop);
                    }
                    return;
                }
                if button != MouseButton::Left {
                    return;
                }
                match state {
                    ElementState::Pressed => {
                        // A summoned picker OWNS the click (modal): a click ON a row
                        // ACCEPTS it (same as Enter), a click OUTSIDE the card DISMISSES
                        // it (same as Esc), a click inside but off a row is swallowed —
                        // it never falls through to move the document cursor beneath the
                        // card. Otherwise: a press ON a page-column edge begins a DIRECT
                        // width resize (symmetric about center) instead of a text
                        // selection; else it's a normal click / selection start.
                        if self.overlay.is_some() {
                            self.overlay_click(event_loop);
                        } else if !self.begin_page_resize_if_hovering(event_loop) {
                            let shift = self.mods.state().contains(ModifiersState::SHIFT);
                            self.on_press(shift);
                            self.sync_view(true);
                        }
                    }
                    ElementState::Released if self.page_resizing => {
                        // Commit + persist the settled page width (sticky).
                        self.end_page_resize();
                    }
                    ElementState::Released => {
                        self.dragging = false;
                        // A plain click (press + release with no drag) leaves the
                        // press-time anchor lingering at the cursor. Collapse it so
                        // a subsequent bare motion (C-p, C-n, …) just moves the
                        // cursor and does NOT extend a phantom selection. A real
                        // drag (or double/triple-click) leaves cursor != anchor,
                        // i.e. has_selection(), so its mark is preserved.
                        if !self.buffer.has_selection() {
                            self.buffer.clear_mark();
                        }
                        self.sync_view(true);
                    }
                }
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // DEBUG key→px: scroll is input awaiting pixels — every wheel
                // path below ends in the arm's request_redraw.
                self.stamp_input();
                // Zoom modifier: Cmd/Super only. (Ctrl must NOT zoom on mac.)
                let zoom_mod = scroll_zoom_intent(self.mods.state());
                // Convert the delta to a line count (LineDelta or PixelDelta).
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * WHEEL_LINES_PER_NOTCH,
                    MouseScrollDelta::PixelDelta(p) => {
                        self.scroll_px_accum += p.y as f32;
                        let whole = (self.scroll_px_accum / WHEEL_PIXELS_PER_LINE).trunc();
                        self.scroll_px_accum -= whole * WHEEL_PIXELS_PER_LINE;
                        whole
                    }
                };
                if self.overlay.is_some() {
                    // A summoned picker OWNS the wheel (it is modal): wheel drives the
                    // LIST (advance the selection/scroll window, like ↑/↓); the document
                    // behind it does NOT scroll. Symmetric with the click/hover consume.
                    if lines.abs() >= 1.0 {
                        self.overlay_wheel(lines);
                    }
                } else if zoom_mod {
                    // Cmd/Super + wheel: zoom in/out (wheel up = zoom in).
                    if lines.abs() >= 1.0 {
                        let dir = lines.signum();
                        self.set_zoom(self.zoom + dir * render::ZOOM_STEP);
                        self.sync_view(true);
                    }
                } else if lines.abs() >= 1.0 {
                    // Free scroll: wheel up moves content down (scroll up), so a
                    // positive wheel y DECREASES the top scroll line.
                    self.wheel_scroll(-lines);
                    self.sync_view(false);
                }
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            WindowEvent::Ime(ime) => {
                self.handle_ime(ime);
                self.sync_view(true);
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    // KEY RELEASE: the only release awl acts on is lifting the HELD
                    // stats-HUD key — a true hold, dismissed the instant it lifts. The
                    // press recorded the trigger key in `hud_key`; releasing the SAME
                    // logical key clears the HUD and re-syncs so it vanishes. Every
                    // other release stays a no-op.
                    if event.state == ElementState::Released {
                        self.on_key_release(&event.logical_key);
                    }
                    return;
                }
                // While composing (a non-empty preedit), the IME owns these keys:
                // they are delivered separately as Ime::Preedit/Commit, so do NOT
                // also route them through the keymap (which would insert raw
                // romaji or move the cursor mid-composition). This guard runs
                // BEFORE the search guard on purpose: the IME wins over search,
                // and because C-s is swallowed here, a search cannot start
                // mid-composition.
                if !self.preedit.is_empty() {
                    return;
                }
                // Ignore lone modifier presses.
                if let Key::Named(n) = &event.logical_key {
                    use winit::keyboard::NamedKey::*;
                    if matches!(n, Control | Shift | Alt | Super | Hyper | Meta) {
                        return;
                    }
                }
                // DEBUG key→px: stamp the dispatch receipt of a real key press —
                // every path from here (search keys, rebind capture, the keymap
                // resolve → apply) ends in request_redraw, so this key's pixels
                // are coming. Placed AFTER the lone-modifier/preedit filters: a
                // bare Ctrl tap or an IME-owned key causes no frame and must not
                // linger as a stale stamp inflating the next input's latency.
                self.stamp_input();
                // POINTER AUTO-HIDE: a real keystroke (past the lone-modifier/IME
                // filters above, same gate `stamp_input` uses) hides the OS
                // pointer IMMEDIATELY — the macOS-native convention
                // (`NSCursor.setHiddenUntilMouseMoves`). Any mouse motion
                // instantly reverses it (the `CursorMoved` arm above); so does
                // the window losing focus (the `Focused(false)` arm above).
                let prev_pointer_hide = self.pointer_hide;
                self.pointer_hide = crate::pointer_hide::on_key(prev_pointer_hide);
                if let Some(visible) =
                    crate::pointer_hide::os_visibility_change(prev_pointer_hide, self.pointer_hide)
                {
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.window.set_cursor_visible(visible);
                    }
                }
                // SEARCH GUARD: when isearch is active, EVERY key (printable,
                // Backspace, Enter, Esc, C-s, C-r, M-c) is consumed by the search
                // surface and never reaches the keymap, so printable keys extend
                // the query instead of inserting into the rope. Placed AFTER the
                // lone-modifier filter (so a bare Shift/Ctrl tap during search is
                // dropped) and AFTER the preedit guard, but BEFORE keymap.resolve.
                if self.search.is_some() {
                    let mods = self.mods;
                    self.handle_search_key(&event.logical_key, &mods, event_loop);
                    self.sync_view(true);
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.window.request_redraw();
                    }
                    return;
                }
                // REBIND MENU live CAPTURE: while the menu is RECORDING, the next press
                // IS the binding — intercepted at the CHORD level, BEFORE keymap
                // resolution, so any combo (C-t / M-f / a bare key) is recorded verbatim
                // rather than run. Enter / Esc are EXCLUDED (they finish / abort the
                // capture via the normal resolve → apply_core path below). Option
                // composition is undone (like the dead-key fix) so Option-f records as
                // M-f, not the composed glyph. The headless replay records PLAIN keys
                // through `apply_core` instead; both call `OverlayState::capture_record`.
                if self.capture_recording() {
                    let is_ctrl_key = matches!(
                        &event.logical_key,
                        Key::Named(winit::keyboard::NamedKey::Enter)
                            | Key::Named(winit::keyboard::NamedKey::Escape)
                    );
                    if !is_ctrl_key {
                        let logical = if self.mods.state().contains(ModifiersState::ALT) {
                            key_without_modifiers(&event)
                        } else {
                            event.logical_key.clone()
                        };
                        let combo = crate::keyspec::format_chord(&logical, self.mods.state());
                        let finished = self
                            .overlay
                            .as_mut()
                            .map(|o| o.capture_record(combo))
                            .unwrap_or(false);
                        if finished {
                            if let Some((slug, binding)) =
                                self.overlay.as_ref().and_then(|o| o.capture_target())
                            {
                                self.rebind_commit(slug, binding, false);
                            }
                        }
                        self.sync_view(true);
                        if let Some(gpu) = self.gpu.as_ref() {
                            gpu.window.request_redraw();
                        }
                        return;
                    }
                }
                // Held arrow / motion keys arrive as OS AUTO-REPEAT events
                // (`event.repeat`). Record it for the next `sync_view` so a held
                // navigation move builds a continuous lagging caret trail, while a
                // discrete tap (`repeat == false`) stays gap-suppressed.
                self.caret_held = event.repeat;
                // macOS OPTION DEAD-KEY FIX (LIVE path only): Option composes a
                // letter into a glyph (Option-f -> 'ƒ'), so `event.logical_key` is the
                // composed char and a Meta chord (M-f / M-b / M-w / M-v / M-< / M->)
                // would never match. When ALT is held, resolve the UN-composed key
                // (`key_without_modifiers`) IF it is a real Meta chord; otherwise keep
                // the composed `logical_key` so Option-accent INPUT (Option-e -> é)
                // still types as text. The headless `--keys` replay already sends the
                // un-composed key + ALT, so this branch is exercised only live (its
                // behaviour with a real composing keyboard needs human confirmation).
                let logical = if self.mods.state().contains(ModifiersState::ALT) {
                    let bare = key_without_modifiers(&event);
                    if self.keymap.is_meta_chord(&bare) {
                        bare
                    } else {
                        event.logical_key.clone()
                    }
                } else {
                    event.logical_key.clone()
                };
                let action = self.keymap.resolve(&logical, &self.mods);
                // WHICH-KEY prefix tracking: read the keymap's post-resolve prefix state.
                // Pressing `C-x` (BeginPrefix) leaves it MID-PREFIX → arm the pause timer
                // (record when, so `about_to_wait` can summon the panel after the pause);
                // any other key resolves/aborts the prefix → dismiss the panel + disarm.
                // Cheap no-op on the common (no-prefix) key.
                self.sync_whichkey_prefix();
                // HELD stats HUD: remember the trigger key AND the modifiers held at
                // summon, so its RELEASE dismisses the HUD — either the key lifting
                // (`on_key_release`) or a summoning modifier dropping (`hud_release_on_mods`,
                // the macOS case where the letter's key-UP never arrives while Cmd is down).
                // The press itself summons it via `apply_core` (sets the process-global); an
                // OS auto-repeat re-affirms the same key/mods.
                if action == Action::ShowStatsHud {
                    self.hud_key = Some(logical.clone());
                    self.hud_mods = self.mods.state();
                }
                // `M-<` / `M->` need Shift just to TYPE `<` / `>`, so that Shift is
                // INCIDENTAL — it must NOT extend the selection (Emacs treats these
                // as pure motion; select via the mark, `C-Space`). Strip it for those
                // two actions before it reaches the Shift+motion select logic.
                let shift = self.mods.state().contains(ModifiersState::SHIFT)
                    && motion_honors_shift_select(&action);
                let exited = self.apply(action, shift, event_loop);
                if exited {
                    return;
                }
                self.sync_view(true);
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                // Advance the caret spring by the real elapsed time since the
                // last animated frame, then draw. If still animating, keep the
                // loop hot (Poll + request another redraw); once settled, go
                // back to Wait so the app idles at 0% CPU until the next input.
                let now = Instant::now();
                let dt = match self.last_frame {
                    Some(prev) => (now - prev).as_secs_f32(),
                    // First animated frame: assume one 60fps tick so the very
                    // first step is sane rather than a huge dt.
                    None => 1.0 / 60.0,
                };
                // Monotonic frames-drawn count (a single add, always — so toggling
                // debug on mid-hot-loop shows a climbing count immediately).
                self.redraw_count += 1;
                // DEBUG panel feed — the ONLY timing work, and all of it gated on
                // the panel being on (the pane never creates the work it measures;
                // the pane-off editor takes zero clock reads). The panel text is
                // shaped inside `pipeline.prepare`, so the values are fed at the
                // TOP of the redraw, BEFORE `gpu.redraw()`: line 1 therefore shows
                // the PREVIOUS completed frame's cost (one-frame lag — this frame's
                // cost isn't knowable until it presents).
                let mut is_stamp = false;
                if crate::debug::debug_on() {
                    // Classify this redraw: a pending un-rendered input wins over a
                    // queued stamp; any redraw out of stillness is activity. Only a
                    // quiet StampQueued redraw IS the settle-stamp frame, which
                    // draws the still-prefixed readout.
                    self.debug_still =
                        crate::debug::still_wake(self.debug_still, self.input_stamp.is_some());
                    is_stamp = self.debug_still == crate::debug::DebugStill::StampQueued;
                    if let Some(gpu) = self.gpu.as_mut() {
                        // ADAPTIVE budget: one vsync of the monitor this window is
                        // on (16.6 at 60 Hz, 8.3 at 120 Hz; 60 Hz fallback when
                        // winit can't name a rate).
                        let budget = crate::debug::budget_ms(
                            gpu.window
                                .current_monitor()
                                .and_then(|m| m.refresh_rate_millihertz()),
                        );
                        let cost = self
                            .frame_costs
                            .last()
                            .and_then(|l| self.frame_costs.worst().map(|w| (l, w)));
                        gpu.pipeline.set_debug_perf(
                            cost,
                            self.last_latency_ms,
                            Some(self.redraw_count),
                            is_stamp,
                            Some(budget),
                        );
                        // Also surface the live GPU memory (macOS: Metal's
                        // currentAllocatedSize; `None` elsewhere → `gpu —`).
                        let bytes = gpu.current_gpu_bytes();
                        gpu.pipeline.set_debug_gpu_bytes(bytes);
                        // AUTOSAVE-ENGINE line: composed EXCLUSIVELY from what
                        // `App::autosave_flush`'s one door already tracks — config's
                        // `autosave_on()`, the clobber guard's `notice`, and the
                        // engine's own last-write clock — so it can never say
                        // anything the engine didn't just do. The only clock read
                        // here (`Instant::now() - autosave_last_ok`) is gated on
                        // `debug_on()` like every other perf read this block makes.
                        let since_secs = self
                            .autosave_last_ok
                            .map(|t| (Instant::now() - t).as_secs());
                        let autosave = crate::debug::autosave_state(
                            self.config.autosave_on(),
                            self.notice.is_some(),
                            since_secs,
                        );
                        gpu.pipeline.set_debug_autosave(Some(autosave));
                    }
                } else if self.input_stamp.is_some()
                    || self.last_latency_ms.is_some()
                    || self.frame_costs.last().is_some()
                {
                    // Panel just turned off: forget the measurements so the next
                    // enable starts fresh (placeholders, then live numbers), and
                    // re-feed the pipeline defaults (still-form placeholders).
                    self.input_stamp = None;
                    self.last_latency_ms = None;
                    self.frame_costs.clear();
                    self.debug_still = crate::debug::DebugStill::Active;
                    if let Some(gpu) = self.gpu.as_mut() {
                        gpu.pipeline.set_debug_perf(None, None, None, true, None);
                        gpu.pipeline.set_debug_autosave(None);
                    }
                }
                // A STATIC open overlay must NOT busy-loop: an idle menu is a frozen
                // frame, so forcing ControlFlow::Poll just because an overlay is open
                // re-ran prepare_overlay/set_rich_text every frame, pegging the CPU.
                // Instead the overlay redraws ON INPUT — every overlay-affecting key
                // (query edit, selection move, filter, open/close) is a KeyboardInput
                // event that routes through `apply` and then calls request_redraw
                // below, and OS key AUTO-REPEAT for a HELD arrow delivers a fresh
                // KeyboardInput per repeat, so a held arrow still repaints promptly.
                // The loop only stays HOT while the caret spring is still animating.
                let (animating, presented) = if let Some(gpu) = self.gpu.as_mut() {
                    // Drive the virtual-clock seam (caret spring + any future live
                    // animator) so the timeline capture and the live loop advance
                    // animation through the SAME entry point.
                    let still = gpu.pipeline.advance(dt);
                    let presented = gpu.redraw();
                    // Once the spring settles the caret is fully static (the I-beam no
                    // longer breathes) and there is nothing else animating, so the loop
                    // idles at 0% CPU until the next input requests a redraw.
                    (still, presented)
                } else {
                    (false, None)
                };
                // DEBUG bookkeeping for the frame that just PRESENTED (`presented`
                // is `Some` only with the panel on — see `Gpu::redraw`): close the
                // key→px span at present-return, and push the measured cost into
                // the ring for the NEXT frame's line 1 — except on the stamp frame,
                // whose cost is measured and DISCARDED (panel bookkeeping, not user
                // workload; displaying it would take yet another frame). An
                // early-return redraw (`None`) keeps the input stamp alive so the
                // latency measures through to the retry frame that really presents.
                if let Some((cost_ms, done)) = presented {
                    if let Some(stamp) = self.input_stamp.take() {
                        self.last_latency_ms = Some((done - stamp).as_secs_f32() * 1000.0);
                    }
                    if !is_stamp {
                        self.frame_costs.push(cost_ms);
                    }
                }

                // Keep the loop hot ONLY while the spring animates — the debug panel
                // schedules ZERO frames of its own (every metric it shows is
                // meaningful for a single sparse frame). The held stats HUD does NOT
                // force frames either: its figures are pure functions of the doc
                // (no session clock), so a held HUD is a single settled frame over
                // the cached frosted backdrop. `last_frame` still tracks ONLY the
                // spring, so the dt fed to `advance` stays correct.
                let keep_hot = animating;
                self.last_frame = if animating { Some(now) } else { None };
                if keep_hot {
                    event_loop.set_control_flow(ControlFlow::Poll);
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.window.request_redraw();
                    }
                } else {
                    // Settled: stop driving frames and idle until next input.
                    event_loop.set_control_flow(ControlFlow::Wait);
                }
                // DEBUG settle-stamp: the first redraw that ends SETTLED while the
                // panel is on queues exactly ONE more frame — the stamp that draws
                // the `still ·` readout with the final true numbers — and then the
                // machine goes fully quiet (the stamp itself requests nothing).
                // Control flow stays `Wait`; `request_redraw` alone delivers the
                // one frame. New input meanwhile simply wins (see `still_wake`).
                if crate::debug::debug_on() {
                    let (next, request_stamp) =
                        crate::debug::still_settle(self.debug_still, animating);
                    self.debug_still = next;
                    if request_stamp {
                        if let Some(gpu) = self.gpu.as_ref() {
                            gpu.window.request_redraw();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// The event loop is exiting (quit / window closed): flush any pending note
    /// save — and the document autosave / scratch stash — so nothing typed right
    /// before quit is lost. The final safety net of the robust-autosave guarantee.
    /// Also the daemon's clean-shutdown door: flush every outstanding `--wait`
    /// connection + unlink the socket special file (native only).
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.flush_note();
        self.autosave_flush();
        // SESSION RESTORE: the final safety net, mirroring the autosave flush
        // right above it (native only; kill-switch gated inside).
        #[cfg(not(target_arch = "wasm32"))]
        self.session_flush();
        #[cfg(not(target_arch = "wasm32"))]
        self.daemon_shutdown();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // WHICH-KEY pause: while a PREFIX (`C-x`) is pending its second key, summon the
        // continuation panel once ~500ms elapses without a follow-up. The timer is
        // ARMED ONLY here, while `prefix_pending_at` is `Some` AND the panel isn't yet
        // shown — a single `WaitUntil` deadline, no perpetual per-frame tick; once it
        // fires (or the prefix resolves, clearing `prefix_pending_at`) nothing re-arms,
        // so the app idles at 0% CPU (DESIGN §6).
        if let Some(pending) = self.prefix_pending_at {
            let deadline = pending + crate::whichkey::PAUSE;
            let elapsed = Instant::now() >= deadline;
            if crate::whichkey::should_summon(true, self.whichkey_shown, elapsed) {
                self.summon_whichkey();
            } else if !self.whichkey_shown && !elapsed && self.last_frame.is_none() {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
        }
        // Debounced spell check: re-scan only after ~150ms with no edits, so a
        // word isn't squiggled while you're still typing it.
        if let Some(dirty) = self.spell_dirty_at {
            let deadline = dirty + SPELL_DEBOUNCE;
            if Instant::now() >= deadline {
                self.run_spellcheck_now();
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            } else if self.last_frame.is_none() {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
        }
        // Debounced quick-note AUTO-SAVE: write the note after ~400ms of quiet, so
        // it persists calmly as you pause. An empty note writes nothing.
        if let Some(dirty) = self.autosave_dirty_at {
            let deadline = dirty + AUTOSAVE_DEBOUNCE;
            if Instant::now() >= deadline {
                self.autosave_dirty_at = None;
                self.autosave_note();
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            } else if self.last_frame.is_none() {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
        }
        // Debounced DOCUMENT AUTOSAVE (the config-gated engine, default ON): the
        // open file is written atomically — or the no-path scratch stashed — after
        // ~1s of idle. Armed ONLY by the live `sync_view` (behind its gpu-present
        // gate), consumed here via the same single-`WaitUntil` pattern as the note
        // autosave above — no hot loop, and structurally unreachable headlessly.
        if let Some(dirty) = self.doc_autosave_at {
            match debounce_due(dirty, AUTOSAVE_IDLE, Instant::now()) {
                true => {
                    self.doc_autosave_at = None;
                    self.autosave_flush();
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.window.request_redraw();
                    }
                }
                false if self.last_frame.is_none() => {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(dirty + AUTOSAVE_IDLE));
                }
                false => {}
            }
        }
        // Debounced theme-preview FONT reshape: while the theme picker's live preview
        // arrows across worlds, only the COLORS applied per step; once the selection
        // rests `THEME_FONT_DEBOUNCE` the one deferred reshape lands here (the paused
        // hover then shows the true face). Each further preview step RE-STAMPS
        // `theme_font_at` (`retint_theme_preview`), sliding the deadline — the same
        // single-`WaitUntil`, idle-safe pattern as the zoom persist below (no hot
        // loop; commit/revert clear the stamp synchronously via `retint_theme_now`).
        if let Some(dirty) = self.theme_font_at {
            match debounce_due(dirty, THEME_FONT_DEBOUNCE, Instant::now()) {
                true => self.apply_deferred_theme_font(),
                false if self.last_frame.is_none() => {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(dirty + THEME_FONT_DEBOUNCE));
                }
                false => {}
            }
        }
        // Debounced STICKY-ZOOM write: persist the SETTLED zoom after ~500ms of quiet,
        // so a rapid Cmd-=/Cmd-- run writes the final value once (not one-per-step).
        // Each new zoom step RE-STAMPS `zoom_persist_at` (via `mark_zoom_dirty`), so the
        // deadline keeps sliding forward until the user pauses — the debounce contract.
        if let Some(dirty) = self.zoom_persist_at {
            match debounce_due(dirty, ZOOM_PERSIST_DEBOUNCE, Instant::now()) {
                true => {
                    self.zoom_persist_at = None;
                    self.persist_zoom_now();
                }
                false if self.last_frame.is_none() => {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(dirty + ZOOM_PERSIST_DEBOUNCE));
                }
                false => {}
            }
        }
    }
}

/// Has a DEBOUNCE window elapsed? `dirty` is when the action was last seen, `window`
/// the quiet period to wait, `now` the current instant: true once `now` has reached
/// `dirty + window` (fire the deferred write), false while still inside the window
/// (keep waiting — a fresh action re-stamps `dirty`, sliding the deadline). Pure, so
/// the debounce decision is unit-testable without an event loop.
fn debounce_due(dirty: Instant, window: Duration, now: Instant) -> bool {
    now.saturating_duration_since(dirty) >= window
}

/// Does this modifier set request wheel-zoom? Cmd/Super only (NOT Ctrl), so a
/// Ctrl+scroll falls through to normal free scrolling. Pure, so it's unit-testable
/// without a window/event loop.
fn scroll_zoom_intent(mods: ModifiersState) -> bool {
    mods.contains(ModifiersState::SUPER)
}

/// Has the held stats HUD's summon chord been BROKEN by a modifier release? The HUD is a
/// momentary hold: `summon` is the modifier set held when it was summoned, `now` is the
/// current set. Any summoning modifier dropping (so `now` no longer CONTAINS all of
/// `summon`) breaks the hold and must dismiss the HUD — this is the macOS path where the
/// trigger letter's key-UP is never delivered while Cmd is down. Pressing EXTRA modifiers
/// (a superset) does not break it. Pure, so it's unit-testable without a window.
fn hud_mods_broken(summon: ModifiersState, now: ModifiersState) -> bool {
    !now.contains(summon)
}

/// Does a held Shift on this action signal SELECT-INTENT (Shift+motion extends
/// the selection, GUI style)? `M-<` / `M->` need Shift just to TYPE the `<` /
/// `>` glyph, so that Shift is INCIDENTAL — Emacs treats them as pure motion
/// (you select via the mark, `C-Space`), so it must NOT extend the selection.
/// Every other action keeps Shift's normal select-extend meaning. Pure, so it's
/// unit-testable without a window/event loop.
fn motion_honors_shift_select(action: &Action) -> bool {
    !matches!(action, Action::BufferStart | Action::BufferEnd)
}

/// The UN-composed logical key for a key event — undoing macOS Option dead-key
/// composition (Option-f -> 'ƒ') so Meta chords resolve. On the desktop backends
/// this defers to winit's `KeyEventExtModifierSupplement::key_without_modifiers`;
/// the web backend has no such composition layer (and doesn't expose the trait),
/// so on wasm the plain logical key already IS the un-composed key.
#[cfg(not(target_arch = "wasm32"))]
fn key_without_modifiers(event: &winit::event::KeyEvent) -> Key {
    event.key_without_modifiers()
}
#[cfg(target_arch = "wasm32")]
fn key_without_modifiers(event: &winit::event::KeyEvent) -> Key {
    event.logical_key.clone()
}

/// Run the windowed editor for an optional file with an active project `root`
/// (and optional `workspace` parent for switch-project). `wait` is the raw
/// `--wait` flag (native-only meaning — see `crate::daemon`'s module doc for
/// the documented scope of what it does and doesn't block on); ignored on wasm.
pub fn run(
    file: Option<PathBuf>,
    root: PathBuf,
    cli_workspace: Option<PathBuf>,
    cli_notes_root: Option<PathBuf>,
    config: Config,
    wait: bool,
) -> anyhow::Result<()> {
    // SINGLE-INSTANCE DAEMON (native only — see `crate::daemon`'s module doc
    // for the full CAPTURE GATE argument: this whole block lives ONLY on this
    // live-App startup path, never on any headless `--screenshot`/`--bench-*`
    // mode). Runs the bind-or-handoff dance BEFORE any window/GPU work, so
    // handing off to an already-running instance exits in milliseconds with no
    // window ever created.
    #[cfg(not(target_arch = "wasm32"))]
    let instance_listener = match crate::daemon::startup(file.as_deref(), wait) {
        Ok(crate::daemon::StartupOutcome::HandedOff) => return Ok(()),
        Ok(crate::daemon::StartupOutcome::Instance(l)) => Some(l),
        Err(e) => {
            // Never let a socket hiccup (permissions, a full /tmp, a bad XDG
            // path, …) block opening the editor — degrade to a normal,
            // non-singleton launch.
            eprintln!("awl: single-instance socket unavailable ({e}); continuing without it");
            None
        }
    };

    let event_loop = EventLoop::<AwlEvent>::with_user_event().build()?;
    #[cfg(not(target_arch = "wasm32"))]
    let proxy = event_loop.create_proxy();
    let mut app = App::new(file, root, cli_workspace, cli_notes_root, config);
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(listener) = instance_listener {
        app.daemon_socket_path = Some(crate::daemon::socket_path());
        crate::daemon::spawn_accept_thread(listener, proxy);
    }

    // NATIVE: `run_app` blocks this thread driving the OS event loop to exit.
    #[cfg(not(target_arch = "wasm32"))]
    {
        event_loop.run_app(&mut app)?;
    }

    // WASM: the browser event loop is the page's own; winit can't BLOCK on it, so
    // `spawn_app` hands the App to requestAnimationFrame and returns immediately
    // (control goes back to JS). The app then lives for the page's lifetime.
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        let _ = wait; // no daemon on wasm; the flag is a native-only concern
        event_loop.spawn_app(app);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_zoom_only_on_super() {
        // Cmd/Super => zoom.
        assert!(scroll_zoom_intent(ModifiersState::SUPER));
        // Ctrl must NOT zoom (the mac bug fix): falls through to free scroll.
        assert!(!scroll_zoom_intent(ModifiersState::CONTROL));
        // No modifiers => no zoom.
        assert!(!scroll_zoom_intent(ModifiersState::empty()));
        // Cmd+Shift still zooms.
        assert!(scroll_zoom_intent(
            ModifiersState::SUPER | ModifiersState::SHIFT
        ));
    }

    #[test]
    fn zoom_debounce_fires_only_after_the_quiet_window() {
        // The STICKY-ZOOM debounce decision: while inside the window the write is
        // deferred (so a rapid Cmd-=/Cmd-- run that re-stamps `dirty` keeps sliding the
        // deadline), and it fires once the window has fully elapsed. Drives the SAME
        // `debounce_due` the `about_to_wait` zoom branch uses.
        let win = ZOOM_PERSIST_DEBOUNCE;
        let dirty = Instant::now();
        // Just after a zoom step: not yet due (still within the quiet window).
        assert!(!debounce_due(dirty, win, dirty));
        assert!(!debounce_due(dirty, win, dirty + win / 2));
        // A fresh step RE-STAMPS dirty later, so an earlier 'now' is still not due —
        // the debounce slides forward instead of firing mid-run.
        let restamped = dirty + win; // a later zoom step moved the stamp
        assert!(!debounce_due(restamped, win, dirty + win)); // now == new dirty: not due
        // Once a FULL quiet window has passed since the last step, it fires.
        assert!(debounce_due(dirty, win, dirty + win));
        assert!(debounce_due(dirty, win, dirty + win + Duration::from_millis(1)));
    }

    #[test]
    fn held_hud_dismisses_when_summon_modifier_lifts() {
        // The stats HUD is a momentary HOLD: summoned with Cmd-I, it must vanish the
        // instant the chord lifts. macOS does not deliver the 'i' key-UP while Cmd is
        // down, so dismissal rides the modifier release instead — this pure predicate is
        // the state machine: pressed-with-Super, then Super gone => clear.
        let summon = ModifiersState::SUPER;
        // Cmd still held (no change, or an OS auto-repeat) => HUD stays.
        assert!(!hud_mods_broken(summon, ModifiersState::SUPER));
        // Cmd RELEASED (mods now empty) => the hold is broken, HUD clears.
        assert!(hud_mods_broken(summon, ModifiersState::empty()));
        // Adding an EXTRA modifier (Cmd+Shift) is still a superset => HUD stays.
        assert!(!hud_mods_broken(
            summon,
            ModifiersState::SUPER | ModifiersState::SHIFT
        ));
        // Swapping Cmd for a different modifier still breaks the summon set => clear.
        assert!(hud_mods_broken(summon, ModifiersState::CONTROL));
        // A no-modifier summon (a rebind to a bare key) is never broken by mods alone;
        // that hold is dismissed by the key-UP path (`on_key_release`) instead.
        assert!(!hud_mods_broken(ModifiersState::empty(), ModifiersState::empty()));
        assert!(!hud_mods_broken(ModifiersState::empty(), ModifiersState::SUPER));
    }

    #[test]
    fn buffer_endpoints_ignore_incidental_shift() {
        // `M-<` / `M->` need Shift just to TYPE `<` / `>`, so that Shift is
        // incidental and must NOT extend the selection — these are pure motion.
        assert!(!motion_honors_shift_select(&Action::BufferStart));
        assert!(!motion_honors_shift_select(&Action::BufferEnd));
        // Every other motion keeps Shift's normal select-extend meaning (the user
        // deliberately held Shift, e.g. Shift+Arrow / M-Shift-f).
        assert!(motion_honors_shift_select(&Action::ForwardChar));
        assert!(motion_honors_shift_select(&Action::ForwardWord));
        assert!(motion_honors_shift_select(&Action::NextLine));
        assert!(motion_honors_shift_select(&Action::LineEnd));
        // Non-motions are unaffected (Shift is ignored by the motion-select logic
        // for them anyway), so they report the default true.
        assert!(motion_honors_shift_select(&Action::InsertChar('a')));
    }

    #[test]
    fn double_click_bumps_the_shared_click_counter_that_also_backs_the_edge_reset() {
        // `bump_click_count` is the ONE shared multi-click detector: a plain
        // document press (`on_press`: a double-click selects a word, a triple
        // selects a line) and a press on the draggable PAGE EDGE
        // (`begin_page_resize_if_hovering`: a double-click there RESETS the width
        // instead of beginning a drag) both branch on its returned count — so
        // proving it reaches 2 on a fast same-spot double click proves the edge
        // gesture recognizes a double-click identically, without needing a live
        // GPU hover test (that half — routing through `App::apply` behind the
        // GPU-gated hover check — stays LIVE-ONLY, like the rest of the drag
        // gesture; the hover math itself is unit-tested in `render::geometry`).
        // No real file content is needed here, so build hermetically (closes
        // the session-restore + scratch-stash doors — see `new_hermetic`'s doc).
        let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
        app.cursor_px = (0.0, 0.0);
        assert_eq!(app.bump_click_count(), 1, "a first press starts a fresh count");
        assert_eq!(app.bump_click_count(), 2, "an immediate same-spot press doubles it");
        assert_eq!(app.bump_click_count(), 3, "a third same-spot press triples it");
        assert_eq!(app.bump_click_count(), 1, "a fourth wraps back to a fresh single click");
        // A press at a DIFFERENT spot never continues the run, however fast.
        app.bump_click_count();
        app.cursor_px = (500.0, 500.0);
        assert_eq!(app.bump_click_count(), 1, "a different spot starts over, not a double-click");
    }

    #[test]
    fn open_serves_the_new_files_text_despite_equal_buffer_versions() {
        // THE LIVE "open a file and it does not appear" BUG: every swapped-in buffer
        // restarts its edit version at 0, and `sync_view`'s rope-clone short-circuit
        // (`view_text`) is keyed by version ALONE — so opening a file from any
        // UN-EDITED buffer (also version 0) hit the stale cache and pushed the OLD
        // document's text to the renderer. The screen repainted, but with the old
        // content, until the first edit bumped the version. The headless capture
        // rebuilds its text per frame and never saw it. This drives the REAL open
        // arm (`load_path`, shared by Go-to-file / Browse / picker click / C-x b)
        // against the REAL cache seam, GPU-less.
        // Reads the REAL disk through the fs seam, so hold the fs TEST_LOCK: a
        // parallel test with an InMemoryFs installed would swallow these files.
        // Can't build hermetically (`App::new_hermetic` injects an empty
        // InMemoryFs, which would make `Buffer::from_file` find neither real
        // fixture below) — disable session restore explicitly instead, so
        // `apply_session_restore` never reads the developer's real
        // `~/.local/share/awl/session.toml` and parks his real open files into
        // this test's registry (the exact leak class `d93109e` fixed).
        let _fs = crate::fs::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("awl-open-swap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("old.txt");
        let new = dir.join("new.txt");
        std::fs::write(&old, "the OLD document\n").unwrap();
        std::fs::write(&new, "the NEW document\n").unwrap();
        let cfg = Config { session_restore: Some(false), ..Config::empty() };
        let mut app = App::new(Some(old), dir.clone(), None, None, cfg);
        // The first sync caches (version 0, old text) — the short-circuit at work.
        assert_eq!(app.view_text(), "the OLD document\n");
        assert_eq!(app.buffer.version(), 0, "an un-edited buffer sits at version 0");
        // Open the second file: a FRESH buffer, version 0 again — the cache key
        // collides. The view text MUST be the new file's, not the cached old one.
        app.load_path(new);
        assert_eq!(
            app.view_text(),
            "the NEW document\n",
            "the opened file's text must reach the view despite the version collision"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_note_drops_the_stale_view_text_cache() {
        // C-x n swaps in a fresh EMPTY note buffer (version 0 again): the previous
        // un-edited buffer's cached text (also version 0) must not survive the swap,
        // or the new note would render as the old document until the first keystroke
        // — the same version-collision as the open arm, on the note door.
        // Real-disk reads through the seam → hold the fs TEST_LOCK (see above),
        // and disable session restore for the same reason the sibling test
        // above does (can't build hermetically — this needs real file bytes).
        let _fs = crate::fs::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("awl-note-swap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("doc.txt");
        std::fs::write(&file, "prior document\n").unwrap();
        let notes = dir.join("notes");
        let cfg = Config { session_restore: Some(false), ..Config::empty() };
        let mut app = App::new(Some(file), dir.clone(), None, Some(notes), cfg);
        assert_eq!(app.view_text(), "prior document\n");
        app.new_note();
        assert_eq!(app.view_text(), "", "the fresh note starts blank on screen");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── The AUTOSAVE ENGINE (App-level, over the InMemoryFs seam) ───────────
    //
    // Each test installs a fake FS via FsGuard so App::new / the flush paths
    // never touch the real disk (or the developer's real scratch stash).

    /// An App over the installed fake FS, opened on `file` with project `root`.
    fn app_on(file: Option<PathBuf>, root: &str, config: Config) -> App {
        App::new(file, PathBuf::from(root), None, None, config)
    }

    // ── GOTO FILE-INDEX FRESHNESS (queue: "file picker freshness") ──────────
    //
    // The go-to overlay (`C-x f`) corpus comes from `App.file_index`, a CACHED
    // field only ever rebuilt on specific triggers (root switch, a note's first
    // save, a rename, a move) — never simply because the picker summoned. A file
    // dropped into the root by another process, or a shell command, while awl
    // sits open would never appear until one of those triggers happened to also
    // fire. The fix: RE-SCAN ON EVERY SUMMON via `App::rescan_file_index` (called
    // from `App::apply`'s `Action::OpenGoto` arm, over the `FileSystem` trait) —
    // no watcher, no TTL, just re-walk right as the overlay opens.

    #[test]
    fn rescan_file_index_picks_up_a_file_created_after_the_last_scan() {
        use crate::fs::{FileSystem, InMemoryFs};
        let mem = InMemoryFs::new().with_file("/proj/a.txt", "a\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(None, "/proj", Config::empty());
        // The initial scan (at App::new) sees only the file that existed then.
        assert_eq!(app.file_index, vec!["a.txt".to_string()]);
        // SUMMON #1 (simulated: `rescan_file_index` is exactly what `C-x f`
        // triggers): still just the one file — nothing has changed yet.
        app.rescan_file_index();
        assert_eq!(app.file_index, vec!["a.txt".to_string()]);
        // A file appears on disk WITHOUT going through awl at all (another
        // process, a git checkout, a plain `touch`) — the picker is CLOSED at
        // this point, so nothing in awl has any reason to know yet.
        mem.write(std::path::Path::new("/proj/b.txt"), b"b\n").unwrap();
        assert_eq!(
            app.file_index,
            vec!["a.txt".to_string()],
            "the cached index does not spontaneously update"
        );
        // SUMMON #2 (`C-x f` again): the fresh scan MUST find it.
        app.rescan_file_index();
        assert_eq!(
            app.file_index,
            vec!["a.txt".to_string(), "b.txt".to_string()],
            "re-summoning must re-scan and pick up the new file"
        );
        // Build the ACTUAL overlay the way `App::apply`'s Goto arm does, to prove
        // the fresh index really reaches the summoned picker's corpus (the same
        // `overlay::build` the live App and headless replay both call).
        let build_ctx = crate::overlay::BuildCtx {
            goto_corpus: app.file_index.clone(),
            goto_open: Vec::new(),
            goto_recent: Vec::new(),
            goto_times: Vec::new(),
            config_keys: &app.config.keys,
            outline_headings: Vec::new(),
            spell_target: None,
            history_entries: Vec::new(),
        };
        let ov = crate::overlay::build(crate::overlay::OverlayKind::Goto, &build_ctx)
            .expect("Goto always summons");
        assert!(ov.corpus.contains(&"b.txt".to_string()), "the new file is listed");
    }

    #[test]
    fn disk_changed_truth_table() {
        use crate::fs::{FileSystem, InMemoryFs};
        let mem = InMemoryFs::new();
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let p = std::path::Path::new("/d/f.md");
        // (None, None): the file never existed — our write CREATES it, no clobber.
        assert!(!App::disk_changed(p, None));
        mem.write(p, b"v1").unwrap();
        let t1 = App::disk_mtime_of(p);
        assert!(t1.is_some(), "the fake records mtimes");
        // (Some, Some) equal → unchanged.
        assert!(!App::disk_changed(p, t1));
        // (Some, None): the file APPEARED externally since we looked.
        assert!(App::disk_changed(p, None));
        // (Some, Some) differing → a real external change.
        std::thread::sleep(Duration::from_millis(2)); // ensure a distinct mtime
        mem.write(p, b"v2").unwrap();
        assert!(App::disk_changed(p, t1));
        // (None, Some): the file was DELETED externally (renamed away here — the
        // trait has no remove op, and a rename models the same disappearance).
        let last = App::disk_mtime_of(p);
        mem.rename(p, std::path::Path::new("/d/elsewhere.md")).unwrap();
        assert!(App::disk_changed(p, last));
    }

    #[test]
    fn autosave_flush_writes_doc_and_snapshots_loose_file() {
        use crate::fs::{FileSystem, InMemoryFs};
        let p = PathBuf::from("/notes/draft.md");
        let mem = InMemoryFs::new().with_file(&p, "v1\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
        assert!(
            app.autosave_last_ok.is_none(),
            "the debug panel's autosave clock is untouched before any write"
        );
        app.buffer.set_text("v2\n");
        app.autosave_flush();
        assert_eq!(mem.read_to_string(&p).unwrap(), "v2\n", "the edit hit the disk");
        assert_eq!(
            app.doc_saved_version,
            Some(app.buffer.version()),
            "the flushed version is bookkept"
        );
        assert!(app.notice.is_none(), "a clean write raises no notice");
        assert!(
            app.autosave_last_ok.is_some(),
            "a real engine write stamps the debug panel's autosave clock"
        );
        // The debug panel's pure composer agrees: enabled + not held + a stamped
        // write => Saved (never Off/Held after a clean autosave).
        assert!(matches!(
            crate::debug::autosave_state(app.config.autosave_on(), app.notice.is_some(), Some(0)),
            crate::debug::AutosaveState::Saved(Some(0))
        ));
        // Every save records: the loose file grew a history snapshot.
        assert!(
            !crate::history::list(&p).is_empty(),
            "autosave records a local-history snapshot for a loose file"
        );
        // An unchanged buffer is not re-written (version bookkeeping short-circuits).
        let t = App::disk_mtime_of(&p);
        app.autosave_flush();
        assert_eq!(App::disk_mtime_of(&p), t, "no redundant write for a clean buffer");
    }

    #[test]
    fn autosave_flush_skips_and_notices_when_disk_changed_externally() {
        use crate::fs::{FileSystem, InMemoryFs};
        let p = PathBuf::from("/notes/draft.md");
        let mem = InMemoryFs::new().with_file(&p, "disk v1\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
        // Someone ELSE writes the file behind awl's back.
        std::thread::sleep(Duration::from_millis(2)); // distinct mtime
        mem.write(&p, b"external edit\n").unwrap();
        app.buffer.set_text("mine\n");
        app.autosave_flush();
        // The CLOBBER GUARD held the write: the external edit survives on disk.
        assert_eq!(
            mem.read_to_string(&p).unwrap(),
            "external edit\n",
            "autosave never overwrites external edits"
        );
        assert_eq!(
            app.notice.as_deref(),
            Some("changed on disk outside awl — autosave held"),
            "a calm notice is raised"
        );
        assert!(
            app.autosave_last_ok.is_none(),
            "a HELD write must never stamp the debug panel's autosave clock — no write happened"
        );
        // The debug panel's pure composer agrees: held wins over "nothing written yet".
        assert_eq!(
            crate::debug::autosave_state(app.config.autosave_on(), app.notice.is_some(), None),
            crate::debug::AutosaveState::Held
        );
        // The version is marked handled so the idle timer doesn't spin; the NEXT
        // edit re-arms the engine (and the notice would recur calmly).
        assert_eq!(app.doc_saved_version, Some(app.buffer.version()));
    }

    #[test]
    fn autosave_off_disables_flush() {
        use crate::fs::{FileSystem, InMemoryFs};
        let p = PathBuf::from("/notes/draft.md");
        let mem = InMemoryFs::new().with_file(&p, "v1\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let cfg = Config {
            autosave: Some(false),
            ..Config::empty()
        };
        let mut app = app_on(Some(p.clone()), "/notes", cfg);
        app.buffer.set_text("v2\n");
        app.autosave_flush();
        assert_eq!(
            mem.read_to_string(&p).unwrap(),
            "v1\n",
            "autosave = false leaves the disk untouched"
        );
        assert!(app.notice.is_none());
        assert!(
            app.autosave_last_ok.is_none(),
            "a disabled engine never stamps the debug panel's autosave clock"
        );
        // The debug panel's pure composer agrees: disabled wins over everything.
        assert_eq!(
            crate::debug::autosave_state(app.config.autosave_on(), app.notice.is_some(), None),
            crate::debug::AutosaveState::Off
        );
    }

    #[test]
    fn load_path_flushes_the_leaving_buffer() {
        use crate::fs::{FileSystem, InMemoryFs};
        let a = PathBuf::from("/notes/a.md");
        let b = PathBuf::from("/notes/b.md");
        let mem = InMemoryFs::new().with_file(&a, "A\n").with_file(&b, "B\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(a.clone()), "/notes", Config::empty());
        app.buffer.set_text("A edited\n");
        app.load_path(b.clone());
        assert_eq!(
            mem.read_to_string(&a).unwrap(),
            "A edited\n",
            "switching files flushes the buffer being left"
        );
        assert_eq!(app.buffer.text(), "B\n", "the new file is open");
        assert_eq!(
            app.doc_saved_version,
            Some(app.buffer.version()),
            "the arriving buffer starts saved"
        );
    }

    // ── i18n WRITE-BACK-ONCE (App::new launch arg + App::load_path switch) ───

    #[test]
    fn launching_on_an_untagged_japanese_file_tags_it_once() {
        use crate::fs::{FileSystem, InMemoryFs};
        let p = PathBuf::from("/notes/nihongo.md");
        let original = "これは日本語の文章です。\n";
        let mem = InMemoryFs::new().with_file(&p, original);
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let app = app_on(Some(p.clone()), "/notes", Config::empty());
        assert_eq!(
            app.buffer.text(),
            format!("---\nlang: ja\n---\n{original}"),
            "an untagged kana-bearing doc is tagged ja on first open"
        );
        // NEVER a silent disk write: the file on disk is untouched, and the
        // buffer reads as DIRTY (past doc_saved_version) so the ordinary
        // autosave engine picks the tag up on the next idle/blur/switch/quit.
        assert_eq!(mem.read_to_string(&p).unwrap(), original, "disk is untouched");
        assert!(
            app.doc_saved_version.unwrap() < app.buffer.version(),
            "the stamped tag is a PENDING edit, not already-saved"
        );
    }

    #[test]
    fn write_back_never_touches_a_pure_latin_document() {
        use crate::fs::InMemoryFs;
        let p = PathBuf::from("/notes/english.md");
        let original = "Just some ordinary English prose.\n";
        let mem = InMemoryFs::new().with_file(&p, original);
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let app = app_on(Some(p.clone()), "/notes", Config::empty());
        assert_eq!(app.buffer.text(), original, "a pure-Latin doc is never touched");
        assert_eq!(
            app.doc_saved_version,
            Some(app.buffer.version()),
            "no edit landed -> still reads as saved"
        );
    }

    #[test]
    fn write_back_never_fires_on_a_non_markdown_file() {
        use crate::fs::InMemoryFs;
        // A `.rs` file with a Japanese string literal: frontmatter is a
        // markdown/notes convention, and stamping `---`/`lang:` text into a
        // code file would corrupt it, so this must stay untouched.
        let p = PathBuf::from("/proj/main.rs");
        let original = "fn main() {\n    println!(\"こんにちは\");\n}\n";
        let mem = InMemoryFs::new().with_file(&p, original);
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let app = app_on(Some(p.clone()), "/proj", Config::empty());
        assert_eq!(app.buffer.text(), original, "a non-markdown file is never tagged");
    }

    #[test]
    fn write_back_uses_the_configured_cjk_priority_for_ambiguous_han() {
        use crate::fs::InMemoryFs;
        let p = PathBuf::from("/notes/hanzi.md");
        let original = "汉字漢字\n"; // Han only, no kana/hangul/bopomofo -> ambiguous
        let mem = InMemoryFs::new().with_file(&p, original);
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let cfg = Config {
            cjk_priority: Some(vec![crate::frontmatter::Lang::ZhHans, crate::frontmatter::Lang::Ja]),
            ..Config::empty()
        };
        let app = app_on(Some(p.clone()), "/notes", cfg);
        assert_eq!(app.buffer.text(), format!("---\nlang: zh-Hans\n---\n{original}"));
    }

    #[test]
    fn write_back_is_undoable_with_cmd_z() {
        use crate::fs::InMemoryFs;
        let p = PathBuf::from("/notes/nihongo.md");
        let original = "こんにちは\n";
        let mem = InMemoryFs::new().with_file(&p, original);
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
        assert_ne!(app.buffer.text(), original, "the tag landed");
        app.buffer.undo();
        assert_eq!(app.buffer.text(), original, "Cmd-Z removes the stamped tag cleanly");
    }

    #[test]
    fn write_back_never_re_tags_a_document_already_carrying_frontmatter() {
        use crate::fs::InMemoryFs;
        let p = PathBuf::from("/notes/tagged.md");
        // Already tagged (as if a previous session's write-back had already
        // fired and been saved) — must never gain a SECOND block.
        let already = "---\nlang: ja\n---\nこんにちは\n";
        let mem = InMemoryFs::new().with_file(&p, already);
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let app = app_on(Some(p.clone()), "/notes", Config::empty());
        assert_eq!(app.buffer.text(), already, "an already-tagged doc is untouched");
        assert_eq!(
            app.doc_saved_version,
            Some(app.buffer.version()),
            "no edit landed -> still reads as saved"
        );
    }

    #[test]
    fn write_back_never_fires_twice_across_a_reopen() {
        use crate::fs::{FileSystem, InMemoryFs};
        let a = PathBuf::from("/notes/a.md");
        let b = PathBuf::from("/notes/nihongo.md");
        let original = "こんにちは\n";
        let mem = InMemoryFs::new().with_file(&a, "hello\n").with_file(&b, original);
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(a.clone()), "/notes", Config::empty());
        // First open of `b`: tags it (still only in-memory — disk untouched).
        app.load_path(b.clone());
        let tagged = app.buffer.text();
        assert_eq!(tagged, format!("---\nlang: ja\n---\n{original}"));
        // Simulate a save (autosave/Cmd-S would write exactly this).
        mem.write(&b, tagged.as_bytes()).unwrap();
        // Switch away, then back: `load_path`'s SWITCH branch (already open in
        // the registry) restores the live buffer untouched — no second call.
        app.load_path(a.clone());
        app.load_path(b.clone());
        assert_eq!(app.buffer.text(), tagged, "no second frontmatter block, live round trip");
        // And a FRESH session reopening the now-tagged file also never re-tags
        // (the write-back gate is `frontmatter::detect`, not a one-shot flag).
        let app2 = app_on(Some(b.clone()), "/notes", Config::empty());
        assert_eq!(app2.buffer.text(), tagged, "a fresh session sees the tag and never re-fires");
    }

    #[test]
    fn load_path_preserves_a_clobber_notice_the_leaving_flush_just_raised() {
        // REGRESSION (code review nit): if the flush `load_path` runs on the
        // buffer being LEFT hits the autosave clobber guard (the file changed
        // on disk outside awl), the notice it raises must survive the switch
        // — the unconditional `self.notice = None` a few lines later used to
        // wipe it in the very same call, before a single frame ever rendered
        // it, so the user never learned their unsaved edit was held.
        use crate::fs::{FileSystem, InMemoryFs};
        let a = PathBuf::from("/notes/a.md");
        let b = PathBuf::from("/notes/b.md");
        let mem = InMemoryFs::new().with_file(&a, "A\n").with_file(&b, "B\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(a.clone()), "/notes", Config::empty());
        app.buffer.set_text("A edited\n");
        // Someone ELSE writes A behind awl's back before we switch away from it.
        std::thread::sleep(Duration::from_millis(2)); // distinct mtime
        mem.write(&a, b"external edit\n").unwrap();

        app.load_path(b.clone());

        assert_eq!(app.buffer.text(), "B\n", "the switch to B still happens");
        assert_eq!(
            mem.read_to_string(&a).unwrap(),
            "external edit\n",
            "the clobber guard held A's write — the external edit is intact"
        );
        assert_eq!(
            app.notice.as_deref(),
            Some("changed on disk outside awl — autosave held"),
            "the notice raised while leaving A must survive into the switch, not vanish unseen"
        );
    }

    #[test]
    fn scratch_stash_and_restore_round_trip() {
        use crate::fs::{FileSystem, InMemoryFs};
        let mem = InMemoryFs::new();
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let stash = crate::fs::scratch_stash_path();
        // A no-file launch, some typing, then a flush (idle/blur/quit all route here).
        let mut app = app_on(None, "/proj", Config::empty());
        app.buffer.set_text("brain dump\n");
        app.autosave_flush();
        assert_eq!(
            mem.read_to_string(&stash).unwrap(),
            "brain dump\n",
            "the scratch stashed"
        );
        assert!(
            !crate::history::list(&stash).is_empty(),
            "the persistent scratch grows its own timeline"
        );
        // A fresh no-argument launch RESTORES it: still path-less, still the
        // markdown-first scratch surface, not a note.
        let mut app2 = app_on(None, "/proj", Config::empty());
        assert_eq!(app2.buffer.text(), "brain dump\n", "the stash restores");
        assert!(app2.buffer.path().is_none(), "restored scratch stays path-less");
        assert!(app2.buffer.is_markdown() && !app2.buffer.is_note());
        // The restore stamped the stash mtime, so a follow-up edit + flush is not
        // mistaken for a two-instance clobber.
        app2.buffer.set_text("brain dump\nmore\n");
        app2.autosave_flush();
        assert_eq!(mem.read_to_string(&stash).unwrap(), "brain dump\nmore\n");
        assert!(app2.notice.is_none(), "no false clobber notice after a restore");
    }

    #[test]
    fn blur_flush_never_reloads_buffer_or_resets_cursor() {
        // WEB STRESS-TEST HYPOTHESIS (characterized, not reproduced): a Playwright
        // run typing "AAA" then, in a LATER dispatch batch, "BBB" observed BBB
        // landing at buffer position 0 instead of after "AAA", as if a blur/
        // visibility flap between the two batches made the web build RE-LOAD the
        // scratch from its localStorage stash mid-session (which would restore
        // the STASHED content and reset the cursor to 0 — restoring a buffer
        // always starts a fresh Buffer at cursor 0, see `App::new`).
        //
        // `WindowEvent::Focused(false)` is the one live door a blur reaches —
        // and it calls exactly `App::autosave_flush` (`app.rs`'s `Focused(false)`
        // arm), which fans out to `stash_scratch_now` for a no-path scratch. That
        // function is a pure WRITE: it reads `self.buffer.text()` and writes it
        // OUT to the stash path; it never calls `crate::fs::active().read_*` or
        // reconstructs `self.buffer`. The ONLY place a stash is ever read back
        // INTO a buffer is `App::new` (a true process/page (re)launch) — never a
        // blur, never any other live-App path. This test pins that down: typing
        // "AAA", flushing (the blur trigger) as many times as a stress test's
        // spurious focus flapping might, then typing "BBB" must land the cursor
        // right after "AAA", not at 0.
        use crate::fs::InMemoryFs;
        let mem = InMemoryFs::new();
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(None, "/proj", Config::empty());
        for c in "AAA".chars() {
            app.buffer.insert_char(c);
        }
        assert_eq!(app.buffer.cursor_char(), 3, "cursor sits after the typed AAA");
        // Simulate the exact call the live `Focused(false)` arm makes — as many
        // times as a flappy test harness might re-fire it between dispatches.
        app.autosave_flush();
        app.autosave_flush();
        app.autosave_flush();
        assert_eq!(app.buffer.text(), "AAA", "a blur-driven flush never reloads content");
        assert_eq!(
            app.buffer.cursor_char(),
            3,
            "a blur-driven flush never resets the cursor — only App::new restores"
        );
        // A later "dispatch batch" continues typing from exactly where it left off.
        for c in "BBB".chars() {
            app.buffer.insert_char(c);
        }
        assert_eq!(
            app.buffer.text(),
            "AAABBB",
            "BBB lands after AAA, not at position 0"
        );
        assert_eq!(app.buffer.cursor_char(), 6);
    }

    #[test]
    fn scratch_restore_skips_empty_stash() {
        use crate::fs::{FileSystem, InMemoryFs};
        // An EMPTY stash restores nothing (plain scratch)… (each half owns its
        // FsGuard — the guard holds the process-wide FS lock, so they must not
        // overlap on one thread.)
        {
            let mem = InMemoryFs::new();
            let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
            mem.write(&crate::fs::scratch_stash_path(), b"").unwrap();
            let app = app_on(None, "/proj", Config::empty());
            assert!(app.buffer.text().is_empty(), "empty stash → plain scratch");
        }
        // …and so does a MISSING one (fresh fake).
        {
            let _g = crate::fs::FsGuard::install(Arc::new(InMemoryFs::new()));
            let app = app_on(None, "/proj", Config::empty());
            assert!(app.buffer.text().is_empty(), "missing stash → plain scratch");
        }
    }

    #[test]
    fn autosave_writes_git_files_but_never_snapshots_them() {
        // LOCKED DECISION 4, both halves at the App seam: autosave still WRITES
        // a git-managed file (writing is not version-meddling), but records NO
        // awl snapshot for it — its timeline stays git log alone.
        use crate::fs::{FileSystem, InMemoryFs};
        let p = PathBuf::from("/repo/doc.md");
        let mem = InMemoryFs::new().with_dir("/repo/.git").with_file(&p, "v1\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(p.clone()), "/repo", Config::empty());
        app.buffer.set_text("v2\n");
        app.autosave_flush();
        assert_eq!(
            mem.read_to_string(&p).unwrap(),
            "v2\n",
            "autosave still WRITES a git-managed file"
        );
        assert!(app.notice.is_none(), "a clean write raises no notice");
        // The snapshot store never grew a log dir — the record gate held.
        let store = crate::fs::data_root().join("history");
        assert!(
            mem.read_dir(&store).map(|v| v.is_empty()).unwrap_or(true),
            "no awl snapshot log for a git-managed file"
        );
    }

    #[test]
    fn scratch_stash_clobber_guard_holds_two_instance_writes() {
        // TWO-INSTANCE SAFETY: another awl (or anything) writes the stash after
        // this instance launched — the flush HOLDS (the external stash content
        // survives) and raises the same calm notice as the document guard.
        use crate::fs::{FileSystem, InMemoryFs};
        let mem = InMemoryFs::new();
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let stash = crate::fs::scratch_stash_path();
        let mut app = app_on(None, "/proj", Config::empty());
        mem.write(&stash, b"the other instance's dump\n").unwrap();
        app.buffer.set_text("mine\n");
        app.autosave_flush();
        assert_eq!(
            mem.read_to_string(&stash).unwrap(),
            "the other instance's dump\n",
            "the stash write is held — external content survives"
        );
        assert_eq!(
            app.notice.as_deref(),
            Some("changed on disk outside awl — autosave held"),
            "the calm notice names the hold"
        );
    }

    #[test]
    fn emptied_scratch_clears_the_stale_stash() {
        // The stash writes EVEN EMPTY text: emptying the restored scratch and
        // flushing must clear yesterday's dump, or a deliberately-emptied
        // scratch would resurrect on the next launch.
        use crate::fs::{FileSystem, InMemoryFs};
        let mem = InMemoryFs::new();
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let stash = crate::fs::scratch_stash_path();
        mem.write(&stash, b"yesterday's dump\n").unwrap();
        let mut app = app_on(None, "/proj", Config::empty());
        assert_eq!(app.buffer.text(), "yesterday's dump\n", "the stash restored");
        app.buffer.set_text("");
        app.autosave_flush();
        assert_eq!(
            mem.read_to_string(&stash).unwrap(),
            "",
            "an emptied scratch clears the stale stash"
        );
        assert!(app.notice.is_none(), "our own restore is not an external edit");
    }

    // ── The HISTORY TIMELINE live preview (App-level, InMemoryFs seam) ───────
    //
    // The preview is DERIVED at ViewState-build time — these tests pin the
    // resolver (`history_preview_text`) and the close contract
    // (`history_overlay_closed`) directly, buffer untouched throughout.

    /// Seed two history versions for `p` and open the History overlay on `app`,
    /// exactly as the OpenHistory gather builds it (timeline_rows → new_history).
    fn open_history_overlay(app: &mut App, p: &std::path::Path) {
        let rows = crate::history::timeline_rows(
            p,
            &app.buffer.text(),
            crate::history::now_millis(),
        );
        app.overlay = Some(crate::overlay::OverlayState::new_history(rows));
    }

    #[test]
    fn history_preview_resolves_without_touching_buffer() {
        use crate::fs::{FileSystem, InMemoryFs};
        let p = PathBuf::from("/notes/draft.md");
        let mem = InMemoryFs::new().with_file(&p, "v2\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        crate::history::record(&p, "v1\n", &Config::empty());
        crate::history::record(&p, "v2\n", &Config::empty());
        let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
        let version_before = app.buffer.version();
        open_history_overlay(&mut app, &p);
        // Row 0 (newest) previews v2; move down → row 1 previews the OLDER v1.
        assert_eq!(app.history_preview_text().as_deref(), Some("v2\n"));
        app.overlay.as_mut().unwrap().move_sel(1);
        assert_eq!(
            app.history_preview_text().as_deref(),
            Some("v1\n"),
            "arrowing the rows previews THAT version"
        );
        // The BUFFER was never touched: content, version, and undo all intact.
        assert_eq!(app.buffer.text(), "v2\n", "preview never mutates the buffer");
        assert_eq!(app.buffer.version(), version_before, "no version bump");
        // The per-id CACHE serves a repeat without re-reading the store: blow the
        // store away and the highlighted row still previews from the cache.
        let hist_dir = crate::fs::data_root().join("history");
        for entry in mem.read_dir(&hist_dir).unwrap_or_default() {
            let _ = mem.rename(&entry.path, std::path::Path::new("/gone"));
        }
        assert_eq!(
            app.history_preview_text().as_deref(),
            Some("v1\n"),
            "a repeat on the same id is a cache hit"
        );
    }

    #[test]
    fn preview_cache_invalidates_on_selection_move() {
        use crate::fs::InMemoryFs;
        let p = PathBuf::from("/notes/draft.md");
        let mem = InMemoryFs::new().with_file(&p, "v2\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        crate::history::record(&p, "v1\n", &Config::empty());
        crate::history::record(&p, "v2\n", &Config::empty());
        let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
        open_history_overlay(&mut app, &p);
        assert_eq!(app.history_preview_text().as_deref(), Some("v2\n"));
        let cached_id = app.history_preview.as_ref().map(|(id, _)| id.clone());
        // Moving the selection to another row (a different id) reloads: the cache
        // is keyed by id, never by "an overlay is open".
        app.overlay.as_mut().unwrap().move_sel(1);
        assert_eq!(app.history_preview_text().as_deref(), Some("v1\n"));
        assert_ne!(
            app.history_preview.as_ref().map(|(id, _)| id.clone()),
            cached_id,
            "the cache now holds the newly highlighted id"
        );
    }

    #[test]
    fn history_close_without_accept_restores_scroll_and_drops_preview() {
        use crate::fs::InMemoryFs;
        let _g = crate::fs::FsGuard::install(Arc::new(InMemoryFs::new()));
        let mut app = app_on(None, "/proj", Config::empty());
        // A shorter previewed version clamped the scroll while the picker was
        // open; the close-without-accept restores the saved scroll EXACTLY
        // ("Esc = back to now") and puts the preview down.
        app.history_scroll_before = Some(42);
        app.scroll_lines = 3;
        app.history_preview = Some(("100".into(), "old\n".into()));
        app.history_overlay_closed(false);
        assert_eq!(app.scroll_lines, 42, "Esc restores the pre-open scroll");
        assert!(app.history_scroll_before.is_none());
        assert!(app.history_preview.is_none(), "the preview is dropped");
        // A real ACCEPT keeps the current viewport (the restored version owns
        // it) — the saved scroll is discarded, the preview still dropped.
        app.history_scroll_before = Some(42);
        app.scroll_lines = 3;
        app.history_preview = Some(("100".into(), "old\n".into()));
        app.history_overlay_closed(true);
        assert_eq!(app.scroll_lines, 3, "an accept never yanks the viewport");
        assert!(app.history_scroll_before.is_none());
        assert!(app.history_preview.is_none());
    }

    #[test]
    fn scratch_buffer_lists_its_stash_history() {
        use crate::fs::InMemoryFs;
        let _g = crate::fs::FsGuard::install(Arc::new(InMemoryFs::new()));
        // The persistent scratch stashes (autosave engine) — recording history
        // under its stash path — and the timeline gather's shared source_path
        // fallback finds it, so the no-path scratch has a summonable timeline.
        let mut app = app_on(None, "/proj", Config::empty());
        app.buffer.set_text("scratch thoughts\n");
        app.autosave_flush();
        let key = crate::history::source_path(
            app.buffer.path(),
            app.file.as_deref(),
            app.buffer.is_note(),
        )
        .expect("the true scratch keys under its stash");
        assert_eq!(key, crate::fs::scratch_stash_path());
        let rows = crate::history::timeline_rows(
            &key,
            &app.buffer.text(),
            crate::history::now_millis(),
        );
        assert!(!rows.is_empty(), "the scratch stash has a timeline");
        // And the preview resolver rides the same key: the newest row previews
        // the stashed content.
        app.overlay = Some(crate::overlay::OverlayState::new_history(rows));
        assert_eq!(
            app.history_preview_text().as_deref(),
            Some("scratch thoughts\n")
        );
    }

    #[test]
    fn notes_keep_their_own_autosave() {
        use crate::fs::{FileSystem, InMemoryFs};
        let mem = InMemoryFs::new();
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(None, "/proj", Config::empty());
        app.buffer.start_note(PathBuf::from("/mynotes"));
        app.buffer.set_text("a note in flight\n");
        app.autosave_flush();
        // The DOC engine leaves notes to their own 400ms flow (flush_note): no
        // scratch stash, no note file written by this door.
        assert!(
            mem.read(&crate::fs::scratch_stash_path()).is_err(),
            "a note is never stashed as scratch"
        );
        assert!(
            mem.read_dir(std::path::Path::new("/mynotes"))
                .map(|v| v.is_empty())
                .unwrap_or(true),
            "autosave_flush does not write note files"
        );
    }

    // ── MULTI-BUFFER REGISTRY (App-level: open/switch preserves everything) ──

    #[test]
    fn load_path_switches_to_already_open_buffer_preserving_edits_and_cursor() {
        // THE v1 OBSERVABLE WIN: re-opening a file already open in this session
        // restores its LIVE buffer (unsaved edits, cursor) instead of re-reading
        // disk. Proven by mutating A's on-disk bytes BEHIND awl's back while B is
        // active, then asserting the restored A shows the in-memory edit, not the
        // disk write.
        use crate::fs::{FileSystem, InMemoryFs};
        let a = PathBuf::from("/proj/a.txt");
        let b = PathBuf::from("/proj/b.txt");
        let mem = InMemoryFs::new().with_file(&a, "alpha\n").with_file(&b, "beta\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(a.clone()), "/proj", Config::empty());
        app.buffer.set_text("ALPHA EDITED\n");
        app.buffer.set_cursor(3);
        assert_eq!(app.open_buffer_count(), 1, "only A is open so far");

        app.load_path(b.clone());
        assert_eq!(app.buffer.text(), "beta\n", "B loads fresh from disk (first open)");
        assert_eq!(app.open_buffer_count(), 2, "A is now backgrounded, not closed");
        app.buffer.set_text("BETA EDITED\n");

        mem.write(&a, b"ALPHA CHANGED ON DISK\n").unwrap();

        app.load_path(a.clone());
        assert_eq!(
            app.buffer.text(),
            "ALPHA EDITED\n",
            "the LIVE unsaved edit survived the round trip, not a re-read from disk"
        );
        assert_eq!(app.buffer.cursor_char(), 3, "the cursor position survived too");
        assert!(app.buffer.is_dirty(), "the unsaved edit is still unsaved");
        assert_eq!(app.open_buffer_count(), 2, "A active again, B backgrounded");

        // And B's OWN edit is preserved too (not silently dropped when we left it).
        app.load_path(b.clone());
        assert_eq!(app.buffer.text(), "BETA EDITED\n", "B's edit also survived");
    }

    #[test]
    fn load_path_reopening_the_active_file_is_a_noop() {
        // Re-"opening" the file that is already active must not disturb anything
        // (no park/restore round trip, no fresh disk read either).
        use crate::fs::InMemoryFs;
        let a = PathBuf::from("/proj/a.txt");
        let mem = InMemoryFs::new().with_file(&a, "alpha\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(a.clone()), "/proj", Config::empty());
        app.buffer.set_text("EDITED IN PLACE\n");
        app.buffer.set_cursor(2);
        app.load_path(a.clone());
        assert_eq!(app.buffer.text(), "EDITED IN PLACE\n");
        assert_eq!(app.buffer.cursor_char(), 2);
        assert_eq!(app.open_buffer_count(), 1, "no phantom second entry");
    }

    #[test]
    fn load_path_recognizes_the_same_file_under_a_differently_spelled_path() {
        // REGRESSION (code review): the registry's identity must be blind to
        // which textual spelling of the same file produced the path — e.g. a
        // CLI file argument typed with no directory component (`cd project &&
        // awl a.txt`, staying relative) vs. that same file's later ROOT-JOINED
        // spelling (`index::resolve`, always absolute — every Goto candidate).
        // Reproduced here with a `.` path component (lexically different, same
        // file) so the fix is proven at the live-App layer, not just headless.
        use crate::fs::InMemoryFs;
        let messy = PathBuf::from("/proj/./a.txt");
        let clean = PathBuf::from("/proj/a.txt");
        let b = PathBuf::from("/proj/b.txt");
        let mem = InMemoryFs::new().with_file(&b, "beta\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(messy.clone()), "/proj", Config::empty());
        app.buffer.set_text("ALPHA EDITED\n");
        assert_eq!(app.open_buffer_count(), 1);

        app.load_path(b.clone());
        assert_eq!(app.open_buffer_count(), 2, "the messy-spelled A is backgrounded");

        app.load_path(clean.clone());
        assert_eq!(
            app.buffer.text(),
            "ALPHA EDITED\n",
            "the CLEAN path found A's live entry (parked under the MESSY spelling) instead of \
             opening a fresh, orphaned copy"
        );
        assert_eq!(
            app.open_buffer_count(),
            2,
            "no orphaned duplicate entry left behind for the messy spelling"
        );
    }

    #[test]
    fn load_path_opens_a_relative_launch_path_then_finds_it_again_via_absolute_path() {
        // REGRESSION (code review, scenario a — the report's EXACT live shape):
        // `cd project && awl a.txt` leaves the launch file argument RELATIVE;
        // reopening the SAME file via its absolute spelling (what a Go-to-file
        // picker candidate always is — `index::resolve` root-joins) must find
        // the SAME live buffer, not silently re-read disk and orphan the
        // relative spelling's dirty entry forever. Needs a REAL chdir (not
        // InMemoryFs, which has no cwd concept) against a real temp dir — hold
        // both the fs TEST_LOCK (real-disk reads race a sibling's InMemoryFs
        // swap) and the CWD_LOCK (chdir is process-global too).
        let _fs = crate::fs::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("awl-relabs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "alpha\n").unwrap();
        let _cwd = crate::fs::CwdGuard::enter(&dir);

        // This test's `App::new` runs against the REAL native FS (it can't use
        // InMemoryFs, per the chdir note above) — so it must explicitly kill
        // SESSION RESTORE, or `apply_session_restore` reads the developer's
        // ACTUAL `~/.local/share/awl/session.toml` and parks every real
        // buffer it names into the registry, inflating `open_buffer_count()`
        // by however many files happen to be open in a live awl session on
        // this machine right now (an environment-coupled failure, not a
        // random flake — see the investigation note in git history).
        let cfg = Config { session_restore: Some(false), ..Config::empty() };
        // The launch argument stays exactly as typed: relative, no directory.
        let mut app = App::new(Some(PathBuf::from("a.txt")), dir.clone(), None, None, cfg);
        app.buffer.set_text("ALPHA EDITED\n");
        app.buffer.set_cursor(3);
        assert_eq!(app.open_buffer_count(), 1, "only the relative-spelled A is open so far");

        // Reopen via the ABSOLUTE spelling.
        app.load_path(dir.join("a.txt"));
        assert_eq!(
            app.buffer.text(),
            "ALPHA EDITED\n",
            "the live edit survived — the absolute spelling found the SAME buffer, not a fresh \
             disk read"
        );
        assert_eq!(app.buffer.cursor_char(), 3, "the cursor position survived too");
        assert_eq!(
            app.open_buffer_count(),
            1,
            "one entry, not two — the relative and absolute spellings key identically"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn switching_buffers_isolates_the_view_text_cache() {
        // THE CACHE-KEY-DISCIPLINE bug class (CLAUDE.md): every swapped-in buffer
        // restarts its edit version at 0, so `view_text`'s version-keyed
        // rope-clone cache MUST travel with its own buffer (not collide with
        // another buffer sitting at the same version) across a three-way swap.
        use crate::fs::InMemoryFs;
        let a = PathBuf::from("/proj/a.txt");
        let b = PathBuf::from("/proj/b.txt");
        let mem = InMemoryFs::new().with_file(&a, "aaa\n").with_file(&b, "bbb\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(a.clone()), "/proj", Config::empty());
        assert_eq!(app.view_text(), "aaa\n");
        app.load_path(b.clone());
        assert_eq!(app.view_text(), "bbb\n", "B's text must not collide with A's stale version-0 cache");
        app.load_path(a.clone());
        assert_eq!(app.view_text(), "aaa\n", "A's OWN cache is restored, not B's");
    }

    #[test]
    fn switching_away_from_a_dirty_file_still_autosaves() {
        // Item 4 of the spec: the existing autosave flush-on-FILE-SWITCH hook
        // (`App::autosave_flush`, the one door) must still fire on a registry
        // switch, exactly as it did on the old single-buffer swap.
        use crate::fs::{FileSystem, InMemoryFs};
        let a = PathBuf::from("/proj/a.txt");
        let b = PathBuf::from("/proj/b.txt");
        let mem = InMemoryFs::new().with_file(&a, "aaa\n").with_file(&b, "bbb\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(a.clone()), "/proj", Config::empty());
        app.buffer.set_text("aaa EDITED\n");
        app.load_path(b.clone());
        assert_eq!(
            mem.read_to_string(&a).unwrap(),
            "aaa EDITED\n",
            "leaving a dirty pathed buffer autosaves it on switch"
        );
    }

    #[test]
    fn new_note_parks_the_previous_buffer_for_a_later_reopen() {
        use crate::fs::InMemoryFs;
        let a = PathBuf::from("/proj/a.txt");
        let mem = InMemoryFs::new().with_file(&a, "aaa\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(a.clone()), "/proj", Config::empty());
        app.buffer.set_text("aaa EDITED\n");
        assert_eq!(app.open_buffer_count(), 1);
        app.new_note();
        assert_eq!(app.open_buffer_count(), 2, "A is parked; the fresh note is active");
        assert_eq!(app.buffer.text(), "", "the new note starts blank");
        app.load_path(a.clone());
        assert_eq!(app.buffer.text(), "aaa EDITED\n", "A's edit survived being backgrounded by C-x n");
    }

    #[test]
    fn registry_cap_evicts_the_lru_clean_buffer_not_a_dirty_one() {
        // Integration proof that `App` is really wired to
        // `crate::buffers::MAX_OPEN_BUFFERS` (the algorithm itself is exhaustively
        // unit-tested in `buffers.rs`): opening one more CLEAN file than the cap
        // allows evicts the oldest clean background entry, so re-opening THAT one
        // reads fresh from disk (its edits, if any, would be gone — here it has
        // none, so we assert the fresh disk content lands, and via the "clean" law
        // that a DIRTY one earlier in the queue is never touched).
        use crate::fs::InMemoryFs;
        let mut mem = InMemoryFs::new();
        for i in 0..crate::buffers::MAX_OPEN_BUFFERS {
            mem = mem.with_file(format!("/proj/f{i}.txt"), "clean\n");
        }
        mem = mem.with_file("/proj/dirty.txt", "will-be-edited\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(PathBuf::from("/proj/dirty.txt")), "/proj", Config::empty());
        app.buffer.set_text("EDITED, NEVER EVICT ME\n");
        // Open every clean file in turn (backgrounding the dirty one first, then
        // each clean one), pushing the registry to (and one past) the cap.
        for i in 0..crate::buffers::MAX_OPEN_BUFFERS {
            app.load_path(PathBuf::from(format!("/proj/f{i}.txt")));
        }
        // The registry now holds MAX_OPEN_BUFFERS backgrounded entries (dirty.txt
        // + f0..f(N-2)) capped by evicting the LRU CLEAN one (f0) — never dirty.txt.
        assert_eq!(app.open_buffer_count(), crate::buffers::MAX_OPEN_BUFFERS, "capped, not unbounded");
        app.load_path(PathBuf::from("/proj/dirty.txt"));
        assert_eq!(
            app.buffer.text(),
            "EDITED, NEVER EVICT ME\n",
            "the dirty buffer survived the whole cap-pressure run"
        );
    }

    #[test]
    fn right_click_word_summons_spell_suggestions() {
        // The right-click path = place the cursor at the clicked word (the GPU
        // hit-test, untestable headlessly), then run the EXISTING OpenSpellSuggest
        // seam at that cursor. This locks the REUSED contract WITHOUT a window: a
        // cursor on a misspelling yields a target with corrections (so the picker
        // summons + builds a Spell overlay), while a correct word yields None — the
        // calm no-op the binding promises. Skipped if the bundled dictionary is absent.
        let Ok(sc) = crate::spell::SpellChecker::new(crate::spell::DictVariant::EnUs) else {
            return;
        };
        let mut buffer = Buffer::from_str("Please recieve this.\n");
        // Simulate the click landing inside the misspelling "recieve".
        let idx = buffer.line_col_to_char(0, 9);
        buffer.set_cursor(idx);
        let (line, col) = buffer.cursor_line_col();
        let t = sc
            .suggest_at(&buffer.text(), line, col)
            .expect("a misspelled word under the right-click yields a target");
        assert!(t.suggestions.iter().any(|w| w == "receive"));
        // What `apply(OpenSpellSuggest)` builds from that target: a Spell picker.
        let ov = crate::overlay::OverlayState::new_spell(
            t.suggestions.clone(),
            (t.misspelling.line, t.misspelling.start_col, t.misspelling.end_col),
        );
        assert_eq!(ov.kind, crate::overlay::OverlayKind::Spell);
        // A right-click on a CORRECTLY-spelled word ("Please") is a calm no-op.
        let ok_idx = buffer.line_col_to_char(0, 2);
        buffer.set_cursor(ok_idx);
        let (l, c) = buffer.cursor_line_col();
        assert!(sc.suggest_at(&buffer.text(), l, c).is_none(), "correct word: no summon");
    }

    // ── HERMETICITY STRUCTURAL GUARD ────────────────────────────────────────
    //
    // Rust's privacy model can express "visible to production plus every
    // descendant module" (what a private `fn new` already gets — every test
    // submodule under `app/` is a descendant of `app`, so it already sees the
    // raw constructor) but NOT "visible to production plus this ONE helper
    // function's own body" — there is no `pub(in path)` spelling that grants
    // access to `new_hermetic`'s definition while denying every sibling test
    // module. So the raw constructor's door can't be sealed at compile time
    // without also blocking the small set of tests that deliberately need
    // the REAL disk (see `App::new_hermetic`'s own doc for that list). This
    // is the honest fallback: a SOURCE-SCAN law test, in the same spirit as
    // `rowlayout.rs`'s / `theme.rs`'s no-wildcard enumerations — a structural
    // fact asserted at test time, cheap to keep honest because the count it
    // guards is small and curated, not a general-purpose linter.
    //
    // NOTE ON THE NEEDLE: the pattern this scan looks for is built at RUNTIME
    // (`app_new_needle`, four separate literals concatenated) rather than
    // spelled out as one contiguous string anywhere in this file — otherwise
    // this very guard's own source text would match itself and inflate its
    // own count. Keep every comment/message below phrased without writing
    // the raw constructor's name directly followed by an open paren.
    //
    // Exact per-file occurrence counts of the needle across the whole crate.
    // Every entry below is individually accounted for (see each call site's
    // own inline comment): either the ONE real production call, a real-disk
    // test that explicitly disables `session_restore` (can't use
    // `new_hermetic` because it needs `Buffer::from_file` to see genuine
    // bytes), or a test already wrapped in `fs::with_fs`/`FsGuard::install`
    // with a controlled fake `InMemoryFs` (hermetic by construction,
    // independent of `session_restore`'s value — `app/session.rs`'s own
    // tests, which specifically exercise session restore, cannot use
    // `new_hermetic` at all since it forces `session_restore: Some(false)`).
    // A test that only needs a plain, don't-care-about-disk `App` must go
    // through `App::new_hermetic` instead, which never contributes to this
    // count at all (its name has an extra `_hermetic` between `new` and the
    // open paren, so it never matches the needle).
    //
    // Adding a NEW raw call anywhere — including a new file — fails this
    // test until the count below is consciously updated, which forces the
    // same two-way choice every existing site already made.
    #[test]
    fn real_fs_app_new_calls_are_all_accounted_for() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        scan_dir_for_app_new(&root, &root, &mut counts);

        let expected: &[(&str, usize)] = &[
            // 1 production call (in `crate::app::run`) + 2 real-disk tests
            // with `session_restore` disabled inline + 1 real-disk chdir
            // test (same treatment) + the `app_on` helper (every one of its
            // 31 callers installs its own fake FS first — this file's
            // `app_on`-callers-all-install-a-fake-fs check, in the section
            // above, verifies that structurally).
            ("app.rs", 5),
            // 1 real-disk test (`finish_buffer_saves_...`), session_restore
            // disabled inline.
            ("app/daemon.rs", 1),
            // 5 calls, every one inside a `crate::fs::with_fs(fake, || ..)`
            // closure seeded with its own `InMemoryFs` — these tests exist
            // specifically to prove what `apply_session_restore` reads back,
            // so they can't use a constructor that forces it off.
            ("app/session.rs", 5),
            // input.rs's click tests all moved onto `App::new_hermetic` —
            // zero raw calls left.
        ];
        let mut expected_map: std::collections::BTreeMap<String, usize> =
            expected.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        // Any file not listed above must have ZERO occurrences.
        for (file, count) in &counts {
            let want = expected_map.remove(file).unwrap_or(0);
            assert_eq!(
                *count, want,
                "unexpected raw-constructor count in {file}: found {count}, expected {want} — \
                 either route the new call through App::new_hermetic, or (if it genuinely needs \
                 real disk) disable session_restore inline / wrap it in fs::with_fs and update \
                 this test's expected count with a comment explaining why"
            );
        }
        for (file, want) in expected_map {
            assert_eq!(0, want, "expected {want} raw-constructor call(s) in {file} but found none — did it move to new_hermetic or a different file?");
        }

        // The ONE production call site must still exist exactly once, naming
        // its real argument list (guards against the count staying right by
        // coincidence while the actual production call moved or was deleted).
        let mut production_hits = 0usize;
        count_substr_in_dir(&root, &production_call_needle(), &mut production_hits);
        assert_eq!(production_hits, 1, "the production App::new call in crate::app::run must exist exactly once");
    }

    /// Built from separate literals at runtime — see the module-doc note
    /// above the guard test for why this can't be one contiguous literal.
    #[cfg(test)]
    fn app_new_needle() -> String {
        ["App", "::", "new", "("].concat()
    }

    #[cfg(test)]
    fn production_call_needle() -> String {
        format!("{}file, root, cli_workspace, cli_notes_root, config);", app_new_needle())
    }

    #[cfg(test)]
    fn scan_dir_for_app_new(
        base: &std::path::Path,
        dir: &std::path::Path,
        counts: &mut std::collections::BTreeMap<String, usize>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let needle = app_new_needle();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan_dir_for_app_new(base, &path, counts);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let n = text.matches(&needle).count();
            if n == 0 {
                continue;
            }
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            counts.insert(rel, n);
        }
    }

    #[cfg(test)]
    fn count_substr_in_dir(dir: &std::path::Path, needle: &str, total: &mut usize) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                count_substr_in_dir(&path, needle, total);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            *total += text.matches(needle).count();
        }
    }
}
