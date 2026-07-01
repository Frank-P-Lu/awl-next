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
    fn browse_left_ascends() {
        // Start one level deep (in docs/).
        let mut overlay: Option<OverlayState> =
            browse_level(OverlayKind::Browse, Some("docs".to_string()));
        let mut accept = None;
        // Left ASCENDS back to the root level.
        drive(&mut overlay, &mut accept, &Action::BackwardChar);
        let ov = overlay.as_ref().expect("still open after ascend");
        assert_eq!(ov.browse_dir, None, "ascend from docs -> root");
        assert!(ov.item_strings().iter().any(|s| s.contains("docs")));
        // Left at the root is a no-op (stays at root, still open).
        drive(&mut overlay, &mut accept, &Action::BackwardChar);
        assert_eq!(overlay.as_ref().unwrap().browse_dir, None);
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
    /// exactly like the windowed app's hook (folders-only + synthetic "." row).
    fn project_browse(ws: &std::path::Path, rel: Option<String>) -> Option<OverlayState> {
        let dir = rel.unwrap_or_else(|| ws.to_string_lossy().to_string());
        let folders: Vec<(String, bool)> =
            crate::index::list_dir_level(std::path::Path::new(&dir), None)
                .into_iter()
                .filter(|e| e.is_dir)
                .map(|e| (e.name, e.is_git))
                .collect();
        Some(OverlayState::new_project(dir, folders))
    }

    #[test]
    fn switch_project_enter_picks_highlighted_folder() {
        let (ws, _fs) = proj_tree();
        let mut browse_to = |k: OverlayKind, rel: Option<String>| {
            assert_eq!(k, OverlayKind::Project);
            project_browse(&ws, rel)
        };
        // Open at ws: corpus is [".", child-a, child-b], default-selected on the
        // first real folder (child-a). Enter PICKS it as the new root (the primary
        // action of the project picker) — it does NOT descend.
        let mut overlay = browse_to(OverlayKind::Project, None);
        let mut accept = None;
        assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("child-a"));
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::Newline);
        assert!(overlay.is_none(), "Enter on a folder PICKS it and closes");
        assert_eq!(
            accept,
            Some((
                OverlayKind::Project,
                ws.join("child-a").to_string_lossy().to_string()
            )),
            "Enter accepts the highlighted folder's ABSOLUTE path"
        );
    }

    #[test]
    fn switch_project_right_descends_into_child() {
        let (ws, _fs) = proj_tree();
        let mut browse_to = |k: OverlayKind, rel: Option<String>| {
            assert_eq!(k, OverlayKind::Project);
            project_browse(&ws, rel)
        };
        // Open at ws, selection on child-a. Right DESCENDS into it (drill in to
        // pick a subfolder); the overlay stays open with child-a's contents.
        let mut overlay = browse_to(OverlayKind::Project, None);
        let mut accept = None;
        assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("child-a"));
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::ForwardChar);
        let ov = overlay.as_ref().expect("still open after descend");
        assert_eq!(
            ov.browse_dir.as_deref(),
            Some(ws.join("child-a").to_string_lossy().as_ref())
        );
        // The descended level lists child-a's subfolder `sub`.
        assert!(ov.item_strings().iter().any(|s| s.contains("sub")), "{:?}", ov.item_strings());
        assert!(accept.is_none(), "descend must not accept");
        // Right again descends (into `sub`).
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::ForwardChar);
        assert_eq!(
            overlay.as_ref().unwrap().browse_dir.as_deref(),
            Some(ws.join("child-a/sub").to_string_lossy().as_ref())
        );
        // `sub` has no subfolders, so selection rests on the "." row; Enter there
        // PICKS the drilled-in current directory (child-a/sub) as the root.
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::Newline);
        assert!(overlay.is_none(), "Enter picks the drilled-in directory");
        assert_eq!(
            accept,
            Some((
                OverlayKind::Project,
                ws.join("child-a/sub").to_string_lossy().to_string()
            )),
            "drilled-in pick is its absolute path"
        );
    }

    /// C-f / C-b reach the navigable intercept AS ForwardChar / BackwardChar while
    /// the overlay is open (the keymap is overlay-unaware, so the chord resolves the
    /// same as the arrows). Resolve the chords through the REAL keymap, then drive
    /// the resulting actions: C-f DESCENDS into the highlighted child (same as Right)
    /// and C-b ASCENDS to the parent (same as Left).
    #[test]
    fn switch_project_c_f_descends_c_b_ascends() {
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
        assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("child-a"));
        // C-f (ForwardChar) DESCENDS into child-a, overlay still open at its level.
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &c_f);
        let ov = overlay.as_ref().expect("still open after C-f descend");
        assert_eq!(
            ov.browse_dir.as_deref(),
            Some(ws.join("child-a").to_string_lossy().as_ref())
        );
        assert!(accept.is_none(), "descend must not accept");
        // C-b (BackwardChar) ASCENDS back to the workspace level.
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &c_b);
        assert_eq!(
            overlay.as_ref().unwrap().browse_dir.as_deref(),
            Some(ws.to_string_lossy().as_ref()),
            "C-b ascends back to the workspace"
        );
    }

    /// Enter on a Project FOLDER SELECTS it as the root (does NOT descend): the
    /// overlay closes and the accept value is that folder's absolute path. Descending
    /// is Right / C-f only. (Companion to `switch_project_right_descends_into_child`.)
    #[test]
    fn switch_project_enter_selects_does_not_descend() {
        let (ws, _fs) = proj_tree();
        let mut browse_to = |k: OverlayKind, rel: Option<String>| {
            assert_eq!(k, OverlayKind::Project);
            project_browse(&ws, rel)
        };
        let mut overlay = browse_to(OverlayKind::Project, None);
        let mut accept = None;
        assert_eq!(overlay.as_ref().unwrap().selected_value(), Some("child-a"));
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::Newline);
        assert!(overlay.is_none(), "Enter on a folder SELECTS + closes (no descend)");
        assert_eq!(
            accept,
            Some((
                OverlayKind::Project,
                ws.join("child-a").to_string_lossy().to_string()
            )),
            "Enter selects the highlighted folder, it does not drill into it"
        );
    }

    #[test]
    fn switch_project_ascends_to_parent() {
        let (ws, _fs) = proj_tree();
        let mut browse_to = |_k: OverlayKind, rel: Option<String>| project_browse(&ws, rel);
        let mut overlay = browse_to(OverlayKind::Project, None);
        let mut accept = None;
        // Backspace (empty query) ASCENDS to ws's PARENT — ABOVE the workspace.
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::DeleteBackward);
        let parent = ws.parent().unwrap().to_string_lossy().to_string();
        let ov = overlay.as_ref().unwrap();
        assert_eq!(ov.browse_dir.as_deref(), Some(parent.as_str()));
        // ws itself now appears as a child folder of its parent.
        let ws_name = ws.file_name().unwrap().to_str().unwrap();
        assert!(ov.item_strings().iter().any(|s| s.contains(ws_name)));
        // Left ascends one MORE level (no root floor for Project).
        drive_bt(&mut overlay, &mut accept, &mut browse_to, &Action::BackwardChar);
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
        // Opens highlighting the active world (Tawny), still active.
        assert_eq!(crate::theme::active().name, "Tawny");
        // Down once previews world index 1 (Potoroo) IMMEDIATELY.
        drive(&mut overlay, &mut accept, &Action::NextLine);
        assert_eq!(crate::theme::active().name, crate::theme::THEMES[1].name);
        // Down again previews world index 2.
        drive(&mut overlay, &mut accept, &Action::NextLine);
        assert_eq!(crate::theme::active().name, crate::theme::THEMES[2].name);
        assert_eq!(overlay.as_ref().unwrap().selected, 2);
        crate::theme::set_active(0);
    }

    #[test]
    fn theme_enter_commits_previewed_world() {
        let _g = THEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::theme::set_active(0);
        let mut overlay = theme_overlay();
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::NextLine); // preview world 1
        drive(&mut overlay, &mut accept, &Action::Newline); // COMMIT
        assert!(overlay.is_none(), "Enter closes the picker");
        assert_eq!(crate::theme::active().name, crate::theme::THEMES[1].name);
        assert_eq!(
            accept,
            Some((OverlayKind::Theme, crate::theme::THEMES[1].name.to_string()))
        );
        crate::theme::set_active(0);
    }

    #[test]
    fn theme_cancel_reverts_to_starting_world() {
        let _g = THEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::theme::set_active(0); // start on Tawny
        let mut overlay = theme_overlay();
        let mut accept = None;
        drive(&mut overlay, &mut accept, &Action::NextLine); // preview world 1
        drive(&mut overlay, &mut accept, &Action::NextLine); // preview world 2
        assert_eq!(crate::theme::active().name, crate::theme::THEMES[2].name);
        drive(&mut overlay, &mut accept, &Action::Cancel); // REVERT
        assert!(overlay.is_none(), "Cancel closes the picker");
        assert_eq!(crate::theme::active().name, "Tawny", "reverted to start");
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

    #[test]
    fn smart_newline_parser_declines_plain_and_inside_marker() {
        // Plain prose: nothing to continue.
        assert!(smart_newline_for("hello", 5).is_none());
        // Caret inside the marker (col 0 of a bullet): plain newline, no dupe.
        assert!(smart_newline_for("- item", 0).is_none());
        // A lone "-" without a trailing space is not a list yet.
        assert!(smart_newline_for("-", 1).is_none());
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

    // --- RECOIL PRIMITIVE: blocked-action trigger logic ----------------------

    /// Drive one action through `apply_core` against a fresh buffer seeded with
    /// `text` and the cursor at char `cursor`, returning the resulting [`Effect`].
    /// No oracle (logical-line fallback), so vertical motion uses the buffer lines.
    fn drive_effect(text: &str, cursor: usize, action: &Action) -> Effect {
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
        apply_core(&mut ctx, action, false)
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
        // Backspace at buffer start / C-d at buffer end remove nothing -> recoil.
        assert_eq!(drive_effect("hi", 0, &Action::DeleteBackward), Effect::Recoil(Right));
        assert_eq!(drive_effect("hi", 2, &Action::DeleteForward), Effect::Recoil(Left));
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
        // A kill-line that removes text gulps.
        assert_eq!(drive_effect("hello", 0, &Action::KillLine), Effect::Gulp);
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

    #[test]
    fn line_edge_motions_do_not_recoil_even_at_the_edge() {
        // C-a at col 0 / C-e at line end are common idempotent presses; we
        // deliberately do NOT recoil there (it would be noisy). They report None.
        assert_eq!(drive_effect("abc", 0, &Action::LineStart), Effect::None);
        assert_eq!(drive_effect("abc", 3, &Action::LineEnd), Effect::None);
    }
