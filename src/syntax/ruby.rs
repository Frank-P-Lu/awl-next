//! Ruby syntax lexer — a minimal hand-written byte scanner following the
//! reference lexers in [`crate::syntax::rust`] and [`crate::syntax::python`]. It
//! recognizes only what the four Alabaster roles need and leaves everything else
//! (keywords, operators, identifiers, punctuation, symbols, `@ivars`, `$globals`)
//! as the default ink:
//!
//! - [`SynKind::Comment`]    — `# line` comments and `=begin` / `=end` block
//!   comments (the block markers must sit in column 0, as Ruby requires).
//! - [`SynKind::Str`]        — `"..."` / `'...'` / `` `...` `` strings (an
//!   interpolated `"#{...}"` is ONE span), `?c` character literals, `%w[..]`-style
//!   percent literals, and `<<~`/`<<-`/`<<"TAG"` here-documents.
//! - [`SynKind::Constant`]   — numeric literals (radix-prefixed, `_` separators,
//!   floats, exponents, `r`/`i` suffixes) and `true` / `false` / `nil`.
//! - [`SynKind::Definition`] — the name after a `def` (skipping a `self.`/`Recv.`
//!   receiver, keeping a `?`/`!`/`=` method suffix) or a `class` / `module`.
//!
//! Span boundaries land on ASCII bytes (quotes, `#`, digits, ASCII identifiers),
//! so multibyte UTF-8 inside a string/comment rides inside the span without ever
//! splitting a char. Pure + single-pass; see the tests below for the contract.

use super::SynKind;
use std::ops::Range;

/// Identifiers that are CONSTANT literals (booleans + the `nil` nil-style value).
const CONST_WORDS: &[&str] = &["true", "false", "nil"];

use super::{is_ident_continue, is_ident_start};
/// True if `c` ends a *value* — so a following `?`/`%` is an operator (ternary /
/// modulo), not the start of a `?c` char literal or a `%w[..]` percent literal.
fn is_value_prev(c: u8) -> bool {
    is_ident_continue(c) || matches!(c, b')' | b']' | b'}')
}

/// What the previously-scanned definition introducer was expecting next.
#[derive(Clone, Copy, PartialEq)]
enum Expect {
    /// Nothing pending.
    No,
    /// After `def`: the next identifier is the method name (skip a `Recv.`
    /// receiver, keep a trailing `?`/`!`/`=`).
    Def,
    /// After `class` / `module`: the next identifier is the type name.
    Name,
}

pub fn spans(text: &str) -> Vec<(Range<usize>, SynKind)> {
    let b = text.as_bytes();
    let n = b.len();
    let mut out: Vec<(Range<usize>, SynKind)> = Vec::new();
    let mut i = 0usize;
    let mut expect = Expect::No;
    // The last non-whitespace byte scanned, used to disambiguate `?`/`%`.
    let mut prev = 0u8;
    // A here-document tag awaiting its body on the next line.
    let mut pending: Option<Vec<u8>> = None;

    while i < n {
        let c = b[i];

        // --- here-document body (consumed when its opener's line ends) ---
        if c == b'\n' {
            if let Some(tag) = pending.take() {
                let body_start = i + 1;
                let end = scan_heredoc_body(b, body_start, &tag);
                if body_start < end.0 {
                    out.push((body_start..end.0, SynKind::Str));
                }
                i = end.1;
                continue;
            }
            i += 1;
            continue;
        }

        // --- block comment: `=begin` ... `=end`, both flush in column 0 ---
        if c == b'=' && (i == 0 || b[i - 1] == b'\n') && starts_with(b, i, b"=begin") {
            let start = i;
            i = scan_block_comment(b, i);
            out.push((start..i, SynKind::Comment));
            continue;
        }

        // --- line comment ---
        if c == b'#' {
            let start = i;
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            out.push((start..i, SynKind::Comment));
            continue;
        }

        // --- here-document opener: `<<~TAG` / `<<-TAG` / `<<"TAG"` ---
        if c == b'<' && i + 1 < n && b[i + 1] == b'<' {
            if let Some((tag, after)) = heredoc_opener(b, i) {
                if pending.is_none() {
                    pending = Some(tag);
                }
                i = after;
                prev = b[i - 1];
                continue;
            }
        }

        // --- percent literal: `%w[..]`, `%q{..}`, `%(..)`, … ---
        if c == b'%' {
            if let Some(end) = percent_literal(b, i, prev) {
                out.push((i..end, SynKind::Str));
                i = end;
                expect = Expect::No;
                prev = b[end - 1];
                continue;
            }
        }

        // --- string / command literal ---
        if c == b'"' || c == b'\'' || c == b'`' {
            let end = scan_string(b, i);
            out.push((i..end, SynKind::Str));
            i = end;
            expect = Expect::No;
            prev = b[end - 1];
            continue;
        }

        // --- character literal: `?a`, `?\n`, `?\u{41}` ---
        if c == b'?' && !is_value_prev(prev) {
            if let Some(end) = char_literal(b, i) {
                out.push((i..end, SynKind::Str));
                i = end;
                expect = Expect::No;
                prev = b[end - 1];
                continue;
            }
        }

        // --- number literal ---
        if c.is_ascii_digit() {
            let start = i;
            i = scan_number(b, i);
            out.push((start..i, SynKind::Constant));
            expect = Expect::No;
            prev = b[i - 1];
            continue;
        }

        // --- identifier / keyword ---
        if is_ident_start(c) {
            let start = i;
            i += 1;
            while i < n && is_ident_continue(b[i]) {
                i += 1;
            }
            match expect {
                Expect::Def => {
                    // A `self.`/`Recv.` receiver precedes the real method name —
                    // skip it (and the dot) and keep expecting the name.
                    if i < n && b[i] == b'.' {
                        i += 1;
                        prev = b'.';
                        continue;
                    }
                    // Keep a Ruby method suffix: `name?`, `name!`, or setter `name=`
                    // (but not `==`/`=~`/`=>`).
                    let mut end = i;
                    if end < n
                        && (matches!(b[end], b'?' | b'!')
                            || (b[end] == b'='
                                && !(end + 1 < n && matches!(b[end + 1], b'=' | b'~' | b'>'))))
                    {
                        end += 1;
                    }
                    out.push((start..end, SynKind::Definition));
                    i = end;
                    expect = Expect::No;
                    prev = b[end - 1];
                    continue;
                }
                Expect::Name => {
                    out.push((start..i, SynKind::Definition));
                    expect = Expect::No;
                    prev = b[i - 1];
                    continue;
                }
                Expect::No => {
                    let word = &text[start..i];
                    if prev != b'.' && CONST_WORDS.contains(&word) {
                        out.push((start..i, SynKind::Constant));
                    } else if word == "def" {
                        expect = Expect::Def;
                    } else if word == "class" || word == "module" {
                        expect = Expect::Name;
                    }
                    prev = b[i - 1];
                    continue;
                }
            }
        }

        // Any other byte (operator, punctuation, whitespace) stays default ink. A
        // non-whitespace token after a def keyword means the name never showed up
        // (e.g. `def []` operator method) — drop the expectation.
        if !c.is_ascii_whitespace() {
            expect = Expect::No;
            prev = c;
        }
        i += 1;
    }

    out
}

/// True if `b[i..]` begins with `needle`.
fn starts_with(b: &[u8], i: usize, needle: &[u8]) -> bool {
    b.len() >= i + needle.len() && &b[i..i + needle.len()] == needle
}

/// Scan a `=begin` … `=end` block comment from `i` (at `=begin`) to just past the
/// `=end` line (or EOF if unterminated).
fn scan_block_comment(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i;
    loop {
        // Advance to the next line.
        while j < n && b[j] != b'\n' {
            j += 1;
        }
        if j >= n {
            return n;
        }
        j += 1; // past the newline -> column 0 of the next line
        if starts_with(b, j, b"=end") {
            // Consume the rest of the `=end` line.
            while j < n && b[j] != b'\n' {
                j += 1;
            }
            return j;
        }
        if j >= n {
            return n;
        }
    }
}

/// Scan a quoted string from the opening quote `q` to just past its matching
/// close (or EOF). Honors `\\` escapes; Ruby strings may span newlines, and an
/// interpolated `#{...}` rides inside the single span.
fn scan_string(b: &[u8], q: usize) -> usize {
    super::scan_quoted(b, q, b[q], false)
}

/// If a `?c` character literal starts at `i`, return the index just past it; else
/// `None` (a ternary `?`). Handles `?\n` escapes and `?\u{41}`.
fn char_literal(b: &[u8], i: usize) -> Option<usize> {
    let n = b.len();
    debug_assert_eq!(b[i], b'?');
    let j = i + 1;
    if j >= n {
        return None;
    }
    if b[j] == b'\\' {
        // Escape body: `\u{..}` runs to the brace, any other escape is one char.
        let mut k = j + 1;
        if k < n && b[k] == b'u' && k + 1 < n && b[k + 1] == b'{' {
            k += 2;
            while k < n && b[k] != b'}' {
                k += 1;
            }
            if k < n {
                k += 1;
            }
        } else if k < n {
            k += 1;
        }
        return Some(k);
    }
    // A single (possibly multibyte) char NOT followed by an identifier char is a
    // char literal; otherwise it is a ternary `?` / a `foo?` method tail.
    let ch_len = utf8_len(b[j]);
    let after = j + ch_len;
    if after >= n || !is_ident_continue(b[after]) {
        Some(after)
    } else {
        None
    }
}

/// Scan a numeric literal beginning at the digit `i`; returns the index just past
/// it. Accepts `0x`/`0o`/`0b`/`0d` radixes, `_` separators, a fractional `.`, an
/// `e`/`E` exponent with optional sign, and trailing `r`/`i` suffixes. A `..` range
/// after the integer is NOT consumed.
fn scan_number(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i + 1;
    if b[i] == b'0'
        && j < n
        && matches!(b[j], b'x' | b'X' | b'o' | b'O' | b'b' | b'B' | b'd' | b'D')
    {
        j += 1;
        while j < n && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
            j += 1;
        }
        return j;
    }
    while j < n {
        let c = b[j];
        if matches!(c, b'e' | b'E') && j + 1 < n && matches!(b[j + 1], b'+' | b'-') {
            // Exponent with an explicit sign.
            j += 2;
        } else if c.is_ascii_alphanumeric() || c == b'_' {
            j += 1;
        } else if c == b'.' {
            // A fractional point — but not the `..` range op, and not a method call
            // on an integer (`.` followed by a non-digit ident start).
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

/// Byte length of the UTF-8 char whose lead byte is `c`.
fn utf8_len(c: u8) -> usize {
    if c < 0x80 {
        1
    } else if c >> 5 == 0b110 {
        2
    } else if c >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

/// If a percent literal starts at `i` (`%w[..]`, `%q{..}`, `%(..)`, …), return the
/// index just past its close; else `None`. `prev` guards the bare bracket form
/// `%(..)` against the modulo operator (`a % (b)`).
fn percent_literal(b: &[u8], i: usize, prev: u8) -> Option<usize> {
    let n = b.len();
    let mut j = i + 1;
    if j >= n {
        return None;
    }
    let open;
    if b[j].is_ascii_alphabetic() {
        // Typed form: a known type letter then a delimiter byte.
        if !matches!(
            b[j],
            b'w' | b'W' | b'i' | b'I' | b'q' | b'Q' | b'r' | b's' | b'x'
        ) {
            return None;
        }
        if j + 1 >= n || !is_delim(b[j + 1]) {
            return None;
        }
        open = b[j + 1];
        j += 2;
    } else {
        // Bare form: only an opening bracket, and only where a value is expected
        // (otherwise it is the modulo operator).
        if is_value_prev(prev) || !matches!(b[j], b'(' | b'[' | b'{' | b'<') {
            return None;
        }
        open = b[j];
        j += 1;
    }
    let close = matched_close(open);
    let mut depth = 1u32;
    while j < n {
        let c = b[j];
        if c == b'\\' {
            j += 2;
            continue;
        }
        if open != close && c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(j + 1);
            }
        }
        j += 1;
    }
    Some(n) // unterminated: run to EOF
}

/// A valid percent-literal delimiter: any non-alphanumeric, non-whitespace byte.
fn is_delim(c: u8) -> bool {
    !c.is_ascii_alphanumeric() && !c.is_ascii_whitespace()
}

/// The closing delimiter for an opening one (brackets mirror; everything else is
/// its own close).
fn matched_close(open: u8) -> u8 {
    match open {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        b'<' => b'>',
        c => c,
    }
}

/// If a here-document opener starts at `i` (`<<~`, `<<-`, or `<<` immediately
/// followed by a quoted tag), return `(tag_bytes, index_past_opener)`; else
/// `None`. A bare `<<IDENT` is intentionally NOT treated as a heredoc — it is
/// indistinguishable from a left-shift (`A << B`).
fn heredoc_opener(b: &[u8], i: usize) -> Option<(Vec<u8>, usize)> {
    let n = b.len();
    let mut j = i + 2; // past `<<`
    if j < n && (b[j] == b'~' || b[j] == b'-') {
        j += 1;
    } else if !(j < n && matches!(b[j], b'"' | b'\'' | b'`')) {
        // No `~`/`-` and no quoted tag -> ambiguous with a left-shift; skip.
        return None;
    }
    if j >= n {
        return None;
    }
    // Quoted tag (`"END"`, `'END'`) or a bare identifier tag.
    if matches!(b[j], b'"' | b'\'' | b'`') {
        let quote = b[j];
        let start = j + 1;
        let mut k = start;
        while k < n && b[k] != quote {
            k += 1;
        }
        if k >= n || k == start {
            return None;
        }
        Some((b[start..k].to_vec(), k + 1))
    } else if is_ident_start(b[j]) {
        let start = j;
        while j < n && is_ident_continue(b[j]) {
            j += 1;
        }
        Some((b[start..j].to_vec(), j))
    } else {
        None
    }
}

/// Scan a here-document body that begins at `body_start` (the byte after the
/// opener's newline). Returns `(body_end, resume)`: `body_end` is where the Str
/// span stops (just before the terminator line), `resume` is where normal scanning
/// continues (just past the terminator line). The terminator is the tag alone on
/// its line (leading whitespace allowed, matching `<<~`/`<<-` leniency).
fn scan_heredoc_body(b: &[u8], body_start: usize, tag: &[u8]) -> (usize, usize) {
    let n = b.len();
    let mut p = body_start;
    while p < n {
        let line_start = p;
        let mut q = p;
        while q < n && b[q] != b'\n' {
            q += 1;
        }
        // Trim leading + trailing whitespace and compare to the tag.
        let mut a = line_start;
        while a < q && b[a].is_ascii_whitespace() {
            a += 1;
        }
        let mut z = q;
        while z > a && b[z - 1].is_ascii_whitespace() {
            z -= 1;
        }
        if &b[a..z] == tag {
            // Body ends before this terminator line; resume past its newline.
            let resume = if q < n { q + 1 } else { n };
            return (line_start, resume);
        }
        if q >= n {
            break; // unterminated: body runs to EOF
        }
        p = q + 1;
    }
    (n, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::testutil::{at, has};

    #[test]
    fn line_comment() {
        let t = "x = 1  # set x\n";
        assert_eq!(at(t, &spans(t), SynKind::Comment), vec!["# set x"]);
    }

    #[test]
    fn block_comment() {
        let t = "code\n=begin\na doc block\nmore\n=end\nafter\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["=begin\na doc block\nmore\n=end"], "{s:?}");
    }

    #[test]
    fn double_and_single_strings() {
        let t = "a = \"hi\"\nb = 'yo'\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec!["\"hi\"", "'yo'"], "{s:?}");
    }

    #[test]
    fn string_with_escape_and_interpolation() {
        let t = "s = \"a\\\"b #{x + 1} c\"";
        let s = spans(t);
        // The whole interpolated string is ONE Str span.
        assert_eq!(at(t, &s, SynKind::Str), vec!["\"a\\\"b #{x + 1} c\""], "{s:?}");
    }

    #[test]
    fn char_literal_not_ternary() {
        let t = "c = ?a\nd = cond ? x : y\n";
        let s = spans(t);
        // `?a` is a char literal; the ternary `?` is NOT.
        assert_eq!(at(t, &s, SynKind::Str), vec!["?a"], "{s:?}");
    }

    #[test]
    fn percent_word_array() {
        let t = "w = %w[foo bar baz]\nq = %q{hi there}\n";
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        assert!(ss.contains(&"%w[foo bar baz]"), "{ss:?}");
        assert!(ss.contains(&"%q{hi there}"), "{ss:?}");
    }

    #[test]
    fn percent_is_not_modulo() {
        // `a % b` and `count % 2` must NOT be read as percent literals.
        let t = "r = a % b\nz = count % 2\n";
        assert!(at(t, &spans(t), SynKind::Str).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn heredoc_squiggly() {
        let t = "sql = <<~SQL\n  SELECT *\n  FROM t\nSQL\nx = 1\n";
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        assert_eq!(ss, vec!["  SELECT *\n  FROM t\n"], "{ss:?}");
        // Scanning resumes after the terminator: the `1` is a Constant.
        assert!(at(t, &s, SynKind::Constant).contains(&"1"), "{s:?}");
    }

    #[test]
    fn left_shift_is_not_a_heredoc() {
        // `list << item` is an append, not a here-document.
        let t = "list << item\nx = 5\n";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Str).is_empty(), "{s:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"5"), "{s:?}");
    }

    #[test]
    fn numbers_and_constants() {
        let t = "a = 42\nb = 0xFF\nc = 3.14\nd = 1_000\ne = 1.5e-3\nok = true\nz = nil\nf = false\n";
        let s = spans(t);
        let cs = at(t, &s, SynKind::Constant);
        for want in ["42", "0xFF", "3.14", "1_000", "1.5e-3", "true", "nil", "false"] {
            assert!(cs.contains(&want), "missing {want}: {cs:?}");
        }
    }

    #[test]
    fn def_and_class_and_module_names() {
        let t = "def frobnicate(x)\nend\nclass Widget\nend\nmodule Util\nend\n";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"frobnicate"), "{ds:?}");
        assert!(ds.contains(&"Widget"), "{ds:?}");
        assert!(ds.contains(&"Util"), "{ds:?}");
        // The `def` keyword itself stays plain.
        assert!(!has(&s, 0, 3, SynKind::Definition), "{s:?}");
    }

    #[test]
    fn def_self_receiver_and_method_suffix() {
        let t = "def self.create\nend\ndef valid?\nend\ndef name=(v)\nend\n";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        // The `self` receiver is skipped; the real names (with `?`/`=`) are marked.
        assert!(ds.contains(&"create"), "{ds:?}");
        assert!(ds.contains(&"valid?"), "{ds:?}");
        assert!(ds.contains(&"name="), "{ds:?}");
        assert!(!ds.contains(&"self"), "receiver must not be a definition: {ds:?}");
    }

    #[test]
    fn keyword_itself_is_not_styled() {
        // Only the NAME is a Definition; `def` stays default ink.
        let t = "def main\nend\n";
        let s = spans(t);
        assert!(!has(&s, 0, 3, SynKind::Definition), "the `def` keyword must stay plain: {s:?}");
        assert!(has(&s, 4, 8, SynKind::Definition), "`main` is the definition: {s:?}");
    }

    #[test]
    fn plain_code_has_no_spans() {
        let t = "result = compute(a, b) + offset";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn reference_snippet() {
        let t = "# add two\ndef add(a, b)\n  total = a + b  # sum\n  total\nend\nMAX = 100\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["# add two", "# sum"], "{s:?}");
        assert!(at(t, &s, SynKind::Definition).contains(&"add"), "{s:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"100"), "{s:?}");
    }
}
