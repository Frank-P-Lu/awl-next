//! websmoke — the CORE web/wasm smoke suite.
//!
//! A tiny set of `#[wasm_bindgen_test]`s that prove awl's platform-agnostic
//! CORE actually RUNS inside the wasm32 runtime — not just that it compiled to
//! wasm (that is L1, the `cargo build --target wasm32-unknown-unknown` stage of
//! `scripts/web-smoke.sh`). These hit the same public-in-crate APIs the equivalent
//! native `#[test]`s do (`Buffer`, `markdown::spans`, `syntax::spans`,
//! `keymap::KeymapState` via `keyspec::parse_chord`), so a regression that only
//! shows up under the wasm target — a native-only assumption leaking into the
//! core — fails here rather than silently in the browser.
//!
//! Gated `#[cfg(all(test, target_arch = "wasm32"))]`, so it is INERT for the
//! native `cargo test` (it never compiles there) and only ever built for the wasm
//! target. Deliberately NO `wasm_bindgen_test_configure!(run_in_browser)` — these
//! are pure-logic core tests with no DOM/WebGPU surface, so the default NODE
//! runner (`wasm-bindgen-test-runner`, L2 of the smoke script) runs them headless
//! with no browser needed.

use wasm_bindgen_test::*;

use crate::keymap::{Action, KeymapState};
use crate::markdown::{self, MdKind};
use crate::syntax::{self, Lang, SynKind};

/// A `Buffer` constructs, edits, and reads back its text in the wasm runtime —
/// the rope round-trip that every keystroke rides. Mirrors the native buffer
/// edit tests' shape (`from_str` + `insert_char` + `text`).
#[wasm_bindgen_test]
fn buffer_edit_round_trip() {
    let mut buf = crate::buffer::Buffer::from_str("ab");
    // Cursor starts at 0; typing 'X' inserts at the front.
    buf.insert_char('X');
    assert_eq!(buf.text(), "Xab", "insert_char at buffer start");
}

/// `markdown::spans` parses a heading in the wasm runtime, emitting the expected
/// `Heading` content kind (mirrors `markdown::tests::heading_dims_hashes_and_styles_title`).
#[wasm_bindgen_test]
fn markdown_spans_emit_heading() {
    let s = markdown::spans("# Title");
    let has_h1 = s.iter().any(|(_, k)| *k == MdKind::Heading(1));
    assert!(has_h1, "'# Title' should yield an H1 heading span: {s:?}");
}

/// `syntax::spans(Lang::Rust, ..)` emits all FOUR Alabaster roles in the wasm
/// runtime — comment, string, constant, definition (mirrors the per-role
/// `syntax::rust::tests`).
#[wasm_bindgen_test]
fn syntax_spans_emit_the_four_roles() {
    let code = "fn foo() { let s = \"hi\"; let n = 42; } // note\n";
    let s = syntax::spans(Lang::Rust, code);
    let kinds: Vec<SynKind> = s.iter().map(|(_, k)| *k).collect();
    assert!(kinds.contains(&SynKind::Definition), "def `foo`: {s:?}");
    assert!(kinds.contains(&SynKind::Str), "string `\"hi\"`: {s:?}");
    assert!(kinds.contains(&SynKind::Constant), "constant `42`: {s:?}");
    assert!(kinds.contains(&SynKind::Comment), "comment `// note`: {s:?}");
}

/// The real keymap resolves a chord parsed by `keyspec::parse_chord` to the
/// expected `Action` in the wasm runtime (mirrors `keymap::tests::ctrl_motions`).
#[wasm_bindgen_test]
fn keymap_resolves_a_chord() {
    let mut km = KeymapState::new();
    let (key, mods) = crate::keyspec::parse_chord("C-f").expect("C-f parses");
    assert_eq!(km.resolve(&key, &mods), Action::ForwardChar, "C-f is ForwardChar");
}

// ── PLATFORM-SCOPED COMMANDS: the REAL compiled-wasm filter + dispatch gate ──────
//
// These two prove the actual behavior in the actual wasm32 binary — not just the
// pure `Platform::Web`-parameterized doors the native suite already covers
// (`commands::tests`, `menu::tests`), which take an EXPLICIT platform and could in
// principle diverge from what `cfg!(target_arch = "wasm32")` really resolves to on
// this target. Only reachable here.

/// `commands::visible()` — driven by `Platform::current()`'s real `cfg!` read —
/// excludes every hide-listed command in the ACTUAL compiled wasm binary.
#[wasm_bindgen_test]
fn visible_commands_exclude_the_hide_list_on_real_wasm() {
    assert_eq!(crate::commands::Platform::current(), crate::commands::Platform::Web);
    let names: Vec<&str> = crate::commands::visible().iter().map(|c| c.name).collect();
    for hidden in [
        "Quit",
        "Finish file",
        "Version history…",
        "Keep version",
        "Lifetime stats",
        "Clean unused assets…",
        "Recent projects…",
        "Keybindings…",
    ] {
        assert!(!names.contains(&hidden), "{hidden} must not appear in the wasm-visible catalog: {names:?}");
    }
    // A representative always-available command survives.
    assert!(names.contains(&"Save"), "Save must stay visible on web: {names:?}");
}

/// The DISPATCH gate actually no-ops a hidden command's `Action` through the real
/// `apply_core` in the compiled wasm binary: `Action::Quit` — which normally signals
/// `Effect::Quit` (see `actions.rs`'s `Action::Quit` arm) — returns `Effect::None`
/// here instead, and leaves the buffer completely untouched (still just "hello",
/// still at the same version) — a still-configured Cmd-Q chord can reach
/// `apply_core` directly, bypassing the (already-filtered) palette entirely, so this
/// is the belt the palette's brace alone can't prove.
#[wasm_bindgen_test]
fn quit_action_is_a_no_op_through_apply_core_on_real_wasm() {
    let mut buffer = crate::buffer::Buffer::from_str("hello");
    let version_before = buffer.version();
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut overlay = None;
    let mut make_overlay = |_: crate::overlay::OverlayKind| None;
    let mut browse_to = |_: crate::overlay::OverlayKind, _: Option<String>| None;
    let mut ctx = crate::actions::ActionCtx {
        buffer: &mut buffer,
        shift_selecting: &mut shift,
        zoom: &mut zoom,
        search: &mut search,
        scroll_page_lines: 1,
        overlay: &mut overlay,
        make_overlay: &mut make_overlay,
        browse_to: &mut browse_to,
        oracle: None,
    };
    let effect = crate::actions::apply_core(&mut ctx, &Action::Quit, false);
    assert_eq!(effect, crate::actions::Effect::None, "Quit must be a no-op effect on web");
    assert_eq!(buffer.text(), "hello", "the buffer must be completely untouched");
    assert_eq!(buffer.version(), version_before, "no edit must have been recorded");
}
