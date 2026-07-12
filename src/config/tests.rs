//! src/config/tests.rs — the config unit-test suite, moved verbatim out of
//! the former `config.rs` monolith (2026-07 code-organization pass); every
//! test's NAME and MODULE PATH are unchanged (`config::tests::foo`) — only
//! which file its source lives in moved.

use super::model::expand_tilde;
use super::*;
use crate::fs::FileSystem; // bring the trait methods (read_to_string, …) into scope
use std::path::PathBuf;


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
    let _n = crate::testlock::serial();
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
    let _s = crate::testlock::serial();
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
    let _o = crate::testlock::serial();
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
fn load_reads_reduce_motion_pref_absent_means_auto() {
    // ACCESSIBILITY TIER 1: `reduce_motion` round-trips through REAL TOML
    // parsing exactly like the other sticky bool prefs, but — UNLIKE them —
    // absence means `auto` (resolved from OS/browser detection at live
    // startup, see `motion::resolve`), never a fixed built-in default; there
    // is deliberately no `reduce_motion_on()` accessor on `Config` for that
    // reason (the resolution needs an `os_reduced` input `Config` can't supply).
    use std::sync::Arc;
    let p = PathBuf::from("/cfg/config.toml");
    let fs = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "reduce_motion = true\n"));
    crate::fs::with_fs(fs, || {
        let cfg = Config::load(p.clone());
        assert_eq!(cfg.reduce_motion, Some(true));
    });
    let fs2 = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "reduce_motion = false\n"));
    crate::fs::with_fs(fs2, || {
        let cfg = Config::load(p.clone());
        assert_eq!(cfg.reduce_motion, Some(false));
    });
    let fs3 = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "theme = \"Tawny\"\n"));
    crate::fs::with_fs(fs3, || {
        let cfg = Config::load(p.clone());
        assert_eq!(cfg.reduce_motion, None, "absent = auto, not a fixed default");
    });
    assert_eq!(Config::empty().reduce_motion, None);
}

/// `reduce_motion` is deliberately ABSENT from `apply_sticky_globals` — see
/// `motion.rs`'s determinism note: it is resolved ONLY by
/// `motion::apply_at_startup`, called ONLY from the live `App::new`, so a
/// `--config` naming `reduce_motion` can never affect a headless capture (which
/// calls `apply_sticky_globals` but never constructs an `App`). This asserts
/// the negative directly: running `apply_sticky_globals` must NOT flip the
/// live `motion::reduced()` global either direction.
#[test]
fn apply_sticky_globals_never_touches_reduce_motion() {
    let _g = crate::testlock::serial();
    let saved = crate::motion::reduced();
    crate::motion::set_reduced(false);
    let cfg = Config { reduce_motion: Some(true), ..Config::empty() };
    cfg.apply_sticky_globals(false, false, false, false, crate::page::PageClass::Prose);
    assert!(
        !crate::motion::reduced(),
        "apply_sticky_globals must never read/apply reduce_motion"
    );
    crate::motion::set_reduced(saved);
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
    let _w = crate::testlock::serial();
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
    let _g = crate::testlock::serial();
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
    let _g = crate::testlock::serial();
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
    let _t = crate::testlock::serial();
    let _p = crate::testlock::serial();
    let _c = crate::testlock::serial();
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

#[test]
fn load_reads_linux_keep_emacs_as_a_toml_array() {
    // THE EMACS-HANDS-ON-LINUX ROUND: `linux_keep_emacs` is a plain TOML array of
    // chord strings, round-tripping through `Config::load` untouched (chord-shape
    // validity is checked later, at `KeymapState::apply_linux_keep` — never here).
    use std::sync::Arc;
    let p = PathBuf::from("/cfg/config.toml");
    let fs = Arc::new(crate::fs::InMemoryFs::new().with_file(
        &p,
        "linux_keep_emacs = [\"C-f\", \"C-b\", \"C-n\", \"C-p\", \"C-a\", \"C-e\"]\n",
    ));
    crate::fs::with_fs(fs, || {
        let cfg = Config::load(p.clone());
        assert_eq!(
            cfg.linux_keep_emacs,
            vec!["C-f", "C-b", "C-n", "C-p", "C-a", "C-e"]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>()
        );
    });
}

#[test]
fn absent_linux_keep_emacs_is_empty() {
    // No config at all, and a config that mentions everything BUT this key, both
    // leave the list empty — the built-in default (today's Linux-native behavior,
    // byte-identical).
    assert!(Config::empty().linux_keep_emacs.is_empty());
    use std::sync::Arc;
    let p = PathBuf::from("/cfg/config.toml");
    let fs = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "notes_root = \"/tmp/notes\"\n"));
    crate::fs::with_fs(fs, || {
        assert!(Config::load(p.clone()).linux_keep_emacs.is_empty());
    });
}

#[test]
fn linux_keep_emacs_lenient_load_skips_non_string_entries() {
    // A non-string array entry is skipped (lenient, matching the rest of this
    // loader) rather than aborting the whole array.
    use std::sync::Arc;
    let p = PathBuf::from("/cfg/config.toml");
    let fs = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "linux_keep_emacs = [\"C-f\", 5, \"C-n\"]\n"));
    crate::fs::with_fs(fs, || {
        let cfg = Config::load(p.clone());
        assert_eq!(cfg.linux_keep_emacs, vec!["C-f".to_string(), "C-n".to_string()]);
    });
}

#[test]
fn linux_keep_emacs_wrong_type_is_ignored_not_a_crash() {
    // A `linux_keep_emacs` that isn't even an array (e.g. a bare string) is
    // simply ignored — degrades to the empty default, never a parse error for
    // the WHOLE file (mirrors every other lenient field here).
    use std::sync::Arc;
    let p = PathBuf::from("/cfg/config.toml");
    let fs = Arc::new(crate::fs::InMemoryFs::new().with_file(
        &p,
        "linux_keep_emacs = \"C-f\"\nnotes_root = \"/tmp/notes\"\n",
    ));
    crate::fs::with_fs(fs, || {
        let cfg = Config::load(p.clone());
        assert!(cfg.linux_keep_emacs.is_empty());
        // The rest of the file still loads fine — one bad key never poisons others.
        assert_eq!(cfg.notes_root, Some(PathBuf::from("/tmp/notes")));
    });
}

// ── THE KEYMAP FLAVOR ROUND ─────────────────────────────────────────────────

#[test]
fn keymap_flavor_absent_defaults_to_native() {
    assert_eq!(Config::empty().keymap_flavor(), crate::keymap::KeymapFlavor::Native);
    use std::sync::Arc;
    let p = PathBuf::from("/cfg/config.toml");
    let fs = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "notes_root = \"/tmp/notes\"\n"));
    crate::fs::with_fs(fs, || {
        assert_eq!(Config::load(p).keymap_flavor(), crate::keymap::KeymapFlavor::Native);
    });
}

#[test]
fn keymap_flavor_parses_native_and_emacs() {
    use std::sync::Arc;
    let p = PathBuf::from("/cfg/config.toml");
    let fs = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "keymap = \"emacs\"\n"));
    crate::fs::with_fs(fs.clone(), || {
        assert_eq!(Config::load(p.clone()).keymap_flavor(), crate::keymap::KeymapFlavor::Emacs);
    });
    let fs2 = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "keymap = \"native\"\n"));
    crate::fs::with_fs(fs2, || {
        assert_eq!(Config::load(p.clone()).keymap_flavor(), crate::keymap::KeymapFlavor::Native);
    });
    // Case-insensitive, like `parse_caret_mode`.
    let fs3 = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "keymap = \"EMACS\"\n"));
    crate::fs::with_fs(fs3, || {
        assert_eq!(Config::load(p).keymap_flavor(), crate::keymap::KeymapFlavor::Emacs);
    });
    let _ = fs;
}

#[test]
fn keymap_flavor_garbage_value_falls_back_to_native_never_a_crash() {
    use std::sync::Arc;
    let p = PathBuf::from("/cfg/config.toml");
    let fs = Arc::new(crate::fs::InMemoryFs::new().with_file(&p, "keymap = \"vim\"\n"));
    crate::fs::with_fs(fs, || {
        // The raw string is still stored (`Config::keymap`), but the lenient
        // accessor reads an unrecognized value exactly like absent.
        let cfg = Config::load(p);
        assert_eq!(cfg.keymap.as_deref(), Some("vim"));
        assert_eq!(cfg.keymap_flavor(), crate::keymap::KeymapFlavor::Native);
    });
}

/// THE INSERT-LINK-YIELDS-TO-KILL-LINE ROUND: `effective_linux_keep` is NEVER
/// truly empty anymore — even a totally absent/default config carries the
/// built-in floor (`keymap::linux_builtin_keep()`, currently just `"C-k"`), on
/// EITHER keymap flavor. Supersedes the old "absent config = empty keep list"
/// assumption the pre-floor tests below this one were written against.
#[test]
fn effective_linux_keep_absent_config_is_the_builtin_floor_not_empty() {
    let eff = Config::empty().effective_linux_keep();
    assert_eq!(eff.len(), crate::keymap::linux_builtin_keep().len());
    for b in crate::keymap::linux_builtin_keep() {
        assert!(eff.iter().any(|e| e == b), "built-in floor chord {b:?} missing from effective_linux_keep");
    }
}

#[test]
fn effective_linux_keep_under_native_is_the_builtin_floor_plus_the_raw_list() {
    let mut cfg = Config::empty();
    cfg.linux_keep_emacs = vec!["C-f".to_string()];
    let eff = cfg.effective_linux_keep();
    assert!(eff.contains(&"C-f".to_string()));
    for b in crate::keymap::linux_builtin_keep() {
        assert!(eff.iter().any(|e| e == b), "built-in floor chord {b:?} missing from effective_linux_keep");
    }
    // The floor plus exactly the one raw entry — no preset (native flavor).
    assert_eq!(eff.len(), crate::keymap::linux_builtin_keep().len() + 1);
}

#[test]
fn effective_linux_keep_under_emacs_widens_to_the_whole_displaced_preset() {
    let mut cfg = Config::empty();
    cfg.keymap = Some("emacs".to_string());
    let eff = cfg.effective_linux_keep();
    // Every letter `LINUX_DISPLACED_LETTERS` names is present as a plain "C-<letter>"
    // chord — the whole-catalog preset, derived from the SAME table the dispatch
    // collision uses (never hand-copied) — PLUS the built-in floor, which the
    // preset itself deliberately never names (`C-k` is unconditional, not
    // flavor-gated; see `linux_builtin_keep()`'s own doc).
    for letter in crate::keymap::linux_emacs_preset_keep() {
        assert!(eff.contains(&letter), "preset chord {letter:?} missing from effective_linux_keep");
    }
    for b in crate::keymap::linux_builtin_keep() {
        assert!(eff.iter().any(|e| e == b), "built-in floor chord {b:?} missing from effective_linux_keep");
    }
    assert_eq!(eff.len(), crate::keymap::linux_emacs_preset_keep().len() + crate::keymap::linux_builtin_keep().len());
}

#[test]
fn effective_linux_keep_under_emacs_unions_with_an_explicit_extra_keep() {
    // An explicit `linux_keep_emacs` entry OUTSIDE the preset (e.g. a chord that
    // isn't in `LINUX_DISPLACED_LETTERS` at all) is unioned in, not dropped.
    let mut cfg = Config::empty();
    cfg.keymap = Some("emacs".to_string());
    cfg.linux_keep_emacs = vec!["C-y".to_string()];
    let eff = cfg.effective_linux_keep();
    assert!(eff.contains(&"C-y".to_string()));
    assert_eq!(
        eff.len(),
        crate::keymap::linux_emacs_preset_keep().len() + crate::keymap::linux_builtin_keep().len() + 1
    );
}

#[test]
fn effective_linux_keep_under_emacs_a_duplicate_explicit_entry_does_not_double_count() {
    // An explicit entry that's ALREADY in the preset (any equivalent spelling)
    // contributes nothing extra — canonical-compare via `linux_keeps_chord`.
    let mut cfg = Config::empty();
    cfg.keymap = Some("emacs".to_string());
    cfg.linux_keep_emacs = vec!["Ctrl-f".to_string()]; // == "C-f", already in the preset
    let eff = cfg.effective_linux_keep();
    assert_eq!(eff.len(), crate::keymap::linux_emacs_preset_keep().len() + crate::keymap::linux_builtin_keep().len());
}

/// The same "a duplicate contributes nothing extra" law, but for an explicit
/// `linux_keep_emacs` entry that collides with the BUILT-IN floor rather than
/// the emacs preset — e.g. a user who (redundantly) names `C-k` themselves.
#[test]
fn effective_linux_keep_a_duplicate_of_the_builtin_floor_does_not_double_count() {
    let mut cfg = Config::empty();
    cfg.linux_keep_emacs = vec!["Ctrl-k".to_string()]; // == "C-k", already the built-in floor
    let eff = cfg.effective_linux_keep();
    assert_eq!(eff.len(), crate::keymap::linux_builtin_keep().len());
}
