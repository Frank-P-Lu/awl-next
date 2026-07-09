//! src/app/input/keys.rs — the KEYBOARD input path: the held-HUD release
//! doors, the hold-⌘ peek feed, the whichkey summon/dismiss, the
//! incremental-search key surface (+ its step/jump/abort/replace helpers),
//! zoom + the GPU-aware page scroll (C-v/M-v), the IME composition
//! lifecycle, and `WindowEvent::KeyboardInput`/`ModifiersChanged` dispatch
//! itself. Split out of the former `app/input.rs` monolith (2026-07
//! code-organization pass); see `mouse` for the pointer-press/click/wheel
//! path and `drags` for the page/image resize state machines.

use crate::app::*;

impl App {
    /// Dismiss the HELD stats HUD when its trigger key is RELEASED. The press
    /// recorded the logical key in `hud_key`; lifting the SAME key clears the HUD —
    /// this is the whole live "hold to peek" half (the press half rides the normal
    /// keymap → `apply_core` path). Any other release is a no-op.
    pub(in crate::app) fn on_key_release(&mut self, released: &Key) {
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
    pub(in crate::app) fn hud_release_on_mods(&mut self, now: ModifiersState) {
        if self.hud_key.is_some() && hud_mods_broken(self.hud_mods, now) {
            self.clear_hud();
        }
    }

    /// Clear the held stats HUD: drop the process-global held flag, forget the trigger
    /// key/modifiers, and re-sync + redraw so the panel and its scrim vanish. Shared by
    /// both dismissal doors (`on_key_release` for the key, `hud_release_on_mods` for the
    /// modifier) so the HUD is a true momentary hold — gone the instant the chord lifts.
    pub(in crate::app) fn clear_hud(&mut self) {
        crate::hud::set_held(false);
        self.hud_key = None;
        self.hud_mods = ModifiersState::empty();
        self.sync_view(false);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// Feed ONE stimulus to the HOLD-⌘ SHORTCUT PEEK machine and apply its side
    /// effects. The pure [`crate::peek::PeekArm::next`] decides the next state; this owns
    /// the App-side consequences — stamping the single `WaitUntil` deadline on the
    /// `Idle → Pending` edge, flipping the process-global open/closed, and re-syncing +
    /// redrawing when the card appears or vanishes. THE ONE DOOR every peek transition
    /// routes through (a modifier change, a joined key, a mouse press, a blur, the hold
    /// timer), so the arm state, the global, and the redraw can never drift. An inert
    /// stimulus (no state change — the common case: typing without ⌘, a stray timer) is a
    /// cheap early return with no redraw.
    pub(in crate::app) fn feed_peek(&mut self, stim: crate::peek::PeekStimulus) {
        let before = self.peek_arm;
        let after = before.next(stim);
        if after == before {
            return;
        }
        self.peek_arm = after;
        use crate::peek::PeekArm::*;
        match after {
            // Idle → Pending: bare ⌘ went down alone — start the hold timer (consumed by
            // the single `WaitUntil` in `about_to_wait`; no card yet).
            Pending => self.peek_armed_at = Some(Instant::now()),
            // Pending → Open: the hold completed — summon the card + redraw.
            Open => {
                self.peek_armed_at = None;
                crate::peek::set_open(true);
                self.sync_view(false);
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            // Any cancellation (broken hold / joined key / click / blur): disarm + close.
            // Only re-sync/redraw when the card was actually up, so a pending-cancel
            // (never drawn) costs no repaint.
            Idle => {
                let was_open = crate::peek::peek_open();
                self.peek_armed_at = None;
                crate::peek::set_open(false);
                if was_open {
                    self.sync_view(false);
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.window.request_redraw();
                    }
                }
            }
        }
    }

    /// WHICH-KEY prefix sync, run right after every `keymap.resolve`. Reads the
    /// keymap's post-resolve prefix state: MID-PREFIX (a `C-x` was just pressed,
    /// awaiting its second key) ARMS the pause timer by stamping `prefix_pending_at`;
    /// any other outcome (the prefix resolved to a command, or aborted via `Esc`/`C-g`)
    /// DISMISSES the panel + disarms. The timer itself (the ~500ms wake) lives in
    /// `about_to_wait` and only fires while `prefix_pending_at` is `Some` — so the
    /// summon costs nothing until a prefix actually hangs (DESIGN §6).
    pub(in crate::app) fn sync_whichkey_prefix(&mut self) {
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
    pub(in crate::app) fn summon_whichkey(&mut self) {
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
    pub(in crate::app) fn dismiss_whichkey(&mut self) {
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
    pub(in crate::app) fn handle_search_key(
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
                // replacing it), Cmd-Option-F reveals+toggles the replace field, Cmd-R
                // focuses the replace field (the headline door — a fresh Cmd-R opened
                // the panel on the find field), and Cmd-G / Cmd-Shift-G MIRROR Cmd-F /
                // Cmd-Shift-F's plain step (P2 — the deeper macOS find-next/previous
                // idiom, alongside Cmd-F's own in-panel step; Cmd-Option-G has no
                // Option-toggle counterpart, so it is simply consumed, no-op). Other
                // Super combos are consumed.
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
                    } else if c.eq_ignore_ascii_case(&'g') && !alt {
                        if shift {
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
                    // ACCEPT: remember the query (P2) before closing, so a
                    // LATER bare Cmd-G re-finds it.
                    if let Some(st) = self.search.as_ref() {
                        crate::search::set_last_query(st.query());
                    }
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
    pub(in crate::app) fn search_step(&mut self, dir: Direction) {
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
    pub(in crate::app) fn search_jump_to_current(&mut self) {
        if let Some(st) = self.search.as_ref() {
            if let Some(m) = st.current_match() {
                self.buffer.set_cursor(m.start);
            }
        }
    }

    /// Esc / C-g: restore the cursor to where search began and close the panel.
    /// REMEMBERS the query first (P2) — a non-empty abandoned search still
    /// survives the close so a later bare Cmd-G re-finds it.
    pub(in crate::app) fn search_abort(&mut self) {
        if let Some(st) = self.search.as_ref() {
            crate::search::set_last_query(st.query());
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
    pub(in crate::app) fn search_replace_current(&mut self) {
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
    pub(in crate::app) fn search_replace_all(&mut self) {
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
    pub(in crate::app) fn set_zoom(&mut self, z: f32) {
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
    pub(in crate::app) fn mark_zoom_dirty(&mut self) {
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
    pub(in crate::app) fn scroll_page(&mut self, dir: isize) -> bool {
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

    /// Handle a platform IME event (Japanese/CJK composition lifecycle).
    ///
    /// * `Enabled`/`Disabled` track whether the IME is active; a Disable clears
    ///   any dangling preedit so a stale composition never lingers.
    /// * `Preedit(text, _)` stores the in-progress composition as a transient
    ///   overlay (rendered underlined at the caret) WITHOUT touching the buffer.
    ///   An empty preedit clears it.
    /// * `Commit(text)` inserts the finalized text (the chosen kanji/kana) into
    ///   the ropey buffer at the cursor and clears the preedit.
    pub(in crate::app) fn handle_ime(&mut self, ime: Ime) {
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

    /// `WindowEvent::ModifiersChanged`: track the live modifier state, and let a
    /// dropped SUMMONING modifier break a held stats-HUD chord (e.g. lifting Cmd
    /// or Option of Option-Cmd-I), covering the macOS case where the character key-UP is never
    /// delivered.
    pub(in crate::app) fn on_modifiers_changed(&mut self, m: Modifiers) {
        self.mods = m;
        self.hud_release_on_mods(m.state());
        // HOLD-⌘ SHORTCUT PEEK: bare ⌘ ALONE arms the hold; any other modifier state
        // (⌘+Shift, a released ⌘, …) breaks it — so a pending peek cancels and an open
        // one closes. Feeding `SuperBroken` while Idle is inert, so ordinary typing
        // (no ⌘) never churns.
        let stim = if peek_is_bare_super(m.state()) {
            crate::peek::PeekStimulus::SuperAlone
        } else {
            crate::peek::PeekStimulus::SuperBroken
        };
        self.feed_peek(stim);
    }

    /// `WindowEvent::Ime`: hand the composition-lifecycle event to `handle_ime`
    /// then re-sync + redraw.
    pub(in crate::app) fn on_ime(&mut self, ime: Ime) {
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
    pub(in crate::app) fn on_keyboard_input(
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
        // WEB/LINUX MENU BAR: a real (non-modifier) key press dismisses an open
        // dropdown — the awl bar's dropdown is mouse-driven (no keyboard nav in v1), so
        // any key closes it (and is otherwise processed normally, exactly like clicking
        // away). Inert unless a dropdown is open, so an ordinary keystroke is a no-op.
        if crate::menubar::open_menu().is_some() {
            crate::menubar::set_open(None);
        }
        // HOLD-⌘ SHORTCUT PEEK: a real (non-modifier) key press means a chord is forming
        // (⌘S, ⌘⇧P's letter, Cmd-I, …), so cancel a pending peek / close an open one
        // BEFORE it can flicker — THE CRUX of the cancellation contract. Inert unless a
        // peek is actually pending/open, so an ordinary keystroke is a no-op here.
        self.feed_peek(crate::peek::PeekStimulus::KeyJoined);
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
        // composed char. Since the identity round retired the built-in Option-letter
        // layer, `is_meta_chord` is true ONLY for a key a config `[keys]` Meta rebind
        // reclaims — so when ALT is held we un-compose (`key_without_modifiers`) ONLY
        // for such a configured chord; otherwise we keep the composed `logical_key` so
        // Option-accent INPUT (Option-e -> é, Option-n -> ñ) types as text. The
        // headless `--keys` replay already sends the un-composed key + ALT, so this
        // branch is exercised only live (its behaviour with a real composing keyboard
        // needs human confirmation).
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
        // CHORD door: a keyboard chord is the FAST, learned path the usage ledger
        // graduates on (see `crate::stats::Door`).
        let exited = self.apply(action, shift, event_loop, crate::stats::Door::Chord);
        if exited {
            return;
        }
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

}
