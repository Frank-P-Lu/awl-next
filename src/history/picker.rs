//! src/history/picker.rs — the SUMMONED TIMELINE picker's read model: the
//! pure row composer ([`TimelineRow`]/[`timeline_rows`]/[`rows_from`]),
//! the All/Session/Today FACETING scheme ([`HISTORY_FACETS`]), and the
//! small pure helpers a row's WHEN/WHICH/counts columns are built from
//! (relative-time labels, the changed-line diff, an auto-description of
//! the edit). Split out of the former `history.rs` monolith (2026-07
//! code-organization pass) — see `store` for [`super::store::Snapshot`],
//! the read-back type this reads, and `prune` for the retention policy
//! (a picker-independent concern this file never touches).

use super::store::{list, load, Snapshot};
use crate::facets::{Facet, FacetItem, FacetScheme};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

// --- The SUMMONED TIMELINE picker's read model ----------------------------
//
// The timeline overlay is a summoned, transient picker (like the theme / outline
// pickers): it shows a file's versions NEWEST-FIRST, each row answering WHEN
// (a relative timestamp, gaining a clock time exactly when siblings share a
// label) and WHICH (the git COMMIT SUBJECT, or an AUTO-DESCRIPTION of what an
// awl snapshot edited), with a faint "+N −M" changed-count vs the CURRENT
// buffer riding the right column. Enter RESTORES the highlighted version.
// [`timeline_rows`] is a thin store-reading shell over the PURE [`rows_from`],
// which both the live App and the headless `--keys` replay build from, so the
// two summon byte-identical rows for a given `now`.

/// One ROW of the timeline picker. `when` answers "when was this?" (a relative
/// label, e.g. `"2 hr ago"`, appended with a ` HH:MM` clock time exactly when
/// sibling rows share the label); `which` answers "which edit was this?" (the
/// git COMMIT SUBJECT for a git-backed row, an [`auto_description`] for an awl
/// snapshot, possibly empty); `counts` is the faint `"+N −M"` changed-line
/// count vs the current buffer; `id` is the opaque restore key [`load`]
/// resolves back to content. Pure data — the overlay composes `when · which`
/// into its main column and rides `counts` in the faint right column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineRow {
    pub when: String,
    pub which: String,
    pub counts: String,
    pub id: String,
    /// The snapshot's wall-clock stamp (MILLIS since the epoch) — the raw datum the
    /// timeline picker's Session / Today lenses bucket by (the `when` string is a
    /// pre-composed relative label, not machine-comparable). Carried straight from
    /// [`Snapshot::timestamp`].
    pub timestamp: u64,
    /// THE CONSCIOUS MARK: `true` for a KEPT (pinned) version — carried from
    /// [`Snapshot::pinned`] so the timeline picker can draw a calm, dim marker in
    /// its secondary column (see `crate::overlay::OverlayState::new_history`).
    pub pinned: bool,
    /// NAMED SAVE POINT: the user's optional name for a kept version — carried
    /// from [`Snapshot::name`]. A named row renders its NAME as the PRIMARY cell
    /// (the timestamp demoted to the secondary column); `None` renders the
    /// ordinary WHEN · WHICH row (see `OverlayState::new_history`).
    pub name: Option<String>,
}

/// Build the timeline picker's ROWS for `path`, NEWEST-FIRST: read the store
/// ([`list`] + [`load`] per snapshot) and hand the pure [`rows_from`] the
/// snapshots, their contents, the `current` buffer text, and the injected
/// `now_ms` — so the row composition itself is a PURE function, unit-testable
/// without git/fs, and identical live vs headless for a fixed `now`. An empty
/// history yields an empty vec (the picker then shows a calm "no history yet"
/// row).
pub fn timeline_rows(path: &Path, current: &str, now_ms: u64) -> Vec<TimelineRow> {
    let snaps = list(path);
    let contents: Vec<String> = snaps
        .iter()
        .map(|s| load(path, &s.id).unwrap_or_default())
        .collect();
    rows_from(&snaps, &contents, current, now_ms)
}

/// The PURE row composer behind [`timeline_rows`]: `snaps` newest-first with
/// their `contents` parallel. Per row: `when` = [`relative_label`], then every
/// GROUP of rows whose label collides gains a ` HH:MM` clock suffix
/// ([`append_clock_when_shared`]) so siblings stay tellable-apart; `which` =
/// the git commit SUBJECT when the snapshot carries one, else (an awl
/// snapshot, or a subject-less commit) an [`auto_description`] of the edit vs
/// the NEXT-OLDER content (`contents[i+1]`, `""` for the oldest — its "edit"
/// is the whole document); `counts` = `"+N −M"` from [`line_diff_counts`] vs
/// `current`. No clock, no store — deterministic for fixed inputs.
pub fn rows_from(
    snaps: &[Snapshot],
    contents: &[String],
    current: &str,
    now_ms: u64,
) -> Vec<TimelineRow> {
    let mut whens: Vec<String> = snaps
        .iter()
        .map(|s| relative_label(now_ms, s.timestamp))
        .collect();
    let stamps: Vec<u64> = snaps.iter().map(|s| s.timestamp).collect();
    append_clock_when_shared(&mut whens, &stamps);
    snaps
        .iter()
        .zip(whens)
        .enumerate()
        .map(|(i, (snap, when))| {
            let content = contents.get(i).map(String::as_str).unwrap_or("");
            let which = match &snap.subject {
                Some(subject) => subject.clone(),
                None => {
                    let prev = contents.get(i + 1).map(String::as_str).unwrap_or("");
                    auto_description(prev, content)
                }
            };
            let (added, removed) = line_diff_counts(current, content);
            TimelineRow {
                when,
                which,
                counts: format!("+{added} −{removed}"),
                id: snap.id.clone(),
                timestamp: snap.timestamp,
                pinned: snap.pinned,
                name: snap.name.clone(),
            }
        })
        .collect()
}

// ── The history timeline's FACETING scheme (All · Session · Today) ─────────────
//
// The summoned History picker is a faceting picker (see `crate::facets`): ←/→
// regroup the versions under a lens. Both time lenses bucket the per-row
// [`TimelineRow::timestamp`] against a REFERENCE clock — Session against this
// session's start, Today against the current calendar day.
//
// DETERMINISM: the reference clocks (`now` / `session_start`) ride the [`FacetItem`]
// as `None` in the headless capture path, which has no wall clock — so Session /
// Today group NOTHING there (`history_bucket` returns `None`), degrading gracefully
// exactly like `index::with_recency`'s `now == None` path. The pure bucket never
// reads a clock itself; a test injects a fixed `now`.

/// The current SESSION's start (millis since epoch), set ONCE at live-app launch
/// ([`mark_session_start`]). `None` until set — so the headless capture (which never
/// launches an `App`) leaves it unset and the Session lens is inert there.
static SESSION_EPOCH_MS: OnceLock<u64> = OnceLock::new();

/// Mark the current wall-clock instant as this SESSION's start (idempotent — only
/// the first call in a process wins). Called once from the live `App` at launch, so
/// History's Session lens has a floor to bucket against; never from the headless path.
pub fn mark_session_start() {
    let _ = SESSION_EPOCH_MS.set(super::store::now_millis());
}

/// This session's start (millis), or `None` when untracked (headless / before
/// [`mark_session_start`]). Fed into the History picker's Session lens reference.
pub fn session_epoch_ms() -> Option<u64> {
    SESSION_EPOCH_MS.get().copied()
}

/// The history timeline's lens strip: **All** (the flat timeline home) · **Session**
/// (versions since this session started) · **Today** (versions from the current
/// calendar day). "All" is parked FIRST (strip index 0), per the settled convention.
const HISTORY_FACET_STRIP: [Facet; 3] = [
    Facet { label: "All", id: "all", sections: &[] },
    Facet { label: "Session", id: "session", sections: &["Session"] },
    Facet { label: "Today", id: "today", sections: &["Today"] },
];

/// Whether two epoch-millis stamps fall on the same UTC calendar day — a plain
/// day-index compare. Pure + injected-clock-testable. NOTE (v1 simplification,
/// logged): the boundary is UTC midnight, not the user's local midnight.
pub(super) fn same_utc_day(a: u64, b: u64) -> bool {
    const DAY_MS: u64 = 86_400_000;
    a / DAY_MS == b / DAY_MS
}

/// The history timeline's [`FacetScheme::bucket`], keyed by strip index (see
/// [`HISTORY_FACET_STRIP`]). Session keeps rows stamped at/after this session's
/// start; Today keeps rows from `now`'s calendar day. A missing per-row stamp OR a
/// missing reference clock (`None`, the headless path) opts the row out — the
/// determinism gate.
pub(super) fn history_bucket(item: FacetItem, lens_idx: usize) -> Option<&'static str> {
    let ts = item.ts?;
    match lens_idx {
        1 => (ts >= item.session_start?).then_some("Session"),
        2 => same_utc_day(ts, item.now?).then_some("Today"),
        _ => None,
    }
}

/// The history timeline's registered [`FacetScheme`], handed back by
/// [`crate::facets::scheme`] for [`crate::overlay::OverlayKind::History`].
pub static HISTORY_FACETS: FacetScheme =
    FacetScheme { strip: &HISTORY_FACET_STRIP, bucket: history_bucket };

/// Disambiguate colliding WHEN labels: for every GROUP of rows whose relative
/// label string collides ("2 hr ago" twice), append the snapshot's ` HH:MM`
/// clock time ([`clock_hm`]) — appended EXACTLY when siblings share a label,
/// so a lone label stays calm and bare. `labels` and `ts` are parallel. Pure.
pub(super) fn append_clock_when_shared(labels: &mut [String], ts: &[u64]) {
    use std::collections::HashMap;
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for l in labels.iter() {
        *seen.entry(l.as_str()).or_insert(0) += 1;
    }
    let shared: Vec<bool> = labels
        .iter()
        .map(|l| seen.get(l.as_str()).copied().unwrap_or(0) > 1)
        .collect();
    for (i, l) in labels.iter_mut().enumerate() {
        if shared[i] {
            if let Some(t) = ts.get(i) {
                l.push(' ');
                l.push_str(&clock_hm(*t));
            }
        }
    }
}

/// The zero-padded `"HH:MM"` clock time of `ts_ms` (millis since the epoch),
/// in UTC — the same civil convention as [`civil_date`], and DIVERGING from
/// the user's local wall clock by their UTC offset (accepted: the label only
/// has to tell two same-relative-label siblings apart, and a pure UTC read
/// keeps the row model clock-free and byte-stable across machines). Pure.
pub fn clock_hm(ts_ms: u64) -> String {
    let mins = (ts_ms / 60_000) % (24 * 60);
    format!("{:02}:{:02}", mins / 60, mins % 60)
}

/// The 0-based index of the FIRST line where `old` and `new` differ, clamped
/// into `new`'s line range (so it always names a line of `new`): paired lines
/// are compared until one text runs out; an identical prefix means the change
/// starts where the shorter ends (an append names the first new line, a
/// truncation the last surviving one). Identical texts clamp to `new`'s last
/// line; an empty `new` reads 0. Pure — the anchor [`auto_description`] hangs
/// its heading lookup on.
pub fn first_changed_line(old: &str, new: &str) -> usize {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let common = a.len().min(b.len());
    for i in 0..common {
        if a[i] != b[i] {
            return i;
        }
    }
    common.min(b.len().saturating_sub(1))
}

/// The character budget for an [`auto_description`] excerpt (the no-heading
/// fallback), kept short so the row's WHICH column stays a glance, not a quote.
pub(super) const EXCERPT_CHARS: usize = 42;

/// An AUTO-DESCRIPTION of the edit that produced `cur` from the next-older
/// `prev` — the timeline's WHICH column for an awl snapshot (git rows carry
/// their commit subject instead). Anchors on [`first_changed_line`], then
/// names the nearest markdown heading AT-OR-ABOVE that line (via
/// [`crate::markdown::headings`], best-effort even for non-markdown content):
/// `edited "Two flows, one engine"` — the raw TITLE, never the picker's
/// depth-indented label. With no heading above, falls back to a short
/// (~[`EXCERPT_CHARS`]-char, `…`-capped) excerpt of the first changed line,
/// returned bare; an empty line yields an empty string (the row then shows
/// its WHEN alone). Pure.
pub fn auto_description(prev: &str, cur: &str) -> String {
    let n = first_changed_line(prev, cur);
    if let Some(h) = crate::markdown::headings(cur)
        .iter()
        .rev()
        .find(|h| h.line <= n)
    {
        return format!("edited \"{}\"", h.text);
    }
    let line = cur.lines().nth(n).unwrap_or("").trim();
    let mut excerpt: String = line.chars().take(EXCERPT_CHARS).collect();
    if line.chars().count() > EXCERPT_CHARS {
        excerpt.push('…');
    }
    excerpt
}

/// Clamp a live `(line, col)` cursor into `text`'s geometry — the HISTORY
/// PREVIEW's cursor guard: a previewed (possibly shorter) version is pushed
/// into the render snapshot while the BUFFER cursor stays where it was, so the
/// drawn caret must be re-bounded into the previewed text or the glyph layer
/// would index past its rows. Lines follow the renderer's `\n`-split model;
/// `col` clamps to the line's CHAR count (end-of-line is a valid caret seat).
/// Shared by the live preview (`sync_view`) and the headless capture fold. Pure.
pub fn clamp_line_col(text: &str, line: usize, col: usize) -> (usize, usize) {
    let lines: Vec<&str> = text.split('\n').collect();
    let l = line.min(lines.len().saturating_sub(1));
    let c = col.min(lines.get(l).map(|s| s.chars().count()).unwrap_or(0));
    (l, c)
}

/// The path a buffer's HISTORY is keyed under — the ONE owner of the
/// derivation every consumer (the App's timeline gather, `restore_history`,
/// the live preview loader, and the headless replay/preview) routes through,
/// so they can never disagree: the buffer's own path, else the App-level
/// `file`, else — for the TRUE SCRATCH (no path, NOT a note) — the persistent
/// scratch stash, whose autosave records under exactly that path (so the
/// scratch's timeline is summonable too). An unnamed NOTE has no history key
/// yet (its first autosave names it) → `None`.
pub fn source_path(
    buffer_path: Option<&Path>,
    file: Option<&Path>,
    is_note: bool,
) -> Option<PathBuf> {
    buffer_path
        .or(file)
        .map(Path::to_path_buf)
        .or_else(|| (!is_note).then(crate::fs::scratch_stash_path))
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
pub(super) fn civil_date(secs: u64) -> String {
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
