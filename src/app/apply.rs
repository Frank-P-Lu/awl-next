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
            // The ONE spell-scope owner: a CODE buffer checks only its
            // prose-comment + string spans (identifiers never squiggle); a prose
            // buffer takes the unscoped path byte-identically.
            self.spell_cache = spell.misspellings_for(&text, self.buffer.syntax_lang());
            self.spell_checked_version = Some(self.buffer.version());
        }
        self.spell_dirty_at = None;
        self.sync_view(false);
    }

    /// THEME-PICKER LIVE PREVIEW re-tint: apply the newly-active world's COLORS
    /// instantly (`sync_theme_colors`, O(1) pipeline re-tints) and DEFER the font
    /// reshape until the selection settles, by (re)stamping the
    /// `THEME_FONT_DEBOUNCE` deadline consumed in `about_to_wait`. The theme-burst
    /// profile showed the font half — a full-document reshape plus the next
    /// frame's new-face atlas rasterization — dominating every preview step, so
    /// arrowing through N worlds now costs N cheap recolors + ONE reshape at rest
    /// instead of N reshape storms. The deferred reshape ALSO re-bakes the per-span
    /// text colors (syntax/markdown/focus are frozen into the AttrsList at shape
    /// time), so a same-FACE world hop that only changes the palette still rides this
    /// same settle-deferral (`needs_theme_reshape` catches it) — the preview stays
    /// O(1) colors-only and the span re-tint lands at rest, not on every arrow.
    /// Landing back on the SAME world (arrowing away and back) cancels the pending
    /// deferral outright (nothing left to restyle). Live-only: the shared headless
    /// replay never routes through here.
    pub(super) fn retint_theme_preview(&mut self) {
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.pipeline.sync_theme_colors();
            self.theme_font_at = if gpu.pipeline.needs_theme_reshape() {
                Some(Instant::now())
            } else {
                None
            };
        }
        self.update_title();
    }

    /// THEME re-tint, SETTLED form: the full synchronous `sync_theme` (colors +
    /// font reshape) plus the title refresh, cancelling any pending deferred
    /// reshape — the commit (Enter) / revert (Esc, C-g, click-away) path, where
    /// the chosen world must apply completely before the picker's absence.
    pub(super) fn retint_theme_now(&mut self) {
        self.theme_font_at = None;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.pipeline.sync_theme();
        }
        self.update_title();
    }

    /// The deferred theme FONT reshape, fired from `about_to_wait` once the
    /// preview selection has rested `THEME_FONT_DEBOUNCE`: reshape the document
    /// into the (long-since color-applied) active world's face, re-push the view,
    /// and draw the frame that pays the one rasterization. A no-op reshape (the
    /// face already matches — e.g. the pending world was re-previewed away and
    /// back) is inherently cheap inside `sync_theme_font`.
    pub(super) fn apply_deferred_theme_font(&mut self) {
        self.theme_font_at = None;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.pipeline.sync_theme_font();
        }
        self.sync_view(false);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
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

    /// PASTE-IMAGE (native, LIVE-only): if the OS clipboard holds an IMAGE rather
    /// than text, save it as a PNG into an `assets/` folder beside the doc (or,
    /// for a no-path scratch buffer, under the data dir) and insert a markdown
    /// image reference at the caret as ONE undoable edit — the Typora/Obsidian
    /// convention. Returns `true` when it HANDLED an image paste (the caller then
    /// SKIPS the normal text yank); `false` when the clipboard held no image or
    /// any step failed gracefully, so the caller falls through to the text paste
    /// unchanged. Mirrors the swallowed-error discipline of the text clipboard
    /// bridge (`sync_kill_to_clipboard`) — NEVER panics on a bad image / a failed
    /// fs write / a mismatched buffer.
    ///
    /// UNDO NOTE (documented): Cmd-Z removes the inserted REF TEXT only; the
    /// written PNG is left on disk as a harmless orphan (like any editor — we do
    /// not track+delete the file on undo). DETERMINISM: the unique filename comes
    /// from PROBING the assets dir (`pasted-1.png`, `pasted-2.png`, …), never a
    /// clock/random; and the whole path lives only on the live App, so a headless
    /// `--screenshot`/`--keys` capture never reaches a real clipboard image.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn try_paste_image(&mut self) -> bool {
        use crate::paste_image;
        let Some(clip) = self.clipboard.as_mut() else {
            return false;
        };
        // No image on the clipboard (the common case: it holds text, or is empty)
        // → let the normal text paste run. `get_image` Errs for text/empty.
        let img = match clip.get_image() {
            Ok(img) => img,
            Err(_) => return false,
        };
        // Encode raw RGBA → PNG bytes; a degenerate / length-mismatched buffer
        // bails to the text path rather than write a broken file.
        let Some(png) = paste_image::encode_rgba_png(img.width, img.height, &img.bytes) else {
            return false;
        };
        let fs = crate::fs::active();
        let data_root = crate::fs::data_root();
        let doc_path = self.buffer.path().map(|p| p.to_path_buf());
        let dir = paste_image::assets_dir(doc_path.as_deref(), &data_root);
        // Make the assets/ folder (idempotent). A failure → fall back to text.
        if fs.create_dir_all(&dir).is_err() {
            return false;
        }
        // Probe the dir for the next free `pasted-N.png` (deterministic — a pure
        // function of the listing, no clock/random).
        let existing: Vec<String> = fs
            .read_dir(&dir)
            .map(|entries| entries.into_iter().map(|e| e.name).collect())
            .unwrap_or_default();
        let filename = paste_image::next_pasted_name(&existing);
        // Write the PNG. A failure → fall back (never leave a partial insert).
        if fs.write(&dir.join(&filename), &png).is_err() {
            return false;
        }
        // Insert the markdown ref at the caret as ONE undoable edit — doc-relative
        // for a saved doc, absolute for a scratch buffer (nothing to be relative to
        // yet). Cmd-Z removes the ref text (the PNG stays, see the undo note above).
        let reference = paste_image::image_ref(doc_path.as_deref(), &data_root, &filename);
        let (_, col) = self.buffer.cursor_line_col();
        let text = paste_image::insert_text(col == 0, &reference);
        let at = self.buffer.cursor_char();
        self.buffer.replace_char_range(at, at, &text);
        // Refresh the view + repaint (self-contained, so ANY `apply` caller — a
        // keypress, the Edit menu's Paste — lands the same).
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
        true
    }

    /// wasm: no native clipboard image path (the browser clipboard is async +
    /// permission-gated and the stub exposes no `get_image`), so paste-image is a
    /// no-op that always falls through to the internal text paste.
    #[cfg(target_arch = "wasm32")]
    pub(super) fn try_paste_image(&mut self) -> bool {
        false
    }

    /// Apply a resolved action; returns true if the app should exit. `shift` is
    /// whether the Shift modifier was held (so a motion extends the selection,
    /// Shift+Arrow style); the app passes the live modifier state.
    pub(super) fn apply(&mut self, action: Action, shift: bool, event_loop: &ActiveEventLoop) -> bool {
        // macOS: About opens the NATIVE standard About panel (the platform
        // convention) rather than the in-app `about.rs` card — for BOTH the
        // App-menu "About Awl" item AND the Cmd-P palette "About" command, since
        // both dispatch through this one seam. Intercept and return BEFORE
        // `apply_core` ever flips the card's process-global, so the in-app card
        // never opens on macOS; every other platform keeps the card exactly as
        // is. (Not `exited` — the app keeps running.)
        #[cfg(target_os = "macos")]
        if matches!(action, Action::About) {
            crate::mac_chrome::show_about_panel();
            return false;
        }

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
            // PASTE-IMAGE first: if the OS clipboard holds an IMAGE (not text),
            // save it as a PNG beside the doc and insert a markdown ref — then
            // we're DONE (skip the text-paste path entirely). A no-image clipboard
            // (or any graceful failure) falls through to the normal text yank.
            // Native + live-only; a byte-identical no-op in the headless capture.
            if self.try_paste_image() {
                return false;
            }
            self.refresh_kill_from_clipboard();
        }

        let mut shift_selecting = self.shift_selecting;
        let mut zoom = self.zoom;
        let mut search = self.search.take();
        let mut overlay = self.overlay.take();
        // CURSOR SHAPE: whether an overlay was open BEFORE this action, so an
        // open/close transition below (summon / accept / cancel — every one of them
        // flows through the one `self.overlay = overlay` reassignment further down)
        // can recompute the OS cursor shape WITHOUT waiting for the next mouse move.
        let overlay_was_open = overlay.is_some();
        // Whether the Theme picker is open BEFORE the core runs: live preview
        // (move / filter) mutates the process-global active theme while it stays
        // open, so the GPU pipelines must be re-tinted even with no accept.
        let theme_overlay_before = overlay
            .as_ref()
            .map(|o| o.kind == crate::overlay::OverlayKind::Theme)
            .unwrap_or(false);
        // Whether the HISTORY timeline is open BEFORE the core runs: its live
        // preview state (the derived document preview + the saved scroll) must be
        // put down the moment the overlay closes, accept or not.
        let history_overlay_before = overlay
            .as_ref()
            .map(|o| o.kind == crate::overlay::OverlayKind::History)
            .unwrap_or(false);
        // The config `[keys]` (cloned to dodge the &mut self.buffer borrow below) so
        // the command palette can show each command's EFFECTIVE binding.
        let config_keys = self.config.keys.clone();
        // Pre-build the overlay-open closure WITHOUT borrowing `self` (the buffer
        // is borrowed mutably below): clone the small bits `make_overlay` needs.
        // GOTO FRESHNESS (queue: "file picker freshness") — RE-SCAN ON EVERY
        // SUMMON: rebuild the file index right as `C-x f` opens, through the
        // `FileSystem` trait (`rescan_file_index`), so a file created on disk
        // since launch (or the last scan) is never missing. No cache TTL, no
        // watcher — a summoned overlay is transient and the walk is disk-cheap
        // for a real project tree. Gated on the action like outline/spell/
        // history below: walking the tree on every OTHER keystroke would be
        // needless disk I/O.
        if matches!(action, Action::OpenGoto) {
            self.rescan_file_index();
        }
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
        // RECENTLY-OPENED FILES → go-to corpus indices, IN MRU ORDER (most-recent
        // first). The persisted MRU (`self.recent_files`) holds ABSOLUTE paths; keep
        // only those under the ACTIVE root and map each to its corpus row. This feeds
        // BOTH the "recently-opened" ranking tier AND the Recent LENS (which shows
        // ONLY these rows, in exactly this order — see `OverlayState::refilter`'s MRU
        // tiebreak + `index::goto_bucket`). Live-only: `recent_files` is empty in the
        // headless capture path, so Recent degrades to the empty state there.
        let goto_recent: Vec<usize> = self
            .recent_files
            .iter()
            .filter_map(|abs| {
                abs.strip_prefix(&self.root)
                    .ok()
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
            })
            .filter_map(|rel| goto_corpus.iter().position(|c| *c == rel))
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
                    sc.suggest_at(&self.buffer.text(), line, col, self.buffer.syntax_lang()).map(|t| {
                        (
                            t.suggestions,
                            (t.misspelling.line, t.misspelling.start_col, t.misspelling.end_col),
                        )
                    })
                })
            } else {
                None
            };
        // HISTORY TIMELINE rows: the current file's versions (newest-first), each
        // answering WHEN + WHICH with a "+N −M" changed-count vs the CURRENT buffer.
        // Gathered HERE (before the &mut self.buffer borrow) and ONLY when the History
        // binding fired — reading + line-diffing the store is pure waste on every
        // other keystroke. The history key derivation lives in ONE place
        // (`history::source_path`): buffer path, else `self.file`, else the persistent
        // scratch's own stash path — so the no-path scratch has a timeline too; only
        // an unnamed note has none (the picker then shows "no history yet"). `now`
        // stamps the relative labels; History is an explicitly-summoned, non-default
        // overlay, so this clock read never touches a default capture.
        let history_entries: Vec<crate::history::TimelineRow> =
            if matches!(action, Action::OpenHistory) {
                match crate::history::source_path(
                    self.buffer.path(),
                    self.file.as_deref(),
                    self.buffer.is_note(),
                ) {
                    Some(path) => crate::history::timeline_rows(
                        &path,
                        &self.buffer.text(),
                        crate::history::now_millis(),
                    ),
                    None => Vec::new(),
                }
            } else {
                Vec::new()
            };
        // The non-navigable builder (Goto / Theme / Command + the buffer-scoped
        // Outline / Spell / History) lives in `overlay`, fed the caller-gathered
        // inputs: the live recency bits + the outline headings / spell target /
        // history rows here, all empty or None in headless except what the replayed
        // buffer + store yield.
        let build_ctx = crate::overlay::BuildCtx {
            goto_corpus,
            goto_open,
            goto_recent,
            goto_times,
            config_keys: &config_keys,
            outline_headings,
            spell_target,
            history_entries,
            // LIVE reference clocks for History's Session / Today lenses.
            history_now: Some(crate::history::now_millis()),
            history_session_start: crate::history::session_epoch_ms(),
            // RECENT PROJECT ROOTS (newest-first, persisted MRU) for the Recent
            // Projects picker. Live-only; the headless path passes an empty list.
            recent_projects: self
                .recent_projects
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            // SETTINGS MENU value cells: the config/project-derived pieces gathered
            // from the live App's config + active root + zoom (the process-global
            // settings are read live inside the readout). Cheap; unused unless the
            // Settings overlay is the one being summoned.
            settings_values: crate::settings::SettingsValues::gather(
                &self.config,
                &self.root,
                self.zoom,
            ),
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
        // The recent-PROJECTS MRU (absolute paths, newest-first) for the Project
        // navigator's Recent lens — captured as strings so the navigator can mark
        // the folders you've switched to. Empty in the headless replay.
        let recent_projects: Vec<String> =
            self.recent_projects.iter().map(|p| p.display().to_string()).collect();
        let mut browse_to = |kind: crate::overlay::OverlayKind, rel: Option<String>| {
            crate::overlay::browse_level(
                kind,
                rel,
                &browse_root,
                &notes_root,
                workspace.as_deref(),
                &recent_projects,
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
        // CURSOR SHAPE: the overlay just opened or closed WITHOUT any mouse motion (a
        // keyboard summon/accept/cancel, or a click routed back through `apply` from
        // `overlay_click`) — recompute now rather than waiting for the next
        // `CursorMoved`. A no-op call while the OS pointer is hidden (the common case
        // for a keyboard-driven open) — see `sync_cursor_icon`'s hidden-pointer gate.
        if self.overlay.is_some() != overlay_was_open {
            self.sync_cursor_icon();
        }
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
        // The HISTORY timeline ACCEPTED (Enter on a real row): the restore is about
        // to land, so the saved scroll is discarded rather than restored below.
        let history_accepted = matches!(
            &effect,
            actions::Effect::OverlayAccept(crate::overlay::OverlayKind::History, _)
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
                // Feed the command palette's Recent lens: record the RUN command in the
                // in-memory MRU. LIVE-ONLY (this handler is the App's, never the headless
                // replay), so a capture never populates it — Recent stays inert there.
                crate::commands::record_recent(&act);
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
                // active project root (re-resolve project + rebuild index), then
                // PERSIST it as the STICKY PROJECT ROOT — a plain relaunch (no file
                // argument, no --root) reopens this same project. A quick-note jump
                // (C-x n, which also calls `set_root`) deliberately does NOT persist
                // here — only a genuine switch-project counts as "the project".
                crate::overlay::OverlayKind::Project => {
                    self.switch_project(PathBuf::from(val));
                }
                // The Recent Projects picker accepted a remembered root's ABSOLUTE
                // path: switch to it exactly like the Project picker (set_root +
                // persist + push-to-front of the MRU), through the ONE owner.
                crate::overlay::OverlayKind::RecentProjects => {
                    self.switch_project(PathBuf::from(val));
                }
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
                // The Dictionary picker COMMITTED (Enter): the core already set the
                // process-global active variant (there is NO live preview here, unlike
                // Theme/Caret — see `overlay.rs`'s Dictionary doc), so reconstruct the
                // App's `SpellChecker` for the new variant (the one real per-switch
                // cost) + persist the sticky pref, mirroring `persist_caret_mode`. A
                // Cancel never reaches here (nothing was set to revert).
                crate::overlay::OverlayKind::Dictionary => {
                    self.set_dictionary(crate::spell::active_variant())
                }
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
                // Cmd-Shift-H: the history timeline accepted a version's restore ID —
                // load that version and replace the buffer with it (an undoable edit).
                crate::overlay::OverlayKind::History => self.restore_history(&val),
                // Settings menu never emits an OverlayAccept(Settings): Enter on a
                // row signals SettingToggle (toggle), swaps to a sub-picker (picker /
                // submenu), or emits OpenSettings (edit-as-text) — handled below /
                // via their own kinds. This arm stays for match exhaustiveness only.
                crate::overlay::OverlayKind::Settings => {}
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
            // Edit FLINCH: a successful typed char / delete / kill-line / Enter; queue
            // the matching caret flinch for the next sync_view (applied after the
            // target is set). The buffer is already mutated by the core.
            actions::Effect::TypeImpact => self.caret_impact = Some(CaretImpact::Type),
            actions::Effect::DeleteSquash => self.caret_impact = Some(CaretImpact::Delete),
            actions::Effect::Gulp => self.caret_impact = Some(CaretImpact::Gulp),
            // PHASE 3 — ENTER JUICE: a successful Newline lands a caret-level
            // touchdown squash (queued the same way as the other edit flinches).
            actions::Effect::LineLand => self.caret_impact = Some(CaretImpact::Land),
            // COPY PULSE: a successful M-w/Cmd-C copy of a non-empty selection.
            // Queued the same way as the other edit flinches; unlike them the
            // pipeline call ALSO brightens the selection quad's own tint (the
            // caret kick alone would be "obvious" but not "understated" — the
            // selection is the thing that was actually acted on).
            actions::Effect::CopyPulse => self.caret_impact = Some(CaretImpact::Copy),
            // SETTINGS MENU toggle: flip the sticky boolean live + persist + refresh
            // the still-open menu's value cell (the menu stays up — see
            // `settings_accept`). The overlay is already back in `self.overlay`.
            actions::Effect::SettingToggle { key } => self.setting_toggle(&key),
            // SETTINGS MENU inline VALUE commit: parse + clamp the typed value, apply
            // it live (page measure / zoom), persist the named key, refresh the cell.
            actions::Effect::SettingValueCommit { key, value } => {
                self.setting_value_commit(&key, &value)
            }
            // SETTINGS MENU path pick: write the named folder key (and re-scope the
            // project for `project_root`), then refresh the re-summoned menu's cell.
            actions::Effect::SettingPathPick { key, path } => self.setting_path_pick(&key, &path),
            // C-x #: the core already saved; notify any daemon `--wait` client
            // waiting on this buffer (native-only — no daemon on wasm) and switch
            // to the previously-open buffer (the LastBuffer swap).
            actions::Effect::FinishBuffer => self.finish_buffer(),
            // "Keep This Version": THE CONSCIOUS MARK — pin the current buffer as a
            // prune-exempt local-history snapshot (the store owns the git/off gates).
            actions::Effect::KeepVersion => self.keep_version(),
            // C-c C-o: open the markdown link under the caret in the OS default
            // browser (a user-initiated handoff — see `App::follow_link`).
            actions::Effect::FollowLink(url) => self.follow_link(&url),
            actions::Effect::Quit | actions::Effect::None => {}
        }
        // HISTORY TIMELINE live-preview lifecycle, mirroring the theme block below:
        // opening the timeline saves the document scroll (a shorter previewed
        // version can destructively clamp it); the moment the overlay is GONE the
        // preview is put down — restore the scroll on a close-without-restore
        // (Esc / click-away / no-op Enter → "back to now exactly"), just discard it
        // on a real accept (the restored version owns the viewport now).
        if matches!(action, Action::OpenHistory)
            && self
                .overlay
                .as_ref()
                .map(|o| o.kind == crate::overlay::OverlayKind::History)
                .unwrap_or(false)
        {
            self.history_scroll_before = Some(self.scroll_lines);
        }
        if history_overlay_before && self.overlay.is_none() {
            self.history_overlay_closed(history_accepted);
        }
        self.post_apply_effects(&action, theme_overlay_before, theme_committed);

        if quit {
            event_loop.exit();
        }
        quit
    }

    /// The HISTORY timeline just CLOSED: drop the live preview (the next sync
    /// pushes the buffer's own text again) and settle the viewport — `accepted`
    /// false (Esc / click-away / empty-row Enter) RESTORES the scroll saved at
    /// open, so "back to now" is exact even after a shorter version's max-scroll
    /// clamp; `accepted` true (a real Enter-restore) just discards it (the
    /// undoable restore owns the viewport). Extracted so the close contract is
    /// unit-testable without an event loop.
    pub(super) fn history_overlay_closed(&mut self, accepted: bool) {
        if accepted {
            self.history_scroll_before = None;
        } else if let Some(s) = self.history_scroll_before.take() {
            self.scroll_lines = s;
        }
        self.history_preview = None;
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

    /// C-c C-o (follow-link-at-point): hand `url` off to the OS default browser.
    /// This is a USER-INITIATED launch — the app spawns the platform opener
    /// (`open` on macOS, `xdg-open` on Linux) or `window.open` on the web — NOT a
    /// network fetch, so awl's zero-network invariant holds (exactly like the
    /// daemon spawning a process, or a shell's `$EDITOR` handoff). LIVE-APP-ONLY:
    /// this method is never reached from the headless `--keys` replay (its
    /// `Effect::FollowLink` arm is a no-op), so a capture never spawns anything.
    /// A spawn failure is logged, never fatal — following a link is best-effort.
    fn follow_link(&self, url: &str) {
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(w) = web_sys::window() {
                let _ = w.open_with_url(url);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            #[cfg(target_os = "macos")]
            let opener = "open";
            #[cfg(all(unix, not(target_os = "macos")))]
            let opener = "xdg-open";
            #[cfg(windows)]
            let opener = "explorer";
            if let Err(e) = std::process::Command::new(opener).arg(url).spawn() {
                eprintln!("follow link: could not open {url:?}: {e}");
            }
        }
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
            // RESET PAGE WIDTH: the core snapped the measure to DEFAULT_MEASURE, so
            // re-wrap + re-push the view exactly like wider/narrower — but CLEAR the
            // sticky override entirely (rather than writing the default back), so a
            // future default change flows through instead of pinning a stale value.
            Action::PageReset => {
                if let Some(gpu) = self.gpu.as_mut() {
                    let (w, h) = (gpu.config.width as f32, gpu.config.height as f32);
                    gpu.pipeline.set_size(w, h);
                }
                self.sync_view(true);
                self.persist_page_reset();
            }
            // Focus mode: no re-wrap (the column geometry is unchanged), but the view
            // must be re-pushed so the pipeline recomputes the active unit + kicks the
            // brighten/dim fade.
            Action::CycleFocusMode => {
                eprintln!("focus mode: {}", crate::focus::mode().name());
                self.sync_view(false);
            }
            // DEBUG panel: the core flipped the process-global; here we just kick ONE
            // redraw so the panel appears (or vanishes) this frame — the pane
            // schedules no frames of its own. Toggled ON, that frame settles into the
            // one still-stamp (see `RedrawRequested`) and goes quiet; toggled OFF,
            // the same handler forgets the measurements so the next enable starts
            // fresh. Render-only: no buffer change.
            Action::ToggleDebug => {
                eprintln!("debug: {}", if crate::debug::debug_on() { "on" } else { "off" });
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            // PERSISTENT MARGIN OUTLINE toggle: the core flipped the process-global;
            // here we PERSIST the sticky pref (write-on-change, like page mode /
            // spellcheck) and kick ONE redraw so the margin outline appears/vanishes
            // this frame. Render-only: no buffer change. The render itself lands next
            // phase; persisting + the redraw are correct now regardless.
            Action::ToggleOutline => {
                let on = crate::outline::outline_on();
                eprintln!("outline: {}", if on { "on" } else { "off" });
                self.persist_pref("outline", if on { "true" } else { "false" });
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            // TYPEWRITER SCROLL toggle: the core flipped the process-global; here we
            // PERSIST the sticky pref (write-on-change, like the outline) and re-sync
            // the view so the caret's row re-pins (or reverts to cursor-follow) THIS
            // frame — `sync_view(true)` re-runs the cursor-follow, which now reads the
            // flipped global. Scroll-only: no buffer change, no reshape.
            Action::ToggleTypewriter => {
                let on = crate::typewriter::typewriter_on();
                eprintln!("typewriter scroll: {}", if on { "on" } else { "off" });
                self.persist_pref("typewriter_scroll", if on { "true" } else { "false" });
                self.sync_view(true);
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            // SPELLCHECK global toggle: the core already flipped the process-global
            // (the shared seam every `misspellings_for`/`suggest_at` call reads), so
            // here we persist the sticky pref and force an IMMEDIATE rescan
            // (`run_spellcheck_now`, which itself `sync_view`s) so existing squiggles
            // vanish/reappear THIS frame rather than waiting for the next edit's
            // debounce. Render-only: no buffer change.
            Action::ToggleSpellcheck => {
                eprintln!("spellcheck: {}", if crate::spell::spellcheck_on() { "on" } else { "off" });
                self.persist_spellcheck();
                self.run_spellcheck_now();
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
        // AUTOMATIC LOCAL SNAPSHOT: an explicit Save (C-x C-s / Cmd-S) just wrote the
        // buffer, so capture a local-history point. The store skips git-managed files
        // (git owns their versioning) and history-off; a scratch buffer with no path
        // is a no-op. Best-effort — a failed snapshot never disrupts the save.
        // A manual save stays a PLAIN save (no special timeline status) — but the
        // AUTOSAVE ENGINE's bookkeeping follows it: the buffer version is now on
        // disk (no redundant idle write), the fresh mtime is the clobber guard's
        // new baseline (a manual save legitimately force-writes over an external
        // change), and any held-write notice is cleared.
        if matches!(action, Action::Save) {
            self.snapshot_after_save();
            if let Some(p) = self.buffer.path().map(|p| p.to_path_buf()) {
                self.disk_mtime = Self::disk_mtime_of(&p);
                self.doc_saved_version = Some(self.buffer.version());
                self.notice = None;
            }
        }
        // Re-tint for the THEME picker: a live preview (overlay still open) OR a
        // commit/revert (overlay just closed) changed the active theme, so reskin
        // the baked GPU pipelines and refresh the title to the now-active world.
        // A PREVIEW re-colors instantly but DEFERS the font reshape until the
        // selection settles (`retint_theme_preview`); a COMMIT/REVERT applies the
        // full switch synchronously and cancels any pending deferral, so Esc can
        // never leave a stray reshape to land after the picker closed.
        if theme_committed || (theme_overlay_before && self.overlay.is_none()) {
            self.retint_theme_now();
        } else if theme_overlay_before {
            self.retint_theme_preview();
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
