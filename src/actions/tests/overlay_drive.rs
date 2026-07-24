//! Overlay-driving tests for browse/settings/rebind/asset-cleaner and
//! misc single-command effects (open settings, keep-version, convert line
//! endings, follow-link) -- split out of the former monolithic
//! `actions::tests` (2026-07 code-organization pass).

use super::super::*;
use crate::overlay::OverlayKind;
use super::{drive, drive_eff, settings_overlay, settings_drive};

#[test]
fn browse_path_helpers() {
    assert_eq!(join_browse(None, "docs"), "docs");
    assert_eq!(join_browse(Some("docs"), "guide.md"), "docs/guide.md");
    assert_eq!(join_browse(Some(""), "x"), "x");
    // ascend: root -> nothing; one level -> root; nested -> parent.
    assert_eq!(browse_parent(None), None);
    assert_eq!(browse_parent(Some("docs")), Some(None));
    assert_eq!(browse_parent(Some("docs/api")), Some(Some("docs".to_string())));
}

#[test]
fn caret_picker_previews_on_move_accepts_on_enter_reverts_on_cancel() {
    use crate::caret::CaretMode;
    // Serialize on the caret global lock (the preview mutates the process-global
    // caret mode, like the theme picker mutates the active theme).
    let _g = crate::testlock::serial();
    crate::caret::set_mode(CaretMode::Block);

    // SUMMON the caret picker (remembering Block as original), then NAVIGATE down:
    // the live preview applies the highlighted look to the process-global so the
    // document caret + the preview switch immediately.
    let mut overlay = Some(OverlayState::new_caret(CaretMode::Block));
    let mut accept = None;
    drive(&mut overlay, &mut accept, &Action::NextLine); // -> Morph
    assert_eq!(crate::caret::mode(), CaretMode::Morph);
    drive(&mut overlay, &mut accept, &Action::NextLine); // -> I-beam
    assert_eq!(crate::caret::mode(), CaretMode::Ibeam);

    // ENTER COMMITS: emits OverlayAccept(Caret, "I-beam") (the caller persists it)
    // and closes the picker; the previewed look stays active.
    drive(&mut overlay, &mut accept, &Action::Newline);
    assert!(overlay.is_none(), "Enter closes the caret picker");
    assert_eq!(accept, Some((OverlayKind::Caret, "I-beam".to_string())));
    assert_eq!(crate::caret::mode(), CaretMode::Ibeam);

    // CANCEL REVERTS: open again (original = I-beam now), preview Block, then Esc
    // restores the look active when it opened — and emits NO accept (no persist).
    crate::caret::set_mode(CaretMode::Ibeam);
    let mut overlay = Some(OverlayState::new_caret(CaretMode::Ibeam));
    let mut accept2 = None;
    drive(&mut overlay, &mut accept2, &Action::PreviousLine); // preview moves up
    drive(&mut overlay, &mut accept2, &Action::PreviousLine); // -> Block previewed
    assert_eq!(crate::caret::mode(), CaretMode::Block);
    drive(&mut overlay, &mut accept2, &Action::Cancel);
    assert!(overlay.is_none(), "Esc closes the caret picker");
    assert_eq!(accept2, None, "a revert must not persist (no accept emitted)");
    assert_eq!(crate::caret::mode(), CaretMode::Ibeam, "Esc reverts to the opened look");

    // Reset the global so later tests see the default.
    crate::caret::set_mode(CaretMode::Block);
}

/// THE BUG this round fixes: opening the Caret-style picker while riding AUTO
/// (no explicit override) and Cancelling — WITHOUT ever picking a different
/// look — must be a true no-op. Before the fix, `Cancel` unconditionally
/// `set_mode`'d `original_caret` (auto's momentary CONCRETE resolution),
/// silently converting "auto" into a permanent pin: merely glancing at the
/// picker and backing out would freeze the caret at that one theme's
/// font-derived look, so it stopped tracking LATER theme switches. Reproduced
/// end-to-end (headlessly, via `--keys`) in
/// `main::run::tests::replay_keys_caret_picker_cancel_from_auto_does_not_pin_it`;
/// this is the pure `apply_core`-level regression at its purest seam.
#[test]
fn caret_picker_cancel_from_auto_restores_auto_not_a_pin() {
    use crate::caret::CaretMode;
    let _g = crate::testlock::serial();
    let _t = crate::testlock::serial();

    // AUTO, on a PROPORTIONAL world: resolves Morph, but no override is set.
    crate::caret::clear_override();
    crate::theme::set_active_by_name("Gumtree").unwrap();
    assert!(crate::caret::is_auto());
    assert_eq!(crate::caret::mode(), CaretMode::Morph);

    // Open the picker (mirrors the real call site: `new_caret(caret::mode())`),
    // preview a different look, then Cancel WITHOUT committing.
    let mut overlay = Some(OverlayState::new_caret(crate::caret::mode()));
    let mut accept = None;
    drive(&mut overlay, &mut accept, &Action::NextLine); // preview -> I-beam
    assert_eq!(crate::caret::mode(), CaretMode::Ibeam);
    drive(&mut overlay, &mut accept, &Action::Cancel);
    assert!(overlay.is_none(), "Esc closes the caret picker");
    assert_eq!(accept, None, "a revert must not persist");

    // THE LAW: Cancel restored AUTO ITSELF, not a pin at Morph (what auto
    // happened to resolve to when the picker opened).
    assert!(crate::caret::is_auto(), "Cancel from auto must restore auto, not pin a concrete mode");
    assert_eq!(crate::caret::mode(), CaretMode::Morph, "Gumtree is still proportional");

    // PROOF it's genuinely auto, not merely coincidentally Morph: switching to
    // a MONO world now must track to Block, exactly as auto always would.
    crate::theme::set_active_by_name("Tawny").unwrap();
    assert_eq!(
        crate::caret::mode(),
        CaretMode::Block,
        "auto still tracks the theme after the picker was opened + cancelled"
    );

    // Restore.
    crate::caret::clear_override();
    crate::theme::set_active(crate::theme::DEFAULT_THEME);
}

#[test]
fn asset_cleaner_enter_arms_trash_and_keeps_the_picker_open() {
    // Build the ASSET CLEANER picker directly (the scan is unit-tested in
    // `assets.rs`); drive Enter through the real apply seam.
    let mk = |rel: &str| crate::assets::Orphan {
        rel: rel.to_string(),
        name: rel.rsplit('/').next().unwrap().to_string(),
        parent: rel.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default(),
        size: Some(10),
    };
    let mut overlay = Some(OverlayState::new_assets(vec![
        mk("assets/orphan-a.png"),
        mk("assets/orphan-b.png"),
    ]));
    // ENTER on the highlighted orphan ARMS TrashAsset with its root-relative path.
    let eff = drive_eff(&mut overlay, &Action::Newline);
    assert_eq!(eff, Effect::TrashAsset { rel: "assets/orphan-a.png".to_string() });
    // The picker STAYS OPEN — the core never closes it or removes the row (the App
    // does that only after a successful trash; a headless replay no-ops the trash).
    assert!(overlay.is_some(), "the asset cleaner stays open after Enter");
    assert_eq!(overlay.as_ref().unwrap().items.len(), 2, "the core leaves the list whole");
}

#[test]
fn asset_cleaner_enter_on_empty_state_is_a_calm_no_op() {
    let mut overlay = Some(OverlayState::new_assets(vec![]));
    // Empty list → nothing selected → Enter is Effect::None, picker stays open.
    assert_eq!(drive_eff(&mut overlay, &Action::Newline), Effect::None);
    assert!(overlay.is_some());
}

#[test]
fn rebind_menu_summon_capture_key_and_reset() {
    // SUMMON the rebind menu via the core (OpenKeybindings → make_overlay).
    let mut overlay = None;
    drive_eff(&mut overlay, &Action::OpenKeybindings);
    assert_eq!(overlay.as_ref().unwrap().kind, OverlayKind::Keybindings);
    // NAVIGATE: fuzzy-filter to "Undo".
    for c in "undo".chars() {
        drive_eff(&mut overlay, &Action::InsertChar(c));
    }
    assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("Undo"));
    // ENTER → ChooseMode (no commit yet).
    assert_eq!(drive_eff(&mut overlay, &Action::Newline), Effect::None);
    assert_eq!(
        overlay.as_ref().unwrap().capture.as_ref().unwrap().stage,
        crate::overlay::CaptureStage::ChooseMode
    );
    // ENTER again → begin recording (KEY mode, default).
    drive_eff(&mut overlay, &Action::Newline);
    assert_eq!(
        overlay.as_ref().unwrap().capture.as_ref().unwrap().stage,
        crate::overlay::CaptureStage::Recording
    );
    // CAPTURE a plain key 'j' → KEY mode finishes instantly → RebindCommit.
    let eff = drive_eff(&mut overlay, &Action::InsertChar('j'));
    assert_eq!(
        eff,
        Effect::RebindCommit {
            slug: "undo".to_string(),
            binding: "j".to_string(),
            confirmed: false
        }
    );

    // RESET: with no capture active, Delete on the highlighted command signals
    // a reset-to-default for that slug.
    let mut overlay = None;
    drive_eff(&mut overlay, &Action::OpenKeybindings);
    for c in "redo".chars() {
        drive_eff(&mut overlay, &Action::InsertChar(c));
    }
    let eff = drive_eff(&mut overlay, &Action::DeleteForward);
    assert_eq!(eff, Effect::RebindReset { slug: "redo".to_string() });
    // Esc closes the menu (generic intercept), capture stays absent.
    drive_eff(&mut overlay, &Action::Cancel);
    assert!(overlay.is_none(), "Esc closes the rebind menu");
}

#[test]
fn settings_toggle_row_signals_setting_toggle_and_keeps_menu_open() {
    // Row 0 is "Caret style" (a Picker); NextLine → row 1, "Page mode" (a Toggle).
    let mut overlay = Some(settings_overlay());
    settings_drive(&mut overlay, &Action::NextLine);
    assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("Page mode"));
    // Enter on a TOGGLE row signals SettingToggle for its config key and leaves
    // the menu OPEN (the App flips + persists + refreshes the cell).
    let eff = settings_drive(&mut overlay, &Action::Newline);
    assert_eq!(eff, Effect::SettingToggle { key: "page_mode".to_string() });
    assert_eq!(
        overlay.as_ref().map(|o| o.kind),
        Some(OverlayKind::Settings),
        "a toggle keeps the settings menu open"
    );
}

/// LAW TEST (the "settings toggle rows dispatch live" round): EVERY row the
/// corpus marks `SettingKind::Toggle` — enumerated straight off
/// `settings::visible_rows()`, never hand-copied, so a row added to the
/// corpus later is swept automatically — resolves through the REAL
/// `apply_core` seam (Enter on that exact row, selected directly by its own
/// corpus index rather than a fuzzy query, so an ambiguous filter can never
/// mis-select a neighbor) to `Effect::SettingToggle` carrying its OWN named
/// key. This is the "does Enter even signal the right thing" half of the
/// live dispatch chain the Keymap-row bug hid in — the row count assertion
/// keeps this test itself honest against the settings corpus (15 toggles;
/// "Date format" left the roster when it became a Picker). Companion:
/// `app::tests::every_settings_toggle_row_dispatches_live_and_flips_its_value`
/// (App-level: the signaled effect is actually APPLIED and the value cell
/// visibly flips — the "does the live door apply it" other half).
#[test]
fn every_settings_toggle_row_signals_its_own_setting_toggle_key() {
    let toggle_rows: Vec<&crate::settings::SettingRow> = crate::settings::visible_rows()
        .into_iter()
        .filter(|r| r.kind == crate::settings::SettingKind::Toggle)
        .collect();
    assert_eq!(
        toggle_rows.len(),
        15,
        "the toggle roster changed size — update this sweep deliberately"
    );
    for row in toggle_rows {
        let mut overlay = Some(settings_overlay());
        let idx = crate::settings::visible_rows()
            .iter()
            .position(|r| r.name == row.name)
            .unwrap();
        overlay.as_mut().unwrap().selected = idx;
        assert_eq!(
            overlay.as_ref().unwrap().selected_value(),
            Some(row.name),
            "index {idx} must select {:?} itself",
            row.name
        );
        let eff = settings_drive(&mut overlay, &Action::Newline);
        let want_key = crate::settings::toggle_key(row.name).expect("a Toggle row always has a key");
        assert_eq!(
            eff,
            Effect::SettingToggle { key: want_key.to_string() },
            "row {:?} did not signal its own toggle key",
            row.name
        );
        assert_eq!(
            overlay.as_ref().map(|o| o.kind),
            Some(OverlayKind::Settings),
            "a toggle keeps the settings menu open (row {:?})",
            row.name
        );
    }
}

// ── THE UNION ROUND: settings rows join the Cmd-P palette ──────────────────

/// Build a COMMAND PALETTE overlay with the settings corpus attached, exactly
/// as `overlay::build`'s real `OverlayKind::Command` arm does — the
/// PALETTE-filtered corpus (`settings::palette_names`/`palette_value_cells`),
/// which excludes any settings row covered by an available command
/// (`settings::COVERED_BY` — see the "one palette door per destination" fix).
fn command_overlay_with_settings() -> OverlayState {
    let mut ov = OverlayState::new_command(
        crate::commands::visible_names(),
        crate::commands::visible_effective_bindings(&[], &[]),
        // No daemon waiter in this fixture: matches the real arm's default
        // (`BuildCtx::has_waiter: false`) for every non-live caller.
        crate::commands::visible_hidden_mask(false),
    );
    ov.attach_settings_rows(
        crate::settings::palette_names(),
        crate::settings::palette_value_cells(&Default::default()),
    );
    ov
}

/// A make_overlay for the union-round tests: the Command palette (settings
/// attached) plus the sub-pickers a settings Picker/Submenu row can open.
fn command_drive(overlay: &mut Option<OverlayState>, action: &Action) -> Effect {
    let mut buffer = Buffer::scratch();
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut make_overlay = |k: OverlayKind| match k {
        OverlayKind::Command => Some(command_overlay_with_settings()),
        OverlayKind::Theme => Some(OverlayState::new_theme(
            crate::theme::THEMES.iter().map(|t| t.name.to_string()).collect(),
            crate::theme::active_index(),
        )),
        OverlayKind::Keybindings => Some(OverlayState::new_keybindings(
            crate::commands::visible_names(),
            crate::commands::visible_effective_bindings(&[], &[]),
        )),
        OverlayKind::CjkLang => Some(OverlayState::new_cjk_lang(
            crate::frontmatter::cjk_priority().first().copied().unwrap_or(crate::frontmatter::Lang::Ja),
        )),
        _ => None,
    };
    let mut browse_to = |_k: OverlayKind, _r: Option<String>| None;
    let mut ctx = ActionCtx {
        buffer: &mut buffer,
        shift_selecting: &mut shift,
        zoom: &mut zoom,
        search: &mut search,
        scroll_page_lines: 1,
        overlay,
        make_overlay: &mut make_overlay,
        browse_to: &mut browse_to,
        oracle: None,
    };
    apply_core(&mut ctx, action, false)
}

/// The palette's corpus is the UNION of commands + NON-COVERED settings — a
/// settings row with no command twin (e.g. "Keymap") is fuzzy-findable there,
/// wears the `§ ` marker glyph in its display text, and shows its CURRENT VALUE
/// in the secondary (binding) column exactly like the Settings menu itself. A
/// COVERED row (e.g. "Theme" — see `settings::COVERED_BY`) is excluded: its
/// covering command is the one door.
#[test]
fn union_palette_lists_settings_rows_with_marker_and_current_value() {
    let mut ov = command_overlay_with_settings();
    // The full corpus is commands ++ palette-visible (non-covered) settings.
    assert_eq!(
        ov.rows.len(),
        crate::commands::visible_names().len() + crate::settings::palette_names().len()
    );
    for ch in ['k', 'e', 'y', 'm', 'a', 'p'] {
        ov.push(ch);
    }
    assert!(
        ov.item_strings().iter().any(|s| s == "§ Keymap"),
        "typing \"keymap\" should surface the marked settings row: {:?}",
        ov.item_strings()
    );
    let idx = ov.item_strings().iter().position(|s| s == "§ Keymap").unwrap();
    assert_eq!(
        ov.item_bindings()[idx],
        crate::settings::value_for(
            crate::settings::SETTINGS.iter().find(|r| r.name == "Keymap").unwrap(),
            &Default::default()
        ),
        "the settings row's secondary column carries its CURRENT VALUE"
    );
}

/// DISPATCH PARITY BY CONSTRUCTION: Enter on a settings TOGGLE row reached via
/// the palette signals the SAME `Effect::SettingToggle{key}` the Settings menu's
/// own accept signals — but, matching a COMMAND row's own "running it closes
/// the palette" convention, the palette closes outright (unlike the Settings
/// menu, which stays open for more toggling). Uses "Reduce motion" — a Toggle
/// row with NO covering command (see `settings::COVERED_BY`), so it's still
/// palette-visible; "Page mode" (which IS covered by "Toggle page mode") is no
/// longer reachable this way — see `covered_rows_are_excluded_from_the_palette_*`
/// in `settings.rs`.
#[test]
fn palette_settings_toggle_row_signals_setting_toggle_and_closes_the_palette() {
    let mut ov = Some(command_overlay_with_settings());
    let idx = ov
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .position(|r| r.accept == "Reduce motion")
        .unwrap();
    ov.as_mut().unwrap().selected =
        ov.as_ref().unwrap().items.iter().position(|&i| i == idx).unwrap();
    assert_eq!(ov.as_ref().unwrap().selected_value(), Some("Reduce motion"));
    let eff = command_drive(&mut ov, &Action::Newline);
    assert_eq!(eff, Effect::SettingToggle { key: "reduce_motion".to_string() });
    assert!(ov.is_none(), "activating a settings row closes the palette");
}

/// A settings PICKER row with NO covering command ("Ambiguous CJK reads as" —
/// see `settings::COVERED_BY`) reached via the palette opens the SAME sub-picker
/// the Settings menu opens — with the breadcrumb set to `Command` (not
/// `Settings`), so canceling it returns to the PALETTE. "Theme" (which IS
/// covered by "Switch theme…") is no longer reachable this way — the exact fix
/// for the reported duplication.
#[test]
fn palette_settings_picker_row_opens_sub_picker_with_command_breadcrumb() {
    let mut ov = Some(command_overlay_with_settings());
    let idx = ov
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .position(|r| r.accept == "Ambiguous CJK reads as")
        .unwrap();
    ov.as_mut().unwrap().selected =
        ov.as_ref().unwrap().items.iter().position(|&i| i == idx).unwrap();
    let eff = command_drive(&mut ov, &Action::Newline);
    assert_eq!(eff, Effect::None);
    let next = ov.as_ref().expect("a sub-picker opened");
    assert_eq!(next.kind, OverlayKind::CjkLang);
    assert_eq!(next.return_to, Some(OverlayKind::Command), "breadcrumb points back to the palette");
}

/// A COVERED settings row (e.g. "Theme") is simply ABSENT from the palette
/// union's corpus — its covering command ("Switch theme…") is the one door.
/// This is the literal user-reported bug's regression guard.
#[test]
fn covered_settings_row_is_absent_from_the_palette_corpus() {
    let ov = command_overlay_with_settings();
    for (row_name, cmd_name) in crate::settings::COVERED_BY {
        let row_count = ov.rows.iter().filter(|r| r.accept.as_str() == *row_name).count();
        if row_name == cmd_name {
            assert_eq!(row_count, 1, "same-named command/settings doors must collapse to one row");
        } else {
            assert_eq!(
                row_count, 0,
                "{row_name:?} must not appear in the palette corpus — {cmd_name:?} covers it"
            );
        }
        assert!(
            ov.rows.iter().any(|r| r.accept == *cmd_name),
            "{cmd_name:?} must still be the one door in the palette corpus"
        );
    }
}

/// An ORDINARY command row (not a setting) is UNCHANGED by the union: Enter
/// still runs it via `Effect::RunAction`.
#[test]
fn union_palette_ordinary_command_row_still_runs() {
    let mut ov = Some(command_overlay_with_settings());
    let idx = ov.as_ref().unwrap().rows.iter().position(|r| r.accept == "Save").unwrap();
    ov.as_mut().unwrap().selected =
        ov.as_ref().unwrap().items.iter().position(|&i| i == idx).unwrap();
    let eff = command_drive(&mut ov, &Action::Newline);
    assert_eq!(eff, Effect::RunAction(Action::Save));
}

#[test]
fn settings_action_row_opens_config_as_text_and_closes() {
    // Fuzzy-filter to the Advanced "Edit config as text" ACTION row.
    let mut overlay = Some(settings_overlay());
    for c in "edit config".chars() {
        settings_drive(&mut overlay, &Action::InsertChar(c));
    }
    assert_eq!(
        overlay.as_ref().unwrap().selected_value(),
        Some("Edit config as text")
    );
    // Enter emits OpenSettings (open config.toml) and CLOSES the menu.
    let eff = settings_drive(&mut overlay, &Action::Newline);
    assert_eq!(eff, Effect::OpenSettings);
    assert!(overlay.is_none(), "the action row closes the menu");
}

#[test]
fn settings_report_problem_row_reuses_the_report_effect_and_closes() {
    let mut overlay = Some(settings_overlay());
    for c in "report problem".chars() {
        settings_drive(&mut overlay, &Action::InsertChar(c));
    }
    assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("Report a Problem"));
    assert_eq!(settings_drive(&mut overlay, &Action::Newline), Effect::ReportProblem);
    assert!(overlay.is_none());
}

#[test]
fn settings_picker_row_opens_sub_picker_with_breadcrumb_then_returns() {
    // Row 0 "Caret style" is a Picker → Enter swaps to the Caret sub-picker,
    // stamping a return_to = Settings breadcrumb (single-level).
    let mut overlay = Some(settings_overlay());
    let eff = settings_drive(&mut overlay, &Action::Newline);
    assert_eq!(eff, Effect::None);
    {
        let ov = overlay.as_ref().unwrap();
        assert_eq!(ov.kind, OverlayKind::Caret, "opened the caret sub-picker");
        assert_eq!(
            ov.return_to,
            Some(OverlayKind::Settings),
            "the sub-picker remembers its way back to Settings"
        );
    }
    // Esc (cancel) on the sub-picker RE-SUMMONS Settings via the breadcrumb —
    // NOT close-to-buffer — and the re-summoned parent carries no breadcrumb.
    settings_drive(&mut overlay, &Action::Cancel);
    let ov = overlay.as_ref().expect("returned to Settings, did not close");
    assert_eq!(ov.kind, OverlayKind::Settings);
    assert_eq!(ov.return_to, None, "single-level: no N-deep stack");
}

#[test]
fn settings_value_row_arms_inline_edit_then_commits_typed_value() {
    // Fuzzy-filter to "Page width (prose)" (a Value row).
    let mut overlay = Some(settings_overlay());
    for c in "prose".chars() {
        settings_drive(&mut overlay, &Action::InsertChar(c));
    }
    assert_eq!(
        overlay.as_ref().unwrap().selected_value(),
        Some("Page width (prose)")
    );
    // Enter ARMS the inline edit sub-state (menu stays open, no effect yet).
    let eff = settings_drive(&mut overlay, &Action::Newline);
    assert_eq!(eff, Effect::None);
    assert!(
        overlay.as_ref().unwrap().value_edit.is_some(),
        "a Value row arms an inline edit"
    );
    // Clear the seeded value, then type a fresh number — routed into the EDIT
    // (value_edit), never the query filter.
    for _ in 0..4 {
        settings_drive(&mut overlay, &Action::DeleteBackward);
    }
    for c in "45".chars() {
        settings_drive(&mut overlay, &Action::InsertChar(c));
    }
    assert_eq!(
        overlay.as_ref().unwrap().value_edit.as_ref().unwrap().input,
        "45"
    );
    // Enter COMMITS: signals SettingValueCommit(named key, typed value), clears the
    // sub-state, keeps the menu open.
    let eff = settings_drive(&mut overlay, &Action::Newline);
    assert_eq!(
        eff,
        Effect::SettingValueCommit {
            key: "page_width_prose".to_string(),
            value: "45".to_string()
        }
    );
    assert!(
        overlay.as_ref().unwrap().value_edit.is_none(),
        "commit clears the inline edit"
    );
    assert_eq!(
        overlay.as_ref().map(|o| o.kind),
        Some(OverlayKind::Settings),
        "the menu stays open after a value commit"
    );
}

#[test]
fn settings_value_edit_cancel_restores_the_cell_and_keeps_menu_open() {
    let mut overlay = Some(settings_overlay());
    for c in "prose".chars() {
        settings_drive(&mut overlay, &Action::InsertChar(c));
    }
    let ci = overlay.as_ref().unwrap().selected_corpus_index().unwrap();
    let orig_cell = overlay.as_ref().unwrap().rows[ci].secondary.clone();
    settings_drive(&mut overlay, &Action::Newline); // arm
    for c in "999".chars() {
        settings_drive(&mut overlay, &Action::InsertChar(c));
    }
    assert_ne!(
        overlay.as_ref().unwrap().rows[ci].secondary,
        orig_cell,
        "the row's cell shows the live typed value"
    );
    // Esc CANCELS: drop the sub-state and revert the cell to its original value.
    let eff = settings_drive(&mut overlay, &Action::Cancel);
    assert_eq!(eff, Effect::None);
    assert!(overlay.as_ref().unwrap().value_edit.is_none());
    assert_eq!(
        overlay.as_ref().unwrap().rows[ci].secondary,
        orig_cell,
        "cancel restores the cell"
    );
    assert_eq!(
        overlay.as_ref().map(|o| o.kind),
        Some(OverlayKind::Settings),
        "cancel keeps the settings menu open (does not close to the buffer)"
    );
}

#[test]
fn settings_path_row_opens_navigator_with_breadcrumb_then_picks_the_named_key() {
    // Fuzzy-filter to "Notes folder" (a Path row).
    let mut overlay = Some(settings_overlay());
    for c in "notes".chars() {
        settings_drive(&mut overlay, &Action::InsertChar(c));
    }
    assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("Notes folder"));
    // Enter opens the folder NAVIGATOR (Project), with a Settings breadcrumb AND
    // the named config key stamped so its accept writes THAT key.
    let eff = settings_drive(&mut overlay, &Action::Newline);
    assert_eq!(eff, Effect::None);
    {
        let ov = overlay.as_ref().unwrap();
        assert_eq!(ov.kind, OverlayKind::Project, "opened the folder navigator");
        assert_eq!(
            ov.return_to,
            Some(OverlayKind::Settings),
            "breadcrumb back to Settings"
        );
        assert_eq!(
            ov.setting_path_key.as_deref(),
            Some("notes_root"),
            "stamped the named path key"
        );
    }
    // Now that Project FACETS, Enter on a FOLDER descends; the pick affordance is
    // the synthetic "." (select-this-folder) row. Up moves onto "." and Enter
    // there signals SettingPathPick for that key (the App writes it), returning to
    // Settings via the breadcrumb.
    settings_drive(&mut overlay, &Action::PreviousLine);
    assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("."));
    let eff = settings_drive(&mut overlay, &Action::Newline);
    assert!(
        matches!(&eff, Effect::SettingPathPick { key, .. } if key == "notes_root"),
        "the navigator accept writes the named key, got {eff:?}"
    );
    assert_eq!(
        overlay.as_ref().map(|o| o.kind),
        Some(OverlayKind::Settings),
        "the navigator returns to Settings via the breadcrumb"
    );
}

#[test]
fn settings_path_navigator_keeps_breadcrumb_across_descend() {
    // Open the folder navigator from the "Notes folder" Path row (stamps the key +
    // breadcrumb), then DESCEND into a folder (Enter, now that Project facets).
    // The breadcrumb must survive the rebuild so the eventual "." pick still
    // writes the named key and returns to Settings.
    let mut overlay = Some(settings_overlay());
    for c in "notes".chars() {
        settings_drive(&mut overlay, &Action::InsertChar(c));
    }
    settings_drive(&mut overlay, &Action::Newline); // opens Project w/ key+breadcrumb
    assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("sub"), "on a folder");
    settings_drive(&mut overlay, &Action::Newline); // Enter DESCENDS (rebuilds the level)
    let ov = overlay.as_ref().unwrap();
    assert_eq!(ov.kind, OverlayKind::Project, "still the navigator after descend");
    assert_eq!(
        ov.setting_path_key.as_deref(),
        Some("notes_root"),
        "the named path key survives a descend"
    );
    assert_eq!(
        ov.return_to,
        Some(OverlayKind::Settings),
        "the Settings breadcrumb survives a descend"
    );
}

#[test]
fn settings_cjk_row_opens_language_picker_and_promotes_on_commit() {
    let _g = crate::testlock::serial();
    crate::frontmatter::set_cjk_priority(&crate::frontmatter::DEFAULT_CJK_PRIORITY);

    // "Ambiguous CJK reads as" is now a PICKER row (the List row grown up).
    let mut overlay = Some(settings_overlay());
    for c in "ambiguous".chars() {
        settings_drive(&mut overlay, &Action::InsertChar(c));
    }
    assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("Ambiguous CJK reads as"));

    // Enter opens the CjkLang sub-picker, breadcrumbed back to Settings — the
    // exact same shape as the Caret/Theme/Dictionary Picker rows.
    let eff = settings_drive(&mut overlay, &Action::Newline);
    assert_eq!(eff, Effect::None);
    {
        let ov = overlay.as_ref().unwrap();
        assert_eq!(ov.kind, OverlayKind::CjkLang, "opened the CJK language sub-picker");
        assert_eq!(ov.return_to, Some(OverlayKind::Settings));
        // Pre-selected on the current front language ("Japanese", the default).
        assert_eq!(ov.selected_value(), Some("Japanese"));
    }

    // Move to "Korean" and commit: PROMOTES it to the front of the live
    // ladder (core-level — both live App and headless replay observe this)
    // and pops back to Settings via the breadcrumb.
    settings_drive(&mut overlay, &Action::NextLine);
    settings_drive(&mut overlay, &Action::NextLine);
    settings_drive(&mut overlay, &Action::NextLine);
    assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("Korean"));
    let eff = settings_drive(&mut overlay, &Action::Newline);
    assert_eq!(
        eff,
        Effect::OverlayAccept(OverlayKind::CjkLang, "ko".to_string())
    );
    assert_eq!(
        crate::frontmatter::cjk_priority(),
        vec![
            crate::frontmatter::Lang::Ko,
            crate::frontmatter::Lang::Ja,
            crate::frontmatter::Lang::ZhHans,
            crate::frontmatter::Lang::ZhHant,
        ],
        "Korean promoted to front, rest keep relative order"
    );
    let ov = overlay.as_ref().expect("returned to Settings, did not close");
    assert_eq!(ov.kind, OverlayKind::Settings);
    assert_eq!(ov.return_to, None, "single-level: no N-deep stack");
    // The re-summoned Settings menu's value cell is FRESH (reads the live
    // global, just promoted).
    assert_eq!(
        crate::settings::value_for(
            &crate::settings::SETTINGS
                .iter()
                .find(|r| r.name == "Ambiguous CJK reads as")
                .unwrap(),
            &Default::default()
        ),
        "Korean"
    );

    // Cleanup for other tests.
    crate::frontmatter::set_cjk_priority(&crate::frontmatter::DEFAULT_CJK_PRIORITY);
}

#[test]
fn open_settings_signals_caller() {
    // OpenSettings is a pure signal: it returns Effect::OpenSettings for the
    // caller to open the config file (no buffer/overlay change in the core).
    let mut buffer = Buffer::scratch();
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut overlay = None;
    let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
    let mut browse_to =
        |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
    let mut ctx = ActionCtx {
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
    let effect = apply_core(&mut ctx, &Action::OpenSettings, false);
    assert_eq!(effect, Effect::OpenSettings, "OpenSettings must signal the caller");
    assert!(overlay.is_none(), "OpenSettings opens no overlay");
}

#[test]
fn keep_version_opens_the_naming_minibuffer_and_enter_commits_the_name() {
    // NAMED SAVE POINTS: "Keep version…" summons the naming MINIBUFFER
    // (`OverlayKind::KeepName`, the Rename/InsertLink shape). Typing builds the
    // optional name in corpus[0]; Enter closes the overlay and signals
    // Effect::KeepVersion { name: Some(..) } for the live App to pin+name the
    // snapshot. The buffer is never touched (the pin is store-side, not an edit).
    let mut buffer = Buffer::from_str("keep me\n");
    let before = buffer.text();
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut overlay = None;
    let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
    let mut browse_to =
        |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
    let mut ctx = ActionCtx {
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
    let effect = apply_core(&mut ctx, &Action::KeepVersion, false);
    assert_eq!(effect, Effect::None, "the summon itself signals nothing yet");
    {
        let ov = ctx.overlay.as_ref().expect("Keep version… opens the naming minibuffer");
        assert_eq!(ov.kind, OverlayKind::KeepName);
        assert!(ov.keep_edit.is_some(), "the modal keep_edit sub-state is armed at build");
        assert_eq!(ov.accepts(), vec![""], "the single row opens empty (no old name)");
        assert_eq!(
            ov.foot_hint(),
            "name this version:    Enter keep   Esc cancel",
            "the prompt rides the same foot_hint seam Rename/InsertLink use"
        );
    }
    // Type a name, then Enter: the intercept closes the overlay and commits.
    for c in "draft A".chars() {
        assert_eq!(apply_core(&mut ctx, &Action::InsertChar(c), false), Effect::None);
    }
    let effect = apply_core(&mut ctx, &Action::Newline, false);
    assert_eq!(
        effect,
        Effect::KeepVersion { name: Some("draft A".into()) },
        "Enter commits the typed name"
    );
    assert!(overlay.is_none(), "commit closes the minibuffer");
    assert_eq!(buffer.text(), before, "pinning never edits the buffer");
    assert!(!buffer.can_undo(), "a pin is not an undoable edit");
}

#[test]
fn keep_version_blank_enter_is_the_plain_keep_and_esc_cancels() {
    // Zero friction preserved: Enter on the EMPTY prompt commits the plain
    // (nameless) keep — Effect::KeepVersion { name: None }, exactly today's
    // behavior one Enter later. Esc cancels with NOTHING signalled.
    let mut buffer = Buffer::from_str("keep me\n");
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut overlay = None;
    let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
    let mut browse_to =
        |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
    let mut ctx = ActionCtx {
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
    // Blank Enter → the plain keep (name: None). A whitespace-only name too.
    apply_core(&mut ctx, &Action::KeepVersion, false);
    apply_core(&mut ctx, &Action::InsertChar(' '), false);
    let effect = apply_core(&mut ctx, &Action::Newline, false);
    assert_eq!(
        effect,
        Effect::KeepVersion { name: None },
        "a blank (whitespace-only) Enter is the plain, nameless keep"
    );
    assert!(ctx.overlay.is_none());
    // Esc → cancels: the overlay closes and NOTHING is signalled.
    apply_core(&mut ctx, &Action::KeepVersion, false);
    apply_core(&mut ctx, &Action::InsertChar('x'), false);
    let effect = apply_core(&mut ctx, &Action::Cancel, false);
    assert_eq!(effect, Effect::None, "Esc keeps nothing");
    assert!(ctx.overlay.is_none(), "Esc closes the minibuffer outright");
}

#[test]
fn convert_line_endings_toggles_the_buffer_eol_as_metadata() {
    use crate::buffer::Eol;
    // The palette "Line endings…" command routes Action::ConvertLineEndings
    // through the SAME apply_core seam a key/menu invocation uses. A fresh buffer
    // is LF; each dispatch flips the on-disk ending (LF <-> CRLF) WITHOUT touching
    // the rope (always pure `\n`), so the change is document METADATA — it marks
    // the buffer dirty + bumps `version` (so autosave rewrites) but is NOT an
    // undoable edit (Cmd-Z does not restore it — the VS Code model).
    let mut buffer = Buffer::from_str("alpha\nbeta\n");
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut overlay = None;
    let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
    let mut browse_to =
        |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
    let text_before = buffer.text();
    assert_eq!(buffer.eol(), Eol::Lf, "a fresh buffer defaults to LF");
    assert!(!buffer.can_undo(), "no edit yet, nothing to undo");

    let mut ctx = ActionCtx {
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
    let version_before = ctx.buffer.version();
    let eff = apply_core(&mut ctx, &Action::ConvertLineEndings, false);
    assert_eq!(eff, Effect::None, "convert is a plain metadata flip, no effect");
    assert_eq!(ctx.buffer.eol(), Eol::Crlf, "first toggle: LF -> CRLF");
    assert_ne!(ctx.buffer.version(), version_before, "a real switch bumps version");
    assert_eq!(ctx.buffer.text(), text_before, "the rope is untouched (still pure \\n)");
    assert!(!ctx.buffer.can_undo(), "EOL is metadata, NOT an undoable edit");

    // A second dispatch flips back to LF (the toggle is total over the two endings).
    apply_core(&mut ctx, &Action::ConvertLineEndings, false);
    assert_eq!(ctx.buffer.eol(), Eol::Lf, "second toggle: CRLF -> LF");
}

#[test]
fn follow_link_signals_the_url_only_when_the_caret_is_inside_a_link() {
    // Action::FollowLink routes through the SAME apply_core seam a key/palette/menu
    // invocation uses. When the caret sits inside a markdown link the pure core
    // extracts its URL and signals `Effect::FollowLink(url)` for the caller to open
    // in the browser (a LIVE-App-only handoff; the headless replay no-ops the
    // effect, so a capture never spawns a browser). A caret OUTSIDE every link is a
    // calm no-op (`Effect::None`) — the core never opens anything itself.
    let mut buffer = Buffer::from_str("see [the essay](http://x/y) now\n");
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut overlay = None;
    let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
    let mut browse_to =
        |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
    // Caret inside the link text `essay`.
    let inside = buffer.text().find("essay").unwrap() + 1;
    buffer.set_cursor(inside);
    let mut ctx = ActionCtx {
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
    let eff = apply_core(&mut ctx, &Action::FollowLink, false);
    assert_eq!(
        eff,
        Effect::FollowLink("http://x/y".to_string()),
        "caret in a link signals its URL"
    );
    // The core mutated nothing (following a link is not an edit).
    assert!(!ctx.buffer.can_undo(), "FollowLink is not an edit");

    // Caret in the leading prose (byte 1) — outside every link — is a no-op.
    ctx.buffer.set_cursor(1);
    assert_eq!(
        apply_core(&mut ctx, &Action::FollowLink, false),
        Effect::None,
        "caret outside a link is the calm no-op"
    );
}

/// WORD-OPS ROUND (b) — the END-TO-END routing proof: ⌥⌫ (`DeleteWordBackward`)
/// drives a WHOLE-word delete of the palette's fuzzy query through the real
/// `apply_core` → `overlay_intercept` seam, while plain ⌫ (`DeleteBackward`)
/// still removes a single char. This pins the match-arm SPLIT the round added
/// (before it, both actions shared one arm that popped a single char).
#[test]
fn palette_query_word_delete_routes_through_apply_core() {
    let names = crate::commands::names();
    let hidden = vec![false; names.len()];
    let mut overlay =
        Some(OverlayState::new_command(names, crate::commands::bindings(), hidden));
    for c in "foo bar baz".chars() {
        drive_eff(&mut overlay, &Action::InsertChar(c));
    }
    assert_eq!(overlay.as_ref().unwrap().query, "foo bar baz");
    // ⌥⌫ / C-⌫ resolve to DeleteWordBackward: a whole trailing word goes.
    drive_eff(&mut overlay, &Action::DeleteWordBackward);
    assert_eq!(overlay.as_ref().unwrap().query, "foo bar ");
    // Plain ⌫ (DeleteBackward) still removes exactly one char.
    drive_eff(&mut overlay, &Action::DeleteBackward);
    assert_eq!(overlay.as_ref().unwrap().query, "foo bar");
    drive_eff(&mut overlay, &Action::DeleteWordBackward);
    assert_eq!(overlay.as_ref().unwrap().query, "foo ");
}

/// ITEM 10 — END-TO-END: `Action::ForwardWord`/`BackwardWord` move the
/// PALETTE QUERY's caret through the real `apply_core` → `overlay_intercept`
/// seam (previously a swallowed no-op), while plain `Action::NextLine` /
/// `PreviousLine` (Up/Down — the list-move actions every kind shares) still
/// move the SELECTED ROW, never the query caret. Proves the routing this item
/// added without disturbing the pre-existing list-navigation arms.
#[test]
fn palette_query_word_motion_routes_through_apply_core_list_move_untouched() {
    let names = crate::commands::names();
    let hidden = vec![false; names.len()];
    let mut overlay =
        Some(OverlayState::new_command(names, crate::commands::bindings(), hidden));
    for c in "foo bar".chars() {
        drive_eff(&mut overlay, &Action::InsertChar(c));
    }
    assert_eq!(overlay.as_ref().unwrap().query.caret(), 7, "caret sits at the end after typing");
    drive_eff(&mut overlay, &Action::BackwardWord);
    assert_eq!(overlay.as_ref().unwrap().query.caret(), 4, "word_left lands before \"bar\"");
    // A subsequent insert splices at the mid-string caret, not the end.
    drive_eff(&mut overlay, &Action::InsertChar('X'));
    assert_eq!(overlay.as_ref().unwrap().query, "foo Xbar");
    drive_eff(&mut overlay, &Action::ForwardWord);
    assert_eq!(
        overlay.as_ref().unwrap().query.caret(),
        overlay.as_ref().unwrap().query.text().chars().count(),
        "word_right walks back to the end"
    );
    // Clear the query back to empty (the full command list ranks, > 1 row) so
    // NextLine has somewhere real to move — proving plain list-move is
    // UNCHANGED by item 10: it moves the selection, never the query caret.
    for _ in 0..8 {
        drive_eff(&mut overlay, &Action::DeleteBackward);
    }
    assert_eq!(overlay.as_ref().unwrap().query, "");
    let caret_before = overlay.as_ref().unwrap().query.caret();
    let selected_before = overlay.as_ref().unwrap().selected;
    drive_eff(&mut overlay, &Action::NextLine);
    assert_eq!(overlay.as_ref().unwrap().query.caret(), caret_before, "list move never touches the caret");
    assert_ne!(
        overlay.as_ref().unwrap().selected,
        selected_before,
        "NextLine still moves the selection (more than one command exists)"
    );
}
