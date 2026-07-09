//! Spell-check engine (v1: detect only).
//!
//! Two responsibilities, kept apart so the markdown-aware tokenizer is unit-
//! testable without a real dictionary:
//!
//!   * [`SpellChecker`] — wraps a [`spellbook::Dictionary`] loaded ONCE from a
//!     bundled LibreOffice Hunspell pair (`include_str!`'d into the binary),
//!     exposing [`SpellChecker::check`] (a microsecond dict lookup).
//!   * [`misspelled_spans`] — the pure, dictionary-parameterized detector: given
//!     the whole document and a `check` predicate, it tokenizes into words and
//!     returns the MISSPELLED ones as `(line, start_col, end_col)` in CHAR
//!     columns (consistent with the advance-aware layout + selection rects), with
//!     markdown skipping of fenced code blocks, inline code, and URLs.
//!
//! CORRECTIONS. [`SpellChecker::suggest`] asks the Hunspell engine for ordered
//! replacement candidates for a word, and [`SpellChecker::suggest_at`] resolves
//! the misspelling the cursor is ON or ADJACENT to (via the pure
//! [`misspelling_at`]) and pairs it with those suggestions — the data the
//! summoned correction picker (Cmd-`;`) lists and the chosen one a single
//! undoable edit replaces.
//!
//! DICTIONARY VARIANTS: awl bundles THREE LibreOffice Hunspell pairs — en_US
//! (default), en_GB, en_AU — all `include_str!`'d into the binary (same
//! self-contained, no-external-files, deterministic-capture contract as the
//! original single dictionary). [`DictVariant`] is the picker/process-global
//! enum (mirroring [`crate::caret::CaretMode`]'s `ALL`/`label`/`from_label`
//! shape); [`SpellChecker::new`] now takes the variant to parse. Parsing a
//! ~50-100k-stem dictionary is a real one-time cost (tens of ms, not a render-
//! frame concern) — see `spell::tests::parse_cost_per_dictionary_variant` for
//! measured numbers — so a SWITCH reparses exactly ONCE, on commit (Enter),
//! never per navigating keystroke (see `overlay/`'s Dictionary picker: unlike
//! Theme/Caret it has NO live preview-on-move).

/// The bundled dictionary PAIRS (LibreOffice Hunspell), `include_str!`'d into
/// the binary so spell-check works with no external files and the headless
/// capture stays self-contained + deterministic. en_US is the historical
/// default (~49.5k stems); en_GB / en_AU are the same LibreOffice dictionary
/// family (license + READMEs alongside them in `assets/dict/`).
const AFF_US: &str = include_str!("../assets/dict/en_US.aff");
const DIC_US: &str = include_str!("../assets/dict/en_US.dic");
const AFF_GB: &str = include_str!("../assets/dict/en_GB.aff");
const DIC_GB: &str = include_str!("../assets/dict/en_GB.dic");
const AFF_AU: &str = include_str!("../assets/dict/en_AU.aff");
const DIC_AU: &str = include_str!("../assets/dict/en_AU.dic");

/// Which bundled Hunspell dictionary variant is active. A process-global
/// selectable enum, mirroring [`crate::caret::CaretMode`] exactly: [`ALL`](Self::ALL)
/// drives the picker corpus, [`label`](Self::label)/[`from_label`](Self::from_label)
/// round-trip the picker's display name, and [`active_variant`]/[`set_active_variant`]
/// are the process-global pair the live App AND the headless capture both read (so a
/// `--config` with `dictionary = "en_AU"` produces a capture using that dictionary
/// with no flags, exactly like `theme`/`caret_mode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DictVariant {
    EnUs,
    EnGb,
    EnAu,
}

impl DictVariant {
    /// Every selectable dictionary, in picker order. The DICTIONARY PICKER lists
    /// these three with their [`label`](Self::label)/[`description`](Self::description).
    pub const ALL: [DictVariant; 3] = [DictVariant::EnUs, DictVariant::EnGb, DictVariant::EnAu];

    fn as_u8(self) -> u8 {
        match self {
            DictVariant::EnUs => 0,
            DictVariant::EnGb => 1,
            DictVariant::EnAu => 2,
        }
    }

    /// The picker ROW title — the human name shown in the dictionary menu (and
    /// matched back via [`from_label`](Self::from_label)). The lower-case wire form
    /// (the config `dictionary = "…"` value) is [`crate::config::dictionary_name`].
    pub fn label(self) -> &'static str {
        match self {
            DictVariant::EnUs => "English (US)",
            DictVariant::EnGb => "English (UK)",
            DictVariant::EnAu => "English (Australia)",
        }
    }

    /// One quiet line describing the variant, drawn dim beside the name in the
    /// dictionary picker (the same right-column shape as the caret-style picker's
    /// descriptions).
    pub fn description(self) -> &'static str {
        match self {
            DictVariant::EnUs => "Hunspell en_US — American spelling",
            DictVariant::EnGb => "Hunspell en_GB — British spelling",
            DictVariant::EnAu => "Hunspell en_AU — Australian spelling",
        }
    }

    /// Resolve a picker ROW title ([`label`](Self::label)) back to its variant —
    /// the inverse, used by the dictionary picker's commit path. Case-insensitive;
    /// `None` for an unknown label.
    pub fn from_label(s: &str) -> Option<DictVariant> {
        Self::ALL.into_iter().find(|v| v.label().eq_ignore_ascii_case(s))
    }

    /// The bundled `(aff, dic)` source pair for this variant.
    fn files(self) -> (&'static str, &'static str) {
        match self {
            DictVariant::EnUs => (AFF_US, DIC_US),
            DictVariant::EnGb => (AFF_GB, DIC_GB),
            DictVariant::EnAu => (AFF_AU, DIC_AU),
        }
    }
}

/// The user's explicit dictionary-variant override. A process-global like the
/// active theme / caret mode; 0/1/2 map directly to [`DictVariant::ALL`] order
/// (unlike caret's "0 = auto" scheme, there is no font-derived default here —
/// absent config is simply `EnUs`, matching the sticky-pref contract).
static ACTIVE_VARIANT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// The SINGLE test mutex serializing every test that mutates a process-global in
/// this module ([`ACTIVE_VARIANT`] AND [`SPELLCHECK_ON`]) — mirrors
/// `caret::TEST_LOCK` / `page::test_lock()` / `nits::TEST_LOCK` (one lock per
/// module, covering every global it owns).
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The EFFECTIVE dictionary variant: the explicit override the picker / config /
/// `apply_sticky_globals` set, else [`DictVariant::EnUs`] (the built-in default).
pub fn active_variant() -> DictVariant {
    match ACTIVE_VARIANT.load(std::sync::atomic::Ordering::Relaxed) {
        1 => DictVariant::EnGb,
        2 => DictVariant::EnAu,
        _ => DictVariant::EnUs,
    }
}

/// Set the explicit dictionary-variant override (the picker's commit path, and
/// `apply_sticky_globals` restoring a remembered `dictionary` pref at launch).
pub fn set_active_variant(v: DictVariant) {
    ACTIVE_VARIANT.store(v.as_u8(), std::sync::atomic::Ordering::Relaxed);
}

/// Whether spell-check is active AT ALL — the GLOBAL escape hatch (default ON),
/// mirroring `nits::NITS_ON` exactly: a process-global read by the ONE owner seam
/// ([`SpellChecker::misspellings_for`] + [`SpellChecker::suggest_at`]) so OFF
/// silences every squiggle — prose comments and the scoped code-string/comment
/// check alike — and turns the spell-suggest picker into a calm no-op, with zero
/// duplicated gating at any call site (render, capture, the right-click seam).
static SPELLCHECK_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// True when spell-check is active (read by [`SpellChecker::misspellings_for`] /
/// [`SpellChecker::suggest_at`] before doing any work).
pub fn spellcheck_on() -> bool {
    SPELLCHECK_ON.load(std::sync::atomic::Ordering::Relaxed)
}

/// Set spell-check on/off explicitly (a config sticky-pref restore / the
/// "Toggle spellcheck" palette command's live flip).
pub fn set_spellcheck_on(on: bool) {
    SPELLCHECK_ON.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Flip spell-check and return the now-active state (the "Toggle spellcheck"
/// palette command). Mirrors [`crate::nits::toggle`] / [`crate::page::toggle`].
pub fn toggle() -> bool {
    let next = !spellcheck_on();
    SPELLCHECK_ON.store(next, std::sync::atomic::Ordering::Relaxed);
    next
}

/// A misspelled word's location in the document, in CHAR columns on a logical
/// line. `[start_col, end_col)` is a half-open char range; the renderer maps it
/// to pixels with the SAME advance-aware layout used for selection rects, so the
/// squiggle lands exactly under the word's glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Misspelling {
    pub line: usize,
    pub start_col: usize,
    pub end_col: usize,
}

/// Loaded-once spell checker. Holds the parsed Hunspell dictionary; `check` is a
/// pure lookup. Construction is the only fallible part (dictionary parse).
pub struct SpellChecker {
    dict: spellbook::Dictionary,
}

impl SpellChecker {
    /// Parse the bundled Hunspell dictionary for `variant`. Returns an error
    /// string if the real-world dictionary fails to parse (so the caller can
    /// REPORT it rather than silently disabling spell-check). This is the ONE
    /// real per-switch cost the dictionary picker pays (see `spell::tests`'s
    /// timed parse test) — never called on a mere navigation move.
    pub fn new(variant: DictVariant) -> Result<Self, String> {
        let (aff, dic) = variant.files();
        let dict = spellbook::Dictionary::new(aff, dic)
            .map_err(|e| format!("failed to parse bundled {} dictionary: {e}", variant.label()))?;
        Ok(Self { dict })
    }

    /// True if `word` is spelled correctly. Hunspell's `check` is already case
    /// aware (it honors capitalized / all-caps forms of dictionary stems and the
    /// dictionary's own proper-noun entries), so we pass the raw word; if the
    /// exact-case form is rejected we additionally accept an all-lowercase match
    /// so a sentence-initial capital of a lowercase-only stem (e.g. "Definately"
    /// vs "definitely") is judged on the stem, not the capitalization.
    pub fn check(&self, word: &str) -> bool {
        if self.dict.check(word) {
            return true;
        }
        let lower = word.to_lowercase();
        if lower != word && self.dict.check(&lower) {
            return true;
        }
        false
    }

    /// Detect all misspelled words in `text`. Thin wrapper over the pure
    /// [`misspelled_spans`] using this dictionary as the predicate.
    pub fn misspellings(&self, text: &str) -> Vec<Misspelling> {
        misspelled_spans(text, |w| self.check(w))
    }

    /// THE ONE OWNER of the spell scope: detect misspellings honoring the
    /// buffer's language. GATED FIRST on the GLOBAL [`spellcheck_on`] toggle — OFF
    /// returns empty unconditionally, so no squiggle survives anywhere (prose or
    /// code) once the user has switched it off. `lang == None` (prose / markdown /
    /// scratch) is [`SpellChecker::misspellings`] VERBATIM over the text PAST any
    /// leading frontmatter block ([`crate::frontmatter::detect`] — metadata, not
    /// manuscript, so a `lang: ja` key is never itself squiggled), with the
    /// result's `line` numbers shifted back up by the block's line count —
    /// otherwise byte-identical, keeping the existing markdown fence /
    /// inline-code / URL skips. `Some(lang)` (a recognized CODE buffer) spell-
    /// checks ONLY the prose regions the lexer already delimits: the PROSE-tier
    /// [`crate::syntax::SynKind::Comment`] spans VERBATIM, and the
    /// [`crate::syntax::SynKind::Str`] spans FURTHER GATED on
    /// [`looks_like_prose_string`] — a STRING squiggles only when its content
    /// reads as prose (multiple space-separated words); a single CODE-VOCABULARY
    /// token (`"struct"`, `"en_AU"`, a format specifier, a CSS selector) never
    /// does. Commented-out code (`CommentCode`), identifiers, keywords, and
    /// everything else can never squiggle. Every spell call site routes through
    /// here (app debounce, capture, framebench), so live + headless can't drift.
    pub fn misspellings_for(
        &self,
        text: &str,
        lang: Option<crate::syntax::Lang>,
    ) -> Vec<Misspelling> {
        if !spellcheck_on() {
            return Vec::new();
        }
        match lang {
            None => match crate::frontmatter::detect(text) {
                Some(fm) => {
                    let line_offset = text[..fm.range.end].matches('\n').count();
                    self.misspellings(&text[fm.range.end..])
                        .into_iter()
                        .map(|m| Misspelling { line: m.line + line_offset, ..m })
                        .collect()
                }
                None => self.misspellings(text),
            },
            Some(l) => {
                let mut ranges: Vec<std::ops::Range<usize>> = crate::syntax::spans(l, text)
                    .into_iter()
                    .filter(|(r, k)| match k {
                        crate::syntax::SynKind::Comment => true,
                        crate::syntax::SynKind::Str => {
                            text.get(r.clone()).is_some_and(looks_like_prose_string)
                        }
                        _ => false,
                    })
                    .map(|(r, _)| r)
                    .collect();
                ranges.sort_by_key(|r| r.start);
                misspelled_spans_scoped(text, |w| self.check(w), &ranges)
            }
        }
    }

    /// Ordered correction candidates for `word`, best first (Hunspell's own
    /// ranking). Empty when the engine has no suggestion. A thin wrapper over
    /// spellbook's `suggest`, owning the output vec so callers needn't manage one.
    pub fn suggest(&self, word: &str) -> Vec<String> {
        let mut out = Vec::new();
        self.dict.suggest(word, &mut out);
        out
    }

    /// Resolve the misspelling the cursor at `(line, col)` is ON or ADJACENT to and
    /// pair it with its correction candidates — the data the summoned spell picker
    /// lists. GATED FIRST on the GLOBAL [`spellcheck_on`] toggle — OFF returns
    /// `None` unconditionally, so `Cmd-;` / a right-click degrades to the same
    /// calm no-op the binding already promises for a correct word. `None` also
    /// when the cursor is not on a flagged word. The returned span is in CHAR
    /// columns on the logical line, so the caller can map it to a buffer char
    /// range for the replace-the-word edit.
    pub fn suggest_at(
        &self,
        text: &str,
        line: usize,
        col: usize,
        lang: Option<crate::syntax::Lang>,
    ) -> Option<SuggestionTarget> {
        if !spellcheck_on() {
            return None;
        }
        // Route through THE ONE OWNER of the spell scope ([`Self::misspellings_for`])
        // rather than a parallel UNSCOPED scan, so the suggest target and the DRAWN
        // squiggle can never disagree: in a CODE buffer an identifier/keyword the
        // scoped scan excludes is never offered a "correction" here either. The
        // spans arrive in document order, so the left-most one wins a column tie —
        // the same rule [`misspelling_at`] applies for prose.
        let m = self
            .misspellings_for(text, lang)
            .into_iter()
            .find(|m| m.line == line && col >= m.start_col && col <= m.end_col)?;
        let word: String = text
            .split('\n')
            .nth(m.line)
            .unwrap_or("")
            .chars()
            .skip(m.start_col)
            .take(m.end_col - m.start_col)
            .collect();
        let suggestions = self.suggest(&word);
        Some(SuggestionTarget {
            misspelling: m,
            word,
            suggestions,
        })
    }
}

/// A misspelled word the cursor sits on, with its ordered correction candidates.
/// Produced by [`SpellChecker::suggest_at`] for the summoned spell picker: the
/// `misspelling` span locates the word to replace, `word` is its current text, and
/// `suggestions` (possibly empty) are what the picker lists.
#[derive(Clone, Debug)]
pub struct SuggestionTarget {
    pub misspelling: Misspelling,
    /// The current (misspelled) word text. Carried for callers/tests that want to
    /// echo it; the live/headless pickers replace by SPAN, so the binary itself
    /// reads only `misspelling` + `suggestions`.
    #[allow(dead_code)]
    pub word: String,
    pub suggestions: Vec<String>,
}

/// The misspelled word the cursor at `(line, col)` is ON or ADJACENT to, if any.
/// "Adjacent" means the cursor sits anywhere in `[start_col, end_col]` INCLUSIVE
/// of both ends, so a caret just before the first letter or just after the last
/// letter still targets the word (typical when you finish typing a word). Pure
/// (the dictionary arrives via `check`) so it's unit-testable with a stub. When
/// two spans somehow touch the same column, the earlier (left-most) one wins.
///
/// Retained as the pure UNSCOPED targeting primitive (the `[start,end]`-inclusive
/// column rule, unit-tested directly); [`SpellChecker::suggest_at`] no longer calls
/// it — it targets via THE ONE OWNER [`SpellChecker::misspellings_for`] so suggest
/// and the drawn squiggle share one scope in a code buffer.
#[allow(dead_code)]
pub fn misspelling_at<F: Fn(&str) -> bool>(
    text: &str,
    line: usize,
    col: usize,
    check: F,
) -> Option<Misspelling> {
    misspelled_spans(text, check)
        .into_iter()
        .find(|m| m.line == line && col >= m.start_col && col <= m.end_col)
}

/// Is `c` a letter we spell-check? We only check Latin-script words for v1, so
/// CJK / other-script letters are treated as non-word here (a CJK run is skipped
/// entirely, never flagged). ASCII fast-path first.
fn is_latin_letter(c: char) -> bool {
    if c.is_ascii_alphabetic() {
        return true;
    }
    if !c.is_alphabetic() {
        return false;
    }
    // Accept the Latin-script blocks (Basic Latin handled above, plus Latin-1
    // supplement / extended and IPA) so accented Latin words (café, naïve) are
    // checked; everything else (CJK, Cyrillic, Greek, ...) is skipped.
    matches!(c as u32,
        0x00C0..=0x024F   // Latin-1 Supplement + Latin Extended-A/B
        | 0x1E00..=0x1EFF // Latin Extended Additional
    )
}

/// True for an apostrophe that may sit INSIDE a word (don't, it's). Both the
/// ASCII `'` and the typographic right single quote are accepted; the dictionary
/// stores apostrophe words with `'`.
fn is_intraword_apostrophe(c: char) -> bool {
    c == '\'' || c == '\u{2019}'
}

/// Tokenize `text` and return the MISSPELLED words as `(line, start_col,
/// end_col)` char spans, skipping markdown code + URLs. `check(word)` returns
/// true for a correctly spelled word. Pure (no I/O, no dictionary) so the
/// markdown heuristics + tokenization are unit-testable with a stub predicate.
///
/// Skipping rules (heuristic, good enough for v1):
///   * Fenced code blocks: a line whose trimmed text starts with ``` toggles a
///     "in code fence" state; lines inside are not checked (nor is the fence).
///   * Inline code: a backtic-delimited run `like this` on a line is skipped.
///   * URLs: a whitespace-delimited token starting http:// https:// or www. is
///     skipped wholesale (so `.../teh` is not flagged).
///   * Tokens containing a digit, or any non-Latin letter, are skipped.
pub fn misspelled_spans<F: Fn(&str) -> bool>(text: &str, check: F) -> Vec<Misspelling> {
    let mut out = Vec::new();
    let mut in_fence = false;

    // `text.split('\n')` yields one entry per logical line and is consistent
    // with how the buffer numbers lines (each '\n' ends a line). A trailing
    // newline yields a final empty line, which is harmless (no words).
    for (line_no, line) in text.split('\n').enumerate() {
        // Fenced code block toggle: a line that is just ``` (optionally with an
        // info string / indentation) flips the state. The fence line itself is
        // never spell-checked.
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        scan_line(line, line_no, &check, &mut out);
    }
    out
}

/// Scan a single (non-fence) line: skip inline-code and URL regions, then emit
/// misspelled word spans for the rest. Columns are CHAR indices into the line.
fn scan_line<F: Fn(&str) -> bool>(
    line: &str,
    line_no: usize,
    check: &F,
    out: &mut Vec<Misspelling>,
) {
    // Work in (char_index, char) units so emitted columns are char columns.
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();
    let mut i = 0usize;

    while i < n {
        let c = chars[i];

        // --- Inline code: skip from an opening backtick to its closing one. ---
        if c == '`' {
            i += 1;
            while i < n && chars[i] != '`' {
                i += 1;
            }
            // Consume the closing backtick if present.
            if i < n {
                i += 1;
            }
            continue;
        }

        // --- URL: skip a whole whitespace-delimited token that looks like one.
        if c.is_ascii_alphabetic() && url_at(&chars, i) {
            while i < n && !chars[i].is_whitespace() {
                i += 1;
            }
            continue;
        }

        // --- Word: a run of Latin letters with intra-word apostrophes. --------
        if is_latin_letter(c) {
            let start = i;
            // Track whether the run held any non-Latin letter or digit; if so we
            // skip it (mixed-script / alphanumeric token).
            let mut skip = false;
            while i < n {
                let ch = chars[i];
                if is_latin_letter(ch) {
                    i += 1;
                } else if is_intraword_apostrophe(ch)
                    && i + 1 < n
                    && is_latin_letter(chars[i + 1])
                {
                    // Apostrophe only counts as intra-word when a letter follows
                    // (so a trailing quote in `dogs'` ends the word cleanly).
                    i += 1;
                } else if ch.is_alphanumeric() {
                    // A digit or non-Latin letter glued to the run: consume the
                    // rest of the alnum run and mark it un-checkable.
                    skip = true;
                    i += 1;
                } else {
                    break;
                }
            }
            if skip {
                continue;
            }
            let word: String = chars[start..i].iter().collect();
            // Trim a possible trailing apostrophe (e.g. from "dogs'") before the
            // dictionary lookup; intra-word apostrophes are kept.
            let trimmed = word.trim_end_matches(|c| is_intraword_apostrophe(c));
            if trimmed.is_empty() {
                continue;
            }
            if !check(trimmed) {
                out.push(Misspelling {
                    line: line_no,
                    start_col: start,
                    end_col: start + trimmed.chars().count(),
                });
            }
            continue;
        }

        i += 1;
    }
}

/// SCOPED detection for CODE buffers: run the SAME tokenizer as
/// [`misspelled_spans`], then keep only the words whose DOCUMENT BYTE range
/// falls FULLY inside one of `prose_ranges` (the lexer-delimited prose regions —
/// prose comments + strings; ranges must be sorted by start, non-overlapping is
/// not required but typical). Scoped mode additionally drops IDENTIFIER-SHAPED
/// words ([`identifier_shaped`]) so `SelInstance` / `WGSL` / `px` never squiggle
/// even inside a comment or string. Pure (dictionary via `check`); prose buffers
/// never take this path, so their output is untouched. Line byte offsets come
/// from ONE running `split('\n')` walk and words arrive in document order, so
/// the range merge is a two-pointer O(doc) pass — fine for a debounced scan.
pub fn misspelled_spans_scoped<F: Fn(&str) -> bool>(
    text: &str,
    check: F,
    prose_ranges: &[std::ops::Range<usize>],
) -> Vec<Misspelling> {
    let all = misspelled_spans(text, check);
    if all.is_empty() || prose_ranges.is_empty() {
        return Vec::new();
    }
    debug_assert!(
        prose_ranges.windows(2).all(|w| w[0].start <= w[1].start),
        "prose_ranges must be sorted by start"
    );
    // Line starts from one running walk; per-line text for char->byte cols.
    let lines: Vec<&str> = text.split('\n').collect();
    let mut line_starts: Vec<usize> = Vec::with_capacity(lines.len());
    let mut acc = 0usize;
    for l in &lines {
        line_starts.push(acc);
        acc += l.len() + 1;
    }
    let mut out = Vec::new();
    let mut ri = 0usize;
    for m in all {
        let Some(line) = lines.get(m.line) else { continue };
        // Char col -> line-local byte offset (chars can be multi-byte).
        let byte_at = |col: usize| {
            line.char_indices()
                .nth(col)
                .map(|(b, _)| b)
                .unwrap_or(line.len())
        };
        let lo = line_starts[m.line] + byte_at(m.start_col);
        let hi = line_starts[m.line] + byte_at(m.end_col);
        // Two-pointer: drop ranges that end before this word can fit. Words are
        // disjoint + ascending, so a range too short for THIS word is too short
        // for every later one.
        while ri < prose_ranges.len() && prose_ranges[ri].end < hi {
            ri += 1;
        }
        let inside = ri < prose_ranges.len()
            && prose_ranges[ri].start <= lo
            && hi <= prose_ranges[ri].end;
        if !inside {
            continue;
        }
        let word: String = line.chars().skip(m.start_col).take(m.end_col - m.start_col).collect();
        if identifier_shaped(&word) {
            continue; // SelInstance / WGSL / px — code vocabulary, never a typo
        }
        out.push(m);
    }
    out
}

/// Does a STRING LITERAL's content read as PROSE rather than a single code
/// token? Mirrors [`crate::syntax::looks_like_code`]'s shape — a small, pure,
/// DEFAULT-TO-SKIP heuristic: PROSE iff the trimmed body holds AT LEAST TWO
/// space-separated tokens that each carry a Latin letter ("hello world", "Item
/// not found" — an ordinary English phrase, incl. one with a `{placeholder}`
/// mixed in, still reads as prose and gets checked word-by-word). A SINGLE
/// token — `"struct"`, `"en_AU"`, a bare format specifier (`"{}"`, `"%d"`), a
/// CSS selector (`".foo-bar"`) — is CODE VOCABULARY, not prose, and the WHOLE
/// string is skipped (no word inside it is even considered, so a bare
/// non-English identifier never gets a chance to look like a typo). An empty
/// string, or one with fewer than two word-shaped tokens, is not prose either
/// (DEFAULT-TO-SKIP, same posture as `looks_like_code`'s DEFAULT-TO-PROSE:
/// when unsure, this heuristic prefers silence over a false-positive squiggle
/// on code vocabulary).
fn looks_like_prose_string(body: &str) -> bool {
    body.split_whitespace()
        .filter(|tok| tok.chars().any(|c| c.is_alphabetic()))
        .count()
        >= 2
}

/// True for a word that reads as CODE VOCABULARY rather than prose — the scoped
/// mode's post-filter (prose buffers never see this): ALL-CAPS of length ≥ 2
/// (`WGSL`), an INTERIOR uppercase (CamelCase — `SelInstance`; a plain
/// sentence-initial capital stays checkable), an underscore, or anything
/// shorter than 3 chars (`px`, `en`-style fragments).
fn identifier_shaped(word: &str) -> bool {
    let n = word.chars().count();
    if n < 3 || word.contains('_') {
        return true;
    }
    if n >= 2 && word.chars().all(|c| !c.is_alphabetic() || c.is_uppercase()) {
        return true;
    }
    word.chars().skip(1).any(|c| c.is_uppercase())
}

/// Does a URL scheme/prefix begin at char index `i`? Matches `http://`,
/// `https://`, or `www.` case-insensitively against the char slice.
fn url_at(chars: &[char], i: usize) -> bool {
    const PREFIXES: &[&str] = &["https://", "http://", "www."];
    for p in PREFIXES {
        let pc: Vec<char> = p.chars().collect();
        if i + pc.len() <= chars.len() {
            let mut ok = true;
            for (k, &want) in pc.iter().enumerate() {
                if !chars[i + k].eq_ignore_ascii_case(&want) {
                    ok = false;
                    break;
                }
            }
            if ok {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub dictionary: only these exact lowercase words are "correct".
    fn stub<'a>(correct: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |w: &str| correct.iter().any(|c| c.eq_ignore_ascii_case(w))
    }

    fn cols(m: &Misspelling) -> (usize, usize, usize) {
        (m.line, m.start_col, m.end_col)
    }

    #[test]
    fn flags_a_single_bad_word() {
        let good = stub(&["hello", "world"]);
        let ms = misspelled_spans("hello wrld", &good);
        assert_eq!(ms.len(), 1);
        assert_eq!(cols(&ms[0]), (0, 6, 10)); // "wrld" at cols 6..10
    }

    #[test]
    fn correct_words_not_flagged() {
        let good = stub(&["the", "quick", "brown", "fox"]);
        assert!(misspelled_spans("the quick brown fox", &good).is_empty());
    }

    #[test]
    fn columns_are_char_indices_after_punctuation() {
        let good = stub(&["a", "test"]);
        // "a, tset." -> "tset"? here we test column math with punctuation.
        let ms = misspelled_spans("a, tset.", &good);
        assert_eq!(ms.len(), 1);
        // "tset" starts at char col 3 (a=0, ,=1, space=2, t=3) and is 4 chars.
        assert_eq!(cols(&ms[0]), (0, 3, 7));
    }

    #[test]
    fn intraword_apostrophe_kept_as_one_word() {
        let good = stub(&["don't", "it's"]);
        assert!(misspelled_spans("don't it's", &good).is_empty());
        // A bad contraction is flagged as a single span including the apostrophe.
        let bad = stub(&["it's"]);
        let ms = misspelled_spans("dont", &bad);
        assert_eq!(ms.len(), 1);
        assert_eq!(cols(&ms[0]), (0, 0, 4));
    }

    #[test]
    fn trailing_apostrophe_trimmed() {
        // "dogs'" (possessive) should check the stem "dogs", not "dogs'".
        let good = stub(&["dogs"]);
        let ms = misspelled_spans("dogs' bones", &good);
        // "bones" is not in the stub -> flagged; "dogs'" is trimmed to "dogs".
        assert_eq!(ms.iter().filter(|m| m.start_col == 0).count(), 0);
    }

    #[test]
    fn digits_make_a_token_unchecked() {
        let none = stub(&[]); // nothing is correct
        // tokens with digits are skipped entirely, so no flags despite empty dict
        assert!(misspelled_spans("abc123 x2 v8", &none).is_empty());
    }

    #[test]
    fn cjk_run_is_skipped() {
        let none = stub(&[]);
        // Japanese should never be flagged (non-Latin script).
        assert!(misspelled_spans("日本語のテスト", &none).is_empty());
        // Mixed: only the Latin word "bad" is considered.
        let ms = misspelled_spans("日本 bad", &none);
        assert_eq!(ms.len(), 1);
        // "bad" starts after "日本 " -> char col 3.
        assert_eq!(ms[0].start_col, 3);
    }

    #[test]
    fn inline_code_is_skipped() {
        let none = stub(&[]);
        // The word inside backticks must NOT be flagged.
        let ms = misspelled_spans("use `wgpu` here", &none);
        // "use" and "here" are flagged (empty dict); "wgpu" is NOT.
        assert!(ms.iter().all(|m| {
            let w_start = m.start_col;
            w_start != 5 // wgpu would start at col 5
        }));
        assert_eq!(ms.len(), 2);
    }

    #[test]
    fn fenced_code_block_is_skipped() {
        let none = stub(&[]);
        let text = "before\n```\nnonsenseword\n```\nafter";
        let ms = misspelled_spans(text, &none);
        // Only "before" (line 0) and "after" (line 4) are checked; the fenced
        // line 2 "nonsenseword" is skipped.
        let lines: Vec<usize> = ms.iter().map(|m| m.line).collect();
        assert!(lines.contains(&0));
        assert!(lines.contains(&4));
        assert!(!lines.contains(&2), "fenced word must be skipped");
    }

    #[test]
    fn url_is_skipped() {
        let none = stub(&[]);
        // The misspelling embedded in the URL ("teh") must NOT be flagged.
        let ms = misspelled_spans("see https://example.com/teh ok", &none);
        // "see" and "ok" are flagged; nothing from the URL.
        assert_eq!(ms.len(), 2);
        let words: Vec<usize> = ms.iter().map(|m| m.start_col).collect();
        assert_eq!(words, vec![0, 28]); // "see"@0, "ok"@28
    }

    #[test]
    fn www_url_is_skipped() {
        let none = stub(&["go", "to"]);
        let ms = misspelled_spans("go to www.bad-spelll.com", &none);
        assert!(ms.is_empty(), "www. URL must be skipped");
    }

    // --- Real dictionary smoke tests (parse + known good/bad words). --------

    #[test]
    fn real_dictionary_parses_and_checks_known_words() {
        let sc = SpellChecker::new(DictVariant::EnUs).expect("bundled en_US dictionary must parse");
        // Known-good words.
        for w in ["sentence", "misspelled", "typo", "definitely", "receive",
                  "the", "quick", "brown", "fox", "hello"] {
            assert!(sc.check(w), "{w:?} should be correct");
        }
        // Known-bad words (the fixture's deliberate misspellings).
        for w in ["sentance", "mispelled", "tpyo", "definately", "recieve"] {
            assert!(!sc.check(w), "{w:?} should be flagged");
        }
    }

    #[test]
    fn real_dictionary_handles_capitalization() {
        let sc = SpellChecker::new(DictVariant::EnUs).unwrap();
        // Sentence-initial capital of a lowercase stem is accepted.
        assert!(sc.check("Hello"));
        assert!(sc.check("The"));
        // ...but a genuinely misspelled capitalized word is still flagged.
        assert!(!sc.check("Definately"));
    }

    #[test]
    fn real_dictionary_on_fixture_finds_exactly_the_five() {
        let sc = SpellChecker::new(DictVariant::EnUs).unwrap();
        let text = "This sentance has a few mispelled words in it.\n\
                    Inline code like `wgpu` and `cosmic_text` must NOT be flagged.\n\
                    ```\nfn main() { let zzz = nonsenseword; }\n```\n\
                    A link https://example.com/teh should be skipped too.\n\
                    Another tpyo here, definately and recieve.";
        let ms = sc.misspellings(text);
        let words: Vec<String> = ms
            .iter()
            .map(|m| {
                let line = text.split('\n').nth(m.line).unwrap();
                line.chars().skip(m.start_col).take(m.end_col - m.start_col).collect()
            })
            .collect();
        assert_eq!(
            words,
            vec!["sentance", "mispelled", "tpyo", "definately", "recieve"],
            "exactly the five deliberate misspellings, nothing from code/URL"
        );
    }

    // --- JAPANESE PINNING (real dictionary): the scanner is ASCII/Latin-word-
    // based ([`is_latin_letter`]), not a language detector — it never even LOOKS
    // at a CJK run, so genuine Japanese prose can never squiggle no matter how
    // "wrong" it might read to a Latin dictionary. Pinned against the REAL
    // bundled en_US dictionary (not a stub), both for prose (`lang == None`) and
    // for a CODE buffer's scoped comment/string scan (`misspellings_for`), so a
    // future change to either path can't quietly start flagging kanji/kana. ---

    #[test]
    fn real_dictionary_never_squiggles_pure_japanese_prose() {
        let sc = SpellChecker::new(DictVariant::EnUs).unwrap();
        // A real, ordinary Japanese sentence — nothing "misspelled" about it, but
        // the point is the scanner never even considers it (no Latin letters).
        let text = "今日は天気がいいですね。散歩に行きましょう。";
        assert!(sc.misspellings(text).is_empty(), "pure JP prose must never squiggle");
        // Same guarantee through the buffer-aware entry point every render/capture
        // call site actually uses.
        assert!(sc.misspellings_for(text, None).is_empty());
        // ...and identically for a JP comment inside a recognized code buffer (the
        // scoped comment/string path), so JP developer comments never squiggle.
        let code = format!("// {text}\nfn f() {{}}\n");
        assert!(sc.misspellings_for(&code, Some(crate::syntax::Lang::Rust)).is_empty());
    }

    #[test]
    fn real_dictionary_mixed_japanese_and_english_only_flags_the_english_word() {
        let sc = SpellChecker::new(DictVariant::EnUs).unwrap();
        // A Japanese sentence with one embedded, genuinely misspelled English
        // word: only that Latin word is ever a candidate — the JP text around it
        // is invisible to the tokenizer.
        let text = "今日は良い天気です recieve 頑張りましょう。";
        let ms = sc.misspellings(text);
        let words: Vec<String> = ms
            .iter()
            .map(|m| text.chars().skip(m.start_col).take(m.end_col - m.start_col).collect())
            .collect();
        assert_eq!(words, vec!["recieve"], "only the embedded English typo flags: {words:?}");
        // A correctly-spelled English word embedded the same way flags nothing.
        let clean = "今日は良い天気です hello 頑張りましょう。";
        assert!(sc.misspellings(clean).is_empty(), "a correct embedded English word is silent");
    }

    // --- Dictionary VARIANTS (en_US / en_GB / en_AU). ------------------------

    /// All three bundled dictionaries parse and answer a shared known-good word.
    /// "Never fabricate dictionary content" is enforced upstream (the files are
    /// the real LibreOffice downloads); this is the in-repo guarantee that they
    /// stay parseable as spellbook (or the bundled files) evolve.
    #[test]
    fn all_three_bundled_dictionaries_parse() {
        for v in DictVariant::ALL {
            let sc = SpellChecker::new(v).unwrap_or_else(|e| panic!("{}: {e}", v.label()));
            assert!(sc.check("hello"), "{}: a universally-shared word must check", v.label());
        }
    }

    /// The whole point of shipping en_GB/en_AU: British/Australian spellings
    /// ("colour", "organise") are WRONG in en_US but correct in the other two —
    /// proves the three dictionaries are genuinely distinct, not the same file
    /// three times over.
    #[test]
    fn variants_disagree_on_british_spelling() {
        let us = SpellChecker::new(DictVariant::EnUs).unwrap();
        let gb = SpellChecker::new(DictVariant::EnGb).unwrap();
        let au = SpellChecker::new(DictVariant::EnAu).unwrap();
        assert!(!us.check("colour"), "en_US should reject the British spelling");
        assert!(gb.check("colour"), "en_GB should accept it");
        assert!(au.check("colour"), "en_AU should accept it");
        assert!(us.check("color"), "en_US should accept its own spelling");
    }

    /// MEASURE + REPORT the one-time parse cost per dictionary (queue item ask):
    /// printed via `eprintln!` (visible with `cargo test -- --nocapture`), not
    /// asserted against a hard budget — a parse is a discrete picker-commit
    /// event, not a per-frame render cost, so there is no fps-style regression
    /// gate here (see `spell.rs`'s module doc + the dictionary picker's
    /// no-live-preview design).
    #[test]
    fn parse_cost_per_dictionary_variant() {
        for v in DictVariant::ALL {
            let t0 = std::time::Instant::now();
            let sc = SpellChecker::new(v).unwrap();
            let elapsed = t0.elapsed();
            eprintln!("spell dictionary parse {}: {:.2}ms", v.label(), elapsed.as_secs_f64() * 1000.0);
            assert!(sc.check("the"), "a parsed dictionary must still answer lookups");
        }
    }

    /// `label`/`from_label` round-trip for every variant (mirrors
    /// `caret::CaretMode`'s `from_label` test), case-insensitively.
    #[test]
    fn dict_variant_label_round_trips() {
        for v in DictVariant::ALL {
            assert_eq!(DictVariant::from_label(v.label()), Some(v));
        }
        assert_eq!(DictVariant::from_label("english (us)"), Some(DictVariant::EnUs));
        assert_eq!(DictVariant::from_label("nonsense"), None);
    }

    #[test]
    fn active_variant_defaults_to_en_us_and_round_trips_through_the_global() {
        // Serialize against the process-global; restore it so other tests (and a
        // re-run of this one) see the documented default.
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = active_variant();
        set_active_variant(DictVariant::EnUs);
        assert_eq!(active_variant(), DictVariant::EnUs, "absent override defaults to en_US");
        set_active_variant(DictVariant::EnGb);
        assert_eq!(active_variant(), DictVariant::EnGb);
        set_active_variant(DictVariant::EnAu);
        assert_eq!(active_variant(), DictVariant::EnAu);
        set_active_variant(saved);
    }

    // --- Scoped detection (code buffers spell-check comments + strings only). --

    #[test]
    fn scoped_keeps_only_words_fully_inside_prose_ranges() {
        let none = stub(&[]); // empty dict: every word flags — the SCOPE decides
        //           0123456789012345678
        let text = "alpha \"beta\" gamma";
        // Only the quoted region (bytes 6..12) is prose-checkable.
        let ms = misspelled_spans_scoped(text, &none, &[6..12]);
        assert_eq!(ms.len(), 1, "only the in-range word survives: {ms:?}");
        assert_eq!(cols(&ms[0]), (0, 7, 11)); // "beta"
        // A word STRADDLING a range boundary is not fully inside -> dropped.
        let ms = misspelled_spans_scoped(text, &none, &[6..9]);
        assert!(ms.is_empty(), "a straddling word must not squiggle");
        // No ranges -> nothing can squiggle.
        assert!(misspelled_spans_scoped(text, &none, &[]).is_empty());
    }

    #[test]
    fn scoped_drops_identifier_shaped_words() {
        let none = stub(&[]);
        let text = "\"SelInstance WGSL px some_var word\"";
        let ms = misspelled_spans_scoped(text, &none, &[0..text.len()]);
        // CamelCase, ALL-CAPS, <3 chars and snake_case all pass silently; only
        // the plain word squiggles. (The tokenizer splits `some_var` at the `_`,
        // so its halves are plain runs — `var` is dropped by nothing... but
        // `some` and `var` are lowercase words and DO flag; the shape filter is
        // about casing/length, not underscores post-split.)
        let words: Vec<String> = ms
            .iter()
            .map(|m| text.chars().skip(m.start_col).take(m.end_col - m.start_col).collect())
            .collect();
        assert!(!words.iter().any(|w| w == "SelInstance"), "CamelCase never squiggles");
        assert!(!words.iter().any(|w| w == "WGSL"), "ALL-CAPS never squiggles");
        assert!(!words.iter().any(|w| w == "px"), "short fragments never squiggle");
        assert!(words.iter().any(|w| w == "word"), "a plain prose word still checks: {words:?}");
    }

    #[test]
    fn misspellings_for_none_is_exactly_the_unscoped_scan() {
        // PROSE BYTE-IDENTITY: `lang == None` must equal `misspellings` by value,
        // including the markdown fence / inline-code / URL skips.
        let sc = SpellChecker::new(DictVariant::EnUs).unwrap();
        let text = "This sentance has a typo.\n```\nfenced zzz\n```\nsee `wgpu` and www.x.com ok";
        assert_eq!(sc.misspellings_for(text, None), sc.misspellings(text));
    }

    #[test]
    fn misspellings_for_excludes_a_leading_frontmatter_block() {
        // i18n: a frontmatter block is metadata, not manuscript — its own text
        // is never spell-checked, and the BODY's misspellings still land at
        // the correct line (shifted UP by the block's line count).
        let sc = SpellChecker::new(DictVariant::EnUs).unwrap();
        // "notalang" would itself misspell if scanned; the body's "sentance"
        // (line 0 of the body, line 3 of the whole doc) must still be found.
        let text = "---\nlang: notalang\n---\nThis sentance has a typo.\n";
        let ms = sc.misspellings_for(text, None);
        assert!(
            ms.iter().all(|m| m.line >= 3),
            "no misspelling may fall inside the frontmatter block: {ms:?}"
        );
        assert!(
            ms.iter().any(|m| m.line == 3),
            "the body's own misspelling still lands at its correct (shifted) line: {ms:?}"
        );
        // A document with NO frontmatter is unaffected (byte-identical).
        let plain = "This sentance has a typo.\n";
        assert_eq!(sc.misspellings_for(plain, None), sc.misspellings(plain));
    }

    #[test]
    fn misspellings_for_scopes_code_buffers_to_comments_and_strings() {
        let sc = SpellChecker::new(DictVariant::EnUs).unwrap();
        // A rust buffer: a typo in a PROSE comment, a typo in a STRING, an
        // un-word identifier, code vocabulary in a comment, and a typo inside
        // COMMENTED-OUT CODE (which must stay silent).
        let text = "// This sentance explains the plan.\n\
                    // SelInstance WGSL px sizes here.\n\
                    fn zzxqv() { let s = \"definately a typo\"; }\n\
                    // let recieve = 1;\n";
        let ms = sc.misspellings_for(text, Some(crate::syntax::Lang::Rust));
        let words: Vec<String> = ms
            .iter()
            .map(|m| {
                let line = text.split('\n').nth(m.line).unwrap();
                line.chars().skip(m.start_col).take(m.end_col - m.start_col).collect()
            })
            .collect();
        assert_eq!(
            words,
            vec!["sentance", "definately"],
            "comment + string typos flag; identifiers / code vocabulary / \
             commented-out code never do"
        );
    }

    #[test]
    fn suggest_at_honors_code_scope_matching_the_drawn_squiggle() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = spellcheck_on();
        set_spellcheck_on(true);
        let sc = SpellChecker::new(DictVariant::EnUs).unwrap();
        // A rust buffer: a misspelled DEFINITION identifier (bare code — the
        // scoped squiggle skips it), a prose typo inside a STRING literal, and a
        // prose typo inside a COMMENT. suggest must agree with the squiggle.
        let text = "fn zzxqv() { let s = \"definately a typo\"; }\n// This sentance explains.\n";
        let line0 = text.split('\n').next().unwrap();
        let ident_col = line0.find("zzxqv").unwrap() + 1; // inside the identifier (ASCII)
        // The identifier has no squiggle in a code buffer, so suggest offers no
        // correction — even though it IS a nonsense word to the dictionary.
        assert!(
            sc.suggest_at(text, 0, ident_col, Some(crate::syntax::Lang::Rust)).is_none(),
            "a bare code identifier has no squiggle, so suggest is a no-op there"
        );
        // Scope is the difference: UNSCOPED (a prose buffer) the same word IS a
        // targetable misspelling — proving suggest isn't silent because it's
        // spelled right.
        assert!(
            sc.suggest_at(text, 0, ident_col, None).is_some(),
            "unscoped, the same identifier is a normal misspelling"
        );
        // A real prose typo inside a STRING still resolves a suggestion.
        let str_col = line0.find("definately").unwrap() + 1;
        let t = sc
            .suggest_at(text, 0, str_col, Some(crate::syntax::Lang::Rust))
            .expect("a prose typo in a string still suggests");
        assert!(t.suggestions.iter().any(|w| w == "definitely"));
        // A real prose typo inside a COMMENT still resolves a suggestion.
        let line1 = text.split('\n').nth(1).unwrap();
        let com_col = line1.find("sentance").unwrap() + 1;
        let t = sc
            .suggest_at(text, 1, com_col, Some(crate::syntax::Lang::Rust))
            .expect("a prose typo in a comment still suggests");
        assert!(t.suggestions.iter().any(|w| w == "sentence"));
        set_spellcheck_on(saved);
    }

    #[test]
    fn suggest_at_excludes_a_frontmatter_block_matching_the_squiggle() {
        // A misspelled VALUE inside a `---` frontmatter block draws no squiggle
        // (`misspellings_for` strips the block), so suggest — routed through that
        // SAME one owner — must offer no target there either, while the BODY's own
        // typo still resolves. This is the suggest/squiggle-agree contract for
        // metadata (the code-scope analog of `suggest_at_honors_code_scope`).
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = spellcheck_on();
        set_spellcheck_on(true);
        let sc = SpellChecker::new(DictVariant::EnUs).unwrap();
        // "notalang" would itself misspell if scanned; "sentance" is the body typo.
        let text = "---\nlang: notalang\n---\nThis sentance has a typo.\n";
        // The frontmatter value has no squiggle, so suggest is a no-op on it.
        let fm_col = "lang: ".chars().count() + 1; // inside "notalang" on line 1
        assert!(
            sc.suggest_at(text, 1, fm_col, None).is_none(),
            "a typo inside a frontmatter block has no squiggle, so suggest is silent"
        );
        // The BODY's own typo (line 3, shifted past the block) still resolves.
        let body_col = "This ".chars().count() + 1; // inside "sentance" on line 3
        let t = sc
            .suggest_at(text, 3, body_col, None)
            .expect("the body's own typo still suggests");
        assert!(t.suggestions.iter().any(|w| w == "sentence"));
        set_spellcheck_on(saved);
    }

    // --- STRING PROSE GATE (scoped code-string spans). ----------------------

    #[test]
    fn looks_like_prose_string_needs_two_word_shaped_tokens() {
        // DEFAULT-TO-SKIP: empty, or a single code-vocabulary token.
        assert!(!looks_like_prose_string(""));
        assert!(!looks_like_prose_string("struct"));
        assert!(!looks_like_prose_string("en_AU"));
        assert!(!looks_like_prose_string("{}"), "a bare format placeholder is one token");
        assert!(!looks_like_prose_string("%d"), "a bare format specifier is one token");
        assert!(!looks_like_prose_string(".foo-bar"), "a CSS selector is one token");
        // PROSE: two or more space-separated word-shaped tokens.
        assert!(looks_like_prose_string("hello world"));
        assert!(looks_like_prose_string("Item {name} not found"), "a sentence with a placeholder is still prose");
    }

    #[test]
    fn string_prose_gate_silences_single_token_code_strings() {
        // ACCEPTANCE CASE (queue item): `syntax::rust::DEF_KEYWORDS`/`CONST_WORDS`
        // are each an ARRAY OF SEPARATE single-token string literals — the exact
        // shape of the reported Currawong-screenshot bug ("struct"/"const"
        // squiggled). None of these bare code-vocabulary tokens are English
        // dictionary words, so before the string-prose gate they all flagged;
        // afterward NONE do, because each lone-token string is skipped wholesale.
        let sc = SpellChecker::new(DictVariant::EnUs).unwrap();
        let text = "const DEF_KEYWORDS: &[&str] = &[\n    \
                    \"fn\", \"struct\", \"enum\", \"trait\", \"type\", \"union\", \
                    \"const\", \"static\", \"mod\",\n];\n\
                    const CONST_WORDS: &[&str] = &[\"true\", \"false\", \"None\"];\n";
        let ms = sc.misspellings_for(text, Some(crate::syntax::Lang::Rust));
        assert!(ms.is_empty(), "single-token code-vocabulary strings must never squiggle: {ms:?}");
    }

    #[test]
    fn string_prose_gate_keeps_the_real_rust_lexer_silent() {
        // Belt-and-suspenders on the REAL file: scan `syntax/rust.rs`'s own source
        // as a Rust buffer and confirm none of its DEF_KEYWORDS/CONST_WORDS bare
        // tokens ever appear among the reported misspellings.
        let sc = SpellChecker::new(DictVariant::EnUs).unwrap();
        let text = include_str!("syntax/rust.rs");
        let ms = sc.misspellings_for(text, Some(crate::syntax::Lang::Rust));
        let flagged: Vec<String> = ms
            .iter()
            .map(|m| {
                let line = text.split('\n').nth(m.line).unwrap();
                line.chars().skip(m.start_col).take(m.end_col - m.start_col).collect()
            })
            .collect();
        for kw in [
            "fn", "struct", "enum", "trait", "type", "union", "const", "static", "mod", "true",
            "false", "None",
        ] {
            assert!(
                !flagged.iter().any(|w| w == kw),
                "{kw:?} must never squiggle as a bare code-vocabulary string: {flagged:?}"
            );
        }
    }

    #[test]
    fn misspellings_for_still_checks_multi_word_prose_strings_in_code() {
        // The gate is a PROSE filter, not an off switch for strings wholesale: a
        // genuine English phrase inside a string still checks word-by-word.
        let sc = SpellChecker::new(DictVariant::EnUs).unwrap();
        let text = "fn f() { let msg = \"this has a typo teh\"; }\n";
        let ms = sc.misspellings_for(text, Some(crate::syntax::Lang::Rust));
        let words: Vec<String> = ms
            .iter()
            .map(|m| {
                let line = text.split('\n').nth(m.line).unwrap();
                line.chars().skip(m.start_col).take(m.end_col - m.start_col).collect()
            })
            .collect();
        assert_eq!(words, vec!["teh"], "a genuine multi-word prose string still checks: {words:?}");
    }

    // --- GLOBAL SPELLCHECK TOGGLE. -------------------------------------------

    #[test]
    fn spellcheck_defaults_on_and_toggle_flips_it() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = spellcheck_on();
        set_spellcheck_on(true);
        assert!(spellcheck_on(), "absent override defaults ON");
        assert!(!toggle(), "toggle flips ON -> off and returns the new state");
        assert!(!spellcheck_on());
        assert!(toggle(), "toggle flips off -> ON and returns the new state");
        assert!(spellcheck_on());
        set_spellcheck_on(saved);
    }

    #[test]
    fn spellcheck_off_silences_misspellings_for_everywhere_prose_and_code() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = spellcheck_on();
        set_spellcheck_on(true);
        let sc = SpellChecker::new(DictVariant::EnUs).unwrap();
        let prose = "This sentance has a typo.";
        let code = "// This sentance explains the plan.\nfn f() { let s = \"a typo teh here\"; }\n";
        assert!(!sc.misspellings_for(prose, None).is_empty(), "on: prose still detects");
        assert!(
            !sc.misspellings_for(code, Some(crate::syntax::Lang::Rust)).is_empty(),
            "on: scoped code still detects"
        );
        set_spellcheck_on(false);
        assert!(sc.misspellings_for(prose, None).is_empty(), "off: prose is silent too");
        assert!(
            sc.misspellings_for(code, Some(crate::syntax::Lang::Rust)).is_empty(),
            "off: scoped code is silent too"
        );
        set_spellcheck_on(saved);
    }

    #[test]
    fn spellcheck_off_makes_suggest_at_a_calm_no_op() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = spellcheck_on();
        set_spellcheck_on(true);
        let sc = SpellChecker::new(DictVariant::EnUs).unwrap();
        let text = "Please recieve this.";
        assert!(sc.suggest_at(text, 0, 9, None).is_some(), "on: a misspelling still resolves");
        set_spellcheck_on(false);
        assert!(sc.suggest_at(text, 0, 9, None).is_none(), "off: the same cursor is now a calm no-op");
        set_spellcheck_on(saved);
    }

    // --- Suggestions + cursor-targeting. ------------------------------------

    #[test]
    fn misspelling_at_targets_word_under_or_adjacent_to_cursor() {
        let good = stub(&["the", "quick"]);
        // "the wrld here" — "wrld" spans cols 4..8 (w=4,r=5,l=6,d=7).
        let text = "the wrld here";
        // ON the word (col inside the span).
        let m = misspelling_at(text, 0, 5, &good).expect("cursor in word");
        assert_eq!((m.start_col, m.end_col), (4, 8));
        // ADJACENT on the left edge (caret just before 'w').
        assert!(misspelling_at(text, 0, 4, &good).is_some());
        // ADJACENT on the right edge (caret just after 'd').
        assert!(misspelling_at(text, 0, 8, &good).is_some());
        // NOT on a flagged word: col 1 sits inside the correctly-spelled "the".
        assert!(misspelling_at(text, 0, 1, &good).is_none());
        // A line with no misspelling at all -> None.
        assert!(misspelling_at("the quick", 0, 2, &good).is_none());
    }

    #[test]
    fn real_dictionary_suggests_corrections() {
        let sc = SpellChecker::new(DictVariant::EnUs).unwrap();
        // A classic typo should suggest the intended word near the top.
        let s = sc.suggest("teh");
        assert!(!s.is_empty(), "engine should offer a correction for 'teh'");
        assert!(
            s.iter().any(|w| w == "the"),
            "'the' should be among the suggestions for 'teh': {s:?}"
        );
        // "recieve" -> "receive".
        let s = sc.suggest("recieve");
        assert!(
            s.iter().any(|w| w == "receive"),
            "'receive' should be suggested for 'recieve': {s:?}"
        );
    }

    #[test]
    fn suggest_at_resolves_word_and_suggestions() {
        let sc = SpellChecker::new(DictVariant::EnUs).unwrap();
        // Cursor inside the misspelling "recieve" (line 0, any col in the span).
        let text = "Please recieve this.";
        let t = sc.suggest_at(text, 0, 9, None).expect("cursor on a misspelling");
        assert_eq!(t.word, "recieve");
        assert_eq!((t.misspelling.start_col, t.misspelling.end_col), (7, 14));
        assert!(t.suggestions.iter().any(|w| w == "receive"));
        // A cursor on a CORRECT word yields nothing (calm no-op for the binding).
        assert!(sc.suggest_at(text, 0, 2, None).is_none(), "'Please' is correct");
    }
}
