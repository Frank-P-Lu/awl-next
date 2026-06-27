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
    Command { name: "Save",              action: Action::Save,            binding: "C-x C-s" },
    Command { name: "Quit",              action: Action::Quit,            binding: "C-x C-c" },
    Command { name: "Search forward",    action: Action::SearchForward,   binding: "C-s"     },
    Command { name: "Search backward",   action: Action::SearchBackward,  binding: "C-r"     },
    Command { name: "Undo",              action: Action::Undo,            binding: "C-/"     },
    Command { name: "Redo",              action: Action::Redo,            binding: "Cmd-S-z" },
    Command { name: "Zoom in",           action: Action::ZoomIn,          binding: "Cmd-="   },
    Command { name: "Zoom out",          action: Action::ZoomOut,         binding: "Cmd--"   },
    Command { name: "Reset zoom",        action: Action::ZoomReset,       binding: "Cmd-0"   },
];

/// The catalog command NAMES, in catalog order — the fuzzy corpus the palette
/// overlay filters over.
pub fn names() -> Vec<String> {
    COMMANDS.iter().map(|c| c.name.to_string()).collect()
}

/// The catalog BINDING labels, parallel to [`names`] — shown dim beside each row.
pub fn bindings() -> Vec<String> {
    COMMANDS.iter().map(|c| c.binding.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_non_empty_and_every_entry_has_a_binding() {
        assert!(!COMMANDS.is_empty(), "the command catalog must list commands");
        for c in COMMANDS {
            assert!(!c.name.trim().is_empty(), "command needs a display name");
            assert!(!c.binding.trim().is_empty(), "command {} needs a binding label", c.name);
        }
        // names()/bindings() stay parallel to the catalog.
        assert_eq!(names().len(), COMMANDS.len());
        assert_eq!(bindings().len(), COMMANDS.len());
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
