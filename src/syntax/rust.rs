//! Rust syntax lexer — the REFERENCE implementation (the template every other
//! `<lang>.rs` follows). A minimal hand-written scanner over the raw bytes; it
//! recognizes only what the four Alabaster roles need and leaves everything else
//! (keywords, operators, identifiers, punctuation) as the default ink:
//!
//! - [`SynKind::Comment`]    — `// line` and `/* block */` (nested) comments.
//! - [`SynKind::Str`]        — `"strings"`, `'c'` char literals, and raw strings
//!   (`r"..."`, `r#"..."#`, plus the `b`-prefixed byte variants).
//! - [`SynKind::Constant`]   — numeric literals (incl. `0x`/`0o`/`0b`, floats,
//!   `_` separators, type suffixes) and the `true` / `false` / `None` literals.
//! - [`SynKind::Definition`] — the identifier right after a `fn` / `struct` /
//!   `enum` / `trait` / `type` / `union` / `const` / `static` / `mod` introducer.
//!
//! Span boundaries always land on ASCII bytes (quotes, `/`, digits, ASCII
//! identifiers), so multibyte UTF-8 inside a string/comment rides along inside the
//! span without ever splitting a char. Pure + allocation-light: one pass, push as
//! we go. See the tests at the bottom for the exact contract on a sample snippet.

use super::SynKind;
use std::ops::Range;

/// The keyword introducers after which the next identifier is the DEFINITION name.
const DEF_KEYWORDS: &[&str] = &[
    "fn", "struct", "enum", "trait", "type", "union", "const", "static", "mod",
];

/// Identifiers that are CONSTANT literals (booleans + the `None` nil-style value).
const CONST_WORDS: &[&str] = &["true", "false", "None"];

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic()
}
fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
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

        // --- block comment (Rust nests them) ---
        if c == b'/' && i + 1 < n && b[i + 1] == b'*' {
            let start = i;
            i += 2;
            let mut depth = 1u32;
            while i < n && depth > 0 {
                if b[i] == b'/' && i + 1 < n && b[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && i + 1 < n && b[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            out.push((start..i, SynKind::Comment));
            continue;
        }

        // --- raw string: (b)r "..." / (b)r#"..."# ---
        if let Some(end) = raw_string(b, i) {
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- byte string / normal string: ("..." or b"...") ---
        if c == b'"' || (c == b'b' && i + 1 < n && b[i + 1] == b'"') {
            let q = if c == b'"' { i } else { i + 1 };
            let end = scan_string(b, q);
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- char literal vs lifetime ---
        if c == b'\'' {
            if let Some(end) = char_literal(b, i) {
                out.push((i..end, SynKind::Str));
                i = end;
                expect_def = false;
                continue;
            }
            // A lifetime (`'a`, `'static`): not a literal — skip the quote and let
            // the following identifier scan as a plain (un-styled) token.
            i += 1;
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
                // The name introduced by the preceding keyword.
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
        // Don't let stray punctuation between a keyword and its name clear the
        // expectation for `fn`/`struct`/… (only whitespace appears there anyway).
        if !c.is_ascii_whitespace() {
            // A non-identifier, non-whitespace token after a def keyword means the
            // name never materialized (e.g. EOF) — drop the expectation.
            expect_def = false;
        }
        i += 1;
    }

    out
}

/// If a raw string literal starts at `i` (`r"`, `r#"`, …, optionally `b`-prefixed),
/// return the byte index just past its close; else `None`. Handles any number of
/// `#` hashes (closing requires the matching `"###`).
fn raw_string(b: &[u8], i: usize) -> Option<usize> {
    let n = b.len();
    let mut j = i;
    if j < n && b[j] == b'b' {
        j += 1;
    }
    if j >= n || b[j] != b'r' {
        return None;
    }
    j += 1;
    let mut hashes = 0usize;
    while j < n && b[j] == b'#' {
        hashes += 1;
        j += 1;
    }
    if j >= n || b[j] != b'"' {
        return None;
    }
    j += 1; // past opening quote
    // Scan for a closing `"` followed by `hashes` `#`s.
    while j < n {
        if b[j] == b'"' {
            let mut k = j + 1;
            let mut h = 0;
            while h < hashes && k < n && b[k] == b'#' {
                h += 1;
                k += 1;
            }
            if h == hashes {
                return Some(k);
            }
        }
        j += 1;
    }
    Some(n) // unterminated: run to EOF
}

/// Scan a normal double-quoted string starting at the opening quote `q`; returns
/// the index just past the closing quote (or EOF if unterminated). Honors `\\`
/// escapes so an escaped quote does not close the string.
fn scan_string(b: &[u8], q: usize) -> usize {
    let n = b.len();
    let mut i = q + 1;
    while i < n {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    n
}

/// If a CHAR literal starts at `i` (`'x'`, `'\n'`, `'\u{1F}'`), return the index
/// just past the closing quote; `None` if it is actually a lifetime (`'a`).
fn char_literal(b: &[u8], i: usize) -> Option<usize> {
    let n = b.len();
    debug_assert_eq!(b[i], b'\'');
    let mut j = i + 1;
    if j >= n {
        return None;
    }
    if b[j] == b'\\' {
        // Escape: skip the backslash + escape body, then require a closing quote.
        j += 1;
        if j < n && b[j] == b'u' {
            // `\u{..}` — skip to the closing brace.
            while j < n && b[j] != b'}' {
                j += 1;
            }
            if j < n {
                j += 1;
            }
        } else if j < n {
            j += 1;
        }
        if j < n && b[j] == b'\'' {
            return Some(j + 1);
        }
        return None;
    }
    // Unescaped: a single (possibly multibyte) char then a closing quote means a
    // char literal; otherwise it is a lifetime.
    let ch_len = utf8_len(b[j]);
    let close = j + ch_len;
    if close < n && b[close] == b'\'' {
        Some(close + 1)
    } else {
        None
    }
}

/// Byte length of the UTF-8 char whose lead byte is `c`.
fn utf8_len(c: u8) -> usize {
    if c < 0x80 {
        1
    } else if c >> 5 == 0b110 {
        2
    } else if c >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

/// Scan a numeric literal beginning at the digit `i`; returns the index just past
/// it. Accepts `0x`/`0o`/`0b` radixes, `_` separators, a fractional `.`, an
/// exponent, and a trailing type suffix (`u32`, `f64`, …). A `..` range operator
/// after the integer is NOT consumed.
fn scan_number(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i + 1;
    // Radix-prefixed integers: consume hex/alnum/underscore freely.
    if b[i] == b'0' && j < n && matches!(b[j], b'x' | b'X' | b'o' | b'O' | b'b' | b'B') {
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
            // A fractional point — but not the `..` range op, and not a method call
            // on an integer (`.` followed by a non-digit ident start).
            if j + 1 < n && (b[j + 1] == b'.' || is_ident_start(b[j + 1])) {
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
    use crate::syntax::testutil::{at, has};

    #[test]
    fn line_comment() {
        let t = "let x = 1; // hi there\n";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Comment) == vec!["// hi there"], "{s:?}");
    }

    #[test]
    fn block_comment_nested() {
        let t = "/* a /* b */ c */ x";
        let s = spans(t);
        // The whole nested block is ONE comment span.
        assert!(has(&s, 0, 17, SynKind::Comment), "{s:?}");
    }

    #[test]
    fn string_with_escaped_quote() {
        let t = r#"let s = "a\"b";"#;
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec![r#""a\"b""#], "{s:?}");
    }

    #[test]
    fn raw_string_with_hashes() {
        let t = r####"let s = r#"he said "hi""#;"####;
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec![r##"r#"he said "hi""#"##], "{s:?}");
    }

    #[test]
    fn char_literal_and_lifetime() {
        let t = "let c = 'x'; fn f<'a>(r: &'a str) {}";
        let s = spans(t);
        // 'x' is a Str; 'a (a lifetime) is NOT.
        assert_eq!(at(t, &s, SynKind::Str), vec!["'x'"], "{s:?}");
    }

    #[test]
    fn char_escape_literal() {
        let t = r"let n = '\n';";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec![r"'\n'"], "{s:?}");
    }

    #[test]
    fn numbers_and_bools() {
        let t = "let a = 42; let b = 0xFF_u8; let c = 3.14; let ok = true;";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        assert!(cs.contains(&"42"), "{cs:?}");
        assert!(cs.contains(&"0xFF_u8"), "{cs:?}");
        assert!(cs.contains(&"3.14"), "{cs:?}");
        assert!(cs.contains(&"true"), "{cs:?}");
    }

    #[test]
    fn range_op_not_eaten_by_number() {
        let t = "for i in 0..5 {}";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        assert!(cs.contains(&"0") && cs.contains(&"5"), "ranges split: {cs:?}");
    }

    #[test]
    fn definition_after_fn_and_struct() {
        let t = "pub fn frobnicate(x: i32) {}\nstruct Widget;\nenum E {}\ntype Alias = u8;";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"frobnicate"), "{ds:?}");
        assert!(ds.contains(&"Widget"), "{ds:?}");
        assert!(ds.contains(&"E"), "{ds:?}");
        assert!(ds.contains(&"Alias"), "{ds:?}");
    }

    #[test]
    fn keyword_itself_is_not_styled() {
        // `fn` keyword stays default ink; only the NAME is a Definition.
        let t = "fn main() {}";
        let s = spans(t);
        assert!(!has(&s, 0, 2, SynKind::Definition), "the `fn` keyword must stay plain: {s:?}");
        assert!(has(&s, 3, 7, SynKind::Definition), "`main` is the definition: {s:?}");
    }

    #[test]
    fn plain_code_has_no_spans() {
        // No comment / literal / def-keyword -> nothing highlighted (Alabaster).
        let t = "let result = compute(a, b) + offset;";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn reference_snippet() {
        // A compact end-to-end snippet asserting all four roles at once.
        let t = "// sum\nfn add(a: i32, b: i32) -> i32 {\n    let total = a + b; // ok\n    return total;\n}\nconst MAX: u32 = 100;\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["// sum", "// ok"], "{s:?}");
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"add") && ds.contains(&"MAX"), "{ds:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"100"), "{s:?}");
    }
}
