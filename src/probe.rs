//! THE LIVE PROBE HARNESS (`--live-script`) — scripted keystrokes + real-window
//! screenshots against the REAL windowed app.
//!
//! ## The class of bug this exists to catch
//!
//! The headless `--screenshot` harness renders offscreen: it rebuilds the text
//! pipeline every capture, never opens an OS window, never presents to a real
//! compositor, and never runs the live `WaitUntil` debounce machinery over real
//! time. That makes it structurally BLIND to three whole bug classes the live
//! app owns alone:
//!   (a) stale caches across live state transitions (the capture rebuilds
//!       everything per frame, so a missed invalidation can never show);
//!   (b) redraw-scheduling gaps (a state change whose frame is simply never
//!       drawn — the capture always draws exactly one frame on purpose);
//!   (c) present/compositor races (the frame is correct but the macOS
//!       window-server shows a stale/blank drawable — `presentsWithTransaction`
//!       territory; provably invisible offscreen).
//! The theme-picker "page vanishes while previewing" bug survived three
//! law-tested fixes precisely because every fix was verified through the
//! offscreen path — which was proven byte-identical across the full 16×16
//! world matrix while the live symptom persisted. CLAUDE.md's rule applies:
//! when a bug won't reproduce headlessly, EXTEND THE HARNESS TOWARD REALITY.
//! This module is that extension: the NORMAL winit loop, the real GPU surface,
//! real presents, real debounce timers — with a script driving the same seams
//! a keystroke drives, and screenshots taken from the COMPOSITOR's side of the
//! window (`CGWindowListCreateImage` of our own window — the window server's
//! current idea of our pixels, not a re-render of what we hoped they were).
//!
//! ## Grammar (deliberately dumb)
//!
//! `--live-script "<step>; <step>; ..."` — semicolon-separated steps:
//!   - `keys <chordspec>` — space-separated chords fed through the REAL keymap
//!     path exactly as keystrokes would (same dispatch tail as
//!     `WindowEvent::KeyboardInput`; see `App::dispatch_pressed_key`). Chords
//!     within one `keys` step are posted back-to-back (a burst); use `sleep`
//!     between steps to dwell.
//!   - `sleep <ms>` — the driver thread pauses; the app runs its normal live
//!     loop (debounces fire, frames present) for that long.
//!   - `move <x> <y>` — move the pointer to PHYSICAL (x, y) through the real
//!     `on_cursor_moved`; while a picker is open this HOVER-previews the row
//!     under the cursor (a hover SWEEP is many `move`s with `sleep`s between —
//!     the dense `CursorMoved` stream no keyboard burst reproduces).
//!   - `wheel <n>` — mouse wheel by n notches (wheel-up positive) through the
//!     real `on_mouse_wheel`; an open picker advances + previews, coordinate-free.
//!   - `shot <name>` — screenshot the real window into `<shots-dir>/<name>.png`
//!     (`--live-shots DIR`, default the system temp dir). Every shot prints one
//!     `LIVE-PROBE shot …` line to stdout for the wrapping script to assert on.
//!   - `quit` — clean exit through the same `Action::Quit` a Cmd-Q takes.
//!     Appended automatically if the script doesn't end with one, so a probe
//!     run always terminates.
//!
//! ## Capture gate + isolation
//!
//! Native-live-only, exactly like the daemon: the flag exists only on
//! `Mode::Windowed`, the driver spawns only inside `crate::app::run`, and no
//! headless `--screenshot`/`--keys` path can ever reach it. The wrapping
//! script (`scripts/live-probe.sh`) points `HOME`/`XDG_CONFIG_HOME`/
//! `XDG_DATA_HOME` at a temp dir so a probe run can never touch the user's
//! real config/session/history — and `app::run` additionally skips the
//! single-instance daemon entirely when a live script is armed, so a probe can
//! never hand its file off to (or hijack the socket of) the user's real
//! running instance, even when launched without the wrapper.

// The TYPES + parser below are portable (so `Mode::Windowed` can carry the
// field on every target); the DRIVER thread and the capture backend — the
// parts that touch an OS — are native-only (`spawn_driver`) and macOS-only
// (`cgshot`). The wasm build parses no CLI, so `LiveScript` is never
// constructed there — the same "field exists, value never does" shape as
// `wait`.

use std::path::PathBuf;

use anyhow::{bail, Result};

/// One parsed `--live-script` step. See the module doc for the grammar.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Feed these chords through the real keymap dispatch, back-to-back.
    Keys(Vec<crate::keyspec::Chord>),
    /// Driver-side pause (ms) while the app's live loop runs normally.
    Sleep(u64),
    /// Screenshot the real window to `<shots-dir>/<name>.png`.
    Shot(String),
    /// Move the pointer to PHYSICAL (x, y) — the real `on_cursor_moved` seam, so
    /// an open picker HOVER-previews the row under the cursor exactly like a live
    /// mouse move (`overlay_hover` → `retint_theme_preview`). A hover SWEEP is
    /// many `move` steps with small `sleep`s between (the dense `CursorMoved`
    /// stream a real sweep produces, which no keyboard burst reproduces).
    MouseMove(f64, f64),
    /// Mouse WHEEL by N notches (sign = direction, wheel-up positive) — the real
    /// `on_mouse_wheel` seam; an open picker advances its selection + previews
    /// (`overlay_wheel` → `retint_theme_preview`), coordinate-free.
    Wheel(f32),
    /// Clean exit via `Action::Quit`.
    Quit,
}

/// The whole armed probe: parsed steps + where shots land. The type is PORTABLE
/// (so `Mode::Windowed` can carry the `Option<LiveScript>` field on every target,
/// the "field exists, value never does" shape shared with `wait`); the fields are
/// only READ by the native driver (`spawn_driver`), so on wasm — where no
/// `LiveScript` is ever constructed — they are legitimately dead.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
#[derive(Debug, Clone)]
pub struct LiveScript {
    pub steps: Vec<Step>,
    pub shots_dir: PathBuf,
}

/// What the driver thread posts into the winit loop (via `EventLoopProxy`,
/// the daemon's own precedent — never cross-thread `App` access). `Sleep`
/// never crosses the channel: the driver sleeps on its own thread. Native-only:
/// the driver + the winit-side handler are both native, so the wasm build (which
/// never arms a probe) never names this type.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
pub enum ProbeEvent {
    /// One chord: dispatched through the same tail a real key press takes.
    Chord(crate::keyspec::Chord),
    /// Screenshot the real window to this exact path (main-thread capture).
    Shot(PathBuf),
    /// Move the pointer to PHYSICAL (x, y) through the real `on_cursor_moved`.
    MouseMove(f64, f64),
    /// Mouse wheel by N notches through the real `on_mouse_wheel`.
    Wheel(f32),
    /// Clean exit through `Action::Quit`.
    Quit,
}

/// THE PROBE-MODE PROCESS GLOBAL: `true` iff this launch armed `--live-script`.
/// Set ONCE in `crate::app::run` before any GPU exists; read by `Gpu::new`
/// (adds `COPY_SRC` to the surface usage) and `Gpu::redraw` (mirrors every
/// PRESENTED frame into the probe's shot texture). `false` on every other
/// launch, keeping the production surface config byte-identical. Mirrors the
/// `debug::debug_on` process-global precedent.
#[cfg(not(target_arch = "wasm32"))]
static LIVE_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(not(target_arch = "wasm32"))]
pub fn set_live_active() {
    LIVE_ACTIVE.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn live_active() -> bool {
    LIVE_ACTIVE.load(std::sync::atomic::Ordering::Relaxed)
}

/// THE FLIGHT RECORDER (native live-App only) — the user's black box for the
/// live "the page vanishes while previewing a theme" report. That bug is a
/// present/compositor race: awl renders the correct frame but the macOS
/// window-server shows a stale/blank drawable, so a readback of OUR OWN frame
/// (the probe mirror) would look fine — the diagnostic signal is the PRESENT
/// PATH itself (was the frame presented or skipped? was the transaction bracket
/// armed? did a redraw get scheduled?), not a pixel of our own render. The
/// vanish also will not reproduce under the automated probe (its non-key window
/// is unfocused, so the ambient tick pauses and present races differ), while the
/// user reproduces it constantly on their real focused window — so the honest
/// tool is to hand them the recorder and read the trace of the next repro.
///
/// Armed by `AWL_FLIGHT_RECORDER=<path>` at launch (`init_flight`, called once
/// from `crate::app::run`, the ONE native live door — a headless capture never
/// reaches it, mirroring the daemon/probe capture gate). When armed, every
/// diagnostic `trace` line ALSO appends to that file (flushed per line, so a
/// crash/force-quit keeps the black box), and the `recording`-gated trace points
/// across the app fire in the NORMAL live session, not just under the probe.
/// Absent env = a total no-op, production byte-identical.
#[cfg(not(target_arch = "wasm32"))]
static FLIGHT_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(not(target_arch = "wasm32"))]
static FLIGHT_SINK: std::sync::Mutex<Option<std::io::BufWriter<std::fs::File>>> =
    std::sync::Mutex::new(None);

/// Process-start monotonic anchor: each flight line is stamped `+<ms>` since this,
/// so present gaps (the vanish signature) read directly off the log while the
/// header carries the wall-clock start for correlating with the user's "it
/// vanished at HH:MM".
#[cfg(not(target_arch = "wasm32"))]
static FLIGHT_START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Arm the flight recorder from `AWL_FLIGHT_RECORDER` if the user set it. Opens the
/// file in APPEND mode (a session adds to the black box, never truncates a prior
/// repro). A missing/empty var or an open failure leaves the recorder OFF — never
/// blocks launch. Idempotent-safe (re-arming just replaces the sink).
#[cfg(not(target_arch = "wasm32"))]
pub fn init_flight() {
    let Some(path) = std::env::var_os("AWL_FLIGHT_RECORDER") else {
        return;
    };
    if path.is_empty() {
        return;
    }
    arm_flight(std::path::Path::new(&path));
}

/// The env-free arming core (so a test can drive it without mutating `std::env` —
/// the `set_var`/`var` data-race hazard). Opens the black box in APPEND mode and
/// writes a header line stamping the build + wall-clock start.
#[cfg(not(target_arch = "wasm32"))]
fn arm_flight(path: &std::path::Path) {
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(f) => {
            let _ = FLIGHT_START.set(std::time::Instant::now());
            if let Ok(mut sink) = FLIGHT_SINK.lock() {
                *sink = Some(std::io::BufWriter::new(f));
            }
            FLIGHT_ACTIVE.store(true, std::sync::atomic::Ordering::Relaxed);
            let wall = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            trace(format_args!(
                "=== flight-recorder armed (awl {}, pid {}, unix {wall}) ===",
                env!("CARGO_PKG_VERSION"),
                std::process::id(),
            ));
        }
        Err(e) => eprintln!("awl: AWL_FLIGHT_RECORDER open failed ({e}); flight recorder off"),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn flight_active() -> bool {
    FLIGHT_ACTIVE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Should the `recording`-gated diagnostic trace points fire? True under EITHER
/// the automated live PROBE (`--live-script`) OR the user's FLIGHT RECORDER
/// (`AWL_FLIGHT_RECORDER`). The vanish-hunt trace points guard on THIS (not the
/// narrower `live_active`) so the same well-placed seams serve both readers —
/// one set of trace points, two consumers, never a parallel copy.
#[cfg(not(target_arch = "wasm32"))]
pub fn recording() -> bool {
    live_active() || flight_active()
}

/// The LIVE PROBE window's fixed LOGICAL size (px): small + corner-anchored (see
/// the window-attrs branch in `App::resumed`), so a probe window never sits
/// center-stage stealing the eye — the companion to the Prohibited activation
/// policy (`crate::app::run`) that keeps it from stealing keyboard FOCUS. The
/// wrapping script (`scripts/live-probe.sh`) renders its HEADLESS references at
/// this exact `--capture-size`, so the pixel comparison stays dpi-agnostic: the
/// live LOGICAL size equals the ref LOGICAL size, and the display's real scale
/// factor is absorbed as the integer block-compare scale. KEEP IN LOCKSTEP with
/// that script's `PROBE_CANVAS`.
#[cfg(not(target_arch = "wasm32"))]
pub const PROBE_LOGICAL_W: f64 = 900.0;
#[cfg(not(target_arch = "wasm32"))]
pub const PROBE_LOGICAL_H: f64 = 600.0;

/// ONE owner of the `PROBE-TRACE …` diagnostic line — the present/crossing/move
/// trace the vanish hunt reads (stamped with a wall-clock `Instant` so the
/// ordering of retint → present-txn → present → settle is legible in the log).
/// Call sites guard on [`live_active`] BEFORE building the `format_args!` (so a
/// normal launch pays nothing), then route the actual print through here — which
/// keeps every trace print in THIS file, so the println-audit (`println_audit`)
/// has exactly one site to account for instead of a scatter across the app
/// modules. stderr, so it never mixes with the `LIVE-PROBE …` stdout protocol
/// the wrapping script asserts on.
#[cfg(not(target_arch = "wasm32"))]
pub fn trace(args: std::fmt::Arguments) {
    // The PROBE reads its ordering off stderr (`PROBE-TRACE …`); only the live
    // probe prints there, so a flight-recorder-only session stays silent on the
    // terminal (the user's normal editor must not spew).
    if live_active() {
        eprintln!("PROBE-TRACE {args} t={:?}", std::time::Instant::now());
    }
    // The FLIGHT RECORDER appends the same line to the user's file, stamped `+<ms>`
    // since arm so present gaps read directly. Flushed per line so a force-quit
    // mid-repro keeps the tail. A poisoned lock or write error is swallowed —
    // diagnostics must never crash the editor.
    if flight_active() {
        if let Ok(mut guard) = FLIGHT_SINK.lock() {
            if let Some(w) = guard.as_mut() {
                use std::io::Write;
                let ms = FLIGHT_START
                    .get()
                    .map(|s| s.elapsed().as_millis())
                    .unwrap_or(0);
                let _ = writeln!(w, "+{ms}ms {args}");
                let _ = w.flush();
            }
        }
    }
}

/// Parse the `--live-script` grammar. A malformed step names itself in the
/// error (this is our own harness input — fail fast, the lenient-user-config
/// posture does not apply). Appends a trailing [`Step::Quit`] when absent so a
/// probe run always terminates.
pub fn parse_script(spec: &str) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    for raw in spec.split(';') {
        let s = raw.trim();
        if s.is_empty() {
            continue;
        }
        let (verb, rest) = match s.split_once(char::is_whitespace) {
            Some((v, r)) => (v, r.trim()),
            None => (s, ""),
        };
        match verb {
            "keys" => {
                if rest.is_empty() {
                    bail!("--live-script: `keys` needs a chord spec (e.g. \"keys Cmd-T Down\")");
                }
                steps.push(Step::Keys(crate::keyspec::parse_chords(rest)?));
            }
            "sleep" => {
                let ms: u64 = rest
                    .parse()
                    .map_err(|_| anyhow::anyhow!("--live-script: `sleep` needs ms, got {rest:?}"))?;
                steps.push(Step::Sleep(ms));
            }
            "shot" => {
                if rest.is_empty()
                    || !rest
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                {
                    bail!("--live-script: `shot` needs a [A-Za-z0-9._-] name, got {rest:?}");
                }
                steps.push(Step::Shot(rest.to_string()));
            }
            "move" => {
                let mut it = rest.split_whitespace();
                let (x, y) = (it.next(), it.next());
                match (x.and_then(|s| s.parse::<f64>().ok()), y.and_then(|s| s.parse::<f64>().ok())) {
                    (Some(x), Some(y)) if it.next().is_none() => steps.push(Step::MouseMove(x, y)),
                    _ => bail!("--live-script: `move` needs PHYSICAL x y (e.g. \"move 900 640\"), got {rest:?}"),
                }
            }
            "wheel" => {
                let n: f32 = rest
                    .parse()
                    .map_err(|_| anyhow::anyhow!("--live-script: `wheel` needs a notch count (e.g. \"wheel -2\"), got {rest:?}"))?;
                steps.push(Step::Wheel(n));
            }
            "quit" => steps.push(Step::Quit),
            other => bail!("--live-script: unknown step {other:?} (keys|sleep|shot|move|wheel|quit)"),
        }
    }
    if steps.is_empty() {
        bail!("--live-script: empty script");
    }
    if steps.last() != Some(&Step::Quit) {
        steps.push(Step::Quit);
    }
    Ok(steps)
}

/// Spawn the driver thread: wait for the app's ready signal (the first
/// GPU-ready, sent by `App::on_gpu_ready`), then walk the steps — sleeping
/// locally, posting everything else into the winit loop through `post` (a
/// `EventLoopProxy::send_event` wrapper; returns `false` once the loop is
/// gone, which ends the walk). The extra settle sleep after the ready signal
/// gives the very first frame time to present before any scripted input.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_driver(
    script: LiveScript,
    ready: std::sync::mpsc::Receiver<()>,
    post: impl Fn(ProbeEvent) -> bool + Send + 'static,
) {
    std::thread::Builder::new()
        .name("awl-live-probe".into())
        .spawn(move || {
            if ready
                .recv_timeout(std::time::Duration::from_secs(15))
                .is_err()
            {
                eprintln!("LIVE-PROBE error: app never signalled ready; quitting");
                let _ = post(ProbeEvent::Quit);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
            for step in script.steps {
                let ok = match step {
                    Step::Sleep(ms) => {
                        std::thread::sleep(std::time::Duration::from_millis(ms));
                        true
                    }
                    Step::Keys(chords) => chords
                        .into_iter()
                        .all(|c| post(ProbeEvent::Chord(c))),
                    Step::MouseMove(x, y) => post(ProbeEvent::MouseMove(x, y)),
                    Step::Wheel(n) => post(ProbeEvent::Wheel(n)),
                    Step::Shot(name) => {
                        post(ProbeEvent::Shot(script.shots_dir.join(format!("{name}.png"))))
                    }
                    Step::Quit => {
                        let _ = post(ProbeEvent::Quit);
                        return;
                    }
                };
                if !ok {
                    return; // event loop closed underneath us
                }
            }
        })
        .expect("spawn live-probe driver thread");
}

// --- The compositor-side window capture (macOS) -------------------------------
//
// `CGWindowListCreateImage` asks the WINDOW SERVER for its current composited
// image of ONE window — our own. Capturing your own process's windows is
// exempt from the Screen Recording TCC permission (the restriction guards
// OTHER apps' content), so this needs no grant, no prompt, and it reads the
// pixels the compositor is actually holding — which is exactly where the
// "page vanishes" class of bug lives. Deprecated API (macOS 14+ points at
// ScreenCaptureKit), but SCK requires the TCC grant even for self-capture;
// this stays the right tool for a self-inspecting harness. A plain C API, so
// declared here directly rather than growing `mac_chrome.rs`'s objc2 surface
// (only the NSWindow number lookup lives there).

#[cfg(target_os = "macos")]
mod cgshot {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGSize {
        w: f64,
        h: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    // kCGWindowListOptionIncludingWindow = 1 << 3 (capture exactly this window).
    const INCLUDING_WINDOW: u32 = 1 << 3;
    // kCGWindowImageBoundsIgnoreFraming (1<<0): no shadow/framing effects;
    // kCGWindowImageBestResolution (1<<3): native (retina) resolution.
    const IMAGE_OPTS: u32 = (1 << 0) | (1 << 3);

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        static CGRectNull: CGRect;
        fn CGWindowListCreateImage(
            bounds: CGRect,
            list_option: u32,
            window_id: u32,
            image_option: u32,
        ) -> *mut core::ffi::c_void; // CGImageRef
        fn CGImageGetWidth(image: *mut core::ffi::c_void) -> usize;
        fn CGImageGetHeight(image: *mut core::ffi::c_void) -> usize;
        fn CGImageGetBytesPerRow(image: *mut core::ffi::c_void) -> usize;
        fn CGImageGetBitsPerPixel(image: *mut core::ffi::c_void) -> usize;
        fn CGImageGetBitmapInfo(image: *mut core::ffi::c_void) -> u32;
        fn CGImageGetDataProvider(image: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
        fn CGDataProviderCopyData(provider: *mut core::ffi::c_void) -> *mut core::ffi::c_void; // CFDataRef
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFDataGetBytePtr(data: *mut core::ffi::c_void) -> *const u8;
        fn CFDataGetLength(data: *mut core::ffi::c_void) -> isize;
        fn CFRelease(cf: *mut core::ffi::c_void);
    }

    /// Ask the window server for its composited image of `window_id` as RGBA.
    /// Returns a short human error on any failure. NOTE: on a machine without
    /// the Screen Recording TCC grant, macOS quietly hands back a tiny generic
    /// PLACEHOLDER thumbnail instead of the window's pixels (observed
    /// empirically: ~194x192 white card for a 2400x1664 window) — the CALLER
    /// must validate the returned dimensions against the real surface size and
    /// fall back to the frame mirror on a mismatch (`App::probe_shot`).
    pub fn capture_window_image(window_id: u32) -> Result<image::RgbaImage, String> {
        // SAFETY: plain C calls; every CF object created here is released on
        // every path before return, and the byte slice is copied out before
        // its owning CFData is released.
        unsafe {
            let image = CGWindowListCreateImage(CGRectNull, INCLUDING_WINDOW, window_id, IMAGE_OPTS);
            if image.is_null() {
                return Err("CGWindowListCreateImage returned null (window gone?)".into());
            }
            let (w, h) = (CGImageGetWidth(image), CGImageGetHeight(image));
            let stride = CGImageGetBytesPerRow(image);
            let bpp = CGImageGetBitsPerPixel(image);
            let info = CGImageGetBitmapInfo(image);
            let provider = CGImageGetDataProvider(image);
            if provider.is_null() || w == 0 || h == 0 || bpp != 32 {
                CFRelease(image);
                return Err(format!("unusable window image ({w}x{h}, {bpp}bpp)"));
            }
            let data = CGDataProviderCopyData(provider);
            if data.is_null() {
                CFRelease(image);
                return Err("CGDataProviderCopyData returned null".into());
            }
            let len = CFDataGetLength(data) as usize;
            let bytes = std::slice::from_raw_parts(CFDataGetBytePtr(data), len);
            // Window-server images are 32bpp; byte order little (kCGBitmapByteOrder32Little,
            // 2 << 12) means BGRA in memory, otherwise ARGB (alpha-first big-endian).
            let little = (info & (3 << 12)) == (2 << 12);
            let mut rgba = vec![0u8; w * h * 4];
            for y in 0..h {
                let row = &bytes[y * stride..y * stride + w * 4];
                for x in 0..w {
                    let px = &row[x * 4..x * 4 + 4];
                    let (r, g, b, a) = if little {
                        (px[2], px[1], px[0], px[3])
                    } else {
                        (px[1], px[2], px[3], px[0])
                    };
                    let o = (y * w + x) * 4;
                    rgba[o..o + 4].copy_from_slice(&[r, g, b, a]);
                }
            }
            CFRelease(data);
            CFRelease(image);
            image::RgbaImage::from_raw(w as u32, h as u32, rgba)
                .ok_or_else(|| "rgba buffer size mismatch".to_string())
        }
    }
}

#[cfg(target_os = "macos")]
pub use cgshot::capture_window_image;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_covers_every_verb_and_appends_the_terminating_quit() {
        let steps =
            parse_script("keys Cmd-T Down; sleep 250; shot dwell-1").expect("parses");
        assert_eq!(steps.len(), 4, "keys + sleep + shot + the appended quit");
        match &steps[0] {
            Step::Keys(chords) => assert_eq!(chords.len(), 2),
            other => panic!("expected Keys, got {other:?}"),
        }
        assert_eq!(steps[1], Step::Sleep(250));
        assert_eq!(steps[2], Step::Shot("dwell-1".into()));
        assert_eq!(steps[3], Step::Quit, "a script always terminates");
    }

    #[test]
    fn parse_keeps_an_explicit_trailing_quit_single() {
        let steps = parse_script("keys Down; quit").expect("parses");
        assert_eq!(steps.len(), 2);
        assert_eq!(steps.last(), Some(&Step::Quit));
    }

    #[test]
    fn parse_covers_mouse_move_and_wheel() {
        let steps = parse_script("move 900 640; wheel -2; wheel 1").expect("parses");
        assert_eq!(steps[0], Step::MouseMove(900.0, 640.0));
        assert_eq!(steps[1], Step::Wheel(-2.0));
        assert_eq!(steps[2], Step::Wheel(1.0));
        assert_eq!(steps.last(), Some(&Step::Quit), "still terminates");
        for bad in ["move 900", "move a b", "move 1 2 3", "wheel nudge"] {
            assert!(parse_script(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    // The window constants are native-only (the probe drives a real NSWindow);
    // the wasm test target must not reference them.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn probe_window_is_smaller_than_the_center_stage_default() {
        // The "small + cornered" contract: the probe window must be strictly
        // smaller than the 1200x800 default the normal editor opens at (so it
        // never reads as the main window), yet comfortably above any degenerate
        // floor (it still has to render a real page + the theme picker for the
        // vanish repro to mean anything). Pure over the constants, so a future
        // resize can't silently make the probe window center-stage again.
        assert!(
            PROBE_LOGICAL_W < 1200.0 && PROBE_LOGICAL_H < 800.0,
            "probe window {PROBE_LOGICAL_W}x{PROBE_LOGICAL_H} must be smaller than the 1200x800 default"
        );
        assert!(
            PROBE_LOGICAL_W >= 640.0 && PROBE_LOGICAL_H >= 400.0,
            "probe window must stay large enough to render a real page + picker"
        );
    }

    /// THE FLIGHT RECORDER LAW: arming (env-free, via `arm_flight`) flips
    /// `flight_active`/`recording`, and a `trace` line lands in the file with the
    /// `+<ms>` stamp — the black box the user enables for the live vanish repro.
    /// Global-state + fs, so it takes the process-wide `serial()` guard and disarms
    /// on the way out (the flag must not leak into a sibling test's `recording()`).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn flight_recorder_arms_and_appends_a_stamped_line() {
        let _g = crate::testlock::serial();
        let path = std::env::temp_dir().join(format!("awl-flight-test-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert!(!flight_active(), "flight starts disarmed");
        assert!(!live_active(), "no probe in a unit test, so recording() == flight_active()");
        arm_flight(&path);
        assert!(flight_active(), "arming flips the flag");
        assert!(recording(), "recording() is true under the flight recorder alone");
        trace(format_args!("preview Galah -> Magpie {}", 42));
        let body = std::fs::read_to_string(&path).expect("the flight file exists");
        assert!(
            body.contains("preview Galah -> Magpie 42"),
            "the traced line landed in the black box, got:\n{body}"
        );
        assert!(body.contains("flight-recorder armed"), "the header line is present:\n{body}");
        assert!(
            body.lines().all(|l| l.starts_with("+") && l.contains("ms ")),
            "every line carries the +<ms> stamp:\n{body}"
        );
        // Disarm + clean up so the process global never leaks past this test.
        FLIGHT_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut s) = FLIGHT_SINK.lock() { *s = None; }
        let _ = std::fs::remove_file(&path);
        assert!(!recording(), "disarmed again — no leak into sibling tests");
    }

    #[test]
    fn parse_rejects_the_malformed_forms_by_name() {
        for (spec, needle) in [
            ("", "empty script"),
            ("dance", "unknown step"),
            ("keys", "needs a chord spec"),
            ("sleep soon", "needs ms"),
            ("shot ../escape", "shot"),
            ("keys NotAChord-", "chord"),
        ] {
            let err = parse_script(spec).expect_err(spec).to_string().to_lowercase();
            assert!(
                err.contains(&needle.to_lowercase()),
                "{spec:?} should fail mentioning {needle:?}, got: {err}"
            );
        }
    }
}
