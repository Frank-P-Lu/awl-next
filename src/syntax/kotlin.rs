//! Kotlin syntax lexer — a minimal hand-written byte scanner following the
//! reference lexers (`rust.rs` / `python.rs`). It emits only the four Alabaster
//! roles and leaves everything else (keywords, operators, identifiers,
//! punctuation) as the default ink:
//!
//! - [`SynKind::Comment`]    — `// line` and `/* block */` comments. Kotlin
//!   block comments NEST, so the whole nested run is one span.
//! - [`SynKind::Str`]        — `"strings"` (with `\` escapes), `'c'` char
//!   literals, and raw/multiline triple-quoted `"""..."""` (no escapes). A
//!   `$name` / `${expr}` interpolation rides INSIDE the one `Str` span.
//! - [`SynKind::Constant`]   — numeric literals (`0x`/`0b` radixes, `_`
//!   separators, `L`/`u`/`f` suffixes, floats) and `true` / `false` / `null`.
//! - [`SynKind::Definition`] — the identifier right after a `fun` / `class` /
//!   `interface` / `object` / `typealias` / `val` / `var` introducer.
//!
//! Span boundaries land on ASCII bytes (quotes, `/`, digits, ASCII identifiers),
//! so multibyte UTF-8 inside a string/comment rides inside the span without ever
//! splitting a char. Pure + single-pass. See the tests below for the contract.

use super::SynKind;
use std::ops::Range;

/// Introducers after which the next identifier is the DEFINITION name. `val`/`var`
/// cover the let-binding case; `enum class X` is caught by `class`.
const DEF_KEYWORDS: &[&str] = &[
    "fun", "class", "interface", "object", "typealias", "val", "var",
];

/// Identifiers that are CONSTANT literals (booleans + the `null` nil value).
const CONST_WORDS: &[&str] = &["true", "false", "null"];

use super::{is_ident_continue, is_ident_start};

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
            let end = super::scan_line_comment(b, i);
            out.push((i..end, SynKind::Comment));
            i = end;
            continue;
        }

        // --- block comment (Kotlin nests them) ---
        if c == b'/' && i + 1 < n && b[i + 1] == b'*' {
            let end = super::scan_block_comment(b, i, true);
            out.push((i..end, SynKind::Comment));
            i = end;
            continue;
        }

        // --- raw/multiline triple-quoted string: """...""" (no escapes) ---
        if c == b'"' && i + 2 < n && b[i + 1] == b'"' && b[i + 2] == b'"' {
            let end = scan_triple(b, i);
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- normal double-quoted string (honors \ escapes) ---
        if c == b'"' {
            let end = scan_string(b, i);
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- char literal: 'x', '\n', 'A' ---
        if c == b'\'' {
            let end = scan_char(b, i);
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
        if !c.is_ascii_whitespace() {
            // A non-identifier token after a def keyword means the name never
            // materialized (e.g. `fun (` for a lambda receiver) — drop it.
            expect_def = false;
        }
        i += 1;
    }

    out
}

/// Scan a normal double-quoted string starting at the opening quote `q`; returns
/// the index just past the closing quote (or EOF / newline if unterminated). A
/// `\` escapes the next byte so an escaped quote does not close the string.
fn scan_string(b: &[u8], q: usize) -> usize {
    super::scan_quoted(b, q, b'"', true)
}

/// Scan a raw/multiline triple-quoted string from the opening `"""` (at `q`) to
/// just past the closing `"""` (or EOF). Raw strings have NO escapes.
fn scan_triple(b: &[u8], q: usize) -> usize {
    let n = b.len();
    let mut i = q + 3;
    while i < n {
        if b[i] == b'"' && i + 2 < n && b[i + 1] == b'"' && b[i + 2] == b'"' {
            return i + 3;
        }
        // A flush close at EOF (`..."""` with nothing after).
        if b[i] == b'"' && i + 3 == n && b[i + 1] == b'"' && b[i + 2] == b'"' {
            return n;
        }
        i += 1;
    }
    n
}

/// Scan a char literal starting at the opening quote `i` (`'x'`, `'\n'`,
/// `'A'`); returns the index just past the closing quote (or EOF). Kotlin
/// has no lifetimes, so a lone quote simply runs to its mate.
fn scan_char(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i + 1;
    while j < n {
        match b[j] {
            b'\\' => j += 2,
            b'\n' => return j,
            b'\'' => return j + 1,
            _ => j += 1,
        }
    }
    n
}

/// Scan a numeric literal beginning at the digit `i`; returns the index just past
/// it. Accepts `0x`/`0b` radixes, `_` separators, a fractional `.`, an exponent,
/// and trailing `L`/`u`/`U`/`f`/`F` suffixes. A `..` range operator after the
/// integer is NOT consumed.
fn scan_number(b: &[u8], i: usize) -> usize {
    super::scan_number(
        b,
        i,
        super::NumOpts { radix: b"xXbB", radix_extra: b"", dot_dot_stops: true },
        is_ident_start,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::testutil::{at, has};

    #[test]
    fn line_comment() {
        let t = "val x = 1 // hi there\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["// hi there"], "{s:?}");
    }

    #[test]
    fn block_comment_nested() {
        let t = "/* a /* b */ c */ x";
        let s = spans(t);
        // The whole nested block is ONE comment span (Kotlin nests).
        assert!(has(&s, 0, 17, SynKind::Comment), "{s:?}");
    }

    #[test]
    fn string_with_escaped_quote() {
        let t = r#"val s = "a\"b""#;
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec![r#""a\"b""#], "{s:?}");
    }

    #[test]
    fn interpolation_is_one_string_span() {
        let t = "val s = \"hi $name and ${a.b}\"\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec!["\"hi $name and ${a.b}\""], "{s:?}");
    }

    #[test]
    fn triple_quoted_multiline() {
        let t = "val s = \"\"\"line one\nline \"two\"\"\"\"\n";
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        assert!(ss.iter().any(|x| x.starts_with("\"\"\"line one")), "{ss:?}");
    }

    #[test]
    fn char_literal_and_escape() {
        let t = "val c = 'x'; val n = '\\n'";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec!["'x'", "'\\n'"], "{s:?}");
    }

    #[test]
    fn numbers_and_constants() {
        let t = "val a = 42; val b = 0xFF_u; val c = 3.14; val d = 100L; val ok = true; val z = null";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["42", "0xFF_u", "3.14", "100L", "true", "null"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn range_op_not_eaten_by_number() {
        let t = "for (i in 0..5) {}";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        assert!(cs.contains(&"0") && cs.contains(&"5"), "ranges split: {cs:?}");
    }

    #[test]
    fn definitions_after_keywords() {
        let t = "fun frobnicate() {}\nclass Widget\ninterface Shape\nobject Single\ntypealias Alias = Int\nval count = 0\nvar total = 0";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        for want in ["frobnicate", "Widget", "Shape", "Single", "Alias", "count", "total"] {
            assert!(ds.contains(&want), "missing def {want}: {ds:?}");
        }
    }

    #[test]
    fn keyword_itself_is_not_styled() {
        // `fun` keyword stays default ink; only the NAME is a Definition.
        let t = "fun main() {}";
        let s = spans(t);
        assert!(!has(&s, 0, 3, SynKind::Definition), "the `fun` keyword must stay plain: {s:?}");
        assert!(has(&s, 4, 8, SynKind::Definition), "`main` is the definition: {s:?}");
    }

    #[test]
    fn plain_code_has_no_spans() {
        // No comment / literal / def-keyword -> nothing highlighted (Alabaster).
        let t = "result = compute(a, b) + offset";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn reference_snippet() {
        let t = "// sum\nfun add(a: Int, b: Int): Int {\n    val total = a + b // ok\n    return total\n}\nconst val MAX = 100\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["// sum", "// ok"], "{s:?}");
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"add") && ds.contains(&"MAX"), "{ds:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"100"), "{s:?}");
    }
}
