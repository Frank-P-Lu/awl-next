//! Python syntax lexer — the second REFERENCE implementation (alongside
//! `rust.rs`). A minimal hand-written byte scanner emitting only the four
//! Alabaster roles; everything else stays the default ink:
//!
//! - [`SynKind::Comment`]    — `# line` comments.
//! - [`SynKind::Str`]        — `'...'` / `"..."` and triple-quoted `'''...'''` /
//!   `"""..."""`, including the `r`/`b`/`f`/`u` string prefixes (and combos).
//! - [`SynKind::Constant`]   — numeric literals and `True` / `False` / `None`.
//! - [`SynKind::Definition`] — the identifier right after a `def` or `class`.
//!
//! Span boundaries land on ASCII bytes; multibyte UTF-8 inside a string/comment
//! rides inside the span. Pure + single-pass. See `rust.rs` for the template this
//! mirrors and the tests below for the exact contract.

use super::SynKind;
use std::ops::Range;

/// Introducers after which the next identifier is the DEFINITION name.
const DEF_KEYWORDS: &[&str] = &["def", "class"];
/// Identifiers that are CONSTANT literals (booleans + the `None` nil value).
const CONST_WORDS: &[&str] = &["True", "False", "None"];

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic()
}
fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
}
/// A valid Python string-prefix letter (`r`/`b`/`f`/`u`, any case).
fn is_prefix(c: u8) -> bool {
    matches!(c, b'r' | b'b' | b'f' | b'u' | b'R' | b'B' | b'F' | b'U')
}

pub fn spans(text: &str) -> Vec<(Range<usize>, SynKind)> {
    let b = text.as_bytes();
    let n = b.len();
    let mut out: Vec<(Range<usize>, SynKind)> = Vec::new();
    let mut i = 0usize;
    let mut expect_def = false;

    while i < n {
        let c = b[i];

        // --- comment ---
        if c == b'#' {
            let start = i;
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            out.push((start..i, SynKind::Comment));
            continue;
        }

        // --- string (with optional prefix, triple or single) ---
        if let Some((quote, triple)) = string_start(b, i) {
            let end = if triple {
                scan_triple(b, quote)
            } else {
                scan_string(b, quote)
            };
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- number ---
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

        if !c.is_ascii_whitespace() {
            expect_def = false;
        }
        i += 1;
    }

    out
}

/// If a string literal starts at `i` — an optional `r`/`b`/`f`/`u` prefix (up to
/// two letters) immediately followed by a quote — return `(quote_index, is_triple)`;
/// else `None`. A bare quote at `i` (no prefix) also matches.
fn string_start(b: &[u8], i: usize) -> Option<(usize, bool)> {
    let n = b.len();
    let mut j = i;
    let mut k = 0;
    while k < 2 && j < n && is_prefix(b[j]) {
        j += 1;
        k += 1;
    }
    if j < n && (b[j] == b'"' || b[j] == b'\'') {
        let q = b[j];
        let triple = j + 2 < n && b[j + 1] == q && b[j + 2] == q;
        Some((j, triple))
    } else {
        None
    }
}

/// Scan a single-quoted string from the opening quote `q` to just past its close
/// (or EOF / end-of-line — a single-quoted Python string does not cross a newline).
fn scan_string(b: &[u8], q: usize) -> usize {
    let n = b.len();
    let quote = b[q];
    let mut i = q + 1;
    while i < n {
        match b[i] {
            b'\\' => i += 2,
            b'\n' => return i, // unterminated single-line string: stop at the newline
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    n
}

/// Scan a triple-quoted string from the opening quote `q` (the first of three) to
/// just past the closing triple (or EOF). Honors `\\` escapes.
fn scan_triple(b: &[u8], q: usize) -> usize {
    let n = b.len();
    let quote = b[q];
    let mut i = q + 3;
    while i < n {
        if b[i] == b'\\' {
            i += 2;
        } else if b[i] == quote && i + 2 < n && b[i + 1] == quote && b[i + 2] == quote {
            return i + 3;
        } else if b[i] == quote && i + 2 == n && i + 1 < n && b[i + 1] == quote {
            // Closing triple flush at EOF.
            return n;
        } else {
            i += 1;
        }
    }
    n
}

/// Scan a numeric literal beginning at the digit `i`; returns the index just past
/// it. Accepts `0x`/`0o`/`0b`, `_` separators, a fractional `.`, exponent, and a
/// trailing `j` imaginary suffix.
fn scan_number(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i + 1;
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
            // Fractional point, but not a `..` (Python has no range op, but be safe)
            // and not an attribute access on an int literal.
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
    fn comment() {
        let t = "x = 1  # set x\n";
        assert_eq!(at(t, &spans(t), SynKind::Comment), vec!["# set x"]);
    }

    #[test]
    fn single_and_double_strings() {
        let t = "a = 'hi'\nb = \"yo\"\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec!["'hi'", "\"yo\""], "{s:?}");
    }

    #[test]
    fn triple_string_multiline() {
        let t = "doc = \"\"\"line one\nline two\"\"\"\n";
        let s = spans(t);
        assert_eq!(
            at(t, &s, SynKind::Str),
            vec!["\"\"\"line one\nline two\"\"\""],
            "{s:?}"
        );
    }

    #[test]
    fn prefixed_strings() {
        let t = "p = r'\\d+'\nq = f\"{x}\"\nr = rb'bytes'\n";
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        assert!(ss.contains(&"r'\\d+'"), "{ss:?}");
        assert!(ss.contains(&"f\"{x}\""), "{ss:?}");
        assert!(ss.contains(&"rb'bytes'"), "{ss:?}");
    }

    #[test]
    fn f_prefix_does_not_swallow_function_call() {
        // `format(` must NOT be read as an f-string prefix.
        let t = "format(x)";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn numbers_and_constants() {
        let t = "a = 42\nb = 0xFF\nc = 3.14\nd = 1_000\nok = True\nz = None\n";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["42", "0xFF", "3.14", "1_000", "True", "None"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn def_and_class_names() {
        let t = "def frobnicate(x):\n    pass\nclass Widget:\n    pass\n";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"frobnicate"), "{ds:?}");
        assert!(ds.contains(&"Widget"), "{ds:?}");
        // The `def`/`class` keywords themselves stay plain.
        assert!(!has(&s, 0, 3, SynKind::Definition), "{s:?}");
    }

    #[test]
    fn plain_code_has_no_spans() {
        let t = "result = compute(a, b) + offset";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn reference_snippet() {
        let t = "# add two\ndef add(a, b):\n    total = a + b  # sum\n    return total\nMAX = 100\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["# add two", "# sum"], "{s:?}");
        assert!(at(t, &s, SynKind::Definition).contains(&"add"), "{s:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"100"), "{s:?}");
    }
}
