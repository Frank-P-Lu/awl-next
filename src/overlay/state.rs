//! `OverlayState` -- the live overlay data model -- plus its per-kind
//! CONSTRUCTORS (`new`/`new_theme`/`new_caret`/`new_project`/…). Split out of
//! the former `overlay.rs` monolith (2026-07 code-organization pass); every
//! item's path is unchanged (`overlay::OverlayState`) -- only the file it
//! lives in moved.

use super::{Capture, KeepEdit, LinkEdit, OverlayKind, RenameEdit, ValueEdit, PIN_TAG};
use crate::textbox::TextBox;

/// The row LABEL for the spell picker's "Add to dictionary" affordance — the ONE
/// owner of its wording, so the built row, the tests, and any future re-summon
/// agree. Matter-of-fact + the word in single quotes (docs voice).
pub fn add_to_dictionary_label(word: &str) -> String {
    format!("Add '{word}' to dictionary")
}

/// ITEM 54 — ONE typed row, replacing the former TWELVE corpus-parallel arrays
/// (`corpus`/`git`/`is_dir`/`bindings`/`times`/`lines`/`heading`/`spell_add`/
/// `history_ids`/`facet_ts`/`is_setting`/`hidden`). Every metadata read routes
/// through a row's own field or its [`RowMeta`] — never a separate array by
/// index — so a reorder or removal carries a row's identity as ONE element
/// (structurally impossible to separate a row from its own metadata, the
/// headline bug class the parallel-array shape invited).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayRow {
    /// The RAW accept value (also the fuzzy corpus text): a root-relative path,
    /// a child/entry name, a world/look/variant/format label, a command name, a
    /// history row's main column, …
    pub accept: String,
    /// The SECONDARY (right) column: a chord label, a look/variant description,
    /// a changed-count, a size + parent dir, a setting's current value, … Empty
    /// when this kind draws no secondary column for this row.
    pub secondary: String,
    /// This entry is a directory (Browse: Enter descends).
    pub is_dir: bool,
    /// This entry is a git repo (gets a marker).
    pub git: bool,
    /// The row's KIND-SPECIFIC payload — see [`RowMeta`]. A row carries EXACTLY
    /// one variant; which variants a given [`OverlayKind`] may produce is the
    /// CLOSED roster [`OverlayKind::row_meta_roster`] declares.
    pub meta: RowMeta,
}

impl OverlayRow {
    /// A PLAIN row carrying just its accept text — every optional field at its
    /// off default (no secondary, no dir/git marker, [`RowMeta::Plain`]). The
    /// common shape most pickers' rows start from.
    fn plain(accept: String) -> Self {
        Self { accept, secondary: String::new(), is_dir: false, git: false, meta: RowMeta::Plain }
    }
}

/// A row's KIND-SPECIFIC metadata — CLOSED: a row carries EXACTLY one variant,
/// never a combination (mutual exclusivity holds per row: a Command row is one
/// of `Plain`/`CommandHidden`/`CommandSetting`; a Spell row is `Plain`/
/// `SpellAdd`; a Goto row is `GotoFile`/`GotoHeading`). Replaces the former
/// `heading`+`lines` / `spell_add` / `history_ids`+`facet_ts` / `is_setting` /
/// `hidden` parallel arrays (item 54). Plain data — `Clone`+`PartialEq`+`Eq`,
/// no rope/undo/syntax machinery — so a row is trivially testable and
/// serialisable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowMeta {
    /// No kind-specific payload — the vast majority of rows, every kind.
    Plain,
    /// Go-to FILE row: the relative "last edited" label (live only; empty in
    /// the headless capture path, for determinism). Was `times[i]`.
    GotoFile { time: String },
    /// Go-to appended document-HEADING row (the fold that retired the
    /// standalone Outline picker): the document LINE Enter jumps to. Was
    /// `heading[i]` (the flag) + `lines[i]` (the line).
    GotoHeading { line: usize },
    /// Command palette appended SETTINGS row (the union round): the setting's
    /// current value rides the row's own `secondary`. Was `is_setting[i]`.
    CommandSetting,
    /// Command palette RUNTIME-gated row: exists in the catalog but is hidden
    /// from selection right now (see `commands::visible_hidden_mask`). Was
    /// `hidden[i]`.
    CommandHidden,
    /// Spell picker's always-terminal "Add '<word>' to dictionary" row. Was
    /// `spell_add[i]`.
    SpellAdd,
    /// History timeline row: the RESTORE id (Enter's accept value) + the
    /// version's wall-clock stamp (millis, for the Session/Today lenses). Was
    /// `history_ids[i]` + `facet_ts[i]`.
    History { id: String, ts: u64 },
}

/// [`RowMeta`]'s discriminant — the ROSTER comparison key
/// [`OverlayKind::row_meta_roster`] declares against, and what
/// [`RowMeta::tag`] maps every variant to. Kept separate from `RowMeta` itself
/// (rather than matching on `RowMeta` directly in the roster) so a roster is
/// plain data (`&'static [RowMetaTag]`), comparable with `contains`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // consumed only by OverlayKind::row_meta_roster + overlay::tests today.
pub enum RowMetaTag {
    Plain,
    GotoFile,
    GotoHeading,
    CommandSetting,
    CommandHidden,
    SpellAdd,
    History,
}

impl RowMeta {
    /// This row's meta TAG. A NO-WILDCARD match — THE EXHAUSTIVENESS WITNESS: a
    /// future `RowMeta` variant fails to compile here (and therefore fails
    /// every kind's `row_meta_roster` too) until this mapping accounts for it.
    #[allow(dead_code)] // consumed only by overlay::tests's exhaustiveness witness + roster sweep today.
    pub fn tag(&self) -> RowMetaTag {
        match self {
            RowMeta::Plain => RowMetaTag::Plain,
            RowMeta::GotoFile { .. } => RowMetaTag::GotoFile,
            RowMeta::GotoHeading { .. } => RowMetaTag::GotoHeading,
            RowMeta::CommandSetting => RowMetaTag::CommandSetting,
            RowMeta::CommandHidden => RowMetaTag::CommandHidden,
            RowMeta::SpellAdd => RowMetaTag::SpellAdd,
            RowMeta::History { .. } => RowMetaTag::History,
        }
    }
}

/// Live overlay state. `rows` is the full candidate list (stable order) — each
/// [`OverlayRow`] carries its own accept text, secondary column, dir/git
/// markers, and kind-specific [`RowMeta`]; `items` is the fuzzy-filtered +
/// ranked VIEW of it (each entry an index into `rows`) the panel shows.
/// `selected` indexes into `items`. `open`/`recent` mark which row indices get
/// a ranking bias (open buffer > recently opened > corpus).
///
/// `browse_dir` is only meaningful for `Browse`: the root-relative directory the
/// current level lists (`None` = the root itself). It is surfaced to the sidecar
/// so a `--keys` descend is verifiable.
#[derive(Debug, Clone)]
pub struct OverlayState {
    pub kind: OverlayKind,
    /// ITEM 45 → ITEM 52 (overlay ALIGNMENT as personality data) — the horizontal
    /// alignment the card draws at, captured from [`crate::render::effective_card_anchor`]
    /// (the ONE resolver: the `AWL_OVERLAY_ALIGN` capture knob, else the active world's
    /// own `render_caps.card_anchor`) at summon and RE-STAMPED on a deliberate crossing.
    ///
    /// ITEM 45 froze it at summon so a theme-preview crossing never moved the open card.
    /// ITEM 52 SUPERSEDES that for a DELIBERATE selection movement: choosing a world drops
    /// you INSIDE it, so the theme picker's card SNAPS into the destination world's own
    /// rail. [`Self::reanchor`] re-reads [`crate::render::effective_card_anchor`] after a
    /// keyboard nav / wheel / page/jump move applies the highlighted world — but PASSIVE
    /// pointer hover leaves it untouched (no spatial chase; the freeze still holds an open
    /// card put under a hovering pointer). Threaded to the render path through
    /// `ViewState::overlay_align` → `resolve_overlay_anchor`, which the render CONSUMERS
    /// read instead of the live world — so an unfrozen live read still can't relocate the
    /// card (the alignment-is-data grep-law), only a deliberate `reanchor` can.
    pub align: crate::theme::CardAnchor,
    /// ITEM 10 — the fuzzy filter text + its CHAR-index caret, one shared
    /// [`TextBox`]. Plain L/R still lens/descend/list (never routed through
    /// the model); only word-motion (Ctrl/Opt-arrow) and typing/backspace
    /// move the caret within it — see `actions/overlay_nav.rs`.
    pub query: TextBox,
    /// ITEM 54 — the full unfiltered candidate ROWS (stable order): each carries
    /// its own accept text, secondary column, dir/git markers, and kind-specific
    /// [`RowMeta`]. Replaces the former `corpus`/`git`/`is_dir`/`bindings`/
    /// `times`/`lines`/`heading`/`spell_add`/`history_ids`/`facet_ts`/
    /// `is_setting`/`hidden` parallel arrays.
    pub rows: Vec<OverlayRow>,
    /// Row indices that are currently OPEN (active file).
    pub open: Vec<usize>,
    /// Row indices that were recently opened (MRU), not currently open.
    pub recent: Vec<usize>,
    /// Filtered + ranked view: each entry is an index into `rows`.
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
    /// Spell picker only: the misspelled word's `(line, start_col, end_col)` CHAR
    /// span, so the accept can map it to a buffer char range and replace it with the
    /// chosen suggestion. `None` for every other kind.
    pub spell_target: Option<(usize, usize, usize)>,
    /// Spell picker only: the misspelled WORD this picker would add to the personal
    /// dictionary — the payload of the "Add to dictionary" row's accept effect, and
    /// the text its row label echoes. `None` for every other kind (and a spell
    /// picker built without a word, which then offers no add row).
    pub add_word: Option<String>,
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
    /// NAMED SAVE POINTS: the "Keep version…" minibuffer's live typed-name
    /// sub-state (`Some` only for `OverlayKind::KeepName`, armed the instant the
    /// overlay is built by [`Self::new_keep_name`] — mirrors `rename_edit`/
    /// `link_edit`'s shape exactly). `None` for every other kind.
    pub keep_edit: Option<KeepEdit>,
    /// DIFF-AS-PREVIEW (History only): whether the keyboard FOCUS sits in the DIFF
    /// PANEL below the picker card (Tab shifts it there; Tab/Esc return it to the
    /// version list). While `true`, ↑/↓ scroll the diff step-wise instead of moving
    /// the version selection, and the panel's card border strengthens one value
    /// step (the calm focus cue). Always `false` for every other kind.
    pub diff_focus: bool,
    /// DIFF-AS-PREVIEW (History only): the diff panel's scroll, in VISUAL ROWS off
    /// the transcript's top — mutated by PgUp/PgDn (both focus states), by ↑/↓
    /// under panel focus, and by the wheel over the page; clamped against the
    /// shaped transcript at ViewState-build (the core can't measure). RESET to 0
    /// whenever the highlighted version changes (a new transcript starts at its
    /// top) — see the reset lines in `move_sel`/`hover_select`/`set_facet_lens`/
    /// `refilter`. Inert (always 0) for every other kind.
    pub diff_scroll: usize,
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
        // ITEM 54: build the typed rows straight from the (corpus, git, is_dir)
        // triple every caller already gathers — GOTO's file rows are ALWAYS
        // `GotoFile` (never bare `Plain`; see `OverlayKind::row_meta_roster`),
        // with an empty `time` until a live [`Self::set_times`] fills it in
        // (headless capture never calls it, so it stays "" there — the
        // determinism gate). Every other kind's rows start `Plain`.
        let rows: Vec<OverlayRow> = corpus
            .into_iter()
            .zip(git)
            .zip(is_dir)
            .map(|((accept, git), is_dir)| {
                let mut row = OverlayRow::plain(accept);
                row.git = git;
                row.is_dir = is_dir;
                if kind == OverlayKind::Goto {
                    row.meta = RowMeta::GotoFile { time: String::new() };
                }
                row
            })
            .collect();
        let mut s = Self {
            kind,
            // ITEM 45: freeze the alignment ONCE, here at summon — every constructor
            // funnels through `new_marked`, so every overlay kind captures it in one
            // place. `effective_card_anchor` honors the `AWL_OVERLAY_ALIGN` capture
            // knob then the active world's data; the render path reads THIS frozen
            // value thereafter, never the live world (so previewing holds the card).
            align: crate::render::effective_card_anchor(),
            query: TextBox::new(),
            rows,
            open,
            recent,
            items: Vec::new(),
            selected: 0,
            scroll: 0,
            browse_dir,
            original_theme: None,
            original_caret: None,
            original_caret_was_auto: false,
            spell_target: None,
            add_word: None,
            capture: None,
            notice: String::new(),
            // Default to the "All" home (strip index 0 = the flat list). A faceting
            // picker LANDS here; ←/→ step into the refinement lenses. Non-faceting
            // kinds ignore it (no scheme).
            facet_lens: 0,
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
            // No keep-name edit on a fresh summon; `new_keep_name` arms it right after.
            keep_edit: None,
            // DIFF-AS-PREVIEW: focus opens on the version LIST, diff at its top.
            diff_focus: false,
            diff_scroll: 0,
        };
        s.refilter();
        s
    }

    /// The RAW accept strings, in row (corpus) order — the FUZZY-MATCH candidate
    /// list [`Self::refilter`] ranks against, and a plain-text convenience for
    /// callers that just want "what's in the corpus" without touching a row's
    /// other fields.
    pub fn accepts(&self) -> Vec<&str> {
        self.rows.iter().map(|r| r.accept.as_str()).collect()
    }

    /// Set every ROW's SECONDARY column, positionally — the bulk-fill several
    /// constructors (each look/variant's description, each command's chord,
    /// each setting's current value, …) use in place of the old flat
    /// `bindings = …` assignment. Rows past `secondaries`' length (or when
    /// `secondaries` is shorter) simply keep their existing secondary.
    pub fn set_secondaries(&mut self, secondaries: Vec<String>) {
        for (row, s) in self.rows.iter_mut().zip(secondaries) {
            row.secondary = s;
        }
    }

    /// Attach the relative "last edited" labels (parallel to the corpus rows)
    /// for the go-to picker — each row's [`RowMeta::GotoFile`] time. Set by the
    /// LIVE app only; the headless path leaves every row's time "" (already the
    /// default from [`new_marked`]), keeping the capture byte-stable.
    pub fn set_times(&mut self, times: Vec<String>) {
        for (i, row) in self.rows.iter_mut().enumerate() {
            row.meta = RowMeta::GotoFile { time: times.get(i).cloned().unwrap_or_default() };
        }
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
    /// secondary column carrying that look's one-line description (drawn dim beside
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
        s.set_secondaries(descriptions);
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
    /// [`crate::spell::DictVariant::ALL`] order), each row's secondary column
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
        s.set_secondaries(descriptions);
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
    /// Dictionary's fixed `ALL` order), each row's secondary column carrying its
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
        s.set_secondaries(descriptions);
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

    /// Build the DATE-FORMAT picker: the corpus is the five formats' EXAMPLE
    /// DATES — each format rendered with `today` ([`crate::dateformat::DateFormat::
    /// format`]), so the PRIMARY column shows exactly what an Insert-Date would
    /// type (what-you-see-is-what-inserts) — with the format's human NAME
    /// ([`crate::dateformat::DateFormat::label`]) as the dim secondary column, the
    /// SAME shape as [`new_dictionary`](Self::new_dictionary) /
    /// [`new_cjk_lang`](Self::new_cjk_lang) (no live-preview/revert bookkeeping —
    /// nothing in the document previews on move; the example dates ARE the
    /// preview). `today` is the caller's `today_ymd` (live clock, or the fixed
    /// [`crate::dateformat::CAPTURE_PLACEHOLDER_YMD`] in a headless capture), so a
    /// capture stays deterministic. `active` pre-selects the current format. The
    /// corpus order == [`crate::dateformat::DateFormat::ALL`] order, so the
    /// accept path maps the selected corpus index straight back to the format.
    pub fn new_date(active: crate::dateformat::DateFormat, today: (i32, u32, u32)) -> Self {
        let (y, m, d) = today;
        let names: Vec<String> = crate::dateformat::DateFormat::ALL
            .iter()
            .map(|f| f.format(y, m, d))
            .collect();
        let descriptions: Vec<String> = crate::dateformat::DateFormat::ALL
            .iter()
            .map(|f| f.label().to_string())
            .collect();
        let n = names.len();
        let mut s = Self::new_marked(
            OverlayKind::Date,
            names,
            vec![false; n],
            vec![false; n],
            Vec::new(),
            Vec::new(),
            None,
        );
        s.set_secondaries(descriptions);
        if let Some(active_index) =
            crate::dateformat::DateFormat::ALL.iter().position(|&f| f == active)
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
        s.selected = s.items.iter().position(|&i| s.rows[i].accept != ".").unwrap_or(0);
        s.scroll_to_selected();
        s
    }

    /// Build the COMMAND PALETTE: the corpus is the command NAMES (in
    /// `commands::visible()` order — the platform-filtered view, so a row index maps
    /// back to that filtered corpus, NOT the raw `commands::COMMANDS` catalog; see
    /// `commands.rs`'s "PLATFORM-SCOPED COMMANDS" section) and each row's secondary
    /// column carries that command's current chord label, shown dim beside the name.
    /// Fuzzy-filterable like the other pickers. `hidden` is the RUNTIME-gated mask
    /// parallel to `names` (`commands::visible_hidden_mask`, e.g. "Finish file" with
    /// no daemon `--wait` client waiting) — applied (as [`RowMeta::CommandHidden`])
    /// BEFORE the final `refilter` so a hidden row never appears even in the
    /// unqueried, freshly-opened list.
    pub fn new_command(names: Vec<String>, bindings: Vec<String>, hidden: Vec<bool>) -> Self {
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
        s.set_secondaries(bindings);
        for (row, h) in s.rows.iter_mut().zip(hidden) {
            if h {
                row.meta = RowMeta::CommandHidden;
            }
        }
        s.refilter();
        s
    }

    /// Build the REBIND MENU: the corpus is the command NAMES (in `commands::visible()`
    /// order — same platform-filtered view as the palette, so a row index maps back to
    /// it, not the raw catalog) and each row's secondary column carries that command's
    /// EFFECTIVE chords, shown beside the name. Identical corpus/bindings shape to the
    /// palette, but `kind = Keybindings`, so Enter starts a CAPTURE rather than running
    /// the command.
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
        s.set_secondaries(bindings);
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
        if let Some(ke) = &self.keep_edit {
            return ke.prompt();
        }
        if let Some(cap) = &self.capture {
            return cap.prompt();
        }
        if !self.notice.is_empty() {
            return self.notice.clone();
        }
        // DIFF-AS-PREVIEW panel focus (History, Tab pressed): the foot line teaches
        // the PANEL's keys instead of the list's. Deliberately WITHOUT the universal
        // "type to filter" lead — typing is swallowed while the panel holds focus,
        // so advertising it here would lie. Esc (back to the list) goes unadvertised
        // per the plain-close precedent; ↵ still restores, so it keeps its cell.
        if self.diff_focus {
            return super::format_hint(&[
                super::HintAction { glyph: "\u{2191}/\u{2193}", label: "scroll" },
                super::HintAction { glyph: "\u{21B5}", label: "restore" },
                super::HintAction { glyph: "tab", label: "back" },
            ]);
        }
        self.kind.hint()
    }

    /// Attach the current markdown document's HEADINGS to a Go-to overlay — the fold
    /// that RETIRED the standalone Outline picker. Each `(display, line)` heading is
    /// APPENDED after the file rows (display = the title indented by depth, the fuzzy
    /// corpus; `line` = where Enter jumps) as a fresh [`RowMeta::GotoHeading`] row —
    /// the file rows stay FIRST (their original corpus order tiebreaks an equal fuzzy
    /// score) and already carry their own [`RowMeta::GotoFile`] from construction, so
    /// nothing about them needs touching here. Item 11's UNIFIED DEFAULT means the
    /// flat `All` home lists them together with the appended heading rows, ranked by
    /// one fuzzy filter — [`Self::refilter`]'s heading gate only drops heading rows
    /// under a file-only REFINEMENT lens (Recent / This folder); the Headings lens
    /// keeps its old job of showing ONLY them, via [`crate::index::goto_bucket`]. An
    /// EMPTY `headings` list is a clean no-op (no `GotoHeading` row ever appears →
    /// every gate keyed off it is inert → the Headings lens reads "no headings yet",
    /// and `All` simply lists the files); a non-markdown buffer never calls this at
    /// all.
    pub fn attach_headings(&mut self, headings: Vec<(String, usize)>) {
        if headings.is_empty() {
            return;
        }
        for (display, line) in headings {
            self.rows.push(OverlayRow {
                accept: display,
                secondary: String::new(),
                is_dir: false,
                git: false,
                meta: RowMeta::GotoHeading { line },
            });
        }
        self.refilter();
    }

    /// THE UNION ROUND: attach the SETTINGS corpus to a freshly-built Command
    /// palette overlay — appended AFTER the command rows (mirrors
    /// [`Self::attach_headings`]'s file-rows-first convention), so the flat `All`
    /// facet lens intermixes commands + settings by fuzzy rank while the File/Edit/
    /// View/Recent lenses (which bucket by `menu_section`/`recent`, neither of which
    /// any setting name matches) naturally exclude them — no bucket code needed.
    /// `names`/`values` are [`crate::settings::visible_names`]/
    /// [`crate::settings::visible_value_cells`] (platform-filtered, parallel), the
    /// SAME corpus the Settings menu itself opens with, so a setting reached via the
    /// palette shows the identical current-value secondary cell — riding each new
    /// row's own `secondary` (a command shows its chord there, a setting its value;
    /// never both on the same row). Each appended row carries
    /// [`RowMeta::CommandSetting`], read by [`Self::selected_setting_row`] (the
    /// accept dispatch) and [`Self::display_of`] (the marker glyph). A no-op when
    /// `names` is empty.
    pub fn attach_settings_rows(&mut self, names: Vec<String>, values: Vec<String>) {
        if names.is_empty() {
            return;
        }
        for (name, value) in names.into_iter().zip(values) {
            self.rows.push(OverlayRow {
                accept: name,
                secondary: value,
                is_dir: false,
                git: false,
                meta: RowMeta::CommandSetting,
            });
        }
        self.refilter();
    }

    /// THE UNION ROUND: the highlighted row's [`crate::settings::SettingRow`], when
    /// the Command palette's selection is one of the APPENDED settings rows
    /// ([`Self::attach_settings_rows`]) — `None` for an ordinary command row, every
    /// other kind, and a settings row somehow absent from the live
    /// [`crate::settings::visible_rows`] (never happens in practice; the row's accept
    /// text IS a `visible_rows` name by construction). Looked up by NAME rather than
    /// a tracked offset, so it can never mis-map regardless of how many commands
    /// precede the settings block.
    pub fn selected_setting_row(&self) -> Option<crate::settings::SettingRow> {
        let ci = self.selected_corpus_index()?;
        let row = self.rows.get(ci)?;
        if !matches!(row.meta, RowMeta::CommandSetting) {
            return None;
        }
        crate::settings::visible_rows()
            .into_iter()
            .find(|r| r.name == row.accept)
            .copied()
    }

    /// Build the SPELL-SUGGESTION picker (Cmd-`;`): a compact CONTEXT MENU (item
    /// 64). The corpus is the dictionary's ordered corrections for `word` (the
    /// misspelled text at `target`'s span), TRUNCATED to the top
    /// [`OverlayKind::MAX_SUGGESTIONS`] (ranking/order preserved — a longer list
    /// simply has its tail dropped, never scrolled or elided-with-a-button), plus
    /// ONE appended "Add '<word>' to dictionary" row — the same surface, no new
    /// chrome class. The add row is flagged [`RowMeta::SpellAdd`] (always the LAST
    /// row — [`crate::overlay::nav`]'s refilter keeps it terminal even under a
    /// query that out-ranks it) and carries `word` in `add_word` so the accept can
    /// emit [`crate::actions::Effect::AddToDictionary`]; it is present even when
    /// the suggestion list is EMPTY (so a word with no correction can still be
    /// added) — the picker always shows at least the one add row.
    pub fn new_spell(mut suggestions: Vec<String>, target: (usize, usize, usize), word: String) -> Self {
        suggestions.truncate(OverlayKind::MAX_SUGGESTIONS);
        suggestions.push(add_to_dictionary_label(&word));
        let len = suggestions.len();
        let mut s = Self::new_marked(
            OverlayKind::Spell,
            suggestions,
            vec![false; len],
            vec![false; len],
            Vec::new(),
            Vec::new(),
            None,
        );
        if let Some(last) = s.rows.last_mut() {
            last.meta = RowMeta::SpellAdd;
        }
        s.spell_target = Some(target);
        s.add_word = Some(word);
        s
    }

    /// Build the SUMMONED HISTORY TIMELINE: `rows` is the file's versions
    /// NEWEST-FIRST ([`crate::history::TimelineRow`]). Each row's MAIN column
    /// composes WHEN + WHICH — `"{when} · {which}"`, or the bare `when` for an
    /// empty `which` — so the body-ink cell answers both questions at a glance
    /// (and the fuzzy filter matches commit subjects / edit descriptions for
    /// free); the faint `"+N −M"` changed-count rides the row's own SECONDARY
    /// column (LABEL size, faint ink — the picker desc-column pattern, zero new
    /// layout); the opaque restore id + wall-clock stamp ride the row's own
    /// [`RowMeta::History`] (the Enter accept value + the faceting clock). Flat +
    /// transient like the other pickers — it vanishes on restore / cancel.
    ///
    /// An EMPTY `rows` builds an empty-corpus picker: it still summons (History always
    /// opens, unlike Outline's no-op-on-empty), and the SHARED empty-state path draws
    /// the calm "no history yet" message row ([`OverlayKind::empty_corpus_message`]),
    /// with Enter a no-op — the one empty-state owner every picker now shares, in
    /// place of History's former bespoke synthetic corpus row.
    ///
    /// `now` / `session_start` are the REFERENCE clocks the Session / Today lenses
    /// bucket against — `Some` live, `None` in the headless capture path (which makes
    /// those two lenses inert, the determinism gate).
    pub fn new_history(
        rows: Vec<crate::history::TimelineRow>,
        now: Option<u64>,
        session_start: Option<u64>,
    ) -> Self {
        let n = rows.len();
        let mut corpus = Vec::with_capacity(n);
        let mut secondaries = Vec::with_capacity(n);
        let mut ids = Vec::with_capacity(n);
        let mut ts = Vec::with_capacity(n);
        for row in rows {
            // NAMED SAVE POINT: a NAMED version's NAME is the PRIMARY cell (the
            // user's own word for the direction, full body ink — and the fuzzy
            // corpus, so typing the name finds it), with the timestamp DEMOTED to
            // the faint secondary column beside the changed-count (`"when ·
            // +N −M"`). Calm, ink/value distinction only — never amber; no new
            // layout path (the same secondary column every row rides). The
            // redundant "pinned" tag is dropped for a named row — the name IS the
            // conscious mark. Unnamed rows are byte-identical to before.
            if let Some(name) = row.name {
                corpus.push(name);
                secondaries.push(format!("{} · {}", row.when, row.counts));
            } else {
                corpus.push(if row.which.is_empty() {
                    row.when
                } else {
                    format!("{} · {}", row.when, row.which)
                });
                // THE CONSCIOUS MARK: a KEPT (pinned) version wears a calm, dim
                // "pinned" tag in the faint secondary column, ahead of its changed-count
                // (`"pinned · +N −M"`) — no amber, no new column; it rides the same
                // secondary column the diff-count already uses, so a pin is
                // findable at a glance AND assertable from the sidecar's `overlay.bindings`.
                secondaries.push(if row.pinned {
                    format!("{PIN_TAG} · {}", row.counts)
                } else {
                    row.counts
                });
            }
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
        s.set_secondaries(secondaries); // the faint right column shows each version's changed-count
        for (row, (id, ts)) in s.rows.iter_mut().zip(ids.into_iter().zip(ts)) {
            row.meta = RowMeta::History { id, ts };
        }
        s.facet_now = now;
        s.facet_session_start = session_start;
        s
    }

    /// Build the ASSET CLEANER picker from the caller-scanned [`crate::assets::Orphan`]
    /// list. The corpus is each orphan's root-relative PATH (the accept/trash key +
    /// the fuzzy corpus — typing a folder narrows), the primary cell shows the leaf
    /// name ([`Self::display_of`]), and the row's own secondary column carries the
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
        // The faint right column shows each orphan's size + parent dir. Rides the
        // shared rowlayout secondary + surfaces to the sidecar (like History's
        // diff counts).
        s.set_secondaries(secondary);
        s
    }

    /// ASSET CLEANER: remove the row whose accept equals `rel` (the file the App
    /// just trashed), keeping the picker open. Removes it from `rows` by VALUE (not
    /// index, so it stays correct regardless of the current query/selection),
    /// re-ranks the remaining rows, and clamps the selection. Returns whether a row
    /// was removed. The App calls this ONLY after a SUCCESSFUL trash, so the list
    /// never claims a file is gone that wasn't (the determinism gate: the pure core
    /// never removes a row — a headless replay's `Effect::TrashAsset` is a no-op, so
    /// its list stays whole).
    pub fn remove_asset_row(&mut self, rel: &str) -> bool {
        let Some(ci) = self.rows.iter().position(|r| r.accept == rel) else {
            return false;
        };
        self.rows.remove(ci);
        // Rebuild `items` (indices shifted) + clamp `selected` against the shorter list.
        self.refilter();
        if self.selected >= self.items.len() {
            self.selected = self.items.len().saturating_sub(1);
        }
        true
    }
}
