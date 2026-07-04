//! FRONTMATTER — a `---`-delimited metadata block at the very TOP of a markdown
//! document (byte 0 only; never recognized mid-document), the Jekyll/Hugo-style
//! convention every static-site/note tool already knows. awl reads exactly ONE
//! key out of it this round: `lang:` (a BCP 47 tag: `en` / `ja` / `zh-Hans` /
//! `zh-Hant` / `ko` — see [`Lang`]) — the doc-language TAG the i18n render
//! ladder and the write-back-once detector (`app/files.rs`) both consult. Every
//! OTHER key is syntactically accepted (so a real frontmatter block with
//! `title:`/`date:`/… parses without failing) but semantically INERT — never
//! crashes, never does anything.
//!
//! [`detect`] is deliberately STRICT, not a loose "starts with `---`" sniff:
//! every non-blank line between the opening and closing `---` must be shaped
//! like `key: value` (an identifier-ish key + a colon), or the whole thing
//! bails (`None`) — a document that merely OPENS with a thematic-break `---`
//! (with ordinary prose before a LATER unrelated `---` break) must never be
//! misread as a frontmatter block and have its real content silently swallowed.
//! A block with NO closing `---` at all is likewise `None` (an unterminated
//! opener is just an ordinary line, not a metadata block).
//!
//! RENDER: `markdown::spans` prepends the block's whole byte range as a
//! `MdKind::ConcealMarkup(ConcealKind::Frontmatter)` span — dim `Markup`
//! styling, and it obeys the SAME block-scoped WYSIWYG conceal a fenced code
//! block does (reveals only while the caret sits somewhere inside the block;
//! `wysiwyg = false` shows it dim-but-visible, never concealed — no new
//! machinery, the exact `Fence` seam generalizes). It is also EXCLUDED from
//! word-count/reading-time, spell-check, and writing-nits (metadata, not
//! manuscript) — see `render/chrome.rs::word_count`, `render/rects.rs::
//! ensure_nit_protos`, and `spell::misspellings_for`'s `None`-lang branch.
//!
//! Pure + total: no clock, no filesystem, no panics on malformed input.

use std::ops::Range;

/// A BCP 47 document-language tag awl recognizes in frontmatter's `lang:` key
/// and the config `cjk_priority` ladder. Unrecognized tags are simply not
/// parsed ([`Lang::parse`] returns `None`) — "unknown keys inert, never
/// crash".
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    En,
    Ja,
    ZhHans,
    ZhHant,
    Ko,
}

/// Every [`Lang`] variant, for iteration in tests/law sweeps.
pub const ALL_LANGS: [Lang; 5] = [Lang::En, Lang::Ja, Lang::ZhHans, Lang::ZhHant, Lang::Ko];

/// The default `cjk_priority` tiebreak ladder (config `cjk_priority`, TOML
/// array of these codes): a Han-only (ambiguous) run/document defaults to
/// Japanese first, then Simplified, then Traditional, then Korean.
pub const DEFAULT_CJK_PRIORITY: [Lang; 4] = [Lang::Ja, Lang::ZhHans, Lang::ZhHant, Lang::Ko];

impl Lang {
    /// Parse a BCP 47 tag (case-insensitively), or `None` for anything
    /// unrecognized — never a crash, never a panic.
    pub fn parse(s: &str) -> Option<Lang> {
        match s.trim().to_ascii_lowercase().as_str() {
            "en" => Some(Lang::En),
            "ja" => Some(Lang::Ja),
            "zh-hans" => Some(Lang::ZhHans),
            "zh-hant" => Some(Lang::ZhHant),
            "ko" => Some(Lang::Ko),
            _ => None,
        }
    }

    /// The canonical BCP 47 code string (round-trips through [`Lang::parse`]).
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ja => "ja",
            Lang::ZhHans => "zh-Hans",
            Lang::ZhHant => "zh-Hant",
            Lang::Ko => "ko",
        }
    }
}

/// One parsed frontmatter block: its whole BYTE RANGE in the document (always
/// `0..end`, including both `---` lines and their trailing newlines) plus the
/// `lang:` tag, if present and recognized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frontmatter {
    pub range: Range<usize>,
    pub lang: Option<Lang>,
}

/// True for a byte that may appear in a frontmatter KEY: ASCII alphanumerics,
/// `_`, `-`. Deliberately narrow (YAML keys in practice are always this shape);
/// anything else on a non-blank line fails the line and bails the whole block.
fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Detect a frontmatter block at byte 0 of `text`, or `None` if there isn't a
/// well-formed one. See the module doc for the strictness rationale.
pub fn detect(text: &str) -> Option<Frontmatter> {
    let mut iter = text.split_inclusive('\n');
    let first = iter.next()?;
    if first.trim_end_matches(['\n', '\r']) != "---" {
        return None;
    }
    let mut offset = first.len();
    let mut lang = None;
    for raw in iter {
        let line = raw.trim_end_matches(['\n', '\r']);
        if line == "---" {
            offset += raw.len();
            return Some(Frontmatter { range: 0..offset, lang });
        }
        if line.trim().is_empty() {
            offset += raw.len();
            continue;
        }
        let Some((key, val)) = line.split_once(':') else {
            return None; // not a `key: value` line -> not a frontmatter block
        };
        let key = key.trim();
        if key.is_empty() || !key.chars().all(is_key_char) {
            return None;
        }
        if key.eq_ignore_ascii_case("lang") {
            lang = Lang::parse(val.trim());
        }
        // Any OTHER key (recognized or not) is syntactically fine and simply
        // inert — "unknown keys inert, never crash".
        offset += raw.len();
    }
    None // no closing `---` found -> not a valid frontmatter block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_lang_tag() {
        let fm = detect("---\nlang: ja\n---\n# Title\n").expect("valid frontmatter");
        assert_eq!(fm.lang, Some(Lang::Ja));
        assert_eq!(&"---\nlang: ja\n---\n# Title\n"[fm.range.clone()], "---\nlang: ja\n---\n");
    }

    #[test]
    fn all_five_bcp47_tags_parse() {
        for (tag, want) in [
            ("en", Lang::En),
            ("ja", Lang::Ja),
            ("zh-Hans", Lang::ZhHans),
            ("zh-Hant", Lang::ZhHant),
            ("ko", Lang::Ko),
        ] {
            let doc = format!("---\nlang: {tag}\n---\n");
            let fm = detect(&doc).unwrap_or_else(|| panic!("{tag} should parse: {doc:?}"));
            assert_eq!(fm.lang, Some(want), "tag {tag}");
            assert_eq!(Lang::parse(want.code()), Some(want), "code() round-trips");
        }
    }

    #[test]
    fn tag_is_case_insensitive_on_the_key_and_value() {
        let fm = detect("---\nLang: JA\n---\n").expect("valid");
        assert_eq!(fm.lang, Some(Lang::Ja));
    }

    #[test]
    fn unknown_lang_value_is_inert_not_a_crash() {
        let fm = detect("---\nlang: klingon\n---\nbody\n").expect("still valid frontmatter");
        assert_eq!(fm.lang, None, "unrecognized tag is simply not parsed");
    }

    #[test]
    fn unknown_keys_are_inert() {
        let fm = detect("---\ntitle: My Doc\ndate: 2026-01-01\nlang: ko\n---\n").expect("valid");
        assert_eq!(fm.lang, Some(Lang::Ko), "lang still extracted alongside unknown keys");
    }

    #[test]
    fn no_lang_key_at_all_is_fine() {
        let fm = detect("---\ntitle: x\n---\n").expect("valid, no lang");
        assert_eq!(fm.lang, None);
    }

    #[test]
    fn only_at_byte_zero_never_mid_document() {
        // A `---` block anywhere but the very start of the text is never a
        // frontmatter block (mid-doc it is only ever a thematic break / setext
        // underline, handled entirely by `markdown::spans`).
        assert!(detect("\n---\nlang: ja\n---\n").is_none(), "leading blank line disqualifies it");
        assert!(detect("prelude\n---\nlang: ja\n---\n").is_none());
    }

    #[test]
    fn no_closing_delimiter_is_not_frontmatter() {
        assert!(detect("---\nlang: ja\n").is_none(), "unterminated opener is not a block");
        assert!(detect("---\n").is_none(), "a bare opener with nothing after it");
        assert!(detect("---").is_none(), "a single dash-rule line, no newline at all");
    }

    #[test]
    fn plain_prose_starting_with_a_rule_stays_untouched() {
        // The classic false-positive risk: a document that legitimately opens
        // with a thematic break, has ordinary prose, and later has an unrelated
        // second break. None of that prose is "key: value" shaped, so `detect`
        // must bail entirely rather than swallowing the prose as metadata.
        let doc = "---\n\nSome opening prose about nothing in particular.\n\n---\n\nMore prose.\n";
        assert!(detect(doc).is_none(), "a non-kv line between the dashes bails: {doc:?}");
    }

    #[test]
    fn blank_lines_inside_the_block_are_tolerated() {
        let fm = detect("---\nlang: ja\n\ntitle: x\n---\n").expect("blank line inside is fine");
        assert_eq!(fm.lang, Some(Lang::Ja));
    }

    #[test]
    fn crlf_line_endings_are_handled() {
        let fm = detect("---\r\nlang: ja\r\n---\r\nbody\r\n").expect("CRLF frontmatter parses");
        assert_eq!(fm.lang, Some(Lang::Ja));
    }

    #[test]
    fn empty_frontmatter_block_is_valid_with_no_lang() {
        let fm = detect("---\n---\n").expect("an immediately-closed block is valid");
        assert_eq!(fm.lang, None);
        assert_eq!(fm.range, 0..8);
    }

    #[test]
    fn all_langs_round_trip_through_parse_and_code() {
        // ALL_LANGS is the law-sweep list every future `Lang` variant must join
        // (a no-wildcard `match` on `Lang` elsewhere is the real compile-time
        // guard; this pins that the LIST itself stays exhaustive by eye).
        assert_eq!(ALL_LANGS.len(), 5);
        for l in ALL_LANGS {
            assert_eq!(Lang::parse(l.code()), Some(l), "{l:?} must round-trip");
        }
        assert_eq!(DEFAULT_CJK_PRIORITY, [Lang::Ja, Lang::ZhHans, Lang::ZhHant, Lang::Ko]);
    }

    #[test]
    fn never_panics_on_garbage() {
        // A grab-bag of malformed/edge inputs must never panic.
        for doc in ["", "-", "--", "----", "---x", "---\n:::\n---\n", "---\n:\n---\n"] {
            let _ = detect(doc);
        }
    }
}
