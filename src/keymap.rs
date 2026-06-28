//! Keymap: translate winit keyboard events into editor `Action`s. The mapping is
//! a small table-driven function rather than a HashMap so it stays allocation
//! free and easy to read. A simple prefix state implements the `C-x` prefix
//! (C-x C-s = save, C-x C-c = quit).
//!
//! This module is winit-aware but editor-buffer-agnostic: it produces `Action`s,
//! which the app layer applies to the `Buffer`. That keeps the dispatch table
//! testable and the buffer logic clean.

use std::collections::HashMap;

use winit::event::Modifiers;
use winit::keyboard::{Key, ModifiersState, NamedKey, SmolStr};

/// A resolved editor command. `app` matches on these to mutate the buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // Motion
    ForwardChar,
    BackwardChar,
    NextLine,
    PreviousLine,
    LineStart,
    LineEnd,
    ForwardWord,
    BackwardWord,
    BufferStart,
    BufferEnd,
    // Editing
    InsertChar(char),
    Newline,
    /// Tab: insert spaces to the next tab stop (soft tabs).
    InsertTab,
    DeleteBackward,
    DeleteWordBackward,
    DeleteForward,
    KillLine,
    Yank,
    /// Undo the last edit group (Cmd+Z / C-/).
    Undo,
    /// Redo the last undone group (Cmd+Shift+Z).
    Redo,
    // Selection / region
    /// C-Space: set the mark (start a selection at the cursor).
    SetMark,
    /// M-w: copy the active region into the kill buffer (keep text).
    CopyRegion,
    /// C-w: kill (cut) the active region into the kill buffer.
    KillRegion,
    // View: zoom
    ZoomIn,
    ZoomOut,
    ZoomReset,
    // View: page scroll (these MOVE the cursor a page, Emacs C-v / M-v).
    PageDown,
    PageUp,
    // Files / control
    Save,
    Quit,
    /// C-s: start incremental search forward (or next match while searching).
    SearchForward,
    /// C-r: start incremental search backward (or previous match while searching).
    SearchBackward,
    /// C-g / Escape: cancel — clears any active selection / prefix.
    Cancel,
    /// C-x t: summon the THEME PICKER overlay (the 8 worlds, fuzzy-filterable, with
    /// live preview). Replaces the blind `C-x t` / `C-x T` cycle (the theme.rs
    /// `cycle` helper remains the programmatic entry point).
    OpenThemeMenu,
    /// Cmd-P (Super+P): summon the COMMAND PALETTE — a fuzzy search over every
    /// named command (with its current key binding shown beside it) that RUNS the
    /// selected command on Enter. Its OWN dedicated key, separate from the C-x
    /// chords; the catalog lives in `commands.rs`.
    OpenCommandPalette,
    /// C-x c: toggle the caret LOOK between the classic Block and the live I-beam
    /// caret. Render-only (no buffer change). `c` for "caret". (Morph is not on this
    /// toggle — reach it via `--caret-mode morph` or the command palette.)
    ToggleCaretMode,
    /// C-x w: toggle PAGE MODE — the centered, measure-capped writing column with
    /// per-world gradient margins. ON by default; toggling OFF lays text edge-to-
    /// edge from the fixed origin (the old behavior). Render-only (no buffer change,
    /// but it re-wraps the document to the new column). `w` for "writing column".
    TogglePageMode,
    /// C-x d: CYCLE FOCUS MODE — Off -> Paragraph -> Sentence -> Off. Dims all
    /// document text except the active unit around the cursor (iA Writer-style), so
    /// the eye rests on the sentence / paragraph being written. Render-only (no
    /// buffer change). `d` for "dim". See `focus.rs`.
    CycleFocusMode,
    /// C-x C-f: summon the GO-TO overlay over the active project's file index.
    /// While it is open, typed chars edit the overlay query (not the buffer).
    OpenGoto,
    /// C-x p: summon the SWITCH-PROJECT overlay over the workspace children.
    OpenProject,
    /// C-x j: summon the one-level BROWSE navigator for the active root. Enter on
    /// a folder descends; Left/Backspace ascends; Enter on a file opens + closes.
    OpenBrowse,
    /// C-x b: toggle to the PREVIOUSLY-opened file (a tiny 2-deep history). A
    /// no-op when nothing was opened before.
    LastBuffer,
    /// C-x n: NEW QUICK NOTE in ONE gesture — jump to the notes project AND open a
    /// fresh empty note buffer. The user just starts typing; the first non-empty
    /// line names the file (slugified), and it auto-saves. `n` for "note".
    NewNote,
    /// C-x m: MOVE the current note into a folder — summons the move-destination
    /// picker (the Browse navigator over the notes root, folders only). `m` for
    /// "move".
    MoveNote,
    /// Settings (command palette): OPEN the config file (`~/.config/awl/config.toml`)
    /// into the buffer for editing AS TEXT, creating the commented default first if
    /// it does not exist. The palette is the entry point; you then edit + `C-x C-s`
    /// to save, which live-reloads the keymap + folders. No default chord (summon it
    /// by name); see `commands.rs` + `config.rs`.
    OpenSettings,
    // Prefix: C-x was pressed; we are waiting for the next key.
    BeginPrefix,
    /// Pressed a key that does nothing (e.g. lone modifier); ignore it.
    Ignore,
}

impl Action {
    /// True when this action is a cursor MOTION (so the app can extend an
    /// active selection / Shift-selection across it). Editing and view actions
    /// are not motions.
    pub fn is_motion(&self) -> bool {
        matches!(
            self,
            Action::ForwardChar
                | Action::BackwardChar
                | Action::NextLine
                | Action::PreviousLine
                | Action::LineStart
                | Action::LineEnd
                | Action::ForwardWord
                | Action::BackwardWord
                | Action::BufferStart
                | Action::BufferEnd
        )
    }

    /// True when this action MUTATES buffer content (and therefore records undo
    /// history). Undo/Redo themselves are NOT edits — they manage the history and
    /// must not seal a group. The app uses this to decide when to seal the open
    /// undo group: any non-edit, non-undo/redo command (motion, save, mark, …)
    /// seals it so one Cmd+Z undoes a sensible chunk.
    pub fn is_edit(&self) -> bool {
        matches!(
            self,
            Action::InsertChar(_)
                | Action::Newline
                | Action::InsertTab
                | Action::DeleteBackward
                | Action::DeleteWordBackward
                | Action::DeleteForward
                | Action::KillLine
                | Action::Yank
                | Action::KillRegion
        )
    }
}

/// A parsed CONFIG binding: either a one-chord rebind or a `C-x <key>` two-chord
/// rebind (the only two shapes the keymap's prefix model supports). Produced by
/// [`parse_binding`] from a `[keys]` chord string.
pub enum Chord {
    /// A single chord, keyed by its `(key, modifiers)`.
    Single(Key, ModifiersState),
    /// The `C-x` prefix followed by one key, keyed by the SECOND key's `(key, mods)`.
    Cx(Key, ModifiersState),
}

/// Tracks multi-key prefix sequences (the `C-x` prefix) AND the runtime keybinding
/// OVERRIDES loaded from the config `[keys]` table. The override maps are consulted
/// BEFORE the static default arms, so a configured chord wins; both are empty by
/// default, so an absent config keeps the allocation-free default dispatch exactly.
#[derive(Default)]
pub struct KeymapState {
    /// True after C-x, until the next key resolves or cancels the prefix.
    in_c_x: bool,
    /// One-chord rebinds: `(key, mods)` -> Action, consulted at the top of `resolve`.
    single: HashMap<(Key, ModifiersState), Action>,
    /// `C-x <key>` rebinds: the SECOND key's `(key, mods)` -> Action, consulted while
    /// mid-prefix before the static `resolve_c_x` arms.
    c_x: HashMap<(Key, ModifiersState), Action>,
}

impl KeymapState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a keymap with the config `[keys]` rebinds applied over the defaults.
    /// `keys` is the `(action-name, chord)` list from [`crate::config::Config`].
    pub fn with_overrides(keys: &[(String, String)]) -> Self {
        let mut km = Self::new();
        km.apply_overrides(keys);
        km
    }

    /// Apply (or RE-apply, on a live config reload) the `[keys]` rebinds. Each entry
    /// maps an action NAME (the command-palette name, slugified) to a chord; a valid
    /// chord OVERRIDES that action's binding (additively — the default chord still
    /// works too). An unknown action or a bad chord is reported to stderr and SKIPPED,
    /// keeping the default — never a crash. Clears any prior overrides first so a
    /// reload reflects exactly the current file.
    pub fn apply_overrides(&mut self, keys: &[(String, String)]) {
        self.single.clear();
        self.c_x.clear();
        for (name, chord) in keys {
            let Some(action) = crate::commands::action_for_name(name) else {
                eprintln!("config [keys]: unknown action {name:?}; ignored");
                continue;
            };
            match parse_binding(chord) {
                Ok(Chord::Single(k, m)) => {
                    self.single.insert((k, m), action);
                }
                Ok(Chord::Cx(k, m)) => {
                    self.c_x.insert((k, m), action);
                }
                Err(e) => {
                    eprintln!("config [keys]: {name} = {chord:?}: {e}; keeping default");
                }
            }
        }
    }

    #[allow(dead_code)]
    pub fn in_prefix(&self) -> bool {
        self.in_c_x
    }

    /// Resolve a key event to an `Action`, updating prefix state. `mods` is the
    /// current modifier state; `logical` is the winit logical key.
    pub fn resolve(&mut self, logical: &Key, mods: &Modifiers) -> Action {
        let state = mods.state();
        // CONFIG REBIND (single chord): a configured one-chord binding wins over the
        // default dispatch. Guarded by `is_empty` so the no-config path stays
        // allocation-free (canonicalising the key allocates a SmolStr). Only when NOT
        // mid-prefix — a C-x sequence resolves through the `c_x` map below instead.
        if !self.in_c_x && !self.single.is_empty() {
            if let Some(a) = self.single.get(&(canon_key(logical), state)) {
                return a.clone();
            }
        }
        let ctrl = state.contains(ModifiersState::CONTROL);
        // On mac, Option (Alt) is used for Meta-style word motion; treat ALT as
        // Meta. SUPER (Cmd / "Logo") drives the mac-native zoom shortcuts.
        let alt = state.contains(ModifiersState::ALT);
        let sup = state.contains(ModifiersState::SUPER);
        let shift = state.contains(ModifiersState::SHIFT);

        // MID-PREFIX (C-x ...): interpret this key as the SECOND key BEFORE the
        // global Super shortcuts below. Otherwise a Cmd combo pressed mid-prefix
        // (Cmd+C/V/Z/P/zoom) would fire its global shortcut AND leave the prefix
        // armed (the early `return` never clears `in_c_x`), so the NEXT key is
        // wrongly swallowed as a C-x second key — a stuck-prefix bug. With the
        // check here, an undefined `C-x <combo>` cancels and clears the prefix,
        // like any other unbound C-x sequence. A configured `C-x <key>` rebind
        // wins over the static `resolve_c_x` arms.
        if self.in_c_x {
            self.in_c_x = false;
            if !self.c_x.is_empty() {
                if let Some(a) = self.c_x.get(&(canon_key(logical), state)) {
                    return a.clone();
                }
            }
            return resolve_c_x(logical, ctrl);
        }

        // Cmd (Super) undo / redo: Cmd+Z = undo, Cmd+Shift+Z = redo. The logical
        // key arrives as 'z' or (with shift) 'Z', so match case-insensitively and
        // branch on the SHIFT modifier. Checked before zoom/char dispatch.
        if sup && !ctrl {
            if let Key::Character(s) = logical {
                if matches!(s.chars().next(), Some('z') | Some('Z')) {
                    return if shift { Action::Redo } else { Action::Undo };
                }
            }
        }

        // Cmd (Super) zoom shortcuts: Cmd+'=' / Cmd+'+' zoom in, Cmd+'-' zoom
        // out, Cmd+'0' reset. Checked before prefix/char dispatch so they work
        // regardless of state. These are the mac-native bindings.
        if sup && !ctrl {
            if let Some(z) = zoom_for_super(logical) {
                return z;
            }
        }

        // Cmd-P (Super+P): summon the COMMAND PALETTE. This is its OWN dedicated
        // key — NOT a C-x chord — so it never disturbs the prefix bindings. 'p' is
        // free under Super (undo=z, zoom ==/+/-/0, clipboard=c/x/v), so no
        // collision. Matched case-insensitively (Shift may produce 'P').
        if sup && !ctrl {
            if let Key::Character(s) = logical {
                if matches!(s.chars().next(), Some('p') | Some('P')) {
                    return Action::OpenCommandPalette;
                }
            }
        }

        // GUI clipboard aliases on SUPER (Cmd on mac, Win/Super key on Linux).
        // ONE binding set covers both platforms and is collision-free with the
        // Ctrl-based mg keymap (C-x is a prefix, C-v is page-down). Reuses the
        // existing kill-ring Actions; the clipboard bridge lives in app.rs.
        // Placed AFTER the Cmd+Z and zoom blocks above so those already returned
        // before we reach c/x/v — zero collision with z / = / + / - / 0.
        // WAYLAND NOTE: on a Wayland compositor (e.g. Hyprland/Omarchy) Super+V
        // only reaches awl if the compositor has not itself bound that chord;
        // that is the user's compositor config, not awl's concern.
        if sup && !ctrl {
            if let Key::Character(s) = logical {
                match s.chars().next() {
                    Some('c') | Some('C') => return Action::CopyRegion,
                    Some('x') | Some('X') => return Action::KillRegion,
                    Some('v') | Some('V') => return Action::Yank,
                    _ => {}
                }
            }
        }

        match logical {
            Key::Named(named) => self.resolve_named(*named, ctrl, alt, state),
            Key::Character(s) => self.resolve_char(s, ctrl, alt),
            _ => Action::Ignore,
        }
    }

    fn resolve_named(
        &mut self,
        named: NamedKey,
        ctrl: bool,
        alt: bool,
        state: ModifiersState,
    ) -> Action {
        // C-Space sets the mark (start a selection). Space without ctrl is a
        // self-inserting space (handled below).
        if let NamedKey::Space = named {
            if ctrl {
                return Action::SetMark;
            }
        }
        match named {
            NamedKey::ArrowLeft => {
                if alt || state.contains(ModifiersState::CONTROL) {
                    Action::BackwardWord
                } else {
                    Action::BackwardChar
                }
            }
            NamedKey::ArrowRight => {
                if alt || state.contains(ModifiersState::CONTROL) {
                    Action::ForwardWord
                } else {
                    Action::ForwardChar
                }
            }
            NamedKey::ArrowUp => Action::PreviousLine,
            NamedKey::ArrowDown => Action::NextLine,
            NamedKey::Home => Action::LineStart,
            NamedKey::End => Action::LineEnd,
            NamedKey::Enter => Action::Newline,
            NamedKey::Tab => Action::InsertTab,
            NamedKey::Backspace if alt || state.contains(ModifiersState::CONTROL) => {
                Action::DeleteWordBackward
            }
            NamedKey::Backspace => Action::DeleteBackward,
            NamedKey::Delete => Action::DeleteForward,
            NamedKey::Space if !alt => Action::InsertChar(' '),
            NamedKey::Space => Action::Ignore,
            NamedKey::Escape => Action::Cancel,
            _ => Action::Ignore,
        }
    }

    fn resolve_char(&mut self, s: &str, ctrl: bool, alt: bool) -> Action {
        // We key off the first char of the logical string. For control combos
        // winit still reports the base character (e.g. "f" for C-f).
        let Some(c) = s.chars().next() else {
            return Action::Ignore;
        };
        let lower = c.to_ascii_lowercase();

        if ctrl && !alt {
            return match lower {
                'f' => Action::ForwardChar,
                'b' => Action::BackwardChar,
                'n' => Action::NextLine,
                'p' => Action::PreviousLine,
                'a' => Action::LineStart,
                'e' => Action::LineEnd,
                'd' => Action::DeleteForward,
                'k' => Action::KillLine,
                'y' => Action::Yank,
                's' => Action::SearchForward,  // C-s: isearch forward
                'r' => Action::SearchBackward, // C-r: isearch backward
                'w' => Action::KillRegion, // C-w: cut region
                'v' => Action::PageDown,   // C-v: scroll/move down a page
                '/' => Action::Undo,       // C-/: undo (Emacs-ish alias)
                'g' => Action::Cancel,
                'x' => {
                    self.in_c_x = true;
                    Action::BeginPrefix
                }
                _ => Action::Ignore,
            };
        }

        if alt && !ctrl {
            // Meta combos. Note '<' and '>' arrive as those characters already
            // (shift applied), so we match on the literal char, not lower.
            return match c {
                'f' | 'F' => Action::ForwardWord,
                'b' | 'B' => Action::BackwardWord,
                'w' | 'W' => Action::CopyRegion, // M-w: copy region
                'v' | 'V' => Action::PageUp,     // M-v: scroll/move up a page
                '<' => Action::BufferStart,
                '>' => Action::BufferEnd,
                _ => Action::Ignore,
            };
        }

        // No control/meta: a self-inserting printable character. Filter out
        // control characters defensively.
        if !c.is_control() {
            Action::InsertChar(c)
        } else {
            Action::Ignore
        }
    }
}

/// Map a Cmd (Super) + key combo to a zoom action. `Cmd+=`/`Cmd++` zoom in,
/// `Cmd+-` zoom out, `Cmd+0` reset. Returns `None` for any other key so the
/// caller falls through to normal dispatch.
fn zoom_for_super(logical: &Key) -> Option<Action> {
    match logical {
        Key::Character(s) => match s.chars().next()? {
            '=' | '+' => Some(Action::ZoomIn),
            '-' | '_' => Some(Action::ZoomOut),
            '0' => Some(Action::ZoomReset),
            _ => None,
        },
        _ => None,
    }
}

/// Second key of a `C-x` sequence.
fn resolve_c_x(logical: &Key, ctrl: bool) -> Action {
    match logical {
        Key::Character(s) => {
            let c = s.chars().next();
            let lower = c.map(|c| c.to_ascii_lowercase());
            // C-x t summons the THEME PICKER (overlay). This replaced the blind
            // `C-x t` / `C-x T` cycle: the picker's Up/Down now move through the
            // worlds with live preview, and Enter commits / Esc reverts. Both 't'
            // and the Shift-produced 'T' open the same picker.
            if !ctrl {
                match c {
                    Some('t') | Some('T') => return Action::OpenThemeMenu,
                    // C-x c (plain 'c'): toggle the caret look (Block <-> Ibeam).
                    // Note C-x C-c (with ctrl) is Quit, handled below; plain 'c'
                    // is otherwise unbound, so this is collision-free.
                    Some('c') => return Action::ToggleCaretMode,
                    // C-x w (plain 'w'): toggle page mode (centered column ⇄ edge-
                    // to-edge). 'w' for "writing column"; a free chord (the plain
                    // chords in use are t/c/p/j/b/n/m), so collision-free.
                    Some('w') => return Action::TogglePageMode,
                    // C-x d (plain 'd'): cycle focus mode (Off -> Paragraph ->
                    // Sentence). 'd' for "dim"; a free chord (the plain chords in use
                    // are t/c/w/p/j/b/n/m), so collision-free.
                    Some('d') => return Action::CycleFocusMode,
                    // C-x p: summon the switch-project overlay (workspace children).
                    Some('p') => return Action::OpenProject,
                    // C-x j: summon the one-level browse navigator. 'j' is a free
                    // chord (t/c cycle theme/caret, p switches project, s/f/b are
                    // C-x C-… combos), so no collision.
                    Some('j') => return Action::OpenBrowse,
                    // C-x b: toggle to the previously-opened file (last-buffer).
                    Some('b') => return Action::LastBuffer,
                    // C-x n: new quick note (jump to notes project + fresh buffer).
                    // 'n' is free (C-n alone is next-line; this is the C-x chord).
                    Some('n') => return Action::NewNote,
                    // C-x m: move the current note into a folder (destination picker).
                    Some('m') => return Action::MoveNote,
                    _ => {}
                }
            }
            match (ctrl, lower) {
                (true, Some('s')) => Action::Save,
                (true, Some('c')) => Action::Quit,
                // C-x C-f: summon the go-to file overlay (emacs find-file feel).
                (true, Some('f')) => Action::OpenGoto,
                // C-x followed by a plain key we don't bind: cancel quietly.
                _ => Action::Cancel,
            }
        }
        _ => Action::Cancel,
    }
}

/// Canonicalise a key for the override maps: a single-character key is folded to
/// lower-case so a configured `C-t` matches whether winit reports `t` or `T`. Named
/// keys (arrows, Enter, …) pass through unchanged. Used on BOTH insert (via
/// `parse_binding`) and lookup so the two agree.
fn canon_key(key: &Key) -> Key {
    match key {
        Key::Character(s) => Key::Character(SmolStr::new(s.to_lowercase())),
        other => other.clone(),
    }
}

/// True when `key` is the single character `c` (case-insensitive). Used to verify a
/// two-chord rebind's prefix really is `C-x`.
fn key_is_char(key: &Key, c: char) -> bool {
    matches!(key, Key::Character(s) if s.eq_ignore_ascii_case(&c.to_string()))
}

/// Parse a config CHORD STRING into a [`Chord`] keyed for the override maps. Reuses
/// the headless [`crate::keyspec::parse_chord`] so config chords and `--keys` chords
/// share one grammar. Two shapes are accepted (matching the keymap's prefix model):
/// a single chord (`"C-t"`, `"M-g"`), or the `C-x` prefix plus one key (`"C-x g"`).
/// Anything else (an unsupported prefix, 3+ chords, an empty/garbled token) is an
/// `Err(String)` the caller reports while keeping the default — never a panic.
pub fn parse_binding(spec: &str) -> Result<Chord, String> {
    let toks: Vec<&str> = spec.split_whitespace().collect();
    match toks.as_slice() {
        [one] => {
            let (k, m) = crate::keyspec::parse_chord(one).map_err(|e| e.to_string())?;
            Ok(Chord::Single(canon_key(&k), m.state()))
        }
        [a, b] => {
            let (ka, ma) = crate::keyspec::parse_chord(a).map_err(|e| e.to_string())?;
            if ma.state() != ModifiersState::CONTROL || !key_is_char(&ka, 'x') {
                return Err(format!(
                    "only the C-x prefix is supported for two-chord bindings, got {a:?}"
                ));
            }
            let (kb, mb) = crate::keyspec::parse_chord(b).map_err(|e| e.to_string())?;
            Ok(Chord::Cx(canon_key(&kb), mb.state()))
        }
        [] => Err("empty binding".to_string()),
        _ => Err(format!(
            "expected one chord or 'C-x <key>', got {} chords",
            toks.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(s: &str) -> Key {
        Key::Character(SmolStr::new(s))
    }

    fn mods(state: ModifiersState) -> Modifiers {
        // Modifiers implements From<ModifiersState> in winit 0.30.
        Modifiers::from(state)
    }

    fn ctrl() -> Modifiers {
        mods(ModifiersState::CONTROL)
    }

    fn alt() -> Modifiers {
        mods(ModifiersState::ALT)
    }

    fn none() -> Modifiers {
        mods(ModifiersState::empty())
    }

    fn sup() -> Modifiers {
        mods(ModifiersState::SUPER)
    }

    #[test]
    fn ctrl_motions() {
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("f"), &ctrl()), Action::ForwardChar);
        assert_eq!(km.resolve(&ch("b"), &ctrl()), Action::BackwardChar);
        assert_eq!(km.resolve(&ch("n"), &ctrl()), Action::NextLine);
        assert_eq!(km.resolve(&ch("p"), &ctrl()), Action::PreviousLine);
        assert_eq!(km.resolve(&ch("a"), &ctrl()), Action::LineStart);
        assert_eq!(km.resolve(&ch("e"), &ctrl()), Action::LineEnd);
    }

    #[test]
    fn ctrl_editing() {
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("d"), &ctrl()), Action::DeleteForward);
        assert_eq!(km.resolve(&ch("k"), &ctrl()), Action::KillLine);
        assert_eq!(km.resolve(&ch("y"), &ctrl()), Action::Yank);
        assert_eq!(km.resolve(&ch("g"), &ctrl()), Action::Cancel);
    }

    #[test]
    fn ctrl_search() {
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("s"), &ctrl()), Action::SearchForward);
        assert_eq!(km.resolve(&ch("r"), &ctrl()), Action::SearchBackward);
    }

    #[test]
    fn meta_word_and_buffer() {
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("f"), &alt()), Action::ForwardWord);
        assert_eq!(km.resolve(&ch("b"), &alt()), Action::BackwardWord);
        assert_eq!(km.resolve(&ch("<"), &alt()), Action::BufferStart);
        assert_eq!(km.resolve(&ch(">"), &alt()), Action::BufferEnd);
    }

    #[test]
    fn self_insert() {
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("h"), &none()), Action::InsertChar('h'));
        assert_eq!(km.resolve(&ch("Z"), &none()), Action::InsertChar('Z'));
    }

    #[test]
    fn c_x_prefix_save_and_quit() {
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert!(km.in_prefix());
        assert_eq!(km.resolve(&ch("s"), &ctrl()), Action::Save);
        assert!(!km.in_prefix());

        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("c"), &ctrl()), Action::Quit);
    }

    fn shift() -> Modifiers {
        mods(ModifiersState::SHIFT)
    }

    #[test]
    fn c_x_t_opens_theme_menu() {
        let mut km = KeymapState::new();
        // C-x t summons the theme picker (replaced the blind cycle).
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("t"), &none()), Action::OpenThemeMenu);
        // C-x T (Shift) opens the same picker; logical char arrives uppercased.
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("T"), &shift()), Action::OpenThemeMenu);
        // OpenThemeMenu is neither a motion nor an edit.
        assert!(!Action::OpenThemeMenu.is_motion());
        assert!(!Action::OpenThemeMenu.is_edit());
    }

    #[test]
    fn cmd_p_opens_command_palette() {
        let mut km = KeymapState::new();
        // Cmd-P (Super+P) summons the command palette; its own dedicated key.
        assert_eq!(km.resolve(&ch("p"), &sup()), Action::OpenCommandPalette);
        // Shift-produced 'P' opens the same palette.
        assert_eq!(km.resolve(&ch("P"), &sup_shift()), Action::OpenCommandPalette);
        // It is neither a motion nor an edit.
        assert!(!Action::OpenCommandPalette.is_motion());
        assert!(!Action::OpenCommandPalette.is_edit());
        // C-p alone is still PreviousLine (the palette didn't shadow the chord).
        assert_eq!(km.resolve(&ch("p"), &ctrl()), Action::PreviousLine);
        // C-x p (plain) still opens the switch-project overlay (unchanged).
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("p"), &none()), Action::OpenProject);
    }

    #[test]
    fn c_x_toggle_caret_mode() {
        let mut km = KeymapState::new();
        // C-x c toggles the caret look (plain 'c', not ctrl which is Quit).
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("c"), &none()), Action::ToggleCaretMode);
        assert!(!km.in_prefix());
        // C-x C-c (ctrl) is still Quit, unaffected by the new plain-c binding.
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("c"), &ctrl()), Action::Quit);
        // ToggleCaretMode is neither a motion nor an edit.
        assert!(!Action::ToggleCaretMode.is_motion());
        assert!(!Action::ToggleCaretMode.is_edit());
    }

    #[test]
    fn c_x_toggle_page_mode() {
        let mut km = KeymapState::new();
        // C-x w toggles page mode (the centered writing column). Plain 'w'.
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("w"), &none()), Action::TogglePageMode);
        assert!(!km.in_prefix());
        // TogglePageMode is neither a motion nor an edit (so the palette catalog
        // includes it and the undo-group logic leaves it alone).
        assert!(!Action::TogglePageMode.is_motion());
        assert!(!Action::TogglePageMode.is_edit());
    }

    #[test]
    fn c_x_overlay_bindings() {
        let mut km = KeymapState::new();
        // C-x C-f opens the go-to overlay.
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("f"), &ctrl()), Action::OpenGoto);
        // C-x p (plain) opens the switch-project overlay.
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("p"), &none()), Action::OpenProject);
        // C-x j (plain) opens the one-level browse navigator.
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("j"), &none()), Action::OpenBrowse);
        // C-x b (plain) toggles to the previously-opened file.
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("b"), &none()), Action::LastBuffer);
        // None is a motion or an edit.
        assert!(!Action::OpenGoto.is_motion());
        assert!(!Action::OpenGoto.is_edit());
        assert!(!Action::OpenBrowse.is_motion());
        assert!(!Action::LastBuffer.is_edit());
    }

    #[test]
    fn c_x_note_bindings() {
        let mut km = KeymapState::new();
        // C-x n (plain) starts a new quick note.
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("n"), &none()), Action::NewNote);
        // C-x m (plain) opens the move-destination picker.
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("m"), &none()), Action::MoveNote);
        // Neither is a motion or an edit.
        assert!(!Action::NewNote.is_motion() && !Action::NewNote.is_edit());
        assert!(!Action::MoveNote.is_motion() && !Action::MoveNote.is_edit());
        // C-n alone is still next-line (the chord didn't shadow it).
        assert_eq!(km.resolve(&ch("n"), &ctrl()), Action::NextLine);
    }

    #[test]
    fn c_x_then_unknown_cancels() {
        let mut km = KeymapState::new();
        km.resolve(&ch("x"), &ctrl());
        assert_eq!(km.resolve(&ch("z"), &none()), Action::Cancel);
        assert!(!km.in_prefix());
    }

    #[test]
    fn c_x_then_super_combo_cancels_and_clears_prefix() {
        // A Cmd/Super combo pressed MID-PREFIX is an undefined `C-x <combo>`: it
        // must CANCEL and clear the prefix, NOT fire its global Cmd shortcut while
        // leaving the prefix armed (which would swallow the next key as a C-x
        // second key — a stuck-prefix bug).
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert!(km.in_prefix());
        // Cmd+V mid-prefix: Cancel (NOT Yank), and the prefix is cleared.
        assert_eq!(km.resolve(&ch("v"), &sup()), Action::Cancel);
        assert!(!km.in_prefix());
        // The next key resolves normally — proof the prefix is no longer stuck.
        assert_eq!(km.resolve(&ch("a"), &none()), Action::InsertChar('a'));
        // Same for Cmd+Z / Cmd+C (other global Super shortcuts).
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("z"), &sup()), Action::Cancel);
        assert!(!km.in_prefix());
        // And Cmd shortcuts still fire normally when NOT mid-prefix (unchanged).
        assert_eq!(km.resolve(&ch("v"), &sup()), Action::Yank);
        assert_eq!(km.resolve(&ch("z"), &sup()), Action::Undo);
    }

    #[test]
    fn region_bindings() {
        let mut km = KeymapState::new();
        // C-Space sets the mark.
        assert_eq!(
            km.resolve(&Key::Named(NamedKey::Space), &ctrl()),
            Action::SetMark
        );
        // C-w cut, M-w copy.
        assert_eq!(km.resolve(&ch("w"), &ctrl()), Action::KillRegion);
        assert_eq!(km.resolve(&ch("w"), &alt()), Action::CopyRegion);
        // plain space still self-inserts.
        assert_eq!(
            km.resolve(&Key::Named(NamedKey::Space), &none()),
            Action::InsertChar(' ')
        );
    }

    #[test]
    fn page_scroll_bindings() {
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("v"), &ctrl()), Action::PageDown);
        assert_eq!(km.resolve(&ch("v"), &alt()), Action::PageUp);
    }

    #[test]
    fn zoom_bindings_super() {
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("="), &sup()), Action::ZoomIn);
        assert_eq!(km.resolve(&ch("+"), &sup()), Action::ZoomIn);
        assert_eq!(km.resolve(&ch("-"), &sup()), Action::ZoomOut);
        assert_eq!(km.resolve(&ch("0"), &sup()), Action::ZoomReset);
        // Without Cmd, '=' is a normal self-insert.
        assert_eq!(km.resolve(&ch("="), &none()), Action::InsertChar('='));
    }

    fn sup_shift() -> Modifiers {
        mods(ModifiersState::SUPER | ModifiersState::SHIFT)
    }

    #[test]
    fn super_clipboard_aliases() {
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("c"), &sup()), Action::CopyRegion);
        assert_eq!(km.resolve(&ch("x"), &sup()), Action::KillRegion);
        assert_eq!(km.resolve(&ch("v"), &sup()), Action::Yank);
        // case-insensitive (Shift held)
        assert_eq!(km.resolve(&ch("C"), &sup_shift()), Action::CopyRegion);
        assert_eq!(km.resolve(&ch("X"), &sup_shift()), Action::KillRegion);
        assert_eq!(km.resolve(&ch("V"), &sup_shift()), Action::Yank);
    }

    #[test]
    fn super_clipboard_does_not_disturb_undo_or_zoom() {
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("z"), &sup()), Action::Undo);
        assert_eq!(km.resolve(&ch("Z"), &sup_shift()), Action::Redo);
        assert_eq!(km.resolve(&ch("0"), &sup()), Action::ZoomReset);
    }

    #[test]
    fn undo_redo_bindings() {
        let mut km = KeymapState::new();
        // Cmd+Z = undo, Cmd+Shift+Z = redo (logical key is 'Z' when shifted).
        assert_eq!(km.resolve(&ch("z"), &sup()), Action::Undo);
        assert_eq!(km.resolve(&ch("Z"), &sup_shift()), Action::Redo);
        // C-/ = undo (Emacs-ish alias).
        assert_eq!(km.resolve(&ch("/"), &ctrl()), Action::Undo);
        // Plain 'z' still self-inserts.
        assert_eq!(km.resolve(&ch("z"), &none()), Action::InsertChar('z'));
    }

    #[test]
    fn edit_classification() {
        assert!(Action::InsertChar('x').is_edit());
        assert!(Action::KillLine.is_edit());
        assert!(!Action::Undo.is_edit());
        assert!(!Action::Redo.is_edit());
        assert!(!Action::ForwardChar.is_edit());
    }

    #[test]
    fn motion_classification() {
        assert!(Action::ForwardChar.is_motion());
        assert!(Action::BufferEnd.is_motion());
        assert!(!Action::InsertChar('x').is_motion());
        assert!(!Action::KillRegion.is_motion());
        assert!(!Action::ZoomIn.is_motion());
    }

    #[test]
    fn config_rebind_single_and_cx() {
        // A single-chord rebind (C-t) and a C-x two-chord rebind (C-x g), keyed by
        // the slugified action names. Overrides are ADDITIVE: the default chords
        // still resolve too.
        let keys = vec![
            ("switch_theme".to_string(), "C-t".to_string()),
            ("go_to_file".to_string(), "C-x g".to_string()),
        ];
        let mut km = KeymapState::with_overrides(&keys);
        // The new single chord triggers the action.
        assert_eq!(km.resolve(&ch("t"), &ctrl()), Action::OpenThemeMenu);
        // The default C-x t still opens the theme menu (additive).
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("t"), &none()), Action::OpenThemeMenu);
        // The new C-x g (plain g) triggers go-to (C-x g was previously -> Cancel).
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("g"), &none()), Action::OpenGoto);
    }

    #[test]
    fn config_bad_chord_keeps_default() {
        // A garbled chord is ignored; the action keeps its default binding and
        // nothing crashes.
        let keys = vec![("save".to_string(), "C-frobnicate".to_string())];
        let mut km = KeymapState::with_overrides(&keys);
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("s"), &ctrl()), Action::Save);
    }

    #[test]
    fn empty_overrides_behave_like_default() {
        // No config = no overrides = the static dispatch, unchanged.
        let mut km = KeymapState::with_overrides(&[]);
        assert_eq!(km.resolve(&ch("f"), &ctrl()), Action::ForwardChar);
        // C-t is unbound by default (no override), so it Ignores rather than firing.
        assert_eq!(km.resolve(&ch("t"), &ctrl()), Action::Ignore);
    }

    #[test]
    fn named_keys() {
        let mut km = KeymapState::new();
        assert_eq!(
            km.resolve(&Key::Named(NamedKey::ArrowLeft), &none()),
            Action::BackwardChar
        );
        assert_eq!(
            km.resolve(&Key::Named(NamedKey::ArrowRight), &alt()),
            Action::ForwardWord
        );
        assert_eq!(
            km.resolve(&Key::Named(NamedKey::Enter), &none()),
            Action::Newline
        );
        assert_eq!(
            km.resolve(&Key::Named(NamedKey::Tab), &none()),
            Action::InsertTab
        );
        assert_eq!(
            km.resolve(&Key::Named(NamedKey::Backspace), &none()),
            Action::DeleteBackward
        );
    }
}
