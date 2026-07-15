//! tests/storyboard_film.rs — the STORYBOARD end-to-end contract, proven on
//! the real binary (`CARGO_BIN_EXE_awl`, the `hermetic_canary.rs` mechanism):
//!
//!   1. **The demo storyboard runs end-to-end** emitting every artifact —
//!      per-step PNG+JSON, film frames, a trace.json with `abort: null`.
//!   2. **Byte-identity** — running the same storyboard twice produces a
//!      byte-identical trace.json AND byte-identical frames (the whole output
//!      tree is compared file-by-file).
//!   3. **The abort fixture aborts naming the effect** — stderr and the
//!      partial trace.json both name `finish_buffer` / `FinishBuffer`, and the
//!      process exits non-zero.
//!   4. **The optional film encode rides a local ffmpeg** — exercised with a
//!      stub `ffmpeg` on PATH (arg plumbing + success handling), so the seam
//!      is pinned without depending on a real encoder being installed.
//!
//! macOS-gated: the checked-in storyboards speak the Mac-convention chords
//! (`s-Down`, `s-p`, `s-w` — the advertised keymap), and the film renderer
//! needs a real GPU adapter (a GPU-less host is detected and skipped, the
//! repo's "no wgpu adapter" pattern).

#![cfg(target_os = "macos")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh, uniquely-named tempdir under the OS temp root (no `tempfile` dep —
/// mirrors `tests/hermetic_canary.rs`).
fn tmp_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("awl-storyboard-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Spawn the real binary from the repo root (so `scenarios/…` resolves), with
/// `$AWL_CONFIG` scrubbed (a pointed-at config would seed the sandbox).
fn run_awl(args: &[&str], extra_path: Option<&Path>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_awl"));
    cmd.args(args).current_dir(env!("CARGO_MANIFEST_DIR")).env_remove("AWL_CONFIG");
    if let Some(dir) = extra_path {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut parts = vec![dir.to_path_buf()];
        parts.extend(std::env::split_paths(&path));
        cmd.env("PATH", std::env::join_paths(parts).unwrap());
    }
    cmd.output().expect("failed to spawn the awl binary under CARGO_BIN_EXE_awl")
}

/// GPU-less host? The storyboard runner refuses up front (the strict missing-
/// oracle error) or the device request fails — either way, skip honestly.
fn is_gpu_less(out: &Output) -> bool {
    let err = String::from_utf8_lossy(&out.stderr);
    err.contains("layout oracle unavailable") || err.contains("adapter")
}

/// Recursive snapshot of a tree: relative path → bytes (`None` for a dir).
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

#[test]
fn demo_storyboard_emits_every_artifact_and_two_runs_are_byte_identical() {
    let run1 = tmp_dir("run1");
    let run2 = tmp_dir("run2");
    let out1 = run_awl(
        &["--storyboard", "scenarios/demo.toml", "--storyboard-out", run1.to_str().unwrap()],
        None,
    );
    if !out1.status.success() && is_gpu_less(&out1) {
        eprintln!("skipping demo_storyboard test: no wgpu adapter on this host");
        return;
    }
    assert!(
        out1.status.success(),
        "demo run failed:\n{}",
        String::from_utf8_lossy(&out1.stderr)
    );

    // 1) Every artifact of a clean run exists.
    let trace = std::fs::read_to_string(run1.join("trace.json")).expect("trace.json written");
    assert!(trace.contains("\"schema\": \"awl-trace/1\""), "trace schema: {trace}");
    assert!(trace.contains("\"abort\": null"), "clean run records no abort");
    // Every action step's PNG + sidecar (step 2 is the first expect step — it
    // renders nothing by design); the film frames; the film-frame identity of
    // a step artifact (step-000.png IS its last film frame, byte-for-byte).
    for stem in ["step-000", "step-001", "step-003"] {
        assert!(run1.join(format!("{stem}.png")).exists(), "{stem}.png exists");
        assert!(run1.join(format!("{stem}.json")).exists(), "{stem}.json exists");
    }
    assert!(!run1.join("step-002.png").exists(), "an expect step renders nothing");
    let frames: Vec<_> = std::fs::read_dir(run1.join("frames")).unwrap().flatten().collect();
    assert!(frames.len() >= 40, "the demo films dozens of frames, got {}", frames.len());
    assert_eq!(
        std::fs::read(run1.join("step-000.png")).unwrap(),
        std::fs::read(run1.join("frames/frame-00000.png")).unwrap(),
        "a step artifact IS its film frame"
    );
    // The step sidecar is the ordinary plain-schema capture sidecar.
    let step_json = std::fs::read_to_string(run1.join("step-005.json")).unwrap();
    assert!(step_json.contains("\"schema\": \"awl-capture/"), "plain sidecar: {step_json}");
    assert!(step_json.contains("\"query\": \"fox\""), "step 5 shows the typed search query");

    // 2) Byte-identity: the second run's whole tree equals the first's.
    let out2 = run_awl(
        &["--storyboard", "scenarios/demo.toml", "--storyboard-out", run2.to_str().unwrap()],
        None,
    );
    assert!(out2.status.success(), "second run failed");
    assert_eq!(
        tree_snapshot(&run1),
        tree_snapshot(&run2),
        "two runs of the same storyboard are byte-identical (trace + frames + steps)"
    );

    let _ = std::fs::remove_dir_all(&run1);
    let _ = std::fs::remove_dir_all(&run2);
}

#[test]
fn abort_fixture_aborts_naming_the_unsupported_effect() {
    let out_dir = tmp_dir("abort");
    let out = run_awl(
        &[
            "--storyboard",
            "scenarios/abort-unsupported.toml",
            "--storyboard-out",
            out_dir.to_str().unwrap(),
        ],
        None,
    );
    if !out.status.success() && is_gpu_less(&out) {
        eprintln!("skipping abort_fixture test: no wgpu adapter on this host");
        return;
    }
    assert!(!out.status.success(), "the abort fixture must exit non-zero");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("unsupported effect `finish_buffer`"), "stderr names the effect: {err}");
    assert!(err.contains("FinishBuffer"), "stderr names the action: {err}");
    // The partial trace records the same abort, byte-for-byte reason.
    let trace = std::fs::read_to_string(out_dir.join("trace.json")).expect("partial trace written");
    assert!(trace.contains("\"abort\": { \"step\": 2"), "abort step recorded: {trace}");
    assert!(trace.contains("unsupported effect `finish_buffer`"), "abort reason: {trace}");
    assert!(trace.contains("\"class\": \"unsupported\""), "the offending chord's record: {trace}");
    // The pre-abort steps still produced their artifacts.
    assert!(out_dir.join("step-000.png").exists());
    let _ = std::fs::remove_dir_all(&out_dir);
}

#[test]
fn film_encode_rides_a_local_ffmpeg_when_present() {
    // A stub `ffmpeg` that validates nothing but writes its LAST argument (the
    // output file) — pins the spawn + arg-order + success plumbing without
    // depending on a real encoder. A broken/absent real ffmpeg is exactly the
    // documented degrade (frames retained, note printed), covered above by the
    // demo run's success either way.
    let bin_dir = tmp_dir("stub-bin");
    let stub = bin_dir.join("ffmpeg");
    std::fs::write(&stub, "#!/bin/sh\nfor a in \"$@\"; do out=\"$a\"; done\nprintf stub > \"$out\"\n")
        .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();

    let out_dir = tmp_dir("films");
    let out = run_awl(
        &["--storyboard", "scenarios/demo.toml", "--storyboard-out", out_dir.to_str().unwrap()],
        Some(&bin_dir),
    );
    if !out.status.success() && is_gpu_less(&out) {
        eprintln!("skipping film_encode test: no wgpu adapter on this host");
        return;
    }
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out_dir.join("film.webm").exists(), "film.webm written via ffmpeg: {stdout}");
    assert!(out_dir.join("film.mp4").exists(), "film.mp4 written via ffmpeg: {stdout}");
    assert!(stdout.contains("film.webm") && stdout.contains("film.mp4"), "{stdout}");
    // Raw frames are retained even when encoding succeeded.
    assert!(out_dir.join("frames").join("frame-00000.png").exists());
    let _ = std::fs::remove_dir_all(&bin_dir);
    let _ = std::fs::remove_dir_all(&out_dir);
}
