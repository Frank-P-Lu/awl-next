//! The PERSISTENT CONFIG: awl's settings live in a text file you edit AS TEXT in
//! awl itself (the Settings command opens it; `C-x C-s` saves + live-reloads). The
//! file is TOML at `$XDG_CONFIG_HOME/awl/config.toml` (or `~/.config/awl/...`):
//!
//! ```toml
//! notes_root = "~/notes"
//! workspace  = "~/code"
//! [keys]
//! switch_theme = "C-x t"   # action-name -> chord
//! ```
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
    /// The `[keys]` table as (action-name, chord) pairs, in file order. The keymap
    /// parses each chord and OVERRIDES the default binding for that named action.
    pub keys: Vec<(String, String)>,
    /// Where this config loaded from (the Settings command's open target). Empty
    /// for [`Config::empty`] (a non-file placeholder).
    pub path: PathBuf,
}

/// The commented template written on the FIRST Settings-open when no config
/// exists, so the user lands in a self-documenting file rather than a blank one.
pub const DEFAULT_TEMPLATE: &str = "\
# awl config — edit as text, then C-x C-s to save (live-reloads keys + folders).
#
# notes_root : where C-x n quick-notes live          (default: ~/notes)
# workspace  : the parent dir whose children C-x p switches between
#                                                     (default: the project's parent)
#
# [keys] : rebind a command to a chord. The ACTION NAME is the command-palette
#   name lower-cased with spaces as underscores (go_to_file, switch_theme, save,
#   new_note, ...). A CHORD is an emacs spec: \"C-t\", \"M-g\", or \"C-x g\"
#   (the C-x prefix plus one key). A bad chord is ignored and the default kept.
#   Open Cmd-P to see every command's name + current chord.

# notes_root = \"~/notes\"
# workspace = \"~/code\"

[keys]
# go_to_file = \"C-x C-f\"
# switch_theme = \"C-x t\"
";

impl Config {
    /// A NON-FILE placeholder config (all defaults, empty path). Used by capture
    /// modes that take no `--config` so they share the one `replay_keys` seam.
    pub fn empty() -> Self {
        Config {
            notes_root: None,
            workspace: None,
            keys: Vec::new(),
            path: PathBuf::new(),
        }
    }

    /// Load settings from `path`. A MISSING or unreadable file yields a pure-defaults
    /// config bound to `path` (so Settings can still create it) — never an error,
    /// never a behaviour change. A PARSE error is reported to stderr and likewise
    /// degrades to defaults, so a half-edited config never crashes the editor.
    pub fn load(path: PathBuf) -> Self {
        let mut cfg = Config {
            notes_root: None,
            workspace: None,
            keys: Vec::new(),
            path,
        };
        let src = match std::fs::read_to_string(&cfg.path) {
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
        if let Some(keys) = table.get("keys").and_then(|v| v.as_table()) {
            for (name, val) in keys {
                if let Some(chord) = val.as_str() {
                    cfg.keys.push((name.clone(), chord.to_string()));
                }
            }
        }
        cfg
    }

    /// Write the commented [`DEFAULT_TEMPLATE`] to `path`, creating parent dirs.
    /// Called by Settings-open when the file does not exist yet.
    pub fn write_default(path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, DEFAULT_TEMPLATE)
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn tmp_path(tag: &str) -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let mut p = std::env::temp_dir();
        p.push(format!("awl_cfg_{}_{}_{}.toml", std::process::id(), tag, id));
        p
    }

    #[test]
    fn absent_config_is_all_defaults() {
        let cfg = Config::load(PathBuf::from("/nonexistent/awl/config.toml"));
        assert!(cfg.notes_root.is_none());
        assert!(cfg.workspace.is_none());
        assert!(cfg.keys.is_empty());
    }

    #[test]
    fn load_reads_folders_and_keys() {
        let p = tmp_path("load");
        std::fs::write(
            &p,
            "notes_root = \"/tmp/my-notes\"\nworkspace = \"/tmp/ws\"\n[keys]\nswitch_theme = \"C-t\"\n",
        )
        .unwrap();
        let cfg = Config::load(p.clone());
        assert_eq!(cfg.notes_root, Some(PathBuf::from("/tmp/my-notes")));
        assert_eq!(cfg.workspace, Some(PathBuf::from("/tmp/ws")));
        assert_eq!(
            cfg.keys,
            vec![("switch_theme".to_string(), "C-t".to_string())]
        );
        let _ = std::fs::remove_file(&p);
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
        let p = tmp_path("bad");
        std::fs::write(&p, "this is = = not valid toml [[[").unwrap();
        let cfg = Config::load(p.clone());
        assert!(cfg.notes_root.is_none() && cfg.workspace.is_none() && cfg.keys.is_empty());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn write_default_then_load_roundtrips() {
        let p = tmp_path("default");
        let _ = std::fs::remove_file(&p);
        Config::write_default(&p).unwrap();
        let cfg = Config::load(p.clone());
        // The template's folder lines are COMMENTED, so a fresh default is all-None.
        assert!(cfg.notes_root.is_none() && cfg.workspace.is_none());
        let _ = std::fs::remove_file(&p);
    }
}
