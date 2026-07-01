//! src/history.rs — AUTOMATIC LOCAL SNAPSHOTS: a tiny, git-free "local history"
//! for LOOSE files (a note, a draft, a scratch buffer that isn't versioned by
//! anything else). This is the STORE + the SAVE-HOOK's engine + the GIT-PRESENCE
//! GATE; phase 2 (the timeline picker) reads it back through [`list`] / [`load`].
//!
//! The shape (why it is the way it is):
//!   * PERSISTENCE goes through the [`crate::fs`] SEAM — never `std::fs` directly —
//!     so the same code snapshots to the real disk on native AND to `localStorage`
//!     on the web (`WebFs` already backs the trait). One store, two backends, free.
//!   * The store is ONE LOG FILE PER SOURCE PATH (`<root>/<hash>.log`), holding a
//!     bounded, newest-first list of FULL-CONTENT snapshots framed by a byte length.
//!     Full copies are simple + robust (no diffing to get wrong); PRUNE keeps it
//!     small (last [`MAX_SNAPSHOTS`], and nothing older than [`MAX_AGE_MS`]). A
//!     single log file (rewritten to prune) means the store needs only the trait's
//!     read/write — no per-file delete op the seam doesn't have.
//!   * The GIT-PRESENCE GATE decides WHO owns a file's history. A file inside a git
//!     repo (a `.git` dir in some ancestor) is git's to version — awl writes NO
//!     snapshot for it, and the timeline reads `git log` / `git show` instead (the
//!     git BACKEND of [`list`] / [`load`]). A LOOSE file (no repo) — or ANY file on
//!     the web, where there is no git — gets awl snapshots. So the two histories
//!     never double up, and awl never fights git.
//!
//! The API phase 2 builds on: [`record`] (the save-hook), [`list`] (newest-first),
//! [`load`] (round-trip the content). Same signatures for both backends.

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
}

/// Keep at most this many snapshots per file (the newest). A bound so an
/// autosaving note can't grow the log without limit.
const MAX_SNAPSHOTS: usize = 50;

/// Drop snapshots older than this (7 days, in millis). Age-prune on top of the
/// count cap so a long-idle file's stale history eventually clears.
const MAX_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1000;

/// The on-disk log's magic first line — a version tag so the format can evolve.
const MAGIC: &str = "awlhist1";

// --- The public API (the phase-2 contract) --------------------------------

/// SAVE-HOOK: record a snapshot of `content` for `path`, if awl owns this file's
/// history. A no-op when history is disabled in `cfg`, or when the file is
/// GIT-MANAGED (git versions it; the timeline reads `git log` for it). Otherwise
/// appends a full-content snapshot to the file's log and PRUNES. DEDUP: if the
/// newest existing snapshot is byte-identical, nothing is written (so a re-save
/// with no change — or an idle autosave — never spams the log). All I/O routes
/// through [`crate::fs::active`], so it works on native AND web. Best-effort:
/// any store error is swallowed (a failed history write must never disrupt a
/// save).
pub fn record(path: &Path, content: &str, cfg: &Config) {
    if !cfg.history_on() {
        return; // history switched off for loose files
    }
    if is_git_managed(path) {
        return; // git owns versioning; awl stays out of its way
    }
    let mut entries = read_log(path);
    // DEDUP: an unchanged buffer re-saved (or autosaved on a pause) adds nothing.
    if entries.first().map(|(_, c)| c == content).unwrap_or(false) {
        return;
    }
    // A strictly-increasing millis stamp doubles as the snapshot id; bump past the
    // newest so two saves in the same millisecond still get distinct ids.
    let mut ts = now_millis();
    if let Some((newest, _)) = entries.first() {
        if ts <= *newest {
            ts = newest + 1;
        }
    }
    entries.insert(0, (ts, content.to_string()));
    prune(&mut entries);
    write_log(path, &entries);
}

/// LIST a file's history, NEWEST FIRST. For a GIT-MANAGED file this reads
/// `git log` (the git backend); if git is unavailable / errors it falls back to
/// the awl log (in case any snapshot was stored before the file was committed).
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

/// The base directory the per-file history logs live under. Native honours the
/// XDG data dir (`$XDG_DATA_HOME/awl/history`, else `~/.local/share/awl/history`),
/// with a relative last-resort so the function is total when no HOME is set. On
/// the web the path is virtual (WebFs maps it onto `localStorage` keys), so a
/// fixed root suffices.
fn history_root() -> PathBuf {
    #[cfg(target_arch = "wasm32")]
    {
        PathBuf::from("/awl/history")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(x) = std::env::var_os("XDG_DATA_HOME") {
            return PathBuf::from(x).join("awl").join("history");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("awl")
                .join("history");
        }
        PathBuf::from("awl-history")
    }
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

/// PRUNE `entries` (newest-first) to stay bounded: cap the COUNT at
/// [`MAX_SNAPSHOTS`], then drop anything older than [`MAX_AGE_MS`]. The just-
/// recorded snapshot (stamped `now`) always survives both cuts. Pure, so the
/// bound is unit-testable without touching the store.
fn prune(entries: &mut Vec<(u64, String)>) {
    if entries.len() > MAX_SNAPSHOTS {
        entries.truncate(MAX_SNAPSHOTS);
    }
    let cutoff = now_millis().saturating_sub(MAX_AGE_MS);
    entries.retain(|(ts, _)| *ts >= cutoff);
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

/// `git log` for a managed file → a newest-first snapshot list (id = commit hash,
/// timestamp = author-commit seconds scaled to millis). `None` if not in a repo,
/// git is missing, or the command fails — the caller then falls back to the awl
/// log. Native only. (Read-back backend — used by [`list`].)
#[cfg(not(target_arch = "wasm32"))]
fn git_list(path: &Path) -> Option<Vec<Snapshot>> {
    let root = git_repo_root(path)?;
    let rel = path.strip_prefix(&root).ok()?;
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["log", "--format=%H %ct", "--"])
        .arg(rel)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut v = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(hash), Some(secs)) = (it.next(), it.next()) else {
            continue;
        };
        let Ok(secs) = secs.parse::<u64>() else {
            continue;
        };
        v.push(Snapshot {
            id: hash.to_string(),
            timestamp: secs * 1000,
        });
    }
    Some(v)
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

    #[test]
    fn prune_bounds_the_count() {
        // More than MAX_SNAPSHOTS distinct saves keep only the newest MAX_SNAPSHOTS.
        let p = PathBuf::from("/notes/many.md");
        crate::fs::with_fs(Arc::new(InMemoryFs::new()), || {
            for i in 0..(MAX_SNAPSHOTS + 20) {
                record(&p, &format!("version {i}"), &cfg_on());
            }
            let snaps = list(&p);
            assert_eq!(snaps.len(), MAX_SNAPSHOTS, "count is bounded");
            // The newest survives; the content is intact.
            assert_eq!(
                load(&p, &snaps[0].id).as_deref(),
                Some(format!("version {}", MAX_SNAPSHOTS + 19).as_str())
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
