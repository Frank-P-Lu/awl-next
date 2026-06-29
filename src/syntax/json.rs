//! JSON syntax lexer — Alabaster, four roles only (see `rust.rs` / `python.rs`
//! for the reference templates this mirrors). A minimal hand-written byte scanner
//! emitting only the four roles; everything else (the structural punctuation
//! `{ } [ ] : ,`) stays the default ink:
//!
//! - [`SynKind::Comment`]    — `// line` and `/* block */` comments. Strict RFC
//!   8259 JSON has no comments, but real `.json` files in tooling are almost
//!   always JSONC (`tsconfig.json`, VS Code `settings.json`, …), so we recede
//!   both comment forms to the dim ink rather than render them as garbage.
//! - [`SynKind::Str`]        — `"..."` string literals (the only JSON string
//!   form), honoring `\"` / `\\` / `\uXXXX` escapes.
//! - [`SynKind::Constant`]   — numeric literals (incl. a leading `-`, fraction,
//!   and exponent) and the `true` / `false` / `null` keywords.
//! - [`SynKind::Definition`] — the OBJECT KEY: a string immediately followed by a
//!   `:`. JSON has no `fn`/`def` introducer, so the key — the NAME a value is
//!   bound to — is the natural best-effort analog of "the name being defined".
//!   (Last-wins: a key is the only string that is a `Definition`, not a `Str`.)
//!
//! Span boundaries land on ASCII bytes (quotes, digits, `/`); multibyte UTF-8
//! inside a string/comment rides inside the span. Pure + single-pass.

use super::SynKind;
use std::ops::Range;

pub fn spans(text: &str) -> Vec<(Range<usize>, SynKind)> {
    let b = text.as_bytes();
    let n = b.len();
    let mut out: Vec<(Range<usize>, SynKind)> = Vec::new();
    let mut i = 0usize;

    while i < n {
        let c = b[i];

        // --- comment (JSONC: `//` line, `/* */` block) ---
        if c == b'/' && i + 1 < n {
            if b[i + 1] == b'/' {
                let start = i;
                i += 2;
                while i < n && b[i] != b'\n' {
                    i += 1;
                }
                out.push((start..i, SynKind::Comment));
                continue;
            }
            if b[i + 1] == b'*' {
                let start = i;
                i += 2;
                while i < n && !(b[i] == b'*' && i + 1 < n && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(n); // consume the closing `*/` (or clamp at EOF)
                out.push((start..i, SynKind::Comment));
                continue;
            }
        }

        // --- string (key -> Definition, otherwise Str) ---
        if c == b'"' {
            let start = i;
            i = scan_string(b, i);
            let kind = if is_key(b, i) {
                SynKind::Definition
            } else {
                SynKind::Str
            };
            out.push((start..i, kind));
            continue;
        }

        // --- number (optional leading `-`) ---
        if c.is_ascii_digit() || (c == b'-' && i + 1 < n && b[i + 1].is_ascii_digit()) {
            let start = i;
            i = scan_number(b, i);
            out.push((start..i, SynKind::Constant));
            continue;
        }

        // --- literal keywords: true / false / null ---
        if c.is_ascii_lowercase() {
            let start = i;
            while i < n && b[i].is_ascii_lowercase() {
                i += 1;
            }
            let word = &text[start..i];
            if matches!(word, "true" | "false" | "null") {
                out.push((start..i, SynKind::Constant));
            }
            continue;
        }

        i += 1;
    }

    out
}

/// Scan a `"`-delimited JSON string from its opening quote `q` to just past the
/// closing quote (or EOF / end-of-line for an unterminated one). Honors `\\`
/// escapes (`\"`, `\\`, `\uXXXX`, …) so an escaped quote does not close early.
fn scan_string(b: &[u8], q: usize) -> usize {
    let n = b.len();
    let mut i = q + 1;
    while i < n {
        match b[i] {
            b'\\' => i += 2,
            b'\n' => return i, // unterminated: JSON strings do not cross a newline
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    n
}

/// Is the string that just ended at `end` an object KEY? — i.e. is the next
/// non-whitespace byte a `:`. Used to tag keys as `Definition` rather than `Str`.
fn is_key(b: &[u8], end: usize) -> bool {
    let n = b.len();
    let mut j = end;
    while j < n && b[j].is_ascii_whitespace() {
        j += 1;
    }
    j < n && b[j] == b':'
}

/// Scan a JSON numeric literal beginning at `i` (a digit, or a `-` before one);
/// returns the index just past it. Accepts an optional leading `-`, an integer
/// part, an optional `.`-fraction, and an optional `e`/`E` exponent with sign.
fn scan_number(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i;
    if j < n && b[j] == b'-' {
        j += 1;
    }
    while j < n && b[j].is_ascii_digit() {
        j += 1;
    }
    if j < n && b[j] == b'.' {
        j += 1;
        while j < n && b[j].is_ascii_digit() {
            j += 1;
        }
    }
    if j < n && (b[j] == b'e' || b[j] == b'E') {
        j += 1;
        if j < n && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        while j < n && b[j].is_ascii_digit() {
            j += 1;
        }
    }
    j
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::testutil::{at};

    #[test]
    fn line_and_block_comments_recede() {
        let t = "{\n  // a line\n  /* a block */\n}\n";
        let s = spans(t);
        assert_eq!(
            at(t, &s, SynKind::Comment),
            vec!["// a line", "/* a block */"],
            "{s:?}"
        );
    }

    #[test]
    fn block_comment_spans_multiple_lines() {
        let t = "/* one\n   two */\n42\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["/* one\n   two */"], "{s:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"42"), "{s:?}");
    }

    #[test]
    fn value_string_is_str() {
        let t = "{ \"k\": \"hello\" }";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec!["\"hello\""], "{s:?}");
    }

    #[test]
    fn string_escapes_do_not_close_early() {
        let t = "[\"a\\\"b\", \"c\\\\\"]";
        let s = spans(t);
        // Two value strings; the escaped `\"` and `\\` stay inside the span.
        assert_eq!(at(t, &s, SynKind::Str), vec!["\"a\\\"b\"", "\"c\\\\\""], "{s:?}");
    }

    #[test]
    fn object_key_is_definition_not_str() {
        let t = "{\n  \"name\" : \"awl\"\n}";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Definition), vec!["\"name\""], "{s:?}");
        assert_eq!(at(t, &s, SynKind::Str), vec!["\"awl\""], "{s:?}");
    }

    #[test]
    fn numbers_and_literal_constants() {
        let t = "{ \"a\": 42, \"b\": -3.14, \"c\": 1e10, \"d\": true, \"e\": false, \"f\": null }";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["42", "-3.14", "1e10", "true", "false", "null"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn structural_punctuation_rides_default_ink() {
        // `{ } [ ] : ,` and a bare identifier-ish run get NO span.
        let t = "{}[],:";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn reference_snippet() {
        let t = "{\n  // config\n  \"port\": 8080,\n  \"debug\": false,\n  \"name\": \"awl\"\n}\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["// config"], "{s:?}");
        let ds = at(t, &s, SynKind::Definition);
        for want in ["\"port\"", "\"debug\"", "\"name\""] {
            assert!(ds.contains(&want), "missing key {want}: {ds:?}");
        }
        assert!(at(t, &s, SynKind::Constant).contains(&"8080"), "{s:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"false"), "{s:?}");
        assert!(at(t, &s, SynKind::Str).contains(&"\"awl\""), "{s:?}");
    }
}
