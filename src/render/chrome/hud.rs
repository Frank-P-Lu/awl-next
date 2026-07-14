//! HELD STATS HUD + ABOUT CARD chrome — the two mutually-exclusive summoned
//! float-cards sharing one pipeline: the %-through-doc figure, the
//! machine-readable [`HudReport`], and the shared shape/upload of the stats HUD
//! (held) or the About card (open). Carved out of `chrome.rs` verbatim, no
//! behaviour change. See [`super`].

use super::*;

impl TextPipeline {
    // ===== HELD STATS HUD =================================================

    /// Push the LIFETIME-ODOMETER snapshot the held HUD's odometer rows render
    /// (characters, writing time, files touched, caret travel, most-lived-in
    /// world). The live App calls this every `sync_view` from its persisted
    /// [`crate::stats::Stats`] store (`App::stats_sync_hud`); the headless capture
    /// never calls it, so the field stays `None` and every odometer row shows the
    /// fixed placeholder — the determinism boundary keeping a `--hud` capture
    /// byte-stable (mirrors the retired `set_hud_session`).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_hud_stats(&mut self, stats: Option<crate::hud::HudStats>) {
        self.hud_stats = stats;
    }

    /// NOTES VERBS round: push the SAVED stat's live state (dirty, or clean +
    /// elapsed seconds since the last successful write — manual save OR
    /// autosave, whichever landed most recently). The live App calls this every
    /// `sync_view` (`App::sync_hud_saved`); the headless capture never calls it,
    /// so the row renders the fixed placeholder — mirrors [`Self::set_hud_stats`]
    /// exactly.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_hud_saved(&mut self, state: Option<crate::hud::HudSaved>) {
        self.hud_saved = state;
    }

    /// CHECK FOR UPDATES round: push the About card's "checked … ago" figure
    /// (the LOCAL "last checked" marker's `Never`/`CheckedAgo(secs)` state).
    /// The live App calls this every `sync_view` (`App::sync_update_checked`);
    /// the headless capture never calls it, so the line renders the fixed dash
    /// placeholder — mirrors [`Self::set_hud_saved`] exactly.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_update_checked(&mut self, state: Option<crate::updates::UpdateChecked>) {
        self.hud_update_checked = state;
    }

    /// Read back the pushed "last checked" state — the sidecar's own accessor
    /// (`capture/sidecar.rs::about_json`), so the pixels and the JSON read the
    /// SAME field, never a second copy.
    pub fn hud_update_checked(&self) -> Option<crate::updates::UpdateChecked> {
        self.hud_update_checked
    }

    /// Push/read the passive pending-crash state. The live App owns the marker;
    /// headless pipelines default false unless a capture law explicitly injects it.
    pub fn set_pending_crash(&mut self, pending: bool) {
        self.hud_pending_crash = pending;
    }

    pub fn hud_pending_crash(&self) -> bool {
        self.hud_pending_crash
    }

    /// Push the HOLD-⌘ SHORTCUT PEEK's personalized rows (the live ledger's graduation
    /// candidates). The live App calls this every `sync_view` (`App::sync_discoverability`);
    /// the headless capture never does, so the field stays empty and the peek card renders
    /// the curated starter six via [`crate::peek::rows_or_starter`] — the determinism
    /// boundary keeping a `--peek` capture byte-stable.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_peek_rows(&mut self, rows: Vec<crate::peek::PeekRow>) {
        self.peek_rows = rows;
    }

    /// The rows the peek card / sidecar actually render THIS frame: the pushed
    /// personalized rows, or the curated starter six when empty (fresh-install ledger OR
    /// a capture). ONE owner shared by the pixels + the sidecar (`peek_report`).
    pub(in crate::render) fn peek_effective_rows(&self) -> Vec<crate::peek::PeekRow> {
        crate::peek::rows_or_starter(&self.peek_rows)
    }

    /// Push the KEYBINDINGS TIPS FOOTER lines ("your top 3"). The live App calls this
    /// every `sync_view` — the top-3 tip one-liners while the Keybindings overlay is open,
    /// empty otherwise; a headless capture never does, so the footer is hidden and a
    /// Keybindings capture is byte-identical.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_keybindings_tips(&mut self, tips: Vec<String>) {
        self.keybindings_tips = tips;
    }

    /// The cursor's position as a whole-PERCENT through the document (0..=100), by
    /// CHAR offset over the total char count (newlines included). Deterministic — a
    /// pure function of the buffer + cursor — so it is shown in a capture. An empty
    /// document reads 0%.
    fn hud_percent(&self) -> u32 {
        let lines = &self.buffer.lines;
        let total_chars: usize = lines.iter().map(|l| l.text().chars().count()).sum();
        let denom = total_chars + lines.len().saturating_sub(1); // + inter-line newlines
        if denom == 0 {
            return 0;
        }
        let mut offset = 0usize;
        for l in lines.iter().take(self.cursor_line) {
            offset += l.text().chars().count() + 1; // + the line's trailing newline
        }
        offset += self.cursor_col;
        (((offset.min(denom) as f32) / denom as f32) * 100.0).round() as u32
    }

    /// The HUD's machine-readable state for the capture sidecar: which WRITER figures it
    /// shows, exactly as the rendered panel does, so the sidecar always agrees with the
    /// pixels. `words` is `None` for a non-markdown buffer (the word-count stat is
    /// omitted there); `percent` is the cursor's %-through-doc. Both are pure functions
    /// of the doc + cursor — no clock/filesystem field remains.
    pub fn hud_report(&self) -> HudReport {
        HudReport {
            held: crate::hud::hud_held(),
            words: self.readout_report(),
            percent: self.hud_percent(),
            lang: self.doc_lang_report(),
            eol: self.eol,
            saved: crate::hud::saved_readout(self.hud_saved),
        }
    }

    /// The summoned LIFETIME STATS card's machine-readable figures for the sidecar
    /// (see [`LifetimeReport`]). The personal ODOMETER split out of the held HUD:
    /// each row is folded to a placeholder in a capture (no live store) by the SAME
    /// [`crate::hud::odometer_rows`] owner the pixels use, so the sidecar can never
    /// claim a figure the card doesn't show. `open` mirrors the process-global.
    pub fn lifetime_report(&self) -> LifetimeReport {
        let [chars, writing, files, caret_travel, world] =
            crate::hud::odometer_rows(self.hud_stats.as_ref()).map(|(_, v)| v);
        LifetimeReport {
            open: crate::lifetime::lifetime_open(),
            chars,
            writing,
            files,
            caret_travel,
            world,
        }
    }

    /// The HOLD-⌘ SHORTCUT PEEK's machine-readable state for the sidecar (see
    /// [`PeekReport`]). `open` mirrors the process-global; `rows` is exactly what the
    /// card renders — the pushed personalized rows, or the curated starter six when
    /// empty (a capture never pushed) — via the same [`Self::peek_effective_rows`] owner
    /// the pixels use, so the two can never disagree.
    pub fn peek_report(&self) -> PeekReport {
        PeekReport {
            open: crate::peek::peek_open(),
            rows: self.peek_effective_rows(),
        }
    }

    /// Shape + upload the held STATS HUD, the summoned ABOUT card, **or** the
    /// summoned LIFETIME STATS card — all three share this ONE float-card pipeline
    /// (`hud_shadow`/`hud_border`/`hud_card`/`hud_buffer`/`hud_renderer`) rather than
    /// each owning a parallel set of wgpu resources, since they are mutually
    /// exclusive summoned states with the same
    /// visual shape (a centered/left-spined `base_300` card over the frosted-blur
    /// backdrop). HUD: a LEFT-ALIGNED readout — each stat a quiet CAPTION in FAINT
    /// ink at LABEL size over its VALUE in CONTENT ink at BODY size (the type
    /// system, ink × size) — NO amber anywhere (amber is the caret's alone).
    /// TRIMMED to the WRITER stats: WORD COUNT + reading time, %-THROUGH-DOC, and
    /// LINE ENDINGS (all PURE functions of the buffer — no clock/fs field remains).
    /// About: "Awl" / the crate version / the active world's name, closed with
    /// that world's own dash fleuron as an end-mark ornament (`about.rs`). Drawn
    /// ONLY while held (`crate::hud::hud_held`) or open (`crate::about::about_open`);
    /// otherwise the text is parked off-screen, so a default capture stays
    /// byte-identical. Every figure is a PURE function of the doc/cursor/active
    /// world, so a `--hud` / About-open capture is deterministic.
    pub(in crate::render) fn prepare_hud(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        // `hud_showing()` folds in the overlay-exclusion gate (an open summoned
        // overlay wins; the raw global stays set so the sidecar's `hud.held`
        // still reflects the held key). About is opened by a palette command that
        // closes the overlay, so it's already exclusive — leave it on the raw flag.
        let held = self.hud_showing();
        let about = crate::about::about_open();
        let lifetime = crate::lifetime::lifetime_open();
        let peek = self.peek_showing();
        let showing = held || about || lifetime || peek;
        // No scrim: while shown, the document recedes behind the shared FROSTED-BLUR
        // backdrop (the `render` blur branch), so the card draws only itself + its
        // content. The card rect (shadow -> raised border -> card) is uploaded once the
        // block extent is measured (shown branch); hidden, park all three so nothing draws.
        if !showing {
            set_float_quads(
                &mut self.hud_shadow,
                &mut self.hud_border,
                &mut self.hud_card,
                device,
                queue,
                width,
                height,
                None,
                true,
            );
        }

        let m = self.metrics;
        let bounds = TextBounds { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        let content = theme::base_content().to_glyphon();
        let faint = theme::faint().to_glyphon();

        // HIDDEN: park an empty buffer off-screen (nothing drawn), matching the
        // corner-readout convention so a non-shown capture is byte-identical.
        if !showing {
            self.hud_buffer
                .set_size(&mut self.font_system, Some(1.0), Some(m.line_height));
            self.hud_buffer.set_text(
                &mut self.font_system,
                "",
                &panel_attrs().color(content),
                Shaping::Advanced,
                None,
            );
            self.hud_buffer
                .shape_until_scroll(&mut self.font_system, false);
            let area = TextArea {
                buffer: &self.hud_buffer,
                left: 0.0,
                top: -1000.0,
                scale: 1.0,
                bounds,
                default_color: content,
                custom_glyphs: &[],
            };
            self.hud_renderer
                .prepare(
                    device,
                    queue,
                    &mut self.font_system,
                    &mut self.atlas,
                    &self.viewport,
                    [area],
                    &mut self.swash_cache,
                )
                .map_err(|e| anyhow::anyhow!("glyphon hud prepare failed: {e:?}"))?;
            return Ok(());
        }

        // Line role, shared by both cards below: 0 = caption (faint/LABEL), 1 =
        // value (content/BODY), 2 = TITLE (content/SECTION — About's "Awl" only).
        let label = crate::markdown::type_scale::LABEL;
        let section = crate::markdown::type_scale::SECTION;
        let body_metrics = GlyphMetrics::new(m.font_size, m.line_height);
        let label_metrics = GlyphMetrics::new(m.font_size * label, m.line_height * label);
        let title_metrics = GlyphMetrics::new(m.font_size * section, m.line_height * section);

        let mut owned: Vec<(String, u8)> = Vec::new();
        if about {
            // ABOUT CARD: "Awl" (title) / the crate version / the active world's
            // own name (faint caption) / that world's dash fleuron as a closing
            // end-mark ornament — see `about.rs`'s module doc. Every figure is a
            // pure function of a `const` + the active theme, so this is
            // deterministic and `--keys`-capture-safe.
            let world = theme::active();
            owned.push(("Awl\n\n".to_string(), 2));
            owned.push((format!("v{}\n", env!("CARGO_PKG_VERSION")), 1));
            // Author + license, calm faint captions under the version.
            owned.push(("by Frank Lu · GPL-3.0\n\n".to_string(), 0));
            owned.push((format!("{}\n", world.name), 0));
            // CHECK FOR UPDATES round: a quiet "checked … ago" caption from the
            // LOCAL "last checked" marker — `None` (a headless capture, which
            // never calls `sync_update_checked`) renders the fixed dash
            // placeholder; `Some(Never)` (live, no marker written yet) OMITS
            // the line entirely (`updates::checked_line`, the ONE owner shared
            // with the sidecar). Never a clock/fs read here — the figure is
            // whatever the live App already pushed this frame.
            if let Some(line) = crate::updates::checked_line(self.hud_update_checked) {
                owned.push((format!("{line}\n"), 0));
            }
            if self.hud_pending_crash {
                owned.push(("previous crash log available · Settings → Report a Problem\n".to_string(), 0));
            }
            // Quiet pointer to the in-app Credits door (⌘P → Credits opens the
            // embedded CREDITS.md as a buffer) — TASTE-FLAGGED wording, see
            // CLAUDE.md's LICENSE + CREDITS round: matches the card's existing
            // faint-caption discipline (ink ladder only, no new accent).
            owned.push(("⌘P → Credits\n\n".to_string(), 0));
            // Role 3 = the closing ORNAMENT: the world's own dash fleuron, shaped in
            // that world's assigned ornament face (`Theme::ornament_face`) — the same
            // face a `---` section break renders in — not the panel's display face.
            owned.push((world.ornaments.dash.to_string(), 3));
        } else if lifetime {
            // LIFETIME STATS CARD: the personal ODOMETER split out of the held HUD —
            // characters, writing time, files touched, caret travel, most-lived-in
            // world. Each row a CAPTION over its VALUE, same LEFT-ALIGNED spine as the
            // held HUD below. LIVE-ONLY: a real reading only from the live App's
            // persisted store; a capture (no store) folds every row to the fixed "—"
            // placeholder via the SAME `odometer_rows` owner the sidecar uses, so a
            // `--lifetime` capture is deterministic and the two can never disagree.
            let rows = crate::hud::odometer_rows(self.hud_stats.as_ref());
            let last = rows.len().saturating_sub(1);
            for (i, (caption, value)) in rows.into_iter().enumerate() {
                owned.push((format!("{caption}\n"), 0)); // caption (label / faint)
                let val_line = if i == last { value } else { format!("{value}\n\n") };
                owned.push((val_line, 1));
            }
        } else if peek {
            // HOLD-⌘ SHORTCUT PEEK: a calm column of shortcuts — the live ledger's
            // graduation candidates (the commands the user reaches via a slow door but
            // has a chord for), or the curated STARTER SIX on a fresh install / in a
            // capture (`peek_effective_rows` folds the empty push to the starter six via
            // the ONE `crate::peek::rows_or_starter` owner the sidecar shares). Each row
            // is the CHORD as the FIGURE (content ink, BODY size) over its command NAME
            // as the CAPTION (faint ink, LABEL size) — the type system's ink × size, NO
            // amber (the caret's alone). The inverse of the HUD's caption-over-value: a
            // shortcut's chord is what you're here to learn, so it leads.
            let rows = self.peek_effective_rows();
            let last = rows.len().saturating_sub(1);
            for (i, row) in rows.into_iter().enumerate() {
                owned.push((format!("{}\n", row.chord), 1)); // chord figure (content / body)
                let name_line =
                    if i == last { row.name } else { format!("{}\n\n", row.name) };
                owned.push((name_line, 0)); // name caption (faint / label)
            }
        } else {
            // The stats, top to bottom: each a quiet CAPTION over its VALUE. TRIMMED to
            // the WRITER figures — WORD COUNT + reading time and %-THROUGH-DOC — both
            // PURE functions of the doc (no clock/filesystem field), so the capture is
            // deterministic. WORD COUNT is markdown-only (omitted for code/plain
            // buffers). EVERY value rides CONTENT ink — NO amber anywhere (the
            // THROUGH-DOC % used to be amber, a DESIGN §3 stretch since `primary` is
            // the caret's alone; it is now plain content ink).
            let mut stats: Vec<(&'static str, String)> = Vec::with_capacity(4);
            // NOTES VERBS round: SAVED — since the last successful write (manual
            // save OR autosave, whichever landed most recently), or "unsaved
            // changes" while dirty right now. LIVE-ONLY (a real clock read): the
            // headless capture never pushes `hud_saved`, so this folds to the
            // fixed placeholder via the SAME `saved_readout` owner `hud_report`
            // uses, keeping the pixels and the sidecar in lockstep.
            stats.push(("SAVED", crate::hud::saved_readout(self.hud_saved)));
            // WORD COUNT + reading time — markdown buffers only (omitted otherwise).
            // Reuses the same `wordcount_text` feeder the bottom-right readout used
            // pre-phase-2.
            let words = self.wordcount_text();
            if !words.is_empty() {
                stats.push(("WORD COUNT", words));
            }
            // i18n: the document's OWN frontmatter `lang:` tag — omitted for an
            // untagged (or non-markdown) document, mirroring WORD COUNT's own
            // omit-when-absent shape. A pure function of the currently-shaped
            // text, so this is deterministic and capture-safe.
            if let Some(lang) = self.doc_lang_report() {
                stats.push(("LANGUAGE", lang.code().to_string()));
            }
            stats.push(("THROUGH DOC", format!("{}%", self.hud_percent())));
            // LINE ENDINGS: the active buffer's on-disk ending ("LF"/"CRLF") — a
            // PURE buffer fact (deterministic, capture-safe), so unlike the dropped
            // clock/fs rows it is always shown with its real value, never a "—".
            stats.push(("LINE ENDINGS", self.eol.label().to_string()));
            // NOTE: the LIFETIME ODOMETER (characters, writing time, files touched,
            // caret travel, your world) is NO LONGER shown here — it moved to its own
            // summoned "Lifetime stats" card (`lifetime.rs`), so the held HUD stays a
            // pure per-doc peek with no placeholder rows at all.

            // LEFT-ALIGNED on a spine: each stat is a CAPTION line (faint ink, LABEL
            // size) directly over its VALUE line (content ink, BODY size — NO amber:
            // the % is plain content ink like the rest, since amber is the caret's
            // alone), in a tight vertical rhythm with a single blank LABEL line
            // between groups (dropped after the last).
            let last = stats.len().saturating_sub(1);
            for (i, (caption, value)) in stats.into_iter().enumerate() {
                owned.push((format!("{caption}\n"), 0)); // caption (label / faint)
                let val_line = if i == last {
                    value
                } else {
                    format!("{value}\n\n") // value + a blank gap before the next group
                };
                owned.push((val_line, 1));
            }
        }
        let base = panel_attrs();
        let spans: Vec<(&str, Attrs)> = owned
            .iter()
            .map(|(s, role)| {
                let attrs = match role {
                    0 => base.clone().color(faint).metrics(label_metrics),
                    2 => base.clone().color(content).metrics(title_metrics),
                    // The About end-mark ornament: override to the world's ornament
                    // face at NORMAL weight (the ornament faces are Regular/400, and a
                    // stale display weight — e.g. IBM Plex Mono's 300 — would trip the
                    // weight_diff fallback filter and drop the face).
                    3 => base
                        .clone()
                        .color(content)
                        .metrics(body_metrics)
                        .family(Family::Name(theme::active().ornament_face))
                        .weight(glyphon::Weight::NORMAL),
                    _ => base.clone().color(content).metrics(body_metrics),
                };
                (s.as_str(), attrs)
            })
            .collect();
        // No alignment (cosmic-text defaults to LEFT): each line starts at the buffer's
        // left edge, and the TextArea `left` (below) plants that spine inside the card.
        // Generous buffer width so the value lines never wrap.
        self.hud_buffer
            .set_size(&mut self.font_system, Some(width as f32), Some(height as f32));
        let default_attrs = base.clone().color(content).metrics(body_metrics);
        self.hud_buffer.set_rich_text(
            &mut self.font_system,
            spans,
            &default_attrs,
            Shaping::Advanced,
            None,
        );
        self.hud_buffer
            .shape_until_scroll(&mut self.font_system, false);
        // Vertically center the stacked block: measure the shaped run extent (height
        // AND max line width) and offset so the column sits in the middle of the canvas.
        let mut block_h = 0.0_f32;
        let mut block_w = 0.0_f32;
        for run in self.hud_buffer.layout_runs() {
            block_h = block_h.max(run.line_top + run.line_height);
            block_w = block_w.max(run.line_w);
        }
        let top = ((height as f32 - block_h) * 0.5).max(TEXT_TOP);
        // The calm card behind the stats: the block + generous padding, centered, risen
        // a value step over the dimmed doc so the figures read on a clean ground — on the
        // same float-panel elevation (shadow -> raised border -> card) as which-key.
        let pad_x = m.char_width * 3.0;
        let pad_y = m.line_height * 0.9;
        let card_w = block_w + pad_x * 2.0;
        let card_h = block_h + pad_y * 2.0;
        let card_x = (width as f32 - card_w) * 0.5;
        let card_y = top - pad_y;
        set_float_quads(
            &mut self.hud_shadow,
            &mut self.hud_border,
            &mut self.hud_card,
            device,
            queue,
            width,
            height,
            Some([card_x, card_y, card_w, card_h]),
            true,
        );
        let area = TextArea {
            buffer: &self.hud_buffer,
            left: card_x + pad_x,
            top,
            scale: 1.0,
            bounds,
            default_color: content,
            custom_glyphs: &[],
        };
        self.hud_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon hud prepare failed: {e:?}"))?;
        Ok(())
    }
}
