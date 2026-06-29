//! C# syntax lexer — a minimal hand-written byte scanner over the raw bytes,
//! mirroring the reference lexers in [`crate::syntax::rust`] and
//! [`crate::syntax::python`]. It recognizes only what the four Alabaster roles
//! need and leaves everything else (keywords, operators, identifiers,
//! punctuation) as the default ink:
//!
//! - [`SynKind::Comment`]    — `// line` (incl. `///` XML doc) and `/* block */`
//!   comments. C# block comments do NOT nest, so the first `*/` closes.
//! - [`SynKind::Str`]        — `"strings"` (with `\` escapes), `@"verbatim"`
//!   (where `""` is the escaped quote and `\` is literal), `$"interpolated"` and
//!   the `$@`/`@$` combo, `"""raw"""` literals, and `'c'` char literals. An
//!   interpolated string is treated as ONE `Str` span (we do not recurse into
//!   the `{ … }` holes).
//! - [`SynKind::Constant`]   — numeric literals (incl. `0x`/`0b`, `_` separators,
//!   `f`/`d`/`m`/`u`/`l` suffixes) and the `true` / `false` / `null` literals.
//! - [`SynKind::Definition`] — the identifier right after a `class` / `struct` /
//!   `interface` / `enum` / `record` / `namespace` introducer. (`delegate` is
//!   deliberately omitted: its NAME follows a return type, not the keyword.)
//!
//! Span boundaries always land on ASCII bytes (quotes, `/`, digits, ASCII
//! identifiers), so multibyte UTF-8 inside a string/comment rides inside the span
//! without ever splitting a char. Pure + single-pass. See the tests at the bottom
//! for the exact contract on a sample snippet.

use super::SynKind;
use std::ops::Range;

/// The keyword introducers after which the next identifier is the DEFINITION name.
const DEF_KEYWORDS: &[&str] = &[
    "class",
    "struct",
    "interface",
    "enum",
    "record",
    "namespace",
];

/// Identifiers that are CONSTANT literals (booleans + the `null` nil-style value).
const CONST_WORDS: &[&str] = &["true", "false", "null"];

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

        // --- line comment (covers `//` and the `///` XML-doc form) ---
        if c == b'/' && i + 1 < n && b[i + 1] == b'/' {
            let start = i;
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            out.push((start..i, SynKind::Comment));
            continue;
        }

        // --- block comment (C# does NOT nest them: first `*/` closes) ---
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

        // --- string (optional `@`/`$` prefix, verbatim / interpolated / raw) ---
        if let Some((q, verbatim)) = string_start(b, i) {
            let end = if is_raw(b, q) {
                scan_raw(b, q)
            } else if verbatim {
                scan_verbatim(b, q)
            } else {
                scan_string(b, q)
            };
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- char literal (C# has no lifetimes, so `'…'` is always a char) ---
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
        // A non-identifier, non-whitespace token after a def keyword means the
        // name never materialized — drop the expectation.
        if !c.is_ascii_whitespace() {
            expect_def = false;
        }
        i += 1;
    }

    out
}

/// If a string literal starts at `i` — an optional `@`/`$` prefix (in either
/// order, e.g. `$@"…"`) immediately followed by a quote — return `(quote_index,
/// is_verbatim)`; else `None`. A bare quote at `i` (no prefix) also matches with
/// `is_verbatim = false`. The `@` flag drives `""`-style quote escaping; `$`
/// (interpolation) does not change how we scan to the closing quote.
fn string_start(b: &[u8], i: usize) -> Option<(usize, bool)> {
    let n = b.len();
    let mut j = i;
    let mut verbatim = false;
    while j < n && (b[j] == b'@' || b[j] == b'$') {
        if b[j] == b'@' {
            verbatim = true;
        }
        j += 1;
    }
    if j < n && b[j] == b'"' {
        Some((j, verbatim))
    } else {
        None
    }
}

/// Whether the quote run at `q` opens a RAW string literal (`"""…"""`, three or
/// more quotes).
fn is_raw(b: &[u8], q: usize) -> bool {
    let n = b.len();
    let mut k = q;
    let mut count = 0;
    while k < n && b[k] == b'"' && count < 3 {
        count += 1;
        k += 1;
    }
    count >= 3
}

/// Scan a raw string literal from the opening quote run at `q` to just past its
/// closing run (or EOF). The literal closes on the first run of at least as many
/// quotes as the opening run.
fn scan_raw(b: &[u8], q: usize) -> usize {
    let n = b.len();
    let mut open = 0usize;
    let mut k = q;
    while k < n && b[k] == b'"' {
        open += 1;
        k += 1;
    }
    let mut i = k;
    while i < n {
        if b[i] == b'"' {
            let mut run = 0usize;
            let mut m = i;
            while m < n && b[m] == b'"' {
                run += 1;
                m += 1;
            }
            if run >= open {
                // Consume the whole trailing quote run as the close (a run longer
                // than the opener still ends the literal — don't leave a stray
                // quote that would start a new string).
                return m;
            }
            i = m;
        } else {
            i += 1;
        }
    }
    n
}

/// Scan a verbatim string (`@"…"`) from the opening quote `q` to just past its
/// close (or EOF). There are no `\` escapes; a doubled `""` is an escaped quote,
/// and the string may span newlines.
fn scan_verbatim(b: &[u8], q: usize) -> usize {
    let n = b.len();
    let mut i = q + 1;
    while i < n {
        if b[i] == b'"' {
            if i + 1 < n && b[i + 1] == b'"' {
                i += 2; // escaped quote
            } else {
                return i + 1;
            }
        } else {
            i += 1;
        }
    }
    n
}

/// Scan a normal double-quoted string from the opening quote `q` to just past its
/// close (or EOF / end-of-line — a non-verbatim C# string does not cross a
/// newline). Honors `\` escapes so an escaped quote does not close the string.
fn scan_string(b: &[u8], q: usize) -> usize {
    let n = b.len();
    let mut i = q + 1;
    while i < n {
        match b[i] {
            b'\\' => i += 2,
            b'\n' => return i, // unterminated single-line string: stop at newline
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    n
}

/// Scan a char literal from the opening quote `i` to just past its close (or
/// EOF / end-of-line). Honors `\` escapes (`'\n'`, `'A'`, `'\''`).
fn scan_char(b: &[u8], i: usize) -> usize {
    let n = b.len();
    debug_assert_eq!(b[i], b'\'');
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
/// and a trailing type suffix (`f`/`d`/`m`/`u`/`l`, any case). A `.` that is a
/// member access on an integer (`.` then an ident start) is NOT consumed.
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
            // A fractional point — but not a member access on an int literal
            // (`5.ToString()`) and not a `..` range/slice.
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
    fn line_and_doc_comment() {
        let t = "int x = 1; // hi\n/// <summary>doc</summary>\n";
        let s = spans(t);
        assert_eq!(
            at(t, &s, SynKind::Comment),
            vec!["// hi", "/// <summary>doc</summary>"],
            "{s:?}"
        );
    }

    #[test]
    fn block_comment_not_nested() {
        // C# block comments do NOT nest: the FIRST `*/` closes.
        let t = "/* a /* b */ c */ x";
        let s = spans(t);
        // The first `*/` (covering "/* a /* b */") closes the comment.
        assert!(has(&s, 0, 12, SynKind::Comment), "{s:?}");
        assert_eq!(&t[0..12], "/* a /* b */");
    }

    #[test]
    fn string_with_escaped_quote() {
        let t = r#"var s = "a\"b";"#;
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec![r#""a\"b""#], "{s:?}");
    }

    #[test]
    fn verbatim_string() {
        // `\` is literal in a verbatim string; `""` is the escaped quote.
        let t = r#"var p = @"C:\tmp\n"; var q = @"a""b";"#;
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        assert!(ss.contains(&r#"@"C:\tmp\n""#), "{ss:?}");
        assert!(ss.contains(&r#"@"a""b""#), "{ss:?}");
    }

    #[test]
    fn interpolated_string() {
        let t = r#"var g = $"hi {name}!"; var v = $@"x{y}";"#;
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        assert!(ss.contains(&r#"$"hi {name}!""#), "{ss:?}");
        assert!(ss.contains(&r#"$@"x{y}""#), "{ss:?}");
    }

    #[test]
    fn raw_string() {
        let t = "var r = \"\"\"he said \"hi\"\"\"\";";
        let s = spans(t);
        // The whole triple-quoted raw literal is ONE Str span.
        assert_eq!(at(t, &s, SynKind::Str), vec!["\"\"\"he said \"hi\"\"\"\""], "{s:?}");
    }

    #[test]
    fn char_literal_and_escape() {
        let t = r"char a = 'x'; char b = '\n'; char c = '\'';";
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        assert!(ss.contains(&"'x'"), "{ss:?}");
        assert!(ss.contains(&r"'\n'"), "{ss:?}");
        assert!(ss.contains(&r"'\''"), "{ss:?}");
    }

    #[test]
    fn numbers_bools_and_null() {
        let t = "var a = 42; var b = 0xFF_u; var c = 3.14f; var d = 1_000m; var ok = true; var z = null; var f = false;";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["42", "0xFF_u", "3.14f", "1_000m", "true", "null", "false"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn member_access_not_eaten_by_number() {
        let t = "var n = 5.ToString();";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        // `5` is a Constant; the `.ToString` must NOT be swallowed into it.
        assert_eq!(cs, vec!["5"], "{cs:?}");
    }

    #[test]
    fn definition_names() {
        let t = "public class Widget {}\nstruct Point {}\ninterface IShape {}\nenum Color {}\nrecord Pair {}\nnamespace App {}";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        for want in ["Widget", "Point", "IShape", "Color", "Pair", "App"] {
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
        let t = "var result = Compute(a, b) + offset;";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn reference_snippet() {
        // A compact end-to-end snippet asserting all four roles at once.
        let t = "// sum\nclass Calc {\n    int Add(int a, int b) { return a + b; } // ok\n    const int Max = 100;\n}\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["// sum", "// ok"], "{s:?}");
        assert!(at(t, &s, SynKind::Definition).contains(&"Calc"), "{s:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"100"), "{s:?}");
    }
}
