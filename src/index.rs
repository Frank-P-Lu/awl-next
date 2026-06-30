//! Per-root file index: the corpus the go-to overlay fuzzy-matches against.
//!
//! The active project is a ROOT directory. Its file list scopes the go-to panel
//! so typing ".env" finds THIS repo's env, not every env on disk. Two roots,
//! one rule each:
//!   * GIT root (`<root>/.git` exists) -> `git ls-files` (tracked) UNION the
//!     PRESENT `.env*` files in the tree, MINUS the heavy junk dirs. The .env
//!     union is deliberate: a repo's `.env` is usually gitignored but is exactly
//!     the file you want to jump to.
//!   * NON-git root -> a recursive walk, skipping those same junk dirs.
//! Either way the returned paths are ROOT-RELATIVE (forward-slashed), so they
//! render compactly and match the way a developer thinks about a tree.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Directory names pruned from EVERY index (git and non-git alike). These are
/// build output / vendored deps / VCS internals — never go-to targets, and
/// walking them would swamp the corpus.
pub const JUNK_DIRS: &[&str] = &["node_modules", ".git", "target", "build", "dist"];

/// True if `name` is a junk directory we always skip.
fn is_junk_dir(name: &str) -> bool {
    JUNK_DIRS.contains(&name)
}

/// True if `name` is an `.env` family file (`.env`, `.env.local`, …). These are
/// force-INCLUDED in a git index even when gitignored, because they are prime
/// go-to targets.
fn is_env_file(name: &str) -> bool {
    name == ".env" || name.starts_with(".env.") || name.starts_with(".env")
}

/// Build the candidate file list for `root`, root-relative. Picks the git or
/// walk strategy based on whether `<root>/.git` exists. The result is sorted and
/// de-duplicated so callers get a stable corpus.
pub fn build_index(root: &Path) -> Vec<String> {
    let mut out = if crate::fs::active().exists(&root.join(".git")) {
        git_index(root)
    } else {
        walk_index(root)
    };
    out.sort();
    out.dedup();
    out
}

/// GIT strategy: `git ls-files` UNION present `.env*`, MINUS junk dirs.
fn git_index(root: &Path) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    if let Ok(o) = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("ls-files")
        .output()
    {
        if o.status.success() {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                let rel = line.trim();
                if rel.is_empty() {
                    continue;
                }
                if rel.split('/').any(is_junk_dir) {
                    continue;
                }
                files.push(rel.to_string());
            }
        }
    }
    // UNION present .env* files (often gitignored, but prime go-to targets).
    let mut env_files = Vec::new();
    walk_collect(root, root, &mut env_files, &mut |name| is_env_file(name));
    files.extend(env_files);
    files
}

/// NON-git strategy: recursive walk skipping junk dirs, every file included.
fn walk_index(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    walk_collect(root, root, &mut files, &mut |_| true);
    files
}

/// Recursively collect ROOT-relative file paths under `dir`, skipping junk
/// directories. `keep(file_name)` filters which files are pushed.
fn walk_collect(
    root: &Path,
    dir: &Path,
    out: &mut Vec<String>,
    keep: &mut dyn FnMut(&str) -> bool,
) {
    let Ok(entries) = crate::fs::active().read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.path;
        let name = entry.name;
        if entry.is_dir {
            if is_junk_dir(&name) {
                continue;
            }
            walk_collect(root, &path, out, keep);
        } else if entry.is_file && keep(&name) {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

/// Resolve a root-relative index entry back to an absolute path under `root`.
pub fn resolve(root: &Path, rel: &str) -> PathBuf {
    root.join(rel)
}

/// A compact, human "last edited" label for a file modified at `mtime`, relative
/// to `now`: "just now" (< 1 min), "Nm ago" (minutes), "Nh ago" (hours), "Nd ago"
/// (days). A future mtime (clock skew) reads as "just now". PURE — no clock read —
/// so it is unit-testable and the live caller injects `now`.
pub fn relative_time(now: SystemTime, mtime: SystemTime) -> String {
    let secs = now.duration_since(mtime).map(|d| d.as_secs()).unwrap_or(0);
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

/// Order `(name, mtime)` entries MOST-RECENTLY-MODIFIED first, breaking ties by
/// name (ascending) so the order is stable + deterministic. PURE (sorts the given
/// vector), so the recency rule is unit-testable without touching the filesystem.
pub fn order_by_recency(mut entries: Vec<(String, SystemTime)>) -> Vec<(String, SystemTime)> {
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    entries
}

/// Re-order a go-to `corpus` (root-relative paths) by "last edited", newest first,
/// and pair each entry with a relative-time label for the picker.
///
/// DETERMINISM GATE: `now` is `Some` only in the LIVE app, where real mtimes are
/// read via `std::fs` metadata. In the HEADLESS capture path `now` is `None`, so
/// NO mtime is read and the corpus keeps its incoming (name) order with EMPTY
/// labels — the `--screenshot` sidecar stays byte-stable.
pub fn with_recency(
    root: &Path,
    corpus: Vec<String>,
    now: Option<SystemTime>,
) -> (Vec<String>, Vec<String>) {
    let Some(now) = now else {
        let times = vec![String::new(); corpus.len()];
        return (corpus, times);
    };
    let entries: Vec<(String, SystemTime)> = corpus
        .into_iter()
        .map(|rel| {
            let mtime = crate::fs::active()
                .metadata(&root.join(&rel))
                .ok()
                .and_then(|md| md.modified)
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (rel, mtime)
        })
        .collect();
    let ordered = order_by_recency(entries);
    let mut names = Vec::with_capacity(ordered.len());
    let mut times = Vec::with_capacity(ordered.len());
    for (rel, mtime) in ordered {
        times.push(relative_time(now, mtime));
        names.push(rel);
    }
    (names, times)
}

/// One entry of a single directory LEVEL (for the browse navigator). `name` is
/// the leaf name; `is_dir` distinguishes a folder (Enter descends) from a file
/// (Enter opens); `is_git` marks a folder that is itself a git repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_git: bool,
}

/// List ONE directory level under `root`/`rel` (rel `None` = the root itself) for
/// the browse navigator: directories first (sorted), then files (sorted). Junk
/// dirs are skipped so the level stays clean. Each directory is probed (cheaply,
/// `<dir>/.git`) for a git marker. Returns an empty list if the path can't be
/// read (e.g. an ascend/descend past a vanished dir).
pub fn list_dir_level(root: &Path, rel: Option<&str>) -> Vec<DirEntry> {
    let dir = match rel {
        Some(r) if !r.is_empty() => root.join(r),
        _ => root.to_path_buf(),
    };
    let mut dirs: Vec<DirEntry> = Vec::new();
    let mut files: Vec<DirEntry> = Vec::new();
    let Ok(entries) = crate::fs::active().read_dir(&dir) else {
        return Vec::new();
    };
    for entry in entries {
        let name = entry.name;
        if entry.is_dir {
            if is_junk_dir(&name) {
                continue;
            }
            let is_git = crate::fs::active().exists(&entry.path.join(".git"));
            dirs.push(DirEntry {
                name,
                is_dir: true,
                is_git,
            });
        } else if entry.is_file {
            files.push(DirEntry {
                name,
                is_dir: false,
                is_git: false,
            });
        }
    }
    dirs.sort_by(|a, b| a.name.cmp(&b.name));
    files.sort_by(|a, b| a.name.cmp(&b.name));
    dirs.extend(files);
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("awl_index_test_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn walk_skips_junk_dirs() {
        // Routed through the FILESYSTEM SEAM (InMemoryFs): the non-git walk strategy
        // runs entirely in memory — no temp dir, no `.git` so build_index walks.
        use std::sync::Arc;
        let root = std::path::PathBuf::from("/proj");
        let mem = crate::fs::InMemoryFs::new()
            .with_file("/proj/notes.md", "n")
            .with_file("/proj/sub/ideas.md", "i")
            .with_file("/proj/node_modules/big.js", "x");
        crate::fs::with_fs(Arc::new(mem), || {
            let idx = build_index(&root);
            assert!(idx.contains(&"notes.md".to_string()));
            assert!(idx.contains(&"sub/ideas.md".to_string()));
            assert!(
                !idx.iter().any(|p| p.contains("node_modules")),
                "junk dir must be pruned: {idx:?}"
            );
        });
    }

    #[test]
    fn resolve_joins_relative_under_root() {
        // The forward-slash relative form the index emits joins back onto root as a
        // host-native PathBuf.
        assert_eq!(
            resolve(Path::new("/root"), "a/b.rs"),
            PathBuf::from("/root").join("a/b.rs")
        );
        // A bare filename joins directly under root.
        assert_eq!(resolve(Path::new("/root"), "notes.md"), PathBuf::from("/root/notes.md"));
    }

    #[test]
    fn env_file_detection() {
        assert!(is_env_file(".env"));
        assert!(is_env_file(".env.local"));
        assert!(is_env_file(".env.production"));
        assert!(!is_env_file("env"));
        assert!(!is_env_file("README.md"));
    }

    #[test]
    fn junk_dir_detection() {
        assert!(is_junk_dir("node_modules"));
        assert!(is_junk_dir("target"));
        assert!(is_junk_dir(".git"));
        assert!(!is_junk_dir("src"));
    }

    #[test]
    fn dir_level_dirs_first_files_sorted() {
        // The browse navigator's one-level listing, over the InMemoryFs seam.
        use std::sync::Arc;
        let root = std::path::PathBuf::from("/proj");
        let mem = crate::fs::InMemoryFs::new()
            .with_file("/proj/README.md", "r")
            .with_dir("/proj/src")
            .with_file("/proj/docs/guide.md", "g")
            .with_dir("/proj/node_modules");
        crate::fs::with_fs(Arc::new(mem), || {
            // Root level: dirs (docs, src) before files (README.md), junk skipped.
            let lvl = list_dir_level(&root, None);
            let names: Vec<&str> = lvl.iter().map(|e| e.name.as_str()).collect();
            assert_eq!(names, vec!["docs", "src", "README.md"], "got {names:?}");
            assert!(lvl[0].is_dir && lvl[1].is_dir && !lvl[2].is_dir);
            // Descend into docs/: shows guide.md.
            let docs = list_dir_level(&root, Some("docs"));
            assert_eq!(docs.len(), 1);
            assert_eq!(docs[0].name, "guide.md");
            assert!(!docs[0].is_dir);
        });
    }

    #[test]
    fn relative_time_buckets() {
        use std::time::Duration;
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000_000);
        let ago = |secs: u64| relative_time(now, now - Duration::from_secs(secs));
        assert_eq!(ago(0), "just now");
        assert_eq!(ago(59), "just now");
        assert_eq!(ago(60), "1m ago");
        assert_eq!(ago(5 * 60), "5m ago");
        assert_eq!(ago(60 * 60), "1h ago");
        assert_eq!(ago(2 * 60 * 60), "2h ago");
        assert_eq!(ago(24 * 60 * 60), "1d ago");
        assert_eq!(ago(3 * 24 * 60 * 60), "3d ago");
        // A future mtime (clock skew) reads as "just now", never panics.
        assert_eq!(relative_time(now, now + Duration::from_secs(99)), "just now");
    }

    #[test]
    fn recency_sort_orders_newest_first() {
        use std::time::Duration;
        let base = SystemTime::UNIX_EPOCH;
        let entries = vec![
            ("old.md".to_string(), base + Duration::from_secs(100)),
            ("newest.md".to_string(), base + Duration::from_secs(900)),
            ("mid.md".to_string(), base + Duration::from_secs(500)),
        ];
        let ordered = order_by_recency(entries);
        let names: Vec<&str> = ordered.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["newest.md", "mid.md", "old.md"]);
    }

    #[test]
    fn recency_ties_break_by_name() {
        use std::time::Duration;
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(42);
        let entries = vec![
            ("b.md".to_string(), t),
            ("a.md".to_string(), t),
            ("c.md".to_string(), t),
        ];
        let ordered = order_by_recency(entries);
        let names: Vec<&str> = ordered.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["a.md", "b.md", "c.md"]);
    }

    #[test]
    fn with_recency_headless_keeps_name_order_no_times() {
        // `now == None` (headless): corpus order is preserved and labels are empty
        // so the capture stays byte-stable (no mtime read).
        let corpus = vec!["a.md".to_string(), "b.md".to_string()];
        let (names, times) = with_recency(Path::new("/nonexistent"), corpus.clone(), None);
        assert_eq!(names, corpus);
        assert_eq!(times, vec![String::new(), String::new()]);
    }

    #[test]
    fn dir_level_marks_git_child() {
        // The git-child marker (`<child>/.git` probe), over the InMemoryFs seam.
        use std::sync::Arc;
        let root = std::path::PathBuf::from("/proj");
        let mem = crate::fs::InMemoryFs::new()
            .with_dir("/proj/repo/.git")
            .with_dir("/proj/plain");
        crate::fs::with_fs(Arc::new(mem), || {
            let lvl = list_dir_level(&root, None);
            let repo = lvl.iter().find(|e| e.name == "repo").unwrap();
            let plain = lvl.iter().find(|e| e.name == "plain").unwrap();
            assert!(repo.is_git, "repo with .git must be marked git");
            assert!(!plain.is_git);
        });
    }
}

