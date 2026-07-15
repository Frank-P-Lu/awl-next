//! tests/hermetic_canary.rs — THE HERMETIC-SCENARIO CANARY: an ordinary Cargo
//! integration test that spawns the REAL `awl` binary (via `CARGO_BIN_EXE_awl`,
//! the same mechanism as `tests/fault_kill9.rs`) under a CANARY HOME/XDG tree
//! salted with bait user files (a config naming a distinctive theme, a notes
//! dir, a scratch stash, a history log), drives a SAVE-BEARING strict scenario
//! (`--screenshot --keys "X s-s Cmd-S-h Esc" --strict-replay`), and asserts:
//!
//!   1. **Zero unexpected filesystem writes** — the canary home AND the input
//!      file are byte-identical after the run (a full recursive snapshot
//!      compare), even though the scenario EDITED and SAVED the document and
//!      summoned the History picker. The save landed in the in-memory sandbox
//!      (`src/scenario.rs`); the only real writes are the PNG + JSON the
//!      command line asked for.
//!   2. **The bait config was never read** — the sidecar reports the built-in
//!      default theme, not the canary's `theme = "Wagtail"`.
//!   3. **The scenario really ran** — the sidecar `text` carries the edit.
//!   4. **Legacy is NOT hermetic (no regression)** — the same capture WITHOUT
//!      `--strict-replay` still reads the user config off the real disk (the
//!      sidecar reports Wagtail), exactly today's documented behavior.
//!
//! This is the spec's canary: hermeticity proven on the real binary through
//! the real `parse_args` → sandbox-install → config-load → replay → capture
//! pipeline, not on an in-process approximation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh, uniquely-named tempdir under the OS temp root (no `tempfile` dep —
/// mirrors `tests/fault_kill9.rs`).
fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("awl-hermetic-canary-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Recursive snapshot of a tree: every path (relative to `root`) → its bytes
/// (`None` for a directory). A new file, a changed byte, a new directory, or a
/// deletion all change the map — "zero unexpected writes" is map equality.
fn tree_snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let p = entry.path();
            let rel = p.strip_prefix(root).unwrap().to_path_buf();
            if p.is_dir() {
                out.insert(rel, None);
                walk(root, &p, out);
            } else {
                out.insert(rel, Some(std::fs::read(&p).unwrap_or_default()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// Spawn the real binary with the canary HOME/XDG environment, panicking (with
/// the child's stderr) if it exits non-zero. The child's CONVENTION is pinned
/// to Mac: this test's `--keys` spec speaks the repo's advertised
/// Mac-convention chords (`s-s` = Cmd-S), so the chords must resolve the same
/// way regardless of the host's `cfg(target_os)` or an ambient
/// `AWL_CONVENTION_FORCE=linux` sweep leaking into the child — the env
/// override wins over the target default (see `Convention::current`), exactly
/// like the suite's convention-parametric tests pin their expectations.
fn run_awl(home: &Path, args: &[&str]) {
    let out = Command::new(env!("CARGO_BIN_EXE_awl"))
        .args(args)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local").join("share"))
        .env("AWL_CONVENTION_FORCE", "mac")
        .env_remove("AWL_CONFIG")
        .env_remove("AWL_CJK_FORCE")
        .env_remove("AWL_FAULT_DELAY_MS")
        .output()
        .expect("failed to spawn the awl binary under CARGO_BIN_EXE_awl");
    assert!(
        out.status.success(),
        "awl {:?} failed: {}\n{}",
        args,
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Parse a capture's sidecar JSON.
fn sidecar(png: &Path) -> serde_json::Value {
    let json = std::fs::read_to_string(png.with_extension("json")).expect("sidecar exists");
    serde_json::from_str(&json).expect("sidecar parses")
}

#[test]
fn strict_scenario_under_a_canary_home_makes_zero_unexpected_writes() {
    let root = tmp_dir("tree");
    let home = root.join("home");
    let inputs = root.join("inputs");
    let outdir = root.join("out");

    // Salt the canary with the user-file surfaces the hermetic contract names:
    // config (with a distinctive theme as READ-bait), notes, the scratch
    // stash, a history log. Any write into — or read applied from — these is a
    // hermeticity breach.
    let cfg_dir = home.join(".config").join("awl");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::write(cfg_dir.join("config.toml"), "theme = \"Wagtail\"\n").unwrap();
    let data_dir = home.join(".local").join("share").join("awl");
    std::fs::create_dir_all(data_dir.join("history")).unwrap();
    std::fs::write(data_dir.join("scratch.md"), "user scratch bait\n").unwrap();
    std::fs::write(data_dir.join("history").join("bait.log"), "history bait\n").unwrap();
    std::fs::create_dir_all(home.join("notes")).unwrap();
    std::fs::write(home.join("notes").join("keep.md"), "notes bait\n").unwrap();

    // The storyboard input: a real file the scenario will open, EDIT, and SAVE.
    std::fs::create_dir_all(&inputs).unwrap();
    let doc = inputs.join("doc.md");
    std::fs::write(&doc, "alpha\n").unwrap();
    std::fs::create_dir_all(&outdir).unwrap();

    let home_before = tree_snapshot(&home);
    let inputs_before = tree_snapshot(&inputs);

    // ── The HERMETIC leg: edit + save + summon History, strictly. ──
    let cap = outdir.join("cap.png");
    run_awl(
        &home,
        &[
            "--screenshot",
            cap.to_str().unwrap(),
            "--keys",
            "X s-s Cmd-S-h Esc",
            "--strict-replay",
            doc.to_str().unwrap(),
        ],
    );

    // The deliverable landed (the ONE expected real write, in its own out dir)…
    assert!(cap.exists(), "the capture PNG is still written for a hermetic run");
    let v = sidecar(&cap);
    // …the scenario really ran (the edit is in the captured text)…
    assert_eq!(v["text"].as_str().unwrap(), "Xalpha\n", "the replayed edit applied");
    // …the bait config was never read (built-in default theme, not Wagtail)…
    assert_ne!(
        v["theme"]["name"].as_str().unwrap(),
        "Wagtail",
        "a hermetic scenario must not read the user's real config"
    );
    // …and the canary is BYTE-IDENTICAL: the save went to the sandbox, the
    // History summon read the sandbox, nothing touched the user's tree or the
    // input file.
    assert_eq!(
        tree_snapshot(&home),
        home_before,
        "zero writes under the canary HOME after a save-bearing strict scenario"
    );
    assert_eq!(
        tree_snapshot(&inputs),
        inputs_before,
        "the real input file keeps every byte the scenario 'saved' over"
    );

    // ── The LEGACY leg (no --strict-replay): today's real-fs behavior kept. ──
    // A plain capture (no --keys, so nothing is saved) DOES read the user's
    // config off the real disk — the sticky Wagtail theme shows in the sidecar.
    let legacy = outdir.join("legacy.png");
    run_awl(&home, &["--screenshot", legacy.to_str().unwrap(), doc.to_str().unwrap()]);
    let v = sidecar(&legacy);
    assert_eq!(
        v["theme"]["name"].as_str().unwrap(),
        "Wagtail",
        "the legacy path still reads the user's real config (hermeticity is the \
         scenario default, not a regression of the one-off harness)"
    );
    assert_eq!(v["text"].as_str().unwrap(), "alpha\n", "legacy read the real file's bytes");
    // A plain read-only capture writes nothing user-owned either (config is
    // read, never written, from this path).
    assert_eq!(tree_snapshot(&home), home_before, "legacy plain capture writes nothing");
    assert_eq!(tree_snapshot(&inputs), inputs_before);

    let _ = std::fs::remove_dir_all(&root);
}
