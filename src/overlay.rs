//! The SUMMONED, TRANSIENT navigation overlay (go-to file / switch project /
//! one-level browse).
//!
//! The overlay is NOT a sidebar/tree/tabs: it appears, is used, and VANISHES on
//! pick. While it is `Some`, typed chars edit the overlay QUERY (never the
//! buffer), Up/Down move the selection, Enter opens the highlighted item, and
//! Esc/C-g cancels. All of that is driven through `actions::apply_core`, so the
//! `--keys` headless replay can open it, type to filter, move, and accept — the
//! whole flow stays agent-verifiable and serializable to the capture sidecar.
//!
//! Three kinds share the one card:
//!   * `Goto`    — the active project's flat file index (fuzzy jump).
//!   * `Project` — a real, navigable FILE EXPLORER for picking the active root.
//!     It starts at the `--workspace` dir but navigates by ABSOLUTE path. It is a
//!     PROJECT PICKER first: Enter PICKS the highlighted folder as the new root
//!     (the synthetic `.` row picks the CURRENT directory). Right DESCENDS into a
//!     folder to pick a subfolder; Left / Backspace ASCENDS (even ABOVE the
//!     workspace). Git folders carry a `• ` marker.
//!   * `Browse`  — ONE directory level at a time for the active root. Enter on a
//!     FOLDER descends (the list becomes that folder's children); Left/Backspace
//!     ASCENDS; Enter on a FILE opens it and closes. Git folders are marked. It
//!     is still summoned + transient — it vanishes on open/cancel, never a tree.

use crate::fuzzy::{self, Tier};
use std::path::Path;

/// Which kind of overlay is open. `Goto` lists the active project's file index;
/// `Project` is a navigable directory explorer (pick any folder as the root);
/// `Browse` walks one directory level of the active root at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    Goto,
    Project,
    Browse,
    /// The THEME picker: lists the worlds by name, fuzzy-filterable, with LIVE
    /// PREVIEW as the selection moves (the highlighted world applies immediately).
    /// Enter commits the previewed world; Esc/C-g reverts to the world that was
    /// active when the picker opened.
    Theme,
    /// The CARET-STYLE picker (Cmd-P → "Caret style"): lists the three caret looks
    /// (Block / Morph / I-beam) each with a one-line description, with a LIVE
    /// ANIMATED PREVIEW of the highlighted look (a "Smash character-select" box where
    /// the caret loops a representative motion). Navigating PREVIEWS the look (applies
    /// it to the process-global so the document caret + the preview switch); Enter
    /// COMMITS + persists it; Esc/C-g reverts to the look active when it opened. It
    /// carries `original_caret` so a Cancel can restore the previous look.
    Caret,
    /// The MOVE-DESTINATION picker (C-x m): reuses the Browse navigator but lists
    /// only FOLDERS (you move a note INTO a folder). It is rooted at the notes
    /// root. Right/`ForwardChar` DESCENDS into the highlighted folder, Left ASCENDS,
    /// and Enter ACCEPTS the destination — either the highlighted folder, or, when
    /// the typed query matches no listed folder, a NEW folder of that name to
    /// create. The accepted value is a notes-root-relative directory path.
    MoveDest,
    /// The DICTIONARY picker (Cmd-P → "Dictionary"): lists the three bundled
    /// spell-check variants (English US / UK / Australia), each with a
    /// one-line description, mirroring the CARET-STYLE picker's layout — EXCEPT
    /// there is NO live preview as the selection moves (a dictionary re-parse is
    /// a genuine one-time cost, tens of ms, not a per-keystroke one — see
    /// `spell.rs`), so navigating just highlights. Enter COMMITS: the process-
    /// global active variant is set THEN (not during navigation), the caller
    /// reconstructs its `SpellChecker` + persists the sticky pref. Esc/C-g
    /// simply closes (nothing was ever previewed to revert).
    Dictionary,
    /// The COMMAND PALETTE (Cmd-P): a fuzzy search over the command CATALOG names
    /// (`commands::COMMANDS`), each row showing the command's current key binding
    /// dim beside it. Enter RUNS the selected command's `Action`; the catalog
    /// order == the corpus order, so the selected corpus index maps straight back
    /// to `COMMANDS[i]`.
    Command,
    /// The OUTLINE picker (Cmd-Shift-O): a fuzzy search over the document's
    /// HEADINGS (`markdown::headings`), each row the heading title indented by its
    /// depth. Enter JUMPS the cursor to that heading's line. Flat + transient like
    /// the other pickers — NOT a persistent outline panel.
    Outline,
    /// The SPELL-SUGGESTION picker (Cmd-`;`): lists the spellchecker's ordered
    /// corrections for the misspelled word the cursor is on. Enter REPLACES that
    /// word with the chosen suggestion (a single undoable edit). Flat + transient;
    /// it carries `spell_target` — the word's `(line, start_col, end_col)` span —
    /// so the accept can locate the word to swap.
    Spell,
    /// The GAME-STYLE REBIND MENU (Cmd-P → "Keybindings"): lists EVERY command +
    /// its two current bindings (like the palette's binding column), fuzzy-filterable.
    /// Enter on a command opens a CAPTURE sub-state ([`Capture`], carried in
    /// `capture`) — choose KEY (one combo, finishes instantly) or CHORD (a sequence,
    /// Enter finishes) — and the captured spec is written to the command's `[keys]`
    /// slot, saved + live-reloaded. Delete on a command RESETS it to default; a
    /// transient `notice` shows conflicts / saves. Summoned + transient, never a
    /// settings window.
    Keybindings,
    /// The SUMMONED HISTORY TIMELINE (Cmd-Shift-H → "History"): lists the current
    /// file's VERSIONS newest-first (from [`crate::history::timeline_rows`]), each
    /// row answering WHEN + WHICH in the main column (`"2 hr ago · edited
    /// \"Title\""` — a relative timestamp, clock-suffixed exactly when siblings
    /// share a label, then the git COMMIT SUBJECT or an awl snapshot's
    /// auto-description) with the faint "+N −M" changed-count vs the current
    /// buffer riding the right column. Navigate (Up/Down/hover/wheel) SELECTS a
    /// version AND LIVE-PREVIEWS it in the document itself (derived at
    /// ViewState-build time — the buffer is never touched; Esc is back-to-now
    /// exactly); Enter RESTORES it — replacing the buffer content with that
    /// version (an undoable edit) — then closes. For a git-managed file it lists
    /// git history (same UI). An empty history shows a calm "no history yet"
    /// row. The restore `id` per row rides the parallel
    /// [`OverlayState::history_ids`]; this is LOCAL HISTORY (automatic, git-free
    /// UX), not a git client — no commit/stage/branch UI.
    History,
}

/// Which phase of a Keybindings CAPTURE we are in (carried by [`Capture`]). Drives
/// what the next key does and what the card prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureStage {
    /// Just after Enter on a command: a two-row choice of KEY vs CHORD (Up/Down
    /// toggles, Enter confirms the mode and begins recording).
    ChooseMode,
    /// Recording presses. KEY mode finishes on the FIRST combo; CHORD mode collects
    /// successive combos (capped at the keymap's 2-deep limit) until Enter finishes.
    Recording,
    /// The finished binding clashes with another command; Enter COMMITS anyway,
    /// Esc aborts. `conflict` names the command already bound.
    Confirm,
}

/// The live CAPTURE sub-state of the Keybindings menu: which command is being
/// rebound, the phase, the KEY-vs-CHORD mode, and the combos captured so far. Pure
/// + serialisable so the capture flows into the sidecar and is unit-testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    /// The catalog (`commands::COMMANDS`) index of the command being rebound. The
    /// Keybindings corpus is in catalog order, so this is the selected corpus index.
    pub cmd_index: usize,
    /// The command's display name (for the prompt + conflict notices).
    pub cmd_name: String,
    pub stage: CaptureStage,
    /// In `ChooseMode`: 0 = KEY row, 1 = CHORD row. Records the chosen mode after.
    pub mode_sel: usize,
    /// `false` = KEY (single combo), `true` = CHORD (a sequence). Set when leaving
    /// `ChooseMode`.
    pub chord_mode: bool,
    /// The combos captured so far (KEY: 0–1; CHORD: up to 2), each a canonical chord
    /// spec (`"C-t"`, `"C-x"`). Joined by spaces, this is the binding being written.
    pub captured: Vec<String>,
    /// `Confirm` stage only: the command this binding already belongs to.
    pub conflict: Option<String>,
}

impl Capture {
    /// The binding SPEC being built — the captured combos joined by spaces
    /// (`"C-x C-s"`). Empty until the first combo is recorded.
    pub fn binding(&self) -> String {
        self.captured.join(" ")
    }

    /// The dim PROMPT line the card shows for this capture phase, surfaced to the
    /// sidecar so the flow is agent-verifiable.
    pub fn prompt(&self) -> String {
        match self.stage {
            CaptureStage::ChooseMode => {
                let key = if self.mode_sel == 0 { "[Key]" } else { "Key" };
                let chord = if self.mode_sel == 1 { "[Chord]" } else { "Chord" };
                format!("Rebind {} — {key} / {chord}   Enter choose   Esc cancel", self.cmd_name)
            }
            CaptureStage::Recording => {
                let so_far = self.binding();
                if self.chord_mode {
                    format!("press the sequence… {so_far}   Enter done   Esc cancel")
                } else {
                    format!("press a key… {so_far}   Esc cancel")
                }
            }
            CaptureStage::Confirm => {
                let who = self.conflict.as_deref().unwrap_or("another command");
                format!("{} already bound to {who} — Enter rebind   Esc cancel", self.binding())
            }
        }
    }
}

impl OverlayKind {
    /// The short mode string used in the capture sidecar.
    pub fn as_str(self) -> &'static str {
        match self {
            OverlayKind::Goto => "goto",
            OverlayKind::Project => "switch",
            OverlayKind::Browse => "browse",
            OverlayKind::Theme => "theme",
            OverlayKind::Caret => "caret",
            OverlayKind::Dictionary => "dictionary",
            OverlayKind::MoveDest => "move",
            OverlayKind::Command => "command",
            OverlayKind::Outline => "outline",
            OverlayKind::Spell => "spell",
            OverlayKind::Keybindings => "keybindings",
            OverlayKind::History => "history",
        }
    }

    /// True for the FILE/FOLDER pickers whose corpus entries are filesystem paths —
    /// the ones that HIDE dot-prefixed entries by default (with a `Cmd-Shift-.`
    /// reveal toggle). Goto (+ recent-files, same corpus) lists root-relative paths;
    /// Browse / MoveDest list one directory LEVEL's leaf names. The Project explorer
    /// is EXCLUDED (its synthetic "." accept-this-folder row would be swallowed by the
    /// dotfile filter); the non-file pickers (theme / command / caret / …) never
    /// match a path, so the toggle is a calm no-op there.
    pub fn hides_dotfiles(self) -> bool {
        matches!(
            self,
            OverlayKind::Goto | OverlayKind::Browse | OverlayKind::MoveDest
        )
    }

    /// One quiet line of control hints for this picker, drawn DIM at the foot of
    /// the overlay card so the select-vs-descend model is discoverable. The
    /// NAVIGABLE explorers (Project/Browse/MoveDest) teach the asymmetry —
    /// `->`/C-f DESCEND, `<-`/C-b ascend, ↵ SELECTS the highlighted item; the
    /// FLAT pickers (Goto/Theme/Command) have no descend, so they only name what
    /// ↵ does. The Return keycap rides the ↵ glyph (bundled in AwlSymbols), matching
    /// the ⌘/⌥ glyphs elsewhere. Rendered + surfaced to the sidecar so it stays
    /// agent-verifiable.
    pub fn hint(self) -> &'static str {
        match self {
            // Select context: ↵ PICKS the folder as the root; descend is ->/C-f.
            OverlayKind::Project => "->/C-f open   \u{21B5} select   <-/C-b up",
            // Select context: ↵ MOVES the note into the folder; descend is ->/C-f.
            OverlayKind::MoveDest => "->/C-f open   \u{21B5} move here   <-/C-b up",
            // Browse OPENS files; ↵ on a folder descends (so does ->/C-f).
            OverlayKind::Browse => "->/C-f open   \u{21B5} open   <-/C-b up",
            // Flat pickers: no descend, ↵ just accepts the highlighted row.
            OverlayKind::Goto => "\u{21B5} open",
            // The faceted theme picker: ←/→ switch the lens, ↑/↓ move the world (live
            // preview), ↵ keeps, Esc reverts to the opening theme. Starts with ↵ (flat
            // picker — no descend).
            OverlayKind::Theme => "\u{21B5} keep   \u{2190}/\u{2192} lens   \u{2191}/\u{2193} world   esc revert",
            // Caret style: Up/Down PREVIEWS the look (live), ↵ APPLIES + persists it.
            OverlayKind::Caret => "\u{21B5} apply",
            // Dictionary: no live preview (a re-parse is real work) — ↵ applies +
            // persists the highlighted variant.
            OverlayKind::Dictionary => "\u{21B5} apply",
            OverlayKind::Command => "\u{21B5} run",
            OverlayKind::Outline => "\u{21B5} jump",
            OverlayKind::Spell => "\u{21B5} replace",
            // The rebind menu: ↵ starts a capture, Delete resets the highlighted
            // command, Esc closes. (In a capture the prompt teaches Key/Chord/Enter/Esc.)
            OverlayKind::Keybindings => "\u{21B5} rebind   Delete reset   Esc close",
            // The history timeline: Enter RESTORES the highlighted version (an undoable
            // edit), Backspace/Esc close. Informational — the actions are keyboard, not
            // buttons (DESIGN: button-free, taught by hints).
            OverlayKind::History => "↵ restore   ⌫/esc close",
        }
    }
}

/// Live overlay state. `corpus` is the full candidate list (the RAW accept
/// values — root-relative paths for Goto, child names for Project, entry names
/// for Browse); `items` is the fuzzy-filtered + ranked view of it that the panel
/// shows. `selected` indexes into `items`. `open_tier`/`recent_tier` mark which
/// corpus entries get a ranking bias (open buffer > recently opened > corpus).
///
/// `git`/`is_dir` are parallel to `corpus`: `git[i]` marks an entry that is a
/// git repo (Project children, Browse folders) so the row gets a small marker;
/// `is_dir[i]` marks a directory so Browse knows Enter should DESCEND rather than
/// open. For Goto every entry is a file (both default false).
///
/// `browse_dir` is only meaningful for `Browse`: the root-relative directory the
/// current level lists (`None` = the root itself). It is surfaced to the sidecar
/// so a `--keys` descend is verifiable.
#[derive(Debug, Clone)]
pub struct OverlayState {
    pub kind: OverlayKind,
    pub query: String,
    /// The full unfiltered candidate corpus (stable order), RAW accept values.
    pub corpus: Vec<String>,
    /// Parallel to `corpus`: entry is a git repo (gets a marker).
    pub git: Vec<bool>,
    /// Parallel to `corpus`: entry is a directory (Browse: Enter descends).
    pub is_dir: Vec<bool>,
    /// Corpus indices that are currently OPEN (active file).
    pub open: Vec<usize>,
    /// Corpus indices that were recently opened (MRU), not currently open.
    pub recent: Vec<usize>,
    /// Filtered + ranked view: each entry is an index into `corpus`.
    pub items: Vec<usize>,
    /// Selected row, indexing into `items`.
    pub selected: usize,
    /// The scroll WINDOW's top row: the `items` index of the FIRST visible row. The
    /// list draws `[scroll, scroll + window_rows)`. KEYBOARD nav (`move_sel`) scrolls
    /// the MINIMUM needed to keep the selection visible; a HOVER only re-highlights
    /// within this band and NEVER moves it (so hovering the top/bottom edge can't make
    /// the list auto-scroll); the WHEEL advances it like ↑/↓. The render pipeline reads
    /// it straight, so the hover hit-test and the drawn rows can never disagree.
    pub scroll: usize,
    /// Browse only: the root-relative directory this level lists (`None` = root).
    pub browse_dir: Option<String>,
    /// Theme picker only: the theme index that was ACTIVE when the picker opened,
    /// so a Cancel can REVERT the live preview to it. `None` for the other kinds.
    pub original_theme: Option<usize>,
    /// Caret-style picker only: the caret LOOK that was active when the picker
    /// opened, so a Cancel can REVERT the live preview to it. `None` otherwise.
    pub original_caret: Option<crate::caret::CaretMode>,
    /// Command palette only: binding LABELS parallel to `corpus` (the current key
    /// chord for each command, shown dim beside its name). Empty for every other
    /// kind. Filtered into row order via [`item_bindings`].
    pub bindings: Vec<String>,
    /// Go-to (notes) only: a relative "last edited" LABEL parallel to `corpus`
    /// (e.g. "5m ago"), shown dim beside each file. Empty for every other kind AND
    /// in the headless capture path (where mtime is never read, for determinism).
    /// Filtered into row order via [`item_times`].
    pub times: Vec<String>,
    /// Outline picker only: the document LINE (0-based) each corpus heading sits
    /// on, parallel to `corpus`. Enter on a row JUMPS the cursor to `lines[i]`.
    /// Empty for every other kind. (The accept value is this line number, not the
    /// heading text, because two headings can share a title.)
    pub lines: Vec<usize>,
    /// Spell picker only: the misspelled word's `(line, start_col, end_col)` CHAR
    /// span, so the accept can map it to a buffer char range and replace it with the
    /// chosen suggestion. `None` for every other kind.
    pub spell_target: Option<(usize, usize, usize)>,
    /// History timeline only: the RESTORE key for each version, parallel to `corpus`
    /// (the row shows a relative timestamp; the id is the opaque handle
    /// [`crate::history::load`] resolves back to content). Enter on a row emits
    /// `history_ids[i]`; empty for every other kind. An empty-string id marks the
    /// synthetic "no history yet" row (Enter is a no-op there).
    pub history_ids: Vec<String>,
    /// Keybindings menu only: the active CAPTURE sub-state, or `None` while browsing
    /// the command list. Drives the capture flow + the sidecar `capture` block.
    pub capture: Option<Capture>,
    /// Keybindings menu only: a transient one-line NOTICE (a conflict warning, a
    /// "saved …" / "reset …" confirmation), drawn dim + surfaced to the sidecar.
    /// Empty for every other kind and between actions.
    pub notice: String,
    /// THEME picker only: the active FACETING lens (Time / Register / Voice /
    /// Temperature / All), cycled by LEFT/RIGHT. Drives the grouping of `items` into
    /// sections ([`Self::item_sections`]) and the lens STRIP. Meaningless (left at the
    /// default [`crate::theme::Lens::All`]) for every other kind.
    pub theme_lens: crate::theme::Lens,
    /// THEME picker only: the SECTION label for each entry in `items`, parallel to it
    /// (the faint uppercase group header a row sits under). Empty strings under
    /// [`crate::theme::Lens::All`] and for every non-theme kind (no grouping). Rebuilt
    /// by [`Self::refilter`] alongside `items`.
    pub item_sections: Vec<String>,
    /// File pickers only ([`OverlayKind::hides_dotfiles`]): whether dot-prefixed
    /// entries are REVEALED. Default `false` — the go-to / browse corpus HIDES any
    /// entry whose basename or an ancestor component starts with `.` (except `.env*`,
    /// [`crate::index::is_hidden_entry`]). `Cmd-Shift-.` (the Finder convention) flips
    /// it via [`Self::toggle_hidden`], which re-runs the display filter in
    /// [`Self::refilter`]. TRANSIENT: every fresh summon defaults hidden again (it's
    /// a field of the live picker, not a sticky global). Ignored by non-file pickers.
    pub show_hidden: bool,
}

impl OverlayState {
    /// Open a fresh overlay over `corpus`, with `open`/`recent` corpus-index
    /// hints for the ranking tiers. The query starts empty (everything matches).
    /// Git/dir markers default to all-false (the Goto case); use [`new_marked`]
    /// for Project/Browse entries that carry markers.
    pub fn new(kind: OverlayKind, corpus: Vec<String>, open: Vec<usize>, recent: Vec<usize>) -> Self {
        let n = corpus.len();
        Self::new_marked(kind, corpus, vec![false; n], vec![false; n], open, recent, None)
    }

    /// Like [`new`], but with explicit `git`/`is_dir` markers (parallel to
    /// `corpus`) and an optional `browse_dir`. Used for the Project picker (git
    /// children) and the Browse navigator (folders vs files, current level).
    #[allow(clippy::too_many_arguments)]
    pub fn new_marked(
        kind: OverlayKind,
        corpus: Vec<String>,
        git: Vec<bool>,
        is_dir: Vec<bool>,
        open: Vec<usize>,
        recent: Vec<usize>,
        browse_dir: Option<String>,
    ) -> Self {
        let mut s = Self {
            kind,
            query: String::new(),
            corpus,
            git,
            is_dir,
            open,
            recent,
            items: Vec::new(),
            selected: 0,
            scroll: 0,
            browse_dir,
            original_theme: None,
            original_caret: None,
            bindings: Vec::new(),
            times: Vec::new(),
            lines: Vec::new(),
            spell_target: None,
            history_ids: Vec::new(),
            capture: None,
            notice: String::new(),
            // Default to the flat All lens; the theme picker overrides it to Time in
            // `new_theme`. Non-theme kinds ignore it.
            theme_lens: crate::theme::Lens::All,
            item_sections: Vec::new(),
            // Fresh summon: dotfiles HIDDEN by default (the toggle is transient).
            show_hidden: false,
        };
        s.refilter();
        s
    }

    /// Attach the relative "last edited" labels (parallel to `corpus`) for the
    /// go-to picker. Set by the LIVE app only; the headless path leaves it empty so
    /// the capture stays byte-stable.
    pub fn set_times(&mut self, times: Vec<String>) {
        self.times = times;
    }

    /// Build the THEME picker: the corpus is the world NAMES (in [`crate::theme::THEMES`]
    /// order), fuzzy-filterable like the others. `original_theme` is the index of
    /// the world active when the picker opened, remembered so a Cancel can REVERT
    /// the live preview. The initial selection is set to the active world so the
    /// open frame shows it highlighted (and preview is a no-op until you move).
    pub fn new_theme(names: Vec<String>, active_index: usize) -> Self {
        let n = names.len();
        let mut s = Self::new_marked(
            OverlayKind::Theme,
            names,
            vec![false; n],
            vec![false; n],
            Vec::new(),
            Vec::new(),
            None,
        );
        s.original_theme = Some(active_index);
        // Open on All (the far-LEFT landing): the ACTIVE world is always present in the
        // flat list, so the picker opens highlighting (and previewing) the current world
        // with no surprise — under the OPT-OUT faceting a world may be hidden on any given
        // lens (the default world, Tawny, is hidden on Time), so a faceted default could
        // neither highlight it nor preview it. RIGHT steps into the faceted lenses.
        s.theme_lens = crate::theme::Lens::All;
        s.refilter();
        // Select the active world in whatever section it now sits in, so the picker
        // opens highlighting (and previewing) the current world.
        if let Some(pos) = s.items.iter().position(|&i| i == active_index) {
            s.selected = pos;
            s.scroll_to_selected();
        }
        s
    }

    /// THEME picker: the lens STRIP for rendering + the sidecar — each lens's label
    /// with a flag marking the ACTIVE one (emphasized by VALUE, never amber). In
    /// [`crate::theme::Lens::STRIP`] order (All parked at the far left). Empty for
    /// every non-theme kind (so the pipeline knows to draw no strip).
    pub fn lens_strip(&self) -> Vec<(String, bool)> {
        if self.kind != OverlayKind::Theme {
            return Vec::new();
        }
        crate::theme::Lens::STRIP
            .iter()
            .map(|l| (l.label().to_string(), *l == self.theme_lens))
            .collect()
    }

    /// THEME picker: switch the faceting lens by `delta` steps along
    /// [`crate::theme::Lens::STRIP`] (clamped at both ends — LEFT at All / RIGHT at
    /// Temperature are no-ops), KEEPING the currently-highlighted world highlighted (it just
    /// moves to its section in the new lens). Regroups the list. A no-op for every
    /// other kind.
    pub fn cycle_lens(&mut self, delta: isize) {
        if self.kind != OverlayKind::Theme {
            return;
        }
        let strip = crate::theme::Lens::STRIP;
        let cur = strip
            .iter()
            .position(|l| *l == self.theme_lens)
            .unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, strip.len() as isize - 1) as usize;
        self.set_theme_lens(strip[next]);
    }

    /// THEME picker: switch DIRECTLY to `lens` (the pointing counterpart to
    /// [`Self::cycle_lens`] — a click on a strip label), KEEPING the highlighted world.
    /// A no-op when it isn't the theme picker or the lens is already active.
    pub fn set_theme_lens(&mut self, lens: crate::theme::Lens) {
        if self.kind != OverlayKind::Theme || lens == self.theme_lens {
            return;
        }
        let keep = self.selected_corpus_index();
        self.theme_lens = lens;
        self.refilter();
        if let Some(ci) = keep {
            if let Some(pos) = self.items.iter().position(|&i| i == ci) {
                self.selected = pos;
            }
        }
        self.scroll_to_selected();
    }

    /// Build the CARET-STYLE picker: the corpus is the three caret-look LABELS (in
    /// [`crate::caret::CaretMode::ALL`] order — Block / Morph / I-beam), each row's
    /// `bindings` column carrying that look's one-line description (drawn dim beside
    /// the name, reusing the palette's right column). `active` is the look in effect
    /// when the picker opened, remembered (`original_caret`) so a Cancel reverts the
    /// live preview, and pre-selected so the open frame previews the current look.
    pub fn new_caret(active: crate::caret::CaretMode) -> Self {
        let names: Vec<String> = crate::caret::CaretMode::ALL
            .iter()
            .map(|m| m.label().to_string())
            .collect();
        let descriptions: Vec<String> = crate::caret::CaretMode::ALL
            .iter()
            .map(|m| m.description().to_string())
            .collect();
        let n = names.len();
        let mut s = Self::new_marked(
            OverlayKind::Caret,
            names,
            vec![false; n],
            vec![false; n],
            Vec::new(),
            Vec::new(),
            None,
        );
        s.bindings = descriptions;
        s.original_caret = Some(active);
        // Empty query => corpus order, so the active look sits at its ALL index;
        // select it so the picker opens previewing the current look.
        if let Some(active_index) = crate::caret::CaretMode::ALL.iter().position(|&m| m == active) {
            if let Some(pos) = s.items.iter().position(|&i| i == active_index) {
                s.selected = pos;
                s.scroll_to_selected();
            }
        }
        s
    }

    /// Build the DICTIONARY picker: the corpus is the three variant LABELS (in
    /// [`crate::spell::DictVariant::ALL`] order), each row's `bindings` column
    /// carrying that variant's one-line description — the SAME shape as
    /// [`new_caret`](Self::new_caret), minus the live-preview/revert bookkeeping
    /// (no `original_*` field: nothing is applied until Enter, so there is
    /// nothing for a Cancel to revert). `active` pre-selects the picker's open
    /// frame on the currently-active variant.
    pub fn new_dictionary(active: crate::spell::DictVariant) -> Self {
        let names: Vec<String> = crate::spell::DictVariant::ALL
            .iter()
            .map(|v| v.label().to_string())
            .collect();
        let descriptions: Vec<String> = crate::spell::DictVariant::ALL
            .iter()
            .map(|v| v.description().to_string())
            .collect();
        let n = names.len();
        let mut s = Self::new_marked(
            OverlayKind::Dictionary,
            names,
            vec![false; n],
            vec![false; n],
            Vec::new(),
            Vec::new(),
            None,
        );
        s.bindings = descriptions;
        if let Some(active_index) = crate::spell::DictVariant::ALL.iter().position(|&v| v == active) {
            if let Some(pos) = s.items.iter().position(|&i| i == active_index) {
                s.selected = pos;
                s.scroll_to_selected();
            }
        }
        s
    }

    /// Build a PROJECT explorer level for the ABSOLUTE directory `dir_abs`,
    /// listing its child `folders` (each `(name, is_git)`). A synthetic `"."`
    /// row is pinned at the TOP (a non-directory entry) meaning "accept THIS
    /// folder as the project root"; the real folders follow. `browse_dir`
    /// carries `dir_abs` so ascend/descend navigate by real absolute path (and
    /// can climb ABOVE the workspace). The initial selection lands on the first
    /// real folder, so Enter PICKS it (or Right descends into it) immediately,
    /// while Up reaches the `"."` accept-this-folder row.
    pub fn new_project(dir_abs: String, folders: Vec<(String, bool)>) -> Self {
        let mut corpus = vec![".".to_string()];
        let mut git = vec![false];
        let mut is_dir = vec![false];
        for (name, is_git) in folders {
            corpus.push(name);
            git.push(is_git);
            is_dir.push(true);
        }
        let mut s = Self::new_marked(
            OverlayKind::Project,
            corpus,
            git,
            is_dir,
            Vec::new(),
            Vec::new(),
            Some(dir_abs),
        );
        // Default to the first real folder so Enter PICKS it (or Right descends)
        // right away; the synthetic "." (accept-this-folder) sits above it, Up.
        s.selected = s.items.iter().position(|&i| s.corpus[i] != ".").unwrap_or(0);
        s.scroll_to_selected();
        s
    }

    /// Build the COMMAND PALETTE: the corpus is the command NAMES (in
    /// `commands::COMMANDS` order, so a row index maps back to the catalog) and
    /// `bindings` carries each command's current chord label, shown dim beside the
    /// name. Fuzzy-filterable like the other pickers.
    pub fn new_command(names: Vec<String>, bindings: Vec<String>) -> Self {
        let n = names.len();
        let mut s = Self::new_marked(
            OverlayKind::Command,
            names,
            vec![false; n],
            vec![false; n],
            Vec::new(),
            Vec::new(),
            None,
        );
        s.bindings = bindings;
        s
    }

    /// Build the REBIND MENU: the corpus is the command NAMES (in `commands::COMMANDS`
    /// order, so a row index maps back to the catalog) and `bindings` carries each
    /// command's EFFECTIVE chords, shown beside the name. Identical corpus/bindings to
    /// the palette, but `kind = Keybindings`, so Enter starts a CAPTURE rather than
    /// running the command.
    pub fn new_keybindings(names: Vec<String>, bindings: Vec<String>) -> Self {
        let n = names.len();
        let mut s = Self::new_marked(
            OverlayKind::Keybindings,
            names,
            vec![false; n],
            vec![false; n],
            Vec::new(),
            Vec::new(),
            None,
        );
        s.bindings = bindings;
        s
    }

    /// REBIND MENU: begin a capture for the highlighted command (catalog index). A
    /// no-op when no row matches the filter. Opens in `ChooseMode` with KEY preselected.
    pub fn start_capture(&mut self) {
        let Some(i) = self.selected_corpus_index() else {
            return;
        };
        self.notice.clear();
        self.capture = Some(Capture {
            cmd_index: i,
            cmd_name: crate::commands::name_of_index(i).to_string(),
            stage: CaptureStage::ChooseMode,
            mode_sel: 0,
            chord_mode: false,
            captured: Vec::new(),
            conflict: None,
        });
    }

    /// REBIND MENU: in `ChooseMode`, move the KEY/CHORD selection (`delta` &lt; 0 → KEY,
    /// &gt; 0 → CHORD). Other phases ignore it.
    pub fn capture_move_mode(&mut self, delta: isize) {
        if let Some(cap) = self.capture.as_mut() {
            if cap.stage == CaptureStage::ChooseMode {
                cap.mode_sel = if delta < 0 { 0 } else { 1 };
            }
        }
    }

    /// REBIND MENU: leave `ChooseMode` — lock in KEY vs CHORD and begin `Recording`.
    pub fn capture_begin_recording(&mut self) {
        if let Some(cap) = self.capture.as_mut() {
            if cap.stage == CaptureStage::ChooseMode {
                cap.chord_mode = cap.mode_sel == 1;
                cap.stage = CaptureStage::Recording;
            }
        }
    }

    /// REBIND MENU: record one captured `combo` (a canonical chord spec) while
    /// `Recording`. Returns `true` when the binding is now COMPLETE — KEY mode after
    /// the first combo (finishes instantly), or CHORD mode once the 2-deep cap is hit
    /// — so the caller can finalise it; `false` while a CHORD still awaits more (Enter).
    /// A no-op outside `Recording`.
    pub fn capture_record(&mut self, combo: String) -> bool {
        let Some(cap) = self.capture.as_mut() else {
            return false;
        };
        if cap.stage != CaptureStage::Recording {
            return false;
        }
        if cap.chord_mode {
            if cap.captured.len() < 2 {
                cap.captured.push(combo);
            }
            // CHORD: a full 2-deep sequence is complete; otherwise wait for Enter.
            cap.captured.len() >= 2
        } else {
            cap.captured = vec![combo];
            true // KEY: one combo finishes instantly.
        }
    }

    /// REBIND MENU: the (slug, binding-spec) for the in-progress capture, or `None`
    /// when nothing has been captured yet. The slug keys the `[keys]` entry; the
    /// binding is the captured combos joined by spaces.
    pub fn capture_target(&self) -> Option<(String, String)> {
        let cap = self.capture.as_ref()?;
        if cap.captured.is_empty() {
            return None;
        }
        Some((crate::commands::slug_of_index(cap.cmd_index), cap.binding()))
    }

    /// REBIND MENU: move the capture into the `Confirm` phase (a clash was found),
    /// remembering `conflict` (the command already bound) for the prompt.
    pub fn capture_into_confirm(&mut self, conflict: String) {
        if let Some(cap) = self.capture.as_mut() {
            cap.stage = CaptureStage::Confirm;
            cap.conflict = Some(conflict);
        }
    }

    /// REBIND MENU: cancel any in-progress capture, returning to the command list.
    pub fn capture_abort(&mut self) {
        self.capture = None;
    }

    /// REBIND MENU: the slug of the highlighted command (for Delete → reset-to-default),
    /// or `None` when no row matches.
    pub fn selected_command_slug(&self) -> Option<String> {
        self.selected_corpus_index().map(crate::commands::slug_of_index)
    }

    /// The line drawn DIM at the FOOT of the card. Normally the per-kind control
    /// hint; for the Keybindings menu an active capture's PROMPT (press a key…) wins,
    /// else a transient NOTICE (saved / reset / conflict), so the rebind flow reads on
    /// the card itself. Other kinds always show `kind.hint()`.
    pub fn foot_hint(&self) -> String {
        if let Some(cap) = &self.capture {
            return cap.prompt();
        }
        if !self.notice.is_empty() {
            return self.notice.clone();
        }
        self.kind.hint().to_string()
    }

    /// Build the OUTLINE picker: `headings` is the document's headings in order,
    /// each `(display, line)` — the display string (title indented by depth) is the
    /// fuzzy corpus, and `line` (parallel) is where Enter jumps the cursor. Flat +
    /// fuzzy like the other summoned pickers; it vanishes on pick.
    pub fn new_outline(headings: Vec<(String, usize)>) -> Self {
        let n = headings.len();
        let mut corpus = Vec::with_capacity(n);
        let mut lines = Vec::with_capacity(n);
        for (display, line) in headings {
            corpus.push(display);
            lines.push(line);
        }
        let mut s = Self::new_marked(
            OverlayKind::Outline,
            corpus,
            vec![false; n],
            vec![false; n],
            Vec::new(),
            Vec::new(),
            None,
        );
        s.lines = lines;
        s
    }

    /// Build the SPELL-SUGGESTION picker: `suggestions` is the spellchecker's
    /// ordered corrections for the misspelled word (the fuzzy corpus, best first),
    /// and `target` is that word's `(line, start_col, end_col)` CHAR span — kept so
    /// the accept can map it to a buffer char range and replace it. The list may be
    /// empty (the engine had no suggestion); the picker still summons (the word IS
    /// flagged), and Enter on an empty list is a no-op close.
    pub fn new_spell(suggestions: Vec<String>, target: (usize, usize, usize)) -> Self {
        let n = suggestions.len();
        let mut s = Self::new_marked(
            OverlayKind::Spell,
            suggestions,
            vec![false; n],
            vec![false; n],
            Vec::new(),
            Vec::new(),
            None,
        );
        s.spell_target = Some(target);
        s
    }

    /// Build the SUMMONED HISTORY TIMELINE: `rows` is the file's versions
    /// NEWEST-FIRST ([`crate::history::TimelineRow`]). Each row's MAIN column
    /// composes WHEN + WHICH — `"{when} · {which}"`, or the bare `when` for an
    /// empty `which` — so the body-ink cell answers both questions at a glance
    /// (and the fuzzy filter matches commit subjects / edit descriptions for
    /// free); the faint `"+N −M"` changed-count rides the EXISTING right binding
    /// column (LABEL size, faint ink — the picker desc-column pattern, zero new
    /// layout); the opaque restore id rides the parallel `history_ids` (the
    /// Enter accept value). An EMPTY `rows` yields a single calm "no history
    /// yet" row with an empty id, so the picker still summons (no crash) and
    /// Enter on it is a no-op. Flat + transient like the other pickers — it
    /// vanishes on restore / cancel.
    pub fn new_history(rows: Vec<crate::history::TimelineRow>) -> Self {
        if rows.is_empty() {
            let mut s = Self::new_marked(
                OverlayKind::History,
                vec!["no history yet".to_string()],
                vec![false],
                vec![false],
                Vec::new(),
                Vec::new(),
                None,
            );
            s.history_ids = vec![String::new()];
            return s;
        }
        let n = rows.len();
        let mut corpus = Vec::with_capacity(n);
        let mut diffs = Vec::with_capacity(n);
        let mut ids = Vec::with_capacity(n);
        for row in rows {
            corpus.push(if row.which.is_empty() {
                row.when
            } else {
                format!("{} · {}", row.when, row.which)
            });
            diffs.push(row.counts);
            ids.push(row.id);
        }
        let mut s = Self::new_marked(
            OverlayKind::History,
            corpus,
            vec![false; n],
            vec![false; n],
            Vec::new(),
            Vec::new(),
            None,
        );
        s.bindings = diffs; // the faint right column shows each version's changed-count
        s.history_ids = ids;
        s
    }

    /// Re-rank `corpus` against the current query into `items`, clamping the
    /// selection. Called after every query edit.
    pub fn refilter(&mut self) {
        let mut ranked: Vec<usize> = fuzzy::rank(&self.query, &self.corpus, |i| {
            if self.open.contains(&i) {
                Tier::Open
            } else if self.recent.contains(&i) {
                Tier::Recent
            } else {
                Tier::Corpus
            }
        })
        .into_iter()
        .map(|r| r.index)
        .collect();
        // DOTFILE DISPLAY FILTER (file pickers only, gated on `show_hidden`): drop any
        // corpus entry whose basename / ancestor component starts with `.` (except
        // `.env*`). The full corpus is untouched — this is purely what's SHOWN — so
        // flipping `show_hidden` and re-running `refilter` reveals them with no
        // filesystem re-read. A no-op for non-file pickers (theme/command/…) and when
        // dotfiles are revealed.
        if !self.show_hidden && self.kind.hides_dotfiles() {
            ranked.retain(|&i| !crate::index::is_hidden_entry(&self.corpus[i]));
        }
        // THEME picker under a real lens: GROUP the (fuzzy-matched) worlds into the
        // lens's sections, in section order, preserving the fuzzy rank WITHIN each
        // section. `item_sections` records each row's section (the faint header). The
        // flat All lens (and every other kind) keeps the plain ranked list.
        let lens = self.theme_lens;
        if self.kind == OverlayKind::Theme && lens != crate::theme::Lens::All {
            let mut items = Vec::with_capacity(ranked.len());
            let mut sections = Vec::with_capacity(ranked.len());
            for sect in lens.sections() {
                for &ci in &ranked {
                    // OPT-OUT faceting: a world with `None` on this lens yields `None`
                    // here, matching no section, so it is omitted from the lens (still
                    // reachable under All). Only `Some(section)` worlds are placed.
                    if crate::theme::tag_for(&self.corpus[ci], lens) == Some(*sect) {
                        items.push(ci);
                        sections.push((*sect).to_string());
                    }
                }
            }
            self.items = items;
            self.item_sections = sections;
        } else {
            self.item_sections = vec![String::new(); ranked.len()];
            self.items = ranked;
        }
        if self.selected >= self.items.len() {
            self.selected = self.items.len().saturating_sub(1);
        }
        self.scroll_to_selected();
    }

    /// THEME picker: the SECTION label for each filtered row, in the same order as
    /// [`Self::item_strings`] — the faint group header a row sits under (empty under
    /// All / for non-theme kinds). Surfaced to the render pipeline + sidecar so the
    /// grouping is drawable AND agent-verifiable.
    pub fn item_sections(&self) -> Vec<String> {
        self.item_sections.clone()
    }

    /// Append a char to the query and refilter. A query edit re-ranks the list, so the
    /// selection + scroll reset to the TOP (the best match).
    pub fn push(&mut self, c: char) {
        self.query.push(c);
        self.selected = 0;
        self.scroll = 0;
        self.refilter();
    }

    /// Remove the last query char and refilter.
    pub fn pop(&mut self) {
        self.query.pop();
        self.selected = 0;
        self.scroll = 0;
        self.refilter();
    }

    /// Cmd-Shift-. : REVEAL / re-hide dot-prefixed entries in THIS file picker (the
    /// Finder "show hidden files" convention). Flips `show_hidden` and re-runs the
    /// display filter (`refilter`) so the listing rebuilds with dotfiles shown/hidden
    /// — no filesystem re-read (the corpus already holds every entry). Resets the
    /// selection to the top (the row set changed under it). A calm NO-OP for a
    /// non-file picker (theme/command/…): those don't hide dotfiles, so there is
    /// nothing to reveal. Returns whether the flag actually flipped.
    pub fn toggle_hidden(&mut self) -> bool {
        if !self.kind.hides_dotfiles() {
            return false;
        }
        self.show_hidden = !self.show_hidden;
        self.selected = 0;
        self.scroll = 0;
        self.refilter();
        true
    }

    /// The per-kind visible ROW CAP. The contextual SPELL popup stays compact (8); every
    /// centered picker shows up to 12. Mirrors the `MAX_ROWS` caps in
    /// [`crate::render`]'s chrome so the scroll math here matches the drawn window.
    pub fn window_rows(&self) -> usize {
        match self.kind {
            OverlayKind::Spell => 8,
            // THEME: the faceted picker shows EVERY world (grouped under faint section
            // headers) with NO scroll — a window past the world count keeps `scroll` at
            // 0 so the interleaved headers never fight the scroll math.
            OverlayKind::Theme => 64,
            _ => 12,
        }
    }

    /// Scroll the window the MINIMUM needed so `selected` sits within
    /// `[scroll, scroll + window_rows)`, then clamp so the final page never shows a
    /// blank tail. Called after any keyboard move / refilter — NEVER on a hover.
    fn scroll_to_selected(&mut self) {
        let window = self.window_rows();
        if window == 0 {
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + window {
            self.scroll = self.selected + 1 - window;
        }
        let max_top = self.items.len().saturating_sub(window);
        if self.scroll > max_top {
            self.scroll = max_top;
        }
    }

    /// Move the selection by `delta` rows, clamped to the visible item range, then
    /// scroll the window to keep the new selection visible (the keyboard ↑/↓ + PgUp/Dn
    /// path). The WHEEL rides this too, so a wheel notch advances the list exactly like
    /// an arrow press.
    pub fn move_sel(&mut self, delta: isize) {
        if self.items.is_empty() {
            self.selected = 0;
            self.scroll = 0;
            return;
        }
        let n = self.items.len() as isize;
        let mut s = self.selected as isize + delta;
        if s < 0 {
            s = 0;
        }
        if s >= n {
            s = n - 1;
        }
        self.selected = s as usize;
        self.scroll_to_selected();
    }

    /// A HOVER re-highlights the row `target` ONLY when it is already within the current
    /// visible band `[scroll, scroll + window_rows)` (and is a real item). Returns whether
    /// the highlight moved. Crucially it NEVER touches `scroll`, so hovering the top /
    /// bottom edge — or anywhere off the visible rows — can't make the list auto-scroll:
    /// a hover highlights what's under the pointer, nothing more.
    pub fn hover_select(&mut self, target: usize) -> bool {
        let window = self.window_rows();
        let last = (self.scroll + window).min(self.items.len());
        if target >= self.scroll && target < last && target != self.selected {
            self.selected = target;
            true
        } else {
            false
        }
    }

    /// The corpus index currently highlighted (into `corpus`/`git`/`is_dir`), or
    /// `None` when no item matches.
    pub fn selected_corpus_index(&self) -> Option<usize> {
        self.items.get(self.selected).copied()
    }

    /// The document LINE the highlighted outline row jumps to (Outline only), or
    /// `None` when no item matches or this isn't an outline picker.
    pub fn selected_line(&self) -> Option<usize> {
        self.selected_corpus_index()
            .and_then(|i| self.lines.get(i).copied())
    }

    /// The RESTORE id of the highlighted history row (History only), or `None` when
    /// no item matches / this isn't a history picker / the row is the synthetic
    /// "no history yet" row (its id is empty). Enter maps this to a restore.
    pub fn selected_history_id(&self) -> Option<&str> {
        self.selected_corpus_index()
            .and_then(|i| self.history_ids.get(i))
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }

    /// The caret LOOK the highlighted row selects (Caret picker only), or `None`
    /// when no item matches or this isn't a caret picker. Maps the highlighted row's
    /// label back to its [`crate::caret::CaretMode`] via [`CaretMode::from_label`].
    pub fn selected_caret_mode(&self) -> Option<crate::caret::CaretMode> {
        if self.kind != OverlayKind::Caret {
            return None;
        }
        self.selected_value()
            .and_then(crate::caret::CaretMode::from_label)
    }

    /// The RAW corpus string currently highlighted (the accept value), or `None`
    /// when no item matches.
    pub fn selected_value(&self) -> Option<&str> {
        self.selected_corpus_index().map(|i| self.corpus[i].as_str())
    }

    /// True when the highlighted entry is a directory (Browse: Enter descends).
    pub fn selected_is_dir(&self) -> bool {
        self.selected_corpus_index()
            .map(|i| self.is_dir[i])
            .unwrap_or(false)
    }

    /// The DISPLAY string for corpus entry `i`: the raw value plus a trailing
    /// `/` for a directory and a leading `• ` git marker for a repo. Markers are
    /// part of the display (and the sidecar) so the switch / browse distinction is
    /// verifiable; the accept value is always the raw corpus string.
    fn display_of(&self, i: usize) -> String {
        let mut s = String::new();
        if self.git.get(i).copied().unwrap_or(false) {
            s.push_str("• ");
        }
        s.push_str(&self.corpus[i]);
        if self.is_dir.get(i).copied().unwrap_or(false) {
            s.push('/');
        }
        s
    }

    /// The filtered DISPLAY strings, top-to-bottom (for rendering AND the
    /// sidecar). Git repos carry a `• ` marker; directories a trailing `/`.
    pub fn item_strings(&self) -> Vec<String> {
        self.items.iter().map(|&i| self.display_of(i)).collect()
    }

    /// The filtered BINDING labels, in the same row order as [`item_strings`]
    /// (Command palette only; empty/blank for every other kind). Lets the render
    /// + sidecar show each command's chord beside its name without re-deriving it.
    pub fn item_bindings(&self) -> Vec<String> {
        self.items
            .iter()
            .map(|&i| self.bindings.get(i).cloned().unwrap_or_default())
            .collect()
    }

    /// The filtered relative-time LABELS, in the same row order as [`item_strings`]
    /// (go-to picker only; empty for every other kind and in headless capture).
    pub fn item_times(&self) -> Vec<String> {
        self.items
            .iter()
            .map(|&i| self.times.get(i).cloned().unwrap_or_default())
            .collect()
    }
}

/// The inputs the FLAT-picker overlay builder ([`build`]) needs, gathered by the
/// caller so the construction itself lives in ONE place (shared by the live App
/// and the headless `--keys` replay). The live-only GO-TO recency bits
/// (`goto_open` / `goto_recent` / `goto_times`) are filled by the App and left
/// EMPTY by the headless path, keeping the capture byte-stable. `config_keys`
/// feeds the command palette's EFFECTIVE bindings.
pub struct BuildCtx<'a> {
    /// The go-to corpus (root-relative paths), already recency-ordered when live.
    pub goto_corpus: Vec<String>,
    /// Corpus indices currently OPEN — ranking bias (live-only; empty headless).
    pub goto_open: Vec<usize>,
    /// Corpus indices recently opened — ranking bias (live-only; empty headless).
    pub goto_recent: Vec<usize>,
    /// Per-file "last edited" labels, parallel to `goto_corpus` (live-only; empty
    /// for a non-notes root AND in headless capture, for determinism).
    pub goto_times: Vec<String>,
    /// Config `[keys]` overrides → the command palette's effective binding column.
    pub config_keys: &'a [(String, Vec<String>)],
    /// The CURRENT buffer's markdown headings (depth-indented label + line) for
    /// the Outline picker. Caller-gathered (it needs the live buffer text); EMPTY
    /// for a non-markdown buffer or one with no headings, so the summon no-ops.
    pub outline_headings: Vec<(String, usize)>,
    /// The Cmd-`;` spell target — the misspelled word's corrections + its span —
    /// resolved by the caller ONLY when the spell binding fired. `None` when the
    /// cursor isn't on a flagged word (or spell-check is off), so the summon no-ops.
    pub spell_target: Option<(Vec<String>, (usize, usize, usize))>,
    /// The HISTORY TIMELINE rows for the current file — [`crate::history::TimelineRow`]
    /// (when / which / counts / id), newest-first — resolved by the caller (via
    /// [`crate::history::timeline_rows`]) ONLY when the History binding fired. EMPTY
    /// otherwise AND when the file has no history yet; an empty list summons the calm
    /// "no history yet" row (History always opens, unlike Outline's no-op-on-empty).
    pub history_entries: Vec<crate::history::TimelineRow>,
}

/// Build the SUMMONED overlay for a non-navigable picker kind (Goto / Theme /
/// Command, plus the buffer-scoped Outline / Spell) from the caller-gathered
/// [`BuildCtx`]. Returns `None` for the navigable explorers (Browse / MoveDest /
/// Project) — those need a directory LEVEL, built by [`browse_level`] — and for
/// an empty Outline / unresolved Spell target, so those summons stay quiet
/// no-ops. Shared by the live App (`app.rs`) and the headless replay (`main.rs`)
/// so both summon byte-identical overlays.
pub fn build(kind: OverlayKind, ctx: &BuildCtx) -> Option<OverlayState> {
    match kind {
        // Go-to: the active project's file index. The open/recent tiers + the
        // relative "last edited" labels are caller-supplied (live-only; empty in
        // headless capture, so `set_times([])` is a no-op there).
        OverlayKind::Goto => {
            let mut ov = OverlayState::new(
                kind,
                ctx.goto_corpus.clone(),
                ctx.goto_open.clone(),
                ctx.goto_recent.clone(),
            );
            ov.set_times(ctx.goto_times.clone());
            Some(ov)
        }
        // Theme picker: every world name + the active index (for revert). Built
        // from THEMES so it auto-extends as worlds are added.
        OverlayKind::Theme => {
            let names: Vec<String> =
                crate::theme::THEMES.iter().map(|t| t.name.to_string()).collect();
            Some(OverlayState::new_theme(names, crate::theme::active_index()))
        }
        // Caret-style picker: the three looks + the active one (for revert). Built
        // from CaretMode::ALL so it auto-extends if a look is added.
        OverlayKind::Caret => Some(OverlayState::new_caret(crate::caret::mode())),
        // Dictionary picker: the three variants + the active one (pre-selected;
        // there is nothing to revert since nothing previews on move).
        OverlayKind::Dictionary => Some(OverlayState::new_dictionary(crate::spell::active_variant())),
        // Command palette: the static command catalog, each row showing its
        // EFFECTIVE chord (config `[keys]` rebinds included), so it teaches the
        // live binding.
        OverlayKind::Command => Some(OverlayState::new_command(
            crate::commands::names(),
            crate::commands::effective_bindings(ctx.config_keys),
        )),
        // Rebind menu: the same command catalog + effective chords as the palette,
        // but opened in capture mode (Enter rebinds rather than runs).
        OverlayKind::Keybindings => Some(OverlayState::new_keybindings(
            crate::commands::names(),
            crate::commands::effective_bindings(ctx.config_keys),
        )),
        // Outline: the caller-gathered headings of the current buffer. An empty
        // list yields None, so the summon is a quiet no-op.
        OverlayKind::Outline => {
            if ctx.outline_headings.is_empty() {
                None
            } else {
                Some(OverlayState::new_outline(ctx.outline_headings.clone()))
            }
        }
        // Spell: the caller-resolved word target + its corrections. None when the
        // cursor isn't on a flagged word, so the summon no-ops.
        OverlayKind::Spell => ctx
            .spell_target
            .clone()
            .map(|(sugg, target)| OverlayState::new_spell(sugg, target)),
        // History: the caller-gathered timeline rows. ALWAYS summons (unlike Outline):
        // an empty list becomes the calm "no history yet" row, so the picker never
        // silently no-ops on a file that simply hasn't been snapshotted yet.
        OverlayKind::History => Some(OverlayState::new_history(ctx.history_entries.clone())),
        // Navigable explorers open via `browse_level` (they need a dir level).
        OverlayKind::Browse | OverlayKind::MoveDest | OverlayKind::Project => None,
    }
}

/// Build ONE directory LEVEL as a navigable overlay of the requested `kind`,
/// shared by the live App and the headless replay (parameterized by the caller's
/// roots so live + capture descend identically):
///   * `Project` navigates by ABSOLUTE path (`rel` IS the absolute dir; `None` =
///     start at `workspace`). Lists child FOLDERS only (git-marked) with a
///     synthetic `.` accept-this-folder row on top. `None` when no workspace.
///   * `MoveDest` walks the NOTES root (`notes_root`), listing FOLDERS only.
///   * `Browse` walks the active root (`active_root`), listing files + folders.
/// `rel` is the root-relative level for the latter two (`None` = the root).
pub fn browse_level(
    kind: OverlayKind,
    rel: Option<String>,
    active_root: &Path,
    notes_root: &Path,
    workspace: Option<&Path>,
) -> Option<OverlayState> {
    if kind == OverlayKind::Project {
        let dir = match rel
            .clone()
            .or_else(|| workspace.map(|w| w.to_string_lossy().to_string()))
        {
            Some(d) => d,
            None => return None, // no workspace configured: nothing to open
        };
        let folders: Vec<(String, bool)> = crate::index::list_dir_level(Path::new(&dir), None)
            .into_iter()
            .filter(|e| e.is_dir)
            .map(|e| (e.name, e.is_git))
            .collect();
        return Some(OverlayState::new_project(dir, folders));
    }
    // MoveDest (C-x m) walks the NOTES root, folders only; Browse walks the active
    // root and lists files + folders.
    let move_dest = kind == OverlayKind::MoveDest;
    let root = if move_dest { notes_root } else { active_root };
    let level = crate::index::list_dir_level(root, rel.as_deref());
    let mut corpus = Vec::new();
    let mut git = Vec::new();
    let mut is_dir = Vec::new();
    for e in &level {
        if move_dest && !e.is_dir {
            continue; // destinations are folders only
        }
        corpus.push(e.name.clone());
        git.push(e.is_git);
        is_dir.push(e.is_dir);
    }
    Some(OverlayState::new_marked(
        kind, corpus, git, is_dir, Vec::new(), Vec::new(), rel,
    ))
}

/// Middle-truncate `s` to at most `max` CHARS with a single `…`, keeping the HEAD and
/// the TAIL — so a filename keeps its extension end. `s` already within `max` is returned
/// unchanged. Used for the directory prefix AND (when the filename alone overflows) the
/// filename itself.
fn elide_middle(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    if max == 1 {
        return "…".to_string();
    }
    let rem = max - 1; // room besides the one ellipsis
    let tail = rem / 2 + rem % 2; // bias the TAIL so the extension survives
    let head = rem - tail;
    let head_s: String = chars[..head].iter().collect();
    let tail_s: String = chars[chars.len() - tail..].iter().collect();
    format!("{head_s}…{tail_s}")
}

/// Elide a file-picker ROW to at most `max` CHARS on ONE line, PRESERVING the filename
/// (the text after the last `/`) and its extension and keeping as much LEADING directory
/// as fits. A row that already fits is returned whole. Otherwise the DIRECTORY is
/// middle-truncated (a single `…`) while the whole filename rides at the end; only when
/// the filename ALONE overflows is the filename itself middle-truncated (still one `…`,
/// still keeping its extension). The last `/` in the result is the figure/ground split
/// point ([`row_split`]): everything through it is the muted directory, the rest the
/// content-ink filename.
pub fn elide_path(path: &str, max: usize) -> String {
    let total = path.chars().count();
    if total <= max {
        return path.to_string();
    }
    match path.rfind('/') {
        Some(byte_slash) => {
            let dir = &path[..=byte_slash]; // through the trailing '/'
            let file = &path[byte_slash + 1..]; // filename + extension
            let file_len = file.chars().count();
            // No room for the whole filename beside an ellipsis → drop the dir and
            // middle-truncate the filename itself (keeping its extension end).
            if file_len + 1 > max {
                return elide_middle(file, max);
            }
            // Keep the WHOLE filename; middle-elide the directory to what's left. The
            // dir's trailing '/' rides its tail, so the split point survives.
            let dir_budget = max - file_len;
            format!("{}{file}", elide_middle(dir, dir_budget))
        }
        None => elide_middle(path, max),
    }
}

/// The figure/ground split of a (possibly elided) picker row: the byte index just PAST
/// the last `/` — everything before it is the DIRECTORY prefix (muted ink), everything
/// from it on is the FILENAME (content ink). `0` when the row has no `/` (a bare
/// filename → all content ink).
pub fn row_split(row: &str) -> usize {
    row.rfind('/').map(|i| i + 1).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> Vec<String> {
        vec![
            ".env".to_string(),
            "README.md".to_string(),
            "src/lib.rs".to_string(),
            "src/main.rs".to_string(),
        ]
    }

    #[test]
    fn empty_query_shows_all() {
        let ov = OverlayState::new(OverlayKind::Goto, corpus(), vec![], vec![]);
        assert_eq!(ov.items.len(), 4);
    }

    #[test]
    fn typing_filters() {
        let mut ov = OverlayState::new(OverlayKind::Goto, corpus(), vec![], vec![]);
        ov.push('e');
        ov.push('n');
        ov.push('v');
        // ".env" should be the top match.
        assert_eq!(ov.selected_value(), Some(".env"));
    }

    #[test]
    fn goto_hides_dotfiles_until_revealed() {
        // A go-to corpus with a hidden dotfile, a hidden dir entry, an `.env` (the
        // earned exception), and ordinary files.
        let corpus = vec![
            ".gitignore".to_string(),
            ".env".to_string(),
            "src/.hidden/x.rs".to_string(),
            "README.md".to_string(),
            "src/main.rs".to_string(),
        ];
        let mut ov = OverlayState::new(OverlayKind::Goto, corpus, vec![], vec![]);
        // Default: dotfiles hidden, `.env` and ordinary files visible.
        let shown = ov.item_strings();
        assert!(!shown.iter().any(|s| s == ".gitignore"), "dotfile hidden: {shown:?}");
        assert!(!shown.iter().any(|s| s == "src/.hidden/x.rs"), "nested dot dir hidden: {shown:?}");
        assert!(shown.iter().any(|s| s == ".env"), ".env stays visible: {shown:?}");
        assert!(shown.iter().any(|s| s == "README.md"));
        assert!(shown.iter().any(|s| s == "src/main.rs"));
        assert!(!ov.show_hidden);
        // Toggle -> dotfiles now revealed alongside everything.
        assert!(ov.toggle_hidden());
        assert!(ov.show_hidden);
        let shown = ov.item_strings();
        assert!(shown.iter().any(|s| s == ".gitignore"), "dotfile revealed: {shown:?}");
        assert!(shown.iter().any(|s| s == "src/.hidden/x.rs"), "nested dot dir revealed: {shown:?}");
        assert!(shown.iter().any(|s| s == ".env"));
        // Toggle back -> hidden again.
        assert!(ov.toggle_hidden());
        assert!(!ov.show_hidden);
        assert!(!ov.item_strings().iter().any(|s| s == ".gitignore"));
    }

    #[test]
    fn browse_hides_dot_leaves_until_revealed() {
        // Browse lists one directory LEVEL: bare leaf names.
        let corpus = vec![
            ".config".to_string(),
            "notes.md".to_string(),
            ".env".to_string(),
        ];
        let git = vec![false; 3];
        let is_dir = vec![true, false, false];
        let mut ov = OverlayState::new_marked(
            OverlayKind::Browse,
            corpus,
            git,
            is_dir,
            vec![],
            vec![],
            None,
        );
        let shown = ov.item_strings();
        assert!(!shown.iter().any(|s| s.starts_with(".config")), "dot dir hidden: {shown:?}");
        assert!(shown.iter().any(|s| s == "notes.md"));
        assert!(shown.iter().any(|s| s == ".env"), ".env visible in browse too");
        assert!(ov.toggle_hidden());
        assert!(ov.item_strings().iter().any(|s| s.starts_with(".config")), "dot dir revealed");
    }

    #[test]
    fn non_file_picker_ignores_hidden_toggle() {
        // A theme/command picker never hides dotfiles and the toggle is a no-op.
        let mut ov = OverlayState::new_command(
            vec!["Save".into(), ".secret command".into()],
            vec!["C-x C-s".into(), String::new()],
        );
        assert!(!ov.kind.hides_dotfiles());
        let before = ov.item_strings();
        assert!(!ov.toggle_hidden(), "toggle is a no-op for a non-file picker");
        assert!(!ov.show_hidden);
        assert_eq!(ov.item_strings(), before, "listing unchanged");
    }

    #[test]
    fn move_clamps() {
        let mut ov = OverlayState::new(OverlayKind::Goto, corpus(), vec![], vec![]);
        ov.move_sel(-1);
        assert_eq!(ov.selected, 0);
        ov.move_sel(100);
        assert_eq!(ov.selected, ov.items.len() - 1);
    }

    #[test]
    fn switch_marks_git_children() {
        // Project picker: repo-alpha/repo-beta are git, plain-notes is not.
        let corpus = vec![
            "plain-notes".to_string(),
            "repo-alpha".to_string(),
            "repo-beta".to_string(),
        ];
        let git = vec![false, true, true];
        let is_dir = vec![true, true, true];
        let ov = OverlayState::new_marked(
            OverlayKind::Project,
            corpus,
            git,
            is_dir,
            vec![],
            vec![],
            None,
        );
        let items = ov.item_strings();
        // Git children get a • marker; the raw name is still a substring.
        assert!(items.iter().any(|s| s.contains("repo-alpha") && s.contains('•')));
        assert!(items.iter().any(|s| s.contains("repo-beta") && s.contains('•')));
        // plain-notes is a plain folder: trailing slash, no git marker.
        let pn = items.iter().find(|s| s.contains("plain-notes")).unwrap();
        assert!(!pn.contains('•'), "plain folder must not be git-marked: {pn}");
        assert!(pn.ends_with('/'));
        // The accept value is always the RAW name (no marker).
        assert_eq!(ov.corpus[ov.selected_corpus_index().unwrap()], "plain-notes");
    }

    #[test]
    fn new_project_pins_accept_row_and_marks_git() {
        // Folders for the explorer level: a plain folder + two git repos.
        let folders = vec![
            ("plain-notes".to_string(), false),
            ("repo-alpha".to_string(), true),
            ("repo-beta".to_string(), true),
        ];
        let ov = OverlayState::new_project("/ws".to_string(), folders);
        assert_eq!(ov.kind.as_str(), "switch");
        // The synthetic "." accept-this-folder row is pinned at the TOP.
        let items = ov.item_strings();
        assert_eq!(items[0], ".");
        // browse_dir carries the ABSOLUTE dir for path navigation.
        assert_eq!(ov.browse_dir.as_deref(), Some("/ws"));
        // Default selection skips "." and lands on the first REAL folder so
        // Right/Enter descends immediately.
        assert_eq!(ov.selected_value(), Some("plain-notes"));
        assert!(ov.selected_is_dir(), "first folder is a directory");
        // Git children keep the • marker; "." is neither git nor a dir.
        assert!(items.iter().any(|s| s.contains("repo-alpha") && s.contains('•')));
        assert!(!items[0].contains('•') && !items[0].ends_with('/'));
    }

    #[test]
    fn theme_picker_groups_by_lens_and_selects_active() {
        use crate::theme::Lens;
        // The full world corpus (in THEMES order) + Gumtree active (its index).
        let names: Vec<String> = crate::theme::THEMES.iter().map(|t| t.name.to_string()).collect();
        let gum = names.iter().position(|n| n == "Gumtree").unwrap();
        // Open with Gumtree active -> mode "theme", opens on the flat All lens.
        let mut ov = OverlayState::new_theme(names.clone(), gum);
        assert_eq!(ov.kind.as_str(), "theme");
        assert_eq!(ov.original_theme, Some(gum));
        assert_eq!(ov.theme_lens, Lens::All, "opens on the flat All landing");
        assert_eq!(ov.selected_value(), Some("Gumtree"));
        // Step into the Time lens to exercise the grouping (Gumtree is shown under Time).
        ov.set_theme_lens(Lens::Time);
        assert_eq!(ov.theme_lens, Lens::Time);
        // The active world is highlighted (and thus previewed) wherever its section is.
        assert_eq!(ov.selected_value(), Some("Gumtree"));
        // Grouped by Time: rows come out in section order (Dawn, Day, Dusk, Night),
        // and each row's parallel section label matches the world's Time tag.
        let sections = ov.item_sections();
        assert_eq!(sections.len(), ov.item_strings().len());
        for (row, name) in ov.item_strings().iter().enumerate() {
            // Every grouped row is a SHOWN world, so its Time tag is `Some`.
            assert_eq!(
                Some(sections[row].as_str()),
                crate::theme::tag_for(name, Lens::Time),
                "row {name} under wrong section"
            );
        }
        // Sections appear in the lens's declared order (no interleaving).
        let order: Vec<&str> = Lens::Time.sections().to_vec();
        let mut last = 0usize;
        for s in &sections {
            let pos = order.iter().position(|o| o == s).unwrap();
            assert!(pos >= last, "sections must be contiguous + ordered: {sections:?}");
            last = pos;
        }
        // No git / dir markers on the theme rows.
        assert!(ov.item_strings().iter().all(|s| !s.contains('•') && !s.ends_with('/')));
    }

    #[test]
    fn theme_lens_cycles_with_all_parked_left_and_keeps_world() {
        use crate::theme::Lens;
        let names: Vec<String> = crate::theme::THEMES.iter().map(|t| t.name.to_string()).collect();
        // Potoroo headlines ALL four faceted lenses, so it survives every regroup.
        let potoroo = names.iter().position(|n| n == "Potoroo").unwrap();
        let mut ov = OverlayState::new_theme(names, potoroo);
        assert_eq!(ov.theme_lens, Lens::All, "opens on the far-left All landing");
        assert_eq!(ov.selected_value(), Some("Potoroo"));
        // The All lens is the flat corpus list (no section headers).
        assert!(ov.item_sections().iter().all(|s| s.is_empty()));
        // LEFT at All is a clamped no-op (nothing before it).
        ov.cycle_lens(-1);
        assert_eq!(ov.theme_lens, Lens::All, "All is the far-left floor");
        // RIGHT steps along the strip; the highlighted world is KEPT across regroups.
        for expect in [Lens::Time, Lens::Register, Lens::Voice, Lens::Temperature] {
            ov.cycle_lens(1);
            assert_eq!(ov.theme_lens, expect);
            assert_eq!(ov.selected_value(), Some("Potoroo"));
        }
        // RIGHT at Temperature is a clamped no-op (it is now the far-right end).
        ov.cycle_lens(1);
        assert_eq!(ov.theme_lens, Lens::Temperature, "Temperature parked at the far right");
        // The lens strip reflects the active lens (exactly one active, All FIRST).
        let strip = ov.lens_strip();
        assert_eq!(strip.len(), 5);
        assert_eq!(strip.first().unwrap().0, "All");
        assert_eq!(strip.iter().filter(|(_, a)| *a).count(), 1);
        assert!(strip[4].1, "Temperature is active");
    }

    #[test]
    fn opted_out_world_hidden_under_its_lens_but_present_under_all() {
        use crate::theme::Lens;
        let names: Vec<String> = crate::theme::THEMES.iter().map(|t| t.name.to_string()).collect();
        let gum = names.iter().position(|n| n == "Gumtree").unwrap();
        let mut ov = OverlayState::new_theme(names, gum);
        // Tawny opts OUT of Voice (voice: None), so it never appears in the Voice
        // grouping — every SHOWN row under Voice has a `Some` Voice tag.
        ov.set_theme_lens(Lens::Voice);
        assert!(
            !ov.item_strings().iter().any(|n| n == "Tawny"),
            "Tawny is hidden under the Voice lens"
        );
        for name in ov.item_strings() {
            assert!(
                crate::theme::tag_for(&name, Lens::Voice).is_some(),
                "{name} shown under Voice must carry a Some tag"
            );
        }
        // But the flat All lens still lists EVERY world, Tawny included (opt-out only
        // trims the faceted lenses; nothing is unreachable).
        ov.set_theme_lens(Lens::All);
        assert_eq!(ov.item_strings().len(), crate::theme::THEMES.len());
        assert!(ov.item_strings().iter().any(|n| n == "Tawny"));
    }

    #[test]
    fn theme_lens_is_flat_all_and_no_strip_for_nontheme() {
        // A non-theme picker never grows a lens strip or section labels.
        let ov = OverlayState::new(OverlayKind::Goto, corpus(), vec![], vec![]);
        assert!(ov.lens_strip().is_empty());
        assert!(ov.item_sections().iter().all(|s| s.is_empty()));
        // cycle_lens on a non-theme picker is inert.
        let mut ov = ov;
        ov.cycle_lens(1);
        assert_eq!(ov.theme_lens, crate::theme::Lens::All);
    }

    #[test]
    fn caret_picker_lists_three_styles_navigates_and_maps_modes() {
        use crate::caret::CaretMode;
        // SUMMON with Block active: the corpus is the three look labels in ALL order,
        // each row's "binding" column carrying its description.
        let ov = OverlayState::new_caret(CaretMode::Block);
        assert_eq!(ov.kind.as_str(), "caret");
        assert_eq!(ov.item_strings(), vec!["Block", "Morph", "I-beam"]);
        assert_eq!(
            ov.item_bindings(),
            vec![
                "rounded square + trailing underline",
                "takes the glyph silhouette",
                "an alive insertion bar",
            ]
        );
        // Opens highlighting the ACTIVE look, and `original_caret` remembers it.
        assert_eq!(ov.selected_value(), Some("Block"));
        assert_eq!(ov.selected_caret_mode(), Some(CaretMode::Block));
        assert_eq!(ov.original_caret, Some(CaretMode::Block));
        // NAVIGATE down the list -> the selected look maps back via from_label.
        let mut ov = ov;
        ov.move_sel(1);
        assert_eq!(ov.selected_caret_mode(), Some(CaretMode::Morph));
        ov.move_sel(1);
        assert_eq!(ov.selected_caret_mode(), Some(CaretMode::Ibeam));
        // Opening with a non-Block look pre-selects THAT row.
        let ov2 = OverlayState::new_caret(CaretMode::Ibeam);
        assert_eq!(ov2.selected_value(), Some("I-beam"));
        assert_eq!(ov2.original_caret, Some(CaretMode::Ibeam));
        // The hint names ↵'s action; flat picker (no descend).
        assert_eq!(OverlayKind::Caret.hint(), "\u{21B5} apply");
        // selected_caret_mode is None for a non-caret picker.
        let theme = OverlayState::new_theme(vec!["Tawny".into()], 0);
        assert_eq!(theme.selected_caret_mode(), None);
    }

    #[test]
    fn command_palette_lists_names_with_parallel_bindings() {
        let names = vec![
            "Go to file".to_string(),
            "Switch theme".to_string(),
            "Save".to_string(),
        ];
        let binds = vec!["C-x C-f".to_string(), "C-x t".to_string(), "C-x C-s".to_string()];
        let mut ov = OverlayState::new_command(names.clone(), binds.clone());
        assert_eq!(ov.kind.as_str(), "command");
        // Empty query: rows are the names in order, bindings stay parallel.
        assert_eq!(ov.item_strings(), names);
        assert_eq!(ov.item_bindings(), binds);
        // Fuzzy filter narrows to "Switch theme" and keeps its binding aligned.
        ov.push('t');
        ov.push('h');
        ov.push('e');
        assert_eq!(ov.selected_value(), Some("Switch theme"));
        assert_eq!(ov.item_bindings().first().map(|s| s.as_str()), Some("C-x t"));
    }

    #[test]
    fn outline_picker_lists_headings_and_jumps_by_line() {
        // (indented display label, document line) for three headings.
        let headings = vec![
            ("Intro".to_string(), 0usize),
            ("  Setup".to_string(), 4usize),
            ("  Usage".to_string(), 9usize),
        ];
        let mut ov = OverlayState::new_outline(headings);
        assert_eq!(ov.kind.as_str(), "outline");
        // Rows are the (indented) titles in order; lines stay parallel.
        assert_eq!(ov.item_strings(), vec!["Intro", "  Setup", "  Usage"]);
        assert_eq!(ov.selected_line(), Some(0));
        // Fuzzy filter to "Usage" -> selected row jumps to its line (9), not its text.
        ov.push('u');
        ov.push('s');
        ov.push('a');
        assert_eq!(ov.selected_value(), Some("  Usage"));
        assert_eq!(ov.selected_line(), Some(9));
        // No git / dir markers on outline rows; the indentation survives in display.
        assert!(ov.item_strings().iter().all(|s| !s.contains('•') && !s.ends_with('/')));
    }

    #[test]
    fn spell_picker_lists_suggestions_and_carries_target() {
        // Three corrections for a word flagged at line 2, cols 6..13.
        let sugg = vec!["receive".to_string(), "relieve".to_string(), "reprieve".to_string()];
        let ov = OverlayState::new_spell(sugg.clone(), (2, 6, 13));
        assert_eq!(ov.kind.as_str(), "spell");
        // Rows are the suggestions in order (best first); the top is selected.
        assert_eq!(ov.item_strings(), sugg);
        assert_eq!(ov.selected_value(), Some("receive"));
        // The target span is carried so the accept can replace the word.
        assert_eq!(ov.spell_target, Some((2, 6, 13)));
        // No git / dir markers on the suggestion rows.
        assert!(ov.item_strings().iter().all(|s| !s.contains('•') && !s.ends_with('/')));
        // The hint names the ↵ action (replace), flat picker (no descend).
        assert_eq!(OverlayKind::Spell.hint(), "\u{21B5} replace");
    }

    /// Three history rows newest-first, exercising both WHICH shapes (a git
    /// subject, an edited-heading description) and an empty which.
    fn history_rows() -> Vec<crate::history::TimelineRow> {
        let row = |when: &str, which: &str, counts: &str, id: &str| crate::history::TimelineRow {
            when: when.to_string(),
            which: which.to_string(),
            counts: counts.to_string(),
            id: id.to_string(),
        };
        vec![
            row("just now", "fix: the engine", "+0 −0", "300"),
            row("2 min ago", "edited \"Two flows\"", "+0 −1", "200"),
            row("1 hr ago", "", "+1 −2", "100"),
        ]
    }

    #[test]
    fn history_picker_lists_versions_navigates_and_carries_ids() {
        let mut ov = OverlayState::new_history(history_rows());
        assert_eq!(ov.kind.as_str(), "history");
        // The top (newest) row is selected; its restore id is the accept value.
        assert_eq!(ov.selected_history_id(), Some("300"));
        // NAVIGATE down -> the selected id tracks the highlighted version.
        ov.move_sel(1);
        assert_eq!(ov.selected_history_id(), Some("200"));
        ov.move_sel(1);
        assert_eq!(ov.selected_history_id(), Some("100"));
        // No git / dir markers on the version rows.
        assert!(ov.item_strings().iter().all(|s| !s.contains('•') && !s.ends_with('/')));
        // The hint teaches restore + close (informational, button-free).
        assert_eq!(OverlayKind::History.hint(), "↵ restore   ⌫/esc close");
        assert!(ov.foot_hint().contains("restore"));
    }

    #[test]
    fn history_rows_show_when_dot_which_and_counts_ride_the_faint_column() {
        // The MAIN column composes "when · which" (the bare when for an empty
        // which); the faint right column carries the "+N −M" changed-counts —
        // the existing binding-column pattern, zero new layout.
        let ov = OverlayState::new_history(history_rows());
        assert_eq!(
            ov.item_strings(),
            vec![
                "just now · fix: the engine",
                "2 min ago · edited \"Two flows\"",
                "1 hr ago",
            ]
        );
        assert_eq!(ov.item_bindings(), vec!["+0 −0", "+0 −1", "+1 −2"]);
        // The composed corpus is what the fuzzy filter matches, so a SUBJECT
        // query finds its version (a free win of the composition).
        let mut ov = OverlayState::new_history(history_rows());
        for c in "engine".chars() {
            ov.push(c);
        }
        assert_eq!(ov.item_strings().len(), 1);
        assert_eq!(ov.selected_history_id(), Some("300"));
    }

    #[test]
    fn history_picker_empty_state_shows_calm_row_and_no_op_accept() {
        // No versions -> a single "no history yet" row whose id is empty (Enter no-ops).
        let ov = OverlayState::new_history(Vec::new());
        assert_eq!(ov.kind.as_str(), "history");
        assert_eq!(ov.item_strings(), vec!["no history yet"]);
        assert_eq!(ov.selected_history_id(), None, "empty-id row is not restorable");
    }

    #[test]
    fn keybindings_capture_key_mode_finishes_instantly() {
        // SUMMON: the rebind menu lists the catalog with its effective chords.
        let names = crate::commands::names();
        let binds = crate::commands::effective_bindings(&[]);
        let mut ov = OverlayState::new_keybindings(names.clone(), binds);
        assert_eq!(ov.kind.as_str(), "keybindings");
        assert_eq!(ov.item_strings(), names);
        assert!(ov.capture.is_none());
        // NAVIGATE: filter to "Undo" so the selected command is deterministic.
        for c in "undo".chars() {
            ov.push(c);
        }
        assert_eq!(ov.selected_value(), Some("Undo"));
        // ENTER → ChooseMode; default selection is KEY.
        ov.start_capture();
        let cap = ov.capture.as_ref().unwrap();
        assert_eq!(cap.stage, CaptureStage::ChooseMode);
        assert_eq!(cap.cmd_name, "Undo");
        assert!(!cap.chord_mode);
        // Choose KEY, begin recording, then ONE combo finishes instantly.
        ov.capture_move_mode(-1); // KEY row
        ov.capture_begin_recording();
        assert_eq!(ov.capture.as_ref().unwrap().stage, CaptureStage::Recording);
        let done = ov.capture_record("C-j".to_string());
        assert!(done, "KEY mode finishes on the first combo");
        assert_eq!(ov.capture_target(), Some(("undo".to_string(), "C-j".to_string())));
    }

    #[test]
    fn keybindings_capture_chord_mode_collects_then_finishes() {
        let mut ov = OverlayState::new_keybindings(
            crate::commands::names(),
            crate::commands::effective_bindings(&[]),
        );
        for c in "save".chars() {
            ov.push(c);
        }
        assert_eq!(ov.selected_value(), Some("Save"));
        ov.start_capture();
        ov.capture_move_mode(1); // CHORD row
        ov.capture_begin_recording();
        assert!(ov.capture.as_ref().unwrap().chord_mode);
        // First combo does NOT finish a chord; the 2-deep cap does.
        assert!(!ov.capture_record("C-x".to_string()));
        assert!(ov.capture_record("C-s".to_string()));
        // A THIRD combo is dropped (capped at 2).
        assert!(ov.capture_record("C-q".to_string()));
        assert_eq!(
            ov.capture_target(),
            Some(("save".to_string(), "C-x C-s".to_string()))
        );
    }

    #[test]
    fn keybindings_confirm_and_reset_helpers() {
        let mut ov = OverlayState::new_keybindings(
            crate::commands::names(),
            crate::commands::effective_bindings(&[]),
        );
        // RESET targets the highlighted command's slug.
        for c in "redo".chars() {
            ov.push(c);
        }
        assert_eq!(ov.selected_command_slug().as_deref(), Some("redo"));
        // CONFLICT: a finished capture can be pushed into the Confirm phase, which the
        // prompt reflects (naming the clashing command). Esc-equivalent aborts it.
        ov.start_capture();
        ov.capture_begin_recording();
        ov.capture_record("C-s".to_string());
        ov.capture_into_confirm("Search forward".to_string());
        let cap = ov.capture.as_ref().unwrap();
        assert_eq!(cap.stage, CaptureStage::Confirm);
        assert!(cap.prompt().contains("Search forward"));
        ov.capture_abort();
        assert!(ov.capture.is_none());
    }

    #[test]
    fn hint_teaches_descend_only_for_navigable_kinds() {
        // Navigable explorers teach the select-vs-descend asymmetry (->/C-f open,
        // Enter selects/accepts, <-/C-b up).
        for k in [OverlayKind::Project, OverlayKind::MoveDest, OverlayKind::Browse] {
            let h = k.hint();
            assert!(h.contains("->/C-f"), "{k:?} hint should teach descend: {h}");
            assert!(h.contains("<-/C-b"), "{k:?} hint should teach ascend: {h}");
            // The Return keycap is the ↵ glyph, not the word "Enter".
            assert!(h.contains('\u{21B5}'), "{k:?} hint should name ↵ Return: {h}");
        }
        // Project ↵ SELECTS; MoveDest ↵ MOVES; Browse ↵ OPENS.
        assert!(OverlayKind::Project.hint().contains("\u{21B5} select"));
        assert!(OverlayKind::MoveDest.hint().contains("move here"));
        assert!(OverlayKind::Browse.hint().contains("\u{21B5} open"));
        // Flat pickers have NO descend hint — ↵ only.
        for k in [OverlayKind::Goto, OverlayKind::Theme, OverlayKind::Command] {
            let h = k.hint();
            assert!(!h.contains("C-f"), "{k:?} is flat, no descend: {h}");
            assert!(h.starts_with('\u{21B5}'), "{k:?} hint names ↵ Return: {h}");
        }
    }

    // A Goto picker over N synthetic rows (row0..rowN-1), empty query so items are in
    // corpus order 1:1.
    fn deep(n: usize) -> OverlayState {
        let corpus: Vec<String> = (0..n).map(|i| format!("row{i}")).collect();
        OverlayState::new(OverlayKind::Goto, corpus, vec![], vec![])
    }

    #[test]
    fn hover_only_highlights_visible_rows_and_never_scrolls() {
        // 40 rows, window 12. Keyboard down to row 30 → the window scrolls so 30 is the
        // BOTTOM visible row (scroll = 30+1-12 = 19), showing items 19..=30.
        let mut ov = deep(40);
        ov.move_sel(30);
        assert_eq!(ov.selected, 30);
        assert_eq!(ov.scroll, 19);
        // Hovering a row INSIDE the visible band re-highlights it WITHOUT moving scroll.
        assert!(ov.hover_select(21));
        assert_eq!(ov.selected, 21);
        assert_eq!(ov.scroll, 19, "a hover must NOT move the scroll window");
        // Hovering the TOP visible row: still no scroll (the bug was this scrolling up).
        assert!(ov.hover_select(19));
        assert_eq!(ov.scroll, 19);
        // Hovering ABOVE the band (a row scrolled off the top) is REJECTED, no change.
        assert!(!ov.hover_select(5));
        assert_eq!(ov.selected, 19);
        assert_eq!(ov.scroll, 19);
        // Hovering BELOW the band (past the last visible row) is likewise rejected.
        assert!(!ov.hover_select(31));
        assert_eq!(ov.selected, 19);
        assert_eq!(ov.scroll, 19);
        // Re-hovering the SAME row is a no-op (returns false, nothing moved).
        assert!(!ov.hover_select(19));
    }

    #[test]
    fn keyboard_move_keeps_selection_in_the_window() {
        let mut ov = deep(40);
        // Down a page-ish: selection tracks, window scrolls the minimum to keep it shown.
        ov.move_sel(15);
        assert_eq!(ov.selected, 15);
        assert_eq!(ov.scroll, 4); // 15+1-12
        assert!(ov.selected >= ov.scroll && ov.selected < ov.scroll + ov.window_rows());
        // Back up above the window → scroll follows up (never leaves selection off-screen).
        ov.move_sel(-14);
        assert_eq!(ov.selected, 1);
        assert_eq!(ov.scroll, 1);
        // A short list never scrolls.
        let mut small = deep(5);
        small.move_sel(100);
        assert_eq!(small.selected, 4);
        assert_eq!(small.scroll, 0);
    }

    #[test]
    fn query_edit_resets_scroll_to_top() {
        let mut ov = deep(40);
        ov.move_sel(30);
        assert_eq!(ov.scroll, 19);
        ov.push('r'); // matches every "rowN" → list stays long, but selection resets top
        assert_eq!(ov.selected, 0);
        assert_eq!(ov.scroll, 0);
    }

    #[test]
    fn elide_keeps_filename_and_extension_with_one_ellipsis() {
        // A deep path, narrow budget: the filename + ext survive, the DIR is elided.
        let out = elide_path("src/app/render/chrome.rs", 16);
        assert!(out.ends_with("chrome.rs"), "filename+ext must survive: {out}");
        assert_eq!(out.matches('…').count(), 1, "exactly one ellipsis: {out}");
        assert!(out.chars().count() <= 16, "fits the budget: {out}");
        // The split point is the last '/': dir prefix (muted) vs filename (content).
        let split = row_split(&out);
        assert!(out[..split].ends_with('/'));
        assert_eq!(&out[split..], "chrome.rs");
        // A row that already fits is returned WHOLE (no ellipsis, no change).
        assert_eq!(elide_path("src/main.rs", 40), "src/main.rs");
        assert_eq!(row_split("src/main.rs"), 4); // "src/"
    }

    #[test]
    fn elide_middle_truncates_the_filename_when_it_alone_overflows() {
        // Filename longer than the whole budget → the filename ITSELF is middle-elided,
        // the directory dropped, extension end kept, still a single ellipsis.
        let out = elide_path("deep/dir/averyveryverylongfilename.rs", 12);
        assert_eq!(out.matches('…').count(), 1, "one ellipsis: {out}");
        assert!(out.chars().count() <= 12, "fits: {out}");
        assert!(out.ends_with(".rs"), "extension survives: {out}");
        assert!(!out.contains('/'), "dir dropped when the filename alone overflows: {out}");
        assert_eq!(row_split(&out), 0, "no '/', so all content ink");
        // A bare filename with no directory elides the same way.
        let bare = elide_path("supercalifragilistic.md", 10);
        assert!(bare.ends_with(".md") && bare.matches('…').count() == 1);
    }

    #[test]
    fn browse_dir_flags_directories() {
        // One level: a folder (docs) and a file (README.md).
        let corpus = vec!["docs".to_string(), "README.md".to_string()];
        let git = vec![false, false];
        let is_dir = vec![true, false];
        let mut ov = OverlayState::new_marked(
            OverlayKind::Browse,
            corpus,
            git,
            is_dir,
            vec![],
            vec![],
            None,
        );
        // docs selected -> a directory; README.md selected -> a file.
        assert!(ov.selected_is_dir());
        ov.move_sel(1);
        assert!(!ov.selected_is_dir());
        assert_eq!(ov.selected_value(), Some("README.md"));
    }
}
