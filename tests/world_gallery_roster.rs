//! tests/world_gallery_roster.rs — item 68's ROSTER-ADDITION LAW, proven on the
//! REAL `awl` binary (`CARGO_BIN_EXE_awl`, the same subprocess mechanism as
//! `tests/hermetic_canary.rs`), not an in-process approximation.
//!
//! `scripts/capture-worlds.sh` enumerates the world roster through exactly one
//! CLI door, `awl --list-worlds` — itself a thin printer over the one
//! code-owned source, `theme::world_names()` (`src/theme/worlds.rs`), which
//! `--help`'s theme line and the unknown-`--theme` error also read. This test
//! is the EXPECTED-ROSTER SNAPSHOT: a hard-coded 18-name list that fails
//! LOUDLY the moment `theme::THEMES` gains, loses, or reorders a world, until
//! a human consciously updates this list (and, by extension, remembers the
//! gallery command that depends on it). That duplication is deliberate — the
//! same shape as `theme::tests::worlds_eleven_dark_seven_light`'s hard-coded
//! `18` — never read `theme::THEMES` directly from here; the whole point is a
//! snapshot independent of the roster's own source.
use std::process::Command;

/// The roster as of item 68, in `theme::THEMES` cycle order. Update this list
/// (and `scripts/capture-worlds.sh`'s expectations, and CAPTURE.md if the
/// count changes) the moment this test fails — that failure IS the law.
const EXPECTED_WORLDS: [&str; 18] = [
    "Tawny",
    "Mopoke",
    "Currawong",
    "Potoroo",
    "Gumtree",
    "Bilby",
    "Saltpan",
    "Quokka",
    "Bombora",
    "Bowerbird",
    "Mulga",
    "Mangrove",
    "Galah",
    "Magpie",
    "Brolga",
    "Wagtail",
    "Firetail",
    "Cassowary",
];

fn awl_bin() -> &'static str {
    env!("CARGO_BIN_EXE_awl")
}

#[test]
fn list_worlds_matches_the_expected_roster_exactly() {
    let out = Command::new(awl_bin())
        .arg("--list-worlds")
        .output()
        .expect("failed to spawn the awl binary under CARGO_BIN_EXE_awl");
    assert!(out.status.success(), "--list-worlds should exit 0");
    let stdout = String::from_utf8(out.stdout).expect("--list-worlds stdout is UTF-8");
    let names: Vec<&str> = stdout.lines().collect();

    // Missing / newly-un-enrolled / reordered world: fails LOUDLY.
    assert_eq!(
        names, EXPECTED_WORLDS,
        "theme::THEMES roster changed — update EXPECTED_WORLDS here AND \
         scripts/capture-worlds.sh's expectations (item 68)"
    );

    // Duplicate world: fails LOUDLY (redundant with the exact-match above,
    // asserted independently so a future EXPECTED_WORLDS edit that
    // accidentally introduces a dup still gets caught).
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "--list-worlds printed a duplicate world name");
}

#[test]
fn help_text_names_every_world_and_advertises_list_worlds() {
    let out = Command::new(awl_bin())
        .arg("--help")
        .output()
        .expect("failed to spawn the awl binary under CARGO_BIN_EXE_awl");
    assert!(out.status.success(), "--help should exit 0");
    let stdout = String::from_utf8(out.stdout).expect("--help stdout is UTF-8");

    // The historical bug (item 68): --help advertised only ten of eighteen
    // worlds because its theme line was a separately hand-copied list. Every
    // name in the roster must appear in the help text now that both routes
    // read the same `theme::world_names()`.
    for name in EXPECTED_WORLDS {
        assert!(stdout.contains(name), "--help is missing world {name:?} (roster drift)");
    }
    assert!(stdout.contains("--list-worlds"), "--help should advertise --list-worlds");
}

#[test]
fn unknown_theme_error_names_every_world() {
    let out = Command::new(awl_bin())
        .args(["--theme", "NoSuchWorldXYZ", "--screenshot"])
        .arg(std::env::temp_dir().join("awl-world-gallery-roster-law.png"))
        .output()
        .expect("failed to spawn the awl binary under CARGO_BIN_EXE_awl");
    assert!(!out.status.success(), "an unknown --theme should fail");
    let stderr = String::from_utf8(out.stderr).expect("stderr is UTF-8");
    for name in EXPECTED_WORLDS {
        assert!(stderr.contains(name), "unknown-theme error is missing world {name:?} (roster drift)");
    }
}
