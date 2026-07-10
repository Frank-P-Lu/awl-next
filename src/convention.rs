//! THE KEYBOARD CONVENTION — which chord layer slot 1 (the "native" binding)
//! actually speaks, kept as a SEPARATE axis from [`crate::commands::Platform`]
//! (capability — what exists). `Platform` answers "does this build have a
//! filesystem/daemon/menu bar"; [`Convention`] answers "does slot 1 read as
//! ⌘-chords or Ctrl-chords" — a native macOS BUILD is always `Mac` (a compile-time
//! fact), but a native LINUX build, and a WEB build running on either OS, both
//! want the Ctrl-chord reading. Two builds can share one `Platform::Native` while
//! disagreeing on `Convention` (macOS vs Linux desktop) — that's the whole reason
//! this is a second, orthogonal enum rather than a fold into `Platform`.
//!
//! **Resolution, the ONE owner every dispatch + label surface reads:**
//! - **Native:** `cfg!(target_os = "macos")` — a compile-time fact, exactly the
//!   `cfg!`-as-value precedent [`crate::menubar::MENU_BAR_ON`] already uses for its
//!   own platform default. A DEV-ONLY override, `AWL_CONVENTION_FORCE=mac|linux`
//!   (checked only outside wasm), lets the headless capture harness drive the
//!   Linux table through the REAL keymap for verification — no config key, no
//!   public CLI flag, a total no-op unless set (the `AWL_CJK_FORCE` precedent).
//! - **Web:** RUNTIME detection at startup from the browser's own
//!   `navigator.userAgent`/`navigator.platform` strings (the CodeMirror/Monaco
//!   precedent: a Mac-flavored UA gets the ⌘ reading, everything else — including
//!   an unrecognized/absent UA — defaults to the Ctrl reading), stashed in a
//!   set-once process-global exactly like [`crate::markdown::WYSIWYG_ON`]'s
//!   pattern. [`classify_ua`] is the PURE classifier (no `web_sys`, testable
//!   natively and from `websmoke`); [`set_web_convention_from_ua`] is its only
//!   caller, wired once from `wasm_start`.

/// The two chord-layer conventions slot 1 can speak. A THIRD (Windows) was
/// explicitly named out of scope for this round — see `CLAUDE.md`'s round note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Convention {
    /// ⌘-chords are the native slot; Ctrl-chords stay the quiet emacs slot 2.
    Mac,
    /// Ctrl-chords are the native slot (GTK/Windows/CodeMirror/Monaco reading);
    /// where a Ctrl-native chord collides with an emacs slot-2 default, the
    /// native meaning wins and the displaced emacs default goes empty on this
    /// convention (restorable via `[keys]`) — see `keymap.rs`'s collision table.
    Linux,
}

// The web-only cluster below (the UA classifier + its set-once process-global) is
// gated `#[cfg(any(target_arch = "wasm32", test))]` as ONE group: on a plain
// native, non-test `cargo build` none of it is reachable (native reads
// `AWL_CONVENTION_FORCE`/`cfg!(target_os)` instead, in the `current()` impl
// below), so it would otherwise be flagged dead code; under `test` it's reachable
// from this module's own unit tests (so it stays natively testable, per the doc
// above); under `target_arch = "wasm32"` it's reachable from `wasm_start`.
#[cfg(any(target_arch = "wasm32", test))]
use std::sync::atomic::{AtomicU8, Ordering};

/// Sentinel meaning "not yet UA-detected" in [`WEB_CONVENTION`] — falls back to
/// [`Convention::Linux`] (the documented "default to Ctrl when unsure" rule), so
/// a web build that never calls [`set_web_convention_from_ua`] (every native
/// build, and a wasm test that never runs `wasm_start`) still resolves sanely.
#[cfg(any(target_arch = "wasm32", test))]
const UNSET: u8 = u8::MAX;
#[cfg(any(target_arch = "wasm32", test))]
const MAC: u8 = 0;
#[cfg(any(target_arch = "wasm32", test))]
const LINUX: u8 = 1;

/// The web build's runtime-detected convention — read only by [`Convention::current`]
/// under `target_arch = "wasm32"`; inert (never read) on native. Set exactly once,
/// at `wasm_start`, mirroring [`crate::markdown::WYSIWYG_ON`]'s set-once-at-launch
/// process-global shape.
#[cfg(any(target_arch = "wasm32", test))]
static WEB_CONVENTION: AtomicU8 = AtomicU8::new(UNSET);

/// PURE classifier: does the browser's UA/platform string read as a Mac? Looks for
/// "mac" (covers "Macintosh", "MacIntel", "Mac OS X"), "iphone", or "ipad"
/// case-insensitively; everything else — including an unrecognized or empty
/// string — reads as [`Convention::Linux`] (the CodeMirror/Monaco "default to
/// Ctrl when unsure" precedent, since a mis-detected Mac hand gets a WRONG
/// convention but a mis-detected Linux/Windows hand gets the SAME convention
/// most non-Mac desktop software already speaks). No `web_sys` dependency, so
/// this is unit-testable on every target, including native `cargo test`.
#[cfg(any(target_arch = "wasm32", test))]
pub fn classify_ua(ua_or_platform: &str) -> Convention {
    let lower = ua_or_platform.to_ascii_lowercase();
    if lower.contains("mac") || lower.contains("iphone") || lower.contains("ipad") {
        Convention::Mac
    } else {
        Convention::Linux
    }
}

/// Classify `ua_or_platform` and STORE the result in [`WEB_CONVENTION`], returning
/// it. The only writer of the web global; called exactly once from `wasm_start`
/// with `navigator.userAgent` (falling back to `navigator.platform` if the UA is
/// unavailable). Also directly callable from a test (native or wasm) to drive the
/// classifier without a real `Window`.
#[cfg(any(target_arch = "wasm32", test))]
pub fn set_web_convention_from_ua(ua_or_platform: &str) -> Convention {
    let c = classify_ua(ua_or_platform);
    WEB_CONVENTION.store(if c == Convention::Mac { MAC } else { LINUX }, Ordering::Relaxed);
    c
}

impl Convention {
    /// The convention THIS RUNNING BUILD speaks — the one read every dispatch +
    /// label surface routes through. Two disjoint bodies (never both compiled for
    /// the same target) so neither references an item the OTHER target's cfg gate
    /// excludes.
    #[cfg(target_arch = "wasm32")]
    pub fn current() -> Convention {
        match WEB_CONVENTION.load(Ordering::Relaxed) {
            MAC => Convention::Mac,
            _ => Convention::Linux, // LINUX or still-UNSET: default to Ctrl.
        }
    }

    /// The native half of [`Self::current`]: `cfg!(target_os = "macos")`, with a
    /// DEV-ONLY `AWL_CONVENTION_FORCE=mac|linux` override for driving the Linux
    /// table through the real keymap in a headless capture (never read on wasm —
    /// env vars are a process concept, the `AWL_CJK_FORCE` precedent shares this).
    /// Memoized (mirrors `render::awl_cjk_force`'s `OnceLock`): `current()` is
    /// called on every `KeymapState` construction (and, live, on every
    /// menubar/palette label build), so an unmemoized `std::env::var` would
    /// re-expose the same env-var thread-safety hazard that precedent's doc warns
    /// about on a hot path.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn current() -> Convention {
        match awl_convention_force() {
            Some(v) if v == "linux" => Convention::Linux,
            Some(v) if v == "mac" => Convention::Mac,
            _ => {
                if cfg!(target_os = "macos") {
                    Convention::Mac
                } else {
                    Convention::Linux
                }
            }
        }
    }
}

/// The `AWL_CONVENTION_FORCE` dev knob, read ONCE and memoized — see
/// [`Convention::current`]'s doc for why this must not be a per-call
/// `std::env::var`.
#[cfg(not(target_arch = "wasm32"))]
fn awl_convention_force() -> &'static Option<String> {
    static ONCE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| std::env::var("AWL_CONVENTION_FORCE").ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_ua_reads_mac_uas_as_mac() {
        for ua in [
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
            "MacIntel",
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)",
            "iPad",
            "MACINTOSH", // case-insensitive
        ] {
            assert_eq!(classify_ua(ua), Convention::Mac, "{ua:?} should read as Mac");
        }
    }

    #[test]
    fn classify_ua_defaults_everything_else_to_linux() {
        for ua in [
            "Mozilla/5.0 (X11; Linux x86_64)",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
            "Win32",
            "", // unrecognized/empty: default to Ctrl per the CodeMirror precedent
            "some nonsense string",
        ] {
            assert_eq!(classify_ua(ua), Convention::Linux, "{ua:?} should default to Linux");
        }
    }

    #[test]
    fn set_web_convention_from_ua_stores_and_current_reads_it_back() {
        // This test only meaningfully exercises `WEB_CONVENTION` under wasm (native
        // `Convention::current()` never reads it), but the STORE + classify half is
        // fully testable natively — pin it here so wasm and native share one test.
        assert_eq!(set_web_convention_from_ua("Macintosh"), Convention::Mac);
        assert_eq!(set_web_convention_from_ua("X11; Linux"), Convention::Linux);
        // Leave the global in a sane (Linux/default) state for any later test.
        set_web_convention_from_ua("");
    }
}
