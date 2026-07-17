//! src/durable.rs — DATA-SAFETY HARDENING: the PRESERVE-ON-CORRUPT recovery
//! contract shared by every durable, app-owned store (session/stats/recents/
//! the mas grant store/the history log/the scratch stash) — everything that
//! isn't the user's own hand-edited `config.toml` (which is deliberately
//! EXEMPT; see this module's doc below).
//!
//! **The problem this closes:** every store's `load()` already degrades
//! LENIENTLY on a malformed or missing file (`Config::load`'s idiom, copied
//! everywhere — `session::load`, `stats::load`, `recents::load`,
//! `mas::load`) — correct for AVAILABILITY (a garbled file must never block
//! a launch), but WRONG for DATA SAFETY when the file was PRESENT and
//! CORRUPTED rather than simply absent: the lenient default silently
//! discards whatever text survived, and the store's next flush overwrites
//! the corrupt file with that (smaller/emptier) default — permanently.
//!
//! **The fix, ONE shared seam:** [`preserve_corrupt`] copies the corrupt
//! ORIGINAL bytes to a sibling file (`<name>.corrupt-<utc-millis>`,
//! zero-padded so lexical sort is chronological sort) BEFORE the lenient
//! default proceeds, then prunes down to the newest [`CORRUPT_BACKUP_KEEP`]
//! siblings (pure prune logic in [`corrupt_siblings_to_prune`], mirroring
//! `crashlog::names_to_prune`'s shape). Best-effort throughout (an I/O
//! failure while backing up must never block the lenient load it is
//! protecting) and routed through the SAME [`crate::fs::active`] backend as
//! everything else, so it works identically on native and web.
//!
//! [`load_toml_store`] is the ONE shared loader every TOML-backed store
//! (`session`, `stats`, `recents`, `mas::GrantStore`) now calls: it detects
//! the difference between "file absent" (never preserved — nothing was
//! lost) and "file present but its TOML SYNTAX itself failed to parse, or
//! it isn't even valid UTF-8" (a real corruption signal — preserved). A
//! file whose TOML parses fine but carries a missing/wrong-typed FIELD is
//! NOT corruption by this definition — that is the lenient partial-default
//! path every `from_toml` already handles on purpose (an old store missing
//! a newly-added field, say), and preserving a backup on every such benign
//! case would spam `.corrupt-*` siblings for no reason.
//!
//! **Config is EXEMPT from the sibling-copy rule (documented, not an
//! oversight):** `config.toml` is the user's own hand-edited file — a parse
//! failure there is almost always a typo the user just made, not disk
//! corruption, and `Config::load` already has its own correct-for-that-case
//! behavior (keep the prior in-memory values + show a notice, never
//! silently reset to defaults — see `config/model.rs`). Backing up a
//! `.corrupt-*` sibling on every fat-fingered edit would litter the config
//! dir for a case that isn't data loss at all (the user's editor buffer +
//! undo history still has their intended text). So `config::write` keeps
//! routing every write through [`crate::fs::write_atomic`] (the PART 1
//! durability fix) but never calls [`preserve_corrupt`].
//!
//! **The history log is a SEPARATE format (not TOML)** and gets its own
//! corruption check colocated with its own parser
//! (`history::store::read_log`) — see that module for the "does this log
//! look trustworthy" logic — but calls the SAME [`preserve_corrupt`] here.
//! Likewise the scratch stash, which isn't structured data at all (plain
//! markdown text): its one possible "failed to parse" is the file existing
//! but not being valid UTF-8, checked at its own read site in `app.rs`.

use std::path::Path;

/// How many `.corrupt-*` siblings a single store keeps — a generous but
/// bounded window (mirrors `crashlog::MAX_CRASH_LOGS`'s "look back across a
/// bad week, never an unbounded pile" reasoning, just narrower: a corrupt
/// store is a much rarer event than a crash).
pub const CORRUPT_BACKUP_KEEP: usize = 5;

/// The corrupt-backup sibling's file name for a store whose own file name is
/// `name`, stamped at `now_ms` (millis since the Unix epoch) plus `seq` (a
/// per-process MONOTONIC disambiguator — see [`next_seq`]). Both are
/// zero-padded to a fixed width so a plain lexical sort of file names IS a
/// chronological sort (millis since epoch comfortably fits in 20 digits for
/// millennia; `seq` in 10) — the same "sortable by construction" trick
/// `crashlog::utc_timestamp` uses with zero-padded date fields. `seq` alone
/// (not `now_ms` alone) is what actually GUARANTEES uniqueness: two corrupt
/// loads landing in the same wall-clock millisecond — entirely realistic
/// under a tight burst, e.g. this module's own prune test — would otherwise
/// collide on the SAME file name and silently overwrite one backup with the
/// next, defeating the whole "keep the newest N" contract before it even
/// starts.
pub fn corrupt_backup_name(name: &str, now_ms: u128, seq: u64) -> String {
    format!("{name}.corrupt-{now_ms:020}-{seq:010}")
}

/// The next value in a process-wide MONOTONIC counter, used purely to
/// disambiguate [`corrupt_backup_name`] when two backups land in the same
/// millisecond (see that function's doc). Never reset, never wraps in any
/// realistic process lifetime (`u64`).
fn next_seq() -> u64 {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// PURE: given the file names present in a store's directory, return the
/// `.corrupt-*` siblings of `stem` to DELETE so at most `keep` newest
/// survive. Sorts lexically (chronological by construction — see
/// [`corrupt_backup_name`]) and never touches any file that isn't a
/// `<stem>.corrupt-*` sibling, so it can't accidentally prune the live store
/// file or another store's siblings sharing the same directory.
pub fn corrupt_siblings_to_prune(names: &[String], stem: &str, keep: usize) -> Vec<String> {
    let prefix = format!("{stem}.corrupt-");
    let mut matching: Vec<&String> = names.iter().filter(|n| n.starts_with(&prefix)).collect();
    matching.sort();
    if matching.len() <= keep {
        return Vec::new();
    }
    matching[..matching.len() - keep].iter().map(|s| (*s).clone()).collect()
}

/// PRESERVE a corrupt store's raw bytes: write them to a timestamped sibling
/// beside `path`, then prune down to [`CORRUPT_BACKUP_KEEP`] newest. Called
/// ONLY when a load found the file PRESENT but unparseable/undecodable —
/// NEVER when the file is simply absent (every call site here is gated on
/// that distinction; see [`load_toml_store`] and `history::store::read_log`).
///
/// Best-effort throughout: a failure to write the backup or to list/prune
/// the directory is swallowed (the lenient load this protects must proceed
/// regardless — losing the ABILITY to recover a corrupt file is far better
/// than losing the EDITOR over a filesystem hiccup while trying to save one).
pub fn preserve_corrupt(path: &Path, raw: &[u8]) {
    let fs = crate::fs::active();
    let Some(parent) = path.parent() else { return };
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return;
    };
    let now_ms = crate::clock::system_now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let backup_path = parent.join(corrupt_backup_name(&name, now_ms, next_seq()));
    let _ = fs.write(&backup_path, raw);
    if let Ok(entries) = fs.read_dir(parent) {
        let existing: Vec<String> =
            entries.into_iter().filter(|e| e.is_file).map(|e| e.name).collect();
        for stale in corrupt_siblings_to_prune(&existing, &name, CORRUPT_BACKUP_KEEP) {
            let _ = fs.remove_file(&parent.join(&stale));
        }
    }
}

/// The SHARED loader for every TOML-backed store (`session`, `stats`,
/// `recents`, `mas::GrantStore`): reads `path` through the active
/// `FileSystem`, and — ONLY when the file is PRESENT — checks whether it
/// preserves before handing it to `parse` (each store's own `from_toml`,
/// which stays exactly as lenient about individual FIELDS as before this
/// round: a valid-but-incomplete table is not corruption).
///
/// Three outcomes:
///   - file absent (`NotFound`) → `T::default()`, nothing preserved (there
///     was nothing to lose).
///   - file present, valid UTF-8, but its TOML SYNTAX fails to parse → the
///     raw text is preserved, then `parse("")`-equivalent proceeds (in
///     practice every `from_toml` returns `T::default()` on unparseable
///     input, so this is `T::default()` too — but routed through the same
///     `parse` closure so a future looser recovery strategy stays a
///     one-function change).
///   - file present but not valid UTF-8 at all (`read_to_string` errors on
///     something other than `NotFound`) → the RAW BYTES are preserved
///     (best-effort re-read via `fs.read`), and `T::default()`.
pub fn load_toml_store<T: Default>(path: &Path, parse: impl FnOnce(&str) -> T) -> T {
    let fs = crate::fs::active();
    match fs.read_to_string(path) {
        Ok(src) => {
            if src.parse::<toml::Table>().is_err() {
                preserve_corrupt(path, src.as_bytes());
            }
            parse(&src)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => T::default(),
        Err(_) => {
            // Present but not valid UTF-8 (or some other read failure short
            // of NotFound): try a raw byte read to preserve what we can.
            if let Ok(raw) = fs.read(path) {
                preserve_corrupt(path, &raw);
            }
            T::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::FileSystem;
    use std::path::PathBuf;
    use std::sync::Arc;

    #[test]
    fn corrupt_backup_name_is_stem_dot_corrupt_dash_padded_millis_dash_padded_seq() {
        assert_eq!(
            corrupt_backup_name("session.toml", 42, 7),
            "session.toml.corrupt-00000000000000000042-0000000007"
        );
    }

    #[test]
    fn corrupt_backup_name_disambiguates_the_same_millisecond_via_seq() {
        // Two backups landing in the exact same wall-clock millisecond (a
        // real scenario under a tight burst) must still get DISTINCT names —
        // `seq` alone is the uniqueness guarantee, not `now_ms`.
        let a = corrupt_backup_name("x.toml", 1000, 0);
        let b = corrupt_backup_name("x.toml", 1000, 1);
        assert_ne!(a, b);
    }

    #[test]
    fn corrupt_siblings_to_prune_keeps_newest_and_ignores_unrelated_names() {
        let names: Vec<String> = vec![
            "session.toml".to_string(),
            "session.toml.corrupt-00000000000000000001".to_string(),
            "session.toml.corrupt-00000000000000000002".to_string(),
            "session.toml.corrupt-00000000000000000003".to_string(),
            "stats.toml.corrupt-00000000000000000099".to_string(), // a DIFFERENT store's sibling
        ];
        let pruned = corrupt_siblings_to_prune(&names, "session.toml", 2);
        assert_eq!(pruned, vec!["session.toml.corrupt-00000000000000000001".to_string()]);
        // Never touches the live file or another store's sibling.
        assert!(!pruned.iter().any(|n| n == "session.toml"));
        assert!(!pruned.iter().any(|n| n.starts_with("stats.toml")));
    }

    #[test]
    fn corrupt_siblings_to_prune_is_a_no_op_under_the_keep_count() {
        let names: Vec<String> = vec!["a.toml.corrupt-1".to_string(), "a.toml.corrupt-2".to_string()];
        assert!(corrupt_siblings_to_prune(&names, "a.toml", 5).is_empty());
        assert!(corrupt_siblings_to_prune(&[], "a.toml", 5).is_empty());
    }

    #[test]
    fn preserve_corrupt_writes_a_sibling_and_prunes_down_to_the_keep_count() {
        let fake = Arc::new(crate::fs::InMemoryFs::new().with_dir("/data"));
        crate::fs::with_fs(fake.clone(), || {
            let path = PathBuf::from("/data/session.toml");
            for i in 0..(CORRUPT_BACKUP_KEEP + 3) {
                preserve_corrupt(&path, format!("garbage {i}").as_bytes());
            }
            let names: Vec<String> = fake
                .read_dir(Path::new("/data"))
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect();
            let siblings: Vec<&String> =
                names.iter().filter(|n| n.starts_with("session.toml.corrupt-")).collect();
            assert_eq!(
                siblings.len(),
                CORRUPT_BACKUP_KEEP,
                "pruned down to the keep count: {names:?}"
            );
        });
    }

    #[test]
    fn preserve_corrupt_never_touches_the_live_store_file() {
        let fake = Arc::new(crate::fs::InMemoryFs::new().with_file("/data/session.toml", "active = 1\n"));
        crate::fs::with_fs(fake.clone(), || {
            preserve_corrupt(&PathBuf::from("/data/session.toml"), b"garbage");
            assert_eq!(
                fake.read_to_string(Path::new("/data/session.toml")).unwrap(),
                "active = 1\n",
                "the live file is untouched — only a NEW sibling is written"
            );
        });
    }

    // --- load_toml_store: the three outcomes ------------------------------

    #[derive(Debug, Default, PartialEq)]
    struct Toy {
        n: i64,
    }
    fn parse_toy(src: &str) -> Toy {
        let n = src.parse::<toml::Table>().ok().and_then(|t| t.get("n").and_then(|v| v.as_integer())).unwrap_or(0);
        Toy { n }
    }

    #[test]
    fn load_toml_store_missing_file_is_default_and_preserves_nothing() {
        let fake = Arc::new(crate::fs::InMemoryFs::new());
        crate::fs::with_fs(fake.clone(), || {
            let path = PathBuf::from("/data/toy.toml");
            assert_eq!(load_toml_store(&path, parse_toy), Toy::default());
            // Nothing preserved: the directory doesn't even exist.
            assert!(fake.read_dir(Path::new("/data")).is_err());
        });
    }

    #[test]
    fn load_toml_store_valid_toml_missing_field_is_lenient_default_no_backup() {
        // Legitimate "old store, new field" case — NOT corruption.
        let fake = Arc::new(crate::fs::InMemoryFs::new().with_file("/data/toy.toml", "other = 3\n"));
        crate::fs::with_fs(fake.clone(), || {
            let path = PathBuf::from("/data/toy.toml");
            assert_eq!(load_toml_store(&path, parse_toy), Toy { n: 0 });
            let names: Vec<String> =
                fake.read_dir(Path::new("/data")).unwrap().into_iter().map(|e| e.name).collect();
            assert!(
                !names.iter().any(|n| n.contains(".corrupt-")),
                "a valid-but-incomplete TOML table must never back up: {names:?}"
            );
        });
    }

    #[test]
    fn load_toml_store_garbled_toml_syntax_preserves_a_sibling_then_defaults() {
        let fake =
            Arc::new(crate::fs::InMemoryFs::new().with_file("/data/toy.toml", "not valid toml {{{"));
        crate::fs::with_fs(fake.clone(), || {
            let path = PathBuf::from("/data/toy.toml");
            assert_eq!(load_toml_store(&path, parse_toy), Toy::default());
            let names: Vec<String> =
                fake.read_dir(Path::new("/data")).unwrap().into_iter().map(|e| e.name).collect();
            let siblings: Vec<&String> =
                names.iter().filter(|n| n.starts_with("toy.toml.corrupt-")).collect();
            assert_eq!(siblings.len(), 1, "the garbled original is preserved: {names:?}");
            let backup = fake.read_to_string(Path::new("/data").join(siblings[0]).as_path()).unwrap();
            assert_eq!(backup, "not valid toml {{{", "the sibling holds the ORIGINAL bytes verbatim");
        });
    }

    #[test]
    fn load_toml_store_next_flush_does_not_destroy_the_preserved_sibling() {
        // The exact bug this round exists to close: a corrupt load followed by
        // a normal save must not wipe out the backup it just made.
        let fake =
            Arc::new(crate::fs::InMemoryFs::new().with_file("/data/toy.toml", "not valid toml {{{"));
        crate::fs::with_fs(fake.clone(), || {
            let path = PathBuf::from("/data/toy.toml");
            let toy = load_toml_store(&path, parse_toy);
            assert_eq!(toy, Toy::default());
            // A normal "save the (now-default) state back" — mirrors every
            // store's own `save()`, which always goes through `write_atomic`
            // on the STORE's own path, never touching a `.corrupt-*` sibling.
            crate::fs::write_atomic(&path, b"n = 0\n").unwrap();
            let names: Vec<String> =
                fake.read_dir(Path::new("/data")).unwrap().into_iter().map(|e| e.name).collect();
            assert!(
                names.iter().any(|n| n.starts_with("toy.toml.corrupt-")),
                "the sibling backup survives the next flush: {names:?}"
            );
            assert_eq!(fake.read_to_string(&path).unwrap(), "n = 0\n");
        });
    }

    // --- THE ATOMIC-WRITE AUDIT LAW: no bare, non-atomic durable write ----
    //
    // A "bare write" here means any of THREE call shapes that bypass
    // `crate::fs::write_atomic`'s temp-sibling-then-rename dance: a plain
    // `write` call chained straight off `active()`, a local `fs` handle's
    // own bare `write` call, or the raw `std::fs` free-function `write`,
    // unmediated by the `FileSystem` trait. (Spelled out as ASSEMBLED
    // fragments in [`bare_write_needles`] rather than as one literal string
    // constant, quite deliberately: this very file's OWN doc prose above
    // and the law test's failure message below both need to name these
    // shapes in plain English for a human reader, and a literal needle
    // sitting in durable.rs's source text would make the scanner catch
    // ITSELF — the same self-reference dodge `app.rs`'s own
    // `real_fs_app_new_calls_are_all_accounted_for` uses for its needle.)
    // Every occurrence of these three needles across the whole crate is
    // counted per file below (source-scan pattern, mirroring `app.rs`'s own
    // law test). Adding a NEW bare write anywhere — including a new file —
    // changes some file's count and fails this test until the table is
    // consciously updated, which forces the same "route it through
    // write_atomic, or justify why not" choice every existing site already
    // made:
    //
    //   src/durable.rs (1)   — `preserve_corrupt`'s OWN backup writer: writing
    //     a brand-new, uniquely-timestamped sibling file that never existed
    //     before, so there is no pre-existing content a tear could destroy —
    //     the one narrow case where "always a new file" makes bare-write
    //     safe by construction (documented in `preserve_corrupt`'s own doc).
    //   src/crashlog.rs (1)  — the mid-panic `write_log` writer, DELIBERATELY
    //     primitive per this round's own instructions ("crashlog's mid-panic
    //     writer stays deliberately primitive") — a panicking thread must
    //     not risk taking a lock or doing a fancier multi-step write.
    //   src/fs.rs (4)        — the FileSystem trait's OWN primitive `write`
    //     implementations (`NativeFs`, `WebFs`) PLUS `write_atomic`'s own
    //     internal write of the tmp sibling, PLUS one `#[cfg(test)]` seed
    //     helper (`seed_write_if_absent`, write-IF-ABSENT — never overwrites
    //     existing content, so a tear can only ever produce a still-absent
    //     or still-fresh file, never corrupt a returning visitor's data).
    //     These four ARE the primitive `write_atomic` and every store above
    //     it are built out of — they cannot recursively route through
    //     themselves.
    //   src/app.rs, src/app/daemon.rs, src/buffers.rs, src/daemon.rs,
    //   src/history/tests.rs, src/index.rs, src/main/run.rs (26 combined)
    //     — every one of these is INSIDE a `#[cfg(test)]` module, seeding a
    //     real temp-dir fixture file directly (never a durable app store) or
    //     (in `history/tests.rs`) deliberately planting garbage to exercise
    //     THIS round's own corrupt-recovery test.
    #[test]
    fn no_bare_durable_write_bypasses_write_atomic_outside_the_accounted_for_sites() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        scan_dir_for_bare_writes(&root, &root, &mut counts);

        let expected: &[(&str, usize)] = &[
            ("app.rs", 4),
            ("app/daemon.rs", 3),
            ("buffers.rs", 1),
            ("crashlog.rs", 1),
            ("daemon.rs", 1),
            ("durable.rs", 1),
            // The export golden-file BLESS helpers (`export::tests::golden` and
            // `export::pdf::tests`, gated on `AWL_BLESS`): write committed test
            // fixtures under `src/export/testdata/`, never a durable user store
            // — a torn write just means re-blessing, and either golden is
            // regenerated on demand.
            ("export/pdf/tests.rs", 1),
            ("export/tests.rs", 1),
            ("fs.rs", 4),
            ("history/tests.rs", 1),
            ("index.rs", 4),
            // Two of these are the fresh-oracle Goto regression's own fixture
            // seeds (`goto_switch_mid_replay_reshapes_the_oracle_to_the_
            // arriving_buffer`) — temp-dir test files, never a durable store;
            // two more are the hermetic-scenario tests' real-disk inputs
            // (seeded precisely to prove the sandbox never writes them back).
            ("main/run.rs", 17),
            // The storyboard runner's `trace.json` write (`write_trace`): a
            // HARNESS DELIVERABLE, not app state — a storyboard run's active
            // backend IS the hermetic sandbox, so routing this through
            // `write_atomic`/`fs::active()` would swallow the artifact the
            // caller asked for (same reason the capture PNG + film frames
            // write with `std::fs`/`image` directly). Overwritten whole per
            // run; a torn write costs one re-run of a deterministic scenario,
            // never user data.
            ("main/story.rs", 1),
            // ONE production site (`build_sandbox`: seeding the hermetic
            // in-memory sandbox INSTANCE before it becomes the active backend
            // — `write_atomic` routes through `fs::active()`, which at seed
            // time is still the real disk, so the direct instance write is
            // the only correct call; nothing durable, nothing on disk) + four
            // test seeds/asserts in its own `#[cfg(test)]` module.
            ("scenario.rs", 5),
        ];
        let expected_map: std::collections::BTreeMap<String, usize> =
            expected.iter().map(|(f, n)| (f.to_string(), *n)).collect();
        assert_eq!(
            counts, expected_map,
            "a bare (non-write_atomic) durable write appeared somewhere unaccounted for — \
             route it through crate::fs::write_atomic, or add it to this table with a \
             comment justifying why not (mirrors app.rs's own real_fs_app_new_calls_are_all_accounted_for)"
        );
    }

    /// The three bare-write call shapes this law test hunts for, ASSEMBLED
    /// from fragments (never written as one literal string in this file) so
    /// the scanner — which walks this very file too — can't match its own
    /// needle definitions. See the law test's doc comment above for why.
    #[cfg(test)]
    fn bare_write_needles() -> [String; 3] {
        [
            ["active()", ".", "write", "("].concat(),
            ["fs", ".", "write", "("].concat(),
            ["std", "::", "fs", "::", "write", "("].concat(),
        ]
    }

    #[cfg(test)]
    fn scan_dir_for_bare_writes(
        base: &std::path::Path,
        dir: &std::path::Path,
        counts: &mut std::collections::BTreeMap<String, usize>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let needles = bare_write_needles();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan_dir_for_bare_writes(base, &path, counts);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let n: usize = needles.iter().map(|needle| text.matches(needle.as_str()).count()).sum();
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
}
