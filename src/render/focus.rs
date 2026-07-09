//! SPAN-ASSEMBLY HELPERS + STATE REPORTS — the per-line `AttrsList` re-lay helpers
//! (`clear_focus_spans` / `color_char_range` / `line_doc_byte_start`) that compose a
//! line's markdown / syntax / CJK / conceal spans, plus the read-only capture-sidecar
//! reports (`md_report` / `wysiwyg_report` / `outline_report` / `syn_report` /
//! `syn_lang_report`).
//!
//! These are inherent methods on [`super::TextPipeline`] — they read its buffer /
//! cursor / metrics state and lay spans on its per-line `AttrsList` through the SAME
//! span seam markdown / syntax / CJK use, so they stay methods rather than free
//! functions. This module is purely a physical home for that cluster, carved out of
//! `render.rs` verbatim.
//!
//! FOCUS MODE was REMOVED (the iA-Writer paragraph/sentence dimming): its driver /
//! fade stepper / settle / sidecar report are gone. The two span-assembly helpers
//! below (`clear_focus_spans` / `color_char_range`) are the load-bearing remainder —
//! they are currently DEAD (their only callers were the removed focus pass) but are
//! kept for pass 2, which re-homes them into `text.rs`'s per-line attrs recipe.

use super::*;

impl TextPipeline {
    /// Reset every buffer line currently carrying an explicit color span back to the
    /// plain document attrs. Retained for pass 2's re-home; DEAD in the live path now
    /// that focus mode is gone (`focus_lines` stays empty), so `#[allow(dead_code)]`.
    #[allow(dead_code)] // pass 2 re-homes this into `text.rs`'s per-line attrs recipe.
    pub(super) fn clear_focus_spans(&mut self) {
        if self.focus_lines.is_empty() {
            return;
        }
        let attrs = self.doc_attrs();
        let fonts = self.resolve_script_fonts();
        let doc_lang = self.doc_lang;
        let cjk_priority = self.cjk_priority.clone();
        // Reset to the PLAIN doc attrs PLUS the per-theme CJK family spans — not a
        // bare `AttrsList::new` — so clearing focus color keeps Japanese in the
        // world's mincho/gothic face (it would otherwise revert to the Latin face).
        let lines = std::mem::take(&mut self.focus_lines);
        // REVEAL-ON-CURSOR: an hr line leaving the active unit re-conceals its `---`
        // unless the caret is on it (mirrors [`build_line_attrs`]).
        let cursor_line = self.cursor_line;
        let cursor_byte = self.line_doc_byte_start(cursor_line);
        for &li in &lines {
            // STALE-INDEX GUARD: `li` was recorded during a PRIOR coloring pass and
            // the buffer may have shrunk since (select-all + type, or a big delete)
            // — that line may no longer exist. Skip it rather than indexing past
            // the end; mirrors the `.get_mut(li)` guard just below, which is why
            // that guard alone wasn't enough (this raw index ran BEFORE it).
            if li >= self.buffer.lines.len() {
                continue;
            }
            let start = self.line_doc_byte_start(li);
            // Preserve a heading line's larger metrics — AND an inline image line's
            // reserved tall row — when it leaves the active unit (else clearing
            // focus would shrink it back to body size). Shared owner with
            // `build_line_attrs` (see [`Self::line_metric_base`]).
            let (lb, row_lh) = self.line_metric_base(li, &attrs);
            if let Some(line) = self.buffer.lines.get_mut(li) {
                let mut al = glyphon::cosmic_text::AttrsList::new(&lb);
                // Re-lay the MARKDOWN base spans (no focus color here — the line is
                // leaving the active unit) so clearing focus keeps its styling, then
                // the SYNTAX base spans, then the CJK family spans on top.
                add_md_line_spans(&mut al, line.text(), start, &lb, &self.md_spans, None);
                add_syn_line_spans(&mut al, line.text(), start, &lb, &self.syn_spans, None);
                add_script_spans(&mut al, line.text(), &lb, doc_lang, &cjk_priority, &fonts);
                if li != cursor_line {
                    add_rule_conceal_span(&mut al, line.text(), start, &lb, &self.md_spans);
                    add_bullet_conceal_span(&mut al, line.text(), &lb);
                }
                add_wysiwyg_conceal_spans(
                    &mut al, line.text(), start, &lb, &self.md_spans, li != cursor_line,
                    cursor_byte, row_lh,
                );
                line.set_attrs_list(al);
            }
        }
        self.buffer.set_redraw(true);
    }

    /// The document BYTE offset of buffer line `li`'s first byte (sum of the
    /// earlier lines' text lengths, each plus one for its `\n`). Used to map the
    /// document-byte markdown spans into a single line's local byte range when
    /// rebuilding that line's `AttrsList`, and by the sidecar reports below. O(li);
    /// the callers touch only a handful of lines, so this stays cheap.
    pub(super) fn line_doc_byte_start(&self, li: usize) -> usize {
        self.buffer
            .lines
            .iter()
            .take(li)
            .map(|l| l.text().len() + 1)
            .sum()
    }

    /// Apply the glyphon `color` as an explicit per-line span over the document char
    /// range `[char_lo, char_hi)`, touching only the buffer lines it intersects and
    /// recording them in `focus_lines`. Char coords are mapped to each line's local
    /// BYTE range (cosmic-text spans are byte-indexed within a `BufferLine`). Retained
    /// for pass 2's re-home; DEAD now that focus mode is gone, so `#[allow(dead_code)]`.
    #[allow(dead_code)] // pass 2 re-homes this into `text.rs`'s per-line attrs recipe.
    pub(super) fn color_char_range(&mut self, char_lo: usize, char_hi: usize, color: glyphon::Color) {
        if char_hi <= char_lo {
            return;
        }
        let attrs = self.doc_attrs();
        let fonts = self.resolve_script_fonts();
        let doc_lang = self.doc_lang;
        let cjk_priority = self.cjk_priority.clone();
        let md_spans = std::mem::take(&mut self.md_spans);
        let syn_spans = std::mem::take(&mut self.syn_spans);
        // REVEAL-ON-CURSOR: keep a recolored hr line's `---` concealed unless the
        // caret is on it (the active unit may include an hr that is not the caret's).
        let cursor_line = self.cursor_line;
        let cursor_byte = self.line_doc_byte_start(cursor_line);
        let mut line_start = 0usize; // absolute char index of this line's first char
        let mut line_byte_start = 0usize; // absolute BYTE index of this line's first byte
        for li in 0..self.buffer.lines.len() {
            let line_chars = self.buffer.lines[li].text().chars().count();
            let line_end = line_start + line_chars; // exclusive of the '\n'
            // Intersect [char_lo, char_hi) with this line's [line_start, line_end).
            let lo = char_lo.max(line_start);
            let hi = char_hi.min(line_end);
            if lo < hi {
                let local_lo = lo - line_start;
                let local_hi = hi - line_start;
                // A HEADING line keeps its larger metrics under focus — AND an inline
                // image line keeps its reserved tall row — so a focused heading/image
                // brightens without shrinking back to body size. Computed BEFORE the
                // `&mut` borrow below; shared owner with `build_line_attrs`
                // (see [`Self::line_metric_base`]).
                let (lb, row_lh) = self.line_metric_base(li, &attrs);
                let line = &mut self.buffer.lines[li];
                let text = line.text();
                let byte_lo = char_to_byte(text, local_lo);
                let byte_hi = char_to_byte(text, local_hi);
                let colored = lb.clone().color(color);
                let mut al = glyphon::cosmic_text::AttrsList::new(&lb);
                // Base MARKDOWN spans across the WHOLE line (the parts OUTSIDE the
                // active-unit colored range keep their normal md styling — dim
                // markup, bold/italic/code/heading content).
                add_md_line_spans(&mut al, text, line_byte_start, &lb, &md_spans, None);
                // Base SYNTAX spans across the whole line (mutually exclusive with
                // md, so only one of these two actually paints anything).
                add_syn_line_spans(&mut al, text, line_byte_start, &lb, &syn_spans, None);
                // Base per-theme CJK family spans across the whole line (the runs
                // OUTSIDE the colored range keep the world's mincho/gothic face).
                add_script_spans(&mut al, text, &lb, doc_lang, &cjk_priority, &fonts);
                // The FOCUS color fills the active range with base+ink (overriding
                // any md/cjk attrs there)...
                al.add_span(byte_lo..byte_hi, &colored);
                // ...then re-apply the MARKDOWN styling WITHIN the focus range with
                // the focus ink as the color override, so the brightened active unit
                // KEEPS its bold/italic/mono/heading weight while taking the full
                // ink (markdown composes under focus without either clobbering).
                add_focus_overlay_spans(
                    &mut al, &md_spans, line_byte_start, text.len(), byte_lo, byte_hi, &lb,
                    color, md_attrs,
                );
                // ...and the same for SYNTAX spans inside the focus range: keep the
                // role styling but take the focus ink (mutually exclusive with md).
                add_focus_overlay_spans(
                    &mut al, &syn_spans, line_byte_start, text.len(), byte_lo, byte_hi, &lb,
                    color, syn_attrs,
                );
                // ...and re-apply the resolved per-script family WITH the color over
                // CJK-family runs that fall inside the colored range, keeping each
                // script in its resolved face while it takes the focus ink.
                for (run, script) in crate::script::script_runs(text) {
                    let id = crate::script::resolve_font_id(doc_lang, Some(script), &cjk_priority);
                    let Some((fam, wt)) = fonts.get(id) else { continue };
                    let colored_script = colored.clone().family(Family::Name(fam)).weight(wt);
                    let r_lo = run.start.max(byte_lo);
                    let r_hi = run.end.min(byte_hi);
                    if r_lo < r_hi {
                        al.add_span(r_lo..r_hi, &colored_script);
                    }
                }
                // Conceal the `---` LAST (transparent ink wins) unless this is the
                // caret's line, so a focused-but-not-edited hr still reads as a fleuron.
                if li != cursor_line {
                    add_rule_conceal_span(&mut al, text, line_byte_start, &lb, &md_spans);
                    add_bullet_conceal_span(&mut al, text, &lb);
                }
                add_wysiwyg_conceal_spans(
                    &mut al, text, line_byte_start, &lb, &md_spans, li != cursor_line,
                    cursor_byte, row_lh,
                );
                line.set_attrs_list(al);
                self.focus_lines.push(li);
            }
            // +1 for the newline separating this line from the next.
            line_start = line_end + 1;
            line_byte_start += self.buffer.lines[li].text().len() + 1;
        }
        self.md_spans = md_spans;
        self.syn_spans = syn_spans;
    }

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
