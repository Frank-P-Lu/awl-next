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
    let mut out = if root.join(".git").exists() {
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
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        let name = entry.file_name().to_string_lossy().to_string();
        if ft.is_dir() {
            if is_junk_dir(&name) {
                continue;
            }
            walk_collect(root, &path, out, keep);
        } else if ft.is_file() && keep(&name) {
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
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let name = entry.file_name().to_string_lossy().to_string();
        if ft.is_dir() {
            if is_junk_dir(&name) {
                continue;
            }
            let is_git = entry.path().join(".git").exists();
            dirs.push(DirEntry {
                name,
                is_dir: true,
                is_git,
            });
        } else if ft.is_file() {
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
        let root = tmp("walk");
        fs::write(root.join("notes.md"), "n").unwrap();
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/ideas.md"), "i").unwrap();
        fs::create_dir_all(root.join("node_modules")).unwrap();
        fs::write(root.join("node_modules/big.js"), "x").unwrap();
        let idx = build_index(&root);
        assert!(idx.contains(&"notes.md".to_string()));
        assert!(idx.contains(&"sub/ideas.md".to_string()));
        assert!(
            !idx.iter().any(|p| p.contains("node_modules")),
            "junk dir must be pruned: {idx:?}"
        );
        let _ = fs::remove_dir_all(&root);
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
        let root = tmp("level");
        fs::write(root.join("README.md"), "r").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("docs/guide.md"), "g").unwrap();
        fs::create_dir_all(root.join("node_modules")).unwrap();
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
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn dir_level_marks_git_child() {
        let root = tmp("gitchild");
        fs::create_dir_all(root.join("repo/.git")).unwrap();
        fs::create_dir_all(root.join("plain")).unwrap();
        let lvl = list_dir_level(&root, None);
        let repo = lvl.iter().find(|e| e.name == "repo").unwrap();
        let plain = lvl.iter().find(|e| e.name == "plain").unwrap();
        assert!(repo.is_git, "repo with .git must be marked git");
        assert!(!plain.is_git);
        let _ = fs::remove_dir_all(&root);
    }
}
