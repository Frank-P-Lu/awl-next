//! Unit + hermetic-App tests for `crate::app`, lifted verbatim out of the
//! former inline `#[cfg(test)] mod tests` (the mechanical decomposition round —
//! zero behaviour change). `super::*` still resolves to `crate::app`, so every
//! reference is unchanged; test names are identical for readable CI diffs.

use super::*;

#[test]
fn gpu_fault_outcomes_skip_once_then_rebuild_without_conflating_validation() {
    assert_eq!(
        gpu_fault_action(GpuLifecycle::Active { oom_skips: 0 }, gpu::GpuFaultKind::OutOfMemory),
        GpuFaultAction::RetryOneFrame,
    );
    assert_eq!(
        gpu_fault_action(GpuLifecycle::Active { oom_skips: 1 }, gpu::GpuFaultKind::OutOfMemory),
        GpuFaultAction::Rebuild,
    );
    for kind in [gpu::GpuFaultKind::DeviceLost, gpu::GpuFaultKind::Internal, gpu::GpuFaultKind::SurfaceRecoveryFailed] {
        assert_eq!(gpu_fault_action(GpuLifecycle::Active { oom_skips: 0 }, kind), GpuFaultAction::Rebuild);
    }
    assert_eq!(
        gpu_fault_action(GpuLifecycle::Active { oom_skips: 0 }, gpu::GpuFaultKind::Validation),
        GpuFaultAction::NoticeOnly,
    );
}

#[test]
fn every_degraded_frame_names_its_retry_or_hold_outcome() {
    assert_eq!(gpu_skip_action(gpu::GpuFrameSkip::Occluded, 0), GpuSkipAction::WaitForWake);
    assert_eq!(gpu_skip_action(gpu::GpuFrameSkip::Timeout, 0), GpuSkipAction::RetryAfter(Duration::from_millis(16)));
    assert_eq!(gpu_skip_action(gpu::GpuFrameSkip::Timeout, 1), GpuSkipAction::RetryAfter(Duration::from_millis(32)));
    assert_eq!(gpu_skip_action(gpu::GpuFrameSkip::Timeout, 20), GpuSkipAction::RetryAfter(Duration::from_millis(512)));
    assert_eq!(gpu_skip_action(gpu::GpuFrameSkip::SurfaceReconfigured, 0), GpuSkipAction::RetryAfter(GPU_SURFACE_RETRY));
    assert_eq!(
        gpu_skip_action(gpu::GpuFrameSkip::SurfaceRecreated, 0),
        GpuSkipAction::RetryWithNoticeAfter(GPU_SURFACE_RETRY, "graphics surface recovered"),
    );
    assert_eq!(
        gpu_skip_action(gpu::GpuFrameSkip::PrepareFailed, 0),
        GpuSkipAction::HoldWithNotice("graphics skipped one frame — editing is safe"),
    );
}

#[test]
fn un_occlusion_wakes_a_repaint_and_occlusion_does_not() {
    // `Occluded → WaitForWake` arms no retry timer, so the un-occlude edge
    // (`false`) is the one that must schedule a redraw; becoming occluded
    // parks the loop and needs no frame.
    assert!(occluded_change_wants_redraw(false));
    assert!(!occluded_change_wants_redraw(true));
}

#[test]
fn skipped_surface_frame_never_drives_the_animation_poll_loop() {
    assert!(!keep_gpu_loop_hot(true, false));
    assert!(!keep_gpu_loop_hot(false, false));
    assert!(!keep_gpu_loop_hot(false, true));
    assert!(keep_gpu_loop_hot(true, true));
}

#[test]
fn wheel_zoom_only_on_super() {
    // Cmd/Super => zoom.
    assert!(scroll_zoom_intent(ModifiersState::SUPER));
    // Ctrl must NOT zoom (the mac bug fix): falls through to free scroll.
    assert!(!scroll_zoom_intent(ModifiersState::CONTROL));
    // No modifiers => no zoom.
    assert!(!scroll_zoom_intent(ModifiersState::empty()));
    // Cmd+Shift still zooms.
    assert!(scroll_zoom_intent(
        ModifiersState::SUPER | ModifiersState::SHIFT
    ));
}

#[test]
fn zoom_debounce_fires_only_after_the_quiet_window() {
    // The STICKY-ZOOM debounce decision: while inside the window the write is
    // deferred (so a rapid Cmd-=/Cmd-- run that re-stamps `dirty` keeps sliding the
    // deadline), and it fires once the window has fully elapsed. Drives the SAME
    // `debounce_due` the `about_to_wait` zoom branch uses.
    let win = ZOOM_PERSIST_DEBOUNCE;
    let dirty = Instant::now();
    // Just after a zoom step: not yet due (still within the quiet window).
    assert!(!debounce_due(dirty, win, dirty));
    assert!(!debounce_due(dirty, win, dirty + win / 2));
    // A fresh step RE-STAMPS dirty later, so an earlier 'now' is still not due —
    // the debounce slides forward instead of firing mid-run.
    let restamped = dirty + win; // a later zoom step moved the stamp
    assert!(!debounce_due(restamped, win, dirty + win)); // now == new dirty: not due
    // Once a FULL quiet window has passed since the last step, it fires.
    assert!(debounce_due(dirty, win, dirty + win));
    assert!(debounce_due(dirty, win, dirty + win + Duration::from_millis(1)));
}

#[test]
fn present_transaction_sync_composes_over_every_source() {
    // The ONE-owner composition `App::sync_present_txn` applies: armed while
    // ANY source needs it (resize stream, move stream, or a theme-preview
    // lava-boundary crossing), off only when ALL three are idle.
    assert!(!present_sync_armed(false, false, false), "idle: async presents");
    assert!(present_sync_armed(true, false, false), "live resize arms it");
    assert!(present_sync_armed(false, true, false), "live move arms it");
    assert!(present_sync_armed(false, false, true), "a preview crossing arms it");
    assert!(present_sync_armed(true, true, false), "corner drag: both streams live");
    assert!(present_sync_armed(true, false, true), "resize + crossing overlap");
    assert!(present_sync_armed(false, true, true), "move + crossing overlap");
    assert!(present_sync_armed(true, true, true), "all three at once");
}

/// THE MOVE-FLASH REGRESSION PIN (user report 2026-07-15, "kinda back"):
/// a `Moved` burst must hold the lamp — phase AND field — for the whole
/// stream, keep presents transaction-synced (the structural gap 318e1fe
/// left: only the ambient tick was gated; the settle redraw and every
/// sibling-debounce present still raced the window-server's move
/// transaction async), and settle exactly once. Drives the REAL `App`
/// state machine through the real `on_moved` / `finish_move_settle`
/// bodies (`about_to_wait`'s debounce arm is just `debounce_due`, pinned
/// separately above).
#[test]
fn moved_stream_holds_the_lamp_and_syncs_presents_until_settle() {
    let _g = crate::testlock::serial();
    let prev = crate::theme::active_index();
    crate::theme::set_active_by_name("Mangrove").unwrap();
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    // An ambient tick was armed before the drag started.
    app.lava_tick_at = Some(Instant::now());
    assert!(!app.present_sync_on, "idle: presents run async");

    // The burst: every event re-stamps the hold and clears the tick arm;
    // the first arms the present-transaction sync for the whole stream.
    for _ in 0..5 {
        app.on_moved(winit::dpi::PhysicalPosition::new(40, 40));
        assert!(app.move_settle_at.is_some(), "the stream holds the stamp");
        assert!(
            app.lava_tick_at.is_none(),
            "no ambient tick may be armed mid-stream"
        );
        assert!(
            app.present_sync_on,
            "every present around the move joins the window-server transaction"
        );
        // Phase (and with it the field — `advance_lava`'s ONLY caller is
        // the tick-due arm) is held: the pause composition closes the gate.
        assert!(
            !crate::lava::lava_should_tick(
                true,
                true,
                false,
                true,
                crate::lava::lava_paused(false, app.move_settle_at.is_some(), false),
            ),
            "the tick gate is closed while the stream is live: phase held"
        );
    }

    // The settle: flags cleared, sync disarmed. Clearing `move_settle_at`
    // is what makes the `about_to_wait` arm (gated on the stamp) unable to
    // fire again — exactly ONE settle redraw per stream.
    app.finish_move_settle();
    assert!(app.move_settle_at.is_none(), "settle clears the hold once");
    assert!(
        app.lava_tick_at.is_none(),
        "the tick re-arms fresh after settle (no catch-up dt)"
    );
    assert!(
        !app.present_sync_on,
        "presents return to async once genuinely settled"
    );
    crate::theme::set_active(prev);
}

/// A corner drag streams BOTH `Resized` and `Moved`: the settle of ONE
/// stream must not strip the other's present-transaction protection (the
/// disarm belongs to the composition's one owner, not to either stream).
#[test]
fn one_streams_settle_never_strips_the_other_streams_present_sync() {
    let _g = crate::testlock::serial();
    let prev = crate::theme::active_index();
    crate::theme::set_active_by_name("Mangrove").unwrap();
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());

    app.arm_live_resize_sync();
    assert!(app.present_sync_on, "resize stream arms the sync");
    app.on_moved(winit::dpi::PhysicalPosition::new(12, 12));
    assert!(app.present_sync_on, "still armed with both streams live");

    // Resize settles first (its window is the shorter one): the move
    // stream is still live, so presents STAY transaction-synced.
    app.finish_resize_settle();
    assert!(app.resize_settle_at.is_none());
    assert!(
        app.present_sync_on,
        "the move stream still owns a claim on the sync"
    );
    app.finish_move_settle();
    assert!(!app.present_sync_on, "both settled: async presents again");
    crate::theme::set_active(prev);
}

/// STRUCTURAL: a non-lava world takes the whole move machinery as a total
/// no-op — no hold stamped, so the settle arm can never fire and a window
/// move schedules ZERO redraws (byte-identical to before the machinery
/// existed).
#[test]
fn a_non_lava_world_takes_a_moved_stream_as_a_total_no_op() {
    let _g = crate::testlock::serial();
    let prev = crate::theme::active_index();
    crate::theme::set_active_by_name("Tawny").unwrap();
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    for _ in 0..3 {
        app.on_moved(winit::dpi::PhysicalPosition::new(40, 40));
    }
    assert!(
        app.move_settle_at.is_none(),
        "no hold: the settle arm can never fire, zero redraws scheduled"
    );
    assert!(!app.present_sync_on, "no stream, no transaction sync");
    crate::theme::set_active(prev);
}

/// THE VANISHING-PAGE FIX — UNCONDITIONAL BRACKET + EVENT-ORDERED TEARDOWN
/// (user report 2026-07-17/18, "arrowing into Magpie makes the writing surface
/// vanish", which survived three `preview_crossing` widenings). Two laws on
/// the REAL `App` state machine:
///  (1) EVERY preview step arms the present-transaction bracket — INCLUDING a
///      step the retired classifier would have called `Steady`: a
///      static→static hop (`Galah→Magpie`, the real Mangrove-gesture LANDING
///      frame the old bracket left exposed) and a lava→lava hop.
///  (2) TEARDOWN IS EVENT-ORDERED: the settle (`finish_crossing_settle`) does
///      NOT disarm — it hands off to `crossing_teardown_pending`, HOLDING the
///      bracket until the post-present hook (`finish_crossing_teardown`)
///      observes the reshaped frame presented in-bracket. So the heavy reshape
///      frame can never coalesce into an unbracketed present (the old race).
#[test]
fn every_preview_step_brackets_and_teardown_waits_for_the_reshape_present() {
    let _g = crate::testlock::serial();
    let prev = crate::theme::active_index();
    let w = |name: &str| *crate::theme::THEMES.iter().find(|t| t.name == name).unwrap();
    let (mangrove, galah) = (w("Mangrove"), w("Galah"));

    crate::theme::set_active_by_name("Mangrove").unwrap();
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    assert!(!app.present_sync_on, "idle: presents run async");

    // (1) The STEADY steps the retired classifier left unbracketed now arm:
    // `Galah→Magpie` (static→static — the real reported LANDING) and
    // `Mangrove→Firetail` (lava→lava). Each starts from a disarmed state.
    for (from, to, label) in
        [(galah, "Magpie", "static->static LANDING"), (mangrove, "Firetail", "lava->lava")]
    {
        app.crossing_settle_at = None;
        app.crossing_teardown_pending = false;
        app.sync_present_txn();
        assert!(!app.present_sync_on, "{label}: starts disarmed");
        crate::theme::set_active_by_name(to).unwrap();
        app.retint_theme_preview(from);
        assert!(app.crossing_settle_at.is_some(), "{label}: the preview stamps the settle");
        assert!(app.present_sync_on, "{label}: the bracket arms unconditionally");
    }

    // (2) EVENT-ORDERED TEARDOWN. Phase 1 (settle) clears the debounce but
    // HOLDS the bracket via the pending teardown — it must NOT disarm here,
    // because the deferred reshape's present has not happened yet.
    app.finish_crossing_settle();
    assert!(app.crossing_settle_at.is_none(), "phase 1 clears the settle debounce");
    assert!(app.crossing_teardown_pending, "phase 1 hands off to the pending teardown");
    assert!(app.present_sync_on, "the bracket is HELD through the reshape present, not torn down");
    // Phase 2 (the post-present hook, after the in-bracket reshape present)
    // is the ONLY thing that disarms.
    app.finish_crossing_teardown();
    assert!(!app.crossing_teardown_pending, "phase 2 clears the pending teardown");
    assert!(!app.present_sync_on, "only after the bracketed reshape present does the bracket disarm");
    crate::theme::set_active(prev);
}

/// A crossing can OVERLAP a live drag: the settle of one source must not
/// strip the other's present-transaction protection (the disarm belongs to
/// the composition's one owner). Mirrors the corner-drag law for the third
/// source.
#[test]
fn a_crossing_settle_never_strips_a_live_resize_streams_present_sync() {
    let _g = crate::testlock::serial();
    let prev = crate::theme::active_index();
    let mangrove = *crate::theme::THEMES
        .iter()
        .find(|t| t.name == "Mangrove")
        .unwrap();

    crate::theme::set_active_by_name("Mangrove").unwrap();
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());

    // A live resize stream, then a preview step (now unconditional).
    app.arm_live_resize_sync();
    assert!(app.present_sync_on, "resize stream arms the sync");
    crate::theme::set_active_by_name("Magpie").unwrap();
    app.retint_theme_preview(mangrove);
    assert!(app.present_sync_on, "still armed with both sources live");

    // The preview settles + fully tears down (both phases): the resize stream
    // still owns a claim through EACH phase — the disarm belongs to the one
    // owner, never to a single source's settle.
    app.finish_crossing_settle();
    assert!(app.crossing_settle_at.is_none());
    assert!(app.present_sync_on, "resize still owns a claim after phase 1");
    app.finish_crossing_teardown();
    assert!(!app.crossing_teardown_pending);
    assert!(app.present_sync_on, "resize still owns a claim after phase 2");
    app.finish_resize_settle();
    assert!(!app.present_sync_on, "both settled: async presents again");
    crate::theme::set_active(prev);
}

#[test]
fn only_live_toasts_expire_sticky_and_clockless_notices_do_not() {
    let now = Instant::now();
    let deadline = now + TOAST_LIFETIME;
    assert!(!notice_expired(NoticeKind::Toast, Some(deadline), now));
    assert!(notice_expired(NoticeKind::Toast, Some(deadline), deadline));
    assert!(!notice_expired(NoticeKind::Sticky, Some(deadline), deadline));
    assert!(!notice_expired(NoticeKind::Toast, None, deadline));
}

#[test]
fn idle_deadlines_compose_without_delaying_poll_or_an_earlier_timer() {
    let now = Instant::now();
    let earlier = now + Duration::from_millis(40);
    let later = now + Duration::from_millis(100);

    assert_eq!(
        control_flow_with_deadline(ControlFlow::Poll, later),
        ControlFlow::Poll,
        "a hot redraw loop always wins"
    );
    assert_eq!(
        control_flow_with_deadline(ControlFlow::Wait, later),
        ControlFlow::WaitUntil(later),
        "an idle unscheduled loop accepts the proposed deadline"
    );
    assert_eq!(
        control_flow_with_deadline(ControlFlow::WaitUntil(earlier), later),
        ControlFlow::WaitUntil(earlier),
        "a later proposal cannot delay the current earlier deadline"
    );
    assert_eq!(
        control_flow_with_deadline(ControlFlow::WaitUntil(later), earlier),
        ControlFlow::WaitUntil(earlier),
        "an earlier proposal advances the current later deadline"
    );
}

#[test]
fn held_hud_dismisses_when_summon_modifier_lifts() {
    // The stats HUD is a momentary HOLD: summoned with Option-Cmd-I, it must vanish the
    // instant the chord lifts. macOS does not deliver the 'i' key-UP while Cmd is
    // down, so dismissal rides the modifier release instead — this pure predicate is
    // the state machine: pressed-with-Super, then Super gone => clear.
    let summon = ModifiersState::SUPER;
    // Cmd still held (no change, or an OS auto-repeat) => HUD stays.
    assert!(!hud_mods_broken(summon, ModifiersState::SUPER));
    // Cmd RELEASED (mods now empty) => the hold is broken, HUD clears.
    assert!(hud_mods_broken(summon, ModifiersState::empty()));
    // Adding an EXTRA modifier (Cmd+Shift) is still a superset => HUD stays.
    assert!(!hud_mods_broken(
        summon,
        ModifiersState::SUPER | ModifiersState::SHIFT
    ));
    // Swapping Cmd for a different modifier still breaks the summon set => clear.
    assert!(hud_mods_broken(summon, ModifiersState::CONTROL));
    // A no-modifier summon (a rebind to a bare key) is never broken by mods alone;
    // that hold is dismissed by the key-UP path (`on_key_release`) instead.
    assert!(!hud_mods_broken(ModifiersState::empty(), ModifiersState::empty()));
    assert!(!hud_mods_broken(ModifiersState::empty(), ModifiersState::SUPER));
}

#[test]
fn buffer_endpoints_shift_intent_keys_on_the_chord_not_the_action() {
    use winit::keyboard::NamedKey;
    // The chord-aware rule: BufferStart/BufferEnd carry INCIDENTAL Shift only
    // for `M-<` / `M->`, whose Shift is needed just to TYPE the `<` / `>`
    // glyph (a `Key::Character`) — those stay pure motion and must NOT extend.
    assert!(!motion_honors_shift_select(&Action::BufferStart, &Key::Character("<".into())));
    assert!(!motion_honors_shift_select(&Action::BufferEnd, &Key::Character(">".into())));
    // But the SAME actions reached through a NAMED navigation key carry a
    // genuine GUI select-intent Shift and MUST extend: Shift+Cmd-Up/Down
    // (macOS) and Shift+Ctrl-Home/End (Linux) select to document start/end,
    // exactly like every platform text field (the live bug this round fixed).
    assert!(motion_honors_shift_select(&Action::BufferStart, &Key::Named(NamedKey::ArrowUp)));
    assert!(motion_honors_shift_select(&Action::BufferEnd, &Key::Named(NamedKey::ArrowDown)));
    assert!(motion_honors_shift_select(&Action::BufferStart, &Key::Named(NamedKey::Home)));
    assert!(motion_honors_shift_select(&Action::BufferEnd, &Key::Named(NamedKey::End)));
    // Every other motion keeps Shift's normal select-extend meaning regardless
    // of key shape (the user deliberately held Shift, e.g. Shift+Arrow / M-Shift-f).
    assert!(motion_honors_shift_select(&Action::ForwardChar, &Key::Named(NamedKey::ArrowRight)));
    assert!(motion_honors_shift_select(&Action::ForwardChar, &Key::Character("f".into())));
    assert!(motion_honors_shift_select(&Action::ForwardWord, &Key::Character("f".into())));
    assert!(motion_honors_shift_select(&Action::NextLine, &Key::Named(NamedKey::ArrowDown)));
    assert!(motion_honors_shift_select(&Action::LineEnd, &Key::Named(NamedKey::End)));
    // Non-motions are unaffected (Shift is ignored by the motion-select logic
    // for them anyway), so they report the default true.
    assert!(motion_honors_shift_select(&Action::InsertChar('a'), &Key::Character("A".into())));
}

#[test]
fn shift_cmd_up_down_extend_to_document_bounds_through_the_live_apply_seam() {
    // LIVE-PATH pin (the resolve → derive-shift → apply_core chain the window
    // runs, minus winit event plumbing): resolve the real chord through the
    // persistent keymap exactly as `dispatch_pressed_key` does, derive the
    // shift flag through the ONE owner keyed on the pressed KEY, then apply.
    // Shift+Cmd-Up must select from mid-document up to (0,0); Shift+Cmd-Down
    // must select from there down to the document end — while `M-<` / `M->`,
    // whose Shift is incidental to typing the glyph, must NOT extend.
    use winit::event::Modifiers;
    use winit::keyboard::{ModifiersState, NamedKey};
    let sup_shift = Modifiers::from(ModifiersState::SUPER | ModifiersState::SHIFT);
    let meta_shift = Modifiers::from(ModifiersState::ALT | ModifiersState::SHIFT);

    // Helper: resolve a (key, mods) chord and apply it with the live shift
    // derivation, returning the post-apply selection.
    fn drive_live(
        key: Key,
        mods: winit::event::Modifiers,
        text: &str,
        start: usize,
    ) -> Option<((usize, usize), (usize, usize))> {
        // Pin the MAC convention: this drives the macOS Cmd-Up/Down chord
        // (Super+Arrow), so the result must not depend on the ambient
        // `AWL_CONVENTION_FORCE` the suite may run under (Linux resolves
        // Super+Arrow differently).
        let mut km = crate::keymap::KeymapState::new_with_convention(
            crate::convention::Convention::Mac,
        );
        let action = km.resolve(&key, &mods);
        let shift = mods.state().contains(ModifiersState::SHIFT)
            && motion_honors_shift_select(&action, &key);
        let mut buffer = crate::buffer::Buffer::from_str(text);
        buffer.set_cursor(start);
        let mut shift_selecting = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut overlay = None;
        let mut make_overlay =
            |_k: crate::overlay::OverlayKind| -> Option<crate::overlay::OverlayState> { None };
        let mut browse_to = |_k: crate::overlay::OverlayKind,
                             _r: Option<String>|
         -> Option<crate::overlay::OverlayState> { None };
        let mut ctx = crate::actions::ActionCtx {
            buffer: &mut buffer,
            shift_selecting: &mut shift_selecting,
            zoom: &mut zoom,
            search: &mut search,
            scroll_page_lines: 1,
            overlay: &mut overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
            oracle: None,
        };
        crate::actions::apply_core(&mut ctx, &action, shift);
        buffer.selection_line_col()
    }

    const TXT: &str = "alpha beta\ngamma delta\nepsilon zeta\n";
    // From mid-line (1,3), Shift+Cmd-Up extends up to (0,0).
    let up = drive_live(Key::Named(NamedKey::ArrowUp), sup_shift, TXT, 14);
    assert_eq!(up, Some(((0, 0), (1, 3))), "Shift+Cmd-Up selects to document start");
    // From the same spot, Shift+Cmd-Down extends down to the doc end — the
    // empty final line after the trailing newline, (3,0).
    let down = drive_live(Key::Named(NamedKey::ArrowDown), sup_shift, TXT, 14);
    assert_eq!(down, Some(((1, 3), (3, 0))), "Shift+Cmd-Down selects to document end");
    // `M-<` (Character `<`, Shift incidental) moves but never extends.
    let emacs = drive_live(Key::Character("<".into()), meta_shift, TXT, 14);
    assert_eq!(emacs, None, "M-< incidental Shift stays pure motion (no selection)");
}

#[test]
fn double_click_bumps_the_shared_click_counter_that_also_backs_the_edge_reset() {
    // `bump_click_count` is the ONE shared multi-click detector: a plain
    // document press (`on_press`: a double-click selects a word, a triple
    // selects a line) and a press on the draggable PAGE EDGE
    // (`begin_page_resize_if_hovering`: a double-click there RESETS the width
    // instead of beginning a drag) both branch on its returned count — so
    // proving it reaches 2 on a fast same-spot double click proves the edge
    // gesture recognizes a double-click identically, without needing a live
    // GPU hover test (that half — routing through `App::apply` behind the
    // GPU-gated hover check — stays LIVE-ONLY, like the rest of the drag
    // gesture; the hover math itself is unit-tested in `render::geometry`).
    // No real file content is needed here, so build hermetically (closes
    // the session-restore + scratch-stash doors — see `new_hermetic`'s doc).
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    app.cursor_px = (0.0, 0.0);
    assert_eq!(app.bump_click_count(), 1, "a first press starts a fresh count");
    assert_eq!(app.bump_click_count(), 2, "an immediate same-spot press doubles it");
    assert_eq!(app.bump_click_count(), 3, "a third same-spot press triples it");
    assert_eq!(app.bump_click_count(), 1, "a fourth wraps back to a fresh single click");
    // A press at a DIFFERENT spot never continues the run, however fast.
    app.bump_click_count();
    app.cursor_px = (500.0, 500.0);
    assert_eq!(app.bump_click_count(), 1, "a different spot starts over, not a double-click");
}

#[test]
fn open_serves_the_new_files_text_despite_equal_buffer_versions() {
    // THE LIVE "open a file and it does not appear" BUG: every swapped-in buffer
    // restarts its edit version at 0, and `sync_view`'s rope-clone short-circuit
    // (`view_text`) is keyed by version ALONE — so opening a file from any
    // UN-EDITED buffer (also version 0) hit the stale cache and pushed the OLD
    // document's text to the renderer. The screen repainted, but with the old
    // content, until the first edit bumped the version. The headless capture
    // rebuilds its text per frame and never saw it. This drives the REAL open
    // arm (`load_path`, shared by Go-to-file / Browse / picker click / C-x b)
    // against the REAL cache seam, GPU-less.
    // Reads the REAL disk through the fs seam, so hold the fs TEST_LOCK: a
    // parallel test with an InMemoryFs installed would swallow these files.
    // Can't build hermetically (`App::new_hermetic` injects an empty
    // InMemoryFs, which would make `Buffer::from_file` find neither real
    // fixture below) — disable session restore explicitly instead, so
    // `apply_session_restore` never reads the developer's real
    // `~/.local/share/awl/session.toml` and parks his real open files into
    // this test's registry (the exact leak class `d93109e` fixed).
    let _fs = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl-open-swap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let old = dir.join("old.txt");
    let new = dir.join("new.txt");
    std::fs::write(&old, "the OLD document\n").unwrap();
    std::fs::write(&new, "the NEW document\n").unwrap();
    let cfg = Config { session_restore: Some(false), ..Config::empty() };
    let mut app = App::new(Some(old), dir.clone(), None, None, cfg);
    // The first sync caches (version 0, old text) — the short-circuit at work.
    assert_eq!(app.view_text(), "the OLD document\n");
    assert_eq!(app.buffer.version(), 0, "an un-edited buffer sits at version 0");
    // Open the second file: a FRESH buffer, version 0 again — the cache key
    // collides. The view text MUST be the new file's, not the cached old one.
    app.load_path(new);
    assert_eq!(
        app.view_text(),
        "the NEW document\n",
        "the opened file's text must reach the view despite the version collision"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_note_drops_the_stale_view_text_cache() {
    // C-x n swaps in a fresh EMPTY note buffer (version 0 again): the previous
    // un-edited buffer's cached text (also version 0) must not survive the swap,
    // or the new note would render as the old document until the first keystroke
    // — the same version-collision as the open arm, on the note door.
    // Real-disk reads through the seam → hold the fs TEST_LOCK (see above),
    // and disable session restore for the same reason the sibling test
    // above does (can't build hermetically — this needs real file bytes).
    let _fs = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl-note-swap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("doc.txt");
    std::fs::write(&file, "prior document\n").unwrap();
    let notes = dir.join("notes");
    let cfg = Config { session_restore: Some(false), ..Config::empty() };
    let mut app = App::new(Some(file), dir.clone(), None, Some(notes), cfg);
    assert_eq!(app.view_text(), "prior document\n");
    app.new_note();
    assert_eq!(app.view_text(), "", "the fresh note starts blank on screen");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── The AUTOSAVE ENGINE (App-level, over the InMemoryFs seam) ───────────
//
// Each test installs a fake FS via FsGuard so App::new / the flush paths
// never touch the real disk (or the developer's real scratch stash).

/// An App over the installed fake FS, opened on `file` with project `root`.
fn app_on(file: Option<PathBuf>, root: &str, config: Config) -> App {
    App::new(file, PathBuf::from(root), None, None, config)
}

impl App {
    /// TEST DRIVER: route one action through the SHARED core against this
    /// App's own buffer/overlay/search — the same seam a live keypress
    /// reaches after `App::apply`'s window-level intercepts (no event loop,
    /// no GPU). Minimal builder closures: an overlay-opening action would
    /// no-op, which the preview-path read-only tests never need.
    fn apply_core_for_test(&mut self, action: &Action) -> crate::actions::Effect {
        let mut shift = self.shift_selecting;
        let mut zoom = self.zoom;
        let mut make = |_k: crate::overlay::OverlayKind| None;
        let mut browse = |_k: crate::overlay::OverlayKind, _p: Option<String>| None;
        let mut ctx = crate::actions::ActionCtx {
            buffer: &mut self.buffer,
            shift_selecting: &mut shift,
            zoom: &mut zoom,
            search: &mut self.search,
            scroll_page_lines: 10,
            overlay: &mut self.overlay,
            make_overlay: &mut make,
            browse_to: &mut browse,
            oracle: None,
        };
        crate::actions::apply_core(&mut ctx, action, false)
    }
}

// ── GOTO FILE-INDEX FRESHNESS (queue: "file picker freshness") ──────────
//
// The go-to overlay (`C-x f`) corpus comes from `App.file_index`, a CACHED
// field only ever rebuilt on specific triggers (root switch, a note's first
// save, a rename, a move) — never simply because the picker summoned. A file
// dropped into the root by another process, or a shell command, while awl
// sits open would never appear until one of those triggers happened to also
// fire. The fix: RE-SCAN ON EVERY SUMMON via `App::rescan_file_index` (called
// from `App::apply`'s `Action::OpenGoto` arm, over the `FileSystem` trait) —
// no watcher, no TTL, just re-walk right as the overlay opens.

#[test]
fn rescan_file_index_picks_up_a_file_created_after_the_last_scan() {
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new().with_file("/proj/a.txt", "a\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(None, "/proj", Config::empty());
    // The initial scan (at App::new) sees only the file that existed then.
    assert_eq!(app.file_index, vec!["a.txt".to_string()]);
    // SUMMON #1 (simulated: `rescan_file_index` is exactly what `C-x f`
    // triggers): still just the one file — nothing has changed yet.
    app.rescan_file_index();
    assert_eq!(app.file_index, vec!["a.txt".to_string()]);
    // A file appears on disk WITHOUT going through awl at all (another
    // process, a git checkout, a plain `touch`) — the picker is CLOSED at
    // this point, so nothing in awl has any reason to know yet.
    mem.write(std::path::Path::new("/proj/b.txt"), b"b\n").unwrap();
    assert_eq!(
        app.file_index,
        vec!["a.txt".to_string()],
        "the cached index does not spontaneously update"
    );
    // SUMMON #2 (`C-x f` again): the fresh scan MUST find it.
    app.rescan_file_index();
    assert_eq!(
        app.file_index,
        vec!["a.txt".to_string(), "b.txt".to_string()],
        "re-summoning must re-scan and pick up the new file"
    );
    // Build the ACTUAL overlay the way `App::apply`'s Goto arm does, to prove
    // the fresh index really reaches the summoned picker's corpus (the same
    // `overlay::build` the live App and headless replay both call).
    let effective_keep = app.config.effective_linux_keep();
    let build_ctx = crate::overlay::BuildCtx {
        goto_corpus: app.file_index.clone(),
        goto_open: Vec::new(),
        goto_recent: Vec::new(),
        goto_times: Vec::new(),
        config_keys: &app.config.keys,
        config_linux_keep: &effective_keep,
        goto_headings: Vec::new(),
        spell_target: None,
        history_entries: Vec::new(),
        history_now: None,
        history_session_start: None,
        settings_values: Default::default(),
        assets: Vec::new(),
    };
    let ov = crate::overlay::build(crate::overlay::OverlayKind::Goto, &build_ctx)
        .expect("Goto always summons");
    assert!(ov.corpus.contains(&"b.txt".to_string()), "the new file is listed");
}

// ── THE KEYMAP FLAVOR ROUND — the Settings "Keymap" toggle round-trip ────

/// Enter on the "Keymap" settings row (`App::toggle_keymap_flavor`, the
/// special-cased door `App::setting_toggle` routes "keymap" through):
/// flips native <-> emacs, PERSISTS the flip format-preservingly (the same
/// `persist_pref` owner every other sticky pref rides), and re-applies the
/// keymap LIVE from the updated in-memory config — proven here by feeding
/// the SAME `app.config.effective_linux_keep()` a fresh `KeymapState`
/// would consume (the exact composition `toggle_keymap_flavor` rebuilds
/// `self.keymap` from) into a `Convention::Linux`-pinned keymap and
/// confirming it now carries the full emacs preset.
#[test]
fn settings_keymap_toggle_flips_persists_and_live_reapplies() {
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let cfg = Config { path: PathBuf::from("/cfg/config.toml"), ..Config::empty() };
    let mut app = app_on(None, "/proj", cfg);
    assert_eq!(app.config.keymap_flavor(), crate::keymap::KeymapFlavor::Native, "starts native");

    // Enter #1: native -> emacs.
    app.toggle_keymap_flavor();
    assert_eq!(app.config.keymap_flavor(), crate::keymap::KeymapFlavor::Emacs, "in-memory mirror flips");
    let written = mem.read_to_string(std::path::Path::new("/cfg/config.toml")).unwrap();
    assert!(written.contains("keymap = \"emacs\""), "persisted format-preservingly: {written:?}");

    // LIVE RE-APPLY: the same composed keep-list the toggle rebuilt
    // `self.keymap` from now carries the WHOLE emacs preset — build a
    // fresh convention-pinned keymap from exactly that composition (the
    // private `KeymapState.linux_keep` field can't be introspected from
    // here, so this proves the INPUT the live rebuild consumed, which
    // `keymap::tests::keymap_flavor_emacs_preset_reverts_every_displaced_chord_to_emacs_meaning`
    // already proves is sufficient to flip dispatch).
    let effective = app.config.effective_linux_keep();
    let preset = crate::keymap::linux_emacs_preset_keep();
    // The insert-link-yields-to-kill-line round's built-in floor
    // (`keymap::linux_builtin_keep()`) rides ALONG with the preset — it is
    // NOT flavor-gated, so it's present under emacs too, just not part of
    // `preset` itself (see `linux_builtin_keep()`'s own doc).
    assert_eq!(
        effective.len(),
        preset.len() + crate::keymap::linux_builtin_keep().len(),
        "the live rebuild's keep-list is the whole preset plus the built-in floor"
    );
    for chord in &preset {
        assert!(effective.contains(chord), "{chord:?} missing from the live rebuild's keep-list");
    }
    for chord in crate::keymap::linux_builtin_keep() {
        assert!(effective.iter().any(|c| c == chord), "{chord:?} missing from the live rebuild's keep-list");
    }

    // Enter #2: emacs -> native (round-trips cleanly, doesn't accumulate).
    app.toggle_keymap_flavor();
    assert_eq!(app.config.keymap_flavor(), crate::keymap::KeymapFlavor::Native, "flips back");
    let written2 = mem.read_to_string(std::path::Path::new("/cfg/config.toml")).unwrap();
    assert!(written2.contains("keymap = \"native\""), "the second toggle persists too: {written2:?}");
    // Native flavor: no preset widening, but the built-in floor is still
    // there (it's unconditional, not flavor-gated) — never truly empty.
    assert_eq!(
        app.config.effective_linux_keep().len(),
        crate::keymap::linux_builtin_keep().len(),
        "native flavor: no preset widening, just the built-in floor"
    );
}

/// LAW TEST (the "settings toggle rows dispatch live" round): EVERY row
/// the corpus marks `SettingKind::Toggle` — enumerated straight off
/// `settings::visible_rows()`, never hand-copied — round-trips through
/// the REAL live door, `App::setting_toggle(key)` (exactly what
/// `Effect::SettingToggle` resolves to at the `app/apply.rs` seam, see
/// `App::apply`'s `Effect::SettingToggle { key } => self.setting_toggle(&key)`
/// arm): the value readout VISIBLY CHANGES after one toggle, and
/// round-trips back to its exact starting value after a second — so a
/// toggle that silently no-ops (the Keymap-row bug: wired in
/// `settings::toggle_key` and in `settings_accept`, but never driven
/// through `App::setting_toggle` itself by any prior test — the prior
/// `settings_keymap_toggle_flips_persists_and_live_reapplies` test called
/// `app.toggle_keymap_flavor()` directly, skipping the string-keyed
/// dispatch a live Enter/click actually goes through) fails here instead
/// of shipping quietly. Companion:
/// `actions::tests::overlay_drive::every_settings_toggle_row_signals_its_own_setting_toggle_key`
/// (the pure `apply_core`-level half: Enter on the row signals the RIGHT
/// key in the first place). Each toggle is undone immediately after
/// asserting it, so every process-global this sweep touches (page /
/// typewriter / wysiwyg / inline images / ligatures / spellcheck /
/// writing nits / outline / menu bar / reduce motion) is back to its
/// pre-test value by the time the lock releases — no leak into a sibling
/// test, mirroring the `page::measure()` save/restore convention used
/// elsewhere in this file.
#[test]
fn every_settings_toggle_row_dispatches_live_and_flips_its_value() {
    use crate::fs::InMemoryFs;
    let _g2 = crate::fs::FsGuard::install(Arc::new(InMemoryFs::new()));
    let _g = crate::testlock::serial();

    let cfg = Config { path: PathBuf::from("/cfg/config.toml"), ..Config::empty() };
    let mut app = app_on(None, "/proj", cfg);

    let toggle_rows: Vec<crate::settings::SettingRow> = crate::settings::visible_rows()
        .into_iter()
        .filter(|r| r.kind == crate::settings::SettingKind::Toggle)
        .copied()
        .collect();
    assert_eq!(
        toggle_rows.len(),
        15,
        "the toggle roster changed size — update this sweep deliberately"
    );

    for row in &toggle_rows {
        let key = crate::settings::toggle_key(row.name).expect("a Toggle row always has a key");
        let values0 = crate::settings::SettingsValues::gather(&app.config, &app.root, app.zoom);
        let before = crate::settings::value_for(row, &values0);

        app.setting_toggle(key);
        let values1 = crate::settings::SettingsValues::gather(&app.config, &app.root, app.zoom);
        let after = crate::settings::value_for(row, &values1);
        assert_ne!(
            before, after,
            "row {:?} (key {:?}) did not visibly flip its value readout — the live dispatch is a silent no-op",
            row.name, key
        );

        // Toggle back — restores the global/config AND proves the flip is
        // a clean round-trip, not a one-way ratchet.
        app.setting_toggle(key);
        let values2 = crate::settings::SettingsValues::gather(&app.config, &app.root, app.zoom);
        let restored = crate::settings::value_for(row, &values2);
        assert_eq!(
            restored, before,
            "row {:?} (key {:?}) did not round-trip back to its starting value",
            row.name, key
        );
    }
}

/// The corpus GREW to carry the row: "Keymap" is a real, visible settings
/// row (mirrors `settings::tests::settings_table_names_are_unique`'s own
/// count law, exercised here through the App's own config/root — a
/// belt-and-suspenders confirmation that the live overlay build would
/// actually list it).
#[test]
fn settings_corpus_includes_the_keymap_row() {
    assert!(crate::settings::visible_names().contains(&"Keymap".to_string()));
    assert_eq!(crate::settings::toggle_key("Keymap"), Some("keymap"));
}

#[test]
fn disk_changed_truth_table() {
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let p = std::path::Path::new("/d/f.md");
    // (None, None): the file never existed — our write CREATES it, no clobber.
    assert!(!App::disk_changed(p, None));
    mem.write(p, b"v1").unwrap();
    let t1 = App::disk_mtime_of(p);
    assert!(t1.is_some(), "the fake records mtimes");
    // (Some, Some) equal → unchanged.
    assert!(!App::disk_changed(p, t1));
    // (Some, None): the file APPEARED externally since we looked.
    assert!(App::disk_changed(p, None));
    // (Some, Some) differing → a real external change.
    std::thread::sleep(Duration::from_millis(2)); // ensure a distinct mtime
    mem.write(p, b"v2").unwrap();
    assert!(App::disk_changed(p, t1));
    // (Some, Some) with the SAME mtime but a DIFFERENT size → a same-tick
    // external edit (equal mtime, changed content) must still be caught by the
    // size guard, or we'd silently overwrite it.
    let cur = App::disk_mtime_of(p).expect("v2 exists");
    let same_tick_other_size = Some(crate::fs::Metadata {
        modified: cur.modified,
        len: cur.len.map(|n| n + 1),
    });
    assert!(App::disk_changed(p, same_tick_other_size));
    // (None, Some): the file was DELETED externally (renamed away here — the
    // trait has no remove op, and a rename models the same disappearance).
    let last = App::disk_mtime_of(p);
    mem.rename(p, std::path::Path::new("/d/elsewhere.md")).unwrap();
    assert!(App::disk_changed(p, last));
}

#[test]
fn autosave_flush_writes_doc_and_snapshots_loose_file() {
    use crate::fs::{FileSystem, InMemoryFs};
    let p = PathBuf::from("/notes/draft.md");
    let mem = InMemoryFs::new().with_file(&p, "v1\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
    assert!(
        app.autosave_last_ok.is_none(),
        "the debug panel's autosave clock is untouched before any write"
    );
    app.buffer.set_text("v2\n");
    app.autosave_flush();
    assert_eq!(mem.read_to_string(&p).unwrap(), "v2\n", "the edit hit the disk");
    assert_eq!(
        app.doc_saved_version,
        Some(app.buffer.version()),
        "the flushed version is bookkept"
    );
    assert!(app.notice.is_none(), "a clean write raises no notice");
    assert!(
        app.autosave_last_ok.is_some(),
        "a real engine write stamps the debug panel's autosave clock"
    );
    // The debug panel's pure composer agrees: enabled + not held + a stamped
    // write => Saved (never Off/Held after a clean autosave).
    assert!(matches!(
        crate::debug::autosave_state(app.config.autosave_on(), app.notice.is_some(), Some(0)),
        crate::debug::AutosaveState::Saved(Some(0))
    ));
    // Every save records: the loose file grew a history snapshot.
    assert!(
        !crate::history::list(&p).is_empty(),
        "autosave records a local-history snapshot for a loose file"
    );
    // An unchanged buffer is not re-written (version bookkeeping short-circuits).
    let t = App::disk_mtime_of(&p);
    app.autosave_flush();
    assert_eq!(App::disk_mtime_of(&p), t, "no redundant write for a clean buffer");
}

#[test]
fn autosave_flush_skips_and_notices_when_disk_changed_externally() {
    use crate::fs::{FileSystem, InMemoryFs};
    let p = PathBuf::from("/notes/draft.md");
    let mem = InMemoryFs::new().with_file(&p, "disk v1\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
    // Someone ELSE writes the file behind awl's back.
    std::thread::sleep(Duration::from_millis(2)); // distinct mtime
    mem.write(&p, b"external edit\n").unwrap();
    app.buffer.set_text("mine\n");
    app.autosave_flush();
    // The CLOBBER GUARD held the write: the external edit survives on disk.
    assert_eq!(
        mem.read_to_string(&p).unwrap(),
        "external edit\n",
        "autosave never overwrites external edits"
    );
    assert_eq!(
        app.notice.as_deref(),
        Some(CLOBBER_NOTICE),
        "a calm notice is raised"
    );
    assert!(
        app.autosave_last_ok.is_none(),
        "a HELD write must never stamp the debug panel's autosave clock — no write happened"
    );
    // The debug panel's pure composer agrees: held wins over "nothing written yet".
    assert_eq!(
        crate::debug::autosave_state(app.config.autosave_on(), app.notice.is_some(), None),
        crate::debug::AutosaveState::Held
    );
    // The version is marked handled so the idle timer doesn't spin; the NEXT
    // edit re-arms the engine (and the notice would recur calmly).
    assert_eq!(app.doc_saved_version, Some(app.buffer.version()));
}

#[test]
fn autosave_off_disables_flush() {
    use crate::fs::{FileSystem, InMemoryFs};
    let p = PathBuf::from("/notes/draft.md");
    let mem = InMemoryFs::new().with_file(&p, "v1\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let cfg = Config {
        autosave: Some(false),
        ..Config::empty()
    };
    let mut app = app_on(Some(p.clone()), "/notes", cfg);
    app.buffer.set_text("v2\n");
    app.autosave_flush();
    assert_eq!(
        mem.read_to_string(&p).unwrap(),
        "v1\n",
        "autosave = false leaves the disk untouched"
    );
    assert!(app.notice.is_none());
    assert!(
        app.autosave_last_ok.is_none(),
        "a disabled engine never stamps the debug panel's autosave clock"
    );
    // The debug panel's pure composer agrees: disabled wins over everything.
    assert_eq!(
        crate::debug::autosave_state(app.config.autosave_on(), app.notice.is_some(), None),
        crate::debug::AutosaveState::Off
    );
}

#[test]
fn load_path_flushes_the_leaving_buffer() {
    use crate::fs::{FileSystem, InMemoryFs};
    let a = PathBuf::from("/notes/a.md");
    let b = PathBuf::from("/notes/b.md");
    let mem = InMemoryFs::new().with_file(&a, "A\n").with_file(&b, "B\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(a.clone()), "/notes", Config::empty());
    app.buffer.set_text("A edited\n");
    app.load_path(b.clone());
    assert_eq!(
        mem.read_to_string(&a).unwrap(),
        "A edited\n",
        "switching files flushes the buffer being left"
    );
    assert_eq!(app.buffer.text(), "B\n", "the new file is open");
    assert_eq!(
        app.doc_saved_version,
        Some(app.buffer.version()),
        "the arriving buffer starts saved"
    );
}

// ── i18n WRITE-BACK-ONCE (App::new launch arg + App::load_path switch) ───

#[test]
fn launching_on_an_untagged_japanese_file_tags_it_once() {
    use crate::fs::{FileSystem, InMemoryFs};
    let p = PathBuf::from("/notes/nihongo.md");
    let original = "これは日本語の文章です。\n";
    let mem = InMemoryFs::new().with_file(&p, original);
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let app = app_on(Some(p.clone()), "/notes", Config::empty());
    assert_eq!(
        app.buffer.text(),
        format!("---\nlang: ja\n---\n{original}"),
        "an untagged kana-bearing doc is tagged ja on first open"
    );
    // NEVER a silent disk write: the file on disk is untouched, and the
    // buffer reads as DIRTY (past doc_saved_version) so the ordinary
    // autosave engine picks the tag up on the next idle/blur/switch/quit.
    assert_eq!(mem.read_to_string(&p).unwrap(), original, "disk is untouched");
    assert!(
        app.doc_saved_version.unwrap() < app.buffer.version(),
        "the stamped tag is a PENDING edit, not already-saved"
    );
}

#[test]
fn write_back_never_touches_a_pure_latin_document() {
    use crate::fs::InMemoryFs;
    let p = PathBuf::from("/notes/english.md");
    let original = "Just some ordinary English prose.\n";
    let mem = InMemoryFs::new().with_file(&p, original);
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let app = app_on(Some(p.clone()), "/notes", Config::empty());
    assert_eq!(app.buffer.text(), original, "a pure-Latin doc is never touched");
    assert_eq!(
        app.doc_saved_version,
        Some(app.buffer.version()),
        "no edit landed -> still reads as saved"
    );
}

#[test]
fn write_back_never_fires_on_a_non_markdown_file() {
    use crate::fs::InMemoryFs;
    // A `.rs` file with a Japanese string literal: frontmatter is a
    // markdown/notes convention, and stamping `---`/`lang:` text into a
    // code file would corrupt it, so this must stay untouched.
    let p = PathBuf::from("/proj/main.rs");
    let original = "fn main() {\n    println!(\"こんにちは\");\n}\n";
    let mem = InMemoryFs::new().with_file(&p, original);
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let app = app_on(Some(p.clone()), "/proj", Config::empty());
    assert_eq!(app.buffer.text(), original, "a non-markdown file is never tagged");
}

#[test]
fn write_back_uses_the_configured_cjk_priority_for_ambiguous_han() {
    use crate::fs::InMemoryFs;
    let p = PathBuf::from("/notes/hanzi.md");
    let original = "汉字漢字\n"; // Han only, no kana/hangul/bopomofo -> ambiguous
    let mem = InMemoryFs::new().with_file(&p, original);
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let cfg = Config {
        cjk_priority: Some(vec![crate::frontmatter::Lang::ZhHans, crate::frontmatter::Lang::Ja]),
        ..Config::empty()
    };
    let app = app_on(Some(p.clone()), "/notes", cfg);
    assert_eq!(app.buffer.text(), format!("---\nlang: zh-Hans\n---\n{original}"));
}

#[test]
fn write_back_is_undoable_with_cmd_z() {
    use crate::fs::InMemoryFs;
    let p = PathBuf::from("/notes/nihongo.md");
    let original = "こんにちは\n";
    let mem = InMemoryFs::new().with_file(&p, original);
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
    assert_ne!(app.buffer.text(), original, "the tag landed");
    app.buffer.undo();
    assert_eq!(app.buffer.text(), original, "Cmd-Z removes the stamped tag cleanly");
}

#[test]
fn write_back_never_re_tags_a_document_already_carrying_frontmatter() {
    use crate::fs::InMemoryFs;
    let p = PathBuf::from("/notes/tagged.md");
    // Already tagged (as if a previous session's write-back had already
    // fired and been saved) — must never gain a SECOND block.
    let already = "---\nlang: ja\n---\nこんにちは\n";
    let mem = InMemoryFs::new().with_file(&p, already);
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let app = app_on(Some(p.clone()), "/notes", Config::empty());
    assert_eq!(app.buffer.text(), already, "an already-tagged doc is untouched");
    assert_eq!(
        app.doc_saved_version,
        Some(app.buffer.version()),
        "no edit landed -> still reads as saved"
    );
}

#[test]
fn write_back_never_fires_twice_across_a_reopen() {
    use crate::fs::{FileSystem, InMemoryFs};
    let a = PathBuf::from("/notes/a.md");
    let b = PathBuf::from("/notes/nihongo.md");
    let original = "こんにちは\n";
    let mem = InMemoryFs::new().with_file(&a, "hello\n").with_file(&b, original);
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(a.clone()), "/notes", Config::empty());
    // First open of `b`: tags it (still only in-memory — disk untouched).
    app.load_path(b.clone());
    let tagged = app.buffer.text();
    assert_eq!(tagged, format!("---\nlang: ja\n---\n{original}"));
    // Simulate a save (autosave/Cmd-S would write exactly this).
    mem.write(&b, tagged.as_bytes()).unwrap();
    // Switch away, then back: `load_path`'s SWITCH branch (already open in
    // the registry) restores the live buffer untouched — no second call.
    app.load_path(a.clone());
    app.load_path(b.clone());
    assert_eq!(app.buffer.text(), tagged, "no second frontmatter block, live round trip");
    // And a FRESH session reopening the now-tagged file also never re-tags
    // (the write-back gate is `frontmatter::detect`, not a one-shot flag).
    let app2 = app_on(Some(b.clone()), "/notes", Config::empty());
    assert_eq!(app2.buffer.text(), tagged, "a fresh session sees the tag and never re-fires");
}

#[test]
fn load_path_preserves_a_clobber_notice_the_leaving_flush_just_raised() {
    // REGRESSION (code review nit): if the flush `load_path` runs on the
    // buffer being LEFT hits the autosave clobber guard (the file changed
    // on disk outside awl), the notice it raises must survive the switch
    // — the unconditional `self.notice = None` a few lines later used to
    // wipe it in the very same call, before a single frame ever rendered
    // it, so the user never learned their unsaved edit was held.
    use crate::fs::{FileSystem, InMemoryFs};
    let a = PathBuf::from("/notes/a.md");
    let b = PathBuf::from("/notes/b.md");
    let mem = InMemoryFs::new().with_file(&a, "A\n").with_file(&b, "B\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(a.clone()), "/notes", Config::empty());
    app.buffer.set_text("A edited\n");
    // Someone ELSE writes A behind awl's back before we switch away from it.
    std::thread::sleep(Duration::from_millis(2)); // distinct mtime
    mem.write(&a, b"external edit\n").unwrap();

    app.load_path(b.clone());

    assert_eq!(app.buffer.text(), "B\n", "the switch to B still happens");
    assert_eq!(
        mem.read_to_string(&a).unwrap(),
        "external edit\n",
        "the clobber guard held A's write — the external edit is intact"
    );
    assert_eq!(
        app.notice.as_deref(),
        Some(CLOBBER_NOTICE),
        "the notice raised while leaving A must survive into the switch, not vanish unseen"
    );
}

#[test]
fn scratch_stash_and_restore_round_trip() {
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let stash = crate::fs::scratch_stash_path();
    // A no-file launch, some typing, then a flush (idle/blur/quit all route here).
    let mut app = app_on(None, "/proj", Config::empty());
    app.buffer.set_text("brain dump\n");
    app.autosave_flush();
    assert_eq!(
        mem.read_to_string(&stash).unwrap(),
        "brain dump\n",
        "the scratch stashed"
    );
    assert!(
        !crate::history::list(&stash).is_empty(),
        "the persistent scratch grows its own timeline"
    );
    // A fresh no-argument launch RESTORES it: still path-less, still the
    // markdown-first scratch surface, not a note.
    let mut app2 = app_on(None, "/proj", Config::empty());
    assert_eq!(app2.buffer.text(), "brain dump\n", "the stash restores");
    assert!(app2.buffer.path().is_none(), "restored scratch stays path-less");
    assert!(app2.buffer.is_markdown() && !app2.buffer.is_note());
    // The restore stamped the stash mtime, so a follow-up edit + flush is not
    // mistaken for a two-instance clobber.
    app2.buffer.set_text("brain dump\nmore\n");
    app2.autosave_flush();
    assert_eq!(mem.read_to_string(&stash).unwrap(), "brain dump\nmore\n");
    assert!(app2.notice.is_none(), "no false clobber notice after a restore");
}

// ── SAVE-FEEDBACK round: scratch Save -> note, notice, dirty title marker ──

#[test]
fn convert_scratch_and_save_promotes_the_buffer_and_retires_the_stash() {
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let notes = PathBuf::from("/notes");
    // Stash an OLD scratch content first, exactly like a real prior session
    // would have — the very ghost-copy risk the round's own doc names.
    let stash = crate::fs::scratch_stash_path();
    mem.write(&stash, b"yesterday's dump\n").unwrap();

    let cfg = Config { notes_root: Some(notes.clone()), ..Config::empty() };
    let mut app = app_on(None, "/proj", cfg);
    assert_eq!(app.buffer.text(), "yesterday's dump\n", "restored from the stash first");
    assert!(app.buffer.path().is_none() && !app.buffer.is_note(), "still a true scratch");

    app.convert_scratch_and_save();

    assert!(app.buffer.is_note(), "Cmd-S promoted the scratch buffer into a note");
    let p = app.buffer.path().unwrap().to_path_buf();
    assert!(p.starts_with(&notes), "the note landed under notes_root: {p:?}");
    assert_eq!(mem.read_to_string(&p).unwrap(), "yesterday's dump\n");
    assert_eq!(app.file.as_deref(), Some(p.as_path()), "App.file tracks the new path");
    assert_eq!(app.notice.as_deref(), Some("saved"));
    // THE STASH IS RETIRED: a later bare relaunch must never resurrect a
    // ghost copy of content that is now a real, named file.
    assert!(mem.read_to_string(&stash).is_err(), "the stash file was removed");
}

#[test]
fn convert_scratch_and_save_second_save_is_a_plain_save() {
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let notes = PathBuf::from("/notes");
    let cfg = Config { notes_root: Some(notes), ..Config::empty() };
    let mut app = app_on(None, "/proj", cfg);
    app.buffer.set_text("first entry\n");
    app.convert_scratch_and_save();
    let named = app.buffer.path().unwrap().to_path_buf();

    // A SECOND explicit save (the buffer is now an ordinary note) must
    // NOT re-run the scratch-conversion machinery — same path, same file,
    // just the updated content. `Buffer::save()` here mirrors exactly
    // what `apply_core`'s `Action::Save` arm does before signalling
    // `Effect::SaveDone`; `finish_manual_save` is its post-save
    // bookkeeping half (see `app::apply`'s `Effect::SaveDone` arm).
    app.buffer.set_text("first entry\nmore\n");
    app.buffer.save().unwrap();
    app.finish_manual_save(true, "saved".to_string());
    assert_eq!(app.buffer.path().unwrap(), named, "no re-homing on the second save");
    assert_eq!(mem.read_to_string(&named).unwrap(), "first entry\nmore\n");
}

#[test]
fn convert_scratch_and_save_unwritable_notes_root_raises_a_calm_notice_never_a_panic() {
    // A `notes_root` that can't be written to (a full disk, a permissions
    // error, …) must surface as the SAME calm notice a failed manual save
    // gets — never a terminal print, never a crash, and the scratch stash
    // is left untouched (nothing succeeded to retire it over).
    struct UnwritableFs;
    impl crate::fs::FileSystem for UnwritableFs {
        fn read_to_string(&self, _p: &std::path::Path) -> std::io::Result<String> {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "unwritable fake"))
        }
        fn read(&self, _p: &std::path::Path) -> std::io::Result<Vec<u8>> {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "unwritable fake"))
        }
        fn write(&self, _p: &std::path::Path, _d: &[u8]) -> std::io::Result<()> {
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "notes_root unwritable"))
        }
        fn create_dir_all(&self, _p: &std::path::Path) -> std::io::Result<()> {
            Ok(())
        }
        fn rename(&self, _f: &std::path::Path, _t: &std::path::Path) -> std::io::Result<()> {
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "notes_root unwritable"))
        }
        fn exists(&self, _p: &std::path::Path) -> bool {
            false
        }
        fn is_dir(&self, _p: &std::path::Path) -> bool {
            false
        }
        fn read_dir(&self, _p: &std::path::Path) -> std::io::Result<Vec<crate::fs::DirEntry>> {
            Ok(vec![])
        }
        fn metadata(&self, _p: &std::path::Path) -> std::io::Result<crate::fs::Metadata> {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "unwritable fake"))
        }
        fn remove_file(&self, _p: &std::path::Path) -> std::io::Result<()> {
            Ok(())
        }
    }
    let _g = crate::fs::FsGuard::install(Arc::new(UnwritableFs));
    let notes = PathBuf::from("/notes");
    let cfg = Config { notes_root: Some(notes), ..Config::empty() };
    let mut app = app_on(None, "/proj", cfg);
    app.buffer.set_text("won't land\n");

    app.convert_scratch_and_save();

    assert!(
        app.notice.as_deref().is_some_and(|n| n.starts_with("save failed:")),
        "a calm failure notice, not a panic: {:?}",
        app.notice
    );
}

// ── NOTES VERBS round: Rename note… / Duplicate note ──

#[test]
fn rename_current_file_happy_path_renames_disk_buffer_and_history() {
    use crate::fs::{FileSystem, InMemoryFs};
    let old = PathBuf::from("/notes/old.md");
    let mem = InMemoryFs::new().with_file(&old, "hi\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    // A prior snapshot exists under the OLD path — the ONE-OWNER rename must
    // carry it over so the timeline survives the rename.
    crate::history::record(&old, "hi\n", &Config::empty());
    assert!(!crate::history::list(&old).is_empty(), "arranged: a snapshot exists");

    let mut app = app_on(Some(old.clone()), "/notes", Config::empty());
    assert_eq!(app.buffer.path(), Some(old.as_path()));

    app.rename_current_file("new.md");

    let new = PathBuf::from("/notes/new.md");
    assert_eq!(app.buffer.path(), Some(new.as_path()), "buffer follows the rename");
    assert_eq!(app.file.as_deref(), Some(new.as_path()), "App.file follows the rename");
    assert_eq!(mem.read_to_string(&new).unwrap(), "hi\n", "content moved");
    assert!(mem.read_to_string(&old).is_err(), "the old path is gone");
    assert_eq!(app.notice.as_deref(), Some("renamed to new.md"));
    // THE ONE-OWNER LAW: the history log followed too.
    assert!(!crate::history::list(&new).is_empty(), "history followed to the new path");
    assert!(crate::history::list(&old).is_empty(), "nothing stranded under the old path");
}

#[test]
fn rename_current_file_refuses_to_clobber_an_existing_name() {
    use crate::fs::{FileSystem, InMemoryFs};
    let old = PathBuf::from("/notes/old.md");
    let taken = PathBuf::from("/notes/taken.md");
    let mem = InMemoryFs::new().with_file(&old, "old body\n").with_file(&taken, "taken body\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(old.clone()), "/notes", Config::empty());

    app.rename_current_file("taken.md");

    assert_eq!(app.buffer.path(), Some(old.as_path()), "buffer stays put — refused, not clobbered");
    assert_eq!(mem.read_to_string(&old).unwrap(), "old body\n", "old untouched");
    assert_eq!(mem.read_to_string(&taken).unwrap(), "taken body\n", "never overwritten");
    assert!(
        app.notice.as_deref().is_some_and(|n| n.contains("already a file named")),
        "a calm refusal notice: {:?}",
        app.notice
    );
}

#[test]
fn rename_current_file_refuses_a_git_managed_file() {
    use crate::fs::{FileSystem, InMemoryFs};
    let old = PathBuf::from("/proj/tracked.md");
    let mem = InMemoryFs::new().with_file(&old, "body\n").with_dir("/proj/.git");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(old.clone()), "/proj", Config::empty());

    app.rename_current_file("renamed.md");

    assert_eq!(app.buffer.path(), Some(old.as_path()), "a git-managed file never renames here");
    assert!(mem.exists(&old), "old path untouched");
    assert!(!mem.exists(&PathBuf::from("/proj/renamed.md")), "no new file created");
    assert!(
        app.notice.as_deref().is_some_and(|n| n.contains("git already tracks")),
        "a calm git-managed refusal notice: {:?}",
        app.notice
    );
}

#[test]
fn rename_current_file_unchanged_or_blank_name_is_a_quiet_no_op() {
    use crate::fs::InMemoryFs;
    let old = PathBuf::from("/notes/old.md");
    let mem = InMemoryFs::new().with_file(&old, "hi\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(old.clone()), "/notes", Config::empty());

    app.rename_current_file("old.md");
    assert_eq!(app.buffer.path(), Some(old.as_path()), "unchanged name: no-op");
    assert!(app.notice.is_none(), "no notice for a no-op");

    app.rename_current_file("   ");
    assert_eq!(app.buffer.path(), Some(old.as_path()), "blank name: no-op");
    assert!(app.notice.is_none(), "no notice for a no-op");
}

#[test]
fn duplicate_current_file_dedups_the_name_and_starts_a_fresh_history_timeline() {
    use crate::fs::{FileSystem, InMemoryFs};
    let old = PathBuf::from("/notes/old.md");
    // A prior "old-2.md" already exists, so the dedup must land on "old-3.md".
    let taken2 = PathBuf::from("/notes/old-2.md");
    let mem =
        InMemoryFs::new().with_file(&old, "on disk\n").with_file(&taken2, "someone else's\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    // The old file has its own history timeline.
    crate::history::record(&old, "on disk\n", &Config::empty());
    assert!(!crate::history::list(&old).is_empty(), "arranged: old has history");

    let mut app = app_on(Some(old.clone()), "/notes", Config::empty());
    // Simulate an UNSAVED edit: the duplicate must carry the LIVE buffer
    // content, not necessarily what's on disk.
    app.buffer.set_text("live edit, not yet flushed\n");

    app.duplicate_current_file();

    let dup = PathBuf::from("/notes/old-3.md");
    assert_eq!(app.buffer.path(), Some(dup.as_path()), "switched to the deduped sibling");
    assert_eq!(app.file.as_deref(), Some(dup.as_path()));
    assert_eq!(
        mem.read_to_string(&dup).unwrap(),
        "live edit, not yet flushed\n",
        "the copy captures the buffer's LIVE content"
    );
    assert!(mem.exists(&old), "the original file is untouched, still present");
    assert!(mem.exists(&taken2), "the pre-existing -2 sibling is never clobbered");
    // FRESH HISTORY: the duplicate is a brand-new file, so its own timeline
    // starts empty, even though the SOURCE had history.
    assert!(crate::history::list(&dup).is_empty(), "the copy starts a fresh history timeline");
    // The ORIGINAL buffer was PARKED (backgrounded), never discarded — its
    // pending edit is still flushed to disk (autosave_flush runs before the
    // dedup scan) and its live state survives in the registry.
    let key = crate::buffers::BufferKey::path(&old);
    assert!(app.buffer_registry.contains(&key), "the original was parked, not dropped");
}

#[test]
fn duplicate_current_file_on_a_pathless_buffer_is_a_quiet_no_op() {
    // HERMETIC: install an InMemoryFs before `App::new` so this test never
    // touches the machine's real `session.toml`/stash (`app_on(None, ..)`
    // runs the full App startup). FsGuard also holds `testlock::serial()`
    // for the test's life, so the pass no longer rides ordering luck.
    use crate::fs::InMemoryFs;
    let mem = InMemoryFs::new().with_dir("/proj");
    let _g = crate::fs::FsGuard::install(Arc::new(mem));
    let mut app = app_on(None, "/proj", Config::empty());
    assert!(app.buffer.path().is_none());
    app.duplicate_current_file();
    assert!(app.buffer.path().is_none(), "nothing to duplicate yet");
    assert!(app.notice.is_none());
}

#[test]
fn finish_manual_save_ok_is_silent_failure_notices_the_error() {
    // SAVE-UX round: a SUCCESSFUL manual save raises NO bottom-center notice
    // (autosave is already silent; a lone non-fading "saved" is just noise).
    // A FAILURE still surfaces its error — errors must never go silent.
    use crate::fs::InMemoryFs;
    let _l = crate::testlock::serial();
    let p = PathBuf::from("/notes/draft.md");
    let mem = InMemoryFs::new().with_file(&p, "v1\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(p.clone()), "/notes", Config::empty());

    app.finish_manual_save(true, "saved".to_string());
    assert_eq!(app.notice.as_deref(), Some("saved"));
    assert_eq!(app.notice_kind, NoticeKind::Toast);
    assert!(app.notice_expires_at.is_none(), "a headless test never arms a live timer");

    app.finish_manual_save(false, "save failed: disk full".to_string());
    assert_eq!(app.notice.as_deref(), Some("save failed: disk full"));
}

#[test]
fn finish_manual_save_clears_a_notes_dirty_marker_immediately() {
    // BUG LOCK-DOWN: `is_document_dirty` reads `autosave_saved_version` for a
    // NOTE, but `finish_manual_save` used to stamp only `doc_saved_version`
    // — so ⌘S on a note left it reading dirty (the title `•` + native
    // titlebar dot lingering) until the note's ~400ms debounced autosave
    // redundantly rewrote and finally stamped the field.
    use crate::fs::InMemoryFs;
    let _l = crate::testlock::serial();
    let notes = PathBuf::from("/notes");
    let mem = InMemoryFs::new().with_dir(&notes);
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(None, "/notes", Config::empty());

    // Make the active buffer a NOTE with content, then write it to disk the
    // way `apply_core`'s `Action::Save` arm does before signalling SaveDone.
    app.buffer.start_note(notes.clone());
    app.buffer.set_text("note body\n");
    app.buffer.save().unwrap();
    assert!(app.buffer.is_note() && app.buffer.path().is_some(), "arranged: a saved note");
    // Pre-bookkeeping the note reads DIRTY: `autosave_saved_version` is still
    // stale (None) against the edited version.
    assert!(app.is_document_dirty(), "arranged: the note reads dirty pre-bookkeeping");

    app.finish_manual_save(true, "saved".to_string());

    assert!(!app.is_document_dirty(), "a note is clean IMMEDIATELY after ⌘S, not ~400ms later");
    assert!(app.autosave_dirty_at.is_none(), "the redundant ~400ms note rewrite is suppressed");
}

#[test]
fn finish_manual_save_clears_a_regular_files_dirty_marker_immediately() {
    // REGRESSION GUARD: a path-backed file reads `doc_saved_version` in
    // `is_document_dirty` — it was always fine, and must stay fine.
    use crate::fs::InMemoryFs;
    let _l = crate::testlock::serial();
    let p = PathBuf::from("/proj/doc.md");
    let mem = InMemoryFs::new().with_file(&p, "v1\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(p.clone()), "/proj", Config::empty());

    app.buffer.set_text("edited body\n");
    app.buffer.save().unwrap();
    assert!(!app.buffer.is_note() && app.buffer.path().is_some(), "arranged: a saved file");
    assert!(app.is_document_dirty(), "arranged: the file reads dirty pre-bookkeeping");

    app.finish_manual_save(true, "saved".to_string());

    assert!(!app.is_document_dirty(), "a regular file is clean immediately after ⌘S");
}

// ── SAVE-FEEDBACK round: the ambient dirty title marker ──

#[test]
fn sync_view_retitles_only_on_an_actual_dirty_flip() {
    let mut app = app_on(None, "/proj", Config::empty());
    assert!(!app.title_dirty, "a fresh scratch buffer starts clean");
    // No gpu/window in a hermetic App: `sync_view` bails before the title
    // comparison (its own gpu-present gate) — this proves the flip-tracking
    // logic itself is reachable + correct via `is_document_dirty` directly,
    // mirroring `update_title_uses_the_same_pure_window_title`'s own
    // "no live window, still exercised" shape.
    assert!(!app.is_document_dirty(), "just-loaded content starts saved");
    app.buffer.set_text("edited\n");
    assert!(app.is_document_dirty(), "an edit past the saved version is dirty");
}

#[test]
fn is_document_dirty_clears_on_autosave_not_just_manual_save() {
    // The definition this round settled on for the title's dirty marker:
    // "unsaved" by the SAME version-vs-saved-version bookkeeping the
    // autosave engine tracks — so an AUTOSAVED (not manually Cmd-S'd)
    // document reads as clean too, never stuck showing the edited marker
    // on content that's already safely on disk.
    use crate::fs::{FileSystem, InMemoryFs};
    let p = PathBuf::from("/notes/draft.md");
    let mem = InMemoryFs::new().with_file(&p, "v1\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
    assert!(!app.is_document_dirty());
    app.buffer.set_text("v2\n");
    assert!(app.is_document_dirty(), "an unsaved edit reads dirty");
    app.autosave_flush(); // NOT a manual save — the background engine
    assert_eq!(mem.read_to_string(&p).unwrap(), "v2\n");
    assert!(!app.is_document_dirty(), "autosave clears the dirty marker too");
}

#[test]
fn scratch_stash_invalid_utf8_preserves_a_corrupt_sibling_then_starts_a_blank_scratch() {
    // DATA-SAFETY HARDENING: the scratch stash IS a manuscript, so a
    // stash file that's PRESENT but fails to decode as UTF-8 text (real
    // disk corruption, never a bug write_atomic itself can produce) must
    // never be silently discarded — a `.corrupt-*` sibling preserves the
    // raw bytes BEFORE `App::new` falls back to a blank scratch buffer.
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let stash = crate::fs::scratch_stash_path();
    // Invalid UTF-8: a lone continuation byte can never decode.
    mem.write(&stash, &[0x2E, 0x62, 0xFF, 0xFE, 0x0A]).unwrap();

    let app = app_on(None, "/proj", Config::empty());
    assert_eq!(app.buffer.text(), "", "an undecodable stash falls back to a blank scratch");
    assert!(app.buffer.path().is_none());

    let dir = stash.parent().unwrap();
    let names: Vec<String> = mem.read_dir(dir).unwrap().into_iter().map(|e| e.name).collect();
    let stash_name = stash.file_name().unwrap().to_string_lossy().into_owned();
    let backup_prefix = format!("{stash_name}.corrupt-");
    let backups: Vec<&String> = names.iter().filter(|n| n.starts_with(&backup_prefix)).collect();
    assert_eq!(backups.len(), 1, "exactly one corrupt sibling preserved: {names:?}");
    let backup_bytes = mem.read(&dir.join(backups[0])).unwrap();
    assert_eq!(
        backup_bytes,
        vec![0x2E, 0x62, 0xFF, 0xFE, 0x0A],
        "the sibling holds the ORIGINAL undecodable bytes verbatim"
    );
}

#[test]
fn blur_flush_never_reloads_buffer_or_resets_cursor() {
    // WEB STRESS-TEST HYPOTHESIS (characterized, not reproduced): a Playwright
    // run typing "AAA" then, in a LATER dispatch batch, "BBB" observed BBB
    // landing at buffer position 0 instead of after "AAA", as if a blur/
    // visibility flap between the two batches made the web build RE-LOAD the
    // scratch from its localStorage stash mid-session (which would restore
    // the STASHED content and reset the cursor to 0 — restoring a buffer
    // always starts a fresh Buffer at cursor 0, see `App::new`).
    //
    // `WindowEvent::Focused(false)` is the one live door a blur reaches —
    // and it calls exactly `App::autosave_flush` (`app.rs`'s `Focused(false)`
    // arm), which fans out to `stash_scratch_now` for a no-path scratch. That
    // function is a pure WRITE: it reads `self.buffer.text()` and writes it
    // OUT to the stash path; it never calls `crate::fs::active().read_*` or
    // reconstructs `self.buffer`. The ONLY place a stash is ever read back
    // INTO a buffer is `App::new` (a true process/page (re)launch) — never a
    // blur, never any other live-App path. This test pins that down: typing
    // "AAA", flushing (the blur trigger) as many times as a stress test's
    // spurious focus flapping might, then typing "BBB" must land the cursor
    // right after "AAA", not at 0.
    use crate::fs::InMemoryFs;
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(None, "/proj", Config::empty());
    for c in "AAA".chars() {
        app.buffer.insert_char(c);
    }
    assert_eq!(app.buffer.cursor_char(), 3, "cursor sits after the typed AAA");
    // Simulate the exact call the live `Focused(false)` arm makes — as many
    // times as a flappy test harness might re-fire it between dispatches.
    app.autosave_flush();
    app.autosave_flush();
    app.autosave_flush();
    assert_eq!(app.buffer.text(), "AAA", "a blur-driven flush never reloads content");
    assert_eq!(
        app.buffer.cursor_char(),
        3,
        "a blur-driven flush never resets the cursor — only App::new restores"
    );
    // A later "dispatch batch" continues typing from exactly where it left off.
    for c in "BBB".chars() {
        app.buffer.insert_char(c);
    }
    assert_eq!(
        app.buffer.text(),
        "AAABBB",
        "BBB lands after AAA, not at position 0"
    );
    assert_eq!(app.buffer.cursor_char(), 6);
}

#[test]
fn scratch_restore_skips_empty_stash() {
    use crate::fs::{FileSystem, InMemoryFs};
    // An EMPTY stash restores nothing (plain scratch)… (each half owns its
    // FsGuard — the guard holds the process-wide FS lock, so they must not
    // overlap on one thread.)
    {
        let mem = InMemoryFs::new();
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        mem.write(&crate::fs::scratch_stash_path(), b"").unwrap();
        let app = app_on(None, "/proj", Config::empty());
        assert!(app.buffer.text().is_empty(), "empty stash → plain scratch");
    }
    // …and so does a MISSING one (fresh fake).
    {
        let _g = crate::fs::FsGuard::install(Arc::new(InMemoryFs::new()));
        let app = app_on(None, "/proj", Config::empty());
        assert!(app.buffer.text().is_empty(), "missing stash → plain scratch");
    }
}

#[test]
fn autosave_writes_git_files_but_never_snapshots_them() {
    // LOCKED DECISION 4, both halves at the App seam: autosave still WRITES
    // a git-managed file (writing is not version-meddling), but records NO
    // awl snapshot for it — its timeline stays git log alone.
    use crate::fs::{FileSystem, InMemoryFs};
    let p = PathBuf::from("/repo/doc.md");
    let mem = InMemoryFs::new().with_dir("/repo/.git").with_file(&p, "v1\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(p.clone()), "/repo", Config::empty());
    app.buffer.set_text("v2\n");
    app.autosave_flush();
    assert_eq!(
        mem.read_to_string(&p).unwrap(),
        "v2\n",
        "autosave still WRITES a git-managed file"
    );
    assert!(app.notice.is_none(), "a clean write raises no notice");
    // The snapshot store never grew a log dir — the record gate held.
    let store = crate::fs::data_root().join("history");
    assert!(
        mem.read_dir(&store).map(|v| v.is_empty()).unwrap_or(true),
        "no awl snapshot log for a git-managed file"
    );
}

#[test]
fn scratch_stash_clobber_guard_holds_two_instance_writes() {
    // TWO-INSTANCE SAFETY: another awl (or anything) writes the stash after
    // this instance launched — the flush HOLDS (the external stash content
    // survives) and raises the same calm notice as the document guard.
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let stash = crate::fs::scratch_stash_path();
    let mut app = app_on(None, "/proj", Config::empty());
    mem.write(&stash, b"the other instance's dump\n").unwrap();
    app.buffer.set_text("mine\n");
    app.autosave_flush();
    assert_eq!(
        mem.read_to_string(&stash).unwrap(),
        "the other instance's dump\n",
        "the stash write is held — external content survives"
    );
    assert_eq!(
        app.notice.as_deref(),
        Some(CLOBBER_NOTICE),
        "the calm notice names the hold"
    );
}

#[test]
fn emptied_scratch_clears_the_stale_stash() {
    // The stash writes EVEN EMPTY text: emptying the restored scratch and
    // flushing must clear yesterday's dump, or a deliberately-emptied
    // scratch would resurrect on the next launch.
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let stash = crate::fs::scratch_stash_path();
    mem.write(&stash, b"yesterday's dump\n").unwrap();
    let mut app = app_on(None, "/proj", Config::empty());
    assert_eq!(app.buffer.text(), "yesterday's dump\n", "the stash restored");
    app.buffer.set_text("");
    app.autosave_flush();
    assert_eq!(
        mem.read_to_string(&stash).unwrap(),
        "",
        "an emptied scratch clears the stale stash"
    );
    assert!(app.notice.is_none(), "our own restore is not an external edit");
}

// ── The HISTORY TIMELINE live preview (App-level, InMemoryFs seam) ───────
//
// The preview is DERIVED at ViewState-build time — these tests pin the
// resolver (`history_preview_text`) and the close contract
// (`history_overlay_closed`) directly, buffer untouched throughout.

/// Seed two history versions for `p` and open the History overlay on `app`,
/// exactly as the OpenHistory gather builds it (timeline_rows → new_history).
fn open_history_overlay(app: &mut App, p: &std::path::Path) {
    let rows = crate::history::timeline_rows(
        p,
        &app.buffer.text(),
        crate::history::now_millis(),
    );
    app.overlay = Some(crate::overlay::OverlayState::new_history(rows, None, None));
}

#[test]
fn history_preview_resolves_without_touching_buffer() {
    use crate::fs::{FileSystem, InMemoryFs};
    let p = PathBuf::from("/notes/draft.md");
    let mem = InMemoryFs::new().with_file(&p, "the second draft entirely\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    crate::history::record(&p, "the first draft wording\n", &Config::empty());
    crate::history::record(&p, "the second draft entirely\n", &Config::empty());
    let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
    let version_before = app.buffer.version();
    open_history_overlay(&mut app, &p);
    // DIFF-AS-PREVIEW: row 0 (newest, identical to the buffer) previews a
    // folds-only transcript; row 1 (older) previews a transcript CARRYING the
    // change marks (the reworded paragraph shows surgery / a rewrite).
    let newest = app.history_preview_text().expect("row 0 previews");
    assert!(newest.starts_with("# Comparing with "), "a titled transcript: {newest}");
    assert!(
        !newest.contains("~~") && !newest.contains("=="),
        "identical content diffs to no marks: {newest}"
    );
    app.overlay.as_mut().unwrap().move_sel(1);
    let older = app.history_preview_text().expect("row 1 previews");
    assert!(
        older.contains("~~") || older.contains("=="),
        "arrowing to the older version previews ITS diff (marks present): {older}"
    );
    // The BUFFER was never touched: content, version, and undo all intact.
    assert_eq!(app.buffer.text(), "the second draft entirely\n");
    assert_eq!(app.buffer.version(), version_before, "no version bump");
    // The per-id CACHE serves a repeat without re-reading the store: blow the
    // store away and the highlighted row still previews from the cache.
    let hist_dir = crate::fs::data_root().join("history");
    for entry in mem.read_dir(&hist_dir).unwrap_or_default() {
        let _ = mem.rename(&entry.path, std::path::Path::new("/gone"));
    }
    assert_eq!(
        app.history_preview_text().as_deref(),
        Some(older.as_str()),
        "a repeat on the same id is a cache hit"
    );
}

#[test]
fn preview_cache_invalidates_on_selection_move() {
    use crate::fs::InMemoryFs;
    let p = PathBuf::from("/notes/draft.md");
    let mem = InMemoryFs::new().with_file(&p, "v2\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    crate::history::record(&p, "v1\n", &Config::empty());
    crate::history::record(&p, "v2\n", &Config::empty());
    let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
    open_history_overlay(&mut app, &p);
    assert!(app.history_preview_text().is_some());
    let cached_id = app.history_preview.as_ref().map(|(id, _)| id.clone());
    // Moving the selection to another row (a different id) re-renders: the
    // cache is keyed by id, never by "an overlay is open". (The selection
    // move also resets the diff panel scroll — the transcript changed.)
    app.overlay.as_mut().unwrap().diff_scroll = 7;
    app.overlay.as_mut().unwrap().move_sel(1);
    assert_eq!(app.overlay.as_ref().unwrap().diff_scroll, 0, "a new version tops the diff out");
    assert!(app.history_preview_text().is_some());
    assert_ne!(
        app.history_preview.as_ref().map(|(id, _)| id.clone()),
        cached_id,
        "the cache now holds the newly highlighted id"
    );
}

#[test]
fn history_close_without_accept_restores_scroll_and_drops_preview() {
    use crate::fs::InMemoryFs;
    let _g = crate::fs::FsGuard::install(Arc::new(InMemoryFs::new()));
    let mut app = app_on(None, "/proj", Config::empty());
    // A shorter previewed version clamped the scroll while the picker was
    // open; the close-without-accept restores the saved scroll EXACTLY
    // ("Esc = back to now") and puts the preview down.
    app.history_scroll_before = Some(42);
    app.scroll_lines = 3;
    app.history_preview = Some(("100".into(), "old\n".into()));
    app.history_overlay_closed(false);
    assert_eq!(app.scroll_lines, 42, "Esc restores the pre-open scroll");
    assert!(app.history_scroll_before.is_none());
    assert!(app.history_preview.is_none(), "the preview is dropped");
    // A real ACCEPT keeps the current viewport (the restored version owns
    // it) — the saved scroll is discarded, the preview still dropped.
    app.history_scroll_before = Some(42);
    app.scroll_lines = 3;
    app.history_preview = Some(("100".into(), "old\n".into()));
    app.history_overlay_closed(true);
    assert_eq!(app.scroll_lines, 3, "an accept never yanks the viewport");
    assert!(app.history_scroll_before.is_none());
    assert!(app.history_preview.is_none());
}

// ── DIFF-AS-PREVIEW — the History picker's writer's-diff preview ────────
//
// The diff IS the picker's live preview now (the takeover Compare view is
// retired). These pin the transcript's shape and the read-only invariants on
// the PREVIEW path (buffer / version / undo untouched — the successor of
// the old diff_view_gate suite). The render is SYNCHRONOUS: the round's
// release perf probe measured ~1-2 ms per diff at SCOPE.md scale (the diff
// folds unchanged regions, so the transcript stays tiny), so no per-arrow
// debounce is warranted; the old settle machinery was cut.

#[test]
fn diff_preview_renders_marked_up_transcript_without_touching_buffer() {
    use crate::fs::InMemoryFs;
    let p = PathBuf::from("/notes/draft.md");
    // Current buffer keeps the first paragraph, drops the second, adds a third.
    let now = "Keep this opening paragraph exactly as it was.\n\nAn entirely fresh third paragraph appears here now.\n";
    let mem = InMemoryFs::new().with_file(&p, now);
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    // Seed an older version (the one the highlighted row compares against).
    let old = "Keep this opening paragraph exactly as it was.\n\nDrop this whole second paragraph entirely please.\n";
    crate::history::record(&p, old, &Config::empty());
    let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
    let version_before = app.buffer.version();
    let text_before = app.buffer.text();
    open_history_overlay(&mut app, &p);
    let transcript = app.history_preview_text().expect("the diff preview is live");
    // The transcript speaks awl's diff vocabulary: a struck deletion (REAL
    // `~~` markdown) AND a highlight-washed insertion (`==`), under a title
    // heading naming the compared row.
    assert!(transcript.starts_with("# Comparing with "), "{transcript}");
    assert!(transcript.contains("~~"), "a struck deletion: {transcript}");
    assert!(transcript.contains("=="), "a washed insertion: {transcript}");
    // The BUFFER was never touched — content, version, undo all intact.
    assert_eq!(app.buffer.text(), text_before, "preview never mutates the buffer");
    assert_eq!(app.buffer.version(), version_before, "no version bump");
    app.buffer.undo();
    assert_eq!(app.buffer.text(), text_before, "undo after preview is inert");
}

#[test]
fn diff_preview_read_only_law_typing_edits_the_query_never_the_buffer() {
    // THE READ-ONLY LAW on the preview path (the successor of the retired
    // diff_view_gate suite): while the History picker's diff preview is up,
    // the overlay's MODALITY is the law — every key routes through
    // `overlay_intercept`, so typing filters the query, Tab shifts focus,
    // PgUp/PgDn scroll the panel, and NOTHING reaches the rope.
    use crate::fs::InMemoryFs;
    let p = PathBuf::from("/notes/draft.md");
    let mem = InMemoryFs::new().with_file(&p, "current words\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    crate::history::record(&p, "older words\n", &Config::empty());
    crate::history::record(&p, "current words\n", &Config::empty());
    let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
    let version_before = app.buffer.version();
    open_history_overlay(&mut app, &p);
    assert!(app.history_preview_text().is_some(), "preview live");
    // Drive the modal intercept exactly as a keypress would (the core seam).
    for act in [
        Action::InsertChar('z'),
        Action::InsertTab,
        Action::PageScrollDown,
        Action::NextLine,
        Action::DeleteBackward,
    ] {
        app.apply_core_for_test(&act);
    }
    assert_eq!(app.buffer.text(), "current words\n", "the rope never changed");
    assert_eq!(app.buffer.version(), version_before, "no version bump");
    // Esc from panel focus returns to the LIST; a second Esc closes — two
    // Escs total from panel to home, and the buffer text is back untouched.
    app.apply_core_for_test(&Action::InsertTab); // focus the panel
    assert!(app.overlay.as_ref().unwrap().diff_focus);
    app.apply_core_for_test(&Action::Cancel);
    assert!(app.overlay.is_some(), "first Esc: back to LIST focus, not home");
    assert!(!app.overlay.as_ref().unwrap().diff_focus);
    app.apply_core_for_test(&Action::Cancel);
    assert!(app.overlay.is_none(), "second Esc closes the picker");
    assert_eq!(app.buffer.version(), version_before, "back to now exactly");
}

#[test]
fn scratch_buffer_lists_its_stash_history() {
    use crate::fs::InMemoryFs;
    let _g = crate::fs::FsGuard::install(Arc::new(InMemoryFs::new()));
    // The persistent scratch stashes (autosave engine) — recording history
    // under its stash path — and the timeline gather's shared source_path
    // fallback finds it, so the no-path scratch has a summonable timeline.
    let mut app = app_on(None, "/proj", Config::empty());
    app.buffer.set_text("scratch thoughts\n");
    app.autosave_flush();
    let key = crate::history::source_path(
        app.buffer.path(),
        app.file.as_deref(),
        app.buffer.is_note(),
    )
    .expect("the true scratch keys under its stash");
    assert_eq!(key, crate::fs::scratch_stash_path());
    let rows = crate::history::timeline_rows(
        &key,
        &app.buffer.text(),
        crate::history::now_millis(),
    );
    assert!(!rows.is_empty(), "the scratch stash has a timeline");
    // And the preview resolver rides the same key: the newest row previews
    // the stashed content.
    app.overlay = Some(crate::overlay::OverlayState::new_history(rows, None, None));
    // DIFF-AS-PREVIEW: the stash's newest snapshot is identical to the
    // buffer, so the preview is a titled folds-only transcript.
    let transcript = app.history_preview_text().expect("the stash previews");
    assert!(transcript.starts_with("# Comparing with "), "{transcript}");
}

#[test]
fn notes_keep_their_own_autosave() {
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(None, "/proj", Config::empty());
    app.buffer.start_note(PathBuf::from("/mynotes"));
    app.buffer.set_text("a note in flight\n");
    app.autosave_flush();
    // The DOC engine leaves notes to their own 400ms flow (flush_note): no
    // scratch stash, no note file written by this door.
    assert!(
        mem.read(&crate::fs::scratch_stash_path()).is_err(),
        "a note is never stashed as scratch"
    );
    assert!(
        mem.read_dir(std::path::Path::new("/mynotes"))
            .map(|v| v.is_empty())
            .unwrap_or(true),
        "autosave_flush does not write note files"
    );
}

// ── MULTI-BUFFER REGISTRY (App-level: open/switch preserves everything) ──

#[test]
fn load_path_switches_to_already_open_buffer_preserving_edits_and_cursor() {
    // THE v1 OBSERVABLE WIN: re-opening a file already open in this session
    // restores its LIVE buffer (unsaved edits, cursor) instead of re-reading
    // disk. Proven by mutating A's on-disk bytes BEHIND awl's back while B is
    // active, then asserting the restored A shows the in-memory edit, not the
    // disk write.
    use crate::fs::{FileSystem, InMemoryFs};
    let a = PathBuf::from("/proj/a.txt");
    let b = PathBuf::from("/proj/b.txt");
    let mem = InMemoryFs::new().with_file(&a, "alpha\n").with_file(&b, "beta\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(a.clone()), "/proj", Config::empty());
    app.buffer.set_text("ALPHA EDITED\n");
    app.buffer.set_cursor(3);
    assert_eq!(app.open_buffer_count(), 1, "only A is open so far");

    app.load_path(b.clone());
    assert_eq!(app.buffer.text(), "beta\n", "B loads fresh from disk (first open)");
    assert_eq!(app.open_buffer_count(), 2, "A is now backgrounded, not closed");
    app.buffer.set_text("BETA EDITED\n");

    mem.write(&a, b"ALPHA CHANGED ON DISK\n").unwrap();

    app.load_path(a.clone());
    assert_eq!(
        app.buffer.text(),
        "ALPHA EDITED\n",
        "the LIVE unsaved edit survived the round trip, not a re-read from disk"
    );
    assert_eq!(app.buffer.cursor_char(), 3, "the cursor position survived too");
    assert!(app.buffer.is_dirty(), "the unsaved edit is still unsaved");
    assert_eq!(app.open_buffer_count(), 2, "A active again, B backgrounded");

    // And B's OWN edit is preserved too (not silently dropped when we left it).
    app.load_path(b.clone());
    assert_eq!(app.buffer.text(), "BETA EDITED\n", "B's edit also survived");
}

// ── PROSE/CODE PAGE-WIDTH SPLIT (App-level buffer-switch resync) ────────

#[test]
fn load_path_switch_reapplies_default_measure_per_kind() {
    // WIRING (1): a buffer SWITCH re-applies the right measure through the
    // existing `set_measure` seam (`App::sync_page_measure`, called from
    // `load_path`). A.md (prose) -> B.rs (code) -> back to A.md, with NO
    // config override, must land on each class's own BUILT-IN default.
    use crate::fs::InMemoryFs;
    let a = PathBuf::from("/proj/a.md");
    let b = PathBuf::from("/proj/b.rs");
    let mem = InMemoryFs::new().with_file(&a, "# hello\n").with_file(&b, "fn main() {}\n");
    let _g2 = crate::fs::FsGuard::install(Arc::new(mem));
    // LOCK ORDER: fs seam first, page lock LAST (see page::test_lock()'s doc)
    // — the reverse order deadlocks against every fs-holding test whose
    // load_path transitively writes the measure.
    let _g = crate::testlock::serial();
    let measure0 = crate::page::measure();
    let mut app = app_on(Some(a.clone()), "/proj", Config::empty());

    // Deliberately wrong, so the switches below can't coincidentally "already"
    // hold the right value.
    crate::page::set_measure(12345);
    app.load_path(b.clone());
    assert_eq!(
        crate::page::measure(),
        crate::page::DEFAULT_MEASURE_CODE,
        "switching to B.rs (code) applies the code default"
    );
    app.load_path(a.clone());
    assert_eq!(
        crate::page::measure(),
        crate::page::DEFAULT_MEASURE,
        "switching back to A.md (prose) applies the prose default"
    );

    crate::page::set_measure(measure0);
}

#[test]
fn load_path_switch_reapplies_custom_measure_overrides() {
    // The SAME A.md/B.rs round trip, but with configured overrides for BOTH
    // classes — the switch must read `Config::measure_for`, not just the
    // built-in defaults.
    use crate::fs::InMemoryFs;
    let a = PathBuf::from("/proj/a.md");
    let b = PathBuf::from("/proj/b.rs");
    let mem = InMemoryFs::new().with_file(&a, "hello\n").with_file(&b, "fn main() {}\n");
    let _g2 = crate::fs::FsGuard::install(Arc::new(mem));
    let _g = crate::testlock::serial(); // fs first, page LAST (see page::test_lock())
    let measure0 = crate::page::measure();
    let cfg = Config { page_width_prose: Some(55), page_width_code: Some(120), ..Config::empty() };
    let mut app = app_on(Some(a.clone()), "/proj", cfg);

    crate::page::set_measure(1);
    app.load_path(b.clone());
    assert_eq!(crate::page::measure(), 120, "B.rs picks up the configured code override");
    app.load_path(a.clone());
    assert_eq!(crate::page::measure(), 55, "back to A.md picks up the configured prose override");

    crate::page::set_measure(measure0);
}

#[test]
fn new_note_always_reapplies_the_prose_measure() {
    // A fresh quick note is always markdown (PROSE), regardless of what kind
    // of buffer was active before it — `new_note` calls the same
    // `sync_page_measure` resync `load_path` does.
    use crate::fs::InMemoryFs;
    let b = PathBuf::from("/proj/b.rs");
    let mem = InMemoryFs::new().with_file(&b, "fn main() {}\n");
    let _g2 = crate::fs::FsGuard::install(Arc::new(mem));
    let _g = crate::testlock::serial(); // fs first, page LAST (see page::test_lock())
    let measure0 = crate::page::measure();
    let mut app = app_on(Some(b.clone()), "/proj", Config::empty());

    crate::page::set_measure(crate::page::DEFAULT_MEASURE_CODE);
    app.new_note();
    assert_eq!(
        crate::page::measure(),
        crate::page::DEFAULT_MEASURE,
        "a new note is prose, so it gets the prose default even leaving a code buffer"
    );

    crate::page::set_measure(measure0);
}

#[test]
fn persist_page_width_writes_the_key_matching_the_active_buffer_kind() {
    // The STICKY WRITE half (drag-resize / C-x { / C-x }): `persist_page_width`
    // must target `page_width_prose` while a prose buffer is active and
    // `page_width_code` while a code buffer is active — never the other key.
    use crate::fs::InMemoryFs;
    let cfg_path = PathBuf::from("/home/.config/awl/config.toml");
    let a = PathBuf::from("/proj/a.md");
    let b = PathBuf::from("/proj/b.rs");
    let mem = InMemoryFs::new().with_file(&a, "hello\n").with_file(&b, "fn main() {}\n");
    let _g2 = crate::fs::FsGuard::install(Arc::new(mem));
    let _g = crate::testlock::serial(); // fs first, page LAST (see page::test_lock())
    let measure0 = crate::page::measure();
    let cfg = Config { path: cfg_path.clone(), ..Config::empty() };
    let mut app = app_on(Some(a.clone()), "/proj", cfg);

    crate::page::set_measure(55);
    app.persist_page_width();
    let reloaded = Config::load(cfg_path.clone());
    assert_eq!(reloaded.page_width_prose, Some(55), "a PROSE buffer persists to page_width_prose");
    assert_eq!(reloaded.page_width_code, None, "the code key is untouched");

    app.load_path(b.clone());
    crate::page::set_measure(130);
    app.persist_page_width();
    let reloaded2 = Config::load(cfg_path.clone());
    assert_eq!(reloaded2.page_width_code, Some(130), "a CODE buffer persists to page_width_code");
    assert_eq!(reloaded2.page_width_prose, Some(55), "the prose key from before survives untouched");

    crate::page::set_measure(measure0);
}

#[test]
fn persist_page_reset_clears_the_key_matching_the_active_buffer_kind() {
    // The RESET half: `persist_page_reset` must clear ONLY the override
    // matching the active buffer's kind, leaving the other class's override
    // (and every other pref) untouched.
    use crate::fs::InMemoryFs;
    let cfg_path = PathBuf::from("/home/.config/awl/config.toml");
    let a = PathBuf::from("/proj/a.md");
    let b = PathBuf::from("/proj/b.rs");
    let mem = InMemoryFs::new().with_file(&a, "hello\n").with_file(&b, "fn main() {}\n");
    let _g2 = crate::fs::FsGuard::install(Arc::new(mem));
    let _g = crate::testlock::serial(); // fs first, page LAST (see page::test_lock())
    let measure0 = crate::page::measure();
    Config::write_pref(&cfg_path, "page_width_prose", "55").unwrap();
    Config::write_pref(&cfg_path, "page_width_code", "130").unwrap();
    let cfg = Config::load(cfg_path.clone());
    let mut app = app_on(Some(b.clone()), "/proj", cfg); // start on the CODE file

    app.persist_page_reset();
    let reloaded = Config::load(cfg_path.clone());
    assert_eq!(reloaded.page_width_code, None, "the code override is cleared");
    assert_eq!(reloaded.page_width_prose, Some(55), "the prose override survives untouched");

    app.load_path(a.clone());
    app.persist_page_reset();
    let reloaded2 = Config::load(cfg_path.clone());
    assert_eq!(reloaded2.page_width_prose, None, "the prose override is now also cleared");

    crate::page::set_measure(measure0);
}

#[test]
fn setting_value_commit_clamps_persists_and_applies_measure_and_zoom() {
    // SETTINGS v2 inline VALUE edit (App half): parse + clamp the typed value,
    // apply it LIVE (page::measure / zoom), and persist the NAMED key.
    use crate::fs::InMemoryFs;
    let cfg_path = PathBuf::from("/home/.config/awl/config.toml");
    let a = PathBuf::from("/proj/a.md"); // a PROSE (.md) buffer
    let mem = InMemoryFs::new().with_file(&a, "hello\n");
    let _g2 = crate::fs::FsGuard::install(Arc::new(mem));
    let _g = crate::testlock::serial(); // fs first, page LAST (see page::test_lock())
    let measure0 = crate::page::measure();
    let cfg = Config { path: cfg_path.clone(), ..Config::empty() };
    let mut app = app_on(Some(a.clone()), "/proj", cfg);

    // In-range prose width: applied LIVE (the active buffer is prose) + persisted.
    app.setting_value_commit("page_width_prose", "45");
    assert_eq!(crate::page::measure(), 45, "a prose-width edit re-wraps live");
    assert_eq!(Config::load(cfg_path.clone()).page_width_prose, Some(45));

    // Out of range: CLAMPED to PAGE_WIDTH_MAX, both live + on disk.
    app.setting_value_commit("page_width_prose", "5000");
    assert_eq!(crate::page::measure(), crate::settings::PAGE_WIDTH_MAX);
    assert_eq!(
        Config::load(cfg_path.clone()).page_width_prose,
        Some(crate::settings::PAGE_WIDTH_MAX)
    );

    // Unparseable: a calm no-op (measure + config unchanged).
    app.setting_value_commit("page_width_prose", "oops");
    assert_eq!(crate::page::measure(), crate::settings::PAGE_WIDTH_MAX);

    // Editing the CODE width while a PROSE buffer is active persists to its own key
    // but does NOT change the visible measure (sync_page_measure reads the active
    // class), so the prose/code split never bleeds.
    app.setting_value_commit("page_width_code", "88");
    assert_eq!(
        crate::page::measure(),
        crate::settings::PAGE_WIDTH_MAX,
        "the code-width edit leaves the prose measure alone"
    );
    assert_eq!(Config::load(cfg_path.clone()).page_width_code, Some(88));

    // ZOOM: the percent readout form parses + clamps through the shared set_zoom
    // owner + persists.
    app.setting_value_commit("zoom", "150%");
    assert!((app.zoom - 1.5).abs() < 1e-4, "150% -> factor 1.5");
    assert_eq!(Config::load(cfg_path.clone()).zoom, Some(1.5));

    crate::page::set_measure(measure0);
}

#[test]
fn load_path_reopening_the_active_file_is_a_noop() {
    // Re-"opening" the file that is already active must not disturb anything
    // (no park/restore round trip, no fresh disk read either).
    use crate::fs::InMemoryFs;
    let a = PathBuf::from("/proj/a.txt");
    let mem = InMemoryFs::new().with_file(&a, "alpha\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(a.clone()), "/proj", Config::empty());
    app.buffer.set_text("EDITED IN PLACE\n");
    app.buffer.set_cursor(2);
    app.load_path(a.clone());
    assert_eq!(app.buffer.text(), "EDITED IN PLACE\n");
    assert_eq!(app.buffer.cursor_char(), 2);
    assert_eq!(app.open_buffer_count(), 1, "no phantom second entry");
}

#[test]
fn load_path_recognizes_the_same_file_under_a_differently_spelled_path() {
    // REGRESSION (code review): the registry's identity must be blind to
    // which textual spelling of the same file produced the path — e.g. a
    // CLI file argument typed with no directory component (`cd project &&
    // awl a.txt`, staying relative) vs. that same file's later ROOT-JOINED
    // spelling (`index::resolve`, always absolute — every Goto candidate).
    // Reproduced here with a `.` path component (lexically different, same
    // file) so the fix is proven at the live-App layer, not just headless.
    use crate::fs::InMemoryFs;
    let messy = PathBuf::from("/proj/./a.txt");
    let clean = PathBuf::from("/proj/a.txt");
    let b = PathBuf::from("/proj/b.txt");
    let mem = InMemoryFs::new().with_file(&b, "beta\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(messy.clone()), "/proj", Config::empty());
    app.buffer.set_text("ALPHA EDITED\n");
    assert_eq!(app.open_buffer_count(), 1);

    app.load_path(b.clone());
    assert_eq!(app.open_buffer_count(), 2, "the messy-spelled A is backgrounded");

    app.load_path(clean.clone());
    assert_eq!(
        app.buffer.text(),
        "ALPHA EDITED\n",
        "the CLEAN path found A's live entry (parked under the MESSY spelling) instead of \
         opening a fresh, orphaned copy"
    );
    assert_eq!(
        app.open_buffer_count(),
        2,
        "no orphaned duplicate entry left behind for the messy spelling"
    );
}

#[test]
fn load_path_opens_a_relative_launch_path_then_finds_it_again_via_absolute_path() {
    // REGRESSION (code review, scenario a — the report's EXACT live shape):
    // `cd project && awl a.txt` leaves the launch file argument RELATIVE;
    // reopening the SAME file via its absolute spelling (what a Go-to-file
    // picker candidate always is — `index::resolve` root-joins) must find
    // the SAME live buffer, not silently re-read disk and orphan the
    // relative spelling's dirty entry forever. Needs a REAL chdir (not
    // InMemoryFs, which has no cwd concept) against a real temp dir — hold
    // both the fs TEST_LOCK (real-disk reads race a sibling's InMemoryFs
    // swap) and the CWD_LOCK (chdir is process-global too).
    let _fs = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl-relabs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "alpha\n").unwrap();
    let _cwd = crate::fs::CwdGuard::enter(&dir);

    // This test's `App::new` runs against the REAL native FS (it can't use
    // InMemoryFs, per the chdir note above) — so it must explicitly kill
    // SESSION RESTORE, or `apply_session_restore` reads the developer's
    // ACTUAL `~/.local/share/awl/session.toml` and parks every real
    // buffer it names into the registry, inflating `open_buffer_count()`
    // by however many files happen to be open in a live awl session on
    // this machine right now (an environment-coupled failure, not a
    // random flake — see the investigation note in git history).
    let cfg = Config { session_restore: Some(false), ..Config::empty() };
    // The launch argument stays exactly as typed: relative, no directory.
    let mut app = App::new(Some(PathBuf::from("a.txt")), dir.clone(), None, None, cfg);
    app.buffer.set_text("ALPHA EDITED\n");
    app.buffer.set_cursor(3);
    assert_eq!(app.open_buffer_count(), 1, "only the relative-spelled A is open so far");

    // Reopen via the ABSOLUTE spelling.
    app.load_path(dir.join("a.txt"));
    assert_eq!(
        app.buffer.text(),
        "ALPHA EDITED\n",
        "the live edit survived — the absolute spelling found the SAME buffer, not a fresh \
         disk read"
    );
    assert_eq!(app.buffer.cursor_char(), 3, "the cursor position survived too");
    assert_eq!(
        app.open_buffer_count(),
        1,
        "one entry, not two — the relative and absolute spellings key identically"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn switching_buffers_isolates_the_view_text_cache() {
    // THE CACHE-KEY-DISCIPLINE bug class (CLAUDE.md): every swapped-in buffer
    // restarts its edit version at 0, so `view_text`'s version-keyed
    // rope-clone cache MUST travel with its own buffer (not collide with
    // another buffer sitting at the same version) across a three-way swap.
    use crate::fs::InMemoryFs;
    let a = PathBuf::from("/proj/a.txt");
    let b = PathBuf::from("/proj/b.txt");
    let mem = InMemoryFs::new().with_file(&a, "aaa\n").with_file(&b, "bbb\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(a.clone()), "/proj", Config::empty());
    assert_eq!(app.view_text(), "aaa\n");
    app.load_path(b.clone());
    assert_eq!(app.view_text(), "bbb\n", "B's text must not collide with A's stale version-0 cache");
    app.load_path(a.clone());
    assert_eq!(app.view_text(), "aaa\n", "A's OWN cache is restored, not B's");
}

#[test]
fn switching_away_from_a_dirty_file_still_autosaves() {
    // Item 4 of the spec: the existing autosave flush-on-FILE-SWITCH hook
    // (`App::autosave_flush`, the one door) must still fire on a registry
    // switch, exactly as it did on the old single-buffer swap.
    use crate::fs::{FileSystem, InMemoryFs};
    let a = PathBuf::from("/proj/a.txt");
    let b = PathBuf::from("/proj/b.txt");
    let mem = InMemoryFs::new().with_file(&a, "aaa\n").with_file(&b, "bbb\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(a.clone()), "/proj", Config::empty());
    app.buffer.set_text("aaa EDITED\n");
    app.load_path(b.clone());
    assert_eq!(
        mem.read_to_string(&a).unwrap(),
        "aaa EDITED\n",
        "leaving a dirty pathed buffer autosaves it on switch"
    );
}

#[test]
fn new_note_parks_the_previous_buffer_for_a_later_reopen() {
    use crate::fs::InMemoryFs;
    let a = PathBuf::from("/proj/a.txt");
    let mem = InMemoryFs::new().with_file(&a, "aaa\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(a.clone()), "/proj", Config::empty());
    app.buffer.set_text("aaa EDITED\n");
    assert_eq!(app.open_buffer_count(), 1);
    app.new_note();
    assert_eq!(app.open_buffer_count(), 2, "A is parked; the fresh note is active");
    assert_eq!(app.buffer.text(), "", "the new note starts blank");
    app.load_path(a.clone());
    assert_eq!(app.buffer.text(), "aaa EDITED\n", "A's edit survived being backgrounded by C-x n");
}

#[test]
fn registry_cap_evicts_the_lru_clean_buffer_not_a_dirty_one() {
    // Integration proof that `App` is really wired to
    // `crate::buffers::MAX_OPEN_BUFFERS` (the algorithm itself is exhaustively
    // unit-tested in `buffers.rs`): opening one more CLEAN file than the cap
    // allows evicts the oldest clean background entry, so re-opening THAT one
    // reads fresh from disk (its edits, if any, would be gone — here it has
    // none, so we assert the fresh disk content lands, and via the "clean" law
    // that a DIRTY one earlier in the queue is never touched).
    use crate::fs::InMemoryFs;
    let mut mem = InMemoryFs::new();
    for i in 0..crate::buffers::MAX_OPEN_BUFFERS {
        mem = mem.with_file(format!("/proj/f{i}.txt"), "clean\n");
    }
    mem = mem.with_file("/proj/dirty.txt", "will-be-edited\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(PathBuf::from("/proj/dirty.txt")), "/proj", Config::empty());
    app.buffer.set_text("EDITED, NEVER EVICT ME\n");
    // Open every clean file in turn (backgrounding the dirty one first, then
    // each clean one), pushing the registry to (and one past) the cap.
    for i in 0..crate::buffers::MAX_OPEN_BUFFERS {
        app.load_path(PathBuf::from(format!("/proj/f{i}.txt")));
    }
    // The registry now holds MAX_OPEN_BUFFERS backgrounded entries (dirty.txt
    // + f0..f(N-2)) capped by evicting the LRU CLEAN one (f0) — never dirty.txt.
    assert_eq!(app.open_buffer_count(), crate::buffers::MAX_OPEN_BUFFERS, "capped, not unbounded");
    app.load_path(PathBuf::from("/proj/dirty.txt"));
    assert_eq!(
        app.buffer.text(),
        "EDITED, NEVER EVICT ME\n",
        "the dirty buffer survived the whole cap-pressure run"
    );
}

#[test]
fn right_click_word_summons_spell_suggestions() {
    // The right-click path = place the cursor at the clicked word (the GPU
    // hit-test, untestable headlessly), then run the EXISTING OpenSpellSuggest
    // seam at that cursor. This locks the REUSED contract WITHOUT a window: a
    // cursor on a misspelling yields a target with corrections (so the picker
    // summons + builds a Spell overlay), while a correct word yields None — the
    // calm no-op the binding promises. Skipped if the bundled dictionary is absent.
    let Ok(sc) = crate::spell::SpellChecker::new(crate::spell::DictVariant::EnUs) else {
        return;
    };
    let mut buffer = Buffer::from_str("Please recieve this.\n");
    // Simulate the click landing inside the misspelling "recieve".
    let idx = buffer.line_col_to_char(0, 9);
    buffer.set_cursor(idx);
    let (line, col) = buffer.cursor_line_col();
    let t = sc
        .suggest_at(&buffer.text(), line, col, buffer.syntax_lang())
        .expect("a misspelled word under the right-click yields a target");
    assert!(t.suggestions.iter().any(|w| w == "receive"));
    // What `apply(OpenSpellSuggest)` builds from that target: a Spell picker.
    let ov = crate::overlay::OverlayState::new_spell(
        t.suggestions.clone(),
        (t.misspelling.line, t.misspelling.start_col, t.misspelling.end_col),
    );
    assert_eq!(ov.kind, crate::overlay::OverlayKind::Spell);
    // A right-click on a CORRECTLY-spelled word ("Please") is a calm no-op.
    let ok_idx = buffer.line_col_to_char(0, 2);
    buffer.set_cursor(ok_idx);
    let (l, c) = buffer.cursor_line_col();
    assert!(sc.suggest_at(&buffer.text(), l, c, buffer.syntax_lang()).is_none(), "correct word: no summon");
}

// ── HERMETICITY STRUCTURAL GUARD ────────────────────────────────────────
//
// Rust's privacy model can express "visible to production plus every
// descendant module" (what a private `fn new` already gets — every test
// submodule under `app/` is a descendant of `app`, so it already sees the
// raw constructor) but NOT "visible to production plus this ONE helper
// function's own body" — there is no `pub(in path)` spelling that grants
// access to `new_hermetic`'s definition while denying every sibling test
// module. So the raw constructor's door can't be sealed at compile time
// without also blocking the small set of tests that deliberately need
// the REAL disk (see `App::new_hermetic`'s own doc for that list). This
// is the honest fallback: a SOURCE-SCAN law test, in the same spirit as
// `rowlayout.rs`'s / `theme/`'s no-wildcard enumerations — a structural
// fact asserted at test time, cheap to keep honest because the count it
// guards is small and curated, not a general-purpose linter.
//
// NOTE ON THE NEEDLE: the pattern this scan looks for is built at RUNTIME
// (`app_new_needle`, four separate literals concatenated) rather than
// spelled out as one contiguous string anywhere in this file — otherwise
// this very guard's own source text would match itself and inflate its
// own count. Keep every comment/message below phrased without writing
// the raw constructor's name directly followed by an open paren.
//
// Exact per-file occurrence counts of the needle across the whole crate.
// Every entry below is individually accounted for (see each call site's
// own inline comment): either the ONE real production call, a real-disk
// test that explicitly disables `session_restore` (can't use
// `new_hermetic` because it needs `Buffer::from_file` to see genuine
// bytes), or a test already wrapped in `fs::with_fs`/`FsGuard::install`
// with a controlled fake `InMemoryFs` (hermetic by construction,
// independent of `session_restore`'s value — `app/session.rs`'s own
// tests, which specifically exercise session restore, cannot use
// `new_hermetic` at all since it forces `session_restore: Some(false)`).
// A test that only needs a plain, don't-care-about-disk `App` must go
// through `App::new_hermetic` instead, which never contributes to this
// count at all (its name has an extra `_hermetic` between `new` and the
// open paren, so it never matches the needle).
//
// Adding a NEW raw call anywhere — including a new file — fails this
// test until the count below is consciously updated, which forces the
// same two-way choice every existing site already made.
#[test]
fn real_fs_app_new_calls_are_all_accounted_for() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    scan_dir_for_app_new(&root, &root, &mut counts);

    let expected: &[(&str, usize)] = &[
        // 1 production call (in `crate::app::run`), the ONLY raw `App::new`
        // left in `app.rs` proper now the test module lives in its own file.
        ("app.rs", 1),
        // The 4 test-side raw calls, moved here with the test module (the
        // mechanical decomposition round): 2 real-disk tests with
        // `session_restore` disabled inline + 1 real-disk chdir test (same
        // treatment) + the `app_on` helper (every one of its callers installs
        // its own fake FS first — this file's
        // `app_on`-callers-all-install-a-fake-fs discipline).
        ("app/tests.rs", 4),
        // 1 real-disk test (`finish_buffer_saves_...`), session_restore
        // disabled inline.
        ("app/daemon.rs", 1),
        // 5 calls, every one inside a `crate::fs::with_fs(fake, || ..)`
        // closure seeded with its own `InMemoryFs` — these tests exist
        // specifically to prove what `apply_session_restore` reads back,
        // so they can't use a constructor that forces it off.
        ("app/session.rs", 5),
        // 3 store tests (2 recent-projects + 1 recent-files), each inside its
        // own `fs::with_fs(fake, ..)` closure seeded with an `InMemoryFs` — they
        // exist specifically to prove what `App::switch_project` / `App::load_path`
        // / `App::new` write to and read back from the recent-projects /
        // recent-files stores, so they need to CONTROL + INSPECT the injected fs
        // (which `new_hermetic`'s private internal fs hides), never real disk.
        // Same treatment as `app/session.rs` above. Plus 3 NO-PATH-PASTE-SAVES-
        // FIRST tests (`ensure_note_named_before_paste_*`), each also inside its
        // own `fs::with_fs(fake, ..)` closure with an `InMemoryFs` handle kept by
        // the test — they exist specifically to prove what
        // `App::ensure_note_named_before_paste` writes to disk (the promoted
        // note's derived path + its saved bytes), so they need the same
        // CONTROL + INSPECT access `new_hermetic` hides. Same treatment. Plus 1
        // CJK-priority persist test
        // (`persist_cjk_priority_writes_the_whole_ordered_ladder_to_config`),
        // inside its own `fs::with_fs(fake, ..)` closure with an `InMemoryFs`
        // handle — proves what `App::persist_cjk_priority` writes to
        // `config.path` on disk, same CONTROL + INSPECT need. Plus 2
        // SPELLCHECK x CONFIG-RELOAD tests (the spell-toggle-x-theme
        // investigation, 2026-07-18:
        // `reload_config_absent_spellcheck_key_leaves_global_untouched` +
        // `reload_config_reapplies_a_persisted_spellcheck_value_immediately`),
        // each inside its own `fs::with_fs(fake, ..)` closure with an
        // `InMemoryFs` handle — they exist specifically to prove what
        // `App::reload_config` reads back from `config.path` on disk (and,
        // for the absent-key case, that it must NOT force a default), same
        // CONTROL + INSPECT need `new_hermetic` hides.
        ("app/files.rs", 9),
        // 9 LIFETIME STATS + USAGE LEDGER + DISCOVERABILITY tests, each inside its own
        // `fs::with_fs(fake, ..)` closure seeded with an `InMemoryFs` — they exist
        // specifically to prove what the tracking hooks / the ledger's
        // `ledger_note_dispatch` + `stats_flush` write to and read back from
        // `stats.toml`, so they need to CONTROL + INSPECT the injected fs (which
        // `new_hermetic`'s private internal fs hides). Same treatment as
        // `app/session.rs` / `app/files.rs` above. (The 3 added by the ledger:
        // door-attribution round-trip, graduation-candidate ranking, kill-switch;
        // the 2 added by the discoverability round: peek/footer ranking from a fake
        // ledger, and the fresh-ledger-empty case.)
        ("app/stats.rs", 9),
        // 6 WRITING STREAKS tests, each inside its own `fs::with_fs(fake, ..)`
        // closure seeded with an `InMemoryFs` — they exist specifically to prove
        // what `streaks_flush` writes to / reads back from `streaks.toml` (and
        // that the kill switch never writes), so they need to CONTROL + INSPECT
        // the injected fs (which `new_hermetic`'s private fs hides). Same
        // treatment as `app/stats.rs` above. `new_hermetic` also won't do here:
        // it restores the real backend on construction return, but these tests
        // keep driving the fs AFTER construction (`new_note`, the summon flush),
        // so the fake must stay active across the whole closure. (The 3 added by
        // the anchor-swallow fix: fresh-note + fresh-scratch record words typed
        // before the first flush, and the card-summon-freshness flush.)
        ("app/streaks.rs", 6),
        // input.rs's click tests all moved onto `App::new_hermetic` —
        // zero raw calls left.
    ];
    let mut expected_map: std::collections::BTreeMap<String, usize> =
        expected.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    // Any file not listed above must have ZERO occurrences.
    for (file, count) in &counts {
        let want = expected_map.remove(file).unwrap_or(0);
        assert_eq!(
            *count, want,
            "unexpected raw-constructor count in {file}: found {count}, expected {want} — \
             either route the new call through App::new_hermetic, or (if it genuinely needs \
             real disk) disable session_restore inline / wrap it in fs::with_fs and update \
             this test's expected count with a comment explaining why"
        );
    }
    for (file, want) in expected_map {
        assert_eq!(0, want, "expected {want} raw-constructor call(s) in {file} but found none — did it move to new_hermetic or a different file?");
    }

    // The ONE production call site must still exist exactly once, naming
    // its real argument list (guards against the count staying right by
    // coincidence while the actual production call moved or was deleted).
    let mut production_hits = 0usize;
    count_substr_in_dir(&root, &production_call_needle(), &mut production_hits);
    assert_eq!(production_hits, 1, "the production App::new call in crate::app::run must exist exactly once");
}

/// Built from separate literals at runtime — see the module-doc note
/// above the guard test for why this can't be one contiguous literal.
#[cfg(test)]
fn app_new_needle() -> String {
    ["App", "::", "new", "("].concat()
}

#[cfg(test)]
fn production_call_needle() -> String {
    format!("{}file, root, cli_workspace, cli_notes_root, config);", app_new_needle())
}

#[cfg(test)]
fn scan_dir_for_app_new(
    base: &std::path::Path,
    dir: &std::path::Path,
    counts: &mut std::collections::BTreeMap<String, usize>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let needle = app_new_needle();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir_for_app_new(base, &path, counts);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let n = text.matches(&needle).count();
        if n == 0 {
            continue;
        }
        let rel = path
            .strip_prefix(base)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        counts.insert(rel, n);
    }
}

#[cfg(test)]
fn count_substr_in_dir(dir: &std::path::Path, needle: &str, total: &mut usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            count_substr_in_dir(&path, needle, total);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        *total += text.matches(needle).count();
    }
}

// ── VIRTUAL-CLOCK FRAME LOOP: the multi-frame scheduling law ──────────────────
// The seam the `--screenshot-frames` capture rests on: a real `App`'s ACTUAL
// `about_to_wait_impl` scheduling body, stepped frame-by-frame under a deterministic
// `VirtualClock` (no winit event loop — a `RecordingScheduler` is the control-flow
// sink) so a LIVE-ONLY cross-frame behaviour can be asserted. The demonstrated one
// is the WHICH-KEY debounce: after a `C-x` prefix is armed at t=0, the continuation
// panel must summon EXACTLY at its `whichkey::PAUSE` deadline step and NEVER before —
// a false→true flip that a single settled `--screenshot` frame cannot express (it
// renders one instant, so it can show the panel up or down but never the TRANSITION,
// nor prove the transition happens at the right time and not one step early).

/// Run the which-key prefix-pause scenario `frames` steps at `step_ms` per frame,
/// returning `(elapsed_ms, whichkey_shown, wait_scheduled)` per frame — the exact
/// state the `--screenshot-frames` capture records. Drives the REAL scheduling body.
#[cfg(test)]
fn run_whichkey_frame_loop(frames: u32, step_ms: u64) -> Vec<(u64, bool, bool)> {
    let _serial = crate::testlock::serial();
    let clock = crate::clock::VirtualClock::new();
    let mut app = App::new_hermetic(None, std::path::PathBuf::from("/"), Config::empty());
    app.set_clock(Box::new(clock.clone()));
    // Arm the prefix at virtual t=0 (the `PrefixTransition::Arm` edge).
    app.arm_whichkey_prefix();

    let sched = RecordingScheduler::new();
    let mut out = Vec::new();
    for i in 0..frames {
        clock.advance_ms(step_ms);
        sched.begin_step();
        app.step_scheduling(&sched);
        let wait_scheduled = matches!(
            sched.scheduled_this_step(),
            Some(ControlFlow::WaitUntil(_))
        );
        out.push(((i as u64 + 1) * step_ms, app.whichkey_is_shown(), wait_scheduled));
    }
    out
}

#[test]
fn whichkey_debounce_summons_exactly_at_its_pause_deadline_step() {
    // 100 ms steps land the 500 ms PAUSE crisply on a frame boundary: elapsed at
    // frame i is (i+1)*100, so frames 0..=3 (100..=400 ms) are still pending and
    // frame 4 (500 ms) is the first summoned frame.
    let pause_ms = crate::whichkey::PAUSE.as_millis() as u64;
    let frames = run_whichkey_frame_loop(8, 100);

    let mut flips = 0usize;
    let mut prev_shown = false;
    for (elapsed_ms, shown, wait_scheduled) in &frames {
        // The debounce fires EXACTLY at the deadline: shown iff virtual time has
        // reached the pause — never one step early, never one step late.
        assert_eq!(
            *shown,
            *elapsed_ms >= pause_ms,
            "which-key panel shown={shown} at t={elapsed_ms}ms but the {pause_ms}ms \
             deadline says it should be {}",
            *elapsed_ms >= pause_ms
        );
        // A single false→true flip: the panel appears once and stays (no flicker).
        if *shown && !prev_shown {
            flips += 1;
        }
        assert!(!(prev_shown && !*shown), "the panel must never un-summon mid-run");
        prev_shown = *shown;

        // REDRAW-SCHEDULING law (a WaitUntil is the winit "wake me at the deadline"):
        // armed EXACTLY while the pause is still pending (not yet elapsed), and NOT
        // once the panel is summoned — the loop must fall quiet, never busy-wait.
        assert_eq!(
            *wait_scheduled,
            *elapsed_ms < pause_ms,
            "WaitUntil armed={wait_scheduled} at t={elapsed_ms}ms; it must be armed \
             only while the pause is pending (t < {pause_ms}ms)"
        );
    }
    assert_eq!(flips, 1, "the panel must flip down→up exactly once across the frames");
    // Sanity: the run actually straddled the deadline (a pre- and a post-summon frame).
    assert!(!frames.first().unwrap().1, "frame 0 (t=100ms) must be pre-summon");
    assert!(frames.last().unwrap().1, "the last frame (t=800ms) must be summoned");
}

#[test]
fn whichkey_debounce_does_not_summon_a_step_before_its_deadline() {
    // 150 ms steps: elapsed 150, 300, 450, 600, … The 450 ms frame is BELOW the
    // 500 ms pause and must stay pending; the 600 ms frame is the first summoned —
    // pinning "not before the deadline" independent of a boundary-aligned step.
    let frames = run_whichkey_frame_loop(5, 150);
    assert_eq!(frames[0], (150, false, true));
    assert_eq!(frames[1], (300, false, true));
    assert_eq!(frames[2], (450, false, true), "still pending one step before 500ms");
    assert_eq!(frames[3], (600, true, false), "summoned the first frame past 500ms");
    assert_eq!(frames[4], (750, true, false), "and stays summoned, loop quiet");
}

#[test]
fn virtual_clock_frame_loop_is_deterministic_across_runs() {
    // The whole point of the injected clock: two runs of the same scenario produce
    // identical per-frame state (the base Instant differs but cancels out of every
    // delta), so the `--screenshot-frames` artifacts are byte-stable.
    let a = run_whichkey_frame_loop(8, 100);
    let b = run_whichkey_frame_loop(8, 100);
    assert_eq!(a, b, "the virtual-clock frame loop must be run-to-run deterministic");
}
