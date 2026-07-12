//! The COMMAND CATALOG: the named, fuzzy-searchable list of editor commands the
//! Cmd-P palette runs. Each entry is a human DISPLAY NAME, the `Action` it
//! dispatches on Enter, and UP TO 2 binding labels — slot 1 = NATIVE (macOS), slot
//! 2 = EMACS — shown dim beside the name so the palette also TEACHES the chords.
//!
//! The TWO-BINDING model (awl's "lean into macOS, progressively enhance with
//! Emacs"): every command has at most two chords, both of which fire. A command may
//! fill only one slot (zoom is Cmd-only; the navigation chords are emacs-only), in
//! which case the other is `""` and is omitted from the label.
//!
//! This is deliberately a flat `static` slice rather than logic baked into the
//! palette UI: the bindings are DATA, not hardcoded strings in the renderer, so
//! this catalog is the seam a future NATIVE REBINDING registry slots into (the slot
//! fields become owned, user-overridable labels). The corpus the overlay
//! fuzzy-matches is [`visible_names`], in that same (platform-filtered) order, so the
//! selected ROW index maps straight back to the real `Action` via
//! [`visible_action_of`] (see the palette accept branch in `actions::apply_core`).
//! Plain `names()` stays as the unfiltered full-catalog baseline, now test-only.
//!
//! `InsertChar` / prefix / ignore are intentionally EXCLUDED: the palette lists
//! actions a user would summon or rebind by name, never self-insertion. MOTIONS
//! are split (user-decided 2026-07-10, superseding the original all-motions
//! exclusion; widened by the emacs-hands-on-Linux round): the curated NAVIGATION
//! motions — word / line / document PLUS char/line (forward/backward char, next/
//! previous line) — ARE catalog rows, so they show in Cmd-P + the Keybindings
//! rebind menu and are rebindable via `[keys]` (the door that lets a hand
//! reclaim the retired Option-letter word motion as `forward_word = "M-f"`, or
//! restore a Linux-displaced `forward_char = "C-f"`). ONLY the plain, unmodified
//! ARROW motions (Left/Right/Up/Down with no modifier) stay keymap-only: an
//! arrow key is not a command anyone summons or rebinds by name (it dispatches
//! via `resolve_named`'s static arms regardless of catalog membership), and the
//! catalog stays calm. The law test `catalog_motions_are_exactly_the_curated_navigation_set`
//! pins the split.

use crate::convention::Convention;
use crate::facets::{Facet, FacetItem, FacetScheme};
use crate::keymap::Action;
use std::sync::Mutex;

/// One catalog entry: a display `name` (fuzzy-searched), the `action` it runs on
/// Enter, and the two binding-label slots. `native` is the slot-1 macOS chord,
/// `emacs` the slot-2 chord; either may be `""` when the command fills only one
/// slot. The labels are data so they can later become rebindable, owned values.
pub struct Command {
    pub name: &'static str,
    pub action: Action,
    /// Slot 1 — the NATIVE (macOS, usually Cmd) chord; `""` if there is no native one.
    pub native: &'static str,
    /// Slot 2 — the EMACS chord; `""` if the command is native-only.
    pub emacs: &'static str,
    /// PLATFORM SCOPE: `true` for a command that only makes sense on a native desktop
    /// process — a real OS shell (Quit), a filesystem/version-history feature backed by
    /// a real disk (Version history…/Keep version/Clean unused assets…), the
    /// multi-instance daemon handoff (Finish file), a project-history MRU that's
    /// native-only state (Recent projects…), or the personal odometer (Lifetime
    /// stats, which reads native-only lifetime stats storage). The rebind menu
    /// (Keybindings…) is NOT in this set — the web-config round gave it a real
    /// `config.toml` to persist into (`fs::web_config_path`, over `WebFs`), so it
    /// is available on both platforms like almost everything else. `false` (the
    /// default for nearly every command) means it is available on every compiled
    /// platform. This is the ONE piece of availability DATA the catalog carries;
    /// every predicate below (`available_on`, `visible`) is a
    /// pure function of it — see [`commands::visible`] for the filtered view every
    /// user-facing surface (palette / rebind menu / menu bar / which-key) routes through.
    pub native_only: bool,
    /// PLATFORM SCOPE, the inverse of `native_only`: `true` for a command that only
    /// makes sense on the WEB build — today just "Download file", the export escape
    /// hatch (a native user already has a real file on real disk; hiding the command
    /// there keeps the palette calm, mirroring `native_only`'s own "hide what doesn't
    /// apply" reasoning in the other direction). `false` (the default for every other
    /// command) means this axis imposes no restriction. `available_on` consults both
    /// flags; a row is never `native_only && web_only` (available nowhere) — guarded
    /// by a law test.
    pub web_only: bool,
}

/// The two platforms awl's command catalog is scoped against. `Native` is every
/// desktop build (macOS/Linux); `Web` is the wasm/browser build. A THIRD class was
/// considered (native-only-but-Linux-fine) and rejected — nothing in today's catalog
/// needs a Native/Linux split, so two is the whole taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Native,
    Web,
}

impl Platform {
    /// The platform THIS COMPILED BINARY runs on — the ONE `cfg!` read in the whole
    /// availability system, so every other function here is a pure, testable function of
    /// an explicit `Platform` rather than sprinkling `cfg!(target_arch = "wasm32")`
    /// around. A native-run test can therefore assert the WEB view directly by passing
    /// `Platform::Web` without any cfg gymnastics.
    pub fn current() -> Platform {
        if cfg!(target_arch = "wasm32") {
            Platform::Web
        } else {
            Platform::Native
        }
    }
}

impl Command {
    /// PURE predicate: is this command available on `platform`? `Native` excludes
    /// every `web_only` command (a native user has real files; the export escape
    /// hatch is pointless there); `Web` excludes every `native_only` command (a
    /// browser tab has no real disk / OS shell / daemon). The single owner every
    /// filtered view below routes through.
    pub fn available_on(&self, platform: Platform) -> bool {
        match platform {
            Platform::Native => !self.web_only,
            Platform::Web => !self.native_only,
        }
    }
}

/// The command catalog, in stable display order. The fuzzy corpus is the NAMES
/// in this order, so a selected row index indexes straight back into this slice.
/// Each row carries its two binding slots — native (Cmd) and emacs.
static COMMAND_SEED: &[Command] = &[
    Command { name: "Go to file…",       action: Action::OpenGoto,        native: "",   emacs: ""        , native_only: false, web_only: false },
    Command { name: "Switch project…",   action: Action::OpenProject,     native: "", emacs: ""        , native_only: false, web_only: false },
    // RECENT PROJECTS: opens the SWITCH-PROJECT navigator pre-lensed onto its Recent
    // lens (the fold that retired the standalone RecentProjects picker; recents are a
    // lens now, see `crate::recents`). No default chord — the palette + File menu ARE
    // its entry points (like Settings/About); a real `Action`, independently rebindable.
    Command { name: "Recent projects…",  action: Action::OpenRecentProjects, native: "",     emacs: ""        , native_only: true, web_only: false },
    Command { name: "Browse files…",     action: Action::OpenBrowse,      native: "",        emacs: ""        , native_only: false, web_only: false },
    // GO TO HEADING: opens GO-TO pre-lensed onto its HEADINGS lens (the fold that
    // retired the standalone Outline picker; jump-to-heading is a Go-to lens now,
    // also reachable via ⌘O → ←/→). Palette-only — no default chord (Cmd-Shift-O
    // toggles the persistent margin outline); still fully reachable + rebindable.
    // Named "Go to heading…" to say what it does, paralleling "Go to file…".
    Command { name: "Go to heading…",    action: Action::OpenOutline,     native: "",        emacs: ""        , native_only: false, web_only: false },
    Command { name: "Spell suggestions…", action: Action::OpenSpellSuggest, native: "", emacs: ""        , native_only: false, web_only: false },
    // VERSION HISTORY (the local-history timeline): renamed from "History" so it no
    // longer shadows the "Local history" setting; says it is the version timeline.
    Command { name: "Version history…",  action: Action::OpenHistory,     native: "", emacs: ""        , native_only: true, web_only: false },
    // CLEAN UNUSED ASSETS: summon the Asset Cleaner — a picker of the ORPHAN image
    // files under the active project (an `assets/` image no document references,
    // `crate::assets`). Enter moves the row's file to the macOS Trash (recoverable).
    // Opens a picker, so it takes the ellipsis (picker-naming convention). No default
    // chord — the palette IS its entry point, like Settings/History; a real `Action`,
    // independently rebindable via `[keys] clean_unused_assets`.
    Command { name: "Clean unused assets…", action: Action::OpenAssetClean, native: "",       emacs: ""        , native_only: true, web_only: false },
    // KEEP VERSION: THE CONSCIOUS MARK — pin the current file's state as a
    // prune-exempt local-history snapshot ("I care about this one"). No default
    // chord — the palette IS its entry point, like Settings/About; a real `Action`,
    // independently rebindable via `[keys] keep_version`.
    Command { name: "Keep version",      action: Action::KeepVersion,     native: "",        emacs: ""        , native_only: true, web_only: false },
    Command { name: "Last file",         action: Action::LastBuffer,      native: "",   emacs: ""        , native_only: false, web_only: false },
    Command { name: "New note",          action: Action::NewNote,         native: "",   emacs: ""        , native_only: false, web_only: false },
    Command { name: "Move note…",        action: Action::MoveNote,        native: "",        emacs: ""        , native_only: false, web_only: false },
    // NOTES VERBS round: familiar Save-As-shaped jobs as calm notes-native verbs.
    // Both are palette-only (no default chord, like Move note…) and WebFs-capable
    // (`FileSystem::rename` is implemented on both native and web) — native_only:
    // false. Availability is an ACCEPT-TIME concern (a pathless scratch buffer or a
    // git-managed file politely declines), not a palette-visibility one, mirroring
    // Move note's own scoping.
    Command { name: "Rename note…",      action: Action::OpenRenameNote,  native: "",        emacs: ""        , native_only: false, web_only: false },
    Command { name: "Duplicate note",    action: Action::DuplicateNote,   native: "",        emacs: ""        , native_only: false, web_only: false },
    // FINISH FILE: the emacsclient "server-edit" convention — save, notify any daemon
    // `--wait` client, and switch to the previously-open file. The emacs `C-x #`
    // default is retired; Cmd-W is its native slot now (P5 of the keybinding
    // idiom audit — awl's closest analogue to "close the document": non-
    // destructive under stray muscle memory, since it saves rather than closes
    // anything). NATIVE-ONLY: the daemon handoff it notifies has no web analog.
    // See `crate::daemon`. (Action stays `FinishBuffer`.)
    Command { name: "Finish file",       action: Action::FinishBuffer,    native: "",   emacs: ""        , native_only: true, web_only: false },
    // FOLLOW LINK: open the markdown link under the caret in the OS default browser
    // (a user-initiated handoff, not an app network fetch). Emacs slot `C-c C-o`
    // (org-mode's open-link-at-point); native slot left empty (no universal macOS
    // convention). A caret outside a link is a calm no-op. Rebindable via `[keys]`.
    Command { name: "Follow link",       action: Action::FollowLink,      native: "",        emacs: "" , native_only: false, web_only: false },
    Command { name: "Switch theme…",     action: Action::OpenThemeMenu,   native: "",   emacs: ""        , native_only: false, web_only: false },
    Command { name: "Caret style…",      action: Action::OpenCaretMenu,   native: "",        emacs: ""        , native_only: false, web_only: false },
    Command { name: "Dictionary…",       action: Action::OpenDictionaryMenu, native: "",     emacs: ""        , native_only: false, web_only: false },
    // TOGGLE SPELLCHECK: the global on/off escape hatch (default ON). No default
    // chord — the palette IS its entry point, like Settings/Dictionary; a real
    // `Action` (unlike the `writing_nits` sentinel below), so it is unambiguous
    // through `RunAction` and independently rebindable via `[keys]`.
    Command { name: "Toggle spellcheck", action: Action::ToggleSpellcheck, native: "",     emacs: ""        , native_only: false, web_only: false },
    Command { name: "Toggle hidden files", action: Action::ToggleHiddenFiles, native: "", emacs: ""  , native_only: false, web_only: false },
    Command { name: "Toggle caret style", action: Action::ToggleCaretMode, native: "",       emacs: ""        , native_only: false, web_only: false },
    Command { name: "Toggle page mode",  action: Action::TogglePageMode,  native: "",        emacs: ""        , native_only: false, web_only: false },
    // TOGGLE WRITING NITS: the quiet mechanical-typo underline highlighter (default
    // ON). A render-only toggle with NO default chord — the palette IS its entry
    // point, like Settings — backed by a real `Action::ToggleWritingNits` (the former
    // `Ignore` sentinel is retired), so it round-trips through `RunAction`
    // unambiguously and is independently rebindable via `[keys] toggle_writing_nits`.
    Command { name: "Toggle writing nits", action: Action::ToggleWritingNits, native: "",    emacs: ""        , native_only: false, web_only: false },
    Command { name: "Widen page",        action: Action::PageWider,       native: "",        emacs: ""        , native_only: false, web_only: false },
    Command { name: "Narrow page",       action: Action::PageNarrower,    native: "",        emacs: ""        , native_only: false, web_only: false },
    // RESET PAGE WIDTH: no default chord — the palette IS its entry point, like
    // Settings, plus a DOUBLE-CLICK on the draggable page edge (`app/input/drags.rs`).
    // "There's no easy way back" once you've dragged/widened/narrowed the column.
    Command { name: "Reset page width",  action: Action::PageReset,       native: "",        emacs: ""        , native_only: false, web_only: false },
    Command { name: "Toggle debug",      action: Action::ToggleDebug,     native: "",        emacs: ""        , native_only: false, web_only: false },
    // TOGGLE OUTLINE: the persistent margin table-of-contents (ON by default,
    // flipped 2026-07-09). The Cmd-Shift-O chord (formerly the summoned heading-jump
    // picker's) now toggles it; rebindable via config `[keys] toggle_outline`.
    Command { name: "Toggle outline",    action: Action::ToggleOutline,   native: "", emacs: ""        , native_only: false, web_only: false },
    // TOGGLE TYPEWRITER SCROLL: pin the caret's line centered so the doc scrolls under
    // it (OFF by default). No default chord — palette-only, like About/Settings; a
    // real `Action`, independently rebindable via config `[keys] toggle_typewriter_scroll`.
    Command { name: "Toggle typewriter scroll", action: Action::ToggleTypewriter, native: "", emacs: ""      , native_only: false, web_only: false },
    // TOGGLE MENU BAR: the awl-rendered menu bar (web/Linux; absent on macOS where the
    // native NSMenu bar is the door). No default chord — palette-only, like
    // About/Settings; a real `Action`, independently rebindable via config `[keys]
    // toggle_menu_bar`. Lets a web/Linux user hide the bar (a user-settled requirement).
    Command { name: "Toggle menu bar",   action: Action::ToggleMenuBar,   native: "",        emacs: ""        , native_only: false, web_only: false },
    // ABOUT: no default chord — the palette IS its entry point (like Settings),
    // plus the macOS menu bar's App → "About Awl" item (`menu.rs`, routed —
    // see that module's doc for why this is NOT muda's predefined About).
    Command { name: "About",             action: Action::About,           native: "",        emacs: ""        , native_only: false, web_only: false },
    // CREDITS: opens the embedded CREDITS.md into the buffer (the Settings-opens-
    // a-buffer pattern, not a summoned card like About/Lifetime — this is prose
    // meant to be read/scrolled, not a stat panel). No default chord — the palette
    // IS its entry point (like Settings/About); a real `Action`, independently
    // rebindable via `[keys] credits`. See `credits.rs`.
    Command { name: "Credits",           action: Action::OpenCredits,     native: "",        emacs: ""        , native_only: false, web_only: false },
    // GUIDE: opens the embedded GUIDE.md into the buffer — the Credits-opens-a-
    // buffer pattern exactly (prose meant to be read/scrolled, not a stat panel
    // or a picker). No default chord — the palette IS its entry point (like
    // Settings/Credits/About); a real `Action`, independently rebindable via
    // `[keys] guide`. See `guide.rs`.
    Command { name: "Guide",             action: Action::OpenGuide,       native: "",        emacs: ""        , native_only: false, web_only: false },
    // LIFETIME STATS: the summoned personal ODOMETER card (characters, writing
    // time, files touched, caret travel, your world) — the LIFETIME figures split
    // out of the held stats HUD. No default chord — the palette IS its entry point
    // (like Settings/About); a real `Action`, independently rebindable via `[keys]
    // lifetime_stats`. See `lifetime.rs`.
    Command { name: "Lifetime stats",    action: Action::LifetimeStats,   native: "",        emacs: ""        , native_only: true, web_only: false },
    // LINE ENDINGS: toggle the active file's on-disk ending (LF <-> CRLF). No default
    // chord — the palette IS its entry point (a rare command, like Settings/About); a
    // real `Action` (`ConvertLineEndings`), independently rebindable via `[keys]`.
    Command { name: "Line endings…",     action: Action::ConvertLineEndings, native: "",     emacs: ""        , native_only: false, web_only: false },
    // ALIGN TABLE: re-pad the GFM table under the caret so its `|` line up (source
    // alignment, never a drawn grid). No default chord — the palette IS its entry
    // point (like Settings/About); a real `Action`, independently rebindable.
    Command { name: "Align table",       action: Action::AlignTable,      native: "",        emacs: ""        , native_only: false, web_only: false },
    // REPORT A PROBLEM: compose a mailto: link to the maintainer, with the
    // newest local crash log's path attached-by-name if one exists (never its
    // content — the crash-visibility privacy law). No default chord — the
    // palette IS its entry point (like Settings/About/Align table); a real
    // `Action`, independently rebindable via `[keys]`. `native_only: false` —
    // available on the web build too (the mailto composition is pure and
    // platform-agnostic; only the crash-log path lookup is native-only). See
    // `crashlog.rs`.
    Command { name: "Report a Problem",  action: Action::ReportProblem,   native: "",        emacs: ""        , native_only: false, web_only: false },
    // DOWNLOAD FILE (WEB-ONLY): the escape hatch for the browser's no-real-
    // filesystem sandbox — export the active buffer as a plain-text download
    // (Blob + object URL + a synthetic `<a download>` click, `web_export.rs`).
    // No default chord — the palette IS its entry point (like Settings/About/
    // Report a Problem); a real `Action`, independently rebindable via `[keys]`.
    // `web_only: true` — HIDDEN on native (a desktop user already has a real
    // file on real disk; see `commands.rs`'s `web_only` field doc).
    Command { name: "Download file",     action: Action::DownloadFile,    native: "",        emacs: ""        , native_only: false, web_only: true },
    // CHECK FOR UPDATES: never a network fetch — records a LOCAL "last checked"
    // marker (best-effort, `updates::record_checked`) then hands off to the OS
    // browser at the site's own `/check?v=…` page, which does the actual version
    // comparison against its own `version.json` (see `updates.rs`). No default
    // chord — the palette IS its entry point (like Report a Problem/About). Uses
    // the SAME `Effect::FollowLink`-style OS-handoff seam `App::follow_link`
    // already provides. `native_only: true` — the web build updates by
    // deploy/refresh, so "checking" is meaningless there.
    Command { name: "Check for Updates", action: Action::CheckForUpdates, native: "",        emacs: ""        , native_only: true, web_only: false },
    // MARKDOWN FORMATTING COMMANDS (see `actions/format.rs`): each a TOGGLE applied as
    // one undoable edit, markdown-only. The three with a UNIVERSAL native convention get
    // a Cmd chord — Cmd-B = Bold, Cmd-I = Italic, Cmd-E = Inline code (all free under
    // plain Super: 'b'/'i'/'e' are unused there). Cmd-I joined this round (the
    // keybinding-idiom audit's Option B): the held stats HUD MOVED to Option-Cmd-I
    // (`keymap.rs`) specifically so plain Cmd-I could become Italic's native slot,
    // rather than stay palette-only. Cmd-Shift-L (Task list) is the ONE block toggle
    // with a genuine native anchor — Apple Notes' checklist idiom (W3); the rest of the
    // block toggles + Highlight/Strikethrough have no obvious native convention, so they
    // stay palette-only (like Align Table). All independently rebindable via `[keys]`
    // (the emacs slot is left empty for a user to fill).
    Command { name: "Blockquote",        action: Action::ToggleBlockquote,   native: "",         emacs: ""        , native_only: false, web_only: false },
    Command { name: "Bullet list",       action: Action::ToggleBulletList,   native: "",         emacs: ""        , native_only: false, web_only: false },
    Command { name: "Numbered list",     action: Action::ToggleNumberedList, native: "",         emacs: ""        , native_only: false, web_only: false },
    Command { name: "Task list",         action: Action::ToggleTaskList,     native: "",  emacs: ""        , native_only: false, web_only: false },
    Command { name: "Heading",           action: Action::ToggleHeading,      native: "",         emacs: ""        , native_only: false, web_only: false },
    Command { name: "Code block",        action: Action::ToggleCodeBlock,    native: "",         emacs: ""        , native_only: false, web_only: false },
    Command { name: "Bold",              action: Action::Bold,               native: "",    emacs: ""        , native_only: false, web_only: false },
    Command { name: "Italic",            action: Action::Italic,             native: "",    emacs: ""        , native_only: false, web_only: false },
    Command { name: "Inline code",       action: Action::InlineCode,         native: "",    emacs: ""        , native_only: false, web_only: false },
    Command { name: "Highlight",         action: Action::Highlight,          native: "",         emacs: ""        , native_only: false, web_only: false },
    Command { name: "Strikethrough",     action: Action::Strikethrough,      native: "",         emacs: ""        , native_only: false, web_only: false },
    // LINKS V2: Cmd-K — the chord the keybinding-idiom audit reserved for exactly
    // this (W1: Bear/Craft/Notion/Things/Ulysses/Slack all spend it on insert/
    // edit-link). Emacs slot deliberately empty (no prior default claimed it —
    // Links v2 is new, not a retirement). See `Action::InsertLink`'s own doc for
    // the three-mode behavior (wrap selection / edit existing link / insert empty
    // markup). On LINUX this chord has NO effective binding by default, on
    // EITHER keymap flavor — `keymap::linux_builtin_keep()` keeps Ctrl-K's
    // kill-line meaning unconditionally (the user's own call: kill-line is too
    // load-bearing for emacs hands to lose), so Insert link is reachable there
    // only via the palette or an explicit `[keys] insert_link = "C-k"`.
    Command { name: "Insert link…",      action: Action::InsertLink,         native: "",    emacs: ""        , native_only: false, web_only: false },
    // NOTE: the held stats HUD (Option-Cmd-I) is deliberately NOT a palette command. It
    // is a momentary HOLD-to-peek (shown while the key is down, gone the instant it
    // lifts), so a DISCRETE selection — which has no key-release to dismiss it — would
    // leave it stuck on. Its ONLY summon path is the held Option-Cmd-I chord (resolved
    // in `keymap.rs`); see `hud.rs`.
    Command { name: "Save",              action: Action::Save,            native: "",   emacs: ""        , native_only: false, web_only: false },
    Command { name: "Quit",              action: Action::Quit,            native: "",   emacs: ""        , native_only: true, web_only: false },
    Command { name: "Search forward",    action: Action::SearchForward,   native: "",   emacs: ""     , native_only: false, web_only: false },
    Command { name: "Search backward",   action: Action::SearchBackward,  native: "", emacs: ""     , native_only: false, web_only: false },
    Command { name: "Find and replace…", action: Action::OpenReplace,     native: "",   emacs: ""        , native_only: false, web_only: false },
    Command { name: "Undo",              action: Action::Undo,            native: "",   emacs: ""     , native_only: false, web_only: false },
    Command { name: "Redo",              action: Action::Redo,            native: "", emacs: ""        , native_only: false, web_only: false },
    // CLIPBOARD + SELECT-ALL: bound in the keymap (native Cmd-C/X/V/A, emacs M-w/C-w/C-y)
    // but previously absent here, so they were invisible to Cmd-P and the rebind menu.
    // Listed with their ACTUAL bindings so they show + become rebindable. (Bare C-a stays
    // LineStart in the emacs slot, so Select all is Cmd-only.)
    Command { name: "Copy",              action: Action::CopyRegion,      native: "",   emacs: ""        , native_only: false, web_only: false },
    Command { name: "Cut",               action: Action::KillRegion,      native: "",   emacs: ""     , native_only: false, web_only: false },
    Command { name: "Paste",             action: Action::Yank,            native: "",   emacs: ""     , native_only: false, web_only: false },
    Command { name: "Select all",        action: Action::SelectAll,       native: "",   emacs: ""        , native_only: false, web_only: false },
    Command { name: "Zoom in",           action: Action::ZoomIn,          native: "",   emacs: ""        , native_only: false, web_only: false },
    Command { name: "Zoom out",          action: Action::ZoomOut,         native: "",   emacs: ""        , native_only: false, web_only: false },
    Command { name: "Reset zoom",        action: Action::ZoomReset,       native: "",   emacs: ""        , native_only: false, web_only: false },
    // MOTION COMMANDS (user-decided 2026-07-10, superseding the original all-motions
    // exclusion — see the module doc): the curated NAVIGATION motions are catalog rows
    // so they show in Cmd-P + the Keybindings rebind menu and are REBINDABLE via
    // `[keys]`. The concrete ask this serves: reclaiming the retired Option-letter
    // word motion — `forward_word = ["M-Right", "M-f"]` / `backward_word =
    // ["M-Left", "M-b"]` — which macOS reserves for typing by DEFAULT (the platform
    // rule that retired the M-letter layer) but a config line may deliberately opt
    // back in. Each row shows its REAL default chord (both slots fire; a config
    // override is ADDITIVE); the emacs slots left empty by that retirement stay
    // empty for the user to fill — never re-shipped. Line start/end keep their
    // surviving bare-control second slots (C-a / C-e), now visible + teachable.
    Command { name: "Forward word",      action: Action::ForwardWord,     native: "",   emacs: ""     , native_only: false, web_only: false },
    Command { name: "Backward word",     action: Action::BackwardWord,    native: "",    emacs: ""     , native_only: false, web_only: false },
    Command { name: "Line start",        action: Action::LineStart,       native: "",  emacs: ""  , native_only: false, web_only: false },
    Command { name: "Line end",          action: Action::LineEnd,         native: "", emacs: ""  , native_only: false, web_only: false },
    Command { name: "Document start",    action: Action::BufferStart,     native: "",    emacs: ""     , native_only: false, web_only: false },
    Command { name: "Document end",      action: Action::BufferEnd,       native: "",  emacs: ""     , native_only: false, web_only: false },
    // THE EMACS-HANDS-ON-LINUX ROUND: the LAST four bare-control nav motions join
    // the catalog (char forward/back, line up/down) — the ones a Linux emacs hand
    // reaches for constantly (C-f/C-b/C-n/C-p) and that the Linux-native collision
    // table (see `keymap.rs`) quietly displaces with Search forward / Bold / New
    // note / Command palette. Making them catalog rows is what lets a `[keys]`
    // line rebind them at ALL — before this round `forward_char` etc. was not a
    // recognized action name, so a Linux hand who wanted C-f back had NO door,
    // config or otherwise (see `linux_keep_emacs` for the actual per-chord fix the
    // collision needed). NO native slot: the plain, unmodified arrow keys already
    // fire these unconditionally in `resolve_named`'s static arms — that dispatch
    // is UNCHANGED by this round ("arrows stay keymap-only static arms as
    // before"), so there is no macOS-flavored CHORD to teach or rebind here, only
    // the emacs letter. `[keys] forward_char = "C-f"` still rebinds it (to any
    // chord, not just the default) like any other catalog command.
    Command { name: "Forward char",      action: Action::ForwardChar,     native: "",          emacs: ""  , native_only: false, web_only: false },
    Command { name: "Backward char",     action: Action::BackwardChar,    native: "",          emacs: ""  , native_only: false, web_only: false },
    Command { name: "Next line",         action: Action::NextLine,        native: "",          emacs: ""  , native_only: false, web_only: false },
    Command { name: "Previous line",     action: Action::PreviousLine,    native: "",          emacs: ""  , native_only: false, web_only: false },
    // Settings: Cmd-, is THE preferences chord since Mac OS X 10.1 (P1 of the
    // keybinding idiom audit — the highest-value single binding in that report).
    // It summons the faceted SETTINGS MENU (the friendly default); the raw
    // config-as-text file lives behind the menu's "Edit config as text" row
    // (`Action::OpenSettings`).
    Command { name: "Settings…",         action: Action::OpenSettingsMenu, native: "",  emacs: ""        , native_only: false, web_only: false },
    // Keybindings has NO default chord either — summon it by name (Cmd-P) like
    // Settings; it is the GAME-STYLE rebind menu (capture a key per command). It is
    // itself rebindable via `[keys] keybindings = "..."`.
    Command { name: "Keybindings…",      action: Action::OpenKeybindings, native: "",        emacs: ""        , native_only: false, web_only: false },
];

/// THE KEYMAP-DEFAULTS-AS-DATA ROUND (CLAUDE.md): the actual command catalog.
/// [`COMMAND_SEED`] above carries every command's NAME, ORDER, `Action`, and
/// platform scope (`native_only`/`web_only`) — hand-written code, unchanged
/// by this round — but its own `native`/`emacs` fields are unused
/// placeholders (always `""` in the literal). The REAL default chord values
/// are looked up ONCE by slug from the embedded `assets/keymap-defaults.toml`
/// ([`crate::keymap_defaults::command_defaults`]) and spliced in here — so a
/// default chord now exists in exactly ONE place (the TOML file), never
/// duplicated as a second literal in this array. `Box::leak` is a one-time,
/// ~80-entry startup cost (the whole `Vec` is memoized by `LazyLock`, so this
/// closure runs at most once per process) — it keeps `Command::native`/
/// `emacs`'s public field TYPE (`&'static str`) unchanged, so every existing
/// consumer (`c.native.trim()`, `COMMANDS.iter()`, `COMMANDS[i]`, …) needed no
/// edit beyond a handful of bare `for c in COMMANDS` loops (which cannot
/// desugar against a `LazyLock`'s owned `Vec` without an explicit `.iter()`,
/// unlike the retired `&'static [Command]` slice, which was `Copy`).
pub static COMMANDS: std::sync::LazyLock<Vec<Command>> = std::sync::LazyLock::new(|| {
    let defaults = crate::keymap_defaults::command_defaults();
    COMMAND_SEED
        .iter()
        .map(|seed| {
            let (native, emacs) = defaults.get(slug(seed.name).as_str()).cloned().unwrap_or_default();
            Command {
                name: seed.name,
                action: seed.action.clone(),
                native: Box::leak(native.into_boxed_str()),
                emacs: Box::leak(emacs.into_boxed_str()),
                native_only: seed.native_only,
                web_only: seed.web_only,
            }
        })
        .collect()
});

/// Join a command's two binding slots into ONE dim palette label, e.g.
/// `"⌘S · C-x C-s"`. The NATIVE (slot 1, macOS) chord renders as mac MODIFIER
/// GLYPHS ([`crate::keyspec::mac_glyph_chord`]: `Cmd-S` → `⌘S`); the EMACS (slot 2)
/// chord keeps its terse text (`C-x C-s`). A single non-empty slot shows alone;
/// both empty yields `""` (the bindless Settings). The `·` separator pairs them.
pub fn join_slots(native: &str, emacs: &str) -> String {
    let native_g = if native.trim().is_empty() {
        String::new()
    } else {
        crate::keyspec::mac_glyph_chord(native)
    };
    match (native_g.is_empty(), emacs.trim().is_empty()) {
        (false, false) => format!("{native_g} · {emacs}"),
        (false, true) => native_g,
        (true, false) => emacs.to_string(),
        (true, true) => String::new(),
    }
}

// ── LINUX-NATIVE KEYMAP: convention-resolved slot 1 ────────────────────────────
//
// THE DATA DESIGN (chosen over per-convention chord COLUMNS): each catalog row
// keeps its ONE mac-flavored `native` string, unchanged — that stays the source
// of truth `bindings()`/`join_slots` read for the Mac baseline. A Linux label or
// dispatch NEVER reads a second stored column; instead it's a PURE, TOTAL
// TRANSLATION of that same string (`keyspec::translate_native_for_linux`, a plain
// Cmd→Ctrl modifier swap) with an EXPLICIT OVERRIDE table below for the handful of
// commands where that naive swap is WRONG. Why this over per-convention columns:
// (1) it keeps the catalog's ONE mac-native field as the single hand-maintained
// fact per command (no risk of the two columns drifting when a mac chord changes
// and the Linux column isn't updated to match); (2) the override table is a
// SHORT, auditable exceptions list rather than 60+ rows of mostly-identical data;
// (3) `keymap.rs`'s dispatch reuses the EXACT SAME override for the handful of
// commands whose action needs a genuinely different resolve-time chord (not just
// a translated label) — see `commands::LINUX_NATIVE_OVERRIDE`'s doc for why those
// three exist.
//
// THE OVERRIDE TABLE, keyed by catalog command NAME, holding the LITERAL Linux
// chord spec to use instead of the naive Cmd→Ctrl swap:
//   - "Line start" / "Line end": mac native is Cmd-Left/Right; naively swapping
//     Super→Control would collide with Ctrl-Left/Right, which the keymap ALREADY
//     binds to word motion (`resolve_named`'s `alt || ctrl` arm, convention-
//     agnostic) — so the Linux-native chord is plain `Home`/`End` instead (no
//     modifier needed; `resolve_named`'s unconditional Home/End arms already fire
//     LineStart/LineEnd on every convention, so no keymap change is needed here —
//     only the LABEL differs from the naive swap).
//   - "Document start" / "Document end": mac native is Cmd-Up/Down; the Linux
//     convention for buffer start/end is Ctrl-Home/Ctrl-End (gedit/VS Code/GTK),
//     not the naively-translated Ctrl-Up/Down — `keymap.rs` gains a matching
//     `Convention::Linux`-gated `Ctrl-Home`/`Ctrl-End` arm (see its module doc).
const LINUX_NATIVE_OVERRIDE: &[(&str, &str)] = &[
    ("Line start", "Home"),
    ("Line end", "End"),
    ("Document start", "C-Home"),
    ("Document end", "C-End"),
];

/// The RESOLVED native chord spec for `c` under `convention` — Mac returns `c.native`
/// UNCHANGED (byte-identical to today, the hard law of this round); Linux consults
/// [`LINUX_NATIVE_OVERRIDE`] first, else falls back to the naive Cmd→Ctrl translation
/// (`keyspec::translate_native_for_linux`). Empty on either convention when the
/// command has no native slot to begin with. This is the ONE owner both `keymap.rs`'s
/// dispatch (for the handful of commands whose ACTION needs the resolved chord, not
/// just its label — via `[keys]`-style literal resolution) and every label surface
/// below route through.
pub fn resolved_native(c: &Command, convention: Convention) -> String {
    if c.native.trim().is_empty() {
        return String::new();
    }
    match convention {
        Convention::Mac => c.native.to_string(),
        Convention::Linux => LINUX_NATIVE_OVERRIDE
            .iter()
            .find(|(name, _)| *name == c.name)
            .map(|(_, chord)| chord.to_string())
            .unwrap_or_else(|| crate::keyspec::translate_native_for_linux(c.native)),
    }
}

/// The DISPLAY LABEL for `c`'s resolved native chord under `convention` — Mac glyphs
/// (`⌘S`) on [`Convention::Mac`], word labels (`Ctrl+S`) on [`Convention::Linux`].
/// `""` when the command has no native slot. THE ONE OWNER every label surface reads
/// (palette rows, the rebind menu, the in-app menubar hints, the hold-⌘ peek) — never
/// call [`crate::keyspec::mac_glyph_chord`] on a raw `c.native` directly outside this
/// function, or a Linux/web build would show a mac glyph under its own convention.
pub fn resolved_native_label(c: &Command, convention: Convention) -> String {
    let native = resolved_native(c, convention);
    if native.trim().is_empty() {
        return String::new();
    }
    match convention {
        Convention::Mac => crate::keyspec::mac_glyph_chord(&native),
        Convention::Linux => crate::keyspec::linux_glyph_chord(&native),
    }
}

/// THE WEB CHORD SANITY ROUND, Tier 2 — [`resolved_native_label`]'s TRUTHFUL
/// sibling: when `c`'s resolved native chord is a browser-reserved accelerator
/// ([`crate::webreserved::is_reserved`]) on `platform`, this shows the command's
/// [`WEB_ALTERNATE`] chord instead (see that table's doc — v2 of the web-chord
/// sanity round, closing the v1 "no replacement chord" gap), or `""` if it has
/// none; otherwise identical to [`resolved_native_label`]. `platform` is an
/// EXPLICIT parameter (not read from [`Platform::current`] internally) — the
/// same testability pattern [`Command::available_on`]/[`action_available`]
/// already use — so a native-run test can assert the WEB view directly by
/// passing [`Platform::Web`] without any cfg gymnastics; every real call site
/// passes [`Platform::current`]. The reserved check only ever fires on
/// [`Platform::Web`] — a native build's chords are never browser-shadowed, so
/// this is byte-identical to [`resolved_native_label`] on every native call
/// site. THE ONE OWNER of "is this command's native chord actually worth
/// showing" — [`join_slots_truthful`] (the two-slot palette/rebind label),
/// `menu::item_chord` (the awl-rendered menu bar's native-only column, which
/// shows on web too), and `keytoken::key_token_label` (the starting docs'
/// chord tokens) all route through it.
pub fn resolved_native_label_truthful(c: &Command, convention: Convention, platform: Platform) -> String {
    let reserved = platform == Platform::Web && crate::webreserved::is_reserved(&resolved_native(c, convention), convention);
    if reserved {
        match web_alternate_for(c, convention) {
            Some(alt) => match convention {
                Convention::Mac => crate::keyspec::mac_glyph_chord(alt),
                Convention::Linux => crate::keyspec::linux_glyph_chord(alt),
            },
            None => String::new(),
        }
    } else {
        resolved_native_label(c, convention)
    }
}

// ── CONVENTION-TRUTHFUL SURFACES ROUND — WEB-ALTERNATE CHORDS ─────────────────
//
// v1 (the web chord sanity round) deliberately left a browser-reserved command
// (New note / Switch theme… — the only two catalog commands BOTH available on
// `Platform::Web` AND carrying a reserved native chord; verified exhaustively
// by `tests::exactly_new_note_and_switch_theme_are_web_reserved_and_available`)
// bindless on the web: `resolved_native_label_truthful` just showed "". This
// table closes that gap with ONE non-reserved, collision-free chord per
// command to become its slot-1 on `Platform::Web` — CONVENTION-KEYED, because
// "collision-free" means something different per convention:
//   - Mac web: native is Cmd, so a bare CTRL-letter is free of both the
//     browser's own mac reservations (`webreserved::MAC_WEB_RESERVED` is
//     entirely Cmd-based) and the static keymap's bare-control emacs arms,
//     PROVIDED the letter isn't already claimed there (`j`/`t` aren't).
//   - Linux web: native IS Ctrl, so Ctrl-N/Ctrl-T are literally the two
//     reserved chords themselves — unusable as their own replacement. A bare
//     ALT-letter is free instead: the identity round fully RETIRED the
//     default Meta-letter keymap layer (see CLAUDE.md's "Emacs default
//     retirement" note), so no default arm claims `M-n`/`M-t`, and Alt is not
//     a browser-reserved modifier on Linux/Windows browsers.
//
// PICKED EMPIRICALLY (a throwaway Playwright probe against real Chromium —
// see the round notes): candidate 1 was `Alt-N`/`Alt-T` on BOTH conventions,
// but Option/Alt on a MAC keyboard is macOS's OWN typing layer even inside a
// browser tab (dead-key accent composition) — Safari in particular can
// compose at the IME layer before a page's `keydown` handler (and its
// `preventDefault()`) ever sees the press, so a Mac-web Alt-chord risks
// silently typing a stray character instead of firing the command. A bare
// Ctrl-letter has no such composition step on ANY platform, so Mac web keeps
// the Ctrl-letter candidates (Ctrl being unavailable on Linux web for the
// reason above, so Linux web keeps the Alt-letter ones).
const WEB_ALTERNATE: &[(&str, &str, &str)] = &[
    // name              mac-web alt   linux-web alt
    ("New note", "C-j", "M-n"),
    ("Switch theme…", "C-t", "M-t"),
];

/// The web-alternate chord SPEC for `c` under `convention` (already convention-
/// keyed — see [`WEB_ALTERNATE`]'s doc), or `None` when `c` has none. Pure data
/// lookup; callers decide whether the situation (a reserved slot-1, on
/// [`Platform::Web`]) actually calls for it.
fn web_alternate_for(c: &Command, convention: Convention) -> Option<&'static str> {
    WEB_ALTERNATE.iter().find(|(name, _, _)| *name == c.name).map(|(_, mac, linux)| match convention {
        Convention::Mac => *mac,
        Convention::Linux => *linux,
    })
}

/// The config `[keys]`-shaped entries that wire every [`WEB_ALTERNATE`] chord
/// into REAL dispatch on [`Platform::Web`] — the keymap has no other seam for
/// "a chord outside the native/emacs static arms," so this reuses the SAME
/// override machinery a user's own `[keys]` line rides
/// (`KeymapState::apply_overrides`, fed from `App::new`'s keymap construction).
/// `existing` is the user's OWN config `[keys]` list — **config still trumps
/// everything**: a command the user has already rebound (by its slug) is
/// skipped here entirely, so their chosen chord is never shadowed by the
/// default alternate. `convention`/`platform` are EXPLICIT parameters,
/// mirroring [`resolved_native_label_truthful`]'s own testability pattern
/// (`Convention::current`/`Platform::current` can't be pinned from a plain
/// native test) — every real call site passes both `::current()`. Returns an
/// empty list on [`Platform::Native`], so a native build's keymap
/// construction is unaffected byte-for-byte.
pub fn web_alternate_keys(
    existing: &[(String, Vec<String>)],
    convention: Convention,
    platform: Platform,
) -> Vec<(String, Vec<String>)> {
    if platform != Platform::Web {
        return Vec::new();
    }
    COMMANDS
        .iter()
        .filter_map(|c| {
            let alt = web_alternate_for(c, convention)?;
            let want = slug(c.name);
            if existing.iter().any(|(name, _)| slug(name) == want) {
                return None; // a `[keys]` override already claims this command
            }
            Some((want, vec![alt.to_string()]))
        })
        .collect()
}

/// Slugify a command name to its config ACTION NAME: lower-case with spaces as
/// underscores ("Go to file…" -> "go_to_file", "Switch theme…" -> "switch_theme").
/// Both the rebinder ([`action_for_name`]) and the palette display
/// ([`effective_bindings`]) key off this, so a `[keys]` entry and the shown chord
/// stay consistent.
///
/// CANONICALIZATION (the `…` picker-suffix gate): a trailing ellipsis is DISPLAY-
/// only (it marks a command that opens a list/menu — "Switch theme…"), so it is
/// stripped BEFORE slugging. This is what lets the ellipsis be added to a picker's
/// label without forking its `[keys]`/menu-routing key — "Switch theme…" and the
/// bare "Switch theme" both key under exactly `switch_theme`. A law test
/// ([`tests::a_trailing_ellipsis_never_forks_a_config_key`]) pins that they can't
/// diverge.
pub fn slug(name: &str) -> String {
    name.trim().trim_end_matches('…').trim().to_ascii_lowercase().replace(' ', "_")
}

/// Resolve a config `[keys]` action NAME to its `Action`. Matches the slugified
/// command name, so both the human label ("Switch theme") and the snake_case form
/// ("switch_theme") work. `None` for an unknown name (the rebinder then skips it).
/// All catalog actions are nullary, so the clone is cheap and total.
pub fn action_for_name(name: &str) -> Option<Action> {
    let want = slug(name);
    COMMANDS
        .iter()
        .find(|c| slug(c.name) == want)
        .map(|c| c.action.clone())
}

/// The config SLUG of the catalog command that dispatches `action`, or `None` when
/// no catalog command carries it (a char/line arrow motion / self-insert / prefix). The
/// SILENT USAGE LEDGER (`crate::stats`) keys its per-command counts off this — the SAME
/// command identity `record_recent` uses (`COMMANDS[i].action == action`), so the
/// ledger and the Recent MRU agree on what counts as "a command". Cheap for the
/// hot path: a non-catalog `action` (typing / arrow motion) returns `None` WITHOUT
/// allocating (the `slug` clone happens only on a real catalog match). NOTE: the
/// curated NAVIGATION motions ARE catalog rows now (rebindable — see the module doc),
/// so they resolve to a slug here; the ledger's own dispatch seam
/// (`App::ledger_note_dispatch`) gates `Action::is_motion` out separately, keeping
/// navigation off the discoverability ledger.
///
/// Native-only (`cfg(not(target_arch = "wasm32"))`): its only callers are the
/// silent command-usage ledger's App-side wiring (`app/stats.rs`), itself
/// native-only (no lifetime odometer on the web build).
#[cfg(not(target_arch = "wasm32"))]
pub fn slug_for_action(action: &Action) -> Option<String> {
    COMMANDS.iter().find(|c| &c.action == action).map(|c| slug(c.name))
}

/// Whether the catalog command with config `slug` carries a NATIVE (macOS) chord — the
/// "has a chord to graduate INTO" predicate the graduation ranking keys on (injected
/// into [`crate::stats::Stats::graduation_candidates`] so the pure ledger query stays
/// catalog-free). `false` for an unknown slug or a palette-only command (empty native
/// slot).
///
/// Native-only, matching [`slug_for_action`]: called only from `app/stats.rs`.
#[cfg(not(target_arch = "wasm32"))]
pub fn has_native_chord(slug_want: &str) -> bool {
    COMMANDS.iter().any(|c| slug(c.name) == slug_want && !c.native.trim().is_empty())
}

/// The DISCOVERABILITY row for a command `slug`: its NATIVE (macOS) chord as modifier
/// glyphs (`keyspec::mac_glyph_chord`) + its display name (ellipsis stripped), or `None`
/// when the slug is unknown OR palette-only (no native chord to teach). The shared
/// resolver behind BOTH the hold-⌘ peek's personalized rows ([`crate::peek::PeekRow`])
/// and the Keybindings footer's tip lines, so the two surfaces name a shortcut
/// identically. Called on the SLOW-DOOR graduation candidates the ledger ranks, every
/// one of which passed [`has_native_chord`], so the `None` arm is only the defensive
/// unknown-slug case.
///
/// Native-only, matching [`slug_for_action`]: called only from `app/stats.rs`.
#[cfg(not(target_arch = "wasm32"))]
pub fn peek_row_for_slug(slug_want: &str) -> Option<crate::peek::PeekRow> {
    let c = COMMANDS.iter().find(|c| slug(c.name) == slug_want)?;
    if c.native.trim().is_empty() {
        return None;
    }
    let chord = resolved_native_label(c, Convention::current());
    if chord.is_empty() {
        return None;
    }
    Some(crate::peek::PeekRow { chord, name: c.name.trim_end_matches('…').trim().to_string() })
}

/// The EFFECTIVE binding label per command, parallel to [`names`], showing BOTH
/// slots. When a config `[keys]` override lists valid chord(s) for the command's
/// action, those (up to 2) are shown joined by `·`; otherwise the static native +
/// emacs defaults are shown. Drives the palette's binding column, so it teaches the
/// chords that ACTUALLY trigger each command. `keys` is the config `[keys]` list;
/// `keep` is the config `linux_keep_emacs` list (see [`join_slots_truthful`]'s doc
/// for what it does to a STATIC label — a `[keys]` OVERRIDE row is unaffected,
/// since an explicit override already says exactly what fires).
pub fn effective_bindings(keys: &[(String, Vec<String>)], keep: &[String]) -> Vec<String> {
    COMMANDS.iter().map(|c| effective_binding_for(c, keys, keep, Platform::current())).collect()
}

/// The EFFECTIVE binding LABEL for ONE command — the per-command body
/// [`effective_bindings`] maps over, factored out so [`visible_effective_bindings`]
/// (the platform-filtered sibling) can share it without a second copy. `platform`
/// is explicit (mirrors [`resolved_native_label_truthful`]'s own testability
/// param) — every real caller passes [`Platform::current`].
fn effective_binding_for(
    c: &Command,
    keys: &[(String, Vec<String>)],
    keep: &[String],
    platform: Platform,
) -> String {
    let convention = Convention::current();
    let chords = effective_chords(c, keys);
    if effective_is_override(c, keys) {
        // A `[keys]` override is CONVENTION-AGNOSTIC (taken literally on every
        // platform — the chord VALUE never gets Cmd→Ctrl translated), but its
        // DISPLAY GLYPHS still route through the ONE resolved label owner: slot 1
        // (index 0) is NATIVE → convention glyphs (mac ⌘ / Linux word labels);
        // slot 2+ is EMACS → terse text, matching the static `join_slots` rule.
        chords
            .iter()
            .enumerate()
            .map(|(i, ch)| {
                if i == 0 {
                    match convention {
                        Convention::Mac => crate::keyspec::mac_glyph_chord(ch),
                        Convention::Linux => crate::keyspec::linux_glyph_chord(ch),
                    }
                } else {
                    ch.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" · ")
    } else {
        join_slots_truthful(c, convention, platform, keep)
    }
}

/// THE WEB CHORD SANITY ROUND — THE LABEL-TRUTH OWNER for a command's STATIC
/// (non-override) two-slot label. Supersedes the old Mac-`join_slots` /
/// Linux-`join_slots_resolved` split with ONE function that joins `c`'s
/// resolved-native + emacs labels for `convention`, but DROPS either half that
/// would not actually fire:
///   - **Tier 2 (web-reserved):** the resolved native chord is a browser
///     accelerator no page can intercept ([`crate::webreserved::is_reserved`]) —
///     checked ONLY on [`Platform::Web`], since a native build's chords are
///     never browser-shadowed.
///   - **Tier 3 (Linux-displaced):** the static emacs default is quietly
///     DISPLACED by [`Convention::Linux`]'s collision table
///     ([`crate::keymap::linux_displaces_emacs_default`]) — checked on EITHER
///     platform, since the collision is a property of the DISPATCH TABLE (a
///     native Linux desktop build has it too), not of being on the web.
///   - **Tier 4 (emacs-hands-on-Linux — the `linux_keep_emacs` config, THE
///     PER-CHORD DOOR this round adds):** `keep` is the config
///     `linux_keep_emacs` list — chords a Linux hand asked to keep their emacs
///     meaning, suppressing that letter's NATIVE-WINS displacement for exactly
///     that chord (see `keymap.rs`'s `KeymapState::linux_keeps` — the SAME
///     `keep` list gates the real dispatch, so a label shown here can never
///     lie about what actually fires). This is TWO-SIDED, mirroring the
///     collision itself: (a) [`crate::keymap::linux_displaces_emacs_default`]
///     is now `keep`-aware — a kept chord is NOT displaced, so its emacs label
///     reappears; (b) the NATIVE command that used to claim that Linux chord
///     must stop advertising it (`native_suppressed` below) — a chord this
///     table shows must be the one that actually wins.
///
/// On `Convention::Mac` + `Platform::Native` (macOS native) NONE of the three
/// checks can ever fire (`Platform::Web` is false; `convention == Linux` is
/// false, so both the Tier-3 displacement AND the Tier-4 keep-list are
/// structurally inert — `keep` is ignored outright on Mac, by construction),
/// so this is BYTE-IDENTICAL to the old `join_slots(c.native, c.emacs)` there —
/// the hard law this round must not break (see
/// `tests::mac_native_label_truth_is_byte_identical_to_join_slots`).
fn join_slots_truthful(c: &Command, convention: Convention, platform: Platform, keep: &[String]) -> String {
    let native_suppressed = convention == Convention::Linux
        && crate::keymap::linux_keeps_chord(keep, &resolved_native(c, convention));
    let native_label = if native_suppressed {
        String::new()
    } else {
        resolved_native_label_truthful(c, convention, platform)
    };

    let emacs_displaced = convention == Convention::Linux
        && crate::keymap::linux_displaces_emacs_default(c.emacs, keep);
    let emacs_label: &str = if emacs_displaced { "" } else { c.emacs };

    match (native_label.is_empty(), emacs_label.trim().is_empty()) {
        (false, false) => format!("{native_label} · {emacs_label}"),
        (false, true) => native_label,
        (true, false) => emacs_label.to_string(),
        (true, true) => String::new(),
    }
}

/// THE GUIDE'S GENERATED KEYS REFERENCE — the drift-proof source for the fenced
/// table between `<!-- GENERATED:keys-reference:BEGIN -->` /
/// `<!-- GENERATED:keys-reference:END -->` in `GUIDE.md`. Every catalog command,
/// its resolved DEFAULT (config-free) chord label under EACH convention — mac
/// glyphs on [`Convention::Mac`], Linux words on [`Convention::Linux`] — via the
/// SAME [`join_slots_truthful`] the palette itself reads (`Platform::Native`
/// throughout: both columns describe an OS convention, not the browser build, so
/// the web-reserved tier never fires here; the Linux-displaced tier DOES, since
/// that collision is a property of the dispatch table on ANY Linux build). The
/// LINUX column's `keep` list is [`crate::config::Config::empty`]'s
/// `effective_linux_keep()` — the DEFAULT, config-free composition (just
/// `keymap::linux_builtin_keep()`, under the default `native` flavor) — so a
/// command like Insert link, unbound on Linux out of the box, correctly shows
/// an empty Linux cell rather than a chord no default install would ever
/// actually honor. The LAW TEST living beside `GUIDE_MD` (`guide::tests::
/// generated_keys_reference_matches_catalog`) regenerates this and diffs it
/// byte-for-byte against the checked-in section — a catalog change (new
/// command, new default chord) fails that test until the doc is regenerated
/// and pasted back in. Regenerate with:
/// `cargo test --bin awl guide::tests::print_generated_keys_reference -- --ignored --nocapture`
#[cfg(test)]
pub(crate) fn generate_keys_reference_markdown() -> String {
    let mut out = String::new();
    out.push_str("| Command | macOS | Linux |\n");
    out.push_str("|---|---|---|\n");
    let default_linux_keep = crate::config::Config::empty().effective_linux_keep();
    for c in COMMANDS.iter() {
        let mac = join_slots_truthful(c, Convention::Mac, Platform::Native, &[]);
        let linux = join_slots_truthful(c, Convention::Linux, Platform::Native, &default_linux_keep);
        out.push_str(&format!("| {} | {mac} | {linux} |\n", c.name));
    }
    out
}

/// The EFFECTIVE chord LIST for one command (NOT joined): a valid config override's
/// chords (up to 2) when present, else the command's static native/emacs slots
/// (empty slots dropped). The per-chord form [`effective_bindings`] joins for
/// display and [`binding_conflict`] compares for clashes. `pub(crate)` — `whichkey.rs`
/// reads it directly (over the platform-filtered [`visible`] commands) to derive
/// prefix continuations without a second binding-resolution copy.
pub(crate) fn effective_chords(c: &Command, keys: &[(String, Vec<String>)]) -> Vec<String> {
    if let Some(over) = override_chords(c, keys) {
        return over;
    }
    [c.native, c.emacs]
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// The VALID config-override chords for `c` (capped at 2), or `None` when the
/// command has no override (so the static defaults apply).
fn override_chords(c: &Command, keys: &[(String, Vec<String>)]) -> Option<Vec<String>> {
    keys.iter()
        .find(|(name, _)| slug(name) == slug(c.name) && action_for_name(name).is_some())
        .map(|(_, chords)| {
            chords
                .iter()
                .filter(|ch| crate::keymap::parse_binding(ch).is_ok())
                .take(2)
                .cloned()
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty())
}

fn effective_is_override(c: &Command, keys: &[(String, Vec<String>)]) -> bool {
    override_chords(c, keys).is_some()
}

/// CONFLICT check for the rebind menu: is `binding` already an effective chord of a
/// command OTHER than `exclude_slug`? Returns the conflicting command's display NAME
/// (the first match) so the menu can warn "already bound to X" before/while writing.
/// Bindings are compared CANONICALLY (`Cmd-S` == `s-s`), so equivalent spellings
/// clash; an unparseable `binding` never conflicts (returns `None`).
pub fn binding_conflict(
    binding: &str,
    exclude_slug: &str,
    keys: &[(String, Vec<String>)],
) -> Option<&'static str> {
    let want = crate::keyspec::canonical_binding(binding)?;
    COMMANDS
        .iter()
        .filter(|c| slug(c.name) != exclude_slug)
        .find(|c| {
            effective_chords(c, keys)
                .iter()
                .any(|ch| crate::keyspec::canonical_binding(ch).as_deref() == Some(want.as_str()))
        })
        .map(|c| c.name)
}

/// The catalog command NAMES, in catalog order — the UNFILTERED full-catalog
/// baseline (see [`visible_names`] for the real, platform-filtered corpus a live
/// build actually fuzzy-matches over). Test-only: kept for tests that deliberately
/// want to enumerate every command, native or not.
#[cfg(test)]
pub fn names() -> Vec<String> {
    COMMANDS.iter().map(|c| c.name.to_string()).collect()
}

/// The catalog DEFAULT binding labels, parallel to [`names`], each joining the
/// command's two slots (`"Cmd-S · C-x C-s"`). The live/headless palette uses
/// [`effective_bindings`] (which overlays config rebinds); this stays as the
/// defaults baseline + test surface.
#[allow(dead_code)]
pub fn bindings() -> Vec<String> {
    COMMANDS
        .iter()
        .map(|c| join_slots(c.native, c.emacs))
        .collect()
}

// ── PLATFORM-SCOPED COMMANDS: the ONE filtered view ────────────────────────────
//
// `COMMANDS` stays the raw, full catalog (every test that wants to enumerate every
// command — native or not — still reads it directly, or via `names()`/`bindings()`
// above, which are DELIBERATELY unfiltered so a native-run test can pin the FULL
// catalog). Every USER-FACING surface (the palette build, the rebind menu build, the
// palette's Enter/accept path, the rebind menu's Delete-to-reset + capture-prompt
// doors, which-key, and the awl-rendered + native menu bars) instead routes through
// `visible()` (and its `visible_*` siblings below) — the ONE narrowed view a command's
// `native_only` flag ever reaches through. A "corpus row index" downstream of
// `visible()` is an index into ITS OWN Vec, never into `COMMANDS` directly — that is
// what keeps a picker's displayed row and its Enter/accept action from ever drifting
// apart once some rows are hidden.

/// The catalog indices AVAILABLE on `platform`, in catalog order. The structural half
/// of the filtered view: [`visible`] narrows this to `Platform::current()`; a
/// native-run test can pass `Platform::Web` directly to assert the web-hidden view
/// without any `cfg!` gymnastics.
fn visible_indices_on(platform: Platform) -> Vec<usize> {
    COMMANDS.iter().enumerate().filter(|(_, c)| c.available_on(platform)).map(|(i, _)| i).collect()
}

/// The catalog commands AVAILABLE on `platform`, in catalog order — [`visible_on`]'s
/// data half of [`visible_indices_on`].
fn visible_on(platform: Platform) -> Vec<&'static Command> {
    visible_indices_on(platform).into_iter().map(|i| &COMMANDS[i]).collect()
}

/// The catalog commands available on THIS COMPILED PLATFORM (`Platform::current()`),
/// in catalog order — THE ONE FILTERED VIEW described above. On native this is
/// byte-identical to walking `COMMANDS` in order (nothing is hidden); on web it drops
/// every `native_only` row.
pub fn visible() -> Vec<&'static Command> {
    visible_on(Platform::current())
}

/// The command NAMES for [`visible`], in corpus order — the fuzzy corpus the palette
/// AND rebind-menu overlay builds filter over (replaces a bare [`names`] at both of
/// those two build sites; `names()` itself stays unfiltered for surfaces/tests that
/// deliberately want the full catalog).
pub fn visible_names() -> Vec<String> {
    visible().iter().map(|c| c.name.to_string()).collect()
}

/// The EFFECTIVE binding labels for [`visible`], parallel to [`visible_names`] — the
/// platform-filtered sibling of [`effective_bindings`], sharing its per-command body
/// (`effective_binding_for`) so the two can never compute a binding label differently.
pub fn visible_effective_bindings(keys: &[(String, Vec<String>)], keep: &[String]) -> Vec<String> {
    visible().iter().map(|c| effective_binding_for(c, keys, keep, Platform::current())).collect()
}

/// The EFFECTIVE chord LISTS for [`visible`], parallel to [`visible_names`] — each
/// command's active chords (a valid config override, else the static native/emacs
/// slots), UN-joined and un-glyphified (empty slots dropped), narrowed to the
/// platform-visible set. This is what which-key (`crate::whichkey::continuations`)
/// derives its prefix rows from, so a hidden command's chord (if it happened to
/// start with a prefix) never surfaces as a continuation on web.
pub fn visible_effective_chord_lists(keys: &[(String, Vec<String>)]) -> Vec<Vec<String>> {
    visible().iter().map(|c| effective_chords(c, keys)).collect()
}

/// Translate a VISIBLE-CORPUS row index (as built by [`visible_names`] /
/// [`visible_effective_bindings`] — the palette's and rebind menu's actual corpus) back
/// to the real catalog `Action` it dispatches. Replaces the old direct `COMMANDS[i]`
/// index into the RAW catalog at the palette's Enter/accept seam
/// (`actions::overlay_nav`), which would silently mis-map once some rows are hidden.
/// Panics out of range — only a picker's own corpus-selected index (always
/// `< visible().len()`) reaches this.
pub fn visible_action_of(corpus_i: usize) -> Action {
    visible()[corpus_i].action.clone()
}

/// The slug of a VISIBLE-CORPUS row index — the rebind menu's Delete-to-reset door
/// (replaces the old raw-catalog `slug_of_index`, since removed for having no
/// caller left once every door routed through the visible-corpus indices).
pub fn visible_slug_of(corpus_i: usize) -> String {
    slug(visible()[corpus_i].name)
}

/// The display NAME of a VISIBLE-CORPUS row index — the rebind capture's prompt door
/// (replaces the old raw-catalog `name_of_index`, since removed for the same reason
/// as `visible_slug_of`'s).
pub fn visible_name_of(corpus_i: usize) -> &'static str {
    visible()[corpus_i].name
}

/// The recently-run-command MRU ([`recent_indices`], catalog-index space), translated
/// into VISIBLE-CORPUS row indices — dropping any catalog index that isn't visible on
/// this platform (a hidden command, if somehow ever recorded, can never show as
/// "recent"). The one door that feeds a built `OverlayState.recent` (corpus-index
/// space), so a stale catalog index there can never point at the wrong visible row.
pub fn visible_recent_indices() -> Vec<usize> {
    let idx = visible_indices_on(Platform::current());
    recent_indices().into_iter().filter_map(|catalog_i| idx.iter().position(|&v| v == catalog_i)).collect()
}

/// The DISPATCH-time gate: is `action` available on `platform`? `true` for any action
/// with NO catalog entry (a motion / self-insert / non-catalog effect always fires, and
/// there is nothing to hide) and for a catalog action that IS available; `false` for a
/// `native_only` catalog action on `Web` OR a `web_only` catalog action on `Native`.
/// This is the BELT to `visible`'s BRACES: even if a chord is still configured/rebound
/// to fire a hidden command, or a stray `Effect::RunAction` re-dispatch names one
/// directly, this stops the actual mutation — hiding a picker row alone is not enough
/// (a keymap chord bypasses the picker entirely). Cheap: at most `COMMANDS.len()` (60)
/// enum comparisons, no allocation. (Was a Native short-circuit before `web_only`
/// existed — now a plain `available_on` lookup on both platforms, since a `web_only`
/// row must actually be gated on Native too.)
pub fn action_available(action: &Action, platform: Platform) -> bool {
    match COMMANDS.iter().find(|c| &c.action == action) {
        Some(c) => c.available_on(platform),
        None => true,
    }
}

// ── The command palette's FACETING scheme (All · File · Edit · View · Recent) ──
//
// The Cmd-P palette is a faceting picker (see `crate::facets`): ←/→ regroup the flat
// catalog under a lens. File / Edit / View mirror the macOS menu bar's grouping;
// Recent lists the most-recently-run commands.
//
// SINGLE-OWNER NOTE (menu section): the task calls for reusing `menu.rs`'s section
// table so there is no second hand-maintained category map. `menu.rs` is, however,
// `#![cfg(target_os = "macos")]` — its `SECTIONS` cannot be referenced from this
// CROSS-PLATFORM palette code. So the SEMANTIC owner of "which menu section a command
// belongs to" lives HERE, in [`menu_section`] (compiled on every target), and the
// macOS `menu.rs` is checked AGAINST it by a drift-guard test
// (`menu::tests::routed_sections_match_command_section`), so the menu's File/Edit/View
// arrays and this owner can never silently disagree — one source of truth, guarded.

/// The catalog command NAMES the macOS menu bar files under **File** — the EXACT
/// display names (ellipsis included), so both the palette faceting (keyed off the
/// display name) and the menu drift-guard read one source of truth.
const FILE_COMMANDS: &[&str] =
    &["New note", "Browse files…", "Switch project…", "Recent projects…", "Save", "Finish file"];
/// … under **Edit**.
const EDIT_COMMANDS: &[&str] = &["Undo", "Redo", "Cut", "Copy", "Paste", "Select all"];
/// … under **View**.
const VIEW_COMMANDS: &[&str] = &[
    "Toggle page mode",
    "Switch theme…",
    "Zoom in",
    "Zoom out",
    "Reset zoom",
    "Toggle debug",
];

/// The menu SECTION (`"File"` / `"Edit"` / `"View"`) command `name` sits under, or
/// `None` for a command in no menu section (the App-menu About/Quit, or any command
/// not surfaced in the menu bar at all). The SINGLE owner of this mapping, consulted
/// by both the palette's File/Edit/View lenses (every platform) and the macOS menu's
/// own drift-guard test — see the module note above.
pub fn menu_section(name: &str) -> Option<&'static str> {
    if FILE_COMMANDS.contains(&name) {
        Some("File")
    } else if EDIT_COMMANDS.contains(&name) {
        Some("Edit")
    } else if VIEW_COMMANDS.contains(&name) {
        Some("View")
    } else {
        None
    }
}

/// The command palette's lens strip: **All** (the flat catalog home) · **File** ·
/// **Edit** · **View** (the menu-section groups) · **Recent** (recently run). "All"
/// is parked FIRST (strip index 0), per the settled convention.
const COMMAND_FACET_STRIP: [Facet; 5] = [
    Facet { label: "All", id: "all", sections: &[] },
    Facet { label: "File", id: "file", sections: &["File"] },
    Facet { label: "Edit", id: "edit", sections: &["Edit"] },
    Facet { label: "View", id: "view", sections: &["View"] },
    Facet { label: "Recent", id: "recent", sections: &["Recent"] },
];

/// The command palette's [`FacetScheme::bucket`], keyed by strip index (see
/// [`COMMAND_FACET_STRIP`]). File/Edit/View delegate to [`menu_section`] over the
/// command NAME (`item.accept`); Recent reads the per-item `recent` flag (populated
/// from the in-memory MRU when the palette is built). A command in no menu section
/// opts out of File/Edit/View (`None` — still reachable under All).
fn command_bucket(item: FacetItem, lens_idx: usize) -> Option<&'static str> {
    match lens_idx {
        1 => (menu_section(item.accept) == Some("File")).then_some("File"),
        2 => (menu_section(item.accept) == Some("Edit")).then_some("Edit"),
        3 => (menu_section(item.accept) == Some("View")).then_some("View"),
        4 => item.recent.then_some("Recent"), // Recent
        _ => None,
    }
}

/// The command palette's registered [`FacetScheme`], handed back by
/// [`crate::facets::scheme`] for [`crate::overlay::OverlayKind::Command`].
pub static COMMAND_FACETS: FacetScheme =
    FacetScheme { strip: &COMMAND_FACET_STRIP, bucket: command_bucket };

// ── Recently-run commands (an in-memory MRU, NOT persisted) ────────────────────
//
// The palette's Recent lens is sourced from a process-global MRU of catalog indices,
// recorded whenever a command is RUN from the palette. It is deliberately in-memory
// only (no disk store this round) — a fresh process starts empty, so a headless
// capture's Recent lens is inert (nothing recorded), honoring the determinism gate.
// Recording is LIVE-APP-ONLY ([`crate::app`]'s `Effect::RunAction` handler), never the
// shared/headless core, so the capture path never mutates this global.

/// How many recently-run commands the MRU remembers.
const RECENT_CAP: usize = 12;

/// The in-memory recently-run-command MRU: catalog indices, most-recent FIRST.
static RECENT: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// Record that the command dispatching `action` was just RUN (from the palette),
/// moving its catalog index to the front of the MRU (deduped, capped at
/// [`RECENT_CAP`]). A no-op for an `action` no catalog command carries. LIVE-ONLY:
/// called from the App's palette-run seam, never the headless replay.
pub fn record_recent(action: &Action) {
    let Some(i) = COMMANDS.iter().position(|c| &c.action == action) else {
        return;
    };
    if let Ok(mut mru) = RECENT.lock() {
        mru.retain(|&x| x != i);
        mru.insert(0, i);
        mru.truncate(RECENT_CAP);
    }
}

/// The recently-run catalog indices (most-recent first) for the palette's Recent
/// lens. Empty in a fresh process (so a headless capture's Recent lens is inert).
pub fn recent_indices() -> Vec<usize> {
    RECENT.lock().map(|m| m.clone()).unwrap_or_default()
}

/// TEST-ONLY: reset the recently-run MRU (so a test that exercises `record_recent`
/// leaves no residue for a later test reading [`recent_indices`]).
#[cfg(test)]
pub fn clear_recent() {
    if let Ok(mut mru) = RECENT.lock() {
        mru.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── THE KEYMAP-DEFAULTS-AS-DATA ROUND — LAW TESTS ON THE DATA ──────────────
    //
    // These pin the embedded `assets/keymap-defaults.toml` against the catalog
    // it feeds: every slug it names is a real `COMMAND_SEED` entry, every
    // `COMMAND_SEED` entry is either named there or explicitly unbound, and the
    // spliced `COMMANDS` that results is what every other test in this module
    // (which predates this round and is otherwise UNCHANGED) already exercises
    // — `catalog_and_keymap_agree_on_every_default_chord` and
    // `no_two_catalog_commands_share_a_default_chord` below now run against
    // DATA-SOURCED values without themselves needing an edit.

    #[test]
    fn every_defaults_toml_slug_names_a_real_catalog_command() {
        for slug_in_file in crate::keymap_defaults::command_defaults().keys() {
            assert!(
                COMMAND_SEED.iter().any(|c| &slug(c.name) == slug_in_file),
                "assets/keymap-defaults.toml names {slug_in_file:?}, which is not a commands::COMMAND_SEED slug"
            );
        }
    }

    #[test]
    fn every_catalog_command_appears_in_the_defaults_toml_or_is_unbound() {
        // "explicitly listed unbound" per the round's own law — this codebase's
        // embedded file lists EVERY command (even the palette-only, all-empty
        // ones), so this degenerates to "every slug is present"; still asserted
        // directly (not merely implied by the reverse-direction test above) so a
        // future author who starts omitting all-empty rows doesn't silently
        // regress past this law without a compile-time nudge.
        let defaults = crate::keymap_defaults::command_defaults();
        for c in COMMAND_SEED.iter() {
            assert!(
                defaults.contains_key(&slug(c.name)),
                "{:?} (slug {:?}) has no entry in assets/keymap-defaults.toml — every catalog \
                 command must appear there, even if unbound (both slots empty)",
                c.name,
                slug(c.name)
            );
        }
    }

    #[test]
    fn defaults_toml_has_no_stale_slugs_and_no_duplicates() {
        // The reverse-coverage pair above already proves a 1:1 SET correspondence;
        // this additionally proves the embedded file has exactly as many entries
        // as the catalog (so a renamed command can't leave an orphaned old-slug
        // row silently sitting in the TOML alongside its new one).
        let defaults = crate::keymap_defaults::command_defaults();
        assert_eq!(
            defaults.len(),
            COMMAND_SEED.len(),
            "assets/keymap-defaults.toml's entry count must equal the catalog's — an orphaned \
             or duplicated slug would slip past the pure set-membership checks alone"
        );
    }

    #[test]
    fn commands_splices_the_embedded_defaults_verbatim() {
        // THE SINGLE-SOURCE LAW, checked directly: `COMMANDS[i].native`/`.emacs`
        // is EXACTLY what `assets/keymap-defaults.toml` names for that command's
        // slug (never a residual literal from `COMMAND_SEED`, which carries only
        // `""` placeholders in both slots by construction).
        let defaults = crate::keymap_defaults::command_defaults();
        for c in COMMANDS.iter() {
            let (native, emacs) = defaults.get(&slug(c.name)).cloned().unwrap_or_default();
            assert_eq!(c.native, native, "{:?}'s native slot must come from the embedded defaults", c.name);
            assert_eq!(c.emacs, emacs, "{:?}'s emacs slot must come from the embedded defaults", c.name);
        }
    }

    #[test]
    fn command_seed_itself_carries_no_residual_chord_literals() {
        // A belt-and-suspenders structural check: `COMMAND_SEED`'s own
        // `native`/`emacs` fields (never read by anything but the `COMMANDS`
        // splice above) must stay blank placeholders — a stray literal chord
        // reintroduced there would silently be DISCARDED by the splice (which
        // always overwrites both fields), so this catches the authoring mistake
        // even though it would otherwise have zero runtime effect.
        for c in COMMAND_SEED.iter() {
            assert_eq!(c.native, "", "{:?}: COMMAND_SEED must not carry a literal native chord", c.name);
            assert_eq!(c.emacs, "", "{:?}: COMMAND_SEED must not carry a literal emacs chord", c.name);
        }
    }

    #[test]
    fn catalog_non_empty_and_named() {
        assert!(!COMMANDS.is_empty(), "the command catalog must list commands");
        for c in COMMANDS.iter() {
            assert!(!c.name.trim().is_empty(), "command needs a display name");
        }
        // Every entry HAS at least one filled slot except the PALETTE-ONLY commands
        // (summoned by name, no default chord); the model is CAPPED at 2 — exactly
        // the two slots exist. The identity round RETIRED the emacs C-x defaults, so
        // the palette-only set grew: every command whose C-x default was retired
        // WITHOUT gaining a native chord (Browse files… / Move note… / Toggle page
        // mode / Toggle caret style / Widen page / Narrow page / Toggle debug) joins
        // the pre-existing bindless set here. Settings… and Finish file left this set
        // in the keybinding-idiom audit (P1 = Cmd-,, P5 = Cmd-W). About's + Recent
        // projects' other summon door is the macOS menu bar, not a keymap chord.
        //
        // The markdown formatting commands are MOSTLY palette-only (like Align table);
        // the exceptions are Bold (Cmd-B), Italic (Cmd-I), and Inline code (Cmd-E) —
        // the universal trio, all three now bound (the audit's Option B moved the held
        // stats HUD off plain Cmd-I) — and Task list (Cmd-Shift-L, the Apple Notes
        // checklist idiom, W3), which are NOT exempt — the assertions below verify
        // theirs.
        const PALETTE_ONLY: &[&str] = &[
            "Keybindings…",
            "Caret style…",
            "Dictionary…",
            "Toggle spellcheck",
            "Toggle writing nits",
            "Reset page width",
            "About",
            "Credits",
            "Guide",
            "Lifetime stats",
            "Line endings…",
            "Align table",
            "Report a Problem",
            "Download file",
            "Check for Updates",
            "Recent projects…",
            "Go to heading…",
            "Toggle typewriter scroll",
            "Toggle menu bar",
            "Keep version",
            "Clean unused assets…",
            // Emacs C-x default retired, no native chord assigned (identity round):
            "Browse files…",
            "Move note…",
            // NOTES VERBS round — same shape as Move note…, no native chord.
            "Rename note…",
            "Duplicate note",
            "Toggle page mode",
            "Toggle caret style",
            "Widen page",
            "Narrow page",
            "Toggle debug",
            // Format toggles with no native convention:
            "Blockquote",
            "Bullet list",
            "Numbered list",
            "Heading",
            "Code block",
            "Highlight",
            "Strikethrough",
        ];
        for c in COMMANDS.iter() {
            if !PALETTE_ONLY.contains(&c.name) {
                assert!(
                    !join_slots(c.native, c.emacs).is_empty(),
                    "command {} needs at least one binding slot",
                    c.name
                );
            }
        }
        // names()/bindings() stay parallel to the catalog.
        assert_eq!(names().len(), COMMANDS.len());
        assert_eq!(bindings().len(), COMMANDS.len());
    }

    #[test]
    fn command_facets_land_on_all_home_then_group_by_menu_section() {
        // "All" is the FIRST lens (strip index 0) with no sections — the flat home a
        // faceting picker lands on, per the settled convention.
        assert_eq!(COMMAND_FACETS.strip[0].id, "all");
        assert!(COMMAND_FACETS.strip[0].sections.is_empty());
        // The strip in order: All · File · Edit · View · Recent.
        let ids: Vec<&str> = COMMAND_FACETS.strip.iter().map(|f| f.id).collect();
        assert_eq!(ids, vec!["all", "file", "edit", "view", "recent"]);
    }

    #[test]
    fn menu_section_buckets_known_commands() {
        assert_eq!(menu_section("Save"), Some("File"));
        assert_eq!(menu_section("New note"), Some("File"));
        assert_eq!(menu_section("Copy"), Some("Edit"));
        assert_eq!(menu_section("Select all"), Some("Edit"));
        assert_eq!(menu_section("Switch theme…"), Some("View"));
        assert_eq!(menu_section("Toggle debug"), Some("View"));
        // App-menu + un-menued commands sit in no palette section.
        assert_eq!(menu_section("Quit"), None);
        assert_eq!(menu_section("About"), None);
        assert_eq!(menu_section("Settings"), None);
        // Every FILE/EDIT/VIEW name is a real catalog command (no typo → dead lens).
        for name in FILE_COMMANDS.iter().chain(EDIT_COMMANDS).chain(VIEW_COMMANDS) {
            assert!(
                COMMANDS.iter().any(|c| &c.name == name),
                "menu-section name {name:?} is not a catalog command"
            );
        }
    }

    #[test]
    fn command_bucket_routes_each_lens() {
        // File lens (strip index 1): only File-section commands land, under "File".
        assert_eq!(command_bucket(FacetItem::new("Save"), 1), Some("File"));
        assert_eq!(command_bucket(FacetItem::new("Copy"), 1), None); // Edit, not File
        // Edit (2) / View (3) likewise.
        assert_eq!(command_bucket(FacetItem::new("Copy"), 2), Some("Edit"));
        assert_eq!(command_bucket(FacetItem::new("Switch theme…"), 3), Some("View"));
        // Recent (4) keys off the per-item flag, independent of menu section.
        let mut recent = FacetItem::new("Undo");
        recent.recent = true;
        assert_eq!(command_bucket(recent, 4), Some("Recent"));
        assert_eq!(command_bucket(FacetItem::new("Undo"), 4), None); // not flagged
        // The All home (index 0) never groups.
        assert_eq!(command_bucket(FacetItem::new("Save"), 0), None);
    }

    #[test]
    fn recent_mru_records_newest_first_deduped_and_capped() {
        clear_recent();
        assert!(recent_indices().is_empty(), "fresh process starts empty");
        record_recent(&Action::Undo);
        record_recent(&Action::Redo);
        record_recent(&Action::Undo); // re-run moves it to front, no dup
        let undo = COMMANDS.iter().position(|c| c.action == Action::Undo).unwrap();
        let redo = COMMANDS.iter().position(|c| c.action == Action::Redo).unwrap();
        assert_eq!(recent_indices(), vec![undo, redo]);
        clear_recent(); // leave no residue for other tests reading the global
    }

    #[test]
    fn action_for_name_matches_label_and_slug() {
        // Both the human label and the snake_case slug resolve to the same action.
        assert_eq!(action_for_name("Switch theme"), Some(Action::OpenThemeMenu));
        assert_eq!(action_for_name("switch_theme"), Some(Action::OpenThemeMenu));
        assert_eq!(action_for_name("go_to_file"), Some(Action::OpenGoto));
        assert_eq!(action_for_name("settings"), Some(Action::OpenSettingsMenu));
        // The DEBUG frame counter is a palette command, so it is rebindable via the
        // config `[keys]` action name ("toggle_debug").
        assert_eq!(action_for_name("Toggle debug"), Some(Action::ToggleDebug));
        assert_eq!(action_for_name("toggle_debug"), Some(Action::ToggleDebug));
        // The persistent margin outline is a palette command too, rebindable via the
        // config `[keys]` action name ("toggle_outline").
        assert_eq!(action_for_name("Toggle outline"), Some(Action::ToggleOutline));
        assert_eq!(action_for_name("toggle_outline"), Some(Action::ToggleOutline));
        // Toggle spellcheck is likewise a real Action, rebindable via
        // "toggle_spellcheck" (as is Toggle writing nits now — no more sentinel).
        assert_eq!(action_for_name("Toggle spellcheck"), Some(Action::ToggleSpellcheck));
        assert_eq!(action_for_name("toggle_spellcheck"), Some(Action::ToggleSpellcheck));
        // The held stats HUD is NOT a palette command — it is a momentary HOLD-to-peek, so
        // a discrete selection (with no key-release to dismiss it) would leave it stuck on.
        // It is summoned ONLY by the held Option-Cmd-I chord (`keymap.rs`), never
        // the catalog.
        assert_eq!(action_for_name("Stats HUD"), None);
        assert_eq!(action_for_name("stats_hud"), None);
        assert_eq!(action_for_name("nope"), None);
    }

    #[test]
    fn a_trailing_ellipsis_never_forks_a_config_key() {
        // THE ELLIPSIS GATE: the `…` picker suffix is DISPLAY-ONLY — `slug` strips it,
        // so a command shown as "Switch theme…" keys under exactly `switch_theme`, the
        // SAME key a `[keys]` entry or the menu-routing table derives. This law pins
        // that a `…` can never fork a second config key.
        for c in COMMANDS.iter() {
            let s = slug(c.name);
            assert!(!s.contains('…'), "{}: slug must not carry the ellipsis: {s:?}", c.name);
            // The suffixed display name and its bare form slug IDENTICALLY, and both
            // resolve to the same action through `action_for_name`.
            let bare = c.name.trim_end_matches('…').trim();
            assert_eq!(slug(bare), s, "{}: bare and suffixed forms must slug the same", c.name);
            assert_eq!(action_for_name(c.name), Some(c.action.clone()), "{}: suffixed rebind", c.name);
            assert_eq!(action_for_name(bare), Some(c.action.clone()), "{}: bare rebind", c.name);
        }
        // Concretely, both spellings (and the ellipsis-suffixed slug) collapse to one
        // key / one action.
        assert_eq!(slug("Switch theme…"), "switch_theme");
        assert_eq!(slug("Switch theme"), "switch_theme");
        assert_eq!(action_for_name("switch_theme…"), Some(Action::OpenThemeMenu));
    }

    /// CONVENTION-PARAMETRIC glyph helper for these two tests: glyphify a literal
    /// chord SPEC (an override value, taken literally — never Cmd→Ctrl
    /// translated, per `effective_binding_for`'s own doc) through the SAME two
    /// pure resolvers it calls, for whichever convention is ambient.
    fn glyph(spec: &str) -> String {
        match Convention::current() {
            Convention::Mac => crate::keyspec::mac_glyph_chord(spec),
            Convention::Linux => crate::keyspec::linux_glyph_chord(spec),
        }
    }

    /// CONVENTION-PARAMETRIC expected label for a catalog command's default
    /// (config-free) binding — the SAME resolver `bindings()`/`effective_bindings`
    /// themselves call (`resolved_native_label(c, Convention::current())`), so a
    /// test computing its expectation through this helper holds on EITHER
    /// convention rather than hardcoding the mac-only glyph form.
    fn label_for(name: &str) -> String {
        let c = COMMANDS.iter().find(|c| c.name == name).unwrap();
        resolved_native_label(c, Convention::current())
    }

    #[test]
    fn effective_bindings_reflect_overrides() {
        // No config: effective == default labels — a MAC-ONLY invariant.
        // `bindings()`/`join_slots` is explicitly documented as "the Mac
        // baseline" (always mac glyphs, never convention-resolved), while
        // `effective_bindings` IS convention-resolved (`Convention::current()`
        // via `effective_binding_for`) — so the two agree only when the ambient
        // convention actually IS Mac; under Linux they correctly diverge (Ctrl
        // word labels vs. the mac-glyph baseline) BY DESIGN.
        if Convention::current() == Convention::Mac {
            assert_eq!(effective_bindings(&[], &[]), bindings());
        }
        // An override for "switch_theme" surfaces in the palette column. Slot 1 (the
        // NATIVE slot) renders as the ACTIVE convention's chord glyphs (mac ⌃T /
        // Linux "Ctrl+T") — the override chord VALUE is taken literally on every
        // convention, only its DISPLAY glyphs vary.
        let keys = vec![("switch_theme".to_string(), vec!["C-t".to_string()])];
        let eff = effective_bindings(&keys, &[]);
        let i = COMMANDS.iter().position(|c| c.name == "Switch theme…").unwrap();
        assert_eq!(eff[i], glyph("C-t"));
        // A BAD chord falls back to the default label (consistent with the keymap) —
        // Switch theme's native default is now Cmd-T (the emacs C-x t is retired).
        let bad = vec![("switch_theme".to_string(), vec!["C-frobnicate".to_string()])];
        let eff = effective_bindings(&bad, &[]);
        assert_eq!(eff[i], label_for("Switch theme…"));
    }

    #[test]
    fn effective_bindings_show_both_slots() {
        // `bindings()` is explicitly documented as "the Mac baseline" — always
        // mac glyphs, convention-INDEPENDENT (see `join_slots`'s module doc) — so
        // every assertion against it stays a literal mac-glyph string
        // deliberately, unlike `effective_bindings` (which IS convention-
        // resolved and needs the `glyph`/`label_for` helpers below).
        //
        // Save's emacs C-x C-s default is retired, so it now shows only its
        // NATIVE slot as mac GLYPHS (`Cmd-S` → `⌘S`).
        let i = COMMANDS.iter().position(|c| c.name == "Save").unwrap();
        assert_eq!(bindings()[i], "⌘S");
        // A single-slot NATIVE command shows just its glyph form (no separator).
        let z = COMMANDS.iter().position(|c| c.name == "Zoom in").unwrap();
        assert_eq!(bindings()[z], "⌘=");
        // Go to file… is now the native Cmd-O door (its emacs C-x C-f is retired).
        let g = COMMANDS.iter().position(|c| c.name == "Go to file…").unwrap();
        assert_eq!(bindings()[g], "⌘O");
        // A command that keeps BOTH a native slot and a SURVIVING emacs chord still
        // joins them — Cut (Cmd-X · C-w): the C-w cut is a bare-control survivor.
        let cut = COMMANDS.iter().position(|c| c.name == "Cut").unwrap();
        assert_eq!(bindings()[cut], "⌘X · C-w");
        // Settings carries its native Cmd-, slot (P1) → the mac glyph label.
        let s = COMMANDS.iter().position(|c| c.name == "Settings…").unwrap();
        assert_eq!(bindings()[s], "⌘,");
        // A 2-chord config override surfaces BOTH chords, joined — slot 1
        // glyphified PER THE ACTIVE CONVENTION (this DOES route through
        // `effective_bindings`, the convention-resolved door), even when it
        // reclaims a retired chord (Save ← Cmd-S + C-x C-s).
        let keys = vec![("save".to_string(), vec!["Cmd-S".to_string(), "C-x C-s".to_string()])];
        assert_eq!(effective_bindings(&keys, &[])[i], format!("{} · C-x C-s", glyph("Cmd-S")));
        // Only the VALID chords of an override are shown; an invalid one is dropped.
        let mixed = vec![("save".to_string(), vec!["Cmd-S".to_string(), "C-frobnicate".to_string()])];
        assert_eq!(effective_bindings(&mixed, &[])[i], glyph("Cmd-S"));
    }

    #[test]
    fn settings_command_present() {
        // The "Settings" palette command now summons the faceted MENU (the friendly
        // default); the raw config-as-text `Action::OpenSettings` lives behind the
        // menu's "Edit config as text" row, not a catalog command of its own.
        assert!(COMMANDS.iter().any(|c| c.action == Action::OpenSettingsMenu));
    }

    #[test]
    fn line_endings_command_present_and_rebindable() {
        // "Line endings…" is a real palette command (no default chord, like
        // Settings/About) backed by `Action::ConvertLineEndings`, so it shows in Cmd-P
        // and is independently rebindable via the config `[keys] line_endings` (the
        // slug strips the display ellipsis).
        let c = COMMANDS
            .iter()
            .find(|c| c.name == "Line endings…")
            .expect("Line endings… must be in the catalog");
        assert_eq!(c.native, "");
        assert_eq!(c.emacs, "");
        assert_eq!(c.action, Action::ConvertLineEndings);
        // Rebindable by both the human label and the snake_case slug.
        assert_eq!(action_for_name("Line endings…"), Some(Action::ConvertLineEndings));
        assert_eq!(action_for_name("line_endings"), Some(Action::ConvertLineEndings));
    }

    #[test]
    fn follow_link_command_present_and_rebindable() {
        // "Follow link" is a real palette command backed by `Action::FollowLink`,
        // with the org-mode emacs chord `C-c C-o` and no native slot; it shows in
        // Cmd-P and is independently rebindable via `[keys] follow_link`.
        let c = COMMANDS
            .iter()
            .find(|c| c.name == "Follow link")
            .expect("Follow link must be in the catalog");
        assert_eq!(c.native, "");
        assert_eq!(c.emacs, "C-c C-o");
        assert_eq!(c.action, Action::FollowLink);
        assert_eq!(action_for_name("Follow link"), Some(Action::FollowLink));
        assert_eq!(action_for_name("follow_link"), Some(Action::FollowLink));
        // The default `C-c C-o` chord parses AND resolves to FollowLink through a
        // fresh MAC-convention keymap (the C-c prefix path) — the catalog/keymap
        // agreement sweep relies on this, pinned here explicitly too. Mac-pinned
        // deliberately: under `Convention::Linux`, bare Ctrl-C is displaced to
        // native Copy (`LINUX_DISPLACED_LETTERS` includes 'c'), so the `C-c`
        // prefix never arms there — that displacement is its own contract, see
        // `keymap.rs`'s collision table doc.
        assert!(crate::keymap::parse_binding("C-c C-o").is_ok());
        assert_eq!(resolve_chord_under("C-c C-o", Convention::Mac), Action::FollowLink);
    }

    #[test]
    fn report_problem_command_present_and_rebindable() {
        // "Report a Problem" is a real palette command (no default chord, like
        // Settings/About) backed by `Action::ReportProblem`, `native_only: false`
        // (available on both platforms), and independently rebindable via
        // `[keys] report_a_problem`.
        let c = COMMANDS
            .iter()
            .find(|c| c.name == "Report a Problem")
            .expect("Report a Problem must be in the catalog");
        assert_eq!(c.native, "");
        assert_eq!(c.emacs, "");
        assert_eq!(c.action, Action::ReportProblem);
        assert!(!c.native_only, "Report a Problem must be available on the web build too");
        assert_eq!(action_for_name("Report a Problem"), Some(Action::ReportProblem));
        assert_eq!(action_for_name("report_a_problem"), Some(Action::ReportProblem));
    }

    #[test]
    fn check_for_updates_command_present_rebindable_and_native_only() {
        // "Check for Updates" is a real palette command (no default chord, like
        // Report a Problem/Settings/About) backed by `Action::CheckForUpdates`,
        // `native_only: true` (the web build updates by deploy, so a "check"
        // command is meaningless there — it must NOT appear in the web view),
        // and independently rebindable via `[keys] check_for_updates`.
        let c = COMMANDS
            .iter()
            .find(|c| c.name == "Check for Updates")
            .expect("Check for Updates must be in the catalog");
        assert_eq!(c.native, "");
        assert_eq!(c.emacs, "");
        assert_eq!(c.action, Action::CheckForUpdates);
        assert!(c.native_only, "Check for Updates must be hidden on the web build");
        assert!(!c.available_on(Platform::Web));
        assert!(c.available_on(Platform::Native));
        assert_eq!(action_for_name("Check for Updates"), Some(Action::CheckForUpdates));
        assert_eq!(action_for_name("check_for_updates"), Some(Action::CheckForUpdates));
    }

    #[test]
    fn toggle_writing_nits_command_present_and_rebindable() {
        // The render-only toggle is in the catalog (palette-only, no default chord),
        // now backed by a REAL `Action::ToggleWritingNits` (the `Ignore` sentinel is
        // retired) so it round-trips through `RunAction` unambiguously.
        let c = COMMANDS
            .iter()
            .find(|c| c.name == "Toggle writing nits")
            .expect("the Toggle writing nits command must be in the catalog");
        assert_eq!(c.native, "");
        assert_eq!(c.emacs, "");
        assert_eq!(c.action, Action::ToggleWritingNits);
        // Summonable + rebindable by both the human label and the snake_case slug.
        assert_eq!(action_for_name("Toggle writing nits"), Some(Action::ToggleWritingNits));
        assert_eq!(action_for_name("toggle_writing_nits"), Some(Action::ToggleWritingNits));
    }

    #[test]
    fn clipboard_and_select_all_in_catalog_with_real_bindings() {
        // The keymap binds these already (native Cmd-C/X/V/A + emacs C-w/C-y); the
        // catalog lists them so they show in Cmd-P and become rebindable, carrying the
        // ACTUAL bindings. Copy's old emacs M-w is retired (the Option-letter layer
        // went quiet), so Copy is now Cmd-only; Cut (C-w) / Paste (C-y) keep their
        // bare-control survivors. Select all is Cmd-only (bare C-a stays LineStart).
        let find = |name: &str| COMMANDS.iter().find(|c| c.name == name).unwrap();
        let copy = find("Copy");
        assert_eq!(copy.action, Action::CopyRegion);
        assert_eq!((copy.native, copy.emacs), ("Cmd-C", ""));
        let cut = find("Cut");
        assert_eq!(cut.action, Action::KillRegion);
        assert_eq!((cut.native, cut.emacs), ("Cmd-X", "C-w"));
        let paste = find("Paste");
        assert_eq!(paste.action, Action::Yank);
        assert_eq!((paste.native, paste.emacs), ("Cmd-V", "C-y"));
        let all = find("Select all");
        assert_eq!(all.action, Action::SelectAll);
        assert_eq!((all.native, all.emacs), ("Cmd-A", ""));
        // Rebindable: each resolves by name + slug.
        assert_eq!(action_for_name("copy"), Some(Action::CopyRegion));
        assert_eq!(action_for_name("select_all"), Some(Action::SelectAll));
    }

    #[test]
    fn keybindings_command_present_and_rebindable() {
        // The rebind menu is itself a palette command + has a slug, so it can be
        // summoned by name AND rebound via `[keys] keybindings = "..."`.
        assert!(COMMANDS.iter().any(|c| c.action == Action::OpenKeybindings));
        assert_eq!(action_for_name("Keybindings"), Some(Action::OpenKeybindings));
        assert_eq!(action_for_name("keybindings"), Some(Action::OpenKeybindings));
    }

    #[test]
    fn version_history_command_present_and_rebindable() {
        // The version-history timeline is a palette command with a slug, so it can be
        // summoned by name AND rebound via `[keys] version_history = "..."`; its
        // default is Cmd-Shift-H. (Renamed from "History" so it no longer shadows the
        // "Local history" setting.)
        assert!(COMMANDS.iter().any(|c| c.action == Action::OpenHistory));
        assert_eq!(action_for_name("Version history…"), Some(Action::OpenHistory));
        assert_eq!(action_for_name("version_history"), Some(Action::OpenHistory));
        let cmd = COMMANDS.iter().find(|c| c.action == Action::OpenHistory).unwrap();
        assert_eq!(cmd.native, "Cmd-S-h");
    }

    #[test]
    fn keep_version_command_present_named_and_rebindable() {
        // THE CONSCIOUS MARK: "Keep version" is a palette-only command (no default
        // chord, like Settings/About) — summonable by name AND resolvable by its slug
        // for `[keys] keep_version = "..."`.
        assert!(COMMANDS.iter().any(|c| c.action == Action::KeepVersion));
        assert_eq!(action_for_name("Keep version"), Some(Action::KeepVersion));
        assert_eq!(action_for_name("keep_version"), Some(Action::KeepVersion));
        let cmd = COMMANDS.iter().find(|c| c.action == Action::KeepVersion).unwrap();
        assert_eq!(cmd.native, "", "palette-only — no default chord");
        assert_eq!(cmd.emacs, "");
    }

    #[test]
    fn binding_conflict_finds_canonical_clash() {
        // C-s is the default Search-forward chord, so binding it elsewhere clashes —
        // reported by the OTHER command's display name, canonically (Ctrl-s == C-s).
        assert_eq!(binding_conflict("C-s", "undo", &[]), Some("Search forward"));
        assert_eq!(binding_conflict("Ctrl-s", "undo", &[]), Some("Search forward"));
        // Excluding the owning command means rebinding it to its OWN chord is no clash.
        assert_eq!(binding_conflict("C-s", "search_forward", &[]), None);
        // A free chord conflicts with nothing.
        assert_eq!(binding_conflict("C-j", "undo", &[]), None);
        // A config override participates: bind "C-j" to save, then C-j clashes there.
        let keys = vec![("save".to_string(), vec!["C-j".to_string()])];
        assert_eq!(binding_conflict("C-j", "undo", &keys), Some("Save"));
        // An unparseable spec never conflicts.
        assert_eq!(binding_conflict("C-frobnicate", "undo", &[]), None);
    }

    #[test]
    fn markdown_formatting_commands_are_all_present_named_and_rebindable() {
        // All 11 formatting commands: name → action, each rebind-addressable by its
        // slug through `action_for_name` (so a `[keys]` entry finds it). The native
        // chords: Bold = Cmd-B, Italic = Cmd-I, Inline Code = Cmd-E (the universal
        // trio — Italic joined this round, the HUD having moved off plain Cmd-I),
        // Task list = Cmd-Shift-L (Apple Notes' checklist idiom); every other
        // formatting command is palette-only (empty native + emacs slot).
        let formatting: &[(&str, Action, &str)] = &[
            ("Blockquote", Action::ToggleBlockquote, ""),
            ("Bullet list", Action::ToggleBulletList, ""),
            ("Numbered list", Action::ToggleNumberedList, ""),
            ("Task list", Action::ToggleTaskList, "Cmd-S-l"),
            ("Heading", Action::ToggleHeading, ""),
            ("Code block", Action::ToggleCodeBlock, ""),
            ("Bold", Action::Bold, "Cmd-B"),
            ("Italic", Action::Italic, "Cmd-I"),
            ("Inline code", Action::InlineCode, "Cmd-E"),
            ("Highlight", Action::Highlight, ""),
            ("Strikethrough", Action::Strikethrough, ""),
        ];
        for (name, action, native) in formatting {
            let cmd = COMMANDS
                .iter()
                .find(|c| c.name == *name)
                .unwrap_or_else(|| panic!("formatting command {name:?} missing from catalog"));
            assert_eq!(&cmd.action, action, "{name}: catalog action");
            assert_eq!(cmd.native, *native, "{name}: native chord slot");
            assert_eq!(cmd.emacs, "", "{name}: emacs slot is left empty for the user");
            // Rebind-addressable by both the human label and its snake_case slug.
            assert_eq!(action_for_name(name), Some(action.clone()), "{name}: label rebind");
            assert_eq!(action_for_name(&slug(name)), Some(action.clone()), "{name}: slug rebind");
        }
        // Cmd-B / Cmd-I / Cmd-E / Cmd-Shift-L introduce no catalog conflict (the
        // pairwise sweep, `no_two_catalog_commands_share_a_default_chord`, proves
        // this exhaustively; these are the same spot-checks the pre-Italic version
        // of this test already made for Bold/Inline code).
        assert_eq!(binding_conflict("Cmd-B", "bold", &[]), None);
        assert_eq!(binding_conflict("Cmd-I", "italic", &[]), None);
        assert_eq!(binding_conflict("Cmd-E", "inline_code", &[]), None);
        assert_eq!(binding_conflict("Cmd-S-l", "task_list", &[]), None);
        // The effective (config-free) palette labels show all four as the
        // ambient convention's native glyphs — computed through the SAME
        // resolver `effective_binding_for` itself uses
        // (`resolved_native_label(c, Convention::current())`), so this holds on
        // EITHER convention rather than hardcoding the mac-only glyph form.
        let eff = effective_bindings(&[], &[]);
        let bold = COMMANDS.iter().position(|c| c.name == "Bold").unwrap();
        let ital = COMMANDS.iter().position(|c| c.name == "Italic").unwrap();
        let code = COMMANDS.iter().position(|c| c.name == "Inline code").unwrap();
        let task = COMMANDS.iter().position(|c| c.name == "Task list").unwrap();
        let convention = Convention::current();
        assert_eq!(eff[bold], resolved_native_label(&COMMANDS[bold], convention));
        assert_eq!(eff[ital], resolved_native_label(&COMMANDS[ital], convention));
        assert_eq!(eff[code], resolved_native_label(&COMMANDS[code], convention));
        assert_eq!(eff[task], resolved_native_label(&COMMANDS[task], convention));
    }

    #[test]
    fn links_v2_command_is_present_named_and_rebindable() {
        // LINKS V2 — the chord the keybinding-idiom audit reserved for exactly
        // this (W1). Cmd-K native, no emacs default (new command, not a
        // retirement), rebind-addressable by label AND slug.
        let cmd = COMMANDS
            .iter()
            .find(|c| c.name == "Insert link…")
            .expect("Insert link… missing from catalog");
        assert_eq!(cmd.action, Action::InsertLink);
        assert_eq!(cmd.native, "Cmd-K");
        assert_eq!(cmd.emacs, "");
        assert!(!cmd.native_only, "Insert link… is available on web too");
        assert_eq!(action_for_name("Insert link…"), Some(Action::InsertLink));
        assert_eq!(action_for_name(&slug("Insert link…")), Some(Action::InsertLink));
        // No conflict with any other command's default chord (the pairwise
        // sweep, `no_two_catalog_commands_share_a_default_chord`, proves this
        // exhaustively; spot-checked here too).
        assert_eq!(binding_conflict("Cmd-K", "insert_link", &[]), None);
    }

    /// Resolve a chord SPEC ("Cmd-S", "C-x C-s", "C-x }") through a FRESH
    /// [`crate::keymap::KeymapState`] pinned to `convention`, token by token,
    /// returning the LAST resolved action — the `C-x` token resolves to
    /// `BeginPrefix` and arms the prefix state, exactly as the live keypresses
    /// would.
    fn resolve_chord_under(spec: &str, convention: Convention) -> Action {
        let mut km = crate::keymap::KeymapState::new_with_convention(convention);
        let mut last = Action::Ignore;
        for tok in spec.split_whitespace() {
            let (key, mods) = crate::keyspec::parse_chord(tok)
                .unwrap_or_else(|e| panic!("catalog chord {spec:?} failed to parse: {e}"));
            last = km.resolve(&key, &mods);
        }
        last
    }

    #[test]
    fn catalog_and_keymap_agree_on_every_default_chord() {
        // THE AGREEMENT SWEEP: the catalog's binding labels are DATA (the palette
        // teaches them; the rebind menu edits them) while the keymap's dispatch
        // arms are hand-written — this loop pins the two together for EVERY
        // command, so a chord shown in Cmd-P always fires exactly that command
        // and a `[keys]` entry always finds its action.
        //
        // CONVENTION-PROOF (per-convention, not just whichever is ambient):
        // `c.native` is always stored in MAC-LITERAL form ("Cmd-O") — under
        // `Convention::Linux` the chord that ACTUALLY fires is the one
        // `commands::resolved_native` computes (a translated/overridden Ctrl
        // chord, per `LINUX_NATIVE_OVERRIDE`/`translate_native_for_linux`), so the
        // native half is checked by resolving THAT translated chord under each
        // convention in turn — never the literal mac string against a Linux
        // keymap (which would never fire native_down at all, see
        // `KeymapState::native_down`'s Super-vs-Ctrl split). The emacs half is
        // OS-agnostic text ("C-s") and is checked directly under BOTH
        // conventions, EXCEPT where `keymap::linux_displaces_emacs_default` says
        // Linux's native layer displaces it (`LINUX_DISPLACED_LETTERS`) — that
        // displacement is its own exhaustively law-tested contract
        // (`keymap::tests::linux_collision_table_matches_the_documented_displaced_list`),
        // not something this sweep should re-assert. SYMMETRICALLY, the NATIVE
        // half skips a chord the DEFAULT (config-free) Linux keep-list holds
        // back (`keymap::linux_builtin_keep()` — Insert link's Ctrl-K, which
        // yields to kill-line out of the box; the insert-link-yields round) —
        // that non-firing is ITS own law-tested contract too
        // (`keymap::tests::out_of_the_box_linux_ctrl_k_is_kill_line_under_both_keymap_flavors`),
        // and the labels never advertise the chord there either
        // (`insert_link_has_no_visible_linux_binding_out_of_the_box_mac_shows_cmd_k`).
        let default_linux_keep = crate::config::Config::empty().effective_linux_keep();
        for c in COMMANDS.iter() {
            for convention in [Convention::Mac, Convention::Linux] {
                if !c.native.trim().is_empty() {
                    let resolved = resolved_native(c, convention);
                    let kept_back = convention == Convention::Linux
                        && crate::keymap::linux_keeps_chord(&default_linux_keep, &resolved);
                    if !resolved.trim().is_empty() && !kept_back {
                        assert!(
                            crate::keymap::parse_binding(&resolved).is_ok(),
                            "{}: {:?}'s resolved native chord {resolved:?} must parse via parse_binding",
                            c.name,
                            convention
                        );
                        assert_eq!(
                            resolve_chord_under(&resolved, convention),
                            c.action,
                            "{}: {:?}'s resolved native chord {resolved:?} must resolve to the catalog action",
                            c.name,
                            convention
                        );
                    }
                }
                if !c.emacs.trim().is_empty() {
                    assert!(
                        crate::keymap::parse_binding(c.emacs).is_ok(),
                        "{}: emacs default {:?} must parse via parse_binding",
                        c.name,
                        c.emacs
                    );
                    if convention == Convention::Linux && crate::keymap::linux_displaces_emacs_default(c.emacs, &[]) {
                        continue; // displaced by native on Linux — covered by keymap.rs's own law test.
                    }
                    assert_eq!(
                        resolve_chord_under(c.emacs, convention),
                        c.action,
                        "{}: {:?}'s emacs default {:?} must resolve to the catalog action",
                        c.name,
                        convention,
                        c.emacs
                    );
                }
            }
            // The config ACTION NAME round-trips: slug(name) → action_for_name →
            // this command's action (every catalog row is rebind-addressable) —
            // convention-independent.
            assert_eq!(
                action_for_name(&slug(c.name)),
                Some(c.action.clone()),
                "{}: slug round-trip through action_for_name",
                c.name
            );
        }
    }

    #[test]
    fn no_two_catalog_commands_share_a_default_chord() {
        // PAIRWISE default-chord conflicts, compared CANONICALLY through the same
        // `binding_conflict` the rebind menu gates on (so `Cmd-S` == `s-s`
        // spellings clash too). An INTENTIONALLY shared chord would be allow-
        // listed here as a (command, command) pair with a comment explaining the
        // share — today there are NONE, so the list is empty and every default
        // chord belongs to exactly one command.
        const INTENTIONALLY_SHARED: &[(&str, &str)] = &[];
        for c in COMMANDS.iter() {
            for chord in [c.native, c.emacs] {
                if chord.trim().is_empty() {
                    continue;
                }
                if let Some(other) = binding_conflict(chord, &slug(c.name), &[]) {
                    let allowlisted = INTENTIONALLY_SHARED.iter().any(|(a, b)| {
                        (*a == c.name && *b == other) || (*a == other && *b == c.name)
                    });
                    assert!(
                        allowlisted,
                        "default chord {chord:?} is bound to BOTH {:?} and {other:?} \
                         (not in the intentional-share allowlist)",
                        c.name
                    );
                }
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn slug_for_action_and_has_native_chord_key_the_usage_ledger() {
        // A catalog command resolves to its slug; the SAME identity `record_recent`
        // uses, so the ledger and the Recent MRU agree on "a command".
        assert_eq!(slug_for_action(&Action::OpenGoto).as_deref(), Some("go_to_file"));
        assert_eq!(slug_for_action(&Action::OpenThemeMenu).as_deref(), Some("switch_theme"));
        // A self-insert / prefix carries no catalog command → None (no alloc). Every
        // MOTION now has one (the emacs-hands-on-Linux round completed the catalog),
        // so `ForwardChar` — the former example here — no longer belongs in this list.
        assert_eq!(slug_for_action(&Action::ForwardChar), Some("forward_char".to_string()));
        assert_eq!(slug_for_action(&Action::InsertChar('x')), None);
        assert_eq!(slug_for_action(&Action::BeginPrefix), None);
        // has_native_chord: true for a native-slot command, false for palette-only.
        assert!(has_native_chord("go_to_file"), "Go to file… carries Cmd-O");
        assert!(has_native_chord("save"), "Save carries Cmd-S");
        assert!(has_native_chord("settings"), "Settings… now carries Cmd-, (P1)");
        assert!(!has_native_chord("browse_files"), "Browse files… is palette-only");
        assert!(!has_native_chord("about"), "About is palette-only");
        assert!(!has_native_chord("reset_page_width"), "Reset page width is palette-only");
        assert!(!has_native_chord("no_such_command"), "unknown slug: false");
        // The two agree: every slug `slug_for_action` yields is a real catalog slug.
        assert!(has_native_chord(&slug_for_action(&Action::Save).unwrap()));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn peek_row_resolves_native_chord_and_name_or_none_for_palette_only() {
        // A native-chord command → its glyph chord (per the active convention,
        // via the SAME resolver `peek_row_for_slug` itself calls) + ellipsis-
        // stripped name.
        assert_eq!(
            peek_row_for_slug("go_to_file"),
            Some(crate::peek::PeekRow { chord: label_for("Go to file…"), name: "Go to file".into() })
        );
        assert_eq!(
            peek_row_for_slug("switch_theme"),
            Some(crate::peek::PeekRow { chord: label_for("Switch theme…"), name: "Switch theme".into() })
        );
        // A palette-only command (no native chord to teach) → None, so it never
        // surfaces as a peek/footer row even if slow-door usage ranks it.
        assert_eq!(peek_row_for_slug("about"), None);
        // Settings now carries Cmd-, (P1), so it DOES resolve a peek row.
        assert_eq!(
            peek_row_for_slug("settings"),
            Some(crate::peek::PeekRow { chord: label_for("Settings…"), name: "Settings".into() })
        );
        // An unknown slug → None (defensive).
        assert_eq!(peek_row_for_slug("no_such_command"), None);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn catalog_motions_are_exactly_the_curated_navigation_set() {
        // THE MOTION SPLIT (user-decided 2026-07-10, superseding the original
        // all-motions exclusion; WIDENED by the emacs-hands-on-Linux round to the
        // last four bare-control nav motions — char forward/back, line up/down —
        // so `[keys]` can finally rebind C-f/C-b/C-n/C-p at all). Every motion
        // `Action::is_motion` names is now a catalog row (palette-visible +
        // rebindable); the split that remains is self-insertion, which never
        // enters the catalog. Kept as a NO-WILDCARD-style completeness sweep
        // (rather than deleting it now that the split is "all of them") so a
        // FUTURE motion added to `is_motion` without a matching catalog row still
        // fails this test loudly, exactly like before.
        const NAVIGATION_MOTIONS: &[Action] = &[
            Action::ForwardChar,
            Action::BackwardChar,
            Action::NextLine,
            Action::PreviousLine,
            Action::ForwardWord,
            Action::BackwardWord,
            Action::LineStart,
            Action::LineEnd,
            Action::BufferStart,
            Action::BufferEnd,
        ];
        for c in COMMANDS.iter() {
            if c.action.is_motion() {
                assert!(
                    NAVIGATION_MOTIONS.contains(&c.action),
                    "{}: a motion outside the curated navigation set entered the catalog",
                    c.name
                );
            }
            assert!(
                !matches!(c.action, Action::InsertChar(_)),
                "{} self-inserts; excluded",
                c.name
            );
        }
        // Every curated motion IS in the catalog (the split is exact, both ways) …
        for m in NAVIGATION_MOTIONS {
            assert!(
                COMMANDS.iter().any(|c| &c.action == m),
                "curated navigation motion {m:?} missing from the catalog"
            );
        }
        // … and every `is_motion` action IS one of the curated ones — the set is
        // now EXACTLY `is_motion`'s own set, no residual keymap-only motion left.
        for m in NAVIGATION_MOTIONS {
            assert!(m.is_motion(), "{m:?} listed as a navigation motion but is_motion() is false");
        }
    }

    #[test]
    fn motion_commands_are_all_present_named_and_rebindable() {
        // The six navigation motions: name → action → REAL default chords, each
        // rebind-addressable by its slug through `action_for_name` (so a `[keys]`
        // entry finds it and the Keybindings menu can capture onto it). The emacs
        // slots emptied by the Option-letter retirement stay empty (the user's to
        // fill); Line start/end keep their surviving bare-control second slots.
        let motions: &[(&str, Action, &str, &str)] = &[
            ("Forward word", Action::ForwardWord, "M-Right", ""),
            ("Backward word", Action::BackwardWord, "M-Left", ""),
            ("Line start", Action::LineStart, "Cmd-Left", "C-a"),
            ("Line end", Action::LineEnd, "Cmd-Right", "C-e"),
            ("Document start", Action::BufferStart, "Cmd-Up", ""),
            ("Document end", Action::BufferEnd, "Cmd-Down", ""),
        ];
        for (name, action, native, emacs) in motions {
            let cmd = COMMANDS
                .iter()
                .find(|c| c.name == *name)
                .unwrap_or_else(|| panic!("motion command {name:?} missing from catalog"));
            assert_eq!(&cmd.action, action, "{name}: catalog action");
            assert_eq!(cmd.native, *native, "{name}: native chord slot");
            assert_eq!(cmd.emacs, *emacs, "{name}: emacs chord slot");
            // Rebind-addressable by both the human label and its snake_case slug.
            assert_eq!(action_for_name(name), Some(action.clone()), "{name}: label rebind");
            assert_eq!(action_for_name(&slug(name)), Some(action.clone()), "{name}: slug rebind");
        }
        // THE CONCRETE ASK this round serves: the retired Option-letter word motion
        // is one `[keys]` line away — `forward_word = "M-f"` / `backward_word =
        // "M-b"` parse through the rebinder's grammar and CONFLICT with nothing
        // (the retirement freed those chords; Option-letters type characters only
        // until a config line deliberately reclaims them).
        for spec in ["M-f", "M-b"] {
            assert!(crate::keymap::parse_binding(spec).is_ok(), "{spec:?} must parse");
        }
        assert_eq!(binding_conflict("M-f", "forward_word", &[]), None);
        assert_eq!(binding_conflict("M-b", "backward_word", &[]), None);
        // And the override surfaces in the palette's binding column (slot 1
        // glyphified per the active convention: M-f → ⌥F on Mac, "Alt+F" on
        // Linux), teaching the chord the user chose.
        let keys = vec![("forward_word".to_string(), vec!["M-f".to_string()])];
        let i = COMMANDS.iter().position(|c| c.name == "Forward word").unwrap();
        assert_eq!(effective_bindings(&keys, &[])[i], glyph("M-f"));
    }

    // ── PLATFORM-SCOPED COMMANDS ────────────────────────────────────────────────
    // All run on the native test binary; `Platform::Web` is asserted directly via
    // `available_on`/`visible_on`/`roster_for`-style explicit-platform doors — no
    // `cfg!` gymnastics, no actual wasm build needed to pin the web-hidden view.

    /// THE HIDE LIST (settled): every one of these — and ONLY these — is
    /// `native_only`, hence unavailable on `Web` and available on `Native`. A future
    /// command added to this list without flipping `native_only`, or a `native_only`
    /// command not in this list, fails here.
    const HIDE_ON_WEB: &[&str] = &[
        "Quit",
        "Finish file",
        "Version history…",
        "Keep version",
        "Lifetime stats",
        "Clean unused assets…",
        "Recent projects…",
        "Check for Updates",
    ];

    /// THE INVERSE HIDE LIST (the WEB ESCAPE HATCHES round): every one of these —
    /// and ONLY these — is `web_only`, hence unavailable on `Native` and available
    /// on `Web`. Mirrors [`HIDE_ON_WEB`] in the other direction.
    const HIDE_ON_NATIVE: &[&str] = &["Download file"];

    #[test]
    fn hide_list_is_exactly_the_native_only_commands() {
        let flagged: std::collections::HashSet<&str> =
            COMMANDS.iter().filter(|c| c.native_only).map(|c| c.name).collect();
        let listed: std::collections::HashSet<&str> = HIDE_ON_WEB.iter().copied().collect();
        assert_eq!(flagged, listed, "native_only flags and the hide list must match exactly");
    }

    #[test]
    fn inverse_hide_list_is_exactly_the_web_only_commands() {
        let flagged: std::collections::HashSet<&str> =
            COMMANDS.iter().filter(|c| c.web_only).map(|c| c.name).collect();
        let listed: std::collections::HashSet<&str> = HIDE_ON_NATIVE.iter().copied().collect();
        assert_eq!(flagged, listed, "web_only flags and the inverse hide list must match exactly");
    }

    #[test]
    fn no_command_is_flagged_unavailable_on_both_platforms() {
        for c in COMMANDS.iter() {
            assert!(
                !(c.native_only && c.web_only),
                "{}: native_only and web_only can never both be true (available nowhere)",
                c.name
            );
        }
    }

    #[test]
    fn web_only_commands_are_unavailable_on_native_available_on_web() {
        for name in HIDE_ON_NATIVE {
            let c = COMMANDS.iter().find(|c| &c.name == name).unwrap_or_else(|| panic!("{name}: missing"));
            assert!(!c.available_on(Platform::Native), "{name}: must be hidden on native");
            assert!(c.available_on(Platform::Web), "{name}: must stay available on web");
        }
    }

    #[test]
    fn hide_listed_commands_are_unavailable_on_web_available_on_native() {
        for name in HIDE_ON_WEB {
            let c = COMMANDS.iter().find(|c| &c.name == name).unwrap_or_else(|| panic!("{name}: missing"));
            assert!(!c.available_on(Platform::Web), "{name}: must be hidden on web");
            assert!(c.available_on(Platform::Native), "{name}: must stay available natively");
        }
    }

    #[test]
    fn every_other_command_is_available_on_both_platforms() {
        for c in COMMANDS.iter() {
            if HIDE_ON_WEB.contains(&c.name) || HIDE_ON_NATIVE.contains(&c.name) {
                continue;
            }
            assert!(c.available_on(Platform::Web), "{}: unexpectedly hidden on web", c.name);
            assert!(c.available_on(Platform::Native), "{}: unexpectedly hidden on native", c.name);
        }
    }

    #[test]
    fn platform_current_is_native_under_a_native_test_binary() {
        // `cargo test` is never a wasm32 target, so `Platform::current()` reads
        // Native here — the compiled-platform door and the explicit-platform door
        // agree on THIS binary by construction.
        assert_eq!(Platform::current(), Platform::Native);
    }

    #[test]
    fn visible_on_native_drops_exactly_the_inverse_hide_list_and_nothing_else() {
        let native = visible_on(Platform::Native);
        assert_eq!(native.len(), COMMANDS.len() - HIDE_ON_NATIVE.len());
        // Order is otherwise preserved exactly (filtering, never reordering).
        let expected: Vec<&str> =
            COMMANDS.iter().map(|c| c.name).filter(|n| !HIDE_ON_NATIVE.contains(n)).collect();
        let actual: Vec<&str> = native.iter().map(|c| c.name).collect();
        assert_eq!(actual, expected, "native visible() must preserve catalog order exactly");
        for name in HIDE_ON_NATIVE {
            assert!(!native.iter().any(|c| &c.name == name), "{name}: leaked into the native view");
        }
        // The compiled-platform door matches the explicit-platform door on native.
        assert_eq!(visible().len(), visible_on(Platform::Native).len());
    }

    #[test]
    fn visible_on_web_drops_exactly_the_hide_list_and_nothing_else() {
        let web = visible_on(Platform::Web);
        assert_eq!(web.len(), COMMANDS.len() - HIDE_ON_WEB.len());
        for c in &web {
            assert!(!HIDE_ON_WEB.contains(&c.name), "{}: should have been hidden on web", c.name);
        }
        for name in HIDE_ON_WEB {
            assert!(!web.iter().any(|c| &c.name == name), "{name}: leaked into the web view");
        }
        // The web-only escape hatch IS present on web.
        for name in HIDE_ON_NATIVE {
            assert!(web.iter().any(|c| &c.name == name), "{name}: missing from the web view");
        }
    }

    /// INDEX-COHERENCE LAW for the filtered palette/rebind-menu corpus: for every
    /// row `i` in `visible()`, `visible_action_of(i)` / `visible_slug_of(i)` /
    /// `visible_name_of(i)` all name THAT SAME row's command — never a raw
    /// `COMMANDS[i]` (which would silently mis-map once rows are hidden). Checked on
    /// both platforms explicitly (`visible_on`), not just the native-compiled
    /// `visible()`, so the web-filtered corpus's own index coherence is pinned too.
    #[test]
    fn visible_corpus_index_coherence_holds_on_both_platforms() {
        for platform in [Platform::Native, Platform::Web] {
            let filtered = visible_on(platform);
            let names: Vec<String> = filtered.iter().map(|c| c.name.to_string()).collect();
            let actions: Vec<Action> = filtered.iter().map(|c| c.action.clone()).collect();
            // `visible()`/`visible_action_of`/etc. are the CURRENT-platform door; on a
            // native test binary they equal the `Platform::Native` explicit view, so
            // only assert the index-translation SHAPE here (name/action pairing),
            // reusable for either platform's filtered Vec directly.
            for (i, (name, action)) in names.iter().zip(actions.iter()).enumerate() {
                let c = filtered[i];
                assert_eq!(&c.name.to_string(), name, "row {i}: name must match its own filtered slot");
                assert_eq!(&c.action, action, "row {i}: action must match its own filtered slot");
            }
        }
        // And concretely, on THIS platform (native): visible_action_of/visible_slug_of/
        // visible_name_of agree with visible() row-for-row.
        let corpus = visible();
        for i in 0..corpus.len() {
            assert_eq!(visible_action_of(i), corpus[i].action, "row {i}: visible_action_of drift");
            assert_eq!(visible_slug_of(i), slug(corpus[i].name), "row {i}: visible_slug_of drift");
            assert_eq!(visible_name_of(i), corpus[i].name, "row {i}: visible_name_of drift");
        }
    }

    #[test]
    fn visible_names_and_bindings_are_parallel_and_match_visible() {
        let corpus = visible();
        let names = visible_names();
        let binds = visible_effective_bindings(&[], &[]);
        assert_eq!(names.len(), corpus.len());
        assert_eq!(binds.len(), corpus.len());
        for (i, c) in corpus.iter().enumerate() {
            assert_eq!(names[i], c.name);
        }
    }

    #[test]
    fn action_available_gates_hidden_actions_only_on_web() {
        // A hidden command's Action: unavailable on Web, available on Native.
        assert!(!action_available(&Action::Quit, Platform::Web));
        assert!(action_available(&Action::Quit, Platform::Native));
        assert!(!action_available(&Action::FinishBuffer, Platform::Web));
        // A non-hidden catalog action: available on both. Keybindings… lost its
        // web-hide flag once a web `config.toml` existed to rebind INTO (the web
        // config round) — the rebind menu is reachable + writes persist there now.
        assert!(action_available(&Action::OpenKeybindings, Platform::Web));
        assert!(action_available(&Action::Save, Platform::Web));
        assert!(action_available(&Action::Save, Platform::Native));
        // A non-catalog action (motion / self-insert) always fires — nothing to hide.
        assert!(action_available(&Action::ForwardChar, Platform::Web));
        assert!(action_available(&Action::InsertChar('x'), Platform::Web));
    }

    #[test]
    fn visible_recent_indices_drops_hidden_catalog_entries_and_translates_the_rest() {
        clear_recent();
        record_recent(&Action::Undo);
        record_recent(&Action::Quit); // a hidden-on-web command
        record_recent(&Action::Redo);
        // On native (this test binary), nothing is hidden, so all three translate.
        let vis = visible_recent_indices();
        assert_eq!(vis.len(), 3);
        let corpus = visible();
        let redo_row = corpus.iter().position(|c| c.action == Action::Redo).unwrap();
        assert_eq!(vis[0], redo_row, "most-recent-first order preserved");
        clear_recent();
    }

    // ── THE WEB CHORD SANITY ROUND ──────────────────────────────────────────────

    /// THE HARD LAW: on `Convention::Mac` + `Platform::Native` (a plain macOS
    /// native build) neither Tier 2 (web-reserved) nor Tier 3 (Linux-displaced)
    /// can ever fire, so [`join_slots_truthful`] must be BYTE-IDENTICAL to the
    /// pre-round `join_slots(c.native, c.emacs)` for EVERY catalog command.
    #[test]
    fn mac_native_label_truth_is_byte_identical_to_join_slots() {
        for c in COMMANDS.iter() {
            assert_eq!(
                join_slots_truthful(c, Convention::Mac, Platform::Native, &[]),
                join_slots(c.native, c.emacs),
                "{} diverged from the pre-round Mac-native label",
                c.name
            );
        }
    }

    /// TIER 2, v2 (the convention-truthful-surfaces round): "New note" (Cmd-N)
    /// and "Switch theme…" (Cmd-T) are exactly the two catalog commands this
    /// round's own bug report names as browser-shadowed — on `Platform::Web`
    /// their native chord label is no longer blank (v1's documented "no
    /// replacement chord" answer); it shows the command's [`WEB_ALTERNATE`]
    /// chord instead, and dispatches through it too (see
    /// `keymap::tests::web_alternate_keys_dispatch_the_real_action_on_web`).
    #[test]
    fn web_reserved_native_chord_shows_its_web_alternate() {
        let new_note = COMMANDS.iter().find(|c| c.name == "New note").unwrap();
        let switch_theme = COMMANDS.iter().find(|c| c.name == "Switch theme…").unwrap();
        for c in [new_note, switch_theme] {
            assert_eq!(c.emacs.trim(), "", "{} must have no emacs slot for this test's claim", c.name);
            for convention in [Convention::Mac, Convention::Linux] {
                let label = resolved_native_label_truthful(c, convention, Platform::Web);
                assert!(!label.is_empty(), "{}: web alternate must not be blank ({convention:?})", c.name);
                assert_ne!(
                    label,
                    resolved_native_label(c, convention),
                    "{}: the web label must be the ALTERNATE, not the (reserved) native one",
                    c.name
                );
                assert_eq!(join_slots_truthful(c, convention, Platform::Web, &[]), label);
                // Native BUILD (Platform::Native): unaffected, the ORIGINAL native chord shows.
                assert_eq!(resolved_native_label_truthful(c, convention, Platform::Native), resolved_native_label(c, convention));
            }
        }
    }

    /// The exact web-alternate LABELS, pinned: Mac web gets bare Ctrl-letters
    /// (Ctrl being free of both the browser's own Cmd-based reservations and
    /// the static bare-control emacs arms for these two letters), Linux web
    /// gets bare Alt-letters (Ctrl is unavailable there — it's the reserved
    /// chord itself; the Meta-letter layer was fully retired, so no default
    /// arm claims these).
    #[test]
    fn web_alternate_labels_are_convention_keyed() {
        let new_note = COMMANDS.iter().find(|c| c.name == "New note").unwrap();
        let switch_theme = COMMANDS.iter().find(|c| c.name == "Switch theme…").unwrap();
        assert_eq!(resolved_native_label_truthful(new_note, Convention::Mac, Platform::Web), "\u{2303}J");
        assert_eq!(resolved_native_label_truthful(switch_theme, Convention::Mac, Platform::Web), "\u{2303}T");
        assert_eq!(resolved_native_label_truthful(new_note, Convention::Linux, Platform::Web), "Alt+N");
        assert_eq!(resolved_native_label_truthful(switch_theme, Convention::Linux, Platform::Web), "Alt+T");
    }

    /// Exhaustive availability check backing this round's own doc comment
    /// (`WEB_ALTERNATE`'s module note): "New note" and "Switch theme…" are
    /// EXACTLY the catalog commands that are (a) available on `Platform::Web`
    /// and (b) carry a browser-reserved native chord on EITHER convention —
    /// no third command silently needs an alternate too.
    #[test]
    fn exactly_new_note_and_switch_theme_are_web_reserved_and_available() {
        let mut hit: Vec<&str> = COMMANDS
            .iter()
            .filter(|c| c.available_on(Platform::Web))
            .filter(|c| {
                [Convention::Mac, Convention::Linux]
                    .iter()
                    .any(|conv| crate::webreserved::is_reserved(&resolved_native(c, *conv), *conv))
            })
            .map(|c| c.name)
            .collect();
        hit.sort_unstable();
        assert_eq!(hit, vec!["New note", "Switch theme…"]);
    }

    /// [`web_alternate_keys`] is a no-op on [`Platform::Native`] (config
    /// construction stays byte-for-byte unaffected), and on [`Platform::Web`]
    /// yields exactly the two web-alternate chords, config-slugged, ready for
    /// `KeymapState::apply_overrides`.
    #[test]
    fn web_alternate_keys_is_inert_on_native_and_populated_on_web() {
        assert_eq!(web_alternate_keys(&[], Convention::Mac, Platform::Native), Vec::new());
        let mut on_web = web_alternate_keys(&[], Convention::Mac, Platform::Web);
        on_web.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            on_web,
            vec![("new_note".to_string(), vec!["C-j".to_string()]), ("switch_theme".to_string(), vec!["C-t".to_string()])]
        );
        let mut on_web_linux = web_alternate_keys(&[], Convention::Linux, Platform::Web);
        on_web_linux.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            on_web_linux,
            vec![("new_note".to_string(), vec!["M-n".to_string()]), ("switch_theme".to_string(), vec!["M-t".to_string()])]
        );
    }

    /// **Config still trumps everything:** a user `[keys]` entry for "New
    /// note" suppresses ITS web alternate entirely (the user's own chosen
    /// chord is never shadowed), while "Switch theme…"'s alternate — untouched
    /// by the user's config — still appears.
    #[test]
    fn web_alternate_keys_skips_a_command_the_user_has_already_rebound() {
        let existing = vec![("new_note".to_string(), vec!["C-x C-n".to_string()])];
        let on_web = web_alternate_keys(&existing, Convention::Mac, Platform::Web);
        assert!(!on_web.iter().any(|(name, _)| name == "new_note"), "user's own new_note rebind must not be shadowed");
        assert!(on_web.iter().any(|(name, _)| name == "switch_theme"), "switch_theme's alternate is still added");
    }

    /// THE DISPATCH HALF: `web_alternate_keys`'s output, fed through the REAL
    /// keymap exactly the way `App::new` wires it, actually resolves the
    /// alternate chord to the command's own `Action` — not just a label that
    /// LOOKS right.
    #[test]
    fn web_alternate_keys_dispatch_the_real_action_on_web() {
        let keys = web_alternate_keys(&[], Convention::Mac, Platform::Web);
        let mut km = crate::keymap::KeymapState::with_overrides(&keys);
        let (key, mods) = crate::keyspec::parse_chord("C-j").expect("C-j parses");
        assert_eq!(km.resolve(&key, &mods), Action::NewNote);
        let (key, mods) = crate::keyspec::parse_chord("C-t").expect("C-t parses");
        assert_eq!(km.resolve(&key, &mods), Action::OpenThemeMenu);
    }

    /// TIER 2, the fallback half: a SYNTHETIC command whose native chord is
    /// web-reserved but which ALSO carries a surviving emacs slot falls back
    /// to that slot on the web — never a blank label when a truthful door
    /// remains.
    #[test]
    fn web_reserved_native_chord_falls_back_to_a_surviving_emacs_slot() {
        let synthetic =
            Command { name: "Synthetic", action: Action::Ignore, native: "Cmd-N", emacs: "C-k", native_only: false, web_only: false };
        // 'k' is NOT in the Linux displaced-letters set (kill-line's own
        // Ctrl-K keeps its emacs meaning unconditionally, via
        // `keymap::linux_builtin_keep()` — see the insert-link-yields-to-
        // kill-line round), so it survives there too.
        assert_eq!(join_slots_truthful(&synthetic, Convention::Mac, Platform::Web, &[]), "C-k");
        assert_eq!(join_slots_truthful(&synthetic, Convention::Linux, Platform::Web, &[]), "C-k");
        // Off the web, the native chord is truthful again and joins normally.
        assert_eq!(join_slots_truthful(&synthetic, Convention::Mac, Platform::Native, &[]), "⌘N · C-k");
    }

    /// TIER 2 on the LINUX convention: "New note"'s Ctrl-translated form
    /// (`Ctrl-N`) is reserved on a Linux-flavored browser too (a NEW tab/
    /// window is universally browser-owned), independent of the Mac table.
    #[test]
    fn linux_web_reserved_uses_the_ctrl_translated_form() {
        let new_note = COMMANDS.iter().find(|c| c.name == "New note").unwrap();
        assert_eq!(resolved_native(new_note, Convention::Linux), "C-n");
        assert!(crate::webreserved::is_reserved("C-n", Convention::Linux));
        // v2: no longer blank — the Linux web alternate (Alt-N) takes over slot 1.
        assert_eq!(resolved_native_label_truthful(new_note, Convention::Linux, Platform::Web), "Alt+N");
    }

    /// TIER 3: "Search forward" (native Cmd-F, emacs `C-s`) under
    /// `Convention::Linux` — Ctrl-S is claimed by Save, so the emacs slot is
    /// displaced and must NOT appear in the joined label, on EITHER platform
    /// (the collision is a dispatch-table property, not a web-only one).
    #[test]
    fn linux_displaced_emacs_default_never_shown_on_either_platform() {
        let search = COMMANDS.iter().find(|c| c.name == "Search forward").unwrap();
        for platform in [Platform::Native, Platform::Web] {
            let label = join_slots_truthful(search, Convention::Linux, platform, &[]);
            assert_eq!(label, "Ctrl+F", "displaced C-s must not appear (platform {platform:?})");
        }
        // Mac convention: the emacs slot is UNCHANGED (Ctrl never reads native
        // there), so the old joined form survives on both platforms.
        assert_eq!(join_slots_truthful(search, Convention::Mac, Platform::Native, &[]), "⌘F · C-s");
    }

    /// TIER 3, the prefix-sequence edge case: "Follow link"'s emacs default is
    /// the two-key `"C-c C-o"` sequence — Ctrl-C now resolves straight to Copy
    /// on Linux, so the WHOLE sequence is displaced (never arms), and Follow
    /// link has no native slot either — the joined label goes fully blank.
    #[test]
    fn linux_displaces_a_prefix_sequence_by_its_first_key() {
        let follow = COMMANDS.iter().find(|c| c.name == "Follow link").unwrap();
        assert_eq!(follow.native.trim(), "");
        assert_eq!(follow.emacs, "C-c C-o");
        assert_eq!(join_slots_truthful(follow, Convention::Linux, Platform::Native, &[]), "");
        // Mac: unaffected, the sequence still shows.
        assert_eq!(join_slots_truthful(follow, Convention::Mac, Platform::Native, &[]), "C-c C-o");
    }

    /// TIER 3, the non-displaced control: "Undo"'s emacs slot `C-/` is a
    /// non-letter chord outside the displaced-letter set entirely, so it
    /// survives Linux exactly like Mac.
    #[test]
    fn non_displaced_emacs_default_survives_linux() {
        let undo = COMMANDS.iter().find(|c| c.name == "Undo").unwrap();
        assert_eq!(join_slots_truthful(undo, Convention::Linux, Platform::Native, &[]), "Ctrl+Z · C-/");
    }

    /// THE LABEL-TRUTH LAW, swept over the WHOLE catalog × every (convention,
    /// platform) pair: [`resolved_native_label_truthful`] is empty whenever
    /// [`crate::webreserved::is_reserved`] says so, and the joined label never
    /// contains a Linux-displaced emacs default as one of its `·`-separated
    /// tokens. A future catalog command that starts colliding fails THIS test
    /// until it is accounted for — the no-wildcard sweep the round's laws ask for.
    #[test]
    fn label_truth_law_holds_across_the_whole_catalog() {
        for c in COMMANDS.iter() {
            for convention in [Convention::Mac, Convention::Linux] {
                for platform in [Platform::Native, Platform::Web] {
                    let native_resolved = resolved_native(c, convention);
                    let reserved = platform == Platform::Web && crate::webreserved::is_reserved(&native_resolved, convention);
                    if reserved {
                        let label = resolved_native_label_truthful(c, convention, platform);
                        let native_label = resolved_native_label(c, convention);
                        assert_ne!(
                            label, native_label,
                            "{}: reserved native chord {native_resolved:?} still shown verbatim ({convention:?}/{platform:?})",
                            c.name
                        );
                        // Either a web alternate (non-blank) or blank (no alternate defined) — but
                        // never the reserved native chord itself.
                        if let Some(alt) = web_alternate_for(c, convention) {
                            let expect = match convention {
                                Convention::Mac => crate::keyspec::mac_glyph_chord(alt),
                                Convention::Linux => crate::keyspec::linux_glyph_chord(alt),
                            };
                            assert_eq!(label, expect, "{}: web alternate label mismatch ({convention:?}/{platform:?})", c.name);
                        } else {
                            assert_eq!(label, "", "{}: no alternate defined, label should be blank ({convention:?}/{platform:?})", c.name);
                        }
                    }
                    let displaced = convention == Convention::Linux && crate::keymap::linux_displaces_emacs_default(c.emacs, &[]);
                    if displaced {
                        let label = join_slots_truthful(c, convention, platform, &[]);
                        assert!(
                            !label.split(" · ").any(|tok| tok == c.emacs),
                            "{}: displaced emacs default {:?} still shown ({convention:?}/{platform:?}) — label was {label:?}",
                            c.name,
                            c.emacs
                        );
                    }
                }
            }
        }
    }

    /// TIER 4 (emacs-hands-on-Linux): "Forward char" (no native slot, emacs
    /// `C-f`) is normally Linux-DISPLACED by "Search forward"'s native Ctrl-F.
    /// A `linux_keep_emacs = ["C-f"]` config UN-displaces it (its emacs label
    /// reappears) AND suppresses "Search forward"'s own native label for that
    /// SAME chord — the two-sided fix, checked on both commands at once so
    /// they can never drift apart.
    #[test]
    fn linux_keep_emacs_restores_the_emacs_label_and_suppresses_the_native_one() {
        let keep = vec!["C-f".to_string()];
        let forward_char = COMMANDS.iter().find(|c| c.name == "Forward char").unwrap();
        let search = COMMANDS.iter().find(|c| c.name == "Search forward").unwrap();

        // Without the keep-list: Forward char's C-f is displaced (blank), Search
        // forward advertises Ctrl+F alongside its own emacs C-s.
        assert_eq!(join_slots_truthful(forward_char, Convention::Linux, Platform::Native, &[]), "");
        assert_eq!(join_slots_truthful(search, Convention::Linux, Platform::Native, &[]), "Ctrl+F");

        // WITH the keep-list: Forward char shows its kept emacs chord; Search
        // forward's native Ctrl+F vanishes (it no longer actually fires there),
        // leaving only Search forward's OWN un-displaced... wait, C-s IS still
        // displaced by Save's native Ctrl-S (unrelated to this keep entry), so
        // Search forward's label goes fully blank — it has NO chord that fires
        // on Linux once C-f is given back to Forward char.
        assert_eq!(join_slots_truthful(forward_char, Convention::Linux, Platform::Native, &keep), "C-f");
        assert_eq!(join_slots_truthful(search, Convention::Linux, Platform::Native, &keep), "");

        // Mac is completely unaffected by a Linux-only keep-list.
        assert_eq!(
            join_slots_truthful(forward_char, Convention::Mac, Platform::Native, &keep),
            join_slots_truthful(forward_char, Convention::Mac, Platform::Native, &[]),
        );
        assert_eq!(
            join_slots_truthful(search, Convention::Mac, Platform::Native, &keep),
            join_slots_truthful(search, Convention::Mac, Platform::Native, &[]),
        );
    }

    /// An UNLISTED chord is unaffected: keeping `C-f` does not touch `C-n`'s own
    /// displacement (New note's native still wins over Next line's emacs `C-n`).
    #[test]
    fn linux_keep_emacs_is_a_per_chord_door_not_a_policy_flip() {
        let keep = vec!["C-f".to_string()];
        let next_line = COMMANDS.iter().find(|c| c.name == "Next line").unwrap();
        assert_eq!(join_slots_truthful(next_line, Convention::Linux, Platform::Native, &keep), "");
        let new_note = COMMANDS.iter().find(|c| c.name == "New note").unwrap();
        assert_eq!(join_slots_truthful(new_note, Convention::Linux, Platform::Native, &keep), "Ctrl+N");
    }

    /// `effective_bindings`/`visible_effective_bindings` (the palette/rebind-menu
    /// doors) thread the keep-list all the way through — not just the pure
    /// `join_slots_truthful` unit.
    #[test]
    fn effective_bindings_reflects_the_linux_keep_emacs_list() {
        // This test's assertions only mean what they say under `Convention::Linux`
        // — pin it explicitly isn't available for `effective_bindings` (it always
        // reads `Convention::current()`), so gate the assertion the way the rest
        // of this suite's convention-proof tests do.
        if Convention::current() != Convention::Linux {
            return;
        }
        let keep = vec!["C-f".to_string()];
        let i = COMMANDS.iter().position(|c| c.name == "Forward char").unwrap();
        assert_eq!(effective_bindings(&[], &[])[i], "");
        assert_eq!(effective_bindings(&[], &keep)[i], "C-f");
    }

    /// THE LAW: `linux_keep_emacs` is a total no-op under `Convention::Mac` — a
    /// non-empty keep-list produces the BYTE-IDENTICAL label as an empty one,
    /// for every catalog command.
    #[test]
    fn linux_keep_emacs_is_inert_on_mac_for_the_whole_catalog() {
        let keep = vec!["C-f".to_string(), "C-b".to_string(), "C-n".to_string(), "C-p".to_string()];
        for c in COMMANDS.iter() {
            assert_eq!(
                join_slots_truthful(c, Convention::Mac, Platform::Native, &keep),
                join_slots_truthful(c, Convention::Mac, Platform::Native, &[]),
                "{}: linux_keep_emacs must be inert on Mac",
                c.name
            );
        }
    }

    // ── THE KEYMAP FLAVOR ROUND ──────────────────────────────────────────────

    /// TIER 4, WHOLE-PRESET FLAVOR: the same two-sided label fix
    /// [`linux_keep_emacs_restores_the_emacs_label_and_suppresses_the_native_one`]
    /// exercises for a hand-picked `["C-f"]`, now exercised for the FULL emacs
    /// flavor preset (`keymap::linux_emacs_preset_keep`) — "Forward char" gets
    /// its emacs `C-f` label back and "Search forward" loses the native
    /// `Ctrl+F` claim it would otherwise show. UNLIKE the hand-picked case,
    /// "Search forward" does NOT go blank here: its OWN emacs default (`C-s`)
    /// is ALSO in the whole preset (the letter `s` is displaced too, by
    /// Save's native Ctrl-S), so Save's native claim is suppressed right back
    /// and Search forward's bare `C-s` reappears — the whole-preset's actual
    /// shape, every displaced letter reverting to its own emacs owner at once.
    #[test]
    fn keymap_flavor_emacs_preset_restores_labels_two_sided() {
        let preset = crate::keymap::linux_emacs_preset_keep();
        let forward_char = COMMANDS.iter().find(|c| c.name == "Forward char").unwrap();
        let search = COMMANDS.iter().find(|c| c.name == "Search forward").unwrap();
        let save = COMMANDS.iter().find(|c| c.name == "Save").unwrap();
        assert_eq!(
            join_slots_truthful(forward_char, Convention::Linux, Platform::Native, &preset),
            "C-f"
        );
        assert_eq!(join_slots_truthful(search, Convention::Linux, Platform::Native, &preset), "C-s");
        // Save's native Ctrl-S claim is suppressed under the whole preset (its
        // letter `s` is kept too) — and Save has no emacs slot (the identity
        // round retired `C-x C-s`), so its label goes fully blank.
        assert_eq!(join_slots_truthful(save, Convention::Linux, Platform::Native, &preset), "");
    }

    /// THE LAW: the emacs flavor's WHOLE PRESET keep-list is ALSO a total no-op
    /// under `Convention::Mac` — no collisions exist there to keep, so every
    /// catalog label is byte-identical with the preset applied or not (mirrors
    /// [`linux_keep_emacs_is_inert_on_mac_for_the_whole_catalog`], but for the
    /// widened whole-catalog preset rather than a hand-picked few chords).
    #[test]
    fn keymap_flavor_emacs_preset_is_inert_on_mac_for_the_whole_catalog() {
        let preset = crate::keymap::linux_emacs_preset_keep();
        for c in COMMANDS.iter() {
            assert_eq!(
                join_slots_truthful(c, Convention::Mac, Platform::Native, &preset),
                join_slots_truthful(c, Convention::Mac, Platform::Native, &[]),
                "{}: the emacs keymap flavor must be inert on Mac",
                c.name
            );
        }
    }

    /// `Config::effective_linux_keep` is the ONE composition owner both dispatch
    /// (`keymap.rs`) and labels (`join_slots_truthful`, via this module) read —
    /// pinning that a `keymap = "emacs"` config produces the SAME label as
    /// passing the preset directly, so the two can never drift.
    #[test]
    fn config_effective_linux_keep_feeds_join_slots_truthful_identically_to_the_bare_preset() {
        let mut cfg = crate::config::Config::empty();
        cfg.keymap = Some("emacs".to_string());
        let via_config = cfg.effective_linux_keep();
        let bare_preset = crate::keymap::linux_emacs_preset_keep();
        let forward_char = COMMANDS.iter().find(|c| c.name == "Forward char").unwrap();
        assert_eq!(
            join_slots_truthful(forward_char, Convention::Linux, Platform::Native, &via_config),
            join_slots_truthful(forward_char, Convention::Linux, Platform::Native, &bare_preset),
        );
    }

    // ── THE INSERT-LINK-YIELDS-TO-KILL-LINE ROUND ──────────────────────────────

    /// HARD LAW (b): Insert link's VISIBLE effective binding is EMPTY on Linux —
    /// out of the box, no user config, under BOTH keymap flavors — while Mac
    /// still shows Cmd-K (the `keymap` flavor is a Linux-only concept; Mac's
    /// label is unaffected regardless). Drives the SAME `Config::
    /// effective_linux_keep()` composition the live palette/rebind-menu read,
    /// so a label surface can never advertise a Linux chord that dispatch (see
    /// `keymap::tests::out_of_the_box_linux_ctrl_k_is_kill_line_under_both_
    /// keymap_flavors`) would never actually honor.
    #[test]
    fn insert_link_has_no_visible_linux_binding_out_of_the_box_mac_shows_cmd_k() {
        let insert_link = COMMANDS.iter().find(|c| c.name == "Insert link…").unwrap();
        for flavor in ["native", "emacs"] {
            let mut cfg = crate::config::Config::empty();
            cfg.keymap = Some(flavor.to_string());
            let keep = cfg.effective_linux_keep();
            assert_eq!(
                join_slots_truthful(insert_link, Convention::Linux, Platform::Native, &keep),
                "",
                "Insert link must show no Linux chord out of the box under keymap={flavor:?}"
            );
            assert_eq!(
                join_slots_truthful(insert_link, Convention::Mac, Platform::Native, &keep),
                "⌘K",
                "Mac must still show Cmd-K under keymap={flavor:?} (the keep list is Linux-only)"
            );
        }
    }

    /// HARD LAW (d): the WHOLE CATALOG's Mac label is unaffected by the new
    /// built-in floor, under BOTH flavors — mirrors
    /// `linux_keep_emacs_is_inert_on_mac_for_the_whole_catalog`/
    /// `keymap_flavor_emacs_preset_is_inert_on_mac_for_the_whole_catalog`, but
    /// driven by the REAL `Config::effective_linux_keep()` composition (which,
    /// as of this round, always contains `keymap::linux_builtin_keep()`'s "C-k").
    #[test]
    fn effective_linux_keep_builtin_floor_is_inert_on_mac_for_the_whole_catalog() {
        for flavor in ["native", "emacs"] {
            let mut cfg = crate::config::Config::empty();
            cfg.keymap = Some(flavor.to_string());
            let keep = cfg.effective_linux_keep();
            for c in COMMANDS.iter() {
                assert_eq!(
                    join_slots_truthful(c, Convention::Mac, Platform::Native, &keep),
                    join_slots_truthful(c, Convention::Mac, Platform::Native, &[]),
                    "{}: the built-in keep floor must be inert on Mac (keymap={flavor:?})",
                    c.name
                );
            }
        }
    }
}

#[cfg(test)]
mod identity_snapshot {
    //! THE KEYMAP-DEFAULTS-AS-DATA ROUND'S OWN IDENTITY-PROOF TOOL: dumps the
    //! ENTIRE `COMMANDS` catalog (name/action/native/emacs/native_only/
    //! web_only) in stable order, one line per command. `--ignored
    //! --nocapture` this before AND after any future round that touches how
    //! `COMMANDS` is constructed (this round used it to diff byte-for-byte
    //! against the pre-refactor base commit, on a temporary copy of this
    //! same test, and got an exact match) — since every downstream consumer
    //! (`effective_bindings`, `join_slots_truthful`, the menu roster, the
    //! GUIDE reference table, the rebind menu, whichkey) is a pure function
    //! of this exact data, an identical dump is sufficient to prove the
    //! whole label/dispatch-agreement surface is behavior-identical without
    //! re-deriving every derived view by hand. Kept `#[ignore]`d (zero cost
    //! to the normal suite) as a reusable tool, not a law test with its own
    //! failure mode — the data-shape laws above already guard the catalog
    //! going forward.
    use super::*;

    #[test]
    #[ignore]
    fn print_full_catalog_snapshot() {
        for c in COMMANDS.iter() {
            println!("{}|{:?}|{}|{}|{}|{}", c.name, c.action, c.native, c.emacs, c.native_only, c.web_only);
        }
    }
}
