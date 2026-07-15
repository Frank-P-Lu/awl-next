//! DETERMINISTIC corpus tiers for the unified `--bench-suite` runner.
//!
//! Every tier is GENERATED at bench time from a fixed seed through a small
//! in-crate PRNG (SplitMix64) — no large fixture blobs live in git (the two
//! `benches/fixtures` files stay for the legacy flags only). The whole point is
//! that the SAME seed yields a BYTE-IDENTICAL corpus on every run and every
//! machine, so a baseline diff compares the same workload and the pinned
//! golden hashes below catch any accidental generator drift (which would
//! silently invalidate `benches/baseline.json`).
//!
//! Tiers (SCOPE: prose is the product, code is light):
//!   * S     — ~500 words, a markdown note.
//!   * M     — ~2,000 words, a markdown essay with light inline styling.
//!   * L     — ~50,000 words, a novel with chapter headings (the tier the
//!             legacy fixtures never had).
//!   * XPARA — pathological: ONE enormous unbroken paragraph (a single logical
//!             line that wraps into hundreds of visual rows).
//!   * XMD   — pathological: heavy markdown (headings at every level, lists,
//!             task lists, quotes, fenced code, tables, highlights, rules).
//!   * CODE  — a large generated `.rs` document (shapes in the world's mono
//!             face and exercises the four Alabaster syntax roles).
//!
//! The vocabulary deliberately includes a few dictionary-missing words
//! ("quokka", "teh", …) so the spell-squiggle pipeline carries a real,
//! deterministic proto load in every tier.

/// The one fixed seed. Changing it changes every tier byte-for-byte — that is
/// a WORKLOAD change and requires a deliberate baseline regeneration (the
/// golden-hash tests below will insist).
const SEED: u64 = 0xAB5_0177_BEAC_0DE5;

/// The corpus tier axis. `ALL` is the suite's iteration order; `name`/`class`
/// use no-wildcard matches so a new tier fails to compile until it is placed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Tier {
    S,
    M,
    L,
    XPara,
    XMd,
    Code,
}

impl Tier {
    pub(super) const ALL: [Tier; 6] =
        [Tier::S, Tier::M, Tier::L, Tier::XPara, Tier::XMd, Tier::Code];

    pub(super) fn name(self) -> &'static str {
        match self {
            Tier::S => "S",
            Tier::M => "M",
            Tier::L => "L",
            Tier::XPara => "XPARA",
            Tier::XMd => "XMD",
            Tier::Code => "CODE",
        }
    }

    /// The page-width class this tier's document would resolve to live
    /// (`page::PageClass` — prose measure vs code measure).
    pub(super) fn class(self) -> crate::page::PageClass {
        match self {
            Tier::S | Tier::M | Tier::L | Tier::XPara | Tier::XMd => crate::page::PageClass::Prose,
            Tier::Code => crate::page::PageClass::Code,
        }
    }

    /// Markdown gate for the tier's `ViewState` (mirrors a no-path note /
    /// `.md` file for the prose tiers; a `.rs` file is never markdown).
    pub(super) fn is_markdown(self) -> bool {
        match self {
            Tier::S | Tier::M | Tier::L | Tier::XPara | Tier::XMd => true,
            Tier::Code => false,
        }
    }

    /// Syntax gate: only the CODE tier carries a language.
    pub(super) fn syn_lang(self) -> Option<crate::syntax::Lang> {
        match self {
            Tier::S | Tier::M | Tier::L | Tier::XPara | Tier::XMd => None,
            Tier::Code => crate::syntax::Lang::from_name("rust"),
        }
    }

    /// The display name a live buffer of this tier would show in the gutter.
    pub(super) fn doc_name(self) -> &'static str {
        match self {
            Tier::S => "note.md",
            Tier::M => "essay.md",
            Tier::L => "novel.md",
            Tier::XPara => "wall.md",
            Tier::XMd => "everything.md",
            Tier::Code => "generated.rs",
        }
    }
}

/// SplitMix64 — tiny, deterministic, zero-dependency. Good enough to spread a
/// word picker; NOT a crypto RNG and never used as one.
pub(super) struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick(&mut self, xs: &[&'static str]) -> &'static str {
        xs[self.below(xs.len())]
    }
}

/// Word pools. "the" is guaranteed per sentence (the search scenario's query);
/// the last few NOUNS are deliberately outside the bundled dictionary so the
/// squiggle pipeline always has deterministic work.
const NOUNS: [&str; 16] = [
    "river", "editor", "lantern", "harbour", "window", "notebook", "orchard", "signal",
    "morning", "letter", "garden", "bridge", "quokka", "bilby", "teh", "wombatling",
];
const VERBS: [&str; 12] = [
    "carries", "settles", "opens", "remembers", "follows", "gathers", "holds", "turns",
    "answers", "leans", "waits", "measures",
];
const ADJS: [&str; 12] = [
    "quiet", "warm", "long", "pale", "steady", "small", "amber", "late",
    "narrow", "calm", "bright", "worn",
];
const TAILS: [&str; 8] = [
    "before the light changes", "without any hurry", "under the same sky",
    "as if it mattered", "for most of the year", "against the far wall",
    "past the old fence", "toward the open door",
];

/// One sentence: "The <adj> <noun> <verb> the <adj> <noun> <tail>." — every
/// sentence contains at least two "the"/"The" tokens, keeping the search
/// scenario's match count high and deterministic in every tier.
fn sentence(rng: &mut Rng) -> String {
    format!(
        "The {} {} {} the {} {} {}.",
        rng.pick(&ADJS),
        rng.pick(&NOUNS),
        rng.pick(&VERBS),
        rng.pick(&ADJS),
        rng.pick(&NOUNS),
        rng.pick(&TAILS),
    )
}

/// One paragraph of `n` sentences on a single line (soft-wrapped by the
/// renderer, like real prose).
fn paragraph(rng: &mut Rng, n: usize) -> String {
    let mut s = String::new();
    for i in 0..n {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&sentence(rng));
    }
    s
}

pub(super) fn count_words(text: &str) -> u64 {
    text.split_whitespace().count() as u64
}

/// Generate a tier's document. Pure: same tier -> same bytes, every run.
pub(super) fn text(tier: Tier) -> String {
    // Per-tier seed offset so tiers don't share a prefix.
    let mut rng = Rng::new(SEED ^ (tier.name().len() as u64) ^ ((tier as u64) << 32));
    match tier {
        Tier::S => prose(&mut rng, 500, "A Note", 1),
        Tier::M => essay(&mut rng, 2_000),
        Tier::L => novel(&mut rng, 50_000),
        Tier::XPara => wall(&mut rng, 5_000),
        Tier::XMd => heavy_markdown(&mut rng, 4_000),
        Tier::Code => rust_source(&mut rng, 2_500),
    }
}

/// Plain markdown prose: a title, paragraphs until `min_words`, `subheads`
/// evenly spread H2s.
fn prose(rng: &mut Rng, min_words: usize, title: &str, subheads: usize) -> String {
    let mut s = format!("# {title}\n\n");
    let mut heads_left = subheads;
    let mut para = 0usize;
    while count_words(&s) < min_words as u64 {
        if heads_left > 0 && para == 2 {
            s.push_str(&format!("## The {} {}\n\n", rng.pick(&ADJS), rng.pick(&NOUNS)));
            heads_left -= 1;
        }
        let n = 3 + rng.below(4);
        s.push_str(&paragraph(rng, n));
        s.push_str("\n\n");
        para += 1;
    }
    s
}

/// An essay: headings every few paragraphs plus LIGHT inline styling (a bold
/// run, an italic run, an inline-code run) so the markdown span pass has
/// ordinary work without tipping into the XMD tier's pathology.
fn essay(rng: &mut Rng, min_words: usize) -> String {
    let mut s = String::from("# An Essay\n\n");
    let mut para = 0usize;
    while count_words(&s) < min_words as u64 {
        if para % 4 == 3 {
            s.push_str(&format!("## On the {} {}\n\n", rng.pick(&ADJS), rng.pick(&NOUNS)));
        }
        let n = 3 + rng.below(4);
        let mut p = paragraph(rng, n);
        match para % 3 {
            0 => p.push_str(&format!(" It stays **{}**.", rng.pick(&ADJS))),
            1 => p.push_str(&format!(" It reads *{}*.", rng.pick(&ADJS))),
            _ => p.push_str(&format!(" See `{}`.", rng.pick(&NOUNS))),
        }
        s.push_str(&p);
        s.push_str("\n\n");
        para += 1;
    }
    s
}

/// A novel: chapter H1s roughly every 2,000 words, plain paragraphs between.
fn novel(rng: &mut Rng, min_words: usize) -> String {
    let mut s = String::new();
    let mut chapter = 0usize;
    let mut next_chapter_at = 0u64;
    while count_words(&s) < min_words as u64 {
        if count_words(&s) >= next_chapter_at {
            chapter += 1;
            s.push_str(&format!("# Chapter {chapter}\n\n"));
            next_chapter_at += 2_000;
        }
        let n = 4 + rng.below(4);
        s.push_str(&paragraph(rng, n));
        s.push_str("\n\n");
    }
    s
}

/// The pathological wall: ONE unbroken paragraph — a single logical line that
/// soft-wraps into hundreds of visual rows (the per-line worst case for the
/// incremental typing path and the wrap/resize step).
fn wall(rng: &mut Rng, min_words: usize) -> String {
    let mut s = String::new();
    while count_words(&s) < min_words as u64 {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(&sentence(rng));
    }
    s.push('\n');
    s
}

/// The heavy-markdown pathology: every block construct the renderer styles,
/// cycled deterministically — headings at every level, lists, task lists,
/// quotes, fenced code (rust + sh), a table, highlights, rules.
fn heavy_markdown(rng: &mut Rng, min_words: usize) -> String {
    let mut s = String::from("# Everything Document\n\n");
    let mut block = 0usize;
    while count_words(&s) < min_words as u64 {
        match block % 8 {
            0 => {
                let level = 1 + (block / 8) % 6;
                s.push_str(&format!(
                    "{} The {} {}\n\n",
                    "#".repeat(level),
                    rng.pick(&ADJS),
                    rng.pick(&NOUNS)
                ));
                let n = 2 + rng.below(2);
                s.push_str(&paragraph(rng, n));
                s.push_str("\n\n");
            }
            1 => {
                for _ in 0..5 {
                    s.push_str(&format!("- {}\n", sentence(rng)));
                }
                s.push('\n');
            }
            2 => {
                s.push_str(&format!("- [ ] {}\n- [x] {}\n- [ ] {}\n\n", sentence(rng), sentence(rng), sentence(rng)));
            }
            3 => {
                s.push_str(&format!("> {}\n> {}\n\n", sentence(rng), sentence(rng)));
            }
            4 => {
                s.push_str("```rust\n");
                s.push_str("// A prose comment that says what the block below is for.\n");
                s.push_str(&format!(
                    "fn {}_{}(n: u64) -> u64 {{\n    let label = \"{} {}\";\n    n + label.len() as u64 + {}\n}}\n",
                    rng.pick(&VERBS),
                    rng.pick(&NOUNS),
                    rng.pick(&ADJS),
                    rng.pick(&NOUNS),
                    rng.below(97)
                ));
                s.push_str("```\n\n");
            }
            5 => {
                s.push_str(&format!(
                    "The {} stays ==always marked== and **bold** and *leaning* with `{}` inline. [The link]({}.md) holds.\n\n",
                    rng.pick(&NOUNS),
                    rng.pick(&NOUNS),
                    rng.pick(&NOUNS)
                ));
            }
            6 => {
                s.push_str("| left | middle | right |\n| --- | --- | --- |\n");
                for _ in 0..3 {
                    s.push_str(&format!(
                        "| {} | {} | {} |\n",
                        rng.pick(&NOUNS),
                        rng.pick(&VERBS),
                        rng.pick(&ADJS)
                    ));
                }
                s.push('\n');
            }
            _ => {
                s.push_str("---\n\n");
                s.push_str(&paragraph(rng, 2));
                s.push_str("\n\n");
            }
        }
        block += 1;
    }
    s
}

/// A large plausible `.rs` file: module docs, prose comments AND
/// commented-out code (the two comment tiers), string literals, numeric
/// constants, definitions — all four Alabaster roles, at scale.
fn rust_source(rng: &mut Rng, min_lines: usize) -> String {
    let mut s = String::from(
        "//! Generated benchmark source. The prose in these comments reads like\n\
         //! real sentences so the two-tier comment classifier has honest work.\n\n",
    );
    let mut item = 0usize;
    while s.lines().count() < min_lines {
        let noun = rng.pick(&NOUNS);
        let verb = rng.pick(&VERBS);
        let adj = rng.pick(&ADJS);
        match item % 4 {
            0 => {
                s.push_str(&format!(
                    "/// The {adj} {noun} {verb} the value it is given.\n\
                     /// {}\n\
                     pub fn {verb}_{noun}_{item}(input: u64) -> u64 {{\n\
                     \x20   // The scale below was measured, not guessed.\n\
                     \x20   let scale: u64 = {};\n\
                     \x20   let label = \"{adj} {noun}\";\n\
                     \x20   // let legacy = input * 2; // superseded by scale\n\
                     \x20   input.wrapping_mul(scale) + label.len() as u64\n\
                     }}\n\n",
                    sentence(rng),
                    3 + rng.below(97),
                ));
            }
            1 => {
                s.push_str(&format!(
                    "/// {}\n\
                     pub struct {}{}{item} {{\n\
                     \x20   pub count: usize,\n\
                     \x20   pub name: String,\n\
                     \x20   pub ratio: f64,\n\
                     }}\n\n",
                    sentence(rng),
                    adj[..1].to_ascii_uppercase(),
                    &adj[1..],
                ));
            }
            2 => {
                s.push_str(&format!(
                    "pub const {}_{item}: u64 = {};\n\
                     pub const {}_NAME_{item}: &str = \"the {adj} {noun}\";\n\n",
                    noun.to_ascii_uppercase(),
                    rng.below(9973),
                    verb.to_ascii_uppercase(),
                ));
            }
            _ => {
                s.push_str(&format!(
                    "fn check_{noun}_{item}(xs: &[u64]) -> bool {{\n\
                     \x20   // Every entry must stay under the measured budget.\n\
                     \x20   xs.iter().all(|&x| x < {} && x != 0)\n\
                     }}\n\n",
                    100 + rng.below(900),
                ));
            }
        }
        item += 1;
    }
    s
}

/// FNV-1a 64 over the corpus bytes — the golden-hash fingerprint the tests pin.
pub(super) fn fingerprint(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same seed -> byte-identical corpus, every tier (the spec's own assert).
    #[test]
    fn corpus_generation_is_deterministic_per_tier() {
        for tier in Tier::ALL {
            assert_eq!(text(tier), text(tier), "tier {} must regenerate byte-identically", tier.name());
        }
    }

    /// Word-count floors and shape facts per tier: S/M/L hit their word
    /// targets, XPARA is genuinely ONE unbroken paragraph, CODE hits its line
    /// target, and every prose tier carries the search scenario's query.
    #[test]
    fn corpus_tiers_hit_their_size_and_shape_targets() {
        let s = text(Tier::S);
        let m = text(Tier::M);
        let l = text(Tier::L);
        let xp = text(Tier::XPara);
        let xm = text(Tier::XMd);
        let code = text(Tier::Code);
        assert!(count_words(&s) >= 500, "S must be >= 500 words");
        assert!(count_words(&m) >= 2_000, "M must be >= 2,000 words");
        assert!(count_words(&l) >= 50_000, "L must be >= 50,000 words");
        assert!(count_words(&xp) >= 5_000, "XPARA must be >= 5,000 words");
        assert!(count_words(&xm) >= 4_000, "XMD must be >= 4,000 words");
        assert_eq!(xp.trim_end().lines().count(), 1, "XPARA must be ONE unbroken line");
        assert!(code.lines().count() >= 2_500, "CODE must be >= 2,500 lines");
        for (tier, t) in [("S", &s), ("M", &m), ("L", &l), ("XPARA", &xp), ("XMD", &xm), ("CODE", &code)] {
            assert!(t.contains("the "), "tier {tier} must contain the search query");
            assert!(t.ends_with('\n'), "tier {tier} must end with a newline");
        }
        // XMD really carries the heavy constructs.
        for needle in ["```rust", "- [ ]", "==always marked==", "| --- |", "\n---\n", "> The"] {
            assert!(xm.contains(needle), "XMD must contain {needle:?}");
        }
        // CODE really carries both comment tiers + strings + constants.
        for needle in ["// The scale below", "// let legacy", "pub fn", "pub const", "\"the "] {
            assert!(code.contains(needle), "CODE must contain {needle:?}");
        }
    }

    /// GOLDEN HASHES: the corpus bytes are pinned. A generator change (vocab,
    /// seed, structure) fails here — that is a WORKLOAD change, and the
    /// baseline must be regenerated deliberately (scripts/bench.sh
    /// --update-baseline) in the same commit that updates these values.
    #[test]
    fn corpus_golden_hashes_are_pinned() {
        let got: Vec<(&str, u64)> = Tier::ALL
            .iter()
            .map(|&t| (t.name(), fingerprint(&text(t))))
            .collect();
        let want: [(&str, u64); 6] = [
            ("S", 0xf5c29768e28e6d99),
            ("M", 0x73001c00faa23162),
            ("L", 0xd43186c3ce43012e),
            ("XPARA", 0xc6fd14e01edbbf97),
            ("XMD", 0x41e4908f785f9ba2),
            ("CODE", 0x7df738255002d62a),
        ];
        for ((name, got), (wname, want)) in got.iter().zip(want.iter()) {
            assert_eq!(name, wname);
            assert_eq!(
                got, want,
                "tier {name} corpus fingerprint drifted — a deliberate workload change must \
                 update this pin AND regenerate benches/baseline.json in the same commit"
            );
        }
    }
}
