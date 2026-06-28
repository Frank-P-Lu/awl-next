//! The COMMAND CATALOG: the named, fuzzy-searchable list of editor commands the
//! Cmd-P palette runs. Each entry is a human DISPLAY NAME, the `Action` it
//! dispatches on Enter, and a human BINDING LABEL (the current default chord),
//! shown dim beside the name so the palette also TEACHES the chord.
//!
//! This is deliberately a flat `static` slice rather than logic baked into the
//! palette UI: the binding is DATA, not a hardcoded string in the renderer, so
//! this catalog is the seam a future NATIVE REBINDING registry slots into (the
//! `binding` field becomes an owned, user-overridable label). The corpus the
//! overlay fuzzy-matches is `names()`, in this exact order, so the selected
//! ROW index maps straight back to `COMMANDS[i].action` (see the palette accept
//! branch in `actions::apply_core`).
//!
//! Motions / `InsertChar` / prefix / ignore are intentionally EXCLUDED: the
//! palette lists command-ish actions a user would summon by name, not cursor
//! motions or self-insertion.

use crate::keymap::Action;

/// One catalog entry: a display `name` (fuzzy-searched), the `action` it runs on
/// Enter, and a `binding` label (the current default chord, shown dim beside the
/// name). `binding` is data so it can later become a rebindable, owned value.
pub struct Command {
    pub name: &'static str,
    pub action: Action,
    pub binding: &'static str,
}

/// The command catalog, in stable display order. The fuzzy corpus is the NAMES
/// in this order, so a selected row index indexes straight back into this slice.
pub static COMMANDS: &[Command] = &[
    Command { name: "Go to file",        action: Action::OpenGoto,        binding: "C-x C-f" },
    Command { name: "Switch project",    action: Action::OpenProject,     binding: "C-x p"   },
    Command { name: "Browse files",      action: Action::OpenBrowse,      binding: "C-x j"   },
    Command { name: "Last file",         action: Action::LastBuffer,      binding: "C-x b"   },
    Command { name: "New note",          action: Action::NewNote,         binding: "C-x n"   },
    Command { name: "Move note",         action: Action::MoveNote,        binding: "C-x m"   },
    Command { name: "Switch theme",      action: Action::OpenThemeMenu,   binding: "C-x t"   },
    Command { name: "Toggle caret mode", action: Action::ToggleCaretMode, binding: "C-x c"   },
    Command { name: "Toggle page mode",  action: Action::TogglePageMode,  binding: "C-x w"   },
    Command { name: "Focus mode",        action: Action::CycleFocusMode,  binding: "C-x d"   },
    Command { name: "Save",              action: Action::Save,            binding: "C-x C-s" },
    Command { name: "Quit",              action: Action::Quit,            binding: "C-x C-c" },
    Command { name: "Search forward",    action: Action::SearchForward,   binding: "C-s"     },
    Command { name: "Search backward",   action: Action::SearchBackward,  binding: "C-r"     },
    Command { name: "Undo",              action: Action::Undo,            binding: "C-/"     },
    Command { name: "Redo",              action: Action::Redo,            binding: "Cmd-S-z" },
    Command { name: "Zoom in",           action: Action::ZoomIn,          binding: "Cmd-="   },
    Command { name: "Zoom out",          action: Action::ZoomOut,         binding: "Cmd--"   },
    Command { name: "Reset zoom",        action: Action::ZoomReset,       binding: "Cmd-0"   },
    // Settings has NO default chord — the palette IS its entry point. It opens the
    // config file (creating the commented default first) for editing as text.
    Command { name: "Settings",          action: Action::OpenSettings,    binding: ""        },
];

/// Slugify a command name to its config ACTION NAME: lower-case with spaces as
/// underscores ("Go to file" -> "go_to_file", "Switch theme" -> "switch_theme").
/// Both the rebinder ([`action_for_name`]) and the palette display
/// ([`effective_bindings`]) key off this, so a `[keys]` entry and the shown chord
/// stay consistent.
fn slug(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace(' ', "_")
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

/// The EFFECTIVE binding label per command, parallel to [`names`]: a config
/// `[keys]` override for that command's action (when present AND a valid chord),
/// else the static default. Drives the palette's binding column, so it shows the
/// chord that ACTUALLY triggers each command. `keys` is the config `[keys]` list.
pub fn effective_bindings(keys: &[(String, String)]) -> Vec<String> {
    COMMANDS
        .iter()
        .map(|c| {
            keys.iter()
                .find(|(name, chord)| {
                    slug(name) == slug(c.name)
                        && action_for_name(name).is_some()
                        && crate::keymap::parse_binding(chord).is_ok()
                })
                .map(|(_, chord)| chord.clone())
                .unwrap_or_else(|| c.binding.to_string())
        })
        .collect()
}

/// The catalog command NAMES, in catalog order — the fuzzy corpus the palette
/// overlay filters over.
pub fn names() -> Vec<String> {
    COMMANDS.iter().map(|c| c.name.to_string()).collect()
}

/// The catalog DEFAULT binding labels, parallel to [`names`]. The live/headless
/// palette uses [`effective_bindings`] (which overlays config rebinds); this stays
/// as the defaults baseline + test surface.
#[allow(dead_code)]
pub fn bindings() -> Vec<String> {
    COMMANDS.iter().map(|c| c.binding.to_string()).collect()
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
        // Every entry HAS a binding label except the bindless Settings (palette-only).
        for c in COMMANDS {
            if c.name != "Settings" {
                assert!(
                    !c.binding.trim().is_empty(),
                    "command {} needs a binding label",
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
        assert_eq!(action_for_name("nope"), None);
    }

    #[test]
    fn effective_bindings_reflect_overrides() {
        // No config: effective == default labels.
        assert_eq!(effective_bindings(&[]), bindings());
        // An override for "switch_theme" surfaces in the palette column.
        let keys = vec![("switch_theme".to_string(), "C-t".to_string())];
        let eff = effective_bindings(&keys);
        let i = COMMANDS.iter().position(|c| c.name == "Switch theme").unwrap();
        assert_eq!(eff[i], "C-t");
        // A BAD chord falls back to the default label (consistent with the keymap).
        let bad = vec![("switch_theme".to_string(), "C-frobnicate".to_string())];
        let eff = effective_bindings(&bad);
        assert_eq!(eff[i], "C-x t");
    }

    #[test]
    fn settings_command_present() {
        assert!(COMMANDS.iter().any(|c| c.action == Action::OpenSettings));
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
