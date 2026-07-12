//! src/history/tests.rs — the history unit-test suite, moved verbatim out
//! of the former `history.rs` monolith (2026-07 code-organization pass);
//! every test's NAME and MODULE PATH are unchanged (`history::tests::foo`)
//! — only which file its source lives in moved. Native-only (git-shell
//! tests), mirroring the module's own `#[cfg]` gate.

use super::picker::{append_clock_when_shared, civil_date, history_bucket, same_utc_day, EXCERPT_CHARS};
use super::prune::{DAY_MS, MAX_TOTAL};
use super::store::{
    log_path, parse_git_log_line, parse_log, parse_log_checked, read_log, serialize_log, write_log,
};
use super::*;
use crate::config::Config;
use crate::facets::FacetItem;
use crate::fs::InMemoryFs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A config with history ON (the loose-file default).
fn cfg_on() -> Config {
    Config::empty()
}

// ── The history timeline's FACETING scheme (All · Session · Today) ─────────
const DAY: u64 = 86_400_000;

#[test]
fn history_facets_land_on_all_home_then_group_by_time() {
    // "All" is the FIRST lens (strip index 0), the flat timeline home.
    assert_eq!(HISTORY_FACETS.strip[0].id, "all");
    assert!(HISTORY_FACETS.strip[0].sections.is_empty());
    let ids: Vec<&str> = HISTORY_FACETS.strip.iter().map(|f| f.id).collect();
    assert_eq!(ids, vec!["all", "session", "today"]);
}

#[test]
fn same_utc_day_compares_day_indices() {
    let base = 100 * DAY + 5_000; // 5s into day 100
    assert!(same_utc_day(base, 100 * DAY + DAY - 1)); // same day, later
    assert!(!same_utc_day(base, 101 * DAY)); // next day
    assert!(!same_utc_day(base, 99 * DAY + DAY - 1)); // previous day
}

#[test]
fn history_bucket_sessions_and_today_by_injected_clock() {
    // now = 3s into day 100; the session started at day 100's midnight.
    let now = 100 * DAY + 3_000;
    let session_start = 100 * DAY;
    let item = |ts: u64| FacetItem {
        accept: "",
        is_dir: false,
        is_git: false,
        recent: false,
        heading: false,
        ts: Some(ts),
        now: Some(now),
        session_start: Some(session_start),
    };
    // Session lens (strip index 1): a stamp AT/AFTER session_start is in-session.
    assert_eq!(history_bucket(item(session_start + 10), 1), Some("Session"));
    assert_eq!(history_bucket(item(session_start - 10), 1), None); // before launch
    // Today lens (index 2): a stamp on `now`'s calendar day is Today.
    assert_eq!(history_bucket(item(100 * DAY + 1), 2), Some("Today"));
    assert_eq!(history_bucket(item(99 * DAY + 1), 2), None); // yesterday
    // A stamp earlier TODAY but before this session's start is Today, not Session.
    // (Here session_start == midnight, so use a session that began mid-day.)
    let mid = 100 * DAY + 2_000;
    let it2 = FacetItem { session_start: Some(mid), ..item(100 * DAY + 1_000) };
    assert_eq!(history_bucket(it2, 1), None, "before this session's start");
    assert_eq!(history_bucket(it2, 2), Some("Today"), "still the same day");
}

#[test]
fn history_bucket_is_inert_without_a_clock_the_determinism_gate() {
    // The headless capture path passes now / session_start = None → both time
    // lenses group NOTHING (degrading gracefully, never a clock read in the bucket).
    let headless = FacetItem {
        accept: "",
        is_dir: false,
        is_git: false,
        recent: false,
        heading: false,
        ts: Some(100 * DAY),
        now: None,
        session_start: None,
    };
    assert_eq!(history_bucket(headless, 1), None, "Session inert with no clock");
    assert_eq!(history_bucket(headless, 2), None, "Today inert with no clock");
}

#[test]
fn timeline_row_carries_the_snapshot_timestamp() {
    let snaps = vec![Snapshot { id: "42".into(), timestamp: 42, subject: None, pinned: false }];
    let rows = rows_from(&snaps, &["hi\n".to_string()], "hi\n", 1_000);
    assert_eq!(rows[0].timestamp, 42, "the row carries its stamp for bucketing");
}

#[test]
fn save_writes_a_snapshot_then_list_and_load_round_trip() {
    // A loose file (no `.git` ancestor) gets an awl snapshot on record; list()
    // sees it and load() reconstructs the exact content. All via the FS seam.
    let p = PathBuf::from("/notes/draft.md");
    crate::fs::with_fs(Arc::new(InMemoryFs::new()), || {
        record(&p, "first body\nwith two lines", &cfg_on());
        let snaps = list(&p);
        assert_eq!(snaps.len(), 1, "one snapshot recorded");
        assert_eq!(
            load(&p, &snaps[0].id).as_deref(),
            Some("first body\nwith two lines"),
            "content round-trips byte-for-byte"
        );
    });
}

#[test]
fn list_is_newest_first() {
    // Two DISTINCT saves stack newest-first; the ids resolve to their content.
    let p = PathBuf::from("/notes/a.md");
    crate::fs::with_fs(Arc::new(InMemoryFs::new()), || {
        record(&p, "one", &cfg_on());
        record(&p, "two", &cfg_on());
        let snaps = list(&p);
        assert_eq!(snaps.len(), 2, "both distinct saves kept");
        assert!(snaps[0].timestamp >= snaps[1].timestamp, "newest first");
        assert_eq!(load(&p, &snaps[0].id).as_deref(), Some("two"));
        assert_eq!(load(&p, &snaps[1].id).as_deref(), Some("one"));
    });
}

#[test]
fn unchanged_content_is_not_re_snapshotted() {
    // DEDUP: re-saving identical content adds no new snapshot (autosave-on-pause
    // must not spam the log).
    let p = PathBuf::from("/notes/same.md");
    crate::fs::with_fs(Arc::new(InMemoryFs::new()), || {
        record(&p, "steady", &cfg_on());
        record(&p, "steady", &cfg_on());
        record(&p, "steady", &cfg_on());
        assert_eq!(list(&p).len(), 1, "identical re-saves dedup to one");
    });
}

// ── The AGED RETENTION LADDER (pure, injected now_ms — no wall clock) ───

/// Build a newest-first (un-pinned) entry list from (age-ms, label) pairs
/// against `now`.
fn entries_aged(now: u64, ages: &[u64]) -> Vec<Entry> {
    let mut v: Vec<Entry> = ages
        .iter()
        .map(|a| Entry { ts: now - a, content: format!("v@{a}"), pinned: false })
        .collect();
    v.sort_by(|a, b| b.ts.cmp(&a.ts)); // newest first
    v
}

#[test]
fn ladder_keeps_everything_under_15_min() {
    // The undo-adjacent recent past stays at FULL resolution: a burst of
    // saves inside the fresh window all survive.
    let now = 1_700_000_000_000u64;
    let mut e = entries_aged(now, &[0, 30_000, 60_000, 5 * 60_000, 14 * 60_000]);
    prune_ladder(&mut e, now);
    assert_eq!(e.len(), 5, "everything <= 15 min old is kept");
}

#[test]
fn ladder_one_survivor_per_session_15min_to_24h() {
    // Three work sessions this afternoon (internal gaps < 15 min, > 15 min
    // of quiet between them): exactly the LAST snapshot of each survives.
    // (The lone 8h save doubles as the file's oldest — the memory rule and
    // the cluster rule agree on it.)
    let now = 1_700_000_000_000u64;
    let hr = 3_600_000u64;
    // Session A: 2h .. 2h20m ago (10-min steps). Session B: 5h .. 5h10m ago.
    // Session C: a lone save 8h ago.
    let mut e = entries_aged(
        now,
        &[2 * hr, 2 * hr + 10 * 60_000, 2 * hr + 20 * 60_000,
          5 * hr, 5 * hr + 10 * 60_000, 8 * hr],
    );
    prune_ladder(&mut e, now);
    let kept: Vec<u64> = e.iter().map(|e| now - e.ts).collect();
    assert_eq!(
        kept,
        vec![2 * hr, 5 * hr, 8 * hr],
        "each session's NEWEST snapshot survives: {kept:?}"
    );
}

#[test]
fn ladder_one_per_day_to_30_days_newest_survives() {
    // Three saves on one day last week + a lone save on an older day: one
    // survivor per day, each its day's newest (the lone oldest save is also
    // what the memory rule protects).
    let now = 1_700_000_000_000u64;
    let day = DAY_MS;
    let mut e = entries_aged(
        now,
        &[5 * day, 5 * day + 3_600_000, 5 * day + 7_200_000, 9 * day],
    );
    prune_ladder(&mut e, now);
    assert_eq!(e.len(), 2, "one per day in the daily band");
    let kept: Vec<u64> = e.iter().map(|e| now - e.ts).collect();
    assert_eq!(kept, vec![5 * day, 9 * day], "the newest of each day: {kept:?}");
}

#[test]
fn ladder_one_per_week_beyond_30_days() {
    // Daily saves across two old weeks (~40 days back): one survivor per
    // week bucket (plus the always-kept oldest), so resolution coarsens but
    // the span stays covered.
    let now = 1_700_000_000_000u64;
    let day = DAY_MS;
    let ages: Vec<u64> = (35..49).map(|d| d * day).collect(); // 14 daily points
    let mut e = entries_aged(now, &ages);
    prune_ladder(&mut e, now);
    assert!(
        e.len() >= 2 && e.len() <= 4,
        "14 old daily saves collapse to ~one per week: {}",
        e.len()
    );
    assert!(
        e.iter().any(|e| now - e.ts == 48 * day),
        "the oldest snapshot survives (memory, not resolution)"
    );
}

#[test]
fn ladder_cap_150_escalates_never_fifo() {
    // >150 snapshots spread over years: the cap holds by CLIMBING the ladder
    // (wider buckets), and the OLDEST snapshot — the file's origin — always
    // survives. Memory kept; resolution pruned.
    let now = 1_700_000_000_000u64;
    let day = DAY_MS;
    // 3 years of every-third-day saves (365 entries) + a fresh burst.
    let mut ages: Vec<u64> = (0..365).map(|i| 2 * day + i * 3 * day).collect();
    ages.extend([0u64, 60_000, 120_000]);
    let oldest_age = *ages.iter().max().unwrap();
    let mut e = entries_aged(now, &ages);
    prune_ladder(&mut e, now);
    assert!(e.len() <= MAX_TOTAL, "cap holds: {}", e.len());
    assert!(
        e.iter().any(|e| now - e.ts == oldest_age),
        "the OLDEST snapshot survives the cap (never FIFO)"
    );
    assert!(
        e.iter().any(|e| now - e.ts == 0),
        "the newest snapshot survives too"
    );
}

#[test]
fn ladder_is_deterministic_and_idempotent() {
    // Same input + same now → identical output; pruning the pruned output
    // again changes nothing (survivors are their own groups' newests).
    let now = 1_700_000_000_000u64;
    let day = DAY_MS;
    let ages: Vec<u64> = vec![
        0, 5 * 60_000, 2 * 3_600_000, 2 * 3_600_000 + 5 * 60_000,
        3 * day, 3 * day + 3_600_000, 40 * day, 41 * day, 200 * day,
    ];
    let mut a = entries_aged(now, &ages);
    let mut b = entries_aged(now, &ages);
    prune_ladder(&mut a, now);
    prune_ladder(&mut b, now);
    assert_eq!(a, b, "deterministic for a fixed now");
    let once = a.clone();
    prune_ladder(&mut a, now);
    assert_eq!(a, once, "prune of pruned is a no-op");
}

// ── THE CONSCIOUS MARK: pinned snapshots are prune-EXEMPT ───────────────

#[test]
fn ladder_exempts_a_pinned_snapshot_its_unpinned_peer_would_lose() {
    // Three saves in ONE old daily bucket (all same day, ~5 days back):
    // newest-first A(5d−2h) · B(5d−1h) · C(5d). The daily band keeps ONE per
    // day — its NEWEST (A) — while the memory rule protects the file's oldest
    // (C); the MIDDLE B is exactly what gets pruned. PINNING B exempts it, so
    // all three survive: a pinned snapshot survives a prune its un-pinned peer
    // loses.
    let now = 20_000 * DAY_MS; // a clean day boundary so the ages share a bucket
    let day = DAY_MS;
    let hr = 3_600_000u64;
    let base = || {
        vec![
            Entry { ts: now - (5 * day - 2 * hr), content: "A-newest".into(), pinned: false },
            Entry { ts: now - (5 * day - hr), content: "B-middle".into(), pinned: false },
            Entry { ts: now - 5 * day, content: "C-oldest".into(), pinned: false },
        ]
    };
    // Un-pinned: the daily band keeps A (bucket newest) + C (protected oldest);
    // the middle B is pruned.
    let mut unpinned = base();
    prune_ladder(&mut unpinned, now);
    let kept: Vec<&str> = unpinned.iter().map(|e| e.content.as_str()).collect();
    assert_eq!(kept, vec!["A-newest", "C-oldest"], "un-pinned middle is pruned: {kept:?}");
    // Pin the middle: the conscious mark makes B prune-EXEMPT, so all survive.
    let mut pinned = base();
    pinned[1].pinned = true;
    prune_ladder(&mut pinned, now);
    let kept: Vec<&str> = pinned.iter().map(|e| e.content.as_str()).collect();
    assert_eq!(
        kept,
        vec!["A-newest", "B-middle", "C-oldest"],
        "the pinned middle survives the same prune: {kept:?}"
    );
}

#[test]
fn ladder_pins_do_not_count_against_the_150_cap() {
    // 200 PINNED snapshots spread over years: EVERY one survives — pins are
    // exempt and don't count against MAX_TOTAL (a pin means "keep this,
    // always"). The un-pinned equivalent is capped (see the FIFO test), so this
    // is the exemption, not the ladder failing to fire.
    let now = 1_700_000_000_000u64;
    let day = DAY_MS;
    let mut pinned: Vec<Entry> = (0..200u64)
        .map(|i| Entry {
            ts: now - (2 * day + i * 3 * day),
            content: format!("v{i}"),
            pinned: true,
        })
        .collect();
    prune_ladder(&mut pinned, now);
    assert_eq!(pinned.len(), 200, "every pinned snapshot survives the cap: {}", pinned.len());
}

#[test]
fn record_pinned_marks_the_snapshot_as_the_conscious_mark() {
    // "Keep version" records a PINNED snapshot the store reports as pinned.
    let p = PathBuf::from("/notes/kept.md");
    crate::fs::with_fs(Arc::new(InMemoryFs::new()), || {
        record_pinned(&p, "keep me", &cfg_on());
        let snaps = list(&p);
        assert_eq!(snaps.len(), 1, "one snapshot recorded");
        assert!(snaps[0].pinned, "record_pinned lands the conscious mark");
    });
}

#[test]
fn record_pinned_upgrades_the_already_newest_version_in_place() {
    // A pin right after a save (identical content) must still land: rather than
    // dedup-skipping, it PINS the existing newest entry in place — no duplicate.
    let p = PathBuf::from("/notes/pin-after-save.md");
    crate::fs::with_fs(Arc::new(InMemoryFs::new()), || {
        record(&p, "steady", &cfg_on()); // an ordinary save
        assert!(!list(&p)[0].pinned, "the plain save is un-pinned");
        record_pinned(&p, "steady", &cfg_on()); // pin the same content
        let snaps = list(&p);
        assert_eq!(snaps.len(), 1, "no duplicate — the newest is upgraded in place");
        assert!(snaps[0].pinned, "the existing newest is now pinned");
    });
}

#[test]
fn pinned_flag_round_trips_across_a_store_write_and_reload() {
    // The pin persists: record_pinned → list reports pinned; a byte-level
    // reload (via the awlhist2 log format) preserves it.
    let p = PathBuf::from("/notes/persist.md");
    crate::fs::with_fs(Arc::new(InMemoryFs::new()), || {
        record(&p, "v1", &cfg_on());
        record_pinned(&p, "v2", &cfg_on());
        let snaps = list(&p);
        // Newest (v2) pinned, older (v1) not.
        assert!(snaps[0].pinned && !snaps[1].pinned, "pin state persists per entry");
    });
}

#[test]
fn an_awlhist1_log_loads_with_every_entry_unpinned() {
    // BACK-COMPAT: a pre-pin store (awlhist1, two-token headers) still loads,
    // every entry reading un-pinned — upgrading never strands an old timeline.
    let bytes = b"awlhist1\n1000 5\nhello\n";
    assert_eq!(
        parse_log(bytes),
        vec![Entry { ts: 1000, content: "hello".into(), pinned: false }],
        "an old two-token header reads pinned = false"
    );
}

#[test]
fn record_at_prunes_with_injected_clock() {
    // The record path runs the ladder with the caller's now_ms: a same-session
    // burst pushed past the fresh window collapses to the session survivor +
    // the fresh tail — all deterministic, no wall clock.
    let p = PathBuf::from("/notes/ladder.md");
    crate::fs::with_fs(Arc::new(InMemoryFs::new()), || {
        let t0 = 1_700_000_000_000u64;
        // A 40-minute session of one save per minute, clock injected.
        for i in 0..40u64 {
            record_at(&p, &format!("v{i}"), &cfg_on(), t0 + i * 60_000, false);
        }
        let snaps = list(&p);
        // Everything older than the last 15 min belongs to the SAME session
        // (1-min gaps), whose newest lives inside the fresh window — so the
        // fresh window's worth survives (15 min + the boundary save).
        assert!(snaps.len() < 40, "the ladder pruned the stale burst");
        assert_eq!(
            load(&p, &snaps[0].id).as_deref(),
            Some("v39"),
            "the newest save always survives"
        );
    });
}

#[test]
fn git_managed_file_is_detected_and_skips_awl_snapshots() {
    // A file inside a repo (a seeded `.git` ancestor) is git-managed: record
    // writes NO awl snapshot, and the gate reports it managed.
    let p = PathBuf::from("/repo/src/main.rs");
    let fs = InMemoryFs::new().with_dir("/repo/.git");
    crate::fs::with_fs(Arc::new(fs), || {
        assert!(is_git_managed(&p), "`.git` ancestor detected");
        assert_eq!(
            git_repo_root(&p).as_deref(),
            Some(Path::new("/repo")),
            "repo root is the `.git` parent"
        );
        record(&p, "fn main() {}", &cfg_on());
        // No awl log was written for a git-managed file (git owns its history).
        assert!(
            crate::fs::active().read(&log_path(&p)).is_err(),
            "no awl snapshot log for a git-managed file"
        );
    });
}

#[test]
fn record_inside_git_repo_writes_nothing_ever() {
    // The NEW contract (supersedes the old record_periodic one): a
    // git-managed file gets NO awl snapshot from ANY record path — its
    // timeline is git log alone. Autosave still WRITES such files (writing
    // is not version-meddling); only the snapshot store stays out.
    let p = PathBuf::from("/repo/src/notes.md");
    let fs = InMemoryFs::new().with_dir("/repo/.git");
    crate::fs::with_fs(Arc::new(fs), || {
        assert!(is_git_managed(&p), "the seeded `.git` ancestor is detected");
        record(&p, "v1", &cfg_on());
        record_at(&p, "v2", &cfg_on(), 1_700_000_000_000, false);
        assert!(
            crate::fs::active().read(&log_path(&p)).is_err(),
            "no awl snapshot log for a git-managed file, from any path"
        );
        // history=false still gates loose files elsewhere — and certainly
        // adds nothing here either.
        let off = Config {
            history: Some(false),
            ..Config::empty()
        };
        record(&p, "v3", &off);
        assert!(crate::fs::active().read(&log_path(&p)).is_err());
    });
}

#[test]
fn parse_git_log_line_extracts_hash_secs_subject() {
    // The pure `%H %ct %s` line parser: hash + seconds→millis + the subject
    // (spaces preserved); a missing subject reads None; junk lines skip.
    let s = parse_git_log_line("abc123 1700000000 fix: the thing, twice")
        .expect("well-formed line parses");
    assert_eq!(s.id, "abc123");
    assert_eq!(s.timestamp, 1_700_000_000_000);
    assert_eq!(s.subject.as_deref(), Some("fix: the thing, twice"));
    // No subject (an empty %s) → None, not Some("").
    let bare = parse_git_log_line("def456 1700000001").expect("subject-less parses");
    assert_eq!(bare.subject, None);
    let trailing = parse_git_log_line("def456 1700000001 ").expect("trailing space");
    assert_eq!(trailing.subject, None);
    // Malformed lines are skipped quietly.
    assert!(parse_git_log_line("").is_none());
    assert!(parse_git_log_line("onlyhash").is_none());
    assert!(parse_git_log_line("hash notasecond subject").is_none());
}

#[test]
fn a_loose_file_beside_a_repo_is_not_git_managed() {
    // A file OUTSIDE any repo (no `.git` ancestor) is loose → awl snapshots it.
    let p = PathBuf::from("/notes/loose.md");
    let fs = InMemoryFs::new().with_dir("/repo/.git");
    crate::fs::with_fs(Arc::new(fs), || {
        assert!(!is_git_managed(&p), "no `.git` ancestor → loose");
        record(&p, "loose note", &cfg_on());
        assert_eq!(list(&p).len(), 1, "loose file gets an awl snapshot");
    });
}

#[test]
fn history_off_disables_snapshots() {
    // The sticky `history = false` setting turns the store off for loose files.
    let p = PathBuf::from("/notes/off.md");
    let cfg = Config {
        history: Some(false),
        ..Config::empty()
    };
    crate::fs::with_fs(Arc::new(InMemoryFs::new()), || {
        record(&p, "nothing kept", &cfg);
        assert!(list(&p).is_empty(), "history off → no snapshots");
    });
}

#[test]
fn log_round_trips_content_with_newlines_and_spaces() {
    // The framed format survives embedded newlines + the header-delimiter space,
    // and the per-entry pinned flag round-trips (one kept, one not).
    let entries = vec![
        Entry { ts: 1_000, content: "line one\nline two\n".to_string(), pinned: true },
        Entry { ts: 999, content: "a b  c\nd".to_string(), pinned: false },
    ];
    let bytes = serialize_log(&entries);
    assert_eq!(parse_log(&bytes), entries, "serialize→parse is lossless");
}

#[test]
fn fnv_is_stable_and_path_keyed() {
    // Distinct paths hash to distinct logs; the same path is stable across calls.
    assert_eq!(log_path(Path::new("/a")), log_path(Path::new("/a")));
    assert_ne!(log_path(Path::new("/a")), log_path(Path::new("/b")));
}

#[test]
fn relative_label_reads_the_time_gap_humanly() {
    // The label bands: just now / min / hr / yesterday / days / a date.
    let now = 10_000 * 24 * 60 * 60 * 1000; // some large epoch-millis anchor
    let min = 60 * 1000;
    let hr = 60 * min;
    let day = 24 * hr;
    assert_eq!(relative_label(now, now), "just now");
    assert_eq!(relative_label(now, now - 30 * 1000), "just now");
    assert_eq!(relative_label(now, now - 2 * min), "2 min ago");
    assert_eq!(relative_label(now, now - 3 * hr), "3 hr ago");
    assert_eq!(relative_label(now, now - 1 * day - hr), "yesterday");
    assert_eq!(relative_label(now, now - 4 * day), "4 days ago");
    // Older than a week -> a YYYY-MM-DD date (spot-check the epoch itself).
    assert_eq!(relative_label(now, now - 30 * day), civil_date((now - 30 * day) / 1000));
    assert_eq!(civil_date(0), "1970-01-01");
    // A future stamp (clock skew) is clamped to "just now".
    assert_eq!(relative_label(now, now + day), "just now");
}

#[test]
fn line_diff_counts_are_add_minus_remove_vs_current() {
    // Identical text -> no change.
    assert_eq!(line_diff_counts("a\nb\nc", "a\nb\nc"), (0, 0));
    // Restoring a version that DROPS a line: +0 −1 vs current.
    assert_eq!(line_diff_counts("a\nb\nc", "a\nc"), (0, 1));
    // Restoring a version that ADDS a line: +1 −0.
    assert_eq!(line_diff_counts("a\nc", "a\nb\nc"), (1, 0));
    // A changed line is one removed + one added (LCS keeps the shared lines).
    assert_eq!(line_diff_counts("a\nb\nc", "a\nB\nc"), (1, 1));
    // Empty current -> restoring adds every line.
    assert_eq!(line_diff_counts("", "x\ny"), (2, 0));
    // The large-input fallback agrees on a simple add.
    let big_old: String = (0..3000).map(|i| format!("l{i}\n")).collect();
    let big_new: String = (0..3001).map(|i| format!("l{i}\n")).collect();
    let (add, rem) = line_diff_counts(&big_old, &big_new);
    assert_eq!((add, rem), (1, 0), "fallback multiset diff counts the one added line");
}

#[test]
fn timeline_rows_are_newest_first_with_labels_and_counts() {
    // Two saves -> two rows, newest first; each row's diff is vs the CURRENT text.
    let p = PathBuf::from("/notes/timeline.md");
    crate::fs::with_fs(Arc::new(InMemoryFs::new()), || {
        record(&p, "one\ntwo", &cfg_on());
        record(&p, "one\ntwo\nthree", &cfg_on());
        let now = now_millis();
        let rows = timeline_rows(&p, "one\ntwo\nthree", now);
        assert_eq!(rows.len(), 2, "both versions listed");
        // Row 0 is the newest (matches current) -> +0 −0; its id round-trips.
        assert_eq!(rows[0].counts, "+0 −0");
        assert_eq!(load(&p, &rows[0].id).as_deref(), Some("one\ntwo\nthree"));
        // Row 1 is the older 2-line version -> restoring it removes "three": +0 −1.
        assert_eq!(rows[1].counts, "+0 −1");
        assert_eq!(load(&p, &rows[1].id).as_deref(), Some("one\ntwo"));
        // WHEN labels are non-empty relative-time strings.
        assert!(!rows[0].when.is_empty() && !rows[1].when.is_empty());
        // WHICH: no headings anywhere -> the newest row's edit describes its
        // first changed line vs the older version (a bare excerpt).
        assert_eq!(rows[0].which, "three");
    });
}

#[test]
fn timeline_rows_empty_for_a_file_with_no_history() {
    let p = PathBuf::from("/notes/fresh.md");
    crate::fs::with_fs(Arc::new(InMemoryFs::new()), || {
        assert!(timeline_rows(&p, "scratch", now_millis()).is_empty());
    });
}

// ── The WHEN + WHICH row model (pure — no store, no clock) ──────────────

#[test]
fn clock_hm_is_utc_zero_padded() {
    // Epoch midnight, a morning, an evening — all UTC, always two digits.
    assert_eq!(clock_hm(0), "00:00");
    assert_eq!(clock_hm(9 * 3_600_000 + 5 * 60_000), "09:05");
    assert_eq!(clock_hm(23 * 3_600_000 + 59 * 60_000 + 59_000), "23:59");
    // A whole number of days later reads the same wall time.
    assert_eq!(clock_hm(3 * 86_400_000 + 14 * 3_600_000 + 32 * 60_000), "14:32");
}

#[test]
fn when_labels_gain_clock_time_only_when_siblings_share() {
    // Two snapshots whose relative labels collide ("2 hr ago") both gain a
    // " HH:MM" suffix; a distinct third label stays calm and bare.
    let now = 1_700_000_000_000u64; // 2023-11-14 22:13:20 UTC
    let hr = 3_600_000u64;
    let mut labels = vec![
        relative_label(now, now - 2 * hr),
        relative_label(now, now - 2 * hr - 20 * 60_000),
        relative_label(now, now - 5 * hr),
    ];
    assert_eq!(labels[0], labels[1], "the fixture really collides");
    assert_ne!(labels[0], labels[2]);
    let ts = vec![now - 2 * hr, now - 2 * hr - 20 * 60_000, now - 5 * hr];
    append_clock_when_shared(&mut labels, &ts);
    assert_eq!(labels[0], format!("2 hr ago {}", clock_hm(ts[0])));
    assert_eq!(labels[1], format!("2 hr ago {}", clock_hm(ts[1])));
    assert_eq!(labels[2], "5 hr ago", "a lone label stays bare");
}

#[test]
fn first_changed_line_covers_edit_append_delete_identical() {
    // An EDIT: the first differing paired line.
    assert_eq!(first_changed_line("a\nb\nc", "a\nB\nc"), 1);
    // An APPEND: the identical prefix ends where old runs out — the first
    // new line is the change.
    assert_eq!(first_changed_line("a\nb", "a\nb\nc"), 2);
    // A DELETE at the end: clamped into new's range — its last line.
    assert_eq!(first_changed_line("a\nb\nc", "a\nb"), 1);
    // IDENTICAL texts: clamped to new's last line (any anchor is honest).
    assert_eq!(first_changed_line("a\nb", "a\nb"), 1);
    // Empty new reads 0 (nothing to point into).
    assert_eq!(first_changed_line("a\nb", ""), 0);
    assert_eq!(first_changed_line("", "x"), 0);
}

#[test]
fn auto_description_names_nearest_heading_above_change() {
    // Two sections; the change lands under the SECOND heading, so the
    // description names it — the raw title, quoted, "edited"-prefixed.
    let prev = "# One flow\n\nbody a\n\n## Two flows, one engine\n\nbody b\n";
    let cur = "# One flow\n\nbody a\n\n## Two flows, one engine\n\nbody b CHANGED\n";
    assert_eq!(auto_description(prev, cur), "edited \"Two flows, one engine\"");
    // A change under the FIRST heading names that one instead.
    let cur2 = "# One flow\n\nbody a CHANGED\n\n## Two flows, one engine\n\nbody b\n";
    assert_eq!(auto_description(prev, cur2), "edited \"One flow\"");
}

#[test]
fn auto_description_falls_back_to_first_changed_line_excerpt() {
    // No heading above the change: a short excerpt of the changed line,
    // returned bare (no "edited" wrapper).
    assert_eq!(auto_description("plain\nlines", "plain\nlines but changed"), "lines but changed");
    // A long line truncates at the char cap with a trailing ellipsis.
    let long = "x".repeat(60);
    let desc = auto_description("", &long);
    assert_eq!(desc.chars().count(), EXCERPT_CHARS + 1, "cap + the ellipsis");
    assert!(desc.ends_with('…'));
    // An empty document yields an empty which (the row shows WHEN alone).
    assert_eq!(auto_description("was here", ""), "");
}

#[test]
fn rows_from_composes_when_which_counts_ids() {
    // Pure composition over injected snapshots + contents: a git row's WHICH
    // is its commit SUBJECT; an awl row's is the auto-description vs the
    // next-older content; the OLDEST diffs against "" (its edit is the whole
    // document); counts are vs the CURRENT buffer; ids pass through.
    let now = 1_700_000_000_000u64;
    let day = 86_400_000u64;
    let snaps = vec![
        Snapshot {
            id: "abc123".into(),
            timestamp: now - 60_000,
            subject: Some("fix: the engine".into()),
            pinned: false,
        },
        Snapshot { id: "1000".into(), timestamp: now - 2 * day, subject: None, pinned: false },
        Snapshot { id: "999".into(), timestamp: now - 40 * day, subject: None, pinned: false },
    ];
    let contents = vec![
        "a\nb\nc".to_string(),
        "a\nb".to_string(),
        "a".to_string(),
    ];
    let rows = rows_from(&snaps, &contents, "a\nb\nc", now);
    assert_eq!(rows.len(), 3);
    // The git subject WINS the which column.
    assert_eq!(rows[0].which, "fix: the engine");
    assert_eq!(rows[0].id, "abc123");
    assert_eq!(rows[0].counts, "+0 −0", "newest matches current");
    // The awl row describes its edit vs the NEXT-OLDER content.
    assert_eq!(rows[1].which, "b", "line 1 appeared vs the older \"a\"");
    assert_eq!(rows[1].counts, "+0 −1");
    // The oldest diffs against "": its whole content is the edit.
    assert_eq!(rows[2].which, "a");
    assert_eq!(rows[2].counts, "+0 −2");
    // All three labels are distinct here -> none gained a clock suffix.
    assert_eq!(rows[0].when, "1 min ago");
    assert_eq!(rows[1].when, "2 days ago");
    assert!(!rows[2].when.contains(':'), "date labels stay bare");
}

#[test]
fn clamp_line_col_bounds_cursor_into_preview() {
    // Inside the text: untouched.
    assert_eq!(clamp_line_col("ab\ncd", 1, 1), (1, 1));
    // End-of-line is a valid caret seat; past it clamps to the line's chars.
    assert_eq!(clamp_line_col("ab\ncd", 0, 99), (0, 2));
    // A line past the previewed doc clamps to its last line (renderer's
    // \n-split model: a trailing newline yields a final empty row).
    assert_eq!(clamp_line_col("ab\ncd", 9, 4), (1, 2));
    assert_eq!(clamp_line_col("ab\n", 9, 9), (1, 0));
    // The empty document pins to origin.
    assert_eq!(clamp_line_col("", 3, 7), (0, 0));
}

// ── Adversarial ladder probes (the verification round) ──────────────────

#[test]
fn ladder_two_bursts_with_quiet_gap_keep_each_bursts_newest_plus_origin() {
    // Two same-day BURSTS past the fresh window, separated by >15 min of
    // quiet: the EXACT survivor set is each burst's newest save, plus the
    // file's origin (the always-kept oldest) — nothing else, in order.
    let now = 1_700_000_000_000u64;
    let m = 60_000u64;
    // Burst A: 40..70 min ago in 5-min steps (gaps < 15 min → one session).
    // Burst B: 3h..3h20m ago in 10-min steps.
    let ages: Vec<u64> = vec![
        40 * m, 45 * m, 50 * m, 55 * m, 60 * m, 65 * m, 70 * m,
        180 * m, 190 * m, 200 * m,
    ];
    let mut e = entries_aged(now, &ages);
    prune_ladder(&mut e, now);
    let kept: Vec<u64> = e.iter().map(|e| now - e.ts).collect();
    assert_eq!(
        kept,
        vec![40 * m, 180 * m, 200 * m],
        "each burst's newest + the protected origin: {kept:?}"
    );
}

#[test]
fn ladder_session_gap_of_exactly_15_min_splits_sessions() {
    // The cluster rule is STRICTLY-less-than: consecutive gaps of exactly
    // 15 min are quiet enough to end a session, so all three saves are
    // their own sessions' newest and all survive.
    let now = 1_700_000_000_000u64;
    let m = 60_000u64;
    let mut e = entries_aged(now, &[16 * m, 31 * m, 46 * m]);
    prune_ladder(&mut e, now);
    assert_eq!(e.len(), 3, "a 15-min gap ends a session (strict <)");
}

#[test]
fn ladder_clock_rewind_overshoot_self_heals_once_time_advances() {
    // ADVERSARIAL CLOCK: stamps AHEAD of `now` (a wall-clock rewind mid-
    // burst) read as age 0 → fresh at EVERY level, so a >150 burst
    // transiently exceeds the cap — the ladder refuses to FIFO memory away
    // to force it. Characterized, not fixed: the overshoot SELF-HEALS —
    // the same pure prune with a later `now` re-enforces the cap.
    let now = 1_700_000_000_000u64;
    let mut e: Vec<Entry> = (0..200u64)
        // newest-first, all "future"
        .map(|i| Entry { ts: now + 200 - i, content: format!("v{i}"), pinned: false })
        .collect();
    prune_ladder(&mut e, now);
    assert_eq!(
        e.len(),
        200,
        "future stamps read fresh: a transient overshoot, never FIFO"
    );
    // Time catches up (half a day): the burst is one session (1-ms gaps),
    // so it collapses to its newest + the protected oldest — cap holds.
    let later = now + 200 + DAY_MS / 2;
    prune_ladder(&mut e, later);
    assert!(e.len() <= MAX_TOTAL, "cap re-enforced: {}", e.len());
    assert_eq!(
        e.len(),
        2,
        "one session survivor + the protected origin: {}",
        e.len()
    );
}

#[test]
fn source_path_prefers_buffer_then_file_then_scratch_stash() {
    use std::path::Path;
    let b = Path::new("/notes/buffer.md");
    let f = Path::new("/notes/file.md");
    // The buffer's own path wins; the App-level file backs it up.
    assert_eq!(source_path(Some(b), Some(f), false).as_deref(), Some(b));
    assert_eq!(source_path(None, Some(f), false).as_deref(), Some(f));
    // The TRUE SCRATCH (no path, not a note) keys under its stash — so the
    // persistent scratch has a summonable timeline.
    assert_eq!(
        source_path(None, None, false),
        Some(crate::fs::scratch_stash_path())
    );
    // An unnamed NOTE has no history key yet (its first autosave names it).
    assert_eq!(source_path(None, None, true), None);
}

// --- PRESERVE-ON-CORRUPT: parse_log_checked's trust flag + read_log's backup ---

#[test]
fn parse_log_checked_trusts_a_clean_log_including_an_empty_body() {
    let entries = vec![Entry { ts: 1, content: "x".into(), pinned: false }];
    let (parsed, trusted) = parse_log_checked(&serialize_log(&entries));
    assert!(trusted, "a well-formed log is trusted");
    assert_eq!(parsed, entries);
    // An empty-but-intact store (just the magic line) is ALSO trusted — an
    // empty log is not corruption.
    let (parsed, trusted) = parse_log_checked(&serialize_log(&[]));
    assert!(trusted, "an empty (magic-only) log is trusted, not corrupt");
    assert!(parsed.is_empty());
}

#[test]
fn parse_log_checked_distrusts_a_garbled_magic_line() {
    let (parsed, trusted) = parse_log_checked(b"not a log at all");
    assert!(!trusted);
    assert!(parsed.is_empty());
}

#[test]
fn parse_log_checked_distrusts_a_truncated_entry_but_still_returns_prior_survivors() {
    // Three entries serialized (written newest-first, so THIRD comes first in
    // the byte stream and FIRST comes last), then the trailing bytes chopped
    // off — a real "kill-9 mid-write" shape (had write_log not been atomic).
    // The two COMPLETE, EARLIER-IN-THE-STREAM entries still parse cleanly;
    // only the LAST (truncated) one is lost, and the overall log is
    // correctly flagged untrusted so it gets backed up rather than silently
    // rewritten short on the next save.
    let full = vec![
        Entry { ts: 3, content: "third".into(), pinned: false },
        Entry { ts: 2, content: "second".into(), pinned: false },
        Entry { ts: 1, content: "first".into(), pinned: false },
    ];
    let bytes = serialize_log(&full);
    let torn = &bytes[..bytes.len() - 3]; // chop off the tail of the LAST entry's content
    let (parsed, trusted) = parse_log_checked(torn);
    assert!(!trusted, "a truncated entry must never read as trustworthy");
    assert_eq!(
        parsed,
        vec![full[0].clone(), full[1].clone()],
        "the two entries fully written before the tear still parse"
    );
}

#[test]
fn read_log_preserves_a_corrupt_sibling_before_returning_the_lenient_partial_parse() {
    let p = PathBuf::from("/notes/history-victim.md");
    crate::fs::with_fs(Arc::new(InMemoryFs::new()), || {
        let lp = log_path(&p);
        // Plant garbage directly (never through write_log) — simulates disk
        // corruption / an old bug that bypassed the atomic writer.
        crate::fs::active().write(&lp, b"garbled beyond recognition").unwrap();
        let entries = read_log(&p);
        assert!(entries.is_empty(), "an unrecoverable log degrades to empty");

        let dir = lp.parent().unwrap();
        let names: Vec<String> =
            crate::fs::active().read_dir(dir).unwrap().into_iter().map(|e| e.name).collect();
        let backup_prefix = format!("{}.corrupt-", lp.file_name().unwrap().to_string_lossy());
        assert!(
            names.iter().any(|n| n.starts_with(&backup_prefix)),
            "the garbled original is preserved to a sibling: {names:?}"
        );

        // THE EXACT BUG THIS ROUND CLOSES: the corrupt-triggering read is
        // immediately followed by a normal `record` (write_log), mirroring
        // what a live save actually does — the sibling backup must SURVIVE
        // that overwrite of the (now-recovered/empty) main log.
        record(&p, "first snapshot after the corruption", &cfg_on());
        let names_after: Vec<String> =
            crate::fs::active().read_dir(dir).unwrap().into_iter().map(|e| e.name).collect();
        assert!(
            names_after.iter().any(|n| n.starts_with(&backup_prefix)),
            "the sibling backup survives the very next write_log flush: {names_after:?}"
        );
        // And the main log itself now holds the new snapshot, unaffected.
        assert_eq!(list(&p).len(), 1);
    });
}

#[test]
fn read_log_never_backs_up_a_merely_empty_or_absent_log() {
    let p = PathBuf::from("/notes/fresh.md");
    crate::fs::with_fs(Arc::new(InMemoryFs::new().with_dir("/notes")), || {
        // No log file at all yet.
        assert!(read_log(&p).is_empty());
        let lp = log_path(&p);
        let dir_exists = crate::fs::active().is_dir(lp.parent().unwrap());
        assert!(!dir_exists, "nothing was ever preserved for an absent log");

        // An intact-but-empty log (write_log with zero entries) round-trips
        // clean, no backup.
        write_log(&p, &[]);
        assert!(read_log(&p).is_empty());
        let names: Vec<String> = crate::fs::active()
            .read_dir(lp.parent().unwrap())
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert!(
            !names.iter().any(|n| n.contains(".corrupt-")),
            "an intact empty log is not corruption: {names:?}"
        );
    });
}
