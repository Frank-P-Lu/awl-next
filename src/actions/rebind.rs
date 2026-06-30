//! The GAME-STYLE REBIND MENU key handling, layered ON TOP of the shared picker
//! intercept. While the `Keybindings` overlay's capture sub-state is active (or for
//! its list-level Enter/Delete), [`keybindings_intercept`] OWNS the key at the chord
//! level — handled before the generic picker nav/filter. A finished capture is
//! signalled back to the caller as an [`Effect`] ([`finalize_capture`] →
//! `RebindCommit` / `RebindReset`) for it to persist + live-reload; the overlay's
//! capture state machine itself lives on `OverlayState`. Carved out of `actions.rs`
//! VERBATIM.

use super::*;

/// REBIND MENU key handling, layered ON TOP of the shared picker intercept. Returns
/// `Some(effect)` when the rebind menu fully handles the key — either the capture
/// sub-state (choose mode / record / confirm) or a list-level Enter (start a capture)
/// / Delete (reset to default). Returns `None` to let the SHARED list nav / fuzzy
/// filter / Esc-close run, so browsing the command list reuses the generic picker.
///
/// The CAPTURE itself is chord-level, which the action stream can't fully express:
/// a PLAIN-key combo arrives here as `InsertChar` (so `--keys` can drive a plain
/// capture headlessly), while a MODIFIED chord (C-t / M-f) is recorded LIVE in
/// `app.rs` before keymap resolution — both call `OverlayState::capture_record`, so
/// the state machine is one place. Commit / reset are signalled back as [`Effect`]s
/// for the caller (live App / headless replay) to persist + reload.
pub(super) fn keybindings_intercept(ctx: &mut ActionCtx, action: &Action) -> Option<Effect> {
    let stage = ctx
        .overlay
        .as_ref()
        .unwrap()
        .capture
        .as_ref()
        .map(|c| c.stage);
    let ov = ctx.overlay.as_mut().unwrap();
    match stage {
        // BROWSING the command list: Enter starts a capture, Delete resets the
        // highlighted command; everything else (nav / filter / Esc) falls through.
        None => match action {
            Action::Newline => {
                ov.start_capture();
                Some(Effect::None)
            }
            Action::DeleteForward => {
                let name = ov.selected_value().map(str::to_string);
                match ov.selected_command_slug() {
                    Some(slug) => {
                        ov.notice = format!("reset {} to default", name.unwrap_or_default());
                        Some(Effect::RebindReset { slug })
                    }
                    None => Some(Effect::None),
                }
            }
            _ => None,
        },
        // CHOOSE KEY vs CHORD: Up/Down (or Left/Right) toggle, Enter begins recording.
        Some(crate::overlay::CaptureStage::ChooseMode) => match action {
            Action::NextLine | Action::ForwardChar => {
                ov.capture_move_mode(1);
                Some(Effect::None)
            }
            Action::PreviousLine | Action::BackwardChar => {
                ov.capture_move_mode(-1);
                Some(Effect::None)
            }
            Action::Newline => {
                ov.capture_begin_recording();
                Some(Effect::None)
            }
            Action::Cancel => {
                ov.capture_abort();
                Some(Effect::None)
            }
            // Modal: swallow any other key so it never reaches the buffer / filter.
            _ => Some(Effect::None),
        },
        // RECORDING: Esc aborts, Enter finishes a CHORD, a printable key records a
        // (plain) combo. KEY mode finishes on the first recorded combo.
        Some(crate::overlay::CaptureStage::Recording) => match action {
            Action::Cancel => {
                ov.capture_abort();
                Some(Effect::None)
            }
            Action::Newline => {
                let empty = ov
                    .capture
                    .as_ref()
                    .map(|c| c.captured.is_empty())
                    .unwrap_or(true);
                if empty {
                    Some(Effect::None) // nothing pressed yet; wait
                } else {
                    Some(finalize_capture(ov, false))
                }
            }
            Action::InsertChar(c) => {
                if ov.capture_record(c.to_string()) {
                    Some(finalize_capture(ov, false))
                } else {
                    Some(Effect::None)
                }
            }
            _ => Some(Effect::None),
        },
        // CONFIRM a conflict: Enter commits anyway, Esc aborts.
        Some(crate::overlay::CaptureStage::Confirm) => match action {
            Action::Newline => Some(finalize_capture(ov, true)),
            Action::Cancel => {
                ov.capture_abort();
                Some(Effect::None)
            }
            _ => Some(Effect::None),
        },
    }
}

/// Turn the in-progress capture into a [`Effect::RebindCommit`] (or a quiet close if
/// nothing was captured). The overlay's capture state is LEFT INTACT so the caller
/// can either commit (then `capture_abort` + refresh) or, on a clash, move it into
/// the `Confirm` phase. `confirmed` marks the Confirm-stage commit (skip re-gating).
pub(super) fn finalize_capture(ov: &mut crate::overlay::OverlayState, confirmed: bool) -> Effect {
    match ov.capture_target() {
        Some((slug, binding)) => Effect::RebindCommit {
            slug,
            binding,
            confirmed,
        },
        None => {
            ov.capture_abort();
            Effect::None
        }
    }
}
