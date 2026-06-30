//! CURSOR MOTION — the non-mutating caret movements: C-f / C-b char motion,
//! C-a / C-e line ends, C-n / C-p vertical motion (keeping a goal column across
//! short lines), M-< / M-> buffer ends, and M-f / M-b word motion. Each clears the
//! kill flag like mg. Carved out of `buffer.rs` verbatim — inherent methods on
//! [`Buffer`].

use super::{is_word_char, Buffer};

impl Buffer {
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
}
