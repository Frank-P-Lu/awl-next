//! src/updates.rs — CHECK FOR UPDATES: the app never phones home, full stop.
//!
//! This module composes ONE static URL (the site's own `/check` page, carrying
//! this build's version as a query param) and remembers WHEN the user last
//! asked, via a tiny marker file — the crash-notice acknowledge-marker pattern
//! (`crashlog.rs`'s `marker_path`/`acknowledge`) reused verbatim, not
//! reinvented. The actual "is there a newer version" COMPARISON happens in the
//! **browser**, against a static `version.json` the site regenerates at deploy
//! (`site/check.html` + `.github/workflows/deploy-web.yml`) — the awl binary
//! itself performs zero network I/O, ever, strengthening (not bending)
//! CLAUDE.md's zero-network law: this is the "banked, narrow amendment" that
//! law already named — a USER-INVOKED command, fired only on a deliberate
//! keypress, that hands off to the OS browser exactly like "Report a Problem"
//! / follow-link-at-point already do, rather than adding any fetch/HTTP client
//! dependency to the app at all.
//!
//! Palette: **"Check for Updates"** (`commands.rs`, `native_only: true` — the
//! web build updates by deploy/refresh, so a "check" command is meaningless
//! there; see the availability laws in `commands.rs`). Firing it does two
//! things: (a) records a LOCAL "last checked" marker
//! (`fs::data_root()/last-update-check`, atomic-write, best-effort — a write
//! failure never blocks the handoff) and (b) hands [`check_url`] off to the OS
//! default browser via the SAME [`crate::app::App::follow_link`] seam
//! `Action::FollowLink` / "Report a Problem" already use. LIVE-APP-ONLY: the
//! headless `--keys` replay's `Effect::CheckForUpdates` arm is a documented
//! no-op (mirrors `crashlog::acknowledge`'s own live-only marker write AND
//! `Effect::FollowLink`'s never-spawns-in-a-capture rule at once), so a
//! settled capture never touches the marker file and never opens a browser.
//!
//! The About card's "checked … ago" line ([`checked_line`]) is the ONE owner
//! shared by the pixels (`render/chrome/hud.rs`) and the sidecar
//! (`capture/sidecar.rs`), mirroring `hud::saved_readout`'s determinism shape
//! exactly (see that function's own doc for the precedent this one repeats).

#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

/// The site's own check page. [`check_url`] composes the full URL a click
/// hands to the OS browser; the site's JS (`site/check.html`) does the actual
/// version comparison against its own `version.json` — never this binary.
pub const CHECK_BASE_URL: &str = "https://awl-editor.fly.dev/check";

/// Compose the browser-handoff URL: the site's check page with this build's
/// version as a `?v=` query param, percent-encoded exactly like every other
/// URL this app composes (`crashlog::url_encode` — reused, not reimplemented,
/// so encoding behavior can never drift between the two OS-handoff URLs this
/// app builds). Pure — no fs/clock — so it is unit-testable without a
/// filesystem.
pub fn check_url(version: &str) -> String {
    format!("{CHECK_BASE_URL}?v={}", crate::crashlog::url_encode(version))
}

// --- Native-only: the "last checked" marker (mirrors crashlog's own gate — --
// --- the command itself is native_only: true, meaningless on the web build) -

/// The "last checked" marker's path: a tiny text file beside the crash logs /
/// scratch stash (`fs::data_root()`), mirroring `crashlog::marker_path`'s own
/// beside-the-logs convention. Holds nothing but a decimal Unix-epoch-seconds
/// timestamp — never document content, never a version string (the marker
/// only ever answers "when", the site answers "what version").
#[cfg(not(target_arch = "wasm32"))]
pub fn marker_path(dir: &Path) -> PathBuf {
    dir.join("last-update-check")
}

/// Record `now_secs` (Unix epoch seconds) as the moment the user last checked.
/// Best-effort, exactly like `crashlog::acknowledge` — a write failure (a
/// read-only data dir, a full disk) never blocks the browser handoff, so this
/// rides [`crate::fs::write_atomic`] and swallows its `Result`.
#[cfg(not(target_arch = "wasm32"))]
pub fn record_checked(dir: &Path, now_secs: u64) {
    let fs = crate::fs::active();
    let _ = fs.create_dir_all(dir);
    let _ = crate::fs::write_atomic(&marker_path(dir), now_secs.to_string().as_bytes());
}

/// Read back the last-checked timestamp (Unix epoch seconds), or `None` if the
/// marker is missing / unreadable / not a valid integer — never a crash on a
/// hand-edited or corrupt marker file.
#[cfg(not(target_arch = "wasm32"))]
pub fn last_checked(dir: &Path) -> Option<u64> {
    crate::fs::active()
        .read_to_string(&marker_path(dir))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
}

/// The About card's "last checked" figure, LIVE-pushed every `sync_view`
/// (`App::sync_hud_update_check`, mirroring `App::sync_hud_saved` /
/// `hud::HudSaved` exactly): `Never` when no marker exists yet (the live card
/// OMITS its line entirely — nothing to report yet), `CheckedAgo` carries the
/// whole elapsed seconds for [`checked_line`] to phrase.
/// Only ever CONSTRUCTED by the live App's native-only `sync_update_checked`
/// (the command itself is `native_only: true` — meaningless on the web
/// build), mirroring `hud::HudSaved`'s own construct-native/read-everywhere
/// shape exactly, hence the same wasm-only dead-code allow rather than a
/// broader gate (`checked_line`, which matches on it, stays reachable on
/// every platform).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub enum UpdateChecked {
    Never,
    CheckedAgo(u64),
}

/// Resolve the live state from the marker + a clock reading, one call per
/// `sync_view` (mirrors `App::sync_hud_saved`'s own shape: read a store,
/// derive an enum, hand it to the pipeline).
#[cfg(not(target_arch = "wasm32"))]
pub fn update_checked_state(dir: &Path, now_secs: u64) -> UpdateChecked {
    match last_checked(dir) {
        Some(then) => UpdateChecked::CheckedAgo(now_secs.saturating_sub(then)),
        None => UpdateChecked::Never,
    }
}

// --- Pure formatting, both platforms (used by the renderer + sidecar) ------

/// Calm relative-time phrasing for elapsed SECONDS since the last check —
/// mirrors `hud::saved_readout`'s own bucket shape, extended with a DAY bucket
/// since "when did I last check for updates" plausibly spans days (unlike a
/// save, which is always session-recent): "just now" under 5s, `"Ns ago"`
/// under a minute, `"Nm ago"` under an hour, `"Nh ago"` under a day, `"Nd ago"`
/// beyond — no upper bound.
fn relative_ago(secs: u64) -> String {
    if secs < 5 {
        "just now".to_string()
    } else if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

/// The ONE owner of the About card's "checked …" line — shared by the pixels
/// (`render/chrome/hud.rs::prepare_hud`) and the sidecar
/// (`capture/sidecar.rs::about_json`), so the two can never disagree.
/// `state = None` is the CAPTURE determinism boundary (the live-only
/// `sync_hud_update_check` seam was never called this run — no clock/fs read
/// happened) and always renders the fixed dash placeholder, mirroring
/// `hud::saved_readout`'s own `None` arm; `Some(Never)` (live, no marker
/// written yet) OMITS the line entirely (`None` return — nothing to report);
/// `Some(CheckedAgo(secs))` phrases the elapsed time via [`relative_ago`].
pub fn checked_line(state: Option<UpdateChecked>) -> Option<String> {
    match state {
        None => Some("checked —".to_string()),
        Some(UpdateChecked::Never) => None,
        Some(UpdateChecked::CheckedAgo(secs)) => Some(format!("checked {}", relative_ago(secs))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- check_url: pure URL composition ------------------------------------

    #[test]
    fn check_url_embeds_the_version_as_a_query_param() {
        assert_eq!(
            check_url("0.1.0"),
            "https://awl-editor.fly.dev/check?v=0.1.0"
        );
    }

    #[test]
    fn check_url_percent_encodes_a_prerelease_suffix() {
        // '+' and other reserved chars round-trip through the SAME encoder
        // `report_problem_mailto` uses — a version like "0.2.0-beta.1+build"
        // must never break the query string.
        let url = check_url("0.2.0+build.7");
        assert!(url.starts_with("https://awl-editor.fly.dev/check?v=0.2.0"));
        assert!(!url.contains('+'), "raw '+' must be percent-encoded: {url}");
    }

    // --- relative_ago / checked_line: pure formatting -----------------------

    #[test]
    fn checked_line_none_is_the_capture_placeholder() {
        assert_eq!(checked_line(None), Some("checked —".to_string()));
    }

    #[test]
    fn checked_line_never_omits_the_line() {
        assert_eq!(checked_line(Some(UpdateChecked::Never)), None);
    }

    #[test]
    fn checked_line_phrases_relative_time_calmly() {
        assert_eq!(
            checked_line(Some(UpdateChecked::CheckedAgo(0))),
            Some("checked just now".to_string())
        );
        assert_eq!(
            checked_line(Some(UpdateChecked::CheckedAgo(4))),
            Some("checked just now".to_string())
        );
        assert_eq!(
            checked_line(Some(UpdateChecked::CheckedAgo(5))),
            Some("checked 5s ago".to_string())
        );
        assert_eq!(
            checked_line(Some(UpdateChecked::CheckedAgo(59))),
            Some("checked 59s ago".to_string())
        );
        assert_eq!(
            checked_line(Some(UpdateChecked::CheckedAgo(60))),
            Some("checked 1m ago".to_string())
        );
        assert_eq!(
            checked_line(Some(UpdateChecked::CheckedAgo(3599))),
            Some("checked 59m ago".to_string())
        );
        assert_eq!(
            checked_line(Some(UpdateChecked::CheckedAgo(3600))),
            Some("checked 1h ago".to_string())
        );
        assert_eq!(
            checked_line(Some(UpdateChecked::CheckedAgo(86_399))),
            Some("checked 23h ago".to_string())
        );
        assert_eq!(
            checked_line(Some(UpdateChecked::CheckedAgo(86_400))),
            Some("checked 1d ago".to_string())
        );
        assert_eq!(
            checked_line(Some(UpdateChecked::CheckedAgo(864_000))),
            Some("checked 10d ago".to_string())
        );
    }

    // --- marker round-trip (native-only, real temp dir, injected clock) ----

    #[cfg(not(target_arch = "wasm32"))]
    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "awl_updates_test_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn last_checked_is_none_before_the_first_record() {
        let dir = tmp_dir("empty");
        assert_eq!(last_checked(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn record_checked_round_trips_the_timestamp() {
        let dir = tmp_dir("roundtrip");
        record_checked(&dir, 1_700_000_000);
        assert_eq!(last_checked(&dir), Some(1_700_000_000));
        // A later record overwrites, never appends.
        record_checked(&dir, 1_700_000_500);
        assert_eq!(last_checked(&dir), Some(1_700_000_500));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn record_checked_creates_a_missing_data_dir() {
        let dir = tmp_dir("missing").join("nested");
        assert!(!dir.exists());
        record_checked(&dir, 42);
        assert_eq!(last_checked(&dir), Some(42));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn last_checked_ignores_a_corrupt_marker() {
        let dir = tmp_dir("corrupt");
        let _ = crate::fs::write_atomic(&marker_path(&dir), b"not-a-number");
        assert_eq!(last_checked(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn update_checked_state_is_never_before_any_record_then_checked_ago_after() {
        let dir = tmp_dir("state");
        assert_eq!(update_checked_state(&dir, 1000), UpdateChecked::Never);
        record_checked(&dir, 1000);
        assert_eq!(
            update_checked_state(&dir, 1090),
            UpdateChecked::CheckedAgo(90)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
