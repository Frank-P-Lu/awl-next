//! THE APPLY BRIDGE: [`App::apply`] resolves an [`Action`] through the shared
//! [`actions::apply_core`] seam (so live editing and headless `--keys` replay
//! behave identically) and carries out the live-only side effects the pure core
//! can't reach — the GPU-measured page scroll, the system-clipboard mirror, the
//! render-toggle window work, theme re-tint, sticky-pref writes. Plus the
//! debounced spell re-scan. Lifted out of `app.rs` verbatim.

use super::*;

impl App {
    /// Recompute spell spans against the current buffer text (called from
    /// about_to_wait once the debounce elapses), then refresh the view.
    pub(super) fn run_spellcheck_now(&mut self) {
        if let Some(spell) = self.spell.as_ref() {
            let text = self.buffer.text();
            self.spell_cache = spell.misspellings(&text);
            self.spell_checked_version = Some(self.buffer.version());
        }
        self.spell_dirty_at = None;
        self.sync_view(false);
    }

    /// Flip the WRITING-NITS highlighter (the "Writing nits" palette command),
    /// persist the new state as a sticky pref, and repaint. Render-only: the buffer
    /// is untouched and the nit underlines are rebuilt from the global each `prepare`,
    /// so a `sync_view` + redraw is all the live App owes the flip. Mirrors the
    /// page/caret render-toggle side effects, but confined here so the toggle needs
    /// no keymap Action of its own.
    pub(super) fn toggle_writing_nits(&mut self) {
        let on = crate::nits::toggle();
        eprintln!("writing nits: {}", if on { "on" } else { "off" });
        self.persist_pref("writing_nits", if on { "true" } else { "false" });
        self.sync_view(false);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    // MIRROR-ON-COPY/KILL. Call AFTER a buffer mutation that may have changed
    // the kill ring top. Writes to the OS clipboard only when the value is
    // non-empty AND differs from what we last wrote (avoids feedback loops and
    // redundant writes; an unchanged kill — e.g. a no-op copy or a selection
    // delete that didn't fill the kill ring — writes nothing).
    //
    // WAYLAND NOTE: on a Wayland compositor (e.g. Hyprland/Omarchy) the write
    // succeeds only if awl holds a clipboard-capable seat; arboard keeps the
    // single App-lifetime Clipboard alive to retain ownership. Errors here are
    // swallowed (graceful degradation) — never panic on a clipboard write.
    pub(super) fn sync_kill_to_clipboard(&mut self) {
        let Some(clip) = self.clipboard.as_mut() else {
            return;
        };
        let killed = self.buffer.kill_buffer();
        if killed.is_empty() {
            return; // never clobber the OS clipboard with an empty kill
        }
        if self.clipboard_last_written.as_deref() == Some(killed) {
            return; // we already wrote exactly this; skip redundant write
        }
        let owned = killed.to_string(); // drop the &self.buffer borrow
        match clip.set_text(owned.clone()) {
            Ok(()) => self.clipboard_last_written = Some(owned),
            Err(_) => {} // graceful degradation: ignore set errors quietly
        }
    }

    // PREFER-EXTERNAL-ON-YANK. Call BEFORE buffer.yank(). If the OS clipboard
    // holds text that differs from what we last wrote/read, the user copied in
    // another app: load it into the kill ring so the yank uses it. Empty/Err
    // reads or an unchanged value keep the internal kill ring untouched.
    pub(super) fn refresh_kill_from_clipboard(&mut self) {
        let Some(clip) = self.clipboard.as_mut() else {
            return;
        };
        let text = match clip.get_text() {
            Ok(t) => t,
            Err(_) => return, // empty / non-text / unsupported: keep internal
        };
        if text.is_empty() {
            return; // empty external clipboard does not override internal kill
        }
        if self.clipboard_last_written.as_deref() == Some(text.as_str()) {
            return; // it's our own value; nothing external changed
        }
        self.buffer.set_kill(&text);
        self.clipboard_last_written = Some(text);
    }

    /// Apply a resolved action; returns true if the app should exit. `shift` is
    /// whether the Shift modifier was held (so a motion extends the selection,
    /// Shift+Arrow style); the app passes the live modifier state.
    pub(super) fn apply(&mut self, action: Action, shift: bool, event_loop: &ActiveEventLoop) -> bool {
        // The buffer/zoom/search core is shared with the headless `--keys`
        // replay via `actions::apply_core`, so live editing and captured replay
        // behave identically. Everything that core can't reach — the system
        // clipboard mirroring and the GPU-measured page size — stays here.
        //
        // The render-only TOGGLES (caret look / page mode / focus mode) flip a
        // process-global. That flip now lives in `apply_core` (the shared seam),
        // so BOTH this live path and the headless `--keys` replay flow through one
        // place; what the core can't reach — the GPU re-wrap on a page-mode change,
        // the view resync, the stderr log — runs as a POST-`apply_core` side effect
        // below (keyed off `matches!(action, …)`, like the Save/clipboard steps),
        // not as an interception that bypasses the core.
        //
        // PageScrollDown/PageScrollUp still intercept here: they need a screenful
        // measured from the live viewport, and the core's `scroll_page_lines` is
        // only the logical-line fallback — so we override those two with the
        // GPU-aware `scroll_page` below.
        // PgDn/PgUp page the BUFFER via the GPU-measured viewport — but ONLY when no
        // overlay is open. While a picker is summoned they PAGE its selection instead,
        // so fall through to `apply_core`'s shared overlay intercept in that case.
        if let Some(handled) = self.page_scroll_intercept(&action) {
            return handled;
        }

        // Yank pulls any newer FOREIGN clipboard text into the on-buffer kill
        // ring BEFORE the core yanks, so an external copy wins (live behavior).
        if matches!(action, Action::Yank) {
            self.refresh_kill_from_clipboard();
        }

        let mut shift_selecting = self.shift_selecting;
        let mut zoom = self.zoom;
        let mut search = self.search.take();
        let mut overlay = self.overlay.take();
        // Whether the Theme picker is open BEFORE the core runs: live preview
        // (move / filter) mutates the process-global active theme while it stays
        // open, so the GPU pipelines must be re-tinted even with no accept.
        let theme_overlay_before = overlay
            .as_ref()
            .map(|o| o.kind == crate::overlay::OverlayKind::Theme)
            .unwrap_or(false);
        // The config `[keys]` (cloned to dodge the &mut self.buffer borrow below) so
        // the command palette can show each command's EFFECTIVE binding.
        let config_keys = self.config.keys.clone();
        // Pre-build the overlay-open closure WITHOUT borrowing `self` (the buffer
        // is borrowed mutably below): clone the small bits `make_overlay` needs.
        // LAST-EDITED RECENCY: for the NOTES root, re-order the go-to corpus
        // most-recently-edited first and attach a relative "last edited" label per
        // file. Live-only (real mtime read here); the headless path passes `None`
        // so the capture stays byte-stable. Other roots keep name order (and skip
        // the per-file mtime stat) so a large repo's picker stays fast.
        let recency_now = if self.root == self.notes_root {
            Some(crate::clock::system_now())
        } else {
            None
        };
        let (goto_corpus, goto_times) =
            crate::index::with_recency(&self.root, self.file_index.clone(), recency_now);
        let goto_open: Vec<usize> = {
            let active_rel = self.file.as_ref().and_then(|p| {
                p.strip_prefix(&self.root)
                    .ok()
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
            });
            goto_corpus
                .iter()
                .enumerate()
                .filter(|(_, c)| Some(*c) == active_rel.as_ref())
                .map(|(i, _)| i)
                .collect()
        };
        let goto_recent: Vec<usize> = goto_corpus
            .iter()
            .enumerate()
            .filter(|(_, c)| self.opened.iter().any(|o| o == *c))
            .map(|(i, _)| i)
            .collect();
        // OUTLINE picker corpus: the CURRENT buffer's markdown headings (each title
        // indented by depth, paired with its line). Read here, BEFORE the closure /
        // the &mut self.buffer borrow below. A non-markdown buffer (or one with no
        // headings) yields an empty list, so the summon becomes a quiet no-op.
        // GATED on the action (like `spell_target` below): parsing the whole document
        // (`headings` allocates the full text + runs pulldown) is pure waste on every
        // OTHER keystroke — the corpus is only consumed when building the Outline
        // overlay, which only `OpenOutline` does.
        let outline_headings: Vec<(String, usize)> =
            if matches!(action, Action::OpenOutline) && self.buffer.is_markdown() {
                crate::markdown::headings(&self.buffer.text())
                    .into_iter()
                    .map(|h| (h.label(), h.line))
                    .collect()
            } else {
                Vec::new()
            };
        // SPELL picker target: the misspelled word the cursor is ON or ADJACENT to,
        // plus its corrections — resolved HERE, before the &mut self.buffer borrow
        // below, and ONLY when the spell binding actually fired (suggestion
        // generation isn't free). `None` when spell-check is off or the cursor isn't
        // on a flagged word, so the summon becomes a calm no-op.
        let spell_target: Option<(Vec<String>, (usize, usize, usize))> =
            if matches!(action, Action::OpenSpellSuggest) {
                self.spell.as_ref().and_then(|sc| {
                    let (line, col) = self.buffer.cursor_line_col();
                    sc.suggest_at(&self.buffer.text(), line, col).map(|t| {
                        (
                            t.suggestions,
                            (t.misspelling.line, t.misspelling.start_col, t.misspelling.end_col),
                        )
                    })
                })
            } else {
                None
            };
        // The non-navigable builder (Goto / Theme / Command + the buffer-scoped
        // Outline / Spell) lives in `overlay`, fed the caller-gathered inputs: the
        // live recency bits + the outline headings / spell target here, all empty
        // or None in headless except what the replayed buffer itself yields.
        let build_ctx = crate::overlay::BuildCtx {
            goto_corpus,
            goto_open,
            goto_recent,
            goto_times,
            config_keys: &config_keys,
            outline_headings,
            spell_target,
        };
        let mut make_overlay =
            |kind: crate::overlay::OverlayKind| crate::overlay::build(kind, &build_ctx);
        // Browse rebuild hook: list ONE level via the shared `overlay::browse_level`
        // builder. `Browse` (C-x j) walks the active root and shows files + folders;
        // `MoveDest` (C-x m) walks the NOTES root and shows FOLDERS only (you move a
        // note into a folder); `Project` (C-x p) walks the workspace by absolute
        // path. Cloned roots dodge the &mut self.buffer borrow.
        let browse_root = self.root.clone();
        let notes_root = self.notes_root.clone();
        let workspace = self.workspace.clone();
        let mut browse_to = |kind: crate::overlay::OverlayKind, rel: Option<String>| {
            crate::overlay::browse_level(
                kind,
                rel,
                &browse_root,
                &notes_root,
                workspace.as_deref(),
            )
        };
        // The visual-line motion LAYOUT ORACLE: the live GPU pipeline, which owns
        // the shaped wrap geometry. A shared borrow of `self.gpu` (disjoint from the
        // `&mut self.buffer` below), so the same `apply_core` seam sees the SAME
        // geometry headless replay sees through its offscreen pipeline. `None` before
        // the window's GPU exists; motion then falls back to LOGICAL lines.
        let oracle = self
            .gpu
            .as_ref()
            .map(|g| &g.pipeline as &dyn actions::LayoutOracle);
        let mut ctx = actions::ActionCtx {
            buffer: &mut self.buffer,
            shift_selecting: &mut shift_selecting,
            zoom: &mut zoom,
            search: &mut search,
            scroll_page_lines: 1,
            overlay: &mut overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
            oracle,
        };
        let effect = actions::apply_core(&mut ctx, &action, shift);
        self.shift_selecting = shift_selecting;
        // ZoomIn/Out/Reset clamp inside the core; mirror the result back so the
        // next sync picks up the new metrics. A Cmd-zoom action ARMS the debounced
        // sticky-zoom write (the wheel path arms it in `set_zoom`).
        let zoom_changed = self.zoom != zoom;
        self.zoom = zoom;
        if zoom_changed
            && matches!(action, Action::ZoomIn | Action::ZoomOut | Action::ZoomReset)
        {
            self.mark_zoom_dirty();
        }
        self.search = search;
        let _ = make_overlay;
        let _ = browse_to;
        self.overlay = overlay;
        // Carry out the ONE deferred EFFECT the core signalled. The signalling
        // paths are mutually exclusive, so a single match (leaning on
        // exhaustiveness) replaces the former cluster of out-param `if`s.
        let quit = matches!(&effect, actions::Effect::Quit);
        // The Theme picker COMMITTED (Enter) or REVERTED (C-g): the core already
        // set the process-global active theme; remember it so we re-tint below.
        let theme_committed = matches!(
            &effect,
            actions::Effect::OverlayAccept(crate::overlay::OverlayKind::Theme, _)
        );
        match effect {
            // COMMAND PALETTE run-on-Enter: the palette closed itself in the core
            // and returned the chosen command. Re-dispatch it through the NORMAL
            // apply path now that the overlay slot is empty — so an overlay-opening
            // command (Go to file / Switch theme) opens cleanly, ToggleCaretMode/
            // PageScrollDown hit their App-special handling, and a Quit propagates. The
            // action here is always Newline (no clipboard/theme post-step), so
            // returning early is safe.
            actions::Effect::RunAction(act) => {
                // WRITING NITS: the render-only toggle rides the `Ignore` sentinel
                // rather than a keymap Action, so intercept it HERE (the palette is the
                // only producer of RunAction) instead of re-dispatching a no-op. Flip
                // the global, persist the sticky pref, and repaint.
                if crate::commands::is_writing_nits(&act) {
                    self.toggle_writing_nits();
                    return false;
                }
                return self.apply(act, shift, event_loop);
            }
            // C-x b last-buffer toggle (history lives here).
            actions::Effect::LastBuffer => self.last_buffer_toggle(),
            // C-x n new quick note (the jump + buffer swap + notes-root config here).
            actions::Effect::NewNote => self.new_note(),
            // Settings: open the config file into the buffer (create the default
            // first if missing). The palette entry runs this via re-dispatch above.
            actions::Effect::OpenSettings => self.open_settings(),
            // The overlay ACCEPTED (Enter): open the chosen file / switch project /
            // move the note. Browse emits its file picks as Goto, so Goto covers both.
            actions::Effect::OverlayAccept(kind, val) => match kind {
                crate::overlay::OverlayKind::Goto => self.open_rel(&val),
                // C-x p: the explorer accepted an ABSOLUTE directory; make it the
                // active project root (re-resolve project + rebuild index).
                crate::overlay::OverlayKind::Project => self.set_root(PathBuf::from(val)),
                // C-x m: move the current note into the chosen destination folder.
                crate::overlay::OverlayKind::MoveDest => self.move_current_note(&val),
                // The Theme picker COMMITTED (Enter) or REVERTED (C-g): the core
                // already set the process-global active theme to `val`; the re-tint
                // below (flagged by `theme_committed`) handles the GPU/title.
                crate::overlay::OverlayKind::Theme => {}
                // The Caret-style picker COMMITTED (Enter): the core already set the
                // process-global caret look via the live preview, so PERSIST it (phase
                // 1's caret_mode preference) so the choice sticks across launches. A
                // Cancel reverts in the core and signals Effect::None, so it never
                // reaches here — persistence is commit-only, like the theme.
                crate::overlay::OverlayKind::Caret => self.persist_caret_mode(),
                crate::overlay::OverlayKind::Browse => {}
                // The command palette never accepts a value — it runs an Action.
                crate::overlay::OverlayKind::Command => {}
                // Cmd-Shift-O: the outline accepted a heading's LINE; jump there.
                crate::overlay::OverlayKind::Outline => self.jump_to_line(&val),
                // Cmd-`;`: the spell picker performed the replace IN the core (it's a
                // buffer edit), so there is nothing to do here — the post-action sync
                // re-runs spell-check on the new text.
                crate::overlay::OverlayKind::Spell => {}
                // The rebind menu never accepts a value — it commits via RebindCommit.
                crate::overlay::OverlayKind::Keybindings => {}
            },
            // REBIND MENU: persist the captured binding (after a conflict gate) /
            // reset to default, then live-reload + refresh the open menu.
            actions::Effect::RebindCommit { slug, binding, confirmed } => {
                self.rebind_commit(slug, binding, confirmed)
            }
            actions::Effect::RebindReset { slug } => self.rebind_reset(slug),
            // BLOCKED-ACTION RECOIL: the requested action couldn't proceed; queue a
            // caret bump away from the wall for the next sync_view (it applies the
            // impulse after setting the spring target). Buffer/cursor are unchanged.
            actions::Effect::Recoil(dir) => self.caret_recoil = Some(dir),
            // PHASE 2 edit FLINCH: a successful typed char / delete / kill-line; queue
            // the matching caret flinch for the next sync_view (applied after the
            // target is set). The buffer is already mutated by the core.
            actions::Effect::TypeImpact => self.caret_impact = Some(CaretImpact::Type),
            actions::Effect::DeleteSquash => self.caret_impact = Some(CaretImpact::Delete),
            actions::Effect::Gulp => self.caret_impact = Some(CaretImpact::Gulp),
            actions::Effect::Quit | actions::Effect::None => {}
        }
        self.post_apply_effects(&action, theme_overlay_before, theme_committed);

        if quit {
            event_loop.exit();
        }
        quit
    }

    /// The PgDn/PgUp intercept: page the BUFFER via the GPU-measured viewport (a
    /// screenful from the live pipeline, which the core's logical-line
    /// `scroll_page_lines` can't reach) — but ONLY with no overlay open. While a picker
    /// is summoned, return `None` so `apply` falls through to `apply_core`'s overlay
    /// intercept (PgDn/PgUp page the picker SELECTION there). `Some(false)` = handled
    /// (the action never moves the app toward exit); a blocked page recoils the caret.
    fn page_scroll_intercept(&mut self, action: &Action) -> Option<bool> {
        if self.overlay.is_none() {
            match action {
                Action::PageScrollDown => {
                    // RECOIL: a page that can't page further (cursor already at the
                    // bottom) bumps the caret UP, away from the wall.
                    if !self.scroll_page(1) {
                        self.caret_recoil = Some(crate::caret::RecoilDir::Up);
                    }
                    self.buffer.seal_undo_group();
                    if !self.buffer.has_selection() {
                        self.shift_selecting = false;
                    }
                    return Some(false);
                }
                Action::PageScrollUp => {
                    // RECOIL: already at the top -> bump the caret DOWN.
                    if !self.scroll_page(-1) {
                        self.caret_recoil = Some(crate::caret::RecoilDir::Down);
                    }
                    self.buffer.seal_undo_group();
                    if !self.buffer.has_selection() {
                        self.shift_selecting = false;
                    }
                    return Some(false);
                }
                _ => {}
            }
        }
        None
    }

    /// POST-`apply_core` side effects the pure core can't reach: the render-only toggle
    /// window/GPU work (caret look / page mode / focus / fps / HUD), the live config
    /// reload on a Settings save, the theme-picker re-tint + sticky-theme write, the
    /// OS-clipboard mirror after a cut/copy, and the delete-word caret streak. Keyed off
    /// `action` (the Save/clipboard pattern), never an interception that bypasses the
    /// core. Runs straight through with no early return.
    fn post_apply_effects(
        &mut self,
        action: &Action,
        theme_overlay_before: bool,
        theme_committed: bool,
    ) {
        // RENDER-ONLY TOGGLES — post-`apply_core` side effects. The core already
        // flipped the process-global (caret look / page mode / focus mode) on the
        // ONE shared seam, so live and `--keys` replay agree; here we do only the
        // window/GPU work the core can't reach, keyed off the action (the
        // Save/clipboard pattern) instead of intercepting before the core.
        match action {
            // Caret look: the buffer is untouched and the cached glyph masks stay
            // valid (keyed by CacheKey), so the trailing `sync_view` + redraw in the
            // caller suffice — just log the new mode.
            Action::ToggleCaretMode => {
                eprintln!(
                    "caret: {}",
                    match crate::caret::mode() {
                        crate::caret::CaretMode::Block => "Block",
                        crate::caret::CaretMode::Morph => "Morph",
                        crate::caret::CaretMode::Ibeam => "Ibeam",
                    }
                );
                // STICKY CARET: remember the new caret style for next launch.
                self.persist_caret_mode();
            }
            // Page mode: the column width changed, so RE-WRAP — `set_size` reshapes
            // the buffer at the new wrap width (a cursor-only resync is not enough),
            // then `sync_view` re-pushes the view so caret/selection x land on the
            // new column.
            Action::TogglePageMode => {
                eprintln!("page mode: {}", if crate::page::page_on() { "on" } else { "off" });
                if let Some(gpu) = self.gpu.as_mut() {
                    let (w, h) = (gpu.config.width as f32, gpu.config.height as f32);
                    gpu.pipeline.set_size(w, h);
                }
                self.sync_view(true);
                // STICKY PAGE MODE: remember on/off for next launch.
                self.persist_page_mode();
            }
            // Page WIDER / NARROWER: the core stepped the measure, so the column pixel
            // width changed — RE-WRAP (`set_size` reshapes at the new wrap width) and
            // re-push the view, exactly like the page-mode toggle, then remember the new
            // width. Zoom is untouched (the glyphs keep their size; only the column and
            // its char-per-line count change).
            Action::PageWider | Action::PageNarrower => {
                if let Some(gpu) = self.gpu.as_mut() {
                    let (w, h) = (gpu.config.width as f32, gpu.config.height as f32);
                    gpu.pipeline.set_size(w, h);
                }
                self.sync_view(true);
                // STICKY PAGE WIDTH: remember the measure for next launch.
                self.persist_page_width();
            }
            // Focus mode: no re-wrap (the column geometry is unchanged), but the view
            // must be re-pushed so the pipeline recomputes the active unit + kicks the
            // brighten/dim fade.
            Action::CycleFocusMode => {
                eprintln!("focus mode: {}", crate::focus::mode().name());
                self.sync_view(false);
            }
            // DEBUG panel: the core flipped the process-global; here we drive frames
            // continuously while it's ON (the RedrawRequested handler keeps the loop
            // hot while `debug_on`) so the frametime line actually ticks. Reset the EMA
            // clock and request a redraw to kick it. Render-only: no buffer change.
            Action::ToggleDebug => {
                eprintln!("debug: {}", if crate::debug::debug_on() { "on" } else { "off" });
                self.debug_clock = None;
                self.debug_ema_ms = None;
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            // HELD stats HUD summoned: the core set the process-global true; here we
            // just kick a redraw so the panel appears this frame. The RedrawRequested
            // handler keeps the loop hot while it's held (so the session timer ticks),
            // and the matching key RELEASE dismisses it (`on_key_release`). Render-only.
            Action::ShowStatsHud => {
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            _ => {}
        }
        // LIVE CONFIG RELOAD: a Save of the config file (Settings buffer) re-applies
        // the keymap overrides + notes_root/workspace immediately. Other saves are
        // untouched. An invalid config keeps prior values (see `reload_config`).
        if matches!(action, Action::Save)
            && self
                .file
                .as_ref()
                .map(|f| !self.config.path.as_os_str().is_empty() && f == &self.config.path)
                .unwrap_or(false)
        {
            self.reload_config();
        }
        // Re-tint for the THEME picker: a live preview (overlay still open) OR a
        // commit/revert (overlay just closed) changed the active theme, so reskin
        // the baked GPU pipelines and refresh the title to the now-active world.
        if theme_overlay_before || theme_committed {
            let active = crate::theme::active();
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.pipeline.sync_theme();
            }
            self.update_title();
            let _ = active;
        }
        // STICKY THEME write-on-change: persist ONLY on the picker's COMMIT/revert
        // (`theme_committed`), never on a live PREVIEW (`theme_overlay_before` while
        // the picker is still open) — so scrolling through worlds doesn't hammer the
        // disk; the SETTLED choice is what's remembered for next launch.
        if theme_committed {
            self.persist_theme();
        }

        // After a cut/copy push the on-buffer kill ring out to the OS clipboard
        // (the one thing the pure core deliberately skips).
        match action {
            Action::DeleteWordBackward
            | Action::KillLine
            | Action::CopyRegion
            | Action::KillRegion => self.sync_kill_to_clipboard(),
            _ => {}
        }

        // Delete-word-backward moves the caret a WHOLE WORD to the left while the
        // text to its right collapses to meet it. Let that caret glide streak like
        // the matching navigation move (M-b) so the removal and the motion read as
        // ONE concurrent gesture, instead of the word vanishing and THEN a bare
        // block sliding. Other edits (typing, Backspace, paste) stay plain slides:
        // Backspace moves only one cell (no visible streak) and kill-line doesn't
        // move the caret at all, so neither shares this defect. The next sync_view
        // consumes this flag.
        if matches!(action, Action::DeleteWordBackward) {
            self.caret_edit_streaks = true;
        }

        // TYPING IMPACT / DELETION SQUASH / KILL-LINE GULP are armed in `apply_core`
        // (the shared seam, so `--keys` replay and live agree) as `Effect::TypeImpact`
        // / `DeleteSquash` / `Gulp` and queued into `self.caret_impact` above. They
        // fire in EVERY caret look — the old I-beam-only typing kick was folded into
        // the universal `type_impact` (squash-pop + a velocity back-kick) — and are
        // mutually exclusive with the blocked-action recoil (a no-op edit recoils, a
        // successful one flinches), so no precedence gate is needed here.
    }
}
