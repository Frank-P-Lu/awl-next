    use super::*;
    use crate::overlay::{OverlayKind, OverlayState};

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

    /// A tiny in-memory tree for the browse navigator: root has `docs/` (dir) and
    /// `README.md` (file); `docs/` has `guide.md` (file) and `api/` (dir). The
    /// `kind` is threaded through so MoveDest rebuilds stay MoveDest.
    fn browse_level(kind: OverlayKind, rel: Option<String>) -> Option<OverlayState> {
        let (corpus, git, is_dir): (Vec<String>, Vec<bool>, Vec<bool>) = match rel.as_deref() {
            None => (
                vec!["docs".into(), "README.md".into()],
                vec![false, false],
                vec![true, false],
            ),
            Some("docs") => (
                vec!["api".into(), "guide.md".into()],
                vec![false, false],
                vec![true, false],
            ),
            _ => (vec![], vec![], vec![]),
        };
        Some(OverlayState::new_marked(
            kind, corpus, git, is_dir, vec![], vec![], rel,
        ))
    }

    /// Drive a single action through `apply_core` with a browse_to backed by
    /// `browse_level`, returning the resulting (overlay, accept).
    fn drive(
        overlay: &mut Option<OverlayState>,
        accept: &mut Option<(OverlayKind, String)>,
        action: &Action,
    ) {
        let mut buffer = Buffer::scratch();
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut make_overlay = |k: OverlayKind| match k {
            OverlayKind::Command => Some(OverlayState::new_command(
                crate::commands::names(),
                crate::commands::bindings(),
            )),
            _ => None,
        };
        let mut browse_to = |kind: OverlayKind, rel: Option<String>| browse_level(kind, rel);
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
        // Mirror the old `overlay_accept` out-param: an accept effect writes the
        // chosen value into `accept`, accumulating across calls like before.
        if let Effect::OverlayAccept(kind, val) = apply_core(&mut ctx, action, false) {
            *accept = Some((kind, val));
        }
    }

    #[test]
    fn caret_picker_previews_on_move_accepts_on_enter_reverts_on_cancel() {
        use crate::caret::CaretMode;
        // Serialize on the caret global lock (the preview mutates the process-global
        // caret mode, like the theme picker mutates the active theme).
        let _g = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    /// Like [`drive`], but also returns the palette's `run_action` out-param so a
    /// test can assert which command Enter dispatched.
    fn drive_run(
        overlay: &mut Option<OverlayState>,
        accept: &mut Option<(OverlayKind, String)>,
        action: &Action,
    ) -> Option<Action> {
        let mut buffer = Buffer::scratch();
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut make_overlay = |k: OverlayKind| match k {
            OverlayKind::Command => Some(OverlayState::new_command(
                crate::commands::names(),
                crate::commands::bindings(),
            )),
            _ => None,
        };
        let mut browse_to = |kind: OverlayKind, rel: Option<String>| browse_level(kind, rel);
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
        // Surface BOTH the palette's run-on-Enter (returned) and any accept value
        // (mirrored into `accept`), matching the former two out-params.
        match apply_core(&mut ctx, action, false) {
            Effect::RunAction(a) => Some(a),
            Effect::OverlayAccept(kind, val) => {
                *accept = Some((kind, val));
                None
            }
            _ => None,
        }
    }

    /// Drive one action against a mutable overlay through `apply_core`, returning the
    /// raw [`Effect`] — for the rebind-menu flow assertions.
    fn drive_eff(overlay: &mut Option<OverlayState>, action: &Action) -> Effect {
        let mut buffer = Buffer::scratch();
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut make_overlay = |k: OverlayKind| match k {
            OverlayKind::Keybindings => Some(OverlayState::new_keybindings(
                crate::commands::names(),
                crate::commands::effective_bindings(&[]),
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

    /// A fresh SETTINGS overlay (table-order corpus + value cells), for the
    /// interaction tests below. Selection lands on row 0 ("Caret style", a Picker).
    fn settings_overlay() -> OverlayState {
        let mut ov = OverlayState::new(
            OverlayKind::Settings,
            crate::settings::names(),
            vec![],
            vec![],
        );
        ov.bindings = crate::settings::value_cells(&Default::default());
        ov
    }

    /// A make_overlay for the settings interaction tests: re-summons Settings (the
    /// breadcrumb target) and builds the Caret sub-picker; everything else is None.
    fn settings_drive(overlay: &mut Option<OverlayState>, action: &Action) -> Effect {
        let mut buffer = Buffer::scratch();
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut make_overlay = |k: OverlayKind| match k {
            OverlayKind::Settings => Some(settings_overlay()),
            OverlayKind::Caret => Some(OverlayState::new_caret(crate::caret::mode())),
            _ => None,
        };
        // A Path row routes to the Project folder navigator; hand back a small one.
        let mut browse_to = |k: OverlayKind, _r: Option<String>| match k {
            OverlayKind::Project => Some(OverlayState::new_project(
                "/work".to_string(),
                vec![("sub".to_string(), false)],
                &[],
            )),
            _ => None,
        };
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
        let orig_cell = overlay.as_ref().unwrap().bindings[ci].clone();
        settings_drive(&mut overlay, &Action::Newline); // arm
        for c in "999".chars() {
            settings_drive(&mut overlay, &Action::InsertChar(c));
        }
        assert_ne!(
            overlay.as_ref().unwrap().bindings[ci],
            orig_cell,
            "the row's cell shows the live typed value"
        );
        // Esc CANCELS: drop the sub-state and revert the cell to its original value.
        let eff = settings_drive(&mut overlay, &Action::Cancel);
        assert_eq!(eff, Effect::None);
        assert!(overlay.as_ref().unwrap().value_edit.is_none());
        assert_eq!(
            overlay.as_ref().unwrap().bindings[ci],
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
        // Fuzzy-filter to "Notes root" (a Path row).
        let mut overlay = Some(settings_overlay());
        for c in "notes".chars() {
            settings_drive(&mut overlay, &Action::InsertChar(c));
        }
        assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("Notes root"));
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
        // Open the folder navigator from the "Notes root" Path row (stamps the key +
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
    fn settings_cjk_list_row_opens_config_as_text_and_closes() {
        let mut overlay = Some(settings_overlay());
        for c in "cjk".chars() {
            settings_drive(&mut overlay, &Action::InsertChar(c));
        }
        assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("CJK priority"));
        // Enter opens config.toml as TEXT (the deliberate v2 scope call) and closes.
        let eff = settings_drive(&mut overlay, &Action::Newline);
        assert_eq!(eff, Effect::OpenSettings);
        assert!(overlay.is_none(), "the cjk list row closes the menu");
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
    fn keep_version_signals_the_caller_without_touching_the_buffer() {
        // THE CONSCIOUS MARK: "Keep This Version" is a pure signal — the core can't
        // reach the history store (no fs/config/path), so it returns
        // Effect::KeepVersion for the live App to pin the snapshot; the buffer and
        // overlay are untouched (the pin is store-side, not an edit).
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
        assert_eq!(effect, Effect::KeepVersion, "KeepVersion must signal the caller");
        assert!(overlay.is_none(), "the conscious mark opens no overlay");
        assert_eq!(buffer.text(), before, "pinning never edits the buffer");
        assert!(!buffer.can_undo(), "a pin is not an undoable edit");
    }

    #[test]
    fn convert_line_endings_toggles_the_buffer_eol_as_metadata() {
        use crate::buffer::Eol;
        // The palette "Convert Line Endings" command routes Action::ConvertLineEndings
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

    #[test]
    fn align_table_aligns_under_caret_is_undoable_and_no_ops_outside() {
        // Action::AlignTable routes through the SAME apply_core seam a palette/menu
        // invocation uses, so `--keys` drives it identically. A no-path buffer is
        // markdown, so the table under the caret aligns.
        let src = "intro\n| Name | V |\n|---|---|\n| a | 100 |\ntail\n";
        let mut buffer = Buffer::from_str(src);
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut overlay = None;
        let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
        let mut browse_to =
            |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };

        // Caret INSIDE the table (on the body row) — align re-pads the block.
        buffer.set_cursor(buffer.line_col_to_char(3, 2));
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
        let before = ctx.buffer.text();
        apply_core(&mut ctx, &Action::AlignTable, false);
        let after = ctx.buffer.text();
        assert_ne!(after, before, "align edited the buffer");
        assert!(
            after.contains("| Name | V   |\n| ---- | --- |\n| a    | 100 |"),
            "the table block is aligned in place: {after:?}"
        );
        // The surrounding prose is untouched.
        assert!(after.starts_with("intro\n") && after.ends_with("tail\n"));

        // UNDOABLE: one Cmd-Z restores the exact pre-align source.
        ctx.buffer.undo();
        assert_eq!(ctx.buffer.text(), before, "undo restores the pre-align source");

        // NO-OP outside a table: caret on the prose intro line does nothing.
        ctx.buffer.set_cursor(0);
        let untouched = ctx.buffer.text();
        let eff = apply_core(&mut ctx, &Action::AlignTable, false);
        assert_eq!(eff, Effect::None, "align outside a table is a calm no-op");
        assert_eq!(ctx.buffer.text(), untouched, "…and edits nothing");
        assert!(!ctx.buffer.can_undo(), "…so there is nothing to undo");
    }

    /// Drive one action through the REAL `apply_core` seam over a fresh markdown
    /// buffer (a no-path scratch buffer is markdown), seeding the cursor + optional
    /// mark first, and return the buffer for assertions. Mirrors `align_table`'s
    /// harness — the same seam a key / palette / `--keys` invocation rides.
    fn drive_format(src: &str, anchor: Option<usize>, cursor: usize, action: &Action) -> Buffer {
        let mut buffer = Buffer::from_str(src);
        if let Some(a) = anchor {
            buffer.set_cursor(a);
            buffer.set_mark();
        }
        buffer.set_cursor(cursor);
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut overlay = None;
        let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
        let mut browse_to =
            |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
        {
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
            apply_core(&mut ctx, action, false);
        }
        buffer
    }

    #[test]
    fn bold_toggle_through_apply_core_is_one_undoable_edit() {
        // Cmd-P → "Bold" routes Action::Bold through the SAME apply_core seam a key /
        // `--keys` invocation rides. Select "quick" (cols 4..9) and toggle bold.
        let mut b = drive_format("the quick fox", Some(4), 9, &Action::Bold);
        assert_eq!(b.text(), "the **quick** fox", "bold wrapped the selection");
        // The selection covers the same visible text, inside the delimiters.
        assert_eq!(b.selection_range(), Some((6, 11)));
        // ONE undo restores the exact pre-toggle text (a full-buffer replace never
        // coalesces — the whole toggle is a single atomic group).
        b.undo();
        assert_eq!(b.text(), "the quick fox", "one Cmd-Z reverts the toggle");
    }

    #[test]
    fn bullet_list_toggle_through_apply_core_round_trips_and_undoes() {
        // Select the two content lines (cols 0..4 over "a\nb\n") and toggle a bullet list.
        let mut b = drive_format("a\nb\nc\n", Some(0), 4, &Action::ToggleBulletList);
        assert_eq!(b.text(), "- a\n- b\nc\n", "every selected line is prefixed");
        // A second dispatch (the selection now spans the prefixed lines) strips them.
        let re = drive_format(&b.text(), b.selection_range().map(|(s, _)| s), b.selection_range().unwrap().1, &Action::ToggleBulletList);
        assert_eq!(re.text(), "a\nb\nc\n", "re-toggle strips the bullets back");
        // And one undo of the FIRST toggle restores the plain lines.
        b.undo();
        assert_eq!(b.text(), "a\nb\nc\n", "one Cmd-Z reverts the bullet toggle");
    }

    #[test]
    fn code_block_toggle_through_apply_core_wraps_and_undoes() {
        let mut b = drive_format("let x = 1;\n", None, 3, &Action::ToggleCodeBlock);
        assert_eq!(b.text(), "```\nlet x = 1;\n```\n", "the caret line is fenced");
        b.undo();
        assert_eq!(b.text(), "let x = 1;\n", "one Cmd-Z reverts the fence");
    }

    #[test]
    fn heading_toggle_is_a_noop_on_a_code_buffer() {
        // Formatting commands are markdown-only: a `.rs` buffer is never touched
        // (block markup would corrupt code). No edit → nothing to undo.
        use std::path::PathBuf;
        let mut buffer = Buffer::from_str("fn main() {}\n");
        buffer.set_path(PathBuf::from("/tmp/x.rs"));
        buffer.set_cursor(0);
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
        apply_core(&mut ctx, &Action::ToggleHeading, false);
        assert_eq!(ctx.buffer.text(), "fn main() {}\n", "a code buffer is left untouched");
        assert!(!ctx.buffer.can_undo(), "no edit was recorded");
    }

    #[test]
    fn command_palette_opens_then_filters() {
        // OpenCommandPalette summons the palette via make_overlay.
        let mut overlay: Option<OverlayState> = None;
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::OpenCommandPalette);
        let ov = overlay.as_ref().expect("palette opened");
        assert_eq!(ov.kind, OverlayKind::Command);
        // Typing "theme" fuzzy-narrows to "Switch theme" at/near the top.
        for c in "theme".chars() {
            drive(&mut overlay, &mut accept, &Action::InsertChar(c));
        }
        let ov = overlay.as_ref().unwrap();
        assert_eq!(ov.selected_value(), Some("Switch theme"));
    }

    #[test]
    fn command_palette_enter_dispatches_selected_action() {
        // Open, filter to "Go to file", Enter -> run_action == OpenGoto and the
        // palette closed (so the caller can re-dispatch into the goto overlay).
        let mut overlay: Option<OverlayState> = None;
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::OpenCommandPalette);
        for c in "goto".chars() {
            drive(&mut overlay, &mut accept, &Action::InsertChar(c));
        }
        let run = drive_run(&mut overlay, &mut accept, &Action::Newline);
        assert!(overlay.is_none(), "palette closes on accept");
        assert_eq!(run, Some(Action::OpenGoto));
        assert!(accept.is_none(), "the palette runs an action, it does not accept a value");
    }

    #[test]
    fn clicking_a_palette_row_runs_that_command() {
        // The MOUSE mechanic (mirror of the keyboard path): a hover/click resolves the
        // row under the pointer to an `items` index via `overlay_row_at`, sets the
        // picker's `selected` to it, then a LEFT-CLICK ACCEPTS through the SAME
        // `Action::Newline` Enter uses. Here we simulate the hit-test result by setting
        // `selected` directly, then assert Newline runs the command on THAT row — so a
        // click is byte-for-byte "Enter on the clicked row". Empty query => `items` is
        // in catalog order, so the row maps straight to `COMMANDS[idx]`.
        let mut overlay: Option<OverlayState> = None;
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::OpenCommandPalette);
        let idx = 3usize; // a deterministic "clicked" row
        overlay.as_mut().unwrap().selected = idx;
        let ran = drive_run(&mut overlay, &mut accept, &Action::Newline);
        assert_eq!(
            ran,
            Some(crate::commands::COMMANDS[idx].action.clone()),
            "a click runs the catalog command on the clicked row"
        );
        assert!(overlay.is_none(), "accepting a palette row closes it");
        assert!(accept.is_none(), "the palette runs an action, it does not accept a value");
    }

    #[test]
    fn clicking_a_spell_suggestion_replaces_the_word() {
        // The CONTEXTUAL SPELL PANEL reuses the SAME click mechanic: a left-click on a
        // suggestion row sets `selected` to it (via `overlay_row_at`) then ACCEPTS via
        // `Action::Newline` — which, for the Spell kind, replaces the targeted word with
        // the chosen suggestion as one undoable edit and closes the panel. Here we
        // simulate the hit-test by setting `selected` directly, then assert the buffer
        // text swapped "teh" -> the clicked suggestion. Mirror of the keyboard Enter.
        let mut buffer = Buffer::from_str("teh quick brown\n");
        // The word "teh" is at line 0, char cols [0, 3): the panel's spell_target.
        let mut overlay: Option<OverlayState> = Some(OverlayState::new_spell(
            vec!["the".into(), "tea".into(), "ten".into()],
            (0, 0, 3),
        ));
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut make_overlay = |_: OverlayKind| None;
        let mut browse_to = |kind: OverlayKind, rel: Option<String>| browse_level(kind, rel);
        // "Click" the second suggestion row ("tea") by setting the selection there.
        overlay.as_mut().unwrap().selected = 1;
        {
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
            let eff = apply_core(&mut ctx, &Action::Newline, false);
            assert!(matches!(eff, Effect::None), "a spell replace edits in-core, no effect");
        }
        assert!(overlay.is_none(), "accepting a suggestion closes the panel");
        assert_eq!(buffer.text(), "tea quick brown\n", "the clicked suggestion replaced the word");
    }

    #[test]
    fn command_palette_run_action_reopens_into_overlay() {
        // The re-dispatch: feeding the run_action (OpenGoto) back through the core
        // with the slot empty opens the goto overlay, proving run-on-Enter chains
        // into another overlay.
        let mut overlay: Option<OverlayState> = None;
        // make_overlay here returns a real Goto overlay so the re-dispatch opens.
        let mut buffer = Buffer::scratch();
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut make_overlay = |k: OverlayKind| match k {
            OverlayKind::Goto => Some(OverlayState::new(
                OverlayKind::Goto,
                vec!["a.rs".into(), "b.rs".into()],
                vec![],
                vec![],
            )),
            _ => None,
        };
        let mut browse_to = |kind: OverlayKind, rel: Option<String>| browse_level(kind, rel);
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
        // Re-dispatch OpenGoto (the palette already closed) -> goto overlay opens.
        apply_core(&mut ctx, &Action::OpenGoto, false);
        assert_eq!(overlay.as_ref().map(|o| o.kind), Some(OverlayKind::Goto));
    }

    #[test]
    fn outline_opens_filters_and_jumps_to_line() {
        // make_overlay returns a real outline over three headings; Enter on the
        // filtered row ACCEPTS its document LINE for the caller to jump the cursor.
        let mut overlay: Option<OverlayState> = None;
        let mut accept: Option<(OverlayKind, String)> = None;
        let mut buffer = Buffer::scratch();
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut make_overlay = |k: OverlayKind| match k {
            OverlayKind::Outline => Some(OverlayState::new_outline(vec![
                ("Intro".into(), 0usize),
                ("Details".into(), 7usize),
                ("Wrap up".into(), 20usize),
            ])),
            _ => None,
        };
        let mut browse_to = |kind: OverlayKind, rel: Option<String>| browse_level(kind, rel);
        {
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
            // Summon -> the outline picker opens over the headings.
            apply_core(&mut ctx, &Action::OpenOutline, false);
            assert_eq!(ctx.overlay.as_ref().map(|o| o.kind), Some(OverlayKind::Outline));
            // Filter to "Details" ...
            for c in "deta".chars() {
                apply_core(&mut ctx, &Action::InsertChar(c), false);
            }
            assert_eq!(ctx.overlay.as_ref().unwrap().selected_value(), Some("Details"));
            // Enter ACCEPTS its line (7) and closes; the value is the line NUMBER.
            if let Effect::OverlayAccept(kind, val) = apply_core(&mut ctx, &Action::Newline, false) {
                accept = Some((kind, val));
            }
        }
        assert!(overlay.is_none(), "outline closes on accept");
        assert_eq!(accept, Some((OverlayKind::Outline, "7".to_string())));
    }

    #[test]
    fn spell_picker_replaces_word_with_chosen_suggestion() {
        // A buffer with a misspelling on line 1 at char cols 4..11 ("recieve").
        let mut buffer = Buffer::from_str("hello\nyou recieve it\n");
        let mut overlay: Option<OverlayState> = None;
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        // make_overlay returns a real spell picker over two corrections, targeting
        // the word span (line 1, cols 4..11), exactly as the live/headless callers
        // build it from `SpellChecker::suggest_at`.
        let mut make_overlay = |k: OverlayKind| match k {
            OverlayKind::Spell => Some(OverlayState::new_spell(
                vec!["receive".into(), "relieve".into()],
                (1, 4, 11),
            )),
            _ => None,
        };
        let mut browse_to = |kind: OverlayKind, rel: Option<String>| browse_level(kind, rel);
        {
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
            // Summon -> the spell picker opens over the suggestions.
            apply_core(&mut ctx, &Action::OpenSpellSuggest, false);
            assert_eq!(ctx.overlay.as_ref().map(|o| o.kind), Some(OverlayKind::Spell));
            assert_eq!(ctx.overlay.as_ref().unwrap().selected_value(), Some("receive"));
            // Enter REPLACES the word with the top suggestion as ONE edit, closes.
            apply_core(&mut ctx, &Action::Newline, false);
        }
        assert!(overlay.is_none(), "spell picker closes on accept");
        // The misspelled "recieve" became "receive"; nothing else changed.
        assert_eq!(buffer.text(), "hello\nyou receive it\n");
        // It is a SINGLE undoable edit: one undo restores the original word.
        buffer.undo();
        assert_eq!(buffer.text(), "hello\nyou recieve it\n");
    }

    #[test]
    fn right_press_retarget_dismisses_first_menu_then_opens_the_second() {
        // The state transition `app/input.rs::on_right_press` performs when a spell menu
        // is ALREADY open and the user right-clicks a SECOND misspelling: it Cancels the
        // open overlay FIRST, then fires OpenSpellSuggest on the new word — so the menu
        // RE-TARGETS instead of being swallowed. (The raw mouse hit-test is GPU/live-only;
        // this drives the pure core sequence the press routes through.)
        let mut buffer = Buffer::from_str("one recieve two seperate three\n");
        // Start with the FIRST word's spell menu already open (target span A).
        let mut overlay: Option<OverlayState> =
            Some(OverlayState::new_spell(vec!["receive".into()], (0, 4, 11)));
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        // The re-fired OpenSpellSuggest resolves the SECOND word (target span B) — as the
        // live caller would from the new cursor position after the right-press hit-test.
        let mut make_overlay = |k: OverlayKind| match k {
            OverlayKind::Spell => Some(OverlayState::new_spell(
                vec!["separate".into(), "desperate".into()],
                (0, 16, 24),
            )),
            _ => None,
        };
        let mut browse_to = |kind: OverlayKind, rel: Option<String>| browse_level(kind, rel);
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
        // The first menu is open on word A.
        assert_eq!(ctx.overlay.as_ref().unwrap().spell_target, Some((0, 4, 11)));
        // RE-TARGET: dismiss the open overlay FIRST …
        apply_core(&mut ctx, &Action::Cancel, false);
        assert!(ctx.overlay.is_none(), "the first menu must be dismissed first");
        // … then open the second word's menu.
        apply_core(&mut ctx, &Action::OpenSpellSuggest, false);
        let ov = ctx.overlay.as_ref().expect("second menu opens");
        assert_eq!(ov.kind, OverlayKind::Spell);
        assert_eq!(ov.spell_target, Some((0, 16, 24)), "re-targeted to word B");
        assert_eq!(ov.selected_value(), Some("separate"));
    }

    #[test]
    fn spell_picker_summon_is_noop_off_a_misspelling() {
        // make_overlay returns None (the cursor isn't on a flagged word), so the
        // binding is a calm no-op: no overlay opens, the buffer is untouched.
        let mut overlay: Option<OverlayState> = None;
        let mut accept: Option<(OverlayKind, String)> = None;
        drive(&mut overlay, &mut accept, &Action::OpenSpellSuggest);
        assert!(overlay.is_none(), "no misspelling at cursor -> no picker");
        assert!(accept.is_none());
    }

    #[test]
    fn browse_descends_into_folder_then_opens_file() {
        // Open at root level.
        let mut overlay: Option<OverlayState> = browse_level(OverlayKind::Browse, None);
        let mut accept: Option<(OverlayKind, String)> = None;
        // Selected row is `docs` (a folder) -> Enter DESCENDS, not closes.
        drive(&mut overlay, &mut accept, &Action::Newline);
        let ov = overlay.as_ref().expect("still open after descend");
        assert_eq!(ov.browse_dir.as_deref(), Some("docs"));
        let items = ov.item_strings();
        assert!(items.iter().any(|s| s.contains("guide.md")), "got {items:?}");
        assert!(accept.is_none(), "descend must not accept");
        // Move selection past the `api` folder onto guide.md, then Enter opens it.
        drive(&mut overlay, &mut accept, &Action::NextLine);
        drive(&mut overlay, &mut accept, &Action::Newline);
        assert!(overlay.is_none(), "overlay closes on file open");
        assert_eq!(accept, Some((OverlayKind::Goto, "docs/guide.md".to_string())));
    }

    #[test]
    fn browse_arrows_cycle_the_lens_ascend_is_backspace() {
        // Browse is now a FACETED explorer: ←/→ switch the lens (they no longer
        // ascend/descend — that moved to ⌫ / Enter). Start one level deep (docs/).
        let mut overlay: Option<OverlayState> =
            browse_level(OverlayKind::Browse, Some("docs".to_string()));
        let mut accept = None;
        // Opens on the flat All landing.
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("all"));
        // RIGHT steps into the first refinement lens (Folders) — WITHOUT ascending
        // or descending: the directory level is unchanged.
        drive(&mut overlay, &mut accept, &Action::ForwardChar);
        let ov = overlay.as_ref().expect("still open after lens step");
        assert_eq!(ov.active_facet_id(), Some("folders"));
        assert_eq!(ov.browse_dir.as_deref(), Some("docs"), "a lens switch must NOT navigate dirs");
        assert!(accept.is_none(), "a lens switch never accepts");
        // LEFT steps back to All (clamped there — Left at All is a no-op).
        drive(&mut overlay, &mut accept, &Action::BackwardChar);
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("all"));
        drive(&mut overlay, &mut accept, &Action::BackwardChar);
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("all"), "Left at All clamps");
        // ASCEND is now Backspace (empty query): docs/ -> root.
        drive(&mut overlay, &mut accept, &Action::DeleteBackward);
        assert_eq!(overlay.as_ref().unwrap().browse_dir, None, "Backspace ascends docs -> root");
    }

    #[test]
    fn goto_arrows_cycle_the_lens() {
        // The FLAT file picker gains the ←/→ lens strip: All -> Recent -> This folder
        // -> By type, driven through the real `apply_core` overlay intercept (so a
        // `--keys "C-x f <right>"` capture reaches the same code).
        let corpus = vec![
            "README.md".to_string(),
            "src/main.rs".to_string(),
            "notes.txt".to_string(),
        ];
        let mut overlay = Some(OverlayState::new(OverlayKind::Goto, corpus, vec![], vec![]));
        let mut accept = None;
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("all"), "lands on All");
        // RIGHT steps along the strip, never accepting.
        drive(&mut overlay, &mut accept, &Action::ForwardChar);
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("recent"));
        drive(&mut overlay, &mut accept, &Action::ForwardChar);
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("folder"));
        drive(&mut overlay, &mut accept, &Action::ForwardChar);
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("type"));
        // RIGHT at the last lens clamps.
        drive(&mut overlay, &mut accept, &Action::ForwardChar);
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("type"), "clamp at last lens");
        // LEFT walks all the way back to the All home.
        for _ in 0..3 {
            drive(&mut overlay, &mut accept, &Action::BackwardChar);
        }
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("all"));
        assert!(accept.is_none(), "a lens switch never accepts");
    }

    #[test]
    fn command_arrows_cycle_the_lens() {
        // The command palette gains the ←/→ lens strip: All -> File -> Edit -> View ->
        // Recent, driven through the real `apply_core` overlay intercept (so a
        // `--keys "C-p <right>"` capture reaches the same code).
        let mut overlay = Some(OverlayState::new_command(
            crate::commands::names(),
            crate::commands::effective_bindings(&[]),
        ));
        let mut accept = None;
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("all"), "lands on All");
        for expect in ["file", "edit", "view", "recent"] {
            drive(&mut overlay, &mut accept, &Action::ForwardChar);
            assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some(expect));
        }
        // RIGHT at the last lens clamps; LEFT walks back to All.
        drive(&mut overlay, &mut accept, &Action::ForwardChar);
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("recent"), "clamp");
        for _ in 0..4 {
            drive(&mut overlay, &mut accept, &Action::BackwardChar);
        }
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("all"));
        assert!(accept.is_none(), "a lens switch never runs a command");
    }

    #[test]
    fn history_arrows_cycle_the_lens() {
        // The history timeline gains the ←/→ lens strip: All -> Session -> Today,
        // driven through the real `apply_core` intercept. (Reference clocks None here,
        // so the time lenses group nothing — the cycle itself is what's under test.)
        let row = |id: &str| crate::history::TimelineRow {
            when: "x".to_string(),
            which: String::new(),
            counts: "+0 −0".to_string(),
            id: id.to_string(),
            timestamp: id.parse().unwrap_or(0),
            pinned: false,
        };
        let mut overlay = Some(OverlayState::new_history(
            vec![row("300"), row("200"), row("100")],
            None,
            None,
        ));
        let mut accept = None;
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("all"), "lands on All");
        drive(&mut overlay, &mut accept, &Action::ForwardChar);
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("session"));
        drive(&mut overlay, &mut accept, &Action::ForwardChar);
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("today"));
        drive(&mut overlay, &mut accept, &Action::ForwardChar);
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("today"), "clamp");
        drive(&mut overlay, &mut accept, &Action::BackwardChar);
        drive(&mut overlay, &mut accept, &Action::BackwardChar);
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("all"));
        assert!(accept.is_none(), "a lens switch never restores a version");
    }

    #[test]
    fn move_dest_right_descends_left_ascends() {
        // MoveDest opens at root; Right DESCENDS into the highlighted folder.
        let mut overlay: Option<OverlayState> = browse_level(OverlayKind::MoveDest, None);
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::ForwardChar);
        let ov = overlay.as_ref().expect("still open after descend");
        assert_eq!(ov.kind, OverlayKind::MoveDest, "descend keeps MoveDest kind");
        assert_eq!(ov.browse_dir.as_deref(), Some("docs"));
        assert!(accept.is_none(), "descend must not accept");
        // Left ASCENDS back to the root.
        drive(&mut overlay, &mut accept, &Action::BackwardChar);
        assert_eq!(overlay.as_ref().unwrap().browse_dir, None);
    }

    /// Drive one action with a CUSTOM `browse_to` (the project explorer tests use
    /// a real temp-dir tree so absolute-path ascend/descend exercise the FS).
    fn drive_bt(
        overlay: &mut Option<OverlayState>,
        accept: &mut Option<(OverlayKind, String)>,
        browse_to: &mut dyn FnMut(OverlayKind, Option<String>) -> Option<OverlayState>,
        action: &Action,
    ) {
        let mut buffer = Buffer::scratch();
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
        let mut ctx = ActionCtx {
            buffer: &mut buffer,
            shift_selecting: &mut shift,
            zoom: &mut zoom,
            search: &mut search,
            scroll_page_lines: 1,
            overlay,
            make_overlay: &mut make_overlay,
            browse_to,
            oracle: None,
        };
        // Mirror the old `overlay_accept` out-param into `accept` (accumulating).
        if let Effect::OverlayAccept(kind, val) = apply_core(&mut ctx, action, false) {
            *accept = Some((kind, val));
        }
    }

    /// Build a `ws/` tree for the project explorer tests — `ws/child-a/sub/`,
    /// `ws/child-b/` — in an InMemoryFs installed via the FILESYSTEM SEAM, so the
    /// explorer's `list_dir_level` runs against a fake (no temp dir). Returns the
    /// workspace root AND an `FsGuard` the caller binds (`let (ws, _fs) = …`) to keep
    /// the fake installed (and the shared lock held) for the test's duration.
    fn proj_tree() -> (std::path::PathBuf, crate::fs::FsGuard) {
        // A deep-enough root so an ascend test can walk to ws's parent AND its
        // grandparent (`/home/dev/ws` → `/home/dev` → `/home`).
        let ws = std::path::PathBuf::from("/home/dev/ws");
        let mem = crate::fs::InMemoryFs::new()
            .with_dir(ws.join("child-a/sub"))
            .with_dir(ws.join("child-b"));
        let guard = crate::fs::FsGuard::install(std::sync::Arc::new(mem));
        (ws, guard)
    }

    /// A `browse_to` that drives the PROJECT explorer over an absolute temp tree,
    /// exactly like the windowed app's hook (folders-only + synthetic "." row). No
    /// recent-projects MRU (the Recent lens is exercised separately in `overlay.rs`).
    fn project_browse(ws: &std::path::Path, rel: Option<String>) -> Option<OverlayState> {
        let dir = rel.unwrap_or_else(|| ws.to_string_lossy().to_string());
        let folders: Vec<(String, bool)> =
            crate::index::list_dir_level(std::path::Path::new(&dir), None)
                .into_iter()
                .filter(|e| e.is_dir)
                .map(|e| (e.name, e.is_git))
                .collect();
        Some(OverlayState::new_project(dir, folders, &[]))
    }

    #[test]
    fn switch_project_enter_descends_into_folder() {
        let (ws, _fs) = proj_tree();
        let mut browse_to = |k: OverlayKind, rel: Option<String>| {
            assert_eq!(k, OverlayKind::Project);
            project_browse(&ws, rel)
        };
        // Open at ws: corpus is [".", child-a, child-b], default-selected on the
        // first real folder (child-a). Now that Project FACETS (←/→ = lens), Enter
        // on a FOLDER DESCENDS into it (Browse-style) — it does NOT accept. The
        // overlay stays open with child-a's contents (its subfolder `sub`).
        let mut overlay = browse_to(OverlayKind::Project, None);
        let mut accept = None;
        assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("child-a"));
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::Newline);
        let ov = overlay.as_ref().expect("still open after Enter descends");
        assert_eq!(
            ov.browse_dir.as_deref(),
            Some(ws.join("child-a").to_string_lossy().as_ref())
        );
        assert!(ov.item_strings().iter().any(|s| s.contains("sub")), "{:?}", ov.item_strings());
        assert!(accept.is_none(), "descend must not accept");
        // Enter again descends (into `sub`).
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::Newline);
        assert_eq!(
            overlay.as_ref().unwrap().browse_dir.as_deref(),
            Some(ws.join("child-a/sub").to_string_lossy().as_ref())
        );
        // `sub` has no subfolders, so selection rests on the "." row; Enter there
        // SELECTS the drilled-in current directory (child-a/sub) as the root.
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::Newline);
        assert!(overlay.is_none(), "Enter on '.' selects the drilled-in directory");
        assert_eq!(
            accept,
            Some((
                OverlayKind::Project,
                ws.join("child-a/sub").to_string_lossy().to_string()
            )),
            "drilled-in select is its absolute path"
        );
    }

    #[test]
    fn switch_project_arrows_cycle_lens_not_descend() {
        let (ws, _fs) = proj_tree();
        let mut browse_to = |k: OverlayKind, rel: Option<String>| {
            assert_eq!(k, OverlayKind::Project);
            project_browse(&ws, rel)
        };
        // Open at ws, selection on child-a. → (ForwardChar) now CYCLES THE LENS to
        // Recent — it does NOT descend: browse_dir stays at ws.
        let mut overlay = browse_to(OverlayKind::Project, None);
        let mut accept = None;
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("all"));
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::ForwardChar);
        let ov = overlay.as_ref().expect("still open after lens cycle");
        assert_eq!(ov.active_facet_id(), Some("recent"), "→ cycles to the Recent lens");
        assert_eq!(
            ov.browse_dir.as_deref(),
            Some(ws.to_string_lossy().as_ref()),
            "→ cycles the lens, it does NOT descend"
        );
        assert!(accept.is_none());
        // ← (BackwardChar) cycles back to the All home.
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::BackwardChar);
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("all"), "← cycles back to All");
    }

    /// C-f / C-b reach the navigable intercept AS ForwardChar / BackwardChar while
    /// the overlay is open (the keymap is overlay-unaware, so the chord resolves the
    /// same as the arrows). Resolve the chords through the REAL keymap, then drive
    /// the resulting actions: on the FACETED Project navigator C-f CYCLES the lens
    /// forward (same as Right) and C-b CYCLES it back (same as Left).
    #[test]
    fn switch_project_c_f_c_b_cycle_the_lens() {
        use crate::keymap::KeymapState;
        use winit::keyboard::{Key, ModifiersState, SmolStr};
        let ctrl = winit::event::Modifiers::from(ModifiersState::CONTROL);
        let mut km = KeymapState::new();
        // C-f and C-b resolve to the SAME actions the arrows do.
        let c_f = km.resolve(&Key::Character(SmolStr::new("f")), &ctrl);
        let c_b = km.resolve(&Key::Character(SmolStr::new("b")), &ctrl);
        assert_eq!(c_f, Action::ForwardChar, "C-f must resolve to ForwardChar");
        assert_eq!(c_b, Action::BackwardChar, "C-b must resolve to BackwardChar");

        let (ws, _fs) = proj_tree();
        let mut browse_to = |k: OverlayKind, rel: Option<String>| project_browse(&ws, rel);
        let mut overlay = browse_to(OverlayKind::Project, None);
        let mut accept = None;
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("all"));
        // C-f (ForwardChar) cycles the lens forward to Recent, overlay still at ws.
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &c_f);
        let ov = overlay.as_ref().expect("still open after C-f lens cycle");
        assert_eq!(ov.active_facet_id(), Some("recent"), "C-f cycles to Recent");
        assert_eq!(ov.browse_dir.as_deref(), Some(ws.to_string_lossy().as_ref()));
        assert!(accept.is_none());
        // C-b (BackwardChar) cycles the lens back to All.
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &c_b);
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("all"), "C-b cycles back to All");
    }

    #[test]
    fn switch_project_ascends_to_parent() {
        let (ws, _fs) = proj_tree();
        let mut browse_to = |_k: OverlayKind, rel: Option<String>| project_browse(&ws, rel);
        let mut overlay = browse_to(OverlayKind::Project, None);
        let mut accept = None;
        // Backspace (empty query) ASCENDS to ws's PARENT — ABOVE the workspace.
        // (Ascend is Backspace now that ←/→ belong to the lens strip.)
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::DeleteBackward);
        let parent = ws.parent().unwrap().to_string_lossy().to_string();
        let ov = overlay.as_ref().unwrap();
        assert_eq!(ov.browse_dir.as_deref(), Some(parent.as_str()));
        // ws itself now appears as a child folder of its parent.
        let ws_name = ws.file_name().unwrap().to_str().unwrap();
        assert!(ov.item_strings().iter().any(|s| s.contains(ws_name)));
        // Backspace ascends one MORE level (no root floor for Project).
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::DeleteBackward);
        let grandparent = ws.parent().unwrap().parent().unwrap().to_string_lossy().to_string();
        assert_eq!(overlay.as_ref().unwrap().browse_dir.as_deref(), Some(grandparent.as_str()));
    }

    #[test]
    fn switch_project_accept_current_dir_sets_root() {
        let (ws, _fs) = proj_tree();
        let ws_str = ws.to_string_lossy().to_string();
        let mut browse_to = |_k: OverlayKind, rel: Option<String>| project_browse(&ws, rel);
        let mut overlay = browse_to(OverlayKind::Project, None);
        let mut accept = None;
        // Up moves from the first folder onto the synthetic "." accept row...
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::PreviousLine);
        assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("."));
        // ...and Enter ACCEPTS the current directory (ws) as the new root.
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::Newline);
        assert!(overlay.is_none(), "accept closes the explorer");
        assert_eq!(accept, Some((OverlayKind::Project, ws_str)));
    }

    #[test]
    fn browse_backspace_ascends() {
        // Backspace (empty query) ascends in Browse, same as Left.
        let mut overlay = browse_level(OverlayKind::Browse, Some("docs".to_string()));
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::DeleteBackward);
        assert_eq!(overlay.as_ref().unwrap().browse_dir, None, "Backspace ascends docs -> root");
    }

    #[test]
    fn move_dest_backspace_ascends() {
        let mut overlay = browse_level(OverlayKind::MoveDest, Some("docs".to_string()));
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::DeleteBackward);
        assert_eq!(overlay.as_ref().unwrap().browse_dir, None, "Backspace ascends docs -> root");
    }

    #[test]
    fn browse_backspace_pops_filter_before_ascending() {
        // With a non-empty fuzzy query, Backspace pops a CHAR (keeps the level);
        // only an EMPTY query ascends. Preserves type-to-filter within a level.
        let mut overlay = browse_level(OverlayKind::Browse, Some("docs".to_string()));
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::InsertChar('g'));
        drive(&mut overlay, &mut accept, &Action::DeleteBackward);
        let ov = overlay.as_ref().unwrap();
        assert_eq!(ov.query, "");
        assert_eq!(ov.browse_dir.as_deref(), Some("docs"), "popping the filter must not ascend");
        drive(&mut overlay, &mut accept, &Action::DeleteBackward);
        assert_eq!(overlay.as_ref().unwrap().browse_dir, None, "now-empty query ascends");
    }

    #[test]
    fn move_dest_enter_accepts_highlighted_folder() {
        // Enter on the highlighted `docs` folder accepts it as the destination.
        let mut overlay: Option<OverlayState> = browse_level(OverlayKind::MoveDest, None);
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::Newline);
        assert!(overlay.is_none(), "accept closes the picker");
        assert_eq!(accept, Some((OverlayKind::MoveDest, "docs".to_string())));
    }

    #[test]
    fn move_dest_type_to_create_folder() {
        // Type a name that matches no listed folder -> accept CREATES that folder.
        let mut overlay: Option<OverlayState> = browse_level(OverlayKind::MoveDest, None);
        let mut accept = None;
        for c in "ideas".chars() {
            drive(&mut overlay, &mut accept, &Action::InsertChar(c));
        }
        // "ideas" matches nothing in {docs, README.md}, so the query is the dest.
        drive(&mut overlay, &mut accept, &Action::Newline);
        assert_eq!(accept, Some((OverlayKind::MoveDest, "ideas".to_string())));
    }

    /// Serialize the theme-picker tests: they mutate the process-global ACTIVE
    /// theme, and cargo runs tests in parallel. The shared `theme::TEST_LOCK` (not a
    /// private duplicate) so these don't race theme.rs / render.rs theme tests.
    use crate::theme::TEST_LOCK as THEME_LOCK;

    fn theme_overlay() -> Option<OverlayState> {
        let names: Vec<String> = crate::theme::THEMES
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        Some(OverlayState::new_theme(names, crate::theme::active_index()))
    }

    #[test]
    fn theme_move_previews_live() {
        let _g = THEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::theme::set_active(0); // Tawny
        let mut overlay = theme_overlay();
        let mut accept = None;
        // Opens on the flat All lens, highlighting the active world (Tawny).
        assert_eq!(crate::theme::active().name, "Tawny");
        assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("Tawny"));
        let start = overlay.as_ref().unwrap().selected;
        // Down moves through the flat list and previews the NEW highlighted world
        // IMMEDIATELY (the whole editor re-themes to it).
        drive(&mut overlay, &mut accept, &Action::NextLine);
        let after1 = overlay.as_ref().unwrap().selected_value().unwrap().to_string();
        assert_ne!(after1, "Tawny", "Down moved to a different world");
        assert_eq!(crate::theme::active().name, after1, "preview follows the highlight");
        drive(&mut overlay, &mut accept, &Action::NextLine);
        let after2 = overlay.as_ref().unwrap().selected_value().unwrap().to_string();
        assert_eq!(crate::theme::active().name, after2);
        assert_eq!(overlay.as_ref().unwrap().selected, start + 2);
        crate::theme::set_active(0);
    }

    #[test]
    fn theme_enter_commits_previewed_world() {
        let _g = THEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::theme::set_active(0);
        let mut overlay = theme_overlay();
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::NextLine); // preview the next grouped world
        let previewed = overlay.as_ref().unwrap().selected_value().unwrap().to_string();
        drive(&mut overlay, &mut accept, &Action::Newline); // COMMIT
        assert!(overlay.is_none(), "Enter closes the picker");
        assert_eq!(crate::theme::active().name, previewed, "Enter keeps the previewed world");
        assert_eq!(accept, Some((OverlayKind::Theme, previewed)));
        crate::theme::set_active(0);
    }

    #[test]
    fn theme_cancel_reverts_to_starting_world() {
        let _g = THEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::theme::set_active(0); // start on Tawny
        let mut overlay = theme_overlay();
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::NextLine); // preview a different world
        drive(&mut overlay, &mut accept, &Action::NextLine);
        assert_ne!(crate::theme::active().name, "Tawny", "moved off the start world");
        drive(&mut overlay, &mut accept, &Action::Cancel); // REVERT
        assert!(overlay.is_none(), "Cancel closes the picker");
        assert_eq!(crate::theme::active().name, "Tawny", "reverted to the opening world");
        crate::theme::set_active(0);
    }

    #[test]
    fn theme_lens_switch_keeps_world_and_previews() {
        let _g = THEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Currawong is shown under Time (Night), so RIGHT into the Time lens keeps it.
        crate::theme::set_active_by_name("Currawong");
        let mut overlay = theme_overlay();
        let mut accept = None;
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("all"));
        // RIGHT switches the LENS (not the row) and keeps Currawong highlighted; the
        // preview is a no-op (same world), so the active theme is unchanged.
        drive(&mut overlay, &mut accept, &Action::ForwardChar);
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("time"));
        assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("Currawong"));
        assert_eq!(crate::theme::active().name, "Currawong");
        // LEFT switches back to All.
        drive(&mut overlay, &mut accept, &Action::BackwardChar);
        assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("all"));
        // Nothing was accepted by a lens switch.
        assert_eq!(accept, None);
        crate::theme::set_active(0);
    }

    #[test]
    fn theme_typing_filters_and_previews() {
        let _g = THEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::theme::set_active(0);
        let mut overlay = theme_overlay();
        let mut accept = None;
        // Type "quo" -> Quokka is the top match, previewed immediately.
        drive(&mut overlay, &mut accept, &Action::InsertChar('q'));
        drive(&mut overlay, &mut accept, &Action::InsertChar('u'));
        drive(&mut overlay, &mut accept, &Action::InsertChar('o'));
        assert_eq!(crate::theme::active().name, "Quokka");
        crate::theme::set_active(0);
    }

    /// Drive one `Newline` through the REAL `apply_core` seam on `buffer` (with the
    /// caret already placed), so a test exercises the smart-Enter wiring end-to-end
    /// exactly as `--keys "RET"` would.
    fn drive_newline(buffer: &mut Buffer) {
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut overlay = None;
        let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
        let mut browse_to =
            |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
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
        apply_core(&mut ctx, &Action::Newline, false);
    }

    /// A markdown buffer (`.md` path) holding `text` with the caret at char `cursor`.
    fn md(text: &str, cursor: usize) -> Buffer {
        let mut b = Buffer::from_str(text);
        b.set_path(std::path::PathBuf::from("note.md"));
        b.set_cursor(cursor);
        b
    }

    #[test]
    fn smart_newline_continues_lists_quotes_and_indent() {
        // Unordered bullet carries to the new line.
        let mut b = md("- a", 3);
        drive_newline(&mut b);
        assert_eq!(b.text(), "- a\n- ");
        assert_eq!(b.cursor_char(), 6);

        // Ordered list AUTO-INCREMENTS the number.
        let mut b = md("1. first", 8);
        drive_newline(&mut b);
        assert_eq!(b.text(), "1. first\n2. ");

        // A double-digit ordered marker keeps counting and preserves the delimiter.
        let mut b = md("9) nine", 7);
        drive_newline(&mut b);
        assert_eq!(b.text(), "9) nine\n10) ");

        // Blockquote continues with the same '>' run.
        let mut b = md("> quote", 7);
        drive_newline(&mut b);
        assert_eq!(b.text(), "> quote\n> ");

        // Leading indentation is preserved on a plain Enter.
        let mut b = md("    code", 8);
        drive_newline(&mut b);
        assert_eq!(b.text(), "    code\n    ");
    }

    #[test]
    fn smart_newline_empty_item_ends_the_block() {
        // Enter on an EMPTY bullet strips the dangling marker (ends the list).
        let mut b = md("- a\n- ", 6);
        drive_newline(&mut b);
        assert_eq!(b.text(), "- a\n");
        assert_eq!(b.cursor_char(), 4);

        // Same for an empty ordered item …
        let mut b = md("1. ", 3);
        drive_newline(&mut b);
        assert_eq!(b.text(), "");
        assert_eq!(b.cursor_char(), 0);

        // … and an empty blockquote.
        let mut b = md("> ", 2);
        drive_newline(&mut b);
        assert_eq!(b.text(), "");
    }

    #[test]
    fn smart_newline_is_markdown_only() {
        // A non-markdown buffer (a path with a non-md extension) gets a PLAIN
        // newline — no marker continuation — so `.rs` / `.txt` editing is
        // byte-identical. (A no-path scratch buffer is now the prose-first writing
        // surface and DOES continue markers; only a saved non-md file opts out.)
        let mut b = Buffer::from_str("- a");
        b.set_path(std::path::PathBuf::from("code.rs"));
        b.set_cursor(3);
        drive_newline(&mut b);
        assert_eq!(b.text(), "- a\n");
        assert_eq!(b.cursor_char(), 4);
    }

    /// Drive one action through the REAL `apply_core` seam on `buffer` (no overlay /
    /// search), exactly as `--keys` would — for the Tab / Shift-Tab list-edit tests.
    fn drive_act(buffer: &mut Buffer, action: &Action) {
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut overlay = None;
        let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
        let mut browse_to =
            |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
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
        apply_core(&mut ctx, action, false);
    }

    #[test]
    fn tab_indents_a_list_line_and_shift_tab_outdents() {
        // TAB on a bullet indents one level (+2 leading spaces); the depth glyph is
        // derived downstream, so only the text changes here.
        let mut b = md("- item", 6);
        drive_act(&mut b, &Action::InsertTab);
        assert_eq!(b.text(), "  - item");
        // The caret rides with the content (+2).
        assert_eq!(b.cursor_char(), 8);

        // SHIFT-TAB outdents it back (−2, clamped at 0 so a second one is a no-op).
        drive_act(&mut b, &Action::Outdent);
        assert_eq!(b.text(), "- item");
        let v = b.version();
        drive_act(&mut b, &Action::Outdent);
        assert_eq!(b.text(), "- item", "outdent clamps at column 0");
        assert_eq!(b.version(), v, "a clamped outdent makes no edit");
    }

    #[test]
    fn tab_indents_an_ordered_list_without_renumbering() {
        // Ordered items indent too (Tab/Shift-Tab), and we do NOT auto-renumber.
        let mut b = md("1. first", 8);
        drive_act(&mut b, &Action::InsertTab);
        assert_eq!(b.text(), "  1. first", "ordered item indents, number unchanged");
        drive_act(&mut b, &Action::Outdent);
        assert_eq!(b.text(), "1. first");
    }

    #[test]
    fn tab_off_a_list_inserts_spaces_not_an_indent() {
        // On a plain prose line Tab keeps the existing soft-tab (to the next 4-stop),
        // so non-list editing is unchanged.
        let mut b = md("hello", 5);
        drive_act(&mut b, &Action::InsertTab);
        assert_eq!(b.text(), "hello   ", "col 5 => 3 spaces to the next 4-stop");
    }

    #[test]
    fn tab_indents_all_selected_list_lines() {
        // A selection spanning three bullets: one Tab indents them ALL as one undo step.
        let mut b = md("- a\n- b\n- c", 0);
        b.set_mark(); // anchor at 0
        b.set_cursor(b.text().chars().count()); // extend to end => whole doc selected
        drive_act(&mut b, &Action::InsertTab);
        assert_eq!(b.text(), "  - a\n  - b\n  - c", "every selected bullet indents");
        // One undo restores the whole block (the indent is atomic).
        b.undo();
        assert_eq!(b.text(), "- a\n- b\n- c", "the block indent is one atomic undo");

        // Shift-Tab outdents a whole selection back, on an already-indented block.
        let mut b = md("  - a\n  - b\n  - c", 0);
        b.set_mark();
        b.set_cursor(b.text().chars().count());
        drive_act(&mut b, &Action::Outdent);
        assert_eq!(b.text(), "- a\n- b\n- c", "every selected bullet outdents");
    }

    #[test]
    fn select_all_selects_the_whole_buffer_region() {
        // A multi-line buffer with the cursor parked mid-document.
        let mut b = Buffer::from_str("alpha\nbeta\ngamma\n");
        let len = b.text().chars().count();
        b.set_cursor(3); // somewhere in the middle, no mark
        assert!(!b.has_selection());

        drive_act(&mut b, &Action::SelectAll);

        // Mark at document start, point at document end => the whole doc is the region.
        assert!(b.has_selection());
        assert_eq!(b.anchor_char(), Some(0));
        assert_eq!(b.cursor_char(), len);
        assert_eq!(b.selection_range(), Some((0, len)));
        // Endpoints span from (line 0, col 0) to the last line's last col.
        let ((l0, c0), (l1, _c1)) = b.selection_line_col().unwrap();
        assert_eq!((l0, c0), (0, 0), "region starts at document start");
        assert_eq!(l1, b.line_count() - 1, "region ends on the last line");
    }

    #[test]
    fn select_all_on_empty_buffer_is_a_safe_no_op() {
        // An EMPTY buffer: select-all must not panic and leaves an empty region
        // (anchor == cursor == 0), so nothing is "selected".
        let mut b = Buffer::from_str("");
        drive_act(&mut b, &Action::SelectAll);
        assert!(!b.has_selection(), "empty buffer => empty region, not a selection");
        assert_eq!(b.cursor_char(), 0);
        assert_eq!(b.selection_range(), None);
    }

    #[test]
    fn kill_region_after_select_all_empties_the_buffer() {
        // Cmd-A then C-w (cut) removes the ENTIRE document.
        let mut b = Buffer::from_str("one\ntwo\nthree\n");
        drive_act(&mut b, &Action::SelectAll);
        drive_act(&mut b, &Action::KillRegion);
        assert_eq!(b.text(), "", "select-all + cut empties the buffer");
        assert!(!b.has_selection());
        // The cut text is in the kill buffer, so a yank restores the whole doc.
        drive_act(&mut b, &Action::Yank);
        assert_eq!(b.text(), "one\ntwo\nthree\n", "the cut whole-doc yanks back");
    }

    #[test]
    fn type_after_select_all_replaces_the_whole_buffer() {
        // Cmd-A then typing a char replaces the ENTIRE selection with that char,
        // as one atomic edit (one undo restores the original document).
        let mut b = Buffer::from_str("keep\nnothing\nof this\n");
        drive_act(&mut b, &Action::SelectAll);
        drive_act(&mut b, &Action::InsertChar('x'));
        assert_eq!(b.text(), "x", "the whole selection is replaced by the typed char");
        assert_eq!(b.cursor_char(), 1);
        b.undo();
        assert_eq!(b.text(), "keep\nnothing\nof this\n", "one undo restores the original");
    }

    #[test]
    fn copy_region_after_select_all_copies_all_and_keeps_text() {
        // Cmd-A then M-w (copy) leaves the text intact but stages the whole doc for
        // a yank (the mark clears, as copy_region does).
        let mut b = Buffer::from_str("copy\nme\n");
        drive_act(&mut b, &Action::SelectAll);
        drive_act(&mut b, &Action::CopyRegion);
        assert_eq!(b.text(), "copy\nme\n", "copy leaves the document unchanged");
        assert!(!b.has_selection(), "copy clears the mark");
        // Yanking at the end appends the copied whole document.
        b.buffer_end();
        drive_act(&mut b, &Action::Yank);
        assert_eq!(b.text(), "copy\nme\ncopy\nme\n", "the copied whole doc yanks in");
    }

    #[test]
    fn smart_newline_parser_declines_plain_and_inside_marker() {
        // Plain prose: nothing to continue.
        assert!(smart_newline_for("hello", 5).is_none());
        // Caret inside the marker (col 0 of a bullet): plain newline, no dupe.
        assert!(smart_newline_for("- item", 0).is_none());
        // A lone "-" without a trailing space is not a list yet.
        assert!(smart_newline_for("-", 1).is_none());
    }

    #[test]
    fn smart_newline_ordered_marker_at_usize_max_saturates_no_overflow() {
        // A pathological ordered marker of exactly `usize::MAX` parses fine, but the
        // continuation used to compute `n + 1` — which OVERFLOWS (panic in debug,
        // wrap-to-0 in release). `saturating_add(1)` pins the number at usize::MAX
        // instead: the marker simply stops counting up rather than crashing.
        let max = usize::MAX; // 18446744073709551615 on 64-bit
        let line = format!("{max}. item");
        let col = line.chars().count();
        match smart_newline_for(&line, col) {
            Some(SmartNewline::Continue(prefix)) => {
                assert_eq!(prefix, format!("{max}. "), "the number saturates, never overflows");
            }
            _ => panic!("expected a continued ordered item at the usize::MAX marker"),
        }
    }

    /// Drive one action through `apply_core` against a real buffer + a (possibly
    /// live) search panel, so a test can step the find/replace surface.
    fn drive_search(buffer: &mut Buffer, search: &mut Option<SearchState>, action: &Action) {
        let mut shift = false;
        let mut zoom = 1.0;
        let mut overlay = None;
        let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
        let mut browse_to =
            |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
        let mut ctx = ActionCtx {
            buffer,
            shift_selecting: &mut shift,
            zoom: &mut zoom,
            search,
            scroll_page_lines: 1,
            overlay: &mut overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
            oracle: None,
        };
        apply_core(&mut ctx, action, false);
    }

    #[test]
    fn search_tab_toggles_replace_field_through_core() {
        // With NO search live, Tab is a plain soft-tab insert (byte-identical to
        // before this feature) — the intercept only fires inside the panel.
        let mut b = Buffer::from_str("alpha beta alpha");
        b.set_cursor(0);
        let mut search = None;
        drive_search(&mut b, &mut search, &Action::InsertTab);
        assert!(search.is_none());
        assert!(b.text().starts_with(' '), "Tab without a search inserts a soft tab");

        // Open isearch (C-s), then a SINGLE Tab reveals the replace field and
        // focuses it — the same affordance App::handle_search_key gives the live
        // editor, now drivable through the core so `--keys "C-s <Tab>"` sets the
        // sidecar's `replace_active`.
        let mut b = Buffer::from_str("alpha beta alpha");
        b.set_cursor(0);
        let mut search = None;
        drive_search(&mut b, &mut search, &Action::SearchForward);
        assert!(!search.as_ref().unwrap().is_replace_active());
        drive_search(&mut b, &mut search, &Action::InsertTab);
        {
            let st = search.as_ref().unwrap();
            assert!(st.is_replace_active());
            assert!(st.is_editing_replacement());
        }
        // A second Tab flips focus back to the query field (one warm panel, no new
        // chrome) — the replace row stays revealed.
        drive_search(&mut b, &mut search, &Action::InsertTab);
        {
            let st = search.as_ref().unwrap();
            assert!(st.is_replace_active());
            assert!(!st.is_editing_replacement());
        }
        // The in-panel Tabs never leaked a soft tab into the document.
        assert_eq!(b.text(), "alpha beta alpha");
    }

    #[test]
    fn cmd_r_opens_replace_on_find_then_focuses_replace_through_core() {
        // Cmd-R with NO search open (Action::OpenReplace) opens the panel with the
        // replace row REVEALED but focus on the FIND field — the redesigned headline
        // door, drivable through the core so `--keys "Cmd-r"` sets the sidecar.
        let mut b = Buffer::from_str("alpha beta alpha");
        b.set_cursor(0);
        let mut search = None;
        drive_search(&mut b, &mut search, &Action::OpenReplace);
        {
            let st = search.as_ref().expect("Cmd-R opens the search panel");
            assert!(st.is_replace_active(), "the replace row is revealed on open");
            assert!(!st.is_editing_replacement(), "focus opens on the find field");
        }
        // Cmd-R AGAIN (panel already open) jumps focus into the replacement field —
        // the search intercept focuses it instead of resetting the search.
        drive_search(&mut b, &mut search, &Action::OpenReplace);
        {
            let st = search.as_ref().unwrap();
            assert!(st.is_replace_active() && st.is_editing_replacement());
        }
        // Tab switches focus back to the find field (the one field-switch key).
        drive_search(&mut b, &mut search, &Action::InsertTab);
        assert!(!search.as_ref().unwrap().is_editing_replacement());
        // None of this leaked into the document.
        assert_eq!(b.text(), "alpha beta alpha");
    }

    // --- RECOIL PRIMITIVE: blocked-action trigger logic ----------------------

    /// Drive one action through `apply_core` against a fresh buffer seeded with
    /// `text` and the cursor at char `cursor`, returning the resulting `(Effect,
    /// cursor_char)` — the cursor is exposed too so a caller can pin the "bump
    /// fires only when the motion did NOT move the cursor" rule alongside the
    /// effect. No oracle (logical-line fallback), so vertical motion uses the
    /// buffer lines.
    fn drive_effect_and_cursor(text: &str, cursor: usize, action: &Action) -> (Effect, usize) {
        let mut buffer = Buffer::from_str(text);
        buffer.set_cursor(cursor);
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
        let effect = apply_core(&mut ctx, action, false);
        drop(ctx);
        (effect, buffer.cursor_char())
    }

    /// [`drive_effect_and_cursor`], discarding the resulting cursor — the common
    /// case for the recoil-trigger tests that only care about the [`Effect`].
    fn drive_effect(text: &str, cursor: usize, action: &Action) -> Effect {
        drive_effect_and_cursor(text, cursor, action).0
    }

    #[test]
    fn blocked_motions_arm_recoil_away_from_the_wall() {
        use crate::caret::RecoilDir::{Down, Left, Right, Up};
        let txt = "ab\ncd"; // chars: a b \n c d  (end == char 5)
        // Horizontal walls.
        assert_eq!(drive_effect(txt, 5, &Action::ForwardChar), Effect::Recoil(Left));
        assert_eq!(drive_effect(txt, 0, &Action::BackwardChar), Effect::Recoil(Right));
        assert_eq!(drive_effect(txt, 5, &Action::ForwardWord), Effect::Recoil(Left));
        assert_eq!(drive_effect(txt, 0, &Action::BackwardWord), Effect::Recoil(Right));
        // BOUNDARY BUMP — line-edge motions already at the edge (C-a/C-e,
        // Cmd-Left/Right): cursor 0 is already line 0's start; cursor 2 is already
        // line 0's end (right before the '\n').
        assert_eq!(drive_effect(txt, 0, &Action::LineStart), Effect::Recoil(Right));
        assert_eq!(drive_effect(txt, 2, &Action::LineEnd), Effect::Recoil(Left));
        // Vertical walls (cursor parked at the end of the last / start of the first
        // line so the logical motion truly can't move).
        assert_eq!(drive_effect(txt, 5, &Action::NextLine), Effect::Recoil(Up));
        assert_eq!(drive_effect(txt, 0, &Action::PreviousLine), Effect::Recoil(Down));
        // Buffer ends already at the end / start.
        assert_eq!(drive_effect(txt, 5, &Action::BufferEnd), Effect::Recoil(Up));
        assert_eq!(drive_effect(txt, 0, &Action::BufferStart), Effect::Recoil(Down));
        // Page scroll that can't page (1 line per page; already at top/bottom).
        assert_eq!(drive_effect(txt, 5, &Action::PageScrollDown), Effect::Recoil(Up));
        assert_eq!(drive_effect(txt, 0, &Action::PageScrollUp), Effect::Recoil(Down));
    }

    #[test]
    fn unblocked_motions_do_not_recoil() {
        let txt = "ab\ncd";
        // Each of these CAN proceed, so no recoil (and the cursor really moved).
        assert_eq!(drive_effect(txt, 0, &Action::ForwardChar), Effect::None);
        assert_eq!(drive_effect(txt, 5, &Action::BackwardChar), Effect::None);
        assert_eq!(drive_effect(txt, 0, &Action::NextLine), Effect::None);
        assert_eq!(drive_effect(txt, 5, &Action::PreviousLine), Effect::None);
        assert_eq!(drive_effect(txt, 0, &Action::BufferEnd), Effect::None);
        assert_eq!(drive_effect(txt, 5, &Action::BufferStart), Effect::None);
        // Line-edge motions NOT already at the edge proceed too (a real relocation).
        assert_eq!(drive_effect(txt, 1, &Action::LineStart), Effect::None);
        assert_eq!(drive_effect(txt, 0, &Action::LineEnd), Effect::None);
    }

    #[test]
    fn blocked_recoil_leaves_buffer_and_cursor_untouched() {
        // The whole point of a recoil: the logical state does NOT change (only the
        // visual caret bumps, live-only), so a settled capture is byte-identical.
        let mut buffer = Buffer::from_str("ab\ncd");
        buffer.set_cursor(5);
        let before_text = buffer.text();
        let before_cursor = buffer.cursor_char();
        let eff = drive_effect("ab\ncd", 5, &Action::ForwardChar);
        assert!(matches!(eff, Effect::Recoil(_)));
        // Re-run on the same buffer instance to assert no mutation slipped through.
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
        apply_core(&mut ctx, &Action::ForwardChar, false);
        drop(ctx);
        assert_eq!(buffer.text(), before_text);
        assert_eq!(buffer.cursor_char(), before_cursor);
    }

    #[test]
    fn exhausted_undo_redo_recoil() {
        use crate::caret::RecoilDir::{Left, Right};
        // A fresh buffer has no history: undo/redo are no-ops -> recoil.
        assert_eq!(drive_effect("hello", 0, &Action::Undo), Effect::Recoil(Left));
        assert_eq!(drive_effect("hello", 0, &Action::Redo), Effect::Recoil(Right));
    }

    #[test]
    fn blocked_delete_recoils_no_op_delete() {
        use crate::caret::RecoilDir::{Left, Right};
        // Backspace at buffer start / C-d / M-d at buffer end remove nothing -> recoil.
        assert_eq!(drive_effect("hi", 0, &Action::DeleteBackward), Effect::Recoil(Right));
        assert_eq!(drive_effect("hi", 2, &Action::DeleteForward), Effect::Recoil(Left));
        assert_eq!(drive_effect("hi", 2, &Action::DeleteWordForward), Effect::Recoil(Left));
        // A delete that DOES remove a char SUCCEEDS -> the caret swallows what it ate
        // (the PHASE 2 inward squash), mutually exclusive with the blocked recoil.
        assert_eq!(drive_effect("hi", 1, &Action::DeleteBackward), Effect::DeleteSquash);
        assert_eq!(drive_effect("hi", 0, &Action::DeleteForward), Effect::DeleteSquash);
    }

    #[test]
    fn successful_edits_arm_the_caret_flinch() {
        // PHASE 2 — a SUCCESSFUL edit flinches the visual caret: a typed char → a
        // typing impact, a backspace / C-d / word-delete → an inward squash, a
        // kill-line → a gulp. The trigger reads the SAME content-version signal the
        // recoil uses (here it CHANGED), so it's drivable + unit-testable with no GPU.
        assert_eq!(drive_effect("hi", 1, &Action::InsertChar('x')), Effect::TypeImpact);
        assert_eq!(drive_effect("hi", 1, &Action::DeleteBackward), Effect::DeleteSquash);
        assert_eq!(drive_effect("hi", 0, &Action::DeleteForward), Effect::DeleteSquash);
        assert_eq!(drive_effect("foo bar", 7, &Action::DeleteWordBackward), Effect::DeleteSquash);
        assert_eq!(drive_effect("foo bar", 0, &Action::DeleteWordForward), Effect::DeleteSquash);
        // A kill-line that removes text gulps.
        assert_eq!(drive_effect("hello", 0, &Action::KillLine), Effect::Gulp);
        // PHASE 3 — ENTER JUICE: a plain Enter lands a caret-level touchdown squash,
        // and so does the markdown smart-Enter's list-continuation edit (same Action,
        // same arm — the flinch is keyed off `Action::Newline`, not which branch fired).
        assert_eq!(drive_effect("hi", 1, &Action::Newline), Effect::LineLand);
        assert_eq!(drive_effect("- item", 6, &Action::Newline), Effect::LineLand);
    }

    #[test]
    fn no_op_edits_and_non_edits_do_not_flinch() {
        // A kill-line at the very end of the buffer removes nothing -> the content
        // version is unchanged, so NO gulp (and no recoil — kill-line has no wall arm).
        assert_eq!(drive_effect("hi", 2, &Action::KillLine), Effect::None);
        // A plain motion is not an edit: it never flinches (it may recoil, tested
        // elsewhere). A mid-buffer forward-char just moves -> None.
        assert_eq!(drive_effect("hi", 0, &Action::ForwardChar), Effect::None);
    }

    /// Per-DELETE fixture for the flinch completeness sweep below. Returns `Some`
    /// for every action that removes a character — a `(text, ok_cursor,
    /// wall_cursor, wall_recoil)`: a cursor where the delete REMOVES a char (must
    /// `DeleteSquash`) and one at the buffer edge where it removes NOTHING (must
    /// recoil that way). Backward deletes eat leftward (blocked at the START,
    /// bump `Right`); forward deletes eat rightward (blocked at the END, bump
    /// `Left`). `KillLine` deletes too but GULPs (its own effect, no wall arm), so
    /// it — and every NON-delete action — returns `None`. The match has NO
    /// wildcard (mirroring `all_actions`'s `_assert_covers`), so a NEW `Action`
    /// variant fails to compile until it is classified here: a future delete can't
    /// silently skip the squash/recoil decision the way `DeleteWordForward` once did.
    fn delete_flinch_fixture(
        action: &Action,
    ) -> Option<(&'static str, usize, usize, crate::caret::RecoilDir)> {
        use crate::caret::RecoilDir::{Left, Right};
        match action {
            Action::DeleteBackward | Action::DeleteWordBackward => Some(("hi", 1, 0, Right)),
            Action::DeleteForward | Action::DeleteWordForward => Some(("hi", 0, 2, Left)),
            // Not a squash/recoil delete: KillLine gulps; the rest never delete.
            Action::ForwardChar
            | Action::BackwardChar
            | Action::NextLine
            | Action::PreviousLine
            | Action::LineStart
            | Action::LineEnd
            | Action::ForwardWord
            | Action::BackwardWord
            | Action::BufferStart
            | Action::BufferEnd
            | Action::InsertChar(_)
            | Action::Newline
            | Action::InsertTab
            | Action::Outdent
            | Action::KillLine
            | Action::Yank
            | Action::Undo
            | Action::Redo
            | Action::SetMark
            | Action::CopyRegion
            | Action::KillRegion
            | Action::SelectAll
            | Action::ZoomIn
            | Action::ZoomOut
            | Action::ZoomReset
            | Action::PageScrollDown
            | Action::PageScrollUp
            | Action::Save
            | Action::Quit
            | Action::SearchForward
            | Action::SearchBackward
            | Action::OpenReplace
            | Action::Cancel
            | Action::OpenThemeMenu
            | Action::OpenCommandPalette
            | Action::OpenOutline
            | Action::OpenSpellSuggest
            | Action::ToggleCaretMode
            | Action::OpenCaretMenu
            | Action::OpenDictionaryMenu
            | Action::ToggleSpellcheck
            | Action::TogglePageMode
            | Action::PageWider
            | Action::PageNarrower
            | Action::PageReset
            | Action::CycleFocusMode
            | Action::ToggleDebug
            | Action::ToggleOutline
            | Action::ToggleTypewriter
            | Action::ToggleHiddenFiles
            | Action::ShowStatsHud
            | Action::OpenGoto
            | Action::OpenProject
            | Action::OpenRecentProjects
            | Action::OpenBrowse
            | Action::LastBuffer
            | Action::NewNote
            | Action::MoveNote
            | Action::OpenSettings
            | Action::OpenSettingsMenu
            | Action::OpenKeybindings
            | Action::OpenHistory
            | Action::KeepVersion
            | Action::FinishBuffer
            | Action::FollowLink
            | Action::BeginPrefix
            | Action::About
            | Action::ConvertLineEndings
            | Action::AlignTable
            | Action::ToggleBlockquote
            | Action::ToggleBulletList
            | Action::ToggleNumberedList
            | Action::ToggleTaskList
            | Action::ToggleHeading
            | Action::ToggleCodeBlock
            | Action::Bold
            | Action::Italic
            | Action::InlineCode
            | Action::Highlight
            | Action::Strikethrough
            | Action::Ignore => None,
        }
    }

    #[test]
    fn every_delete_squashes_on_success_and_recoils_on_a_no_op() {
        // COMPLETENESS SWEEP over `all_actions()` (compile-time-complete via its
        // `_assert_covers`): every DELETE flinches BOTH ways — an inward
        // `DeleteSquash` when it removes a char, and a boundary `Recoil` when it
        // removes nothing at the buffer edge. `delete_flinch_fixture`'s no-wildcard
        // match forces every new `Action` to be classified, so a future delete
        // can't silently ship with no caret feedback — the exact gap M-d
        // (`DeleteWordForward`) fell through, missing from BOTH `impact_for` and
        // `recoil_for` while every other delete flinched.
        for action in all_actions() {
            let Some((text, ok_cursor, wall_cursor, dir)) = delete_flinch_fixture(&action) else {
                continue;
            };
            assert_eq!(
                drive_effect(text, ok_cursor, &action),
                Effect::DeleteSquash,
                "{action:?}: a delete that removes a char must squash"
            );
            assert_eq!(
                drive_effect(text, wall_cursor, &action),
                Effect::Recoil(dir),
                "{action:?}: a delete with nothing to remove must recoil {dir:?}"
            );
        }
    }

    // --- COPY PULSE: the arm decision at the apply seam ----------------------

    /// Like [`drive_act`] but returns the resulting [`Effect`] — the COPY PULSE
    /// tests need to `set_mark`/`set_cursor` on the buffer BEFORE dispatch
    /// (unlike [`drive_effect`], which only ever seeds a bare cursor position),
    /// so they build the buffer themselves and drive it through the REAL
    /// `apply_core` seam directly.
    fn drive_act_effect(buffer: &mut Buffer, action: &Action) -> Effect {
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut overlay = None;
        let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
        let mut browse_to =
            |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
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
        apply_core(&mut ctx, action, false)
    }

    #[test]
    fn copy_with_selection_arms_the_copy_pulse() {
        // M-w / Cmd-C over a NON-EMPTY selection: the caret gets a gentle pulse
        // and the selection quad brightens (`Effect::CopyPulse`) — copy's one
        // common, otherwise-invisible action finally gets in-world feedback. The
        // document itself is untouched (copy never edits).
        let mut b = Buffer::from_str("copy me");
        b.set_mark();
        b.set_cursor(4); // "copy" selected
        assert_eq!(drive_act_effect(&mut b, &Action::CopyRegion), Effect::CopyPulse);
        assert_eq!(b.text(), "copy me", "copy leaves the document unchanged");
        assert!(!b.has_selection(), "copy_region still clears the mark as before");
    }

    #[test]
    fn copy_without_selection_does_not_pulse() {
        // No mark at all: M-w is the pre-existing documented no-op (nothing
        // selected, nothing to copy) — it must NOT gain a pulse.
        let mut b = Buffer::from_str("nothing selected");
        assert_eq!(drive_act_effect(&mut b, &Action::CopyRegion), Effect::None);

        // A mark set exactly AT the cursor (an EMPTY region, `anchor == cursor`)
        // is the same documented no-op — `has_selection()` is false either way.
        let mut b2 = Buffer::from_str("nothing selected");
        b2.set_mark();
        assert_eq!(drive_act_effect(&mut b2, &Action::CopyRegion), Effect::None);
    }

    #[test]
    fn cut_does_not_arm_the_copy_pulse() {
        // C-w / KillRegion has a VISIBLE result (the text vanishes) — it must
        // never arm the copy pulse, even over an active selection identical to
        // the one that just armed it above.
        let mut b = Buffer::from_str("cut me");
        b.set_mark();
        b.set_cursor(3);
        assert_eq!(drive_act_effect(&mut b, &Action::KillRegion), Effect::None);
        assert_eq!(b.text(), " me", "the cut actually removed the selected text");
    }

    #[test]
    fn line_edge_motions_recoil_at_the_edge_and_move_off_it() {
        // BOUNDARY BUMP: C-a at col 0 / C-e at line end are common idempotent
        // presses, but a silent no-op still reads as "nothing happened" rather than
        // "you're at the edge" — so they now bump the caret, quiet like every other
        // wall (a superseded decision; see `recoil_for`'s doc). Off the edge they
        // still just move (no recoil).
        use crate::caret::RecoilDir::{Left, Right};
        assert_eq!(drive_effect("abc", 0, &Action::LineStart), Effect::Recoil(Right));
        assert_eq!(drive_effect("abc", 3, &Action::LineEnd), Effect::Recoil(Left));
        assert_eq!(drive_effect("abc", 1, &Action::LineStart), Effect::None);
        assert_eq!(drive_effect("abc", 0, &Action::LineEnd), Effect::None);
    }

    /// Per-MOTION boundary FIXTURE for the boundary-bump completeness sweep below:
    /// text + cursor char index that puts THIS `Action::is_motion` variant already
    /// at its wall (so it truly cannot move), plus the [`crate::caret::RecoilDir`]
    /// the bump must fire in. Reuses the same two-line "ab\ncd" fixture as
    /// `blocked_motions_arm_recoil_away_from_the_wall`. A NEW motion classified
    /// `is_motion` without a decision here panics `boundary_motions_bump_only_when_blocked`
    /// LOUDLY instead of silently shipping a silent no-op boundary.
    fn motion_boundary_fixture(action: &Action) -> (&'static str, usize, crate::caret::RecoilDir) {
        use crate::caret::RecoilDir::{Down, Left, Right, Up};
        const TXT: &str = "ab\ncd"; // chars: a b \n c d (end == char 5); "ab" / "cd"
        match action {
            Action::ForwardChar => (TXT, 5, Left),
            Action::BackwardChar => (TXT, 0, Right),
            Action::ForwardWord => (TXT, 5, Left),
            Action::BackwardWord => (TXT, 0, Right),
            Action::NextLine => (TXT, 5, Up),
            Action::PreviousLine => (TXT, 0, Down),
            Action::LineStart => (TXT, 0, Right),
            Action::LineEnd => (TXT, 2, Left),
            Action::BufferStart => (TXT, 0, Down),
            Action::BufferEnd => (TXT, 5, Up),
            _ => panic!(
                "{action:?} is classified Action::is_motion but has no boundary-bump \
                 fixture decided in `motion_boundary_fixture` — pick its wall position \
                 + recoil direction there"
            ),
        }
    }

    #[test]
    fn boundary_motions_bump_only_when_blocked() {
        // BOUNDARY BUMP completeness sweep, over `all_actions()`'s compile-time-complete
        // enumeration (the SAME gate `every_classified_motion_extends_shift_selection_
        // and_no_mover_is_missing` uses below): for every `Action::is_motion` variant,
        // the motion BLOCKED at its wall (`motion_boundary_fixture`) must recoil AND
        // leave the cursor exactly where it was, while the SAME motion driven from a
        // mid-document position with no wall in any direction (cursor 14 in the fixture
        // below — the shift-selection sweep already proves every motion actually moves
        // the point from there) must NOT recoil and must actually move the cursor. This
        // pins "the bump fires only when the motion did not move the cursor" at the
        // `apply_core` seam, for every wall on both sides.
        let no_wall = "alpha beta\ngamma delta\nepsilon zeta\n";
        for action in all_actions() {
            if !action.is_motion() {
                continue;
            }
            let (wall_text, wall_cursor, dir) = motion_boundary_fixture(&action);
            let (blocked_effect, blocked_cursor) =
                drive_effect_and_cursor(wall_text, wall_cursor, &action);
            assert_eq!(
                blocked_effect,
                Effect::Recoil(dir),
                "{action:?}: blocked at its wall must recoil {dir:?}"
            );
            assert_eq!(
                blocked_cursor, wall_cursor,
                "{action:?}: a recoil must not move the cursor"
            );

            let (unblocked_effect, unblocked_cursor) =
                drive_effect_and_cursor(no_wall, 14, &action);
            assert_ne!(
                unblocked_effect,
                Effect::Recoil(dir),
                "{action:?}: an unblocked motion must not bump"
            );
            assert_ne!(
                unblocked_cursor, 14,
                "{action:?}: an unblocked motion must actually move the cursor"
            );
        }
    }

    #[test]
    fn history_picker_enter_emits_restore_id_of_the_highlighted_version() {
        // SUMMON the timeline with three versions (newest-first); NAVIGATE down and
        // ENTER emits OverlayAccept(History, <id>) for the highlighted version, then
        // closes. The caller resolves the id via history::load + set_text (undoable).
        let row = |when: &str, which: &str, counts: &str, id: &str| crate::history::TimelineRow {
            when: when.to_string(),
            which: which.to_string(),
            counts: counts.to_string(),
            id: id.to_string(),
            timestamp: id.parse().unwrap_or(0),
            pinned: false,
        };
        let rows = vec![
            row("just now", "edited \"A\"", "+0 −0", "300"),
            row("2 min ago", "edited \"B\"", "+0 −1", "200"),
            row("1 hr ago", "", "+1 −2", "100"),
        ];
        let mut overlay = Some(OverlayState::new_history(rows, None, None));
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::NextLine); // highlight "2 min ago"
        drive(&mut overlay, &mut accept, &Action::Newline);
        assert!(overlay.is_none(), "Enter closes the history picker");
        assert_eq!(accept, Some((OverlayKind::History, "200".to_string())));
    }

    #[test]
    fn history_picker_empty_state_enter_is_a_no_op_close() {
        // The "no history yet" row has an empty id: Enter emits NO accept, just closes.
        let mut overlay = Some(OverlayState::new_history(Vec::new(), None, None));
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::Newline);
        assert!(overlay.is_none(), "Enter closes even the empty-state picker");
        assert_eq!(accept, None, "empty-state row restores nothing");
    }

    // --- SHIFT-SELECTION at the apply_core seam --------------------------------
    //
    // The selection-on-motion state machine (transient Shift+motion vs the sticky
    // C-Space mark) lives at the TOP of `apply_core`, but until now no test passed
    // `shift=true` through the seam. These drive it exactly as held-Shift arrows
    // would: first shift-motion sets the mark, a run extends, an unshifted motion
    // collapses, the C-Space mark survives unshifted motion, and Cancel clears.

    /// Drive one action through the REAL `apply_core` seam with an EXPLICIT
    /// `shift` flag and a PERSISTENT `shift_selecting` slot, so a test can walk a
    /// whole Shift+motion run (set → extend → collapse) across calls.
    fn drive_shift(
        buffer: &mut Buffer,
        shift_selecting: &mut bool,
        action: &Action,
        shift: bool,
    ) -> Effect {
        let mut zoom = 1.0;
        let mut search = None;
        let mut overlay = None;
        let mut make_overlay = |_k: OverlayKind| -> Option<OverlayState> { None };
        let mut browse_to =
            |_k: OverlayKind, _r: Option<String>| -> Option<OverlayState> { None };
        let mut ctx = ActionCtx {
            buffer,
            shift_selecting,
            zoom: &mut zoom,
            search: &mut search,
            scroll_page_lines: 1,
            overlay: &mut overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
            oracle: None,
        };
        apply_core(&mut ctx, action, shift)
    }

    #[test]
    fn about_opens_and_any_key_dismisses_it() {
        // `Action::About` OPENS the summoned card (a process global, mirroring
        // `hud`/`debug` — see `about.rs`); the VERY NEXT key through `apply_core`
        // (ANY action at all — a plain motion here, deliberately not Esc) closes
        // it again and is otherwise consumed (no other effect: the cursor must
        // not move even though `ForwardChar` normally would).
        let _g = crate::about::test_lock();
        crate::about::set_open(false);
        let mut b = Buffer::from_str("alpha beta");
        let mut sel = false;
        let cursor0 = b.cursor_char();

        drive_shift(&mut b, &mut sel, &Action::About, false);
        assert!(crate::about::about_open(), "Action::About opens the card");
        assert_eq!(b.cursor_char(), cursor0, "opening About never touches the buffer");

        // ANY key — a plain forward-char motion, not Esc — dismisses it and is
        // fully consumed: the motion must NOT actually move the cursor.
        drive_shift(&mut b, &mut sel, &Action::ForwardChar, false);
        assert!(!crate::about::about_open(), "the next key closes the card");
        assert_eq!(b.cursor_char(), cursor0, "the dismissing key is consumed, not applied");

        // Once closed, the SAME action now runs normally (proves the intercept
        // only fires while the card is actually open).
        drive_shift(&mut b, &mut sel, &Action::ForwardChar, false);
        assert_eq!(b.cursor_char(), cursor0 + 1, "ForwardChar works again once About is closed");

        crate::about::set_open(false);
    }

    #[test]
    fn shift_motion_sets_mark_extends_then_unshifted_motion_collapses() {
        let mut b = Buffer::from_str("alpha beta\ngamma delta");
        let mut sel = false;
        // The FIRST Shift+motion sets the mark at the pre-motion cursor, moves
        // the point, and arms the transient flag.
        drive_shift(&mut b, &mut sel, &Action::ForwardChar, true);
        assert_eq!(b.anchor_char(), Some(0), "first shift-motion sets the mark");
        assert_eq!(b.selection_range(), Some((0, 1)));
        assert!(sel, "the transient shift flag arms");
        // A RUN of shifted motions keeps the SAME anchor and extends the head —
        // across a word motion, a line-edge, and a vertical move.
        drive_shift(&mut b, &mut sel, &Action::ForwardWord, true);
        assert_eq!(b.anchor_char(), Some(0));
        assert_eq!(b.selection_range(), Some((0, 5)), "shift-word extends to 'alpha'");
        drive_shift(&mut b, &mut sel, &Action::LineEnd, true);
        assert_eq!(b.selection_range(), Some((0, 10)), "shift-C-e extends to line end");
        drive_shift(&mut b, &mut sel, &Action::NextLine, true);
        let (_, head) = b.selection_range().unwrap();
        assert!(head > 10, "shift-C-n extends onto the next line");
        assert_eq!(b.anchor_char(), Some(0), "the anchor never moves during the run");
        // Shift RELEASED + a plain motion: the TRANSIENT selection collapses.
        drive_shift(&mut b, &mut sel, &Action::ForwardChar, false);
        assert!(!b.has_selection(), "unshifted motion collapses the shift-selection");
        assert_eq!(b.anchor_char(), None);
        assert!(!sel, "the transient flag disarms");
    }

    #[test]
    fn shift_selection_extends_backwards_too() {
        // A Shift+backward run anchors at the start cursor and pulls the head LEFT
        // (selection_range orders the endpoints).
        let mut b = Buffer::from_str("one two three");
        let mut sel = false;
        b.set_cursor(7); // end of "two"
        drive_shift(&mut b, &mut sel, &Action::BackwardWord, true);
        assert_eq!(b.anchor_char(), Some(7));
        assert_eq!(b.selection_range(), Some((4, 7)), "shift-M-b selects 'two'");
        drive_shift(&mut b, &mut sel, &Action::BufferStart, true);
        assert_eq!(b.selection_range(), Some((0, 7)), "shift-M-< extends to the top");
    }

    #[test]
    fn c_space_sticky_mark_survives_unshifted_motion_until_cancel() {
        // The STICKY (emacs) mode: C-Space sets the mark, and PLAIN motions keep
        // extending it — the unshifted-collapse branch only fires for the
        // transient Shift mode. C-g then clears the region.
        let mut b = Buffer::from_str("one two three");
        let mut sel = false;
        drive_shift(&mut b, &mut sel, &Action::SetMark, false);
        assert_eq!(b.anchor_char(), Some(0));
        assert!(!sel, "C-Space is the sticky mark, not a Shift-selection");
        drive_shift(&mut b, &mut sel, &Action::ForwardWord, false);
        assert_eq!(b.selection_range(), Some((0, 3)), "a plain motion extends the mark");
        drive_shift(&mut b, &mut sel, &Action::ForwardWord, false);
        assert_eq!(b.selection_range(), Some((0, 7)), "…and keeps extending");
        // A SHIFTED motion mid-region keeps the existing anchor (no re-mark).
        drive_shift(&mut b, &mut sel, &Action::ForwardChar, true);
        assert_eq!(b.anchor_char(), Some(0), "shift over a live mark keeps the anchor");
        // Cancel (C-g / Esc) clears the region.
        drive_shift(&mut b, &mut sel, &Action::Cancel, false);
        assert!(!b.has_selection(), "Cancel clears the sticky region");
        assert_eq!(b.anchor_char(), None);
        assert!(!sel);
    }

    #[test]
    fn cancel_clears_a_shift_selection() {
        let mut b = Buffer::from_str("alpha beta");
        let mut sel = false;
        drive_shift(&mut b, &mut sel, &Action::ForwardWord, true);
        assert!(b.has_selection());
        drive_shift(&mut b, &mut sel, &Action::Cancel, false);
        assert!(!b.has_selection(), "Cancel clears the shift-selection");
        assert_eq!(b.anchor_char(), None);
        assert!(!sel, "…and disarms the transient flag");
    }

    /// EVERY `Action` variant, one representative each, for the completeness
    /// sweep below. The inner `_assert_covers` match lists every variant with NO
    /// wildcard arm, so ADDING a new Action variant fails to compile here until
    /// the new variant is added to this list — the sweep can never silently miss
    /// a new motion.
    fn all_actions() -> Vec<Action> {
        fn _assert_covers(a: &Action) {
            match a {
                Action::ForwardChar
                | Action::BackwardChar
                | Action::NextLine
                | Action::PreviousLine
                | Action::LineStart
                | Action::LineEnd
                | Action::ForwardWord
                | Action::BackwardWord
                | Action::BufferStart
                | Action::BufferEnd
                | Action::InsertChar(_)
                | Action::Newline
                | Action::InsertTab
                | Action::Outdent
                | Action::DeleteBackward
                | Action::DeleteWordBackward
                | Action::DeleteWordForward
                | Action::DeleteForward
                | Action::KillLine
                | Action::Yank
                | Action::Undo
                | Action::Redo
                | Action::SetMark
                | Action::CopyRegion
                | Action::KillRegion
                | Action::SelectAll
                | Action::ZoomIn
                | Action::ZoomOut
                | Action::ZoomReset
                | Action::PageScrollDown
                | Action::PageScrollUp
                | Action::Save
                | Action::Quit
                | Action::SearchForward
                | Action::SearchBackward
                | Action::OpenReplace
                | Action::Cancel
                | Action::OpenThemeMenu
                | Action::OpenCommandPalette
                | Action::OpenOutline
                | Action::OpenSpellSuggest
                | Action::ToggleCaretMode
                | Action::OpenCaretMenu
                | Action::OpenDictionaryMenu
                | Action::ToggleSpellcheck
                | Action::TogglePageMode
                | Action::PageWider
                | Action::PageNarrower
                | Action::PageReset
                | Action::CycleFocusMode
                | Action::ToggleDebug
                | Action::ToggleOutline
                | Action::ToggleTypewriter
                | Action::ToggleHiddenFiles
                | Action::ShowStatsHud
                | Action::OpenGoto
                | Action::OpenProject
                | Action::OpenRecentProjects
                | Action::OpenBrowse
                | Action::LastBuffer
                | Action::NewNote
                | Action::MoveNote
                | Action::OpenSettings
                | Action::OpenSettingsMenu
                | Action::OpenKeybindings
                | Action::OpenHistory
                | Action::KeepVersion
                | Action::FinishBuffer
                | Action::FollowLink
                | Action::BeginPrefix
                | Action::About
                | Action::ConvertLineEndings
                | Action::AlignTable
                | Action::ToggleBlockquote
                | Action::ToggleBulletList
                | Action::ToggleNumberedList
                | Action::ToggleTaskList
                | Action::ToggleHeading
                | Action::ToggleCodeBlock
                | Action::Bold
                | Action::Italic
                | Action::InlineCode
                | Action::Highlight
                | Action::Strikethrough
                | Action::Ignore => {}
            }
        }
        vec![
            Action::ForwardChar,
            Action::BackwardChar,
            Action::NextLine,
            Action::PreviousLine,
            Action::LineStart,
            Action::LineEnd,
            Action::ForwardWord,
            Action::BackwardWord,
            Action::BufferStart,
            Action::BufferEnd,
            Action::InsertChar('x'),
            Action::Newline,
            Action::InsertTab,
            Action::Outdent,
            Action::DeleteBackward,
            Action::DeleteWordBackward,
            Action::DeleteWordForward,
            Action::DeleteForward,
            Action::KillLine,
            Action::Yank,
            Action::Undo,
            Action::Redo,
            Action::SetMark,
            Action::CopyRegion,
            Action::KillRegion,
            Action::SelectAll,
            Action::ZoomIn,
            Action::ZoomOut,
            Action::ZoomReset,
            Action::PageScrollDown,
            Action::PageScrollUp,
            Action::Save,
            Action::Quit,
            Action::SearchForward,
            Action::SearchBackward,
            Action::OpenReplace,
            Action::Cancel,
            Action::OpenThemeMenu,
            Action::OpenCommandPalette,
            Action::OpenOutline,
            Action::OpenSpellSuggest,
            Action::ToggleCaretMode,
            Action::OpenCaretMenu,
            Action::OpenDictionaryMenu,
            Action::ToggleSpellcheck,
            Action::TogglePageMode,
            Action::PageWider,
            Action::PageNarrower,
            Action::PageReset,
            Action::CycleFocusMode,
            Action::ToggleDebug,
            Action::ToggleOutline,
            Action::ToggleTypewriter,
            Action::ToggleHiddenFiles,
            Action::ShowStatsHud,
            Action::OpenGoto,
            Action::OpenProject,
            Action::OpenRecentProjects,
            Action::OpenBrowse,
            Action::LastBuffer,
            Action::NewNote,
            Action::MoveNote,
            Action::OpenSettings,
            Action::OpenSettingsMenu,
            Action::OpenKeybindings,
            Action::OpenHistory,
            Action::KeepVersion,
            Action::FinishBuffer,
            Action::FollowLink,
            Action::BeginPrefix,
            Action::About,
            Action::ConvertLineEndings,
            Action::AlignTable,
            Action::ToggleBlockquote,
            Action::ToggleBulletList,
            Action::ToggleNumberedList,
            Action::ToggleTaskList,
            Action::ToggleHeading,
            Action::ToggleCodeBlock,
            Action::Bold,
            Action::Italic,
            Action::InlineCode,
            Action::Highlight,
            Action::Strikethrough,
            Action::Ignore,
        ]
    }

    #[test]
    fn every_classified_motion_extends_shift_selection_and_no_mover_is_missing() {
        // COMPLETENESS SWEEP over the hand-kept `Action::is_motion` list (the gate
        // of `apply_core`'s selection-on-motion state machine). For EVERY variant
        // (the list is compile-time-complete — see `all_actions`), driven with
        // shift=true from a mid-document cursor:
        //   * a classified MOTION must set the mark at the pre-motion cursor,
        //     leave an extended selection, and keep that anchor across a RUN;
        //   * an action that MOVED the cursor WITHOUT editing must BE classified
        //     a motion — unless it is a documented non-motion mover below — so a
        //     NEW motion variant missing from the hand-kept list fails HERE
        //     instead of silently not extending under Shift.
        // Several arms flip process-globals (page/caret/focus/debug/hud/about), so
        // hold those TEST_LOCKs (page before caret — the shared ordering the
        // config sticky-globals test established) and snapshot/restore the
        // globals. `about` joins this set because the sweep drives
        // `Action::About` (which opens the card) through the SAME apply_core
        // seam every other action in the sweep rides — a concurrent test
        // flipping the about global without this lock would otherwise leak its
        // state into (or steal it from) this sweep's iterations.
        let _pg = crate::page::test_lock();
        let _ca = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _fo = crate::focus::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _db = crate::debug::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _hu = crate::hud::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _sp = crate::spell::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ab = crate::about::test_lock();
        let caret0 = crate::caret::mode();
        let page0 = crate::page::page_on();
        let measure0 = crate::page::measure();
        let focus0 = crate::focus::mode();
        let debug0 = crate::debug::debug_on();
        let hud0 = crate::hud::hud_held();
        let spellcheck0 = crate::spell::spellcheck_on();
        let about0 = crate::about::about_open();

        // Deliberately NON-motion actions that still MOVE the cursor: SelectAll
        // sets its own discrete region (not a Shift-extend), and the page scrolls
        // are viewport-page moves excluded from `is_motion` (Shift+PageDown does
        // not extend today). A new mover belongs in `is_motion` or — consciously
        // — here.
        let exempt_movers = [
            Action::SelectAll,
            Action::PageScrollDown,
            Action::PageScrollUp,
        ];

        for action in all_actions() {
            let mut b = Buffer::from_str("alpha beta\ngamma delta\nepsilon zeta\n");
            b.set_cursor(14); // line 1, col 3 — no wall in any direction
            let cursor0 = b.cursor_char();
            let version0 = b.version();
            let mut sel = false;
            drive_shift(&mut b, &mut sel, &action, true);
            if action.is_motion() {
                assert_eq!(
                    b.anchor_char(),
                    Some(cursor0),
                    "{action:?}: first shift-motion must set the mark at the pre-motion cursor"
                );
                assert_ne!(
                    b.cursor_char(),
                    cursor0,
                    "{action:?}: a mid-document motion must move the point"
                );
                assert!(b.has_selection(), "{action:?}: shift-motion must leave a selection");
                assert!(sel, "{action:?}: the transient shift flag must arm");
                // The RUN extends: a second shifted motion keeps the same anchor.
                drive_shift(&mut b, &mut sel, &action, true);
                assert_eq!(
                    b.anchor_char(),
                    Some(cursor0),
                    "{action:?}: a shift RUN must keep the anchor"
                );
            } else if b.version() == version0 && b.cursor_char() != cursor0 {
                assert!(
                    exempt_movers.contains(&action),
                    "{action:?} moved the cursor without editing but is not in \
                     Action::is_motion — a new motion is missing from the hand-kept list"
                );
            }
            // `Action::About` OPENS the About card (a process global, unlike every
            // other sweep member here) — reset it after each iteration so it can
            // never leak into the NEXT action in this same sweep (apply_core's
            // top-of-function About-dismiss intercept would otherwise swallow it).
            crate::about::set_open(false);
        }

        // Leave the process-globals exactly as found.
        crate::caret::set_mode(caret0);
        crate::page::set_page_on(page0);
        crate::page::set_measure(measure0);
        crate::focus::set_mode(focus0);
        crate::debug::set_debug_on(debug0);
        crate::hud::set_held(hud0);
        crate::spell::set_spellcheck_on(spellcheck0);
        crate::about::set_open(about0);
    }

    #[test]
    fn history_restore_via_set_text_is_one_undoable_edit() {
        // The RESTORE mechanism (App::restore_history) is `Buffer::set_text(version)`.
        // Prove it round-trips undoably: typing builds "edited", a restore swaps in the
        // old "version one", and a single C-/ undo brings "edited" back.
        let mut buffer = Buffer::scratch();
        for c in "edited".chars() {
            buffer.insert_char(c);
        }
        buffer.seal_undo_group();
        assert_eq!(buffer.text(), "edited");
        buffer.set_text("version one"); // the restore edit (set_text seals its own group)
        assert_eq!(buffer.text(), "version one");
        buffer.undo();
        assert_eq!(buffer.text(), "edited", "C-/ undoes the restore");
    }
