//! THE FORMAT POPOVER — the reveal-on-select floating micro-toolbar (taste-
//! exception #3, the DESIGN.md "never a floating format bar" reversal: the deep
//! rule "summoned, not furniture" still holds because the SELECTION GESTURE is
//! the summons). A mouse selection (drag-release or double-click word-select) in
//! a markdown buffer floats a small row of format buttons over the selection:
//!
//!   B · I · == · ` · ~~ · H · Link
//!
//! Each button fires an EXISTING catalog [`Action`] through `App::apply` (the
//! menu-bar precedent — there is NO popover-only edit path; the law test
//! `buttons_fire_catalog_actions` enumerates the row no-wildcard). The row is
//! STATE-REFLECTIVE: an active toggle (the selection is already bold / italic /
//! …) draws lit, and the ONE `H` button CYCLES H1 → H2 → H3 → off, showing the
//! current level as its label.
//!
//! This module owns the pure DATA — the button roster, the sticky on/off global,
//! and the render-facing model [`PopoverModel`]. The pure PLAN that reads the
//! selection state and decides each button's lit/label lives in
//! [`crate::actions::popover`] (it needs the format toggles' own detection
//! internals, so it sits beside them).

use crate::keymap::Action;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the format popover is active. DEFAULT ON — a mouse selection in a
/// markdown buffer floats the format row. OFF is a TOTAL no-op: no gesture ever
/// summons it, and a capture is byte-identical to a build without the feature.
/// Mirrors [`crate::markdown::wysiwyg_on`] exactly (a process-global read by the
/// live App's mouse path + the capture probe, set once at launch from the config
/// sticky pref, flipped live by the settings menu).
static POPOVER_ON: AtomicBool = AtomicBool::new(true);

/// True when the format popover is active (read by the live App's summon path +
/// the capture force-summon probe).
pub fn popover_on() -> bool {
    POPOVER_ON.load(Ordering::Relaxed)
}

/// Set the format popover on/off explicitly — the config sticky-pref launch-apply
/// (mirrors [`crate::markdown::set_wysiwyg_on`]).
pub fn set_popover_on(on: bool) {
    POPOVER_ON.store(on, Ordering::Relaxed);
}

/// The seven format buttons, LEFT-TO-RIGHT in the row. A no-wildcard enum: [`ALL`]
/// lists them in draw order and the plan / render / hit-test all iterate it, so a
/// new button lands in one place and the law test forces it a wired catalog
/// [`Action`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopoverButton {
    /// `**bold**` — [`Action::Bold`].
    Bold,
    /// `*italic*` — [`Action::Italic`].
    Italic,
    /// `==highlight==` — [`Action::Highlight`].
    Highlight,
    /// `` `code` `` — [`Action::InlineCode`].
    Code,
    /// `~~strike~~` — [`Action::Strikethrough`].
    Strike,
    /// The state-reflective HEADING cycler (H1 → H2 → H3 → off) —
    /// [`Action::HeadingCycle`].
    Heading,
    /// `[text](url)` — [`Action::InsertLink`] (`link::plan` decides wrap / edit /
    /// insert from the same selection state).
    Link,
}

/// THE ROSTER — every button, in row draw order. The plan, the renderer, the
/// hit-test and the law test all read THIS one list (no-wildcard), so the button
/// set has a single owner.
pub const ALL: &[PopoverButton] = &[
    PopoverButton::Bold,
    PopoverButton::Italic,
    PopoverButton::Highlight,
    PopoverButton::Code,
    PopoverButton::Strike,
    PopoverButton::Heading,
    PopoverButton::Link,
];

impl PopoverButton {
    /// The catalog [`Action`] this button fires through `App::apply` — the ONE
    /// structural law of the popover (no button has a private edit path). Every
    /// arm is an existing markdown-formatting Action.
    pub fn action(self) -> Action {
        match self {
            PopoverButton::Bold => Action::Bold,
            PopoverButton::Italic => Action::Italic,
            PopoverButton::Highlight => Action::Highlight,
            PopoverButton::Code => Action::InlineCode,
            PopoverButton::Strike => Action::Strikethrough,
            PopoverButton::Heading => Action::HeadingCycle,
            PopoverButton::Link => Action::InsertLink,
        }
    }

    /// The button's RESTING label (the `Heading` label is overridden per level by
    /// the plan — see [`crate::actions::popover::plan`]). Short glyphs echoing the
    /// markdown they insert.
    pub fn base_label(self) -> &'static str {
        match self {
            PopoverButton::Bold => "B",
            PopoverButton::Italic => "I",
            PopoverButton::Highlight => "==",
            PopoverButton::Code => "`",
            PopoverButton::Strike => "~~",
            PopoverButton::Heading => "H",
            PopoverButton::Link => "Link",
        }
    }
}

/// One button's render-facing state: which button, whether it draws LIT (the
/// selection already carries this format), and the label to draw (usually
/// [`PopoverButton::base_label`], but the `Heading` button shows `H1`/`H2`/`H3`
/// at its current level).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonState {
    pub button: PopoverButton,
    pub active: bool,
    pub label: String,
}

/// The whole popover's render model for one frame: the ordered button states. A
/// PURE function of the buffer's text + selection (built by
/// [`crate::actions::popover::plan`]), rebuilt each sync so the lit toggles track
/// the live document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PopoverModel {
    pub buttons: Vec<ButtonState>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_off_global_round_trips() {
        let _g = crate::testlock::serial();
        let saved = popover_on();
        set_popover_on(false);
        assert!(!popover_on());
        set_popover_on(true);
        assert!(popover_on());
        set_popover_on(saved);
    }

    #[test]
    fn every_button_maps_to_a_format_action() {
        // The structural law's pure half: each roster button fires a real catalog
        // Action (never a popover-private path). The catalog cross-check lives in
        // `commands.rs` (it needs COMMANDS); here we only assert the mapping exists
        // and is one of the markdown-formatting actions.
        for &b in ALL {
            let a = b.action();
            assert!(
                matches!(
                    a,
                    Action::Bold
                        | Action::Italic
                        | Action::Highlight
                        | Action::InlineCode
                        | Action::Strikethrough
                        | Action::HeadingCycle
                        | Action::InsertLink
                ),
                "{b:?} must fire a markdown-formatting catalog Action, got {a:?}"
            );
        }
    }

    #[test]
    fn roster_is_the_locked_seven_in_order() {
        let labels: Vec<&str> = ALL.iter().map(|b| b.base_label()).collect();
        assert_eq!(labels, vec!["B", "I", "==", "`", "~~", "H", "Link"]);
    }
}
