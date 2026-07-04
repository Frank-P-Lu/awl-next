//! SCRIPT CLASSIFICATION — a pure Unicode-scalar-value classifier over the
//! four non-Latin scripts the i18n round distinguishes (Kana / Hangul /
//! Bopomofo / Han), and the two ladders built on top of it:
//!
//!  - [`dominant_cjk`] scans a WHOLE document once for the doc-language
//!    WRITE-BACK detector (`app/files.rs`'s untagged-doc-open path): an
//!    unambiguous script (kana -> ja, hangul -> ko, bopomofo -> zh-Hant hint)
//!    always wins over a merely-present Han run; a Han-ONLY document is
//!    ambiguous and falls to the config `cjk_priority` tiebreak
//!    ([`doc_lang_for`]).
//!  - [`resolve_font_id`] is the per-RUN RENDER resolution ladder
//!    (`render/spans.rs`'s per-script span generalization of the old
//!    Japanese-only `add_cjk_spans`): (a) the document's own frontmatter
//!    `lang:` tag, if compatible with this run's script; (b) else the run's
//!    own unambiguous script mapping; (c) else (a Han run with no compatible
//!    tag) the `cjk_priority` tiebreak; (d) else [`crate::theme::FontId::Latin`]
//!    (the base default — the guaranteed floor; a CJK-classified run always
//!    resolves by (c), so (d) is reached only for an already-Latin run, which
//!    never calls this in practice).
//!
//! Pure + deterministic (no clock, no I/O) — every function here is a plain
//! `&str`/`char` -> value transform, unit-testable with no GPU/buffer/theme.

use crate::frontmatter::Lang;
use crate::theme::FontId;
use std::ops::Range;

/// The four non-Latin SCRIPTS awl distinguishes for doc-lang detection and
/// per-run font resolution. Deliberately narrower than a full Unicode script
/// database — just the signals the i18n ladder needs; a Latin/ASCII/digit/
/// punctuation/whitespace codepoint classifies as `None` (see
/// [`classify_char`]), since a Latin run never needs script-based resolution
/// (it already shapes in the world's own display face).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Script {
    /// Hiragana + Katakana (+ their phonetic extensions) — unambiguously
    /// Japanese.
    Kana,
    /// Hangul syllables + Jamo — unambiguously Korean.
    Hangul,
    /// Bopomofo (Zhuyin) — a strong Traditional-Chinese hint (the phonetic
    /// system used almost exclusively in Taiwan).
    Bopomofo,
    /// CJK Unified Ideographs (+ Extension A + compatibility ideographs) —
    /// "Han", ambiguous on its own among ja / zh-Hans / zh-Hant / ko (all four
    /// use Han characters); resolved by a doc tag or the `cjk_priority`
    /// tiebreak.
    Han,
}

impl Script {
    /// This script's OWN unambiguous [`FontId`] mapping, independent of any
    /// doc tag — ladder step (b). `Han` is deliberately `None`: it is
    /// ambiguous among all four CJK languages and always needs either a
    /// compatible doc tag (step a) or the `cjk_priority` tiebreak (step c).
    pub fn natural_font_id(self) -> Option<FontId> {
        match self {
            Script::Kana => Some(FontId::Ja),
            Script::Hangul => Some(FontId::Ko),
            Script::Bopomofo => Some(FontId::ZhHant),
            Script::Han => None,
        }
    }
}

/// Classify ONE scalar value's script. `None` for Latin/ASCII/digits/
/// punctuation/whitespace/anything else — only the four CJK-family scripts
/// classify as `Some`. Mirrors [`crate::render::spans::is_cjk`]'s codepoint
/// ranges (kept in sync by hand — both are Unicode block membership tests),
/// generalized to name WHICH script a codepoint belongs to rather than just
/// "is this CJK".
pub fn classify_char(c: char) -> Option<Script> {
    match c as u32 {
        0x3040..=0x309F | 0x31F0..=0x31FF => Some(Script::Kana), // Hiragana + phonetic ext
        0x30A0..=0x30FF => Some(Script::Kana),                   // Katakana
        0xAC00..=0xD7A3 => Some(Script::Hangul),                 // Hangul syllables
        0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xD7B0..=0xD7FF => {
            Some(Script::Hangul) // Hangul Jamo (+ extended A/B)
        }
        0x3105..=0x312F | 0x31A0..=0x31BF => Some(Script::Bopomofo), // Bopomofo + ext
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => Some(Script::Han),
        _ => None,
    }
}

/// Maximal contiguous byte ranges sharing the SAME classified [`Script`]
/// within `text` — the per-run detector [`crate::render::spans::add_cjk_spans`]'s
/// generalization walks (mirrors [`crate::render::spans::cjk_runs`], now
/// naming WHICH script each run is instead of a flat "is CJK"). A script
/// boundary (e.g. Han -> Kana) always starts a new run even with zero bytes
/// between them; a non-CJK byte (Latin/space/punctuation) ends the current
/// run without starting a new one. Byte indices are valid `char` boundaries
/// (from `char_indices`), safe for `AttrsList::add_span`.
pub fn script_runs(text: &str) -> Vec<(Range<usize>, Script)> {
    let mut runs = Vec::new();
    let mut cur: Option<(usize, Script)> = None;
    for (i, c) in text.char_indices() {
        match classify_char(c) {
            Some(s) => match cur {
                Some((_, cs)) if cs == s => {} // continue the run
                Some((start, cs)) => {
                    runs.push((start..i, cs));
                    cur = Some((i, s));
                }
                None => cur = Some((i, s)),
            },
            None => {
                if let Some((start, cs)) = cur.take() {
                    runs.push((start..i, cs));
                }
            }
        }
    }
    if let Some((start, cs)) = cur.take() {
        runs.push((start..text.len(), cs));
    }
    runs
}

/// The document's DOMINANT CJK script signal, scanning the WHOLE text once —
/// the doc-lang WRITE-BACK detector's core (`app/files.rs`). Priority order
/// (an UNAMBIGUOUS script always wins over a merely-present Han run): Kana
/// (Japanese) > Hangul (Korean) > Bopomofo (a zh-Hant hint) > Han (ambiguous,
/// falls to the `cjk_priority` tiebreak via [`doc_lang_for`]) > `None` (no CJK
/// at all — a pure-Latin document, never touched by write-back).
pub fn dominant_cjk(text: &str) -> Option<Script> {
    let mut has_bopomofo = false;
    let mut has_han = false;
    for c in text.chars() {
        match classify_char(c) {
            Some(Script::Kana) => return Some(Script::Kana),
            Some(Script::Hangul) => return Some(Script::Hangul),
            Some(Script::Bopomofo) => has_bopomofo = true,
            Some(Script::Han) => has_han = true,
            None => {}
        }
    }
    if has_bopomofo {
        Some(Script::Bopomofo)
    } else if has_han {
        Some(Script::Han)
    } else {
        None
    }
}

/// Resolve a [`dominant_cjk`] signal into the concrete [`Lang`] tag write-back
/// stamps, using `cjk_priority` (the config ladder, default `[Ja, ZhHans,
/// ZhHant, Ko]` — [`crate::frontmatter::DEFAULT_CJK_PRIORITY`]) to break Han's
/// ambiguity. Kana/Hangul/Bopomofo are unambiguous and ignore the priority
/// ladder entirely; only `Han` consults it, falling back to `Lang::Ja` if the
/// configured ladder is empty (never panics — total function).
pub fn doc_lang_for(script: Script, cjk_priority: &[Lang]) -> Lang {
    match script {
        Script::Kana => Lang::Ja,
        Script::Hangul => Lang::Ko,
        Script::Bopomofo => Lang::ZhHant,
        Script::Han => cjk_priority.first().copied().unwrap_or(Lang::Ja),
    }
}

impl Lang {
    /// The [`FontId`] a document TAGGED with this language picks for a run
    /// detected as `script` — render ladder step (a). `None` when this tag has
    /// no natural mapping for `script` (e.g. a `ja`-tagged doc meeting a
    /// Hangul run), so the caller falls through to step (b)/(c). `En` never
    /// maps anything (an English tag never overrides CJK resolution).
    pub fn font_id_for_script(self, script: Option<Script>) -> Option<FontId> {
        use Script::*;
        match (self, script) {
            (Lang::Ja, Some(Kana) | Some(Han)) => Some(FontId::Ja),
            (Lang::ZhHans, Some(Han)) => Some(FontId::ZhHans),
            (Lang::ZhHant, Some(Han) | Some(Bopomofo)) => Some(FontId::ZhHant),
            (Lang::Ko, Some(Hangul) | Some(Han)) => Some(FontId::Ko),
            _ => None,
        }
    }
}

/// THE render resolution ladder for ONE text run's [`FontId`] — the pure
/// recipe [`crate::render::spans::add_cjk_spans`]'s per-script generalization
/// walks for every run:
///
///  (a) the doc tag's own mapping for this run's script, if compatible;
///  (b) else the script's own unambiguous mapping ([`Script::natural_font_id`]);
///  (c) else (a Han run with no compatible tag) the `cjk_priority` ladder;
///  (d) else [`FontId::Latin`] (the base default / guaranteed floor — reached
///      only when `detected` is `None`, i.e. a run this ladder shouldn't
///      normally be asked about at all).
///
/// `doc_lang` is the document's frontmatter tag, if any; `detected` is the
/// run's own [`Script`] (`None` for a plain Latin run); `cjk_priority` is the
/// config ladder (default `[Ja, ZhHans, ZhHant, Ko]`).
pub fn resolve_font_id(doc_lang: Option<Lang>, detected: Option<Script>, cjk_priority: &[Lang]) -> FontId {
    if let Some(dl) = doc_lang {
        if let Some(id) = dl.font_id_for_script(detected) {
            return id;
        }
    }
    let Some(s) = detected else {
        return FontId::Latin;
    };
    if let Some(id) = s.natural_font_id() {
        return id;
    }
    match doc_lang_for(s, cjk_priority) {
        Lang::Ja => FontId::Ja,
        Lang::ZhHans => FontId::ZhHans,
        Lang::ZhHant => FontId::ZhHant,
        Lang::Ko => FontId::Ko,
        Lang::En => FontId::Latin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_char_covers_each_script() {
        assert_eq!(classify_char('あ'), Some(Script::Kana), "hiragana");
        assert_eq!(classify_char('ア'), Some(Script::Kana), "katakana");
        assert_eq!(classify_char('한'), Some(Script::Hangul), "hangul syllable");
        assert_eq!(classify_char('ㅎ'), Some(Script::Hangul), "hangul jamo");
        assert_eq!(classify_char('ㄅ'), Some(Script::Bopomofo), "bopomofo");
        assert_eq!(classify_char('漢'), Some(Script::Han), "han/kanji/hanzi");
        assert_eq!(classify_char('字'), Some(Script::Han));
    }

    #[test]
    fn classify_char_latin_ascii_digits_punct_are_none() {
        for c in ['a', 'Z', '0', '9', ' ', '\n', '.', ',', '!', 'é', 'ñ'] {
            assert_eq!(classify_char(c), None, "{c:?} must not classify as a CJK script");
        }
    }

    #[test]
    fn script_runs_splits_mixed_text_by_script_boundary() {
        // "hi漢字ですaは" -- latin "hi", han "漢字", kana "です", latin "a", kana "は"
        let text = "hi漢字ですaは";
        let runs = script_runs(text);
        let tagged: Vec<(&str, Script)> = runs.iter().map(|(r, s)| (&text[r.clone()], *s)).collect();
        assert_eq!(
            tagged,
            vec![("漢字", Script::Han), ("です", Script::Kana), ("は", Script::Kana)],
            "runs: {tagged:?}"
        );
    }

    #[test]
    fn script_runs_empty_for_pure_latin() {
        assert!(script_runs("just some english prose").is_empty());
    }

    #[test]
    fn script_runs_boundary_with_no_gap_still_splits() {
        // Han immediately followed by Hangul with zero bytes between: two runs.
        let text = "漢한";
        let runs = script_runs(text);
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert_eq!(runs[0].1, Script::Han);
        assert_eq!(runs[1].1, Script::Hangul);
        assert_eq!(runs[0].0.end, runs[1].0.start, "no gap between the two runs");
    }

    #[test]
    fn dominant_cjk_kana_wins_even_with_han_present() {
        // Standard Japanese prose mixes kana + kanji; kana must win unambiguously.
        assert_eq!(dominant_cjk("これは漢字です"), Some(Script::Kana));
    }

    #[test]
    fn dominant_cjk_hangul_wins_even_with_han_present() {
        assert_eq!(dominant_cjk("한국어와 漢字"), Some(Script::Hangul));
    }

    #[test]
    fn dominant_cjk_bopomofo_hints_zh_hant_over_bare_han() {
        assert_eq!(dominant_cjk("國字ㄍㄨㄛˊ"), Some(Script::Bopomofo));
    }

    #[test]
    fn dominant_cjk_han_only_is_ambiguous_han() {
        assert_eq!(dominant_cjk("汉字漢字"), Some(Script::Han));
    }

    #[test]
    fn dominant_cjk_pure_latin_is_none() {
        assert_eq!(dominant_cjk("nothing but english here"), None);
        assert_eq!(dominant_cjk(""), None);
    }

    #[test]
    fn doc_lang_for_unambiguous_scripts_ignore_priority() {
        let priority = [Lang::Ko, Lang::ZhHant, Lang::ZhHans, Lang::Ja];
        assert_eq!(doc_lang_for(Script::Kana, &priority), Lang::Ja);
        assert_eq!(doc_lang_for(Script::Hangul, &priority), Lang::Ko);
        assert_eq!(doc_lang_for(Script::Bopomofo, &priority), Lang::ZhHant);
    }

    #[test]
    fn doc_lang_for_han_consults_priority_ladder() {
        assert_eq!(
            doc_lang_for(Script::Han, &[Lang::Ja, Lang::ZhHans, Lang::ZhHant, Lang::Ko]),
            Lang::Ja
        );
        assert_eq!(
            doc_lang_for(Script::Han, &[Lang::ZhHans, Lang::Ja, Lang::ZhHant, Lang::Ko]),
            Lang::ZhHans
        );
        assert_eq!(doc_lang_for(Script::Han, &[]), Lang::Ja, "empty ladder never panics, defaults ja");
    }

    #[test]
    fn resolve_font_id_ladder_step_a_doc_tag_wins_when_compatible() {
        let priority = crate::frontmatter::DEFAULT_CJK_PRIORITY;
        assert_eq!(resolve_font_id(Some(Lang::Ja), Some(Script::Kana), &priority), FontId::Ja);
        assert_eq!(resolve_font_id(Some(Lang::Ja), Some(Script::Han), &priority), FontId::Ja);
        assert_eq!(resolve_font_id(Some(Lang::ZhHans), Some(Script::Han), &priority), FontId::ZhHans);
    }

    #[test]
    fn resolve_font_id_ladder_step_b_incompatible_tag_falls_to_script() {
        // The exact scenario the task spec calls out: a ja-tagged doc with an
        // embedded hangul run. Step (a) has no ko mapping for a ja tag, so it
        // falls to (b): the run's OWN script (hangul -> ko).
        let priority = crate::frontmatter::DEFAULT_CJK_PRIORITY;
        assert_eq!(resolve_font_id(Some(Lang::Ja), Some(Script::Hangul), &priority), FontId::Ko);
    }

    #[test]
    fn resolve_font_id_ladder_step_c_untagged_han_uses_cjk_priority() {
        // No doc tag at all, a bare Han run: falls all the way to (c), the
        // cjk_priority tiebreak.
        assert_eq!(
            resolve_font_id(None, Some(Script::Han), &[Lang::ZhHant, Lang::Ja, Lang::ZhHans, Lang::Ko]),
            FontId::ZhHant
        );
        assert_eq!(
            resolve_font_id(None, Some(Script::Han), &crate::frontmatter::DEFAULT_CJK_PRIORITY),
            FontId::Ja,
            "default ladder is ja-first"
        );
    }

    #[test]
    fn resolve_font_id_ladder_step_d_no_script_is_latin_floor() {
        assert_eq!(
            resolve_font_id(Some(Lang::Ja), None, &crate::frontmatter::DEFAULT_CJK_PRIORITY),
            FontId::Latin
        );
        assert_eq!(resolve_font_id(None, None, &crate::frontmatter::DEFAULT_CJK_PRIORITY), FontId::Latin);
    }

    #[test]
    fn resolve_font_id_untagged_unambiguous_scripts_use_natural_mapping() {
        let priority = crate::frontmatter::DEFAULT_CJK_PRIORITY;
        assert_eq!(resolve_font_id(None, Some(Script::Kana), &priority), FontId::Ja);
        assert_eq!(resolve_font_id(None, Some(Script::Hangul), &priority), FontId::Ko);
        assert_eq!(resolve_font_id(None, Some(Script::Bopomofo), &priority), FontId::ZhHant);
    }

    #[test]
    fn resolve_font_id_en_tag_never_overrides_cjk() {
        // An `en`-tagged doc with a stray CJK run: `En` has no font_id_for_script
        // mapping, so it falls straight through to the run's own script.
        let priority = crate::frontmatter::DEFAULT_CJK_PRIORITY;
        assert_eq!(resolve_font_id(Some(Lang::En), Some(Script::Kana), &priority), FontId::Ja);
    }
}
