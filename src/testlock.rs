//! `testlock` — THE one process-wide reentrant test-serialization guard.
//!
//! awl keeps a fistful of process-GLOBAL state that tests read and write: the
//! active THEME, PAGE mode/measure, the caret look, the debug / hud / peek /
//! menubar flags, the spell / nits / markdown / outline / typewriter /
//! frontmatter sticky globals, the summoned about / lifetime CARD flags, and
//! the swappable fs / trash / socket-dir / cwd backends. `cargo test` runs in
//! parallel, so two tests touching the same global race — and when each global
//! was guarded by its OWN `Mutex` with a documented acquire ORDER, a test that
//! took two of them in the wrong order could ABBA-deadlock (a real 3-way hang
//! lived in the about↔lifetime pair; the theme↔page pair produced the
//! wash-cache/geometry flake).
//!
//! The cure is STRUCTURAL: ONE lock for all of it. Every test — and every
//! `cfg(test)` global WRITER: the `page` measure setters, `apply_core`'s
//! card-dismissal intercepts, `fs::FsGuard` / `fs::CwdGuard` / `assets`'s
//! `with_trash`, `daemon`'s socket-dir gate — acquires [`serial`]. With a
//! single lock there is no acquire order left to invert, so the ABBA class is
//! UNREPRESENTABLE.
//!
//! The one subtlety a single lock forces is REENTRANCY: a test holds the guard
//! across its whole window and then calls a writer (or drives `apply_core`,
//! which acquires it too) on the SAME thread — so acquisition is keyed on a
//! thread-local "this thread already holds it" flag, and a nested acquire
//! returns a no-op guard instead of self-deadlocking. Only the OUTERMOST guard
//! owns the release. Poison is absorbed (`into_inner`), mirroring the old
//! raw-mutex convention: a failed assertion in one test must not cascade a
//! poisoned-lock panic into every later one.
//!
//! The cost is COARSER parallelism — every global-touching test now serializes
//! against every other — accepted deliberately (the pure, global-free unit
//! tests still run fully parallel). This is the single owner that replaced the
//! old `theme::TEST_LOCK` / `fs::TEST_LOCK` / `page::test_lock` /
//! `about`+`lifetime` composite / caret / debug / hud / … family.

use std::cell::Cell;
use std::sync::{Mutex, MutexGuard};

/// The one mutex behind [`serial`]. Never touched outside this module.
static TEST_MUTEX: Mutex<()> = Mutex::new(());

thread_local! {
    /// True while THIS thread holds [`TEST_MUTEX`] via the OUTERMOST live
    /// [`SerialGuard`] — the reentrancy key.
    static HELD: Cell<bool> = const { Cell::new(false) };
}

/// The guard [`serial`] returns. `inner: None` is the reentrant (already-held)
/// case: dropping it releases nothing; only the outermost guard clears the
/// thread flag and unlocks the mutex.
pub(crate) struct SerialGuard {
    inner: Option<MutexGuard<'static, ()>>,
}

impl Drop for SerialGuard {
    fn drop(&mut self) {
        if self.inner.is_some() {
            HELD.with(|h| h.set(false));
        }
    }
}

/// Acquire THE process-wide test-serialization lock: blocks until free, absorbs
/// poison, and is REENTRANT per thread (a nested acquire on a thread that
/// already holds it returns a no-op guard instead of self-deadlocking). The
/// ONLY door to the mutex.
pub(crate) fn serial() -> SerialGuard {
    if HELD.with(|h| h.get()) {
        return SerialGuard { inner: None };
    }
    let guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    HELD.with(|h| h.set(true));
    SerialGuard { inner: Some(guard) }
}

/// True iff THIS thread currently holds the guard (via a live [`serial`]
/// guard). For the law tests below.
pub(crate) fn currently_held() -> bool {
    HELD.with(|h| h.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_is_reentrant_and_the_outermost_guard_owns_the_release() {
        let g1 = serial();
        assert!(currently_held(), "the outer guard sets the thread flag");
        let g2 = serial(); // nested acquire on the SAME thread: must not deadlock
        drop(g2);
        assert!(
            currently_held(),
            "dropping the inner (no-op) guard must NOT release the outer hold"
        );
        drop(g1);
        assert!(!currently_held(), "the outermost drop clears the flag");
    }

    #[test]
    fn nested_guards_from_many_former_lock_sites_share_one_underlying_lock() {
        // The collapse's core promise: what used to be a theme lock + a page
        // lock + a caret lock (three DIFFERENT mutexes, taken in a fixed order)
        // is now ONE reentrant guard. Acquiring it three deep on one thread must
        // never deadlock and the outermost must own the release.
        let a = serial();
        let b = serial();
        let c = serial();
        assert!(currently_held());
        drop(c);
        drop(b);
        assert!(currently_held(), "still held while the outermost lives");
        drop(a);
        assert!(!currently_held(), "released once the outermost drops");
    }

    #[test]
    fn a_writer_thread_blocks_until_the_guard_is_released() {
        // THE mutual-exclusion law (the visual_* / wash-cache flake fix rests on
        // it): a thread that does not hold the guard cannot proceed past its own
        // acquire while another thread holds it.
        let g = serial();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer = {
            let done = done.clone();
            std::thread::spawn(move || {
                let _held = serial();
                done.store(true, std::sync::atomic::Ordering::SeqCst);
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            !done.load(std::sync::atomic::Ordering::SeqCst),
            "the other thread must still be blocked while we hold the guard"
        );
        drop(g);
        writer.join().unwrap();
        assert!(
            done.load(std::sync::atomic::Ordering::SeqCst),
            "the blocked thread proceeds once the guard is released"
        );
    }

    #[test]
    fn a_global_writer_nested_under_the_guard_never_self_deadlocks() {
        // A page-global WRITER (`set_measure`) acquires the guard INTERNALLY under
        // `cfg(test)`. A test that already holds the guard and then drives such a
        // writer must nest for free (not self-deadlock), and the write must land.
        let _g = serial();
        crate::page::set_measure(33);
        assert_eq!(crate::page::measure(), 33, "the nested writer's write lands");
        crate::page::set_measure(crate::page::DEFAULT_MEASURE); // leave as found
    }

    #[test]
    fn an_inner_guard_drop_never_releases_the_outer_hold_for_a_following_writer() {
        // Models `apply_core` (and any writer): while a test holds the guard,
        // a nested acquire+drop must NOT release the test's outer hold, so a
        // FOLLOWING nested writer still serializes under the same outer window.
        let outer = serial();
        {
            let _inner = serial();
        }
        assert!(currently_held(), "the outer hold survives an inner acquire+drop");
        crate::page::set_measure(44);
        assert_eq!(crate::page::measure(), 44);
        crate::page::set_measure(crate::page::DEFAULT_MEASURE);
        drop(outer);
        assert!(!currently_held());
    }
}
