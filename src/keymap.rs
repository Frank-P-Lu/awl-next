//! Keymap: translate winit keyboard events into editor `Action`s. The mapping is
//! a small table-driven function rather than a HashMap so it stays allocation
//! free and easy to read. A simple prefix state implements the `C-x` prefix
//! (C-x C-s = save, C-x C-c = quit).
//!
//! This module is winit-aware but editor-buffer-agnostic: it produces `Action`s,
//! which the app layer applies to the `Buffer`. That keeps the dispatch table
//! testable and the buffer logic clean.

use winit::event::Modifiers;
use winit::keyboard::{Key, ModifiersState, NamedKey};

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
    /// C-x c: toggle the caret LOOK between the classic Block and the glyph-shape
    /// Morph caret. Render-only (no buffer change). `c` for "caret".
    ToggleCaretMode,
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

/// Tracks multi-key prefix sequences. Currently only the `C-x` prefix exists.
#[derive(Default)]
pub struct KeymapState {
    /// True after C-x, until the next key resolves or cancels the prefix.
    in_c_x: bool,
}

impl KeymapState {
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(dead_code)]
    pub fn in_prefix(&self) -> bool {
        self.in_c_x
    }

    /// Resolve a key event to an `Action`, updating prefix state. `mods` is the
    /// current modifier state; `logical` is the winit logical key.
    pub fn resolve(&mut self, logical: &Key, mods: &Modifiers) -> Action {
        let state = mods.state();
        let ctrl = state.contains(ModifiersState::CONTROL);
        // On mac, Option (Alt) is used for Meta-style word motion; treat ALT as
        // Meta. SUPER (Cmd / "Logo") drives the mac-native zoom shortcuts.
        let alt = state.contains(ModifiersState::ALT);
        let sup = state.contains(ModifiersState::SUPER);
        let shift = state.contains(ModifiersState::SHIFT);

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

        // If we are mid-prefix (C-x ...), interpret this key as the second key.
        if self.in_c_x {
            self.in_c_x = false;
            return resolve_c_x(logical, ctrl);
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
                    // C-x c (plain 'c'): toggle the caret look (Block <-> Morph).
                    // Note C-x C-c (with ctrl) is Quit, handled below; plain 'c'
                    // is otherwise unbound, so this is collision-free.
                    Some('c') => return Action::ToggleCaretMode,
                    // C-x p: summon the switch-project overlay (workspace children).
                    Some('p') => return Action::OpenProject,
                    // C-x j: summon the one-level browse navigator. 'j' is a free
                    // chord (t/c cycle theme/caret, p switches project, s/f/b are
                    // C-x C-… combos), so no collision.
                    Some('j') => return Action::OpenBrowse,
                    // C-x b: toggle to the previously-opened file (last-buffer).
                    Some('b') => return Action::LastBuffer,
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

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::SmolStr;

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
    fn c_x_then_unknown_cancels() {
        let mut km = KeymapState::new();
        km.resolve(&ch("x"), &ctrl());
        assert_eq!(km.resolve(&ch("z"), &none()), Action::Cancel);
        assert!(!km.in_prefix());
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
