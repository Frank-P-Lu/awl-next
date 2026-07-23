//! item 65 capture-level fold laws: the ONE PIXEL LAW this round's affordance
//! work must never break — collapsing a section and then unfolding it again must
//! restore the capture BYTE-IDENTICALLY (PNG + sidecar) to the pre-collapse frame.
//! Driven through the REAL `Buffer` fold gestures (`toggle_fold_at_cursor`) and the
//! REAL `capture_with` entry point — the same harness a live `--screenshot` uses —
//! so this is a genuine end-to-end proof, not a re-derivation of the pure fold math
//! `fold::tests` / `render::tests::folds` already cover at the purer seams.

use super::super::*;
use super::adapter_available;
use crate::buffer::Buffer;

/// IDENTICAL RESTORATION AFTER UNFOLD: capture (unfolded) -> collapse -> capture
/// (must differ from the first — else this proves nothing) -> unfold -> capture
/// (must be byte-identical to the FIRST, both the PNG and the sidecar JSON). Also
/// exercises the item 65 Outline correlation along the way: the sidecar's
/// `outline.collapsed` names the folded heading's index while collapsed, and is
/// empty again once restored.
#[test]
fn collapse_then_unfold_restores_the_capture_byte_identically() {
    if !adapter_available() {
        eprintln!("skipping collapse_then_unfold_restores_the_capture_byte_identically: no wgpu adapter");
        return;
    }
    let _g = crate::testlock::serial();

    let dir = std::env::temp_dir().join(format!("awl_fold_restore_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let text = "# Alpha\n\nalpha body 1\nalpha body 2\n\n## Beta\n\nbeta body\n\n# Gamma\n\ngamma body\n";
    let mut buf = Buffer::from_str(text);
    // The caret starts at (0,0) — on "# Alpha", the heading `toggle_fold_at_cursor`
    // below folds.

    // --- BASE: nothing folded. ---
    let base_png = dir.join("base.png");
    capture_with(&base_png, &buf, &CaptureOpts::default()).expect("base capture");
    let base_bytes = std::fs::read(&base_png).unwrap();
    let base_json = std::fs::read_to_string(base_png.with_extension("json")).unwrap();
    assert!(
        base_json.contains("\"collapsed\": []"),
        "nothing folded yet: outline.collapsed is empty in the base capture"
    );

    // --- COLLAPSE: fold "# Alpha" (the heading enclosing the caret). ---
    let folded_heading = buf.toggle_fold_at_cursor();
    assert_eq!(folded_heading, Some(0), "Alpha (line 0) is the heading that folded");
    assert!(buf.has_folds(), "the buffer now carries one fold");
    let mid_png = dir.join("mid.png");
    capture_with(&mid_png, &buf, &CaptureOpts::default()).expect("collapsed capture");
    let mid_json = std::fs::read_to_string(mid_png.with_extension("json")).unwrap();
    assert_ne!(
        mid_json, base_json,
        "the collapsed capture must actually differ from the base — else this round-trip proves nothing"
    );
    assert!(
        mid_json.contains("\"collapsed\": [0]"),
        "outline.collapsed names Alpha's own heading index (0) while it is folded: {mid_json}"
    );
    assert!(
        !mid_json.contains("Beta"),
        "DESCENDANT SUPPRESSION: Beta (buried under the folded Alpha) must not appear anywhere in the sidecar: {mid_json}"
    );

    // --- UNFOLD: toggle the SAME heading again. ---
    let unfolded_heading = buf.toggle_fold_at_cursor();
    assert_eq!(unfolded_heading, Some(0), "toggling again unfolds the same heading");
    assert!(!buf.has_folds(), "the buffer carries no folds again");
    let after_png = dir.join("after.png");
    capture_with(&after_png, &buf, &CaptureOpts::default()).expect("restored capture");
    let after_bytes = std::fs::read(&after_png).unwrap();
    let after_json = std::fs::read_to_string(after_png.with_extension("json")).unwrap();

    assert_eq!(
        after_bytes, base_bytes,
        "unfolding must restore the rendered PNG byte-for-byte identical to the pre-collapse frame"
    );
    assert_eq!(
        after_json, base_json,
        "unfolding must restore the sidecar JSON byte-for-byte identical to the pre-collapse frame"
    );
}
