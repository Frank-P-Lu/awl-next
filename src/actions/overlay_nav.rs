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
    // SETTINGS VALUE EDIT: while an inline numeric edit is active (Enter landed on a
    // page-width / zoom row), the Settings menu OWNS every key modally — digits (plus
    // `.`/`%` for zoom) build the value in the row's own cell, Backspace deletes, Enter
    // COMMITS (signals `SettingValueCommit` for the App to parse-clamp-apply-persist),
    // Esc CANCELS (restores the cell). Checked FIRST so an arrow/char never leaks to
    // the list nav below while editing. The `Value` kind arms this in `settings_accept`.
    if ctx.overlay.as_ref().unwrap().value_edit.is_some() {
        match action {
            Action::InsertChar(c) => {
                ctx.overlay.as_mut().unwrap().value_edit_push(*c);
                return Effect::None;
            }
            Action::DeleteBackward | Action::DeleteWordBackward => {
                ctx.overlay.as_mut().unwrap().value_edit_pop();
                return Effect::None;
            }
            Action::Newline => {
                // Commit: pull the (key, typed value), clear the sub-state (menu stays
                // open), and signal the App to parse + clamp + apply + persist.
                let target = ctx.overlay.as_ref().unwrap().value_edit_target();
                ctx.overlay.as_mut().unwrap().value_edit = None;
                return match target {
                    Some((key, value)) => Effect::SettingValueCommit { key, value },
                    None => Effect::None,
                };
            }
            Action::Cancel => {
                ctx.overlay.as_mut().unwrap().value_edit_cancel();
                return Effect::None;
            }
            // Every other key is swallowed (the edit is modal to the row).
            _ => return Effect::None,
        }
    }
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
                let bc = Breadcrumb::of(ov);
                if let Some(parent) = ascend_target(ov) {
                    if let Some(mut next) = (ctx.browse_to)(ov.kind, parent) {
                        bc.apply(&mut next);
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
            // In the NON-faceting navigable explorer (MOVE-DEST) Right DESCENDS into
            // the highlighted folder (a no-op on a file row): Right descends, Left
            // ascends. (Browse + Project FACET, so they took the lens-cycle branch
            // above; their descend rides Enter, ascend rides Backspace.) For a flat,
            // non-faceting picker Right is a down-move.
            if ov.kind == crate::overlay::OverlayKind::MoveDest {
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
            // Up for a flat picker; in the NON-faceting explorer (MOVE-DEST) Left
            // ASCENDS one directory level (rebuilds the list with the parent's
            // children), flooring at its root. (Browse + Project FACET, so they took
            // the lens-cycle branch above; their ascend rides Backspace.)
            if ov.kind == crate::overlay::OverlayKind::MoveDest {
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
            // Accept. For BROWSE / PROJECT (both faceted navigators), Enter on a
            // FOLDER descends (rebuilds the list with that folder's children)
            // instead of closing; Browse Enter on a FILE opens it (emitted as a Goto
            // path) and closes, while Project Enter on the synthetic "." row SELECTS
            // the current dir as the root. For Goto, Enter emits the chosen value and
            // closes. A no-match closes without emitting.
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
                // SETTINGS MENU accept: toggle in place / open a sub-picker (with a
                // return_to breadcrumb) / open config-as-text / no-op. Handled in one
                // seam so the borrow of `ctx.overlay` is scoped there.
                if ov.kind == crate::overlay::OverlayKind::Settings {
                    return settings_accept(ctx);
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
                close_overlay(ctx);
                return eff;
            }
            if ov.kind == crate::overlay::OverlayKind::Project {
                // PROJECT NAVIGATOR (now FACETED like Browse — ←/→ cycle the
                // All/Recent lens): Enter on a real FOLDER DESCENDS into it (drill
                // in, overlay stays open at that level), while Enter on the synthetic
                // "." row SELECTS the CURRENT directory as the project root. Ascend
                // is on Backspace. This mirrors Browse exactly (folder descends, the
                // accept affordance is a dedicated row) now that ←/→ belong to the
                // lens strip.
                if ov.selected_is_dir() {
                    // Carry the Settings breadcrumb forward so descending while picking
                    // a path for a Settings row keeps writing THAT key (see `Breadcrumb`).
                    let bc = Breadcrumb::of(ov);
                    if let Some(name) = ov.selected_value().map(|s| s.to_string()) {
                        let child = descend_target(ov, &name);
                        if let Some(mut next) = (ctx.browse_to)(ov.kind, Some(child)) {
                            bc.apply(&mut next);
                            *ctx.overlay = Some(next);
                        }
                    }
                    return Effect::None;
                }
                // The "." select-this-folder row (or no match): the current
                // directory itself (always the absolute browse_dir). We emit the
                // absolute path the caller feeds to set_root (re-index + recompute
                // branch/dirty) and CLOSE.
                let dir = ov.browse_dir.clone();
                // A navigator opened FROM a Settings PATH row (its `setting_path_key`
                // is set) writes THAT config key instead of switching the project —
                // `close_overlay` re-summons Settings via the `return_to` breadcrumb.
                let path_key = ov.setting_path_key.clone();
                let eff = match dir.filter(|d| !d.is_empty()) {
                    Some(dir) => match path_key {
                        Some(key) => Effect::SettingPathPick { key, path: dir },
                        None => Effect::OverlayAccept(crate::overlay::OverlayKind::Project, dir),
                    },
                    None => Effect::None,
                };
                close_overlay(ctx);
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
                close_overlay(ctx);
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
                close_overlay(ctx);
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
                close_overlay(ctx);
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
                close_overlay(ctx);
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
                close_overlay(ctx);
                return eff;
            }
            if ov.kind == crate::overlay::OverlayKind::Goto && ov.selected_is_heading() {
                // GO-TO's HEADINGS lens (the retired Outline picker): the highlighted
                // row is a document heading, so JUMP the cursor to its line rather than
                // open a file. Emit the LINE NUMBER (titles can repeat, so the line is
                // the accept value, not the text); a file row falls through to the
                // ordinary Goto open below. A no-match closes silently.
                let eff = match ov.selected_line() {
                    Some(line) => Effect::JumpToLine(line),
                    None => Effect::None,
                };
                close_overlay(ctx);
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
                close_overlay(ctx);
                return eff;
            }
            let eff = match ov.selected_value() {
                Some(v) => Effect::OverlayAccept(ov.kind, v.to_string()),
                None => Effect::None,
            };
            close_overlay(ctx);
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
            close_overlay(ctx);
            return eff;
        }
        // Any other action while the overlay is up is swallowed (the overlay
        // is modal); it never reaches the buffer.
        _ => return Effect::None,
    }
}

/// Close the summoned overlay — but if it carries a `return_to` BREADCRUMB (the
/// settings menu opened it as a sub-picker), RE-SUMMON that parent instead of
/// closing to the buffer. The ONE owner of the summoned-picker close, so every
/// accept / cancel path honors the breadcrumb identically. SINGLE-LEVEL: the
/// re-summoned parent (built fresh via `make_overlay`, so its value cells reflect
/// the change the sub-picker just committed) carries no breadcrumb of its own, so
/// there is no N-deep stack. A `None` breadcrumb (every normal top-level summon)
/// closes to the buffer exactly as `*ctx.overlay = None` always did.
pub(super) fn close_overlay(ctx: &mut ActionCtx) {
    let back = ctx.overlay.as_ref().and_then(|o| o.return_to);
    *ctx.overlay = match back {
        Some(kind) => (ctx.make_overlay)(kind),
        None => None,
    };
}

/// SETTINGS MENU accept (Enter on a row): dispatch by the highlighted row's
/// [`crate::settings::SettingKind`] — a TOGGLE signals [`Effect::SettingToggle`]
/// and keeps the menu OPEN (the caller flips + persists + refreshes the value
/// cell); a PICKER / SUBMENU swaps the overlay for that sub-picker, stamping a
/// `return_to = Settings` breadcrumb so its commit/cancel returns here; the
/// ADVANCED "Edit config as text" row closes the menu and opens config.toml
/// ([`Effect::OpenSettings`]); a VALUE row arms the inline numeric edit sub-state; a
/// PATH row opens the folder navigator (breadcrumb back to Settings); a LIST row
/// opens config-as-text (the v2 scope call for cjk_priority). The corpus is in
/// [`crate::settings::SETTINGS`] table order, so the selected corpus index maps
/// straight back to the row.
fn settings_accept(ctx: &mut ActionCtx) -> Effect {
    let Some(ci) = ctx.overlay.as_ref().unwrap().selected_corpus_index() else {
        // No row matches the filter: close (Settings itself carries no breadcrumb).
        close_overlay(ctx);
        return Effect::None;
    };
    let row = crate::settings::SETTINGS[ci];
    match row.kind {
        // Flip IN PLACE: leave the menu open, signal the caller to toggle + persist +
        // refresh the value cell. A row with no key (shouldn't happen for a Toggle) is
        // a calm no-op rather than a signal.
        crate::settings::SettingKind::Toggle => match crate::settings::toggle_key(row.name) {
            Some(key) => Effect::SettingToggle { key: key.to_string() },
            None => Effect::None,
        },
        // Open the sub-picker with a breadcrumb back to Settings. `make_overlay` builds
        // it from the live globals (theme/caret/dictionary/keybindings), so a commit
        // reflects in the value cell when `close_overlay` re-summons Settings.
        crate::settings::SettingKind::Picker | crate::settings::SettingKind::Submenu => {
            if let Some(target) = crate::settings::sub_overlay(row.name) {
                if let Some(mut next) = (ctx.make_overlay)(target) {
                    next.return_to = Some(crate::overlay::OverlayKind::Settings);
                    *ctx.overlay = Some(next);
                }
            }
            Effect::None
        }
        // "Edit config as text": close the menu, open config.toml (the raw escape
        // hatch — the same Effect the old config-as-text Settings command fired).
        crate::settings::SettingKind::Action => {
            *ctx.overlay = None;
            Effect::OpenSettings
        }
        // VALUE (page widths / zoom): arm the inline numeric edit sub-state, seeded
        // from the row's current cell. The menu stays open; the modal intercept above
        // then owns the keys until Enter commits / Esc cancels.
        crate::settings::SettingKind::Value => {
            if let Some(key) = crate::settings::value_key(row.name) {
                ctx.overlay
                    .as_mut()
                    .unwrap()
                    .start_value_edit(key.to_string(), row.name.to_string());
            }
            Effect::None
        }
        // PATH (notes_root / workspace / project_root): open the folder NAVIGATOR (the
        // Project picker, which roams the filesystem by absolute path) with a
        // `return_to = Settings` breadcrumb + the config key stamped, so its accept
        // writes THAT key and returns here rather than switching the project blindly.
        crate::settings::SettingKind::Path => {
            if let Some(key) = crate::settings::path_key(row.name) {
                if let Some(mut nav) = (ctx.browse_to)(crate::overlay::OverlayKind::Project, None) {
                    nav.return_to = Some(crate::overlay::OverlayKind::Settings);
                    nav.setting_path_key = Some(key.to_string());
                    *ctx.overlay = Some(nav);
                }
            }
            Effect::None
        }
        // LIST (cjk_priority): a bespoke inline reorder UI is over-engineering for a
        // rare Han-tiebreak setting, so open config.toml as TEXT (the same escape hatch
        // as the Advanced row) — the deliberate v2 scope call (see `SettingKind::List`).
        crate::settings::SettingKind::List => {
            *ctx.overlay = None;
            Effect::OpenSettings
        }
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

/// A folder navigator's Settings BREADCRUMB (`return_to` + `setting_path_key`),
/// SNAPSHOTTED off the current level so it can be re-applied to a freshly-rebuilt
/// one AFTER the previous overlay's borrow has ended (the borrow checker forbids
/// reading `prev` while writing `*ctx.overlay`). A navigator opened FROM a Settings
/// PATH row must keep writing THAT config key (and return to Settings) even as you
/// descend / ascend to find the folder — a rebuilt level starts with both fields
/// `None`, so without carrying them a descend/ascend would silently drop the
/// breadcrumb. A plain navigator (both already `None`) carries nothing — a no-op.
/// The ONE owner of this carry-forward: applied at the Project descend (Enter) seam
/// and the shared ascend (Backspace) seam — the only rebuilds a Settings-opened
/// navigator can reach (Browse / MoveDest are never opened from a Settings row).
struct Breadcrumb {
    return_to: Option<crate::overlay::OverlayKind>,
    setting_path_key: Option<String>,
}

impl Breadcrumb {
    /// Snapshot the breadcrumb off `ov` before it is replaced.
    fn of(ov: &OverlayState) -> Self {
        Self { return_to: ov.return_to, setting_path_key: ov.setting_path_key.clone() }
    }
    /// Re-apply the snapshot onto a rebuilt level.
    fn apply(self, next: &mut OverlayState) {
        next.return_to = self.return_to;
        next.setting_path_key = self.setting_path_key;
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
