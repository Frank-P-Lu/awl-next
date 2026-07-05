//! src/about.rs — the summoned ABOUT card: state only (rendering lives in
//! `render/chrome.rs`, which reuses the HUD's float-card pipeline verbatim).
//!
//! A calm, centered info card — "Awl", the crate version, the active theme
//! world's name, and a closing ornament (the world's own dash fleuron, the
//! same glyph a `---` rule renders as) — summoned via Cmd-P → "About" (and,
//! on macOS, the menu bar's App → "About Awl" item). Unlike the HELD stats HUD
//! (`hud.rs`), this is NOT a hold: it OPENS and stays open until dismissed by
//! ANY key or mouse click — the modal-summon pattern the navigation overlay
//! already uses, just with no content to navigate.
//!
//! **Why this exists at all (not muda's predefined About dialog):** see
//! `menu.rs`'s module doc for the full mechanism, but in short — the OS About
//! panel is genuinely OS chrome with no correctness reason to route UNLESS you
//! also want it to look and feel like the rest of awl (calm, one warm accent,
//! `base_300` float card) rather than a stock AppKit dialog. Routing it also
//! means it works identically on Linux (no native menu bar there at all) and is
//! `--keys`/sidecar drivable like everything else in this app.
//!
//! One process-global mirrors the `debug`/`focus`/`hud` pattern:
//!   * `ABOUT_OPEN` — whether the card is drawn (DEFAULT OFF / closed).
//!
//! Dismissal is intentionally NOT scoped to Esc: `actions::apply_core` closes
//! it on the very first key it sees while open (any key, consumed, no other
//! effect — see its top-of-function intercept), and the live `App` closes it
//! on any mouse press too (`app/input.rs`). This is deliberately looser than
//! the navigation overlay's Esc/Enter contract: an about card has nothing to
//! navigate, so any dismissal gesture is equally correct.
//!
//! **Why `apply_core` itself takes [`test_lock`] (not just the tests that flip
//! the flag):** unlike `page`/`caret`/`focus`/`debug`/`hud` — globals a test
//! only races if IT ALSO reads/writes them — `about_open()` is checked at the
//! very TOP of `apply_core`, UNCONDITIONALLY, for every action. That makes the
//! about global a hazard for tests that have never heard of `about.rs`: if
//! `about_opens_and_any_key_dismisses_it` (or the `is_motion` completeness
//! sweep, the only two tests that ever drive `Action::About`) sets the flag
//! true on one thread, ANY other concurrently-running test's own unrelated
//! `apply_core` call can walk straight into the top-of-function intercept,
//! silently swallow its own action, and return `Effect::None` instead of
//! whatever it expected — confirmed live (`boundary_motions_bump_only_when_
//! blocked` / `blocked_motions_arm_recoil_away_from_the_wall` failing under
//! parallel `cargo test`, traced to exactly this). Holding [`test_lock`] on
//! EVERY reader (i.e. `about::TEST_LOCK`-style per-test discipline) can't
//! close that gap — a test that doesn't know to ask for the lock can't be
//! made to. So `apply_core` acquires [`test_lock`] itself, for the WHOLE
//! call, under `cfg(test)` — mirroring `page.rs`'s WRITER-side structural fix,
//! but applied at the one call site that matters instead of scattering it
//! across every test. Reentrant per thread (a test that already holds the
//! lock across its own read/write window, e.g. via `Action::About`, nests for
//! free), so it can never self-deadlock.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the About card is drawn. DEFAULT OFF (closed) — summoned only via
/// the palette "About" command / macOS menu "About Awl" item.
static ABOUT_OPEN: AtomicBool = AtomicBool::new(false);

/// True when the About card is currently summoned.
pub fn about_open() -> bool {
    ABOUT_OPEN.load(Ordering::Relaxed)
}

/// Open or close the card explicitly.
pub fn set_open(open: bool) {
    ABOUT_OPEN.store(open, Ordering::Relaxed);
}

/// The raw mutex behind [`test_lock`] — never touched directly outside this
/// module; see the module doc for why `apply_core` itself is a lock holder.
#[cfg(test)]
static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
thread_local! {
    /// True while THIS thread holds [`TEST_MUTEX`] via an [`AboutTestGuard`] —
    /// the reentrancy key for [`test_lock`], mirroring `page::HOLDS_PAGE_LOCK`.
    static HOLDS_ABOUT_LOCK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Guard for [`test_lock`]. `inner: None` is the reentrant (already-held) case:
/// dropping it releases nothing; the outermost guard clears the thread flag.
#[cfg(test)]
pub(crate) struct AboutTestGuard {
    inner: Option<std::sync::MutexGuard<'static, ()>>,
}

#[cfg(test)]
impl Drop for AboutTestGuard {
    fn drop(&mut self) {
        if self.inner.is_some() {
            HOLDS_ABOUT_LOCK.with(|h| h.set(false));
        }
    }
}

/// Acquire the about-global test lock: blocks until free, absorbs poison, and
/// is REENTRANT per thread (a lock-holding test calling into `apply_core` —
/// which itself takes this lock — nests for free instead of deadlocking). The
/// ONLY door to the mutex; see the module doc for why `apply_core` is itself
/// a caller, not just the tests that explicitly flip the flag.
#[cfg(test)]
pub(crate) fn test_lock() -> AboutTestGuard {
    if HOLDS_ABOUT_LOCK.with(|h| h.get()) {
        return AboutTestGuard { inner: None };
    }
    let guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    HOLDS_ABOUT_LOCK.with(|h| h.set(true));
    AboutTestGuard { inner: Some(guard) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_closed() {
        let _g = test_lock();
        set_open(false);
        assert!(!about_open(), "the About card is closed by default");
    }

    #[test]
    fn set_open_drives_the_flag() {
        let _g = test_lock();
        set_open(false);
        set_open(true);
        assert!(about_open());
        set_open(false);
        assert!(!about_open());
    }

    #[test]
    fn test_lock_is_reentrant_and_the_outermost_guard_owns_the_release() {
        let g1 = test_lock();
        assert!(HOLDS_ABOUT_LOCK.with(|h| h.get()));
        let g2 = test_lock(); // nested acquire on the SAME thread: must not deadlock
        drop(g2);
        assert!(HOLDS_ABOUT_LOCK.with(|h| h.get()), "outer guard still held after inner drops");
        drop(g1);
        assert!(!HOLDS_ABOUT_LOCK.with(|h| h.get()), "outermost drop releases the flag");
    }
}
