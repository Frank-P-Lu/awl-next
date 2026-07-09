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
    /// Delete the word AFTER the cursor — the forward mirror of
    /// [`Action::DeleteWordBackward`]. The former `M-d` default is retired (the whole
    /// Option-letter layer went quiet — macOS reserves it for typographer dead keys);
    /// it now rides ⌥+forward-Delete (the macOS-native forward word-delete, the mirror
    /// of ⌥⌫), with C-Delete as a quiet second door.
    DeleteWordForward,
    /// Cmd-⌫ (Super+Backspace): the macOS-native "delete to the beginning of the
    /// line" — remove everything from the caret back to the LOGICAL line start,
    /// leaving the caret there. One undoable edit; a calm no-op at column 0.
    DeleteToLineStart,
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
    /// Cmd-T: summon the THEME PICKER overlay (the worlds, fuzzy-filterable, with
    /// live preview). The native switch-theme door (the emacs `C-x t` default is
    /// retired); the theme.rs `cycle` helper remains the programmatic entry point.
    OpenThemeMenu,
    /// Cmd-P (Super+P): summon the COMMAND PALETTE — a fuzzy search over every
    /// named command (with its current key binding shown beside it) that RUNS the
    /// selected command on Enter. Its OWN dedicated key, separate from the C-x
    /// chords; the catalog lives in `commands.rs`.
    OpenCommandPalette,
    /// Palette "Go to heading…": open GO-TO pre-lensed onto its HEADINGS lens — a
    /// fuzzy search over the document's HEADINGS that JUMPS the cursor to the chosen
    /// heading's line on Enter. The fold that retired the standalone Outline picker:
    /// jump-to-heading is a Go-to lens now, reachable both here and via ⌘O → ←/→.
    /// (Internal name kept `OpenOutline`; the palette label is "Go to heading…".)
    OpenOutline,
    /// Cmd-`;` (Super+`;`): summon the SPELL-SUGGESTION picker for the misspelled
    /// word the cursor is ON or ADJACENT to — a list of the spellchecker's ordered
    /// corrections that REPLACES the word with the chosen one (a single undoable
    /// edit) on Enter. A calm no-op when the cursor isn't on a flagged word. Its
    /// OWN dedicated key, like Cmd-P / Cmd-Shift-O; rebindable via `[keys]`.
    OpenSpellSuggest,
    /// Toggle the caret LOOK between the classic Block and the live I-beam caret.
    /// Render-only (no buffer change). Palette-only now (the emacs `C-x c` default is
    /// retired). (Morph is not on this toggle — reach it via `--caret-mode morph` or
    /// the caret-style picker.) The PICKER ([`OpenCaretMenu`]) is the discoverable,
    /// preview-driven path.
    ToggleCaretMode,
    /// Cmd-P → "Caret style…": summon the CARET-STYLE PICKER overlay (the three looks
    /// — Block / Morph / I-beam — each with a description and a LIVE ANIMATED PREVIEW
    /// of the highlighted look). Navigating PREVIEWS the look; Enter APPLIES +
    /// PERSISTS it; Esc reverts. The preview-driven sibling of the blind `C-x c`
    /// toggle. Rebindable via `[keys]`; no default chord (palette-summoned).
    OpenCaretMenu,
    /// Cmd-P → "Dictionary…": summon the DICTIONARY picker (the three bundled
    /// spell-check variants — English US / UK / Australia — each with a
    /// description). UNLIKE the theme/caret pickers there is NO live preview as
    /// the selection moves (a dictionary re-parse is a real one-time cost, not a
    /// per-keystroke one); Enter APPLIES + PERSISTS the highlighted variant,
    /// reconstructing the spell-check engine. No default chord (palette-
    /// summoned); rebindable via `[keys]`. See `spell.rs` / `overlay.rs`.
    OpenDictionaryMenu,
    /// Cmd-P → "Toggle spellcheck": flip the GLOBAL spell-check on/off (default
    /// ON — the escape hatch for no-squiggles-ever people). OFF silences EVERY
    /// squiggle (prose comments and code strings alike, per `spell.rs`'s ONE
    /// owner gate) and turns `Cmd-;` / a right-click into a calm no-op. A real
    /// `Action` (not the `writing_nits` sentinel hack) so it round-trips through
    /// `RunAction` unambiguously; render-only (no buffer change), sticky
    /// (persisted like `writing_nits`). No default chord (palette-summoned);
    /// rebindable via `[keys]`. See `spell.rs`.
    ToggleSpellcheck,
    /// Cmd-Shift-. : while a FILE PICKER is open (go-to / browse), REVEAL or re-hide
    /// dot-prefixed entries (the Finder "show hidden files" convention). Handled ONLY
    /// inside the overlay intercept (a no-op when no picker is open); flips the active
    /// picker's transient `show_hidden` flag and rebuilds its listing. No buffer
    /// change. Rebindable via `[keys] toggle_hidden_files`. See `overlay.rs` /
    /// `index::is_hidden_entry`.
    ToggleHiddenFiles,
    /// Toggle PAGE MODE — the centered, measure-capped writing column with per-world
    /// gradient margins. ON by default; toggling OFF lays text edge-to-edge from the
    /// fixed origin (the old behavior). Render-only (no buffer change, but it re-wraps
    /// the document). Palette-only now (the emacs `C-x w` default is retired).
    TogglePageMode,
    /// PAGE WIDER — widen the centered writing column's MEASURE by a step (more
    /// characters per line at the same glyph size). Zoom-independent: this sizes the
    /// PAGE, zoom sizes the glyphs. Persisted as a sticky preference. Palette-only now
    /// (the emacs `C-x }` default is retired). Render-only (re-wraps).
    PageWider,
    /// PAGE NARROWER — narrow the writing column's MEASURE by a step. The counterpart
    /// to [`PageWider`]; persisted; palette-only now (the emacs `C-x {` is retired).
    PageNarrower,
    /// RESET PAGE WIDTH — snap the measure back to the ACTIVE buffer's OWN built-in
    /// default (see [`crate::page::PageClass::default_measure`] — 70 prose / 100
    /// code) and CLEAR the sticky `page_width_prose`/`page_width_code` config
    /// override matching that SAME class entirely (back to `None`, which already
    /// means "use the built-in default"), so a future default change flows through
    /// instead of pinning a stale value. The "there's no easy way back" fix for
    /// [`PageWider`]/[`PageNarrower`]. No default chord — reachable via the palette
    /// ("Reset page width") and a DOUBLE-CLICK on the draggable page edge
    /// (pointing-not-buttons); rebindable via `[keys]`. Render-only (re-wraps).
    PageReset,
    /// TOGGLE the DEBUG panel — the dim top-left dev readout (frametime/fps, zoom,
    /// viewport, cursor, theme/caret/page, md+syn), OFF by default. Render-only (no
    /// buffer change). See `debug.rs`. Reachable via the `--debug` flag and the palette
    /// (the emacs `C-x r` default is retired).
    ToggleDebug,
    /// Cmd-Shift-O: TOGGLE the persistent MARGIN OUTLINE — the ambient
    /// table-of-contents that lingers in the page margin, OFF by default. Flips the
    /// `outline::OUTLINE_ON` process-global (like `ToggleDebug`), persisted sticky.
    /// Render-only (no buffer change). Jump-to-heading is now a GO-TO lens ("Go to
    /// heading…", `OpenOutline`), not a standalone picker. See `outline.rs`.
    ToggleOutline,
    /// Palette "Toggle typewriter scroll": TOGGLE typewriter scroll — pin the caret's row
    /// centered so the document scrolls under a stationary caret (iA Writer-style),
    /// OFF by default. Flips the
    /// `typewriter::TYPEWRITER_ON` process-global (like `ToggleOutline`), persisted
    /// sticky. Scroll-only (no buffer change), palette-only + rebindable. See
    /// `typewriter.rs`.
    ToggleTypewriter,
    /// Palette "Toggle writing nits": TOGGLE the quiet mechanical-typo underline
    /// highlighter (default ON — quiet + helpful, like spellcheck). Flips the
    /// `nits::NITS_ON` process-global (like `ToggleSpellcheck`), persisted sticky.
    /// Render-only (no buffer change); the nit underlines rebuild from the global
    /// each `prepare`. No default chord (palette-summoned); rebindable via `[keys]
    /// toggle_writing_nits`. See `nits.rs`. (Replaced the former `Ignore`-sentinel
    /// hack — this is now a real, unambiguous `Action`.)
    ToggleWritingNits,
    /// Cmd-I (held): SUMMON the held STATS HUD — a calm centered metadata panel
    /// (file-created date, session time, word count, %-through-doc) shown WHILE the
    /// key is held and dismissed on release (the "hold to peek the map" affordance).
    /// Render-only (no buffer change); `i` for "info". The live window holds it via
    /// the press/release pair; a headless `--hud` flag / `--keys "Cmd-I"` replay
    /// summons it for the settled capture. See `hud.rs`.
    ShowStatsHud,
    /// Palette "About" (macOS menu: App → "About Awl"): OPEN the summoned About
    /// card (name, version, active world, an end-mark ornament) — a calm
    /// `apply_core`-routed card, not muda's predefined About dialog. Stays open
    /// until dismissed by ANY key (`apply_core`'s top-of-function intercept
    /// while `about::about_open()`) or mouse click. Render-only (no buffer
    /// change). See `about.rs`.
    About,
    /// Palette "Lifetime stats": OPEN the summoned Lifetime stats card — the
    /// personal ODOMETER (characters typed, time writing, files touched, caret
    /// travel, most-lived-in world) that used to trail the HELD stats HUD. A calm
    /// `apply_core`-routed card, mirroring About: stays open until dismissed by
    /// ANY key (`apply_core`'s top-of-function intercept while
    /// `lifetime::lifetime_open()`) or mouse click. Render-only (no buffer
    /// change). See `lifetime.rs`.
    LifetimeStats,
    /// Palette "Line endings…": TOGGLE the active buffer's line-ending
    /// discipline (`LF`↔`CRLF`, [`crate::buffer::Eol`]) — the rope is byte-identical
    /// either way (always pure `\n`); only the ON-DISK encoding a save restores
    /// differs. Document-level metadata, NOT an undoable edit (Cmd-Z does not
    /// restore it, mirroring VS Code); it marks the buffer dirty + bumps `version`
    /// so autosave rewrites with the new ending. No default chord — the palette IS
    /// its entry point, like Settings/About. See `buffer.rs`'s `set_eol`.
    ConvertLineEndings,
    /// Palette "Align table": RE-PAD the GFM markdown table under the caret so its
    /// `|` line up (Prettier-style monospace alignment of the SOURCE — awl never
    /// draws a grid). Finds the table block around the caret, replaces it via
    /// [`crate::markdown::align_table`] as ONE undoable edit (Cmd-Z restores the
    /// pre-align source); a calm no-op when the caret is not in a table. No default
    /// chord — the palette IS its entry point (like Settings/About); a real
    /// `Action`, independently rebindable via `[keys]`. See `markdown.rs`.
    AlignTable,
    // --- Markdown formatting commands (see `actions/format.rs`) --------------
    // Every one is a TOGGLE (apply the format when absent on the target, STRIP it
    // when present) applied as ONE undoable edit; all markdown-only (a no-op on a
    // `.rs`/`.txt` buffer). No default chord — palette-summoned (like Align Table),
    // independently rebindable via `[keys]`.
    /// BLOCKQUOTE toggle: prefix each caret/selected line with `> `.
    ToggleBlockquote,
    /// BULLET LIST toggle: prefix each caret/selected line with `- `.
    ToggleBulletList,
    /// NUMBERED LIST toggle: prefix caret/selected lines with `1. `, `2. `… (renumbered).
    ToggleNumberedList,
    /// TASK LIST toggle: prefix each caret/selected line with `- [ ] `.
    ToggleTaskList,
    /// HEADING toggle: prefix the caret/selected line(s) with one `# ` (cycle out of scope).
    ToggleHeading,
    /// FENCED CODE BLOCK toggle: wrap the caret line / selected range in ``` fences.
    ToggleCodeBlock,
    /// BOLD toggle: wrap the selection / word under the caret in `**…**`.
    Bold,
    /// ITALIC toggle: wrap in `*…*`.
    Italic,
    /// INLINE CODE toggle: wrap in `` `…` ``.
    InlineCode,
    /// HIGHLIGHT toggle: wrap in `==…==` (the Obsidian/Typora de-facto mark).
    Highlight,
    /// STRIKETHROUGH toggle: wrap in `~~…~~`.
    Strikethrough,
    /// Cmd-O: summon the GO-TO overlay over the active project's file index — the
    /// native go-to-file door (the emacs `C-x C-f` default is retired). While it is
    /// open, typed chars edit the overlay query (not the buffer).
    OpenGoto,
    /// Cmd-Shift-P: summon the SWITCH-PROJECT overlay over the workspace children —
    /// the native switch-project door (the emacs `C-x p` default is retired).
    OpenProject,
    /// Palette / File menu "Recent projects…": open the SWITCH-PROJECT navigator
    /// pre-lensed onto its RECENT lens — the roots you have most-recently switched to
    /// (from [`crate::recents`], marked among the workspace children). The fold that
    /// retired the standalone RecentProjects picker: recents are a lens now. No default
    /// chord (the palette + File menu ARE its entry points, like Settings/About).
    OpenRecentProjects,
    /// Summon the one-level BROWSE navigator for the active root — palette-only (the
    /// wandering navigator; the emacs `C-x j` default is retired). Enter on a folder
    /// descends; Left/Backspace ascends; Enter on a file opens + closes.
    OpenBrowse,
    /// Ctrl-Tab: toggle to the PREVIOUSLY-opened file (a tiny 2-deep history) — the
    /// native last-file door (the emacs `C-x b` default is retired). A no-op when
    /// nothing was opened before.
    LastBuffer,
    /// Cmd-N: NEW QUICK NOTE in ONE gesture — jump to the notes project AND open a
    /// fresh empty note buffer (the emacs `C-x n` default is retired). The user just
    /// starts typing; the first non-empty line names the file (slugified), and it
    /// auto-saves.
    NewNote,
    /// MOVE the current note into a folder — summons the move-destination picker (the
    /// Browse navigator over the notes root, folders only). Palette-only (the emacs
    /// `C-x m` default is retired).
    MoveNote,
    /// Settings: OPEN the config file (`~/.config/awl/config.toml`) into the buffer
    /// for editing AS TEXT, creating the commented default first if it does not
    /// exist. Formerly the "Settings…" palette command's action; now the SETTINGS
    /// MENU's "Edit config as text" ACTION row (the raw escape hatch) — that wiring
    /// lands next phase, so the variant is momentarily unconstructed (the settings
    /// shell ships first). The `apply_core` arm + `Effect::OpenSettings` handling are
    /// already in place. See `settings.rs` + `config.rs`.
    #[allow(dead_code)] // next-phase: fired by the settings menu's "Edit config as text" row.
    OpenSettings,
    /// Settings (command palette): summon the SETTINGS MENU — a summoned, transient,
    /// faceted picker of every editor setting (categories as lenses) with each
    /// setting's current value in the secondary column. The FRIENDLY default entry
    /// point for the "Settings…" palette command; the raw config-as-text file lives
    /// behind the menu's "Edit config as text" row ([`OpenSettings`]). No default
    /// chord (summon by name); see `settings.rs` + `overlay.rs`.
    OpenSettingsMenu,
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
    /// a plain Cmd-H free; also a palette command ("Version history…"), rebindable via `[keys]`.
    /// See `overlay.rs` (`OverlayKind::History`) + `history.rs`.
    OpenHistory,
    /// Clean unused assets (summon by name, Cmd-P): open the ASSET CLEANER — a
    /// summoned, transient picker listing the ORPHAN image files under the active
    /// project (an image under an `assets/` directory that no document references, per
    /// [`crate::assets::scan`]). Enter on a row moves that file to the macOS TRASH
    /// (recoverable — never `rm`; the row leaves the list, the picker stays open). No
    /// default chord (a palette command, "Clean unused assets…", like Settings/About),
    /// rebindable via `[keys] clean_unused_assets`. See `overlay.rs`
    /// (`OverlayKind::Assets`) + `assets.rs`.
    OpenAssetClean,
    /// THE CONSCIOUS MARK ("Keep version"): record the CURRENT buffer state as
    /// a PINNED local-history snapshot — the deliberate "I care about this one"
    /// action. A pinned snapshot is prune-EXEMPT (it survives the aged retention
    /// ladder / the cap unconditionally; see [`crate::history::prune_ladder`]). The
    /// pure core only SIGNALS it ([`crate::actions::Effect::KeepVersion`]); the
    /// live App does the actual store write (needs the buffer path + config + fs),
    /// so the headless replay no-ops it (the history determinism gate). No default
    /// chord — a palette command ("Keep version"), rebindable via `[keys]`.
    KeepVersion,
    /// FINISH the active buffer (the emacsclient "server-edit" convention; the emacs
    /// `C-x #` default is retired, so it is palette-only now): save it, notify any
    /// daemon `--wait` client waiting on it, and
    /// switch to the previously-open buffer (the same swap [`Action::LastBuffer`]
    /// performs). The core only does the SAVE (identically to [`Action::Save`], so
    /// history/mtime bookkeeping stays on one door); the daemon-notify + buffer-swap
    /// are caller-level (the pure core can't reach the daemon, and headless replay has
    /// none to notify). Also a palette command ("Finish file"), rebindable via
    /// `[keys]`. See `crate::daemon`.
    FinishBuffer,
    /// Cmd-click a markdown link (the advertised mouse affordance), or the emacs
    /// `C-c C-o` chord (the org-mode "open link at point" convention, kept): if the
    /// caret sits inside a markdown link, open that link's URL in the default browser.
    /// The pure core extracts the URL ([`crate::markdown::link_at`]) and signals it
    /// back as [`crate::actions::Effect::FollowLink`]; the live App performs the OS
    /// browser handoff (a user-initiated launch, not an app network fetch — the
    /// zero-network invariant holds). A caret outside every link is a calm no-op.
    /// Headless replay never opens a browser (the effect is live-App-only). Also a
    /// palette command ("Follow link"), rebindable via `[keys]`.
    FollowLink,
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
                | Action::DeleteToLineStart
                | Action::DeleteForward
                | Action::KillLine
                | Action::Yank
                | Action::KillRegion
                | Action::AlignTable
                | Action::ToggleBlockquote
                | Action::ToggleBulletList
                | Action::ToggleNumberedList
                | Action::ToggleTaskList
                | Action::ToggleHeading
                | Action::ToggleCodeBlock
                | Action::Bold
                | Action::Italic
                | Action::InlineCode
                | Action::Highlight
                | Action::Strikethrough
        )
    }
}

/// A parsed CONFIG binding: a one-chord rebind, or a `C-x <key>` / `C-c <key>`
/// two-chord rebind (the shapes the keymap's prefix model supports). Produced by
/// [`parse_binding`] from a `[keys]` chord string.
pub enum Chord {
    /// A single chord, keyed by its `(key, modifiers)`.
    Single(Key, ModifiersState),
    /// The `C-x` prefix followed by one key, keyed by the SECOND key's `(key, mods)`.
    Cx(Key, ModifiersState),
    /// The `C-c` prefix followed by one key, keyed by the SECOND key's `(key, mods)`
    /// — the emacs "user command" prefix (org-mode's `C-c C-o` follow-link lives
    /// here). Mirrors [`Chord::Cx`] exactly; the two prefixes are the only ones the
    /// keymap's model supports.
    Cc(Key, ModifiersState),
}

/// Tracks multi-key prefix sequences (the `C-x` prefix) AND the runtime keybinding
/// OVERRIDES loaded from the config `[keys]` table. The override maps are consulted
/// BEFORE the static default arms, so a configured chord wins; both are empty by
/// default, so an absent config keeps the allocation-free default dispatch exactly.
#[derive(Default)]
pub struct KeymapState {
    /// True after C-x, until the next key resolves or cancels the prefix.
    in_c_x: bool,
    /// True after C-c, until the next key resolves or cancels the prefix (the
    /// second, org-mode-style prefix — mirrors `in_c_x`).
    in_c_c: bool,
    /// One-chord rebinds: `(key, mods)` -> Action, consulted at the top of `resolve`.
    single: HashMap<(Key, ModifiersState), Action>,
    /// `C-x <key>` rebinds: the SECOND key's `(key, mods)` -> Action, consulted while
    /// mid-prefix before the static `resolve_c_x` arms.
    c_x: HashMap<(Key, ModifiersState), Action>,
    /// `C-c <key>` rebinds: the SECOND key's `(key, mods)` -> Action, consulted while
    /// mid-C-c-prefix before the static `resolve_c_c` arms (mirrors `c_x`).
    c_c: HashMap<(Key, ModifiersState), Action>,
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
        self.c_c.clear();
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
                    Ok(Chord::Cc(k, m)) => {
                        self.c_c.insert((k, m), action.clone());
                    }
                    Err(e) => {
                        eprintln!("config [keys]: {name} = {chord:?}: {e}; keeping default");
                    }
                }
            }
        }
    }

    pub fn in_prefix(&self) -> bool {
        self.in_c_x || self.in_c_c
    }

    /// True when `key` — interpreted as the UN-COMPOSED logical key while Alt/Meta is
    /// held — would resolve to a real Meta (Option) chord rather than self-insert.
    ///
    /// This exists for the LIVE macOS Option dead-key fix (`app.rs`): Option composes
    /// a letter into a glyph (Option-f -> 'ƒ'), so `event.logical_key` is the composed
    /// char and a Meta chord would never match. The app asks this of the key WITHOUT
    /// Option composition (`key_without_modifiers`): if it IS a Meta chord, the app
    /// feeds the un-composed key to [`resolve`]; otherwise it keeps the composed char
    /// so Option-accent text INPUT (Option-e -> é) still types.
    ///
    /// Since the identity round RETIRED the built-in Option-letter layer (macOS owns
    /// those keys for typing), there are NO default Meta chords left — a key is a Meta
    /// chord ONLY when a config `[keys]` rebind reclaims it with Meta (Alt). So an
    /// unbound Option-letter always keeps its composed glyph and self-inserts, while a
    /// user-configured Option chord is still un-composed to match. Keyed by the
    /// canonical key. The headless `--keys` path already sends the un-composed key +
    /// ALT, so this predicate is only consulted live.
    pub fn is_meta_chord(&self, key: &Key) -> bool {
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
        if !self.in_c_x && !self.in_c_c && !self.single.is_empty() {
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
        // check here, an undefined `C-x <combo>` cancels and clears the prefix.
        //
        // THE C-x DEFAULTS ARE RETIRED (identity round): the static second-key
        // arms are gone, so C-x is now a bare, defaultless prefix — the MACHINERY
        // (prefix state + the `c_x` config-override map + the which-key panel) is
        // KEPT so a `[keys]` "C-x <key>" line reclaims any chord, but WITHOUT a
        // config binding a C-x sequence just cancels quietly.
        if self.in_c_x {
            self.in_c_x = false;
            if !self.c_x.is_empty() {
                if let Some(a) = self.c_x.get(&(canon_key(logical), state)) {
                    return a.clone();
                }
            }
            return Action::Cancel;
        }

        // MID-PREFIX (C-c ...): the org-mode-style second prefix, mirroring the
        // C-x block above exactly. A configured `C-c <key>` rebind wins over the
        // static `resolve_c_c` arms; an unbound `C-c <combo>` cancels + clears.
        if self.in_c_c {
            self.in_c_c = false;
            if !self.c_c.is_empty() {
                if let Some(a) = self.c_c.get(&(canon_key(logical), state)) {
                    return a.clone();
                }
            }
            return resolve_c_c(logical, ctrl);
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

        // Cmd-Shift-P (Super+Shift+P): SWITCH PROJECT — the native switch-project
        // door (the emacs `C-x p` default is retired this round). SHIFT distinguishes
        // it from the plain Cmd-P palette below; the shifted char arrives as 'P' (or
        // 'p' on layouts that don't case-fold). Checked FIRST so a plain Cmd-P still
        // falls through to the palette.
        if sup && shift && !ctrl {
            if let Key::Character(s) = logical {
                if matches!(s.chars().next(), Some('p') | Some('P')) {
                    return Action::OpenProject;
                }
            }
        }

        // Cmd-P (Super+P): summon the COMMAND PALETTE. This is its OWN dedicated
        // key — NOT a C-x chord — so it never disturbs the prefix bindings. 'p' is
        // free under Super (undo=z, zoom ==/+/-/0, clipboard=c/x/v), so no
        // collision. Plain (no Shift) — Cmd-Shift-P is Switch project, above.
        if sup && !ctrl {
            if let Key::Character(s) = logical {
                if matches!(s.chars().next(), Some('p') | Some('P')) {
                    return Action::OpenCommandPalette;
                }
            }
        }

        // Cmd-Shift-O (Super+Shift+O): TOGGLE the persistent MARGIN OUTLINE. SHIFT is
        // required so plain Cmd-O (Go to file) stays free; the logical char arrives as
        // 'O' (or 'o') when shifted. Its own dedicated key, like Cmd-P — collision-free
        // (the Super combos in use are z, =/+/-/0, p, o, c/x/v). Jump-to-heading is now
        // a Go-to lens ("Go to heading…", `OpenOutline`), not a standalone picker.
        if sup && shift && !ctrl {
            if let Key::Character(s) = logical {
                if matches!(s.chars().next(), Some('o') | Some('O')) {
                    return Action::ToggleOutline;
                }
            }
        }

        // THE NATIVE DOORS — the macOS-native slot-1 chords the identity round
        // advertises (their emacs `C-x` defaults are retired): Cmd-O = GO TO FILE
        // (the go-somewhere door; Cmd-Shift-O above stays the margin outline toggle,
        // so this is the plain unshifted 'o'), Cmd-N = NEW NOTE, Cmd-T = SWITCH THEME,
        // Cmd-Q = QUIT (the clean-shutdown path, same as the menu's routed Quit).
        // 'o'/'n'/'t'/'q' are all free under Super. Placed AFTER the Cmd-Shift-O arm
        // so a shifted 'O' resolves to the outline, not go-to. Case-folded; `!alt` so
        // an Option-composed char still self-inserts.
        if sup && !ctrl && !alt {
            if let Key::Character(s) = logical {
                match s.chars().next() {
                    Some('o') | Some('O') => return Action::OpenGoto,
                    Some('n') | Some('N') => return Action::NewNote,
                    Some('t') | Some('T') => return Action::OpenThemeMenu,
                    Some('q') | Some('Q') => return Action::Quit,
                    _ => {}
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

        // Cmd-Shift-. : REVEAL / hide dot-prefixed entries in the active file picker
        // (the Finder "show hidden files" chord). SHIFT+'.' arrives as '>' on a US
        // layout (Shift composes the glyph), or stays '.' on layouts/paths that don't
        // compose (and the headless `s-S-.` replay sends a bare '.'), so accept
        // EITHER. Collision-free: the Super combos in use are z, =/+/-/0, p, o, h, c/x/v,
        // f, ';', i, a — none is '.'/'>'. Handled by the overlay intercept; a no-op
        // when no picker is open.
        if sup && shift && !ctrl {
            if let Key::Character(s) = logical {
                if matches!(s.chars().next(), Some('.') | Some('>')) {
                    return Action::ToggleHiddenFiles;
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

        // Cmd-B / Cmd-E: the two markdown INLINE toggles with a universal native
        // convention — Cmd-B = Bold, Cmd-E = Inline code (a markdown-only edit; a
        // calm no-op on a non-markdown buffer, gated in `apply_core`). Both 'b' and
        // 'e' are free under Super (the used set is z, =/+/-/0, c/x/v, f, r, a, i,
        // ';'), so no collision. Cmd-I (the universal ITALIC chord) is DELIBERATELY
        // absent here — it is already the held stats HUD above — so Italic stays a
        // palette-only command. All three are rebindable via `[keys]`. Case-folded;
        // `!alt` so an Option-composed char still self-inserts. Placed after the
        // clipboard + Cmd-F/R/A blocks so those already returned.
        if sup && !ctrl && !alt {
            if let Key::Character(s) = logical {
                match s.chars().next() {
                    Some('b') | Some('B') => return Action::Bold,
                    Some('e') | Some('E') => return Action::InlineCode,
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
            // Ctrl-Tab: switch to the LAST (previously-open) buffer — the native
            // slot-1 door (the emacs `C-x b` default is retired). Checked before the
            // indent arms so it never inserts a tab. Native-only in practice: a
            // browser grabs Ctrl-Tab on the web build, where the palette is the door.
            NamedKey::Tab if ctrl => Action::LastBuffer,
            // Shift-Tab OUTDENTS a list level (Tab indents); on a plain line it strips
            // up to two leading spaces (a no-op with none).
            NamedKey::Tab if state.contains(ModifiersState::SHIFT) => Action::Outdent,
            NamedKey::Tab => Action::InsertTab,
            // Cmd-⌫ (Super+Backspace): delete to the beginning of the line — the
            // macOS-native deletion. Checked before the word-delete arm so Super wins.
            NamedKey::Backspace if sup => Action::DeleteToLineStart,
            // ⌥⌫ (Option+Backspace) is the advertised slot-1 WORD delete; C-⌫ stays a
            // quiet second door to the same op.
            NamedKey::Backspace if alt || state.contains(ModifiersState::CONTROL) => {
                Action::DeleteWordBackward
            }
            NamedKey::Backspace => Action::DeleteBackward,
            // ⌥+forward-Delete (Option + the forward-delete key, fn+Delete on a
            // laptop): delete the word AFTER the caret — the macOS-native forward
            // mirror of ⌥⌫; C-Delete is a quiet second door.
            NamedKey::Delete if alt || state.contains(ModifiersState::CONTROL) => {
                Action::DeleteWordForward
            }
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
                // C-c: the org-mode "user command" PREFIX (its only default second
                // key today is C-o = follow-link). Ctrl-C was previously unbound
                // (`Ignore`); on macOS copy is Cmd-C (Super), so this is collision-
                // free. Mirrors the C-x prefix handling.
                'c' => {
                    self.in_c_c = true;
                    Action::BeginPrefix
                }
                _ => Action::Ignore,
            };
        }

        // THE OPTION-LETTER LAYER IS RETIRED (identity round). macOS reserves
        // Option-letters for TYPING — dead keys (Option-e → é, Option-n → ñ), the
        // em-dash (Option-Shift-hyphen), the bullet (Option-8) — which the writer
        // audience needs, and every M-letter chord awl claimed stole one. So an
        // Option-composed char now FALLS THROUGH to self-insert below (the live app
        // keeps the composed glyph; see `is_meta_chord`). Their old actions survive
        // on native chords: word motion → ⌥←/→ (the ARROWS, in `resolve_named`),
        // copy → Cmd-C, buffer ends → Cmd-Up/Down, page-up → PageUp. A config `[keys]`
        // Meta rebind can still reclaim any Option chord (`is_meta_chord` un-composes
        // it live), so the layer is retired, not removed.

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

/// Second key of a `C-c` sequence — the org-mode-style prefix. Its only default
/// binding is `C-c C-o` = FOLLOW LINK (open the markdown link under the caret in
/// the browser). Any other second key cancels quietly, exactly like `resolve_c_x`.
fn resolve_c_c(logical: &Key, ctrl: bool) -> Action {
    match logical {
        Key::Character(s) => {
            let lower = s.chars().next().map(|c| c.to_ascii_lowercase());
            match (ctrl, lower) {
                // C-c C-o: follow the link at point (org-mode's open-link chord).
                (true, Some('o')) => Action::FollowLink,
                // C-c followed by a key we don't bind: cancel quietly.
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
/// a single chord (`"C-t"`, `"M-g"`), or a `C-x`/`C-c` prefix plus one key (`"C-x g"`,
/// `"C-c C-o"`). Anything else (an unsupported prefix, 3+ chords, an empty/garbled
/// token) is an `Err(String)` the caller reports while keeping the default — never a panic.
pub fn parse_binding(spec: &str) -> Result<Chord, String> {
    let toks: Vec<&str> = spec.split_whitespace().collect();
    match toks.as_slice() {
        [one] => {
            let (k, m) = crate::keyspec::parse_chord(one).map_err(|e| e.to_string())?;
            Ok(Chord::Single(canon_key(&k), m.state()))
        }
        [a, b] => {
            let (ka, ma) = crate::keyspec::parse_chord(a).map_err(|e| e.to_string())?;
            let is_cx = ma.state() == ModifiersState::CONTROL && key_is_char(&ka, 'x');
            let is_cc = ma.state() == ModifiersState::CONTROL && key_is_char(&ka, 'c');
            if !is_cx && !is_cc {
                return Err(format!(
                    "only the C-x / C-c prefixes are supported for two-chord bindings, got {a:?}"
                ));
            }
            let (kb, mb) = crate::keyspec::parse_chord(b).map_err(|e| e.to_string())?;
            if is_cx {
                Ok(Chord::Cx(canon_key(&kb), mb.state()))
            } else {
                Ok(Chord::Cc(canon_key(&kb), mb.state()))
            }
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
    fn option_letter_layer_is_retired_word_and_buffer_moved_to_native() {
        // The identity round RETIRED the whole Option-letter layer (macOS owns those
        // keys for typographer dead keys), so an Option+letter now SELF-INSERTS its
        // base char (live, the composed glyph) instead of firing a Meta chord.
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("f"), &alt()), Action::InsertChar('f'), "M-f retired");
        assert_eq!(km.resolve(&ch("b"), &alt()), Action::InsertChar('b'), "M-b retired");
        assert_eq!(km.resolve(&ch("w"), &alt()), Action::InsertChar('w'), "M-w retired");
        assert_eq!(km.resolve(&ch("v"), &alt()), Action::InsertChar('v'), "M-v retired");
        assert_eq!(km.resolve(&ch("<"), &alt()), Action::InsertChar('<'), "M-< retired");
        assert_eq!(km.resolve(&ch(">"), &alt()), Action::InsertChar('>'), "M-> retired");
        // Their actions survive on NATIVE chords: word motion → ⌥←/→ (the ARROWS),
        // buffer ends → Cmd-Up/Down.
        assert_eq!(km.resolve(&Key::Named(NamedKey::ArrowRight), &alt()), Action::ForwardWord);
        assert_eq!(km.resolve(&Key::Named(NamedKey::ArrowLeft), &alt()), Action::BackwardWord);
        assert_eq!(km.resolve(&Key::Named(NamedKey::ArrowUp), &sup()), Action::BufferStart);
        assert_eq!(km.resolve(&Key::Named(NamedKey::ArrowDown), &sup()), Action::BufferEnd);
    }

    #[test]
    fn option_forward_delete_deletes_word_forward() {
        // The former M-d kill-word is retired; forward word-delete now rides
        // ⌥+forward-Delete (the macOS-native mirror of ⌥⌫), with C-Delete a quiet
        // second door. A bare Option+letter 'd' just self-inserts now.
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&Key::Named(NamedKey::Delete), &alt()), Action::DeleteWordForward);
        assert_eq!(km.resolve(&Key::Named(NamedKey::Delete), &ctrl()), Action::DeleteWordForward);
        assert_eq!(km.resolve(&Key::Named(NamedKey::Delete), &none()), Action::DeleteForward);
        assert_eq!(km.resolve(&ch("d"), &alt()), Action::InsertChar('d'));
    }

    #[test]
    fn self_insert() {
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("h"), &none()), Action::InsertChar('h'));
        assert_eq!(km.resolve(&ch("Z"), &none()), Action::InsertChar('Z'));
    }

    #[test]
    fn c_x_defaults_are_retired_but_the_prefix_machinery_survives() {
        // The identity round emptied every C-x SECOND-KEY default. C-x still ARMS the
        // prefix (machinery kept for `[keys]` recovery + which-key), but with no config
        // binding the second key cancels quietly — Save/Quit live on their native
        // chords now (C-x C-s / C-x C-c retired).
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert!(km.in_prefix(), "C-x still arms the prefix");
        assert_eq!(km.resolve(&ch("s"), &ctrl()), Action::Cancel, "C-x C-s retired");
        assert!(!km.in_prefix(), "the second key clears the prefix");
        // Every former C-x default now cancels: C-c (quit), t (theme), w (page),
        // c (caret), r (debug), }/{ (page width), #, b, j, C-f.
        for (k, m) in [
            (ch("c"), ctrl()),
            (ch("t"), none()),
            (ch("w"), none()),
            (ch("c"), none()),
            (ch("r"), none()),
            (ch("}"), none()),
            (ch("{"), none()),
            (ch("#"), none()),
            (ch("b"), none()),
            (ch("j"), none()),
            (ch("f"), ctrl()),
        ] {
            assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
            assert_eq!(km.resolve(&k, &m), Action::Cancel, "C-x second key retired");
            assert!(!km.in_prefix());
        }
        // Save / Quit are reachable on their NATIVE chords instead.
        assert_eq!(km.resolve(&ch("s"), &sup()), Action::Save);
        assert_eq!(km.resolve(&ch("q"), &sup()), Action::Quit);
    }

    #[test]
    fn native_doors_resolve() {
        // The identity round's advertised slot-1 doors (their emacs C-x defaults
        // retired): Cmd-O go-to-file, Cmd-N new note, Cmd-T switch theme, Cmd-Q quit,
        // Cmd-Shift-P switch project, Ctrl-Tab last buffer.
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("o"), &sup()), Action::OpenGoto);
        assert_eq!(km.resolve(&ch("O"), &sup()), Action::OpenGoto);
        assert_eq!(km.resolve(&ch("n"), &sup()), Action::NewNote);
        assert_eq!(km.resolve(&ch("t"), &sup()), Action::OpenThemeMenu);
        assert_eq!(km.resolve(&ch("q"), &sup()), Action::Quit);
        assert_eq!(km.resolve(&ch("P"), &sup_shift()), Action::OpenProject);
        assert_eq!(km.resolve(&ch("p"), &sup_shift()), Action::OpenProject);
        assert_eq!(km.resolve(&Key::Named(NamedKey::Tab), &ctrl()), Action::LastBuffer);
        // None of these plain letters is shadowed — they still self-insert bare.
        for c in ["o", "n", "t", "q"] {
            assert_eq!(km.resolve(&ch(c), &none()), Action::InsertChar(c.chars().next().unwrap()));
        }
        // A plain Tab is still the soft-tab / list indent (only Ctrl-Tab is last-buffer).
        assert_eq!(km.resolve(&Key::Named(NamedKey::Tab), &none()), Action::InsertTab);
        // None is a motion or an edit (palette-eligible, undo-neutral).
        for a in [
            Action::OpenGoto,
            Action::NewNote,
            Action::OpenThemeMenu,
            Action::OpenProject,
            Action::LastBuffer,
        ] {
            assert!(!a.is_motion());
            assert!(!a.is_edit());
        }
    }

    #[test]
    fn c_c_prefix_follows_link() {
        // The org-mode-style C-c prefix: C-c arms the prefix, C-c C-o = FollowLink.
        // (Ctrl-C alone was previously unbound; copy is Cmd-C, not Ctrl-C.)
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("c"), &ctrl()), Action::BeginPrefix);
        assert!(km.in_prefix(), "C-c arms the prefix");
        assert_eq!(km.resolve(&ch("o"), &ctrl()), Action::FollowLink);
        assert!(!km.in_prefix(), "the second key clears the prefix");

        // An unbound C-c second key cancels quietly and clears the prefix.
        assert_eq!(km.resolve(&ch("c"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("z"), &ctrl()), Action::Cancel);
        assert!(!km.in_prefix());
    }

    fn shift() -> Modifiers {
        mods(ModifiersState::SHIFT)
    }

    #[test]
    fn cmd_p_opens_command_palette() {
        let mut km = KeymapState::new();
        // Cmd-P (Super+P, no Shift) summons the command palette; its own dedicated key.
        assert_eq!(km.resolve(&ch("p"), &sup()), Action::OpenCommandPalette);
        // Cmd-SHIFT-P is now Switch project (NOT the palette) — the shift arm wins.
        assert_eq!(km.resolve(&ch("P"), &sup_shift()), Action::OpenProject);
        // It is neither a motion nor an edit.
        assert!(!Action::OpenCommandPalette.is_motion());
        assert!(!Action::OpenCommandPalette.is_edit());
        // C-p alone is still PreviousLine (the palette didn't shadow the chord).
        assert_eq!(km.resolve(&ch("p"), &ctrl()), Action::PreviousLine);
        // C-x p (plain) is now a retired (Cancel) sequence.
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("p"), &none()), Action::Cancel);
    }

    #[test]
    fn cmd_shift_o_toggles_outline_and_plain_cmd_o_goes_to_file() {
        let mut km = KeymapState::new();
        // Cmd-Shift-O TOGGLES the persistent margin outline (logical char is 'O' when
        // shifted).
        assert_eq!(km.resolve(&ch("O"), &sup_shift()), Action::ToggleOutline);
        assert_eq!(km.resolve(&ch("o"), &sup_shift()), Action::ToggleOutline);
        // Plain Cmd-O (no Shift) is now GO TO FILE (the native door) — Shift picks the
        // outline, no Shift picks go-to.
        assert_eq!(km.resolve(&ch("o"), &sup()), Action::OpenGoto);
        // Plain 'o' still self-inserts (neither chord shadowed it).
        assert_eq!(km.resolve(&ch("o"), &none()), Action::InsertChar('o'));
        // Neither is a motion or an edit.
        assert!(!Action::ToggleOutline.is_motion() && !Action::ToggleOutline.is_edit());
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
    fn retired_c_x_actions_stay_undo_neutral_non_motions() {
        // The commands whose C-x default retired (caret/page toggles, page-width
        // nudgers, debug, finish) are still palette-reachable, so they must stay
        // NON-motions and NON-edits (undo-neutral) — the catalog + undo-group logic
        // rely on it even though no chord fires them by default now.
        for a in [
            Action::ToggleCaretMode,
            Action::TogglePageMode,
            Action::PageWider,
            Action::PageNarrower,
            Action::ToggleDebug,
            Action::FinishBuffer,
            Action::OpenBrowse,
            Action::MoveNote,
        ] {
            assert!(!a.is_motion(), "{a:?} must not be a motion");
            assert!(!a.is_edit(), "{a:?} must not be an edit");
        }
    }

    #[test]
    fn cmd_backspace_deletes_to_line_start() {
        // Cmd-⌫ (Super+Backspace) is the macOS-native delete-to-line-start; ⌥⌫ / C-⌫
        // stay word-delete. It is an edit (mutates + records undo), not a motion.
        let mut km = KeymapState::new();
        assert_eq!(
            km.resolve(&Key::Named(NamedKey::Backspace), &sup()),
            Action::DeleteToLineStart
        );
        assert_eq!(
            km.resolve(&Key::Named(NamedKey::Backspace), &alt()),
            Action::DeleteWordBackward
        );
        assert_eq!(
            km.resolve(&Key::Named(NamedKey::Backspace), &ctrl()),
            Action::DeleteWordBackward
        );
        assert_eq!(
            km.resolve(&Key::Named(NamedKey::Backspace), &none()),
            Action::DeleteBackward
        );
        assert!(Action::DeleteToLineStart.is_edit());
        assert!(!Action::DeleteToLineStart.is_motion());
    }

    #[test]
    fn cmd_shift_period_toggles_hidden_files() {
        let mut km = KeymapState::new();
        // Cmd-Shift-. reveals/hides dotfiles in the active file picker. The shifted
        // glyph arrives as '>' on a US layout OR stays '.' (headless `s-S-.`); accept
        // either.
        assert_eq!(km.resolve(&ch("."), &sup_shift()), Action::ToggleHiddenFiles);
        assert_eq!(km.resolve(&ch(">"), &sup_shift()), Action::ToggleHiddenFiles);
        // It is neither a motion nor an edit (palette-listed, undo-neutral).
        assert!(!Action::ToggleHiddenFiles.is_motion());
        assert!(!Action::ToggleHiddenFiles.is_edit());
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
    fn cmd_b_bolds_and_cmd_e_inline_codes() {
        let mut km = KeymapState::new();
        // Cmd-B (Super+'b') toggles Bold; Cmd-E (Super+'e') toggles Inline code — the
        // two markdown inline toggles with a universal native convention. Case-folded.
        assert_eq!(km.resolve(&ch("b"), &sup()), Action::Bold);
        assert_eq!(km.resolve(&ch("B"), &sup()), Action::Bold);
        assert_eq!(km.resolve(&ch("e"), &sup()), Action::InlineCode);
        assert_eq!(km.resolve(&ch("E"), &sup()), Action::InlineCode);
        // Cmd-I is NOT Italic — it stays the held stats HUD (Italic is palette-only),
        // so the universal Cmd-B/I/E trio is deliberately Cmd-B + Cmd-E only.
        assert_eq!(km.resolve(&ch("i"), &sup()), Action::ShowStatsHud);
        // Plain 'b'/'e' (no Super) self-insert — the chords didn't shadow them.
        assert_eq!(km.resolve(&ch("b"), &none()), Action::InsertChar('b'));
        assert_eq!(km.resolve(&ch("e"), &none()), Action::InsertChar('e'));
        // Both are edits (they mutate the buffer) and neither is a motion.
        assert!(Action::Bold.is_edit());
        assert!(Action::InlineCode.is_edit());
        assert!(!Action::Bold.is_motion());
        assert!(!Action::InlineCode.is_motion());
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
        // C-w cut survives (bare-control); M-w copy is retired (Option-letter layer)
        // — copy is Cmd-C now, and Option-w self-inserts.
        assert_eq!(km.resolve(&ch("w"), &ctrl()), Action::KillRegion);
        assert_eq!(km.resolve(&ch("w"), &alt()), Action::InsertChar('w'));
        assert_eq!(km.resolve(&ch("c"), &sup()), Action::CopyRegion);
        // plain space still self-inserts.
        assert_eq!(
            km.resolve(&Key::Named(NamedKey::Space), &none()),
            Action::InsertChar(' ')
        );
    }

    #[test]
    fn page_scroll_bindings() {
        let mut km = KeymapState::new();
        // C-v page-down survives (bare-control); M-v page-up is retired (Option-letter
        // layer) — page-up is the PageUp key now, and Option-v self-inserts.
        assert_eq!(km.resolve(&ch("v"), &ctrl()), Action::PageScrollDown);
        assert_eq!(km.resolve(&ch("v"), &alt()), Action::InsertChar('v'));
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
        // A single-chord rebind (C-t) and a C-x two-chord rebind (C-x g) — the latter
        // demonstrates the RECOVERY path: a `[keys]` "C-x <key>" line reclaims a C-x
        // sequence even though every C-x DEFAULT is now retired.
        let keys = vec![
            ("switch_theme".to_string(), vec!["C-t".to_string()]),
            ("go_to_file".to_string(), vec!["C-x g".to_string()]),
        ];
        let mut km = KeymapState::with_overrides(&keys);
        // The configured single chord triggers the action (native Cmd-T also works).
        assert_eq!(km.resolve(&ch("t"), &ctrl()), Action::OpenThemeMenu);
        assert_eq!(km.resolve(&ch("t"), &sup()), Action::OpenThemeMenu);
        // The retired default C-x t now cancels (no additive default any more).
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("t"), &none()), Action::Cancel);
        // The configured C-x g (plain g) reclaims a C-x sequence and triggers go-to.
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("g"), &none()), Action::OpenGoto);
    }

    #[test]
    fn config_bad_chord_keeps_default() {
        // A garbled chord is ignored; the action keeps its default binding (Save's is
        // the native Cmd-S now) and nothing crashes.
        let keys = vec![("save".to_string(), vec!["C-frobnicate".to_string()])];
        let mut km = KeymapState::with_overrides(&keys);
        assert_eq!(km.resolve(&ch("s"), &sup()), Action::Save);
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
        // The mac-native SLOT-1 defaults, with the SURVIVING bare-control emacs chords.
        let mut km = KeymapState::new();
        // Cmd-S saves (the emacs C-x C-s default is retired).
        assert_eq!(km.resolve(&ch("s"), &sup()), Action::Save);
        assert_eq!(km.resolve(&ch("S"), &sup_shift()), Action::Save);
        // Cmd-Left / Cmd-Right = line start / end (alongside the surviving C-a / C-e).
        let cmd_arrow = |km: &mut KeymapState, n| km.resolve(&Key::Named(n), &sup());
        assert_eq!(cmd_arrow(&mut km, NamedKey::ArrowLeft), Action::LineStart);
        assert_eq!(cmd_arrow(&mut km, NamedKey::ArrowRight), Action::LineEnd);
        // Cmd-Up / Cmd-Down = buffer start / end (the M-< / M-> emacs defaults are
        // retired; these are the only default buffer-end chords now).
        assert_eq!(cmd_arrow(&mut km, NamedKey::ArrowUp), Action::BufferStart);
        assert_eq!(cmd_arrow(&mut km, NamedKey::ArrowDown), Action::BufferEnd);
        // The SURVIVING bare-control nav chords still resolve.
        assert_eq!(km.resolve(&ch("a"), &ctrl()), Action::LineStart);
        assert_eq!(km.resolve(&ch("e"), &ctrl()), Action::LineEnd);
        // The retired chords no longer fire: M-< / M-> self-insert, C-x C-s cancels.
        assert_eq!(km.resolve(&ch("<"), &alt()), Action::InsertChar('<'));
        assert_eq!(km.resolve(&ch(">"), &alt()), Action::InsertChar('>'));
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        assert_eq!(km.resolve(&ch("s"), &ctrl()), Action::Cancel);
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
        // The second key resolves (a retired default now cancels) AND clears the prefix.
        assert_eq!(km.resolve(&ch("s"), &ctrl()), Action::Cancel);
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
        // (slot 1 native, slot 2 emacs). The native Cmd-T default fires too.
        let keys = vec![("switch_theme".to_string(), vec!["s-t".to_string(), "C-t".to_string()])];
        let mut km = KeymapState::with_overrides(&keys);
        assert_eq!(km.resolve(&ch("t"), &sup()), Action::OpenThemeMenu); // slot 1
        assert_eq!(km.resolve(&ch("t"), &ctrl()), Action::OpenThemeMenu); // slot 2
        // A list is CAPPED at 2: a third chord is ignored — so the M-g slot-3 override
        // is never inserted, and (the Option-letter layer being retired) Option-g just
        // self-inserts 'g'.
        let capped = vec![(
            "go_to_file".to_string(),
            vec!["C-x g".to_string(), "s-g".to_string(), "M-g".to_string()],
        )];
        let mut km = KeymapState::with_overrides(&capped);
        assert_eq!(km.resolve(&ch("g"), &sup()), Action::OpenGoto); // slot 2 honoured
        assert_eq!(km.resolve(&ch("g"), &alt()), Action::InsertChar('g')); // slot 3 dropped
    }

    #[test]
    fn is_meta_chord_only_true_for_configured_option_rebinds() {
        // The built-in Option-letter layer is RETIRED, so NO letter is a Meta chord by
        // default — an unbound Option-letter keeps its composed glyph and self-inserts.
        let km = KeymapState::new();
        for c in ["f", "b", "w", "v", "d", "e", "<", ">"] {
            assert!(!km.is_meta_chord(&ch(c)), "{c:?} is no longer a built-in Meta chord");
        }
        assert!(!km.is_meta_chord(&Key::Named(NamedKey::ArrowLeft)));
        // A config Meta rebind qualifies, so an Option-composed rebind un-composes.
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
