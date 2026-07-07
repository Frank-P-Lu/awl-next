//! RECENTLY-OPENED FILES — the persisted most-recently-OPENED file paths (a small
//! MRU list), so the go-to picker's "Recent" lens can show the files you have
//! ACTUALLY opened recently (a real MRU) rather than the whole corpus.
//!
//! This is the FILE sibling of [`crate::recents`] (recent PROJECT roots): the same
//! machine-state-beside-the-scratch-stash reasoning, the same hand-rolled TOML
//! shape, the same lenient degrade-to-empty load. It REUSES `crate::recents`'
//! generic list helpers ([`crate::recents::push`] / [`crate::recents::load`] /
//! [`crate::recents::save`] — all pure PathBuf-list operations over an explicit
//! path + the shared `recent` TOML key), so the push/dedup/cap + (de)serialization
//! logic keeps ONE owner; this module only names its OWN file + cap.
//!
//! **Where it lives:** `fs::data_root()/recent-files.toml`, beside the scratch
//! stash, the session file, and `recent-projects.toml` — NOT inside `config.toml`
//! (machine state the app reads/writes as you work, not the user's own settings).
//! Its own file rather than a field on the session state: like the recent-projects
//! MRU, it should survive even with `session_restore` off.
//!
//! **Determinism (CRITICAL):** every read/write goes through the `App`-side seam
//! (`App::new` loads it; `App::load_path` pushes + saves), which lives only on the
//! live `App` — so a headless `--screenshot`/`--keys` capture, which never
//! constructs an `App`, is STRUCTURALLY incapable of touching this file, and the
//! go-to Recent lens is fed an EMPTY MRU there (degrading to the empty state).
//! Native persistence matches `crate::recents` exactly: the `crate::fs::active()`
//! seam is a WebFs no-op-to-`localStorage` on wasm, and the whole store is only
//! ever reached from the live `App`.

use std::path::PathBuf;

/// How many recently-opened files to remember (most-recent-first). Deeper than the
/// recent-PROJECTS cap ([`crate::recents::CAP`]) — a file MRU is stepped through
/// more granularly than a project one.
pub const CAP: usize = 20;

/// Where the recent-files file lives: beside the scratch stash + session +
/// recent-projects files, same data root.
pub fn recent_files_path() -> PathBuf {
    crate::fs::data_root().join("recent-files.toml")
}

/// Push `file` (an ABSOLUTE path) to the FRONT of `list` as the most-recently-opened
/// file, DEDUPED (a prior occurrence moves to the front, never duplicates) + CAPPED
/// at [`CAP`]. Delegates to the ONE list-MRU owner ([`crate::recents::push`]) — same
/// rule, no second copy.
pub fn push(list: Vec<PathBuf>, file: PathBuf) -> Vec<PathBuf> {
    crate::recents::push(list, file, CAP)
}

/// Load the persisted recent-files list, degrading a MISSING/garbage file to an
/// EMPTY list (via [`crate::recents::load`], the shared lenient reader).
pub fn load() -> Vec<PathBuf> {
    crate::recents::load(&recent_files_path())
}

/// Persist `list` ATOMICALLY (via [`crate::recents::save`], the shared atomic
/// writer). A save error is the caller's to report + swallow — a lost MRU entry is
/// never worth crashing a file open.
pub fn save(list: &[PathBuf]) -> std::io::Result<()> {
    crate::recents::save(&recent_files_path(), list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_puts_the_newest_at_the_front_deduped() {
        let list = push(Vec::new(), PathBuf::from("/w/a.md"));
        let list = push(list, PathBuf::from("/w/b.md"));
        // Re-opening a.md moves it to the front, never duplicates it.
        let list = push(list, PathBuf::from("/w/a.md"));
        assert_eq!(
            list,
            vec![PathBuf::from("/w/a.md"), PathBuf::from("/w/b.md")]
        );
        assert_eq!(list.iter().filter(|p| *p == &PathBuf::from("/w/a.md")).count(), 1);
    }

    #[test]
    fn push_caps_at_twenty_dropping_the_oldest() {
        let mut list = Vec::new();
        for i in 0..(CAP + 8) {
            list = push(list, PathBuf::from(format!("/w/f{i}.md")));
        }
        assert_eq!(list.len(), CAP, "capped at CAP=20");
        assert_eq!(list[0], PathBuf::from(format!("/w/f{}.md", CAP + 7)), "newest at front");
        assert!(!list.contains(&PathBuf::from("/w/f0.md")), "oldest fell off");
    }

    #[test]
    fn load_and_save_round_trip_through_in_memory_fs() {
        use std::sync::Arc;
        let fake = Arc::new(crate::fs::InMemoryFs::new());
        crate::fs::with_fs(fake, || {
            assert!(load().is_empty(), "missing file: empty list");
            let list = vec![PathBuf::from("/w/a.md"), PathBuf::from("/w/sub/b.md")];
            save(&list).unwrap();
            assert_eq!(load(), list, "round-trips through recent-files.toml");
        });
    }
}
