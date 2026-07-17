//! THE FORMAT POPOVER — the pure PLAN: read the buffer's text + selection and
//! decide each button's lit/label, producing the render-facing
//! [`crate::popover::PopoverModel`]. It sits BESIDE `format.rs` because it reads
//! that module's own active-state detection ([`format::inline_active`] /
//! [`format::heading_level`]) — the SAME predicates the toggles themselves use, so
//! a button can never light up out of step with what its Action would do.
//!
//! The button ROSTER + the catalog-Action mapping live in [`crate::popover`]
//! (pure data, no format dependency); this module only fills in the per-frame
//! `active`/`label` from the live selection.

use super::format::{self, InlineKind};
use crate::popover::{ButtonState, PopoverButton, PopoverModel, ALL};

/// The [`InlineKind`] a button's active-state reads through, or `None` for the two
/// non-inline buttons (`Heading` reads [`format::heading_level`], `Link` reads
/// [`crate::markdown::link_at_full`]).
fn inline_kind(b: PopoverButton) -> Option<InlineKind> {
    match b {
        PopoverButton::Bold => Some(InlineKind::Bold),
        PopoverButton::Italic => Some(InlineKind::Italic),
        PopoverButton::Highlight => Some(InlineKind::Highlight),
        PopoverButton::Code => Some(InlineKind::InlineCode),
        PopoverButton::Strike => Some(InlineKind::Strikethrough),
        PopoverButton::Heading | PopoverButton::Link => None,
    }
}

/// Byte offset of char index `cursor` into `text` (the coordinate
/// [`crate::markdown::link_at_full`] works in).
fn char_to_byte(text: &str, cursor: usize) -> usize {
    text.char_indices().nth(cursor).map(|(b, _)| b).unwrap_or(text.len())
}

/// Build the popover's render model for the current selection state, or `None`
/// when it should not show (a non-markdown buffer — the format toggles are
/// markdown-only, so a popover of no-ops there would be a lie). The `summon`
/// decision (a mouse gesture opened it, a selection is present, the feature is on)
/// is the CALLER's; this only decides button lit/label.
///
/// PURE — the single verification seam the spec asks for (unit-tested below): the
/// same `(text, anchor, cursor)` always yields the same model.
pub(crate) fn plan(
    text: &str,
    anchor: Option<usize>,
    cursor: usize,
    is_markdown: bool,
) -> Option<PopoverModel> {
    if !is_markdown {
        return None;
    }
    let level = format::heading_level(text, anchor, cursor);
    let byte = char_to_byte(text, cursor);
    let in_link = crate::markdown::link_at_full(text, byte).is_some();

    let buttons = ALL
        .iter()
        .map(|&button| {
            let (active, label) = match button {
                PopoverButton::Heading => (
                    level > 0,
                    if level == 0 {
                        "H".to_string()
                    } else {
                        format!("H{level}")
                    },
                ),
                PopoverButton::Link => (in_link, button.base_label().to_string()),
                b => (
                    inline_kind(b)
                        .map(|k| format::inline_active(k, text, anchor, cursor))
                        .unwrap_or(false),
                    button.base_label().to_string(),
                ),
            };
            ButtonState { button, active, label }
        })
        .collect();

    Some(PopoverModel { buttons })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(m: &PopoverModel) -> Vec<String> {
        m.buttons.iter().map(|b| b.label.clone()).collect()
    }
    fn active(m: &PopoverModel, button: PopoverButton) -> bool {
        m.buttons.iter().find(|b| b.button == button).unwrap().active
    }

    #[test]
    fn non_markdown_never_summons() {
        assert!(plan("plain text", Some(0), 5, false).is_none());
    }

    #[test]
    fn plain_selection_lights_nothing_and_labels_h() {
        // "the quick fox", select "quick" (chars 4..9), unformatted.
        let m = plan("the quick fox", Some(4), 9, true).unwrap();
        assert_eq!(labels(&m), vec!["B", "I", "==", "`", "~~", "H", "Link"]);
        for b in &m.buttons {
            assert!(!b.active, "{:?} should be unlit on plain text", b.button);
        }
    }

    #[test]
    fn bold_selection_lights_the_b_button() {
        // "the **quick** fox": select the inner "quick" (chars 6..11). B lights.
        // (I is DELIBERATELY not asserted here: `**` contains a single `*`, so the
        // italic toggle would strip it too — the popover reflects that toggle
        // behavior verbatim, so both can legitimately light. `==` has no such
        // overlap, so it is the clean negative.)
        let m = plan("the **quick** fox", Some(6), 11, true).unwrap();
        assert!(active(&m, PopoverButton::Bold), "B lit inside **…**");
        assert!(!active(&m, PopoverButton::Highlight), "== unlit on bold text");
    }

    #[test]
    fn heading_button_reflects_the_level_and_lights() {
        let off = plan("Title\n", Some(0), 3, true).unwrap();
        assert_eq!(off.buttons.last().is_some(), true);
        let h = off.buttons.iter().find(|b| b.button == PopoverButton::Heading).unwrap();
        assert_eq!(h.label, "H");
        assert!(!h.active);

        let h2 = plan("## Sec\n", Some(4), 4, true).unwrap();
        let hb = h2.buttons.iter().find(|b| b.button == PopoverButton::Heading).unwrap();
        assert_eq!(hb.label, "H2", "H button shows the current level");
        assert!(hb.active);
    }

    #[test]
    fn link_button_lights_when_the_caret_sits_in_a_link() {
        // "see [awl](https://awl.dev) now" — caret inside the link text.
        let text = "see [awl](https://awl.dev) now";
        let caret = 6; // inside "awl"
        let m = plan(text, None, caret, true).unwrap();
        assert!(active(&m, PopoverButton::Link), "Link lit inside a link");
        // Caret out on plain text → unlit.
        let m2 = plan(text, None, 1, true).unwrap();
        assert!(!active(&m2, PopoverButton::Link));
    }
}
