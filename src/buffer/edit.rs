//! TEXT EDITING OPS — the mutating commands, all routed through
//! [`Buffer::apply_edit`]: self-insert, newline, the soft tab, the markdown
//! smart-newline primitive (`replace_before_cursor`), range replacement, the
//! backward / forward / word deletes, C-k kill-line (logical + the visual
//! `kill_line_to`), and C-y yank. Carved out of `buffer.rs` verbatim — inherent
//! methods on [`Buffer`].

use super::{is_word_char, Buffer};

impl Buffer {
    // --- Editing ----------------------------------------------------------

    pub fn insert_char(&mut self, c: char) {
        self.clear_kill_flag();
        self.goal_col = None;
        let before = self.cursor;
        // An active selection is replaced by the typed character: the deletion +
        // insertion form ONE atomic edit (one undo restores the original text).
        if let Some((start, end)) = self.selection_range() {
            self.anchor = None;
            let mut s = String::new();
            s.push(c);
            self.apply_edit(start, end - start, &s, before, start + 1);
        } else {
            self.anchor = None;
            let mut s = String::new();
            s.push(c);
            self.apply_edit(self.cursor, 0, &s, before, before + 1);
        }
    }

    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    /// Tab: insert spaces up to the next tab stop (soft tabs, 4-wide) as ONE
    /// atomic edit, so a single undo removes the whole indent. Replaces an active
    /// selection like a normal insert. Tab stops are measured in chars from the
    /// line start (fine for the ASCII/prose v1; wide glyphs are a later refinement).
    pub fn insert_tab(&mut self) {
        const TAB_WIDTH: usize = 4;
        self.clear_kill_flag();
        self.goal_col = None;
        let before = self.cursor;
        let sel = self.selection_range();
        let start = sel.map(|(s, _)| s).unwrap_or(self.cursor);
        let (_, col) = self.char_to_line_col(start);
        let k = TAB_WIDTH - (col % TAB_WIDTH);
        let spaces = " ".repeat(k);
        if let Some((s, e)) = sel {
            self.anchor = None;
            self.apply_edit(s, e - s, &spaces, before, s + k);
        } else {
            self.anchor = None;
            self.apply_edit(self.cursor, 0, &spaces, before, before + k);
        }
    }

    /// Smart-input primitive for the markdown Enter path: as ONE atomic edit
    /// (one undo step), remove the `remove_before` chars immediately before the
    /// cursor and insert `insert` in their place, leaving the cursor after the
    /// inserted text. Used by `actions::smart_newline` to either insert a "\n" +
    /// continuation prefix (`remove_before == 0`) or strip a dangling list /
    /// blockquote marker when ending a block (`insert == ""`). An active selection
    /// is overwritten first, like the other self-inserts.
    pub fn replace_before_cursor(&mut self, remove_before: usize, insert: &str) {
        self.clear_kill_flag();
        self.goal_col = None;
        let before = self.cursor;
        if let Some((start, end)) = self.selection_range() {
            self.anchor = None;
            let after = start + insert.chars().count();
            self.apply_edit(start, end - start, insert, before, after);
            return;
        }
        self.anchor = None;
        let rb = remove_before.min(before);
        let start = before - rb;
        let after = start + insert.chars().count();
        self.apply_edit(start, rb, insert, before, after);
    }

    /// Replace the char range `[start, end)` with `text` as ONE atomic, UNDOABLE
    /// edit (a replacement never coalesces, so a single undo restores the original
    /// text), leaving the cursor just after the inserted text. Used by the
    /// spell-suggest picker to swap a misspelled word for the chosen correction.
    /// Clamps both ends to the rope and drops any active selection/mark, like the
    /// other self-inserts.
    pub fn replace_char_range(&mut self, start: usize, end: usize, text: &str) {
        self.clear_kill_flag();
        self.goal_col = None;
        let len = self.rope.len_chars();
        let start = start.min(len);
        let end = end.min(len).max(start);
        self.anchor = None;
        let before = self.cursor;
        let after = start + text.chars().count();
        self.apply_edit(start, end - start, text, before, after);
    }

    /// Backspace: delete the char before the cursor. With an active selection,
    /// delete the selection instead (modern editor behavior).
    pub fn delete_backward(&mut self) {
        self.clear_kill_flag();
        self.goal_col = None;
        if self.delete_selection() {
            return;
        }
        if self.cursor > 0 {
            let before = self.cursor;
            self.apply_edit(self.cursor - 1, 1, "", before, before - 1);
        }
    }

    /// M-Backspace / Option-Backspace: delete the word before the cursor (into
    /// the kill buffer, so C-y can bring it back). With an active selection,
    /// delete that instead.
    pub fn delete_word_backward(&mut self) {
        self.goal_col = None;
        if self.delete_selection() {
            self.last_was_kill = false;
            return;
        }
        let mut i = self.cursor;
        while i > 0 && !is_word_char(self.rope.char(i - 1)) {
            i -= 1;
        }
        while i > 0 && is_word_char(self.rope.char(i - 1)) {
            i -= 1;
        }
        if i < self.cursor {
            self.kill = self.rope.slice(i..self.cursor).to_string();
            let before = self.cursor;
            // A word kill is its own atomic undo group (whitespace-bounded).
            self.seal_undo_group();
            self.apply_edit(i, self.cursor - i, "", before, i);
            self.seal_undo_group();
        }
        self.last_was_kill = false;
    }

    /// C-d: delete the char at the cursor. With an active selection, delete the
    /// selection instead.
    pub fn delete_forward(&mut self) {
        self.clear_kill_flag();
        self.goal_col = None;
        if self.delete_selection() {
            return;
        }
        if self.cursor < self.rope.len_chars() {
            let before = self.cursor;
            self.apply_edit(self.cursor, 1, "", before, before);
        }
    }

    /// C-k: kill to end of line. If the cursor is already at end of line, kill
    /// the newline (joining the next line). Consecutive kills append to the kill
    /// buffer rather than replacing it.
    pub fn kill_line(&mut self) {
        self.goal_col = None;
        let (line, _) = self.cursor_line_col();
        let line_end_no_nl = self.line_start(line) + self.line_len(line);
        let killed: String;
        let end;
        if self.cursor < line_end_no_nl {
            // Kill to end of line (not including newline).
            end = line_end_no_nl;
        } else if self.cursor < self.rope.len_chars() {
            // At end of line: kill the newline itself.
            end = self.cursor + 1;
        } else {
            // End of buffer: nothing to kill.
            self.last_was_kill = true;
            return;
        }
        killed = self.rope.slice(self.cursor..end).to_string();
        if self.last_was_kill {
            self.kill.push_str(&killed);
        } else {
            self.kill = killed;
        }
        let before = self.cursor;
        // Each C-k is a forward-delete at the cursor; consecutive kills coalesce
        // into one undo group (they share the same start), but they never merge
        // with a preceding insertion run.
        if !self.last_was_kill {
            self.seal_undo_group();
        }
        self.apply_edit(self.cursor, end - before, "", before, before);
        self.last_was_kill = true;
    }

    /// VISUAL C-k: kill from the cursor to char index `end` — the end of the
    /// current VISUAL row, supplied by `apply_core`'s layout oracle. If the cursor
    /// is already AT (or past) `end`, fall back to [`Self::kill_line`] so C-k still
    /// kills the trailing newline and joins the next line, exactly as in logical
    /// mode. Because a soft-wrap boundary biases the caret onto the FOLLOWING
    /// visual row (its `start_col` == the prior row's `end_col`), the cursor only
    /// equals the row end at the LOGICAL line's end — so the join branch fires
    /// precisely where today's C-k would. Shares the kill-coalescing + undo-group
    /// machinery with [`Self::kill_line`].
    pub fn kill_line_to(&mut self, end: usize) {
        if self.cursor >= end {
            return self.kill_line();
        }
        self.goal_col = None;
        let before = self.cursor;
        let killed = self.rope.slice(before..end).to_string();
        if self.last_was_kill {
            self.kill.push_str(&killed);
        } else {
            self.kill = killed;
        }
        if !self.last_was_kill {
            self.seal_undo_group();
        }
        self.apply_edit(before, end - before, "", before, before);
        self.last_was_kill = true;
    }

    /// C-y: yank (insert) the kill buffer at the cursor. An active selection is
    /// replaced by the yanked text.
    pub fn yank(&mut self) {
        self.clear_kill_flag();
        self.goal_col = None;
        let before = self.cursor;
        let s = self.kill.clone();
        // Replace an active selection with the yanked text as ONE atomic edit.
        if let Some((start, end)) = self.selection_range() {
            self.anchor = None;
            if s.is_empty() {
                // Nothing to yank: still delete the selection (as its own edit).
                self.seal_undo_group();
                self.apply_edit(start, end - start, "", before, start);
                self.seal_undo_group();
            } else {
                let after = start + s.chars().count();
                self.seal_undo_group();
                self.apply_edit(start, end - start, &s, before, after);
                self.seal_undo_group();
            }
        } else {
            self.anchor = None;
            if s.is_empty() {
                return;
            }
            let after = before + s.chars().count();
            // A yank is an atomic group, not coalesced with adjacent typing.
            self.seal_undo_group();
            self.apply_edit(before, 0, &s, before, after);
            self.seal_undo_group();
        }
    }
}
