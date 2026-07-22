//! FOLDS — collapse a markdown SECTION (an ATX heading plus the body + deeper
//! headings under it) to VIEW state only. A fold is NEVER file content: the rope
//! stays plain text, nothing is written to disk, and the set of folded headings
//! lives in memory on the [`crate::buffer::Buffer`] for the app run only (it
//! survives a buffer switch via the registry, NOT a relaunch, and is never on the
//! undo timeline). This module is the PURE logic — heading detection, a folded
//! heading's hidden-line extent, the "collapse other sections" set, and the
//! auto-expand rules — over a per-line heading-level vector and a `BTreeSet` of
//! folded heading lines. No rope, no render, no globals, so every rule is a
//! unit test at the purest seam.
//!
//! A "section" of a heading at line `h` of level `L` is the CONTIGUOUS run of
//! lines `h+1 ..= end-1` where `end` is the first later line that is itself a
//! heading of level `<= L` (a sibling or shallower heading), or the end of the
//! document. Folding `h` hides exactly that run — its body AND any deeper
//! headings nested inside it (so folding a parent hides its children whole). The
//! heading line `h` itself is NEVER hidden.

use std::collections::BTreeSet;

/// The heading LEVEL implied by a line's LEADING `#` run (after optional indent) —
/// `0` (not a heading) / `1` (`#`) / `2` (`##`) / `3+`. Mirrors
/// [`crate::render::spans::md_line_heading_level`] (the render SIZE half) EXACTLY so
/// a foldable section is precisely a sized heading: keyed off the raw hash COUNT,
/// not a fully-valid ATX heading, so `#foo` (no space) is a level-1 heading just as
/// it renders larger. `md` gates it: a non-markdown buffer has no headings, so
/// nothing is ever foldable there.
pub fn heading_level(line: &str, md: bool) -> u8 {
    if !md {
        return 0;
    }
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    let mut hashes = 0u8;
    while i < b.len() && b[i] == b'#' {
        hashes = hashes.saturating_add(1);
        i += 1;
    }
    hashes
}

/// The per-LOGICAL-LINE heading level for the whole document (`0` for body lines).
/// The one input every other function in this module reads, so the caller derives
/// it once per fold operation. Splitting on `\n` yields one entry per logical line
/// (matching the rope's line indexing the caret/selection use).
pub fn heading_levels(text: &str, md: bool) -> Vec<u8> {
    text.split('\n').map(|l| heading_level(l, md)).collect()
}

/// The half-open `[start, end)` range of lines a folded heading at `h` HIDES — its
/// body and every deeper nested heading, up to (but not including) the next
/// sibling-or-shallower heading (or the document end). `start` is always `h + 1`
/// (the heading line itself is never hidden). Returns `(h+1, h+1)` (empty) when `h`
/// is out of range or is not a heading (a stale fold whose heading was edited away).
pub fn section_range(levels: &[u8], h: usize) -> (usize, usize) {
    let n = levels.len();
    if h >= n || levels[h] == 0 {
        return (h + 1, h + 1);
    }
    let l = levels[h];
    let mut end = h + 1;
    while end < n && !(levels[end] != 0 && levels[end] <= l) {
        end += 1;
    }
    (h + 1, end)
}

/// The per-line HIDDEN mask: `hidden[i]` is true when line `i` is inside the
/// section of at least one folded heading. A folded heading's own line is never
/// hidden. Stale folds (an entry that is no longer a heading line) contribute
/// nothing. O(folds x section length); folds is small.
pub fn hidden_lines(levels: &[u8], folds: &BTreeSet<usize>) -> Vec<bool> {
    let n = levels.len();
    let mut hidden = vec![false; n];
    for &h in folds {
        let (s, e) = section_range(levels, h);
        for cell in hidden.iter_mut().take(e).skip(s) {
            *cell = true;
        }
    }
    hidden
}

/// The number of LINES a folded heading at `h` hides (its section length) — the
/// count shown in the quiet "… N lines" tail on a collapsed heading. `0` when `h`
/// is not a folded-worthy heading or its section is empty.
pub fn hidden_count(levels: &[u8], h: usize) -> usize {
    let (s, e) = section_range(levels, h);
    e.saturating_sub(s)
}

/// The "… N lines" TAIL data for every folded heading that is ITSELF still
/// VISIBLE — one `(heading_line, hidden_count)` per collapsed section whose
/// heading a reviewer / the render can see, ascending. A folded heading nested
/// inside another folded parent is HIDDEN (its own line collapses into the
/// parent's section), so it contributes NO tail — the parent's tail already
/// accounts for it in its own count. Empty when nothing is folded. PURE
/// (`heading_line` is a FULL-document line; the render remaps it into filtered
/// space via [`Filter::line`]).
pub fn fold_tails(levels: &[u8], folds: &BTreeSet<usize>) -> Vec<(usize, usize)> {
    if folds.is_empty() {
        return Vec::new();
    }
    let hidden = hidden_lines(levels, folds);
    folds
        .iter()
        .filter(|&&h| !hidden.get(h).copied().unwrap_or(false)) // the heading itself is visible
        .filter_map(|&h| {
            let n = hidden_count(levels, h);
            (n > 0).then_some((h, n))
        })
        .collect()
}

/// The innermost heading whose SECTION contains `line` — i.e. the nearest heading
/// at or before `line` that `line` sits under. When `line` IS a heading line, that
/// heading is returned (a caret on a heading toggles that heading). `None` when
/// `line` is body text before the first heading (no enclosing section).
pub fn enclosing_heading(levels: &[u8], line: usize) -> Option<usize> {
    let n = levels.len();
    if line < n && levels[line] != 0 {
        return Some(line);
    }
    // Scan backward for the nearest heading whose section still reaches `line`.
    let mut i = line.min(n.saturating_sub(1));
    loop {
        if levels[i] != 0 {
            let (_, e) = section_range(levels, i);
            if line < e {
                return Some(i);
            }
            // A shallower heading whose section ends before `line`: `line` is not
            // under it, and no earlier heading can reach past it either.
            return None;
        }
        if i == 0 {
            return None;
        }
        i -= 1;
    }
}

/// The set of headings to KEEP unfolded so the caret's section stays fully open:
/// the innermost enclosing heading of `line`, its ancestor chain (each shallower
/// enclosing heading), and every deeper heading nested inside the innermost one.
/// Empty when `line` is before the first heading (no section to preserve).
fn kept_open(levels: &[u8], line: usize) -> BTreeSet<usize> {
    let mut kept = BTreeSet::new();
    let Some(here) = enclosing_heading(levels, line) else {
        return kept;
    };
    kept.insert(here);
    // Ancestor chain: each strictly-shallower heading walking backward.
    let mut lvl = levels[here];
    let mut i = here;
    while i > 0 {
        i -= 1;
        if levels[i] != 0 && levels[i] < lvl {
            kept.insert(i);
            lvl = levels[i];
        }
    }
    // Descendants inside `here`'s section (all deeper headings).
    let (s, e) = section_range(levels, here);
    for (j, cell) in levels.iter().enumerate().take(e).skip(s) {
        if *cell != 0 {
            kept.insert(j);
        }
    }
    kept
}

/// "Collapse other sections" (the daily-notes gesture): fold EVERY heading except
/// the caret's section — its enclosing chain and everything nested inside it stay
/// open, every sibling / unrelated section collapses. When the caret is before the
/// first heading (no section), every heading folds.
pub fn collapse_others(levels: &[u8], caret_line: usize) -> BTreeSet<usize> {
    let keep = kept_open(levels, caret_line);
    (0..levels.len())
        .filter(|&i| levels[i] != 0 && !keep.contains(&i))
        .collect()
}

/// Toggle the fold on the heading enclosing `caret_line`. Returns the heading line
/// that was toggled (so the caller can leave the caret sensibly placed), or `None`
/// when there is no enclosing heading (nothing to fold).
pub fn toggle_at(levels: &[u8], folds: &mut BTreeSet<usize>, caret_line: usize) -> Option<usize> {
    let h = enclosing_heading(levels, caret_line)?;
    if !folds.remove(&h) {
        folds.insert(h);
    }
    Some(h)
}

/// AUTO-EXPAND (reveal): drop every folded heading whose section HIDES `line`, so a
/// caret / edit / jump landing inside a fold makes that fold's content visible
/// again. Removing ALL such folds (not just the innermost) reveals a line hidden by
/// nested folds in one step. Returns true when any fold was removed.
pub fn expand_containing(levels: &[u8], folds: &mut BTreeSet<usize>, line: usize) -> bool {
    let before = folds.len();
    folds.retain(|&h| {
        let (s, e) = section_range(levels, h);
        !(line >= s && line < e)
    });
    folds.len() != before
}

/// AUTO-EXPAND for a SELECTION: a selection must never span a fold INVISIBLY, so
/// reveal every folded heading whose hidden section intersects the inclusive line
/// range `lo..=hi`. Returns true when any fold was removed.
pub fn expand_range(levels: &[u8], folds: &mut BTreeSet<usize>, lo: usize, hi: usize) -> bool {
    let before = folds.len();
    folds.retain(|&h| {
        let (s, e) = section_range(levels, h);
        // Hidden lines are [s, e); the selection covers [lo, hi]. Reveal on any
        // overlap.
        !(s < e && lo < e && hi >= s)
    });
    folds.len() != before
}

/// Drop stale folds — entries that no longer name a heading line (the heading text
/// was edited so the leading `#` is gone). Keeps the fold set honest after edits so
/// a later re-typed `#` at that line does not silently re-collapse. Returns true
/// when any entry was pruned.
pub fn prune_stale(levels: &[u8], folds: &mut BTreeSet<usize>) -> bool {
    let before = folds.len();
    folds.retain(|&h| h < levels.len() && levels[h] != 0);
    folds.len() != before
}

/// The FOLD-FILTERED view of a document's text: the visible lines only (hidden
/// lines dropped), plus the index math to remap a full-document line to its index
/// in the filtered text. The render pipeline shapes [`Self::text`] — so a hidden
/// line is simply never laid out and contributes ZERO height (there is no separate
/// "zero-height row" bookkeeping; the row does not exist). When nothing is hidden
/// the text is byte-identical to the input and every remap is the identity, so an
/// unfolded document renders exactly as before.
pub struct Filter {
    /// The visible lines joined by `\n` — what the pipeline shapes.
    pub text: String,
    /// Per FULL-document line: the number of hidden lines with a strictly smaller
    /// index. A visible full line `i` sits at filtered index `i - prefix[i]`.
    prefix_hidden: Vec<usize>,
    /// The per-full-line hidden mask (borrowed shape kept for [`Self::visible`]).
    hidden: Vec<bool>,
}

impl Filter {
    /// Build the filtered text + remap table from the document text and a per-line
    /// hidden mask (as [`hidden_lines`] produces). `hidden` shorter/longer than the
    /// line count is tolerated (missing entries treated as visible).
    pub fn new(text: &str, hidden: &[bool]) -> Self {
        let lines: Vec<&str> = text.split('\n').collect();
        let mut prefix_hidden = Vec::with_capacity(lines.len());
        let mut running = 0usize;
        let mut kept: Vec<&str> = Vec::with_capacity(lines.len());
        for (i, line) in lines.iter().enumerate() {
            prefix_hidden.push(running);
            if hidden.get(i).copied().unwrap_or(false) {
                running += 1;
            } else {
                kept.push(line);
            }
        }
        Filter {
            text: kept.join("\n"),
            prefix_hidden,
            hidden: hidden.to_vec(),
        }
    }

    /// True when at least one line is hidden (so filtering actually changed the
    /// text). Cheap short-circuit for the common unfolded document.
    pub fn any_hidden(&self) -> bool {
        self.hidden.iter().any(|&h| h)
    }

    /// Is full-document line `full` visible (not hidden)?
    pub fn visible(&self, full: usize) -> bool {
        !self.hidden.get(full).copied().unwrap_or(false)
    }

    /// Remap a FULL-document line index to its index in the filtered text. A visible
    /// line maps exactly; a hidden line collapses onto the following visible line's
    /// index (callers drop hidden-line data rather than rely on that).
    pub fn line(&self, full: usize) -> usize {
        let p = self
            .prefix_hidden
            .get(full)
            .copied()
            .unwrap_or_else(|| self.prefix_hidden.last().copied().unwrap_or(0));
        full.saturating_sub(p)
    }

    /// Remap a FULL-document `(line, col)` position to filtered space (column
    /// unchanged — folding only removes whole lines).
    pub fn pos(&self, (l, c): (usize, usize)) -> (usize, usize) {
        (self.line(l), c)
    }
}

/// Apply a fold set to a fully-built [`crate::render::ViewState`]: replace `text`
/// with the fold-filtered document and remap `cursor_line` / `selection` /
/// `search_matches` / `misspelled` into that filtered line space, dropping any
/// search hit or misspelling that sits on a now-hidden line. The action-seam
/// auto-expand guarantees the caret and any selection endpoints are on VISIBLE
/// lines, so those remaps are exact. A no-op when the mask hides nothing (the text
/// and every coordinate stay byte-identical). Shared by the live `sync_view` and
/// the headless capture builder so the two flows cannot drift.
///
/// `tails` is the FULL-document `(heading_line, hidden_count)` list ([`fold_tails`]);
/// each entry's heading is remapped into filtered space here and recorded as a
/// [`crate::render::FoldTail`] so the render can hang the quiet "… N lines" glyph on
/// the collapsed heading's own row (the count is fold-derived, not re-shaped).
pub fn apply_to_view(
    view: &mut crate::render::ViewState,
    hidden: &[bool],
    tails: &[(usize, usize)],
) {
    let filter = Filter::new(&view.text, hidden);
    if !filter.any_hidden() {
        return;
    }
    view.fold_tails = tails
        .iter()
        .map(|&(h, n)| crate::render::FoldTail {
            line: filter.line(h),
            hidden: n,
        })
        .collect();
    view.cursor_line = filter.line(view.cursor_line);
    view.selection = view
        .selection
        .map(|(a, b)| (filter.pos(a), filter.pos(b)));
    view.search_matches = view
        .search_matches
        .iter()
        .copied()
        .filter(|(a, b)| filter.visible(a.0) && filter.visible(b.0))
        .map(|(a, b)| (filter.pos(a), filter.pos(b)))
        .collect();
    // Keep `search_current` in range after any hidden matches were dropped (the
    // real amber caret marks the current hit; this index only bounds the list).
    view.search_current = match view.search_current {
        Some(_) if view.search_matches.is_empty() => None,
        Some(i) => Some(i.min(view.search_matches.len() - 1)),
        None => None,
    };
    view.misspelled = std::mem::take(&mut view.misspelled)
        .into_iter()
        .filter(|m| filter.visible(m.line))
        .map(|mut m| {
            m.line = filter.line(m.line);
            m
        })
        .collect();
    view.text = filter.text;
}

#[cfg(test)]
mod tests;
