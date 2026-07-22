//! Markdown formatting-command toggles (align table, bold/bullet/code-block),
//! smart-newline continuation, Tab list indent/outdent, and select-all --
//! split out of the former monolithic `actions::tests` (2026-07
//! code-organization pass).

use super::super::*;
use crate::overlay::OverlayKind;
use super::{drive_format, drive_newline, md, drive_act};

#[test]
fn align_table_aligns_under_caret_is_undoable_and_no_ops_outside() {
    // Action::AlignTable routes through the SAME apply_core seam a palette/menu
    // invocation uses, so `--keys` drives it identically. A no-path buffer is
    // markdown, so the table under the caret aligns.
    let src = "intro\n| Name | V |\n|---|---|\n| a | 100 |\ntail\n";
    let mut buffer = Buffer::from_str(src);
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut overlay = None;
    let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
    let mut browse_to =
        |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };

    // Caret INSIDE the table (on the body row) — align re-pads the block.
    buffer.set_cursor(buffer.line_col_to_char(3, 2));
    let mut ctx = ActionCtx {
        buffer: &mut buffer,
        shift_selecting: &mut shift,
        zoom: &mut zoom,
        search: &mut search,
        scroll_page_lines: 1,
        overlay: &mut overlay,
        make_overlay: &mut make_overlay,
        browse_to: &mut browse_to,
        oracle: None,
    };
    let before = ctx.buffer.text();
    apply_core(&mut ctx, &Action::AlignTable, false);
    let after = ctx.buffer.text();
    assert_ne!(after, before, "align edited the buffer");
    assert!(
        after.contains("| Name | V   |\n| ---- | --- |\n| a    | 100 |"),
        "the table block is aligned in place: {after:?}"
    );
    // The surrounding prose is untouched.
    assert!(after.starts_with("intro\n") && after.ends_with("tail\n"));

    // UNDOABLE: one Cmd-Z restores the exact pre-align source.
    ctx.buffer.undo();
    assert_eq!(ctx.buffer.text(), before, "undo restores the pre-align source");

    // NO-OP outside a table: caret on the prose intro line does nothing.
    ctx.buffer.set_cursor(0);
    let untouched = ctx.buffer.text();
    let eff = apply_core(&mut ctx, &Action::AlignTable, false);
    assert_eq!(eff, Effect::None, "align outside a table is a calm no-op");
    assert_eq!(ctx.buffer.text(), untouched, "…and edits nothing");
    assert!(!ctx.buffer.can_undo(), "…so there is nothing to undo");
}

#[test]
fn bold_toggle_through_apply_core_is_one_undoable_edit() {
    // Cmd-P → "Bold" routes Action::Bold through the SAME apply_core seam a key /
    // `--keys` invocation rides. Select "quick" (cols 4..9) and toggle bold.
    let mut b = drive_format("the quick fox", Some(4), 9, &Action::Bold);
    assert_eq!(b.text(), "the **quick** fox", "bold wrapped the selection");
    // The selection covers the same visible text, inside the delimiters.
    assert_eq!(b.selection_range(), Some((6, 11)));
    // ONE undo restores the exact pre-toggle text (a full-buffer replace never
    // coalesces — the whole toggle is a single atomic group).
    b.undo();
    assert_eq!(b.text(), "the quick fox", "one Cmd-Z reverts the toggle");
}

#[test]
fn bullet_list_toggle_through_apply_core_round_trips_and_undoes() {
    // Select the two content lines (cols 0..4 over "a\nb\n") and toggle a bullet list.
    let mut b = drive_format("a\nb\nc\n", Some(0), 4, &Action::ToggleBulletList);
    assert_eq!(b.text(), "- a\n- b\nc\n", "every selected line is prefixed");
    // A second dispatch (the selection now spans the prefixed lines) strips them.
    let re = drive_format(&b.text(), b.selection_range().map(|(s, _)| s), b.selection_range().unwrap().1, &Action::ToggleBulletList);
    assert_eq!(re.text(), "a\nb\nc\n", "re-toggle strips the bullets back");
    // And one undo of the FIRST toggle restores the plain lines.
    b.undo();
    assert_eq!(b.text(), "a\nb\nc\n", "one Cmd-Z reverts the bullet toggle");
}

#[test]
fn code_block_toggle_through_apply_core_wraps_and_undoes() {
    let mut b = drive_format("let x = 1;\n", None, 3, &Action::ToggleCodeBlock);
    assert_eq!(b.text(), "```\nlet x = 1;\n```\n", "the caret line is fenced");
    b.undo();
    assert_eq!(b.text(), "let x = 1;\n", "one Cmd-Z reverts the fence");
}

#[test]
fn heading_toggle_is_a_noop_on_a_code_buffer() {
    // Formatting commands are markdown-only: a `.rs` buffer is never touched
    // (block markup would corrupt code). No edit → nothing to undo.
    use std::path::PathBuf;
    let mut buffer = Buffer::from_str("fn main() {}\n");
    buffer.set_path(PathBuf::from("/tmp/x.rs"));
    buffer.set_cursor(0);
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut overlay = None;
    let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
    let mut browse_to =
        |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
    let mut ctx = ActionCtx {
        buffer: &mut buffer,
        shift_selecting: &mut shift,
        zoom: &mut zoom,
        search: &mut search,
        scroll_page_lines: 1,
        overlay: &mut overlay,
        make_overlay: &mut make_overlay,
        browse_to: &mut browse_to,
        oracle: None,
    };
    apply_core(&mut ctx, &Action::ToggleHeading, false);
    assert_eq!(ctx.buffer.text(), "fn main() {}\n", "a code buffer is left untouched");
    assert!(!ctx.buffer.can_undo(), "no edit was recorded");
}

#[test]
fn smart_newline_continues_lists_quotes_and_indent() {
    // Unordered bullet carries to the new line.
    let mut b = md("- a", 3);
    drive_newline(&mut b);
    assert_eq!(b.text(), "- a\n- ");
    assert_eq!(b.cursor_char(), 6);

    // Ordered list AUTO-INCREMENTS the number.
    let mut b = md("1. first", 8);
    drive_newline(&mut b);
    assert_eq!(b.text(), "1. first\n2. ");

    // A double-digit ordered marker keeps counting and preserves the delimiter.
    let mut b = md("9) nine", 7);
    drive_newline(&mut b);
    assert_eq!(b.text(), "9) nine\n10) ");

    // Blockquote continues with the same '>' run.
    let mut b = md("> quote", 7);
    drive_newline(&mut b);
    assert_eq!(b.text(), "> quote\n> ");

    // Leading indentation is preserved on a plain Enter.
    let mut b = md("    code", 8);
    drive_newline(&mut b);
    assert_eq!(b.text(), "    code\n    ");
}

#[test]
fn smart_newline_empty_item_ends_the_block() {
    // Enter on an EMPTY bullet strips the dangling marker (ends the list).
    let mut b = md("- a\n- ", 6);
    drive_newline(&mut b);
    assert_eq!(b.text(), "- a\n");
    assert_eq!(b.cursor_char(), 4);

    // Same for an empty ordered item …
    let mut b = md("1. ", 3);
    drive_newline(&mut b);
    assert_eq!(b.text(), "");
    assert_eq!(b.cursor_char(), 0);

    // … and an empty blockquote.
    let mut b = md("> ", 2);
    drive_newline(&mut b);
    assert_eq!(b.text(), "");
}

#[test]
fn smart_newline_is_markdown_only() {
    // A non-markdown buffer (a path with a non-md extension) gets a PLAIN
    // newline — no marker continuation — so `.rs` / `.txt` editing is
    // byte-identical. (A no-path scratch buffer is now the prose-first writing
    // surface and DOES continue markers; only a saved non-md file opts out.)
    let mut b = Buffer::from_str("- a");
    b.set_path(std::path::PathBuf::from("code.rs"));
    b.set_cursor(3);
    drive_newline(&mut b);
    assert_eq!(b.text(), "- a\n");
    assert_eq!(b.cursor_char(), 4);
}

#[test]
fn tab_indents_a_list_line_and_shift_tab_outdents() {
    // TAB on a bullet indents one level (+2 leading spaces); the depth glyph is
    // derived downstream, so only the text changes here.
    let mut b = md("- item", 6);
    drive_act(&mut b, &Action::InsertTab);
    assert_eq!(b.text(), "  - item");
    // The caret rides with the content (+2).
    assert_eq!(b.cursor_char(), 8);

    // SHIFT-TAB outdents it back (−2, clamped at 0 so a second one is a no-op).
    drive_act(&mut b, &Action::Outdent);
    assert_eq!(b.text(), "- item");
    let v = b.version();
    drive_act(&mut b, &Action::Outdent);
    assert_eq!(b.text(), "- item", "outdent clamps at column 0");
    assert_eq!(b.version(), v, "a clamped outdent makes no edit");
}

#[test]
fn tab_indents_an_ordered_list_without_renumbering() {
    // Ordered items indent too (Tab/Shift-Tab), and we do NOT auto-renumber.
    let mut b = md("1. first", 8);
    drive_act(&mut b, &Action::InsertTab);
    assert_eq!(b.text(), "  1. first", "ordered item indents, number unchanged");
    drive_act(&mut b, &Action::Outdent);
    assert_eq!(b.text(), "1. first");
}

#[test]
fn tab_off_a_list_inserts_spaces_not_an_indent() {
    // On a plain prose line Tab keeps the existing soft-tab (to the next 4-stop),
    // so non-list editing is unchanged.
    let mut b = md("hello", 5);
    drive_act(&mut b, &Action::InsertTab);
    assert_eq!(b.text(), "hello   ", "col 5 => 3 spaces to the next 4-stop");
}

#[test]
fn tab_indents_all_selected_list_lines() {
    // A selection spanning three bullets: one Tab indents them ALL as one undo step.
    let mut b = md("- a\n- b\n- c", 0);
    b.set_mark(); // anchor at 0
    b.set_cursor(b.text().chars().count()); // extend to end => whole doc selected
    drive_act(&mut b, &Action::InsertTab);
    assert_eq!(b.text(), "  - a\n  - b\n  - c", "every selected bullet indents");
    // One undo restores the whole block (the indent is atomic).
    b.undo();
    assert_eq!(b.text(), "- a\n- b\n- c", "the block indent is one atomic undo");

    // Shift-Tab outdents a whole selection back, on an already-indented block.
    let mut b = md("  - a\n  - b\n  - c", 0);
    b.set_mark();
    b.set_cursor(b.text().chars().count());
    drive_act(&mut b, &Action::Outdent);
    assert_eq!(b.text(), "- a\n- b\n- c", "every selected bullet outdents");
}

#[test]
fn select_all_selects_the_whole_buffer_region() {
    // A multi-line buffer with the cursor parked mid-document.
    let mut b = Buffer::from_str("alpha\nbeta\ngamma\n");
    let len = b.text().chars().count();
    b.set_cursor(3); // somewhere in the middle, no mark
    assert!(!b.has_selection());

    drive_act(&mut b, &Action::SelectAll);

    // Mark at document start, point at document end => the whole doc is the region.
    assert!(b.has_selection());
    assert_eq!(b.anchor_char(), Some(0));
    assert_eq!(b.cursor_char(), len);
    assert_eq!(b.selection_range(), Some((0, len)));
    // Endpoints span from (line 0, col 0) to the last line's last col.
    let ((l0, c0), (l1, _c1)) = b.selection_line_col().unwrap();
    assert_eq!((l0, c0), (0, 0), "region starts at document start");
    assert_eq!(l1, b.line_count() - 1, "region ends on the last line");
}

#[test]
fn select_all_on_empty_buffer_is_a_safe_no_op() {
    // An EMPTY buffer: select-all must not panic and leaves an empty region
    // (anchor == cursor == 0), so nothing is "selected".
    let mut b = Buffer::from_str("");
    drive_act(&mut b, &Action::SelectAll);
    assert!(!b.has_selection(), "empty buffer => empty region, not a selection");
    assert_eq!(b.cursor_char(), 0);
    assert_eq!(b.selection_range(), None);
}

#[test]
fn kill_region_after_select_all_empties_the_buffer() {
    // Cmd-A then C-w (cut) removes the ENTIRE document.
    let mut b = Buffer::from_str("one\ntwo\nthree\n");
    drive_act(&mut b, &Action::SelectAll);
    drive_act(&mut b, &Action::KillRegion);
    assert_eq!(b.text(), "", "select-all + cut empties the buffer");
    assert!(!b.has_selection());
    // The cut text is in the kill buffer, so a yank restores the whole doc.
    drive_act(&mut b, &Action::Yank);
    assert_eq!(b.text(), "one\ntwo\nthree\n", "the cut whole-doc yanks back");
}

#[test]
fn type_after_select_all_replaces_the_whole_buffer() {
    // Cmd-A then typing a char replaces the ENTIRE selection with that char,
    // as one atomic edit (one undo restores the original document).
    let mut b = Buffer::from_str("keep\nnothing\nof this\n");
    drive_act(&mut b, &Action::SelectAll);
    drive_act(&mut b, &Action::InsertChar('x'));
    assert_eq!(b.text(), "x", "the whole selection is replaced by the typed char");
    assert_eq!(b.cursor_char(), 1);
    b.undo();
    assert_eq!(b.text(), "keep\nnothing\nof this\n", "one undo restores the original");
}

#[test]
fn copy_region_after_select_all_copies_all_and_keeps_text() {
    // Cmd-A then M-w (copy) leaves the text intact but stages the whole doc for
    // a yank (the mark clears, as copy_region does).
    let mut b = Buffer::from_str("copy\nme\n");
    drive_act(&mut b, &Action::SelectAll);
    drive_act(&mut b, &Action::CopyRegion);
    assert_eq!(b.text(), "copy\nme\n", "copy leaves the document unchanged");
    assert!(!b.has_selection(), "copy clears the mark");
    // Yanking at the end appends the copied whole document.
    b.buffer_end();
    drive_act(&mut b, &Action::Yank);
    assert_eq!(b.text(), "copy\nme\ncopy\nme\n", "the copied whole doc yanks in");
}

#[test]
fn smart_newline_parser_declines_plain_and_inside_marker() {
    // Plain prose: nothing to continue.
    assert!(smart_newline_for("hello", 5).is_none());
    // Caret inside the marker (col 0 of a bullet): plain newline, no dupe.
    assert!(smart_newline_for("- item", 0).is_none());
    // A lone "-" without a trailing space is not a list yet.
    assert!(smart_newline_for("-", 1).is_none());
}

#[test]
fn dash_then_enter_leaves_a_writable_line_item_40() {
    // ITEM 40 regression — `-` then Enter must never strand an UNWRITABLE empty
    // item. Decided semantics (2026-07-23): a lone `-` (no trailing space) is not
    // a list yet, so Enter falls through to a PLAIN newline — the dash stays a
    // literal `-` on its own line with a fresh blank line below. Drive the whole
    // gesture through the REAL apply_core seam exactly as `--keys "- Enter x"`
    // does (InsertChar → Newline → InsertChar), then assert the typed character
    // actually LANDED after the newline and the caret advanced onto it — i.e. the
    // new line is writable, not eaten. (`-` alone yields no `md_spans`, so nothing
    // conceals; the buffer-level writability contract is the floor this pins.)
    let mut b = md("", 0);
    drive_act(&mut b, &Action::InsertChar('-'));
    drive_act(&mut b, &Action::Newline);
    drive_act(&mut b, &Action::InsertChar('x'));
    assert_eq!(b.text(), "-\nx", "the dash stays literal and `x` lands on the new line");
    // Caret sits AFTER the `x`: char 3 over "-\nx" — the line the user landed on
    // genuinely took the keystroke.
    assert_eq!(b.cursor_char(), 3, "caret advanced past the written `x`");
    let (line, col) = b.cursor_line_col();
    assert_eq!((line, col), (1, 1), "caret is on the new line, one column in");
}

#[test]
fn smart_newline_ordered_marker_at_usize_max_saturates_no_overflow() {
    // A pathological ordered marker of exactly `usize::MAX` parses fine, but the
    // continuation used to compute `n + 1` — which OVERFLOWS (panic in debug,
    // wrap-to-0 in release). `saturating_add(1)` pins the number at usize::MAX
    // instead: the marker simply stops counting up rather than crashing.
    let max = usize::MAX; // 18446744073709551615 on 64-bit
    let line = format!("{max}. item");
    let col = line.chars().count();
    match smart_newline_for(&line, col) {
        Some(SmartNewline::Continue(prefix)) => {
            assert_eq!(prefix, format!("{max}. "), "the number saturates, never overflows");
        }
        _ => panic!("expected a continued ordered item at the usize::MAX marker"),
    }
}
