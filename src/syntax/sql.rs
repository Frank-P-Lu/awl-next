//! SQL syntax lexer — a minimal hand-written byte scanner mirroring the
//! reference lexers in `rust.rs` / `python.rs`. It emits only the four Alabaster
//! roles and leaves everything else (keywords, operators, identifiers,
//! punctuation) on the default ink:
//!
//! - [`SynKind::Comment`]    — `-- line` and `/* block */` comments.
//! - [`SynKind::Str`]        — `'...'` string literals (with the SQL `''`
//!   doubled-quote escape) and Postgres `$tag$...$tag$` dollar-quoted strings.
//! - [`SynKind::Constant`]   — numeric literals and the `NULL` / `TRUE` / `FALSE`
//!   keywords (SQL keywords are case-insensitive, so matching is too).
//! - [`SynKind::Definition`] — the identifier named by a `CREATE … TABLE` / `VIEW`
//!   / `INDEX` / `FUNCTION` / `PROCEDURE` / `TRIGGER` / `TYPE` / … introducer
//!   (best-effort; the `IF NOT EXISTS` / `CONCURRENTLY` modifiers are skipped).
//!
//! Double-quoted (`"…"`) and backtick (`` `…` ``) tokens are DELIMITED
//! IDENTIFIERS in SQL, not strings, so they get NO span — but the scanner still
//! consumes them whole so an apostrophe inside (`"it's"`) cannot open a stray
//! string. Span boundaries land on ASCII bytes; multibyte UTF-8 inside a
//! string/comment rides inside the span. Pure + single-pass; see the tests below.

use super::SynKind;
use std::ops::Range;

/// Object-type introducers after which the next identifier is the DEFINITION name
/// (matched case-insensitively, like all SQL keywords).
const DEF_KEYWORDS: &[&str] = &[
    "table",
    "view",
    "index",
    "function",
    "procedure",
    "trigger",
    "database",
    "schema",
    "sequence",
    "type",
    "role",
    "user",
];

/// Words that may sit BETWEEN a def introducer and the name (`CREATE TABLE IF NOT
/// EXISTS t`, `CREATE INDEX CONCURRENTLY i`); skip them without clearing the
/// expectation so the real name still lands as the [`SynKind::Definition`].
const DEF_SKIP_WORDS: &[&str] = &["if", "not", "exists", "concurrently"];

/// Identifiers that are CONSTANT literals (booleans + the `NULL` nil-style value).
const CONST_WORDS: &[&str] = &["null", "true", "false"];

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic()
}
fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
}
/// Case-insensitive membership test against one of the keyword tables.
fn contains_ci(table: &[&str], word: &str) -> bool {
    table.iter().any(|k| word.eq_ignore_ascii_case(k))
}

pub fn spans(text: &str) -> Vec<(Range<usize>, SynKind)> {
    let b = text.as_bytes();
    let n = b.len();
    let mut out: Vec<(Range<usize>, SynKind)> = Vec::new();
    let mut i = 0usize;
    // Set when the previous significant token was a DEF_KEYWORD; the next
    // identifier (past any skip-words) is then the defined NAME.
    let mut expect_def = false;

    while i < n {
        let c = b[i];

        // --- line comment (`-- …`) ---
        if c == b'-' && i + 1 < n && b[i + 1] == b'-' {
            let start = i;
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            out.push((start..i, SynKind::Comment));
            continue;
        }

        // --- block comment (`/* … */`; standard SQL blocks do NOT nest) ---
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

        // --- single-quoted string (`''` is an escaped quote) ---
        if c == b'\'' {
            let start = i;
            i = scan_quoted(b, i, b'\'');
            out.push((start..i, SynKind::Str));
            expect_def = false;
            continue;
        }

        // --- dollar-quoted string (Postgres `$tag$ … $tag$`) ---
        if c == b'$' {
            if let Some(end) = dollar_quote(b, i) {
                out.push((i..end, SynKind::Str));
                i = end;
                expect_def = false;
                continue;
            }
            // Not a dollar-quote (e.g. a `$1` parameter): plain punctuation.
            i += 1;
            continue;
        }

        // --- delimited identifier (`"…"` or `` `…` ``): NOT a string, no span,
        //     but consume it whole so an inner apostrophe can't open a string ---
        if c == b'"' || c == b'`' {
            let close = c;
            i += 1;
            while i < n {
                if b[i] == close {
                    // A doubled delimiter (`""`) is an escaped one — stay inside.
                    if i + 1 < n && b[i + 1] == close {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            // A quoted identifier IS a name, so honor a pending definition.
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
                if contains_ci(DEF_SKIP_WORDS, word) {
                    // A modifier between the introducer and the name — keep waiting.
                } else {
                    out.push((start..i, SynKind::Definition));
                    expect_def = false;
                }
            } else if contains_ci(CONST_WORDS, word) {
                out.push((start..i, SynKind::Constant));
            } else if contains_ci(DEF_KEYWORDS, word) {
                expect_def = true;
            }
            continue;
        }

        // Any other byte (operator, punctuation, whitespace) stays default ink.
        // A non-identifier, non-whitespace token while awaiting a name means it
        // never materialized — drop the expectation.
        if !c.is_ascii_whitespace() {
            expect_def = false;
        }
        i += 1;
    }

    out
}

/// Scan a quoted run from the opening `quote` at `i` to just past its close (or
/// EOF). A doubled quote (`''` / `""`) is an escape and does NOT close the run.
fn scan_quoted(b: &[u8], i: usize, quote: u8) -> usize {
    let n = b.len();
    let mut j = i + 1;
    while j < n {
        if b[j] == quote {
            if j + 1 < n && b[j + 1] == quote {
                j += 2; // escaped quote
                continue;
            }
            return j + 1;
        }
        j += 1;
    }
    n
}

/// If a Postgres dollar-quoted string starts at `i` (`$$…$$` or `$tag$…$tag$`,
/// where `tag` is an identifier), return the byte index just past its close; else
/// `None`. The tag must be terminated by a `$`, which rules out `$1` parameters.
fn dollar_quote(b: &[u8], i: usize) -> Option<usize> {
    let n = b.len();
    debug_assert_eq!(b[i], b'$');
    let mut j = i + 1;
    // The optional tag: ident-shaped, no leading digit.
    if j < n && b[j].is_ascii_digit() {
        return None;
    }
    while j < n && is_ident_continue(b[j]) {
        j += 1;
    }
    if j >= n || b[j] != b'$' {
        return None;
    }
    let tag = &b[i..=j]; // the full `$tag$` delimiter, incl. both `$`s
    let mut k = j + 1;
    while k < n {
        if b[k] == b'$' && k + tag.len() <= n && &b[k..k + tag.len()] == tag {
            return Some(k + tag.len());
        }
        k += 1;
    }
    Some(n) // unterminated: run to EOF
}

/// Scan a numeric literal beginning at the digit `i`; returns the index just past
/// it. Accepts a fractional `.`, an `e`/`E` exponent (with optional sign), and a
/// leading `0x` hex form. A standalone `.` not followed by a digit stops the scan.
fn scan_number(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i + 1;
    // Hex form (`0x1F`) — some dialects.
    if b[i] == b'0' && j < n && matches!(b[j], b'x' | b'X') {
        j += 1;
        while j < n && b[j].is_ascii_hexdigit() {
            j += 1;
        }
        return j;
    }
    while j < n {
        let c = b[j];
        if c.is_ascii_digit() {
            j += 1;
        } else if c == b'.' && j + 1 < n && b[j + 1].is_ascii_digit() {
            j += 1;
        } else if (c == b'e' || c == b'E')
            && j + 1 < n
            && (b[j + 1].is_ascii_digit()
                || ((b[j + 1] == b'+' || b[j + 1] == b'-')
                    && j + 2 < n
                    && b[j + 2].is_ascii_digit()))
        {
            // Exponent: consume `e`, the optional sign, and the digits below.
            j += if b[j + 1] == b'+' || b[j + 1] == b'-' { 2 } else { 1 };
        } else {
            break;
        }
    }
    j
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at<'a>(text: &'a str, s: &[(Range<usize>, SynKind)], k: SynKind) -> Vec<&'a str> {
        s.iter().filter(|(_, kk)| *kk == k).map(|(r, _)| &text[r.clone()]).collect()
    }
    fn has(s: &[(Range<usize>, SynKind)], lo: usize, hi: usize, k: SynKind) -> bool {
        s.iter().any(|(r, kk)| r.start == lo && r.end == hi && *kk == k)
    }

    #[test]
    fn line_comment() {
        let t = "SELECT 1; -- pick one\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["-- pick one"], "{s:?}");
    }

    #[test]
    fn block_comment() {
        let t = "/* a\n   b */ SELECT 1;";
        let s = spans(t);
        assert!(has(&s, 0, 12, SynKind::Comment), "{s:?}");
    }

    #[test]
    fn string_with_doubled_quote_escape() {
        // `''` is SQL's escaped single quote — it must NOT close the string.
        let t = "SELECT 'it''s fine';";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec!["'it''s fine'"], "{s:?}");
    }

    #[test]
    fn dollar_quoted_string() {
        let t = "SELECT $tag$he said 'hi'$tag$;";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec!["$tag$he said 'hi'$tag$"], "{s:?}");
    }

    #[test]
    fn double_quoted_is_identifier_not_string() {
        // A delimited identifier is NOT a string; the inner apostrophe is inert.
        let t = "SELECT \"it's a col\" FROM t;";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Str).is_empty(), "{s:?}");
    }

    #[test]
    fn numbers_and_constants() {
        let t = "SELECT 42, 3.14, 1e3, NULL, TRUE, false;";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["42", "3.14", "1e3", "NULL", "TRUE", "false"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn definition_after_create_table() {
        let t = "CREATE TABLE users (id INT);";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Definition).contains(&"users"), "{s:?}");
    }

    #[test]
    fn definition_skips_if_not_exists() {
        let t = "CREATE TABLE IF NOT EXISTS widgets (id INT);";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"widgets"), "{ds:?}");
        // The skipped modifier words are NOT marked as the definition.
        assert!(!ds.contains(&"IF"), "{ds:?}");
    }

    #[test]
    fn definition_after_view_and_function() {
        let t = "CREATE OR REPLACE VIEW active AS SELECT 1;\nCREATE FUNCTION foo() RETURNS INT;";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"active"), "{ds:?}");
        assert!(ds.contains(&"foo"), "{ds:?}");
    }

    #[test]
    fn keyword_itself_is_not_styled() {
        // SELECT / FROM / CREATE / TABLE all stay default ink; only the NAME styles.
        let t = "CREATE TABLE t (id INT);";
        let s = spans(t);
        // `CREATE` (0..6) and `TABLE` (7..12) get no span.
        assert!(!has(&s, 0, 6, SynKind::Definition), "{s:?}");
        assert!(!has(&s, 7, 12, SynKind::Definition), "{s:?}");
        assert!(at(t, &s, SynKind::Comment).is_empty(), "{s:?}");
    }

    #[test]
    fn plain_query_has_no_spans() {
        let t = "SELECT a, b FROM t WHERE a = b;";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn reference_snippet() {
        let t = "-- seed\nCREATE TABLE users (\n  id INT,\n  name TEXT DEFAULT 'anon'\n);\nINSERT INTO users VALUES (1, NULL); /* done */\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["-- seed", "/* done */"], "{s:?}");
        assert!(at(t, &s, SynKind::Definition).contains(&"users"), "{s:?}");
        assert!(at(t, &s, SynKind::Str).contains(&"'anon'"), "{s:?}");
        let cs = at(t, &s, SynKind::Constant);
        assert!(cs.contains(&"1") && cs.contains(&"NULL"), "{cs:?}");
    }
}
