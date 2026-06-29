//! C syntax lexer — a minimal hand-written byte scanner following the reference
//! lexer in [`crate::syntax::rust`]. It recognizes only what the four Alabaster
//! roles need and leaves everything else (keywords, operators, identifiers,
//! punctuation, preprocessor directives) as the default ink:
//!
//! - [`SynKind::Comment`]    — `// line` and `/* block */` comments. (C block
//!   comments do NOT nest — the first `*/` closes.)
//! - [`SynKind::Str`]        — `"strings"` and `'c'` char literals, including the
//!   encoding prefixes `L`, `u`, `U`, and `u8` (`L"..."`, `u8"..."`, `U'x'`, …).
//! - [`SynKind::Constant`]   — numeric literals (decimal, `0x`/`0b` radix, octal,
//!   floats, `u`/`l`/`f` suffixes) and the `true` / `false` / `NULL` / `nullptr`
//!   literals.
//! - [`SynKind::Definition`] — the identifier right after a `struct` / `union` /
//!   `enum` introducer (C tags the name right after the keyword; full function /
//!   typedef-name detection needs a real parser, so we stay best-effort here).
//!
//! Span boundaries always land on ASCII bytes (quotes, `/`, digits, ASCII
//! identifiers), so multibyte UTF-8 inside a string/comment rides along inside the
//! span without ever splitting a char. Pure + allocation-light: one pass, push as
//! we go. See the tests at the bottom for the exact contract on a sample snippet.

use super::SynKind;
use std::ops::Range;

/// The keyword introducers after which the next identifier is the DEFINITION name.
/// C has no `fn`/`func`; the reliably-positioned names are the tag types.
const DEF_KEYWORDS: &[&str] = &["struct", "union", "enum"];

/// Identifiers that are CONSTANT literals (booleans + the nil-style values).
const CONST_WORDS: &[&str] = &["true", "false", "NULL", "nullptr"];

use super::{is_ident_continue, is_ident_start};

/// If a string/char encoding prefix (`L`, `u`, `U`, `u8`) begins at `i` and is
/// immediately followed by a quote, return the byte index of that quote; else
/// `None`.
fn string_prefix(b: &[u8], i: usize) -> Option<usize> {
    let n = b.len();
    match b[i] {
        b'L' | b'U' if i + 1 < n && (b[i + 1] == b'"' || b[i + 1] == b'\'') => Some(i + 1),
        b'u' => {
            if i + 2 < n && b[i + 1] == b'8' && (b[i + 2] == b'"' || b[i + 2] == b'\'') {
                Some(i + 2)
            } else if i + 1 < n && (b[i + 1] == b'"' || b[i + 1] == b'\'') {
                Some(i + 1)
            } else {
                None
            }
        }
        _ => None,
    }
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

        // --- block comment (C does NOT nest them) ---
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

        // --- prefixed string / char (L"..", u8"..", U'x', …) ---
        if let Some(q) = string_prefix(b, i) {
            let end = if b[q] == b'"' {
                scan_string(b, q)
            } else {
                char_literal(b, q)
            };
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- string literal ---
        if c == b'"' {
            let end = scan_string(b, i);
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- char literal ---
        if c == b'\'' {
            let end = char_literal(b, i);
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
            if DEF_KEYWORDS.contains(&word) {
                // A tag introducer — the NEXT identifier is the name. (Handles
                // `typedef struct Foo` too: `struct` re-arms the expectation.)
                expect_def = true;
            } else if expect_def {
                out.push((start..i, SynKind::Definition));
                expect_def = false;
            } else if CONST_WORDS.contains(&word) {
                out.push((start..i, SynKind::Constant));
            }
            continue;
        }

        // Any other byte (operator, punctuation, whitespace, `#`) stays default
        // ink. A non-identifier token between a tag keyword and its name means the
        // name never materialized (e.g. an anonymous `struct {`) — drop the
        // expectation, but let intervening whitespace ride.
        if !c.is_ascii_whitespace() {
            expect_def = false;
        }
        i += 1;
    }

    out
}

/// Scan a normal double-quoted string starting at the opening quote `q`; returns
/// the index just past the closing quote (or EOF if unterminated). Honors `\\`
/// escapes so an escaped quote does not close the string.
fn scan_string(b: &[u8], q: usize) -> usize {
    super::scan_quoted(b, q, b'"', false)
}

/// Scan a CHAR literal starting at the opening quote `q` (`'x'`, `'\n'`, `'\0'`);
/// returns the index just past the closing quote (or EOF if unterminated). Honors
/// `\\` escapes. Unlike Rust there are no lifetimes, so a `'` always opens a char.
fn char_literal(b: &[u8], q: usize) -> usize {
    let n = b.len();
    let mut i = q + 1;
    while i < n {
        match b[i] {
            b'\\' => i += 2,
            b'\'' => return i + 1,
            _ => i += 1,
        }
    }
    n
}

/// Scan a numeric literal beginning at the digit `i`; returns the index just past
/// it. Accepts `0x`/`0X` hex, `0b`/`0B` binary, octal, a fractional `.`, an
/// exponent, and trailing type suffixes (`u`, `l`, `ll`, `f`, …).
fn scan_number(b: &[u8], i: usize) -> usize {
    super::scan_number(
        b,
        i,
        super::NumOpts { radix: b"xXbB", radix_extra: b"", dot_dot_stops: false },
        is_ident_start,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::testutil::{at, has};

    #[test]
    fn line_comment() {
        let t = "int x = 1; // hi there\n";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Comment) == vec!["// hi there"], "{s:?}");
    }

    #[test]
    fn block_comment_does_not_nest() {
        let t = "/* a /* b */ c */ x";
        let s = spans(t);
        // C closes at the FIRST `*/` — `/* a /* b */` is the whole comment.
        assert!(has(&s, 0, 12, SynKind::Comment), "{s:?}");
    }

    #[test]
    fn string_with_escaped_quote() {
        let t = "char *s = \"a\\\"b\";";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec!["\"a\\\"b\""], "{s:?}");
    }

    #[test]
    fn wide_and_utf8_string_prefixes() {
        let t = "L\"wide\" u8\"utf\" U\"big\"";
        let s = spans(t);
        let strs = at(t, &s, SynKind::Str);
        assert!(strs.contains(&"L\"wide\""), "{strs:?}");
        assert!(strs.contains(&"u8\"utf\""), "{strs:?}");
        assert!(strs.contains(&"U\"big\""), "{strs:?}");
    }

    #[test]
    fn char_literal_and_escape() {
        let t = "char c = 'x'; char n = '\\n';";
        let s = spans(t);
        let strs = at(t, &s, SynKind::Str);
        assert!(strs.contains(&"'x'"), "{strs:?}");
        assert!(strs.contains(&"'\\n'"), "{strs:?}");
    }

    #[test]
    fn numbers_and_constants() {
        let t = "int a = 42; unsigned b = 0xFFu; double c = 3.14; long d = 0b1010; void *p = NULL; _Bool ok = true;";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        assert!(cs.contains(&"42"), "{cs:?}");
        assert!(cs.contains(&"0xFFu"), "{cs:?}");
        assert!(cs.contains(&"3.14"), "{cs:?}");
        assert!(cs.contains(&"0b1010"), "{cs:?}");
        assert!(cs.contains(&"NULL"), "{cs:?}");
        assert!(cs.contains(&"true"), "{cs:?}");
    }

    #[test]
    fn member_access_not_eaten_by_number() {
        // A leading-digit token must stop before a `.field` member access.
        let t = "x = a1.field;";
        let s = spans(t);
        // `a1` is an identifier (not a number start) -> nothing highlighted here.
        assert!(at(t, &s, SynKind::Constant).is_empty(), "{s:?}");
    }

    #[test]
    fn definition_after_struct_enum_union() {
        let t = "struct Widget { int x; };\nenum Color { RED };\nunion Pad { int i; };";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"Widget"), "{ds:?}");
        assert!(ds.contains(&"Color"), "{ds:?}");
        assert!(ds.contains(&"Pad"), "{ds:?}");
    }

    #[test]
    fn typedef_struct_names_the_tag() {
        // `typedef struct Node` -> `Node` is the (re-armed) definition; `struct`
        // itself stays plain.
        let t = "typedef struct Node Node;";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Definition).contains(&"Node"), "{s:?}");
        assert!(!has(&s, 8, 14, SynKind::Definition), "the `struct` keyword must stay plain: {s:?}");
    }

    #[test]
    fn keyword_itself_is_not_styled() {
        // The `struct` keyword stays default ink; only the NAME is a Definition.
        let t = "struct Foo {};";
        let s = spans(t);
        assert!(!has(&s, 0, 6, SynKind::Definition), "the `struct` keyword must stay plain: {s:?}");
        assert!(has(&s, 7, 10, SynKind::Definition), "`Foo` is the definition: {s:?}");
    }

    #[test]
    fn plain_code_and_keywords_have_no_spans() {
        // No comment / literal / def-keyword -> nothing highlighted (Alabaster).
        // `int`, `compute`, etc. are keywords/identifiers and ride the default ink.
        let t = "int result = compute(a, b) + offset;";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn include_directive_rides_default_ink() {
        // The `#include` directive stays plain; only the quoted header is a Str.
        let t = "#include <stdio.h>\n#include \"local.h\"\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec!["\"local.h\""], "{s:?}");
    }

    #[test]
    fn reference_snippet() {
        // A compact end-to-end snippet asserting all four roles at once.
        let t = "// sum\nstruct Acc { int n; };\nint add(int a, int b) {\n    int total = a + b; // ok\n    return total;\n}\n#define MAX 100\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["// sum", "// ok"], "{s:?}");
        assert!(at(t, &s, SynKind::Definition).contains(&"Acc"), "{s:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"100"), "{s:?}");
    }
}
