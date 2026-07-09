//! src/lifetime.rs — the summoned LIFETIME STATS card: state only (rendering
//! lives in `render/chrome/hud.rs`, which reuses the HUD's float-card pipeline
//! verbatim, exactly like the About card does).
//!
//! A calm, centered personal ODOMETER — characters typed, time writing, files
//! touched, caret travel (as a whimsical desk distance), and the most-lived-in
//! theme world ("your world") — summoned via Cmd-P → "Lifetime stats". These are
//! the LIFETIME figures that used to ride along at the bottom of the HELD stats
//! HUD (Cmd-I); the HUD was trimmed to its per-doc writer figures and the odometer
//! moved here, to a read-it-don't-peek summoned card of its own.
//!
//! Unlike the HELD stats HUD (`hud.rs`), this is NOT a hold: it OPENS and stays
//! open until dismissed by ANY key or mouse click — the SAME modal-summon pattern
//! the About card (`about.rs`) uses, just with different content. It reuses the
//! About wiring verbatim across every seam (open-flag global, the `apply_core`
//! top-of-function any-key dismissal, the live App's any-mouse-press dismissal).
//!
//! One process-global mirrors the `about`/`debug`/`focus`/`hud` pattern:
//!   * `LIFETIME_OPEN` — whether the card is drawn (DEFAULT OFF / closed).
//!
//! Determinism: the figures are LIVE-ONLY — the live App pushes its persisted
//! [`crate::stats::Stats`] snapshot into the pipeline every `sync_view`; a headless
//! capture never does, so every odometer row folds to the fixed
//! [`crate::hud::PLACEHOLDER`] and a `--lifetime` capture is byte-stable across
//! machines. The open-flag defaults false, so a default `--screenshot` is
//! byte-identical.
//!
//! **Why `apply_core` itself takes [`test_lock`]:** identical to the reasoning in
//! `about.rs`'s module doc — `lifetime_open()` is checked at the very TOP of
//! `apply_core`, UNCONDITIONALLY, for every action (the any-key dismissal), so a
//! test that has never heard of this module could otherwise have its own unrelated
//! `apply_core` call walk into the dismissal intercept and silently swallow its
//! action. `apply_core` acquires [`test_lock`] itself under `cfg(test)`, reentrant
//! per thread, so it can never self-deadlock.
//!
//! **Lock order — about ⊂ lifetime, both OUTSIDE the page chain:** [`test_lock`]
//! grabs `about` FIRST itself (so no caller holds `lifetime` without `about` — the
//! structural cure for the about↔lifetime ABBA). But neither is chained to `page`:
//! `apply_core` RELEASES both right after its top-of-function intercepts, before it
//! can reach a page writer (see `actions::apply_core`), so `page` is never taken
//! while about/lifetime are held. That keeps this two-lock family entirely separate
//! from the theme → fs → page chain — a page-holding test entering `apply_core`
//! can never ABBA it, so `page::test_lock` needs no knowledge of about/lifetime.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the Lifetime stats card is drawn. DEFAULT OFF (closed) — summoned only
/// via the palette "Lifetime stats" command / the `--lifetime` capture flag.
static LIFETIME_OPEN: AtomicBool = AtomicBool::new(false);

/// True when the Lifetime stats card is currently summoned.
pub fn lifetime_open() -> bool {
    LIFETIME_OPEN.load(Ordering::Relaxed)
}

/// Open or close the card explicitly.
pub fn set_open(open: bool) {
    LIFETIME_OPEN.store(open, Ordering::Relaxed);
}

/// The raw mutex behind [`test_lock`] — never touched directly outside this
/// module; see the module doc (and `about.rs`) for why `apply_core` itself is a
/// lock holder.
#[cfg(test)]
static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
thread_local! {
    /// True while THIS thread holds [`TEST_MUTEX`] via a [`LifetimeTestGuard`] —
    /// the reentrancy key for [`test_lock`], mirroring `about::HOLDS_ABOUT_LOCK`.
    static HOLDS_LIFETIME_LOCK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Guard for [`test_lock`]. `inner: None` is the reentrant (already-held) case:
/// dropping it releases nothing; the outermost guard clears the thread flag.
///
/// It also OWNS the [`crate::about::AboutTestGuard`] that [`test_lock`] acquires
/// FIRST (the canonical about → lifetime order). `_about` is declared AFTER
/// `inner`, so field-drop order releases the lifetime mutex BEFORE the about one
/// — the exact reverse of the acquire, the tidy nesting a lock pair wants.
#[cfg(test)]
pub(crate) struct LifetimeTestGuard {
    inner: Option<std::sync::MutexGuard<'static, ()>>,
    _about: crate::about::AboutTestGuard,
}

#[cfg(test)]
impl Drop for LifetimeTestGuard {
    fn drop(&mut self) {
        if self.inner.is_some() {
            HOLDS_LIFETIME_LOCK.with(|h| h.set(false));
        }
    }
}

/// Acquire the lifetime-global test lock: blocks until free, absorbs poison, and
/// is REENTRANT per thread (a lock-holding test calling into `apply_core` — which
/// itself takes this lock — nests for free instead of deadlocking). The ONLY door
/// to the mutex; see the module doc (and `about.rs`) for why `apply_core` is
/// itself a caller.
///
/// **Composite — grabs `about` FIRST.** The canonical order is about → lifetime
/// (the order `apply_core` takes them at its top). This door acquires the `about`
/// lock BEFORE the lifetime one, unconditionally, and hands it back inside the
/// returned guard — so no caller can ever hold the lifetime lock WITHOUT already
/// holding `about`, and the two can never be acquired inverted across threads. The
/// ABBA that would otherwise arise (a test holding `lifetime` while `apply_core`
/// waits on it, `apply_core` holding `about` while that test waits on it) is
/// STRUCTURALLY impossible, not merely avoided by per-test convention. Both grabs
/// are per-thread reentrant, so `apply_core`'s own about-then-lifetime nests free.
#[cfg(test)]
pub(crate) fn test_lock() -> LifetimeTestGuard {
    // `about` first — see the doc above; reentrant, so a thread already holding it
    // (e.g. `apply_core`, which took it one line before it takes ours) nests free.
    let about = crate::about::test_lock();
    if HOLDS_LIFETIME_LOCK.with(|h| h.get()) {
        return LifetimeTestGuard { inner: None, _about: about };
    }
    let guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    HOLDS_LIFETIME_LOCK.with(|h| h.set(true));
    LifetimeTestGuard { inner: Some(guard), _about: about }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_closed() {
        let _g = test_lock();
        set_open(false);
        assert!(!lifetime_open(), "the Lifetime stats card is closed by default");
    }

    #[test]
    fn set_open_drives_the_flag() {
        let _g = test_lock();
        set_open(false);
        set_open(true);
        assert!(lifetime_open());
        set_open(false);
        assert!(!lifetime_open());
    }

    #[test]
    fn test_lock_is_reentrant_and_the_outermost_guard_owns_the_release() {
        let g1 = test_lock();
        assert!(HOLDS_LIFETIME_LOCK.with(|h| h.get()));
        let g2 = test_lock(); // nested acquire on the SAME thread: must not deadlock
        drop(g2);
        assert!(HOLDS_LIFETIME_LOCK.with(|h| h.get()), "outer guard still held after inner drops");
        drop(g1);
        assert!(!HOLDS_LIFETIME_LOCK.with(|h| h.get()), "outermost drop releases the flag");
    }

    /// The composite guard's contract — the structural cure for the about↔lifetime
    /// ABBA: acquiring the lifetime lock ALSO acquires `about` (grabbed FIRST, the
    /// canonical about → lifetime order `apply_core` takes), and dropping releases
    /// both. Because holding `lifetime` implies already holding `about`, the two
    /// locks can never be acquired inverted on two threads, so the cross-thread
    /// cycle is impossible by construction rather than by per-test discipline.
    #[test]
    fn test_lock_composite_holds_about_first_and_releases_both() {
        assert!(!crate::about::currently_held(), "about lock free before we acquire lifetime");
        let g = test_lock();
        assert!(HOLDS_LIFETIME_LOCK.with(|h| h.get()), "lifetime lock held");
        assert!(crate::about::currently_held(), "acquiring lifetime composite-holds about");
        drop(g);
        assert!(!HOLDS_LIFETIME_LOCK.with(|h| h.get()), "lifetime released");
        assert!(!crate::about::currently_held(), "dropping the lifetime guard releases about too");
    }
}
