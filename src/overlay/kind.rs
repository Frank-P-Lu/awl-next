//! `OverlayKind` -- which picker/navigator is open -- its accept-disposition
//! law, and the picker-row hint-line shape. Split out of the former
//! `overlay.rs` monolith (2026-07 code-organization pass); every item's path
//! is unchanged (`overlay::OverlayKind`, `overlay::AcceptDisposition`,
//! `overlay::HintAction`, `overlay::format_hint`) -- only the file it lives
//! in moved.

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
    /// The CARET-STYLE picker (Cmd-P → "Caret style…"): lists the three caret looks
    /// (Block / Morph / I-beam) each with a one-line description, with a LIVE
    /// ANIMATED PREVIEW of the highlighted look (a "Smash character-select" box where
    /// the caret loops a representative motion). Navigating PREVIEWS the look (applies
    /// it to the process-global so the document caret + the preview switch); Enter
    /// COMMITS + persists it; Esc/C-g reverts to the look active when it opened — back
    /// to AUTO itself (never a pin) when that's what was active, so merely opening the
    /// picker to look and backing out is a true no-op. It carries `original_caret` (the
    /// look active when it opened, for the ordinary revert) and
    /// `original_caret_was_auto` (whether that was auto's momentary resolution rather
    /// than a real pin) so a Cancel can restore the previous state exactly.
    Caret,
    /// The MOVE-DESTINATION picker (C-x m): reuses the Browse navigator but lists
    /// only FOLDERS (you move a note INTO a folder). It is rooted at the notes
    /// root. Right/`ForwardChar` DESCENDS into the highlighted folder, Left ASCENDS,
    /// and Enter ACCEPTS the destination — either the highlighted folder, or, when
    /// the typed query matches no listed folder, a NEW folder of that name to
    /// create. The accepted value is a notes-root-relative directory path.
    MoveDest,
    /// The DICTIONARY picker (Cmd-P → "Dictionary…"): lists the three bundled
    /// spell-check variants (English US / UK / Australia), each with a
    /// one-line description, mirroring the CARET-STYLE picker's layout — EXCEPT
    /// there is NO live preview as the selection moves (a dictionary re-parse is
    /// a genuine one-time cost, tens of ms, not a per-keystroke one — see
    /// `spell.rs`), so navigating just highlights. Enter COMMITS: the process-
    /// global active variant is set THEN (not during navigation), the caller
    /// reconstructs its `SpellChecker` + persists the sticky pref. Esc/C-g
    /// simply closes (nothing was ever previewed to revert).
    Dictionary,
    /// The CJK-PRIORITY LANGUAGE picker (Settings → "Ambiguous CJK reads as…"):
    /// lists the four ambiguity-ladder languages (Japanese / Simplified Chinese
    /// / Traditional Chinese / Korean, in [`crate::frontmatter::DEFAULT_CJK_PRIORITY`]
    /// order) in WRITER WORDS, mirroring the DICTIONARY picker's shape exactly —
    /// no live preview (picking a Han-tiebreak default is not something you
    /// preview character-by-character), pre-selected on whichever language
    /// currently sits at the FRONT of the live ladder. Enter PROMOTES the
    /// highlighted language to the front of [`crate::frontmatter::cjk_priority`]
    /// (the rest keep their relative order) — set core-level, in
    /// `actions::overlay_nav`, exactly like Theme/Caret/Dictionary, so both the
    /// live App and a headless `--keys` replay observe the promotion. Esc/C-g
    /// simply closes (nothing was ever previewed to revert).
    CjkLang,
    /// The COMMAND PALETTE (Cmd-P): a fuzzy search over the command CATALOG names
    /// (`commands::COMMANDS`), each row showing the command's current key binding
    /// dim beside it. Enter RUNS the selected command's `Action`; the catalog
    /// order == the corpus order, so the selected corpus index maps straight back
    /// to `COMMANDS[i]`.
    Command,
    /// The SPELL-SUGGESTION picker (Cmd-`;`): lists the spellchecker's ordered
    /// corrections for the misspelled word the cursor is on. Enter REPLACES that
    /// word with the chosen suggestion (a single undoable edit). Flat + transient;
    /// it carries `spell_target` — the word's `(line, start_col, end_col)` span —
    /// so the accept can locate the word to swap.
    Spell,
    /// The GAME-STYLE REBIND MENU (Cmd-P → "Keybindings…"): lists EVERY command +
    /// its two current bindings (like the palette's binding column), fuzzy-filterable.
    /// Enter on a command opens a CAPTURE sub-state ([`Capture`], carried in
    /// `capture`) — choose KEY (one combo, finishes instantly) or CHORD (a sequence,
    /// Enter finishes) — and the captured spec is written to the command's `[keys]`
    /// slot, saved + live-reloaded. Delete on a command RESETS it to default; a
    /// transient `notice` shows conflicts / saves. Summoned + transient, never a
    /// settings window.
    Keybindings,
    /// The SUMMONED HISTORY TIMELINE (Cmd-Shift-H → "Version history…"): lists the current
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
    /// The SETTINGS MENU (Cmd-P → "Settings…"): a faceted, fuzzy-filterable list of
    /// every editor setting ([`crate::settings::SETTINGS`]), the CATEGORIES as
    /// lenses (All · Editor · Appearance · Writing · Files · Keybindings ·
    /// Advanced). Each row's SECONDARY column shows the setting's CURRENT VALUE
    /// (read from the same owners the renderer reads). v1: the menu OPENS +
    /// DISPLAYS; Enter interactions (toggle / edit / open a sub-picker) are wired
    /// next phase. Summoned + transient, never a settings window.
    Settings,
    /// The ASSET CLEANER (Cmd-P → "Clean unused assets…"): a flat, fuzzy-filterable
    /// list of the ORPHAN image files under the active project — an image under an
    /// `assets/` directory that no document references ([`crate::assets::scan`]). Each
    /// row's PRIMARY cell is the file NAME, its SECONDARY the human size + parent dir
    /// (via the `bindings` column). Enter moves that file to the macOS TRASH
    /// (recoverable — never `rm`; live-App-only, a headless no-op) and REMOVES the row
    /// while the picker STAYS OPEN ([`OverlayState::remove_asset_row`]); Esc closes. An
    /// empty list shows the calm "no unused assets" row (always summons, like History).
    /// The corpus is the root-relative PATHS (accept/trash key + fuzzy corpus); the
    /// displayed name is the leaf ([`OverlayState::display_of`]).
    Assets,
    /// NOTES VERBS round: the RENAME minibuffer (Cmd-P → "Rename note…") — a
    /// single-row prompt, pre-filled with the current filename, whose typing is
    /// owned entirely by the modal `rename_edit` sub-state (mirroring the Settings
    /// menu's inline `ValueEdit`, generalized to free text): every keystroke
    /// mutates `corpus[0]` directly (so the live-typed name IS the row's primary
    /// cell — no separate preview column), Enter commits (`Effect::RenameNoteCommit`,
    /// core closes the overlay), Esc cancels. There is no list to browse — the
    /// corpus always holds exactly the one editable row.
    Rename,
    /// LINKS V2: the Cmd-K minibuffer (Insert link…) — a single-row URL prompt,
    /// mirroring [`OverlayKind::Rename`]'s exact shape: every keystroke mutates
    /// `corpus[0]` directly (the live-typed URL IS the row's primary cell), Enter
    /// commits (the whole edit is applied to the buffer INSIDE the core — no
    /// `Effect`, since it never needs the filesystem), Esc cancels. Pre-filled
    /// empty / from the clipboard-if-URL / from an existing link's current URL,
    /// per `Action::InsertLink`'s own doc. No list to browse.
    InsertLink,
    /// NAMED SAVE POINTS: the "Keep version…" minibuffer — a single-row prompt
    /// for the kept version's OPTIONAL name, mirroring [`OverlayKind::Rename`]'s
    /// exact shape (the modal `keep_edit` sub-state owns every key; the
    /// live-typed name IS the row's primary cell). Enter COMMITS
    /// (`Effect::KeepVersion { name }` — `Some(name)` for a typed name, `None`
    /// for a blank Enter, which is exactly the pre-name plain keep: zero
    /// friction preserved), Esc cancels (nothing recorded). Opens empty (there
    /// is no old name to seed — a fresh point is being marked). No list to
    /// browse.
    KeepName,
}

/// How a picker's ACCEPT (Enter on a committed item) disposes of the breadcrumb
/// stack — the pop-vs-close-all classification. The BREADCRUMB rule for Esc/cancel
/// is uniform (it always POPS back to the summoning overlay via `return_to`); an
/// ACCEPT differs by what it *does*:
///   * [`Navigate`](AcceptDisposition::Navigate) — the accept lands you in a RESULT
///     (open a file, jump to a heading, switch the project, restore a version, move
///     a note, run a command), so it closes the WHOLE stack to the buffer even when
///     a parent breadcrumb is set: you asked to go somewhere, not to configure the
///     summoning overlay.
///   * [`ValuePick`](AcceptDisposition::ValuePick) — the accept just COMMITS a
///     setting the summoning overlay was picking (keep a theme, apply a caret look /
///     dictionary). It POPS back to the parent (like Esc) ONLY when the parent
///     [retains its value-pick child](OverlayKind::retains_value_pick_child) — i.e.
///     SETTINGS, a configuration surface you keep using (the Settings sub-picker
///     precedent). Launched from the COMMAND palette (a one-shot launcher) or summoned
///     DIRECTLY (no parent), the commit COMPLETES the action, so it closes to the
///     buffer rather than re-open the launcher. (Esc still pops back universally.)
///   * [`StayOpen`](AcceptDisposition::StayOpen) — the accept never closes at all
///     (trash an orphan and keep listing, start a rebind capture, toggle a setting).
///
/// The ONE owner of the classification, swept by a NO-WILDCARD law test
/// (`overlay::tests::every_kind_declares_an_accept_disposition`), so a future kind
/// fails to compile until it declares its pop-vs-close class. NOTE: the `Project`
/// navigator carries a documented CONTEXTUAL override — when it was opened FROM a
/// Settings PATH row (`setting_path_key` set) its accept POPS back to Settings
/// instead of closing-all, handled at that accept seam, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptDisposition {
    Navigate,
    ValuePick,
    StayOpen,
}

impl OverlayKind {
    /// Every overlay kind, for the enumerating law tests (the `match` arms that
    /// enumerate `OverlayKind` with a NO-WILDCARD sweep — `facets::scheme`,
    /// `rowlayout` — are the real compile-time guards; this is iteration
    /// convenience, kept in lockstep by hand like `CaretMode::ALL`).
    #[allow(dead_code)] // consumed only by the `facets`/law tests today.
    pub const ALL: [OverlayKind; 17] = [
        OverlayKind::Goto,
        OverlayKind::Project,
        OverlayKind::Browse,
        OverlayKind::Theme,
        OverlayKind::Caret,
        OverlayKind::Dictionary,
        OverlayKind::CjkLang,
        OverlayKind::MoveDest,
        OverlayKind::Command,
        OverlayKind::Spell,
        OverlayKind::Keybindings,
        OverlayKind::History,
        OverlayKind::Settings,
        OverlayKind::Assets,
        OverlayKind::Rename,
        OverlayKind::InsertLink,
        OverlayKind::KeepName,
    ];

    /// Resolve a capture-sidecar MODE string ([`Self::as_str`]) back to its kind —
    /// derived from [`Self::ALL`] + `as_str` (no third match to maintain), so it
    /// can never disagree with the forward mapping. `None` for an unknown string.
    /// Lets the headless capture path consult the REAL per-kind owners
    /// ([`Self::draws_title_prefix`]) instead of re-listing mode strings by hand —
    /// the aligned-copy drift the KeepName round caught in `capture/modes.rs`.
    pub fn from_mode(mode: &str) -> Option<OverlayKind> {
        Self::ALL.iter().copied().find(|k| k.as_str() == mode)
    }

    /// The short mode string used in the capture sidecar.
    pub fn as_str(self) -> &'static str {
        match self {
            OverlayKind::Goto => "goto",
            OverlayKind::Project => "switch",
            OverlayKind::Browse => "browse",
            OverlayKind::Theme => "theme",
            OverlayKind::Caret => "caret",
            OverlayKind::Dictionary => "dictionary",
            OverlayKind::CjkLang => "cjk_lang",
            OverlayKind::MoveDest => "move",
            OverlayKind::Command => "command",
            OverlayKind::Spell => "spell",
            OverlayKind::Keybindings => "keybindings",
            OverlayKind::History => "history",
            OverlayKind::Settings => "settings",
            OverlayKind::Assets => "assets",
            OverlayKind::Rename => "rename",
            OverlayKind::InsertLink => "insert_link",
            OverlayKind::KeepName => "keep_version",
        }
    }

    /// This kind's ACCEPT disposition — the ONE owner of the breadcrumb
    /// pop-vs-close-all-vs-stay classification (see [`AcceptDisposition`]). A
    /// NO-WILDCARD match: a future kind fails to compile here until it declares its
    /// class, so the breadcrumb behaviour can never be silently forgotten.
    pub fn accept_disposition(self) -> AcceptDisposition {
        use AcceptDisposition::*;
        match self {
            // Open a file / descend-then-open, jump to a heading, switch the project
            // root, move the note, restore a version, or run a chosen command — every
            // one LANDS you in the result, so close the whole stack to the buffer.
            // (The command palette also RE-DISPATCHES its choice, which then opens any
            // sub-overlay stamped `return_to = Command`; the palette itself closes.)
            OverlayKind::Goto
            | OverlayKind::Browse
            | OverlayKind::Project
            | OverlayKind::MoveDest
            | OverlayKind::Spell
            | OverlayKind::History
            | OverlayKind::Command => Navigate,
            // Keep a theme / apply a caret look / apply a dictionary / promote a CJK
            // language — the accept just commits the value the summoning overlay was
            // picking, so POP back to it.
            OverlayKind::Theme
            | OverlayKind::Caret
            | OverlayKind::Dictionary
            | OverlayKind::CjkLang => ValuePick,
            // Trash an orphan (row leaves, list stays), start a rebind capture, or the
            // settings menu's own toggles / sub-picker swaps / inline value edits — the
            // accept never closes the overlay.
            OverlayKind::Assets | OverlayKind::Keybindings | OverlayKind::Settings => StayOpen,
            // RENAME: a commit LANDS you in a result (the file is renamed), so it
            // closes the whole stack — same class as MoveDest. (In practice the
            // `rename_edit` modal intercept closes the overlay itself the instant
            // Enter commits, before this classification is ever consulted for a
            // Rename accept — declared here anyway so the law test's no-wildcard
            // sweep can't silently forget this kind.)
            OverlayKind::Rename => Navigate,
            // LINKS V2: a commit LANDS the edit in the buffer (a real result), so
            // it closes the whole stack — same class as Rename. (In practice the
            // `link_edit` modal intercept closes the overlay itself the instant
            // Enter commits, before this classification is ever consulted —
            // declared here anyway so the law test's no-wildcard sweep can't
            // silently forget this kind.)
            OverlayKind::InsertLink => Navigate,
            // NAMED SAVE POINTS: a commit LANDS a result (the version is kept),
            // so it closes the whole stack — same class as Rename/InsertLink.
            // (In practice the `keep_edit` modal intercept closes the overlay
            // itself at Enter, before this is consulted — declared anyway for
            // the no-wildcard sweep.)
            OverlayKind::KeepName => Navigate,
        }
    }

    /// When a VALUE-PICK sub-picker (Theme / Caret / Dictionary / CjkLang) was
    /// summoned FROM this overlay, does COMMITTING it (Enter) RE-SUMMON this overlay,
    /// or land in the buffer? True ONLY for the SETTINGS menu — a persistent
    /// configuration surface you keep using, so a commit pops back to keep
    /// configuring (the "Settings sub-picker precedent"). The COMMAND palette is a
    /// one-shot LAUNCHER: a committed value-pick COMPLETES the command you launched,
    /// so it belongs in the buffer — re-opening the launcher (which re-appears on its
    /// Recent lens) reads as a stray "recent" menu popping up, the user-reported
    /// ship-blocker "Switch theme → select → it goes into the recent files menu". Esc
    /// still pops back to the summoning overlay UNIFORMLY (see `close_overlay`); only
    /// an ACCEPT consults this. Consulted by VALUE (the stored `return_to` kind), never
    /// by enum position, so retiring a sibling variant can never re-aim a breadcrumb.
    pub fn retains_value_pick_child(self) -> bool {
        matches!(self, OverlayKind::Settings)
    }

    /// True for the FILE/FOLDER pickers whose corpus entries are filesystem paths —
    /// the ones that HIDE dot-prefixed entries by default (with a `Cmd-Shift-.`
    /// reveal toggle). Goto (+ recent-files, same corpus) lists root-relative paths;
    /// Browse / MoveDest list one directory LEVEL's leaf names; Project navigates the
    /// workspace's child folders. Project INCLUDES itself here so `.git`/`.claude`/…
    /// dotfolders hide by default too — its synthetic "." accept-this-folder row is
    /// NOT a dotfile to hide (the `refilter` filter exempts it explicitly), and the
    /// `.env*` exception from [`crate::index::is_hidden_entry`] still applies. The
    /// non-file pickers (theme / command / caret / …) never match a path, so the
    /// toggle is a calm no-op there.
    pub fn hides_dotfiles(self) -> bool {
        matches!(
            self,
            OverlayKind::Goto | OverlayKind::Browse | OverlayKind::MoveDest | OverlayKind::Project
        )
    }

    /// The per-kind visible ROW CAP — the ONE owner of each picker's window size, read by
    /// BOTH [`OverlayState::window_rows`] (the hover / keyboard / scroll math) AND the
    /// render pipeline's drawn window (threaded via
    /// [`crate::render::ViewState::overlay_window_rows`]), so the two can never disagree
    /// about which rows are on screen. The contextual SPELL popup stays compact (8); the
    /// flat THEME picker sizes to the whole world roster so every world is browsable
    /// without a scroll (the render path then reduces it to fit the canvas); every other
    /// centered picker shows up to 12.
    pub fn window_rows(self) -> usize {
        match self {
            OverlayKind::Spell => 8,
            OverlayKind::Theme => crate::theme::THEMES.len(),
            _ => 12,
        }
    }

    /// The ordered control-hint ACTIONS for this picker — the DATA half of the foot
    /// hint. Each picker supplies only its own actions, in the ONE canonical ORDER:
    /// the PRIMARY action (what ↵ does) first, NAVIGATION next (lens / descend /
    /// ascend), and CANCEL (esc) LAST. The consistent SHAPE — `glyph SPACE label`,
    /// joined by [`HINT_SEP`] — is [`format_hint`]'s alone, so every picker's foot
    /// line reads identically spaced + ordered regardless of which keys it teaches.
    ///
    /// GLYPH conventions (one vocabulary across every picker): `↵` Return (bundled in
    /// AwlSymbols, matching the ⌘/⌥ chord glyphs); `←`/`→`/`↑`/`↓` the arrow keys
    /// (combined `←/→` for a lens axis); `⌫` Backspace (ascend a level); and a short
    /// lowercase WORD (`esc`, `del`) for a key with no bundled glyph.
    pub fn hint_actions(self) -> Vec<HintAction> {
        // Every summoned overlay is a navigable LIST — ↑/↓ (and C-n/C-p) always move
        // the selection — so the MOVE affordance is UNIVERSAL and LEADS every kind's
        // line. Prepended here in the ONE shared owner so no kind can forget to teach
        // it (users otherwise reach for the OS-eaten Ctrl-Up/Down and think nav is
        // broken). The per-kind primary/nav/cancel actions follow, from `kind_actions`.
        let mut actions = vec![HintAction { glyph: "\u{2191}/\u{2193}", label: "move" }];
        actions.extend(self.kind_actions());
        actions
    }

    /// The per-kind primary/nav/cancel hint actions, in canonical order. The
    /// UNIVERSAL `↑/↓ move` lead is prepended by [`Self::hint_actions`] (every list
    /// navigates the same way), so an arm here only names what is SPECIFIC to the kind.
    fn kind_actions(self) -> Vec<HintAction> {
        // The primary ↵ action every picker leads with (after the shared ↑/↓ move).
        let enter = |label| HintAction { glyph: "\u{21B5}", label };
        let key = |glyph, label| HintAction { glyph, label };
        match self {
            // Project is a FACETED navigator (All / Recent lens): ↵ SELECTS the
            // project (on a folder it descends; on the "." row it picks the current
            // dir), ←/→ switch the lens, ⌫ ascends a level — matching Browse.
            OverlayKind::Project => vec![
                enter("select"),
                key("\u{2190}/\u{2192}", "lens"),
                key("\u{232B}", "up"),
            ],
            // Select context: ↵ MOVES the note into the folder; → descends, ← ascends.
            OverlayKind::MoveDest => vec![
                enter("move here"),
                key("\u{2192}", "open"),
                key("\u{2190}", "up"),
            ],
            // Browse is a FACETED explorer: ↵ on a folder descends / on a file opens,
            // ←/→ switch the lens, ⌫ ascends a level.
            OverlayKind::Browse => vec![
                enter("open"),
                key("\u{2190}/\u{2192}", "lens"),
                key("\u{232B}", "up"),
            ],
            // Go-to is a FACETED flat picker: ↵ opens, ←/→ switch the lens.
            OverlayKind::Goto => vec![enter("open"), key("\u{2190}/\u{2192}", "lens")],
            // The flat theme picker: ↵ keeps, esc reverts to the opening theme. (↑/↓
            // moves the world with live preview — taught by the shared universal
            // `↑/↓ move` lead, so it is not repeated here.) The runtime lens strip was
            // retired (2026-07-15) — the picker is a flat browsable world list.
            OverlayKind::Theme => vec![enter("keep"), key("esc", "revert")],
            // Caret style: Up/Down PREVIEWS the look (live), ↵ APPLIES + persists it.
            OverlayKind::Caret => vec![enter("apply")],
            // Dictionary: no live preview (a re-parse is real work) — ↵ applies +
            // persists the highlighted variant.
            OverlayKind::Dictionary => vec![enter("apply")],
            // CJK-priority language: no live preview (mirrors Dictionary) — ↵
            // promotes the highlighted language to the front of the ladder.
            OverlayKind::CjkLang => vec![enter("apply")],
            // The faceted command palette: ↵ runs, ←/→ switch the lens (All / File /
            // Edit / View / Recent).
            OverlayKind::Command => vec![enter("run"), key("\u{2190}/\u{2192}", "lens")],
            OverlayKind::Spell => vec![enter("replace")],
            // The rebind menu: ↵ starts a capture, del resets the highlighted command,
            // esc closes. (In a capture the prompt teaches Key/Chord/Enter/Esc.)
            OverlayKind::Keybindings => vec![
                enter("rebind"),
                key("del", "reset"),
                key("esc", "close"),
            ],
            // The faceted history timeline: ↵ RESTORES the highlighted version (an
            // undoable edit), ⇥ COMPARES it against the current buffer (the read-only
            // prose-diff view), ←/→ switch the lens (All / Session / Today), esc closes.
            OverlayKind::History => vec![
                enter("restore"),
                key("tab", "compare"),
                key("\u{2190}/\u{2192}", "lens"),
                key("esc", "close"),
            ],
            // The faceted settings menu: ↵ edits the highlighted setting (toggle /
            // open a sub-picker — wired next phase), ←/→ switch the category lens,
            // esc closes.
            OverlayKind::Settings => vec![
                enter("edit"),
                key("\u{2190}/\u{2192}", "lens"),
                key("esc", "close"),
            ],
            // The asset cleaner: ↵ TRASHES the highlighted orphan (recoverable; the
            // row leaves + the picker stays open), esc closes. A flat list — no lens.
            OverlayKind::Assets => vec![enter("trash"), key("esc", "close")],
            // The RENAME minibuffer: no list nav at all (a single editable row) —
            // its own `rename_edit` prompt (via `foot_hint`) teaches Enter/Esc, so
            // this table's shared ↑/↓ move lead is the only universal bit that
            // actually applies; declared minimal rather than omitted, so every kind
            // stays under the no-wildcard sweep.
            OverlayKind::Rename => vec![enter("rename"), key("esc", "cancel")],
            // LINKS V2: no list nav either (a single editable row) — its own
            // `link_edit` prompt (via `foot_hint`) teaches Enter/Esc, mirroring
            // Rename exactly.
            OverlayKind::InsertLink => vec![enter("insert link"), key("esc", "cancel")],
            // NAMED SAVE POINTS: no list nav either (a single editable row) —
            // its own `keep_edit` prompt (via `foot_hint`) teaches Enter/Esc,
            // mirroring Rename/InsertLink exactly.
            OverlayKind::KeepName => vec![enter("keep"), key("esc", "cancel")],
        }
    }

    /// One quiet line of control hints for this picker, drawn DIM at the foot of the
    /// overlay card so the select-vs-descend model is discoverable. The per-kind
    /// action DATA is [`Self::hint_actions`]; the shared [`format_hint`] owns the
    /// consistent formatting (`glyph label`, [`HINT_SEP`]-joined, move→primary→nav→
    /// cancel order). Rendered + surfaced to the sidecar so it stays agent-verifiable.
    pub fn hint(self) -> String {
        format_hint(&self.hint_actions())
    }

    /// The calm line a picker shows when its CORPUS is empty (nothing to list at
    /// all — as opposed to a query that filtered a non-empty corpus down to zero,
    /// which reads the universal "no matches" in [`OverlayState::empty_message`]).
    /// The ONE owner of each picker's empty-corpus wording — the history timeline's
    /// long-standing "no history yet" generalized to every kind, so an empty
    /// history / empty spell suggestion list / empty folder all read as one calm,
    /// consistent, dim message row (never a blank card).
    pub fn empty_corpus_message(self) -> &'static str {
        match self {
            OverlayKind::History => "no history yet",
            OverlayKind::Spell => "no suggestions",
            OverlayKind::Browse => "this folder is empty",
            OverlayKind::Goto | OverlayKind::Project | OverlayKind::MoveDest => "no files here",
            // The asset cleaner: an empty corpus means nothing to clean up.
            OverlayKind::Assets => "no unused assets",
            OverlayKind::Theme
            | OverlayKind::Caret
            | OverlayKind::Dictionary
            | OverlayKind::CjkLang
            | OverlayKind::Command
            | OverlayKind::Keybindings
            | OverlayKind::Settings => "no matches",
            // RENAME always summons with exactly one row (the editable name) —
            // this arm is structurally unreachable, but every kind still needs one
            // under the no-wildcard sweep.
            OverlayKind::Rename => "no matches",
            // LINKS V2 always summons with exactly one row (the editable URL) —
            // this arm is structurally unreachable, mirroring Rename.
            OverlayKind::InsertLink => "no matches",
            // NAMED SAVE POINTS always summons with exactly one row (the
            // editable name) — structurally unreachable, mirroring Rename.
            OverlayKind::KeepName => "no matches",
        }
    }

    /// THE OVERLAY-TITLES ROUND: the short, lowercase name this picker announces
    /// itself with — a QUIET PREFIX drawn on the picker's own input line ("<title>
    /// › " in muted ink before the typed query), so routing from the palette into
    /// another picker (Keybindings / Settings / Themes / …) always says where you
    /// landed. A NO-WILDCARD match: a future kind fails to compile here until it
    /// names itself, mirroring [`Self::accept_disposition`]/[`Self::hint_actions`].
    /// Not every kind actually DRAWS this prefix (the RENDER exceptions — Spell has
    /// no input line to prefix, and Rename/InsertLink already orient via their own
    /// modal prompt — are a render-time choice, not a reason to leave a kind
    /// unnamed here).
    pub fn title(self) -> &'static str {
        match self {
            OverlayKind::Goto => "go to",
            OverlayKind::Project => "switch project",
            OverlayKind::Browse => "browse",
            OverlayKind::Theme => "themes",
            OverlayKind::Caret => "caret style",
            OverlayKind::MoveDest => "move note",
            OverlayKind::Dictionary => "dictionary",
            OverlayKind::CjkLang => "ambiguous cjk",
            OverlayKind::Command => "commands",
            OverlayKind::Spell => "spelling",
            OverlayKind::Keybindings => "keybindings",
            OverlayKind::History => "version history",
            OverlayKind::Settings => "settings",
            OverlayKind::Assets => "unused assets",
            OverlayKind::Rename => "rename",
            OverlayKind::InsertLink => "insert link",
            OverlayKind::KeepName => "keep version",
        }
    }

    /// THE OVERLAY-TITLES ROUND: does this kind's RENDER draw the `title() › `
    /// prefix on its input line? `false` for Rename/InsertLink/KeepName — their
    /// own modal prompt (`foot_hint`, "rename to:"/"link to:"/"name this
    /// version:") already orients, so a second self-announcement would be
    /// redundant chrome; the SIDECAR still reports [`Self::title`]
    /// unconditionally for every kind (the law is "every kind names itself",
    /// not "every kind draws it" — see [`Self::title`]'s own doc). Spell (no
    /// input line at all, `header_rows == 0`) needs no exclusion here — the
    /// render path simply never reaches a query line to prefix for it.
    pub fn draws_title_prefix(self) -> bool {
        !matches!(
            self,
            OverlayKind::Rename | OverlayKind::InsertLink | OverlayKind::KeepName
        )
    }

    /// THE SETTINGS-MARKER GLYPH (the union round): a settings row reached via the
    /// command palette draws this glyph, dim in muted ink, before its name (e.g.
    /// `"§ Keymap"`) so it reads as visibly a SETTING, never a command. Measured
    /// against the bundled `AwlMarks.ttf` (awl's own symbol set, `render::
    /// SYMBOL_FAMILY`) FIRST per the round's own priority order — § (U+00A7,
    /// SECTION SIGN) is already one of that face's typographic marks (alongside
    /// † ‡ • ◦ ▪, see `theme::ornament`'s module doc), so it renders IDENTICALLY
    /// on every world and every platform (bundled, never a system fallback — the
    /// same guarantee the chord glyphs `⌘⇧⌥` lean on). The gear ⚙ (U+2699) was
    /// also measured and does NOT exist in `AwlMarks.ttf` — confirmed via
    /// `fontTools.ttLib`'s cmap — so it loses to § outright; a system-font gear
    /// would also violate the "identical on every platform" bar the bundled face
    /// meets for free. `render::spans::is_symbol` already lists § (it's a
    /// pre-existing reference mark), so no font-routing code changed — only this
    /// marker's USE is new.
    ///
    /// The full marker PREFIX (glyph + one space) a settings row's display text
    /// carries — the single owner both [`OverlayState::display_of`] (which
    /// prepends it) and [`crate::overlay::row_split`] (which recognizes it as the
    /// muted-ink figure/ground split point, exactly like a file row's directory
    /// prefix) read, so the two can never disagree about where the marker ends.
    pub const SETTINGS_MARKER_PREFIX: &'static str = "§ ";

    /// The calm line a FACETING picker shows when a REFINEMENT lens (a strip index
    /// past the flat `All` home) filtered the corpus down to zero — distinct from an
    /// empty CORPUS ([`Self::empty_corpus_message`]) or a query that matched nothing
    /// (the universal "no matches"). `None` for the flat `All` lens (index 0) and any
    /// lens with no special wording, so [`OverlayState::empty_message`] falls back to
    /// the corpus message. The Go-to **Recent** lens is the warm one: nothing has been
    /// opened yet, so it invites rather than reports. Every other refinement lens with
    /// no members reads the calm catch-all "nothing here".
    pub fn empty_lens_message(self, lens: &str) -> Option<&'static str> {
        match (self, lens) {
            // Go-to Recent: a real MRU that is empty until you open something.
            (OverlayKind::Goto, "recent") => Some("no recent files yet"),
            // Go-to Headings: the current doc's headings — empty over a non-markdown
            // buffer (or a markdown one with no headings yet). The fold that retired
            // the standalone Outline picker keeps its calm empty-state wording.
            (OverlayKind::Goto, "headings") => Some("no headings yet"),
            // Project Recent: the recent-projects MRU, empty until you switch projects.
            (OverlayKind::Project, "recent") => Some("no recent projects yet"),
            // Any other refinement lens (This folder / By type / File / Session / …)
            // that happens to have no members: one calm catch-all.
            (_, "all") => None,
            _ => Some("nothing here"),
        }
    }
}

/// One control-hint action on a picker's dim foot line: a key GLYPH (a bundled
/// symbol like `↵`, an arrow, or a short word like `esc` for a key with no glyph)
/// and the LABEL naming what it does. The DATA half of the foot hint; the
/// consistent SHAPE is [`format_hint`]'s. See [`OverlayKind::hint_actions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HintAction {
    pub glyph: &'static str,
    pub label: &'static str,
}

/// The ONE separator between hint actions — a calm triple space, shared by every
/// picker so the foot line never reads unevenly spaced.
pub const HINT_SEP: &str = "   ";

/// THE CONSCIOUS MARK's picker label: the calm, dim tag a KEPT (pinned) history
/// version wears in the timeline's faint secondary column (see
/// [`OverlayState::new_history`]). A plain word, never a glyph the doc fonts might
/// lack and never amber — figure/ground by value alone.
pub const PIN_TAG: &str = "pinned";

/// Format an ordered list of hint actions into the one canonical foot-hint line:
/// `glyph label   glyph label   …` in move→primary→nav→cancel order. The SINGLE
/// owner of the hint-line shape, so every picker's foot hint reads identically
/// spaced. Each picker supplies only its ordered [`HintAction`] data
/// ([`OverlayKind::hint_actions`], which prepends the universal `↑/↓ move` lead).
pub fn format_hint(actions: &[HintAction]) -> String {
    actions
        .iter()
        .map(|a| format!("{} {}", a.glyph, a.label))
        .collect::<Vec<_>>()
        .join(HINT_SEP)
}
