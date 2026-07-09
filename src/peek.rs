//! src/peek.rs — the HOLD-⌘ SHORTCUT PEEK: the pure hold/cancel state machine, the
//! drawn-flag process-global, the [`PeekRow`] content shape, and the curated
//! STARTER SIX fallback.
//!
//! Holding BARE ⌘ (Super alone, nothing else) for a beat ([`HOLD_PEEK_MS`]) summons
//! a calm centered card of shortcuts the user actually reaches for but hasn't learned
//! yet — the discoverability round's "surfaced ONLY where the user chooses to look,
//! never a nudge" law applied to the one gesture every macOS user already makes when
//! they're hunting for a shortcut (holding ⌘ and staring at a menu). It reuses the
//! held stats HUD's exact float-card pipeline (`render/chrome/hud.rs`), dismissing the
//! instant the hold breaks — a true HOLD, like the HUD, NOT a modal like the About /
//! Lifetime cards.
//!
//! **CANCELLATION IS THE CRUX.** The peek must NEVER flicker in front of a real chord:
//! ⌘S, ⌘⇧P, a ⌘-click (Follow link), the window blurring — any of these instantly
//! kills a pending peek and closes an open one. The pure [`PeekArm`] state machine
//! ([`PeekArm::next`]) models exactly that, WITHOUT a clock or winit, so every
//! cancellation path is unit-testable; the live `App` feeds it stimuli and consults
//! `peek_armed_at` for the single `WaitUntil` deadline (the same idle-safe timer
//! pattern the which-key pause uses — no hot loop).
//!
//! **Determinism:** the peek is a HELD, clocked, live-only surface exactly like the
//! stats HUD. The live App pushes the personalized [`PeekRow`]s (its ledger's
//! graduation candidates) into the pipeline every `sync_view`; a headless capture
//! never does, so the pipeline's rows stay empty and the pure [`starter_rows`]
//! fallback renders — the `--peek` capture flag summons the SETTLED card showing the
//! curated STARTER SIX, byte-stable across machines. A default capture (not summoned)
//! draws nothing and is byte-identical.

use std::sync::atomic::{AtomicBool, Ordering};

/// How long BARE ⌘ must be held — alone and uninterrupted — before the shortcut peek
/// summons. A TASTE constant (flagged for live tuning, named like `THEME_FONT_DEBOUNCE`
/// / `HOLD_PEEK_MS`): ~600ms is long enough that a fast ⌘-chord (⌘S, ⌘⇧P) is always a
/// press-and-release well under it (never flickering the card), short enough that a
/// deliberate "what were the shortcuts again?" pause lands it promptly. LIVE-ONLY feel.
pub const HOLD_PEEK_MS: u64 = 600;

/// How many personalized shortcut rows the peek card shows at most — the top-N slow-door
/// commands the user keeps reaching but has a chord for (the ledger's graduation
/// ranking). ~6 keeps the card a calm glance, not a dashboard.
pub const PEEK_ROWS: usize = 6;

/// Whether the shortcut-peek card is drawn. DEFAULT OFF: the calm room shows no card
/// until BARE ⌘ is held past [`HOLD_PEEK_MS`] (the live gesture) or the `--peek`
/// capture flag forces it.
static PEEK_OPEN: AtomicBool = AtomicBool::new(false);

/// True when the shortcut-peek card is currently summoned.
pub fn peek_open() -> bool {
    PEEK_OPEN.load(Ordering::Relaxed)
}

/// Open or close the card explicitly. The live App calls this from [`PeekArm`]'s
/// transitions (open on the hold completing, close on any cancellation); the `--peek`
/// flag passes `true` for a settled capture.
pub fn set_open(open: bool) {
    PEEK_OPEN.store(open, Ordering::Relaxed);
}

/// ONE peek row: a chord GLYPH figure (`"⌘O"`) over/beside its command NAME caption
/// (`"Go to file"`). The type system's ink × size — the chord rides content ink at
/// BODY size (the figure), the name faint ink at LABEL size (the caption) — NEVER amber
/// (the caret's alone). Owned strings so the App can hand personalized rows across the
/// pipeline seam by value, exactly like `HudStats`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeekRow {
    /// The macOS modifier-glyph chord, e.g. `"⌘O"` (`keyspec::mac_glyph_chord`).
    pub chord: String,
    /// The command's display name, e.g. `"Go to file"` (ellipsis stripped).
    pub name: String,
}

/// The curated STARTER SIX — the shortcuts a fresh install shows before the ledger has
/// learned anything, so the card is NEVER empty. The four IDENTITY-ROUND doors
/// (⌘O go-to-file, ⌘⇧P switch-project, ⌘P command-palette, ⌘F find) plus ⌘S save and
/// ⌘T switch-theme: the six a new user most wants in their fingers. Kept as
/// (chord-spec, name) pairs glyphified through the SAME `keyspec::mac_glyph_chord` the
/// personalized rows use, so a starter row and a learned row render identically.
const STARTER: &[(&str, &str)] = &[
    ("Cmd-O", "Go to file"),
    ("Cmd-S-p", "Switch project"),
    ("Cmd-P", "Command palette"),
    ("Cmd-F", "Find"),
    ("Cmd-S", "Save"),
    ("Cmd-T", "Switch theme"),
];

/// The curated STARTER SIX as [`PeekRow`]s (chords glyphified). The pure fallback shown
/// when the ledger has no personalized rows yet — on a fresh install (live) AND in a
/// headless `--peek` capture (no live ledger), so the two agree byte-for-byte.
pub fn starter_rows() -> Vec<PeekRow> {
    STARTER
        .iter()
        .map(|(spec, name)| PeekRow {
            chord: crate::keyspec::mac_glyph_chord(spec),
            name: name.to_string(),
        })
        .collect()
}

/// The rows the peek card actually renders: the App-pushed personalized `rows` when it
/// has any, else the curated [`starter_rows`]. ONE owner of the empty→starter fallback,
/// shared by the pixels (`render/chrome/hud.rs`) and the sidecar (`capture/sidecar.rs`)
/// so they can never disagree. An empty slice (a capture never pushed, or a fresh-install
/// ledger with no candidates) folds to the starter six.
pub fn rows_or_starter(rows: &[PeekRow]) -> Vec<PeekRow> {
    if rows.is_empty() {
        starter_rows()
    } else {
        rows.to_vec()
    }
}

/// The pure hold-⌘ peek ARM state — the "hold bare ⌘ for a beat" gesture and every
/// cancellation, modeled WITHOUT a clock or winit. The live App holds one of these +
/// the arm `Instant`; it feeds [`PeekStimulus`]es through [`Self::next`] and stamps the
/// deadline on the `Idle → Pending` edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PeekArm {
    /// Nothing pending, no card shown — the idle default.
    #[default]
    Idle,
    /// BARE ⌘ went down alone; the hold timer is running. When it elapses (still
    /// `Pending`), the card summons.
    Pending,
    /// The card is summoned (the hold completed) and stays up until the hold breaks.
    Open,
}

/// A stimulus fed to the [`PeekArm`] machine. The live App maps its raw input events to
/// these: `ModifiersChanged` → [`SuperAlone`]/[`SuperBroken`], a key press past the
/// lone-modifier filter → [`KeyJoined`], a mouse press / focus loss → [`Interrupt`], and
/// the hold-timer deadline → [`Elapsed`].
///
/// [`SuperAlone`]: PeekStimulus::SuperAlone
/// [`SuperBroken`]: PeekStimulus::SuperBroken
/// [`KeyJoined`]: PeekStimulus::KeyJoined
/// [`Interrupt`]: PeekStimulus::Interrupt
/// [`Elapsed`]: PeekStimulus::Elapsed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeekStimulus {
    /// Modifiers became EXACTLY bare ⌘ (Super alone) — the only state that arms.
    SuperAlone,
    /// Modifiers are no longer bare ⌘ (⌘+another modifier, or ⌘ released) — the hold
    /// chord broke, so a pending peek cancels and an open one closes.
    SuperBroken,
    /// A non-modifier key press joined ⌘ (⌘S, ⌘⇧P's letter, Cmd-I, …) — a real chord is
    /// forming, so the peek cancels/closes instantly. THE CRUX: this is what keeps ⌘S
    /// from ever flickering the card.
    KeyJoined,
    /// An external interruption — a mouse press (a ⌘-click is Follow link) or the window
    /// losing focus — cancels/closes.
    Interrupt,
    /// The hold-timer deadline fired while still `Pending`: summon the card.
    Elapsed,
}

impl PeekArm {
    /// The pure transition: fold `stim` into this state, returning the next one. No
    /// clock, no side effects — the live App reads the RESULT to drive the process-
    /// global + the `WaitUntil` deadline (see `App::feed_peek`).
    ///
    /// The table, in one breath: bare ⌘ alone ARMS from idle and holds a pending/open
    /// peek; the timer OPENS a pending one; a joined key, a broken modifier, or any
    /// interruption returns to idle from anywhere (cancel pending, close open). A stray
    /// `Elapsed` while already idle/open is inert (it can only meaningfully fire while
    /// pending).
    pub fn next(self, stim: PeekStimulus) -> PeekArm {
        use PeekArm::*;
        use PeekStimulus::*;
        match (self, stim) {
            // Idle: only a bare-⌘ press arms; everything else stays idle.
            (Idle, SuperAlone) => Pending,
            (Idle, _) => Idle,
            // Pending: the timer opens it; a re-affirmed bare ⌘ holds; any break cancels.
            (Pending, Elapsed) => Open,
            (Pending, SuperAlone) => Pending,
            (Pending, SuperBroken | KeyJoined | Interrupt) => Idle,
            // Open: stays open while ⌘ is re-affirmed / the timer re-fires; any break
            // closes it.
            (Open, SuperAlone | Elapsed) => Open,
            (Open, SuperBroken | KeyJoined | Interrupt) => Idle,
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use PeekArm::*;
    use PeekStimulus::*;

    #[test]
    fn defaults_closed() {
        let _g = crate::testlock::serial();
        set_open(false);
        assert!(!peek_open(), "the shortcut-peek card is closed by default");
    }

    #[test]
    fn set_open_drives_the_flag() {
        let _g = crate::testlock::serial();
        set_open(false);
        set_open(true);
        assert!(peek_open());
        set_open(false);
        assert!(!peek_open());
    }

    #[test]
    fn bare_super_arms_then_timer_opens() {
        // The happy path: ⌘ alone arms Pending, the hold timer opens the card.
        assert_eq!(Idle.next(SuperAlone), Pending);
        assert_eq!(Pending.next(Elapsed), Open);
        // A re-affirmed bare ⌘ (an OS-repeated ModifiersChanged) holds each state.
        assert_eq!(Pending.next(SuperAlone), Pending);
        assert_eq!(Open.next(SuperAlone), Open);
        assert_eq!(Open.next(Elapsed), Open);
    }

    #[test]
    fn a_second_key_kills_a_pending_peek() {
        // THE CRUX: ⌘ down (Pending), then S joins → the peek is cancelled before it can
        // ever flicker. ⌘S must feel like a plain save.
        let armed = Idle.next(SuperAlone);
        assert_eq!(armed, Pending);
        assert_eq!(armed.next(KeyJoined), Idle, "a joined key kills the pending peek");
    }

    #[test]
    fn releasing_super_before_the_threshold_shows_nothing() {
        // Press ⌘ then release it before the hold elapses: Pending → Idle, the card
        // never opened.
        let armed = Idle.next(SuperAlone);
        assert_eq!(armed.next(SuperBroken), Idle, "release before threshold = nothing");
    }

    #[test]
    fn a_click_or_blur_cancels_pending_and_closes_open() {
        // A ⌘-click (Follow link) or the window blurring interrupts either state.
        assert_eq!(Pending.next(Interrupt), Idle);
        assert_eq!(Open.next(Interrupt), Idle);
    }

    #[test]
    fn an_open_peek_closes_when_super_lifts() {
        // Holding past the threshold opens the card; lifting ⌘ closes it (a true hold).
        assert_eq!(Open.next(SuperBroken), Idle);
        // A key pressed while it's open (e.g. finally hitting the shortcut) closes it too.
        assert_eq!(Open.next(KeyJoined), Idle);
    }

    #[test]
    fn a_stray_elapsed_while_idle_is_inert() {
        // The timer can only meaningfully fire while Pending; a late/stray Elapsed in
        // any other state changes nothing.
        assert_eq!(Idle.next(Elapsed), Idle);
        assert_eq!(Open.next(Elapsed), Open);
    }

    #[test]
    fn starter_rows_are_the_curated_six_glyphified() {
        let rows = starter_rows();
        assert_eq!(rows.len(), 6, "the curated starter six");
        assert_eq!(rows[0], PeekRow { chord: "⌘O".into(), name: "Go to file".into() });
        assert_eq!(rows[1], PeekRow { chord: "⌘⇧P".into(), name: "Switch project".into() });
        assert_eq!(rows[2].chord, "⌘P");
        assert_eq!(rows[3], PeekRow { chord: "⌘F".into(), name: "Find".into() });
        assert_eq!(rows[4], PeekRow { chord: "⌘S".into(), name: "Save".into() });
        assert_eq!(rows[5], PeekRow { chord: "⌘T".into(), name: "Switch theme".into() });
    }

    #[test]
    fn rows_or_starter_folds_empty_to_the_starter_six() {
        // A capture (never pushed) / fresh-install ledger (no candidates) → starter six.
        assert_eq!(rows_or_starter(&[]), starter_rows());
        // A non-empty push wins verbatim.
        let learned = vec![PeekRow { chord: "⌘;".into(), name: "Spell suggestions".into() }];
        assert_eq!(rows_or_starter(&learned), learned);
    }
}
