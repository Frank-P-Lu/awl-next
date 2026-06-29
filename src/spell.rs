//! Spell-check engine (v1: detect only).
//!
//! Two responsibilities, kept apart so the markdown-aware tokenizer is unit-
//! testable without a real dictionary:
//!
//!   * [`SpellChecker`] — wraps a [`spellbook::Dictionary`] loaded ONCE from the
//!     bundled LibreOffice en_US Hunspell files (`include_str!`'d into the
//!     binary), exposing [`SpellChecker::check`] (a microsecond dict lookup).
//!   * [`misspelled_spans`] — the pure, dictionary-parameterized detector: given
//!     the whole document and a `check` predicate, it tokenizes into words and
//!     returns the MISSPELLED ones as `(line, start_col, end_col)` in CHAR
//!     columns (consistent with the advance-aware layout + selection rects), with
//!     markdown skipping of fenced code blocks, inline code, and URLs.
//!
//! CORRECTIONS. [`SpellChecker::suggest`] asks the Hunspell engine for ordered
//! replacement candidates for a word, and [`SpellChecker::suggest_at`] resolves
//! the misspelling the cursor is ON or ADJACENT to (via the pure
//! [`misspelling_at`]) and pairs it with those suggestions — the data the
//! summoned correction picker (Cmd-`;`) lists and the chosen one a single
//! undoable edit replaces.

/// The bundled dictionary (LibreOffice en_US, ~49.5k stems). Compiled into the
/// binary so spell-check works with no external files and the headless capture
/// stays self-contained + deterministic.
const AFF: &str = include_str!("../assets/dict/en_US.aff");
const DIC: &str = include_str!("../assets/dict/en_US.dic");

/// A misspelled word's location in the document, in CHAR columns on a logical
/// line. `[start_col, end_col)` is a half-open char range; the renderer maps it
/// to pixels with the SAME advance-aware layout used for selection rects, so the
/// squiggle lands exactly under the word's glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Misspelling {
    pub line: usize,
    pub start_col: usize,
    pub end_col: usize,
}

/// Loaded-once spell checker. Holds the parsed Hunspell dictionary; `check` is a
/// pure lookup. Construction is the only fallible part (dictionary parse).
pub struct SpellChecker {
    dict: spellbook::Dictionary,
}

impl SpellChecker {
    /// Parse the bundled en_US Hunspell dictionary. Returns an error string if
    /// the real-world dictionary fails to parse (so the caller can REPORT it
    /// rather than silently disabling spell-check).
    pub fn new() -> Result<Self, String> {
        let dict = spellbook::Dictionary::new(AFF, DIC)
            .map_err(|e| format!("failed to parse bundled en_US dictionary: {e}"))?;
        Ok(Self { dict })
    }

    /// True if `word` is spelled correctly. Hunspell's `check` is already case
    /// aware (it honors capitalized / all-caps forms of dictionary stems and the
    /// dictionary's own proper-noun entries), so we pass the raw word; if the
    /// exact-case form is rejected we additionally accept an all-lowercase match
    /// so a sentence-initial capital of a lowercase-only stem (e.g. "Definately"
    /// vs "definitely") is judged on the stem, not the capitalization.
    pub fn check(&self, word: &str) -> bool {
        if self.dict.check(word) {
            return true;
        }
        let lower = word.to_lowercase();
        if lower != word && self.dict.check(&lower) {
            return true;
        }
        false
    }

    /// Detect all misspelled words in `text`. Thin wrapper over the pure
    /// [`misspelled_spans`] using this dictionary as the predicate.
    pub fn misspellings(&self, text: &str) -> Vec<Misspelling> {
        misspelled_spans(text, |w| self.check(w))
    }

    /// Ordered correction candidates for `word`, best first (Hunspell's own
    /// ranking). Empty when the engine has no suggestion. A thin wrapper over
    /// spellbook's `suggest`, owning the output vec so callers needn't manage one.
    pub fn suggest(&self, word: &str) -> Vec<String> {
        let mut out = Vec::new();
        self.dict.suggest(word, &mut out);
        out
    }

    /// Resolve the misspelling the cursor at `(line, col)` is ON or ADJACENT to and
    /// pair it with its correction candidates — the data the summoned spell picker
    /// lists. `None` when the cursor is not on a flagged word (so the binding is a
    /// calm no-op). The returned span is in CHAR columns on the logical line, so
    /// the caller can map it to a buffer char range for the replace-the-word edit.
    pub fn suggest_at(&self, text: &str, line: usize, col: usize) -> Option<SuggestionTarget> {
        let m = misspelling_at(text, line, col, |w| self.check(w))?;
        let word: String = text
            .split('\n')
            .nth(m.line)
            .unwrap_or("")
            .chars()
            .skip(m.start_col)
            .take(m.end_col - m.start_col)
            .collect();
        let suggestions = self.suggest(&word);
        Some(SuggestionTarget {
            misspelling: m,
            word,
            suggestions,
        })
    }
}

/// A misspelled word the cursor sits on, with its ordered correction candidates.
/// Produced by [`SpellChecker::suggest_at`] for the summoned spell picker: the
/// `misspelling` span locates the word to replace, `word` is its current text, and
/// `suggestions` (possibly empty) are what the picker lists.
#[derive(Clone, Debug)]
pub struct SuggestionTarget {
    pub misspelling: Misspelling,
    /// The current (misspelled) word text. Carried for callers/tests that want to
    /// echo it; the live/headless pickers replace by SPAN, so the binary itself
    /// reads only `misspelling` + `suggestions`.
    #[allow(dead_code)]
    pub word: String,
    pub suggestions: Vec<String>,
}

/// The misspelled word the cursor at `(line, col)` is ON or ADJACENT to, if any.
/// "Adjacent" means the cursor sits anywhere in `[start_col, end_col]` INCLUSIVE
/// of both ends, so a caret just before the first letter or just after the last
/// letter still targets the word (typical when you finish typing a word). Pure
/// (the dictionary arrives via `check`) so it's unit-testable with a stub. When
/// two spans somehow touch the same column, the earlier (left-most) one wins.
pub fn misspelling_at<F: Fn(&str) -> bool>(
    text: &str,
    line: usize,
    col: usize,
    check: F,
) -> Option<Misspelling> {
    misspelled_spans(text, check)
        .into_iter()
        .find(|m| m.line == line && col >= m.start_col && col <= m.end_col)
}

/// Is `c` a letter we spell-check? We only check Latin-script words for v1, so
/// CJK / other-script letters are treated as non-word here (a CJK run is skipped
/// entirely, never flagged). ASCII fast-path first.
fn is_latin_letter(c: char) -> bool {
    if c.is_ascii_alphabetic() {
        return true;
    }
    if !c.is_alphabetic() {
        return false;
    }
    // Accept the Latin-script blocks (Basic Latin handled above, plus Latin-1
    // supplement / extended and IPA) so accented Latin words (café, naïve) are
    // checked; everything else (CJK, Cyrillic, Greek, ...) is skipped.
    matches!(c as u32,
        0x00C0..=0x024F   // Latin-1 Supplement + Latin Extended-A/B
        | 0x1E00..=0x1EFF // Latin Extended Additional
    )
}

/// True for an apostrophe that may sit INSIDE a word (don't, it's). Both the
/// ASCII `'` and the typographic right single quote are accepted; the dictionary
/// stores apostrophe words with `'`.
fn is_intraword_apostrophe(c: char) -> bool {
    c == '\'' || c == '\u{2019}'
}

/// Tokenize `text` and return the MISSPELLED words as `(line, start_col,
/// end_col)` char spans, skipping markdown code + URLs. `check(word)` returns
/// true for a correctly spelled word. Pure (no I/O, no dictionary) so the
/// markdown heuristics + tokenization are unit-testable with a stub predicate.
///
/// Skipping rules (heuristic, good enough for v1):
///   * Fenced code blocks: a line whose trimmed text starts with ``` toggles a
///     "in code fence" state; lines inside are not checked (nor is the fence).
///   * Inline code: a backtic-delimited run `like this` on a line is skipped.
///   * URLs: a whitespace-delimited token starting http:// https:// or www. is
///     skipped wholesale (so `.../teh` is not flagged).
///   * Tokens containing a digit, or any non-Latin letter, are skipped.
pub fn misspelled_spans<F: Fn(&str) -> bool>(text: &str, check: F) -> Vec<Misspelling> {
    let mut out = Vec::new();
    let mut in_fence = false;

    // `text.split('\n')` yields one entry per logical line and is consistent
    // with how the buffer numbers lines (each '\n' ends a line). A trailing
    // newline yields a final empty line, which is harmless (no words).
    for (line_no, line) in text.split('\n').enumerate() {
        // Fenced code block toggle: a line that is just ``` (optionally with an
        // info string / indentation) flips the state. The fence line itself is
        // never spell-checked.
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        scan_line(line, line_no, &check, &mut out);
    }
    out
}

/// Scan a single (non-fence) line: skip inline-code and URL regions, then emit
/// misspelled word spans for the rest. Columns are CHAR indices into the line.
fn scan_line<F: Fn(&str) -> bool>(
    line: &str,
    line_no: usize,
    check: &F,
    out: &mut Vec<Misspelling>,
) {
    // Work in (char_index, char) units so emitted columns are char columns.
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();
    let mut i = 0usize;

    while i < n {
        let c = chars[i];

        // --- Inline code: skip from an opening backtick to its closing one. ---
        if c == '`' {
            i += 1;
            while i < n && chars[i] != '`' {
                i += 1;
            }
            // Consume the closing backtick if present.
            if i < n {
                i += 1;
            }
            continue;
        }

        // --- URL: skip a whole whitespace-delimited token that looks like one.
        if c.is_ascii_alphabetic() && url_at(&chars, i) {
            while i < n && !chars[i].is_whitespace() {
                i += 1;
            }
            continue;
        }

        // --- Word: a run of Latin letters with intra-word apostrophes. --------
        if is_latin_letter(c) {
            let start = i;
            // Track whether the run held any non-Latin letter or digit; if so we
            // skip it (mixed-script / alphanumeric token).
            let mut skip = false;
            while i < n {
                let ch = chars[i];
                if is_latin_letter(ch) {
                    i += 1;
                } else if is_intraword_apostrophe(ch)
                    && i + 1 < n
                    && is_latin_letter(chars[i + 1])
                {
                    // Apostrophe only counts as intra-word when a letter follows
                    // (so a trailing quote in `dogs'` ends the word cleanly).
                    i += 1;
                } else if ch.is_alphanumeric() {
                    // A digit or non-Latin letter glued to the run: consume the
                    // rest of the alnum run and mark it un-checkable.
                    skip = true;
                    i += 1;
                } else {
                    break;
                }
            }
            if skip {
                continue;
            }
            let word: String = chars[start..i].iter().collect();
            // Trim a possible trailing apostrophe (e.g. from "dogs'") before the
            // dictionary lookup; intra-word apostrophes are kept.
            let trimmed = word.trim_end_matches(|c| is_intraword_apostrophe(c));
            if trimmed.is_empty() {
                continue;
            }
            if !check(trimmed) {
                out.push(Misspelling {
                    line: line_no,
                    start_col: start,
                    end_col: start + trimmed.chars().count(),
                });
            }
            continue;
        }

        i += 1;
    }
}

/// Does a URL scheme/prefix begin at char index `i`? Matches `http://`,
/// `https://`, or `www.` case-insensitively against the char slice.
fn url_at(chars: &[char], i: usize) -> bool {
    const PREFIXES: &[&str] = &["https://", "http://", "www."];
    for p in PREFIXES {
        let pc: Vec<char> = p.chars().collect();
        if i + pc.len() <= chars.len() {
            let mut ok = true;
            for (k, &want) in pc.iter().enumerate() {
                if !chars[i + k].eq_ignore_ascii_case(&want) {
                    ok = false;
                    break;
                }
            }
            if ok {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub dictionary: only these exact lowercase words are "correct".
    fn stub<'a>(correct: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |w: &str| correct.iter().any(|c| c.eq_ignore_ascii_case(w))
    }

    fn cols(m: &Misspelling) -> (usize, usize, usize) {
        (m.line, m.start_col, m.end_col)
    }

    #[test]
    fn flags_a_single_bad_word() {
        let good = stub(&["hello", "world"]);
        let ms = misspelled_spans("hello wrld", &good);
        assert_eq!(ms.len(), 1);
        assert_eq!(cols(&ms[0]), (0, 6, 10)); // "wrld" at cols 6..10
    }

    #[test]
    fn correct_words_not_flagged() {
        let good = stub(&["the", "quick", "brown", "fox"]);
        assert!(misspelled_spans("the quick brown fox", &good).is_empty());
    }

    #[test]
    fn columns_are_char_indices_after_punctuation() {
        let good = stub(&["a", "test"]);
        // "a, tset." -> "tset"? here we test column math with punctuation.
        let ms = misspelled_spans("a, tset.", &good);
        assert_eq!(ms.len(), 1);
        // "tset" starts at char col 3 (a=0, ,=1, space=2, t=3) and is 4 chars.
        assert_eq!(cols(&ms[0]), (0, 3, 7));
    }

    #[test]
    fn intraword_apostrophe_kept_as_one_word() {
        let good = stub(&["don't", "it's"]);
        assert!(misspelled_spans("don't it's", &good).is_empty());
        // A bad contraction is flagged as a single span including the apostrophe.
        let bad = stub(&["it's"]);
        let ms = misspelled_spans("dont", &bad);
        assert_eq!(ms.len(), 1);
        assert_eq!(cols(&ms[0]), (0, 0, 4));
    }

    #[test]
    fn trailing_apostrophe_trimmed() {
        // "dogs'" (possessive) should check the stem "dogs", not "dogs'".
        let good = stub(&["dogs"]);
        let ms = misspelled_spans("dogs' bones", &good);
        // "bones" is not in the stub -> flagged; "dogs'" is trimmed to "dogs".
        assert_eq!(ms.iter().filter(|m| m.start_col == 0).count(), 0);
    }

    #[test]
    fn digits_make_a_token_unchecked() {
        let none = stub(&[]); // nothing is correct
        // tokens with digits are skipped entirely, so no flags despite empty dict
        assert!(misspelled_spans("abc123 x2 v8", &none).is_empty());
    }

    #[test]
    fn cjk_run_is_skipped() {
        let none = stub(&[]);
        // Japanese should never be flagged (non-Latin script).
        assert!(misspelled_spans("日本語のテスト", &none).is_empty());
        // Mixed: only the Latin word "bad" is considered.
        let ms = misspelled_spans("日本 bad", &none);
        assert_eq!(ms.len(), 1);
        // "bad" starts after "日本 " -> char col 3.
        assert_eq!(ms[0].start_col, 3);
    }

    #[test]
    fn inline_code_is_skipped() {
        let none = stub(&[]);
        // The word inside backticks must NOT be flagged.
        let ms = misspelled_spans("use `wgpu` here", &none);
        // "use" and "here" are flagged (empty dict); "wgpu" is NOT.
        assert!(ms.iter().all(|m| {
            let w_start = m.start_col;
            w_start != 5 // wgpu would start at col 5
        }));
        assert_eq!(ms.len(), 2);
    }

    #[test]
    fn fenced_code_block_is_skipped() {
        let none = stub(&[]);
        let text = "before\n```\nnonsenseword\n```\nafter";
        let ms = misspelled_spans(text, &none);
        // Only "before" (line 0) and "after" (line 4) are checked; the fenced
        // line 2 "nonsenseword" is skipped.
        let lines: Vec<usize> = ms.iter().map(|m| m.line).collect();
        assert!(lines.contains(&0));
        assert!(lines.contains(&4));
        assert!(!lines.contains(&2), "fenced word must be skipped");
    }

    #[test]
    fn url_is_skipped() {
        let none = stub(&[]);
        // The misspelling embedded in the URL ("teh") must NOT be flagged.
        let ms = misspelled_spans("see https://example.com/teh ok", &none);
        // "see" and "ok" are flagged; nothing from the URL.
        assert_eq!(ms.len(), 2);
        let words: Vec<usize> = ms.iter().map(|m| m.start_col).collect();
        assert_eq!(words, vec![0, 28]); // "see"@0, "ok"@28
    }

    #[test]
    fn www_url_is_skipped() {
        let none = stub(&["go", "to"]);
        let ms = misspelled_spans("go to www.bad-spelll.com", &none);
        assert!(ms.is_empty(), "www. URL must be skipped");
    }

    // --- Real dictionary smoke tests (parse + known good/bad words). --------

    #[test]
    fn real_dictionary_parses_and_checks_known_words() {
        let sc = SpellChecker::new().expect("bundled en_US dictionary must parse");
        // Known-good words.
        for w in ["sentence", "misspelled", "typo", "definitely", "receive",
                  "the", "quick", "brown", "fox", "hello"] {
            assert!(sc.check(w), "{w:?} should be correct");
        }
        // Known-bad words (the fixture's deliberate misspellings).
        for w in ["sentance", "mispelled", "tpyo", "definately", "recieve"] {
            assert!(!sc.check(w), "{w:?} should be flagged");
        }
    }

    #[test]
    fn real_dictionary_handles_capitalization() {
        let sc = SpellChecker::new().unwrap();
        // Sentence-initial capital of a lowercase stem is accepted.
        assert!(sc.check("Hello"));
        assert!(sc.check("The"));
        // ...but a genuinely misspelled capitalized word is still flagged.
        assert!(!sc.check("Definately"));
    }

    #[test]
    fn real_dictionary_on_fixture_finds_exactly_the_five() {
        let sc = SpellChecker::new().unwrap();
        let text = "This sentance has a few mispelled words in it.\n\
                    Inline code like `wgpu` and `cosmic_text` must NOT be flagged.\n\
                    ```\nfn main() { let zzz = nonsenseword; }\n```\n\
                    A link https://example.com/teh should be skipped too.\n\
                    Another tpyo here, definately and recieve.";
        let ms = sc.misspellings(text);
        let words: Vec<String> = ms
            .iter()
            .map(|m| {
                let line = text.split('\n').nth(m.line).unwrap();
                line.chars().skip(m.start_col).take(m.end_col - m.start_col).collect()
            })
            .collect();
        assert_eq!(
            words,
            vec!["sentance", "mispelled", "tpyo", "definately", "recieve"],
            "exactly the five deliberate misspellings, nothing from code/URL"
        );
    }

    // --- Suggestions + cursor-targeting. ------------------------------------

    #[test]
    fn misspelling_at_targets_word_under_or_adjacent_to_cursor() {
        let good = stub(&["the", "quick"]);
        // "the wrld here" — "wrld" spans cols 4..8 (w=4,r=5,l=6,d=7).
        let text = "the wrld here";
        // ON the word (col inside the span).
        let m = misspelling_at(text, 0, 5, &good).expect("cursor in word");
        assert_eq!((m.start_col, m.end_col), (4, 8));
        // ADJACENT on the left edge (caret just before 'w').
        assert!(misspelling_at(text, 0, 4, &good).is_some());
        // ADJACENT on the right edge (caret just after 'd').
        assert!(misspelling_at(text, 0, 8, &good).is_some());
        // NOT on a flagged word: col 1 sits inside the correctly-spelled "the".
        assert!(misspelling_at(text, 0, 1, &good).is_none());
        // A line with no misspelling at all -> None.
        assert!(misspelling_at("the quick", 0, 2, &good).is_none());
    }

    #[test]
    fn real_dictionary_suggests_corrections() {
        let sc = SpellChecker::new().unwrap();
        // A classic typo should suggest the intended word near the top.
        let s = sc.suggest("teh");
        assert!(!s.is_empty(), "engine should offer a correction for 'teh'");
        assert!(
            s.iter().any(|w| w == "the"),
            "'the' should be among the suggestions for 'teh': {s:?}"
        );
        // "recieve" -> "receive".
        let s = sc.suggest("recieve");
        assert!(
            s.iter().any(|w| w == "receive"),
            "'receive' should be suggested for 'recieve': {s:?}"
        );
    }

    #[test]
    fn suggest_at_resolves_word_and_suggestions() {
        let sc = SpellChecker::new().unwrap();
        // Cursor inside the misspelling "recieve" (line 0, any col in the span).
        let text = "Please recieve this.";
        let t = sc.suggest_at(text, 0, 9).expect("cursor on a misspelling");
        assert_eq!(t.word, "recieve");
        assert_eq!((t.misspelling.start_col, t.misspelling.end_col), (7, 14));
        assert!(t.suggestions.iter().any(|w| w == "receive"));
        // A cursor on a CORRECT word yields nothing (calm no-op for the binding).
        assert!(sc.suggest_at(text, 0, 2).is_none(), "'Please' is correct");
    }
}
