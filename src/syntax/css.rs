//! CSS syntax lexer — a minimal hand-written byte scanner in the shape of the
//! reference lexers (`rust.rs` / `python.rs`). It recognizes only what the four
//! Alabaster roles need and leaves everything else (selectors, property names,
//! keywords, named colors like `red`, operators, punctuation) as the default ink:
//!
//! - [`SynKind::Comment`]    — `/* block */` comments (CSS has no line comment and
//!   block comments do NOT nest).
//! - [`SynKind::Str`]        — `"..."` / `'...'` string literals (honoring `\`
//!   escapes; a CSS string does not cross an unescaped newline).
//! - [`SynKind::Constant`]   — numeric literals with their unit/percent suffix
//!   (`10px`, `1.5em`, `50%`, `1e3`) and `#rgb`/`#rgba`/`#rrggbb`/`#rrggbbaa` hex
//!   colors. CSS has no boolean/null literals.
//! - [`SynKind::Definition`] — the NAME being defined: a custom-property name in
//!   declaration position (`--brand:` — distinguished from a `var(--brand)` USE by
//!   the trailing `:`) and the name after an `@keyframes` / `@counter-style`
//!   introducer.
//!
//! Span boundaries land on ASCII bytes (`/`, quotes, `#`, digits, ASCII idents),
//! so multibyte UTF-8 inside a string/comment rides inside the span without ever
//! splitting a char. Pure + single-pass. See the tests for the exact contract.

use super::SynKind;
use std::ops::Range;

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c == b'-' || c.is_ascii_alphabetic()
}
fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c == b'-' || c.is_ascii_alphanumeric()
}

/// True if an at-rule named `name` (vendor prefix allowed) introduces a NAME, i.e.
/// the next identifier is the DEFINITION (`@keyframes foo`, `@counter-style bar`).
fn is_def_at_rule(name: &str) -> bool {
    name == "keyframes" || name.ends_with("-keyframes") || name == "counter-style"
}

pub fn spans(text: &str) -> Vec<(Range<usize>, SynKind)> {
    let b = text.as_bytes();
    let n = b.len();
    let mut out: Vec<(Range<usize>, SynKind)> = Vec::new();
    let mut i = 0usize;
    // Set when the previous token was a naming at-rule introducer; the next
    // identifier is then the defined NAME.
    let mut expect_def = false;

    while i < n {
        let c = b[i];

        // --- block comment (CSS comments do NOT nest) ---
        if c == b'/' && i + 1 < n && b[i + 1] == b'*' {
            let end = scan_block_comment(b, i);
            out.push((i..end, SynKind::Comment));
            i = end;
            continue;
        }

        // --- string ---
        if c == b'"' || c == b'\'' {
            let end = scan_string(b, i);
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- at-rule: the `@name` keyword stays plain, but a naming at-rule arms
        // the next identifier as the DEFINITION name. ---
        if c == b'@' {
            let mut j = i + 1;
            while j < n && is_ident_continue(b[j]) {
                j += 1;
            }
            expect_def = is_def_at_rule(&text[i + 1..j]);
            i = j;
            continue;
        }

        // --- hex color (`#fff`, `#ffffffaa`) — vs an id selector like `#header` ---
        if c == b'#' {
            if let Some(end) = hex_color(b, i) {
                out.push((i..end, SynKind::Constant));
                i = end;
                expect_def = false;
                continue;
            }
            i += 1;
            continue;
        }

        // --- number (with optional sign, fraction, exponent, unit/percent) ---
        if is_number_start(b, i) {
            let start = i;
            i = scan_number(b, i);
            out.push((start..i, SynKind::Constant));
            expect_def = false;
            continue;
        }

        // --- custom property: `--name` is a DEFINITION only in declaration
        // position (a trailing `:`); a `var(--name)` USE is left plain. ---
        if c == b'-' && i + 1 < n && b[i + 1] == b'-' {
            let mut j = i + 2;
            while j < n && is_ident_continue(b[j]) {
                j += 1;
            }
            if next_significant_is_colon(b, j) {
                out.push((i..j, SynKind::Definition));
            }
            i = j;
            expect_def = false;
            continue;
        }

        // --- identifier (selector / property / keyword / value) ---
        if is_ident_start(c) {
            let start = i;
            i += 1;
            while i < n && is_ident_continue(b[i]) {
                i += 1;
            }
            if expect_def {
                out.push((start..i, SynKind::Definition));
                expect_def = false;
            }
            // Everything else (keywords, named colors, property names) stays plain.
            continue;
        }

        // Any other byte (operator, punctuation) — a non-whitespace token between a
        // naming at-rule and its name means the name never materialized.
        if !c.is_ascii_whitespace() {
            expect_def = false;
        }
        i += 1;
    }

    out
}

/// Scan a `/* … */` block comment from the opening `/` at `i` to just past the
/// closing `*/` (or EOF if unterminated). CSS block comments do not nest.
fn scan_block_comment(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i + 2;
    while j < n {
        if b[j] == b'*' && j + 1 < n && b[j + 1] == b'/' {
            return j + 2;
        }
        j += 1;
    }
    n
}

/// Scan a quoted string from the opening quote `q` to just past its close (or EOF /
/// end-of-line — a CSS string does not cross an unescaped newline). Honors `\\`.
fn scan_string(b: &[u8], q: usize) -> usize {
    super::scan_quoted(b, q, b[q], true)
}

/// True if a numeric literal starts at `i`: a digit, a `.`-then-digit, or a leading
/// `+`/`-` sign immediately before either.
fn is_number_start(b: &[u8], i: usize) -> bool {
    let n = b.len();
    let c = b[i];
    if c.is_ascii_digit() {
        return true;
    }
    if c == b'.' && i + 1 < n && b[i + 1].is_ascii_digit() {
        return true;
    }
    if (c == b'-' || c == b'+') && i + 1 < n {
        if b[i + 1].is_ascii_digit() {
            return true;
        }
        if b[i + 1] == b'.' && i + 2 < n && b[i + 2].is_ascii_digit() {
            return true;
        }
    }
    false
}

/// Scan a numeric literal beginning at `i`; returns the index just past it. Accepts
/// a leading sign, a fractional `.`, an `e`/`E` exponent, and a trailing unit
/// (`px`/`em`/…) or `%`.
fn scan_number(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i;
    if b[j] == b'-' || b[j] == b'+' {
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
    // Exponent: `e`/`E` only when followed by an (optionally signed) digit, so a
    // unit like `em` is NOT mistaken for one.
    if j < n && (b[j] == b'e' || b[j] == b'E') {
        let mut k = j + 1;
        if k < n && (b[k] == b'+' || b[k] == b'-') {
            k += 1;
        }
        if k < n && b[k].is_ascii_digit() {
            j = k;
            while j < n && b[j].is_ascii_digit() {
                j += 1;
            }
        }
    }
    // Unit suffix: a `%` or an alphabetic dimension (`px`, `deg`, `rem`, …).
    if j < n && b[j] == b'%' {
        j += 1;
    } else {
        while j < n && b[j].is_ascii_alphabetic() {
            j += 1;
        }
    }
    j
}

/// If a hex color (`#` then 3/4/6/8 hex digits, not part of a longer name) starts at
/// `i`, return the index just past it; else `None` (e.g. an id selector `#header`).
fn hex_color(b: &[u8], i: usize) -> Option<usize> {
    let n = b.len();
    let mut j = i + 1;
    while j < n && b[j].is_ascii_hexdigit() {
        j += 1;
    }
    let len = j - (i + 1);
    // A trailing identifier char means it is a name (`#abcg`, `#fff-x`), not a color.
    if j < n && is_ident_continue(b[j]) {
        return None;
    }
    if matches!(len, 3 | 4 | 6 | 8) {
        Some(j)
    } else {
        None
    }
}

/// Starting at `j`, skip whitespace and `/* … */` comments and report whether the
/// next significant byte is a `:` — i.e. whether a `--name` is in declaration
/// position (a definition) rather than a `var(--name)` use.
fn next_significant_is_colon(b: &[u8], j: usize) -> bool {
    let n = b.len();
    let mut k = j;
    loop {
        while k < n && b[k].is_ascii_whitespace() {
            k += 1;
        }
        if k + 1 < n && b[k] == b'/' && b[k + 1] == b'*' {
            k = scan_block_comment(b, k);
            continue;
        }
        break;
    }
    k < n && b[k] == b':'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::testutil::{at, has};

    #[test]
    fn block_comment() {
        let t = "/* a comment */\nbody { color: red; }";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["/* a comment */"], "{s:?}");
    }

    #[test]
    fn multiline_block_comment_is_one_span() {
        let t = "/* line one\n   line two */\np {}";
        let s = spans(t);
        assert!(has(&s, 0, 26, SynKind::Comment), "{s:?}");
    }

    #[test]
    fn strings_both_quotes() {
        let t = "a { content: \"hi\"; background: url('x.png'); }";
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        assert!(ss.contains(&"\"hi\""), "{ss:?}");
        assert!(ss.contains(&"'x.png'"), "{ss:?}");
    }

    #[test]
    fn string_with_escaped_quote() {
        let t = "a { content: \"a\\\"b\"; }";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec!["\"a\\\"b\""], "{s:?}");
    }

    #[test]
    fn numbers_units_percent_and_hex() {
        let t = "x { margin: 10px; width: 50%; line-height: 1.5; color: #336699; opacity: .5; }";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["10px", "50%", "1.5", "#336699", ".5"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn short_and_alpha_hex_colors() {
        let t = "a { color: #fff; border-color: #ffaa0080; }";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        assert!(cs.contains(&"#fff"), "{cs:?}");
        assert!(cs.contains(&"#ffaa0080"), "{cs:?}");
    }

    #[test]
    fn id_selector_is_not_a_hex_color() {
        let t = "#header { color: red; }";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Constant).is_empty(), "{s:?}");
    }

    #[test]
    fn custom_property_definition_vs_use() {
        let t = ":root { --brand: #336699; }\na { color: var(--brand); }";
        let s = spans(t);
        // The declaration `--brand:` is a Definition; the `var(--brand)` use is not.
        assert_eq!(at(t, &s, SynKind::Definition), vec!["--brand"], "{s:?}");
    }

    #[test]
    fn keyframes_name_is_a_definition() {
        let t = "@keyframes slide { from { left: 0; } to { left: 100px; } }";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"slide"), "{ds:?}");
        // The `@keyframes` keyword itself stays plain (no span over it).
        assert!(!has(&s, 0, 10, SynKind::Definition), "{s:?}");
    }

    #[test]
    fn property_and_selector_are_not_highlighted() {
        // A bare rule has no comment/literal/definition -> nothing highlighted.
        let t = "div.card > p:hover { color: red; display: flex; }";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn em_unit_is_not_an_exponent() {
        let t = "h1 { font-size: 3em; margin: 1e3px; }";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        assert!(cs.contains(&"3em"), "{cs:?}");
        assert!(cs.contains(&"1e3px"), "{cs:?}");
    }

    #[test]
    fn reference_snippet() {
        let t = "/* theme */\n:root { --gap: 8px; }\n@keyframes pulse { to { opacity: 1; } }\n.box { margin: var(--gap); color: #f00; }\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["/* theme */"], "{s:?}");
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"--gap"), "{ds:?}");
        assert!(ds.contains(&"pulse"), "{ds:?}");
        let cs = at(t, &s, SynKind::Constant);
        assert!(cs.contains(&"8px"), "{cs:?}");
        assert!(cs.contains(&"#f00"), "{cs:?}");
    }
}
