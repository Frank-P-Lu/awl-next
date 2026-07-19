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

#[cfg(test)]
mod crossing;

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

/// Quiet period after the last `Moved` tick before the MOVE stream is considered
/// settled: the lamp resumes, ONE settle redraw fires, and presents go back to
/// async (`App::finish_move_settle`). DELIBERATELY LONGER than
/// `RESIZE_SYNC_SETTLE`: a resize stream's ticks stop exactly when the drag
/// stops, but a MOVE stream's quiet gaps include mid-drag stationary HOLDS with
/// the title bar still grabbed — at the old 150ms, a hesitation un-paused the
/// lamp mid-grab, and the resumed ambient presents raced the window-server's
/// move transaction the instant the drag continued (the "flash while moving is
/// kinda back" report, 2026-07-15 — the same compositor-race class the
/// resize-stretch fix closed). One second outlasts an ordinary hesitation; the
/// lamp drifts so slowly (~67 s loop) that the longer hold is imperceptible.
/// TASTE TUNABLE — flagged for live review.
const MOVE_SETTLE: Duration = Duration::from_millis(1000);

/// Quiet period a THEME-PREVIEW lava-boundary crossing keeps the present-
/// transaction sync armed (debounce; macOS-only effect). When a preview step
/// swaps a ticking lava world for a static non-lava one (or back), the ~10 fps
/// ambient present cadence starts/stops underfoot; arming the transaction sync
/// makes the crossing frame (and one settle follow-up) JOIN the compositor's
/// transaction instead of racing it, so the swapchain can't strand a stale
/// drawable — the "writing surface vanishes" report. Each further crossing
/// re-stamps the deadline (`retint_theme_preview`), so a rapid arrow burst
/// through the boundary keeps it armed and settles once you rest — the same
/// single-`WaitUntil` shape as `RESIZE_SYNC_SETTLE` / the theme-font debounce.
/// Sized like `RESIZE_SYNC_SETTLE`: long enough to bracket the crossing frame +
/// its follow-up, short enough that the sync cost is paid only around a crossing.
const CROSSING_SYNC_SETTLE: Duration = Duration::from_millis(150);

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

/// A pending ZOOM ANCHOR: the document char + the screen y that char should hold, so
/// the next `sync_view` (which reshapes to the just-changed zoom) keeps that point
/// fixed on screen instead of anchoring at the viewport top. Captured at the OLD zoom
/// BEFORE the deferred reshape (both zoom paths arm it — the wheel with the POINTER's
/// char + y, the keyboard with the CARET's, or the viewport-centre char when the caret
/// is off-screen), consumed once by `sync_view` via [`TextPipeline::zoom_anchor_scroll`]
/// (the one owner of the anchored-scroll math). Live-only: the headless capture never
/// builds an `App`, so its single-frame scroll stays cursor-follow (unchanged).
#[derive(Clone, Copy, Debug)]
struct ZoomAnchor {
    line: usize,
    col: usize,
    screen_y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GpuLifecycle { AwaitingWindow, Active { oom_skips: u8 }, Suspended, Rebuilding }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GpuFaultAction { RetryOneFrame, Rebuild, NoticeOnly }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GpuSkipAction {
    WaitForWake,
    RetryAfter(Duration),
    RetryWithNoticeAfter(Duration, &'static str),
    HoldWithNotice(&'static str),
}
const GPU_SURFACE_RETRY: Duration = Duration::from_millis(16);
fn gpu_fault_action(lifecycle: GpuLifecycle, kind: gpu::GpuFaultKind) -> GpuFaultAction {
    match kind {
        gpu::GpuFaultKind::OutOfMemory if matches!(lifecycle, GpuLifecycle::Active { oom_skips: 0 }) => GpuFaultAction::RetryOneFrame,
        gpu::GpuFaultKind::Validation => GpuFaultAction::NoticeOnly,
        _ => GpuFaultAction::Rebuild,
    }
}
fn gpu_skip_action(skip: gpu::GpuFrameSkip, timeout_streak: u8) -> GpuSkipAction {
    match skip {
        gpu::GpuFrameSkip::Occluded => GpuSkipAction::WaitForWake,
        gpu::GpuFrameSkip::Timeout => GpuSkipAction::RetryAfter(Duration::from_millis(
            16_u64 << timeout_streak.min(5),
        )),
        gpu::GpuFrameSkip::SurfaceReconfigured => GpuSkipAction::RetryAfter(GPU_SURFACE_RETRY),
        gpu::GpuFrameSkip::SurfaceRecreated => {
            GpuSkipAction::RetryWithNoticeAfter(GPU_SURFACE_RETRY, "graphics surface recovered")
        }
        gpu::GpuFrameSkip::PrepareFailed => {
            GpuSkipAction::HoldWithNotice("graphics skipped one frame — editing is safe")
        }
    }
}
fn keep_gpu_loop_hot(animating: bool, frame_presented: bool) -> bool {
    animating && frame_presented
}
/// Map a live GPU skip cause onto the soak probe's [`crate::soak_gpu::SkipKind`]
/// so each cause is counted SEPARATELY (the collapse into one `skipped` total is
/// what hid the zero-drawable occlusion investigation).
#[cfg(not(target_arch = "wasm32"))]
fn soak_skip_kind(skip: gpu::GpuFrameSkip) -> crate::soak_gpu::SkipKind {
    match skip {
        gpu::GpuFrameSkip::Timeout => crate::soak_gpu::SkipKind::Timeout,
        gpu::GpuFrameSkip::Occluded => crate::soak_gpu::SkipKind::Occluded,
        gpu::GpuFrameSkip::SurfaceReconfigured => crate::soak_gpu::SkipKind::SurfaceReconfigured,
        gpu::GpuFrameSkip::SurfaceRecreated => crate::soak_gpu::SkipKind::SurfaceRecreated,
        gpu::GpuFrameSkip::PrepareFailed => crate::soak_gpu::SkipKind::PrepareFailed,
    }
}
/// `WindowEvent::Occluded`: whether an occlusion CHANGE should schedule a
/// repaint. The GPU skip path parks `Occluded → WaitForWake` with no retry
/// timer, so an un-occlusion (`false`) is the wake that must request a redraw;
/// becoming occluded (`true`) needs nothing — the next acquire returns
/// `Occluded` and re-parks the loop. Pure so it is unit-testable off-window.
fn occluded_change_wants_redraw(occluded: bool) -> bool {
    !occluded
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
    #[cfg(not(target_arch = "wasm32"))]
    backend_name: String,
    faults: gpu::GpuFaultInbox,
    #[cfg(not(target_arch = "wasm32"))]
    inject_surface_loss: bool,
    /// LIVE PROBE frame mirror (`--live-script` only, else forever `None`): a
    /// persistent texture every PRESENTED frame is blitted into just before
    /// `present()`, so a probe `shot` can read back what the compositor was
    /// LAST HANDED — without forcing a redraw that would repaint over exactly
    /// the stale-frame / missed-redraw bug classes the probe hunts. See
    /// `Gpu::mirror_presented_frame` + `crate::probe`'s module doc.
    #[cfg(not(target_arch = "wasm32"))]
    probe_mirror: Option<wgpu::Texture>,
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
/// The `about_to_wait` scheduling body: every debounce / settle deadline, the
/// ambient (lava/stars) tick, event-toast expiry, GPU acquire retries + soak
/// drive — one `WaitUntil` each, lifted out of the trait method (a thin
/// delegate now) so the file's #1 collision seam has its own home.
mod schedule;
/// The virtual-clock frame loop's headless control-flow sink, re-exported so the
/// capture harness (`crate::capture::frames`) + the scheduling law can step the
/// real `about_to_wait_impl` body off-window. `Scheduler` stays crate-internal too
/// (the trait the body is generic over; `ActiveEventLoop` is the live impl).
#[cfg(any(test, not(target_arch = "wasm32")))]
pub(crate) use schedule::RecordingScheduler;
/// The apply bridge: resolve an `Action` + live-only side effects.
mod apply;
/// The single-instance DAEMON's App-side wiring (native only): react to a
/// posted `DaemonEvent`, finish a buffer for C-x #, tear down on quit.
mod daemon;
/// The LIVE PROBE HARNESS's App-side wiring (native only): scripted chords
/// through the real dispatch tail + compositor-side window shots. See
/// `crate::probe`'s module doc.
mod probe;
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
/// WRITING STREAKS' App-side wiring (native only): the per-buffer word-delta
/// sampling on the autosave flush triggers, the local-calendar-day read, and the
/// live year-view push. `crate::streaks` owns the pure store + calendar/intensity
/// arithmetic + the (de)serializer.
mod streaks;

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
    recovery_window: Option<Arc<Window>>,
    gpu_lifecycle: GpuLifecycle,
    gpu_retry_at: Option<Instant>,
    gpu_timeout_streak: u8,
    #[cfg(not(target_arch = "wasm32"))]
    soak: Option<crate::soak_gpu::Controller>,
    #[cfg(not(target_arch = "wasm32"))]
    soak_recovery_pending: Option<crate::soak_gpu::FaultKind>,
    #[cfg(not(target_arch = "wasm32"))]
    soak_passed: Option<bool>,
    /// LIVE PROBE (`--live-script`): the one-shot "first frame is up" signal
    /// the driver thread blocks on before feeding scripted input — sent (and
    /// the sender dropped) from `on_gpu_ready`, alongside a `focus_window()`
    /// so the probe window is frontmost/unoccluded (the wgpu macOS occlusion
    /// tripwire: a non-visible window presents nothing). `None` outside a
    /// probe run — zero cost on a normal launch.
    #[cfg(not(target_arch = "wasm32"))]
    probe_ready: Option<std::sync::mpsc::Sender<()>>,
    /// WASM-only handoff slot for the ASYNC GPU init. The browser main thread can't
    /// block, so `Gpu::new` runs on a `spawn_local` future that parks its result
    /// here; `window_event` moves it into `gpu` on the first frame. `Rc<RefCell>`
    /// because the future and the App share it on the (single) wasm main thread.
    #[cfg(target_arch = "wasm32")]
    gpu_pending: std::rc::Rc<std::cell::RefCell<Option<Result<Gpu, String>>>>,
    /// Timestamp of the previous animated frame, for real-time spring dt. `None`
    /// while idle; set on the first animating redraw and cleared once settled.
    last_frame: Option<Instant>,
    /// THE ONE TIME OWNER for scheduling + animation (see `crate::clock::Clock`).
    /// Every debounce/settle deadline, the frame `dt`, the ambient tick, toast
    /// expiry, GPU-retry timing, and the App's sense-of-time stamps read
    /// `self.clock.now()` — never the free `Instant::now()`. `RealClock` on the
    /// shipped app (a pure pass-through, so captures stay byte-identical);
    /// `Box<dyn>` so a deterministic clock can slot in behind the same field.
    /// The `app::clock_law` grep-test fences the module against a raw read.
    clock: Box<dyn crate::clock::Clock>,
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
    /// THE FORMAT POPOVER (`crate::popover`): `true` while the reveal-on-select
    /// format toolbar is summoned. Set ONLY by a MOUSE selection gesture (a
    /// drag-release with a non-empty selection / a double-click word-select) in a
    /// markdown buffer — a KEYBOARD selection never summons it (the spec's
    /// mouse-only rule). Cleared when the selection collapses, on Esc / a keyboard
    /// selection extend / a click off its buttons / a buffer swap / a caret move
    /// with no selection. The render MODEL (which buttons lit, the `H` level) is
    /// recomputed each `sync_view` from the live selection (`actions::popover::plan`),
    /// so it stays open + reflective across format applies. LIVE-ONLY: the headless
    /// capture force-summons via the `AWL_POPOVER` probe instead of this flag.
    popover_open: bool,
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
    /// DIFF-AS-PREVIEW cache: the `(id, transcript)` of the WRITER'S DIFF the open
    /// History overlay's highlighted row resolves to — the marked-up-manuscript
    /// comparison of the CURRENT buffer against that version, built once per id by
    /// the one owner [`crate::history::diff_preview`] so an arrow/hover/wheel burst
    /// over the rows never re-diffs per sync. The preview itself is DERIVED at
    /// ViewState-build time (`sync_view` overrides the pushed text) — the Buffer,
    /// its version, and its undo history are NEVER touched. Dropped the moment the
    /// overlay closes (Esc = back to now exactly). `None` = no preview. (The old
    /// read-only Compare TAKEOVER that used to own the transcript is RETIRED —
    /// this preview override seam is the one surviving diff surface.)
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
    /// Pending ZOOM ANCHOR (see [`ZoomAnchor`]): the document point + screen y the
    /// next reflow should hold fixed, so a keyboard ⌘± zooms around the CARET (not
    /// the viewport top) and the wheel zooms around the POINTER. `None` = plain
    /// top-anchored scroll. Consumed once by `sync_view`.
    zoom_anchor: Option<ZoomAnchor>,
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
    /// Only ever stamped on a lava world (`App::on_moved`'s gate) — a non-lava
    /// world takes the whole move machinery as a structural no-op (zero
    /// redraws scheduled by a move).
    move_settle_at: Option<Instant>,
    /// A THEME-PREVIEW step just crossed the lava boundary (a ticking lava world
    /// THE PREVIEW-SETTLE debounce: stamped by `App::retint_theme_preview` on
    /// EVERY theme-picker preview step (arrow / hover / wheel), re-stamped on each
    /// further step so it fires `CROSSING_SYNC_SETTLE` after the user STOPS
    /// navigating. Its presence arms the present-transaction bracket
    /// (`present_sync_armed`'s third source), so every preview frame — including
    /// the LANDING frame — joins the compositor's transaction rather than racing
    /// it (the vanishing-page fix, 2026-07-18: the old `preview_crossing`
    /// conditional bracketed only the transient boundary crossed mid-nav and left
    /// the actual landing frame unbracketed — three widenings never caught it, so
    /// the classification was RETIRED for unconditional arming). Structurally
    /// unreachable in a headless capture (the shared replay never previews).
    crossing_settle_at: Option<Instant>,
    /// EVENT-ORDERED bracket teardown: set by `finish_crossing_settle` once the
    /// preview settle elapses (the deferred font reshape has just been APPLIED in
    /// the same `about_to_wait` pass but not yet PRESENTED). It HOLDS the
    /// present-transaction bracket ON (a second source in `sync_present_txn`'s OR)
    /// until the next present — the one carrying the reshaped frame — completes
    /// INSIDE the bracket; the post-present hook in `on_redraw_requested` then
    /// clears it and disarms. This replaces the old timer-race (settle turned the
    /// bracket off in the SAME pass the reshape redraw was requested, so the
    /// heaviest frame coalesced into one UNBRACKETED present) with a mechanical
    /// happens-after: reshape present ⇒ THEN teardown. Live-only.
    crossing_teardown_pending: bool,
    /// Shadow of the CAMetalLayer's `presentsWithTransaction` flag — the ONE
    /// owner of its composition is `App::sync_present_txn`, which arms it while
    /// ANY live source (resize OR move stream, OR a theme-preview lava-boundary
    /// crossing) is active and disarms it only once ALL have settled
    /// (`present_sync_armed`). The MOVE half is the
    /// move-flash fix's structural core: any present that happens around a
    /// window move (the settle redraw, a sibling debounce firing mid-stream, a
    /// cross-display `ScaleFactorChanged` redraw) now JOINS the window-server's
    /// move transaction instead of racing it — the same cure the resize-stretch
    /// artifact already had, extended to the move stream it never covered.
    /// Tracked on every platform (the objc call itself is macOS-only), so the
    /// state machine stays unit-testable everywhere.
    present_sync_on: bool,
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
    /// WRITING STREAKS: the persisted daily-net-words record (native-only, LOCAL/
    /// PRIVATE — beside `stats.toml`). Loaded once at launch, sampled + persisted on
    /// the SAME autosave flush triggers as the odometer.
    #[cfg(not(target_arch = "wasm32"))]
    streaks: crate::streaks::Streaks,
    /// The active buffer's word count at the last streaks sample — the `last` side
    /// of the per-flush word DELTA. `None` until the first sample of a buffer (a
    /// fresh launch or right after a buffer swap), so opening a file's existing
    /// words is ANCHORED (never counted as "written"); the next flush records the
    /// delta from there. Reset to `None` on every buffer swap (`streaks_reset_baseline`).
    #[cfg(not(target_arch = "wasm32"))]
    streaks_baseline: Option<usize>,
    /// Whether the streaks record has unpersisted changes since the last flush, so
    /// a flush that recorded nothing (an anchor, or a no-net-change idle) skips the
    /// atomic write.
    #[cfg(not(target_arch = "wasm32"))]
    streaks_dirty: bool,
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
    fn rebuild_gpu(&mut self, event_loop: &ActiveEventLoop, reason: &str) {
        if self.gpu_lifecycle == GpuLifecycle::Rebuilding { return; }
        let Some(window) = self.recovery_window.clone() else { event_loop.exit(); return };
        self.gpu = None;
        self.gpu_lifecycle = GpuLifecycle::Rebuilding;
        self.last_frame = None;
        self.gpu_retry_at = None;
        self.gpu_timeout_streak = 0;
        self.input_stamp = None;
        self.set_sticky_notice(format!("{reason} — rebuilding graphics…"));
        let display_handle = event_loop.owned_display_handle();
        #[cfg(not(target_arch = "wasm32"))]
        match pollster::block_on(Gpu::new(window, display_handle)) {
            Ok(gpu) => {
                self.gpu = Some(gpu);
                self.gpu_lifecycle = GpuLifecycle::Active { oom_skips: 0 };
                self.set_toast_notice("graphics recovered");
                self.on_gpu_ready();
            }
            Err(e) => { eprintln!("failed to rebuild render state: {e}"); self.set_sticky_notice("graphics could not recover — closing safely"); event_loop.exit(); }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let slot = self.gpu_pending.clone();
            let wake = window.clone();
            wasm_bindgen_futures::spawn_local(async move {
                *slot.borrow_mut() = Some(Gpu::new(window, display_handle).await.map_err(|e| e.to_string()));
                wake.request_redraw();
            });
        }
    }

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
        // THE ONE TIME OWNER: the shipped `RealClock` (a pure `Instant::now()`
        // pass-through). Built before the literal so the session-timer origin
        // reads it (a `clock.now()` BORROW), then the box is moved into the
        // `clock` field. A deterministic clock would swap only this one line.
        let clock: Box<dyn crate::clock::Clock> = Box::new(crate::clock::RealClock);
        #[cfg(not(target_arch = "wasm32"))]
        let stats_origin = clock.now();
        let mut app = Self {
            file,
            buffer,
            keymap,
            clock,
            mods: Modifiers::default(),
            prefix_pending_at: None,
            whichkey_shown: false,
            scroll_lines: 0,
            gpu: None,
            recovery_window: None,
            gpu_lifecycle: GpuLifecycle::AwaitingWindow,
            gpu_retry_at: None,
            gpu_timeout_streak: 0,
            #[cfg(not(target_arch = "wasm32"))]
            soak: None,
            #[cfg(not(target_arch = "wasm32"))]
            soak_recovery_pending: None,
            #[cfg(not(target_arch = "wasm32"))]
            soak_passed: None,
            #[cfg(not(target_arch = "wasm32"))]
            probe_ready: None,
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
            popover_open: false,
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
            zoom_anchor: None,
            theme_font_at: None,
            lava_tick_at: None,
            focused: true,
            resize_settle_at: None,
            move_settle_at: None,
            crossing_settle_at: None,
            crossing_teardown_pending: false,
            present_sync_on: false,
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
            stats_origin,
            #[cfg(not(target_arch = "wasm32"))]
            stats_last_input_ms: None,
            #[cfg(not(target_arch = "wasm32"))]
            stats_last_caret_xy: None,
            #[cfg(not(target_arch = "wasm32"))]
            stats_last_cursor: None,
            #[cfg(not(target_arch = "wasm32"))]
            stats_dirty: false,
            // WRITING STREAKS: load the persisted record (empty on a fresh install),
            // beside the odometer. The baseline anchors on the first flush.
            #[cfg(not(target_arch = "wasm32"))]
            streaks: crate::streaks::load(&crate::streaks::streaks_path()),
            #[cfg(not(target_arch = "wasm32"))]
            streaks_baseline: None,
            #[cfg(not(target_arch = "wasm32"))]
            streaks_dirty: false,
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
        // WRITING STREAKS: set the INITIAL word-delta anchor now that every startup
        // buffer decision (scratch-stash restore + session restore, which can swap
        // the active buffer) has settled. An awl-CREATED scratch (no path — fresh
        // empty OR resumed stash) anchors EAGERLY at its birth word count, so words
        // typed before the first idle flush are recorded rather than swallowed by a
        // lazy first-flush anchor (the anchor-swallow bug); a resumed stash's own
        // words are anchored, never miscounted as today's writing. An opened FILE
        // (CLI arg or session-restored active) keeps the LAZY anchor — its
        // pre-existing words are not "writing" — so `streaks_baseline` stays `None`.
        #[cfg(not(target_arch = "wasm32"))]
        if app.file.is_none() {
            app.streaks_anchor_now();
        }
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
        self.notice_expires_at = self.gpu.as_ref().map(|_| self.clock.now() + TOAST_LIFETIME);
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

#[cfg(not(target_arch = "wasm32"))]
impl App {
    /// Build a HERMETIC, `gpu`-less App for the FRAME-LOOP capture
    /// (`--screenshot-frames`) — a native, non-test sibling of the `#[cfg(test)]`
    /// `new_hermetic`, deterministic by construction: an `InMemoryFs` (empty — no
    /// config/session/stash to read, no user disk touched, the same door
    /// `crate::scenario` uses for a strict replay) and reduce-motion/session-restore
    /// pinned off, exactly like `new_hermetic`. The capture harness swaps a
    /// [`crate::clock::VirtualClock`] in ([`set_clock`](Self::set_clock)) and steps
    /// the real scheduling body; this App renders nothing itself (`gpu: None`), so
    /// its buffer is just the scheduling driver — the harness draws the document +
    /// the panel its state reports through its OWN offscreen pipeline. Constructs via
    /// `Self::new` (not the raw open-paren needle the accounting guard scans for), so
    /// that guard is unaffected.
    ///
    /// Installs the `InMemoryFs` via the production `fs::set_active` (the SAME door
    /// `crate::scenario::install_hermetic_fs` uses for a strict replay), restoring the
    /// prior backend when construction returns — no test-only `with_fs`/serial lock
    /// (this is a single-threaded one-shot CLI, never a concurrent test). Routes
    /// through `Self::new`, not the raw constructor's open-paren needle, so the
    /// real-FS-constructor accounting guard is unaffected.
    pub(crate) fn new_headless_scheduler(root: PathBuf, config: Config) -> Self {
        let config = Config { session_restore: Some(false), reduce_motion: Some(false), ..config };
        let prev = crate::fs::active();
        crate::fs::set_active(Arc::new(crate::fs::InMemoryFs::new()));
        let app = Self::new(None, root, None, None, config);
        crate::fs::set_active(prev);
        app
    }
}

impl App {
    /// Shared post-GPU-init: fold the monitor's DPI scale into the metrics BEFORE
    /// the first sync (so the opening frame is proportioned like the capture on a
    /// HiDPI screen), push the initial view, and request the opening frame. Called
    /// inline after the NATIVE blocking init, and from `window_event` once the WASM
    /// async init deposits its GPU.
    fn on_gpu_ready(&mut self) {
        let Some(gpu) = self.gpu.as_ref() else { return };
        self.gpu_retry_at = None;
        self.gpu_timeout_streak = 0;
        let sf = gpu.window.scale_factor() as f32;
        self.dpi = sf;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.pipeline.set_dpi(sf);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let (Some(soak), Some(gpu)) = (self.soak.as_mut(), self.gpu.as_ref()) {
            soak.observe_backend(gpu.backend_name().to_string());
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
            let Some(gpu) = self.gpu.as_ref() else { return };
            let size = gpu.window.inner_size();
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.resize(size.width.max(1), size.height.max(1));
            }
        }
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
        // LIVE PROBE ready signal: the window + GPU exist, so the driver thread
        // may start feeding scripted input. FIRST make the window unoccludable:
        // the wgpu macOS occlusion gate returns `SurfaceError::Occluded` before
        // `nextDrawable()` for a window without `NSWindowOcclusionStateVisible`
        // — and a probe launched from a (fullscreen) terminal opens BEHIND it,
        // so without this the run presents ZERO frames (observed: every shot
        // "no frame has presented yet"). AlwaysOnTop guarantees visibility for
        // the run's few seconds regardless of the launching terminal;
        // `focus_window` additionally asks for key status so the run matches
        // the reported live conditions (focused editing).
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = self.probe_ready.take() {
            if let Some(gpu) = self.gpu.as_ref() {
                gpu.window
                    .set_window_level(winit::window::WindowLevel::AlwaysOnTop);
                gpu.window.focus_window();
            }
            let _ = tx.send(());
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn drive_gpu_soak(&mut self, event_loop: &ActiveEventLoop) {
        if self.soak.is_none() { return; }
        let now = self.clock.now();
        let metal = self.gpu.as_ref().and_then(Gpu::current_gpu_bytes);
        let (finished, stimuli) = {
            let Some(soak) = self.soak.as_mut() else { return };
            soak.sample_if_due(now, metal);
            let finished = soak.finished(now);
            let mut stimuli = Vec::new();
            if !finished {
                for _ in 0..32 {
                    let Some(stimulus) = soak.next_stimulus(now) else { break };
                    let stop = matches!(stimulus, crate::soak_gpu::Stimulus::Inject(_) | crate::soak_gpu::Stimulus::Resize { .. });
                    stimuli.push(stimulus);
                    if stop { break; }
                }
            }
            (finished, stimuli)
        };
        if finished {
            let Some(soak) = self.soak.as_ref() else { return };
            let report = soak.report(now);
            self.soak_passed = Some(report.passed());
            report.print();
            event_loop.exit();
            return;
        }
        for stimulus in stimuli.iter().copied() {
            match stimulus {
                crate::soak_gpu::Stimulus::SetLavaTheme => { let _ = crate::theme::set_active_by_name("Mangrove"); self.retint_theme_now(); self.sync_view(true); }
                crate::soak_gpu::Stimulus::Resize { width, height } => if let Some(w) = self.recovery_window.as_ref() { let _ = w.request_inner_size(winit::dpi::PhysicalSize::new(width, height)); },
                crate::soak_gpu::Stimulus::ThemeNext => { crate::theme::cycle(1); self.retint_theme_now(); self.sync_view(true); if let Some(s) = self.soak.as_mut() { s.observe_theme_switch(); } }
                crate::soak_gpu::Stimulus::Overlay { open } => {
                    let action = if open { Action::OpenCommandPalette } else { Action::Cancel };
                    let _ = self.apply(action, false, event_loop, crate::stats::Door::Chord);
                    if !open && self.overlay.is_none() { if let Some(s) = self.soak.as_mut() { s.observe_overlay_cycle(); } }
                }
                crate::soak_gpu::Stimulus::Inject(kind) => {
                    // LIMITATION (acceptable): `soak_recovery_pending` is a
                    // single slot, so two injections landing before a present
                    // observes the first recovery would drop the earlier
                    // timing. The schedule cannot reach that state — each fault
                    // kind is injected EXACTLY ONCE (`Controller::injected` is a
                    // one-shot latch per kind) and the batch loop above STOPS on
                    // any `Inject`, so at most one injection is issued per tick
                    // and the next present clears the slot before the following
                    // kind can fire. A queue would be dead generality here.
                    self.soak_recovery_pending = Some(kind);
                    if let Some(gpu) = self.gpu.as_mut() { match kind {
                        crate::soak_gpu::FaultKind::OutOfMemory => gpu.inject_fault(gpu::GpuFaultInjection::OutOfMemory),
                        crate::soak_gpu::FaultKind::SurfaceLost => gpu.inject_surface_loss(),
                        crate::soak_gpu::FaultKind::DeviceLost => gpu.inject_fault(gpu::GpuFaultInjection::DeviceLost),
                    }}
                }
            }
        }
        if !stimuli.is_empty() { if let Some(gpu) = self.gpu.as_ref() { gpu.window.request_redraw(); } }
        if self.last_frame.is_none() { event_loop.set_control_flow(control_flow_with_deadline(event_loop.control_flow(), now + if stimuli.len() == 32 { Duration::from_millis(1) } else { Duration::from_millis(100) })); }
    }

    /// DEBUG key→px: stamp the receipt of an input that will request a redraw —
    /// key presses AND mouse press/scroll, which share the `request_redraw` path.
    /// `get_or_insert` keeps only the FIRST un-rendered input per frame, so under
    /// coalescing the measured latency is the worst case. Gated on the panel
    /// being on (zero clock reads otherwise); the span closes at present-return
    /// in `RedrawRequested`. Wasm-safe (`crate::clock::Instant`).
    fn stamp_input(&mut self) {
        if crate::debug::debug_on() {
            let now = self.clock.now();
            self.input_stamp.get_or_insert(now);
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
    /// A live-probe step posted by the `--live-script` driver thread (see
    /// `crate::probe`'s module doc) — a scripted chord for the real dispatch
    /// tail, a compositor-side window shot, or the terminating quit.
    Probe(crate::probe::ProbeEvent),
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
            AwlEvent::Probe(e) => self.handle_probe_event(_event_loop, e),
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        if self.recovery_window.is_some() {
            self.rebuild_gpu(event_loop, "graphics resumed");
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
                .with_title(if self.soak.is_some() { "Awl GPU probe — keep visible".to_string() } else { title })
                .with_visible(true);
            // LIVE PROBE: a small, corner-anchored, DETERMINISTIC window
            // (`crate::probe::PROBE_LOGICAL_*`). Overrides any restored session
            // frame — a probe run is isolated (temp HOME) and must land in a
            // known small corner, not wherever the last real window happened to
            // sit. Anchored near the top-left, clear of the menu bar.
            // ALWAYS-ON-TOP is the occlusion cure: a non-activating (Prohibited)
            // window never comes to front, so it would otherwise sit OCCLUDED
            // behind the user's windows and wgpu would skip every present (the
            // occlusion tripwire) — leaving the harness blind to the very
            // present-race it exists to catch. `WindowLevel::AlwaysOnTop` floats
            // it above other windows so it stays unoccluded and presents fire,
            // WITHOUT making it key (window LEVEL is z-order, not focus — verified
            // FOCUS-GAINED stays 0). Small + cornered keeps the always-on-top
            // window out of the way.
            if crate::probe::live_active() {
                attrs
                    .with_inner_size(LogicalSize::new(
                        crate::probe::PROBE_LOGICAL_W,
                        crate::probe::PROBE_LOGICAL_H,
                    ))
                    .with_position(winit::dpi::LogicalPosition::new(48.0, 64.0))
                    // `with_active(false)` → winit shows the window via
                    // `orderFront` instead of `makeKeyAndOrderFront`, so it never
                    // becomes the KEY window (no keyboard-focus theft). Paired with
                    // the Prohibited policy + `activate_ignoring_other_apps(false)`
                    // in `crate::app::run`. Only the probe opts out of focus; a
                    // normal launch keeps the default active window.
                    .with_active(false)
            } else {
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
        self.recovery_window = Some(window.clone());
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
                self.gpu_lifecycle = GpuLifecycle::Active { oom_skips: 0 };
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
                self.set_sticky_notice("graphics unavailable — closing safely");
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
            self.gpu_lifecycle = GpuLifecycle::Rebuilding;
            let slot = self.gpu_pending.clone();
            let win = window.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match Gpu::new(window, display_handle).await {
                    Ok(gpu) => {
                        *slot.borrow_mut() = Some(Ok(gpu));
                        win.request_redraw();
                    }
                    Err(e) => { *slot.borrow_mut() = Some(Err(e.to_string())); win.request_redraw(); }
                }
            });
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.gpu = None;
        self.gpu_lifecycle = GpuLifecycle::Suspended;
        self.last_frame = None;
        self.input_stamp = None;
        self.resize_settle_at = None;
        self.move_settle_at = None;
        self.crossing_settle_at = None;
        self.crossing_teardown_pending = false;
        self.present_sync_on = false;
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
            if let Some(result) = pending {
                match result {
                    Ok(gpu) => { self.gpu = Some(gpu); self.gpu_lifecycle = GpuLifecycle::Active { oom_skips: 0 }; self.on_gpu_ready(); }
                    Err(e) => { log::error!("failed to rebuild render state: {e}"); self.set_sticky_notice("graphics could not recover — closing safely"); event_loop.exit(); }
                }
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
            WindowEvent::Occluded(occluded) => self.on_occluded(occluded),
            WindowEvent::Resized(size) => self.on_resized(event_loop, size),
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
        // WRITING STREAKS: the final day-delta flush, beside the odometer's.
        #[cfg(not(target_arch = "wasm32"))]
        self.streaks_flush();
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "mas")))]
        self.daemon_shutdown();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // The full scheduling body (every debounce / settle deadline + the
        // ambient tick + GPU retries) lives in `app/schedule.rs`; a trait impl
        // can't span files, so this method is a thin delegate to the inherent
        // `App::about_to_wait_impl` moved there. `ActiveEventLoop` is the live
        // `Scheduler` sink (a headless `RecordingScheduler` is the other, driven by
        // `step_scheduling`), so the SAME body runs under the virtual-clock harness.
        self.about_to_wait_impl(event_loop);
        // The GPU SOAK drive runs LAST (its historical position at the end of the
        // scheduling body) but OUTSIDE it: it needs the real `&ActiveEventLoop`
        // (resizes the recovery window, sets its own control flow) and always runs on
        // real time, so it never belongs on the clock-steppable path. No-ops unless a
        // `--soak-gpu` run is active.
        #[cfg(not(target_arch = "wasm32"))]
        self.drive_gpu_soak(event_loop);
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

/// Should the CAMetalLayer's `presentsWithTransaction` be armed? ONE owner of
/// the composition: armed while ANY source needs it — a RESIZE drag, a MOVE
/// drag, or a THEME-PREVIEW lava-boundary crossing — disarmed only once ALL have
/// settled (a corner drag streams both resize+move; a crossing can overlap a
/// drag; the settle of one source must never strip another's protection). Pure,
/// so the composition is unit-testable without a window; `App::sync_present_txn`
/// is the sole applier.
fn present_sync_armed(resize_active: bool, move_active: bool, crossing_active: bool) -> bool {
    resize_active || move_active || crossing_active
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

/// Does a held Shift on this CHORD signal SELECT-INTENT (Shift+motion extends
/// the selection, GUI style)? The rule keys on the pressed CHORD, not just the
/// `Action`, because BufferStart/BufferEnd are reached two very different ways:
///   * `M-<` / `M->` (emacs) need Shift just to TYPE the `<` / `>` glyph — a
///     `Key::Character` — so that Shift is INCIDENTAL (Emacs treats them as pure
///     motion; you select via the mark, `C-Space`) and must NOT extend.
///   * Shift+Cmd-Up/Down (macOS) and Shift+Ctrl-Home/End (Linux) reach the SAME
///     actions through a `Key::Named` navigation key — a genuine GUI
///     select-intent Shift the platform text fields all honor — and MUST extend.
/// So the ONE discriminator is the key's shape: a named navigation key extends,
/// a printable glyph whose Shift is needed just to type it does not. Every OTHER
/// action keeps Shift's normal select-extend meaning regardless of key. Pure, so
/// it's unit-testable without a window/event loop. THE ONE OWNER of the rule:
/// both the live key dispatch (`app/input/keys.rs`, passing the resolved logical
/// key) and the headless `--keys` replay
/// (`main/run.rs::ReplaySession::apply_chord`, passing the chord's key) derive
/// their `apply_core` shift flag through this fn, so an `S-` chord in a spec
/// signals select-intent exactly as a live held Shift does — never a parallel
/// copy of the rule.
pub(crate) fn motion_honors_shift_select(action: &Action, key: &Key) -> bool {
    match action {
        Action::BufferStart | Action::BufferEnd => matches!(key, Key::Named(_)),
        _ => true,
    }
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
    #[cfg(not(target_arch = "wasm32"))] soak: Option<crate::soak_gpu::SoakConfig>,
    #[cfg(not(target_arch = "wasm32"))] live: Option<crate::probe::LiveScript>,
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
    if soak.is_none() { crate::crashlog::install_hook(); }

    // FLIGHT RECORDER (native live-App only, capture-gated exactly like the crash
    // hook / daemon above — headless `--screenshot`/`--keys`/`--bench-*` never
    // reach `run`): if `AWL_FLIGHT_RECORDER=<path>` is set, arm the append-only
    // present/bracket/redraw trace so the user's next live theme-preview "page
    // vanishes" repro leaves a black box. A no-op when the env is absent.
    #[cfg(not(target_arch = "wasm32"))]
    crate::probe::init_flight();

    // SINGLE-INSTANCE DAEMON (native only, and compiled out entirely under
    // `mas` — see `crate::daemon`'s module doc for the full CAPTURE GATE
    // argument: this whole block lives ONLY on this live-App startup path,
    // never on any headless `--screenshot`/`--bench-*` mode). Runs the
    // bind-or-handoff dance BEFORE any window/GPU work, so handing off to an
    // already-running instance exits in milliseconds with no window ever
    // created. Under `mas`, Launch Services already refuses a second launch
    // and there is no CLI to hand a path off with, so this is simply absent.
    // The LIVE PROBE additionally skips the daemon outright (defense in depth
    // beyond the wrapper script's env isolation): a probe launch must never
    // hand its file off to — or bind over the socket of — the user's real
    // running instance. See `crate::probe`'s module doc.
    #[cfg(all(not(target_arch = "wasm32"), not(feature = "mas")))]
    let instance_listener = if soak.is_some() || live.is_some() { None } else { match crate::daemon::startup(file.as_deref(), wait) {
        Ok(crate::daemon::StartupOutcome::HandedOff) => return Ok(()),
        Ok(crate::daemon::StartupOutcome::Instance(l)) => Some(l),
        Err(e) => {
            // Never let a socket hiccup (permissions, a full /tmp, a bad XDG
            // path, …) block opening the editor — degrade to a normal,
            // non-singleton launch.
            eprintln!("awl: single-instance socket unavailable ({e}); continuing without it");
            None
        }
    }};

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
    if soak.is_none() { crate::mas::restore_all_grants(); }

    // LIVE PROBE (`--live-script`): launch WITHOUT STEALING FOCUS. Three winit
    // defaults each steal it and must all be turned off (the Accessory policy
    // alone does NOT — it only governs Dock/cmd-tab presence; the app still
    // activates and auto-keys its window, verified the hard way):
    //   1. ACTIVATION POLICY → Prohibited: the app can never be ACTIVATED, so
    //      `activateIgnoringOtherApps` is a no-op and no window of ours can become
    //      key (Accessory was insufficient — a `Focused(true)` still fired). No
    //      Dock icon, no cmd-tab entry, no menu-bar takeover either.
    //   2. `activate_ignoring_other_apps` defaults to TRUE → winit calls
    //      `NSApp.activateIgnoringOtherApps(true)` at launch, yanking the whole
    //      app (and the user's keyboard) to the foreground. Forced OFF here.
    //   3. (paired with the window's `with_active(false)` below → `orderFront`
    //      instead of `makeKeyAndOrderFront`, so the window shows but never
    //      becomes KEY.)
    // Net: the probe window appears on screen (visible + unoccluded — the wgpu
    // occlusion gate is about display VISIBILITY, not key status, so presents
    // still fire) while the user keeps typing into whatever they were using. The
    // driver injects chords straight into the event loop, never OS key focus, so
    // nothing the probe needs is lost. A normal launch stays Regular + active —
    // byte-identical activation to before.
    #[cfg(not(target_arch = "wasm32"))]
    let event_loop = {
        #[allow(unused_mut)]
        let mut builder = EventLoop::<AwlEvent>::with_user_event();
        #[cfg(target_os = "macos")]
        if live.is_some() {
            use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
            // PROHIBITED (not Accessory): Accessory still lets the app ACTIVATE on
            // launch, and an active app auto-makes its front window key — which
            // stole the user's keyboard (observed: a `Focused(true)` still fired).
            // A Prohibited app can never be activated, so `activateIgnoringOtherApps`
            // is a no-op and no window of ours can become key. The window is still
            // shown (`orderFront`) and composited, so presents/occlusion are
            // unaffected (verified nonzero in the smoke run).
            builder.with_activation_policy(ActivationPolicy::Prohibited);
            builder.with_activate_ignoring_other_apps(false);
        }
        builder.build()?
    };
    #[cfg(target_arch = "wasm32")]
    let event_loop = EventLoop::<AwlEvent>::with_user_event().build()?;
    #[cfg(not(target_arch = "wasm32"))]
    let proxy = event_loop.create_proxy();
    // `mut` is only needed on native (the macOS-menu-proxy stash + the
    // `run_app(&mut app)` call below); on wasm `app` is moved straight into
    // `spawn_app` without ever being mutated. Kept as ONE call site (never
    // duplicated across a `#[cfg]` split) — a law test below counts every
    // raw constructor call in this file.
    #[cfg(not(target_arch = "wasm32"))]
    let config = if soak.is_some() {
        crate::fs::set_active(Arc::new(crate::fs::InMemoryFs::new()));
        Config { session_restore: Some(false), autosave: Some(false), stats: Some(false), reduce_motion: Some(false), ..config }
    } else { config };
    #[allow(unused_mut)]
    let mut app = App::new(file, root, cli_workspace, cli_notes_root, config);
    #[cfg(not(target_arch = "wasm32"))]
    { app.soak = soak.map(crate::soak_gpu::Controller::new); }
    // NATIVE MACOS MENU BAR: stash a proxy clone now (before the daemon's own
    // clone below is potentially moved away) so `resumed()` can install the
    // menu bar once the window/NSApp exists — see `App::menu_proxy`'s doc.
    #[cfg(target_os = "macos")]
    {
        if app.soak.is_none() { app.menu_proxy = Some(proxy.clone()); }
    }
    // LIVE PROBE (`--live-script`): arm the ready signal and spawn the driver
    // thread (the daemon's own EventLoopProxy precedent — scripted steps are
    // posted into the winit loop, never cross-thread `App` access). Shots land
    // in the script's shots dir, created here so the very first `shot` step
    // can't fail on a missing directory.
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(script) = live {
        // Arm the process-global FIRST — `Gpu::new` (called from `resumed`,
        // strictly later) reads it to add COPY_SRC + the frame mirror.
        crate::probe::set_live_active();
        if let Err(e) = std::fs::create_dir_all(&script.shots_dir) {
            eprintln!("LIVE-PROBE error: cannot create shots dir {}: {e}", script.shots_dir.display());
        }
        let (tx, rx) = std::sync::mpsc::channel();
        app.probe_ready = Some(tx);
        let probe_proxy = proxy.clone();
        crate::probe::spawn_driver(script, rx, move |e| {
            probe_proxy.send_event(AwlEvent::Probe(e)).is_ok()
        });
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
        if app.soak.is_some() && app.soak_passed != Some(true) {
            anyhow::bail!("native GPU soak did not meet its verification contract");
        }
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
mod tests;
#[cfg(test)]
mod clock_law;
