//! Pure incremental-search model. No winit/gpu. Operates on a document string +
//! query, computes all match CHAR ranges, supports next/prev with wrap.
//!
//! The query lives in its OWN String (like the IME preedit), never spliced into
//! the rope. All offsets are CHAR indices (not bytes) so they map directly to
//! `Buffer::set_cursor` / `char_to_line_col` even for multibyte text.

// The ONE search/replace key-interception seam (`keys::intercept`) — shared by
// the live window's `App::handle_search_key` and the headless `--keys` replay
// guard, so the two drivers cannot drift.
pub mod keys;

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

/// What a [`SearchState::step`] did — so the caller knows whether to move the
/// cursor and whether to RECOIL the caret (the Emacs "failing I-search → press
/// again to wrap" feedback).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepOutcome {
    /// Advanced to an adjacent match within the buffer; the cursor follows.
    Moved,
    /// At the boundary (last match going forward / first going backward) with the
    /// wrap NOT yet armed: the current match did NOT change (the search "fails" at
    /// the edge). The caller RECOILS the caret in `dir` and the wrap is now armed —
    /// the NEXT same-direction step wraps. Emacs's two-press wrap.
    RecoiledAtBoundary(Direction),
    /// A second same-direction step at the boundary: WRAPPED to the far end (first
    /// match forward / last backward); the cursor follows. The arm is cleared.
    Wrapped,
    /// No matches at all (empty/failing query): nothing to do.
    NoMatches,
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
    /// REPLACE mode: once revealed, the SAME panel hosts a second (replacement)
    /// field. `false` keeps the plain isearch panel; the buffer is untouched until
    /// a replace fires, so a search that never reveals replace behaves exactly as
    /// before.
    replace_active: bool,
    /// The replacement text — its OWN String like `query`, never spliced into the
    /// rope until replace-current / replace-all is invoked.
    replacement: String,
    /// Which field typing edits: `false` = the search query (default), `true` = the
    /// replacement. Tab / Cmd-Option-F flip it (revealing the replace field the
    /// first time).
    editing_replacement: bool,
    /// The Emacs two-press WRAP arm. `Some(dir)` once a step has FAILED at the
    /// boundary in `dir` (a forward step at the last match / a backward step at the
    /// first): the cursor stayed put and the caret recoiled, and the NEXT step in
    /// `dir` wraps to the far end. ANY other action clears it — a new/edited query
    /// (`recompute`), a direction change, or a successful in-buffer step.
    wrap_armed: Option<Direction>,
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
            replace_active: false,
            replacement: String::new(),
            editing_replacement: false,
            wrap_armed: None,
        }
    }

    /// Begin a search anchored at `origin`, PREFILLED with `query` and
    /// immediately recomputing matches over `haystack` — ONE atomic open
    /// rather than [`Self::start`] plus a manual `push_char` loop, so there is
    /// no intermediate empty-query match state. Feeds both prefill doors the
    /// keybinding-idiom audit asks for: an active selection (Cmd-F, Xcode's
    /// "search for selection", W2) and the REMEMBERED last query (a bare
    /// Cmd-G/Cmd-Shift-G re-find, P2) — see `actions/motion.rs::start_search`,
    /// the one caller. An empty `query` behaves exactly like [`Self::start`].
    pub fn start_with_query(origin: usize, direction: Direction, query: &str, haystack: &str) -> Self {
        let mut s = Self::start(origin, direction);
        if !query.is_empty() {
            s.query = query.to_string();
            s.recompute(haystack);
        }
        s
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
        // Any query edit (push/pop/toggle-case) or re-anchor (refind) is an "other
        // action" that DISARMS the two-press wrap: the boundary state machine only
        // chains across consecutive same-direction steps.
        self.wrap_armed = None;
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

    /// Step to the next / previous match — the Emacs "failing I-search → press
    /// again to wrap" model (NOT a silent modulo wrap). Records `dir` as the active
    /// direction and returns a [`StepOutcome`] so the caller can move the cursor and
    /// recoil the caret:
    ///   * MID-BUFFER → advance to the adjacent match ([`StepOutcome::Moved`]).
    ///   * AT THE BOUNDARY (last match forward / first backward), wrap not yet armed
    ///     → the current match stays put, the wrap ARMS, and we return
    ///     [`StepOutcome::RecoiledAtBoundary`] so the caller bumps the caret.
    ///   * A SECOND same-direction step at the boundary → wrap to the far end and
    ///     clear the arm ([`StepOutcome::Wrapped`]).
    /// A DIRECTION CHANGE disarms the wrap (and steps normally that way); a query
    /// edit disarms it via `recompute`. No matches → [`StepOutcome::NoMatches`].
    pub fn step(&mut self, dir: Direction) -> StepOutcome {
        let len = self.matches.len();
        if len == 0 {
            self.direction = dir;
            self.wrap_armed = None;
            return StepOutcome::NoMatches;
        }
        // A direction change is an "other action": it clears any pending wrap arm
        // so the boundary chain only spans consecutive SAME-direction steps.
        if self.wrap_armed != Some(dir) {
            self.wrap_armed = None;
        }
        self.direction = dir;
        let cur = self.current.unwrap_or(0);
        let at_boundary = match dir {
            Direction::Forward => cur + 1 >= len,
            Direction::Backward => cur == 0,
        };
        if at_boundary {
            if self.wrap_armed == Some(dir) {
                // Second press at the boundary: wrap to the far end, disarm.
                self.wrap_armed = None;
                self.current = Some(match dir {
                    Direction::Forward => 0,
                    Direction::Backward => len - 1,
                });
                StepOutcome::Wrapped
            } else {
                // First press at the boundary: the search "fails" — stay put, arm the
                // wrap, and ask the caller to recoil the caret away from the edge.
                self.wrap_armed = Some(dir);
                StepOutcome::RecoiledAtBoundary(dir)
            }
        } else {
            // A normal in-buffer step disarms any stale wrap and advances.
            self.wrap_armed = None;
            self.current = Some(match dir {
                Direction::Forward => cur + 1,
                Direction::Backward => cur - 1,
            });
            StepOutcome::Moved
        }
    }

    /// Whether the two-press WRAP is currently ARMED, and in which direction — for
    /// the sidecar / tests to observe the boundary state machine.
    #[allow(dead_code)]
    pub fn wrap_armed(&self) -> Option<Direction> {
        self.wrap_armed
    }

    // --- find + replace -----------------------------------------------------
    //
    // Replace is a MODE of the same panel: the search query stays the needle, a
    // second field holds the replacement. The model never touches the rope — it
    // computes the post-replace text and the caller writes it back — so it stays
    // pure + unit-testable, mirroring `find_all`.

    /// SWITCH the focused field find↔replace, revealing the replace row the first
    /// time. Bound to Tab in the panel — the one field-switch key. Once the replace
    /// row is shown (e.g. via Cmd-R) Tab is a pure focus toggle.
    pub fn toggle_replace(&mut self) {
        if self.replace_active {
            self.editing_replacement = !self.editing_replacement;
        } else {
            self.replace_active = true;
            self.editing_replacement = true;
        }
    }

    /// Reveal the labeled replace row WITHOUT moving focus off the find field — the
    /// fresh Cmd-R open state (both rows shown, the amber caret still on the query).
    /// Idempotent: a re-reveal never steals focus back to the query.
    pub fn reveal_replace(&mut self) {
        self.replace_active = true;
    }

    /// Reveal the replace row AND move focus into the replacement — Cmd-R pressed
    /// again while the panel is already open jumps the caret into the replace field.
    pub fn focus_replacement(&mut self) {
        self.replace_active = true;
        self.editing_replacement = true;
    }

    /// Move focus back to the FIND field (the query) — the click-to-focus
    /// counterpart to [`Self::focus_replacement`], for a mouse press on the find
    /// row. Leaves the replace row's revealed state untouched (a click never hides
    /// it); a no-op when the query already has focus.
    pub fn focus_query(&mut self) {
        self.editing_replacement = false;
    }

    /// Append a char to the replacement field. The replacement is NOT searched,
    /// so the match set is unchanged (no recompute).
    pub fn push_replace_char(&mut self, c: char) {
        self.replacement.push(c);
    }

    /// Drop the last char of the replacement field (Backspace in the replace field).
    pub fn pop_replace_char(&mut self) {
        self.replacement.pop();
    }

    /// RE-ANCHOR the search at `origin` and recompute FORWARD against `haystack`.
    /// Used after a replace mutates the document so `current` lands on the next
    /// match at/after the edit (wrapping), skipping the just-inserted replacement.
    pub fn refind(&mut self, origin: usize, haystack: &str) {
        self.origin = origin;
        self.direction = Direction::Forward;
        self.recompute(haystack);
    }

    /// REPLACE-CURRENT: replace the CURRENT match in `haystack` with the
    /// replacement string, returning the new full document text, and ADVANCE
    /// `current` to the next match after the replacement. `None` when there is no
    /// current match. Pure w.r.t. the rope; the caller writes the text back and
    /// moves the cursor onto the (new) current match.
    pub fn replace_current_text(&mut self, haystack: &str) -> Option<String> {
        let m = self.current_match()?;
        let chars: Vec<char> = haystack.chars().collect();
        let mut out = String::with_capacity(haystack.len() + self.replacement.len());
        out.extend(chars[..m.start].iter());
        out.push_str(&self.replacement);
        out.extend(chars[m.end..].iter());
        let resume = m.start + self.replacement.chars().count();
        self.refind(resume, &out);
        Some(out)
    }

    /// REPLACE-ALL: return the full document text with EVERY current-query match
    /// replaced by the replacement string. Pure; returns the haystack unchanged
    /// (clone) when there are no matches. Replacements use the already-computed,
    /// non-overlapping `matches`, so a replacement that itself contains the needle
    /// is NOT re-replaced. The caller writes the text back, then `refind`s.
    pub fn replace_all_text(&self, haystack: &str) -> String {
        if self.matches.is_empty() {
            return haystack.to_string();
        }
        let chars: Vec<char> = haystack.chars().collect();
        let mut out = String::with_capacity(haystack.len());
        let mut prev = 0usize;
        for m in &self.matches {
            out.extend(chars[prev..m.start].iter());
            out.push_str(&self.replacement);
            prev = m.end;
        }
        out.extend(chars[prev..].iter());
        out
    }

    /// True once the replace field has been revealed (the panel shows both fields).
    pub fn is_replace_active(&self) -> bool {
        self.replace_active
    }

    /// True while typing edits the REPLACEMENT field (vs. the search query).
    pub fn is_editing_replacement(&self) -> bool {
        self.editing_replacement
    }

    /// The current replacement text (for the panel render + the sidecar).
    pub fn replacement(&self) -> &str {
        &self.replacement
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

// --- the REMEMBERED last search query (P2's honest Cmd-G re-find) ----------
//
// A tiny process-global mirroring `commands::RECENT`'s own MRU pattern: the
// last NON-EMPTY query a search closed with (Enter accept / Esc abort — live,
// `app/input/keys.rs` — or the headless `Action::Cancel` arm, the ONE search-
// close door `--keys` replay can reach; `actions.rs`). `start_search`
// (`actions/motion.rs`) consults it as the prefill FALLBACK when there is no
// active selection to prefer, so a bare Cmd-G/Cmd-Shift-G — with the panel
// already closed and nothing selected — genuinely re-finds the last thing you
// searched for, mirroring the Safari/browser convention. A fresh process
// starts empty, so a default `--screenshot` (and every test that never
// exercises this door) is unaffected.
use std::sync::Mutex;

static LAST_QUERY: Mutex<String> = Mutex::new(String::new());

/// Remember `query` as the last search term, IF non-empty — an EMPTY close
/// (a search opened and abandoned before typing anything) never overwrites a
/// still-useful remembered query.
pub fn set_last_query(query: &str) {
    if query.is_empty() {
        return;
    }
    if let Ok(mut q) = LAST_QUERY.lock() {
        *q = query.to_string();
    }
}

/// The remembered last search query (empty in a fresh process, or after
/// [`clear_last_query`]).
pub fn last_query() -> String {
    LAST_QUERY.lock().map(|q| q.clone()).unwrap_or_default()
}

/// TEST-ONLY: reset the remembered query so a test exercising it leaves no
/// residue for a later test reading [`last_query`] (mirrors
/// `commands::clear_recent`).
#[cfg(test)]
pub fn clear_last_query() {
    if let Ok(mut q) = LAST_QUERY.lock() {
        q.clear();
    }
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
        // Two-press wrap (Emacs failing-isearch): at the last match a forward step
        // RECOILS (stays put + arms); the NEXT forward step wraps. Mirror backward.
        let hay = "x.x.x";
        let mut s = SearchState::start(0, Direction::Forward);
        s.push_char('x', hay); // matches at 0,2,4; current 0
        assert_eq!(s.current_match(), Some(m(0, 1)));
        assert_eq!(s.step(Direction::Forward), StepOutcome::Moved);
        assert_eq!(s.current_match(), Some(m(2, 3)));
        assert_eq!(s.step(Direction::Forward), StepOutcome::Moved);
        assert_eq!(s.current_match(), Some(m(4, 5)));
        // First press at the last match: recoil, stay put, arm the wrap.
        assert_eq!(
            s.step(Direction::Forward),
            StepOutcome::RecoiledAtBoundary(Direction::Forward)
        );
        assert_eq!(s.current_match(), Some(m(4, 5)), "recoil does not advance");
        // Second press: wrap to the first.
        assert_eq!(s.step(Direction::Forward), StepOutcome::Wrapped);
        assert_eq!(s.current_match(), Some(m(0, 1)));
        // Now at the first match, a backward step recoils then wraps to the last.
        assert_eq!(
            s.step(Direction::Backward),
            StepOutcome::RecoiledAtBoundary(Direction::Backward)
        );
        assert_eq!(s.current_match(), Some(m(0, 1)), "recoil does not advance");
        assert_eq!(s.step(Direction::Backward), StepOutcome::Wrapped);
        assert_eq!(s.current_match(), Some(m(4, 5)));
    }

    #[test]
    fn wrap_arm_set_and_cleared_by_other_actions() {
        let hay = "x.x.x";
        // ARMS at the forward boundary; a DIRECTION CHANGE disarms (and steps).
        let mut s = SearchState::start(0, Direction::Forward);
        s.push_char('x', hay); // 0,2,4; current 0
        s.step(Direction::Forward); // ->2
        s.step(Direction::Forward); // ->4 (last)
        assert_eq!(s.wrap_armed(), None);
        assert_eq!(
            s.step(Direction::Forward),
            StepOutcome::RecoiledAtBoundary(Direction::Forward)
        );
        assert_eq!(s.wrap_armed(), Some(Direction::Forward));
        // A backward step is an "other action": it clears the arm AND moves (not a
        // wrap), since we are no longer at the relevant boundary.
        assert_eq!(s.step(Direction::Backward), StepOutcome::Moved);
        assert_eq!(s.wrap_armed(), None);
        assert_eq!(s.current_match(), Some(m(2, 3)));

        // A QUERY EDIT clears the arm too.
        let mut s2 = SearchState::start(0, Direction::Forward);
        s2.push_char('x', hay);
        s2.step(Direction::Forward); // ->2
        s2.step(Direction::Forward); // ->4
        s2.step(Direction::Forward); // arm
        assert_eq!(s2.wrap_armed(), Some(Direction::Forward));
        s2.push_char('.', hay); // query "x." now matches at 0,2; recompute disarms
        assert_eq!(s2.wrap_armed(), None);
        let mut s3 = SearchState::start(0, Direction::Forward);
        s3.push_char('x', hay);
        s3.step(Direction::Forward);
        s3.step(Direction::Forward);
        s3.step(Direction::Forward); // arm
        s3.pop_char(hay); // also disarms via recompute
        assert_eq!(s3.wrap_armed(), None);
    }

    #[test]
    fn wrap_arm_single_match_alternates_recoil_and_wrap() {
        // With ONE match there is no "next": a forward step recoils (arms), the next
        // wraps to itself, and the pattern alternates. No panic on the len-1 edge.
        let hay = "..x..";
        let mut s = SearchState::start(0, Direction::Forward);
        s.push_char('x', hay); // one match at 2
        assert_eq!(s.current_match(), Some(m(2, 3)));
        assert_eq!(
            s.step(Direction::Forward),
            StepOutcome::RecoiledAtBoundary(Direction::Forward)
        );
        assert_eq!(s.step(Direction::Forward), StepOutcome::Wrapped);
        assert_eq!(s.current_match(), Some(m(2, 3)));
    }

    #[test]
    fn step_noop_when_empty() {
        let mut s = SearchState::start(0, Direction::Forward);
        s.push_char('z', "abc"); // no matches
        assert_eq!(s.current_match(), None);
        assert_eq!(s.step(Direction::Forward), StepOutcome::NoMatches); // must not panic
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
    fn replace_mode_reveal_and_focus_toggle() {
        let mut s = SearchState::start(0, Direction::Forward);
        // Off by default: a plain isearch never reveals the replace field.
        assert!(!s.is_replace_active());
        assert!(!s.is_editing_replacement());
        // First toggle reveals the replace field AND moves focus to it.
        s.toggle_replace();
        assert!(s.is_replace_active());
        assert!(s.is_editing_replacement());
        // Subsequent toggles flip focus between the two fields (panel stays open).
        s.toggle_replace();
        assert!(s.is_replace_active());
        assert!(!s.is_editing_replacement());
        s.toggle_replace();
        assert!(s.is_editing_replacement());
        // The replacement field edits independently of the query.
        for c in "X".chars() {
            s.push_replace_char(c);
        }
        s.push_replace_char('Y');
        assert_eq!(s.replacement(), "XY");
        s.pop_replace_char();
        assert_eq!(s.replacement(), "X");
    }

    #[test]
    fn reveal_replace_keeps_find_focus_then_focus_replacement_moves_it() {
        // Cmd-R OPEN: the replace row is revealed but focus stays on the FIND field
        // (so you type the needle first) — the redesigned open state.
        let mut s = SearchState::start(0, Direction::Forward);
        s.reveal_replace();
        assert!(s.is_replace_active(), "replace row is revealed");
        assert!(!s.is_editing_replacement(), "focus stays on the find field");
        // Idempotent: a second reveal never steals focus back once you've moved on.
        s.toggle_replace(); // Tab -> switch to replace
        assert!(s.is_editing_replacement());
        s.reveal_replace();
        assert!(s.is_editing_replacement(), "reveal_replace never yanks focus");
        // Cmd-R AGAIN (focus_replacement) forces focus into the replacement.
        let mut s2 = SearchState::start(0, Direction::Forward);
        s2.reveal_replace();
        assert!(!s2.is_editing_replacement());
        s2.focus_replacement();
        assert!(s2.is_replace_active() && s2.is_editing_replacement());
    }

    /// CLICK-TO-SWITCH-FIELD's pure state change: a press on the REPLACE row
    /// (`focus_replacement`) edits the replacement; a press on the FIND row
    /// (`focus_query`) returns to the query — and `focus_query` leaves the replace
    /// row revealed (a click never hides it). These are the two doors
    /// `App::panel_click` drives off `TextPipeline::panel_hit`.
    #[test]
    fn click_focus_doors_switch_the_edited_field() {
        let mut s = SearchState::start(0, Direction::Forward);
        s.focus_replacement(); // click the replace row
        assert!(s.is_replace_active());
        assert!(s.is_editing_replacement());
        s.focus_query(); // click the find row
        assert!(!s.is_editing_replacement(), "focus returns to the query");
        assert!(s.is_replace_active(), "the replace row stays revealed");
        // Idempotent: clicking the already-focused field is inert.
        s.focus_query();
        assert!(!s.is_editing_replacement());
    }

    #[test]
    fn replace_all_text_swaps_every_match() {
        // "line" three times; replace-all with "row" rewrites all three and the
        // returned text no longer matches the needle.
        let hay = "line one\nline two\nline three";
        let mut s = SearchState::start(0, Direction::Forward);
        for c in "line".chars() {
            s.push_char(c, hay);
        }
        s.toggle_replace();
        for c in "row".chars() {
            s.push_replace_char(c);
        }
        assert_eq!(s.hit_count(), 3);
        let out = s.replace_all_text(hay);
        assert_eq!(out, "row one\nrow two\nrow three");
        // A no-match query leaves the text untouched.
        let mut z = SearchState::start(0, Direction::Forward);
        z.push_char('z', hay);
        assert_eq!(z.replace_all_text(hay), hay);
    }

    #[test]
    fn replace_current_text_replaces_one_then_advances() {
        // Three "x" matches; replace-current swaps the first and advances current
        // to the next match, so repeated replace-current walks forward.
        let hay = "x.x.x";
        let mut s = SearchState::start(0, Direction::Forward);
        s.push_char('x', hay); // matches at 0,2,4; current = 0
        s.toggle_replace();
        s.push_replace_char('Y'); // single-char replacement keeps offsets simple
        assert_eq!(s.current_match(), Some(m(0, 1)));
        let t1 = s.replace_current_text(hay).unwrap();
        // First "x" became "Y"; current advanced to the next "x" (now at index 2).
        assert_eq!(t1, "Y.x.x");
        assert_eq!(s.current_match(), Some(m(2, 3)));
        // Replace again against the updated text: the second "x" becomes "Y".
        let t2 = s.replace_current_text(&t1).unwrap();
        assert_eq!(t2, "Y.Y.x");
        assert_eq!(s.current_match(), Some(m(4, 5)));
    }

    #[test]
    fn replace_current_text_handles_multibyte() {
        // The replacement splices by CHAR index, so multibyte needles/text are fine.
        let hay = "café au lait, café noir";
        let mut s = SearchState::start(0, Direction::Forward);
        for c in "café".chars() {
            s.push_char(c, hay);
        }
        s.toggle_replace();
        for c in "thé".chars() {
            s.push_replace_char(c);
        }
        let out = s.replace_current_text(hay).unwrap();
        assert_eq!(out, "thé au lait, café noir");
    }

    #[test]
    fn replace_writeback_roundtrips_buffer_and_lands_cursor() {
        // Reproduce the app.rs replace orchestration (set_text + cursor jump +
        // refind) WITHOUT winit — that glue is otherwise untested (replace can't be
        // driven headlessly). Mirrors App::search_replace_all / _current exactly.
        use crate::buffer::Buffer;

        // REPLACE-ALL: every match is swapped in one write-back; refind at the
        // origin then finds nothing (the replacement holds no needle), so the jump
        // is a no-op and the document reads fully rewritten.
        let mut buf = Buffer::from_str("line one\nline two\nline three");
        let mut st = SearchState::start(0, Direction::Forward);
        let q_hay = buf.text();
        for c in "line".chars() {
            st.push_char(c, &q_hay);
        }
        st.toggle_replace();
        for c in "row".chars() {
            st.push_replace_char(c);
        }
        // --- App::search_replace_all ---
        let hay = buf.text();
        let new_text = st.replace_all_text(&hay);
        let origin = st.origin();
        assert_ne!(new_text, hay, "replace-all must change the text");
        buf.set_text(&new_text);
        let new_hay = buf.text();
        st.refind(origin, &new_hay);
        if let Some(mm) = st.current_match() {
            buf.set_cursor(mm.start);
        }
        assert_eq!(buf.text(), "row one\nrow two\nrow three");
        assert_eq!(st.current_match(), None, "no needle remains after replace-all");

        // REPLACE-CURRENT: swap one match, write back, and land the cursor on the
        // NEXT match so a repeated Enter walks forward.
        let mut buf = Buffer::from_str("x.x.x");
        let mut st = SearchState::start(0, Direction::Forward);
        st.push_char('x', &buf.text());
        st.toggle_replace();
        st.push_replace_char('Y');
        let replace_current_once = |buf: &mut Buffer, st: &mut SearchState| {
            let hay = buf.text();
            if let Some(t) = st.replace_current_text(&hay) {
                buf.set_text(&t);
                if let Some(mm) = st.current_match() {
                    buf.set_cursor(mm.start);
                }
            }
        };
        replace_current_once(&mut buf, &mut st);
        assert_eq!(buf.text(), "Y.x.x");
        assert_eq!(st.current_match(), Some(m(2, 3)));
        assert_eq!(buf.cursor_char(), 2, "cursor lands on the next match");
        replace_current_once(&mut buf, &mut st);
        assert_eq!(buf.text(), "Y.Y.x");
        assert_eq!(buf.cursor_char(), 4);

        // MULTIBYTE: the cursor lands on the next match by CHAR index past the
        // multibyte replacement (é is 2 bytes but one char).
        let mut buf = Buffer::from_str("café au lait, café noir");
        let mut st = SearchState::start(0, Direction::Forward);
        for c in "café".chars() {
            st.push_char(c, &buf.text());
        }
        st.toggle_replace();
        for c in "thé".chars() {
            st.push_replace_char(c);
        }
        replace_current_once(&mut buf, &mut st);
        assert_eq!(buf.text(), "thé au lait, café noir");
        assert_eq!(buf.cursor_char(), 13, "next 'café' starts at char 13");
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

    #[test]
    fn start_with_query_prefills_and_matches_immediately() {
        let hay = "alpha beta alpha gamma alpha";
        let s = SearchState::start_with_query(0, Direction::Forward, "alpha", hay);
        assert_eq!(s.query(), "alpha");
        assert_eq!(s.hit_count(), 3);
        assert!(s.current_match().is_some(), "the prefilled query is matched, not blank");
        // An empty prefill behaves exactly like `start` (no matches, empty query).
        let blank = SearchState::start_with_query(0, Direction::Forward, "", hay);
        assert_eq!(blank.query(), "");
        assert_eq!(blank.hit_count(), 0);
    }

    #[test]
    fn last_query_remembers_and_is_reset_by_clear() {
        let _g = crate::testlock::serial();
        clear_last_query();
        assert_eq!(last_query(), "", "a fresh/cleared process remembers nothing");
        set_last_query("needle");
        assert_eq!(last_query(), "needle");
        // A LATER empty close never overwrites a still-useful remembered query
        // (an abandoned blank search shouldn't erase the last real one).
        set_last_query("");
        assert_eq!(last_query(), "needle");
        set_last_query("second");
        assert_eq!(last_query(), "second");
        clear_last_query(); // leave no residue for other tests reading the global
    }
}
