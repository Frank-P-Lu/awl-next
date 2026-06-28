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
