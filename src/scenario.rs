//! src/scenario.rs — the HERMETIC SCENARIO FILESYSTEM: one seam that decides
//! scenario-vs-real fs.
//!
//! A SCENARIO run (today: `--screenshot --keys … --strict-replay`, the strict
//! door the storyboard phases build on) is HERMETIC BY DEFAULT: before its
//! config loads, the process-global filesystem ([`crate::fs::active`]) is
//! swapped to an [`InMemoryFs`] SANDBOX seeded from exactly the inputs the
//! command line names — the launch file's bytes and an explicitly-passed
//! config (`--config` / `$AWL_CONFIG`). Every fs consumer downstream — the
//! config load, the buffer open, the project `.git` probe, the index walk, a
//! replayed save, a History read, a Settings open — reads and writes the
//! sandbox, so a scenario NEVER touches the user's real files (config, notes,
//! history, session, scratch stash, autosave; the crash hook and daemon are
//! live-App-only doors no capture opens). External handoffs (URL open, mailto,
//! Trash, download) are already observed-not-performed via
//! [`crate::replay::classify`]'s Intercepted class — together the two seams
//! make a scenario run's only real side effects the PNG + JSON it was asked to
//! write (the harness deliverable itself deliberately bypasses the app seam:
//! `capture` writes it with `std::fs`/`image`, so the sandbox can't swallow
//! the artifact the caller named).
//!
//! The LEGACY paths keep their behavior byte-for-byte: a plain `--screenshot`
//! (or motion/timeline/held capture) still reads the named file — and the
//! user's own config — straight off the real disk, and a replayed save really
//! writes it (CAPTURE.md's documented caveat). Hermeticity is the SCENARIO
//! default, not a regression of the one-off harness.
//!
//! STRUCTURAL hermeticity: [`install_hermetic_fs`] has exactly ONE production
//! call site — `args::parse_args`'s strict-replay arm, BEFORE `Config::load`
//! — so "which fs does this run see" is decided in one place, once, and no
//! later fs consumer can dodge the sandbox (they all go through
//! `fs::active()`). The `.git` probe rides the same seam
//! (`project::Project::resolve`), so a sandboxed root resolves as non-git and
//! the read-only `git` SUBPROCESSES (`git_branch`/`git_dirty` — the one fs
//! reader that bypasses the trait) are structurally never spawned.
//! `tests/hermetic_canary.rs` proves the whole contract on the real binary:
//! a save-bearing strict scenario under a canary HOME/XDG leaves the canary
//! tree byte-identical.
//!
//! Storyboard seeding (phase 5) extends the SEEDS, not the seam: a storyboard
//! hands [`build_sandbox`] more files (fixtures, config, history) through the
//! same door.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::fs::{FileSystem, InMemoryFs};

/// One seeded file: a storyboard input's path + the bytes it carries into the
/// sandbox. Paths are seeded VERBATIM (the sandbox stores keys as given), so
/// the run resolves them exactly as the CLI spelled them.
pub struct Seed {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

/// Gather the CLI-named storyboard inputs from the REAL disk, exactly once.
/// Deliberately `std::fs` (not the seam): seeding is the one boundary crossing
/// INTO the sandbox, performed before it exists. A missing/unreadable input
/// yields no seed — the scenario then sees an absent file, the same degrade
/// `Buffer::from_file` / `Config::load` give the legacy path.
pub fn cli_seeds(file: Option<&Path>, config: Option<&Path>) -> Vec<Seed> {
    let mut seeds = Vec::new();
    for path in [file, config].into_iter().flatten() {
        if let Ok(bytes) = std::fs::read(path) {
            seeds.push(Seed { path: path.to_path_buf(), bytes });
        }
    }
    seeds
}

/// Build the sandbox: every seed written at its own path (parent dirs implied,
/// exactly like a native write into an existing tree), plus a directory marker
/// per named `root` so `read_dir`/`is_dir` on an explicit `--root` see an
/// (empty) directory rather than an error. Pure over its inputs — the caller
/// decides whether to install it.
pub fn build_sandbox(seeds: &[Seed], roots: &[&Path]) -> InMemoryFs {
    let fs = InMemoryFs::new();
    for r in roots {
        // Infallible on the in-memory backend; `let _` keeps the signature simple.
        let _ = fs.create_dir_all(r);
    }
    for s in seeds {
        let _ = fs.write(&s.path, &s.bytes);
    }
    fs
}

/// THE ONE PRODUCTION DOOR: swap the process-global fs to a hermetic sandbox
/// seeded from the CLI-named inputs. Called once, from `args::parse_args`'s
/// strict-replay arm, BEFORE `Config::load` — so the config itself already
/// loads through the sandbox.
///
/// `config_arg` is the explicit `--config` flag; `$AWL_CONFIG` (the same
/// explicit opt-in `config::config_path` honours, in the same precedence) is
/// folded in here so a deliberately pointed-at test config still reaches the
/// scenario. The user's IMPLICIT `~/.config/awl/config.toml` never does: the
/// sandbox simply has no file at the XDG path, so `Config::load` degrades to
/// pure defaults exactly like a machine with no config.
pub fn install_hermetic_fs(file: Option<&Path>, config_arg: Option<&Path>, root: Option<&Path>) {
    let explicit_config: Option<PathBuf> = config_arg
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("AWL_CONFIG").map(PathBuf::from));
    let seeds = cli_seeds(file, explicit_config.as_deref());
    let roots: Vec<&Path> = root.into_iter().collect();
    crate::fs::set_active(Arc::new(build_sandbox(&seeds, &roots)));
}

// NOTE (phase 5): the storyboard runner reuses THIS same door — its document
// rides the `file` seed slot, and the sandbox's `write` already marks every
// seeded file's parent as a directory, so the runner's root resolution + index
// walk see the document's own directory with no extra marker.

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, uniquely-named real tempdir for arranging seed inputs.
    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("awl-scenario-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn cli_seeds_reads_the_named_inputs_and_skips_missing_ones() {
        let dir = tmp_dir("seeds");
        let doc = dir.join("doc.md");
        let cfg = dir.join("cfg.toml");
        std::fs::write(&doc, "# body\n").unwrap();
        std::fs::write(&cfg, "theme = \"Undertow\"\n").unwrap();

        // Both present: two seeds, verbatim bytes, in (file, config) order.
        let seeds = cli_seeds(Some(&doc), Some(&cfg));
        assert_eq!(seeds.len(), 2);
        assert_eq!(seeds[0].path, doc);
        assert_eq!(seeds[0].bytes, b"# body\n");
        assert_eq!(seeds[1].path, cfg);
        assert_eq!(seeds[1].bytes, b"theme = \"Undertow\"\n");

        // A missing input yields NO seed (the scenario sees an absent file —
        // the same degrade the legacy path gives), never an error.
        let missing = dir.join("nope.md");
        assert!(cli_seeds(Some(&missing), None).is_empty());
        assert_eq!(cli_seeds(None, Some(&cfg)).len(), 1);
        assert!(cli_seeds(None, None).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sandbox_contains_exactly_the_seeds_and_the_root_marker() {
        let doc = PathBuf::from("/proj/doc.md");
        let seeds = vec![Seed { path: doc.clone(), bytes: b"alpha\n".to_vec() }];
        let root = PathBuf::from("/proj");
        let fs = build_sandbox(&seeds, &[&root]);
        // The seed is readable at its verbatim path; its parent doubles as the
        // (marked) root dir, so the index walk sees exactly the seeded input.
        assert_eq!(fs.read_to_string(&doc).unwrap(), "alpha\n");
        assert!(fs.is_dir(&root), "the named root is a directory");
        let names: Vec<String> =
            fs.read_dir(&root).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["doc.md".to_string()]);
        // NOTHING else: the user-config shape of path is absent, so a config
        // load inside the sandbox degrades to pure defaults.
        assert!(fs.read_to_string(Path::new("/home/u/.config/awl/config.toml")).is_err());
        // And the root carries no `.git`, so `Project::resolve` classifies it
        // non-git and never spawns the read-only git subprocesses.
        assert!(!fs.exists(&root.join(".git")));
    }

    #[test]
    fn seeded_documents_parent_reads_as_a_directory_with_no_extra_marker() {
        // The storyboard runner resolves its project root from the seeded
        // document's own directory — the sandbox's `write` marks every seeded
        // file's ancestors as dirs, so no storyboard-specific door is needed.
        let doc = PathBuf::from("scenarios/demo.md");
        let seeds = vec![Seed { path: doc.clone(), bytes: b"seeded\n".to_vec() }];
        let fs = build_sandbox(&seeds, &[]);
        assert!(fs.is_dir(Path::new("scenarios")), "parent implied by the seed write");
        let names: Vec<String> =
            fs.read_dir(Path::new("scenarios")).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["demo.md".to_string()]);
    }

    #[test]
    fn install_hermetic_fs_swaps_the_active_backend_to_the_seeded_sandbox() {
        let dir = tmp_dir("install");
        let doc = dir.join("doc.md");
        std::fs::write(&doc, "real bytes\n").unwrap();
        // FsGuard(current) restores whatever `install_hermetic_fs` swaps in —
        // even on a failed assert — so no sibling test ever sees the sandbox.
        let _restore = crate::fs::FsGuard::install(crate::fs::active());
        install_hermetic_fs(Some(&doc), None, Some(&dir));
        // The active backend now serves the seeded copy…
        assert_eq!(crate::fs::active().read_to_string(&doc).unwrap(), "real bytes\n");
        // …and a write through the seam lands in the sandbox, NEVER on disk.
        crate::fs::active().write(&doc, b"sandbox edit\n").unwrap();
        assert_eq!(crate::fs::active().read_to_string(&doc).unwrap(), "sandbox edit\n");
        assert_eq!(
            std::fs::read_to_string(&doc).unwrap(),
            "real bytes\n",
            "the REAL file keeps every byte"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
