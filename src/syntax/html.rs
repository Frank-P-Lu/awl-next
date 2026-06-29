//! HTML syntax lexer — a minimal hand-written byte scanner emitting only the four
//! Alabaster roles; everything else (tag names, attribute names, `<`/`>`/`=`
//! punctuation, text content) stays the default ink:
//!
//! - [`SynKind::Comment`]    — `<!-- ... -->` block comments (the only comment
//!   HTML has; they cross newlines and recede to the dim ink).
//! - [`SynKind::Str`]        — quoted attribute VALUES (`"..."` / `'...'`); HTML
//!   uses no backslash escapes, so a value runs to its matching quote (newlines
//!   allowed) — entities inside it carry the meaning, not `\`.
//! - [`SynKind::Constant`]   — character/entity references: named (`&copy;`),
//!   decimal (`&#169;`) and hex (`&#x1F600;`). These are HTML's literal values
//!   (a numeric reference IS a number), so they map to the Constant role.
//! - [`SynKind::Definition`] — the NAME an element introduces: the value of an
//!   `id=`/`name=` attribute (the closest HTML analogue to "the name being
//!   defined"). Marked best-effort, keyword-(attribute-)driven, mirroring the
//!   `expect_def` flag in `rust.rs`/`python.rs`.
//!
//! Raw-text elements (`<script>` / `<style>`) have their bodies SKIPPED (left as
//! the default ink) so embedded JS/CSS — with its own `<`, quotes and `//`
//! sequences — is never mis-lexed as HTML. That body is out of scope here (a
//! best-effort limitation; awl is for prose + light editing). Span boundaries land
//! on ASCII bytes (`<`, `>`, quotes, `&`, `;`), so multibyte UTF-8 inside a
//! string/comment rides inside the span. Pure + single-pass. See the tests below
//! for the exact contract on a sample document.

use super::SynKind;
use std::ops::Range;

/// A byte that may appear in a tag name or attribute name (letters, digits, plus
/// the `-`/`_`/`:`/`.` used by data-attributes and XML namespaces).
fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b':' | b'.')
}

/// True if `b[i..]` begins with `needle`.
fn starts_with(b: &[u8], i: usize, needle: &[u8]) -> bool {
    b.len() >= i + needle.len() && &b[i..i + needle.len()] == needle
}

pub fn spans(text: &str) -> Vec<(Range<usize>, SynKind)> {
    let b = text.as_bytes();
    let n = b.len();
    let mut out: Vec<(Range<usize>, SynKind)> = Vec::new();
    let mut i = 0usize;
    // Inside the `<...>` of a start/end tag (attribute territory) vs text content.
    let mut in_tag = false;
    // The start tag's name (Some only between `<name` and its `>`), used to detect
    // the raw-text elements whose body we skip.
    let mut tag_name: Option<Range<usize>> = None;
    // The next attribute VALUE is an `id`/`name` value -> mark it `Definition`.
    let mut expect_def = false;

    while i < n {
        let c = b[i];

        if !in_tag {
            // --- block comment: <!-- ... --> (crosses newlines) ---
            if c == b'<' && starts_with(b, i, b"<!--") {
                let start = i;
                i += 4;
                while i < n && !starts_with(b, i, b"-->") {
                    i += 1;
                }
                i = if i < n { i + 3 } else { n };
                out.push((start..i, SynKind::Comment));
                continue;
            }

            // --- tag start: '<' followed by a name, '/', '!' or '?' ---
            if c == b'<' {
                match b.get(i + 1).copied() {
                    Some(b'/') => {
                        // End tag: skip `</name`, leave the name in the default ink.
                        in_tag = true;
                        tag_name = None;
                        expect_def = false;
                        i += 2;
                        while i < n && is_name_char(b[i]) {
                            i += 1;
                        }
                        continue;
                    }
                    Some(x) if x.is_ascii_alphabetic() => {
                        // Start tag: consume `<name`, remember the name (default ink).
                        i += 1;
                        let ns = i;
                        while i < n && is_name_char(b[i]) {
                            i += 1;
                        }
                        tag_name = Some(ns..i);
                        in_tag = true;
                        expect_def = false;
                        continue;
                    }
                    Some(b'!') | Some(b'?') => {
                        // Declaration (`<!DOCTYPE …`) or processing instruction —
                        // enter attribute territory but with no element name.
                        in_tag = true;
                        tag_name = None;
                        expect_def = false;
                        i += 1;
                        continue;
                    }
                    // A bare `<` in text (e.g. `a < b`): just punctuation.
                    _ => {
                        i += 1;
                        continue;
                    }
                }
            }

            // --- entity / character reference in text content ---
            if c == b'&' {
                if let Some(end) = scan_entity(b, i) {
                    out.push((i..end, SynKind::Constant));
                    i = end;
                    continue;
                }
            }

            i += 1;
            continue;
        }

        // --- inside a tag ---
        if c == b'>' {
            in_tag = false;
            let self_close = i > 0 && b[i - 1] == b'/';
            i += 1;
            // A non-self-closed <script>/<style>: skip its raw-text body so the
            // embedded JS/CSS is never mis-lexed as HTML.
            if !self_close {
                if let Some(rng) = tag_name.clone() {
                    let name = &text[rng];
                    if name.eq_ignore_ascii_case("script") || name.eq_ignore_ascii_case("style") {
                        i = find_raw_close(b, i, name.as_bytes());
                    }
                }
            }
            tag_name = None;
            expect_def = false;
            continue;
        }

        // --- quoted attribute value ---
        if c == b'"' || c == b'\'' {
            let start = i;
            let end = scan_attr_string(b, i);
            out.push((start..end, SynKind::Str));
            if expect_def {
                // The NAME inside the quotes is the definition (last-wins over Str).
                let inner_start = start + 1;
                let inner_end = if end > inner_start && b[end - 1] == c {
                    end - 1
                } else {
                    end
                };
                if inner_end > inner_start {
                    out.push((inner_start..inner_end, SynKind::Definition));
                }
                expect_def = false;
            }
            i = end;
            continue;
        }

        // --- attribute name (or an unquoted value) ---
        if is_name_char(c) {
            let start = i;
            i += 1;
            while i < n && is_name_char(b[i]) {
                i += 1;
            }
            if expect_def {
                // Unquoted `id=value` / `name=value`: this token IS the value.
                out.push((start..i, SynKind::Definition));
                expect_def = false;
            } else {
                let word = &text[start..i];
                if word.eq_ignore_ascii_case("id") || word.eq_ignore_ascii_case("name") {
                    // Only an attribute (a `=` follows, after optional whitespace)
                    // introduces a definition — not a bare/boolean `id`.
                    let mut k = i;
                    while k < n && b[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    expect_def = k < n && b[k] == b'=';
                }
            }
            continue;
        }

        // `=`, whitespace, `/`, etc. — punctuation; keep waiting for the value.
        i += 1;
    }

    out
}

/// Scan a quoted attribute value from the opening quote `q` to just past its
/// matching close (or EOF). HTML attribute values have no `\` escapes and MAY
/// span newlines, so we run straight to the next identical quote.
fn scan_attr_string(b: &[u8], q: usize) -> usize {
    let n = b.len();
    let quote = b[q];
    let mut i = q + 1;
    while i < n {
        if b[i] == quote {
            return i + 1;
        }
        i += 1;
    }
    n
}

/// If a valid character/entity reference starts at the `&` at `i`, return the index
/// just past its closing `;`; else `None` (a bare `&` is plain punctuation).
/// Accepts named (`&amp;`), decimal (`&#169;`) and hex (`&#x1F600;`) references.
fn scan_entity(b: &[u8], i: usize) -> Option<usize> {
    let n = b.len();
    let mut j = i + 1;
    if j < n && b[j] == b'#' {
        j += 1;
        if j < n && (b[j] == b'x' || b[j] == b'X') {
            j += 1;
            let ds = j;
            while j < n && b[j].is_ascii_hexdigit() {
                j += 1;
            }
            if j > ds && j < n && b[j] == b';' {
                return Some(j + 1);
            }
            return None;
        }
        let ds = j;
        while j < n && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > ds && j < n && b[j] == b';' {
            return Some(j + 1);
        }
        return None;
    }
    let ds = j;
    while j < n && b[j].is_ascii_alphanumeric() {
        j += 1;
    }
    if j > ds && j < n && b[j] == b';' {
        Some(j + 1)
    } else {
        None
    }
}

/// From `from`, find the `<` of the closing `</name>` for a raw-text element
/// (case-insensitive), so its body can be skipped; returns `n` if none is found.
fn find_raw_close(b: &[u8], from: usize, name: &[u8]) -> usize {
    let n = b.len();
    let mut i = from;
    while i < n {
        if b[i] == b'<' && i + 1 < n && b[i + 1] == b'/' {
            let ns = i + 2;
            if matches_ci(b, ns, name) {
                let after = ns + name.len();
                // The name must end the closing tag's name (terminator follows).
                if after >= n || !is_name_char(b[after]) {
                    return i;
                }
            }
        }
        i += 1;
    }
    n
}

/// True if `b[i..]` begins with `needle`, comparing ASCII case-insensitively.
fn matches_ci(b: &[u8], i: usize, needle: &[u8]) -> bool {
    b.len() >= i + needle.len()
        && b[i..i + needle.len()]
            .iter()
            .zip(needle)
            .all(|(x, y)| x.eq_ignore_ascii_case(y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at<'a>(text: &'a str, s: &[(Range<usize>, SynKind)], k: SynKind) -> Vec<&'a str> {
        s.iter()
            .filter(|(_, kk)| *kk == k)
            .map(|(r, _)| &text[r.clone()])
            .collect()
    }

    #[test]
    fn block_comment_recedes() {
        let t = "<p>hi</p><!-- a note -->\n";
        assert_eq!(at(t, &spans(t), SynKind::Comment), vec!["<!-- a note -->"]);
    }

    #[test]
    fn multiline_comment() {
        let t = "<!-- one\ntwo -->\n";
        assert_eq!(at(t, &spans(t), SynKind::Comment), vec!["<!-- one\ntwo -->"]);
    }

    #[test]
    fn attribute_values_are_strings() {
        let t = "<a href=\"/x\" title='go'>link</a>";
        let s = spans(t);
        let ss = at(t, &s, SynKind::Str);
        assert!(ss.contains(&"\"/x\""), "{ss:?}");
        assert!(ss.contains(&"'go'"), "{ss:?}");
    }

    #[test]
    fn entities_are_constants() {
        let t = "x &amp; y &#169; z &#x1F600; end";
        let cs = at(t, &spans(t), SynKind::Constant);
        assert!(cs.contains(&"&amp;"), "{cs:?}");
        assert!(cs.contains(&"&#169;"), "{cs:?}");
        assert!(cs.contains(&"&#x1F600;"), "{cs:?}");
    }

    #[test]
    fn bare_ampersand_is_not_an_entity() {
        let t = "a & b &not;valid here";
        // `& ` (no `;`) is plain; `&not;` is a complete reference.
        let cs = at(t, &spans(t), SynKind::Constant);
        assert_eq!(cs, vec!["&not;"], "{cs:?}");
    }

    #[test]
    fn id_and_name_values_are_definitions() {
        let t = "<div id=\"main\"><input name='email'></div>";
        let s = spans(t);
        let ds = at(t, &s, SynKind::Definition);
        assert!(ds.contains(&"main"), "{ds:?}");
        assert!(ds.contains(&"email"), "{ds:?}");
        // The id value is also a Str (coarse span under the Definition).
        assert!(at(t, &s, SynKind::Str).contains(&"\"main\""), "{s:?}");
    }

    #[test]
    fn unquoted_id_value_is_definition() {
        let t = "<div id=main class=box>";
        let ds = at(t, &spans(t), SynKind::Definition);
        assert_eq!(ds, vec!["main"], "{ds:?}");
    }

    #[test]
    fn tag_and_attribute_names_stay_plain() {
        // No tag name, attribute name, or `class` value gets a span.
        let t = "<section class=\"hero\" role=banner>text</section>";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Definition).is_empty(), "{s:?}");
        assert!(at(t, &s, SynKind::Comment).is_empty(), "{s:?}");
        // Only the quoted class value is a Str; tag/attr identifiers ride default.
        assert_eq!(at(t, &s, SynKind::Str), vec!["\"hero\""], "{s:?}");
    }

    #[test]
    fn doctype_does_not_crash_or_overhighlight() {
        let t = "<!DOCTYPE html>\n<html lang=\"en\"></html>";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Str), vec!["\"en\""], "{s:?}");
        assert!(at(t, &s, SynKind::Comment).is_empty(), "{s:?}");
    }

    #[test]
    fn bare_less_than_in_text_is_punctuation() {
        let t = "if a < b then";
        assert!(spans(t).is_empty(), "{:?}", spans(t));
    }

    #[test]
    fn script_body_is_skipped() {
        // Embedded JS — its `<`, quotes and `//` must NOT be lexed as HTML.
        let t = "<script>var s = \"x < y\"; // note\n</script><b>ok</b>";
        let s = spans(t);
        assert!(at(t, &s, SynKind::Str).is_empty(), "{s:?}");
        assert!(at(t, &s, SynKind::Comment).is_empty(), "{s:?}");
    }

    #[test]
    fn reference_snippet() {
        let t = "<!-- header -->\n<header id=\"top\">\n  <a href=\"/\">Home &amp; Away</a>\n</header>\n";
        let s = spans(t);
        assert_eq!(at(t, &s, SynKind::Comment), vec!["<!-- header -->"], "{s:?}");
        assert!(at(t, &s, SynKind::Definition).contains(&"top"), "{s:?}");
        assert!(at(t, &s, SynKind::Str).contains(&"\"/\""), "{s:?}");
        assert!(at(t, &s, SynKind::Constant).contains(&"&amp;"), "{s:?}");
    }
}
