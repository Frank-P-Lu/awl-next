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
