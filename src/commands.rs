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
//! Motions / `InsertChar` / prefix / ignore are intentionally EXCLUDED: the
//! palette lists command-ish actions a user would summon by name, not cursor
//! motions or self-insertion. (The native Cmd-arrow motions therefore live only in
//! the keymap, not here.)

use crate::keymap::Action;

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
    Command { name: "Go to file",        action: Action::OpenGoto,        native: "",        emacs: "C-x C-f" },
    Command { name: "Switch project",    action: Action::OpenProject,     native: "",        emacs: "C-x p"   },
    Command { name: "Browse files",      action: Action::OpenBrowse,      native: "",        emacs: "C-x j"   },
    Command { name: "Outline",           action: Action::OpenOutline,     native: "Cmd-S-o", emacs: ""        },
    Command { name: "Spell suggestions",  action: Action::OpenSpellSuggest, native: "Cmd-;", emacs: ""        },
    Command { name: "Last file",         action: Action::LastBuffer,      native: "",        emacs: "C-x b"   },
    Command { name: "New note",          action: Action::NewNote,         native: "",        emacs: "C-x n"   },
    Command { name: "Move note",         action: Action::MoveNote,        native: "",        emacs: "C-x m"   },
    Command { name: "Switch theme",      action: Action::OpenThemeMenu,   native: "",        emacs: "C-x t"   },
    Command { name: "Caret style",       action: Action::OpenCaretMenu,   native: "",        emacs: ""        },
    Command { name: "Toggle caret mode", action: Action::ToggleCaretMode, native: "",        emacs: "C-x c"   },
    Command { name: "Toggle page mode",  action: Action::TogglePageMode,  native: "",        emacs: "C-x w"   },
    Command { name: "Focus mode",        action: Action::CycleFocusMode,  native: "",        emacs: "C-x d"   },
    Command { name: "Toggle FPS",        action: Action::ToggleFps,       native: "",        emacs: "C-x r"   },
    // NOTE: the held stats HUD (Cmd-I) is deliberately NOT a palette command. It is a
    // momentary HOLD-to-peek (shown while the key is down, gone the instant it lifts), so
    // a DISCRETE selection — which has no key-release to dismiss it — would leave it stuck
    // on. Its ONLY summon path is the held Cmd-I key (resolved in `keymap.rs`); see `hud.rs`.
    Command { name: "Save",              action: Action::Save,            native: "Cmd-S",   emacs: "C-x C-s" },
    Command { name: "Quit",              action: Action::Quit,            native: "",        emacs: "C-x C-c" },
    Command { name: "Search forward",    action: Action::SearchForward,   native: "Cmd-F",   emacs: "C-s"     },
    Command { name: "Search backward",   action: Action::SearchBackward,  native: "Cmd-S-f", emacs: "C-r"     },
    Command { name: "Replace",           action: Action::OpenReplace,     native: "Cmd-M-f", emacs: ""        },
    Command { name: "Undo",              action: Action::Undo,            native: "Cmd-Z",   emacs: "C-/"     },
    Command { name: "Redo",              action: Action::Redo,            native: "Cmd-S-z", emacs: ""        },
    Command { name: "Zoom in",           action: Action::ZoomIn,          native: "Cmd-=",   emacs: ""        },
    Command { name: "Zoom out",          action: Action::ZoomOut,         native: "Cmd--",   emacs: ""        },
    Command { name: "Reset zoom",        action: Action::ZoomReset,       native: "Cmd-0",   emacs: ""        },
    // Settings has NO default chord — the palette IS its entry point. It opens the
    // config file (creating the commented default first) for editing as text.
    Command { name: "Settings",          action: Action::OpenSettings,    native: "",        emacs: ""        },
    // Keybindings has NO default chord either — summon it by name (Cmd-P) like
    // Settings; it is the GAME-STYLE rebind menu (capture a key per command). It is
    // itself rebindable via `[keys] keybindings = "..."`.
    Command { name: "Keybindings",       action: Action::OpenKeybindings, native: "",        emacs: ""        },
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
/// underscores ("Go to file" -> "go_to_file", "Switch theme" -> "switch_theme").
/// Both the rebinder ([`action_for_name`]) and the palette display
/// ([`effective_bindings`]) key off this, so a `[keys]` entry and the shown chord
/// stay consistent.
pub fn slug(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace(' ', "_")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_non_empty_and_named() {
        assert!(!COMMANDS.is_empty(), "the command catalog must list commands");
        for c in COMMANDS {
            assert!(!c.name.trim().is_empty(), "command needs a display name");
        }
        // Every entry HAS at least one filled slot except the bindless, palette-only
        // Settings / Keybindings / Caret style; the model is CAPPED at 2 — exactly the
        // two slots exist.
        for c in COMMANDS {
            if c.name != "Settings" && c.name != "Keybindings" && c.name != "Caret style" {
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
    fn action_for_name_matches_label_and_slug() {
        // Both the human label and the snake_case slug resolve to the same action.
        assert_eq!(action_for_name("Switch theme"), Some(Action::OpenThemeMenu));
        assert_eq!(action_for_name("switch_theme"), Some(Action::OpenThemeMenu));
        assert_eq!(action_for_name("go_to_file"), Some(Action::OpenGoto));
        assert_eq!(action_for_name("settings"), Some(Action::OpenSettings));
        // The DEBUG frame counter is a palette command, so it is rebindable via the
        // config `[keys]` action name ("toggle_fps").
        assert_eq!(action_for_name("Toggle FPS"), Some(Action::ToggleFps));
        assert_eq!(action_for_name("toggle_fps"), Some(Action::ToggleFps));
        // The held stats HUD is NOT a palette command — it is a momentary HOLD-to-peek, so
        // a discrete selection (with no key-release to dismiss it) would leave it stuck on.
        // It is summoned ONLY by the held Cmd-I key (`keymap.rs`), never the catalog.
        assert_eq!(action_for_name("Stats HUD"), None);
        assert_eq!(action_for_name("stats_hud"), None);
        assert_eq!(action_for_name("nope"), None);
    }

    #[test]
    fn effective_bindings_reflect_overrides() {
        // No config: effective == default labels.
        assert_eq!(effective_bindings(&[]), bindings());
        // An override for "switch_theme" surfaces in the palette column. Slot 1 (the
        // NATIVE slot) renders as mac modifier GLYPHS, so `C-t` shows as `⌃T`.
        let keys = vec![("switch_theme".to_string(), vec!["C-t".to_string()])];
        let eff = effective_bindings(&keys);
        let i = COMMANDS.iter().position(|c| c.name == "Switch theme").unwrap();
        assert_eq!(eff[i], "⌃T");
        // A BAD chord falls back to the default label (consistent with the keymap).
        let bad = vec![("switch_theme".to_string(), vec!["C-frobnicate".to_string()])];
        let eff = effective_bindings(&bad);
        assert_eq!(eff[i], "C-x t");
    }

    #[test]
    fn effective_bindings_show_both_slots() {
        // Save fills BOTH slots: the NATIVE slot 1 renders mac GLYPHS (`Cmd-S` → `⌘S`),
        // the EMACS slot 2 keeps its terse text — joined by `·`.
        let i = COMMANDS.iter().position(|c| c.name == "Save").unwrap();
        assert_eq!(bindings()[i], "⌘S · C-x C-s");
        // A single-slot NATIVE command shows just its glyph form (no separator).
        let z = COMMANDS.iter().position(|c| c.name == "Zoom in").unwrap();
        assert_eq!(bindings()[z], "⌘=");
        // A single-slot EMACS command keeps its terse text.
        let g = COMMANDS.iter().position(|c| c.name == "Go to file").unwrap();
        assert_eq!(bindings()[g], "C-x C-f");
        // Settings has no slots → empty label.
        let s = COMMANDS.iter().position(|c| c.name == "Settings").unwrap();
        assert_eq!(bindings()[s], "");
        // A 2-chord config override surfaces BOTH chords, joined — slot 1 glyphified.
        let keys = vec![("save".to_string(), vec!["Cmd-S".to_string(), "C-x C-s".to_string()])];
        assert_eq!(effective_bindings(&keys)[i], "⌘S · C-x C-s");
        // Only the VALID chords of an override are shown; an invalid one is dropped.
        let mixed = vec![("save".to_string(), vec!["Cmd-S".to_string(), "C-frobnicate".to_string()])];
        assert_eq!(effective_bindings(&mixed)[i], "⌘S");
    }

    #[test]
    fn settings_command_present() {
        assert!(COMMANDS.iter().any(|c| c.action == Action::OpenSettings));
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
    fn catalog_excludes_motions_and_insert() {
        for c in COMMANDS {
            assert!(!c.action.is_motion(), "{} is a motion; excluded", c.name);
            assert!(
                !matches!(c.action, Action::InsertChar(_)),
                "{} self-inserts; excluded",
                c.name
            );
        }
    }
}
