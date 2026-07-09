//! src/page.rs — PAGE MODE state.
//!
//! Page mode lays the document out as a CENTERED writing column (Typora-style)
//! with a maximum MEASURE in characters, so prose never stretches edge-to-edge
//! on a wide / fullscreen window. The extra window width becomes MARGINS on both
//! sides, which the [`crate::background`] shader paints with a per-world gradient
//! so the calm flat page reads as a clean shape FLOATING on a styled ground.
//!
//! Two process-globals mirror the `theme`/`caret` pattern so the runtime toggle
//! (`C-x w`), the command palette ("Toggle page mode"), and the headless flags
//! (`--page on|off`, `--measure N`) all write the same place without threading a
//! config through the pipeline:
//!   * `PAGE_ON`  — whether the centered column is active (DEFAULT ON).
//!   * `MEASURE`  — the column's maximum width in characters (DEFAULT 80).
//!
//! The render pipeline reads these each frame via [`page_on`] / [`measure`] to
//! derive the column's pixel left + width (see `TextPipeline::column_left` /
//! `column_width`), so flipping either re-wraps + re-centers the text.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Default PROSE measure (column width in characters). ~70 sits squarely in
/// Butterick's 45–90 comfort band (≈2.5 lowercase alphabets); 80 read a touch
/// wide (just past 3 alphabets). Sticky + adjustable via Page wider/narrower.
/// See [`PageClass`]: this is the `Prose` class's own built-in default —
/// `page_width_prose` in config (`crate::config::Config::page_width_prose`).
pub const DEFAULT_MEASURE: usize = 70;

/// Default CODE measure (column width in characters) — rustfmt's own
/// `max_width` convention (the settled call: code reads comfortably wider
/// than prose's 70-char comfort band, and already wraps at this width by
/// convention). See [`PageClass::Code`] — `page_width_code` in config
/// (`crate::config::Config::page_width_code`).
pub const DEFAULT_MEASURE_CODE: usize = 100;

/// Which STICKY page-width MEASURE a buffer draws its column width from — the
/// 70-char prose comfort measure is a PROSE number (Butterick's line-length
/// band), and code wants its own, wider convention. Two independent config
/// keys (`page_width_prose` / `page_width_code`, `crate::config::Config`) each
/// persist their class's override; [`Self::default_measure`] is the built-in
/// fallback when a class has none.
///
/// THE ONE CLASSIFIER: [`Self::of_syntax`] — a recognized CODE language means
/// `Code`; `None` (markdown, the no-path scratch/quick-note surface, or an
/// unrecognized plain-text file like `.txt`/`.env`) means `Prose`. Both
/// `crate::buffer::Buffer::page_class` (the live/headless buffer) and
/// `crate::render::TextPipeline::page_class` (the sidecar, driven by the
/// pipeline's own shaped `syn_lang`) delegate here, so the two can never
/// disagree about which class a document belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PageClass {
    /// Markdown, the no-path scratch/quick-note surface, or an unrecognized
    /// plain-text file — everything [`crate::syntax::Lang::from_path`] does
    /// NOT recognize as a code language.
    Prose,
    /// A recognized CODE file (`Buffer::syntax_lang().is_some()`).
    Code,
}

impl PageClass {
    /// THE classifier, driven by a resolved (or absent) syntax-highlighting
    /// language — mirrors `Buffer::syntax_lang`'s own code/not-code gate
    /// exactly, so page-width class and syntax highlighting can never
    /// disagree about what counts as "code".
    pub fn of_syntax(syn_lang: Option<crate::syntax::Lang>) -> Self {
        match syn_lang {
            Some(_) => PageClass::Code,
            None => PageClass::Prose,
        }
    }

    /// Classify a FILE PATH the same way, for the ONE call site that must
    /// decide a class before any `Buffer` exists: the initial sticky-measure
    /// apply at launch (`crate::config::Config::apply_sticky_globals`,
    /// called from `main::args` with the launch file argument). `None` (no
    /// file / a bare launch) is always `Prose`, matching `Buffer::is_markdown`'s
    /// own no-path default.
    pub fn of_path(path: Option<&std::path::Path>) -> Self {
        Self::of_syntax(path.and_then(crate::syntax::Lang::from_path))
    }

    /// This class's own BUILT-IN default measure, used when its config key
    /// (`page_width_prose`/`page_width_code`) is unset — [`DEFAULT_MEASURE`]
    /// for `Prose`, [`DEFAULT_MEASURE_CODE`] for `Code`.
    pub fn default_measure(self) -> usize {
        match self {
            PageClass::Prose => DEFAULT_MEASURE,
            PageClass::Code => DEFAULT_MEASURE_CODE,
        }
    }
}

/// The step (in characters) the "Page wider" / "Page narrower" commands move the
/// measure by. A calm nudge — a few keypresses spans the usable band.
pub const MEASURE_STEP: usize = 4;

/// The usable band the wider/narrower commands stay within. The floor keeps the
/// column from collapsing to an unreadable sliver; the ceiling keeps prose from
/// stretching back toward edge-to-edge (which is the page toggle's job, not the
/// width's). Hand-edited config values are still honoured verbatim (only the
/// COMMANDS clamp), and the render column additionally caps to the window.
pub const MIN_MEASURE: usize = 20;
pub const MAX_MEASURE: usize = 140;

/// Whether page mode (the centered capped column) is active. DEFAULT ON: the app
/// opens with the calm centered column; the toggle drops to edge-to-edge.
static PAGE_ON: AtomicBool = AtomicBool::new(true);

/// The column's maximum width in CHARACTERS. The pixel width is this times the
/// (zoomed) glyph advance, clamped to the window. Tunable via `--measure N` and
/// reachable as a setting; the render pipeline reads it each frame.
static MEASURE: AtomicUsize = AtomicUsize::new(DEFAULT_MEASURE);

/// True when the centered column is active.
pub fn page_on() -> bool {
    PAGE_ON.load(Ordering::Relaxed)
}

/// Set page mode on/off explicitly (the `--page on|off` flag, a settings write).
pub fn set_page_on(on: bool) {
    // Writers self-serialize under test — see [`test_lock`]'s module note.
    #[cfg(test)]
    let _g = test_lock();
    PAGE_ON.store(on, Ordering::Relaxed);
}

/// Flip page mode and return the now-active state (the `C-x w` chord + palette).
pub fn toggle() -> bool {
    // The guard spans the whole read-modify-write, not just the store.
    #[cfg(test)]
    let _g = test_lock();
    let next = !page_on();
    PAGE_ON.store(next, Ordering::Relaxed);
    next
}

/// The column measure (characters). Floored at 1 so the column never collapses.
pub fn measure() -> usize {
    MEASURE.load(Ordering::Relaxed).max(1)
}

/// Set the column measure in characters (the `--measure N` flag, a setting).
/// Setting a measure does NOT itself enable page mode; callers that want the
/// column visible also call [`set_page_on`].
pub fn set_measure(chars: usize) {
    // Writers self-serialize under test — see [`test_lock`]'s module note. This
    // is the seam that un-flaked `run::tests::visual_*`: transitive writers
    // (`replay_keys`' Goto measure resync, `App::sync_page_measure`,
    // `apply_sticky_globals`) all land here, so no test can interleave a write
    // into another test's locked read window.
    #[cfg(test)]
    let _g = test_lock();
    MEASURE.store(chars.max(1), Ordering::Relaxed);
}

/// Widen the page by one [`MEASURE_STEP`] (the "Page wider" command), clamped to
/// [`MAX_MEASURE`], and return the now-active measure so the caller can persist it.
/// Zoom-independent: this changes the PAGE geometry (more chars per line at the same
/// glyph size), the settable counterpart to zoom's glyph-only scaling.
pub fn widen() -> usize {
    // The guard spans the whole read-modify-write; the nested `set_measure`
    // acquire is the reentrant no-op case.
    #[cfg(test)]
    let _g = test_lock();
    let next = (measure() + MEASURE_STEP).min(MAX_MEASURE);
    set_measure(next);
    next
}

/// Narrow the page by one [`MEASURE_STEP`] (the "Page narrower" command), clamped to
/// [`MIN_MEASURE`], and return the now-active measure so the caller can persist it.
pub fn narrow() -> usize {
    // See `widen` — same whole-RMW guard, same reentrant nested acquire.
    #[cfg(test)]
    let _g = test_lock();
    let next = measure().saturating_sub(MEASURE_STEP).max(MIN_MEASURE);
    set_measure(next);
    next
}

/// Serializes EVERY test that reads or writes the page globals, ACROSS modules.
/// Page mode is a process-wide `AtomicBool`/`AtomicUsize`, so a `render` test
/// reading `column_width()` (which folds `page_on()`/`measure()`) must not race
/// a page test flipping them mid-shape, or the two diverge non-deterministically.
///
/// ONE OWNER, TWO HALVES (the page-width-split flake fix, 2026-07):
///  * READERS — a test whose assertions depend on the globals holding a value
///    across a window (set measure, shape, assert) takes [`test_lock`] for that
///    whole window, exactly like the old raw `TEST_LOCK` mutex this replaced.
///  * WRITERS — every write path ([`set_measure`], [`set_page_on`], [`toggle`],
///    [`widen`], [`narrow`]) acquires the SAME lock internally under
///    `cfg(test)`, so a test that mutates the globals only transitively (a
///    `replay_keys` Goto hitting the per-kind measure resync, the live App's
///    `sync_page_measure`, `apply_sticky_globals`, an apply-seam page action)
///    is serialized STRUCTURALLY — no test can forget a lock it never knew it
///    needed. This is what un-flaked `run::tests::visual_*`: the multi-buffer
///    Goto replay tests became page-global writers when the prose/code
///    page-width split taught `replay_keys` to re-apply the measure on a
///    buffer switch, and they (correctly, at the time) never held the page
///    lock themselves.
///
/// REENTRANT by thread (the one subtlety): a lock-holding test that calls a
/// writer (every page test does) must not self-deadlock, so acquisition is
/// keyed on a thread-local "this thread already holds it" flag — a nested
/// acquire returns a no-op guard. Poisoning is absorbed (`into_inner`), same
/// as the old raw-mutex convention: a failed assertion in one test must not
/// cascade into every later one.
///
/// LOCK ORDER across suites (page is always LAST): `theme::TEST_LOCK` →
/// fs-side locks (`fs::TEST_LOCK` AND `fs::FsGuard::install`'s seam mutex) →
/// `page::test_lock()`. The render tests hold theme→page, the replay/App tests
/// hold fs→page; nothing may acquire theme/fs while holding the page lock, or the
/// internal writer acquire becomes an ABBA deadlock — an fs-holding test's
/// `load_path` writes the measure (waits on page) while a page-holding test
/// installs an `FsGuard` (waits on fs). Caught live, once.
///
/// The about/lifetime locks are deliberately NOT in this chain: `apply_core` holds
/// them only across its top-of-function card-dismissal intercepts and RELEASES them
/// before any page-writer arm (see `actions::apply_core`), so `page` is never
/// acquired while `about`/`lifetime` are held — a page-holding test that then
/// enters `apply_core` can never ABBA it. `about` sits above `lifetime` (the
/// `lifetime::test_lock` composite), but both sit OUTSIDE the page chain entirely.
#[cfg(test)]
static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
thread_local! {
    /// True while THIS thread holds [`TEST_MUTEX`] via a [`PageTestGuard`] —
    /// the reentrancy key for [`test_lock`].
    static HOLDS_PAGE_LOCK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Guard for [`test_lock`]. `inner: None` is the reentrant (already-held) case:
/// dropping it releases nothing; the outermost guard clears the thread flag.
#[cfg(test)]
pub(crate) struct PageTestGuard {
    inner: Option<std::sync::MutexGuard<'static, ()>>,
}

#[cfg(test)]
impl Drop for PageTestGuard {
    fn drop(&mut self) {
        if self.inner.is_some() {
            HOLDS_PAGE_LOCK.with(|h| h.set(false));
        }
    }
}

/// Acquire the page-global test lock (see the module note above): blocks until
/// free, absorbs poison, and is REENTRANT per thread (a nested acquire — e.g. a
/// lock-holding test driving [`set_measure`] — returns a no-op guard instead of
/// deadlocking). The ONLY door to the mutex; the writer fns take it themselves.
#[cfg(test)]
pub(crate) fn test_lock() -> PageTestGuard {
    if HOLDS_PAGE_LOCK.with(|h| h.get()) {
        return PageTestGuard { inner: None };
    }
    let guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    HOLDS_PAGE_LOCK.with(|h| h.set(true));
    PageTestGuard { inner: Some(guard) }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── test_lock() — the reentrant page-global test lock ──────────────────
    // (the page-width-split flake fix: writers self-serialize; see the
    // module note on `test_lock`)

    #[test]
    fn test_lock_is_reentrant_and_the_outermost_guard_owns_the_release() {
        let g1 = test_lock();
        assert!(HOLDS_PAGE_LOCK.with(|h| h.get()), "the outer guard sets the thread flag");
        let g2 = test_lock(); // nested acquire on the SAME thread: must not deadlock
        drop(g2);
        assert!(
            HOLDS_PAGE_LOCK.with(|h| h.get()),
            "dropping the inner (no-op) guard must NOT release the outer hold"
        );
        // A writer on the holding thread rides the reentrant no-op path.
        set_measure(DEFAULT_MEASURE);
        drop(g1);
        assert!(
            !HOLDS_PAGE_LOCK.with(|h| h.get()),
            "the outermost drop clears the flag (a leak here would self-deadlock \
             the thread's next acquire)"
        );
    }

    #[test]
    fn writers_on_another_thread_block_while_the_lock_is_held() {
        // THE LAW the visual_* flake fix rests on: a page-global WRITE from a
        // thread that does not hold the lock (a Goto replay's measure resync,
        // `sync_page_measure`, `apply_sticky_globals`) can never land inside
        // another test's locked read window.
        let g = test_lock();
        set_measure(33);
        // The writer flips `done` only AFTER its internal acquire lets the write
        // land. Asserting on the flag (not the global's later value) keeps this
        // law test itself parallel-safe: once we release, any OTHER suite test
        // may legally write the global, so its exact value is unassertable.
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer = {
            let done = done.clone();
            std::thread::spawn(move || {
                set_measure(44);
                done.store(true, std::sync::atomic::Ordering::SeqCst);
            })
        };
        // Give the writer a beat to reach its (blocked) internal acquire. If it
        // could interleave, the read below would see 44 and `done` would flip.
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(measure(), 33, "the unheld thread's write cannot land while we hold");
        assert!(
            !done.load(std::sync::atomic::Ordering::SeqCst),
            "the writer must still be blocked while we hold"
        );
        drop(g);
        writer.join().unwrap();
        assert!(
            done.load(std::sync::atomic::Ordering::SeqCst),
            "the blocked write lands once the lock is released"
        );
        let _g = test_lock();
        set_measure(DEFAULT_MEASURE); // leave as found
    }

    #[test]
    fn defaults_on_at_seventy() {
        let _g = test_lock();
        set_page_on(true);
        set_measure(DEFAULT_MEASURE);
        assert!(page_on());
        assert_eq!(measure(), 70);
    }

    #[test]
    fn toggle_flips_on_off() {
        let _g = test_lock();
        set_page_on(true);
        assert!(!toggle()); // on -> off
        assert!(!page_on());
        assert!(toggle()); // off -> on
        assert!(page_on());
        set_page_on(true);
    }

    #[test]
    fn measure_floor_is_one() {
        let _g = test_lock();
        set_measure(0);
        assert_eq!(measure(), 1);
        set_measure(DEFAULT_MEASURE);
    }

    #[test]
    fn widen_narrow_step_the_measure_and_report_it() {
        let _g = test_lock();
        set_measure(DEFAULT_MEASURE); // 80
        assert_eq!(widen(), DEFAULT_MEASURE + MEASURE_STEP);
        assert_eq!(measure(), DEFAULT_MEASURE + MEASURE_STEP);
        assert_eq!(narrow(), DEFAULT_MEASURE); // back to start
        assert_eq!(measure(), DEFAULT_MEASURE);
        set_measure(DEFAULT_MEASURE);
    }

    #[test]
    fn widen_narrow_clamp_to_the_band() {
        let _g = test_lock();
        // Narrowing bottoms out at MIN_MEASURE, widening tops out at MAX_MEASURE.
        set_measure(MIN_MEASURE);
        assert_eq!(narrow(), MIN_MEASURE, "never below the floor");
        set_measure(MAX_MEASURE);
        assert_eq!(widen(), MAX_MEASURE, "never above the ceiling");
        // A sub-floor start snaps UP to the floor on narrow (max(.., MIN)).
        set_measure(5);
        assert_eq!(narrow(), MIN_MEASURE);
        set_measure(DEFAULT_MEASURE);
    }

    // ── PageClass (prose/code page-width split) ─────────────────────────────

    #[test]
    fn of_syntax_classifies_code_vs_prose() {
        assert_eq!(PageClass::of_syntax(Some(crate::syntax::Lang::Rust)), PageClass::Code);
        assert_eq!(PageClass::of_syntax(None), PageClass::Prose);
    }

    #[test]
    fn of_path_classifies_by_recognized_extension() {
        use std::path::Path;
        assert_eq!(PageClass::of_path(Some(Path::new("/a/main.rs"))), PageClass::Code);
        // Markdown, an unrecognized extension, and no path at all are all Prose.
        assert_eq!(PageClass::of_path(Some(Path::new("/a/notes.md"))), PageClass::Prose);
        assert_eq!(PageClass::of_path(Some(Path::new("/a/notes.txt"))), PageClass::Prose);
        assert_eq!(PageClass::of_path(None), PageClass::Prose);
    }

    #[test]
    fn default_measure_per_class() {
        assert_eq!(PageClass::Prose.default_measure(), DEFAULT_MEASURE);
        assert_eq!(PageClass::Code.default_measure(), DEFAULT_MEASURE_CODE);
        assert_eq!(DEFAULT_MEASURE, 70);
        assert_eq!(DEFAULT_MEASURE_CODE, 100);
    }
}
