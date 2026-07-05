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

    /// TAB list-indent: add one nesting level ([`crate::markdown::LIST_INDENT`] = 2
    /// leading spaces) to the caret line, or to EVERY line of an active selection, as
    /// ONE atomic, undoable edit. The bullet glyph is depth-derived, so it re-cycles
    /// automatically. Truly empty lines are skipped (no trailing-space litter). The
    /// cursor + selection are remapped so the region stays over the same text.
    pub fn indent_lines(&mut self) {
        self.reindent(false);
    }

    /// SHIFT-TAB outdent: remove up to one nesting level (2 leading spaces, clamped at
    /// 0) from the caret line or every selected line, as ONE atomic, undoable edit —
    /// the reverse of [`Self::indent_lines`]. A no-op (no version bump) when no line
    /// has any leading space to strip.
    pub fn outdent_lines(&mut self) {
        self.reindent(true);
    }

    /// Shared indent / outdent engine. Determines the affected logical line range
    /// (the caret line, or every line a selection touches — a selection ending at
    /// column 0 does NOT pull in that trailing line), rebuilds those lines with
    /// [`crate::markdown::LIST_INDENT`] spaces added (`outdent == false`) or removed
    /// (`true`, clamped), and applies the whole block as ONE atomic replace so a
    /// single undo restores it. The cursor and selection anchor are remapped by each
    /// line's own delta, so a block selection stays over the same lines/columns. A
    /// no-change pass (e.g. outdenting lines with no indent) returns without touching
    /// the buffer, so recoil / version stay clean.
    fn reindent(&mut self, outdent: bool) {
        self.clear_kill_flag();
        self.goal_col = None;
        let step = crate::markdown::LIST_INDENT;
        let (start_line, end_line) = match self.selection_range() {
            Some((s, e)) => {
                let (l0, _) = self.char_to_line_col(s);
                let (mut l1, c1) = self.char_to_line_col(e);
                if c1 == 0 && l1 > l0 {
                    l1 -= 1; // a selection ending at col 0 excludes that line
                }
                (l0, l1)
            }
            None => {
                let (l, _) = self.cursor_line_col();
                (l, l)
            }
        };

        let block_start = self.line_start(start_line);
        let block_end = self.line_start(end_line) + self.line_len(end_line);
        // Per-line CHAR delta (+step / −removed), and the rebuilt block text.
        let mut deltas: Vec<isize> = Vec::with_capacity(end_line - start_line + 1);
        let mut new_block = String::new();
        for line in start_line..=end_line {
            let text = self.line_text(line);
            let (out, delta): (String, isize) = if outdent {
                let lead = text.chars().take_while(|&c| c == ' ').count();
                let remove = lead.min(step);
                (text[remove..].to_string(), -(remove as isize))
            } else if text.is_empty() {
                (text, 0)
            } else {
                (format!("{}{text}", " ".repeat(step)), step as isize)
            };
            deltas.push(delta);
            new_block.push_str(&out);
            if line < end_line {
                new_block.push('\n');
            }
        }
        if deltas.iter().all(|&d| d == 0) {
            return; // nothing to do — keep version / recoil untouched
        }

        // Remap a pre-edit char index into the post-edit buffer.
        let remap = |p: usize| -> usize {
            if p <= block_start {
                return p;
            }
            let (lp, cp) = self.char_to_line_col(p);
            if lp < start_line {
                return p;
            }
            if lp > end_line {
                let total: isize = deltas.iter().sum();
                return (p as isize + total) as usize;
            }
            let base: isize = deltas[..lp - start_line].iter().sum();
            let d = deltas[lp - start_line];
            if d >= 0 {
                (p as isize + base + d) as usize
            } else {
                let removed = (-d) as usize;
                let new_col = cp - cp.min(removed);
                (self.line_start(lp) as isize + base) as usize + new_col
            }
        };

        let before = self.cursor;
        let new_cursor = remap(self.cursor);
        let new_anchor = self.anchor.map(remap);
        self.apply_edit(block_start, block_end - block_start, &new_block, before, new_cursor);
        self.anchor = new_anchor;
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

    /// M-d: delete the word AFTER the cursor (into the kill buffer, so C-y can
    /// bring it back) — the forward mirror of [`Self::delete_word_backward`],
    /// removing exactly what [`Self::forward_word`] (M-f) would move over: skip a
    /// run of non-word chars, then the word. Char/grapheme-safe (indices are char
    /// indices into the rope) and a clean NO-OP at end-of-buffer. With an active
    /// selection, delete that instead.
    pub fn delete_word_forward(&mut self) {
        self.goal_col = None;
        if self.delete_selection() {
            self.last_was_kill = false;
            return;
        }
        let len = self.rope.len_chars();
        let mut j = self.cursor;
        while j < len && !is_word_char(self.rope.char(j)) {
            j += 1;
        }
        while j < len && is_word_char(self.rope.char(j)) {
            j += 1;
        }
        if j > self.cursor {
            self.kill = self.rope.slice(self.cursor..j).to_string();
            let before = self.cursor;
            // A word kill is its own atomic undo group (whitespace-bounded).
            self.seal_undo_group();
            // The cursor stays put; the text to its right collapses to meet it.
            self.apply_edit(self.cursor, j - self.cursor, "", before, before);
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
        // C-k deactivates the region (Emacs semantics). Clearing here also
        // prevents a BACKWARD mark (anchor past the cursor) from dangling past
        // the rope's new end after the kill shrinks it — an out-of-bounds slice
        // in the next selection-consuming op.
        self.anchor = None;
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
        // C-k deactivates the region (see [`Self::kill_line`]) — also clears a
        // dangling BACKWARD mark so it can't slice past the shrunk rope later.
        self.anchor = None;
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
