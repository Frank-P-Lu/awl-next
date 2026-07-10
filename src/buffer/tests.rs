//! Unit tests for the buffer module — cursor / motion / selection / undo-redo /
//! quick-note naming. Carved out of `buffer.rs` verbatim into
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
    fn consecutive_word_kills_forward_accumulate() {
        // M-d M-d must ACCUMULATE both words in the kill ring (not overwrite),
        // so C-y brings back EVERYTHING killed in the run — the same append
        // precedent as consecutive C-k, respecting forward order.
        let mut buf = b("alpha beta gamma");
        buf.delete_word_forward(); // kills "alpha"
        assert_eq!(buf.kill_buffer(), "alpha");
        buf.delete_word_forward(); // kills " beta", APPENDS
        assert_eq!(buf.kill_buffer(), "alpha beta");
        assert_eq!(buf.text(), " gamma");
        buf.yank();
        assert_eq!(buf.text(), "alpha beta gamma");
    }

    #[test]
    fn consecutive_word_kills_backward_accumulate() {
        // M-Backspace M-Backspace accumulates in reading order (a BACKWARD kill
        // PREPENDS), so C-y restores both words left-to-right rather than only
        // the last-killed one.
        let mut buf = b("alpha beta");
        buf.buffer_end();
        buf.delete_word_backward(); // kills "beta"
        assert_eq!(buf.kill_buffer(), "beta");
        buf.delete_word_backward(); // kills "alpha ", PREPENDS
        assert_eq!(buf.kill_buffer(), "alpha beta");
        assert_eq!(buf.text(), "");
        buf.yank();
        assert_eq!(buf.text(), "alpha beta");
    }

    #[test]
    fn word_kill_then_move_starts_a_fresh_kill() {
        // A non-kill command between word-kills resets the kill flag, so the
        // next kill REPLACES the ring rather than accumulating (Emacs semantics).
        let mut buf = b("alpha beta gamma");
        buf.delete_word_forward(); // kills "alpha"
        assert_eq!(buf.kill_buffer(), "alpha");
        buf.forward_char(); // a motion resets the kill flag
        buf.delete_word_forward(); // fresh kill, REPLACES
        assert_eq!(buf.kill_buffer(), "beta");
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
    fn delete_to_line_start_removes_back_to_line_start_undoable_no_kill_ring() {
        // Cmd-⌫: remove from the caret back to the LOGICAL line start, caret lands
        // there. It does NOT touch the kill ring (a delete, not a cut) and is one
        // undoable step.
        let mut buf = b("hello world\nsecond");
        for _ in 0..6 {
            buf.forward_char(); // caret just after "hello " (col 6)
        }
        buf.delete_to_line_start();
        assert_eq!(buf.text(), "world\nsecond");
        assert_eq!(buf.cursor_char(), 0, "caret lands at the line start");
        assert_eq!(buf.kill_buffer(), "", "delete-to-line-start never fills the kill ring");
        buf.undo();
        assert_eq!(buf.text(), "hello world\nsecond", "one undoable step restores the prefix");

        // On a LATER line it stops at THAT line's start (never crosses the newline).
        let mut buf = b("alpha\nbeta gamma");
        buf.next_line(); // line 1, col 0
        for _ in 0..5 {
            buf.forward_char(); // after "beta " (col 5)
        }
        buf.delete_to_line_start();
        assert_eq!(buf.text(), "alpha\ngamma");
        // At column 0 it is a calm no-op — the version does not bump.
        let v = buf.version();
        buf.delete_to_line_start();
        assert_eq!(buf.text(), "alpha\ngamma");
        assert_eq!(buf.version(), v, "no-op at the line start leaves the version untouched");
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
    fn consecutive_kills_coalesce_into_one_undo_group() {
        // C-k C-k (kill the line's content, then its newline) is ONE user
        // gesture, so a single C-/ restores it fully — even though the first
        // kill removed whitespace-bearing text (which normally seals a group).
        let mut buf = b("foo bar baz\nsecond");
        buf.kill_line(); // kill "foo bar baz"
        buf.kill_line(); // at eol -> kill the newline, joining "second"
        assert_eq!(buf.text(), "second");
        buf.undo(); // ONE undo must restore the whole kill run
        assert_eq!(buf.text(), "foo bar baz\nsecond");
        assert!(!buf.can_undo(), "the kill run should be a single undo group");
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
    fn selected_text_reads_the_active_region_ordered() {
        let mut buf = b("hello world");
        buf.set_mark();
        for _ in 0..5 {
            buf.forward_char();
        }
        assert_eq!(buf.selected_text().as_deref(), Some("hello"));
        // Ordered regardless of which end the cursor sits at.
        buf.clear_mark();
        buf.buffer_end();
        buf.set_mark();
        for _ in 0.."world".len() {
            buf.backward_char();
        }
        assert_eq!(buf.selected_text().as_deref(), Some("world"));
        // No selection => None.
        buf.clear_mark();
        assert_eq!(buf.selected_text(), None);
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
    fn kill_line_clears_a_backward_mark_no_oob_slice() {
        // Regression: C-k with a BACKWARD active mark (anchor AFTER the cursor)
        // used to leave `anchor` dangling past the rope's shrunk end, so the
        // next selection-consuming op sliced out of bounds and panicked in
        // ropey. C-k must deactivate the region (Emacs semantics).
        let mut buf = b("hello world");
        buf.buffer_end(); // cursor at 11
        buf.set_mark(); // anchor at 11
        buf.buffer_start(); // cursor at 0, anchor 11 (backward selection)
        assert_eq!(buf.selection_range(), Some((0, 11)));
        buf.kill_line(); // kills "hello world" -> rope now empty
        assert_eq!(buf.text(), "");
        assert!(!buf.has_selection(), "C-k deactivates the region");
        assert_eq!(buf.anchor_char(), None);
        // The op that used to panic: a copy with the stale backward mark.
        buf.copy_region(); // must NOT panic (no OOB slice)
        assert_eq!(buf.selection_range(), None);
    }

    #[test]
    fn kill_line_clears_a_forward_mark_too() {
        // Control: a FORWARD mark (anchor BEFORE cursor) is likewise cleared.
        let mut buf = b("hello world");
        buf.set_mark(); // anchor at 0
        buf.buffer_end(); // cursor at 11, anchor 0 (forward selection)
        assert_eq!(buf.selection_range(), Some((0, 11)));
        buf.kill_line(); // at eol -> nothing to kill, but region deactivates
        assert!(!buf.has_selection(), "C-k deactivates the region");
        assert_eq!(buf.anchor_char(), None);
        buf.copy_region(); // must NOT panic
        assert_eq!(buf.selection_range(), None);
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
    fn is_url_recognizes_http_https_and_rejects_prose_and_paths() {
        // Real URLs.
        assert!(is_url("https://example.com"));
        assert!(is_url("http://example.com/the/essay?q=1#frag"));
        assert!(is_url("ftp://host/file"));
        // NOT URLs: plain prose, a bare path, an interior-space string, a bare
        // scheme with no host, an empty string, a multi-line clipboard.
        assert!(!is_url("the essay"));
        assert!(!is_url("https://has a space"));
        assert!(!is_url("/Users/frank/notes.md"));
        assert!(!is_url("./relative/path"));
        assert!(!is_url("http://")); // nothing after `://`
        assert!(!is_url("://nohost"));
        assert!(!is_url("example.com")); // no scheme
        assert!(!is_url(""));
        assert!(!is_url("https://a\nhttps://b"));
    }

    #[test]
    fn paste_url_over_selection_in_markdown_wraps_as_one_undoable_link() {
        // Markdown buffer (no path => markdown). Select "the essay", paste a URL.
        let mut buf = b("the essay");
        buf.set_kill("https://example.com");
        buf.select_range(0, 9); // select the whole "the essay"
        buf.yank();
        assert_eq!(buf.text(), "[the essay](https://example.com)");
        // ONE undoable edit: Cmd-Z restores the original text (the selection).
        buf.undo();
        assert_eq!(buf.text(), "the essay");
        assert!(!buf.can_undo());
    }

    #[test]
    fn paste_url_with_no_selection_is_a_normal_paste() {
        // No selection: URL is inserted verbatim, never wrapped.
        let mut buf = b("");
        buf.set_kill("https://example.com");
        buf.yank();
        assert_eq!(buf.text(), "https://example.com");
    }

    #[test]
    fn paste_nonurl_over_selection_is_a_normal_replace() {
        let mut buf = b("the essay");
        buf.set_kill("some prose");
        buf.select_range(0, 9);
        buf.yank();
        assert_eq!(buf.text(), "some prose");
    }

    #[test]
    fn paste_url_over_selection_in_code_buffer_is_a_normal_replace() {
        // A `.rs` buffer is NOT markdown: a URL over a selection stays a plain
        // replace — never `[x](url)` in code.
        let mut buf = b("the essay");
        buf.set_path(std::path::PathBuf::from("/tmp/x.rs"));
        assert!(!buf.is_markdown());
        buf.set_kill("https://example.com");
        buf.select_range(0, 9);
        buf.yank();
        assert_eq!(buf.text(), "https://example.com");
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

    // --- UNDO/REDO ROUNDTRIP INVARIANT: a deterministic mixed-op script --------
    //
    // The coalescing suite above pins the GROUPING rules one op-shape at a time;
    // this pins the ENGINE's global invariant over a mixed script: undo-to-bottom
    // recovers the original document, redo-to-top recovers the final one, the redo
    // walk retraces the undo trajectory state-for-state, and interleaved undo/redo
    // at arbitrary depths always lands on a trajectory state. Table-driven and
    // fully deterministic — no clock, no randomness.

    /// One scripted operation for the roundtrip invariant. Pure `Buffer` calls
    /// only, chosen to cover every edit family the engine records: coalescing
    /// self-inserts (incl. multi-byte unicode), backspace/forward-delete runs,
    /// kill-line + yank, the atomic replace seams, and the motion-seal the app
    /// performs between edit bursts.
    enum ScriptOp {
        /// Type `str` char-by-char (self-insert, coalescing as live typing would).
        Type(&'static str),
        /// Backspace N times (a coalescing delete run).
        Backspace(usize),
        /// C-d N times (a coalescing forward-delete run).
        DeleteForward(usize),
        /// C-k at the current cursor.
        KillLine,
        /// C-y the current kill buffer (its own atomic group).
        Yank,
        /// Replace char range [a, b) with text (the spell-picker seam; atomic).
        Replace(usize, usize, &'static str),
        /// MOTION-SEAL: seal the open undo group + park the cursor at `idx`
        /// (clamped) — exactly what the app does between edit bursts.
        Seal(usize),
        /// Select [a, b) (clamped) then type `c` over it — an atomic
        /// selection-replace edit.
        SelectType(usize, usize, char),
    }

    fn run_script_op(buf: &mut Buffer, op: &ScriptOp) {
        match op {
            ScriptOp::Type(s) => {
                for c in s.chars() {
                    buf.insert_char(c);
                }
            }
            ScriptOp::Backspace(n) => {
                for _ in 0..*n {
                    buf.delete_backward();
                }
            }
            ScriptOp::DeleteForward(n) => {
                for _ in 0..*n {
                    buf.delete_forward();
                }
            }
            ScriptOp::KillLine => buf.kill_line(),
            ScriptOp::Yank => buf.yank(),
            ScriptOp::Replace(a, b, s) => buf.replace_char_range(*a, *b, s),
            ScriptOp::Seal(idx) => {
                buf.seal_undo_group();
                buf.set_cursor(*idx);
            }
            ScriptOp::SelectType(a, b, c) => {
                buf.select_range(*a, *b);
                buf.insert_char(*c);
            }
        }
    }

    #[test]
    fn mixed_op_script_undo_redo_roundtrip_invariant() {
        // The one deterministic script, replayed over TWO starting documents (the
        // empty scratch and a multi-line unicode doc) so the invariant holds from
        // both a cold start and mid-document surgery.
        let script: &[ScriptOp] = &[
            ScriptOp::Type("héllo wörld"),
            ScriptOp::Seal(5),
            ScriptOp::Type(" 日本語🦘"),
            ScriptOp::Seal(0),
            ScriptOp::KillLine,
            ScriptOp::Yank,
            ScriptOp::Seal(3),
            ScriptOp::DeleteForward(2),
            ScriptOp::Type("mixed ops\nsecond line"),
            ScriptOp::Backspace(4),
            ScriptOp::SelectType(1, 6, 'X'),
            ScriptOp::Replace(0, 3, "swapped—"),
            ScriptOp::Yank,
        ];
        for start in ["", "alpha béta\nガンマ delta\nepsilon\n"] {
            let mut buf = b(start);
            // Snapshot the text after EVERY op — the op-boundary states.
            let mut op_snaps: Vec<String> = vec![buf.text()];
            for op in script {
                run_script_op(&mut buf, op);
                op_snaps.push(buf.text());
            }
            let final_text = buf.text();
            assert_ne!(final_text, start, "the script must actually edit");

            // UNDO TO BOTTOM, recording the full trajectory (top state first).
            let mut down: Vec<String> = vec![final_text.clone()];
            while buf.can_undo() {
                buf.undo();
                down.push(buf.text());
            }
            assert_eq!(buf.text(), start, "undo-to-bottom restores the original document");
            assert_eq!(buf.cursor_char(), 0, "the cursor rides back to its pre-script seat");
            assert!(!buf.can_undo());

            // REDO TO TOP retraces the SAME trajectory in reverse, state-for-state.
            let mut pos = down.len() - 1; // index into `down` of the current state
            while buf.can_redo() {
                buf.redo();
                pos -= 1;
                assert_eq!(buf.text(), down[pos], "each redo step retraces the undo trajectory");
            }
            assert_eq!(pos, 0, "redo drains back to the top");
            assert_eq!(buf.text(), final_text, "redo-to-top restores the final document");

            // Every OP-BOUNDARY snapshot appears ON the trajectory, in order (an
            // op may contribute several groups — whitespace seals, yank atomicity
            // — so the trajectory has extra INTRA-op states between them).
            let up: Vec<&String> = down.iter().rev().collect(); // original → final
            let mut j = 0usize;
            for snap in &op_snaps {
                while j < up.len() && up[j] != snap {
                    j += 1;
                }
                assert!(
                    j < up.len(),
                    "op-boundary state {snap:?} missing from the undo/redo trajectory"
                );
            }

            // INTERLEAVED undo/redo at several depths: walk a deterministic dance
            // from the top, tracking the expected trajectory index — every stop
            // must land exactly on the recorded state.
            let bottom = down.len() - 1;
            let mut pos = 0usize; // 0 == top (final text)
            for &(u, r) in &[(3usize, 1usize), (5, 2), (2, 4), (bottom, bottom)] {
                for _ in 0..u {
                    if buf.can_undo() {
                        buf.undo();
                        pos += 1;
                    }
                }
                assert_eq!(buf.text(), down[pos], "after an undo run of {u}");
                for _ in 0..r {
                    if buf.can_redo() {
                        buf.redo();
                        pos -= 1;
                    }
                }
                assert_eq!(buf.text(), down[pos], "after a redo run of {r}");
            }
        }
    }

    // --- LINE ENDINGS (VS Code model): normalize-on-load, restore-on-save ------
    //
    // RESOLVED (was the CRLF / lone-CR / U+2028 divergence). ropey now counts
    // LF-ONLY — its `unicode_lines`/`cr_lines` features are OFF (Cargo.toml), so
    // `len_lines`/`char_to_line`/`line_to_char` recognize a break at '\n' and
    // NOWHERE else. `Buffer::from_file` NORMALIZES every '\r\n' to '\n' before the
    // text enters the rope while remembering the file's `Eol`, and a save restores
    // it ([`Buffer::disk_bytes`]). Two consequences, both proven below:
    //   (a) the buffer is purely '\n'-based, so it AGREES with the '\n'-only
    //       renderer by construction — no CRLF/lone-CR line-model divergence;
    //   (b) a lone '\r' / NEL / LS / PS is ordinary CONTENT, never a line break.
    // A CRLF file therefore round-trips byte-for-byte; a lone '\r' is preserved
    // verbatim inside its line. (`from_str` — a raw, un-normalizing constructor —
    // is characterized separately: a '\r' forced in that way is now CONTENT too,
    // since counting is LF-only.)

    #[test]
    fn eol_detect_picks_the_dominant_ending() {
        assert_eq!(Eol::detect(""), Eol::Lf, "empty file → LF default");
        assert_eq!(Eol::detect("no newline at all"), Eol::Lf);
        assert_eq!(Eol::detect("a\nb\nc\n"), Eol::Lf);
        assert_eq!(Eol::detect("a\r\nb\r\nc\r\n"), Eol::Crlf);
        // MIXED: the MAJORITY ending wins. 3 CRLF vs 1 lone LF → CRLF.
        assert_eq!(Eol::detect("a\r\nb\r\nc\r\nd\ne"), Eol::Crlf);
        // MIXED: 1 CRLF vs 2 lone LF → LF (CRLF is the minority).
        assert_eq!(Eol::detect("a\r\nb\nc\nd"), Eol::Lf);
        // A TIE falls to LF, the conservative default.
        assert_eq!(Eol::detect("a\r\nb\n"), Eol::Lf);
        // A lone '\r' is NOT a '\r\n' pair — it never counts toward CRLF.
        assert_eq!(Eol::detect("a\rb\nc\n"), Eol::Lf);
    }

    #[test]
    fn raw_crlf_via_from_str_counts_lf_only_cr_is_content() {
        // `from_str` does NOT normalize (only `from_file` does), so it can force a
        // '\r' into the rope. LF-only counting means that '\r' is CONTENT, not a
        // break: "abc\r\ndef\r\n" is 3 lines (the two '\n'), and line 0 is "abc\r".
        let mut buf = b("abc\r\ndef\r\n");
        assert_eq!(buf.line_count(), 3, "LF-only: the two '\\n' make 3 lines");

        // C-e on line 0 runs to just before the '\n' (past the content '\r').
        buf.line_end_motion();
        assert_eq!(buf.cursor_char(), 4);
        assert_eq!(buf.cursor_line_col(), (0, 4));

        // Typing there does NOT create a new line — the '\r' is inert content, so
        // the count stays 3 (the pre-fix model wrongly made the orphaned CR a
        // break and reported 4). This is the resolved divergence.
        buf.insert_char('X');
        assert_eq!(buf.text(), "abc\rX\ndef\r\n");
        assert_eq!(buf.line_count(), 3, "the content '\\r' is never a break");
    }

    #[test]
    fn lf_file_loads_and_saves_byte_identical() {
        // REGRESSION: a Unix-ended file is unchanged in every respect — detected
        // LF, rope byte-for-byte, and re-saved byte-for-byte (the pre-round path).
        use crate::fs::FileSystem;
        use std::sync::Arc;
        let path = std::path::PathBuf::from("/docs/unix.md");
        let raw = "alpha\nbeta\ngamma\n";
        let mem = crate::fs::InMemoryFs::new().with_file(&path, raw);
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            let mut buf = Buffer::from_file(&path);
            assert_eq!(buf.eol(), Eol::Lf);
            assert_eq!(buf.text(), raw, "LF rope is byte-identical");
            buf.save().unwrap();
            assert_eq!(mem.read(&path).unwrap(), raw.as_bytes(), "LF save unchanged");
        });
    }

    #[test]
    fn crlf_file_normalizes_on_load_and_round_trips_byte_for_byte() {
        // The headline: a Windows-ended file loads with a PURELY '\n' rope (no CR
        // survives) tagged `Eol::Crlf`, and a save restores '\r\n' so the on-disk
        // bytes are IDENTICAL to what was loaded.
        use crate::fs::FileSystem;
        use std::sync::Arc;
        let path = std::path::PathBuf::from("/docs/win.md");
        let raw = "# Title\r\nline two\r\nline three\r\n";
        let mem = crate::fs::InMemoryFs::new().with_file(&path, raw);
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            let mut buf = Buffer::from_file(&path);
            assert_eq!(buf.eol(), Eol::Crlf, "detected CRLF");
            assert_eq!(buf.text(), "# Title\nline two\nline three\n", "rope is LF-only");
            assert!(!buf.text().contains('\r'), "no CR ever enters the rope");
            // Buffer and the '\n'-only renderer now agree: 4 lines (trailing '\n').
            assert_eq!(buf.line_count(), 4);
            buf.save().unwrap();
            assert_eq!(
                mem.read(&path).unwrap(),
                raw.as_bytes(),
                "CRLF round-trips byte-for-byte"
            );
        });
    }

    #[test]
    fn mixed_eol_file_picks_dominant_and_normalizes_all_lines() {
        // 3 CRLF vs 1 lone LF → dominant CRLF. On load, EVERY ending (the lone LF
        // included) becomes '\n' in the rope; on save, EVERY line is re-emitted
        // CRLF — a VS Code-style normalize (so it deliberately does NOT preserve
        // the original minority '\n').
        use crate::fs::FileSystem;
        use std::sync::Arc;
        let path = std::path::PathBuf::from("/docs/mixed.md");
        let raw = "a\r\nb\r\nc\r\nd\ne";
        let mem = crate::fs::InMemoryFs::new().with_file(&path, raw);
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            let mut buf = Buffer::from_file(&path);
            assert_eq!(buf.eol(), Eol::Crlf, "CRLF is the majority");
            assert_eq!(buf.text(), "a\nb\nc\nd\ne", "all endings normalized to LF");
            assert!(!buf.text().contains('\r'));
            buf.save().unwrap();
            assert_eq!(
                mem.read(&path).unwrap(),
                b"a\r\nb\r\nc\r\nd\r\ne",
                "save re-emits CRLF uniformly (normalize, not preserve)"
            );
        });
    }

    #[test]
    fn lone_cr_nel_ls_ps_are_content_not_line_breaks() {
        // The documented lone-CR (and NEL / LS / PS) decision: these are CONTENT,
        // never breaks — the buffer now AGREES with the '\n'-only renderer instead
        // of diverging (the pre-round model counted each as its own break).
        for sep in ["\r", "\u{0085}", "\u{2028}", "\u{2029}"] {
            let text = format!("ab{sep}cd");
            let mut buf = b(&text);
            assert_eq!(buf.line_count(), 1, "{sep:?} is content, so one line");
            // C-e runs to the true end of the single 5-char line ("ab_cd"), past
            // the separator (which is inert content).
            buf.set_cursor(0);
            buf.line_end_motion();
            assert_eq!(buf.cursor_line_col(), (0, 5), "{sep:?}: C-e reaches col 5");
            // One C-k takes the WHOLE line, separator included (it's just content).
            let mut buf = b(&text);
            buf.kill_line();
            assert_eq!(buf.text(), "", "{sep:?}: the whole line is one kill");
            assert_eq!(buf.kill_buffer(), text, "{sep:?}: separator killed as content");
        }
    }

    #[test]
    fn lone_cr_file_is_preserved_verbatim_and_round_trips() {
        // A lone '\r' in a LOADED file stays literal content (not a CRLF signal,
        // not a break), and round-trips byte-for-byte through an LF save.
        use crate::fs::FileSystem;
        use std::sync::Arc;
        let path = std::path::PathBuf::from("/docs/cr.md");
        let raw = "ab\rcd\nef\n";
        let mem = crate::fs::InMemoryFs::new().with_file(&path, raw);
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            let mut buf = Buffer::from_file(&path);
            assert_eq!(buf.eol(), Eol::Lf, "a lone CR never signals CRLF");
            assert_eq!(buf.text(), raw, "the lone CR is preserved verbatim");
            assert_eq!(buf.line_count(), 3, "LF-only: the two '\\n' make 3 lines");
            buf.set_cursor(0);
            buf.line_end_motion();
            assert_eq!(buf.cursor_line_col(), (0, 5), "C-e runs past the content CR");
            buf.save().unwrap();
            assert_eq!(mem.read(&path).unwrap(), raw.as_bytes(), "byte-for-byte");
        });
    }

    #[test]
    fn caret_column_over_former_crlf_matches_the_lf_equivalent() {
        // The SAME document, once CRLF-ended and once LF-ended. After load both
        // ropes are pure '\n', so EVERY caret / motion result is identical — there
        // is no '\r' in the rope for the caret to land "inside".
        use crate::fs::FileSystem;
        use std::sync::Arc;
        let crlf_path = std::path::PathBuf::from("/docs/w.md");
        let lf_path = std::path::PathBuf::from("/docs/u.md");
        let mem = crate::fs::InMemoryFs::new()
            .with_file(&crlf_path, "hello\r\nworld\r\n")
            .with_file(&lf_path, "hello\nworld\n");
        crate::fs::with_fs(Arc::new(mem), || {
            let mut win = Buffer::from_file(&crlf_path);
            let mut nix = Buffer::from_file(&lf_path);
            assert_eq!(win.eol(), Eol::Crlf);
            assert_eq!(nix.eol(), Eol::Lf);
            assert_eq!(win.text(), nix.text(), "identical rope content after load");
            // C-e on line 0: both land at col 5 (end of "hello"), never on a CR.
            win.set_cursor(0);
            nix.set_cursor(0);
            win.line_end_motion();
            nix.line_end_motion();
            assert_eq!(win.cursor_line_col(), (0, 5));
            assert_eq!(win.cursor_line_col(), nix.cursor_line_col());
            assert_eq!(win.cursor_char(), nix.cursor_char(), "same absolute index");
            // Vertical motion onto line 1 also matches exactly.
            win.next_line();
            nix.next_line();
            assert_eq!(win.cursor_line_col(), nix.cursor_line_col());
            assert_eq!(win.cursor_char(), nix.cursor_char());
        });
    }

    #[test]
    fn set_eol_flips_encoding_is_metadata_not_an_undoable_edit() {
        // Switching the ending is a DOCUMENT-LEVEL setting, not a text edit: the
        // rope content is untouched, only `disk_bytes` differs. A real switch bumps
        // `version` + dirties (so the autosave engine rewrites); a no-op switch is
        // inert; and undo does NOT restore the ending (it is off the timeline).
        let mut buf = Buffer::from_str("a\nb\n");
        assert_eq!(buf.eol(), Eol::Lf);
        assert_eq!(buf.disk_bytes(), b"a\nb\n");
        let v0 = buf.version();
        buf.set_eol(Eol::Crlf);
        assert_eq!(buf.eol(), Eol::Crlf);
        assert_eq!(buf.text(), "a\nb\n", "rope content is untouched");
        assert_eq!(buf.disk_bytes(), b"a\r\nb\r\n", "on-disk encoding flipped");
        assert!(buf.is_dirty());
        assert!(buf.version() > v0, "version bumped so autosave picks it up");
        // A no-op switch (same ending) changes nothing.
        let v1 = buf.version();
        buf.set_eol(Eol::Crlf);
        assert_eq!(buf.version(), v1, "same ending: no version bump");

        // Undo reverts the TEXT edit but leaves the EOL where it was set — the
        // ending is metadata, not part of the undo history (documented choice).
        let mut buf2 = Buffer::from_str("x");
        buf2.insert_char('y'); // one undoable text edit
        buf2.set_eol(Eol::Crlf);
        buf2.undo();
        assert_eq!(buf2.text(), "x", "undo reverts the text edit");
        assert_eq!(buf2.eol(), Eol::Crlf, "EOL is metadata — undo does not restore it");
    }

    #[test]
    fn crlf_encode_is_idempotent_never_double_encodes_existing_crlf() {
        // REGRESSION (data loss, commit f953392): `disk_bytes` used to re-encode
        // '\n' -> '\r\n' assuming the rope was pure-'\n'. If a real '\r\n' reached
        // the rope by a door OTHER than `from_file` (a pasted CRLF clipboard, a
        // history restore), an `Eol::Crlf` save DOUBLE-encoded it to '\r\r\n' —
        // byte corruption. `from_str` (raw, un-normalizing) forces that state.
        let mut buf = Buffer::from_str("a\r\nb");
        buf.set_eol(Eol::Crlf);
        assert_eq!(
            buf.disk_bytes(),
            b"a\r\nb",
            "existing \\r\\n encodes to a SINGLE \\r\\n, never \\r\\r\\n"
        );
        // A pure-'\n' rope (the normal invariant) still encodes exactly once.
        let mut buf2 = Buffer::from_str("a\nb\n");
        buf2.set_eol(Eol::Crlf);
        assert_eq!(buf2.disk_bytes(), b"a\r\nb\r\n", "pure-\\n encodes as before");
        // A lone '\r' (no '\n' after it) stays CONTENT, never sprouts a '\n'.
        let mut buf3 = Buffer::from_str("a\rb\nc");
        buf3.set_eol(Eol::Crlf);
        assert_eq!(
            buf3.disk_bytes(),
            b"a\rb\r\nc",
            "lone \\r preserved; only the real break becomes \\r\\n"
        );
    }

    #[test]
    fn pasted_crlf_round_trips_to_a_single_crlf_per_line_never_doubled() {
        // The entry-door half of the fix: an external CRLF clipboard value pasted
        // (set_kill + yank) into a `Crlf` buffer must round-trip to ONE '\r\n' per
        // line on save, never '\r\r\n'. `set_kill` normalizes '\r\n' -> '\n' on the
        // way in (matching `from_file`), so the rope stays purely '\n'.
        let mut buf = Buffer::from_str("");
        buf.set_eol(Eol::Crlf);
        buf.set_kill("x\r\ny"); // an external Windows-clipboard value
        assert_eq!(buf.kill_buffer(), "x\ny", "\\r\\n normalized out of the kill ring");
        buf.yank();
        assert_eq!(buf.text(), "x\ny", "the rope is purely '\\n' after paste");
        assert!(!buf.text().contains('\r'), "no CR ever enters the rope");
        assert_eq!(
            buf.disk_bytes(),
            b"x\r\ny",
            "a single \\r\\n per line on save, never doubled"
        );
        // A lone '\r' in the pasted value is content and survives the normalize.
        let mut buf2 = Buffer::from_str("");
        buf2.set_kill("p\rq");
        assert_eq!(buf2.kill_buffer(), "p\rq", "lone \\r stays content in the kill ring");
    }

    // --- QUICK NOTE: title slug, collision suffixing, auto-name on save --------

    #[test]
    fn note_stem_titles() {
        assert_eq!(note_stem("Japanese week 12"), "japanese-week-12");
        assert_eq!(note_stem("  Hello,  World!  "), "hello-world");
        assert_eq!(note_stem("UPPER Case"), "upper-case");
        // Punctuation-only / empty -> the "scratch" fallback.
        assert_eq!(note_stem("!!!"), "scratch");
        assert_eq!(note_stem(""), "scratch");
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

    // --- SAVE-FEEDBACK round: `Buffer::save_as_note` (scratch -> note on manual save) ---

    #[test]
    fn save_as_note_converts_a_true_scratch_buffer_and_writes_it() {
        use crate::fs::FileSystem;
        use std::sync::Arc;
        let notes = std::path::PathBuf::from("/notes");
        let mem = crate::fs::InMemoryFs::new(); // notes dir does NOT exist yet
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            let mut buf = Buffer::scratch();
            for c in "brain dump".chars() {
                buf.insert_char(c);
            }
            assert!(!buf.is_note(), "a true scratch buffer starts as no note");
            buf.save_as_note(&notes).unwrap();
            assert!(buf.is_note(), "save_as_note promotes it to a note");
            let p = buf.path().unwrap();
            assert_eq!(p.file_name().unwrap(), "brain-dump.md");
            assert!(p.starts_with(&notes));
            assert!(mem.exists(p), "the notes_root dir was created and the file written");
        });
    }

    #[test]
    fn save_as_note_second_call_is_a_plain_save_same_path() {
        use crate::fs::FileSystem;
        use std::sync::Arc;
        let notes = std::path::PathBuf::from("/notes");
        let mem = crate::fs::InMemoryFs::new().with_dir(&notes);
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            let mut buf = Buffer::scratch();
            for c in "first draft".chars() {
                buf.insert_char(c);
            }
            buf.save_as_note(&notes).unwrap();
            let named = buf.path().unwrap().to_path_buf();
            for c in " continued".chars() {
                buf.insert_char(c);
            }
            // A SECOND save (now that it's a real note) never re-derives or
            // re-homes the filename — it's a plain `save()` at the same path.
            buf.save_as_note(&notes).unwrap();
            assert_eq!(buf.path().unwrap(), named);
            assert_eq!(mem.read_to_string(&named).unwrap(), "first draft continued");
        });
    }

    #[test]
    fn save_as_note_already_a_note_is_untouched_by_the_conversion_step() {
        // A buffer that is ALREADY a note (e.g. C-x n) keeps its OWN note_dir —
        // `save_as_note` must never re-home it at the passed-in `notes_root`.
        use std::sync::Arc;
        let own_dir = std::path::PathBuf::from("/project/scratch-notes");
        let other_notes_root = std::path::PathBuf::from("/notes");
        let mem = crate::fs::InMemoryFs::new().with_dir(&own_dir).with_dir(&other_notes_root);
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            let mut buf = Buffer::scratch();
            buf.start_note(own_dir.clone());
            for c in "already a note".chars() {
                buf.insert_char(c);
            }
            buf.save_as_note(&other_notes_root).unwrap();
            assert!(buf.path().unwrap().starts_with(&own_dir), "kept its own note home");
        });
    }

    /// A minimal [`crate::fs::FileSystem`] fake whose `write` ALWAYS fails —
    /// standing in for a `notes_root` that exists but isn't writable (a full
    /// disk, a permissions error, …). `InMemoryFs` has no such mode (every
    /// write always succeeds), so this is the smallest fake that can exercise
    /// the failure path `Buffer::save`'s `write_atomic` call can genuinely
    /// take. Every other method is a total no-op / `NotFound` — nothing this
    /// test needs reads through them.
    struct UnwritableFs;
    impl crate::fs::FileSystem for UnwritableFs {
        fn read_to_string(&self, _path: &std::path::Path) -> std::io::Result<String> {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "unwritable fake"))
        }
        fn read(&self, _path: &std::path::Path) -> std::io::Result<Vec<u8>> {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "unwritable fake"))
        }
        fn write(&self, _path: &std::path::Path, _data: &[u8]) -> std::io::Result<()> {
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "notes_root unwritable"))
        }
        fn create_dir_all(&self, _path: &std::path::Path) -> std::io::Result<()> {
            Ok(()) // "creating" the dir succeeds; the WRITE into it is what fails
        }
        fn rename(&self, _from: &std::path::Path, _to: &std::path::Path) -> std::io::Result<()> {
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "notes_root unwritable"))
        }
        fn exists(&self, _path: &std::path::Path) -> bool {
            false
        }
        fn is_dir(&self, _path: &std::path::Path) -> bool {
            false
        }
        fn read_dir(&self, _path: &std::path::Path) -> std::io::Result<Vec<crate::fs::DirEntry>> {
            Ok(vec![])
        }
        fn metadata(&self, _path: &std::path::Path) -> std::io::Result<crate::fs::Metadata> {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "unwritable fake"))
        }
        fn remove_file(&self, _path: &std::path::Path) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn save_as_note_unwritable_notes_root_surfaces_as_an_err_never_panics() {
        // A `notes_root` that exists but can't be WRITTEN to surfaces the
        // failure as the same `Err` `save` already returns — the caller
        // (`App::convert_scratch_and_save`) turns it into a calm notice,
        // never a terminal print, never a panic.
        use std::sync::Arc;
        let notes = std::path::PathBuf::from("/notes");
        crate::fs::with_fs(Arc::new(UnwritableFs), || {
            let mut buf = Buffer::scratch();
            for c in "will not land".chars() {
                buf.insert_char(c);
            }
            assert!(buf.save_as_note(&notes).is_err());
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
    fn page_class_mirrors_syntax_lang_presence() {
        // The prose/code page-width split (`crate::page::PageClass`): a recognized
        // CODE file is `Code`, everything else — markdown, an unrecognized plain-text
        // file, or the no-path scratch surface — is `Prose`. Mirrors
        // `syntax_lang_gates_code_only` exactly, since `page_class` is defined in
        // terms of `syntax_lang`.
        let mut code = Buffer::from_str("fn main() {}");
        code.set_path("/p/main.rs".into());
        assert_eq!(code.page_class(), crate::page::PageClass::Code);

        let mut md = Buffer::from_str("# heading");
        md.set_path("/p/notes.md".into());
        assert_eq!(md.page_class(), crate::page::PageClass::Prose);

        let mut txt = Buffer::from_str("plain prose");
        txt.set_path("/p/notes.txt".into());
        assert_eq!(txt.page_class(), crate::page::PageClass::Prose);

        let scratch = Buffer::from_str("scratch");
        assert_eq!(scratch.page_class(), crate::page::PageClass::Prose);
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
