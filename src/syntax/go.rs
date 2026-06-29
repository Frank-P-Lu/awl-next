//! Go syntax lexer — a minimal hand-written byte scanner mirroring the reference
//! lexer in [`crate::syntax::rust`]. It recognizes only what the four Alabaster
//! roles need and leaves everything else (keywords, operators, identifiers,
//! punctuation) as the default ink:
//!
//! - [`SynKind::Comment`]    — `// line` and `/* block */` comments (Go blocks do
//!   NOT nest).
//! - [`SynKind::Str`]        — interpreted `"..."` strings, raw `` `...` `` strings
//!   (multiline, no escapes), and `'r'` rune literals.
//! - [`SynKind::Constant`]   — numeric literals (incl. `0x`/`0o`/`0b`, hex floats,
//!   `_` separators, the `i` imaginary suffix) and `true` / `false` / `nil` /
//!   `iota`.
//! - [`SynKind::Definition`] — the identifier right after a `func` / `type` / `var`
//!   / `const` / `package` introducer (a `func` method receiver in parens is
//!   skipped so the METHOD name is the one marked).
//!
//! Span boundaries land on ASCII bytes (quotes, `/`, `` ` ``, digits, ASCII
//! identifiers), so multibyte UTF-8 inside a string/comment/rune rides inside the
//! span without ever splitting a char. Pure + single-pass. See `rust.rs` for the
//! template this mirrors and the tests below for the exact contract.

use super::SynKind;
use std::ops::Range;

/// Introducers after which the next identifier is the DEFINITION name.
const DEF_KEYWORDS: &[&str] = &["func", "type", "var", "const", "package"];
/// Identifiers that are CONSTANT literals (booleans + the `nil` value + `iota`).
const CONST_WORDS: &[&str] = &["true", "false", "nil", "iota"];

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
    // True while `expect_def` was raised by `func`, so a method receiver in
    // parens (`func (r *T) Name()`) can be skipped before the NAME.
    let mut def_func = false;

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

        // --- block comment (Go blocks do NOT nest) ---
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

        // --- raw string: `...` (no escapes, may span newlines) ---
        if c == b'`' {
            let start = i;
            i += 1;
            while i < n && b[i] != b'`' {
                i += 1;
            }
            if i < n {
                i += 1; // past closing backtick
            }
            out.push((start..i, SynKind::Str));
            expect_def = false;
            continue;
        }

        // --- interpreted string: "..." ---
        if c == b'"' {
            let end = scan_string(b, i);
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- rune literal: 'r' / '\n' / 'é' (Go has no lifetimes) ---
        if c == b'\'' {
            let end = scan_rune(b, i);
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
                // The name introduced by the preceding keyword.
                out.push((start..i, SynKind::Definition));
                expect_def = false;
            } else if CONST_WORDS.contains(&word) {
                out.push((start..i, SynKind::Constant));
            } else if DEF_KEYWORDS.contains(&word) {
                expect_def = true;
                def_func = word == "func";
            }
            continue;
        }

        // A `func` method receiver `(r *T)` precedes the method NAME — skip the
        // balanced parens but keep the expectation alive.
        if c == b'(' && expect_def && def_func {
            i = skip_parens(b, i);
            continue;
        }

        // Any other byte (operator, punctuation, whitespace) stays default ink.
        // Whitespace between a keyword and its name must NOT clear the expectation.
        if !c.is_ascii_whitespace() {
            expect_def = false;
        }
        i += 1;
    }

    out
}

/// Scan an interpreted double-quoted string starting at the opening quote `q`;
/// returns the index just past the closing quote (or the newline / EOF if
/// unterminated — a Go interpreted string cannot cross a raw newline). Honors `\\`
/// escapes so an escaped quote does not close the string.
fn scan_string(b: &[u8], q: usize) -> usize {
    let n = b.len();
    let mut i = q + 1;
    while i < n {
        match b[i] {
            b'\\' => i += 2,
            b'\n' => return i, // unterminated: stop at the newline
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    n
}

/// Scan a rune literal from the opening quote `q` to just past the closing quote
/// (or the newline / EOF if unterminated). Honors `\\` escapes; Go has no
/// lifetimes, so every `'` opens a rune.
fn scan_rune(b: &[u8], q: usize) -> usize {
    let n = b.len();
    let mut i = q + 1;
    while i < n {
        match b[i] {
            b'\\' => i += 2,
            b'\n' => return i,
            b'\'' => return i + 1,
            _ => i += 1,
        }
    }
    n
}

/// Skip a balanced `(...)` group starting at the open paren `i`; returns the index
/// just past the matching close paren (or EOF). Used to step over a method
/// receiver between `func` and the method NAME.
fn skip_parens(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i + 1;
    let mut depth = 1u32;
    while j < n && depth > 0 {
        match b[j] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        j += 1;
    }
    j
}

/// Scan a numeric literal beginning at the digit `i`; returns the index just past
/// it. Accepts `0x`/`0o`/`0b` radixes, `_` separators, a fractional `.`, an
/// exponent, and the trailing `i` imaginary suffix. A `.` that opens a method call
/// on an integer literal (`x.foo`) is NOT consumed.
fn scan_number(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i + 1;
    // Radix-prefixed integers: consume hex/alnum/underscore freely.
    if b[i] == b'0' && j < n && matches!(b[j], b'x' | b'X' | b'o' | b'O' | b'b' | b'B') {
        j += 1;
        while j < n && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b'.') {
            j += 1;
        }
        return j;
    }
    while j < n {
        let c = b[j];
        if c.is_ascii_alphanumeric() || c == b'_' {
            j += 1;
        } else if c == b'.' {
            // A fractional point, but not an attribute access on an int literal.
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
    use crate::syntax::testutil::{at, has};

    #[test]
    fn line_comment() {
        let t = "x := 1 // hi there\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["// hi there"], "{s:?}");
    }

    #[test]
    fn block_comment_not_nested() {
        // Go blocks do NOT nest: the FIRST `*/` closes it.
        let t = "/* a /* b */ c";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["/* a /* b */"], "{s:?}");
    }

    #[test]
    fn interpreted_string_with_escaped_quote() {
        let t = r#"s := "a\"b""#;
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec![r#""a\"b""#], "{s:?}");
    }

    #[test]
    fn raw_string_multiline() {
        let t = "s := `line one\nline two`\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec!["`line one\nline two`"], "{s:?}");
    }

    #[test]
    fn rune_literals() {
        let t = "a := 'x'; b := '\\n'; c := '世'\n";
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        assert!(ss.contains(&"'x'"), "{ss:?}");
        assert!(ss.contains(&"'\\n'"), "{ss:?}");
        assert!(ss.contains(&"'世'"), "{ss:?}");
    }

    #[test]
    fn numbers_and_constants() {
        let t = "a := 42; b := 0xFF_u; c := 3.14; d := 1_000; e := 2i; ok := true; z := nil; i := iota\n";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["42", "0xFF_u", "3.14", "1_000", "2i", "true", "nil", "iota"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn int_method_call_not_eaten() {
        // `x.foo` — the `.` is an attribute access, not a fractional point.
        let t = "n := 3\nv := n.foo\n";
        let s = spans(t);
        // Only the bare `3` is a constant; nothing swallows past it.
        assert!(at(t, &s, SynKind::Constant).contains(&"3"), "{s:?}");
    }

    #[test]
    fn definitions_after_keywords() {
        let t = "package main\nfunc add(a int) int { return a }\ntype Widget struct{}\nvar count int\nconst Max = 100\n";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        for want in ["main", "add", "Widget", "count", "Max"] {
            assert!(ds.contains(&want), "missing {want}: {ds:?}");
        }
    }

    #[test]
    fn method_receiver_is_skipped() {
        // The method NAME `Area`, not the receiver `r`, is the Definition.
        let t = "func (r *Rect) Area() int { return 0 }\n";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"Area"), "{ds:?}");
        assert!(!ds.contains(&"r"), "receiver wrongly marked: {ds:?}");
    }

    #[test]
    fn keyword_itself_is_not_styled() {
        // `func` keyword stays default ink; only the NAME is a Definition.
        let t = "func main() {}";
        let s = spans(t);
        assert!(!has(&s, 0, 4, SynKind::Definition), "`func` must stay plain: {s:?}");
        assert!(has(&s, 5, 9, SynKind::Definition), "`main` is the definition: {s:?}");
    }

    #[test]
    fn plain_code_has_no_spans() {
        // No comment / literal / def-keyword -> nothing highlighted (Alabaster).
        let t = "result := compute(a, b) + offset";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn reference_snippet() {
        // A compact end-to-end snippet asserting all four roles at once.
        let t = "// sum\nfunc add(a int, b int) int {\n\ttotal := a + b // ok\n\treturn total\n}\nconst Max = 100\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["// sum", "// ok"], "{s:?}");
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"add") && ds.contains(&"Max"), "{ds:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"100"), "{s:?}");
    }
}
