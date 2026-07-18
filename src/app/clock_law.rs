//! THE CLOCK-SEAM LAW (grep-law test, mirrors `println_audit`/`theme_caps_law`'s
//! scanner shape): structurally fences the `app` module — every file under
//! `src/app.rs` + `src/app/**` — against a raw `Instant::now` re-appearing on the
//! SCHEDULING / ANIMATION path. That path is the whole reason [`crate::clock`]'s
//! `Clock` seam exists: every debounce/settle deadline (`app::schedule`), the
//! caret-spring frame `dt`, the ambient tick, toast expiry, GPU-retry timing, and
//! the App's own sense-of-time stamps (session origin, save marks, key→px input
//! receipt) now read `self.clock.now()`, so a future deterministic clock can STEP
//! the whole path frame-by-frame under the headless harness. A new raw
//! `Instant::now` there would be a silent hole in that seam.
//!
//! Scope is the `app` module only, mirroring `theme_caps_law`'s `src/render/**`
//! scoping: category-(a) time reads (the scheduling/animation ones this law
//! governs) all live here. Genuine real-WORK MEASUREMENT — the `--bench-*`
//! harnesses, `--soak-gpu`'s own injected start instant, the crash-log/probe
//! diagnostics — legitimately reads the raw monotonic clock (a virtual clock
//! would report a fictional duration for real work) and lives OUTSIDE the `app`
//! module, so it is out of scope here by construction.
//!
//! The [`ALLOWED`] table is the small, explicit allow-list: the only raw
//! `Instant::now` reads that MAY remain inside the `app` module, each a
//! real-work MEASUREMENT (not scheduling), at its exact current count. A read
//! appearing anywhere else — or a changed count in an allow-listed file —
//! fails loudly: route it through `self.clock`, or justify it here.
//!
//! Skips (identical shape to the sibling scanners): every `#[cfg(test)]`-gated
//! item (brace-balanced), any file literally named `tests.rs`, any `tests/`
//! directory, this file itself (its doc + table name `Instant::now` in prose),
//! and doc/plain comment lines (`//`/`///`/`//!` — history prose, not code).

/// The audited allow-list: raw `Instant::now` reads that MAY stay inside the
/// `app` module, keyed by path relative to `src/`, with the exact expected
/// count. Every entry is a real-WORK MEASUREMENT reading wall time by
/// necessity — NOT a scheduling/animation read (those all route through
/// `self.clock`). Any file NOT listed here must contain zero raw reads.
const ALLOWED: &[(&str, usize)] = &[
    // `Gpu::redraw`'s three GPU-STAGE perf stamps (`debug.then(Instant::now)`
    // for prepare / post-acquire / present-return), on the `Gpu` render backend
    // — not the App, and gated on the DEBUG panel. They measure the real GPU
    // submit latency the debug panel reports; a virtual clock would report a
    // fictional GPU cost, and the determinism contract already makes these
    // capture-invisible (fixed placeholders headless).
    ("app/gpu.rs", 3),
    // `set_dictionary`'s parse-cost measurement (`parsed in {:.2}ms`): times the
    // real dictionary reconstruction — wall-clock by necessity, a diagnostic,
    // not a scheduled deadline.
    ("app/files.rs", 1),
    // The `--soak-gpu` recovery-latency feed (`observe_recovered(kind, ..)`):
    // records WHEN a GPU-fault recovery presented so the soak report can measure
    // its real duration. `--soak-gpu` is an isolated, live-only stress harness
    // that always runs on real time (never a virtual clock).
    ("app/window.rs", 1),
];

/// Count raw `Instant::now` needles on this line — matches both the call form
/// `Instant::now()` and the fn-pointer form `Instant::now` (and the fully
/// qualified `std::time::Instant::now`), the same substring in every case.
/// `self.clock.now()` does NOT contain the needle, so a routed read never
/// counts. Returns 0 for a doc/plain comment line (prose, not code).
fn needle_count(line: &str) -> usize {
    if line.trim_start().starts_with("//") {
        return 0; // doc comment or plain comment — prose that may name the needle.
    }
    line.matches("Instant::now").count()
}

/// Scan `text`, skipping every `#[cfg(test)...]`-gated item (brace-balanced,
/// identical shape to `println_audit`/`theme_caps_law`'s scanners), and sum the
/// needle count over every line OUTSIDE a skipped region.
fn scan_file(text: &str) -> usize {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Normal,
        AfterCfgTest,
        InSkippedBlock(i32),
    }
    let mut state = State::Normal;
    let mut n = 0usize;
    for line in text.lines() {
        state = match state {
            State::Normal => {
                let t = line.trim_start();
                if t.starts_with("#[cfg(test)") || t.starts_with("#[cfg(all(test") {
                    State::AfterCfgTest
                } else {
                    n += needle_count(line);
                    State::Normal
                }
            }
            State::AfterCfgTest => {
                let t = line.trim_start();
                if t.starts_with("#[") {
                    State::AfterCfgTest
                } else if line.contains('{') {
                    let d = line.matches('{').count() as i32 - line.matches('}').count() as i32;
                    if d <= 0 {
                        State::Normal
                    } else {
                        State::InSkippedBlock(d)
                    }
                } else if line.trim_end().ends_with(';') {
                    State::Normal
                } else {
                    State::AfterCfgTest
                }
            }
            State::InSkippedBlock(depth) => {
                let d = depth + line.matches('{').count() as i32 - line.matches('}').count() as i32;
                if d <= 0 {
                    State::Normal
                } else {
                    State::InSkippedBlock(d)
                }
            }
        };
    }
    n
}

/// Walk `dir` recursively, accumulating `src/`-relative path → needle count for
/// every non-test `.rs` file. Skips `tests/` dirs, `tests.rs` files, and this
/// scanner file (self-match on its own doc/table).
fn scan_dir(
    src_root: &std::path::Path,
    dir: &std::path::Path,
    counts: &mut std::collections::BTreeMap<String, usize>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some("tests") {
                continue;
            }
            scan_dir(src_root, &path, counts);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if fname == "tests.rs" || fname == "clock_law.rs" {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let n = scan_file(&text);
        if n == 0 {
            continue;
        }
        let rel = path.strip_prefix(src_root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        counts.insert(rel, n);
    }
}

#[test]
fn app_module_reads_time_only_through_the_clock_seam() {
    let src_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();

    // The `app` module = the root file `src/app.rs` + everything under `src/app/`.
    let app_rs = src_root.join("app.rs");
    if let Ok(text) = std::fs::read_to_string(&app_rs) {
        let n = scan_file(&text);
        if n > 0 {
            counts.insert("app.rs".to_string(), n);
        }
    }
    scan_dir(&src_root, &src_root.join("app"), &mut counts);

    let expected: std::collections::BTreeMap<String, usize> =
        ALLOWED.iter().map(|(f, n)| (f.to_string(), *n)).collect();

    assert_eq!(
        counts, expected,
        "a raw `Instant::now` appeared in the `app` module outside the allow-listed \
         real-work-measurement sites (or an allow-listed count changed) — the scheduling / \
         animation path must read `self.clock.now()` so a deterministic clock can step it. \
         Route it through `self.clock`, or (only if it genuinely measures real elapsed work) \
         add it to `clock_law::ALLOWED` with a reason. See this module's doc comment."
    );
}

#[test]
fn needle_count_matches_both_call_and_fn_pointer_forms_and_skips_comments() {
    assert_eq!(needle_count("        let deadline = Instant::now() >= x;"), 1);
    assert_eq!(needle_count("        let t0 = debug.then(Instant::now);"), 1);
    assert_eq!(needle_count("        let t0 = std::time::Instant::now();"), 1);
    assert_eq!(needle_count("        self.x = Some(self.clock.now());"), 0);
    assert_eq!(needle_count("    /// reads `Instant::now()` — history prose"), 0);
    assert_eq!(needle_count("    // mentions Instant::now in a comment"), 0);
}

#[test]
fn scan_file_skips_cfg_test_instant_reads() {
    let text = "\
fn live() { let a = Instant::now(); }
#[cfg(test)]
mod t {
    fn f() { let b = Instant::now(); let c = Instant::now(); }
}
fn also_live() { let d = Instant::now(); }
";
    // Only the two non-test `Instant::now` reads count; the cfg(test) block's
    // two are skipped.
    assert_eq!(scan_file(text), 2);
}
