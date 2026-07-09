//! GFM tables: on-demand SOURCE alignment ([`align_table`], Prettier-style
//! re-padding) and the pixel GRID layout the WYSIWYG render uses
//! ([`table_column_layout`]/`table_pan_*`/[`table_align_offset`]), plus the
//! shared [`ColAlign`] + row/cell parsing both lean on. Split out of the
//! former `markdown.rs` monolith (2026-07 code-organization pass); every
//! item's path is unchanged (`markdown::align_table`, …) -- only the file
//! it lives in moved.

use super::{ConcealKind, MdKind};
use std::ops::Range;

/// Dim a GFM table's STRUCTURAL markup within its byte `range` (into `text`):
/// every literal `|` cell-delimiter pipe on a row becomes a [`MdKind::TablePipe`]
/// span, and the whole HEADER-SEPARATOR row (`|---|:--:|---|`) becomes one
/// [`MdKind::TableSep`] span. pulldown emits NO event for the pipes or the
/// separator row, so we derive both from the table's raw text — but we only ever
/// look INSIDE a range pulldown already ruled a table, so this never mis-fires on
/// a stray `|` in ordinary prose. awl is a SOURCE editor: the markup recedes to
/// the dim ink, no grid is ever drawn. The header/body CELL content is left to the
/// inline Text pass (header cells additionally get a [`MdKind::TableHeader`] tag
/// from the `TableCell` event); a pipe never overlaps a cell's content, so the
/// spans compose cleanly.
pub(super) fn push_table_markup(out: &mut Vec<(Range<usize>, MdKind)>, text: &str, range: &Range<usize>) {
    // The whole-table BLOCK conceal span (WYSIWYG): off the caret's block the
    // renderer hides every source row and draws a pixel GRID in its place; the
    // caret entering the block reveals the source and parks the grid (the
    // heading model — see `ConcealKind::Table`). Additive, laid FIRST so the
    // dim `TablePipe`/`TableSep`/`TableHeader` spans still ride the revealed source.
    out.push((range.clone(), MdKind::ConcealMarkup(ConcealKind::Table)));
    let s = &text[range.clone()];
    let mut off = 0usize; // byte offset of the current line, relative to `s`
    for (li, line) in s.split_inclusive('\n').enumerate() {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let base = range.start + off;
        // GFM's header-separator is ALWAYS the table's second line — guarding by
        // index (not shape alone) means a body cell whose content is literally `---`
        // is never mistaken for it.
        if li == 1 && is_separator_row(content) {
            // The whole `-`/`:`/`|` run (first to last non-whitespace) is one dim span.
            let lead = content.len() - content.trim_start().len();
            let tail = content.trim_end().len();
            if tail > lead {
                out.push((base + lead..base + tail, MdKind::TableSep));
            }
        } else {
            for (i, b) in content.bytes().enumerate() {
                if b == b'|' {
                    out.push((base + i..base + i + 1, MdKind::TablePipe));
                }
            }
        }
        off += line.len();
    }
}

/// True when `s` is a GFM table HEADER-SEPARATOR row — a non-empty line built only
/// of pipes / dashes / colons / spaces / tabs that contains at least one `-` (the
/// delimiter run under the header). pulldown consumes this row without an event, so
/// [`push_table_markup`] recognizes it by shape to dim it whole.
fn is_separator_row(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty()
        && t.contains('-')
        && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

/// A GFM column's alignment, parsed from its header-separator cell (`:---` left,
/// `---:` right, `:--:` center, `---` none). Drives how [`sep_cell`] re-emits the
/// separator's colons at the aligned column width; the DATA cells are always
/// left-aligned (padded on the right) in v1 — the markers are preserved for the
/// reader/other tools, not used to re-justify cell content.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ColAlign {
    None,
    Left,
    Right,
    Center,
}

/// Display width of `s` in monospace cells — a heuristic, since the codebase
/// bundles no `unicode-width` crate: a CJK / fullwidth scalar (the SAME broad
/// range set the renderer's `is_cjk` uses) counts as 2 columns, every other char
/// as 1. CAVEAT: this is not a full East-Asian-Width table (combining marks,
/// emoji ZWJ sequences, and a `\|` escape — counted as its two literal source
/// bytes' worth, 2 — are all approximated), but it matches how awl's own monospace
/// grid renders CJK, so the pipes line up for the common Latin+CJK case.
fn cell_display_width(s: &str) -> usize {
    s.chars()
        .map(|c| if is_wide_cell_char(c) { 2 } else { 1 })
        .sum()
}

/// Whether `c` occupies two monospace columns — the same CJK / fullwidth ranges
/// the renderer's `is_cjk` treats as a wide glyph (kept in sync by construction;
/// see [`cell_display_width`]'s caveat).
fn is_wide_cell_char(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F   // Hangul Jamo
        | 0x2E80..=0x303E // CJK radicals / Kangxi / symbols & punctuation
        | 0x3041..=0x33FF // Hiragana … CJK compatibility
        | 0x3400..=0x4DBF // CJK Ext A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs
        | 0xA000..=0xA4CF // Yi
        | 0xAC00..=0xD7A3 // Hangul syllables
        | 0xF900..=0xFAFF // CJK compatibility ideographs
        | 0xFE30..=0xFE4F // CJK compatibility forms
        | 0xFF00..=0xFF60 // fullwidth forms
        | 0xFFE0..=0xFFE6 // fullwidth signs
    )
}

/// Parse a header-separator cell (its content between two pipes, e.g. `:--:`) into
/// its [`ColAlign`]. Colons on both ends = center, left end = left, right end =
/// right, neither = none.
pub(crate) fn parse_col_align(cell: &str) -> ColAlign {
    let t = cell.trim();
    match (t.starts_with(':'), t.ends_with(':') && t.len() > 1) {
        (true, true) => ColAlign::Center,
        (true, false) => ColAlign::Left,
        (false, true) => ColAlign::Right,
        (false, false) => ColAlign::None,
    }
}

/// Split ONE table row's source into its trimmed cell contents, honoring a `\|`
/// escape (an escaped pipe is part of the cell, never a delimiter). The structural
/// empty cells produced by the leading/trailing outer pipes are dropped, so
/// `| a | b |` yields `["a", "b"]` and a pipeless line yields the whole line as one
/// cell.
pub(crate) fn split_row_cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let mut cells: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut escaped = false;
    for c in t.chars() {
        if escaped {
            cur.push(c);
            escaped = false;
        } else if c == '\\' {
            cur.push(c);
            escaped = true;
        } else if c == '|' {
            cells.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    cells.push(cur.trim().to_string());
    // Drop the empty cell before the first `|` / after the last `|` (the outer pipes).
    if t.starts_with('|') && cells.first().is_some_and(|c| c.is_empty()) {
        cells.remove(0);
    }
    if t.ends_with('|') && cells.last().is_some_and(|c| c.is_empty()) {
        cells.pop();
    }
    cells
}

/// Re-emit one column's SEPARATOR cell (`ColAlign` + target `width`), keeping the
/// alignment colons and filling the rest with `-` so its total width matches the
/// data cells. `width` is already floored to each align's minimum by [`align_table`].
fn sep_cell(align: ColAlign, width: usize) -> String {
    match align {
        ColAlign::None => "-".repeat(width),
        ColAlign::Left => format!(":{}", "-".repeat(width - 1)),
        ColAlign::Right => format!("{}:", "-".repeat(width - 1)),
        ColAlign::Center => format!(":{}:", "-".repeat(width - 2)),
    }
}

/// Re-pad ONE GFM table's source so every `|` lines up (Prettier-style monospace
/// alignment), returning the aligned lines joined by `\n` (no trailing newline).
///
/// Contract:
/// - Column count = the MAX cell count across all rows; RAGGED rows (missing
///   trailing cells) are padded with empty cells so every row has the same pipes.
/// - Each column's width = the max [`cell_display_width`] of its non-separator
///   cells, floored to what its alignment marker needs (none≥1, left/right≥2,
///   center≥3) so the re-emitted separator is always valid.
/// - Data cells are LEFT-aligned (padded on the right) with exactly one space of
///   padding inside each pipe: `| cell  | cell |`.
/// - The header-SEPARATOR row (always the second line of a GFM table) is re-emitted
///   as dashes at the column width with its `:` alignment markers PRESERVED.
/// - IDEMPOTENT: aligning already-aligned source returns it unchanged.
///
/// Width uses DISPLAY width (CJK = 2) where possible — see [`cell_display_width`]'s
/// caveat for the heuristic's limits. Pure; no clock, no allocation beyond output.
pub fn align_table(table_src: &str) -> String {
    let lines: Vec<&str> = table_src.split('\n').collect();
    // The separator is ALWAYS the 2nd line of a GFM table (guarded by index, like
    // `push_table_markup`), so a body cell of literal `---` is never mistaken for it.
    let sep_idx = 1usize;
    let rows: Vec<Vec<String>> = lines.iter().map(|l| split_row_cells(l)).collect();
    let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if ncols == 0 {
        return table_src.to_string();
    }
    // Per-column alignment, read from the separator row's cells (missing → none).
    let aligns: Vec<ColAlign> = (0..ncols)
        .map(|c| {
            rows.get(sep_idx)
                .and_then(|r| r.get(c))
                .map(|s| parse_col_align(s))
                .unwrap_or(ColAlign::None)
        })
        .collect();
    // Per-column width = max non-separator cell display width, floored to the
    // alignment marker's minimum so the separator re-emits validly.
    let mut widths = vec![0usize; ncols];
    for (ri, row) in rows.iter().enumerate() {
        if ri == sep_idx {
            continue;
        }
        for (ci, cell) in row.iter().enumerate() {
            widths[ci] = widths[ci].max(cell_display_width(cell));
        }
    }
    for (c, w) in widths.iter_mut().enumerate() {
        let min = match aligns[c] {
            ColAlign::None => 1,
            ColAlign::Left | ColAlign::Right => 2,
            ColAlign::Center => 3,
        };
        *w = (*w).max(min);
    }
    // Re-emit every row at the aligned widths.
    let mut out = Vec::with_capacity(lines.len());
    for (ri, row) in rows.iter().enumerate() {
        let mut s = String::from("|");
        for (c, width) in widths.iter().copied().enumerate() {
            s.push(' ');
            if ri == sep_idx {
                s.push_str(&sep_cell(aligns[c], width));
            } else {
                let cell = row.get(c).map(String::as_str).unwrap_or("");
                s.push_str(cell);
                for _ in cell_display_width(cell)..width {
                    s.push(' ');
                }
            }
            s.push(' ');
            s.push('|');
        }
        out.push(s);
    }
    out.join("\n")
}

/// Lay out a table's columns in pixels the CSS AUTO-TABLE way — the fix for the
/// "Da wn"/"Tim e" mid-word-break bug the old proportional-shrink clamp caused.
///
/// `mins[c]` is column `c`'s MIN-CONTENT floor: the widest UNBREAKABLE run in the
/// column (its longest word incl. the header) plus inner padding — a column NEVER
/// narrows below this, so a word NEVER breaks mid-word. `maxs[c]` is its
/// MAX-CONTENT width (the widest cell laid on one line + padding). `gap` is the
/// inter-column whitespace; `avail` is the writing-column width. Returns each
/// column's left x (relative to the text origin, 0-based) and final width.
///
/// Three regimes, exactly the CSS auto-table shape:
///  1. **Fits** — the max-content total ≤ `avail`: columns keep their natural
///     (max-content) widths, left-anchored. Nothing wraps.
///  2. **Squeeze** — min-content fits but max-content overflows: the surplus
///     (`avail − min_total`) is distributed across columns ∝ `(max − min)`, so
///     PHRASE columns (wide max−min spread) absorb the squeeze by WORD-boundary
///     wrapping while TOKEN columns (min == max) stay rigid. Total lands at
///     `avail`.
///  3. **Overflow** — the min-content floors themselves exceed `avail`: every
///     column sits at its min-content floor (a word still never breaks) and the
///     grid's total width EXCEEDS `avail`. It grows into the margins and, past
///     the visible width, PANS horizontally (`table_pan_*`). Mid-word breaks
///     only ever occur in the degenerate case of a single word wider than a whole
///     column — never from allocation.
///
/// The returned total (`xs.last() + ws.last()`) MAY exceed `avail` (regime 3) —
/// the caller draws into the margins and pans; it never silently shrinks a
/// column below its word floor. Pure; no clock, O(columns).
pub(crate) fn table_column_layout(
    mins: &[f32],
    maxs: &[f32],
    gap: f32,
    avail: f32,
) -> (Vec<f32>, Vec<f32>) {
    let n = maxs.len();
    if n == 0 {
        return (Vec::new(), Vec::new());
    }
    let gap = gap.max(0.0);
    let gaps_total = gap * (n - 1) as f32;
    // Each column's min floor is capped at its own max (a degenerate min > max —
    // e.g. a mis-measured single glyph — can never push a column past its content).
    let col_max: Vec<f32> = maxs.iter().map(|w| w.max(0.0)).collect();
    let col_min: Vec<f32> = (0..n)
        .map(|c| mins.get(c).copied().unwrap_or(0.0).max(0.0).min(col_max[c]))
        .collect();
    let max_total: f32 = col_max.iter().sum::<f32>() + gaps_total;
    let min_total: f32 = col_min.iter().sum::<f32>() + gaps_total;

    let widths: Vec<f32> = if max_total <= avail || max_total <= min_total + 1e-3 {
        // Regime 1 (fits) — and the degenerate no-spread case (min == max, nothing
        // to distribute): everything at max-content.
        col_max
    } else if min_total >= avail {
        // Regime 3 (overflow) — floors exceed the column; every column at its
        // min-content floor, the grid overflows into the margins / pans.
        col_min
    } else {
        // Regime 2 (squeeze) — grow each column from min toward max ∝ (max − min).
        let surplus = avail - min_total;
        let spread: f32 = (0..n).map(|c| col_max[c] - col_min[c]).sum();
        (0..n)
            .map(|c| {
                if spread > 0.0 {
                    col_min[c] + surplus * (col_max[c] - col_min[c]) / spread
                } else {
                    col_min[c]
                }
            })
            .collect()
    };

    let mut xs = Vec::with_capacity(n);
    let mut ws = Vec::with_capacity(n);
    let mut x = 0.0f32;
    for &w in &widths {
        xs.push(x);
        ws.push(w);
        x += w + gap;
    }
    (xs, ws)
}

/// The MAX pan offset (px) for a table whose laid-out grid is `content_w` wide
/// shown in a viewport `view_w` wide: `max(0, content_w − view_w)`. Zero when the
/// grid fits (nothing to pan). Pure — the clamp owner shared by the live gesture
/// and the indicator-bar geometry.
pub(crate) fn table_pan_max(content_w: f32, view_w: f32) -> f32 {
    (content_w - view_w).max(0.0)
}

/// Clamp a requested horizontal pan `offset` (px, ≥ 0 = grid shifted left) into
/// `[0, table_pan_max(content_w, view_w)]`. Pure; the ONE owner both the live
/// gesture and the draw path route through so a stale offset can never pan a
/// fitting (or now-narrower) grid off its rails.
pub(crate) fn table_pan_clamp(offset: f32, content_w: f32, view_w: f32) -> f32 {
    offset.max(0.0).min(table_pan_max(content_w, view_w))
}

/// Geometry of the THIN horizontal pan INDICATOR bar `[x, y, w, h]` for a table
/// that overflows its viewport, or `None` when the grid fits (`content_w ≤
/// view_w` → nothing to indicate). A scrollbar-thumb proportion: the bar's width
/// is `view_w²/content_w` (the visible fraction) and its left tracks the pan
/// (`pan/content_w` of the track). `left`/`bottom` are the table's viewport left
/// and bottom edges; `thick` the bar thickness. Value-step tint, never amber,
/// transient (drawn only while panning / on hover — a live-only concern). Pure +
/// unit-tested; the gesture that feeds `pan` is live-only.
pub(crate) fn table_pan_bar(
    content_w: f32,
    view_w: f32,
    pan: f32,
    left: f32,
    bottom: f32,
    thick: f32,
) -> Option<[f32; 4]> {
    if content_w <= view_w + 1e-3 || view_w <= 0.0 || content_w <= 0.0 {
        return None;
    }
    let pan = table_pan_clamp(pan, content_w, view_w);
    let frac = (view_w / content_w).clamp(0.0, 1.0);
    let bar_w = (view_w * frac).max(thick * 2.0).min(view_w);
    // The thumb's left rides the pan as a fraction of the SCROLLABLE track, so a
    // full pan lands the thumb flush against the viewport's right edge.
    let travel = (view_w - bar_w).max(0.0);
    let max_pan = table_pan_max(content_w, view_w);
    let t = if max_pan > 0.0 { pan / max_pan } else { 0.0 };
    let bar_x = left + travel * t;
    Some([bar_x, bottom - thick, bar_w, thick])
}

/// The horizontal offset (from a column box's LEFT edge) at which to place a cell
/// of shaped width `cell_w` inside a column of width `col_w`, honoring the
/// column's `align` and an inner `pad`. `None`/`Left` anchor at `pad`; `Right`
/// pushes the cell to `col_w - cell_w - pad`; `Center` splits the slack. Clamped
/// so an OVER-WIDE cell (wider than its column) always left-anchors at `pad` and
/// clips at the right edge rather than spilling left. Pure.
pub(crate) fn table_align_offset(align: ColAlign, col_w: f32, cell_w: f32, pad: f32) -> f32 {
    let raw = match align {
        ColAlign::None | ColAlign::Left => pad,
        ColAlign::Right => col_w - cell_w - pad,
        ColAlign::Center => (col_w - cell_w) * 0.5,
    };
    // The cell's left must sit in [pad, col_w - cell_w] (the right bound collapses
    // to `pad` for an over-wide cell, so both clamps agree on left-anchoring it).
    let hi = (col_w - cell_w).max(pad);
    raw.max(pad).min(hi)
}

/// A line "looks like" a GFM table row for BLOCK detection: trimmed non-empty and
/// containing at least one `|`. (A real table block must ALSO carry a separator
/// row — see [`table_block_lines`] — so pipe-bearing prose is never aligned.)
fn looks_like_table_row(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && t.contains('|')
}

/// The `[start, end)` LINE range of the GFM table containing `cursor_line`, or
/// `None` if the caret is not inside one. A table is the MAXIMAL run of consecutive
/// [`looks_like_table_row`] lines around the caret that ALSO contains a
/// header-separator row (`|---|`) — the separator requirement is what keeps a stray
/// run of pipe-bearing prose from being treated as a table. Pure; `lines` is the
/// document split on `\n`. Used by [`crate::keymap::Action::AlignTable`].
pub fn table_block_lines(lines: &[&str], cursor_line: usize) -> Option<(usize, usize)> {
    if cursor_line >= lines.len() || !looks_like_table_row(lines[cursor_line]) {
        return None;
    }
    let mut start = cursor_line;
    while start > 0 && looks_like_table_row(lines[start - 1]) {
        start -= 1;
    }
    let mut end = cursor_line + 1;
    while end < lines.len() && looks_like_table_row(lines[end]) {
        end += 1;
    }
    if lines[start..end].iter().any(|l| is_separator_row(l)) {
        Some((start, end))
    } else {
        None
    }
}
