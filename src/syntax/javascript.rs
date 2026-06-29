//! JavaScript syntax lexer — follows the `rust.rs` / `python.rs` template. A
//! minimal hand-written byte scanner emitting only the four Alabaster roles;
//! everything else (keywords, operators, identifiers, punctuation) stays the
//! default ink:
//!
//! - [`SynKind::Comment`]    — `// line` and `/* block */` comments (JS does NOT
//!   nest block comments — the first `*/` closes).
//! - [`SynKind::Str`]        — `"..."` / `'...'` strings and `` `...` `` template
//!   literals (a template — interpolations and all — is ONE `Str` span).
//! - [`SynKind::Constant`]   — numeric literals (incl. `0x`/`0o`/`0b`, floats,
//!   exponents, `_` separators, BigInt `n` suffix) and the `true` / `false` /
//!   `null` / `undefined` / `NaN` / `Infinity` literals.
//! - [`SynKind::Definition`] — the identifier right after a `function` / `class` /
//!   `const` / `let` / `var` introducer.
//!
//! Span boundaries land on ASCII bytes (quotes, `/`, digits, ASCII identifiers),
//! so multibyte UTF-8 inside a string/comment rides inside the span without ever
//! splitting a char. Pure + single-pass. JS's regex-literal-vs-division ambiguity
//! is deliberately sidestepped: a lone `/` is left as a plain operator (regex
//! literals are not one of the four roles), so no division is ever mis-read.

use super::SynKind;
use std::ops::Range;

/// Introducers after which the next identifier is the DEFINITION name.
const DEF_KEYWORDS: &[&str] = &["function", "class", "const", "let", "var"];
/// Identifiers that are CONSTANT literals (booleans + the nil-style + numeric
/// keyword values JS exposes as globals).
const CONST_WORDS: &[&str] = &["true", "false", "null", "undefined", "NaN", "Infinity"];

use super::{is_ident_continue_dollar as is_ident_continue, is_ident_start_dollar as is_ident_start};

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

        // --- block comment (JS does NOT nest: first `*/` closes) ---
        if c == b'/' && i + 1 < n && b[i + 1] == b'*' {
            let end = super::scan_block_comment(b, i, false);
            out.push((i..end, SynKind::Comment));
            i = end;
            continue;
        }

        // --- template literal: `...` (may span lines; `${}` rides inside) ---
        if c == b'`' {
            let end = scan_template(b, i);
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- normal string: "..." or '...' ---
        if c == b'"' || c == b'\'' {
            let end = scan_string(b, i);
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
        // A `*` between `function` and its name (a generator: `function* g`) must
        // not clear the expectation; other non-whitespace tokens do.
        if !c.is_ascii_whitespace() && c != b'*' {
            expect_def = false;
        }
        i += 1;
    }

    out
}

/// Scan a normal quoted string from the opening quote `q` to just past its close
/// (or EOF / end-of-line — a `'`/`"` string does not cross a raw newline). Honors
/// `\\` escapes so an escaped quote does not close the string.
fn scan_string(b: &[u8], q: usize) -> usize {
    super::scan_quoted(b, q, b[q], true)
}

/// Scan a template literal from the opening backtick to just past its close (or
/// EOF). Templates may span newlines; `${...}` interpolations ride INSIDE the one
/// span. Honors `\\` escapes.
fn scan_template(b: &[u8], q: usize) -> usize {
    let n = b.len();
    let mut i = q + 1;
    while i < n {
        match b[i] {
            b'\\' => i += 2,
            b'`' => return i + 1,
            _ => i += 1,
        }
    }
    n
}

/// Scan a numeric literal beginning at the digit `i`; returns the index just past
/// it. Accepts `0x`/`0o`/`0b` radixes, `_` separators, a fractional `.`, an
/// exponent, and a trailing BigInt `n` suffix. A `..` is not consumed.
fn scan_number(b: &[u8], i: usize) -> usize {
    super::scan_number(
        b,
        i,
        super::NumOpts { radix: b"xXoObB", radix_extra: b"", dot_dot_stops: true },
        is_ident_start,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::testutil::{at, has};

    #[test]
    fn line_comment() {
        let t = "let x = 1; // hi there\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["// hi there"], "{s:?}");
    }

    #[test]
    fn block_comment_not_nested() {
        // JS block comments do NOT nest: the first `*/` closes.
        let t = "/* a /* b */ c */ x";
        let s = spans(t);
        assert!(has(&s, 0, 12, SynKind::Comment), "{s:?}");
    }

    #[test]
    fn string_with_escaped_quote() {
        let t = r#"let s = "a\"b";"#;
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec![r#""a\"b""#], "{s:?}");
    }

    #[test]
    fn single_quoted_string() {
        let t = "let s = 'hi';";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec!["'hi'"], "{s:?}");
    }

    #[test]
    fn template_literal_with_interpolation() {
        let t = "let s = `hi ${name} and\nmore`;";
        let s = spans(t);
        // The whole template (interpolation + newline) is ONE Str span.
        assert_eq!(at(t, &s, SynKind::Str), vec!["`hi ${name} and\nmore`"], "{s:?}");
    }

    #[test]
    fn numbers_and_constants() {
        let t = "let a = 42; let b = 0xFF; let c = 3.14; let d = 1_000n; let ok = true; let z = null;";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["42", "0xFF", "3.14", "1_000n", "true", "null"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn undefined_and_nan_are_constants() {
        let t = "let a = undefined; let b = NaN; let c = Infinity;";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["undefined", "NaN", "Infinity"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn definition_after_function_class_and_binding() {
        let t = "function frobnicate(x) {}\nclass Widget {}\nconst MAX = 100;\nlet count = 0;";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"frobnicate"), "{ds:?}");
        assert!(ds.contains(&"Widget"), "{ds:?}");
        assert!(ds.contains(&"MAX"), "{ds:?}");
        assert!(ds.contains(&"count"), "{ds:?}");
    }

    #[test]
    fn generator_name_after_function_star() {
        // `function* gen` — the `*` must not clear the definition expectation.
        let t = "function* gen() {}";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Definition).contains(&"gen"), "{s:?}");
    }

    #[test]
    fn keyword_itself_is_not_styled() {
        // `function` keyword stays default ink; only the NAME is a Definition.
        let t = "function main() {}";
        let s = spans(t);
        assert!(!has(&s, 0, 8, SynKind::Definition), "the `function` keyword must stay plain: {s:?}");
        assert!(has(&s, 9, 13, SynKind::Definition), "`main` is the definition: {s:?}");
    }

    #[test]
    fn division_is_not_a_comment_or_string() {
        // A lone `/` is plain — not a comment, not a regex Str.
        let t = "return a / b;";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn plain_code_has_no_spans() {
        let t = "return compute(a, b) + offset;";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn reference_snippet() {
        // A compact end-to-end snippet asserting all four roles at once.
        let t = "// sum\nfunction add(a, b) {\n    const total = a + b; // ok\n    return total;\n}\nconst MAX = 100;\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["// sum", "// ok"], "{s:?}");
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"add") && ds.contains(&"MAX") && ds.contains(&"total"), "{ds:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"100"), "{s:?}");
    }
}
