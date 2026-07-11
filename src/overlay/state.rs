//! `OverlayState` -- the live overlay data model -- plus its per-kind
//! CONSTRUCTORS (`new`/`new_theme`/`new_caret`/`new_project`/…). Split out of
//! the former `overlay.rs` monolith (2026-07 code-organization pass); every
//! item's path is unchanged (`overlay::OverlayState`) -- only the file it
//! lives in moved.

use super::{Capture, LinkEdit, OverlayKind, RenameEdit, ValueEdit, PIN_TAG};

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
    /// Caret-style picker only: whether NO explicit override was set when the
    /// picker opened (`crate::caret::is_auto()` at construction) — i.e. `true`
    /// iff `original_caret` is merely AUTO's momentary resolution, not a real
    /// pin. A Cancel must revert to auto ITSELF then (`caret::clear_override`),
    /// never re-pin `original_caret`'s concrete value — otherwise merely
    /// opening the picker to look and backing out silently freezes the caret
    /// at whatever look auto happened to resolve to, breaking its per-theme
    /// tracking for the rest of the session (the bug this field fixes). Always
    /// `false` for every other kind.
    pub original_caret_was_auto: bool,
    /// Command palette only: binding LABELS parallel to `corpus` (the current key
    /// chord for each command, shown dim beside its name). Empty for every other
    /// kind. Filtered into row order via [`item_bindings`].
    pub bindings: Vec<String>,
    /// Go-to (notes) only: a relative "last edited" LABEL parallel to `corpus`
    /// (e.g. "5m ago"), shown dim beside each file. Empty for every other kind AND
    /// in the headless capture path (where mtime is never read, for determinism).
    /// Filtered into row order via [`item_times`].
    pub times: Vec<String>,
    /// Go-to's HEADINGS lens only: the document LINE (0-based) each HEADING row jumps
    /// to, parallel to `corpus`. Enter on a heading row JUMPS the cursor to `lines[i]`
    /// (the accept value is this line number, not the heading text, because two
    /// headings can share a title). Empty for every other kind; for a Go-to it is
    /// padded to the full corpus length — file rows carry an unused `0` (guarded by
    /// the parallel `heading` flag), heading rows carry their real line.
    pub lines: Vec<usize>,
    /// Go-to only: parallel to `corpus`, true for the appended document-HEADING rows
    /// (the fold that retired the standalone Outline picker), false for the ordinary
    /// FILE rows. Drives the Headings-lens gate in [`Self::refilter`] (headings show
    /// ONLY under the Headings lens) and the accept split ([`Self::selected_is_heading`]:
    /// a heading row jumps to `lines[i]`, a file row opens the path). EMPTY for every
    /// other kind AND for a Go-to over a buffer with no headings — the gate is then inert.
    pub heading: Vec<bool>,
    /// Spell picker only: the misspelled word's `(line, start_col, end_col)` CHAR
    /// span, so the accept can map it to a buffer char range and replace it with the
    /// chosen suggestion. `None` for every other kind.
    pub spell_target: Option<(usize, usize, usize)>,
    /// History timeline only: the RESTORE key for each version, parallel to `corpus`
    /// (the row shows a relative timestamp; the id is the opaque handle
    /// [`crate::history::load`] resolves back to content). Enter on a row emits
    /// `history_ids[i]`; empty for every other kind AND for an empty history (which
    /// lists no rows and shows the shared "no history yet" empty-state message).
    pub history_ids: Vec<String>,
    /// Keybindings menu only: the active CAPTURE sub-state, or `None` while browsing
    /// the command list. Drives the capture flow + the sidecar `capture` block.
    pub capture: Option<Capture>,
    /// Keybindings menu only: a transient one-line NOTICE (a conflict warning, a
    /// "saved …" / "reset …" confirmation), drawn dim + surfaced to the sidecar.
    /// Empty for every other kind and between actions.
    pub notice: String,
    /// FACETING pickers only: the active lens as an INDEX into this picker's
    /// [`crate::facets::FacetScheme::strip`] (0 = the "All" home / flat list),
    /// cycled by LEFT/RIGHT. Drives the grouping of `items` into sections
    /// ([`Self::item_sections`]) and the lens STRIP. Left at 0 (the flat list) for a
    /// non-faceting picker, where [`crate::facets::scheme`] returns `None` and every
    /// facet method is a no-op. GENERIC: the picker's own scheme (keyed by
    /// [`Self::kind`]) supplies the lenses + bucketing — no per-picker type here.
    pub facet_lens: usize,
    /// HISTORY picker only: each corpus version's wall-clock stamp (millis),
    /// parallel to `corpus`, so the Session / Today lenses can bucket by time. Empty
    /// for every other kind (those rows carry no `ts`, so their time-less lenses,
    /// if any, opt every row out). Set by [`Self::new_history`].
    pub facet_ts: Vec<u64>,
    /// FACETING pickers with a clock-relative lens (History) only: the REFERENCE
    /// clock (millis) — `Some(now)` live, `None` in the headless capture path (which
    /// makes those lenses inert, the determinism gate). `None` for every other kind.
    pub facet_now: Option<u64>,
    /// History picker only: the current SESSION's start (millis) — `Some` live,
    /// `None` headless / untracked. Reference for the Session lens.
    pub facet_session_start: Option<u64>,
    /// THEME picker only: the SECTION label for each entry in `items`, parallel to it
    /// (the faint uppercase group header a row sits under). Empty strings under
    /// [`crate::theme::Lens::All`] and for every non-theme kind (no grouping). Rebuilt
    /// by [`Self::refilter`] alongside `items`.
    pub item_sections: Vec<String>,
    /// BREADCRUMB: the parent overlay to RE-SUMMON when THIS picker POPS (Esc/cancel,
    /// or a [`AcceptDisposition::ValuePick`] accept), instead of closing to the
    /// buffer. Two stamping doors, both single-level:
    ///   * `Some(OverlayKind::Settings)` when the settings menu opens a sub-picker
    ///     (theme / caret / dictionary / keybindings) or its PATH navigator — stamped
    ///     in place by `overlay_nav::settings_accept`.
    ///   * `Some(OverlayKind::Command)` when the COMMAND PALETTE runs a command that
    ///     opens an overlay — the palette closes then re-dispatches, and the resulting
    ///     overlay is stamped by `overlay_nav::stamp_return_to` at the palette
    ///     re-dispatch seam (live App + headless replay both).
    /// SINGLE-LEVEL only — the re-summoned parent is built FRESH (no breadcrumb of its
    /// own), so there is no N-deep stack and no A→B→A loop. `None` for a normal
    /// top-level summon (⌘O / ⌘T / a menu click / the vast majority), which closes to
    /// the buffer exactly as before. A NAVIGATING accept (open a file, switch project,
    /// restore a version) IGNORES this and closes the whole stack (`close_to_buffer`).
    pub return_to: Option<OverlayKind>,
    /// SETTINGS VALUE-EDIT sub-state: `Some` while a [`crate::settings::SettingKind::Value`]
    /// row is being edited inline (page widths / zoom), driving the modal intercept +
    /// the live cell. `None` for every other overlay and while just browsing Settings.
    pub value_edit: Option<ValueEdit>,
    /// FOLDER-NAVIGATOR opened FROM a Settings PATH row: the config key whose folder is
    /// being picked ("notes_root"/"workspace"/"project_root"). `Some` turns the Project
    /// navigator's Enter into a [`crate::actions::Effect::SettingPathPick`] (write the
    /// key + return to Settings) instead of the normal switch-project accept. `None` for
    /// every ordinary navigator summon.
    pub setting_path_key: Option<String>,
    /// File pickers only ([`OverlayKind::hides_dotfiles`]): whether dot-prefixed
    /// entries are REVEALED. Default `false` — the go-to / browse corpus HIDES any
    /// entry whose basename or an ancestor component starts with `.` (except `.env*`,
    /// [`crate::index::is_hidden_entry`]). `Cmd-Shift-.` (the Finder convention) flips
    /// it via [`Self::toggle_hidden`], which re-runs the display filter in
    /// [`Self::refilter`]. TRANSIENT: every fresh summon defaults hidden again (it's
    /// a field of the live picker, not a sticky global). Ignored by non-file pickers.
    pub show_hidden: bool,
    /// NOTES VERBS round: the RENAME minibuffer's live typed-state (`Some` only for
    /// `OverlayKind::Rename`, armed the instant the overlay is built by
    /// [`Self::new_rename`] — never toggled on later, unlike `value_edit`/`capture`,
    /// since Rename has nothing to browse before typing starts). `None` for every
    /// other kind.
    pub rename_edit: Option<RenameEdit>,
    /// LINKS V2: the Cmd-K minibuffer's live typed-URL sub-state (`Some` only for
    /// `OverlayKind::InsertLink`, armed the instant the overlay is built by
    /// [`Self::new_link_edit`] — mirrors `rename_edit`'s shape exactly). `None`
    /// for every other kind.
    pub link_edit: Option<LinkEdit>,
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
            original_caret_was_auto: false,
            bindings: Vec::new(),
            times: Vec::new(),
            lines: Vec::new(),
            heading: Vec::new(),
            spell_target: None,
            history_ids: Vec::new(),
            capture: None,
            notice: String::new(),
            // Default to the "All" home (strip index 0 = the flat list). A faceting
            // picker LANDS here; ←/→ step into the refinement lenses. Non-faceting
            // kinds ignore it (no scheme).
            facet_lens: 0,
            facet_ts: Vec::new(),
            facet_now: None,
            facet_session_start: None,
            item_sections: Vec::new(),
            // No breadcrumb by default: a top-level summon closes to the buffer. The
            // settings menu stamps this on a sub-picker it opens (see `close_overlay`).
            return_to: None,
            // No inline value edit / path-pick target on a fresh summon; the settings
            // menu arms these when Enter lands on a Value / Path row.
            value_edit: None,
            setting_path_key: None,
            // Fresh summon: dotfiles HIDDEN by default (the toggle is transient).
            show_hidden: false,
            // No rename edit on a fresh summon; `new_rename` arms it right after.
            rename_edit: None,
            // No link edit on a fresh summon; `new_link_edit` arms it right after.
            link_edit: None,
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
        // Open on All (the far-LEFT home, strip index 0): the ACTIVE world is always
        // present in the flat list, so the picker opens highlighting (and previewing) the
        // current world with no surprise — under the OPT-OUT faceting a world may be hidden
        // on any given lens (the default world, Tawny, is hidden on Time), so a faceted
        // default could neither highlight it nor preview it. RIGHT steps into the lenses.
        s.facet_lens = 0;
        s.refilter();
        // Select the active world in whatever section it now sits in, so the picker
        // opens highlighting (and previewing) the current world.
        if let Some(pos) = s.items.iter().position(|&i| i == active_index) {
            s.selected = pos;
            s.scroll_to_selected();
        }
        s
    }

    /// Build the CARET-STYLE picker: the corpus is the three caret-look LABELS (in
    /// [`crate::caret::CaretMode::ALL`] order — Block / Morph / I-beam), each row's
    /// `bindings` column carrying that look's one-line description (drawn dim beside
    /// the name, reusing the palette's right column). `active` is the look in effect
    /// when the picker opened, remembered (`original_caret`) so a Cancel reverts the
    /// live preview, and pre-selected so the open frame previews the current look.
    ///
    /// `original_caret_was_auto` is captured HERE, from the live `crate::caret::
    /// is_auto()` global, not derived from `active` — the two real call sites
    /// (`overlay::build`, the live App's palette handler) always pass
    /// `crate::caret::mode()` as `active`, so the global is in step by
    /// construction; a Cancel then knows whether reverting means restoring a
    /// real pin or clearing back to auto (see [`crate::caret::clear_override`]).
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
        s.original_caret_was_auto = crate::caret::is_auto();
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

    /// Build the CJK-PRIORITY LANGUAGE picker: the corpus is the four ambiguity-
    /// ladder languages' writer-word LABELS (in
    /// [`crate::frontmatter::DEFAULT_CJK_PRIORITY`] order — the canonical DISPLAY
    /// order; this is NOT re-sorted to the live ladder's own order, exactly like
    /// Dictionary's fixed `ALL` order), each row's `bindings` column carrying its
    /// one-line description — the SAME shape as [`new_dictionary`](Self::new_dictionary).
    /// `active` (the language currently at the FRONT of the live ladder)
    /// pre-selects the picker's open frame.
    pub fn new_cjk_lang(active: crate::frontmatter::Lang) -> Self {
        let names: Vec<String> = crate::frontmatter::DEFAULT_CJK_PRIORITY
            .iter()
            .map(|l| l.label().to_string())
            .collect();
        let descriptions: Vec<String> = crate::frontmatter::DEFAULT_CJK_PRIORITY
            .iter()
            .map(|l| l.description().to_string())
            .collect();
        let n = names.len();
        let mut s = Self::new_marked(
            OverlayKind::CjkLang,
            names,
            vec![false; n],
            vec![false; n],
            Vec::new(),
            Vec::new(),
            None,
        );
        s.bindings = descriptions;
        if let Some(active_index) =
            crate::frontmatter::DEFAULT_CJK_PRIORITY.iter().position(|&l| l == active)
        {
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
    /// real folder, so Enter DESCENDS into it (or the `"."` row above, Up, SELECTS
    /// this folder as the root) — matching the Browse navigator now that Project
    /// FACETS (←/→ cycle the All/Recent lens).
    ///
    /// `recent_roots` is the persisted recent-PROJECTS MRU (absolute paths,
    /// most-recent first, [`crate::recents`]) — each folder present at THIS level
    /// whose absolute path is in the MRU is marked in `recent` (in MRU order), so
    /// the **Recent** lens lists exactly those, newest-first. A folder not in the
    /// MRU opts out; an EMPTY MRU (fresh session / the headless capture path) leaves
    /// `recent` empty, so the Recent lens shows its empty state.
    pub fn new_project(
        dir_abs: String,
        folders: Vec<(String, bool)>,
        recent_roots: &[String],
    ) -> Self {
        let mut corpus = vec![".".to_string()];
        let mut git = vec![false];
        let mut is_dir = vec![false];
        for (name, is_git) in folders {
            corpus.push(name);
            git.push(is_git);
            is_dir.push(true);
        }
        // Match each recent-PROJECT root (in MRU order) to a folder at THIS level by
        // ABSOLUTE path (base dir + child name), collecting the corpus indices in MRU
        // order so the Recent lens reads most-recent first (refilter's MRU tiebreak
        // consumes the order). A recent root not present here simply opts out; the
        // synthetic "." row (index 0) is skipped.
        let base = std::path::Path::new(&dir_abs);
        let mut recent = Vec::new();
        for root in recent_roots {
            let rp = std::path::Path::new(root);
            if let Some(ci) = (1..corpus.len()).find(|&i| base.join(&corpus[i]) == rp) {
                if !recent.contains(&ci) {
                    recent.push(ci);
                }
            }
        }
        let mut s = Self::new_marked(
            OverlayKind::Project,
            corpus,
            git,
            is_dir,
            Vec::new(),
            recent,
            Some(dir_abs),
        );
        // Default to the first real folder so Enter DESCENDS into it right away; the
        // synthetic "." (select-this-folder) sits above it, Up.
        s.selected = s.items.iter().position(|&i| s.corpus[i] != ".").unwrap_or(0);
        s.scroll_to_selected();
        s
    }

    /// Build the COMMAND PALETTE: the corpus is the command NAMES (in
    /// `commands::visible()` order — the platform-filtered view, so a row index maps
    /// back to that filtered corpus, NOT the raw `commands::COMMANDS` catalog; see
    /// `commands.rs`'s "PLATFORM-SCOPED COMMANDS" section) and `bindings` carries each
    /// command's current chord label, shown dim beside the name. Fuzzy-filterable like
    /// the other pickers.
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

    /// Build the REBIND MENU: the corpus is the command NAMES (in `commands::visible()`
    /// order — same platform-filtered view as the palette, so a row index maps back to
    /// it, not the raw catalog) and `bindings` carries each command's EFFECTIVE
    /// chords, shown beside the name. Identical corpus/bindings shape to the palette,
    /// but `kind = Keybindings`, so Enter starts a CAPTURE rather than running the
    /// command.
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

    /// REBIND MENU: the slug of the highlighted command (for Delete → reset-to-default),
    /// or `None` when no row matches.
    pub fn selected_command_slug(&self) -> Option<String> {
        self.selected_corpus_index().map(crate::commands::visible_slug_of)
    }

    /// The line drawn DIM at the FOOT of the card. Normally the per-kind control
    /// hint; for the Keybindings menu an active capture's PROMPT (press a key…) wins,
    /// else a transient NOTICE (saved / reset / conflict), so the rebind flow reads on
    /// the card itself. Other kinds always show `kind.hint()`.
    pub fn foot_hint(&self) -> String {
        if let Some(re) = &self.rename_edit {
            return re.prompt();
        }
        if let Some(le) = &self.link_edit {
            return le.prompt();
        }
        if let Some(cap) = &self.capture {
            return cap.prompt();
        }
        if !self.notice.is_empty() {
            return self.notice.clone();
        }
        self.kind.hint()
    }

    /// Attach the current markdown document's HEADINGS to a Go-to overlay — the fold
    /// that RETIRED the standalone Outline picker. Each `(display, line)` heading is
    /// APPENDED after the file rows (display = the title indented by depth, the fuzzy
    /// corpus; `line` = where Enter jumps), carrying its `heading` flag + jump line in
    /// the parallel arrays. The file rows stay FIRST, so the flat All home + the file
    /// lenses (Recent / This folder / By type) still list files only —
    /// [`Self::refilter`]'s heading gate hides these appended rows everywhere EXCEPT
    /// the Headings lens, where [`crate::index::goto_bucket`] re-admits them. An EMPTY
    /// `headings` list is a clean no-op (the `heading` flag stays empty → the gate is
    /// inert → the Headings lens reads "no headings yet"); a non-markdown buffer never
    /// calls this at all.
    pub fn attach_headings(&mut self, headings: Vec<(String, usize)>) {
        if headings.is_empty() {
            return;
        }
        let n = self.corpus.len();
        // Pad the two heading-parallel arrays over the existing FILE rows first
        // (files: not a heading, unused line 0), then append one row per heading.
        self.heading = vec![false; n];
        self.lines = vec![0; n];
        for (display, line) in headings {
            self.corpus.push(display);
            self.git.push(false);
            self.is_dir.push(false);
            self.heading.push(true);
            self.lines.push(line);
        }
        self.refilter();
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
    /// Enter accept value). Flat + transient like the other pickers — it vanishes on
    /// restore / cancel.
    ///
    /// An EMPTY `rows` builds an empty-corpus picker: it still summons (History always
    /// opens, unlike Outline's no-op-on-empty), and the SHARED empty-state path draws
    /// the calm "no history yet" message row ([`OverlayKind::empty_corpus_message`]),
    /// with Enter a no-op — the one empty-state owner every picker now shares, in
    /// place of History's former bespoke synthetic corpus row.
    ///
    /// `now` / `session_start` are the REFERENCE clocks the Session / Today lenses
    /// bucket against — `Some` live, `None` in the headless capture path (which makes
    /// those two lenses inert, the determinism gate). Each row's own stamp rides
    /// [`crate::history::TimelineRow::timestamp`] into the parallel `facet_ts`.
    pub fn new_history(
        rows: Vec<crate::history::TimelineRow>,
        now: Option<u64>,
        session_start: Option<u64>,
    ) -> Self {
        let n = rows.len();
        let mut corpus = Vec::with_capacity(n);
        let mut diffs = Vec::with_capacity(n);
        let mut ids = Vec::with_capacity(n);
        let mut ts = Vec::with_capacity(n);
        for row in rows {
            corpus.push(if row.which.is_empty() {
                row.when
            } else {
                format!("{} · {}", row.when, row.which)
            });
            // THE CONSCIOUS MARK: a KEPT (pinned) version wears a calm, dim
            // "pinned" tag in the faint secondary column, ahead of its changed-count
            // (`"pinned · +N −M"`) — no amber, no new column; it rides the existing
            // `bindings` right-column the diff-count already uses, so a pin is
            // findable at a glance AND assertable from the sidecar's `overlay.bindings`.
            diffs.push(if row.pinned {
                format!("{PIN_TAG} · {}", row.counts)
            } else {
                row.counts
            });
            ids.push(row.id);
            ts.push(row.timestamp);
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
        s.facet_ts = ts;
        s.facet_now = now;
        s.facet_session_start = session_start;
        s
    }

    /// Build the ASSET CLEANER picker from the caller-scanned [`crate::assets::Orphan`]
    /// list. The corpus is each orphan's root-relative PATH (the accept/trash key +
    /// the fuzzy corpus — typing a folder narrows), the primary cell shows the leaf
    /// name ([`Self::display_of`]), and the `bindings` secondary column carries the
    /// human size + parent dir ([`crate::assets::secondary_label`]). ALWAYS summons
    /// (like History): an empty list shows the calm "no unused assets" row rather than
    /// silently no-op'ing.
    pub fn new_assets(orphans: Vec<crate::assets::Orphan>) -> Self {
        let n = orphans.len();
        let mut corpus = Vec::with_capacity(n);
        let mut secondary = Vec::with_capacity(n);
        for o in &orphans {
            secondary.push(crate::assets::secondary_label(o));
            corpus.push(o.rel.clone());
        }
        let mut s = Self::new_marked(
            OverlayKind::Assets,
            corpus,
            vec![false; n],
            vec![false; n],
            Vec::new(),
            Vec::new(),
            None,
        );
        // The faint right column shows each orphan's size + parent dir. Reuse the
        // `bindings` column (like History's diff counts) so it rides the shared
        // rowlayout secondary + surfaces to the sidecar.
        s.bindings = secondary;
        s
    }

    /// ASSET CLEANER: remove the row whose corpus entry equals `rel` (the file the App
    /// just trashed), keeping the picker open. Removes the entry from `corpus` + every
    /// parallel column, re-ranks the remaining rows, and clamps the selection —
    /// removing by VALUE (not index) so it stays correct regardless of the current
    /// query/selection. Returns whether a row was removed. The App calls this ONLY
    /// after a SUCCESSFUL trash, so the list never claims a file is gone that wasn't
    /// (the determinism gate: the pure core never removes a row — a headless replay's
    /// `Effect::TrashAsset` is a no-op, so its list stays whole).
    pub fn remove_asset_row(&mut self, rel: &str) -> bool {
        let Some(ci) = self.corpus.iter().position(|c| c == rel) else {
            return false;
        };
        self.corpus.remove(ci);
        // Keep every corpus-parallel column in lockstep (assets fills only `bindings`;
        // the rest are all empty/false, but drain uniformly so the method can't drift).
        for col in [&mut self.git, &mut self.is_dir] {
            if ci < col.len() {
                col.remove(ci);
            }
        }
        if ci < self.bindings.len() {
            self.bindings.remove(ci);
        }
        if ci < self.times.len() {
            self.times.remove(ci);
        }
        // Rebuild `items` (indices shifted) + clamp `selected` against the shorter list.
        self.refilter();
        if self.selected >= self.items.len() {
            self.selected = self.items.len().saturating_sub(1);
        }
        true
    }
}
