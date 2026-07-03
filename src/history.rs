//! src/history.rs — AUTOMATIC LOCAL SNAPSHOTS: a tiny, git-free "local history"
//! for LOOSE files (a note, a draft, the persistent scratch stash — anything not
//! versioned by anything else). This is the STORE + the SAVE-HOOK's engine + the
//! GIT-PRESENCE GATE; the timeline picker reads it back through [`list`] / [`load`].
//!
//! The shape (why it is the way it is):
//!   * PERSISTENCE goes through the [`crate::fs`] SEAM — never `std::fs` directly —
//!     so the same code snapshots to the real disk on native AND to `localStorage`
//!     on the web (`WebFs` already backs the trait). One store, two backends, free.
//!   * The store is ONE LOG FILE PER SOURCE PATH (`<root>/<hash>.log`), holding a
//!     bounded, newest-first list of FULL-CONTENT snapshots framed by a byte length.
//!     Full copies are simple + robust (no diffing to get wrong); the AGED
//!     RETENTION LADDER ([`prune_ladder`]) keeps it small by thinning RESOLUTION,
//!     never memory: everything fresh is kept, then one survivor per work session
//!     up to a day, one per day up to a month, one per week beyond — the total
//!     capped at [`MAX_TOTAL`] by climbing the ladder harder (never FIFO). A
//!     single log file (rewritten to prune) means the store needs only the trait's
//!     read/write — no per-file delete op the seam doesn't have.
//!   * The GIT-PRESENCE GATE decides WHO owns a file's history, ABSOLUTELY. A file
//!     inside a git repo (a `.git` dir in some ancestor) is git's to version — awl
//!     writes NO snapshot for it, EVER (no save hook, no autosave hook — writing
//!     the file itself is not version-meddling; snapshotting it would be), and the
//!     timeline reads `git log` / `git show` instead (the git BACKEND of [`list`]
//!     / [`load`]). A LOOSE file (no repo) — or ANY file on the web, where there
//!     is no git — gets awl snapshots. So the two histories never double up, and
//!     awl never fights git. (This SUPERSEDES the old `record_periodic` contract,
//!     which snapshotted inside repos on an opt-in interval; the autosave engine
//!     replaced the interval, and git files are now git-only.)
//!
//! The read/write API: [`record`] (the save-hook — every save, manual or
//! autosave), [`list`] (newest-first), [`load`] (round-trip the content). Same
//! signatures for both backends.

use crate::config::Config;
use std::path::{Path, PathBuf};

/// One point in a file's history — a timestamp + an opaque id [`load`] resolves
/// back to content. For an awl snapshot the id is the millis timestamp as a
/// string; for a git-backed entry it is the commit hash. `timestamp` is always
/// MILLIS since the Unix epoch (git's second-granular `%ct` is scaled up), so the
/// two backends sort the same way (newest first).
///
/// [`Snapshot`] / [`list`] / [`load`] are the read-back contract the SUMMONED
/// HISTORY TIMELINE picker consumes (see [`timeline_rows`]); the save-hook side is
/// [`record`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The opaque restore key: an awl snapshot's millis-stamp string, or a git
    /// commit hash. Pass it back to [`load`] to reconstruct the content.
    pub id: String,
    /// Millis since the Unix epoch when this snapshot was taken / committed.
    pub timestamp: u64,
    /// The git COMMIT SUBJECT for a git-backed entry (the timeline's WHICH
    /// column), `None` for an awl snapshot (the timeline derives an
    /// auto-description from the content instead).
    pub subject: Option<String>,
}

// --- The AGED RETENTION LADDER's rungs (all millis) ------------------------

/// Keep EVERYTHING at most this old (15 min): the undo-adjacent recent past
/// stays at full resolution.
const FRESH_MS: u64 = 15 * 60_000;

/// Two snapshots closer than this belong to the same WORK SESSION (15 min of
/// quiet ends a session); between the fresh window and a day old, only each
/// session's LAST snapshot survives.
const SESSION_GAP_MS: u64 = 15 * 60_000;

/// One day, the session-band horizon: older than this, resolution drops to
/// one snapshot per day.
const DAY_MS: u64 = 86_400_000;

/// The daily band's horizon (~30 days): older than this, resolution drops to
/// one snapshot per week.
const OLD_HORIZON_MS: u64 = 30 * DAY_MS;

/// One week, the oldest band's bucket width.
const WEEK_MS: u64 = 7 * DAY_MS;

/// The BACKSTOP total cap per file. Enforced by climbing the ladder HARDER
/// (each level doubles the session gap + bucket widths and halves the fresh
/// window) — never by FIFO-dropping the oldest memory.
const MAX_TOTAL: usize = 150;

/// The on-disk log's magic first line — a version tag so the format can evolve.
const MAGIC: &str = "awlhist1";

// --- The public API (the phase-2 contract) --------------------------------

/// SAVE-HOOK: record a snapshot of `content` for `path`, if awl owns this file's
/// history. A no-op when history is disabled in `cfg`, or when the file is
/// GIT-MANAGED — that gate is UNCONDITIONAL: a file in a repo gets NO awl
/// snapshot from ANY record path, ever (git versions it; the timeline reads
/// `git log` for it). Otherwise appends a full-content snapshot to the file's
/// log and PRUNES via the aged retention ladder. DEDUP: if the newest existing
/// snapshot is byte-identical, nothing is written (so a re-save with no change —
/// or an idle autosave — never spams the log). All I/O routes through
/// [`crate::fs::active`], so it works on native AND web. Best-effort: any store
/// error is swallowed (a failed history write must never disrupt a save).
pub fn record(path: &Path, content: &str, cfg: &Config) {
    record_at(path, content, cfg, now_millis());
}

/// [`record`] with an INJECTED clock (`now_ms`), so the ladder prune is
/// exercised deterministically in tests — the wall-clock read lives only in the
/// thin `record` shell. Same gates, dedup, store.
pub(crate) fn record_at(path: &Path, content: &str, cfg: &Config, now_ms: u64) {
    if !cfg.history_on() {
        return; // history switched off for loose files
    }
    if is_git_managed(path) {
        return; // git owns versioning; awl stays out of its way — always
    }
    let mut entries = read_log(path);
    // DEDUP: an unchanged buffer re-saved (or autosaved on a pause) adds nothing.
    if entries.first().map(|(_, c)| c == content).unwrap_or(false) {
        return;
    }
    // A strictly-increasing millis stamp doubles as the snapshot id; bump past the
    // newest so two saves in the same millisecond still get distinct ids.
    let mut ts = now_ms;
    if let Some((newest, _)) = entries.first() {
        if ts <= *newest {
            ts = newest + 1;
        }
    }
    entries.insert(0, (ts, content.to_string()));
    prune_ladder(&mut entries, now_ms);
    write_log(path, &entries);
}

/// LIST a file's history, NEWEST FIRST. A GIT-MANAGED file's timeline IS git
/// log — awl never snapshots it (the unconditional gate in [`record_at`]), so
/// this reads `git log` (the git backend, ids = commit hashes, subjects
/// carried); only if git itself is unavailable / errors does it fall back to
/// the awl log (e.g. snapshots stored before the file was first committed).
/// For a loose file it reads the awl log. Empty when there is no history.
/// (Read-back API — consumed by the timeline picker via [`timeline_rows`].)
pub fn list(path: &Path) -> Vec<Snapshot> {
    if is_git_managed(path) {
        if let Some(v) = git_list(path) {
            return v;
        }
        // git unavailable: fall back to any awl snapshots.
    }
    read_log(path)
        .into_iter()
        .map(|(ts, _)| Snapshot {
            id: ts.to_string(),
            timestamp: ts,
            subject: None,
        })
        .collect()
}

/// LOAD the content of one snapshot (`id` from a [`list`] entry). For a
/// git-managed file this runs `git show <id>:<relpath>`; for a loose file it
/// finds the matching entry in the awl log. `None` if the id is unknown / the
/// backend can't produce it. The reconstructed String is byte-for-byte what was
/// captured, so a restore is just replacing the buffer text (undoable via the
/// existing undo — the timeline's Enter → `Buffer::set_text`). (Read-back API.)
pub fn load(path: &Path, id: &str) -> Option<String> {
    if is_git_managed(path) {
        if let Some(c) = git_show(path, id) {
            return Some(c);
        }
        // git unavailable: fall through to the awl log.
    }
    read_log(path)
        .into_iter()
        .find(|(ts, _)| ts.to_string() == id)
        .map(|(_, c)| c)
}

// --- The SUMMONED TIMELINE picker's read model ----------------------------
//
// The timeline overlay is a summoned, transient picker (like the theme / outline
// pickers): it shows a file's versions NEWEST-FIRST as rows of a RELATIVE
// timestamp + a tiny "+N −M lines" changed-count vs the CURRENT buffer, and Enter
// RESTORES the highlighted version. [`timeline_rows`] is the pure read model both
// the live App and the headless `--keys` replay build from, so the two summon
// byte-identical rows for a given `now`.

/// One ROW of the timeline picker: a display `label` (relative timestamp), a
/// `diff` count ("+N −M") of what restoring this version would change vs the
/// current buffer, and the opaque `id` [`load`] resolves back to content. Pure
/// data — the overlay carries these three parallel columns.
pub type TimelineRow = (String, String, String);

/// Build the timeline picker's ROWS for `path`, NEWEST-FIRST: for each snapshot a
/// `(relative-time label, "+N −M" changed-line count vs `current`, restore id)`.
/// The count is what RESTORING that version would do to the current buffer (a
/// simple line diff, [`line_diff_counts`]). `now_ms` is injected (millis since the
/// epoch) so the relative labels are a PURE function of the store + the clock —
/// unit-testable, and identical live vs headless for a fixed `now`. An empty
/// history yields an empty vec (the picker then shows a calm "no history yet" row).
pub fn timeline_rows(path: &Path, current: &str, now_ms: u64) -> Vec<TimelineRow> {
    list(path)
        .into_iter()
        .map(|s| {
            let label = relative_label(now_ms, s.timestamp);
            let content = load(path, &s.id).unwrap_or_default();
            let (added, removed) = line_diff_counts(current, &content);
            (label, format!("+{added} −{removed}"), s.id)
        })
        .collect()
}

/// A calm, human RELATIVE-TIME label for a snapshot taken at `ts_ms`, read at
/// `now_ms` (both millis since the epoch): "just now" (< 1 min), "N min ago",
/// "N hr ago", "yesterday" (1 day), "N days ago" (< a week), else a `YYYY-MM-DD`
/// date. A future stamp (clock skew) reads as "just now". PURE — the clock is the
/// injected `now_ms` — so it is unit-testable and deterministic.
pub fn relative_label(now_ms: u64, ts_ms: u64) -> String {
    let secs = now_ms.saturating_sub(ts_ms) / 1000;
    const MIN: u64 = 60;
    const HR: u64 = 60 * MIN;
    const DAY: u64 = 24 * HR;
    if secs < MIN {
        "just now".to_string()
    } else if secs < HR {
        let n = secs / MIN;
        format!("{n} min ago")
    } else if secs < DAY {
        let n = secs / HR;
        format!("{n} hr ago")
    } else if secs < 2 * DAY {
        "yesterday".to_string()
    } else if secs < 7 * DAY {
        let n = secs / DAY;
        format!("{n} days ago")
    } else {
        civil_date(ts_ms / 1000)
    }
}

/// A `YYYY-MM-DD` date for `secs` since the Unix epoch (UTC), for snapshots older
/// than a week. Pure civil-date arithmetic (Howard Hinnant's algorithm) — no
/// chrono / clock dependency, wasm-safe.
fn civil_date(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    // days since 1970-01-01 -> (year, month, day), UTC.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// A SIMPLE line diff: how many lines RESTORING `new` would ADD and REMOVE vs
/// `old`, as `(added, removed)`. Computed from the line-level LONGEST COMMON
/// SUBSEQUENCE (so a moved / unchanged line isn't miscounted), which is accurate
/// for the small notes/drafts local history covers. A size GUARD falls back to a
/// cheap multiset difference on pathologically large inputs so the O(n·m) table
/// can never blow up. Pure + deterministic — unit-testable.
pub fn line_diff_counts(old: &str, new: &str) -> (usize, usize) {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let (n, m) = (a.len(), b.len());
    // GUARD: skip the quadratic table for very large files; approximate with a
    // multiset (bag) difference, which is O(n+m) and never allocates a big grid.
    if (n + 1).saturating_mul(m + 1) > 1_000_000 {
        return multiset_line_diff(&a, &b);
    }
    // LCS length via a rolling DP row (only the length is needed).
    let mut prev = vec![0u32; m + 1];
    let mut cur = vec![0u32; m + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            cur[j] = if a[i] == b[j] {
                prev[j + 1] + 1
            } else {
                prev[j].max(cur[j + 1])
            };
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let lcs = prev[0] as usize;
    (m - lcs, n - lcs)
}

/// The large-input FALLBACK for [`line_diff_counts`]: a multiset (bag) difference
/// of the two line lists — lines present more times in `new` count as added, more
/// times in `old` as removed. Order-insensitive (a pure move reads as no change),
/// which is a fine approximation at the sizes that trip the guard.
fn multiset_line_diff(a: &[&str], b: &[&str]) -> (usize, usize) {
    use std::collections::HashMap;
    let mut counts: HashMap<&str, i64> = HashMap::new();
    for &l in a {
        *counts.entry(l).or_insert(0) -= 1;
    }
    for &l in b {
        *counts.entry(l).or_insert(0) += 1;
    }
    let mut added = 0usize;
    let mut removed = 0usize;
    for delta in counts.values() {
        if *delta > 0 {
            added += *delta as usize;
        } else {
            removed += (-*delta) as usize;
        }
    }
    (added, removed)
}

// --- The git-presence gate ------------------------------------------------

/// True if `path` lives inside a git repository — i.e. a `.git` directory exists
/// in some ancestor. This is the SNAPSHOT GATE: a file in a repo is git's to
/// version (awl writes no snapshot; the timeline reads `git log`), while a LOOSE
/// file (no ancestor `.git`) — or ANY file on the web, where localStorage carries
/// no `.git` — is awl's to snapshot. Walking for `.git` goes through the FS trait,
/// so it is deterministic + testable against an [`crate::fs::InMemoryFs`].
pub fn is_git_managed(path: &Path) -> bool {
    git_repo_root(path).is_some()
}

/// The git repository root for `path`: the nearest ANCESTOR directory that holds
/// a `.git` entry, or `None` if the file is not inside a repo. Walks parents via
/// [`crate::fs::active`] (so it sees the InMemoryFs / WebFs virtual trees too, not
/// only the real disk). The returned root anchors the `git -C <root>` backend
/// calls and the repo-relative path they need.
pub fn git_repo_root(path: &Path) -> Option<PathBuf> {
    let fs = crate::fs::active();
    let mut cur = path.parent();
    while let Some(dir) = cur {
        if fs.is_dir(&dir.join(".git")) {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

// --- The awl snapshot store (log file, via the FS trait) ------------------

/// The base directory the per-file history logs live under:
/// `<data_root>/history` — the XDG-honouring (web-virtual) awl data root lives
/// in [`crate::fs::data_root`], shared with the scratch stash.
fn history_root() -> PathBuf {
    crate::fs::data_root().join("history")
}

/// The log-file path for a source `path`: `<history_root>/<hash>.log`, where the
/// hash is a stable FNV-1a of the full path string (so the store is keyed by the
/// file, and two files never collide). Stable across runs (unlike a randomly-
/// seeded `DefaultHasher`), so yesterday's snapshots are still findable today.
fn log_path(path: &Path) -> PathBuf {
    history_root().join(format!("{:016x}.log", fnv1a(&path.to_string_lossy())))
}

/// A stable FNV-1a hash of `s` — deterministic across processes (no random seed),
/// which the log key requires so a file's history persists between launches.
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Read a file's snapshot log into a NEWEST-FIRST `(millis, content)` list. A
/// missing / unreadable / malformed log reads as empty (history is best-effort —
/// a corrupt log must never crash a save or a timeline open). Routes through the
/// FS trait, so it reads the real disk on native and localStorage on the web.
fn read_log(path: &Path) -> Vec<(u64, String)> {
    let bytes = match crate::fs::active().read(&log_path(path)) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    parse_log(&bytes)
}

/// Serialize `entries` (newest-first) back to the log file, creating the history
/// dir first. Best-effort: a write error is swallowed (a failed history write
/// must never disrupt the user's save). Routes through the FS trait.
fn write_log(path: &Path, entries: &[(u64, String)]) {
    let fs = crate::fs::active();
    let lp = log_path(path);
    if let Some(parent) = lp.parent() {
        let _ = fs.create_dir_all(parent);
    }
    let _ = fs.write(&lp, &serialize_log(entries));
}

/// The AGED RETENTION LADDER: prune `entries` (newest-first) so RESOLUTION
/// thins with age while MEMORY is kept. A PURE function of `(entries, now_ms)`
/// — the clock is injected, so the whole policy is deterministic and
/// unit-testable without the store. Level 0 keeps: everything fresher than
/// [`FRESH_MS`]; one snapshot per WORK SESSION (gaps < [`SESSION_GAP_MS`])
/// up to a day; one per DAY up to [`OLD_HORIZON_MS`]; one per WEEK beyond.
/// If the keep-set still exceeds [`MAX_TOTAL`], the ladder is climbed HARDER
/// (each level halves the fresh window and doubles the gap + bucket widths)
/// until it fits — NEVER FIFO: the file's oldest snapshot always survives.
/// The just-recorded snapshot (stamped `now`) is always fresh, so it survives.
///
/// CONSCIOUS MARK (banked, not built): a `pinned` flag per entry would union
/// into every level's keep-set, exempting a deliberately marked version
/// (pin-a-version-before-major-surgery) from all bands. Slot it here.
pub(crate) fn prune_ladder(entries: &mut Vec<(u64, String)>, now_ms: u64) {
    let ts: Vec<u64> = entries.iter().map(|(t, _)| *t).collect();
    let mut chosen = ladder_keep(&ts, now_ms, 0);
    for level in 1..=32u32 {
        if chosen.iter().filter(|k| **k).count() <= MAX_TOTAL {
            break;
        }
        chosen = ladder_keep(&ts, now_ms, level);
    }
    let mut keep = chosen.into_iter();
    entries.retain(|_| keep.next().unwrap_or(true));
}

/// One LEVEL of the retention ladder: which of the newest-first timestamps
/// `ts` survive, as a parallel keep-mask. `level` scales the rungs — the fresh
/// window HALVES (`FRESH_MS >> level`) and the session gap / day / week bucket
/// widths DOUBLE (`<< level`) each step, so a higher level keeps strictly less
/// and the cap loop in [`prune_ladder`] terminates. Band boundaries (a day, 30
/// days) stay FIXED; only the resolution inside each band coarsens. Survivor
/// of a session/day/week = its LAST (newest) snapshot; the OLDEST timestamp is
/// always kept outright (memory, not resolution). Pure.
fn ladder_keep(ts: &[u64], now_ms: u64, level: u32) -> Vec<bool> {
    let fresh = FRESH_MS >> level.min(63);
    let gap = SESSION_GAP_MS << level.min(20);
    let day_w = DAY_MS << level.min(20);
    let week_w = WEEK_MS << level.min(20);
    let n = ts.len();
    let mut keep = vec![false; n];
    // Walk OLD → NEW (reverse of the stored order) so session clustering reads
    // gaps forward in time. Track the previous member's band + key to decide
    // survivors: in the session band a new cluster starts when the forward gap
    // reaches `gap`; in the bucketed bands a new bucket starts when `ts / width`
    // changes. The NEWEST member of each cluster/bucket survives — i.e. the last
    // index visited before the cluster/bucket changes (indices shrink as time
    // advances, so "newest of the group" = the final i in that group).
    #[derive(PartialEq)]
    enum Band {
        Fresh,
        Session,
        Daily(u64),
        Weekly(u64),
    }
    let band_of = |t: u64| -> Band {
        let age = now_ms.saturating_sub(t);
        if age <= fresh {
            Band::Fresh
        } else if age <= DAY_MS {
            Band::Session
        } else if age <= OLD_HORIZON_MS {
            Band::Daily(t / day_w)
        } else {
            Band::Weekly(t / week_w)
        }
    };
    let mut prev: Option<(usize, Band, u64)> = None; // (index, band, ts)
    for i in (0..n).rev() {
        let t = ts[i];
        let band = band_of(t);
        if let Some((pi, pband, pt)) = &prev {
            let same_group = match (&band, pband) {
                (Band::Fresh, Band::Fresh) => true, // fresh keeps all anyway
                (Band::Session, Band::Session) => t.saturating_sub(*pt) < gap,
                (Band::Daily(b), Band::Daily(pb)) => b == pb,
                (Band::Weekly(b), Band::Weekly(pb)) => b == pb,
                _ => false,
            };
            if !same_group {
                // The previous group closed: its newest member survives.
                keep[*pi] = true;
            }
        }
        if band == Band::Fresh {
            keep[i] = true; // the fresh band keeps everything
        }
        prev = Some((i, band, t));
    }
    if let Some((pi, _, _)) = prev {
        keep[pi] = true; // the final (newest) group's survivor
    }
    // MEMORY over resolution: the file's ORIGIN — its oldest snapshot — is never
    // pruned away, whatever bucket alignment says.
    if let Some(last) = keep.last_mut() {
        *last = true;
    }
    keep
}

/// Frame `entries` into the log format: a `MAGIC` line, then per snapshot a
/// `"<millis> <bytelen>\n"` header, the exact `bytelen` content bytes, and a
/// trailing `\n` separator. The explicit byte length makes content with embedded
/// newlines (every multi-line note) round-trip losslessly.
fn serialize_log(entries: &[(u64, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC.as_bytes());
    out.push(b'\n');
    for (ts, content) in entries {
        out.extend_from_slice(format!("{ts} {}\n", content.len()).as_bytes());
        out.extend_from_slice(content.as_bytes());
        out.push(b'\n');
    }
    out
}

/// Parse the log format [`serialize_log`] writes back into a `(millis, content)`
/// list, preserving order (newest-first as stored). Anything malformed stops the
/// parse and returns what was read so far (a truncated / partial log degrades
/// gracefully rather than crashing).
fn parse_log(bytes: &[u8]) -> Vec<(u64, String)> {
    let mut out = Vec::new();
    // Skip the magic line (tolerate its absence — treat the whole thing as body
    // only if it actually starts with the magic; otherwise bail to empty).
    let body = match bytes.strip_prefix(MAGIC.as_bytes()) {
        Some(rest) => rest.strip_prefix(b"\n").unwrap_or(rest),
        None => return out,
    };
    let mut i = 0;
    while i < body.len() {
        // Read the header line up to '\n'.
        let Some(nl) = body[i..].iter().position(|&b| b == b'\n') else {
            break;
        };
        let header = &body[i..i + nl];
        i += nl + 1;
        let header = match std::str::from_utf8(header) {
            Ok(h) => h,
            Err(_) => break,
        };
        let mut parts = header.split_whitespace();
        let (Some(ts_s), Some(len_s)) = (parts.next(), parts.next()) else {
            break;
        };
        let (Ok(ts), Ok(len)) = (ts_s.parse::<u64>(), len_s.parse::<usize>()) else {
            break;
        };
        if i + len > body.len() {
            break; // truncated content: stop cleanly
        }
        let content = String::from_utf8_lossy(&body[i..i + len]).into_owned();
        out.push((ts, content));
        i += len;
        // Skip the single '\n' separator after the content, if present.
        if i < body.len() && body[i] == b'\n' {
            i += 1;
        }
    }
    out
}

/// Wall-clock now as millis since the Unix epoch, WASM-SAFE (via [`crate::clock`],
/// which shims the browser clock — std's `SystemTime::now()` panics on wasm). Public
/// so the timeline's caller can stamp `now` for [`relative_label`] without re-deriving
/// the wasm-safe read.
pub fn now_millis() -> u64 {
    crate::clock::system_now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// --- The git backend (list / load for git-managed files) ------------------
//
// Native shells out to `git`; the web has no git (and no process API), so the
// wasm builds compile inert stubs. Both return `None` on any failure so the
// callers ([`list`] / [`load`]) fall back to the awl log.

/// `git log` for a managed file → a newest-first snapshot list (id = commit
/// hash, timestamp = author-commit seconds scaled to millis, subject = the
/// commit's one-line summary for the timeline's WHICH column). `None` if not in
/// a repo, git is missing, or the command fails — the caller then falls back to
/// the awl log. Native only. (Read-back backend — used by [`list`].)
#[cfg(not(target_arch = "wasm32"))]
fn git_list(path: &Path) -> Option<Vec<Snapshot>> {
    let root = git_repo_root(path)?;
    let rel = path.strip_prefix(&root).ok()?;
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["log", "--format=%H %ct %s", "--"])
        .arg(rel)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Some(text.lines().filter_map(parse_git_log_line).collect())
}

/// Parse one `git log --format=%H %ct %s` line into a [`Snapshot`]: the commit
/// hash, the commit seconds (scaled to millis so the two backends sort alike),
/// and the SUBJECT (everything after the second space — subjects keep their own
/// spaces; an empty subject reads as `None`). A malformed line yields `None`
/// (skipped). Pure, so the git read-model is unit-testable without a repo.
#[cfg(not(target_arch = "wasm32"))]
fn parse_git_log_line(line: &str) -> Option<Snapshot> {
    let mut it = line.splitn(3, ' ');
    let hash = it.next().filter(|h| !h.is_empty())?;
    let secs = it.next()?.parse::<u64>().ok()?;
    let subject = it
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some(Snapshot {
        id: hash.to_string(),
        timestamp: secs * 1000,
        subject,
    })
}

/// `git show <rev>:<relpath>` → the file's content at that commit. `None` on any
/// failure (caller falls back to the awl log). Native only. (Read-back backend.)
#[cfg(not(target_arch = "wasm32"))]
fn git_show(path: &Path, id: &str) -> Option<String> {
    let root = git_repo_root(path)?;
    let rel = path.strip_prefix(&root).ok()?;
    let spec = format!("{id}:{}", rel.to_string_lossy());
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["show", &spec])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Web has no git (and no process API): the git backend is inert, so [`list`] /
/// [`load`] always use the awl log. (In practice `is_git_managed` is already
/// false on the web — localStorage has no `.git` — so these never even run.)
#[cfg(target_arch = "wasm32")]
fn git_list(_path: &Path) -> Option<Vec<Snapshot>> {
    None
}
#[cfg(target_arch = "wasm32")]
fn git_show(_path: &Path, _id: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::InMemoryFs;
    use std::sync::Arc;

    /// A config with history ON (the loose-file default).
    fn cfg_on() -> Config {
        Config::empty()
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

    /// Build a newest-first entry list from (age-ms, label) pairs against `now`.
    fn entries_aged(now: u64, ages: &[u64]) -> Vec<(u64, String)> {
        let mut v: Vec<(u64, String)> = ages
            .iter()
            .map(|a| (now - a, format!("v@{a}")))
            .collect();
        v.sort_by(|a, b| b.0.cmp(&a.0)); // newest first
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
        let kept: Vec<u64> = e.iter().map(|(t, _)| now - t).collect();
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
        let kept: Vec<u64> = e.iter().map(|(t, _)| now - t).collect();
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
            e.iter().any(|(t, _)| now - t == 48 * day),
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
            e.iter().any(|(t, _)| now - t == oldest_age),
            "the OLDEST snapshot survives the cap (never FIFO)"
        );
        assert!(
            e.iter().any(|(t, _)| now - t == 0),
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
                record_at(&p, &format!("v{i}"), &cfg_on(), t0 + i * 60_000);
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
            record_at(&p, "v2", &cfg_on(), 1_700_000_000_000);
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
        // The framed format survives embedded newlines + the header-delimiter space.
        let entries = vec![
            (1_000u64, "line one\nline two\n".to_string()),
            (999u64, "a b  c\nd".to_string()),
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
            assert_eq!(rows[0].1, "+0 −0");
            assert_eq!(load(&p, &rows[0].2).as_deref(), Some("one\ntwo\nthree"));
            // Row 1 is the older 2-line version -> restoring it removes "three": +0 −1.
            assert_eq!(rows[1].1, "+0 −1");
            assert_eq!(load(&p, &rows[1].2).as_deref(), Some("one\ntwo"));
            // Labels are non-empty relative-time strings.
            assert!(!rows[0].0.is_empty() && !rows[1].0.is_empty());
        });
    }

    #[test]
    fn timeline_rows_empty_for_a_file_with_no_history() {
        let p = PathBuf::from("/notes/fresh.md");
        crate::fs::with_fs(Arc::new(InMemoryFs::new()), || {
            assert!(timeline_rows(&p, "scratch", now_millis()).is_empty());
        });
    }
}
