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
            // Idle → Pending: the convention's bare arming modifier went down alone —
            // start the hold timer (consumed by the single `WaitUntil` in
            // `about_to_wait`; no card yet).
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
    /// is `Some`). A thin delegate to the ONE renderer-independent interception
    /// seam — [`crate::search::keys::intercept`], shared verbatim with the
    /// headless `--keys` replay's search guard (`main/run.rs`), so the live
    /// panel and a replayed capture cannot drift. The seam consumes EVERY key
    /// (query/replacement typing, Backspace, C-s/C-r/arrow steps, M-c case
    /// toggle, Tab/Cmd-R field moves, Enter accept/replace, Cmd-Enter
    /// replace-all, Esc/C-g abort) and moves the REAL buffer cursor onto the
    /// current match, so the existing amber caret shows it for free. The
    /// returned recoil is the one LIVE-only consequence — a boundary step's
    /// failing-I-search bump — armed here on the visual caret.
    pub(in crate::app) fn handle_search_key(
        &mut self,
        logical: &Key,
        mods: &Modifiers,
        _event_loop: &ActiveEventLoop,
    ) {
        if let Some(dir) = crate::search::keys::intercept(
            &mut self.search,
            &mut self.buffer,
            logical,
            mods.state(),
        ) {
            self.caret_recoil = Some(dir);
        }
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
        self.zoom_reflow.queue();
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
        // HOLD-⌘ SHORTCUT PEEK: the ACTIVE CONVENTION's bare arming modifier ALONE
        // arms the hold (`peek::is_bare_arming_modifier` / `peek::arming_modifier` — ⌘
        // on Mac, Ctrl on Linux, the ONE convention→modifier owner); any other modifier
        // state (that modifier plus another, a release, or the OTHER platform's
        // modifier — bare Super is now inert under Linux convention, since the
        // compositor owns it) breaks it — so a pending peek cancels and an open one
        // closes. Feeding `ArmBroken` while Idle is inert, so ordinary typing (no
        // arming modifier) never churns.
        let convention = crate::convention::Convention::current();
        let stim = if crate::peek::is_bare_arming_modifier(m.state(), convention) {
            crate::peek::PeekStimulus::ArmAlone
        } else {
            crate::peek::PeekStimulus::ArmBroken
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
        // (⌘S, ⌘⇧P's letter, Cmd-I, … on Mac; C-f, C-s, … on Linux, where the SAME
        // arming modifier also carries the emacs nav layer), so cancel a pending peek /
        // close an open one BEFORE it can flicker — THE CRUX of the cancellation
        // contract. Inert unless a peek is actually pending/open, so an ordinary
        // keystroke is a no-op here.
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
        let defer_zoom_sync = matches!(action, Action::ZoomIn | Action::ZoomOut | Action::ZoomReset);
        let exited = self.apply(action, shift, event_loop, crate::stats::Door::Chord);
        if exited {
            return;
        }
        if !defer_zoom_sync {
            self.sync_view(true);
        }
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

}
