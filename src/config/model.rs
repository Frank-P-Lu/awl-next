//! src/config/model.rs — the [`Config`] data model + its TOML PARSE half (the
//! "model+parse" seam of the 2026-07 code-organization split; see
//! `config/mod.rs` for the full module doc). Holds every field, the accessors
//! that turn an `Option<T>` field into an effective value (`*_on`,
//! `measure_for`, `cjk_priority_or_default`), [`Config::empty`], and
//! [`Config::load`] (the lenient TOML reader) plus its small parse helpers —
//! everything a READER of the config needs. Format-preserving WRITES live in
//! `config::write`; the launch-time process-global APPLY lives in
//! `config::apply`.

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
    /// (no conceal, no inline-code pill, no fenced-block panel — see `markdown/`).
    pub wysiwyg: Option<bool>,
    /// `popover` — the FORMAT POPOVER on/off: a mouse selection (drag-release /
    /// double-click word-select) in a markdown buffer floats a small format
    /// toolbar (B · I · == · ` · ~~ · H · Link) over the selection; `None` = the
    /// built-in default (ON, like wysiwyg — no CLI flag). OFF is a TOTAL no-op: no
    /// gesture ever summons it (byte-identical to a build without the feature).
    /// Applied at launch to the `crate::popover::POPOVER_ON` process-global
    /// (`apply_sticky_globals`) and flipped live by the settings menu. Lenient
    /// parse (a non-bool value is ignored → default), like every other sticky bool.
    pub popover: Option<bool>,
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
    /// `reduce_motion` — ACCESSIBILITY TIER 1: settle every juice animator
    /// (caret spring/glide, squash-pop flinches, trailing streak, copy pulse,
    /// the caret-style picker's preview loop) INSTANTLY to its final state
    /// instead of easing over time; `None` = `auto` (the real OS accessibility
    /// preference where one is reachable — macOS `NSWorkspace`, web
    /// `matchMedia` — else OFF on native Linux, a documented scope trim). An
    /// explicit `true`/`false` here always wins over `auto`, either direction.
    /// Applied ONCE at live startup (`crate::motion::apply_at_startup`, called
    /// from `App::new` only — never a headless capture path, see `motion.rs`'s
    /// determinism note) and flipped live by the "Reduce motion" settings-menu
    /// toggle, which also persists an explicit value here.
    pub reduce_motion: Option<bool>,
    /// `ambient_motion` — the AMBIENT-BACKGROUND kill-switch (the lava-lamp
    /// ground's slow ~10 fps drift, `crate::lava`): `None` = the built-in default
    /// (ON, like autosave/session_restore — a quiet sticky toggle, no CLI flag).
    /// OFF freezes any time-varying background to its settled frame (a lava world
    /// still DRAWS, just static). Read ONLY by the live `App`'s ambient tick gate
    /// (`crate::lava::lava_should_tick`) — the headless capture never ticks, so
    /// this can never affect a screenshot (a capture is always the frozen t=0
    /// phase regardless). Independent of `reduce_motion` (which also freezes it,
    /// as an accessibility guarantee): a user may keep juice on yet turn the
    /// ambient drift off, or vice-versa.
    pub ambient_motion: Option<bool>,
    /// `keymap` — the KEYMAP FLAVOR preset (`"native"` | `"emacs"`); `None`/an
    /// unrecognized value = the built-in default (`Native`, today's behavior
    /// byte-identical). `Emacs` widens the `linux_keep_emacs` per-chord door
    /// into a whole-catalog preset UNDER [`crate::convention::Convention::Linux`]
    /// ONLY (structurally inert on Mac, exactly like `linux_keep_emacs` itself
    /// — see [`Self::effective_linux_keep`], the one composition owner every
    /// keymap-construction/reload/label call site routes through instead of
    /// reading `linux_keep_emacs` directly). Stored as the raw string (mirrors
    /// `caret_mode`/`dictionary`); [`Self::keymap_flavor`] is the lenient
    /// accessor. No CLI flag — a sticky Settings-menu toggle row ("Keymap")
    /// flips + persists it, mirroring `reduce_motion`.
    pub keymap: Option<String>,
    /// `date_format` — the "Insert Date" / Settings-menu "Date format" row's
    /// chosen format, stored as the raw persisted SLUG (mirrors `caret_mode`/
    /// `dictionary`/`keymap` — `crate::dateformat::DateFormat::config_name`);
    /// `None`/an unrecognized value = the built-in default (`DD/MM/YY`,
    /// `crate::dateformat::DateFormat::default`). No CLI flag — a sticky
    /// Settings-menu cycling row flips + persists it, mirroring `keymap`.
    pub date_format: Option<String>,
    /// The `[keys]` table as (action-name, chords) pairs, in file order. Each value
    /// is a LIST of up to 2 chords — conceptually slot 1 = NATIVE (macOS), slot 2 =
    /// EMACS — and the keymap parses each chord and OVERRIDES that named action's
    /// binding (additively; both fire). A single TOML string (`save = "C-x C-s"`)
    /// loads as a one-element list, so the old one-chord form stays back-compatible.
    pub keys: Vec<(String, Vec<String>)>,
    /// `linux_keep_emacs` — THE EMACS-HANDS-ON-LINUX per-chord door: a TOML array
    /// of chord strings (e.g. `["C-f", "C-b", "C-n", "C-p"]`) that, under
    /// [`crate::convention::Convention::Linux`] ONLY, keep their EMACS/static
    /// meaning instead of yielding to the native-wins collision (see
    /// `keymap.rs`'s collision-table doc + `KeymapState::linux_keeps`). Each
    /// listed chord's native command stays reachable by palette/menu/another
    /// chord — this only suppresses that ONE chord's native claim. Empty (the
    /// default) = today's Linux-native behavior, byte-identical. Ignored
    /// entirely on `Convention::Mac` (nothing to keep there). Parsed leniently
    /// — an unparsable entry is skipped + reported
    /// (`KeymapState::apply_linux_keep`), never a crash.
    pub linux_keep_emacs: Vec<String>,
    /// Where this config loaded from (the Settings command's open target). Empty
    /// for [`Config::empty`] (a non-file placeholder).
    pub path: PathBuf,
}

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
            popover: None,
            inline_images: None,
            code_ligatures: None,
            cjk_priority: None,
            session_restore: None,
            outline: None,
            menu_bar: None,
            typewriter_scroll: None,
            stats: None,
            reduce_motion: None,
            ambient_motion: None,
            keymap: None,
            date_format: None,
            keys: Vec::new(),
            linux_keep_emacs: Vec::new(),
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

    /// Whether AMBIENT BACKGROUND MOTION (the lava-lamp ground's slow drift) is
    /// enabled. Absent = the built-in default (ON). Read only by the live `App`'s
    /// ambient-tick gate (`crate::lava::lava_should_tick`) — the headless capture
    /// never constructs the tick, so this can never affect a screenshot.
    pub fn ambient_motion_on(&self) -> bool {
        self.ambient_motion.unwrap_or(true)
    }

    /// Whether the LIFETIME STATS odometer (see `stats.rs`) tracks + persists.
    /// Absent = the built-in default (ON). Read only by the live `App`'s native
    /// tracking hooks — the headless capture never constructs them, so this can
    /// never affect a screenshot.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn stats_on(&self) -> bool {
        self.stats.unwrap_or(true)
    }

    /// Whether the persistent MARGIN OUTLINE is enabled (the STORED pref's
    /// leniency law: absent = the built-in default ON, like the other sticky
    /// toggles — flipped 2026-07-09, see `outline.rs`'s module doc). TEST-ONLY
    /// since the every-toggle-dispatches sweep: production reads the live global
    /// (`crate::outline::outline_on`) everywhere — the renderer, the sidecar,
    /// AND the settings readout — while `apply_sticky_globals` seeds that global
    /// from the raw `self.outline` field directly; this derived form survives
    /// only for the leniency law test in `config/tests.rs`.
    #[cfg(test)]
    pub fn outline_on(&self) -> bool {
        self.outline.unwrap_or(true)
    }

    /// Whether the awl-RENDERED menu bar is enabled (the STORED pref's leniency
    /// law: absent = the PLATFORM default — ON for web/Linux, OFF for macOS,
    /// matching `menubar::MENU_BAR_ON`'s own `cfg`-derived default). TEST-ONLY
    /// since the every-toggle-dispatches sweep, exactly like [`Self::outline_on`]
    /// above: production reads the live global (`crate::menubar::menu_bar_on`)
    /// everywhere, and `apply_sticky_globals` seeds it from the raw field.
    #[cfg(test)]
    pub fn menu_bar_on(&self) -> bool {
        self.menu_bar.unwrap_or(cfg!(not(target_os = "macos")))
    }

    /// The EFFECTIVE keymap flavor: the configured `keymap` value if it parses,
    /// else the built-in default ([`crate::keymap::KeymapFlavor::Native`]) —
    /// mirrors `parse_caret_mode`'s leniency (an unrecognized string is treated
    /// exactly like absent, never an error).
    pub fn keymap_flavor(&self) -> crate::keymap::KeymapFlavor {
        self.keymap.as_deref().and_then(crate::keymap::KeymapFlavor::parse).unwrap_or_default()
    }

    /// The EFFECTIVE `linux_keep_emacs` list — THE ONE COMPOSITION OWNER every
    /// keymap-construction / reload / label call site routes through instead of
    /// reading `linux_keep_emacs` directly, so the keymap-flavor preset (and,
    /// as of the insert-link-yields-to-kill-line round, the built-in floor
    /// below) can never drift from the per-chord doors they're built from.
    /// ALWAYS seeded with `crate::keymap::linux_builtin_keep()` first (a chord
    /// kept unconditionally, on EITHER flavor — currently just `C-k`, so
    /// Insert link's native Ctrl-K never displaces kill-line by default; the
    /// user's own call, logged on `linux_builtin_keep()`'s own doc). On top of
    /// that floor: under [`crate::keymap::KeymapFlavor::Native`] (the default)
    /// this is the built-in floor UNIONED with `linux_keep_emacs` — for a
    /// config with an EMPTY `linux_keep_emacs`, that's just the floor itself,
    /// no longer a bare empty list (the one behavior change from the pre-floor
    /// shape: this function is never truly empty anymore). Under
    /// [`crate::keymap::KeymapFlavor::Emacs`] it's the floor UNIONED with the
    /// WHOLE emacs-hands-on-Linux collision-table preset
    /// ([`crate::keymap::linux_emacs_preset_keep`]) UNIONED with the user's own
    /// explicit `linux_keep_emacs` entries (a duplicate anywhere in this chain
    /// — canonical-compare, via [`crate::keymap::linux_keeps_chord`] —
    /// contributes nothing extra; the preset itself never names `C-k`, since
    /// the floor already covers it unconditionally). `keymap.rs`'s dispatch +
    /// `commands.rs`'s label-truth owner both consult exactly this list (never
    /// the raw `linux_keep_emacs` field), so neither the preset nor the floor
    /// can ever lie about what actually fires. Structurally inert on
    /// `Convention::Mac`, same as the raw field — `KeymapState::linux_keeps`
    /// gates on convention regardless of what this returns.
    pub fn effective_linux_keep(&self) -> Vec<String> {
        let mut keep: Vec<String> =
            crate::keymap::linux_builtin_keep().iter().map(|s| s.to_string()).collect();
        if self.keymap_flavor() == crate::keymap::KeymapFlavor::Emacs {
            for p in crate::keymap::linux_emacs_preset_keep() {
                if !crate::keymap::linux_keeps_chord(&keep, &p) {
                    keep.push(p);
                }
            }
        }
        for k in &self.linux_keep_emacs {
            if !crate::keymap::linux_keeps_chord(&keep, k) {
                keep.push(k.clone());
            }
        }
        keep
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
            popover: None,
            inline_images: None,
            code_ligatures: None,
            cjk_priority: None,
            session_restore: None,
            outline: None,
            menu_bar: None,
            typewriter_scroll: None,
            stats: None,
            reduce_motion: None,
            ambient_motion: None,
            keymap: None,
            date_format: None,
            keys: Vec::new(),
            linux_keep_emacs: Vec::new(),
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
        if let Some(b) = table.get("popover").and_then(|v| v.as_bool()) {
            cfg.popover = Some(b);
        }
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
        // `reduce_motion` — ACCESSIBILITY TIER 1, default `auto` (absent). An
        // explicit `true`/`false` here always wins over the OS/browser read.
        if let Some(b) = table.get("reduce_motion").and_then(|v| v.as_bool()) {
            cfg.reduce_motion = Some(b);
        }
        // `ambient_motion` — the ambient-background (lava-lamp) motion kill-switch,
        // default ON (like autosave/session_restore; no CLI flag).
        if let Some(b) = table.get("ambient_motion").and_then(|v| v.as_bool()) {
            cfg.ambient_motion = Some(b);
        }
        // `keymap` — the KEYMAP FLAVOR preset, stored as the raw string (mirrors
        // `caret_mode`/`dictionary`); an unrecognized value is kept verbatim here
        // and simply reads as "unset" through the lenient `keymap_flavor()`
        // accessor — never a parse error, never a crash.
        if let Some(s) = table.get("keymap").and_then(|v| v.as_str()) {
            cfg.keymap = Some(s.to_string());
        }
        // `date_format` — "Insert Date" / the Settings row's chosen format,
        // stored as the raw slug (mirrors `keymap`/`caret_mode`/`dictionary`);
        // an unrecognized value is kept verbatim here and simply reads as
        // "unset" through `DateFormat::from_config_name`'s lenient parse —
        // never a parse error, never a crash.
        if let Some(s) = table.get("date_format").and_then(|v| v.as_str()) {
            cfg.date_format = Some(s.to_string());
        }
        // `linux_keep_emacs` — THE EMACS-HANDS-ON-LINUX per-chord door: a TOML
        // array of chord strings. Every non-string entry is skipped (lenient,
        // like every other array field here); the CHORD-SHAPE validity (does it
        // even parse as a single chord?) is checked later, at the actual
        // consumption door (`KeymapState::apply_linux_keep`) — never here, so a
        // bad entry degrades exactly like a bad `[keys]` chord (reported +
        // skipped, never a crash) rather than silently emptying the whole list.
        if let Some(arr) = table.get("linux_keep_emacs").and_then(|v| v.as_array()) {
            cfg.linux_keep_emacs = arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
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

/// The USER (personal) DICTIONARY path: `dictionary.txt` beside `config.toml` in
/// the SAME config dir (GLOBAL across projects, hand-editable). Derived from the
/// resolved config path so the two always sit together — one config dir, one word
/// list. `None` when the config path has no parent (the `Config::empty`
/// placeholder's blank path, or a bare relative fallback with no directory) —
/// there is then nowhere durable to keep the list, so "Add to dictionary" stays
/// an in-memory-only session add.
pub fn dictionary_path(config_path: &Path) -> Option<PathBuf> {
    let parent = config_path.parent()?;
    if parent.as_os_str().is_empty() {
        return None;
    }
    Some(parent.join("dictionary.txt"))
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
pub(super) fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(s)
}
