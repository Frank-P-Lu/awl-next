use super::*;

#[test]
fn gpu_fault_outcomes_skip_once_then_rebuild_without_conflating_validation() {
    assert_eq!(
        gpu_fault_action(
            GpuLifecycle::Active { oom_skips: 0 },
            gpu::GpuFaultKind::OutOfMemory
        ),
        GpuFaultAction::RetryOneFrame,
    );
    assert_eq!(
        gpu_fault_action(
            GpuLifecycle::Active { oom_skips: 1 },
            gpu::GpuFaultKind::OutOfMemory
        ),
        GpuFaultAction::Rebuild,
    );
    for kind in [
        gpu::GpuFaultKind::DeviceLost,
        gpu::GpuFaultKind::Internal,
        gpu::GpuFaultKind::SurfaceRecoveryFailed,
    ] {
        assert_eq!(
            gpu_fault_action(GpuLifecycle::Active { oom_skips: 0 }, kind),
            GpuFaultAction::Rebuild
        );
    }
    assert_eq!(
        gpu_fault_action(
            GpuLifecycle::Active { oom_skips: 0 },
            gpu::GpuFaultKind::Validation
        ),
        GpuFaultAction::NoticeOnly,
    );
}

#[test]
fn every_degraded_frame_names_its_retry_or_hold_outcome() {
    assert_eq!(
        gpu_skip_action(gpu::GpuFrameSkip::Occluded, 0),
        GpuSkipAction::WaitForWake
    );
    assert_eq!(
        gpu_skip_action(gpu::GpuFrameSkip::Timeout, 0),
        GpuSkipAction::RetryAfter(Duration::from_millis(16))
    );
    assert_eq!(
        gpu_skip_action(gpu::GpuFrameSkip::Timeout, 1),
        GpuSkipAction::RetryAfter(Duration::from_millis(32))
    );
    assert_eq!(
        gpu_skip_action(gpu::GpuFrameSkip::Timeout, 20),
        GpuSkipAction::RetryAfter(Duration::from_millis(512))
    );
    assert_eq!(
        gpu_skip_action(gpu::GpuFrameSkip::SurfaceReconfigured, 0),
        GpuSkipAction::RetryAfter(GPU_SURFACE_RETRY)
    );
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
    assert!(debounce_due(
        dirty,
        win,
        dirty + win + Duration::from_millis(1)
    ));
}

#[test]
fn present_transaction_sync_composes_over_every_source() {
    // The ONE-owner composition `App::sync_present_txn` applies: armed while
    // ANY source needs it (resize stream, move stream, or a theme-preview
    // lava-boundary crossing), off only when ALL three are idle.
    assert!(
        !present_sync_armed(false, false, false),
        "idle: async presents"
    );
    assert!(
        present_sync_armed(true, false, false),
        "live resize arms it"
    );
    assert!(present_sync_armed(false, true, false), "live move arms it");
    assert!(
        present_sync_armed(false, false, true),
        "a preview crossing arms it"
    );
    assert!(
        present_sync_armed(true, true, false),
        "corner drag: both streams live"
    );
    assert!(
        present_sync_armed(true, false, true),
        "resize + crossing overlap"
    );
    assert!(
        present_sync_armed(false, true, true),
        "move + crossing overlap"
    );
    assert!(present_sync_armed(true, true, true), "all three at once");
}

/// A GPU rebuild installs a new CAMetalLayer whose transaction flag starts as
/// unknown to the App. Even if the desired bit equals the old layer's shadow,
/// the one owner must treat that shadow as invalid and apply it to the new
/// layer before allowing the equality fast-path again.
#[test]
fn gpu_replacement_invalidates_and_reestablishes_the_present_sync_shadow() {
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());

    app.crossing_settle_at = Some(Instant::now());
    app.sync_present_txn();
    assert!(app.present_sync_on);
    assert!(app.present_sync_valid);

    // This is the state at the replacement seam in `on_gpu_ready`: the old
    // value remains useful as state, but cannot describe the fresh layer.
    app.present_sync_valid = false;
    app.sync_present_txn();
    assert!(app.present_sync_on, "the live crossing claim is preserved");
    assert!(
        app.present_sync_valid,
        "the current layer's shadow is established before equality may elide work"
    );

    app.sync_present_txn();
    assert!(app.present_sync_valid, "steady-state sync stays idempotent");
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
    let w = |name: &str| {
        *crate::theme::THEMES
            .iter()
            .find(|t| t.name == name)
            .unwrap()
    };
    let (mangrove, galah) = (w("Mangrove"), w("Galah"));

    crate::theme::set_active_by_name("Mangrove").unwrap();
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    assert!(!app.present_sync_on, "idle: presents run async");

    // (1) The STEADY steps the retired classifier left unbracketed now arm:
    // `Galah→Magpie` (static→static — the real reported LANDING) and
    // `Mangrove→Firetail` (lava→lava). Each starts from a disarmed state.
    for (from, to, label) in [
        (galah, "Magpie", "static->static LANDING"),
        (mangrove, "Firetail", "lava->lava"),
    ] {
        app.crossing_settle_at = None;
        app.crossing_teardown_pending = false;
        app.sync_present_txn();
        assert!(!app.present_sync_on, "{label}: starts disarmed");
        crate::theme::set_active_by_name(to).unwrap();
        app.retint_theme_preview(from);
        assert!(
            app.crossing_settle_at.is_some(),
            "{label}: the preview stamps the settle"
        );
        assert!(
            app.present_sync_on,
            "{label}: the bracket arms unconditionally"
        );
    }

    // (2) EVENT-ORDERED TEARDOWN. Phase 1 (settle) clears the debounce but
    // HOLDS the bracket via the pending teardown — it must NOT disarm here,
    // because the deferred reshape's present has not happened yet.
    app.finish_crossing_settle();
    assert!(
        app.crossing_settle_at.is_none(),
        "phase 1 clears the settle debounce"
    );
    assert!(
        app.crossing_teardown_pending,
        "phase 1 hands off to the pending teardown"
    );
    assert!(
        app.present_sync_on,
        "the bracket is HELD through the reshape present, not torn down"
    );
    // Phase 2 (the post-present hook, after the in-bracket reshape present)
    // is the ONLY thing that disarms.
    app.finish_crossing_teardown();
    assert!(
        !app.crossing_teardown_pending,
        "phase 2 clears the pending teardown"
    );
    assert!(
        !app.present_sync_on,
        "only after the bracketed reshape present does the bracket disarm"
    );
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
    assert!(
        app.present_sync_on,
        "resize still owns a claim after phase 1"
    );
    app.finish_crossing_teardown();
    assert!(!app.crossing_teardown_pending);
    assert!(
        app.present_sync_on,
        "resize still owns a claim after phase 2"
    );
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
    assert!(!notice_expired(
        NoticeKind::Sticky,
        Some(deadline),
        deadline
    ));
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
    assert!(!hud_mods_broken(
        ModifiersState::empty(),
        ModifiersState::empty()
    ));
    assert!(!hud_mods_broken(
        ModifiersState::empty(),
        ModifiersState::SUPER
    ));
}

#[test]
fn buffer_endpoints_shift_intent_keys_on_the_chord_not_the_action() {
    use winit::keyboard::NamedKey;
    // The chord-aware rule: BufferStart/BufferEnd carry INCIDENTAL Shift only
    // for `M-<` / `M->`, whose Shift is needed just to TYPE the `<` / `>`
    // glyph (a `Key::Character`) — those stay pure motion and must NOT extend.
    assert!(!motion_honors_shift_select(
        &Action::BufferStart,
        &Key::Character("<".into())
    ));
    assert!(!motion_honors_shift_select(
        &Action::BufferEnd,
        &Key::Character(">".into())
    ));
    // But the SAME actions reached through a NAMED navigation key carry a
    // genuine GUI select-intent Shift and MUST extend: Shift+Cmd-Up/Down
    // (macOS) and Shift+Ctrl-Home/End (Linux) select to document start/end,
    // exactly like every platform text field (the live bug this round fixed).
    assert!(motion_honors_shift_select(
        &Action::BufferStart,
        &Key::Named(NamedKey::ArrowUp)
    ));
    assert!(motion_honors_shift_select(
        &Action::BufferEnd,
        &Key::Named(NamedKey::ArrowDown)
    ));
    assert!(motion_honors_shift_select(
        &Action::BufferStart,
        &Key::Named(NamedKey::Home)
    ));
    assert!(motion_honors_shift_select(
        &Action::BufferEnd,
        &Key::Named(NamedKey::End)
    ));
    // Every other motion keeps Shift's normal select-extend meaning regardless
    // of key shape (the user deliberately held Shift, e.g. Shift+Arrow / M-Shift-f).
    assert!(motion_honors_shift_select(
        &Action::ForwardChar,
        &Key::Named(NamedKey::ArrowRight)
    ));
    assert!(motion_honors_shift_select(
        &Action::ForwardChar,
        &Key::Character("f".into())
    ));
    assert!(motion_honors_shift_select(
        &Action::ForwardWord,
        &Key::Character("f".into())
    ));
    assert!(motion_honors_shift_select(
        &Action::NextLine,
        &Key::Named(NamedKey::ArrowDown)
    ));
    assert!(motion_honors_shift_select(
        &Action::LineEnd,
        &Key::Named(NamedKey::End)
    ));
    // Non-motions are unaffected (Shift is ignored by the motion-select logic
    // for them anyway), so they report the default true.
    assert!(motion_honors_shift_select(
        &Action::InsertChar('a'),
        &Key::Character("A".into())
    ));
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
        let mut km =
            crate::keymap::KeymapState::new_with_convention(crate::convention::Convention::Mac);
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
    assert_eq!(
        up,
        Some(((0, 0), (1, 3))),
        "Shift+Cmd-Up selects to document start"
    );
    // From the same spot, Shift+Cmd-Down extends down to the doc end — the
    // empty final line after the trailing newline, (3,0).
    let down = drive_live(Key::Named(NamedKey::ArrowDown), sup_shift, TXT, 14);
    assert_eq!(
        down,
        Some(((1, 3), (3, 0))),
        "Shift+Cmd-Down selects to document end"
    );
    // `M-<` (Character `<`, Shift incidental) moves but never extends.
    let emacs = drive_live(Key::Character("<".into()), meta_shift, TXT, 14);
    assert_eq!(
        emacs, None,
        "M-< incidental Shift stays pure motion (no selection)"
    );
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
    assert_eq!(
        app.bump_click_count(),
        1,
        "a first press starts a fresh count"
    );
    assert_eq!(
        app.bump_click_count(),
        2,
        "an immediate same-spot press doubles it"
    );
    assert_eq!(
        app.bump_click_count(),
        3,
        "a third same-spot press triples it"
    );
    assert_eq!(
        app.bump_click_count(),
        1,
        "a fourth wraps back to a fresh single click"
    );
    // A press at a DIFFERENT spot never continues the run, however fast.
    app.bump_click_count();
    app.cursor_px = (500.0, 500.0);
    assert_eq!(
        app.bump_click_count(),
        1,
        "a different spot starts over, not a double-click"
    );
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
    let cfg = Config {
        session_restore: Some(false),
        ..Config::empty()
    };
    let mut app = App::new(Some(old), dir.clone(), None, None, cfg);
    // The first sync caches (version 0, old text) — the short-circuit at work.
    assert_eq!(app.view_text(), "the OLD document\n");
    assert_eq!(
        app.buffer.version(),
        0,
        "an un-edited buffer sits at version 0"
    );
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
    let cfg = Config {
        session_restore: Some(false),
        ..Config::empty()
    };
    let mut app = App::new(Some(file), dir.clone(), None, Some(notes), cfg);
    assert_eq!(app.view_text(), "prior document\n");
    app.new_note();
    assert_eq!(app.view_text(), "", "the fresh note starts blank on screen");
    let _ = std::fs::remove_dir_all(&dir);
}
