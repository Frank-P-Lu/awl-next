//! BUFFER + CONFIG management: opening project-relative files, the last-buffer
//! toggle, quick-note creation / auto-save / live-rename / move, the window
//! title, the active project root, and the sticky-preference + rebind-menu
//! config writes (open Settings, persist theme/zoom/page/caret, live reload,
//! commit/reset a captured binding). Lifted out of `app.rs` verbatim.

use super::*;

impl App {
    /// Settings command: open the config file into the buffer for editing AS TEXT,
    /// creating the commented default first if it does not exist. The palette runs
    /// this; you then edit + C-x C-s to save, which live-reloads (see `reload_config`).
    pub(super) fn open_settings(&mut self) {
        let path = self.config.path.clone();
        if path.as_os_str().is_empty() {
            return; // no resolvable config path (no HOME); nothing to open
        }
        if !crate::fs::active().exists(&path) {
            if let Err(e) = Config::write_default(&path) {
                eprintln!("could not create config {}: {e}", path.display());
                return;
            }
        }
        self.load_path(path);
    }

    /// WRITE-ON-CHANGE for a STICKY PREFERENCE (theme/zoom/page_mode/caret_mode):
    /// persist the settled value to config.toml format-preservingly (reusing the
    /// rebind menu's surgical [`Config::write_pref`] — comments + `[keys]` + the
    /// other prefs survive) and mirror it into the in-memory [`Self::config`] so a
    /// later live reload / conflict check sees the current value. A no-op when there
    /// is no resolvable config path (e.g. no HOME), and silent on a write error (a
    /// failed remember must never disrupt the edit). `value` is the formatted RHS.
    pub(super) fn persist_pref(&mut self, key: &str, value: &str) {
        let path = self.config.path.clone();
        if path.as_os_str().is_empty() {
            return; // no config path (no HOME): nothing to remember
        }
        if let Err(e) = Config::write_pref(&path, key, value) {
            eprintln!("could not persist {key} to {}: {e}", path.display());
            return;
        }
        // Keep the in-memory config in step with the file so it stays the source of
        // truth between explicit reloads.
        match key {
            "theme" => self.config.theme = Some(value.trim_matches('"').to_string()),
            "caret_mode" => self.config.caret_mode = Some(value.trim_matches('"').to_string()),
            "page_mode" => self.config.page_mode = Some(value == "true"),
            "page_width" => self.config.page_width = value.parse().ok(),
            "zoom" => self.config.zoom = value.parse().ok(),
            "writing_nits" => self.config.writing_nits = Some(value == "true"),
            _ => {}
        }
    }

    /// Persist the now-active THEME name (write-on-change after a theme commit/revert).
    pub(super) fn persist_theme(&mut self) {
        let name = crate::theme::active().name;
        self.persist_pref("theme", &format!("\"{name}\""));
    }

    /// Persist the now-active PAGE MODE (write-on-change after a page-mode toggle).
    pub(super) fn persist_page_mode(&mut self) {
        let on = crate::page::page_on();
        self.persist_pref("page_mode", if on { "true" } else { "false" });
    }

    /// Persist the now-active PAGE WIDTH / measure (write-on-change after a Page wider
    /// / Page narrower command). Zoom-independent: remembers the COLUMN width, not the
    /// glyph size (zoom has its own sticky pref).
    pub(super) fn persist_page_width(&mut self) {
        let w = crate::page::measure();
        self.persist_pref("page_width", &w.to_string());
    }

    /// Persist the now-active CARET MODE (write-on-change after a caret-mode change).
    /// Phase 2 relies on this seam to remember the caret style across launches.
    pub(super) fn persist_caret_mode(&mut self) {
        let name = crate::config::caret_mode_name(crate::caret::mode());
        self.persist_pref("caret_mode", &format!("\"{name}\""));
    }

    /// Persist the SETTLED zoom (the DEBOUNCED write-on-change). Called from
    /// `about_to_wait` once the zoom has been quiet for `ZOOM_PERSIST_DEBOUNCE`, so a
    /// rapid Cmd-=/Cmd-- run writes the final value once, not one-per-step. Trims the
    /// float to 3 places so the file stays tidy.
    pub(super) fn persist_zoom_now(&mut self) {
        let z = self.zoom;
        self.persist_pref("zoom", &format!("{z:.3}"));
    }

    /// Live-reload after the config file is SAVED in the editor: re-read it, rebuild
    /// the keymap overrides, and re-fold notes_root/workspace (flag > config >
    /// default, so a CLI flag still wins). A bad chord keeps its default + prints a
    /// note inside `apply_overrides`; nothing here can crash. Folder changes affect
    /// the NEXT C-x n / C-x p; the keymap change is immediate.
    pub(super) fn reload_config(&mut self) {
        let cfg = Config::load(self.config.path.clone());
        self.keymap.apply_overrides(&cfg.keys);
        self.notes_root =
            crate::resolve_notes_root(&self.cli_notes_root.clone().or_else(|| cfg.notes_root.clone()));
        let workspace_opt = self.cli_workspace.clone().or_else(|| cfg.workspace.clone());
        self.workspace = Some(crate::resolve_workspace(&workspace_opt, &self.root));
        self.config = cfg;
    }

    /// REBIND MENU commit: persist a captured `binding` to the command `slug`'s
    /// `[keys]` slot, then live-reload + refresh the open menu. A CONFLICT (the binding
    /// already belongs to another command) is GATED unless the user already accepted
    /// it: the menu moves to its `Confirm` phase (showing what's bound) and waits for a
    /// second Enter, so nothing is written behind the user's back. Otherwise the
    /// binding is merged into the command's existing slots (cap 2, newest first),
    /// written to `config.toml`, and the keymap re-applied immediately.
    pub(super) fn rebind_commit(&mut self, slug: String, binding: String, confirmed: bool) {
        if !confirmed {
            if let Some(other) = crate::commands::binding_conflict(&binding, &slug, &self.config.keys) {
                if let Some(ov) = self.overlay.as_mut() {
                    ov.capture_into_confirm(other.to_string());
                    ov.notice = format!("'{binding}' already bound to {other}");
                }
                return;
            }
        }
        let existing: Vec<String> = self
            .config
            .keys
            .iter()
            .find(|(n, _)| crate::commands::slug(n) == slug)
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let merged = Config::merge_slot(&existing, &binding);
        let path = self.config.path.clone();
        if path.as_os_str().is_empty() {
            self.refresh_rebind_overlay("no config path; not saved".to_string());
            return;
        }
        if let Err(e) = Config::write_binding(&path, &slug, Some(&merged)) {
            eprintln!("rebind: could not write {}: {e}", path.display());
        }
        self.reload_config();
        self.refresh_rebind_overlay(format!("bound {slug} -> {binding}"));
    }

    /// REBIND MENU reset-to-default (Delete on a command): REMOVE the command's
    /// `[keys]` entry, persist, and live-reload so its built-in default applies again.
    pub(super) fn rebind_reset(&mut self, slug: String) {
        let path = self.config.path.clone();
        if !path.as_os_str().is_empty() {
            if let Err(e) = Config::write_binding(&path, &slug, None) {
                eprintln!("rebind: could not reset {}: {e}", path.display());
            }
        }
        self.reload_config();
        self.refresh_rebind_overlay(format!("reset {slug} to default"));
    }

    /// After a rebind commit/reset + live-reload, refresh the still-open Keybindings
    /// menu: close any capture, re-pull the EFFECTIVE binding column from the new
    /// config, and set the status `notice`. A no-op if the menu isn't open.
    pub(super) fn refresh_rebind_overlay(&mut self, notice: String) {
        let keys = self.config.keys.clone();
        if let Some(ov) = self.overlay.as_mut() {
            if ov.kind == crate::overlay::OverlayKind::Keybindings {
                ov.capture = None;
                ov.bindings = crate::commands::effective_bindings(&keys);
                ov.notice = notice;
            }
        }
    }

    /// True while the rebind menu is RECORDING a capture, so the live key handler
    /// routes the next press into the capture (a chord-level interception) rather than
    /// through the keymap. Enter / Esc are excluded by the caller (they finish / abort).
    pub(super) fn capture_recording(&self) -> bool {
        self.overlay
            .as_ref()
            .map(|o| {
                o.kind == crate::overlay::OverlayKind::Keybindings
                    && matches!(
                        o.capture.as_ref().map(|c| c.stage),
                        Some(crate::overlay::CaptureStage::Recording)
                    )
            })
            .unwrap_or(false)
    }

    /// Open a project-relative path: swap in a fresh Buffer, reset cursor/undo,
    /// keep `App.file` + window title in sync, and push the prior file onto the
    /// MRU `opened` stack so `recently-opened` ranking and last-buffer work. The
    /// product model is open/switch only — no file ops — so we just re-read from
    /// disk. `rel` is a root-relative index entry.
    pub(super) fn open_rel(&mut self, rel: &str) {
        let path = crate::index::resolve(&self.root, rel);
        // Push the file we are LEAVING onto the MRU (as a root-relative path).
        if let Some(prev) = &self.file {
            if let Ok(p) = prev.strip_prefix(&self.root) {
                let prev_rel = p.to_string_lossy().replace('\\', "/");
                self.opened.retain(|e| e != &prev_rel);
                self.opened.push(prev_rel);
            }
        }
        self.load_path(path);
    }

    /// C-x b last-buffer toggle: flip between the current and previously-opened
    /// file (a tiny 2-deep history). No-op until a second file has been opened.
    /// The two paths simply swap, so repeated C-x b ping-pongs between them.
    pub(super) fn last_buffer_toggle(&mut self) {
        let Some(prev) = self.prev_file.clone() else {
            return; // nothing opened before; toggle is a quiet no-op
        };
        self.load_path(prev);
    }

    /// Swap in the buffer for `path`: remember the file we are LEAVING as
    /// `prev_file` (the 2-deep last-buffer history), re-read from disk (open/switch
    /// only — no file ops), and reset the per-file render/undo state. Shared by
    /// `open_rel` and the C-x b toggle so both keep the history honest.
    pub(super) fn load_path(&mut self, path: PathBuf) {
        // ROBUST AUTOSAVE: before we drop the current buffer, flush any pending
        // note write so nothing typed in the last debounce window is lost.
        self.flush_note();
        // The file we are leaving becomes the last-buffer target.
        self.prev_file = self.file.take();
        self.buffer = Buffer::from_file(&path);
        self.file = Some(path);
        self.search = None;
        self.preedit.clear();
        // A brand-new buffer starts at version 0; match the synced version so the
        // next sync_view doesn't read the delta as an edit and streak the caret.
        self.caret_synced_version = self.buffer.version();
        self.spell_checked_version = None;
        self.update_title();
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// Jump the cursor to the START of the 0-based `line` (passed as a string —
    /// the outline picker's accept value). Clears any selection, then re-syncs the
    /// view so the heading scrolls into view. A malformed value is ignored.
    pub(super) fn jump_to_line(&mut self, line_str: &str) {
        let Ok(line) = line_str.parse::<usize>() else {
            return;
        };
        let idx = self.buffer.line_col_to_char(line, 0);
        self.buffer.clear_mark();
        self.buffer.set_cursor(idx);
        self.shift_selecting = false;
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// Set the window title from the active file + theme (kept in one place so
    /// open/switch/theme-cycle all agree).
    pub(super) fn update_title(&self) {
        if let Some(gpu) = self.gpu.as_ref() {
            // An UNTITLED quick note (a note buffer with no derived filename yet)
            // shows the "scratch" PLACEHOLDER until its first line names it — so a
            // brand-new C-x n note reads as "scratch" in the window title.
            let title = match &self.file {
                Some(p) => p.display().to_string(),
                None if self.buffer.is_note() => "scratch".to_string(),
                None => "*scratch*".to_string(),
            };
            gpu.window.set_title(&format!(
                "awl - {} [{}]",
                title,
                crate::theme::active().name
            ));
        }
    }

    /// Make `new_root` the ACTIVE project: re-resolve the project, rebuild the
    /// file index, reset the MRU, and re-sync the view. Shared by switch-project
    /// (C-x p) and the new-note jump (C-x n) so both re-scope the go-to list the
    /// same way. No buffer is opened here (that is the caller's concern).
    pub(super) fn set_root(&mut self, new_root: PathBuf) {
        // ROBUST AUTOSAVE: switching project re-scopes (and may precede a buffer
        // swap), so flush a pending note write first — never lose the open note.
        self.flush_note();
        self.root = new_root;
        self.project = crate::project::Project::resolve(&self.root);
        self.file_index = crate::index::build_index(&self.root);
        self.opened.clear();
        self.sync_view(false);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// C-x n: NEW QUICK NOTE in one gesture. Jump the active project to the notes
    /// root AND swap in a fresh empty note buffer; the user starts typing
    /// immediately. The filename is derived (slugified first line) + auto-saved on
    /// the first pause — see [`Self::autosave_note`]. The file we are leaving
    /// becomes the last-buffer (C-x b) target.
    pub(super) fn new_note(&mut self) {
        // The notes root may not exist yet; create it lazily so the project +
        // index resolve and the first save has somewhere to land.
        let _ = crate::fs::active().create_dir_all(&self.notes_root);
        self.set_root(self.notes_root.clone());
        self.prev_file = self.file.take();
        self.buffer.start_note(self.notes_root.clone());
        self.search = None;
        self.preedit.clear();
        self.caret_synced_version = self.buffer.version();
        self.spell_checked_version = None;
        self.autosave_saved_version = None;
        self.autosave_dirty_at = None;
        self.update_title();
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// ROBUST-AUTOSAVE flush: write a pending note save IMMEDIATELY, bypassing the
    /// debounce, so nothing typed in the last quiet window is lost when we switch
    /// away from / close the note. Called before opening another file (`load_path`),
    /// switching project / starting a new note (`set_root`), on focus-out, and on
    /// quit. A truly empty note still writes nothing (no litter); a non-note buffer
    /// or an already-saved version is a no-op.
    pub(super) fn flush_note(&mut self) {
        if self.buffer.is_note() && self.autosave_saved_version != Some(self.buffer.version()) {
            self.autosave_dirty_at = None;
            self.autosave_note();
        }
    }

    /// Auto-save the active NOTE (live only, debounced). The buffer derives its
    /// filename from the first non-empty line on the first save (empty note writes
    /// nothing — no litter); once named, the filename LOCKS. On the first naming
    /// save we sync `App.file` / title / the go-to index so the note is findable.
    pub(super) fn autosave_note(&mut self) {
        self.autosave_saved_version = Some(self.buffer.version());
        if !self.buffer.is_note() {
            return;
        }
        let had_path = self.buffer.path().is_some();
        match self.buffer.save() {
            Ok(()) => {
                if !had_path {
                    if let Some(p) = self.buffer.path() {
                        let p = p.to_path_buf();
                        eprintln!("note: {}", p.display());
                        self.file = Some(p);
                        self.update_title();
                        // Re-scope the go-to index so the new note is jump-able.
                        self.file_index = crate::index::build_index(&self.root);
                    }
                } else {
                    // Already named: the filename LIVE-TRACKS the first line, so a
                    // mid-typing typo fixed later renames the file to match.
                    self.rename_note_to_title();
                }
                // AUTOMATIC LOCAL SNAPSHOT: a loose note just hit the disk, so capture
                // a history point (git-managed files + history-off are skipped inside).
                self.snapshot_after_save();
            }
            // Empty note (no first line yet): nothing to write. Stay quiet.
            Err(_) => {}
        }
    }

    /// SAVE-HOOK for AUTOMATIC LOCAL HISTORY: after a successful save, record a
    /// snapshot of the current buffer to the local history store (see
    /// [`crate::history::record`]). The store itself decides whether to keep it —
    /// a GIT-MANAGED file (git owns its versioning) or `history = false` writes
    /// nothing; a loose note/draft (or any file on the web) is snapshotted, keyed
    /// by its path + a timestamp, and pruned to stay bounded. A no-op for a scratch
    /// buffer that has no bound path yet. Best-effort: any store error is swallowed
    /// inside `record`, so a failed history write never disrupts the save.
    pub(super) fn snapshot_after_save(&self) {
        let path = self.buffer.path().or(self.file.as_deref());
        if let Some(path) = path {
            crate::history::record(path, &self.buffer.text(), &self.config);
        }
    }

    /// RESTORE a local-history VERSION into the buffer (the summoned timeline's Enter).
    /// Resolves `id` to its captured content via [`crate::history::load`] — the awl log
    /// for a loose file, `git show` for a git-managed one — and replaces the whole
    /// buffer with it via [`crate::buffer::Buffer::set_text`], which is ONE atomic,
    /// undoable edit (so C-/ undoes the restore, exactly like any other edit). Keyed on
    /// the SAME path the snapshot store records under (`buffer.path()`, else `self.file`).
    /// A no-op for a scratch buffer with no path, or an unknown / unresolvable id
    /// (best-effort — a failed restore must never disrupt the buffer).
    pub(super) fn restore_history(&mut self, id: &str) {
        let path = self
            .buffer
            .path()
            .or(self.file.as_deref())
            .map(|p| p.to_path_buf());
        if let Some(path) = path {
            if let Some(content) = crate::history::load(&path, id) {
                self.buffer.set_text(&content);
            }
        }
    }

    /// OPT-IN periodic autosnapshot (the finer-interval `autosnapshot_secs` knob).
    /// DEFAULT OFF (interval 0) → this returns immediately and is fully inert. When
    /// enabled it records a snapshot at most once per configured interval of quiet,
    /// keyed off `last_autosnapshot`; unlike the save-hook it also runs INSIDE a git
    /// repo — via [`crate::history::record_periodic`], which bypasses ONLY the
    /// git-presence gate (the save-hook's [`crate::history::record`] would return
    /// early for a git-managed file and silently break this contract). The
    /// history-off switch, dedup, and prune bound still apply inside. Called from
    /// `about_to_wait`. Returns true if a snapshot was taken (so the caller can
    /// refresh its timer).
    pub(super) fn maybe_periodic_snapshot(&mut self) -> bool {
        let secs = self.config.autosnapshot_secs();
        if secs == 0 {
            return false; // OFF by default: inert, no behaviour change
        }
        let now = crate::clock::Instant::now();
        let due = self
            .last_autosnapshot
            .map(|t| now.saturating_duration_since(t).as_secs() >= secs)
            .unwrap_or(true);
        if !due {
            return false;
        }
        self.last_autosnapshot = Some(now);
        self.snapshot_periodic();
        true
    }

    /// The PERIODIC sibling of [`Self::snapshot_after_save`]: record the current
    /// buffer through [`crate::history::record_periodic`], which snapshots even a
    /// GIT-MANAGED file (the whole point of the `autosnapshot_secs` knob — a
    /// between-commit safety net). Keyed on the same path the save hook records
    /// under; a scratch buffer with no bound path is a quiet no-op, and any store
    /// error is swallowed inside the recorder (best-effort, like the save hook).
    pub(super) fn snapshot_periodic(&self) {
        let path = self.buffer.path().or(self.file.as_deref());
        if let Some(path) = path {
            crate::history::record_periodic(path, &self.buffer.text(), &self.config);
        }
    }

    /// LIVE-RENAME the active note's file to follow its FIRST LINE. Called after an
    /// autosave of an already-named note: re-derive the title slug ([the same
    /// derivation the first save uses](crate::buffer::note_stem)); if the file's
    /// name no longer matches it, `fs::rename` to the fresh slug (non-clobbering,
    /// mirroring [`Self::move_current_note`]) and re-sync `App.file`, the buffer's
    /// path, the window title, and the go-to index. A no-op when the name already
    /// tracks the title or the note has gone empty. Notes only.
    pub(super) fn rename_note_to_title(&mut self) {
        if !self.buffer.is_note() {
            return;
        }
        let Some(old) = self.file.clone() else {
            return;
        };
        let text = self.buffer.text();
        // An emptied first line keeps the current name (nothing meaningful to
        // re-derive); there is nothing to rename TO.
        let Some(line) = crate::buffer::first_nonempty_line(&text) else {
            return;
        };
        let stem = crate::buffer::note_stem(line);
        let new_path = match crate::buffer::rename_to_stem(&old, &stem) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("rename failed ({}): {e}", old.display());
                return;
            }
        };
        if new_path == old {
            return; // name already tracks the title
        }
        eprintln!("renamed {} -> {}", old.display(), new_path.display());
        self.buffer.set_path(new_path.clone());
        self.file = Some(new_path);
        self.update_title();
        // Re-scope the go-to index so the note is jump-able under its new name.
        self.file_index = crate::index::build_index(&self.root);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// C-x m accept: MOVE the current note into `dest_rel` (a directory relative to
    /// the notes root; `""` = the notes root itself), keeping the filename. Creates
    /// the destination folder if needed, refuses to clobber (numeric suffix), then
    /// re-points the buffer + `App.file` so editing/auto-save continue at the new
    /// path. A true `std::fs::rename` move — never a copy.
    pub(super) fn move_current_note(&mut self, dest_rel: &str) {
        let Some(old) = self.file.clone() else {
            return; // no current file to move
        };
        let dest_dir = if dest_rel.is_empty() {
            self.notes_root.clone()
        } else {
            self.notes_root.join(dest_rel)
        };
        // The actual mkdir + no-clobber + rename lives in `buffer::move_file` (the
        // one move primitive, unit-tested on a temp dir).
        let new_path = match crate::buffer::move_file(&old, &dest_dir) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("move failed ({} -> {}): {e}", old.display(), dest_dir.display());
                return;
            }
        };
        if new_path == old {
            return; // already there: nothing changed
        }
        eprintln!("moved {} -> {}", old.display(), new_path.display());
        self.buffer.set_path(new_path.clone());
        self.file = Some(new_path);
        // Keep auto-saving into the note's new home.
        if self.buffer.is_note() {
            self.buffer.set_note_dir(dest_dir);
        }
        self.update_title();
        self.file_index = crate::index::build_index(&self.root);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }
}
