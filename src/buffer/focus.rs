//! FOCUS-MODE UNIT BOUNDS — the pure `&str` helpers that compute the ACTIVE UNIT
//! (paragraph / sentence) char range around a cursor for focus mode. Free functions
//! (not `Buffer` methods) so the render path and the headless sidecar can compute
//! the identical range from `ViewState.text` without owning a `Buffer`. Carved out
//! of `buffer.rs` verbatim; glob-re-exported from the module root so the
//! `crate::buffer::paragraph_bounds_str` / `sentence_bounds_str` call sites resolve
//! unchanged.

// --- FOCUS-MODE unit bounds (pure, over &str) -----------------------------
//
// These compute the ACTIVE UNIT char range around a cursor for focus mode. They
// are free functions over `&str` (not just `Buffer` methods) so the render path
// and the headless sidecar can compute the identical range from `ViewState.text`
// without owning a `Buffer`. Char-indexed throughout, matching the rest of awl's
// caret / selection model (1 char = 1 column).

/// Per-line char spans `(start, end)` of `text`, where `end` is EXCLUSIVE of the
/// line's trailing newline. There is one entry per line (so a trailing newline
/// yields a final empty line), mirroring how the editor counts lines.
fn line_char_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = 0usize;
    for part in text.split('\n') {
        let len = part.chars().count();
        spans.push((start, start + len));
        start += len + 1; // +1 for the '\n' between lines
    }
    spans
}

/// The char range `[start, end)` of the PARAGRAPH containing `idx`: the maximal run
/// of consecutive NON-BLANK lines around `idx`'s line, delimited by blank lines
/// (a blank line is empty or all-whitespace). When `idx` sits on a BLANK line — the
/// gap between paragraphs, common mid-write — there is no paragraph under the cursor;
/// lighting that empty line would dim the WHOLE page, so we RE-ANCHOR onto the nearest
/// non-blank paragraph: prefer the one ABOVE (the paragraph just finished, where the
/// eye rests), else the nearest BELOW (cursor on leading blank lines). Only an
/// ALL-BLANK document yields an empty range. The returned range excludes the trailing
/// newline of the last line. Robust on empty text (returns `(0, 0)`).
pub fn paragraph_bounds_str(text: &str, idx: usize) -> (usize, usize) {
    let spans = line_char_spans(text);
    let lines: Vec<&str> = text.split('\n').collect();
    if spans.is_empty() {
        return (0, 0);
    }
    let n = text.chars().count();
    let idx = idx.min(n);
    // The line containing idx: the last line whose start is <= idx (the next line's
    // start is end+1 > idx whenever idx is within this line, incl. at its end).
    let li = spans.iter().rposition(|&(s, _)| s <= idx).unwrap_or(0);
    let is_blank = |i: usize| lines[i].trim().is_empty();
    // The line whose paragraph we light. On a blank line, bias to the nearest real
    // paragraph (above, then below); an all-blank document collapses to an empty
    // range at the cursor's line start.
    let anchor = if !is_blank(li) {
        li
    } else if let Some(above) = (0..li).rev().find(|&i| !is_blank(i)) {
        above
    } else if let Some(below) = (li + 1..lines.len()).find(|&i| !is_blank(i)) {
        below
    } else {
        return (spans[li].0, spans[li].0);
    };
    let mut top = anchor;
    while top > 0 && !is_blank(top - 1) {
        top -= 1;
    }
    let mut bot = anchor;
    while bot + 1 < lines.len() && !is_blank(bot + 1) {
        bot += 1;
    }
    (spans[top].0, spans[bot].1)
}

/// True when the line of `text` (as `chars`) containing `idx` is BLANK — empty or
/// all-whitespace. Used by focus to tell "cursor resting in the gap between
/// paragraphs" (re-anchor onto real prose) from "cursor between two sentences on the
/// same line" (keep the forward bias).
fn line_is_blank_at(chars: &[char], idx: usize) -> bool {
    let n = chars.len();
    let idx = idx.min(n);
    let mut ls = idx;
    while ls > 0 && chars[ls - 1] != '\n' {
        ls -= 1;
    }
    let mut le = idx;
    while le < n && chars[le] != '\n' {
        le += 1;
    }
    chars[ls..le].iter().all(|c| c.is_whitespace())
}

/// The nearest non-whitespace char index to `idx`, preferring the closest one ABOVE
/// (`< idx`, the prose just written) and falling back to the closest BELOW (`>= idx`).
/// `None` only when the whole document is whitespace. Lets focus re-anchor off a blank
/// gap onto real text, prefer-above-then-below, so the page is never fully dimmed.
fn nearest_nonblank_char(chars: &[char], idx: usize) -> Option<usize> {
    let idx = idx.min(chars.len());
    let mut i = idx;
    while i > 0 {
        i -= 1;
        if !chars[i].is_whitespace() {
            return Some(i);
        }
    }
    (idx..chars.len()).find(|&j| !chars[j].is_whitespace())
}

/// The sentence span `[s, e)` around `anchor`, splitting on a terminator (`.`/`!`/`?`)
/// followed by whitespace/EOF, biasing a between-sentences `anchor` FORWARD to the
/// upcoming sentence. The pure walk shared by [`sentence_bounds_str`]; may be empty
/// when `anchor` is past the last sentence (only trailing whitespace ahead).
fn sentence_span_at(chars: &[char], anchor: usize) -> (usize, usize) {
    let n = chars.len();
    let is_term = |c: char| c == '.' || c == '!' || c == '?';
    // A sentence BOUNDARY closes at position `i` when chars[i] is a terminator and
    // the next char is whitespace or the end of the buffer.
    let boundary_at = |i: usize| -> bool {
        is_term(chars[i]) && (i + 1 >= n || chars[i + 1].is_whitespace())
    };
    // START: walk left until the char to the left closes the previous sentence,
    // then skip the whitespace that follows that terminator (biasing a between-
    // sentences cursor forward onto the upcoming sentence).
    let mut s = anchor.min(n);
    while s > 0 && !boundary_at(s - 1) {
        s -= 1;
    }
    while s < n && chars[s].is_whitespace() {
        s += 1;
    }
    // END: walk right from the start through the next closing terminator (inclusive).
    let mut e = s;
    while e < n && !boundary_at(e) {
        e += 1;
    }
    if e < n {
        e += 1; // include the terminator itself
    }
    (s, e.max(s))
}

/// The char range `[start, end)` of the SENTENCE containing `idx`. Sentences split
/// on a terminator (`.`/`!`/`?`) that is followed by whitespace/newline or the end
/// of the buffer; the returned range starts at the first non-whitespace char after
/// the previous terminator and ends just past the terminator that closes the
/// sentence. When the cursor sits BETWEEN sentences on a line of prose (in the
/// whitespace after a terminator), the bias is FORWARD to the upcoming sentence.
/// When the cursor rests on a BLANK line (the gap between paragraphs, or leading/
/// trailing blank lines) there is no sentence under it — forward-biasing through that
/// whitespace could run off the buffer end and return an EMPTY range, greying the
/// whole page — so we RE-ANCHOR onto the nearest real sentence, preferring the one
/// ABOVE (just finished), else the first BELOW. Only an ALL-BLANK document yields an
/// empty range. Robust at the buffer start/end and on empty text (returns `(0, 0)`).
pub fn sentence_bounds_str(text: &str, idx: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n == 0 {
        return (0, 0);
    }
    let idx = idx.min(n);
    // On a blank line, anchor on the nearest real sentence (above, then below) rather
    // than forward-biasing into the void; on a line of prose, keep `idx` so the
    // between-sentences forward bias still holds.
    let anchor = if line_is_blank_at(&chars, idx) {
        match nearest_nonblank_char(&chars, idx) {
            Some(a) => a,
            None => return (idx, idx), // all-blank document
        }
    } else {
        idx
    };
    let (s, e) = sentence_span_at(&chars, anchor);
    if e > s {
        return (s, e);
    }
    // Safety net: an empty span (e.g. the cursor in trailing whitespace at EOF on an
    // otherwise non-blank line) must never dim the whole page when prose exists —
    // re-anchor onto the nearest real sentence (prefer above).
    if let Some(a) = nearest_nonblank_char(&chars, anchor) {
        return sentence_span_at(&chars, a);
    }
    (s, e)
}
