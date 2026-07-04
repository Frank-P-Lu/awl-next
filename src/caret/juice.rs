//! CARET JUICE — the live-only edit/blocked-action flinches layered OVER the
//! spring as PURE draw-time scale + velocity impulses that always decay back to
//! the SAME resting caret (so a settled headless capture is byte-identical): the
//! cosmetic squash-pop (`kick_pop`/`kick_squash`/`step_pop`/`pop_scale`), the
//! typing impact, the deletion squash, the kill-line gulp, the Enter line-landing
//! squash (all velocity-damped via `impact_damp`), and the velocity-kick primitives
//! (`kick`/`recoil`) the I-beam recoil + blocked-action bump ride.
//!
//! These stay inherent methods on [`CaretAnim`], lifted out of `caret.rs`
//! VERBATIM; a child module sees its ancestor's private fields so the flinches
//! keep full access with NO behaviour change. The tunable `CARET_*` magnitudes
//! live in the `caret` root and resolve here via `use super::*`.

use super::*;


impl CaretAnim {
    /// KICK the cosmetic squash-pop: reset its progress to 0 (fully squashed),
    /// restarting the scale animation. Called by [`nav_to`] on each navigation move
    /// that actually relocates the caret. Idempotent under rapid re-fire (a held
    /// arrow): it simply re-zeroes the progress, so the pop keeps restarting rather
    /// than accumulating. PURELY cosmetic — it touches no position/velocity state, so
    /// the caret position is never affected.
    pub fn kick_pop(&mut self) {
        self.kick_squash(CARET_POP_SCALE, CARET_POP_MS);
    }

    /// KICK the cosmetic squash-pop to an explicit FLOOR over an explicit DURATION
    /// (ms) — the generalized pop the nav bounce, the deletion squash, the kill-line
    /// gulp and the typing impact all share. Resets the progress to 0 (fully squashed
    /// to `floor`); [`step_pop`] eases it back to 1.0 over `ms`. PURELY a draw-time
    /// scale (touches no position/velocity), so the caret never moves and the settled
    /// capture stays byte-identical.
    fn kick_squash(&mut self, floor: f32, ms: f32) {
        self.pop_floor = floor;
        self.pop_ms = ms;
        self.pop_t = 0.0;
    }

    /// The VELOCITY-DAMP factor in `[0, 1]` for an edit flinch, read from the caret's
    /// CURRENT spring speed BEFORE the kick is added: ~1.0 at rest (a deliberate
    /// keystroke → full thunk), falling to 0 as the speed reaches
    /// [`CARET_TYPE_IMPACT_DAMP_VEL`] (a fast burst — held backspace / mashed typing,
    /// the spring still racing from the prior keystroke → the flinch smooths into a
    /// slide and never strobes). Pure, so the damping is unit-testable.
    fn impact_damp(&self) -> f32 {
        let speed = (self.vel.x * self.vel.x + self.vel.y * self.vel.y).sqrt();
        (1.0 - speed / CARET_TYPE_IMPACT_DAMP_VEL).clamp(0.0, 1.0)
    }

    /// TYPING IMPACT (PHASE 2): the visual caret FLINCHES as a character is typed — a
    /// quick squash-pop ([`CARET_TYPE_IMPACT_SCALE`]) PLUS a velocity BACK-KICK
    /// ([`CARET_TYPE_IMPACT_KICK`]) AGAINST the forward insertion, so it recoils at the
    /// keystroke and the spring (its target already at the new cell) settles it forward.
    /// VELOCITY-DAMPED by [`impact_damp`]: a deliberate keystroke lands the full thunk,
    /// a fast burst smooths into a slide. Rides only the VISUAL caret — the logical
    /// cursor and `target` are untouched (no input latency), and it decays to the SAME
    /// resting caret, so a settled capture is byte-identical. Fires in EVERY caret look.
    pub fn type_impact(&mut self) {
        let damp = self.impact_damp();
        // Lerp the squash floor toward 1.0 (no squash) as the damp falls.
        let floor = 1.0 - (1.0 - CARET_TYPE_IMPACT_SCALE) * damp;
        self.kick_squash(floor, CARET_POP_MS);
        // Back-kick AGAINST forward typing (leftward); the spring — already targeting
        // the new cell to the right — then settles the caret forward past the flinch.
        self.kick(-CARET_TYPE_IMPACT_KICK * damp, 0.0);
    }

    /// DELETION SQUASH (PHASE 2): a small INWARD squash ([`CARET_DELETE_SQUASH`]) as a
    /// backspace / C-d swallows the character into the caret — the mark compresses
    /// toward the deletion point ("it eats what it deletes"). The OPPOSITE of typing's
    /// outward flinch: a PURE scale collapse with NO velocity kick. VELOCITY-DAMPED so
    /// a held backspace never strobes. Draw-time scale only; decays to the same resting
    /// caret (byte-identical settled capture). Every caret look.
    pub fn delete_squash(&mut self) {
        let damp = self.impact_damp();
        let floor = 1.0 - (1.0 - CARET_DELETE_SQUASH) * damp;
        self.kick_squash(floor, CARET_POP_MS);
    }

    /// KILL-LINE GULP (PHASE 2): a BIGGER, longer caret pulse ([`CARET_GULP_SCALE`] over
    /// [`CARET_GULP_MS`]) — a single satisfying swallow as a whole line vanishes into
    /// the caret. VELOCITY-DAMPED (a held C-k won't strobe). Draw-time scale only;
    /// decays to the same resting caret. Every caret look.
    pub fn gulp(&mut self) {
        let damp = self.impact_damp();
        let floor = 1.0 - (1.0 - CARET_GULP_SCALE) * damp;
        self.kick_squash(floor, CARET_GULP_MS);
    }

    /// ENTER JUICE — LINE LANDING (PHASE 3): a caret-level "touchdown" squash
    /// ([`CARET_LINE_LAND_SCALE`]) as the caret takes the new line under Enter,
    /// springing back to 1.0 over [`CARET_LINE_LAND_MS`]. PURE draw-time scale, NO
    /// velocity kick (see the constant's doc: Newline's vertical reflow already SNAPS
    /// via [`CaretAnim::jump_to`], and a kick on this axis would re-introduce the
    /// exact caret-lags-on-Enter lag that snap fixed). VELOCITY-DAMPED via
    /// [`impact_damp`] like the other edit flinches, so a fast held-Enter burst
    /// smooths into a slide and never strobes. Fires in EVERY caret look; decays to
    /// the SAME resting caret (byte-identical settled capture).
    pub fn line_land(&mut self) {
        let damp = self.impact_damp();
        let floor = 1.0 - (1.0 - CARET_LINE_LAND_SCALE) * damp;
        self.kick_squash(floor, CARET_LINE_LAND_MS);
    }

    /// Tick the cosmetic squash-pop by `dt` seconds, easing its progress back toward
    /// 1.0 (settled) over [`CARET_POP_MS`]. Returns true while the pop is still in
    /// flight (progress < 1), so the renderer's `advance(dt)` seam can OR it into the
    /// "keep redrawing" signal and the live loop stays hot only WHILE popping, then
    /// idles. A no-op (returns false) once settled, so it never adds a busy loop.
    /// Independent of the spring `step`: a small move snaps the position instantly and
    /// leaves the spring un-animating, yet the pop still plays through this tick.
    pub fn step_pop(&mut self, dt: f32) -> bool {
        if self.pop_t >= 1.0 {
            return false;
        }
        self.pop_t = (self.pop_t + dt * 1000.0 / self.pop_ms).min(1.0);
        self.pop_t < 1.0
    }

    /// The cosmetic scale to draw the caret mark at THIS frame: 1.0 at rest, dipping
    /// to the current `pop_floor` ([`CARET_POP_SCALE`] for a nav bounce, a delete /
    /// gulp / typing floor for an edit flinch) the instant a kick fires and
    /// smoothstep-easing back to 1.0 as [`step_pop`] runs the clock. The renderer multiplies the drawn
    /// rect's width/height (and corner) by this, about the UNCHANGED centre — so the
    /// caret squashes and springs back in place without ever moving.
    pub fn pop_scale(&self) -> f32 {
        // Smoothstep ease so the spring-back is soft (no linear kink as it lands).
        let e = self.pop_t * self.pop_t * (3.0 - 2.0 * self.pop_t);
        self.pop_floor + (1.0 - self.pop_floor) * e
    }

    /// Scale a caret rect's `(w, h, corner)` by THIS frame's cosmetic squash-pop
    /// ([`pop_scale`]) — the pure twin of the renderer's `pop_scaled`, exposed so the
    /// caret-style picker's preview can squash-pop with the SAME machinery as the
    /// document caret. At rest the factor is 1.0 (identity, byte-stable capture).
    pub fn pop_scale_dims(&self, w: f32, h: f32, corner: f32) -> (f32, f32, f32) {
        let s = self.pop_scale();
        (w * s, h * s, corner * s)
    }

    /// Inject a one-shot velocity IMPULSE into the spring (px/s), used by the
    /// I-beam caret's typing RECOIL: the spring then self-settles the kick through
    /// the same integration, so the bar nudges and springs back with no extra
    /// per-frame logic. `dx > 0` recoils right (InsertChar), `dx < 0` flinches left
    /// (DeleteBackward). (Newline no longer kicks: a vertical reflow now SNAPS via
    /// [`jump_to`], and a downward gravity-drop would reintroduce the very lag of
    /// the insertion point that snap removes.) Marks the spring animating so the
    /// step loop runs the kick out. Purely additive to the current velocity, so a
    /// kick mid-glide rides on top of the in-flight motion.
    pub fn kick(&mut self, dx: f32, dy: f32) {
        self.vel.x += dx;
        self.vel.y += dy;
        self.animating = true;
    }

    /// RECOIL the visual caret in `dir` — a BLOCKED-ACTION bump. A discrete action
    /// was requested but could not proceed (a motion into a wall, an exhausted
    /// undo, a delete with nothing to remove), so the caret gets a one-shot
    /// velocity IMPULSE ([`CARET_RECOIL_IMPULSE`]) AWAY from the wall and the
    /// existing spring settles it back. Purely a velocity kick on the VISUAL caret
    /// (reuses [`kick`]); the logical cursor is untouched, and the spring decays to
    /// the SAME resting caret, so a settled headless capture is byte-identical. The
    /// kick is ADDITIVE, so a recoil mid-glide rides on top of the in-flight motion.
    pub fn recoil(&mut self, dir: RecoilDir) {
        let (dx, dy) = dir.impulse();
        self.kick(dx, dy);
    }
}
