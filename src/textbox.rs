//! ITEM 10 — ONE SHARED SINGLE-LINE TEXTBOX MODEL: text + CHAR-index caret +
//! motion/edit/word rules, shared by the 7 end-only single-line fields this
//! item routes through it — picker query, Rename, Insert-link URL,
//! Keep-version name, Settings value, Find query, Replace text (see
//! [`TextField::ALL`]). Pure text + caret + motion — NO char filtering (a
//! Settings digit/`.`/`%` gate, a Rename `/`-reject), NO refilter/recompute/
//! commit; those stay owned by each surface (`overlay::capture`,
//! `overlay::nav`, `search::mod` respectively) exactly as before item 10.
//!
//! CHAR-INDEX DISCIPLINE: `caret` is a CHAR index into `text`
//! (`0..=text.chars().count()`), NEVER a byte offset — `String::insert` /
//! `replace_range` take BYTE indices, so every splice below converts
//! explicitly via `char_indices` (`Self::byte_of`). This is the Unicode trap
//! the parity tests in this file guard: CJK / combining marks / emoji are
//! multi-byte in UTF-8 but exactly ONE caret step, and a byte offset used as a
//! caret position panics (or silently splits a multibyte char) the first time
//! a field holds one.
//!
//! TWO DISTINCT WORD RULES, never conflated (see `buffer.rs`'s own doc on
//! [`crate::buffer::word_delete_backward_boundary`]): word MOTION
//! ([`TextBox::word_left`] / [`TextBox::word_right`], Ctrl/Opt-arrow)
//! delegates to the SAME [`crate::buffer::word_forward_boundary`] /
//! [`crate::buffer::word_backward_boundary`] free fns the document buffer's
//! own `Buffer::forward_word` / `backward_word` use; word DELETE
//! ([`TextBox::delete_word_back`] / [`TextBox::delete_word_forward`],
//! Opt-Backspace / Opt-forward-Delete) delegates to the SEPARATE
//! `word_delete_*_boundary` owners the document's `delete_word_backward` /
//! `_forward` (and the pre-item-10 minibuffer word-delete,
//! `overlay::nav::truncate_trailing_word`) already share. Wiring motion to
//! the delete rule (or vice versa) makes a textbox's opt-arrow disagree with
//! the document's own M-b/M-f — the item's headline trap.

use crate::buffer::{
    word_backward_boundary, word_delete_backward_boundary, word_delete_forward_boundary,
    word_forward_boundary,
};

/// A single-line text field: its content plus a CHAR-index caret. Shared by
/// every end-only minibuffer field (see the module doc) so motion/edit/word
/// rules exist in exactly ONE place — "same behavior ⇒ same code".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextBox {
    text: String,
    /// CHAR index into `text`, always in `0..=text.chars().count()`.
    caret: usize,
}

impl TextBox {
    /// An empty field, caret at 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// A field pre-filled with `s`, caret at the END — the seeding every
    /// existing minibuffer used before item 10 (Rename / Insert-link /
    /// Settings all start from the current value, caret ready to backspace
    /// it; only Keep-version seeds empty, via [`Self::new`]).
    pub fn seeded(s: &str) -> Self {
        Self { text: s.to_string(), caret: s.chars().count() }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn len_chars(&self) -> usize {
        self.text.chars().count()
    }

    /// Move the caret to `at`, CLAMPED to `[0, len_chars]` — never panics on
    /// an out-of-range request. Not yet wired to a live surface (no field
    /// currently jumps its caret to an arbitrary position); exercised by the
    /// parity/unit tests below and kept for a future click-to-place caller.
    #[allow(dead_code)]
    pub fn set_caret(&mut self, at: usize) {
        self.caret = at.min(self.len_chars());
    }

    /// The BYTE offset of CHAR index `idx` within `text` (`idx` may equal
    /// `len_chars()`, yielding `text.len()`) — the ONE char->byte conversion
    /// every splice below routes through, so a multibyte field (CJK /
    /// combining / emoji) never panics on a byte-misaligned
    /// `String::insert` / `replace_range` (the Unicode trap this module's
    /// doc names).
    fn byte_of(&self, idx: usize) -> usize {
        if idx == 0 {
            return 0;
        }
        self.text.char_indices().nth(idx).map(|(b, _)| b).unwrap_or(self.text.len())
    }

    /// Insert `c` at the caret and advance past it. Accepts ANY char — no
    /// filtering (a Settings digit gate / Rename `/`-reject is the CALLER's
    /// job, applied before this is reached; see the module doc).
    pub fn insert(&mut self, c: char) {
        let b = self.byte_of(self.caret);
        self.text.insert(b, c);
        self.caret += 1;
    }

    /// Backspace: delete the char BEFORE the caret. A no-op at the start.
    pub fn delete_back(&mut self) {
        if self.caret == 0 {
            return;
        }
        let end = self.byte_of(self.caret);
        let start = self.byte_of(self.caret - 1);
        self.text.replace_range(start..end, "");
        self.caret -= 1;
    }

    /// Forward-delete: remove the char AT the caret. A no-op at the end. Not
    /// yet wired to a live surface (none of the 7 fields bind a plain
    /// forward-Delete — only the word-delete variant, `delete_word_forward`,
    /// is claimed); kept for API completeness + the boundary-safety test below.
    #[allow(dead_code)]
    pub fn delete_forward(&mut self) {
        if self.caret >= self.len_chars() {
            return;
        }
        let start = self.byte_of(self.caret);
        let end = self.byte_of(self.caret + 1);
        self.text.replace_range(start..end, "");
        // Caret unchanged: the following char slides up to meet it.
    }

    /// One char LEFT.
    pub fn char_left(&mut self) {
        if self.caret > 0 {
            self.caret -= 1;
        }
    }

    /// One char RIGHT.
    pub fn char_right(&mut self) {
        if self.caret < self.len_chars() {
            self.caret += 1;
        }
    }

    /// WORD motion right — delegates to the SAME boundary rule
    /// [`Buffer::forward_word`](crate::buffer::Buffer::forward_word) uses
    /// (skip non-word, then skip word). NEVER the word-DELETE boundary — see
    /// the module doc's "two word rules" trap.
    pub fn word_right(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        self.caret = word_forward_boundary(self.caret, chars.len(), |i| chars[i]);
    }

    /// WORD motion left — the exact mirror of [`Self::word_right`].
    pub fn word_left(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        self.caret = word_backward_boundary(self.caret, |i| chars[i]);
    }

    /// WORD delete backward (⌥⌫) — the SAME token-class rule the document
    /// buffer's `delete_word_backward` uses, NOT the motion rule above.
    pub fn delete_word_back(&mut self) {
        if self.caret == 0 {
            return;
        }
        // Only the chars BEFORE the caret matter to the backward rule.
        let chars: Vec<char> = self.text.chars().take(self.caret).collect();
        let new_caret = word_delete_backward_boundary(self.caret, |i| chars[i]);
        let start = self.byte_of(new_caret);
        let end = self.byte_of(self.caret);
        self.text.replace_range(start..end, "");
        self.caret = new_caret;
    }

    /// WORD delete forward (⌥+forward-Delete) — the exact mirror of
    /// [`Self::delete_word_back`].
    pub fn delete_word_forward(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let len = chars.len();
        if self.caret >= len {
            return;
        }
        let stop = word_delete_forward_boundary(self.caret, len, |i| chars[i]);
        let start = self.byte_of(self.caret);
        let end = self.byte_of(stop);
        self.text.replace_range(start..end, "");
        // Caret unchanged: it sits at the start of what was just deleted.
    }
}

impl PartialEq<&str> for TextBox {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

impl PartialEq<str> for TextBox {
    fn eq(&self, other: &str) -> bool {
        self.text == other
    }
}

/// THE 7-FIELD ROSTER — every end-only single-line surface item 10 routes
/// through [`TextBox`]. Mirrors `OverlayKind::ALL`'s law
/// (`overlay/tests.rs`): [`Self::ALL`] plus a NO-WILDCARD match anywhere the
/// roster must stay exhaustive means an 8th field breaks compilation instead
/// of silently missing a sweep.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextField {
    /// The summoned-picker fuzzy query (Goto / Command / Theme / …).
    PickerQuery,
    /// The Rename minibuffer's typed filename.
    Rename,
    /// The Cmd-K Insert-link minibuffer's typed URL.
    InsertLink,
    /// The "Keep version…" minibuffer's typed (optional) name.
    KeepVersion,
    /// The Settings menu's inline numeric VALUE edit (page width / zoom).
    SettingsValue,
    /// The find/replace panel's search query.
    FindQuery,
    /// The find/replace panel's replacement text.
    ReplaceText,
}

impl TextField {
    #[allow(dead_code)]
    pub const ALL: [TextField; 7] = [
        TextField::PickerQuery,
        TextField::Rename,
        TextField::InsertLink,
        TextField::KeepVersion,
        TextField::SettingsValue,
        TextField::FindQuery,
        TextField::ReplaceText,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;

    // --- A. NO-WILDCARD 7-FIELD ROSTER --------------------------------------

    /// THE 7-FIELD ROSTER LAW: every [`TextField::ALL`] member has a home in
    /// this NO-WILDCARD match — an 8th field variant fails to COMPILE this
    /// test until it is added here, so a future single-line surface can never
    /// dodge the "route it through TextBox" sweep silently. Mirrors
    /// `OverlayKind::ALL`'s own exhaustive-match law.
    #[test]
    fn all_seven_fields_have_a_home_no_wildcard() {
        for f in TextField::ALL {
            match f {
                TextField::PickerQuery => {}
                TextField::Rename => {}
                TextField::InsertLink => {}
                TextField::KeepVersion => {}
                TextField::SettingsValue => {}
                TextField::FindQuery => {}
                TextField::ReplaceText => {}
            }
        }
        assert_eq!(TextField::ALL.len(), 7, "the roster is exactly the 7 fields item 10 names");
    }

    // --- B. UNICODE / BUFFER PARITY -----------------------------------------

    /// One (text, description) fixture per Unicode class the parity table
    /// sweeps: plain ASCII, CJK (multibyte, no combining), a combining
    /// grapheme cluster (base + U+0301 COMBINING ACUTE — two Rust `char`s,
    /// ONE visual glyph, exercising that both models step by SCALAR not
    /// grapheme), an emoji (multibyte, single scalar here), and a
    /// PUNCTUATION-ADJACENT fixture ending "word, " — its trailing char is
    /// whitespace immediately preceded by punctuation, the ONE shape where
    /// word MOTION (`word_backward_boundary`: collapse ALL non-word chars —
    /// space AND punctuation — before hitting a word char) and word DELETE
    /// (`word_delete_backward_boundary`: collapse whitespace only, then ONE
    /// token of the resulting class) actually disagree. `"hello world foo"`
    /// etc. never place punctuation next to a boundary, so the motion/delete
    /// rules coincide on them — a `word_left` mis-wired to the delete
    /// boundary would pass this whole table without this fixture (see the
    /// module doc's "two word rules" trap).
    fn fixtures() -> Vec<(&'static str, &'static str)> {
        vec![
            ("ascii", "hello world foo"),
            ("cjk", "日本語 text 二つ目"),
            ("combining", "cafe\u{0301} au lait\u{0301} noir"),
            ("emoji", "hi 🎉 there 🚀 world"),
            ("punct", "abc, "),
        ]
    }

    /// PARITY: starting both a [`TextBox`] and a [`Buffer`] on the SAME text
    /// with the caret at the SAME char index, an identical sequence of
    /// char-motion / word-motion / word-delete ops must land the SAME char
    /// index in both — `TextBox` is not a second, silently-diverging
    /// implementation of the document's own rules.
    #[test]
    fn textbox_char_and_word_motion_match_buffer_char_indices() {
        for (label, text) in fixtures() {
            let start = text.chars().count() / 2;
            let mut tb = TextBox::seeded(text);
            tb.set_caret(start);
            let mut buf = Buffer::from_str(text);
            buf.set_cursor(start);
            assert_eq!(tb.caret(), buf.cursor_char(), "{label}: seeded caret parity");

            // Walk forward by word twice, then back by word once, then char-step
            // in both directions — the SAME ops on both models.
            tb.word_right();
            buf.forward_word();
            assert_eq!(tb.caret(), buf.cursor_char(), "{label}: word_right #1");

            tb.word_right();
            buf.forward_word();
            assert_eq!(tb.caret(), buf.cursor_char(), "{label}: word_right #2");

            tb.word_left();
            buf.backward_word();
            assert_eq!(tb.caret(), buf.cursor_char(), "{label}: word_left");

            tb.char_right();
            buf.forward_char();
            assert_eq!(tb.caret(), buf.cursor_char(), "{label}: char_right");

            tb.char_left();
            buf.backward_char();
            assert_eq!(tb.caret(), buf.cursor_char(), "{label}: char_left");
        }
    }

    /// PARITY: word DELETE lands the SAME char index (and removes the SAME
    /// text) in both models — the DISTINCT rule from word motion above.
    #[test]
    fn textbox_word_delete_matches_buffer_word_delete() {
        for (label, text) in fixtures() {
            let start = text.chars().count();
            let mut tb = TextBox::seeded(text);
            let mut buf = Buffer::from_str(text);
            buf.set_cursor(start);

            tb.delete_word_back();
            buf.delete_word_backward();
            assert_eq!(tb.caret(), buf.cursor_char(), "{label}: delete_word_back caret");
            assert_eq!(tb.text(), buf.text(), "{label}: delete_word_back text");

            // Forward word-delete from the START of what remains.
            let mut tb2 = TextBox::seeded(tb.text());
            tb2.set_caret(0);
            let mut buf2 = Buffer::from_str(buf.text().as_str());
            buf2.set_cursor(0);
            tb2.delete_word_forward();
            buf2.delete_word_forward();
            assert_eq!(tb2.caret(), buf2.cursor_char(), "{label}: delete_word_forward caret");
            assert_eq!(tb2.text(), buf2.text(), "{label}: delete_word_forward text");
        }
    }

    /// A multibyte splice never panics and never splits a char: inserting /
    /// backspacing / forward-deleting mid-string around CJK, a combining
    /// mark, and an emoji all leave `text` valid UTF-8 with the expected
    /// content — the CHAR-index-not-byte-offset discipline the module doc
    /// names, exercised at every splice site.
    #[test]
    fn multibyte_splices_never_panic_and_stay_char_correct() {
        // CJK: insert a char between two multibyte glyphs.
        let mut tb = TextBox::seeded("日本語");
        tb.set_caret(1); // between 日 and 本
        tb.insert('X');
        assert_eq!(tb.text(), "日X本語");
        assert_eq!(tb.caret(), 2);

        // Combining mark: backspace removes exactly the trailing combining
        // scalar (ONE char step), not the whole cluster.
        let mut tb = TextBox::seeded("e\u{0301}"); // e + combining acute
        assert_eq!(tb.caret(), 2, "two scalars, two char steps");
        tb.delete_back();
        assert_eq!(tb.text(), "e");
        assert_eq!(tb.caret(), 1);

        // Emoji: forward-delete mid-string.
        let mut tb = TextBox::seeded("a🚀b");
        tb.set_caret(1); // just after 'a', before the emoji
        tb.delete_forward();
        assert_eq!(tb.text(), "ab");
        assert_eq!(tb.caret(), 1);
    }

    // --- misc TextBox unit coverage ------------------------------------------

    #[test]
    fn new_is_empty_caret_zero() {
        let tb = TextBox::new();
        assert!(tb.is_empty());
        assert_eq!(tb.caret(), 0);
    }

    #[test]
    fn seeded_puts_caret_at_the_end() {
        let tb = TextBox::seeded("abc");
        assert_eq!(tb.caret(), 3);
        assert_eq!(tb.text(), "abc");
    }

    #[test]
    fn set_caret_clamps_never_panics() {
        let mut tb = TextBox::seeded("abc");
        tb.set_caret(999);
        assert_eq!(tb.caret(), 3);
    }

    #[test]
    fn insert_at_mid_caret_splices_not_appends() {
        let mut tb = TextBox::seeded("ac");
        tb.set_caret(1);
        tb.insert('b');
        assert_eq!(tb.text(), "abc");
        assert_eq!(tb.caret(), 2);
    }

    #[test]
    fn delete_back_and_forward_are_boundary_safe() {
        let mut tb = TextBox::new();
        tb.delete_back(); // no-op, no panic
        tb.delete_forward(); // no-op, no panic
        assert_eq!(tb.text(), "");
    }

    #[test]
    fn eq_str_impl_matches_plain_string_compare() {
        let tb = TextBox::seeded("hello");
        assert_eq!(tb, "hello");
        assert_ne!(tb, "world");
    }
}
