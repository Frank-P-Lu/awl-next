//! WRITING NITS — the quiet mechanical-typo highlighter.
//!
//! A calm "tidy this" hint drawn as a MUTED STRAIGHT underline (mirrors the spell
//! squiggle geometry, but flat + neutral instead of wavy + error-red) under the
//! handful of GENUINE mechanical slips prose picks up. It is deliberately NOT a
//! grammar checker (SCOPE: awl is not a word processor) — it flags only three
//! per-line, byte-mechanical mistakes and NOTHING stylistic:
//!
//!   * MULTIPLE SPACES between words — a run of 2+ consecutive spaces that is NOT
//!     leading indentation (indentation is meaningful for lists / code, so a
//!     line's LEADING whitespace is never flagged). The whole run is flagged.
//!   * SPACE BEFORE PUNCTUATION — a space immediately before `, . ; : ! ? )`
//!     (an English-typography slip). The space AND the punctuation are flagged.
//!   * TRAILING WHITESPACE — whitespace at the end of a line, with ONE exception:
//!     EXACTLY TWO trailing spaces is a Markdown HARD LINE BREAK (intentional), so
//!     that alone is left alone; one space, three-or-more spaces, or a tab (any
//!     other trailing run) is flagged.
//!
//! Explicitly NOT flagged (voice / style, not mistakes): repeated punctuation
//! (`!!!`, `???`), multiple blank lines, and straight-vs-curly quotes. The
//! detector is ASCII/whitespace/punctuation-shaped, not word-based, so it never
//! looks at a character's script — a pure Japanese (or any non-Latin) prose line
//! with no stray double-space/trailing-whitespace is simply never flagged (see
//! `nits::tests::pure_japanese_prose_is_never_flagged`; a genuine mechanical slip
//! embedded in JP prose still flags exactly like in English — see
//! `japanese_prose_with_a_genuine_double_space_still_flags`; the sibling
//! [`crate::spell`] pinning lives in `spell::tests::real_dictionary_never_squiggles_pure_japanese_prose`
//! / `real_dictionary_mixed_japanese_and_english_only_flags_the_english_word`).
//!
//! The detector is a PURE per-line function ([`line_nits`]) returning half-open
//! CHAR-column spans, so it is unit-testable with no GPU; the renderer maps each
//! span to the muted straight underline with the SAME advance-aware layout the
//! spell squiggle + selection rects use. A process-global [`NITS_ON`] toggle
//! (DEFAULT ON — it is quiet + helpful, consistent with spellcheck) mirrors the
//! `page`/`focus` globals so the palette command, the config sticky pref, and the
//! render pipeline all read one place.
//!
//! TWO scope refinements live in the RENDERER, not here (this module stays a pure
//! per-line function of TEXT alone — no cursor, no buffer kind):
//!   * REVEAL-ON-CURSOR — the render pipeline (`render/rects.rs`) suppresses every
//!     nit on the line the CARET occupies: a line is judged only once you've moved
//!     off it (the active line is workspace, not manuscript), mirroring the
//!     existing `rule_lines`/`bullet_marks` reveal-on-cursor pattern.
//!   * CODE-BUFFER SCOPE — a recognized code buffer restricts nits to the lexer's
//!     own PROSE regions (comment + string spans), via [`span_in_prose_ranges`],
//!     mirroring `spell::misspellings_for`'s scoping exactly: alignment whitespace
//!     and other code-shaped spacing never nit. A prose/markdown buffer is
//!     untouched.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the writing-nits highlighter is active. DEFAULT ON: the app opens with
/// the quiet underlines showing (like spellcheck); the toggle hides them all.
static NITS_ON: AtomicBool = AtomicBool::new(true);

/// True when the writing-nits highlighter is active (read each frame by the
/// render pipeline to decide whether to build any nit underlines).
pub fn nits_on() -> bool {
    NITS_ON.load(Ordering::Relaxed)
}

/// Set the highlighter on/off explicitly (a settings write / the sticky-pref
/// launch-apply). The render pipeline reads [`nits_on`] each frame.
pub fn set_nits_on(on: bool) {
    NITS_ON.store(on, Ordering::Relaxed);
}

/// Flip the highlighter and return the now-active state (the "Writing nits"
/// palette command). Mirrors [`crate::page::toggle`].
pub fn toggle() -> bool {
    let next = !nits_on();
    NITS_ON.store(next, Ordering::Relaxed);
    next
}

/// The punctuation a space must NOT immediately precede (English typography): a
/// comma / period / semicolon / colon / bang / question mark / close-paren. The
/// OPEN paren is intentionally excluded — `foo (bar)` is correct spacing.
fn is_space_before_punct(c: char) -> bool {
    matches!(c, ',' | '.' | ';' | ':' | '!' | '?' | ')')
}

/// Detect the mechanical nits on ONE logical line, returning half-open CHAR-column
/// spans `[start, end)` to underline (in document order, coalesced so overlapping
/// rules — e.g. a double space that is ALSO a space-before-punct — merge into a
/// single span). Pure: no I/O, no GPU, so the rules are unit-testable directly.
///
/// See the module docs for the exact rules; the one subtlety is the Markdown
/// HARD-BREAK exception (exactly two trailing spaces) which is left un-flagged.
pub fn line_nits(line: &str) -> Vec<(usize, usize)> {
    line_nits_inner(line, false)
}

/// [`line_nits`] for a line that is a GFM TABLE ROW: identical, except the
/// MULTIPLE-SPACES rule is suppressed. Column alignment (`| Name  | Value |`) uses
/// interior runs of 2+ spaces DELIBERATELY, so flagging them is a false positive
/// (the banked bug). Space-before-punctuation and trailing-whitespace still flag —
/// those remain genuine slips inside a cell. The caller
/// (`render::rects::ensure_nit_protos`) selects this variant for lines the parsed
/// table spans mark as rows (see [`crate::markdown::MdKind::is_table_markup`]).
pub fn line_nits_table_row(line: &str) -> Vec<(usize, usize)> {
    line_nits_inner(line, true)
}

/// Shared body of [`line_nits`] / [`line_nits_table_row`]. `in_table_row` suppresses
/// ONLY the multiple-consecutive-spaces rule (intentional column alignment) while
/// keeping space-before-punctuation and trailing-whitespace.
fn line_nits_inner(line: &str, in_table_row: bool) -> Vec<(usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();
    if n == 0 {
        return Vec::new(); // an empty line has nothing to tidy (blank lines are fine)
    }
    let mut flag = vec![false; n];

    // The last non-whitespace char index. `None` => the whole line is whitespace.
    let last_content = chars.iter().rposition(|c| !c.is_whitespace());

    match last_content {
        None => {
            // ALL-WHITESPACE line: stray whitespace with nothing to indent, so treat
            // the whole run as TRAILING whitespace. The hard-break exception (exactly
            // two spaces) still applies, so a lone `  ` blank line is left alone.
            let two_space_break = n == 2 && chars[0] == ' ' && chars[1] == ' ';
            if !two_space_break {
                flag.iter_mut().for_each(|f| *f = true);
            }
        }
        Some(lc) => {
            // Leading indentation ends at the first non-whitespace char — meaningful
            // for lists / code, so it is NEVER flagged (skip it entirely below).
            let lead = chars
                .iter()
                .position(|c| !c.is_whitespace())
                .unwrap_or(0);

            // --- TRAILING WHITESPACE: the run after the last content char. ---
            let tw_start = lc + 1;
            let tw_len = n - tw_start;
            if tw_len > 0 {
                // EXACTLY two trailing SPACES is a Markdown hard line break — the one
                // intentional trailing-whitespace form, so it is left un-flagged. Any
                // other run (1 space, 3+ spaces, a tab, or a mixed space/tab pair) is
                // a genuine nit.
                let two_space_break =
                    tw_len == 2 && chars[tw_start] == ' ' && chars[tw_start + 1] == ' ';
                if !two_space_break {
                    (tw_start..n).for_each(|i| flag[i] = true);
                }
            }

            // --- INTERIOR (between the indent and the last content char). ---
            let mut i = lead;
            while i <= lc {
                if chars[i] == ' ' {
                    // Measure the maximal space run [i, j); `j` lands on the first
                    // non-space (guaranteed <= lc, since chars[lc] is content).
                    let mut j = i;
                    while j <= lc && chars[j] == ' ' {
                        j += 1;
                    }
                    let run_len = j - i;
                    // MULTIPLE SPACES between words: flag the whole 2+ run — UNLESS
                    // this is a table row, where interior 2+ runs are column alignment.
                    if run_len >= 2 && !in_table_row {
                        (i..j).for_each(|k| flag[k] = true);
                    }
                    // SPACE BEFORE PUNCTUATION: the run's LAST space sits right before
                    // the punctuation (covers a single space too). Flag the space + the
                    // punctuation char.
                    if j <= lc && is_space_before_punct(chars[j]) {
                        flag[j - 1] = true;
                        flag[j] = true;
                    }
                    i = j;
                } else {
                    i += 1;
                }
            }
        }
    }

    // Coalesce contiguous flagged columns into half-open spans.
    let mut spans = Vec::new();
    let mut k = 0;
    while k < n {
        if flag[k] {
            let start = k;
            while k < n && flag[k] {
                k += 1;
            }
            spans.push((start, k));
        } else {
            k += 1;
        }
    }
    spans
}

/// CODE-BUFFER SCOPE: true when the CHAR span `[start_col, end_col)` on `line`
/// (which begins at document BYTE offset `line_byte_start`) falls FULLY inside one
/// of `prose_ranges` (document byte ranges, sorted by start — the lexer-delimited
/// PROSE regions: comments + strings) — mirroring
/// [`crate::spell::misspelled_spans_scoped`]'s full-containment rule exactly. A
/// span straddling a prose-range boundary, or sitting entirely in plain code, is
/// NOT in scope. Pure (no I/O, no pipeline), so it's unit-testable directly; the
/// caller ([`crate::render::TextPipeline::ensure_nit_protos`]) supplies
/// `prose_ranges` only for a recognized CODE buffer — a prose/markdown buffer
/// never calls this, so its nits stay untouched.
pub fn span_in_prose_ranges(
    line: &str,
    line_byte_start: usize,
    start_col: usize,
    end_col: usize,
    prose_ranges: &[std::ops::Range<usize>],
) -> bool {
    let byte_of =
        |col: usize| line.char_indices().nth(col).map(|(b, _)| b).unwrap_or(line.len());
    let lo = line_byte_start + byte_of(start_col);
    let hi = line_byte_start + byte_of(end_col);
    prose_ranges.iter().any(|r| r.start <= lo && hi <= r.end)
}

/// Detect nits across a whole document, as `(line, start_col, end_col)` char
/// spans — the doc-level convenience the renderer's per-line loop mirrors and the
/// tests assert against. Splits on `\n` exactly like the buffer numbers lines.
#[cfg(test)]
pub fn document_nits(text: &str) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    for (li, line) in text.split('\n').enumerate() {
        for (s, e) in line_nits(line) {
            out.push((li, s, e));
        }
    }
    out
}

/// Serializes tests that read or write the process-global [`NITS_ON`], mirroring
/// `crate::page::test_lock()`: the render nit-underline tests flip it, so a
/// concurrent reader must not race the writer.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    // --- The toggle global. --------------------------------------------------

    #[test]
    fn default_on_and_toggle_flips() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_nits_on(true);
        assert!(nits_on(), "the highlighter defaults ON");
        assert!(!toggle(), "on -> off");
        assert!(!nits_on());
        assert!(toggle(), "off -> on");
        assert!(nits_on());
        set_nits_on(true);
    }

    // --- The three rules (flagged). ------------------------------------------

    #[test]
    fn double_space_between_words_is_flagged() {
        // The 2-space run at cols [1,3) is flagged; the single spaces are not.
        assert_eq!(line_nits("a  b"), vec![(1, 3)]);
        assert_eq!(line_nits("one two"), Vec::new(), "single spaces are fine");
        // A 3-space run flags the whole run.
        assert_eq!(line_nits("a   b"), vec![(1, 4)]);
    }

    #[test]
    fn space_before_punctuation_is_flagged() {
        // The space + the comma are flagged together (space@2, comma@3 -> [2,4)).
        assert_eq!(line_nits("hi , there"), vec![(2, 4)]);
        // Each listed punctuation mark triggers it.
        for (s, punct) in [
            ("a .", '.'),
            ("a ,", ','),
            ("a ;", ';'),
            ("a :", ':'),
            ("a !", '!'),
            ("a ?", '?'),
            ("a )", ')'),
        ] {
            let _ = punct;
            assert_eq!(line_nits(s), vec![(1, 3)], "{s:?} flags the space+punct");
        }
        // Correct spacing (no space before) is never flagged.
        assert_eq!(line_nits("hi, there"), Vec::new());
        // An OPEN paren after a space is CORRECT (`foo (bar)`), not a nit.
        assert_eq!(line_nits("foo (bar)"), Vec::new());
    }

    #[test]
    fn trailing_whitespace_is_flagged_except_the_two_space_hardbreak() {
        // One trailing space -> flagged.
        assert_eq!(line_nits("foo "), vec![(3, 4)]);
        // Exactly TWO trailing spaces -> the Markdown hard break -> NOT flagged.
        assert_eq!(line_nits("foo  "), Vec::new(), "2 trailing spaces = md hard break");
        // Three trailing spaces -> flagged (the whole run).
        assert_eq!(line_nits("foo   "), vec![(3, 6)]);
        // A trailing TAB -> flagged (not two spaces).
        assert_eq!(line_nits("foo\t"), vec![(3, 4)]);
        // A trailing space+tab pair (len 2 but not two spaces) -> flagged.
        assert_eq!(line_nits("foo \t"), vec![(3, 5)]);
    }

    // --- Leading indentation is meaningful (never flagged). ------------------

    #[test]
    fn leading_indentation_is_not_flagged() {
        // A list item's leading spaces (even many) are meaningful, never a nit.
        assert_eq!(line_nits("    - item"), Vec::new());
        assert_eq!(line_nits("\t\tcode()"), Vec::new());
        // ...but an INTERIOR double space on an indented line still flags.
        assert_eq!(line_nits("    - a  b"), vec![(7, 9)]);
    }

    // --- Stylistic choices are NOT flagged. ---------------------------------

    #[test]
    fn stylistic_choices_are_not_flagged() {
        // Repeated punctuation is voice, not a mistake.
        assert_eq!(line_nits("wow!!!"), Vec::new());
        assert_eq!(line_nits("really???"), Vec::new());
        assert_eq!(line_nits("wait..."), Vec::new());
        // Straight vs curly quotes are untouched.
        assert_eq!(line_nits("\u{201c}hi\u{201d} and 'yo'"), Vec::new());
        // A blank line (and thus consecutive blank lines) is empty -> nothing.
        assert_eq!(line_nits(""), Vec::new());
    }

    // --- Overlap / coalescing + doc-level. ----------------------------------

    #[test]
    fn overlapping_rules_coalesce_into_one_span() {
        // "hi  ." : the 2-space run [2,4) AND the space-before-period merge with the
        // period into a single span [2,5).
        assert_eq!(line_nits("hi  ."), vec![(2, 5)]);
    }

    #[test]
    fn document_nits_reports_line_indexed_spans() {
        let text = "clean line\nbad  spacing\nno trailing";
        // Only line 1 (the double space) is flagged.
        assert_eq!(document_nits(text), vec![(1, 3, 5)]);
        // Multiple blank lines across the doc contribute nothing.
        assert_eq!(document_nits("a\n\n\n\nb"), Vec::new());
    }

    // --- JAPANESE PROSE PINNING (the detector is script-agnostic, not word-
    // based, so clean JP prose is simply never flagged; a genuine mechanical
    // slip embedded in JP prose still flags exactly like it would in English). --

    #[test]
    fn pure_japanese_prose_is_never_flagged() {
        // A real Japanese sentence with ordinary (single, full-width-free) spacing
        // has nothing mechanically wrong with it -> zero nits, same as any clean
        // English sentence.
        assert_eq!(line_nits("今日は良い天気です。"), Vec::new());
        assert_eq!(
            document_nits("今日は良い天気です。\n猫が好きです。"),
            Vec::new()
        );
    }

    #[test]
    fn japanese_prose_with_a_genuine_double_space_still_flags() {
        // The rules are about WHITESPACE/PUNCTUATION shape, not word script, so a
        // real double-space between JP words is flagged exactly as it would be in
        // English (the columns are simply wherever the run sits).
        let ms = document_nits("猫が  好きです。");
        assert_eq!(ms.len(), 1, "a double space inside JP prose still nits: {ms:?}");
    }

    // --- TABLE-ROW EXEMPTION (`line_nits_table_row`). ------------------------

    #[test]
    fn table_row_multi_space_is_exempt_but_real_slips_still_flag() {
        // Column alignment: the interior 2+ space runs in a table row are NOT flagged.
        assert_eq!(line_nits_table_row("| Name  | Value |"), Vec::new());
        assert_eq!(line_nits_table_row("| foo    | 1 |"), Vec::new());
        // The SAME line under the normal rule DOES flag its alignment runs (proving
        // the exemption is what silences it, not the shape).
        assert!(
            !line_nits("| Name  | Value |").is_empty(),
            "the normal rule flags the aligned double spaces"
        );
        // A genuine space-before-punctuation slip inside a cell still flags even in a
        // table row (only the multi-space rule is suppressed).
        assert_eq!(line_nits_table_row("| a , b |"), vec![(3, 5)]);
        // Trailing whitespace still flags in a table row.
        assert_eq!(line_nits_table_row("| a | b | "), vec![(9, 10)]);
    }

    // --- CODE-BUFFER SCOPE (`span_in_prose_ranges`). -------------------------

    #[test]
    fn span_in_prose_ranges_requires_full_containment() {
        //           0123456789 1
        //           x  y // a  b
        let line = "x  y // a  b";
        // The comment ("// a  b") is cols/bytes [5, 12) — the lexer's Comment span.
        // The code-side double space (cols 1..3, "x  y") is NOT inside it.
        assert!(!span_in_prose_ranges(line, 0, 1, 3, &[5..12]));
        // The comment's own double space (cols 9..11, "a  b") IS inside it.
        assert!(span_in_prose_ranges(line, 0, 9, 11, &[5..12]));
        // No prose ranges at all -> nothing is ever in scope.
        assert!(!span_in_prose_ranges(line, 0, 9, 11, &[]));
    }

    #[test]
    fn span_in_prose_ranges_rejects_a_straddling_span() {
        let line = "\"hi\"  x";
        // The string prose range is bytes 0..4; a span [3,6) (closing quote through
        // the double space) straddles the boundary and must NOT be considered
        // fully inside.
        assert!(!span_in_prose_ranges(line, 0, 3, 6, &[0..4]));
    }

    #[test]
    fn span_in_prose_ranges_honors_a_nonzero_line_byte_start() {
        // A span on a LATER line: line_byte_start offsets both ends, so a prose
        // range expressed in DOCUMENT bytes still matches correctly.
        let line = "a  b";
        // Document byte range for this line is [10, 14); the interior double
        // space is document bytes [11, 13).
        assert!(span_in_prose_ranges(line, 10, 1, 3, &[10..14]));
        assert!(!span_in_prose_ranges(line, 10, 1, 3, &[0..4]), "a range on a different line must not match");
    }
}
