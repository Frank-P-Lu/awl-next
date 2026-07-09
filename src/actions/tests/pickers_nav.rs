//! Command palette, go-to, spell-suggest, browse, switch-project, and theme
//! picker driving -- lens cycling, ascend/descend, live preview -- split out
//! of the former monolithic `actions::tests` (2026-07 code-organization
//! pass).

use super::super::*;
use crate::overlay::OverlayKind;
use super::{browse_level, drive, drive_run, drive_bt, proj_tree, project_browse, theme_overlay, THEME_LOCK};

#[test]
fn command_palette_opens_then_filters() {
    // OpenCommandPalette summons the palette via make_overlay.
    let mut overlay: Option<OverlayState> = None;
    let mut accept = None;
    drive(&mut overlay, &mut accept, &Action::OpenCommandPalette);
    let ov = overlay.as_ref().expect("palette opened");
    assert_eq!(ov.kind, OverlayKind::Command);
    // Typing "theme" fuzzy-narrows to "Switch theme…" at/near the top.
    for c in "theme".chars() {
        drive(&mut overlay, &mut accept, &Action::InsertChar(c));
    }
    let ov = overlay.as_ref().unwrap();
    assert_eq!(ov.selected_value(), Some("Switch theme…"));
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
fn go_to_heading_opens_filters_and_jumps_to_line() {
    // "Go to heading…" (the retired Outline picker) opens GO-TO pre-lensed onto
    // its HEADINGS lens; Enter on the filtered heading row emits Effect::JumpToLine
    // for the caller to move the cursor — NOT an OverlayAccept file-open.
    let mut overlay: Option<OverlayState> = None;
    let mut jumped: Option<usize> = None;
    let mut buffer = Buffer::scratch();
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut make_overlay = |k: OverlayKind| match k {
        OverlayKind::Goto => {
            let mut ov =
                OverlayState::new(OverlayKind::Goto, vec!["notes.md".into()], vec![], vec![]);
            ov.attach_headings(vec![
                ("Intro".into(), 0usize),
                ("Details".into(), 7usize),
                ("Wrap up".into(), 20usize),
            ]);
            Some(ov)
        }
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
        // "Go to heading…" -> Go-to opens pre-lensed onto the Headings lens.
        apply_core(&mut ctx, &Action::OpenOutline, false);
        let ov = ctx.overlay.as_ref().unwrap();
        assert_eq!(ov.kind, OverlayKind::Goto);
        assert_eq!(ov.active_facet_id(), Some("headings"));
        // Filter to "Details" ...
        for c in "deta".chars() {
            apply_core(&mut ctx, &Action::InsertChar(c), false);
        }
        assert_eq!(ctx.overlay.as_ref().unwrap().selected_value(), Some("Details"));
        // Enter JUMPS to its line (7) and closes.
        if let Effect::JumpToLine(line) = apply_core(&mut ctx, &Action::Newline, false) {
            jumped = Some(line);
        }
    }
    assert!(overlay.is_none(), "go-to closes on a heading accept");
    assert_eq!(jumped, Some(7));
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
    // -> By type -> Headings (the fold that retired the Outline picker), driven
    // through the real `apply_core` overlay intercept (so a `--keys "C-x f <right>"`
    // capture reaches the same code).
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
    drive(&mut overlay, &mut accept, &Action::ForwardChar);
    assert_eq!(overlay.as_ref().unwrap().active_facet_id(), Some("headings"));
    // RIGHT at the last lens clamps.
    drive(&mut overlay, &mut accept, &Action::ForwardChar);
    assert_eq!(
        overlay.as_ref().unwrap().active_facet_id(),
        Some("headings"),
        "clamp at last lens"
    );
    // LEFT walks all the way back to the All home.
    for _ in 0..4 {
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

#[test]
fn recent_projects_opens_switch_project_on_the_recent_lens() {
    // THE FOLD: "Recent projects…" (Action::OpenRecentProjects) opens the
    // SWITCH-PROJECT navigator pre-lensed onto its Recent lens — the fold that
    // retired the standalone RecentProjects picker. Driven through the real
    // `apply_core` seam with the shared `project_browse` navigator hook.
    let (ws, _fs) = proj_tree();
    let mut overlay: Option<OverlayState> = None;
    let mut buffer = Buffer::scratch();
    let mut shift = false;
    let mut zoom = 1.0;
    let mut search = None;
    let mut make_overlay = |_k: OverlayKind| None;
    let mut browse_to = |_k: OverlayKind, rel: Option<String>| project_browse(&ws, rel);
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
        apply_core(&mut ctx, &Action::OpenRecentProjects, false);
    }
    let ov = overlay.as_ref().expect("Recent projects opens the navigator");
    assert_eq!(ov.kind, OverlayKind::Project, "it IS the switch-project navigator");
    assert_eq!(ov.active_facet_id(), Some("recent"), "pre-lensed onto the Recent lens");
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

/// BREADCRUMB POP — a Theme picker opened FROM the palette (return_to = Command)
/// POPS back to the palette on Esc (still reverting the previewed world), NOT to
/// the buffer. `drive`'s make_overlay re-summons a Command palette for exactly
/// this. (The re-summoned palette carries no breadcrumb of its own — single-level.)
#[test]
fn theme_from_palette_pops_back_to_palette_on_esc() {
    let _g = THEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::theme::set_active(0);
    let mut overlay = theme_overlay();
    overlay.as_mut().unwrap().return_to = Some(OverlayKind::Command);
    let mut accept = None;
    drive(&mut overlay, &mut accept, &Action::NextLine); // preview off Tawny
    assert_ne!(crate::theme::active().name, "Tawny");
    drive(&mut overlay, &mut accept, &Action::Cancel); // Esc → POP, not close
    let ov = overlay.as_ref().expect("Esc pops back to the palette, not the buffer");
    assert_eq!(ov.kind, OverlayKind::Command, "re-summoned the command palette");
    assert_eq!(ov.return_to, None, "single-level: the palette carries no breadcrumb");
    assert_eq!(crate::theme::active().name, "Tawny", "the preview still reverted");
    crate::theme::set_active(0);
}

/// SHIP-BLOCKER REGRESSION — a VALUE-PICKING accept (Enter/keep) on a Theme picker
/// launched FROM THE COMMAND PALETTE lands in the BUFFER, NOT back in the palette.
/// The palette is a one-shot launcher; picking a theme COMPLETES the launched
/// command, so re-opening the launcher (which re-appears on its Recent lens) — the
/// user-reported "Switch theme → select → it goes into the recent files menu" —
/// must not happen. The commit still fires (`accept` carries the kept world). Esc
/// (not accept) still pops back to the palette; see the sibling `_on_esc` test.
#[test]
fn theme_from_palette_closes_to_buffer_on_keep_not_a_recent_menu() {
    let _g = THEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::theme::set_active(0);
    let mut overlay = theme_overlay();
    overlay.as_mut().unwrap().return_to = Some(OverlayKind::Command);
    let mut accept = None;
    drive(&mut overlay, &mut accept, &Action::NextLine); // preview the next world
    let previewed = overlay.as_ref().unwrap().selected_value().unwrap().to_string();
    drive(&mut overlay, &mut accept, &Action::Newline); // keep → CLOSE to buffer
    assert!(overlay.is_none(), "keeping a palette-launched theme lands in the buffer");
    assert_eq!(accept, Some((OverlayKind::Theme, previewed)), "the keep still committed");
    crate::theme::set_active(0);
}

/// The COUNTERPART: a value-pick launched FROM SETTINGS (a configuration surface
/// you keep using) DOES pop back to Settings on commit — the genuine "keep
/// configuring" breadcrumb the palette case must not share. Only the summoning
/// overlay's `retains_value_pick_child` differs; the accept path is identical.
#[test]
fn theme_from_settings_pops_back_to_settings_on_keep() {
    let _g = THEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::theme::set_active(0);
    let mut overlay = theme_overlay();
    overlay.as_mut().unwrap().return_to = Some(OverlayKind::Settings);
    let mut accept = None;
    drive(&mut overlay, &mut accept, &Action::NextLine); // preview the next world
    let previewed = overlay.as_ref().unwrap().selected_value().unwrap().to_string();
    drive(&mut overlay, &mut accept, &Action::Newline); // keep → POP back to Settings
    let ov = overlay.as_ref().expect("keep from Settings pops back, not to the buffer");
    assert_eq!(ov.kind, OverlayKind::Settings, "re-summoned the Settings menu");
    assert_eq!(ov.return_to, None, "single-level: the re-summoned parent carries no crumb");
    assert_eq!(accept, Some((OverlayKind::Theme, previewed)), "the keep still committed");
    crate::theme::set_active(0);
}

/// The palette-opened FILE picker's Enter is NAVIGATING — it closes the WHOLE
/// stack even with a return_to breadcrumb (you land in the file, not back in the
/// palette). From the palette this matches a value-pick's close-to-buffer (above);
/// they diverge only under a SETTINGS breadcrumb, where a value-pick pops back and
/// a navigator still closes-all — per `OverlayKind::accept_disposition`.
#[test]
fn goto_from_palette_closes_all_on_open_not_pop() {
    let mut overlay = Some(OverlayState::new(
        OverlayKind::Goto,
        vec!["README.md".to_string()],
        vec![],
        vec![],
    ));
    overlay.as_mut().unwrap().return_to = Some(OverlayKind::Command);
    let mut accept = None;
    drive(&mut overlay, &mut accept, &Action::Newline); // open the file
    assert!(overlay.is_none(), "a navigating accept closes the whole stack to the buffer");
    assert_eq!(accept, Some((OverlayKind::Goto, "README.md".to_string())));
}

/// [`stamp_return_to`] fills ONLY an empty breadcrumb (never overwriting a Settings
/// sub-picker's own `return_to = Settings`), stamps only when an overlay is open,
/// and is a no-op for a `None` parent (a terminal command opened nothing).
#[test]
fn stamp_return_to_fills_only_an_empty_breadcrumb() {
    // Fresh overlay (no breadcrumb) + Command parent → stamped Command.
    let mut ov = Some(OverlayState::new(OverlayKind::Theme, vec!["Tawny".into()], vec![], vec![]));
    stamp_return_to(&mut ov, Some(OverlayKind::Command));
    assert_eq!(ov.as_ref().unwrap().return_to, Some(OverlayKind::Command));
    // A pre-set breadcrumb (Settings sub-picker) is NEVER overwritten.
    ov.as_mut().unwrap().return_to = Some(OverlayKind::Settings);
    stamp_return_to(&mut ov, Some(OverlayKind::Command));
    assert_eq!(ov.as_ref().unwrap().return_to, Some(OverlayKind::Settings), "existing breadcrumb kept");
    // A None parent (terminal command) is a no-op even on an empty breadcrumb.
    let mut ov2 = Some(OverlayState::new(OverlayKind::Theme, vec!["Tawny".into()], vec![], vec![]));
    stamp_return_to(&mut ov2, None);
    assert_eq!(ov2.as_ref().unwrap().return_to, None);
    // No overlay open → no-op, no panic.
    let mut none: Option<OverlayState> = None;
    stamp_return_to(&mut none, Some(OverlayKind::Command));
    assert!(none.is_none());
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
