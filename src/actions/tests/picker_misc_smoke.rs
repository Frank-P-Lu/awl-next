//! Search/replace driving, history-picker restore, the About/Lifetime-Stats
//! summoned cards, shift-selection, and the full every-command apply-seam
//! smoke sweep -- split out of the former monolithic `actions::tests`
//! (2026-07 code-organization pass).

use super::super::*;
use crate::overlay::OverlayKind;
use super::{browse_level, drive, drive_search, drive_shift, all_actions, smoke_command_kind, rich_markdown_buffer, SmokeKind};

#[test]
fn tab_without_a_search_is_a_plain_soft_tab_through_core() {
    // With NO search live, Tab is a plain soft-tab insert. The IN-PANEL Tab
    // (reveal / flip the replace field) deliberately no longer lives in
    // `apply_core` at all: while the panel is open EVERY key is consumed
    // BEFORE keymap resolution by the ONE shared interception seam
    // (`crate::search::keys::intercept` — the live guard and the headless
    // replay guard are the same code, and its own tests cover the panel's
    // whole operation set), so `apply_core` can never see an in-panel key.
    let mut b = Buffer::from_str("alpha beta alpha");
    b.set_cursor(0);
    let mut search = None;
    drive_search(&mut b, &mut search, &Action::InsertTab);
    assert!(search.is_none());
    assert!(b.text().starts_with(' '), "Tab without a search inserts a soft tab");
}

#[test]
fn cmd_r_opens_replace_revealed_with_focus_on_find_through_core() {
    // Cmd-R with NO search open (Action::OpenReplace) opens the panel with the
    // replace row REVEALED but focus on the FIND field — the redesigned headline
    // door, drivable through the core so `--keys "Cmd-r"` sets the sidecar.
    // (Cmd-R / Tab WITHIN the already-open panel are the search-key seam's job
    // — `crate::search::keys::tests::tab_and_cmd_r_move_between_the_two_fields`
    // — never `apply_core`'s: the shared guard consumes them first.)
    let mut b = Buffer::from_str("alpha beta alpha");
    b.set_cursor(0);
    let mut search = None;
    drive_search(&mut b, &mut search, &Action::OpenReplace);
    let st = search.as_ref().expect("Cmd-R opens the search panel");
    assert!(st.is_replace_active(), "the replace row is revealed on open");
    assert!(!st.is_editing_replacement(), "focus opens on the find field");
    assert_eq!(b.text(), "alpha beta alpha", "opening never touches the document");
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
fn history_picker_tab_emits_compare_version_of_the_highlighted_row() {
    // THE WRITER'S DIFF from the picker: TAB over a highlighted version emits
    // Effect::CompareVersion(<id>) (not a restore) and CLOSES the picker — the
    // caller resolves the id via history::load + renders the read-only diff view.
    let row = |id: &str| crate::history::TimelineRow {
        when: "just now".into(),
        which: String::new(),
        counts: "+0 −0".into(),
        id: id.to_string(),
        timestamp: id.parse().unwrap_or(0),
        pinned: false,
    };
    let mut overlay = Some(OverlayState::new_history(
        vec![row("300"), row("200"), row("100")],
        None,
        None,
    ));
    // Highlight the middle row, then TAB (Action::InsertTab) to compare it.
    let mut accept = None;
    drive(&mut overlay, &mut accept, &Action::NextLine);
    // Drive Tab directly through the core to inspect the returned effect.
    let mut buffer = Buffer::scratch();
    let (mut shift, mut zoom, mut search) = (false, 1.0f32, None);
    let mut make_overlay = |_k: OverlayKind| None;
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
    let eff = apply_core(&mut ctx, &Action::InsertTab, false);
    assert_eq!(eff, Effect::CompareVersion("200".to_string()), "Tab compares the highlighted row");
    assert!(overlay.is_none(), "compare closes the picker (you navigated into the diff)");
    assert_eq!(buffer.text(), "", "compare never touches the buffer in the core");
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

#[test]
fn about_opens_and_any_key_dismisses_it() {
    // `Action::About` OPENS the summoned card (a process global, mirroring
    // `hud`/`debug` — see `about.rs`); the VERY NEXT key through `apply_core`
    // (ANY action at all — a plain motion here, deliberately not Esc) closes
    // it again and is otherwise consumed (no other effect: the cursor must
    // not move even though `ForwardChar` normally would).
    let _g = crate::testlock::serial();
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
fn lifetime_stats_opens_and_any_key_dismisses_it() {
    // `Action::LifetimeStats` OPENS the summoned card (a process global,
    // mirroring About — see `lifetime.rs`); the VERY NEXT key through
    // `apply_core` (ANY action — a plain motion here, deliberately not Esc)
    // closes it again and is otherwise consumed (the cursor must not move even
    // though `ForwardChar` normally would).
    let _g = crate::testlock::serial();
    crate::lifetime::set_open(false);
    let mut b = Buffer::from_str("alpha beta");
    let mut sel = false;
    let cursor0 = b.cursor_char();

    drive_shift(&mut b, &mut sel, &Action::LifetimeStats, false);
    assert!(crate::lifetime::lifetime_open(), "Action::LifetimeStats opens the card");
    assert_eq!(b.cursor_char(), cursor0, "opening the card never touches the buffer");

    // ANY key — a plain forward-char motion, not Esc — dismisses it and is
    // fully consumed: the motion must NOT actually move the cursor.
    drive_shift(&mut b, &mut sel, &Action::ForwardChar, false);
    assert!(!crate::lifetime::lifetime_open(), "the next key closes the card");
    assert_eq!(b.cursor_char(), cursor0, "the dismissing key is consumed, not applied");

    // Once closed, the SAME action now runs normally (proves the intercept only
    // fires while the card is actually open).
    drive_shift(&mut b, &mut sel, &Action::ForwardChar, false);
    assert_eq!(b.cursor_char(), cursor0 + 1, "ForwardChar works again once the card is closed");

    crate::lifetime::set_open(false);
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
    // hold those TEST_LOCKs and snapshot/restore the globals. `about` is in this
    // set because the sweep drives `Action::About` (which opens the card)
    // through the SAME apply_core seam every other action rides — a concurrent
    // test flipping the about global without this lock would otherwise leak its
    // state into (or steal it from) this sweep's iterations. Order is safe
    // regardless: `apply_core` releases about/lifetime before any page writer
    // (see its SCOPE note), so page-then-about here can never ABBA it.
    let _pg = crate::testlock::serial();
    let _ca = crate::testlock::serial();
    let _db = crate::testlock::serial();
    let _hu = crate::testlock::serial();
    let _sp = crate::testlock::serial();
    let _ab = crate::testlock::serial();
    // `Action::ToggleWritingNits` is in `all_actions()` and flips the nits global
    // through this same seam, so hold its lock + snapshot/restore too.
    let _nt = crate::testlock::serial();
    let caret0 = crate::caret::mode();
    let page0 = crate::page::page_on();
    let measure0 = crate::page::measure();
    let debug0 = crate::debug::debug_on();
    let hud0 = crate::hud::hud_held();
    let spellcheck0 = crate::spell::spellcheck_on();
    let about0 = crate::about::about_open();
    let nits0 = crate::nits::nits_on();

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
        // The Lifetime stats card's open-global has the SAME property (its own
        // dismiss intercept), so reset it too.
        crate::about::set_open(false);
        crate::lifetime::set_open(false);
    }

    // Leave the process-globals exactly as found.
    crate::caret::set_mode(caret0);
    crate::page::set_page_on(page0);
    crate::page::set_measure(measure0);
    crate::debug::set_debug_on(debug0);
    crate::hud::set_held(hud0);
    crate::spell::set_spellcheck_on(spellcheck0);
    crate::about::set_open(about0);
    crate::nits::set_nits_on(nits0);
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

#[test]
fn every_catalog_command_dispatches_without_panicking() {
    // Many catalog arms flip a process-global (page / caret / debug / hud / spell
    // / nits / about / lifetime / outline / typewriter) or READ one while
    // building an overlay, so hold each global's TEST_LOCK and snapshot /
    // restore every one, so this sweep leaves NO residue and never races a
    // concurrent reader. `about`/`lifetime` are in the set because the sweep
    // drives `Action::About` / `Action::LifetimeStats` via the same apply_core
    // seam. Order is safe regardless: `apply_core` releases about/lifetime
    // before any page writer (see its SCOPE note), so page-then-about/lifetime
    // here can never ABBA it.
    let _pg = crate::testlock::serial();
    let _ca = crate::testlock::serial();
    let _db = crate::testlock::serial();
    let _hu = crate::testlock::serial();
    let _sp = crate::testlock::serial();
    let _ab = crate::testlock::serial();
    let _lf = crate::testlock::serial();
    let _ol = crate::testlock::serial();
    let _tw = crate::testlock::serial();
    let _nt = crate::testlock::serial();
    let caret0 = crate::caret::mode();
    let page0 = crate::page::page_on();
    let measure0 = crate::page::measure();
    let debug0 = crate::debug::debug_on();
    let hud0 = crate::hud::hud_held();
    let spellcheck0 = crate::spell::spellcheck_on();
    let about0 = crate::about::about_open();
    let lifetime0 = crate::lifetime::lifetime_open();
    let outline0 = crate::outline::outline_on();
    let typewriter0 = crate::typewriter::typewriter_on();
    let nits0 = crate::nits::nits_on();

    // The overlay-build context: fed enough for EVERY summoning command to open
    // (a go-to corpus with a folded-in heading, a spell target) —
    // History/Theme/Caret/Dictionary/Settings/Keybindings always open. The
    // navigable explorers (Project / Browse / MoveDest, incl. "Recent projects…"
    // pre-lensed) open via `browse_to` below (the shared `browse_level` fixture),
    // not this ctx.
    let bctx = crate::overlay::BuildCtx {
        goto_corpus: vec!["README.md".to_string(), "src/main.rs".to_string()],
        goto_open: vec![],
        goto_recent: vec![],
        goto_times: vec![],
        config_keys: &[],
        config_linux_keep: &[],
        goto_headings: vec![("Heading One".to_string(), 0)],
        spell_target: Some((vec!["speling".to_string(), "spieling".to_string()], (0, 0, 3))),
        history_entries: vec![],
        history_now: None,
        history_session_start: None,
        settings_values: crate::settings::SettingsValues::default(),
        assets: vec![],
    };

    for c in crate::commands::COMMANDS.iter() {
        // The About card's "open" global OWNS the very next key (apply_core's
        // top-of-fn dismiss intercept), so reset it before EACH dispatch — else
        // a prior `Action::About` iteration would make the next command a no-op
        // dismiss instead of running it. The Lifetime stats card has the SAME
        // any-key-dismiss intercept, so reset it too.
        crate::about::set_open(false);
        crate::lifetime::set_open(false);

        let mut buffer = rich_markdown_buffer();
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut overlay: Option<OverlayState> = None;

        // Dispatch through the REAL seam in an inner scope so `ctx`'s borrows of
        // `buffer`/`overlay` end before the coherence reads below.
        let eff = {
            let mut make_overlay = |kind: OverlayKind| crate::overlay::build(kind, &bctx);
            let mut browse_to =
                |kind: OverlayKind, rel: Option<String>| browse_level(kind, rel);
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
            apply_core(&mut ctx, &c.action, false)
        };

        // COHERENCE: the buffer is still a valid rope and the cursor is in range
        // (a panic anywhere above would already have failed the test — this is
        // the cheap "did it survive intact" confirmation).
        let n = buffer.text().chars().count();
        assert!(
            buffer.cursor_char() <= n,
            "{}: cursor {} out of bounds ({} chars) after dispatch",
            c.name,
            buffer.cursor_char(),
            n
        );

        let kind = smoke_command_kind(&c.action);
        assert_ne!(
            kind,
            SmokeKind::NotCatalog,
            "{}: a catalog command must not be classified NotCatalog (add it under the sweep)",
            c.name
        );
        match kind {
            // A summon must have opened an overlay this frame.
            SmokeKind::Opener => assert!(
                overlay.is_some(),
                "{}: an overlay-summoning command left no overlay open",
                c.name
            ),
            // The exact deferred effect the core signals back.
            SmokeKind::Deferred => {
                let ok = match &c.action {
                    Action::Quit => eff == Effect::Quit,
                    Action::LastBuffer => eff == Effect::LastBuffer,
                    Action::NewNote => eff == Effect::NewNote,
                    Action::OpenCredits => eff == Effect::OpenCredits,
                    Action::OpenGuide => eff == Effect::OpenGuide,
                    Action::KeepVersion => eff == Effect::KeepVersion,
                    // Markdown fixture: Compare with version… defers the latest-version
                    // resolve + diff-view open to the live App.
                    Action::CompareVersion => eff == Effect::CompareLatest,
                    Action::FinishBuffer => eff == Effect::FinishBuffer,
                    // Caret sits inside the fixture link, so a URL resolves.
                    Action::FollowLink => matches!(eff, Effect::FollowLink(_)),
                    Action::ReportProblem => eff == Effect::ReportProblem,
                    // WEB-ONLY: this sweep drives the RAW `COMMANDS` catalog on
                    // the native test binary, where `web_only: true` gates
                    // `DownloadFile` off entirely at `apply_core`'s dispatch
                    // belt (`commands::action_available`) — the effect is
                    // structurally `None` here, never `Effect::DownloadFile`
                    // (that only ever fires under `Platform::Web`; see
                    // `commands.rs`'s `action_available` doc).
                    Action::DownloadFile => eff == Effect::None,
                    Action::CheckForUpdates => eff == Effect::CheckForUpdates,
                    Action::DuplicateNote => eff == Effect::DuplicateNote,
                    // The smoke fixture is a markdown buffer, so export signals
                    // its format for the live App to render + write.
                    Action::ExportWord => eff == Effect::Export(crate::export::Format::Docx),
                    Action::ExportHtml => eff == Effect::Export(crate::export::Format::Html),
                    // PDF export is native-only; on wasm `Format::Pdf` does not exist
                    // and the apply arm yields no effect.
                    #[cfg(not(target_arch = "wasm32"))]
                    Action::ExportPdf => eff == Effect::Export(crate::export::Format::Pdf),
                    #[cfg(target_arch = "wasm32")]
                    Action::ExportPdf => eff == Effect::None,
                    other => panic!("{other:?} classified Deferred but has no effect check"),
                };
                assert!(ok, "{}: unexpected deferred effect {:?}", c.name, eff);
            }
            // In-place commands: no panic is the assertion; About also flips its
            // global (checked so a broken card summon is caught).
            SmokeKind::InPlace => {
                if c.action == Action::About {
                    assert!(
                        crate::about::about_open(),
                        "About must summon the card (its open global)"
                    );
                }
                if c.action == Action::LifetimeStats {
                    assert!(
                        crate::lifetime::lifetime_open(),
                        "Lifetime stats must summon the card (its open global)"
                    );
                }
            }
            SmokeKind::NotCatalog => unreachable!("guarded by the assert above"),
        }
    }

    // Leave every process-global exactly as found.
    crate::caret::set_mode(caret0);
    crate::page::set_page_on(page0);
    crate::page::set_measure(measure0);
    crate::debug::set_debug_on(debug0);
    crate::hud::set_held(hud0);
    crate::spell::set_spellcheck_on(spellcheck0);
    crate::about::set_open(about0);
    crate::lifetime::set_open(lifetime0);
    crate::outline::set_outline_on(outline0);
    crate::typewriter::set_typewriter_on(typewriter0);
    crate::nits::set_nits_on(nits0);
}
