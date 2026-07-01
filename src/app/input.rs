//! INPUT handling: the held-HUD release doors, the incremental-search key
//! surface (and its step / jump / abort / replace helpers), wheel scroll +
//! wheel-zoom, the GPU-aware page scroll, the pixel->char hit test, the
//! left/right mouse press + drag selection, and the IME composition
//! lifecycle. Everything `window_event` dispatches into; lifted out verbatim.

use super::*;

impl App {
    /// Dismiss the HELD stats HUD when its trigger key is RELEASED. The press
    /// recorded the logical key in `hud_key`; lifting the SAME key clears the HUD —
    /// this is the whole live "hold to peek" half (the press half rides the normal
    /// keymap → `apply_core` path). Any other release is a no-op.
    pub(super) fn on_key_release(&mut self, released: &Key) {
        if self.hud_key.as_ref() == Some(released) {
            self.clear_hud();
        }
    }

    /// Dismiss the HELD stats HUD when a SUMMONING modifier is released. macOS does not
    /// deliver a key-UP for a character key while Cmd is held (and the user commonly
    /// lifts Cmd before the letter), so `on_key_release` alone leaves the HUD stuck-on;
    /// a `ModifiersChanged` that drops any modifier present at summon time means the
    /// hold chord is broken, so the HUD vanishes. The pure decision is
    /// [`hud_mods_broken`] (unit-tested without a window).
    pub(super) fn hud_release_on_mods(&mut self, now: ModifiersState) {
        if self.hud_key.is_some() && hud_mods_broken(self.hud_mods, now) {
            self.clear_hud();
        }
    }

    /// Clear the held stats HUD: drop the process-global held flag, forget the trigger
    /// key/modifiers, and re-sync + redraw so the panel and its scrim vanish. Shared by
    /// both dismissal doors (`on_key_release` for the key, `hud_release_on_mods` for the
    /// modifier) so the HUD is a true momentary hold — gone the instant the chord lifts.
    pub(super) fn clear_hud(&mut self) {
        crate::hud::set_held(false);
        self.hud_key = None;
        self.hud_mods = ModifiersState::empty();
        self.sync_view(false);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// Route a key to the active search surface (only called while `self.search`
    /// is `Some`). Mirrors the keymap's modifier extraction. Consumes EVERY key:
    /// printable chars extend the query, Backspace shortens it, C-s/C-r step
    /// next/prev, Enter accepts, Esc / C-g abort, M-c toggles case. After any
    /// change that yields a current match, the REAL buffer cursor is moved onto
    /// it so the existing amber caret shows the current match for free.
    pub(super) fn handle_search_key(
        &mut self,
        logical: &Key,
        mods: &Modifiers,
        _event_loop: &ActiveEventLoop,
    ) {
        use winit::keyboard::NamedKey;
        let state = mods.state();
        let ctrl = state.contains(ModifiersState::CONTROL);
        let alt = state.contains(ModifiersState::ALT);
        let sup = state.contains(ModifiersState::SUPER);
        let shift = state.contains(ModifiersState::SHIFT);
        // Which field a self-insert / Backspace edits: the replacement (true) or
        // the search query (false). A bool copy, so the immutable borrow is dropped
        // before the arms below take a mutable borrow of `self.search`.
        let editing_replacement = self
            .search
            .as_ref()
            .map(|s| s.is_editing_replacement())
            .unwrap_or(false);

        match logical {
            Key::Character(s) => {
                let Some(c) = s.chars().next() else { return };
                // Cmd-based Find/Replace chords WITHIN the panel: Cmd-F skips to the
                // next match, Cmd-Shift-F the previous (so you can pass a match without
                // replacing it), Cmd-Option-F reveals+toggles the replace field, and
                // Cmd-R focuses the replace field (the headline door — a fresh Cmd-R
                // opened the panel on the find field). Other Super combos are consumed.
                if sup && !ctrl {
                    if c.eq_ignore_ascii_case(&'f') {
                        if alt {
                            if let Some(st) = self.search.as_mut() {
                                st.toggle_replace();
                            }
                        } else if shift {
                            self.search_step(Direction::Backward);
                        } else {
                            self.search_step(Direction::Forward);
                        }
                    } else if c.eq_ignore_ascii_case(&'r') && !alt {
                        if let Some(st) = self.search.as_mut() {
                            st.focus_replacement();
                        }
                    }
                    return;
                }
                if ctrl && !alt {
                    match c.to_ascii_lowercase() {
                        's' => self.search_step(Direction::Forward),
                        'r' => self.search_step(Direction::Backward),
                        'g' => self.search_abort(),
                        _ => {} // other ctrl combos: consumed, no-op
                    }
                } else if alt && !ctrl {
                    if matches!(c, 'c' | 'C') {
                        // M-c / Alt+c toggles case sensitivity.
                        let hay = self.buffer.text();
                        if let Some(st) = self.search.as_mut() {
                            st.toggle_case(&hay);
                        }
                        self.search_jump_to_current();
                    }
                } else if !c.is_control() {
                    // Self-insert into the FOCUSED field. The replacement is not
                    // searched, so typing it never moves a match; query edits do.
                    if editing_replacement {
                        if let Some(st) = self.search.as_mut() {
                            st.push_replace_char(c);
                        }
                    } else {
                        let hay = self.buffer.text();
                        if let Some(st) = self.search.as_mut() {
                            st.push_char(c, &hay);
                        }
                        self.search_jump_to_current();
                    }
                }
            }
            // Tab is the one FIELD-SWITCH key: flip focus find↔replace (revealing the
            // replace row the first time). No longer overloaded — Enter replaces, Tab
            // only moves between the two fields of the one warm panel.
            Key::Named(NamedKey::Tab) => {
                if let Some(st) = self.search.as_mut() {
                    st.toggle_replace();
                }
            }
            // Down / Up SKIP to the next / previous match without replacing (alongside
            // Cmd-F / Cmd-Shift-F), so you can pass over a match you don't want changed.
            Key::Named(NamedKey::ArrowDown) => self.search_step(Direction::Forward),
            Key::Named(NamedKey::ArrowUp) => self.search_step(Direction::Backward),
            Key::Named(NamedKey::Backspace) => {
                if editing_replacement {
                    if let Some(st) = self.search.as_mut() {
                        st.pop_replace_char();
                    }
                } else {
                    let hay = self.buffer.text();
                    if let Some(st) = self.search.as_mut() {
                        st.pop_char(&hay);
                    }
                    self.search_jump_to_current();
                }
            }
            Key::Named(NamedKey::Enter) => {
                // The clarified core loop: once replace is active, Enter ALWAYS
                // replaces the current match + advances to the next (regardless of
                // which field has focus) — Cmd-Enter replaces ALL. In a PLAIN find
                // (no replace row), Enter ACCEPTS (closes, leaving the cursor on the
                // current match). Esc / C-g is the "done" door out of replace.
                let replace_active = self
                    .search
                    .as_ref()
                    .map(|s| s.is_replace_active())
                    .unwrap_or(false);
                if sup && replace_active {
                    self.search_replace_all();
                } else if replace_active {
                    self.search_replace_current();
                } else {
                    self.search = None;
                    self.buffer.seal_undo_group();
                }
            }
            Key::Named(NamedKey::Space) if !ctrl && !alt && !sup => {
                // Space arrives as a Named key (not a Character), so without this
                // arm it would fall through to the no-op below and never reach the
                // focused field. Ctrl/Alt/Cmd+Space stay no-ops.
                if editing_replacement {
                    if let Some(st) = self.search.as_mut() {
                        st.push_replace_char(' ');
                    }
                } else {
                    let hay = self.buffer.text();
                    if let Some(st) = self.search.as_mut() {
                        st.push_char(' ', &hay);
                    }
                    self.search_jump_to_current();
                }
            }
            Key::Named(NamedKey::Escape) => self.search_abort(),
            _ => {} // any other named key: consumed, no-op
        }
    }

    /// C-s / C-r while searching: advance to the next/previous match (wrapping)
    /// and move the real cursor onto it.
    pub(super) fn search_step(&mut self, dir: Direction) {
        let outcome = self.search.as_mut().map(|st| st.step(dir));
        // A forward step that FAILS at the last match (backward at the first) does
        // NOT advance — it recoils the caret and arms the two-press wrap. Bump the
        // caret away from the search-travel wall (forward travels toward the end ->
        // bump UP; backward -> DOWN), mirroring the blocked-motion recoil.
        if let Some(crate::search::StepOutcome::RecoiledAtBoundary(d)) = outcome {
            self.caret_recoil = Some(match d {
                Direction::Forward => crate::caret::RecoilDir::Up,
                Direction::Backward => crate::caret::RecoilDir::Down,
            });
        }
        self.search_jump_to_current();
    }

    /// Move the real buffer cursor onto the current match (if any) so the amber
    /// document caret lands on it. No-op (cursor unchanged) when there is no
    /// current match — we don't jump on a no-match query.
    pub(super) fn search_jump_to_current(&mut self) {
        if let Some(st) = self.search.as_ref() {
            if let Some(m) = st.current_match() {
                self.buffer.set_cursor(m.start);
            }
        }
    }

    /// Esc / C-g: restore the cursor to where search began and close the panel.
    pub(super) fn search_abort(&mut self) {
        if let Some(st) = self.search.as_ref() {
            let origin = st.origin();
            self.buffer.set_cursor(origin);
        }
        self.buffer.clear_mark();
        self.search = None;
    }

    /// REPLACE-CURRENT (Enter in the replace field): swap the active match for the
    /// replacement text, write the new document back as one atomic edit, and ADVANCE
    /// the search to the next match (the cursor follows). The panel stays open so a
    /// repeated Enter walks forward replacing. A no-op unless replace mode is active
    /// and there is a current match.
    pub(super) fn search_replace_current(&mut self) {
        let hay = self.buffer.text();
        let new_text = match self.search.as_mut() {
            Some(st) if st.is_replace_active() => st.replace_current_text(&hay),
            _ => return,
        };
        if let Some(t) = new_text {
            self.buffer.set_text(&t);
            self.search_jump_to_current();
        }
    }

    /// REPLACE-ALL (Cmd-Enter): swap EVERY current-query match for the replacement
    /// in one atomic, undoable edit, then re-anchor the (now usually empty) match
    /// set at the search origin. A no-op unless replace mode is active and the text
    /// actually changes.
    pub(super) fn search_replace_all(&mut self) {
        let hay = self.buffer.text();
        let (new_text, origin) = match self.search.as_ref() {
            Some(st) if st.is_replace_active() => (st.replace_all_text(&hay), st.origin()),
            _ => return,
        };
        if new_text == hay {
            return;
        }
        self.buffer.set_text(&new_text);
        let new_hay = self.buffer.text();
        if let Some(st) = self.search.as_mut() {
            st.refind(origin, &new_hay);
        }
        self.search_jump_to_current();
    }

    /// Set the zoom factor (clamped) and reset glyph metrics on next sync. The
    /// wheel-zoom path; also arms the debounced STICKY-ZOOM write.
    pub(super) fn set_zoom(&mut self, z: f32) {
        let clamped = render::clamp_zoom(z);
        if clamped != self.zoom {
            self.zoom = clamped;
            self.mark_zoom_dirty();
        }
    }

    /// Arm the DEBOUNCED sticky-zoom write: stamp "now" so `about_to_wait` persists
    /// the settled zoom after `ZOOM_PERSIST_DEBOUNCE` of quiet (one write per rapid
    /// Cmd-=/Cmd-- run, not one-per-step). Kicks a redraw so the loop reaches
    /// `about_to_wait` to schedule the flush even if nothing else is animating.
    pub(super) fn mark_zoom_dirty(&mut self) {
        self.zoom_persist_at = Some(Instant::now());
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// C-v / M-v: move the cursor by (roughly) one screenful of lines, Emacs
    /// style. `dir` is +1 (down) or -1 (up). The subsequent cursor-follow sync
    /// scrolls the viewport to keep the cursor visible. Returns whether the cursor
    /// actually moved — `false` means the page was BLOCKED (already at the top /
    /// bottom), which the caller turns into a caret recoil.
    pub(super) fn scroll_page(&mut self, dir: isize) -> bool {
        let cursor_before = self.buffer.cursor_line_col();
        let visible = if let Some(gpu) = self.gpu.as_ref() {
            let line_height = render::LINE_HEIGHT * self.zoom * self.dpi;
            render::visible_lines_z(gpu.config.height as f32, line_height)
        } else {
            1
        };
        // A "screenful" is now ~one viewport of VISUAL rows (leave a couple of
        // rows of overlap for context). Move the cursor one logical line at a
        // time, but stop once its VISUAL row has advanced by about a screenful —
        // so paging through a wrapped doc advances by what's on screen, not by a
        // screenful of LOGICAL lines (which would overshoot far past the viewport).
        let target_rows = visible.saturating_sub(2).max(1);
        // The cursor's visual row before paging; the loop stops once we've moved
        // ~target_rows visual rows away. Falls back to a logical-line page (the
        // old behavior) when the pipeline isn't up yet.
        let start_row = match self.gpu.as_ref() {
            Some(gpu) => {
                let (l, c) = self.buffer.cursor_line_col();
                Some(gpu.pipeline.visual_row_of(l, c))
            }
            None => None,
        };
        // Hard cap on logical-line steps so we can never loop unbounded: at most
        // target_rows logical lines (each logical line is >= 1 visual row).
        for _ in 0..target_rows {
            let before = self.buffer.cursor_line_col();
            if dir > 0 {
                self.buffer.next_line();
            } else {
                self.buffer.previous_line();
            }
            let after = self.buffer.cursor_line_col();
            // Reached a buffer boundary (cursor didn't move): stop.
            if after == before {
                break;
            }
            if let (Some(start), Some(gpu)) = (start_row, self.gpu.as_ref()) {
                let row = gpu.pipeline.visual_row_of(after.0, after.1);
                let moved = (row as isize - start as isize).unsigned_abs();
                if moved >= target_rows {
                    break;
                }
            }
        }
        self.buffer.cursor_line_col() != cursor_before
    }

    /// Map the current mouse pixel position to a buffer char index, accounting
    /// for scroll + zoom, then clamp to the document. Returns the char index.
    pub(super) fn hit_test_char(&self) -> usize {
        let (px, py) = self.cursor_px;
        // Advance-aware hit test: walk the REAL shaped glyph advances so a click
        // lands on the right glyph for mixed CJK + Latin lines. Falls back to the
        // fixed-pitch free function only if the pipeline is not yet up.
        let (line, col) = match self.gpu.as_ref() {
            Some(gpu) => gpu.pipeline.hit_test(px, py, self.scroll_lines),
            None => render::hit_test(
                px,
                py,
                self.scroll_lines,
                &render::Metrics::with_dpi(self.zoom, self.dpi),
                render::TEXT_LEFT,
            ),
        };
        self.buffer.line_col_to_char(line, col)
    }

    /// Handle a primary-button press: hit-test, set the anchor, and (for double
    /// / triple clicks) select the word / line under the cursor.
    pub(super) fn on_press(&mut self) {
        let now = Instant::now();
        // Multi-click detection: same spot, within the time window.
        let near = {
            let (lx, ly) = self.last_click_px;
            (self.cursor_px.0 - lx).abs() < 4.0 && (self.cursor_px.1 - ly).abs() < 4.0
        };
        let recent = self
            .last_click_time
            .map(|t| now.duration_since(t) < Duration::from_millis(MULTICLICK_MS))
            .unwrap_or(false);
        if recent && near {
            self.click_count = (self.click_count % 3) + 1;
        } else {
            self.click_count = 1;
        }
        self.last_click_time = Some(now);
        self.last_click_px = self.cursor_px;

        // A click is a non-edit gesture: seal the open undo group so text typed
        // after relocating the cursor is its own undo step.
        self.buffer.seal_undo_group();
        let idx = self.hit_test_char();
        self.dragging = true;
        match self.click_count {
            1 => {
                // Single click: place the cursor, clear any selection.
                self.drag_granularity = DragGranularity::Char;
                self.buffer.set_cursor(idx);
                self.buffer.clear_mark();
                self.buffer.set_anchor(idx);
                self.shift_selecting = false;
            }
            2 => {
                // Double click: select the word under the cursor.
                self.drag_granularity = DragGranularity::Word;
                let (s, e) = self.buffer.word_bounds(idx);
                self.buffer.select_range(s, e);
            }
            _ => {
                // Triple click: select the whole line.
                self.drag_granularity = DragGranularity::Line;
                let (s, e) = self.buffer.line_bounds(idx);
                self.buffer.select_range(s, e);
            }
        }
    }

    /// A pointer HOVER over an open picker: hit-test the row under the cursor and move
    /// the selection onto it — the mouse twin of an arrow-key move. It applies the SAME
    /// live preview a keyboard move does (`actions::preview_overlay`: the Theme picker
    /// re-tints to the hovered world, the Caret picker swaps the look; every flat picker
    /// is inert), so hovering previews exactly like arrowing. A calm no-op when the
    /// pointer is off the rows or already on the highlighted one. Uniform across EVERY
    /// picker kind — the row geometry comes from the one `overlay_row_at` hit-test.
    pub(super) fn overlay_hover(&mut self) {
        let hit = self
            .gpu
            .as_ref()
            .and_then(|g| g.pipeline.overlay_row_at(self.cursor_px.0, self.cursor_px.1));
        let Some(idx) = hit else { return };
        // Move the selection onto the hovered row (no-op if it is already there).
        let kind = match self.overlay.as_mut() {
            Some(ov) if idx < ov.items.len() && ov.selected != idx => {
                ov.selected = idx;
                ov.kind
            }
            _ => return,
        };
        // LIVE PREVIEW, identical to the keyboard nav path.
        if let Some(ov) = self.overlay.as_ref() {
            crate::actions::preview_overlay(ov);
        }
        // A Theme preview mutated the process-global active world: re-tint the baked GPU
        // pipelines + window title so the hover previews it live, mirroring the theme
        // branch of `post_apply_effects`.
        if kind == crate::overlay::OverlayKind::Theme {
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.pipeline.sync_theme();
            }
            self.update_title();
        }
        self.sync_view(false);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// A LEFT-CLICK while a picker is open: if it lands on a candidate row, move the
    /// selection there and ACCEPT it — the exact `Action::Newline` the keyboard's Enter
    /// runs, so a click opens the file / runs the command / commits the theme / descends
    /// the folder identically (one path, every kind). A click OFF the rows is SWALLOWED
    /// (the picker is modal — it never falls through to `on_press`, which would place the
    /// document cursor beneath the card). Always consumes the click while an overlay is
    /// open.
    pub(super) fn overlay_click(&mut self, event_loop: &ActiveEventLoop) {
        let hit = self
            .gpu
            .as_ref()
            .and_then(|g| g.pipeline.overlay_row_at(self.cursor_px.0, self.cursor_px.1));
        let Some(idx) = hit else { return };
        if let Some(ov) = self.overlay.as_mut() {
            if idx < ov.items.len() {
                ov.selected = idx;
            }
        }
        // ACCEPT through the shared apply path — byte-for-byte the same as Enter on the
        // highlighted row (open / run / commit / descend / replace, per kind).
        self.apply(Action::Newline, false, event_loop);
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// Handle a SECONDARY-button (right-click) press: hit-test + place the cursor at
    /// the word under the pointer exactly like a single left-click (no drag, no
    /// selection), then summon the EXISTING spell-suggestion picker for that word.
    /// Misspelled → suggestions; otherwise `OpenSpellSuggest` no-ops (calm). Zero new
    /// spell logic — it reuses the same `suggest_at` path Cmd-`;` uses.
    pub(super) fn on_right_press(&mut self, event_loop: &ActiveEventLoop) {
        // A click is a non-edit gesture: seal the open undo group first.
        self.buffer.seal_undo_group();
        let idx = self.hit_test_char();
        self.dragging = false;
        self.buffer.set_cursor(idx);
        self.buffer.clear_mark();
        self.buffer.set_anchor(idx);
        self.shift_selecting = false;
        // Fire the spell picker for the word now under the cursor (same Action the
        // Cmd-`;` chord runs, so the overlay + sidecar behave identically).
        let _ = self.apply(Action::OpenSpellSuggest, false, event_loop);
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// Handle mouse motion while the button is held: extend the selection to the
    /// current pixel position, by the drag's granularity (char/word/line).
    pub(super) fn on_drag(&mut self) {
        if !self.dragging {
            return;
        }
        let idx = self.hit_test_char();
        match self.drag_granularity {
            DragGranularity::Char => self.buffer.set_cursor(idx),
            DragGranularity::Word => {
                // Extend by whole words: keep the original anchor word, move the
                // cursor to the far edge of the word under the pointer.
                let anchor = self.buffer.anchor_char().unwrap_or(idx);
                let (ws, we) = self.buffer.word_bounds(idx);
                if idx >= anchor {
                    self.buffer.set_cursor(we);
                } else {
                    self.buffer.set_cursor(ws);
                }
            }
            DragGranularity::Line => {
                let anchor = self.buffer.anchor_char().unwrap_or(idx);
                let (ls, le) = self.buffer.line_bounds(idx);
                if idx >= anchor {
                    self.buffer.set_cursor(le);
                } else {
                    self.buffer.set_cursor(ls);
                }
            }
        }
    }

    // === DIRECT-MANIPULATION PAGE RESIZE ================================
    // In PAGE MODE, hovering the pointer within a few px of the centered column's
    // left/right surface edge summons a horizontal-resize cursor (no visible handle —
    // awl-minimal, the proximity IS the affordance); a press-drag there adjusts the
    // settable PAGE WIDTH live, symmetric about center, and release persists it. The
    // hover decision + the drag→measure math are pure (`TextPipeline::page_resize_hover`
    // / `page_resize_measure_at`, unit-tested in `render::geometry`); the CURSOR flip +
    // the in-motion drag FEEL are LIVE-ONLY (a real winit window, not headless).

    /// LIVE-ONLY: flip the OS cursor to the column-resize glyph while the pointer hovers
    /// a page-column edge, and back to the default when it leaves. Set only on a CHANGE
    /// so we don't spam winit every move. The pure hover test is `page_resize_hover`.
    pub(super) fn update_resize_cursor(&mut self) {
        let hovering = self
            .gpu
            .as_ref()
            .map(|g| g.pipeline.page_resize_hover(self.cursor_px.0))
            .unwrap_or(false);
        if hovering == self.resize_cursor_on {
            return;
        }
        self.resize_cursor_on = hovering;
        if let Some(gpu) = self.gpu.as_ref() {
            let icon = if hovering { CursorIcon::ColResize } else { CursorIcon::Default };
            gpu.window.set_cursor(icon);
        }
    }

    /// If a left press landed ON a page-column edge, begin a DIRECT page-width resize
    /// drag (symmetric about center) instead of a text selection, and snap the edge to
    /// the press x. Returns whether a resize began (so the caller skips `on_press`).
    /// LIVE-ONLY gesture; the hover test + measure math it calls are unit-tested.
    pub(super) fn begin_page_resize_if_hovering(&mut self) -> bool {
        let hovering = self
            .gpu
            .as_ref()
            .map(|g| g.pipeline.page_resize_hover(self.cursor_px.0))
            .unwrap_or(false);
        if !hovering {
            return false;
        }
        // A resize is a non-edit gesture: seal the open undo group like a click does.
        self.buffer.seal_undo_group();
        self.page_resizing = true;
        self.apply_page_resize();
        true
    }

    /// LIVE page-width drag step: re-derive the measure from the pointer and re-wrap.
    /// Only the release (`end_page_resize`) persists the sticky width.
    pub(super) fn on_page_resize_drag(&mut self) {
        if !self.page_resizing {
            return;
        }
        self.apply_page_resize();
    }

    /// Set the page MEASURE from the current pointer x (symmetric about the window
    /// center, clamped to the band), re-wrap the buffer at the new column width, and
    /// redraw. Shared by the initial press + every drag move. Re-wrap mirrors the
    /// `PageWider`/`PageNarrower` command path (`set_size` reshapes at the new width).
    fn apply_page_resize(&mut self) {
        let target = self
            .gpu
            .as_ref()
            .map(|g| g.pipeline.page_resize_measure_at(self.cursor_px.0));
        if let Some(target) = target {
            if target != crate::page::measure() {
                crate::page::set_measure(target);
                if let Some(gpu) = self.gpu.as_mut() {
                    let (w, h) = (gpu.config.width as f32, gpu.config.height as f32);
                    gpu.pipeline.set_size(w, h);
                }
                self.sync_view(true);
            }
        }
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// Finish a page-width resize on button RELEASE: drop the drag flag and PERSIST the
    /// settled width (sticky, exactly like the C-x } / C-x { keyboard commands).
    pub(super) fn end_page_resize(&mut self) {
        self.page_resizing = false;
        self.persist_page_width();
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// Handle a platform IME event (Japanese/CJK composition lifecycle).
    ///
    /// * `Enabled`/`Disabled` track whether the IME is active; a Disable clears
    ///   any dangling preedit so a stale composition never lingers.
    /// * `Preedit(text, _)` stores the in-progress composition as a transient
    ///   overlay (rendered underlined at the caret) WITHOUT touching the buffer.
    ///   An empty preedit clears it.
    /// * `Commit(text)` inserts the finalized text (the chosen kanji/kana) into
    ///   the ropey buffer at the cursor and clears the preedit.
    pub(super) fn handle_ime(&mut self, ime: Ime) {
        match ime {
            Ime::Enabled => {
                self.ime_enabled = true;
            }
            Ime::Disabled => {
                self.ime_enabled = false;
                self.preedit.clear();
            }
            Ime::Preedit(text, _cursor) => {
                // The provisional composition string; shown underlined at the
                // caret. Empty => composition ended/cleared.
                self.preedit = text;
            }
            Ime::Commit(text) => {
                // Finalize: the preedit is replaced by the committed text, which
                // is the only part that actually enters the buffer.
                self.preedit.clear();
                for c in text.chars() {
                    self.buffer.insert_char(c);
                }
            }
        }
    }

    /// Apply a wheel scroll of `lines` (positive = content moves up / scroll
    /// down). Free scroll: moves the viewport WITHOUT moving the cursor.
    pub(super) fn wheel_scroll(&mut self, lines: f32) {
        // The scroll unit is a VISUAL ROW. The wheel delta is already in rows
        // (line notches / accumulated pixels per row), so just clamp to the
        // document's total-visual-row max so a wrapped doc can scroll all the way
        // to its last visual row.
        let max = if let Some(gpu) = self.gpu.as_ref() {
            gpu.pipeline.max_scroll_rows(gpu.config.height as f32)
        } else {
            0
        };
        // Round toward the scroll direction so small notches still move.
        let delta = lines.round() as isize;
        let cur = self.scroll_lines as isize;
        let next = (cur + delta).clamp(0, max as isize);
        self.scroll_lines = next as usize;
    }
}
