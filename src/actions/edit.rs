//! The MARKDOWN smart-Enter edit — the one dispatch arm whose behavior is richer
//! than a bare buffer call. `apply_core`'s `Newline` arm asks [`smart_newline`] to
//! continue a list / blockquote (ordered lists AUTO-INCREMENT), END the block on an
//! empty item, or carry leading indentation forward; a `false` return falls through
//! to a plain `insert_newline`, byte-identical to before. The DECISION ([`SmartNewline`]
//! + [`smart_newline_for`]) is pure over one line's text + cursor column, so it is
//! unit-testable without a buffer/GPU. Carved out of `actions.rs` VERBATIM.

use super::*;

/// MARKDOWN-only smart Enter. Returns `true` when it performed the edit; `false`
/// tells the caller to do a plain `insert_newline`. Reads only the current line's
/// text + cursor column and mutates through the buffer's atomic edit seam, so it
/// stays pure and `--keys`-drivable (live and replay can't drift). Gated on
/// `is_markdown`, and skipped while a selection is active (a plain newline, which
/// overwrites the selection, is the right thing there).
pub(super) fn smart_newline(ctx: &mut ActionCtx) -> bool {
    if !ctx.buffer.is_markdown() || ctx.buffer.has_selection() {
        return false;
    }
    let (line, col) = ctx.buffer.cursor_line_col();
    let text = ctx.buffer.line_text(line);
    match smart_newline_for(&text, col) {
        Some(SmartNewline::Continue(prefix)) => {
            let mut s = String::with_capacity(prefix.len() + 1);
            s.push('\n');
            s.push_str(&prefix);
            ctx.buffer.replace_before_cursor(0, &s);
            true
        }
        Some(SmartNewline::EndBlock { strip }) => {
            // Empty list item / blockquote: drop the dangling marker, leaving the
            // line blank with the caret at column 0 — the list/quote has ended.
            ctx.buffer.replace_before_cursor(strip, "");
            true
        }
        None => false,
    }
}

/// TAB dispatch: on a markdown LIST context (the caret line — or ANY line of an
/// active selection — is a list item), indent one nesting level; ELSEWHERE fall back
/// to the soft-tab insert, byte-identical to before. Keeping the list-vs-plain gate
/// here (over the SHARED [`crate::markdown::list_item`] detection) is what makes Tab
/// `--keys`-drivable and testable without a GPU.
pub(super) fn list_tab(ctx: &mut ActionCtx) {
    if ctx.buffer.is_markdown() && selection_or_cursor_on_list(ctx) {
        ctx.buffer.indent_lines();
    } else {
        ctx.buffer.insert_tab();
    }
}

/// SHIFT-TAB dispatch: outdent one nesting level across the caret line or selection.
/// Uniform (not list-gated): off a list it simply strips up to two leading spaces —
/// a clean no-op when there are none, so it never surprises on plain prose.
pub(super) fn list_outdent(ctx: &mut ActionCtx) {
    ctx.buffer.outdent_lines();
}

/// True when the Tab should INDENT rather than soft-tab: the caret line, or any line
/// an active selection spans, is a markdown [`crate::markdown::list_item`].
fn selection_or_cursor_on_list(ctx: &ActionCtx) -> bool {
    let is_list = |l: usize| crate::markdown::list_item(&ctx.buffer.line_text(l)).is_some();
    match ctx.buffer.selection_line_col() {
        Some(((l0, _), (l1, _))) => (l0.min(l1)..=l0.max(l1)).any(is_list),
        None => is_list(ctx.buffer.cursor_line_col().0),
    }
}

/// The outcome of a markdown smart Enter, computed purely from one line.
pub(super) enum SmartNewline {
    /// Insert a newline then this continuation prefix (indent + the next marker).
    Continue(String),
    /// The current item / quote is EMPTY: strip `strip` chars before the cursor
    /// (the dangling indent + marker) and insert nothing, ending the block.
    EndBlock { strip: usize },
}

/// Decide the markdown smart-Enter behavior for the current `line` text and
/// cursor `col` (chars from the line start). Pure — no buffer / GPU. After any
/// leading indentation it recognizes, in order:
///  * a blockquote (`>`…) — continued with the same `>` run;
///  * an unordered list (`-`/`*`/`+` + space) — continued with the same bullet;
///  * an ordered list (`N.`/`N)` + space) — continued with the number INCREMENTED;
///  * else bare indentation — preserved on a plain Enter.
/// An EMPTY marker line ends the block (`EndBlock`); bare indentation is only ever
/// carried, never ended. Returns `None` when there's nothing to continue (plain
/// prose, or the caret sits inside the marker), so the caller does an ordinary
/// newline.
pub(super) fn smart_newline_for(line: &str, col: usize) -> Option<SmartNewline> {
    let chars: Vec<char> = line.chars().collect();
    // Leading indentation (spaces / tabs) — shared by every branch below.
    let mut i = 0;
    while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
        i += 1;
    }

    // Blockquote: a run of '>' and spaces; continue with the same run.
    if i < chars.len() && chars[i] == '>' {
        let mut j = i;
        while j < chars.len() && (chars[j] == '>' || chars[j] == ' ') {
            j += 1;
        }
        if col < j {
            return None; // caret inside the marker → plain newline
        }
        if chars[j..].iter().all(|c| c.is_whitespace()) {
            return Some(SmartNewline::EndBlock { strip: col });
        }
        return Some(SmartNewline::Continue(chars[..j].iter().collect()));
    }

    // Unordered list: '-' / '*' / '+' then a space.
    if i + 1 < chars.len() && matches!(chars[i], '-' | '*' | '+') && chars[i + 1] == ' ' {
        let prefix_len = i + 2;
        if col < prefix_len {
            return None;
        }
        if chars[prefix_len..].iter().all(|c| c.is_whitespace()) {
            return Some(SmartNewline::EndBlock { strip: col });
        }
        let indent: String = chars[..i].iter().collect();
        return Some(SmartNewline::Continue(format!("{indent}{} ", chars[i])));
    }

    // Ordered list: a run of digits then '.' or ')' then a space.
    let mut d = i;
    while d < chars.len() && chars[d].is_ascii_digit() {
        d += 1;
    }
    if d > i && d + 1 < chars.len() && matches!(chars[d], '.' | ')') && chars[d + 1] == ' ' {
        let prefix_len = d + 2;
        if col < prefix_len {
            return None;
        }
        if chars[prefix_len..].iter().all(|c| c.is_whitespace()) {
            return Some(SmartNewline::EndBlock { strip: col });
        }
        let indent: String = chars[..i].iter().collect();
        let n: usize = chars[i..d].iter().collect::<String>().parse().unwrap_or(0);
        let delim = chars[d];
        // `saturating_add` so a pathological `usize::MAX.` marker can't overflow
        // (panic in debug, wrap-to-0 in release) — it simply pins at usize::MAX.
        return Some(SmartNewline::Continue(format!("{indent}{}{delim} ", n.saturating_add(1))));
    }

    // Bare indentation: carry it forward on a plain Enter (only when the caret is
    // at/after the indentation). No "end on empty" — indentation is just kept.
    if i > 0 && col >= i {
        return Some(SmartNewline::Continue(chars[..i].iter().collect()));
    }

    None
}
