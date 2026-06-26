//! fzf-style subsequence fuzzy matching + ranking for the go-to overlay.
//!
//! A query matches a candidate when its chars appear IN ORDER (not necessarily
//! contiguous) somewhere in the candidate, case-insensitively. The score rewards
//! the qualities that make a match feel "right": contiguous runs, matches at the
//! start of the string or after a path separator / word boundary, and an overall
//! tighter span. On top of the raw character score we layer a TIER bias so the
//! product model's ranking (open buffers > recently-opened > corpus) is honored
//! regardless of the textual score.

/// Where a candidate sits in the ranking hierarchy. A higher tier ALWAYS sorts
/// above a lower one (it adds a large constant to the score), so an open buffer
/// beats a corpus file even on a weaker textual match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// A buffer currently open (the active file). Ranked highest.
    Open,
    /// A recently-opened file (MRU), but not currently open.
    Recent,
    /// Plain corpus file from the index.
    Corpus,
}

impl Tier {
    fn bias(self) -> i64 {
        match self {
            Tier::Open => 1_000_000,
            Tier::Recent => 500_000,
            Tier::Corpus => 0,
        }
    }
}

/// Score `candidate` against `query` (subsequence match). Returns `None` if the
/// query is not a subsequence of the candidate. An empty query matches every
/// candidate with a neutral score of 0 (so the list shows in its natural order).
/// Higher is better.
pub fn score(query: &str, candidate: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    let cand: Vec<char> = candidate.chars().collect();
    let cl: Vec<char> = cand
        .iter()
        .flat_map(|c| c.to_lowercase())
        .collect::<Vec<_>>();
    // `cl` (lowercased) may differ in length from `cand` for some scripts; for
    // the ascii paths we deal with they match 1:1, which keeps boundary checks
    // simple. Guard by using indices into `cand` only when lengths agree.
    let same_len = cl.len() == cand.len();

    let mut qi = 0usize;
    let mut score: i64 = 0;
    let mut prev_match: Option<usize> = None;
    let mut first_match: Option<usize> = None;

    for (ci, &cc) in cl.iter().enumerate() {
        if qi >= q.len() {
            break;
        }
        if cc == q[qi] {
            if first_match.is_none() {
                first_match = Some(ci);
            }
            // Base reward for a matched char.
            score += 10;
            // Contiguity bonus: adjacent to the previous match.
            if let Some(p) = prev_match {
                if ci == p + 1 {
                    score += 15;
                }
            }
            // Boundary bonus: at index 0, or right after a separator / word
            // boundary in the ORIGINAL candidate (so capital-camel and path
            // segments score higher, fzf-style).
            let at_boundary = ci == 0
                || (same_len
                    && ci > 0
                    && matches!(cand[ci - 1], '/' | '_' | '-' | '.' | ' '))
                || (same_len && ci > 0 && cand[ci - 1].is_lowercase() && cand[ci].is_uppercase());
            if at_boundary {
                score += 20;
            }
            prev_match = Some(ci);
            qi += 1;
        }
    }

    if qi != q.len() {
        return None; // not a full subsequence
    }

    // Penalize a sprawling match span (prefer tight matches) and a late start.
    // The span penalty is weighted so a scattered match (chars spread across the
    // string) loses to a contiguous run even when the scattered one collects more
    // boundary bonuses — contiguity is the dominant signal, fzf-style.
    if let (Some(f), Some(l)) = (first_match, prev_match) {
        let span = (l - f) as i64;
        let q_len = q.len() as i64;
        // Extra gap beyond the minimum (a perfectly contiguous match has span ==
        // q_len - 1, i.e. zero extra gap).
        let extra_gap = (span - (q_len - 1)).max(0);
        score -= extra_gap * 5; // each gap cell hurts
        score -= f as i64 / 4; // earlier start -> higher score
    }
    Some(score)
}

/// A scored, ranked candidate. `index` is the candidate's position in the input
/// slice (so the caller can map back to the original path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ranked {
    pub index: usize,
    pub score: i64,
}

/// Rank `candidates` against `query`, dropping non-matches. `tier(i)` reports the
/// ranking tier of candidate `i` (open / recent / corpus); its bias is added to
/// the textual score so the product-model hierarchy is honored. Results are
/// sorted best-first; ties break by the candidate's original order (stable),
/// giving deterministic output for capture verification.
pub fn rank(
    query: &str,
    candidates: &[String],
    mut tier: impl FnMut(usize) -> Tier,
) -> Vec<Ranked> {
    let mut out: Vec<Ranked> = Vec::new();
    for (i, c) in candidates.iter().enumerate() {
        if let Some(s) = score(query, c) {
            out.push(Ranked {
                index: i,
                score: s + tier(i).bias(),
            });
        }
    }
    // Sort by score desc, then by original index asc for a stable, deterministic
    // order (so equal-score corpus files keep their sorted-path order).
    out.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus(_: usize) -> Tier {
        Tier::Corpus
    }

    #[test]
    fn subsequence_matches_and_rejects() {
        assert!(score("mn", "src/main.rs").is_some());
        assert!(score("xyz", "src/main.rs").is_none());
        assert!(score("", "anything").is_some());
    }

    #[test]
    fn mn_ranks_main() {
        let cands = vec![
            "src/lib.rs".to_string(),
            "src/main.rs".to_string(),
            "README.md".to_string(),
        ];
        let r = rank("mn", &cands, corpus);
        assert!(!r.is_empty());
        assert_eq!(cands[r[0].index], "src/main.rs", "mn -> main.rs, got {r:?}");
    }

    #[test]
    fn prefix_beats_scattered() {
        // "rea" is a contiguous prefix of "README.md" but scattered in "src/area.rs".
        let cands = vec!["src/area.rs".to_string(), "README.md".to_string()];
        let r = rank("rea", &cands, corpus);
        assert_eq!(cands[r[0].index], "README.md", "prefix should win: {r:?}");
    }

    #[test]
    fn contiguous_prefix_beats_scattered() {
        // "env" is a contiguous boundary-anchored run in ".env"; in "e_n_v.rs"
        // the chars are split across separators (not contiguous), so it scores
        // lower despite also matching.
        let contiguous = score("env", ".env").unwrap();
        let scattered = score("env", "early/nope/vague.rs").unwrap();
        assert!(
            contiguous > scattered,
            "contiguous {contiguous} vs scattered {scattered}"
        );
    }

    #[test]
    fn open_tier_beats_corpus() {
        let cands = vec!["src/main.rs".to_string(), "src/zzz_main.rs".to_string()];
        // Even though zzz_main also matches, mark main.rs as Open so it wins.
        let r = rank("main", &cands, |i| {
            if i == 0 {
                Tier::Open
            } else {
                Tier::Corpus
            }
        });
        assert_eq!(cands[r[0].index], "src/main.rs");
    }
}
