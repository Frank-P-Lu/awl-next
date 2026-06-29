//! The PERSISTENT CONFIG: awl's settings live in a text file you edit AS TEXT in
//! awl itself (the Settings command opens it; `C-x C-s` saves + live-reloads). The
//! file is TOML at `$XDG_CONFIG_HOME/awl/config.toml` (or `~/.config/awl/...`):
//!
//! ```toml
//! notes_root = "~/notes"
//! workspace  = "~/code"
//! [keys]
//! save         = ["Cmd-S", "C-x C-s"]  # up to 2 chords: native + emacs
//! switch_theme = "C-x t"               # a single chord still works
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
# awl config — edit as text, then C-x C-s to save (live-reloads keys + folders).
#
# notes_root : where C-x n quick-notes live          (default: ~/notes)
# workspace  : the parent dir whose children C-x p switches between
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
#   + both effective chords, or Cmd-P -> \"Keybindings\" to rebind by PRESSING the
#   key (it writes this table for you).

# notes_root = \"~/notes\"
# workspace = \"~/code\"

[keys]
# save = [\"Cmd-S\", \"C-x C-s\"]
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
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, DEFAULT_TEMPLATE)
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
            std::fs::create_dir_all(parent)?;
        }
        let src = match std::fs::read_to_string(path) {
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
        std::fs::write(path, out)
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
            vec![("switch_theme".to_string(), vec!["C-t".to_string()])]
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn load_reads_two_binding_list_capped_at_two() {
        // A `[keys]` value may be a LIST of up to 2 chords (slot 1 native, slot 2
        // emacs). A single string still loads as a one-element list (back-compat);
        // a 3+ list is capped at the first two.
        let p = tmp_path("twoslot");
        std::fs::write(
            &p,
            "[keys]\nsave = [\"Cmd-S\", \"C-x C-s\"]\nundo = \"Cmd-Z\"\nredo = [\"a\", \"b\", \"c\"]\n",
        )
        .unwrap();
        let cfg = Config::load(p.clone());
        let get = |k: &str| cfg.keys.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get("save"), Some(vec!["Cmd-S".to_string(), "C-x C-s".to_string()]));
        assert_eq!(get("undo"), Some(vec!["Cmd-Z".to_string()]));
        // Three chords supplied; the model caps at 2.
        assert_eq!(get("redo"), Some(vec!["a".to_string(), "b".to_string()]));
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
    fn tilde_in_folder_path_expands_to_home() {
        // A `~/x` notes_root resolves against the CURRENT $HOME (read-only — no env
        // mutation, so this can't race other tests). Skipped if HOME is unset.
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        // expand_tilde directly...
        assert_eq!(expand_tilde("~/x"), PathBuf::from(&home).join("x"));
        // ...and through the load seam (notes_root + workspace both expand).
        let p = tmp_path("tilde");
        std::fs::write(&p, "notes_root = \"~/n\"\nworkspace = \"~/w\"\n").unwrap();
        let cfg = Config::load(p.clone());
        assert_eq!(cfg.notes_root, Some(PathBuf::from(&home).join("n")));
        assert_eq!(cfg.workspace, Some(PathBuf::from(&home).join("w")));
        let _ = std::fs::remove_file(&p);
        // A non-tilde path passes through verbatim.
        assert_eq!(expand_tilde("/abs/x"), PathBuf::from("/abs/x"));
    }

    // Serialize the env-mutating config_path test (set/remove_var touch the
    // process-global environment). Only this test mutates these three vars, and no
    // other awl test reads them, so one lock is enough.
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
        let p = tmp_path("writebind");
        let _ = std::fs::remove_file(&p);
        // Seed a hand-edited config WITH a comment and a folder line.
        std::fs::write(
            &p,
            "# my notes\nnotes_root = \"/tmp/n\"\n[keys]\nswitch_theme = \"C-t\"\n",
        )
        .unwrap();
        // SET a brand-new entry (inserted under [keys]); the comment + folder survive.
        Config::write_binding(&p, "save", Some(&["Cmd-S".to_string(), "C-x C-s".to_string()])).unwrap();
        let cfg = Config::load(p.clone());
        let get = |k: &str| cfg.keys.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get("save"), Some(vec!["Cmd-S".to_string(), "C-x C-s".to_string()]));
        assert_eq!(get("switch_theme"), Some(vec!["C-t".to_string()]));
        assert_eq!(cfg.notes_root, Some(PathBuf::from("/tmp/n")));
        let raw = std::fs::read_to_string(&p).unwrap();
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
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn write_binding_seeds_missing_file_with_template() {
        let p = tmp_path("writebind_new");
        let _ = std::fs::remove_file(&p);
        // No file yet: the writer seeds the documented template, then adds the entry.
        Config::write_binding(&p, "undo", Some(&["C-j".to_string()])).unwrap();
        let raw = std::fs::read_to_string(&p).unwrap();
        assert!(raw.contains("awl config"), "template seeded: {raw}");
        let cfg = Config::load(p.clone());
        assert_eq!(
            cfg.keys.iter().find(|(n, _)| n == "undo").map(|(_, v)| v.clone()),
            Some(vec!["C-j".to_string()])
        );
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
