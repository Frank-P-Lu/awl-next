//! LINKS V2 — the pure logic behind Cmd-K (`Action::InsertLink`): deciding which
//! [`LinkEditMode`] a press lands in (from buffer state alone), and building the
//! final whole-buffer text once the URL minibuffer commits. Mirrors
//! `actions/format.rs`'s shape (pure transform in, [`FormatResult`]-style out,
//! applied as ONE atomic edit via `Buffer::apply_format`) — the same "same
//! behavior, same code" reasoning: an insert-link edit and a format toggle are
//! both "replace a byte range with a new wrapped/inserted string, then land the
//! cursor sensibly", so they share the apply primitive even though their pure
//! transforms live in separate, purpose-named modules (mirroring the block/inline
//! split within `format.rs` itself).

use super::format;
use super::*;
use crate::overlay::{LinkEditMode, OverlayState};

/// Char index of byte offset `byte` into `text` — the one conversion seam this
/// module needs (`markdown::link_at_full` returns DOCUMENT byte offsets, matching
/// pulldown's own coordinate space; `Buffer::apply_format`/`replace_char_range`
/// want CHAR indices). O(byte) but called at most once per Cmd-K press, never
/// per-frame.
fn byte_to_char(text: &str, byte: usize) -> usize {
    text.as_bytes()[..byte.min(text.len())]
        .iter()
        .filter(|&&b| (b & 0xC0) != 0x80) // count UTF-8 lead bytes only
        .count()
}

/// LINKS V2: decide the [`LinkEditMode`] + prefill URL for a fresh Cmd-K press,
/// purely from `text` + the selection/cursor CHAR state (no buffer mutation, so
/// this is safe to call speculatively). `kill_head` is the clipboard/kill-ring
/// head (`Buffer::kill_buffer`) — used as the prefill IFF it looks like a URL
/// ([`crate::buffer::is_url`]); the nice-touch "you probably want to paste this"
/// seed the task asked for, flagged as a taste call (a URL sitting in the kill
/// ring might be stale / not what the user means to link to — logged for live
/// review, not hidden).
///
/// Three cases, in priority order:
///   1. An ACTIVE SELECTION wraps that exact span: `[selection](url)`.
///   2. No selection, but the caret sits INSIDE an existing link
///      ([`crate::markdown::link_at_full`]): EDIT mode — re-prompt with that
///      link's own current URL (not the kill head — editing an existing link
///      should show what's there, not overwrite the prefill with something
///      unrelated), rewriting the same range on commit.
///   3. Neither: insert empty `[](url)` markup at the caret.
pub(super) fn plan(text: &str, anchor: Option<usize>, cursor: usize, kill_head: &str) -> (LinkEditMode, String) {
    let (s, e, has_sel) = crate::actions::format::sel_range(anchor, cursor);
    let url_prefill = || {
        if crate::buffer::is_url(kill_head) {
            kill_head.to_string()
        } else {
            String::new()
        }
    };
    if has_sel {
        let wrapped: String = text.chars().skip(s).take(e - s).collect();
        return (LinkEditMode::WithText { start: s, end: e, text: wrapped }, url_prefill());
    }
    let byte = text.char_indices().nth(cursor).map(|(b, _)| b).unwrap_or(text.len());
    if let Some(link) = crate::markdown::link_at_full(text, byte) {
        let start = byte_to_char(text, link.start);
        let end = byte_to_char(text, link.end);
        return (LinkEditMode::WithText { start, end, text: link.link_text }, link.url);
    }
    (LinkEditMode::Empty { at: cursor }, url_prefill())
}

/// LINKS V2 COMMIT: build the new whole-buffer text + cursor/anchor to restore,
/// given the (already-decided) `mode` and the typed `url`. Mirrors
/// `format::FormatResult`'s exact shape so the caller applies it the same way
/// (`Buffer::apply_format`) — an empty `url` is still applied verbatim (an empty
/// `[text]()`/`[]()"` is a harmless, correctable markdown oddity, not worth a
/// silent-cancel special case the user didn't ask for).
pub(super) fn commit(text: &str, mode: &LinkEditMode, url: &str) -> format::FormatResult {
    let chars: Vec<char> = text.chars().collect();
    match mode {
        LinkEditMode::WithText { start, end, text: inner } => {
            let start = (*start).min(chars.len());
            let end = (*end).min(chars.len()).max(start);
            let mut out = String::new();
            out.extend(&chars[..start]);
            out.push('[');
            out.push_str(inner);
            out.push_str("](");
            out.push_str(url);
            out.push(')');
            out.extend(&chars[end..]);
            let cursor = start + 1 + inner.chars().count() + 2 + url.chars().count() + 1;
            format::FormatResult { text: out, anchor: None, cursor }
        }
        LinkEditMode::Empty { at } => {
            let at = (*at).min(chars.len());
            let mut out = String::new();
            out.extend(&chars[..at]);
            out.push('[');
            out.push(']');
            out.push('(');
            out.push_str(url);
            out.push(')');
            out.extend(&chars[at..]);
            // Caret lands BETWEEN the brackets, ready to type the link text.
            format::FormatResult { text: out, anchor: None, cursor: at + 1 }
        }
    }
}

/// `Action::InsertLink` dispatch: markdown buffers only (a calm no-op elsewhere,
/// matching the formatting toggles' own availability honesty), summon the
/// minibuffer via [`plan`]. The actual edit happens on Enter, inside the modal
/// intercept (`actions/overlay_nav.rs`) — see [`commit`].
pub(super) fn open_insert_link(ctx: &mut ActionCtx) {
    if !ctx.buffer.is_markdown() {
        return;
    }
    let text = ctx.buffer.text();
    let anchor = ctx.buffer.anchor_char();
    let cursor = ctx.buffer.cursor_char();
    let kill = ctx.buffer.kill_buffer().to_string();
    let (mode, prefill) = plan(&text, anchor, cursor, &kill);
    *ctx.overlay = Some(OverlayState::new_link_edit(prefill, mode));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::overlay::OverlayKind;

    // --- plan(): mode + prefill decision ------------------------------------

    #[test]
    fn plan_with_selection_wraps_it() {
        let (mode, prefill) = plan("hello world", Some(0), 5, "");
        assert_eq!(
            mode,
            LinkEditMode::WithText { start: 0, end: 5, text: "hello".to_string() }
        );
        assert_eq!(prefill, "");
    }

    #[test]
    fn plan_with_selection_prefills_from_a_url_looking_kill_head() {
        let (_, prefill) = plan("hello world", Some(0), 5, "https://example.com");
        assert_eq!(prefill, "https://example.com");
    }

    #[test]
    fn plan_with_selection_ignores_a_non_url_kill_head() {
        let (_, prefill) = plan("hello world", Some(0), 5, "just some prose");
        assert_eq!(prefill, "");
    }

    #[test]
    fn plan_with_no_selection_and_no_link_inserts_empty() {
        let (mode, prefill) = plan("hello world", None, 5, "");
        assert_eq!(mode, LinkEditMode::Empty { at: 5 });
        assert_eq!(prefill, "");
    }

    #[test]
    fn plan_with_no_selection_and_no_link_prefills_from_url_kill_head() {
        let (mode, prefill) = plan("hello world", None, 5, "https://x.test/y");
        assert_eq!(mode, LinkEditMode::Empty { at: 5 });
        assert_eq!(prefill, "https://x.test/y");
    }

    #[test]
    fn plan_with_caret_inside_an_existing_link_is_edit_mode() {
        let text = "see [the text](https://old.example/path) here";
        // Caret inside "the text".
        let cursor = text.find("text").unwrap();
        let (mode, prefill) = plan(text, None, cursor, "https://irrelevant.kill.head");
        let start = text.find('[').unwrap();
        let end = text.find(')').unwrap() + 1;
        assert_eq!(
            mode,
            LinkEditMode::WithText { start, end, text: "the text".to_string() }
        );
        // Prefill is the EXISTING link's URL, never the kill head.
        assert_eq!(prefill, "https://old.example/path");
    }

    #[test]
    fn plan_with_caret_outside_any_link_is_not_edit_mode() {
        let text = "before [link](https://x.test) after";
        let cursor = 3; // inside "before", nowhere near the link
        let (mode, _) = plan(text, None, cursor, "");
        assert_eq!(mode, LinkEditMode::Empty { at: 3 });
    }

    // --- commit(): the pure text build ---------------------------------------

    #[test]
    fn commit_with_text_wraps_as_markdown_link() {
        let text = "hello world";
        let mode = LinkEditMode::WithText { start: 0, end: 5, text: "hello".to_string() };
        let r = commit(text, &mode, "https://example.com");
        assert_eq!(r.text, "[hello](https://example.com) world");
        assert_eq!(r.anchor, None);
        // Cursor lands right after the closing paren.
        assert_eq!(&r.text[..r.cursor], "[hello](https://example.com)");
    }

    #[test]
    fn commit_empty_inserts_brackets_with_caret_between_them() {
        let text = "hello world";
        let mode = LinkEditMode::Empty { at: 5 };
        let r = commit(text, &mode, "https://example.com");
        assert_eq!(r.text, "hello[](https://example.com) world");
        // Caret sits BETWEEN the brackets, ready to type the link text.
        assert_eq!(r.cursor, 6);
        assert_eq!(&r.text[5..7], "[]");
    }

    #[test]
    fn commit_edit_mode_rewrites_the_url_preserving_the_link_text() {
        let text = "see [the text](https://old.example/path) here";
        let start = text.find('[').unwrap();
        let end = text.find(')').unwrap() + 1;
        let mode = LinkEditMode::WithText { start, end, text: "the text".to_string() };
        let r = commit(text, &mode, "https://new.example/path");
        assert_eq!(r.text, "see [the text](https://new.example/path) here");
    }

    // --- open_insert_link(): the full apply_core dispatch ---------------------

    fn drive_open(text: &str, anchor: Option<usize>, cursor: usize) -> Option<crate::overlay::OverlayState> {
        let mut buffer = Buffer::from_str(text);
        buffer.set_cursor(cursor);
        if let Some(a) = anchor {
            buffer.select_range(a, cursor);
        }
        let mut shift_selecting = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut overlay = None;
        let mut make_overlay = |_k: OverlayKind| -> Option<crate::overlay::OverlayState> { None };
        let mut browse_to =
            |_k: OverlayKind, _r: Option<String>| -> Option<crate::overlay::OverlayState> { None };
        let mut ctx = ActionCtx {
            buffer: &mut buffer,
            shift_selecting: &mut shift_selecting,
            zoom: &mut zoom,
            search: &mut search,
            scroll_page_lines: 1,
            overlay: &mut overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
            oracle: None,
        };
        apply_core(&mut ctx, &Action::InsertLink, false);
        overlay
    }

    #[test]
    fn insert_link_opens_the_minibuffer_on_a_markdown_buffer() {
        let ov = drive_open("hello world", None, 5).expect("overlay must open");
        assert_eq!(ov.kind, OverlayKind::InsertLink);
        assert!(ov.link_edit.is_some());
    }

    #[test]
    fn insert_link_is_a_calm_no_op_on_a_non_markdown_buffer() {
        let mut buffer = Buffer::from_str("fn main() {}");
        buffer.set_path(std::path::PathBuf::from("x.rs"));
        buffer.set_cursor(3);
        assert!(!buffer.is_markdown());
        let mut shift_selecting = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut overlay = None;
        let mut make_overlay = |_k: OverlayKind| -> Option<crate::overlay::OverlayState> { None };
        let mut browse_to =
            |_k: OverlayKind, _r: Option<String>| -> Option<crate::overlay::OverlayState> { None };
        let mut ctx = ActionCtx {
            buffer: &mut buffer,
            shift_selecting: &mut shift_selecting,
            zoom: &mut zoom,
            search: &mut search,
            scroll_page_lines: 1,
            overlay: &mut overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
            oracle: None,
        };
        apply_core(&mut ctx, &Action::InsertLink, false);
        assert!(overlay.is_none(), "a non-markdown buffer must not open the link minibuffer");
    }

    /// Full flow: Cmd-K on a selection → type a URL → Enter commits as ONE
    /// undoable edit → Cmd-Z restores the exact pre-edit text + selection.
    #[test]
    fn full_wrap_flow_commits_one_undoable_edit_and_undo_restores_exactly() {
        let mut buffer = Buffer::from_str("hello world");
        buffer.select_range(0, 5); // "hello"
        let before_version = buffer.version();

        let mut shift_selecting = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut overlay = None;
        {
            let mut make_overlay = |_k: OverlayKind| -> Option<crate::overlay::OverlayState> { None };
            let mut browse_to =
                |_k: OverlayKind, _r: Option<String>| -> Option<crate::overlay::OverlayState> { None };
            let mut ctx = ActionCtx {
                buffer: &mut buffer,
                shift_selecting: &mut shift_selecting,
                zoom: &mut zoom,
                search: &mut search,
                scroll_page_lines: 1,
                overlay: &mut overlay,
                make_overlay: &mut make_overlay,
                browse_to: &mut browse_to,
                oracle: None,
            };
            apply_core(&mut ctx, &Action::InsertLink, false);
        }
        let ov = overlay.as_mut().expect("overlay must open");
        for c in "https://example.com".chars() {
            ov.link_edit_push(c);
        }

        {
            let mut make_overlay = |_k: OverlayKind| -> Option<crate::overlay::OverlayState> { None };
            let mut browse_to =
                |_k: OverlayKind, _r: Option<String>| -> Option<crate::overlay::OverlayState> { None };
            let mut ctx = ActionCtx {
                buffer: &mut buffer,
                shift_selecting: &mut shift_selecting,
                zoom: &mut zoom,
                search: &mut search,
                scroll_page_lines: 1,
                overlay: &mut overlay,
                make_overlay: &mut make_overlay,
                browse_to: &mut browse_to,
                oracle: None,
            };
            apply_core(&mut ctx, &Action::Newline, false);
        }
        assert!(overlay.is_none(), "commit closes the overlay");
        assert_eq!(buffer.text(), "[hello](https://example.com) world");
        assert!(buffer.version() > before_version, "the commit is a real edit");

        buffer.undo();
        assert_eq!(buffer.text(), "hello world", "undo restores the exact pre-edit text");
        // `apply_format` (the same atomic-replace primitive every markdown format
        // toggle uses) restores the CURSOR position undo recorded, not the prior
        // SELECTION (a whole-buffer replace's undo group carries no anchor) —
        // matching every other formatting-command toggle's own undo shape, not a
        // Links-v2-specific gap.
        assert_eq!(buffer.cursor_char(), 5, "undo restores the cursor to its pre-commit position");
    }

    /// Esc cancels the minibuffer cleanly — no buffer edit at all.
    #[test]
    fn esc_cancels_with_no_buffer_change() {
        let mut buffer = Buffer::from_str("hello world");
        buffer.set_cursor(5);
        let mut shift_selecting = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut overlay = None;
        let mut make_overlay = |_k: OverlayKind| -> Option<crate::overlay::OverlayState> { None };
        let mut browse_to =
            |_k: OverlayKind, _r: Option<String>| -> Option<crate::overlay::OverlayState> { None };
        let mut ctx = ActionCtx {
            buffer: &mut buffer,
            shift_selecting: &mut shift_selecting,
            zoom: &mut zoom,
            search: &mut search,
            scroll_page_lines: 1,
            overlay: &mut overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
            oracle: None,
        };
        apply_core(&mut ctx, &Action::InsertLink, false);
        assert!(ctx.overlay.is_some());
        apply_core(&mut ctx, &Action::Cancel, false);
        assert!(ctx.overlay.is_none(), "Esc/Cancel closes the minibuffer");
        drop(ctx);
        assert_eq!(buffer.text(), "hello world", "cancel never edits the buffer");
    }

    // --- byte_to_char ----------------------------------------------------------

    #[test]
    fn byte_to_char_handles_multibyte_prefixes() {
        // "héllo" — 'é' is 2 bytes, so byte offset 3 (right after 'é') is char 2.
        let text = "héllo";
        assert_eq!(byte_to_char(text, 0), 0);
        assert_eq!(byte_to_char(text, 3), 2);
        assert_eq!(byte_to_char(text, text.len()), text.chars().count());
    }
}
