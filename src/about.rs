//! src/about.rs — the summoned ABOUT card: state only (rendering lives in
//! `render/chrome.rs`, which reuses the HUD's float-card pipeline verbatim).
//!
//! A calm, centered info card — "Awl", the crate version, the active theme
//! world's name, and a closing ornament (the world's own dash fleuron, the
//! same glyph a `---` rule renders as) — summoned via Cmd-P → "About" (and,
//! on macOS, the menu bar's App → "About Awl" item). Unlike the HELD stats HUD
//! (`hud.rs`), this is NOT a hold: it OPENS and stays open until dismissed by
//! ANY key or mouse click — the modal-summon pattern the navigation overlay
//! already uses, just with no content to navigate.
//!
//! **Why this exists at all (not muda's predefined About dialog):** see
//! `menu.rs`'s module doc for the full mechanism, but in short — the OS About
//! panel is genuinely OS chrome with no correctness reason to route UNLESS you
//! also want it to look and feel like the rest of awl (calm, one warm accent,
//! `base_300` float card) rather than a stock AppKit dialog. Routing it also
//! means it works identically on Linux (no native menu bar there at all) and is
//! `--keys`/sidecar drivable like everything else in this app.
//!
//! One process-global mirrors the `debug`/`focus`/`hud` pattern:
//!   * `ABOUT_OPEN` — whether the card is drawn (DEFAULT OFF / closed).
//!
//! Dismissal is intentionally NOT scoped to Esc: `actions::apply_core` closes
//! it on the very first key it sees while open (any key, consumed, no other
//! effect — see its top-of-function intercept), and the live `App` closes it
//! on any mouse press too (`app/input/mouse.rs`). This is deliberately looser than
//! the navigation overlay's Esc/Enter contract: an about card has nothing to
//! navigate, so any dismissal gesture is equally correct.
//!
//! **Why `apply_core` itself acquires [`crate::testlock::serial`] under test:**
//! `about_open()` is checked at the very TOP of `apply_core`, UNCONDITIONALLY,
//! for every action. That makes the about global a hazard for tests that have
//! never heard of `about.rs`: if the one test that drives `Action::About` sets
//! the flag true on one thread, ANY other concurrently-running test's own
//! unrelated `apply_core` call could otherwise walk into the top-of-function
//! intercept, swallow its own action, and return `Effect::None` instead of what
//! it expected (confirmed live). Holding a lock only on the tests that KNOW to
//! ask can't close that gap, so `apply_core` acquires the ONE process-wide guard
//! itself under `cfg(test)`, reentrant per thread — a test already holding it
//! (e.g. via `Action::About`) nests for free. Because there is now a SINGLE guard
//! for EVERY process-global, the old about/lifetime/page acquire ORDER — and the
//! ABBA it once risked — is gone by construction; see [`crate::testlock`].

/// Whether the About card is drawn. DEFAULT OFF (closed) — summoned only via
/// the palette "About" command / macOS menu "About Awl" item. The shared
/// summoned-card flag mechanism (see [`crate::card::CardFlag`]).
static ABOUT: crate::card::CardFlag = crate::card::CardFlag::new();

/// True when the About card is currently summoned.
pub fn about_open() -> bool {
    ABOUT.is_open()
}

/// Open or close the card explicitly.
pub fn set_open(open: bool) {
    ABOUT.set_open(open);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_closed() {
        let _g = crate::testlock::serial();
        set_open(false);
        assert!(!about_open(), "the About card is closed by default");
    }

    #[test]
    fn set_open_drives_the_flag() {
        let _g = crate::testlock::serial();
        set_open(false);
        set_open(true);
        assert!(about_open());
        set_open(false);
        assert!(!about_open());
    }
}
