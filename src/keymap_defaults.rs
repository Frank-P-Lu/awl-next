//! THE KEYMAP-DEFAULTS-AS-DATA ROUND: parses the embedded
//! `assets/keymap-defaults.toml` ONCE, at first access, into the two
//! lookups every other module reads — [`command_defaults`] (slug -> exactly
//! two chord slots, `(native, emacs)`) and [`linux_builtin_keep`] (the
//! unconditional Linux keep-floor chords, formerly the hand-written
//! `keymap::LINUX_BUILTIN_KEEP` const). This is now THE single place a
//! default keyboard chord lives — `commands::COMMANDS` (its `native`/
//! `emacs` slots) and `keymap::linux_builtin_keep` both build FROM this
//! module rather than carrying their own literal strings. See
//! `assets/keymap-defaults.toml`'s own header for the file's grammar, and
//! CLAUDE.md's "KEYMAP DEFAULTS AS DATA" round for the full design.
//!
//! **What deliberately did NOT move here** (policy, not assignment — see
//! CLAUDE.md): the plain-arrow motions (uncataloged, dispatched by
//! `keymap::resolve_named`'s static arms regardless of any catalog/data
//! entry), the Cmd-to-Ctrl convention TRANSLATION rule and its override
//! table (`commands::LINUX_NATIVE_OVERRIDE`), the native-wins Linux
//! collision table (`keymap::LINUX_DISPLACED_LETTERS`), the prefix-sequence
//! dispatch machinery (`keymap::resolve`/`resolve_named`/`resolve_char`),
//! `webreserved`, and `commands::WEB_ALTERNATE`.
//! Those are per-platform POLICY (how a chord VALUE resolves/collides/
//! dispatches), not the chord VALUES themselves — this module only ever
//! answers "what IS the default chord for command X", never "how does a
//! keypress become an `Action`".
//!
//! **Parse-error policy, the opposite of `config::Config::load`'s leniency,
//! deliberately:** this file ships INSIDE the binary and is never user-
//! edited, so a malformed embedded file is this codebase's own bug, not a
//! user mistake — it panics loudly at first access (which happens at or
//! before the first test/screenshot/keypress touches `commands::COMMANDS`)
//! rather than silently degrading to an empty keymap. `config.toml` stays
//! lenient because a USER'S typo must never break launch; this file has no
//! user in the loop to make a typo.

use std::collections::HashMap;
use std::sync::LazyLock;

const RAW: &str = include_str!("../assets/keymap-defaults.toml");

struct Parsed {
    /// slug -> (native, emacs); a slot is `""` when the command has no
    /// default chord in that slot. The catalog seam requires exactly one row
    /// for every command, including commands with two empty slots.
    commands: HashMap<String, (String, String)>,
    /// The unconditional Linux keep floor (currently just `"C-k"`) — see the
    /// TOML file's own header.
    linux_builtin_keep: Vec<&'static str>,
}

static PARSED: LazyLock<Parsed> = LazyLock::new(|| parse(RAW));

fn parse(raw: &str) -> Parsed {
    let table: toml::Table = raw
        .parse()
        .unwrap_or_else(|e| panic!("assets/keymap-defaults.toml failed to parse (embedded, build-time bug): {e}"));

    for key in table.keys() {
        assert!(
            matches!(key.as_str(), "commands" | "linux_builtin_keep"),
            "assets/keymap-defaults.toml: unknown top-level key {key:?}"
        );
    }

    let mut commands = HashMap::new();
    let cmds = table
        .get("commands")
        .and_then(|v| v.as_table())
        .unwrap_or_else(|| panic!("assets/keymap-defaults.toml: missing [commands] table"));
    for (slug, val) in cmds {
        let arr = val.as_array().unwrap_or_else(|| {
            panic!("assets/keymap-defaults.toml: commands.{slug} must be a 2-element array")
        });
        assert_eq!(
            arr.len(), 2,
            "assets/keymap-defaults.toml: commands.{slug} must have exactly 2 chord slots"
        );
        let native = arr[0]
            .as_str()
            .unwrap_or_else(|| {
                panic!("assets/keymap-defaults.toml: commands.{slug}[0] must be a string")
            })
            .to_string();
        let emacs = arr[1]
            .as_str()
            .unwrap_or_else(|| {
                panic!("assets/keymap-defaults.toml: commands.{slug}[1] must be a string")
            })
            .to_string();
        commands.insert(slug.clone(), (native, emacs));
    }

    let linux_builtin_keep: Vec<&'static str> = table
        .get("linux_builtin_keep")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|v| {
                    v.as_str().unwrap_or_else(|| {
                        panic!(
                            "assets/keymap-defaults.toml: linux_builtin_keep entries must be strings"
                        )
                    })
                })
                .map(|s| -> &'static str { Box::leak(s.to_string().into_boxed_str()) })
                .collect()
        })
        .unwrap_or_else(|| panic!("assets/keymap-defaults.toml: missing linux_builtin_keep array"));

    Parsed { commands, linux_builtin_keep }
}

/// Slug -> `(native, emacs)` default chord slots — the single source
/// `commands::COMMANDS` splices its `native`/`emacs` slots from.
pub fn command_defaults() -> &'static HashMap<String, (String, String)> {
    &PARSED.commands
}

/// The unconditional Linux keep-floor chords (formerly the hand-written
/// `keymap::LINUX_BUILTIN_KEEP` const) — same `&'static [&'static str]`
/// shape as the retired const, so every call site needs only `()` added.
pub(crate) fn linux_builtin_keep() -> &'static [&'static str] {
    &PARSED.linux_builtin_keep
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_file_parses_without_panicking() {
        let d = command_defaults();
        assert!(!d.is_empty(), "the embedded defaults file must name at least one command");
    }

    #[test]
    fn linux_builtin_keep_floor_is_ck() {
        assert_eq!(linux_builtin_keep(), &["C-k"]);
    }

    #[test]
    fn every_slug_is_a_real_catalog_slug() {
        // Cross-checked more thoroughly in `commands::tests`; this is the
        // pure-data half (no catalog dependency) — every key at least LOOKS
        // like a slug (lower-case, ascii, underscores only).
        for slug in command_defaults().keys() {
            assert!(
                slug.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "slug {slug:?} is not lower_snake_case"
            );
        }
    }

    #[test]
    #[should_panic(expected = "exactly 2 chord slots")]
    fn embedded_command_slots_are_exactly_two_strings() {
        let _ = parse("linux_builtin_keep = []\n[commands]\nsave = [\"Cmd-S\"]\n");
    }

    #[test]
    #[should_panic(expected = "must be a string")]
    fn embedded_command_slot_types_fail_loudly() {
        let _ = parse("linux_builtin_keep = []\n[commands]\nsave = [1, \"\"]\n");
    }

    #[test]
    #[should_panic(expected = "unknown top-level key")]
    fn embedded_unknown_top_level_data_fails_loudly() {
        let _ = parse("linux_builtin_keep = []\nextra = true\n[commands]\nsave = [\"Cmd-S\", \"\"]\n");
    }
}
