//! TEXT EDITING OPS — the mutating commands, all routed through
//! [`Buffer::apply_edit`]: self-insert, newline, the soft tab, the markdown
//! smart-newline primitive (`replace_before_cursor`), range replacement, the
//! backward / forward / word deletes, C-k kill-line (logical + the visual
//! `kill_line_to`), and C-y yank. Carved out of `buffer.rs` verbatim — inherent
//! methods on [`Buffer`].

use super::Buffer;

/// URL-SHAPE test for the paste-URL-over-selection → markdown-link convention.
/// Deliberately simple + conservative (documented shape, not a validator): the
/// string must be a single `scheme://…` token — a `://` after an ASCII-alpha
/// scheme (`http`, `https`, `ftp`, …), no interior whitespace, and non-empty
/// authority text after the `://`. Anything else (plain prose, a bare filesystem
/// path, a scheme with nothing after `://`, a multi-line clipboard) is NOT a URL,
/// so the paste stays a normal replace.
pub fn is_url(s: &str) -> bool {
    // No surrounding or interior whitespace — a URL is one bare token.
    if s.is_empty() || s.chars().any(char::is_whitespace) {
        return false;
    }
    let Some(scheme_end) = s.find("://") else {
        return false;
    };
    let scheme = &s[..scheme_end];
    // A non-empty scheme of ASCII letters (RFC-ish: letter-led; we keep it to
    // pure letters, which covers http/https/ftp/mailto-less real cases).
    if scheme.is_empty() || !scheme.bytes().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    // Something must follow `://` (a host) — reject a bare `http://`.
    !s[scheme_end + 3..].is_empty()
}

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

    /// Insert literal `s` at the cursor as ONE atomic, undoable edit —
    /// replacing an active selection exactly like [`Self::insert_char`], but
    /// SEALED on both sides (mirrors [`Self::apply_format`]'s own sealing
    /// discipline) so it never coalesces with adjacent typing: a single
    /// Cmd-Z removes exactly `s`, regardless of whether the user was
    /// mid-typing right before or after. A plain multi-char sibling of
    /// `insert_char` (char-by-char) for a DISCRETE, non-typing insert — used
    /// by "Insert Date" (`App::insert_date` / the headless replay's
    /// `Effect::InsertDate` arm) to land the formatted date string. A no-op
    /// for an empty `s` (nothing to insert, nothing to seal around).
    pub fn insert_text(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.clear_kill_flag();
        self.goal_col = None;
        let before = self.cursor;
        self.seal_undo_group();
        if let Some((start, end)) = self.selection_range() {
            self.anchor = None;
            self.apply_edit(start, end - start, s, before, start + s.chars().count());
        } else {
            self.anchor = None;
            self.apply_edit(self.cursor, 0, s, before, before + s.chars().count());
        }
        self.seal_undo_group();
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

    /// MARKDOWN FORMAT TOGGLE: replace the ENTIRE buffer with `new_text` as ONE
    /// atomic, undoable edit (sealed on both sides so it never coalesces with adjacent
    /// typing — a single Cmd-Z reverts the toggle), then restore the selection to
    /// `[anchor, cursor]` over the new text. A NO-OP when `new_text` equals the current
    /// text (no edit is recorded, so the undo history stays meaningful — mirrors the
    /// align-table command). The pure transform lives in `actions::format`; this is the
    /// buffer seam it applies through.
    pub fn apply_format(&mut self, new_text: &str, anchor: Option<usize>, cursor: usize) {
        self.clear_kill_flag();
        self.goal_col = None;
        if new_text == self.rope.to_string() {
            return; // nothing changed — keep the timeline meaningful
        }
        let before = self.cursor;
        let len = self.rope.len_chars();
        self.anchor = None;
        // Seal on both sides so the whole toggle is exactly one undo group even when
        // the buffer was empty (a bare insert could otherwise coalesce with prior typing).
        self.seal_undo_group();
        self.apply_edit(0, len, new_text, before, cursor);
        self.seal_undo_group();
        // Restore the selection over the freshly-applied text.
        let max = self.rope.len_chars();
        self.cursor = cursor.min(max);
        self.anchor = anchor.map(|a| a.min(max));
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
        let i =
            super::word_delete_backward_boundary(self.cursor, |k| self.rope.char(k));
        if i < self.cursor {
            let killed = self.rope.slice(i..self.cursor).to_string();
            // Consecutive word-kills ACCUMULATE into the kill ring (the same
            // append precedent `kill_line` sets), so C-y brings back EVERY word
            // killed in a run, not just the last. A BACKWARD kill removes text to
            // the LEFT of the prior one, so it PREPENDS to keep the ring in
            // reading order (Emacs kill-ring accumulation).
            if self.last_was_kill {
                let mut acc = killed;
                acc.push_str(&self.kill);
                self.kill = acc;
            } else {
                self.kill = killed;
            }
            let before = self.cursor;
            // A word kill is its own atomic undo group (whitespace-bounded).
            self.seal_undo_group();
            self.apply_edit(i, self.cursor - i, "", before, i);
            self.seal_undo_group();
            self.last_was_kill = true;
        } else {
            self.last_was_kill = false;
        }
    }

    /// ⌥+forward-Delete: delete the token AFTER the cursor (into the kill buffer,
    /// so C-y can bring it back) — the exact forward mirror of
    /// [`Self::delete_word_backward`] and its
    /// [`word_delete_forward_boundary`](super::word_delete_forward_boundary): fold
    /// any LEADING whitespace, then remove exactly ONE token class — a word run OR
    /// a punctuation run, never both. So `⎸... abc` deletes only `...`, leaving
    /// ` abc` (the word survives), mirroring `... abc⎸` ⌥⌫ deleting only `abc`.
    /// Char/grapheme-safe (indices are char indices into the rope) and a clean
    /// NO-OP at end-of-buffer. With an active selection, delete that instead.
    pub fn delete_word_forward(&mut self) {
        self.goal_col = None;
        if self.delete_selection() {
            self.last_was_kill = false;
            return;
        }
        let len = self.rope.len_chars();
        let j = super::word_delete_forward_boundary(self.cursor, len, |k| self.rope.char(k));
        if j > self.cursor {
            let killed = self.rope.slice(self.cursor..j).to_string();
            // Consecutive word-kills ACCUMULATE into the kill ring (the same
            // append precedent `kill_line` sets), so C-y brings back EVERY word
            // killed in a run, not just the last. A FORWARD kill removes text to
            // the RIGHT of the prior one, so it APPENDS to keep the ring in
            // reading order (Emacs kill-ring accumulation).
            if self.last_was_kill {
                self.kill.push_str(&killed);
            } else {
                self.kill = killed;
            }
            let before = self.cursor;
            // A word kill is its own atomic undo group (whitespace-bounded).
            self.seal_undo_group();
            // The cursor stays put; the text to its right collapses to meet it.
            self.apply_edit(self.cursor, j - self.cursor, "", before, before);
            self.seal_undo_group();
            self.last_was_kill = true;
        } else {
            self.last_was_kill = false;
        }
    }

    /// Cmd-⌫: delete from the cursor back to the LOGICAL line start, leaving the
    /// caret there — the macOS-native "delete to beginning of line". Unlike a
    /// word-kill this does NOT touch the kill ring (it is a delete, not a cut,
    /// matching macOS). One atomic undo group; a clean NO-OP at column 0. With an
    /// active selection, delete that instead.
    pub fn delete_to_line_start(&mut self) {
        self.clear_kill_flag();
        self.goal_col = None;
        if self.delete_selection() {
            return;
        }
        let (line, _) = self.cursor_line_col();
        let start = self.line_start(line);
        if start < self.cursor {
            let before = self.cursor;
            self.seal_undo_group();
            self.apply_edit(start, self.cursor - start, "", before, start);
            self.seal_undo_group();
        }
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
        self.apply_kill_edit(before, end - before, before);
    }

    /// Apply a C-k kill's forward-delete, coalescing CONSECUTIVE kills into ONE
    /// undo group even across a whitespace-bearing kill: the whole kill run is a
    /// single user gesture, so the common `C-k C-k` (kill the line's content,
    /// then its newline) restores in one `C-/`. A kill never merges with a
    /// preceding INSERTION run (different edit direction). `record_edit`'s
    /// ordinary whitespace seal — which keeps each typed word its own undo step —
    /// is untouched; only this kill path overrides it.
    fn apply_kill_edit(&mut self, start: usize, len: usize, before: usize) {
        let following_kill = self.last_was_kill;
        if following_kill {
            // Reopen a group the PRIOR kill may have sealed on its whitespace so
            // this kill coalesces into it (`last_edit_kind` is still Delete).
            self.undo_group_open = true;
        } else {
            self.seal_undo_group();
        }
        self.apply_edit(start, len, "", before, before);
        if following_kill {
            // Keep the group open for a further kill; `record_edit` sealed it if
            // this kill removed whitespace.
            self.undo_group_open = true;
        }
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
        self.apply_kill_edit(before, end - before, before);
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
            } else if self.is_markdown() && is_url(&s) {
                // PASTE-URL-OVER-SELECTION → MARKDOWN LINK (markdown buffers
                // only — a `.rs`/`.txt` paste of a URL over a selection stays a
                // normal replace, never `[x](url)` in code). Wrap the selected
                // text as `[selected](url)` in ONE undoable edit; Cmd-Z restores
                // the original selection. The selected text comes from the rope,
                // the URL from the (already clipboard-refreshed) kill ring — no
                // new plumbing.
                let sel = self.rope.slice(start..end).to_string();
                let link = format!("[{sel}]({s})");
                let after = start + link.chars().count();
                self.seal_undo_group();
                self.apply_edit(start, end - start, &link, before, after);
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
