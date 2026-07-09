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
//! fuzzy-matches is `names()`, in this exact order, so the selected ROW index maps
//! straight back to `COMMANDS[i].action` (see the palette accept branch in
//! `actions::apply_core`).
//!
//! `InsertChar` / prefix / ignore are intentionally EXCLUDED: the palette lists
//! actions a user would summon or rebind by name, never self-insertion. MOTIONS
//! are split (user-decided 2026-07-10, superseding the original all-motions
//! exclusion): the curated NAVIGATION motions (word / line / document) ARE catalog
//! rows — so they show in Cmd-P + the Keybindings rebind menu and are rebindable
//! via `[keys]` (the door that lets a hand reclaim the retired Option-letter word
//! motion as `forward_word = "M-f"` etc.) — while the char/line ARROW motions
//! (`ForwardChar` / `NextLine` / …) stay keymap-only: arrows are not commands
//! anyone summons or rebinds, and the catalog stays calm. The law test
//! `catalog_motions_are_exactly_the_curated_navigation_set` pins the split.

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
}

/// The command catalog, in stable display order. The fuzzy corpus is the NAMES
/// in this order, so a selected row index indexes straight back into this slice.
/// Each row carries its two binding slots — native (Cmd) and emacs.
pub static COMMANDS: &[Command] = &[
    Command { name: "Go to file…",       action: Action::OpenGoto,        native: "Cmd-O",   emacs: ""        },
    Command { name: "Switch project…",   action: Action::OpenProject,     native: "Cmd-S-p", emacs: ""        },
    // RECENT PROJECTS: opens the SWITCH-PROJECT navigator pre-lensed onto its Recent
    // lens (the fold that retired the standalone RecentProjects picker; recents are a
    // lens now, see `crate::recents`). No default chord — the palette + File menu ARE
    // its entry points (like Settings/About); a real `Action`, independently rebindable.
    Command { name: "Recent projects…",  action: Action::OpenRecentProjects, native: "",     emacs: ""        },
    Command { name: "Browse files…",     action: Action::OpenBrowse,      native: "",        emacs: ""        },
    // GO TO HEADING: opens GO-TO pre-lensed onto its HEADINGS lens (the fold that
    // retired the standalone Outline picker; jump-to-heading is a Go-to lens now,
    // also reachable via ⌘O → ←/→). Palette-only — no default chord (Cmd-Shift-O
    // toggles the persistent margin outline); still fully reachable + rebindable.
    // Named "Go to heading…" to say what it does, paralleling "Go to file…".
    Command { name: "Go to heading…",    action: Action::OpenOutline,     native: "",        emacs: ""        },
    Command { name: "Spell suggestions…", action: Action::OpenSpellSuggest, native: "Cmd-;", emacs: ""        },
    // VERSION HISTORY (the local-history timeline): renamed from "History" so it no
    // longer shadows the "Local history" setting; says it is the version timeline.
    Command { name: "Version history…",  action: Action::OpenHistory,     native: "Cmd-S-h", emacs: ""        },
    // CLEAN UNUSED ASSETS: summon the Asset Cleaner — a picker of the ORPHAN image
    // files under the active project (an `assets/` image no document references,
    // `crate::assets`). Enter moves the row's file to the macOS Trash (recoverable).
    // Opens a picker, so it takes the ellipsis (picker-naming convention). No default
    // chord — the palette IS its entry point, like Settings/History; a real `Action`,
    // independently rebindable via `[keys] clean_unused_assets`.
    Command { name: "Clean unused assets…", action: Action::OpenAssetClean, native: "",       emacs: ""        },
    // KEEP VERSION: THE CONSCIOUS MARK — pin the current file's state as a
    // prune-exempt local-history snapshot ("I care about this one"). No default
    // chord — the palette IS its entry point, like Settings/About; a real `Action`,
    // independently rebindable via `[keys] keep_version`.
    Command { name: "Keep version",      action: Action::KeepVersion,     native: "",        emacs: ""        },
    Command { name: "Last file",         action: Action::LastBuffer,      native: "C-Tab",   emacs: ""        },
    Command { name: "New note",          action: Action::NewNote,         native: "Cmd-N",   emacs: ""        },
    Command { name: "Move note…",        action: Action::MoveNote,        native: "",        emacs: ""        },
    // FINISH FILE: the emacsclient "server-edit" convention — save, notify any daemon
    // `--wait` client, and switch to the previously-open file. Palette-only now (the
    // emacs `C-x #` default is retired). See `crate::daemon`. (Action stays
    // `FinishBuffer`.)
    Command { name: "Finish file",       action: Action::FinishBuffer,    native: "",        emacs: ""        },
    // FOLLOW LINK: open the markdown link under the caret in the OS default browser
    // (a user-initiated handoff, not an app network fetch). Emacs slot `C-c C-o`
    // (org-mode's open-link-at-point); native slot left empty (no universal macOS
    // convention). A caret outside a link is a calm no-op. Rebindable via `[keys]`.
    Command { name: "Follow link",       action: Action::FollowLink,      native: "",        emacs: "C-c C-o" },
    Command { name: "Switch theme…",     action: Action::OpenThemeMenu,   native: "Cmd-T",   emacs: ""        },
    Command { name: "Caret style…",      action: Action::OpenCaretMenu,   native: "",        emacs: ""        },
    Command { name: "Dictionary…",       action: Action::OpenDictionaryMenu, native: "",     emacs: ""        },
    // TOGGLE SPELLCHECK: the global on/off escape hatch (default ON). No default
    // chord — the palette IS its entry point, like Settings/Dictionary; a real
    // `Action` (unlike the `writing_nits` sentinel below), so it is unambiguous
    // through `RunAction` and independently rebindable via `[keys]`.
    Command { name: "Toggle spellcheck", action: Action::ToggleSpellcheck, native: "",     emacs: ""        },
    Command { name: "Toggle hidden files", action: Action::ToggleHiddenFiles, native: "Cmd-S-.", emacs: ""  },
    Command { name: "Toggle caret style", action: Action::ToggleCaretMode, native: "",       emacs: ""        },
    Command { name: "Toggle page mode",  action: Action::TogglePageMode,  native: "",        emacs: ""        },
    // TOGGLE WRITING NITS: the quiet mechanical-typo underline highlighter (default
    // ON). A render-only toggle with NO default chord — the palette IS its entry
    // point, like Settings — backed by a real `Action::ToggleWritingNits` (the former
    // `Ignore` sentinel is retired), so it round-trips through `RunAction`
    // unambiguously and is independently rebindable via `[keys] toggle_writing_nits`.
    Command { name: "Toggle writing nits", action: Action::ToggleWritingNits, native: "",    emacs: ""        },
    Command { name: "Widen page",        action: Action::PageWider,       native: "",        emacs: ""        },
    Command { name: "Narrow page",       action: Action::PageNarrower,    native: "",        emacs: ""        },
    // RESET PAGE WIDTH: no default chord — the palette IS its entry point, like
    // Settings, plus a DOUBLE-CLICK on the draggable page edge (`app/input/drags.rs`).
    // "There's no easy way back" once you've dragged/widened/narrowed the column.
    Command { name: "Reset page width",  action: Action::PageReset,       native: "",        emacs: ""        },
    Command { name: "Toggle debug",      action: Action::ToggleDebug,     native: "",        emacs: ""        },
    // TOGGLE OUTLINE: the persistent margin table-of-contents (ON by default,
    // flipped 2026-07-09). The Cmd-Shift-O chord (formerly the summoned heading-jump
    // picker's) now toggles it; rebindable via config `[keys] toggle_outline`.
    Command { name: "Toggle outline",    action: Action::ToggleOutline,   native: "Cmd-S-o", emacs: ""        },
    // TOGGLE TYPEWRITER SCROLL: pin the caret's line centered so the doc scrolls under
    // it (OFF by default). No default chord — palette-only, like About/Settings; a
    // real `Action`, independently rebindable via config `[keys] toggle_typewriter_scroll`.
    Command { name: "Toggle typewriter scroll", action: Action::ToggleTypewriter, native: "", emacs: ""      },
    // TOGGLE MENU BAR: the awl-rendered menu bar (web/Linux; absent on macOS where the
    // native NSMenu bar is the door). No default chord — palette-only, like
    // About/Settings; a real `Action`, independently rebindable via config `[keys]
    // toggle_menu_bar`. Lets a web/Linux user hide the bar (a user-settled requirement).
    Command { name: "Toggle menu bar",   action: Action::ToggleMenuBar,   native: "",        emacs: ""        },
    // ABOUT: no default chord — the palette IS its entry point (like Settings),
    // plus the macOS menu bar's App → "About Awl" item (`menu.rs`, routed —
    // see that module's doc for why this is NOT muda's predefined About).
    Command { name: "About",             action: Action::About,           native: "",        emacs: ""        },
    // LIFETIME STATS: the summoned personal ODOMETER card (characters, writing
    // time, files touched, caret travel, your world) — the LIFETIME figures split
    // out of the held stats HUD. No default chord — the palette IS its entry point
    // (like Settings/About); a real `Action`, independently rebindable via `[keys]
    // lifetime_stats`. See `lifetime.rs`.
    Command { name: "Lifetime stats",    action: Action::LifetimeStats,   native: "",        emacs: ""        },
    // LINE ENDINGS: toggle the active file's on-disk ending (LF <-> CRLF). No default
    // chord — the palette IS its entry point (a rare command, like Settings/About); a
    // real `Action` (`ConvertLineEndings`), independently rebindable via `[keys]`.
    Command { name: "Line endings…",     action: Action::ConvertLineEndings, native: "",     emacs: ""        },
    // ALIGN TABLE: re-pad the GFM table under the caret so its `|` line up (source
    // alignment, never a drawn grid). No default chord — the palette IS its entry
    // point (like Settings/About); a real `Action`, independently rebindable.
    Command { name: "Align table",       action: Action::AlignTable,      native: "",        emacs: ""        },
    // MARKDOWN FORMATTING COMMANDS (see `actions/format.rs`): each a TOGGLE applied as
    // one undoable edit, markdown-only. The three with a UNIVERSAL native convention get
    // a Cmd chord — Cmd-B = Bold, Cmd-E = Inline code (both free under Super: 'b'/'e' are
    // unused there). Cmd-I (the universal Italic chord) is DELIBERATELY NOT taken — it is
    // already the held stats HUD (`keymap.rs`), so Italic stays palette-only rather than
    // steal that chord. The block toggles + Highlight/Strikethrough have no obvious native
    // convention, so they are palette-only (like Align Table). All independently
    // rebindable via `[keys]` (the emacs slot is left empty for a user to fill).
    Command { name: "Blockquote",        action: Action::ToggleBlockquote,   native: "",      emacs: "" },
    Command { name: "Bullet list",       action: Action::ToggleBulletList,   native: "",      emacs: "" },
    Command { name: "Numbered list",     action: Action::ToggleNumberedList, native: "",      emacs: "" },
    Command { name: "Task list",         action: Action::ToggleTaskList,     native: "",      emacs: "" },
    Command { name: "Heading",           action: Action::ToggleHeading,      native: "",      emacs: "" },
    Command { name: "Code block",        action: Action::ToggleCodeBlock,    native: "",      emacs: "" },
    Command { name: "Bold",              action: Action::Bold,               native: "Cmd-B", emacs: "" },
    Command { name: "Italic",            action: Action::Italic,             native: "",      emacs: "" },
    Command { name: "Inline code",       action: Action::InlineCode,         native: "Cmd-E", emacs: "" },
    Command { name: "Highlight",         action: Action::Highlight,          native: "",      emacs: "" },
    Command { name: "Strikethrough",     action: Action::Strikethrough,      native: "",      emacs: "" },
    // NOTE: the held stats HUD (Cmd-I) is deliberately NOT a palette command. It is a
    // momentary HOLD-to-peek (shown while the key is down, gone the instant it lifts), so
    // a DISCRETE selection — which has no key-release to dismiss it — would leave it stuck
    // on. Its ONLY summon path is the held Cmd-I key (resolved in `keymap.rs`); see `hud.rs`.
    Command { name: "Save",              action: Action::Save,            native: "Cmd-S",   emacs: ""        },
    Command { name: "Quit",              action: Action::Quit,            native: "Cmd-Q",   emacs: ""        },
    Command { name: "Search forward",    action: Action::SearchForward,   native: "Cmd-F",   emacs: "C-s"     },
    Command { name: "Search backward",   action: Action::SearchBackward,  native: "Cmd-S-f", emacs: "C-r"     },
    Command { name: "Find and replace…", action: Action::OpenReplace,     native: "Cmd-R",   emacs: ""        },
    Command { name: "Undo",              action: Action::Undo,            native: "Cmd-Z",   emacs: "C-/"     },
    Command { name: "Redo",              action: Action::Redo,            native: "Cmd-S-z", emacs: ""        },
    // CLIPBOARD + SELECT-ALL: bound in the keymap (native Cmd-C/X/V/A, emacs M-w/C-w/C-y)
    // but previously absent here, so they were invisible to Cmd-P and the rebind menu.
    // Listed with their ACTUAL bindings so they show + become rebindable. (Bare C-a stays
    // LineStart in the emacs slot, so Select all is Cmd-only.)
    Command { name: "Copy",              action: Action::CopyRegion,      native: "Cmd-C",   emacs: ""        },
    Command { name: "Cut",               action: Action::KillRegion,      native: "Cmd-X",   emacs: "C-w"     },
    Command { name: "Paste",             action: Action::Yank,            native: "Cmd-V",   emacs: "C-y"     },
    Command { name: "Select all",        action: Action::SelectAll,       native: "Cmd-A",   emacs: ""        },
    Command { name: "Zoom in",           action: Action::ZoomIn,          native: "Cmd-=",   emacs: ""        },
    Command { name: "Zoom out",          action: Action::ZoomOut,         native: "Cmd--",   emacs: ""        },
    Command { name: "Reset zoom",        action: Action::ZoomReset,       native: "Cmd-0",   emacs: ""        },
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
    Command { name: "Forward word",      action: Action::ForwardWord,     native: "M-Right",   emacs: ""     },
    Command { name: "Backward word",     action: Action::BackwardWord,    native: "M-Left",    emacs: ""     },
    Command { name: "Line start",        action: Action::LineStart,       native: "Cmd-Left",  emacs: "C-a"  },
    Command { name: "Line end",          action: Action::LineEnd,         native: "Cmd-Right", emacs: "C-e"  },
    Command { name: "Document start",    action: Action::BufferStart,     native: "Cmd-Up",    emacs: ""     },
    Command { name: "Document end",      action: Action::BufferEnd,       native: "Cmd-Down",  emacs: ""     },
    // Settings has NO default chord — the palette IS its entry point. It summons the
    // faceted SETTINGS MENU (the friendly default); the raw config-as-text file lives
    // behind the menu's "Edit config as text" row (`Action::OpenSettings`).
    Command { name: "Settings…",         action: Action::OpenSettingsMenu, native: "",       emacs: ""        },
    // Keybindings has NO default chord either — summon it by name (Cmd-P) like
    // Settings; it is the GAME-STYLE rebind menu (capture a key per command). It is
    // itself rebindable via `[keys] keybindings = "..."`.
    Command { name: "Keybindings…",      action: Action::OpenKeybindings, native: "",        emacs: ""        },
];

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

/// The slugified action name for catalog command `i` (panics out of range — only
/// the overlay's own indices, which are corpus==catalog order, reach this). Used by
/// the rebind menu to key a `[keys]` entry off the highlighted command.
pub fn slug_of_index(i: usize) -> String {
    slug(COMMANDS[i].name)
}

/// The display NAME of catalog command `i` (for the rebind menu's prompt / notices).
pub fn name_of_index(i: usize) -> &'static str {
    COMMANDS[i].name
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
pub fn slug_for_action(action: &Action) -> Option<String> {
    COMMANDS.iter().find(|c| &c.action == action).map(|c| slug(c.name))
}

/// Whether the catalog command with config `slug` carries a NATIVE (macOS) chord — the
/// "has a chord to graduate INTO" predicate the graduation ranking keys on (injected
/// into [`crate::stats::Stats::graduation_candidates`] so the pure ledger query stays
/// catalog-free). `false` for an unknown slug or a palette-only command (empty native
/// slot).
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
pub fn peek_row_for_slug(slug_want: &str) -> Option<crate::peek::PeekRow> {
    let c = COMMANDS.iter().find(|c| slug(c.name) == slug_want)?;
    let native = c.native.trim();
    if native.is_empty() {
        return None;
    }
    Some(crate::peek::PeekRow {
        chord: crate::keyspec::mac_glyph_chord(native),
        name: c.name.trim_end_matches('…').trim().to_string(),
    })
}

/// The EFFECTIVE binding label per command, parallel to [`names`], showing BOTH
/// slots. When a config `[keys]` override lists valid chord(s) for the command's
/// action, those (up to 2) are shown joined by `·`; otherwise the static native +
/// emacs defaults are shown. Drives the palette's binding column, so it teaches the
/// chords that ACTUALLY trigger each command. `keys` is the config `[keys]` list.
pub fn effective_bindings(keys: &[(String, Vec<String>)]) -> Vec<String> {
    COMMANDS
        .iter()
        .map(|c| {
            let chords = effective_chords(c, keys);
            if effective_is_override(c, keys) {
                // Slot 1 (index 0) is NATIVE → mac glyphs; slot 2+ is EMACS → terse
                // text, matching the static `join_slots` rule.
                chords
                    .iter()
                    .enumerate()
                    .map(|(i, ch)| {
                        if i == 0 {
                            crate::keyspec::mac_glyph_chord(ch)
                        } else {
                            ch.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" · ")
            } else {
                join_slots(c.native, c.emacs)
            }
        })
        .collect()
}

/// The EFFECTIVE chord LIST for one command (NOT joined): a valid config override's
/// chords (up to 2) when present, else the command's static native/emacs slots
/// (empty slots dropped). The per-chord form [`effective_bindings`] joins for
/// display and [`binding_conflict`] compares for clashes.
fn effective_chords(c: &Command, keys: &[(String, Vec<String>)]) -> Vec<String> {
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

/// The catalog command NAMES, in catalog order — the fuzzy corpus the palette
/// overlay filters over.
pub fn names() -> Vec<String> {
    COMMANDS.iter().map(|c| c.name.to_string()).collect()
}

/// The EFFECTIVE chord LISTS per command, parallel to [`names`] — each command's
/// active chords (a valid config override, else the static native/emacs slots),
/// UN-joined and un-glyphified (empty slots dropped). This is the raw data the
/// WHICH-KEY panel derives its prefix continuations from (`crate::whichkey`), so the
/// panel filters the chords that start with a prefix (`C-x …`) straight off the
/// catalog + config and can never drift from a hardcoded duplicate list. The
/// per-command joined DISPLAY form is [`effective_bindings`]; this is the structured
/// sibling for machine consumers.
pub fn effective_chord_lists(keys: &[(String, Vec<String>)]) -> Vec<Vec<String>> {
    COMMANDS.iter().map(|c| effective_chords(c, keys)).collect()
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

    #[test]
    fn catalog_non_empty_and_named() {
        assert!(!COMMANDS.is_empty(), "the command catalog must list commands");
        for c in COMMANDS {
            assert!(!c.name.trim().is_empty(), "command needs a display name");
        }
        // Every entry HAS at least one filled slot except the PALETTE-ONLY commands
        // (summoned by name, no default chord); the model is CAPPED at 2 — exactly
        // the two slots exist. The identity round RETIRED the emacs C-x defaults, so
        // the palette-only set grew: every command whose C-x default was retired
        // WITHOUT gaining a native chord (Browse files… / Move note… / Finish file /
        // Toggle page mode / Toggle caret style / Widen page / Narrow page / Toggle
        // debug) joins the pre-existing bindless set here. About's + Recent projects'
        // other summon door is the macOS menu bar, not a keymap chord.
        //
        // The markdown formatting commands are MOSTLY palette-only (like Align table);
        // the exceptions are Bold (Cmd-B) and Inline code (Cmd-E), which DO carry a
        // native chord and so are NOT exempt — the assertion below verifies theirs.
        // Italic is palette-only despite its universal Cmd-I convention (Cmd-I is the
        // held stats HUD; see the catalog note).
        const PALETTE_ONLY: &[&str] = &[
            "Settings…",
            "Keybindings…",
            "Caret style…",
            "Dictionary…",
            "Toggle spellcheck",
            "Toggle writing nits",
            "Reset page width",
            "About",
            "Lifetime stats",
            "Line endings…",
            "Align table",
            "Recent projects…",
            "Go to heading…",
            "Toggle typewriter scroll",
            "Toggle menu bar",
            "Keep version",
            "Clean unused assets…",
            // Emacs C-x default retired, no native chord assigned (identity round):
            "Browse files…",
            "Move note…",
            "Finish file",
            "Toggle page mode",
            "Toggle caret style",
            "Widen page",
            "Narrow page",
            "Toggle debug",
            // Format toggles with no native convention:
            "Blockquote",
            "Bullet list",
            "Numbered list",
            "Task list",
            "Heading",
            "Code block",
            "Italic",
            "Highlight",
            "Strikethrough",
        ];
        for c in COMMANDS {
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
        // It is summoned ONLY by the held Cmd-I key (`keymap.rs`), never the catalog.
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
        for c in COMMANDS {
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

    #[test]
    fn effective_bindings_reflect_overrides() {
        // No config: effective == default labels.
        assert_eq!(effective_bindings(&[]), bindings());
        // An override for "switch_theme" surfaces in the palette column. Slot 1 (the
        // NATIVE slot) renders as mac modifier GLYPHS, so `C-t` shows as `⌃T`.
        let keys = vec![("switch_theme".to_string(), vec!["C-t".to_string()])];
        let eff = effective_bindings(&keys);
        let i = COMMANDS.iter().position(|c| c.name == "Switch theme…").unwrap();
        assert_eq!(eff[i], "⌃T");
        // A BAD chord falls back to the default label (consistent with the keymap) —
        // Switch theme's native default is now Cmd-T (the emacs C-x t is retired).
        let bad = vec![("switch_theme".to_string(), vec!["C-frobnicate".to_string()])];
        let eff = effective_bindings(&bad);
        assert_eq!(eff[i], "⌘T");
    }

    #[test]
    fn effective_bindings_show_both_slots() {
        // Save's emacs C-x C-s default is retired, so it now shows only its NATIVE
        // slot as mac GLYPHS (`Cmd-S` → `⌘S`).
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
        // Settings has no slots → empty label.
        let s = COMMANDS.iter().position(|c| c.name == "Settings…").unwrap();
        assert_eq!(bindings()[s], "");
        // A 2-chord config override surfaces BOTH chords, joined — slot 1 glyphified,
        // even when it reclaims a retired chord (Save ← Cmd-S + C-x C-s).
        let keys = vec![("save".to_string(), vec!["Cmd-S".to_string(), "C-x C-s".to_string()])];
        assert_eq!(effective_bindings(&keys)[i], "⌘S · C-x C-s");
        // Only the VALID chords of an override are shown; an invalid one is dropped.
        let mixed = vec![("save".to_string(), vec!["Cmd-S".to_string(), "C-frobnicate".to_string()])];
        assert_eq!(effective_bindings(&mixed)[i], "⌘S");
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
        // fresh keymap (the C-c prefix path) — the catalog/keymap agreement sweep
        // relies on this, pinned here explicitly too.
        assert!(crate::keymap::parse_binding("C-c C-o").is_ok());
        assert_eq!(resolve_default_chord("C-c C-o"), Action::FollowLink);
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
        // chords: Bold = Cmd-B, Inline Code = Cmd-E; every other formatting command
        // is palette-only (empty native + emacs slot).
        let formatting: &[(&str, Action, &str)] = &[
            ("Blockquote", Action::ToggleBlockquote, ""),
            ("Bullet list", Action::ToggleBulletList, ""),
            ("Numbered list", Action::ToggleNumberedList, ""),
            ("Task list", Action::ToggleTaskList, ""),
            ("Heading", Action::ToggleHeading, ""),
            ("Code block", Action::ToggleCodeBlock, ""),
            ("Bold", Action::Bold, "Cmd-B"),
            ("Italic", Action::Italic, ""),
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
        // Cmd-I (the universal Italic convention) is DELIBERATELY not taken — it is the
        // held stats HUD (`keymap.rs`), so binding it to Italic here would be a clash.
        // Italic therefore carries no native chord (asserted `""` above), and Cmd-B /
        // Cmd-E introduce no catalog conflict (the pairwise sweep proves the latter).
        assert_eq!(binding_conflict("Cmd-B", "bold", &[]), None);
        assert_eq!(binding_conflict("Cmd-E", "inline_code", &[]), None);
        // The effective (config-free) palette labels show the two native chords as
        // mac glyphs, and Italic shows nothing.
        let eff = effective_bindings(&[]);
        let bold = COMMANDS.iter().position(|c| c.name == "Bold").unwrap();
        let ital = COMMANDS.iter().position(|c| c.name == "Italic").unwrap();
        let code = COMMANDS.iter().position(|c| c.name == "Inline code").unwrap();
        assert_eq!(eff[bold], "⌘B");
        assert_eq!(eff[ital], "");
        assert_eq!(eff[code], "⌘E");
    }

    /// Resolve a catalog DEFAULT chord ("Cmd-S", "C-x C-s", "C-x }") through a
    /// FRESH default [`crate::keymap::KeymapState`], token by token, returning the
    /// LAST resolved action — the `C-x` token resolves to `BeginPrefix` and arms
    /// the prefix state, exactly as the live keypresses would.
    fn resolve_default_chord(spec: &str) -> Action {
        let mut km = crate::keymap::KeymapState::new();
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
        for c in COMMANDS {
            for chord in [c.native, c.emacs] {
                if chord.trim().is_empty() {
                    continue; // palette-only slot (Settings / Keybindings / …)
                }
                // 1) Every non-empty slot PARSES as a config binding — the
                //    rebinder's grammar accepts the very defaults it displays.
                assert!(
                    crate::keymap::parse_binding(chord).is_ok(),
                    "{}: default chord {chord:?} must parse via parse_binding",
                    c.name
                );
                // 2) The chord RESOLVES through a fresh default keymap to exactly
                //    the catalog action, so label and dispatch can never drift.
                assert_eq!(
                    resolve_default_chord(chord),
                    c.action,
                    "{}: default chord {chord:?} must resolve to the catalog action",
                    c.name
                );
            }
            // 3) The config ACTION NAME round-trips: slug(name) → action_for_name
            //    → this command's action (every catalog row is rebind-addressable).
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
        for c in COMMANDS {
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

    #[test]
    fn slug_for_action_and_has_native_chord_key_the_usage_ledger() {
        // A catalog command resolves to its slug; the SAME identity `record_recent`
        // uses, so the ledger and the Recent MRU agree on "a command".
        assert_eq!(slug_for_action(&Action::OpenGoto).as_deref(), Some("go_to_file"));
        assert_eq!(slug_for_action(&Action::OpenThemeMenu).as_deref(), Some("switch_theme"));
        // A motion / self-insert / prefix carries no catalog command → None (no alloc).
        assert_eq!(slug_for_action(&Action::ForwardChar), None);
        assert_eq!(slug_for_action(&Action::InsertChar('x')), None);
        assert_eq!(slug_for_action(&Action::BeginPrefix), None);
        // has_native_chord: true for a native-slot command, false for palette-only.
        assert!(has_native_chord("go_to_file"), "Go to file… carries Cmd-O");
        assert!(has_native_chord("save"), "Save carries Cmd-S");
        assert!(!has_native_chord("settings"), "Settings… is palette-only");
        assert!(!has_native_chord("about"), "About is palette-only");
        assert!(!has_native_chord("reset_page_width"), "Reset page width is palette-only");
        assert!(!has_native_chord("no_such_command"), "unknown slug: false");
        // The two agree: every slug `slug_for_action` yields is a real catalog slug.
        assert!(has_native_chord(&slug_for_action(&Action::Save).unwrap()));
    }

    #[test]
    fn peek_row_resolves_native_chord_and_name_or_none_for_palette_only() {
        // A native-chord command → its glyph chord + ellipsis-stripped name.
        assert_eq!(
            peek_row_for_slug("go_to_file"),
            Some(crate::peek::PeekRow { chord: "⌘O".into(), name: "Go to file".into() })
        );
        assert_eq!(
            peek_row_for_slug("switch_theme"),
            Some(crate::peek::PeekRow { chord: "⌘T".into(), name: "Switch theme".into() })
        );
        // A palette-only command (no native chord to teach) → None, so it never
        // surfaces as a peek/footer row even if slow-door usage ranks it.
        assert_eq!(peek_row_for_slug("settings"), None);
        assert_eq!(peek_row_for_slug("about"), None);
        // An unknown slug → None (defensive).
        assert_eq!(peek_row_for_slug("no_such_command"), None);
    }

    #[test]
    fn catalog_motions_are_exactly_the_curated_navigation_set() {
        // THE MOTION SPLIT (user-decided 2026-07-10, superseding the original
        // all-motions exclusion): the curated NAVIGATION motions are catalog rows
        // (palette-visible + rebindable); the char/line ARROW motions stay
        // keymap-only. Self-insertion never enters the catalog.
        const NAVIGATION_MOTIONS: &[Action] = &[
            Action::ForwardWord,
            Action::BackwardWord,
            Action::LineStart,
            Action::LineEnd,
            Action::BufferStart,
            Action::BufferEnd,
        ];
        for c in COMMANDS {
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
        // … and the arrow motions stay OUT (spot-pinned; `slug_for_action` is the
        // structural gate every arrow press rides).
        for m in [Action::ForwardChar, Action::BackwardChar, Action::NextLine, Action::PreviousLine] {
            assert_eq!(slug_for_action(&m), None, "{m:?} must stay keymap-only");
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
        // glyphified: M-f → ⌥F), teaching the chord the user chose.
        let keys = vec![("forward_word".to_string(), vec!["M-f".to_string()])];
        let i = COMMANDS.iter().position(|c| c.name == "Forward word").unwrap();
        assert_eq!(effective_bindings(&keys)[i], "⌥F");
    }
}
