//! VIEWSTATE — the render-relevant editor SNAPSHOT the pipeline draws.
//!
//! Pure data: both the windowed app and the headless capture build one
//! ([`ViewState`]) and hand it to [`super::TextPipeline`]. Carved out of
//! `render.rs` VERBATIM — the struct definition plus its canonical
//! [`ViewState::base`] default (the ONE place a new field is defaulted, so
//! every bench / perf / frame / capture / test scaffold inherits it). Because
//! the type is `&self`-free and carries no GPU state it lifts cleanly; it is
//! re-exported from `render` so `crate::render::ViewState` resolves unchanged
//! for every caller. The live App's EXHAUSTIVE `sync_view` deliberately stays
//! in `app/viewstate.rs` and does NOT route through `base()`.

use super::*;

/// The render-relevant snapshot of the editor. Pure data so both the windowed
/// app and the headless capture can build one and hand it to the pipeline.
pub struct ViewState {
    /// Full buffer text.
    pub text: String,
    /// Cursor line (0-based) and column (0-based, in chars).
    pub cursor_line: usize,
    pub cursor_col: usize,
    /// The caret's wrap AFFINITY (see [`crate::caret::Affinity`]): which visual row
    /// the caret RENDERS on when `cursor_col` lands exactly on a shared soft-wrap
    /// boundary. `Upstream` (set by a visual line-END motion) renders on the UPPER
    /// row's trailing edge; `Downstream` (the default) on the lower row's leading
    /// edge. Read ONLY by the caret's own placement (`caret_affinity`), so every
    /// other overlay is unaffected.
    pub caret_affinity: crate::caret::Affinity,
    /// Number of VISUAL ROWS scrolled off the top. Each visual row is one
    /// `line_height`-tall soft-wrapped sub-line, so on a wrapped document this is
    /// NOT the same as a logical-line count: it advances by what's actually drawn,
    /// letting the last wrapped row reach the bottom. For a non-wrapped document
    /// visual rows == logical lines, so this is unchanged from the old meaning.
    pub scroll_lines: usize,
    /// Zoom factor (1.0 = default). Drives all zoomed metrics.
    pub zoom: f32,
    /// Active selection as ordered ((line0,col0),(line1,col1)) endpoints, or
    /// `None` when there is no selection. line0/col0 is the earlier endpoint.
    pub selection: Option<((usize, usize), (usize, usize))>,
    /// In-progress IME composition string, shown as a TRANSIENT underlined
    /// overlay at the cursor WITHOUT being committed to the buffer. Empty when no
    /// composition is active. Rendered via the same Advanced-shaping path so CJK
    /// preedit shows real glyphs; the caret sits at the preedit's end.
    pub preedit: String,
    /// Misspelled word spans (line, [start_col, end_col) in CHAR columns), to be
    /// drawn with a wavy red underline. Computed by the [`crate::spell`] engine
    /// from `text` (NOT including the preedit). Empty when nothing is flagged.
    pub misspelled: Vec<crate::spell::Misspelling>,
    /// True when this view follows a text EDIT (typing/delete/paste/newline)
    /// rather than pure navigation. Drives the caret's underline suppression:
    /// edits always slide as a plain block, navigation streaks only on jumps.
    pub is_edit_move: bool,
    /// True when this move came from an OS KEY AUTO-REPEAT (a HELD arrow / motion
    /// key), from `winit`'s `KeyEvent.repeat`. Drives the caret's held-trail: held
    /// navigation keeps the spring springy and draws ONE continuous lagging streak
    /// (well past the gap) instead of a strobing/vanishing per-hop one; a single
    /// tap (`false`) keeps the gap-suppressed lone-hop behaviour. The deterministic
    /// capture/test paths leave this `false`.
    pub held: bool,
    /// Active isearch matches as ordered ((l0,c0),(l1,c1)) CHAR ranges in
    /// document order. Empty when search inactive or zero hits. Same coordinate
    /// convention as `selection`, so highlight rects reuse the selection rect
    /// algorithm.
    pub search_matches: Vec<((usize, usize), (usize, usize))>,
    /// Index into search_matches of the CURRENT match (the real caret sits on
    /// it). None when no matches. The current match is shown by the real amber
    /// caret, not a distinct highlight color.
    pub search_current: Option<usize>,
    /// The live query string shown in the panel (NOT in the rope).
    pub search_query: String,
    /// True while the search panel is open (drives drawing the card + panel text).
    pub search_active: bool,
    /// Case-sensitive toggle state, for the "Aa" indicator.
    pub search_case_sensitive: bool,
    /// REPLACE mode: the same panel reveals a second (replacement) field. Drives
    /// drawing the replace row + sizing the card to two lines.
    pub search_replace_active: bool,
    /// The live replacement string shown in the replace field (NOT in the rope).
    pub search_replacement: String,
    /// Which field the amber caret rides: `false` = the search query (row 0),
    /// `true` = the replacement (row 1).
    pub search_editing_replacement: bool,
    /// True while the summoned navigation OVERLAY is open (go-to / switch). Drives
    /// drawing the overlay card + candidate list + selected-row highlight.
    pub overlay_active: bool,
    /// CRISP-BACKDROP exception: true for the overlays whose entire job is showing
    /// the LIVE document state — the THEME PICKER, the CARET-STYLE PICKER, and the
    /// HISTORY TIMELINE — so the document behind them stays CRISP (no frosted blur,
    /// no dim): the theme picker needs the real theme colours visible, the caret
    /// picker the live caret preview, and the history timeline previews the
    /// highlighted VERSION in the document itself. Every other full overlay
    /// (`false`) gets the cached frosted-blur backdrop.
    pub overlay_crisp: bool,
    /// The overlay's live query string (shown on the query line, with the amber
    /// caret at its end). Empty when no overlay.
    pub overlay_query: String,
    /// THE OVERLAY-TITLES ROUND: this picker's short self-announcement
    /// ([`crate::overlay::OverlayKind::title`]), drawn as a quiet MUTED prefix
    /// ("<title> › ") before the query text on the picker's own input line. Empty
    /// when no overlay is open.
    pub overlay_title: &'static str,
    /// The overlay's filtered + ranked candidate strings, top-to-bottom.
    pub overlay_items: Vec<String>,
    /// EMPTY STATE: `Some(message)` when the overlay has NO candidate rows (an empty
    /// corpus or a query that filtered everything out) — the chrome draws one dim,
    /// non-selectable message row. `None` whenever there ARE rows. Sourced from
    /// [`crate::overlay::OverlayState::empty_notice`], the one owner shared with the
    /// sidecar `overlay.empty` field.
    pub overlay_empty: Option<String>,
    /// Command palette only: binding labels parallel to `overlay_items` (each
    /// command's current chord, drawn dim and right-aligned beside its name).
    /// Empty for every other overlay kind.
    pub overlay_bindings: Vec<String>,
    /// Go-to (notes) picker only: a relative "last edited" label parallel to
    /// `overlay_items` (e.g. "5m ago"), drawn dim and right-aligned beside each
    /// file. Empty for every other overlay kind AND in the headless capture path
    /// (mtime is never read there, so the sidecar stays byte-stable).
    pub overlay_times: Vec<String>,
    /// Project / Browse pickers only: a dim `"git"` tag parallel to `overlay_items`
    /// for each row that is itself a git repo (`""` otherwise), drawn right-aligned in
    /// the SECONDARY column like the palette chords. EMPTY when no row is a git repo
    /// (so a git-free listing keeps no secondary column). From the one owner
    /// [`crate::overlay::OverlayState::item_git_tags`].
    pub overlay_git: Vec<String>,
    /// The selected row, indexing into `overlay_items`.
    pub overlay_selected: usize,
    /// The scroll WINDOW's top row: the `overlay_items` index of the FIRST visible row.
    /// Owned by [`crate::overlay::OverlayState::scroll`] (the source of truth for the
    /// list's scroll position); the pipeline reads it straight so the drawn rows + the
    /// hover hit-test share ONE window and can never disagree.
    pub overlay_scroll: usize,
    /// The per-kind visible-ROW CAP from the ONE owner
    /// [`crate::overlay::OverlayState::window_rows`] (8 for the contextual spell popup,
    /// 12 for the flat + most faceted pickers, larger for the theme picker which shows
    /// every world). The pipeline uses it as the window cap for BOTH the flat card and
    /// the faceted/grouped card (over items), so the drawn rows can never disagree with
    /// the hover / keyboard item-window that `window_rows` also drives. Defaults to 12
    /// when no overlay is open (inert — nothing is drawn).
    pub overlay_window_rows: usize,
    /// One quiet DIM control-hint line drawn at the foot of the overlay card
    /// (per-kind; e.g. "↵ select   → open   ← up" for switch-project, from the shared
    /// `overlay::format_hint` owner), so the select-vs-descend model is discoverable.
    /// Empty = no hint row drawn.
    pub overlay_hint: String,
    /// THEME PICKER only: the faceting lens STRIP — each lens label plus a flag
    /// marking the ACTIVE one (emphasized by VALUE + a thin underline, never amber).
    /// In strip order with All parked at the far left. EMPTY for every other overlay
    /// kind (so the pipeline draws no strip). Drives the theme picker's branch.
    pub overlay_lens: Vec<(String, bool)>,
    /// THEME PICKER only: the SECTION label for each entry in `overlay_items`,
    /// parallel to it — the faint uppercase group header a row sits under (empty under
    /// the All lens / for every non-theme kind). A header line is drawn before a row
    /// whenever its section differs from the previous row's.
    pub overlay_sections: Vec<String>,
    /// CARET-STYLE PICKER preview: `Some(look)` while that picker is open (the look
    /// the highlighted row selects), `None` for every other state. Drives the LIVE
    /// ANIMATED preview box on the card — the pipeline loops its preview caret in this
    /// look while it is `Some`, and STOPS (back to idle) the instant it goes `None`.
    pub caret_preview: Option<CaretMode>,
    /// PAGE-MODE GUTTER: the buffer's display name (`notes.md`, or the derived
    /// `scratch`/slug name for an unsaved note), shown LABEL-sized + muted in the
    /// BOTTOM-LEFT margin gutter — orientation relocated out of the writing column
    /// into the side (DESIGN §4). Empty hides the gutter; the gutter is page-mode
    /// only (edge-to-edge has no margin to hold it).
    pub gutter_name: String,
    /// PAGE-MODE GUTTER: the active project name, stacked LABEL-sized + FAINT under
    /// the filename. Empty draws filename-only.
    pub gutter_project: String,
    /// MARKDOWN STYLING: true when the active buffer is a markdown document
    /// (`.md`/`.markdown` by file extension). Gates the markdown span pass so a
    /// code/plain buffer (`.rs`, `.txt`, an unnamed scratch) is left untouched —
    /// its `#` comments etc. are NOT dimmed, and it renders byte-identically.
    pub is_markdown: bool,
    /// INLINE IMAGES: the directory a RELATIVE image path (`![alt](img.png)`)
    /// resolves against — the open document's own parent dir. `None` for a
    /// no-path scratch/note buffer (a relative path then resolves against the
    /// process cwd) or when the feature is off. Absolute image paths ignore it.
    /// Only read on native, markdown buffers with `inline_images_on()`.
    pub doc_dir: Option<std::path::PathBuf>,
    /// SYNTAX HIGHLIGHTING: the CODE language for this buffer, or `None` when it
    /// must not be highlighted (`.env`/`.md`/`.txt`/unknown/scratch — see
    /// [`crate::buffer::Buffer::syntax_lang`]). Gates the syntax span pass so a
    /// non-code buffer renders byte-identically. Mutually exclusive with
    /// `is_markdown` (a `.md` buffer has `None` here).
    pub syn_lang: Option<crate::syntax::Lang>,
    /// SPELL CONTEXTUAL PANEL: the misspelled word's `(line, start_col, end_col)`
    /// CHAR span when the open overlay is the SPELL picker, else `None`. `Some` turns
    /// the summoned overlay from the centered takeover card into a small floating
    /// panel anchored AT the word (built on `prepare_float_panel`): the doc stays
    /// crisp (no frosted blur, no scrim), and `overlay_geometry` positions the card
    /// just below the word's screen rect. `None` for every other overlay kind — those
    /// render the centered card unchanged.
    pub overlay_spell: Option<(usize, usize, usize)>,
    /// CALM NOTICE: one quiet line drawn LABEL-sized in the muted ink at the
    /// bottom-center of the canvas (today: the autosave clobber guard's
    /// "changed on disk outside awl — autosave held"). Empty draws NOTHING —
    /// the label parks off-screen, so a default capture stays byte-identical.
    /// LIVE-ONLY by construction (autosave can never fire headlessly), so it
    /// has no sidecar field.
    pub notice: String,
    /// i18n: the Han-ambiguity TIEBREAK ladder (config `cjk_priority`, default
    /// `[Ja, ZhHans, ZhHant, Ko]`) the per-run render resolution ladder
    /// consults for a Han run with no compatible doc-language tag (see
    /// `crate::script::resolve_font_id` step (c)). Every non-live caller
    /// (bench/perfbench/framebench/capture fixtures) uses the built-in default
    /// (`crate::frontmatter::DEFAULT_CJK_PRIORITY`); only the live `App`
    /// (`app/viewstate.rs`) threads the user's configured value.
    pub cjk_priority: Vec<crate::frontmatter::Lang>,
    /// LINE ENDINGS: the active buffer's on-disk line-ending discipline
    /// ([`crate::buffer::Eol`] — `Lf`/`Crlf`). Unlike `doc_lang`/`syn_lang`, this
    /// CANNOT be re-derived from `text` (the rope is always pure-`\n`; the ending
    /// is document metadata the buffer remembers from load), so the live App
    /// threads it here. A PURE fact of the buffer, so the held stats HUD shows its
    /// real value in a headless capture (unlike the dropped clock/fs HUD fields)
    /// and the sidecar asserts it (`hud.eol`).
    pub eol: crate::buffer::Eol,
    /// THE FORMAT POPOVER model for this frame ([`crate::popover::PopoverModel`]),
    /// or `None` when the popover is down. Built by the caller — the live App's
    /// `sync_view` (mouse-summoned + config-gated) or the capture force-summon
    /// probe (`AWL_POPOVER`) — from [`crate::actions::popover::plan`] over the
    /// current selection, so the lit toggles + the `H` button's level stay live
    /// and reflective across format applies. Drives the floating button row + its
    /// hit-test + the sidecar `popover` block. The row is ANCHORED off `selection`
    /// (its earlier endpoint), so a `Some` model always rides a live selection.
    /// `None` parks every popover quad/glyph empty, so a default capture is
    /// byte-identical.
    pub popover: Option<crate::popover::PopoverModel>,
    /// DIFF-AS-PREVIEW: true while the History picker's writer's-diff preview is
    /// what the document shows — the page column dresses as a CARD (the float-panel
    /// border + elevation around the column, content clipped to the panel band; see
    /// `prepare_diff_panel` / `doc_clip_band`). False for every ordinary frame, so a
    /// default capture is byte-identical.
    pub diff_panel: bool,
    /// DIFF-AS-PREVIEW focus cue: true when Tab moved the keyboard focus INTO the
    /// diff panel — its card border strengthens one value step (content ink) and
    /// widens a px (the value-free half of the cue, so it survives a one-bit
    /// world). Never amber. Inert unless `diff_panel`.
    pub diff_panel_focus: bool,
    /// COLLAPSED SECTIONS (folds): the FULL-document logical lines of the ATX
    /// headings whose sections are folded, ascending. VIEW state only — the rope is
    /// untouched. `text` above is already the FOLD-FILTERED document (hidden lines
    /// dropped by [`crate::fold::Filter`], so they shape to ZERO height — the row
    /// simply does not exist), and `cursor_line` / `selection` / `search_matches` /
    /// `misspelled` are in that same filtered line space; this field carries the
    /// (unfiltered) folded-heading lines so the sidecar can report the fold state
    /// and a future tail/chevron pass can find each collapsed heading. Empty when
    /// nothing is folded, so a default capture is byte-identical.
    pub folds: Vec<usize>,
}

impl ViewState {
    /// The CANONICAL default `ViewState` — an empty, unscrolled, unzoomed prose
    /// buffer with every search / overlay field inert. This is the ONE owner of
    /// "what a fresh ViewState looks like": the bench / perf / frame / capture /
    /// test scaffolds all build on it (`ViewState { <real fields>, ..base() }`),
    /// so a NEW ViewState field is defaulted in exactly ONE place here and every
    /// scaffold inherits it automatically — retiring the old "update all six
    /// initializers or the build breaks at merge" ritual. The live App's
    /// `sync_view` (`src/app/viewstate.rs`) stays deliberately EXHAUSTIVE (it sets
    /// every field from live state and MUST fail to compile when a field is added,
    /// forcing a conscious render decision) — it is the one authoritative site and
    /// does not route through `base()`.
    ///
    /// Non-inert defaults: `zoom = 1.0`, `overlay_window_rows = 12` (the no-overlay
    /// cap the pipeline windows against), `cjk_priority = DEFAULT_CJK_PRIORITY`, and
    /// `eol = Eol::Lf` — matching the value every scaffold previously spelled out.
    pub fn base() -> Self {
        ViewState {
            text: String::new(),
            cursor_line: 0,
            cursor_col: 0,
            caret_affinity: crate::caret::Affinity::Downstream,
            scroll_lines: 0,
            zoom: 1.0,
            selection: None,
            preedit: String::new(),
            misspelled: Vec::new(),
            is_edit_move: false,
            held: false,
            search_matches: Vec::new(),
            search_current: None,
            search_query: String::new(),
            search_active: false,
            search_case_sensitive: false,
            search_replace_active: false,
            search_replacement: String::new(),
            search_editing_replacement: false,
            overlay_active: false,
            overlay_crisp: false,
            overlay_query: String::new(),
            overlay_title: "",
            overlay_items: Vec::new(),
            overlay_empty: None,
            overlay_bindings: Vec::new(),
            overlay_times: Vec::new(),
            overlay_git: Vec::new(),
            overlay_selected: 0,
            overlay_scroll: 0,
            overlay_window_rows: 12,
            overlay_hint: String::new(),
            overlay_lens: Vec::new(),
            overlay_sections: Vec::new(),
            caret_preview: None,
            gutter_name: String::new(),
            gutter_project: String::new(),
            is_markdown: false,
            doc_dir: None,
            syn_lang: None,
            overlay_spell: None,
            notice: String::new(),
            cjk_priority: crate::frontmatter::DEFAULT_CJK_PRIORITY.to_vec(),
            eol: crate::buffer::Eol::Lf,
            popover: None,
            diff_panel: false,
            diff_panel_focus: false,
            folds: Vec::new(),
        }
    }
}
