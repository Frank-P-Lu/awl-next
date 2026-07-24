//! Building a picker/navigator [`OverlayState`] from the caller-gathered
//! [`BuildCtx`] -- the flat pickers (`build`) and one directory LEVEL of a
//! navigable explorer (`browse_level`) -- plus the row-elision helpers picker
//! rendering shares (`elide_path`/`row_split`). Split out of the former
//! `overlay.rs` monolith (2026-07 code-organization pass); every item's path
//! is unchanged -- only the file it lives in moved.

use super::{OverlayKind, OverlayState};
use std::path::Path;

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
    /// Config `linux_keep_emacs` — the per-chord door that keeps a kept chord's
    /// emacs meaning showing (and suppresses the native label it would otherwise
    /// display) in the SAME effective binding column, under `Convention::Linux`
    /// only (see `commands::join_slots_truthful`'s Tier 4). Empty on Mac and on
    /// every headless capture that doesn't pass `--config`.
    pub config_linux_keep: &'a [String],
    /// The CURRENT buffer's markdown headings (depth-indented label + line) for
    /// Go-to's HEADINGS lens (the fold that retired the standalone Outline picker).
    /// Caller-gathered (it needs the live buffer text); EMPTY for a non-markdown
    /// buffer or one with no headings, so the Headings lens simply reads empty.
    pub goto_headings: Vec<(String, usize)>,
    /// The Cmd-`;` spell target — the misspelled word's corrections, its span, AND
    /// its current TEXT — resolved by the caller ONLY when the spell binding fired.
    /// `None` when the cursor isn't on a flagged word (or spell-check is off), so
    /// the summon no-ops. The word text builds the "Add '<word>' to dictionary" row
    /// label + rides the add-row accept effect ([`OverlayState::new_spell`]).
    pub spell_target: Option<(Vec<String>, (usize, usize, usize), String)>,
    /// The HISTORY TIMELINE rows for the current file — [`crate::history::TimelineRow`]
    /// (when / which / counts / id), newest-first — resolved by the caller (via
    /// [`crate::history::timeline_rows`]) ONLY when the History binding fired. EMPTY
    /// otherwise AND when the file has no history yet; an empty list summons the calm
    /// "no history yet" row (History always opens; the Headings lens simply reads empty).
    pub history_entries: Vec<crate::history::TimelineRow>,
    /// The REFERENCE clock (millis) for the History picker's Today lens — `Some`
    /// live, `None` in the headless capture path (so the clock-relative lenses stay
    /// inert, the determinism gate).
    pub history_now: Option<u64>,
    /// The current session's start (millis) for the History picker's Session lens —
    /// `Some` live, `None` headless / untracked.
    pub history_session_start: Option<u64>,
    /// The config/project-derived VALUE inputs for the SETTINGS menu's secondary
    /// column ([`crate::settings::SettingsValues`]). The process-global settings
    /// (theme / page mode / caret / spell / markdown / nits) are read LIVE inside
    /// the readout, so only the config pieces are gathered by the caller — the live
    /// App from `self.config` + root + zoom, the headless replay from its `config`.
    /// Empty [`Default`] for a non-Settings summon (unused there).
    pub settings_values: crate::settings::SettingsValues,
    /// The ASSET CLEANER's scanned ORPHAN list ([`crate::assets::scan`]) — filled by
    /// the caller ONLY when the "Clean unused assets" binding fired (scanning the whole
    /// project tree is pure waste otherwise), EMPTY for every other summon. The live
    /// App AND the headless replay both fill it from the same scan over the
    /// [`crate::fs`] seam, so a `--keys` capture sees the real orphan list.
    pub assets: Vec<crate::assets::Orphan>,
    /// Is a daemon `--wait` client actively waiting on the CURRENT buffer right
    /// now (`crate::daemon`'s module doc, `App::wait_conns`)? The ONE live fact
    /// behind the Command palette's "Finish file" row visibility
    /// (`commands::visible_hidden_mask`) — `true` only on the live App, when the
    /// daemon exists AND some connection is parked waiting. Structurally `false`
    /// in the headless capture/replay path (which never imports `crate::daemon`
    /// at all — the daemon capture gate) and on wasm/`mas` (no daemon compiled),
    /// so a default palette build hides the row deterministically everywhere but
    /// a real `EDITOR=awl --wait` round-trip.
    pub has_waiter: bool,
}

/// Build the SUMMONED overlay for a non-navigable picker kind (Goto / Theme /
/// Command, plus the buffer-scoped Spell) from the caller-gathered [`BuildCtx`].
/// Returns `None` for the navigable explorers (Browse / MoveDest / Project) —
/// those need a directory LEVEL, built by [`browse_level`] — and for an unresolved
/// Spell target, so those summons stay quiet no-ops. Shared by the live App
/// (`app.rs`) and the headless replay (`main.rs`) so both summon byte-identical
/// overlays.
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
            // Fold the current doc's HEADINGS in as the Headings lens's corpus (the
            // retired Outline picker). Appended after the files; empty for a
            // non-markdown buffer (the lens then reads "no headings yet").
            ov.attach_headings(ctx.goto_headings.clone());
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
        // CJK-priority language picker: the four languages + whichever currently
        // sits at the FRONT of the live ladder (pre-selected; nothing previews
        // on move, mirroring Dictionary).
        OverlayKind::CjkLang => Some(OverlayState::new_cjk_lang(
            crate::frontmatter::cjk_priority()
                .first()
                .copied()
                .unwrap_or(crate::frontmatter::Lang::Ja),
        )),
        // Date-format picker: the five formats, EACH row's primary text rendered
        // with `today` (live clock, or the fixed placeholder in a headless
        // capture — the SAME `today_ymd` the Settings "Date format" row previews,
        // so both surfaces agree), the active format pre-selected. Nothing
        // previews on move (the example dates ARE the preview), so no revert
        // bookkeeping — the Dictionary shape.
        OverlayKind::Date => Some(OverlayState::new_date(
            crate::dateformat::active_format(),
            ctx.settings_values.today_ymd,
        )),
        // Command palette: the PLATFORM-FILTERED command catalog
        // (`commands::visible()` — hides desktop-only commands on web; byte-identical
        // to the full catalog on native), each row showing its EFFECTIVE chord (config
        // `[keys]` rebinds included), so it teaches the live binding.
        OverlayKind::Command => {
            let mut ov = OverlayState::new_command(
                crate::commands::visible_names(),
                crate::commands::visible_effective_bindings(ctx.config_keys, ctx.config_linux_keep),
                // RUNTIME gate: "Finish file" only shows while a daemon `--wait`
                // client is actively waiting (see `BuildCtx::has_waiter`'s doc).
                crate::commands::visible_hidden_mask(ctx.has_waiter),
            );
            // The Recent lens reads the in-memory recently-run MRU (empty in a fresh
            // process, so headless Recent is inert), translated into VISIBLE-CORPUS
            // indices (`visible_recent_indices`) so it can never point at a hidden row.
            ov.recent = crate::commands::visible_recent_indices();
            // THE UNION ROUND: the SETTINGS corpus joins the palette — appended after
            // the commands (mirrors Go-to's headings-after-files convention), so the
            // flat All lens fuzzy-ranks commands + settings together while the File/
            // Edit/View/Recent lenses (which bucket by `menu_section`/`recent`, neither
            // of which any setting name matches) naturally exclude them, no bucket code
            // needed. Same platform-filtered value readout the Settings menu itself
            // opens with, so a setting reached via the palette shows the identical
            // current-value secondary cell.
            //
            // ONE PALETTE DOOR PER DESTINATION (the union round's own follow-up fix):
            // `palette_names`/`palette_value_cells` are `visible_names`/
            // `visible_value_cells` MINUS every row whose covering command
            // (`settings::COVERED_BY`) is available on this platform — e.g. "Theme" is
            // excluded here because "Switch theme…" already opens the identical
            // `OverlayKind::Theme`, the exact door-duplication the user reported
            // ("what's the difference between the new theme option and the switch
            // theme option???"). A covered row stays fully reachable from the Settings
            // menu itself, which reads `visible_names`/`visible_value_cells` unfiltered.
            ov.attach_settings_rows(
                crate::settings::palette_names(),
                crate::settings::palette_value_cells(&ctx.settings_values),
            );
            Some(ov)
        }
        // Rebind menu: the same platform-filtered command catalog + effective chords
        // as the palette, but opened in capture mode (Enter rebinds rather than runs).
        OverlayKind::Keybindings => Some(OverlayState::new_keybindings(
            crate::commands::visible_names(),
            crate::commands::visible_effective_bindings(ctx.config_keys, ctx.config_linux_keep),
        )),
        // Spell: the caller-resolved word target + its corrections. None when the
        // cursor isn't on a flagged word, so the summon no-ops.
        OverlayKind::Spell => ctx
            .spell_target
            .clone()
            .map(|(sugg, target, word)| OverlayState::new_spell(sugg, target, word)),
        // History: the caller-gathered timeline rows. ALWAYS summons: an empty list
        // becomes the calm "no history yet" row, so the picker never silently no-ops
        // on a file that simply hasn't been snapshotted yet.
        OverlayKind::History => Some(OverlayState::new_history(
            ctx.history_entries.clone(),
            ctx.history_now,
            ctx.history_session_start,
        )),
        // Settings menu: the flat settings corpus (display names) + each setting's
        // current VALUE in the secondary (binding) column, read via the settings
        // readout against the caller-gathered config/project values. It FACETS by
        // category (the scheme is registered), so it lands on the flat All home and
        // ←/→ step through the category lenses. Always summons.
        OverlayKind::Settings => {
            let mut ov = OverlayState::new(
                kind,
                crate::settings::visible_names(),
                Vec::new(),
                Vec::new(),
            );
            ov.set_secondaries(crate::settings::visible_value_cells(&ctx.settings_values));
            Some(ov)
        }
        // Asset cleaner: the caller-scanned orphan list. ALWAYS summons (like
        // History): an empty list becomes the calm "no unused assets" row.
        OverlayKind::Assets => Some(OverlayState::new_assets(ctx.assets.clone())),
        // Navigable explorers open via `browse_level` (they need a dir level).
        OverlayKind::Browse | OverlayKind::MoveDest | OverlayKind::Project => None,
        // NOTES VERBS round: the Rename minibuffer is built directly at its
        // `Action::OpenRenameNote` apply_core arm (`OverlayState::new_rename`) — it
        // needs only the buffer's own path, no caller-gathered context — so this
        // generic builder never constructs one. This arm exists for exhaustiveness.
        OverlayKind::Rename => None,
        // LINKS V2: the InsertLink minibuffer is built directly at its
        // `Action::InsertLink` apply_core arm (`link::open_insert_link` →
        // `OverlayState::new_link_edit`) — it needs only the buffer's own
        // selection/cursor/text, no caller-gathered context — so this generic
        // builder never constructs one. This arm exists for exhaustiveness.
        OverlayKind::InsertLink => None,
        // NAMED SAVE POINTS: the Keep-version minibuffer is built directly at
        // its `Action::KeepVersion` apply_core arm (`OverlayState::new_keep_name`)
        // — it needs no caller-gathered context at all (the prompt opens empty) —
        // so this generic builder never constructs one. Exhaustiveness arm.
        OverlayKind::KeepName => None,
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
///
/// `recent_projects` is the persisted recent-PROJECTS MRU (absolute paths,
/// newest-first) — passed straight through to [`OverlayState::new_project`] so the
/// Project navigator's **Recent** lens can mark the folders you've switched to. It
/// is EMPTY for the other kinds (they have no Recent lens) and in the headless
/// replay (the determinism gate — recents is live-only persisted state).
pub fn browse_level(
    kind: OverlayKind,
    rel: Option<String>,
    active_root: &Path,
    notes_root: &Path,
    workspace: Option<&Path>,
    recent_projects: &[String],
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
        return Some(OverlayState::new_project(dir, folders, recent_projects));
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
    // THE UNION ROUND: a settings row's marker PREFIX (`"§ "`, `OverlayKind::
    // SETTINGS_MARKER_PREFIX`) is figure/ground-split exactly like a directory
    // prefix — the glyph recedes to muted ink, the setting name stays content ink.
    // Checked first (a setting name never itself contains a `/`).
    if row.starts_with(OverlayKind::SETTINGS_MARKER_PREFIX) {
        return OverlayKind::SETTINGS_MARKER_PREFIX.len();
    }
    // ITEM 11: a Go-to HEADING row's marker PREFIX (`"❡ "`, `OverlayKind::
    // HEADING_MARKER_PREFIX`) is figure/ground-split the same way — the glyph
    // recedes to muted ink, the (indented) title stays content ink. Checked next
    // (a heading title never itself starts with the settings glyph).
    if row.starts_with(OverlayKind::HEADING_MARKER_PREFIX) {
        return OverlayKind::HEADING_MARKER_PREFIX.len();
    }
    row.rfind('/').map(|i| i + 1).unwrap_or(0)
}
