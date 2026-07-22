// ── HERMETICITY STRUCTURAL GUARD ────────────────────────────────────────
//
// Rust's privacy model can express "visible to production plus every
// descendant module" (what a private `fn new` already gets — every test
// submodule under `app/` is a descendant of `app`, so it already sees the
// raw constructor) but NOT "visible to production plus this ONE helper
// function's own body" — there is no `pub(in path)` spelling that grants
// access to `new_hermetic`'s definition while denying every sibling test
// module. So the raw constructor's door can't be sealed at compile time
// without also blocking the small set of tests that deliberately need
// the REAL disk (see `App::new_hermetic`'s own doc for that list). This
// is the honest fallback: a SOURCE-SCAN law test, in the same spirit as
// `rowlayout.rs`'s / `theme/`'s no-wildcard enumerations — a structural
// fact asserted at test time, cheap to keep honest because the count it
// guards is small and curated, not a general-purpose linter.
//
// NOTE ON THE NEEDLE: the pattern this scan looks for is built at RUNTIME
// (`app_new_needle`, four separate literals concatenated) rather than
// spelled out as one contiguous string anywhere in this file — otherwise
// this very guard's own source text would match itself and inflate its
// own count. Keep every comment/message below phrased without writing
// the raw constructor's name directly followed by an open paren.
//
// Exact per-file occurrence counts of the needle across the whole crate.
// Every entry below is individually accounted for (see each call site's
// own inline comment): either the ONE real production call, a real-disk
// test that explicitly disables `session_restore` (can't use
// `new_hermetic` because it needs `Buffer::from_file` to see genuine
// bytes), or a test already wrapped in `fs::with_fs`/`FsGuard::install`
// with a controlled fake `InMemoryFs` (hermetic by construction,
// independent of `session_restore`'s value — `app/session.rs`'s own
// tests, which specifically exercise session restore, cannot use
// `new_hermetic` at all since it forces `session_restore: Some(false)`).
// A test that only needs a plain, don't-care-about-disk `App` must go
// through `App::new_hermetic` instead, which never contributes to this
// count at all (its name has an extra `_hermetic` between `new` and the
// open paren, so it never matches the needle).
//
// Adding a NEW raw call anywhere — including a new file — fails this
// test until the count below is consciously updated, which forces the
// same two-way choice every existing site already made.
#[test]
fn real_fs_app_new_calls_are_all_accounted_for() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    scan_dir_for_app_new(&root, &root, &mut counts);

    let expected: &[(&str, usize)] = &[
        // 1 production call (in `crate::app::run`), the ONLY raw `App::new`
        // left in `app.rs` proper now the test module lives in its own file.
        ("app.rs", 1),
        // The 4 test-side raw calls retain their former accounting after the
        // feature split: 2 real-disk lifecycle tests with `session_restore`
        // disabled inline, 1 real-disk chdir/buffer test (same treatment),
        // and the `app_on` helper (every caller installs its own fake FS).
        ("app/tests/lifecycle.rs", 2),
        ("app/tests/buffers.rs", 1),
        ("app/tests/common.rs", 1),
        // 1 real-disk test (`finish_buffer_saves_...`), session_restore
        // disabled inline.
        ("app/daemon.rs", 1),
        // 5 calls, every one inside a `crate::fs::with_fs(fake, || ..)`
        // closure seeded with its own `InMemoryFs` — these tests exist
        // specifically to prove what `apply_session_restore` reads back,
        // so they can't use a constructor that forces it off.
        ("app/session.rs", 5),
        // 3 store tests (2 recent-projects + 1 recent-files), each inside its
        // own `fs::with_fs(fake, ..)` closure seeded with an `InMemoryFs` — they
        // exist specifically to prove what `App::switch_project` / `App::load_path`
        // / `App::new` write to and read back from the recent-projects /
        // recent-files stores, so they need to CONTROL + INSPECT the injected fs
        // (which `new_hermetic`'s private internal fs hides), never real disk.
        // Same treatment as `app/session.rs` above. Plus 3 NO-PATH-PASTE-SAVES-
        // FIRST tests (`ensure_note_named_before_paste_*`), each also inside its
        // own `fs::with_fs(fake, ..)` closure with an `InMemoryFs` handle kept by
        // the test — they exist specifically to prove what
        // `App::ensure_note_named_before_paste` writes to disk (the promoted
        // note's derived path + its saved bytes), so they need the same
        // CONTROL + INSPECT access `new_hermetic` hides. Same treatment. Plus 1
        // CJK-priority persist test
        // (`persist_cjk_priority_writes_the_whole_ordered_ladder_to_config`),
        // inside its own `fs::with_fs(fake, ..)` closure with an `InMemoryFs`
        // handle — proves what `App::persist_cjk_priority` writes to
        // `config.path` on disk, same CONTROL + INSPECT need. Plus 2
        // SPELLCHECK x CONFIG-RELOAD tests (the spell-toggle-x-theme
        // investigation, 2026-07-18:
        // `reload_config_absent_spellcheck_key_leaves_global_untouched` +
        // `reload_config_reapplies_a_persisted_spellcheck_value_immediately`),
        // each inside its own `fs::with_fs(fake, ..)` closure with an
        // `InMemoryFs` handle — they exist specifically to prove what
        // `App::reload_config` reads back from `config.path` on disk (and,
        // for the absent-key case, that it must NOT force a default), same
        // CONTROL + INSPECT need `new_hermetic` hides. Plus 1 NOTES FLIP round
        // test (`notes_flip_round_trips_the_live_app_project_root`), inside its
        // own `fs::with_fs(fake, ..)` closure with an `InMemoryFs` handle — it
        // exists specifically to prove `App::notes_flip` never writes the
        // sticky `project_root` config key nor the recent-projects store, so it
        // needs the same CONTROL + INSPECT access `new_hermetic` hides. Plus 2
        // ADD-TO-DICTIONARY tests (item 39:
        // `add_to_dictionary_persists_the_word_and_silences_it_live` +
        // `startup_loads_the_personal_dictionary_so_an_added_word_never_squiggles_across_a_restart`),
        // each inside its own `fs::with_fs(fake, ..)` closure with an `InMemoryFs`
        // handle — they prove what `App::add_to_dictionary` writes to (and
        // `App::new` → `load_user_dictionary` reads back from) `dictionary.txt`
        // beside `config.toml` on disk, the same CONTROL + INSPECT need
        // `new_hermetic` hides.
        ("app/files.rs", 12),
        // 9 LIFETIME STATS + USAGE LEDGER + DISCOVERABILITY tests, each inside its own
        // `fs::with_fs(fake, ..)` closure seeded with an `InMemoryFs` — they exist
        // specifically to prove what the tracking hooks / the ledger's
        // `ledger_note_dispatch` + `stats_flush` write to and read back from
        // `stats.toml`, so they need to CONTROL + INSPECT the injected fs (which
        // `new_hermetic`'s private internal fs hides). Same treatment as
        // `app/session.rs` / `app/files.rs` above. (The 3 added by the ledger:
        // door-attribution round-trip, graduation-candidate ranking, kill-switch;
        // the 2 added by the discoverability round: peek/footer ranking from a fake
        // ledger, and the fresh-ledger-empty case.)
        ("app/stats.rs", 9),
        // 6 WRITING STREAKS tests, each inside its own `fs::with_fs(fake, ..)`
        // closure seeded with an `InMemoryFs` — they exist specifically to prove
        // what `streaks_flush` writes to / reads back from `streaks.toml` (and
        // that the kill switch never writes), so they need to CONTROL + INSPECT
        // the injected fs (which `new_hermetic`'s private fs hides). Same
        // treatment as `app/stats.rs` above. `new_hermetic` also won't do here:
        // it restores the real backend on construction return, but these tests
        // keep driving the fs AFTER construction (`new_note`, the summon flush),
        // so the fake must stay active across the whole closure. (The 3 added by
        // the anchor-swallow fix: fresh-note + fresh-scratch record words typed
        // before the first flush, and the card-summon-freshness flush.)
        ("app/streaks.rs", 6),
        // input.rs's click tests all moved onto `App::new_hermetic` —
        // zero raw calls left.
    ];
    let mut expected_map: std::collections::BTreeMap<String, usize> =
        expected.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    // Any file not listed above must have ZERO occurrences.
    for (file, count) in &counts {
        let want = expected_map.remove(file).unwrap_or(0);
        assert_eq!(
            *count, want,
            "unexpected raw-constructor count in {file}: found {count}, expected {want} — \
             either route the new call through App::new_hermetic, or (if it genuinely needs \
             real disk) disable session_restore inline / wrap it in fs::with_fs and update \
             this test's expected count with a comment explaining why"
        );
    }
    for (file, want) in expected_map {
        assert_eq!(
            0, want,
            "expected {want} raw-constructor call(s) in {file} but found none — did it move to new_hermetic or a different file?"
        );
    }

    // The ONE production call site must still exist exactly once, naming
    // its real argument list (guards against the count staying right by
    // coincidence while the actual production call moved or was deleted).
    let mut production_hits = 0usize;
    count_substr_in_dir(&root, &production_call_needle(), &mut production_hits);
    assert_eq!(
        production_hits, 1,
        "the production App::new call in crate::app::run must exist exactly once"
    );
}

/// Built from separate literals at runtime — see the module-doc note
/// above the guard test for why this can't be one contiguous literal.
#[cfg(test)]
fn app_new_needle() -> String {
    ["App", "::", "new", "("].concat()
}

#[cfg(test)]
fn production_call_needle() -> String {
    format!(
        "{}file, root, cli_workspace, cli_notes_root, config);",
        app_new_needle()
    )
}

#[cfg(test)]
fn scan_dir_for_app_new(
    base: &std::path::Path,
    dir: &std::path::Path,
    counts: &mut std::collections::BTreeMap<String, usize>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let needle = app_new_needle();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir_for_app_new(base, &path, counts);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let n = text.matches(&needle).count();
        if n == 0 {
            continue;
        }
        let rel = path
            .strip_prefix(base)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        counts.insert(rel, n);
    }
}

#[cfg(test)]
fn count_substr_in_dir(dir: &std::path::Path, needle: &str, total: &mut usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            count_substr_in_dir(&path, needle, total);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        *total += text.matches(needle).count();
    }
}
