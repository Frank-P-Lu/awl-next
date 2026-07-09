//! RECENT PROJECT ROOTS — the persisted most-recently-switched-to project
//! folders (a small MRU list), surfaced as the SWITCH-PROJECT navigator's
//! **Recent** lens (and the "Recent projects…" palette/File-menu door that opens
//! it pre-lensed there — the fold that retired the standalone RecentProjects
//! picker). This module is the PURE data model + (de)serializer + the
//! push/dedup/cap rule; the App-side wiring (loading at launch, pushing on every
//! switch-project, marking the lens) lives in `app.rs` / `app/apply.rs`.
//!
//! **Where it lives:** beside the scratch stash + session file
//! (`fs::data_root()/recent-projects.toml`), NOT inside `config.toml` — the
//! config file is the user's own hand-edited settings; this is MACHINE STATE
//! the app itself reads and writes as you work (mirrors [`crate::session`]
//! exactly — same data root, same "its own file, not folded into config"
//! reasoning). Deliberately its OWN file rather than a field on the session
//! state: the recent list is a cross-session MRU that should survive even with
//! `session_restore` turned off, so it does not ride the session kill-switch.
//!
//! **Format:** a hand-rolled TOML writer ([`to_toml`], mirrors
//! `crate::session::to_toml`) paired with the crate's existing `toml` PARSER
//! ([`from_toml`]), so reading stays lenient — a malformed/missing file
//! degrades to an EMPTY list, never a crash.
//!
//! **Determinism (CRITICAL):** every read/write goes through the `App`-side
//! seam (`App::new` loads it; `App::switch_project` pushes + saves), which is
//! native-only and lives only on the live `App` — so a headless
//! `--screenshot`/`--keys` capture, which never constructs an `App`, is
//! STRUCTURALLY incapable of reading or writing this file. The Project
//! navigator's Recent lens is fed an EMPTY MRU in the headless build path
//! (`main/run.rs`), so a capture stays byte-stable.

use std::path::{Path, PathBuf};

/// How many recent project roots to remember (most-recent-first). A calm,
/// menu-friendly depth — enough to jump back to what you were in, not a
/// full history.
pub const CAP: usize = 10;

/// Where the recent-projects file lives: beside the scratch stash + session
/// file, same data root.
pub fn recents_path() -> PathBuf {
    crate::fs::data_root().join("recent-projects.toml")
}

/// Push `root` to the FRONT of `list` as the most-recent project, DEDUPED
/// (any prior occurrence of the same path is removed, so it moves to the front
/// rather than duplicating) and CAPPED at `cap` (the oldest fall off the end).
/// PURE — no fs, injected list — the whole store rule in one testable place.
pub fn push(mut list: Vec<PathBuf>, root: PathBuf, cap: usize) -> Vec<PathBuf> {
    list.retain(|p| p != &root);
    list.insert(0, root);
    list.truncate(cap);
    list
}

/// Load the persisted recent-projects list from `path` through the active
/// `FileSystem` backend. A MISSING or unparseable file degrades to an EMPTY
/// list — never an error, mirroring [`crate::session::load`]'s leniency.
pub fn load(path: &Path) -> Vec<PathBuf> {
    match crate::fs::active().read_to_string(path) {
        Ok(src) => from_toml(&src),
        Err(_) => Vec::new(),
    }
}

/// Persist `list` to `path` ATOMICALLY (temp-sibling + rename, via
/// [`crate::fs::write_atomic`] — the same primitive the autosave engine, the
/// scratch stash, and the session file use).
pub fn save(path: &Path, list: &[PathBuf]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = crate::fs::active().create_dir_all(parent);
    }
    crate::fs::write_atomic(path, to_toml(list).as_bytes())
}

/// Serialize `list` to the on-disk TOML shape — pure, no fs. Hand-rolled
/// (mirrors `crate::session::to_toml`); read back through the real `toml`
/// parser via [`from_toml`], so the two halves never have to hand-agree on
/// escaping rules.
pub fn to_toml(list: &[PathBuf]) -> String {
    let mut out = String::new();
    out.push_str("recent = [\n");
    for p in list {
        out.push_str("  ");
        out.push_str(&quote(p));
        out.push_str(",\n");
    }
    out.push_str("]\n");
    out
}

/// A path as a quoted + escaped TOML basic string.
fn quote(p: &Path) -> String {
    let s = p.display().to_string();
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Parse the on-disk TOML shape back into the recent list — pure, no fs.
/// LENIENT throughout (mirrors [`crate::session::from_toml`]): an unparseable
/// document or a missing/wrong-typed `recent` array yields an EMPTY list;
/// non-string entries within the array are skipped rather than erroring.
pub fn from_toml(src: &str) -> Vec<PathBuf> {
    let Ok(table) = src.parse::<toml::Table>() else {
        return Vec::new();
    };
    let Some(arr) = table.get("recent").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| v.as_str())
        .map(PathBuf::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_puts_the_newest_at_the_front() {
        let list = push(Vec::new(), PathBuf::from("/a"), CAP);
        let list = push(list, PathBuf::from("/b"), CAP);
        assert_eq!(list, vec![PathBuf::from("/b"), PathBuf::from("/a")]);
    }

    #[test]
    fn push_dedupes_moving_an_existing_root_to_the_front() {
        let list = vec![PathBuf::from("/a"), PathBuf::from("/b"), PathBuf::from("/c")];
        // Re-switching to /c moves it to the front, never duplicates it.
        let list = push(list, PathBuf::from("/c"), CAP);
        assert_eq!(
            list,
            vec![PathBuf::from("/c"), PathBuf::from("/a"), PathBuf::from("/b")]
        );
        assert_eq!(list.iter().filter(|p| *p == &PathBuf::from("/c")).count(), 1);
    }

    #[test]
    fn push_caps_the_list_dropping_the_oldest() {
        let mut list = Vec::new();
        for i in 0..(CAP + 5) {
            list = push(list, PathBuf::from(format!("/p{i}")), CAP);
        }
        assert_eq!(list.len(), CAP, "capped at CAP");
        assert_eq!(list[0], PathBuf::from(format!("/p{}", CAP + 4)), "newest at front");
        // The oldest (/p0.. /p4) fell off the end.
        assert!(!list.contains(&PathBuf::from("/p0")));
    }

    #[test]
    fn round_trips_through_toml() {
        let list = vec![PathBuf::from("/home/me/proj-a"), PathBuf::from("/home/me/proj b")];
        assert_eq!(from_toml(&to_toml(&list)), list);
    }

    #[test]
    fn empty_or_garbage_toml_yields_empty_list() {
        assert_eq!(from_toml(""), Vec::<PathBuf>::new());
        assert_eq!(from_toml("not valid toml {{{"), Vec::<PathBuf>::new());
        assert_eq!(from_toml("other = 3\n"), Vec::<PathBuf>::new());
    }

    #[test]
    fn load_and_save_round_trip_through_in_memory_fs() {
        use std::sync::Arc;
        let fake = Arc::new(crate::fs::InMemoryFs::new());
        crate::fs::with_fs(fake, || {
            let path = PathBuf::from("/data/recent-projects.toml");
            assert!(load(&path).is_empty(), "missing file: empty list");
            let list = vec![PathBuf::from("/w/a"), PathBuf::from("/w/b")];
            save(&path, &list).unwrap();
            assert_eq!(load(&path), list);
        });
    }
}
