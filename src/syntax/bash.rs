//! Bash syntax lexer — a minimal hand-written byte scanner following the
//! reference lexers in [`crate::syntax::rust`] and [`crate::syntax::python`]. It
//! emits only the four Alabaster roles and leaves everything else (keywords like
//! `if`/`then`/`for`, operators, builtins, variable references, punctuation) as
//! the default ink:
//!
//! - [`SynKind::Comment`]    — `#` line comments (incl. the `#!` shebang). A `#`
//!   only opens a comment at a word boundary, so `$#`, `${#arr}`, and `a=b#c` stay
//!   plain — matching the shell's own rule.
//! - [`SynKind::Str`]        — `'literal'` (no escapes), `"double"` (with `\`
//!   escapes), the `$'ansi-c'` and `$"locale"` forms, and HERE-DOC bodies
//!   (`<<EOF` … `EOF`, including the `<<-` tab-stripped variant).
//! - [`SynKind::Constant`]   — numeric literals and the `true` / `false` words.
//! - [`SynKind::Definition`] — a function NAME: the identifier after the
//!   `function` keyword, or the name in the `name() { … }` form.
//!
//! Span boundaries land on ASCII bytes (`#`, quotes, `$`, digits, ASCII
//! identifiers), so multibyte UTF-8 inside a string/comment/here-doc rides inside
//! the span without ever splitting a char. Pure + single-pass (here-docs queue a
//! pending delimiter, flushed at the next newline). See the tests for the exact
//! contract on a representative script.

use super::SynKind;
use std::ops::Range;

/// Identifiers that are CONSTANT literals (the boolean builtins; Bash has no nil).
const CONST_WORDS: &[&str] = &["true", "false"];

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic()
}
fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
}
/// A `#` opens a comment only at a word boundary: start of input, or after
/// whitespace or a command separator. This keeps `$#` / `${#x}` / `a=b#c` plain.
fn opens_comment(prev: Option<u8>) -> bool {
    match prev {
        None => true,
        Some(p) => p.is_ascii_whitespace() || matches!(p, b';' | b'&' | b'|' | b'('),
    }
}

pub fn spans(text: &str) -> Vec<(Range<usize>, SynKind)> {
    let b = text.as_bytes();
    let n = b.len();
    let mut out: Vec<(Range<usize>, SynKind)> = Vec::new();
    let mut i = 0usize;
    // Set when the previous token was the `function` keyword; the next identifier
    // is then the defined NAME.
    let mut expect_def = false;
    // Pending here-doc delimiters (text, is-`<<-`): queued when we scan `<<WORD`,
    // flushed in FIFO order at each following newline.
    let mut heredocs: Vec<(String, bool)> = Vec::new();

    while i < n {
        let c = b[i];
        let prev = if i > 0 { Some(b[i - 1]) } else { None };

        // --- newline: flush the next pending here-doc body ---
        if c == b'\n' {
            if !heredocs.is_empty() {
                let (delim, dash) = heredocs.remove(0);
                let body_start = i + 1;
                let body_end = scan_heredoc_body(b, body_start, &delim, dash);
                if body_end > body_start {
                    out.push((body_start..body_end, SynKind::Str));
                }
                i = body_end;
                continue;
            }
            i += 1;
            continue;
        }

        // --- line comment ---
        if c == b'#' && opens_comment(prev) {
            let start = i;
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            out.push((start..i, SynKind::Comment));
            expect_def = false;
            continue;
        }

        // --- `$`: ANSI-C / locale string, or a variable reference (consumed so a
        // `$1` positional or `$#` does not read as a number / comment) ---
        if c == b'$' && i + 1 < n {
            match b[i + 1] {
                b'\'' => {
                    let end = scan_quoted(b, i + 1, true);
                    out.push((i..end, SynKind::Str));
                    i = end;
                    expect_def = false;
                    continue;
                }
                b'"' => {
                    let end = scan_quoted(b, i + 1, true);
                    out.push((i..end, SynKind::Str));
                    i = end;
                    expect_def = false;
                    continue;
                }
                b'{' => {
                    // `${ … }` — skip the whole expansion (contents stay plain).
                    i += 2;
                    while i < n && b[i] != b'}' {
                        i += 1;
                    }
                    if i < n {
                        i += 1;
                    }
                    expect_def = false;
                    continue;
                }
                b'(' => {
                    // `$( … )` command substitution — let the inner code scan
                    // normally so it highlights too. Just step over the `$`.
                    i += 1;
                    continue;
                }
                d if is_ident_start(d) || d.is_ascii_digit() => {
                    i += 2;
                    while i < n && is_ident_continue(b[i]) {
                        i += 1;
                    }
                    expect_def = false;
                    continue;
                }
                b'@' | b'*' | b'#' | b'?' | b'!' | b'-' | b'$' => {
                    i += 2;
                    expect_def = false;
                    continue;
                }
                _ => {
                    i += 1;
                    continue;
                }
            }
        }

        // --- single-quoted string (literal: no escapes) ---
        if c == b'\'' {
            let end = scan_quoted(b, i, false);
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- double-quoted string (with `\` escapes) ---
        if c == b'"' {
            let end = scan_quoted(b, i, true);
            out.push((i..end, SynKind::Str));
            i = end;
            expect_def = false;
            continue;
        }

        // --- here-doc opener `<<WORD` / `<<-WORD` (not the `<<<` here-string) ---
        if c == b'<' && i + 1 < n && b[i + 1] == b'<' {
            if i + 2 < n && b[i + 2] == b'<' {
                i += 3; // `<<<` here-string: the operand scans as a normal arg
                continue;
            }
            if let Some((end, delim, dash)) = heredoc_delim(b, i) {
                heredocs.push((delim, dash));
                i = end;
                expect_def = false;
                continue;
            }
            i += 2;
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
            } else if word == "function" {
                expect_def = true;
            } else if is_func_def(b, i) {
                // The `name() { … }` form — the name is the definition.
                out.push((start..i, SynKind::Definition));
            }
            continue;
        }

        // Any other byte (operator, punctuation) stays default ink, and ends any
        // pending `function`-name expectation if a name never materialized.
        if !c.is_ascii_whitespace() {
            expect_def = false;
        }
        i += 1;
    }

    out
}

/// Scan a quoted string starting at the opening quote `q`; returns the index just
/// past the closing quote (or EOF if unterminated). With `escapes`, a `\\` skips
/// the next byte so an escaped quote does not close the string — that is the rule
/// for `"…"`, `$'…'`, and `$"…"`. Without it (a `'…'` literal) backslashes are
/// ordinary and only the next `'` closes.
fn scan_quoted(b: &[u8], q: usize, escapes: bool) -> usize {
    let n = b.len();
    let quote = b[q];
    let mut i = q + 1;
    while i < n {
        let c = b[i];
        if escapes && c == b'\\' {
            i += 2;
        } else if c == quote {
            return i + 1;
        } else {
            i += 1;
        }
    }
    n
}

/// True if a function definition of the form `name ()` / `name()` follows the
/// identifier that ends at `i` — i.e. (optional spaces) `(` (optional spaces) `)`.
fn is_func_def(b: &[u8], i: usize) -> bool {
    let n = b.len();
    let mut k = i;
    while k < n && (b[k] == b' ' || b[k] == b'\t') {
        k += 1;
    }
    if k >= n || b[k] != b'(' {
        return false;
    }
    k += 1;
    while k < n && (b[k] == b' ' || b[k] == b'\t') {
        k += 1;
    }
    k < n && b[k] == b')'
}

/// Parse a here-doc delimiter starting at the `<<` (`b[i] == b[i+1] == b'<'`).
/// Returns `(end, delim, is_dash)` where `end` is the index just past the
/// delimiter word, `delim` is the (unquoted) terminator to match, and `is_dash`
/// marks the `<<-` tab-stripping form. `None` if no delimiter word follows.
fn heredoc_delim(b: &[u8], i: usize) -> Option<(usize, String, bool)> {
    let n = b.len();
    let mut j = i + 2;
    let dash = j < n && b[j] == b'-';
    if dash {
        j += 1;
    }
    while j < n && (b[j] == b' ' || b[j] == b'\t') {
        j += 1;
    }
    if j >= n {
        return None;
    }
    let mut delim = String::new();
    match b[j] {
        b'\'' | b'"' => {
            let quote = b[j];
            j += 1;
            while j < n && b[j] != quote {
                delim.push(b[j] as char);
                j += 1;
            }
            if j < n {
                j += 1; // past the closing quote
            }
        }
        _ => {
            if b[j] == b'\\' {
                j += 1; // `<<\EOF` — the backslash only suppresses expansion
            }
            while j < n && (is_ident_continue(b[j]) || b[j] == b'.' || b[j] == b'-') {
                delim.push(b[j] as char);
                j += 1;
            }
        }
    }
    if delim.is_empty() {
        None
    } else {
        Some((j, delim, dash))
    }
}

/// Scan a here-doc body beginning at `start` (the first byte of the line after the
/// opener). Returns the index of the start of the terminator line — i.e. the body
/// covers `start..return` — or EOF if the delimiter is never seen. For the `<<-`
/// form, leading TABS on a line are ignored when matching the terminator.
fn scan_heredoc_body(b: &[u8], start: usize, delim: &str, dash: bool) -> usize {
    let n = b.len();
    let mut line = start;
    while line < n {
        let ls = line;
        let mut le = line;
        while le < n && b[le] != b'\n' {
            le += 1;
        }
        let mut cs = ls;
        if dash {
            while cs < le && b[cs] == b'\t' {
                cs += 1;
            }
        }
        if &b[cs..le] == delim.as_bytes() {
            return ls;
        }
        if le >= n {
            return n;
        }
        line = le + 1;
    }
    n
}

/// Scan a numeric literal beginning at the digit `i`; returns the index just past
/// it. Accepts a `0x` hex run, `_` separators, and a fractional `.` followed by a
/// digit. Bash integers are the common case; this stays deliberately small.
fn scan_number(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i + 1;
    if b[i] == b'0' && j < n && matches!(b[j], b'x' | b'X') {
        j += 1;
        while j < n && (b[j].is_ascii_hexdigit() || b[j] == b'_') {
            j += 1;
        }
        return j;
    }
    while j < n {
        let c = b[j];
        if c.is_ascii_digit() || c == b'_' {
            j += 1;
        } else if c == b'.' && j + 1 < n && b[j + 1].is_ascii_digit() {
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

    fn has(s: &[(Range<usize>, SynKind)], lo: usize, hi: usize, k: SynKind) -> bool {
        s.iter().any(|(r, kk)| r.start == lo && r.end == hi && *kk == k)
    }
    /// The substring each span of role `k` covers, for readable assertions.
    fn at<'a>(text: &'a str, s: &[(Range<usize>, SynKind)], k: SynKind) -> Vec<&'a str> {
        s.iter().filter(|(_, kk)| *kk == k).map(|(r, _)| &text[r.clone()]).collect()
    }

    #[test]
    fn shebang_and_line_comment() {
        let t = "#!/bin/bash\nls -la # list files\n";
        let s = spans(t);
        assert_eq!(
            at(t, &s, SynKind::Comment),
            vec!["#!/bin/bash", "# list files"],
            "{s:?}"
        );
    }

    #[test]
    fn hash_not_at_word_boundary_is_not_a_comment() {
        // `$#`, `${#x}`, and `a=b#c` must stay plain (no comment span).
        let t = "echo a=b#c ${#arr} $#\n";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn single_double_and_dollar_strings() {
        let t = "x='raw \\n'\ny=\"hi $z\"\nz=$'a\\nb'\nw=$\"loc\"\n";
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        assert!(ss.contains(&"'raw \\n'"), "{ss:?}");
        assert!(ss.contains(&"\"hi $z\""), "{ss:?}");
        assert!(ss.contains(&"$'a\\nb'"), "{ss:?}");
        assert!(ss.contains(&"$\"loc\""), "{ss:?}");
    }

    #[test]
    fn escaped_quote_does_not_close_double_string() {
        let t = "s=\"a\\\"b\"";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec!["\"a\\\"b\""], "{s:?}");
    }

    #[test]
    fn numbers_and_booleans() {
        let t = "n=42\nh=0xFF\nf=3.14\nok=true\nbad=false\n";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["42", "0xFF", "3.14", "true", "false"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn positional_param_is_not_a_number() {
        // `$1` is a variable reference, not the constant `1`.
        let t = "echo $1 $2";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn function_keyword_and_paren_forms() {
        let t = "function greet {\n  echo hi\n}\nhello() {\n  echo yo\n}\n";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"greet"), "{ds:?}");
        assert!(ds.contains(&"hello"), "{ds:?}");
        // The `function` keyword itself stays plain.
        assert!(!has(&s, 0, 8, SynKind::Definition), "`function` must stay plain: {s:?}");
    }

    #[test]
    fn keywords_are_not_styled() {
        // `if`/`then`/`fi` ride the default ink — nothing to highlight here.
        let t = "if [ -f x ]; then\n  echo ok\nfi\n";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn heredoc_body_is_a_string() {
        let t = "cat <<EOF\nhello $name\nworld\nEOF\necho done\n";
        let s = spans(t);
        assert_eq!(
            at(t, &s, SynKind::Str),
            vec!["hello $name\nworld\n"],
            "{s:?}"
        );
    }

    #[test]
    fn dash_heredoc_strips_leading_tabs_on_terminator() {
        let t = "cat <<-END\n\tindented\n\tEND\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec!["\tindented\n"], "{s:?}");
    }

    #[test]
    fn plain_code_has_no_spans() {
        let t = "result=$(compute a b)\nexport PATH=/usr/bin\n";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn reference_snippet() {
        let t = "#!/bin/sh\n# greet someone\ngreet() {\n  local name=\"$1\"\n  echo \"hi $name\" # say it\n}\nMAX=100\n";
        let s = spans(t);
        assert_eq!(
            at(t, &s, SynKind::Comment),
            vec!["#!/bin/sh", "# greet someone", "# say it"],
            "{s:?}"
        );
        assert!(at(t, &s, SynKind::Definition).contains(&"greet"), "{s:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"100"), "{s:?}");
        assert!(at(t, &s, SynKind::Str).contains(&"\"$1\""), "{s:?}");
    }
}
