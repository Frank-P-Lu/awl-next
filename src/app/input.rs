//! INPUT handling: the held-HUD release doors, the incremental-search key
//! surface (and its step / jump / abort / replace helpers), wheel scroll +
//! wheel-zoom, the GPU-aware page scroll, the pixel->char hit test, the
//! left/right mouse press + drag selection, and the IME composition
//! lifecycle. Everything `window_event` dispatches into; lifted out verbatim.

use super::*;

/// INLINE-IMAGE DRAG-RESIZE (v2, live app only): the in-flight state of a
/// bottom-right-handle drag on an inline image. Snapshotted at press
/// ([`App::begin_image_resize_if_hovering`]) and carried until release: the image's
/// document byte `range` (the `![alt](path)` span — the write-back target), the
/// image's on-screen LEFT edge (`image_left`, the drag anchor — width = pointer.x −
/// image_left), and the current live-preview `width` (pipeline state, NOT a buffer
/// edit, until the release stamps the `|NNN` hint back as one undoable edit).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ImageDrag {
    /// Document byte range of the `![alt](path)` image span (write-back target).
    pub(crate) range: (usize, usize),
    /// On-screen LEFT edge (px) of the image rect — the drag anchor.
    pub(crate) image_left: f32,
    /// The current live-preview DISPLAY WIDTH (px); rounded to the `|NNN` hint on release.
    pub(crate) width: f32,
}

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

    /// WHICH-KEY prefix sync, run right after every `keymap.resolve`. Reads the
    /// keymap's post-resolve prefix state: MID-PREFIX (a `C-x` was just pressed,
    /// awaiting its second key) ARMS the pause timer by stamping `prefix_pending_at`;
    /// any other outcome (the prefix resolved to a command, or aborted via `Esc`/`C-g`)
    /// DISMISSES the panel + disarms. The timer itself (the ~500ms wake) lives in
    /// `about_to_wait` and only fires while `prefix_pending_at` is `Some` — so the
    /// summon costs nothing until a prefix actually hangs (DESIGN §6).
    pub(super) fn sync_whichkey_prefix(&mut self) {
        let transition = crate::whichkey::on_key(
            self.keymap.in_prefix(),
            self.prefix_pending_at.is_some(),
            self.whichkey_shown,
        );
        match transition {
            // Freshly mid-prefix: (re-)arm the pause. The panel is not shown yet — it
            // appears only once the pause elapses in `about_to_wait`.
            crate::whichkey::PrefixTransition::Arm => {
                self.prefix_pending_at = Some(Instant::now());
            }
            // The prefix just resolved or aborted: put the panel down at once (summoned
            // + transient — it never lingers past the chord).
            crate::whichkey::PrefixTransition::Dismiss => self.dismiss_whichkey(),
            crate::whichkey::PrefixTransition::Ignore => {}
        }
    }

    /// Summon the which-key panel NOW (the pause elapsed with the prefix still pending):
    /// derive the pending prefix's continuation rows from the command CATALOG (config
    /// overrides folded in, so the panel can't drift) and push them into the pipeline,
    /// then redraw. Marks `whichkey_shown` so the pause timer stops re-arming.
    pub(super) fn summon_whichkey(&mut self) {
        self.whichkey_shown = true;
        let rows: Vec<(String, String)> = crate::whichkey::continuations_cx(&self.config.keys)
            .into_iter()
            .map(|c| (c.key, c.name))
            .collect();
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.pipeline.set_whichkey(Some(rows));
            gpu.window.request_redraw();
        }
    }

    /// Put the which-key panel down + disarm the pause timer. Idempotent — clearing an
    /// already-down panel just redraws nothing new. Redraws only when the panel was
    /// actually shown, so a bare prefix that never paused long enough costs no repaint.
    pub(super) fn dismiss_whichkey(&mut self) {
        self.prefix_pending_at = None;
        let was_shown = self.whichkey_shown;
        self.whichkey_shown = false;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.pipeline.set_whichkey(None);
            if was_shown {
                gpu.window.request_redraw();
            }
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

    /// Multi-click detection: same spot, within the time window (`MULTICLICK_MS`) —
    /// bump the running click count (wrapping 1/2/3) and stamp `last_click_time` /
    /// `last_click_px` for the NEXT press, then return the now-current count.
    /// Shared by a normal document press ([`Self::on_press`]) and a press on the
    /// draggable page-column edge ([`Self::begin_page_resize_if_hovering`]) so a
    /// double-click reads the same wherever the pointer lands — one owner, so the
    /// two can't drift apart on what counts as "a double-click".
    pub(super) fn bump_click_count(&mut self) -> u32 {
        let now = Instant::now();
        let near = {
            let (lx, ly) = self.last_click_px;
            (self.cursor_px.0 - lx).abs() < 4.0 && (self.cursor_px.1 - ly).abs() < 4.0
        };
        let recent = self
            .last_click_time
            .map(|t| now.duration_since(t) < Duration::from_millis(MULTICLICK_MS))
            .unwrap_or(false);
        self.click_count = if recent && near { (self.click_count % 3) + 1 } else { 1 };
        self.last_click_time = Some(now);
        self.last_click_px = self.cursor_px;
        self.click_count
    }

    /// Handle a primary-button press: hit-test, set the anchor, and (for double
    /// / triple clicks) select the word / line under the cursor. `shift` is
    /// whether Shift was held at press time: a SHIFT-CLICK extends the existing
    /// selection (the standard gesture everywhere — TextEdit/Xcode/browsers/…)
    /// instead of starting a fresh one, so it must never `clear_mark`.
    pub(super) fn on_press(&mut self, shift: bool) {
        let click_count = self.bump_click_count();
        // A click is a non-edit gesture: seal the open undo group so text typed
        // after relocating the cursor is its own undo step.
        self.buffer.seal_undo_group();
        let idx = self.hit_test_char();
        self.dragging = true;
        match click_count {
            1 if shift => {
                // SHIFT-CLICK: keep the mark if one is already active, else drop
                // it at the cursor's CURRENT position (before this click moves
                // it) — then move only the cursor to the hit point. Never
                // `clear_mark`; that's what a plain click is for. Double/triple
                // click arms are unaffected (shift only modifies the single-click
                // arm — a shift+double-click still lands here as click_count 1
                // relative to the NEW spot, since a shift-click is usually a
                // fresh spot rather than a same-spot repeat).
                self.drag_granularity = DragGranularity::Char;
                if self.buffer.anchor_char().is_none() {
                    self.buffer.set_anchor(self.buffer.cursor_char());
                }
                self.buffer.set_cursor(idx);
                self.shift_selecting = true;
            }
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

    /// CLICK-TO-JUMP on a persistent MARGIN OUTLINE row: hit-test the pointer against
    /// the outline's OWN row geometry (`TextPipeline::outline_hit_line`, which folds in
    /// the whole shown/hidden gate — off / non-page / non-md / too-narrow all return
    /// `None`) and, on a hit, jump the caret to that heading's line — the same
    /// `jump_to_line` the retired summoned Outline picker used. Returns whether the
    /// press landed on a row (so the caller skips the document press). A benign,
    /// user-approved navigation affordance (DESIGN.md outline amendment: "click-to-jump
    /// only") — NOT a resizable/focusable sidebar. Never fires while an overlay is open
    /// (its scrim owns the click first, handled upstream in `on_mouse_input`).
    pub(super) fn outline_click(&mut self) -> bool {
        let (px, py) = self.cursor_px;
        let line = self
            .gpu
            .as_ref()
            .and_then(|g| g.pipeline.outline_hit_line(px, py, g.config.height));
        if let Some(line) = line {
            self.jump_to_line(&line.to_string());
            true
        } else {
            false
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
        // Re-highlight ONLY the row genuinely under the pointer AMONG THE VISIBLE ROWS.
        // `hover_select` never moves the scroll window (and rejects a row outside the
        // visible band / already-selected), so hovering the top/bottom edge can't make
        // the list auto-scroll — a hover highlights, it never scrolls.
        let kind = match self.overlay.as_mut() {
            Some(ov) => {
                if !ov.hover_select(idx) {
                    return;
                }
                ov.kind
            }
            None => return,
        };
        // LIVE PREVIEW, identical to the keyboard nav path.
        if let Some(ov) = self.overlay.as_ref() {
            crate::actions::preview_overlay(ov);
        }
        // A Theme preview mutated the process-global active world: re-tint the baked GPU
        // pipelines + window title so the hover previews it live, mirroring the theme
        // branch of `post_apply_effects` — colors instantly, the font reshape deferred
        // to the settle (`retint_theme_preview`), so sweeping the pointer down the
        // list costs one recolor per row, not one reshape storm per row.
        if kind == crate::overlay::OverlayKind::Theme {
            self.retint_theme_preview();
        }
        self.sync_view(false);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// The mouse WHEEL while a picker is OPEN: it OWNS the wheel (the doc behind it does
    /// NOT scroll), advancing the SELECTION like ↑/↓ — wheel DOWN moves the highlight
    /// down, wheel UP moves it up — and the scroll window follows (`move_sel`). `lines` is
    /// the wheel delta in rows (positive = wheel up); a fractional notch rounds. Applies
    /// the same LIVE PREVIEW the keyboard nav does, so wheeling the Theme picker previews
    /// each world exactly like arrowing.
    pub(super) fn overlay_wheel(&mut self, lines: f32) {
        let delta = -(lines.round() as isize); // wheel DOWN (lines < 0) advances (↓)
        if delta == 0 {
            return;
        }
        let kind = match self.overlay.as_mut() {
            Some(ov) => {
                ov.move_sel(delta);
                ov.kind
            }
            None => return,
        };
        if let Some(ov) = self.overlay.as_ref() {
            crate::actions::preview_overlay(ov);
        }
        if kind == crate::overlay::OverlayKind::Theme {
            // Wheel preview: colors now, font reshape on settle (see overlay_hover).
            self.retint_theme_preview();
        }
        self.sync_view(false);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// A LEFT-CLICK while a picker is open, resolved against the overlay card:
    ///   * ON a candidate ROW → move the selection there and ACCEPT it — the exact
    ///     `Action::Newline` the keyboard's Enter runs, so a click opens the file /
    ///     runs the command / commits the theme / descends the folder identically
    ///     (one path, every kind).
    ///   * OUTSIDE the card rect → DISMISS the overlay, routed through the SAME
    ///     `Action::Cancel` Esc / C-g uses (so a Theme / Caret live preview reverts
    ///     too). Click-away-to-dismiss is GENERAL across every summoned overlay
    ///     (palette / pickers / spell / history / …) — the card rect + row hit-test
    ///     both come from the one kind-agnostic `overlay_geometry`.
    ///   * INSIDE the card but off a row (query line / foot hint) → SWALLOWED (the
    ///     picker stays modal; it never falls through to `on_press`, which would place
    ///     the document cursor beneath the card).
    /// Always consumes the click while an overlay is open.
    pub(super) fn overlay_click(&mut self, event_loop: &ActiveEventLoop) {
        let (px, py) = self.cursor_px;
        let (row_hit, lens_hit, card) = self
            .gpu
            .as_ref()
            .map(|g| {
                (
                    g.pipeline.overlay_row_at(px, py),
                    g.pipeline.overlay_lens_at(px, py),
                    g.pipeline.overlay_card_rect(),
                )
            })
            .unwrap_or((None, None, None));

        // FACETED PICKER: a click on a LENS label switches the facet (keeping the
        // selection), then previews + re-tints — the pointing counterpart to LEFT/RIGHT.
        // Handled before the row hit-test (the strip sits above the rows, never overlaps).
        if let Some(lens_idx) = lens_hit {
            if let Some(ov) = self.overlay.as_mut() {
                ov.set_facet_lens(lens_idx);
            }
            if let Some(ov) = self.overlay.as_ref() {
                crate::actions::preview_overlay(ov);
            }
            // Lens-click preview: colors now, font reshape on settle (see overlay_hover).
            self.retint_theme_preview();
            self.sync_view(false);
            if let Some(gpu) = self.gpu.as_ref() {
                gpu.window.request_redraw();
            }
            return;
        }

        if let Some(idx) = row_hit {
            // ON a row: ACCEPT through the shared apply path — byte-for-byte the same
            // as Enter on the highlighted row (open / run / commit / descend / replace).
            if let Some(ov) = self.overlay.as_mut() {
                if idx < ov.items.len() {
                    ov.selected = idx;
                }
            }
            self.apply(Action::Newline, false, event_loop);
        } else {
            // Off the rows. A click INSIDE the card (query line / foot hint) is
            // swallowed to keep the picker modal; a click OUTSIDE the card dismisses it.
            let inside = card
                .map(|[x, y, w, h]| px >= x && px <= x + w && py >= y && py <= y + h)
                .unwrap_or(false);
            if inside {
                return;
            }
            self.apply(Action::Cancel, false, event_loop);
        }
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// A LEFT-CLICK inside the summoned find/replace panel: CLICK-TO-SWITCH-FIELD.
    /// A press on the FIND row focuses the query (`editing_replacement = false`); a
    /// press on the REPLACE row focuses the replacement (`editing_replacement =
    /// true`) — the amber caret then rides the clicked field (Batch-1 fixed the
    /// replace caret-x, so focusing via click places it correctly). A press
    /// ELSEWHERE inside the card (the key-hint line, inter-row gaps) is a calm
    /// no-op — swallowed, never dismissing the search or moving the document cursor
    /// beneath the panel. Returns `true` when the press landed on/in the panel and
    /// was handled; `false` (off the card / panel down) lets the caller fall
    /// through to the normal document press. The find↔replace decision is the pure
    /// `TextPipeline::panel_hit` (unit-tested); this only wires the field state +
    /// redraw, mirroring the two focus doors `handle_search_key` already uses.
    pub(super) fn panel_click(&mut self) -> bool {
        let (px, py) = self.cursor_px;
        let hit = self.gpu.as_ref().and_then(|g| g.pipeline.panel_hit(px, py));
        match hit {
            Some(crate::render::PanelHit::Find) => {
                if let Some(st) = self.search.as_mut() {
                    st.focus_query();
                }
            }
            Some(crate::render::PanelHit::Replace) => {
                if let Some(st) = self.search.as_mut() {
                    st.focus_replacement();
                }
            }
            // In the card but off an editable row: swallow (a calm no-op).
            Some(crate::render::PanelHit::Elsewhere) => {}
            // Off the panel: let the press fall through to the document.
            None => return false,
        }
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
        true
    }

    /// Handle a SECONDARY-button (right-click) press: hit-test + place the cursor at
    /// the word under the pointer exactly like a single left-click (no drag, no
    /// selection), then summon the EXISTING spell-suggestion picker for that word.
    /// Misspelled → suggestions; otherwise `OpenSpellSuggest` no-ops (calm). Zero new
    /// spell logic — it reuses the same `suggest_at` path Cmd-`;` uses.
    pub(super) fn on_right_press(&mut self, event_loop: &ActiveEventLoop) {
        // RE-TARGET: a right press ALWAYS dismisses any open overlay FIRST (through the
        // same `Action::Cancel` Esc uses, so a Theme/Caret preview reverts), then hit-tests
        // the word now under the pointer and opens ITS suggestions. So right-clicking a
        // SECOND misspelling while the first spell menu is open swaps the menu to the new
        // word instead of being swallowed by the modal overlay.
        if self.overlay.is_some() {
            let _ = self.apply(Action::Cancel, false, event_loop);
        }
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

    /// LIVE-ONLY: recompute the CONTEXT-AWARE OS cursor shape (`cursor_shape.rs`) for
    /// the current mouse position + interaction state, and flip `Window::set_cursor`
    /// ONLY when it actually changed (`cursor_shape::cursor_icon_change` — no per-move
    /// winit chatter). Every context flag reads an EXISTING hit-test — `page_resizing`
    /// (the live drag flag), `self.overlay.is_some()`, `page_resize_hover` (the same
    /// proximity test the page-edge press/hover already uses), and
    /// `over_writing_column` (the same column bounds `page_resize_hover` reads) — so
    /// this never invents parallel geometry, it only arbitrates priority among the
    /// existing regions (`cursor_shape::cursor_icon_for`).
    ///
    /// Called on every `CursorMoved`, and again from the two doors that change this
    /// context WITHOUT any mouse motion: a page-edge drag beginning/ending
    /// (`begin_page_resize_if_hovering` / `end_page_resize`) and a summoned overlay
    /// opening/closing (`App::apply`'s one `self.overlay = overlay` assignment).
    ///
    /// COMPOSES with pointer auto-hide: while the OS pointer is `Hidden`
    /// (`pointer_hide::PointerHide`), the `set_cursor` call is skipped outright (there
    /// is nothing visible to update) and the cache is left untouched, so the very next
    /// un-hide — always a `CursorMoved`, which recomputes context before anything else
    /// — compares the fresh icon against the still-accurate cache and lands directly on
    /// the context-correct shape instead of a stale one from before the hide.
    pub(super) fn sync_cursor_icon(&mut self) {
        let Some(gpu) = self.gpu.as_ref() else { return };
        let (px, py) = self.cursor_px;
        // The pointing-hand affordance now covers EVERY summoned picker's clickable
        // rows (Command-P / go-to / browse / theme / history / keybindings / spell /
        // …), not just spell — reuses the SAME kind-agnostic `overlay_row_at`
        // hit-test the pickers' own click handling uses (`overlay_click`), so a
        // hovered row can never disagree with a clickable one. `overlay_row_at`
        // already returns `None` off a row (the query line, foot hint, scrim, empty
        // gaps), so this lights up only on a real actionable row.
        let overlay_open = self.overlay.is_some();
        let over_clickable_overlay_row =
            overlay_open && gpu.pipeline.overlay_row_at(px, py).is_some();
        // The overlay's editable query-filter line reads as a text field (I-beam) —
        // same `overlay_geometry` the field renders from, via `over_overlay_query`.
        let over_query_input = overlay_open && gpu.pipeline.over_overlay_query(px, py);
        // A clickable MARGIN-OUTLINE row reads as click-to-jump (the pointing hand),
        // reusing the outline's OWN row geometry (`outline_hit_line`, which folds in
        // the whole hidden/off gate). Only while no overlay is open — an overlay's
        // scrim covers the outline, so the outline never claims the hand behind it.
        let over_outline_row = !overlay_open
            && gpu
                .pipeline
                .outline_hit_line(px, py, gpu.config.height)
                .is_some();
        // An inline image's bottom-right resize HANDLE reads as the diagonal
        // corner-resize glyph, exactly like the page edge — reuses the SAME
        // `image_handle_at` hit-test the press path uses, over the SAME images
        // layout the `ImageQuadPipeline` draws (no parallel geometry). Only a hover
        // matters here; the active-drag flag rides `self.image_resizing`.
        let over_image_handle = gpu.pipeline.image_handle_at(px, py).is_some();
        let ctx = crate::cursor_shape::CursorContext {
            dragging_edge: self.page_resizing,
            overlay_open,
            over_edge: gpu.pipeline.page_resize_hover(px),
            over_text: gpu.pipeline.over_writing_column(px),
            over_clickable_overlay_row,
            over_query_input,
            over_outline_row,
            dragging_image: self.image_resizing.is_some(),
            over_image_handle,
        };
        let desired = crate::cursor_shape::cursor_icon_for(ctx);
        let hidden = self.pointer_hide == crate::pointer_hide::PointerHide::Hidden;
        if let Some(icon) = crate::cursor_shape::cursor_icon_change(self.cursor_icon, desired, hidden)
        {
            gpu.window.set_cursor(icon);
            self.cursor_icon = icon;
        }
    }

    /// If a left press landed ON a page-column edge, begin a DIRECT page-width resize
    /// drag (symmetric about center) instead of a text selection, and snap the edge to
    /// the press x — UNLESS it's the SECOND click of a DOUBLE-CLICK on the edge, in
    /// which case it RESETS the page width to the built-in default instead
    /// (pointing-not-buttons — the same affordance games/DAWs use on a divider for
    /// "back to default"). Returns whether the edge press was handled (so the caller
    /// skips `on_press`). Shares the SAME multi-click detection `on_press` uses
    /// (`bump_click_count`), so a double-click on the edge is recognized exactly like
    /// a double-click anywhere else in the document. LIVE-ONLY gesture; the hover
    /// test + measure math + the reset action itself are unit-tested.
    pub(super) fn begin_page_resize_if_hovering(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let hovering = self
            .gpu
            .as_ref()
            .map(|g| g.pipeline.page_resize_hover(self.cursor_px.0))
            .unwrap_or(false);
        if !hovering {
            return false;
        }
        // A resize (or a reset) is a non-edit gesture either way: seal the open
        // undo group like a click does, before branching.
        self.buffer.seal_undo_group();
        if self.bump_click_count() == 2 {
            // DOUBLE-CLICK on the draggable edge: reset instead of beginning a drag.
            // Routes through the real Action via `App::apply`, so it is the exact
            // same path the palette command and a rebound `--keys` chord take.
            self.apply(crate::keymap::Action::PageReset, false, event_loop);
            return true;
        }
        self.page_resizing = true;
        // The context flipped to "dragging the edge" WITHOUT any mouse motion: recompute
        // the cursor shape right now (`dragging_edge` outranks everything), not just on
        // the next `CursorMoved`.
        self.sync_cursor_icon();
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
        if let Some(gpu) = self.gpu.as_mut() {
            // DRAG READOUT: a quiet muted char-count near the pointer while the edge
            // is held (Butterick's line-length rule made visible) — live for the
            // whole gesture (press through every move); cleared on release.
            let (px, py) = self.cursor_px;
            gpu.pipeline.set_page_drag_readout(Some((px, py, crate::page::measure())));
            gpu.window.request_redraw();
        }
    }

    /// Finish a page-width resize on button RELEASE: drop the drag flag and PERSIST the
    /// settled width (sticky, exactly like the C-x } / C-x { keyboard commands).
    pub(super) fn end_page_resize(&mut self) {
        self.page_resizing = false;
        self.persist_page_width();
        if let Some(gpu) = self.gpu.as_mut() {
            // Drop the drag readout — gone the instant the edge is released.
            gpu.pipeline.set_page_drag_readout(None);
            gpu.window.request_redraw();
        }
        // The context flipped off "dragging the edge" WITHOUT any mouse motion:
        // recompute now (usually resumes the edge-hover or plain-text shape rather
        // than waiting for the next `CursorMoved`).
        self.sync_cursor_icon();
    }

    // === INLINE-IMAGE DRAG-RESIZE (v2, LIVE-ONLY) ==========================
    // Templated on the page-column drag directly above: BEGIN in the left-Pressed
    // priority chain (ahead of `on_press` + `begin_page_resize_if_hovering`), TRACK
    // a live-preview width in pipeline state (never the buffer), END with ONE
    // undoable `|NNN` write-back on release (templated on `write_back_lang_tag_once`).
    // The pure hit-test (`geometry::image_handle_hit`), px→width clamp
    // (`image_resize_width`), and the alt write-back (`markdown::image_width_hint_edit`)
    // are unit-tested; the gesture itself needs a real window/GPU (no MouseInput in a
    // capture), so it is LIVE-ONLY.

    /// If a left press landed ON an inline image's bottom-right resize HANDLE, begin a
    /// DIRECT drag-resize of that image (its width tracks the pointer, previewed live
    /// without touching the buffer) instead of a text selection. Returns whether the
    /// handle press was handled (so the caller skips the page-resize / doc-click path).
    /// Mirrors [`Self::begin_page_resize_if_hovering`]: seal the open undo group (a
    /// resize is a non-edit gesture until the release), record the drag, flip the
    /// cursor shape now, and apply the first preview step. LIVE-ONLY gesture; the hover
    /// hit-test + width math + the write-back are unit-tested.
    pub(super) fn begin_image_resize_if_hovering(&mut self) -> bool {
        let (px, py) = self.cursor_px;
        // The hit-test lives on the pipeline (where the images layout + the pure
        // `geometry::image_handle_hit` live), mirroring `page_resize_hover` — no raw
        // geometry leaks to the app. Returns the hit image's byte range + left edge.
        let hit = self.gpu.as_ref().and_then(|g| g.pipeline.image_handle_at(px, py));
        let Some((range, image_left)) = hit else {
            return false;
        };
        // A resize is a non-edit gesture: seal the open undo group like a click does,
        // so the single write-back on release is its own clean undo entry.
        self.buffer.seal_undo_group();
        // `width` is a placeholder; `apply_image_resize` below sets it from the pointer.
        self.image_resizing = Some(ImageDrag { range, image_left, width: 0.0 });
        // The context flipped to "dragging an image" WITHOUT any mouse motion:
        // recompute the cursor shape now, not just on the next `CursorMoved`.
        self.sync_cursor_icon();
        self.apply_image_resize();
        true
    }

    /// LIVE image drag-resize step: re-derive the display width from the pointer and
    /// preview it. Only the release ([`Self::end_image_resize`]) writes the buffer.
    pub(super) fn on_image_resize_drag(&mut self) {
        if self.image_resizing.is_none() {
            return;
        }
        self.apply_image_resize();
    }

    /// Set the dragged image's live-preview DISPLAY WIDTH from the current pointer x
    /// (anchored at its left edge, clamped to `[MIN_IMAGE_W, wrap]`), push it to the
    /// pipeline as a preview override (NOT a buffer edit), re-fit + redraw. Shared by
    /// the initial press + every drag move. The re-fit mirrors the page-resize dance:
    /// the pipeline's `set_image_preview` marks itself dirty so the next `sync_view`
    /// forces the reshape that re-runs the image layout at the new width.
    fn apply_image_resize(&mut self) {
        let Some(drag) = self.image_resizing else {
            return;
        };
        let px = self.cursor_px.0;
        let width = self
            .gpu
            .as_ref()
            .map(|g| g.pipeline.image_resize_width_at(px, drag.image_left));
        let Some(width) = width else {
            return;
        };
        if let Some(d) = self.image_resizing.as_mut() {
            d.width = width;
        }
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.pipeline
                .set_image_preview(Some((drag.range.0, drag.range.1, width)));
        }
        self.sync_view(false);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// Finish an image drag-resize on button RELEASE: clear the drag flag + the
    /// pipeline preview, then WRITE the settled `|NNN` width hint back into the image's
    /// alt as ONE undoable edit ([`Self::write_back_image_width`]). Mirrors
    /// [`Self::end_page_resize`]'s clear-then-persist shape.
    pub(super) fn end_image_resize(&mut self) {
        let Some(drag) = self.image_resizing.take() else {
            return;
        };
        if let Some(gpu) = self.gpu.as_mut() {
            // Drop the live preview — the committed `|NNN` hint drives the fit now.
            gpu.pipeline.set_image_preview(None);
        }
        self.write_back_image_width(drag.range, drag.width);
        self.sync_view(false);
        // The context flipped off "dragging an image" WITHOUT any mouse motion.
        self.sync_cursor_icon();
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

    // === window_event ARM BODIES ========================================
    // Lifted verbatim out of `App::window_event`'s `match` (which is now a thin
    // dispatcher). Each method IS one arm; the `return`s inside are the former
    // arm-level early-returns (nothing ran after the match, so they're
    // equivalent). The window-lifecycle / redraw arms live in `app/window.rs`.

    /// `WindowEvent::ModifiersChanged`: track the live modifier state, and let a
    /// dropped SUMMONING modifier break a held stats-HUD chord (e.g. lifting Cmd
    /// of Cmd-I), covering the macOS case where the character key-UP is never
    /// delivered.
    pub(super) fn on_modifiers_changed(&mut self, m: Modifiers) {
        self.mods = m;
        self.hud_release_on_mods(m.state());
    }

    /// `WindowEvent::CursorMoved`: track the pointer, un-hide the auto-hidden OS
    /// pointer, drive whichever pointer OWNER is active (overlay hover / live
    /// page-resize drag / text-selection drag), then recompute the context-aware
    /// cursor shape once for the move regardless of which branch fired.
    pub(super) fn on_cursor_moved(&mut self, position: winit::dpi::PhysicalPosition<f64>) {
        self.cursor_px = (position.x as f32, position.y as f32);
        // POINTER AUTO-HIDE: ANY mouse motion snaps back to Visible instantly —
        // cancels a pending typing-hide countdown and un-hides an already-hidden
        // pointer in the same move (`pointer_hide::on_mouse_move` is always
        // `-> Visible`). `os_visibility_change` decides whether that crossed the
        // hidden/visible boundary, so `set_cursor_visible` is only ever called on
        // an actual change.
        let prev_pointer_hide = self.pointer_hide;
        self.pointer_hide = crate::pointer_hide::on_mouse_move(prev_pointer_hide);
        if let Some(visible) =
            crate::pointer_hide::os_visibility_change(prev_pointer_hide, self.pointer_hide)
        {
            if let Some(gpu) = self.gpu.as_ref() {
                gpu.window.set_cursor_visible(visible);
            }
        }
        // A summoned picker OWNS the pointer (it is modal, the doc receding
        // behind it): a hover moves + previews the row under the cursor, exactly
        // like an arrow move. A live PAGE-WIDTH resize drag owns the pointer next
        // (the grabbed column edge tracks it, re-wrapping live); otherwise a live
        // text selection extends.
        if self.overlay.is_some() {
            self.overlay_hover();
        } else if self.page_resizing {
            self.on_page_resize_drag();
        } else if self.image_resizing.is_some() {
            // A live INLINE-IMAGE drag-resize owns the pointer: the image's width
            // tracks it (previewed live in pipeline state, no buffer edit yet).
            self.on_image_resize_drag();
        } else if self.dragging {
            self.on_drag();
            self.sync_view(true);
            if let Some(gpu) = self.gpu.as_ref() {
                gpu.window.request_redraw();
            }
        }
        // CONTEXT-AWARE CURSOR SHAPE: recompute on every move regardless of which
        // branch above fired (a text-selection drag still reads as "over text",
        // an overlay hover still reads as the plain arrow, …) — one decision, not
        // a per-branch special case. See `cursor_shape.rs`.
        self.sync_cursor_icon();
    }

    /// `WindowEvent::MouseInput`: the left/right press+release surface — input
    /// stamping, the summoned-about-card dismiss, right-click spell suggestions,
    /// and the left-button press/drag/resize/release state machine.
    pub(super) fn on_mouse_input(
        &mut self,
        event_loop: &ActiveEventLoop,
        state: ElementState,
        button: MouseButton,
    ) {
        // DEBUG key→px: a mouse press is input awaiting pixels too — it
        // shares the request_redraw path (left falls through to it below;
        // right redraws inside `on_right_press`). Other buttons return
        // without a frame, so they are not stamped.
        if state == ElementState::Pressed && matches!(button, MouseButton::Left | MouseButton::Right)
        {
            self.stamp_input();
        }
        // SUMMONED ABOUT CARD: like `apply_core`'s own top-of-function key
        // intercept (`actions.rs`), ANY mouse press while the card is open
        // dismisses it and is otherwise fully swallowed — never falls
        // through to spell-suggest, an overlay click, or a document
        // press/selection. See `about.rs`.
        if state == ElementState::Pressed
            && matches!(button, MouseButton::Left | MouseButton::Right)
            && crate::about::about_open()
        {
            crate::about::set_open(false);
            self.sync_view(true);
            if let Some(gpu) = self.gpu.as_ref() {
                gpu.window.request_redraw();
            }
            return;
        }
        // RIGHT-CLICK → spell suggestions: hit-test + place the cursor at the
        // word under the pointer (same hit_test as a left-click), then fire the
        // EXISTING spell-suggestion picker. On a misspelled word it lists
        // corrections; elsewhere it's a calm no-op. Reuses suggest_at /
        // OpenSpellSuggest wholesale — no new spell logic.
        if button == MouseButton::Right {
            if state == ElementState::Pressed {
                self.on_right_press(event_loop);
            }
            return;
        }
        if button != MouseButton::Left {
            return;
        }
        match state {
            ElementState::Pressed => {
                // A summoned picker OWNS the click (modal): a click ON a row
                // ACCEPTS it (same as Enter), a click OUTSIDE the card DISMISSES
                // it (same as Esc), a click inside but off a row is swallowed —
                // it never falls through to move the document cursor beneath the
                // card. Otherwise: a press ON a page-column edge begins a DIRECT
                // width resize (symmetric about center) instead of a text
                // selection; else it's a normal click / selection start.
                if self.overlay.is_some() {
                    self.overlay_click(event_loop);
                } else if self.search.is_some() && self.panel_click() {
                    // CLICK-TO-SWITCH-FIELD: a press on the find/replace panel
                    // focused a field (or was an in-card no-op); it never falls
                    // through to a document press. A press OFF the panel returns
                    // false and continues to the page-resize / doc-click path.
                } else if self.begin_image_resize_if_hovering() {
                    // A press ON an inline image's bottom-right resize HANDLE begins a
                    // DIRECT drag-resize (its width tracks the pointer, previewed live)
                    // instead of a text selection — checked AHEAD of the page-column
                    // edge + the document press, since a handle sits inside the column.
                } else if !self.begin_page_resize_if_hovering(event_loop) {
                    // A press on a persistent MARGIN OUTLINE row jumps the caret to
                    // that heading (click-to-jump) instead of a document press; a press
                    // anywhere else is a normal click / selection start.
                    if !self.outline_click() {
                        let shift = self.mods.state().contains(ModifiersState::SHIFT);
                        self.on_press(shift);
                        self.sync_view(true);
                    }
                }
            }
            ElementState::Released if self.image_resizing.is_some() => {
                // Commit the settled image width: write the `|NNN` hint back as ONE
                // undoable edit (mutually exclusive with a page-resize / selection).
                self.end_image_resize();
            }
            ElementState::Released if self.page_resizing => {
                // Commit + persist the settled page width (sticky).
                self.end_page_resize();
            }
            ElementState::Released => {
                self.dragging = false;
                // A plain click (press + release with no drag) leaves the
                // press-time anchor lingering at the cursor. Collapse it so
                // a subsequent bare motion (C-p, C-n, …) just moves the
                // cursor and does NOT extend a phantom selection. A real
                // drag (or double/triple-click) leaves cursor != anchor,
                // i.e. has_selection(), so its mark is preserved.
                if !self.buffer.has_selection() {
                    self.buffer.clear_mark();
                }
                self.sync_view(true);
            }
        }
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// `WindowEvent::MouseWheel`: an overlay owns the wheel (drives the list),
    /// else Cmd/Super+wheel zooms, else free scroll. Converts the LineDelta /
    /// PixelDelta into a whole-row count first.
    pub(super) fn on_mouse_wheel(&mut self, delta: MouseScrollDelta) {
        // DEBUG key→px: scroll is input awaiting pixels — every wheel
        // path below ends in the arm's request_redraw.
        self.stamp_input();
        // Zoom modifier: Cmd/Super only. (Ctrl must NOT zoom on mac.)
        let zoom_mod = scroll_zoom_intent(self.mods.state());
        // Convert the delta to a line count (LineDelta or PixelDelta).
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, y) => y * WHEEL_LINES_PER_NOTCH,
            MouseScrollDelta::PixelDelta(p) => {
                self.scroll_px_accum += p.y as f32;
                let whole = (self.scroll_px_accum / WHEEL_PIXELS_PER_LINE).trunc();
                self.scroll_px_accum -= whole * WHEEL_PIXELS_PER_LINE;
                whole
            }
        };
        if self.overlay.is_some() {
            // A summoned picker OWNS the wheel (it is modal): wheel drives the
            // LIST (advance the selection/scroll window, like ↑/↓); the document
            // behind it does NOT scroll. Symmetric with the click/hover consume.
            if lines.abs() >= 1.0 {
                self.overlay_wheel(lines);
            }
        } else if zoom_mod {
            // Cmd/Super + wheel: zoom in/out (wheel up = zoom in).
            if lines.abs() >= 1.0 {
                let dir = lines.signum();
                self.set_zoom(self.zoom + dir * render::ZOOM_STEP);
                self.sync_view(true);
            }
        } else if lines.abs() >= 1.0 {
            // Free scroll: wheel up moves content down (scroll up), so a
            // positive wheel y DECREASES the top scroll line.
            self.wheel_scroll(-lines);
            self.sync_view(false);
        }
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// `WindowEvent::Ime`: hand the composition-lifecycle event to `handle_ime`
    /// then re-sync + redraw.
    pub(super) fn on_ime(&mut self, ime: Ime) {
        self.handle_ime(ime);
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// `WindowEvent::KeyboardInput`: the full press pipeline — release handling,
    /// the preedit / lone-modifier / search / rebind-capture guards, the macOS
    /// Option dead-key fix, then keymap resolve → `apply`. Preserves every
    /// early-return exactly.
    pub(super) fn on_keyboard_input(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: winit::event::KeyEvent,
    ) {
        if event.state != ElementState::Pressed {
            // KEY RELEASE: the only release awl acts on is lifting the HELD
            // stats-HUD key — a true hold, dismissed the instant it lifts. The
            // press recorded the trigger key in `hud_key`; releasing the SAME
            // logical key clears the HUD and re-syncs so it vanishes. Every
            // other release stays a no-op.
            if event.state == ElementState::Released {
                self.on_key_release(&event.logical_key);
            }
            return;
        }
        // While composing (a non-empty preedit), the IME owns these keys:
        // they are delivered separately as Ime::Preedit/Commit, so do NOT
        // also route them through the keymap (which would insert raw
        // romaji or move the cursor mid-composition). This guard runs
        // BEFORE the search guard on purpose: the IME wins over search,
        // and because C-s is swallowed here, a search cannot start
        // mid-composition.
        if !self.preedit.is_empty() {
            return;
        }
        // Ignore lone modifier presses.
        if let Key::Named(n) = &event.logical_key {
            use winit::keyboard::NamedKey::*;
            if matches!(n, Control | Shift | Alt | Super | Hyper | Meta) {
                return;
            }
        }
        // DEBUG key→px: stamp the dispatch receipt of a real key press —
        // every path from here (search keys, rebind capture, the keymap
        // resolve → apply) ends in request_redraw, so this key's pixels
        // are coming. Placed AFTER the lone-modifier/preedit filters: a
        // bare Ctrl tap or an IME-owned key causes no frame and must not
        // linger as a stale stamp inflating the next input's latency.
        self.stamp_input();
        // POINTER AUTO-HIDE: a real keystroke (past the lone-modifier/IME
        // filters above, same gate `stamp_input` uses) hides the OS
        // pointer IMMEDIATELY — the macOS-native convention
        // (`NSCursor.setHiddenUntilMouseMoves`). Any mouse motion
        // instantly reverses it (the `CursorMoved` arm above); so does
        // the window losing focus (the `Focused(false)` arm above).
        let prev_pointer_hide = self.pointer_hide;
        self.pointer_hide = crate::pointer_hide::on_key(prev_pointer_hide);
        if let Some(visible) =
            crate::pointer_hide::os_visibility_change(prev_pointer_hide, self.pointer_hide)
        {
            if let Some(gpu) = self.gpu.as_ref() {
                gpu.window.set_cursor_visible(visible);
            }
        }
        // SEARCH GUARD: when isearch is active, EVERY key (printable,
        // Backspace, Enter, Esc, C-s, C-r, M-c) is consumed by the search
        // surface and never reaches the keymap, so printable keys extend
        // the query instead of inserting into the rope. Placed AFTER the
        // lone-modifier filter (so a bare Shift/Ctrl tap during search is
        // dropped) and AFTER the preedit guard, but BEFORE keymap.resolve.
        if self.search.is_some() {
            let mods = self.mods;
            self.handle_search_key(&event.logical_key, &mods, event_loop);
            self.sync_view(true);
            if let Some(gpu) = self.gpu.as_ref() {
                gpu.window.request_redraw();
            }
            return;
        }
        // REBIND MENU live CAPTURE: while the menu is RECORDING, the next press
        // IS the binding — intercepted at the CHORD level, BEFORE keymap
        // resolution, so any combo (C-t / M-f / a bare key) is recorded verbatim
        // rather than run. Enter / Esc are EXCLUDED (they finish / abort the
        // capture via the normal resolve → apply_core path below). Option
        // composition is undone (like the dead-key fix) so Option-f records as
        // M-f, not the composed glyph. The headless replay records PLAIN keys
        // through `apply_core` instead; both call `OverlayState::capture_record`.
        if self.capture_recording() {
            let is_ctrl_key = matches!(
                &event.logical_key,
                Key::Named(winit::keyboard::NamedKey::Enter)
                    | Key::Named(winit::keyboard::NamedKey::Escape)
            );
            if !is_ctrl_key {
                let logical = if self.mods.state().contains(ModifiersState::ALT) {
                    key_without_modifiers(&event)
                } else {
                    event.logical_key.clone()
                };
                let combo = crate::keyspec::format_chord(&logical, self.mods.state());
                let finished = self
                    .overlay
                    .as_mut()
                    .map(|o| o.capture_record(combo))
                    .unwrap_or(false);
                if finished {
                    if let Some((slug, binding)) =
                        self.overlay.as_ref().and_then(|o| o.capture_target())
                    {
                        self.rebind_commit(slug, binding, false);
                    }
                }
                self.sync_view(true);
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
                return;
            }
        }
        // Held arrow / motion keys arrive as OS AUTO-REPEAT events
        // (`event.repeat`). Record it for the next `sync_view` so a held
        // navigation move builds a continuous lagging caret trail, while a
        // discrete tap (`repeat == false`) stays gap-suppressed.
        self.caret_held = event.repeat;
        // macOS OPTION DEAD-KEY FIX (LIVE path only): Option composes a
        // letter into a glyph (Option-f -> 'ƒ'), so `event.logical_key` is the
        // composed char and a Meta chord (M-f / M-b / M-w / M-v / M-< / M->)
        // would never match. When ALT is held, resolve the UN-composed key
        // (`key_without_modifiers`) IF it is a real Meta chord; otherwise keep
        // the composed `logical_key` so Option-accent INPUT (Option-e -> é)
        // still types as text. The headless `--keys` replay already sends the
        // un-composed key + ALT, so this branch is exercised only live (its
        // behaviour with a real composing keyboard needs human confirmation).
        let logical = if self.mods.state().contains(ModifiersState::ALT) {
            let bare = key_without_modifiers(&event);
            if self.keymap.is_meta_chord(&bare) {
                bare
            } else {
                event.logical_key.clone()
            }
        } else {
            event.logical_key.clone()
        };
        let action = self.keymap.resolve(&logical, &self.mods);
        // LIFETIME STATS: record this press into the odometer — a keystroke, a
        // printable char iff it resolved to an insert, and the capped active-
        // writing interval since the previous press. On the keyboard-input path
        // past every filter (lone-modifier/IME/preedit/search/capture), so it
        // counts real presses only; config-gated + native-only inside.
        #[cfg(not(target_arch = "wasm32"))]
        self.stats_note_keystroke(matches!(action, Action::InsertChar(_)));
        // WHICH-KEY prefix tracking: read the keymap's post-resolve prefix state.
        // Pressing `C-x` (BeginPrefix) leaves it MID-PREFIX → arm the pause timer
        // (record when, so `about_to_wait` can summon the panel after the pause);
        // any other key resolves/aborts the prefix → dismiss the panel + disarm.
        // Cheap no-op on the common (no-prefix) key.
        self.sync_whichkey_prefix();
        // HELD stats HUD: remember the trigger key AND the modifiers held at
        // summon, so its RELEASE dismisses the HUD — either the key lifting
        // (`on_key_release`) or a summoning modifier dropping (`hud_release_on_mods`,
        // the macOS case where the letter's key-UP never arrives while Cmd is down).
        // The press itself summons it via `apply_core` (sets the process-global); an
        // OS auto-repeat re-affirms the same key/mods.
        if action == Action::ShowStatsHud {
            self.hud_key = Some(logical.clone());
            self.hud_mods = self.mods.state();
        }
        // `M-<` / `M->` need Shift just to TYPE `<` / `>`, so that Shift is
        // INCIDENTAL — it must NOT extend the selection (Emacs treats these
        // as pure motion; select via the mark, `C-Space`). Strip it for those
        // two actions before it reaches the Shift+motion select logic.
        let shift = self.mods.state().contains(ModifiersState::SHIFT)
            && motion_honors_shift_select(&action);
        let exited = self.apply(action, shift, event_loop);
        if exited {
            return;
        }
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }
}

#[cfg(test)]
mod click_tests {
    use super::*;
    use crate::render::{Metrics, TEXT_LEFT, TEXT_TOP};

    // Every `App` below is built via `App::new_hermetic` (see its doc on
    // `App::new` in `app.rs`) — these tests only care about click/selection
    // behavior over a `set_text` fixture, never real file content, so the
    // hermetic constructor's injected `InMemoryFs` + disabled session-restore
    // keep them from ever touching the developer's real
    // `~/.local/share/awl/{session.toml,scratch.md}`.

    /// Place a synthetic press at document (line 0, `col`) — the GPU-less
    /// `hit_test_char` fallback path (`render::hit_test` with fixed-pitch
    /// `Metrics`), so this drives the exact same math a real click does.
    fn press_at_col(app: &mut App, col: usize, shift: bool) {
        let m = Metrics::with_dpi(app.zoom, app.dpi);
        app.cursor_px = (TEXT_LEFT + col as f32 * m.char_width, TEXT_TOP);
        app.on_press(shift);
    }

    #[test]
    fn plain_click_clears_the_mark_and_places_the_cursor() {
        let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
        app.buffer.set_text("hello world");
        app.buffer.set_cursor(0);
        app.buffer.set_mark(); // an existing selection from a prior gesture
        press_at_col(&mut app, 6, false); // "w" of "world"
        assert!(!app.buffer.has_selection(), "a plain click drops any selection");
        assert_eq!(app.buffer.cursor_char(), 6);
    }

    #[test]
    fn shift_click_extends_from_the_cursors_prior_position() {
        // No existing mark: a shift-click must DROP the mark at wherever the
        // cursor already sat (char 0), then move ONLY the cursor to the hit
        // point — never `clear_mark`.
        let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
        app.buffer.set_text("hello world");
        app.buffer.set_cursor(0);
        assert!(app.buffer.anchor_char().is_none());
        press_at_col(&mut app, 6, true);
        assert_eq!(app.buffer.anchor_char(), Some(0), "mark drops at the prior cursor spot");
        assert_eq!(app.buffer.cursor_char(), 6, "cursor moves to the click");
        assert_eq!(app.buffer.selection_range(), Some((0, 6)));
    }

    #[test]
    fn shift_click_keeps_an_already_active_mark() {
        // A mark is already active (e.g. from C-Space or a prior shift-click):
        // a further shift-click must NOT move the mark, only the cursor.
        let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
        app.buffer.set_text("hello world");
        app.buffer.set_cursor(2);
        app.buffer.set_anchor(1); // mark pinned at char 1
        press_at_col(&mut app, 9, true);
        assert_eq!(app.buffer.anchor_char(), Some(1), "an active mark is never disturbed");
        assert_eq!(app.buffer.cursor_char(), 9);
    }

    #[test]
    fn double_and_triple_click_arms_ignore_shift() {
        // The word/line-select arms (click_count 2/3) are untouched by shift —
        // shift only modifies the single-click arm.
        let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
        app.buffer.set_text("hello world");
        // A first click at col 0 primes the multi-click detector; the SECOND
        // press at the same spot (inside `on_press`'s own `bump_click_count`
        // call) is recognized as the double-click, exactly as two real clicks
        // would be.
        press_at_col(&mut app, 0, false);
        press_at_col(&mut app, 0, true);
        // A double click at col 0 still selects the word "hello" wholesale,
        // exactly as an un-shifted double click would.
        assert_eq!(app.buffer.selection_range(), Some((0, 5)));
    }
}
