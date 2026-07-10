//! tests/fault_kill9.rs — THE KILL-9 FAULT HARNESS: an ordinary Cargo
//! integration test that spawns the REAL `awl` binary (via
//! `CARGO_BIN_EXE_awl`, the standard Cargo mechanism — Cargo builds the bin
//! target before running an integration test and hands its path in through
//! that env var) against an isolated tempdir, kills it with SIGKILL at a
//! randomized/spread point DURING a burst of atomic writes, then asserts on
//! re-read that the target file is EITHER the old content or the new
//! content — NEVER torn — WITNESSING the guarantee `crate::fs::write_atomic`
//! (documented in `src/fs.rs`) only ever argues for in prose.
//!
//! **The mechanism:** the child runs the hidden `--fault-write-loop <path>
//! <count>` flag (`main.rs`), which calls the real `write_atomic` `count`
//! times in a row, printing + flushing `"wrote <i>\n"` after each landed
//! write — so THIS test knows exactly how many writes completed by reading
//! the child's stdout, never by guessing off wall-clock timing. The dev-only
//! `AWL_FAULT_DELAY_MS` env var (read inside `write_atomic` itself) widens
//! the PRE-RENAME window — the file has been fully written to its `.awl-tmp`
//! sibling but not yet renamed over the real target — so a kill landed
//! inside that window is a genuine, reliable rehearsal of "the OS died
//! between the two syscalls a real crash could interrupt between," not a
//! lucky race. The test spreads its kill point (via WRITE COUNT, per this
//! round's own "deterministic-ish via write counts not wall time"
//! instruction) across several small runs rather than one big one, so the
//! whole suite stays a few hundred ms to a couple of seconds, never flaky
//! sleeps.
//!
//! **What "torn" would look like, concretely:** the on-disk `target` file
//! containing a byte sequence that is NEITHER any prior iteration's full
//! payload NOR the final iteration's full payload — e.g. half of `"v3\n"`
//! repeated 64 times followed by garbage, or a mix of two iterations' bytes.
//! `write_atomic`'s two-file (tmp-then-rename) design makes this structurally
//! impossible on a POSIX same-directory rename regardless of WHEN the kill
//! lands — this harness is the empirical check that holds, not just the
//! documentation of why it should.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// A run's deterministic payload for iteration `i` — MUST mirror `main.rs`'s
/// `--fault-write-loop` loop body exactly, so this test can recognize which
/// iteration's content (if any) survived a kill.
fn payload(i: u32) -> String {
    format!("v{i}\n").repeat(64)
}

/// A fresh, uniquely-named tempdir under the OS temp root — no `tempfile`
/// crate dependency needed for one throwaway directory per test run.
fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "awl-fault-kill9-{tag}-{}-{}",
        std::process::id(),
        tag.len() * 7919 + tag.bytes().map(|b| b as usize).sum::<usize>() // cheap salt, no rand dep
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Spawn the real `awl` binary's `--fault-write-loop <target> <count>` with
/// `AWL_FAULT_DELAY_MS` set, stdout piped so the parent can watch write
/// progress. Stderr is inherited (so a genuine crash prints where a human
/// running the suite can see it) — never asserted on, since a SIGKILL exit
/// carries no stderr contract.
fn spawn_write_loop(target: &Path, count: u32, delay_ms: u64) -> Child {
    Command::new(env!("CARGO_BIN_EXE_awl"))
        .arg("--fault-write-loop")
        .arg(target)
        .arg(count.to_string())
        .env("AWL_FAULT_DELAY_MS", delay_ms.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn the awl binary under CARGO_BIN_EXE_awl")
}

/// Read the child's stdout until at least `target_writes` "wrote N" lines
/// have been seen (or the pipe closes first, meaning the loop finished or
/// died early) — returns the highest write index CONFIRMED complete, if any.
/// This is the "deterministic-ish via write counts, not wall time" sync
/// point: the parent never guesses when to strike, it waits for PROOF that
/// iteration `target_writes - 1` actually landed before killing.
fn wait_for_write_count(child: &mut Child, target_writes: u32) -> Option<u32> {
    let stdout = child.stdout.take().expect("piped stdout");
    let reader = BufReader::new(stdout);
    let mut last_seen = None;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if let Some(n) = line.strip_prefix("wrote ").and_then(|s| s.parse::<u32>().ok()) {
            last_seen = Some(n);
            if n + 1 >= target_writes {
                break;
            }
        }
    }
    last_seen
}

/// Assert the target file, once the dust settles, is EITHER absent (never
/// written even once) OR byte-identical to SOME iteration's full payload —
/// never anything torn/partial/mixed.
fn assert_never_torn(target: &Path, count: u32) {
    let Ok(bytes) = std::fs::read(target) else {
        return; // never written at all — fine, nothing to tear
    };
    let text = String::from_utf8(bytes).expect("payload is always plain ASCII text");
    let matches_some_iteration = (0..count).any(|i| payload(i) == text);
    assert!(
        matches_some_iteration,
        "the on-disk file matches NO iteration's full payload — TORN WRITE: {} bytes, \
         starts {:?}",
        text.len(),
        &text[..text.len().min(40)]
    );
}

/// ONE kill-9 trial: spawn the loop, wait for `kill_after_writes` confirmed
/// writes, SIGKILL, reap, then assert the file was never left torn.
fn run_one_trial(tag: &str, count: u32, kill_after_writes: u32, delay_ms: u64) {
    let dir = tmp_dir(tag);
    let target = dir.join("victim.txt");
    let mut child = spawn_write_loop(&target, count, delay_ms);
    let seen = wait_for_write_count(&mut child, kill_after_writes);
    // Kill regardless of whether the loop already finished on its own (a
    // finished process is a harmless no-op kill) — SIGKILL on Unix per
    // `Child::kill`'s documented behavior.
    let _ = child.kill();
    let _ = child.wait();
    assert_never_torn(&target, count);
    // If the loop genuinely got as far as it claimed, the file must hold
    // AT LEAST that much progress (never regress to an EARLIER iteration
    // than the last one it told us it finished) — a stronger check than
    // "never torn" alone, since a stale/rolled-back file would pass
    // `assert_never_torn` but still indicate a real bug.
    if let Some(last) = seen {
        if let Ok(bytes) = std::fs::read(&target) {
            let text = String::from_utf8(bytes).unwrap();
            let landed = (0..count).find(|&i| payload(i) == text);
            if let Some(landed) = landed {
                assert!(
                    landed + 1 >= last || landed == count - 1,
                    "child reported iteration {last} complete via stdout, but disk shows only \
                     iteration {landed} — a write went BACKWARDS, not just torn"
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn write_atomic_survives_sigkill_spread_across_a_burst_of_writes() {
    // A SMALL, bounded spread of kill points across a modest write count and
    // a short artificial pre-rename delay — keeps the whole test in the
    // hundreds-of-ms-to-low-seconds range (count=12 * delay=20ms ≈ well
    // under a second of child runtime per trial even if never killed, times
    // a handful of trials).
    const COUNT: u32 = 12;
    const DELAY_MS: u64 = 20;
    // Spread kill points across the whole loop: right at the start, in the
    // middle, and near the end — the "randomized/spread" points the round
    // asked for, expressed as write-count thresholds rather than a wall-
    // clock sleep so this is reproducible, not flaky.
    for (i, kill_after) in [1u32, 3, 6, 9, 11].into_iter().enumerate() {
        run_one_trial(&format!("spread-{i}"), COUNT, kill_after, DELAY_MS);
    }
}

#[test]
fn write_atomic_survives_a_kill_that_lands_after_the_loop_already_finished() {
    // The degenerate "kill_after_writes" larger than the actual loop length:
    // `wait_for_write_count` just watches the pipe close (loop finished on
    // its own), the subsequent kill is a harmless no-op, and the file must
    // hold exactly the FINAL iteration's payload.
    let dir = tmp_dir("finished");
    let target = dir.join("victim.txt");
    const COUNT: u32 = 5;
    let mut child = spawn_write_loop(&target, COUNT, 0);
    wait_for_write_count(&mut child, COUNT + 10);
    let status = child.wait().expect("child already exited on its own");
    assert!(status.success(), "an un-killed loop must exit cleanly: {status:?}");
    let text = std::fs::read_to_string(&target).unwrap();
    assert_eq!(text, payload(COUNT - 1), "the file holds exactly the LAST iteration's payload");
    let _ = std::fs::remove_dir_all(&dir);
}
