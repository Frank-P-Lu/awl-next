//! UNDO / REDO ENGINE — the single content-mutation choke point ([`Buffer::apply_edit`])
//! plus the op-based history it records (`record_edit` / `contiguous_with_top`
//! coalescing rules) and the undo / redo / seal navigation. EVERY editing method
//! routes through `apply_edit`, so nothing escapes the history or the version
//! counter. Carved out of `buffer.rs` verbatim — inherent methods on [`Buffer`];
//! `apply_edit` is `pub(super)` so the sibling edit / selection modules can drive it.

use super::{Buffer, Edit, EditKind};

impl Buffer {
    // --- Undo / redo engine -----------------------------------------------

    /// THE single content-mutation choke point. Replaces the chars in
    /// `start..start+remove_len` with `insert`, moves the cursor to
    /// `cursor_after`, bumps the version + dirty flag, and records the change for
    /// undo. `cursor_before` is the cursor position to restore on undo (usually
    /// the cursor as it was when the edit began). EVERY editing method routes
    /// through here so nothing escapes the history or the version counter.
    ///
    /// `pub(super)` so the sibling `edit` / `selection` submodules (and the module
    /// root) can drive it; the surrounding `record_edit` / `contiguous_with_top`
    /// coalescing stay private to this module.
    pub(super) fn apply_edit(
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
        // Any content edit ends a visual vertical run (the wrap geometry just
        // moved), so the next C-n/C-p recomputes the sticky visual goal-x.
        self.goal_x = None;
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
}
