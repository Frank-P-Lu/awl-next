//! C++ syntax lexer — a minimal hand-written byte scanner in the shape of the
//! reference lexers (`rust.rs`, `python.rs`). It recognizes only what the four
//! Alabaster roles need and leaves everything else (keywords, operators,
//! identifiers, punctuation, preprocessor directives) as the default ink:
//!
//! - [`SynKind::Comment`]    — `// line` and `/* block */` comments (C++ block
//!   comments do NOT nest — the first `*/` closes them).
//! - [`SynKind::Str`]        — `"strings"` and `'c'` char literals, their encoding
//!   prefixes (`L`/`u`/`U`/`u8`), and raw strings `R"delim(...)delim"` (with the
//!   same prefixes, e.g. `LR"(...)"`). Escapes are honored so `\"` / `\'` don't
//!   close the literal.
//! - [`SynKind::Constant`]   — numeric literals (incl. `0x`/`0b` radixes, floats,
//!   hex-float `p` exponents, C++14 `'` digit separators, type suffixes) and the
//!   `true` / `false` / `nullptr` / `NULL` literals.
//! - [`SynKind::Definition`] — the identifier right after a `class` / `struct` /
//!   `union` / `enum` / `namespace` / `concept` introducer (best-effort; an
//!   `enum class Name` skips the inner `class` so `Name` is the definition).
//!
//! Span boundaries always land on ASCII bytes (quotes, `/`, digits, ASCII
//! identifiers), so multibyte UTF-8 inside a string/comment rides inside the span
//! without ever splitting a char. Pure + allocation-light: one pass, push as we go.
//! See the tests at the bottom for the exact contract on a sample snippet.

use super::SynKind;
use std::ops::Range;

/// Introducers after which the next identifier is the DEFINITION name.
const DEF_KEYWORDS: &[&str] = &["class", "struct", "union", "enum", "namespace", "concept"];
/// Identifiers that are CONSTANT literals (booleans + the nil-style values).
const CONST_WORDS: &[&str] = &["true", "false", "nullptr", "NULL"];

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

        // --- block comment (C++ does NOT nest them) ---
        if c == b'/' && i + 1 < n && b[i + 1] == b'*' {
            let end = super::scan_block_comment(b, i, false);
            out.push((i..end, SynKind::Comment));
            i = end;
            continue;
        }

        // --- raw string: R"d(...)d" with optional L/u/U/u8 prefix ---
        if let Some(end) = raw_string(b, i) {
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- normal string: "..." with optional L/u/U/u8 prefix ---
        if let Some(p) = enc_prefix_for(b, i, b'"') {
            let end = scan_string(b, i + p);
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- char literal: 'c' with optional L/u/U/u8 prefix ---
        if let Some(p) = enc_prefix_for(b, i, b'\'') {
            let end = scan_char(b, i + p);
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
            // NOT the shared `super::ident_role`: C++ must check the INTRODUCER
            // before the pending-name so `enum class Name` chains past the inner
            // `class` to `Name` (see `enum_class_names_the_type`), so it keeps its
            // own introducer-first order here.
            let word = &text[start..i];
            if CONST_WORDS.contains(&word) {
                out.push((start..i, SynKind::Constant));
                expect_def = false;
            } else if DEF_KEYWORDS.contains(&word) {
                // A def introducer. `enum class Name` chains two of them; keep
                // expecting so the NAME (not the inner `class`) is the definition.
                expect_def = true;
            } else if expect_def {
                // The name introduced by the preceding keyword.
                out.push((start..i, SynKind::Definition));
                expect_def = false;
            }
            continue;
        }

        // Any other byte (operator, punctuation, `#` directive, whitespace) stays
        // default ink. A non-identifier, non-whitespace token after a def keyword
        // means the name never materialized — drop the expectation.
        if !c.is_ascii_whitespace() {
            expect_def = false;
        }
        i += 1;
    }

    out
}

/// If `b[i..]` opens a (non-raw) string/char literal with the given `quote`,
/// optionally behind a `L` / `u` / `U` / `u8` encoding prefix, return the prefix
/// length (0 for a bare quote); else `None`.
fn enc_prefix_for(b: &[u8], i: usize, quote: u8) -> Option<usize> {
    let n = b.len();
    if i < n && b[i] == quote {
        return Some(0);
    }
    if b[i] == b'u' && i + 1 < n && b[i + 1] == b'8' && i + 2 < n && b[i + 2] == quote {
        return Some(2);
    }
    if matches!(b[i], b'L' | b'u' | b'U') && i + 1 < n && b[i + 1] == quote {
        return Some(1);
    }
    None
}

/// If a RAW string literal starts at `i` (`R"d(...)d"`, optionally behind a
/// `L`/`u`/`U`/`u8` prefix), return the byte index just past its close; else
/// `None`. The delimiter `d` is the (≤16, no spaces/parens) run between `"` and
/// `(`; the literal closes at the matching `)d"`.
fn raw_string(b: &[u8], i: usize) -> Option<usize> {
    let n = b.len();
    let mut j = i;
    // Optional encoding prefix: L / u / U / u8.
    if j < n && b[j] == b'u' && j + 1 < n && b[j + 1] == b'8' {
        j += 2;
    } else if j < n && matches!(b[j], b'L' | b'u' | b'U') {
        j += 1;
    }
    if j >= n || b[j] != b'R' {
        return None;
    }
    j += 1;
    if j >= n || b[j] != b'"' {
        return None;
    }
    j += 1; // past opening quote
    let delim_start = j;
    while j < n
        && b[j] != b'('
        && b[j] != b'"'
        && !b[j].is_ascii_whitespace()
        && j - delim_start < 16
    {
        j += 1;
    }
    if j >= n || b[j] != b'(' {
        return None;
    }
    let delim = &b[delim_start..j];
    j += 1; // past `(`
    // Scan for the closing `)delim"`.
    while j < n {
        if b[j] == b')' {
            let k = j + 1;
            let end = k + delim.len();
            if end < n && &b[k..end] == delim && b[end] == b'"' {
                return Some(end + 1);
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
    super::scan_quoted(b, q, b'"', false)
}

/// Scan a char literal starting at the opening quote `q`; returns the index just
/// past the closing `'` (or EOF if unterminated). Honors `\\` escapes; C++ allows
/// multi-char literals (`'ab'`), so we run to the next unescaped quote.
fn scan_char(b: &[u8], q: usize) -> usize {
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
/// it. Accepts `0x`/`0b` radixes, C++14 `'` digit separators, a fractional `.`,
/// decimal/hex (`e`/`p`) exponents with a sign, and a trailing type suffix
/// (`u`, `ll`, `f`, …).
fn scan_number(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i + 1;
    // Radix-prefixed integers: consume hex/alnum/underscore/separator freely.
    if b[i] == b'0' && j < n && matches!(b[j], b'x' | b'X' | b'b' | b'B') {
        j += 1;
        while j < n && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b'\'') {
            // A hex float still wants its `p`-exponent sign; handled below by the
            // generic loop, but radix bodies rarely carry one, so keep it simple.
            if matches!(b[j], b'p' | b'P')
                && j + 1 < n
                && matches!(b[j + 1], b'+' | b'-')
            {
                j += 2;
                continue;
            }
            j += 1;
        }
        return j;
    }
    while j < n {
        let c = b[j];
        if c.is_ascii_alphanumeric() || c == b'_' || c == b'\'' {
            j += 1;
        } else if c == b'.' {
            j += 1;
        } else if matches!(c, b'+' | b'-') && matches!(b[j - 1], b'e' | b'E' | b'p' | b'P') {
            // Signed exponent (`1e-9`, `0x1p+4`).
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
        let t = "int x = 1; // hi there\n";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Comment) == vec!["// hi there"], "{s:?}");
    }

    #[test]
    fn block_comment_not_nested() {
        // C++ block comments do NOT nest: the first `*/` closes it.
        let t = "/* a /* b */ c */ x";
        let s = spans(t);
        assert!(has(&s, 0, 12, SynKind::Comment), "{s:?}");
    }

    #[test]
    fn string_with_escaped_quote() {
        let t = r#"auto s = "a\"b";"#;
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec![r#""a\"b""#], "{s:?}");
    }

    #[test]
    fn prefixed_and_char_strings() {
        let t = r#"auto w = L"wide"; char c = u8'x';"#;
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        assert!(ss.contains(&r#"L"wide""#), "{ss:?}");
        assert!(ss.contains(&"u8'x'"), "{ss:?}");
    }

    #[test]
    fn raw_string_with_delimiter() {
        let t = r####"auto s = R"qq(he said "hi")qq";"####;
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec![r##"R"qq(he said "hi")qq""##], "{s:?}");
    }

    #[test]
    fn char_escape_literal() {
        let t = r"char n = '\n';";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec![r"'\n'"], "{s:?}");
    }

    #[test]
    fn numbers_separators_and_constants() {
        let t = "auto a = 42; auto b = 0xFF'u8; auto c = 3.14e-2; auto m = 1'000'000; bool ok = true; void* p = nullptr;";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        assert!(cs.contains(&"42"), "{cs:?}");
        assert!(cs.contains(&"0xFF'u8"), "{cs:?}");
        assert!(cs.contains(&"3.14e-2"), "{cs:?}");
        assert!(cs.contains(&"1'000'000"), "{cs:?}");
        assert!(cs.contains(&"true"), "{cs:?}");
        assert!(cs.contains(&"nullptr"), "{cs:?}");
    }

    #[test]
    fn definitions_after_introducers() {
        let t = "class Widget {};\nstruct Point {};\nenum E {};\nnamespace ns {}\nunion U {};";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"Widget"), "{ds:?}");
        assert!(ds.contains(&"Point"), "{ds:?}");
        assert!(ds.contains(&"E"), "{ds:?}");
        assert!(ds.contains(&"ns"), "{ds:?}");
        assert!(ds.contains(&"U"), "{ds:?}");
    }

    #[test]
    fn enum_class_names_the_type() {
        // `enum class Color` — the inner `class` is skipped; `Color` is the def.
        let t = "enum class Color { Red, Green };";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"Color"), "{ds:?}");
        assert!(!ds.contains(&"class"), "the `class` keyword must stay plain: {ds:?}");
    }

    #[test]
    fn keyword_itself_is_not_styled() {
        // `struct` keyword stays default ink; only the NAME is a Definition.
        let t = "struct Foo {};";
        let s = spans(t);
        assert!(!has(&s, 0, 6, SynKind::Definition), "the `struct` keyword must stay plain: {s:?}");
        assert!(has(&s, 7, 10, SynKind::Definition), "`Foo` is the definition: {s:?}");
    }

    #[test]
    fn plain_code_has_no_spans() {
        // No comment / literal / def-keyword -> nothing highlighted (Alabaster).
        let t = "int result = compute(a, b) + offset;";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn reference_snippet() {
        // A compact end-to-end snippet asserting all four roles at once.
        let t = "// sum\nint add(int a, int b) {\n    return a + b; // ok\n}\nclass Box { int n = 100; };\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["// sum", "// ok"], "{s:?}");
        assert!(at(t, &s, SynKind::Definition).contains(&"Box"), "{s:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"100"), "{s:?}");
    }
}
