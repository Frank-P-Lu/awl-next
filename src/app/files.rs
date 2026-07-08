//! BUFFER + CONFIG management: opening project-relative files, the last-buffer
//! toggle, quick-note creation / auto-save / live-rename / move, the window
//! title, the active project root, the DOCUMENT AUTOSAVE ENGINE (config-gated,
//! default ON: atomic write on idle/blur/switch/quit with a clobber guard, plus
//! the persistent scratch stash), and the sticky-preference + rebind-menu
//! config writes (open Settings, persist theme/zoom/page/caret, live reload,
//! commit/reset a captured binding). Lifted out of `app.rs` verbatim.

use super::*;
use std::path::Path;

/// The App-level per-buffer bookkeeping that must travel with a buffer when it
/// is BACKGROUNDED (parked in `App::buffer_registry`) and restored when it
/// comes back to the foreground — everything `App` tracks about the ACTIVE
/// buffer beyond the `Buffer` itself (whose cursor/selection/undo/dirty are
/// already its own business). Mirrors the App fields it snapshots 1:1; see
/// `App::snapshot_extra` / `App::restore_extra`.
///
/// NOT carried here (deliberately): the quick-NOTE debounce fields
/// (`autosave_dirty_at` / `autosave_saved_version`) stay App-global — they only
/// ever matter while `buffer.is_note()`, and a note only becomes registry-
/// keyable once it has been named (given a real path), at which point it is an
/// ordinary pathed buffer for every OTHER purpose here; a stale value simply
/// re-triggers one redundant (harmless) autosave on reactivation.
#[derive(Default)]
pub(super) struct BufferExtra {
    /// Whether the buffer's active selection (if any) was begun with Shift —
    /// TRANSIENT, but tied to THIS buffer's `anchor`, so it travels with it
    /// rather than leaking whatever the LAST-active buffer happened to leave it
    /// at (a plain unshifted motion in the reactivated buffer resets it anyway;
    /// this only matters for the one motion right after a switch).
    pub shift_selecting: bool,
    pub scroll_lines: usize,
    pub spell_cache: Vec<crate::spell::Misspelling>,
    pub spell_checked_version: Option<u64>,
    pub spell_dirty_at: Option<Instant>,
    pub sync_text_cache: Option<(u64, String)>,
    pub caret_synced_version: u64,
    pub doc_saved_version: Option<u64>,
    pub scratch_saved_version: Option<u64>,
    pub disk_mtime: Option<crate::fs::Metadata>,
    pub scratch_mtime: Option<crate::fs::Metadata>,
    pub doc_autosave_at: Option<Instant>,
}

impl App {
    /// Snapshot the App-level per-buffer fields into a [`BufferExtra`], taking
    /// each one (leaving the App field at its default) — the caller
    /// immediately overwrites them from either a restored entry or a fresh
    /// buffer's defaults, so nothing is ever read in this transient in-between
    /// state.
    fn snapshot_extra(&mut self) -> BufferExtra {
        BufferExtra {
            shift_selecting: std::mem::take(&mut self.shift_selecting),
            scroll_lines: self.scroll_lines,
            spell_cache: std::mem::take(&mut self.spell_cache),
            spell_checked_version: self.spell_checked_version.take(),
            spell_dirty_at: self.spell_dirty_at.take(),
            sync_text_cache: self.sync_text_cache.take(),
            caret_synced_version: self.caret_synced_version,
            doc_saved_version: self.doc_saved_version.take(),
            scratch_saved_version: self.scratch_saved_version.take(),
            disk_mtime: self.disk_mtime.take(),
            scratch_mtime: self.scratch_mtime.take(),
            doc_autosave_at: self.doc_autosave_at.take(),
        }
    }

    /// Restore a [`BufferExtra`] (from a re-activated registry entry, or a
    /// freshly-built default for a first-time open) into the App fields —
    /// the inverse of `snapshot_extra`.
    fn restore_extra(&mut self, extra: BufferExtra) {
        self.shift_selecting = extra.shift_selecting;
        self.scroll_lines = extra.scroll_lines;
        self.spell_cache = extra.spell_cache;
        self.spell_checked_version = extra.spell_checked_version;
        self.spell_dirty_at = extra.spell_dirty_at;
        self.sync_text_cache = extra.sync_text_cache;
        self.caret_synced_version = extra.caret_synced_version;
        self.doc_saved_version = extra.doc_saved_version;
        self.scratch_saved_version = extra.scratch_saved_version;
        self.disk_mtime = extra.disk_mtime;
        self.scratch_mtime = extra.scratch_mtime;
        self.doc_autosave_at = extra.doc_autosave_at;
    }

    /// PARK the active buffer into `buffer_registry` under its stable identity
    /// (a no-op for an ephemeral, still-empty pathless note — see
    /// `crate::buffers::BufferKey::of`), leaving `self.buffer` a throwaway
    /// scratch placeholder for the caller to immediately overwrite. The ONE
    /// door every "the active buffer is about to be replaced" site goes
    /// through (`load_path`, `new_note`), so backgrounding a buffer always
    /// preserves the same state.
    fn park_active_buffer(&mut self) {
        let Some(key) = crate::buffers::BufferKey::of(&self.buffer) else {
            return;
        };
        let buffer = std::mem::replace(&mut self.buffer, Buffer::scratch());
        let extra = self.snapshot_extra();
        self.buffer_registry
            .park(key, crate::buffers::Entry { buffer, extra });
    }

    /// How many buffers are open right now (the active one + everything
    /// backgrounded) — feeds the sidecar-analog debug line / future chrome.
    /// Not yet surfaced live (no chrome in v1); kept here as the one place
    /// that knows the count.
    #[allow(dead_code)]
    pub(super) fn open_buffer_count(&self) -> usize {
        self.buffer_registry.len() + 1
    }


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
            "dictionary" => self.config.dictionary = Some(value.trim_matches('"').to_string()),
            "page_mode" => self.config.page_mode = Some(value == "true"),
            "page_width_prose" => self.config.page_width_prose = value.parse().ok(),
            "page_width_code" => self.config.page_width_code = value.parse().ok(),
            "zoom" => self.config.zoom = value.parse().ok(),
            "writing_nits" => self.config.writing_nits = Some(value == "true"),
            "spellcheck" => self.config.spellcheck = Some(value == "true"),
            // Settings-menu TOGGLES that were previously write-only (no mirror): keep
            // `self.config` in step with disk so the still-open menu's value cell
            // (read from `self.config` for the mechanism-B keys) and a later
            // conflict/reload check both see the current value.
            "autosave" => self.config.autosave = Some(value == "true"),
            "history" => self.config.history = Some(value == "true"),
            "session_restore" => self.config.session_restore = Some(value == "true"),
            "wysiwyg" => self.config.wysiwyg = Some(value == "true"),
            "inline_images" => self.config.inline_images = Some(value == "true"),
            "outline" => self.config.outline = Some(value == "true"),
            "project_root" => {
                self.config.project_root = Some(PathBuf::from(value.trim_matches('"')))
            }
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

    /// Persist the now-active SPELLCHECK on/off (write-on-change after "Toggle
    /// Spellcheck"). Mirrors `persist_page_mode` / the writing-nits persist call.
    pub(super) fn persist_spellcheck(&mut self) {
        let on = crate::spell::spellcheck_on();
        self.persist_pref("spellcheck", if on { "true" } else { "false" });
    }

    /// SETTINGS MENU toggle (Enter on a `SettingKind::Toggle` row): flip the sticky
    /// boolean `key`, apply it LIVE this frame, PERSIST the negated value, then
    /// refresh the STILL-OPEN menu's value cell. Two mechanisms:
    ///   * PROCESS-GLOBAL (page_mode / wysiwyg / inline_images / spellcheck /
    ///     writing_nits) — flip the shared global so the renderer picks it up, then
    ///     reshape / rescan / repaint as that global demands (this is the seam that
    ///     closes the WYSIWYG live-apply gap: `set_wysiwyg_on` fires HERE, and the
    ///     pipeline's per-frame wysiwyg/inline latch — see `render.rs` `set_view` —
    ///     forces the conceal restyle the incremental text diff would otherwise skip);
    ///   * PROCESS-GLOBAL (outline) — flip `outline::OUTLINE_ON` so the renderer
    ///     picks up the margin outline this frame, then repaint (like writing_nits).
    ///   * CONFIG-ONLY (autosave / history / session_restore) — no global;
    ///     persisting the flipped value into `self.config` is enough (they are read
    ///     live from the config on demand).
    /// Persistence rides the ONE `persist_pref` owner (its mirror-match now covers
    /// every key here), so there is no bespoke per-toggle writer to drift.
    pub(super) fn setting_toggle(&mut self, key: &str) {
        // Read the CURRENT value from the SAME owner the readout reads, then negate.
        let now = match key {
            "page_mode" => crate::page::page_on(),
            "wysiwyg" => crate::markdown::wysiwyg_on(),
            "inline_images" => crate::markdown::inline_images_on(),
            "spellcheck" => crate::spell::spellcheck_on(),
            "writing_nits" => crate::nits::nits_on(),
            "autosave" => self.config.autosave_on(),
            "history" => self.config.history_on(),
            "session_restore" => self.config.session_restore_on(),
            "outline" => crate::outline::outline_on(),
            _ => return, // unknown key: a calm no-op
        };
        let next = !now;
        // (a) Apply the mechanism-A process-globals LIVE so the flip renders. wysiwyg
        //     / inline_images are the two that had NO live-apply path before this seam.
        match key {
            "page_mode" => crate::page::set_page_on(next),
            "wysiwyg" => crate::markdown::set_wysiwyg_on(next),
            "inline_images" => crate::markdown::set_inline_images_on(next),
            "spellcheck" => crate::spell::set_spellcheck_on(next),
            "writing_nits" => crate::nits::set_nits_on(next),
            "outline" => crate::outline::set_outline_on(next),
            _ => {} // mechanism-B: config-only, applied on read
        }
        // (b) Persist the negated value (the mirror-match keeps `self.config` in step).
        self.persist_pref(key, if next { "true" } else { "false" });
        // (c) Reshape / rescan / repaint as the flipped global demands.
        match key {
            // A page-column / conceal / image change: re-wrap (page mode) + let the
            // next frame's wysiwyg/inline latch restyle the conceal, then re-push.
            "page_mode" | "wysiwyg" | "inline_images" => {
                if let Some(gpu) = self.gpu.as_mut() {
                    let (w, h) = (gpu.config.width as f32, gpu.config.height as f32);
                    gpu.pipeline.set_size(w, h);
                }
                self.sync_view(true);
            }
            // Squiggles vanish/reappear this frame (mirrors `ToggleSpellcheck`).
            "spellcheck" => self.run_spellcheck_now(),
            // Render-only nit highlighter (mirrors `toggle_writing_nits`).
            "writing_nits" => self.sync_view(false),
            // Render-only margin outline (mirrors `writing_nits`): repaint so the
            // outline appears/vanishes this frame (the draw lands next phase).
            "outline" => self.sync_view(false),
            _ => {}
        }
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
        // (d) Refresh the still-open menu's value cell in place.
        self.refresh_settings_overlay();
    }

    /// After a settings toggle, rebuild the STILL-OPEN settings menu's value cells in
    /// place (mirrors [`Self::refresh_rebind_overlay`]): re-gather the config/project
    /// values so the flipped row's SECONDARY column reflects the new state (the
    /// process-globals are re-read live inside the readout). A no-op if the settings
    /// menu isn't the open overlay.
    pub(super) fn refresh_settings_overlay(&mut self) {
        let values =
            crate::settings::SettingsValues::gather(&self.config, &self.root, self.zoom);
        if let Some(ov) = self.overlay.as_mut() {
            if ov.kind == crate::overlay::OverlayKind::Settings {
                ov.bindings = crate::settings::value_cells(&values);
            }
        }
    }

    /// SETTINGS MENU inline VALUE commit (Enter on a `SettingKind::Value` row): parse
    /// the typed `raw` for config `key`, CLAMP it to that setting's sane range, apply
    /// it LIVE, and PERSIST the NAMED key — then refresh the still-open menu's cell.
    /// Unlike the drag / `C-x {` write (`persist_page_width`, which targets the ACTIVE
    /// buffer's class), the row NAMES its class, so we write exactly `key` and re-sync
    /// through the ONE `sync_page_measure` owner (which applies live iff that class is
    /// the active buffer's — editing the code width while a `.md` is open is persisted
    /// but not visibly re-wrapped, correctly). An unparseable value is a calm no-op
    /// (the cell reverts on the next refresh). Zoom rides the SAME `set_zoom` +
    /// `persist_zoom_now` path the wheel / ⌘± owner uses.
    pub(super) fn setting_value_commit(&mut self, key: &str, raw: &str) {
        match key {
            "page_width_prose" | "page_width_code" => {
                if let Ok(n) = raw.trim().parse::<usize>() {
                    let clamped = crate::settings::clamp_page_width(n);
                    // Persist the NAMED key (the mirror-match keeps `self.config` in
                    // step), then re-resolve the measure for the active buffer's class.
                    self.persist_pref(key, &clamped.to_string());
                    self.sync_page_measure();
                    self.sync_view(true);
                }
            }
            "zoom" => {
                if let Some(z) = crate::settings::parse_zoom(raw) {
                    self.set_zoom(z); // clamps + re-metrics next sync (the ⌘± owner)
                    self.persist_zoom_now(); // a discrete commit persists at once
                    self.sync_view(true);
                }
            }
            _ => {}
        }
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
        self.refresh_settings_overlay();
    }

    /// SETTINGS MENU path pick (the folder navigator opened from a `SettingKind::Path`
    /// row accepted a folder): write the NAMED config key `key` for `path`. For
    /// `project_root` this IS a genuine switch-project (re-index + persist +
    /// recent-MRU, the ONE `switch_project` owner); for `notes_root`/`workspace` we
    /// persist the key then `reload_config`, which re-folds `self.notes_root`/
    /// `self.workspace` (flag > config > default) so the NEXT `C-x n`/`C-x p` uses the
    /// new folder. Either way the still-open (re-summoned) menu's cell is refreshed.
    pub(super) fn setting_path_pick(&mut self, key: &str, path: &str) {
        match key {
            "project_root" => self.switch_project(PathBuf::from(path)),
            "notes_root" | "workspace" => {
                self.persist_pref(key, &format!("\"{path}\""));
                self.reload_config();
            }
            _ => {}
        }
        self.refresh_settings_overlay();
    }

    /// The config key naming the sticky page-width pref for `class` — the ONE
    /// owner every persist/reset/resync call routes the class->key mapping
    /// through, so it can never drift between them.
    fn page_width_key(class: crate::page::PageClass) -> &'static str {
        match class {
            crate::page::PageClass::Prose => "page_width_prose",
            crate::page::PageClass::Code => "page_width_code",
        }
    }

    /// Persist the now-active PAGE WIDTH / measure (write-on-change after a Page wider
    /// / Page narrower command, or a page-column drag release) to the key matching
    /// the ACTIVE buffer's KIND (`page_width_prose` vs `page_width_code` — see
    /// [`crate::page::PageClass`]), so widening a `.rs` file never bleeds into the
    /// prose measure a `.md` file reads. Zoom-independent: remembers the COLUMN
    /// width, not the glyph size (zoom has its own sticky pref).
    pub(super) fn persist_page_width(&mut self) {
        let w = crate::page::measure();
        let key = Self::page_width_key(self.buffer.page_class());
        self.persist_pref(key, &w.to_string());
    }

    /// "Reset Page Width" WRITE-ON-CHANGE: CLEAR the sticky override MATCHING the
    /// active buffer's KIND entirely (format-preserving removal,
    /// [`Config::remove_pref`]) rather than writing that class's default measure
    /// back — the `Option` already means "built-in default", so a future
    /// [`crate::page::PageClass::default_measure`] change flows through instead of
    /// pinning a stale value. Never touches the OTHER class's override. A no-op
    /// when there is no resolvable config path (e.g. no HOME), and silent on a
    /// write error, mirroring `persist_pref`.
    pub(super) fn persist_page_reset(&mut self) {
        let path = self.config.path.clone();
        if path.as_os_str().is_empty() {
            return; // no config path (no HOME): nothing to remember
        }
        let class = self.buffer.page_class();
        let key = Self::page_width_key(class);
        if let Err(e) = Config::remove_pref(&path, key) {
            eprintln!("could not clear {key} in {}: {e}", path.display());
            return;
        }
        match class {
            crate::page::PageClass::Prose => self.config.page_width_prose = None,
            crate::page::PageClass::Code => self.config.page_width_code = None,
        }
    }

    /// Re-apply the STICKY PAGE-WIDTH MEASURE for the ACTIVE buffer's KIND — the
    /// buffer OPEN/SWITCH half of the prose/code split (see
    /// [`crate::page::PageClass`]): a prose document (markdown / no-path /
    /// unrecognized) reads `page_width_prose`, a recognized code file reads
    /// `page_width_code`, each falling back to its own built-in default when
    /// unconfigured ([`Config::measure_for`]). Called after every buffer swap
    /// (`load_path`, `new_note`) and after a live config reload, so opening a
    /// `.rs` after a `.md` (or back) always shows THAT file's own measure — never
    /// a value carried over from whatever was active before.
    ///
    /// Mirrors the exact re-wrap dance `Action::PageWider`/`TogglePageMode`
    /// already do in `apply.rs`: force `set_size` (which re-derives the wrap
    /// width from the now-updated `page::measure()` and invalidates `row_geom` on
    /// an actual change — see `TextPipeline::set_size`) so the very next
    /// cursor-follow scroll computation reads FRESH row geometry instead of a
    /// stale pre-switch layout for one frame. (The per-frame `sync_wrap_width`
    /// invariant in `prepare` would eventually self-correct on its own, but only
    /// on the NEXT drawn frame — this keeps the switch itself glitch-free. A
    /// no-op pre-GPU-init, since `set_size` only runs when `self.gpu` exists.)
    pub(super) fn sync_page_measure(&mut self) {
        let target = self.config.measure_for(self.buffer.page_class());
        crate::page::set_measure(target);
        if let Some(gpu) = self.gpu.as_mut() {
            let (w, h) = (gpu.config.width as f32, gpu.config.height as f32);
            gpu.pipeline.set_size(w, h);
        }
    }

    /// Persist the now-active PROJECT ROOT (write-on-change after a switch-project,
    /// C-x p, commit) — the STICKY PROJECT pref: a plain relaunch (no file argument,
    /// no `--root`) reopens this same project (see `resolve_root` in `main/run.rs`).
    pub(super) fn persist_project_root(&mut self) {
        let root = self.root.display().to_string();
        self.persist_pref("project_root", &format!("\"{root}\""));
    }

    /// SWITCH the active project to `new_root` — the ONE owner of a genuine
    /// switch-project (both the `Project` picker's accepted folder AND the
    /// Recent Projects picker route here). Re-scopes the root ([`Self::set_root`]),
    /// persists it as the STICKY project (a plain relaunch reopens it,
    /// [`Self::persist_project_root`]), AND pushes it to the front of the
    /// persisted RECENT list ([`Self::push_recent_project`]). A quick-note jump
    /// (C-x n) deliberately does NOT come through here — it calls `set_root`
    /// directly, so it neither persists the sticky root nor counts as a "recent
    /// project" (only an intentional switch does).
    pub(super) fn switch_project(&mut self, new_root: PathBuf) {
        self.set_root(new_root.clone());
        self.persist_project_root();
        self.push_recent_project(new_root);
    }

    /// Push `root` to the FRONT of the persisted RECENT PROJECT ROOTS (deduped +
    /// capped, [`crate::recents::push`]) and save the list ATOMICALLY. A save
    /// error is reported and swallowed (a lost MRU entry is never worth crashing
    /// a project switch). Native/live only — the headless capture never
    /// constructs an `App`, so this file is never touched from a capture.
    pub(super) fn push_recent_project(&mut self, root: PathBuf) {
        let list = std::mem::take(&mut self.recent_projects);
        self.recent_projects = crate::recents::push(list, root, crate::recents::CAP);
        if let Err(e) = crate::recents::save(&crate::recents::recents_path(), &self.recent_projects) {
            eprintln!("recent-projects save failed: {e}");
        }
    }

    /// Push `file` to the FRONT of the persisted RECENTLY-OPENED FILES MRU (deduped
    /// + capped, [`crate::recent_files::push`]) and save it ATOMICALLY. A save error
    /// is reported + swallowed (a lost MRU entry is never worth crashing a file
    /// open). Native/live only — the headless capture never constructs an `App`, so
    /// `recent-files.toml` is never touched from a capture. The FILE sibling of
    /// [`Self::push_recent_project`].
    pub(super) fn push_recent_file(&mut self, file: PathBuf) {
        let list = std::mem::take(&mut self.recent_files);
        self.recent_files = crate::recent_files::push(list, file);
        if let Err(e) = crate::recent_files::save(&self.recent_files) {
            eprintln!("recent-files save failed: {e}");
        }
    }

    /// Persist the now-active CARET MODE (write-on-change after a caret-mode change).
    /// Phase 2 relies on this seam to remember the caret style across launches.
    pub(super) fn persist_caret_mode(&mut self) {
        let name = crate::config::caret_mode_name(crate::caret::mode());
        self.persist_pref("caret_mode", &format!("\"{name}\""));
    }

    /// Persist the now-active DICTIONARY variant (write-on-change after the
    /// Dictionary picker commits).
    pub(super) fn persist_dictionary(&mut self) {
        let name = crate::config::dictionary_name(crate::spell::active_variant());
        self.persist_pref("dictionary", &format!("\"{name}\""));
    }

    /// SWITCH the active spell-check dictionary: reconstruct the App's
    /// [`crate::spell::SpellChecker`] for `variant` (the ONE real per-switch cost —
    /// timed + reported here, so a live switch's latency is observable), then
    /// INVALIDATE the spell debounce + squiggle cache (`spell_checked_version` /
    /// `spell_dirty_at`) and recompute IMMEDIATELY — a discrete picker commit
    /// deserves instant feedback, not the next-edit debounce — before persisting
    /// the sticky pref. A failed parse disables spell-check (reported to stderr),
    /// exactly like the `App::new` startup path.
    pub(super) fn set_dictionary(&mut self, variant: crate::spell::DictVariant) {
        let t0 = std::time::Instant::now();
        self.spell = match crate::spell::SpellChecker::new(variant) {
            Ok(sc) => Some(sc),
            Err(e) => {
                eprintln!("dictionary switch failed: {e}");
                None
            }
        };
        eprintln!(
            "dictionary switched to {}: parsed in {:.2}ms",
            crate::config::dictionary_name(variant),
            t0.elapsed().as_secs_f64() * 1000.0
        );
        // CACHE-KEY DISCIPLINE: `spell_checked_version` gates on the BUFFER's
        // version alone, which the dictionary switch never bumps — so without this
        // reset the stale cache would look "current" until the next edit. Clearing
        // it (and any pending debounce) forces `run_spellcheck_now` to actually
        // re-scan against the new dictionary right away.
        self.spell_checked_version = None;
        self.spell_dirty_at = None;
        self.run_spellcheck_now();
        self.persist_dictionary();
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
    ///
    /// SPELLCHECK and the PAGE-WIDTH pair (`page_width_prose`/`page_width_code`)
    /// are ALSO re-applied here (unlike the other sticky prefs — theme / page /
    /// caret / writing_nits / dictionary — which apply ONCE at launch via
    /// `apply_sticky_globals` and otherwise only change via their own live
    /// toggle): a hand-edited `spellcheck = false` saved straight into the config
    /// buffer takes effect immediately, exactly like using the "Toggle
    /// Spellcheck" palette command, and the rescan below clears/restores
    /// squiggles in the SAME frame rather than waiting for the next edit's
    /// debounce. Likewise, hand-editing `page_width_code` while a `.rs` file is
    /// open re-wraps it immediately (`sync_page_measure`), since the config alone
    /// (not a live toggle) is the only way to change either key's OVERRIDE value.
    pub(super) fn reload_config(&mut self) {
        let cfg = Config::load(self.config.path.clone());
        self.keymap.apply_overrides(&cfg.keys);
        self.notes_root =
            crate::resolve_notes_root(&self.cli_notes_root.clone().or_else(|| cfg.notes_root.clone()));
        let workspace_opt = self.cli_workspace.clone().or_else(|| cfg.workspace.clone());
        self.workspace = Some(crate::resolve_workspace(&workspace_opt, &self.root));
        crate::spell::set_spellcheck_on(cfg.spellcheck.unwrap_or(true));
        self.config = cfg;
        // STICKY PAGE WIDTH: an edited `page_width_prose`/`page_width_code` takes
        // effect immediately too, re-resolved against the buffer that is CURRENTLY
        // active (its kind is unchanged by a config reload; only the configured
        // override might be).
        self.sync_page_measure();
        self.run_spellcheck_now();
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
    /// keep `App.file` + window title in sync. The product model is open/switch
    /// only — no file ops — so we just re-read from disk. `rel` is a root-relative
    /// index entry. The recently-opened-files MRU is pushed inside [`Self::load_path`]
    /// (the ONE door every real-file open routes through), so this stays a thin
    /// resolve-and-load.
    pub(super) fn open_rel(&mut self, rel: &str) {
        let path = crate::index::resolve(&self.root, rel);
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
    /// `prev_file` (the 2-deep last-buffer history), then either SWITCH to its
    /// already-open live buffer (unsaved edits + cursor + scroll + undo + spell
    /// state all survive — the multi-buffer registry win) or read it fresh from
    /// disk for a first-time open. Shared by `open_rel` and the C-x b toggle so
    /// both keep the history honest.
    pub(super) fn load_path(&mut self, path: PathBuf) {
        // ROBUST AUTOSAVE: before we drop the current buffer, flush any pending
        // note write so nothing typed in the last debounce window is lost — and
        // flush the LEAVING document / scratch through the autosave engine
        // (locked decision: save on file switch).
        self.flush_note();
        self.autosave_flush();
        // If the flush we just ran raised the clobber-guard notice (the file we
        // are LEAVING changed on disk outside awl, so its unsaved edit could
        // not be safely autosaved), that notice must survive the switch below
        // — otherwise the unconditional clear a few lines down would wipe it
        // in the very same call it was set, so the user never sees it at all
        // (code review nit: a real, if minor, live bug — the warning fires
        // and vanishes before a single frame renders it).
        let clobber_notice_just_raised = self.notice.is_some();
        // Already the active file: a no-op reopen preserves everything for free
        // (and avoids parking a buffer under its own key). Compared via the
        // SAME normalized identity the registry uses (`BufferKey::path`), not
        // raw path equality — a relative launch argument and its later
        // root-joined spelling (see `BufferKey::path`'s doc) must both be
        // recognized as "already here", or this falls through into an
        // unnecessary (if harmless, post-fix) park/take round trip.
        if self.file.as_deref().map(crate::buffers::BufferKey::path)
            == Some(crate::buffers::BufferKey::path(&path))
        {
            return;
        }
        // The file we are leaving becomes the last-buffer target.
        self.prev_file = self.file.take();
        self.park_active_buffer();
        let key = crate::buffers::BufferKey::path(&path);
        match self.buffer_registry.take(&key) {
            // ALREADY OPEN elsewhere in this session: switch to its LIVE buffer
            // instead of re-reading disk — unsaved edits, cursor, scroll, undo,
            // and spell-cache state all survive the round trip.
            Some(entry) => {
                self.buffer = entry.buffer;
                self.restore_extra(entry.extra);
            }
            // First time open this session: read fresh from disk.
            None => {
                self.buffer = Buffer::from_file(&path);
                self.restore_extra(BufferExtra::default());
                // AUTOSAVE bookkeeping for the ARRIVING file: its buffer IS the
                // on-disk content, so it starts saved; the current mtime is the
                // clobber guard's baseline. Stamped BEFORE the i18n write-back
                // below, so a stamped tag correctly reads as a PENDING edit
                // (buffer.version() past doc_saved_version) rather than being
                // mistaken for already-on-disk content — autosave picks it up
                // on the next idle/blur/switch/quit exactly like any other edit.
                self.disk_mtime = Self::disk_mtime_of(&path);
                self.doc_saved_version = Some(self.buffer.version());
                // A brand-new buffer starts at version 0; match the synced
                // version so the next sync_view doesn't read the delta as an
                // edit and streak the caret.
                self.caret_synced_version = self.buffer.version();
                // i18n WRITE-BACK-ONCE: an untagged CJK document gets a `lang:`
                // frontmatter tag stamped in as one normal undoable edit (never
                // for a pure-Latin doc, never a second time on a doc that
                // already carries a frontmatter block). Live-App-only by
                // construction (called only from this fresh-open branch) — the
                // headless `load_buffer` never reaches this function at all.
                self.write_back_lang_tag_once();
            }
        }
        if !clobber_notice_just_raised {
            self.notice = None;
        }
        self.file = Some(path.clone());
        // RECENTLY-OPENED FILES MRU: this file was just OPENED (either fresh from
        // disk or switched-to from the buffer registry — BOTH arrive here), so push
        // it to the front of the persisted MRU that feeds the go-to Recent lens +
        // recency tier. After the already-active early-return above, so re-selecting
        // the current file is a no-op that never re-orders the MRU.
        self.push_recent_file(path.clone());
        // LIFETIME STATS: record this open into the distinct-files set (deduped),
        // beside the recent-files MRU push — the same door. Native-only + config-
        // gated inside; a re-open of an already-seen path is inert.
        #[cfg(not(target_arch = "wasm32"))]
        self.stats_touch_file(path);
        // LIFETIME STATS: the buffer just swapped — drop the caret-travel anchor
        // so the new document's first caret sample re-anchors rather than counting
        // the cross-document coordinate jump as travel.
        #[cfg(not(target_arch = "wasm32"))]
        self.stats_reset_caret_anchor();
        // LIFETIME STATS: flush on the file-SWITCH trigger (the same door the
        // autosave flush above rides), so the just-recorded touch + any pending
        // keystroke/caret increments survive the switch (native only; gated).
        #[cfg(not(target_arch = "wasm32"))]
        self.stats_flush();
        self.search = None;
        self.preedit.clear();
        // The HISTORY TIMELINE preview cache is keyed to the buffer we just left;
        // a stale hit would preview the wrong file's version. The overlay is
        // never open across a buffer swap in practice, but this is the same
        // defensive drop `history_overlay_closed` already does on a real close.
        self.history_preview = None;
        // STICKY PAGE WIDTH: re-apply the measure for the ARRIVING buffer's own
        // kind (prose vs code — see `Config::measure_for`) BEFORE `sync_view`, so
        // its cursor-follow scroll math reads freshly re-wrapped row geometry
        // rather than whatever the LEAVING buffer's kind left behind.
        self.sync_page_measure();
        self.update_title();
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// i18n WRITE-BACK-ONCE: on a fresh (first-time-this-session) open of an
    /// UNTAGGED markdown document that contains CJK, stamp a `lang:`
    /// frontmatter tag in as ONE normal undoable buffer edit — never a silent
    /// disk write (the version bump is picked up by the ordinary autosave
    /// engine on the next idle/blur/switch/quit, exactly like any other edit;
    /// Cmd-Z removes it cleanly, restoring the pre-tag text and cursor). Called
    /// ONLY from [`Self::load_path`]'s fresh-disk-read branch, so:
    ///  - a PURE-LATIN document ([`crate::script::dominant_cjk`] returns `None`)
    ///    is NEVER touched — no frontmatter block, no version bump, no undo
    ///    entry;
    ///  - a document that ALREADY carries a frontmatter block (tagged or not)
    ///    is NEVER re-tagged — [`crate::frontmatter::detect`] finds it and this
    ///    returns immediately, so write-back happens AT MOST ONCE in a
    ///    document's life (a later reopen this session hits the buffer-
    ///    registry SWITCH branch instead, which never calls this at all; a
    ///    reopen in a FRESH session sees the tag already on disk from the
    ///    first pass and detects it, so it still never re-fires);
    ///  - a NON-markdown buffer (a `.rs`/`.txt`/`.env` path) is never touched —
    ///    frontmatter is a markdown/notes convention, and stamping literal
    ///    `---`/`lang:` text into a code file would corrupt it.
    /// A Han-only (ambiguous) document resolves via the config `cjk_priority`
    /// ladder (default ja-first); an unambiguous script (kana/hangul/bopomofo)
    /// always wins regardless of the ladder — see `crate::script::dominant_cjk`
    /// / `doc_lang_for`.
    pub(super) fn write_back_lang_tag_once(&mut self) {
        if !self.buffer.is_markdown() {
            return;
        }
        let text = self.buffer.text();
        if crate::frontmatter::detect(&text).is_some() {
            return; // already carries a frontmatter block — never re-tag
        }
        let Some(script) = crate::script::dominant_cjk(&text) else {
            return; // pure Latin — never touched
        };
        let lang = crate::script::doc_lang_for(script, &self.config.cjk_priority_or_default());
        let block = format!("---\nlang: {}\n---\n", lang.code());
        self.buffer.replace_char_range(0, 0, &block);
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

    /// Re-scan `self.root`'s file index through the `FileSystem` trait (git
    /// `ls-files` union `.env*`, or a recursive walk — see `index::build_index`)
    /// and replace the cached `file_index` with the fresh result. The ONE owner
    /// of "make the go-to corpus current": every trigger that can make the old
    /// index stale (a root switch, a note's first save, a rename, a move) calls
    /// this rather than re-deriving the same line; the Goto summon itself
    /// (`C-x f`, `app/apply.rs`) also calls it — RE-SCAN ON EVERY SUMMON (queue:
    /// "file picker freshness"), so a file created on disk after the app
    /// launched or last scanned is never missing. No cache TTL, no watcher: a
    /// summoned overlay is transient and the walk is disk-cheap for a real
    /// project tree (measured on this repo: see `index::tests::build_index_on_this_repo_is_fast`).
    pub(super) fn rescan_file_index(&mut self) {
        self.file_index = crate::index::build_index(&self.root);
    }

    /// Make `new_root` the ACTIVE project: re-resolve the project, rebuild the
    /// file index, reset the MRU, and re-sync the view. Shared by switch-project
    /// (C-x p) and the new-note jump (C-x n) so both re-scope the go-to list the
    /// same way. No buffer is opened here (that is the caller's concern).
    pub(super) fn set_root(&mut self, new_root: PathBuf) {
        // ROBUST AUTOSAVE: switching project re-scopes (and may precede a buffer
        // swap), so flush a pending note write first — never lose the open note.
        // The document autosave / scratch stash flushes on the same trigger.
        self.flush_note();
        self.autosave_flush();
        self.root = new_root;
        self.project = crate::project::Project::resolve(&self.root);
        self.rescan_file_index();
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
        // PARK the buffer we are leaving (registered under its own identity if
        // it has one) exactly like `load_path`, so a later C-x b / reopen finds
        // it live rather than re-reading disk.
        self.park_active_buffer();
        self.buffer.start_note(self.notes_root.clone());
        self.restore_extra(BufferExtra::default());
        self.search = None;
        self.preedit.clear();
        self.caret_synced_version = self.buffer.version();
        self.autosave_saved_version = None;
        self.autosave_dirty_at = None;
        // STICKY PAGE WIDTH: a fresh note is always markdown (PROSE), so this
        // re-applies `page_width_prose` regardless of what the leaving buffer's
        // kind was — mirrors `load_path`'s own resync.
        self.sync_page_measure();
        // LIFETIME STATS: a fresh note is a buffer swap — drop the caret-travel
        // anchor so its first caret sample re-anchors (see `load_path`).
        #[cfg(not(target_arch = "wasm32"))]
        self.stats_reset_caret_anchor();
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
                        self.rescan_file_index();
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

    /// SAVE-HOOK for AUTOMATIC LOCAL HISTORY: after a successful save (manual OR
    /// autosave — every save records), record a snapshot of the current buffer to
    /// the local history store (see [`crate::history::record`]). The store itself
    /// decides whether to keep it — a GIT-MANAGED file (git owns its versioning,
    /// unconditionally) or `history = false` writes nothing; a loose note/draft
    /// (or any file on the web) is snapshotted, keyed by its path + a timestamp,
    /// and pruned by the aged retention ladder. A no-op for a scratch buffer that
    /// has no bound path yet (the scratch stash records under its own stash
    /// path). Best-effort: any store error is swallowed inside `record`, so a
    /// failed history write never disrupts the save.
    ///
    /// CONSCIOUS MARK (banked, not built): a deliberate pin-this-version-before-
    /// major-surgery flag would be minted here and carried into the store,
    /// exempt from the ladder. See `history::prune_ladder`.
    pub(super) fn snapshot_after_save(&self) {
        let path = self.buffer.path().or(self.file.as_deref());
        if let Some(path) = path {
            crate::history::record(path, &self.buffer.text(), &self.config);
        }
    }

    /// THE CONSCIOUS MARK ("Keep This Version"): record the CURRENT buffer state as a
    /// PINNED, prune-EXEMPT local-history snapshot ([`crate::history::record_pinned`]).
    /// Keyed on the SAME path the snapshot store records/restores under
    /// ([`crate::history::source_path`]: the buffer's own path, else `self.file`, else
    /// the persistent scratch's stash path — so the scratch can be pinned too). A
    /// no-op for an unnamed note (no history key yet), a git-managed file (git owns
    /// its versioning — awl pins nothing there), or `history = false`; the store
    /// itself enforces those gates. Best-effort: any store error is swallowed inside
    /// `record_pinned`, so a failed pin never disrupts the buffer.
    pub(super) fn keep_version(&self) {
        let path = crate::history::source_path(
            self.buffer.path(),
            self.file.as_deref(),
            self.buffer.is_note(),
        );
        if let Some(path) = path {
            crate::history::record_pinned(&path, &self.buffer.text(), &self.config);
        }
    }

    /// RESTORE a local-history VERSION into the buffer (the summoned timeline's Enter).
    /// Resolves `id` to its captured content via [`crate::history::load`] — the awl log
    /// for a loose file, `git show` for a git-managed one — and replaces the whole
    /// buffer with it via [`crate::buffer::Buffer::set_text`], which is ONE atomic,
    /// undoable edit (so C-/ undoes the restore, exactly like any other edit). Keyed on
    /// the SAME path the snapshot store records under ([`crate::history::source_path`]:
    /// `buffer.path()`, else `self.file`, else the persistent scratch's stash path — so
    /// the scratch timeline restores too). A no-op for an unnamed note, or an unknown /
    /// unresolvable id (best-effort — a failed restore must never disrupt the buffer).
    pub(super) fn restore_history(&mut self, id: &str) {
        let path = crate::history::source_path(
            self.buffer.path(),
            self.file.as_deref(),
            self.buffer.is_note(),
        );
        if let Some(path) = path {
            if let Some(content) = crate::history::load(&path, id) {
                self.buffer.set_text(&content);
            }
        }
    }

    /// The current on-disk STAT (mtime + byte length) of `path` via the FS trait,
    /// or `None` when the file doesn't exist. The clobber guard's stat — wasm-safe
    /// (the times are `crate::clock::SystemTime`).
    pub(super) fn disk_mtime_of(path: &Path) -> Option<crate::fs::Metadata> {
        crate::fs::active().metadata(path).ok()
    }

    /// CLOBBER-GUARD truth table: has `path` changed on disk since `last` (our
    /// last-known stat)? `(current, last)`:
    ///   * `(None, None)`  → false — the file never existed; our write CREATES it.
    ///   * `(Some, Some)`  → changed iff the MTIME moved OR the SIZE differs. The
    ///     size guard catches an external edit that lands within the SAME mtime
    ///     tick as our last stat (equal mtime, changed content → changed length),
    ///     which a bare mtime compare would silently overwrite.
    ///   * `(Some, None)`  → true — the file APPEARED externally since we looked.
    ///   * `(None, Some)`  → true — the file was DELETED externally.
    /// Pure over the stat, so the four arms are unit-testable.
    pub(super) fn disk_changed(path: &Path, last: Option<crate::fs::Metadata>) -> bool {
        match (Self::disk_mtime_of(path), last) {
            (None, None) => false,
            (Some(c), Some(l)) => {
                c.modified != l.modified
                    || match (c.len, l.len) {
                        (Some(cl), Some(ll)) => cl != ll,
                        _ => false,
                    }
            }
            (Some(_), None) => true,
            (None, Some(_)) => true,
        }
    }

    /// The AUTOSAVE ENGINE's flush — the one door every trigger goes through
    /// (idle, window blur, file switch, quit). Config-gated (`autosave`, default
    /// ON). Routes by buffer kind: a NOTE keeps its own 400ms flow (untouched); a
    /// pathed document writes atomically via [`Self::autosave_doc_now`]; a true
    /// scratch (no path, not a note) stashes via [`Self::stash_scratch_now`].
    /// Lives only on the live `App`, so the headless capture is structurally
    /// autosave-free (determinism law).
    pub(super) fn autosave_flush(&mut self) {
        self.doc_autosave_at = None;
        if !self.config.autosave_on() {
            return;
        }
        if self.buffer.is_note() {
            return; // notes have their own debounced autosave (flush_note)
        }
        if self.buffer.path().is_some() {
            self.autosave_doc_now();
        } else {
            self.stash_scratch_now();
        }
    }

    /// Quietly SAVE the open document NOW (the autosave engine's pathed-buffer
    /// arm): skip when the buffer version is already on disk; hold the write —
    /// with a calm notice — when the file changed on disk outside awl (the
    /// CLOBBER GUARD; a manual Cmd-S still force-writes per the locked
    /// contract); otherwise write atomically, re-stat the mtime, clear the
    /// notice, and record a history snapshot (the store's git gate + dedup +
    /// ladder decide what's kept). Errors go to stderr, never disrupt.
    fn autosave_doc_now(&mut self) {
        let Some(path) = self.buffer.path().map(|p| p.to_path_buf()) else {
            return;
        };
        let version = self.buffer.version();
        if self.doc_saved_version == Some(version) {
            return; // nothing new to write
        }
        if Self::disk_changed(&path, self.disk_mtime) {
            self.notice = Some("changed on disk outside awl — autosave held".to_string());
            // Mark the version handled so the idle timer doesn't spin on the
            // same content; the next edit re-arms (and the notice recurs calmly).
            self.doc_saved_version = Some(version);
            return;
        }
        // Restore the buffer's remembered line ending on the way out (CRLF files
        // round-trip byte-for-byte; LF is byte-identical to `text().as_bytes()`).
        match crate::fs::write_atomic(&path, &self.buffer.disk_bytes()) {
            Ok(()) => {
                self.doc_saved_version = Some(version);
                self.disk_mtime = Self::disk_mtime_of(&path);
                self.notice = None;
                // DEBUG PANEL: stamp the engine's own "last wrote successfully"
                // clock, the ONLY place it is ever written (see `autosave_last_ok`).
                self.autosave_last_ok = Some(Instant::now());
                // Every save records a snapshot (dedup + the git gate live inside).
                self.snapshot_after_save();
            }
            Err(e) => eprintln!("autosave failed ({}): {e}", path.display()),
        }
    }

    /// STASH the persistent SCRATCH buffer NOW (the autosave engine's no-path
    /// arm): write the whole text — EVEN empty, so an emptied scratch clears a
    /// stale stash — atomically to [`crate::fs::scratch_stash_path`], guarded by
    /// the same clobber truth-table (two awl instances sharing one stash), then
    /// grow the stash's own ladder timeline via [`crate::history::record`]. The
    /// restore half lives in `App::new` (a no-argument launch).
    fn stash_scratch_now(&mut self) {
        let version = self.buffer.version();
        if self.scratch_saved_version == Some(version) {
            return; // stash already holds this content
        }
        let path = crate::fs::scratch_stash_path();
        if Self::disk_changed(&path, self.scratch_mtime) {
            self.notice = Some("changed on disk outside awl — autosave held".to_string());
            self.scratch_saved_version = Some(version);
            return;
        }
        let text = self.buffer.text();
        let fs = crate::fs::active();
        if let Some(parent) = path.parent() {
            let _ = fs.create_dir_all(parent);
        }
        // A true scratch buffer is always Lf, but route the write through the ONE
        // encoder for uniformity; the history snapshot stays the internal pure-`\n`
        // `text` (awl's own store — see the "Line endings" note in CLAUDE.md).
        match crate::fs::write_atomic(&path, &self.buffer.disk_bytes()) {
            Ok(()) => {
                self.scratch_saved_version = Some(version);
                self.scratch_mtime = Self::disk_mtime_of(&path);
                self.notice = None;
                // DEBUG PANEL: stamp the engine's own "last wrote successfully"
                // clock, the ONLY place it is ever written (see `autosave_last_ok`).
                self.autosave_last_ok = Some(Instant::now());
                // The persistent scratch grows a timeline of its own.
                crate::history::record(&path, &text, &self.config);
            }
            Err(e) => eprintln!("scratch stash failed ({}): {e}", path.display()),
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
        self.rescan_file_index();
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
        self.rescan_file_index();
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn switch_project_pushes_and_persists_the_recent_root() {
        let fake = Arc::new(
            crate::fs::InMemoryFs::new()
                .with_dir("/w/proj-a")
                .with_dir("/w/proj-b"),
        );
        crate::fs::with_fs(fake, || {
            let mut app = App::new(None, PathBuf::from("/w/proj-a"), None, None, Config::empty());
            // Fresh launch: no recents yet (missing store).
            assert!(app.recent_projects.is_empty());

            // Switching to two projects pushes each to the FRONT, newest-first.
            app.switch_project(PathBuf::from("/w/proj-a"));
            app.switch_project(PathBuf::from("/w/proj-b"));
            assert_eq!(
                app.recent_projects,
                vec![PathBuf::from("/w/proj-b"), PathBuf::from("/w/proj-a")],
            );
            assert_eq!(app.root, PathBuf::from("/w/proj-b"), "root followed the switch");

            // Re-switching to proj-a moves it to the front (dedup, never a dupe).
            app.switch_project(PathBuf::from("/w/proj-a"));
            assert_eq!(
                app.recent_projects,
                vec![PathBuf::from("/w/proj-a"), PathBuf::from("/w/proj-b")],
            );

            // The list is PERSISTED: a second launch reads it back (via the store).
            let reloaded = crate::recents::load(&crate::recents::recents_path());
            assert_eq!(
                reloaded,
                vec![PathBuf::from("/w/proj-a"), PathBuf::from("/w/proj-b")],
            );
        });
    }

    #[test]
    fn opening_files_pushes_them_onto_the_recent_files_mru_and_persists() {
        let fake = Arc::new(
            crate::fs::InMemoryFs::new()
                .with_file("/w/proj/a.md", "a")
                .with_file("/w/proj/b.md", "b")
                .with_file("/w/proj/c.md", "c"),
        );
        crate::fs::with_fs(fake, || {
            let mut app = App::new(None, PathBuf::from("/w/proj"), None, None, Config::empty());
            assert!(app.recent_files.is_empty(), "fresh launch: empty MRU");

            // Opening three files pushes each to the FRONT (most-recent first). Both
            // load_path branches route here; a fresh disk read is the None branch.
            app.load_path(PathBuf::from("/w/proj/a.md"));
            app.load_path(PathBuf::from("/w/proj/b.md"));
            app.load_path(PathBuf::from("/w/proj/c.md"));
            assert_eq!(
                app.recent_files,
                vec![
                    PathBuf::from("/w/proj/c.md"),
                    PathBuf::from("/w/proj/b.md"),
                    PathBuf::from("/w/proj/a.md"),
                ],
            );

            // Re-opening a.md (the buffer-registry SWITCH branch) moves it to the
            // front — dedup, never a dupe.
            app.load_path(PathBuf::from("/w/proj/a.md"));
            assert_eq!(
                app.recent_files,
                vec![
                    PathBuf::from("/w/proj/a.md"),
                    PathBuf::from("/w/proj/c.md"),
                    PathBuf::from("/w/proj/b.md"),
                ],
            );

            // Re-selecting the ALREADY-ACTIVE file is a no-op (load_path's early
            // return), so the MRU is untouched — no re-order, no dupe.
            app.load_path(PathBuf::from("/w/proj/a.md"));
            assert_eq!(app.recent_files.len(), 3, "no-op reopen never re-orders / dupes");
            assert_eq!(app.recent_files[0], PathBuf::from("/w/proj/a.md"));

            // PERSISTED: a second launch reads the MRU back through the store.
            assert_eq!(crate::recent_files::load(), app.recent_files);
        });
    }

    #[test]
    fn app_new_loads_the_persisted_recent_projects() {
        let fake = Arc::new(crate::fs::InMemoryFs::new().with_dir("/w/proj-a"));
        crate::fs::with_fs(fake, || {
            // Pre-seed the store, then launch: App::new loads it into the field.
            crate::recents::save(
                &crate::recents::recents_path(),
                &[PathBuf::from("/w/proj-a"), PathBuf::from("/w/proj-b")],
            )
            .unwrap();
            let app = App::new(None, PathBuf::from("/w/proj-a"), None, None, Config::empty());
            assert_eq!(
                app.recent_projects,
                vec![PathBuf::from("/w/proj-a"), PathBuf::from("/w/proj-b")],
            );
        });
    }
}
