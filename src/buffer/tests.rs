//! Unit tests for the buffer module — cursor / motion / selection / undo-redo /
//! quick-note naming / focus-mode bounds. Carved out of `buffer.rs` verbatim into
//! the module root's `#[cfg(test)] mod tests;`. `use super::*` reaches every item
//! (the re-exported free functions included) exactly as before.

    use super::*;

    fn b(s: &str) -> Buffer {
        Buffer::from_str(s)
    }

    #[test]
    fn cursor_line_col_basic() {
        let mut buf = b("hello\nworld");
        assert_eq!(buf.cursor_line_col(), (0, 0));
        buf.buffer_end();
        assert_eq!(buf.cursor_line_col(), (1, 5));
    }

    #[test]
    fn paragraph_bounds_around_cursor() {
        // Two paragraphs separated by a blank line.
        let text = "First para line one.\nFirst para line two.\n\nSecond paragraph here.\n";
        // A cursor anywhere in the first paragraph selects both of its lines but
        // NOT the blank line or the second paragraph.
        let blank_at = text.chars().position(|_| false); // placeholder, unused
        let _ = blank_at;
        let first_end = "First para line one.\nFirst para line two.".chars().count();
        // cursor at char 5 (inside line one).
        assert_eq!(paragraph_bounds_str(text, 5), (0, first_end));
        // cursor inside line two of the first paragraph -> same paragraph.
        let in_line_two = "First para line one.\nFirst ".chars().count();
        assert_eq!(paragraph_bounds_str(text, in_line_two), (0, first_end));
        // cursor in the second paragraph -> just the second paragraph.
        let second_start = "First para line one.\nFirst para line two.\n\n".chars().count();
        let second_end = second_start + "Second paragraph here.".chars().count();
        let in_second = second_start + 3;
        assert_eq!(paragraph_bounds_str(text, in_second), (second_start, second_end));
        // cursor on the blank line now lights the paragraph ABOVE (never the empty
        // gap), so the page is never fully dimmed mid-write.
        let blank_start = "First para line one.\nFirst para line two.\n".chars().count();
        assert_eq!(paragraph_bounds_str(text, blank_start), (0, first_end));
    }

    #[test]
    fn paragraph_bounds_on_blank_line_lights_paragraph_above() {
        // A cursor on the blank GAP between two paragraphs must light the paragraph
        // just finished (above), not return an empty range that greys the whole page.
        let text = "Alpha one.\nAlpha two.\n\nBeta one.\nBeta two.\n";
        let above_end = "Alpha one.\nAlpha two.".chars().count();
        let blank = "Alpha one.\nAlpha two.\n".chars().count(); // start of the blank line
        let (s, e) = paragraph_bounds_str(text, blank);
        assert_eq!((s, e), (0, above_end));
        assert!(s != e, "blank-line paragraph range must be non-empty");
    }

    #[test]
    fn paragraph_bounds_leading_blank_uses_paragraph_below() {
        // Cursor on leading blank lines (nothing above) -> the first paragraph below.
        let text = "\n\nFirst real.\nMore.\n";
        let start = "\n\n".chars().count();
        let end = "\n\nFirst real.\nMore.".chars().count();
        assert_eq!(paragraph_bounds_str(text, 0), (start, end));
        assert_eq!(paragraph_bounds_str(text, 1), (start, end));
    }

    #[test]
    fn paragraph_bounds_all_blank_is_empty() {
        // No prose anywhere -> an empty range is acceptable (nothing to light).
        let text = "\n  \n\t\n";
        let (s, e) = paragraph_bounds_str(text, 2);
        assert_eq!(s, e);
    }

    #[test]
    fn sentence_bounds_on_blank_line_lights_sentence_above() {
        // Cursor on the blank line between two paragraphs -> the last sentence above
        // (the one just finished), never an empty forward-biased range off the end.
        let text = "Alpha. Beta.\n\nGamma. Delta.\n";
        let start = "Alpha. ".chars().count();
        let end = "Alpha. Beta.".chars().count();
        let blank = "Alpha. Beta.\n".chars().count(); // start of the blank line
        let (s, e) = sentence_bounds_str(text, blank);
        assert_eq!((s, e), (start, end));
        assert!(s != e, "blank-line sentence range must be non-empty");
    }

    #[test]
    fn sentence_bounds_leading_blank_uses_sentence_below() {
        // Leading blank lines, nothing above -> the first sentence below.
        let text = "\n\nFirst. Second.\n";
        let start = "\n\n".chars().count();
        let end = "\n\nFirst.".chars().count();
        assert_eq!(sentence_bounds_str(text, 0), (start, end));
    }

    #[test]
    fn sentence_bounds_all_blank_is_empty() {
        let text = "\n \n";
        let (s, e) = sentence_bounds_str(text, 1);
        assert_eq!(s, e);
    }

    #[test]
    fn sentence_bounds_splits_on_terminators() {
        let text = "One sentence. Two sentence! Three?";
        // cursor inside the first sentence.
        assert_eq!(sentence_bounds_str(text, 2), (0, "One sentence.".chars().count()));
        // cursor inside the second sentence.
        let two_start = "One sentence. ".chars().count();
        let two_end = "One sentence. Two sentence!".chars().count();
        assert_eq!(sentence_bounds_str(text, two_start + 2), (two_start, two_end));
        // cursor in the third sentence (ends at EOF, no trailing whitespace).
        let three_start = "One sentence. Two sentence! ".chars().count();
        assert_eq!(sentence_bounds_str(text, three_start + 1), (three_start, text.chars().count()));
    }

    #[test]
    fn sentence_bounds_between_sentences_biases_forward() {
        let text = "Alpha. Beta.";
        // cursor sitting on the space AFTER the first terminator -> the upcoming
        // "Beta." sentence, not "Alpha.".
        let space_idx = "Alpha.".chars().count(); // index of the space
        let (s, e) = sentence_bounds_str(text, space_idx + 1);
        assert_eq!((s, e), ("Alpha. ".chars().count(), text.chars().count()));
    }

    #[test]
    fn bounds_robust_on_empty() {
        assert_eq!(paragraph_bounds_str("", 0), (0, 0));
        assert_eq!(sentence_bounds_str("", 0), (0, 0));
    }

    #[test]
    fn buffer_bounds_methods_match_free_fns() {
        // The Buffer wrappers delegate to the pure helpers over the buffer text.
        let buf = b("Hello there. Second one.\n\nNext para.");
        let idx = 3; // inside "Hello there."
        assert_eq!(buf.paragraph_bounds(idx), paragraph_bounds_str(&buf.text(), idx));
        assert_eq!(buf.sentence_bounds(idx), sentence_bounds_str(&buf.text(), idx));
        assert_eq!(buf.sentence_bounds(idx), (0, "Hello there.".chars().count()));
    }

    #[test]
    fn forward_backward_char() {
        let mut buf = b("ab");
        buf.forward_char();
        assert_eq!(buf.cursor_char(), 1);
        buf.forward_char();
        assert_eq!(buf.cursor_char(), 2);
        buf.forward_char(); // clamp at end
        assert_eq!(buf.cursor_char(), 2);
        buf.backward_char();
        assert_eq!(buf.cursor_char(), 1);
        buf.backward_char();
        buf.backward_char(); // clamp at start
        assert_eq!(buf.cursor_char(), 0);
    }

    #[test]
    fn line_start_end() {
        let mut buf = b("hello\nworld");
        buf.next_line(); // now on line 1
        buf.line_end_motion();
        assert_eq!(buf.cursor_line_col(), (1, 5));
        buf.line_start_motion();
        assert_eq!(buf.cursor_line_col(), (1, 0));
    }

    #[test]
    fn vertical_keeps_goal_column() {
        // line 0 long, line 1 short, line 2 long. Goal column should survive
        // the short middle line.
        let mut buf = b("abcdef\nxy\nABCDEF");
        // move to col 5 on line 0
        for _ in 0..5 {
            buf.forward_char();
        }
        assert_eq!(buf.cursor_line_col(), (0, 5));
        buf.next_line(); // line 1 only has 2 chars -> clamp to col 2
        assert_eq!(buf.cursor_line_col(), (1, 2));
        buf.next_line(); // line 2 long -> restore goal col 5
        assert_eq!(buf.cursor_line_col(), (2, 5));
    }

    #[test]
    fn word_motion_forward() {
        let mut buf = b("foo bar.baz");
        buf.forward_word();
        assert_eq!(buf.cursor_char(), 3); // after "foo"
        buf.forward_word();
        assert_eq!(buf.cursor_char(), 7); // after "bar"
        buf.forward_word();
        assert_eq!(buf.cursor_char(), 11); // after "baz"
    }

    #[test]
    fn word_motion_backward() {
        let mut buf = b("foo bar baz");
        buf.buffer_end();
        buf.backward_word();
        assert_eq!(buf.cursor_char(), 8); // start of "baz"
        buf.backward_word();
        assert_eq!(buf.cursor_char(), 4); // start of "bar"
        buf.backward_word();
        assert_eq!(buf.cursor_char(), 0); // start of "foo"
    }

    #[test]
    fn word_motion_skips_leading_punct() {
        let mut buf = b("  ..foo");
        buf.forward_word();
        assert_eq!(buf.cursor_char(), 7); // jumps over spaces+dots to end of foo
    }

    #[test]
    fn insert_and_delete() {
        let mut buf = b("");
        buf.insert_char('h');
        buf.insert_char('i');
        assert_eq!(buf.text(), "hi");
        assert_eq!(buf.cursor_char(), 2);
        buf.delete_backward();
        assert_eq!(buf.text(), "h");
        buf.backward_char();
        buf.delete_forward();
        assert_eq!(buf.text(), "");
    }

    #[test]
    fn delete_word_forward_mid_line() {
        // M-d at a word start deletes exactly that word (leaves trailing space).
        let mut buf = b("foo bar baz");
        buf.delete_word_forward();
        assert_eq!(buf.text(), " bar baz");
        assert_eq!(buf.cursor_char(), 0); // cursor stays; text collapsed to meet it
    }

    #[test]
    fn delete_word_forward_stops_at_word_end() {
        // Mid-word, M-d removes only the rest of the current word — not the next.
        let mut buf = b("foo bar");
        buf.forward_char(); // cursor after 'f'
        buf.delete_word_forward();
        assert_eq!(buf.text(), "f bar");
        assert_eq!(buf.cursor_char(), 1);
    }

    #[test]
    fn delete_word_forward_skips_leading_whitespace() {
        // Like M-f, it skips a run of non-word chars, then eats the word.
        let mut buf = b("foo   bar baz");
        for _ in 0..3 {
            buf.forward_char(); // cursor at the first space (col 3)
        }
        buf.delete_word_forward();
        assert_eq!(buf.text(), "foo baz"); // "   bar" removed
        assert_eq!(buf.cursor_char(), 3);
    }

    #[test]
    fn delete_word_forward_end_of_buffer_is_noop() {
        let mut buf = b("foo");
        buf.buffer_end();
        buf.delete_word_forward(); // no panic, no over-delete
        assert_eq!(buf.text(), "foo");
        assert_eq!(buf.cursor_char(), 3);
    }

    #[test]
    fn delete_word_forward_is_char_safe() {
        // Multi-byte chars are word chars indexed by CHAR, so no byte-boundary panic.
        let mut buf = b("café wörld");
        buf.delete_word_forward();
        assert_eq!(buf.text(), " wörld");
        assert_eq!(buf.cursor_char(), 0);
    }

    #[test]
    fn delete_word_forward_yank_round_trip() {
        // The killed word lands in the kill buffer, so C-y brings it back.
        let mut buf = b("foo bar");
        buf.delete_word_forward();
        assert_eq!(buf.text(), " bar");
        buf.yank();
        assert_eq!(buf.text(), "foo bar");
    }

    #[test]
    fn insert_newline_splits() {
        let mut buf = b("helloworld");
        for _ in 0..5 {
            buf.forward_char();
        }
        buf.insert_newline();
        assert_eq!(buf.text(), "hello\nworld");
        assert_eq!(buf.cursor_line_col(), (1, 0));
    }

    #[test]
    fn tab_inserts_spaces_to_next_stop() {
        let mut buf = b("");
        buf.insert_tab();
        assert_eq!(buf.text(), "    "); // col 0 -> a full 4-wide tab
        let mut buf2 = b("ab");
        buf2.buffer_end(); // col 2
        buf2.insert_tab();
        assert_eq!(buf2.text(), "ab  "); // 2 spaces to reach the next stop
    }

    #[test]
    fn tab_is_a_single_undo() {
        let mut buf = b("x");
        buf.buffer_end(); // col 1
        buf.insert_tab(); // 3 spaces to the next stop
        assert_eq!(buf.text(), "x   ");
        buf.undo();
        assert_eq!(buf.text(), "x");
    }

    #[test]
    fn kill_line_to_eol() {
        let mut buf = b("hello world\nsecond");
        for _ in 0..6 {
            buf.forward_char();
        }
        buf.kill_line();
        assert_eq!(buf.text(), "hello \nsecond");
        assert_eq!(buf.kill_buffer(), "world");
    }

    #[test]
    fn kill_line_at_eol_kills_newline() {
        let mut buf = b("hello\nworld");
        buf.line_end_motion(); // end of "hello", before '\n'
        buf.kill_line(); // kills the newline -> join
        assert_eq!(buf.text(), "helloworld");
    }

    #[test]
    fn consecutive_kills_append() {
        let mut buf = b("hello world\n");
        // kill "hello world" then the newline, accumulating in kill buffer
        buf.kill_line();
        assert_eq!(buf.kill_buffer(), "hello world");
        buf.kill_line(); // at eol now -> kills newline, appends
        assert_eq!(buf.kill_buffer(), "hello world\n");
        assert_eq!(buf.text(), "");
    }

    #[test]
    fn kill_then_move_resets_accumulation() {
        let mut buf = b("aaa\nbbb");
        buf.kill_line(); // kill "aaa", kill="aaa"
        assert_eq!(buf.kill_buffer(), "aaa");
        buf.forward_char(); // a motion resets the kill flag
        buf.line_end_motion();
        buf.kill_line(); // now on the (joined) tail; fresh kill, not appended
        assert_ne!(buf.kill_buffer(), "aaa\n");
    }

    #[test]
    fn yank_inserts_kill_buffer() {
        let mut buf = b("hello world");
        for _ in 0..6 {
            buf.forward_char();
        }
        buf.kill_line(); // kill "world"
        buf.buffer_start();
        buf.yank();
        assert_eq!(buf.text(), "worldhello ");
        assert_eq!(buf.cursor_char(), 5);
    }

    #[test]
    fn kill_and_yank_roundtrip() {
        let mut buf = b("line one\nline two");
        buf.kill_line(); // kill "line one"
        buf.delete_forward(); // remove the leftover newline
        // buffer now "line two", kill = "line one"
        buf.buffer_end();
        buf.insert_newline();
        buf.yank();
        assert_eq!(buf.text(), "line two\nline one");
    }

    #[test]
    fn dirty_flag_tracks_edits() {
        let mut buf = b("x");
        assert!(!buf.is_dirty());
        buf.forward_char();
        assert!(!buf.is_dirty()); // motion doesn't dirty
        buf.insert_char('y');
        assert!(buf.is_dirty());
    }

    // --- Selection tests --------------------------------------------------

    #[test]
    fn set_mark_then_motion_extends_region() {
        let mut buf = b("hello world");
        buf.set_mark(); // anchor at 0
        for _ in 0..5 {
            buf.forward_char();
        }
        // region is [0,5) = "hello"
        assert_eq!(buf.selection_range(), Some((0, 5)));
        assert!(buf.has_selection());
    }

    #[test]
    fn clear_mark_drops_selection() {
        let mut buf = b("abc");
        buf.set_mark();
        buf.forward_char();
        assert!(buf.has_selection());
        buf.clear_mark();
        assert!(!buf.has_selection());
        assert_eq!(buf.selection_range(), None);
    }

    #[test]
    fn selection_orders_endpoints_when_cursor_before_anchor() {
        let mut buf = b("abcdef");
        buf.buffer_end(); // cursor at 6
        buf.set_mark(); // anchor at 6
        for _ in 0..3 {
            buf.backward_char(); // cursor at 3, anchor 6
        }
        assert_eq!(buf.selection_range(), Some((3, 6))); // ordered
    }

    #[test]
    fn selection_span_across_lines() {
        // "line0\nline1\nline2": anchor mid-line0, cursor mid-line2.
        let mut buf = b("line0\nline1\nline2");
        for _ in 0..2 {
            buf.forward_char(); // cursor at col 2 line 0
        }
        buf.set_mark();
        // move to line 2 col 3
        buf.next_line();
        buf.next_line();
        buf.line_start_motion();
        for _ in 0..3 {
            buf.forward_char();
        }
        let ((l0, c0), (l1, c1)) = buf.selection_line_col().unwrap();
        assert_eq!((l0, c0), (0, 2));
        assert_eq!((l1, c1), (2, 3));
    }

    #[test]
    fn kill_region_cuts_and_fills_kill_buffer() {
        let mut buf = b("hello world");
        buf.set_mark();
        for _ in 0..5 {
            buf.forward_char(); // select "hello"
        }
        buf.kill_region();
        assert_eq!(buf.text(), " world");
        assert_eq!(buf.kill_buffer(), "hello");
        assert_eq!(buf.cursor_char(), 0);
        assert!(!buf.has_selection());
    }

    #[test]
    fn set_kill_roundtrips_through_kill_buffer() {
        let mut buf = b("");
        buf.set_kill("hello");
        assert_eq!(buf.kill_buffer(), "hello");
        // overwrites, does not append
        buf.set_kill("world");
        assert_eq!(buf.kill_buffer(), "world");
        // empty is allowed and clears
        buf.set_kill("");
        assert_eq!(buf.kill_buffer(), "");
    }

    #[test]
    fn set_kill_does_not_chain_with_kill_line() {
        // set_kill must NOT set last_was_kill, so a following C-k must REPLACE
        // (fresh kill), not append to, the value we set.
        let mut buf = b("abc\n");
        buf.set_kill("EXTERNAL");
        buf.kill_line(); // cursor at start of line -> kills "abc"
        assert_eq!(buf.kill_buffer(), "abc"); // replaced, NOT "EXTERNALabc"
    }

    #[test]
    fn copy_region_keeps_text() {
        let mut buf = b("hello world");
        buf.set_mark();
        for _ in 0..5 {
            buf.forward_char();
        }
        buf.copy_region();
        assert_eq!(buf.text(), "hello world"); // unchanged
        assert_eq!(buf.kill_buffer(), "hello");
        assert!(!buf.has_selection()); // mark cleared by copy
    }

    #[test]
    fn kill_then_yank_region_roundtrip() {
        let mut buf = b("hello world");
        buf.set_mark();
        for _ in 0..5 {
            buf.forward_char();
        }
        buf.kill_region(); // buffer " world", kill "hello"
        buf.buffer_end();
        buf.yank();
        assert_eq!(buf.text(), " worldhello");
    }

    #[test]
    fn typing_replaces_selection() {
        let mut buf = b("hello world");
        buf.set_mark();
        for _ in 0..5 {
            buf.forward_char(); // select "hello"
        }
        buf.insert_char('X');
        assert_eq!(buf.text(), "X world");
        assert!(!buf.has_selection());
        assert_eq!(buf.cursor_char(), 1);
    }

    #[test]
    fn backspace_deletes_selection() {
        let mut buf = b("hello world");
        buf.set_mark();
        for _ in 0..5 {
            buf.forward_char();
        }
        buf.delete_backward();
        assert_eq!(buf.text(), " world");
        assert!(!buf.has_selection());
    }

    #[test]
    fn yank_replaces_selection() {
        let mut buf = b("hello world");
        // put "XX" in kill buffer via kill_region of a throwaway
        buf.select_range(0, 0);
        buf.kill = "XX".to_string();
        buf.select_range(0, 5); // select "hello"
        buf.yank();
        assert_eq!(buf.text(), "XX world");
    }

    #[test]
    fn word_bounds_on_word_char() {
        let buf = b("foo bar.baz");
        // idx 5 is inside "bar"
        assert_eq!(buf.word_bounds(5), (4, 7));
        // idx 0 inside "foo"
        assert_eq!(buf.word_bounds(0), (0, 3));
        // idx at the space (3) -> the run of non-word chars [3,4)
        assert_eq!(buf.word_bounds(3), (3, 4));
    }

    #[test]
    fn line_bounds_includes_newline() {
        let buf = b("aaa\nbbb\nccc");
        // line 1 ("bbb") spans chars [4,8) including its trailing newline
        assert_eq!(buf.line_bounds(5), (4, 8));
        // last line has no trailing newline
        assert_eq!(buf.line_bounds(9), (8, 11));
    }

    #[test]
    fn line_col_to_char_roundtrips() {
        let buf = b("hello\nworld\n!");
        for &idx in &[0usize, 3, 5, 6, 9, 11, 12] {
            let (l, c) = buf.char_to_line_col(idx);
            assert_eq!(buf.line_col_to_char(l, c), idx, "roundtrip at {idx}");
        }
    }

    // --- Click / drag selection-collapse tests ----------------------------
    // These model the exact buffer API sequence the app's mouse handlers and
    // motion-extend path use, so a plain click can never leave a phantom
    // selection that a later bare motion would extend.

    /// A single click places the cursor and (to support a future drag) sets the
    /// anchor at the same index. The press-time state has NO visible selection
    /// (anchor == cursor), so the release-time collapse must clear the anchor,
    /// after which a bare motion just moves the cursor without selecting.
    #[test]
    fn plain_click_then_motion_does_not_select() {
        let mut buf = b("line0\nline1\nline2");
        buf.buffer_end(); // pretend we clicked near the end
        let idx = buf.cursor_char();
        // on_press, single click:
        buf.set_cursor(idx);
        buf.clear_mark();
        buf.set_anchor(idx); // anchor == cursor: no visible selection yet
        assert!(!buf.has_selection());
        // Released with no drag: the app collapses the lingering anchor when
        // has_selection() is false.
        if !buf.has_selection() {
            buf.clear_mark();
        }
        assert_eq!(buf.anchor_char(), None, "plain click must clear the anchor");
        // A bare motion (e.g. C-p / PreviousLine) must NOT create a selection.
        buf.previous_line();
        assert!(!buf.has_selection(), "bare motion after plain click selected");
        assert_eq!(buf.selection_range(), None);
    }

    /// A click-DRAG (cursor moves away from the press-time anchor) leaves a real
    /// selection, so the release-time collapse must preserve it.
    #[test]
    fn click_drag_still_selects() {
        let mut buf = b("hello world");
        // on_press at 0:
        buf.set_cursor(0);
        buf.clear_mark();
        buf.set_anchor(0);
        // on_drag (Char granularity) to idx 5:
        buf.set_cursor(5);
        assert!(buf.has_selection());
        // Released: has_selection() is true -> anchor preserved.
        if !buf.has_selection() {
            buf.clear_mark();
        }
        assert!(buf.has_selection(), "click-drag selection was dropped");
        assert_eq!(buf.selection_range(), Some((0, 5)));
    }

    /// An explicit mark (C-Space / SetMark) followed by a motion must still
    /// extend the region (Emacs `mg` sticky behavior) — the click-collapse fix
    /// only touches the mouse-release path, never the keyboard mark path.
    #[test]
    fn mark_then_motion_still_extends_after_click_fix() {
        let mut buf = b("hello world");
        // simulate a prior plain click leaving a clean (no-anchor) state:
        buf.set_cursor(0);
        buf.clear_mark();
        assert_eq!(buf.anchor_char(), None);
        // C-Space:
        buf.set_mark();
        // motion extends:
        for _ in 0..5 {
            buf.forward_char();
        }
        assert!(buf.has_selection());
        assert_eq!(buf.selection_range(), Some((0, 5)));
    }

    // --- Undo / redo tests ------------------------------------------------

    /// Type text then undo: the buffer returns to empty and the cursor home.
    #[test]
    fn undo_restores_empty_after_typing() {
        let mut buf = b("");
        for c in "abc".chars() {
            buf.insert_char(c);
        }
        assert_eq!(buf.text(), "abc");
        assert!(buf.can_undo());
        buf.undo();
        assert_eq!(buf.text(), "");
        assert_eq!(buf.cursor_char(), 0);
        assert!(!buf.can_undo());
    }

    /// Undo then redo round-trips back to the typed text + cursor.
    #[test]
    fn undo_then_redo_restores_text() {
        let mut buf = b("");
        for c in "abc".chars() {
            buf.insert_char(c);
        }
        buf.undo();
        assert_eq!(buf.text(), "");
        assert!(buf.can_redo());
        buf.redo();
        assert_eq!(buf.text(), "abc");
        assert_eq!(buf.cursor_char(), 3);
        assert!(!buf.can_redo());
    }

    /// Typing "hello world" then ONE undo removes the last word group ("world");
    /// a SECOND undo removes "hello " (word + its trailing space).
    #[test]
    fn undo_coalesces_per_word() {
        let mut buf = b("");
        for c in "hello world".chars() {
            buf.insert_char(c);
        }
        assert_eq!(buf.text(), "hello world");
        buf.undo();
        assert_eq!(buf.text(), "hello ");
        buf.undo();
        assert_eq!(buf.text(), "");
        assert!(!buf.can_undo());
    }

    /// A space is an undo boundary on BOTH sides: each word is independently
    /// undoable, and the space rides with the word before it.
    #[test]
    fn each_word_is_its_own_group() {
        let mut buf = b("");
        for c in "one two three".chars() {
            buf.insert_char(c);
        }
        buf.undo();
        assert_eq!(buf.text(), "one two ");
        buf.undo();
        assert_eq!(buf.text(), "one ");
        buf.undo();
        assert_eq!(buf.text(), "");
    }

    /// Replacing a selection then undo restores the ORIGINAL selected text (one
    /// atomic step), and the buffer text is exactly as before the replace.
    #[test]
    fn undo_restores_replaced_selection() {
        let mut buf = b("hello world");
        buf.select_range(0, 5); // select "hello"
        buf.insert_char('X'); // replace with "X"
        assert_eq!(buf.text(), "X world");
        buf.undo();
        assert_eq!(buf.text(), "hello world");
        // Cursor restored to where it was before the edit.
        assert_eq!(buf.cursor_char(), 5);
        assert!(!buf.has_selection());
    }

    /// Yank-over-selection then undo restores the original selected text in one
    /// step.
    #[test]
    fn undo_restores_yank_over_selection() {
        let mut buf = b("hello world");
        buf.kill = "ZZ".to_string();
        buf.select_range(0, 5); // select "hello"
        buf.yank();
        assert_eq!(buf.text(), "ZZ world");
        buf.undo();
        assert_eq!(buf.text(), "hello world");
    }

    /// A NEW edit after an undo clears the redo stack (linear history).
    #[test]
    fn new_edit_after_undo_clears_redo() {
        let mut buf = b("");
        for c in "abc".chars() {
            buf.insert_char(c);
        }
        buf.undo();
        assert!(buf.can_redo());
        buf.insert_char('Z');
        assert_eq!(buf.text(), "Z");
        assert!(!buf.can_redo());
        buf.redo(); // no-op now
        assert_eq!(buf.text(), "Z");
    }

    /// Sealing the group (a non-edit command) splits a same-direction run so each
    /// side is undone separately even though both were insertions.
    #[test]
    fn seal_splits_insertion_run() {
        let mut buf = b("");
        for c in "abc".chars() {
            buf.insert_char(c);
        }
        buf.seal_undo_group(); // simulate a cursor motion between bursts
        for c in "def".chars() {
            buf.insert_char(c);
        }
        assert_eq!(buf.text(), "abcdef");
        buf.undo();
        assert_eq!(buf.text(), "abc");
        buf.undo();
        assert_eq!(buf.text(), "");
    }

    /// Direction flip (insert then delete) starts a new group: undoing the delete
    /// does not also undo the preceding insertions.
    #[test]
    fn direction_flip_starts_new_group() {
        let mut buf = b("");
        for c in "abcd".chars() {
            buf.insert_char(c);
        }
        buf.delete_backward(); // delete 'd'
        buf.delete_backward(); // delete 'c'
        assert_eq!(buf.text(), "ab");
        buf.undo(); // undoes the deletion run -> "abcd"
        assert_eq!(buf.text(), "abcd");
        buf.undo(); // undoes the insertion -> ""
        assert_eq!(buf.text(), "");
    }

    /// A backspace run coalesces into one undo group.
    #[test]
    fn backspace_run_coalesces() {
        let mut buf = b("abcdef");
        buf.buffer_end();
        buf.delete_backward();
        buf.delete_backward();
        buf.delete_backward();
        assert_eq!(buf.text(), "abc");
        buf.undo();
        assert_eq!(buf.text(), "abcdef");
        assert_eq!(buf.cursor_char(), 6);
    }

    /// undo/redo bump the version counter so the view/spell layer re-syncs.
    #[test]
    fn undo_redo_bump_version() {
        let mut buf = b("");
        buf.insert_char('a');
        let v_after_type = buf.version();
        buf.undo();
        assert!(buf.version() > v_after_type);
        let v_after_undo = buf.version();
        buf.redo();
        assert!(buf.version() > v_after_undo);
    }

    #[test]
    fn line_col_to_char_clamps_col() {
        let buf = b("hi\nlonger");
        // col past end of line 0 clamps to end of "hi" (char index 2)
        assert_eq!(buf.line_col_to_char(0, 99), 2);
        // line past end clamps to last line
        let (l, _) = buf.char_to_line_col(buf.line_col_to_char(99, 0));
        assert_eq!(l, 1);
    }

    // --- QUICK NOTE: title slug, collision suffixing, auto-name on save --------

    #[test]
    fn slugify_titles() {
        assert_eq!(slugify("Japanese week 12"), "japanese-week-12");
        assert_eq!(slugify("  Hello,  World!  "), "hello-world");
        assert_eq!(slugify("UPPER Case"), "upper-case");
        // Punctuation-only / empty -> a usable fallback.
        assert_eq!(slugify("!!!"), "note");
        assert_eq!(slugify(""), "note");
    }

    #[test]
    fn first_nonempty_line_skips_blanks() {
        assert_eq!(first_nonempty_line("\n\n  \nReal title\nmore"), Some("Real title"));
        assert_eq!(first_nonempty_line("   \n\t"), None);
        assert_eq!(first_nonempty_line(""), None);
    }

    fn note_tmp(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("awl_note_test_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn unique_path_suffixes_on_collision() {
        // unique_path probes existence through the FILESYSTEM SEAM, so drive it with
        // an InMemoryFs (no temp dir).
        use crate::fs::FileSystem;
        use std::sync::Arc;
        let dir = std::path::PathBuf::from("/notes");
        let mem = crate::fs::InMemoryFs::new().with_dir(&dir);
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            // First is the bare name; once it exists, the next is -2, then -3.
            let p1 = unique_path(&dir, "japanese-week-12", "md");
            assert_eq!(p1.file_name().unwrap(), "japanese-week-12.md");
            mem.write(&p1, b"x").unwrap();
            let p2 = unique_path(&dir, "japanese-week-12", "md");
            assert_eq!(p2.file_name().unwrap(), "japanese-week-12-2.md");
            mem.write(&p2, b"x").unwrap();
            let p3 = unique_path(&dir, "japanese-week-12", "md");
            assert_eq!(p3.file_name().unwrap(), "japanese-week-12-3.md");
        });
    }

    #[test]
    fn note_save_derives_filename_from_first_line() {
        // The quick-note save path (slug derivation + collision suffix + filename
        // lock), routed through the FILESYSTEM SEAM (InMemoryFs) — no temp dir.
        use crate::fs::FileSystem;
        use std::sync::Arc;
        let dir = std::path::PathBuf::from("/notes");
        let mem = crate::fs::InMemoryFs::new().with_dir(&dir);
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            // An EMPTY note writes nothing (no litter): save bails.
            let mut buf = Buffer::scratch();
            buf.set_note_dir(dir.clone());
            assert!(buf.is_note());
            assert!(buf.save().is_err());
            assert!(buf.path().is_none());
            // Type a title; save now DERIVES <slug>.md and writes it.
            for c in "Japanese week 12".chars() {
                buf.insert_char(c);
            }
            buf.save().unwrap();
            let p = buf.path().unwrap().to_path_buf();
            assert_eq!(p.file_name().unwrap(), "japanese-week-12.md");
            assert!(mem.exists(&p));
            // Filename LOCKS: editing the first line + re-saving keeps the same path.
            buf.buffer_start();
            for c in "X ".chars() {
                buf.insert_char(c);
            }
            buf.save().unwrap();
            assert_eq!(buf.path().unwrap(), p, "filename must lock after first save");
            // A SECOND fresh note with the same title collides -> -2 suffix.
            let mut buf2 = Buffer::scratch();
            buf2.set_note_dir(dir.clone());
            for c in "Japanese week 12".chars() {
                buf2.insert_char(c);
            }
            buf2.save().unwrap();
            assert_eq!(buf2.path().unwrap().file_name().unwrap(), "japanese-week-12-2.md");
        });
    }

    #[test]
    fn display_name_for_gutter_saved_derived_and_scratch() {
        // A SAVED file shows its bound file name.
        let mut saved = Buffer::scratch();
        saved.set_path(std::path::PathBuf::from("/tmp/notes/today.md"));
        assert_eq!(saved.display_name(), "today.md");
        // An UNSAVED note shows the name it WOULD derive on first save (<slug>.md).
        let note = Buffer::from_str("Grocery list\nmilk\n");
        assert_eq!(note.display_name(), "grocery-list.md");
        // An untitled / empty buffer falls back to the scratch placeholder.
        let blank = Buffer::scratch();
        assert_eq!(blank.display_name(), "scratch.md");
    }

    #[test]
    fn note_one_word_first_line_names_file() {
        // A single-word first line yields <word>.md (no dash, no fallback).
        use crate::fs::FileSystem;
        use std::sync::Arc;
        let dir = std::path::PathBuf::from("/notes");
        let mem = crate::fs::InMemoryFs::new().with_dir(&dir);
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            let mut buf = Buffer::scratch();
            buf.set_note_dir(dir.clone());
            for c in "foo".chars() {
                buf.insert_char(c);
            }
            buf.save().unwrap();
            assert_eq!(buf.path().unwrap().file_name().unwrap(), "foo.md");
            assert!(mem.exists(buf.path().unwrap()));
        });
    }

    #[test]
    fn note_empty_writes_no_file() {
        // A truly empty note (only whitespace) NEVER writes — no litter.
        use crate::fs::FileSystem;
        use std::sync::Arc;
        let dir = std::path::PathBuf::from("/notes");
        let mem = crate::fs::InMemoryFs::new().with_dir(&dir);
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            let mut buf = Buffer::scratch();
            buf.set_note_dir(dir.clone());
            for c in "   \n\t  ".chars() {
                buf.insert_char(c);
            }
            assert!(buf.save().is_err());
            assert!(buf.path().is_none());
            // Nothing landed in the fake filesystem.
            let count = mem.read_dir(&dir).map(|d| d.len()).unwrap_or(0);
            assert_eq!(count, 0, "empty note must not write a file");
        });
    }

    #[test]
    fn note_content_without_title_falls_back_to_scratch() {
        // A first line with content but NO derivable title (punctuation only)
        // falls back to scratch.md, then scratch-2.md on the next such note.
        use std::sync::Arc;
        let dir = std::path::PathBuf::from("/notes");
        let mem = crate::fs::InMemoryFs::new().with_dir(&dir);
        crate::fs::with_fs(Arc::new(mem), || {
            let mut buf = Buffer::scratch();
            buf.set_note_dir(dir.clone());
            for c in "!!!".chars() {
                buf.insert_char(c);
            }
            buf.save().unwrap();
            assert_eq!(buf.path().unwrap().file_name().unwrap(), "scratch.md");
            // A second untitled-content note collides -> scratch-2.md.
            let mut buf2 = Buffer::scratch();
            buf2.set_note_dir(dir.clone());
            for c in "???".chars() {
                buf2.insert_char(c);
            }
            buf2.save().unwrap();
            assert_eq!(buf2.path().unwrap().file_name().unwrap(), "scratch-2.md");
        });
    }

    #[test]
    fn move_file_relocates_and_no_clobbers() {
        // The C-x m move (true rename + no-clobber + buffer re-point + save at new
        // home), all over the FILESYSTEM SEAM (InMemoryFs) — no real disk.
        use crate::fs::FileSystem;
        use std::sync::Arc;
        let root = std::path::PathBuf::from("/notes");
        let sub = root.join("archive");
        let old = root.join("idea.md");
        let mem = crate::fs::InMemoryFs::new()
            .with_dir(&sub)
            .with_file(&old, "body");
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            // A note at the root, opened into a buffer.
            let mut buf = Buffer::from_file(&old);
            // MOVE into archive/: a true rename — old path gone, new path present.
            let new = move_file(&old, &sub).unwrap();
            assert_eq!(new, sub.join("idea.md"));
            assert!(!mem.exists(&old), "old path must be gone after a move");
            assert!(mem.exists(&new), "new path must exist after a move");
            // The buffer re-points so future saves land at the new home.
            buf.set_path(new.clone());
            assert_eq!(buf.path().unwrap(), new);
            buf.insert_char('!');
            buf.save().unwrap();
            assert_eq!(mem.read_to_string(&new).unwrap(), "!body");
            // NO CLOBBER: moving a second `idea.md` into archive/ suffixes it.
            let other = root.join("idea.md");
            mem.write(&other, b"two").unwrap();
            let new2 = move_file(&other, &sub).unwrap();
            assert_eq!(new2.file_name().unwrap(), "idea-2.md");
            assert!(mem.exists(&new2) && !mem.exists(&other));
        });
    }

    #[test]
    fn syntax_lang_gates_code_only() {
        // The gate that controls whether the renderer emits ANY syntax spans: code
        // extensions highlight; markdown / txt / scratch must NOT. A path with a
        // non-markdown extension (.rs / .txt) is ALSO not markdown, so the markdown
        // and code styling passes stay mutually exclusive.
        let mut code = Buffer::from_str("fn main() {}");
        code.set_path("/p/main.rs".into());
        assert_eq!(code.syntax_lang(), Some(crate::syntax::Lang::Rust));
        assert!(!code.is_markdown(), "a .rs file is code, not markdown");

        let mut md = Buffer::from_str("# heading");
        md.set_path("/p/notes.md".into());
        assert!(md.syntax_lang().is_none(), "markdown must not syntax-highlight");
        assert!(md.is_markdown(), "and it IS markdown");

        let mut txt = Buffer::from_str("plain prose");
        txt.set_path("/p/notes.txt".into());
        assert!(txt.syntax_lang().is_none(), ".txt must not syntax-highlight");
        assert!(!txt.is_markdown(), "a .txt file is plain prose, not markdown");

        // The bare scratch buffer (no path) now reads as markdown — the prose-first
        // writing surface — yet syntax is path-based, so it is never code-highlighted
        // (markdown and code remain mutually exclusive).
        let scratch = Buffer::from_str("scratch");
        assert!(scratch.syntax_lang().is_none());
        assert!(scratch.is_markdown(), "the scratch writing surface IS markdown");
    }

    #[test]
    fn note_is_markdown_from_first_keystroke() {
        // A QUICK NOTE is conceptually always markdown (it auto-saves as `.md`), so
        // it must read as markdown the instant it is summoned — BEFORE its first
        // save derives a path. While you type the title, styling must already apply.
        let dir = note_tmp("md_gate");
        let mut note = Buffer::scratch();
        note.start_note(dir);
        assert!(note.path().is_none(), "a fresh note has no path yet");
        assert!(note.is_markdown(), "an unsaved note is markdown from the start");
        // ...and it must NOT be code-highlighted: syntax is path-based, a note has
        // no code extension, so markdown and code stay mutually exclusive.
        assert!(note.syntax_lang().is_none(), "a note never syntax-highlights");

        // Once saved, the note's path ends in `.md`, so it stays markdown.
        let mut saved = Buffer::from_str("# titled");
        saved.set_path("/notes/titled.md".into());
        assert!(saved.is_markdown(), "a saved note keeps reading as markdown");

        // The bare SCRATCH buffer (no note_dir, no path) is ALSO markdown now —
        // awl's blank launch surface is a prose-first writing surface, so `#` /
        // `**` style as you type. It is NOT a note, and (syntax is path-based) it
        // is never code-highlighted, so markdown and code stay mutually exclusive.
        let scratch = Buffer::scratch();
        assert!(scratch.is_markdown(), "the scratch writing surface IS markdown");
        assert!(!scratch.is_note(), "but it is not a quick note");
        assert!(scratch.syntax_lang().is_none(), "scratch is never code-highlighted");
    }

    #[test]
    fn rename_to_stem_tracks_title_and_no_clobbers() {
        // Live-rename (slug re-derive + true move + idempotence + no-clobber), all
        // over the FILESYSTEM SEAM (InMemoryFs) — no temp dir.
        use crate::fs::FileSystem;
        use std::sync::Arc;
        let dir = std::path::PathBuf::from("/notes");
        let mem = crate::fs::InMemoryFs::new().with_dir(&dir);
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            // A note frozen under a mid-typing TYPO: "strong opion" -> the file.
            let typo = unique_path(&dir, &note_stem("strong opion"), "md");
            assert_eq!(typo.file_name().unwrap(), "strong-opion.md");
            mem.write(&typo, b"strong opion\nbody").unwrap();
            // Fixing the title re-derives the slug and RENAMES the file to match;
            // the content rides along (a true move), the typo path is gone.
            let fixed = rename_to_stem(&typo, &note_stem("strong opinion")).unwrap();
            assert_eq!(fixed.file_name().unwrap(), "strong-opinion.md");
            assert!(mem.exists(&fixed) && !mem.exists(&typo));
            assert_eq!(mem.read_to_string(&fixed).unwrap(), "strong opion\nbody");
            // IDEMPOTENT: re-deriving the SAME title is a no-op (no churn).
            let again = rename_to_stem(&fixed, &note_stem("strong opinion")).unwrap();
            assert_eq!(again, fixed);
            // A collision-suffixed sibling already TRACKS its title: not churned.
            let sib = dir.join("strong-opinion-2.md");
            mem.write(&sib, b"x").unwrap();
            let sib_same = rename_to_stem(&sib, &note_stem("strong opinion")).unwrap();
            assert_eq!(sib_same, sib, "a -N suffix already tracks the title");
            // NO CLOBBER: renaming a THIRD note to a taken slug appends a suffix
            // (strong-opinion.md + strong-opinion-2.md exist -> -3).
            let third = dir.join("draft.md");
            mem.write(&third, b"y").unwrap();
            let third_new = rename_to_stem(&third, &note_stem("Strong Opinion")).unwrap();
            assert_eq!(third_new.file_name().unwrap(), "strong-opinion-3.md");
            assert!(mem.exists(&third_new) && !mem.exists(&third));
        });
    }
