//! src/history/store.rs — the STORE half: the on-disk snapshot log (one
//! log file per source path, via the [`crate::fs`] seam so it works on
//! native AND web), the GIT-PRESENCE GATE that decides whether awl or git
//! owns a file's history, the git read-back BACKEND (native `git log`/
//! `git show` shells, inert wasm stubs), and the public read/write API
//! ([`record`]/[`record_pinned`]/[`list`]/[`load`]) every caller uses.
//! Split out of the former `history.rs` monolith (2026-07
//! code-organization pass); see `prune` for the aged retention ladder
//! [`record_at`] calls after every append, and `picker` for the timeline
//! picker's read model built on top of [`list`]/[`load`].

use super::prune::prune_ladder;
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
    /// THE CONSCIOUS MARK: `true` for a deliberately KEPT (pinned) awl snapshot —
    /// prune-EXEMPT (it survives the aged retention ladder / the [`MAX_TOTAL`] cap
    /// unconditionally, and does not count against them). Always `false` for a
    /// git-backed entry (a commit is git's to keep, not awl's to pin). Carried into
    /// the timeline's [`TimelineRow::pinned`] so the picker can mark it.
    pub pinned: bool,
    /// NAMED SAVE POINT: the user's optional NAME for a kept version ("draft A",
    /// "before the rewrite") — the intent marker "this is a direction I might want
    /// to come back to". `None` for a plain keep and for every git-backed entry (a
    /// commit's name is its subject). Carried into [`TimelineRow::name`] so the
    /// timeline renders the named point distinctly (name as the primary cell).
    pub name: Option<String>,
}

/// ONE stored snapshot in a file's awl log: a millis timestamp, the FULL
/// content captured, the CONSCIOUS-MARK `pinned` flag, and the optional NAMED
/// SAVE POINT `name`. This is the store's own record type (the log-file rows
/// [`serialize_log`]/[`parse_log`] frame, the ladder prunes); [`Snapshot`] is
/// the read-back view [`list`] hands the timeline. `pinned`/`name` ride through
/// the store so a KEPT version survives a prune AND round-trips across launches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub ts: u64,
    pub content: String,
    pub pinned: bool,
    pub name: Option<String>,
}

/// The on-disk log's magic first line — a version tag so the format can evolve.
/// `awlhist2` adds a PER-ENTRY pinned flag (a third header token). A pre-pin
/// `awlhist1` log still loads (its entries read `pinned = false`); the parser
/// tolerates either magic + a 2-or-3-token header, so old stores degrade cleanly.
///
/// NAMED SAVE POINTS did NOT bump this magic — a name is an OPTIONAL FOURTH
/// header token ([`encode_name`], percent-encoded so it stays whitespace-free),
/// absent for an unnamed entry. Deliberately BIDIRECTIONALLY compatible:
/// a nameless awlhist2 log parses with `name = None` here, and an OLDER awl
/// binary reading a NAMED log simply never consumes the fourth token (its
/// `split_whitespace` header walk stops at the pin flag) — a new magic would
/// instead make the old binary distrust the whole store (preserve-corrupt +
/// empty), stranding the timeline.
const MAGIC: &str = "awlhist2";

/// The pre-pin log magic ([`MAGIC`]'s predecessor) — still ACCEPTED on read (its
/// two-token headers parse with `pinned = false`), never written.
const MAGIC_V1: &str = "awlhist1";

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
    record_at(path, content, cfg, now_millis(), false, None);
}

/// THE CONSCIOUS MARK's save-hook: record `content` as a PINNED snapshot — the
/// deliberate "keep this version" action — with an optional NAME (the NAMED SAVE
/// POINT: `Some("draft A")` from the Keep-version minibuffer, `None` for a plain
/// keep). Identical to [`record`] but the stored entry is prune-EXEMPT (see
/// [`prune_ladder`]); if `content` already matches the newest snapshot (a pin
/// right after a save), that existing entry is PINNED — and, when a name is
/// given, NAMED — in place rather than skipped, so the mark always lands. Same
/// git / history-off gates as [`record`] (a git-managed file's timeline is git
/// log — awl pins nothing there, named or not: the existing silent-no-op story).
/// Best-effort; any store error is swallowed.
pub fn record_pinned(path: &Path, content: &str, cfg: &Config, name: Option<&str>) {
    record_at(path, content, cfg, now_millis(), true, name);
}

/// [`record`] with an INJECTED clock (`now_ms`) + explicit `pinned`/`name`, so
/// the ladder prune + the pin/name paths are exercised deterministically in tests
/// — the wall-clock read lives only in the thin `record`/`record_pinned` shells.
/// Same gates, dedup, store. A whitespace-only `name` normalizes to `None` (a
/// blank minibuffer Enter is exactly the plain keep).
pub(crate) fn record_at(
    path: &Path,
    content: &str,
    cfg: &Config,
    now_ms: u64,
    pinned: bool,
    name: Option<&str>,
) {
    if !cfg.history_on() {
        return; // history switched off for loose files
    }
    if is_git_managed(path) {
        return; // git owns versioning; awl stays out of its way — always
    }
    let name: Option<String> = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string);
    let mut entries = read_log(path);
    // DEDUP: an unchanged buffer re-saved (or autosaved on a pause) adds nothing —
    // EXCEPT a pin of the already-newest version, which upgrades that entry's mark
    // in place (so "Keep version" right after a save still pins something). A NAMED
    // keep of the newest also lands (or RENAMES) its name in place; a plain
    // (nameless) re-pin never erases an existing name.
    if entries.first().map(|e| e.content == content).unwrap_or(false) {
        if pinned {
            if let Some(first) = entries.first_mut() {
                let rename = name.is_some() && first.name != name;
                if !first.pinned || rename {
                    first.pinned = true;
                    if rename {
                        first.name = name;
                    }
                    prune_ladder(&mut entries, now_ms);
                    write_log(path, &entries);
                }
            }
        }
        return;
    }
    // A strictly-increasing millis stamp doubles as the snapshot id; bump past the
    // newest so two saves in the same millisecond still get distinct ids.
    let mut ts = now_ms;
    if let Some(first) = entries.first() {
        if ts <= first.ts {
            ts = first.ts + 1;
        }
    }
    entries.insert(0, Entry { ts, content: content.to_string(), pinned, name });
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
        .map(|e| Snapshot {
            id: e.ts.to_string(),
            timestamp: e.ts,
            subject: None,
            pinned: e.pinned,
            name: e.name,
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
        .find(|e| e.ts.to_string() == id)
        .map(|e| e.content)
}

// --- The git-presence gate ------------------------------------------------

/// True if `path` lives inside a git repository — i.e. a `.git` directory exists
/// in some ancestor. This is the SNAPSHOT GATE: a file in a repo is git's to
/// version (awl writes no snapshot; the timeline reads `git log`), while a LOOSE
/// file (no ancestor `.git`) — or ANY file on the web, where localStorage carries
/// no `.git` — is awl's to snapshot. Walking for `.git` goes through the FS trait,
/// so it is deterministic + testable against an [`crate::fs::InMemoryFs`].
/// NOTES VERBS round: RENAME a file's history log to follow it — the one owner
/// so `App::rename_current_file` never has to know the log's on-disk shape
/// (`log_path`'s hash, `history_root()`). Best-effort (mirrors `record`/every
/// other store write): a missing log (nothing snapshotted yet), a git-managed
/// file (never had an awl log — `record_at`'s own unconditional gate), or a
/// destination collision (should not happen — `log_path` hashes the FULL new
/// path, which by construction has no existing log the instant BEFORE the file
/// itself is renamed there) are all silent no-ops; a real failure never disrupts
/// the rename that already succeeded on disk. A no-op when `old == new`.
pub fn rename(old: &Path, new: &Path) -> std::io::Result<()> {
    if old == new {
        return Ok(());
    }
    let old_log = log_path(old);
    let fs = crate::fs::active();
    if !fs.exists(&old_log) {
        return Ok(()); // nothing snapshotted yet — nothing to carry over
    }
    fs.rename(&old_log, &log_path(new))
}

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
pub(super) fn log_path(path: &Path) -> PathBuf {
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
///
/// PRESERVE-ON-CORRUPT: a NON-EMPTY log that [`parse_log_checked`] flags as
/// UNTRUSTED (a garbled magic line, or a header/content that stopped the
/// parse before reaching the end of the file — i.e. a genuinely truncated or
/// garbled store, not merely "not yet written") is first backed up to a
/// `.corrupt-*` sibling (`crate::durable::preserve_corrupt`) BEFORE this
/// function returns whatever entries survived the partial parse — so the
/// very next [`write_log`] (which always writes the FULL, now-possibly-
/// shorter entry list back) can never silently destroy content that never
/// made it into `entries` in the first place.
pub(super) fn read_log(path: &Path) -> Vec<Entry> {
    let lp = log_path(path);
    let bytes = match crate::fs::active().read(&lp) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    let (entries, trusted) = parse_log_checked(&bytes);
    if !trusted && !bytes.is_empty() {
        crate::durable::preserve_corrupt(&lp, &bytes);
    }
    entries
}

/// Serialize `entries` (newest-first) back to the log file, creating the history
/// dir first. Best-effort: a write error is swallowed (a failed history write
/// must never disrupt the user's save). Routes through
/// [`crate::fs::write_atomic`] (temp-sibling + rename) rather than a bare
/// `fs.write` — a kill-9 mid-write must leave the OLD full log or the NEW one,
/// never a torn log that [`parse_log_checked`] would then read as untrusted
/// on the very next launch.
pub(super) fn write_log(path: &Path, entries: &[Entry]) {
    let fs = crate::fs::active();
    let lp = log_path(path);
    if let Some(parent) = lp.parent() {
        let _ = fs.create_dir_all(parent);
    }
    let _ = crate::fs::write_atomic(&lp, &serialize_log(entries));
}

/// Frame `entries` into the log format: a `MAGIC` line, then per snapshot a
/// `"<millis> <bytelen> <pin>[ <name>]\n"` header (`pin` = `1` for a KEPT/pinned
/// entry, else `0`; `<name>` = the OPTIONAL percent-encoded NAMED-SAVE-POINT name
/// via [`encode_name`], omitted entirely for an unnamed entry — so a name-free
/// log is byte-identical to the pre-name format), the exact `bytelen` content
/// bytes, and a trailing `\n` separator. The explicit byte length makes content
/// with embedded newlines (every multi-line note) round-trip losslessly.
pub(super) fn serialize_log(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC.as_bytes());
    out.push(b'\n');
    for e in entries {
        let pin = if e.pinned { 1 } else { 0 };
        match &e.name {
            Some(n) => out.extend_from_slice(
                format!("{} {} {pin} {}\n", e.ts, e.content.len(), encode_name(n)).as_bytes(),
            ),
            None => {
                out.extend_from_slice(format!("{} {} {pin}\n", e.ts, e.content.len()).as_bytes())
            }
        }
        out.extend_from_slice(e.content.as_bytes());
        out.push(b'\n');
    }
    out
}

/// Percent-encode a NAMED SAVE POINT's name into a single WHITESPACE-FREE header
/// token: `%` and every whitespace/control CHAR (the same `char::is_whitespace`
/// predicate `split_whitespace` splits on — so U+3000 ideographic space is
/// covered too, not just ASCII space) become `%XX` per UTF-8 byte (two uppercase
/// hex digits); every other char passes verbatim, so a CJK name stays readable
/// in the log. Keeping the token whitespace-free is what lets the header stay a
/// `split_whitespace` parse (and what keeps an OLDER binary's 3-token parser
/// safely ignoring it). Pure; inverse of [`decode_name`].
pub(super) fn encode_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut buf = [0u8; 4];
    for c in name.chars() {
        if c == '%' || c.is_whitespace() || c.is_control() {
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Decode [`encode_name`]'s token back to the name: `%XX` → the byte, everything
/// else verbatim; the byte string reads back as (lossy) UTF-8. A malformed `%`
/// escape passes through literally rather than failing — a name is display text,
/// and the store must never distrust a whole log over one odd byte. Pure.
pub(super) fn decode_name(token: &str) -> String {
    let bytes = token.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let Some(hex) = bytes.get(i + 1..i + 3) {
                if let Ok(b) = u8::from_str_radix(std::str::from_utf8(hex).unwrap_or(""), 16) {
                    out.push(b);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse the log format [`serialize_log`] writes back into an [`Entry`] list,
/// preserving order (newest-first as stored). Anything malformed stops the parse
/// and returns what was read so far (a truncated / partial log degrades
/// gracefully rather than crashing). BACK-COMPAT: accepts either the current
/// [`MAGIC`] (`awlhist2`, three-token headers) or the pre-pin [`MAGIC_V1`]
/// (`awlhist1`, two-token headers) — a missing third token reads `pinned = false`,
/// so an old store loads with every entry un-pinned.
///
/// Thin wrapper over [`parse_log_checked`] that drops its TRUST flag, for
/// existing tests that only want the entries. Production code calls
/// [`parse_log_checked`] directly (via [`read_log`]) now that its trust flag
/// is load-bearing — test-only, so a non-test build doesn't warn about a
/// wrapper nothing outside the test suite calls. Gated with the SAME cfg its
/// only callers carry (`super::tests` is `#[cfg(all(test, not(target_arch =
/// "wasm32")))]`, history being native-only), so the wasm TEST build — where
/// those callers don't compile — doesn't see it as dead code either.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(super) fn parse_log(bytes: &[u8]) -> Vec<Entry> {
    parse_log_checked(bytes).0
}

/// [`parse_log`]'s exact algorithm, ALSO reporting whether the parse reached
/// the natural end of the log cleanly (`trusted = true`) or stopped early
/// because something looked wrong (`trusted = false`) — a garbled/missing
/// magic line, a truncated header, or content that ran past the end of the
/// file. `trusted = false` is the PRESERVE-ON-CORRUPT signal [`read_log`]
/// acts on: a log an EMPTY body (nothing after a valid magic line) is still
/// `trusted = true` — that's just an empty-but-intact store, not corruption.
pub(super) fn parse_log_checked(bytes: &[u8]) -> (Vec<Entry>, bool) {
    let mut out = Vec::new();
    // Skip the magic line (either known version). Bail to empty + UNTRUSTED
    // if it's neither — a garbled first line means a store we can't trust.
    let body = if let Some(rest) = bytes.strip_prefix(MAGIC.as_bytes()) {
        rest.strip_prefix(b"\n").unwrap_or(rest)
    } else if let Some(rest) = bytes.strip_prefix(MAGIC_V1.as_bytes()) {
        rest.strip_prefix(b"\n").unwrap_or(rest)
    } else {
        return (out, false);
    };
    let mut i = 0;
    while i < body.len() {
        // Read the header line up to '\n'.
        let Some(nl) = body[i..].iter().position(|&b| b == b'\n') else {
            return (out, false); // truncated mid-header: untrusted
        };
        let header = &body[i..i + nl];
        i += nl + 1;
        let header = match std::str::from_utf8(header) {
            Ok(h) => h,
            Err(_) => return (out, false),
        };
        let mut parts = header.split_whitespace();
        let (Some(ts_s), Some(len_s)) = (parts.next(), parts.next()) else {
            return (out, false);
        };
        let (Ok(ts), Ok(len)) = (ts_s.parse::<u64>(), len_s.parse::<usize>()) else {
            return (out, false);
        };
        // The pin flag is the OPTIONAL third token (absent in an awlhist1 header →
        // false); any value other than "1" reads as un-pinned.
        let pinned = parts.next() == Some("1");
        // The NAMED SAVE POINT's name is the OPTIONAL fourth token (percent-
        // encoded, see `encode_name`); absent — every pre-name log — reads `None`.
        let name = parts.next().map(decode_name).filter(|n| !n.is_empty());
        if i + len > body.len() {
            return (out, false); // truncated content: stop cleanly, untrusted
        }
        let content = String::from_utf8_lossy(&body[i..i + len]).into_owned();
        out.push(Entry { ts, content, pinned, name });
        i += len;
        // Skip the single '\n' separator after the content, if present.
        if i < body.len() && body[i] == b'\n' {
            i += 1;
        }
    }
    (out, true)
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
pub(super) fn parse_git_log_line(line: &str) -> Option<Snapshot> {
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
        // A git commit is git's to keep; awl never pins one (only loose-file awl
        // snapshots carry the conscious mark) — and never names one either (a
        // commit's name is its subject, already carried above).
        pinned: false,
        name: None,
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
