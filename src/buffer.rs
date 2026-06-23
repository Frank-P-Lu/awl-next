//! The editor buffer: a ropey `Rope` plus a cursor, plus all the pure editing
//! and motion logic. This module has NO rendering and NO winit dependency, so it
//! is unit-testable in isolation (see the `tests` module at the bottom). The
//! keymap turns key events into method calls on this type.

use std::path::{Path, PathBuf};

use ropey::Rope;

/// A character classification used for word motion (M-f / M-b). "Word"
/// characters are alphanumeric or underscore; everything else is punctuation or
/// whitespace, matching mg/emacs default word syntax closely enough for v1.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// One recorded edit, the unit of undo. We store the CHANGE (op-based history),
/// not a whole-document snapshot, so memory is proportional to what was edited.
/// At char index `start`, the text `removed` was replaced by the text `inserted`.
/// `cursor_before` is where the cursor sat before the edit (restored on undo);
/// `cursor_after` is where it landed after (restored on redo). Inverting an edit
/// (undo) re-inserts `removed` in place of `inserted` and restores `cursor_before`.
#[derive(Clone, Debug)]
struct Edit {
    start: usize,
    removed: String,
    inserted: String,
    cursor_before: usize,
    cursor_after: usize,
}

/// The direction of the last recorded edit, used for coalescing. An insertion
/// run and a deletion run never merge into the same group.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Insert,
    Delete,
}

/// The text buffer + cursor. The cursor is stored as an absolute char index into
/// the rope; line/column are derived on demand. A "goal column" is remembered so
/// that vertical motion (C-n/C-p) keeps a stable column across short lines.
pub struct Buffer {
    rope: Rope,
    /// Absolute char index of the cursor, in `0..=rope.len_chars()`.
    cursor: usize,
    /// Remembered visual column for vertical motion; `None` means "recompute".
    goal_col: Option<usize>,
    /// The file this buffer is bound to (for C-x C-s). `None` for scratch.
    path: Option<PathBuf>,
    /// Kill buffer (C-k / C-y). Appended to by consecutive kills.
    kill: String,
    /// Whether the previous command was a kill, so consecutive C-k appends.
    last_was_kill: bool,
    /// Dirty flag (unsaved changes).
    dirty: bool,
    /// Selection mark: the anchor char index. The selection is the span between
    /// `anchor` and `cursor`. `None` means no active selection. Set by C-Space
    /// (set-mark) or a Shift+motion / mouse drag; cleared by C-g or a plain
    /// motion that does not extend.
    anchor: Option<usize>,
    /// Monotonic edit version, bumped on every mutation of the rope CONTENT. Lets
    /// callers (the view sync / spell debounce) detect "did the text change?" with
    /// a cheap `u64` compare instead of cloning + comparing the whole rope string
    /// each keystroke. Cursor/selection-only changes do NOT bump it.
    version: u64,
    /// Undo stack: completed (and the in-progress) edit groups, oldest first.
    /// Each group is a run of coalesced [`Edit`]s applied together; one undo pops
    /// and inverts the whole top group. A fresh edit may extend the top group (see
    /// coalescing rules in [`record_edit`]) or push a new one.
    undo_stack: Vec<Vec<Edit>>,
    /// Redo stack: groups popped by undo, ready to re-apply. Cleared by any NEW
    /// edit (linear, modern-editor history — undo is not itself undoable).
    redo_stack: Vec<Vec<Edit>>,
    /// True when the top undo group is "open" and a contiguous same-direction edit
    /// may coalesce into it. Sealed (set false) by [`seal_undo_group`] after any
    /// non-edit command, and internally when a group-breaking edit occurs.
    undo_group_open: bool,
    /// The direction of the last recorded edit, for coalescing decisions.
    last_edit_kind: Option<EditKind>,
}

impl Buffer {
    /// Empty scratch buffer (no file).
    pub fn scratch() -> Self {
        Self::from_rope(Rope::new(), None)
    }

    /// Load a file into a buffer. A missing file yields an empty buffer bound to
    /// that path (so the first C-x C-s creates it), matching mg behavior.
    pub fn from_file(path: &Path) -> Self {
        let rope = match std::fs::read_to_string(path) {
            Ok(s) => Rope::from_str(&s),
            Err(_) => Rope::new(),
        };
        Self::from_rope(rope, Some(path.to_path_buf()))
    }

    /// Build directly from a string (used in tests and scratch construction).
    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Self {
        Self::from_rope(Rope::from_str(s), None)
    }

    fn from_rope(rope: Rope, path: Option<PathBuf>) -> Self {
        Self {
            rope,
            cursor: 0,
            goal_col: None,
            path,
            kill: String::new(),
            last_was_kill: false,
            dirty: false,
            anchor: None,
            version: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            undo_group_open: false,
            last_edit_kind: None,
        }
    }

    /// The current edit version (bumped on every content mutation). A cheap change
    /// token for the view-sync / spell-debounce hot path: equal versions ⇒ the
    /// rope text is unchanged, so a full-string compare can be skipped.
    pub fn version(&self) -> u64 {
        self.version
    }

    // --- Accessors --------------------------------------------------------

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn line_count(&self) -> usize {
        // ropey counts a trailing newline as ending a line; for display we want
        // at least one line.
        self.rope.len_lines().max(1)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    #[allow(dead_code)]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn kill_buffer(&self) -> &str {
        &self.kill
    }

    /// Pure setter for the kill ring's top entry. Used by the app's clipboard
    /// bridge to load an external OS-clipboard value before a yank. Overwrites
    /// (does not append) and MUST NOT touch `last_was_kill`: loading an external
    /// value is not a kill, so a subsequent C-k must start a fresh kill rather
    /// than chaining onto this. No winit/gpu/arboard here — buffer stays pure.
    pub fn set_kill(&mut self, s: &str) {
        self.kill.clear();
        self.kill.push_str(s);
    }

    /// Cursor as (line, column) both 0-based, column measured in chars.
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let line = self.rope.char_to_line(self.cursor);
        let line_start = self.rope.line_to_char(line);
        (line, self.cursor - line_start)
    }

    #[allow(dead_code)]
    pub fn cursor_char(&self) -> usize {
        self.cursor
    }

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
        self.cursor = idx.min(self.rope.len_chars());
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

    // --- Internal line geometry helpers -----------------------------------

    /// Char index of the start of `line`.
    fn line_start(&self, line: usize) -> usize {
        self.rope.line_to_char(line)
    }

    /// Number of chars on `line` EXCLUDING the trailing newline (if any).
    fn line_len(&self, line: usize) -> usize {
        let total_lines = self.rope.len_lines();
        if line >= total_lines {
            return 0;
        }
        let start = self.rope.line_to_char(line);
        let end = if line + 1 < total_lines {
            self.rope.line_to_char(line + 1)
        } else {
            self.rope.len_chars()
        };
        let mut len = end - start;
        // Trim a single trailing '\n' from the count.
        if len > 0 {
            let last = self.rope.char(end - 1);
            if last == '\n' {
                len -= 1;
            }
        }
        len
    }

    fn clear_kill_flag(&mut self) {
        self.last_was_kill = false;
    }

    // --- Motion -----------------------------------------------------------

    pub fn forward_char(&mut self) {
        self.clear_kill_flag();
        self.goal_col = None;
        if self.cursor < self.rope.len_chars() {
            self.cursor += 1;
        }
    }

    pub fn backward_char(&mut self) {
        self.clear_kill_flag();
        self.goal_col = None;
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn line_start_motion(&mut self) {
        self.clear_kill_flag();
        self.goal_col = None;
        let (line, _) = self.cursor_line_col();
        self.cursor = self.line_start(line);
    }

    pub fn line_end_motion(&mut self) {
        self.clear_kill_flag();
        self.goal_col = None;
        let (line, _) = self.cursor_line_col();
        self.cursor = self.line_start(line) + self.line_len(line);
    }

    pub fn next_line(&mut self) {
        self.vertical(1);
    }

    pub fn previous_line(&mut self) {
        self.vertical(-1);
    }

    /// Move the cursor `delta` lines (negative = up), preserving the goal column.
    fn vertical(&mut self, delta: isize) {
        self.clear_kill_flag();
        let (line, col) = self.cursor_line_col();
        let goal = self.goal_col.unwrap_or(col);
        let target_line = line as isize + delta;
        if target_line < 0 {
            // At top: go to start of first line but keep goal column.
            self.cursor = 0;
            self.goal_col = Some(goal);
            return;
        }
        let last_line = self.line_count() - 1;
        let target_line = (target_line as usize).min(last_line);
        let target_len = self.line_len(target_line);
        let target_col = goal.min(target_len);
        self.cursor = self.line_start(target_line) + target_col;
        self.goal_col = Some(goal);
    }

    pub fn buffer_start(&mut self) {
        self.clear_kill_flag();
        self.goal_col = None;
        self.cursor = 0;
    }

    pub fn buffer_end(&mut self) {
        self.clear_kill_flag();
        self.goal_col = None;
        self.cursor = self.rope.len_chars();
    }

    pub fn forward_word(&mut self) {
        self.clear_kill_flag();
        self.goal_col = None;
        let len = self.rope.len_chars();
        let mut i = self.cursor;
        // Skip non-word chars, then skip word chars.
        while i < len && !is_word_char(self.rope.char(i)) {
            i += 1;
        }
        while i < len && is_word_char(self.rope.char(i)) {
            i += 1;
        }
        self.cursor = i;
    }

    pub fn backward_word(&mut self) {
        self.clear_kill_flag();
        self.goal_col = None;
        let mut i = self.cursor;
        // Skip non-word chars going left, then skip word chars going left.
        while i > 0 && !is_word_char(self.rope.char(i - 1)) {
            i -= 1;
        }
        while i > 0 && is_word_char(self.rope.char(i - 1)) {
            i -= 1;
        }
        self.cursor = i;
    }

    // --- Undo / redo engine -----------------------------------------------

    /// THE single content-mutation choke point. Replaces the chars in
    /// `start..start+remove_len` with `insert`, moves the cursor to
    /// `cursor_after`, bumps the version + dirty flag, and records the change for
    /// undo. `cursor_before` is the cursor position to restore on undo (usually
    /// the cursor as it was when the edit began). EVERY editing method routes
    /// through here so nothing escapes the history or the version counter.
    fn apply_edit(
        &mut self,
        start: usize,
        remove_len: usize,
        insert: &str,
        cursor_before: usize,
        cursor_after: usize,
    ) {
        let end = start + remove_len;
        let removed: String = if remove_len > 0 {
            self.rope.slice(start..end).to_string()
        } else {
            String::new()
        };
        if remove_len > 0 {
            self.rope.remove(start..end);
        }
        if !insert.is_empty() {
            self.rope.insert(start, insert);
        }
        self.cursor = cursor_after;
        self.dirty = true;
        self.version += 1;
        let edit = Edit {
            start,
            removed,
            inserted: insert.to_string(),
            cursor_before,
            cursor_after,
        };
        self.record_edit(edit);
    }

    /// Push `edit` onto the undo history, coalescing it into the open top group
    /// when it continues the same kind of run, else starting a new group. A new
    /// edit always clears the redo stack (linear history).
    ///
    /// A NEW group starts when:
    ///  * the group is sealed (a non-edit command ran), or
    ///  * the edit direction flips (insert <-> delete), or
    ///  * the edit is non-contiguous (cursor jumped between edits), or
    ///  * whitespace (space / newline) was just typed (so each word / line is its
    ///    own undo step) or just deleted.
    fn record_edit(&mut self, edit: Edit) {
        self.redo_stack.clear();

        let kind = if !edit.inserted.is_empty() && edit.removed.is_empty() {
            EditKind::Insert
        } else if edit.inserted.is_empty() && !edit.removed.is_empty() {
            EditKind::Delete
        } else {
            // A replace (selection overwrite, etc.) is its own atomic group.
            EditKind::Insert
        };

        // Replacements (both removed and inserted non-empty) never coalesce.
        let is_replace = !edit.inserted.is_empty() && !edit.removed.is_empty();

        // Whitespace (space / newline / tab) is an undo boundary: a typed space
        // joins the END of the current word's group (so undoing "hello " removes
        // the word AND its trailing space in one step), then SEALS so the next
        // word starts fresh. We therefore allow a contiguous whitespace edit to
        // coalesce, but force a seal afterwards.
        let edits_ws = |s: &str| s.chars().any(|c| c == ' ' || c == '\n' || c == '\t');
        let touches_ws = edits_ws(&edit.inserted) || edits_ws(&edit.removed);

        let can_coalesce = self.undo_group_open
            && !is_replace
            && self.last_edit_kind == Some(kind)
            && self.contiguous_with_top(&edit, kind);

        if can_coalesce {
            self.undo_stack.last_mut().unwrap().push(edit);
        } else {
            self.undo_stack.push(vec![edit]);
        }

        self.last_edit_kind = Some(kind);
        // The group stays open to absorb a contiguous same-kind successor, UNLESS
        // this edit touched whitespace (seal so the next word/line is its own
        // step) or was a replacement (always atomic).
        self.undo_group_open = !touches_ws && !is_replace;
    }

    /// Is `edit` contiguous with the last edit in the open top group, for its
    /// `kind`? For insertions, the new insert must begin exactly where the prior
    /// one ended. For deletions, the new deletion must be adjacent (backspace runs
    /// delete just before the prior start; forward-delete runs delete at the same
    /// start).
    fn contiguous_with_top(&self, edit: &Edit, kind: EditKind) -> bool {
        let Some(group) = self.undo_stack.last() else {
            return false;
        };
        let Some(prev) = group.last() else {
            return false;
        };
        match kind {
            EditKind::Insert => {
                // Prior insert occupied prev.start .. prev.start+len; the cursor
                // sat at the end. A continuing insert starts at that end.
                let prev_end = prev.start + prev.inserted.chars().count();
                edit.start == prev_end && edit.cursor_before == prev.cursor_after
            }
            EditKind::Delete => {
                // Backspace: each deletion removes the char(s) ending exactly where
                // the prior deletion began (edit.start + len == prev.start).
                // Forward-delete (C-d): deletes at the SAME start repeatedly.
                let del_end = edit.start + edit.removed.chars().count();
                let backspace_contig = del_end == prev.start;
                let forward_contig = edit.start == prev.start;
                (backspace_contig || forward_contig)
                    && edit.cursor_before == prev.cursor_after
            }
        }
    }

    /// Seal the open undo group so the NEXT edit starts a fresh group. The app
    /// calls this after any non-edit command (cursor motion, save, set-mark, …)
    /// so one Cmd+Z undoes a sensible chunk rather than spilling across a motion.
    pub fn seal_undo_group(&mut self) {
        self.undo_group_open = false;
        self.last_edit_kind = None;
    }

    /// True if there is anything to undo (for UI / tests).
    #[allow(dead_code)]
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// True if there is anything to redo (for UI / tests).
    #[allow(dead_code)]
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Undo the most recent edit group: invert each edit in reverse order
    /// (re-insert removed text, remove inserted text), restore the cursor to where
    /// it was before that group, clear any selection, and move the group onto the
    /// redo stack. Bumps the version so the view re-syncs / reshapes / re-spells.
    pub fn undo(&mut self) {
        let Some(group) = self.undo_stack.pop() else {
            return;
        };
        // Invert in reverse application order.
        for edit in group.iter().rev() {
            let ins_len = edit.inserted.chars().count();
            // Remove what this edit inserted.
            if ins_len > 0 {
                self.rope.remove(edit.start..edit.start + ins_len);
            }
            // Put back what it removed.
            if !edit.removed.is_empty() {
                self.rope.insert(edit.start, &edit.removed);
            }
        }
        // Restore the cursor to the start of the group's first edit's "before".
        self.cursor = group.first().map(|e| e.cursor_before).unwrap_or(self.cursor);
        self.anchor = None;
        self.goal_col = None;
        self.dirty = true;
        self.version += 1;
        self.last_was_kill = false;
        self.redo_stack.push(group);
        // The history boundary is now hard: a later edit must not coalesce across
        // this undo.
        self.undo_group_open = false;
        self.last_edit_kind = None;
    }

    /// Redo the most recently undone group: re-apply each edit in forward order
    /// (remove removed, insert inserted), restore the cursor to where the group
    /// left it, clear any selection, and move the group back onto the undo stack.
    pub fn redo(&mut self) {
        let Some(group) = self.redo_stack.pop() else {
            return;
        };
        for edit in group.iter() {
            let rem_len = edit.removed.chars().count();
            if rem_len > 0 {
                self.rope.remove(edit.start..edit.start + rem_len);
            }
            if !edit.inserted.is_empty() {
                self.rope.insert(edit.start, &edit.inserted);
            }
        }
        self.cursor = group.last().map(|e| e.cursor_after).unwrap_or(self.cursor);
        self.anchor = None;
        self.goal_col = None;
        self.dirty = true;
        self.version += 1;
        self.last_was_kill = false;
        self.undo_stack.push(group);
        self.undo_group_open = false;
        self.last_edit_kind = None;
    }

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

    // --- Word / line bounds (for double / triple click) -------------------

    /// The char range `[start, end)` of the word containing or adjacent to
    /// `idx`. If `idx` is on a word char, returns that whole word; otherwise the
    /// run of non-word chars under it. Used by double-click select-word.
    pub fn word_bounds(&self, idx: usize) -> (usize, usize) {
        let len = self.rope.len_chars();
        if len == 0 {
            return (0, 0);
        }
        let idx = idx.min(len);
        // Decide which class we are selecting: prefer the char AT idx, else the
        // char before it (so a click at end-of-word still grabs the word).
        let class_at = |i: usize| -> Option<bool> {
            if i < len {
                Some(is_word_char(self.rope.char(i)))
            } else {
                None
            }
        };
        let want = class_at(idx)
            .or_else(|| if idx > 0 { class_at(idx - 1) } else { None })
            .unwrap_or(true);
        let mut start = idx;
        while start > 0 && is_word_char(self.rope.char(start - 1)) == want {
            start -= 1;
        }
        let mut end = idx;
        while end < len && is_word_char(self.rope.char(end)) == want {
            end += 1;
        }
        (start, end)
    }

    /// The char range `[start, end)` of the line containing `idx`, INCLUDING the
    /// trailing newline if present (so triple-click selects the whole line).
    pub fn line_bounds(&self, idx: usize) -> (usize, usize) {
        let idx = idx.min(self.rope.len_chars());
        let line = self.rope.char_to_line(idx);
        let start = self.line_start(line);
        let total_lines = self.rope.len_lines();
        let end = if line + 1 < total_lines {
            self.rope.line_to_char(line + 1)
        } else {
            self.rope.len_chars()
        };
        (start, end)
    }

    /// Select an explicit char range: set the mark at `start` and the cursor at
    /// `end` (both clamped). Used by double/triple-click and the `--sel` hook.
    pub fn select_range(&mut self, start: usize, end: usize) {
        self.clear_kill_flag();
        self.goal_col = None;
        let max = self.rope.len_chars();
        self.anchor = Some(start.min(max));
        self.cursor = end.min(max);
    }

    // --- Files ------------------------------------------------------------

    /// Save to the bound path. Returns Err if there is no path.
    pub fn save(&mut self) -> anyhow::Result<()> {
        match &self.path {
            Some(p) => {
                std::fs::write(p, self.rope.to_string())?;
                self.dirty = false;
                Ok(())
            }
            None => anyhow::bail!("no file bound to this buffer (scratch)"),
        }
    }
}

#[cfg(test)]
mod tests {
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
}
