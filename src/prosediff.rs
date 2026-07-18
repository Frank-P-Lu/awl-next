//! PROSE DIFF — the marked-up manuscript (THE WRITER'S DIFF).
//!
//! THE WRITER'S DIFF (board item, thesis settled with the user 2026-07-17, shipped):
//! the code diff is the WRONG metaphor for prose. Lines aren't meaning-units; hunks
//! serve verification, not reading; and a delete+insert PAIR *lies* about a move —
//! the commonest writer revision. The RIGHT metaphor is the 400-year-old marked-up
//! manuscript: inline, in reading order, in the document, spoken in awl's OWN
//! vocabulary — struck+muted deletions, insertions in the highlight wash, moves
//! marked distinctly, unchanged stretches folded to a quiet `⋯ ¶ ⋯` row.
//!
//! This module is the PURE CORE (fully unit-tested) plus a serializer that lays the
//! diff into a marked-up-markdown transcript the EXISTING renderer draws with its
//! real strike / `==highlight==` / blockquote-dim vocabulary — so the diff view
//! adds ZERO render-path code. The live "Compare with version…" flow
//! ([`crate::app`]) hands the CURRENT buffer + a chosen history version to
//! [`render_markdown`]; the resulting transcript is shown read-only in the writing
//! column (`App::diff_view`), and the capture harness renders the same transcript
//! headlessly (`AWL_DIFF_OLD`/`AWL_DIFF_NEW`, see [`env_capture`]) so the view is
//! pixel-verifiable.
//!
//! Two prose-native REQUIREMENTS the code diff can't express, both lived here:
//!   * **Move detection** — a relocated paragraph reads as *moved*, never as a
//!     delete+insert pair (LCS backbone + a higher-bar greedy match on the
//!     leftovers).
//!   * **Rewrite coalescing** — past a change-density threshold a paragraph stops
//!     showing word-level surgery and reads as old-struck-whole / new-washed-whole,
//!     because "edited" and "rewritten" are different authorial acts. The threshold
//!     is a PARAMETER (the gate picked SENTENCE granularity × 0.5 coalescing — see
//!     [`Params::shipping`]).

use std::sync::OnceLock;

/// Word- vs sentence-level granularity for the within-paragraph diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gran {
    /// Diff over whitespace-delimited word tokens (finest — surgical).
    Word,
    /// Diff over sentence tokens (calmer — a changed sentence swaps whole).
    Sentence,
}

/// The two knobs the diff carries: within-paragraph granularity + the coalescing
/// threshold. [`Params::shipping`] is the gate-approved recipe the live view uses;
/// [`Params::default`] (Word × 0.5) is the exploration baseline the core tests pin.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub gran: Gran,
    /// Change DENSITY (0..=1, = `1 - similarity`) above which a modified paragraph
    /// COALESCES to a whole-paragraph rewrite instead of inline word surgery.
    pub coalesce: f32,
}

impl Default for Params {
    fn default() -> Self {
        Params { gran: Gran::Word, coalesce: 0.5 }
    }
}

impl Params {
    /// The SHIPPING recipe the live "Compare with version…" view uses — the gate's
    /// pick (user, 2026-07-18): SENTENCE-level granularity (a touched sentence swaps
    /// whole rather than showing word-by-word surgery — calmer for prose) at a 0.5
    /// coalescing threshold (a paragraph reworded past halfway reads as a clean
    /// old-struck / new-washed rewrite). The one owner both the live App and the
    /// capture harness read, so they can never diverge on what a shipped diff looks
    /// like.
    pub const fn shipping() -> Self {
        Params { gran: Gran::Sentence, coalesce: 0.5 }
    }
}

/// A within-paragraph segment of the inline diff (word/sentence granularity).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seg {
    /// Carried through unchanged (default ink).
    Same(String),
    /// Added (renders in the highlight wash).
    Ins(String),
    /// Removed (renders struck).
    Del(String),
}

/// Which way a relocated paragraph travelled (cosmetic — drives the marker arrow).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveDir {
    Up,
    Down,
}

/// One block of the diff transcript, in NEW-document reading order.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// A run of `n` consecutive untouched paragraphs, folded to one quiet row.
    Fold(usize),
    /// A paragraph edited in place — inline word/sentence segments.
    Modified(Vec<Seg>),
    /// A paragraph whose change density crossed the coalescing threshold: the old
    /// text struck whole, the new text washed whole.
    Rewritten { old: String, new: String },
    /// A wholly new paragraph.
    Inserted(String),
    /// A wholly removed paragraph.
    Deleted(String),
    /// A paragraph relocated (matched by similarity, out of order) — shown ONCE, at
    /// its new location, marked as moved rather than as delete+insert.
    Moved { text: String, dir: MoveDir },
}

// --- tuning (probe constants; the gallery varies `coalesce`, these stay fixed) ---

/// A paragraph pair below this similarity is NOT an in-place edit (backbone) — low
/// so even a heavy rewrite still MATCHES (then density decides Modified vs Rewritten).
const BACKBONE_SIM_MIN: f32 = 0.25;
/// A leftover (off-backbone) pair must clear this HIGHER bar to read as a move — a
/// relocation carries most of its words with it.
const MOVE_SIM_MIN: f32 = 0.55;

// ---------------------------------------------------------------------------
// Tokenizing
// ---------------------------------------------------------------------------

/// Split into paragraphs on blank-line boundaries, trimmed of surrounding blank
/// lines but keeping each paragraph's internal newlines (docs are hard-wrapped).
fn paragraphs(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for line in s.lines() {
        if line.trim().is_empty() {
            if !cur.is_empty() {
                out.push(cur.join("\n"));
                cur.clear();
            }
        } else {
            cur.push(line);
        }
    }
    if !cur.is_empty() {
        out.push(cur.join("\n"));
    }
    out
}

/// Word tokens: alternating runs of non-whitespace and whitespace, so a lossless
/// join reproduces the source exactly (whitespace, including `\n`, is its own token).
fn word_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        let ws = c.is_whitespace();
        let mut tok = String::new();
        while let Some(&c2) = chars.peek() {
            if c2.is_whitespace() != ws {
                break;
            }
            tok.push(c2);
            chars.next();
        }
        out.push(tok);
    }
    out
}

/// Sentence tokens: split after `.`/`!`/`?` runs that are followed by whitespace or
/// end-of-text, keeping the terminator and its trailing whitespace with the sentence
/// (lossless join). A calmer granularity — a touched sentence swaps as one.
fn sentence_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        cur.push(c);
        if matches!(c, '.' | '!' | '?') {
            // absorb any run of terminators + following whitespace
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j], '.' | '!' | '?') {
                cur.push(chars[j]);
                j += 1;
            }
            let followed_by_break = j >= chars.len() || chars[j].is_whitespace();
            if followed_by_break {
                while j < chars.len() && chars[j].is_whitespace() {
                    cur.push(chars[j]);
                    j += 1;
                }
                out.push(std::mem::take(&mut cur));
                i = j;
                continue;
            }
            i = j;
            continue;
        }
        i += 1;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn tokens(s: &str, gran: Gran) -> Vec<String> {
    match gran {
        Gran::Word => word_tokens(s),
        Gran::Sentence => sentence_tokens(s),
    }
}

/// Content (non-blank) tokens only — the unit of similarity + density, so leading
/// indentation and inter-word spacing never dominate the measure.
fn content_tokens(toks: &[String]) -> Vec<&str> {
    toks.iter()
        .filter(|t| !t.trim().is_empty())
        .map(|t| t.as_str())
        .collect()
}

// ---------------------------------------------------------------------------
// LCS (the one shared primitive: paragraph alignment AND within-paragraph diff)
// ---------------------------------------------------------------------------

/// Classic LCS length table over two token slices (by equality).
fn lcs_table<T: PartialEq>(a: &[T], b: &[T]) -> Vec<Vec<u32>> {
    let mut dp = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    dp
}

/// difflib-style ratio: `2·|LCS| / (|a| + |b|)`, over content tokens. 1.0 = identical.
fn ratio(a: &[&str], b: &[&str]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let dp = lcs_table(a, b);
    let common = dp[0][0] as f32;
    2.0 * common / (a.len() + b.len()) as f32
}

/// Within-paragraph inline diff → merged Same/Del/Ins segments (lossless).
fn seg_diff(old: &str, new: &str, gran: Gran) -> Vec<Seg> {
    let a = tokens(old, gran);
    let b = tokens(new, gran);
    let dp = lcs_table(&a, &b);
    let (mut i, mut j) = (0usize, 0usize);
    // Accumulate into runs, flushing when the kind changes.
    let mut segs: Vec<Seg> = Vec::new();
    let push = |segs: &mut Vec<Seg>, mk: fn(String) -> Seg, s: &str, tag: u8| {
        // merge with the previous run of the same tag
        if let Some(last) = segs.last_mut() {
            let same_tag = match (last, tag) {
                (Seg::Same(x), 0) | (Seg::Del(x), 1) | (Seg::Ins(x), 2) => {
                    x.push_str(s);
                    true
                }
                _ => false,
            };
            if same_tag {
                return;
            }
        }
        segs.push(mk(s.to_string()));
    };
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            push(&mut segs, Seg::Same, &a[i], 0);
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            push(&mut segs, Seg::Del, &a[i], 1);
            i += 1;
        } else {
            push(&mut segs, Seg::Ins, &b[j], 2);
            j += 1;
        }
    }
    while i < a.len() {
        push(&mut segs, Seg::Del, &a[i], 1);
        i += 1;
    }
    while j < b.len() {
        push(&mut segs, Seg::Ins, &b[j], 2);
        j += 1;
    }
    segs
}

/// Change density of a seg list = changed content tokens / total content tokens,
/// i.e. `1 - similarity`. Drives the coalescing decision.
fn seg_density(segs: &[Seg], gran: Gran) -> f32 {
    let count = |s: &str| content_tokens(&tokens(s, gran)).len();
    let (mut same, mut changed) = (0usize, 0usize);
    for seg in segs {
        match seg {
            Seg::Same(s) => same += count(s),
            Seg::Ins(s) | Seg::Del(s) => changed += count(s),
        }
    }
    let total = same * 2 + changed;
    if total == 0 {
        0.0
    } else {
        changed as f32 / total as f32
    }
}

// ---------------------------------------------------------------------------
// Paragraph alignment + move detection
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Same,
    Edit,
    Move,
}

/// A matched paragraph pair (old index, new index) and how it matched.
#[derive(Clone, Copy, Debug)]
struct Pair {
    oi: usize,
    ni: usize,
    role: Role,
}

/// Similarity-aware LCS over paragraphs → the ordered BACKBONE of in-place matches
/// (Same when identical, Edit otherwise). Increasing in both indices by construction,
/// so a relocation is deliberately NOT captured here — it falls to the leftovers.
fn backbone(old: &[Vec<&str>], new: &[Vec<&str>]) -> Vec<Pair> {
    let (n, m) = (old.len(), new.len());
    // score[i][j] = best total similarity aligning old[i..] with new[j..]
    let mut score = vec![vec![0.0f32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let r = ratio(&old[i], &new[j]);
            let diag = if r >= BACKBONE_SIM_MIN {
                score[i + 1][j + 1] + r
            } else {
                f32::MIN
            };
            score[i][j] = diag.max(score[i + 1][j]).max(score[i][j + 1]);
        }
    }
    // backtrack
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        let r = ratio(&old[i], &new[j]);
        let diag = if r >= BACKBONE_SIM_MIN {
            score[i + 1][j + 1] + r
        } else {
            f32::MIN
        };
        if diag >= score[i + 1][j] && diag >= score[i][j] && r >= BACKBONE_SIM_MIN {
            let role = if r >= 0.999 { Role::Same } else { Role::Edit };
            pairs.push(Pair { oi: i, ni: j, role });
            i += 1;
            j += 1;
        } else if score[i + 1][j] >= score[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// Greedy best-first match over the LEFTOVER (unmatched) paragraphs, above the
/// higher MOVE bar → relocations. Everything still unmatched is a pure delete/insert.
fn detect_moves(
    old: &[Vec<&str>],
    new: &[Vec<&str>],
    used_old: &mut [bool],
    used_new: &mut [bool],
) -> Vec<Pair> {
    let mut cands: Vec<(f32, usize, usize)> = Vec::new();
    for (oi, o) in old.iter().enumerate() {
        if used_old[oi] {
            continue;
        }
        for (ni, nw) in new.iter().enumerate() {
            if used_new[ni] {
                continue;
            }
            let r = ratio(o, nw);
            if r >= MOVE_SIM_MIN {
                cands.push((r, oi, ni));
            }
        }
    }
    cands.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut moves = Vec::new();
    for (_, oi, ni) in cands {
        if used_old[oi] || used_new[ni] {
            continue;
        }
        used_old[oi] = true;
        used_new[ni] = true;
        moves.push(Pair { oi, ni, role: Role::Move });
    }
    moves
}

// ---------------------------------------------------------------------------
// Diff (the pure entry point)
// ---------------------------------------------------------------------------

/// Diff two prose documents into an ordered, reading-order block list. Pure +
/// deterministic — the whole design surface the gallery + tests exercise.
pub fn diff(old: &str, new: &str, p: Params) -> Vec<Block> {
    let old_ps = paragraphs(old);
    let new_ps = paragraphs(new);
    // Own the word tokens so the `&str` content views below stay alive.
    let old_words: Vec<Vec<String>> = old_ps.iter().map(|s| word_tokens(s)).collect();
    let new_words: Vec<Vec<String>> = new_ps.iter().map(|s| word_tokens(s)).collect();
    let old_tok: Vec<Vec<&str>> = old_words.iter().map(|w| content_tokens(w)).collect();
    let new_tok: Vec<Vec<&str>> = new_words.iter().map(|w| content_tokens(w)).collect();

    let bb = backbone(&old_tok, &new_tok);
    let mut used_old = vec![false; old_ps.len()];
    let mut used_new = vec![false; new_ps.len()];
    // classification per index (None = not yet placed)
    let mut old_role: Vec<Option<Pair>> = vec![None; old_ps.len()];
    let mut new_role: Vec<Option<Pair>> = vec![None; new_ps.len()];
    for pr in &bb {
        used_old[pr.oi] = true;
        used_new[pr.ni] = true;
        old_role[pr.oi] = Some(*pr);
        new_role[pr.ni] = Some(*pr);
    }
    let moves = detect_moves(&old_tok, &new_tok, &mut used_old, &mut used_new);
    for pr in &moves {
        old_role[pr.oi] = Some(*pr);
        new_role[pr.ni] = Some(*pr);
    }

    // Two-pointer merge → reading order. Old-only (deletes / moved-away) flush first
    // within a gap, then new-only (inserts / moved-in), then the shared anchor.
    let mut blocks: Vec<Block> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let (no, nn) = (old_ps.len(), new_ps.len());
    while i < no || j < nn {
        // old-only side first
        if i < no {
            match old_role[i] {
                None => {
                    blocks.push(Block::Deleted(old_ps[i].clone()));
                    i += 1;
                    continue;
                }
                Some(pr) if pr.role == Role::Move => {
                    // content emitted at its NEW location; skip here
                    i += 1;
                    continue;
                }
                _ => {}
            }
        }
        // new-only side
        if j < nn {
            match new_role[j] {
                None => {
                    blocks.push(Block::Inserted(new_ps[j].clone()));
                    j += 1;
                    continue;
                }
                Some(pr) if pr.role == Role::Move => {
                    let dir = if pr.ni <= pr.oi { MoveDir::Up } else { MoveDir::Down };
                    blocks.push(Block::Moved { text: new_ps[pr.ni].clone(), dir });
                    j += 1;
                    continue;
                }
                _ => {}
            }
        }
        // both anchors — must be partners in the backbone
        if i < no && j < nn {
            if let (Some(a), Some(b)) = (old_role[i], new_role[j]) {
                if a.oi == b.oi && a.ni == b.ni {
                    match a.role {
                        Role::Same => blocks.push(Block::Fold(1)),
                        Role::Edit => {
                            let segs = seg_diff(&old_ps[i], &new_ps[j], p.gran);
                            if seg_density(&segs, p.gran) > p.coalesce {
                                blocks.push(Block::Rewritten {
                                    old: old_ps[i].clone(),
                                    new: new_ps[j].clone(),
                                });
                            } else {
                                blocks.push(Block::Modified(segs));
                            }
                        }
                        Role::Move => {}
                    }
                    i += 1;
                    j += 1;
                    continue;
                }
            }
        }
        // safety fall-through (shouldn't trigger): advance the laggard
        if i < no {
            i += 1;
        } else {
            j += 1;
        }
    }

    merge_folds(blocks)
}

/// Collapse consecutive `Fold(_)` blocks into one summed fold row.
fn merge_folds(blocks: Vec<Block>) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for b in blocks {
        if let Block::Fold(n) = b {
            if let Some(Block::Fold(m)) = out.last_mut() {
                *m += n;
                continue;
            }
        }
        out.push(b);
    }
    out
}

// ---------------------------------------------------------------------------
// Serialize → marked-up markdown (awl's OWN render vocabulary)
// ---------------------------------------------------------------------------

/// Struck deletions speak REAL markdown now: `~~…~~`, wrapped per line by
/// [`wrap_inline`] — routed through the renderer's own `MdKind::Strikethrough`
/// (the strikethrough-render round), whose muted ink + drawn strike line come
/// from THE ONE strike owner (`render::spans::strike_line_band` /
/// `strike_ink`), the same fns the format popover's `S` button reads.
///
/// HISTORY (the retired mechanism): before the renderer could draw `~~strike~~`
/// at all, deletions were struck by inserting a COMBINING LONG STROKE OVERLAY
/// (`\u{0336}`) after every non-whitespace char — genuine struck glyphs with
/// zero render-path code, at the cost of per-word gaps (whitespace had to stay
/// unstruck or read as "- - -" leaders) and of the transcript carrying invisible
/// combining marks. With real strikethrough in the render vocabulary the
/// serializer says what it means; the drawn line crosses spaces cleanly, so the
/// whitespace exception died with the mechanism.
fn strike(s: &str) -> String {
    wrap_inline(s, "~~")
}

/// Wrap each physical line's content in `==…==` (single-line pairs only — a
/// cross-line `==` is inert in awl), preserving leading indentation.
fn highlight_lines(s: &str) -> String {
    s.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.is_empty() {
                line.to_string()
            } else {
                let indent = &line[..line.len() - trimmed.len()];
                format!("{indent}=={trimmed}==")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Wrap ONE inline insertion run in the highlight wash markers — see
/// [`wrap_inline`] (the shared shape [`strike`] rides too).
fn highlight_inline(s: &str) -> String {
    wrap_inline(s, "==")
}

/// Wrap ONE inline run (may straddle hard-wrap newlines) in `marker` pairs so no
/// pair crosses a line, and so leading/trailing whitespace stays OUTSIDE the
/// markers (a `== foo ==` with inner padding is inert in awl's scan, and a
/// `~~ foo ~~` fails GFM's flanking rules the same way). THE ONE wrapper both
/// the `==insertion==` wash and the `~~deletion~~` strike serialize through —
/// merge, don't align.
fn wrap_inline(s: &str, marker: &str) -> String {
    let mut out = String::new();
    for (k, piece) in s.split('\n').enumerate() {
        if k > 0 {
            out.push('\n');
        }
        let trimmed = piece.trim();
        if trimmed.is_empty() {
            out.push_str(piece);
            continue;
        }
        let lead = &piece[..piece.len() - piece.trim_start().len()];
        let tail = &piece[piece.trim_end().len()..];
        out.push_str(lead);
        out.push_str(marker);
        out.push_str(trimmed);
        out.push_str(marker);
        out.push_str(tail);
    }
    out
}

/// Reduce markdown SOURCE to plain prose for the manuscript diff: a marked-up
/// manuscript shows WORDS, not syntax — and, pragmatically, stripping the inline
/// markers means the `==wash==` / strike serialization never has to wrap nested
/// markdown (a `==**bold**==` pair is fragile). Applied identically to both versions
/// before diffing, so the alignment is unaffected. Deliberately light: it neutralizes
/// emphasis/code/link/marker syntax, nothing semantic.
fn strip_markdown(s: &str) -> String {
    let mut out = String::new();
    for (li, line) in s.lines().enumerate() {
        if li > 0 {
            out.push('\n');
        }
        // strip a leading block marker (heading / bullet / quote / numbered)
        let mut rest = line;
        let trimmed = rest.trim_start();
        let indent_len = rest.len() - trimmed.len();
        let indent = &rest[..indent_len];
        let after: Option<&str> = if let Some(a) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")).or_else(|| trimmed.strip_prefix("+ ")) {
            Some(a)
        } else if trimmed.starts_with('#') {
            Some(trimmed.trim_start_matches('#').trim_start())
        } else if let Some(a) = trimmed.strip_prefix("> ") {
            Some(a)
        } else {
            trimmed
                .find(". ")
                .filter(|&i| trimmed[..i].chars().all(|c| c.is_ascii_digit()) && i > 0)
                .map(|i| &trimmed[i + 2..])
        };
        if let Some(a) = after {
            out.push_str(indent);
            rest = a;
        } else {
            out.push_str(indent);
        }
        // strip inline emphasis/code markers, keeping the inner text
        let bytes: Vec<char> = rest.chars().collect();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            match c {
                '`' => {}
                '*' | '_' => {}
                // `~` is an inline marker in awl's markdown now (`~~strike~~`,
                // the strikethrough-render round): a literal tilde inside a
                // deleted run would open/close the serializer's own `~~` wrap
                // early — exactly the nested-markdown fragility this strip
                // exists to remove (the `*`/`_`/backtick precedent).
                '~' => {}
                '[' | ']' => {}
                '(' if i > 0 && bytes[i - 1] == ']' => {
                    // skip a link's (url) tail entirely
                    while i < bytes.len() && bytes[i] != ')' {
                        i += 1;
                    }
                }
                _ => out.push(c),
            }
            i += 1;
        }
    }
    out
}

/// Prefix every physical line with `> ` (blockquote → dim, `>` auto-conceals).
fn blockquote(s: &str) -> String {
    s.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n")
}

/// Serialize a block list into a marked-up-markdown transcript. The leading heading
/// parks the headless caret (byte 0) on a throwaway line so every diff marker below
/// stays WYSIWYG-concealed (clean preview), never revealed-raw.
pub fn render_markdown_blocks(blocks: &[Block], title: &str) -> String {
    let mut out = format!("# {title}\n\n");
    for b in blocks {
        match b {
            Block::Fold(n) => {
                let unit = if *n == 1 { "paragraph" } else { "paragraphs" };
                out.push_str(&format!("> ⋯  {n} {unit} unchanged  ⋯\n\n"));
            }
            Block::Modified(segs) => {
                for seg in segs {
                    match seg {
                        Seg::Same(s) => out.push_str(s),
                        Seg::Ins(s) => out.push_str(&highlight_inline(s)),
                        Seg::Del(s) => out.push_str(&strike(s)),
                    }
                }
                out.push_str("\n\n");
            }
            Block::Rewritten { old, new } => {
                out.push_str(&blockquote(&strike(old)));
                out.push_str("\n\n");
                out.push_str(&highlight_lines(new));
                out.push_str("\n\n");
            }
            Block::Inserted(s) => {
                out.push_str(&highlight_lines(s));
                out.push_str("\n\n");
            }
            Block::Deleted(s) => {
                out.push_str(&blockquote(&strike(s)));
                out.push_str("\n\n");
            }
            Block::Moved { text, dir } => {
                let arrow = match dir {
                    MoveDir::Up => "↑",
                    MoveDir::Down => "↓",
                };
                // italic blockquote + distinct marker: same content, relocated.
                let body = text.replace('\n', " ");
                out.push_str(&format!("> *⇄  moved {arrow} — {body}*\n\n"));
            }
        }
    }
    out
}

/// One-call convenience: strip both docs to plain prose, diff, and render the
/// transcript. The strip is a RENDER-path concern (the pure [`diff`] stays raw).
pub fn render_markdown(old: &str, new: &str, p: Params, title: &str) -> String {
    diff_and_render(old, new, p, title).0
}

/// Like [`render_markdown`], but ALSO returns the [`DiffCounts`] of the block list
/// — the shared owner both the transcript AND the capture sidecar's `diff` state
/// block derive from, so they can never disagree about what the transcript contains.
pub fn diff_and_render(old: &str, new: &str, p: Params, title: &str) -> (String, DiffCounts) {
    let (o, n) = (strip_markdown(old), strip_markdown(new));
    let blocks = diff(&o, &n, p);
    let md = render_markdown_blocks(&blocks, title);
    (md, count_blocks(&blocks))
}

// ---------------------------------------------------------------------------
// Sidecar counts — the diff-view STATE oracle (the capture reports these; the
// pixel assertions verify the APPEARANCE, per the sidecar-vs-appearance tripwire).
// ---------------------------------------------------------------------------

/// A count of the diff's block kinds — the compact STATE the capture sidecar's
/// `diff` block reports so an agent can verify "am I looking at a diff, and does it
/// carry deletions / insertions / moves / folds". Pure over the [`Block`] list.
/// APPEARANCE ("the struck region is muted", "the wash is present") is asserted
/// over the PNG's pixels, never inferred from these — this is a state oracle only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiffCounts {
    /// Paragraphs shown STRUCK whole (a coalesced rewrite's old side, or a pure
    /// deletion) — the marks that must render muted+struck.
    pub struck: usize,
    /// Paragraphs shown WASHED whole (a coalesced rewrite's new side, or a pure
    /// insertion) — the marks that must render in the highlight wash.
    pub washed: usize,
    /// Paragraphs edited IN PLACE (inline word/sentence segments).
    pub modified: usize,
    /// Relocated paragraphs, shown once at the new location.
    pub moved: usize,
    /// Folded unchanged-stretch rows (`⋯ N paragraphs unchanged ⋯`).
    pub folds: usize,
}

/// Tally a block list into its [`DiffCounts`] — the sidecar's state view of a diff.
pub fn count_blocks(blocks: &[Block]) -> DiffCounts {
    let mut c = DiffCounts::default();
    for b in blocks {
        match b {
            Block::Fold(_) => c.folds += 1,
            Block::Modified(_) => c.modified += 1,
            Block::Rewritten { .. } => {
                c.struck += 1;
                c.washed += 1;
            }
            Block::Inserted(_) => c.washed += 1,
            Block::Deleted(_) => c.struck += 1,
            Block::Moved { .. } => c.moved += 1,
        }
    }
    c
}

// ---------------------------------------------------------------------------
// Capture harness entry (capture-only) — mirrors the `AWL_POPOVER` / `AWL_CJK_FORCE`
// precedent: read ONCE, a total no-op unless both version paths are set. This is
// how the headless `--screenshot` renders the diff VIEW (a live App feature) so it
// is pixel-verifiable; a normal capture never touches it.
// ---------------------------------------------------------------------------

/// A resolved capture-harness diff request: the two version texts + the shipping
/// params + a title. Built from the `AWL_DIFF_*` env vars ([`env_capture`]).
pub struct EnvCapture {
    pub old: String,
    pub new: String,
    pub params: Params,
    pub title: String,
}

/// Parse the `AWL_DIFF_*` env vars. Returns `Some` only when BOTH version paths are
/// set and readable — otherwise the capture path behaves exactly as today (byte-
/// identical). `AWL_DIFF_GRAN=word` overrides the shipping SENTENCE default;
/// `AWL_DIFF_COALESCE` overrides the 0.5 threshold; `AWL_DIFF_TITLE` the heading.
pub fn env_capture() -> Option<&'static Option<EnvCapture>> {
    static ONCE: OnceLock<Option<EnvCapture>> = OnceLock::new();
    Some(ONCE.get_or_init(|| {
        let old_p = std::env::var("AWL_DIFF_OLD").ok()?;
        let new_p = std::env::var("AWL_DIFF_NEW").ok()?;
        let old = std::fs::read_to_string(&old_p).ok()?;
        let new = std::fs::read_to_string(&new_p).ok()?;
        let mut params = Params::shipping();
        if let Ok("word") = std::env::var("AWL_DIFF_GRAN").as_deref() {
            params.gran = Gran::Word;
        }
        if let Some(c) = std::env::var("AWL_DIFF_COALESCE")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
        {
            params.coalesce = c.clamp(0.0, 1.0);
        }
        let title =
            std::env::var("AWL_DIFF_TITLE").unwrap_or_else(|_| "Comparing versions".into());
        Some(EnvCapture { old, new, params, title })
    }))
}

/// The marked-up transcript + counts + title for the active capture-harness diff
/// request, if any — called by the capture path in place of loading a file so
/// `--screenshot` renders the read-only diff view and the sidecar reports its
/// state. `None` everywhere else (byte-identical ordinary capture).
pub fn env_capture_render() -> Option<(String, DiffCounts, String)> {
    match env_capture() {
        Some(Some(p)) => {
            let (md, counts) = diff_and_render(&p.old, &p.new, p.params, &p.title);
            Some((md, counts, p.title.clone()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wp(s: &str) -> Vec<String> {
        word_tokens(s)
    }

    #[test]
    fn word_tokens_join_losslessly() {
        let s = "The  quick\nbrown fox.";
        assert_eq!(wp(s).concat(), s);
    }

    #[test]
    fn sentence_tokens_join_losslessly() {
        let s = "One sentence. Two! Three? A trailing bit";
        assert_eq!(sentence_tokens(s).concat(), s);
        // four units: three terminated + the trailing bit
        assert_eq!(sentence_tokens(s).len(), 4);
    }

    #[test]
    fn paragraphs_split_on_blank_lines() {
        let s = "a\nb\n\n\nc\n";
        assert_eq!(paragraphs(s), vec!["a\nb".to_string(), "c".to_string()]);
    }

    #[test]
    fn ratio_bounds() {
        let aw = wp("the cat sat");
        let a = content_tokens(&aw);
        assert_eq!(ratio(&a, &a), 1.0);
        let bw = wp("a totally different string here");
        let b = content_tokens(&bw);
        assert!(ratio(&a, &b) < 0.34);
    }

    // --- within-paragraph diff ---

    #[test]
    fn seg_diff_word_level_marks_ins_and_del() {
        let segs = seg_diff("the quick brown fox", "the slow brown fox", Gran::Word);
        // must be lossless on each side
        let old: String = segs
            .iter()
            .filter_map(|s| match s {
                Seg::Same(x) | Seg::Del(x) => Some(x.clone()),
                _ => None,
            })
            .collect();
        let new: String = segs
            .iter()
            .filter_map(|s| match s {
                Seg::Same(x) | Seg::Ins(x) => Some(x.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(old, "the quick brown fox");
        assert_eq!(new, "the slow brown fox");
        assert!(segs.iter().any(|s| matches!(s, Seg::Del(x) if x.contains("quick"))));
        assert!(segs.iter().any(|s| matches!(s, Seg::Ins(x) if x.contains("slow"))));
    }

    #[test]
    fn seg_diff_sentence_swaps_whole_sentence() {
        let old = "First stays. Second changes a lot here.";
        let new = "First stays. A totally rewritten second one.";
        let segs = seg_diff(old, new, Gran::Sentence);
        // "First stays. " is a Same sentence; the second is Del+Ins whole
        assert!(segs.iter().any(|s| matches!(s, Seg::Same(x) if x.contains("First stays"))));
        assert!(segs.iter().any(|s| matches!(s, Seg::Del(_))));
        assert!(segs.iter().any(|s| matches!(s, Seg::Ins(_))));
    }

    // --- coalescing (the density threshold) ---

    #[test]
    fn density_low_for_small_edit_high_for_rewrite() {
        let small = seg_diff("the quick brown fox jumps", "the quick brown fox leaps", Gran::Word);
        let big = seg_diff("the quick brown fox jumps", "an entirely new clause appears", Gran::Word);
        assert!(seg_density(&small, Gran::Word) < 0.25);
        assert!(seg_density(&big, Gran::Word) > 0.6);
    }

    #[test]
    fn coalesce_threshold_flips_modified_to_rewritten() {
        // Shares enough of a spine ("The cat sat on the") to ALIGN as a backbone
        // edit, but is heavily reworded (~half the words changed) — so the threshold,
        // not the alignment, decides Modified vs Rewritten.
        let old = "The cat sat quietly on the warm mat by the old door.";
        let new = "The cat sat nervously on the cold floor near the new window.";
        // low threshold → coalesced whole rewrite
        let lo = diff(old, new, Params { gran: Gran::Word, coalesce: 0.3 });
        assert!(
            lo.iter().any(|b| matches!(b, Block::Rewritten { .. })),
            "low threshold should coalesce: {lo:?}"
        );
        // high threshold → stays inline word-level modified
        let hi = diff(old, new, Params { gran: Gran::Word, coalesce: 0.95 });
        assert!(!hi.iter().any(|b| matches!(b, Block::Rewritten { .. })));
        assert!(hi.iter().any(|b| matches!(b, Block::Modified(_))));
    }

    #[test]
    fn three_thresholds_give_three_distinct_outputs() {
        // The HONEST-AXIS property: two edited paragraphs at DIFFERENT change
        // densities, so a threshold sweep between/around them yields three
        // provably-distinct rewrite counts (2 / 1 / 0) — never the c50≡c70
        // collapse that a fixture whose densities all sit below 0.5 exhibits.
        // A lightly-edited paragraph (~1/4 words swapped) and a heavily-edited
        // one (~2/3 words swapped), both still sharing enough spine to ALIGN.
        let old = "\
The quiet river wound its slow way past the sleeping village every single morning.

She counted the coins twice and wrote the total in the ledger before she left.";
        let new = "\
The quiet river wound its lazy course past the drowsy hamlet under each grey dawn.

She, by long habit, tallied the takings and inked the sum into the ledger before departing.";
        let count_rw = |c: f32| {
            diff(old, new, Params { gran: Gran::Word, coalesce: c })
                .iter()
                .filter(|b| matches!(b, Block::Rewritten { .. }))
                .count()
        };
        // both paragraphs aligned as edits (no stray delete/insert of a whole para)
        let blocks = diff(old, new, Params { gran: Gran::Word, coalesce: 0.5 });
        assert!(!blocks.iter().any(|b| matches!(b, Block::Deleted(_) | Block::Inserted(_))));
        // low threshold coalesces BOTH; a middle one coalesces only the heavy
        // paragraph; a high one leaves both inline — three distinct cells.
        let (lo, mid, hi) = (count_rw(0.30), count_rw(0.55), count_rw(0.80));
        assert_eq!((lo, mid, hi), (2, 1, 0), "expected 2/1/0 rewrites, got {lo}/{mid}/{hi}");
    }

    // --- paragraph alignment ---

    #[test]
    fn alignment_same_insert_delete() {
        let old = "Alpha paragraph one.\n\nBeta paragraph two.";
        let new = "Alpha paragraph one.\n\nA brand new middle paragraph.\n\nBeta paragraph two.";
        let blocks = diff(old, new, Params::default());
        assert!(blocks.iter().any(|b| matches!(b, Block::Inserted(x) if x.contains("brand new"))));
        // two untouched paragraphs survive as folds
        assert!(blocks.iter().any(|b| matches!(b, Block::Fold(_))));
        assert!(!blocks.iter().any(|b| matches!(b, Block::Deleted(_))));
    }

    #[test]
    fn pure_deletion_is_a_deleted_block() {
        let old = "Keep this one.\n\nDrop this whole paragraph entirely.";
        let new = "Keep this one.";
        let blocks = diff(old, new, Params::default());
        assert!(blocks.iter().any(|b| matches!(b, Block::Deleted(x) if x.contains("Drop this"))));
    }

    // --- move detection (the prose-native requirement) ---

    #[test]
    fn relocated_paragraph_reads_as_moved_not_delete_plus_insert() {
        // three anchors; the "movable" paragraph jumps from the end to the front
        let old = "\
Anchor one stays put here.

Anchor two also stays.

The movable paragraph about migrating birds.";
        let new = "\
The movable paragraph about migrating birds.

Anchor one stays put here.

Anchor two also stays.";
        let blocks = diff(old, new, Params::default());
        assert!(
            blocks.iter().any(|b| matches!(b, Block::Moved { text, .. } if text.contains("migrating birds"))),
            "expected a Moved block, got {blocks:?}"
        );
        // and NOT a delete+insert pair for the same content
        assert!(!blocks.iter().any(|b| matches!(b, Block::Deleted(x) if x.contains("migrating birds"))));
        assert!(!blocks.iter().any(|b| matches!(b, Block::Inserted(x) if x.contains("migrating birds"))));
    }

    #[test]
    fn unrelated_swap_is_not_a_move() {
        // low-similarity swap: two unrelated paragraphs → delete+insert, no false move
        let old = "The quantum theory of gravitation.\n\nRecipes for sourdough bread.";
        let new = "Recipes for sourdough bread.\n\nThe quantum theory of gravitation.";
        let blocks = diff(old, new, Params::default());
        // identical paragraphs → the aligner matches one as backbone, the other moves;
        // this is a LEGITIMATE move (same text relocated), so assert we didn't split it
        // into a delete+insert of identical content.
        let moved = blocks.iter().filter(|b| matches!(b, Block::Moved { .. })).count();
        let del = blocks.iter().filter(|b| matches!(b, Block::Deleted(_))).count();
        let ins = blocks.iter().filter(|b| matches!(b, Block::Inserted(_))).count();
        assert!(moved >= 1);
        assert_eq!(del, 0);
        assert_eq!(ins, 0);
    }

    // --- serialization vocabulary ---

    #[test]
    fn transcript_uses_awls_vocabulary() {
        let old = "Keep me.\n\nDelete me completely please.";
        let new = "Keep me.\n\nA fresh inserted paragraph here.";
        let md = render_markdown(old, new, Params::default(), "T");
        assert!(md.starts_with("# T\n\n"));
        assert!(md.contains("~~")); // struck deletion — REAL markdown strikethrough
        assert!(md.contains("==")); // highlight-washed insertion
        assert!(md.contains("> ")); // blockquote (dim) for the deletion + folds
    }

    #[test]
    fn strike_wraps_per_line_and_keeps_whitespace_outside_markers() {
        // REAL `~~` markdown (the strikethrough-render round; the combining-
        // stroke `\u{0336}` mechanism is RETIRED — see `strike`'s doc). Each
        // hard-wrapped line carries its own pair (a cross-line pair would defeat
        // the line-scoped marker conceal), and lead/tail whitespace stays
        // OUTSIDE the markers (`~~ foo ~~` inner padding fails GFM's flanking
        // rules, exactly like `== foo ==` is inert in awl's scan).
        let s = strike("A short  digression\nspanning two lines.");
        assert_eq!(s, "~~A short  digression~~\n~~spanning two lines.~~");
        // Whitespace-only pieces stay unwrapped; indentation survives outside.
        assert_eq!(strike("  indented tail  "), "  ~~indented tail~~  ");
        assert_eq!(strike("word\n\nnext"), "~~word~~\n\n~~next~~");
        // No combining marks anywhere — the transcript is plain visible text.
        assert!(!s.contains('\u{0336}'));
        // And it rides the ONE wrapper the `==` wash shares (merge, don't align).
        assert_eq!(highlight_inline("a b\nc"), "==a b==\n==c==");
    }

    #[test]
    fn strip_markdown_removes_tildes_so_the_strike_wrap_never_nests() {
        // A literal `~~` in source prose would close the serializer's own wrap
        // early; the strip neutralizes tildes exactly like `*`/`_`/backticks.
        assert_eq!(strip_markdown("approx ~40 chars, ~~old style~~"), "approx 40 chars, old style");
    }

    #[test]
    fn render_is_deterministic() {
        let old = "One two three.\n\nFour five six seven.";
        let new = "One two three changed.\n\nFour five six seven.";
        let a = render_markdown(old, new, Params::default(), "T");
        let b = render_markdown(old, new, Params::default(), "T");
        assert_eq!(a, b);
    }

    // --- shipping recipe + sidecar counts ---

    #[test]
    fn shipping_recipe_is_sentence_half() {
        let p = Params::shipping();
        assert_eq!(p.gran, Gran::Sentence);
        assert_eq!(p.coalesce, 0.5);
    }

    #[test]
    fn count_blocks_tallies_each_kind() {
        // A deleted paragraph, an inserted one, and an untouched pair (folded).
        let old = "Keep me here.\n\nDrop this whole paragraph entirely.\n\nAnd keep this.";
        let new = "Keep me here.\n\nA fresh inserted paragraph here.\n\nAnd keep this.";
        let blocks = diff(old, new, Params::shipping());
        let c = count_blocks(&blocks);
        assert_eq!(c.struck, 1, "one struck deletion: {blocks:?}");
        assert_eq!(c.washed, 1, "one washed insertion: {blocks:?}");
        // The two untouched paragraphs survive as fold rows.
        assert!(c.folds >= 1, "at least one fold: {blocks:?}");
        assert_eq!(c.moved, 0);
    }

    #[test]
    fn count_blocks_rewrite_is_both_struck_and_washed() {
        // A heavily-reworded paragraph coalesces to old-struck-whole / new-washed-whole,
        // so it contributes to BOTH the struck and washed tallies (the sidecar's honest
        // view of "this paragraph shows a full crossed-out block AND a full wash").
        let old = "The cat sat quietly on the warm mat by the old door.";
        let new = "The cat sat nervously on the cold floor near the new window.";
        let blocks = diff(old, new, Params { gran: Gran::Word, coalesce: 0.3 });
        assert!(blocks.iter().any(|b| matches!(b, Block::Rewritten { .. })));
        let c = count_blocks(&blocks);
        assert_eq!((c.struck, c.washed), (1, 1), "{blocks:?}");
    }
}
