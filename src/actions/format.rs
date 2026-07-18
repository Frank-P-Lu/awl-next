//! The MARKDOWN FORMATTING-COMMAND engine — the pure toggle transforms behind the
//! block / inline format Actions (`ToggleBlockquote`, `Bold`, …). Every command is a
//! TOGGLE (apply the format when it is absent on the target, STRIP it when present)
//! and lands as ONE atomic, undoable edit through [`crate::buffer::Buffer::apply_format`]
//! (a whole-buffer replace never coalesces, so a single Cmd-Z reverts each toggle).
//!
//! The transforms are PURE — a document's text plus its selection/cursor (char
//! indices) go in, the new text plus the selection to restore over it come out
//! ([`FormatResult`]) — so they are unit-testable without a `Buffer`, GPU, or clock.
//! `apply_core`'s arms call the two thin wrappers ([`apply_block_format`] /
//! [`apply_inline_format`]) which read the buffer's text + selection, run the pure
//! transform, and apply the result; a transform that changes nothing is a calm no-op
//! (no edit, so undo stays meaningful) exactly like the align-table command.
//!
//! TWO FAMILIES:
//!   * BLOCK toggles operate on the SELECTED LINES (or the caret line with no
//!     selection): a per-line prefix (`> `, `- `, `1. `, `- [ ] `, `# `) placed AFTER
//!     any leading indentation so the toggle round-trips, or a fenced wrapper
//!     (`CodeBlock`) placed above/below the range.
//!   * INLINE toggles operate on the SELECTION within a line (or the word under the
//!     caret, or empty delimiters with the caret between them): a symmetric wrapper
//!     (`**`, `*`, `` ` ``, `==`, `~~`).

use super::*;

/// A pure format transform's result: the WHOLE new document text plus the selection
/// to restore over it (char indices INTO THE NEW TEXT). `anchor == None` is a bare
/// caret; `Some(a)` selects `[a.min(cursor), a.max(cursor)]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FormatResult {
    pub text: String,
    pub anchor: Option<usize>,
    pub cursor: usize,
}

/// The BLOCK format toggles — a per-line prefix, or the fenced `CodeBlock` wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BlockKind {
    Blockquote,
    Bullet,
    Numbered,
    Task,
    Heading,
    CodeBlock,
}

/// The INLINE format toggles — a symmetric delimiter pair around the selection.
/// `pub(crate)`: the format POPOVER (`crate::actions::popover`) names these kinds
/// to read each button's active/lit state through [`inline_active`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InlineKind {
    Bold,
    Italic,
    InlineCode,
    Highlight,
    Strikethrough,
}

impl InlineKind {
    /// The delimiter this kind wraps a span with (both sides, symmetric).
    fn delim(self) -> &'static str {
        match self {
            InlineKind::Bold => "**",
            InlineKind::Italic => "*",
            InlineKind::InlineCode => "`",
            InlineKind::Highlight => "==",
            InlineKind::Strikethrough => "~~",
        }
    }
}

// --- Wrappers driven by apply_core ------------------------------------------

/// Run a BLOCK toggle over the caret line / selection and apply it as one undoable
/// edit. A markdown-only command (a `.rs`/`.txt` buffer is left untouched — block
/// markup would corrupt code), and a calm no-op when the transform changes nothing.
pub(super) fn apply_block_format(ctx: &mut ActionCtx, kind: BlockKind) {
    if !ctx.buffer.is_markdown() {
        return;
    }
    let text = ctx.buffer.text();
    let anchor = ctx.buffer.anchor_char();
    let cursor = ctx.buffer.cursor_char();
    let r = block_toggle(kind, &text, anchor, cursor);
    ctx.buffer.apply_format(&r.text, r.anchor, r.cursor);
}

/// Run an INLINE toggle over the selection / word under the caret and apply it as one
/// undoable edit. Markdown-only, calm no-op when nothing changes.
pub(super) fn apply_inline_format(ctx: &mut ActionCtx, kind: InlineKind) {
    if !ctx.buffer.is_markdown() {
        return;
    }
    let text = ctx.buffer.text();
    let anchor = ctx.buffer.anchor_char();
    let cursor = ctx.buffer.cursor_char();
    let r = inline_toggle(kind, &text, anchor, cursor);
    ctx.buffer.apply_format(&r.text, r.anchor, r.cursor);
}

// --- Shared line helpers ----------------------------------------------------

/// The document split into logical lines (newline-excluded), always ≥ 1 element —
/// `split('\n')` roundtrips exactly with `join('\n')` (a trailing `\n` yields a
/// trailing empty line).
fn split_lines(text: &str) -> Vec<String> {
    text.split('\n').map(str::to_string).collect()
}

/// Char index of the first char of line `l` (its start-of-line position).
fn line_start_char(lines: &[String], l: usize) -> usize {
    lines[..l].iter().map(|s| s.chars().count() + 1).sum()
}

/// (line, col) of an absolute char index over `lines`.
fn char_to_line_col(lines: &[String], idx: usize) -> (usize, usize) {
    let mut acc = 0;
    for (l, line) in lines.iter().enumerate() {
        let len = line.chars().count();
        if idx <= acc + len {
            return (l, idx - acc);
        }
        acc += len + 1; // + the newline
    }
    let last = lines.len() - 1;
    (last, lines[last].chars().count())
}

/// The selection as `(start, end, has_selection)` char indices (ordered). No mark, or
/// a mark exactly at the cursor, is a bare caret (`start == end`, `has == false`).
pub(super) fn sel_range(anchor: Option<usize>, cursor: usize) -> (usize, usize, bool) {
    match anchor {
        Some(a) if a != cursor => (a.min(cursor), a.max(cursor), true),
        _ => (cursor, cursor, false),
    }
}

/// Leading indentation (spaces / tabs) length, in chars.
fn indent_len(line: &[char]) -> usize {
    line.iter().take_while(|&&c| c == ' ' || c == '\t').count()
}

/// True when `line[from..]` begins with `pat`.
fn starts_with_at(line: &[char], from: usize, pat: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    from + p.len() <= line.len() && line[from..from + p.len()] == p[..]
}

/// If `line[from..]` opens with an ordered-list marker (`\d+` then `.`/`)` then a
/// space), the marker's char length; else `None`.
fn numbered_prefix_len(line: &[char], from: usize) -> Option<usize> {
    let mut d = from;
    while d < line.len() && line[d].is_ascii_digit() {
        d += 1;
    }
    if d > from && d + 1 < line.len() && matches!(line[d], '.' | ')') && line[d + 1] == ' ' {
        Some((d - from) + 2)
    } else {
        None
    }
}

// --- BLOCK toggle -----------------------------------------------------------

/// The prefix a per-line block kind APPLIES at the indentation boundary. `seq` is the
/// 1-based position among the lines being prefixed (only `Numbered` reads it).
fn block_prefix(kind: BlockKind, seq: usize) -> String {
    match kind {
        BlockKind::Blockquote => "> ".to_string(),
        BlockKind::Bullet => "- ".to_string(),
        BlockKind::Numbered => format!("{seq}. "),
        BlockKind::Task => "- [ ] ".to_string(),
        BlockKind::Heading => "# ".to_string(),
        BlockKind::CodeBlock => String::new(), // handled by the fenced-wrapper branch
    }
}

/// The char length of the prefix already present on `line` for `kind` (after indent),
/// or `None` when the line does not carry that kind's prefix.
fn present_prefix_len(kind: BlockKind, line: &[char], ind: usize) -> Option<usize> {
    match kind {
        BlockKind::Blockquote => starts_with_at(line, ind, "> ").then_some(2),
        BlockKind::Bullet => starts_with_at(line, ind, "- ").then_some(2),
        BlockKind::Numbered => numbered_prefix_len(line, ind),
        BlockKind::Task => {
            if starts_with_at(line, ind, "- [ ] ")
                || starts_with_at(line, ind, "- [x] ")
                || starts_with_at(line, ind, "- [X] ")
            {
                Some(6)
            } else {
                None
            }
        }
        BlockKind::Heading => starts_with_at(line, ind, "# ").then_some(2),
        BlockKind::CodeBlock => None,
    }
}

/// True when `line` (trimmed) is a fenced-code marker line (``` … ```).
fn is_fence(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

/// Toggle a BLOCK format over the caret line / selected lines. See the module doc.
fn block_toggle(kind: BlockKind, text: &str, anchor: Option<usize>, cursor: usize) -> FormatResult {
    let lines = split_lines(text);
    let (s, e, has_sel) = sel_range(anchor, cursor);
    let (first, _) = char_to_line_col(&lines, s);
    let (mut last, end_col) = char_to_line_col(&lines, e);
    // A selection ending at column 0 does not pull in that trailing line.
    if has_sel && last > first && end_col == 0 {
        last -= 1;
    }

    if kind == BlockKind::CodeBlock {
        return code_block_toggle(&lines, first, last, has_sel);
    }

    // Toggle direction: STRIP iff every NON-EMPTY affected line already carries the
    // prefix; else APPLY to every non-empty line (blank lines are left untouched, no
    // trailing-marker litter — mirrors the indent engine).
    let chars: Vec<Vec<char>> = lines.iter().map(|s| s.chars().collect()).collect();
    let nonempty: Vec<usize> = (first..=last)
        .filter(|&l| !lines[l].trim().is_empty())
        .collect();
    let all_prefixed = !nonempty.is_empty()
        && nonempty
            .iter()
            .all(|&l| present_prefix_len(kind, &chars[l], indent_len(&chars[l])).is_some());
    let strip = all_prefixed;

    let mut new_lines = lines.clone();
    // Per-line operation on the CARET line, for the no-selection cursor remap.
    let mut first_op: (i64, usize) = (0, 0); // (signed delta, indentation position)
    let mut seq = 0usize;
    for l in first..=last {
        if lines[l].trim().is_empty() {
            continue; // blank line: never prefixed / stripped
        }
        let line = &chars[l];
        let ind = indent_len(line);
        let (rebuilt, delta, at): (String, i64, usize) = if strip {
            let plen = present_prefix_len(kind, line, ind).unwrap_or(0);
            let mut v: Vec<char> = line[..ind].to_vec();
            v.extend_from_slice(&line[ind + plen..]);
            (v.into_iter().collect(), -(plen as i64), ind)
        } else {
            seq += 1;
            let prefix = block_prefix(kind, seq);
            let mut v: Vec<char> = line[..ind].to_vec();
            v.extend(prefix.chars());
            v.extend_from_slice(&line[ind..]);
            (v.into_iter().collect(), prefix.chars().count() as i64, ind)
        };
        if l == first {
            first_op = (delta, at);
        }
        new_lines[l] = rebuilt;
    }

    let new_text = new_lines.join("\n");
    let (anchor, cursor) = if has_sel {
        // Re-select the whole affected line range (block commands act on whole lines).
        let a = line_start_char(&new_lines, first);
        let c = line_start_char(&new_lines, last) + new_lines[last].chars().count();
        (Some(a), c)
    } else {
        // Bare caret: remap the original column through the caret line's own delta.
        let (_, col) = char_to_line_col(&lines, cursor);
        let (delta, at) = first_op;
        let new_col = remap_col(col, delta, at);
        (None, line_start_char(&new_lines, first) + new_col)
    };
    FormatResult { text: new_text, anchor, cursor }
}

/// Map a column through a single-line prefix add/strip: `delta > 0` inserted `delta`
/// chars at `at`; `delta < 0` removed `-delta` chars starting at `at`.
fn remap_col(col: usize, delta: i64, at: usize) -> usize {
    if delta > 0 {
        if col >= at {
            col + delta as usize
        } else {
            col
        }
    } else if delta < 0 {
        let plen = (-delta) as usize;
        if col <= at {
            col
        } else if col >= at + plen {
            col - plen
        } else {
            at
        }
    } else {
        col
    }
}

/// The fenced `CodeBlock` wrapper toggle: unwrap when the range's first + last lines
/// are already fence markers, else wrap the range in ``` lines above and below.
fn code_block_toggle(lines: &[String], first: usize, last: usize, _has_sel: bool) -> FormatResult {
    let already = last > first && is_fence(&lines[first]) && is_fence(&lines[last]);
    if already {
        // UNWRAP: drop the opening + closing fence lines; select what remains between.
        let mut new_lines: Vec<String> = Vec::with_capacity(lines.len() - 2);
        new_lines.extend_from_slice(&lines[..first]);
        new_lines.extend_from_slice(&lines[first + 1..last]);
        new_lines.extend_from_slice(&lines[last + 1..]);
        let inner = last - first - 1; // body line count
        let new_text = new_lines.join("\n");
        let (anchor, cursor) = if inner > 0 {
            let a = line_start_char(&new_lines, first);
            let body_last = first + inner - 1;
            let c = line_start_char(&new_lines, body_last) + new_lines[body_last].chars().count();
            (Some(a), c)
        } else {
            (None, line_start_char(&new_lines, first))
        };
        FormatResult { text: new_text, anchor, cursor }
    } else {
        // WRAP: ``` above the range and ``` below it; select the whole fenced block.
        let mut new_lines: Vec<String> = Vec::with_capacity(lines.len() + 2);
        new_lines.extend_from_slice(&lines[..first]);
        new_lines.push("```".to_string());
        new_lines.extend_from_slice(&lines[first..=last]);
        new_lines.push("```".to_string());
        new_lines.extend_from_slice(&lines[last + 1..]);
        let new_text = new_lines.join("\n");
        let close = last + 2; // index of the closing fence in new_lines
        let a = line_start_char(&new_lines, first);
        let c = line_start_char(&new_lines, close) + new_lines[close].chars().count();
        FormatResult { text: new_text, anchor: Some(a), cursor: c }
    }
}

// --- HEADING CYCLE (the format popover's `H` button) ------------------------

/// The ATX heading level of one line's `chars` (measured AFTER `ind` leading
/// indentation): the count of consecutive leading `#` immediately followed by a
/// space, `1..=6`, else `0`. A bare `#word` (no space) or `####### ` (7+, past
/// markdown's max) is NOT a heading — level `0`.
fn line_heading_level(line: &[char], ind: usize) -> usize {
    let mut h = ind;
    while h < line.len() && line[h] == '#' {
        h += 1;
    }
    let n = h - ind;
    if (1..=6).contains(&n) && h < line.len() && line[h] == ' ' {
        n
    } else {
        0
    }
}

/// The char length of the heading prefix already present on `line` (after `ind`):
/// `level + 1` (the `#`s plus the one space), or `0` when the line is not a heading.
fn heading_prefix_char_len(line: &[char], ind: usize) -> usize {
    let lvl = line_heading_level(line, ind);
    if lvl > 0 {
        lvl + 1
    } else {
        0
    }
}

/// The next level in the popover `H` button's cycle: off → H1 → H2 → H3 → off. An
/// already-deeper heading (H4–H6, only reachable by hand-typing) also cycles back
/// to off, so the button always lands the user in the 1–3 band or clears it.
fn next_heading_level(cur: usize) -> usize {
    match cur {
        0 => 1,
        1 => 2,
        2 => 3,
        _ => 0,
    }
}

/// The heading level the format popover's `H` button reflects: the level of the
/// FIRST non-empty affected line (caret line / first selected line), `0` when none.
/// PURE; the popover's state-reflective oracle.
pub(crate) fn heading_level(text: &str, anchor: Option<usize>, cursor: usize) -> usize {
    let lines = split_lines(text);
    let (s, e, has_sel) = sel_range(anchor, cursor);
    let (first, _) = char_to_line_col(&lines, s);
    let (mut last, end_col) = char_to_line_col(&lines, e);
    if has_sel && last > first && end_col == 0 {
        last -= 1;
    }
    let chars: Vec<Vec<char>> = lines.iter().map(|s| s.chars().collect()).collect();
    (first..=last)
        .find(|&l| !lines[l].trim().is_empty())
        .map(|l| line_heading_level(&chars[l], indent_len(&chars[l])))
        .unwrap_or(0)
}

/// Run the popover `H` button's heading CYCLE over the caret line / selected
/// lines, applying it as one undoable edit. Markdown-only; a calm no-op elsewhere.
pub(super) fn apply_heading_cycle(ctx: &mut ActionCtx) {
    if !ctx.buffer.is_markdown() {
        return;
    }
    let text = ctx.buffer.text();
    let anchor = ctx.buffer.anchor_char();
    let cursor = ctx.buffer.cursor_char();
    let r = heading_cycle(&text, anchor, cursor);
    ctx.buffer.apply_format(&r.text, r.anchor, r.cursor);
}

/// Cycle the heading level of the caret line / selected lines: the target level is
/// `next_heading_level` of the FIRST non-empty affected line's current level, and
/// every non-empty affected line is rewritten to that level (its old `#…` prefix
/// stripped, the target `#…` prefix applied at the indentation boundary). Blank
/// lines are left untouched (mirrors `block_toggle`).
fn heading_cycle(text: &str, anchor: Option<usize>, cursor: usize) -> FormatResult {
    let lines = split_lines(text);
    let (s, e, has_sel) = sel_range(anchor, cursor);
    let (first, _) = char_to_line_col(&lines, s);
    let (mut last, end_col) = char_to_line_col(&lines, e);
    if has_sel && last > first && end_col == 0 {
        last -= 1;
    }
    let chars: Vec<Vec<char>> = lines.iter().map(|s| s.chars().collect()).collect();
    let cur = (first..=last)
        .find(|&l| !lines[l].trim().is_empty())
        .map(|l| line_heading_level(&chars[l], indent_len(&chars[l])))
        .unwrap_or(0);
    let target = next_heading_level(cur);
    let prefix: String = if target > 0 {
        format!("{} ", "#".repeat(target))
    } else {
        String::new()
    };
    let new_len = prefix.chars().count();

    let mut new_lines = lines.clone();
    // (signed delta at the caret line, indentation pos, old-prefix char len) — the
    // no-selection cursor remap reads these off the caret's own line.
    let mut first_op: (i64, usize, usize) = (0, 0, 0);
    for l in first..=last {
        if lines[l].trim().is_empty() {
            continue;
        }
        let line = &chars[l];
        let ind = indent_len(line);
        let existing = heading_prefix_char_len(line, ind);
        let mut v: Vec<char> = line[..ind].to_vec();
        v.extend(prefix.chars());
        v.extend_from_slice(&line[ind + existing..]);
        if l == first {
            first_op = (new_len as i64 - existing as i64, ind, existing);
        }
        new_lines[l] = v.into_iter().collect();
    }

    let new_text = new_lines.join("\n");
    let (anchor, cursor) = if has_sel {
        let a = line_start_char(&new_lines, first);
        let c = line_start_char(&new_lines, last) + new_lines[last].chars().count();
        (Some(a), c)
    } else {
        let (_, col) = char_to_line_col(&lines, cursor);
        let (_, at, existing) = first_op;
        // Model the prefix rewrite as STRIP the old `existing` chars at `at`, then
        // INSERT the new `new_len` chars at `at` — composing the SAME `remap_col`
        // the single-prefix block toggle uses, so a caret at the line start rides an
        // inserted heading prefix (like typing) and clamps sanely on a strip.
        let stripped = remap_col(col, -(existing as i64), at);
        let new_col = remap_col(stripped, new_len as i64, at);
        (None, line_start_char(&new_lines, first) + new_col)
    };
    FormatResult { text: new_text, anchor, cursor }
}

// --- INLINE toggle ----------------------------------------------------------

/// True when a char is part of a word (for the no-selection word-wrap).
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The char span an inline toggle acts on: the selection, or the word under the
/// caret, or (with neither) a bare caret (`want_caret`). Shared by [`inline_toggle`]
/// and the popover's [`inline_active`] so the two can never disagree on WHICH span
/// a button reads.
fn inline_span(chars: &[char], anchor: Option<usize>, cursor: usize) -> (usize, usize, bool) {
    let (s, e, has_sel) = sel_range(anchor, cursor);
    if has_sel {
        return (s, e, false);
    }
    let mut a = cursor;
    while a > 0 && is_word_char(chars[a - 1]) {
        a -= 1;
    }
    let mut b = cursor;
    while b < chars.len() && is_word_char(chars[b]) {
        b += 1;
    }
    if b > a {
        (a, b, false)
    } else {
        (cursor, cursor, true) // no word → empty-delimiter insert
    }
}

/// WHERE `kind`'s delimiters sit relative to span `[ws, we)`, when it is already
/// wrapped — the ONE definition of "already formatted" shared by the toggle (it
/// STRIPs) and the popover's lit oracle ([`inline_active`], it LIGHTS), so they
/// can never disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineWrap {
    /// The delimiters immediately SURROUND the span (`**|foo|**`, and the empty
    /// `**||**` toggle-off where `ws == we`) — strip removes them just outside.
    Surrounding,
    /// The span itself OPENS+CLOSES with the delimiters (a fully selected
    /// `**foo**`) — strip removes them from the span's own ends.
    Enclosing,
}

/// Decide whether `[ws, we)` is already wrapped by `kind`, and if so where its
/// delimiters sit. The syntactic delimiter match is the CANDIDATE; a NON-empty
/// span is then CONFIRMED against the real markdown parse ([`content_is_kind`]),
/// which is what makes a single `*` that is actually half of a `**` bold fence
/// read as NOT-italic — the lit-I-inside-bold fix. Without that confirmation the
/// old code saw `**bold**` as `*` + `*bold*` + `*`, lit the popover's I inside
/// bold, and STRIPPED the inner pair (degrading bold to italic) on toggle. An
/// EMPTY delimited span (`**||**`) has no content to parse and stays purely
/// syntactic (the empty-delimiter toggle-off round-trip).
fn inline_wrap(kind: InlineKind, chars: &[char], text: &str, ws: usize, we: usize) -> Option<InlineWrap> {
    let d: Vec<char> = kind.delim().chars().collect();
    let dl = d.len();
    let eq = |from: usize| from + dl <= chars.len() && chars[from..from + dl] == d[..];
    let (plan, content_empty) = if ws >= dl && we + dl <= chars.len() && eq(ws - dl) && eq(we) {
        (InlineWrap::Surrounding, ws == we)
    } else if we - ws >= 2 * dl && eq(ws) && eq(we - dl) {
        (InlineWrap::Enclosing, we - ws == 2 * dl)
    } else {
        return None;
    };
    if content_empty || content_is_kind(kind, text, ws, we) {
        Some(plan)
    } else {
        None
    }
}

/// True when the (non-empty) span `[ws, we)` renders as `kind` under the REAL
/// markdown parse ([`crate::markdown::spans`]) — the disambiguator that tells a
/// genuine `*italic*` from a `**bold**` fence's `*`s. Probes ONE char index
/// strictly inside the span: its byte offset lands in the CONTENT run whether the
/// span abuts the delimiters (surrounding) or encloses them (fully selected), so a
/// single probe serves both [`InlineWrap`] arms.
fn content_is_kind(kind: InlineKind, text: &str, ws: usize, we: usize) -> bool {
    let mid_char = ws + (we - ws) / 2;
    let mid_byte = char_to_byte(text, mid_char);
    crate::markdown::spans(text)
        .into_iter()
        .any(|(r, k)| r.contains(&mid_byte) && kind_matches_span(kind, k))
}

/// Which [`crate::markdown::MdKind`] content span(s) mean "already `kind`". Bold /
/// Italic each also match `BoldItalic` (so I lights inside `***both***` and toggles
/// it back to bold, `**both**`), which is exactly why the naive syntactic check
/// couldn't be trusted alone.
fn kind_matches_span(kind: InlineKind, k: crate::markdown::MdKind) -> bool {
    use crate::markdown::MdKind;
    match kind {
        InlineKind::Bold => matches!(k, MdKind::Bold | MdKind::BoldItalic),
        InlineKind::Italic => matches!(k, MdKind::Italic | MdKind::BoldItalic),
        InlineKind::InlineCode => matches!(k, MdKind::Code { inline: true }),
        InlineKind::Highlight => matches!(k, MdKind::Highlight),
        InlineKind::Strikethrough => matches!(k, MdKind::Strikethrough),
    }
}

/// Byte offset of char index `char_idx` into `text` (its length when past the end).
fn char_to_byte(text: &str, char_idx: usize) -> usize {
    text.char_indices().nth(char_idx).map(|(b, _)| b).unwrap_or(text.len())
}

/// Whether the format popover's inline button for `kind` should draw LIT — i.e.
/// toggling it would STRIP (the selection / caret-word is already wrapped). PURE;
/// the popover's active-state oracle, routed through the SAME [`inline_span`] +
/// [`inline_wrap`] the toggle uses.
pub(crate) fn inline_active(kind: InlineKind, text: &str, anchor: Option<usize>, cursor: usize) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let (ws, we, _) = inline_span(&chars, anchor, cursor);
    inline_wrap(kind, &chars, text, ws, we).is_some()
}

/// Toggle an INLINE format over the selection / word under the caret. See module doc.
fn inline_toggle(kind: InlineKind, text: &str, anchor: Option<usize>, cursor: usize) -> FormatResult {
    let chars: Vec<char> = text.chars().collect();
    let d: Vec<char> = kind.delim().chars().collect();
    let dl = d.len();

    // The span to (un)wrap: the selection, or the word under the caret. With neither,
    // insert empty delimiters with the caret between them.
    let (ws, we, want_caret) = inline_span(&chars, anchor, cursor);

    // STRIP — the span is already wrapped by `kind`. WHERE the delimiters sit comes
    // from the ONE shared owner [`inline_wrap`] (the same the popover lights from),
    // so the toggle can never strip a `*` that is really half of a `**` bold fence:
    // pressing I inside `**bold**` falls through to WRAP → `***bold***`, never a
    // silent bold→italic degrade.
    match inline_wrap(kind, &chars, text, ws, we) {
        Some(InlineWrap::Surrounding) => {
            // Delimiters immediately surround the span (the round-trip path; also the
            // empty-delimiter toggle-off, caret between the two and `ws == we`).
            let mut out: Vec<char> = Vec::with_capacity(chars.len() - 2 * dl);
            out.extend_from_slice(&chars[..ws - dl]);
            out.extend_from_slice(&chars[ws..we]);
            out.extend_from_slice(&chars[we + dl..]);
            let (a, c) = (ws - dl, we - dl);
            return finish_inline(out, ws == we, a, c);
        }
        Some(InlineWrap::Enclosing) => {
            // The span itself begins and ends with the delimiters (selected `**x**`).
            let mut out: Vec<char> = Vec::with_capacity(chars.len() - 2 * dl);
            out.extend_from_slice(&chars[..ws]);
            out.extend_from_slice(&chars[ws + dl..we - dl]);
            out.extend_from_slice(&chars[we..]);
            let (a, c) = (ws, we - 2 * dl);
            return finish_inline(out, false, a, c);
        }
        None => {}
    }

    // WRAP — insert the delimiters around the span (empty span → caret between).
    let mut out: Vec<char> = Vec::with_capacity(chars.len() + 2 * dl);
    out.extend_from_slice(&chars[..ws]);
    out.extend_from_slice(&d);
    out.extend_from_slice(&chars[ws..we]);
    out.extend_from_slice(&d);
    out.extend_from_slice(&chars[we..]);
    if want_caret {
        // Empty delimiters: bare caret between the two delimiters.
        let c = ws + dl;
        FormatResult { text: out.into_iter().collect(), anchor: None, cursor: c }
    } else {
        // Keep the selection over the same visible text (now inside the delimiters).
        let (a, c) = (ws + dl, we + dl);
        finish_inline(out, false, a, c)
    }
}

/// Assemble an inline [`FormatResult`], collapsing to a bare caret when the visible
/// span is now empty (`empty == true`).
fn finish_inline(out: Vec<char>, empty: bool, a: usize, c: usize) -> FormatResult {
    let text: String = out.into_iter().collect();
    if empty || a == c {
        FormatResult { text, anchor: None, cursor: c }
    } else {
        FormatResult { text, anchor: Some(a), cursor: c }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply a BLOCK toggle to `text` with the selection `[anchor, cursor]`.
    fn blk(kind: BlockKind, text: &str, anchor: Option<usize>, cursor: usize) -> FormatResult {
        block_toggle(kind, text, anchor, cursor)
    }
    /// Apply an INLINE toggle to `text` with the selection `[anchor, cursor]`.
    fn inl(kind: InlineKind, text: &str, anchor: Option<usize>, cursor: usize) -> FormatResult {
        inline_toggle(kind, text, anchor, cursor)
    }

    // --- BLOCK: apply ------------------------------------------------------

    #[test]
    fn blockquote_applies_to_the_caret_line() {
        let r = blk(BlockKind::Blockquote, "hello\nworld\n", None, 2);
        assert_eq!(r.text, "> hello\nworld\n");
        // The caret rode the inserted prefix (col 2 -> col 4), no selection.
        assert_eq!(r.anchor, None);
        assert_eq!(r.cursor, 4);
    }

    #[test]
    fn bullet_prefixes_every_selected_line() {
        // Select lines 0..=1 (the whole "a" and "b" lines).
        let r = blk(BlockKind::Bullet, "a\nb\nc\n", Some(0), 3);
        assert_eq!(r.text, "- a\n- b\nc\n");
    }

    #[test]
    fn numbered_list_renumbers_on_apply() {
        let r = blk(BlockKind::Numbered, "one\ntwo\nthree\n", Some(0), 11);
        assert_eq!(r.text, "1. one\n2. two\n3. three\n");
    }

    #[test]
    fn task_list_applies_open_checkbox() {
        let r = blk(BlockKind::Task, "todo\n", None, 0);
        assert_eq!(r.text, "- [ ] todo\n");
    }

    #[test]
    fn heading_toggles_one_hash() {
        let r = blk(BlockKind::Heading, "Title\n", None, 0);
        assert_eq!(r.text, "# Title\n");
    }

    // --- HEADING CYCLE (the popover `H` button) ---------------------------

    #[test]
    fn heading_cycle_walks_off_1_2_3_off() {
        // off → H1
        let a = heading_cycle("Title\n", None, 0);
        assert_eq!(a.text, "# Title\n");
        assert_eq!(heading_level(&a.text, a.anchor, a.cursor), 1);
        // H1 → H2
        let b = heading_cycle(&a.text, a.anchor, a.cursor);
        assert_eq!(b.text, "## Title\n");
        // H2 → H3
        let c = heading_cycle(&b.text, b.anchor, b.cursor);
        assert_eq!(c.text, "### Title\n");
        // H3 → off
        let d = heading_cycle(&c.text, c.anchor, c.cursor);
        assert_eq!(d.text, "Title\n");
        assert_eq!(heading_level(&d.text, d.anchor, d.cursor), 0);
    }

    #[test]
    fn heading_cycle_caret_rides_the_prefix() {
        // Caret at col 0 of "Title": off→H1 pushes it past the inserted "# ".
        let a = heading_cycle("Title\n", None, 0);
        assert_eq!(a.cursor, 2, "caret rode the inserted `# ` prefix");
        // A caret INSIDE the word stays with the word (shifts by the prefix len).
        let b = heading_cycle("Title\n", None, 3);
        assert_eq!(&b.text[..], "# Title\n");
        assert_eq!(b.cursor, 5);
    }

    #[test]
    fn heading_cycle_applies_one_level_to_all_selected_lines() {
        // Two lines selected, mixed levels → both land at the FIRST line's next level.
        let src = "# one\ntwo\n"; // first line is H1, second is plain
        let r = heading_cycle(src, Some(0), 9);
        // first line H1 → H2, and the whole range is rewritten to H2.
        assert_eq!(r.text, "## one\n## two\n");
    }

    #[test]
    fn heading_level_reads_the_first_nonempty_line() {
        assert_eq!(heading_level("plain\n", None, 0), 0);
        assert_eq!(heading_level("## sec\n", None, 3), 2);
        // A bare `#word` (no space) is not a heading.
        assert_eq!(heading_level("#notation\n", None, 2), 0);
    }

    // --- INLINE active-state oracle (the popover lit test) ----------------

    #[test]
    fn inline_active_matches_the_toggle_strip_condition() {
        // Unformatted selection → not active.
        assert!(!inline_active(InlineKind::Bold, "the quick fox", Some(4), 9));
        // Inner selection of an existing wrap → active (surrounding delimiters).
        assert!(inline_active(InlineKind::Bold, "the **quick** fox", Some(6), 11));
        // Fully-selected wrapped span → active (span begins+ends with delimiters).
        assert!(inline_active(InlineKind::Bold, "a **beta** c", Some(2), 10));
        // Caret-word (no selection) inside a wrap → active.
        assert!(inline_active(InlineKind::Italic, "a *word* here", None, 4));
    }

    /// The lit-I-inside-bold TRUTH TABLE: `**` is bold's fence, not two italic
    /// markers. The disambiguator ([`content_is_kind`]) reads the real markdown
    /// parse so a bare `*` inside a `**` never lights I.
    #[test]
    fn inline_active_disambiguates_bold_from_italic() {
        // `*i*` — a genuine lone-`*` italic: I active, B not.
        assert!(inline_active(InlineKind::Italic, "*i*", None, 1), "*i* is italic");
        assert!(!inline_active(InlineKind::Bold, "*i*", None, 1), "*i* is not bold");
        // `**b**` — plain bold: B active, I DARK (the fix; pre-fix I lit here).
        assert!(inline_active(InlineKind::Bold, "**b**", None, 2), "**b** is bold");
        assert!(!inline_active(InlineKind::Italic, "**b**", None, 2), "**b** is NOT italic");
        // `***bi***` — both: I and B both active (I strips to `**bi**`).
        assert!(inline_active(InlineKind::Italic, "***bi***", None, 4), "***bi*** is italic too");
        assert!(inline_active(InlineKind::Bold, "***bi***", None, 4), "***bi*** is bold too");
        // `**a *i* b**` — a nested `*i*` inside bold: I active on "i", and DARK on
        // the plain-bold "a" (I inside plain bold text = not active).
        assert!(inline_active(InlineKind::Italic, "**a *i* b**", None, 5), "italic on the nested i");
        assert!(!inline_active(InlineKind::Italic, "**a *i* b**", None, 2), "I dark on plain-bold a");
        // EDGE — fully SELECTING `**b**` (delimiters included): B active, I dark.
        assert!(inline_active(InlineKind::Bold, "**b**", Some(0), 5), "select-all **b** is bold");
        assert!(!inline_active(InlineKind::Italic, "**b**", Some(0), 5), "select-all **b** not italic");
        // EDGE — caret ON the opening marker (no word) lights nothing.
        assert!(!inline_active(InlineKind::Italic, "*i*", None, 0), "caret on the * marker: dark");
    }

    /// Pressing I inside plain bold WRAPS italic inside it (`***bold***`), never
    /// strips the bold — the (b) half of the lit-I bug — and the result renders as
    /// bold+italic; a second I strips the italic back to plain bold (round-trip).
    #[test]
    fn toggling_italic_inside_bold_wraps_then_strips_back() {
        let a = inl(InlineKind::Italic, "**bold**", None, 4);
        assert_eq!(a.text, "***bold***", "I wraps italic inside bold, no bold→italic degrade");
        let rendered = crate::markdown::spans(&a.text);
        assert!(
            rendered.iter().any(|(_, k)| *k == crate::markdown::MdKind::BoldItalic),
            "***bold*** renders bold+italic: {rendered:?}"
        );
        let b = inl(InlineKind::Italic, &a.text, a.anchor, a.cursor);
        assert_eq!(b.text, "**bold**", "second I strips the italic, bold survives");
    }

    #[test]
    fn block_prefix_lands_after_indentation() {
        // Indented line: the marker goes AFTER the leading spaces (round-trips).
        let r = blk(BlockKind::Bullet, "  item\n", None, 4);
        assert_eq!(r.text, "  - item\n");
    }

    // --- BLOCK: strip / round-trip -----------------------------------------

    #[test]
    fn blockquote_round_trips() {
        let src = "hello\nworld\n";
        let a = blk(BlockKind::Blockquote, src, None, 2);
        // Re-toggle the SAME (now prefixed) line strips it back to the original.
        let b = blk(BlockKind::Blockquote, &a.text, a.anchor, a.cursor);
        assert_eq!(b.text, src, "apply then strip restores the original text");
    }

    #[test]
    fn bullet_multiline_round_trips() {
        let src = "a\nb\nc\n";
        let a = blk(BlockKind::Bullet, src, Some(0), 3);
        assert_eq!(a.text, "- a\n- b\nc\n");
        // The apply left a full-line selection over lines 0..=1; re-toggle strips.
        let b = blk(BlockKind::Bullet, &a.text, a.anchor, a.cursor);
        assert_eq!(b.text, src);
    }

    #[test]
    fn numbered_list_round_trips() {
        let src = "one\ntwo\nthree\n";
        let a = blk(BlockKind::Numbered, src, Some(0), 11);
        let b = blk(BlockKind::Numbered, &a.text, a.anchor, a.cursor);
        assert_eq!(b.text, src);
    }

    #[test]
    fn task_list_round_trips_and_strips_a_checked_box() {
        // Applied then stripped restores the plain line...
        let src = "todo\n";
        let a = blk(BlockKind::Task, src, None, 0);
        let b = blk(BlockKind::Task, &a.text, a.anchor, a.cursor);
        assert_eq!(b.text, src);
        // ...and a CHECKED box also counts as a task prefix and strips.
        let checked = blk(BlockKind::Task, "- [x] done\n", None, 8);
        assert_eq!(checked.text, "done\n");
    }

    #[test]
    fn heading_round_trips() {
        let src = "Title\n";
        let a = blk(BlockKind::Heading, src, None, 0);
        let b = blk(BlockKind::Heading, &a.text, a.anchor, a.cursor);
        assert_eq!(b.text, src);
    }

    #[test]
    fn blank_lines_in_a_selection_are_left_untouched() {
        // "a\n\nb" — the blank middle line gets no marker; both real lines do.
        let src = "a\n\nb\n";
        let r = blk(BlockKind::Bullet, src, Some(0), 4);
        assert_eq!(r.text, "- a\n\n- b\n");
    }

    #[test]
    fn selection_ending_at_col_zero_excludes_the_trailing_line() {
        // Select "a\n" exactly (end at line 1 col 0): only line 0 is prefixed.
        let r = blk(BlockKind::Bullet, "a\nb\n", Some(0), 2);
        assert_eq!(r.text, "- a\nb\n");
    }

    // --- BLOCK: fenced code wrapper ----------------------------------------

    #[test]
    fn code_block_wraps_then_unwraps() {
        let src = "let x = 1;\nlet y = 2;\n";
        let a = blk(BlockKind::CodeBlock, src, Some(0), 21);
        assert_eq!(a.text, "```\nlet x = 1;\nlet y = 2;\n```\n");
        // The apply selected the whole fenced block; re-toggle unwraps it.
        let b = blk(BlockKind::CodeBlock, &a.text, a.anchor, a.cursor);
        assert_eq!(b.text, src);
    }

    #[test]
    fn code_block_wraps_a_single_line_with_no_selection() {
        let r = blk(BlockKind::CodeBlock, "code\n", None, 2);
        assert_eq!(r.text, "```\ncode\n```\n");
    }

    // --- INLINE: apply -----------------------------------------------------

    #[test]
    fn bold_wraps_the_selection() {
        // Select "quick" in "the quick fox" (cols 4..9).
        let r = inl(InlineKind::Bold, "the quick fox", Some(4), 9);
        assert_eq!(r.text, "the **quick** fox");
        // The selection now covers the same visible text, inside the delimiters.
        assert_eq!((r.anchor, r.cursor), (Some(6), 11));
    }

    #[test]
    fn italic_wraps_the_selection() {
        let r = inl(InlineKind::Italic, "a word here", Some(2), 6);
        assert_eq!(r.text, "a *word* here");
    }

    #[test]
    fn inline_code_wraps_the_selection() {
        let r = inl(InlineKind::InlineCode, "call foo now", Some(5), 8);
        assert_eq!(r.text, "call `foo` now");
    }

    #[test]
    fn highlight_and_strikethrough_wrap() {
        let h = inl(InlineKind::Highlight, "mark me", Some(0), 4);
        assert_eq!(h.text, "==mark== me");
        let s = inl(InlineKind::Strikethrough, "cut me", Some(0), 3);
        assert_eq!(s.text, "~~cut~~ me");
    }

    // --- INLINE: strip / round-trip ----------------------------------------

    #[test]
    fn bold_round_trips_via_surrounding_delimiters() {
        let src = "the quick fox";
        let a = inl(InlineKind::Bold, src, Some(4), 9);
        // Re-toggle with the wrapped selection strips the surrounding delimiters.
        let b = inl(InlineKind::Bold, &a.text, a.anchor, a.cursor);
        assert_eq!(b.text, src, "apply then strip restores the original text");
        assert_eq!((b.anchor, b.cursor), (Some(4), 9), "selection back over the same text");
    }

    #[test]
    fn every_inline_kind_round_trips() {
        for kind in [
            InlineKind::Bold,
            InlineKind::Italic,
            InlineKind::InlineCode,
            InlineKind::Highlight,
            InlineKind::Strikethrough,
        ] {
            let src = "alpha beta gamma";
            let a = inl(kind, src, Some(6), 10); // "beta"
            let b = inl(kind, &a.text, a.anchor, a.cursor);
            assert_eq!(b.text, src, "{kind:?} must round-trip");
        }
    }

    #[test]
    fn stripping_a_fully_selected_wrapped_span() {
        // Selection covers the delimiters too: "**beta**" selected -> strip inner.
        let text = "a **beta** c";
        let r = inl(InlineKind::Bold, text, Some(2), 10);
        assert_eq!(r.text, "a beta c");
    }

    // --- INLINE: no selection ----------------------------------------------

    #[test]
    fn no_selection_wraps_the_word_under_the_caret() {
        // Caret inside "quick" (col 6) with no selection wraps the whole word.
        let r = inl(InlineKind::Bold, "the quick fox", None, 6);
        assert_eq!(r.text, "the **quick** fox");
        assert_eq!((r.anchor, r.cursor), (Some(6), 11), "selection over the wrapped word");
    }

    #[test]
    fn no_selection_no_word_inserts_empty_delimiters_with_caret_between() {
        // Caret on a blank line: insert "**" and place the caret between them.
        let r = inl(InlineKind::Bold, "one\n\ntwo\n", None, 4);
        assert_eq!(r.text, "one\n****\ntwo\n");
        assert_eq!(r.anchor, None, "empty delimiters leave a bare caret");
        assert_eq!(r.cursor, 6, "caret sits between the two delimiters");
    }

    #[test]
    fn no_selection_empty_delimiters_round_trip() {
        // Insert empty delimiters on a blank line, then a second toggle with the caret
        // between them removes them again (the span between the delimiters is empty).
        let a = inl(InlineKind::Italic, "one\n\ntwo\n", None, 4);
        assert_eq!(a.text, "one\n**\ntwo\n");
        assert_eq!(a.cursor, 5, "caret sits between the two delimiters");
        let b = inl(InlineKind::Italic, &a.text, a.anchor, a.cursor);
        assert_eq!(b.text, "one\n\ntwo\n", "toggling empty delimiters off restores the text");
        assert_eq!(b.cursor, 4, "caret lands where the delimiters were");
    }
}
