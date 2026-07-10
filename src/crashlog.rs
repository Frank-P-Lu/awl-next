//! src/crashlog.rs — LOCAL, OFFLINE crash visibility: the solo-dev feedback
//! channel that never violates awl's zero-network invariant (see CLAUDE.md's
//! "Supply chain" section — awl never phones home, ever). Three doors:
//!
//!   1. **Panic hook (native, LIVE APP ONLY):** on ANY panic, write a plain-text
//!      log to `fs::data_root()/crashes/awl-crash-<UTC timestamp>.log` — version,
//!      OS/arch, uptime, the panic message/location, and a backtrace — then
//!      re-raise via the CHAINED previous hook. Installed exactly once, from
//!      `crate::app::run`'s native branch (see the CAPTURE GATE below).
//!   2. **"Report a Problem" palette command (every platform):** composes a
//!      `mailto:` link to the maintainer, with a calm what-happened template and
//!      (if a crash log exists) that log's PATH — never its content — for the
//!      human to attach by hand. Opens through the SAME OS-handoff seam
//!      `Action::FollowLink` uses (`App::follow_link`).
//!   3. **Quiet next-launch notice (native, TASTE-flagged):** if a crash log is
//!      newer than the last one the user was shown, the existing bottom-center
//!      notice machinery says so once, then a marker file remembers it was shown.
//!
//! **THE PRIVACY LAW (this round's law test): a crash log — and a "Report a
//! Problem" body — can NEVER contain document content.** Enforced at the TYPE
//! level, not by convention: [`format_log`] and [`report_problem_mailto`] accept
//! only [`PanicMeta`] (static build metadata) plus plain strings the panic
//! runtime itself hands them (message/location/backtrace) — neither function's
//! signature has anywhere to plug in a `Buffer` or a document path, so there is
//! structurally nothing here that COULD leak a document's text. Not even a
//! user-doc file PATH is logged in v1 (see [`format_log`]'s doc). A content test
//! (`format_log_never_contains_a_sentinel_planted_in_a_live_document`) backs the
//! type-level guarantee with an empirical one.
//!
//! **CAPTURE GATE (mirrors the daemon/session-restore precedent exactly):**
//! [`install_hook`] is called from ONLY ONE place, `crate::app::run`'s native
//! branch — never `--screenshot`/`--keys`/`--bench-*`, which build a bare
//! `Buffer` or hermetic `App` directly and never call `crate::app::run`. So a
//! headless capture is STRUCTURALLY incapable of installing the hook; tripwire
//! test in `main/run.rs`: `headless_screenshot_never_installs_the_crash_hook`.
//!
//! **Writer is PRIMITIVE, deliberately (`write_log`):** a direct `std::fs`
//! create+write, never `fs::write_atomic`'s temp-sibling-then-rename dance and
//! never the `FileSystem` trait seam (whose active-backend lookup takes an
//! `RwLock` read guard — a lock a panicking thread might already hold, however
//! unlikely). Mid-panic, simpler wins: a half-written crash log is still far
//! more useful than losing the log to a second panic inside a fancier write
//! path. Every step here is best-effort — no unwrap, nothing panics again.
//!
//! **Wasm:** `console_error_panic_hook` (wired in `main.rs::wasm_start`) is the
//! web build's crash log — the browser console. The disk-backed machinery here
//! (`install_hook`/the crashes dir/the next-launch notice) is native-only:
//! `localStorage` quota is precious (see WEB.md), so this round doesn't add a
//! second, disk-shaped crash store on top of the console awl already has. The
//! PURE mailto composition (`report_problem_mailto`/[`url_encode`]) compiles and
//! is usable on every platform — "Report a Problem" is `native_only: false`.

// `Path`/`PathBuf` are only touched by the native-only crashes-dir machinery
// below (the pure mailto/url-encode composition above needs no filesystem
// types at all) — gated to match, so the wasm build (which compiles this
// module for its "Report a Problem" half) carries no unused-import warning.
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

/// The maintainer address every "Report a Problem" `mailto:` is addressed to —
/// the repo's own git author email. USER-CHANGEABLE: edit this one const to
/// redirect reports to a different inbox; nothing else in the app reads it.
pub const MAINTAINER_EMAIL: &str = "franklu.99@outlook.com";

/// How many crash logs to keep on disk — oldest pruned first. A generous but
/// bounded window: enough to look back across a bad week, never an unbounded
/// pile. Native-only (the wasm build has no crashes dir to prune — see the
/// module doc).
#[cfg(not(target_arch = "wasm32"))]
pub const MAX_CRASH_LOGS: usize = 20;

/// Static panic/build metadata — THE ONLY inputs [`format_log`] and
/// [`report_problem_mailto`] ever see besides the panic runtime's own plain
/// strings. See this module's privacy-law doc: neither function's signature has
/// anywhere to plug in document content, by construction.
#[derive(Debug, Clone)]
pub struct PanicMeta {
    pub version: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
    /// Seconds since this session started, if known (best-effort — the panic
    /// hook closure captures a start `Instant`; `None` elsewhere, e.g. the
    /// "Report a Problem" command, which has no comparable session origin).
    /// Only ever READ by [`format_log`] (native-only — the wasm build never
    /// looks at it, hence the dead-code allow there; it's still SET on every
    /// platform, since `PanicMeta::current` is the one shared constructor).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub uptime_secs: Option<u64>,
}

impl PanicMeta {
    /// This build's own static metadata (version/OS/arch), with `uptime_secs`
    /// left for the caller to fill in from whatever live clock it has (or
    /// `None`).
    pub fn current(uptime_secs: Option<u64>) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            uptime_secs,
        }
    }
}

// --- Pure composition: mailto: URL (every platform) ------------------------

/// Minimal RFC 3986 percent-encoding for a `mailto:` URL's `subject`/`body`
/// query values: everything outside the UNRESERVED set (`A-Za-z0-9-_.~`) is
/// escaped, so spaces, newlines, `&`, `?`, `#`, `%` itself — anything a calm
/// multi-line template needs — survive as literal text on the receiving end
/// rather than truncating or corrupting the URL.
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Compose the "Report a Problem" `mailto:` URL — addressed to
/// [`MAINTAINER_EMAIL`], subject `"awl problem report (v<version>)"`, and a
/// calm template body: a what-happened prompt, the version/OS line, and — only
/// when `crash_log_path` is `Some` — a line naming the newest crash log's PATH
/// with a "please attach this file" note. `mailto:` links cannot attach files,
/// and the body deliberately NEVER inlines the log's own content (the privacy
/// law) — just its path, so a human can find and attach it by hand. PURE: a
/// function of `meta` + the caller-supplied path string only, so this is fully
/// unit-testable without touching a filesystem.
pub fn report_problem_mailto(meta: &PanicMeta, crash_log_path: Option<&str>) -> String {
    let subject = format!("awl problem report (v{})", meta.version);
    let mut body = String::new();
    body.push_str("What happened?\n\n\n");
    body.push_str("----\n");
    body.push_str(&format!("awl v{} - {} {}\n", meta.version, meta.os, meta.arch));
    if let Some(p) = crash_log_path {
        body.push_str("\nA crash log was found from a previous session. Please attach this file:\n");
        body.push_str(p);
        body.push('\n');
    }
    format!(
        "mailto:{}?subject={}&body={}",
        MAINTAINER_EMAIL,
        url_encode(&subject),
        url_encode(&body)
    )
}

/// The quiet next-launch notice's exact wording (a TASTE default, flagged for
/// live review — see this module's doc). One owner so the live wiring and any
/// test asserting it can never drift apart. Native-only — the notice itself is
/// a native-only feature (see the module doc's wasm split).
#[cfg(not(target_arch = "wasm32"))]
pub fn notice_text() -> &'static str {
    "awl crashed last time — \u{2318}P → Report a Problem"
}

// --- Native-only: the crashes dir, the writer, pruning, the notice marker --

/// Where crash logs live: `<data_root>/crashes/`.
#[cfg(not(target_arch = "wasm32"))]
pub fn crashes_dir() -> PathBuf {
    crate::fs::data_root().join("crashes")
}

/// A `YYYY-MM-DDTHH-MM-SSZ` UTC stamp for `secs` since the Unix epoch —
/// filename-safe (no `:`) and lexicographically sortable (so a plain string
/// sort of filenames IS a chronological sort). Pure civil-date/time arithmetic
/// (Howard Hinnant's days-from-civil algorithm, the same one `history/picker.rs`'s
/// `civil_date` uses for its date half) — no chrono dependency, wasm-safe (though
/// this fn is native-only in practice, since only the panic hook calls it).
#[cfg(not(target_arch = "wasm32"))]
pub fn utc_timestamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let tod = secs % 86_400;
    let (h, m, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let mo = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}-{m:02}-{s:02}Z")
}

/// The crash log's FILE NAME (not the full path) for a UTC-seconds stamp.
#[cfg(not(target_arch = "wasm32"))]
pub fn log_file_name(secs: u64) -> String {
    format!("awl-crash-{}.log", utc_timestamp(secs))
}

/// Render the plain-text crash log — PURE, and where the privacy law is
/// enforced at the type level: the only inputs are `meta` (static build
/// metadata) and the panic's own message/location/backtrace text. NO user
/// document path is logged, not even a basename — v1's deliberate scope: the
/// log identifies the BUILD and the CRASH, never what the user had open.
#[cfg(not(target_arch = "wasm32"))]
pub fn format_log(meta: &PanicMeta, message: &str, location: Option<&str>, backtrace: &str) -> String {
    let mut out = String::new();
    out.push_str("awl crash report\n");
    out.push_str(&format!("version:  {}\n", meta.version));
    out.push_str(&format!("platform: {} {}\n", meta.os, meta.arch));
    if let Some(secs) = meta.uptime_secs {
        out.push_str(&format!("uptime:   {secs}s\n"));
    }
    out.push_str(&format!("location: {}\n", location.unwrap_or("unknown")));
    out.push('\n');
    out.push_str("message:\n");
    out.push_str(message);
    out.push('\n');
    out.push_str("\nbacktrace:\n");
    out.push_str(backtrace);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Extract a panic's human-readable message — the `&str`/`String` payload
/// virtually every real panic carries (`panic!("...")`, an `.unwrap()`
/// message, an `assert!` failure); anything else prints a generic placeholder.
/// Never any document content: a panic payload is never a `Buffer`.
#[cfg(not(target_arch = "wasm32"))]
pub fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    if let Some(s) = info.payload().downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "(non-string panic payload)".to_string()
    }
}

/// Write `contents` to `dir/name`, creating `dir` first — a PRIMITIVE direct
/// `std::fs` write (see this module's doc for why: mid-panic, simplicity wins
/// over `fs::write_atomic`'s temp-and-rename dance or the `FileSystem` trait's
/// `RwLock`). Best-effort: returns the `io::Error` rather than panicking again.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_log(dir: &Path, name: &str, contents: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(name);
    std::fs::write(&path, contents.as_bytes())?;
    Ok(path)
}

/// List `dir`'s crash log filenames (`awl-crash-*.log`), sorted ascending
/// (oldest first) — the filenames sort chronologically by construction
/// ([`utc_timestamp`]'s zero-padded, big-endian fields). A missing/unreadable
/// dir reads as empty, never an error (a fresh install has no crashes yet).
#[cfg(not(target_arch = "wasm32"))]
fn list_logs(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("awl-crash-") && n.ends_with(".log"))
        .collect();
    names.sort();
    names
}

/// PURE: given the SORTED (oldest-first) list of existing log names, return the
/// ones to PRUNE so at most `keep` newest survive — by NAME order (already
/// chronological), never by a separate mtime read, so this is fully unit-testable
/// with plain strings and no filesystem at all. Native-only (pruning is part of
/// the native crashes-dir machinery — see the module doc's wasm split).
#[cfg(not(target_arch = "wasm32"))]
pub fn names_to_prune(names: &[String], keep: usize) -> Vec<String> {
    if names.len() <= keep {
        return Vec::new();
    }
    names[..names.len() - keep].to_vec()
}

/// Prune `dir` down to the newest `keep` crash logs, deleting the rest. The I/O
/// wrapper around the pure [`names_to_prune`] — best-effort (an individual
/// remove failure is silently skipped, never fatal, exactly like the writer).
#[cfg(not(target_arch = "wasm32"))]
pub fn prune_dir(dir: &Path, keep: usize) {
    let names = list_logs(dir);
    for n in names_to_prune(&names, keep) {
        let _ = std::fs::remove_file(dir.join(n));
    }
}

/// List `dir`'s crash log filenames through the ACTIVE [`crate::fs::FileSystem`]
/// backend — test-hermetic (an `InMemoryFs`-backed test, or the "TEST
/// HERMETICITY" door `App::new_hermetic` installs, sees none unless it wrote
/// some) — for ordinary (non-panic) reads: the next-launch notice check and
/// "Report a Problem"'s newest-log lookup. Contrast with the panic hook's own
/// [`list_logs`], a raw `std::fs` read reserved for the hook's closure (see this
/// module's doc on why the writer stays primitive); every OTHER App-startup
/// read in this codebase (recents/session/stats/…) already goes through this
/// same seam, so this is consistent with the existing convention, not a new one.
#[cfg(not(target_arch = "wasm32"))]
fn list_logs_via_fs(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = crate::fs::active()
        .read_dir(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.is_file)
        .map(|e| e.name)
        .filter(|n| n.starts_with("awl-crash-") && n.ends_with(".log"))
        .collect();
    names.sort();
    names
}

/// The newest crash log's FILENAME in `dir`, if any (the LAST name once
/// sorted — chronologically last by construction). Ordinary-path read (see
/// [`list_logs_via_fs`]'s doc) — used by [`pending_notice`] and by
/// `App::report_problem`'s mailto composer.
#[cfg(not(target_arch = "wasm32"))]
pub fn newest_log(dir: &Path) -> Option<String> {
    list_logs_via_fs(dir).pop()
}

/// The next-launch NOTICE marker's path: a tiny text file, beside the logs
/// themselves, remembering the filename of the crash log the user was last
/// SHOWN the notice for (so it fires once per crash, not once per launch).
#[cfg(not(target_arch = "wasm32"))]
fn marker_path(dir: &Path) -> PathBuf {
    dir.join(".last-acknowledged")
}

/// If a crash log exists that is NEWER than the last acknowledged one, return
/// its filename (the caller shows [`notice_text`] and calls [`acknowledge`]).
/// `None` when there is no crash log, or the newest one was already shown.
/// Ordinary-path read, through the active `FileSystem` backend.
#[cfg(not(target_arch = "wasm32"))]
pub fn pending_notice(dir: &Path) -> Option<String> {
    let newest = newest_log(dir)?;
    let acked = crate::fs::active().read_to_string(&marker_path(dir)).ok();
    if acked.as_deref() == Some(newest.as_str()) {
        None
    } else {
        Some(newest)
    }
}

/// Mark `name` as the acknowledged (already-notified) crash log. Best-effort —
/// a write failure just means the notice may recur next launch, never fatal.
/// Ordinary-path write, through the active `FileSystem` backend.
#[cfg(not(target_arch = "wasm32"))]
pub fn acknowledge(dir: &Path, name: &str) {
    let fs = crate::fs::active();
    let _ = fs.create_dir_all(dir);
    let _ = fs.write(&marker_path(dir), name.as_bytes());
}

// --- The panic hook itself (native, LIVE APP ONLY — see the CAPTURE GATE) --

/// Testable witness for whether [`install_hook`] has run in THIS process — read
/// only by the headless-capture-gate tripwire test in `main/run.rs`
/// (`headless_screenshot_never_installs_the_crash_hook`), never by live code.
#[cfg(not(target_arch = "wasm32"))]
static HOOK_INSTALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether [`install_hook`] has been called yet in this process. Test-only.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub fn hook_installed_for_test() -> bool {
    HOOK_INSTALLED.load(std::sync::atomic::Ordering::SeqCst)
}

/// Install the panic hook that writes a local crash log on ANY panic —
/// LIVE APP ONLY (see this module's CAPTURE GATE doc: called exactly once,
/// from `crate::app::run`'s native branch, never any `--screenshot`/`--keys`/
/// `--bench-*` path). CHAINS the previous hook (never replaces it — the
/// default hook's own stderr backtrace print still happens, so `RUST_BACKTRACE`
/// behavior is unchanged) and never panics itself: every I/O step inside the
/// closure is best-effort.
#[cfg(not(target_arch = "wasm32"))]
pub fn install_hook() {
    HOOK_INSTALLED.store(true, std::sync::atomic::Ordering::SeqCst);
    let start = crate::clock::Instant::now();
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = panic_message(info);
        let location =
            info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let backtrace = std::backtrace::Backtrace::force_capture();
        let meta = PanicMeta::current(Some(start.elapsed().as_secs()));
        let text = format_log(&meta, &message, location.as_deref(), &backtrace.to_string());
        let dir = crashes_dir();
        let secs = crate::clock::system_now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if write_log(&dir, &log_file_name(secs), &text).is_ok() {
            prune_dir(&dir, MAX_CRASH_LOGS);
        }
        prev(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Pure composition: mailto: URL (every platform) --------------------

    #[test]
    fn url_encode_escapes_reserved_and_whitespace_leaves_unreserved_bare() {
        assert_eq!(url_encode("abcXYZ019-_.~"), "abcXYZ019-_.~");
        assert_eq!(url_encode(" "), "%20");
        assert_eq!(url_encode("\n"), "%0A");
        assert_eq!(url_encode("a&b?c#d%e"), "a%26b%3Fc%23d%25e");
    }

    #[test]
    fn mailto_composes_maintainer_subject_and_version_without_a_crash_log() {
        let meta = PanicMeta { version: "1.2.3", os: "macos", arch: "aarch64", uptime_secs: None };
        let url = report_problem_mailto(&meta, None);
        assert!(url.starts_with(&format!("mailto:{MAINTAINER_EMAIL}?")), "{url}");
        assert!(url.contains("subject=awl%20problem%20report%20%28v1.2.3%29"), "{url}");
        // No attach-this-file line when there's no crash log.
        assert!(!url.contains("attach"), "{url}");
        // The version/OS line is present in the (percent-encoded) body.
        assert!(url.contains(&url_encode("awl v1.2.3 - macos aarch64")), "{url}");
    }

    #[test]
    fn mailto_names_the_crash_log_path_when_one_exists_never_its_content() {
        let meta = PanicMeta { version: "1.2.3", os: "macos", arch: "aarch64", uptime_secs: None };
        let path = "/home/x/.local/share/awl/crashes/awl-crash-2026-01-01T00-00-00Z.log";
        let url = report_problem_mailto(&meta, Some(path));
        assert!(url.contains("attach"), "{url}");
        assert!(url.contains(&url_encode(path)), "{url}: must name the log's own path");
    }

    // --- Native-only: format / prune / marker (real tempdirs, no data_root) --

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn notice_text_names_the_report_a_problem_command() {
        assert!(notice_text().contains("Report a Problem"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "awl-crashlog-test-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn utc_timestamp_formats_the_epoch_and_a_known_date() {
        assert_eq!(utc_timestamp(0), "1970-01-01T00-00-00Z");
        // 2021-01-01T00:00:00Z = 1609459200.
        assert_eq!(utc_timestamp(1_609_459_200), "2021-01-01T00-00-00Z");
        assert_eq!(log_file_name(0), "awl-crash-1970-01-01T00-00-00Z.log");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn format_log_privacy_law_never_contains_a_sentinel_planted_in_a_live_document() {
        // THE PRIVACY LAW, empirically: a huge, absurd "document" string sits
        // nearby (as a real local, exactly like a live document buffer would),
        // but `format_log`'s signature has nowhere to accept it — only `meta` +
        // plain panic strings. The sentinel must be nowhere in the output.
        let sentinel = "SUPER-SECRET-DOCUMENT-CONTENT-4f9c2a";
        let _live_document_stand_in = format!("# My Diary\n\n{sentinel}\nDear diary, today...");
        let meta = PanicMeta::current(Some(42));
        let log = format_log(&meta, "index out of bounds: the len is 3 but the index is 5", Some("src/x.rs:10:1"), "0: foo\n1: bar");
        assert!(!log.contains(sentinel), "crash log must never contain document content");
        assert!(log.contains("version:"));
        assert!(log.contains(meta.version));
        assert!(log.contains("uptime:   42s"));
        assert!(log.contains("src/x.rs:10:1"));
        assert!(log.contains("index out of bounds"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn format_log_omits_uptime_line_when_unknown() {
        let meta = PanicMeta::current(None);
        let log = format_log(&meta, "boom", None, "");
        assert!(!log.contains("uptime:"));
        assert!(log.contains("location: unknown"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn names_to_prune_keeps_only_the_newest_and_is_a_pure_name_sort() {
        let names: Vec<String> = (0..25).map(|i| format!("awl-crash-2026-01-{i:02}T00-00-00Z.log")).collect();
        let pruned = names_to_prune(&names, MAX_CRASH_LOGS);
        assert_eq!(pruned.len(), 5, "25 logs, keep 20 -> prune the oldest 5");
        assert_eq!(pruned, names[..5].to_vec(), "prunes the OLDEST (lowest-sorting) names");
        assert!(names_to_prune(&names, 100).is_empty(), "never prunes below the count on hand");
        assert!(names_to_prune(&[], 5).is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn write_log_creates_the_dir_and_round_trips_content() {
        let dir = tmp_dir("write");
        let sub = dir.join("crashes");
        let path = write_log(&sub, "awl-crash-2026-01-01T00-00-00Z.log", "hello crash").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello crash");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn prune_dir_deletes_down_to_the_newest_keep_real_files_on_disk() {
        let dir = tmp_dir("prune");
        for i in 0..5 {
            write_log(&dir, &format!("awl-crash-2026-01-{:02}T00-00-00Z.log", i + 1), "x").unwrap();
        }
        prune_dir(&dir, 2);
        let remaining = list_logs(&dir);
        assert_eq!(remaining.len(), 2, "prune_dir must leave exactly `keep` files on real disk");
        assert_eq!(
            remaining,
            vec![
                "awl-crash-2026-01-04T00-00-00Z.log".to_string(),
                "awl-crash-2026-01-05T00-00-00Z.log".to_string()
            ],
            "the two NEWEST survive"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn newest_log_is_none_on_an_empty_or_missing_dir() {
        // `newest_log` reads through the ACTIVE `FileSystem` backend
        // (`list_logs_via_fs`), a process global other tests swap via
        // `fs::with_fs` — take the shared serial guard so this test's reads
        // can't race a concurrent swap on another thread (the same discipline
        // every other global-touching test in this codebase follows; see
        // `crate::testlock`).
        let _g = crate::testlock::serial();
        let dir = tmp_dir("empty");
        assert_eq!(newest_log(&dir), None);
        assert_eq!(newest_log(&dir.join("nonexistent-nested")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pending_notice_fires_once_per_crash_then_acknowledges() {
        // See `newest_log_is_none_on_an_empty_or_missing_dir`'s comment: this
        // exercises the same `fs::active()`-backed reads/writes.
        let _g = crate::testlock::serial();
        let dir = tmp_dir("notice");
        // No crash logs yet: nothing to notify.
        assert_eq!(pending_notice(&dir), None);

        write_log(&dir, "awl-crash-2026-01-01T00-00-00Z.log", "x").unwrap();
        let pending = pending_notice(&dir);
        assert_eq!(pending.as_deref(), Some("awl-crash-2026-01-01T00-00-00Z.log"));
        acknowledge(&dir, pending.as_deref().unwrap());
        // Same crash, already shown: quiet now.
        assert_eq!(pending_notice(&dir), None);

        // A NEWER crash lands: fires again.
        write_log(&dir, "awl-crash-2026-06-01T00-00-00Z.log", "y").unwrap();
        assert_eq!(pending_notice(&dir).as_deref(), Some("awl-crash-2026-06-01T00-00-00Z.log"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn hook_not_installed_by_default_in_a_plain_test_process() {
        // No test drives `install_hook` (it would poison every other test's panic
        // behavior in the same process), so the witness stays false across the
        // whole `cargo test` run — this is also exactly what the headless-capture
        // gate needs to be true.
        assert!(!hook_installed_for_test());
    }
}
