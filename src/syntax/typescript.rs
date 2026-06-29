//! TypeScript syntax lexer — a minimal hand-written byte scanner mirroring the
//! reference lexers in `rust.rs` / `python.rs`. It recognizes only what the four
//! Alabaster roles need and leaves everything else (keywords, operators,
//! identifiers, punctuation) as the default ink:
//!
//! - [`SynKind::Comment`]    — `// line` and `/* block */` comments (TS blocks do
//!   NOT nest — the first `*/` closes).
//! - [`SynKind::Str`]        — `"..."`, `'...'`, and `` `...` `` template literals
//!   (multiline; an interpolated `${…}` rides inside the one Str span).
//! - [`SynKind::Constant`]   — numeric literals (`0x`/`0o`/`0b`, floats, `_`
//!   separators, exponents, the `n` BigInt suffix) and `true` / `false` / `null`
//!   / `undefined`.
//! - [`SynKind::Definition`] — the identifier right after a `function` / `class` /
//!   `interface` / `type` / `enum` / `namespace` / `module` introducer or a
//!   `const` / `let` / `var` binding.
//!
//! Span boundaries land on ASCII bytes (quotes, `/`, digits, ASCII identifiers),
//! so multibyte UTF-8 inside a string/comment rides inside the span without ever
//! splitting a char. Pure + single-pass. See the tests below for the contract.

use super::SynKind;
use std::ops::Range;

/// Introducers after which the next identifier is the DEFINITION name.
const DEF_KEYWORDS: &[&str] = &[
    "function", "class", "interface", "type", "enum", "namespace", "module",
    "const", "let", "var",
];

/// Identifiers that are CONSTANT literals (booleans + the nil-style values).
const CONST_WORDS: &[&str] = &["true", "false", "null", "undefined"];

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c == b'$' || c.is_ascii_alphabetic()
}
fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c == b'$' || c.is_ascii_alphanumeric()
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
            let end = super::scan_line_comment(b, i);
            out.push((i..end, SynKind::Comment));
            i = end;
            continue;
        }

        // --- block comment (TS does NOT nest — first `*/` closes) ---
        if c == b'/' && i + 1 < n && b[i + 1] == b'*' {
            let end = super::scan_block_comment(b, i, false);
            out.push((i..end, SynKind::Comment));
            i = end;
            continue;
        }

        // --- template literal: `...` (multiline; `${…}` rides inside) ---
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
        if !c.is_ascii_whitespace() {
            // A non-identifier, non-whitespace token after a def keyword means the
            // name never materialized — drop the expectation.
            expect_def = false;
        }
        i += 1;
    }

    out
}

/// Scan a normal quoted string (`"` or `'`) starting at the opening quote `q`;
/// returns the index just past the closing quote (or EOF / end-of-line — a normal
/// TS string does not cross an unescaped newline). Honors `\\` escapes.
fn scan_string(b: &[u8], q: usize) -> usize {
    super::scan_quoted(b, q, b[q], true)
}

/// Scan a template literal from the opening backtick `q` to just past its close
/// (or EOF). Templates span newlines; honors `\\` escapes. An interpolated `${…}`
/// is NOT lexed specially — the whole literal is one [`SynKind::Str`] span.
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
/// exponent, and the trailing `n` BigInt suffix.
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
        assert!(at(t, &s, SynKind::Comment) == vec!["// hi there"], "{s:?}");
    }

    #[test]
    fn block_comment_does_not_nest() {
        let t = "/* a /* b */ c */ x";
        let s = spans(t);
        // The FIRST `*/` closes the block (TS comments don't nest).
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
        let t = "const c = 'hi';";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec!["'hi'"], "{s:?}");
    }

    #[test]
    fn template_literal_multiline_and_interpolation() {
        let t = "const g = `line one ${x + 1}\nline two`;";
        let s = spans(t);
        // The whole template — newline + `${…}` — is ONE Str span.
        assert_eq!(
            at(t, &s, SynKind::Str),
            vec!["`line one ${x + 1}\nline two`"],
            "{s:?}"
        );
    }

    #[test]
    fn numbers_and_constants() {
        let t = "let a = 42; let b = 0xFF; let c = 3.14; let d = 1_000n; let ok = true; let z = null; let u = undefined;";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["42", "0xFF", "3.14", "1_000n", "true", "null", "undefined"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn spread_not_eaten_by_number() {
        let t = "const xs = [0, ...rest];";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        assert!(cs.contains(&"0"), "{cs:?}");
    }

    #[test]
    fn definitions_after_introducers() {
        let t = "function frobnicate(x: number) {}\nclass Widget {}\ninterface Shape {}\ntype Alias = number;\nenum Color {}\nconst MAX = 100;";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        for want in ["frobnicate", "Widget", "Shape", "Alias", "Color", "MAX"] {
            assert!(ds.contains(&want), "missing {want}: {ds:?}");
        }
    }

    #[test]
    fn keyword_itself_is_not_styled() {
        // `function` keyword stays default ink; only the NAME is a Definition.
        let t = "function main() {}";
        let s = spans(t);
        assert!(!has(&s, 0, 8, SynKind::Definition), "the keyword must stay plain: {s:?}");
        assert!(has(&s, 9, 13, SynKind::Definition), "`main` is the definition: {s:?}");
    }

    #[test]
    fn plain_code_has_no_spans() {
        // No comment / literal / def-keyword -> nothing highlighted (Alabaster).
        let t = "result = compute(a, b) + offset;";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn reference_snippet() {
        let t = "// sum\nfunction add(a: number, b: number): number {\n    const total = a + b; // ok\n    return total;\n}\nconst MAX = 100;\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["// sum", "// ok"], "{s:?}");
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"add") && ds.contains(&"total") && ds.contains(&"MAX"), "{ds:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"100"), "{s:?}");
    }
}
