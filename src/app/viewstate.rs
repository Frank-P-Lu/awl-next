//! VIEW SNAPSHOT: build the [`ViewState`] from the current buffer + scroll +
//! zoom + selection + search + overlay and push it into the pipeline
//! ([`App::sync_view`]), plus anchoring the OS IME candidate window to the
//! caret. The one bridge from editor state to the render pipeline; lifted out
//! of `app.rs` verbatim.

use super::*;

impl App {
    /// Build the render snapshot from the current buffer + scroll + zoom +
    /// selection and push it into the pipeline. When `follow` is true (cursor
    /// moved / text edited), the scroll is clamped so the cursor stays on
    /// screen; when false (free wheel scroll), the scroll is left untouched so
    /// the viewport moves independently of the cursor.
    pub(super) fn sync_view(&mut self, follow: bool) {
        if self.gpu.is_none() {
            return;
        }
        let height = self.gpu.as_ref().unwrap().config.height as f32;
        let (cursor_line, cursor_col) = self.buffer.cursor_line_col();
        // Re-run spell detection only when the buffer text changed. We detect a
        // change via the cheap edit VERSION (a `u64` bump per content mutation)
        // instead of cloning + comparing the whole rope string each keystroke. The
        // preedit composition is deliberately NOT included, so composing text is
        // never flagged. Debounced: if the version changed, just mark it dirty and
        // keep showing the previous squiggles; the re-scan runs in about_to_wait
        // after ~150ms of quiet so a word isn't flagged while you're still typing.
        if self.spell.is_some() && self.spell_checked_version != Some(self.buffer.version()) {
            self.spell_dirty_at = Some(Instant::now());
        }
        // Schedule a debounced AUTO-SAVE for the active quick note when its text
        // changed. This lives ONLY here (the live windowed path, gated by the
        // gpu-present check above), so the headless capture/replay never auto-writes
        // — the determinism + no-fixture-mutation guarantee. The write fires in
        // `about_to_wait` after a quiet period.
        if self.buffer.is_note() && self.autosave_saved_version != Some(self.buffer.version()) {
            self.autosave_dirty_at = Some(Instant::now());
        }
        // ROPE-CLONE SHORT-CIRCUIT: `sync_view` runs on every cursor move / scroll /
        // selection change, none of which bump the buffer version — yet each would
        // otherwise walk the whole rope into a fresh `String`. Reuse the last clone
        // (a memcpy) while the version is unchanged; re-materialise the rope only after
        // a real edit. The resulting `text` bytes are identical either way.
        let text_version = self.buffer.version();
        let text = match &self.sync_text_cache {
            Some((v, t)) if *v == text_version => t.clone(),
            _ => {
                let t = self.buffer.text();
                self.sync_text_cache = Some((text_version, t.clone()));
                t
            }
        };

        // Did this sync follow a text EDIT? A bumped buffer version since the last
        // sync means the cursor moved because of typing/delete/paste/newline (vs.
        // pure navigation), so the caret slides as a plain block with no underline
        // however far it jumped (Enter, a wide glyph, a paste). Captured once per
        // sync so the re-push below reuses the same value.
        let version = self.buffer.version();
        // A delete-word edit DID bump the version, but its caret should still
        // streak like the equivalent navigation move (M-b): the removed word
        // collapses while the caret glides left across the gap, as ONE concurrent
        // motion. So when `caret_edit_streaks` was set for this sync, treat the
        // move as navigation (not an edit) for the underline-suppression test only.
        // One-shot: reset it so the next sync goes back to the default.
        let streak_override = std::mem::take(&mut self.caret_edit_streaks);
        let is_edit_move = version != self.caret_synced_version && !streak_override;
        self.caret_synced_version = version;
        // Was the keypress driving this sync an OS auto-repeat (a HELD arrow)?
        // One-shot, like `caret_edit_streaks`: consumed here so a following
        // non-keyboard sync (IME/wheel) doesn't inherit a stale held flag.
        let held = std::mem::take(&mut self.caret_held);

        // Map the active isearch state (if any) into render-facing fields: each
        // match CHAR range -> ((l,c),(l,c)) so highlight quads reuse the
        // selection-rect geometry; the current match is shown only by the real
        // amber caret (already moved onto it by handle_search_key).
        let (
            search_matches,
            search_current,
            search_query,
            search_active,
            search_case_sensitive,
            search_replace_active,
            search_replacement,
            search_editing_replacement,
        ) = self.search_view_fields();

        // Build the snapshot once and push it so the pipeline shapes the CURRENT
        // text/zoom. The scroll offset is counted in VISUAL ROWS; row geometry
        // (and thus the cursor's visual row + the document's total rows) does not
        // depend on the scroll value, so we can read those AFTER this first push
        // and only need to re-push if cursor-follow moves the scroll.
        let mut view = ViewState {
            text,
            cursor_line,
            cursor_col,
            scroll_lines: self.scroll_lines,
            zoom: self.zoom,
            selection: self.buffer.selection_line_col(),
            preedit: self.preedit.clone(),
            misspelled: self.spell_cache.clone(),
            is_edit_move,
            held,
            search_matches,
            search_current,
            search_query,
            search_active,
            search_case_sensitive,
            search_replace_active,
            search_replacement,
            search_editing_replacement,
            overlay_active: self.overlay.is_some(),
            // CRISP-BACKDROP exception: the THEME and CARET-STYLE pickers keep the doc
            // crisp behind them (live theme colours / caret preview); every other full
            // overlay gets the frosted-blur backdrop.
            overlay_crisp: self
                .overlay
                .as_ref()
                .map(|o| {
                    matches!(
                        o.kind,
                        crate::overlay::OverlayKind::Theme | crate::overlay::OverlayKind::Caret
                    )
                })
                .unwrap_or(false),
            overlay_query: self
                .overlay
                .as_ref()
                .map(|o| o.query.clone())
                .unwrap_or_default(),
            overlay_items: self
                .overlay
                .as_ref()
                .map(|o| o.item_strings())
                .unwrap_or_default(),
            overlay_bindings: self
                .overlay
                .as_ref()
                .map(|o| o.item_bindings())
                .unwrap_or_default(),
            overlay_times: self
                .overlay
                .as_ref()
                .map(|o| o.item_times())
                .unwrap_or_default(),
            overlay_selected: self.overlay.as_ref().map(|o| o.selected).unwrap_or(0),
            overlay_hint: self
                .overlay
                .as_ref()
                .map(|o| o.foot_hint())
                .unwrap_or_default(),
            // CARET-STYLE PICKER preview: while that picker is open, the look its
            // highlighted row selects (drives the live animated preview box). `None`
            // for every other state, so the preview loop runs ONLY while it is open.
            caret_preview: self
                .overlay
                .as_ref()
                .filter(|o| o.kind == crate::overlay::OverlayKind::Caret)
                .and_then(|o| o.selected_caret_mode()),
            // PAGE-MODE GUTTER: the buffer's display name (saved file name, or the
            // derived scratch/slug name for an unsaved note) over the project name.
            gutter_name: self.buffer.display_name(),
            gutter_project: self.project.name.clone(),
            // MARKDOWN STYLING gate: a buffer is "markdown" only once it has a
            // `.md`/`.markdown` path. An unnamed scratch / `.rs` / `.txt` buffer is
            // left untouched (no markup dimming of `#` comments etc.).
            is_markdown: self.buffer.is_markdown(),
            syn_lang: self.buffer.syntax_lang(),
            // SPELL contextual panel: when the open overlay is the spell picker, its
            // target word span turns the overlay into a small floating panel anchored
            // at the word (no blur). `None` for every other overlay / no overlay.
            overlay_spell: self
                .overlay
                .as_ref()
                .filter(|o| o.kind == crate::overlay::OverlayKind::Spell)
                .and_then(|o| o.spell_target),
        };
        {
            let gpu = self.gpu.as_mut().unwrap();
            gpu.pipeline.set_view(&view);
        }

        // Cursor-follow (an edit / cursor move): adjust the VISUAL-ROW scroll so the
        // cursor's visual row sits in the viewport. FOCUS MODE folds TYPEWRITER
        // scrolling into cursor-follow: while focus is active (Paragraph / Sentence)
        // the cursor's row is CENTERED vertically (the active unit rests at the eye
        // line); when focus is Off the minimal-adjust is kept EXACTLY (only nudge
        // the scroll enough to reveal the row). For a non-wrapped doc the cursor's
        // visual row == its logical line, so the Off path is identical to the
        // previous logical-line cursor-follow.
        let prev_scroll = self.scroll_lines;
        if follow {
            let pipeline = &self.gpu.as_ref().unwrap().pipeline;
            let cursor_row = pipeline.visual_row_of(cursor_line, cursor_col);
            self.scroll_lines = if crate::focus::mode() == crate::focus::FocusMode::Off {
                // Variable-row-height aware: scroll minimally so the cursor's row
                // (taller on a heading) is fully visible, summing real row heights.
                pipeline.scroll_to_show_row(cursor_row, self.scroll_lines, height)
            } else {
                // TYPEWRITER: center the cursor's row (variable-height aware too).
                pipeline.scroll_to_center_row(cursor_row, height)
            };
        }
        // Always keep scroll within document bounds (pixel-accurate "does it fit").
        let max = self.gpu.as_ref().unwrap().pipeline.max_scroll_rows(height);
        self.scroll_lines = self.scroll_lines.min(max);

        // Re-push only if the scroll actually changed (cheap; avoids a redundant
        // reshape on the common no-scroll-change path).
        if self.scroll_lines != prev_scroll {
            view.scroll_lines = self.scroll_lines;
            self.gpu.as_mut().unwrap().pipeline.set_view(&view);
        }
        // Keep the OS candidate window anchored to the (advance-aware) caret.
        self.update_ime_cursor_area();

        // Apply the one-shot caret IMPULSE queued by `apply` for this sync (edit
        // flinch / blocked-action recoil), AFTER the spring target is set above so it
        // rides on top and self-settles back to rest.
        self.apply_caret_impulses();
    }

    /// Map the active isearch state (if any) into the render-facing snapshot fields:
    /// each match CHAR range -> ((l,c),(l,c)) so highlight quads reuse the
    /// selection-rect geometry; the current match is shown only by the real amber
    /// caret (already moved onto it by `handle_search_key`). `None` search -> empty.
    fn search_view_fields(
        &self,
    ) -> (
        Vec<((usize, usize), (usize, usize))>,
        Option<usize>,
        String,
        bool,
        bool,
        bool,
        String,
        bool,
    ) {
        if let Some(st) = self.search.as_ref() {
            let matches = st
                .matches()
                .iter()
                .map(|m| {
                    (
                        self.buffer.char_to_line_col(m.start),
                        self.buffer.char_to_line_col(m.end),
                    )
                })
                .collect();
            (
                matches,
                st.current_index(),
                st.query().to_string(),
                true,
                st.is_case_sensitive(),
                st.is_replace_active(),
                st.replacement().to_string(),
                st.is_editing_replacement(),
            )
        } else {
            (
                Vec::new(),
                None,
                String::new(),
                false,
                false,
                false,
                String::new(),
                false,
            )
        }
    }

    /// Apply the one-shot caret IMPULSE `apply` queued for this sync — the PHASE 2
    /// edit FLINCH (a successful typed char / delete / kill-line) OR the blocked-action
    /// RECOIL — fired in EVERY caret look AFTER `sync_view` set the spring target, so it
    /// rides on top and the spring self-settles it back to rest. One-shot: cleared on
    /// consume. The caller already requested a redraw; the breathe loop plays it out.
    fn apply_caret_impulses(&mut self) {
        // PHASE 2 edit FLINCH: a SUCCESSFUL typed char / delete / kill-line flinches
        // the visual caret (squash-pop + back-kick / inward squash / gulp).
        if let Some(imp) = self.caret_impact.take() {
            if let Some(gpu) = self.gpu.as_mut() {
                match imp {
                    CaretImpact::Type => gpu.pipeline.caret_type_impact(),
                    CaretImpact::Delete => gpu.pipeline.caret_delete_squash(),
                    CaretImpact::Gulp => gpu.pipeline.caret_gulp(),
                }
            }
        }
        // BLOCKED-ACTION RECOIL: a motion/scroll/undo/delete that couldn't proceed
        // bumps the visual caret away from the wall (every caret look).
        if let Some(dir) = self.caret_recoil.take() {
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.pipeline.caret_recoil(dir);
            }
        }
    }

    /// Tell winit where the composition caret is (in physical pixels) so the
    /// platform IME floats its candidate list by the caret. Reads the pipeline's
    /// real caret rect (which already accounts for any active preedit end).
    pub(super) fn update_ime_cursor_area(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let (x, y, w, h) = gpu.pipeline.caret_pixel_rect();
        gpu.window.set_ime_cursor_area(
            winit::dpi::PhysicalPosition::new(x as f64, y as f64),
            winit::dpi::PhysicalSize::new(w.max(1.0) as f64, h.max(1.0) as f64),
        );
    }
}
