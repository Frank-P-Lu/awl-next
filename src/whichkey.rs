//! WHICH-KEY — the summoned, transient "what can follow this prefix?" panel.
//!
//! awl has ONE multi-key prefix, `C-x` (see `keymap.rs`): press it, then a second
//! key resolves a command. The identity round RETIRED the C-x DEFAULTS (native
//! chords are the advertised keymap now), so the prefix ships EMPTY — but the
//! machinery stays, and a `[keys]` "C-x <key>" line reclaims any chord. Which-key
//! makes whatever C-x continuations exist DISCOVERABLE the calm way: press the prefix,
//! PAUSE (~500ms), and a small panel is SUMMONED listing every follow-up key and what
//! it does. It is summoned + transient — it appears only on the pause and vanishes the
//! instant the chord completes or aborts (DESIGN §5: summoned, not furniture). It
//! TEACHES the keys (informational, button-free — you still press the key), and its
//! hints ride the MUTED/FAINT ink, never amber (DESIGN §3: `primary` is the caret's
//! alone).
//!
//! DERIVED, NOT DUPLICATED. The continuation list is derived from the command CATALOG
//! (`commands.rs`), the source of truth for "what does `C-x <key>` do" — every catalog
//! command carries its effective bindings (config overrides folded in), so
//! [`continuations`] filters the ones that START with the prefix and reads off the
//! `(follow-up key, command name)` pairs. The keymap's static `C-x` arms all map to a
//! catalog command, so this can't drift. The prefix is a parameter, so a future second
//! prefix is supported generically.
//!
//! The pause TIMER + summon/dismiss STATE live in `app.rs` (the pause deadline is armed
//! only while a prefix is pending — §6: idle stays 0% CPU, no perpetual per-frame tick);
//! the RENDER (a bottom-left float panel) lives in `render/chrome.rs`. This module is
//! the pure DERIVATION + the headless force-global, both unit-testable without a window.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// awl's ONE live prefix. Which-key is generic over the prefix (so a future second
/// prefix reuses [`continuations`]), but this is the one the app arms a timer for.
pub const PREFIX_CX: &str = "C-x";

/// The PAUSE before the panel is summoned: press the prefix, wait this long WITHOUT a
/// follow-up key, and the panel appears. Short enough to feel responsive to a learner,
/// long enough that a fluent `C-x C-s` never summons it. The `app.rs` timer arms a
/// single `WaitUntil` this far out and disarms once the panel shows or the prefix
/// resolves — no ongoing tick (DESIGN §6).
pub const PAUSE: Duration = Duration::from_millis(500);

/// One row of the which-key panel: the follow-up `key` (shown muted/faint, e.g.
/// `"C-s"`, `"t"`, `"}"`) and the `name` of the command it runs (e.g. `"Save"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Continuation {
    /// The SECOND-key label, exactly as the catalog spells it (terse emacs form).
    pub key: String,
    /// The command's display name from the catalog.
    pub name: String,
}

/// The `(follow-up key, command name)` rows for `prefix`, DERIVED from the command
/// catalog's effective bindings (`keys` = the config `[keys]` overrides). A command
/// contributes a row when one of its effective chords is `"<prefix> <key>"` (exactly
/// two tokens whose first canonicalises to `prefix`); the second token is the row's
/// key. Sorted by key so the panel order is stable + groups the `C-…` continuations
/// together. Pure — no window, no clock — so the whole derivation is unit-testable.
pub fn continuations(prefix: &str, keys: &[(String, Vec<String>)]) -> Vec<Continuation> {
    let want = match crate::keyspec::canonical_binding(prefix) {
        Some(p) => p,
        None => return Vec::new(),
    };
    let names = crate::commands::visible_names();
    let chord_lists = crate::commands::visible_effective_chord_lists(keys);
    let mut rows: Vec<Continuation> = Vec::new();
    for (name, chords) in names.iter().zip(chord_lists.iter()) {
        for chord in chords {
            if let Some(key) = prefix_follow_up(&want, chord) {
                rows.push(Continuation { key, name: name.clone() });
            }
        }
    }
    // Stable, sensible order: by the key label. Uppercase `C` (0x43) sorts before the
    // lowercase plain letters (0x63), so the `C-…` chords cluster ahead of the bare
    // keys — a calm grouping without a hand-maintained order.
    rows.sort_by(|a, b| a.key.cmp(&b.key));
    rows
}

/// The which-key rows for the main `C-x` prefix (the common call site).
pub fn continuations_cx(keys: &[(String, Vec<String>)]) -> Vec<Continuation> {
    continuations(PREFIX_CX, keys)
}

/// If `chord` is a two-token sequence whose FIRST token canonicalises to `want`
/// (the already-canonicalised prefix), return the raw SECOND token — the panel's
/// follow-up key; else `None`. The raw token is kept as the display key (the
/// catalog's own terse spelling, e.g. `C-s` / `t` / `}`).
fn prefix_follow_up(want: &str, chord: &str) -> Option<String> {
    let toks: Vec<&str> = chord.split_whitespace().collect();
    if toks.len() != 2 {
        return None;
    }
    let head = crate::keyspec::canonical_binding(toks[0])?;
    (head == *want).then(|| toks[1].to_string())
}

/// The transition the App applies after a key resolves, based on the keymap's
/// post-resolve prefix state. Pure so the state machine is unit-testable without a
/// window (see [`on_key`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixTransition {
    /// A prefix (`C-x`) is now pending its second key: (re)ARM the pause timer.
    Arm,
    /// The prefix just resolved to a command or aborted (`Esc` / `C-g`): PUT the panel
    /// down + disarm the timer — summoned + transient, it never lingers past the chord.
    Dismiss,
    /// A plain key with no prefix in play: nothing to do (the common, cheap case).
    Ignore,
}

/// Decide the which-key transition after a key resolves. `in_prefix` is the keymap's
/// prefix state AFTER resolving the key; `pending`/`shown` are the panel's current
/// state. Entering a prefix ARMS; leaving one while pending-or-shown DISMISSES;
/// otherwise IGNORE. Pure — the whole prefix→panel state machine in one function.
pub fn on_key(in_prefix: bool, pending: bool, shown: bool) -> PrefixTransition {
    if in_prefix {
        PrefixTransition::Arm
    } else if pending || shown {
        PrefixTransition::Dismiss
    } else {
        PrefixTransition::Ignore
    }
}

/// Should the panel be SUMMONED now? True only while a prefix is `pending`, the panel
/// is not already `shown`, and the pause deadline has `elapsed`. So the timer summons
/// exactly once per pending prefix and only after the pause — the App feeds
/// `elapsed = now >= armed_at + PAUSE`. Pure, so the timer gate is unit-testable.
pub fn should_summon(pending: bool, shown: bool, elapsed: bool) -> bool {
    pending && !shown && elapsed
}

/// Process-global forcing the which-key panel SHOWN for a headless capture — the
/// `--whichkey` flag's door, mirroring the `--fps` / `--hud` globals. The live window
/// never sets this (it summons the panel via the real pause timer in `app.rs`); a
/// capture sets it so `--whichkey --screenshot` renders the SETTLED summoned panel
/// deterministically, while a default capture (unset) draws nothing and stays
/// byte-identical.
static FORCE_SHOWN: AtomicBool = AtomicBool::new(false);

/// Force the which-key panel shown (or not) for the headless capture path.
pub fn set_force_shown(on: bool) {
    FORCE_SHOWN.store(on, Ordering::Relaxed);
}

/// Is the which-key panel being FORCED shown for a capture (`--whichkey`)?
pub fn force_shown() -> bool {
    FORCE_SHOWN.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The C-x continuations derive from the catalog's EFFECTIVE bindings. Since the
    /// identity round retired every C-x default, the DEFAULT (no-config) list is now
    /// EMPTY — the panel only teaches C-x chords a user has RECLAIMED via `[keys]`.
    #[test]
    fn cx_continuations_are_empty_by_default_and_reflect_reclaimed_chords() {
        assert!(continuations_cx(&[]).is_empty(), "no C-x defaults remain to teach");
        // A user reclaims a few C-x sequences; the panel teaches exactly those.
        let keys = vec![
            ("save".to_string(), vec!["C-x C-s".to_string()]),
            ("switch_theme".to_string(), vec!["C-x t".to_string()]),
            ("new_note".to_string(), vec!["C-x n".to_string()]),
        ];
        let rows = continuations_cx(&keys);
        let has = |key: &str, name: &str| rows.iter().any(|r| r.key == key && r.name == name);
        assert!(has("C-s", "Save"));
        assert!(has("t", "Switch theme…"));
        assert!(has("n", "New note"));
    }

    /// Only `C-x …` bindings become rows — a native-only / non-prefixed command
    /// (Zoom in = `Cmd-=`; Search forward = `Cmd-F` / `C-s`) never leaks in, even
    /// among reclaimed C-x chords.
    #[test]
    fn non_prefix_bindings_excluded() {
        let keys = vec![("save".to_string(), vec!["C-x C-s".to_string()])];
        let rows = continuations_cx(&keys);
        assert!(!rows.iter().any(|r| r.name == "Zoom in"));
        assert!(!rows.iter().any(|r| r.name == "Search forward"));
        // Settings… carries Cmd-, (P1) but no C-x continuation — never a row here.
        assert!(!rows.iter().any(|r| r.name == "Settings…"));
    }

    /// The rows are sorted by key (deterministic), and the `C-…` chords group ahead
    /// of the bare-letter continuations (over a set of RECLAIMED C-x chords).
    #[test]
    fn rows_sorted_and_grouped() {
        let keys = vec![
            ("save".to_string(), vec!["C-x C-s".to_string()]),
            ("switch_theme".to_string(), vec!["C-x t".to_string()]),
            ("new_note".to_string(), vec!["C-x n".to_string()]),
        ];
        let rows = continuations_cx(&keys);
        let ks: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
        let mut sorted = ks.clone();
        sorted.sort();
        assert_eq!(ks, sorted, "rows must be sorted by key");
        // A `C-…` chord precedes any bare lowercase letter (uppercase C sorts first).
        let first_ctrl = ks.iter().position(|k| k.starts_with("C-")).unwrap();
        let first_letter = ks.iter().position(|k| *k == "t").unwrap();
        assert!(first_ctrl < first_letter);
    }

    /// A config `[keys]` rebind of a catalog command onto a `C-x <key>` chord flows
    /// through — the panel reflects the EFFECTIVE binding, not just the static default.
    #[test]
    fn config_override_reflected() {
        // Rebind "Switch theme…" from `C-x t` to `C-x g`; the panel should show `g`.
        let keys = vec![("switch_theme".to_string(), vec!["C-x g".to_string()])];
        let rows = continuations(PREFIX_CX, &keys);
        assert!(rows.iter().any(|r| r.key == "g" && r.name == "Switch theme…"));
        // The old default `t` for Switch theme… is gone (the override replaced it).
        assert!(!rows.iter().any(|r| r.key == "t" && r.name == "Switch theme…"));
    }

    /// An unknown / unparseable prefix yields no rows (never a panic).
    #[test]
    fn bad_prefix_is_empty() {
        assert!(continuations("C-frobnicate", &[]).is_empty());
    }

    /// The force-global round-trips (the `--whichkey` capture door).
    #[test]
    fn force_shown_round_trips() {
        set_force_shown(true);
        assert!(force_shown());
        set_force_shown(false);
        assert!(!force_shown());
    }

    /// STATE MACHINE: pressing the prefix (keymap now in-prefix) ARMS the pause timer,
    /// whatever the prior panel state.
    #[test]
    fn entering_prefix_arms() {
        // From idle.
        assert_eq!(on_key(true, false, false), PrefixTransition::Arm);
        // A second `C-x` while pending re-arms (resets the pause).
        assert_eq!(on_key(true, true, false), PrefixTransition::Arm);
    }

    /// STATE MACHINE: a FOLLOW-UP key that resolves the command (keymap no longer
    /// in-prefix) DISMISSES the panel — whether it had only been armed or was shown.
    #[test]
    fn follow_up_key_dismisses() {
        // Resolve while armed-but-not-yet-shown.
        assert_eq!(on_key(false, true, false), PrefixTransition::Dismiss);
        // Resolve while the panel was already shown (Esc / C-g abort lands here too —
        // both leave the keymap NOT in-prefix).
        assert_eq!(on_key(false, true, true), PrefixTransition::Dismiss);
        assert_eq!(on_key(false, false, true), PrefixTransition::Dismiss);
    }

    /// STATE MACHINE: a plain key with no prefix in play is a cheap no-op (the common
    /// case) — no summon, no dismiss.
    #[test]
    fn plain_key_is_ignored() {
        assert_eq!(on_key(false, false, false), PrefixTransition::Ignore);
    }

    /// TIMER GATE: the panel summons EXACTLY when a prefix is pending, it is not
    /// already shown, and the pause has elapsed — and never otherwise (so the timer
    /// arms only while pending and fires once).
    #[test]
    fn summon_gate() {
        assert!(should_summon(true, false, true), "pending + not shown + elapsed → summon");
        assert!(!should_summon(true, false, false), "pause not yet elapsed → wait");
        assert!(!should_summon(true, true, true), "already shown → no re-summon");
        assert!(!should_summon(false, false, true), "no prefix pending → nothing");
    }
}
