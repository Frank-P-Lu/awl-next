//! FOCUS COLORING + STATE REPORTS — the typewriter/paragraph focus pass that tints
//! the active unit to full ink while the rest recedes (`update_focus` /
//! `refresh_focus_spans` / `clear_focus_spans` / `color_char_range`), its
//! deterministic settle + per-frame fade step, plus the read-only capture-sidecar
//! reports (`focus_report` / `md_report` / `syn_report` / `syn_lang_report`).
//!
//! These are inherent methods on [`super::TextPipeline`] — they overlay focus color
//! spans on its per-line `AttrsList` through the SAME span seam markdown / syntax /
//! CJK use, reading its buffer / cursor / metrics state, so they stay methods rather
//! than free functions. This module is purely a physical home for that cohesive focus
//! cluster, carved out of `render.rs` verbatim; a child module sees its ancestor's
//! private items, so access to `TextPipeline`'s fields/helpers is unchanged and the
//! focus pixels are byte-identical.

use super::*;

impl TextPipeline {
    /// FOCUS MODE driver: recompute the active unit around the cursor for the
    /// current [`crate::focus::mode`], kick the brighten/dim crossfade when the cursor
    /// JUMPS to a different unit (LIVE; the first application snaps), and (re)apply the
    /// per-line color spans. `reshaped` forces a reapply because a document reshape
    /// drops spans.
    ///
    /// `is_edit` distinguishes a text edit from a pure cursor move. It matters because
    /// the active unit's char RANGE shifts whenever the unit grows/shrinks under the
    /// caret — typing one char in the active paragraph bumps its end index by one — so
    /// a raw range compare would read "same unit, one char longer" as "entered a new
    /// unit" and re-kick the crossfade on EVERY keystroke (a visible per-keystroke
    /// flash). The fade is therefore kicked only for a NON-edit move that lands in a
    /// unit DISJOINT from the prior one; an in-unit edit (or any overlapping range)
    /// just snaps `focus_cur` to the new bounds at full ink, leaving the fade settled.
    ///
    /// Off is the cheap path: any prior focus coloring is cleared once and the whole
    /// document rides the full-ink default again. The dim of the non-active text is
    /// applied for FREE via the `default_color` chosen in [`Self::prepare`]; only the
    /// (small) active unit carries an explicit full-ink span here.
    pub(super) fn update_focus(&mut self, text: &str, reshaped: bool, is_edit: bool) {
        // REVEAL-ON-CURSOR: keep every hr line's `---` conceal/reveal in step with the
        // caret line on EVERY set_view (a pure cursor move re-lays no text otherwise).
        // Runs regardless of focus mode; idempotent when no hr boundary was crossed.
        // `reshaped` forces the rescan (a text edit / restyle dropped the per-line
        // attrs); an ordinary same-line move / scroll is gated out inside.
        self.refresh_rule_conceal(reshaped);
        let mode = crate::focus::mode();
        if mode == crate::focus::FocusMode::Off {
            // Leaving focus mode (or never in it): drop any spans ONCE, then idle.
            // Only re-shape if we actually cleared a colored line — so an ordinary
            // cursor move with focus off stays free (no reshape).
            if !self.focus_lines.is_empty() {
                self.clear_focus_spans();
                // The cleared lines' shaping was reset; re-shape so they lay out at
                // full ink again.
                self.buffer.shape_until_scroll(&mut self.font_system, false);
            }
            self.focus_cur = None;
            self.focus_prev = None;
            self.focus_t = 1.0;
            self.focus_initialized = false;
            self.focus_sig = None;
            return;
        }
        let cur_char = line_col_to_char_index(text, self.cursor_line, self.cursor_col);
        let desired = crate::focus::active_range(text, cur_char, mode);
        if desired != self.focus_cur {
            // The active unit's range changed. Two cases the raw compare conflates:
            //   * the cursor JUMPED to a different unit — the new range is DISJOINT
            //     from the old, so kick the live crossfade (old dims, new brightens);
            //   * the SAME unit merely grew/shrank under the caret (an edit, or any
            //     range that still OVERLAPS the old since the caret never left it) —
            //     adopt the new bounds SILENTLY so typing doesn't re-run the fade
            //     every keystroke. The very first application always snaps (settled).
            let jumped = !is_edit
                && match (self.focus_cur, desired) {
                    // [a0,a1) and [b0,b1) are disjoint iff one ends at-or-before the
                    // other begins.
                    (Some((a0, a1)), Some((b0, b1))) => a0 >= b1 || b0 >= a1,
                    _ => true,
                };
            if self.focus_initialized && jumped {
                self.focus_prev = self.focus_cur;
                self.focus_t = 0.0;
            } else {
                self.focus_prev = None;
                self.focus_t = 1.0;
            }
            self.focus_cur = desired;
            self.focus_initialized = true;
        }
        self.refresh_focus_spans(reshaped);
    }

    /// Reset every buffer line currently carrying a focus color span back to the
    /// plain document attrs (so it rides the `default_color`). Used when focus turns
    /// Off and as the first step of a coloring refresh.
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
        let base_fs = self.metrics.font_size;
        let base_lh = self.metrics.line_height;
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
            // Preserve a heading line's larger metrics when it leaves the active
            // unit (else clearing focus would shrink it back to body size).
            let scale = md_line_scale(self.buffer.lines[li].text(), self.md_enabled);
            let lb = scaled_base_attrs(&attrs, base_fs, base_lh, scale);
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
                    cursor_byte, base_lh * scale,
                );
                line.set_attrs_list(al);
            }
        }
        self.buffer.set_redraw(true);
    }

    /// (Re)write the per-line focus color spans for the current `focus_cur` (full,
    /// fading IN) and `focus_prev` (fading OUT) ranges. Guarded by a signature so a
    /// settled, unchanged frame skips the work (no reshape on idle). `force` (a text
    /// reshape just happened) bypasses the guard since the spans were just dropped.
    pub(super) fn refresh_focus_spans(&mut self, force: bool) {
        let mode = crate::focus::mode();
        // Bucket the fade progress so tiny float jitter doesn't thrash the signature,
        // but every visible step during a fade still triggers a recolor.
        let bucket = (self.focus_t.clamp(0.0, 1.0) * 256.0) as u32;
        let sig = (
            match mode {
                crate::focus::FocusMode::Off => 0u8,
                crate::focus::FocusMode::Paragraph => 1,
                crate::focus::FocusMode::Sentence => 2,
            },
            self.focus_cur,
            self.focus_prev,
            bucket,
        );
        if !force && self.focus_sig == Some(sig) {
            return;
        }
        // Clear last frame's colored lines, then paint this frame's ranges.
        self.clear_focus_spans();
        let full = theme::base_content();
        let dim = crate::focus::dim_srgb();
        // The just-entered unit brightens dim -> full; the just-left unit dims
        // full -> dim. A smoothstep ease keeps the crossfade calm.
        let t = smoothstep(self.focus_t.clamp(0.0, 1.0));
        if let Some((s, e)) = self.focus_cur {
            let c = lerp_srgb(dim, full, t).to_glyphon();
            self.color_char_range(s, e, c);
        }
        if let Some((s, e)) = self.focus_prev {
            let c = lerp_srgb(dim, full, 1.0 - t).to_glyphon();
            self.color_char_range(s, e, c);
        }
        self.focus_sig = Some(sig);
        // The colored / cleared lines had their per-line shaping reset by
        // `set_attrs_list`; re-shape so they lay out with the new attrs before the
        // next `prepare`. Lines whose attrs did not actually change no-op'd the reset
        // and stay cached, so this only re-shapes the (few) active-unit lines.
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        self.buffer.set_redraw(true);
    }

    /// The document BYTE offset of buffer line `li`'s first byte (sum of the
    /// earlier lines' text lengths, each plus one for its `\n`). Used to map the
    /// document-byte markdown spans into a single line's local byte range when
    /// rebuilding that line's `AttrsList` (focus clear / recolor). O(li); the focus
    /// paths touch only a handful of lines, so this stays cheap.
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
    /// BYTE range (cosmic-text spans are byte-indexed within a `BufferLine`).
    pub(super) fn color_char_range(&mut self, char_lo: usize, char_hi: usize, color: glyphon::Color) {
        if char_hi <= char_lo {
            return;
        }
        let attrs = self.doc_attrs();
        let fonts = self.resolve_script_fonts();
        let doc_lang = self.doc_lang;
        let cjk_priority = self.cjk_priority.clone();
        let base_fs = self.metrics.font_size;
        let base_lh = self.metrics.line_height;
        let md = self.md_enabled;
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
                let line = &mut self.buffer.lines[li];
                let text = line.text();
                let byte_lo = char_to_byte(text, local_lo);
                let byte_hi = char_to_byte(text, local_hi);
                // A HEADING line keeps its larger metrics under focus: derive the
                // line's scaled base, and the colored fill from THAT, so a focused
                // heading brightens without shrinking back to body size.
                let scale = md_line_scale(text, md);
                let lb = scaled_base_attrs(&attrs, base_fs, base_lh, scale);
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
                    cursor_byte, base_lh * scale,
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

    /// FOCUS MODE: place the dim/full coloring at its SETTLED state (active unit at
    /// full ink, the rest dim) with NO clock consulted — the deterministic capture
    /// pose, mirroring [`Self::settle_caret`]. Live animation never calls this.
    pub fn settle_focus(&mut self) {
        self.focus_prev = None;
        self.focus_t = 1.0;
        self.refresh_focus_spans(true);
    }

    /// FOCUS MODE: the active range + mode for the sidecar, as char offsets over the
    /// document text. `(mode_name, active_start, active_end)`; the range is `None`
    /// when focus is Off.
    pub fn focus_report(&self) -> (&'static str, Option<(usize, usize)>) {
        (crate::focus::mode().name(), self.focus_cur)
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

    /// Advance the FOCUS-MODE brighten/dim crossfade by `dt` seconds, recolor the
    /// affected lines, and report whether the fade is still in flight (so the live
    /// loop stays hot until it lands, then idles). A no-op when focus is Off or the
    /// fade has already settled — so it never adds a permanent busy loop.
    pub(super) fn step_focus(&mut self, dt: f32) -> bool {
        if crate::focus::mode() == crate::focus::FocusMode::Off || self.focus_t >= 1.0 {
            return false;
        }
        self.focus_t = (self.focus_t + dt / crate::focus::FOCUS_FADE_SECS).min(1.0);
        if self.focus_t >= 1.0 {
            // Settled: the just-left unit is fully dim now; stop recoloring it.
            self.focus_prev = None;
        }
        self.refresh_focus_spans(false);
        self.focus_t < 1.0
    }
}
