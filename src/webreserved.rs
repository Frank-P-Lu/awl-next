//! THE WEB CHORD SANITY ROUND, Tier 2 — the small set of modifier chords a
//! BROWSER PAGE can never intercept, on either keyboard convention. Chrome/
//! Firefox/Safari all handle these at the browser-chrome layer BEFORE a
//! `keydown` ever reaches the page's JS — `event.preventDefault()` is a no-op
//! against them (unlike, say, Ctrl-S's "Save Page" dialog or Ctrl-F's find
//! bar, both of which ARE preventable once the canvas has focus — see Tier 1,
//! `app.rs`'s canvas-focus wiring). This module is DATA, not a code path: one
//! small table per [`Convention`], plus the ONE pure membership test every
//! label surface (`commands::join_slots_resolved`, `menu::item_chord`) routes
//! through — so a command whose resolved native chord is reserved never
//! CLAIMS a chord the browser will actually eat.
//!
//! v1 SCOPE (logged, not attempted): no replacement chord is invented for an
//! affected command (New note / Switch theme…) — a truthful label (falling
//! back to slot 2 emacs, or to no chord shown) is this round's whole answer.
//! Picking + shipping a new default binding for these is a v2 taste call.

use crate::convention::Convention;

/// One browser-reserved accelerator: its chord SPEC in the same terse form
/// [`crate::keyspec::parse_chord`] accepts, plus a short human `reason` (docs +
/// this module's own tests only — never rendered to the user, so a plain
/// `cargo build` never reads it; `#[allow(dead_code)]` mirrors the codebase's
/// existing single-field precedent, justified by
/// `tests::every_table_entry_round_trips_through_its_own_canonical_form`
/// exercising it on every entry).
pub struct Reserved {
    pub spec: &'static str,
    #[allow(dead_code)]
    pub reason: &'static str,
}

/// Reserved on a Mac-flavored browser (Safari/Chrome/Firefox on macOS).
pub const MAC_WEB_RESERVED: &[Reserved] = &[
    Reserved { spec: "Cmd-N", reason: "new window" },
    Reserved { spec: "Cmd-S-N", reason: "new private window" },
    Reserved { spec: "Cmd-T", reason: "new tab" },
    Reserved { spec: "Cmd-S-T", reason: "reopen closed tab" },
    Reserved { spec: "Cmd-W", reason: "close tab" },
    Reserved { spec: "Cmd-S-W", reason: "close window" },
    Reserved { spec: "Cmd-Q", reason: "quit browser" },
];

/// Reserved on a Linux/Windows-flavored browser (the [`Convention::Linux`]
/// reading — GTK/Chrome/Firefox on non-Mac desktops). No Ctrl-Q entry: unlike
/// Cmd-Q on macOS, Ctrl-Q is not a universal non-Mac browser-quit convention
/// (Chrome/Firefox on Linux/Windows do not reserve it), so it is left off this
/// table deliberately, not by oversight.
pub const LINUX_WEB_RESERVED: &[Reserved] = &[
    Reserved { spec: "C-n", reason: "new window" },
    Reserved { spec: "C-S-n", reason: "new private window" },
    Reserved { spec: "C-t", reason: "new tab" },
    Reserved { spec: "C-S-t", reason: "reopen closed tab" },
    Reserved { spec: "C-w", reason: "close tab" },
    Reserved { spec: "C-S-w", reason: "close window" },
];

/// The reserved table for `convention` — the ONE data owner every check below
/// reads (no separate list ever forks off this one).
pub fn reserved_for(convention: Convention) -> &'static [Reserved] {
    match convention {
        Convention::Mac => MAC_WEB_RESERVED,
        Convention::Linux => LINUX_WEB_RESERVED,
    }
}

/// Is `chord_spec` (an already CONVENTION-RESOLVED native chord, e.g.
/// `"Cmd-N"` on Mac or `"C-n"` on Linux — see `commands::resolved_native`) a
/// browser-reserved accelerator under `convention`? Compares CANONICAL forms
/// (`keyspec::canonical_binding`) so case/modifier-order differences never
/// cause a false negative; an unparsable `chord_spec` is never reserved
/// (matches [`crate::keyspec::parse_chord`]'s own tolerant-passthrough
/// philosophy — never panics, never over-claims). Pure; callers decide
/// WHETHER to even ask (only `Platform::Web` cares — a native build's chords
/// are never browser-shadowed).
pub fn is_reserved(chord_spec: &str, convention: Convention) -> bool {
    let Some(want) = crate::keyspec::canonical_binding(chord_spec) else {
        return false;
    };
    reserved_for(convention)
        .iter()
        .any(|r| crate::keyspec::canonical_binding(r.spec).as_deref() == Some(want.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_reserved_catches_new_note_and_switch_theme() {
        // The two catalog commands this round's own bug report names as
        // affected: New note (Cmd-N) and Switch theme… (Cmd-T).
        assert!(is_reserved("Cmd-N", Convention::Mac));
        assert!(is_reserved("Cmd-T", Convention::Mac));
        assert!(is_reserved("Cmd-Q", Convention::Mac));
    }

    #[test]
    fn linux_reserved_catches_the_ctrl_translated_forms() {
        assert!(is_reserved("C-n", Convention::Linux));
        assert!(is_reserved("C-t", Convention::Linux));
        // Ctrl-Q is deliberately NOT reserved on the Linux convention.
        assert!(!is_reserved("C-q", Convention::Linux));
    }

    #[test]
    fn case_and_word_form_do_not_matter() {
        assert!(is_reserved("cmd-n", Convention::Mac));
        assert!(is_reserved("Ctrl-N", Convention::Linux));
    }

    #[test]
    fn ordinary_chords_are_never_reserved() {
        for spec in ["Cmd-S", "Cmd-F", "Cmd-B", "C-s", "C-f", ""] {
            assert!(!is_reserved(spec, Convention::Mac), "{spec:?} on Mac");
            assert!(!is_reserved(spec, Convention::Linux), "{spec:?} on Linux");
        }
    }

    #[test]
    fn every_table_entry_round_trips_through_its_own_canonical_form() {
        // Structural sanity: every spec in both tables must itself PARSE (a
        // typo'd chord spec would silently make `is_reserved` always false
        // for that entry — this catches that class of bug at test time).
        for r in MAC_WEB_RESERVED.iter().chain(LINUX_WEB_RESERVED) {
            assert!(
                crate::keyspec::canonical_binding(r.spec).is_some(),
                "reserved spec {:?} ({}) fails to parse",
                r.spec,
                r.reason
            );
        }
    }
}
