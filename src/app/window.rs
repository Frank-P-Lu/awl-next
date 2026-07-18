//! WINDOW-LIFECYCLE + redraw arm bodies lifted out of `App::window_event` (now a
//! thin dispatcher). These are the arms that are NOT input: focus-lost flush,
//! physical resize, DPI (scale-factor) refold, and the redraw-request frame
//! loop (caret spring advance + present + the debug-panel perf feed). Lifted
//! verbatim — each method IS one former `match` arm, so the ORDER and behaviour
//! (including the redraw arm's control-flow decisions) are unchanged. The input
//! arms live in `app/input/`.

use super::*;

impl App {
    fn handle_gpu_fault(&mut self, event_loop: &ActiveEventLoop, fault: gpu::GpuFault) {
        eprintln!("gpu {:?}: {}", fault.kind, fault.message);
        match gpu_fault_action(self.gpu_lifecycle, fault.kind) {
            GpuFaultAction::RetryOneFrame => {
                self.gpu_lifecycle = GpuLifecycle::Active { oom_skips: 1 };
                self.set_sticky_notice("graphics memory pressure — skipped one frame");
                if let Some(gpu) = self.gpu.as_ref() { gpu.window.request_redraw(); }
            }
            GpuFaultAction::NoticeOnly => self.set_sticky_notice("graphics rejected one frame — editing is safe"),
            GpuFaultAction::Rebuild => {
                let reason = match fault.kind {
                    gpu::GpuFaultKind::OutOfMemory => "graphics memory stayed full",
                    gpu::GpuFaultKind::DeviceLost => "graphics device was lost",
                    gpu::GpuFaultKind::Internal => "graphics backend stopped responding",
                    gpu::GpuFaultKind::SurfaceRecoveryFailed => "window surface could not recover",
                    gpu::GpuFaultKind::Validation => "graphics rejected repeated work",
                };
                self.rebuild_gpu(event_loop, reason);
            }
        }
    }

    fn handle_gpu_frame_outcome(&mut self, event_loop: &ActiveEventLoop, outcome: gpu::GpuFrameOutcome) -> Result<(Option<(f32, Instant)>, bool), ()> {
        match outcome {
            gpu::GpuFrameOutcome::Presented(perf) => {
                self.gpu_lifecycle = GpuLifecycle::Active { oom_skips: 0 };
                self.gpu_retry_at = None;
                self.gpu_timeout_streak = 0;
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(soak) = self.soak.as_mut() { soak.observe_frame(crate::soak_gpu::FrameOutcome::Presented, true); if let Some(kind) = self.soak_recovery_pending.take() { soak.observe_recovered(kind, Instant::now()); } }
                Ok((perf, true))
            }
            gpu::GpuFrameOutcome::Skipped(skip) => {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(soak) = self.soak.as_mut() { soak.observe_frame(crate::soak_gpu::FrameOutcome::Skipped(soak_skip_kind(skip)), false); }
                let action = gpu_skip_action(skip, self.gpu_timeout_streak);
                self.gpu_timeout_streak = if skip == gpu::GpuFrameSkip::Timeout {
                    self.gpu_timeout_streak.saturating_add(1)
                } else {
                    0
                };
                match action {
                    GpuSkipAction::WaitForWake => self.gpu_retry_at = None,
                    GpuSkipAction::RetryAfter(delay) => {
                        self.gpu_retry_at = Some(Instant::now() + delay);
                    }
                    GpuSkipAction::RetryWithNoticeAfter(delay, notice) => {
                        self.set_toast_notice(notice);
                        self.gpu_retry_at = Some(Instant::now() + delay);
                    }
                    GpuSkipAction::HoldWithNotice(notice) => {
                        self.gpu_retry_at = None;
                        self.set_sticky_notice(notice);
                    }
                }
                Ok((None, false))
            }
            gpu::GpuFrameOutcome::Fault(fault) => {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(soak) = self.soak.as_mut() { soak.observe_frame(crate::soak_gpu::FrameOutcome::Skipped(crate::soak_gpu::SkipKind::Fault), false); }
                self.handle_gpu_fault(event_loop, fault);
                Err(())
            }
        }
    }
    /// `WindowEvent::Focused(false)`: the window lost focus. ROBUST AUTOSAVE —
    /// flush a pending note write, the document autosave / scratch stash, and
    /// (native only) the session restore state on the same blur trigger. Also
    /// resets the OS pointer to Visible so a focus change never leaves it hidden
    /// behind another app.
    /// `WindowEvent::Focused(true)`: the window regained focus. RESUME the ambient
    /// lava tick (`crate::lava`): mark focused + clear the tick stamp so
    /// `about_to_wait` re-arms it FRESH (avoiding one huge `dt` catch-up bob from
    /// the blurred gap), and request a redraw so the lamp repaints and the tick
    /// re-arms this turn. Inert for a non-lava world (nothing to resume — the tick
    /// gate stays false), so no extra frame is scheduled there.
    pub(super) fn on_focus_gained(&mut self) {
        self.focused = true;
        self.lava_tick_at = None;
        if let Some(gpu) = self.gpu.as_ref() { gpu.window.request_redraw(); }
    }

    /// `WindowEvent::Occluded`: the window's compositor visibility changed.
    /// When it becomes VISIBLE again (`occluded == false`), request a redraw —
    /// the GPU skip path parked `Occluded → WaitForWake` with no retry timer, so
    /// without this wake an un-occluded window could sit un-repainted until some
    /// unrelated event happened to arrive. Becoming occluded needs no action
    /// (the next acquire returns `Occluded` and re-parks the loop). The decision
    /// is the pure `occluded_change_wants_redraw` so it is unit-testable.
    pub(super) fn on_occluded(&mut self, occluded: bool) {
        if occluded_change_wants_redraw(occluded) {
            if let Some(gpu) = self.gpu.as_ref() { gpu.window.request_redraw(); }
        }
    }

    pub(super) fn on_focus_lost(&mut self) {
        // AMBIENT LAVA TICK: the window lost focus — PAUSE the lava drift (hold the
        // current phase, stop scheduling frames) so a backgrounded window costs 0%
        // CPU. `about_to_wait`'s gate reads `self.focused`; clearing the stamp means
        // a later regain re-arms fresh rather than firing a huge catch-up dt.
        self.focused = false;
        self.lava_tick_at = None;
        // ROBUST AUTOSAVE: the window lost focus (the user switched away);
        // flush a pending note write now so a note is never left unsaved
        // behind another app — and flush the document autosave / scratch
        // stash on the same trigger (locked decision: save on blur).
        self.flush_note();
        self.autosave_flush();
        // SESSION RESTORE: persist the open-file set / active buffer /
        // cursor+scroll / window frame on the SAME blur trigger the
        // autosave engine uses (native only; kill-switch gated inside).
        #[cfg(not(target_arch = "wasm32"))]
        self.session_flush();
        // LIFETIME STATS: persist the odometer on the SAME blur trigger (native
        // only; config-gated + dirty-gated inside).
        #[cfg(not(target_arch = "wasm32"))]
        self.stats_flush();
        // WRITING STREAKS: sample the day-delta on the SAME blur trigger.
        #[cfg(not(target_arch = "wasm32"))]
        self.streaks_flush();
        // HOLD-⌘ SHORTCUT PEEK: the window losing focus breaks the hold — cancel a
        // pending peek / close an open one, so it never lingers behind another app
        // (the macOS focus-loss edge the HUD's own `hud_release_on_mods` covers for
        // the held HUD). Inert unless a peek is pending/open.
        self.feed_peek(crate::peek::PeekStimulus::Interrupt);
        // POINTER AUTO-HIDE: a focus change must never leave the OS
        // pointer hidden behind another app — reset to Visible on blur
        // too, on the same trigger as the autosave flush above.
        let prev_pointer_hide = self.pointer_hide;
        self.pointer_hide = crate::pointer_hide::PointerHide::Visible;
        if let Some(visible) =
            crate::pointer_hide::os_visibility_change(prev_pointer_hide, self.pointer_hide)
        {
            if let Some(gpu) = self.gpu.as_ref() {
                gpu.window.set_cursor_visible(visible);
            }
        }
    }

    /// `WindowEvent::Resized`: resize the surface, re-sync the view (re-wraps the
    /// column at the new physical size), and redraw.
    ///
    /// **SYNCHRONOUS REDRAW ON RESIZE** (macOS live-resize correctness): a bare
    /// `request_redraw()` only QUEUES a `RedrawRequested` for winit's run loop
    /// to deliver later — normally prompt, but during a live-resize drag it
    /// leaves a real gap in which the surface has been reconfigured to the NEW
    /// size while the LAST rendered frame is still the OLD one. Drawing +
    /// presenting the just-reconfigured surface RIGHT HERE, gated to an actual
    /// size change, closes that gap outright rather than depending on the
    /// queued redraw's timing. `gpu.redraw()` alone never touches the caret-
    /// spring / debug-panel bookkeeping that lives in `on_redraw_requested`
    /// (spring advance, `redraw_count`, key→px stamping) — it is a pure,
    /// idempotent "draw what's already prepared, right now", so calling it
    /// here is safe alongside the unchanged trailing `request_redraw()` below,
    /// which still keeps that bookkeeping on its normal cadence.
    ///
    /// This alone does not fully cure a FAST drag, though: even a synchronous
    /// present here can still lose a race against AppKit's own resize-tracking
    /// Core Animation transaction at high drag speed, which is what shows as
    /// the compositor briefly STRETCHING the last frame instead of showing a
    /// blank/stale one — see `Gpu::set_presents_with_transaction`'s doc for
    /// the companion half of this fix (`arm_live_resize_sync` below).
    pub(super) fn on_resized(&mut self, event_loop: &ActiveEventLoop, size: winit::dpi::PhysicalSize<u32>) {
        let mut changed = false;
        let mut request_redraw = true;
        #[cfg(not(target_arch = "wasm32"))]
        let mut reconfigured = false;
        if let Some(gpu) = self.gpu.as_mut() {
            changed = gpu.config.width != size.width || gpu.config.height != size.height;
            if changed {
                gpu.pipeline
                    .hold_lava_field_viewport(gpu.config.width, gpu.config.height);
            }
            #[cfg(not(target_arch = "wasm32"))]
            { reconfigured = gpu.resize(size.width, size.height) == gpu::GpuResizeOutcome::Reconfigured; }
            #[cfg(target_arch = "wasm32")]
            { gpu.resize(size.width, size.height); }
        }
        #[cfg(not(target_arch = "wasm32"))]
        if reconfigured { if let Some(soak) = self.soak.as_mut() { soak.observe_resize(); } }
        self.sync_view(true);
        if changed {
            self.arm_live_resize_sync();
            let outcome = self.gpu.as_mut().map(Gpu::redraw);
            if let Some(outcome) = outcome {
                request_redraw = self.handle_gpu_frame_outcome(event_loop, outcome)
                    .is_ok_and(|(_, presented)| presented);
            }
        }
        if request_redraw { if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }}
    }

    /// Re-arm the cross-platform live-resize settle debounce and (through the
    /// one owner `sync_present_txn`) turn `presentsWithTransaction` ON if no
    /// live stream had it armed yet. Re-stamp the settle deadline either way —
    /// `about_to_wait`'s `RESIZE_SYNC_SETTLE` debounce flips it back off
    /// `RESIZE_SYNC_SETTLE` after the LAST tick, not the first, so a fast
    /// multi-tick drag keeps sliding the deadline forward exactly like the
    /// theme-font/zoom-persist debounces. See `resize_settle_at`'s own doc for
    /// the full mechanism + the user-reported symptom this closes.
    pub(super) fn arm_live_resize_sync(&mut self) {
        self.resize_settle_at = Some(Instant::now());
        self.sync_present_txn();
    }

    /// `WindowEvent::Moved`: the window-server is actively moving the window —
    /// hold the lava lamp (re-stamp the settle debounce, clear the tick arm so
    /// the phase can't advance mid-stream) and, through `sync_present_txn`, arm
    /// `presentsWithTransaction` for the WHOLE stream. The transaction sync is
    /// the structural half of the move-flash fix: pausing the ambient tick
    /// (318e1fe) stopped the ~10 fps mid-move presents, but every OTHER present
    /// around a move — the settle redraw, a sibling debounce (spell/autosave/
    /// toast/zoom-persist) firing mid-stream, a cross-display
    /// `ScaleFactorChanged` redraw — still presented ASYNC and raced the
    /// window-server's move transaction (the diagnosed compositor-flash class;
    /// the resize path already had this cure, the move path never did). Gated
    /// on the lava CAPABILITY: a non-lava world presents nothing around a move,
    /// so it takes this arm as a TOTAL no-op (zero redraws scheduled — the
    /// structural guarantee) and its `Moved` events stay byte-identical to
    /// before the move machinery existed.
    pub(super) fn on_moved(&mut self, _position: winit::dpi::PhysicalPosition<i32>) {
        // Gated on the AMBIENT capability (`Theme::has_ambient_motion` — lava
        // OR twinkling stars, the one gate): both push the same ~10 fps async
        // ambient presents this hold exists to keep out of the window-server's
        // move transaction. A static world presents nothing around a move, so
        // it takes this arm as a TOTAL no-op, byte-identical as ever.
        if crate::theme::active().has_ambient_motion() {
            #[cfg(not(target_arch = "wasm32"))]
            if crate::probe::live_active() { eprintln!("PROBE-TRACE on_moved (ambient world) t={:?}", std::time::Instant::now()); }
            self.move_settle_at = Some(Instant::now());
            self.lava_tick_at = None;
            self.sync_present_txn();
        }
    }

    /// THE ONE APPLIER of the `presentsWithTransaction` composition
    /// (`present_sync_armed`: resize stream OR move stream — see
    /// `App::present_sync_on`'s doc). Idempotent per state: the objc call fires
    /// only on a real transition, so a fast `Moved`/`Resized` burst re-stamping
    /// its debounce costs no per-event layer traffic. The shadow flag is
    /// tracked on every platform; the layer call is macOS-only (the artifact
    /// class is the macOS window-server transaction race).
    pub(super) fn sync_present_txn(&mut self) {
        let want = present_sync_armed(
            self.resize_settle_at.is_some(),
            self.move_settle_at.is_some(),
            self.crossing_settle_at.is_some(),
        );
        if want == self.present_sync_on {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if crate::probe::live_active() {
            eprintln!(
                "PROBE-TRACE present_txn {} (resize={} move={} crossing={}) t={:?}",
                if want { "ON" } else { "OFF" },
                self.resize_settle_at.is_some(),
                self.move_settle_at.is_some(),
                self.crossing_settle_at.is_some(),
                std::time::Instant::now(),
            );
        }
        self.present_sync_on = want;
        #[cfg(target_os = "macos")]
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.set_presents_with_transaction(want);
        }
    }

    /// The RESIZE stream's settle (the `RESIZE_SYNC_SETTLE` debounce elapsed
    /// with no further `Resized` tick): snap the lava field to the final
    /// viewport, drop this stream's claim on the present-transaction sync (the
    /// one owner keeps it armed while a MOVE stream is still live), and request
    /// the ONE settle redraw. Clearing `resize_settle_at` first is what makes
    /// the settle fire exactly once — the `about_to_wait` arm is gated on the
    /// stamp being present.
    pub(super) fn finish_resize_settle(&mut self) {
        self.resize_settle_at = None;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.pipeline
                .settle_lava_field_viewport(gpu.config.width, gpu.config.height);
        }
        self.sync_present_txn();
        // Route through `on_redraw_requested`'s own control-flow decision
        // instead of leaving a now-elapsed `WaitUntil` in place (which would
        // busy-spin `about_to_wait` until the next real input).
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// The MOVE stream's settle (the `MOVE_SETTLE` debounce elapsed with no
    /// further `Moved` tick): clear the hold, clear the tick arm (so the lamp
    /// re-arms FRESH rather than replaying the held gap as a catch-up dt), drop
    /// this stream's claim on the present-transaction sync (armed stays armed
    /// while a RESIZE stream is still live — a corner drag streams both), and
    /// request the ONE settle redraw. The phase and the field were held for the
    /// whole stream (`lava::lava_paused` closed the only door to
    /// `advance_lava`; a pure move never touches `lava_field_viewport`), so
    /// this redraw presents the SAME lava the move started with — no snap, no
    /// flash. Clearing `move_settle_at` first makes it fire exactly once.
    pub(super) fn finish_move_settle(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if crate::probe::live_active() { eprintln!("PROBE-TRACE finish_move_settle t={:?}", std::time::Instant::now()); }
        self.move_settle_at = None;
        self.lava_tick_at = None;
        self.sync_present_txn();
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// The THEME-PREVIEW CROSSING settle (the `CROSSING_SYNC_SETTLE` debounce
    /// elapsed with no further boundary crossing): drop this source's claim on
    /// the present-transaction sync (the one owner keeps it armed while a resize
    /// or move stream is still live — a crossing can overlap a drag) and request
    /// the ONE guaranteed follow-up present. The crossing frame itself already
    /// presented in-transaction (the sync was armed the instant the crossing was
    /// detected, before the keypress's redraw ran); this settle redraw is the
    /// bracket's far edge — a second solid present after the cadence changed, so
    /// the compositor can never be left holding a single stale drawable. Clearing
    /// `crossing_settle_at` first is what makes the `about_to_wait` arm (gated on
    /// the stamp) fire exactly once. Live-only: a headless capture never previews.
    pub(super) fn finish_crossing_settle(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if crate::probe::live_active() { eprintln!("PROBE-TRACE finish_crossing_settle t={:?}", std::time::Instant::now()); }
        self.crossing_settle_at = None;
        self.sync_present_txn();
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// `WindowEvent::ScaleFactorChanged`: the window moved to a monitor with a
    /// different DPI. Refold the new scale into the metrics; a paired `Resized`
    /// (physical size change) follows to re-wrap the column. Both keep the page
    /// proportioned.
    pub(super) fn on_scale_factor_changed(&mut self, scale_factor: f64) {
        let sf = scale_factor as f32;
        self.dpi = sf;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.pipeline.set_dpi(sf);
        }
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// `WindowEvent::RedrawRequested`: advance the caret spring by the real
    /// elapsed time since the last animated frame, then draw. If still animating,
    /// keep the loop hot (Poll + request another redraw); once settled, go back to
    /// Wait so the app idles at 0% CPU until the next input. Also feeds the
    /// DEBUG-panel perf lines (all timing work gated on `debug_on()`) and drives
    /// its settle-stamp.
    pub(super) fn on_redraw_requested(&mut self, event_loop: &ActiveEventLoop) {
        let fault = self.gpu.as_ref().and_then(|g| g.take_faults().into_iter().next());
        if let Some(fault) = fault {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(soak) = self.soak.as_mut() { soak.observe_frame(crate::soak_gpu::FrameOutcome::Skipped(crate::soak_gpu::SkipKind::Fault), false); }
            self.handle_gpu_fault(event_loop, fault);
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        // Consume a wheel/key zoom burst at the present boundary: every input
        // already updated `self.zoom`; one sync now reflows directly to the
        // latest requested level. Put it before the frame clock to preserve the
        // pre-coalescing spring timing (input-side sync used to finish before
        // RedrawRequested began) while key→px still measures through present.
        if self.zoom_reflow.take() {
            self.sync_view(true);
        }
        let now = Instant::now();
        let dt = match self.last_frame {
            Some(prev) => (now - prev).as_secs_f32(),
            // First animated frame: assume one 60fps tick so the very
            // first step is sane rather than a huge dt.
            None => 1.0 / 60.0,
        };
        // Monotonic frames-drawn count (a single add, always — so toggling
        // debug on mid-hot-loop shows a climbing count immediately).
        self.redraw_count += 1;
        // DEBUG panel feed — the ONLY timing work, and all of it gated on
        // the panel being on (the pane never creates the work it measures;
        // the pane-off editor takes zero clock reads). The panel text is
        // shaped inside `pipeline.prepare`, so the values are fed at the
        // TOP of the redraw, BEFORE `gpu.redraw()`: line 1 therefore shows
        // the PREVIOUS completed frame's cost (one-frame lag — this frame's
        // cost isn't knowable until it presents).
        let mut is_stamp = false;
        if crate::debug::debug_on() {
            // Classify this redraw: a pending un-rendered input wins over a
            // queued stamp; any redraw out of stillness is activity. Only a
            // quiet StampQueued redraw IS the settle-stamp frame, which
            // draws the still-prefixed readout.
            self.debug_still =
                crate::debug::still_wake(self.debug_still, self.input_stamp.is_some());
            is_stamp = self.debug_still == crate::debug::DebugStill::StampQueued;
            if let Some(gpu) = self.gpu.as_mut() {
                // ADAPTIVE budget: one vsync of the monitor this window is
                // on (16.6 at 60 Hz, 8.3 at 120 Hz; 60 Hz fallback when
                // winit can't name a rate).
                let budget = crate::debug::budget_ms(
                    gpu.window
                        .current_monitor()
                        .and_then(|m| m.refresh_rate_millihertz()),
                );
                let cost = self
                    .frame_costs
                    .last()
                    .and_then(|l| self.frame_costs.worst().map(|w| (l, w)));
                gpu.pipeline.set_debug_perf(
                    cost,
                    self.last_latency_ms,
                    Some(self.redraw_count),
                    is_stamp,
                    Some(budget),
                );
                // Also surface the live GPU memory (macOS: Metal's
                // currentAllocatedSize; `None` elsewhere → `gpu —`).
                let bytes = gpu.current_gpu_bytes();
                gpu.pipeline.set_debug_gpu_bytes(bytes);
                // AUTOSAVE-ENGINE line: composed EXCLUSIVELY from what
                // `App::autosave_flush`'s one door already tracks — config's
                // `autosave_on()`, the clobber guard's `notice`, and the
                // engine's own last-write clock — so it can never say
                // anything the engine didn't just do. The only clock read
                // here (`Instant::now() - autosave_last_ok`) is gated on
                // `debug_on()` like every other perf read this block makes.
                let since_secs = self
                    .autosave_last_ok
                    .map(|t| (Instant::now() - t).as_secs());
                let autosave = crate::debug::autosave_state(
                    self.config.autosave_on(),
                    self.notice.is_some(),
                    since_secs,
                );
                gpu.pipeline.set_debug_autosave(Some(autosave));
            }
        } else if self.input_stamp.is_some()
            || self.last_latency_ms.is_some()
            || self.frame_costs.last().is_some()
        {
            // Panel just turned off: forget the measurements so the next
            // enable starts fresh (placeholders, then live numbers), and
            // re-feed the pipeline defaults (still-form placeholders).
            self.input_stamp = None;
            self.last_latency_ms = None;
            self.frame_costs.clear();
            self.debug_still = crate::debug::DebugStill::Active;
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.pipeline.set_debug_perf(None, None, None, true, None);
                gpu.pipeline.set_debug_autosave(None);
            }
        }
        // A STATIC open overlay must NOT busy-loop: an idle menu is a frozen
        // frame, so forcing ControlFlow::Poll just because an overlay is open
        // re-ran prepare_overlay/set_rich_text every frame, pegging the CPU.
        // Instead the overlay redraws ON INPUT — every overlay-affecting key
        // (query edit, selection move, filter, open/close) is a KeyboardInput
        // event that routes through `apply` and then calls request_redraw
        // below, and OS key AUTO-REPEAT for a HELD arrow delivers a fresh
        // KeyboardInput per repeat, so a held arrow still repaints promptly.
        // The loop only stays HOT while the caret spring is still animating.
        let (animating, outcome) = if let Some(gpu) = self.gpu.as_mut() {
            // Drive the virtual-clock seam (caret spring + any future live
            // animator) so the timeline capture and the live loop advance
            // animation through the SAME entry point.
            let still = gpu.pipeline.advance(dt);
            let presented = gpu.redraw();
            // Once the spring settles the caret is fully static (the I-beam no
            // longer breathes) and there is nothing else animating, so the loop
            // idles at 0% CPU until the next input requests a redraw.
            (still, presented)
        } else {
            return;
        };
        let (presented, frame_presented) = match self.handle_gpu_frame_outcome(event_loop, outcome) {
            Ok(result) => result,
            Err(()) => { event_loop.set_control_flow(ControlFlow::Wait); return; }
        };
        // DEBUG bookkeeping for the frame that just PRESENTED (`presented`
        // is `Some` only with the panel on — see `Gpu::redraw`): close the
        // key→px span at present-return, and push the measured cost into
        // the ring for the NEXT frame's line 1 — except on the stamp frame,
        // whose cost is measured and DISCARDED (panel bookkeeping, not user
        // workload; displaying it would take yet another frame). An
        // early-return redraw (`None`) keeps the input stamp alive so the
        // latency measures through to the retry frame that really presents.
        if let Some((cost_ms, done)) = presented {
            if let Some(stamp) = self.input_stamp.take() {
                self.last_latency_ms = Some((done - stamp).as_secs_f32() * 1000.0);
            }
            if !is_stamp {
                self.frame_costs.push(cost_ms);
            }
        }

        // Keep the loop hot ONLY while the spring animates — the debug panel
        // schedules ZERO frames of its own (every metric it shows is
        // meaningful for a single sparse frame). The held stats HUD does NOT
        // force frames either: its figures are pure functions of the doc
        // (no session clock), so a held HUD is a single settled frame over
        // the cached frosted backdrop. `last_frame` still tracks ONLY the
        // spring, so the dt fed to `advance` stays correct.
        // A failed acquire never drives the animation Poll loop. The spring
        // simply resumes from the next OS/input/timed wake; otherwise an
        // occluded window can allocate and prepare thousands of unseen frames.
        let keep_hot = keep_gpu_loop_hot(animating, frame_presented);
        self.last_frame = if keep_hot { Some(now) } else { None };
        if keep_hot {
            event_loop.set_control_flow(ControlFlow::Poll);
            if let Some(gpu) = self.gpu.as_ref() {
                gpu.window.request_redraw();
            }
        } else {
            // Settled: stop driving frames and idle until next input.
            event_loop.set_control_flow(ControlFlow::Wait);
        }
        // DEBUG settle-stamp: the first redraw that ends SETTLED while the
        // panel is on queues exactly ONE more frame — the stamp that draws
        // the `still ·` readout with the final true numbers — and then the
        // machine goes fully quiet (the stamp itself requests nothing).
        // Control flow stays `Wait`; `request_redraw` alone delivers the
        // one frame. New input meanwhile simply wins (see `still_wake`).
        if crate::debug::debug_on() {
            let (next, request_stamp) = crate::debug::still_settle(self.debug_still, animating);
            self.debug_still = next;
            if request_stamp {
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
        }
    }
}
