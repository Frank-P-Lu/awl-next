//! THE SAVE-FEEDBACK round's pure `apply_core` seam: `Action::Save`'s TWO
//! outcomes — `Effect::ConvertScratchAndSave` for a TRUE scratch buffer (no
//! path, never named as a note) and `Effect::SaveDone { ok, message }` for
//! everything else (an already-pathed buffer, or a buffer already started as
//! a note via `set_note_dir`/`start_note`, named or not).

use super::super::*;
use crate::overlay::OverlayKind;

/// Drive `Action::Save` against a caller-built `buffer` (so the test controls
/// its path/note_dir/text — the exact axes the Save arm's gate reads), through
/// the real `apply_core` seam, returning the resulting `Effect`.
fn drive_save(buffer: &mut Buffer) -> Effect {
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut overlay = None;
    let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
    let mut browse_to = |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
    let mut ctx = ActionCtx {
        buffer,
        shift_selecting: &mut shift,
        zoom: &mut zoom,
        search: &mut search,
        scroll_page_lines: 1,
        overlay: &mut overlay,
        make_overlay: &mut make_overlay,
        browse_to: &mut browse_to,
        oracle: None,
    };
    apply_core(&mut ctx, &Action::Save, false)
}

#[test]
fn true_scratch_buffer_save_signals_convert_scratch_and_save() {
    // No path, never named as a note (Buffer::scratch()) — the exact bug
    // report's shape: Cmd-S on the bare launch surface.
    let mut buffer = Buffer::scratch();
    for c in "brain dump".chars() {
        buffer.insert_char(c);
    }
    assert_eq!(drive_save(&mut buffer), Effect::ConvertScratchAndSave);
    // The core itself never touches the filesystem/buffer path for this
    // effect — that's the caller's job (it has `notes_root`, the core doesn't).
    assert!(buffer.path().is_none());
    assert!(!buffer.is_note());
}

#[test]
fn empty_scratch_buffer_save_still_signals_convert_not_a_plain_save_failure() {
    // An EMPTY true-scratch buffer still gets the SAME gate decision (no
    // path, not a note) — the caller's `save_as_note` is what discovers
    // there's nothing to name yet (an `Err`), not the core.
    let mut buffer = Buffer::scratch();
    assert_eq!(drive_save(&mut buffer), Effect::ConvertScratchAndSave);
}

#[test]
fn already_pathed_buffer_save_writes_and_signals_save_done_ok() {
    use std::sync::Arc;
    let path = std::path::PathBuf::from("/docs/a.md");
    let mem = crate::fs::InMemoryFs::new().with_dir("/docs");
    crate::fs::with_fs(Arc::new(mem.clone()), || {
        let mut buffer = Buffer::from_str("hello");
        buffer.set_path(path.clone());
        let effect = drive_save(&mut buffer);
        assert_eq!(effect, Effect::SaveDone { ok: true, message: "saved".to_string() });
        use crate::fs::FileSystem;
        assert_eq!(mem.read_to_string(&path).unwrap(), "hello");
    });
}

#[test]
fn already_a_note_buffer_named_or_not_never_signals_convert() {
    // A buffer that is ALREADY a note — even one that hasn't derived a
    // filename yet (`start_note`, no path) — takes the ELSE branch: a plain
    // `Buffer::save()` call, not the scratch-conversion effect. An EMPTY
    // named-nothing-yet note surfaces its OWN existing "empty note: nothing
    // to save yet" failure as `SaveDone { ok: false, .. }`.
    let mut buffer = Buffer::scratch();
    buffer.start_note(std::path::PathBuf::from("/notes"));
    let effect = drive_save(&mut buffer);
    assert_eq!(
        effect,
        Effect::SaveDone { ok: false, message: "save failed: empty note: nothing to save yet".to_string() }
    );
}

#[test]
fn a_note_with_text_saves_and_derives_its_filename_via_save_done_ok() {
    use std::sync::Arc;
    let dir = std::path::PathBuf::from("/notes");
    let mem = crate::fs::InMemoryFs::new().with_dir(&dir);
    crate::fs::with_fs(Arc::new(mem.clone()), || {
        let mut buffer = Buffer::scratch();
        buffer.start_note(dir.clone());
        for c in "first light".chars() {
            buffer.insert_char(c);
        }
        let effect = drive_save(&mut buffer);
        assert_eq!(effect, Effect::SaveDone { ok: true, message: "saved".to_string() });
        assert_eq!(buffer.path().unwrap().file_name().unwrap(), "first-light.md");
    });
}

#[test]
fn second_save_on_a_converted_scratch_buffer_is_a_plain_save_done() {
    // The "second Save is a plain save" contract, exercised at the apply_core
    // seam directly: once the buffer HAS a path (as it would after the
    // caller's `Effect::ConvertScratchAndSave` handling converted it), a
    // further `Action::Save` takes the ordinary `SaveDone` branch, never
    // `ConvertScratchAndSave` again.
    use std::sync::Arc;
    let dir = std::path::PathBuf::from("/notes");
    let mem = crate::fs::InMemoryFs::new().with_dir(&dir);
    crate::fs::with_fs(Arc::new(mem), || {
        let mut buffer = Buffer::scratch();
        for c in "second time".chars() {
            buffer.insert_char(c);
        }
        // Mirror what `App::convert_scratch_and_save` does for the FIRST save.
        buffer.save_as_note(&dir).unwrap();
        assert!(buffer.path().is_some());

        buffer.insert_char('!');
        let effect = drive_save(&mut buffer);
        assert_eq!(effect, Effect::SaveDone { ok: true, message: "saved".to_string() });
    });
}
