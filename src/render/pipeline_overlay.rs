//! PIPELINE OVERLAY — the per-frame ANIMATION advance of [`super::TextPipeline`].
//!
//! The single virtual-clock seam (`advance`) and every animator it OR-folds in,
//! carved out of `render.rs` VERBATIM: the summoned-overlay ENTRANCE spring +
//! living-band + slant/grow motion (`overlay_*` / `living_band_*`), the
//! lava/stars background field (`advance_lava` + the viewport holds + the
//! render-phase reads), the copy-pulse juice, and the caret-style-picker
//! preview. i.e. the time-varying `advance(dt)` surface, named for the
//! overlay-motion cluster that dominates it. Methods stay inherent on
//! `TextPipeline` (a child module sees its ancestor's private fields), so the
//! capture output is byte-identical. `copy_pulse_settle` is widened private ->
//! `pub(in crate::render)` for its pre-existing cross-submodule caller
//! (`layers`) — reachability preserved exactly.

use super::*;

impl TextPipeline {
    /// THE single virtual-clock seam: advance every time-varying renderer state by
    /// `dt` seconds and report whether ANYTHING is still animating (so the caller
    /// keeps redrawing). The caret spring is the primary animator; any future
    /// animator (a status fade) that exposes the same `step(dt) -> still_animating`
    /// contract is OR-folded in here, e.g. `self.step_caret(dt) | self.fade.step(dt)`.
    /// Both the windowed loop and the deterministic timeline capture drive the clock
    /// through this one entry point, so neither needs to know WHICH animation it advances.
    pub fn advance(&mut self, dt: f32) -> bool {
        self.step_caret(dt)
            | self.step_caret_preview(dt)
            | self.step_copy_pulse(dt)
            | self.step_overlay_juice(dt)
    }

    /// LIVE-APP-ONLY: arm the motion-juice animators (overlay entrance spring
    /// + selection-band slide — the FIRETAIL-MAXIMALIST-SHOWCASE round's
    /// [`theme::MotionJuice`] capability). Called exactly once, from the live
    /// App's GPU init (`app/gpu.rs`); every headless capture / bench / test
    /// pipeline never calls it, so those paths render the settled state
    /// STRUCTURALLY (the determinism law's "live-only animation renders its
    /// settled state in capture", enforced by construction rather than by a
    /// per-frame check). Arming alone changes nothing: the animators also
    /// require a non-CALM effective [`theme::MotionJuice`] (no world ships
    /// one — the `AWL_MOTION_FORCE` probe is the only current door) and fold
    /// to nothing under Reduce Motion.
    pub fn arm_live_juice(&mut self) {
        self.juice_live = true;
    }

    /// Tick the overlay ENTRANCE spring + selection-band SLIDE by `dt`
    /// seconds. Returns true while either is still easing (keeps the live
    /// redraw loop hot exactly as long as the juice plays — then idle).
    ///
    /// ACCESSIBILITY TIER 1 — REDUCE MOTION: both animators settle INSTANTLY
    /// (same final position, zero frames of ease — `motion.rs`'s pure
    /// time-compression contract), mirroring `step_copy_pulse`'s gate
    /// exactly. Law-tested by `overlay_juice_folds_to_nothing_under_reduce_
    /// motion` (render/tests/motion_juice.rs).
    fn step_overlay_juice(&mut self, dt: f32) -> bool {
        if crate::motion::reduced() {
            self.overlay_enter_t = 1.0;
            self.overlay_band_t = 1.0;
            return false;
        }
        let mut hot = false;
        if self.overlay_enter_t < 1.0 {
            self.overlay_enter_t =
                (self.overlay_enter_t + dt * 1000.0 / OVERLAY_ENTRANCE_MS).min(1.0);
            hot |= self.overlay_enter_t < 1.0;
        }
        if self.overlay_band_t < 1.0 {
            self.overlay_band_t =
                (self.overlay_band_t + dt * 1000.0 / OVERLAY_BAND_SLIDE_MS).min(1.0);
            hot |= self.overlay_band_t < 1.0;
        }
        hot
    }

    /// The overlay card's ENTRANCE y-offset THIS frame: exactly `0.0` when
    /// settled (every capture, every CALM world, Reduce Motion, and every
    /// frame after the ~200ms spring lands — `card_y + 0.0` is bit-identical
    /// to the pre-round geometry), else the eased drop-in: the card starts
    /// [`OVERLAY_ENTRANCE_DROP_PX`] ABOVE its resting place and springs down
    /// with a small overshoot ([`crate::ease::out_back`]). Folded into
    /// `card_y` at the END of both geometry owners (`overlay_geometry` /
    /// `theme_overlay_geometry`) — after all row-fit math, so the transient
    /// offset can never change how many rows the card shows — and because the
    /// geometry is the ONE shared source, the card quad, rows, band, caret,
    /// and hit-tests all ride the spring together (never desynced).
    pub(in crate::render) fn overlay_entrance_offset(&self) -> f32 {
        if self.overlay_enter_t >= 1.0 {
            return 0.0;
        }
        -(1.0 - crate::ease::out_back(self.overlay_enter_t)) * OVERLAY_ENTRANCE_DROP_PX
    }

    /// THE ONE band-RETARGET owner: point the shared chase state
    /// (`overlay_band_from`/`overlay_band_last`/`overlay_band_t`) at a NEW
    /// `target`, continuing smoothly from wherever the band is actually drawn
    /// RIGHT NOW if a transition is still in flight. Shared by both animators
    /// that chase the selected row — the ordinary [`Self::overlay_band_drawn`]
    /// (`BandResponse::Slide`) and the living-band choreography
    /// ([`Self::living_band_phase`]) — so a rapid re-target (arrow-key repeat
    /// outrunning the ~110ms slide) can never teleport the visual anchor to
    /// the STALE previous target instead of where the band visually sits.
    ///
    /// THE BUG THIS CLOSED: `living_band_phase` used to set
    /// `overlay_band_from = last` (the previous call's TARGET) on every
    /// re-target, discarding the in-flight interpolation. Held-down/fast
    /// Down presses (each firing before the prior ~110ms ease settled) then
    /// SNAPPED the drawn band to each stale intermediate target instead of
    /// gliding continuously — a visible pop that reads as "the highlight
    /// lags/jumps behind what Enter would actually run" (the selection-desync
    /// report). `overlay_band_drawn` already computed the correct current
    /// eased position (`cur`); this is that fix, promoted to the ONE shared
    /// owner so the two seams can never diverge again.
    ///
    /// A fresh overlay (`overlay_band_last == None`) SETTLES rather than
    /// easing — there is no meaningful previous row to glide from.
    fn retarget_band(&mut self, target: f32) {
        match self.overlay_band_last {
            Some(last) if (last - target).abs() > 0.5 => {
                // A selection move: start the slide FROM wherever the band is
                // drawn right now (mid-flight moves chain smoothly).
                let cur = if self.overlay_band_t < 1.0 {
                    let e = crate::ease::out_back(self.overlay_band_t);
                    self.overlay_band_from + (last - self.overlay_band_from) * e
                } else {
                    last
                };
                self.overlay_band_from = cur;
                self.overlay_band_t = 0.0;
                self.overlay_band_last = Some(target);
            }
            None => {
                // First frame of a fresh overlay: no previous row — settle.
                self.overlay_band_from = target;
                self.overlay_band_last = Some(target);
                self.overlay_band_t = 1.0;
            }
            _ => {}
        }
    }

    /// The selection BAND's drawn row-top for a target `row_top` this frame —
    /// the [`theme::BandResponse::Slide`] seam, called only by
    /// `overlay_draw_card`. Snap worlds (every world today), unarmed
    /// pipelines (every capture), and Reduce Motion all return `target`
    /// verbatim (byte-identical). A Slide world eases from the previous row's
    /// top with the same gentle overshoot spring as the entrance. Purely
    /// visual: the shaped rows and the hit-test never move.
    pub(in crate::render) fn overlay_band_drawn(&mut self, target: f32) -> f32 {
        let slide = self.juice_live
            && !crate::motion::reduced()
            && crate::render::effective_motion_juice().band == theme::BandResponse::Slide;
        if !slide {
            self.overlay_band_last = Some(target);
            self.overlay_band_t = 1.0;
            return target;
        }
        self.retarget_band(target);
        if self.overlay_band_t >= 1.0 {
            return target;
        }
        let e = crate::ease::out_back(self.overlay_band_t);
        self.overlay_band_from + (target - self.overlay_band_from) * e
    }

    /// ARM B LIVING-BAND PROBE — the band's TRAVEL (`from_top`, `to_top`) + PHASE
    /// `t` for the morph / two-shape choreography this frame. Two modes:
    ///
    /// * PINNED (`force.phase` set — the capture frame-dump path): a synthetic
    ///   travel from [`livingband::PIN_JUMP_ROWS`] rows BELOW the selected row,
    ///   sliding up to it, held at the fixed phase. Deterministic (no clock), so
    ///   `--screenshot` dumps a byte-stable mid-flight frame.
    /// * LIVE (`force.phase` absent): reuses the SAME `overlay_band_from/last/t`
    ///   tracking the ordinary slide uses, through the ONE shared retarget owner
    ///   [`Self::retarget_band`] (a fresh overlay settles; a selection move
    ///   chains smoothly from wherever the band is actually drawn right now, not
    ///   the stale previous target — see that owner's doc for the bug this
    ///   closed). [`Self::step_overlay_juice`] advances `overlay_band_t`, and
    ///   Reduce Motion folds it to `1.0` (settled) — so the whole choreography
    ///   inherits the accessibility contract for free.
    ///
    /// Called ONLY from `overlay_draw_card`'s Pane arm when the probe is set; the
    /// ordinary path never reaches it, so an unset-env run is byte-identical.
    pub(in crate::render) fn living_band_phase(
        &mut self,
        force: livingband::MotionForce,
        target: f32,
        lh: f32,
    ) -> (f32, f32, f32) {
        if let Some(phase) = force.phase {
            let from = target + livingband::PIN_JUMP_ROWS * lh;
            return (from, target, phase.clamp(0.0, 1.0));
        }
        // SETTLE in every unarmed pipeline (every capture) and under Reduce Motion —
        // mirrors [`Self::overlay_band_drawn`]. A settled frame is `morph_band(target,
        // target, .., 1.0)` = the exact target rect, so with MORPH (the shipped live
        // default) a settled capture is BYTE-IDENTICAL to the ordinary single band;
        // the choreography only breathes in the live app. This is what makes the
        // on-by-default flip safe, and gives the whole choreography the accessibility
        // contract (Reduce Motion → no motion) for free.
        if !self.juice_live || crate::motion::reduced() {
            self.overlay_band_last = Some(target);
            self.overlay_band_t = 1.0;
            return (target, target, 1.0);
        }
        self.retarget_band(target);
        (self.overlay_band_from, self.overlay_band_last.unwrap_or(target), self.overlay_band_t)
    }

    /// ARM B LIVING-BAND PROBE — the choreography's drawn rects this frame, from
    /// the pure phase math ([`livingband`]). Returns `(primary, echo, cross)`
    /// full-width row rects: `primary` for `overlay_rows` (the leading band),
    /// `echo` for `overlay_bars` (the chasing echo — empty for the single-band
    /// MORPH), and `cross` for `overlay_cross` (the brightest crossing — empty
    /// unless a two-shape overlap exists this frame). Pure over its inputs (no
    /// GPU, no clock); `&self` only.
    pub(in crate::render) fn living_band_rects(
        &self,
        force: livingband::MotionForce,
        from: f32,
        to: f32,
        t: f32,
        card_x: f32,
        card_w: f32,
        lh: f32,
    ) -> (Vec<[f32; 4]>, Vec<[f32; 4]>, Vec<[f32; 4]>) {
        let params = force.choreo.params();
        if force.choreo.is_two_shape() {
            let s = livingband::two_shape_band(from, to, lh, t, &params);
            let primary = vec![[card_x, s.primary_top, card_w, s.height]];
            let echo = vec![[card_x, s.echo_top, card_w, s.height]];
            let cross = s
                .overlap
                .map(|o| vec![[card_x, o.top, card_w, o.height]])
                .unwrap_or_default();
            (primary, echo, cross)
        } else {
            let b = livingband::morph_band(from, to, lh, t, &params);
            (vec![[card_x, b.top, card_w, b.height]], Vec::new(), Vec::new())
        }
    }

    /// The slant FAN-IN progress this frame (motion choreography 3): the fraction
    /// of the diagonal stair currently drawn. `1.0` (full stagger) in EVERY
    /// capture and on every unarmed / CALM pipeline (byte-identical to the settled
    /// slant), so the determinism law holds by construction; the mid-animation
    /// frame-dump probe ([`crate::render::overlay_motion_probe`]) pins it; a live
    /// SpringIn world eases it from `0` as the card springs in (the stair
    /// UNFURLS). Reduce Motion → `1.0` (settled instantly). It multiplies the
    /// per-row DRAW offset only — the width TAX stays at the full max offset, so
    /// rows never reflow mid-flight (they are pre-elided for the settled stair and
    /// merely slide into place).
    pub(in crate::render) fn overlay_slant_progress(&self) -> f32 {
        if let Some(m) = crate::render::overlay_motion_probe() {
            return crate::ease::out_back(m.enter);
        }
        if !self.juice_live || crate::motion::reduced() {
            return 1.0;
        }
        crate::ease::out_back(self.overlay_enter_t)
    }

    /// The selected-bar GROW-POP progress this frame (motion choreography 4): the
    /// fraction of the `grow_px` ledge currently extended. `1.0` (full ledge) in
    /// every capture / unarmed / CALM pipeline (byte-identical); pinned by the
    /// frame-dump probe; on a live Slide world it rides `overlay_band_t` so the
    /// ledge COLLAPSES then juts back out on each selection move (the grow and the
    /// band slide share one timer, one spring). Reduce Motion → `1.0`.
    pub(in crate::render) fn overlay_grow_progress(&self) -> f32 {
        if let Some(m) = crate::render::overlay_motion_probe() {
            return crate::ease::out_back(m.band);
        }
        if !self.juice_live || crate::motion::reduced() {
            return 1.0;
        }
        crate::ease::out_back(self.overlay_band_t)
    }

    /// The per-DISPLAY-ROW slant DRAW offset (device px) this frame — the ONE
    /// owner every slant consumer (the row text areas, the Pane selected band,
    /// and the Bars plates) reads, so the stair, its fan-in, and every surface
    /// that rides it can never disagree. `0.0` when the slant probe is unset
    /// (byte-identical); else [`crate::render::slant_offset`] scaled by the
    /// fan-in progress. Unsigned (always steps right, width-taxed on the right);
    /// the right-anchor composition rides the EXISTING grow mirror, not a slant
    /// mirror (banked — a left-stepping stair clips the text bounds' left edge).
    pub(in crate::render) fn overlay_slant_dx(&self, row: usize) -> f32 {
        match crate::render::overlay_slant() {
            None => 0.0,
            Some(s) => crate::render::slant_offset(&s, row) * self.overlay_slant_progress(),
        }
    }

    /// THE EFFECTIVE margin background this frame — the active world's own
    /// [`theme::Background`], UNLESS the dev gallery knob (`AWL_LAVA=...`) forces a
    /// [`Background::Lava`] over it (`crate::lava::env_override`). For every one of
    /// the fifteen shipped worlds (no knob) this is exactly `theme::background()`,
    /// so both the lava layer and the sidecar report precisely what's drawn.
    pub fn effective_background(&self) -> theme::Background {
        crate::lava::env_override().unwrap_or_else(theme::background)
    }

    /// THE EFFECTIVE lava PHASE this frame, resolving the determinism ladder in
    /// one place ([`crate::lava::lava_phase_for`]): the dev gallery knob's fixed
    /// phase wins outright; else Reduce Motion pins [`crate::lava::LAVA_FROZEN_PHASE`];
    /// else the App-driven [`Self::lava_phase`] (which stays the frozen 0.0 in a
    /// headless capture, since the capture never ticks — so a capture always
    /// renders the fixed t=0 phase). Read by [`Self::prepare_lava_layer`] + the
    /// capture sidecar.
    pub fn lava_render_phase(&self) -> f32 {
        crate::lava::lava_phase_for(
            self.lava_phase,
            crate::motion::reduced(),
            crate::lava::env_phase(),
        )
    }

    /// THE EFFECTIVE TWINKLE PHASE this frame — the SAME determinism ladder as
    /// [`Self::lava_render_phase`] (one resolver, [`crate::lava::lava_phase_for`]),
    /// fed the stars' own dev gallery knob (`AWL_STARS_PHASE`): env override >
    /// Reduce-Motion freeze (static stars — present, not twinkling) > the
    /// App-driven ambient [`Self::lava_phase`] (ONE clock, two consumers; the
    /// frozen 0.0 in every headless capture, since the capture never ticks).
    /// Read by [`Self::prepare_stars_layer`] + the capture sidecar.
    pub fn stars_render_phase(&self) -> f32 {
        crate::lava::lava_phase_for(
            self.lava_phase,
            crate::motion::reduced(),
            crate::stars::env_phase(),
        )
    }

    /// Advance the lava lamp's animation phase by `dt` seconds — called ONLY by
    /// the live App's slow ambient tick (`App::about_to_wait`), NEVER `advance()`'s
    /// hot per-frame loop (the lava's whole point is a ~10 fps sparse cadence, not
    /// full refresh). Delayed wakes clamp to one ambient step and wrap over the
    /// field's full two-cycle period ([`crate::lava::advance_phase`]).
    pub fn advance_lava(&mut self, dt: f32) {
        self.lava_phase = crate::lava::advance_phase(self.lava_phase, dt);
    }

    pub fn hold_lava_field_viewport(&mut self, width: u32, height: u32) {
        if self.lava_field_viewport[0] <= 0.0 || self.lava_field_viewport[1] <= 0.0 {
            self.lava_field_viewport = [width as f32, height as f32];
        }
    }

    pub fn settle_lava_field_viewport(&mut self, width: u32, height: u32) {
        self.lava_field_viewport = [width as f32, height as f32];
    }

    pub fn lava_blur_active(&self) -> bool {
        self.backdrop_blur()
    }

    /// Pin the lava lamp's phase to the FROZEN composition — the live App calls
    /// this when the lamp must be static (Reduce Motion, or `ambient_motion` off),
    /// so resuming from a hard-frozen state restarts from the settled frame rather
    /// than a stale mid-bob.
    pub fn freeze_lava(&mut self) {
        self.lava_phase = crate::lava::LAVA_FROZEN_PHASE;
    }

    /// COPY PULSE: kick the selection quad's brighten/decay AND the caret's own
    /// gentle pulse — a successful M-w/Cmd-C copy of a non-empty selection,
    /// otherwise entirely invisible. Resets [`Self::copy_pulse_t`] to 0 (full
    /// brighten); [`Self::step_copy_pulse`] eases it back to 1.0 (settled) over
    /// [`COPY_PULSE_MS`] on the live clock, consumed by
    /// [`Self::prepare_selection_layer`]. Idempotent under rapid re-fire (copying
    /// again mid-decay just restarts the pulse). Live-only: nothing in the
    /// headless `--keys` replay path calls this (see `main/run.rs`'s
    /// `Effect::CopyPulse` no-op arm), so a default capture never carries a boost.
    pub fn copy_pulse(&mut self) {
        self.copy_pulse_t = 0.0;
        self.caret.copy_pulse();
    }

    /// Tick the copy-pulse's decay by `dt` seconds, easing [`Self::copy_pulse_t`]
    /// back toward 1.0 (settled) over [`COPY_PULSE_MS`]. Returns true while still
    /// in flight, so [`Self::advance`]'s "keep redrawing" OR-fold stays hot only
    /// while the pulse plays, then idles — mirrors [`crate::caret::CaretAnim::step_pop`]
    /// exactly.
    fn step_copy_pulse(&mut self, dt: f32) -> bool {
        // ACCESSIBILITY TIER 1 — REDUCE MOTION: settle the selection-tint
        // brighten INSTANTLY to its resting (fully-settled) value instead of
        // decaying over `dt` — same final color, zero frames of ease. Mirrors
        // `step_caret`'s gate exactly; see `motion.rs`'s determinism note (this
        // branch is unreachable from a headless capture path).
        if crate::motion::reduced() {
            self.copy_pulse_t = 1.0;
            return false;
        }
        if self.copy_pulse_t >= 1.0 {
            return false;
        }
        self.copy_pulse_t = (self.copy_pulse_t + dt * 1000.0 / COPY_PULSE_MS).min(1.0);
        self.copy_pulse_t < 1.0
    }

    /// The copy-pulse's EASED settle fraction THIS frame — 0.0 at the instant of
    /// the kick (full brighten), 1.0 once settled (the plain theme tint, and the
    /// permanent value in every headless capture). Smoothstep eased, mirroring
    /// [`crate::caret::CaretAnim::pop_scale`]'s ease exactly. Consumed by
    /// [`Self::prepare_selection_layer`] to blend the selection quad's color.
    pub(in crate::render) fn copy_pulse_settle(&self) -> f32 {
        copy_pulse_ease(self.copy_pulse_t)
    }

    /// Advance the CARET-STYLE picker's live preview loop by `dt` — but ONLY while
    /// that picker is open (`caret_preview.is_some()`). Returns true while it is open
    /// (so the live loop stays HOT and the preview keeps looping); the instant the
    /// picker closes (`None`) this returns false, the loop idles, and the preview
    /// stops — back to 0% idle CPU (DESIGN §6). The geometry is seeded in `prepare`
    /// each frame (it needs the card layout), so a frame with no geometry yet still
    /// reports "open" to keep the loop alive until the first prepare seeds it.
    fn step_caret_preview(&mut self, dt: f32) -> bool {
        if self.caret_preview.is_none() {
            return false;
        }
        // ACCESSIBILITY TIER 1 — REDUCE MOTION: the caret-style picker's
        // choreographed demo (typing/gliding/deleting on a loop) settles to
        // its fixed, fully-typed end-state instead of looping — the SAME
        // frame a headless capture already renders for this preview
        // (`CaretDemo::settle`), so the picker still shows the selected
        // look correctly, just without the ambient motion. Returns `false`
        // (not still-animating) so the redraw loop is free to idle.
        if crate::motion::reduced() {
            self.caret_demo.settle();
            return false;
        }
        self.caret_demo.step(dt);
        true
    }
}
