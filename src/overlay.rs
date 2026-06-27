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
//!   * `Project` — the workspace's child directories (switch the active root).
//!     Git children carry a small marker distinct from plain folders.
//!   * `Browse`  — ONE directory level at a time for the active root. Enter on a
//!     FOLDER descends (the list becomes that folder's children); Left/Backspace
//!     ASCENDS; Enter on a FILE opens it and closes. Git folders are marked. It
//!     is still summoned + transient — it vanishes on open/cancel, never a tree.

use crate::fuzzy::{self, Tier};

/// Which kind of overlay is open. `Goto` lists the active project's file index;
/// `Project` lists the workspace's child projects; `Browse` walks one directory
/// level of the active root at a time.
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
        };
        s.refilter();
        s
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
