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
// a best-effort ASYNC mirror onto `navigator.clipboard` (the WEB ESCAPE
// HATCHES round — arboard itself still won't compile for wasm32, and the
// browser clipboard is async + permission-gated, so it can't fit arboard's
// SYNC surface directly; `web_clipboard::Clipboard` below adapts it). COPY
// mirrors out fire-and-forget; PASTE stays internal-only (see that module's
// doc for why). `App::new` always gets `Some(Clipboard)` on both platforms
// now (native: a real system clipboard, or `None` only if `Clipboard::new()`
// itself errs — e.g. no display server; web: always `Some`, `set_text`/
// `get_text` degrade individually instead).
#[cfg(not(target_arch = "wasm32"))]
use arboard::Clipboard;
#[cfg(target_arch = "wasm32")]
use web_clipboard::Clipboard;

#[cfg(target_arch = "wasm32")]
mod web_clipboard {
    //! Best-effort async bridge onto the browser Clipboard API
    //! (`navigator.clipboard`), landed in the WEB ESCAPE HATCHES round. Mirrors
    //! the slice of arboard's sync API `app.rs` uses (`new`/`set_text`/
    //! `get_text`) so `App`'s clipboard-mirror call sites (`sync_kill_to_
    //! clipboard` / `refresh_kill_from_clipboard`) need no platform branching.
    //!
    //! **COPY (`set_text`) mirrors out for real.** `writeText` is fire-and-
    //! forget via `wasm_bindgen_futures::spawn_local` — NEVER blocks the
    //! editor (matches `App::follow_link`'s / `web_export::trigger_download`'s
    //! own never-await-inline discipline) — and any rejection (permission
    //! denied, insecure context / non-HTTPS, no focus) is swallowed exactly
    //! like a failed native arboard write: a calm degrade back to the
    //! internal-kill-ring-only behavior this stub always had, never a panic,
    //! never a user-visible error.
    //!
    //! **PASTE (`get_text`) is DELIBERATELY NOT wired to `readText`** — a
    //! logged, honest asymmetry (see `WEB.md`), not an oversight. `readText`
    //! needs "transient activation" (a currently-live, un-consumed user
    //! gesture) in Chromium, and awl's key dispatch reaches this call several
    //! async hops downstream of the real DOM `keydown` (winit's own event
    //! queue, then `App::apply`) — by the time it would run, that gesture is
    //! very likely already stale. The realistic outcomes are a silent
    //! `NotAllowedError` on every call (Chromium) or a NEW permission prompt
    //! on every single paste (Firefox, which does not consult the Permissions
    //! API for `clipboard-read` the way Chromium does) — exactly the "prompt
    //! storm" this round's own spec says to avoid rather than ship. So
    //! `get_text` always `Err`s here: Yank stays on the internal kill-ring
    //! only, byte-identical to before this round.
    pub struct Clipboard;

    impl Clipboard {
        pub fn new() -> Result<Self, &'static str> {
            Ok(Self)
        }

        pub fn set_text(&mut self, text: String) -> Result<(), &'static str> {
            let Some(window) = web_sys::window() else {
                return Err("no window (headless/detached wasm context)");
            };
            let clipboard = window.navigator().clipboard();
            let promise = clipboard.write_text(&text);
            wasm_bindgen_futures::spawn_local(async move {
                // Fire-and-forget: a rejected promise (permission denied,
                // insecure context, lost focus) is swallowed here — the
                // internal kill-ring already holds the value, so nothing
                // user-visible is lost even if this silently fails.
                let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
            });
            // Optimistic Ok: the write itself is async/best-effort, but the
            // caller (`App::sync_kill_to_clipboard`) only uses this return to
            // update its own dedup cache (`clipboard_last_written`) — marking
            // it written now is harmless even on a silent async failure,
            // since the internal kill-ring stays the source of truth either way.
            Ok(())
        }

        pub fn get_text(&mut self) -> Result<String, &'static str> {
            // See the module doc: readText is deliberately not wired (the
            // lost-transient-activation / "prompt storm" risk this round's
            // spec explicitly calls out as a reason to ship copy-out only).
            Err("clipboard read unavailable on web (see WEB.md)")
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

/// How long a completed file EVENT stays in the calm bottom-center readout.
/// Toasts are armed only by the live App (once a GPU/window exists), and expire
/// through one `WaitUntil`; captures never own this clock.
const TOAST_LIFETIME: Duration = Duration::from_millis(2500);
const CLOBBER_NOTICE: &str =
    "changed on disk outside awl — ⌘S keeps yours · reopen for theirs";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum NoticeKind {
    Toast,
    #[default]
    Sticky,
}

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

/// AMBIENT LAVA TICK period — the lava-lamp ground's slow drift cadence
/// (`crate::lava::LAVA_TICK_MS`). A single `WaitUntil` this far out in
/// `about_to_wait` advances the phase + requests one redraw + re-arms, so a lava
/// world costs ~10 sparse frames/sec (NEVER the caret spring's hot per-frame
/// loop), and a non-lava world costs zero (the tick never arms). See
/// `App::tick_lava`.
const LAVA_TICK: Duration = Duration::from_millis(crate::lava::LAVA_TICK_MS);

/// Quiet period after the last LIVE-RESIZE `Resized` tick before the macOS
/// Core-Animation-transaction present sync (`Gpu::set_presents_with_
/// transaction`) is flipped back OFF (debounce; macOS-only — see
/// `resize_settle_at`'s doc for the full mechanism). A fast drag re-stamps the
/// deadline on every tick (`App::arm_live_resize_sync`), so this only fires
/// once the drag genuinely stops. TASTE TUNABLE, mirrors `THEME_FONT_
/// DEBOUNCE`'s value: short enough that the transaction-sync cost (Apple's
/// own documented throughput trade-off for `presentsWithTransaction`) is paid
/// only while actually dragging, long enough that a brief pause mid-drag
/// doesn't flap it on/off.
const RESIZE_SYNC_SETTLE: Duration = Duration::from_millis(150);
const MOVE_SETTLE: Duration = Duration::from_millis(150);

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
/// Physical-px SLOP a text-selection drag must travel past the press position
/// before it arms (extends the selection) — the phantom-selection-click fix. Below
/// this, a `CursorMoved` while `dragging` is pointer jitter (or a WYSIWYG reveal
/// reflow under a stationary pointer) and must not move the cursor away from the
/// press's own hit-test result. Matches the multi-click "same spot" tolerance
/// (`bump_click_count`'s own `4.0`) — both answer "did the pointer really move",
/// just for two different gestures. See `App::exceeds_drag_slop` (`app/input/mouse.rs`).
const DRAG_ARM_SLOP_PX: f32 = 4.0;

/// What kind of unit the current drag is selecting by (set on press).
#[derive(Clone, Copy, PartialEq)]
enum DragGranularity {
    Char,
    Word,
    Line,
}

/// Which edit FLINCH the next `sync_view` fires on the visual caret: a typed char
/// squash-pops + back-kicks (PHASE 2), a delete squashes inward (PHASE 2), a
/// kill-line gulps (PHASE 2), Enter lands a caret-level touchdown squash (PHASE 3),
/// a successful copy pulses gently (the COPY PULSE round — also brightens the
/// selection quad's tint, so `Copy`'s pipeline call does a touch more than the
/// other arms; see `TextPipeline::copy_pulse`). Armed from the matching
/// [`actions::Effect`] (`TypeImpact` / `DeleteSquash` / `Gulp` / `LineLand` /
/// `CopyPulse`).
#[derive(Clone, Copy)]
enum CaretImpact {
    Type,
    Delete,
    Gulp,
    Land,
    Copy,
}

/// A one-bit latest-wins gate for expensive zoom layout. Winit may deliver many
/// wheel/key events before the redraw they request; every event updates
/// `App::zoom`, while this gate makes the redraw consume exactly one reflow at
/// the newest value. This is deliberately present-opportunity coalescing, not a
/// time debounce: the very next redraw still paints the requested zoom.
#[derive(Default)]
struct ZoomReflow {
    pending: bool,
}

impl ZoomReflow {
    fn queue(&mut self) {
        self.pending = true;
    }

    fn take(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }

    fn clear(&mut self) {
        self.pending = false;
    }
}

struct Gpu {
    instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// The format the frame is RENDERED through — always the sRGB variant of the
    /// surface's config format. On native it EQUALS `config.format` (the platform
    /// already offers an `*-Srgb` surface format). On the web the canvas only
    /// permits a NON-srgb config format (`bgra8unorm`/`rgba8unorm`; the WebGPU
    /// spec forbids an `*-srgb` primary canvas format), so we configure the base
    /// format, list its srgb variant in `config.view_formats`, and render through
    /// an srgb VIEW — otherwise the shader-linearised grounds/selection/caret get
    /// written WITHOUT the sRGB encode and the whole scene reads too dark (the
    /// margins collapse to near-black). See `Gpu::new`.
    view_format: wgpu::TextureFormat,
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
/// Window-lifecycle + redraw arms lifted out of `window_event`: focus-lost
/// flush, resize / DPI refold, and the redraw-request frame loop.
mod window;
/// The apply bridge: resolve an `Action` + live-only side effects.
mod apply;
/// The single-instance DAEMON's App-side wiring (native only): react to a
/// posted `DaemonEvent`, finish a buffer for C-x #, tear down on quit.
mod daemon;
/// The native macOS MENU BAR's App-side wiring (`cfg(target_os = "macos")`
/// only): resolve a fired menu item's id and re-dispatch it through the SAME
/// `App::apply` seam a keypress uses. `crate::menu` owns the pure roster +
/// muda construction; see its module doc for the full design.
mod menu;
/// SESSION RESTORE's App-side wiring (native only): capture + apply the
/// persisted open-file set / active buffer / cursor+scroll / window frame.
mod session;
/// LIFETIME STATS' App-side wiring (native only): the tracking hooks
/// (keystrokes/chars/active-time/per-world on the keyboard-input path, caret
/// travel on view sync, files-touched on open) + the flush on the autosave
/// triggers. `crate::stats` owns the pure store + injected-clock helpers.
mod stats;

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
    /// HOLD-⌘ SHORTCUT PEEK arm state (`crate::peek::PeekArm`): the pure hold/cancel
    /// machine. Fed stimuli from the raw input handlers (`ModifiersChanged` → the
    /// convention's bare arming modifier alone/broken — `peek::is_bare_arming_modifier`,
    /// ⌘ on Mac, Ctrl on Linux — a joined key press, a mouse press / blur) and the
    /// hold-timer deadline; its result drives the process-global (`peek::set_open`) +
    /// the single `WaitUntil` in `about_to_wait`. LIVE-ONLY — a headless capture never
    /// constructs an `App`, so the peek is summoned there only by the `--peek` flag.
    peek_arm: crate::peek::PeekArm,
    /// When the convention's bare arming modifier went down alone (the `Idle → Pending`
    /// edge), or `None` when not pending — the single `WaitUntil` deadline base: the
    /// peek opens once `peek_armed_at + HOLD_PEEK_MS` elapses with the hold unbroken.
    /// Armed only while `peek_arm == Pending`, so the app idles at 0% CPU once it
    /// resolves (the which-key pause pattern).
    peek_armed_at: Option<Instant>,
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
    /// Pixel position of the CURRENT press (`cursor_px` at the moment `on_press`
    /// ran) — the drag-arm anchor `drag_armed` measures pointer travel against.
    /// Physical px, like `cursor_px`. See `App::exceeds_drag_slop` (`app/input/mouse.rs`).
    drag_press_px: (f32, f32),
    /// True once the pointer has traveled past the drag-arm SLOP threshold since
    /// the current press (`App::exceeds_drag_slop`) — sticky for the rest of the
    /// gesture once tripped. THE PHANTOM-SELECTION FIX: a WYSIWYG reveal reflow can
    /// relocate glyphs under an otherwise-STATIONARY pointer between press and
    /// release (concealed markup regaining its real advance once the caret lands on
    /// that line), which used to look identical to a real drag because `on_drag`
    /// re-hit-tested on every `CursorMoved` regardless of actual pixel travel. Now a
    /// `CursorMoved` while `dragging` only extends the selection once real travel is
    /// proven — a reflow under a still pointer reads as a plain click (no selection
    /// arms), never a drag. Reset to `false` on every fresh press. See `on_press` /
    /// `on_cursor_moved` in `app/input/mouse.rs`.
    drag_armed: bool,
    /// True while a DIRECT page-width resize drag is in progress (a press that landed
    /// on a page-column edge; the grabbed rendered edge drives the measure LIVE,
    /// and the release commits + persists it). Mutually exclusive with a text
    /// selection `dragging` — a press near a boundary starts this instead.
    page_resizing: bool,
    /// The page edge that armed the active width drag. Captured at press time so
    /// adaptive outline-rail reflow cannot switch the gesture's geometry mid-drag.
    page_resize_edge: Option<crate::render::ResizeEdge>,
    /// INLINE-IMAGE DRAG-RESIZE (v2, live app only): `Some` while a press that landed
    /// on an image's bottom-right resize handle is being dragged — the pointer's
    /// distance past the image's left edge drives its DISPLAY WIDTH live (a pipeline
    /// preview, not a buffer edit), and the release writes the `|NNN` hint back as ONE
    /// undoable edit. Mutually exclusive with `page_resizing` AND a text-selection
    /// `dragging` — the press begins exactly one of the three. See `app/input/`.
    image_resizing: Option<crate::app::input::ImageDrag>,
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
    /// The persisted RECENT PROJECT ROOTS (newest-first, capped, deduped — see
    /// [`crate::recents`]). Loaded once at launch (native only; empty on wasm /
    /// headless), pushed-to-front + saved on every switch-project
    /// ([`App::switch_project`]), and offered by the Recent Projects picker.
    recent_projects: Vec<PathBuf>,
    /// The persisted RECENTLY-OPENED FILES MRU (ABSOLUTE paths, most-recent FIRST,
    /// capped + deduped — see [`crate::recent_files`]). Loaded once at launch,
    /// pushed-to-front + saved on every real-file open ([`App::push_recent_file`],
    /// called from [`App::load_path`]). Drives BOTH the go-to ranker's
    /// "recently-opened" tier AND the go-to "Recent" LENS (which shows ONLY the
    /// files in this MRU, in this order). Live/native only; the headless capture
    /// never constructs an `App`, so it never reads or writes this store.
    recent_files: Vec<PathBuf>,
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
    /// Our last-known on-disk STAT (mtime + byte length) of the open file (stamped
    /// on load and after each of our own writes) — the CLOBBER GUARD's baseline: an
    /// autosave first re-stats the file, and a mismatch (moved mtime OR a
    /// same-tick size change) means someone else wrote it, so the write is HELD
    /// with a calm notice instead of overwriting external edits. Wasm-safe (the
    /// times are `crate::clock::SystemTime`, never std).
    disk_mtime: Option<crate::fs::Metadata>,
    /// Buffer version of the no-path SCRATCH buffer last stashed to
    /// [`crate::fs::scratch_stash_path`], so an unchanged scratch isn't re-written.
    scratch_saved_version: Option<u64>,
    /// Last-known on-disk stat of the scratch stash (two-instance clobber
    /// safety, mirroring `disk_mtime`).
    scratch_mtime: Option<crate::fs::Metadata>,
    /// When the AUTOSAVE ENGINE last wrote successfully THIS session (the doc
    /// autosave OR the scratch stash) — stamped ONLY inside `autosave_doc_now` /
    /// `stash_scratch_now`'s `Ok` arms (i.e. exclusively through
    /// `App::autosave_flush`'s one door, past its clobber-guard check), so the
    /// debug panel's `autosave saved · Ns ago` line can never claim a write the
    /// engine didn't just make. `None` before the first successful write. Feeds
    /// `crate::debug::autosave_state` at redraw time (gated on `debug_on()`, like
    /// every other clock read the panel takes) — never read otherwise.
    autosave_last_ok: Option<Instant>,
    /// NOTES VERBS round: when the LAST successful write of ANY kind landed —
    /// manual save (`finish_manual_save`), the scratch→note conversion
    /// (`convert_scratch_and_save`), a note's own debounced autosave
    /// (`autosave_note`), OR the document autosave engine (`autosave_doc_now` /
    /// `stash_scratch_now`) — stamped alongside each of those on SUCCESS,
    /// deliberately a SEPARATE field from `autosave_last_ok` (that one is scoped
    /// to the autosave engine's own debug-panel line and would otherwise
    /// conflate "the engine wrote" with "anything wrote"). Feeds the held HUD's
    /// SAVED stat (`App::sync_hud_saved`) — `None` before the first successful
    /// write this session.
    last_saved_ok: Option<Instant>,
    /// A CALM NOTICE for the bottom of the canvas. Completed file events are
    /// live-only TOASTS; conditions needing a decision (failures and the external
    /// edit guard) are STICKY until resolved. `None` draws nothing.
    notice: Option<String>,
    notice_kind: NoticeKind,
    notice_expires_at: Option<Instant>,
    /// Newest unacknowledged crash-log filename. It never occupies the center
    /// notice: About and Settings surface it passively until Report a Problem
    /// acknowledges it.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pending_crash: Option<String>,
    /// SAVE-FEEDBACK round: the dirty-state the window title/titlebar last
    /// rendered — the cheapest honest hook for "on dirty-state transitions"
    /// (`CLAUDE.md`'s own phrasing): `sync_view` (already called on nearly
    /// every edit/cursor-move, gated on the gpu-present check) compares
    /// `buffer.is_dirty()` against this cache and only re-titles on an actual
    /// FLIP, so typing doesn't re-format the title string or make a
    /// `set_title`/`set_document_edited` OS call every keystroke — only when
    /// clean↔dirty actually changes. `App::update_title` is the ONE place
    /// that writes it back in step (so ANY caller of `update_title`, not just
    /// `sync_view`'s own comparison, keeps this cache honest).
    title_dirty: bool,
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
    /// Latest-wins zoom layout gate. Wheel/Cmd-zoom input updates `zoom`
    /// immediately, but the expensive document reflow is consumed once at the
    /// next present opportunity. Any intervening ordinary `sync_view` clears it
    /// because that sync necessarily applies the newest zoom already.
    zoom_reflow: ZoomReflow,
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
    /// AMBIENT LAVA TICK — the lava-lamp ground's slow ~10 fps drift clock
    /// (`crate::lava`). `Some(when)` = the last tick instant, driving the single
    /// `WaitUntil(when + LAVA_TICK)` in `about_to_wait` that advances the phase by
    /// one FIXED/CLAMPED ambient step +
    /// requests one redraw + re-arms — a SLOW sparse cadence, NEVER the caret
    /// spring's hot per-frame loop. Armed ONLY while `lava::lava_should_tick` holds
    /// (a lava world active, `ambient_motion` on, motion not reduced, the window
    /// focused), so a non-lava world (every world today) schedules ZERO frames and
    /// idles at 0% CPU. Cleared on blur / whenever the lamp goes static. `None` =
    /// not ticking (live only — a headless capture never constructs this, so it can
    /// never advance the phase; a capture is always the frozen t=0 phase).
    lava_tick_at: Option<Instant>,
    /// Whether the window currently HAS focus — tracked so the ambient lava tick
    /// PAUSES on blur (`WindowEvent::Focused`). Starts `true` (a window is focused
    /// on creation); only the live App reads it (the ambient-tick gate), so a
    /// headless capture is unaffected.
    focused: bool,
    /// LIVE-RESIZE settle state (all platforms; macOS additionally synchronizes
    /// the Core Animation transaction): when the last genuine
    /// `WindowEvent::Resized` tick landed. While present, lava holds its last
    /// settled field geometry; on macOS the CAMetalLayer's
    /// `presentsWithTransaction` is armed ON. Metal's surface presents
    /// ASYNCHRONOUSLY by default, so during a FAST drag the window's own
    /// resize animation (a Core Animation transaction AppKit commits on every
    /// tick) can commit before our next frame presents — the compositor then
    /// has nothing fresh to show and instead SCALES the last-presented
    /// (stale-size) drawable to cover the new bounds, the classic macOS
    /// "content stretches while dragging fast" artifact (user-reported: a
    /// SLOW, one-pixel-at-a-time drag was already fine after the rail-ramp
    /// fix; a FAST drag still visibly stretched). `presentsWithTransaction
    /// (true)` makes our present JOIN that same transaction instead of racing
    /// it. `about_to_wait`'s `RESIZE_SYNC_SETTLE` debounce flips it back OFF
    /// once ticks stop arriving (Apple's own documented trade-off: a
    /// transaction-synced present costs a touch of throughput, so it's armed
    /// only while genuinely dragging, not left on permanently). `None` =
    /// not currently resizing (live only; a headless capture never resizes a
    /// real window, so this is structurally unreachable there).
    resize_settle_at: Option<Instant>,
    /// A stream of `Moved` events means the window-server is actively moving
    /// the window. Hold ambient lava presents until this debounce settles.
    move_settle_at: Option<Instant>,
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
    /// LIFETIME STATS odometer (native only, `stats` config-gated): the persisted
    /// running counters, loaded once at launch and flushed on the autosave
    /// triggers (idle/blur/switch/quit). Lives ONLY on the live `App` — the
    /// headless capture never constructs one, so `stats.toml` is untouchable
    /// there (tripwire: `headless_replay_never_touches_the_stats_file`).
    #[cfg(not(target_arch = "wasm32"))]
    stats: crate::stats::Stats,
    /// Monotonic base for the active-writing clock: `stats_origin.elapsed()` gives
    /// the millis a keystroke is stamped at (fed as `now_ms` into the injected-
    /// clock [`crate::stats::active_delta`] rule).
    #[cfg(not(target_arch = "wasm32"))]
    stats_origin: Instant,
    /// The previous keystroke's stamp (millis since `stats_origin`), the `last`
    /// side of the active-writing interval. `None` until the first keystroke this
    /// session, so that first press banks no interval.
    #[cfg(not(target_arch = "wasm32"))]
    stats_last_input_ms: Option<u64>,
    /// The caret's last-sampled DOCUMENT-space position (scroll-independent), for
    /// the caret-travel accumulator — diffed against the current position each
    /// `sync_view`, but only ADDED when the logical cursor actually moved (so a
    /// scroll or a reshape never fakes distance). `None` until the first sample.
    #[cfg(not(target_arch = "wasm32"))]
    stats_last_caret_xy: Option<(f32, f32)>,
    /// The caret's last-sampled logical (line, col) — the gate that decides
    /// whether a `stats_last_caret_xy` change is a real move (counted) or mere
    /// re-layout/scroll (anchor refreshed, no distance added).
    #[cfg(not(target_arch = "wasm32"))]
    stats_last_cursor: Option<(usize, usize)>,
    /// Whether the odometer has unsaved increments since the last flush, so a
    /// flush with nothing new skips the atomic write.
    #[cfg(not(target_arch = "wasm32"))]
    stats_dirty: bool,
    /// SINGLE-INSTANCE DAEMON (native only, and compiled out under `mas` — see
    /// `crate::daemon`'s module doc): the socket special file's path, so
    /// `daemon::daemon_shutdown` can unlink it on a clean quit — `None` when this
    /// launch never became the instance (a socket error degraded to a normal,
    /// non-singleton launch; see `crate::app::run`).
    #[cfg(all(not(target_arch = "wasm32"), not(feature = "mas")))]
    daemon_socket_path: Option<PathBuf>,
    /// SINGLE-INSTANCE DAEMON (native only, and compiled out under `mas`):
    /// every daemon `--wait` client's still-
    /// open connection, keyed by the [`crate::buffers::BufferKey`] of the buffer it
    /// is waiting on. `Action::FinishBuffer` (Cmd-W) notifies + drains the entry for
    /// the buffer being finished; `daemon::daemon_shutdown` drains everything on
    /// quit (a dropped `Waiter` closes its socket, which the client treats as done
    /// too — see `crate::daemon`'s module doc).
    #[cfg(all(not(target_arch = "wasm32"), not(feature = "mas")))]
    wait_conns: std::collections::HashMap<crate::buffers::BufferKey, Vec<crate::daemon::Waiter>>,
    /// NATIVE MACOS MENU BAR: the event-loop proxy stashed at construction so
    /// `resumed()` can install the menu bar (and register muda's event
    /// handler) once NSApp/the window exists — menu install needs to happen
    /// AFTER window creation, but the proxy is only obtainable in
    /// `crate::app::run`, before control ever reaches `resumed()`. Taken
    /// (`Option::take`) the one time it is used, so a second `resumed()` call
    /// (there isn't one today, but the existing `gpu.is_some()` guard already
    /// covers that) can never double-install. `None` after install, or in any
    /// test build that never goes through `crate::app::run`.
    #[cfg(target_os = "macos")]
    menu_proxy: Option<winit::event_loop::EventLoopProxy<AwlEvent>>,
    /// The installed menu bar's Rust-side handle, kept alive for the app's
    /// whole lifetime. **This field's only job is to never be dropped before
    /// `App` itself is.** `crate::menu::install`'s doc explains why: every
    /// native `NSMenuItem` stashes a raw (non-retaining) pointer back into
    /// this value's owned `Rc<RefCell<MenuChild>>` chain, so letting it drop
    /// (the v1 bug — the return value used to be an unstored local) leaves
    /// every menu item pointing at freed memory, and clicking ANY of them —
    /// About, Quit, a routed item — is a use-after-free. Never read after
    /// `resumed()` stores it; `Option` only so the field can start `None`
    /// before the window/NSApp exist.
    #[cfg(target_os = "macos")]
    _menu_bar: Option<muda::Menu>,
}

impl App {
    fn new(
        file: Option<PathBuf>,
        root: PathBuf,
        cli_workspace: Option<PathBuf>,
        cli_notes_root: Option<PathBuf>,
        config: Config,
    ) -> Self {
        // ACCESSIBILITY TIER 1 — REDUCE MOTION: resolve the config->OS ladder
        // ONCE, here, at live startup (native + wasm both construct `App`
        // through this one seam). See `motion.rs`'s module doc for the full
        // resolution ladder + the determinism guarantee this call site is the
        // ONLY place in the whole codebase that may consult OS/browser motion
        // detection — never a headless capture path.
        crate::motion::apply_at_startup(&config);
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
                Ok(_) => Buffer::scratch(), // present but empty: nothing to preserve
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Buffer::scratch(),
                Err(_) => {
                    // PRESERVE-ON-CORRUPT (the scratch stash IS a manuscript):
                    // the file exists but failed to decode as UTF-8 text — a
                    // real corruption signal, not a fresh install. Back up
                    // the raw bytes to a `.corrupt-*` sibling before falling
                    // back to a blank scratch buffer, so those bytes are
                    // never silently discarded (and never overwritten away
                    // by the very next scratch-stash flush).
                    if let Ok(raw) = crate::fs::active().read(&stash) {
                        crate::durable::preserve_corrupt(&stash, &raw);
                    }
                    Buffer::scratch()
                }
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
        // Load the persisted RECENT PROJECT ROOTS (the Recent Projects picker's
        // MRU). Through the `FileSystem` seam, so it degrades to an empty list on a
        // fresh install (missing file) and works on wasm (WebFs) too. Only ever
        // reached on the live `App` — the headless capture never constructs one.
        let recent_projects = crate::recents::load(&crate::recents::recents_path());
        // Load the persisted RECENTLY-OPENED FILES MRU (the go-to Recent lens's
        // source). Same `FileSystem` seam + degrade-to-empty leniency as the
        // recent-projects load above; only ever reached on the live `App`.
        let recent_files = crate::recent_files::load();
        // Build the keymap with the config `[keys]` rebinds AND the EFFECTIVE
        // `linux_keep_emacs` list applied over the defaults — `effective_linux_keep`
        // widens to the whole keymap-flavor preset under `keymap = "emacs"`, else is
        // the raw list unchanged (see `Config::effective_linux_keep`'s doc).
        //
        // CONVENTION-TRUTHFUL SURFACES ROUND: on `Platform::Web`, every browser-
        // reserved command's web-alternate chord (`commands::web_alternate_keys`)
        // is merged in BEHIND the user's own `[keys]` — config still trumps
        // everything, since `web_alternate_keys` itself skips any command the
        // user has already rebound. A no-op `vec![]` on `Platform::Native`, so a
        // native build's keymap is unaffected byte-for-byte.
        let mut keys_with_web_alt = config.keys.clone();
        keys_with_web_alt.extend(crate::commands::web_alternate_keys(&config.keys, crate::convention::Convention::current(), crate::commands::Platform::current()));
        let keymap = KeymapState::with_overrides_and_keep(&keys_with_web_alt, &config.effective_linux_keep());
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
            peek_arm: crate::peek::PeekArm::default(),
            peek_armed_at: None,
            zoom,
            dpi: 1.0,
            cursor_px: (0.0, 0.0),
            dragging: false,
            drag_press_px: (0.0, 0.0),
            drag_armed: false,
            page_resizing: false,
            page_resize_edge: None,
            image_resizing: None,
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
            recent_projects,
            recent_files,
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
            last_saved_ok: None,
            notice: None,
            notice_kind: NoticeKind::Sticky,
            notice_expires_at: None,
            pending_crash: None,
            title_dirty: false,
            history_preview: None,
            history_scroll_before: None,
            zoom_persist_at: None,
            zoom_reflow: ZoomReflow::default(),
            theme_font_at: None,
            lava_tick_at: None,
            focused: true,
            resize_settle_at: None,
            move_settle_at: None,
            config,
            cli_notes_root,
            cli_workspace,
            buffer_registry: crate::buffers::BufferRegistry::default(),
            #[cfg(not(target_arch = "wasm32"))]
            restored_window: None,
            // LIFETIME STATS: load the persisted odometer through the same
            // `FileSystem` seam the recent-* MRUs use (degrades to an empty
            // `Stats` on a fresh install), and start the active-writing clock.
            // Only ever reached on the live `App` — never the headless capture.
            #[cfg(not(target_arch = "wasm32"))]
            stats: crate::stats::load(&crate::stats::stats_path()),
            #[cfg(not(target_arch = "wasm32"))]
            stats_origin: Instant::now(),
            #[cfg(not(target_arch = "wasm32"))]
            stats_last_input_ms: None,
            #[cfg(not(target_arch = "wasm32"))]
            stats_last_caret_xy: None,
            #[cfg(not(target_arch = "wasm32"))]
            stats_last_cursor: None,
            #[cfg(not(target_arch = "wasm32"))]
            stats_dirty: false,
            #[cfg(all(not(target_arch = "wasm32"), not(feature = "mas")))]
            daemon_socket_path: None,
            #[cfg(all(not(target_arch = "wasm32"), not(feature = "mas")))]
            wait_conns: std::collections::HashMap::new(),
            #[cfg(target_os = "macos")]
            menu_proxy: None,
            #[cfg(target_os = "macos")]
            _menu_bar: None,
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
        // A previous crash is passive state, not a startup interruption: retain
        // the marker for About + Settings, and acknowledge it only when the user
        // chooses Report a Problem.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dir = crate::crashlog::crashes_dir();
            app.pending_crash = crate::crashlog::pending_notice(&dir);
        }
        app
    }

    fn set_sticky_notice(&mut self, text: impl Into<String>) {
        self.notice = Some(text.into());
        self.notice_kind = NoticeKind::Sticky;
        self.notice_expires_at = None;
    }

    fn set_toast_notice(&mut self, text: impl Into<String>) {
        self.notice = Some(text.into());
        self.notice_kind = NoticeKind::Toast;
        // A real window is the live/capture boundary: unit tests and headless
        // replay keep the text deterministic but never arm a wall-clock expiry.
        self.notice_expires_at = self.gpu.as_ref().map(|_| Instant::now() + TOAST_LIFETIME);
    }

    fn clear_notice(&mut self) {
        self.notice = None;
        self.notice_kind = NoticeKind::Sticky;
        self.notice_expires_at = None;
    }

    fn clobber_notice_active(&self) -> bool {
        self.notice_kind == NoticeKind::Sticky && self.notice.as_deref() == Some(CLOBBER_NOTICE)
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
        // Pin reduce-motion OFF for hermetic App-level tests too (mirrors the
        // `session_restore` override above): `App::new` calls
        // `motion::apply_at_startup`, and a test runner whose machine actually
        // has the OS "Reduce Motion" preference on must not silently flip every
        // spring/flinch test's animation behavior to instant-settle.
        let config = Config { session_restore: Some(false), reduce_motion: Some(false), ..config };
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

/// The winit USER EVENT type this app's event loop carries: the single-
/// instance daemon's posted events on every native platform, PLUS (macOS
/// only) a fired native menu-bar item's raw id — an uninhabited no-op on wasm
/// (the browser has no process/socket/menu-bar concept; `crate::daemon` and
/// `crate::menu` both compile out there entirely). Growing this enum (the
/// `Menu` variant) is what FORCES `user_event`'s match below to grow a
/// matching arm — the exhaustiveness check is the whole point.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) enum AwlEvent {
    /// A posted [`crate::daemon::DaemonEvent`] (see `crate::daemon`'s module
    /// doc) — absent under `mas` (the daemon module compiles out entirely
    /// there; see `src/mas.rs`'s module doc).
    #[cfg(not(feature = "mas"))]
    Daemon(crate::daemon::DaemonEvent),
    /// A fired native macOS menu-bar item's raw muda id string (see
    /// `crate::menu`'s module doc) — resolved to an `Action` and re-dispatched
    /// through the SAME `App::apply` seam a keypress uses
    /// (`App::handle_menu_event`, `app/menu.rs`).
    #[cfg(target_os = "macos")]
    Menu(String),
}
#[cfg(target_arch = "wasm32")]
type AwlEvent = ();

impl ApplicationHandler<AwlEvent> for App {
    /// A daemon event or (macOS only) a fired menu item, posted by their
    /// respective source (the daemon's accept-loop thread / muda's global
    /// event handler) via `EventLoopProxy::send_event` — always runs on this,
    /// the normal winit thread. A no-op on wasm (there is no `AwlEvent`
    /// variant to construct there).
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: AwlEvent) {
        #[cfg(not(target_arch = "wasm32"))]
        match _event {
            #[cfg(not(feature = "mas"))]
            AwlEvent::Daemon(e) => self.handle_daemon_event(e),
            #[cfg(target_os = "macos")]
            AwlEvent::Menu(id) => self.handle_menu_event(id, _event_loop),
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        // THE PURE title string (`app::files::window_title`) — same owner
        // `App::update_title` uses on every later open/switch/theme-cycle, so
        // the very first frame's window title already names the document (and
        // the active world) rather than starting bare and waiting for the
        // first `update_title()` call to catch up.
        let title = files::window_title(
            self.file.as_deref(),
            self.buffer.is_note(),
            crate::theme::active().name,
            self.is_document_dirty(),
        );
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
                // NATIVE MACOS MENU BAR: install now that the window (and
                // therefore NSApp) exists — `Menu::init_for_nsapp` and the
                // root `Menu`'s own construction both require the real
                // process main thread, which `resumed()` always runs on.
                // `menu_proxy` is `take()`n so a later `resumed()` call (the
                // `gpu.is_some()` guard at the top already prevents that
                // today) could never double-install. The returned `Menu` is
                // STORED in `self._menu_bar`, never just dropped — see that
                // field's doc + `crate::menu::install`'s doc for the
                // use-after-free this fixes (every native `NSMenuItem` keeps
                // a raw, non-retaining pointer into this value's Rc chain).
                #[cfg(target_os = "macos")]
                if let Some(proxy) = self.menu_proxy.take() {
                    self._menu_bar = Some(crate::menu::install(proxy, AwlEvent::Menu));
                    // TEMPLATE ICONS: mark every routed item's NSImage a template
                    // image so AppKit tints it to the current appearance's label
                    // ink (and the correct on-highlight tint) instead of the
                    // pre-baked flat gray `menu_icons.rs` draws — must run AFTER
                    // `install` has handed the real NSMenu tree to AppKit.
                    crate::mac_chrome::mark_menu_icons_as_templates();
                }
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
        // A thin dispatcher: each substantial arm's body lives in a focused
        // method (`app/input/` for the input arms, `app/window.rs` for the
        // window-lifecycle + redraw arms). The ORDER and early-returns are
        // winit-sensitive; each delegate reproduces its arm verbatim, so the
        // `return`s that were arm-level are now method-level (nothing runs after
        // the match, so the two are behaviourally identical).
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(true) => self.on_focus_gained(),
            WindowEvent::Focused(false) => self.on_focus_lost(),
            WindowEvent::Resized(size) => self.on_resized(size),
            WindowEvent::Moved(position) => self.on_moved(position),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.on_scale_factor_changed(scale_factor);
            }
            WindowEvent::ModifiersChanged(m) => self.on_modifiers_changed(m),
            WindowEvent::CursorMoved { position, .. } => self.on_cursor_moved(position),
            WindowEvent::MouseInput { state, button, .. } => {
                self.on_mouse_input(event_loop, state, button);
            }
            WindowEvent::MouseWheel { delta, .. } => self.on_mouse_wheel(delta),
            WindowEvent::Ime(ime) => self.on_ime(ime),
            WindowEvent::KeyboardInput { event, .. } => self.on_keyboard_input(event_loop, event),
            WindowEvent::RedrawRequested => self.on_redraw_requested(event_loop),
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
        // LIFETIME STATS: the final odometer flush, mirroring the session flush
        // right above it (native only; config + dirty gated inside).
        #[cfg(not(target_arch = "wasm32"))]
        self.stats_flush();
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "mas")))]
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
        // HOLD-⌘ SHORTCUT PEEK: while a bare-arming-modifier hold is PENDING, summon the
        // card once ~600ms elapses with the hold unbroken. The timer is ARMED ONLY while
        // `peek_armed_at` is `Some` (the `PeekArm::Pending` state) — a single `WaitUntil`
        // deadline, no perpetual tick; feeding `Elapsed` opens the card and clears the
        // stamp, so nothing re-arms and the app idles at 0% CPU (the which-key pattern).
        if let Some(armed) = self.peek_armed_at {
            let deadline = armed + Duration::from_millis(crate::peek::HOLD_PEEK_MS);
            if Instant::now() >= deadline {
                self.feed_peek(crate::peek::PeekStimulus::Elapsed);
            } else if self.last_frame.is_none() {
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
                    // LIFETIME STATS: piggyback the same ~1s idle door, so the
                    // odometer is crash-safe without its own timer (native only;
                    // config + dirty gated inside).
                    #[cfg(not(target_arch = "wasm32"))]
                    self.stats_flush();
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
                    // Fire like the sibling debounces above: request a redraw so the
                    // RedrawRequested handler re-decides control flow (Wait when settled),
                    // instead of leaving it at this now-elapsed WaitUntil — which would
                    // busy-spin the loop at ~100% CPU until the next input (DESIGN §6).
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.window.request_redraw();
                    }
                }
                false if self.last_frame.is_none() => {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(dirty + ZOOM_PERSIST_DEBOUNCE));
                }
                false => {}
            }
        }
        // LIVE-RESIZE CONTENT-STRETCH FIX settle (macOS only — see
        // `resize_settle_at`'s doc): once `RESIZE_SYNC_SETTLE` passes with no
        // further `Resized` ticks, flip the CAMetalLayer's `presentsWithTransaction`
        // back OFF (paying its throughput cost only while a drag is actually live).
        // Each new tick RE-STAMPS `resize_settle_at` (`App::arm_live_resize_sync`),
        // sliding the deadline exactly like the theme-font/zoom-persist debounces
        // above — the same single-`WaitUntil` shape, so a still window costs nothing.
        if let Some(dirty) = self.resize_settle_at {
            match debounce_due(dirty, RESIZE_SYNC_SETTLE, Instant::now()) {
                true => {
                    self.resize_settle_at = None;
                    if let Some(gpu) = self.gpu.as_mut() {
                        gpu.pipeline
                            .settle_lava_field_viewport(gpu.config.width, gpu.config.height);
                    }
                    #[cfg(target_os = "macos")]
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.set_presents_with_transaction(false);
                    }
                    // Same reason as the sibling debounces above: route through
                    // `on_redraw_requested`'s own control-flow decision instead of
                    // leaving this now-elapsed `WaitUntil` in place (which would
                    // busy-spin `about_to_wait` until the next real input).
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.window.request_redraw();
                    }
                }
                false if self.last_frame.is_none() => {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(dirty + RESIZE_SYNC_SETTLE));
                }
                false => {}
            }
        }
        if let Some(dirty) = self.move_settle_at {
            match debounce_due(dirty, MOVE_SETTLE, Instant::now()) {
                true => {
                    self.move_settle_at = None;
                    self.lava_tick_at = None;
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.window.request_redraw();
                    }
                }
                false if self.last_frame.is_none() => {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(dirty + MOVE_SETTLE));
                }
                false => {}
            }
        }
        // AMBIENT LAVA TICK — the lava-lamp ground's slow ~10 fps drift, awl's
        // FIRST time-varying background. A single `WaitUntil` cadence (NEVER the
        // caret spring's hot per-frame `Poll` loop): when it elapses, advance the
        // phase, request ONE redraw, and re-arm. Armed ONLY while
        // `lava::lava_should_tick` holds — a lava world is active AND
        // `ambient_motion` is on AND motion is not reduced AND the window is
        // focused (pause on blur). Firetail + Mangrove are the two lava worlds;
        // every other world has a static background, so `is_lava()` is false and
        // schedules ZERO ambient frames — preserving 0% idle CPU there.
        let lava_active = crate::theme::background().is_lava();
        let lava_paused = self.resize_settle_at.is_some()
            || self.move_settle_at.is_some()
            || self
                .gpu
                .as_ref()
                .is_some_and(|gpu| gpu.pipeline.lava_blur_active());
        if crate::lava::lava_should_tick(
            lava_active,
            self.config.ambient_motion_on(),
            crate::motion::reduced(),
            self.focused,
            lava_paused,
        ) {
            let now = Instant::now();
            match self.lava_tick_at {
                Some(last) if now.saturating_duration_since(last) >= LAVA_TICK => {
                    // Due: hand the elapsed time to the bounded ambient advance
                    // (`lava::ambient_tick_dt` clamps it to one fixed tick), so a
                    // delayed macOS window-drag wake cannot catch up in one jump.
                    // The follow-up `about_to_wait` pass (after the redraw) re-arms
                    // the single `WaitUntil` via the `_` arm below.
                    let dt = (now - last).as_secs_f32();
                    self.lava_tick_at = Some(now);
                    if let Some(gpu) = self.gpu.as_mut() {
                        gpu.pipeline.advance_lava(dt);
                        gpu.window.request_redraw();
                    }
                }
                _ => {
                    // Not due yet (or the first arm): keep/arm the single
                    // `WaitUntil`, but NEVER override the caret spring's hot `Poll`
                    // loop (guard on `last_frame`, like every sibling debounce) —
                    // during a caret glide the lava still advances above at ~10 fps
                    // while the frame itself redraws at full refresh.
                    let last = *self.lava_tick_at.get_or_insert(now);
                    if self.last_frame.is_none() {
                        event_loop.set_control_flow(control_flow_with_deadline(
                            event_loop.control_flow(),
                            last + LAVA_TICK,
                        ));
                    }
                }
            }
        } else if lava_active {
            // A lava world, but the lamp must be STATIC: reduce motion OR ambient
            // motion off (blur is handled at the focus edge, which merely HOLDS the
            // phase). Stop ticking; hard-freeze the phase to the settled frame so a
            // later resume restarts cleanly rather than from a stale mid-bob.
            self.lava_tick_at = None;
            if crate::motion::reduced() || !self.config.ambient_motion_on() {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.pipeline.freeze_lava();
                }
            }
        }
        // EVENT TOAST expiry: one live-only deadline, consumed once. This runs
        // after sibling timers so it can choose the EARLIER deadline instead of
        // delaying a lava tick (or being delayed by one). Poll always wins.
        if let Some(deadline) = self.notice_expires_at {
            if notice_expired(self.notice_kind, Some(deadline), Instant::now()) {
                self.clear_notice();
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            } else if self.last_frame.is_none() {
                event_loop.set_control_flow(control_flow_with_deadline(
                    event_loop.control_flow(),
                    deadline,
                ));
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

/// Compose one idle deadline with the event loop's current intent. A hot `Poll`
/// always wins; an unscheduled `Wait` accepts the proposal; and two deadlines
/// resolve to the earlier one so a slow ambient concern cannot delay a faster
/// sibling timer. Pure, keeping the shared lava/toast scheduling law testable
/// without a window or event loop.
fn control_flow_with_deadline(current: ControlFlow, proposed: Instant) -> ControlFlow {
    match current {
        ControlFlow::Poll => ControlFlow::Poll,
        ControlFlow::Wait => ControlFlow::WaitUntil(proposed),
        ControlFlow::WaitUntil(current) => ControlFlow::WaitUntil(current.min(proposed)),
    }
}

/// Pure notice lifetime law: only a Toast carrying a reached live deadline may
/// disappear. Sticky state and clockless/headless toasts never expire.
fn notice_expired(kind: NoticeKind, deadline: Option<Instant>, now: Instant) -> bool {
    kind == NoticeKind::Toast && deadline.is_some_and(|d| now >= d)
}

/// Does this modifier set request wheel-zoom? Cmd/Super only (NOT Ctrl), so a
/// Ctrl+scroll falls through to normal free scrolling. Pure, so it's unit-testable
/// without a window/event loop.
fn scroll_zoom_intent(mods: ModifiersState) -> bool {
    mods.contains(ModifiersState::SUPER)
}

#[cfg(test)]
#[test]
fn zoom_reflow_gate_collapses_a_burst_to_one_present_opportunity() {
    let mut gate = ZoomReflow::default();
    for _ in 0..12 {
        gate.queue();
    }
    assert!(gate.take(), "a queued burst owes exactly one reflow");
    assert!(!gate.take(), "the same present opportunity cannot reflow twice");
    gate.queue();
    gate.clear();
    assert!(!gate.take(), "an intervening ordinary sync consumes the debt");
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
    // CRASH VISIBILITY (native only — mirrors the daemon's own CAPTURE GATE
    // exactly): install the panic hook FIRST, before any window/GPU/daemon
    // work, so a panic anywhere downstream — including the daemon dance right
    // below — still gets a local crash log. `crate::crashlog::install_hook` is
    // called from ONLY this one door; `--screenshot`/`--keys`/`--bench-*` never
    // reach `crate::app::run` at all, so a headless capture structurally never
    // installs it (tripwire: `main::run::tests::
    // headless_screenshot_never_installs_the_crash_hook`).
    #[cfg(not(target_arch = "wasm32"))]
    crate::crashlog::install_hook();

    // SINGLE-INSTANCE DAEMON (native only, and compiled out entirely under
    // `mas` — see `crate::daemon`'s module doc for the full CAPTURE GATE
    // argument: this whole block lives ONLY on this live-App startup path,
    // never on any headless `--screenshot`/`--bench-*` mode). Runs the
    // bind-or-handoff dance BEFORE any window/GPU work, so handing off to an
    // already-running instance exits in milliseconds with no window ever
    // created. Under `mas`, Launch Services already refuses a second launch
    // and there is no CLI to hand a path off with, so this is simply absent.
    #[cfg(all(not(target_arch = "wasm32"), not(feature = "mas")))]
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

    // Mark this LIVE session's start, so the History picker's Session lens has a
    // floor to bucket versions against. Live-launch-only (never the headless capture,
    // which never reaches `run`), so a capture's Session lens stays inert.
    crate::history::mark_session_start();

    // MAS SECURITY-SCOPED BOOKMARKS: resolve + start accessing every folder
    // grant persisted from an earlier launch, so this launch's FIRST touch of
    // a previously-granted root needs no fresh powerbox panel. Native macOS
    // `mas` builds only — see `src/mas.rs`'s module doc. Lives on this exact
    // live-App startup path (never `--screenshot`/`--keys`), matching every
    // other native-only startup door's capture gate above.
    #[cfg(all(feature = "mas", target_os = "macos"))]
    crate::mas::restore_all_grants();

    let event_loop = EventLoop::<AwlEvent>::with_user_event().build()?;
    #[cfg(not(target_arch = "wasm32"))]
    let proxy = event_loop.create_proxy();
    // `mut` is only needed on native (the macOS-menu-proxy stash + the
    // `run_app(&mut app)` call below); on wasm `app` is moved straight into
    // `spawn_app` without ever being mutated. Kept as ONE call site (never
    // duplicated across a `#[cfg]` split) — a law test below counts every
    // raw constructor call in this file.
    #[allow(unused_mut)]
    let mut app = App::new(file, root, cli_workspace, cli_notes_root, config);
    // NATIVE MACOS MENU BAR: stash a proxy clone now (before the daemon's own
    // clone below is potentially moved away) so `resumed()` can install the
    // menu bar once the window/NSApp exists — see `App::menu_proxy`'s doc.
    #[cfg(target_os = "macos")]
    {
        app.menu_proxy = Some(proxy.clone());
    }
    #[cfg(all(not(target_arch = "wasm32"), not(feature = "mas")))]
    if let Some(listener) = instance_listener {
        app.daemon_socket_path = Some(crate::daemon::socket_path());
        crate::daemon::spawn_accept_thread(listener, proxy, AwlEvent::Daemon);
    }
    // MAS: no daemon exists to hand `--wait` off to (see the module doc) —
    // the flag is simply inert on this flavor, mirroring the wasm no-op below.
    #[cfg(all(not(target_arch = "wasm32"), feature = "mas"))]
    let _ = wait;

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
    fn only_live_toasts_expire_sticky_and_clockless_notices_do_not() {
        let now = Instant::now();
        let deadline = now + TOAST_LIFETIME;
        assert!(!notice_expired(NoticeKind::Toast, Some(deadline), now));
        assert!(notice_expired(NoticeKind::Toast, Some(deadline), deadline));
        assert!(!notice_expired(NoticeKind::Sticky, Some(deadline), deadline));
        assert!(!notice_expired(NoticeKind::Toast, None, deadline));
    }

    #[test]
    fn idle_deadlines_compose_without_delaying_poll_or_an_earlier_timer() {
        let now = Instant::now();
        let earlier = now + Duration::from_millis(40);
        let later = now + Duration::from_millis(100);

        assert_eq!(
            control_flow_with_deadline(ControlFlow::Poll, later),
            ControlFlow::Poll,
            "a hot redraw loop always wins"
        );
        assert_eq!(
            control_flow_with_deadline(ControlFlow::Wait, later),
            ControlFlow::WaitUntil(later),
            "an idle unscheduled loop accepts the proposed deadline"
        );
        assert_eq!(
            control_flow_with_deadline(ControlFlow::WaitUntil(earlier), later),
            ControlFlow::WaitUntil(earlier),
            "a later proposal cannot delay the current earlier deadline"
        );
        assert_eq!(
            control_flow_with_deadline(ControlFlow::WaitUntil(later), earlier),
            ControlFlow::WaitUntil(earlier),
            "an earlier proposal advances the current later deadline"
        );
    }

    #[test]
    fn held_hud_dismisses_when_summon_modifier_lifts() {
        // The stats HUD is a momentary HOLD: summoned with Option-Cmd-I, it must vanish the
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
        let _fs = crate::testlock::serial();
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
        let _fs = crate::testlock::serial();
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
        let effective_keep = app.config.effective_linux_keep();
        let build_ctx = crate::overlay::BuildCtx {
            goto_corpus: app.file_index.clone(),
            goto_open: Vec::new(),
            goto_recent: Vec::new(),
            goto_times: Vec::new(),
            config_keys: &app.config.keys,
            config_linux_keep: &effective_keep,
            goto_headings: Vec::new(),
            spell_target: None,
            history_entries: Vec::new(),
            history_now: None,
            history_session_start: None,
            settings_values: Default::default(),
            assets: Vec::new(),
        };
        let ov = crate::overlay::build(crate::overlay::OverlayKind::Goto, &build_ctx)
            .expect("Goto always summons");
        assert!(ov.corpus.contains(&"b.txt".to_string()), "the new file is listed");
    }

    // ── THE KEYMAP FLAVOR ROUND — the Settings "Keymap" toggle round-trip ────

    /// Enter on the "Keymap" settings row (`App::toggle_keymap_flavor`, the
    /// special-cased door `App::setting_toggle` routes "keymap" through):
    /// flips native <-> emacs, PERSISTS the flip format-preservingly (the same
    /// `persist_pref` owner every other sticky pref rides), and re-applies the
    /// keymap LIVE from the updated in-memory config — proven here by feeding
    /// the SAME `app.config.effective_linux_keep()` a fresh `KeymapState`
    /// would consume (the exact composition `toggle_keymap_flavor` rebuilds
    /// `self.keymap` from) into a `Convention::Linux`-pinned keymap and
    /// confirming it now carries the full emacs preset.
    #[test]
    fn settings_keymap_toggle_flips_persists_and_live_reapplies() {
        use crate::fs::{FileSystem, InMemoryFs};
        let mem = InMemoryFs::new();
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let cfg = Config { path: PathBuf::from("/cfg/config.toml"), ..Config::empty() };
        let mut app = app_on(None, "/proj", cfg);
        assert_eq!(app.config.keymap_flavor(), crate::keymap::KeymapFlavor::Native, "starts native");

        // Enter #1: native -> emacs.
        app.toggle_keymap_flavor();
        assert_eq!(app.config.keymap_flavor(), crate::keymap::KeymapFlavor::Emacs, "in-memory mirror flips");
        let written = mem.read_to_string(std::path::Path::new("/cfg/config.toml")).unwrap();
        assert!(written.contains("keymap = \"emacs\""), "persisted format-preservingly: {written:?}");

        // LIVE RE-APPLY: the same composed keep-list the toggle rebuilt
        // `self.keymap` from now carries the WHOLE emacs preset — build a
        // fresh convention-pinned keymap from exactly that composition (the
        // private `KeymapState.linux_keep` field can't be introspected from
        // here, so this proves the INPUT the live rebuild consumed, which
        // `keymap::tests::keymap_flavor_emacs_preset_reverts_every_displaced_chord_to_emacs_meaning`
        // already proves is sufficient to flip dispatch).
        let effective = app.config.effective_linux_keep();
        let preset = crate::keymap::linux_emacs_preset_keep();
        // The insert-link-yields-to-kill-line round's built-in floor
        // (`keymap::linux_builtin_keep()`) rides ALONG with the preset — it is
        // NOT flavor-gated, so it's present under emacs too, just not part of
        // `preset` itself (see `linux_builtin_keep()`'s own doc).
        assert_eq!(
            effective.len(),
            preset.len() + crate::keymap::linux_builtin_keep().len(),
            "the live rebuild's keep-list is the whole preset plus the built-in floor"
        );
        for chord in &preset {
            assert!(effective.contains(chord), "{chord:?} missing from the live rebuild's keep-list");
        }
        for chord in crate::keymap::linux_builtin_keep() {
            assert!(effective.iter().any(|c| c == chord), "{chord:?} missing from the live rebuild's keep-list");
        }

        // Enter #2: emacs -> native (round-trips cleanly, doesn't accumulate).
        app.toggle_keymap_flavor();
        assert_eq!(app.config.keymap_flavor(), crate::keymap::KeymapFlavor::Native, "flips back");
        let written2 = mem.read_to_string(std::path::Path::new("/cfg/config.toml")).unwrap();
        assert!(written2.contains("keymap = \"native\""), "the second toggle persists too: {written2:?}");
        // Native flavor: no preset widening, but the built-in floor is still
        // there (it's unconditional, not flavor-gated) — never truly empty.
        assert_eq!(
            app.config.effective_linux_keep().len(),
            crate::keymap::linux_builtin_keep().len(),
            "native flavor: no preset widening, just the built-in floor"
        );
    }

    /// LAW TEST (the "settings toggle rows dispatch live" round): EVERY row
    /// the corpus marks `SettingKind::Toggle` — enumerated straight off
    /// `settings::visible_rows()`, never hand-copied — round-trips through
    /// the REAL live door, `App::setting_toggle(key)` (exactly what
    /// `Effect::SettingToggle` resolves to at the `app/apply.rs` seam, see
    /// `App::apply`'s `Effect::SettingToggle { key } => self.setting_toggle(&key)`
    /// arm): the value readout VISIBLY CHANGES after one toggle, and
    /// round-trips back to its exact starting value after a second — so a
    /// toggle that silently no-ops (the Keymap-row bug: wired in
    /// `settings::toggle_key` and in `settings_accept`, but never driven
    /// through `App::setting_toggle` itself by any prior test — the prior
    /// `settings_keymap_toggle_flips_persists_and_live_reapplies` test called
    /// `app.toggle_keymap_flavor()` directly, skipping the string-keyed
    /// dispatch a live Enter/click actually goes through) fails here instead
    /// of shipping quietly. Companion:
    /// `actions::tests::overlay_drive::every_settings_toggle_row_signals_its_own_setting_toggle_key`
    /// (the pure `apply_core`-level half: Enter on the row signals the RIGHT
    /// key in the first place). Each toggle is undone immediately after
    /// asserting it, so every process-global this sweep touches (page /
    /// typewriter / wysiwyg / inline images / ligatures / spellcheck /
    /// writing nits / outline / menu bar / reduce motion) is back to its
    /// pre-test value by the time the lock releases — no leak into a sibling
    /// test, mirroring the `page::measure()` save/restore convention used
    /// elsewhere in this file.
    #[test]
    fn every_settings_toggle_row_dispatches_live_and_flips_its_value() {
        use crate::fs::InMemoryFs;
        let _g2 = crate::fs::FsGuard::install(Arc::new(InMemoryFs::new()));
        let _g = crate::testlock::serial();

        let cfg = Config { path: PathBuf::from("/cfg/config.toml"), ..Config::empty() };
        let mut app = app_on(None, "/proj", cfg);

        let toggle_rows: Vec<crate::settings::SettingRow> = crate::settings::visible_rows()
            .into_iter()
            .filter(|r| r.kind == crate::settings::SettingKind::Toggle)
            .copied()
            .collect();
        assert_eq!(
            toggle_rows.len(),
            14,
            "the toggle roster changed size — update this sweep deliberately"
        );

        for row in &toggle_rows {
            let key = crate::settings::toggle_key(row.name).expect("a Toggle row always has a key");
            let values0 = crate::settings::SettingsValues::gather(&app.config, &app.root, app.zoom);
            let before = crate::settings::value_for(row, &values0);

            app.setting_toggle(key);
            let values1 = crate::settings::SettingsValues::gather(&app.config, &app.root, app.zoom);
            let after = crate::settings::value_for(row, &values1);
            assert_ne!(
                before, after,
                "row {:?} (key {:?}) did not visibly flip its value readout — the live dispatch is a silent no-op",
                row.name, key
            );

            // Toggle back — restores the global/config AND proves the flip is
            // a clean round-trip, not a one-way ratchet.
            app.setting_toggle(key);
            let values2 = crate::settings::SettingsValues::gather(&app.config, &app.root, app.zoom);
            let restored = crate::settings::value_for(row, &values2);
            assert_eq!(
                restored, before,
                "row {:?} (key {:?}) did not round-trip back to its starting value",
                row.name, key
            );
        }
    }

    /// The corpus GREW to carry the row: "Keymap" is a real, visible settings
    /// row (mirrors `settings::tests::settings_table_names_are_unique`'s own
    /// count law, exercised here through the App's own config/root — a
    /// belt-and-suspenders confirmation that the live overlay build would
    /// actually list it).
    #[test]
    fn settings_corpus_includes_the_keymap_row() {
        assert!(crate::settings::visible_names().contains(&"Keymap".to_string()));
        assert_eq!(crate::settings::toggle_key("Keymap"), Some("keymap"));
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
        // (Some, Some) with the SAME mtime but a DIFFERENT size → a same-tick
        // external edit (equal mtime, changed content) must still be caught by the
        // size guard, or we'd silently overwrite it.
        let cur = App::disk_mtime_of(p).expect("v2 exists");
        let same_tick_other_size = Some(crate::fs::Metadata {
            modified: cur.modified,
            len: cur.len.map(|n| n + 1),
        });
        assert!(App::disk_changed(p, same_tick_other_size));
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
            Some(CLOBBER_NOTICE),
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
            Some(CLOBBER_NOTICE),
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

    // ── SAVE-FEEDBACK round: scratch Save -> note, notice, dirty title marker ──

    #[test]
    fn convert_scratch_and_save_promotes_the_buffer_and_retires_the_stash() {
        use crate::fs::{FileSystem, InMemoryFs};
        let mem = InMemoryFs::new();
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let notes = PathBuf::from("/notes");
        // Stash an OLD scratch content first, exactly like a real prior session
        // would have — the very ghost-copy risk the round's own doc names.
        let stash = crate::fs::scratch_stash_path();
        mem.write(&stash, b"yesterday's dump\n").unwrap();

        let cfg = Config { notes_root: Some(notes.clone()), ..Config::empty() };
        let mut app = app_on(None, "/proj", cfg);
        assert_eq!(app.buffer.text(), "yesterday's dump\n", "restored from the stash first");
        assert!(app.buffer.path().is_none() && !app.buffer.is_note(), "still a true scratch");

        app.convert_scratch_and_save();

        assert!(app.buffer.is_note(), "Cmd-S promoted the scratch buffer into a note");
        let p = app.buffer.path().unwrap().to_path_buf();
        assert!(p.starts_with(&notes), "the note landed under notes_root: {p:?}");
        assert_eq!(mem.read_to_string(&p).unwrap(), "yesterday's dump\n");
        assert_eq!(app.file.as_deref(), Some(p.as_path()), "App.file tracks the new path");
        assert_eq!(app.notice.as_deref(), Some("saved"));
        // THE STASH IS RETIRED: a later bare relaunch must never resurrect a
        // ghost copy of content that is now a real, named file.
        assert!(mem.read_to_string(&stash).is_err(), "the stash file was removed");
    }

    #[test]
    fn convert_scratch_and_save_second_save_is_a_plain_save() {
        use crate::fs::{FileSystem, InMemoryFs};
        let mem = InMemoryFs::new();
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let notes = PathBuf::from("/notes");
        let cfg = Config { notes_root: Some(notes), ..Config::empty() };
        let mut app = app_on(None, "/proj", cfg);
        app.buffer.set_text("first entry\n");
        app.convert_scratch_and_save();
        let named = app.buffer.path().unwrap().to_path_buf();

        // A SECOND explicit save (the buffer is now an ordinary note) must
        // NOT re-run the scratch-conversion machinery — same path, same file,
        // just the updated content. `Buffer::save()` here mirrors exactly
        // what `apply_core`'s `Action::Save` arm does before signalling
        // `Effect::SaveDone`; `finish_manual_save` is its post-save
        // bookkeeping half (see `app::apply`'s `Effect::SaveDone` arm).
        app.buffer.set_text("first entry\nmore\n");
        app.buffer.save().unwrap();
        app.finish_manual_save(true, "saved".to_string());
        assert_eq!(app.buffer.path().unwrap(), named, "no re-homing on the second save");
        assert_eq!(mem.read_to_string(&named).unwrap(), "first entry\nmore\n");
    }

    #[test]
    fn convert_scratch_and_save_unwritable_notes_root_raises_a_calm_notice_never_a_panic() {
        // A `notes_root` that can't be written to (a full disk, a permissions
        // error, …) must surface as the SAME calm notice a failed manual save
        // gets — never a terminal print, never a crash, and the scratch stash
        // is left untouched (nothing succeeded to retire it over).
        struct UnwritableFs;
        impl crate::fs::FileSystem for UnwritableFs {
            fn read_to_string(&self, _p: &std::path::Path) -> std::io::Result<String> {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "unwritable fake"))
            }
            fn read(&self, _p: &std::path::Path) -> std::io::Result<Vec<u8>> {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "unwritable fake"))
            }
            fn write(&self, _p: &std::path::Path, _d: &[u8]) -> std::io::Result<()> {
                Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "notes_root unwritable"))
            }
            fn create_dir_all(&self, _p: &std::path::Path) -> std::io::Result<()> {
                Ok(())
            }
            fn rename(&self, _f: &std::path::Path, _t: &std::path::Path) -> std::io::Result<()> {
                Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "notes_root unwritable"))
            }
            fn exists(&self, _p: &std::path::Path) -> bool {
                false
            }
            fn is_dir(&self, _p: &std::path::Path) -> bool {
                false
            }
            fn read_dir(&self, _p: &std::path::Path) -> std::io::Result<Vec<crate::fs::DirEntry>> {
                Ok(vec![])
            }
            fn metadata(&self, _p: &std::path::Path) -> std::io::Result<crate::fs::Metadata> {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "unwritable fake"))
            }
            fn remove_file(&self, _p: &std::path::Path) -> std::io::Result<()> {
                Ok(())
            }
        }
        let _g = crate::fs::FsGuard::install(Arc::new(UnwritableFs));
        let notes = PathBuf::from("/notes");
        let cfg = Config { notes_root: Some(notes), ..Config::empty() };
        let mut app = app_on(None, "/proj", cfg);
        app.buffer.set_text("won't land\n");

        app.convert_scratch_and_save();

        assert!(
            app.notice.as_deref().is_some_and(|n| n.starts_with("save failed:")),
            "a calm failure notice, not a panic: {:?}",
            app.notice
        );
    }

    // ── NOTES VERBS round: Rename note… / Duplicate note ──

    #[test]
    fn rename_current_file_happy_path_renames_disk_buffer_and_history() {
        use crate::fs::{FileSystem, InMemoryFs};
        let old = PathBuf::from("/notes/old.md");
        let mem = InMemoryFs::new().with_file(&old, "hi\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        // A prior snapshot exists under the OLD path — the ONE-OWNER rename must
        // carry it over so the timeline survives the rename.
        crate::history::record(&old, "hi\n", &Config::empty());
        assert!(!crate::history::list(&old).is_empty(), "arranged: a snapshot exists");

        let mut app = app_on(Some(old.clone()), "/notes", Config::empty());
        assert_eq!(app.buffer.path(), Some(old.as_path()));

        app.rename_current_file("new.md");

        let new = PathBuf::from("/notes/new.md");
        assert_eq!(app.buffer.path(), Some(new.as_path()), "buffer follows the rename");
        assert_eq!(app.file.as_deref(), Some(new.as_path()), "App.file follows the rename");
        assert_eq!(mem.read_to_string(&new).unwrap(), "hi\n", "content moved");
        assert!(mem.read_to_string(&old).is_err(), "the old path is gone");
        assert_eq!(app.notice.as_deref(), Some("renamed to new.md"));
        // THE ONE-OWNER LAW: the history log followed too.
        assert!(!crate::history::list(&new).is_empty(), "history followed to the new path");
        assert!(crate::history::list(&old).is_empty(), "nothing stranded under the old path");
    }

    #[test]
    fn rename_current_file_refuses_to_clobber_an_existing_name() {
        use crate::fs::{FileSystem, InMemoryFs};
        let old = PathBuf::from("/notes/old.md");
        let taken = PathBuf::from("/notes/taken.md");
        let mem = InMemoryFs::new().with_file(&old, "old body\n").with_file(&taken, "taken body\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(old.clone()), "/notes", Config::empty());

        app.rename_current_file("taken.md");

        assert_eq!(app.buffer.path(), Some(old.as_path()), "buffer stays put — refused, not clobbered");
        assert_eq!(mem.read_to_string(&old).unwrap(), "old body\n", "old untouched");
        assert_eq!(mem.read_to_string(&taken).unwrap(), "taken body\n", "never overwritten");
        assert!(
            app.notice.as_deref().is_some_and(|n| n.contains("already a file named")),
            "a calm refusal notice: {:?}",
            app.notice
        );
    }

    #[test]
    fn rename_current_file_refuses_a_git_managed_file() {
        use crate::fs::{FileSystem, InMemoryFs};
        let old = PathBuf::from("/proj/tracked.md");
        let mem = InMemoryFs::new().with_file(&old, "body\n").with_dir("/proj/.git");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(old.clone()), "/proj", Config::empty());

        app.rename_current_file("renamed.md");

        assert_eq!(app.buffer.path(), Some(old.as_path()), "a git-managed file never renames here");
        assert!(mem.exists(&old), "old path untouched");
        assert!(!mem.exists(&PathBuf::from("/proj/renamed.md")), "no new file created");
        assert!(
            app.notice.as_deref().is_some_and(|n| n.contains("git already tracks")),
            "a calm git-managed refusal notice: {:?}",
            app.notice
        );
    }

    #[test]
    fn rename_current_file_unchanged_or_blank_name_is_a_quiet_no_op() {
        use crate::fs::InMemoryFs;
        let old = PathBuf::from("/notes/old.md");
        let mem = InMemoryFs::new().with_file(&old, "hi\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(old.clone()), "/notes", Config::empty());

        app.rename_current_file("old.md");
        assert_eq!(app.buffer.path(), Some(old.as_path()), "unchanged name: no-op");
        assert!(app.notice.is_none(), "no notice for a no-op");

        app.rename_current_file("   ");
        assert_eq!(app.buffer.path(), Some(old.as_path()), "blank name: no-op");
        assert!(app.notice.is_none(), "no notice for a no-op");
    }

    #[test]
    fn duplicate_current_file_dedups_the_name_and_starts_a_fresh_history_timeline() {
        use crate::fs::{FileSystem, InMemoryFs};
        let old = PathBuf::from("/notes/old.md");
        // A prior "old-2.md" already exists, so the dedup must land on "old-3.md".
        let taken2 = PathBuf::from("/notes/old-2.md");
        let mem =
            InMemoryFs::new().with_file(&old, "on disk\n").with_file(&taken2, "someone else's\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        // The old file has its own history timeline.
        crate::history::record(&old, "on disk\n", &Config::empty());
        assert!(!crate::history::list(&old).is_empty(), "arranged: old has history");

        let mut app = app_on(Some(old.clone()), "/notes", Config::empty());
        // Simulate an UNSAVED edit: the duplicate must carry the LIVE buffer
        // content, not necessarily what's on disk.
        app.buffer.set_text("live edit, not yet flushed\n");

        app.duplicate_current_file();

        let dup = PathBuf::from("/notes/old-3.md");
        assert_eq!(app.buffer.path(), Some(dup.as_path()), "switched to the deduped sibling");
        assert_eq!(app.file.as_deref(), Some(dup.as_path()));
        assert_eq!(
            mem.read_to_string(&dup).unwrap(),
            "live edit, not yet flushed\n",
            "the copy captures the buffer's LIVE content"
        );
        assert!(mem.exists(&old), "the original file is untouched, still present");
        assert!(mem.exists(&taken2), "the pre-existing -2 sibling is never clobbered");
        // FRESH HISTORY: the duplicate is a brand-new file, so its own timeline
        // starts empty, even though the SOURCE had history.
        assert!(crate::history::list(&dup).is_empty(), "the copy starts a fresh history timeline");
        // The ORIGINAL buffer was PARKED (backgrounded), never discarded — its
        // pending edit is still flushed to disk (autosave_flush runs before the
        // dedup scan) and its live state survives in the registry.
        let key = crate::buffers::BufferKey::path(&old);
        assert!(app.buffer_registry.contains(&key), "the original was parked, not dropped");
    }

    #[test]
    fn duplicate_current_file_on_a_pathless_buffer_is_a_quiet_no_op() {
        let mut app = app_on(None, "/proj", Config::empty());
        assert!(app.buffer.path().is_none());
        app.duplicate_current_file();
        assert!(app.buffer.path().is_none(), "nothing to duplicate yet");
        assert!(app.notice.is_none());
    }

    #[test]
    fn finish_manual_save_ok_is_silent_failure_notices_the_error() {
        // SAVE-UX round: a SUCCESSFUL manual save raises NO bottom-center notice
        // (autosave is already silent; a lone non-fading "saved" is just noise).
        // A FAILURE still surfaces its error — errors must never go silent.
        use crate::fs::InMemoryFs;
        let _l = crate::testlock::serial();
        let p = PathBuf::from("/notes/draft.md");
        let mem = InMemoryFs::new().with_file(&p, "v1\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(p.clone()), "/notes", Config::empty());

        app.finish_manual_save(true, "saved".to_string());
        assert_eq!(app.notice.as_deref(), Some("saved"));
        assert_eq!(app.notice_kind, NoticeKind::Toast);
        assert!(app.notice_expires_at.is_none(), "a headless test never arms a live timer");

        app.finish_manual_save(false, "save failed: disk full".to_string());
        assert_eq!(app.notice.as_deref(), Some("save failed: disk full"));
    }

    #[test]
    fn finish_manual_save_clears_a_notes_dirty_marker_immediately() {
        // BUG LOCK-DOWN: `is_document_dirty` reads `autosave_saved_version` for a
        // NOTE, but `finish_manual_save` used to stamp only `doc_saved_version`
        // — so ⌘S on a note left it reading dirty (the title `•` + native
        // titlebar dot lingering) until the note's ~400ms debounced autosave
        // redundantly rewrote and finally stamped the field.
        use crate::fs::InMemoryFs;
        let _l = crate::testlock::serial();
        let notes = PathBuf::from("/notes");
        let mem = InMemoryFs::new().with_dir(&notes);
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(None, "/notes", Config::empty());

        // Make the active buffer a NOTE with content, then write it to disk the
        // way `apply_core`'s `Action::Save` arm does before signalling SaveDone.
        app.buffer.start_note(notes.clone());
        app.buffer.set_text("note body\n");
        app.buffer.save().unwrap();
        assert!(app.buffer.is_note() && app.buffer.path().is_some(), "arranged: a saved note");
        // Pre-bookkeeping the note reads DIRTY: `autosave_saved_version` is still
        // stale (None) against the edited version.
        assert!(app.is_document_dirty(), "arranged: the note reads dirty pre-bookkeeping");

        app.finish_manual_save(true, "saved".to_string());

        assert!(!app.is_document_dirty(), "a note is clean IMMEDIATELY after ⌘S, not ~400ms later");
        assert!(app.autosave_dirty_at.is_none(), "the redundant ~400ms note rewrite is suppressed");
    }

    #[test]
    fn finish_manual_save_clears_a_regular_files_dirty_marker_immediately() {
        // REGRESSION GUARD: a path-backed file reads `doc_saved_version` in
        // `is_document_dirty` — it was always fine, and must stay fine.
        use crate::fs::InMemoryFs;
        let _l = crate::testlock::serial();
        let p = PathBuf::from("/proj/doc.md");
        let mem = InMemoryFs::new().with_file(&p, "v1\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(p.clone()), "/proj", Config::empty());

        app.buffer.set_text("edited body\n");
        app.buffer.save().unwrap();
        assert!(!app.buffer.is_note() && app.buffer.path().is_some(), "arranged: a saved file");
        assert!(app.is_document_dirty(), "arranged: the file reads dirty pre-bookkeeping");

        app.finish_manual_save(true, "saved".to_string());

        assert!(!app.is_document_dirty(), "a regular file is clean immediately after ⌘S");
    }

    // ── SAVE-FEEDBACK round: the ambient dirty title marker ──

    #[test]
    fn sync_view_retitles_only_on_an_actual_dirty_flip() {
        let mut app = app_on(None, "/proj", Config::empty());
        assert!(!app.title_dirty, "a fresh scratch buffer starts clean");
        // No gpu/window in a hermetic App: `sync_view` bails before the title
        // comparison (its own gpu-present gate) — this proves the flip-tracking
        // logic itself is reachable + correct via `is_document_dirty` directly,
        // mirroring `update_title_uses_the_same_pure_window_title`'s own
        // "no live window, still exercised" shape.
        assert!(!app.is_document_dirty(), "just-loaded content starts saved");
        app.buffer.set_text("edited\n");
        assert!(app.is_document_dirty(), "an edit past the saved version is dirty");
    }

    #[test]
    fn is_document_dirty_clears_on_autosave_not_just_manual_save() {
        // The definition this round settled on for the title's dirty marker:
        // "unsaved" by the SAME version-vs-saved-version bookkeeping the
        // autosave engine tracks — so an AUTOSAVED (not manually Cmd-S'd)
        // document reads as clean too, never stuck showing the edited marker
        // on content that's already safely on disk.
        use crate::fs::{FileSystem, InMemoryFs};
        let p = PathBuf::from("/notes/draft.md");
        let mem = InMemoryFs::new().with_file(&p, "v1\n");
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
        assert!(!app.is_document_dirty());
        app.buffer.set_text("v2\n");
        assert!(app.is_document_dirty(), "an unsaved edit reads dirty");
        app.autosave_flush(); // NOT a manual save — the background engine
        assert_eq!(mem.read_to_string(&p).unwrap(), "v2\n");
        assert!(!app.is_document_dirty(), "autosave clears the dirty marker too");
    }

    #[test]
    fn scratch_stash_invalid_utf8_preserves_a_corrupt_sibling_then_starts_a_blank_scratch() {
        // DATA-SAFETY HARDENING: the scratch stash IS a manuscript, so a
        // stash file that's PRESENT but fails to decode as UTF-8 text (real
        // disk corruption, never a bug write_atomic itself can produce) must
        // never be silently discarded — a `.corrupt-*` sibling preserves the
        // raw bytes BEFORE `App::new` falls back to a blank scratch buffer.
        use crate::fs::{FileSystem, InMemoryFs};
        let mem = InMemoryFs::new();
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        let stash = crate::fs::scratch_stash_path();
        // Invalid UTF-8: a lone continuation byte can never decode.
        mem.write(&stash, &[0x2E, 0x62, 0xFF, 0xFE, 0x0A]).unwrap();

        let app = app_on(None, "/proj", Config::empty());
        assert_eq!(app.buffer.text(), "", "an undecodable stash falls back to a blank scratch");
        assert!(app.buffer.path().is_none());

        let dir = stash.parent().unwrap();
        let names: Vec<String> = mem.read_dir(dir).unwrap().into_iter().map(|e| e.name).collect();
        let stash_name = stash.file_name().unwrap().to_string_lossy().into_owned();
        let backup_prefix = format!("{stash_name}.corrupt-");
        let backups: Vec<&String> = names.iter().filter(|n| n.starts_with(&backup_prefix)).collect();
        assert_eq!(backups.len(), 1, "exactly one corrupt sibling preserved: {names:?}");
        let backup_bytes = mem.read(&dir.join(backups[0])).unwrap();
        assert_eq!(
            backup_bytes,
            vec![0x2E, 0x62, 0xFF, 0xFE, 0x0A],
            "the sibling holds the ORIGINAL undecodable bytes verbatim"
        );
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
            Some(CLOBBER_NOTICE),
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
        app.overlay = Some(crate::overlay::OverlayState::new_history(rows, None, None));
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
        app.overlay = Some(crate::overlay::OverlayState::new_history(rows, None, None));
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

    // ── PROSE/CODE PAGE-WIDTH SPLIT (App-level buffer-switch resync) ────────

    #[test]
    fn load_path_switch_reapplies_default_measure_per_kind() {
        // WIRING (1): a buffer SWITCH re-applies the right measure through the
        // existing `set_measure` seam (`App::sync_page_measure`, called from
        // `load_path`). A.md (prose) -> B.rs (code) -> back to A.md, with NO
        // config override, must land on each class's own BUILT-IN default.
        use crate::fs::InMemoryFs;
        let a = PathBuf::from("/proj/a.md");
        let b = PathBuf::from("/proj/b.rs");
        let mem = InMemoryFs::new().with_file(&a, "# hello\n").with_file(&b, "fn main() {}\n");
        let _g2 = crate::fs::FsGuard::install(Arc::new(mem));
        // LOCK ORDER: fs seam first, page lock LAST (see page::test_lock()'s doc)
        // — the reverse order deadlocks against every fs-holding test whose
        // load_path transitively writes the measure.
        let _g = crate::testlock::serial();
        let measure0 = crate::page::measure();
        let mut app = app_on(Some(a.clone()), "/proj", Config::empty());

        // Deliberately wrong, so the switches below can't coincidentally "already"
        // hold the right value.
        crate::page::set_measure(12345);
        app.load_path(b.clone());
        assert_eq!(
            crate::page::measure(),
            crate::page::DEFAULT_MEASURE_CODE,
            "switching to B.rs (code) applies the code default"
        );
        app.load_path(a.clone());
        assert_eq!(
            crate::page::measure(),
            crate::page::DEFAULT_MEASURE,
            "switching back to A.md (prose) applies the prose default"
        );

        crate::page::set_measure(measure0);
    }

    #[test]
    fn load_path_switch_reapplies_custom_measure_overrides() {
        // The SAME A.md/B.rs round trip, but with configured overrides for BOTH
        // classes — the switch must read `Config::measure_for`, not just the
        // built-in defaults.
        use crate::fs::InMemoryFs;
        let a = PathBuf::from("/proj/a.md");
        let b = PathBuf::from("/proj/b.rs");
        let mem = InMemoryFs::new().with_file(&a, "hello\n").with_file(&b, "fn main() {}\n");
        let _g2 = crate::fs::FsGuard::install(Arc::new(mem));
        let _g = crate::testlock::serial(); // fs first, page LAST (see page::test_lock())
        let measure0 = crate::page::measure();
        let cfg = Config { page_width_prose: Some(55), page_width_code: Some(120), ..Config::empty() };
        let mut app = app_on(Some(a.clone()), "/proj", cfg);

        crate::page::set_measure(1);
        app.load_path(b.clone());
        assert_eq!(crate::page::measure(), 120, "B.rs picks up the configured code override");
        app.load_path(a.clone());
        assert_eq!(crate::page::measure(), 55, "back to A.md picks up the configured prose override");

        crate::page::set_measure(measure0);
    }

    #[test]
    fn new_note_always_reapplies_the_prose_measure() {
        // A fresh quick note is always markdown (PROSE), regardless of what kind
        // of buffer was active before it — `new_note` calls the same
        // `sync_page_measure` resync `load_path` does.
        use crate::fs::InMemoryFs;
        let b = PathBuf::from("/proj/b.rs");
        let mem = InMemoryFs::new().with_file(&b, "fn main() {}\n");
        let _g2 = crate::fs::FsGuard::install(Arc::new(mem));
        let _g = crate::testlock::serial(); // fs first, page LAST (see page::test_lock())
        let measure0 = crate::page::measure();
        let mut app = app_on(Some(b.clone()), "/proj", Config::empty());

        crate::page::set_measure(crate::page::DEFAULT_MEASURE_CODE);
        app.new_note();
        assert_eq!(
            crate::page::measure(),
            crate::page::DEFAULT_MEASURE,
            "a new note is prose, so it gets the prose default even leaving a code buffer"
        );

        crate::page::set_measure(measure0);
    }

    #[test]
    fn persist_page_width_writes_the_key_matching_the_active_buffer_kind() {
        // The STICKY WRITE half (drag-resize / C-x { / C-x }): `persist_page_width`
        // must target `page_width_prose` while a prose buffer is active and
        // `page_width_code` while a code buffer is active — never the other key.
        use crate::fs::InMemoryFs;
        let cfg_path = PathBuf::from("/home/.config/awl/config.toml");
        let a = PathBuf::from("/proj/a.md");
        let b = PathBuf::from("/proj/b.rs");
        let mem = InMemoryFs::new().with_file(&a, "hello\n").with_file(&b, "fn main() {}\n");
        let _g2 = crate::fs::FsGuard::install(Arc::new(mem));
        let _g = crate::testlock::serial(); // fs first, page LAST (see page::test_lock())
        let measure0 = crate::page::measure();
        let cfg = Config { path: cfg_path.clone(), ..Config::empty() };
        let mut app = app_on(Some(a.clone()), "/proj", cfg);

        crate::page::set_measure(55);
        app.persist_page_width();
        let reloaded = Config::load(cfg_path.clone());
        assert_eq!(reloaded.page_width_prose, Some(55), "a PROSE buffer persists to page_width_prose");
        assert_eq!(reloaded.page_width_code, None, "the code key is untouched");

        app.load_path(b.clone());
        crate::page::set_measure(130);
        app.persist_page_width();
        let reloaded2 = Config::load(cfg_path.clone());
        assert_eq!(reloaded2.page_width_code, Some(130), "a CODE buffer persists to page_width_code");
        assert_eq!(reloaded2.page_width_prose, Some(55), "the prose key from before survives untouched");

        crate::page::set_measure(measure0);
    }

    #[test]
    fn persist_page_reset_clears_the_key_matching_the_active_buffer_kind() {
        // The RESET half: `persist_page_reset` must clear ONLY the override
        // matching the active buffer's kind, leaving the other class's override
        // (and every other pref) untouched.
        use crate::fs::InMemoryFs;
        let cfg_path = PathBuf::from("/home/.config/awl/config.toml");
        let a = PathBuf::from("/proj/a.md");
        let b = PathBuf::from("/proj/b.rs");
        let mem = InMemoryFs::new().with_file(&a, "hello\n").with_file(&b, "fn main() {}\n");
        let _g2 = crate::fs::FsGuard::install(Arc::new(mem));
        let _g = crate::testlock::serial(); // fs first, page LAST (see page::test_lock())
        let measure0 = crate::page::measure();
        Config::write_pref(&cfg_path, "page_width_prose", "55").unwrap();
        Config::write_pref(&cfg_path, "page_width_code", "130").unwrap();
        let cfg = Config::load(cfg_path.clone());
        let mut app = app_on(Some(b.clone()), "/proj", cfg); // start on the CODE file

        app.persist_page_reset();
        let reloaded = Config::load(cfg_path.clone());
        assert_eq!(reloaded.page_width_code, None, "the code override is cleared");
        assert_eq!(reloaded.page_width_prose, Some(55), "the prose override survives untouched");

        app.load_path(a.clone());
        app.persist_page_reset();
        let reloaded2 = Config::load(cfg_path.clone());
        assert_eq!(reloaded2.page_width_prose, None, "the prose override is now also cleared");

        crate::page::set_measure(measure0);
    }

    #[test]
    fn setting_value_commit_clamps_persists_and_applies_measure_and_zoom() {
        // SETTINGS v2 inline VALUE edit (App half): parse + clamp the typed value,
        // apply it LIVE (page::measure / zoom), and persist the NAMED key.
        use crate::fs::InMemoryFs;
        let cfg_path = PathBuf::from("/home/.config/awl/config.toml");
        let a = PathBuf::from("/proj/a.md"); // a PROSE (.md) buffer
        let mem = InMemoryFs::new().with_file(&a, "hello\n");
        let _g2 = crate::fs::FsGuard::install(Arc::new(mem));
        let _g = crate::testlock::serial(); // fs first, page LAST (see page::test_lock())
        let measure0 = crate::page::measure();
        let cfg = Config { path: cfg_path.clone(), ..Config::empty() };
        let mut app = app_on(Some(a.clone()), "/proj", cfg);

        // In-range prose width: applied LIVE (the active buffer is prose) + persisted.
        app.setting_value_commit("page_width_prose", "45");
        assert_eq!(crate::page::measure(), 45, "a prose-width edit re-wraps live");
        assert_eq!(Config::load(cfg_path.clone()).page_width_prose, Some(45));

        // Out of range: CLAMPED to PAGE_WIDTH_MAX, both live + on disk.
        app.setting_value_commit("page_width_prose", "5000");
        assert_eq!(crate::page::measure(), crate::settings::PAGE_WIDTH_MAX);
        assert_eq!(
            Config::load(cfg_path.clone()).page_width_prose,
            Some(crate::settings::PAGE_WIDTH_MAX)
        );

        // Unparseable: a calm no-op (measure + config unchanged).
        app.setting_value_commit("page_width_prose", "oops");
        assert_eq!(crate::page::measure(), crate::settings::PAGE_WIDTH_MAX);

        // Editing the CODE width while a PROSE buffer is active persists to its own key
        // but does NOT change the visible measure (sync_page_measure reads the active
        // class), so the prose/code split never bleeds.
        app.setting_value_commit("page_width_code", "88");
        assert_eq!(
            crate::page::measure(),
            crate::settings::PAGE_WIDTH_MAX,
            "the code-width edit leaves the prose measure alone"
        );
        assert_eq!(Config::load(cfg_path.clone()).page_width_code, Some(88));

        // ZOOM: the percent readout form parses + clamps through the shared set_zoom
        // owner + persists.
        app.setting_value_commit("zoom", "150%");
        assert!((app.zoom - 1.5).abs() < 1e-4, "150% -> factor 1.5");
        assert_eq!(Config::load(cfg_path.clone()).zoom, Some(1.5));

        crate::page::set_measure(measure0);
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
        let _fs = crate::testlock::serial();
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
            .suggest_at(&buffer.text(), line, col, buffer.syntax_lang())
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
        assert!(sc.suggest_at(&buffer.text(), l, c, buffer.syntax_lang()).is_none(), "correct word: no summon");
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
    // `rowlayout.rs`'s / `theme/`'s no-wildcard enumerations — a structural
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
            // 3 store tests (2 recent-projects + 1 recent-files), each inside its
            // own `fs::with_fs(fake, ..)` closure seeded with an `InMemoryFs` — they
            // exist specifically to prove what `App::switch_project` / `App::load_path`
            // / `App::new` write to and read back from the recent-projects /
            // recent-files stores, so they need to CONTROL + INSPECT the injected fs
            // (which `new_hermetic`'s private internal fs hides), never real disk.
            // Same treatment as `app/session.rs` above. Plus 3 NO-PATH-PASTE-SAVES-
            // FIRST tests (`ensure_note_named_before_paste_*`), each also inside its
            // own `fs::with_fs(fake, ..)` closure with an `InMemoryFs` handle kept by
            // the test — they exist specifically to prove what
            // `App::ensure_note_named_before_paste` writes to disk (the promoted
            // note's derived path + its saved bytes), so they need the same
            // CONTROL + INSPECT access `new_hermetic` hides. Same treatment. Plus 1
            // CJK-priority persist test
            // (`persist_cjk_priority_writes_the_whole_ordered_ladder_to_config`),
            // inside its own `fs::with_fs(fake, ..)` closure with an `InMemoryFs`
            // handle — proves what `App::persist_cjk_priority` writes to
            // `config.path` on disk, same CONTROL + INSPECT need.
            ("app/files.rs", 7),
            // 9 LIFETIME STATS + USAGE LEDGER + DISCOVERABILITY tests, each inside its own
            // `fs::with_fs(fake, ..)` closure seeded with an `InMemoryFs` — they exist
            // specifically to prove what the tracking hooks / the ledger's
            // `ledger_note_dispatch` + `stats_flush` write to and read back from
            // `stats.toml`, so they need to CONTROL + INSPECT the injected fs (which
            // `new_hermetic`'s private internal fs hides). Same treatment as
            // `app/session.rs` / `app/files.rs` above. (The 3 added by the ledger:
            // door-attribution round-trip, graduation-candidate ranking, kill-switch;
            // the 2 added by the discoverability round: peek/footer ranking from a fake
            // ledger, and the fresh-ledger-empty case.)
            ("app/stats.rs", 9),
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
