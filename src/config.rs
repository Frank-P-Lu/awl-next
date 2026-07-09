//! The PERSISTENT CONFIG: awl's settings live in a text file you edit AS TEXT in
//! awl itself (the Settings command opens it; `Cmd-S` saves + live-reloads). The
//! file is TOML at `$XDG_CONFIG_HOME/awl/config.toml` (or `~/.config/awl/...`):
//!
//! ```toml
//! notes_root = "~/notes"
//! workspace  = "~/code"
//! [keys]
//! save         = ["Cmd-S", "C-x C-s"]  # up to 2 chords: native + your own emacs
//! switch_theme = "Cmd-T"               # a single chord still works
//! ```
//!
//! Every command takes UP TO 2 bindings (slot 1 = NATIVE/macOS, slot 2 = EMACS);
//! both fire. A `[keys]` value is therefore a LIST of up to 2 chords, or a single
//! string (the old form) for a one-chord rebind.
//!
//! PRECEDENCE is always explicit CLI flag > config file > built-in default, so an
//! ABSENT config (or any absent field) reproduces the current defaults exactly —
//! loading is purely additive and never changes behaviour on its own. The keymap
//! consumes [`Config::keys`] (see `keymap::KeymapState::with_overrides`); `main` /
//! `app` fold `notes_root`/`workspace` into the existing `resolve_*` paths.

use std::path::{Path, PathBuf};

/// The loaded settings. Every field is OPTIONAL: `None`/empty means "absent",
/// which the resolution paths read as "fall back to the built-in default", so a
/// missing config file is indistinguishable from the old hardcoded behaviour.
pub struct Config {
    /// `notes_root` (quick-notes home for C-x n / C-x m). `None` = default `~/notes`.
    pub notes_root: Option<PathBuf>,
    /// `workspace` (switch-project parent for C-x p). `None` = default `root.parent`.
    pub workspace: Option<PathBuf>,
    /// STICKY PREFERENCES — the launch state the editor REMEMBERS across runs. Each
    /// is a genuine preference set by a state-changing action (theme cycle, zoom,
    /// page toggle, caret toggle), persisted on change and restored on launch. `None`
    /// = absent → the built-in default (so an empty config reproduces the defaults).
    /// Ephemeral session states (the while-writing toggles) are NOT here.
    ///
    /// `theme` — the last-selected world NAME (e.g. `"Quokka"`); `None` = default world.
    pub theme: Option<String>,
    /// `zoom` — the last zoom factor; `None` = the first-run default (`0.8`).
    pub zoom: Option<f32>,
    /// `page_mode` — page mode on/off; `None` = the built-in default (on).
    pub page_mode: Option<bool>,
    /// `page_width_prose` — the centered writing column's MEASURE in characters
    /// for a PROSE buffer (markdown / the no-path scratch-or-note surface / an
    /// unrecognized plain-text file), adjusted by the Widen page / Narrow page
    /// commands while a prose buffer is active; `None` = the built-in default
    /// ([`crate::page::DEFAULT_MEASURE`], ~70). See `page_width_code` for the CODE
    /// counterpart and [`crate::page::PageClass`] for which applies to the ACTIVE
    /// buffer. Zoom is decoupled from both: zoom scales the glyphs, these scale
    /// the column. The RETIRED single `page_width` key (this pair's predecessor)
    /// is simply an unknown key to the lenient loader now — silently inert, never
    /// migrated.
    pub page_width_prose: Option<usize>,
    /// `page_width_code` — the CODE counterpart to `page_width_prose`: the
    /// column MEASURE while a recognized syntax-highlighted file is active;
    /// `None` = the built-in default ([`crate::page::DEFAULT_MEASURE_CODE`],
    /// ~100 — rustfmt's own `max_width` convention).
    pub page_width_code: Option<usize>,
    /// `caret_mode` — the caret look NAME (`"block"`/`"morph"`/`"ibeam"`); `None` =
    /// the font-derived default.
    pub caret_mode: Option<String>,
    /// `dictionary` — the active spell-check dictionary NAME (`"en_US"`/`"en_GB"`/
    /// `"en_AU"`); `None` = the built-in default (`en_US`), so an absent key
    /// reproduces the historical single-dictionary behaviour byte-identically.
    pub dictionary: Option<String>,
    /// `writing_nits` — the quiet mechanical-typo underline highlighter on/off;
    /// `None` = the built-in default (ON, like spellcheck — it is quiet + helpful).
    pub writing_nits: Option<bool>,
    /// `spellcheck` — the GLOBAL spell-check on/off (the escape hatch for
    /// no-squiggles-ever people); `None` = the built-in default (ON). OFF silences
    /// every squiggle — prose AND the scoped code-string/comment check alike — and
    /// turns the spell-suggest picker (Cmd-`;` / right-click) into a calm no-op.
    /// Toggled by the "Toggle spellcheck" palette command; see `spell.rs`.
    pub spellcheck: Option<bool>,
    /// `history` — automatic LOCAL SNAPSHOTS on save for LOOSE (non-git) files
    /// on/off; `None` = the built-in default (ON). A file inside a git repo is
    /// never snapshotted regardless (git owns its versioning — see
    /// [`crate::history`]); this only gates the loose-file store.
    pub history: Option<bool>,
    /// `project_root` — the ACTIVE PROJECT ROOT last selected via switch-project
    /// (C-x p), write-on-change like theme/caret/page (not a hand-edited folder
    /// default like `notes_root`/`workspace`). `None` = no remembered project
    /// (today's default: derive from the launch file's directory, or cwd).
    /// Restored ONLY on a BARE launch — no file argument AND no explicit
    /// `--root` — mirroring the scratch-buffer stash's exact restore condition
    /// (see `resolve_root` in `main/run.rs`); opening a specific file still
    /// scopes to that file's own directory, unaffected.
    pub project_root: Option<PathBuf>,
    /// `autosave` — the quiet write-on-idle/blur/switch/quit engine on/off;
    /// `None` = the built-in default (ON). Gates the live App's idle autosave,
    /// the blur/switch/quit flushes, and the scratch-buffer stash — never the
    /// headless capture, which is structurally autosave-free.
    pub autosave: Option<bool>,
    /// `wysiwyg` — the markdown CONCEAL-on-cursor amendment on/off ("if the caret
    /// is on that line, show the actual markdown; otherwise show the preview" —
    /// headings/bold/italic/inline-code/`==highlight==` markup hides off the
    /// caret's line, plus a fenced block's marker lines off the caret's whole
    /// block); `None` = the built-in default (ON, like autosave/spellcheck — no
    /// CLI flag). OFF reproduces today's always-visible markup byte-identically
    /// (no conceal, no inline-code pill, no fenced-block panel — see `markdown.rs`).
    pub wysiwyg: Option<bool>,
    /// `inline_images` — render a markdown `![alt](path.png)` reference as the
    /// decoded IMAGE in a tall fit-to-column row (its source concealing off the
    /// caret's line), rather than plain source text; `None` = the built-in
    /// default (ON, like wysiwyg — no CLI flag). OFF renders the `![alt](path)`
    /// source as plain text byte-identically to the pre-feature editor. NATIVE-
    /// ONLY: the feature is unconditionally off on wasm (see
    /// [`crate::markdown::inline_images_on`]), so this pref is inert there.
    pub inline_images: Option<bool>,
    /// `code_ligatures` — CODE-buffer PROGRAMMING ligatures (the arrow / `!=` /
    /// `=>` / `::` glyphs the pitch-safe monos ship, riding `calt`) on/off;
    /// `None` = the built-in default (ON, like wysiwyg — no CLI flag). OFF renders
    /// code ligature-free for every mono. Gates ONLY code — PROSE standard fi/fl
    /// ligatures are always on regardless (see `crate::render::text::font_features`).
    /// Applied to the `crate::render::CODE_LIGATURES_ON` process-global at launch
    /// (`apply_sticky_globals`) and flipped live by the settings menu.
    pub code_ligatures: Option<bool>,
    /// `cjk_priority` — the i18n round's Han-ambiguity TIEBREAK ladder: an
    /// ordered list of BCP 47 tags (`crate::frontmatter::Lang`) consulted ONLY
    /// when a document/run's dominant CJK script is bare Han (ambiguous among
    /// ja/zh-Hans/zh-Hant/ko — kana/hangul/bopomofo are unambiguous and never
    /// consult this). `None` (or an empty/all-unrecognized list) = the built-in
    /// default `["ja", "zh-Hans", "zh-Hant", "ko"]`
    /// ([`crate::frontmatter::DEFAULT_CJK_PRIORITY`]). Read by the live App's
    /// write-back-once doc-lang detector (`app/files.rs`) and available to the
    /// render resolution ladder; unrecognized tags in the list are simply
    /// skipped (never a crash).
    pub cjk_priority: Option<Vec<crate::frontmatter::Lang>>,
    /// `session_restore` — reopen the previous SESSION on a plain relaunch:
    /// every open file, which one was active, each file's remembered
    /// cursor/scroll, and (native only) the window frame; `None` = the
    /// built-in default (ON, like autosave/history/wysiwyg — the settings-
    /// discipline escape hatch, not a chrome toggle). OFF makes the engine
    /// vanish BOTH ways: nothing is ever written on quit/blur, and the
    /// session file is never read back at launch. See `session.rs` /
    /// `app/session.rs`.
    pub session_restore: Option<bool>,
    /// `outline` — the persistent margin table-of-contents on/off; `None` = the
    /// built-in default (ON, like the other sticky toggles — flipped 2026-07-09,
    /// a user-decided taste reversal of the original opt-in-off call; see
    /// `outline.rs`'s module doc). A config `outline = false` still wins, either
    /// direction. Applied at launch to the `outline::OUTLINE_ON` process-global
    /// (`apply_sticky_globals`), flipped live by the "Toggle outline" command /
    /// settings menu, and read by the renderer + capture sidecar each reshape.
    pub outline: Option<bool>,
    /// `menu_bar` — the awl-RENDERED menu bar on/off (`menubar.rs`). `None` = the
    /// PLATFORM default: ON for web/Linux (where the OS gives no chrome), effectively
    /// absent on macOS (the native NSMenu bar is the door — the global defaults OFF
    /// there, and the awl bar draws nothing). A config `menu_bar = false` hides it on
    /// web/Linux (a user-settled requirement); `menu_bar = true` even forces it on
    /// macOS. Applied at launch to the `menubar::MENU_BAR_ON` process-global
    /// (`apply_sticky_globals`), flipped live by the "Toggle menu bar" command /
    /// settings menu, and read by the renderer + capture sidecar each frame.
    pub menu_bar: Option<bool>,
    /// `typewriter_scroll` — pin the caret's row centered so the document scrolls
    /// under a stationary caret (iA Writer-style); `None` = the built-in default
    /// (OFF, opt-in — unlike the outline, still a
    /// scroll behavior the user turns ON, not a chrome default). Applied at launch to the
    /// `typewriter::TYPEWRITER_ON` process-global (`apply_sticky_globals`), flipped
    /// live by the "Toggle typewriter scroll" command / settings menu, and read by
    /// `sync_view`'s cursor-follow + the capture scroll computation.
    pub typewriter_scroll: Option<bool>,
    /// `stats` — the LIFETIME STATS odometer (chars typed, keystrokes, active-
    /// writing time, files touched, caret travel, per-world time) on/off; `None`
    /// = the built-in default (ON, like autosave/session_restore — a quiet
    /// personal, LOCAL + PRIVATE odometer, never uploaded). OFF makes the engine
    /// vanish: no tracking, no `stats.toml` writes. Native-only (wasm no-op); read
    /// only by the live `App`, so it can never affect a headless capture. See
    /// `stats.rs`.
    pub stats: Option<bool>,
    /// The `[keys]` table as (action-name, chords) pairs, in file order. Each value
    /// is a LIST of up to 2 chords — conceptually slot 1 = NATIVE (macOS), slot 2 =
    /// EMACS — and the keymap parses each chord and OVERRIDES that named action's
    /// binding (additively; both fire). A single TOML string (`save = "C-x C-s"`)
    /// loads as a one-element list, so the old one-chord form stays back-compatible.
    pub keys: Vec<(String, Vec<String>)>,
    /// Where this config loaded from (the Settings command's open target). Empty
    /// for [`Config::empty`] (a non-file placeholder).
    pub path: PathBuf,
}

/// The commented template written on the FIRST Settings-open when no config
/// exists, so the user lands in a self-documenting file rather than a blank one.
pub const DEFAULT_TEMPLATE: &str = "\
# awl config — edit as text, then Cmd-S to save (live-reloads keys + folders).
#
# notes_root : where Cmd-N quick-notes live          (default: ~/notes)
# workspace  : the parent dir whose children Cmd-Shift-P switches between
#                                                     (default: the project's parent)
#
# [keys] : rebind a command. The ACTION NAME is the command-palette name
#   lower-cased with spaces as underscores (go_to_file, switch_theme, save,
#   new_note, ...). Every command takes UP TO 2 bindings — slot 1 = NATIVE
#   (macOS Cmd), slot 2 = EMACS — and BOTH fire, so a value is a LIST of up to
#   two chords. A single string is the one-chord form. A CHORD is a key spec:
#   \"Cmd-S\", \"C-t\", \"M-g\", or \"C-x g\" (the C-x prefix plus one key) —
#   modifiers: Cmd-/s- = Super, C- = Ctrl, M-/Option- = Meta, S- = Shift. A bad
#   chord is ignored and the default kept. Open Cmd-P to see each command's name
#   + both effective chords, or Cmd-P -> \"Keybindings…\" to rebind by PRESSING the
#   key (it writes this table for you).

# notes_root = \"~/notes\"
# workspace = \"~/code\"

# STICKY PREFERENCES — awl REMEMBERS these across launches and rewrites them here
# whenever you change them live (no settings menu; the action IS the setting). You
# can also hand-edit them. Absent = the built-in default.
#   theme      : the world to launch in (Tawny, Quokka, Gumtree, ...) — set by Cmd-T
#   zoom       : the launch zoom factor (default 0.8) — set by Cmd-= / Cmd--
#   page_mode  : centered page column on/off (default on) — toggled by its command
#   page_width_prose : the writing column MEASURE in characters for a PROSE buffer
#                (markdown / the scratch-or-note surface / an unrecognized plain-text
#                file) — default 70. Set by the Widen page / Narrow page commands
#                while a prose buffer is active.
#   page_width_code : the writing column MEASURE in characters for a CODE buffer
#                (a recognized syntax-highlighted file) — default 100 (rustfmt's own
#                max_width). Which one applies follows the ACTIVE buffer's own kind;
#                zoom is DECOUPLED from both — zoom sizes the glyphs, these size the
#                column.
#   caret_mode : caret look (block | morph | ibeam) — set by the Caret style… /
#                Toggle caret style commands
#   dictionary : spell-check dictionary (en_US | en_GB | en_AU) — default en_US;
#                set via Cmd-P -> \"Dictionary…\"
#   writing_nits : the quiet mechanical-typo underline highlighter on/off
#                (default on) — toggled by the \"Toggle writing nits\" palette command
#   spellcheck : the GLOBAL spell-check on/off (default on) — OFF silences every
#                squiggle (prose and code strings/comments alike) and turns the
#                spell-suggest picker into a calm no-op — toggled by the
#                \"Toggle spellcheck\" palette command
#   history    : automatic LOCAL SNAPSHOTS on save for LOOSE (non-git) files
#                (default on), pruned by the aged retention ladder (resolution
#                thins with age; memory is kept). A file inside a git repo is
#                never snapshotted — git owns its versioning; the timeline reads
#                git history instead.
#   autosave   : quietly SAVE the open file on idle (~1s after you stop typing),
#                window blur, file switch, and quit (default on). Writes are atomic
#                and never overwrite a file changed outside awl (a calm notice instead).
#                The unsaved scratch buffer stashes + restores across launches.
#   project_root : the project folder a BARE launch (no file argument) reopens —
#                set automatically by switch-project (C-x p); an explicit --root
#                flag always wins over this.
#   wysiwyg    : conceal markdown markup off the caret's line (default on) — a
#                heading's `#`, bold/italic `**`/`*`/`_`, inline `` ` `` backticks,
#                and `==highlight==` marks hide until the caret lands on that
#                line; a fenced code block's marker lines hide until the caret is
#                anywhere inside the block. Set false for today's always-visible
#                markup.
#   inline_images : render a markdown `![alt](img.png)` reference as the decoded
#                image in a tall fit-to-column row — its source concealing off the
#                caret's line — instead of plain text (default on, native only).
#                An Obsidian `![alt|300](img.png)` width hint sizes it.
#   code_ligatures : programming ligatures (-> => != >= :: |>) in CODE buffers on
#                the pitch-safe monos (JetBrains Mono, Iosevka) — default on. Set
#                false for ligature-free code. Prose fi/fl ligatures are always on.
#   cjk_priority : the Han-ambiguity tiebreak ladder (default [\"ja\", \"zh-Hans\",
#                \"zh-Hant\", \"ko\"]) — consulted ONLY when an untagged document's
#                CJK content is bare Han (kanji/hanzi with no kana/hangul/bopomofo
#                to disambiguate it); an unrecognized tag in the list is skipped.
#                Used by the write-back-once doc-language tagger on first open of
#                an untagged CJK document (adds a `---\\nlang: ..\\n---` frontmatter
#                block as one undoable edit) and by the per-run render ladder.
#   session_restore : reopen the previous session on a plain relaunch — every
#                open file, the active one, each file's cursor/scroll, and the
#                native window frame (default on). OFF disables both writing
#                the session file (on quit/blur) and reading it back.
#   outline    : the persistent margin table-of-contents (default on) — a faint
#                marginalia TOC that tracks the section you are in.
#   menu_bar   : the awl-rendered menu bar across the top (web/Linux only, default
#                on there; absent on macOS, which has the native menu bar).
#   typewriter_scroll : pin the caret's line centered so the document scrolls
#                under a stationary caret (default OFF, opt-in) — iA Writer-style;
#                the caret rides the doc edges naturally (no centering above the
#                top / below the bottom).
#   stats      : the lifetime stats odometer — chars typed, keystrokes, active-
#                writing time, files touched, caret travel, per-world time
#                (default on). LOCAL + PRIVATE, never uploaded. Native-only. OFF
#                disables all tracking and never writes stats.toml.
# theme = \"Tawny\"
# zoom = 0.8
# page_mode = true
# page_width_prose = 70
# page_width_code = 100
# caret_mode = \"block\"
# dictionary = \"en_US\"
# writing_nits = true
# spellcheck = true
# history = true
# autosave = true
# project_root = \"~/code/my-project\"
# wysiwyg = true
# inline_images = true
# code_ligatures = true
# cjk_priority = [\"ja\", \"zh-Hans\", \"zh-Hant\", \"ko\"]
# session_restore = true
# outline = true
# menu_bar = true
# typewriter_scroll = false
# stats = true

[keys]
# save = [\"Cmd-S\", \"C-x C-s\"]
# go_to_file = \"Cmd-O\"
# switch_theme = \"Cmd-T\"
";

impl Config {
    /// A NON-FILE placeholder config (all defaults, empty path). Used by capture
    /// modes that take no `--config` so they share the one `replay_keys` seam.
    pub fn empty() -> Self {
        Config {
            notes_root: None,
            workspace: None,
            theme: None,
            zoom: None,
            page_mode: None,
            page_width_prose: None,
            page_width_code: None,
            caret_mode: None,
            dictionary: None,
            writing_nits: None,
            spellcheck: None,
            history: None,
            autosave: None,
            project_root: None,
            wysiwyg: None,
            inline_images: None,
            code_ligatures: None,
            cjk_priority: None,
            session_restore: None,
            outline: None,
            menu_bar: None,
            typewriter_scroll: None,
            stats: None,
            keys: Vec::new(),
            path: PathBuf::new(),
        }
    }

    /// Whether AUTOMATIC LOCAL SNAPSHOTS are enabled for loose (non-git) files.
    /// Absent = the built-in default (ON) — a loose note/draft keeps a git-free
    /// local history. Read by the save-hook ([`crate::history::record`]).
    pub fn history_on(&self) -> bool {
        self.history.unwrap_or(true)
    }

    /// Whether the quiet AUTOSAVE engine (write on idle / blur / file switch /
    /// quit, plus the scratch-buffer stash) is enabled. Absent = the built-in
    /// default (ON). Read only by the live `App` — the headless capture never
    /// constructs the autosave machinery, so this can't affect a screenshot.
    pub fn autosave_on(&self) -> bool {
        self.autosave.unwrap_or(true)
    }

    /// Whether the SESSION RESTORE engine (persist + reopen the previous
    /// open-file set / active buffer / cursor+scroll / window frame) is
    /// enabled. Absent = the built-in default (ON). Read only by the live
    /// `App` (`app/session.rs`) — the headless capture never constructs the
    /// session machinery, so this can't affect a screenshot.
    pub fn session_restore_on(&self) -> bool {
        self.session_restore.unwrap_or(true)
    }

    /// Whether the LIFETIME STATS odometer (see `stats.rs`) tracks + persists.
    /// Absent = the built-in default (ON). Read only by the live `App`'s native
    /// tracking hooks — the headless capture never constructs them, so this can
    /// never affect a screenshot.
    pub fn stats_on(&self) -> bool {
        self.stats.unwrap_or(true)
    }

    /// Whether the persistent MARGIN OUTLINE is enabled (the STORED pref, used to
    /// seed the `outline::OUTLINE_ON` global at launch + read by the settings menu).
    /// Absent = the built-in default (ON, like the other sticky toggles — flipped
    /// 2026-07-09, see `outline.rs`'s module doc). The renderer/sidecar read the
    /// live global (`crate::outline::outline_on`), which this seeds and the
    /// toggles keep in step.
    pub fn outline_on(&self) -> bool {
        self.outline.unwrap_or(true)
    }

    /// Whether the awl-RENDERED menu bar is enabled (the STORED pref, used to seed the
    /// `menubar::MENU_BAR_ON` global at launch + read by the settings menu). Absent =
    /// the PLATFORM default: ON for web/Linux, OFF for macOS (native NSMenu bar is the
    /// door — matching `menubar::MENU_BAR_ON`'s own `cfg`-derived default). The
    /// renderer/sidecar read the live global (`crate::menubar::menu_bar_on`), which
    /// this seeds and the toggles keep in step.
    pub fn menu_bar_on(&self) -> bool {
        self.menu_bar.unwrap_or(cfg!(not(target_os = "macos")))
    }

    /// The EFFECTIVE `cjk_priority` ladder: the configured list if present AND
    /// non-empty (an explicit-but-all-garbage list is treated the same as
    /// absent — it must never leave the ladder empty and non-functional),
    /// else the built-in default `[Ja, ZhHans, ZhHant, Ko]`
    /// ([`crate::frontmatter::DEFAULT_CJK_PRIORITY`]). Read by the live App's
    /// write-back-once doc-lang detector.
    pub fn cjk_priority_or_default(&self) -> Vec<crate::frontmatter::Lang> {
        match &self.cjk_priority {
            Some(v) if !v.is_empty() => v.clone(),
            _ => crate::frontmatter::DEFAULT_CJK_PRIORITY.to_vec(),
        }
    }

    /// The EFFECTIVE page-width MEASURE for `class`: the configured override
    /// (`page_width_prose`/`page_width_code`) if present, else that class's own
    /// built-in default ([`crate::page::PageClass::default_measure`]). The ONE
    /// place every reader of "what measure applies to a buffer of this kind"
    /// goes through — the initial launch apply (`Self::apply_sticky_globals`),
    /// the live App's buffer-switch resync (`App::sync_page_measure`), and the
    /// headless `--keys` Goto switch — so the three can never disagree.
    pub fn measure_for(&self, class: crate::page::PageClass) -> usize {
        let configured = match class {
            crate::page::PageClass::Prose => self.page_width_prose,
            crate::page::PageClass::Code => self.page_width_code,
        };
        configured.unwrap_or_else(|| class.default_measure())
    }

    /// Load settings from `path`. A MISSING or unreadable file yields a pure-defaults
    /// config bound to `path` (so Settings can still create it) — never an error,
    /// never a behaviour change. A PARSE error is reported to stderr and likewise
    /// degrades to defaults, so a half-edited config never crashes the editor.
    pub fn load(path: PathBuf) -> Self {
        let mut cfg = Config {
            notes_root: None,
            workspace: None,
            theme: None,
            zoom: None,
            page_mode: None,
            page_width_prose: None,
            page_width_code: None,
            caret_mode: None,
            dictionary: None,
            writing_nits: None,
            spellcheck: None,
            history: None,
            autosave: None,
            project_root: None,
            wysiwyg: None,
            inline_images: None,
            code_ligatures: None,
            cjk_priority: None,
            session_restore: None,
            outline: None,
            menu_bar: None,
            typewriter_scroll: None,
            stats: None,
            keys: Vec::new(),
            path,
        };
        let src = match crate::fs::active().read_to_string(&cfg.path) {
            Ok(s) => s,
            Err(_) => return cfg, // absent/unreadable: pure defaults, no behaviour change
        };
        let table: toml::Table = match src.parse() {
            Ok(t) => t,
            Err(e) => {
                eprintln!(
                    "config {}: parse error: {e}; using defaults",
                    cfg.path.display()
                );
                return cfg;
            }
        };
        if let Some(s) = table.get("notes_root").and_then(|v| v.as_str()) {
            cfg.notes_root = Some(expand_tilde(s));
        }
        if let Some(s) = table.get("workspace").and_then(|v| v.as_str()) {
            cfg.workspace = Some(expand_tilde(s));
        }
        // STICKY PROJECT ROOT — a path like notes_root/workspace above, but
        // WRITE-ON-CHANGE (persisted by `App::persist_project_root` on every
        // switch-project commit) rather than hand-edited only. See the field doc.
        if let Some(s) = table.get("project_root").and_then(|v| v.as_str()) {
            cfg.project_root = Some(expand_tilde(s));
        }
        // STICKY PREFERENCES (theme/zoom/page/caret). Each is read leniently — a
        // wrong-typed value is simply ignored (stays None → the built-in default),
        // never an error, matching the rest of the additive load. `zoom` accepts a
        // TOML float OR integer; `page_mode` a bool.
        if let Some(s) = table.get("theme").and_then(|v| v.as_str()) {
            cfg.theme = Some(s.to_string());
        }
        if let Some(z) = table.get("zoom").and_then(toml_as_f32) {
            cfg.zoom = Some(z);
        }
        if let Some(b) = table.get("page_mode").and_then(|v| v.as_bool()) {
            cfg.page_mode = Some(b);
        }
        // `page_width_prose` / `page_width_code` are character counts: accept a
        // TOML integer (or a float that rounds), floored at 1 so a stray 0 never
        // collapses the column. The RETIRED single `page_width` key (this pair's
        // predecessor) is simply an unknown key to this lenient loader now — a
        // stale line in an existing config is silently inert, never migrated
        // (no users but the author himself).
        if let Some(w) = table.get("page_width_prose").and_then(toml_as_usize) {
            cfg.page_width_prose = Some(w.max(1));
        }
        if let Some(w) = table.get("page_width_code").and_then(toml_as_usize) {
            cfg.page_width_code = Some(w.max(1));
        }
        if let Some(s) = table.get("caret_mode").and_then(|v| v.as_str()) {
            cfg.caret_mode = Some(s.to_string());
        }
        if let Some(s) = table.get("dictionary").and_then(|v| v.as_str()) {
            cfg.dictionary = Some(s.to_string());
        }
        if let Some(b) = table.get("writing_nits").and_then(|v| v.as_bool()) {
            cfg.writing_nits = Some(b);
        }
        if let Some(b) = table.get("spellcheck").and_then(|v| v.as_bool()) {
            cfg.spellcheck = Some(b);
        }
        // LOCAL HISTORY: `history` gates the loose-file snapshot store (default on);
        // `autosave` gates the quiet write-on-idle/blur/switch/quit engine (default
        // on). A stale `autosnapshot_secs` line (the retired periodic knob) is
        // simply an unknown key to this lenient loader — silently inert.
        if let Some(b) = table.get("history").and_then(|v| v.as_bool()) {
            cfg.history = Some(b);
        }
        if let Some(b) = table.get("autosave").and_then(|v| v.as_bool()) {
            cfg.autosave = Some(b);
        }
        // WYSIWYG has no CLI flag either (like writing_nits/spellcheck): default on.
        if let Some(b) = table.get("wysiwyg").and_then(|v| v.as_bool()) {
            cfg.wysiwyg = Some(b);
        }
        // INLINE IMAGES: no CLI flag (like wysiwyg): default on (native-only).
        if let Some(b) = table.get("inline_images").and_then(|v| v.as_bool()) {
            cfg.inline_images = Some(b);
        }
        // CODE LIGATURES: no CLI flag (like wysiwyg): default on.
        if let Some(b) = table.get("code_ligatures").and_then(|v| v.as_bool()) {
            cfg.code_ligatures = Some(b);
        }
        // `cjk_priority` — a TOML array of BCP 47 tag strings; unrecognized
        // entries (a typo, a script that isn't one of the five) are simply
        // skipped, never an error (mirrors the rest of this lenient loader).
        if let Some(arr) = table.get("cjk_priority").and_then(|v| v.as_array()) {
            let langs: Vec<crate::frontmatter::Lang> = arr
                .iter()
                .filter_map(|v| v.as_str().and_then(crate::frontmatter::Lang::parse))
                .collect();
            cfg.cjk_priority = Some(langs);
        }
        // SESSION RESTORE has no CLI flag either (like autosave/history): a plain
        // bool kill-switch, default on.
        if let Some(b) = table.get("session_restore").and_then(|v| v.as_bool()) {
            cfg.session_restore = Some(b);
        }
        // `outline` — margin TOC, default ON (surfaced by the settings menu).
        if let Some(b) = table.get("outline").and_then(|v| v.as_bool()) {
            cfg.outline = Some(b);
        }
        // `menu_bar` — the awl-rendered menu bar, default ON on web/Linux + OFF on
        // macOS (platform-derived; surfaced by the settings menu). No CLI flag beyond
        // the capture-only `--menu-bar`.
        if let Some(b) = table.get("menu_bar").and_then(|v| v.as_bool()) {
            cfg.menu_bar = Some(b);
        }
        // `typewriter_scroll` — pin the caret row centered, default OFF (opt-in).
        if let Some(b) = table.get("typewriter_scroll").and_then(|v| v.as_bool()) {
            cfg.typewriter_scroll = Some(b);
        }
        // `stats` — the lifetime odometer, default ON (native-only, LOCAL/PRIVATE).
        if let Some(b) = table.get("stats").and_then(|v| v.as_bool()) {
            cfg.stats = Some(b);
        }
        if let Some(keys) = table.get("keys").and_then(|v| v.as_table()) {
            for (name, val) in keys {
                // A binding is EITHER a single chord string (back-compat) OR a LIST of
                // up to 2 chords (slot 1 = native, slot 2 = emacs). Anything past the
                // first two is dropped — the model is capped at 2. A non-string entry
                // in the list is skipped; a wholly empty value contributes nothing.
                let chords: Vec<String> = match val {
                    toml::Value::String(s) => vec![s.clone()],
                    toml::Value::Array(arr) => arr
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .take(2)
                        .collect(),
                    _ => continue,
                };
                if !chords.is_empty() {
                    cfg.keys.push((name.clone(), chords));
                }
            }
        }
        cfg
    }

    /// Write the commented [`DEFAULT_TEMPLATE`] to `path`, creating parent dirs.
    /// Called by Settings-open when the file does not exist yet.
    pub fn write_default(path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            crate::fs::active().create_dir_all(parent)?;
        }
        crate::fs::active().write(path, DEFAULT_TEMPLATE.as_bytes())
    }

    /// Merge a freshly-captured `binding` into a command's EXISTING config slots,
    /// honouring the 2-binding cap: the new binding goes FIRST (newest wins), prior
    /// slots follow, duplicates (compared CANONICALLY, so `Cmd-S` == `s-s`) drop, and
    /// the list is capped at 2. So rebinding a command twice keeps the two most recent
    /// custom chords; rebinding to an existing slot is idempotent. Pure — the rebind
    /// menu computes the new slot list with this, then persists it via [`write_binding`].
    pub fn merge_slot(existing: &[String], binding: &str) -> Vec<String> {
        let mut out: Vec<String> = vec![binding.to_string()];
        for ch in existing {
            let dup = out.iter().any(|o| {
                crate::keyspec::canonical_binding(o) == crate::keyspec::canonical_binding(ch)
            });
            if !dup {
                out.push(ch.clone());
            }
        }
        out.truncate(2);
        out
    }

    /// PERSIST a `[keys]` rebind to `path`, format-PRESERVINGLY (comments + other
    /// settings survive): `chords = Some([...])` sets the command's slots, `None`
    /// REMOVES the entry (reset-to-default). The matching non-comment `slug = …` line
    /// is replaced in place; a new entry is inserted under the `[keys]` header (added
    /// if absent). A missing file is seeded from [`DEFAULT_TEMPLATE`] first so the
    /// user keeps the documented comments. Used by the rebind menu's commit + reset.
    pub fn write_binding(path: &Path, slug: &str, chords: Option<&[String]>) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            crate::fs::active().create_dir_all(parent)?;
        }
        let src = match crate::fs::active().read_to_string(path) {
            Ok(s) => s,
            Err(_) => DEFAULT_TEMPLATE.to_string(),
        };
        let new_line = chords.map(|cs| {
            let quoted: Vec<String> = cs.iter().map(|c| format!("\"{c}\"")).collect();
            format!("{slug} = [{}]", quoted.join(", "))
        });
        let mut lines: Vec<String> = src.lines().map(str::to_string).collect();
        // An EXISTING uncommented `slug = …` line (whitespace-tolerant), if any.
        let existing = lines.iter().position(|l| {
            let t = l.trim_start();
            !t.starts_with('#')
                && t
                    .strip_prefix(slug)
                    .map(|r| r.trim_start().starts_with('='))
                    .unwrap_or(false)
        });
        match (existing, new_line) {
            // Replace an existing entry's value.
            (Some(i), Some(line)) => lines[i] = line,
            // Remove an existing entry (reset-to-default).
            (Some(i), None) => {
                lines.remove(i);
            }
            // Insert a new entry under [keys] (append the header if it is missing).
            (None, Some(line)) => {
                match lines.iter().position(|l| l.trim() == "[keys]") {
                    Some(h) => lines.insert(h + 1, line),
                    None => {
                        if lines.last().map(|l| !l.trim().is_empty()).unwrap_or(false) {
                            lines.push(String::new());
                        }
                        lines.push("[keys]".to_string());
                        lines.push(line);
                    }
                }
            }
            // Nothing to remove: leave the file untouched.
            (None, None) => return Ok(()),
        }
        let mut out = lines.join("\n");
        out.push('\n');
        crate::fs::active().write(path, out.as_bytes())
    }

    /// LAUNCH-APPLY the remembered THEME / PAGE / CARET onto the process-globals
    /// (`theme::set_active_by_name` / `page::set_page_on` / `caret::set_mode`), so the
    /// editor opens in the state it was last left. Honours flag > config: each
    /// `*_flag` says the matching CLI flag was already supplied (and thus already set
    /// the global), so that pref is SKIPPED — the explicit flag wins. A stale/unknown
    /// remembered theme or caret value is ignored (keeps the built-in default). ZOOM is
    /// deliberately NOT here: it is per-instance, applied via `config.zoom` in
    /// `App::new` (live) and folded into `opts.zoom` (capture). Used by `main` after
    /// the config loads; the windowed + capture paths share this one seam.
    ///
    /// `measure_flag` says the `--measure N` flag already set the page WIDTH global, so
    /// the remembered per-class override is SKIPPED (the explicit flag wins) —
    /// mirroring how `page_flag` gates the remembered `page_mode`. `initial_class`
    /// is the STARTING buffer's [`crate::page::PageClass`] (derived from the launch
    /// `file` argument via `PageClass::of_path` — no `Buffer` exists yet at this call
    /// site), so the remembered `page_width_prose`/`page_width_code` resolves to the
    /// class that actually matters for the very first frame. A later buffer SWITCH
    /// (live `App::sync_page_measure`, or the headless `--keys` Goto switch) re-reads
    /// [`Self::measure_for`] against the buffer THEN active, independent of this
    /// initial pin.
    pub fn apply_sticky_globals(
        &self,
        theme_flag: bool,
        page_flag: bool,
        caret_flag: bool,
        measure_flag: bool,
        initial_class: crate::page::PageClass,
    ) {
        if !theme_flag {
            if let Some(name) = self.theme.as_deref() {
                crate::theme::set_active_by_name(name);
            }
        }
        if !page_flag {
            if let Some(on) = self.page_mode {
                crate::page::set_page_on(on);
            }
        }
        if !measure_flag {
            crate::page::set_measure(self.measure_for(initial_class));
        }
        if !caret_flag {
            if let Some(m) = self.caret_mode.as_deref().and_then(parse_caret_mode) {
                crate::caret::set_mode(m);
            }
        }
        // WRITING NITS has no CLI flag (it is a quiet, always-available hint), so the
        // remembered value applies unconditionally when present; absent = the built-in
        // default (ON), which the `nits::NITS_ON` global already carries.
        if let Some(on) = self.writing_nits {
            crate::nits::set_nits_on(on);
        }
        // SPELLCHECK has no CLI flag either (like writing_nits): the remembered
        // on/off applies unconditionally when present; absent = the built-in
        // default (ON), which the `spell::SPELLCHECK_ON` global already carries.
        if let Some(on) = self.spellcheck {
            crate::spell::set_spellcheck_on(on);
        }
        // DICTIONARY has no CLI flag either (like writing_nits): the remembered
        // variant applies unconditionally when present + recognized; absent/unknown
        // leaves the `spell::ACTIVE_VARIANT` global at its built-in default (en_US),
        // so a plain launch — and a default `--screenshot` — stays byte-identical.
        if let Some(v) = self.dictionary.as_deref().and_then(parse_dictionary) {
            crate::spell::set_active_variant(v);
        }
        // WYSIWYG has no CLI flag either (like writing_nits/spellcheck): the
        // remembered on/off applies unconditionally when present; absent = the
        // built-in default (ON), which `markdown::WYSIWYG_ON` already carries.
        if let Some(on) = self.wysiwyg {
            crate::markdown::set_wysiwyg_on(on);
        }
        // INLINE IMAGES: same pattern — the remembered on/off applies when
        // present; absent = the built-in default (ON), which
        // `markdown::INLINE_IMAGES_ON` already carries (and which is inert on
        // wasm, where `inline_images_on()` ignores the flag).
        if let Some(on) = self.inline_images {
            crate::markdown::set_inline_images_on(on);
        }
        // CODE LIGATURES: same pattern (no CLI flag) — the remembered on/off
        // applies when present; absent = the built-in default (ON), which
        // `render::CODE_LIGATURES_ON` already carries. Gates only code buffers'
        // programming ligatures; prose fi/fl is always on regardless.
        if let Some(on) = self.code_ligatures {
            crate::render::set_code_ligatures_on(on);
        }
        // PERSISTENT MARGIN OUTLINE: like the toggles above, the built-in default
        // is ON (`outline::OUTLINE_ON` starts true — flipped 2026-07-09, a
        // user-decided taste reversal of the original opt-in-off call; see
        // `outline.rs`'s module doc). A remembered value applies unconditionally
        // when present, EITHER direction (a config `outline = false` still wins);
        // absent leaves the global at its own default (ON), so a plain launch
        // with no config carries the new default forward.
        if let Some(on) = self.outline {
            crate::outline::set_outline_on(on);
        }
        // MENU BAR: the built-in default is PLATFORM-derived (`menubar::MENU_BAR_ON`
        // starts ON for web/Linux, OFF for macOS). A remembered value applies
        // unconditionally when present, EITHER direction (a config `menu_bar = false`
        // hides it on web/Linux); absent leaves the global at its own platform default,
        // so a plain launch with no config carries the right default forward. The
        // `--menu-bar` capture flag sets the global directly (before this runs).
        if let Some(on) = self.menu_bar {
            crate::menubar::set_menu_bar_on(on);
        }
        // TYPEWRITER SCROLL: unlike the outline, still opt-in — the built-in
        // default is OFF (`typewriter::TYPEWRITER_ON` starts false). A remembered
        // value applies unconditionally when present; absent leaves the global
        // OFF, so a plain launch (and a default `--screenshot`) keeps the
        // cursor-follow scroll → byte-identical.
        if let Some(on) = self.typewriter_scroll {
            crate::typewriter::set_typewriter_on(on);
        }
        // CJK AMBIGUITY LADDER: seed the live process global (`frontmatter::
        // cjk_priority()`, read by the Settings menu's "Ambiguous CJK reads as"
        // row) from a configured list, normalized to a well-formed 4-member
        // permutation. Absent config leaves the global at its own built-in
        // default (`DEFAULT_CJK_PRIORITY`), so a plain launch (and a default
        // `--screenshot`) is unaffected. The RENDER ladder is unaffected either
        // way — it stays `self.cjk_priority_or_default()`, read fresh.
        if let Some(v) = &self.cjk_priority {
            crate::frontmatter::set_cjk_priority(v);
        }
    }

    /// PERSIST a TOP-LEVEL scalar PREFERENCE (theme/zoom/page_mode/caret_mode) to
    /// `path`, format-PRESERVINGLY — the same surgical upsert as [`write_binding`]
    /// but for a top-level `key = value`, so comments + the `[keys]` table + the
    /// other prefs survive. `value` is the already-formatted RHS (a quoted string,
    /// a number, or `true`/`false`). This is the WRITE-ON-CHANGE seam: when the user
    /// switches theme / zooms / toggles page / changes caret, the live `App` calls
    /// this with the settled value (zoom DEBOUNCED in `app.rs`).
    ///
    /// A matching UNCOMMENTED top-level `key = …` line (one that precedes any
    /// `[table]` header, so it can't be a key nested inside `[keys]`) is replaced in
    /// place; otherwise the entry is INSERTED just before the first `[table]` header
    /// (keeping it in the top-level table — a top-level key written AFTER `[keys]`
    /// would parse as a member of that table), or appended if the file has no header.
    /// A missing file is seeded from [`DEFAULT_TEMPLATE`] first so the comments stay.
    pub fn write_pref(path: &Path, key: &str, value: &str) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            crate::fs::active().create_dir_all(parent)?;
        }
        let src = match crate::fs::active().read_to_string(path) {
            Ok(s) => s,
            Err(_) => DEFAULT_TEMPLATE.to_string(),
        };
        let new_line = format!("{key} = {value}");
        let mut lines: Vec<String> = src.lines().map(str::to_string).collect();
        // The first `[table]` header — top-level keys must stay strictly above it.
        let first_header = lines
            .iter()
            .position(|l| l.trim_start().starts_with('['));
        let existing = find_top_level_key(&lines, key);
        match existing {
            Some(i) => lines[i] = new_line,
            None => match first_header {
                // Insert just above the first table header so it stays top-level.
                Some(h) => lines.insert(h, new_line),
                // No header at all: append (optionally after a blank separator).
                None => {
                    if lines.last().map(|l| !l.trim().is_empty()).unwrap_or(false) {
                        lines.push(String::new());
                    }
                    lines.push(new_line);
                }
            },
        }
        let mut out = lines.join("\n");
        out.push('\n');
        crate::fs::active().write(path, out.as_bytes())
    }

    /// REMOVE a top-level scalar PREFERENCE entirely, format-preservingly — the
    /// RESET counterpart to [`write_pref`] for an action whose "built-in default"
    /// is expressed by the key's ABSENCE (`None`) rather than by writing the default
    /// value back, so a future default change flows through instead of pinning a
    /// stale value (used by "Reset page width": clearing `page_width_prose` /
    /// `page_width_code` — whichever matches the active buffer's kind — rather
    /// than writing that class's default back). Mirrors [`write_binding`]'s
    /// reset branch. A matching
    /// UNCOMMENTED top-level `key = …` line is deleted; a MISSING file or an ABSENT
    /// key is a silent no-op (nothing to remove) — never an error.
    pub fn remove_pref(path: &Path, key: &str) -> std::io::Result<()> {
        let Ok(src) = crate::fs::active().read_to_string(path) else {
            return Ok(()); // no file: nothing to remove
        };
        let mut lines: Vec<String> = src.lines().map(str::to_string).collect();
        let Some(i) = find_top_level_key(&lines, key) else {
            return Ok(()); // key absent: nothing to remove
        };
        lines.remove(i);
        let mut out = lines.join("\n");
        out.push('\n');
        crate::fs::active().write(path, out.as_bytes())
    }
}

/// Locate an EXISTING uncommented top-level `key = …` line in `lines` — strictly
/// BEFORE any `[table]` header, so `key` can't collide with a same-named entry
/// nested inside e.g. `[keys]`. The shared lookup [`Config::write_pref`] (replace)
/// and [`Config::remove_pref`] (delete) both key off, so the two writers can never
/// disagree on what counts as "the same key" (merge, don't align).
fn find_top_level_key(lines: &[String], key: &str) -> Option<usize> {
    let first_header = lines.iter().position(|l| l.trim_start().starts_with('['));
    lines.iter().enumerate().position(|(i, l)| {
        if let Some(h) = first_header {
            if i >= h {
                return false;
            }
        }
        let t = l.trim_start();
        !t.starts_with('#')
            && t.strip_prefix(key)
                .map(|r| r.trim_start().starts_with('='))
                .unwrap_or(false)
    })
}

/// Format a caret [`crate::caret::CaretMode`] as its config NAME (the value
/// `caret_mode = "…"` stores) — the inverse of [`parse_caret_mode`].
pub fn caret_mode_name(m: crate::caret::CaretMode) -> &'static str {
    match m {
        crate::caret::CaretMode::Block => "block",
        crate::caret::CaretMode::Morph => "morph",
        crate::caret::CaretMode::Ibeam => "ibeam",
    }
}

/// Parse a config `caret_mode` NAME into a [`crate::caret::CaretMode`]
/// (case-insensitive). An unrecognized value → `None` (keep the default).
pub fn parse_caret_mode(s: &str) -> Option<crate::caret::CaretMode> {
    match s.trim().to_ascii_lowercase().as_str() {
        "block" => Some(crate::caret::CaretMode::Block),
        "morph" => Some(crate::caret::CaretMode::Morph),
        "ibeam" => Some(crate::caret::CaretMode::Ibeam),
        _ => None,
    }
}

/// Format a [`crate::spell::DictVariant`] as its config NAME (the value
/// `dictionary = "…"` stores) — the inverse of [`parse_dictionary`]. NOTE this is
/// the underscored wire form (`"en_US"`), distinct from the picker's human
/// [`crate::spell::DictVariant::label`] (`"English (US)"`) — same split as
/// `caret_mode_name` vs `CaretMode::label`.
pub fn dictionary_name(v: crate::spell::DictVariant) -> &'static str {
    match v {
        crate::spell::DictVariant::EnUs => "en_US",
        crate::spell::DictVariant::EnGb => "en_GB",
        crate::spell::DictVariant::EnAu => "en_AU",
    }
}

/// Parse a config `dictionary` NAME into a [`crate::spell::DictVariant`]
/// (case-insensitive, underscore/hyphen-tolerant so `"en-gb"` also resolves).
/// An unrecognized value → `None` (keep the default, en_US).
pub fn parse_dictionary(s: &str) -> Option<crate::spell::DictVariant> {
    match s.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "en_us" => Some(crate::spell::DictVariant::EnUs),
        "en_gb" => Some(crate::spell::DictVariant::EnGb),
        "en_au" => Some(crate::spell::DictVariant::EnAu),
        _ => None,
    }
}

/// Resolve the CONFIG PATH: explicit `--config <path>` wins, then `$AWL_CONFIG`,
/// then `$XDG_CONFIG_HOME/awl/config.toml`, then `~/.config/awl/config.toml`. A
/// last-resort relative path keeps the function total when no HOME is set.
pub fn config_path(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(p) = explicit {
        return p;
    }
    if let Some(p) = std::env::var_os("AWL_CONFIG") {
        return PathBuf::from(p);
    }
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(x).join("awl").join("config.toml");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("awl")
            .join("config.toml");
    }
    PathBuf::from("awl-config.toml")
}

/// Read a TOML number as `f32`, accepting either a float (`0.8`) or an integer
/// (`1`) so a hand-edited `zoom = 1` is not silently dropped. Anything else → None
/// — INCLUDING TOML's literal `nan`/`inf` special floats (and an f64 that
/// overflows the f32 cast to ±inf): a remembered `zoom = nan` would poison every
/// zoom-derived metric, so a non-finite value reads as absent (the built-in
/// default), like any other wrong-typed pref in the lenient load.
fn toml_as_f32(v: &toml::Value) -> Option<f32> {
    v.as_float()
        .map(|f| f as f32)
        .or_else(|| v.as_integer().map(|i| i as f32))
        .filter(|f| f.is_finite())
}

/// Read a TOML number as a `usize` char count, accepting an integer (`80`) or a
/// float that rounds (`80.0`). Negatives / anything else → None.
fn toml_as_usize(v: &toml::Value) -> Option<usize> {
    v.as_integer()
        .and_then(|i| usize::try_from(i).ok())
        .or_else(|| {
            v.as_float()
                .filter(|f| *f >= 0.0)
                .map(|f| f.round() as usize)
        })
}

/// Expand a leading `~/` to `$HOME` so hand-edited paths read naturally. Anything
/// else passes through verbatim.
fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::FileSystem; // bring the trait methods (read_to_string, …) into scope

    #[test]
    fn absent_config_is_all_defaults() {
        let cfg = Config::load(PathBuf::from("/nonexistent/awl/config.toml"));
        assert!(cfg.notes_root.is_none());
        assert!(cfg.workspace.is_none());
        assert!(cfg.keys.is_empty());
    }

    #[test]
    fn load_reads_folders_and_keys() {
        // Routed through the FILESYSTEM SEAM: a HashMap-backed InMemoryFs stands in
        // for the disk, so the load logic is exercised with NO real file (proves the
        // trait swap works + removes the temp-dir dependence).
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let fs = Arc::new(crate::fs::InMemoryFs::new().with_file(
            &p,
            "notes_root = \"/tmp/my-notes\"\nworkspace = \"/tmp/ws\"\n[keys]\nswitch_theme = \"C-t\"\n",
        ));
        crate::fs::with_fs(fs, || {
            let cfg = Config::load(p.clone());
            assert_eq!(cfg.notes_root, Some(PathBuf::from("/tmp/my-notes")));
            assert_eq!(cfg.workspace, Some(PathBuf::from("/tmp/ws")));
            assert_eq!(
                cfg.keys,
                vec![("switch_theme".to_string(), vec!["C-t".to_string()])]
            );
        });
    }

    #[test]
    fn load_reads_two_binding_list_capped_at_two() {
        // A `[keys]` value may be a LIST of up to 2 chords (slot 1 native, slot 2
        // emacs). A single string still loads as a one-element list (back-compat);
        // a 3+ list is capped at the first two.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let fs = Arc::new(crate::fs::InMemoryFs::new().with_file(
            &p,
            "[keys]\nsave = [\"Cmd-S\", \"C-x C-s\"]\nundo = \"Cmd-Z\"\nredo = [\"a\", \"b\", \"c\"]\n",
        ));
        crate::fs::with_fs(fs, || {
            let cfg = Config::load(p.clone());
            let get = |k: &str| cfg.keys.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
            assert_eq!(get("save"), Some(vec!["Cmd-S".to_string(), "C-x C-s".to_string()]));
            assert_eq!(get("undo"), Some(vec!["Cmd-Z".to_string()]));
            // Three chords supplied; the model caps at 2.
            assert_eq!(get("redo"), Some(vec!["a".to_string(), "b".to_string()]));
        });
    }

    #[test]
    fn precedence_flag_beats_config_beats_default() {
        // The resolution rule the wiring uses: flag.or(config). A CLI flag wins;
        // absent flag falls to config; absent both falls to the resolver default.
        let flag = Some(PathBuf::from("/flag"));
        let from_cfg = Some(PathBuf::from("/cfg"));
        assert_eq!(flag.clone().or(from_cfg.clone()), Some(PathBuf::from("/flag")));
        assert_eq!(None.or(from_cfg.clone()), from_cfg);
        assert_eq!(Option::<PathBuf>::None.or(None), None);
    }

    #[test]
    fn malformed_config_degrades_to_defaults() {
        // Through the InMemoryFs seam: a garbage file still degrades to defaults.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/bad.toml");
        let fs = Arc::new(
            crate::fs::InMemoryFs::new().with_file(&p, "this is = = not valid toml [[["),
        );
        crate::fs::with_fs(fs, || {
            let cfg = Config::load(p.clone());
            assert!(cfg.notes_root.is_none() && cfg.workspace.is_none() && cfg.keys.is_empty());
        });
    }

    #[test]
    fn tilde_in_folder_path_expands_to_home() {
        // A `~/x` notes_root resolves against the CURRENT $HOME. We only READ $HOME,
        // but `config_path_env_precedence` MUTATES it, so hold the shared ENV_LOCK to
        // serialize against that writer (otherwise the read races its set_var).
        // Skipped if HOME is unset.
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        // expand_tilde directly...
        assert_eq!(expand_tilde("~/x"), PathBuf::from(&home).join("x"));
        // ...and through the load seam (notes_root + workspace both expand), over the
        // InMemoryFs seam (no temp file).
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let fs = Arc::new(crate::fs::InMemoryFs::new()
            .with_file(&p, "notes_root = \"~/n\"\nworkspace = \"~/w\"\n"));
        crate::fs::with_fs(fs, || {
            let cfg = Config::load(p.clone());
            assert_eq!(cfg.notes_root, Some(PathBuf::from(&home).join("n")));
            assert_eq!(cfg.workspace, Some(PathBuf::from(&home).join("w")));
        });
        // A non-tilde path passes through verbatim.
        assert_eq!(expand_tilde("/abs/x"), PathBuf::from("/abs/x"));
    }

    // Serialize tests that touch the process-global environment (`HOME` etc.):
    // `config_path_env_precedence` MUTATES these vars, and `tilde_…` READS `HOME`, so
    // both hold this lock to avoid a read/write race under parallel test execution.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn config_path_env_precedence() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Snapshot the three vars so the test leaves the environment untouched.
        let snap = ["AWL_CONFIG", "XDG_CONFIG_HOME", "HOME"]
            .map(|k| (k, std::env::var_os(k)));
        let restore = || {
            for (k, v) in &snap {
                // SAFETY: serialized by ENV_LOCK; no other test reads these vars.
                unsafe {
                    match v {
                        Some(val) => std::env::set_var(k, val),
                        None => std::env::remove_var(k),
                    }
                }
            }
        };
        // SAFETY: serialized by ENV_LOCK (see restore()).
        unsafe {
            std::env::set_var("AWL_CONFIG", "/awl/explicit.toml");
            std::env::set_var("XDG_CONFIG_HOME", "/xdg");
            std::env::set_var("HOME", "/home/me");
        }
        // Explicit flag beats everything.
        assert_eq!(config_path(Some(PathBuf::from("/flag.toml"))), PathBuf::from("/flag.toml"));
        // No flag: $AWL_CONFIG wins next.
        assert_eq!(config_path(None), PathBuf::from("/awl/explicit.toml"));
        // No AWL_CONFIG: fall to $XDG_CONFIG_HOME/awl/config.toml.
        unsafe { std::env::remove_var("AWL_CONFIG") };
        assert_eq!(config_path(None), PathBuf::from("/xdg/awl/config.toml"));
        // No XDG either: fall to $HOME/.config/awl/config.toml.
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
        assert_eq!(config_path(None), PathBuf::from("/home/me/.config/awl/config.toml"));
        restore();
    }

    #[test]
    fn merge_slot_caps_at_two_newest_first_dedup() {
        // Newest binding goes first; existing slots follow; canonical duplicates drop.
        assert_eq!(Config::merge_slot(&[], "C-j"), vec!["C-j".to_string()]);
        assert_eq!(
            Config::merge_slot(&["C-x C-s".to_string()], "Cmd-S"),
            vec!["Cmd-S".to_string(), "C-x C-s".to_string()]
        );
        // Re-capturing the same chord (different spelling) is idempotent (no dupe).
        assert_eq!(
            Config::merge_slot(&["s-s".to_string()], "Cmd-S"),
            vec!["Cmd-S".to_string()]
        );
        // A third distinct binding pushes the oldest off (cap 2).
        assert_eq!(
            Config::merge_slot(&["C-a".to_string(), "C-b".to_string()], "C-c"),
            vec!["C-c".to_string(), "C-a".to_string()]
        );
    }

    #[test]
    fn write_binding_sets_replaces_and_resets_preserving_comments() {
        // Full write→read roundtrip over the InMemoryFs seam (no disk): seed a hand-
        // edited config, then set/replace/reset bindings through `Config::write_binding`
        // — which routes its create_dir_all/read/write through `fs::active()`.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let mem = crate::fs::InMemoryFs::new().with_file(
            &p,
            "# my notes\nnotes_root = \"/tmp/n\"\n[keys]\nswitch_theme = \"C-t\"\n",
        );
        let fs = Arc::new(mem.clone());
        crate::fs::with_fs(fs, || {
            // SET a brand-new entry (inserted under [keys]); comment + folder survive.
            Config::write_binding(&p, "save", Some(&["Cmd-S".to_string(), "C-x C-s".to_string()])).unwrap();
            let cfg = Config::load(p.clone());
            let get = |k: &str| cfg.keys.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
            assert_eq!(get("save"), Some(vec!["Cmd-S".to_string(), "C-x C-s".to_string()]));
            assert_eq!(get("switch_theme"), Some(vec!["C-t".to_string()]));
            assert_eq!(cfg.notes_root, Some(PathBuf::from("/tmp/n")));
            let raw = mem.read_to_string(&p).unwrap();
            assert!(raw.contains("# my notes"), "comment preserved: {raw}");
            // REPLACE an existing entry in place (live-reload picks up the new value).
            Config::write_binding(&p, "switch_theme", Some(&["C-x t".to_string()])).unwrap();
            let cfg = Config::load(p.clone());
            assert_eq!(
                cfg.keys.iter().find(|(n, _)| n == "switch_theme").map(|(_, v)| v.clone()),
                Some(vec!["C-x t".to_string()])
            );
            // RESET removes the entry (None), so the default applies again.
            Config::write_binding(&p, "save", None).unwrap();
            let cfg = Config::load(p.clone());
            assert!(cfg.keys.iter().all(|(n, _)| n != "save"), "save reset to default");
        });
    }

    #[test]
    fn write_binding_seeds_missing_file_with_template() {
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let mem = crate::fs::InMemoryFs::new();
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            // No file yet: the writer seeds the documented template, then adds the entry.
            Config::write_binding(&p, "undo", Some(&["C-j".to_string()])).unwrap();
            let raw = mem.read_to_string(&p).unwrap();
            assert!(raw.contains("awl config"), "template seeded: {raw}");
            let cfg = Config::load(p.clone());
            assert_eq!(
                cfg.keys.iter().find(|(n, _)| n == "undo").map(|(_, v)| v.clone()),
                Some(vec!["C-j".to_string()])
            );
        });
    }

    #[test]
    fn write_default_then_load_roundtrips() {
        // Over the InMemoryFs seam: write_default seeds the template (creating its
        // parent dirs in the fake), and load reads it back.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/awl/config.toml");
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            Config::write_default(&p).unwrap();
            let cfg = Config::load(p.clone());
            // The template's folder lines are COMMENTED, so a fresh default is all-None.
            assert!(cfg.notes_root.is_none() && cfg.workspace.is_none());
            // The new sticky-pref lines are ALSO commented examples → all-None default.
            assert!(cfg.theme.is_none() && cfg.zoom.is_none());
            assert!(cfg.page_mode.is_none() && cfg.caret_mode.is_none());
            // writing_nits is a commented example too → None → the built-in default (ON).
            assert!(cfg.writing_nits.is_none());
            // spellcheck rides the same commented-example pattern → None → default ON.
            assert!(cfg.spellcheck.is_none());
            // autosave rides the same commented-example pattern → None → default ON.
            assert!(cfg.autosave.is_none() && cfg.autosave_on());
            // project_root is a commented example too → None → derive from file/cwd.
            assert!(cfg.project_root.is_none());
            // wysiwyg rides the same commented-example pattern → None → default ON.
            assert!(cfg.wysiwyg.is_none(), "wysiwyg absent → the built-in default (ON)");
        });
    }

    // ── STICKY PREFERENCES ──────────────────────────────────────────────────

    #[test]
    fn load_reads_the_four_sticky_prefs() {
        // theme/zoom/page_mode/caret_mode round-trip from the file into the Config.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let fs = Arc::new(crate::fs::InMemoryFs::new().with_file(
            &p,
            "theme = \"Quokka\"\nzoom = 0.8\npage_mode = false\ncaret_mode = \"ibeam\"\n",
        ));
        crate::fs::with_fs(fs, || {
            let cfg = Config::load(p.clone());
            assert_eq!(cfg.theme.as_deref(), Some("Quokka"));
            assert_eq!(cfg.zoom, Some(0.8));
            assert_eq!(cfg.page_mode, Some(false));
            assert_eq!(cfg.caret_mode.as_deref(), Some("ibeam"));
        });
    }

    #[test]
    fn load_reads_writing_nits_pref() {
        // writing_nits round-trips from the file into the Config as a bool.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let fs = Arc::new(
            crate::fs::InMemoryFs::new().with_file(&p, "writing_nits = false\n"),
        );
        crate::fs::with_fs(fs, || {
            assert_eq!(Config::load(p.clone()).writing_nits, Some(false));
        });
        // Absent → None (the built-in default, ON).
        let fs2 = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "theme = \"Tawny\"\n"));
        crate::fs::with_fs(fs2, || {
            assert_eq!(Config::load(p.clone()).writing_nits, None);
        });
    }

    #[test]
    fn apply_sticky_globals_restores_writing_nits() {
        // The remembered writing_nits value lands on the process-global (it has no CLI
        // flag, so it applies unconditionally). Hold the nits TEST_LOCK + restore.
        let _n = crate::nits::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let nits0 = crate::nits::nits_on();
        // A config remembering OFF flips the (default-on) global off.
        crate::nits::set_nits_on(true);
        let cfg = Config {
            writing_nits: Some(false),
            ..Config::empty()
        };
        cfg.apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert!(!crate::nits::nits_on(), "writing_nits=false restored to off");
        // A config remembering ON flips it back on.
        let cfg_on = Config {
            writing_nits: Some(true),
            ..Config::empty()
        };
        cfg_on.apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert!(crate::nits::nits_on(), "writing_nits=true restored to on");
        // ABSENT (None) leaves the global untouched (the default carries it).
        crate::nits::set_nits_on(true);
        Config::empty().apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert!(crate::nits::nits_on(), "absent pref leaves the global as-is");
        crate::nits::set_nits_on(nits0);
    }

    #[test]
    fn write_pref_persists_writing_nits() {
        // The "Toggle writing nits" toggle persists via write_pref("writing_nits", ..); a
        // reload restores it. Comments + [keys] survive (shared surgical upsert).
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let mem = crate::fs::InMemoryFs::new();
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            Config::write_pref(&p, "writing_nits", "false").unwrap();
            assert_eq!(Config::load(p.clone()).writing_nits, Some(false));
            Config::write_pref(&p, "writing_nits", "true").unwrap();
            assert_eq!(Config::load(p.clone()).writing_nits, Some(true));
            let raw = mem.read_to_string(&p).unwrap();
            assert!(raw.contains("awl config"), "template comments survive: {raw}");
        });
    }

    #[test]
    fn load_reads_spellcheck_pref() {
        // spellcheck round-trips from the file into the Config as a bool, mirroring
        // writing_nits exactly.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let fs = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "spellcheck = false\n"));
        crate::fs::with_fs(fs, || {
            assert_eq!(Config::load(p.clone()).spellcheck, Some(false));
        });
        // Absent → None (the built-in default, ON).
        let fs2 = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "theme = \"Tawny\"\n"));
        crate::fs::with_fs(fs2, || {
            assert_eq!(Config::load(p.clone()).spellcheck, None);
        });
    }

    #[test]
    fn load_reads_session_restore_pref_and_session_restore_on_defaults_true() {
        // The kill-switch round-trips like autosave/history; absent means the
        // built-in default (ON), and `session_restore_on()` reflects it exactly.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let fs = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "session_restore = false\n"));
        crate::fs::with_fs(fs, || {
            let cfg = Config::load(p.clone());
            assert_eq!(cfg.session_restore, Some(false));
            assert!(!cfg.session_restore_on());
        });
        let fs2 = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "theme = \"Tawny\"\n"));
        crate::fs::with_fs(fs2, || {
            let cfg = Config::load(p.clone());
            assert_eq!(cfg.session_restore, None);
            assert!(cfg.session_restore_on(), "absent = built-in default ON");
        });
        assert!(Config::empty().session_restore_on(), "Config::empty() also defaults ON");
    }

    #[test]
    fn apply_sticky_globals_restores_spellcheck() {
        // The remembered spellcheck value lands on the process-global (no CLI flag,
        // so it applies unconditionally). Hold spell's TEST_LOCK + restore.
        let _s = crate::spell::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = crate::spell::spellcheck_on();
        // A config remembering OFF flips the (default-on) global off.
        crate::spell::set_spellcheck_on(true);
        let cfg = Config {
            spellcheck: Some(false),
            ..Config::empty()
        };
        cfg.apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert!(!crate::spell::spellcheck_on(), "spellcheck=false restored to off");
        // A config remembering ON flips it back on.
        let cfg_on = Config {
            spellcheck: Some(true),
            ..Config::empty()
        };
        cfg_on.apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert!(crate::spell::spellcheck_on(), "spellcheck=true restored to on");
        // ABSENT (None) leaves the global untouched (the default carries it).
        crate::spell::set_spellcheck_on(true);
        Config::empty().apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert!(crate::spell::spellcheck_on(), "absent pref leaves the global as-is");
        crate::spell::set_spellcheck_on(saved);
    }

    #[test]
    fn apply_sticky_globals_restores_outline() {
        // The margin outline's built-in default is ON (flipped 2026-07-09). A
        // remembered value lands on the process-global EITHER direction; absent
        // leaves it at its own default. Hold outline's TEST_LOCK.
        let _o = crate::outline::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = crate::outline::outline_on();
        // A config remembering OFF flips the (default-on) global off — proves the
        // `outline = false` override wins over the new ON default.
        crate::outline::set_outline_on(true);
        let cfg_off = Config {
            outline: Some(false),
            ..Config::empty()
        };
        cfg_off.apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert!(!crate::outline::outline_on(), "outline=false restored to off, overriding the ON default");
        // A config remembering ON flips it back on.
        let cfg_on = Config {
            outline: Some(true),
            ..Config::empty()
        };
        cfg_on.apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert!(crate::outline::outline_on(), "outline=true restored to on");
        // ABSENT (None) leaves the global untouched (its own default, now ON, carries it).
        crate::outline::set_outline_on(false);
        Config::empty().apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert!(!crate::outline::outline_on(), "absent pref leaves the global as-is");
        crate::outline::set_outline_on(saved);
    }

    #[test]
    fn load_reads_outline_pref_and_outline_on_defaults_true() {
        // Mirrors `load_reads_session_restore_pref_and_session_restore_on_defaults_true`:
        // the round-trip through REAL TOML parsing (not a hand-built `Config`), proving
        // the built-in default is ON (2026-07-09 flip) and a config `outline = false`
        // still wins over it.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let fs = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "outline = false\n"));
        crate::fs::with_fs(fs, || {
            let cfg = Config::load(p.clone());
            assert_eq!(cfg.outline, Some(false));
            assert!(!cfg.outline_on(), "an explicit outline=false overrides the ON default");
        });
        let fs2 = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "theme = \"Tawny\"\n"));
        crate::fs::with_fs(fs2, || {
            let cfg = Config::load(p.clone());
            assert_eq!(cfg.outline, None);
            assert!(cfg.outline_on(), "absent = built-in default ON");
        });
        assert!(Config::empty().outline_on(), "Config::empty() also defaults ON");
    }

    #[test]
    fn write_pref_persists_spellcheck() {
        // The "Toggle spellcheck" command persists via write_pref("spellcheck", ..);
        // a reload restores it. Comments + [keys] survive (shared surgical upsert).
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let mem = crate::fs::InMemoryFs::new();
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            Config::write_pref(&p, "spellcheck", "false").unwrap();
            assert_eq!(Config::load(p.clone()).spellcheck, Some(false));
            Config::write_pref(&p, "spellcheck", "true").unwrap();
            assert_eq!(Config::load(p.clone()).spellcheck, Some(true));
            let raw = mem.read_to_string(&p).unwrap();
            assert!(raw.contains("awl config"), "template comments survive: {raw}");
        });
    }

    #[test]
    fn write_pref_persists_settings_menu_toggles() {
        // The settings-menu toggles (App::setting_toggle) persist via
        // write_pref(<key>, "true"/"false"); a reload restores each. This is the
        // DISK half of the round-trip that App::persist_pref's mirror-match keeps
        // in step with self.config — every key the toggle seam writes must load back.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let mem = crate::fs::InMemoryFs::new();
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            for key in [
                "autosave",
                "history",
                "session_restore",
                "wysiwyg",
                "inline_images",
                "code_ligatures",
                "outline",
                "menu_bar",
                "typewriter_scroll",
            ] {
                Config::write_pref(&p, key, "false").unwrap();
                let cfg = Config::load(p.clone());
                let got = match key {
                    "autosave" => cfg.autosave,
                    "history" => cfg.history,
                    "session_restore" => cfg.session_restore,
                    "wysiwyg" => cfg.wysiwyg,
                    "inline_images" => cfg.inline_images,
                    "code_ligatures" => cfg.code_ligatures,
                    "outline" => cfg.outline,
                    "menu_bar" => cfg.menu_bar,
                    "typewriter_scroll" => cfg.typewriter_scroll,
                    _ => unreachable!(),
                };
                assert_eq!(got, Some(false), "{key} did not round-trip false");
            }
        });
    }

    #[test]
    fn zoom_accepts_integer_or_float() {
        // A hand-edited `zoom = 1` (TOML integer) must not be silently dropped.
        use crate::fs::FileSystem;
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let mem = crate::fs::InMemoryFs::new();
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            mem.write(&p, b"zoom = 1\n").unwrap();
            assert_eq!(Config::load(p.clone()).zoom, Some(1.0));
            mem.write(&p, b"zoom = 1.6\n").unwrap();
            assert_eq!(Config::load(p.clone()).zoom, Some(1.6));
            // A wrong-typed value is ignored (stays None → the default applies).
            mem.write(&p, b"zoom = \"big\"\n").unwrap();
            assert_eq!(Config::load(p.clone()).zoom, None);
        });
    }

    #[test]
    fn zoom_rejects_non_finite_values() {
        // TOML 1.0 admits literal `nan` / `inf` special floats. A remembered
        // `zoom = nan` would ride into `App::new` / the capture opts and poison
        // every zoom-derived metric, so the lenient read drops any non-finite
        // value (stays None → the built-in default) — same fate as a wrong-typed
        // string. A normal finite float still reads through unchanged.
        use crate::fs::FileSystem;
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let mem = crate::fs::InMemoryFs::new();
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            for junk in ["zoom = nan\n", "zoom = inf\n", "zoom = -inf\n", "zoom = +nan\n"] {
                mem.write(&p, junk.as_bytes()).unwrap();
                assert_eq!(
                    Config::load(p.clone()).zoom,
                    None,
                    "{junk:?} must read as absent, not a poisoned float"
                );
            }
            mem.write(&p, b"zoom = 1.25\n").unwrap();
            assert_eq!(Config::load(p.clone()).zoom, Some(1.25));
        });
    }

    #[test]
    fn autosave_reads_and_defaults_on() {
        // The quiet autosave engine: absent → accessor true (ON, the locked
        // default); an explicit `autosave = false` round-trips and turns it off.
        use crate::fs::FileSystem;
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let mem = crate::fs::InMemoryFs::new();
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            assert!(Config::empty().autosave_on(), "default is ON");
            assert_eq!(Config::empty().autosave, None, "absent key stays None");
            mem.write(&p, b"autosave = false\n").unwrap();
            let cfg = Config::load(p.clone());
            assert_eq!(cfg.autosave, Some(false));
            assert!(!cfg.autosave_on(), "autosave = false disables the engine");
            mem.write(&p, b"autosave = true\n").unwrap();
            assert!(Config::load(p.clone()).autosave_on());
        });
    }

    #[test]
    fn load_reads_wysiwyg_pref() {
        // wysiwyg round-trips from the file into the Config as a bool, mirroring
        // `load_reads_writing_nits_pref` exactly — no CLI flag, no `Config`-level
        // accessor (the effective value lives on the `markdown::WYSIWYG_ON`
        // process-global, applied via `apply_sticky_globals`).
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let fs = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "wysiwyg = false\n"));
        crate::fs::with_fs(fs, || {
            assert_eq!(Config::load(p.clone()).wysiwyg, Some(false));
        });
        // Absent → None (the built-in default, ON).
        let fs2 = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "theme = \"Tawny\"\n"));
        crate::fs::with_fs(fs2, || {
            assert_eq!(Config::load(p.clone()).wysiwyg, None);
        });
    }

    #[test]
    fn apply_sticky_globals_restores_wysiwyg() {
        // The remembered wysiwyg value lands on the process-global (no CLI flag,
        // so it applies unconditionally) — mirrors
        // `apply_sticky_globals_restores_writing_nits` exactly.
        let _w = crate::markdown::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = crate::markdown::wysiwyg_on();
        crate::markdown::set_wysiwyg_on(true);
        let cfg = Config { wysiwyg: Some(false), ..Config::empty() };
        cfg.apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert!(!crate::markdown::wysiwyg_on(), "wysiwyg=false restored to off");
        let cfg_on = Config { wysiwyg: Some(true), ..Config::empty() };
        cfg_on.apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert!(crate::markdown::wysiwyg_on(), "wysiwyg=true restored to on");
        crate::markdown::set_wysiwyg_on(true);
        Config::empty().apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert!(crate::markdown::wysiwyg_on(), "absent pref leaves the global as-is");
        crate::markdown::set_wysiwyg_on(saved);
    }

    #[test]
    fn apply_sticky_globals_restores_code_ligatures() {
        // The remembered code_ligatures value lands on the `render::CODE_LIGATURES_ON`
        // process-global (no CLI flag, applies unconditionally) — mirrors the
        // wysiwyg/writing_nits restore exactly. Only this test writes that global.
        let saved = crate::render::code_ligatures_on();
        crate::render::set_code_ligatures_on(true);
        let cfg = Config { code_ligatures: Some(false), ..Config::empty() };
        cfg.apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert!(!crate::render::code_ligatures_on(), "code_ligatures=false restored to off");
        let cfg_on = Config { code_ligatures: Some(true), ..Config::empty() };
        cfg_on.apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert!(crate::render::code_ligatures_on(), "code_ligatures=true restored to on");
        crate::render::set_code_ligatures_on(false);
        Config::empty().apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert!(!crate::render::code_ligatures_on(), "absent pref leaves the global as-is");
        crate::render::set_code_ligatures_on(saved);
    }

    #[test]
    fn stale_autosnapshot_secs_key_is_ignored() {
        // BACK-COMPAT for the retired periodic knob: an existing config still
        // carrying `autosnapshot_secs = 300` loads clean — the lenient loader
        // reads only known keys, so the stale line is silently inert and every
        // other field keeps its default. No migration, no error, no behaviour.
        use crate::fs::FileSystem;
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let mem = crate::fs::InMemoryFs::new();
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            mem.write(&p, b"autosnapshot_secs = 300\nhistory = true\n").unwrap();
            let cfg = Config::load(p.clone());
            assert_eq!(cfg.history, Some(true), "known keys still load");
            assert_eq!(cfg.autosave, None, "stale knob doesn't leak into autosave");
            assert!(cfg.autosave_on() && cfg.history_on(), "defaults intact");
            assert!(cfg.notes_root.is_none() && cfg.keys.is_empty());
        });
    }

    #[test]
    fn caret_mode_name_round_trips() {
        for m in [
            crate::caret::CaretMode::Block,
            crate::caret::CaretMode::Morph,
            crate::caret::CaretMode::Ibeam,
        ] {
            assert_eq!(parse_caret_mode(caret_mode_name(m)), Some(m));
        }
        // Case-insensitive; an unknown value is None (keep the default).
        assert_eq!(parse_caret_mode("IBEAM"), Some(crate::caret::CaretMode::Ibeam));
        assert_eq!(parse_caret_mode("squiggle"), None);
    }

    #[test]
    fn dictionary_name_round_trips() {
        for v in crate::spell::DictVariant::ALL {
            assert_eq!(parse_dictionary(dictionary_name(v)), Some(v));
        }
        assert_eq!(dictionary_name(crate::spell::DictVariant::EnUs), "en_US");
        assert_eq!(dictionary_name(crate::spell::DictVariant::EnGb), "en_GB");
        assert_eq!(dictionary_name(crate::spell::DictVariant::EnAu), "en_AU");
        // Case-insensitive + hyphen-tolerant; an unknown value is None (default).
        assert_eq!(parse_dictionary("EN_AU"), Some(crate::spell::DictVariant::EnAu));
        assert_eq!(parse_dictionary("en-gb"), Some(crate::spell::DictVariant::EnGb));
        assert_eq!(parse_dictionary("klingon"), None);
    }

    #[test]
    fn load_reads_dictionary_pref_absent_is_none() {
        // `dictionary` round-trips like the other sticky prefs; an absent key stays
        // None (the built-in en_US default applies via `active_variant()`).
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let fs = Arc::new(
            crate::fs::InMemoryFs::new().with_file(&p, "dictionary = \"en_AU\"\n"),
        );
        crate::fs::with_fs(fs, || {
            assert_eq!(Config::load(p.clone()).dictionary.as_deref(), Some("en_AU"));
        });
        let fs2 = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "theme = \"Tawny\"\n"));
        crate::fs::with_fs(fs2, || {
            assert_eq!(Config::load(p.clone()).dictionary, None);
        });
    }

    #[test]
    fn apply_sticky_globals_restores_cjk_priority() {
        // The configured ladder seeds the live global at launch (mirrors
        // `apply_sticky_globals_restores_dictionary`); an absent pref leaves the
        // global at its own built-in default.
        let _g = crate::frontmatter::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::frontmatter::set_cjk_priority(&crate::frontmatter::DEFAULT_CJK_PRIORITY);
        let cfg = Config {
            cjk_priority: Some(vec![
                crate::frontmatter::Lang::Ko,
                crate::frontmatter::Lang::ZhHant,
            ]),
            ..Config::empty()
        };
        cfg.apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        // Normalized: the two named tags lead (in order), the rest fill in.
        assert_eq!(
            crate::frontmatter::cjk_priority(),
            vec![
                crate::frontmatter::Lang::Ko,
                crate::frontmatter::Lang::ZhHant,
                crate::frontmatter::Lang::Ja,
                crate::frontmatter::Lang::ZhHans,
            ]
        );
        // Absent pref leaves the global untouched (not reset to default).
        Config::empty().apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert_eq!(
            crate::frontmatter::cjk_priority()[0],
            crate::frontmatter::Lang::Ko,
            "absent config leaves the global as it was, not silently reset"
        );
        crate::frontmatter::set_cjk_priority(&crate::frontmatter::DEFAULT_CJK_PRIORITY);
    }

    #[test]
    fn apply_sticky_globals_restores_dictionary() {
        // The remembered dictionary lands on the process-global (no CLI flag, like
        // writing_nits) — hold spell's TEST_LOCK + restore so this can't race the
        // dictionary picker / other tests that flip the same global.
        let _g = crate::spell::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = crate::spell::active_variant();
        crate::spell::set_active_variant(crate::spell::DictVariant::EnUs);
        let cfg = Config {
            dictionary: Some("en_AU".to_string()),
            ..Config::empty()
        };
        cfg.apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert_eq!(crate::spell::active_variant(), crate::spell::DictVariant::EnAu);
        // Absent pref leaves the global untouched.
        crate::spell::set_active_variant(crate::spell::DictVariant::EnGb);
        Config::empty().apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert_eq!(crate::spell::active_variant(), crate::spell::DictVariant::EnGb);
        // An unrecognized value is ignored too (keeps the current global).
        let bad = Config {
            dictionary: Some("klingon".to_string()),
            ..Config::empty()
        };
        bad.apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert_eq!(crate::spell::active_variant(), crate::spell::DictVariant::EnGb);
        crate::spell::set_active_variant(saved);
    }

    #[test]
    fn write_pref_persists_dictionary() {
        // The dictionary picker's write-on-commit path (mirrors
        // `write_pref_persists_writing_nits`).
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let mem = crate::fs::InMemoryFs::new();
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            Config::write_pref(&p, "dictionary", "\"en_GB\"").unwrap();
            assert_eq!(Config::load(p.clone()).dictionary.as_deref(), Some("en_GB"));
            Config::write_pref(&p, "dictionary", "\"en_AU\"").unwrap();
            assert_eq!(Config::load(p.clone()).dictionary.as_deref(), Some("en_AU"));
            let raw = mem.read_to_string(&p).unwrap();
            assert!(raw.contains("awl config"), "template comments survive: {raw}");
        });
    }

    #[test]
    fn write_pref_persists_cjk_priority_as_a_toml_array() {
        // The CJK-priority picker's write-on-commit path (`App::persist_cjk_priority`)
        // writes the WHOLE ordered ladder as a TOML array RHS, not one scalar —
        // `write_pref` treats it as an opaque already-formatted string either way.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let mem = crate::fs::InMemoryFs::new();
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            Config::write_pref(&p, "cjk_priority", "[\"ko\", \"ja\", \"zh-Hans\", \"zh-Hant\"]")
                .unwrap();
            let loaded = Config::load(p.clone());
            assert_eq!(
                loaded.cjk_priority,
                Some(vec![
                    crate::frontmatter::Lang::Ko,
                    crate::frontmatter::Lang::Ja,
                    crate::frontmatter::Lang::ZhHans,
                    crate::frontmatter::Lang::ZhHant,
                ])
            );
            // A second promotion (re-upserts the SAME key in place, comments survive).
            Config::write_pref(&p, "cjk_priority", "[\"zh-Hant\", \"ko\", \"ja\", \"zh-Hans\"]")
                .unwrap();
            let loaded2 = Config::load(p.clone());
            assert_eq!(loaded2.cjk_priority.unwrap()[0], crate::frontmatter::Lang::ZhHant);
            let raw = mem.read_to_string(&p).unwrap();
            assert!(raw.contains("awl config"), "template comments survive: {raw}");
            assert_eq!(
                raw.lines().filter(|l| l.trim_start().starts_with("cjk_priority")).count(),
                1,
                "upserts in place, never duplicates the key"
            );
        });
    }

    #[test]
    fn write_pref_upserts_without_clobbering_keys_or_comments() {
        // The write-on-change sticky-pref path, exercised over the InMemoryFs seam.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let mem = crate::fs::InMemoryFs::new().with_file(
            &p,
            "# my notes\nnotes_root = \"/tmp/n\"\n[keys]\nswitch_theme = \"C-t\"\n",
        );
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            // SET each sticky pref. They must land ABOVE [keys] (top-level), the comment
            // + folder + the rebind all survive, and a re-load reads them back.
            Config::write_pref(&p, "theme", "\"Quokka\"").unwrap();
            Config::write_pref(&p, "zoom", "0.800").unwrap();
            Config::write_pref(&p, "page_mode", "false").unwrap();
            Config::write_pref(&p, "caret_mode", "\"ibeam\"").unwrap();
            let cfg = Config::load(p.clone());
            assert_eq!(cfg.theme.as_deref(), Some("Quokka"));
            assert_eq!(cfg.zoom, Some(0.8));
            assert_eq!(cfg.page_mode, Some(false));
            assert_eq!(cfg.caret_mode.as_deref(), Some("ibeam"));
            // The [keys] rebind + the folder + the comment are untouched.
            assert_eq!(
                cfg.keys.iter().find(|(n, _)| n == "switch_theme").map(|(_, v)| v.clone()),
                Some(vec!["C-t".to_string()])
            );
            assert_eq!(cfg.notes_root, Some(PathBuf::from("/tmp/n")));
            let raw = mem.read_to_string(&p).unwrap();
            assert!(raw.contains("# my notes"), "comment preserved: {raw}");
            // The sticky prefs must precede the [keys] header so they parse top-level.
            let theme_at = raw.find("\ntheme =").or_else(|| raw.find("theme =")).unwrap();
            let keys_at = raw.find("[keys]").unwrap();
            assert!(theme_at < keys_at, "theme written above [keys]: {raw}");

            // RE-WRITE a pref in place (the write-on-change path): the value replaces,
            // no duplicate line appears. (Count line-starts so `switch_theme` doesn't
            // count as a `theme` line.)
            Config::write_pref(&p, "theme", "\"Gumtree\"").unwrap();
            let raw = mem.read_to_string(&p).unwrap();
            let theme_lines = raw.lines().filter(|l| l.trim_start().starts_with("theme =")).count();
            assert_eq!(theme_lines, 1, "no duplicate theme line: {raw}");
            assert_eq!(Config::load(p.clone()).theme.as_deref(), Some("Gumtree"));
        });
    }

    #[test]
    fn write_pref_seeds_missing_file_with_template() {
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let mem = crate::fs::InMemoryFs::new();
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            // No file yet: the writer seeds the documented template, then upserts.
            Config::write_pref(&p, "theme", "\"Quokka\"").unwrap();
            let raw = mem.read_to_string(&p).unwrap();
            assert!(raw.contains("awl config"), "template seeded: {raw}");
            // It landed above the template's [keys] header (still top-level), so it loads.
            assert_eq!(Config::load(p.clone()).theme.as_deref(), Some("Quokka"));
        });
    }

    #[test]
    fn write_pref_appends_when_no_table_header() {
        // A config with NO `[keys]`/table header: the pref just appends.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let fs = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "notes_root = \"/tmp/n\"\n"));
        crate::fs::with_fs(fs, || {
            Config::write_pref(&p, "zoom", "0.800").unwrap();
            let cfg = Config::load(p.clone());
            assert_eq!(cfg.zoom, Some(0.8));
            assert_eq!(cfg.notes_root, Some(PathBuf::from("/tmp/n")));
        });
    }

    #[test]
    fn page_width_prose_and_code_persist_and_round_trip_independently() {
        // The Page wider / Page narrower commands persist the new measure via
        // write_pref("page_width_prose"/"page_width_code", "N") — whichever key
        // matches the ACTIVE buffer's kind (`App::persist_page_width`); a reload
        // restores it. The two keys are fully independent. Comments + [keys] survive.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let mem = crate::fs::InMemoryFs::new();
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            Config::write_pref(&p, "page_width_prose", "96").unwrap();
            let cfg = Config::load(p.clone());
            assert_eq!(cfg.page_width_prose, Some(96), "page_width_prose round-trips");
            assert_eq!(cfg.page_width_code, None, "page_width_code is untouched");
            // A float or bare integer both parse; a 0 floors to 1 (never collapses).
            Config::write_pref(&p, "page_width_prose", "0").unwrap();
            assert_eq!(Config::load(p.clone()).page_width_prose, Some(1));

            Config::write_pref(&p, "page_width_code", "120").unwrap();
            let cfg2 = Config::load(p.clone());
            assert_eq!(cfg2.page_width_code, Some(120), "page_width_code round-trips");
            assert_eq!(cfg2.page_width_prose, Some(1), "page_width_prose survives untouched");

            let raw = mem.read_to_string(&p).unwrap();
            assert!(raw.contains("awl config"), "template comments survive: {raw}");
        });
    }

    #[test]
    fn remove_pref_clears_the_page_width_override_matching_the_key_format_preservingly() {
        // "Reset page width" clears the sticky override entirely (rather than
        // writing the default back) via remove_pref("page_width_prose"/"_code") —
        // the Option already means "built-in default", so a future PageClass
        // default change flows through. Comments + [keys] + the OTHER key + OTHER
        // prefs survive untouched — clearing one class never touches the other.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let mem = crate::fs::InMemoryFs::new();
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            Config::write_pref(&p, "page_width_prose", "96").unwrap();
            Config::write_pref(&p, "page_width_code", "120").unwrap();
            Config::write_pref(&p, "theme", "\"Quokka\"").unwrap();
            Config::write_binding(&p, "save", Some(&["Cmd-S".to_string()])).unwrap();
            assert_eq!(Config::load(p.clone()).page_width_prose, Some(96));

            Config::remove_pref(&p, "page_width_prose").unwrap();
            let cfg = Config::load(p.clone());
            assert_eq!(cfg.page_width_prose, None, "the prose override is gone -> built-in default");
            assert_eq!(cfg.page_width_code, Some(120), "the CODE override is untouched");
            // Untouched siblings survive the surgical removal.
            assert_eq!(cfg.theme, Some("Quokka".to_string()));
            assert_eq!(cfg.keys, vec![("save".to_string(), vec!["Cmd-S".to_string()])]);
            // The LIVE line is gone (only the commented TEMPLATE mentions of
            // "page_width_prose" remain, e.g. "# page_width_prose = 70").
            let raw = mem.read_to_string(&p).unwrap();
            assert!(
                !raw.lines().any(|l| l.trim() == "page_width_prose = 96"),
                "the uncommented line itself is deleted: {raw}"
            );
            assert!(raw.contains("awl config"), "template comments survive: {raw}");

            // A SECOND removal (nothing left to remove) is a silent no-op.
            Config::remove_pref(&p, "page_width_prose").unwrap();
            assert_eq!(Config::load(p.clone()).page_width_prose, None);

            // A MISSING file is also a silent no-op (never an error).
            let missing = PathBuf::from("/cfg/nope.toml");
            Config::remove_pref(&missing, "page_width_prose").unwrap();

            // Clearing the OTHER key (page_width_code) is likewise scoped.
            Config::remove_pref(&p, "page_width_code").unwrap();
            let cfg2 = Config::load(p.clone());
            assert_eq!(cfg2.page_width_code, None, "the code override is now also gone");
            assert_eq!(cfg2.theme, Some("Quokka".to_string()), "siblings still untouched");
        });
    }

    #[test]
    fn legacy_page_width_key_is_silently_inert() {
        // The RETIRED single `page_width` key (this pair's predecessor, no
        // migration): a stale line in an existing config is simply an unknown
        // key to the lenient loader — never read, never crashes, and both new
        // keys fall through to their own class default.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let fs = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "page_width = 999\n"));
        crate::fs::with_fs(fs, || {
            let cfg = Config::load(p.clone());
            assert_eq!(cfg.page_width_prose, None, "the retired key is never read");
            assert_eq!(cfg.page_width_code, None);
            assert_eq!(
                cfg.measure_for(crate::page::PageClass::Prose),
                crate::page::DEFAULT_MEASURE,
                "unaffected by the stale legacy key"
            );
            assert_eq!(
                cfg.measure_for(crate::page::PageClass::Code),
                crate::page::DEFAULT_MEASURE_CODE
            );
        });
    }

    #[test]
    fn measure_for_resolves_per_kind_default_or_configured_override() {
        // The per-kind default resolution: unconfigured falls back to each
        // class's own built-in default; a configured override wins per-class,
        // independently.
        let empty = Config::empty();
        assert_eq!(empty.measure_for(crate::page::PageClass::Prose), crate::page::DEFAULT_MEASURE);
        assert_eq!(empty.measure_for(crate::page::PageClass::Code), crate::page::DEFAULT_MEASURE_CODE);

        let cfg = Config { page_width_prose: Some(55), ..Config::empty() };
        assert_eq!(cfg.measure_for(crate::page::PageClass::Prose), 55, "prose override wins");
        assert_eq!(
            cfg.measure_for(crate::page::PageClass::Code),
            crate::page::DEFAULT_MEASURE_CODE,
            "code stays at its own default (untouched by the prose override)"
        );

        let cfg2 = Config { page_width_code: Some(130), ..Config::empty() };
        assert_eq!(
            cfg2.measure_for(crate::page::PageClass::Prose),
            crate::page::DEFAULT_MEASURE,
            "prose stays at its own default"
        );
        assert_eq!(cfg2.measure_for(crate::page::PageClass::Code), 130, "code override wins");
    }

    #[test]
    fn apply_sticky_globals_restores_theme_page_caret_and_honours_flags() {
        // LAUNCH-APPLY: a loaded config's theme/page/caret land on the process-globals,
        // EXCEPT where the corresponding flag was supplied (flag > config). Mutates the
        // shared globals, so hold their test locks (order: theme, page, caret — no other
        // test acquires caret-then-theme, so this can't deadlock). Snapshot + restore so
        // the globals are left as found for the other tests.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _p = crate::page::test_lock();
        let _c = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let theme0 = crate::theme::active_index();
        let page0 = crate::page::page_on();
        let measure0 = crate::page::measure();
        let caret0 = crate::caret::mode();

        // A config remembering Quokka / page-off / prose-width-50 / code-width-130 /
        // ibeam, with NO flags supplied, must apply all four — and the MEASURE
        // resolves per the passed `initial_class` (the launch file's own kind).
        let cfg = Config {
            theme: Some("Quokka".to_string()),
            page_mode: Some(false),
            page_width_prose: Some(50),
            page_width_code: Some(130),
            caret_mode: Some("ibeam".to_string()),
            ..Config::empty()
        };
        crate::page::set_page_on(true); // start opposite so the apply is observable
        crate::page::set_measure(80);
        cfg.apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert_eq!(crate::theme::active().name, "Quokka");
        assert!(!crate::page::page_on(), "page_mode restored to off");
        assert_eq!(crate::page::measure(), 50, "PROSE launch file gets page_width_prose");
        assert_eq!(crate::caret::mode(), crate::caret::CaretMode::Ibeam);

        // The SAME config, launched on a CODE file instead, resolves the OTHER key.
        crate::page::set_measure(80);
        cfg.apply_sticky_globals(false, false, false, false, crate::page::PageClass::Code);
        assert_eq!(crate::page::measure(), 130, "CODE launch file gets page_width_code");

        // With every flag SUPPLIED (true), the config is SKIPPED — the flag-set globals
        // win. Set globals to a known different state, then confirm apply leaves them.
        crate::theme::set_active_by_name("Gumtree");
        crate::page::set_page_on(true);
        crate::page::set_measure(72);
        crate::caret::set_mode(crate::caret::CaretMode::Block);
        cfg.apply_sticky_globals(true, true, true, true, crate::page::PageClass::Prose);
        assert_eq!(crate::theme::active().name, "Gumtree", "theme flag won");
        assert!(crate::page::page_on(), "page flag won");
        assert_eq!(crate::page::measure(), 72, "measure flag won");
        assert_eq!(crate::caret::mode(), crate::caret::CaretMode::Block, "caret flag won");

        // A stale/unknown remembered theme/caret is ignored (no panic, default kept).
        crate::theme::set_active_by_name("Gumtree");
        let bad = Config {
            theme: Some("NotAWorld".to_string()),
            caret_mode: Some("squiggle".to_string()),
            ..Config::empty()
        };
        bad.apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
        assert_eq!(crate::theme::active().name, "Gumtree", "unknown theme ignored");

        // Restore the globals for the rest of the suite.
        crate::theme::set_active(theme0);
        crate::page::set_page_on(page0);
        crate::page::set_measure(measure0);
        crate::caret::set_mode(caret0);
    }

    // ── STICKY PROJECT ROOT ─────────────────────────────────────────────────

    #[test]
    fn load_reads_project_root_pref_with_tilde_expansion() {
        // project_root round-trips like the other sticky prefs, and expands a
        // leading `~/` like notes_root/workspace (it's a path, after all).
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let fs = Arc::new(
            crate::fs::InMemoryFs::new().with_file(&p, "project_root = \"~/code/thing\"\n"),
        );
        crate::fs::with_fs(fs, || {
            assert_eq!(
                Config::load(p.clone()).project_root,
                Some(PathBuf::from(&home).join("code/thing"))
            );
        });
        // Absent -> None (today's default: derive from the launch file / cwd).
        let fs2 = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "theme = \"Tawny\"\n"));
        crate::fs::with_fs(fs2, || {
            assert_eq!(Config::load(p.clone()).project_root, None);
        });
    }

    #[test]
    fn write_pref_persists_project_root() {
        // The switch-project write-on-commit path (mirrors
        // `write_pref_persists_dictionary`): C-x p persists the new root as a
        // quoted absolute path; a reload restores it, comments survive.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        let mem = crate::fs::InMemoryFs::new();
        crate::fs::with_fs(Arc::new(mem.clone()), || {
            Config::write_pref(&p, "project_root", "\"/home/me/work/repo-a\"").unwrap();
            assert_eq!(
                Config::load(p.clone()).project_root,
                Some(PathBuf::from("/home/me/work/repo-a"))
            );
            // Switching AGAIN replaces in place (no duplicate line).
            Config::write_pref(&p, "project_root", "\"/home/me/work/repo-b\"").unwrap();
            assert_eq!(
                Config::load(p.clone()).project_root,
                Some(PathBuf::from("/home/me/work/repo-b"))
            );
            let raw = mem.read_to_string(&p).unwrap();
            assert!(raw.contains("awl config"), "template comments survive: {raw}");
            let lines = raw.lines().filter(|l| l.trim_start().starts_with("project_root =")).count();
            assert_eq!(lines, 1, "no duplicate project_root line: {raw}");
        });
    }

    #[test]
    fn sticky_prefs_and_keybindings_coexist_in_one_file() {
        // The two surgical writers (write_pref for top-level prefs, write_binding for
        // [keys]) must not clobber each other — the launch-apply contract phase 2
        // builds on persists BOTH the caret pref AND keybindings into one file.
        use std::sync::Arc;
        let p = PathBuf::from("/cfg/config.toml");
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            Config::write_binding(&p, "save", Some(&["Cmd-S".to_string()])).unwrap();
            Config::write_pref(&p, "caret_mode", "\"morph\"").unwrap();
            Config::write_binding(&p, "undo", Some(&["Cmd-Z".to_string()])).unwrap();
            Config::write_pref(&p, "zoom", "1.200").unwrap();
            let cfg = Config::load(p.clone());
            assert_eq!(cfg.caret_mode.as_deref(), Some("morph"));
            assert_eq!(cfg.zoom, Some(1.2));
            let get = |k: &str| cfg.keys.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
            assert_eq!(get("save"), Some(vec!["Cmd-S".to_string()]));
            assert_eq!(get("undo"), Some(vec!["Cmd-Z".to_string()]));
        });
    }
}
