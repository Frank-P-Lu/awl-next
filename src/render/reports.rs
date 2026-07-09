//! STATE REPORTS — the read-only capture-sidecar reports over the pipeline's
//! shaped state: the markdown styling spans (`md_report`), the WYSIWYG conceal
//! state (`wysiwyg_report`), the persistent margin outline (`outline_current` /
//! `outline_report`), and the syntax highlighting spans + detected language
//! (`syn_report` / `syn_lang_report`).
//!
//! These are inherent methods on [`super::TextPipeline`] — they read its buffer /
//! cursor / span state and are pure functions of the settled frame (no clock), so
//! a capture reports exactly what was rendered. Each shares its ONE deriving rule
//! with the renderer (`wysiwyg_report` rides [`wysiwyg_reveals`] + [`line_has_code_span`];
//! `outline_report` rides [`super::TextPipeline::outline_current`]), so the sidecar
//! can never claim a state the pixels don't match. This module is purely a physical
//! home for that read-only report cluster, carved out of the old `render/focus.rs`
//! (whose focus-mode driver / fade / settle are gone) verbatim.

use super::*;

impl TextPipeline {
    /// MARKDOWN STYLING: the styled spans for the capture sidecar, as
    /// `(start_byte, end_byte, tag)` over the shaped document text. Empty for a
    /// non-markdown buffer. Deterministic (a pure function of the text), so a
    /// capture reports exactly what was rendered — an agent can assert, e.g., that
    /// a heading's `#` falls in a `"markup"` span and its title in `"h1"`.
    pub fn md_report(&self) -> Vec<(usize, usize, String)> {
        self.md_spans
            .iter()
            .map(|(r, k)| {
                // A fenced-code SYNTAX span carries its LANGUAGE too, so the sidecar
                // reports WHICH lexer produced the role — `code_<lang>_<role>` (e.g.
                // `code_rust_comment`, `code_bash_string`). Every other kind keeps its
                // plain static tag, so a non-fence markdown buffer stays byte-identical.
                let tag = match k {
                    crate::markdown::MdKind::CodeSyntax { role, lang } => {
                        format!("code_{}_{}", lang.name(), role.tag())
                    }
                    other => other.tag().to_string(),
                };
                (r.start, r.end, tag)
            })
            .collect()
    }

    /// WYSIWYG: the current CONCEAL state for the capture sidecar — `(on,
    /// concealed)` where `on` mirrors [`crate::markdown::wysiwyg_on`] and
    /// `concealed` lists exactly the `(start_byte, end_byte, kind_tag)` ranges
    /// the renderer is ACTUALLY drawing transparent this settled frame (empty
    /// when `on` is false, or when every concealable span sits revealed under
    /// the caret). Shares the ONE reveal rule ([`wysiwyg_reveals`]) and the ONE
    /// fence body/marker split ([`line_has_code_span`]) with
    /// `add_wysiwyg_conceal_spans` (the renderer), so the sidecar can never claim
    /// something is concealed that isn't actually drawn that way. `md_spans`
    /// itself is UNCHANGED by this round (still tagged `"markup"`/`"code"`); this
    /// is the separate, additive report the WYSIWYG round introduces.
    pub fn wysiwyg_report(&self) -> (bool, Vec<(usize, usize, &'static str)>) {
        let on = crate::markdown::wysiwyg_on();
        let mut out = Vec::new();
        if !on {
            return (on, out);
        }
        use crate::markdown::{ConcealKind, MdKind};
        let cursor_start = self.line_doc_byte_start(self.cursor_line);
        let cursor_end = cursor_start
            + self
                .buffer
                .lines
                .get(self.cursor_line)
                .map(|l| l.text().len())
                .unwrap_or(0);
        // Line byte-offset table, built lazily only if a Fence span is present
        // (the common non-fence case never pays for it).
        let mut line_starts: Option<Vec<usize>> = None;
        for (r, kind) in &self.md_spans {
            let ck = match *kind {
                MdKind::ConcealMarkup(ck) => ck,
                _ => continue,
            };
            // LINE-scoped kinds never cross lines, so "off the caret's line" is
            // exactly "this span's start doesn't fall in the caret line's byte
            // bounds" (irrelevant for `Fence`, which ignores this flag).
            let conceal_off_cursor = !(r.start >= cursor_start && r.start < cursor_end);
            if wysiwyg_reveals(ck, conceal_off_cursor, cursor_start, r) {
                continue;
            }
            if ck != ConcealKind::Fence {
                out.push((r.start, r.end, ck.tag()));
                continue;
            }
            // Fence: emit only the MARKER-line sub-ranges (never the body),
            // mirroring `add_wysiwyg_conceal_spans`'s per-line skip exactly.
            let starts = line_starts.get_or_insert_with(|| {
                let mut v = Vec::with_capacity(self.buffer.lines.len());
                let mut s = 0usize;
                for line in self.buffer.lines.iter() {
                    v.push(s);
                    s += line.text().len() + 1;
                }
                v
            });
            let mut li = match starts.binary_search(&r.start) {
                Ok(i) => i,
                Err(i) => i.saturating_sub(1),
            };
            while li < self.buffer.lines.len() && starts[li] < r.end {
                let ls = starts[li];
                let le = ls + self.buffer.lines[li].text().len();
                if !line_has_code_span(&self.md_spans, ls, le) {
                    let lo = r.start.max(ls);
                    let hi = r.end.min(le);
                    if lo < hi {
                        out.push((lo, hi, ck.tag()));
                    }
                }
                li += 1;
            }
        }
        (on, out)
    }

    /// PERSISTENT MARGIN OUTLINE: the CURRENT heading — the index (into
    /// [`Self::outline_headings`]) of the nearest heading AT or ABOVE the caret's
    /// line, or `None` when the caret sits above the first heading (or there are
    /// no headings). A pure function of `cursor_line` + the stashed heading list,
    /// O(headings). Shared by [`Self::set_view`] (to refresh `last_outline_current`)
    /// and [`Self::outline_report`] (the sidecar) so the render and the sidecar can
    /// never disagree about which section reads as current.
    pub fn outline_current(&self) -> Option<usize> {
        self.outline_headings
            .iter()
            .rposition(|h| h.line <= self.cursor_line)
    }

    /// PERSISTENT MARGIN OUTLINE: the capture sidecar's `outline` block —
    /// `(on, headings, current)` where `on` mirrors [`crate::outline::outline_on`],
    /// `headings` is `(text, level, line)` per heading in document order, and
    /// `current` is [`Self::outline_current`] (the nearest heading at/above the
    /// caret, or `None`). The heading list + current are reported REGARDLESS of
    /// `on` (they are pure text/caret facts the render will consume) — only the
    /// on-screen drawing is gated on `on`, which stays OFF by default so a plain
    /// `--screenshot` reports `on: false` and is byte-identical.
    pub fn outline_report(
        &self,
    ) -> (bool, Vec<(&str, u8, usize)>, Option<usize>) {
        let on = crate::outline::outline_on();
        let headings = self
            .outline_headings
            .iter()
            .map(|h| (h.text.as_str(), h.level, h.line))
            .collect();
        (on, headings, self.outline_current())
    }

    /// SYNTAX HIGHLIGHTING: the styled spans for the capture sidecar, as
    /// `(start_byte, end_byte, tag)` over the shaped document text (tag is one of
    /// `comment`/`string`/`constant`/`definition`). Empty for a non-code buffer.
    /// Deterministic (a pure function of the text + language), so a capture reports
    /// exactly what was rendered — an agent can assert, e.g., that a `// foo`
    /// comment falls in a `"comment"` span.
    pub fn syn_report(&self) -> Vec<(usize, usize, &'static str)> {
        self.syn_spans
            .iter()
            .map(|(r, k)| (r.start, r.end, k.tag()))
            .collect()
    }

    /// SYNTAX HIGHLIGHTING: the DETECTED code language's stable name for the capture
    /// sidecar's `syn_lang` field (e.g. `"rust"`), or `None` for a non-code buffer.
    /// It tracks the same [`crate::buffer::Buffer::syntax_lang`] gate that decides
    /// whether `syn_spans` are emitted, so the sidecar's language and spans always
    /// agree. Mirrors [`Self::syn_report`].
    pub fn syn_lang_report(&self) -> Option<&'static str> {
        self.syn_lang.map(|l| l.name())
    }
}
