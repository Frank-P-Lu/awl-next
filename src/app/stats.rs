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
        let now_ms = self.stats_origin.elapsed().as_millis() as u64;
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
}
