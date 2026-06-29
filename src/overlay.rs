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
    /// The MOVE-DESTINATION picker (C-x m): reuses the Browse navigator but lists
    /// only FOLDERS (you move a note INTO a folder). It is rooted at the notes
    /// root. Right/`ForwardChar` DESCENDS into the highlighted folder, Left ASCENDS,
    /// and Enter ACCEPTS the destination — either the highlighted folder, or, when
    /// the typed query matches no listed folder, a NEW folder of that name to
    /// create. The accepted value is a notes-root-relative directory path.
    MoveDest,
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
            OverlayKind::MoveDest => "move",
            OverlayKind::Command => "command",
            OverlayKind::Outline => "outline",
            OverlayKind::Spell => "spell",
            OverlayKind::Keybindings => "keybindings",
        }
    }

    /// One quiet line of control hints for this picker, drawn DIM at the foot of
    /// the overlay card so the select-vs-descend model is discoverable. The
    /// NAVIGABLE explorers (Project/Browse/MoveDest) teach the asymmetry —
    /// `->`/C-f DESCEND, `<-`/C-b ascend, Enter SELECTS the highlighted item; the
    /// FLAT pickers (Goto/Theme/Command) have no descend, so they only name what
    /// Enter does. Rendered + surfaced to the sidecar so it stays agent-verifiable.
    pub fn hint(self) -> &'static str {
        match self {
            // Select context: Enter PICKS the folder as the root; descend is ->/C-f.
            OverlayKind::Project => "->/C-f open   Enter select   <-/C-b up",
            // Select context: Enter MOVES the note into the folder; descend is ->/C-f.
            OverlayKind::MoveDest => "->/C-f open   Enter move here   <-/C-b up",
            // Browse OPENS files; Enter on a folder descends (so does ->/C-f).
            OverlayKind::Browse => "->/C-f open   Enter open   <-/C-b up",
            // Flat pickers: no descend, Enter just accepts the highlighted row.
            OverlayKind::Goto => "Enter open",
            OverlayKind::Theme => "Enter select",
            OverlayKind::Command => "Enter run",
            OverlayKind::Outline => "Enter jump",
            OverlayKind::Spell => "Enter replace",
            // The rebind menu: Enter starts a capture, Delete resets the highlighted
            // command, Esc closes. (In a capture the prompt teaches Key/Chord/Enter/Esc.)
            OverlayKind::Keybindings => "Enter rebind   Delete reset   Esc close",
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
    /// Browse only: the root-relative directory this level lists (`None` = root).
    pub browse_dir: Option<String>,
    /// Theme picker only: the theme index that was ACTIVE when the picker opened,
    /// so a Cancel can REVERT the live preview to it. `None` for the other kinds.
    pub original_theme: Option<usize>,
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
    /// Keybindings menu only: the active CAPTURE sub-state, or `None` while browsing
    /// the command list. Drives the capture flow + the sidecar `capture` block.
    pub capture: Option<Capture>,
    /// Keybindings menu only: a transient one-line NOTICE (a conflict warning, a
    /// "saved …" / "reset …" confirmation), drawn dim + surfaced to the sidecar.
    /// Empty for every other kind and between actions.
    pub notice: String,
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
            browse_dir,
            original_theme: None,
            bindings: Vec::new(),
            times: Vec::new(),
            lines: Vec::new(),
            spell_target: None,
            capture: None,
            notice: String::new(),
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
        // Empty query => items are in corpus order, so the active world sits at
        // `active_index`; select it so the picker opens on the current world.
        if let Some(pos) = s.items.iter().position(|&i| i == active_index) {
            s.selected = pos;
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

    /// Re-rank `corpus` against the current query into `items`, clamping the
    /// selection. Called after every query edit.
    pub fn refilter(&mut self) {
        let ranked = fuzzy::rank(&self.query, &self.corpus, |i| {
            if self.open.contains(&i) {
                Tier::Open
            } else if self.recent.contains(&i) {
                Tier::Recent
            } else {
                Tier::Corpus
            }
        });
        self.items = ranked.into_iter().map(|r| r.index).collect();
        if self.selected >= self.items.len() {
            self.selected = self.items.len().saturating_sub(1);
        }
    }

    /// Append a char to the query and refilter.
    pub fn push(&mut self, c: char) {
        self.query.push(c);
        self.selected = 0;
        self.refilter();
    }

    /// Remove the last query char and refilter.
    pub fn pop(&mut self) {
        self.query.pop();
        self.selected = 0;
        self.refilter();
    }

    /// Move the selection by `delta` rows, clamped to the visible item range.
    pub fn move_sel(&mut self, delta: isize) {
        if self.items.is_empty() {
            self.selected = 0;
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
    fn theme_picker_lists_names_and_selects_active() {
        let names = vec![
            "Tawny".to_string(),
            "Potoroo".to_string(),
            "Gumtree".to_string(),
        ];
        // Open with index 2 active -> that row is selected; mode is "theme".
        let ov = OverlayState::new_theme(names.clone(), 2);
        assert_eq!(ov.kind.as_str(), "theme");
        assert_eq!(ov.original_theme, Some(2));
        assert_eq!(ov.item_strings(), names);
        assert_eq!(ov.selected_value(), Some("Gumtree"));
        // No git / dir markers on the theme rows.
        assert!(ov.item_strings().iter().all(|s| !s.contains('•') && !s.ends_with('/')));
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
        // The hint names the Enter action (replace), flat picker (no descend).
        assert_eq!(OverlayKind::Spell.hint(), "Enter replace");
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
            assert!(h.contains("Enter"), "{k:?} hint should name Enter: {h}");
        }
        // Project Enter SELECTS; MoveDest Enter MOVES; Browse Enter OPENS.
        assert!(OverlayKind::Project.hint().contains("Enter select"));
        assert!(OverlayKind::MoveDest.hint().contains("move here"));
        assert!(OverlayKind::Browse.hint().contains("Enter open"));
        // Flat pickers have NO descend hint — Enter only.
        for k in [OverlayKind::Goto, OverlayKind::Theme, OverlayKind::Command] {
            let h = k.hint();
            assert!(!h.contains("C-f"), "{k:?} is flat, no descend: {h}");
            assert!(h.starts_with("Enter"), "{k:?} hint names Enter: {h}");
        }
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
