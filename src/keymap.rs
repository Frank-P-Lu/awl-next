//! Keymap: translate winit keyboard events into editor `Action`s. Catalogued
//! defaults are seeded into maps from `assets/keymap-defaults.toml`; raw input,
//! prefix arming, and platform aliases remain small hand-written policy. User
//! config maps sit above defaults, so rebinding stays additive and wins conflicts.
//!
//! This module is winit-aware but editor-buffer-agnostic: it produces `Action`s,
//! which the app layer applies to the `Buffer`. That keeps the dispatch table
//! testable and the buffer logic clean.

use std::collections::HashMap;

use winit::event::Modifiers;
use winit::keyboard::{Key, ModifiersState, NamedKey, SmolStr};

use crate::convention::Convention;

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
    /// jumps focus into the replacement (consumed by the shared search-key seam,
    /// `crate::search::keys::intercept`, on both drivers). Tab switches fields.
    OpenReplace,
    /// C-g / Escape: cancel — clears any active selection / prefix.
    Cancel,
    /// Cmd-T: summon the THEME PICKER overlay (the worlds, fuzzy-filterable, with
    /// live preview). The native switch-theme door (the emacs `C-x t` default is
    /// retired); the theme/ `cycle` helper remains the programmatic entry point.
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
    /// summoned); rebindable via `[keys]`. See `spell.rs` / `overlay/`.
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
    /// change. Rebindable via `[keys] toggle_hidden_files`. See `overlay/` /
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
    /// Palette "Toggle menu bar": TOGGLE the awl-RENDERED menu bar — the slim strip of
    /// menu titles across the top of the canvas, shown by default on web/Linux (where
    /// the OS gives no chrome) and absent on macOS (the native NSMenu bar is the door).
    /// Flips the `menubar::MENU_BAR_ON` process-global (like `ToggleOutline`), persisted
    /// sticky. Render-only (no buffer change), palette-only + rebindable. See
    /// `menubar.rs`.
    ToggleMenuBar,
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
    /// Option-Cmd-I (held): SUMMON the held STATS HUD — a calm centered metadata
    /// panel (file-created date, session time, word count, %-through-doc) shown
    /// WHILE the key is held and dismissed on release (the "hold to peek the map"
    /// affordance). Render-only (no buffer change); `i` for "info", ⌥ for the
    /// macOS inspector/info idiom (⌥⌘I opens Get Info in Finder) — MOVED off
    /// plain Cmd-I (the keybinding-idiom audit's Option B) so bare Cmd-I could
    /// become the universal Italic chord instead. The live window holds it via
    /// the press/release pair; a headless `--hud` flag / `--keys "Option-Cmd-I"`
    /// replay summons it for the settled capture. See `hud.rs`.
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
    /// Palette "Writing streaks": OPEN the summoned Writing streaks card — the
    /// year-calendar heatmap of how much you've written each day (net words),
    /// plus the current streak + today's words. A calm `apply_core`-routed card,
    /// mirroring About/Lifetime: stays open until dismissed by ANY key
    /// (`apply_core`'s top-of-function intercept while `streaks::streaks_open()`)
    /// or mouse click — EXCEPT ←/→, which flip the card between its per-day
    /// heatmap and cumulative running-total pages (`apply_core`'s streaks-view
    /// intercept; every summon opens on the heatmap). Render-only (no buffer
    /// change). See `streaks.rs`.
    WritingStreaks,
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
    /// `Action`, independently rebindable via `[keys]`. See `markdown/`.
    AlignTable,
    /// Palette "Report a Problem": compose a `mailto:` link to the maintainer —
    /// subject `"awl problem report (v…)"`, a calm what-happened template body,
    /// and (if a local crash log exists) that log's PATH with a "please attach
    /// this file" line (`mailto:` cannot attach a file; the body never inlines
    /// the log's own content — see the crash-visibility privacy law). The pure
    /// core can't reach the crash-log directory or the OS mail client, so it
    /// signals [`crate::actions::Effect::ReportProblem`] for the live App to
    /// compose (`crashlog::report_problem_mailto`) and open through the SAME
    /// OS-handoff seam `Action::FollowLink` uses (`App::follow_link`). No
    /// document content is ever touched. No default chord (palette-only, like
    /// Settings/About); `native_only: false` — available on the web build too.
    /// Headless replay never opens anything (live-App-only). See `crashlog.rs`.
    ReportProblem,
    /// Palette "Download file" (WEB-ONLY — `web_only: true`, the inverse of
    /// `native_only`): export the ACTIVE buffer's text as a browser download —
    /// `Blob` + object URL + a synthetic `<a download>` click (`web_export.rs`).
    /// A native user already has a real file on real disk (this command is hidden
    /// there entirely — see `commands.rs`'s `web_only` field); on the web build it
    /// is the escape hatch for the no-real-filesystem/no-OS-clipboard sandbox (see
    /// WEB.md). The pure core can't touch `web_sys` (no DOM handoff seam in
    /// `ActionCtx`), so it signals a bare request
    /// ([`crate::actions::Effect::DownloadFile`]) for the live App to perform.
    /// Escapes the SCRATCH buffer too (its virtual `display_name()`). No default
    /// chord (palette-only, like Settings/About); rebindable via `[keys]`.
    /// LIVE-APP-ONLY: headless `--keys` replay never touches the DOM, so it is a
    /// no-op there — a settled capture stays byte-identical. See `web_export.rs`.
    DownloadFile,
    /// Palette "Check for Updates": the app NEVER phones home — this composes
    /// ONE static URL (the site's `/check` page, carrying `CARGO_PKG_VERSION`
    /// as a `?v=` query param — [`crate::updates::check_url`]) and hands it to
    /// the OS browser through the SAME OS-handoff seam `Action::FollowLink` /
    /// `Action::ReportProblem` use (`App::follow_link`); the actual
    /// version-comparison happens in the browser, against a static
    /// `version.json` the site regenerates at deploy — never a fetch from this
    /// binary. The pure core can't reach the fs/OS-handoff, so it signals
    /// [`crate::actions::Effect::CheckForUpdates`] for the live App to (a)
    /// record a LOCAL "last checked" marker (best-effort,
    /// `updates::record_checked`) and (b) open the browser. No document
    /// content is ever touched. No default chord (palette-only, like Report a
    /// Problem/Settings/About); `native_only: true` — the web build updates by
    /// deploy, so the command is meaningless there. Headless replay never
    /// writes the marker or opens anything (live-App-only, mirroring
    /// `ReportProblem`/`FollowLink`). See `updates.rs`.
    CheckForUpdates,
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
    /// HEADING CYCLE: the state-reflective heading cycler behind the format
    /// popover's ONE `H` button — off → H1 → H2 → H3 → off on the caret/selected
    /// line(s). A real palette command ("Cycle heading"), markdown-only, applied as
    /// one undoable edit (`actions::format::heading_cycle`). Distinct from
    /// [`ToggleHeading`](Action::ToggleHeading) (a single `# ` on/off): the popover
    /// wants a level cycle, so it fires THIS.
    HeadingCycle,
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
    /// EXPORT AS WORD (palette-only, markdown buffers): render the document to a
    /// neutral-house-style `.docx` beside the file (or into `notes_root` for a
    /// scratch buffer) — the pure core signals [`crate::actions::Effect::Export`];
    /// the live App builds + writes the bytes (`crate::export`). No default chord.
    ExportWord,
    /// EXPORT AS HTML (palette-only, markdown buffers): render the document to a
    /// standalone, print-tuned `.html` sibling (the documented PDF path — open it
    /// and Print → Save as PDF). Same [`crate::actions::Effect::Export`] seam as
    /// [`ExportWord`](Action::ExportWord). No default chord.
    ExportHtml,
    /// EXPORT AS PDF (native-only, palette-only, markdown buffers): render the
    /// document to a self-contained, paginated A4 `.pdf` sibling. The native
    /// App writes it through the same [`crate::actions::Effect::Export`] seam as
    /// [`ExportWord`](Action::ExportWord). No default chord; hidden on web.
    ExportPdf,
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
    /// NOTES VERBS round: RENAME the current file — summons a minibuffer-style prompt
    /// pre-filled with the current filename; Enter commits the rename on disk, Esc
    /// cancels. Palette-only, no default chord. A no-op summon (no overlay opens) on a
    /// pathless buffer (scratch / an unnamed note) — there is nothing to rename yet.
    /// See `app/files.rs::rename_current_file`.
    OpenRenameNote,
    /// NOTES VERBS round: DUPLICATE the current file to a sibling, auto-named via the
    /// same no-clobber dedup [`crate::buffer::unique_path`] uses (`name-2.md`, …), and
    /// switch to the copy as the active buffer (a fresh history timeline — a copy is a
    /// new file). Palette-only, no default chord. A no-op on a pathless buffer.
    DuplicateNote,
    /// Settings: OPEN the config file (`~/.config/awl/config.toml`) into the buffer
    /// for editing AS TEXT, creating the commented default first if it does not
    /// exist. Formerly the "Settings…" palette command's action; now the SETTINGS
    /// MENU's "Edit config as text" ACTION row (the raw escape hatch) — that wiring
    /// lands next phase, so the variant is momentarily unconstructed (the settings
    /// shell ships first). The `apply_core` arm + `Effect::OpenSettings` handling are
    /// already in place. See `settings.rs` + `config/`.
    #[allow(dead_code)] // next-phase: fired by the settings menu's "Edit config as text" row.
    OpenSettings,
    /// Settings (command palette): summon the SETTINGS MENU — a summoned, transient,
    /// faceted picker of every editor setting (categories as lenses) with each
    /// setting's current value in the secondary column. The FRIENDLY default entry
    /// point for the "Settings…" palette command; the raw config-as-text file lives
    /// behind the menu's "Edit config as text" row ([`OpenSettings`]). No default
    /// chord (summon by name); see `settings.rs` + `overlay/`.
    OpenSettingsMenu,
    /// Keybindings (command palette): summon the GAME-STYLE REBIND MENU — a summoned,
    /// transient picker listing every command + its two bindings, where Enter on a
    /// command captures a new KEY or CHORD and writes it to the config `[keys]` slot
    /// (saved + live-reloaded). No default chord (summon by name, Cmd-P); rebindable
    /// via `[keys] keybindings`. See `overlay/` (the capture sub-state) + `actions.rs`.
    OpenKeybindings,
    /// Credits (command palette): open the embedded `CREDITS.md` into the buffer —
    /// the warm, human-readable thank-you (type designers, the dictionary, the
    /// tools-of-thought influences), pointing at `THIRD-PARTY-LICENSES.md` for the
    /// full generated crate inventory. Mirrors the Settings-opens-a-buffer door
    /// exactly (see `App::open_credits`, `app/files.rs`). No default chord (summon
    /// by name); see `credits.rs`.
    OpenCredits,
    /// Guide (command palette): open the embedded `GUIDE.md` into the buffer —
    /// the user guide (where your words live, the notes model, keys, looks, the
    /// config file). Mirrors the Credits-opens-a-buffer door exactly (see
    /// `App::open_guide`, `app/files.rs`). No default chord (summon by name);
    /// see `guide.rs`.
    OpenGuide,
    /// Cmd-Shift-H (Super+Shift+H): summon the HISTORY TIMELINE — a summoned,
    /// transient picker listing the current file's local-history VERSIONS
    /// newest-first (relative timestamps + a "+N −M lines" changed-count), where Enter
    /// RESTORES the highlighted version into the buffer (an undoable edit). SHIFT keeps
    /// a plain Cmd-H free; also a palette command ("Version history…"), rebindable via `[keys]`.
    /// See `overlay/` (`OverlayKind::History`) + `history/`.
    OpenHistory,
    /// THE WRITER'S DIFF (palette "Compare with version…", markdown buffers only):
    /// open the READ-ONLY prose-diff view comparing the CURRENT buffer against a
    /// past version — the marked-up manuscript (`crate::prosediff`: struck deletions,
    /// washed insertions, moves, folds). From the BUFFER (no overlay) it compares
    /// against the most-recent version (a loose file's newest history snapshot, or a
    /// git-managed file's HEAD via `git show`); from the open HISTORY picker it
    /// compares against the HIGHLIGHTED row. Esc returns to the live document exactly
    /// (the buffer is never touched). No default chord (a palette command like
    /// Version history / Settings), rebindable via `[keys] compare_with_version`.
    CompareVersion,
    /// Clean unused assets (summon by name, Cmd-P): open the ASSET CLEANER — a
    /// summoned, transient picker listing the ORPHAN image files under the active
    /// project (an image under an `assets/` directory that no document references, per
    /// [`crate::assets::scan`]). Enter on a row moves that file to the macOS TRASH
    /// (recoverable — never `rm`; the row leaves the list, the picker stays open). No
    /// default chord (a palette command, "Clean unused assets…", like Settings/About),
    /// rebindable via `[keys] clean_unused_assets`. See `overlay/`
    /// (`OverlayKind::Assets`) + `assets.rs`.
    OpenAssetClean,
    /// THE CONSCIOUS MARK ("Keep version…"): record the CURRENT buffer state as
    /// a PINNED local-history snapshot — the deliberate "I care about this one"
    /// action — via a MINIBUFFER PROMPT for an optional NAME (the NAMED SAVE
    /// POINT; the Rename/InsertLink precedent, `OverlayKind::KeepName`). Enter
    /// with text keeps a NAMED point; a blank Enter is the plain keep (zero
    /// friction preserved); Esc cancels. A pinned/named snapshot is prune-EXEMPT
    /// (it survives the aged retention ladder / the cap unconditionally; see
    /// [`crate::history::prune_ladder`]). The core owns the whole prompt flow
    /// (drivable under `--keys`) and only SIGNALS the commit
    /// ([`crate::actions::Effect::KeepVersion`]); the live App does the actual
    /// store write (needs the buffer path + config + fs), so the headless replay
    /// no-ops the commit (the history determinism gate). No default chord — a
    /// palette command ("Keep version…"), rebindable via `[keys] keep_version`.
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
    /// LINKS V2 — Cmd-K, the chord the keybinding-idiom audit reserved for exactly
    /// this (W1): summon a minibuffer URL prompt and, on commit, apply ONE undoable
    /// markdown-link edit. Markdown buffers only (a `.rs`/`.txt` buffer is a calm
    /// no-op, matching the formatting toggles' own availability honesty). Three
    /// modes, chosen purely from buffer state at press time (`actions/link.rs`):
    /// an ACTIVE SELECTION wraps as `[selection](url)`; the CARET INSIDE AN
    /// EXISTING LINK (`markdown::link_at_full`) re-prompts with that link's current
    /// URL and REWRITES it in place; otherwise inserts empty `[](url)` markup with
    /// the caret landing between the brackets, ready to type the link text. The
    /// prompt is prefilled from the kill/clipboard head when it looks like a URL
    /// ([`crate::buffer::is_url`]), else empty. See `overlay::LinkEdit`.
    InsertLink,
    /// Palette "Insert Date": insert TODAY'S date at the caret, formatted per
    /// the user's chosen [`crate::dateformat::DateFormat`] (Settings menu →
    /// "Date format", default `22/07/26` DD/MM/YY), as ONE undoable edit. The
    /// pure core can't read a clock or `Config`, so this only SIGNALS
    /// [`crate::actions::Effect::InsertDate`]; the live App
    /// (`App::insert_date`) reads the real wall clock + the active format
    /// process-global and performs the insert, and the headless `--keys`
    /// replay does the SAME insert against a FIXED placeholder date
    /// (`dateformat::CAPTURE_PLACEHOLDER_YMD`) so a capture stays
    /// deterministic. No default chord (both slots empty, like Align Table) —
    /// palette-summoned, rebindable via `[keys] insert_date`. See
    /// `dateformat.rs`.
    InsertDate,
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
                | Action::HeadingCycle
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

/// Tracks multi-key prefix sequences and the two data-backed binding layers. The
/// embedded catalog defaults are parsed into `default_*` once per construction (and
/// reseeded when Linux keep policy changes); user `[keys]` bindings live in the
/// `override_*` maps and are consulted first, preserving additive override precedence.
pub struct KeymapState {
    /// Which chord layer slot 1 speaks — [`Convention::Mac`] (⌘) or
    /// [`Convention::Linux`] (Ctrl). Defaults to [`Convention::current`]; a test or
    /// the headless capture harness (via `AWL_CONVENTION_FORCE`) can pin either
    /// explicitly through [`Self::new_with_convention`]. See the module doc's
    /// "THE LINUX-NATIVE KEYMAP" section for the whole collision-resolution story.
    convention: Convention,
    /// True after C-x, until the next key resolves or cancels the prefix.
    in_c_x: bool,
    /// True after C-c, until the next key resolves or cancels the prefix (the
    /// second, org-mode-style prefix — mirrors `in_c_x`).
    in_c_c: bool,
    default_single: HashMap<(Key, ModifiersState), Action>,
    default_c_x: HashMap<(Key, ModifiersState), Action>,
    default_c_c: HashMap<(Key, ModifiersState), Action>,
    override_single: HashMap<(Key, ModifiersState), Action>,
    override_c_x: HashMap<(Key, ModifiersState), Action>,
    override_c_c: HashMap<(Key, ModifiersState), Action>,
    /// THE EMACS-HANDS-ON-LINUX ROUND — the config `linux_keep_emacs` list, parsed
    /// into concrete `(key, mods)` chords: on [`Convention::Linux`], a chord in
    /// this set does NOT participate in the native-wins collision (see
    /// [`Self::linux_keeps`]) — its bare-control emacs meaning fires instead. Built
    /// by [`Self::apply_linux_keep`], consulted ONLY when `convention ==
    /// Convention::Linux` (so a Mac keymap can carry a non-empty set — e.g. a test
    /// exercising `apply_linux_keep` before switching convention — and it is still
    /// STRUCTURALLY inert there, matching "Mac convention ignores the key
    /// entirely"). NEVER truly empty by construction — [`Self::apply_linux_keep`]
    /// always seeds `linux_builtin_keep()` first (the insert-link-yields-to-
    /// kill-line floor), so an absent config keeps today's dispatch PLUS that one
    /// unconditional floor chord; every OTHER letter still needs an explicit
    /// `linux_keep_emacs`/`keymap = "emacs"` opt-in, unchanged.
    linux_keep: std::collections::HashSet<(Key, ModifiersState)>,
}

impl Default for KeymapState {
    fn default() -> Self {
        let mut km = Self {
            convention: Convention::current(),
            in_c_x: false,
            in_c_c: false,
            default_single: HashMap::new(),
            default_c_x: HashMap::new(),
            default_c_c: HashMap::new(),
            override_single: HashMap::new(),
            override_c_x: HashMap::new(),
            override_c_c: HashMap::new(),
            linux_keep: std::collections::HashSet::new(),
        };
        // Seed the unconditional built-in keep floor (see `apply_linux_keep`'s
        // doc) — so even a `KeymapState` that never has `apply_linux_keep`
        // called on it (a bare `new`/`new_with_convention`, the shape most of
        // this module's own unit tests use) still carries the floor.
        km.apply_linux_keep(&[]);
        km
    }
}

impl KeymapState {
    pub fn new() -> Self {
        Self::default()
    }

    /// [`Self::new`], but pinning [`Convention`] explicitly rather than reading
    /// [`Convention::current`] — the door a unit test uses to drive the Linux
    /// table through the REAL keymap without depending on the compiled target or
    /// the env-var override (which the headless capture harness uses instead,
    /// via `AWL_CONVENTION_FORCE` — already reached for free through
    /// `Convention::current()`, so no production call site needs THIS door).
    /// Test-only, mirroring `commands::names()`'s `#[cfg(test)]` precedent.
    #[cfg(test)]
    pub fn new_with_convention(convention: Convention) -> Self {
        let mut km = Self { convention, ..Self::default() };
        km.seed_defaults();
        km
    }

    /// Build a keymap with the config `[keys]` rebinds applied over the defaults.
    /// `keys` is the `(action-name, chords)` list from [`crate::config::Config`].
    pub fn with_overrides(keys: &[(String, Vec<String>)]) -> Self {
        let mut km = Self::new();
        km.apply_overrides(keys);
        km
    }

    /// [`Self::with_overrides`], but pinning [`Convention`] explicitly (mirrors
    /// [`Self::new_with_convention`]). Test-only, same reasoning.
    #[cfg(test)]
    pub fn with_overrides_and_convention(keys: &[(String, Vec<String>)], convention: Convention) -> Self {
        let mut km = Self::new_with_convention(convention);
        km.apply_overrides(keys);
        km
    }

    /// [`Self::with_overrides`], ALSO applying the config `linux_keep_emacs` list
    /// (see [`Self::apply_linux_keep`]) — the real production door every live/
    /// headless call site should use once it has a [`crate::config::Config`] in
    /// hand (`App::new`, the `--keys` replay keymap built in `main/args.rs`);
    /// `with_overrides` alone
    /// stays as the simpler door for the many call sites (mostly tests) that
    /// never touch the keep-list.
    pub fn with_overrides_and_keep(keys: &[(String, Vec<String>)], keep: &[String]) -> Self {
        let mut km = Self::with_overrides(keys);
        km.apply_linux_keep(keep);
        km
    }

    /// True when this convention's NATIVE modifier alone is held (never together
    /// with the OTHER convention's own physical modifier, so the two never
    /// double-fire): [`Convention::Mac`] wants Super without Control;
    /// [`Convention::Linux`] wants Control without Super. THE ONE GATE every
    /// native policy arm below reads. Catalog default collision precedence is
    /// applied while seeding the maps; this helper remains for uncatalogued
    /// native aliases such as Cmd-P and Cmd-G.
    fn native_down(&self, state: ModifiersState) -> bool {
        match self.convention {
            Convention::Mac => {
                state.contains(ModifiersState::SUPER) && !state.contains(ModifiersState::CONTROL)
            }
            Convention::Linux => {
                state.contains(ModifiersState::CONTROL) && !state.contains(ModifiersState::SUPER)
            }
        }
    }

    /// Rebuild the catalog-default dispatch layer from the same resolved command
    /// slots every label surface reads. Platform collision policy stays here: on
    /// Linux a kept emacs chord suppresses its native claimant, while a displaced
    /// emacs chord is omitted. Duplicate effective defaults are an embedded-data
    /// bug and panic unless both rows intentionally resolve to the same action.
    fn seed_defaults(&mut self) {
        self.default_single.clear();
        self.default_c_x.clear();
        self.default_c_c.clear();

        for command in crate::commands::COMMANDS.iter() {
            let native = crate::commands::resolved_native(command, self.convention);
            let native_suppressed = self.convention == Convention::Linux
                && linux_keeps_chord_raw(&self.linux_keep, &native);
            let emacs_displaced = self.convention == Convention::Linux
                && linux_displaces_emacs_default_raw(command.emacs, &self.linux_keep);

            if !emacs_displaced {
                self.insert_default(command.emacs, command.action.clone(), command.name);
            }
            if !native_suppressed {
                self.insert_default(&native, command.action.clone(), command.name);
            }
        }

        // The old case-folded static arms ignored Shift unless a shifted chord
        // had its own meaning. Preserve that selection/uppercase behavior by
        // filling only otherwise-unclaimed shifted variants after exact defaults.
        let shifted: Vec<_> = self
            .default_single
            .iter()
            .filter(|((_, mods), _)| !mods.contains(ModifiersState::SHIFT))
            .map(|((key, mods), action)| {
                ((key.clone(), *mods | ModifiersState::SHIFT), action.clone())
            })
            .collect();
        for (chord, action) in shifted {
            self.default_single.entry(chord).or_insert(action);
        }

        // The old bare-Control arms also ignored a simultaneously-held Super
        // modifier. Keep that input-edge compatibility without retaining a
        // second Action table: derive the otherwise-unclaimed variants from the
        // catalog's emacs slots. `C-Tab` is the one native slot whose literal
        // modifier is Control rather than Cmd, and had the same behavior.
        for command in crate::commands::COMMANDS.iter() {
            // Native dispatch requires exactly one convention modifier, so a
            // simultaneously-held Super always bypassed Linux displacement and
            // reached the bare-Control emacs arm.
            self.insert_control_super_variants(
                command.emacs,
                command.action.clone(),
                command.name,
            );
            if command.native.starts_with("C-") {
                self.insert_control_super_variants(
                    command.native,
                    command.action.clone(),
                    command.name,
                );
            }
        }
    }

    fn insert_default(&mut self, spec: &str, action: Action, name: &str) {
        if spec.trim().is_empty() {
            return;
        }
        let chord = parse_binding(spec).unwrap_or_else(|e| {
            panic!("assets/keymap-defaults.toml: {name:?} has invalid chord {spec:?}: {e}")
        });
        match chord {
            Chord::Single(k, m) => {
                insert_default_entry(
                    &mut self.default_single,
                    (k.clone(), m),
                    action.clone(),
                    name,
                    spec,
                );
            }
            Chord::Cx(k, m) => {
                insert_default_entry(&mut self.default_c_x, (k, m), action, name, spec);
            }
            Chord::Cc(k, m) => {
                insert_default_entry(&mut self.default_c_c, (k, m), action, name, spec);
            }
        }
    }

    fn insert_control_super_variants(&mut self, spec: &str, action: Action, name: &str) {
        if spec.trim().is_empty() {
            return;
        }
        let chord = parse_binding(spec).unwrap_or_else(|e| {
            panic!("assets/keymap-defaults.toml: {name:?} has invalid chord {spec:?}: {e}")
        });
        let add_super = |mods: ModifiersState| {
            mods.contains(ModifiersState::CONTROL).then_some(mods | ModifiersState::SUPER)
        };
        match chord {
            Chord::Single(k, m) => {
                if let Some(m) = add_super(m) {
                    self.default_single
                        .entry((k.clone(), m))
                        .or_insert(action.clone());
                    self.default_single
                        .entry((k, m | ModifiersState::SHIFT))
                        .or_insert(action);
                }
            }
            Chord::Cx(k, m) => {
                if let Some(m) = add_super(m) {
                    self.default_c_x
                        .entry((k.clone(), m))
                        .or_insert(action.clone());
                    self.default_c_x
                        .entry((k, m | ModifiersState::SHIFT))
                        .or_insert(action);
                }
            }
            Chord::Cc(k, m) => {
                if let Some(m) = add_super(m) {
                    self.default_c_c
                        .entry((k.clone(), m))
                        .or_insert(action.clone());
                    self.default_c_c
                        .entry((k, m | ModifiersState::SHIFT))
                        .or_insert(action);
                }
            }
        }
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
        self.override_single.clear();
        self.override_c_x.clear();
        self.override_c_c.clear();
        for (name, chords) in keys {
            let Some(action) = crate::commands::action_for_name(name) else {
                eprintln!("config [keys]: unknown action {name:?}; ignored");
                continue;
            };
            for chord in chords.iter().take(2) {
                match parse_binding(chord) {
                    Ok(Chord::Single(k, m)) => {
                        self.override_single.insert((k, m), action.clone());
                    }
                    Ok(Chord::Cx(k, m)) => {
                        self.override_c_x.insert((k, m), action.clone());
                    }
                    Ok(Chord::Cc(k, m)) => {
                        self.override_c_c.insert((k, m), action.clone());
                    }
                    Err(e) => {
                        eprintln!("config [keys]: {name} = {chord:?}: {e}; keeping default");
                    }
                }
            }
        }
    }

    /// Apply (or RE-apply, on a live config reload) the `linux_keep_emacs` list —
    /// THE PER-CHORD DOOR the emacs-hands-on-Linux round adds: under
    /// [`Convention::Linux`], every chord named here is EXEMPTED from the
    /// native-wins collision (`native_down`'s displacement), so its bare-control
    /// emacs meaning keeps firing instead of the native chord that would
    /// otherwise claim that letter (see the module's collision-table doc). A
    /// chord is a plain SINGLE spec (`"C-f"`, no `C-x`/`C-c` prefix — the
    /// collision only ever touches single Ctrl-letter chords); a bad/unparseable
    /// entry, or one that isn't a single chord, is reported to stderr and
    /// SKIPPED (never a crash), mirroring [`Self::apply_overrides`]'s leniency.
    /// On [`Convention::Mac`] the list is parsed but the set stays
    /// consultable-yet-inert — [`Self::linux_keeps`] gates on convention too, so
    /// even a stray non-empty set can never fire there (belt + suspenders with
    /// the convention check at the call site in [`Self::resolve`]).
    ///
    /// THE INSERT-LINK-YIELDS-TO-KILL-LINE ROUND: clears any prior keep-set
    /// first (so a reload reflects exactly the current file), then ALWAYS
    /// re-seeds `linux_builtin_keep()` before layering `keep` on top — the
    /// built-in floor is UNREMOVABLE by this function, whether called with the
    /// full `Config::effective_linux_keep()` composition, a hand-rolled test
    /// list, or an empty one. This is what makes the floor real even for a
    /// caller (a bare unit test, `linux_emacs_preset_keep()` applied on its
    /// own) that never threads it through `Config` at all.
    pub fn apply_linux_keep(&mut self, keep: &[String]) {
        self.linux_keep.clear();
        for chord in linux_builtin_keep().iter().copied().chain(keep.iter().map(String::as_str)) {
            match parse_binding(chord) {
                Ok(Chord::Single(k, m)) => {
                    self.linux_keep.insert((k, m));
                }
                Ok(_) => {
                    eprintln!(
                        "config linux_keep_emacs: {chord:?}: only a single chord (no C-x/C-c prefix) is supported; ignored"
                    );
                }
                Err(e) => {
                    eprintln!("config linux_keep_emacs: {chord:?}: {e}; ignored");
                }
            }
        }
        self.seed_defaults();
    }

    /// True when `key`+`state` is a chord this Linux keymap has been told to
    /// KEEP emacs' meaning for (see [`Self::apply_linux_keep`]) — the predicate
    /// [`Self::seed_defaults`] consults so a kept chord seeds the emacs meaning
    /// instead of the native one. Always `false` on
    /// [`Convention::Mac`] ("Mac convention ignores the key entirely" — the
    /// law), independent of whether the keep-set happens to be populated.
    fn linux_keeps(&self, key: &Key, state: ModifiersState) -> bool {
        self.convention == Convention::Linux && self.linux_keep.contains(&(canon_key(key), state))
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
        self.override_single
            .keys()
            .any(|(mk, ms)| *mk == k && ms.contains(ModifiersState::ALT))
    }

    /// Resolve a key event to an `Action`, updating prefix state. `mods` is the
    /// current modifier state; `logical` is the winit logical key.
    pub fn resolve(&mut self, logical: &Key, mods: &Modifiers) -> Action {
        let state = mods.state();
        // A configured one-chord binding wins over the seeded catalog default.
        // `SmolStr` keeps ordinary single-key canonicalization inline; no map or
        // binding allocation occurs after construction. Only when NOT mid-prefix.
        if !self.in_c_x && !self.in_c_c {
            let chord = (canon_key(logical), state);
            if let Some(a) = self.override_single.get(&chord) {
                return a.clone();
            }
            if let Some(a) = self.default_single.get(&chord) {
                return a.clone();
            }
        }
        let ctrl = state.contains(ModifiersState::CONTROL);
        // On mac, Option (Alt) is used for Meta-style word motion; treat ALT as
        // Meta. SUPER (Cmd / "Logo") drives the mac-native zoom shortcuts.
        let alt = state.contains(ModifiersState::ALT);
        let sup = state.contains(ModifiersState::SUPER);
        let shift = state.contains(ModifiersState::SHIFT);
        // THE LINUX-NATIVE KEYMAP: `native` is [`Self::native_down`] — Super-without-
        // Control on Mac, Control-without-Super on Linux — the ONE convention-
        // resolved gate every "native slot 1" arm below now reads (was a bare
        // `sup && !ctrl`). Nothing about WHERE these arms sit in `resolve` changed,
        // which is what makes "native wins on collision" true for free: a Linux
        // This gate now serves only uncatalogued native aliases; catalog default
        // collision precedence was already resolved when `seed_defaults` ran.
        //
        // THE EMACS-HANDS-ON-LINUX PER-CHORD DOOR: `&& !self.linux_keeps(..)`
        // (a no-op on Mac and on an empty/absent `linux_keep_emacs` config — see
        // `linux_keeps`'s doc) exempts THIS EXACT chord from the native-wins
        // collision for uncatalogued aliases too, matching the seeded catalog map.
        let native = self.native_down(state) && !self.linux_keeps(logical, state);

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
            let chord = (canon_key(logical), state);
            if let Some(a) = self.override_c_x.get(&chord) {
                return a.clone();
            }
            if let Some(a) = self.default_c_x.get(&chord) {
                return a.clone();
            }
            return Action::Cancel;
        }

        // MID-PREFIX (C-c ...): the org-mode-style second prefix, mirroring the
        // C-x block above exactly. A configured `C-c <key>` rebind wins over the
        // data-backed default map; an unbound `C-c <combo>` cancels + clears.
        if self.in_c_c {
            self.in_c_c = false;
            let chord = (canon_key(logical), state);
            if let Some(a) = self.override_c_c.get(&chord) {
                return a.clone();
            }
            if let Some(a) = self.default_c_c.get(&chord) {
                return a.clone();
            }
            return Action::Cancel;
        }

        // Some layouts report the shifted glyph rather than the catalogued base
        // key. These are input-normalization aliases, not second default values.
        if native {
            match logical {
                Key::Character(s) if s.as_str() == "+" => return Action::ZoomIn,
                Key::Character(s) if s.as_str() == "_" => return Action::ZoomOut,
                _ => {}
            }
        }

        // Cmd-P (Super+P): summon the COMMAND PALETTE. This is its OWN dedicated
        // key — NOT a C-x chord — so it never disturbs the prefix bindings. 'p' is
        // free under Super (undo=z, zoom ==/+/-/0, clipboard=c/x/v), so no
        // collision. Plain (no Shift) — Cmd-Shift-P is Switch project, above.
        if native {
            if let Key::Character(s) = logical {
                if matches!(s.chars().next(), Some('p') | Some('P')) {
                    return Action::OpenCommandPalette;
                }
            }
        }

        // Cmd-. (Super+'.', no Shift): CANCEL — the HIG's ancient cancel synonym
        // (predates Esc on the Mac; every dialog still honors it). Quiet: no
        // menu label, no palette entry, no advertisement — just the chord a Mac
        // hand reaches for without thinking (P4). `!shift` distinguishes it from
        // Cmd-Shift-. (ToggleHiddenFiles) above, so the two never collide.
        if native && !shift {
            if let Key::Character(s) = logical {
                if s.chars().next() == Some('.') {
                    return Action::Cancel;
                }
            }
        }

        // Option-Cmd-I (Super+Alt+'i'): SUMMON the held STATS HUD while the key is
        // held (the live press/release pair holds + dismisses it; here we map the
        // press to the action). MOVED off plain Cmd-I (the keybinding-idiom
        // audit's Option B, user-decided): every Mac writing app spends bare
        // Cmd-I on Italic, so the HUD relocates to the macOS inspector/info
        // idiom (⌥⌘I opens Get Info in Finder) — `i` still for "info", ⌥ still
        // reads as "more/inspect". No tap-vs-hold machinery; this is a single,
        // ordinary chord like any other, just gated on Alt. The HUD is
        // HOLD-ONLY: it is deliberately NOT a palette command (a discrete
        // selection could not be released to dismiss it), so this is its sole
        // summon. See `hud.rs`.
        if native && alt {
            if let Key::Character(s) = logical {
                if matches!(s.chars().next(), Some('i') | Some('I')) {
                    return Action::ShowStatsHud;
                }
            }
        }

        // Some layouts report the shifted glyph for the catalogued Cmd-Shift-.
        // chord. Normalize that alternate logical-key spelling at the input edge.
        if native && shift && matches!(logical, Key::Character(s) if s.as_str() == ">") {
            return Action::ToggleHiddenFiles;
        }

        // Cmd-Option-F is a legacy uncatalogued alias for the catalogued Cmd-R
        // Find-and-replace door.
        if native && alt {
            if let Key::Character(s) = logical {
                if matches!(s.chars().next(), Some('f') | Some('F')) {
                    return Action::OpenReplace;
                }
            }
        }

        // Cmd-G / Cmd-Shift-G: FIND NEXT / PREVIOUS — the deeper macOS idiom
        // (TextEdit, Safari, Notes, Pages, Xcode, BBEdit all step this way, not
        // "press Find again"; P2). Literal ALIASES of the SAME `SearchForward`/
        // `SearchBackward` actions Cmd-F/Cmd-Shift-F fire: with no search open
        // this OPENS one — prefilled from an active selection, else the
        // REMEMBERED last query (`actions/motion.rs::start_search`), so a bare
        // Cmd-G after a prior search's panel closed genuinely re-finds. While a
        // panel is already open the live App routes keys to `handle_search_key`
        // instead, which carries its own mirrored Cmd-G/Shift-Cmd-G arm (a plain
        // step, like its Cmd-F/Shift-Cmd-F arm). 'g' is free under Super (the
        // used set is z, =/+/-/0, p, o, c/x/v, f, r, a, b, e, ';', w, ,), so no
        // collision. Case-folded.
        if native && !alt {
            if let Key::Character(s) = logical {
                if matches!(s.chars().next(), Some('g') | Some('G')) {
                    return if shift { Action::SearchBackward } else { Action::SearchForward };
                }
            }
        }

        match logical {
            Key::Named(named) => self.resolve_named(*named, ctrl, alt, state),
            Key::Character(s) => self.resolve_char(s, ctrl, alt, sup),
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
        // Plain arrows and convention-independent Ctrl-arrow word aliases are input
        // policy. Catalogued Option/Cmd arrow defaults have already resolved through
        // the data-backed map above.
        let sup = state.contains(ModifiersState::SUPER);
        match named {
            NamedKey::ArrowLeft => {
                if state.contains(ModifiersState::CONTROL) {
                    Action::BackwardWord
                } else {
                    Action::BackwardChar
                }
            }
            NamedKey::ArrowRight => {
                if state.contains(ModifiersState::CONTROL) {
                    Action::ForwardWord
                } else {
                    Action::ForwardChar
                }
            }
            NamedKey::ArrowUp => Action::PreviousLine,
            NamedKey::ArrowDown => Action::NextLine,
            // THE LINUX-NATIVE override for "Document start"/"Document end"
            // (`commands::LINUX_NATIVE_OVERRIDE`): Ctrl-Home/Ctrl-End is the
            // gedit/VS Code/GTK convention for buffer start/end — NOT the naive
            // Cmd→Ctrl translation of Cmd-Up/Down (which would land on Ctrl-Up/Down,
            // an unclaimed but non-idiomatic chord). Convention-gated (never fires
            // on Mac, where Cmd-Up/Down already owns this) and CHECKED BEFORE the
            // unconditional Home/End arms below, so plain Home/End keep meaning
            // line start/end on every convention — only the CTRL-held combination
            // differs by convention.
            NamedKey::Home if self.convention == Convention::Linux && ctrl => Action::BufferStart,
            NamedKey::End if self.convention == Convention::Linux && ctrl => Action::BufferEnd,
            // "Line start"/"Line end"'s OWN Linux-native override
            // (`commands::LINUX_NATIVE_OVERRIDE`) is exactly these unconditional
            // arms — Home/End already fire LineStart/LineEnd on every convention
            // with no modifier needed, so no further keymap change was needed there.
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

    fn resolve_char(&mut self, s: &str, ctrl: bool, alt: bool, sup: bool) -> Action {
        // We key off the first char of the logical string. For control combos
        // winit still reports the base character (e.g. "f" for C-f).
        let Some(c) = s.chars().next() else {
            return Action::Ignore;
        };
        let lower = c.to_ascii_lowercase();

        if ctrl && !alt {
            return match lower {
                'd' => Action::DeleteForward,
                'k' => Action::KillLine,
                'v' => Action::PageScrollDown,
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

        // THE UNBOUND-SUPER SWALLOW GUARD (keybinding audit, 2026-07): every bound
        // Cmd-<x> chord already returned earlier in `resolve` (Cmd-Z, Cmd-S, zoom,
        // Cmd-P, Cmd-B/I/E, …) or via a `[keys]` override (consulted before dispatch
        // ever reaches here). Reaching here WITH Super held means the chord truly
        // has no meaning — mac convention is that an unhandled Cmd combo is inert
        // (at most a beep), never text, so ⌘H/⌘K/⌘D/… must NOT type their letter
        // into the document. This intentionally also swallows Cmd+Option combos
        // (Option's dead-key composition doesn't apply once Cmd is held — a
        // Cmd-chord reads as a shortcut attempt, not typing) and Cmd+Control
        // combos with no ctrl arm above. A bare Control chord (no Super) is NOT
        // affected — it already fell through the `ctrl && !alt` match above with
        // its own `Ignore` default.
        //
        // ⌘K WAS RESERVED here (unbound, falling into this guard) since the
        // keybinding-idiom audit's W1 — Bear/Craft/Notion/Things/Ulysses/Slack all
        // spend Cmd-K on insert/edit-link, the single strongest writer-cluster
        // chord awl didn't yet claim. LINKS V2 spent it: Cmd-K now resolves to
        // `Action::InsertLink` in the native-doors block above, so it no longer
        // reaches this guard.
        if sup {
            return Action::Ignore;
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

        // No control/meta/super: a self-inserting printable character. Filter out
        // control characters defensively.
        if !c.is_control() {
            Action::InsertChar(c)
        } else {
            Action::Ignore
        }
    }
}

fn insert_default_entry(
    map: &mut HashMap<(Key, ModifiersState), Action>,
    chord: (Key, ModifiersState),
    action: Action,
    name: &str,
    spec: &str,
) {
    if let Some(existing) = map.get(&chord) {
        assert_eq!(
            existing, &action,
            "assets/keymap-defaults.toml: conflicting effective default {spec:?} for {name:?}: {existing:?} versus {action:?}"
        );
        return;
    }
    map.insert(chord, action);
}

fn linux_keeps_chord_raw(
    keep: &std::collections::HashSet<(Key, ModifiersState)>,
    chord_spec: &str,
) -> bool {
    match parse_binding(chord_spec) {
        Ok(Chord::Single(k, m)) => keep.contains(&(k, m)),
        _ => false,
    }
}

fn linux_displaces_emacs_default_raw(
    emacs: &str,
    keep: &std::collections::HashSet<(Key, ModifiersState)>,
) -> bool {
    let Some(first) = emacs.split_whitespace().next() else {
        return false;
    };
    let Ok((key, mods)) = crate::keyspec::parse_chord(first) else {
        return false;
    };
    if mods.state() != ModifiersState::CONTROL {
        return false;
    }
    let Key::Character(s) = &key else {
        return false;
    };
    s.chars().next().is_some_and(|c| {
        LINUX_DISPLACED_LETTERS.contains(&c.to_ascii_lowercase())
            && !keep.contains(&(canon_key(&key), mods.state()))
    })
}

// ── THE LINUX-NATIVE COLLISION TABLE (user-approved policy: NATIVE WINS) ───────
//
// Under `Convention::Linux`, slot 1 translates to Ctrl, where it collides with a
// handful of bare-Control emacs slots. `seed_defaults` applies this table before
// inserting either claimant: native wins unless the exact emacs chord is kept.
// A `[keys]` binding remains able to reclaim either meaning because overrides are
// consulted before the seeded default map.
//
// The FULL, exhaustively-computed displaced list (verified by
// `tests::linux_collision_table_matches_the_documented_displaced_list`, which
// drives a REAL `Convention::Linux` `KeymapState` over every bare Ctrl+letter and
// asserts every chord OUTSIDE this list still resolves IDENTICALLY between
// conventions — so a future emacs default that starts colliding fails this test
// until it is adjudicated here):
//
//   Ctrl-S: Save               displaces  C-s: Search forward (emacs slot 2)
//   Ctrl-P: Command palette    displaces  C-p: Previous line (static arm)
//   Ctrl-N: New note           displaces  C-n: Next line (static arm)
//   Ctrl-W: Finish file        displaces  C-w: Cut (emacs slot 2)
//   Ctrl-F: Search forward     displaces  C-f: Forward char (static arm)
//   Ctrl-E: Inline code        displaces  C-e: Line end (emacs slot 2)
//   Ctrl-A: Select all         displaces  C-a: Line start (emacs slot 2)
//   Ctrl-G: Search forward*    displaces  C-g: Cancel (static arm)
//   Ctrl-R: Find and replace   displaces  C-r: Search backward (emacs slot 2)
//   Ctrl-B: Bold               displaces  C-b: Backward char (static arm)
//   Ctrl-C: Copy               displaces  C-c: the bare C-c PREFIX (its only
//                                          default sub-binding, C-c C-o = Follow
//                                          link, is Follow link's OWN emacs slot —
//                                          so Follow link loses its default chord
//                                          entirely on Linux, restorable via
//                                          `[keys] follow_link = "C-c C-o"`)
//   Ctrl-X: Cut                displaces  C-x: the bare C-x PREFIX (carries no
//                                          default sub-bindings of its own post-
//                                          identity-round, but a `[keys]` "C-x
//                                          <key>" override becomes unreachable via
//                                          a bare Ctrl-X first key on Linux, since
//                                          it now resolves immediately instead of
//                                          arming the prefix — a genuine, logged
//                                          product consequence, not an oversight)
//   Ctrl-V: Paste               displaces  C-v: Page scroll down (static arm)
//
//   * Ctrl-G's native meaning is "find next" (a literal alias of Search forward,
//     matching Cmd-G's own mac behavior) — its "displaced" victim is the SAME
//     action Cancel, so the practical loss is losing Ctrl-G as a Cancel synonym
//     (C-g fully retired as Cancel's chord on Linux); Escape and the native
//     Cmd-.-turned-Ctrl-. arm both still cancel.
//
// NOT displaced, despite appearing in illustrative examples elsewhere: Ctrl-D
// (Delete forward) — no command ever bound Cmd-D per its own A1 refusal, so it
// keeps its emacs meaning UNCHANGED on Linux too.
//
// Ctrl-K (Insert link) is a THIRD, DELIBERATELY DIFFERENT case from either of
// those two — LINKS V2 (see `Action::InsertLink`'s doc) spent Cmd-K, which
// WOULD have put `k` on the displaced-letters list above exactly like every
// other native-doors chord, but the user rejected that trade outright: kill-
// line is too load-bearing for emacs hands to lose by default. So `k` is NOT
// in `LINUX_DISPLACED_LETTERS` — instead `linux_builtin_keep()` (below) names it
// as an UNCONDITIONAL keep, seeded on every `KeymapState::apply_linux_keep`
// call regardless of `linux_keep_emacs`/the `keymap` flavor. The practical
// upshot: Ctrl-K stays kill-line out of the box on Linux, in BOTH keymap
// flavors, with NO config needed — Insert link simply has no effective Linux
// binding by default (still one `[keys] insert_link = "C-k"` line away for a
// Linux hand who explicitly wants the trade; see `commands.rs`'s catalog
// entry).

/// The LETTERS the table above displaces (every `Ctrl-<letter>` whose native
/// meaning wins on [`Convention::Linux`]) — the ONE data owner both
/// `tests::linux_collision_table_matches_the_documented_displaced_list` (which
/// still separately pins EACH letter's resolved `Action`) and
/// [`linux_displaces_emacs_default`] (the LABEL-TRUTH half — is an emacs
/// default worth SHOWING under this convention) read, so the dispatch table and
/// the label truth can never silently drift apart. `k` is deliberately NOT
/// here — see `linux_builtin_keep()`'s doc for why Insert link's Ctrl-K is a
/// third, unconditionally-kept case rather than an ordinary displaced letter.
pub(crate) const LINUX_DISPLACED_LETTERS: &[char] =
    &['s', 'p', 'n', 'w', 'f', 'e', 'a', 'g', 'r', 'b', 'c', 'x', 'v'];

/// THE INSERT-LINK-YIELDS-TO-KILL-LINE ROUND (settled — the user's own call:
/// "kill-line is too load-bearing for emacs hands to lose by default") — chords
/// that keep their EMACS meaning on [`Convention::Linux`] UNCONDITIONALLY,
/// independent of `linux_keep_emacs`/the `keymap` flavor preset. Currently just
/// `C-k` (Kill line survives Links v2's Cmd-K spend): unlike every letter in
/// [`LINUX_DISPLACED_LETTERS`] (which a user must opt BACK into via
/// `linux_keep_emacs`/`keymap = "emacs"` to keep), `C-k` never displaces at
/// all out of the box, on EITHER keymap flavor — the native Insert-link chord
/// simply has NO effective Linux binding by default (still one `[keys]
/// insert_link = "C-k"` line away for a Linux hand who explicitly wants the
/// trade — a `[keys]` override is consulted before this floor, same as every
/// other override).
///
/// Consumed from TWO structurally separate places that must agree (mirroring
/// how [`LINUX_DISPLACED_LETTERS`] itself already feeds both the dispatch
/// table and the label-truth functions): [`KeymapState::apply_linux_keep`]
/// seeds it UNCONDITIONALLY on every call (the dispatch half — a reload can
/// never clear it away) and [`crate::config::Config::effective_linux_keep`]
/// seeds it into the composed keep-list it returns (the label half —
/// `commands::join_slots_truthful` never touches `KeymapState` directly, so
/// it needs its own copy of the same guarantee). `Convention::Mac` never
/// consults `linux_keep` at all, so this is structurally inert there — Cmd-K
/// stays Insert link on Mac, unconditionally.
///
/// THE KEYMAP-DEFAULTS-AS-DATA ROUND: this is now a thin accessor over
/// [`crate::keymap_defaults::linux_builtin_keep`] (itself parsed once from
/// the embedded `assets/keymap-defaults.toml`'s `linux_builtin_keep` array)
/// rather than a literal `const` — the value (`["C-k"]`) is unchanged, only
/// where it lives moved, so every call site needed only `()` added.
pub(crate) fn linux_builtin_keep() -> &'static [&'static str] {
    crate::keymap_defaults::linux_builtin_keep()
}

/// THE WEB CHORD SANITY ROUND, Tier 3 — is `emacs` (a command's static slot-2
/// text, e.g. `"C-s"` or the `"C-c C-o"` prefix sequence) quietly DISPLACED under
/// [`Convention::Linux`]? Checks only the emacs default's FIRST key: a bare
/// (no Shift/Alt/Super) `Ctrl-<letter>` whose letter appears in
/// [`LINUX_DISPLACED_LETTERS`] is displaced — this covers both a single-chord
/// default (`"C-s"`) and a prefix sequence whose FIRST key is itself claimed
/// (`"C-c C-o"`: Ctrl-C now resolves straight to Copy, so the whole sequence
/// never arms). `false` for an empty/unparsable emacs slot, or a modified chord
/// (`"C-/"`, `"C-y"`) outside the displaced-letter set.
///
/// `keep` is the config `linux_keep_emacs` list (THE EMACS-HANDS-ON-LINUX
/// per-chord door) — a chord named there is NEVER displaced, regardless of
/// whether its letter is in [`LINUX_DISPLACED_LETTERS`] (checked via
/// [`linux_keeps_chord`], the SAME canonical-compare helper the label owner's
/// native-suppression half uses, so the two directions of this round's fix can
/// never disagree about what "kept" means). Pure — the label-truth owner
/// (`commands::join_slots_truthful`) is the only caller; mirrors the dispatch
/// collision table structurally, never re-derives it.
pub(crate) fn linux_displaces_emacs_default(emacs: &str, keep: &[String]) -> bool {
    let Some(first) = emacs.split_whitespace().next() else {
        return false;
    };
    let Ok((key, mods)) = crate::keyspec::parse_chord(first) else {
        return false;
    };
    if mods.state() != ModifiersState::CONTROL {
        return false; // must be a BARE Ctrl chord — no Shift/Alt/Super riders.
    }
    let Key::Character(s) = &key else {
        return false;
    };
    let letter_displaced =
        s.chars().next().is_some_and(|c| LINUX_DISPLACED_LETTERS.contains(&c.to_ascii_lowercase()));
    letter_displaced && !linux_keeps_chord(keep, first)
}

/// Is `chord_spec` (a raw chord string, e.g. `"C-f"` or a command's resolved
/// native chord like `"Ctrl-F"`) present in the LINUX KEEP-LIST `keep`, compared
/// CANONICALLY ([`crate::keyspec::canonical_binding`], so `"C-f"` == `"Ctrl-f"`
/// == `"Control-F"`)? `false` for an empty/unparsable `chord_spec` on EITHER
/// side. The ONE comparison both halves of the emacs-hands-on-Linux label fix
/// share: [`linux_displaces_emacs_default`] (does a kept chord stop displacing
/// the emacs default?) and `commands::join_slots_truthful`'s native-suppression
/// check (does a kept chord stop the NATIVE command from advertising it?) — so
/// the two directions can never quietly disagree about what "kept" means.
pub(crate) fn linux_keeps_chord(keep: &[String], chord_spec: &str) -> bool {
    let Some(want) = crate::keyspec::canonical_binding(chord_spec) else {
        return false;
    };
    keep.iter().any(|k| crate::keyspec::canonical_binding(k).as_deref() == Some(want.as_str()))
}

/// THE KEYMAP FLAVOR ROUND — a config `keymap = "native" | "emacs"` PRESET,
/// orthogonal to [`Convention`] (which decides whether slot 1 SPEAKS ⌘-chords
/// or Ctrl-chords). `Native` (the default) is today's behavior byte-identical.
/// `Emacs` widens the emacs-hands-on-Linux `linux_keep_emacs` PER-CHORD door
/// (see [`KeymapState::apply_linux_keep`]/[`linux_keeps_chord`] above) into a
/// whole-catalog PRESET: every chord [`LINUX_DISPLACED_LETTERS`] names keeps
/// its emacs meaning, unioned with the user's own explicit `linux_keep_emacs`
/// entries — see `crate::config::Config::effective_linux_keep`, THE ONE
/// COMPOSITION OWNER (this module stays unaware of the config field entirely;
/// it only ever sees the already-composed `keep` list `with_overrides_and_keep`/
/// `apply_linux_keep` take). Inert on [`Convention::Mac`] structurally, same as
/// `linux_keep_emacs` itself — no collisions exist there to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeymapFlavor {
    #[default]
    Native,
    Emacs,
}

impl KeymapFlavor {
    /// Parse a config `keymap` value (case-insensitive). An unrecognized string
    /// (including empty) -> `None`, so the caller keeps the built-in default
    /// (`Native`) — mirrors [`crate::config::parse_caret_mode`]'s leniency.
    pub fn parse(s: &str) -> Option<KeymapFlavor> {
        match s.trim().to_ascii_lowercase().as_str() {
            "native" => Some(KeymapFlavor::Native),
            "emacs" => Some(KeymapFlavor::Emacs),
            _ => None,
        }
    }

    /// The config NAME this flavor writes/reads as (the inverse of [`Self::parse`]).
    pub fn config_name(self) -> &'static str {
        match self {
            KeymapFlavor::Native => "native",
            KeymapFlavor::Emacs => "emacs",
        }
    }
}

/// The `Emacs` flavor's PRESET keep-list: every `Ctrl-<letter>` chord
/// [`LINUX_DISPLACED_LETTERS`] names, formatted as a plain single-chord spec
/// (`"C-f"`) ready for [`KeymapState::apply_linux_keep`]/[`linux_keeps_chord`].
/// Derived FROM the displaced-letters table itself — NEVER hand-copied — so a
/// future change to the collision table flows into the preset automatically
/// (the no-drift law this round's tests pin: the preset always equals the
/// displaced set, letter for letter). Deliberately does NOT include `C-k` —
/// `linux_builtin_keep()` covers it unconditionally, on EITHER flavor, so it
/// has no business in a flavor-gated preset; `Config::effective_linux_keep`
/// unions both in regardless of which flavor is active.
pub fn linux_emacs_preset_keep() -> Vec<String> {
    LINUX_DISPLACED_LETTERS.iter().map(|c| format!("C-{c}")).collect()
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

    /// The live catalog rendered as one `slug|native@mac|native@linux|emacs`
    /// line per command, newline-terminated — the value feeding the frozen
    /// `catalog_chord_snapshot_is_frozen` guard. `native@mac`/`native@linux` go
    /// through `commands::resolved_native` (the one owner of the Cmd->Ctrl
    /// translation + override table); the emacs slot is convention-agnostic text.
    fn catalog_chord_snapshot() -> String {
        let mut out = String::new();
        for c in crate::commands::COMMANDS.iter() {
            out.push_str(&format!(
                "{}|{}|{}|{}\n",
                crate::commands::slug(c.name),
                crate::commands::resolved_native(c, Convention::Mac),
                crate::commands::resolved_native(c, Convention::Linux),
                c.emacs,
            ));
        }
        out
    }

    const CATALOG_CHORD_SNAPSHOT: &str = "\
go_to_file|Cmd-O|C-o|
switch_project|Cmd-S-p|C-S-p|
recent_projects|||
browse_files|||
go_to_heading|||
spell_suggestions|Cmd-;|C-;|
version_history|Cmd-S-h|C-S-h|
compare_with_version|||
clean_unused_assets|||
keep_version|||
last_file|C-Tab|C-Tab|
new_note|Cmd-N|C-n|
move_note|||
rename_note|||
duplicate_note|||
finish_file|Cmd-W|C-w|
follow_link|||C-c C-o
switch_theme|Cmd-T|C-t|
caret_style|||
dictionary|||
toggle_spellcheck|||
toggle_hidden_files|Cmd-S-.|C-S-.|
toggle_caret_style|||
toggle_page_mode|||
toggle_writing_nits|||
widen_page|||
narrow_page|||
reset_page_width|||
toggle_debug|||
toggle_outline|Cmd-S-o|C-S-o|
toggle_typewriter_scroll|||
toggle_menu_bar|||
about|||
credits|||
guide|||
lifetime_stats|||
writing_streaks|||
line_endings|||
align_table|||
insert_date|||
report_a_problem|||
download_file|||
check_for_updates|||
blockquote|||
bullet_list|||
numbered_list|||
task_list|Cmd-S-l|C-S-l|
heading|||
cycle_heading|||
code_block|||
bold|Cmd-B|C-b|
italic|Cmd-I|C-i|
inline_code|Cmd-E|C-e|
highlight|||
strikethrough|||
export_as_word|||
export_as_html|||
export_as_pdf|||
insert_link|Cmd-K|C-k|
save|Cmd-S|C-s|
quit|Cmd-Q|C-q|
search_forward|Cmd-F|C-f|C-s
search_backward|Cmd-S-f|C-S-f|C-r
find_and_replace|Cmd-R|C-r|
undo|Cmd-Z|C-z|C-/
redo|Cmd-S-z|C-S-z|
copy|Cmd-C|C-c|
cut|Cmd-X|C-x|C-w
paste|Cmd-V|C-v|C-y
select_all|Cmd-A|C-a|
zoom_in|Cmd-=|C-=|
zoom_out|Cmd--|C--|
reset_zoom|Cmd-0|C-0|
forward_word|M-Right|M-Right|
backward_word|M-Left|M-Left|
line_start|Cmd-Left|Home|C-a
line_end|Cmd-Right|End|C-e
document_start|Cmd-Up|C-Home|
document_end|Cmd-Down|C-End|
forward_char|||C-f
backward_char|||C-b
next_line|||C-n
previous_line|||C-p
delete_word_forward|||
delete_word_backward|||
settings|Cmd-,|C-,|
keybindings|||
";

    // CONVENTION-PROOF SHADOW: the vast majority of this module's tests build a
    // bare `KeymapState::new()`/`with_overrides(..)` and assert MAC-native
    // outcomes (`"Cmd-…"`-shaped expectations, retired-emacs-default checks,
    // …) — pinning the DEFAULT construction door to `Convention::Mac` inside
    // this test module is the honest fix (these tests document specifically
    // what a MAC-convention keymap does; Linux's own collision/displacement
    // behavior is separately, exhaustively law-tested by
    // `linux_collision_table_matches_the_documented_displaced_list` and its
    // neighbors below, which all call `new_with_convention`/
    // `with_overrides_and_convention` EXPLICITLY and are therefore untouched by
    // this shadow). A thin newtype + `Deref`/`DerefMut` to the real
    // `super::KeymapState` lets every existing `km.resolve(..)`/`km.in_prefix()`
    // call site keep working unchanged; only the two DEFAULT constructors are
    // overridden — the two EXPLICIT-convention constructors forward their
    // argument verbatim, so a test that already pins `Convention::Linux`
    // explicitly is completely unaffected.
    struct KeymapState(super::KeymapState);
    impl KeymapState {
        fn new() -> Self {
            Self(super::KeymapState::new_with_convention(Convention::Mac))
        }
        fn with_overrides(keys: &[(String, Vec<String>)]) -> Self {
            Self(super::KeymapState::with_overrides_and_convention(keys, Convention::Mac))
        }
        fn new_with_convention(convention: Convention) -> Self {
            Self(super::KeymapState::new_with_convention(convention))
        }
        fn with_overrides_and_convention(keys: &[(String, Vec<String>)], convention: Convention) -> Self {
            Self(super::KeymapState::with_overrides_and_convention(keys, convention))
        }
    }
    impl std::ops::Deref for KeymapState {
        type Target = super::KeymapState;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }
    impl std::ops::DerefMut for KeymapState {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.0
        }
    }

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
    fn both_convention_modifiers_keep_the_data_backed_emacs_fallback() {
        let both = mods(ModifiersState::CONTROL | ModifiersState::SUPER);
        for convention in [Convention::Mac, Convention::Linux] {
            let mut km = KeymapState::new_with_convention(convention);
            assert_eq!(km.resolve(&ch("f"), &both), Action::ForwardChar);
            assert_eq!(km.resolve(&ch("s"), &both), Action::SearchForward);
            assert_eq!(km.resolve(&ch("c"), &both), Action::BeginPrefix);
            assert_eq!(km.resolve(&ch("o"), &both), Action::FollowLink);
        }
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
    fn cmd_w_finishes_file_and_cmd_comma_opens_settings() {
        // P5: Cmd-W (Super+'w') = Finish file — awl's closest analogue to
        // "close the document". P1: Cmd-, (Super+',') = Settings — the
        // preferences idiom since Mac OS X 10.1. Both case-folded where
        // applicable; neither is a motion or an edit.
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("w"), &sup()), Action::FinishBuffer);
        assert_eq!(km.resolve(&ch("W"), &sup()), Action::FinishBuffer);
        assert_eq!(km.resolve(&ch(","), &sup()), Action::OpenSettingsMenu);
        // Plain 'w'/',' (no Super) are unshadowed.
        assert_eq!(km.resolve(&ch("w"), &none()), Action::InsertChar('w'));
        assert_eq!(km.resolve(&ch(","), &none()), Action::InsertChar(','));
        for a in [Action::FinishBuffer, Action::OpenSettingsMenu] {
            assert!(!a.is_motion());
            assert!(!a.is_edit());
        }
    }

    #[test]
    fn cmd_period_cancels_quietly() {
        // P4: Cmd-. (Super+'.', no Shift) is the HIG's ancient cancel synonym —
        // quiet, no menu label, no palette entry. Cmd-Shift-. stays
        // ToggleHiddenFiles (the Finder convention), unaffected.
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("."), &sup()), Action::Cancel);
        assert_eq!(km.resolve(&ch("."), &sup_shift()), Action::ToggleHiddenFiles);
        assert_eq!(km.resolve(&ch(">"), &sup_shift()), Action::ToggleHiddenFiles);
        // Plain '.' (no Super) still self-inserts.
        assert_eq!(km.resolve(&ch("."), &none()), Action::InsertChar('.'));
    }

    #[test]
    fn cmd_shift_l_toggles_task_list() {
        // W3: Cmd-Shift-L — Apple Notes' checklist idiom. A plain Cmd-L (the
        // BBEdit/Xcode go-to-line convention awl declines) stays unbound.
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("L"), &sup_shift()), Action::ToggleTaskList);
        assert_eq!(km.resolve(&ch("l"), &sup_shift()), Action::ToggleTaskList);
        assert_eq!(km.resolve(&ch("l"), &sup()), Action::Ignore, "plain Cmd-L stays unbound");
        assert_eq!(km.resolve(&ch("l"), &none()), Action::InsertChar('l'));
        assert!(!Action::ToggleTaskList.is_motion());
        assert!(Action::ToggleTaskList.is_edit());
    }

    #[test]
    fn cmd_k_opens_insert_link() {
        // LINKS V2 — the chord the keybinding-idiom audit reserved for exactly
        // this. Case-folded; plain 'k' (no Super) still self-inserts.
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("k"), &sup()), Action::InsertLink);
        assert_eq!(km.resolve(&ch("K"), &sup()), Action::InsertLink);
        assert_eq!(km.resolve(&ch("k"), &none()), Action::InsertChar('k'));
        assert!(!Action::InsertLink.is_motion());
        assert!(!Action::InsertLink.is_edit());
    }

    #[test]
    fn cmd_g_aliases_search_forward_and_backward() {
        // P2: Cmd-G / Cmd-Shift-G are literal aliases of Cmd-F / Cmd-Shift-F's
        // own actions (SearchForward/SearchBackward) — the deeper macOS
        // find-next/previous idiom.
        let mut km = KeymapState::new();
        assert_eq!(km.resolve(&ch("g"), &sup()), Action::SearchForward);
        assert_eq!(km.resolve(&ch("G"), &sup()), Action::SearchForward);
        assert_eq!(km.resolve(&ch("G"), &sup_shift()), Action::SearchBackward);
        assert_eq!(km.resolve(&ch("g"), &sup_shift()), Action::SearchBackward);
        // Plain 'g' (no Super) self-inserts; C-g (bare Control) is still Cancel.
        assert_eq!(km.resolve(&ch("g"), &none()), Action::InsertChar('g'));
        assert_eq!(km.resolve(&ch("g"), &ctrl()), Action::Cancel);
        // Cmd+Option+G has no arm (Option distinguishes it) — swallowed, not
        // self-inserted (the unbound-super guard).
        assert_eq!(km.resolve(&ch("g"), &sup_alt()), Action::Ignore);
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
        // Plain Cmd-H (no Shift) is NOT the timeline — Shift is required, and it is
        // NOT self-insert either: an unbound Super chord is a calm no-op (the
        // unbound-super swallow guard), never a typed 'h'.
        assert_eq!(km.resolve(&ch("h"), &sup()), Action::Ignore);
        // Plain 'h' (no Super) still self-inserts (the chord didn't shadow it).
        assert_eq!(km.resolve(&ch("h"), &none()), Action::InsertChar('h'));
        // It is neither a motion nor an edit.
        assert!(!Action::OpenHistory.is_motion());
        assert!(!Action::OpenHistory.is_edit());
    }

    #[test]
    fn unbound_super_chords_are_calm_noops() {
        // THE UNBOUND-SUPER SWALLOW GUARD (keybinding audit, 2026-07-09): on macOS
        // an unhandled Cmd combo is inert (at most a beep) — it never types its
        // letter into the document. Every letter/symbol with no default Cmd
        // binding must resolve to Ignore, never InsertChar. 'k' is NO LONGER on
        // this list — LINKS V2 spent Cmd-K on `Action::InsertLink` (see
        // `cmd_k_opens_insert_link`); it is proven bound elsewhere, not unbound
        // here. 'l' likewise stays unbound PLAIN (only Cmd-Shift-L, task list, is
        // bound — see `cmd_shift_l_toggles_task_list`).
        let mut km = KeymapState::new();
        for c in ['d', 'j', 'l', 'u', 'm', 'h'] {
            assert_eq!(
                km.resolve(&ch(&c.to_string()), &sup()),
                Action::Ignore,
                "Cmd-{c} is unbound and must be a calm no-op, not self-insert"
            );
        }
        // An unbound symbol under Cmd is swallowed too.
        assert_eq!(km.resolve(&ch("'"), &sup()), Action::Ignore);
        // Cmd+Option combos with no binding are ALSO swallowed — Option's dead-key
        // composition doesn't compose once Cmd is held, so this reads as an
        // attempted (if unbound) shortcut, not typing.
        assert_eq!(km.resolve(&ch("k"), &sup_alt()), Action::Ignore);
        // Cmd+Control combos with no ctrl arm are swallowed too.
        assert_eq!(
            km.resolve(
                &ch("h"),
                &mods(ModifiersState::SUPER | ModifiersState::CONTROL)
            ),
            Action::Ignore
        );
        // A configured `[keys]` Super rebind still wins over the guard — the
        // override map is consulted before default dispatch ever reaches the
        // swallow check.
        let keys = vec![("go_to_file".to_string(), vec!["Cmd-k".to_string()])];
        let mut km_bound = KeymapState::with_overrides(&keys);
        assert_eq!(km_bound.resolve(&ch("k"), &sup()), Action::OpenGoto);
    }

    #[test]
    fn bare_control_unbound_was_already_a_calm_noop_and_still_is() {
        // Companion to the Super guard above: a BARE Control chord (no Super) with
        // no default `resolve_char` ctrl arm was already `Ignore` before this
        // round (the `ctrl && !alt` match's own default arm) — confirming that
        // half of the audit's ask needed no fix, and pinning it against regressing
        // alongside the new Super guard.
        let mut km = KeymapState::new();
        for c in ['h', 'j', 'l', 'm', 'o', 't', 'u', 'z'] {
            assert_eq!(
                km.resolve(&ch(&c.to_string()), &ctrl()),
                Action::Ignore,
                "C-{c} is unbound and must stay a calm no-op"
            );
        }
        // Plain Option-composed letters keep inserting — typing (dead keys,
        // em-dash, bullet) must never be swallowed by either guard.
        assert_eq!(km.resolve(&ch("g"), &alt()), Action::InsertChar('g'));
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
    fn option_cmd_i_summons_stats_hud_plain_cmd_i_is_italic() {
        let mut km = KeymapState::new();
        // Option-Cmd-I (Super+Alt+'i') summons the held stats HUD — moved off
        // plain Cmd-I (Option B). Case-folded ('i'/'I').
        assert_eq!(km.resolve(&ch("i"), &sup_alt()), Action::ShowStatsHud);
        assert_eq!(km.resolve(&ch("I"), &sup_alt()), Action::ShowStatsHud);
        // Plain Cmd-I (no Alt) is NOT the HUD any more — it is now Italic.
        assert_eq!(km.resolve(&ch("i"), &sup()), Action::Italic);
        assert_eq!(km.resolve(&ch("I"), &sup()), Action::Italic);
        // Plain 'i' (no Super) self-inserts.
        assert_eq!(km.resolve(&ch("i"), &none()), Action::InsertChar('i'));
        // ShowStatsHud is neither a motion nor an edit (hold-only, undo-neutral);
        // Italic is an edit, not a motion.
        assert!(!Action::ShowStatsHud.is_motion());
        assert!(!Action::ShowStatsHud.is_edit());
        assert!(Action::Italic.is_edit());
        assert!(!Action::Italic.is_motion());
    }

    #[test]
    fn cmd_b_i_e_are_the_universal_bold_italic_inline_code_trio() {
        let mut km = KeymapState::new();
        // Cmd-B toggles Bold; Cmd-I toggles Italic; Cmd-E toggles Inline code — the
        // three markdown inline toggles with a universal native convention, all
        // free under plain Super now that the HUD moved to Option-Cmd-I.
        // Case-folded.
        assert_eq!(km.resolve(&ch("b"), &sup()), Action::Bold);
        assert_eq!(km.resolve(&ch("B"), &sup()), Action::Bold);
        assert_eq!(km.resolve(&ch("i"), &sup()), Action::Italic);
        assert_eq!(km.resolve(&ch("I"), &sup()), Action::Italic);
        assert_eq!(km.resolve(&ch("e"), &sup()), Action::InlineCode);
        assert_eq!(km.resolve(&ch("E"), &sup()), Action::InlineCode);
        // Plain 'b'/'i'/'e' (no Super) self-insert — the chords didn't shadow them.
        assert_eq!(km.resolve(&ch("b"), &none()), Action::InsertChar('b'));
        assert_eq!(km.resolve(&ch("i"), &none()), Action::InsertChar('i'));
        assert_eq!(km.resolve(&ch("e"), &none()), Action::InsertChar('e'));
        // All three are edits (they mutate the buffer) and none is a motion.
        assert!(Action::Bold.is_edit());
        assert!(Action::Italic.is_edit());
        assert!(Action::InlineCode.is_edit());
        assert!(!Action::Bold.is_motion());
        assert!(!Action::Italic.is_motion());
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
        assert_eq!(km.resolve(&ch("_"), &sup_shift()), Action::ZoomOut);
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

    // ── THE LINUX-NATIVE KEYMAP ─────────────────────────────────────────────────

    fn sup_ctrl() -> Modifiers {
        mods(ModifiersState::SUPER | ModifiersState::CONTROL)
    }

    fn ctrl_shift() -> Modifiers {
        mods(ModifiersState::CONTROL | ModifiersState::SHIFT)
    }

    fn ctrl_alt() -> Modifiers {
        mods(ModifiersState::CONTROL | ModifiersState::ALT)
    }

    /// THE MAC-BYTE-IDENTICAL LAW: a `Convention::Mac` `KeymapState` resolves
    /// EVERY chord this file's other tests already pin, identically — pinned here
    /// via `new_with_convention` (rather than relying on the ambient compiled
    /// target) so this law holds even if these tests ever ran on a non-mac CI
    /// runner. Spot-checks the widest possible spread: undo/save/zoom/palette/
    /// native-doors/search/select-all/clipboard/formatting chords, PLUS every
    /// bare Ctrl+letter this round's collision table is about (which must stay
    /// their ORIGINAL emacs meaning on Mac, since `native_down` never claims Ctrl
    /// there).
    #[test]
    fn mac_convention_is_byte_identical_to_the_pre_round_table() {
        let mut km = KeymapState::new_with_convention(Convention::Mac);
        assert_eq!(km.resolve(&ch("z"), &sup()), Action::Undo);
        assert_eq!(km.resolve(&ch("s"), &sup()), Action::Save);
        assert_eq!(km.resolve(&ch("p"), &sup()), Action::OpenCommandPalette);
        assert_eq!(km.resolve(&ch("n"), &sup()), Action::NewNote);
        assert_eq!(km.resolve(&ch("w"), &sup()), Action::FinishBuffer);
        assert_eq!(km.resolve(&ch("f"), &sup()), Action::SearchForward);
        assert_eq!(km.resolve(&ch("e"), &sup()), Action::InlineCode);
        assert_eq!(km.resolve(&ch("a"), &sup()), Action::SelectAll);
        assert_eq!(km.resolve(&ch("g"), &sup()), Action::SearchForward);
        assert_eq!(km.resolve(&ch("r"), &sup()), Action::OpenReplace);
        assert_eq!(km.resolve(&ch("b"), &sup()), Action::Bold);
        assert_eq!(km.resolve(&ch("c"), &sup()), Action::CopyRegion);
        assert_eq!(km.resolve(&ch("x"), &sup()), Action::KillRegion);
        assert_eq!(km.resolve(&ch("v"), &sup()), Action::Yank);
        // Every bare Ctrl+letter the Linux table displaces keeps its EMACS meaning
        // unchanged under Mac — nothing here reads Ctrl as native.
        for (letter, want) in [
            ('s', Action::SearchForward),
            ('p', Action::PreviousLine),
            ('n', Action::NextLine),
            ('w', Action::KillRegion),
            ('f', Action::ForwardChar),
            ('e', Action::LineEnd),
            ('a', Action::LineStart),
            ('g', Action::Cancel),
            ('r', Action::SearchBackward),
            ('b', Action::BackwardChar),
            ('v', Action::PageScrollDown),
        ] {
            let mut km2 = KeymapState::new_with_convention(Convention::Mac);
            assert_eq!(km2.resolve(&ch(&letter.to_string()), &ctrl()), want, "Ctrl-{letter} on Mac");
        }
        // C-x / C-c still enter the prefix on Mac.
        let mut km3 = KeymapState::new_with_convention(Convention::Mac);
        assert_eq!(km3.resolve(&ch("x"), &ctrl()), Action::BeginPrefix);
        let mut km4 = KeymapState::new_with_convention(Convention::Mac);
        assert_eq!(km4.resolve(&ch("c"), &ctrl()), Action::BeginPrefix);
    }

    /// THE FROZEN CATALOG-CHORD SNAPSHOT (structural per-command value pinning).
    /// Because catalog labels and dispatch now share ONE seed
    /// (`assets/keymap-defaults.toml`), the exhaustive agreement sweeps
    /// (`commands::tests::catalog_and_keymap_agree_on_every_default_chord`,
    /// `every_catalog_default_slot_dispatches_through_real_keymap_under_both_conventions_and_flavors`)
    /// can no longer catch a wrong default CHORD — they read the same parse on
    /// both sides. This table restores that guard: a checked-in literal of every
    /// command's slug -> resolved chord strings across BOTH slots and BOTH
    /// conventions (`native@mac | native@linux | emacs`). Adding, retyping, or
    /// removing a default chord in the TOML shifts exactly one line here and
    /// fails this test, forcing a conscious re-freeze — nothing about a new
    /// command's chords can slip past silently.
    ///
    /// REGENERATED DELIBERATELY, never auto-synced: run
    /// `cargo test -p awl print_catalog_chord_snapshot -- --ignored --nocapture`,
    /// eyeball the diff, and paste the block below (the `print_full_catalog_snapshot`
    /// precedent). The manual step IS the point — an accidental chord change must
    /// cost a visible, reviewed edit, not a rubber-stamp.
    #[test]
    fn catalog_chord_snapshot_is_frozen() {
        assert_eq!(catalog_chord_snapshot(), CATALOG_CHORD_SNAPSHOT);
    }

    /// Regeneration tool for `catalog_chord_snapshot_is_frozen` — prints the
    /// exact block to paste into `CATALOG_CHORD_SNAPSHOT`. `#[ignore]`d (zero
    /// cost to the normal suite), a reusable tool, not a law with its own
    /// failure mode.
    #[test]
    #[ignore]
    fn print_catalog_chord_snapshot() {
        print!("{}", catalog_chord_snapshot());
    }

    /// THE DISPLACED-LIST LAW: drives a REAL `Convention::Linux` `KeymapState`
    /// over every documented collision chord (the table above the keep helpers)
    /// and asserts it resolves to the NATIVE meaning — then sweeps every OTHER
    /// bare Ctrl+letter the static emacs table binds and asserts THOSE resolve
    /// IDENTICALLY to the Mac-convention table (nothing outside the documented
    /// list drifted). A future emacs default that starts colliding on Linux fails
    /// the second half of this test until it is adjudicated into the table above.
    #[test]
    fn linux_collision_table_matches_the_documented_displaced_list() {
        let displaced: &[(char, Action)] = &[
            ('s', Action::Save),
            ('p', Action::OpenCommandPalette),
            ('n', Action::NewNote),
            ('w', Action::FinishBuffer),
            ('f', Action::SearchForward),
            ('e', Action::InlineCode),
            ('a', Action::SelectAll),
            ('g', Action::SearchForward), // "find next" — same action as Cmd-G
            ('r', Action::OpenReplace),
            ('b', Action::Bold),
            ('v', Action::Yank),
        ];
        for (letter, want) in displaced {
            let mut km = KeymapState::new_with_convention(Convention::Linux);
            assert_eq!(
                km.resolve(&ch(&letter.to_string()), &ctrl()),
                *want,
                "Ctrl-{letter} on Linux must resolve to the native meaning"
            );
        }
        // Ctrl-C / Ctrl-X: native Copy/Cut win over the bare prefixes.
        let mut kc = KeymapState::new_with_convention(Convention::Linux);
        assert_eq!(kc.resolve(&ch("c"), &ctrl()), Action::CopyRegion);
        let mut kx = KeymapState::new_with_convention(Convention::Linux);
        assert_eq!(kx.resolve(&ch("x"), &ctrl()), Action::KillRegion);

        // NOT displaced (no native chord claims these letters on Linux either):
        // Ctrl-K and Ctrl-D keep their ordinary emacs meaning, unchanged — Ctrl-K
        // via `linux_builtin_keep()`'s unconditional floor (Links v2 spent Cmd-K,
        // which would otherwise have claimed it exactly like every other
        // native-doors letter; the user's own call kept kill-line by default
        // instead — see the collision-table doc above), Ctrl-D because no command
        // ever bound Cmd-D at all.
        let mut kk = KeymapState::new_with_convention(Convention::Linux);
        assert_eq!(kk.resolve(&ch("k"), &ctrl()), Action::KillLine);
        let mut kd = KeymapState::new_with_convention(Convention::Linux);
        assert_eq!(kd.resolve(&ch("d"), &ctrl()), Action::DeleteForward);

        // The FULL bare-control letter roster from `resolve_char`'s emacs match arm,
        // swept: every letter OUTSIDE the displaced list above resolves IDENTICALLY
        // between Mac and Linux conventions — "exactly the computed collisions, no
        // more, no less".
        let displaced_letters: Vec<char> = displaced.iter().map(|(l, _)| *l).chain(['c', 'x']).collect();
        let all_bare_ctrl_letters = ['f', 'b', 'n', 'p', 'a', 'e', 'd', 'k', 'y', 's', 'r', 'w', 'v', 'g', 'x', 'c'];
        for letter in all_bare_ctrl_letters {
            if displaced_letters.contains(&letter) {
                continue;
            }
            let mut mac = KeymapState::new_with_convention(Convention::Mac);
            let mut linux = KeymapState::new_with_convention(Convention::Linux);
            let key = ch(&letter.to_string());
            assert_eq!(
                mac.resolve(&key, &ctrl()),
                linux.resolve(&key, &ctrl()),
                "Ctrl-{letter} must resolve identically on both conventions (not in the displaced list)"
            );
        }

        // ONE SOURCE OF TRUTH: `LINUX_DISPLACED_LETTERS` (the label-truth owner's
        // data) must be EXACTLY this same set — sorted-and-deduped comparison so a
        // future letter added to one and not the other fails loudly here.
        let mut from_const: Vec<char> = LINUX_DISPLACED_LETTERS.to_vec();
        from_const.sort_unstable();
        let mut from_test = displaced_letters.clone();
        from_test.sort_unstable();
        from_test.dedup();
        assert_eq!(from_const, from_test, "LINUX_DISPLACED_LETTERS drifted from this test's own displaced list");
    }

    /// THE WEB CHORD SANITY ROUND, Tier 3 — [`linux_displaces_emacs_default`]'s own
    /// unit contract: a bare `Ctrl-<displaced letter>` (single chord or the FIRST
    /// key of a prefix sequence) is displaced; a modified chord, a non-displaced
    /// letter, or an empty slot is not.
    #[test]
    fn linux_displaces_emacs_default_flags_exactly_the_collision_table() {
        // Single-chord defaults that collide.
        for emacs in ["C-s", "C-r", "C-w", "C-a", "C-e"] {
            assert!(linux_displaces_emacs_default(emacs, &[]), "{emacs:?} should be displaced");
        }
        // A prefix sequence whose FIRST key collides (Follow link's "C-c C-o":
        // Ctrl-C now resolves straight to Copy, so the sequence never arms).
        assert!(linux_displaces_emacs_default("C-c C-o", &[]));
        // NOT displaced: a modified chord outside the bare-Ctrl-letter shape...
        assert!(!linux_displaces_emacs_default("C-/", &[])); // Undo's emacs slot
        assert!(!linux_displaces_emacs_default("C-y", &[])); // Paste's emacs slot — 'y' is not claimed
        // ...a bare Ctrl letter NOT in the displaced set — Ctrl-D (never claimed)
        // and Ctrl-K (Links v2 spent Cmd-K, but `linux_builtin_keep()` keeps kill-
        // line unconditionally, so it's not on `LINUX_DISPLACED_LETTERS` at all;
        // see the collision-table doc above the keep helpers)...
        assert!(!linux_displaces_emacs_default("C-d", &[]));
        assert!(!linux_displaces_emacs_default("C-k", &[]));
        // ...and an empty/unparsable slot.
        assert!(!linux_displaces_emacs_default("", &[]));
        assert!(!linux_displaces_emacs_default("   ", &[]));
    }

    /// THE EMACS-HANDS-ON-LINUX ROUND — a `keep`-listed chord is no longer
    /// displaced, on ANY equivalent spelling (canonical compare).
    #[test]
    fn linux_displaces_emacs_default_respects_the_keep_list() {
        let keep = vec!["C-f".to_string(), "Ctrl-b".to_string()];
        assert!(!linux_displaces_emacs_default("C-f", &keep), "C-f is kept");
        assert!(!linux_displaces_emacs_default("C-b", &keep), "C-b is kept via an equivalent spelling");
        // An UNLISTED displaced letter is still displaced — the keep-list is a
        // per-chord door, not a policy flip.
        assert!(linux_displaces_emacs_default("C-s", &keep), "C-s is not in the keep list");
        assert!(linux_displaces_emacs_default("C-n", &keep), "C-n is not in the keep list");
    }

    /// THE EMACS-HANDS-ON-LINUX ROUND — the actual DISPATCH half: a `keep`-listed
    /// chord resolves to its emacs/static meaning under `Convention::Linux`,
    /// while an unlisted chord (and every listed chord under Mac) is unaffected.
    #[test]
    fn linux_keep_emacs_restores_dispatch_for_kept_chords_only() {
        let keep = vec![
            "C-f".to_string(),
            "C-b".to_string(),
            "C-n".to_string(),
            "C-p".to_string(),
            "C-a".to_string(),
            "C-e".to_string(),
        ];
        let mut km = KeymapState::new_with_convention(Convention::Linux);
        km.apply_linux_keep(&keep);
        assert_eq!(km.resolve(&ch("f"), &ctrl()), Action::ForwardChar, "C-f kept");
        assert_eq!(km.resolve(&ch("b"), &ctrl()), Action::BackwardChar, "C-b kept");
        assert_eq!(km.resolve(&ch("n"), &ctrl()), Action::NextLine, "C-n kept");
        assert_eq!(km.resolve(&ch("p"), &ctrl()), Action::PreviousLine, "C-p kept");
        assert_eq!(km.resolve(&ch("a"), &ctrl()), Action::LineStart, "C-a kept");
        assert_eq!(km.resolve(&ch("e"), &ctrl()), Action::LineEnd, "C-e kept");
        // An UNLISTED chord still displaces normally — C-c stays Copy (native
        // wins), not the bare C-c prefix.
        assert_eq!(km.resolve(&ch("c"), &ctrl()), Action::CopyRegion, "C-c not kept: native still wins");

        // Without ANY keep-list, the same chords resolve to their NATIVE meaning
        // (the pre-round behavior — the round is a per-chord opt-in, not a flip).
        let mut plain = KeymapState::new_with_convention(Convention::Linux);
        assert_eq!(plain.resolve(&ch("f"), &ctrl()), Action::SearchForward);
        assert_eq!(plain.resolve(&ch("n"), &ctrl()), Action::NewNote);

        // MAC IGNORES THE LIST ENTIRELY (the law): a keep-listed chord under
        // `Convention::Mac` resolves exactly as it would with an empty list —
        // Ctrl-F on Mac was never a native chord to begin with (Mac's native
        // layer speaks Cmd, not Ctrl), so it's ForwardChar regardless.
        let mut mac_kept = KeymapState::new_with_convention(Convention::Mac);
        mac_kept.apply_linux_keep(&keep);
        let mut mac_plain = KeymapState::new_with_convention(Convention::Mac);
        for letter in ['f', 'b', 'n', 'p', 'a', 'e'] {
            let key = ch(&letter.to_string());
            assert_eq!(
                mac_kept.resolve(&key, &ctrl()),
                mac_plain.resolve(&key, &ctrl()),
                "Ctrl-{letter} on Mac must be unaffected by a non-empty linux_keep_emacs list"
            );
        }
    }

    /// A bad/unsupported `linux_keep_emacs` entry (a two-chord `C-x`/`C-c`
    /// prefix spec, or outright garbage) is reported + skipped — never a crash,
    /// never poisoning the rest of the list.
    #[test]
    fn linux_keep_emacs_bad_entry_is_skipped_not_a_crash() {
        let mut km = KeymapState::new_with_convention(Convention::Linux);
        km.apply_linux_keep(&["C-x g".to_string(), "C-frobnicate".to_string(), "C-f".to_string()]);
        // The one VALID entry still took effect...
        assert_eq!(km.resolve(&ch("f"), &ctrl()), Action::ForwardChar);
        // ...and a fresh C-x still arms the ordinary bare prefix (the bad
        // "C-x g" entry never reached the keep-set).
        assert_eq!(km.resolve(&ch("x"), &ctrl()), Action::KillRegion, "C-x is not itself kept");
    }

    /// A live config RELOAD re-applies the keep-list exactly like
    /// `apply_overrides` re-applies `[keys]` — a later `apply_linux_keep` call
    /// clears the prior set first, never accumulating stale entries.
    #[test]
    fn apply_linux_keep_reload_replaces_not_accumulates() {
        let mut km = KeymapState::new_with_convention(Convention::Linux);
        km.apply_linux_keep(&["C-f".to_string()]);
        assert_eq!(km.resolve(&ch("f"), &ctrl()), Action::ForwardChar);
        assert_eq!(km.resolve(&ch("n"), &ctrl()), Action::NewNote, "C-n not yet kept");
        // Reload with a DIFFERENT list: C-f goes back to native, C-n is now kept.
        km.apply_linux_keep(&["C-n".to_string()]);
        assert_eq!(km.resolve(&ch("f"), &ctrl()), Action::SearchForward, "C-f reverted on reload");
        assert_eq!(km.resolve(&ch("n"), &ctrl()), Action::NextLine, "C-n now kept");
    }

    // ── THE KEYMAP FLAVOR ROUND ──────────────────────────────────────────────

    /// THE NO-DRIFT LAW: [`linux_emacs_preset_keep`] is derived FROM
    /// [`LINUX_DISPLACED_LETTERS`] itself — exactly one `"C-<letter>"` chord per
    /// displaced letter, no more, no less. A future letter added to (or removed
    /// from) the displaced table flows into the preset automatically; this test
    /// pins that the two can never silently diverge.
    #[test]
    fn linux_emacs_preset_keep_equals_the_displaced_letters_no_drift() {
        let preset = linux_emacs_preset_keep();
        assert_eq!(preset.len(), LINUX_DISPLACED_LETTERS.len());
        for letter in LINUX_DISPLACED_LETTERS {
            let want = format!("C-{letter}");
            assert!(preset.contains(&want), "preset missing {want:?} for displaced letter {letter:?}");
        }
        // And nothing EXTRA: every preset entry canonically matches some displaced
        // letter's chord.
        for chord in &preset {
            assert!(
                LINUX_DISPLACED_LETTERS.iter().any(|l| *chord == format!("C-{l}")),
                "preset chord {chord:?} has no matching displaced letter"
            );
        }
    }

    /// THE KEYMAP FLAVOR ROUND — the actual DISPATCH half: applying the WHOLE
    /// emacs preset (as `Config::effective_linux_keep` would under `keymap =
    /// "emacs"`) reverts EVERY displaced bare-control chord back to its emacs
    /// meaning under `Convention::Linux` — not just the three named in the task
    /// spec (C-f forward-char, C-s isearch-forward, C-g cancel), but ALL of
    /// them, since nothing was explicitly listed one by one (the preset IS the
    /// whole displaced set). Each reverted chord's resolution matches EXACTLY
    /// what the SAME bare Ctrl-letter resolves to under `Convention::Mac` (where
    /// Ctrl never carries a native meaning at all — i.e. the untouched emacs
    /// default), so this test doubles as "the flavor preset makes Linux behave
    /// like Mac's Ctrl reading, letter for letter".
    #[test]
    fn keymap_flavor_emacs_preset_reverts_every_displaced_chord_to_emacs_meaning() {
        let preset = linux_emacs_preset_keep();
        let mut km = KeymapState::new_with_convention(Convention::Linux);
        km.apply_linux_keep(&preset);
        for letter in LINUX_DISPLACED_LETTERS {
            let key = ch(&letter.to_string());
            let mut mac_reference = KeymapState::new_with_convention(Convention::Mac);
            let want = mac_reference.resolve(&key, &ctrl());
            let mut linux_kept = KeymapState::new_with_convention(Convention::Linux);
            linux_kept.apply_linux_keep(&preset);
            assert_eq!(
                linux_kept.resolve(&key, &ctrl()),
                want,
                "Ctrl-{letter} under the emacs flavor preset should match Mac's untouched emacs meaning"
            );
        }
        // Spelled out explicitly per the task's own worked example.
        let mut nav = KeymapState::new_with_convention(Convention::Linux);
        nav.apply_linux_keep(&preset);
        assert_eq!(nav.resolve(&ch("f"), &ctrl()), Action::ForwardChar, "C-f nav");
        let mut isearch = KeymapState::new_with_convention(Convention::Linux);
        isearch.apply_linux_keep(&preset);
        assert_eq!(isearch.resolve(&ch("s"), &ctrl()), Action::SearchForward, "C-s isearch");
        let mut cancel = KeymapState::new_with_convention(Convention::Linux);
        cancel.apply_linux_keep(&preset);
        assert_eq!(cancel.resolve(&ch("g"), &ctrl()), Action::Cancel, "C-g cancel");
        let _ = km; // exercised above; keep the earlier binding for readability
    }

    /// A chord OUTSIDE the displaced set is UNCHANGED by the emacs flavor
    /// preset — it was never displaced to begin with, so keeping it is a
    /// no-op, not a second policy layer. 'd'/'y' are never claimed by any
    /// native command at all; 'k' is a DIFFERENT flavor of "outside the
    /// preset" — it IS native-claimed (Links v2's Cmd-K), but
    /// `linux_builtin_keep()`'s unconditional floor already keeps it before the
    /// preset ever gets applied, so applying (or not applying) the preset
    /// makes no observable difference to it either.
    #[test]
    fn keymap_flavor_emacs_preset_is_a_no_op_for_non_displaced_chords() {
        let preset = linux_emacs_preset_keep();
        let mut plain = KeymapState::new_with_convention(Convention::Linux);
        let mut kept = KeymapState::new_with_convention(Convention::Linux);
        kept.apply_linux_keep(&preset);
        for letter in ['k', 'd', 'y'] {
            let key = ch(&letter.to_string());
            assert_eq!(
                plain.resolve(&key, &ctrl()),
                kept.resolve(&key, &ctrl()),
                "Ctrl-{letter} (never displaced) must be unaffected by the emacs preset"
            );
        }
    }

    /// Config `[keys]` STILL wins over the flavor preset for a named chord —
    /// the CARVE-OUT layer this round's Omarchy recipe leans on (Copy/Cut/Paste
    /// pinned native even under `keymap = "emacs"`). Mirrors this module's
    /// `apply_overrides` doc: a `[keys]` override is consulted BEFORE any static
    /// arm, keep-list included.
    #[test]
    fn config_keys_override_wins_over_the_emacs_preset() {
        let preset = linux_emacs_preset_keep();
        let mut km = KeymapState::with_overrides_and_convention(
            &[("copy".to_string(), vec!["C-c".to_string()])],
            Convention::Linux,
        );
        km.apply_linux_keep(&preset);
        // Copy is EXPLICITLY rebound to C-c via `[keys]` — that wins outright,
        // even though the emacs preset ALSO keeps C-c (its bare-prefix meaning).
        assert_eq!(km.resolve(&ch("c"), &ctrl()), Action::CopyRegion, "[keys] override wins over the preset");
    }

    // ── THE INSERT-LINK-YIELDS-TO-KILL-LINE ROUND ───────────────────────────

    /// HARD LAW (a): with an EMPTY user config, Ctrl-K resolves to Kill line on
    /// Linux under BOTH keymap flavors — the user's decided outcome ("kill-line
    /// is too load-bearing for emacs hands to lose by default"). Driven through
    /// the REAL composition owner, `Config::effective_linux_keep`, exactly like
    /// `App::new`/headless replay construct their keymap — not a bare
    /// `KeymapState` with a hand-rolled list, so this is honestly "a real Linux
    /// keymap with empty config", not just the primitive's own mechanics.
    #[test]
    fn out_of_the_box_linux_ctrl_k_is_kill_line_under_both_keymap_flavors() {
        for flavor in ["native", "emacs"] {
            let mut cfg = crate::config::Config::empty();
            cfg.keymap = Some(flavor.to_string());
            let keep = cfg.effective_linux_keep();
            let mut km = KeymapState::new_with_convention(Convention::Linux);
            km.apply_linux_keep(&keep);
            assert_eq!(
                km.resolve(&ch("k"), &ctrl()),
                Action::KillLine,
                "Ctrl-K must stay kill-line out of the box under keymap={flavor:?}"
            );
        }
    }

    /// HARD LAW (c): an explicit `[keys] insert_link = "C-k"` override on Linux
    /// STILL dispatches Insert link — the override-before-static/keep seam
    /// (`self.override_single`, consulted at the very top of `resolve`, before any
    /// default/policy arm) wins over `linux_builtin_keep()`'s floor exactly
    /// like it already wins over any other static or kept chord. The control
    /// (no override, same keep list) confirms kill-line still wins otherwise —
    /// so the override is genuinely doing the work, not some other accident.
    #[test]
    fn keys_override_reclaims_ctrl_k_for_insert_link_on_linux_over_the_builtin_keep() {
        let keep = crate::config::Config::empty().effective_linux_keep();
        let mut km = KeymapState::with_overrides_and_convention(
            &[("insert_link".to_string(), vec!["C-k".to_string()])],
            Convention::Linux,
        );
        km.apply_linux_keep(&keep);
        assert_eq!(km.resolve(&ch("k"), &ctrl()), Action::InsertLink, "[keys] override wins over the built-in keep");

        let mut plain = KeymapState::new_with_convention(Convention::Linux);
        plain.apply_linux_keep(&keep);
        assert_eq!(plain.resolve(&ch("k"), &ctrl()), Action::KillLine, "control: without the override, kill-line wins");
    }

    /// Every OTHER native chord (no letter collision) still fires under Linux, on
    /// its Ctrl-translated form.
    #[test]
    fn linux_convention_resolves_untranslated_native_chords() {
        let mut km = KeymapState::new_with_convention(Convention::Linux);
        assert_eq!(km.resolve(&ch("z"), &ctrl()), Action::Undo);
        assert_eq!(km.resolve(&ch("Z"), &ctrl_shift()), Action::Redo);
        assert_eq!(km.resolve(&ch("t"), &ctrl()), Action::OpenThemeMenu);
        assert_eq!(km.resolve(&ch("o"), &ctrl()), Action::OpenGoto);
        assert_eq!(km.resolve(&ch("q"), &ctrl()), Action::Quit);
        assert_eq!(km.resolve(&ch(","), &ctrl()), Action::OpenSettingsMenu);
        assert_eq!(km.resolve(&ch(";"), &ctrl()), Action::OpenSpellSuggest);
        assert_eq!(km.resolve(&ch("i"), &ctrl()), Action::Italic);
        assert_eq!(km.resolve(&ch("i"), &ctrl_alt()), Action::ShowStatsHud);
        assert_eq!(km.resolve(&ch("l"), &ctrl_shift()), Action::ToggleTaskList);
        assert_eq!(km.resolve(&ch("h"), &ctrl_shift()), Action::OpenHistory);
        assert_eq!(km.resolve(&ch("o"), &ctrl_shift()), Action::ToggleOutline);
        assert_eq!(km.resolve(&ch("p"), &ctrl_shift()), Action::OpenProject);
        assert_eq!(km.resolve(&ch("="), &ctrl()), Action::ZoomIn);
        assert_eq!(km.resolve(&ch("0"), &ctrl()), Action::ZoomReset);
        // A SUPER-only (Windows-key) press is NOT the Linux native modifier — it
        // falls through to the unhandled-super swallow guard, staying inert
        // (never self-inserting), exactly as on Mac.
        assert_eq!(km.resolve(&ch("s"), &sup()), Action::Ignore);
        // Holding BOTH Ctrl and Super claims neither convention's native gate
        // (native_down requires its own modifier ALONE); it falls through to the
        // plain bare-Ctrl emacs arm, which doesn't itself check Super.
        assert_eq!(km.resolve(&ch("s"), &sup_ctrl()), Action::SearchForward);
    }

    /// Document start/end's Linux-native OVERRIDE (`commands::LINUX_NATIVE_OVERRIDE`):
    /// Ctrl-Home/Ctrl-End, not the naive Ctrl-Up/Down translation of Cmd-Up/Down —
    /// and Line start/end's own override (plain Home/End) already fires on every
    /// convention with no keymap change needed.
    #[test]
    fn linux_convention_buffer_start_end_use_ctrl_home_end_not_ctrl_up_down() {
        let mut km = KeymapState::new_with_convention(Convention::Linux);
        assert_eq!(km.resolve(&Key::Named(NamedKey::Home), &ctrl()), Action::BufferStart);
        assert_eq!(km.resolve(&Key::Named(NamedKey::End), &ctrl()), Action::BufferEnd);
        // Plain Home/End still mean line start/end on Linux (unconditional arm).
        assert_eq!(km.resolve(&Key::Named(NamedKey::Home), &none()), Action::LineStart);
        assert_eq!(km.resolve(&Key::Named(NamedKey::End), &none()), Action::LineEnd);
        // On Mac, Ctrl-Home/End is NOT buffer start/end (that's Cmd-Up/Down there;
        // the convention gate never fires for Mac).
        let mut mac = KeymapState::new_with_convention(Convention::Mac);
        assert_eq!(mac.resolve(&Key::Named(NamedKey::Home), &ctrl()), Action::LineStart);
        assert_eq!(mac.resolve(&Key::Named(NamedKey::End), &ctrl()), Action::LineEnd);
    }

    /// `[keys]` overrides are CONVENTION-AGNOSTIC — a configured chord is taken
    /// literally on every convention, never translated.
    #[test]
    fn keys_overrides_are_convention_agnostic() {
        let cfg = vec![("toggle_debug".to_string(), vec!["Cmd-J".to_string()])];
        let mut linux = KeymapState::with_overrides_and_convention(&cfg, Convention::Linux);
        // The LITERAL configured chord (Super+J) still fires on Linux, unchanged —
        // it is NOT translated to Ctrl-J.
        assert_eq!(linux.resolve(&ch("j"), &sup()), Action::ToggleDebug);
        // And the naive Ctrl-translation of that SAME spec is NOT what fires — a
        // bare unbound Ctrl+letter is a calm `Ignore` (the ordinary emacs-branch
        // default), never a self-insert or the overridden action.
        assert_eq!(linux.resolve(&ch("j"), &ctrl()), Action::Ignore);
    }

    fn resolve_spec(km: &mut KeymapState, spec: &str) -> Vec<Action> {
        spec.split_whitespace()
            .map(|token| {
                let (key, mods) = crate::keyspec::parse_chord(token).unwrap_or_else(|e| {
                    panic!("catalog default {spec:?} contains invalid token {token:?}: {e}")
                });
                km.resolve(&key, &mods)
            })
            .collect()
    }

    /// The central data-backed dispatch law: every non-empty catalog slot is
    /// exercised through the same constructor + `resolve` stream as live input.
    /// Mac has no collision exceptions. Linux's native flavor names its two
    /// intentional policy exceptions (native-wins displacement and C-k's keep
    /// floor); the emacs flavor restores every displaced emacs slot.
    ///
    /// SCOPE: since catalog slots and dispatch share one seed, the chord-VALUE
    /// axis here is a round-trip — this law does NOT pin which chord a command
    /// carries. It pins that every seeded slot DISPATCHES (reaches `resolve` and
    /// fires) and that the hand-written Linux POLICY layer (displacement / keep /
    /// flavor) routes each slot correctly. The literal VALUE oracle is
    /// `mac_convention_is_byte_identical_to_the_pre_round_table` +
    /// `catalog_chord_snapshot_is_frozen`.
    #[test]
    fn every_catalog_default_slot_dispatches_through_real_keymap_under_both_conventions_and_flavors() {
        for command in crate::commands::COMMANDS.iter() {
            for spec in [command.native, command.emacs] {
                if spec.is_empty() {
                    continue;
                }
                let mut mac = KeymapState::new_with_convention(Convention::Mac);
                let trace = resolve_spec(&mut mac, spec);
                assert_eq!(trace.last(), Some(&command.action), "Mac: {} {spec:?}", command.name);
                if spec.split_whitespace().count() == 2 {
                    assert_eq!(trace.first(), Some(&Action::BeginPrefix), "Mac prefix trace: {}", command.name);
                }
            }
        }

        let native_keep: Vec<String> = linux_builtin_keep().iter().map(|s| (*s).to_string()).collect();
        let mut emacs_keep = linux_emacs_preset_keep();
        emacs_keep.extend(linux_builtin_keep().iter().map(|s| (*s).to_string()));
        for command in crate::commands::COMMANDS.iter() {
            let native = crate::commands::resolved_native(command, Convention::Linux);
            if !native.is_empty() {
                let mut km = KeymapState::new_with_convention(Convention::Linux);
                km.apply_linux_keep(&native_keep);
                let actual = resolve_spec(&mut km, &native);
                if linux_keeps_chord(&native_keep, &native) {
                    assert_ne!(actual.last(), Some(&command.action), "Linux native keep must suppress {} {native:?}", command.name);
                } else {
                    assert_eq!(actual.last(), Some(&command.action), "Linux native: {} {native:?}", command.name);
                }

                let mut emacs_flavor = KeymapState::new_with_convention(Convention::Linux);
                emacs_flavor.apply_linux_keep(&emacs_keep);
                let actual = resolve_spec(&mut emacs_flavor, &native);
                if !linux_keeps_chord(&emacs_keep, &native) {
                    assert_eq!(actual.last(), Some(&command.action), "Linux emacs flavor non-collision: {} {native:?}", command.name);
                }
            }

            if !command.emacs.is_empty() {
                let mut native_flavor = KeymapState::new_with_convention(Convention::Linux);
                native_flavor.apply_linux_keep(&native_keep);
                let actual = resolve_spec(&mut native_flavor, command.emacs);
                if linux_displaces_emacs_default(command.emacs, &native_keep) {
                    assert_ne!(actual.last(), Some(&command.action), "Linux native flavor must displace {} {:?}", command.name, command.emacs);
                } else {
                    assert_eq!(actual.last(), Some(&command.action), "Linux native flavor: {} {:?}", command.name, command.emacs);
                }

                let mut emacs_flavor = KeymapState::new_with_convention(Convention::Linux);
                emacs_flavor.apply_linux_keep(&emacs_keep);
                let trace = resolve_spec(&mut emacs_flavor, command.emacs);
                assert_eq!(trace.last(), Some(&command.action), "Linux emacs flavor: {} {:?}", command.name, command.emacs);
                if command.emacs.split_whitespace().count() == 2 {
                    assert_eq!(trace.first(), Some(&Action::BeginPrefix), "Linux emacs prefix trace: {}", command.name);
                }
            }
        }
    }

    #[test]
    #[should_panic(expected = "conflicting effective default")]
    fn conflicting_embedded_defaults_fail_loudly_at_the_map_seam() {
        let mut map = HashMap::new();
        insert_default_entry(
            &mut map,
            (ch("q"), ModifiersState::SUPER),
            Action::Quit,
            "Quit",
            "Cmd-Q",
        );
        insert_default_entry(
            &mut map,
            (ch("q"), ModifiersState::SUPER),
            Action::Save,
            "Save",
            "Cmd-Q",
        );
    }

    /// A slot mutation has one owner: the same chord text labels the command and
    /// seeds dispatch. This test uses a local catalog-shaped row so the embedded
    /// asset itself remains immutable during the test.
    #[test]
    fn changing_one_valid_default_slot_changes_both_label_and_dispatch() {
        let mutated = crate::commands::Command {
            name: "Save",
            action: Action::Save,
            native: "Cmd-J",
            emacs: "",
            native_only: false,
            web_only: false,
        };
        assert_eq!(crate::commands::join_slots(mutated.native, mutated.emacs), "⌘J");

        let mut km = KeymapState::new_with_convention(Convention::Mac);
        km.default_single.clear();
        km.default_c_x.clear();
        km.default_c_c.clear();
        km.insert_default(mutated.native, mutated.action, mutated.name);
        assert_eq!(resolve_spec(&mut km, "Cmd-J").last(), Some(&Action::Save));
        assert_eq!(resolve_spec(&mut km, "Cmd-S").last(), Some(&Action::Ignore));
    }
}
