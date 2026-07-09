//! src/lifetime.rs — the summoned LIFETIME STATS card: state only (rendering
//! lives in `render/chrome/hud.rs`, which reuses the HUD's float-card pipeline
//! verbatim, exactly like the About card does).
//!
//! A calm, centered personal ODOMETER — characters typed, time writing, files
//! touched, caret travel (as a whimsical desk distance), and the most-lived-in
//! theme world ("your world") — summoned via Cmd-P → "Lifetime stats". These are
//! the LIFETIME figures that used to ride along at the bottom of the HELD stats
//! HUD (Cmd-I); the HUD was trimmed to its per-doc writer figures and the odometer
//! moved here, to a read-it-don't-peek summoned card of its own.
//!
//! Unlike the HELD stats HUD (`hud.rs`), this is NOT a hold: it OPENS and stays
//! open until dismissed by ANY key or mouse click — the SAME modal-summon pattern
//! the About card (`about.rs`) uses, just with different content. It reuses the
//! About wiring verbatim across every seam (open-flag global, the `apply_core`
//! top-of-function any-key dismissal, the live App's any-mouse-press dismissal).
//!
//! One process-global mirrors the `about`/`debug`/`focus`/`hud` pattern:
//!   * `LIFETIME_OPEN` — whether the card is drawn (DEFAULT OFF / closed).
//!
//! Determinism: the figures are LIVE-ONLY — the live App pushes its persisted
//! [`crate::stats::Stats`] snapshot into the pipeline every `sync_view`; a headless
//! capture never does, so every odometer row folds to the fixed
//! [`crate::hud::PLACEHOLDER`] and a `--lifetime` capture is byte-stable across
//! machines. The open-flag defaults false, so a default `--screenshot` is
//! byte-identical.
//!
//! **Why `apply_core` itself acquires [`crate::testlock::serial`] under test:**
//! identical to the reasoning in `about.rs`'s module doc — `lifetime_open()` is
//! checked at the very TOP of `apply_core`, UNCONDITIONALLY, for every action (the
//! any-key dismissal), so a test that has never heard of this module could
//! otherwise have its own unrelated `apply_core` call walk into the dismissal
//! intercept and silently swallow its action. `apply_core` acquires the ONE
//! process-wide guard itself under `cfg(test)`, reentrant per thread, so it can
//! never self-deadlock. Because a SINGLE guard now covers every process-global,
//! there is no lock ORDER left to invert — the about↔lifetime ABBA (a real 3-way
//! hang, once) is gone by construction; see [`crate::testlock`].

/// Whether the Lifetime stats card is drawn. DEFAULT OFF (closed) — summoned only
/// via the palette "Lifetime stats" command / the `--lifetime` capture flag. The
/// shared summoned-card flag mechanism (see [`crate::card::CardFlag`]).
static LIFETIME: crate::card::CardFlag = crate::card::CardFlag::new();

/// True when the Lifetime stats card is currently summoned.
pub fn lifetime_open() -> bool {
    LIFETIME.is_open()
}

/// Open or close the card explicitly.
pub fn set_open(open: bool) {
    LIFETIME.set_open(open);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_closed() {
        let _g = crate::testlock::serial();
        set_open(false);
        assert!(!lifetime_open(), "the Lifetime stats card is closed by default");
    }

    #[test]
    fn set_open_drives_the_flag() {
        let _g = crate::testlock::serial();
        set_open(false);
        set_open(true);
        assert!(lifetime_open());
        set_open(false);
        assert!(!lifetime_open());
    }
}
