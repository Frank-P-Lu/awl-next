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
/// PINNED to `Convention::Mac` (mirroring `keymap.rs`'s own
/// `new_with_convention` precedent for convention-specific assertions) rather
/// than `KeymapState::new()`'s ambient `Convention::current()`: on the wasm
/// target that ambient reads `Convention::Linux` (the "UA never detected"
/// default — no real `wasm_start` ever ran to call
/// `set_web_convention_from_ua`) — the linux-native-keymap round's OWN
/// documented behavior swaps slot 1's Cmd-F to Ctrl-F for Search Forward under
/// `Linux`, which structurally displaces the emacs `C-f` = `ForwardChar`
/// default (native wins on collision). Pinning `Mac` here keeps this test's
/// contract — "the parser + resolver plumbing wires a chord to an Action in
/// the real wasm runtime" — independent of that convention-dependent collision,
/// exactly like `ctrl_motions` independently exercises the Mac-convention
/// reading (its own ambient default on the native dev machine).
#[wasm_bindgen_test]
fn keymap_resolves_a_chord() {
    let mut km = KeymapState::new_with_convention(crate::convention::Convention::Mac);
    let (key, mods) = crate::keyspec::parse_chord("C-f").expect("C-f parses");
    assert_eq!(km.resolve(&key, &mods), Action::ForwardChar, "C-f is ForwardChar under Mac convention");
}

/// THE LINUX-NATIVE KEYMAP: `convention::classify_ua` — the pure UA/platform-string
/// classifier `wasm_start` feeds `navigator.userAgent` into — runs identically in the
/// actual wasm32 runtime (mirrors `convention::tests::classify_ua_*`; this is the
/// wasm-target half of "testable on every target", since the native tests alone don't
/// prove the classifier compiles/behaves under wasm32 too).
#[wasm_bindgen_test]
fn classify_ua_reads_mac_and_defaults_others_to_linux_on_real_wasm() {
    use crate::convention::{classify_ua, Convention};
    assert_eq!(classify_ua("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)"), Convention::Mac);
    assert_eq!(classify_ua("Mozilla/5.0 (X11; Linux x86_64)"), Convention::Linux);
    assert_eq!(classify_ua("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"), Convention::Linux);
    assert_eq!(classify_ua(""), Convention::Linux, "an unrecognized/empty UA defaults to Ctrl");
}

/// `convention::set_web_convention_from_ua` — the ONLY writer of the web-detected
/// convention global — stores + reads back through the real `Convention::current()`
/// wasm-target path (the half `classify_ua` alone doesn't prove: that the STORE
/// actually reaches the resolver every dispatch/label surface calls).
#[wasm_bindgen_test]
fn set_web_convention_from_ua_drives_convention_current_on_real_wasm() {
    use crate::convention::{set_web_convention_from_ua, Convention};
    assert_eq!(set_web_convention_from_ua("Macintosh"), Convention::Mac);
    assert_eq!(Convention::current(), Convention::Mac);
    assert_eq!(set_web_convention_from_ua("X11; Linux x86_64"), Convention::Linux);
    assert_eq!(Convention::current(), Convention::Linux);
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

// ── WEB CHORD SANITY: label truth on the REAL compiled wasm binary ──────────────
//
// The native suite (`commands::tests`) proves the pure, explicit-`Platform`-
// parameterized doors; these two prove the actual OUTCOME on `Platform::current()`
// as `cfg!(target_arch = "wasm32")` really resolves it here, composed with a REAL
// UA-driven `Convention::current()` — the same "only reachable here" gap
// `visible_commands_exclude_the_hide_list_on_real_wasm` closes for the platform
// filter.

/// TIER 2 on the real wasm binary: with the UA-detected convention set to Mac,
/// "New note"'s native Cmd-N chord — a browser-reserved accelerator — never
/// appears in its EFFECTIVE binding label (the palette/rebind-menu door), and
/// "Save"'s ordinary Cmd-S chord is untouched.
#[wasm_bindgen_test]
fn web_reserved_native_chord_is_hidden_from_the_real_palette_label() {
    use crate::convention::set_web_convention_from_ua;
    assert_eq!(crate::commands::Platform::current(), crate::commands::Platform::Web);
    set_web_convention_from_ua("Macintosh");
    let binds = crate::commands::visible_effective_bindings(&[], &[]);
    let names = crate::commands::visible_names();
    let new_note = names.iter().position(|n| n == "New note").unwrap();
    assert_eq!(binds[new_note], "", "New note's Cmd-N must not appear on the web");
    let save = names.iter().position(|n| n == "Save").unwrap();
    assert_eq!(binds[save], "⌘S", "an ordinary chord is untouched");
    set_web_convention_from_ua(""); // leave the global in its default state
}

/// TIER 3 on the real wasm binary: with the UA-detected convention set to
/// Linux, "Search forward"'s emacs default (`C-s`) is displaced by its OWN
/// native Ctrl-S meaning (Save) and must not appear in the effective label —
/// independent of the Tier-2 platform check, proving the two tiers compose on
/// the real target.
#[wasm_bindgen_test]
fn linux_displaced_emacs_default_is_hidden_from_the_real_palette_label() {
    use crate::convention::set_web_convention_from_ua;
    set_web_convention_from_ua("X11; Linux x86_64");
    let binds = crate::commands::visible_effective_bindings(&[], &[]);
    let names = crate::commands::visible_names();
    let search = names.iter().position(|n| n == "Search forward").unwrap();
    assert_eq!(binds[search], "Ctrl+F", "the displaced C-s default must not appear");
    set_web_convention_from_ua(""); // leave the global in its default state
}

// ── WEB ESCAPE HATCHES: "Download file" on the REAL compiled wasm binary ────────
//
// The actual DOM handoff (`web_export::trigger_download`) needs a real `window`/
// `document`, which the default NODE runner (see the module doc) doesn't provide —
// that half is confirmed only by `cargo build --target wasm32-unknown-unknown`
// (L1) compiling it at all, plus live/Playwright confirmation. What IS reachable
// here, exactly like `visible_commands_exclude_the_hide_list_on_real_wasm` /
// `quit_action_is_a_no_op_through_apply_core_on_real_wasm` above: the pure
// filename derivation, the command's PRESENCE in the real wasm-filtered catalog
// (the mirror image of the native-only hide list), and the dispatch gate
// signaling the real `Effect::DownloadFile` through `apply_core`.

/// `web_export::filename_for` — pure, no DOM — derives the same name a save
/// would, in the real wasm runtime (mirrors `web_export::tests` natively).
#[wasm_bindgen_test]
fn download_filename_derivation_on_real_wasm() {
    let mut b = crate::buffer::Buffer::from_str("hello");
    b.set_path(std::path::PathBuf::from("/tmp/notes.md"));
    assert_eq!(crate::web_export::filename_for(&b), "notes.md");

    let scratch = crate::buffer::Buffer::from_str("");
    assert_eq!(crate::web_export::filename_for(&scratch), "scratch.md");
}

/// "Download file" — the inverse of the native-only hide list — is PRESENT in
/// the real wasm-filtered catalog (mirrors `visible_commands_exclude_the_hide_
/// list_on_real_wasm`'s shape, in the other direction).
#[wasm_bindgen_test]
fn download_file_command_is_visible_on_real_wasm() {
    assert_eq!(crate::commands::Platform::current(), crate::commands::Platform::Web);
    let names: Vec<&str> = crate::commands::visible().iter().map(|c| c.name).collect();
    assert!(names.contains(&"Download file"), "Download file must be visible on web: {names:?}");
}

/// `Action::DownloadFile` signals the real `Effect::DownloadFile` through
/// `apply_core` in the compiled wasm binary, touching nothing in the buffer
/// (the pure core has no DOM handoff seam — see `actions.rs`'s doc on the
/// variant) — the reachable half of the dispatch gate this smoke tier can prove
/// without a real `window`/`document`.
#[wasm_bindgen_test]
fn download_file_action_signals_the_effect_through_apply_core_on_real_wasm() {
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
    let effect = crate::actions::apply_core(&mut ctx, &Action::DownloadFile, false);
    assert_eq!(effect, crate::actions::Effect::DownloadFile, "DownloadFile must signal its effect on web");
    assert_eq!(buffer.text(), "hello", "the buffer must be completely untouched");
    assert_eq!(buffer.version(), version_before, "no edit must have been recorded");
}
