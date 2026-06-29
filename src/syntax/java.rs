//! Java syntax lexer — a minimal hand-written byte scanner following the
//! reference lexers in [`crate::syntax::rust`] and [`crate::syntax::python`]. It
//! emits only the four Alabaster roles and leaves everything else (keywords,
//! operators, identifiers, punctuation) as the default ink:
//!
//! - [`SynKind::Comment`]    — `// line` and `/* block */` comments (Java block
//!   comments do NOT nest — the first `*/` closes).
//! - [`SynKind::Str`]        — `"strings"`, `'c'` char literals, and `"""` text
//!   blocks (Java 13+ multi-line strings).
//! - [`SynKind::Constant`]   — numeric literals (incl. `0x`/`0b`/octal, `_`
//!   separators, `L`/`f`/`d` suffixes, floats/exponents) and the `true` /
//!   `false` / `null` literals.
//! - [`SynKind::Definition`] — the identifier right after a `class` / `interface`
//!   / `enum` / `record` introducer.
//!
//! Span boundaries land on ASCII bytes (quotes, `/`, digits, ASCII identifiers),
//! so multibyte UTF-8 inside a string/comment rides inside the span without ever
//! splitting a char. Pure + single-pass. See the tests below for the contract.

use super::SynKind;
use std::ops::Range;

/// Introducers after which the next identifier is the DEFINITION name.
const DEF_KEYWORDS: &[&str] = &["class", "interface", "enum", "record"];
/// Identifiers that are CONSTANT literals (booleans + the `null` nil value).
const CONST_WORDS: &[&str] = &["true", "false", "null"];

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c == b'$' || c.is_ascii_alphabetic()
}
fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c == b'$' || c.is_ascii_alphanumeric()
}

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

        // --- line comment ---
        if c == b'/' && i + 1 < n && b[i + 1] == b'/' {
            let start = i;
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            out.push((start..i, SynKind::Comment));
            continue;
        }

        // --- block comment (Java does NOT nest them) ---
        if c == b'/' && i + 1 < n && b[i + 1] == b'*' {
            let start = i;
            i += 2;
            while i < n {
                if b[i] == b'*' && i + 1 < n && b[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            out.push((start..i, SynKind::Comment));
            continue;
        }

        // --- text block: """ ... """ ---
        if c == b'"' && i + 2 < n && b[i + 1] == b'"' && b[i + 2] == b'"' {
            let end = scan_text_block(b, i);
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- normal string ---
        if c == b'"' {
            let end = scan_string(b, i, b'"');
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- char literal (Java has no lifetimes — `'` always opens one) ---
        if c == b'\'' {
            let end = scan_string(b, i, b'\'');
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
            } else if CONST_WORDS.contains(&word) {
                out.push((start..i, SynKind::Constant));
            } else if DEF_KEYWORDS.contains(&word) {
                expect_def = true;
            }
            continue;
        }

        // Any other byte (operator, punctuation, whitespace) stays default ink.
        // A non-identifier, non-whitespace token after a def keyword means the
        // name never materialized — drop the expectation.
        if !c.is_ascii_whitespace() {
            expect_def = false;
        }
        i += 1;
    }

    out
}

/// Scan a single-quoted string or char literal from the opening quote `q` to just
/// past its close (or EOF / end-of-line — neither crosses a newline in Java).
/// Honors `\\` escapes so an escaped quote does not close the literal.
fn scan_string(b: &[u8], q: usize, quote: u8) -> usize {
    let n = b.len();
    let mut i = q + 1;
    while i < n {
        match b[i] {
            b'\\' => i += 2,
            b'\n' => return i, // unterminated single-line literal: stop at newline
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    n
}

/// Scan a text block from the opening `"""` (at `q`) to just past the closing
/// `"""` (or EOF). Honors `\\` escapes.
fn scan_text_block(b: &[u8], q: usize) -> usize {
    let n = b.len();
    let mut i = q + 3;
    while i < n {
        if b[i] == b'\\' {
            i += 2;
        } else if b[i] == b'"' && i + 2 < n && b[i + 1] == b'"' && b[i + 2] == b'"' {
            return i + 3;
        } else if b[i] == b'"' && i + 2 == n && i + 1 < n && b[i + 1] == b'"' {
            return n; // closing triple flush at EOF
        } else {
            i += 1;
        }
    }
    n
}

/// Scan a numeric literal beginning at the digit `i`; returns the index just past
/// it. Accepts `0x`/`0b` radixes, `_` separators, a fractional `.`, an exponent,
/// and a trailing type suffix (`L`/`l`/`f`/`F`/`d`/`D`). A `.` not followed by a
/// digit (method call on an int literal) is NOT consumed.
fn scan_number(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i + 1;
    // Radix-prefixed integers: consume hex/alnum/underscore freely.
    if b[i] == b'0' && j < n && matches!(b[j], b'x' | b'X' | b'b' | b'B') {
        j += 1;
        while j < n && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
            j += 1;
        }
        return j;
    }
    while j < n {
        let c = b[j];
        if c.is_ascii_alphanumeric() || c == b'_' {
            j += 1;
        } else if c == b'.' {
            // A fractional point, but not an attribute / method access on an int
            // literal (`.` followed by a non-digit identifier start).
            if j + 1 < n && is_ident_start(b[j + 1]) {
                break;
            }
            j += 1;
        } else {
            break;
        }
    }
    j
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at<'a>(text: &'a str, s: &[(Range<usize>, SynKind)], k: SynKind) -> Vec<&'a str> {
        s.iter().filter(|(_, kk)| *kk == k).map(|(r, _)| &text[r.clone()]).collect()
    }
    fn has(s: &[(Range<usize>, SynKind)], lo: usize, hi: usize, k: SynKind) -> bool {
        s.iter().any(|(r, kk)| r.start == lo && r.end == hi && *kk == k)
    }

    #[test]
    fn line_comment() {
        let t = "int x = 1; // hi there\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["// hi there"], "{s:?}");
    }

    #[test]
    fn block_comment_does_not_nest() {
        let t = "/* a /* b */ c */ x";
        let s = spans(t);
        // The FIRST `*/` closes (Java block comments do not nest).
        assert!(has(&s, 0, 12, SynKind::Comment), "{s:?}");
    }

    #[test]
    fn string_with_escaped_quote() {
        let t = "String s = \"a\\\"b\";";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec!["\"a\\\"b\""], "{s:?}");
    }

    #[test]
    fn char_literal_and_escape() {
        let t = "char c = 'x'; char n = '\\n'; char u = '\\u0041';";
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        assert!(ss.contains(&"'x'"), "{ss:?}");
        assert!(ss.contains(&"'\\n'"), "{ss:?}");
        assert!(ss.contains(&"'\\u0041'"), "{ss:?}");
    }

    #[test]
    fn text_block_multiline() {
        let t = "String d = \"\"\"\nline one\nline two\"\"\";";
        let s = spans(t);
        assert_eq!(
            at(t, &s, SynKind::Str),
            vec!["\"\"\"\nline one\nline two\"\"\""],
            "{s:?}"
        );
    }

    #[test]
    fn numbers_bools_and_null() {
        let t = "int a = 42; long b = 0xFF_L; double c = 3.14e2; var f = 1_000L; boolean ok = true; Object z = null; boolean no = false;";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["42", "0xFF_L", "3.14e2", "1_000L", "true", "null", "false"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn member_access_not_eaten_by_number() {
        // `0.toString()` style: the `.` before an identifier must not be consumed.
        let t = "x = 5.length;";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Constant).contains(&"5"), "{s:?}");
    }

    #[test]
    fn definition_after_class_and_friends() {
        let t = "class Widget {}\ninterface Drawable {}\nenum Color {}\nrecord Point(int x) {}";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        for want in ["Widget", "Drawable", "Color", "Point"] {
            assert!(ds.contains(&want), "missing {want}: {ds:?}");
        }
    }

    #[test]
    fn keyword_itself_is_not_styled() {
        // `class` keyword stays default ink; only the NAME is a Definition.
        let t = "class Foo {}";
        let s = spans(t);
        assert!(!has(&s, 0, 5, SynKind::Definition), "the `class` keyword must stay plain: {s:?}");
        assert!(has(&s, 6, 9, SynKind::Definition), "`Foo` is the definition: {s:?}");
    }

    #[test]
    fn plain_code_has_no_spans() {
        // No comment / literal / def-keyword -> nothing highlighted (Alabaster).
        let t = "int result = compute(a, b) + offset;";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn reference_snippet() {
        let t = "// sum\nclass Adder {\n    int add(int a, int b) {\n        return a + b; // ok\n    }\n}\nstatic final int MAX = 100;\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["// sum", "// ok"], "{s:?}");
        assert!(at(t, &s, SynKind::Definition).contains(&"Adder"), "{s:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"100"), "{s:?}");
    }
}
