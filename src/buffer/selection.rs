//! SELECTION + CURSOR PLACEMENT — the mark / region model, the raw cursor
//! setters, and the line/col conversions that sit beside them. `set_mark` /
//! `clear_mark` / `set_anchor` manage the selection anchor; `selection_range` /
//! `selection_line_col` read it; `set_cursor` / `set_cursor_visual` move the caret
//! WITHOUT disturbing the mark (so a Shift+motion or mouse drag extends a region);
//! `delete_selection` / `copy_region` / `kill_region` act on it. Carved out of
//! `buffer.rs` verbatim — these stay inherent methods on [`Buffer`].

use super::Buffer;

impl Buffer {
    // --- Selection (mark / region) ----------------------------------------

    /// C-Space: set the mark at the current cursor (start a selection).
    pub fn set_mark(&mut self) {
        self.clear_kill_flag();
        self.anchor = Some(self.cursor);
    }

    /// C-g: clear the mark (cancel the selection). Cursor unchanged.
    pub fn clear_mark(&mut self) {
        self.anchor = None;
    }

    /// Set the mark to an explicit char index (used by mouse-press to begin a
    /// drag selection). Clamped into range.
    pub fn set_anchor(&mut self, idx: usize) {
        self.clear_kill_flag();
        self.anchor = Some(idx.min(self.rope.len_chars()));
    }

    /// True when a mark is set and spans a non-empty region.
    pub fn has_selection(&self) -> bool {
        matches!(self.anchor, Some(a) if a != self.cursor)
    }

    /// The active mark (anchor), if any. `None` = no selection.
    #[allow(dead_code)]
    pub fn anchor_char(&self) -> Option<usize> {
        self.anchor
    }

    /// The selection as an ordered `(start, end)` char range (start <= end), or
    /// `None` when there is no non-empty selection.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        match self.anchor {
            Some(a) if a != self.cursor => {
                Some((a.min(self.cursor), a.max(self.cursor)))
            }
            _ => None,
        }
    }

    /// The selection expressed in line/col endpoints, ordered so the first
    /// endpoint is earlier in the buffer. Returns `((l0,c0),(l1,c1))` or `None`.
    /// Used by the renderer to build highlight rectangles.
    pub fn selection_line_col(&self) -> Option<((usize, usize), (usize, usize))> {
        let (start, end) = self.selection_range()?;
        Some((self.char_to_line_col(start), self.char_to_line_col(end)))
    }

    /// Convert an absolute char index to (line, col).
    pub fn char_to_line_col(&self, idx: usize) -> (usize, usize) {
        let idx = idx.min(self.rope.len_chars());
        let line = self.rope.char_to_line(idx);
        let line_start = self.rope.line_to_char(line);
        (line, idx - line_start)
    }

    /// The text of `line` EXCLUDING the trailing newline. Used by the markdown
    /// smart-newline to read the current block's prefix (list marker / blockquote
    /// / indentation) so Enter can continue or end it. Pure read; no allocation
    /// beyond the one returned line.
    pub fn line_text(&self, line: usize) -> String {
        let start = self.line_start(line);
        let len = self.line_len(line);
        self.rope.slice(start..start + len).to_string()
    }

    /// Convert a (line, col) to an absolute char index, clamping col to the
    /// line's length and line to the buffer. The inverse of [`char_to_line_col`]
    /// for in-range inputs; used by mouse hit-testing.
    pub fn line_col_to_char(&self, line: usize, col: usize) -> usize {
        let last_line = self.line_count() - 1;
        let line = line.min(last_line);
        let len = self.line_len(line);
        self.line_start(line) + col.min(len)
    }

    /// Move the cursor to an absolute char index (clamped), WITHOUT touching the
    /// mark, so a Shift+motion or mouse drag extends the selection. Resets the
    /// goal column and kill flag like the other motions.
    pub fn set_cursor(&mut self, idx: usize) {
        self.clear_kill_flag();
        self.goal_col = None;
        self.goal_x = None;
        self.cursor = idx.min(self.rope.len_chars());
    }

    /// The remembered VISUAL goal-x for visual-line vertical motion (see the
    /// `goal_x` field). `apply_core`'s layout oracle reads this at the start of a
    /// C-n/C-p: `Some(x)` means a run of vertical moves is in progress and the
    /// caret should stay under `x`; `None` means recompute from the caret's current
    /// visual x.
    pub fn goal_x(&self) -> Option<f32> {
        self.goal_x
    }

    /// Place the caret at char index `idx` for a VISUAL vertical move, REMEMBERING
    /// `goal_x` (the TEXT_LEFT-relative pixel column to stay under across the run).
    /// Unlike [`Self::set_cursor`] this does NOT clear `goal_x`, so consecutive
    /// C-n/C-p keep the same screen column through soft wraps; like it, it leaves
    /// the mark untouched (so Shift+C-n extends the region). The next non-vertical
    /// motion or edit clears `goal_x` via `clear_kill_flag` / `apply_edit`.
    pub fn set_cursor_visual(&mut self, idx: usize, goal_x: f32) {
        self.last_was_kill = false;
        self.goal_col = None;
        self.cursor = idx.min(self.rope.len_chars());
        self.goal_x = Some(goal_x);
    }

    /// Delete the active selection (if any) and place the cursor at its start.
    /// Returns true if something was deleted. Used before self-insert / yank so
    /// typing replaces the selection (modern editor behavior).
    pub fn delete_selection(&mut self) -> bool {
        if let Some((start, end)) = self.selection_range() {
            let before = self.cursor;
            self.anchor = None;
            self.goal_col = None;
            self.apply_edit(start, end - start, "", before, start);
            true
        } else {
            self.anchor = None;
            false
        }
    }

    /// M-w: copy the active selection into the kill buffer, leaving text intact
    /// and clearing the mark. No-op (clears mark) when there is no selection.
    pub fn copy_region(&mut self) {
        self.clear_kill_flag();
        if let Some((start, end)) = self.selection_range() {
            self.kill = self.rope.slice(start..end).to_string();
        }
        self.anchor = None;
    }

    /// C-w: kill (cut) the active selection into the kill buffer and remove it
    /// from the buffer, placing the cursor at the region start.
    pub fn kill_region(&mut self) {
        if let Some((start, end)) = self.selection_range() {
            self.kill = self.rope.slice(start..end).to_string();
            let before = self.cursor;
            self.anchor = None;
            self.goal_col = None;
            // A region kill is its own atomic undo group.
            self.seal_undo_group();
            self.apply_edit(start, end - start, "", before, start);
            self.seal_undo_group();
        } else {
            self.anchor = None;
            self.goal_col = None;
        }
        // A region kill does not chain with C-k line kills.
        self.last_was_kill = false;
    }
}
