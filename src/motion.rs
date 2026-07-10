//! src/motion.rs — ACCESSIBILITY TIER 1: REDUCE MOTION, one process-global
//! ([`reduced`]) resolved ONCE at LIVE startup and read at every animation-step
//! seam so every juice animator SETTLES INSTANTLY to its exact final state
//! (same position, same color, same everything) instead of easing over time.
//! Mirrors the `wysiwyg`/`debug` sticky-process-global pattern exactly
//! (`AtomicBool` + a getter/setter, no threading through render args).
//!
//! **DESIGN.md §3 note, logged honestly:** the caret's spring/juice is
//! deliberately the one thing DESIGN.md grants motion ("the caret is the only
//! thing allowed juice"). This module is the documented, USER-NEED override of
//! that law — a real vestibular-accessibility preference always wins over taste.
//! It is not a new design license: every gated animator still settles at the
//! SAME final state a full-motion run would reach: motion-off is a pure TIME
//! COMPRESSION, never a feature change (no different final caret position, no
//! skipped flinch, just zero frames of easing in between).
//!
//! **Resolution ladder** ([`resolve`], pure + unit-tested): the config key
//! `reduce_motion` (`true`/`false`) wins outright when present; ABSENT means
//! `auto` — the real OS accessibility preference where one is reachable
//! (macOS `NSWorkspace.accessibilityDisplayShouldReduceMotion`, wired via
//! [`crate::mac_chrome::system_reduce_motion`]; web
//! `matchMedia("(prefers-reduced-motion: reduce)")`), else OFF (a documented
//! scope trim for native Linux, which has no reliable cross-desktop
//! accessibility API wired here yet — the config key remains the door: set
//! `reduce_motion = true` by hand).
//!
//! **Startup-read only** (mirrors the wysiwyg/menu-bar precedent): [`resolve`]
//! runs exactly ONCE, from [`apply_at_startup`], called only by the live App's
//! own startup path (`App::new`, both native and wasm — see its call site). A
//! live OS-preference flip while awl is already running is NOT picked up mid-
//! session; a relaunch re-resolves. The Settings menu's "Reduce motion" row
//! (an ordinary sticky [`crate::settings::SettingKind::Toggle`], like Outline/
//! WYSIWYG) DOES flip the running global immediately — that's an explicit user
//! action, not a live OS-pref poll, and it persists an explicit `reduce_motion`
//! value so a later relaunch keeps the user's own choice over `auto`.
//!
//! **DETERMINISM (critical):** [`apply_at_startup`] — the ONLY function that
//! ever calls OS/browser detection or reads `Config::reduce_motion` — is called
//! ONLY by `App::new`. Every headless capture entry point
//! (`--screenshot`/`--keys`/`--capture-timeline`/`--capture-held`/`--bench-*`)
//! builds its `TextPipeline`/buffer directly and never constructs an `App`, so
//! [`reduced()`] stays at its type-level default (`false`) in EVERY capture,
//! regardless of what `--config` names — a default capture is BYTE-IDENTICAL,
//! and `--screenshot-motion` / `--capture-timeline` (which explicitly inject or
//! step a virtual glide) are unaffected. Tripwire:
//! `main::run::tests::headless_replay_never_touches_reduced_motion`.
//!
//! **What "reduced" gates** (every seam, enumerated — see each site's own doc):
//! the caret spring glide + squash-pop flinches + trailing streak
//! ([`crate::render::TextPipeline::step_caret`]), the copy-pulse selection
//! brighten/decay ([`crate::render::TextPipeline::step_copy_pulse`]), and the
//! caret-style picker's choreographed preview loop
//! ([`crate::render::TextPipeline::step_caret_preview`]) — the three (and only
//! three) callees [`crate::render::TextPipeline::advance`] OR-folds together,
//! so a future fourth animator that doesn't route through `advance` won't be
//! silently ungated (any new dt-consuming stepper should join that same seam).
//! There is currently no separate scroll-glide animator to gate (scroll is an
//! instant target, no spring) — noted so a future one doesn't get missed.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether every juice animator should settle INSTANTLY rather than ease over
/// time. DEFAULT OFF — a plain launch (and every headless capture) keeps the
/// full glide/flinch feel until [`apply_at_startup`] (live-only) says otherwise.
static REDUCED_ON: AtomicBool = AtomicBool::new(false);

/// True while reduce-motion is active (read by every animation-step seam).
pub fn reduced() -> bool {
    REDUCED_ON.load(Ordering::Relaxed)
}

/// Set reduce-motion on/off explicitly — [`apply_at_startup`]'s resolved value,
/// and the Settings menu's "Reduce motion" toggle (`App::setting_toggle`).
pub fn set_reduced(on: bool) {
    REDUCED_ON.store(on, Ordering::Relaxed);
}

/// THE resolution ladder, pure + fully unit-testable without any real OS call:
/// an explicit `config_pref` (the `reduce_motion` config key) wins outright in
/// EITHER direction; `None` (absent config) falls to `os_reduced` (the platform
/// accessibility read, or `false` where none exists).
pub fn resolve(config_pref: Option<bool>, os_reduced: bool) -> bool {
    config_pref.unwrap_or(os_reduced)
}

/// LIVE-STARTUP-ONLY: resolve the config→OS ladder for `config`'s
/// `reduce_motion` pref (consulting the real platform accessibility API where
/// one exists) and apply it to [`REDUCED_ON`]. Called exactly once, from
/// `App::new` — see the module doc's DETERMINISM note for why this must never
/// be reached from a headless capture path.
pub fn apply_at_startup(config: &crate::config::Config) {
    set_reduced(resolve(config.reduce_motion, os_reduced_motion()));
}

/// The OS/browser accessibility read for the `auto` half of the ladder —
/// dispatched per platform at compile time. macOS asks `NSWorkspace`; wasm asks
/// the browser's `matchMedia`; every other native target (Linux) has no
/// reliable cross-desktop API wired here yet, so `auto` reads as OFF there (a
/// documented scope trim — `reduce_motion = true` in the config is the door).
#[cfg(target_os = "macos")]
fn os_reduced_motion() -> bool {
    crate::mac_chrome::system_reduce_motion()
}

#[cfg(target_arch = "wasm32")]
fn os_reduced_motion() -> bool {
    web_sys::window()
        .and_then(|w| w.match_media("(prefers-reduced-motion: reduce)").ok().flatten())
        .map(|mql| mql.matches())
        .unwrap_or(false)
}

#[cfg(all(not(target_os = "macos"), not(target_arch = "wasm32")))]
fn os_reduced_motion() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_true_wins_over_os_false() {
        assert!(resolve(Some(true), false));
    }

    #[test]
    fn config_false_wins_over_os_true() {
        assert!(!resolve(Some(false), true));
    }

    #[test]
    fn absent_config_falls_to_os_true() {
        assert!(resolve(None, true));
    }

    #[test]
    fn absent_config_falls_to_os_false() {
        assert!(!resolve(None, false));
    }

    #[test]
    fn getter_setter_round_trip() {
        let _g = crate::testlock::serial();
        let saved = reduced();
        set_reduced(true);
        assert!(reduced());
        set_reduced(false);
        assert!(!reduced());
        set_reduced(saved);
    }

    /// Native non-mac/non-wasm `auto` is a documented OFF (no OS API wired).
    #[cfg(all(not(target_os = "macos"), not(target_arch = "wasm32")))]
    #[test]
    fn native_linux_auto_is_off() {
        assert!(!os_reduced_motion());
    }
}
