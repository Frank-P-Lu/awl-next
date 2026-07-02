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

/// Quiet period after the last zoom step before the STICKY ZOOM is persisted to
/// config (debounce). Cmd-=/Cmd-- fire one step per press, so a write-per-step would
/// hammer the disk; instead `about_to_wait` writes the SETTLED zoom once you pause.
const ZOOM_PERSIST_DEBOUNCE: Duration = Duration::from_millis(500);

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

/// Which PHASE 2 edit FLINCH the next `sync_view` fires on the visual caret: a typed
/// char squash-pops + back-kicks, a delete squashes inward, a kill-line gulps. Armed
/// from the matching [`actions::Effect`] (`TypeImpact` / `DeleteSquash` / `Gulp`).
#[derive(Clone, Copy)]
enum CaretImpact {
    Type,
    Delete,
    Gulp,
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
    /// Whether the OS cursor is currently the horizontal-resize glyph (pointer hovering
    /// a page-column edge). Tracked so the icon is set only on a CHANGE, not every move.
    resize_cursor_on: bool,
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
    /// PHASE 2 edit FLINCH requested by `apply` for the ONE next `sync_view`: a
    /// SUCCESSFUL typed char ([`CaretImpact::Type`]) squash-pops + back-kicks, a delete
    /// ([`CaretImpact::Delete`]) squashes the caret inward, a kill-line
    /// ([`CaretImpact::Gulp`]) pulses a bigger gulp. Consumed by the next `sync_view`
    /// AFTER it sets the spring target, so the flinch rides on top and the spring
    /// self-settles it back to rest. Fires in EVERY caret look (all juice on the
    /// caret). `None` = no edit flinch this sync.
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
    /// When the OPT-IN periodic autosnapshot (`autosnapshot_secs`) last fired, so
    /// the next one waits out the configured interval. `None` until the first fire.
    /// Inert while the interval is 0 (the default) — see `maybe_periodic_snapshot`.
    last_autosnapshot: Option<crate::clock::Instant>,
    /// When the zoom last changed and a STICKY-ZOOM write is pending; the debounced
    /// write fires after `ZOOM_PERSIST_DEBOUNCE` of quiet in `about_to_wait`, so a
    /// rapid Cmd-=/Cmd-- run persists the SETTLED value once instead of per step.
    /// `None` = nothing pending (live only — headless never schedules this).
    zoom_persist_at: Option<Instant>,
    /// The loaded persistent config (keybinding overrides + folder defaults + the
    /// Settings-open path). Re-loaded when the config file is SAVED in the editor,
    /// which live-reapplies the keymap + folders.
    config: Config,
    /// The RAW `--notes-root` flag (None = unset), remembered so a live config reload
    /// re-folds precedence (flag > config > default) without the flag ever losing.
    cli_notes_root: Option<PathBuf>,
    /// The RAW `--workspace` flag (None = unset), remembered for the same reason.
    cli_workspace: Option<PathBuf>,
}

impl App {
    fn new(
        file: Option<PathBuf>,
        root: PathBuf,
        cli_workspace: Option<PathBuf>,
        cli_notes_root: Option<PathBuf>,
        config: Config,
    ) -> Self {
        let buffer = match &file {
            Some(p) => Buffer::from_file(p),
            None => Buffer::scratch(),
        };
        let initial_version = buffer.version();
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
        Self {
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
            hud_key: None,
            hud_mods: ModifiersState::empty(),
            zoom,
            dpi: 1.0,
            cursor_px: (0.0, 0.0),
            dragging: false,
            page_resizing: false,
            resize_cursor_on: false,
            drag_granularity: DragGranularity::Char,
            last_click_time: None,
            last_click_px: (0.0, 0.0),
            click_count: 0,
            scroll_px_accum: 0.0,
            shift_selecting: false,
            preedit: String::new(),
            ime_enabled: false,
            search: None,
            spell: match crate::spell::SpellChecker::new() {
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
            last_autosnapshot: None,
            zoom_persist_at: None,
            config,
            cli_notes_root,
            cli_workspace,
        }
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

impl ApplicationHandler for App {
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
        let attrs = Window::default_attributes()
            .with_inner_size(LogicalSize::new(1200.0, 800.0))
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
                // behind another app.
                self.flush_note();
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
                // A summoned picker OWNS the pointer (it is modal, the doc receding
                // behind it): a hover moves + previews the row under the cursor, exactly
                // like an arrow move. A live PAGE-WIDTH resize drag owns the pointer next
                // (the grabbed column edge tracks it, re-wrapping live); otherwise a live
                // text selection extends; otherwise (idle hover) flip the OS cursor to
                // the resize glyph when over a column edge.
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
                } else {
                    self.update_resize_cursor();
                }
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
                        } else if !self.begin_page_resize_if_hovering() {
                            self.on_press();
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
    /// save so nothing typed right before quit is lost. The final safety net of the
    /// robust-autosave guarantee.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.flush_note();
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
        // OPT-IN periodic autosnapshot (`autosnapshot_secs`): a finer local-history
        // cadence between saves. DEFAULT OFF (interval 0) → this is INERT (an early
        // return; no snapshot, no redraw, no control-flow change), so it is fully
        // behaviour-preserving unless the user opts in.
        self.maybe_periodic_snapshot();
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
/// (and optional `workspace` parent for switch-project).
pub fn run(
    file: Option<PathBuf>,
    root: PathBuf,
    cli_workspace: Option<PathBuf>,
    cli_notes_root: Option<PathBuf>,
    config: Config,
) -> anyhow::Result<()> {
    let event_loop = EventLoop::new()?;
    let app = App::new(file, root, cli_workspace, cli_notes_root, config);

    // NATIVE: `run_app` blocks this thread driving the OS event loop to exit.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut app = app;
        event_loop.run_app(&mut app)?;
    }

    // WASM: the browser event loop is the page's own; winit can't BLOCK on it, so
    // `spawn_app` hands the App to requestAnimationFrame and returns immediately
    // (control goes back to JS). The app then lives for the page's lifetime.
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
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
        let dir = std::env::temp_dir().join(format!("awl-open-swap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("old.txt");
        let new = dir.join("new.txt");
        std::fs::write(&old, "the OLD document\n").unwrap();
        std::fs::write(&new, "the NEW document\n").unwrap();
        let mut app = App::new(Some(old), dir.clone(), None, None, Config::empty());
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
        let dir = std::env::temp_dir().join(format!("awl-note-swap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("doc.txt");
        std::fs::write(&file, "prior document\n").unwrap();
        let notes = dir.join("notes");
        let mut app = App::new(Some(file), dir.clone(), None, Some(notes), Config::empty());
        assert_eq!(app.view_text(), "prior document\n");
        app.new_note();
        assert_eq!(app.view_text(), "", "the fresh note starts blank on screen");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn right_click_word_summons_spell_suggestions() {
        // The right-click path = place the cursor at the clicked word (the GPU
        // hit-test, untestable headlessly), then run the EXISTING OpenSpellSuggest
        // seam at that cursor. This locks the REUSED contract WITHOUT a window: a
        // cursor on a misspelling yields a target with corrections (so the picker
        // summons + builds a Spell overlay), while a correct word yields None — the
        // calm no-op the binding promises. Skipped if the bundled dictionary is absent.
        let Ok(sc) = crate::spell::SpellChecker::new() else {
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
}
