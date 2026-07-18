//! The `about_to_wait` SCHEDULING body, lifted verbatim out of
//! `App::about_to_wait` (the decomposition round — zero behaviour change).
//! This is the winit idle pass that owns every debounce / settle deadline:
//! which-key + hold-peek pauses, the spell / note / document autosave idle
//! timers, the theme-font / sticky-zoom / resize / move / crossing settles,
//! the ambient (lava/stars) tick, event-toast expiry, GPU acquire retries,
//! and the GPU soak drive — each a single `WaitUntil` (never a hot per-frame
//! loop), guarded on `last_frame` so the caret spring's `Poll` always wins.
//!
//! A trait impl can't span files, so the body moves to an inherent `App`
//! method here and the `ApplicationHandler::about_to_wait` in `app.rs` stays
//! a thin delegate. `use super::*` reaches every free helper it calls
//! (`debounce_due`, `control_flow_with_deadline`, `notice_expired`, the
//! debounce-window consts) — those stay in `app.rs`, shared with other sites.

use super::*;

impl App {
    pub(super) fn about_to_wait_impl(&mut self, event_loop: &ActiveEventLoop) {
        // WHICH-KEY pause: while a PREFIX (`C-x`) is pending its second key, summon the
        // continuation panel once ~500ms elapses without a follow-up. The timer is
        // ARMED ONLY here, while `prefix_pending_at` is `Some` AND the panel isn't yet
        // shown — a single `WaitUntil` deadline, no perpetual per-frame tick; once it
        // fires (or the prefix resolves, clearing `prefix_pending_at`) nothing re-arms,
        // so the app idles at 0% CPU (DESIGN §6).
        if let Some(pending) = self.prefix_pending_at {
            let deadline = pending + crate::whichkey::PAUSE;
            let elapsed = self.clock.now() >= deadline;
            if crate::whichkey::should_summon(true, self.whichkey_shown, elapsed) {
                self.summon_whichkey();
            } else if !self.whichkey_shown && !elapsed && self.last_frame.is_none() {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
        }
        // HOLD-⌘ SHORTCUT PEEK: while a bare-arming-modifier hold is PENDING, summon the
        // card once ~600ms elapses with the hold unbroken. The timer is ARMED ONLY while
        // `peek_armed_at` is `Some` (the `PeekArm::Pending` state) — a single `WaitUntil`
        // deadline, no perpetual tick; feeding `Elapsed` opens the card and clears the
        // stamp, so nothing re-arms and the app idles at 0% CPU (the which-key pattern).
        if let Some(armed) = self.peek_armed_at {
            let deadline = armed + Duration::from_millis(crate::peek::HOLD_PEEK_MS);
            if self.clock.now() >= deadline {
                // ZOOM-SUPPRESSION GATE: the pause elapsed, but if a zoom is in flight
                // (the sticky-zoom debounce window is open) the card would pop up over
                // the text being resized — feed the cancelling `ArmBroken` instead of
                // `Elapsed`, folding the pending hold back to Idle. It re-arms only once
                // the zoom settles. (`peek_allowed` is the ONE pure suppression owner,
                // shared with the arming seam in `on_modifiers_changed`.)
                let stim = if crate::peek::peek_allowed(self.zoom_in_flight()) {
                    crate::peek::PeekStimulus::Elapsed
                } else {
                    crate::peek::PeekStimulus::ArmBroken
                };
                self.feed_peek(stim);
            } else if self.last_frame.is_none() {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
        }
        // Debounced spell check: re-scan only after ~150ms with no edits, so a
        // word isn't squiggled while you're still typing it.
        if let Some(dirty) = self.spell_dirty_at {
            let deadline = dirty + SPELL_DEBOUNCE;
            if self.clock.now() >= deadline {
                self.run_spellcheck_now();
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            } else if self.last_frame.is_none() {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
        }
        // Debounced quick-note AUTO-SAVE: write the note after ~400ms of quiet, so
        // it persists calmly as you pause. An empty note writes nothing.
        if let Some(dirty) = self.autosave_dirty_at {
            let deadline = dirty + AUTOSAVE_DEBOUNCE;
            if self.clock.now() >= deadline {
                self.autosave_dirty_at = None;
                self.autosave_note();
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            } else if self.last_frame.is_none() {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
        }
        // Debounced DOCUMENT AUTOSAVE (the config-gated engine, default ON): the
        // open file is written atomically — or the no-path scratch stashed — after
        // ~1s of idle. Armed ONLY by the live `sync_view` (behind its gpu-present
        // gate), consumed here via the same single-`WaitUntil` pattern as the note
        // autosave above — no hot loop, and structurally unreachable headlessly.
        if let Some(dirty) = self.doc_autosave_at {
            match debounce_due(dirty, AUTOSAVE_IDLE, self.clock.now()) {
                true => {
                    self.doc_autosave_at = None;
                    self.autosave_flush();
                    // LIFETIME STATS: piggyback the same ~1s idle door, so the
                    // odometer is crash-safe without its own timer (native only;
                    // config + dirty gated inside).
                    #[cfg(not(target_arch = "wasm32"))]
                    self.stats_flush();
                    // WRITING STREAKS: sample the day-delta on the same idle door.
                    #[cfg(not(target_arch = "wasm32"))]
                    self.streaks_flush();
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.window.request_redraw();
                    }
                }
                false if self.last_frame.is_none() => {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(dirty + AUTOSAVE_IDLE));
                }
                false => {}
            }
        }
        // Debounced theme-preview FONT reshape: while the theme picker's live preview
        // arrows across worlds, only the COLORS applied per step; once the selection
        // rests `THEME_FONT_DEBOUNCE` the one deferred reshape lands here (the paused
        // hover then shows the true face). Each further preview step RE-STAMPS
        // `theme_font_at` (`retint_theme_preview`), sliding the deadline — the same
        // single-`WaitUntil`, idle-safe pattern as the zoom persist below (no hot
        // loop; commit/revert clear the stamp synchronously via `retint_theme_now`).
        if let Some(dirty) = self.theme_font_at {
            match debounce_due(dirty, THEME_FONT_DEBOUNCE, self.clock.now()) {
                true => self.apply_deferred_theme_font(),
                false if self.last_frame.is_none() => {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(dirty + THEME_FONT_DEBOUNCE));
                }
                false => {}
            }
        }
        // Debounced STICKY-ZOOM write: persist the SETTLED zoom after ~500ms of quiet,
        // so a rapid Cmd-=/Cmd-- run writes the final value once (not one-per-step).
        // Each new zoom step RE-STAMPS `zoom_persist_at` (via `mark_zoom_dirty`), so the
        // deadline keeps sliding forward until the user pauses — the debounce contract.
        if let Some(dirty) = self.zoom_persist_at {
            match debounce_due(dirty, ZOOM_PERSIST_DEBOUNCE, self.clock.now()) {
                true => {
                    self.zoom_persist_at = None;
                    self.persist_zoom_now();
                    // The gesture settled: clear the floating zoom readout (armed per
                    // step in `mark_zoom_dirty`), parking its label off-screen again.
                    // Fire like the sibling debounces above: request a redraw so the
                    // RedrawRequested handler re-decides control flow (Wait when settled),
                    // instead of leaving it at this now-elapsed WaitUntil — which would
                    // busy-spin the loop at ~100% CPU until the next input (DESIGN §6).
                    if let Some(gpu) = self.gpu.as_mut() {
                        gpu.pipeline.set_zoom_readout(None);
                        gpu.window.request_redraw();
                    }
                }
                false if self.last_frame.is_none() => {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(dirty + ZOOM_PERSIST_DEBOUNCE));
                }
                false => {}
            }
        }
        // LIVE-RESIZE CONTENT-STRETCH FIX settle (macOS only — see
        // `resize_settle_at`'s doc): once `RESIZE_SYNC_SETTLE` passes with no
        // further `Resized` ticks, flip the CAMetalLayer's `presentsWithTransaction`
        // back OFF (paying its throughput cost only while a drag is actually live).
        // Each new tick RE-STAMPS `resize_settle_at` (`App::arm_live_resize_sync`),
        // sliding the deadline exactly like the theme-font/zoom-persist debounces
        // above — the same single-`WaitUntil` shape, so a still window costs nothing.
        if let Some(dirty) = self.resize_settle_at {
            match debounce_due(dirty, RESIZE_SYNC_SETTLE, self.clock.now()) {
                true => self.finish_resize_settle(),
                false if self.last_frame.is_none() => {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(dirty + RESIZE_SYNC_SETTLE));
                }
                false => {}
            }
        }
        // MOVE-stream settle (mirrors the resize debounce above; see
        // `MOVE_SETTLE`'s doc for why its window is deliberately longer).
        if let Some(dirty) = self.move_settle_at {
            match debounce_due(dirty, MOVE_SETTLE, self.clock.now()) {
                true => self.finish_move_settle(),
                false if self.last_frame.is_none() => {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(dirty + MOVE_SETTLE));
                }
                false => {}
            }
        }
        // THEME-PREVIEW CROSSING settle (mirrors the resize/move debounces above;
        // see `CROSSING_SYNC_SETTLE`'s doc). Disarms the present-transaction sync
        // and fires the ONE follow-up present once a boundary crossing has rested.
        if let Some(dirty) = self.crossing_settle_at {
            match debounce_due(dirty, CROSSING_SYNC_SETTLE, self.clock.now()) {
                true => self.finish_crossing_settle(),
                false if self.last_frame.is_none() => {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(dirty + CROSSING_SYNC_SETTLE));
                }
                false => {}
            }
        }
        // AMBIENT TICK — the slow ~10 fps drift clock behind awl's time-varying
        // grounds: the lava lamp (Firetail/Mangrove) AND the twinkling stars
        // (Currawong) — ONE clock, two consumers (`TextPipeline::lava_phase`).
        // A single `WaitUntil` cadence (NEVER the caret spring's hot per-frame
        // `Poll` loop): when it elapses, advance the phase, request ONE redraw,
        // and re-arm. Armed ONLY while `lava::lava_should_tick` holds — an
        // ambient-motion world is active (`Theme::has_ambient_motion`, the ONE
        // gate) AND `ambient_motion` is on AND motion is not reduced AND the
        // window is focused (pause on blur). Every static world schedules ZERO
        // ambient frames — preserving 0% idle CPU there.
        let lava_active = crate::theme::active().has_ambient_motion();
        let lava_paused = crate::lava::lava_paused(
            self.resize_settle_at.is_some(),
            self.move_settle_at.is_some(),
            self.gpu
                .as_ref()
                .is_some_and(|gpu| gpu.pipeline.lava_blur_active()),
        );
        if crate::lava::lava_should_tick(
            lava_active,
            self.config.ambient_motion_on(),
            crate::motion::reduced(),
            self.focused,
            lava_paused,
        ) {
            let now = self.clock.now();
            match self.lava_tick_at {
                Some(last) if now.saturating_duration_since(last) >= LAVA_TICK => {
                    // Due: hand the elapsed time to the bounded ambient advance
                    // (`lava::ambient_tick_dt` clamps it to one fixed tick), so a
                    // delayed macOS window-drag wake cannot catch up in one jump.
                    // The follow-up `about_to_wait` pass (after the redraw) re-arms
                    // the single `WaitUntil` via the `_` arm below.
                    let dt = (now - last).as_secs_f32();
                    self.lava_tick_at = Some(now);
                    if let Some(gpu) = self.gpu.as_mut() {
                        gpu.pipeline.advance_lava(dt);
                        gpu.window.request_redraw();
                    }
                }
                _ => {
                    // Not due yet (or the first arm): keep/arm the single
                    // `WaitUntil`, but NEVER override the caret spring's hot `Poll`
                    // loop (guard on `last_frame`, like every sibling debounce) —
                    // during a caret glide the lava still advances above at ~10 fps
                    // while the frame itself redraws at full refresh.
                    let last = *self.lava_tick_at.get_or_insert(now);
                    if self.last_frame.is_none() {
                        event_loop.set_control_flow(control_flow_with_deadline(
                            event_loop.control_flow(),
                            last + LAVA_TICK,
                        ));
                    }
                }
            }
        } else if lava_active {
            // An ambient-motion world (lava or stars), but the ground must be
            // STATIC: reduce motion OR ambient motion off (blur is handled at
            // the focus edge, which merely HOLDS the phase). Stop ticking;
            // hard-freeze the shared phase to the settled frame so a later
            // resume restarts cleanly rather than from a stale mid-breath.
            self.lava_tick_at = None;
            if crate::motion::reduced() || !self.config.ambient_motion_on() {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.pipeline.freeze_lava();
                }
            }
        }
        // EVENT TOAST expiry: one live-only deadline, consumed once. This runs
        // after sibling timers so it can choose the EARLIER deadline instead of
        // delaying a lava tick (or being delayed by one). Poll always wins.
        if let Some(deadline) = self.notice_expires_at {
            if notice_expired(self.notice_kind, Some(deadline), self.clock.now()) {
                self.clear_notice();
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            } else if self.last_frame.is_none() {
                event_loop.set_control_flow(control_flow_with_deadline(
                    event_loop.control_flow(),
                    deadline,
                ));
            }
        }
        // GPU acquire retries are App-owned timers, never immediate redraw
        // recursion. Occlusion deliberately arms no timer: an OS visibility,
        // input, lava, or probe wake is the next opportunity. Timeout and
        // surface reconfiguration arrive here after their bounded delay.
        if let Some(deadline) = self.gpu_retry_at {
            if self.clock.now() >= deadline {
                self.gpu_retry_at = None;
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            } else if self.last_frame.is_none() {
                event_loop.set_control_flow(control_flow_with_deadline(
                    event_loop.control_flow(),
                    deadline,
                ));
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.drive_gpu_soak(event_loop);
    }
}
