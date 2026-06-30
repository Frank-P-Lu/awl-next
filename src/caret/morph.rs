//! CARET SHAPE MORPH + STREAK — the geometry that conveys MOTION: the settle
//! factor (rounded square ⇄ trailing streak), the true travel vector + effective
//! draw axis, the axis-free in-motion streak geometry (`motion_geometry`), the
//! held-streak length, and the cosmetic | trail decoupled from position
//! (`trail_gate`/`kick_trail`/`step_trail`/`trail_geometry`/…).
//!
//! These stay inherent methods on [`CaretAnim`], carved out of `caret.rs`
//! VERBATIM. A child module sees its ancestor's private fields, so the pure
//! geometry keeps full access with NO behaviour change — the renderer and the unit
//! tests share these exact functions, so the capture output is byte-identical. The
//! shape/settle/trail constants live in the `caret` root and resolve here via
//! `use super::*`.

use super::*;


impl CaretAnim {
    /// The UNIT travel direction of the current glide — the TRUE motion vector, not
    /// an axis. Prefers the spring velocity (the live direction of travel); when the
    /// caret is nearly stopped it falls back to the remaining vector to the target,
    /// and finally to +x. This is what makes the in-motion trail a direct line from
    /// where the caret WAS to where it IS: a horizontal move → ±x, a vertical move →
    /// ±y, and a DIAGONAL move (e.g. an incremental-search jump between matches on a
    /// different row AND column) → the real slanted vector, never snapped to an axis.
    pub fn travel_dir(&self) -> (f32, f32) {
        let speed = (self.vel.x * self.vel.x + self.vel.y * self.vel.y).sqrt();
        if speed > 1.0 {
            return (self.vel.x / speed, self.vel.y / speed);
        }
        let dx = self.target.x - self.pos.x;
        let dy = self.target.y - self.pos.y;
        let d = (dx * dx + dy * dy).sqrt();
        if d > f32::EPSILON {
            (dx / d, dy / d)
        } else {
            (1.0, 0.0)
        }
    }

    /// The EFFECTIVE draw axis for the morph: the true travel direction `u`
    /// mid-glide, easing back to +x (axis-aligned) as the caret settles so the
    /// RESTING caret is always an upright rounded square. The x-sign of `u` is held
    /// through the blend so a horizontal move never passes through a degenerate
    /// zero vector; a pure-vertical move rotates from the bar toward upright as it
    /// lands (imperceptible, since the streak has nearly re-formed by then).
    fn eff_axis(&self, u: (f32, f32), s: f32) -> (f32, f32) {
        let motion = 1.0 - s;
        let sign_x = if u.0 < 0.0 { -1.0 } else { 1.0 };
        let ex = u.0 * motion + sign_x * s;
        let ey = u.1 * motion;
        let mag = (ex * ex + ey * ey).sqrt();
        if mag < 1e-6 {
            (1.0, 0.0)
        } else {
            (ex / mag, ey / mag)
        }
    }

    /// Unified, axis-FREE morph geometry for the single caret quad. Returns the
    /// rect CENTER (px), its half-length ALONG travel, its half-thickness ACROSS
    /// travel, and the unit travel AXIS. ONE rule covers every direction (no
    /// if-vertical / if-horizontal branch):
    ///   * AT REST (settle 1): an upright block — length `block_w`, thickness
    ///     `block_h`, axis +x, centred on the glyph cell.
    ///   * IN MOTION (settle→0): a thin streak of `streak_len` along the TRUE
    ///     travel vector, thickness `streak_thin`, the body trailing BACK along the
    ///     travel axis from a LEADING edge anchored at the caret's vertical CENTRE
    ///     (`pos.y`) for EVERY mode and direction. There is NO baseline drop: a
    ///     same-row (horizontal) move runs a centred sweep THROUGH the line centre
    ///     (not an underline under the text), exactly like the centre-to-centre
    ///     vertical / diagonal trail. Only the X moves to the glyph-cell centre.
    /// Pure (takes the zoomed metric scalars, no GPU), so the renderer and the unit
    /// tests share it.
    pub fn motion_geometry(
        &self,
        block_w: f32,
        block_h: f32,
        streak_thin: f32,
        streak_len: f32,
        streak_gap: f32,
        text_center_drop: f32,
    ) -> (Sample, f32, f32, (f32, f32)) {
        // While HOLDING, draw the trail in its full IN-MOTION form: pin the morph
        // blend to motion (s = 0). The held length (`streak_len`) is already a
        // STEADY constant; letting the oscillating settle factor blend it back
        // toward the resting block (it tracks instantaneous velocity, which pulses
        // once per auto-repeat) would re-introduce the per-repeat breathing and
        // could shrink the drawn length below the gap. Pinning keeps a constant thin
        // streak trailing the caret. NON-held keeps the natural settle morph.
        let s = if self.holding {
            0.0
        } else {
            self.settle_factor()
        };
        let motion = 1.0 - s;
        let axis = self.eff_axis(self.travel_dir(), s);
        let along = streak_len + (block_w - streak_len) * s;
        let across = streak_thin + (block_h - streak_thin) * s;
        // Leading edge: the glyph-cell centre x, at the caret's vertical anchor.
        // The trail is CENTRE-anchored for every mode and direction (no
        // drop-to-baseline detour), but the anchor itself is the TEXT optical centre,
        // not the geometric line-box centre: `pos.y` is the line-box centre, which
        // reads slightly high over the letters, so we drop it by `text_center_drop`
        // to the x-height middle. The drop is scaled by `motion` so it ONLY affects
        // the moving trail — at rest (motion 0) the block stays exactly on `pos.y`.
        let head = Sample {
            x: self.pos.x + block_w * 0.5,
            y: self.pos.y + text_center_drop * motion,
        };
        // Centre sits half the length back along the travel axis from the head, so
        // the streak TRAILS the leading edge; at rest (motion 0) centre == head ==
        // the block centre on the glyph.
        let center = Sample {
            x: head.x - axis.0 * (along * 0.5) * motion,
            y: head.y - axis.1 * (along * 0.5) * motion,
        };
        // Inset the TAIL — the ORIGIN-side end, AWAY from the caret, where the move
        // STARTED — by `streak_gap` ALONG the travel vector, but ONLY in motion (the
        // resting block keeps its full width). Shorten the length by the gap and slide
        // the centre toward the HEAD by half the removed length, so the LEADING edge
        // (the head, glued to the caret) is UNCHANGED and only the tail pulls in →
        // a gap opens between the start point and the trail. A move shorter than the
        // gap clamps the length to 0 (`max(0)`), so it draws NO streak.
        // While HOLDING (continuous/held motion) the full ~1.5-char gap would
        // swallow each one-char hop's trail, so demote it to a small cosmetic
        // trim; the lone-hop suppression (full gap) is kept for a discrete tap.
        let gap_eff = if self.holding {
            streak_gap * HELD_GAP_FRAC
        } else {
            streak_gap
        };
        let gap = gap_eff * motion;
        let inset = (along - gap).max(0.0);
        let removed = along - inset; // = gap, or = along when the gap swallows it
        let center = Sample {
            x: center.x + axis.0 * removed * 0.5,
            y: center.y + axis.1 * removed * 0.5,
        };
        (center, inset * 0.5, across * 0.5, axis)
    }

    /// The in-motion TRAIL as its two endpoints `(tail, head)` in absolute pixels —
    /// a DIRECT line from where the caret WAS (tail) to where it IS (head), along the
    /// true travel vector. ALWAYS anchored at the caret's vertical CENTRE — for every
    /// mode and direction, horizontal included (no baseline drop). Derived from
    /// [`motion_geometry`] so it always matches the drawn quad. A test reads these to
    /// assert a diagonal trail truly slants (not axis-snapped), and that every trail
    /// (horizontal / vertical / diagonal) anchors at the centre. Test-only inspector
    /// over the same `motion_geometry` the renderer draws from (the production path
    /// uses that directly), so it carries no runtime cost.
    #[cfg(test)]
    pub fn trail_endpoints(
        &self,
        block_w: f32,
        block_h: f32,
        streak_thin: f32,
        streak_len: f32,
        streak_gap: f32,
        text_center_drop: f32,
    ) -> (Sample, Sample) {
        let (c, half_along, _half_across, axis) = self.motion_geometry(
            block_w,
            block_h,
            streak_thin,
            streak_len,
            streak_gap,
            text_center_drop,
        );
        let tail = Sample {
            x: c.x - axis.0 * half_along,
            y: c.y - axis.1 * half_along,
        };
        let head = Sample {
            x: c.x + axis.0 * half_along,
            y: c.y + axis.1 * half_along,
        };
        (tail, head)
    }

    /// The trailing-streak LENGTH (px) for the current spring state. `speed_len`
    /// is the speed-derived length (`Metrics::streak_len_for_speed`); when NOT
    /// holding the result is that, floored by this frame's travel so a fast glide
    /// bridges with no gaps — byte-identical to the old `speed_len.max(frame_dist)`.
    ///
    /// While HOLDING (a continuous auto-repeat drag) the speed-/span-derived length
    /// OSCILLATES once per repeat — each ~30ms re-target spikes the spring velocity,
    /// which partly settles before the next, so the length (and the lag span) pulse
    /// in lock-step. That made the trail breathe and occasionally dip below the gap.
    /// Instead we return a STEADY `held_len` ([`HELD_STREAK_LEN`]), clamped to
    /// `max_len`, so the held trail is a constant-length streak trailing the caret
    /// rather than a per-repeat pulse.
    pub fn streak_length(&self, speed_len: f32, max_len: f32, held_len: f32) -> f32 {
        if !self.holding {
            return speed_len.max(self.frame_dist());
        }
        held_len.min(max_len)
    }

    /// A smooth [0,1] factor: 1.0 when the caret is at rest on its target (so the
    /// shape is the resting rounded square ON the glyph), → 0 while it is far from
    /// target and/or moving fast (so the shape drops to the baseline and stretches
    /// into the trailing underline). Driven by BOTH distance and speed so the
    /// square only re-forms once the caret has actually arrived and decelerated —
    /// mid-glide (fast spring) it reads as a streak on the line.
    ///
    /// Pure function of the current spring state, so the morph is unit-testable.
    pub fn settle_factor(&self) -> f32 {
        // Typing-sized hops never drop to the underline: the caret stays the
        // rounded square and just slides to the next cell.
        if self.streak_suppressed {
            return 1.0;
        }
        let dx = self.target.x - self.pos.x;
        let dy = self.target.y - self.pos.y;
        let dist = (dx * dx + dy * dy).sqrt();
        let speed = (self.vel.x * self.vel.x + self.vel.y * self.vel.y).sqrt();
        // Each term is 1.0 when the corresponding quantity is ~0 and decays toward
        // 0 as it grows. We take the MIN so either "still far" OR "still fast"
        // keeps the caret collapsed; both must be small for the underline to form.
        let by_dist = 1.0 - (dist / SETTLE_DIST_SCALE).clamp(0.0, 1.0);
        let by_vel = 1.0 - (speed / SETTLE_VEL_SCALE).clamp(0.0, 1.0);
        let raw = by_dist.min(by_vel);
        // Smoothstep so the re-form eases in (no linear kink as it lands).
        raw * raw * (3.0 - 2.0 * raw)
    }

    /// GATE for the cosmetic | trail: does a move from `from` to `to` qualify to draw
    /// the streak, and is it VERTICAL? Returns `Some(vertical)` when it qualifies, or
    /// `None` for a short same-row hop that should show NO streak. Split on
    /// row-crossing exactly like [`is_zip_move`]: a move that crosses a row is VERTICAL
    /// and ALWAYS qualifies (any single line shows the |); a same-row move qualifies
    /// only when its horizontal distance exceeds [`CARET_TRAIL_MIN_CHARS`] advances.
    /// Pure + zoom-invariant (via `glyph_advance`/`line_height`), so it is testable.
    fn trail_gate(&self, from: Sample, to: Sample) -> Option<bool> {
        let rows = (to.y - from.y).abs() / self.line_height;
        if rows >= 0.5 {
            Some(true) // vertical move (any row change) -> the | always shows
        } else if (to.x - from.x).abs() > CARET_TRAIL_MIN_CHARS * self.glyph_advance {
            Some(false) // a real horizontal JUMP -> a horizontal streak
        } else {
            None // a short same-row hop -> no streak (just snap + pop)
        }
    }

    /// KICK the cosmetic | trail: if the move qualifies (see [`trail_gate`]), latch a
    /// fresh fading streak from `from` to `to` and reset its fade to 0 (full alpha);
    /// otherwise CLEAR any leftover streak (`trail_present = false`) so a short hop
    /// draws none. `held` marks an auto-repeat so the renderer/report can treat the
    /// re-kicked stream as one steady | . PURELY cosmetic — touches no
    /// position/velocity, so the caret position is never affected (mirrors `kick_pop`).
    pub fn kick_trail(&mut self, from: Sample, to: Sample, held: bool) {
        match self.trail_gate(from, to) {
            Some(vertical) => {
                self.trail_present = true;
                self.trail_from = from;
                self.trail_to = to;
                self.trail_t = 0.0;
                // Restart the SWEEP: the leading edge whips from `from` toward `to`
                // over CARET_TRAIL_SWEEP_MS before the fade begins. A held re-kick
                // re-zeroes it, but the held path pins the drawn span to full so the
                // re-zero is invisible (steady stream); see `trail_sweep_p`.
                self.trail_sweep_t = 0.0;
                self.trail_vertical = vertical;
                self.trail_held = held;
            }
            None => {
                self.trail_present = false;
            }
        }
    }

    /// Tick the cosmetic | trail fade by `dt`, easing its progress toward 1.0 (faded
    /// out) over [`CARET_TRAIL_MS`]. Returns true while a streak is still visible (so
    /// `advance(dt)` keeps the live loop hot, then idles), false once it has faded /
    /// is absent. Independent of the spring (a snapped small move leaves the spring
    /// un-animating yet the streak still fades through this tick). A held auto-repeat
    /// re-kicks via [`kick_trail`] each repeat, topping the fade back up so it reads as
    /// one continuous | until release, then this fades it out.
    pub fn step_trail(&mut self, dt: f32) -> bool {
        if !self.trail_present || self.trail_t >= 1.0 {
            return false;
        }
        // Two phases on one clock: SWEEP first (the leading edge whips old→new), then
        // FADE. Any dt that overshoots the sweep boundary SPILLS its remainder into the
        // fade, so the total visible duration is CARET_TRAIL_MS regardless of step size
        // (a coarse timeline dt never stalls on the boundary) and stays deterministic.
        let mut dms = dt * 1000.0;
        if self.trail_sweep_t < 1.0 {
            let need = (1.0 - self.trail_sweep_t) * CARET_TRAIL_SWEEP_MS;
            if dms <= need {
                self.trail_sweep_t = (self.trail_sweep_t + dms / CARET_TRAIL_SWEEP_MS).min(1.0);
                dms = 0.0;
            } else {
                self.trail_sweep_t = 1.0;
                dms -= need;
            }
        }
        if dms > 0.0 {
            // Fade runs only AFTER the sweep completes; alpha is held at peak during
            // the sweep (trail_t stays 0), then eases to 0 over the remaining window.
            let fade_ms = (CARET_TRAIL_MS - CARET_TRAIL_SWEEP_MS).max(1.0);
            self.trail_t = (self.trail_t + dms / fade_ms).min(1.0);
        }
        if self.trail_t >= 1.0 {
            self.trail_present = false;
        }
        self.trail_present
    }

    /// SETTLE the cosmetic | trail to its absent state (no streak, fully faded). Called
    /// by the frozen capture paths ([`snap_to_target`]/[`inject_motion`]) so the
    /// headless `--screenshot` renders the trail-absent settled frame and stays
    /// byte-deterministic.
    pub(super) fn settle_trail(&mut self) {
        self.trail_present = false;
        self.trail_t = 1.0;
        self.trail_sweep_t = 1.0;
    }

    /// The eased SWEEP progress in `[0, 1]`: 0 = the streak's leading edge sits at the
    /// OLD caret position (just kicked); 1 = it has whipped along to the NEW (caret)
    /// position, so the full old→new span is drawn. The renderer/`trail_geometry` lerps
    /// the head between the two endpoints by this, so over the first
    /// [`CARET_TRAIL_SWEEP_MS`] the eye reads a fast directional SWEEP toward the caret
    /// (the position itself stays pinned). Ease-OUT (cubic) so the edge whips out fast
    /// and decelerates as it ARRIVES on the caret.
    ///
    /// A HELD auto-repeat pins this to 1.0 (the full span every frame): a held arrow
    /// re-kicks the sweep each ~30ms, and animating the per-repeat draw-on would make
    /// the drawn length pulse/strobe; pinning keeps ONE steady continuous streak, the
    /// downward motion coming instead from each repeat's old→new span advancing a line.
    pub fn trail_sweep_p(&self) -> f32 {
        if self.trail_held {
            return 1.0;
        }
        let t = self.trail_sweep_t.clamp(0.0, 1.0);
        let inv = 1.0 - t;
        1.0 - inv * inv * inv
    }

    /// The cosmetic streak's current alpha: 0 when absent/faded, else
    /// [`CARET_TRAIL_ALPHA`] eased down by the smoothstepped fade. A held re-kick keeps
    /// `trail_t` near 0, so a held run stays at (near) peak alpha — one steady |.
    pub fn trail_alpha(&self) -> f32 {
        if !self.trail_present {
            return 0.0;
        }
        let t = self.trail_t.clamp(0.0, 1.0);
        let e = t * t * (3.0 - 2.0 * t);
        CARET_TRAIL_ALPHA * (1.0 - e)
    }

    /// Whether a cosmetic streak is being drawn this frame (present AND not yet faded).
    pub fn trail_active(&self) -> bool {
        self.trail_present && self.trail_t < 1.0
    }

    /// Whether the current cosmetic streak is a VERTICAL | (a row change) vs a
    /// horizontal jump streak. Only meaningful while [`trail_active`].
    pub fn is_trail_vertical(&self) -> bool {
        self.trail_vertical
    }

    /// Whether the current cosmetic streak belongs to a HELD auto-repeat (re-kicked
    /// each repeat → one steady, continuous |). Only meaningful while [`trail_active`].
    pub fn is_trail_held(&self) -> bool {
        self.trail_held
    }

    /// COSMETIC | TRAIL geometry: the streak quad SWEEPING from the OLD caret position
    /// toward the NEW one, DECOUPLED from the (already-snapped) spring position.
    /// Returns the rect CENTER (px), half-length ALONG travel, half-thickness ACROSS,
    /// and the unit travel AXIS — same shape the renderer feeds the caret quad. The
    /// TAIL is anchored at the origin (`trail_from`); the HEAD (leading edge) is lerped
    /// from `trail_from` toward `trail_to` by the eased [`trail_sweep_p`], so during the
    /// sweep window the streak DRAWS ON in the travel direction (old→new) and lands with
    /// its head glued to the new caret. After the sweep the head rests on `trail_to`
    /// (full span) and the streak fades. A small cosmetic inset trims the origin-side
    /// tail (the lone-hop suppression is the gate's job, not the gap's, so a single-line
    /// | still draws nearly full once swept). The vertical anchor is dropped to the text
    /// optical centre (`text_center_drop`) so the streak runs THROUGH the letters. Pure
    /// (takes the zoomed metric scalars), so the renderer and the unit tests share it.
    pub fn trail_geometry(
        &self,
        streak_thin: f32,
        streak_gap: f32,
        text_center_drop: f32,
        center_x_drop: f32,
    ) -> (Sample, f32, f32, (f32, f32)) {
        // The AXIS is the full, stable old→new direction (so a near-zero sweep extent
        // can't degenerate it); the HEAD is lerped along that axis by the sweep.
        let p = self.trail_sweep_p();
        let full_dx = self.trail_to.x - self.trail_from.x;
        let full_dy = self.trail_to.y - self.trail_from.y;
        let full = (full_dx * full_dx + full_dy * full_dy).sqrt();
        let axis = if full > 1e-6 {
            (full_dx / full, full_dy / full)
        } else {
            (1.0, 0.0)
        };
        // `center_x_drop` (half the caret cell width) slides the streak from the cell's
        // LEFT edge (the raw caret x) to its horizontal CENTRE, so the | runs down the
        // MIDDLE of the block instead of hugging its left side — the horizontal twin of
        // `text_center_drop`. Applied to both endpoints so the whole axis re-centres.
        let tail_pt = Sample {
            x: self.trail_from.x + center_x_drop,
            y: self.trail_from.y + text_center_drop,
        };
        // The leading edge has swept a fraction `p` of the way old→new.
        let head_pt = Sample {
            x: self.trail_from.x + full_dx * p + center_x_drop,
            y: self.trail_from.y + full_dy * p + text_center_drop,
        };
        let dx = head_pt.x - tail_pt.x;
        let dy = head_pt.y - tail_pt.y;
        let along = (dx * dx + dy * dy).sqrt();
        // Cosmetic tail trim only (NOT the full lone-hop suppression gap — the gate
        // already suppressed short hops), so even a single-line | draws nearly full.
        let gap = streak_gap * HELD_GAP_FRAC;
        let inset = (along - gap).max(0.0);
        let removed = along - inset;
        // Midpoint of the full span, slid toward the HEAD by half the trimmed length so
        // the head stays glued to the new caret and only the tail pulls in.
        let center = Sample {
            x: (tail_pt.x + head_pt.x) * 0.5 + axis.0 * removed * 0.5,
            y: (tail_pt.y + head_pt.y) * 0.5 + axis.1 * removed * 0.5,
        };
        (center, inset * 0.5, streak_thin * 0.5, axis)
    }
}
