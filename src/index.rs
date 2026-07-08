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
// Go-to's path-derived lenses (This folder / By type), the `recent` flag for Go-to's
// Recent lens (the recently-OPENED-files MRU, [`crate::recent_files`]), the `is_dir`
// / `is_git` flags for Browse's Folders / Files / Git-repos split. No filesystem
// read, no clock inside the bucket.

/// The FIXED section roster for the go-to **By type** lens (a faceting lens's
/// sections must be a `&'static` set, and each item buckets into exactly one). A
/// recognized language extension → `Code`; markdown/text/data get their own bucket;
/// everything else (unknown or extensionless) falls to `Other`. Referenced by
/// [`GOTO_FACET_STRIP`] so the strip and [`goto_type_section`] can never drift.
pub const GOTO_TYPE_SECTIONS: &[&str] = &["Markdown", "Code", "Text", "Data", "Other"];

/// Which [`GOTO_TYPE_SECTIONS`] bucket a root-relative path `rel` sits in, by its
/// filename extension (lower-cased). Extensionless / unrecognized files fall to
/// `Other`; an `.env*` file is `Data`. PURE — the go-to By-type lens's whole rule.
pub fn goto_type_section(rel: &str) -> &'static str {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("md" | "markdown") => "Markdown",
        Some(
            "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "c" | "h" | "cc" | "cpp" | "hpp"
            | "java" | "rb" | "sh" | "lua" | "swift" | "kt" | "php" | "sql" | "css" | "html"
            | "zig" | "hs" | "ml" | "ex" | "exs" | "clj",
        ) => "Code",
        Some("txt" | "text" | "rst" | "org" | "adoc") => "Text",
        Some("toml" | "json" | "yaml" | "yml" | "ini" | "cfg" | "conf" | "env" | "xml") => "Data",
        _ => "Other",
    }
}

/// Go-to's lens strip: **All** (flat home) · **Recent** (recently-OPENED files, a
/// real MRU — EMPTY until you open something) · **This folder** · **By type**. All
/// FIRST (the landing lens); the rest are ←/→ refinements.
const GOTO_FACET_STRIP: [Facet; 4] = [
    Facet { label: "All", id: "all", sections: &[] },
    Facet { label: "Recent", id: "recent", sections: &["Recent"] },
    Facet { label: "This folder", id: "folder", sections: &["This folder"] },
    Facet { label: "By type", id: "type", sections: GOTO_TYPE_SECTIONS },
];

/// Go-to's [`FacetScheme::bucket`], keyed by the strip index (see [`GOTO_FACET_STRIP`]).
/// `Recent` shows ONLY the files ACTUALLY OPENED recently — a real MRU: an item opts
/// IN iff `item.recent` (populated from the persisted recently-opened-files store,
/// [`crate::recent_files`], via [`crate::overlay::OverlayState`]'s `recent` vec) and
/// OUT (returns `None`) otherwise, so on a fresh session with nothing opened the lens
/// is EMPTY and shows the empty state. MRU order (most-recent first) is applied by
/// `refilter`'s MRU tiebreak, not here. `This folder` keeps only top-level entries
/// (no `/` in the path); `By type` buckets by extension.
fn goto_bucket(item: FacetItem, lens_idx: usize) -> Option<&'static str> {
    match lens_idx {
        1 => item.recent.then_some("Recent"), // Recent: ONLY recently-OPENED files (a real MRU)
        2 => (!item.accept.contains('/')).then_some("This folder"), // top level of the root only
        3 => Some(goto_type_section(item.accept)), // By type
        _ => None,                                 // 0 = All (never grouped)
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
        let _fs = crate::fs::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _fs = crate::fs::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        assert!(!is_env_file("README.md"));
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
        assert!(!is_hidden_entry("README.md"));
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
    fn goto_type_section_buckets_by_extension() {
        assert_eq!(goto_type_section("README.md"), "Markdown");
        assert_eq!(goto_type_section("src/main.rs"), "Code");
        assert_eq!(goto_type_section("notes.txt"), "Text");
        assert_eq!(goto_type_section("Cargo.toml"), "Data");
        assert_eq!(goto_type_section("config/.env"), "Data");
        assert_eq!(goto_type_section("Makefile"), "Other"); // extensionless
        assert_eq!(goto_type_section("archive.tar.gz"), "Other"); // last ext only, unknown
        // Every returned label is a member of the FIXED roster (the strip and the
        // bucket can't drift — refilter only keeps a bucket that matches a section).
        for rel in ["a.md", "b.rs", "c.txt", "d.toml", "e", "f.png"] {
            assert!(GOTO_TYPE_SECTIONS.contains(&goto_type_section(rel)), "{rel}");
        }
    }

    #[test]
    fn goto_picker_lands_on_all_then_groups_by_folder_and_type() {
        use crate::overlay::{OverlayKind, OverlayState};
        let corpus = vec![
            "README.md".to_string(),
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
        assert!(shown.iter().any(|s| s == "README.md"));
        assert!(shown.iter().any(|s| s == "notes.txt"));
        assert!(!shown.iter().any(|s| s.contains('/')), "nested files opt out: {shown:?}");
        // "By type" (strip index 3): every row's section == its extension bucket.
        ov.set_facet_lens(3);
        assert_eq!(ov.active_facet_id(), Some("type"));
        let items = ov.item_strings();
        let sections = ov.item_sections();
        assert_eq!(items.len(), sections.len());
        assert_eq!(items.len(), 4, "By type shows every file (each buckets somewhere)");
        for (row, name) in items.iter().enumerate() {
            assert_eq!(sections[row], goto_type_section(name), "row {name} mis-bucketed");
        }
        assert!(sections.contains(&"Markdown".to_string()));
        assert!(sections.contains(&"Code".to_string()));
        assert!(sections.contains(&"Text".to_string()));
    }

    #[test]
    fn goto_recent_lens_shows_only_opened_files_in_mru_order() {
        use crate::overlay::{OverlayKind, OverlayState};
        let corpus = vec![
            "README.md".to_string(),   // 0 — never opened
            "src/main.rs".to_string(), // 1 — opened (2nd most recent)
            "src/lib.rs".to_string(),  // 2 — never opened
            "notes.txt".to_string(),   // 3 — opened (most recent)
        ];
        // The recently-opened MRU (most-recent FIRST) as corpus indices: notes.txt
        // then src/main.rs. README + lib were never opened.
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
        let corpus = vec!["README.md".to_string(), "src/main.rs".to_string()];
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

