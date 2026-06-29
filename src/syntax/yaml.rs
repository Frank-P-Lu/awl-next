//! YAML syntax lexer — a minimal hand-written scanner following the reference
//! lexers (`rust.rs`, `python.rs`). It recognizes only what the four Alabaster
//! roles need and leaves everything else (plain scalars, anchors `&a`, aliases
//! `*a`, tags `!!str`, the `:`/`-` structure punctuation) as the default ink:
//!
//! - [`SynKind::Comment`]    — `# line` comments. YAML only comments when the `#`
//!   begins a token (start of line or preceded by whitespace), so a `#` inside a
//!   plain scalar (`http://x#frag`) is NOT a comment.
//! - [`SynKind::Str`]        — `"double"` (with `\` escapes) and `'single'` (with
//!   the doubled-quote `''` escape) quoted scalars, plus the whole body of a `|` /
//!   `>` block scalar (a multi-line string literal).
//! - [`SynKind::Constant`]   — numeric scalars (ints, floats, `0x`/`0o`/`0b`,
//!   `.inf`/`.nan`) and the boolean / null words (`true`/`false`, `null`/`~`, and
//!   the YAML-1.1 `yes`/`no`/`on`/`off` family, any case).
//! - [`SynKind::Definition`] — the mapping KEY (`key: value`) — YAML's analog of a
//!   defined name. Only PLAIN keys are marked; a quoted key rides as a `Str`.
//!
//! YAML is line-oriented, so the scan walks line by line (still emitting document
//! byte offsets); per line it peels the indentation + `- ` sequence markers, marks
//! a plain `key:` as the Definition, then scans the value for strings / numbers /
//! constants / a trailing comment. Span boundaries land on ASCII bytes (quotes,
//! `#`, digits), so multibyte UTF-8 inside a scalar rides inside the span. Pure +
//! single-pass per line. See the tests at the bottom for the exact contract.

use super::SynKind;
use std::ops::Range;

/// Boolean + null scalar words YAML treats as constants (Core schema plus the
/// YAML-1.1 `yes`/`no`/`on`/`off` family that most tooling still honors). `~` is
/// the shorthand null. Matched as a WHOLE plain token, so `online` is untouched.
const CONST_WORDS: &[&str] = &[
    "true", "false", "True", "False", "TRUE", "FALSE", //
    "null", "Null", "NULL", "~", //
    "yes", "no", "Yes", "No", "YES", "NO", //
    "on", "off", "On", "Off", "ON", "OFF",
];

pub fn spans(text: &str) -> Vec<(Range<usize>, SynKind)> {
    let b = text.as_bytes();
    let n = b.len();
    let mut out: Vec<(Range<usize>, SynKind)> = Vec::new();
    // When `Some(indent)`, we are inside a `|`/`>` block scalar opened by a key at
    // that indentation; deeper (or blank) lines are its literal string body.
    let mut block_indent: Option<usize> = None;

    let mut ls = 0usize;
    while ls <= n {
        let le = line_end(b, ls);
        let indent = leading_spaces(b, ls, le);
        let blank = ls + indent >= le;

        let mut handled = false;
        if let Some(bi) = block_indent {
            if blank {
                handled = true; // blank lines stay inside the block
            } else if indent > bi {
                // Literal block body: the whole content is one string span.
                out.push((ls + indent..le, SynKind::Str));
                handled = true;
            } else {
                block_indent = None; // dedent ends the block; fall through
            }
        }
        if !handled {
            block_indent = scan_line(b, ls, le, indent, &mut out);
        }

        if le == n {
            break;
        }
        ls = le + 1;
    }
    out
}

/// Index of the end of the line starting at `ls` (the `\n`, or `n` at EOF).
fn line_end(b: &[u8], ls: usize) -> usize {
    let mut i = ls;
    while i < b.len() && b[i] != b'\n' {
        i += 1;
    }
    i
}

/// Count leading spaces of the line `[ls, le)` (YAML forbids tab indentation).
fn leading_spaces(b: &[u8], ls: usize, le: usize) -> usize {
    let mut k = 0;
    while ls + k < le && b[ls + k] == b' ' {
        k += 1;
    }
    k
}

/// Scan one line `[ls, le)` (whose leading-space `indent` is precomputed): mark a
/// plain mapping key as `Definition` and scan the value. Returns `Some(indent)` if
/// the value opened a `|`/`>` block scalar (so the caller treats deeper lines as
/// its string body).
fn scan_line(
    b: &[u8],
    ls: usize,
    le: usize,
    indent: usize,
    out: &mut Vec<(Range<usize>, SynKind)>,
) -> Option<usize> {
    let mut i = ls + indent;
    // Peel any `- ` sequence markers (possibly nested: `- - item`).
    loop {
        if i < le && b[i] == b'-' && (i + 1 == le || b[i + 1] == b' ') {
            i += 1;
            while i < le && b[i] == b' ' {
                i += 1;
            }
        } else {
            break;
        }
    }
    if i >= le {
        return None;
    }
    let content = i;

    // --- plain mapping key: `name:` (colon followed by space / EOL) ---
    let mut value_start = content;
    if b[content] != b'"' && b[content] != b'\'' && b[content] != b'#' {
        let mut j = content;
        let mut colon = None;
        while j < le {
            let c = b[j];
            if c == b'#' && j > content && b[j - 1] == b' ' {
                break; // a comment begins before any `: ` — not a key line
            }
            if c == b':' && (j + 1 == le || b[j + 1] == b' ' || b[j + 1] == b'\t') {
                colon = Some(j);
                break;
            }
            j += 1;
        }
        if let Some(j) = colon {
            let mut ke = j; // trim trailing spaces from the key (`key : v`)
            while ke > content && b[ke - 1] == b' ' {
                ke -= 1;
            }
            if ke > content {
                out.push((content..ke, SynKind::Definition));
            }
            value_start = j + 1;
        }
    }

    // --- block-scalar indicator: a value of `|`/`>` (+ chomp/indent indicators) ---
    let block = block_scalar(b, value_start, le).then_some(indent);

    scan_value(b, value_start, le, out);
    block
}

/// Does the value `[start, le)` consist solely of a `|`/`>` block-scalar header
/// (the indicator, optional `+`/`-`/digit modifiers, then only spaces or a
/// trailing comment)?
fn block_scalar(b: &[u8], start: usize, le: usize) -> bool {
    let mut i = start;
    while i < le && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    if i >= le || (b[i] != b'|' && b[i] != b'>') {
        return false;
    }
    i += 1;
    while i < le && b[i] != b' ' && b[i] != b'\t' {
        if !matches!(b[i], b'+' | b'-' | b'0'..=b'9') {
            return false;
        }
        i += 1;
    }
    while i < le && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    i >= le || b[i] == b'#'
}

/// Scan the value range `[start, end)` for the inline roles: quoted strings, a
/// trailing `# comment`, and numeric / boolean / null constants. Plain scalars,
/// anchors, aliases, tags and structure punctuation are left as default ink.
fn scan_value(b: &[u8], start: usize, end: usize, out: &mut Vec<(Range<usize>, SynKind)>) {
    let mut i = start;
    while i < end {
        let c = b[i];
        if c == b' ' || c == b'\t' {
            i += 1;
            continue;
        }
        // `#` at a token boundary (we just skipped whitespace) is a comment.
        if c == b'#' {
            out.push((i..end, SynKind::Comment));
            return;
        }
        if c == b'"' || c == b'\'' {
            let q = scan_qstring(b, i, end, c);
            out.push((i..q, SynKind::Str));
            i = q;
            continue;
        }
        // Flow-collection punctuation — skip and scan the items inside.
        if matches!(c, b'[' | b']' | b'{' | b'}' | b',') {
            i += 1;
            continue;
        }
        // A plain token, bounded by whitespace / flow punctuation.
        let ts = i;
        while i < end && !matches!(b[i], b' ' | b'\t' | b',' | b'[' | b']' | b'{' | b'}') {
            i += 1;
        }
        let tok = &b[ts..i];
        if is_number(tok) || is_const_word(tok) {
            out.push((ts..i, SynKind::Constant));
        }
    }
}

/// Scan a quoted scalar from its opening `quote` at `q` to just past its close (or
/// `end`). Double quotes honor `\` escapes; single quotes use the doubled-quote
/// `''` escape (no backslashes).
fn scan_qstring(b: &[u8], q: usize, end: usize, quote: u8) -> usize {
    let mut i = q + 1;
    if quote == b'\'' {
        while i < end {
            if b[i] == b'\'' {
                if i + 1 < end && b[i + 1] == b'\'' {
                    i += 2; // an escaped quote `''`, not the close
                    continue;
                }
                return i + 1;
            }
            i += 1;
        }
        return end;
    }
    while i < end {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    end
}

fn is_const_word(tok: &[u8]) -> bool {
    CONST_WORDS.iter().any(|w| w.as_bytes() == tok)
}

/// Case-insensitive byte-slice equality.
fn eq_ic(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eq_ignore_ascii_case(y))
}

/// Is the whole plain token a numeric scalar? Accepts an optional sign, the
/// `0x`/`0o`/`0b` radixes, decimals/floats with an exponent, and the special
/// `.inf` / `.nan` floats. Requires the ENTIRE token to be numeric, so a date like
/// `2024-01-01` or a version `1.2.3` is left untouched.
fn is_number(s: &[u8]) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut i = 0;
    if s[0] == b'+' || s[0] == b'-' {
        i += 1;
    }
    let rest = &s[i..];
    if rest.is_empty() {
        return false;
    }
    if eq_ic(rest, b".inf") || eq_ic(rest, b".nan") {
        return true;
    }
    // Radix-prefixed integers.
    if rest.len() > 2 && rest[0] == b'0' {
        match rest[1] | 0x20 {
            b'x' => return rest[2..].iter().all(u8::is_ascii_hexdigit),
            b'o' => return rest[2..].iter().all(|c| matches!(c, b'0'..=b'7')),
            b'b' => return rest[2..].iter().all(|c| matches!(c, b'0' | b'1')),
            _ => {}
        }
    }
    // Decimal / float, with at most one `.` and an optional `e`/`E` exponent.
    let mut seen_digit = false;
    let mut seen_dot = false;
    let mut k = 0;
    while k < rest.len() {
        let c = rest[k];
        if c.is_ascii_digit() {
            seen_digit = true;
            k += 1;
        } else if c == b'.' && !seen_dot {
            seen_dot = true;
            k += 1;
        } else if (c == b'e' || c == b'E') && seen_digit {
            k += 1;
            if k < rest.len() && (rest[k] == b'+' || rest[k] == b'-') {
                k += 1;
            }
            let mut exp_digit = false;
            while k < rest.len() && rest[k].is_ascii_digit() {
                exp_digit = true;
                k += 1;
            }
            return exp_digit && k == rest.len();
        } else {
            return false;
        }
    }
    seen_digit
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::testutil::{at, has};

    #[test]
    fn line_and_inline_comments() {
        let t = "# header\nname: awl  # trailing\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["# header", "# trailing"], "{s:?}");
    }

    #[test]
    fn hash_inside_plain_scalar_is_not_a_comment() {
        // No space before `#`, so it stays part of the plain scalar (default ink).
        let t = "url: http://example.com#frag\n";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Comment).is_empty(), "{s:?}");
    }

    #[test]
    fn double_and_single_quoted_strings() {
        let t = "a: \"hi there\"\nb: 'it''s ok'\n";
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        assert!(ss.contains(&"\"hi there\""), "{ss:?}");
        // The doubled `''` is an escape, so the whole `'it''s ok'` is one span.
        assert!(ss.contains(&"'it''s ok'"), "{ss:?}");
    }

    #[test]
    fn numbers_booleans_and_null() {
        let t = "i: 42\nf: 3.14\nh: 0xFF\nn: -1.5e3\nb: true\nc: off\nz: null\nw: ~\n";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["42", "3.14", "0xFF", "-1.5e3", "true", "off", "null", "~"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn dates_and_versions_are_not_numbers() {
        // A whole-token numeric requirement keeps timestamps / versions plain.
        let t = "when: 2024-01-01\nver: 1.2.3\n";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Constant).is_empty(), "{s:?}");
    }

    #[test]
    fn mapping_key_is_the_definition() {
        let t = "database:\n  host: localhost\n  port: 5432\n";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"database"), "{ds:?}");
        assert!(ds.contains(&"host"), "{ds:?}");
        assert!(ds.contains(&"port"), "{ds:?}");
        // `5432` rides as a Constant, not a Definition.
        assert!(at(t, &s, SynKind::Constant).contains(&"5432"), "{s:?}");
    }

    #[test]
    fn key_under_sequence_marker() {
        let t = "items:\n  - name: alpha\n  - name: beta\n";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        // Both the `items` key and the per-item `name` keys are Definitions.
        assert_eq!(ds, vec!["items", "name", "name"], "{s:?}");
    }

    #[test]
    fn plain_scalar_value_is_not_highlighted() {
        // A bare value rides the default ink — only its KEY is styled.
        let t = "mode: production\n";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Constant).is_empty(), "{s:?}");
        assert!(at(t, &s, SynKind::Str).is_empty(), "{s:?}");
        assert_eq!(at(t, &s, SynKind::Definition), vec!["mode"], "{s:?}");
    }

    #[test]
    fn block_scalar_body_is_a_string() {
        let t = "script: |\n  echo \"hi\"\n  run 99\nnext: 1\n";
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        // Each deeper line of the block is one literal-string span.
        assert!(ss.contains(&"echo \"hi\""), "{ss:?}");
        assert!(ss.contains(&"run 99"), "{ss:?}");
        // The dedented `next:` line resumes normal scanning.
        assert!(at(t, &s, SynKind::Definition).contains(&"next"), "{s:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"1"), "{s:?}");
    }

    #[test]
    fn flow_collection_constants() {
        let t = "nums: [1, 2, 3]\nflags: {a: true, b: false}\n";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["1", "2", "3", "true", "false"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn document_marker_is_plain() {
        let t = "---\nkey: 1\n";
        let s = spans(t);
        // `---` is not a key/sequence/value role.
        assert!(!has(&s, 0, 3, SynKind::Definition), "{s:?}");
        assert_eq!(at(t, &s, SynKind::Definition), vec!["key"], "{s:?}");
    }

    #[test]
    fn empty_doc_has_no_spans() {
        assert!(spans("").is_empty());
        assert!(spans("\n\n").is_empty());
    }

    #[test]
    fn reference_snippet() {
        // A compact end-to-end snippet asserting all four roles at once.
        let t = "# app config\nname: \"awl\"\nversion: 1.4\nenabled: true\npath: /usr/local  # note\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["# app config", "# note"], "{s:?}");
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"name") && ds.contains(&"version"), "{ds:?}");
        assert!(at(t, &s, SynKind::Str).contains(&"\"awl\""), "{s:?}");
        let cs = at(t, &s, SynKind::Constant);
        assert!(cs.contains(&"1.4") && cs.contains(&"true"), "{cs:?}");
    }
}
