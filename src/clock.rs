//! Platform clock — the SINGLE place the native/wasm time split lives, so no
//! call site can re-introduce the wasm panic by reaching for raw `std::time`.
//!
//! On `wasm32-unknown-unknown` std's `Instant::now()` AND `SystemTime::now()`
//! PANIC — `"time not implemented on this platform"` — because the browser
//! exposes no std clock. The `web-time` crate (a WASM-ONLY target dep; never in
//! the native dependency graph) shims them over the JS `Performance`/`Date`
//! APIs. The split here is deliberately ASYMMETRIC, because the two clocks play
//! different roles:
//!
//! * [`Instant`] — the live editor's MONOTONIC wall-clock (spring dt, debounces,
//!   the session timer). It is App-local, never crosses a module boundary, and
//!   has no std interop, so it is the REAL platform type: `std` on native,
//!   `web_time::Instant` on wasm. `Instant::now()` therefore works on both.
//!
//! * [`SystemTime`] — a wall-clock STAMP that crosses module boundaries: the FS
//!   seam reads file mtimes as `std::time::SystemTime` (the `NativeFs` backend,
//!   compiled on every target, hands back `std::fs` times), and index/hud/app
//!   carry them around. So the TYPE stays `std::time::SystemTime` on every
//!   target — it is wasm-SAFE *as a type*, only its `::now()` clock READ panics.
//!   Use [`system_now`] for that one read: native calls std directly; wasm draws
//!   the JS epoch clock via `web-time` and rebuilds a std `SystemTime` by ADDING
//!   to the const `UNIX_EPOCH` (never the unsupported native clock). Native is
//!   byte-identical; const arithmetic (`SystemTime::UNIX_EPOCH + Duration`) and
//!   `duration_since` stay std and never touch a clock, so they are wasm-safe.
#[cfg(not(target_arch = "wasm32"))]
pub use std::time::Instant;
#[cfg(target_arch = "wasm32")]
pub use web_time::Instant;

// The cross-boundary stamp type is std on EVERY target (see the module note):
// it must interop with `std::fs` mtimes that `NativeFs` produces even on wasm.
pub use std::time::SystemTime;

/// Wall-clock now, WASM-SAFE. std's `SystemTime::now()` PANICS on
/// `wasm32-unknown-unknown` (no platform clock); on wasm this draws the SAME std
/// `SystemTime` from the JS epoch clock via `web-time`, built by ADDING millis to
/// the const `UNIX_EPOCH` so it never reads the unsupported native clock. Native
/// is the byte-identical `std::time::SystemTime::now()`.
#[cfg(not(target_arch = "wasm32"))]
pub fn system_now() -> SystemTime {
    SystemTime::now()
}
#[cfg(target_arch = "wasm32")]
pub fn system_now() -> SystemTime {
    let ms = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(ms)
}

/// The live editor's ONE owner of "what monotonic time is it" for SCHEDULING
/// and ANIMATION. Every consumer that used to call the free `Instant::now()` on
/// the scheduling/animation path — each debounce/settle deadline in
/// `app::schedule`, the caret-spring frame `dt`, the ambient (lava/stars) tick,
/// toast expiry, GPU-retry timing, and the App's own sense-of-time stamps
/// (session origin, save marks, the key→px input receipt) — now reads
/// `App::clock.now()` through this seam instead.
///
/// WHY a seam and not the free function: the headless capture harness has NO
/// clock and renders a single SETTLED frame, so live-only bug classes
/// (redraw-scheduling gaps, cache invalidation that only misbehaves ACROSS
/// frames) are invisible to it. Routing the whole scheduling/animation path
/// through one injectable [`Clock`] is the structural precondition for a future
/// deterministic clock the harness can STEP frame-by-frame, making those
/// classes capturable — without any world/theme code branching on time. The
/// `app::clock_law` grep-test fences the `app` module against a raw
/// `Instant::now` re-appearing outside two documented perf-measurement
/// exceptions.
///
/// What this is deliberately NOT: the wall-clock STAMP path (file mtimes,
/// history/`SystemTime` epoch reads) keeps [`system_now`]; and genuine
/// real-WORK MEASUREMENT — the `--bench-*` harnesses, `--soak-gpu` (which
/// injects its own start instant), the GPU-stage perf timing, the
/// crash-log/probe diagnostics — reads the raw monotonic clock by necessity,
/// because a virtual clock would report a fictional duration for real elapsed
/// work. Those live outside the `app` module and outside this law's scope.
pub trait Clock {
    /// Monotonic "now". [`RealClock`] forwards to the platform `Instant::now()`.
    fn now(&self) -> Instant;
}

/// The shipped clock: a zero-sized PURE pass-through to the platform monotonic
/// clock, so the live app's timing is byte-for-byte what it was before the seam
/// existed (the whole point of the MINIMUM landing — a `RealClock`-backed App
/// captures identically). A deterministic sibling clock can slot in behind the
/// same `Box<dyn Clock>` field without any consumer changing.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealClock;

impl Clock for RealClock {
    #[inline]
    fn now(&self) -> Instant {
        Instant::now()
    }
}
