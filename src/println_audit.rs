//! THE PRINTLN AUDIT (SAVE-FEEDBACK round): a structural law test guarding
//! against a println!/eprintln! call sneaking into a RUNTIME path where it
//! would be invisible on a GUI launch (the reported bug — a `Cmd-S` failure on
//! Linux only ever printed to the TERMINAL, never in-app) and, worse, actively
//! confusing on a terminal launch (state-toggle chatter like "page mode: on"
//! narrating every keystroke to stdout nobody is reading).
//!
//! Every user-facing `println!`/`eprintln!` in `src/` earns one of three fates
//! (see `CLAUDE.md`'s SAVE-FEEDBACK round note for the full table):
//!   (a) ROUTED through the existing bottom-center notice seam (`App::notice`)
//!       — save failures, an explicit "Move note" failure, anything the user
//!       should see;
//!   (b) SILENCED — state-toggle chatter the UI already shows some other way
//!       (the toggle's own visible effect, the window title, …);
//!   (c) KEPT, with a reason — genuine CLI/diagnostic output: `--help`,
//!       `--screenshot`'s "wrote …" deliverable, the bench harnesses' tabular
//!       output, startup-before-a-window-exists errors (a malformed config,
//!       a font/dictionary/GPU init failure), and other best-effort background
//!       bookkeeping whose failure is rare and non-fatal (candidates for a
//!       future notice-routing pass, not fixed this round — see the table).
//!
//! [`no_stray_println_outside_the_audited_table`] is the tripwire: it walks
//! `src/` (mirroring `durable.rs`'s own bare-write scanner — the same
//! precedent, generalized from "durable write calls" to "print macro calls"),
//! SKIPPING every `#[cfg(test)]`-gated item (a brace-balanced skip, so a test
//! module's own `eprintln!("skipping …: no wgpu adapter")` fixtures — the vast
//! majority of the raw count in this crate — never count against the law) and
//! every file living under a `tests/` directory or literally named `tests.rs`,
//! then compares the per-file match COUNT against the audited table below. A
//! new print macro appearing in an already-audited file changes that file's
//! count and fails loudly; a print macro appearing in a NEW file fails loudly
//! too (the file is simply absent from the table). Either way: route it
//! through the notice seam, silence it, or add it here with a reason — never
//! let it slip back into the terminal unaccounted for.

/// The audited, CURRENT expected count of print-macro call sites per file
/// (relative to `src/`), for every file NOT under a `tests/` directory and NOT
/// literally named `tests.rs` (see the scanner below for the cfg(test) skip).
/// Each row's reason is the file's fate (c) unless noted — (a)/(b) fates left
/// no residual print behind, so they don't appear here at all.
const EXPECTED: &[(&str, usize)] = &[
    // Startup / rare live-only failure paths, largely before or around a
    // usable window (spell dictionary / clipboard / render-state init, the
    // daemon socket) — none has a `self.notice` seam to route through this
    // early, and each is a one-time, non-recurring condition.
    ("app.rs", 6),
    // LIVE PROBE harness protocol/diagnostic lines (fate (c), CLI harness
    // output): the driver's ready-timeout warning + the ONE `PROBE-TRACE …`
    // owner (`probe.rs::trace`, the single stderr print site every present /
    // crossing / move trace routes through, so the scattered call sites in
    // `app/apply.rs` / `app/gpu.rs` / `app/window.rs` carry no print macro of
    // their own), the per-shot `LIVE-PROBE shot …` protocol lines the wrapping
    // script asserts on (`app/probe.rs`), and `app.rs`'s shots-dir creation
    // failure (counted in the row above). The third print site is the FLIGHT
    // RECORDER's `AWL_FLIGHT_RECORDER` open-failure warning (`init_flight`) — a
    // one-time startup-before-notice diagnostic (fate (c)), like the config/GPU
    // init failures above; when the recorder can't open its file it says so and
    // stays off rather than failing the launch.
    ("probe.rs", 3),
    ("app/probe.rs", 5),
    // "follow link: could not open …" — a rare OS-handoff failure
    // (C-c C-o). Flagged as a future notice-routing candidate, not fixed
    // this round (out of the reported bug's scope).
    ("app/apply.rs", 1),
    // Best-effort background bookkeeping failures (config/credits/guide
    // write, a sticky-pref/rebind write, the recent-files/projects MRU
    // save, a dictionary switch, the autosave/scratch-stash engine) — all
    // rare, all non-fatal by design ("never disrupt the edit/save").
    // Flagged as future notice-routing candidates; not fixed this round to
    // keep the round's own diff focused on the reported bug (manual Save +
    // the toggle-chatter class it named explicitly). The USER GUIDE round
    // added ONE more (`open_guide`'s on-disk-refresh failure), mirroring
    // `open_credits`'s existing one verbatim. Item 39 (Add to dictionary) adds
    // ONE more: `add_to_dictionary`'s rare personal-dictionary FILE-append
    // failure (I/O error, unresolvable path) — non-fatal by design (the word is
    // already silenced in memory that session), same best-effort-write class as
    // the sticky-pref writes above; a future notice-routing candidate.
    ("app/files.rs", 14),
    // GPU/render-pipeline errors (`prepare`/`render`) retain a stderr
    // diagnostic while App-owned recovery also paints the calm notice.
    ("app/gpu.rs", 2),
    // Classified callback/device/surface failures are logged once at the
    // App recovery seam as well as being routed through the visible notice.
    ("app/window.rs", 1),
    ("app/session.rs", 1),
    ("app/stats.rs", 1),
    // WRITING STREAKS: the `streaks save failed: {e}` stderr line (a failed atomic
    // write of `streaks.toml` must never disrupt the editor — it warns and moves on,
    // mirroring `app/stats.rs`'s own `stats save failed` line).
    ("app/streaks.rs", 1),
    // `--bench-typing`'s tabular CLI output.
    ("bench.rs", 4),
    // "buffer registry over cap" — an edge-case warning (every backgrounded
    // buffer is dirty), not part of this round's reported bug.
    ("buffers.rs", 1),
    // Headless capture harness diagnostics ("spell-check disabled for
    // capture: …") — CLI/test-harness output, not live-app chatter.
    ("capture/animated.rs", 2),
    ("capture/modes.rs", 1),
    ("capture/oracle.rs", 1),
    // Config TOML parse error — startup, before a window exists.
    ("config/model.rs", 1),
    // `[keys]`/`linux_keep_emacs` config-authoring diagnostics (an unknown
    // action name / an unparseable chord, incl. the emacs-keep-list parse)
    // — reachable both at startup (legitimately a stderr diagnostic) and
    // via a LIVE Settings-buffer reload; the live half is a logged gap, not
    // fixed this round.
    ("keymap.rs", 4),
    // `--help` + other CLI-only output.
    ("main.rs", 2),
    // `--help`'s big usage dump, plus `--list-worlds` (item 68): a
    // machine-readable roster dump for `scripts/capture-worlds.sh` and any
    // other script that wants the world list without parsing --help. Both
    // are fate (c) — genuine CLI/diagnostic stdout, not app-runtime chatter.
    ("main/args.rs", 2),
    // `--screenshot`/`--screenshot-motion*`/`--screenshot-frames`/`--capture-*`'s
    // "wrote …" deliverable output — this IS the CLI's product, read by
    // scripts/agents — plus the permissive `--keys` replay's ONE stderr warning
    // seam (the strict-replay round: `replay::warn_line` fires when a replay crosses
    // an Unsupported/Intercepted effect; CLI diagnostic output by design, and the
    // same string is recorded in the replay result so tests pin it). (The 8th is the
    // virtual-clock frame-loop capture's own "wrote N frame(s)…" deliverable line.)
    ("main/run.rs", 8),
    // `--storyboard`'s deliverable output (the run summary + "wrote film…"),
    // plus the BEST-EFFORT film-encode notes ("no ffmpeg on PATH", a nonzero
    // ffmpeg exit, a non-UTF-8 output path) — CLI product + diagnostics by
    // design; the raw frames are always retained, so each note is advisory.
    ("main/story.rs", 5),
    // `--print-menu-roster`'s hidden-flag CLI output (`scripts/smoke-menus.sh`).
    ("menu.rs", 1),
    // `AWL_FONT` + `AWL_CHROME_FACE_FILE` dev-only env var override
    // diagnostics (the second is the Firetail-showcase round's audition-font
    // loader: a missing/unreadable candidate file prints a note and is
    // skipped — the same advisory class as `AWL_FONT`'s fallback note). The
    // third is `read_forced_knob`'s LOUD-fallback note: a SET-but-unrecognized
    // `AWL_*_FORCE` value (a retired skin like the killed `chips`, or a typo)
    // names itself + the grammar before falling back to the world default, so a
    // stale gallery re-shoot can't silently duplicate the default.
    ("render.rs", 3),
    // `--bench-frame` / `--bench-theme-burst` / `--bench-zoom-burst` /
    // `--bench-frost`'s tabular CLI output.
    ("render/framebench.rs", 39),
    ("render/perfbench.rs", 8),
    // `--bench-caret`'s tabular CLI output (item 57): the header + column ruler,
    // the per-position rows, and the machine-readable verdict line — the same
    // CLI-harness class as the bench entries above.
    ("render/caretbench.rs", 6),
    // `--bench-suite`'s tabular CLI output + the baseline diff report — the
    // same CLI-harness class as the four bench entries above.
    ("render/benchsuite/mod.rs", 12),
    ("render/benchsuite/report.rs", 9),
    // `--soak-gpu`'s bounded native-probe report is CLI product: result,
    // counters (incl. the per-cause `skipped_by_kind` breakdown), memory
    // summaries, recovery timings, and explicit defects. All print sites live
    // in the report submodule; `soak_gpu/mod.rs` (the schedule/observe half)
    // prints nothing, so it does not appear here.
    ("soak_gpu/report.rs", 8),
];

/// The pure per-line needle counter: matches `println!(` / `eprintln!(` as a
/// whole macro-call token — trying `eprintln!(` FIRST at each position, so
/// its trailing `println!(` suffix is consumed as part of THAT one match
/// rather than counted a second, phantom time (the naive "just count both
/// substrings separately" trap: `"eprintln!(".contains("println!(")`).
/// Advances by one whole `char` on a non-match, so a non-ASCII line (the
/// `"こんにちは"` test fixture in `app.rs`) never panics on a bad byte offset.
fn needle_count(line: &str) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < line.len() {
        let rest = &line[i..];
        if rest.starts_with("eprintln!(") {
            n += 1;
            i += "eprintln!(".len();
        } else if rest.starts_with("println!(") {
            n += 1;
            i += "println!(".len();
        } else {
            i += rest.chars().next().map(char::len_utf8).unwrap_or(1);
        }
    }
    n
}

/// Scan `path`'s text, skipping every `#[cfg(test)]`/`#[cfg(all(test…`-gated
/// item via a brace-balanced skip (approximate — good enough for this crate's
/// consistent style, mirroring `durable.rs::scan_dir_for_bare_writes`'s own
/// "not a real parser, just a disciplined heuristic" scope), and sum the
/// needle count over every line OUTSIDE a skipped region.
fn scan_file(text: &str) -> usize {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Normal,
        /// Saw a `#[cfg(test)...]` attribute line; waiting to see whether the
        /// following item opens a brace block (skip until balanced) or is a
        /// bare `mod tests;` declaration (skip just that one line).
        AfterCfgTest,
        /// Inside a skipped brace-delimited item, at net depth `.0` (always > 0).
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
                    State::AfterCfgTest // a stacked attribute; keep waiting
                } else if line.contains('{') {
                    let d = line.matches('{').count() as i32 - line.matches('}').count() as i32;
                    if d <= 0 {
                        State::Normal
                    } else {
                        State::InSkippedBlock(d)
                    }
                } else if line.trim_end().ends_with(';') {
                    State::Normal // a bare `mod tests;` declaration
                } else {
                    State::AfterCfgTest // a multi-line signature; keep waiting
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

fn scan_dir(
    base: &std::path::Path,
    dir: &std::path::Path,
    counts: &mut std::collections::BTreeMap<String, usize>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // A `tests/` directory is entirely test fixtures/harness code —
            // its own `eprintln!("skipping …: no wgpu adapter")` guards are
            // never runtime-reachable.
            if path.file_name().and_then(|n| n.to_str()) == Some("tests") {
                continue;
            }
            scan_dir(base, &path, counts);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        // A file literally named `tests.rs` (an inline test submodule split
        // out of its parent, e.g. `src/buffer/tests.rs`) is test-only too.
        if path.file_name().and_then(|n| n.to_str()) == Some("tests.rs") {
            continue;
        }
        // This module's OWN doc comments/table quote `println!(`/`eprintln!(`
        // by name (the audit table, the fate descriptions) — self-matches
        // that aren't real call sites. Skip it, mirroring `durable.rs`'s own
        // "the scanner walks this very file too" self-match note (there
        // solved by assembling needles from fragments instead; skipping is
        // simpler here since this file has no real print-macro calls of its
        // own to guard).
        if path.file_name().and_then(|n| n.to_str()) == Some("println_audit.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let n = scan_file(&text);
        if n == 0 {
            continue;
        }
        let rel = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        counts.insert(rel, n);
    }
}

#[test]
fn no_stray_println_outside_the_audited_table() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    scan_dir(&root, &root, &mut counts);

    let expected: std::collections::BTreeMap<String, usize> =
        EXPECTED.iter().map(|(f, n)| (f.to_string(), *n)).collect();

    assert_eq!(
        counts, expected,
        "a println!/eprintln! call appeared somewhere unaccounted for (a new file, or a \
         changed count in an already-audited one) — give it a fate: route it through the \
         `App::notice` seam (a), silence it (b), or add it to `println_audit::EXPECTED` \
         with a reason (c). See this module's doc comment for the full audit."
    );
}

#[test]
fn needle_count_never_double_counts_eprintln_as_two_hits() {
    assert_eq!(needle_count(r#"eprintln!("x: {e}");"#), 1);
    assert_eq!(needle_count(r#"println!("x");"#), 1);
    assert_eq!(needle_count("no macro here at all"), 0);
    assert_eq!(
        needle_count(r#"println!("a"); eprintln!("b");"#),
        2,
        "one of each on the same line counts as two, not three"
    );
}
