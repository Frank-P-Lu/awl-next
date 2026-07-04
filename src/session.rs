//! SESSION RESTORE — the persisted "where you left off" state (which files
//! were open, which one was active, each file's remembered cursor/scroll, and
//! the native window frame), so a plain relaunch reopens the workspace
//! roughly as it was left. This module is the PURE data model + (de)serializer
//! + clamp math; the App-side wiring (when to capture / restore, how it folds
//! into `App::new` and the buffer registry) lives in `app/session.rs`.
//!
//! **Where it lives:** beside the scratch stash
//! (`fs::data_root()/session.toml`), NOT inside `config.toml` — the config
//! file is the user's own hand-edited settings (theme, keybindings, sticky
//! prefs); this is MACHINE STATE the app itself reads and writes every run,
//! so it gets its own file (mirrors [`crate::fs::scratch_stash_path`] exactly
//! — same directory, same "beside it, not folded into it" reasoning).
//!
//! **Format:** a hand-rolled TOML writer ([`to_toml`], mirrors
//! `capture/sidecar.rs`'s hand-rolled JSON — no serde) paired with the
//! crate's existing `toml` PARSER (the same one `config.rs` already uses via
//! [`from_toml`]), so reading stays lenient — a malformed/missing file
//! degrades to an empty [`SessionState`], never a crash — without a second
//! dependency (the `toml` crate loads with only the `parse` feature; it has
//! no serializer to reach for).
//!
//! **Determinism (CRITICAL):** every read/write goes through the `App`-side
//! seam in `app/session.rs` (`session_flush` / `apply_session_restore`),
//! which is native-only and lives only on the live `App` (armed by the SAME
//! blur+quit triggers the autosave engine uses) — so a headless
//! `--screenshot`/`--keys` capture, which never constructs an `App`, is
//! STRUCTURALLY incapable of reading or writing this file. See the tripwire
//! test `main::run::tests::headless_replay_never_touches_the_session_file`.

use std::path::{Path, PathBuf};

/// One open buffer's remembered position: SMALL ints, never a content
/// snapshot — the file on disk is still the source of truth. `line`/`col`
/// are the 0-based cursor coordinates
/// ([`crate::buffer::Buffer::cursor_line_col`]); `scroll` is the visual-row
/// scroll offset (`App::scroll_lines` / `app::files::BufferExtra::scroll_lines`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BufferPos {
    pub line: usize,
    pub col: usize,
    pub scroll: usize,
}

/// The native window's last-known FRAME: OUTER position (top-left, physical
/// px — where it sat on screen) plus INNER size (physical px — what the
/// layout wrapped to). Mixing outer-position with inner-size is a small,
/// deliberate imprecision (a title bar's few px of decoration are not worth
/// plumbing outer-vs-inner separately for both axes) — a TASTE CALL, logged
/// in `app/session.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowFrame {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// A monitor's bounds in the SAME physical-pixel space as [`WindowFrame`] —
/// the pure input [`clamp_frame_to_screens`] clamps against, kept independent
/// of `winit::monitor::MonitorHandle` so the clamp math is unit-testable with
/// no window / event-loop at all. The App-side call site maps
/// `ActiveEventLoop::available_monitors()` into these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl ScreenRect {
    fn contains_point(&self, x: i32, y: i32) -> bool {
        x >= self.x
            && x < self.x + self.width as i32
            && y >= self.y
            && y < self.y + self.height as i32
    }
}

/// The whole persisted session. `active` names which open file was in the
/// foreground; `buffers` is every open PATHED file (the no-path scratch
/// buffer is deliberately never a member here — it keeps its own persistent
/// stash, see the module doc) paired with its remembered [`BufferPos`];
/// `window` is the native frame, `None` when nothing was ever captured
/// (wasm, or a pre-this-round session file).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionState {
    pub active: Option<PathBuf>,
    pub buffers: Vec<(PathBuf, BufferPos)>,
    pub window: Option<WindowFrame>,
}

/// Where the session file lives: beside the scratch stash, same data root.
pub fn session_path() -> PathBuf {
    crate::fs::data_root().join("session.toml")
}

/// Load the persisted session from `path` through the active `FileSystem`
/// backend. A MISSING or unparseable file degrades to [`SessionState::default`]
/// (empty) — never an error, mirroring [`crate::config::Config::load`]'s
/// leniency.
pub fn load(path: &Path) -> SessionState {
    match crate::fs::active().read_to_string(path) {
        Ok(src) => from_toml(&src),
        Err(_) => SessionState::default(),
    }
}

/// Persist `state` to `path` ATOMICALLY (temp-sibling + rename, via
/// [`crate::fs::write_atomic`] — the same primitive the autosave engine and
/// the scratch stash use).
pub fn save(path: &Path, state: &SessionState) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = crate::fs::active().create_dir_all(parent);
    }
    crate::fs::write_atomic(path, to_toml(state).as_bytes())
}

/// Serialize `state` to the on-disk TOML shape — pure, no fs. Hand-rolled
/// (mirrors `capture/sidecar.rs`'s hand-rolled JSON) since the crate's `toml`
/// dependency loads with only the `parse` feature (no serializer built in);
/// reading it back goes through the real `toml` parser via [`from_toml`], so
/// the two halves never have to hand-agree on escaping rules.
pub fn to_toml(state: &SessionState) -> String {
    let mut out = String::new();
    if let Some(active) = &state.active {
        out.push_str(&format!("active = {}\n", quote(active)));
    }
    if let Some(w) = &state.window {
        out.push_str("\n[window]\n");
        out.push_str(&format!("x = {}\n", w.x));
        out.push_str(&format!("y = {}\n", w.y));
        out.push_str(&format!("width = {}\n", w.width));
        out.push_str(&format!("height = {}\n", w.height));
    }
    for (path, pos) in &state.buffers {
        out.push_str("\n[[buffer]]\n");
        out.push_str(&format!("path = {}\n", quote(path)));
        out.push_str(&format!("line = {}\n", pos.line));
        out.push_str(&format!("col = {}\n", pos.col));
        out.push_str(&format!("scroll = {}\n", pos.scroll));
    }
    out
}

/// A path as a quoted + escaped TOML basic string.
fn quote(p: &Path) -> String {
    let s = p.display().to_string();
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Parse the on-disk TOML shape back into a [`SessionState`] — pure, no fs.
/// LENIENT throughout (mirrors `Config::load`): an unparseable document, a
/// missing/wrong-typed field, or a malformed `[[buffer]]` entry is simply
/// skipped rather than erroring, so a half-written or hand-edited session
/// file never blocks a launch.
pub fn from_toml(src: &str) -> SessionState {
    let mut state = SessionState::default();
    let Ok(table) = src.parse::<toml::Table>() else {
        return state;
    };
    if let Some(s) = table.get("active").and_then(|v| v.as_str()) {
        state.active = Some(PathBuf::from(s));
    }
    if let Some(w) = table.get("window").and_then(|v| v.as_table()) {
        let x = w.get("x").and_then(|v| v.as_integer());
        let y = w.get("y").and_then(|v| v.as_integer());
        let width = w.get("width").and_then(|v| v.as_integer());
        let height = w.get("height").and_then(|v| v.as_integer());
        if let (Some(x), Some(y), Some(width), Some(height)) = (x, y, width, height) {
            state.window = Some(WindowFrame {
                x: x as i32,
                y: y as i32,
                width: width.max(0) as u32,
                height: height.max(0) as u32,
            });
        }
    }
    if let Some(arr) = table.get("buffer").and_then(|v| v.as_array()) {
        for entry in arr {
            let Some(t) = entry.as_table() else { continue };
            let Some(path) = t.get("path").and_then(|v| v.as_str()) else { continue };
            let as_usize = |t: &toml::Table, key: &str| {
                t.get(key).and_then(|v| v.as_integer()).unwrap_or(0).max(0) as usize
            };
            let pos = BufferPos {
                line: as_usize(t, "line"),
                col: as_usize(t, "col"),
                scroll: as_usize(t, "scroll"),
            };
            state.buffers.push((PathBuf::from(path), pos));
        }
    }
    state
}

/// The survivors of `state.buffers`: entries whose path still `exists()`
/// through the active `FileSystem` backend — the vanished-file SKIP the
/// restore applies (a file deleted/moved since the last session is simply
/// dropped, never an error). Order preserved.
pub fn existing_buffers(state: &SessionState) -> Vec<(PathBuf, BufferPos)> {
    let fs = crate::fs::active();
    state
        .buffers
        .iter()
        .filter(|(p, _)| fs.exists(p))
        .cloned()
        .collect()
}

/// Clamp a remembered [`WindowFrame`] against the CURRENT set of connected
/// screens, so a disconnected monitor can never strand the window off every
/// visible display. Pure — no winit dependency.
///
/// Picks the screen whose bounds CONTAIN the frame's remembered top-left
/// corner (the strongest "this window lived on THAT monitor" signal); if none
/// does (the monitor that had it is gone), falls back to the FIRST screen in
/// the list (the caller passes its primary first). The frame is then shrunk
/// to fit that screen (never bigger than it) and its position clamped so the
/// whole window sits within its bounds. An empty `screens` list (a query that
/// failed) is a no-op — nothing to clamp against.
pub fn clamp_frame_to_screens(frame: WindowFrame, screens: &[ScreenRect]) -> WindowFrame {
    let Some(&first) = screens.first() else {
        return frame;
    };
    let target = screens
        .iter()
        .find(|s| s.contains_point(frame.x, frame.y))
        .copied()
        .unwrap_or(first);
    let width = frame.width.min(target.width.max(1));
    let height = frame.height.min(target.height.max(1));
    let max_x = target.x + target.width as i32 - width as i32;
    let max_y = target.y + target.height as i32 - height as i32;
    let x = frame.x.clamp(target.x, max_x.max(target.x));
    let y = frame.y.clamp(target.y, max_y.max(target.y));
    WindowFrame { x, y, width, height }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let state = SessionState {
            active: Some(PathBuf::from("/proj/a.md")),
            buffers: vec![
                (PathBuf::from("/proj/a.md"), BufferPos { line: 3, col: 5, scroll: 2 }),
                (PathBuf::from("/proj/b.rs"), BufferPos { line: 0, col: 0, scroll: 0 }),
            ],
            window: Some(WindowFrame { x: 10, y: 20, width: 1200, height: 800 }),
        };
        let text = to_toml(&state);
        assert_eq!(from_toml(&text), state);
    }

    #[test]
    fn empty_or_garbage_toml_yields_default() {
        assert_eq!(from_toml(""), SessionState::default());
        assert_eq!(from_toml("not valid toml {{{"), SessionState::default());
    }

    #[test]
    fn negative_window_position_round_trips() {
        // A window parked to the LEFT of / ABOVE the primary monitor's origin
        // (a real multi-monitor arrangement) has negative outer coordinates.
        let state = SessionState {
            active: None,
            buffers: Vec::new(),
            window: Some(WindowFrame { x: -1200, y: -40, width: 800, height: 600 }),
        };
        assert_eq!(from_toml(&to_toml(&state)), state);
    }

    #[test]
    fn load_and_save_round_trip_through_in_memory_fs() {
        use std::sync::Arc;
        let fake = Arc::new(crate::fs::InMemoryFs::new());
        crate::fs::with_fs(fake, || {
            let path = PathBuf::from("/data/session.toml");
            assert_eq!(load(&path), SessionState::default(), "missing file: empty session");
            let state = SessionState {
                active: Some(PathBuf::from("/n/a.md")),
                buffers: vec![(
                    PathBuf::from("/n/a.md"),
                    BufferPos { line: 1, col: 2, scroll: 4 },
                )],
                window: None,
            };
            save(&path, &state).unwrap();
            assert_eq!(load(&path), state);
        });
    }

    #[test]
    fn existing_buffers_skips_vanished_files_and_preserves_order() {
        use std::sync::Arc;
        let fake = Arc::new(
            crate::fs::InMemoryFs::new()
                .with_file("/n/keep1.md", "x")
                .with_file("/n/keep2.md", "y"),
        );
        crate::fs::with_fs(fake, || {
            let state = SessionState {
                active: None,
                buffers: vec![
                    (PathBuf::from("/n/keep1.md"), BufferPos::default()),
                    (PathBuf::from("/n/gone.md"), BufferPos::default()),
                    (PathBuf::from("/n/keep2.md"), BufferPos::default()),
                ],
                window: None,
            };
            let survivors = existing_buffers(&state);
            let paths: Vec<&Path> = survivors.iter().map(|(p, _)| p.as_path()).collect();
            assert_eq!(
                paths,
                vec![Path::new("/n/keep1.md"), Path::new("/n/keep2.md")],
                "the vanished file is dropped, survivor order preserved"
            );
        });
    }

    #[test]
    fn clamp_keeps_a_frame_already_on_a_known_screen_untouched() {
        let screens = [ScreenRect { x: 0, y: 0, width: 1920, height: 1080 }];
        let frame = WindowFrame { x: 100, y: 100, width: 1200, height: 800 };
        assert_eq!(clamp_frame_to_screens(frame, &screens), frame);
    }

    #[test]
    fn clamp_pulls_a_frame_back_onto_the_primary_when_its_monitor_is_gone() {
        // The frame lived on a SECOND monitor to the right (x starts at 2400)
        // that is no longer connected; only the primary (0,0 1920x1080) remains.
        let screens = [ScreenRect { x: 0, y: 0, width: 1920, height: 1080 }];
        let frame = WindowFrame { x: 2400, y: 300, width: 1200, height: 800 };
        let clamped = clamp_frame_to_screens(frame, &screens);
        assert!(clamped.x >= 0 && clamped.x + clamped.width as i32 <= 1920);
        assert!(clamped.y >= 0 && clamped.y + clamped.height as i32 <= 1080);
    }

    #[test]
    fn clamp_shrinks_a_frame_bigger_than_the_target_screen() {
        let screens = [ScreenRect { x: 0, y: 0, width: 1024, height: 768 }];
        let frame = WindowFrame { x: 0, y: 0, width: 1920, height: 1080 };
        let clamped = clamp_frame_to_screens(frame, &screens);
        assert_eq!(clamped.width, 1024);
        assert_eq!(clamped.height, 768);
    }

    #[test]
    fn clamp_picks_the_screen_containing_the_frames_origin_among_several() {
        let screens = [
            ScreenRect { x: 0, y: 0, width: 1920, height: 1080 },
            ScreenRect { x: 1920, y: 0, width: 2560, height: 1440 },
        ];
        // The frame's origin sits on the SECOND screen; it must not be
        // clamped down to the first screen's (smaller) bounds.
        let frame = WindowFrame { x: 2000, y: 100, width: 1200, height: 800 };
        let clamped = clamp_frame_to_screens(frame, &screens);
        assert_eq!(clamped, frame, "already fits within its own screen: untouched");
    }

    #[test]
    fn clamp_with_no_screens_is_a_no_op() {
        let frame = WindowFrame { x: 5000, y: 5000, width: 1200, height: 800 };
        assert_eq!(clamp_frame_to_screens(frame, &[]), frame);
    }
}
