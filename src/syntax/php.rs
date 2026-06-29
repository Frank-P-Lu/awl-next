//! PHP syntax lexer — a minimal hand-written byte scanner following the
//! reference lexers ([`crate::syntax::rust`] / [`crate::syntax::python`]). It
//! emits only the four Alabaster roles and leaves everything else (keywords,
//! operators, `$variables`, identifiers, punctuation) as the default ink:
//!
//! - [`SynKind::Comment`]    — `// line`, `# line`, and `/* block */` comments
//!   (PHP block comments do NOT nest). A `#[` attribute is NOT a comment.
//! - [`SynKind::Str`]        — `'single'` / `"double"` (interpolated) strings as
//!   one span each, plus heredoc / nowdoc (`<<<LABEL … LABEL`).
//! - [`SynKind::Constant`]   — numeric literals (`0x`/`0o`/`0b`, floats, `_`
//!   separators) and the `true` / `false` / `null` literals (case-insensitive).
//! - [`SynKind::Definition`] — the identifier right after a `function` / `class` /
//!   `interface` / `trait` / `enum` / `const` introducer (case-insensitive).
//!
//! Span boundaries land on ASCII bytes (quotes, `/`, `#`, digits, ASCII
//! identifiers), so multibyte UTF-8 inside a string/comment rides inside the span
//! without ever splitting a char. Pure + single-pass. See the tests at the bottom
//! for the exact contract on a sample snippet.

use super::SynKind;
use std::ops::Range;

/// Introducers after which the next identifier is the DEFINITION name. Matched
/// case-insensitively (PHP keywords are case-insensitive).
const DEF_KEYWORDS: &[&str] = &["function", "class", "interface", "trait", "enum", "const"];
/// Identifiers that are CONSTANT literals (booleans + the `null` nil value).
/// Matched case-insensitively (`TRUE`/`True`/`true` are all the same in PHP).
const CONST_WORDS: &[&str] = &["true", "false", "null"];

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic() || c >= 0x80
}
fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric() || c >= 0x80
}
use super::matches_word_ci as matches_word;

pub fn spans(text: &str) -> Vec<(Range<usize>, SynKind)> {
    let b = text.as_bytes();
    let n = b.len();
    let mut out: Vec<(Range<usize>, SynKind)> = Vec::new();
    let mut i = 0usize;
    // Set when the previous significant token was a DEF_KEYWORD; the next
    // identifier is then the defined NAME.
    let mut expect_def = false;

    while i < n {
        let c = b[i];

        // --- line comment: `//` or `#` (but `#[` is an attribute, not a comment) ---
        if (c == b'/' && i + 1 < n && b[i + 1] == b'/')
            || (c == b'#' && !(i + 1 < n && b[i + 1] == b'['))
        {
            let end = super::scan_line_comment(b, i);
            out.push((i..end, SynKind::Comment));
            i = end;
            continue;
        }

        // --- block comment (PHP does NOT nest them) ---
        if c == b'/' && i + 1 < n && b[i + 1] == b'*' {
            let end = super::scan_block_comment(b, i, false);
            out.push((i..end, SynKind::Comment));
            i = end;
            continue;
        }

        // --- heredoc / nowdoc: `<<<LABEL … LABEL` ---
        if c == b'<' && i + 2 < n && b[i + 1] == b'<' && b[i + 2] == b'<' {
            if let Some(end) = heredoc(b, i) {
                out.push((i..end, SynKind::Str));
                i = end;
                expect_def = false;
                continue;
            }
        }

        // --- string: '…' or "…" ---
        if c == b'"' || c == b'\'' {
            let end = scan_string(b, i);
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- number literal ---
        if c.is_ascii_digit() {
            let start = i;
            i = scan_number(b, i);
            out.push((start..i, SynKind::Constant));
            expect_def = false;
            continue;
        }

        // --- `$variable`: skip the sigil so the name scans as a plain token ---
        if c == b'$' {
            expect_def = false;
            i += 1;
            continue;
        }

        // --- identifier / keyword ---
        if is_ident_start(c) {
            let start = i;
            i += 1;
            while i < n && is_ident_continue(b[i]) {
                i += 1;
            }
            let word = &text[start..i];
            if expect_def {
                out.push((start..i, SynKind::Definition));
                expect_def = false;
            } else if matches_word(CONST_WORDS, word) {
                out.push((start..i, SynKind::Constant));
            } else if matches_word(DEF_KEYWORDS, word) {
                expect_def = true;
            }
            continue;
        }

        // Any other byte (operator, punctuation, whitespace) stays default ink.
        if !c.is_ascii_whitespace() {
            // A non-identifier token after a def keyword means the name never
            // materialized — drop the expectation.
            expect_def = false;
        }
        i += 1;
    }

    out
}

/// Scan a single- or double-quoted string from the opening quote `q` to just past
/// its close (or EOF if unterminated). Honors `\\` escapes so an escaped quote
/// does not close the string; an interpolated double-quoted string is one span.
fn scan_string(b: &[u8], q: usize) -> usize {
    super::scan_quoted(b, q, b[q], false)
}

/// If a heredoc / nowdoc starts at `i` (`<<<LABEL`, `<<<"LABEL"`, or `<<<'LABEL'`
/// for a nowdoc), return the byte index just past the closing label; else `None`.
/// The closing label is matched at the start of a line, allowing PHP 7.3+ leading
/// indentation, and must not be glued to a longer identifier.
fn heredoc(b: &[u8], i: usize) -> Option<usize> {
    let n = b.len();
    let mut j = i + 3; // past `<<<`
    // PHP forbids space here, but be lenient with a stray space/tab.
    while j < n && (b[j] == b' ' || b[j] == b'\t') {
        j += 1;
    }
    let quote = if j < n && (b[j] == b'"' || b[j] == b'\'') {
        let q = b[j];
        j += 1;
        Some(q)
    } else {
        None
    };
    // The label is a normal identifier.
    let label_start = j;
    if j >= n || !is_ident_start(b[j]) {
        return None;
    }
    while j < n && is_ident_continue(b[j]) {
        j += 1;
    }
    let label = &b[label_start..j];
    // A quoted label must close with the same quote.
    if let Some(qc) = quote {
        if j < n && b[j] == qc {
            j += 1;
        } else {
            return None;
        }
    }
    // The rest of the opening line must be only whitespace before the newline.
    while j < n && b[j] != b'\n' {
        if b[j] != b' ' && b[j] != b'\t' && b[j] != b'\r' {
            return None;
        }
        j += 1;
    }
    if j >= n {
        return Some(n); // unterminated: run to EOF
    }
    j += 1; // past the opening newline

    // Scan body lines for the closing label.
    while j < n {
        let mut k = j;
        while k < n && (b[k] == b' ' || b[k] == b'\t') {
            k += 1;
        }
        if k + label.len() <= n && &b[k..k + label.len()] == label {
            let after = k + label.len();
            // The closer must not be part of a longer identifier (`LABEL` vs
            // `LABELX`); `;`, `,`, a newline, or EOF all end it cleanly.
            if after >= n || !is_ident_continue(b[after]) {
                return Some(after);
            }
        }
        // Advance to the next line.
        while j < n && b[j] != b'\n' {
            j += 1;
        }
        if j < n {
            j += 1;
        }
    }
    Some(n) // unterminated: run to EOF
}

/// Scan a numeric literal beginning at the digit `i`; returns the index just past
/// it. Accepts `0x`/`0o`/`0b` radixes, `_` separators, a fractional `.`, and an
/// exponent. A `.` that begins a method/property access on an int is not eaten.
fn scan_number(b: &[u8], i: usize) -> usize {
    super::scan_number(
        b,
        i,
        super::NumOpts { radix: b"xXoObB", radix_extra: b"", dot_dot_stops: true },
        is_ident_start,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::testutil::{at, has};

    #[test]
    fn line_comments_slash_and_hash() {
        let t = "$x = 1; // slash\n$y = 2; # hash\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["// slash", "# hash"], "{s:?}");
    }

    #[test]
    fn hash_attribute_is_not_a_comment() {
        // PHP 8 attribute `#[...]` must NOT recede as a comment.
        let t = "#[Route('/x')]\nfunction f() {}";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Comment).is_empty(), "{s:?}");
        // The string inside the attribute still styles.
        assert!(at(t, &s, SynKind::Str).contains(&"'/x'"), "{s:?}");
    }

    #[test]
    fn block_comment_is_not_nested() {
        let t = "/* a /* b */ c */ $x";
        let s = spans(t);
        // PHP block comments do NOT nest: the first `*/` closes it.
        assert!(has(&s, 0, 12, SynKind::Comment), "{s:?}");
    }

    #[test]
    fn strings_single_and_double() {
        let t = "$a = 'hi';\n$b = \"yo $a\";\n";
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        assert!(ss.contains(&"'hi'"), "{ss:?}");
        assert!(ss.contains(&"\"yo $a\""), "{ss:?}");
    }

    #[test]
    fn string_with_escaped_quote() {
        let t = r#"$s = "a\"b";"#;
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec![r#""a\"b""#], "{s:?}");
    }

    #[test]
    fn heredoc_and_nowdoc() {
        let t = "$a = <<<EOT\nhello $x\nEOT;\n$b = <<<'RAW'\nliteral $x\nRAW;\n";
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        assert!(ss.iter().any(|x| x.starts_with("<<<EOT") && x.ends_with("EOT")), "{ss:?}");
        assert!(ss.iter().any(|x| x.starts_with("<<<'RAW'") && x.ends_with("RAW")), "{ss:?}");
    }

    #[test]
    fn numbers_and_constants() {
        let t = "$a = 42; $b = 0xFF; $c = 3.14; $d = 1_000; $e = true; $f = null;";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["42", "0xFF", "3.14", "1_000", "true", "null"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn constants_are_case_insensitive() {
        let t = "$a = TRUE; $b = Null; $c = False;";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        assert!(cs.contains(&"TRUE") && cs.contains(&"Null") && cs.contains(&"False"), "{cs:?}");
    }

    #[test]
    fn definitions_after_introducers() {
        let t = "function frobnicate($x) {}\nclass Widget {}\ninterface Shape {}\ntrait T {}\nenum Suit {}\nconst MAX = 1;\n";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        for want in ["frobnicate", "Widget", "Shape", "T", "Suit", "MAX"] {
            assert!(ds.contains(&want), "missing {want}: {ds:?}");
        }
    }

    #[test]
    fn keyword_itself_is_not_styled() {
        // `function` stays default ink; only the NAME is a Definition.
        let t = "function main() {}";
        let s = spans(t);
        assert!(!has(&s, 0, 8, SynKind::Definition), "the `function` keyword must stay plain: {s:?}");
        assert!(has(&s, 9, 13, SynKind::Definition), "`main` is the definition: {s:?}");
    }

    #[test]
    fn variables_are_not_highlighted() {
        let t = "$result = compute($a, $b) + $offset;";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn reference_snippet() {
        let t = "<?php\n// add two\nfunction add($a, $b) {\n    $total = $a + $b; # sum\n    return $total;\n}\nconst MAX = 100;\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["// add two", "# sum"], "{s:?}");
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"add") && ds.contains(&"MAX"), "{ds:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"100"), "{s:?}");
    }
}
