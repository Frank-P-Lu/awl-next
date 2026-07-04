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
    /// Tab: on a markdown LIST item indent one nesting level (+2 leading spaces,
    /// across a whole selection); ELSEWHERE insert spaces to the next tab stop
    /// (soft tabs). The list-vs-plain decision is made in `apply_core`.
    InsertTab,
    /// Shift-Tab: OUTDENT one nesting level (−2 leading spaces, clamped at 0) across
    /// the caret line or selection — the reverse of a list [`InsertTab`]. Off a list
    /// it simply strips up to two leading spaces (a no-op when there are none).
    Outdent,
    DeleteBackward,
    DeleteWordBackward,
    /// M-d: delete the word AFTER the cursor (the Emacs kill-word) — the forward
    /// mirror of [`Action::DeleteWordBackward`].
    DeleteWordForward,
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
    /// Cmd-A (Super+'a'): SELECT ALL — mark at document start, point at document
    /// end, so the whole buffer is the active region (the mac-native convention;
    /// the Emacs slot keeps C-a = line-start). A no-op empty region on an empty
    /// buffer. NOT a motion (it sets its OWN region, not a Shift-extend) and NOT
    /// an edit (no content change).
    SelectAll,
    // View: zoom
    ZoomIn,
    ZoomOut,
    ZoomReset,
    // View: page scroll (these MOVE the cursor a page, Emacs C-v / M-v).
    PageScrollDown,
    PageScrollUp,
    // Files / control
    Save,
    Quit,
    /// C-s / Cmd-F: start incremental search forward (or next match while
    /// searching). Cmd-F is the native-default Find chord, additive to C-s.
    SearchForward,
    /// C-r / Cmd-Shift-F: start incremental search backward (or previous match
    /// while searching). Additive to C-r.
    SearchBackward,
    /// Cmd-R (headline) / Cmd-Option-F (legacy): summon the find-and-replace panel
    /// — the SAME panel as isearch, with the labeled REPLACE row revealed (a MODE of
    /// the one panel, no separate chrome). A fresh open shows the replace row but
    /// keeps focus on the FIND field; pressing Cmd-R again while the panel is open
    /// jumps focus into the replacement (handled in `App::handle_search_key` +
    /// `apply_core`'s search intercept). Tab switches fields.
    OpenReplace,
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
    /// Cmd-Shift-O (Super+Shift+O): summon the OUTLINE picker — a fuzzy search over
    /// the document's HEADINGS that JUMPS the cursor to the chosen heading's line on
    /// Enter. Its OWN dedicated key (Shift distinguishes it from a free Cmd-O);
    /// summoned + transient, never a persistent outline panel.
    OpenOutline,
    /// Cmd-`;` (Super+`;`): summon the SPELL-SUGGESTION picker for the misspelled
    /// word the cursor is ON or ADJACENT to — a list of the spellchecker's ordered
    /// corrections that REPLACES the word with the chosen one (a single undoable
    /// edit) on Enter. A calm no-op when the cursor isn't on a flagged word. Its
    /// OWN dedicated key, like Cmd-P / Cmd-Shift-O; rebindable via `[keys]`.
    OpenSpellSuggest,
    /// C-x c: toggle the caret LOOK between the classic Block and the live I-beam
    /// caret. Render-only (no buffer change). `c` for "caret". (Morph is not on this
    /// toggle — reach it via `--caret-mode morph` or the caret-style picker.) The
    /// quick cycle is kept for power use; the PICKER ([`OpenCaretMenu`]) is the
    /// discoverable, preview-driven path.
    ToggleCaretMode,
    /// Cmd-P → "Caret style": summon the CARET-STYLE PICKER overlay (the three looks
    /// — Block / Morph / I-beam — each with a description and a LIVE ANIMATED PREVIEW
    /// of the highlighted look). Navigating PREVIEWS the look; Enter APPLIES +
    /// PERSISTS it; Esc reverts. The preview-driven sibling of the blind `C-x c`
    /// toggle. Rebindable via `[keys]`; no default chord (palette-summoned).
    OpenCaretMenu,
    /// Cmd-P → "Dictionary": summon the DICTIONARY picker (the three bundled
    /// spell-check variants — English US / UK / Australia — each with a
    /// description). UNLIKE the theme/caret pickers there is NO live preview as
    /// the selection moves (a dictionary re-parse is a real one-time cost, not a
    /// per-keystroke one); Enter APPLIES + PERSISTS the highlighted variant,
    /// reconstructing the spell-check engine. No default chord (palette-
    /// summoned); rebindable via `[keys]`. See `spell.rs` / `overlay.rs`.
    OpenDictionaryMenu,
    /// Cmd-P → "Toggle Spellcheck": flip the GLOBAL spell-check on/off (default
    /// ON — the escape hatch for no-squiggles-ever people). OFF silences EVERY
    /// squiggle (prose comments and code strings alike, per `spell.rs`'s ONE
    /// owner gate) and turns `Cmd-;` / a right-click into a calm no-op. A real
    /// `Action` (not the `writing_nits` sentinel hack) so it round-trips through
    /// `RunAction` unambiguously; render-only (no buffer change), sticky
    /// (persisted like `writing_nits`). No default chord (palette-summoned);
    /// rebindable via `[keys]`. See `spell.rs`.
    ToggleSpellcheck,
    /// C-x w: toggle PAGE MODE — the centered, measure-capped writing column with
    /// per-world gradient margins. ON by default; toggling OFF lays text edge-to-
    /// edge from the fixed origin (the old behavior). Render-only (no buffer change,
    /// but it re-wraps the document to the new column). `w` for "writing column".
    TogglePageMode,
    /// C-x } : PAGE WIDER — widen the centered writing column's MEASURE by a step
    /// (more characters per line at the same glyph size). Zoom-independent: this sizes
    /// the PAGE, zoom sizes the glyphs. Persisted as a sticky preference. Mnemonic
    /// mirrors Emacs `C-x }` (enlarge window horizontally). Render-only (re-wraps).
    PageWider,
    /// C-x { : PAGE NARROWER — narrow the writing column's MEASURE by a step. The
    /// counterpart to [`PageWider`]; persisted; Emacs `C-x {` mnemonic (shrink window).
    PageNarrower,
    /// RESET PAGE WIDTH — snap the measure back to [`crate::page::DEFAULT_MEASURE`]
    /// and CLEAR the sticky `page_width` config override entirely (back to `None`,
    /// which already means "use the built-in default"), so a future default change
    /// flows through instead of pinning a stale value. The "there's no easy way
    /// back" fix for [`PageWider`]/[`PageNarrower`]. No default chord — reachable via
    /// the palette ("Reset Page Width") and a DOUBLE-CLICK on the draggable page
    /// edge (pointing-not-buttons); rebindable via `[keys]`. Render-only (re-wraps).
    PageReset,
    /// C-x d: CYCLE FOCUS MODE — Off -> Paragraph -> Sentence -> Off. Dims all
    /// document text except the active unit around the cursor (iA Writer-style), so
    /// the eye rests on the sentence / paragraph being written. Render-only (no
    /// buffer change). `d` for "dim". See `focus.rs`.
    CycleFocusMode,
    /// C-x r: TOGGLE the DEBUG panel — the dim top-left dev readout (frametime/fps,
    /// zoom, viewport, cursor, theme/caret/page, md+syn), OFF by default. Render-only
    /// (no buffer change); `r` for "rate". See `debug.rs`. Also reachable via the
    /// `--debug` flag and the palette.
    ToggleDebug,
    /// Cmd-I (held): SUMMON the held STATS HUD — a calm centered metadata panel
    /// (file-created date, session time, word count, %-through-doc) shown WHILE the
    /// key is held and dismissed on release (the "hold to peek the map" affordance).
    /// Render-only (no buffer change); `i` for "info". The live window holds it via
    /// the press/release pair; a headless `--hud` flag / `--keys "Cmd-I"` replay
    /// summons it for the settled capture. See `hud.rs`.
    ShowStatsHud,
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
    /// Keybindings (command palette): summon the GAME-STYLE REBIND MENU — a summoned,
    /// transient picker listing every command + its two bindings, where Enter on a
    /// command captures a new KEY or CHORD and writes it to the config `[keys]` slot
    /// (saved + live-reloaded). No default chord (summon by name, Cmd-P); rebindable
    /// via `[keys] keybindings`. See `overlay.rs` (the capture sub-state) + `actions.rs`.
    OpenKeybindings,
    /// Cmd-Shift-H (Super+Shift+H): summon the HISTORY TIMELINE — a summoned,
    /// transient picker listing the current file's local-history VERSIONS
    /// newest-first (relative timestamps + a "+N −M lines" changed-count), where Enter
    /// RESTORES the highlighted version into the buffer (an undoable edit). SHIFT keeps
    /// a plain Cmd-H free; also a palette command ("History"), rebindable via `[keys]`.
    /// See `overlay.rs` (`OverlayKind::History`) + `history.rs`.
    OpenHistory,
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
                | Action::Outdent
                | Action::DeleteBackward
                | Action::DeleteWordBackward
                | Action::DeleteWordForward
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
    /// `keys` is the `(action-name, chords)` list from [`crate::config::Config`].
    pub fn with_overrides(keys: &[(String, Vec<String>)]) -> Self {
        let mut km = Self::new();
        km.apply_overrides(keys);
        km
    }

    /// Apply (or RE-apply, on a live config reload) the `[keys]` rebinds. Each entry
    /// maps an action NAME (the command-palette name, slugified) to a LIST of up to 2
    /// chords (slot 1 = native, slot 2 = emacs); each valid chord OVERRIDES that
    /// action's binding (additively — both the configured chords AND the default
    /// still fire). An unknown action or a bad chord is reported to stderr and
    /// SKIPPED, keeping the default — never a crash. Only the FIRST TWO chords of a
    /// list are honoured (the model is capped at 2). Clears any prior overrides first
    /// so a reload reflects exactly the current file.
    pub fn apply_overrides(&mut self, keys: &[(String, Vec<String>)]) {
        self.single.clear();
        self.c_x.clear();
        for (name, chords) in keys {
            let Some(action) = crate::commands::action_for_name(name) else {
                eprintln!("config [keys]: unknown action {name:?}; ignored");
                continue;
            };
            for chord in chords.iter().take(2) {
                match parse_binding(chord) {
                    Ok(Chord::Single(k, m)) => {
                        self.single.insert((k, m), action.clone());
                    }
                    Ok(Chord::Cx(k, m)) => {
                        self.c_x.insert((k, m), action.clone());
                    }
                    Err(e) => {
                        eprintln!("config [keys]: {name} = {chord:?}: {e}; keeping default");
                    }
                }
            }
        }
    }

    pub fn in_prefix(&self) -> bool {
        self.in_c_x
    }

    /// True when `key` — interpreted as the UN-COMPOSED logical key while Alt/Meta is
    /// held — would resolve to a real Meta (Option) chord rather than self-insert.
    ///
    /// This exists for the LIVE macOS Option dead-key fix (`app.rs`): Option composes
    /// a letter into a glyph (Option-f -> 'ƒ'), so `event.logical_key` is the composed
    /// char and built-in Meta chords (M-f / M-b / M-w / M-v / M-d / M-< / M->) would never
    /// match. The app asks this of the key WITHOUT Option composition
    /// (`key_without_modifiers`): if it IS a Meta chord, the app feeds the un-composed
    /// key to [`resolve`]; otherwise it keeps the composed char so Option-accent text
    /// INPUT (Option-e -> é) still types. The headless `--keys` path already sends the
    /// un-composed key + ALT, so this predicate is only consulted live.
    pub fn is_meta_chord(&self, key: &Key) -> bool {
        if let Key::Character(s) = key {
            // The built-in Meta chords' base characters (case as they arrive: '<'/'>'
            // already carry their Shift; letters may be either case).
            if matches!(
                s.chars().next(),
                Some('f' | 'F' | 'b' | 'B' | 'w' | 'W' | 'v' | 'V' | 'd' | 'D' | '<' | '>')
            ) {
                return true;
            }
        }
        // A config `[keys]` rebind that uses Meta (Alt) on this key also qualifies, so
        // an Option-composed rebind is un-composed too. Keyed by the canonical key.
        let k = canon_key(key);
        self.single
            .keys()
            .any(|(mk, ms)| *mk == k && ms.contains(ModifiersState::ALT))
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

        // Cmd-S: SAVE — the mac-native save chord, ADDITIVE to the emacs `C-x C-s`
        // (which still works through the prefix path below). 's' is free under Super
        // (z=undo, =/+/-/0=zoom, p, o, c/x/v, f), so no collision. Matched
        // case-insensitively (Shift may produce 'S'); placed before the char dispatch.
        if sup && !ctrl {
            if let Key::Character(s) = logical {
                if matches!(s.chars().next(), Some('s') | Some('S')) {
                    return Action::Save;
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

        // Cmd-Shift-O (Super+Shift+O): summon the OUTLINE picker. SHIFT is required
        // so plain Cmd-O stays free; the logical char arrives as 'O' (or 'o') when
        // shifted. Its own dedicated key, like Cmd-P — collision-free (the Super
        // combos in use are z, =/+/-/0, p, c/x/v).
        if sup && shift && !ctrl {
            if let Key::Character(s) = logical {
                if matches!(s.chars().next(), Some('o') | Some('O')) {
                    return Action::OpenOutline;
                }
            }
        }

        // Cmd-Shift-H (Super+Shift+H): summon the HISTORY TIMELINE picker. SHIFT keeps
        // a plain Cmd-H free; the logical char arrives as 'H' (or 'h') when shifted.
        // Its own dedicated key, like Cmd-Shift-O — collision-free (the Super combos in
        // use are z, =/+/-/0, p, o, c/x/v, f, ';', i, a). Rebindable via `[keys]`.
        if sup && shift && !ctrl {
            if let Key::Character(s) = logical {
                if matches!(s.chars().next(), Some('h') | Some('H')) {
                    return Action::OpenHistory;
                }
            }
        }

        // Cmd-`;` (Super+';'): summon the SPELL-SUGGESTION picker for the word at
        // the cursor. Its own dedicated key, like Cmd-P / Cmd-Shift-O. ';' is free
        // under Super (z, =/+/-/0, p, o, c/x/v, f), so no collision. No SHIFT so the
        // native-feeling chord is a single press; rebindable via `[keys]`.
        if sup && !ctrl {
            if let Key::Character(s) = logical {
                if s.chars().next() == Some(';') {
                    return Action::OpenSpellSuggest;
                }
            }
        }

        // Cmd-I (Super+'i'): SUMMON the held STATS HUD while the key is held (the
        // live press/release pair holds + dismisses it; here we map the press to the
        // action). `i` for "info" — free under Super (z, =/+/-/0, p, o, c/x/v, f, ';'),
        // so no collision. No SHIFT so the hold is a single native-feeling chord. The HUD
        // is HOLD-ONLY: it is deliberately NOT a palette command (a discrete selection
        // could not be released to dismiss it), so this is its sole summon. See `hud.rs`.
        if sup && !ctrl {
            if let Key::Character(s) = logical {
                if matches!(s.chars().next(), Some('i') | Some('I')) {
                    return Action::ShowStatsHud;
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

        // Cmd-F: incremental search forward (mirrors C-s); Cmd-Shift-F: backward
        // (mirrors C-r); Cmd-Option-F: open the search panel with the REPLACE field
        // revealed. The native-default Find direction, ADDITIVE to the C-s/C-r
        // isearch chords (which keep working). 'f' is free under Super (z, =/+/-/0,
        // p, o, c/x/v), so no collision. Placed after the clipboard block so
        // c/x/v already returned. Checked case-insensitively (Shift gives 'F').
        // NOTE: the mid-prefix (C-x ...) check now lives EARLIER (before the Super
        // shortcuts), so a Cmd combo pressed mid-prefix cancels+clears the prefix
        // rather than firing here — see the `in_c_x` block above.
        if sup && !ctrl {
            if let Key::Character(s) = logical {
                if matches!(s.chars().next(), Some('f') | Some('F')) {
                    return if alt {
                        Action::OpenReplace
                    } else if shift {
                        Action::SearchBackward
                    } else {
                        Action::SearchForward
                    };
                }
            }
        }

        // Cmd-R: the HEADLINE find-and-replace door — open (or, while the panel is
        // already up, focus) the replace field. Additive to the legacy Cmd-Option-F
        // above; 'r' is free under Super (z, =/+/-/0, p, o, c/x/v, f), no collision.
        // Placed after the clipboard + Cmd-F blocks so those already returned.
        if sup && !ctrl && !alt {
            if let Key::Character(s) = logical {
                if matches!(s.chars().next(), Some('r') | Some('R')) {
                    return Action::OpenReplace;
                }
            }
        }

        // Cmd-A (Super+'a'): SELECT ALL the whole buffer — the mac-native default.
        // The Emacs slot is untouched: bare C-a (Ctrl, no Super) is still LineStart
        // in `resolve_char`. 'a' is free under Super (z, =/+/-/0, p, o, c/x/v, f, r,
        // i, ';'), so no collision. Placed after the clipboard + Cmd-F/R blocks so
        // those already returned; case-folded ('a'/'A').
        if sup && !ctrl {
            if let Key::Character(s) = logical {
                if matches!(s.chars().next(), Some('a') | Some('A')) {
                    return Action::SelectAll;
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
        // Cmd (Super) + arrow are the mac-native line/buffer motions: Cmd-Left /
        // Cmd-Right = line start / end (alongside C-a / C-e), Cmd-Up / Cmd-Down =
        // buffer start / end (alongside M-< / M->). Shift still extends the selection
        // (handled in `app`), so Cmd-Shift-Left selects to line start, etc.
        let sup = state.contains(ModifiersState::SUPER);
        match named {
            NamedKey::ArrowLeft => {
                if sup {
                    Action::LineStart
                } else if alt || state.contains(ModifiersState::CONTROL) {
                    Action::BackwardWord
                } else {
                    Action::BackwardChar
                }
            }
            NamedKey::ArrowRight => {
                if sup {
                    Action::LineEnd
                } else if alt || state.contains(ModifiersState::CONTROL) {
                    Action::ForwardWord
                } else {
                    Action::ForwardChar
                }
            }
            NamedKey::ArrowUp if sup => Action::BufferStart,
            NamedKey::ArrowDown if sup => Action::BufferEnd,
            NamedKey::ArrowUp => Action::PreviousLine,
            NamedKey::ArrowDown => Action::NextLine,
            NamedKey::Home => Action::LineStart,
            NamedKey::End => Action::LineEnd,
            // PageUp / PageDown move a page (cursor + viewport). Previously unbound, so
            // this is purely additive; in a summoned picker they PAGE the selection.
            NamedKey::PageUp => Action::PageScrollUp,
            NamedKey::PageDown => Action::PageScrollDown,
            NamedKey::Enter => Action::Newline,
            // Shift-Tab OUTDENTS a list level (Tab indents); on a plain line it strips
            // up to two leading spaces (a no-op with none).
            NamedKey::Tab if state.contains(ModifiersState::SHIFT) => Action::Outdent,
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
                'v' => Action::PageScrollDown, // C-v: scroll/move down a page
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
                'v' | 'V' => Action::PageScrollUp, // M-v: scroll/move up a page
                'd' | 'D' => Action::DeleteWordForward, // M-d: kill word forward
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
                    // C-x } / C-x { : page WIDER / NARROWER (adjust the writing-column
                    // measure). Mnemonic mirrors Emacs' enlarge/shrink-window-
                    // horizontally. The `}`/`{` glyphs arrive Shift-produced, so match
                    // them directly (not the base `]`/`[`). Free chords, no collision.
                    Some('}') => return Action::PageWider,
                    Some('{') => return Action::PageNarrower,
                    // C-x r (plain 'r'): toggle the DEBUG frame counter. 'r' for
                    // "rate"; a free chord (the plain chords in use are
                    // t/c/w/d/p/j/b/n/m), so collision-free.
                    Some('r') => return Action::ToggleDebug,
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
    fn cmd_f_find_and_replace_bindings() {
        let mut km = KeymapState::new();
        // Cmd-F starts/steps forward search (native Find); Cmd-Shift-F backward.
        assert_eq!(km.resolve(&ch("f"), &sup()), Action::SearchForward);
        assert_eq!(km.resolve(&ch("F"), &sup_shift()), Action::SearchBackward);
        // Cmd-Option-F opens the panel in replace mode (legacy door).
        assert_eq!(km.resolve(&ch("f"), &sup_alt()), Action::OpenReplace);
        // Cmd-R is the HEADLINE replace door (both cases, additive to Cmd-Option-F).
        assert_eq!(km.resolve(&ch("r"), &sup()), Action::OpenReplace);
        assert_eq!(km.resolve(&ch("R"), &sup_shift()), Action::OpenReplace);
        // The C-s / C-r isearch chords MUST keep working (additive, not replaced).
        assert_eq!(km.resolve(&ch("s"), &ctrl()), Action::SearchForward);
        assert_eq!(km.resolve(&ch("r"), &ctrl()), Action::SearchBackward);
        // Plain 'f' still self-inserts; C-f is still ForwardChar.
        assert_eq!(km.resolve(&ch("f"), &none()), Action::InsertChar('f'));
        assert_eq!(km.resolve(&ch("f"), &ctrl()), Action::ForwardChar);
        // None of the find/replace actions is a motion or an edit.
        assert!(!Action::OpenReplace.is_motion() && !Action::OpenReplace.is_edit());
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
    fn meta_d_deletes_word_forward() {
        // M-d is the Emacs kill-word — the forward mirror of M-Backspace. The
        // bare key resolves here; the LIVE Option-∂ composition routes through
        // `is_meta_chord('d')` (asserted above) back to this same arm.
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("d"), &alt()), Action::DeleteWordForward);
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
    fn cmd_shift_o_opens_outline() {
        let mut km = KeymapState::new();
        // Cmd-Shift-O summons the outline picker (logical char is 'O' when shifted).
        assert_eq!(km.resolve(&ch("O"), &sup_shift()), Action::OpenOutline);
        // A lowercase 'o' with Super+Shift opens it too (defensive case-fold).
        assert_eq!(km.resolve(&ch("o"), &sup_shift()), Action::OpenOutline);
        // Plain Cmd-O (no Shift) is NOT the outline — Shift is required, so it falls
        // through to the normal self-insert path (Super alone doesn't bind 'o').
        assert_eq!(km.resolve(&ch("o"), &sup()), Action::InsertChar('o'));
        // Plain 'o' still self-inserts (the chord didn't shadow it).
        assert_eq!(km.resolve(&ch("o"), &none()), Action::InsertChar('o'));
        // It is neither a motion nor an edit.
        assert!(!Action::OpenOutline.is_motion());
        assert!(!Action::OpenOutline.is_edit());
    }

    #[test]
    fn cmd_shift_h_opens_history() {
        let mut km = KeymapState::new();
        // Cmd-Shift-H summons the history timeline (logical char is 'H' when shifted).
        assert_eq!(km.resolve(&ch("H"), &sup_shift()), Action::OpenHistory);
        // A lowercase 'h' with Super+Shift opens it too (defensive case-fold).
        assert_eq!(km.resolve(&ch("h"), &sup_shift()), Action::OpenHistory);
        // Plain Cmd-H (no Shift) is NOT the timeline — Shift is required, so it falls
        // through to self-insert (Super alone doesn't bind 'h').
        assert_eq!(km.resolve(&ch("h"), &sup()), Action::InsertChar('h'));
        // Plain 'h' still self-inserts (the chord didn't shadow it).
        assert_eq!(km.resolve(&ch("h"), &none()), Action::InsertChar('h'));
        // It is neither a motion nor an edit.
        assert!(!Action::OpenHistory.is_motion());
        assert!(!Action::OpenHistory.is_edit());
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
    fn c_x_brace_pages_wider_and_narrower() {
        let mut km = KeymapState::new();
        // C-x } widens the writing column, C-x { narrows it (Emacs enlarge/shrink-
        // window-horizontally mnemonic). The `}`/`{` arrive Shift-produced.
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("}"), &none()), Action::PageWider);
        assert!(!km.in_prefix());
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("{"), &none()), Action::PageNarrower);
        assert!(!km.in_prefix());
        // Neither is a motion or an edit (palette-eligible, undo-neutral).
        for a in [Action::PageWider, Action::PageNarrower] {
            assert!(!a.is_motion());
            assert!(!a.is_edit());
        }
    }

    #[test]
    fn c_x_toggle_debug() {
        let mut km = KeymapState::new();
        // C-x r toggles the DEBUG frame counter. Plain 'r' (C-r alone is search).
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("r"), &none()), Action::ToggleDebug);
        assert!(!km.in_prefix());
        // ToggleDebug is neither a motion nor an edit (palette-listed, undo-neutral).
        assert!(!Action::ToggleDebug.is_motion());
        assert!(!Action::ToggleDebug.is_edit());
    }

    #[test]
    fn cmd_i_summons_stats_hud() {
        let mut km = KeymapState::new();
        // Cmd-I (Super+'i') summons the held stats HUD. Case-folded ('i'/'I').
        assert_eq!(km.resolve(&ch("i"), &sup()), Action::ShowStatsHud);
        assert_eq!(km.resolve(&ch("I"), &sup()), Action::ShowStatsHud);
        // Plain 'i' (no Super) self-inserts — it is NOT the HUD.
        assert_eq!(km.resolve(&ch("i"), &none()), Action::InsertChar('i'));
        // ShowStatsHud is neither a motion nor an edit (hold-only, undo-neutral).
        assert!(!Action::ShowStatsHud.is_motion());
        assert!(!Action::ShowStatsHud.is_edit());
    }

    #[test]
    fn cmd_a_selects_all() {
        let mut km = KeymapState::new();
        // Cmd-A (Super+'a') selects the whole buffer. Case-folded ('a'/'A').
        assert_eq!(km.resolve(&ch("a"), &sup()), Action::SelectAll);
        assert_eq!(km.resolve(&ch("A"), &sup()), Action::SelectAll);
        // Cmd-Shift-A still selects all (Shift is irrelevant for select-all).
        assert_eq!(km.resolve(&ch("A"), &sup_shift()), Action::SelectAll);
        // The EMACS slot is untouched: bare C-a (Ctrl, no Super) is LINE START,
        // and plain 'a' self-inserts — this is the 2-binding model.
        assert_eq!(km.resolve(&ch("a"), &ctrl()), Action::LineStart);
        assert_eq!(km.resolve(&ch("a"), &none()), Action::InsertChar('a'));
        // SelectAll is neither a motion (it sets its own region) nor an edit.
        assert!(!Action::SelectAll.is_motion());
        assert!(!Action::SelectAll.is_edit());
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
        assert_eq!(km.resolve(&ch("v"), &ctrl()), Action::PageScrollDown);
        assert_eq!(km.resolve(&ch("v"), &alt()), Action::PageScrollUp);
        // PageDown / PageUp named keys page too (additive; in a picker they page the
        // selection). They were previously unbound.
        assert_eq!(
            km.resolve(&Key::Named(NamedKey::PageDown), &none()),
            Action::PageScrollDown
        );
        assert_eq!(
            km.resolve(&Key::Named(NamedKey::PageUp), &none()),
            Action::PageScrollUp
        );
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

    fn sup_alt() -> Modifiers {
        mods(ModifiersState::SUPER | ModifiersState::ALT)
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
            ("switch_theme".to_string(), vec!["C-t".to_string()]),
            ("go_to_file".to_string(), vec!["C-x g".to_string()]),
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
        let keys = vec![("save".to_string(), vec!["C-frobnicate".to_string()])];
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
    fn native_cmd_motion_and_save_defaults() {
        // The mac-native SLOT-1 defaults, ADDITIVE to the emacs slot-2 chords.
        let mut km = KeymapState::new();
        // Cmd-S saves (alongside C-x C-s).
        assert_eq!(km.resolve(&ch("s"), &sup()), Action::Save);
        assert_eq!(km.resolve(&ch("S"), &sup_shift()), Action::Save);
        // Cmd-Left / Cmd-Right = line start / end (alongside C-a / C-e).
        let cmd_arrow = |km: &mut KeymapState, n| km.resolve(&Key::Named(n), &sup());
        assert_eq!(cmd_arrow(&mut km, NamedKey::ArrowLeft), Action::LineStart);
        assert_eq!(cmd_arrow(&mut km, NamedKey::ArrowRight), Action::LineEnd);
        // Cmd-Up / Cmd-Down = buffer start / end (alongside M-< / M->).
        assert_eq!(cmd_arrow(&mut km, NamedKey::ArrowUp), Action::BufferStart);
        assert_eq!(cmd_arrow(&mut km, NamedKey::ArrowDown), Action::BufferEnd);
        // The emacs slot-2 chords STILL resolve (additive, nothing broken).
        assert_eq!(km.resolve(&ch("a"), &ctrl()), Action::LineStart);
        assert_eq!(km.resolve(&ch("e"), &ctrl()), Action::LineEnd);
        assert_eq!(km.resolve(&ch("<"), &alt()), Action::BufferStart);
        assert_eq!(km.resolve(&ch(">"), &alt()), Action::BufferEnd);
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("s"), &ctrl()), Action::Save);
        // Plain arrows are unchanged (no Super = char / line motion).
        assert_eq!(km.resolve(&Key::Named(NamedKey::ArrowLeft), &none()), Action::BackwardChar);
        assert_eq!(km.resolve(&Key::Named(NamedKey::ArrowUp), &none()), Action::PreviousLine);
        // Plain 's' still self-inserts (Cmd-S didn't shadow it).
        assert_eq!(km.resolve(&ch("s"), &none()), Action::InsertChar('s'));
    }

    #[test]
    fn in_prefix_tracks_the_c_x_sequence() {
        // The which-key App reads `in_prefix()` right after each resolve: it must be
        // FALSE at rest, TRUE the instant `C-x` is pressed (awaiting the second key),
        // and FALSE again once any second key resolves — the exact pending window the
        // pause timer arms over.
        let mut km = KeymapState::new();
        assert!(!km.in_prefix(), "idle: not mid-prefix");
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert!(km.in_prefix(), "after C-x: mid-prefix (pending the second key)");
        // The second key resolves the command AND clears the prefix.
        assert_eq!(km.resolve(&ch("s"), &ctrl()), Action::Save);
        assert!(!km.in_prefix(), "after the second key: prefix cleared");
        // An ABORT (C-g mid-prefix) also clears the prefix (Esc behaves the same).
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert!(km.in_prefix());
        assert_eq!(km.resolve(&ch("g"), &ctrl()), Action::Cancel);
        assert!(!km.in_prefix(), "abort clears the prefix");
    }

    #[test]
    fn two_binding_list_resolves_both_slots() {
        // A `[keys]` value is a LIST of up to 2 chords; BOTH resolve to the action
        // (slot 1 native, slot 2 emacs), and the static default also still fires.
        let keys = vec![("switch_theme".to_string(), vec!["s-t".to_string(), "C-t".to_string()])];
        let mut km = KeymapState::with_overrides(&keys);
        assert_eq!(km.resolve(&ch("t"), &sup()), Action::OpenThemeMenu); // slot 1
        assert_eq!(km.resolve(&ch("t"), &ctrl()), Action::OpenThemeMenu); // slot 2
        // The static default C-x t is untouched.
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("t"), &none()), Action::OpenThemeMenu);
        // A list is CAPPED at 2: a third chord is ignored.
        let capped = vec![(
            "go_to_file".to_string(),
            vec!["C-x g".to_string(), "s-g".to_string(), "M-g".to_string()],
        )];
        let mut km = KeymapState::with_overrides(&capped);
        assert_eq!(km.resolve(&ch("g"), &sup()), Action::OpenGoto); // slot 2 honoured
        assert_eq!(km.resolve(&ch("g"), &alt()), Action::Ignore); // slot 3 dropped
    }

    #[test]
    fn is_meta_chord_identifies_option_composable_chords() {
        let km = KeymapState::new();
        // The built-in Meta chords (the ones macOS Option-composes into glyphs).
        for c in ["f", "b", "w", "v", "d", "<", ">"] {
            assert!(km.is_meta_chord(&ch(c)), "{c:?} is a Meta chord");
        }
        // A non-Meta letter and any Named key are NOT (so Option-accent text passes).
        assert!(!km.is_meta_chord(&ch("e")));
        assert!(!km.is_meta_chord(&Key::Named(NamedKey::ArrowLeft)));
        // A config Meta rebind also qualifies, so an Option-composed rebind un-composes.
        let km = KeymapState::with_overrides(&[("toggle_debug".to_string(), vec!["M-q".to_string()])]);
        assert!(km.is_meta_chord(&ch("q")));
        // The same key without a Meta rebind does not.
        assert!(!KeymapState::new().is_meta_chord(&ch("q")));
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
        // Shift-Tab is the OUTDENT chord (Tab alone stays the indent / soft-tab).
        assert_eq!(
            km.resolve(&Key::Named(NamedKey::Tab), &shift()),
            Action::Outdent
        );
        assert_eq!(
            km.resolve(&Key::Named(NamedKey::Backspace), &none()),
            Action::DeleteBackward
        );
    }
}
