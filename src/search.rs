//! Pure incremental-search model. No winit/gpu. Operates on a document string +
//! query, computes all match CHAR ranges, supports next/prev with wrap.
//!
//! The query lives in its OWN String (like the IME preedit), never spliced into
//! the rope. All offsets are CHAR indices (not bytes) so they map directly to
//! `Buffer::set_cursor` / `char_to_line_col` even for multibyte text.

/// A single match as a half-open CHAR range `[start, end)` into the document.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Match {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    Forward,
    Backward,
}

/// Live isearch state. Owned by `App` as `Option<SearchState>`; the query is its
/// OWN String, never spliced into the rope.
pub struct SearchState {
    query: String,
    /// Default false (case-insensitive).
    case_sensitive: bool,
    /// All matches for the current query, in buffer order.
    matches: Vec<Match>,
    /// Index into `matches`, `None` when there are no matches.
    current: Option<usize>,
    /// Last C-s / C-r direction; biases the initial pick + the step direction.
    direction: Direction,
    /// Cursor char index at search start; restored on abort.
    origin: usize,
}

impl SearchState {
    /// Begin a search anchored at `origin` (the cursor char index at start).
    /// The query is empty and there are no matches yet.
    pub fn start(origin: usize, direction: Direction) -> Self {
        Self {
            query: String::new(),
            case_sensitive: false,
            matches: Vec::new(),
            current: None,
            direction,
            origin,
        }
    }

    // --- query editing (each recomputes matches + re-picks current) ---------

    pub fn push_char(&mut self, c: char, haystack: &str) {
        self.query.push(c);
        self.recompute(haystack);
    }

    pub fn pop_char(&mut self, haystack: &str) {
        self.query.pop();
        self.recompute(haystack);
    }

    pub fn toggle_case(&mut self, haystack: &str) {
        self.case_sensitive = !self.case_sensitive;
        self.recompute(haystack);
    }

    /// Refill `matches` for the current query, then pick `current` anchored at
    /// `origin` (deterministic for capture/sidecar):
    ///   * Forward  → first match with `start >= origin`, else wrap to first.
    ///   * Backward → last match with `start <= origin`, else wrap to last.
    fn recompute(&mut self, haystack: &str) {
        self.matches = find_all(haystack, &self.query, self.case_sensitive);
        self.current = if self.matches.is_empty() {
            None
        } else {
            match self.direction {
                Direction::Forward => Some(
                    self.matches
                        .iter()
                        .position(|m| m.start >= self.origin)
                        .unwrap_or(0),
                ),
                Direction::Backward => Some(
                    self.matches
                        .iter()
                        .rposition(|m| m.start <= self.origin)
                        .unwrap_or(self.matches.len() - 1),
                ),
            }
        };
    }

    // --- navigation (C-s / C-r while already searching) --------------------

    /// Step to the next / previous match, wrapping around buffer ends. No-op
    /// when there are no matches. Also records `dir` as the active direction.
    pub fn step(&mut self, dir: Direction) {
        self.direction = dir;
        let len = self.matches.len();
        if len == 0 {
            return;
        }
        let cur = self.current.unwrap_or(0);
        self.current = Some(match dir {
            Direction::Forward => (cur + 1) % len,
            Direction::Backward => (cur + len - 1) % len,
        });
    }

    // --- accessors for App + render + capture -------------------------------
    //
    // Several of these are consumed by the RENDER stage (panel + sidecar) and
    // are not yet referenced by the CORE stage; allow dead_code so the CORE
    // build stays warning-clean, mirroring buffer.rs's not-yet-used accessors.

    #[allow(dead_code)]
    pub fn query(&self) -> &str {
        &self.query
    }

    #[allow(dead_code)]
    pub fn is_case_sensitive(&self) -> bool {
        self.case_sensitive
    }

    #[allow(dead_code)]
    pub fn matches(&self) -> &[Match] {
        &self.matches
    }

    pub fn current_match(&self) -> Option<Match> {
        self.current.map(|i| self.matches[i])
    }

    #[allow(dead_code)]
    pub fn hit_count(&self) -> usize {
        self.matches.len()
    }

    /// The 1-based ordinal of the current match for the "n/total" counter.
    #[allow(dead_code)]
    pub fn current_ordinal(&self) -> Option<usize> {
        self.current.map(|i| i + 1)
    }

    /// Index of the current match within `matches` (0-based), for the renderer.
    #[allow(dead_code)]
    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    /// True only when a non-empty query has zero hits (the ERROR-red state).
    #[allow(dead_code)]
    pub fn has_no_matches(&self) -> bool {
        !self.query.is_empty() && self.matches.is_empty()
    }

    pub fn origin(&self) -> usize {
        self.origin
    }
}

/// All non-overlapping CHAR-range matches of `needle` in `haystack`,
/// left-to-right. When `!case_sensitive`, characters are compared via
/// per-char lowercasing. Empty needle => empty vec.
///
/// CHAR-indexed (not byte-indexed): offsets are positions in the char stream,
/// so they feed `set_cursor` / `char_to_line_col` correctly for multibyte text.
///
/// NOTE: case folding is done per-char (`to_lowercase()` of each char compared
/// pairwise), so it assumes equal char counts between the needle char and the
/// haystack char. Exotic multi-char foldings (ß → "ss", İ → "i̇") are NOT
/// supported in v1.
pub fn find_all(haystack: &str, needle: &str, case_sensitive: bool) -> Vec<Match> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let hay: Vec<char> = haystack.chars().collect();
    let ndl: Vec<char> = needle.chars().collect();
    let nlen = ndl.len();
    if nlen > hay.len() {
        return out;
    }
    let mut i = 0usize;
    while i + nlen <= hay.len() {
        if char_window_matches(&hay[i..i + nlen], &ndl, case_sensitive) {
            out.push(Match {
                start: i,
                end: i + nlen,
            });
            i += nlen; // non-overlapping
        } else {
            i += 1;
        }
    }
    out
}

fn char_window_matches(window: &[char], needle: &[char], case_sensitive: bool) -> bool {
    window.iter().zip(needle.iter()).all(|(a, b)| {
        if case_sensitive {
            a == b
        } else {
            chars_eq_fold(*a, *b)
        }
    })
}

/// Case-insensitive single-char compare via per-char lowercasing.
fn chars_eq_fold(a: char, b: char) -> bool {
    if a == b {
        return true;
    }
    a.to_lowercase().eq(b.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(start: usize, end: usize) -> Match {
        Match { start, end }
    }

    #[test]
    fn find_all_basic() {
        assert_eq!(find_all("hello world", "world", false), vec![m(6, 11)]);
        assert_eq!(find_all("hello world", "o", false), vec![m(4, 4 + 1), m(7, 8)]);
    }

    #[test]
    fn find_all_multiple_hits() {
        // "line" appears three times.
        let hay = "line one\nline two\nline three";
        let got = find_all(hay, "line", false);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], m(0, 4));
    }

    #[test]
    fn find_all_non_overlapping() {
        // "aa" in "aaaa" => 2 matches (non-overlapping, resume past the hit).
        assert_eq!(find_all("aaaa", "aa", false), vec![m(0, 2), m(2, 4)]);
    }

    #[test]
    fn find_all_case_insensitive_default_vs_sensitive() {
        assert_eq!(find_all("Hello HELLO hello", "hello", false).len(), 3);
        assert_eq!(find_all("Hello HELLO hello", "hello", true), vec![m(12, 17)]);
    }

    #[test]
    fn find_all_empty_needle() {
        assert!(find_all("anything", "", false).is_empty());
        assert!(find_all("", "x", false).is_empty());
    }

    #[test]
    fn find_all_multibyte_char_offsets() {
        // "naïve café" — the 'ï' (U+00EF) and 'é' (U+00E9) are multibyte in
        // UTF-8, so byte offsets would differ from char offsets. We assert CHAR
        // offsets.
        let hay = "naïve café";
        // chars: n a ï v e ' ' c a f é  => 'café' starts at char index 6.
        let got = find_all(hay, "café", false);
        assert_eq!(got, vec![m(6, 10)]);
        // The matched chars are exactly "café".
        let chars: Vec<char> = hay.chars().collect();
        let matched: String = chars[got[0].start..got[0].end].iter().collect();
        assert_eq!(matched, "café");

        // A CJK string: "日本語日本語", needle "日本" appears twice non-overlapping.
        let cjk = "日本語日本語";
        let g = find_all(cjk, "日本", false);
        assert_eq!(g, vec![m(0, 2), m(3, 5)]);
    }

    #[test]
    fn current_pick_forward_at_or_after_origin_then_wrap() {
        // matches of "x" at char positions 2, 6, 10.
        let hay = "..x...x...x";
        // origin between first and second match -> picks the one at 6.
        let mut s = SearchState::start(4, Direction::Forward);
        s.push_char('x', hay);
        assert_eq!(s.hit_count(), 3);
        assert_eq!(s.current_match(), Some(m(6, 7)));
        // origin past the last match -> wrap to first.
        let mut s2 = SearchState::start(100, Direction::Forward);
        s2.push_char('x', hay);
        assert_eq!(s2.current_match(), Some(m(2, 3)));
    }

    #[test]
    fn current_pick_backward_at_or_before_origin_then_wrap() {
        let hay = "..x...x...x";
        // origin after the second match -> picks the one at 6 (last <= origin).
        let mut s = SearchState::start(8, Direction::Backward);
        s.push_char('x', hay);
        assert_eq!(s.current_match(), Some(m(6, 7)));
        // origin before the first match -> wrap to last.
        let mut s2 = SearchState::start(0, Direction::Backward);
        s2.push_char('x', hay);
        assert_eq!(s2.current_match(), Some(m(10, 11)));
    }

    #[test]
    fn step_forward_and_backward_wrap() {
        let hay = "x.x.x";
        let mut s = SearchState::start(0, Direction::Forward);
        s.push_char('x', hay); // matches at 0,2,4; current 0
        assert_eq!(s.current_match(), Some(m(0, 1)));
        s.step(Direction::Forward);
        assert_eq!(s.current_match(), Some(m(2, 3)));
        s.step(Direction::Forward);
        assert_eq!(s.current_match(), Some(m(4, 5)));
        s.step(Direction::Forward); // wraps to first
        assert_eq!(s.current_match(), Some(m(0, 1)));
        s.step(Direction::Backward); // wraps to last
        assert_eq!(s.current_match(), Some(m(4, 5)));
    }

    #[test]
    fn step_noop_when_empty() {
        let mut s = SearchState::start(0, Direction::Forward);
        s.push_char('z', "abc"); // no matches
        assert_eq!(s.current_match(), None);
        s.step(Direction::Forward); // must not panic
        assert_eq!(s.current_match(), None);
    }

    #[test]
    fn push_then_pop_restores_match_set() {
        let hay = "abc abd";
        let mut s = SearchState::start(0, Direction::Forward);
        s.push_char('a', hay); // matches at 0,4
        s.push_char('b', hay); // matches at 0,4 (ab, ab)
        let two = s.hit_count();
        assert_eq!(two, 2);
        s.push_char('c', hay); // only "abc" => 1 match
        assert_eq!(s.hit_count(), 1);
        s.pop_char(hay); // back to "ab" => 2 matches
        assert_eq!(s.hit_count(), 2);
        assert_eq!(s.query(), "ab");
    }

    #[test]
    fn toggle_case_changes_hit_count() {
        let hay = "Hello HELLO hello";
        let mut s = SearchState::start(0, Direction::Forward);
        for c in "hello".chars() {
            s.push_char(c, hay);
        }
        assert_eq!(s.hit_count(), 3); // insensitive default
        s.toggle_case(hay);
        assert!(s.is_case_sensitive());
        assert_eq!(s.hit_count(), 1); // only exact "hello"
        s.toggle_case(hay);
        assert_eq!(s.hit_count(), 3);
    }

    #[test]
    fn has_no_matches_only_when_query_nonempty_and_zero_hits() {
        let mut s = SearchState::start(0, Direction::Forward);
        assert!(!s.has_no_matches()); // empty query
        s.push_char('z', "abc");
        assert!(s.has_no_matches()); // non-empty, zero hits
        s.pop_char("abc");
        assert!(!s.has_no_matches()); // empty again
        s.push_char('a', "abc");
        assert!(!s.has_no_matches()); // has a hit
    }

    #[test]
    fn origin_preserved_across_edits() {
        let mut s = SearchState::start(42, Direction::Forward);
        s.push_char('a', "aaa");
        s.push_char('a', "aaa");
        s.pop_char("aaa");
        s.toggle_case("aaa");
        assert_eq!(s.origin(), 42);
    }

    #[test]
    fn ordinal_is_one_based() {
        let hay = "x.x.x";
        let mut s = SearchState::start(0, Direction::Forward);
        s.push_char('x', hay);
        assert_eq!(s.current_ordinal(), Some(1));
        s.step(Direction::Forward);
        assert_eq!(s.current_ordinal(), Some(2));
    }
}
