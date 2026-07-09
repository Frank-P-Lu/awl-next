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

/// Every [`Lang`] variant, for iteration in tests/law sweeps AND
/// [`Lang::from_label`]'s reverse lookup.
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

    /// The WRITER-FACING name (Settings menu / the CJK-priority language picker)
    /// — distinct from [`Self::code`] (the machine BCP 47 tag). Round-trips
    /// through [`Self::from_label`].
    pub fn label(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ja => "Japanese",
            Lang::ZhHans => "Simplified Chinese",
            Lang::ZhHant => "Traditional Chinese",
            Lang::Ko => "Korean",
        }
    }

    /// Resolve a picker ROW title ([`Self::label`]) back to its `Lang` —
    /// case-insensitive, `None` for an unknown label. The inverse used by the
    /// CJK-priority picker's commit path (mirrors `DictVariant::from_label`).
    pub fn from_label(s: &str) -> Option<Lang> {
        ALL_LANGS.into_iter().find(|l| l.label().eq_ignore_ascii_case(s))
    }

    /// One quiet line describing the language, drawn dim beside the name in the
    /// CJK-priority picker (the same right-column shape as the caret-style /
    /// dictionary pickers' descriptions).
    pub fn description(self) -> &'static str {
        match self {
            Lang::En => "Not part of the CJK ambiguity ladder",
            Lang::Ja => "Kana + Jōyō kanji — the built-in default",
            Lang::ZhHans => "Simplified hanzi (GB 2312)",
            Lang::ZhHant => "Traditional hanzi",
            Lang::Ko => "Hangul syllables",
        }
    }

    /// Pack to a small byte code for the live [`CJK_PRIORITY`] global — NOT the
    /// BCP 47 code (that's [`Self::code`]); just a compact `const`-friendly
    /// discriminant for [`pack_priority`]/[`unpack_priority`].
    const fn as_u8(self) -> u8 {
        match self {
            Lang::En => 0,
            Lang::Ja => 1,
            Lang::ZhHans => 2,
            Lang::ZhHant => 3,
            Lang::Ko => 4,
        }
    }

    /// The inverse of [`Self::as_u8`]; `None` for a byte that was never one of
    /// ours (defensive — the packed global should never actually produce one).
    const fn from_u8(v: u8) -> Option<Lang> {
        match v {
            0 => Some(Lang::En),
            1 => Some(Lang::Ja),
            2 => Some(Lang::ZhHans),
            3 => Some(Lang::ZhHant),
            4 => Some(Lang::Ko),
            _ => None,
        }
    }
}

/// Pack an ordered 4-`Lang` ladder into one `u32` (one byte per slot,
/// LSB-first) — the storage shape for the [`CJK_PRIORITY`] atomic, chosen so
/// the whole ladder reads/writes lock-free in one instruction, mirroring
/// `spell::ACTIVE_VARIANT` / `caret::MODE_OVERRIDE`'s single-atomic pattern
/// generalized from "one value" to "one small ordered list".
const fn pack_priority(langs: [Lang; 4]) -> u32 {
    (langs[0].as_u8() as u32)
        | ((langs[1].as_u8() as u32) << 8)
        | ((langs[2].as_u8() as u32) << 16)
        | ((langs[3].as_u8() as u32) << 24)
}

/// The inverse of [`pack_priority`].
fn unpack_priority(v: u32) -> Vec<Lang> {
    [v as u8, (v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8]
        .into_iter()
        .filter_map(Lang::from_u8)
        .collect()
}

/// NORMALIZE an arbitrary `langs` slice into a well-formed 4-member CJK
/// ladder: `En` is dropped (it is never part of the ambiguity ladder),
/// duplicates drop (first occurrence wins), and any CJK lang the input left
/// out is appended in [`DEFAULT_CJK_PRIORITY`] order — so the live global can
/// never be stored partial, duplicated, or missing a member, regardless of
/// what a hand-edited config or a defensive caller hands in.
fn normalize_priority(langs: &[Lang]) -> [Lang; 4] {
    let mut out: Vec<Lang> = Vec::with_capacity(4);
    for &l in langs {
        if l != Lang::En && !out.contains(&l) {
            out.push(l);
        }
    }
    for &l in &DEFAULT_CJK_PRIORITY {
        if !out.contains(&l) {
            out.push(l);
        }
    }
    out.truncate(4);
    [out[0], out[1], out[2], out[3]]
}

/// The LIVE process-global CJK ambiguity-tiebreak LADDER — mirrors
/// `spell::ACTIVE_VARIANT` / `caret::MODE_OVERRIDE`: seeded from the config
/// `cjk_priority` pref at launch ([`crate::config::Config::apply_sticky_globals`]),
/// read by the Settings menu's "Ambiguous CJK reads as" row, and SET by the CJK
/// language picker's Enter — inside the shared `apply_core` seam
/// (`actions::overlay_nav`), exactly like the Theme/Caret/Dictionary pickers —
/// so both the live App AND a headless `--keys` replay observe the promotion
/// identically (the whole reason this is a process global rather than a plain
/// `Config` field: `main::run::replay_keys` holds only an `&Config`, so a
/// config-only value could never round-trip through a `--keys` capture).
///
/// The RENDER ladder itself stays config-driven, unaffected by this global:
/// `Config::cjk_priority_or_default` is re-derived from the live App's own
/// `self.config` every reshape (kept in step by the picker's App-only persist
/// step, `App::persist_cjk_priority`), and the headless capture pipeline pins
/// [`DEFAULT_CJK_PRIORITY`] regardless (a pre-existing, documented v1
/// simplification — see `CAPTURE.md`/`CLAUDE.md`'s i18n-round notes). This
/// global exists ONLY so the Settings row's own round trip is observable.
static CJK_PRIORITY: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(pack_priority(DEFAULT_CJK_PRIORITY));

/// The SINGLE test mutex serializing every test that mutates [`CJK_PRIORITY`] —
/// mirrors `spell::TEST_LOCK` / `caret::TEST_LOCK`.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The live CJK ambiguity-tiebreak ladder (always a well-formed 4-member
/// permutation of the CJK [`Lang`]s — see [`normalize_priority`]).
pub fn cjk_priority() -> Vec<Lang> {
    unpack_priority(CJK_PRIORITY.load(std::sync::atomic::Ordering::Relaxed))
}

/// Set the live ladder, normalizing `langs` first ([`normalize_priority`]) so
/// the global can never be corrupted by a partial/duplicate/foreign-tag input.
/// Called by `apply_sticky_globals` (seed at launch) and the CJK picker's
/// core-level accept (`actions::overlay_nav`).
pub fn set_cjk_priority(langs: &[Lang]) {
    CJK_PRIORITY.store(
        pack_priority(normalize_priority(langs)),
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// PROMOTE `lang` to the FRONT of the CURRENT live ladder, keeping the
/// relative order of the rest — the CJK-priority picker's whole point. Pure
/// function of [`cjk_priority`]'s current value + the picked tag;
/// [`set_cjk_priority`] applies the result. `lang` need not already be a CJK
/// tag — [`normalize_priority`] (via `set_cjk_priority`) will fold it in
/// correctly regardless (a defensive floor, not a documented input contract).
pub fn promote_cjk_priority(lang: Lang) -> Vec<Lang> {
    let current = cjk_priority();
    let mut out = vec![lang];
    out.extend(current.into_iter().filter(|&l| l != lang));
    out
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

    #[test]
    fn every_lang_label_round_trips_through_from_label() {
        for l in ALL_LANGS {
            assert_eq!(Lang::from_label(l.label()), Some(l), "{l:?} must round-trip");
        }
        // Case-insensitive, like DictVariant::from_label.
        assert_eq!(Lang::from_label("japanese"), Some(Lang::Ja));
        assert_eq!(Lang::from_label("nonsense"), None);
    }

    #[test]
    fn pack_unpack_priority_round_trips() {
        for perm in [
            DEFAULT_CJK_PRIORITY,
            [Lang::Ko, Lang::ZhHant, Lang::ZhHans, Lang::Ja],
            [Lang::ZhHans, Lang::Ja, Lang::Ko, Lang::ZhHant],
        ] {
            assert_eq!(unpack_priority(pack_priority(perm)), perm.to_vec());
        }
    }

    #[test]
    fn normalize_priority_drops_en_dedups_and_fills_gaps() {
        // A short, En-polluted, duplicated list still normalizes to a full,
        // well-formed 4-member CJK permutation — En dropped, first occurrence
        // of a dup wins, and the missing members fill in DEFAULT order.
        let n = normalize_priority(&[Lang::En, Lang::Ko, Lang::Ko, Lang::En]);
        assert_eq!(n, [Lang::Ko, Lang::Ja, Lang::ZhHans, Lang::ZhHant]);
        // An already-well-formed permutation is untouched (order preserved).
        let full = [Lang::ZhHant, Lang::Ko, Lang::Ja, Lang::ZhHans];
        assert_eq!(normalize_priority(&full), full);
        // A totally empty input falls back to the built-in default order.
        assert_eq!(normalize_priority(&[]), DEFAULT_CJK_PRIORITY);
    }

    #[test]
    fn cjk_priority_global_defaults_seeds_sets_and_promotes() {
        let _g = TEST_LOCK.lock().unwrap();
        // Reset to the built-in default so this test is order-independent.
        set_cjk_priority(&DEFAULT_CJK_PRIORITY);
        assert_eq!(cjk_priority(), DEFAULT_CJK_PRIORITY.to_vec());

        // Promoting the already-front language is a no-op (still the same order).
        assert_eq!(promote_cjk_priority(Lang::Ja), DEFAULT_CJK_PRIORITY.to_vec());

        // Promoting Korean moves it to the front; the REST keep their relative
        // order (Ja, ZhHans, ZhHant) — the picker's whole point.
        let promoted = promote_cjk_priority(Lang::Ko);
        assert_eq!(promoted, vec![Lang::Ko, Lang::Ja, Lang::ZhHans, Lang::ZhHant]);
        set_cjk_priority(&promoted);
        assert_eq!(cjk_priority(), promoted);

        // Promoting zh-Hant (currently 3rd) keeps Ko/Ja's relative order.
        let promoted2 = promote_cjk_priority(Lang::ZhHant);
        assert_eq!(promoted2, vec![Lang::ZhHant, Lang::Ko, Lang::Ja, Lang::ZhHans]);

        // Cleanup: leave the global at the built-in default for other tests.
        set_cjk_priority(&DEFAULT_CJK_PRIORITY);
    }

    #[test]
    fn set_cjk_priority_normalizes_a_malformed_input() {
        let _g = TEST_LOCK.lock().unwrap();
        set_cjk_priority(&[Lang::En, Lang::Ko]);
        assert_eq!(cjk_priority(), vec![Lang::Ko, Lang::Ja, Lang::ZhHans, Lang::ZhHant]);
        set_cjk_priority(&DEFAULT_CJK_PRIORITY);
    }
}
