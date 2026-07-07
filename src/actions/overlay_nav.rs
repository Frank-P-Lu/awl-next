//! The SUMMONED NAVIGATION OVERLAY's key handling — the modal intercept that, while
//! a picker is open, OWNS every key (printable chars filter the query, Up/Down move
//! the selection, Right/Left descend/ascend the explorers, Enter accepts, Esc/C-g
//! cancels-or-reverts). Routing this through the shared core (not just `App`) is what
//! makes the overlay drivable under `--keys`. [`overlay_intercept`] is the dispatch
//! `apply_core` calls the instant `ctx.overlay.is_some()`; the small browse-path
//! helpers below ([`join_browse`] / [`browse_parent`] / [`descend_target`] /
//! [`ascend_target`] / [`move_dest_value`]) compute the explorer's descend/ascend/
//! accept targets, and [`preview_overlay`] applies the Theme/Caret live preview as the
//! selection moves. Carved out of `actions.rs` VERBATIM (the `is_some()` block lifted
//! into one named seam).

use super::*;

/// How many rows a PgUp/PgDn pages the SUMMONED picker selection — one card-ful (the
/// overlay renders up to 12 rows; see `render/chrome.rs` `MAX_ROWS`).
const OVERLAY_PAGE: isize = 12;

/// The modal OVERLAY INTERCEPT. When the summoned navigation overlay is open, it OWNS
/// every key: printable chars extend the overlay query (never the rope), Up/Down (and
/// C-n/C-p, which resolve to NextLine/PreviousLine) move the selection, Enter accepts
/// the highlighted item, Esc/C-g cancels. Routing this through the shared core (rather
/// than only in `App`) is exactly what makes the overlay drivable under `--keys` — the
/// same mistake the isearch panel made (its query routing lives in `App`, so `--keys`
/// can't type into it) is deliberately avoided here. Returns the one [`Effect`] the key
/// signals back; `apply_core` returns it directly (the overlay is modal, so the key
/// never reaches the buffer).
pub(super) fn overlay_intercept(ctx: &mut ActionCtx, action: &Action) -> Effect {
    // REBIND MENU: while its capture sub-state is active (or for its list-level
    // Enter/Delete), the menu OWNS the key at the chord level — handled before the
    // generic picker intercept. Returns Some(effect) when fully handled; None to
    // fall through to the shared list nav/filter below.
    if ctx.overlay.as_ref().unwrap().kind == crate::overlay::OverlayKind::Keybindings {
        if let Some(eff) = keybindings_intercept(ctx, action) {
            return eff;
        }
    }
    match action {
        Action::InsertChar(c) => {
            ctx.overlay.as_mut().unwrap().push(*c);
            // Typing to fuzzy-filter also PREVIEWS the new top/selected match.
            preview_overlay(ctx.overlay.as_ref().unwrap());
            return Effect::None;
        }
        Action::DeleteBackward | Action::DeleteWordBackward => {
            // In the navigable explorers (Browse / MoveDest / Project),
            // Backspace doubles as "go to PARENT" once the fuzzy filter is
            // empty (file-explorer muscle memory): a non-empty query pops a
            // char (preserving filtering), an empty query ASCENDS like Left.
            let ov = ctx.overlay.as_ref().unwrap();
            let navigable = matches!(
                ov.kind,
                crate::overlay::OverlayKind::Browse
                    | crate::overlay::OverlayKind::MoveDest
                    | crate::overlay::OverlayKind::Project
            );
            if navigable && ov.query.is_empty() {
                if let Some(parent) = ascend_target(ov) {
                    if let Some(next) = (ctx.browse_to)(ov.kind, parent) {
                        *ctx.overlay = Some(next);
                    }
                }
                return Effect::None;
            }
            ctx.overlay.as_mut().unwrap().pop();
            preview_overlay(ctx.overlay.as_ref().unwrap());
            return Effect::None;
        }
        Action::NextLine => {
            ctx.overlay.as_mut().unwrap().move_sel(1);
            // LIVE PREVIEW: moving the selection in the Theme picker applies
            // that world immediately (no-op for the other overlay kinds).
            preview_overlay(ctx.overlay.as_ref().unwrap());
            return Effect::None;
        }
        // PgDn / PgUp (C-v / M-v / the named keys) PAGE the selection a card-ful
        // at a time (`move_sel` clamps), so a long picker — the rebind menu's full
        // command list — is pageable, not just one-row-at-a-time.
        Action::PageScrollDown => {
            ctx.overlay.as_mut().unwrap().move_sel(OVERLAY_PAGE);
            preview_overlay(ctx.overlay.as_ref().unwrap());
            return Effect::None;
        }
        Action::PageScrollUp => {
            ctx.overlay.as_mut().unwrap().move_sel(-OVERLAY_PAGE);
            preview_overlay(ctx.overlay.as_ref().unwrap());
            return Effect::None;
        }
        Action::ForwardChar => {
            let ov = ctx.overlay.as_ref().unwrap();
            // FACETED PICKER (goto / browse / theme): LEFT/RIGHT switch the faceting
            // LENS (keeping the same item highlighted), NOT the row selection — the
            // lens-switcher model, checked BEFORE the navigable branch so a FACETED
            // explorer (Browse) cycles its lens on ←/→ while descend rides Enter (on a
            // folder) and ascend rides Backspace. The regroup may land the item in a
            // new section; preview it (a no-op when it's the same item).
            if ov.is_faceting() {
                ctx.overlay.as_mut().unwrap().cycle_lens(1);
                preview_overlay(ctx.overlay.as_ref().unwrap());
                return Effect::None;
            }
            // In the NON-faceting navigable explorers (MOVE-DEST / PROJECT) Right
            // DESCENDS into the highlighted folder (a no-op on a file row): Right/Enter
            // descend, Left/Backspace ascend. For a flat, non-faceting picker Right is
            // a down-move.
            if matches!(
                ov.kind,
                crate::overlay::OverlayKind::Browse
                    | crate::overlay::OverlayKind::MoveDest
                    | crate::overlay::OverlayKind::Project
            ) {
                if ov.selected_is_dir() {
                    if let Some(name) = ov.selected_value().map(|s| s.to_string()) {
                        let child = descend_target(ov, &name);
                        if let Some(next) = (ctx.browse_to)(ov.kind, Some(child)) {
                            *ctx.overlay = Some(next);
                        }
                    }
                }
                return Effect::None;
            }
            ctx.overlay.as_mut().unwrap().move_sel(1);
            preview_overlay(ctx.overlay.as_ref().unwrap());
            return Effect::None;
        }
        Action::PreviousLine => {
            ctx.overlay.as_mut().unwrap().move_sel(-1);
            preview_overlay(ctx.overlay.as_ref().unwrap());
            return Effect::None;
        }
        Action::BackwardChar => {
            let ov = ctx.overlay.as_ref().unwrap();
            // FACETED PICKER (goto / browse / theme): LEFT cycles the faceting lens
            // back (keeping the item) — checked BEFORE the navigable branch so a
            // faceted explorer (Browse) cycles its lens on ←, with ascend on Backspace.
            if ov.is_faceting() {
                ctx.overlay.as_mut().unwrap().cycle_lens(-1);
                preview_overlay(ctx.overlay.as_ref().unwrap());
                return Effect::None;
            }
            // Up for a flat picker; in the NON-faceting explorers (MOVE-DEST /
            // PROJECT) Left ASCENDS one directory level (rebuilds the list with the
            // parent's children). MoveDest floors at its root; Project climbs by
            // absolute path with no floor (so it can go ABOVE the workspace).
            if matches!(
                ov.kind,
                crate::overlay::OverlayKind::Browse
                    | crate::overlay::OverlayKind::MoveDest
                    | crate::overlay::OverlayKind::Project
            ) {
                if let Some(parent) = ascend_target(ov) {
                    if let Some(next) = (ctx.browse_to)(ov.kind, parent) {
                        *ctx.overlay = Some(next);
                    }
                }
                return Effect::None;
            }
            ctx.overlay.as_mut().unwrap().move_sel(-1);
            preview_overlay(ctx.overlay.as_ref().unwrap());
            return Effect::None;
        }
        Action::Newline => {
            // Accept. For BROWSE, Enter on a FOLDER descends (rebuilds the
            // list with that folder's children) instead of closing; Enter on a
            // FILE opens it (emitted as a Goto path) and closes. For Goto /
            // Project, Enter emits the chosen value and closes (Project Enter on
            // a folder PICKS it as the root — descend is on Right). A no-match
            // closes without emitting.
            //
            // SPELL suggestion accept: REPLACE the targeted misspelled word with
            // the chosen suggestion as ONE undoable edit, then close. The owned
            // bits (the picked suggestion + the word's char span) are pulled out
            // first so the immutable overlay borrow is released before the buffer
            // is mutated. An empty/no-match list just closes (no edit).
            {
                let ov = ctx.overlay.as_ref().unwrap();
                if ov.kind == crate::overlay::OverlayKind::Spell {
                    let pick = ov.selected_value().map(|s| s.to_string());
                    let target = ov.spell_target;
                    if let (Some(word), Some((line, start, end))) = (pick, target) {
                        let s = ctx.buffer.line_col_to_char(line, start);
                        let e = ctx.buffer.line_col_to_char(line, end);
                        ctx.buffer.replace_char_range(s, e, &word);
                    }
                    *ctx.overlay = None;
                    return Effect::None;
                }
            }
            let ov = ctx.overlay.as_ref().unwrap();
            if ov.kind == crate::overlay::OverlayKind::Browse {
                let mut eff = Effect::None;
                if let Some(name) = ov.selected_value().map(|s| s.to_string()) {
                    if ov.selected_is_dir() {
                        // Descend: parent dir = browse_dir, child = name.
                        let child = join_browse(ov.browse_dir.as_deref(), &name);
                        if let Some(next) = (ctx.browse_to)(ov.kind, Some(child)) {
                            *ctx.overlay = Some(next);
                        }
                        return Effect::None;
                    }
                    // File: open via the Goto path so the caller's open_rel
                    // loads it. The accept value is the FULL root-relative path.
                    let rel = join_browse(ov.browse_dir.as_deref(), &name);
                    eff = Effect::OverlayAccept(crate::overlay::OverlayKind::Goto, rel);
                }
                *ctx.overlay = None;
                return eff;
            }
            if ov.kind == crate::overlay::OverlayKind::Project {
                // PROJECT PICKER: the primary action of Enter is "make this
                // folder the project". Enter on a real FOLDER ACCEPTS that
                // folder's ABSOLUTE path as the new root (descend is on Right,
                // not Enter). Enter on the synthetic "." row ACCEPTS the CURRENT
                // directory. Either way we emit the absolute path the caller
                // feeds to set_root (re-index + recompute branch/dirty), and
                // CLOSE — never a silent no-op.
                let dir = if ov.selected_is_dir() {
                    // The highlighted folder's absolute path = current dir + name.
                    ov.selected_value().map(|name| descend_target(ov, name))
                } else {
                    // The "." accept-this-folder row (or no match): the current
                    // directory itself (always the absolute browse_dir).
                    ov.browse_dir.clone()
                };
                let eff = match dir.filter(|d| !d.is_empty()) {
                    Some(dir) => Effect::OverlayAccept(crate::overlay::OverlayKind::Project, dir),
                    None => Effect::None,
                };
                *ctx.overlay = None;
                return eff;
            }
            if ov.kind == crate::overlay::OverlayKind::MoveDest {
                // ACCEPT a destination FOLDER (notes-root-relative). Enter on a
                // highlighted folder moves into it; a typed name matching no
                // folder is a NEW folder to create; nothing typed/selected
                // accepts the CURRENT level. The caller does the mkdir + move.
                let eff = match move_dest_value(ov) {
                    Some(dest) => Effect::OverlayAccept(crate::overlay::OverlayKind::MoveDest, dest),
                    None => Effect::None,
                };
                *ctx.overlay = None;
                return eff;
            }
            if ov.kind == crate::overlay::OverlayKind::Command {
                // RUN the highlighted command. The corpus order == the catalog
                // order, so the selected corpus index maps straight back to
                // `COMMANDS[i]`. Close the palette FIRST so the caller's
                // re-dispatch lands with the slot empty (an overlay-opening
                // command can then open into it); a no-match closes silently.
                let eff = ov
                    .selected_corpus_index()
                    .map(|i| Effect::RunAction(crate::commands::COMMANDS[i].action.clone()))
                    .unwrap_or(Effect::None);
                *ctx.overlay = None;
                return eff;
            }
            if ov.kind == crate::overlay::OverlayKind::Theme {
                // COMMIT: the highlighted world is ALREADY active (live preview
                // applied it as the selection moved), so Enter just keeps it and
                // closes. Emit the committed name so the caller can re-tint its
                // GPU pipelines / window title to match.
                let eff = match ov.selected_value() {
                    Some(v) => Effect::OverlayAccept(ov.kind, v.to_string()),
                    None => Effect::None,
                };
                *ctx.overlay = None;
                return eff;
            }
            if ov.kind == crate::overlay::OverlayKind::Caret {
                // COMMIT: the highlighted look is ALREADY active (live preview
                // applied it to the process-global as the selection moved), so
                // Enter keeps it and closes. Emit the committed look's LABEL so
                // the caller can PERSIST the caret style (phase 1's `caret_mode`
                // preference) — the picker's whole point over the blind toggle.
                let eff = match ov.selected_value() {
                    Some(v) => Effect::OverlayAccept(ov.kind, v.to_string()),
                    None => Effect::None,
                };
                *ctx.overlay = None;
                return eff;
            }
            if ov.kind == crate::overlay::OverlayKind::Dictionary {
                // COMMIT: UNLIKE Theme/Caret there is NO live preview during
                // navigation (a dictionary re-parse is a real one-time cost — see
                // `spell.rs` — so it happens exactly ONCE, here, on accept). Set
                // the process-global THEN emit the committed label so the caller
                // (App) reconstructs its `SpellChecker` + persists the pref.
                let eff = match ov.selected_value().and_then(crate::spell::DictVariant::from_label) {
                    Some(dv) => {
                        crate::spell::set_active_variant(dv);
                        Effect::OverlayAccept(ov.kind, dv.label().to_string())
                    }
                    None => Effect::None,
                };
                *ctx.overlay = None;
                return eff;
            }
            if ov.kind == crate::overlay::OverlayKind::Outline {
                // JUMP to the highlighted heading's line. Emit the LINE NUMBER
                // (not the heading text — titles can repeat) so the caller moves
                // the cursor there; a no-match closes silently.
                let eff = match ov.selected_line() {
                    Some(line) => Effect::OverlayAccept(ov.kind, line.to_string()),
                    None => Effect::None,
                };
                *ctx.overlay = None;
                return eff;
            }
            if ov.kind == crate::overlay::OverlayKind::History {
                // RESTORE the highlighted version. Emit its opaque restore ID (which
                // the caller resolves via `history::load` and writes back with
                // `Buffer::set_text` — one undoable edit, so C-/ undoes the restore).
                // The synthetic "no history yet" row has an empty id, so Enter there
                // just closes (no-op).
                let eff = match ov.selected_history_id() {
                    Some(id) => Effect::OverlayAccept(ov.kind, id.to_string()),
                    None => Effect::None,
                };
                *ctx.overlay = None;
                return eff;
            }
            let eff = match ov.selected_value() {
                Some(v) => Effect::OverlayAccept(ov.kind, v.to_string()),
                None => Effect::None,
            };
            *ctx.overlay = None;
            return eff;
        }
        Action::ToggleHiddenFiles => {
            // Cmd-Shift-. : REVEAL / re-hide dot-prefixed entries in THIS picker (the
            // Finder convention). A no-op for a non-file picker (`toggle_hidden`
            // gates on the kind), so it's safe to route uniformly. Rebuilds the
            // listing with the new `show_hidden` flag; the sidecar reflects it.
            ctx.overlay.as_mut().unwrap().toggle_hidden();
            return Effect::None;
        }
        Action::Cancel => {
            // REVERT the live preview: the Theme picker restores the world, and
            // the Caret picker restores the LOOK, that was active when it opened.
            // Other overlays just close.
            let ov = ctx.overlay.as_ref().unwrap();
            let eff = if ov.kind == crate::overlay::OverlayKind::Theme {
                if let Some(orig) = ov.original_theme {
                    crate::theme::set_active(orig);
                }
                // Signal the revert so the caller can re-tint to the restored
                // world. The accept VALUE is the restored world's name.
                let name = crate::theme::active().name.to_string();
                Effect::OverlayAccept(crate::overlay::OverlayKind::Theme, name)
            } else if ov.kind == crate::overlay::OverlayKind::Caret {
                // Restore the look active when the picker opened (undo the live
                // preview). NO Effect: a revert must NOT persist — the caret
                // preference only changes on a COMMIT (Enter), exactly like the
                // theme's persist-on-commit-only rule. The process-global is reset
                // here so the document caret returns to the pre-picker look.
                if let Some(orig) = ov.original_caret {
                    crate::caret::set_mode(orig);
                }
                Effect::None
            } else {
                Effect::None
            };
            *ctx.overlay = None;
            return eff;
        }
        // Any other action while the overlay is up is swallowed (the overlay
        // is modal); it never reaches the buffer.
        _ => return Effect::None,
    }
}

/// Join a browse directory (root-relative, `None` = root) with a child entry
/// name into a single root-relative, forward-slashed path.
pub(super) fn join_browse(dir: Option<&str>, name: &str) -> String {
    match dir {
        Some(d) if !d.is_empty() => format!("{d}/{name}"),
        _ => name.to_string(),
    }
}

/// The PARENT browse directory of `dir` (root-relative, `None` = root), as the
/// value to pass back to `browse_to`. Returns `None` when already at the root
/// (Left there is a no-op). One level up: `docs/api` -> `Some("docs")`, `docs`
/// -> `Some(None)` (i.e. the root), root -> `None`.
pub(super) fn browse_parent(dir: Option<&str>) -> Option<Option<String>> {
    match dir {
        None => None, // already at root; nothing above
        Some(d) => match d.rsplit_once('/') {
            Some((parent, _)) => Some(Some(parent.to_string())),
            None => Some(None), // one level deep -> back to root
        },
    }
}

/// The DESCEND target for the highlighted folder `name` in `ov`, as the value to
/// pass back to `browse_to`. `Project` navigates by ABSOLUTE path (so it can roam
/// the whole filesystem); `Browse`/`MoveDest` stay root-relative.
pub(super) fn descend_target(ov: &OverlayState, name: &str) -> String {
    match ov.kind {
        crate::overlay::OverlayKind::Project => std::path::Path::new(ov.browse_dir.as_deref().unwrap_or(""))
            .join(name)
            .to_string_lossy()
            .to_string(),
        _ => join_browse(ov.browse_dir.as_deref(), name),
    }
}

/// The ASCEND target (parent directory) for `ov`. Outer `None` = can't ascend
/// (no-op). `Project` uses real `Path::parent()` with NO root floor (so Left /
/// Backspace climb ABOVE the workspace, stopping only at the filesystem root);
/// `Browse`/`MoveDest` floor at their root via [`browse_parent`].
pub(super) fn ascend_target(ov: &OverlayState) -> Option<Option<String>> {
    match ov.kind {
        crate::overlay::OverlayKind::Project => std::path::Path::new(ov.browse_dir.as_deref().unwrap_or("/"))
            .parent()
            .map(|p| Some(p.to_string_lossy().to_string())),
        _ => browse_parent(ov.browse_dir.as_deref()),
    }
}

/// The accepted MOVE destination for a `MoveDest` overlay, as a notes-root-relative
/// directory path (`""` = the notes root itself). Precedence: a highlighted FOLDER
/// (move into it); else a non-empty typed QUERY that matched no folder (a NEW folder
/// to create at this level); else the CURRENT level. The caller mkdir's + moves.
pub(super) fn move_dest_value(ov: &OverlayState) -> Option<String> {
    // A highlighted folder is the destination (descend-as-accept).
    if let Some(name) = ov.selected_value() {
        if ov.selected_is_dir() {
            return Some(join_browse(ov.browse_dir.as_deref(), name));
        }
    }
    // No folder highlighted: a typed name becomes a NEW folder at this level.
    let q = ov.query.trim();
    if !q.is_empty() {
        return Some(join_browse(ov.browse_dir.as_deref(), q));
    }
    // Nothing typed or selected: accept the current level (`None` root -> "").
    Some(ov.browse_dir.clone().unwrap_or_default())
}

/// LIVE PREVIEW for the Theme picker: if `ov` is the Theme overlay, apply its
/// currently-highlighted world to the process-global active theme so the rendered
/// frame shows it immediately. A no-op for every other overlay kind (and when no
/// item matches the filter). Driven from the overlay move / filter paths so the
/// preview is identical under `--keys` and live.
/// LIVE PREVIEW as the selection moves in a preview-carrying picker: the THEME
/// picker re-tints to the highlighted world, the CARET-STYLE picker applies the
/// highlighted look to the process-global (so BOTH the document caret and the
/// picker's preview box switch to it). A no-op for every other overlay kind. The
/// caller persists nothing here — preview is ephemeral; only the Enter COMMIT path
/// writes the preference (mirroring the theme picker's commit-only persistence).
pub(crate) fn preview_overlay(ov: &OverlayState) {
    match ov.kind {
        crate::overlay::OverlayKind::Theme => {
            if let Some(name) = ov.selected_value() {
                crate::theme::set_active_by_name(name);
            }
        }
        crate::overlay::OverlayKind::Caret => {
            if let Some(m) = ov.selected_caret_mode() {
                crate::caret::set_mode(m);
            }
        }
        _ => {}
    }
}
