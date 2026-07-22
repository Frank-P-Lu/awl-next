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

use crate::clock::SystemTime;
use crate::facets::{Facet, FacetItem, FacetScheme};
use std::path::{Path, PathBuf};

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
pub fn is_env_file(name: &str) -> bool {
    name == ".env" || name.starts_with(".env.") || name.starts_with(".env")
}

/// True if a corpus entry `rel` (a root-relative, forward-slashed path OR a bare
/// browse-level leaf name) should be HIDDEN from the file pickers by default — the
/// Finder "hide dotfiles" convention. An entry is hidden when its basename OR any
/// ancestor path component starts with `.`, with ONE earned exception: an `.env*`
/// family file ([`is_env_file`]) STAYS visible (it's usually gitignored but is
/// exactly the file you want to jump to — the same force-include rationale the
/// git index applies). PURE — the `show_hidden` picker toggle re-runs the display
/// filter against this, so a capture can assert dotfiles absent by default +
/// present after the toggle.
pub fn is_hidden_entry(rel: &str) -> bool {
    let basename = rel.rsplit('/').next().unwrap_or(rel);
    if is_env_file(basename) {
        return false; // the earned exception: `.env*` files stay visible
    }
    rel.split('/').any(|component| component.starts_with('.'))
}

// --- The FILE pickers' GENERIC facet schemes --------------------------------
//
// The go-to (flat file index) and browse (one directory level) pickers plug into
// the picker-agnostic faceted-lens machinery ([`crate::facets`]) exactly the way
// the theme picker does — each authors a [`FacetScheme`] (its lens strip + a bucket
// fn) that [`crate::facets::scheme`] hands back for its [`crate::overlay::OverlayKind`].
// "All" is HOME (strip index 0, the flat list); LEFT/RIGHT step into the refinements.
//
// The bucketing is a PURE function of the [`FacetItem`] — the accept string for
// Go-to's path-derived This-folder lens, the `recent` flag for Go-to's Recent lens
// (the recently-OPENED-files MRU, [`crate::recent_files`]), the `is_dir` / `is_git`
// flags for Browse's Folders / Files / Git-repos split. No filesystem read, no clock
// inside the bucket.

/// Go-to's lens strip: **All** (flat home — the current doc's HEADINGS mixed with
/// FILES in one fuzzy-ranked list, the unified default; see [`crate::overlay::nav`]'s
/// `refilter`) · **Recent** (recently-OPENED files, a real MRU — EMPTY until you open
/// something) · **This folder** · **Headings** (an explicit refinement down to ONLY
/// the current markdown doc's headings — the fold that retired the standalone
/// Outline picker). All FIRST (the landing lens); the rest are ←/→ refinements.
/// Headings is parked LAST — it swaps the corpus from files to the doc's headings, so
/// it reads as the furthest refinement. (TASTE CALL, logged: the Headings lens is
/// ALWAYS on the strip, even over a non-markdown buffer, where it reads empty ("no
/// headings yet") — a static strip keeps the lens indices stable for the generic
/// bucket/cycle machinery, and an empty lens is calmer than per-instance strip
/// surgery.) The former **By type** lens was CUT (decision: redundant once the
/// unified All list exists — a fuzzy query already reaches a file by its extension).
const GOTO_FACET_STRIP: [Facet; 4] = [
    Facet { label: "All", id: "all", sections: &[] },
    Facet { label: "Recent", id: "recent", sections: &["Recent"] },
    Facet { label: "This folder", id: "folder", sections: &["This folder"] },
    Facet { label: "Headings", id: "headings", sections: &["Headings"] },
];

/// Go-to's [`FacetScheme::bucket`], keyed by the strip index (see [`GOTO_FACET_STRIP`]).
/// `Recent` shows ONLY the files ACTUALLY OPENED recently — a real MRU: an item opts
/// IN iff `item.recent` (populated from the persisted recently-opened-files store,
/// [`crate::recent_files`], via [`crate::overlay::OverlayState`]'s `recent` vec) and
/// OUT (returns `None`) otherwise, so on a fresh session with nothing opened the lens
/// is EMPTY and shows the empty state. MRU order (most-recent first) is applied by
/// `refilter`'s MRU tiebreak, not here. `This folder` keeps only top-level FILE
/// entries (no `/` in the path, and NOT a heading row — `refilter`'s heading gate
/// already keeps headings out of every lens but All/Headings, so this arm never sees
/// one, but the explicit `!item.heading` guard keeps the rule true even if that gate
/// ever changes). `Headings` keeps ONLY the document-heading rows (`item.heading`) —
/// Go-to's corpus appends the doc's headings after its files.
fn goto_bucket(item: FacetItem, lens_idx: usize) -> Option<&'static str> {
    match lens_idx {
        1 => item.recent.then_some("Recent"), // Recent: ONLY recently-OPENED files (a real MRU)
        2 => (!item.heading && !item.accept.contains('/')).then_some("This folder"),
        3 => item.heading.then_some("Headings"), // Headings: the doc's heading rows only
        _ => None,                               // 0 = All (never grouped)
    }
}

/// Go-to's registered [`FacetScheme`], handed back by [`crate::facets::scheme`] for
/// [`crate::overlay::OverlayKind::Goto`].
pub static GOTO_FACETS: FacetScheme = FacetScheme { strip: &GOTO_FACET_STRIP, bucket: goto_bucket };

/// Browse's lens strip: **All** (flat home) · **Folders** · **Files** · **Git
/// repos**. All FIRST (the landing lens); the rest are ←/→ refinements over the
/// current directory level.
const BROWSE_FACET_STRIP: [Facet; 4] = [
    Facet { label: "All", id: "all", sections: &[] },
    Facet { label: "Folders", id: "folders", sections: &["Folders"] },
    Facet { label: "Files", id: "files", sections: &["Files"] },
    Facet { label: "Git repos", id: "git", sections: &["Git repos"] },
];

/// Browse's [`FacetScheme::bucket`], keyed by strip index (see [`BROWSE_FACET_STRIP`]).
/// Splits the level by the universal per-item flags on the [`FacetItem`]: `Folders`
/// = `is_dir`, `Files` = `!is_dir`, `Git repos` = `is_git` (a git-repo folder is in
/// BOTH Folders and Git repos — the lenses are independent facets, not a partition).
fn browse_bucket(item: FacetItem, lens_idx: usize) -> Option<&'static str> {
    match lens_idx {
        1 => item.is_dir.then_some("Folders"),
        2 => (!item.is_dir).then_some("Files"),
        3 => item.is_git.then_some("Git repos"),
        _ => None, // 0 = All (never grouped)
    }
}

/// Browse's registered [`FacetScheme`], handed back by [`crate::facets::scheme`] for
/// [`crate::overlay::OverlayKind::Browse`].
pub static BROWSE_FACETS: FacetScheme =
    FacetScheme { strip: &BROWSE_FACET_STRIP, bucket: browse_bucket };

/// The switch-project navigator's lens strip: **All** (the flat workspace-folder
/// listing — the landing, unchanged behavior) · **Recent** (the folders that are
/// in the recent-PROJECTS MRU, [`crate::recents`], most-recent first — EMPTY until
/// you've switched projects). All FIRST (the landing lens); `Recent` is the one
/// ←/→ refinement, mirroring Go-to's own Recent lens.
const PROJECT_FACET_STRIP: [Facet; 2] = [
    Facet { label: "All", id: "all", sections: &[] },
    Facet { label: "Recent", id: "recent", sections: &["Recent"] },
];

/// The switch-project navigator's [`FacetScheme::bucket`], keyed by strip index
/// (see [`PROJECT_FACET_STRIP`]). `Recent` shows ONLY the folders that are in the
/// recent-PROJECTS MRU — an item opts IN iff `item.recent` (populated in
/// [`crate::overlay::OverlayState::new_project`] from the persisted MRU, matched by
/// absolute path) and OUT (returns `None`) otherwise, so on a fresh session with
/// nothing switched-to the lens is EMPTY and shows the empty state. MRU order
/// (most-recent first) is applied by `refilter`'s MRU tiebreak, exactly like Go-to.
/// The synthetic "." accept-this-folder row never opts in (its `recent` is false),
/// so it stays an All-home affordance.
fn project_bucket(item: FacetItem, lens_idx: usize) -> Option<&'static str> {
    match lens_idx {
        1 => item.recent.then_some("Recent"), // Recent: ONLY the recent-projects MRU
        _ => None,                            // 0 = All (never grouped)
    }
}

/// The switch-project navigator's registered [`FacetScheme`], handed back by
/// [`crate::facets::scheme`] for [`crate::overlay::OverlayKind::Project`].
pub static PROJECT_FACETS: FacetScheme =
    FacetScheme { strip: &PROJECT_FACET_STRIP, bucket: project_bucket };

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

/// GIT strategy: `git ls-files` (tracked) UNION `git ls-files --others
/// --exclude-standard` (untracked-but-not-gitignored — a brand-new file you just
/// created is a go-to target BEFORE you `git add` it; `--exclude-standard` still
/// honours `.gitignore`, so build junk stays out) UNION present `.env*`, MINUS
/// junk dirs.
fn git_index(root: &Path) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    files.extend(git_ls(root, &["ls-files"]));
    files.extend(git_ls(root, &["ls-files", "--others", "--exclude-standard"]));
    // UNION present .env* files (often gitignored, but prime go-to targets).
    let mut env_files = Vec::new();
    walk_collect(root, root, &mut env_files, &mut |name| is_env_file(name));
    files.extend(env_files);
    files
}

/// Run `git -C <root> <args…>` and return stdout as ROOT-relative lines, skipping
/// blanks and any path under a junk dir. Empty on any git failure (no git, not a
/// repo, non-zero exit) — the caller then just has fewer candidates, never a crash.
fn git_ls(root: &Path, args: &[&str]) -> Vec<String> {
    let mut files = Vec::new();
    if let Ok(o) = std::process::Command::new("git").arg("-C").arg(root).args(args).output() {
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
    fn build_index_on_this_repo_is_fast() {
        // MEASURE, not guess (CLAUDE.md): the "file picker freshness" decision
        // (queue, 2026-07-04) is RE-SCAN ON EVERY SUMMON, on the assumption a
        // real project tree's scan is disk-cheap enough that a summoned overlay
        // — transient by design — never needs a cache. Confirm it against
        // awl-next's OWN tree (a real git repo, not a synthetic fixture) and
        // print the timing. No hard bound is asserted (a raw wall-clock number
        // is too machine/CI-dependent to make a good regression gate) — this is
        // a recorded measurement, not a perf test; see the queue item + the
        // orchestrator report for the number this produced.
        // Real-disk read through the fs seam -> hold TEST_LOCK (mirrors the
        // real-disk tests in app.rs) so a parallel InMemoryFs install can't
        // swallow it.
        let _fs = crate::testlock::serial();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let start = std::time::Instant::now();
        let idx = build_index(&root);
        let elapsed = start.elapsed();
        eprintln!("build_index({}): {} files in {elapsed:?}", root.display(), idx.len());
        assert!(!idx.is_empty(), "this repo has tracked files");
        assert!(idx.iter().any(|p| p == "src/index.rs"), "this very file is tracked");
    }

    #[test]
    fn git_index_lists_untracked_but_not_gitignored_files() {
        // The git strategy shells out to real `git`, so this needs a real on-disk
        // repo (not the InMemoryFs seam). Hold TEST_LOCK so a parallel InMemoryFs
        // install can't swallow the .env walk half (mirrors the real-repo test above).
        let _fs = crate::testlock::serial();
        let base = std::env::temp_dir().join(format!(
            "awl-idx-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git").arg("-C").arg(&base).args(args).output().unwrap()
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(base.join("tracked.md"), "t").unwrap();
        git(&["add", "tracked.md"]);
        git(&["commit", "-qm", "init"]);
        std::fs::write(base.join("brand-new.md"), "n").unwrap(); // untracked, NOT ignored
        std::fs::write(base.join(".gitignore"), "ignored.md\n").unwrap();
        std::fs::write(base.join("ignored.md"), "x").unwrap(); // untracked, gitignored

        let idx = build_index(&base);
        assert!(idx.contains(&"tracked.md".to_string()), "tracked file present: {idx:?}");
        assert!(
            idx.contains(&"brand-new.md".to_string()),
            "untracked-but-not-ignored file must appear (the C-x f freshness fix): {idx:?}"
        );
        assert!(
            !idx.contains(&"ignored.md".to_string()),
            "a gitignored file must still be excluded: {idx:?}"
        );
        std::fs::remove_dir_all(&base).ok();
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
        assert!(!is_env_file("doc-fixture.md"));
    }

    #[test]
    fn hidden_entry_filter() {
        // A dotfile basename hides; a nested dotfile (basename OR ancestor) hides.
        assert!(is_hidden_entry(".gitignore"));
        assert!(is_hidden_entry("sub/.hidden"));
        assert!(is_hidden_entry(".config/x")); // ancestor component starts with '.'
        assert!(is_hidden_entry(".git/config"));
        // The earned exception: `.env*` family files stay VISIBLE.
        assert!(!is_hidden_entry(".env"));
        assert!(!is_hidden_entry(".env.local"));
        assert!(!is_hidden_entry("config/.env.production"));
        // Ordinary files stay visible.
        assert!(!is_hidden_entry("normal.rs"));
        assert!(!is_hidden_entry("src/main.rs"));
        assert!(!is_hidden_entry("doc-fixture.md"));
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
            .with_file("/proj/doc-fixture.md", "r")
            .with_dir("/proj/src")
            .with_file("/proj/docs/guide.md", "g")
            .with_dir("/proj/node_modules");
        crate::fs::with_fs(Arc::new(mem), || {
            // Root level: dirs (docs, src) before files (doc-fixture.md), junk skipped.
            let lvl = list_dir_level(&root, None);
            let names: Vec<&str> = lvl.iter().map(|e| e.name.as_str()).collect();
            assert_eq!(names, vec!["docs", "src", "doc-fixture.md"], "got {names:?}");
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
    fn goto_picker_lands_on_all_then_groups_by_folder() {
        use crate::overlay::{OverlayKind, OverlayState};
        let corpus = vec![
            "doc-fixture.md".to_string(),
            "src/main.rs".to_string(),
            "src/lib.rs".to_string(),
            "notes.txt".to_string(),
        ];
        let mut ov = OverlayState::new(OverlayKind::Goto, corpus, vec![], vec![]);
        // HOME LAW: "All" is FIRST on the strip and the picker LANDS on it (flat list,
        // no sections).
        assert_eq!(ov.lens_strip().first().map(|(l, _)| l.clone()), Some("All".to_string()));
        assert_eq!(ov.active_facet_id(), Some("all"));
        assert_eq!(ov.items.len(), 4);
        assert!(ov.item_sections().iter().all(|s| s.is_empty()));
        // "This folder" (strip index 2): only top-level entries; the src/* files opt out.
        ov.set_facet_lens(2);
        assert_eq!(ov.active_facet_id(), Some("folder"));
        let shown = ov.item_strings();
        assert!(shown.iter().any(|s| s == "doc-fixture.md"));
        assert!(shown.iter().any(|s| s == "notes.txt"));
        assert!(!shown.iter().any(|s| s.contains('/')), "nested files opt out: {shown:?}");
    }

    /// THE UNIFIED-LIST LAW (item 11): the DEFAULT Go-to list (`All`, strip index 0)
    /// mixes the current doc's HEADING rows with its FILE rows, ranked together by
    /// ONE fuzzy filter — the redundant "By type" facet is gone (there is no strip
    /// index 3 left for it; index 3 is now Headings). A query substring that matches
    /// a heading title AND a query substring that matches a filename both surface
    /// their row from the SAME `All` list, and picking a heading row is distinguished
    /// from a file row via `selected_is_heading` (the accept-time split), plus a
    /// dim "heading" secondary-column tag ([`crate::overlay::OverlayState::item_times`]) —
    /// the rowlayout PRIMARY/SECONDARY disambiguator this item asked for.
    #[test]
    fn goto_all_lens_unifies_headings_and_files_in_one_fuzzy_list() {
        use crate::overlay::{OverlayKind, OverlayState};
        const H: &str = OverlayKind::HEADING_MARKER_PREFIX;
        let corpus = vec!["doc-fixture.md".to_string(), "src/main.rs".to_string()];
        let mut ov = OverlayState::new(OverlayKind::Goto, corpus, vec![], vec![]);
        ov.attach_headings(vec![
            ("Introduction".to_string(), 3),
            ("  Widgets".to_string(), 7),
        ]);
        // Strip lost "By type": All / Recent / This folder / Headings — 4 lenses.
        let strip: Vec<String> = ov.lens_strip().into_iter().map(|(l, _)| l).collect();
        assert_eq!(strip, vec!["All", "Recent", "This folder", "Headings"]);
        assert_eq!(ov.active_facet_id(), Some("all"));
        // ALL home: BOTH files AND headings show, mixed — the unified default. A
        // heading row carries the `❡ ` KIND-HINT marker (item 11's rowlayout
        // PRIMARY-cell disambiguator); a file row never does.
        let all = ov.item_strings();
        assert!(all.iter().any(|s| s == "doc-fixture.md"), "{all:?}");
        assert!(all.iter().any(|s| s == "src/main.rs"), "{all:?}");
        assert!(
            all.iter().any(|s| s == &format!("{H}Introduction")),
            "heading row present under All, marked: {all:?}"
        );
        assert!(
            all.iter().any(|s| s == &format!("{H}  Widgets")),
            "heading row present under All, marked: {all:?}"
        );
        assert_eq!(all.len(), 4, "2 files + 2 headings, one flat list");
        // A query substring that ONLY a heading title matches surfaces it from All
        // (the fuzzy filter runs over the RAW corpus title, unaffected by the
        // display-only marker).
        ov.push('w');
        ov.push('i');
        ov.push('d');
        assert_eq!(
            ov.item_strings(),
            vec![format!("{H}  Widgets")],
            "heading-only query: {:?}",
            ov.item_strings()
        );
        assert!(ov.selected_is_heading());
        assert_eq!(ov.selected_line(), Some(7));
        for _ in 0..3 {
            ov.pop();
        }
        // A query substring that ONLY a filename matches surfaces it from the SAME
        // All list — one fuzzy filter reaches both kinds.
        ov.push('m');
        ov.push('a');
        ov.push('i');
        ov.push('n');
        assert_eq!(ov.item_strings(), vec!["src/main.rs".to_string()]);
        assert!(!ov.selected_is_heading());
        for _ in 0..4 {
            ov.pop();
        }
        // A second disambiguator: the secondary column tags a heading row "heading";
        // a file row's secondary cell is blank in headless (no mtime read).
        let times = ov.item_times();
        let idx = |name: &str| all.iter().position(|s| s == name).unwrap();
        assert_eq!(times[idx(&format!("{H}Introduction"))], "heading");
        assert_eq!(times[idx(&format!("{H}  Widgets"))], "heading");
        assert_eq!(times[idx("doc-fixture.md")], "");
        assert_eq!(times[idx("src/main.rs")], "");
    }

    /// GATE: a doc with NO headings still lists its files under All — attaching an
    /// empty heading list is a clean no-op (never a crash, never an empty list).
    #[test]
    fn goto_all_lens_lists_files_even_with_no_headings() {
        use crate::overlay::{OverlayKind, OverlayState};
        let corpus = vec!["a.rs".to_string(), "b.rs".to_string()];
        let mut ov = OverlayState::new(OverlayKind::Goto, corpus, vec![], vec![]);
        ov.attach_headings(Vec::new()); // non-markdown buffer / no headings
        assert_eq!(ov.active_facet_id(), Some("all"));
        assert_eq!(ov.item_strings(), vec!["a.rs".to_string(), "b.rs".to_string()]);
    }

    #[test]
    fn goto_recent_lens_shows_only_opened_files_in_mru_order() {
        use crate::overlay::{OverlayKind, OverlayState};
        let corpus = vec![
            "doc-fixture.md".to_string(),   // 0 — never opened
            "src/main.rs".to_string(), // 1 — opened (2nd most recent)
            "src/lib.rs".to_string(),  // 2 — never opened
            "notes.txt".to_string(),   // 3 — opened (most recent)
        ];
        // The recently-opened MRU (most-recent FIRST) as corpus indices: notes.txt
        // then src/main.rs. doc-fixture + lib were never opened.
        let recent = vec![3usize, 1usize];
        let mut ov = OverlayState::new(OverlayKind::Goto, corpus, vec![], recent);
        ov.set_facet_lens(1);
        assert_eq!(ov.active_facet_id(), Some("recent"));
        // ONLY the opened files show, and in MRU order (most-recent first) — the
        // whole point of the fix (previously this returned the WHOLE corpus).
        assert_eq!(
            ov.item_strings(),
            vec!["notes.txt".to_string(), "src/main.rs".to_string()],
        );
        // Every surviving row sits under the single "Recent" section header.
        assert!(ov.item_sections().iter().all(|s| s == "Recent"));
    }

    #[test]
    fn goto_recent_lens_is_empty_on_a_fresh_session() {
        use crate::overlay::{OverlayKind, OverlayState};
        let corpus = vec!["doc-fixture.md".to_string(), "src/main.rs".to_string()];
        // Nothing opened yet → empty MRU → the Recent lens is EMPTY (shows the empty
        // state), NOT the whole corpus.
        let mut ov = OverlayState::new(OverlayKind::Goto, corpus, vec![], vec![]);
        ov.set_facet_lens(1);
        assert_eq!(ov.active_facet_id(), Some("recent"));
        assert!(
            ov.item_strings().is_empty(),
            "Recent is empty with no opened files: {:?}",
            ov.item_strings()
        );
    }

    #[test]
    fn browse_picker_splits_folders_files_and_git() {
        use crate::overlay::{OverlayKind, OverlayState};
        // One directory level: a git-repo folder, a plain folder, two files.
        let corpus = vec![
            "repo".to_string(),
            "plain".to_string(),
            "a.md".to_string(),
            "b.rs".to_string(),
        ];
        let git = vec![true, false, false, false];
        let is_dir = vec![true, true, false, false];
        let mut ov = OverlayState::new_marked(
            OverlayKind::Browse, corpus, git, is_dir, vec![], vec![], None,
        );
        // Lands on All (flat), All parked first on the strip.
        assert_eq!(ov.lens_strip().first().map(|(l, _)| l.clone()), Some("All".to_string()));
        assert_eq!(ov.active_facet_id(), Some("all"));
        assert_eq!(ov.items.len(), 4);
        // Folders (index 1): only the two directories.
        ov.set_facet_lens(1);
        assert_eq!(ov.active_facet_id(), Some("folders"));
        let f = ov.item_strings();
        assert!(f.iter().any(|s| s.contains("repo")) && f.iter().any(|s| s.contains("plain")));
        assert!(!f.iter().any(|s| s.contains("a.md")), "files hidden under Folders: {f:?}");
        // Files (index 2): only the two files.
        ov.set_facet_lens(2);
        assert_eq!(ov.active_facet_id(), Some("files"));
        let f = ov.item_strings();
        assert!(f.iter().any(|s| s.contains("a.md")) && f.iter().any(|s| s.contains("b.rs")));
        assert!(!f.iter().any(|s| s.contains("repo")), "folders hidden under Files: {f:?}");
        // Git repos (index 3): ONLY the git-marked entry (the task's pinned example).
        ov.set_facet_lens(3);
        assert_eq!(ov.active_facet_id(), Some("git"));
        let g = ov.item_strings();
        assert_eq!(g.len(), 1, "only the git repo appears: {g:?}");
        // The name column is clean (no bullet); the git marker rides the SECONDARY tag.
        assert!(g[0].contains("repo") && !g[0].contains('•'), "git repo name, no bullet: {g:?}");
        assert_eq!(ov.item_git_tags(), vec!["git".to_string()], "the git tag marks it");
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

