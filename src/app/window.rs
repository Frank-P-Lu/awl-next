//! WINDOW-LIFECYCLE + redraw arm bodies lifted out of `App::window_event` (now a
//! thin dispatcher). These are the arms that are NOT input: focus-lost flush,
//! physical resize, DPI (scale-factor) refold, and the redraw-request frame
//! loop (caret spring advance + present + the debug-panel perf feed). Lifted
//! verbatim — each method IS one former `match` arm, so the ORDER and behaviour
//! (including the redraw arm's control-flow decisions) are unchanged. The input
//! arms live in `app/input/`.

use super::*;

impl App {
    /// `WindowEvent::Focused(false)`: the window lost focus. ROBUST AUTOSAVE —
    /// flush a pending note write, the document autosave / scratch stash, and
    /// (native only) the session restore state on the same blur trigger. Also
    /// resets the OS pointer to Visible so a focus change never leaves it hidden
    /// behind another app.
    pub(super) fn on_focus_lost(&mut self) {
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
    pub(super) fn on_resized(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.resize(size.width, size.height);
        }
        self.sync_view(true);
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
        let (animating, presented) = if let Some(gpu) = self.gpu.as_mut() {
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
            (false, None)
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
        let keep_hot = animating;
        self.last_frame = if animating { Some(now) } else { None };
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
