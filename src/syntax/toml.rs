//! TOML syntax lexer — a hand-written byte scanner emitting only the four
//! Alabaster roles (everything else — `=`, brackets, separators — rides the
//! default ink). Mirrors the reference lexers in [`crate::syntax::rust`] and
//! [`crate::syntax::python`]:
//!
//! - [`SynKind::Comment`]    — `# line` comments (TOML's only comment form).
//! - [`SynKind::Str`]        — basic `"..."`, literal `'...'`, and the multiline
//!   `"""..."""` / `'''...'''` variants (basic strings honor `\` escapes;
//!   literal strings do not).
//! - [`SynKind::Constant`]   — integers (incl. `0x`/`0o`/`0b`, `_` separators,
//!   signs), floats (`e` exponents, `inf`/`nan`), booleans, and date-times.
//! - [`SynKind::Definition`] — the KEY being defined. TOML has no `fn`/`class`
//!   introducer, so the definition is keyword-free and POSITIONAL: the bare /
//!   quoted / dotted key to the LEFT of `=`, and every name in a `[table]` or
//!   `[[array.of.tables]]` header (incl. inline-table keys). The VALUE side and
//!   the `=` itself stay the default ink.
//!
//! Detection is line/position aware: a small `mode` (key vs value) plus a bracket
//! stack (so an inline-table `{ k = v }` re-enters key context while an array
//! `[ … ]` stays in value context across newlines). Span boundaries land on ASCII
//! bytes; multibyte UTF-8 inside a string/comment rides inside the span. Pure +
//! single-pass. See the tests below for the exact contract on a sample snippet.

use super::SynKind;
use std::ops::Range;

/// Value-side bare words that are CONSTANT literals (booleans + the special
/// floats). TOML has no `null`/`nil`.
const CONST_WORDS: &[&str] = &["true", "false", "inf", "nan"];

/// Whether we are scanning a KEY (left of `=`, or a table-header name) or a VALUE.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Key,
    Value,
}

/// An open bracket on the value-side stack. An array stays in value context (and
/// may span newlines); an inline table re-enters key context for its members.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bracket {
    Array,
    Table,
}

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic()
}
fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
}
/// A character allowed in a TOML bare key (`A-Z a-z 0-9 _ -`).
fn is_bare_key(c: u8) -> bool {
    c == b'_' || c == b'-' || c.is_ascii_alphanumeric()
}

pub fn spans(text: &str) -> Vec<(Range<usize>, SynKind)> {
    let b = text.as_bytes();
    let n = b.len();
    let mut out: Vec<(Range<usize>, SynKind)> = Vec::new();
    let mut i = 0usize;
    let mut mode = Mode::Key;
    let mut stack: Vec<Bracket> = Vec::new();

    while i < n {
        let c = b[i];

        // --- newline: a fresh line returns to key context (unless we are still
        // inside a multi-line array value). ---
        if c == b'\n' {
            if stack.is_empty() {
                mode = Mode::Key;
            }
            i += 1;
            continue;
        }
        if c == b' ' || c == b'\t' || c == b'\r' {
            i += 1;
            continue;
        }

        // --- comment (anywhere outside a string) ---
        if c == b'#' {
            let start = i;
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            out.push((start..i, SynKind::Comment));
            continue;
        }

        match mode {
            Mode::Key => {
                // A line-leading `[name]` / `[[name]]` table header (only at the
                // top level — never inside an inline table).
                if stack.is_empty() && c == b'[' {
                    i += 1; // past `[`
                    let double = i < n && b[i] == b'[';
                    if double {
                        i += 1;
                    }
                    while i < n && b[i] != b']' && b[i] != b'\n' {
                        let d = b[i];
                        if d == b'"' || d == b'\'' {
                            let end = scan_string(b, i);
                            out.push((i..end, SynKind::Definition));
                            i = end;
                        } else if is_bare_key(d) {
                            let s = i;
                            while i < n && is_bare_key(b[i]) {
                                i += 1;
                            }
                            out.push((s..i, SynKind::Definition));
                        } else {
                            // dots, whitespace inside the header
                            i += 1;
                        }
                    }
                    if i < n && b[i] == b']' {
                        i += 1;
                    }
                    if double && i < n && b[i] == b']' {
                        i += 1;
                    }
                    continue;
                }

                // A quoted key (`"key" = …` / `'key' = …`) — single-line only.
                if c == b'"' || c == b'\'' {
                    let end = scan_string(b, i);
                    out.push((i..end, SynKind::Definition));
                    i = end;
                    continue;
                }

                // The key/value separator switches us to value context.
                if c == b'=' {
                    mode = Mode::Value;
                    i += 1;
                    continue;
                }

                // A dotted-key separator — skip it; each segment is its own name.
                if c == b'.' {
                    i += 1;
                    continue;
                }

                // A bare key (may begin with a digit, e.g. `1234 = true`).
                if is_bare_key(c) {
                    let start = i;
                    while i < n && is_bare_key(b[i]) {
                        i += 1;
                    }
                    out.push((start..i, SynKind::Definition));
                    continue;
                }

                i += 1;
            }
            Mode::Value => {
                // Strings.
                if c == b'"' || c == b'\'' {
                    let end = scan_string(b, i);
                    out.push((i..end, SynKind::Str));
                    i = end;
                    continue;
                }

                // Brackets: arrays stay value-context; inline tables re-enter key.
                if c == b'[' {
                    stack.push(Bracket::Array);
                    i += 1;
                    continue;
                }
                if c == b']' {
                    if stack.last() == Some(&Bracket::Array) {
                        stack.pop();
                    }
                    i += 1;
                    continue;
                }
                if c == b'{' {
                    stack.push(Bracket::Table);
                    mode = Mode::Key;
                    i += 1;
                    continue;
                }
                if c == b'}' {
                    if stack.last() == Some(&Bracket::Table) {
                        stack.pop();
                    }
                    mode = Mode::Value;
                    i += 1;
                    continue;
                }
                if c == b',' {
                    // The next entry of an inline table is another key.
                    if stack.last() == Some(&Bracket::Table) {
                        mode = Mode::Key;
                    }
                    i += 1;
                    continue;
                }

                // Numbers, signed numbers, and date-times.
                let signed_num = (c == b'+' || c == b'-')
                    && i + 1 < n
                    && (b[i + 1].is_ascii_digit()
                        || word_at(b, i + 1, b"inf")
                        || word_at(b, i + 1, b"nan"));
                if c.is_ascii_digit() || signed_num {
                    let start = i;
                    i = scan_constant(b, i);
                    out.push((start..i, SynKind::Constant));
                    continue;
                }

                // Bare value words: only the booleans / special floats are styled.
                if is_ident_start(c) {
                    let start = i;
                    i += 1;
                    while i < n && is_ident_continue(b[i]) {
                        i += 1;
                    }
                    let word = &text[start..i];
                    if CONST_WORDS.contains(&word) {
                        out.push((start..i, SynKind::Constant));
                    }
                    continue;
                }

                i += 1;
            }
        }
    }

    out
}

/// Scan a string literal whose opening quote is at `i` — basic (`"`) or literal
/// (`'`), single- or triple-quoted — and return the index just past its close (or
/// EOF if unterminated). Basic strings honor `\` escapes; literal strings do not.
/// A single-line string never crosses a newline.
fn scan_string(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let q = b[i];
    let basic = q == b'"';
    let triple = i + 2 < n && b[i + 1] == q && b[i + 2] == q;
    if triple {
        let mut j = i + 3;
        while j < n {
            if basic && b[j] == b'\\' {
                j += 2;
                continue;
            }
            if b[j] == q && j + 2 < n && b[j + 1] == q && b[j + 2] == q {
                return j + 3;
            }
            j += 1;
        }
        n
    } else {
        let mut j = i + 1;
        while j < n {
            if basic && b[j] == b'\\' {
                j += 2;
                continue;
            }
            match b[j] {
                b'\n' => return j, // unterminated single-line string
                c if c == q => return j + 1,
                _ => j += 1,
            }
        }
        n
    }
}

/// Scan a numeric / date-time constant beginning at `i` (optionally signed); return
/// the index just past it. Handles `0x`/`0o`/`0b` radixes, `_` separators, float
/// exponents, `inf`/`nan`, and the date-time forms (incl. the single space between
/// a date and a time in `1979-05-27 07:32:00`).
fn scan_constant(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i;
    if j < n && (b[j] == b'+' || b[j] == b'-') {
        j += 1;
    }
    if word_at(b, j, b"inf") || word_at(b, j, b"nan") {
        return j + 3;
    }
    // Radix-prefixed integers.
    if j + 1 < n && b[j] == b'0' && matches!(b[j + 1], b'x' | b'X' | b'o' | b'O' | b'b' | b'B') {
        j += 2;
        while j < n && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
            j += 1;
        }
        return j;
    }
    let is_date = looks_like_date(b, j);
    let mut had_space = false;
    while j < n {
        let c = b[j];
        if c.is_ascii_alphanumeric()
            || c == b'_'
            || c == b'.'
            || c == b':'
            || c == b'-'
            || c == b'+'
        {
            j += 1;
        } else if c == b' ' && is_date && !had_space && j + 1 < n && b[j + 1].is_ascii_digit() {
            // The single space joining a date and a time in an RFC-3339 date-time.
            had_space = true;
            j += 1;
        } else {
            break;
        }
    }
    j
}

/// Whether the bytes at `i` are exactly `w` followed by a non-identifier boundary
/// (so `inf`/`nan` match but `info`/`nanos` do not).
fn word_at(b: &[u8], i: usize, w: &[u8]) -> bool {
    if i + w.len() > b.len() || &b[i..i + w.len()] != w {
        return false;
    }
    let after = i + w.len();
    !(after < b.len() && is_ident_continue(b[after]))
}

/// Whether the bytes at `i` begin a `YYYY-MM-DD` calendar date.
fn looks_like_date(b: &[u8], i: usize) -> bool {
    if i + 10 > b.len() {
        return false;
    }
    let d = |k: usize| b[i + k].is_ascii_digit();
    d(0) && d(1) && d(2) && d(3)
        && b[i + 4] == b'-'
        && d(5) && d(6)
        && b[i + 7] == b'-'
        && d(8) && d(9)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::testutil::{at, has};

    #[test]
    fn line_comment() {
        let t = "# a heading comment\nkey = 1\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["# a heading comment"], "{s:?}");
    }

    #[test]
    fn trailing_comment_after_value() {
        let t = "port = 8080 # the http port\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["# the http port"], "{s:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"8080"), "{s:?}");
    }

    #[test]
    fn basic_and_literal_strings() {
        let t = "a = \"hi\\\"there\"\nb = 'raw\\nliteral'\n";
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        assert!(ss.contains(&"\"hi\\\"there\""), "{ss:?}");
        // A literal string does NOT honor escapes: the whole `'...'` is one Str.
        assert!(ss.contains(&"'raw\\nliteral'"), "{ss:?}");
    }

    #[test]
    fn multiline_string() {
        let t = "doc = \"\"\"line one\nline two\"\"\"\n";
        let s = spans(t);
        assert_eq!(
            at(t, &s, SynKind::Str),
            vec!["\"\"\"line one\nline two\"\"\""],
            "{s:?}"
        );
    }

    #[test]
    fn numbers_and_booleans() {
        let t = "i = 42\nh = 0xDEAD_beef\nf = 6.626e-34\nn = -3.14\nok = true\nno = false\nz = inf\n";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["42", "0xDEAD_beef", "6.626e-34", "-3.14", "true", "false", "inf"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn datetime_is_constant() {
        let t = "ts = 1979-05-27 07:32:00\nd = 1979-05-27\no = 1979-05-27T07:32:00Z\n";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        assert!(cs.contains(&"1979-05-27 07:32:00"), "{cs:?}");
        assert!(cs.contains(&"1979-05-27"), "{cs:?}");
        assert!(cs.contains(&"1979-05-27T07:32:00Z"), "{cs:?}");
    }

    #[test]
    fn bare_dotted_and_quoted_keys_are_definitions() {
        let t = "name = \"awl\"\nserver.host = \"localhost\"\n\"quoted key\" = 1\n";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"name"), "{ds:?}");
        assert!(ds.contains(&"server") && ds.contains(&"host"), "dotted: {ds:?}");
        assert!(ds.contains(&"\"quoted key\""), "{ds:?}");
        // The string VALUE is a Str, not a Definition.
        assert!(at(t, &s, SynKind::Str).contains(&"\"awl\""), "{s:?}");
    }

    #[test]
    fn table_headers_are_definitions() {
        let t = "[server]\nhost = \"x\"\n[[products]]\nname = \"y\"\n[a.b.c]\n";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        for want in ["server", "products", "a", "b", "c"] {
            assert!(ds.contains(&want), "missing header name {want}: {ds:?}");
        }
        // The bracket punctuation rides the default ink.
        assert!(!has(&s, 0, 1, SynKind::Definition), "the `[` must stay plain: {s:?}");
    }

    #[test]
    fn inline_table_keys_and_array_values() {
        let t = "pt = { x = 1, y = 2 }\nlist = [ 1, 2, 3 ]\n";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        // `pt`, `list`, and the inline-table members `x` / `y`, are definitions.
        assert!(ds.contains(&"pt") && ds.contains(&"x") && ds.contains(&"y"), "{ds:?}");
        assert!(ds.contains(&"list"), "list is a key: {ds:?}");
        // Array elements are plain constants, not keys.
        let cs = at(t, &s, SynKind::Constant);
        for want in ["1", "2", "3"] {
            assert!(cs.contains(&want), "{cs:?}");
        }
    }

    #[test]
    fn multiline_array_stays_in_value_context() {
        // Newlines inside an array must NOT flip elements back into key context.
        let t = "ports = [\n  8001,\n  8002,\n]\n";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        assert_eq!(ds, vec!["ports"], "only the key is a definition: {ds:?}");
        let cs = at(t, &s, SynKind::Constant);
        assert!(cs.contains(&"8001") && cs.contains(&"8002"), "{cs:?}");
    }

    #[test]
    fn separator_and_value_words_stay_plain() {
        // The `=` separator is never styled, and a value `true` is a Constant
        // (not a Definition); the key alone is the Definition.
        let t = "enabled = true\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Definition), vec!["enabled"], "{s:?}");
        assert_eq!(at(t, &s, SynKind::Constant), vec!["true"], "{s:?}");
        // Nothing covers the `=` at byte 8.
        assert!(!s.iter().any(|(r, _)| r.contains(&8)), "`=` must stay plain: {s:?}");
    }

    #[test]
    fn reference_snippet() {
        let t = "# config\ntitle = \"awl\"\n[server]\nport = 8080 # default\nhosts = [ \"a\", \"b\" ]\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["# config", "# default"], "{s:?}");
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"title") && ds.contains(&"server") && ds.contains(&"port"), "{ds:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"8080"), "{s:?}");
        let ss = at(t, &s, SynKind::Str);
        assert!(ss.contains(&"\"awl\"") && ss.contains(&"\"a\"") && ss.contains(&"\"b\""), "{ss:?}");
    }
}
