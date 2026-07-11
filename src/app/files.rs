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
    /// this; you then edit + Cmd-S to save, which live-reloads (see `reload_config`).
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

    /// Credits command: open the embedded `CREDITS.md` into the buffer, exactly
    /// like Settings opens the config file. UNLIKE Settings, the source of truth
    /// is the BINARY (`credits::CREDITS_MD`), not a user-owned disk file — so this
    /// always REFRESHES the on-disk view to the embedded text before opening it
    /// (never a create-if-missing; the doc must never drift from what shipped).
    /// Routed through a real path (under `fs::data_root()`) rather than left
    /// path-less: a path-less buffer reads as SCRATCH to the autosave engine
    /// (`autosave_flush`'s `buffer.path().is_none()` arm), which would silently
    /// overwrite the user's real scratch stash the next time autosave flushes —
    /// see `credits.rs`'s module doc for the full reasoning.
    pub(super) fn open_credits(&mut self) {
        let path = crate::fs::data_root().join("credits.md");
        let fs = crate::fs::active();
        if let Some(parent) = path.parent() {
            let _ = fs.create_dir_all(parent);
        }
        if let Err(e) = crate::fs::write_atomic(&path, crate::credits::CREDITS_MD.as_bytes()) {
            eprintln!("could not write credits view {}: {e}", path.display());
            return;
        }
        self.load_path(path);
    }

    /// Guide command: open the embedded `GUIDE.md` into the buffer, exactly like
    /// Credits opens `CREDITS.md` (same on-disk-refresh-then-load pattern, same
    /// reasoning for why it is NOT left path-less — see `open_credits`'s doc
    /// above and `guide.rs`'s module doc). Rendered through `guide::render`
    /// (the CONVENTION-TRUTHFUL SURFACES round's chord-token substitution) at
    /// OPEN TIME for the live convention/platform, so the doc always names the
    /// chord that actually fires under THIS session.
    pub(super) fn open_guide(&mut self) {
        let path = crate::fs::data_root().join("guide.md");
        let fs = crate::fs::active();
        if let Some(parent) = path.parent() {
            let _ = fs.create_dir_all(parent);
        }
        let rendered = crate::guide::render(crate::convention::Convention::current(), crate::commands::Platform::current());
        if let Err(e) = crate::fs::write_atomic(&path, rendered.as_bytes()) {
            eprintln!("could not write guide view {}: {e}", path.display());
            return;
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
            "code_ligatures" => self.config.code_ligatures = Some(value == "true"),
            "outline" => self.config.outline = Some(value == "true"),
            "menu_bar" => self.config.menu_bar = Some(value == "true"),
            "reduce_motion" => self.config.reduce_motion = Some(value == "true"),
            // KEYMAP FLAVOR: a quoted string ("native"/"emacs"), not a bool — mirrors
            // "theme"/"caret_mode"/"dictionary" above, not the bool toggles.
            "keymap" => self.config.keymap = Some(value.trim_matches('"').to_string()),
            // The CJK ladder is written as a whole TOML array (see
            // `persist_cjk_priority`); the mirror reads the LIVE process global
            // (already updated by the picker's core-level accept) rather than
            // re-parsing the formatted `value` string back into a `Vec<Lang>`.
            "cjk_priority" => self.config.cjk_priority = Some(crate::frontmatter::cjk_priority()),
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
        // KEYMAP is NOT a plain bool config key (its value is "native"/"emacs", not
        // "true"/"false"), so it can't ride the generic bool mechanism below —
        // special-cased here, before the generic `now`/`next` match, and handled
        // by its own dedicated door (`toggle_keymap_flavor`).
        if key == "keymap" {
            self.toggle_keymap_flavor();
            return;
        }
        // Read the CURRENT value from the SAME owner the readout reads, then negate.
        let now = match key {
            "page_mode" => crate::page::page_on(),
            "typewriter_scroll" => crate::typewriter::typewriter_on(),
            "wysiwyg" => crate::markdown::wysiwyg_on(),
            "inline_images" => crate::markdown::inline_images_on(),
            "code_ligatures" => crate::render::code_ligatures_on(),
            "spellcheck" => crate::spell::spellcheck_on(),
            "writing_nits" => crate::nits::nits_on(),
            "autosave" => self.config.autosave_on(),
            "history" => self.config.history_on(),
            "session_restore" => self.config.session_restore_on(),
            "outline" => crate::outline::outline_on(),
            "menu_bar" => crate::menubar::menu_bar_on(),
            "reduce_motion" => crate::motion::reduced(),
            _ => return, // unknown key: a calm no-op
        };
        let next = !now;
        // (a) Apply the mechanism-A process-globals LIVE so the flip renders. wysiwyg
        //     / inline_images are the two that had NO live-apply path before this seam.
        match key {
            "page_mode" => crate::page::set_page_on(next),
            "typewriter_scroll" => crate::typewriter::set_typewriter_on(next),
            "wysiwyg" => crate::markdown::set_wysiwyg_on(next),
            "inline_images" => crate::markdown::set_inline_images_on(next),
            "code_ligatures" => crate::render::set_code_ligatures_on(next),
            "spellcheck" => crate::spell::set_spellcheck_on(next),
            "writing_nits" => crate::nits::set_nits_on(next),
            "outline" => crate::outline::set_outline_on(next),
            // ACCESSIBILITY TIER 1: an explicit toggle wins over `auto` from
            // here on — this is a deliberate user action, not a live OS-pref
            // poll (see `motion.rs`'s module doc). Any glide/flinch already in
            // flight settles on its very next step (the gate lives in
            // `advance`'s three callees; nothing further to force here).
            "reduce_motion" => crate::motion::set_reduced(next),
            "menu_bar" => crate::menubar::set_menu_bar_on(next),
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
            // A font-feature change (ligatures) alters `doc_attrs` but neither the
            // text nor the wrap column, so the incremental set_view would skip the
            // reshape — force it (set_view's `force` reshapes with the fresh attrs).
            "code_ligatures" => self.sync_view(true),
            // Squiggles vanish/reappear this frame (mirrors `ToggleSpellcheck`).
            "spellcheck" => self.run_spellcheck_now(),
            // Render-only nit highlighter (mirrors `Action::ToggleWritingNits`).
            "writing_nits" => self.sync_view(false),
            // Render-only margin outline (mirrors `writing_nits`): repaint so the
            // outline appears/vanishes this frame (the draw lands next phase).
            "outline" => self.sync_view(false),
            // The menu bar reserves vertical space via `doc_top`, so re-sync WITH
            // follow to re-inset the document below (or reclaim) the bar strip THIS
            // frame — mirrors the ToggleMenuBar apply arm.
            "menu_bar" => self.sync_view(true),
            // Scroll-only typewriter pin: re-sync with follow so the caret's row
            // re-centers (or reverts to cursor-follow) THIS frame — the cursor-follow
            // in `sync_view` now reads the flipped global.
            "typewriter_scroll" => self.sync_view(true),
            _ => {}
        }
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
        // (d) Refresh the still-open menu's value cell in place.
        self.refresh_settings_overlay();
    }

    /// THE KEYMAP FLAVOR TOGGLE (Enter on the "Keymap" settings row): flip
    /// `Config::keymap_flavor` (native <-> emacs), PERSIST it (a quoted string,
    /// not a bool — [`Self::persist_pref`] handles both shapes identically,
    /// like "theme"/"caret_mode"), then RE-APPLY the keymap live from the
    /// updated in-memory config — the SAME two calls [`Self::reload_config`]
    /// makes (`apply_overrides` + `apply_linux_keep` against the now-effective,
    /// flavor-widened keep list), so a live toggle takes effect immediately,
    /// exactly like hand-editing `keymap = "emacs"` into the config buffer and
    /// saving it.
    ///
    /// Deliberately NOT `self.reload_config()` (a re-READ from disk): a config
    /// with a genuinely EMPTY `path` (a bare `Config::empty()`, used by native
    /// test scaffolding — the web build now always resolves a real
    /// `fs::web_config_path()`, so this is no longer the web build's own case)
    /// would silently DISCARD the flip, since both `reload_config`'s fresh
    /// `Config::load` and `persist_pref`'s own disk write bail out early on an
    /// empty path. Instead the in-memory mirror is set HERE, unconditionally,
    /// before attempting the disk write, and the keymap is rebuilt straight
    /// from that mirror.
    ///
    /// WEB: the disk write now genuinely persists (`fs::web_config_path` over
    /// `WebFs`/`localStorage` — see the web-config round), so a keymap-flavor
    /// flip survives a page reload exactly like on native.
    pub(super) fn toggle_keymap_flavor(&mut self) {
        let next = match self.config.keymap_flavor() {
            crate::keymap::KeymapFlavor::Native => crate::keymap::KeymapFlavor::Emacs,
            crate::keymap::KeymapFlavor::Emacs => crate::keymap::KeymapFlavor::Native,
        };
        self.config.keymap = Some(next.config_name().to_string());
        self.persist_pref("keymap", &format!("\"{}\"", next.config_name()));
        let mut keys_with_web_alt = self.config.keys.clone();
        keys_with_web_alt.extend(crate::commands::web_alternate_keys(&self.config.keys, crate::convention::Convention::current(), crate::commands::Platform::current()));
        self.keymap.apply_overrides(&keys_with_web_alt);
        self.keymap.apply_linux_keep(&self.config.effective_linux_keep());
        self.refresh_settings_overlay();
        // Every sibling settings-mutation door (`setting_toggle`'s generic
        // path, `setting_value_commit`, `setting_path_pick`) ends in a
        // `request_redraw` of its own rather than leaning on whatever
        // generic post-dispatch redraw its caller happens to also issue —
        // match that convention here too (currently masked live by the
        // keyboard/mouse input handlers' own unconditional post-apply
        // redraw, but this door should not silently depend on that).
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// After a settings toggle, rebuild the STILL-OPEN settings menu's value cells in
    /// place (mirrors [`Self::refresh_rebind_overlay`]): re-gather the config/project
    /// values so the flipped row's SECONDARY column reflects the new state (the
    /// process-globals are re-read live inside the readout). A no-op if the settings
    /// menu isn't the open overlay. Reads through [`crate::settings::visible_value_cells`]
    /// — the SAME platform-filtered view `overlay::build`'s own `OverlayKind::Settings`
    /// branch seeds `ov.bindings` from — never the raw unfiltered
    /// [`crate::settings::value_cells`]; on native the two coincide (nothing is
    /// filtered), but a refresh must stay index-coherent with `ov.corpus`
    /// (`visible_names()`) even on web, where "Edit config as text" is hidden.
    pub(super) fn refresh_settings_overlay(&mut self) {
        let values =
            crate::settings::SettingsValues::gather(&self.config, &self.root, self.zoom);
        if let Some(ov) = self.overlay.as_mut() {
            if ov.kind == crate::overlay::OverlayKind::Settings {
                ov.bindings = crate::settings::visible_value_cells(&values);
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

    /// "Reset page width" WRITE-ON-CHANGE: CLEAR the sticky override MATCHING the
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
        // A cancelled MAS grant panel (see `set_root`'s doc) means the switch
        // never happened — never persist/MRU a root we didn't actually move
        // into.
        if !self.set_root(new_root.clone()) {
            return;
        }
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

    /// Persist the now-active CJK ambiguity LADDER (write-on-change after the
    /// CJK-priority language picker commits) — mirrors `persist_dictionary`,
    /// except the value is a whole ORDERED LIST rather than one scalar: the
    /// core already promoted + set the live global
    /// (`frontmatter::set_cjk_priority`), so this just formats it as a TOML
    /// array RHS and writes it through the same format-preserving `write_pref`
    /// (which only cares that `value` is an already-formatted RHS — an array
    /// upserts exactly like a string/bool/number). The config file keeps the
    /// FULL ordered list (not just the promoted front), so hand-editing and an
    /// old config both keep working unchanged.
    pub(super) fn persist_cjk_priority(&mut self) {
        let ladder = crate::frontmatter::cjk_priority();
        let quoted: Vec<String> = ladder.iter().map(|l| format!("\"{}\"", l.code())).collect();
        self.persist_pref("cjk_priority", &format!("[{}]", quoted.join(", ")));
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
        let mut keys_with_web_alt = cfg.keys.clone();
        keys_with_web_alt.extend(crate::commands::web_alternate_keys(&cfg.keys, crate::convention::Convention::current(), crate::commands::Platform::current()));
        self.keymap.apply_overrides(&keys_with_web_alt);
        self.keymap.apply_linux_keep(&cfg.effective_linux_keep());
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
        let keep = self.config.effective_linux_keep();
        if let Some(ov) = self.overlay.as_mut() {
            if ov.kind == crate::overlay::OverlayKind::Keybindings {
                ov.capture = None;
                ov.bindings = crate::commands::effective_bindings(&keys, &keep);
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
        // MAS SANDBOX GRANT GATE (native macOS `mas` builds only — see
        // `src/mas.rs`'s module doc): `path` may live outside the container.
        // `ensure_access` is a no-op fast-path for anything inside it or
        // inside an already-granted root; otherwise it powerboxes the user via
        // the system folder panel BEFORE any read below is attempted. A
        // cancelled panel aborts the open outright — never let a doomed read
        // fail against a silent sandbox `EPERM` instead.
        #[cfg(all(feature = "mas", target_os = "macos"))]
        if !crate::mas::ensure_access(&path) {
            return;
        }
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

    /// INLINE-IMAGE DRAG-RESIZE (v2) WRITE-BACK: stamp the settled `|NNN` width hint
    /// into the image's ALT text as ONE undoable edit — templated on
    /// [`Self::write_back_lang_tag_once`]'s single-`replace_char_range` shape. `range`
    /// is the `![alt](path)` span's DOCUMENT BYTE range (from the drag), `width_px` the
    /// final display width (rounded to the int hint). The pure
    /// [`crate::markdown::image_width_hint_edit`] computes the alt sub-range +
    /// replacement (Obsidian `![alt|NNN](path)`); we convert its byte offsets to buffer
    /// CHAR indices and apply one sealed replace. A non-empty replace never coalesces,
    /// so the whole drag is a single Cmd-Z (restoring the pre-drag size + text).
    ///
    /// NUANCE (from the lang-tag precedent): `replace_char_range` moves the caret to
    /// the edit end — but a MOUSE drag must NOT move the text caret. So snapshot the
    /// cursor, apply, then restore it (shifted by the edit's length delta only when it
    /// sat past the edit), so the caret stays exactly where it was.
    pub(super) fn write_back_image_width(&mut self, range: (usize, usize), width_px: f32) {
        if !self.buffer.is_markdown() {
            return;
        }
        let width = width_px.round().max(1.0) as u32;
        let text = self.buffer.text();
        let (bstart, bend) = range;
        let Some(src) = text.get(bstart..bend) else {
            return;
        };
        let Some((alt_b0, alt_b1, new_alt)) = crate::markdown::image_width_hint_edit(src, width)
        else {
            return;
        };
        // src-relative byte offsets -> absolute document byte offsets -> char indices.
        let abs_b0 = bstart + alt_b0;
        let abs_b1 = bstart + alt_b1;
        let c0 = text[..abs_b0].chars().count();
        let c1 = text[..abs_b1].chars().count();
        let new_len = new_alt.chars().count();
        // No-op guard: the alt already reads exactly the target — keep the timeline
        // meaningful (mirrors `apply_format`'s equal-text short-circuit).
        if text.get(abs_b0..abs_b1) == Some(new_alt.as_str()) {
            return;
        }
        // Snapshot the caret so the mouse drag never moves it (see the doc nuance).
        let saved = self.buffer.cursor_char();
        let delta = new_len as isize - (c1 - c0) as isize;
        self.buffer.seal_undo_group();
        self.buffer.replace_char_range(c0, c1, &new_alt);
        self.buffer.seal_undo_group();
        let restored = if saved <= c0 {
            saved
        } else if saved >= c1 {
            (saved as isize + delta).max(0) as usize
        } else {
            c0
        };
        self.buffer.set_cursor(restored);
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
    /// open/switch/theme-cycle all agree). Wraps the pure [`window_title`] — the
    /// ONE owner of the actual string, also used by the initial window
    /// construction in `resumed()` (before a `gpu`/window exists to `set_title`
    /// on), so a fresh launch's very first title and every later update agree.
    /// SAVE-FEEDBACK round: "is the active buffer UNSAVED", by the SAME
    /// version-vs-saved-version bookkeeping the autosave engine already
    /// tracks (`sync_view`'s own arm-check, mirrored here as a read) — NOT
    /// the raw `Buffer::is_dirty()` edit-tracked bit, which autosave (a
    /// direct `fs::write_atomic`, never routed through `Buffer::save`)
    /// deliberately never clears. Using the raw bit would leave the title's
    /// edited marker (and the native titlebar dot) stuck on indefinitely on
    /// an actively-autosaved document, even though its content is already
    /// safely on disk — misleading, and the opposite of what autosave is
    /// FOR. This reads true "unsaved" — cleared the instant ANY successful
    /// write lands, manual save or autosave alike — matching every
    /// conventional editor's own dirty-dot behavior.
    pub(super) fn is_document_dirty(&self) -> bool {
        if self.buffer.is_note() {
            self.autosave_saved_version != Some(self.buffer.version())
        } else if self.buffer.path().is_some() {
            self.doc_saved_version != Some(self.buffer.version())
        } else {
            self.scratch_saved_version != Some(self.buffer.version())
        }
    }

    /// NOTES VERBS round: push the held HUD's SAVED stat state into the pipeline —
    /// `Dirty` while the buffer has unsaved changes RIGHT NOW (`is_document_dirty`,
    /// the SAME check the window title's dirty-dot uses), else `Saved(secs)` from
    /// `last_saved_ok` (the last successful write of ANY kind — manual save, the
    /// scratch→note conversion, a note's own autosave, or the document autosave
    /// engine), else `None` when nothing has ever saved yet this session (renders
    /// the fixed placeholder). Called every `sync_view`, mirroring `stats_sync_hud`
    /// exactly — LIVE-ONLY (a real clock read), so a headless capture never calls
    /// this and the pipeline field stays `None`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn sync_hud_saved(&mut self) {
        let state = if self.is_document_dirty() {
            Some(crate::hud::HudSaved::Dirty)
        } else {
            self.last_saved_ok
                .map(|t| crate::hud::HudSaved::Saved(Instant::now().duration_since(t).as_secs()))
        };
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        gpu.pipeline.set_hud_saved(state);
    }

    /// CHECK FOR UPDATES round: push the About card's "checked … ago" figure —
    /// reads the LOCAL "last checked" marker (`updates::update_checked_state`,
    /// `Never` if no marker exists yet, `CheckedAgo(secs)` otherwise) against a
    /// real clock. Called every `sync_view`, mirroring `sync_hud_saved` exactly
    /// — LIVE-ONLY (a real clock + fs read), so a headless capture never calls
    /// this and the pipeline field stays `None` (the About card's determinism
    /// boundary — `updates::checked_line(None)` renders the fixed placeholder).
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn sync_update_checked(&mut self) {
        let dir = crate::fs::data_root();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let state = crate::updates::update_checked_state(&dir, now);
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        gpu.pipeline.set_update_checked(Some(state));
    }

    pub(super) fn update_title(&mut self) {
        // SAVE-FEEDBACK round: keep `title_dirty` (the cache `sync_view`
        // compares against for its "only re-title on a real flip" gate — see
        // its own doc) in step with whatever this call actually renders, no
        // matter which caller reached here.
        let dirty = self.is_document_dirty();
        self.title_dirty = dirty;
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.set_title(&window_title(
                self.file.as_deref(),
                self.buffer.is_note(),
                crate::theme::active().name,
                dirty,
            ));
            // NATIVE macOS TITLEBAR DIRTY-DOT: winit exposes this directly
            // (`WindowExtMacOS::set_document_edited` — the grey dot in the
            // titlebar's close button, the same convention every native Mac
            // document app uses), so no bespoke `mac_chrome.rs` plumbing is
            // needed for this one. LIVE-ONLY (needs human confirmation) — the
            // headless capture never constructs a `gpu`/window, so this is
            // unreachable from `--screenshot`/`--keys` and adds no sidecar
            // field, mirroring `cursor_shape`'s `set_cursor` precedent.
            #[cfg(target_os = "macos")]
            {
                use winit::platform::macos::WindowExtMacOS;
                gpu.window.set_document_edited(dirty);
            }
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
    /// Returns `false` ONLY when a MAS sandbox grant panel was cancelled (see
    /// the gate below) — every other path always switches and returns `true`;
    /// callers that persist a "switched to" fact (the sticky root, the recent-
    /// projects MRU) must check this before doing so.
    pub(super) fn set_root(&mut self, new_root: PathBuf) -> bool {
        // MAS SANDBOX GRANT GATE (native macOS `mas` builds only — see
        // `src/mas.rs`'s module doc): a project root reaches outside the
        // container far more often than a single file does, so this is the
        // OTHER real "touch outside the sandbox" door (Switch project…, the
        // C-x n notes-root jump). Same no-op-inside/first-touch-outside shape
        // as `load_path`'s gate.
        #[cfg(all(feature = "mas", target_os = "macos"))]
        if !crate::mas::ensure_access(&new_root) {
            return false;
        }
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
        true
    }

    /// C-x n: NEW QUICK NOTE in one gesture. Jump the active project to the notes
    /// root AND swap in a fresh empty note buffer; the user starts typing
    /// immediately. The filename is derived (slugified first line) + auto-saved on
    /// the first pause — see [`Self::autosave_note`]. The file we are leaving
    /// becomes the last-buffer (C-x b) target.
    pub(super) fn new_note(&mut self) {
        // The notes root may not exist yet; create it lazily so the project +
        // index resolve and the first save has somewhere to land. (A MAS
        // sandbox build against an ungranted external `notes_root` simply has
        // this first attempt fail silently — see `set_root`'s own grant gate
        // right below, which prompts and then this directory genuinely gets
        // created on the buffer's first save.)
        let _ = crate::fs::active().create_dir_all(&self.notes_root);
        // A cancelled MAS grant panel means the jump never happened — bail
        // before touching the buffer at all (mirrors `switch_project`).
        if !self.set_root(self.notes_root.clone()) {
            return;
        }
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
                        // SAVE-FEEDBACK round: no terminal echo — a background
                        // autosave naming a fresh note is silent chatter (the
                        // window title already renders the new name).
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
                // NOTES VERBS round: the held HUD's SAVED stat.
                self.last_saved_ok = Some(Instant::now());
            }
            // Empty note (no first line yet): nothing to write. Stay quiet.
            Err(_) => {}
        }
    }

    /// PASTE-IMAGE'S NO-PATH PRE-SAVE (`App::try_paste_image`, `app/apply.rs`): a
    /// path-less buffer — the bare scratch surface, or an unnamed quick note —
    /// has no directory to hang an `assets/` folder off of. Give it one FIRST by
    /// reusing the EXISTING quick-note auto-name save (`Self::autosave_note` →
    /// `Buffer::save`'s first-line-derived filename), rather than inventing a
    /// parallel naming rule. A plain scratch buffer (never summoned via C-x n —
    /// `note_dir` unset) is first PROMOTED into a note rooted at
    /// `self.notes_root`, the same home C-x n uses, via `Buffer::set_note_dir`
    /// (content-preserving — unlike `start_note`, nothing is reset) — so it now
    /// follows the notes model going forward (its own debounced autosave,
    /// live-rename-to-title, the aged-history ladder, …) exactly as if C-x n had
    /// started it. An already-in-progress note (`note_dir` already set) is left
    /// pointed at its own dir. An EMPTY buffer has no first line to derive a name
    /// from yet — `autosave_note` (via `Buffer::save`) errs quietly and the
    /// buffer stays path-less; the caller (`try_paste_image`) falls back to its
    /// pre-existing absolute data-root location rather than blocking the paste.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn ensure_note_named_before_paste(&mut self) {
        if !self.buffer.is_note() {
            let _ = crate::fs::active().create_dir_all(&self.notes_root);
            self.buffer.set_note_dir(self.notes_root.clone());
        }
        self.autosave_note();
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

    /// SAVE-FEEDBACK round: finish an explicit manual save (`Effect::SaveDone`,
    /// an already-pathed or already-note buffer's `C-x C-s` / `Cmd-S`) — the
    /// core already ran the SAME `Buffer::save` call every save path uses,
    /// `ok`/`message` report the outcome. On SUCCESS, capture a local-history
    /// point (the store's git gate / history-off / dedup all decide what's
    /// kept) and follow the AUTOSAVE ENGINE's own bookkeeping (the buffer
    /// version is now on disk — no redundant idle write; the fresh mtime is
    /// the clobber guard's new baseline — a manual save legitimately
    /// force-writes over an external change). Either way, raise the ONE
    /// user-visible acknowledgment this round adds — a brief "saved" that
    /// fades per the existing notice behavior, or the error message on a
    /// genuine failure (an unnamed empty note's "empty note: nothing to save
    /// yet" included) — replacing the round's own bug, where both fates only
    /// ever reached a terminal `eprintln!` (invisible on a GUI launch,
    /// printed to the wrong place from a terminal one). Autosave stays
    /// SILENT — only this explicit user action is acknowledged.
    pub(super) fn finish_manual_save(&mut self, ok: bool, message: String) {
        if ok {
            self.snapshot_after_save();
            if let Some(p) = self.buffer.path().map(|p| p.to_path_buf()) {
                self.disk_mtime = Self::disk_mtime_of(&p);
                self.doc_saved_version = Some(self.buffer.version());
            }
            // NOTES VERBS round: the held HUD's SAVED stat.
            self.last_saved_ok = Some(Instant::now());
        }
        self.notice = Some(message);
    }

    /// SAVE-FEEDBACK round: `Cmd-S` / `C-x C-s` on the TRUE scratch surface
    /// (`Effect::ConvertScratchAndSave`) — convert the pathless buffer into a
    /// real note, reusing the EXACT auto-name machinery
    /// [`Self::ensure_note_named_before_paste`] already established for the
    /// paste-image door ([`crate::buffer::Buffer::save_as_note`]: `set_note_dir`
    /// then `Buffer::save`, which derives the filename from the first line via
    /// the same `note_stem` a `C-x n` note uses), then finish the bookkeeping a
    /// normal manual save would (title, go-to index, the fresh note's own
    /// sticky page measure — a brand-new note is always PROSE, mirroring
    /// `new_note`'s resync) and RETIRE the persistent SCRATCH STASH: the
    /// content just became a real, named file, so a later bare relaunch must
    /// never resurrect a ghost copy of it from the old stash (best-effort —
    /// a failed remove never disrupts the save that already succeeded).
    /// Raises the SAME calm "saved" / "save failed: …" notice a plain manual
    /// save does — never a terminal print. A `notes_root` that doesn't exist
    /// or isn't writable surfaces here as the failure notice, never a crash.
    ///
    /// USER-FLIPPABLE (logged, not hidden): this round settled on "scratch
    /// Save promotes to a note" as the fix for the reported bug (silent save
    /// failure on Linux) — a future preference could instead make this
    /// notice-only ("nothing to save yet — start a note first"), leaving the
    /// scratch buffer untouched. Both are one function to swap here.
    pub(super) fn convert_scratch_and_save(&mut self) {
        match self.buffer.save_as_note(&self.notes_root) {
            Ok(()) => {
                if let Some(p) = self.buffer.path() {
                    self.file = Some(p.to_path_buf());
                }
                self.update_title();
                self.rescan_file_index();
                self.sync_page_measure();
                // RETIRE THE STASH: best-effort, mirroring every other
                // fallible bookkeeping call in this file — a failed remove
                // never disrupts the save that already succeeded.
                let _ = crate::fs::active().remove_file(&crate::fs::scratch_stash_path());
                self.scratch_saved_version = None;
                self.scratch_mtime = None;
                // The note's own debounced autosave now owns this buffer;
                // mark the version we just wrote as already-saved so the
                // next idle tick doesn't immediately rewrite it (mirrors
                // `autosave_note`'s own post-save bookkeeping).
                self.autosave_saved_version = Some(self.buffer.version());
                self.autosave_dirty_at = None;
                self.snapshot_after_save();
                if let Some(p) = self.buffer.path().map(|p| p.to_path_buf()) {
                    self.disk_mtime = Self::disk_mtime_of(&p);
                    self.doc_saved_version = Some(self.buffer.version());
                }
                self.notice = Some("saved".to_string());
                // NOTES VERBS round: the held HUD's SAVED stat.
                self.last_saved_ok = Some(Instant::now());
            }
            Err(e) => {
                self.notice = Some(format!("save failed: {e}"));
            }
        }
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// THE CONSCIOUS MARK ("Keep version"): record the CURRENT buffer state as a
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

    /// ASSET CLEANER: move the orphan at root-relative `rel` to the OS Trash
    /// (recoverable — never `rm`), then — ONLY on success — remove its row from the
    /// still-open picker (`OverlayState::remove_asset_row`), so the list shrinks as you
    /// clean and the picker stays up. A failure (a missing file, a non-macOS platform,
    /// an OS refusal) LEAVES the row and shows a calm dim notice. The trash goes through
    /// the injectable [`crate::assets::TrashCan`] seam, so a test drives it with a fake
    /// (the REAL macOS `NSFileManager` call is live-only, flagged).
    pub(super) fn trash_asset(&mut self, rel: String) {
        let abs = self.root.join(&rel);
        match crate::assets::active_trash().trash(&abs) {
            Ok(()) => {
                if let Some(ov) = self.overlay.as_mut() {
                    ov.remove_asset_row(&rel);
                    ov.notice.clear();
                }
            }
            Err(msg) => {
                if let Some(ov) = self.overlay.as_mut() {
                    ov.notice = format!("couldn't move to Trash: {msg}");
                }
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
                // NOTES VERBS round: the held HUD's SAVED stat.
                self.last_saved_ok = Some(Instant::now());
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
                // NOTES VERBS round: the held HUD's SAVED stat.
                self.last_saved_ok = Some(Instant::now());
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
            // SAVE-FEEDBACK round: no terminal echo — a live-rename is a
            // background autosave hiccup (the note is still safely saved
            // under its OLD name; nothing was lost), never worth interrupting
            // the user with a notice over.
            Err(_) => return,
        };
        if new_path == old {
            return; // name already tracks the title
        }
        // SAVE-FEEDBACK round: no terminal echo on success either — the
        // window title already renders the new name.
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
                // SAVE-FEEDBACK round: an explicit "Move note" is a discrete
                // user action, so a failure gets the SAME calm bottom-center
                // notice a failed manual save does — never a terminal print.
                self.notice = Some(format!("move failed: {e}"));
                return;
            }
        };
        if new_path == old {
            return; // already there: nothing changed
        }
        // No success notice — the window title already renders the new path.
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

    /// NOTES VERBS round: RENAME the current file to `new_name` (a bare filename,
    /// same directory — the minibuffer never lets a typed name cross directories,
    /// see `RenameEdit::push`'s `/`-rejection). THE ONE OWNER of every path-keyed
    /// store that must follow a rename: the buffer's own path, `App.file`, and the
    /// local-history log ([`crate::history::rename`]) — the multi-buffer REGISTRY
    /// never needs touching here because it only ever holds BACKGROUNDED buffers,
    /// and a rename only ever acts on the ACTIVE one; the recent-files MRU and the
    /// session file are left untouched, mirroring `move_current_note`'s own
    /// established scope (a soft MRU / a machine-state snapshot, not a hard
    /// identity — both self-heal on the next open/quit). REFUSES calmly (a notice,
    /// no write) rather than clobbering: a NAME COLLISION with an existing file, or
    /// a GIT-MANAGED source (git owns naming there — `git mv` is the honest tool).
    /// A blank or UNCHANGED typed name is a quiet no-op (nothing to rename to).
    pub(super) fn rename_current_file(&mut self, new_name: &str) {
        let Some(old) = self.file.clone() else {
            return; // nothing to rename (the prompt shouldn't have opened either)
        };
        let trimmed = new_name.trim();
        if trimmed.is_empty() {
            return;
        }
        let old_name = old.file_name().map(|s| s.to_string_lossy().to_string());
        if old_name.as_deref() == Some(trimmed) {
            return; // unchanged — nothing to do
        }
        if crate::history::is_git_managed(&old) {
            self.notice = Some("can't rename a file git already tracks".to_string());
            return;
        }
        let dest = match old.parent() {
            Some(p) => p.join(trimmed),
            None => PathBuf::from(trimmed),
        };
        if crate::fs::active().exists(&dest) {
            self.notice = Some(format!("already a file named \"{trimmed}\" here"));
            return;
        }
        if let Err(e) = crate::fs::active().rename(&old, &dest) {
            self.notice = Some(format!("rename failed: {e}"));
            return;
        }
        // Best-effort: the history log follows the file; a failed carry-over never
        // disrupts the rename that already succeeded on disk.
        let _ = crate::history::rename(&old, &dest);
        self.buffer.set_path(dest.clone());
        self.file = Some(dest.clone());
        if self.buffer.is_note() {
            if let Some(dir) = dest.parent() {
                self.buffer.set_note_dir(dir.to_path_buf());
            }
        }
        self.update_title();
        self.rescan_file_index();
        self.notice = Some(format!("renamed to {trimmed}"));
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// NOTES VERBS round: DUPLICATE the current file — copy the CURRENT buffer
    /// content (including any unsaved edits — a duplicate captures what you're
    /// actually looking at, not necessarily what's on disk) to an auto-named
    /// sibling, then open the copy as the active buffer via the ordinary
    /// [`Self::load_path`] door — which PARKS the original first (so ITS live
    /// edits are never lost) and gives the copy a genuinely FRESH history timeline
    /// (a brand-new `Buffer::from_file`, a brand-new local-history log — nothing
    /// carries over, since the copy is a new file). The sibling name is chosen by
    /// the SAME no-clobber dedup [`crate::buffer::unique_path`] uses elsewhere
    /// (`move_current_note`/live-rename) — `name-2.md`, `name-3.md`, … — never a
    /// space-separated `"name 2.md"`, matching the codebase's own established
    /// convention. A pathless buffer (scratch / an unnamed note) is a calm no-op —
    /// there is nothing to duplicate yet. Flushes any pending debounced write
    /// FIRST so the ORIGINAL reliably exists on disk under its own name before the
    /// dedup scan runs (otherwise a not-yet-flushed `old` would look "free" to
    /// `unique_path` and the copy could collide with it).
    pub(super) fn duplicate_current_file(&mut self) {
        let Some(old) = self.file.clone() else {
            return; // scratch: nothing to duplicate
        };
        self.flush_note();
        self.autosave_flush();
        let bytes = self.buffer.disk_bytes();
        let dir = old.parent().map(Path::to_path_buf).unwrap_or_default();
        let stem = old.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let ext = old.extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let new_path = crate::buffer::unique_path(&dir, &stem, &ext);
        match crate::fs::write_atomic(&new_path, &bytes) {
            Ok(()) => {
                self.load_path(new_path);
                self.notice = Some("duplicated".to_string());
            }
            Err(e) => {
                self.notice = Some(format!("duplicate failed: {e}"));
            }
        }
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }
}

/// THE window title string — a PURE function of "which document, which world,
/// is it dirty", so it is unit-testable without a real window
/// (`Window::set_title`/`with_title` are the only two live call sites:
/// [`App::update_title`] and the initial `Window::default_attributes()` in
/// `resumed()`, which reads this BEFORE a `gpu`/window exists to set a title
/// on). An UNTITLED quick note (a note buffer with no derived filename yet)
/// shows the "scratch" placeholder until its first line names it, so a
/// brand-new C-x n note reads as "scratch" — distinct from the no-path,
/// non-note SCRATCH launch surface's "*scratch*". The active WORLD name is
/// always the trailing `[…]` suffix — this is also the accessibility win
/// noted in `ACCESSIBILITY.md`: a screen reader's window list announces the
/// actual document, not a bare "awl".
///
/// SAVE-FEEDBACK round: `dirty` (`Buffer::is_dirty()`) prepends the
/// conventional macOS/VS Code EDITED marker — a leading `"• "` — the same
/// glyph macOS's own "unsaved changes" affordance uses elsewhere in the OS,
/// so it reads as ambient chrome rather than a new symbol to learn. TASTE
/// FLAGGED (logged, not hidden): the glyph itself (`•` vs a bare `*`, vs
/// nothing at all) is a live-review call — this round picked the bullet for
/// its quieter weight against the amber-caret-only design law (DESIGN §3);
/// see `App::update_title`'s doc for the matching native titlebar dot.
pub(super) fn window_title(file: Option<&Path>, is_note: bool, theme_name: &str, dirty: bool) -> String {
    let name = match file {
        Some(p) => p.display().to_string(),
        None if is_note => "scratch".to_string(),
        None => "*scratch*".to_string(),
    };
    let mark = if dirty { "\u{2022} " } else { "" };
    format!("awl - {mark}{name} [{theme_name}]")
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::sync::Arc;

    // --- window_title (ACCESSIBILITY TIER 1: the window names the document) ---

    #[test]
    fn window_title_names_a_pathed_file_and_the_active_world() {
        let t = window_title(Some(Path::new("/tmp/notes/draft.md")), false, "Quokka", false);
        assert_eq!(t, "awl - /tmp/notes/draft.md [Quokka]");
    }

    #[test]
    fn window_title_untitled_note_reads_scratch() {
        let t = window_title(None, true, "Tawny", false);
        assert_eq!(t, "awl - scratch [Tawny]");
    }

    #[test]
    fn window_title_bare_launch_scratch_reads_star_scratch_star() {
        let t = window_title(None, false, "Tawny", false);
        assert_eq!(t, "awl - *scratch* [Tawny]");
    }

    #[test]
    fn window_title_untitled_note_and_bare_scratch_are_distinct() {
        assert_ne!(
            window_title(None, true, "Tawny", false),
            window_title(None, false, "Tawny", false)
        );
    }

    // --- SAVE-FEEDBACK round: the dirty edited-marker, dirty × scratch/note/file ---

    #[test]
    fn window_title_dirty_pathed_file_gets_the_leading_marker() {
        let t = window_title(Some(Path::new("/tmp/notes/draft.md")), false, "Quokka", true);
        assert_eq!(t, "awl - \u{2022} /tmp/notes/draft.md [Quokka]");
    }

    #[test]
    fn window_title_clean_pathed_file_has_no_marker() {
        let t = window_title(Some(Path::new("/tmp/notes/draft.md")), false, "Quokka", false);
        assert!(!t.contains('\u{2022}'), "a clean buffer's title carries no edited marker");
    }

    #[test]
    fn window_title_dirty_untitled_note_gets_the_marker_too() {
        let t = window_title(None, true, "Tawny", true);
        assert_eq!(t, "awl - \u{2022} scratch [Tawny]");
    }

    #[test]
    fn window_title_dirty_bare_scratch_gets_the_marker_too() {
        let t = window_title(None, false, "Tawny", true);
        assert_eq!(t, "awl - \u{2022} *scratch* [Tawny]");
    }

    #[test]
    fn window_title_dirty_is_only_ever_a_leading_marker_insertion() {
        // Every other field held fixed — the dirty flip is EXACTLY inserting
        // "• " right after "awl - " and nothing else in the string moves.
        let clean = window_title(Some(Path::new("a.md")), false, "Bilby", false);
        let dirty = window_title(Some(Path::new("a.md")), false, "Bilby", true);
        assert_ne!(clean, dirty);
        assert_eq!(dirty, format!("awl - \u{2022} a.md [Bilby]"));
        assert_eq!(clean, format!("awl - a.md [Bilby]"));
        assert_eq!(dirty, clean.replacen("awl - ", "awl - \u{2022} ", 1));
    }

    #[test]
    fn update_title_uses_the_same_pure_window_title() {
        let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
        app.buffer.set_text("hello");
        // No live `gpu`/window in a hermetic App (see `App::update_title`'s gate) —
        // this proves the call is a harmless no-op off a real window and exercises
        // the same code path `resumed()`/`load_path`/theme-switch drive.
        app.update_title();
    }

    #[test]
    fn image_width_hint_write_back_is_one_undoable_edit_that_keeps_the_cursor() {
        // The v2 drag-resize WRITE-BACK over a real Buffer (the buffer/markdown seam):
        // insert/replace `|NNN` in the alt as ONE undoable edit, restoring the mouse
        // caret rather than moving it to the edit end.
        let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
        app.buffer.set_text("![a cat](cat.png)\ntail\n");
        assert!(app.buffer.is_markdown(), "a no-path scratch buffer is markdown");
        // Caret parked on the SECOND line (past the image span) — a mouse drag must
        // never move it.
        let cursor = app.buffer.text().chars().count() - 1;
        app.buffer.set_cursor(cursor);

        // INSERT: a 300px drag stamps `|300` into the hint-less alt (round from 300.4).
        app.write_back_image_width((0, 17), 300.4);
        assert_eq!(app.buffer.text(), "![a cat|300](cat.png)\ntail\n");
        // The caret shifted by the +4-char insertion (stayed on its glyph), never
        // jumped to the edit end.
        assert_eq!(app.buffer.cursor_char(), cursor + 4, "caret past the edit shifts by the delta");

        // ONE undoable edit: a single Cmd-Z restores the pre-drag text exactly.
        app.buffer.undo();
        assert_eq!(app.buffer.text(), "![a cat](cat.png)\ntail\n", "one Cmd-Z restores the size");

        // REPLACE: an existing `|NNN` is swapped in place (still one edit); a caret
        // BEFORE the edit never moves.
        app.buffer.set_text("![a cat|300](cat.png)\n");
        app.buffer.set_cursor(0);
        app.write_back_image_width((0, 21), 128.0);
        assert_eq!(app.buffer.text(), "![a cat|128](cat.png)\n");
        assert_eq!(app.buffer.cursor_char(), 0, "a caret before the edit stays put");
        app.buffer.undo();
        assert_eq!(app.buffer.text(), "![a cat|300](cat.png)\n", "one Cmd-Z restores the prior hint");

        // No-op guard: re-committing the SAME width records nothing (keeps the timeline
        // meaningful) — the text is unchanged and a following undo reaches PAST it.
        app.buffer.set_text("![a cat|200](cat.png)\n");
        app.buffer.set_cursor(3);
        app.write_back_image_width((0, 21), 200.0);
        assert_eq!(app.buffer.text(), "![a cat|200](cat.png)\n", "same width is a no-op");
        assert_eq!(app.buffer.cursor_char(), 3, "a no-op never disturbs the caret");
    }

    #[test]
    fn trash_asset_moves_the_file_and_removes_the_row_via_the_fake_seam() {
        let mut app = App::new_hermetic(None, PathBuf::from("/proj"), Config::empty());
        // Arm the ASSET CLEANER picker with two orphans (the scan is unit-tested in
        // `assets.rs`; here we drive the App's trash + row-removal wiring).
        let mk = |rel: &str| crate::assets::Orphan {
            rel: rel.to_string(),
            name: rel.rsplit('/').next().unwrap().to_string(),
            parent: rel.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default(),
            size: Some(42),
        };
        app.overlay = Some(crate::overlay::OverlayState::new_assets(vec![
            mk("assets/keep.png"),
            mk("assets/drop.png"),
        ]));

        let fake = Arc::new(crate::assets::FakeTrash::default());
        let recorder = fake.clone();
        crate::assets::with_trash(fake, || {
            app.trash_asset("assets/drop.png".to_string());
        });

        // The file was sent to the (fake) Trash at the ROOT-joined absolute path.
        assert_eq!(
            recorder.trashed.lock().unwrap().as_slice(),
            &[app.root.join("assets/drop.png")],
        );
        // The picker STAYS OPEN and the trashed row LEAVES the list.
        let ov = app.overlay.as_ref().expect("picker stays open after a trash");
        assert_eq!(ov.item_strings(), vec!["keep.png"]);
        assert!(ov.notice.is_empty(), "a successful trash shows no error notice");
    }

    /// A trash FAILURE (a backend that errors) LEAVES the row + shows a calm notice —
    /// the list never shrinks unless the file actually went to the Trash.
    #[test]
    fn trash_asset_failure_keeps_the_row_and_notes_the_error() {
        use std::path::Path;
        struct FailTrash;
        impl crate::assets::TrashCan for FailTrash {
            fn trash(&self, _p: &Path) -> Result<(), String> {
                Err("nope".to_string())
            }
        }
        let mut app = App::new_hermetic(None, PathBuf::from("/proj"), Config::empty());
        app.overlay = Some(crate::overlay::OverlayState::new_assets(vec![
            crate::assets::Orphan {
                rel: "assets/x.png".into(),
                name: "x.png".into(),
                parent: "assets".into(),
                size: Some(1),
            },
        ]));
        crate::assets::with_trash(Arc::new(FailTrash), || {
            app.trash_asset("assets/x.png".to_string());
        });
        let ov = app.overlay.as_ref().unwrap();
        assert_eq!(ov.item_strings(), vec!["x.png"], "a failed trash keeps the row");
        assert!(ov.notice.contains("Trash"), "a calm notice explains the failure");
    }

    // ── NO-PATH PASTE SAVES FIRST (the paste-image seam, `app/apply.rs::
    // try_paste_image`) ──────────────────────────────────────────────────────

    /// A bare SCRATCH buffer (never summoned via C-x n) with real text in it: the
    /// pre-paste save promotes it into a note rooted at `notes_root` and derives
    /// a path from its first line — the SAME name/derivation a real quick note's
    /// first autosave would produce. Proves the "gains a path under notes_root"
    /// half of the paste-image contract.
    #[test]
    fn ensure_note_named_before_paste_promotes_a_scratch_buffer_and_saves_under_notes_root() {
        use crate::fs::{FileSystem, InMemoryFs};
        let fake = Arc::new(InMemoryFs::new());
        crate::fs::with_fs(fake.clone(), || {
            let mut app = App::new(
                None,
                PathBuf::from("/proj"),
                None,
                Some(PathBuf::from("/notes")),
                Config::empty(),
            );
            assert!(!app.buffer.is_note(), "a bare launch buffer starts as plain scratch");
            assert!(app.buffer.path().is_none());
            app.buffer.set_text("My Pasted Screenshot\n\nsome body text\n");

            app.ensure_note_named_before_paste();

            assert!(app.buffer.is_note(), "promoted into a note living under notes_root");
            let path = app.buffer.path().expect("gained a path").to_path_buf();
            assert!(
                path.starts_with("/notes"),
                "the derived path lives under notes_root: {}",
                path.display()
            );
            assert_eq!(path.extension().and_then(|e| e.to_str()), Some("md"));
            // The slug came from the first non-empty line, matching the notes
            // system's own derivation (`buffer::note_stem`).
            assert!(
                path.file_stem().unwrap().to_string_lossy().contains("pasted-screenshot"),
                "filename derives from the first line: {}",
                path.display()
            );
            // The save actually landed on disk (not just an in-memory path stamp).
            assert_eq!(
                fake.read_to_string(&path).unwrap(),
                "My Pasted Screenshot\n\nsome body text\n"
            );
            // `App.file` + the title track the freshly-named note, exactly like a
            // real quick note's first autosave.
            assert_eq!(app.file.as_deref(), Some(path.as_path()));
        });
    }

    /// An ALREADY-STARTED note (`note_dir` set, still unnamed) is left pointed at
    /// its own dir — never re-promoted/re-rooted at `notes_root` a second time.
    #[test]
    fn ensure_note_named_before_paste_leaves_an_in_progress_note_dir_alone() {
        use crate::fs::InMemoryFs;
        let fake = Arc::new(InMemoryFs::new());
        crate::fs::with_fs(fake.clone(), || {
            let mut app = App::new(None, PathBuf::from("/proj"), None, Some(PathBuf::from("/notes")), Config::empty());
            app.buffer.start_note(PathBuf::from("/elsewhere"));
            app.buffer.set_text("Elsewhere Note\n");

            app.ensure_note_named_before_paste();

            let path = app.buffer.path().expect("gained a path");
            assert!(
                path.starts_with("/elsewhere"),
                "an in-progress note's own dir is respected, not overridden: {}",
                path.display()
            );
        });
    }

    /// An EMPTY buffer (no first line to derive a name from) fails the save
    /// quietly and stays path-less — the caller (`try_paste_image`) falls back to
    /// its pre-existing absolute data-root location rather than blocking the
    /// paste. Also proves the promotion side effect (now a note) survives the
    /// failed save, matching what typing-then-pausing would do from here.
    #[test]
    fn ensure_note_named_before_paste_on_an_empty_buffer_stays_path_less() {
        use crate::fs::InMemoryFs;
        let fake = Arc::new(InMemoryFs::new());
        crate::fs::with_fs(fake, || {
            let mut app = App::new(None, PathBuf::from("/proj"), None, Some(PathBuf::from("/notes")), Config::empty());
            assert_eq!(app.buffer.text(), "", "a fresh scratch buffer starts empty");

            app.ensure_note_named_before_paste();

            assert!(app.buffer.path().is_none(), "no first line to derive a name from");
            assert!(app.buffer.is_note(), "promoted regardless — matches typing-then-pausing");
        });
    }

    #[test]
    fn persist_cjk_priority_writes_the_whole_ordered_ladder_to_config() {
        // App::persist_cjk_priority (fired by Effect::OverlayAccept(CjkLang, ..)
        // after the core promotes + sets the live global) writes the WHOLE
        // ordered ladder as a TOML array and mirrors it into `self.config`.
        let _g = crate::testlock::serial();
        let fake = Arc::new(crate::fs::InMemoryFs::new().with_dir("/w/proj"));
        crate::fs::with_fs(fake, || {
            let mut config = Config::empty();
            config.path = PathBuf::from("/cfg/config.toml");
            let mut app = App::new(None, PathBuf::from("/w/proj"), None, None, config);

            // The core already promoted Korean to the front (mirrors what
            // `actions::overlay_nav`'s CjkLang accept branch does).
            crate::frontmatter::set_cjk_priority(&crate::frontmatter::promote_cjk_priority(
                crate::frontmatter::Lang::Ko,
            ));
            app.persist_cjk_priority();

            let want = vec![
                crate::frontmatter::Lang::Ko,
                crate::frontmatter::Lang::Ja,
                crate::frontmatter::Lang::ZhHans,
                crate::frontmatter::Lang::ZhHant,
            ];
            assert_eq!(app.config.cjk_priority, Some(want.clone()), "mirrored in-memory");
            let reloaded = Config::load(PathBuf::from("/cfg/config.toml"));
            assert_eq!(reloaded.cjk_priority, Some(want), "persisted to disk");

            crate::frontmatter::set_cjk_priority(&crate::frontmatter::DEFAULT_CJK_PRIORITY);
        });
    }

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
