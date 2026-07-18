//! src/history/ — AUTOMATIC LOCAL SNAPSHOTS, split by natural seam (2026-07
//! code-organization pass) out of the former `history.rs` monolith:
//! [`store`] (the on-disk log + git-presence gate + git backend + the
//! public record/list/load API), [`prune`] (the aged retention ladder
//! [`store::record_at`] calls after every append), and [`picker`] (the
//! summoned timeline picker's read model — rows, faceting, the small pure
//! label/diff/description helpers). Every external path
//! (`history::record`, `history::TimelineRow`, `history::prune_ladder`, …)
//! is unchanged — this file only re-exports.
//!
//! The shape (why it is the way it is):
//!   * PERSISTENCE goes through the [`crate::fs`] SEAM — never `std::fs` directly —
//!     so the same code snapshots to the real disk on native AND to `localStorage`
//!     on the web (`WebFs` already backs the trait). One store, two backends, free.
//!   * The store is ONE LOG FILE PER SOURCE PATH (`<root>/<hash>.log`), holding a
//!     bounded, newest-first list of FULL-CONTENT snapshots framed by a byte length.
//!     Full copies are simple + robust (no diffing to get wrong); the AGED
//!     RETENTION LADDER ([`prune::prune_ladder`]) keeps it small by thinning RESOLUTION,
//!     never memory: everything fresh is kept, then one survivor per work session
//!     up to a day, one per day up to a month, one per week beyond — the total
//!     capped at `MAX_TOTAL` by climbing the ladder harder (never FIFO). A
//!     single log file (rewritten to prune) means the store needs only the trait's
//!     read/write — no per-file delete op the seam doesn't have.
//!   * The GIT-PRESENCE GATE decides WHO owns a file's history, ABSOLUTELY. A file
//!     inside a git repo (a `.git` dir in some ancestor) is git's to version — awl
//!     writes NO snapshot for it, EVER (no save hook, no autosave hook — writing
//!     the file itself is not version-meddling; snapshotting it would be), and the
//!     timeline reads `git log` / `git show` instead (the git BACKEND of [`store::list`]
//!     / [`store::load`]). A LOOSE file (no repo) — or ANY file on the web, where there
//!     is no git — gets awl snapshots. So the two histories never double up, and
//!     awl never fights git. (This SUPERSEDES the old `record_periodic` contract,
//!     which snapshotted inside repos on an opt-in interval; the autosave engine
//!     replaced the interval, and git files are now git-only.)
//!
//! The read/write API: [`record`] (the save-hook — every save, manual or
//! autosave), [`list`] (newest-first), [`load`] (round-trip the content). Same
//! signatures for both backends.

mod picker;
mod prune;
mod store;

pub use picker::{
    clamp_line_col, diff_preview, mark_session_start, session_epoch_ms, source_path,
    timeline_rows, TimelineRow, HISTORY_FACETS,
};
#[allow(unused_imports)] // auto_description/clock_hm/first_changed_line/line_diff_counts/
// relative_label/rows_from: public API surface (the pure row-composer helpers
// `timeline_rows` itself calls internally, unqualified — no separate re-export
// needed there), reached in-crate via `history::tests` (this module's own
// `#[cfg(test)]` suite) rather than this re-export in a non-test build.
pub use picker::{
    auto_description, clock_hm, first_changed_line, line_diff_counts, relative_label, rows_from,
};
#[allow(unused_imports)] // prune_ladder: pub(crate) API surface (`store::record_at`
// reaches it directly, unqualified, needing no re-export there), reached in-crate
// via `history::tests` (this module's own `#[cfg(test)]` suite) rather than this
// re-export in a non-test build.
pub(crate) use prune::prune_ladder;
#[allow(unused_imports)] // Entry: pub(crate) API surface (the store's own record
// type prune_ladder operates on), reached in-crate via `store::Entry` directly
// (prune.rs, tests.rs) rather than this re-export today — kept for parity with
// its pre-split reachability at `crate::history::Entry`.
pub(crate) use store::Entry;
#[allow(unused_imports)] // record_at: pub(crate) API surface (the shared
// record/record_pinned/prune-ladder shell, injected-clock testable), reached
// in-crate via `store::record_at` directly (store.rs's own record/record_pinned,
// tests.rs) rather than this re-export today — kept for parity with its
// pre-split reachability at `crate::history::record_at`.
pub(crate) use store::record_at;
pub use store::{load, now_millis, record, record_pinned, rename};
#[allow(unused_imports)] // Snapshot/is_git_managed/git_repo_root/list: public API
// surface, reached in-crate via `history::tests` (this module's own
// `#[cfg(test)]` suite, plus `app.rs`'s own test module for `list`) rather than
// this re-export in a non-test build.
pub use store::{git_repo_root, is_git_managed, list, Snapshot};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
