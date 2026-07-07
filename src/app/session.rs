//! SESSION RESTORE's App-side wiring (native only — `cfg(not(target_arch =
//! "wasm32"))`, mirroring the single-instance daemon's own gate): the CAPTURE
//! half (`session_flush`, called from the same blur+quit doors the autosave
//! engine's own flush uses) and the RESTORE half (`apply_session_restore`,
//! called once from `App::new`). `crate::session` owns the pure data model +
//! (de)serializer + window-frame clamp math; this file is the seam that folds
//! it into the live `App` — the buffer registry, the active buffer, and (on
//! `resumed()`, in `app.rs`) the window frame.
//!
//! **Native-only scope trim (TASTE CALL, logged):** the whole engine is gated
//! off on wasm, like the daemon. A browser tab has no discrete "quit, then
//! relaunch a new process" — its persistence story is the existing scratch
//! stash (which already survives a reload via `localStorage`) plus, if ever
//! wanted, a page-lifecycle hook; that is out of scope here. This keeps the
//! window-frame half (genuinely native-only — a `<canvas>` has no OS frame to
//! remember) and the open-file-set half under ONE gate instead of splitting
//! the feature down the middle.
//!
//! **Determinism:** both halves live ONLY on the live `App`; the headless
//! `--screenshot`/`--keys` capture never constructs one (`main::run::replay_keys`
//! / `load_buffer` build a bare `Buffer` directly), so a capture is
//! STRUCTURALLY incapable of touching the session file — see
//! `main::run::tests::headless_replay_never_touches_the_session_file`.

use super::*;

impl App {
    /// SESSION FLUSH — the CAPTURE half's one door (mirrors the autosave
    /// engine's `autosave_flush`): snapshot every open PATHED buffer's path +
    /// cursor + scroll (the active one, via `self.file`/`self.buffer`, plus
    /// every backgrounded one still in the registry), which one is active,
    /// and the native window frame, then write it atomically beside the
    /// scratch stash. Config-gated (`session_restore`, default ON — the SAME
    /// flag also gates the restore half, so turning it off makes the feature
    /// vanish both ways); a no-op when off.
    ///
    /// Called from the SAME two triggers the autosave engine's blur/quit
    /// flushes use (window blur + `exiting()`) — deliberately NOT idle or
    /// file-switch (a TASTE CALL, logged): the open-file SET changes rarely
    /// enough that the coarser two triggers are plenty, and capturing the
    /// window frame on every idle tick / file switch would mean writing it on
    /// every resize-drag frame too. The no-path SCRATCH buffer is never a
    /// member of `buffers` (it keeps its own persistent stash — composing,
    /// not duplicating, per the module doc).
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn session_flush(&mut self) {
        if !self.config.session_restore_on() {
            return;
        }
        let mut buffers = Vec::new();
        if let Some(path) = self.file.clone() {
            let (line, col) = self.buffer.cursor_line_col();
            buffers.push((
                path,
                crate::session::BufferPos { line, col, scroll: self.scroll_lines },
            ));
        }
        for (_key, entry) in self.buffer_registry.iter() {
            let Some(path) = entry.buffer.path() else {
                continue; // the parked Scratch entry (if any): not a session member
            };
            let (line, col) = entry.buffer.cursor_line_col();
            buffers.push((
                path.to_path_buf(),
                crate::session::BufferPos { line, col, scroll: entry.extra.scroll_lines },
            ));
        }
        // Best-effort: `outer_position` can fail (e.g. some Wayland compositors
        // refuse it) — degrade to no window frame rather than skip the whole
        // flush, mirroring every other "never let a live-only quirk disrupt the
        // rest of the save" pattern in this codebase.
        let window = self.gpu.as_ref().and_then(|gpu| {
            let pos = gpu.window.outer_position().ok()?;
            let size = gpu.window.inner_size();
            Some(crate::session::WindowFrame {
                x: pos.x,
                y: pos.y,
                width: size.width,
                height: size.height,
            })
        });
        let state = crate::session::SessionState { active: self.file.clone(), buffers, window };
        if let Err(e) = crate::session::save(&crate::session::session_path(), &state) {
            eprintln!("session save failed: {e}");
        }
    }

    /// SESSION RESTORE's apply half, called ONCE from `App::new` (after the
    /// scratch-stash restore has already picked `self.buffer`/`self.file`).
    /// `file_arg_given` is whether THIS launch named an explicit file:
    ///
    ///  - a BARE launch (`false`): the session's own remembered `active`
    ///    file (if it SURVIVES — still exists on disk) becomes the active
    ///    buffer, its cursor/scroll restored; every OTHER surviving file is
    ///    parked into the buffer registry (backgrounded, cursor/scroll
    ///    restored too). Composes with — never replaces — the scratch-stash
    ///    outcome: a session with no `active` (or an `active` that vanished)
    ///    leaves `self.buffer`/`self.file` exactly as the stash restore left
    ///    them, and its OTHER survivors still get parked.
    ///  - a launch WITH a file argument (`true`, TASTE CALL — logged in
    ///    CLAUDE.md): that file STAYS active (never overridden), but the
    ///    rest of the session still restores BEHIND it into the registry —
    ///    the daemon hands a `--wait`/plain launch off into a long-lived
    ///    instance, so the session belongs to the INSTANCE, not to any one
    ///    launch's argument.
    ///
    /// A vanished file (deleted/moved since the last session) is silently
    /// skipped (`crate::session::existing_buffers`); the kill-switch
    /// (`config.session_restore_on()`) makes this whole function a no-op,
    /// including the window-frame stash into `self.restored_window`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn apply_session_restore(&mut self, file_arg_given: bool) {
        if !self.config.session_restore_on() {
            return;
        }
        let state = crate::session::load(&crate::session::session_path());
        self.restored_window = state.window;
        let survivors = crate::session::existing_buffers(&state);
        if survivors.is_empty() {
            return;
        }
        // A BARE launch may adopt the session's own `active` file (if it
        // survived); a launch WITH a file argument keeps that file active no
        // matter what the session says.
        let active_path = if file_arg_given {
            None
        } else {
            state
                .active
                .as_ref()
                .and_then(|p| survivors.iter().find(|(sp, _)| sp == p).cloned())
        };
        if let Some((path, pos)) = &active_path {
            let mut buffer = Buffer::from_file(path);
            Self::apply_restored_pos(&mut buffer, *pos);
            self.disk_mtime = Self::disk_mtime_of(path);
            self.doc_saved_version = Some(buffer.version());
            self.caret_synced_version = buffer.version();
            self.scroll_lines = pos.scroll;
            self.buffer = buffer;
            self.file = Some(path.clone());
        }
        for (path, pos) in &survivors {
            if active_path.as_ref().map(|(p, _)| p) == Some(path) {
                continue; // just became the active buffer above
            }
            if self.file.as_deref() == Some(path.as_path()) {
                continue; // already this launch's CLI-argument file
            }
            let mut buffer = Buffer::from_file(path);
            Self::apply_restored_pos(&mut buffer, *pos);
            let extra = files::BufferExtra {
                scroll_lines: pos.scroll,
                doc_saved_version: Some(buffer.version()),
                disk_mtime: Self::disk_mtime_of(path),
                caret_synced_version: buffer.version(),
                ..Default::default()
            };
            self.buffer_registry
                .park(crate::buffers::BufferKey::path(path), crate::buffers::Entry { buffer, extra });
        }
    }

    /// Place `buffer`'s cursor at the remembered (line, col), clamped exactly
    /// like `App::jump_to_line` does (`Buffer::line_col_to_char` already
    /// clamps both the line and the column). Shared by both restore arms
    /// above so a freshly-loaded restored buffer always lands its cursor the
    /// same way.
    #[cfg(not(target_arch = "wasm32"))]
    fn apply_restored_pos(buffer: &mut Buffer, pos: crate::session::BufferPos) {
        let idx = buffer.line_col_to_char(pos.line, pos.col);
        buffer.clear_mark();
        buffer.set_cursor(idx);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Arc;

    /// Build a `SessionState` the way `session_flush` would, sparing tests the
    /// need to fully replicate its exact shape by hand.
    fn state(
        active: Option<&str>,
        buffers: &[(&str, usize, usize, usize)],
    ) -> crate::session::SessionState {
        crate::session::SessionState {
            active: active.map(PathBuf::from),
            buffers: buffers
                .iter()
                .map(|(p, line, col, scroll)| {
                    (
                        PathBuf::from(p),
                        crate::session::BufferPos { line: *line, col: *col, scroll: *scroll },
                    )
                })
                .collect(),
            window: None,
        }
    }

    #[test]
    fn bare_launch_restores_active_and_parks_the_rest() {
        let fake = Arc::new(
            crate::fs::InMemoryFs::new()
                .with_file("/n/a.md", "one\ntwo\nthree\n")
                .with_file("/n/b.md", "alpha\nbeta\n"),
        );
        crate::fs::with_fs(fake, || {
            let session_path = crate::session::session_path();
            let s = state(Some("/n/a.md"), &[("/n/a.md", 1, 2, 3), ("/n/b.md", 0, 1, 0)]);
            crate::session::save(&session_path, &s).unwrap();

            let mut app = App::new(None, PathBuf::from("/n"), None, None, Config::empty());

            assert_eq!(app.file, Some(PathBuf::from("/n/a.md")), "session active file wins");
            assert_eq!(app.buffer.cursor_line_col(), (1, 2), "cursor restored");
            assert_eq!(app.scroll_lines, 3, "scroll restored");
            assert_eq!(app.buffer_registry.len(), 1, "the OTHER survivor is parked");
            assert!(app.buffer_registry.contains(&crate::buffers::BufferKey::path(Path::new("/n/b.md"))));

            // Switching to it finds the restored cursor/scroll, not a fresh 0,0.
            app.load_path(PathBuf::from("/n/b.md"));
            assert_eq!(app.buffer.cursor_line_col(), (0, 1));
            assert_eq!(app.scroll_lines, 0);
        });
    }

    #[test]
    fn file_argument_launch_stays_active_but_restores_the_rest_behind_it() {
        let fake = Arc::new(
            crate::fs::InMemoryFs::new()
                .with_file("/n/a.md", "one\ntwo\n")
                .with_file("/n/b.md", "alpha\nbeta\n"),
        );
        crate::fs::with_fs(fake, || {
            let session_path = crate::session::session_path();
            // The session's own "active" was b.md, but THIS launch names a.md.
            let s = state(Some("/n/b.md"), &[("/n/b.md", 1, 0, 2)]);
            crate::session::save(&session_path, &s).unwrap();

            let app = App::new(
                Some(PathBuf::from("/n/a.md")),
                PathBuf::from("/n"),
                None,
                None,
                Config::empty(),
            );

            assert_eq!(app.file, Some(PathBuf::from("/n/a.md")), "the CLI file argument wins");
            assert_eq!(
                app.buffer.cursor_line_col(),
                (0, 0),
                "the CLI-argument file opens at its own start, not the session's remembered cursor"
            );
            assert_eq!(app.buffer_registry.len(), 1, "b.md still restores BEHIND the active file");
            assert!(app.buffer_registry.contains(&crate::buffers::BufferKey::path(Path::new("/n/b.md"))));
        });
    }

    #[test]
    fn vanished_session_file_is_silently_skipped() {
        let fake = Arc::new(crate::fs::InMemoryFs::new().with_file("/n/keep.md", "x\n"));
        crate::fs::with_fs(fake, || {
            let session_path = crate::session::session_path();
            let s = state(Some("/n/gone.md"), &[("/n/gone.md", 5, 5, 5), ("/n/keep.md", 0, 0, 0)]);
            crate::session::save(&session_path, &s).unwrap();

            let app = App::new(None, PathBuf::from("/n"), None, None, Config::empty());

            // "/n/gone.md" never existed: it must never become active, and must
            // never appear in the registry.
            assert_ne!(app.file, Some(PathBuf::from("/n/gone.md")));
            assert!(!app
                .buffer_registry
                .contains(&crate::buffers::BufferKey::path(Path::new("/n/gone.md"))));
            // "keep.md" survives and gets parked (it wasn't the session's
            // `active`, which vanished, so it's just a background survivor —
            // and since the session named no SURVIVING active file, the
            // scratch-stash outcome for `self.buffer`/`self.file` stands).
            assert!(app
                .buffer_registry
                .contains(&crate::buffers::BufferKey::path(Path::new("/n/keep.md"))));
        });
    }

    #[test]
    fn kill_switch_off_restores_nothing_and_leaves_no_registry_entries() {
        let fake = Arc::new(
            crate::fs::InMemoryFs::new()
                .with_file("/n/a.md", "one\ntwo\n")
                .with_file("/n/b.md", "alpha\n"),
        );
        crate::fs::with_fs(fake, || {
            let session_path = crate::session::session_path();
            let s = state(Some("/n/a.md"), &[("/n/a.md", 1, 0, 0), ("/n/b.md", 0, 0, 0)]);
            crate::session::save(&session_path, &s).unwrap();

            let cfg = Config { session_restore: Some(false), ..Config::empty() };
            let app = App::new(None, PathBuf::from("/n"), None, None, cfg);

            assert_eq!(app.file, None, "the kill-switch leaves the plain scratch buffer active");
            assert_eq!(app.buffer_registry.len(), 0, "nothing is parked when the switch is off");
            assert_eq!(app.restored_window, None, "no window frame is restored either");
        });
    }

    #[test]
    fn session_flush_writes_the_active_and_backgrounded_buffers_then_round_trips() {
        let fake = Arc::new(
            crate::fs::InMemoryFs::new()
                .with_file("/n/a.md", "one\ntwo\nthree\n")
                .with_file("/n/b.md", "alpha\nbeta\n"),
        );
        crate::fs::with_fs(fake, || {
            let mut app = App::new(
                Some(PathBuf::from("/n/a.md")),
                PathBuf::from("/n"),
                None,
                None,
                Config::empty(),
            );
            app.buffer.set_cursor(app.buffer.line_col_to_char(2, 1));
            app.scroll_lines = 7;
            app.load_path(PathBuf::from("/n/b.md")); // a.md is now backgrounded

            app.session_flush();

            let saved = crate::session::load(&crate::session::session_path());
            assert_eq!(saved.active, Some(PathBuf::from("/n/b.md")));
            let a_pos = saved
                .buffers
                .iter()
                .find(|(p, _)| p == Path::new("/n/a.md"))
                .map(|(_, pos)| *pos)
                .expect("a.md was flushed as a backgrounded buffer");
            assert_eq!(a_pos, crate::session::BufferPos { line: 2, col: 1, scroll: 7 });
        });
    }
}
