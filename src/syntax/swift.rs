//! Swift syntax lexer — a minimal hand-written byte scanner following the
//! `rust.rs` / `python.rs` template, emitting only the four Alabaster roles and
//! leaving everything else (keywords, operators, identifiers, punctuation) on the
//! default ink:
//!
//! - [`SynKind::Comment`]    — `// line` and `/* block */` (Swift nests them, and
//!   doc comments `///` / `/** */` ride here too).
//! - [`SynKind::Str`]        — `"strings"`, multiline `"""..."""`, and raw strings
//!   with any number of `#` delimiters (`#"..."#`, `##"..."##`, raw multiline
//!   `#"""..."""#`). An interpolated string (`"\(x)"`) is one `Str` span.
//! - [`SynKind::Constant`]   — numeric literals (`0x`/`0o`/`0b`, floats, exponents,
//!   `_` separators, hex floats) and the `true` / `false` / `nil` literals.
//! - [`SynKind::Definition`] — the identifier right after a `func` / `class` /
//!   `struct` / `enum` / `protocol` / `extension` / `actor` / `typealias` /
//!   `associatedtype` / `precedencegroup` / `let` / `var` introducer.
//!
//! Span boundaries always land on ASCII bytes (quotes, `/`, `#`, digits, ASCII
//! identifiers), so multibyte UTF-8 inside a string/comment rides inside the span
//! without ever splitting a char. Pure + single-pass; see the tests at the bottom
//! for the exact contract on a sample snippet.

use super::SynKind;
use std::ops::Range;

/// Introducers after which the next identifier is the DEFINITION name. Swift's
/// `let` / `var` bindings are included (best-effort: the name after `let x = …`).
const DEF_KEYWORDS: &[&str] = &[
    "func",
    "class",
    "struct",
    "enum",
    "protocol",
    "extension",
    "actor",
    "typealias",
    "associatedtype",
    "precedencegroup",
    "let",
    "var",
];

/// Identifiers that are CONSTANT literals (booleans + the `nil` nil-style value).
const CONST_WORDS: &[&str] = &["true", "false", "nil"];

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

        // --- line comment (incl. `///` doc comments) ---
        if c == b'/' && i + 1 < n && b[i + 1] == b'/' {
            let end = super::scan_line_comment(b, i);
            out.push((i..end, SynKind::Comment));
            i = end;
            continue;
        }

        // --- block comment (Swift nests them; `/** */` doc comments included) ---
        if c == b'/' && i + 1 < n && b[i + 1] == b'*' {
            let end = super::scan_block_comment(b, i, true);
            out.push((i..end, SynKind::Comment));
            i = end;
            continue;
        }

        // --- string (plain / multiline / raw, with optional `#` delimiters) ---
        if c == b'"' || c == b'#' {
            if let Some(end) = string_at(b, i) {
                out.push((i..end, SynKind::Str));
                i = end;
                expect_def = false;
                continue;
            }
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
            if let Some(kind) = super::ident_role(word, DEF_KEYWORDS, CONST_WORDS, &mut expect_def) {
                out.push((start..i, kind));
            }
            continue;
        }

        // Any other byte (operator, punctuation, whitespace) stays default ink. A
        // non-identifier, non-whitespace token after a def keyword means the name
        // never materialized (e.g. `func +(…)`) — drop the expectation.
        if !c.is_ascii_whitespace() {
            expect_def = false;
        }
        i += 1;
    }

    out
}

/// If a string literal starts at `i` — an optional run of `#` delimiters followed
/// by a `"` (or `"""` for multiline) — return the byte index just past its close
/// (or EOF if unterminated); else `None`. A bare `#` not followed by a quote (a
/// `#if` / `#selector` directive) yields `None` so the `#` rides default ink.
fn string_at(b: &[u8], i: usize) -> Option<usize> {
    let n = b.len();
    let mut j = i;
    let mut hashes = 0usize;
    while j < n && b[j] == b'#' {
        hashes += 1;
        j += 1;
    }
    if j >= n || b[j] != b'"' {
        return None;
    }
    let triple = j + 2 < n && b[j + 1] == b'"' && b[j + 2] == b'"';
    // In a RAW string (`hashes > 0`) backslash does NOT escape; the only close is
    // the quote(s) followed by the matching `#` run.
    let raw = hashes > 0;
    let mut k = if triple { j + 3 } else { j + 1 };
    while k < n {
        let ch = b[k];
        if !raw && ch == b'\\' {
            k += 2; // escaped char (incl. `\(` interpolation, `\"`)
            continue;
        }
        if ch == b'"' {
            if triple {
                if k + 2 < n && b[k + 1] == b'"' && b[k + 2] == b'"' {
                    if let Some(end) = hashes_after(b, k + 3, hashes) {
                        return Some(end);
                    }
                }
            } else if let Some(end) = hashes_after(b, k + 1, hashes) {
                return Some(end);
            }
        }
        // A plain single-line string does not cross a newline.
        if !triple && ch == b'\n' {
            return Some(k);
        }
        k += 1;
    }
    Some(n) // unterminated: run to EOF
}

/// If exactly `hashes` `#` bytes appear at `p`, return the index just past them;
/// else `None`. (`hashes == 0` trivially matches at `p`.)
fn hashes_after(b: &[u8], p: usize, hashes: usize) -> Option<usize> {
    let n = b.len();
    let mut h = 0;
    let mut q = p;
    while h < hashes && q < n && b[q] == b'#' {
        h += 1;
        q += 1;
    }
    if h == hashes {
        Some(q)
    } else {
        None
    }
}

/// Scan a numeric literal beginning at the digit `i`; returns the index just past
/// it. Accepts `0x`/`0o`/`0b` radixes, `_` separators, a fractional `.`, decimal
/// (`e`) and hex-float (`p`) exponents with a sign, and Swift's `0x1.8p2` form. A
/// `...` / `..<` range operator after the integer is NOT consumed.
fn scan_number(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i + 1;
    // Radix-prefixed integers (and hex floats): consume hex/alnum/underscore, a
    // fractional `.` before a hex digit, and a `p`-exponent sign.
    if b[i] == b'0' && j < n && matches!(b[j], b'x' | b'X' | b'o' | b'O' | b'b' | b'B') {
        j += 1;
        while j < n {
            let c = b[j];
            if c.is_ascii_alphanumeric() || c == b'_' {
                j += 1;
            } else if c == b'.'
                && j + 1 < n
                && b[j + 1] != b'.'
                && b[j + 1].is_ascii_hexdigit()
            {
                j += 1;
            } else if (c == b'+' || c == b'-') && j > 0 && matches!(b[j - 1], b'p' | b'P') {
                j += 1;
            } else {
                break;
            }
        }
        return j;
    }
    while j < n {
        let c = b[j];
        if c.is_ascii_alphanumeric() || c == b'_' {
            j += 1;
        } else if c == b'.' {
            // A fractional point — but not the `...`/`..<` range op, and not a
            // member access on an integer (`.` before a non-digit ident start).
            if j + 1 < n && (b[j + 1] == b'.' || is_ident_start(b[j + 1])) {
                break;
            }
            j += 1;
        } else if (c == b'+' || c == b'-') && j > 0 && matches!(b[j - 1], b'e' | b'E') {
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
        let t = "let x = 1 // hi there\n";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Comment) == vec!["// hi there"], "{s:?}");
    }

    #[test]
    fn block_comment_nested() {
        let t = "/* a /* b */ c */ x";
        let s = spans(t);
        // The whole nested block is ONE comment span (Swift nests).
        assert!(has(&s, 0, 17, SynKind::Comment), "{s:?}");
    }

    #[test]
    fn string_with_escaped_quote() {
        let t = r#"let s = "a\"b""#;
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec![r#""a\"b""#], "{s:?}");
    }

    #[test]
    fn interpolation_is_one_span() {
        let t = r#"let g = "hi \(name)!""#;
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec![r#""hi \(name)!""#], "{s:?}");
    }

    #[test]
    fn multiline_string() {
        let t = "let d = \"\"\"\nline one\nline two\n\"\"\"\n";
        let s = spans(t);
        assert_eq!(
            at(t, &s, SynKind::Str),
            vec!["\"\"\"\nline one\nline two\n\"\"\""],
            "{s:?}"
        );
    }

    #[test]
    fn raw_string_with_hashes() {
        let t = r##"let s = #"he said "hi""#"##;
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec![r##"#"he said "hi""#"##], "{s:?}");
    }

    #[test]
    fn raw_multiline_string() {
        let t = "let r = #\"\"\"\na \"b\" c\n\"\"\"#\n";
        let s = spans(t);
        assert_eq!(
            at(t, &s, SynKind::Str),
            vec!["#\"\"\"\na \"b\" c\n\"\"\"#"],
            "{s:?}"
        );
    }

    #[test]
    fn hash_directive_is_not_a_string() {
        // `#if`/`#selector` must NOT be read as a (raw) string opener.
        let t = "#if DEBUG\nlet x = 1\n#endif\n";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Str).is_empty(), "{s:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"1"), "{s:?}");
    }

    #[test]
    fn numbers_and_constants() {
        let t = "let a = 42; let b = 0xFF; let c = 3.14; let d = 1_000; let e = 1.5e-3; let ok = true; let z: Int? = nil;";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["42", "0xFF", "3.14", "1_000", "1.5e-3", "true", "nil"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn range_op_not_eaten_by_number() {
        let t = "for i in 0..<5 {}";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        assert!(cs.contains(&"0") && cs.contains(&"5"), "range split: {cs:?}");
    }

    #[test]
    fn definitions_after_introducers() {
        let t = "func frobnicate() {}\nclass Widget {}\nstruct Point {}\nenum E {}\nprotocol P {}\ntypealias Alias = Int\nlet count = 3\nvar total = 0\n";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        for want in [
            "frobnicate", "Widget", "Point", "E", "P", "Alias", "count", "total",
        ] {
            assert!(ds.contains(&want), "missing {want}: {ds:?}");
        }
    }

    #[test]
    fn keyword_itself_is_not_styled() {
        // `func` keyword stays default ink; only the NAME is a Definition.
        let t = "func main() {}";
        let s = spans(t);
        assert!(!has(&s, 0, 4, SynKind::Definition), "the `func` keyword must stay plain: {s:?}");
        assert!(has(&s, 5, 9, SynKind::Definition), "`main` is the definition: {s:?}");
    }

    #[test]
    fn plain_code_has_no_spans() {
        // No comment / literal / def-keyword -> nothing highlighted (Alabaster).
        let t = "result = compute(a, b) + offset";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn reference_snippet() {
        // A compact end-to-end snippet asserting all four roles at once.
        let t = "// sum\nfunc add(_ a: Int, _ b: Int) -> Int {\n    let total = a + b // ok\n    return total\n}\nlet MAX = 100\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["// sum", "// ok"], "{s:?}");
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"add") && ds.contains(&"total") && ds.contains(&"MAX"), "{ds:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"100"), "{s:?}");
    }
}
