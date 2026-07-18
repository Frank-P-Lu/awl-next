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

/// A DETERMINISTIC virtual clock: the sibling [`Clock`] the [`RealClock`] doc
/// foretold, slotting behind the same `Box<dyn Clock>` field with zero consumer
/// changes. Time NEVER passes on its own — it advances ONLY by the fixed steps a
/// driver injects ([`advance`](Self::advance)) — so the whole scheduling /
/// animation path an `App` runs (`app::schedule`'s debounce/settle deadlines, the
/// ambient tick, the App's sense-of-time stamps) can be STEPPED frame-by-frame
/// under the headless harness, making the live-only bug classes (a redraw-
/// scheduling gap, a debounce firing early/late, an animation phase not advancing)
/// CAPTURABLE. That is the whole reason the [`Clock`] seam exists (see its doc).
///
/// DETERMINISM. The one real read is the arbitrary monotonic `base` captured once
/// at construction — a scaffold, never observed directly. Every scheduling
/// consumer compares clock-derived values to EACH OTHER (`now() >= stamp +
/// window`, `now - last` for a spring dt), so the `base` term CANCELS: two runs
/// with different bases take byte-identical decisions and render byte-identical
/// frames, because only the injected DELTAS matter. `advance` never reads a wall
/// clock and no path consults randomness. (Living in `clock.rs`, not the `app`
/// module, this construction-time `Instant::now()` is outside `app::clock_law`'s
/// scope by the same rule that lets [`RealClock`] read it here — and it is
/// deterministic-by-delta, so it needs no allow-list bump.)
///
/// The inner elapsed count is an `Arc<AtomicU64>` of nanoseconds so a CLONE shares
/// the SAME timeline: the frame loop keeps one handle to `advance`, and hands a
/// clone into `App::clock`, so the App reads exactly the time the driver stepped.
#[cfg(any(test, not(target_arch = "wasm32")))]
#[derive(Clone)]
pub struct VirtualClock {
    /// The arbitrary monotonic origin, captured ONCE (the only real read); every
    /// `now()` is `base + elapsed`, and every consumer uses deltas, so `base`
    /// cancels and the value is deterministic-by-delta (see the type doc).
    base: Instant,
    /// Virtual nanoseconds elapsed since `base`, shared across clones so the
    /// driver's `advance` is visible through the clone the App reads.
    elapsed_nanos: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

#[cfg(any(test, not(target_arch = "wasm32")))]
impl VirtualClock {
    /// A fresh virtual clock at virtual time zero. The base is captured here (the
    /// only real-clock read); it is never observed directly (see the type doc's
    /// determinism note).
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
            elapsed_nanos: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Advance virtual time by `dt`. Deterministic — never reads a wall clock.
    /// Visible immediately through every clone (they share the inner count).
    pub fn advance(&self, dt: std::time::Duration) {
        self.elapsed_nanos
            .fetch_add(dt.as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
    }

    /// Convenience: advance virtual time by `ms` milliseconds (the frame loop's
    /// per-frame step).
    pub fn advance_ms(&self, ms: u64) {
        self.advance(std::time::Duration::from_millis(ms));
    }

    /// Virtual time elapsed since construction — a pure delta (base-independent),
    /// so it is the deterministic quantity to STAMP a frame-loop sidecar with.
    pub fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::from_nanos(self.elapsed_nanos.load(std::sync::atomic::Ordering::Relaxed))
    }
}

#[cfg(any(test, not(target_arch = "wasm32")))]
impl Default for VirtualClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, not(target_arch = "wasm32")))]
impl Clock for VirtualClock {
    #[inline]
    fn now(&self) -> Instant {
        self.base + self.elapsed()
    }
}

#[cfg(test)]
mod virtual_clock_tests {
    use super::*;

    #[test]
    fn starts_at_zero_and_advances_only_when_stepped() {
        let clock = VirtualClock::new();
        let t0 = clock.now();
        // No wall time passes on its own: a re-read without an advance is identical.
        assert_eq!(clock.now(), t0, "virtual time must not pass on its own");
        assert_eq!(clock.elapsed(), std::time::Duration::ZERO);

        clock.advance_ms(150);
        assert_eq!(clock.elapsed(), std::time::Duration::from_millis(150));
        assert_eq!(clock.now(), t0 + std::time::Duration::from_millis(150));

        clock.advance(std::time::Duration::from_millis(350));
        assert_eq!(clock.elapsed(), std::time::Duration::from_millis(500));
    }

    #[test]
    fn a_clone_shares_the_same_timeline() {
        // The frame loop keeps one handle and hands a clone to `App::clock`; a step
        // on either handle must be visible through the other (shared inner count).
        let driver = VirtualClock::new();
        let injected = driver.clone();
        driver.advance_ms(500);
        assert_eq!(injected.elapsed(), std::time::Duration::from_millis(500));
        assert_eq!(injected.now(), driver.now());
    }

    #[test]
    fn deltas_are_base_independent() {
        // Two clocks with independent bases (the arbitrary real read) reach the same
        // DECISION for the same injected steps — the determinism guarantee: a
        // deadline of `stamp + window` vs `now` depends only on the deltas.
        let window = std::time::Duration::from_millis(500);
        let a = VirtualClock::new();
        let b = VirtualClock::new();
        let stamp_a = a.now();
        let stamp_b = b.now();
        for _ in 0..4 {
            a.advance_ms(100);
            b.advance_ms(100);
        }
        // Both at t=400 < 500: neither deadline has passed.
        assert_eq!(a.now() >= stamp_a + window, b.now() >= stamp_b + window);
        assert!(!(a.now() >= stamp_a + window));
        a.advance_ms(100);
        b.advance_ms(100);
        // Both at t=500 == deadline: both fire, regardless of their differing bases.
        assert_eq!(a.now() >= stamp_a + window, b.now() >= stamp_b + window);
        assert!(a.now() >= stamp_a + window);
    }
}
