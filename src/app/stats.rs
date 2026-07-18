//! LIFETIME STATS' App-side wiring (native only — `cfg(not(target_arch =
//! "wasm32"))`, mirroring the daemon / session restore's own gate): the TRACKING
//! HOOKS the live `App` calls from its existing seams, plus the FLUSH on the
//! autosave triggers. `crate::stats` owns the pure store + injected-clock helpers
//! + the (de)serializer; this file is the seam that folds it into the live `App`.
//!
//! **The hooks (each config-gated on `stats_on()`, each native-only):**
//!  - [`Self::stats_note_keystroke`] — on the keyboard-input path
//!    (`on_keyboard_input`, past every filter): a keystroke, a printable char iff
//!    it resolved to an insert, and the capped active-writing interval since the
//!    previous press (attributed to the active theme world).
//!  - [`Self::stats_track_caret`] — at the end of `sync_view` (the one live
//!    bridge every caret move passes through): the caret's DOCUMENT-space travel,
//!    added only when the logical cursor actually moved.
//!  - [`Self::stats_touch_file`] — from `load_path`, beside `push_recent_file`:
//!    the distinct-files set.
//!  - [`Self::stats_flush`] — the atomic write, on the SAME idle/blur/switch/quit
//!    triggers the autosave engine's own flush uses.
//!
//! **Determinism:** all four live ONLY on the live `App`; the headless capture
//! never constructs one, so a `--screenshot`/`--keys` capture is STRUCTURALLY
//! incapable of touching `stats.toml` — see
//! `main::run::tests::headless_replay_never_touches_the_stats_file`.

use super::*;

impl App {
    /// Record ONE keyboard press into the odometer. `printable` is whether the
    /// press resolved to an `Action::InsertChar` (a real character written).
    /// Bumps `keystrokes` (+ `chars_typed` when printable) and folds the capped
    /// active-writing interval (see [`crate::stats::active_delta`]) into the total
    /// + the active world's bucket, stamping the current keystroke as the next
    /// interval's `last`. A no-op when the odometer is off.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn stats_note_keystroke(&mut self, printable: bool) {
        if !self.config.stats_on() {
            return;
        }
        // Session clock through the ONE time owner (origin was stamped by the
        // same `self.clock`), so a deterministic clock would govern the active-
        // writing odometer too. `RealClock` makes this `stats_origin.elapsed()`.
        let now_ms = self.clock.now().duration_since(self.stats_origin).as_millis() as u64;
        let world = crate::theme::active().name;
        self.stats
            .record_keystroke(printable, world, self.stats_last_input_ms, now_ms);
        self.stats_last_input_ms = Some(now_ms);
        self.stats_dirty = true;
    }

    /// Sample the caret and accumulate its DOCUMENT-space travel. Called at the
    /// end of `sync_view`, once the pipeline's caret target reflects this sync's
    /// cursor. Distance is added ONLY when the logical (line, col) changed since
    /// the last sample — a pure scroll or a re-layout (heading reshape) just
    /// refreshes the anchor, so stale pre-reshape coords never leak into a later
    /// real move. A no-op when the odometer is off or the GPU is not up yet
    /// (nothing to read a caret position from).
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn stats_track_caret(&mut self) {
        if !self.config.stats_on() {
            return;
        }
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let xy = gpu.pipeline.caret_doc_xy();
        let cur = self.buffer.cursor_line_col();
        if let (Some(prev_xy), Some(prev_cur)) = (self.stats_last_caret_xy, self.stats_last_cursor)
        {
            if cur != prev_cur {
                self.stats.record_caret_move(prev_xy, xy);
                self.stats_dirty = true;
            }
        }
        self.stats_last_caret_xy = Some(xy);
        self.stats_last_cursor = Some(cur);
    }

    /// Record a file OPEN into the distinct-files set (deduped). Called from
    /// `load_path`, the same door the recent-files MRU rides. A re-open of an
    /// already-seen path is inert (never re-marks the odometer dirty). A no-op
    /// when the odometer is off.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn stats_touch_file(&mut self, path: PathBuf) {
        if !self.config.stats_on() {
            return;
        }
        if self.stats.touch_file(path) {
            self.stats_dirty = true;
        }
    }

    /// Drop the caret-travel anchor across a BUFFER SWAP (file open / new note),
    /// so the first caret sample in the new document re-anchors instead of
    /// counting the jump between two documents' incomparable coordinate spaces as
    /// travel. The next `sync_view`'s `stats_track_caret` sees `None` and simply
    /// records the fresh position (no distance added).
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn stats_reset_caret_anchor(&mut self) {
        self.stats_last_caret_xy = None;
        self.stats_last_cursor = None;
    }

    /// The live odometer snapshot the HUD DISPLAY phase reads (the store + this
    /// accessor is all the next phase needs). Kept beside the tracking hooks so the
    /// HUD never reaches into a private field.
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(dead_code)]
    pub(super) fn stats_report(&self) -> &crate::stats::Stats {
        &self.stats
    }

    /// Push the LIFETIME-ODOMETER snapshot into the pipeline for the held HUD's
    /// odometer rows (characters / writing time / files touched / caret travel /
    /// most-lived-in world). Called every `sync_view` — the field is cheap to hold
    /// and only read when the HUD is summoned. When the odometer is OFF we push
    /// `None`, so the rows honestly read as the `"—"` placeholder rather than a
    /// misleading row of zeros. This is the LIVE-ONLY seam that keeps a `--hud`
    /// capture (which never calls `sync_view`) showing placeholders — mirroring the
    /// retired `set_hud_session`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn stats_sync_hud(&mut self) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let snapshot = if self.config.stats_on() {
            Some(crate::hud::HudStats {
                chars_typed: self.stats.chars_typed,
                active_writing_ms: self.stats.active_writing_ms,
                files_touched: self.stats.files_touched_count(),
                caret_distance_px: self.stats.caret_distance_px,
                world: self.stats.most_used_world().map(|(name, _)| name.to_string()),
            })
        } else {
            None
        };
        gpu.pipeline.set_hud_stats(snapshot);
    }

    /// Push the DISCOVERABILITY surfaces' content into the pipeline every `sync_view`
    /// (LIVE-ONLY, native-only): the HOLD-⌘ peek's personalized rows and the Keybindings
    /// footer's top-3 tips, both derived from the SILENT USAGE LEDGER's graduation
    /// ranking (the SAME query, top-6 for the peek / top-3 for the footer). A headless
    /// capture never calls this, so the pipeline's peek rows stay EMPTY (→ the curated
    /// STARTER SIX renders, deterministic) and the footer tips stay empty (→ the footer
    /// is hidden, a Keybindings capture byte-identical). Cheap: the ledger is catalog-
    /// sized, so ranking it per sync is negligible (like `stats_sync_hud`).
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn sync_discoverability(&mut self) {
        let peek_rows = self.peek_rows_from_ledger();
        // The footer tips ride ONLY while the Keybindings overlay is open (so no OTHER
        // flat picker ever grows a footer); empty otherwise → the footer hides.
        let tips = if self.overlay_is_keybindings() {
            self.keybinding_tips_from_ledger()
        } else {
            Vec::new()
        };
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.pipeline.set_peek_rows(peek_rows);
            gpu.pipeline.set_keybindings_tips(tips);
        }
    }

    /// The HOLD-⌘ peek's personalized rows from the ledger: the top-[`crate::peek::PEEK_ROWS`]
    /// graduation candidates resolved to chord+name (a candidate lacking a native chord is
    /// dropped by `peek_row_for_slug`, though the ranking already excludes those). Empty on
    /// a fresh install / with tracking off → the pipeline falls back to the starter six.
    /// Pure over `self.stats`, so a fake ledger pins it without a GPU.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn peek_rows_from_ledger(&self) -> Vec<crate::peek::PeekRow> {
        self.stats
            .graduation_candidates(crate::commands::has_native_chord, crate::peek::PEEK_ROWS)
            .iter()
            .filter_map(|(slug, _)| crate::commands::peek_row_for_slug(slug))
            .collect()
    }

    /// The Keybindings footer's "your top 3" tip lines from the ledger: each a
    /// `"⌘O  Go to file"` one-liner (chord + two spaces + name) over the top-3 graduation
    /// candidates. Pure over `self.stats`, so a fake ledger pins the content without a GPU.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn keybinding_tips_from_ledger(&self) -> Vec<String> {
        self.stats
            .graduation_candidates(crate::commands::has_native_chord, 3)
            .iter()
            .filter_map(|(slug, _)| crate::commands::peek_row_for_slug(slug))
            .map(|r| format!("{}  {}", r.chord, r.name))
            .collect()
    }

    /// Whether the currently-summoned overlay (if any) is the Keybindings rebind menu —
    /// the gate for pushing the footer tips (the footer belongs to that one picker).
    #[cfg(not(target_arch = "wasm32"))]
    fn overlay_is_keybindings(&self) -> bool {
        self.overlay
            .as_ref()
            .map(|o| o.kind == crate::overlay::OverlayKind::Keybindings)
            .unwrap_or(false)
    }

    /// Record ONE command dispatch into the SILENT USAGE LEDGER, attributed to the
    /// `door` it came through (chord / palette / menu). Called at the TOP of
    /// [`Self::apply`] — the ONE seam every door funnels through (a keyboard chord, the
    /// palette's `Effect::RunAction` re-dispatch, and the macOS menu handler all reach
    /// `apply`), so all three attribute here without a parallel path, and the truly-hot
    /// typing / motion path is filtered for free (`slug_for_action` yields `None` for a
    /// non-catalog action, allocating nothing). Marks the store dirty so the next
    /// autosave-trigger flush persists it beside the lifetime odometer in the same
    /// `stats.toml`. A no-op when the odometer is off.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn ledger_note_dispatch(
        &mut self,
        action: &crate::keymap::Action,
        door: crate::stats::Door,
    ) {
        if !self.config.stats_on() {
            return;
        }
        // MOTIONS never reach the ledger, even the catalog-listed navigation ones
        // (word / line / document motion joined the catalog 2026-07-10 to become
        // REBINDABLE — see `commands.rs`'s module doc): navigation is not a
        // "command" for the discoverability ledger, and without this gate every
        // ⌥→ / Cmd-← press would key a ledger row AND dirty the store (an idle
        // flush after mere cursor travel). The catalog-membership filter below
        // still drops typing / prefix / arrow motions for free.
        if action.is_motion() {
            return;
        }
        let Some(slug) = crate::commands::slug_for_action(action) else {
            return;
        };
        self.stats.record_command(slug, door);
        self.stats_dirty = true;
    }

    /// Flush the odometer to disk ATOMICALLY, on the SAME idle/blur/switch/quit
    /// triggers the autosave engine's own flush uses. A no-op when the feature is
    /// off OR nothing has changed since the last flush (the `stats_dirty` gate, so
    /// a quiet blur/quit writes nothing). Errors go to stderr, never disrupt.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn stats_flush(&mut self) {
        if !self.config.stats_on() || !self.stats_dirty {
            return;
        }
        if let Err(e) = crate::stats::save(&crate::stats::stats_path(), &self.stats) {
            eprintln!("stats save failed: {e}");
        }
        self.stats_dirty = false;
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn keystrokes_and_chars_accrue_then_flush_round_trips() {
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let mut app = App::new(None, PathBuf::from("/n"), None, None, Config::empty());
            // Three presses: two printable inserts, one motion.
            app.stats_note_keystroke(true);
            app.stats_note_keystroke(true);
            app.stats_note_keystroke(false);
            assert_eq!(app.stats.keystrokes, 3);
            assert_eq!(app.stats.chars_typed, 2, "only the printable presses count as chars");
            assert!(app.stats_dirty, "increments mark the store dirty");

            app.stats_flush();
            assert!(!app.stats_dirty, "flush clears the dirty flag");
            let saved = crate::stats::load(&crate::stats::stats_path());
            assert_eq!(saved.keystrokes, 3);
            assert_eq!(saved.chars_typed, 2);
        });
    }

    #[test]
    fn touch_file_records_distinct_opens_and_dedupes() {
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let mut app = App::new(None, PathBuf::from("/n"), None, None, Config::empty());
            app.stats_touch_file(PathBuf::from("/n/a.md"));
            app.stats_touch_file(PathBuf::from("/n/b.md"));
            app.stats_touch_file(PathBuf::from("/n/a.md")); // a re-open
            assert_eq!(app.stats.files_touched_count(), 2, "distinct count, not open count");
        });
    }

    #[test]
    fn flush_is_a_no_op_when_nothing_changed() {
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let mut app = App::new(None, PathBuf::from("/n"), None, None, Config::empty());
            // No increments yet: a flush must not even create the file.
            app.stats_flush();
            assert!(
                crate::fs::active().read(&crate::stats::stats_path()).is_err(),
                "a clean flush writes nothing"
            );
        });
    }

    #[test]
    fn kill_switch_off_tracks_nothing_and_never_writes() {
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let cfg = Config { stats: Some(false), ..Config::empty() };
            let mut app = App::new(None, PathBuf::from("/n"), None, None, cfg);
            app.stats_note_keystroke(true);
            app.stats_touch_file(PathBuf::from("/n/a.md"));
            assert_eq!(app.stats.keystrokes, 0, "off: no tracking");
            assert!(!app.stats_dirty);
            app.stats_flush();
            assert!(
                crate::fs::active().read(&crate::stats::stats_path()).is_err(),
                "off: never writes stats.toml"
            );
        });
    }

    #[test]
    fn ledger_attributes_doors_by_the_dispatched_action_and_round_trips() {
        use crate::keymap::Action;
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let mut app = App::new(None, PathBuf::from("/n"), None, None, Config::empty());
            // A catalog command dispatched through each of the three doors.
            app.ledger_note_dispatch(&Action::OpenGoto, crate::stats::Door::Chord);
            app.ledger_note_dispatch(&Action::OpenGoto, crate::stats::Door::Chord);
            app.ledger_note_dispatch(&Action::OpenGoto, crate::stats::Door::Palette);
            app.ledger_note_dispatch(&Action::OpenThemeMenu, crate::stats::Door::Menu);
            // A NON-catalog action (arrow motion / self-insert) keys no row — the hot
            // path — and a CATALOG-listed navigation motion (rebindable since
            // 2026-07-10) is gated out by `is_motion` just the same: cursor travel
            // never keys a ledger row nor dirties the store.
            app.ledger_note_dispatch(&Action::ForwardChar, crate::stats::Door::Chord);
            app.ledger_note_dispatch(&Action::InsertChar('z'), crate::stats::Door::Chord);
            app.ledger_note_dispatch(&Action::ForwardWord, crate::stats::Door::Chord);
            app.ledger_note_dispatch(&Action::LineStart, crate::stats::Door::Chord);

            let goto = app.stats.command_counts("go_to_file");
            assert_eq!((goto.chord, goto.palette, goto.menu), (2, 1, 0));
            let theme = app.stats.command_counts("switch_theme");
            assert_eq!((theme.chord, theme.palette, theme.menu), (0, 0, 1));
            assert_eq!(app.stats.command_usage.len(), 2, "only catalog commands keyed rows");
            assert!(app.stats_dirty, "a recorded dispatch marks the store dirty");

            // Persists into (and reloads from) the SAME stats.toml as the odometer.
            app.stats_flush();
            let saved = crate::stats::load(&crate::stats::stats_path());
            assert_eq!(saved.command_usage, app.stats.command_usage);
        });
    }

    #[test]
    fn ledger_graduation_candidates_wire_through_the_real_catalog() {
        use crate::keymap::Action;
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let mut app = App::new(None, PathBuf::from("/n"), None, None, Config::empty());
            // Reached repeatedly via the palette; Go to file… HAS a native chord (Cmd-O).
            for _ in 0..4 {
                app.ledger_note_dispatch(&Action::OpenGoto, crate::stats::Door::Palette);
            }
            // Keep version is palette-only (no native chord) — must be excluded even
            // though it is the most-used slow-door command here.
            for _ in 0..9 {
                app.ledger_note_dispatch(&Action::KeepVersion, crate::stats::Door::Palette);
            }
            // The candidate query wired through the catalog's own `has_native_chord`.
            let cands = app.stats.graduation_candidates(crate::commands::has_native_chord, 5);
            let slugs: Vec<&str> = cands.iter().map(|(s, _)| s.as_str()).collect();
            assert_eq!(slugs, vec!["go_to_file"], "chordless Keep version excluded");
            assert!(!app.stats.is_graduated("go_to_file"), "not yet graduated on slow-door use");

            // Now learn the Cmd-O chord GRADUATION_N times: it drops off the candidates.
            for _ in 0..crate::stats::GRADUATION_N {
                app.ledger_note_dispatch(&Action::OpenGoto, crate::stats::Door::Chord);
            }
            assert!(app.stats.is_graduated("go_to_file"), "chord in the fingers now");
            assert!(
                app.stats
                    .graduation_candidates(crate::commands::has_native_chord, 5)
                    .is_empty(),
                "a graduated command is no longer a candidate"
            );
        });
    }

    #[test]
    fn discoverability_surfaces_rank_slow_door_use_from_a_fake_ledger() {
        use crate::keymap::Action;
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let mut app = App::new(None, PathBuf::from("/n"), None, None, Config::empty());
            // A fake ledger: three native-chord commands reached via slow doors, ranked
            // by slow-door count (Go to file 4 > Switch theme 2 > Version history 1).
            for _ in 0..4 {
                app.ledger_note_dispatch(&Action::OpenGoto, crate::stats::Door::Palette);
            }
            app.ledger_note_dispatch(&Action::OpenThemeMenu, crate::stats::Door::Palette);
            app.ledger_note_dispatch(&Action::OpenThemeMenu, crate::stats::Door::Menu);
            app.ledger_note_dispatch(&Action::OpenHistory, crate::stats::Door::Palette);
            // Keep version is palette-only (no native chord) → never a peek/footer row
            // even though it's reached via a slow door.
            for _ in 0..9 {
                app.ledger_note_dispatch(&Action::KeepVersion, crate::stats::Door::Palette);
            }

            // CONVENTION-PARAMETRIC expected chord labels: `peek_row_for_slug`/
            // `peek_rows_from_ledger` resolve each chord through
            // `commands::resolved_native_label(c, Convention::current())` — Mac ⌘
            // glyphs on `Convention::Mac`, Linux word labels (`"Ctrl+O"`) on
            // `Convention::Linux` (see `convention.rs`'s doc + the AWL_CONVENTION_FORCE
            // dev knob CI's linux runner exercises via the real `cfg(target_os)` path).
            // Computing the expectation through the SAME resolver — rather than a
            // hardcoded mac-only literal — keeps this law true on EITHER convention,
            // never just whichever one happens to be ambient.
            let label_for = |action: &Action| -> String {
                let c = crate::commands::COMMANDS.iter().find(|c| c.action == *action).unwrap();
                crate::commands::resolved_native_label(c, crate::convention::Convention::current())
            };
            let goto_chord = label_for(&Action::OpenGoto);
            let theme_chord = label_for(&Action::OpenThemeMenu);
            let history_chord = label_for(&Action::OpenHistory);

            // The PEEK rows: chord+name, ranked, chordless Keep version excluded.
            let peek = app.peek_rows_from_ledger();
            let names: Vec<&str> = peek.iter().map(|r| r.name.as_str()).collect();
            assert_eq!(names, vec!["Go to file", "Switch theme", "Version history"]);
            assert_eq!(peek[0].chord, goto_chord);
            assert_eq!(peek[1].chord, theme_chord);

            // The FOOTER tips: the SAME ranking, top 3, as "<chord>  <name>" one-liners.
            let tips = app.keybinding_tips_from_ledger();
            assert_eq!(
                tips,
                vec![
                    format!("{goto_chord}  Go to file"),
                    format!("{theme_chord}  Switch theme"),
                    format!("{history_chord}  Version history"),
                ]
            );
        });
    }

    #[test]
    fn discoverability_surfaces_are_empty_on_a_fresh_ledger() {
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let app = App::new(None, PathBuf::from("/n"), None, None, Config::empty());
            // Nothing tracked yet → no personalized rows / tips. The pipeline then falls
            // back to the curated starter six for the peek, and the footer hides.
            assert!(app.peek_rows_from_ledger().is_empty(), "fresh ledger: no peek rows");
            assert!(app.keybinding_tips_from_ledger().is_empty(), "fresh ledger: no footer tips");
        });
    }

    #[test]
    fn ledger_off_records_no_command_usage() {
        use crate::keymap::Action;
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let cfg = Config { stats: Some(false), ..Config::empty() };
            let mut app = App::new(None, PathBuf::from("/n"), None, None, cfg);
            app.ledger_note_dispatch(&Action::OpenGoto, crate::stats::Door::Chord);
            assert!(app.stats.command_usage.is_empty(), "off: the ledger stays empty");
            assert!(!app.stats_dirty);
        });
    }
}
